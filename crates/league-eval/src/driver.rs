//! Core Evaluation Mode driver (`play`, `branch`, `select` helper) for `league-eval`.
//!
//! Coordinates [`league_core::EvalHost`], [`league_core::ClubBot`], and `league-relay` without ever
//! invoking non-eval host APIs (`ClubHost`), `write_save_and_seal` (`R8`), or `engine-internals`.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use league_core::{
    ClubBot, EvalActSubmission, EvalHost, EvalSeatApplyReport, HARD_TURN_CAP, PublicSweepReceipt,
    ReplayOutcome, Seat, TableId, TurnPhase, replay_eval_season,
};
use league_relay::{
    ApiKey, CallLogEntry, MemoryMode, PreviewToolMode, RawHttpResponse, Round, SeatBackend,
    SeatFile, SeatOutcome, SeatSpec, TranscriptEntry, Transport, TransportError, TurnConfig,
    TurnSummary, UntrustedWhisper, run_turn, sha256_hex,
};
use serde::Serialize;

use crate::EvalError;
use crate::records::{
    BranchArmOutcome, BranchSummary, EvalArm, EvalDecisionRecord, EvalManifest, EvalManifestSeat,
    EvalSetupFile, FailedRelayAttempt, PASS_B_LIMIT_STATEMENT, RefusalCodeCounts,
    RefusedBundleRecord, WrongPartCounter, extract_wrapper_metadata, summarize_branch_outcomes,
};
use crate::reply::{ReplyRefusalCode, parse_eval_act_reply, parse_eval_talk_reply};
use crate::schema::build_eval_response_format_config;
use crate::select::{SelectionList, expand_home_path};
use crate::view::{
    build_act_negotiation_inputs, render_preview_state_json, render_seat_view_markdown,
};
use crate::wording::EVAL_REPLY_WORDING_ARM;

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
    /// Path to 4-seat `seats.json` (`D6`).
    pub seats_path: PathBuf,
    /// Path to seat wrapper markdown brief (`1.7`).
    pub wrapper_path: PathBuf,
    /// Optional explicit `--wrapper-version` string override (`1.7`).
    pub wrapper_version: Option<String>,
    /// Evaluation arm (`off` or `on`; `control` rejected in PR 2).
    pub arm: EvalArm,
    /// Table identifier (`TableId`).
    pub table_id: u64,
    /// Table layout seed passed to `EvalHost::from_standard_setup`.
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
    /// Path to seat wrapper markdown brief (`1.7`).
    pub wrapper_path: PathBuf,
    /// Arms to evaluate from the frozen pre-act state (e.g. `[Off, On]`).
    pub arms: Vec<EvalArm>,
    /// Root output directory where only spectator/public branch artifacts are written (`Item 15`).
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
        EvalArm::Control => Err(EvalError::ControlArmNotImplemented),
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

#[derive(Debug, Default)]
struct RoundAttemptDiagnostics {
    attempts: u32,
    failed_attempts: Vec<FailedRelayAttempt>,
    schema_failure_reason: Option<String>,
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

fn read_round_attempt_diagnostics(
    calls_path: &Path,
    start_line: usize,
    day_str: &str,
    round: Round,
    guild: &str,
) -> Result<RoundAttemptDiagnostics, EvalError> {
    if !calls_path.exists() {
        return Ok(RoundAttemptDiagnostics::default());
    }
    let raw = fs::read_to_string(calls_path)
        .map_err(|e| EvalError::Io(format!("read {}: {e}", calls_path.display())))?;
    let mut out = RoundAttemptDiagnostics::default();
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
        if entry.outcome != "RESERVED" && entry.outcome != "CAP_REACHED" {
            out.attempts = out.attempts.saturating_add(1);
        }
        let is_success =
            entry.error.is_none() && matches!(entry.outcome.as_str(), "OK" | "TOOL_CALL");
        if !is_success {
            let reason = entry.error.clone().unwrap_or_else(|| entry.outcome.clone());
            if is_relay_schema_or_json_error(&reason) {
                out.schema_failure_reason = Some(reason.clone());
            }
            out.failed_attempts.push(FailedRelayAttempt {
                attempt: entry.attempt,
                outcome: entry.outcome,
                reason,
            });
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
    let refusal_code = diag
        .schema_failure_reason
        .as_ref()
        .map(|_| ReplyRefusalCode::ReplySchemaInvalid);
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
        wp.record_no_reply();
    }
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
        notes_present: false,
        raw_reply: None,
        attempts: diag.attempts,
        failed_attempts: diag.failed_attempts,
        loaded_history_sha256,
        refusal_code: refusal_code.map(|c| c.as_str().to_owned()),
        refusal_reason: diag.schema_failure_reason,
        invalid_recipients: Vec::new(),
        no_reply: true,
        apply_report: None,
        refused_bundle: None,
        club_bot_baseline,
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
                    if rec.no_reply {
                        wp.record_no_reply();
                    } else {
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
                        wp.record_delivered_reply(wrong_part);
                    }
                }
            }
        }
    }
    Ok(RecountedManifestCounters {
        by_seat,
        by_arm,
        wrong_part: wp,
    })
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
    played_turns: u8,
    seats_file: &SeatFile,
    memory_mode: MemoryMode,
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
        seats: manifest_seats,
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
    Ok(extract_wrapper_metadata(&raw_bytes, explicit_version))
}

fn seat_kind_label(seat: &SeatSpec) -> &'static str {
    match seat.backend {
        SeatBackend::File { .. } => "ClubBot",
        SeatBackend::Local(_) | SeatBackend::Openrouter(_) => "Model",
    }
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
        {
            return Err(EvalError::IncompatibleEvalSetup(
                "cannot resume game_dir with mismatched --arm, --table-id, --seed, --wrapper, --memory-mode, or --seats"
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
        let host =
            EvalHost::from_standard_setup(TableId(opts.table_id), 4, opts.seed, bot_tie_seed)?;
        persist_eval_game_state(&host, &opts.game_dir)?;
        host
    };

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
        response_schema_sha256: Some(response_schema_sha256),
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
        .with_allowed_vm_hosts(opts.allowed_vm_hosts.clone());

        let talk_calls_offset = count_calls_lines(&calls_path)?;
        let talk_summary = run_turn(&talk_cfg, transport, redaction_key)?;
        for (idx, seat) in seats_file.seats.iter().enumerate() {
            let keeps_notes = seat_keeps_notes(seat, &seats_file, opts.memory_mode);
            let record = match seat.backend {
                SeatBackend::File { .. } => EvalDecisionRecord {
                    game_id: host.game_id().to_owned(),
                    table_id: opts.table_id,
                    day,
                    round: Round::Talk,
                    seat: idx as u8,
                    guild: seat.guild.clone(),
                    arm: opts.arm,
                    seat_kind: seat_kind_label(seat).to_owned(),
                    keeps_notes: false,
                    notes_present: false,
                    raw_reply: None,
                    attempts: 0,
                    failed_attempts: Vec::new(),
                    loaded_history_sha256: BTreeMap::new(),
                    refusal_code: None,
                    refusal_reason: None,
                    invalid_recipients: Vec::new(),
                    no_reply: false,
                    apply_report: None,
                    refused_bundle: None,
                    club_bot_baseline: None,
                },
                SeatBackend::Local(_) | SeatBackend::Openrouter(_) => {
                    let diag = read_round_attempt_diagnostics(
                        &calls_path,
                        talk_calls_offset,
                        &day_str,
                        Round::Talk,
                        &seat.guild,
                    )?;
                    match find_report(&talk_summary, &seat.guild) {
                        Some(SeatOutcome::Delivered { from_seat_path, .. }) => {
                            let raw_reply = fs::read_to_string(from_seat_path).map_err(|e| {
                                EvalError::Io(format!(
                                    "read reply {}: {e}",
                                    from_seat_path.display()
                                ))
                            })?;
                            let parsed = parse_eval_talk_reply(
                                &raw_reply,
                                idx as u8,
                                &seat.guild,
                                &seated_guilds,
                                keeps_notes,
                            );
                            let wrong_part = parsed
                                .refusal_code
                                .filter(|&c| c == ReplyRefusalCode::TalkReplyHasOrders);
                            wrong_part_refusals.record_delivered_reply(wrong_part);

                            let seat_counts =
                                refusal_by_seat.entry(seat.guild.clone()).or_default();
                            let arm_counts = refusal_by_arm
                                .entry(opts.arm.as_str().to_owned())
                                .or_default();
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
                            EvalDecisionRecord {
                                game_id: host.game_id().to_owned(),
                                table_id: opts.table_id,
                                day,
                                round: Round::Talk,
                                seat: idx as u8,
                                guild: seat.guild.clone(),
                                arm: opts.arm,
                                seat_kind: seat_kind_label(seat).to_owned(),
                                keeps_notes,
                                notes_present: parsed.notes_present,
                                raw_reply: Some(raw_reply),
                                attempts: diag.attempts,
                                failed_attempts: diag.failed_attempts,
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
                            }
                        }
                        _ => build_no_reply_record(
                            host.game_id(),
                            opts.table_id,
                            day,
                            Round::Talk,
                            idx as u8,
                            &seat.guild,
                            opts.arm,
                            keeps_notes,
                            diag,
                            BTreeMap::new(),
                            None,
                            Some(&mut refusal_by_seat),
                            Some(&mut refusal_by_arm),
                            Some(&mut wrong_part_refusals),
                        ),
                    }
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
        snapshot_pre_act_scratchpads(&opts.game_dir, &checkpoints_dir, day, &seated_guilds)?;

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
                    let rec = EvalDecisionRecord {
                        game_id: host.game_id().to_owned(),
                        table_id: opts.table_id,
                        day,
                        round: Round::Act,
                        seat: idx as u8,
                        guild: seat.guild.clone(),
                        arm: opts.arm,
                        seat_kind: seat_kind_label(seat).to_owned(),
                        keeps_notes: false,
                        notes_present: false,
                        raw_reply: None,
                        attempts: 0,
                        failed_attempts: Vec::new(),
                        loaded_history_sha256: BTreeMap::new(),
                        refusal_code: None,
                        refusal_reason: None,
                        invalid_recipients: Vec::new(),
                        no_reply: false,
                        apply_report: None,
                        refused_bundle: None,
                        club_bot_baseline: Some(bot_sub.clone()),
                    };
                    (bot_sub, rec)
                }
                SeatBackend::Local(_) | SeatBackend::Openrouter(_) => {
                    let diag = read_round_attempt_diagnostics(
                        &calls_path,
                        act_calls_offset,
                        &day_str,
                        Round::Act,
                        &seat.guild,
                    )?;
                    match find_report(&act_summary, &seat.guild) {
                        Some(SeatOutcome::Delivered { from_seat_path, .. }) => {
                            let raw_reply = fs::read_to_string(from_seat_path).map_err(|e| {
                                EvalError::Io(format!(
                                    "read reply {}: {e}",
                                    from_seat_path.display()
                                ))
                            })?;
                            let parsed = parse_eval_act_reply(&raw_reply, idx as u8, keeps_notes);
                            let wrong_part = parsed
                                .refusal_code
                                .filter(|&c| c == ReplyRefusalCode::ActReplyHasMessages);
                            wrong_part_refusals.record_delivered_reply(wrong_part);

                            if let Some(code) = parsed.refusal_code {
                                refusal_by_seat
                                    .entry(seat.guild.clone())
                                    .or_default()
                                    .record(code);
                                refusal_by_arm
                                    .entry(opts.arm.as_str().to_owned())
                                    .or_default()
                                    .record(code);
                            }

                            let rec = EvalDecisionRecord {
                                game_id: host.game_id().to_owned(),
                                table_id: opts.table_id,
                                day,
                                round: Round::Act,
                                seat: idx as u8,
                                guild: seat.guild.clone(),
                                arm: opts.arm,
                                seat_kind: seat_kind_label(seat).to_owned(),
                                keeps_notes,
                                notes_present: parsed.notes_present,
                                raw_reply: Some(raw_reply),
                                attempts: diag.attempts,
                                failed_attempts: diag.failed_attempts,
                                loaded_history_sha256: BTreeMap::new(),
                                refusal_code: parsed.refusal_code.map(|c| c.as_str().to_owned()),
                                refusal_reason: parsed.refusal_reason,
                                invalid_recipients: Vec::new(),
                                no_reply: false,
                                apply_report: None,
                                refused_bundle: None,
                                club_bot_baseline: Some(bot_sub),
                            };
                            (parsed.submission, rec)
                        }
                        _ => (
                            EvalActSubmission::default(),
                            build_no_reply_record(
                                host.game_id(),
                                opts.table_id,
                                day,
                                Round::Act,
                                idx as u8,
                                &seat.guild,
                                opts.arm,
                                keeps_notes,
                                diag,
                                BTreeMap::new(),
                                Some(bot_sub),
                                Some(&mut refusal_by_seat),
                                Some(&mut refusal_by_arm),
                                Some(&mut wrong_part_refusals),
                            ),
                        ),
                    }
                }
            };
            submissions.insert(idx as u8, submission);
            pending_act_records.push(record);
        }

        let apply_outcome = host.apply_act_submissions(&submissions)?;
        host.write_act_recorded_seal(&checkpoints_dir)?;

        for mut rec in pending_act_records {
            if let Some(report) = apply_outcome.seats.get(&rec.seat) {
                rec.refused_bundle = build_refused_bundle_record(
                    &submissions[&rec.seat],
                    report,
                    opts.arm,
                    &rec.guild,
                    &mut refusal_by_seat,
                    &mut refusal_by_arm,
                );
                rec.apply_report = Some(report.clone());
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

        // Write `manifest.json` after every completed turn so counts stay current on disk (`B5`).
        let turn_manifest = build_eval_manifest(
            host.game_id(),
            opts.table_id,
            opts.seed,
            opts.arm,
            &relay_wording_arm,
            &wrapper_version,
            &wrapper_sha256,
            played_turns,
            &seats_file,
            opts.memory_mode,
            &refusal_by_seat,
            &refusal_by_arm,
            &wrong_part_refusals,
        );
        write_json_pretty_atomic(&manifest_path, &turn_manifest)?;
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

    let manifest = build_eval_manifest(
        host.game_id(),
        opts.table_id,
        opts.seed,
        opts.arm,
        &relay_wording_arm,
        &wrapper_version,
        &wrapper_sha256,
        played_turns,
        &seats_file,
        opts.memory_mode,
        &refusal_by_seat,
        &refusal_by_arm,
        &wrong_part_refusals,
    );
    write_json_pretty_atomic(&manifest_path, &manifest)?;
    Ok(manifest)
}

fn build_refused_bundle_record(
    sub: &EvalActSubmission,
    report: &EvalSeatApplyReport,
    arm: EvalArm,
    guild: &str,
    refusal_by_seat: &mut BTreeMap<String, RefusalCodeCounts>,
    refusal_by_arm: &mut BTreeMap<String, RefusalCodeCounts>,
) -> Option<RefusedBundleRecord> {
    let code = report.refused_bundle_code.as_ref()?;
    refusal_by_seat
        .entry(guild.to_owned())
        .or_default()
        .refused_bundles += 1;
    refusal_by_arm
        .entry(arm.as_str().to_owned())
        .or_default()
        .refused_bundles += 1;
    Some(RefusedBundleRecord {
        submitted_bundle: sub.bundle.clone(),
        error_code: code.clone(),
        bad_leg_index: "leg not named".to_owned(),
    })
}

fn snapshot_pre_act_scratchpads(
    game_dir: &Path,
    checkpoints_dir: &Path,
    day: u8,
    guilds: &[String],
) -> Result<(), EvalError> {
    let relay_seats_dir = game_dir.join("relay").join("seats");
    if !relay_seats_dir.exists() {
        return Ok(());
    }
    for guild in guilds {
        let src = relay_seats_dir.join(format!("{guild}.scratchpad.md"));
        if src.exists() {
            let content = fs::read_to_string(&src)
                .map_err(|e| EvalError::Io(format!("read scratchpad {}: {e}", src.display())))?;
            let dst = checkpoints_dir.join(format!("day-{day:02}-pre-act.{guild}.scratchpad.md"));
            write_text_atomic(&dst, &content)?;
        }
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

/// Evaluate one or more frozen pre-act checkpoints across independent arms (`off`, `on`; `D4`, `Item 12`–`15`).
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
    for &arm in &opts.arms {
        let _ = arm_to_preview_mode(arm)?;
    }

    let (_wrapper_version, wrapper_sha256) =
        validate_wrapper_and_extract(&opts.wrapper_path, None)?;
    let seats_sha256 = hash_file_sha256(&opts.seats_path, "seats file")?;
    let expected_memory_mode = memory_mode_str(opts.memory_mode);
    let seats_file =
        SeatFile::load_with_allowed_hosts(&opts.seats_path, redaction_key, &opts.allowed_vm_hosts)?;
    validate_eval_seats_file(&seats_file)?;
    let response_format = build_eval_response_format_config()?;

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

    let mut all_outcomes = Vec::new();
    for (prefix, focal_spec, target_out_dir) in targets {
        let outcomes = branch_one_target(
            &prefix,
            &focal_spec,
            &target_out_dir,
            &seats_file,
            opts,
            &wrapper_sha256,
            expected_memory_mode,
            &seats_sha256,
            &response_format,
            transport,
            redaction_key,
        )?;
        all_outcomes.extend(outcomes);
    }

    let branch_summary: BranchSummary = summarize_branch_outcomes(&all_outcomes);
    write_json_pretty_atomic(&opts.out_dir.join("branch_summary.json"), &branch_summary)?;
    Ok(all_outcomes)
}

#[allow(clippy::too_many_arguments)]
fn branch_one_target(
    checkpoint_prefix: &Path,
    focal_spec: &str,
    target_out_dir: &Path,
    seats_file: &SeatFile,
    opts: &BranchOptions,
    wrapper_sha256: &str,
    expected_memory_mode: &str,
    expected_seats_sha256: &str,
    response_format: &league_relay::ResponseFormatConfig,
    transport: &dyn Transport,
    redaction_key: Option<&ApiKey>,
) -> Result<Vec<BranchArmOutcome>, EvalError> {
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

    let (source_transcript_entries, tx_sha_opt) =
        load_pre_act_transcript_prefix(source_game_dir, &focal_guild, day)?;
    if let Some((rel, sha)) = tx_sha_opt {
        loaded_history_sha256.insert(rel, sha);
    }

    let source_scratchpad = if let Some(ckpts) = checkpoint_prefix.parent() {
        let pad_rel = format!("checkpoints/day-{day_str}-pre-act.{focal_guild}.scratchpad.md");
        let pad_path = source_game_dir.join(&pad_rel);
        let ckpt_pad_path =
            ckpts.join(format!("day-{day_str}-pre-act.{focal_guild}.scratchpad.md"));
        if ckpt_pad_path.exists() {
            let txt = fs::read_to_string(&ckpt_pad_path).map_err(|e| {
                EvalError::Io(format!(
                    "read source scratchpad {}: {e}",
                    ckpt_pad_path.display()
                ))
            })?;
            let key = if pad_path == ckpt_pad_path {
                pad_rel
            } else {
                ckpt_pad_path.to_string_lossy().into_owned()
            };
            loaded_history_sha256.insert(key, sha256_hex(txt.as_bytes()));
            Some(txt)
        } else {
            None
        }
    } else {
        None
    };

    for &arm in &opts.arms {
        ensure_arm_dir_empty(&target_out_dir.join(arm.as_str()))?;
    }

    let bot = ClubBot::new(probe_host.bot_tie_seed());
    let mut outcomes = Vec::with_capacity(opts.arms.len());

    for &arm in &opts.arms {
        let preview_mode = arm_to_preview_mode(arm)?;
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
                let rec = EvalDecisionRecord {
                    game_id: source_game_id.clone(),
                    table_id,
                    day,
                    round: Round::Act,
                    seat: focal_idx as u8,
                    guild: focal_guild.clone(),
                    arm,
                    seat_kind: "ClubBot".to_owned(),
                    keeps_notes: false,
                    notes_present: false,
                    raw_reply: None,
                    attempts: 0,
                    failed_attempts: Vec::new(),
                    loaded_history_sha256: loaded_history_sha256.clone(),
                    refusal_code: None,
                    refusal_reason: None,
                    invalid_recipients: Vec::new(),
                    no_reply: false,
                    apply_report: None,
                    refused_bundle: None,
                    club_bot_baseline: Some(bot_sub.clone()),
                };
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
                .with_allowed_vm_hosts(opts.allowed_vm_hosts.clone());

                let arm_calls_path = arm_dir.join("relay").join("calls.jsonl");
                let offset = count_calls_lines(&arm_calls_path)?;
                let summary = run_turn(&branch_cfg, transport, redaction_key)?;
                let diag = read_round_attempt_diagnostics(
                    &arm_calls_path,
                    offset,
                    &day_str,
                    Round::Act,
                    &focal_guild,
                )?;
                match find_report(&summary, &focal_guild) {
                    Some(SeatOutcome::Delivered { from_seat_path, .. }) => {
                        let raw_reply = fs::read_to_string(from_seat_path).map_err(|e| {
                            EvalError::Io(format!(
                                "read branch reply {}: {e}",
                                from_seat_path.display()
                            ))
                        })?;
                        let parsed = parse_eval_act_reply(&raw_reply, focal_idx as u8, keeps_notes);
                        let rec = EvalDecisionRecord {
                            game_id: source_game_id.clone(),
                            table_id,
                            day,
                            round: Round::Act,
                            seat: focal_idx as u8,
                            guild: focal_guild.clone(),
                            arm,
                            seat_kind: "Model".to_owned(),
                            keeps_notes,
                            notes_present: parsed.notes_present,
                            raw_reply: Some(raw_reply),
                            attempts: diag.attempts,
                            failed_attempts: diag.failed_attempts,
                            loaded_history_sha256: loaded_history_sha256.clone(),
                            refusal_code: parsed.refusal_code.map(|c| c.as_str().to_owned()),
                            refusal_reason: parsed.refusal_reason,
                            invalid_recipients: Vec::new(),
                            no_reply: false,
                            apply_report: None,
                            refused_bundle: None,
                            club_bot_baseline: Some(bot_sub),
                        };
                        (parsed.submission, rec)
                    }
                    _ => (
                        EvalActSubmission::default(),
                        build_no_reply_record(
                            &source_game_id,
                            table_id,
                            day,
                            Round::Act,
                            focal_idx as u8,
                            &focal_guild,
                            arm,
                            keeps_notes,
                            diag,
                            loaded_history_sha256.clone(),
                            Some(bot_sub),
                            None,
                            None,
                            None,
                        ),
                    ),
                }
            };

        let mut spliced = recorded_subs.clone();
        spliced.insert(focal_idx as u8, focal_sub.clone());
        let apply_outcome = host.apply_act_submissions(&spliced)?;
        if let Some(report) = apply_outcome.seats.get(&(focal_idx as u8)) {
            if let Some(ref code) = report.refused_bundle_code {
                decision_rec.refused_bundle = Some(RefusedBundleRecord {
                    submitted_bundle: focal_sub.bundle,
                    error_code: code.clone(),
                    bad_leg_index: "leg not named".to_owned(),
                });
            }
            decision_rec.apply_report = Some(report.clone());
        }
        let receipt: PublicSweepReceipt = host.sweep_turn()?;

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
        write_json_pretty_atomic(&arm_dir.join("decision.json"), &decision_rec)?;
        write_json_pretty_atomic(&arm_dir.join("receipt.json"), &receipt)?;
        write_json_pretty_atomic(&arm_dir.join("outcome.json"), &arm_outcome)?;
        outcomes.push(arm_outcome);
    }

    Ok(outcomes)
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

fn load_pre_act_transcript_prefix(
    source_game_dir: &Path,
    focal_guild: &str,
    branch_day: u8,
) -> Result<TranscriptPrefixWithDigest, EvalError> {
    let rel_path = format!("relay/seats/{focal_guild}.jsonl");
    let path = source_game_dir.join(&rel_path);
    if !path.exists() {
        return Ok((Vec::new(), None));
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
