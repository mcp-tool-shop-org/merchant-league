//! Command-line parsing and execution for the `league-relay` binary.
//!
//! Kept in the library crate so every CLI validation, redaction, and exit-code
//! path is covered by both unit and integration tests.

use crate::error::{RelayError, ToStructuredError, scrub_structured_error_with_keys};
use crate::relay::{
    GameSetupRecord, MemoryMode, RelayDiagSink, ResponseFormatConfig, Round, SeatOutcome,
    TurnConfig, TurnSummary, run_turn_with_diag, validate_day, validate_scratchpad_setup,
};
use crate::seats::{SeatBackend, SeatFile, validate_vm_host_allowance};
use crate::secret::{
    ApiKey, FORBIDDEN_LOCAL_KEY_ENVS, LEAGUE_LOG_LEVEL_ENV, LiveGate, LocalGate, LogLevel,
    MAX_UNRECORDED_CHARGE_SCAN_BYTES, OPENROUTER_API_KEY_ENV, collect_env_redaction_keys,
    panic_hook_log_level, redact_secrets, redact_secrets_multi_with_limit,
    set_panic_hook_log_level, truncate_after_redaction,
};
use crate::transport::{
    LocalOpenAiTransport, MockDirTransport, RawHttpResponse, Transport, TransportError,
    UreqTransport,
};
use crate::whisper_relay::{
    WhisperRelayCoordinator, WhisperRelayStateSnapshot, WhisperTableConfig,
    batch_to_negotiation_inputs,
};
use crate::wording::{PreviewToolMode, wording_slot};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Exit code `0`: clean completion (`SHIP_GATE.md` `:27`).
pub const EXIT_OK: i32 = 0;
/// Exit code `1`: user, CLI, or configuration error (`SHIP_GATE.md` `:27`).
pub const EXIT_USER_ERROR: i32 = 1;
/// Exit code `2`: runtime error (`TURN_LOCKED`, `UNRECORDED_CHARGE`, I/O, corrupt log, or zero-clean turn).
pub const EXIT_RUNTIME_ERROR: i32 = 2;
/// Exit code `3`: partial success (turn recorded cleanly with `>= 1` clean and `>= 1` failed seat).
pub const EXIT_PARTIAL_SUCCESS: i32 = 3;

/// Output encoding selected by `--error-format` (`SHIP_GATE.md` `:26`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ErrorFormat {
    /// Human-readable single-line redacted [`StructuredError`](crate::StructuredError) text (`default`).
    #[default]
    Text,
    /// Compact machine-readable [`StructuredError`](crate::StructuredError) JSON on `stderr`.
    Json,
}

impl ErrorFormat {
    /// Parse `"text"` or `"json"` (case-insensitive).
    pub fn parse(raw: &str) -> Result<Self, RelayError> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            _ => Err(RelayError::Usage(
                "--error-format must be 'text' or 'json'".to_owned(),
            )),
        }
    }

    /// Lowercase CLI label (`"text"` or `"json"`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Json => "json",
        }
    }
}

/// Usage text printed for `--help`.
pub const USAGE_TEXT: &str = "\
Usage:
  league-relay turn --game-dir <dir> --day <DD> --round <talk|act> --seats <file> [options]
  league-relay prebook-check --seats <file> --wrapper <path> [--response-schema <path>] [options]
  league-relay whisper-tick --table-keys <file> --envelopes <file> --out-dir <dir> --table-id <id> --turn <N> --sub-round <K> [options]

Options:
  --wrapper <path>                 System wrapper markdown sent as role='system' (required on turn 1)
  --memory-mode <mode>             Prompt memory mode: snapshot (default), emc, or full
  --response-schema <path>         Optional JSON Schema file sent in response_format (json_schema)
  --scratchpad                     Require persistent scratchpad notes (requires snapshot mode + schema)
  --no-scratchpad                  Disable persistent scratchpad notes for this run
  --mixed-scratchpad-arm           Permit a deliberate arm mixing scratchpad and no-scratchpad seats
  --cap-gpu-secs <n>               Cumulative GPU-seconds ceiling for local OpenAI-compatible seats
  --allow-vm-host <host>           Additional VM-local server address permitted in local seat base_url
  --live                           Permit live OpenRouter calls (requires OPENROUTER_API_KEY and --cap-credits)
  --cap-credits <n>                Cumulative credit cap checked against relay/calls.jsonl before each call
  --mock-dir <dir>                 Run against canned offline responses in <dir>/responses.json
  --preview-tool <off|control|on>  Select act-round drafting or what-if preview tool (off default)
  --error-format <text|json>       Error output encoding on stderr: text (default) or json
  --log-level <level>              Log verbosity: silent, normal (default), verbose, or debug (or LEAGUE_LOG_LEVEL)
  -q, --silent                     Shorthand for --log-level silent
  -v, --verbose                    Shorthand for --log-level verbose (timings and cap accounting)
  --debug                          Shorthand for --log-level debug (redacted transport, hash, lock metadata, backtrace)
  -h, --help                       Print this help message
";

/// Parsed command-line request.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum CliCommand {
    /// Print help text and exit `0`.
    Help,
    /// Execute one `turn`.
    Turn(TurnArgs),
    /// Run a self-contained offline mock `turn` in a fresh temporary game directory (`P43-B3`).
    PrebookCheck(PrebookCheckArgs),
    /// Validate signed Game/Club whispers and release a discrete sub-round tick-barrier batch.
    WhisperTick(WhisperTickArgs),
}

/// Parsed arguments for `league-relay whisper-tick`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhisperTickArgs {
    /// `--table-keys <file>`
    pub table_keys_path: PathBuf,
    /// `--envelopes <file>`
    pub envelopes_path: PathBuf,
    /// `--out-dir <dir>`
    pub out_dir: PathBuf,
    /// `--table-id <id>`
    pub table_id: u64,
    /// `--turn <N>`
    pub turn: u8,
    /// `--sub-round <K>`
    pub sub_round: u8,
    /// `--state-file <file>`
    pub state_file: Option<PathBuf>,
    /// Optional `--game-dir <dir>` for writing `to-seat/day-<DD>-act-<Guild>.negotiation.json`
    pub game_dir: Option<PathBuf>,
    /// Optional `--day <DD>` paired with `--game-dir` and `--batna-file`
    pub day: Option<String>,
    /// Optional `--batna-file <file>` paired with `--game-dir` and `--day`
    pub batna_file: Option<PathBuf>,
    /// `--error-format <text|json>`
    pub error_format: ErrorFormat,
    /// Explicit CLI `--log-level` / `-q` / `-v` / `--debug` override, if passed.
    pub log_level: Option<LogLevel>,
}

/// Parsed arguments for `league-relay prebook-check` (`P43-B3`).
#[derive(Debug, Clone, PartialEq)]
pub struct PrebookCheckArgs {
    /// `--seats <file>`
    pub seats_path: PathBuf,
    /// `--wrapper <path>`
    pub wrapper_path: PathBuf,
    /// `--response-schema <path>`
    pub response_schema_path: Option<PathBuf>,
    /// `--memory-mode <snapshot|emc|full>`
    pub memory_mode: MemoryMode,
    /// `--scratchpad` / `--no-scratchpad`
    pub scratchpad_requested: Option<bool>,
    /// `--mixed-scratchpad-arm`
    pub mixed_scratchpad_arm: bool,
    /// `--allow-vm-host <host>`
    pub allowed_vm_hosts: Vec<String>,
    /// `--preview-tool <off|control|on>`
    pub preview_tool: PreviewToolMode,
    /// `--error-format <text|json>`
    pub error_format: ErrorFormat,
    /// Explicit CLI `--log-level` / `-q` / `-v` / `--debug` override, if passed.
    pub log_level: Option<LogLevel>,
}

/// Parsed arguments for `league-relay turn`.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnArgs {
    /// `--game-dir <dir>`
    pub game_dir: PathBuf,
    /// `--day <DD>`
    pub day: String,
    /// `--round <talk|act>`
    pub round: Round,
    /// `--seats <file>`
    pub seats_path: PathBuf,
    /// `--wrapper <path>`
    pub wrapper_path: Option<PathBuf>,
    /// `--memory-mode <snapshot|emc|full>`
    pub memory_mode: MemoryMode,
    /// `--response-schema <path>`
    pub response_schema_path: Option<PathBuf>,
    /// `--scratchpad` / `--no-scratchpad`
    pub scratchpad_requested: Option<bool>,
    /// `--mixed-scratchpad-arm`
    pub mixed_scratchpad_arm: bool,
    /// `--live`
    pub live: bool,
    /// `--cap-credits <n>`
    pub cap_credits: Option<f64>,
    /// `--cap-gpu-secs <n>`
    pub cap_gpu_secs: Option<f64>,
    /// `--allow-vm-host <host>`
    pub allowed_vm_hosts: Vec<String>,
    /// `--mock-dir <dir>`
    pub mock_dir: Option<PathBuf>,
    /// `--preview-tool <off|control|on>`
    pub preview_tool: PreviewToolMode,
    /// `--error-format <text|json>`
    pub error_format: ErrorFormat,
    /// Explicit CLI `--log-level` / `-q` / `-v` / `--debug` override, if passed.
    pub log_level: Option<LogLevel>,
}

/// Parse CLI arguments (excluding `argv[0]`), never echoing flag values on unknown/key flags.
pub fn parse_args<I, S>(args: I) -> Result<CliCommand, RelayError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    parse_args_with_key(args, None)
}

fn set_log_level_checked(slot: &mut Option<LogLevel>, next: LogLevel) -> Result<(), RelayError> {
    if let Some(prev) = *slot
        && prev != next
    {
        return Err(RelayError::Usage(format!(
            "conflicting log-level flags ('{}' and '{}')",
            prev.as_str(),
            next.as_str()
        )));
    }
    *slot = Some(next);
    Ok(())
}

fn try_consume_hygiene_flag(
    flag: &str,
    inline_val: Option<&str>,
    tokens: &[String],
    idx: usize,
    error_format: &mut ErrorFormat,
    seen_error_format: &mut Option<ErrorFormat>,
    log_level: &mut Option<LogLevel>,
) -> Result<Option<usize>, RelayError> {
    match flag {
        "--error-format" => {
            let (val, step) = take_flag_value(flag, inline_val, tokens, idx)?;
            let parsed = ErrorFormat::parse(val)?;
            if let Some(prev) = *seen_error_format
                && prev != parsed
            {
                return Err(RelayError::Usage(format!(
                    "conflicting --error-format values ('{}' and '{}')",
                    prev.as_str(),
                    parsed.as_str()
                )));
            }
            *seen_error_format = Some(parsed);
            *error_format = parsed;
            Ok(Some(step))
        }
        "--log-level" => {
            let (val, step) = take_flag_value(flag, inline_val, tokens, idx)?;
            let parsed = LogLevel::parse(val)?;
            set_log_level_checked(log_level, parsed)?;
            Ok(Some(step))
        }
        "-q" | "--silent" => {
            if inline_val.is_some() {
                return Err(RelayError::Usage(format!("{flag} takes no value")));
            }
            set_log_level_checked(log_level, LogLevel::Silent)?;
            Ok(Some(1))
        }
        "-v" | "--verbose" => {
            if inline_val.is_some() {
                return Err(RelayError::Usage(format!("{flag} takes no value")));
            }
            set_log_level_checked(log_level, LogLevel::Verbose)?;
            Ok(Some(1))
        }
        "--debug" => {
            if inline_val.is_some() {
                return Err(RelayError::Usage("--debug takes no value".to_owned()));
            }
            set_log_level_checked(log_level, LogLevel::Debug)?;
            Ok(Some(1))
        }
        _ => Ok(None),
    }
}

/// Best-effort scan of raw CLI arguments for `--error-format`, used when full parsing fails.
#[must_use]
pub fn infer_error_format_best_effort<I, S>(args: I) -> ErrorFormat
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let tokens: Vec<String> = args.into_iter().map(|s| s.as_ref().to_owned()).collect();
    let mut format = ErrorFormat::default();
    let mut idx = 0;
    while idx < tokens.len() {
        let arg = &tokens[idx];
        if let Some(val) = arg.strip_prefix("--error-format=") {
            if let Ok(parsed) = ErrorFormat::parse(val) {
                format = parsed;
            }
            idx += 1;
        } else if arg == "--error-format" {
            if let Some(next) = tokens.get(idx + 1)
                && let Ok(parsed) = ErrorFormat::parse(next)
            {
                format = parsed;
            }
            idx += 2;
        } else {
            idx += 1;
        }
    }
    format
}

/// Best-effort scan of raw CLI arguments and `LEAGUE_LOG_LEVEL` for early panic-hook setup.
pub fn infer_log_level_best_effort<I, S, E>(args: I, mut env_reader: E) -> LogLevel
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
    E: FnMut(&str) -> Option<String>,
{
    let tokens: Vec<String> = args.into_iter().map(|s| s.as_ref().to_owned()).collect();
    let mut cli_level: Option<LogLevel> = None;
    let mut idx = 0;
    while idx < tokens.len() {
        let arg = &tokens[idx];
        if let Some(val) = arg.strip_prefix("--log-level=") {
            if let Ok(parsed) = LogLevel::parse(val) {
                cli_level = Some(parsed);
            }
            idx += 1;
        } else if arg == "--log-level" {
            if let Some(next) = tokens.get(idx + 1)
                && let Ok(parsed) = LogLevel::parse(next)
            {
                cli_level = Some(parsed);
            }
            idx += 2;
        } else if matches!(arg.as_str(), "-q" | "--silent") {
            cli_level = Some(LogLevel::Silent);
            idx += 1;
        } else if matches!(arg.as_str(), "-v" | "--verbose") {
            cli_level = Some(LogLevel::Verbose);
            idx += 1;
        } else if arg == "--debug" {
            cli_level = Some(LogLevel::Debug);
            idx += 1;
        } else {
            idx += 1;
        }
    }
    if let Some(lvl) = cli_level {
        return lvl;
    }
    if let Some(raw_env) = read_named_env(&mut env_reader, LEAGUE_LOG_LEVEL_ENV)
        && !raw_env.trim().is_empty()
        && let Ok(parsed) = LogLevel::parse(&raw_env)
    {
        return parsed;
    }
    LogLevel::Normal
}

/// Parse CLI arguments with an optional [`ApiKey`] so any shortened flag name is redacted before truncation.
pub fn parse_args_with_key<I, S>(
    args: I,
    redaction_key: Option<&ApiKey>,
) -> Result<CliCommand, RelayError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let tokens: Vec<String> = args.into_iter().map(|s| s.as_ref().to_owned()).collect();
    if tokens.is_empty() {
        return Err(RelayError::Usage(
            "missing subcommand (expected 'turn' or '--help')".to_owned(),
        ));
    }

    let mut error_format = ErrorFormat::default();
    let mut seen_error_format: Option<ErrorFormat> = None;
    let mut log_level: Option<LogLevel> = None;

    let mut pos = 0;
    while pos < tokens.len() {
        let raw_arg = &tokens[pos];
        if matches!(raw_arg.as_str(), "-h" | "--help") {
            return Ok(CliCommand::Help);
        }
        if !raw_arg.starts_with('-') {
            break;
        }
        let (flag, inline_val) = match raw_arg.split_once('=') {
            Some((f, v)) => (f, Some(v)),
            None => (raw_arg.as_str(), None),
        };
        if is_forbidden_key_flag(flag) || looks_like_cli_secret(raw_arg) {
            return Err(RelayError::SecretInForbiddenPlace(
                "API key must be read from OPENROUTER_API_KEY only, never passed as an argument"
                    .to_owned(),
            ));
        }
        if let Some(step) = try_consume_hygiene_flag(
            flag,
            inline_val,
            &tokens,
            pos,
            &mut error_format,
            &mut seen_error_format,
            &mut log_level,
        )? {
            pos += step;
            continue;
        }
        return Err(RelayError::Usage(format!(
            "unknown option '{}' before subcommand (expected 'turn', 'prebook-check', or '--help')",
            sanitize_flag_name(flag, redaction_key)
        )));
    }

    if pos >= tokens.len() {
        return Err(RelayError::Usage(
            "missing subcommand (expected 'turn', 'prebook-check', or '--help')".to_owned(),
        ));
    }
    if pos + 1 == tokens.len() && matches!(tokens[pos].as_str(), "-h" | "--help" | "help") {
        return Ok(CliCommand::Help);
    }
    if tokens[pos] == "prebook-check" {
        return parse_prebook_check_args(
            &tokens[pos..],
            redaction_key,
            error_format,
            seen_error_format,
            log_level,
        );
    }
    if tokens[pos] == "whisper-tick" {
        return parse_whisper_tick_args(
            &tokens[pos..],
            redaction_key,
            error_format,
            seen_error_format,
            log_level,
        );
    }
    if tokens[pos] != "turn" {
        let first_flag = tokens[pos].split('=').next().unwrap_or(&tokens[pos]);
        if is_forbidden_key_flag(first_flag) || looks_like_cli_secret(&tokens[pos]) {
            return Err(RelayError::SecretInForbiddenPlace(
                "API key must be read from OPENROUTER_API_KEY only, never passed as an argument"
                    .to_owned(),
            ));
        }
        return Err(RelayError::Usage(
            "unknown subcommand (expected 'turn', 'prebook-check', 'whisper-tick', or '--help')"
                .to_owned(),
        ));
    }

    let sub_tokens = &tokens[pos..];
    let mut game_dir: Option<PathBuf> = None;
    let mut day: Option<String> = None;
    let mut round: Option<Round> = None;
    let mut seats_path: Option<PathBuf> = None;
    let mut wrapper_path: Option<PathBuf> = None;
    let mut memory_mode = MemoryMode::default();
    let mut response_schema_path: Option<PathBuf> = None;
    let mut scratchpad_requested: Option<bool> = None;
    let mut mixed_scratchpad_arm = false;
    let mut live = false;
    let mut cap_credits: Option<f64> = None;
    let mut cap_gpu_secs: Option<f64> = None;
    let mut allowed_vm_hosts: Vec<String> = Vec::new();
    let mut mock_dir: Option<PathBuf> = None;
    let mut preview_tool = PreviewToolMode::default();

    let mut idx = 1;
    while idx < sub_tokens.len() {
        let raw_arg = &sub_tokens[idx];
        if matches!(raw_arg.as_str(), "-h" | "--help") {
            return Ok(CliCommand::Help);
        }

        let (flag, inline_val) = match raw_arg.split_once('=') {
            Some((f, v)) => (f, Some(v)),
            None => (raw_arg.as_str(), None),
        };

        if is_forbidden_key_flag(flag) || looks_like_cli_secret(raw_arg) {
            return Err(RelayError::SecretInForbiddenPlace(
                "API key must be read from OPENROUTER_API_KEY only, never passed as an argument"
                    .to_owned(),
            ));
        }

        if let Some(step) = try_consume_hygiene_flag(
            flag,
            inline_val,
            sub_tokens,
            idx,
            &mut error_format,
            &mut seen_error_format,
            &mut log_level,
        )? {
            idx += step;
            continue;
        }

        match flag {
            "--live" => {
                if inline_val.is_some() {
                    return Err(RelayError::Usage("--live takes no value".to_owned()));
                }
                live = true;
                idx += 1;
            }
            "--game-dir" => {
                let (val, step) = take_flag_value(flag, inline_val, sub_tokens, idx)?;
                game_dir = Some(PathBuf::from(val));
                idx += step;
            }
            "--day" => {
                let (val, step) = take_flag_value(flag, inline_val, sub_tokens, idx)?;
                validate_day(val)?;
                day = Some(val.to_owned());
                idx += step;
            }
            "--round" => {
                let (val, step) = take_flag_value(flag, inline_val, sub_tokens, idx)?;
                round = Some(Round::parse(val)?);
                idx += step;
            }
            "--seats" => {
                let (val, step) = take_flag_value(flag, inline_val, sub_tokens, idx)?;
                seats_path = Some(PathBuf::from(val));
                idx += step;
            }
            "--wrapper" => {
                let (val, step) = take_flag_value(flag, inline_val, sub_tokens, idx)?;
                wrapper_path = Some(PathBuf::from(val));
                idx += step;
            }
            "--memory-mode" => {
                let (val, step) = take_flag_value(flag, inline_val, sub_tokens, idx)?;
                memory_mode = MemoryMode::parse(val)?;
                idx += step;
            }
            "--response-schema" => {
                let (val, step) = take_flag_value(flag, inline_val, sub_tokens, idx)?;
                response_schema_path = Some(PathBuf::from(val));
                idx += step;
            }
            "--scratchpad" => {
                let req = match inline_val {
                    None => true,
                    Some("true") => true,
                    Some("false") => false,
                    Some(_) => {
                        return Err(RelayError::Usage(
                            "--scratchpad takes no value (or '=true' / '=false')".to_owned(),
                        ));
                    }
                };
                if scratchpad_requested.is_some_and(|prev| prev != req) {
                    return Err(RelayError::Usage(
                        "--scratchpad and --no-scratchpad cannot be used together".to_owned(),
                    ));
                }
                scratchpad_requested = Some(req);
                idx += 1;
            }
            "--no-scratchpad" => {
                if inline_val.is_some() {
                    return Err(RelayError::Usage(
                        "--no-scratchpad takes no value".to_owned(),
                    ));
                }
                if scratchpad_requested == Some(true) {
                    return Err(RelayError::Usage(
                        "--scratchpad and --no-scratchpad cannot be used together".to_owned(),
                    ));
                }
                scratchpad_requested = Some(false);
                idx += 1;
            }
            "--mixed-scratchpad-arm" => {
                if inline_val.is_some() {
                    return Err(RelayError::Usage(
                        "--mixed-scratchpad-arm takes no value".to_owned(),
                    ));
                }
                mixed_scratchpad_arm = true;
                idx += 1;
            }
            "--cap-credits" => {
                let (val, step) = take_flag_value(flag, inline_val, sub_tokens, idx)?;
                let parsed: f64 = val
                    .parse::<f64>()
                    .ok()
                    .filter(|n| n.is_finite() && *n >= 0.0)
                    .ok_or_else(|| {
                        RelayError::InvalidCap(
                            "--cap-credits must be a finite non-negative number".to_owned(),
                        )
                    })?;
                cap_credits = Some(parsed);
                idx += step;
            }
            "--cap-gpu-secs" => {
                let (val, step) = take_flag_value(flag, inline_val, sub_tokens, idx)?;
                let parsed: f64 = val
                    .parse::<f64>()
                    .ok()
                    .filter(|n| n.is_finite() && *n >= 0.0)
                    .ok_or_else(|| {
                        RelayError::InvalidCap(
                            "--cap-gpu-secs must be a finite non-negative number".to_owned(),
                        )
                    })?;
                cap_gpu_secs = Some(parsed);
                idx += step;
            }
            "--allow-vm-host" => {
                let (val, step) = take_flag_value(flag, inline_val, sub_tokens, idx)?;
                let normalized = validate_vm_host_allowance(val)?;
                if !allowed_vm_hosts.contains(&normalized) {
                    allowed_vm_hosts.push(normalized);
                }
                idx += step;
            }
            "--mock-dir" => {
                let (val, step) = take_flag_value(flag, inline_val, sub_tokens, idx)?;
                mock_dir = Some(PathBuf::from(val));
                idx += step;
            }
            "--preview-tool" => {
                let (val, step) = take_flag_value(flag, inline_val, sub_tokens, idx)?;
                preview_tool = PreviewToolMode::parse(val)?;
                idx += step;
            }
            "--communication" => {
                let (val, step) = take_flag_value(flag, inline_val, sub_tokens, idx)?;
                parse_communication_cli_flag(val)?;
                idx += step;
            }
            other => {
                if !other.starts_with('-') {
                    return Err(RelayError::Usage(
                        "unexpected positional argument (all options require a '--flag')"
                            .to_owned(),
                    ));
                }
                return Err(RelayError::Usage(format!(
                    "unknown option '{}'",
                    sanitize_flag_name(other, redaction_key)
                )));
            }
        }
    }

    if live && mock_dir.is_some() {
        return Err(RelayError::Usage(
            "--live and --mock-dir cannot be used together".to_owned(),
        ));
    }
    if cap_credits.is_some() && cap_gpu_secs.is_some() {
        return Err(RelayError::Usage(
            "--cap-credits and --cap-gpu-secs cannot be used together".to_owned(),
        ));
    }

    Ok(CliCommand::Turn(TurnArgs {
        game_dir: game_dir
            .ok_or_else(|| RelayError::Usage("missing required --game-dir <dir>".to_owned()))?,
        day: day.ok_or_else(|| RelayError::Usage("missing required --day <DD>".to_owned()))?,
        round: round
            .ok_or_else(|| RelayError::Usage("missing required --round <talk|act>".to_owned()))?,
        seats_path: seats_path
            .ok_or_else(|| RelayError::Usage("missing required --seats <file>".to_owned()))?,
        wrapper_path,
        memory_mode,
        response_schema_path,
        scratchpad_requested,
        mixed_scratchpad_arm,
        live,
        cap_credits,
        cap_gpu_secs,
        allowed_vm_hosts,
        mock_dir,
        preview_tool,
        error_format,
        log_level,
    }))
}

fn parse_communication_cli_flag(raw: &str) -> Result<(), RelayError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "talk" => Ok(()),
        "silent" => Err(RelayError::SilentMarketEvalOnly(
            "`--communication silent` is refused in Play mode; use `league-eval`".to_owned(),
        )),
        _ => Err(RelayError::Usage(
            "--communication must be 'talk' or 'silent'".to_owned(),
        )),
    }
}

fn take_flag_value<'a>(
    flag: &str,
    inline: Option<&'a str>,
    tokens: &'a [String],
    idx: usize,
) -> Result<(&'a str, usize), RelayError> {
    let (candidate, step) = match inline {
        Some(v) => (Some(v), 1),
        None => (tokens.get(idx + 1).map(String::as_str), 2),
    };
    match candidate {
        Some(v) if !v.is_empty() => Ok((v, step)),
        _ => Err(RelayError::Usage(format!(
            "option '{flag}' requires a non-empty value"
        ))),
    }
}

fn is_forbidden_key_flag(flag: &str) -> bool {
    let lower = flag.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "-k" | "--key"
            | "--api-key"
            | "--apikey"
            | "--openrouter-key"
            | "--openrouter-api-key"
            | "--openai-api-key"
            | "--vllm-api-key"
            | "--local-api-key"
            | "--token"
            | "--secret"
            | "--bearer"
    ) || lower.ends_with("-api-key")
}

fn looks_like_cli_secret(raw: &str) -> bool {
    let lower = raw.trim().to_ascii_lowercase();
    lower.starts_with("sk-")
        || lower.contains("sk-or-")
        || lower.contains("sk-proj-")
        || lower.starts_with("bearer ")
}

fn sanitize_flag_name(raw: &str, redaction_key: Option<&ApiKey>) -> String {
    let scrubbed_raw = redact_secrets(raw, redaction_key);
    let head = scrubbed_raw.split('=').next().unwrap_or(&scrubbed_raw);
    truncate_after_redaction(head, 48)
}

/// Transport used when every seat in the file uses the `file` backend.
pub struct NoopManualTransport;

impl Transport for NoopManualTransport {
    fn post_chat_completions(
        &self,
        _url: &str,
        _body: &str,
        _timeout: Duration,
    ) -> Result<RawHttpResponse, TransportError> {
        Err(TransportError::Network(
            "manual-only transport cannot send network requests".to_owned(),
        ))
    }
}

struct PanicHookLevelGuard {
    previous: LogLevel,
}

impl Drop for PanicHookLevelGuard {
    fn drop(&mut self) {
        set_panic_hook_log_level(self.previous);
    }
}

fn read_named_env<E>(env_reader: &mut E, name: &str) -> Option<String>
where
    E: FnMut(&str) -> Option<String>,
{
    let probe = env_reader("__LEAGUE_RELAY_UNSET_PROBE__");
    let value = env_reader(name)?;
    if probe.as_deref() == Some(value.as_str()) {
        None
    } else {
        Some(value)
    }
}

fn resolve_log_level<E>(
    cli_override: Option<LogLevel>,
    env_reader: &mut E,
) -> Result<LogLevel, RelayError>
where
    E: FnMut(&str) -> Option<String>,
{
    if let Some(level) = cli_override {
        return Ok(level);
    }
    if let Some(raw_env) = read_named_env(env_reader, LEAGUE_LOG_LEVEL_ENV)
        && !raw_env.trim().is_empty()
    {
        return LogLevel::parse(&raw_env);
    }
    Ok(LogLevel::Normal)
}

/// Run CLI command with an injected environment lookup and output streams.
///
/// Unified exit codes (`SHIP_GATE.md` `:27`):
/// - `0` ([`EXIT_OK`]): clean completion, `--help`, or passing `prebook-check`.
/// - `1` ([`EXIT_USER_ERROR`]): invalid CLI arguments, forbidden secret placement, config/setup, or live-gate refusal.
/// - `2` ([`EXIT_RUNTIME_ERROR`]): runtime failure (`TURN_LOCKED`, `UNRECORDED_CHARGE`, I/O, corrupt log, failed `prebook-check`, or zero-clean `turn`).
/// - `3` ([`EXIT_PARTIAL_SUCCESS`]): `turn` recorded cleanly with `>= 1` clean seat and `>= 1` failed seat.
pub fn execute_cli<I, S, E>(
    args: I,
    mut env_reader: E,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
    E: FnMut(&str) -> Option<String>,
{
    let tokens: Vec<String> = args.into_iter().map(|s| s.as_ref().to_owned()).collect();
    let mut error_format = infer_error_format_best_effort(&tokens);
    let mut resolved_log_level = infer_log_level_best_effort(&tokens, |k| env_reader(k));

    let env_key = ApiKey::from_env_reader(|k| env_reader(k));
    let mut redaction_keys: Vec<ApiKey> = Vec::new();
    if let Ok(ref primary) = env_key {
        redaction_keys.push(primary.clone());
    }
    for extra in collect_env_redaction_keys(|k| env_reader(k)) {
        if !redaction_keys
            .iter()
            .any(|existing| existing.secret_str() == extra.secret_str())
        {
            redaction_keys.push(extra);
        }
    }
    let any_forbidden_local_key = env_key.is_ok()
        || FORBIDDEN_LOCAL_KEY_ENVS.iter().any(|env_name| {
            *env_name != OPENROUTER_API_KEY_ENV
                && env_reader(env_name).is_some_and(|val| !val.trim().is_empty())
        });

    let _panic_level_guard = PanicHookLevelGuard {
        previous: panic_hook_log_level(),
    };
    set_panic_hook_log_level(resolved_log_level);

    let mut diag_lines: Vec<String> = Vec::new();
    let outcome = run_cli_inner(
        &tokens,
        &env_key,
        any_forbidden_local_key,
        &redaction_keys,
        &mut env_reader,
        &mut error_format,
        &mut resolved_log_level,
        &mut diag_lines,
        stdout,
    );

    for line in &diag_lines {
        let _ = writeln!(stderr, "{line}");
    }

    match outcome {
        Ok(CliRunOutcome::Turn(summary)) => {
            if resolved_log_level != LogLevel::Silent {
                let rendered = redact_secrets_multi_with_limit(
                    &summary.format_paths_and_hashes(),
                    &redaction_keys,
                    MAX_UNRECORDED_CHARGE_SCAN_BYTES,
                );
                let _ = write!(stdout, "{rendered}");
            }
            summary.exit_code()
        }
        Ok(CliRunOutcome::PrebookOk | CliRunOutcome::Help | CliRunOutcome::WhisperTickOk) => {
            EXIT_OK
        }
        Ok(CliRunOutcome::PrebookSeatFailed) => EXIT_RUNTIME_ERROR,
        Err(e) => {
            let exit_code = e.exit_code();
            let scan_limit = e.redaction_scan_limit();
            let structured = scrub_structured_error_with_keys(
                &e.to_structured_error(),
                &redaction_keys,
                scan_limit,
            );
            match error_format {
                ErrorFormat::Text => {
                    let formatted = redact_secrets_multi_with_limit(
                        &structured.format_text(),
                        &redaction_keys,
                        MAX_UNRECORDED_CHARGE_SCAN_BYTES,
                    );
                    let _ = writeln!(stderr, "error: {formatted}");
                }
                ErrorFormat::Json => match structured.to_json_string() {
                    Ok(json) => {
                        let redacted = redact_secrets_multi_with_limit(
                            &json,
                            &redaction_keys,
                            MAX_UNRECORDED_CHARGE_SCAN_BYTES,
                        );
                        let _ = writeln!(stderr, "{redacted}");
                    }
                    Err(_) => {
                        let _ = writeln!(
                            stderr,
                            "{{\"code\":{},\"message\":\"internal error serialization failure\",\"hint\":\"report this bug\",\"retryable\":false}}",
                            serde_json::Value::String(structured.code.to_owned())
                        );
                    }
                },
            }
            exit_code
        }
    }
}

enum CliRunOutcome {
    Help,
    Turn(TurnSummary),
    PrebookOk,
    PrebookSeatFailed,
    WhisperTickOk,
}

fn parse_prebook_check_args(
    tokens: &[String],
    redaction_key: Option<&ApiKey>,
    mut error_format: ErrorFormat,
    mut seen_error_format: Option<ErrorFormat>,
    mut log_level: Option<LogLevel>,
) -> Result<CliCommand, RelayError> {
    let mut seats_path: Option<PathBuf> = None;
    let mut wrapper_path: Option<PathBuf> = None;
    let mut response_schema_path: Option<PathBuf> = None;
    let mut memory_mode = MemoryMode::default();
    let mut scratchpad_requested: Option<bool> = None;
    let mut mixed_scratchpad_arm = false;
    let mut allowed_vm_hosts: Vec<String> = Vec::new();
    let mut preview_tool = PreviewToolMode::default();

    let mut idx = 1;
    while idx < tokens.len() {
        let raw_arg = &tokens[idx];
        if matches!(raw_arg.as_str(), "-h" | "--help") {
            return Ok(CliCommand::Help);
        }
        let (flag, inline_val) = match raw_arg.split_once('=') {
            Some((f, v)) => (f, Some(v)),
            None => (raw_arg.as_str(), None),
        };
        if is_forbidden_key_flag(flag) || looks_like_cli_secret(raw_arg) {
            return Err(RelayError::SecretInForbiddenPlace(
                "API key must be read from OPENROUTER_API_KEY only, never passed as an argument"
                    .to_owned(),
            ));
        }
        if let Some(step) = try_consume_hygiene_flag(
            flag,
            inline_val,
            tokens,
            idx,
            &mut error_format,
            &mut seen_error_format,
            &mut log_level,
        )? {
            idx += step;
            continue;
        }
        match flag {
            "--seats" => {
                let (val, step) = take_flag_value(flag, inline_val, tokens, idx)?;
                seats_path = Some(PathBuf::from(val));
                idx += step;
            }
            "--wrapper" => {
                let (val, step) = take_flag_value(flag, inline_val, tokens, idx)?;
                wrapper_path = Some(PathBuf::from(val));
                idx += step;
            }
            "--response-schema" => {
                let (val, step) = take_flag_value(flag, inline_val, tokens, idx)?;
                response_schema_path = Some(PathBuf::from(val));
                idx += step;
            }
            "--memory-mode" => {
                let (val, step) = take_flag_value(flag, inline_val, tokens, idx)?;
                memory_mode = MemoryMode::parse(val)?;
                idx += step;
            }
            "--scratchpad" => {
                let req = match inline_val {
                    None | Some("true") => true,
                    Some("false") => false,
                    Some(_) => {
                        return Err(RelayError::Usage(
                            "--scratchpad takes no value (or '=true' / '=false')".to_owned(),
                        ));
                    }
                };
                if scratchpad_requested.is_some_and(|prev| prev != req) {
                    return Err(RelayError::Usage(
                        "--scratchpad and --no-scratchpad cannot be used together".to_owned(),
                    ));
                }
                scratchpad_requested = Some(req);
                idx += 1;
            }
            "--no-scratchpad" => {
                if inline_val.is_some() {
                    return Err(RelayError::Usage(
                        "--no-scratchpad takes no value".to_owned(),
                    ));
                }
                if scratchpad_requested == Some(true) {
                    return Err(RelayError::Usage(
                        "--scratchpad and --no-scratchpad cannot be used together".to_owned(),
                    ));
                }
                scratchpad_requested = Some(false);
                idx += 1;
            }
            "--mixed-scratchpad-arm" => {
                if inline_val.is_some() {
                    return Err(RelayError::Usage(
                        "--mixed-scratchpad-arm takes no value".to_owned(),
                    ));
                }
                mixed_scratchpad_arm = true;
                idx += 1;
            }
            "--allow-vm-host" => {
                let (val, step) = take_flag_value(flag, inline_val, tokens, idx)?;
                let normalized = validate_vm_host_allowance(val)?;
                if !allowed_vm_hosts.contains(&normalized) {
                    allowed_vm_hosts.push(normalized);
                }
                idx += step;
            }
            "--preview-tool" => {
                let (val, step) = take_flag_value(flag, inline_val, tokens, idx)?;
                preview_tool = PreviewToolMode::parse(val)?;
                idx += step;
            }
            "--communication" => {
                let (val, step) = take_flag_value(flag, inline_val, tokens, idx)?;
                parse_communication_cli_flag(val)?;
                idx += step;
            }
            "--live" => {
                return Err(RelayError::Usage(
                    "prebook-check runs offline against mock transport only; omit --live"
                        .to_owned(),
                ));
            }
            other => {
                if !other.starts_with('-') {
                    return Err(RelayError::Usage(
                        "unexpected positional argument (all options require a '--flag')"
                            .to_owned(),
                    ));
                }
                return Err(RelayError::Usage(format!(
                    "unknown option '{}'",
                    sanitize_flag_name(other, redaction_key)
                )));
            }
        }
    }

    Ok(CliCommand::PrebookCheck(PrebookCheckArgs {
        seats_path: seats_path
            .ok_or_else(|| RelayError::Usage("missing required --seats <file>".to_owned()))?,
        wrapper_path: wrapper_path
            .ok_or_else(|| RelayError::Usage("missing required --wrapper <path>".to_owned()))?,
        response_schema_path,
        memory_mode,
        scratchpad_requested,
        mixed_scratchpad_arm,
        allowed_vm_hosts,
        preview_tool,
        error_format,
        log_level,
    }))
}

fn parse_whisper_tick_args(
    tokens: &[String],
    redaction_key: Option<&ApiKey>,
    mut error_format: ErrorFormat,
    mut seen_error_format: Option<ErrorFormat>,
    mut log_level: Option<LogLevel>,
) -> Result<CliCommand, RelayError> {
    let mut table_keys_path: Option<PathBuf> = None;
    let mut envelopes_path: Option<PathBuf> = None;
    let mut out_dir: Option<PathBuf> = None;
    let mut table_id: Option<u64> = None;
    let mut turn: Option<u8> = None;
    let mut sub_round: Option<u8> = None;
    let mut state_file: Option<PathBuf> = None;
    let mut game_dir: Option<PathBuf> = None;
    let mut day: Option<String> = None;
    let mut batna_file: Option<PathBuf> = None;

    let mut idx = 1;
    while idx < tokens.len() {
        let raw_arg = &tokens[idx];
        if matches!(raw_arg.as_str(), "-h" | "--help") {
            return Ok(CliCommand::Help);
        }
        let (flag, inline_val) = match raw_arg.split_once('=') {
            Some((f, v)) => (f, Some(v)),
            None => (raw_arg.as_str(), None),
        };
        if is_forbidden_key_flag(flag) || looks_like_cli_secret(raw_arg) {
            return Err(RelayError::SecretInForbiddenPlace(
                "API key must be read from OPENROUTER_API_KEY only, never passed as an argument"
                    .to_owned(),
            ));
        }
        if let Some(step) = try_consume_hygiene_flag(
            flag,
            inline_val,
            tokens,
            idx,
            &mut error_format,
            &mut seen_error_format,
            &mut log_level,
        )? {
            idx += step;
            continue;
        }
        match flag {
            "--table-keys" => {
                let (val, step) = take_flag_value(flag, inline_val, tokens, idx)?;
                table_keys_path = Some(PathBuf::from(val));
                idx += step;
            }
            "--envelopes" => {
                let (val, step) = take_flag_value(flag, inline_val, tokens, idx)?;
                envelopes_path = Some(PathBuf::from(val));
                idx += step;
            }
            "--out-dir" => {
                let (val, step) = take_flag_value(flag, inline_val, tokens, idx)?;
                out_dir = Some(PathBuf::from(val));
                idx += step;
            }
            "--table-id" => {
                let (val, step) = take_flag_value(flag, inline_val, tokens, idx)?;
                let parsed = val.parse::<u64>().map_err(|_| {
                    RelayError::Usage("--table-id must be a non-negative integer".to_owned())
                })?;
                table_id = Some(parsed);
                idx += step;
            }
            "--turn" => {
                let (val, step) = take_flag_value(flag, inline_val, tokens, idx)?;
                let parsed = val.parse::<u8>().ok().filter(|n| *n >= 1).ok_or_else(|| {
                    RelayError::Usage("--turn must be an integer >= 1".to_owned())
                })?;
                turn = Some(parsed);
                idx += step;
            }
            "--sub-round" => {
                let (val, step) = take_flag_value(flag, inline_val, tokens, idx)?;
                let parsed = val.parse::<u8>().ok().filter(|n| *n >= 1).ok_or_else(|| {
                    RelayError::Usage("--sub-round must be an integer >= 1".to_owned())
                })?;
                sub_round = Some(parsed);
                idx += step;
            }
            "--state-file" => {
                let (val, step) = take_flag_value(flag, inline_val, tokens, idx)?;
                state_file = Some(PathBuf::from(val));
                idx += step;
            }
            "--game-dir" => {
                let (val, step) = take_flag_value(flag, inline_val, tokens, idx)?;
                game_dir = Some(PathBuf::from(val));
                idx += step;
            }
            "--day" => {
                let (val, step) = take_flag_value(flag, inline_val, tokens, idx)?;
                validate_day(val)?;
                day = Some(val.to_owned());
                idx += step;
            }
            "--batna-file" => {
                let (val, step) = take_flag_value(flag, inline_val, tokens, idx)?;
                batna_file = Some(PathBuf::from(val));
                idx += step;
            }
            other => {
                if !other.starts_with('-') {
                    return Err(RelayError::Usage(
                        "unexpected positional argument (all options require a '--flag')"
                            .to_owned(),
                    ));
                }
                return Err(RelayError::Usage(format!(
                    "unknown option '{}'",
                    sanitize_flag_name(other, redaction_key)
                )));
            }
        }
    }

    let neg_flags_present = [game_dir.is_some(), day.is_some(), batna_file.is_some()]
        .into_iter()
        .filter(|b| *b)
        .count();
    if neg_flags_present != 0 && neg_flags_present != 3 {
        return Err(RelayError::Usage(
            "--game-dir, --day, and --batna-file must be supplied together".to_owned(),
        ));
    }

    Ok(CliCommand::WhisperTick(WhisperTickArgs {
        table_keys_path: table_keys_path
            .ok_or_else(|| RelayError::Usage("missing required --table-keys <file>".to_owned()))?,
        envelopes_path: envelopes_path
            .ok_or_else(|| RelayError::Usage("missing required --envelopes <file>".to_owned()))?,
        out_dir: out_dir
            .ok_or_else(|| RelayError::Usage("missing required --out-dir <dir>".to_owned()))?,
        table_id: table_id
            .ok_or_else(|| RelayError::Usage("missing required --table-id <id>".to_owned()))?,
        turn: turn.ok_or_else(|| RelayError::Usage("missing required --turn <N>".to_owned()))?,
        sub_round: sub_round
            .ok_or_else(|| RelayError::Usage("missing required --sub-round <K>".to_owned()))?,
        state_file,
        game_dir,
        day,
        batna_file,
        error_format,
        log_level,
    }))
}

fn run_whisper_tick(
    args: WhisperTickArgs,
    redaction_keys: &[ApiKey],
    stdout: &mut dyn Write,
    log_level: LogLevel,
) -> Result<CliRunOutcome, RelayError> {
    let redact_ref = redaction_keys.first();
    let keys_raw = fs::read_to_string(&args.table_keys_path).map_err(|e| {
        RelayError::Io(redact_secrets(
            &format!("could not read {}: {e}", args.table_keys_path.display()),
            redact_ref,
        ))
    })?;
    let config: WhisperTableConfig = serde_json::from_str(&keys_raw).map_err(|e| {
        RelayError::SeatConfig(redact_secrets(
            &format!("invalid whisper table keys JSON: {e}"),
            redact_ref,
        ))
    })?;
    config.validate()?;
    if config.table_id.0 != args.table_id {
        return Err(RelayError::IncompatibleSetup(format!(
            "whisper table keys table_id {} != --table-id {}",
            config.table_id.0, args.table_id
        )));
    }

    let env_raw = fs::read_to_string(&args.envelopes_path).map_err(|e| {
        RelayError::Io(redact_secrets(
            &format!("could not read {}: {e}", args.envelopes_path.display()),
            redact_ref,
        ))
    })?;
    let env_val: serde_json::Value = serde_json::from_str(&env_raw).map_err(|e| {
        RelayError::Usage(redact_secrets(
            &format!("invalid --envelopes JSON: {e}"),
            redact_ref,
        ))
    })?;
    let env_items: &[serde_json::Value] = if let Some(arr) = env_val.as_array() {
        arr.as_slice()
    } else if let Some(arr) = env_val.get("envelopes").and_then(|v| v.as_array()) {
        arr.as_slice()
    } else {
        return Err(RelayError::Usage(
            "--envelopes JSON must be an array or an object with an 'envelopes' array".to_owned(),
        ));
    };

    let mut coord = match &args.state_file {
        Some(path) if path.exists() => {
            let state_raw = fs::read_to_string(path).map_err(|e| {
                RelayError::Io(redact_secrets(
                    &format!("could not read {}: {e}", path.display()),
                    redact_ref,
                ))
            })?;
            let snap: WhisperRelayStateSnapshot =
                serde_json::from_str(&state_raw).map_err(|e| {
                    RelayError::CorruptLog(redact_secrets(
                        &format!("invalid whisper state JSON in {}: {e}", path.display()),
                        redact_ref,
                    ))
                })?;
            WhisperRelayCoordinator::from_snapshot(
                config.clone(),
                args.turn,
                args.sub_round,
                &snap,
            )?
        }
        _ => WhisperRelayCoordinator::new(config.clone(), args.turn, args.sub_round)?,
    };

    for item in env_items {
        let _ = coord.ingest_raw_envelope_value(item);
    }
    let release = coord.release_sub_round()?;

    fs::create_dir_all(&args.out_dir).map_err(|e| {
        RelayError::Io(redact_secrets(
            &format!("could not create {}: {e}", args.out_dir.display()),
            redact_ref,
        ))
    })?;
    for batch in &release.batches {
        let out_file = args.out_dir.join(format!(
            "turn-{:02}-sub-{:02}-seat-{}-{}.whispers.txt",
            release.turn, release.sub_round, batch.recipient_seat, batch.recipient_guild
        ));
        let mut content = batch.rendered_frames.join("\n");
        if !content.is_empty() {
            content.push('\n');
        }
        fs::write(&out_file, content).map_err(|e| {
            RelayError::Io(redact_secrets(
                &format!("could not write {}: {e}", out_file.display()),
                redact_ref,
            ))
        })?;
    }
    let report_path = args.out_dir.join("whisper-tick-report.json");
    let report_pretty = serde_json::to_string_pretty(&release)
        .map_err(|e| RelayError::Io(format!("failed to serialize whisper tick report: {e}")))?;
    fs::write(&report_path, report_pretty).map_err(|e| {
        RelayError::Io(redact_secrets(
            &format!("could not write {}: {e}", report_path.display()),
            redact_ref,
        ))
    })?;

    if let (Some(game_dir), Some(day), Some(batna_path)) =
        (&args.game_dir, &args.day, &args.batna_file)
    {
        let batna_raw = fs::read_to_string(batna_path).map_err(|e| {
            RelayError::Io(redact_secrets(
                &format!("could not read {}: {e}", batna_path.display()),
                redact_ref,
            ))
        })?;
        if batna_raw.trim().is_empty() {
            return Err(RelayError::Usage(
                "--batna-file must contain non-empty fallback text".to_owned(),
            ));
        }
        let to_seat_dir = game_dir.join("to-seat");
        fs::create_dir_all(&to_seat_dir).map_err(|e| {
            RelayError::Io(redact_secrets(
                &format!("could not create {}: {e}", to_seat_dir.display()),
                redact_ref,
            ))
        })?;
        for batch in &release.batches {
            let neg = batch_to_negotiation_inputs(batch, &batna_raw)?;
            let neg_path = to_seat_dir.join(format!(
                "day-{day}-act-{}.negotiation.json",
                batch.recipient_guild
            ));
            let neg_json = serde_json::to_string_pretty(&neg).map_err(|e| {
                RelayError::Io(format!("failed to serialize negotiation JSON: {e}"))
            })?;
            fs::write(&neg_path, neg_json).map_err(|e| {
                RelayError::Io(redact_secrets(
                    &format!("could not write {}: {e}", neg_path.display()),
                    redact_ref,
                ))
            })?;
        }
    }

    if let Some(state_path) = &args.state_file {
        if let Some(parent) = state_path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|e| {
                RelayError::Io(redact_secrets(
                    &format!("could not create {}: {e}", parent.display()),
                    redact_ref,
                ))
            })?;
        }
        let snap_json = serde_json::to_string_pretty(&coord.to_snapshot())
            .map_err(|e| RelayError::Io(format!("failed to serialize whisper state: {e}")))?;
        fs::write(state_path, snap_json).map_err(|e| {
            RelayError::Io(redact_secrets(
                &format!("could not write {}: {e}", state_path.display()),
                redact_ref,
            ))
        })?;
    }

    if log_level != LogLevel::Silent {
        let compact = serde_json::to_string(&release)
            .map_err(|e| RelayError::Io(format!("failed to serialize whisper tick report: {e}")))?;
        let _ = writeln!(stdout, "WHISPER_TICK_REPORT: {compact}");
    }

    Ok(CliRunOutcome::WhisperTickOk)
}

struct TempPrebookDir {
    root: PathBuf,
}

impl TempPrebookDir {
    fn create() -> Result<Self, RelayError> {
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        let base = std::env::temp_dir();
        let pid = std::process::id();
        for _ in 0..1024 {
            let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
            let candidate = base.join(format!("league-relay-prebook-{pid}-{seq}"));
            match fs::create_dir(&candidate) {
                Ok(()) => return Ok(Self { root: candidate }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => {
                    return Err(RelayError::Io(format!(
                        "could not create temporary prebook-check dir {}: {e}",
                        candidate.display()
                    )));
                }
            }
        }
        Err(RelayError::Io(
            "could not allocate unique temporary prebook-check directory".to_owned(),
        ))
    }
}

impl Drop for TempPrebookDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn resolve_local_schema_ref<'a>(
    root_schema: &'a serde_json::Value,
    ref_str: &str,
) -> Option<&'a serde_json::Value> {
    let pointer = ref_str.strip_prefix('#')?;
    root_schema.pointer(pointer)
}

fn synthesize_mock_value_for_schema(
    schema: &serde_json::Value,
    root_schema: &serde_json::Value,
    pad_key: &str,
    is_root_pad_field: bool,
    depth: usize,
) -> serde_json::Value {
    if is_root_pad_field || depth > 16 {
        return serde_json::Value::String(String::new());
    }
    let Some(obj) = schema.as_object() else {
        return serde_json::json!({});
    };
    if let Some(ref_s) = obj.get("$ref").and_then(|v| v.as_str())
        && let Some(target) = resolve_local_schema_ref(root_schema, ref_s)
    {
        return synthesize_mock_value_for_schema(target, root_schema, pad_key, false, depth + 1);
    }
    if let Some(c) = obj.get("const") {
        return c.clone();
    }
    if let Some(arr) = obj.get("enum").and_then(|v| v.as_array())
        && let Some(first) = arr.first()
    {
        return first.clone();
    }
    for branch_kw in ["anyOf", "oneOf"] {
        if let Some(arr) = obj.get(branch_kw).and_then(|v| v.as_array())
            && let Some(first) = arr.first()
        {
            return synthesize_mock_value_for_schema(first, root_schema, pad_key, false, depth + 1);
        }
    }
    let ty = obj.get("type").and_then(|v| v.as_str());
    if ty == Some("object")
        || (ty.is_none() && (obj.contains_key("properties") || obj.contains_key("required")))
    {
        let mut out_map = serde_json::Map::new();
        let props_opt = obj.get("properties").and_then(|v| v.as_object());
        if let Some(req_arr) = obj.get("required").and_then(|v| v.as_array()) {
            for req_k in req_arr.iter().filter_map(|v| v.as_str()) {
                let is_pad = depth == 0 && req_k == pad_key;
                let sub = props_opt
                    .and_then(|p| p.get(req_k))
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({"type": "string"}));
                out_map.insert(
                    req_k.to_owned(),
                    synthesize_mock_value_for_schema(&sub, root_schema, pad_key, is_pad, depth + 1),
                );
            }
        }
        return serde_json::Value::Object(out_map);
    }
    match ty {
        Some("array") => serde_json::Value::Array(Vec::new()),
        Some("string") => serde_json::Value::String("ok".to_owned()),
        Some("integer") => serde_json::json!(0),
        Some("number") => serde_json::json!(0.0),
        Some("boolean") => serde_json::Value::Bool(false),
        Some("null") => serde_json::Value::Null,
        _ => serde_json::json!({}),
    }
}

fn build_prebook_mock_responses_json(response_format: Option<&ResponseFormatConfig>) -> String {
    let pad_key = wording_slot("scratchpad_schema_key");
    let pinned_pad_desc = wording_slot("scratchpad_schema_description");
    let make_rule = |when_contains: &str, content: String| {
        serde_json::json!({
            "when_contains": when_contains,
            "status": 200,
            "response": {
                "id": "prebook-mock-1",
                "model": "prebook-mock",
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": content
                    }
                }],
                "usage": {
                    "prompt_tokens": 16,
                    "completion_tokens": 8,
                    "total_tokens": 24,
                    "cost": 0.0
                }
            }
        })
    };
    let rules = match response_format {
        Some(rf) if rf.schema_has_scratchpad_property() => {
            let with_pad =
                synthesize_mock_value_for_schema(&rf.schema, &rf.schema, &pad_key, false, 0)
                    .to_string();
            let stripped = rf.without_scratchpad_property();
            let without_pad = synthesize_mock_value_for_schema(
                &stripped.schema,
                &stripped.schema,
                &pad_key,
                false,
                0,
            )
            .to_string();
            vec![
                make_rule(&pinned_pad_desc, with_pad),
                make_rule("", without_pad),
            ]
        }
        Some(rf) => {
            let content =
                synthesize_mock_value_for_schema(&rf.schema, &rf.schema, &pad_key, false, 0)
                    .to_string();
            vec![make_rule("", content)]
        }
        None => vec![make_rule("", "PREBOOK_MOCK_OK\n".to_owned())],
    };
    serde_json::json!({ "rules": rules }).to_string()
}

fn infer_prebook_vm_hosts(seats_path: &std::path::Path) -> Vec<String> {
    let Ok(raw) = fs::read_to_string(seats_path) else {
        return Vec::new();
    };
    let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return Vec::new();
    };
    let entries = if let Some(arr) = val.as_array() {
        arr.as_slice()
    } else if let Some(arr) = val.get("seats").and_then(|s| s.as_array()) {
        arr.as_slice()
    } else {
        return Vec::new();
    };
    let mut hosts = Vec::new();
    for entry in entries {
        if entry.get("backend").and_then(|b| b.as_str()) != Some("local") {
            continue;
        }
        let Some(base_url) = entry.get("base_url").and_then(|u| u.as_str()) else {
            continue;
        };
        let trimmed = base_url.trim();
        let Some(after_scheme) = trimmed
            .strip_prefix("http://")
            .or_else(|| trimmed.strip_prefix("https://"))
        else {
            continue;
        };
        let authority = after_scheme.split('/').next().unwrap_or(after_scheme);
        let host_part = if let Some(rest) = authority.strip_prefix('[') {
            rest.split(']').next().unwrap_or("")
        } else {
            authority.split(':').next().unwrap_or("")
        };
        if let Ok(ip) = host_part.parse::<std::net::IpAddr>() {
            let is_private_or_ula = match ip {
                std::net::IpAddr::V4(v4) => v4.is_private(),
                std::net::IpAddr::V6(v6) => {
                    v6.to_ipv4_mapped().is_none()
                        && !v6.is_unicast_link_local()
                        && v6.is_unique_local()
                }
            };
            if is_private_or_ula
                && let Ok(norm) = validate_vm_host_allowance(host_part)
                && !hosts.contains(&norm)
            {
                hosts.push(norm);
            }
        }
    }
    hosts
}

#[allow(clippy::too_many_arguments)]
fn run_prebook_check(
    args: PrebookCheckArgs,
    any_forbidden_local_key: bool,
    redaction_keys: &[ApiKey],
    stdout: &mut dyn Write,
    log_level: LogLevel,
    diag_sink: &RelayDiagSink,
    diag_lines: &mut Vec<String>,
) -> Result<CliRunOutcome, RelayError> {
    let redact_ref = redaction_keys.first();
    if any_forbidden_local_key {
        return Err(RelayError::SecretInForbiddenPlace(
            "prebook-check refuses API keys; unset OPENROUTER_API_KEY, OPENAI_API_KEY, VLLM_API_KEY, and LOCAL_API_KEY".to_owned(),
        ));
    }

    let effective_vm_hosts = if args.allowed_vm_hosts.is_empty() {
        infer_prebook_vm_hosts(&args.seats_path)
    } else {
        args.allowed_vm_hosts.clone()
    };

    let seats =
        SeatFile::load_with_allowed_hosts(&args.seats_path, redact_ref, &effective_vm_hosts)?;
    let response_format = match &args.response_schema_path {
        Some(path) => Some(ResponseFormatConfig::load(path, redact_ref)?),
        None => None,
    };

    let temp = TempPrebookDir::create()?;
    let game_dir = temp.root.join("game");
    let to_seat_dir = game_dir.join("to-seat");
    let mock_dir = temp.root.join("mock");
    fs::create_dir_all(&to_seat_dir).map_err(|e| {
        RelayError::Io(redact_secrets(
            &format!("could not create {}: {e}", to_seat_dir.display()),
            redact_ref,
        ))
    })?;
    fs::create_dir_all(&mock_dir).map_err(|e| {
        RelayError::Io(redact_secrets(
            &format!("could not create {}: {e}", mock_dir.display()),
            redact_ref,
        ))
    })?;

    for seat in &seats.seats {
        let seat_in = to_seat_dir.join(format!("day-01-talk-{}.md", seat.guild));
        fs::write(&seat_in, "PREBOOK CHECK PUBLIC STATE\n").map_err(|e| {
            RelayError::Io(redact_secrets(
                &format!("could not write {}: {e}", seat_in.display()),
                redact_ref,
            ))
        })?;
    }

    let responses_path = mock_dir.join("responses.json");
    fs::write(
        &responses_path,
        build_prebook_mock_responses_json(response_format.as_ref()),
    )
    .map_err(|e| {
        RelayError::Io(redact_secrets(
            &format!("could not write {}: {e}", responses_path.display()),
            redact_ref,
        ))
    })?;

    let (cap_credits, cap_gpu_secs) = if seats.has_local_seat() {
        let sum_gpu: f64 = seats
            .seats
            .iter()
            .filter_map(|s| match &s.backend {
                SeatBackend::Local(l) => Some(l.max_gpu_secs_per_call),
                _ => None,
            })
            .sum();
        (None, Some(sum_gpu * 2.0 + 60.0))
    } else if seats.has_openrouter_seat() {
        let sum_cred: f64 = seats
            .seats
            .iter()
            .filter_map(|s| match &s.backend {
                SeatBackend::Openrouter(o) => Some(o.max_cost_per_call),
                _ => None,
            })
            .sum();
        (Some(sum_cred * 2.0 + 10.0), None)
    } else {
        (None, None)
    };

    let cfg = TurnConfig::new(
        game_dir.clone(),
        "01".to_owned(),
        Round::Talk,
        seats.clone(),
        Some(args.wrapper_path),
        false,
        cap_credits,
    )
    .with_memory_mode(args.memory_mode)
    .with_cap_gpu_secs(cap_gpu_secs)
    .with_response_format(response_format)
    .with_scratchpad(args.scratchpad_requested)
    .with_mixed_scratchpad_arm(seats.mixed_scratchpad_arm || args.mixed_scratchpad_arm)
    .with_allowed_vm_hosts(effective_vm_hosts)
    .with_log_level(log_level)
    .with_preview_tool(args.preview_tool)
    .with_extra_redaction_keys(redaction_keys.to_vec());

    validate_scratchpad_setup(&cfg)?;

    let turn_res = if seats.has_local_seat() || seats.has_openrouter_seat() {
        let mock_transport = MockDirTransport::load_with_key(&mock_dir, redact_ref)?;
        run_turn_with_diag(&cfg, &mock_transport, redact_ref, Some(diag_sink))
    } else {
        run_turn_with_diag(&cfg, &NoopManualTransport, redact_ref, Some(diag_sink))
    };
    diag_lines.extend(diag_sink.drain_redacted(redaction_keys));
    let summary = turn_res?;

    let setup_raw = fs::read_to_string(game_dir.join("relay/setup.json")).map_err(|e| {
        RelayError::Io(redact_secrets(
            &format!("could not read setup.json after prebook-check: {e}"),
            redact_ref,
        ))
    })?;
    let setup: GameSetupRecord = serde_json::from_str(&setup_raw).map_err(|e| {
        RelayError::CorruptLog(redact_secrets(
            &format!("invalid setup.json after prebook-check: {e}"),
            redact_ref,
        ))
    })?;

    if log_level != LogLevel::Silent {
        for (seat_spec, report) in seats.seats.iter().zip(summary.reports.iter()) {
            let backend_label = match seat_spec.backend {
                SeatBackend::Local(_) => "local",
                SeatBackend::Openrouter(_) => "openrouter",
                SeatBackend::File { .. } => "file",
            };
            match &report.outcome {
                SeatOutcome::Delivered { attempts, .. } => {
                    let _ = writeln!(
                        stdout,
                        "prebook-check: seat={} backend={backend_label} outcome=OK attempts={attempts}",
                        report.guild
                    );
                }
                SeatOutcome::RelayedByHand => {
                    let _ = writeln!(
                        stdout,
                        "prebook-check: seat={} backend={backend_label} outcome=FILE_MANUAL",
                        report.guild
                    );
                }
                _ => {
                    let _ = writeln!(
                        stdout,
                        "prebook-check: seat={} backend={backend_label} outcome=FAILED",
                        report.guild
                    );
                }
            }
        }
    }

    if summary.all_clean() {
        if log_level != LogLevel::Silent {
            let _ = writeln!(
                stdout,
                "prebook-check: OK seats={} wording_arm={} memory_mode={} scratchpad=\"{}\" non_pinned_strings={}",
                seats.seats.len(),
                setup.wording_arm,
                setup.memory_mode,
                setup.scratchpad,
                setup.non_pinned_model_facing_strings.len()
            );
        }
        Ok(CliRunOutcome::PrebookOk)
    } else {
        Ok(CliRunOutcome::PrebookSeatFailed)
    }
}

#[allow(clippy::too_many_arguments)]
fn run_cli_inner<I, S, E>(
    args: I,
    env_key: &Result<ApiKey, RelayError>,
    any_forbidden_local_key: bool,
    redaction_keys: &[ApiKey],
    env_reader: &mut E,
    resolved_error_format: &mut ErrorFormat,
    resolved_log_level: &mut LogLevel,
    diag_lines: &mut Vec<String>,
    stdout: &mut dyn Write,
) -> Result<CliRunOutcome, RelayError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
    E: FnMut(&str) -> Option<String>,
{
    let redact_ref = redaction_keys.first();
    let turn_args = match parse_args_with_key(args, redact_ref)? {
        CliCommand::Help => {
            let _ = write!(stdout, "{USAGE_TEXT}");
            return Ok(CliRunOutcome::Help);
        }
        CliCommand::PrebookCheck(p) => {
            *resolved_error_format = p.error_format;
            let effective_log_level = resolve_log_level(p.log_level, env_reader)?;
            *resolved_log_level = effective_log_level;
            set_panic_hook_log_level(effective_log_level);
            let diag_sink = RelayDiagSink::new(effective_log_level);
            return run_prebook_check(
                p,
                any_forbidden_local_key,
                redaction_keys,
                stdout,
                effective_log_level,
                &diag_sink,
                diag_lines,
            );
        }
        CliCommand::WhisperTick(w) => {
            *resolved_error_format = w.error_format;
            let effective_log_level = resolve_log_level(w.log_level, env_reader)?;
            *resolved_log_level = effective_log_level;
            set_panic_hook_log_level(effective_log_level);
            return run_whisper_tick(w, redaction_keys, stdout, effective_log_level);
        }
        CliCommand::Turn(t) => t,
    };

    *resolved_error_format = turn_args.error_format;
    let effective_log_level = resolve_log_level(turn_args.log_level, env_reader)?;
    *resolved_log_level = effective_log_level;
    set_panic_hook_log_level(effective_log_level);
    let diag_sink = RelayDiagSink::new(effective_log_level);

    let seats = SeatFile::load_with_allowed_hosts(
        &turn_args.seats_path,
        redact_ref,
        &turn_args.allowed_vm_hosts,
    )?;

    if seats.has_local_seat() && any_forbidden_local_key {
        return Err(RelayError::SecretInForbiddenPlace(
            "local transport refuses API keys; unset OPENROUTER_API_KEY, OPENAI_API_KEY, and VLLM_API_KEY so keys never leak into VM logs".to_owned(),
        ));
    }

    let response_format = match &turn_args.response_schema_path {
        Some(path) => Some(ResponseFormatConfig::load(path, redact_ref)?),
        None => None,
    };

    let base_cfg = TurnConfig::new(
        turn_args.game_dir,
        turn_args.day,
        turn_args.round,
        seats.clone(),
        turn_args.wrapper_path,
        turn_args.live,
        turn_args.cap_credits,
    )
    .with_memory_mode(turn_args.memory_mode)
    .with_cap_gpu_secs(turn_args.cap_gpu_secs)
    .with_response_format(response_format)
    .with_scratchpad(turn_args.scratchpad_requested)
    .with_mixed_scratchpad_arm(turn_args.mixed_scratchpad_arm)
    .with_allowed_vm_hosts(turn_args.allowed_vm_hosts.clone())
    .with_log_level(effective_log_level)
    .with_preview_tool(turn_args.preview_tool)
    .with_extra_redaction_keys(redaction_keys.to_vec());

    validate_scratchpad_setup(&base_cfg)?;

    let turn_res = if let Some(mock_dir) = &turn_args.mock_dir {
        let mock_transport = MockDirTransport::load_with_key(mock_dir, redact_ref)?;
        let mut cfg = base_cfg;
        cfg.live = false;
        run_turn_with_diag(&cfg, &mock_transport, redact_ref, Some(&diag_sink))
    } else if seats.has_local_seat() {
        let local_gate = LocalGate::acquire(
            turn_args.live,
            turn_args.cap_gpu_secs,
            any_forbidden_local_key,
            &turn_args.allowed_vm_hosts,
        )?;
        let cfg = base_cfg.with_cap_gpu_secs(Some(local_gate.cap_gpu_secs()));
        let local_transport = LocalOpenAiTransport::new(local_gate);
        run_turn_with_diag(&cfg, &local_transport, None, Some(&diag_sink))
    } else if turn_args.live || seats.has_openrouter_seat() {
        let gate = LiveGate::acquire(turn_args.live, turn_args.cap_credits, env_key.clone())?;
        let mut cfg = base_cfg;
        cfg.live = true;
        cfg.cap_credits = Some(gate.cap_credits());
        let live_transport = UreqTransport::new(gate);
        run_turn_with_diag(&cfg, &live_transport, redact_ref, Some(&diag_sink))
    } else {
        run_turn_with_diag(
            &base_cfg,
            &NoopManualTransport,
            redact_ref,
            Some(&diag_sink),
        )
    };
    diag_lines.extend(diag_sink.drain_redacted(redaction_keys));
    Ok(CliRunOutcome::Turn(turn_res?))
}

#[cfg(test)]
mod silent_flag_tests {
    use super::*;
    use crate::relay::run_turn;
    use crate::seats::SeatFile;
    use std::path::PathBuf;

    #[test]
    fn play_mode_refuses_silent_communication_flag_and_turn_config() {
        assert!(parse_communication_cli_flag("talk").is_ok());
        assert!(matches!(
            parse_communication_cli_flag("bogus"),
            Err(RelayError::Usage(_))
        ));

        for sub in ["turn", "prebook-check"] {
            let mut out = Vec::new();
            let mut err = Vec::new();
            let code = execute_cli(
                ["--error-format", "json", sub, "--communication", "silent"],
                |_| None,
                &mut out,
                &mut err,
            );
            assert_eq!(code, EXIT_USER_ERROR);
            let err_str = String::from_utf8_lossy(&err);
            assert!(
                err_str.contains("SILENT_MARKET_EVAL_ONLY"),
                "expected SILENT_MARKET_EVAL_ONLY in {err_str}"
            );
        }

        let seats = match SeatFile::from_json_str(
            r#"{"seats":[{"guild":"Foundry","backend":"file","model":"ClubBot"}]}"#,
            None,
        ) {
            Ok(s) => s,
            Err(e) => panic!("parse ClubBot seat: {e}"),
        };
        let cfg = TurnConfig::new(
            PathBuf::from("nonexistent"),
            "01".to_owned(),
            Round::Act,
            seats,
            None,
            false,
            None,
        )
        .with_silent_market(true);
        let err = match run_turn(&cfg, &NoopManualTransport, None) {
            Err(e) => e,
            Ok(_) => panic!("silent_market without eval_mode must refuse"),
        };
        assert_eq!(err.code(), "SILENT_MARKET_EVAL_ONLY");
        assert_eq!(err.exit_code(), EXIT_USER_ERROR);
        assert!(err.hint().contains("league-eval"));
    }
}
