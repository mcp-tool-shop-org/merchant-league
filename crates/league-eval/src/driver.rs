//! Core Evaluation Mode driver (`play`, `branch`, `select` helper) for `league-eval`.
//!
//! Coordinates [`league_core::EvalHost`], [`league_core::ClubBot`], and `league-relay` without ever
//! invoking non-eval host APIs (`ClubHost`), `write_save_and_seal`, or `engine-internals`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use league_core::{
    ClubBot, EvalActSubmission, EvalHost, EvalSeatApplyReport, HARD_TURN_CAP, PublicSweepReceipt,
    ReplayOutcome, Seat, TableId, TurnPhase, replay_eval_season,
};
use league_relay::{
    ApiKey, CallLogEntry, MemoryMode, PreviewToolMode, RawHttpResponse, Round,
    SavedDiscardedReplyRecord, SavedHttpReplyRecord, SavedToolRoundRecord, SeatBackend, SeatFile,
    SeatOutcome, SeatSpec, TranscriptEntry, Transport, TransportError, TurnConfig, TurnSummary,
    UntrustedWhisper, parse_preview_tool_arguments, run_turn, sha256_hex,
};
use serde::Serialize;

use crate::EvalError;
use crate::records::{
    BranchArmOutcome, BranchManifest, BranchSummary, DecisionTokenTopLogprobs,
    DecisionTopLogprobEntry, EvalArm, EvalDecisionRecord, EvalManifest, EvalManifestSeat,
    EvalSetupFile, FailedRelayAttempt, PASS_B_LIMIT_STATEMENT, RefusalCodeCounts,
    RefusedBundleRecord, TurnRegentAuditRecord, WrongPartCounter, count_bundle_legs,
    summarize_branch_outcomes,
};
use crate::reply::{
    ReplyRefusalCode, classify_wrong_round_from_raw_text, parse_eval_act_reply,
    parse_eval_talk_reply,
};
use crate::schema::build_eval_response_format_config;
use crate::select::{SelectionList, expand_home_path};
use crate::serving::{
    ServingManifestRecord, WordingArmDigests, capture_serving_env_snapshot,
    load_and_validate_serving_manifest, resolve_verified_crate_git_sha,
};
use crate::view::{
    build_act_negotiation_inputs, render_preview_state_json, render_seat_view_markdown,
};
use crate::wording::{EVAL_REPLY_WORDING_ARM, resolve_pinned_wrapper};

/// Inert mock transport for all-`ClubBot` offline runs with zero HTTP calls.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopTransport;

impl Transport for NoopTransport {
    fn post_chat_completions(
        &self,
        _url: &str,
        _body: &str,
        _timeout: Duration,
    ) -> Result<RawHttpResponse, TransportError> {
        Err(TransportError::Network(
            "NoopTransport invoked unexpectedly with an automated seat".to_owned(),
        ))
    }
}

/// Arguments for [`run_play_with_transport`].
#[derive(Debug, Clone)]
pub struct PlayOptions {
    /// Target game directory (`eval-<table>.{save.json,seal}`, `setup.json`, `manifest.json`, `checkpoints/`).
    pub game_dir: PathBuf,
    /// Path to 4-seat `seats.json`.
    pub seats_path: PathBuf,
    /// Path to pinned seat wrapper markdown brief.
    pub wrapper_path: PathBuf,
    /// Optional explicit `--wrapper-version` string (must match the pinned version when set).
    pub wrapper_version: Option<String>,
    /// Optional path to `--serving-manifest <path>` (required whenever any seat uses a `Model` backend).
    pub serving_manifest_path: Option<PathBuf>,
    /// Evaluation arm (`off`, `control`, or `on`).
    pub arm: EvalArm,
    /// Table identifier (`TableId`).
    pub table_id: u64,
    /// Table layout seed passed to `EvalHost`.
    pub seed: u64,
    /// Maximum turns to play in this run (`1..=32`).
    pub turns: u8,
    /// Prompt memory mode (`snapshot`, `emc`, or `full`).
    pub memory_mode: MemoryMode,
    /// Live transport gate flag.
    pub live: bool,
    /// OpenRouter cumulative credit spend ceiling.
    pub cap_credits: Option<f64>,
    /// Local VM cumulative GPU-seconds spend ceiling.
    pub cap_gpu_secs: Option<f64>,
    /// Explicitly permitted VM-local hosts beyond loopback.
    pub allowed_vm_hosts: Vec<String>,
}

/// Arguments for [`run_branch_with_transport`].
#[derive(Debug, Clone)]
pub struct BranchOptions {
    /// Pre-act checkpoint prefix (`<game_dir>/checkpoints/day-DD-pre-act`); mutually exclusive with `list_path`.
    pub checkpoint_prefix: Option<PathBuf>,
    /// Path to selection JSON written by `league-eval select --out`; mutually exclusive with `checkpoint_prefix`.
    pub list_path: Option<PathBuf>,
    /// Focal seat guild or `0..3` seat index (required with `checkpoint_prefix`).
    pub focal_seat: Option<String>,
    /// Path to 4-seat `seats.json` providing the focal seat's backend/sampling config.
    pub seats_path: PathBuf,
    /// Path to pinned seat wrapper markdown brief.
    pub wrapper_path: PathBuf,
    /// Optional path to `--serving-manifest <path>` (required when branching a `Model` seat).
    pub serving_manifest_path: Option<PathBuf>,
    /// Arms to evaluate from the frozen pre-act state (e.g. `[Off, Control, On]`).
    pub arms: Vec<EvalArm>,
    /// Root output directory where only spectator/public branch artifacts are written.
    pub out_dir: PathBuf,
    /// Prompt memory mode (`snapshot`, `emc`, or `full`).
    pub memory_mode: MemoryMode,
    /// Live transport gate flag.
    pub live: bool,
    /// OpenRouter credit ceiling per branch invocation.
    pub cap_credits: Option<f64>,
    /// Local VM GPU-seconds ceiling per branch invocation.
    pub cap_gpu_secs: Option<f64>,
    /// Explicitly permitted VM-local hosts beyond loopback.
    pub allowed_vm_hosts: Vec<String>,
}

/// Single persistence gateway for `league-eval` (`R8`): calls `EvalHost::write_game_save_and_seal`
/// and never `EvalHost::write_save_and_seal`.
pub fn persist_eval_game_state(
    host: &EvalHost,
    game_dir: &Path,
) -> Result<(PathBuf, PathBuf), EvalError> {
    host.write_game_save_and_seal(game_dir)
        .map_err(EvalError::from)
}

/// Write `val` formatted as 2-space pretty JSON + trailing `\n` atomically via temp file + rename.
pub(crate) fn write_json_pretty_atomic<T: Serialize>(
    path: &Path,
    val: &T,
) -> Result<(), EvalError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|e| EvalError::Io(format!("create directory {}: {e}", parent.display())))?;
    }
    let mut rendered = serde_json::to_string_pretty(val)
        .map_err(|e| EvalError::Io(format!("serialize JSON for {}: {e}", path.display())))?;
    rendered.push('\n');
    write_text_atomic(path, &rendered)
}

pub(crate) fn write_text_atomic(path: &Path, text: &str) -> Result<(), EvalError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|e| EvalError::Io(format!("create directory {}: {e}", parent.display())))?;
    }
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("eval-artifact");
    let tmp_path = path.with_file_name(format!("{file_name}.tmp.{}", std::process::id()));
    if tmp_path.exists() {
        let _ = fs::remove_file(&tmp_path);
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp_path)
        .map_err(|e| EvalError::Io(format!("open temp file {}: {e}", tmp_path.display())))?;
    if let Err(err) = file.write_all(text.as_bytes()).and_then(|()| file.flush()) {
        drop(file);
        let _ = fs::remove_file(&tmp_path);
        return Err(EvalError::Io(format!(
            "write temp file {}: {err}",
            tmp_path.display()
        )));
    }
    drop(file);
    if path.exists() {
        fs::remove_file(path)
            .map_err(|e| EvalError::Io(format!("replace existing {}: {e}", path.display())))?;
    }
    if let Err(err) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(EvalError::Io(format!(
            "rename {} -> {}: {err}",
            tmp_path.display(),
            path.display()
        )));
    }
    Ok(())
}

/// Validate Evaluation Mode seat count (`D6` = 4 seats), `"file"` backend policy (`1.7`, Test 6),
/// and ban `reasoning.exclude_from_transcript: true` (`A1.3`).
pub fn validate_eval_seats_file(seats_file: &SeatFile) -> Result<(), EvalError> {
    if seats_file.seats.len() != 4 {
        return Err(EvalError::InvalidSeatCount {
            actual: seats_file.seats.len(),
        });
    }
    for seat in &seats_file.seats {
        let exclude_from_transcript = match &seat.backend {
            SeatBackend::File { model } => {
                if model.as_deref() != Some("ClubBot") {
                    return Err(EvalError::UnsupportedFileSeatModel {
                        guild: seat.guild.clone(),
                        model: model.clone(),
                    });
                }
                false
            }
            SeatBackend::Local(l) => l
                .reasoning
                .as_ref()
                .is_some_and(|r| r.exclude_from_transcript),
            SeatBackend::Openrouter(o) => o
                .reasoning
                .as_ref()
                .is_some_and(|r| r.exclude_from_transcript),
        };
        if exclude_from_transcript {
            return Err(EvalError::EvalThinkStrippingForbidden(format!(
                "seat '{}' sets reasoning.exclude_from_transcript=true, which is forbidden in Evaluation Mode",
                seat.guild
            )));
        }
    }
    Ok(())
}

/// Determine whether `seat` keeps `"notes"` under `league-eval` & `league-relay` rules (`1.4`).
#[must_use]
pub fn seat_keeps_notes(seat: &SeatSpec, seats_file: &SeatFile, memory_mode: MemoryMode) -> bool {
    if memory_mode != MemoryMode::Snapshot {
        return false;
    }
    match &seat.backend {
        SeatBackend::Local(_) => seat.scratchpad.or(seats_file.scratchpad) != Some(false),
        SeatBackend::Openrouter(_) | SeatBackend::File { .. } => false,
    }
}

fn arm_to_preview_mode(arm: EvalArm) -> Result<PreviewToolMode, EvalError> {
    match arm {
        EvalArm::Off => Ok(PreviewToolMode::Off),
        EvalArm::Control => Ok(PreviewToolMode::Control),
        EvalArm::On => Ok(PreviewToolMode::On),
    }
}

fn memory_mode_str(mode: MemoryMode) -> &'static str {
    match mode {
        MemoryMode::Snapshot => "snapshot",
        MemoryMode::Emc => "emc",
        MemoryMode::Full => "full",
    }
}

fn draw_os_bot_tie_seed() -> Result<u64, EvalError> {
    let mut buf = [0_u8; 8];
    for _ in 0..8 {
        getrandom::fill(&mut buf)
            .map_err(|e| EvalError::Io(format!("OS CSPRNG failure drawing bot_tie_seed: {e}")))?;
        let val = u64::from_le_bytes(buf);
        if val != 0 {
            return Ok(val);
        }
    }
    Err(EvalError::Io(
        "OS CSPRNG repeatedly yielded all-zero bot_tie_seed".to_owned(),
    ))
}

fn hash_file_sha256(path: &Path, label: &str) -> Result<String, EvalError> {
    let bytes = fs::read(path)
        .map_err(|e| EvalError::Io(format!("read {label} {}: {e}", path.display())))?;
    Ok(sha256_hex(&bytes))
}

#[derive(Debug, Default, Clone)]
struct FirstAttemptFailure {
    reason: String,
    is_schema_or_json: bool,
    discarded_reply_path: Option<String>,
    discarded_reply_sha256: Option<String>,
    discarded_raw_reply: Option<String>,
}

#[derive(Debug, Default)]
struct RoundAttemptDiagnostics {
    attempts: u32,
    failed_attempts: Vec<FailedRelayAttempt>,
    first_failure: Option<FirstAttemptFailure>,
    prompt_tokens: u64,
    completion_tokens: u64,
    wall_secs: Option<f64>,
    tool_calls_used: u32,
    intermediate_bundle_sha256s: Vec<String>,
    prompt_token_ids_sha256s: Vec<String>,
    reply_token_ids: Vec<Vec<u64>>,
    reply_top_logprobs: Vec<Vec<DecisionTokenTopLogprobs>>,
}

fn count_calls_lines(calls_path: &Path) -> Result<usize, EvalError> {
    if !calls_path.exists() {
        return Ok(0);
    }
    let raw = fs::read_to_string(calls_path)
        .map_err(|e| EvalError::Io(format!("read {}: {e}", calls_path.display())))?;
    Ok(raw.lines().filter(|l| !l.trim().is_empty()).count())
}

fn is_relay_schema_or_json_error(reason: &str) -> bool {
    reason.starts_with("structured output invalid JSON")
        || reason.starts_with("structured output failed schema")
}

fn find_discarded_reply_sidecar(
    run_root: &Path,
    day_str: &str,
    round: Round,
    guild: &str,
    expected_sha: &str,
    expected_reservation_id: Option<&str>,
) -> (Option<String>, Option<String>, Option<String>) {
    let seats_dir = run_root.join("relay").join("seats");
    let Ok(entries) = fs::read_dir(&seats_dir) else {
        return (None, None, None);
    };
    let prefix = format!("{guild}.day-{day_str}-{round}.discarded-round-");
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if !ft.is_file() {
            continue;
        }
        let name_os = entry.file_name();
        let Some(file_name) = name_os.to_str() else {
            continue;
        };
        if !file_name.starts_with(&prefix) || !file_name.ends_with(".json") {
            continue;
        }
        let Ok(bytes) = fs::read(entry.path()) else {
            continue;
        };
        if sha256_hex(&bytes) != expected_sha {
            continue;
        }
        let Ok(rec) = serde_json::from_slice::<SavedDiscardedReplyRecord>(&bytes) else {
            continue;
        };
        if expected_reservation_id.is_some_and(|rid| rec.reservation_id != rid) {
            continue;
        }
        return (
            Some(format!("relay/seats/{file_name}")),
            Some(expected_sha.to_owned()),
            rec.assistant_content,
        );
    }
    (None, None, None)
}

fn is_wire_error_code(wire_json: Option<&str>, expected_code: &str) -> bool {
    let Some(raw) = wire_json else {
        return false;
    };
    serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|v| v.get("error_code")?.as_str().map(str::to_owned))
        .is_some_and(|c| c == expected_code)
}

fn parse_top_logprob_entries(raw: &serde_json::Value) -> Vec<DecisionTopLogprobEntry> {
    match raw {
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|item| {
                let obj = item.as_object()?;
                let token = obj.get("token")?.as_str()?.to_owned();
                let logprob = obj.get("logprob")?.as_f64()?;
                let token_id = obj
                    .get("token_id")
                    .or_else(|| obj.get("id"))
                    .and_then(serde_json::Value::as_u64);
                Some(DecisionTopLogprobEntry {
                    token,
                    token_id,
                    logprob,
                })
            })
            .take(5)
            .collect(),
        serde_json::Value::Object(map) => {
            let mut entries: Vec<DecisionTopLogprobEntry> = map
                .iter()
                .filter_map(|(k, v)| {
                    v.as_f64().map(|lp| DecisionTopLogprobEntry {
                        token: k.clone(),
                        token_id: None,
                        logprob: lp,
                    })
                })
                .collect();
            entries.sort_by(|a, b| {
                b.logprob
                    .total_cmp(&a.logprob)
                    .then_with(|| a.token.cmp(&b.token))
            });
            entries.truncate(5);
            entries
        }
        _ => Vec::new(),
    }
}

fn collect_http_reply_telemetry(
    body_val: &serde_json::Value,
    prompt_token_ids_sha256s: &mut Vec<String>,
    reply_token_ids: &mut Vec<Vec<u64>>,
    reply_top_logprobs: &mut Vec<Vec<DecisionTokenTopLogprobs>>,
) -> Result<(), EvalError> {
    let prompt_ids = body_val
        .get("prompt_token_ids")
        .and_then(serde_json::Value::as_array)
        .and_then(|arr| {
            arr.iter()
                .map(serde_json::Value::as_u64)
                .collect::<Option<Vec<u64>>>()
        })
        .filter(|ids| !ids.is_empty())
        .ok_or_else(|| {
            EvalError::MissingReplyTelemetry(
                "HTTP 200 reply missing non-empty `prompt_token_ids` for `prompt_token_ids_sha256s`"
                    .to_owned(),
            )
        })?;
    let prompt_bytes = serde_json::to_vec(&prompt_ids)
        .map_err(|e| EvalError::Io(format!("encode prompt_token_ids: {e}")))?;
    prompt_token_ids_sha256s.push(sha256_hex(&prompt_bytes));

    let first_choice = body_val
        .get("choices")
        .and_then(serde_json::Value::as_array)
        .and_then(|arr| arr.first())
        .ok_or_else(|| {
            EvalError::MissingReplyTelemetry(
                "HTTP 200 reply missing `choices[0]` for reply telemetry".to_owned(),
            )
        })?;

    let explicit_reply_ids = first_choice
        .get("token_ids")
        .or_else(|| body_val.get("completion_token_ids"))
        .or_else(|| body_val.get("token_ids"))
        .and_then(serde_json::Value::as_array)
        .and_then(|arr| {
            arr.iter()
                .map(serde_json::Value::as_u64)
                .collect::<Option<Vec<u64>>>()
        });

    let lp_content = first_choice
        .get("logprobs")
        .and_then(|lp| lp.get("content"))
        .and_then(serde_json::Value::as_array)
        .filter(|arr| !arr.is_empty())
        .ok_or_else(|| {
            EvalError::MissingReplyTelemetry(
                "HTTP 200 reply missing non-empty `choices[0].logprobs.content` for `reply_top_logprobs`"
                    .to_owned(),
            )
        })?;

    let mut extracted_logprobs: Vec<DecisionTokenTopLogprobs> =
        Vec::with_capacity(lp_content.len());
    let mut fallback_ids_from_logprobs: Vec<u64> = Vec::with_capacity(lp_content.len());
    for (idx, step) in lp_content.iter().enumerate() {
        let obj = step.as_object().ok_or_else(|| {
            EvalError::MissingReplyTelemetry(format!(
                "HTTP 200 reply `choices[0].logprobs.content[{idx}]` must be a JSON object"
            ))
        })?;
        let token = obj
            .get("token")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                EvalError::MissingReplyTelemetry(format!(
                    "HTTP 200 reply `choices[0].logprobs.content[{idx}]` missing string `token`"
                ))
            })?;
        let logprob = obj
            .get("logprob")
            .and_then(serde_json::Value::as_f64)
            .ok_or_else(|| {
                EvalError::MissingReplyTelemetry(format!(
                    "HTTP 200 reply `choices[0].logprobs.content[{idx}]` missing numeric `logprob`"
                ))
            })?;
        let token_id = obj
            .get("token_id")
            .or_else(|| obj.get("id"))
            .and_then(serde_json::Value::as_u64);
        if let Some(tid) = token_id {
            fallback_ids_from_logprobs.push(tid);
        }
        let top_logprobs = obj
            .get("top_logprobs")
            .map(parse_top_logprob_entries)
            .unwrap_or_default();
        if top_logprobs.is_empty() {
            return Err(EvalError::MissingReplyTelemetry(format!(
                "HTTP 200 reply `choices[0].logprobs.content[{idx}]` missing non-empty `top_logprobs`"
            )));
        }
        extracted_logprobs.push(DecisionTokenTopLogprobs {
            token: token.to_owned(),
            token_id,
            logprob,
            top_logprobs,
        });
    }

    let resolved_reply_ids = match explicit_reply_ids {
        Some(ids) if !ids.is_empty() => ids,
        _ if fallback_ids_from_logprobs.len() == extracted_logprobs.len()
            && !fallback_ids_from_logprobs.is_empty() =>
        {
            fallback_ids_from_logprobs
        }
        _ => {
            return Err(EvalError::MissingReplyTelemetry(
                "HTTP 200 reply missing non-empty reply token IDs (`token_ids` or `logprobs.content[].id`) for `reply_token_ids`"
                    .to_owned(),
            ));
        }
    };

    reply_token_ids.push(resolved_reply_ids);
    reply_top_logprobs.push(extracted_logprobs);
    Ok(())
}

fn read_round_attempt_diagnostics(
    run_root: &Path,
    calls_path: &Path,
    start_line: usize,
    day_str: &str,
    round: Round,
    seat_index: u8,
    guild: &str,
) -> Result<RoundAttemptDiagnostics, EvalError> {
    if !calls_path.exists() {
        return Ok(RoundAttemptDiagnostics::default());
    }
    let raw = fs::read_to_string(calls_path)
        .map_err(|e| EvalError::Io(format!("read {}: {e}", calls_path.display())))?;
    let mut out = RoundAttemptDiagnostics::default();
    let mut wall_sum = 0.0_f64;
    let mut saw_wall = false;
    let mut round_resv_ids: BTreeSet<String> = BTreeSet::new();

    for (idx, line) in raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .enumerate()
        .skip(start_line)
    {
        let entry: CallLogEntry = serde_json::from_str(line).map_err(|e| {
            EvalError::Io(format!(
                "parse {} line {}: {e}",
                calls_path.display(),
                idx + 1
            ))
        })?;
        if entry.day != day_str || entry.round != round || !entry.guild.eq_ignore_ascii_case(guild)
        {
            continue;
        }
        if let Some(ref rid) = entry.reservation_id {
            round_resv_ids.insert(rid.clone());
        }
        if entry.outcome != "RESERVED" && entry.outcome != "CAP_REACHED" {
            out.attempts = out.attempts.saturating_add(1);
            out.prompt_tokens = out
                .prompt_tokens
                .saturating_add(entry.prompt_tokens.unwrap_or(0));
            out.completion_tokens = out
                .completion_tokens
                .saturating_add(entry.completion_tokens.unwrap_or(0));
            if let Some(w) = entry.wall_secs {
                wall_sum += w;
                saw_wall = true;
            }
        }
        let is_success =
            entry.error.is_none() && matches!(entry.outcome.as_str(), "OK" | "TOOL_CALL");
        if !is_success {
            let reason = entry.error.clone().unwrap_or_else(|| entry.outcome.clone());
            // Always lock the first actual failed call attempt; reconciled orphaned RESERVED rows
            // stay in `failed_attempts` with their diagnostic reason without poisoning resumed calls.
            if entry.outcome != "RESERVED" && out.first_failure.is_none() {
                let (discarded_reply_path, discarded_reply_sha256, discarded_raw_reply) =
                    match entry.reply_sha256.as_deref() {
                        Some(sha) => find_discarded_reply_sidecar(
                            run_root,
                            day_str,
                            round,
                            guild,
                            sha,
                            entry.reservation_id.as_deref(),
                        ),
                        None => (None, None, None),
                    };
                out.first_failure = Some(FirstAttemptFailure {
                    reason: reason.clone(),
                    is_schema_or_json: is_relay_schema_or_json_error(&reason),
                    discarded_reply_path,
                    discarded_reply_sha256,
                    discarded_raw_reply,
                });
            }
            out.failed_attempts.push(FailedRelayAttempt {
                attempt: entry.attempt,
                outcome: entry.outcome,
                reason,
            });
        }
    }
    if saw_wall {
        out.wall_secs = Some(wall_sum);
    }

    let seats_dir = run_root.join("relay").join("seats");
    if seats_dir.is_dir() && !round_resv_ids.is_empty() {
        let tool_prefix = format!("{guild}.day-{day_str}-{round}.tool-round-");
        let http_prefix = format!("{guild}.day-{day_str}-{round}.http-reply-round-");
        let mut tool_records: Vec<SavedToolRoundRecord> = Vec::new();
        let mut http_records: Vec<SavedHttpReplyRecord> = Vec::new();
        let entries = fs::read_dir(&seats_dir)
            .map_err(|e| EvalError::Io(format!("read {}: {e}", seats_dir.display())))?;
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if !ft.is_file() {
                continue;
            }
            let name_os = entry.file_name();
            let Some(fname) = name_os.to_str() else {
                continue;
            };
            if fname.starts_with(&tool_prefix) && fname.ends_with(".json") {
                if let Ok(bytes) = fs::read(entry.path())
                    && let Ok(rec) = serde_json::from_slice::<SavedToolRoundRecord>(&bytes)
                    && round_resv_ids.contains(&rec.reservation_id)
                {
                    tool_records.push(rec);
                }
            } else if fname.starts_with(&http_prefix)
                && fname.ends_with(".json")
                && let Ok(bytes) = fs::read(entry.path())
                && let Ok(rec) = serde_json::from_slice::<SavedHttpReplyRecord>(&bytes)
                && round_resv_ids.contains(&rec.reservation_id)
            {
                http_records.push(rec);
            }
        }
        tool_records.sort_by_key(|r| (r.round_index, r.attempt));
        for rec in tool_records {
            for call in rec.calls {
                if !call.tool_result_sent {
                    continue;
                }
                let wire = call.result_wire_json.as_deref();
                if is_wire_error_code(wire, "CALL_LIMIT_REACHED") {
                    continue;
                }
                out.tool_calls_used = out.tool_calls_used.saturating_add(1);
                if is_wire_error_code(wire, "MALFORMED_TOOL_CALL") {
                    continue;
                }
                if let Some(parsed_req) = parse_preview_tool_arguments(&call.arguments, seat_index)
                    && let Ok(bundle_bytes) = serde_json::to_vec(&parsed_req.bundle)
                {
                    out.intermediate_bundle_sha256s
                        .push(sha256_hex(&bundle_bytes));
                }
            }
        }

        http_records.sort_by_key(|r| (r.round_index, r.attempt));
        for rec in http_records {
            if rec.status == 200
                && !rec.is_late
                && let Ok(body_val) = serde_json::from_str::<serde_json::Value>(&rec.body)
                && body_val.get("error").is_none_or(serde_json::Value::is_null)
            {
                collect_http_reply_telemetry(
                    &body_val,
                    &mut out.prompt_token_ids_sha256s,
                    &mut out.reply_token_ids,
                    &mut out.reply_top_logprobs,
                )?;
            }
        }
    }

    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn build_no_reply_record(
    game_id: &str,
    table_id: u64,
    day: u8,
    round: Round,
    seat: u8,
    guild: &str,
    arm: EvalArm,
    keeps_notes: bool,
    diag: RoundAttemptDiagnostics,
    loaded_history_sha256: BTreeMap<String, String>,
    club_bot_baseline: Option<EvalActSubmission>,
    refusal_by_seat: Option<&mut BTreeMap<String, RefusalCodeCounts>>,
    refusal_by_arm: Option<&mut BTreeMap<String, RefusalCodeCounts>>,
    wrong_part_refusals: Option<&mut WrongPartCounter>,
) -> EvalDecisionRecord {
    let first = diag.first_failure;
    let raw_reply = first.as_ref().and_then(|f| f.discarded_raw_reply.clone());
    let discarded_reply_path = first.as_ref().and_then(|f| f.discarded_reply_path.clone());
    let discarded_reply_sha256 = first
        .as_ref()
        .and_then(|f| f.discarded_reply_sha256.clone());

    let (wrong_round, notes_present) = raw_reply.as_deref().map_or((None, false), |raw| {
        classify_wrong_round_from_raw_text(round, raw, keeps_notes)
    });

    // Classify wrong-round BEFORE REPLY_SCHEMA_INVALID so schema-rejected wrong-round replies
    // get their specific wrong-round code and count toward the stop-line counter.
    let (refusal_code, refusal_reason, wrong_part_code) =
        if let Some((wp_code, wp_reason)) = wrong_round {
            (Some(wp_code), Some(wp_reason), Some(wp_code))
        } else if let Some(ref f) = first
            && f.is_schema_or_json
        {
            (
                Some(ReplyRefusalCode::ReplySchemaInvalid),
                Some(f.reason.clone()),
                None,
            )
        } else {
            (None, None, None)
        };

    if let Some(code) = refusal_code {
        if let Some(by_seat) = refusal_by_seat {
            by_seat.entry(guild.to_owned()).or_default().record(code);
        }
        if let Some(by_arm) = refusal_by_arm {
            by_arm
                .entry(arm.as_str().to_owned())
                .or_default()
                .record(code);
        }
    }
    if let Some(wp) = wrong_part_refusals {
        if let Some(wp_code) = wrong_part_code {
            wp.record_wrong_part_no_reply(wp_code);
        } else {
            wp.record_no_reply();
        }
    }
    let parse_failure = matches!(refusal_code, Some(ReplyRefusalCode::ReplySchemaInvalid))
        || first.as_ref().is_some_and(|f| f.is_schema_or_json);
    EvalDecisionRecord {
        game_id: game_id.to_owned(),
        table_id,
        day,
        round,
        seat,
        guild: guild.to_owned(),
        arm,
        seat_kind: "Model".to_owned(),
        keeps_notes,
        notes_present,
        raw_reply,
        discarded_reply_path,
        discarded_reply_sha256,
        attempts: diag.attempts,
        failed_attempts: diag.failed_attempts,
        prompt_tokens: diag.prompt_tokens,
        completion_tokens: diag.completion_tokens,
        wall_secs: diag.wall_secs,
        tool_calls_used: diag.tool_calls_used,
        parse_failure,
        refused_market_actions: 0,
        refused_bundle_legs: 0,
        cleared_non_empty_actions: false,
        intermediate_bundle_sha256s: diag.intermediate_bundle_sha256s,
        prompt_token_ids_sha256s: diag.prompt_token_ids_sha256s,
        reply_token_ids: diag.reply_token_ids,
        reply_top_logprobs: diag.reply_top_logprobs,
        loaded_history_sha256,
        refusal_code: refusal_code.map(|c| c.as_str().to_owned()),
        refusal_reason,
        invalid_recipients: Vec::new(),
        no_reply: true,
        apply_report: None,
        refused_bundle: None,
        club_bot_baseline,
    }
}

#[allow(clippy::too_many_arguments)]
fn resolve_talk_model_round(
    game_id: &str,
    table_id: u64,
    day: u8,
    seat: u8,
    guild: &str,
    arm: EvalArm,
    keeps_notes: bool,
    seated_guilds: &[String],
    outcome: Option<&SeatOutcome>,
    diag: RoundAttemptDiagnostics,
    whispers_by_guild: &mut BTreeMap<String, Vec<UntrustedWhisper>>,
    refusal_by_seat: &mut BTreeMap<String, RefusalCodeCounts>,
    refusal_by_arm: &mut BTreeMap<String, RefusalCodeCounts>,
    wrong_part_refusals: &mut WrongPartCounter,
) -> Result<EvalDecisionRecord, EvalError> {
    if diag.first_failure.is_some() {
        return Ok(build_no_reply_record(
            game_id,
            table_id,
            day,
            Round::Talk,
            seat,
            guild,
            arm,
            keeps_notes,
            diag,
            BTreeMap::new(),
            None,
            Some(refusal_by_seat),
            Some(refusal_by_arm),
            Some(wrong_part_refusals),
        ));
    }
    match outcome {
        Some(SeatOutcome::Delivered { from_seat_path, .. }) => {
            let raw_reply = fs::read_to_string(from_seat_path).map_err(|e| {
                EvalError::Io(format!("read reply {}: {e}", from_seat_path.display()))
            })?;
            let parsed = parse_eval_talk_reply(&raw_reply, seat, guild, seated_guilds, keeps_notes);
            let wrong_part = parsed
                .refusal_code
                .filter(|&c| c == ReplyRefusalCode::TalkReplyHasOrders);
            wrong_part_refusals.record_delivered_reply(wrong_part);

            let seat_counts = refusal_by_seat.entry(guild.to_owned()).or_default();
            let arm_counts = refusal_by_arm.entry(arm.as_str().to_owned()).or_default();
            if let Some(code) = parsed.refusal_code {
                seat_counts.record(code);
                arm_counts.record(code);
            }
            for d in &parsed.invalid_recipients {
                seat_counts.record(d.code);
                arm_counts.record(d.code);
            }
            for (recipient_guild, whisper) in parsed.accepted_whispers {
                if let Some(bucket) = whispers_by_guild.get_mut(&recipient_guild) {
                    bucket.push(whisper);
                }
            }
            let parse_failure = matches!(
                parsed.refusal_code,
                Some(ReplyRefusalCode::ReplySchemaInvalid | ReplyRefusalCode::TalkReplyInvalid)
            );
            Ok(EvalDecisionRecord {
                game_id: game_id.to_owned(),
                table_id,
                day,
                round: Round::Talk,
                seat,
                guild: guild.to_owned(),
                arm,
                seat_kind: "Model".to_owned(),
                keeps_notes,
                notes_present: parsed.notes_present,
                raw_reply: Some(raw_reply),
                discarded_reply_path: None,
                discarded_reply_sha256: None,
                attempts: diag.attempts,
                failed_attempts: diag.failed_attempts,
                prompt_tokens: diag.prompt_tokens,
                completion_tokens: diag.completion_tokens,
                wall_secs: diag.wall_secs,
                tool_calls_used: diag.tool_calls_used,
                parse_failure,
                refused_market_actions: 0,
                refused_bundle_legs: 0,
                cleared_non_empty_actions: false,
                intermediate_bundle_sha256s: diag.intermediate_bundle_sha256s,
                prompt_token_ids_sha256s: diag.prompt_token_ids_sha256s,
                reply_token_ids: diag.reply_token_ids,
                reply_top_logprobs: diag.reply_top_logprobs,
                loaded_history_sha256: BTreeMap::new(),
                refusal_code: parsed.refusal_code.map(|c| c.as_str().to_owned()),
                refusal_reason: parsed.refusal_reason,
                invalid_recipients: parsed
                    .invalid_recipients
                    .iter()
                    .map(|d| {
                        format!(
                            "{}: {}[{}]: {}",
                            d.code.as_str(),
                            d.to_guild,
                            d.message_index,
                            d.reason
                        )
                    })
                    .collect(),
                no_reply: false,
                apply_report: None,
                refused_bundle: None,
                club_bot_baseline: None,
            })
        }
        _ => Ok(build_no_reply_record(
            game_id,
            table_id,
            day,
            Round::Talk,
            seat,
            guild,
            arm,
            keeps_notes,
            diag,
            BTreeMap::new(),
            None,
            Some(refusal_by_seat),
            Some(refusal_by_arm),
            Some(wrong_part_refusals),
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn resolve_act_model_round(
    game_id: &str,
    table_id: u64,
    day: u8,
    seat: u8,
    guild: &str,
    arm: EvalArm,
    keeps_notes: bool,
    outcome: Option<&SeatOutcome>,
    diag: RoundAttemptDiagnostics,
    loaded_history_sha256: BTreeMap<String, String>,
    bot_sub: EvalActSubmission,
    refusal_by_seat: Option<&mut BTreeMap<String, RefusalCodeCounts>>,
    refusal_by_arm: Option<&mut BTreeMap<String, RefusalCodeCounts>>,
    wrong_part_refusals: Option<&mut WrongPartCounter>,
) -> Result<(EvalActSubmission, EvalDecisionRecord), EvalError> {
    if diag.first_failure.is_some() {
        return Ok((
            EvalActSubmission::default(),
            build_no_reply_record(
                game_id,
                table_id,
                day,
                Round::Act,
                seat,
                guild,
                arm,
                keeps_notes,
                diag,
                loaded_history_sha256,
                Some(bot_sub),
                refusal_by_seat,
                refusal_by_arm,
                wrong_part_refusals,
            ),
        ));
    }
    match outcome {
        Some(SeatOutcome::Delivered { from_seat_path, .. }) => {
            let raw_reply = fs::read_to_string(from_seat_path).map_err(|e| {
                EvalError::Io(format!("read reply {}: {e}", from_seat_path.display()))
            })?;
            let parsed = parse_eval_act_reply(&raw_reply, seat, keeps_notes);
            if let Some(wp) = wrong_part_refusals {
                let wrong_part = parsed
                    .refusal_code
                    .filter(|&c| c == ReplyRefusalCode::ActReplyHasMessages);
                wp.record_delivered_reply(wrong_part);
            }
            if let Some(code) = parsed.refusal_code {
                if let Some(by_seat) = refusal_by_seat {
                    by_seat.entry(guild.to_owned()).or_default().record(code);
                }
                if let Some(by_arm) = refusal_by_arm {
                    by_arm
                        .entry(arm.as_str().to_owned())
                        .or_default()
                        .record(code);
                }
            }
            let parse_failure = matches!(
                parsed.refusal_code,
                Some(ReplyRefusalCode::ReplySchemaInvalid | ReplyRefusalCode::ActReplyInvalid)
            );
            let rec = EvalDecisionRecord {
                game_id: game_id.to_owned(),
                table_id,
                day,
                round: Round::Act,
                seat,
                guild: guild.to_owned(),
                arm,
                seat_kind: "Model".to_owned(),
                keeps_notes,
                notes_present: parsed.notes_present,
                raw_reply: Some(raw_reply),
                discarded_reply_path: None,
                discarded_reply_sha256: None,
                attempts: diag.attempts,
                failed_attempts: diag.failed_attempts,
                prompt_tokens: diag.prompt_tokens,
                completion_tokens: diag.completion_tokens,
                wall_secs: diag.wall_secs,
                tool_calls_used: diag.tool_calls_used,
                parse_failure,
                refused_market_actions: 0,
                refused_bundle_legs: 0,
                cleared_non_empty_actions: false,
                intermediate_bundle_sha256s: diag.intermediate_bundle_sha256s,
                prompt_token_ids_sha256s: diag.prompt_token_ids_sha256s,
                reply_token_ids: diag.reply_token_ids,
                reply_top_logprobs: diag.reply_top_logprobs,
                loaded_history_sha256,
                refusal_code: parsed.refusal_code.map(|c| c.as_str().to_owned()),
                refusal_reason: parsed.refusal_reason,
                invalid_recipients: Vec::new(),
                no_reply: false,
                apply_report: None,
                refused_bundle: None,
                club_bot_baseline: Some(bot_sub),
            };
            Ok((parsed.submission, rec))
        }
        _ => Ok((
            EvalActSubmission::default(),
            build_no_reply_record(
                game_id,
                table_id,
                day,
                Round::Act,
                seat,
                guild,
                arm,
                keeps_notes,
                diag,
                loaded_history_sha256,
                Some(bot_sub),
                refusal_by_seat,
                refusal_by_arm,
                wrong_part_refusals,
            ),
        )),
    }
}

fn ensure_no_partial_from_seat_for_day(game_dir: &Path, active_day: u8) -> Result<(), EvalError> {
    let from_seat_dir = game_dir.join("from-seat");
    if !from_seat_dir.exists() {
        return Ok(());
    }
    let entries = fs::read_dir(&from_seat_dir).map_err(|e| {
        EvalError::Io(format!(
            "read from-seat dir {}: {e}",
            from_seat_dir.display()
        ))
    })?;
    for entry in entries {
        let entry = entry.map_err(|e| EvalError::Io(format!("scan from-seat dir: {e}")))?;
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        if let Some(rest) = name_str.strip_prefix("day-")
            && rest.len() >= 2
            && let Ok(file_day) = rest[..2].parse::<u8>()
            && file_day >= active_day
        {
            return Err(EvalError::PartialTurnFromSeatExists(format!(
                "cannot resume at day {active_day}: found existing partial-turn reply {}",
                entry.path().display()
            )));
        }
    }
    Ok(())
}

fn init_refusal_maps(
    seats_file: &SeatFile,
    arm: EvalArm,
) -> (
    BTreeMap<String, RefusalCodeCounts>,
    BTreeMap<String, RefusalCodeCounts>,
) {
    let mut by_seat = BTreeMap::new();
    for s in &seats_file.seats {
        by_seat.insert(s.guild.clone(), RefusalCodeCounts::default());
    }
    let by_arm = BTreeMap::from([(arm.as_str().to_owned(), RefusalCodeCounts::default())]);
    (by_seat, by_arm)
}

struct RecountedManifestCounters {
    by_seat: BTreeMap<String, RefusalCodeCounts>,
    by_arm: BTreeMap<String, RefusalCodeCounts>,
    wrong_part: WrongPartCounter,
}

fn recount_manifest_counters_from_decisions(
    decisions_dir: &Path,
    played_turns: u8,
    seats_file: &SeatFile,
    active_arm: EvalArm,
) -> Result<RecountedManifestCounters, EvalError> {
    let (mut by_seat, mut by_arm) = init_refusal_maps(seats_file, active_arm);
    let mut wp = WrongPartCounter::default();
    for day in 1..=played_turns {
        for round in [Round::Talk, Round::Act] {
            for seat in &seats_file.seats {
                let dec_path =
                    decisions_dir.join(format!("day-{day:02}-{round}-{}.json", seat.guild));
                let raw = fs::read_to_string(&dec_path).map_err(|e| {
                    EvalError::Io(format!(
                        "cannot recount manifest from missing decision {}: {e}",
                        dec_path.display()
                    ))
                })?;
                let rec: EvalDecisionRecord = serde_json::from_str(&raw)
                    .map_err(|e| EvalError::Io(format!("parse {}: {e}", dec_path.display())))?;
                let seat_counts = by_seat.entry(rec.guild.clone()).or_default();
                let arm_counts = by_arm.entry(rec.arm.as_str().to_owned()).or_default();
                if let Some(code_str) = rec.refusal_code.as_deref()
                    && let Some(code) = ReplyRefusalCode::from_code_str(code_str)
                {
                    seat_counts.record(code);
                    arm_counts.record(code);
                }
                for _ in &rec.invalid_recipients {
                    seat_counts.record(ReplyRefusalCode::MessageRecipientInvalid);
                    arm_counts.record(ReplyRefusalCode::MessageRecipientInvalid);
                }
                if rec.refused_bundle.is_some() {
                    seat_counts.refused_bundles += 1;
                    arm_counts.refused_bundles += 1;
                }
                if rec.seat_kind == "Model" {
                    let wrong_part = rec
                        .refusal_code
                        .as_deref()
                        .and_then(ReplyRefusalCode::from_code_str)
                        .filter(|&c| {
                            matches!(
                                c,
                                ReplyRefusalCode::TalkReplyHasOrders
                                    | ReplyRefusalCode::ActReplyHasMessages
                            )
                        });
                    if rec.no_reply {
                        if let Some(wp_code) = wrong_part {
                            wp.record_wrong_part_no_reply(wp_code);
                        } else {
                            wp.record_no_reply();
                        }
                    } else {
                        wp.record_delivered_reply(wrong_part);
                    }
                }
            }
        }
    }
    wp.validate_rates()?;
    Ok(RecountedManifestCounters {
        by_seat,
        by_arm,
        wrong_part: wp,
    })
}

fn resolve_serving_manifest_opt(
    serving_manifest_path: Option<&Path>,
    requires_serving_manifest: bool,
) -> Result<Option<ServingManifestRecord>, EvalError> {
    if let Some(path) = serving_manifest_path {
        return load_and_validate_serving_manifest(path).map(Some);
    }
    if requires_serving_manifest {
        return Err(EvalError::MissingServingManifest(
            "automated model seat evaluation requires `--serving-manifest <path>`".to_owned(),
        ));
    }
    Ok(None)
}

fn seat_regent_flags(
    host: &EvalHost,
    seats_file: &SeatFile,
) -> (Vec<bool>, BTreeMap<String, bool>) {
    let spec = host.public_spec();
    let mut vec_flags = Vec::with_capacity(seats_file.seats.len());
    let mut map_flags = BTreeMap::new();
    for (idx, seat) in seats_file.seats.iter().enumerate() {
        let has_regent = spec.seats.get(idx).is_some_and(|s| s.regent.is_some());
        vec_flags.push(has_regent);
        map_flags.insert(seat.guild.clone(), has_regent);
    }
    (vec_flags, map_flags)
}

fn build_turn_regent_audit(receipts: &[PublicSweepReceipt]) -> Vec<TurnRegentAuditRecord> {
    receipts
        .iter()
        .map(|r| TurnRegentAuditRecord {
            turn: r.turn,
            converted_to_ai: r.converted_to_ai.clone(),
            regent_acted_seats: r.regent_acted_seats.clone(),
        })
        .collect()
}

fn find_model_seat_regent_intervention(
    receipt: &PublicSweepReceipt,
    model_seats: &[bool],
    seats_file: &SeatFile,
) -> Option<String> {
    let mut hits: Vec<String> = Vec::new();
    for (field_label, seat_indices) in [
        ("converted_to_ai", &receipt.converted_to_ai),
        ("regent_acted_seats", &receipt.regent_acted_seats),
    ] {
        for &seat_idx in seat_indices {
            if model_seats.get(usize::from(seat_idx)) == Some(&true) {
                let guild = seats_file
                    .seats
                    .get(usize::from(seat_idx))
                    .map_or("unknown", |s| s.guild.as_str());
                hits.push(format!("{field_label}[seat={seat_idx},guild={guild}]"));
            }
        }
    }
    if hits.is_empty() {
        None
    } else {
        Some(format!(
            "turn {} sweep intervened on automated model seat(s): {}",
            receipt.turn,
            hits.join(", ")
        ))
    }
}

fn ensure_no_model_seat_regent_at_startup(
    host: &EvalHost,
    model_seats: &[bool],
    has_regents: &[bool],
    seats_file: &SeatFile,
) -> Result<(), EvalError> {
    for receipt in host.public_receipts() {
        if let Some(msg) = find_model_seat_regent_intervention(receipt, model_seats, seats_file) {
            return Err(EvalError::ModelSeatRegentIntervention(format!(
                "startup check refused already-voided run: {msg}"
            )));
        }
    }
    for (idx, (&is_model, &has_regent)) in model_seats.iter().zip(has_regents.iter()).enumerate() {
        if is_model && has_regent {
            let guild = seats_file
                .seats
                .get(idx)
                .map_or("unknown", |s| s.guild.as_str());
            return Err(EvalError::ModelSeatRegentIntervention(format!(
                "startup check refused run: automated model seat [seat={idx},guild={guild}] has a Standing Regent configured"
            )));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn empty_clubbot_decision(
    game_id: &str,
    table_id: u64,
    day: u8,
    round: Round,
    seat: u8,
    guild: &str,
    arm: EvalArm,
    loaded_history_sha256: BTreeMap<String, String>,
    club_bot_baseline: Option<EvalActSubmission>,
) -> EvalDecisionRecord {
    EvalDecisionRecord {
        game_id: game_id.to_owned(),
        table_id,
        day,
        round,
        seat,
        guild: guild.to_owned(),
        arm,
        seat_kind: "ClubBot".to_owned(),
        keeps_notes: false,
        notes_present: false,
        raw_reply: None,
        discarded_reply_path: None,
        discarded_reply_sha256: None,
        attempts: 0,
        failed_attempts: Vec::new(),
        prompt_tokens: 0,
        completion_tokens: 0,
        wall_secs: None,
        tool_calls_used: 0,
        parse_failure: false,
        refused_market_actions: 0,
        refused_bundle_legs: 0,
        cleared_non_empty_actions: false,
        intermediate_bundle_sha256s: Vec::new(),
        prompt_token_ids_sha256s: Vec::new(),
        reply_token_ids: Vec::new(),
        reply_top_logprobs: Vec::new(),
        loaded_history_sha256,
        refusal_code: None,
        refusal_reason: None,
        invalid_recipients: Vec::new(),
        no_reply: false,
        apply_report: None,
        refused_bundle: None,
        club_bot_baseline,
    }
}

fn populate_act_apply_telemetry(
    rec: &mut EvalDecisionRecord,
    sub: &EvalActSubmission,
    report: &EvalSeatApplyReport,
    arm: EvalArm,
    refusal_by_seat: Option<&mut BTreeMap<String, RefusalCodeCounts>>,
    refusal_by_arm: Option<&mut BTreeMap<String, RefusalCodeCounts>>,
) -> Result<(), EvalError> {
    rec.refused_market_actions =
        u32::try_from(report.refused_market_actions.len()).unwrap_or(u32::MAX);
    if let Some(code) = report.refused_bundle_code.as_ref() {
        rec.refused_bundle_legs = count_bundle_legs(&sub.bundle);
        if let Some(by_seat) = refusal_by_seat {
            by_seat
                .entry(rec.guild.clone())
                .or_default()
                .refused_bundles += 1;
        }
        if let Some(by_arm) = refusal_by_arm {
            by_arm
                .entry(arm.as_str().to_owned())
                .or_default()
                .refused_bundles += 1;
        }
        rec.refused_bundle = Some(RefusedBundleRecord {
            submitted_bundle: sub.bundle.clone(),
            error_code: code.clone(),
            bad_leg_index: "leg not named".to_owned(),
        });
    } else {
        rec.refused_bundle_legs = 0;
        rec.refused_bundle = None;
    }
    let accepted_market_main = sub
        .market_actions
        .len()
        .checked_sub(report.refused_market_actions.len())
        .ok_or_else(|| {
            EvalError::InvalidActionCount(format!(
                "refused_market_actions ({}) exceeded submitted market_actions ({})",
                report.refused_market_actions.len(),
                sub.market_actions.len()
            ))
        })?;
    let accepted_pass_b = sub
        .pass_b_actions
        .len()
        .checked_sub(report.refused_pass_b_actions.len())
        .ok_or_else(|| {
            EvalError::InvalidActionCount(format!(
                "refused_pass_b_actions ({}) exceeded submitted pass_b_actions ({})",
                report.refused_pass_b_actions.len(),
                sub.pass_b_actions.len()
            ))
        })?;
    let accepted_market = accepted_market_main + accepted_pass_b;
    let accepted_bundle_legs = if report.refused_bundle_code.is_some() {
        0
    } else {
        count_bundle_legs(&sub.bundle)
    };
    rec.cleared_non_empty_actions = accepted_market > 0 || accepted_bundle_legs > 0;
    rec.apply_report = Some(report.clone());
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn build_eval_manifest(
    game_id: &str,
    table_id: u64,
    seed: u64,
    arm: EvalArm,
    relay_wording_arm: &str,
    wrapper_version: &str,
    wrapper_sha256: &str,
    response_schema_sha256: &str,
    crate_git_sha: &str,
    played_turns: u8,
    seats_file: &SeatFile,
    memory_mode: MemoryMode,
    has_regents: &[bool],
    seat_regents: BTreeMap<String, bool>,
    turn_regent_audit: Vec<TurnRegentAuditRecord>,
    voided: bool,
    void_error_code: Option<String>,
    voided_games_by_arm: BTreeMap<String, u64>,
    serving: Option<ServingManifestRecord>,
    serving_env_snapshot: BTreeMap<String, String>,
    refusal_by_seat: &BTreeMap<String, RefusalCodeCounts>,
    refusal_by_arm: &BTreeMap<String, RefusalCodeCounts>,
    wrong_part_refusals: &WrongPartCounter,
) -> EvalManifest {
    let manifest_seats: Vec<EvalManifestSeat> = seats_file
        .seats
        .iter()
        .enumerate()
        .map(|(idx, seat)| {
            let (backend, model) = seat_backend_and_model(seat);
            EvalManifestSeat {
                seat: idx as u8,
                guild: seat.guild.clone(),
                backend: backend.to_owned(),
                model,
                keeps_notes: seat_keeps_notes(seat, seats_file, memory_mode),
                has_regent: has_regents.get(idx).copied().unwrap_or(false),
            }
        })
        .collect();

    EvalManifest {
        game_id: game_id.to_owned(),
        table_id,
        seed,
        arm,
        eval_reply_wording_arm: EVAL_REPLY_WORDING_ARM.to_owned(),
        relay_wording_arm: relay_wording_arm.to_owned(),
        wrapper_version: wrapper_version.to_owned(),
        wrapper_sha256: wrapper_sha256.to_owned(),
        pass_b_limit: PASS_B_LIMIT_STATEMENT.to_owned(),
        played_turns,
        crate_git_sha: crate_git_sha.to_owned(),
        wording_digests: Some(WordingArmDigests::compute(
            relay_wording_arm,
            wrapper_version,
            wrapper_sha256,
            response_schema_sha256,
        )),
        serving,
        serving_env_snapshot,
        seats: manifest_seats,
        seat_regents,
        turn_regent_audit,
        voided,
        void_error_code,
        voided_games_by_arm,
        refusal_counts_by_seat: refusal_by_seat.clone(),
        refusal_counts_by_arm: refusal_by_arm.clone(),
        wrong_part_refusals: wrong_part_refusals.clone(),
    }
}

fn dir_has_eval_save_or_seal(dir: &Path) -> Result<bool, EvalError> {
    if !dir.exists() {
        return Ok(false);
    }
    let entries = fs::read_dir(dir)
        .map_err(|e| EvalError::Io(format!("read game_dir {}: {e}", dir.display())))?;
    for entry in entries {
        let entry = entry.map_err(|e| EvalError::Io(format!("scan game_dir: {e}")))?;
        if let Some(name) = entry.file_name().to_str()
            && name.starts_with("eval-")
            && (name.ends_with(".save.json") || name.ends_with(".seal"))
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn validate_wrapper_and_extract(
    wrapper_path: &Path,
    explicit_version: Option<&str>,
) -> Result<(String, String), EvalError> {
    if !wrapper_path.exists() {
        return Err(EvalError::MissingWrapper(format!(
            "wrapper brief not found at {}",
            wrapper_path.display()
        )));
    }
    let raw_bytes = fs::read(wrapper_path).map_err(|e| {
        EvalError::MissingWrapper(format!(
            "cannot read wrapper brief {}: {e}",
            wrapper_path.display()
        ))
    })?;
    if raw_bytes.is_empty() {
        return Err(EvalError::MissingWrapper(format!(
            "wrapper brief {} is empty",
            wrapper_path.display()
        )));
    }
    resolve_pinned_wrapper(&raw_bytes, explicit_version)
}

fn seat_backend_and_model(seat: &SeatSpec) -> (&'static str, String) {
    match &seat.backend {
        SeatBackend::Local(l) => ("local", l.model.clone()),
        SeatBackend::Openrouter(o) => ("openrouter", o.model.clone()),
        SeatBackend::File { model } => (
            "file",
            model.clone().unwrap_or_else(|| "ClubBot".to_owned()),
        ),
    }
}

fn find_report<'a>(summary: &'a TurnSummary, guild: &str) -> Option<&'a SeatOutcome> {
    summary
        .reports
        .iter()
        .find(|r| r.guild.eq_ignore_ascii_case(guild))
        .map(|r| &r.outcome)
}

/// Run an Evaluation Mode game cleanly using `transport` (`off` or `on`; offline `NoopTransport` or
/// mock HTTP `Transport` in tests, real HTTP `Transport` in live runs).
pub fn run_play_with_transport(
    opts: &PlayOptions,
    transport: &dyn Transport,
    redaction_key: Option<&ApiKey>,
) -> Result<EvalManifest, EvalError> {
    if opts.turns == 0 || opts.turns > HARD_TURN_CAP {
        return Err(EvalError::Usage(format!(
            "--turns must be in 1..={HARD_TURN_CAP}, got {}",
            opts.turns
        )));
    }
    let preview_mode = arm_to_preview_mode(opts.arm)?;
    let (wrapper_version, wrapper_sha256) =
        validate_wrapper_and_extract(&opts.wrapper_path, opts.wrapper_version.as_deref())?;
    let seats_sha256 = hash_file_sha256(&opts.seats_path, "seats file")?;
    let mem_mode = memory_mode_str(opts.memory_mode).to_owned();
    let seats_file =
        SeatFile::load_with_allowed_hosts(&opts.seats_path, redaction_key, &opts.allowed_vm_hosts)?;
    validate_eval_seats_file(&seats_file)?;

    let model_seats: Vec<bool> = seats_file
        .seats
        .iter()
        .map(|s| !matches!(s.backend, SeatBackend::File { .. }))
        .collect();
    let has_model_seat = model_seats.iter().any(|&is_model| is_model);
    let crate_git_sha = resolve_verified_crate_git_sha()?;
    let serving_record =
        resolve_serving_manifest_opt(opts.serving_manifest_path.as_deref(), has_model_seat)?;
    let serving_manifest_sha256 = serving_record
        .as_ref()
        .map(|rec| {
            serde_json::to_vec(rec)
                .map(|bytes| sha256_hex(&bytes))
                .map_err(|e| EvalError::Io(format!("encode serving manifest digest: {e}")))
        })
        .transpose()?;
    let serving_env_snapshot = capture_serving_env_snapshot()?;

    let response_format = build_eval_response_format_config()?;
    let response_schema_sha256 = response_format.canonical_sha256();
    let relay_wording_arm = preview_mode.wording_arm().to_owned();

    fs::create_dir_all(&opts.game_dir)
        .map_err(|e| EvalError::Io(format!("create game_dir {}: {e}", opts.game_dir.display())))?;

    let setup_path = opts.game_dir.join("setup.json");
    let manifest_path = opts.game_dir.join("manifest.json");

    let mut host = if setup_path.exists() {
        let raw_setup = fs::read_to_string(&setup_path)
            .map_err(|e| EvalError::Io(format!("read {}: {e}", setup_path.display())))?;
        let existing_setup: EvalSetupFile = serde_json::from_str(&raw_setup)
            .map_err(|e| EvalError::Io(format!("parse {}: {e}", setup_path.display())))?;
        let host = EvalHost::read_game_save_and_seal(&opts.game_dir, TableId(opts.table_id))?;
        if existing_setup.game_id != host.game_id() {
            return Err(EvalError::IncompatibleEvalSetup(format!(
                "setup.json game_id '{}' does not match seal game_id '{}'",
                existing_setup.game_id,
                host.game_id()
            )));
        }
        if existing_setup.arm != opts.arm
            || existing_setup.table_id != opts.table_id
            || existing_setup.seed != opts.seed
            || existing_setup.wrapper_sha256 != wrapper_sha256
            || existing_setup.memory_mode != mem_mode
            || existing_setup.seats_sha256 != seats_sha256
            || existing_setup.serving_manifest_sha256 != serving_manifest_sha256
        {
            return Err(EvalError::IncompatibleEvalSetup(
                "cannot resume game_dir with mismatched --arm, --table-id, --seed, --wrapper, --memory-mode, --seats, or --serving-manifest"
                    .to_owned(),
            ));
        }
        host
    } else if dir_has_eval_save_or_seal(&opts.game_dir)? {
        let host = EvalHost::fork_duplicate_from_game_dir(&opts.game_dir, TableId(opts.table_id))?;
        persist_eval_game_state(&host, &opts.game_dir)?;
        host
    } else {
        let bot_tie_seed = draw_os_bot_tie_seed()?;
        let host = EvalHost::from_standard_setup_with_model_seats(
            TableId(opts.table_id),
            4,
            opts.seed,
            bot_tie_seed,
            &model_seats,
        )?;
        persist_eval_game_state(&host, &opts.game_dir)?;
        host
    };

    let (has_regents, seat_regents) = seat_regent_flags(&host, &seats_file);
    ensure_no_model_seat_regent_at_startup(&host, &model_seats, &has_regents, &seats_file)?;

    let setup_record = EvalSetupFile {
        game_id: host.game_id().to_owned(),
        table_id: opts.table_id,
        seed: opts.seed,
        turns: opts.turns,
        seat_count: 4,
        arm: opts.arm,
        eval_reply_wording_arm: EVAL_REPLY_WORDING_ARM.to_owned(),
        relay_wording_arm: relay_wording_arm.clone(),
        wrapper_version: wrapper_version.clone(),
        wrapper_sha256: wrapper_sha256.clone(),
        memory_mode: mem_mode,
        seats_sha256,
        response_schema_sha256: Some(response_schema_sha256.clone()),
        serving_manifest_sha256,
    };
    write_json_pretty_atomic(&setup_path, &setup_record)?;

    let to_seat_dir = opts.game_dir.join("to-seat");
    let decisions_dir = opts.game_dir.join("decisions");
    let receipts_dir = opts.game_dir.join("receipts");
    let checkpoints_dir = opts.game_dir.join("checkpoints");
    let calls_path = opts.game_dir.join("relay").join("calls.jsonl");

    let mut played_turns = u8::try_from(host.public_receipts().len()).unwrap_or(u8::MAX);
    let (mut refusal_by_seat, mut refusal_by_arm, mut wrong_part_refusals) = if played_turns > 0 {
        let recounted = recount_manifest_counters_from_decisions(
            &decisions_dir,
            played_turns,
            &seats_file,
            opts.arm,
        )?;
        (recounted.by_seat, recounted.by_arm, recounted.wrong_part)
    } else {
        let (by_seat, by_arm) = init_refusal_maps(&seats_file, opts.arm);
        (by_seat, by_arm, WrongPartCounter::default())
    };

    let seated_guilds: Vec<String> = seats_file.seats.iter().map(|s| s.guild.clone()).collect();
    let bot = ClubBot::new(host.bot_tie_seed());

    while played_turns < opts.turns && host.phase() == TurnPhase::TalkAndMarket {
        let day = host.turn();
        ensure_no_partial_from_seat_for_day(&opts.game_dir, day)?;
        let day_str = format!("{day:02}");

        // --- TALK ROUND ---
        let mut whispers_by_guild: BTreeMap<String, Vec<UntrustedWhisper>> = BTreeMap::new();
        for g in &seated_guilds {
            whispers_by_guild.insert(g.clone(), Vec::new());
        }

        for (idx, seat) in seats_file.seats.iter().enumerate() {
            let obs = host.observe_seat(Seat(idx as u8))?;
            let prompt_md = render_seat_view_markdown(Round::Talk, &obs)?;
            write_text_atomic(
                &to_seat_dir.join(format!("day-{day_str}-talk-{}.md", seat.guild)),
                &prompt_md,
            )?;
        }

        let talk_cfg = TurnConfig::new(
            opts.game_dir.clone(),
            day_str.clone(),
            Round::Talk,
            seats_file.clone(),
            Some(opts.wrapper_path.clone()),
            opts.live,
            opts.cap_credits,
        )
        .with_cap_gpu_secs(opts.cap_gpu_secs)
        .with_memory_mode(opts.memory_mode)
        .with_response_format(Some(response_format.clone()))
        .with_preview_tool(preview_mode)
        .with_eval_mode(true)
        .with_allowed_vm_hosts(opts.allowed_vm_hosts.clone());

        let talk_calls_offset = count_calls_lines(&calls_path)?;
        let talk_summary = run_turn(&talk_cfg, transport, redaction_key)?;
        for (idx, seat) in seats_file.seats.iter().enumerate() {
            let keeps_notes = seat_keeps_notes(seat, &seats_file, opts.memory_mode);
            let record = match seat.backend {
                SeatBackend::File { .. } => empty_clubbot_decision(
                    host.game_id(),
                    opts.table_id,
                    day,
                    Round::Talk,
                    idx as u8,
                    &seat.guild,
                    opts.arm,
                    BTreeMap::new(),
                    None,
                ),
                SeatBackend::Local(_) | SeatBackend::Openrouter(_) => {
                    let diag = read_round_attempt_diagnostics(
                        &opts.game_dir,
                        &calls_path,
                        talk_calls_offset,
                        &day_str,
                        Round::Talk,
                        idx as u8,
                        &seat.guild,
                    )?;
                    resolve_talk_model_round(
                        host.game_id(),
                        opts.table_id,
                        day,
                        idx as u8,
                        &seat.guild,
                        opts.arm,
                        keeps_notes,
                        &seated_guilds,
                        find_report(&talk_summary, &seat.guild),
                        diag,
                        &mut whispers_by_guild,
                        &mut refusal_by_seat,
                        &mut refusal_by_arm,
                        &mut wrong_part_refusals,
                    )?
                }
            };
            write_json_pretty_atomic(
                &decisions_dir.join(format!("day-{day_str}-talk-{}.json", seat.guild)),
                &record,
            )?;
        }

        // Write `negotiation.json` for ALL 4 seats (`1.5`: every seat sees `eval_batna` on `act`).
        for seat in &seats_file.seats {
            let whispers = whispers_by_guild.remove(&seat.guild).unwrap_or_default();
            let neg = build_act_negotiation_inputs(whispers);
            write_json_pretty_atomic(
                &to_seat_dir.join(format!("day-{day_str}-act-{}.negotiation.json", seat.guild)),
                &neg,
            )?;
        }

        // --- PRE-ACT CHECKPOINT (`R9`: BEFORE act prompt or reply collection) ---
        host.write_pre_act_checkpoint(&checkpoints_dir)?;
        snapshot_pre_act_scratchpads(
            &opts.game_dir,
            &checkpoints_dir,
            day,
            &seats_file,
            opts.memory_mode,
        )?;

        // --- ACT ROUND ---
        let mut seat_observations = Vec::with_capacity(4);
        for (idx, seat) in seats_file.seats.iter().enumerate() {
            let obs = host.observe_seat(Seat(idx as u8))?;
            let act_md = render_seat_view_markdown(Round::Act, &obs)?;
            write_text_atomic(
                &to_seat_dir.join(format!("day-{day_str}-act-{}.md", seat.guild)),
                &act_md,
            )?;
            if opts.arm == EvalArm::On && !matches!(seat.backend, SeatBackend::File { .. }) {
                let preview_json = render_preview_state_json(&obs)?;
                write_text_atomic(
                    &to_seat_dir.join(format!(
                        "day-{day_str}-act-{}.preview-state.json",
                        seat.guild
                    )),
                    &preview_json,
                )?;
            }
            seat_observations.push(obs);
        }

        let act_cfg = TurnConfig::new(
            opts.game_dir.clone(),
            day_str.clone(),
            Round::Act,
            seats_file.clone(),
            Some(opts.wrapper_path.clone()),
            opts.live,
            opts.cap_credits,
        )
        .with_cap_gpu_secs(opts.cap_gpu_secs)
        .with_memory_mode(opts.memory_mode)
        .with_response_format(Some(response_format.clone()))
        .with_preview_tool(preview_mode)
        .with_eval_mode(true)
        .with_allowed_vm_hosts(opts.allowed_vm_hosts.clone());

        let act_calls_offset = count_calls_lines(&calls_path)?;
        let act_summary = run_turn(&act_cfg, transport, redaction_key)?;

        let mut submissions: BTreeMap<u8, EvalActSubmission> = BTreeMap::new();
        let mut baselines: Vec<EvalActSubmission> = Vec::with_capacity(4);
        let mut pending_act_records: Vec<EvalDecisionRecord> = Vec::with_capacity(4);

        for (idx, seat) in seats_file.seats.iter().enumerate() {
            let own_mandate = host.mandate_for_seat(Seat(idx as u8))?;
            let bot_dec = bot.decide_full_turn(&seat_observations[idx], own_mandate);
            let bot_sub = EvalActSubmission {
                market_actions: bot_dec.pass_a_actions,
                pass_b_actions: bot_dec.pass_b_actions,
                bundle: bot_dec.bundle,
            };
            baselines.push(bot_sub.clone());

            let keeps_notes = seat_keeps_notes(seat, &seats_file, opts.memory_mode);
            let (submission, record) = match seat.backend {
                SeatBackend::File { .. } => {
                    let rec = empty_clubbot_decision(
                        host.game_id(),
                        opts.table_id,
                        day,
                        Round::Act,
                        idx as u8,
                        &seat.guild,
                        opts.arm,
                        BTreeMap::new(),
                        Some(bot_sub.clone()),
                    );
                    (bot_sub, rec)
                }
                SeatBackend::Local(_) | SeatBackend::Openrouter(_) => {
                    let diag = read_round_attempt_diagnostics(
                        &opts.game_dir,
                        &calls_path,
                        act_calls_offset,
                        &day_str,
                        Round::Act,
                        idx as u8,
                        &seat.guild,
                    )?;
                    resolve_act_model_round(
                        host.game_id(),
                        opts.table_id,
                        day,
                        idx as u8,
                        &seat.guild,
                        opts.arm,
                        keeps_notes,
                        find_report(&act_summary, &seat.guild),
                        diag,
                        BTreeMap::new(),
                        bot_sub,
                        Some(&mut refusal_by_seat),
                        Some(&mut refusal_by_arm),
                        Some(&mut wrong_part_refusals),
                    )?
                }
            };
            submissions.insert(idx as u8, submission);
            pending_act_records.push(record);
        }

        let apply_outcome = host.apply_act_submissions(&submissions)?;
        host.write_act_recorded_seal(&checkpoints_dir)?;

        for mut rec in pending_act_records {
            let seat_idx = rec.seat;
            if let Some(report) = apply_outcome.seats.get(&seat_idx) {
                populate_act_apply_telemetry(
                    &mut rec,
                    &submissions[&seat_idx],
                    report,
                    opts.arm,
                    Some(&mut refusal_by_seat),
                    Some(&mut refusal_by_arm),
                )?;
            }
            write_json_pretty_atomic(
                &decisions_dir.join(format!("day-{day_str}-act-{}.json", rec.guild)),
                &rec,
            )?;
        }

        let receipt = host.sweep_turn()?;
        write_json_pretty_atomic(
            &receipts_dir.join(format!("day-{day_str}.receipt.json")),
            &receipt,
        )?;
        persist_eval_game_state(&host, &opts.game_dir)?;
        played_turns = u8::try_from(host.public_receipts().len()).unwrap_or(u8::MAX);

        let regent_hit = find_model_seat_regent_intervention(&receipt, &model_seats, &seats_file);
        let turn_regent_audit = build_turn_regent_audit(host.public_receipts());
        let voided = regent_hit.is_some();
        let void_error_code = regent_hit
            .as_ref()
            .map(|_| "MODEL_SEAT_REGENT_INTERVENTION".to_owned());
        let voided_games_by_arm =
            BTreeMap::from([(opts.arm.as_str().to_owned(), u64::from(voided))]);

        // Write `manifest.json` after every completed turn so counts and Regent audit stay current on disk.
        wrong_part_refusals.validate_rates()?;
        let turn_manifest = build_eval_manifest(
            host.game_id(),
            opts.table_id,
            opts.seed,
            opts.arm,
            &relay_wording_arm,
            &wrapper_version,
            &wrapper_sha256,
            &response_schema_sha256,
            &crate_git_sha,
            played_turns,
            &seats_file,
            opts.memory_mode,
            &has_regents,
            seat_regents.clone(),
            turn_regent_audit,
            voided,
            void_error_code,
            voided_games_by_arm,
            serving_record.clone(),
            serving_env_snapshot.clone(),
            &refusal_by_seat,
            &refusal_by_arm,
            &wrong_part_refusals,
        );
        write_json_pretty_atomic(&manifest_path, &turn_manifest)?;
        if let Some(msg) = regent_hit {
            return Err(EvalError::ModelSeatRegentIntervention(msg));
        }
    }

    if host.phase() == TurnPhase::AwaitingMandateReveal {
        let (scoreboard, replay_log, _predicates, metrics) = host.finalize_eval_season()?;
        let replayed: ReplayOutcome = replay_eval_season(&replay_log)?;
        if replayed.scoreboard != scoreboard {
            return Err(EvalError::Io(
                "replayed scoreboard diverged from live finalized scoreboard".to_owned(),
            ));
        }
        write_json_pretty_atomic(
            &opts
                .game_dir
                .join(format!("eval-{}.replay.json", opts.table_id)),
            &replay_log,
        )?;
        write_json_pretty_atomic(&opts.game_dir.join("scoreboard.json"), &scoreboard)?;
        write_json_pretty_atomic(&opts.game_dir.join("metrics.json"), &metrics)?;
        persist_eval_game_state(&host, &opts.game_dir)?;
    }

    let existing_regent_hit = host
        .public_receipts()
        .iter()
        .find_map(|r| find_model_seat_regent_intervention(r, &model_seats, &seats_file));
    let turn_regent_audit = build_turn_regent_audit(host.public_receipts());
    let voided = existing_regent_hit.is_some();
    let void_error_code = existing_regent_hit
        .as_ref()
        .map(|_| "MODEL_SEAT_REGENT_INTERVENTION".to_owned());
    let voided_games_by_arm = BTreeMap::from([(opts.arm.as_str().to_owned(), u64::from(voided))]);

    wrong_part_refusals.validate_rates()?;
    let manifest = build_eval_manifest(
        host.game_id(),
        opts.table_id,
        opts.seed,
        opts.arm,
        &relay_wording_arm,
        &wrapper_version,
        &wrapper_sha256,
        &response_schema_sha256,
        &crate_git_sha,
        played_turns,
        &seats_file,
        opts.memory_mode,
        &has_regents,
        seat_regents,
        turn_regent_audit,
        voided,
        void_error_code,
        voided_games_by_arm,
        serving_record,
        serving_env_snapshot,
        &refusal_by_seat,
        &refusal_by_arm,
        &wrong_part_refusals,
    );
    write_json_pretty_atomic(&manifest_path, &manifest)?;
    if let Some(msg) = existing_regent_hit {
        return Err(EvalError::ModelSeatRegentIntervention(msg));
    }
    Ok(manifest)
}

fn snapshot_pre_act_scratchpads(
    game_dir: &Path,
    checkpoints_dir: &Path,
    day: u8,
    seats_file: &SeatFile,
    memory_mode: MemoryMode,
) -> Result<(), EvalError> {
    let relay_seats_dir = game_dir.join("relay").join("seats");
    for seat in &seats_file.seats {
        if !seat_keeps_notes(seat, seats_file, memory_mode) {
            continue;
        }
        let guild = &seat.guild;
        let src = relay_seats_dir.join(format!("{guild}.scratchpad.md"));
        let content = if src.exists() {
            fs::read_to_string(&src)
                .map_err(|e| EvalError::Io(format!("read scratchpad {}: {e}", src.display())))?
        } else {
            String::new()
        };
        let dst = checkpoints_dir.join(format!("day-{day:02}-pre-act.{guild}.scratchpad.md"));
        write_text_atomic(&dst, &content)?;
    }
    Ok(())
}

fn ensure_arm_dir_empty(arm_dir: &Path) -> Result<(), EvalError> {
    if !arm_dir.exists() {
        return Ok(());
    }
    let mut entries = fs::read_dir(arm_dir)
        .map_err(|e| EvalError::Io(format!("read branch arm dir {}: {e}", arm_dir.display())))?;
    if entries
        .next()
        .transpose()
        .map_err(|e| EvalError::Io(format!("scan branch arm dir {}: {e}", arm_dir.display())))?
        .is_some()
    {
        return Err(EvalError::BranchArmDirNotEmpty(format!(
            "branch output directory {} already exists and is non-empty",
            arm_dir.display()
        )));
    }
    Ok(())
}

/// Evaluate one or more frozen pre-act checkpoints across independent arms (`off`, `control`, `on`).
///
/// Writes only spectator/public artifacts under `opts.out_dir` (never `.seal` files or secret bytes).
pub fn run_branch_with_transport(
    opts: &BranchOptions,
    transport: &dyn Transport,
    redaction_key: Option<&ApiKey>,
) -> Result<Vec<BranchArmOutcome>, EvalError> {
    if opts.arms.is_empty() {
        return Err(EvalError::Usage(
            "branch requires at least one `--arm` / `--arms` entry".to_owned(),
        ));
    }
    let mut active_arms_joined = Vec::with_capacity(opts.arms.len());
    for &arm in &opts.arms {
        active_arms_joined.push(arm_to_preview_mode(arm)?.wording_arm());
    }
    let active_arms_str = active_arms_joined.join(",");

    let (wrapper_version, wrapper_sha256) = validate_wrapper_and_extract(&opts.wrapper_path, None)?;
    let seats_sha256 = hash_file_sha256(&opts.seats_path, "seats file")?;
    let expected_memory_mode = memory_mode_str(opts.memory_mode);
    let seats_file =
        SeatFile::load_with_allowed_hosts(&opts.seats_path, redaction_key, &opts.allowed_vm_hosts)?;
    validate_eval_seats_file(&seats_file)?;
    let response_format = build_eval_response_format_config()?;
    let response_schema_sha256 = response_format.canonical_sha256();

    let targets: Vec<(PathBuf, String, PathBuf)> = match (&opts.checkpoint_prefix, &opts.list_path)
    {
        (Some(prefix), None) => {
            let Some(ref focal) = opts.focal_seat else {
                return Err(EvalError::Usage(
                    "branch with `--checkpoint` requires `--seat <guild|0..3>`".to_owned(),
                ));
            };
            vec![(prefix.clone(), focal.clone(), opts.out_dir.clone())]
        }
        (None, Some(list_path)) => {
            let raw = fs::read_to_string(list_path).map_err(|e| {
                EvalError::CheckpointNotFound(format!(
                    "read selection list {}: {e}",
                    list_path.display()
                ))
            })?;
            let list: SelectionList = serde_json::from_str(&raw).map_err(|e| {
                EvalError::Io(format!("parse selection list {}: {e}", list_path.display()))
            })?;
            if list.states.is_empty() {
                return Err(EvalError::Usage(format!(
                    "selection list {} contains zero states",
                    list_path.display()
                )));
            }
            list.states
                .into_iter()
                .enumerate()
                .map(|(i, s)| {
                    let target_out = opts
                        .out_dir
                        .join(format!("item-{i:03}-day-{:02}-{}", s.day, s.seat_guild));
                    (
                        expand_home_path(&s.checkpoint_prefix),
                        s.seat_guild,
                        target_out,
                    )
                })
                .collect()
        }
        _ => {
            return Err(EvalError::Usage(
                "branch requires exactly one of `--checkpoint` or `--list`".to_owned(),
            ));
        }
    };

    let has_model_focal = targets.iter().try_fold(false, |acc, (_, focal_spec, _)| {
        let idx = resolve_focal_seat(&seats_file, focal_spec)?;
        Ok::<bool, EvalError>(
            acc || !matches!(seats_file.seats[idx].backend, SeatBackend::File { .. }),
        )
    })?;
    let crate_git_sha = resolve_verified_crate_git_sha()?;
    let serving_record =
        resolve_serving_manifest_opt(opts.serving_manifest_path.as_deref(), has_model_focal)?;
    let serving_env_snapshot = capture_serving_env_snapshot()?;

    for (_, _, target_out_dir) in &targets {
        for &arm in &opts.arms {
            ensure_arm_dir_empty(&target_out_dir.join(arm.as_str()))?;
        }
    }
    let summary_path = opts.out_dir.join("branch_summary.json");
    if summary_path.exists() {
        return Err(EvalError::BranchSummaryExists(format!(
            "branch summary {} already exists; pass a fresh --out directory",
            summary_path.display()
        )));
    }

    let mut all_outcomes = Vec::new();
    let mut seat_regents = BTreeMap::new();
    let mut turn_regent_audit_by_arm: BTreeMap<String, Vec<TurnRegentAuditRecord>> =
        BTreeMap::new();
    let mut voided_games_by_arm: BTreeMap<String, u64> = opts
        .arms
        .iter()
        .map(|a| (a.as_str().to_owned(), 0))
        .collect();
    let mut first_void_error: Option<String> = None;

    for (prefix, focal_spec, target_out_dir) in targets {
        let target_run = branch_one_target(
            &prefix,
            &focal_spec,
            &target_out_dir,
            &seats_file,
            opts,
            &wrapper_version,
            &wrapper_sha256,
            expected_memory_mode,
            &seats_sha256,
            &response_format,
            &response_schema_sha256,
            &crate_git_sha,
            serving_record.as_ref(),
            &serving_env_snapshot,
            transport,
            redaction_key,
        )?;
        all_outcomes.extend(target_run.outcomes);
        seat_regents = target_run.seat_regents;
        for (arm_key, records) in target_run.turn_regent_audit_by_arm {
            turn_regent_audit_by_arm
                .entry(arm_key)
                .or_default()
                .extend(records);
        }
        for (arm_key, count) in target_run.voided_games_by_arm {
            *voided_games_by_arm.entry(arm_key).or_insert(0) += count;
        }
        if first_void_error.is_none() {
            first_void_error = target_run.first_void_error;
        }
    }

    let mut branch_summary: BranchSummary = summarize_branch_outcomes(&all_outcomes);
    branch_summary.voided_games_by_arm = voided_games_by_arm.clone();
    branch_summary.wrong_part_refusals.validate_rates()?;
    write_json_pretty_atomic(&summary_path, &branch_summary)?;

    let voided = first_void_error.is_some();
    let void_error_code = first_void_error
        .as_ref()
        .map(|_| "MODEL_SEAT_REGENT_INTERVENTION".to_owned());
    let branch_manifest = BranchManifest {
        crate_git_sha,
        wrapper_version: wrapper_version.clone(),
        wrapper_sha256: wrapper_sha256.clone(),
        pass_b_limit: PASS_B_LIMIT_STATEMENT.to_owned(),
        arms: opts.arms.clone(),
        wording_digests: Some(WordingArmDigests::compute(
            &active_arms_str,
            &wrapper_version,
            &wrapper_sha256,
            &response_schema_sha256,
        )),
        serving: serving_record,
        serving_env_snapshot,
        seat_regents,
        turn_regent_audit_by_arm,
        voided,
        void_error_code,
        voided_games_by_arm,
        summary: branch_summary,
    };
    write_json_pretty_atomic(&opts.out_dir.join("manifest.json"), &branch_manifest)?;
    if let Some(msg) = first_void_error {
        return Err(EvalError::ModelSeatRegentIntervention(msg));
    }
    Ok(all_outcomes)
}

struct BranchTargetResult {
    outcomes: Vec<BranchArmOutcome>,
    seat_regents: BTreeMap<String, bool>,
    turn_regent_audit_by_arm: BTreeMap<String, Vec<TurnRegentAuditRecord>>,
    voided_games_by_arm: BTreeMap<String, u64>,
    first_void_error: Option<String>,
}

#[allow(clippy::too_many_arguments)]
fn branch_one_target(
    checkpoint_prefix: &Path,
    focal_spec: &str,
    target_out_dir: &Path,
    seats_file: &SeatFile,
    opts: &BranchOptions,
    wrapper_version: &str,
    wrapper_sha256: &str,
    expected_memory_mode: &str,
    expected_seats_sha256: &str,
    response_format: &league_relay::ResponseFormatConfig,
    response_schema_sha256: &str,
    crate_git_sha: &str,
    serving_record: Option<&ServingManifestRecord>,
    serving_env_snapshot: &BTreeMap<String, String>,
    transport: &dyn Transport,
    redaction_key: Option<&ApiKey>,
) -> Result<BranchTargetResult, EvalError> {
    let save_path = append_extension(checkpoint_prefix, "save.json");
    let seal_path = append_extension(checkpoint_prefix, "seal");
    if !save_path.exists() || !seal_path.exists() {
        return Err(EvalError::CheckpointNotFound(format!(
            "missing pre-act checkpoint files {} / {}",
            save_path.display(),
            seal_path.display()
        )));
    }

    let probe_host = EvalHost::fork_from_checkpoint(&save_path, &seal_path)?;
    let day = probe_host.turn();
    let day_str = format!("{day:02}");
    let table_id = probe_host.table_id().0;
    let source_game_id = probe_host.game_id().to_owned();
    let (has_regents, seat_regents) = seat_regent_flags(&probe_host, seats_file);
    let model_seats: Vec<bool> = seats_file
        .seats
        .iter()
        .map(|s| !matches!(s.backend, SeatBackend::File { .. }))
        .collect();
    ensure_no_model_seat_regent_at_startup(&probe_host, &model_seats, &has_regents, seats_file)?;

    let recorded_seal_path =
        checkpoint_prefix.with_file_name(format!("day-{day_str}-act-recorded.seal"));
    if !recorded_seal_path.exists() {
        return Err(EvalError::CheckpointNotFound(format!(
            "missing recorded act seal {}",
            recorded_seal_path.display()
        )));
    }
    let recorded_subs = probe_host.read_recorded_act_submissions(&recorded_seal_path)?;

    let focal_idx = resolve_focal_seat(seats_file, focal_spec)?;
    let focal_seat_spec = &seats_file.seats[focal_idx];
    let focal_guild = focal_seat_spec.guild.clone();

    let Some(source_game_dir) = checkpoint_prefix.parent().and_then(|p| p.parent()) else {
        return Err(EvalError::MissingSourceSetup(format!(
            "cannot infer source game directory from checkpoint {}",
            checkpoint_prefix.display()
        )));
    };
    let source_setup_path = source_game_dir.join("setup.json");
    if !source_setup_path.exists() {
        return Err(EvalError::MissingSourceSetup(format!(
            "missing source setup.json at {}",
            source_setup_path.display()
        )));
    }
    let raw_setup = fs::read_to_string(&source_setup_path).map_err(|e| {
        EvalError::Io(format!(
            "read source setup {}: {e}",
            source_setup_path.display()
        ))
    })?;
    let source_setup: EvalSetupFile = serde_json::from_str(&raw_setup).map_err(|e| {
        EvalError::Io(format!(
            "parse source setup {}: {e}",
            source_setup_path.display()
        ))
    })?;
    if source_setup.game_id != source_game_id
        || source_setup.table_id != table_id
        || source_setup.wrapper_sha256 != wrapper_sha256
        || source_setup.memory_mode != expected_memory_mode
        || source_setup.seats_sha256 != expected_seats_sha256
    {
        return Err(EvalError::IncompatibleEvalSetup(format!(
            "branch options do not match source setup {}: check --wrapper, --memory-mode, and --seats",
            source_setup_path.display()
        )));
    }

    let neg_rel_path = format!("to-seat/day-{day_str}-act-{focal_guild}.negotiation.json");
    let neg_src_path = source_game_dir.join(&neg_rel_path);
    if !neg_src_path.exists() {
        return Err(EvalError::MissingBranchNegotiation(format!(
            "missing source negotiation file {}",
            neg_src_path.display()
        )));
    }
    let source_negotiation_json = fs::read_to_string(&neg_src_path).map_err(|e| {
        EvalError::Io(format!(
            "read source negotiation {}: {e}",
            neg_src_path.display()
        ))
    })?;
    let mut loaded_history_sha256 = BTreeMap::new();
    loaded_history_sha256.insert(neg_rel_path, sha256_hex(source_negotiation_json.as_bytes()));

    let is_model_focal = !matches!(focal_seat_spec.backend, SeatBackend::File { .. });
    let (source_transcript_entries, tx_sha_opt) = if is_model_focal {
        load_pre_act_transcript_prefix(source_game_dir, &focal_guild, day)?
    } else {
        (Vec::new(), None)
    };
    if let Some((rel, sha)) = tx_sha_opt {
        loaded_history_sha256.insert(rel, sha);
    }

    let keeps_focal_notes = seat_keeps_notes(focal_seat_spec, seats_file, opts.memory_mode);
    let source_scratchpad = if is_model_focal && keeps_focal_notes {
        let pad_rel = format!("checkpoints/day-{day_str}-pre-act.{focal_guild}.scratchpad.md");
        let Some(ckpts) = checkpoint_prefix.parent() else {
            return Err(EvalError::MissingBranchScratchpad(format!(
                "missing checkpoint parent directory for pre-act scratchpad '{pad_rel}'"
            )));
        };
        let ckpt_pad_path =
            ckpts.join(format!("day-{day_str}-pre-act.{focal_guild}.scratchpad.md"));
        if !ckpt_pad_path.exists() {
            return Err(EvalError::MissingBranchScratchpad(format!(
                "missing pre-act scratchpad {} required for branch",
                ckpt_pad_path.display()
            )));
        }
        let txt = fs::read_to_string(&ckpt_pad_path).map_err(|e| {
            EvalError::Io(format!(
                "read source scratchpad {}: {e}",
                ckpt_pad_path.display()
            ))
        })?;
        // Always record the relative checkpoint key in loaded_history_sha256, never a local path.
        loaded_history_sha256.insert(pad_rel, sha256_hex(txt.as_bytes()));
        Some(txt)
    } else {
        None
    };

    for &arm in &opts.arms {
        ensure_arm_dir_empty(&target_out_dir.join(arm.as_str()))?;
    }

    let bot = ClubBot::new(probe_host.bot_tie_seed());
    let mut outcomes = Vec::with_capacity(opts.arms.len());
    let mut turn_regent_audit_by_arm: BTreeMap<String, Vec<TurnRegentAuditRecord>> =
        BTreeMap::new();
    let mut voided_games_by_arm: BTreeMap<String, u64> = BTreeMap::new();
    let mut first_void_error: Option<String> = None;

    for &arm in &opts.arms {
        let preview_mode = arm_to_preview_mode(arm)?;
        let relay_wording_arm = preview_mode.wording_arm().to_owned();
        let arm_dir = target_out_dir.join(arm.as_str());
        fs::create_dir_all(&arm_dir).map_err(|e| {
            EvalError::Io(format!("create branch arm dir {}: {e}", arm_dir.display()))
        })?;

        let mut host = EvalHost::fork_from_checkpoint(&save_path, &seal_path)?;
        let obs = host.observe_seat(Seat(focal_idx as u8))?;
        let own_mandate = *host.mandate_for_seat(Seat(focal_idx as u8))?;
        let bot_dec = bot.decide_full_turn(&obs, &own_mandate);
        let bot_sub = EvalActSubmission {
            market_actions: bot_dec.pass_a_actions,
            pass_b_actions: bot_dec.pass_b_actions,
            bundle: bot_dec.bundle,
        };

        let keeps_notes = seat_keeps_notes(focal_seat_spec, seats_file, opts.memory_mode);
        let to_seat_dir = arm_dir.join("to-seat");
        let act_md = render_seat_view_markdown(Round::Act, &obs)?;
        write_text_atomic(
            &to_seat_dir.join(format!("day-{day_str}-act-{focal_guild}.md")),
            &act_md,
        )?;
        write_text_atomic(
            &to_seat_dir.join(format!("day-{day_str}-act-{focal_guild}.negotiation.json")),
            &source_negotiation_json,
        )?;

        let (focal_sub, mut decision_rec) =
            if matches!(focal_seat_spec.backend, SeatBackend::File { .. }) {
                let rec = empty_clubbot_decision(
                    &source_game_id,
                    table_id,
                    day,
                    Round::Act,
                    focal_idx as u8,
                    &focal_guild,
                    arm,
                    loaded_history_sha256.clone(),
                    Some(bot_sub.clone()),
                );
                (bot_sub, rec)
            } else {
                if arm == EvalArm::On {
                    let preview_json = render_preview_state_json(&obs)?;
                    write_text_atomic(
                        &to_seat_dir.join(format!(
                            "day-{day_str}-act-{focal_guild}.preview-state.json"
                        )),
                        &preview_json,
                    )?;
                }

                if !source_transcript_entries.is_empty() {
                    let mut jsonl = String::new();
                    for entry in &source_transcript_entries {
                        let line = serde_json::to_string(entry).map_err(|e| {
                            EvalError::Io(format!("serialize branch transcript entry: {e}"))
                        })?;
                        jsonl.push_str(&line);
                        jsonl.push('\n');
                    }
                    write_text_atomic(
                        &arm_dir
                            .join("relay")
                            .join("seats")
                            .join(format!("{focal_guild}.jsonl")),
                        &jsonl,
                    )?;
                }
                if let Some(ref pad_text) = source_scratchpad {
                    write_text_atomic(
                        &arm_dir
                            .join("relay")
                            .join("seats")
                            .join(format!("{focal_guild}.scratchpad.md")),
                        pad_text,
                    )?;
                }

                let branch_seats = build_single_focal_seat_file(seats_file, focal_idx);
                let branch_cfg = TurnConfig::new(
                    arm_dir.clone(),
                    day_str.clone(),
                    Round::Act,
                    branch_seats,
                    Some(opts.wrapper_path.clone()),
                    opts.live,
                    opts.cap_credits,
                )
                .with_cap_gpu_secs(opts.cap_gpu_secs)
                .with_memory_mode(opts.memory_mode)
                .with_response_format(Some(response_format.clone()))
                .with_preview_tool(preview_mode)
                .with_eval_mode(true)
                .with_allowed_vm_hosts(opts.allowed_vm_hosts.clone());

                let arm_calls_path = arm_dir.join("relay").join("calls.jsonl");
                let offset = count_calls_lines(&arm_calls_path)?;
                let summary = run_turn(&branch_cfg, transport, redaction_key)?;
                let diag = read_round_attempt_diagnostics(
                    &arm_dir,
                    &arm_calls_path,
                    offset,
                    &day_str,
                    Round::Act,
                    focal_idx as u8,
                    &focal_guild,
                )?;
                resolve_act_model_round(
                    &source_game_id,
                    table_id,
                    day,
                    focal_idx as u8,
                    &focal_guild,
                    arm,
                    keeps_notes,
                    find_report(&summary, &focal_guild),
                    diag,
                    loaded_history_sha256.clone(),
                    bot_sub,
                    None,
                    None,
                    None,
                )?
            };

        let mut spliced = recorded_subs.clone();
        spliced.insert(focal_idx as u8, focal_sub.clone());
        let apply_outcome = host.apply_act_submissions(&spliced)?;
        if let Some(report) = apply_outcome.seats.get(&(focal_idx as u8)) {
            populate_act_apply_telemetry(&mut decision_rec, &focal_sub, report, arm, None, None)?;
        }
        let receipt: PublicSweepReceipt = host.sweep_turn()?;
        let arm_audit = vec![TurnRegentAuditRecord {
            turn: receipt.turn,
            converted_to_ai: receipt.converted_to_ai.clone(),
            regent_acted_seats: receipt.regent_acted_seats.clone(),
        }];
        turn_regent_audit_by_arm
            .entry(arm.as_str().to_owned())
            .or_default()
            .extend(arm_audit.clone());
        let regent_hit = find_model_seat_regent_intervention(&receipt, &model_seats, seats_file);
        let arm_voided = regent_hit.is_some();
        *voided_games_by_arm
            .entry(arm.as_str().to_owned())
            .or_insert(0) += u64::from(arm_voided);
        if first_void_error.is_none() {
            first_void_error = regent_hit;
        }

        let arm_outcome = BranchArmOutcome {
            source_game_id: source_game_id.clone(),
            table_id,
            day,
            focal_seat: focal_idx as u8,
            focal_guild: focal_guild.clone(),
            arm,
            decision: decision_rec.clone(),
            receipt: receipt.clone(),
        };
        let arm_summary = summarize_branch_outcomes(std::slice::from_ref(&arm_outcome));
        let arm_manifest = build_eval_manifest(
            &source_game_id,
            table_id,
            source_setup.seed,
            arm,
            &relay_wording_arm,
            wrapper_version,
            wrapper_sha256,
            response_schema_sha256,
            crate_git_sha,
            1,
            seats_file,
            opts.memory_mode,
            &has_regents,
            seat_regents.clone(),
            arm_audit,
            arm_voided,
            arm_voided.then(|| "MODEL_SEAT_REGENT_INTERVENTION".to_owned()),
            BTreeMap::from([(arm.as_str().to_owned(), u64::from(arm_voided))]),
            serving_record.cloned(),
            serving_env_snapshot.clone(),
            &arm_summary.refusal_counts_by_seat,
            &arm_summary.refusal_counts_by_arm,
            &arm_summary.wrong_part_refusals,
        );
        write_json_pretty_atomic(&arm_dir.join("decision.json"), &decision_rec)?;
        write_json_pretty_atomic(&arm_dir.join("receipt.json"), &receipt)?;
        write_json_pretty_atomic(&arm_dir.join("outcome.json"), &arm_outcome)?;
        write_json_pretty_atomic(&arm_dir.join("manifest.json"), &arm_manifest)?;
        outcomes.push(arm_outcome);
    }

    Ok(BranchTargetResult {
        outcomes,
        seat_regents,
        turn_regent_audit_by_arm,
        voided_games_by_arm,
        first_void_error,
    })
}

fn build_single_focal_seat_file(source: &SeatFile, focal_idx: usize) -> SeatFile {
    let seats = source
        .seats
        .iter()
        .enumerate()
        .map(|(idx, s)| {
            if idx == focal_idx {
                s.clone()
            } else {
                SeatSpec {
                    guild: s.guild.clone(),
                    backend: SeatBackend::File {
                        model: Some("ClubBot".to_owned()),
                    },
                    scratchpad: None,
                }
            }
        })
        .collect();
    SeatFile {
        seats,
        scratchpad: source.scratchpad,
        mixed_scratchpad_arm: source.mixed_scratchpad_arm,
    }
}

type TranscriptPrefixWithDigest = (Vec<TranscriptEntry>, Option<(String, String)>);

fn all_prior_model_rounds_were_no_reply(
    source_game_dir: &Path,
    focal_guild: &str,
    branch_day: u8,
) -> bool {
    let decisions_dir = source_game_dir.join("decisions");
    let mut prior_rounds: Vec<(u8, Round)> = Vec::new();
    for d in 1..branch_day {
        prior_rounds.push((d, Round::Talk));
        prior_rounds.push((d, Round::Act));
    }
    prior_rounds.push((branch_day, Round::Talk));
    for (d, r) in prior_rounds {
        let dec_path = decisions_dir.join(format!("day-{d:02}-{r}-{focal_guild}.json"));
        let Ok(raw) = fs::read_to_string(&dec_path) else {
            return false;
        };
        let Ok(rec) = serde_json::from_str::<EvalDecisionRecord>(&raw) else {
            return false;
        };
        if !rec.no_reply {
            return false;
        }
    }
    true
}

fn load_pre_act_transcript_prefix(
    source_game_dir: &Path,
    focal_guild: &str,
    branch_day: u8,
) -> Result<TranscriptPrefixWithDigest, EvalError> {
    let rel_path = format!("relay/seats/{focal_guild}.jsonl");
    let path = source_game_dir.join(&rel_path);
    if !path.exists() {
        if all_prior_model_rounds_were_no_reply(source_game_dir, focal_guild, branch_day) {
            return Ok((Vec::new(), None));
        }
        return Err(EvalError::MissingBranchTranscript(format!(
            "missing pre-act transcript {} required for branch",
            path.display()
        )));
    }
    let raw = fs::read_to_string(&path)
        .map_err(|e| EvalError::Io(format!("read source transcript {}: {e}", path.display())))?;
    let file_digest = (rel_path, sha256_hex(raw.as_bytes()));
    let mut all_entries = Vec::new();
    for (idx, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let entry: TranscriptEntry = serde_json::from_str(line).map_err(|e| {
            EvalError::Io(format!(
                "parse source transcript {} line {}: {e}",
                path.display(),
                idx + 1
            ))
        })?;
        all_entries.push(entry);
    }
    if all_entries.is_empty() {
        return Ok((Vec::new(), Some(file_digest)));
    }

    let mut cursor = 0;
    let system_entry = if all_entries[0].role == "system" {
        cursor = 1;
        Some(all_entries[0].clone())
    } else {
        None
    };

    let mut filtered_pairs = Vec::new();
    for pair in all_entries[cursor..].as_chunks::<2>().0 {
        let user = &pair[0];
        let asst = &pair[1];
        let entry_day = user.day.parse::<u8>().unwrap_or(0);
        let strictly_before =
            entry_day < branch_day || (entry_day == branch_day && user.round == Round::Talk);
        if strictly_before {
            filtered_pairs.push(user.clone());
            filtered_pairs.push(asst.clone());
        }
    }

    if filtered_pairs.is_empty() {
        return Ok((Vec::new(), Some(file_digest)));
    }
    let mut result = Vec::with_capacity(filtered_pairs.len() + 1);
    if let Some(sys) = system_entry {
        result.push(sys);
    }
    result.extend(filtered_pairs);
    Ok((result, Some(file_digest)))
}

fn resolve_focal_seat(seats_file: &SeatFile, focal_spec: &str) -> Result<usize, EvalError> {
    if let Some(pos) = seats_file
        .seats
        .iter()
        .position(|s| s.guild.eq_ignore_ascii_case(focal_spec.trim()))
    {
        return Ok(pos);
    }
    if let Ok(idx) = focal_spec.trim().parse::<usize>()
        && idx < seats_file.seats.len()
    {
        return Ok(idx);
    }
    Err(EvalError::UnknownFocalSeat(format!(
        "focal seat '{focal_spec}' not found in seats.json"
    )))
}

fn append_extension(prefix: &Path, ext: &str) -> PathBuf {
    let mut os = prefix.as_os_str().to_os_string();
    os.push(".");
    os.push(ext);
    PathBuf::from(os)
}

#[cfg(test)]
mod tests {
    use super::*;
    use league_relay::CostUnit;
    use std::fmt::Display;

    fn must_ok<T, E: Display>(res: Result<T, E>, ctx: &str) -> T {
        match res {
            Ok(v) => v,
            Err(e) => panic!("{ctx}: {e}"),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn write_discarded_sidecar_for_test(
        run_root: &Path,
        guild: &str,
        day: &str,
        round: Round,
        attempt: u8,
        outcome: &str,
        reason: &str,
        reservation_id: &str,
        content: &str,
    ) -> (String, String) {
        let rec = SavedDiscardedReplyRecord {
            day: day.to_owned(),
            round,
            guild: guild.to_owned(),
            run_id: "test-run".to_owned(),
            reservation_id: reservation_id.to_owned(),
            request_body_sha256: "req".to_owned(),
            round_index: 0,
            attempt,
            outcome: outcome.to_owned(),
            discard_reason: reason.to_owned(),
            is_late: false,
            assistant_content: Some(content.to_owned()),
            assistant_content_sha256: Some(sha256_hex(content.as_bytes())),
            reasoning: None,
            raw_tool_calls: None,
            body_redacted: false,
        };
        let fname = format!("{guild}.day-{day}-{round}.discarded-round-0-attempt-{attempt}.json");
        let path = run_root.join("relay").join("seats").join(&fname);
        let Some(parent) = path.parent() else {
            panic!("discarded sidecar has no parent");
        };
        must_ok(fs::create_dir_all(parent), "mkdir seats");
        let json = must_ok(serde_json::to_string_pretty(&rec), "ser");
        must_ok(fs::write(&path, json.as_bytes()), "write discarded");
        let sha = sha256_hex(&must_ok(fs::read(&path), "read discarded"));
        (format!("relay/seats/{fname}"), sha)
    }

    #[allow(clippy::too_many_arguments)]
    fn call_log_row_for_test(
        day: &str,
        round: Round,
        guild: &str,
        attempt: u8,
        outcome: &str,
        error: Option<&str>,
        reply_sha256: Option<String>,
        reservation_id: &str,
    ) -> String {
        let entry = CallLogEntry {
            time_utc: "2026-11-21T00:00:00.000Z".to_owned(),
            day: day.to_owned(),
            round,
            guild: guild.to_owned(),
            attempt,
            trigger: if attempt == 1 {
                "turn".to_owned()
            } else {
                "harness".to_owned()
            },
            model_requested: "m".to_owned(),
            model_returned: Some("m".to_owned()),
            request_body_sha256: "req".to_owned(),
            http_reply_sha256: None,
            reply_sha256,
            prompt_tokens: Some(1),
            completion_tokens: Some(1),
            total_tokens: Some(2),
            reasoning_tokens: None,
            cost_unit: CostUnit::Credits,
            wall_secs: Some(0.01),
            cost: Some(0.0),
            charged_credits: 0.0,
            latency_ms: 1,
            outcome: outcome.to_owned(),
            error: error.map(str::to_owned),
            reservation_id: Some(reservation_id.to_owned()),
        };
        must_ok(serde_json::to_string(&entry), "ser call row")
    }

    #[test]
    fn first_attempt_failure_is_never_overwritten_by_later_attempt() {
        let run_root =
            std::env::temp_dir().join(format!("league_eval_driver_unit_{}", std::process::id()));
        let _ = fs::remove_dir_all(&run_root);
        must_ok(fs::create_dir_all(&run_root), "mkdir run_root");

        let wrong_talk = r#"{"messages":[],"market_actions":[],"bundle":{"unknown_field":1}}"#;
        let schema_reason = "structured output failed schema 'eval_reply_v2': '$.bundle' contains disallowed property 'unknown_field'";
        let (disc_rel_1, disc_sha_1) = write_discarded_sidecar_for_test(
            &run_root,
            "Foundry",
            "01",
            Round::Talk,
            1,
            "harness",
            schema_reason,
            "res-1",
            wrong_talk,
        );
        let (_disc_rel_2, disc_sha_2) = write_discarded_sidecar_for_test(
            &run_root,
            "Foundry",
            "01",
            Round::Talk,
            2,
            "NO_REPLY",
            "structured output invalid JSON (expected value)",
            "res-2",
            "not json",
        );

        // Case A: Attempt 1 failed with wrong-round + schema error, and a hypothetical attempt 2
        // succeeded and wrote `from-seat/`. Attempt 1's refusal must still be the recorded outcome.
        let from_seat_clean = run_root.join("from-seat/day-01-talk-Foundry.md");
        let Some(from_seat_parent) = from_seat_clean.parent() else {
            panic!("from-seat has no parent");
        };
        must_ok(fs::create_dir_all(from_seat_parent), "mkdir from-seat");
        must_ok(
            fs::write(
                &from_seat_clean,
                r#"{"messages":[{"to_guild":"Quarry","body":"hi"}],"market_actions":[],"bundle":{}}"#,
            ),
            "write from-seat",
        );

        let calls_path = run_root.join("relay/calls.jsonl");
        let rows_case_a = [
            call_log_row_for_test(
                "01",
                Round::Talk,
                "Foundry",
                1,
                "harness",
                Some(schema_reason),
                Some(disc_sha_1.clone()),
                "res-1",
            ),
            call_log_row_for_test("01", Round::Talk, "Foundry", 2, "OK", None, None, "res-2"),
        ];
        must_ok(
            fs::write(
                &calls_path,
                format!("{}\n{}\n", rows_case_a[0], rows_case_a[1]),
            ),
            "write calls A",
        );

        let diag_a = must_ok(
            read_round_attempt_diagnostics(
                &run_root,
                &calls_path,
                0,
                "01",
                Round::Talk,
                0,
                "Foundry",
            ),
            "diag A",
        );
        let clean_outcome = SeatOutcome::Delivered {
            attempts: 2,
            from_seat_path: from_seat_clean,
            transcript_path: run_root.join("relay/seats/Foundry.jsonl"),
            calls_path: calls_path.clone(),
            request_body_sha256: "req".to_owned(),
            reply_sha256: "ok".to_owned(),
        };
        let mut whispers = BTreeMap::new();
        whispers.insert("Quarry".to_owned(), Vec::new());
        let mut by_seat = BTreeMap::new();
        let mut by_arm = BTreeMap::new();
        let mut wp = WrongPartCounter::default();
        let rec_a = must_ok(
            resolve_talk_model_round(
                "g1",
                1,
                1,
                0,
                "Foundry",
                EvalArm::Off,
                false,
                &["Foundry".to_owned(), "Quarry".to_owned()],
                Some(&clean_outcome),
                diag_a,
                &mut whispers,
                &mut by_seat,
                &mut by_arm,
                &mut wp,
            ),
            "resolve A",
        );
        assert!(
            rec_a.no_reply,
            "attempt 1 failure must block attempt 2 delivery"
        );
        assert!(whispers["Quarry"].is_empty());
        assert_eq!(rec_a.refusal_code.as_deref(), Some("TALK_REPLY_HAS_ORDERS"));
        assert_eq!(
            rec_a.discarded_reply_path.as_deref(),
            Some(disc_rel_1.as_str())
        );
        assert_eq!(
            rec_a.discarded_reply_sha256.as_deref(),
            Some(disc_sha_1.as_str())
        );
        assert_eq!(wp.talk_reply_has_orders, 1);

        // Case B: Attempt 1 failed with wrong-round + schema error and attempt 2 failed with
        // non-JSON `NO_REPLY`: attempt 1 (`TALK_REPLY_HAS_ORDERS`, sidecar 1) wins over
        // attempt 2 (`REPLY_SCHEMA_INVALID`, sidecar 2).
        let rows_case_b = [
            call_log_row_for_test(
                "01",
                Round::Talk,
                "Foundry",
                1,
                "harness",
                Some(schema_reason),
                Some(disc_sha_1.clone()),
                "res-1",
            ),
            call_log_row_for_test(
                "01",
                Round::Talk,
                "Foundry",
                2,
                "NO_REPLY",
                Some("structured output invalid JSON (expected value)"),
                Some(disc_sha_2),
                "res-2",
            ),
        ];
        must_ok(
            fs::write(
                &calls_path,
                format!("{}\n{}\n", rows_case_b[0], rows_case_b[1]),
            ),
            "write calls B",
        );
        let diag_b = must_ok(
            read_round_attempt_diagnostics(
                &run_root,
                &calls_path,
                0,
                "01",
                Round::Talk,
                0,
                "Foundry",
            ),
            "diag B",
        );
        let no_reply_outcome = SeatOutcome::NoReply {
            calls_path: calls_path.clone(),
            request_body_sha256: "req".to_owned(),
        };
        let rec_b = must_ok(
            resolve_talk_model_round(
                "g1",
                1,
                1,
                0,
                "Foundry",
                EvalArm::On,
                false,
                &["Foundry".to_owned(), "Quarry".to_owned()],
                Some(&no_reply_outcome),
                diag_b,
                &mut whispers,
                &mut by_seat,
                &mut by_arm,
                &mut wp,
            ),
            "resolve B",
        );
        assert_eq!(rec_b.refusal_code.as_deref(), Some("TALK_REPLY_HAS_ORDERS"));
        assert_eq!(
            rec_b.discarded_reply_path.as_deref(),
            Some(disc_rel_1.as_str())
        );
        assert_eq!(
            rec_b.discarded_reply_sha256.as_deref(),
            Some(disc_sha_1.as_str())
        );

        // Case C: `resolve_act_model_round` likewise locks on attempt 1 even if attempt 2 delivered.
        let wrong_act = r#"{"messages":[{"to_guild":"Quarry","body":"hi"}],"market_actions":[],"bundle":{},"unknown_key":1}"#;
        let act_schema_reason = "structured output failed schema 'eval_reply_v2': '$' contains disallowed property 'unknown_key'";
        let (act_disc_rel_1, act_disc_sha_1) = write_discarded_sidecar_for_test(
            &run_root,
            "Foundry",
            "01",
            Round::Act,
            1,
            "harness",
            act_schema_reason,
            "res-act-1",
            wrong_act,
        );
        let from_seat_act_clean = run_root.join("from-seat/day-01-act-Foundry.md");
        must_ok(
            fs::write(
                &from_seat_act_clean,
                r#"{"messages":[],"market_actions":[],"bundle":{"debt_repayment_cred":5}}"#,
            ),
            "write clean act from-seat",
        );
        let rows_case_c = [
            call_log_row_for_test(
                "01",
                Round::Act,
                "Foundry",
                1,
                "harness",
                Some(act_schema_reason),
                Some(act_disc_sha_1.clone()),
                "res-act-1",
            ),
            call_log_row_for_test(
                "01",
                Round::Act,
                "Foundry",
                2,
                "OK",
                None,
                None,
                "res-act-2",
            ),
        ];
        must_ok(
            fs::write(
                &calls_path,
                format!("{}\n{}\n", rows_case_c[0], rows_case_c[1]),
            ),
            "write calls C",
        );
        let diag_c = must_ok(
            read_round_attempt_diagnostics(
                &run_root,
                &calls_path,
                0,
                "01",
                Round::Act,
                0,
                "Foundry",
            ),
            "diag C",
        );
        let act_clean_outcome = SeatOutcome::Delivered {
            attempts: 2,
            from_seat_path: from_seat_act_clean,
            transcript_path: run_root.join("relay/seats/Foundry.jsonl"),
            calls_path,
            request_body_sha256: "req".to_owned(),
            reply_sha256: "ok".to_owned(),
        };
        let (sub_c, rec_c) = must_ok(
            resolve_act_model_round(
                "g1",
                1,
                1,
                0,
                "Foundry",
                EvalArm::Off,
                false,
                Some(&act_clean_outcome),
                diag_c,
                BTreeMap::new(),
                EvalActSubmission::default(),
                Some(&mut by_seat),
                Some(&mut by_arm),
                Some(&mut wp),
            ),
            "resolve C",
        );
        assert_eq!(sub_c, EvalActSubmission::default());
        assert!(rec_c.no_reply);
        assert_eq!(
            rec_c.refusal_code.as_deref(),
            Some("ACT_REPLY_HAS_MESSAGES")
        );
        assert_eq!(
            rec_c.discarded_reply_path.as_deref(),
            Some(act_disc_rel_1.as_str())
        );
        assert_eq!(
            rec_c.discarded_reply_sha256.as_deref(),
            Some(act_disc_sha_1.as_str())
        );
        assert_eq!(wp.act_reply_has_messages, 1);

        let _ = fs::remove_dir_all(&run_root);
    }

    #[test]
    fn wrong_round_runs_on_any_discarded_first_failure_including_late_discarded() {
        let run_root = std::env::temp_dir().join(format!(
            "league_eval_late_wrong_round_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&run_root);
        must_ok(fs::create_dir_all(&run_root), "mkdir run_root");

        let late_talk_with_orders = r#"{"messages":[],"market_actions":[{"action_kind":"town_import","good":{"asset_kind":"raw","good_index":0},"amount":10}],"bundle":{}}"#;
        let late_reason = "late HTTP reply arrived after 5.000s deadline";
        let (disc_rel, disc_sha) = write_discarded_sidecar_for_test(
            &run_root,
            "Foundry",
            "01",
            Round::Talk,
            1,
            "LATE_DISCARDED",
            late_reason,
            "res-late-1",
            late_talk_with_orders,
        );
        let calls_path = run_root.join("relay/calls.jsonl");
        let row = call_log_row_for_test(
            "01",
            Round::Talk,
            "Foundry",
            1,
            "LATE_DISCARDED",
            Some(late_reason),
            Some(disc_sha.clone()),
            "res-late-1",
        );
        must_ok(
            fs::write(&calls_path, format!("{row}\n")),
            "write late call",
        );

        let diag = must_ok(
            read_round_attempt_diagnostics(
                &run_root,
                &calls_path,
                0,
                "01",
                Round::Talk,
                0,
                "Foundry",
            ),
            "diag late",
        );
        let outcome = SeatOutcome::NoReply {
            calls_path,
            request_body_sha256: "req".to_owned(),
        };
        let mut whispers = BTreeMap::new();
        whispers.insert("Quarry".to_owned(), Vec::new());
        let mut by_seat = BTreeMap::new();
        let mut by_arm = BTreeMap::new();
        let mut wp = WrongPartCounter::default();
        let rec = must_ok(
            resolve_talk_model_round(
                "g-late",
                1,
                1,
                0,
                "Foundry",
                EvalArm::Off,
                false,
                &["Foundry".to_owned(), "Quarry".to_owned()],
                Some(&outcome),
                diag,
                &mut whispers,
                &mut by_seat,
                &mut by_arm,
                &mut wp,
            ),
            "resolve late",
        );
        assert!(rec.no_reply);
        assert_eq!(rec.refusal_code.as_deref(), Some("TALK_REPLY_HAS_ORDERS"));
        assert_eq!(rec.discarded_reply_path.as_deref(), Some(disc_rel.as_str()));
        assert_eq!(
            rec.discarded_reply_sha256.as_deref(),
            Some(disc_sha.as_str())
        );
        assert_eq!(wp.talk_reply_has_orders, 1);
        assert_eq!(wp.no_reply_wrong_part_refusals, 1);
        assert_eq!(wp.delivered_wrong_part_refusals, 0);
        assert_eq!(wp.wrong_part_rate_delivered, 0.0);
        assert!((wp.wrong_part_rate_all_rounds - 1.0).abs() < 1e-12);
        must_ok(wp.validate_rates(), "validate late rates");

        // Rate > 1.0 is refused cleanly by `try_recompute_rates` / `validate_rates`, never clamped.
        let mut bad = WrongPartCounter {
            talk_reply_has_orders: 2,
            act_reply_has_messages: 0,
            delivered_wrong_part_refusals: 2,
            no_reply_wrong_part_refusals: 0,
            total_wrong_part_refusals: 2,
            delivered_model_replies: 1,
            total_model_rounds: 1,
            wrong_part_rate_delivered: 0.0,
            wrong_part_rate_all_rounds: 0.0,
            exceeds_five_percent_preflight_stop_line_delivered: false,
            exceeds_five_percent_preflight_stop_line_all_rounds: false,
        };
        let err = match bad.try_recompute_rates() {
            Err(e) => e,
            Ok(()) => panic!("rate > 1.0 must be refused, never clamped"),
        };
        assert_eq!(err.code(), "INVALID_WRONG_PART_RATE");

        let _ = fs::remove_dir_all(&run_root);
    }

    #[test]
    fn fix2_checked_sub_refuses_refused_exceeding_submitted() {
        let mut rec = empty_clubbot_decision(
            "g-count",
            1,
            1,
            Round::Act,
            0,
            "Foundry",
            EvalArm::Off,
            BTreeMap::new(),
            None,
        );
        let empty_sub = EvalActSubmission::default();
        let bad_pass_a = league_core::EvalSeatApplyReport {
            refused_market_actions: vec![(0, "BAD".to_owned())],
            refused_pass_b_actions: Vec::new(),
            refused_bundle_code: None,
        };
        let err_a = match populate_act_apply_telemetry(
            &mut rec,
            &empty_sub,
            &bad_pass_a,
            EvalArm::Off,
            None,
            None,
        ) {
            Err(e) => e,
            Ok(()) => panic!("refused_market_actions > market_actions must return Err"),
        };
        assert_eq!(err_a.code(), "INVALID_ACTION_COUNT");

        let bad_pass_b = league_core::EvalSeatApplyReport {
            refused_market_actions: Vec::new(),
            refused_pass_b_actions: vec![(0, "BAD_B".to_owned())],
            refused_bundle_code: None,
        };
        let err_b = match populate_act_apply_telemetry(
            &mut rec,
            &empty_sub,
            &bad_pass_b,
            EvalArm::Off,
            None,
            None,
        ) {
            Err(e) => e,
            Ok(()) => panic!("refused_pass_b_actions > pass_b_actions must return Err"),
        };
        assert_eq!(err_b.code(), "INVALID_ACTION_COUNT");
    }

    #[test]
    fn fix2_s8_startup_check_refuses_prior_receipt_even_with_has_regents_false() {
        let tmp_seats = std::env::temp_dir().join(format!(
            "ml_driver_fix2_s8_seats_{}.json",
            std::process::id()
        ));
        let seats_json = serde_json::json!({
            "seats": [
                {
                    "guild": "Foundry",
                    "backend": "local",
                    "base_url": "http://127.0.0.1:18080/v1",
                    "model": "Qwen/Qwen3-32B",
                    "temperature": 0.0,
                    "max_output_tokens": 512,
                    "timeout_secs": 5.0,
                    "max_gpu_secs_per_call": 10.0
                },
                { "guild": "Quarry", "backend": "file", "model": "ClubBot" },
                { "guild": "Harbor", "backend": "file", "model": "ClubBot" },
                { "guild": "Mint", "backend": "file", "model": "ClubBot" }
            ]
        });
        must_ok(
            fs::write(&tmp_seats, seats_json.to_string()),
            "write seat file",
        );
        let seats = must_ok(
            SeatFile::load_with_allowed_hosts(&tmp_seats, None, &[]),
            "load seat file",
        );
        let _ = fs::remove_file(&tmp_seats);
        let mut host = must_ok(
            EvalHost::from_standard_setup_with_model_seats(TableId(98), 4, 7, 1, &[false; 4]),
            "create host",
        );
        for _ in 0..4 {
            let _ = must_ok(
                host.apply_act_submissions(&BTreeMap::new()),
                "apply empty turn",
            );
            let _ = must_ok(host.sweep_turn(), "sweep empty turn");
        }
        assert!(
            host.public_receipts()
                .iter()
                .any(|r| r.regent_acted_seats.contains(&0) || r.converted_to_ai.contains(&0)),
            "expected prior turn receipt with seat 0 Regent hit"
        );
        let err = match ensure_no_model_seat_regent_at_startup(
            &host,
            &[true, false, false, false],
            &[false; 4],
            &seats,
        ) {
            Err(e) => e,
            Ok(()) => {
                panic!("prior Regent hit on model seat must refuse even if has_regents is false")
            }
        };
        assert_eq!(err.code(), "MODEL_SEAT_REGENT_INTERVENTION");
        assert!(
            err.to_string()
                .contains("startup check refused already-voided run"),
            "unexpected message: {err}"
        );
    }
}
