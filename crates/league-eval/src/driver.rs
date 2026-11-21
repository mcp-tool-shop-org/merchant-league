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
    ApiKey, MemoryMode, PreviewToolMode, RawHttpResponse, Round, SeatBackend, SeatFile,
    SeatOutcome, SeatSpec, TranscriptEntry, Transport, TransportError, TurnConfig, TurnSummary,
    UntrustedWhisper, run_turn,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::EvalError;
use crate::records::{
    BranchArmOutcome, EvalArm, EvalDecisionRecord, EvalManifest, EvalManifestSeat, EvalSetupFile,
    PASS_B_LIMIT_STATEMENT, RefusalCodeCounts, RefusedBundleRecord, WrongPartCounter,
    extract_wrapper_metadata,
};
use crate::reply::{ReplyRefusalCode, parse_eval_act_reply, parse_eval_talk_reply};
use crate::schema::build_eval_response_format_config;
use crate::select::SelectionList;
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

/// Validate Evaluation Mode seat count (`D6` = 4 seats) and `"file"` backend policy (`1.7`, Test 6).
pub fn validate_eval_seats_file(seats_file: &SeatFile) -> Result<(), EvalError> {
    if seats_file.seats.len() != 4 {
        return Err(EvalError::InvalidSeatCount {
            actual: seats_file.seats.len(),
        });
    }
    for seat in &seats_file.seats {
        if let SeatBackend::File { model } = &seat.backend
            && model.as_deref() != Some("ClubBot")
        {
            return Err(EvalError::UnsupportedFileSeatModel {
                guild: seat.guild.clone(),
                model: model.clone(),
            });
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

fn derive_bot_tie_seed(layout_seed: u64, table_id: u64) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(b"league-eval/bot-tie-seed/v1");
    hasher.update(layout_seed.to_le_bytes());
    hasher.update(table_id.to_le_bytes());
    let digest: [u8; 32] = hasher.finalize().into();
    let mut arr = [0_u8; 8];
    arr.copy_from_slice(&digest[..8]);
    let val = u64::from_le_bytes(arr);
    if val == 0 { 1 } else { val }
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
            return Err(EvalError::Usage(format!(
                "setup.json game_id '{}' does not match seal game_id '{}'",
                existing_setup.game_id,
                host.game_id()
            )));
        }
        if existing_setup.arm != opts.arm
            || existing_setup.table_id != opts.table_id
            || existing_setup.seed != opts.seed
            || existing_setup.wrapper_sha256 != wrapper_sha256
        {
            return Err(EvalError::Usage(
                "cannot resume game_dir with mismatched --arm, --table-id, --seed, or --wrapper"
                    .to_owned(),
            ));
        }
        host
    } else if dir_has_eval_save_or_seal(&opts.game_dir)? {
        let host = EvalHost::fork_duplicate_from_game_dir(&opts.game_dir, TableId(opts.table_id))?;
        persist_eval_game_state(&host, &opts.game_dir)?;
        host
    } else {
        let bot_tie_seed = derive_bot_tie_seed(opts.seed, opts.table_id);
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
        response_schema_sha256: Some(response_schema_sha256),
    };
    write_json_pretty_atomic(&setup_path, &setup_record)?;

    let (mut refusal_by_seat, mut refusal_by_arm, mut wrong_part_refusals) = if manifest_path
        .exists()
    {
        let raw_m = fs::read_to_string(&manifest_path)
            .map_err(|e| EvalError::Io(format!("read {}: {e}", manifest_path.display())))?;
        let prev: EvalManifest = serde_json::from_str(&raw_m)
            .map_err(|e| EvalError::Io(format!("parse {}: {e}", manifest_path.display())))?;
        (
            prev.refusal_counts_by_seat,
            prev.refusal_counts_by_arm,
            prev.wrong_part_refusals,
        )
    } else {
        let mut by_seat = BTreeMap::new();
        for s in &seats_file.seats {
            by_seat.insert(s.guild.clone(), RefusalCodeCounts::default());
        }
        let by_arm = BTreeMap::from([(opts.arm.as_str().to_owned(), RefusalCodeCounts::default())]);
        (by_seat, by_arm, WrongPartCounter::default())
    };

    let seated_guilds: Vec<String> = seats_file.seats.iter().map(|s| s.guild.clone()).collect();
    let bot = ClubBot::new(host.bot_tie_seed());
    let to_seat_dir = opts.game_dir.join("to-seat");
    let decisions_dir = opts.game_dir.join("decisions");
    let receipts_dir = opts.game_dir.join("receipts");
    let checkpoints_dir = opts.game_dir.join("checkpoints");

    let mut played_turns = u8::try_from(host.public_receipts().len()).unwrap_or(u8::MAX);
    while played_turns < opts.turns && host.phase() == TurnPhase::TalkAndMarket {
        let day = host.turn();
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
                    refusal_code: None,
                    refusal_reason: None,
                    invalid_recipients: Vec::new(),
                    no_reply: false,
                    apply_report: None,
                    refused_bundle: None,
                    club_bot_baseline: None,
                },
                SeatBackend::Local(_) | SeatBackend::Openrouter(_) => {
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
                            wrong_part_refusals.record_reply(wrong_part);

                            let seat_counts =
                                refusal_by_seat.entry(seat.guild.clone()).or_default();
                            let arm_counts = refusal_by_arm
                                .entry(opts.arm.as_str().to_owned())
                                .or_default();
                            if let Some(code) = parsed.refusal_code {
                                seat_counts.record(code);
                                arm_counts.record(code);
                            }
                            for diag in &parsed.invalid_recipients {
                                seat_counts.record(diag.code);
                                arm_counts.record(diag.code);
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
                                refusal_code: parsed.refusal_code.map(|c| c.as_str().to_owned()),
                                refusal_reason: parsed.refusal_reason,
                                invalid_recipients: parsed
                                    .invalid_recipients
                                    .iter()
                                    .map(|d| {
                                        format!("{}[{}]: {}", d.to_guild, d.message_index, d.reason)
                                    })
                                    .collect(),
                                no_reply: false,
                                apply_report: None,
                                refused_bundle: None,
                                club_bot_baseline: None,
                            }
                        }
                        _ => {
                            wrong_part_refusals.record_reply(None);
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
                                notes_present: false,
                                raw_reply: None,
                                refusal_code: None,
                                refusal_reason: None,
                                invalid_recipients: Vec::new(),
                                no_reply: true,
                                apply_report: None,
                                refused_bundle: None,
                                club_bot_baseline: None,
                            }
                        }
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
                            wrong_part_refusals.record_reply(wrong_part);

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
                        _ => {
                            wrong_part_refusals.record_reply(None);
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
                                notes_present: false,
                                raw_reply: None,
                                refusal_code: None,
                                refusal_reason: None,
                                invalid_recipients: Vec::new(),
                                no_reply: true,
                                apply_report: None,
                                refused_bundle: None,
                                club_bot_baseline: Some(bot_sub),
                            };
                            (EvalActSubmission::default(), rec)
                        }
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
                keeps_notes: seat_keeps_notes(seat, &seats_file, opts.memory_mode),
            }
        })
        .collect();

    let manifest = EvalManifest {
        game_id: host.game_id().to_owned(),
        table_id: opts.table_id,
        seed: opts.seed,
        arm: opts.arm,
        eval_reply_wording_arm: EVAL_REPLY_WORDING_ARM.to_owned(),
        relay_wording_arm,
        wrapper_version,
        wrapper_sha256,
        pass_b_limit: PASS_B_LIMIT_STATEMENT.to_owned(),
        played_turns,
        seats: manifest_seats,
        refusal_counts_by_seat: refusal_by_seat,
        refusal_counts_by_arm: refusal_by_arm,
        wrong_part_refusals,
    };
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

    let _wrapper_meta = validate_wrapper_and_extract(&opts.wrapper_path, None)?;
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
                    (s.checkpoint_prefix, s.seat_guild, target_out)
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
            &response_format,
            transport,
            redaction_key,
        )?;
        all_outcomes.extend(outcomes);
    }
    Ok(all_outcomes)
}

#[allow(clippy::too_many_arguments)]
fn branch_one_target(
    checkpoint_prefix: &Path,
    focal_spec: &str,
    target_out_dir: &Path,
    seats_file: &SeatFile,
    opts: &BranchOptions,
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

    let source_game_dir = checkpoint_prefix.parent().and_then(|p| p.parent());
    let source_negotiation_json = source_game_dir.and_then(|gd| {
        let p = gd
            .join("to-seat")
            .join(format!("day-{day_str}-act-{focal_guild}.negotiation.json"));
        fs::read_to_string(p).ok()
    });
    let source_transcript_entries = source_game_dir
        .map(|gd| load_pre_act_transcript_prefix(gd, &focal_guild, day))
        .transpose()?
        .unwrap_or_default();
    let source_scratchpad = checkpoint_prefix
        .parent()
        .map(|ckpts| ckpts.join(format!("day-{day_str}-pre-act.{focal_guild}.scratchpad.md")))
        .filter(|p| p.exists())
        .and_then(|p| fs::read_to_string(p).ok());

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
        let (focal_sub, mut decision_rec) =
            if matches!(focal_seat_spec.backend, SeatBackend::File { .. }) {
                let act_md = render_seat_view_markdown(Round::Act, &obs)?;
                write_text_atomic(
                    &arm_dir
                        .join("to-seat")
                        .join(format!("day-{day_str}-act-{focal_guild}.md")),
                    &act_md,
                )?;
                let neg_text = match &source_negotiation_json {
                    Some(txt) => txt.clone(),
                    None => {
                        let neg = build_act_negotiation_inputs(Vec::new());
                        let mut s = serde_json::to_string_pretty(&neg).map_err(|e| {
                            EvalError::Io(format!("serialize negotiation fallback: {e}"))
                        })?;
                        s.push('\n');
                        s
                    }
                };
                write_text_atomic(
                    &arm_dir
                        .join("to-seat")
                        .join(format!("day-{day_str}-act-{focal_guild}.negotiation.json")),
                    &neg_text,
                )?;

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
                let to_seat_dir = arm_dir.join("to-seat");
                let act_md = render_seat_view_markdown(Round::Act, &obs)?;
                write_text_atomic(
                    &to_seat_dir.join(format!("day-{day_str}-act-{focal_guild}.md")),
                    &act_md,
                )?;
                if arm == EvalArm::On {
                    let preview_json = render_preview_state_json(&obs)?;
                    write_text_atomic(
                        &to_seat_dir.join(format!(
                            "day-{day_str}-act-{focal_guild}.preview-state.json"
                        )),
                        &preview_json,
                    )?;
                }
                let neg_text = match &source_negotiation_json {
                    Some(txt) => txt.clone(),
                    None => {
                        let neg = build_act_negotiation_inputs(Vec::new());
                        let mut s = serde_json::to_string_pretty(&neg).map_err(|e| {
                            EvalError::Io(format!("serialize negotiation fallback: {e}"))
                        })?;
                        s.push('\n');
                        s
                    }
                };
                write_text_atomic(
                    &to_seat_dir.join(format!("day-{day_str}-act-{focal_guild}.negotiation.json")),
                    &neg_text,
                )?;

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

                let summary = run_turn(&branch_cfg, transport, redaction_key)?;
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
                    _ => {
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
                            notes_present: false,
                            raw_reply: None,
                            refusal_code: None,
                            refusal_reason: None,
                            invalid_recipients: Vec::new(),
                            no_reply: true,
                            apply_report: None,
                            refused_bundle: None,
                            club_bot_baseline: Some(bot_sub),
                        };
                        (EvalActSubmission::default(), rec)
                    }
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

fn load_pre_act_transcript_prefix(
    source_game_dir: &Path,
    focal_guild: &str,
    branch_day: u8,
) -> Result<Vec<TranscriptEntry>, EvalError> {
    let path = source_game_dir
        .join("relay")
        .join("seats")
        .join(format!("{focal_guild}.jsonl"));
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(&path)
        .map_err(|e| EvalError::Io(format!("read source transcript {}: {e}", path.display())))?;
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
        return Ok(Vec::new());
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
        return Ok(Vec::new());
    }
    let mut result = Vec::with_capacity(filtered_pairs.len() + 1);
    if let Some(sys) = system_entry {
        result.push(sys);
    }
    result.extend(filtered_pairs);
    Ok(result)
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
