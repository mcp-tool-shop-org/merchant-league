//! Command-line parsing and execution for the `league-relay` binary.
//!
//! Kept in the library crate so every CLI validation, redaction, and exit-code
//! path is covered by both unit and integration tests.

use crate::error::RelayError;
use crate::relay::{
    MemoryMode, ResponseFormatConfig, Round, TurnConfig, TurnSummary, run_turn, validate_day,
    validate_scratchpad_setup,
};
use crate::seats::{SeatFile, validate_vm_host_allowance};
use crate::secret::{
    ApiKey, FORBIDDEN_LOCAL_KEY_ENVS, LiveGate, LocalGate, MAX_REDACTION_SCAN_BYTES,
    MAX_UNRECORDED_CHARGE_SCAN_BYTES, OPENROUTER_API_KEY_ENV, redact_secrets,
    redact_secrets_multi_with_limit, truncate_after_redaction,
};
use crate::transport::{
    LocalOpenAiTransport, MockDirTransport, RawHttpResponse, Transport, TransportError,
    UreqTransport,
};
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

/// Usage text printed for `--help`.
pub const USAGE_TEXT: &str = "\
Usage:
  league-relay turn --game-dir <dir> --day <DD> --round <talk|act> --seats <file> [options]

Options:
  --wrapper <path>           System wrapper markdown sent as role='system' (required on turn 1)
  --memory-mode <mode>       Prompt memory mode: snapshot (default), emc, or full
  --response-schema <path>   Optional JSON Schema file sent in response_format (json_schema)
  --scratchpad               Require persistent scratchpad notes (requires snapshot mode + schema)
  --no-scratchpad            Disable persistent scratchpad notes for this run
  --mixed-scratchpad-arm     Permit a deliberate arm mixing scratchpad and no-scratchpad seats
  --cap-gpu-secs <n>         Cumulative GPU-seconds ceiling for local OpenAI-compatible seats
  --allow-vm-host <host>     Additional VM-local server address permitted in local seat base_url
  --live                     Permit live OpenRouter calls (requires OPENROUTER_API_KEY and --cap-credits)
  --cap-credits <n>          Cumulative credit cap checked against relay/calls.jsonl before each call
  --mock-dir <dir>           Run against canned offline responses in <dir>/responses.json
  -h, --help                 Print this help message
";

/// Parsed command-line request.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum CliCommand {
    /// Print help text and exit `0`.
    Help,
    /// Execute one `turn`.
    Turn(TurnArgs),
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
}

/// Parse CLI arguments (excluding `argv[0]`), never echoing flag values on unknown/key flags.
pub fn parse_args<I, S>(args: I) -> Result<CliCommand, RelayError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    parse_args_with_key(args, None)
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
    if tokens.len() == 1 && matches!(tokens[0].as_str(), "-h" | "--help" | "help") {
        return Ok(CliCommand::Help);
    }
    if tokens[0] != "turn" {
        let first_flag = tokens[0].split('=').next().unwrap_or(&tokens[0]);
        if is_forbidden_key_flag(first_flag) || looks_like_cli_secret(&tokens[0]) {
            return Err(RelayError::SecretInForbiddenPlace(
                "API key must be read from OPENROUTER_API_KEY only, never passed as an argument"
                    .to_owned(),
            ));
        }
        return Err(RelayError::Usage(
            "unknown subcommand (expected 'turn' or '--help')".to_owned(),
        ));
    }

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

        match flag {
            "--live" => {
                if inline_val.is_some() {
                    return Err(RelayError::Usage("--live takes no value".to_owned()));
                }
                live = true;
                idx += 1;
            }
            "--game-dir" => {
                let (val, step) = take_flag_value(flag, inline_val, &tokens, idx)?;
                game_dir = Some(PathBuf::from(val));
                idx += step;
            }
            "--day" => {
                let (val, step) = take_flag_value(flag, inline_val, &tokens, idx)?;
                validate_day(val)?;
                day = Some(val.to_owned());
                idx += step;
            }
            "--round" => {
                let (val, step) = take_flag_value(flag, inline_val, &tokens, idx)?;
                round = Some(Round::parse(val)?);
                idx += step;
            }
            "--seats" => {
                let (val, step) = take_flag_value(flag, inline_val, &tokens, idx)?;
                seats_path = Some(PathBuf::from(val));
                idx += step;
            }
            "--wrapper" => {
                let (val, step) = take_flag_value(flag, inline_val, &tokens, idx)?;
                wrapper_path = Some(PathBuf::from(val));
                idx += step;
            }
            "--memory-mode" => {
                let (val, step) = take_flag_value(flag, inline_val, &tokens, idx)?;
                memory_mode = MemoryMode::parse(val)?;
                idx += step;
            }
            "--response-schema" => {
                let (val, step) = take_flag_value(flag, inline_val, &tokens, idx)?;
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
                let (val, step) = take_flag_value(flag, inline_val, &tokens, idx)?;
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
                let (val, step) = take_flag_value(flag, inline_val, &tokens, idx)?;
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
                let (val, step) = take_flag_value(flag, inline_val, &tokens, idx)?;
                let normalized = validate_vm_host_allowance(val)?;
                if !allowed_vm_hosts.contains(&normalized) {
                    allowed_vm_hosts.push(normalized);
                }
                idx += step;
            }
            "--mock-dir" => {
                let (val, step) = take_flag_value(flag, inline_val, &tokens, idx)?;
                mock_dir = Some(PathBuf::from(val));
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
    }))
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

/// Run CLI command with an injected environment lookup and output streams.
///
/// Returns `0` on clean completion, `2` if any seat failed to deliver a reply,
/// `3` if a sent call's post-call line could not be written to either `calls.jsonl`
/// or `calls.pending.jsonl` (its pre-call reservation stays held in `calls.reserved.jsonl`),
/// `4` if `<game-dir>/relay/turn.lock` is held by
/// another run or deleted/replaced mid-turn, or `1` on invalid CLI,
/// configuration, live-gate, or file I/O refusal.
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
    let env_key = ApiKey::from_env_reader(|k| env_reader(k));
    let mut redaction_keys: Vec<ApiKey> = Vec::new();
    if let Ok(ref primary) = env_key {
        redaction_keys.push(primary.clone());
    }
    let mut any_forbidden_local_key = env_key.is_ok();
    for env_name in FORBIDDEN_LOCAL_KEY_ENVS {
        if *env_name == OPENROUTER_API_KEY_ENV {
            continue;
        }
        if let Some(val) = env_reader(env_name)
            && !val.trim().is_empty()
        {
            any_forbidden_local_key = true;
            if let Ok(extra) = ApiKey::from_env_reader(|_| Some(val)) {
                redaction_keys.push(extra);
            }
        }
    }
    let redact_ref = redaction_keys.first();

    match run_cli_inner(args, &env_key, any_forbidden_local_key, redact_ref, stdout) {
        Ok(Some(summary)) => {
            let rendered = redact_secrets_multi_with_limit(
                &summary.format_paths_and_hashes(),
                &redaction_keys,
                MAX_UNRECORDED_CHARGE_SCAN_BYTES,
            );
            let _ = write!(stdout, "{rendered}");
            if summary.all_clean() { 0 } else { 2 }
        }
        Ok(None) => 0,
        Err(e) => {
            let (exit_code, scan_limit) = match &e {
                RelayError::UnrecordedCharge { .. } => (3, MAX_UNRECORDED_CHARGE_SCAN_BYTES),
                RelayError::TurnLocked { .. } => (4, MAX_REDACTION_SCAN_BYTES),
                _ => (1, MAX_REDACTION_SCAN_BYTES),
            };
            let _ = writeln!(
                stderr,
                "error: {}",
                redact_secrets_multi_with_limit(&e.to_string(), &redaction_keys, scan_limit)
            );
            exit_code
        }
    }
}

fn run_cli_inner<I, S>(
    args: I,
    env_key: &Result<ApiKey, RelayError>,
    any_forbidden_local_key: bool,
    redact_ref: Option<&ApiKey>,
    stdout: &mut dyn Write,
) -> Result<Option<TurnSummary>, RelayError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let turn_args = match parse_args_with_key(args, redact_ref)? {
        CliCommand::Help => {
            let _ = write!(stdout, "{USAGE_TEXT}");
            return Ok(None);
        }
        CliCommand::Turn(t) => t,
    };

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
    .with_allowed_vm_hosts(turn_args.allowed_vm_hosts.clone());

    validate_scratchpad_setup(&base_cfg)?;

    let summary = if let Some(mock_dir) = &turn_args.mock_dir {
        let mock_transport = MockDirTransport::load_with_key(mock_dir, redact_ref)?;
        let mut cfg = base_cfg;
        cfg.live = false;
        run_turn(&cfg, &mock_transport, redact_ref)?
    } else if seats.has_local_seat() {
        let local_gate = LocalGate::acquire(
            turn_args.live,
            turn_args.cap_gpu_secs,
            any_forbidden_local_key,
            &turn_args.allowed_vm_hosts,
        )?;
        let cfg = base_cfg.with_cap_gpu_secs(Some(local_gate.cap_gpu_secs()));
        let local_transport = LocalOpenAiTransport::new(local_gate);
        run_turn(&cfg, &local_transport, None)?
    } else if turn_args.live || seats.has_openrouter_seat() {
        let gate = LiveGate::acquire(turn_args.live, turn_args.cap_credits, env_key.clone())?;
        let mut cfg = base_cfg;
        cfg.live = true;
        cfg.cap_credits = Some(gate.cap_credits());
        let live_transport = UreqTransport::new(gate);
        run_turn(&cfg, &live_transport, redact_ref)?
    } else {
        run_turn(&base_cfg, &NoopManualTransport, redact_ref)?
    };
    Ok(Some(summary))
}
