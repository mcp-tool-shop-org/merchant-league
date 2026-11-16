//! Command-line parsing and execution for the `league-relay` binary.
//!
//! Kept in the library crate so every CLI validation, redaction, and exit-code
//! path is covered by both unit and integration tests.

use crate::error::RelayError;
use crate::relay::{
    GameSetupRecord, MemoryMode, ResponseFormatConfig, Round, SeatOutcome, TurnConfig, TurnSummary,
    run_turn, validate_day, validate_scratchpad_setup,
};
use crate::seats::{SeatBackend, SeatFile, validate_vm_host_allowance};
use crate::secret::{
    ApiKey, FORBIDDEN_LOCAL_KEY_ENVS, LiveGate, LocalGate, MAX_REDACTION_SCAN_BYTES,
    MAX_UNRECORDED_CHARGE_SCAN_BYTES, OPENROUTER_API_KEY_ENV, redact_secrets,
    redact_secrets_multi_with_limit, truncate_after_redaction,
};
use crate::transport::{
    LocalOpenAiTransport, MockDirTransport, RawHttpResponse, Transport, TransportError,
    UreqTransport,
};
use crate::wording::wording_slot;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Usage text printed for `--help`.
pub const USAGE_TEXT: &str = "\
Usage:
  league-relay turn --game-dir <dir> --day <DD> --round <talk|act> --seats <file> [options]
  league-relay prebook-check --seats <file> --wrapper <path> [--response-schema <path>] [options]

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
    /// Run a self-contained offline mock `turn` in a fresh temporary game directory (`P43-B3`).
    PrebookCheck(PrebookCheckArgs),
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
    if tokens[0] == "prebook-check" {
        return parse_prebook_check_args(&tokens, redaction_key);
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
            "unknown subcommand (expected 'turn', 'prebook-check', or '--help')".to_owned(),
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
        Ok(CliRunOutcome::Turn(summary)) => {
            let rendered = redact_secrets_multi_with_limit(
                &summary.format_paths_and_hashes(),
                &redaction_keys,
                MAX_UNRECORDED_CHARGE_SCAN_BYTES,
            );
            let _ = write!(stdout, "{rendered}");
            if summary.all_clean() { 0 } else { 2 }
        }
        Ok(CliRunOutcome::PrebookOk | CliRunOutcome::Help) => 0,
        Ok(CliRunOutcome::PrebookSeatFailed) => 2,
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

enum CliRunOutcome {
    Help,
    Turn(TurnSummary),
    PrebookOk,
    PrebookSeatFailed,
}

fn parse_prebook_check_args(
    tokens: &[String],
    redaction_key: Option<&ApiKey>,
) -> Result<CliCommand, RelayError> {
    let mut seats_path: Option<PathBuf> = None;
    let mut wrapper_path: Option<PathBuf> = None;
    let mut response_schema_path: Option<PathBuf> = None;
    let mut memory_mode = MemoryMode::default();
    let mut scratchpad_requested: Option<bool> = None;
    let mut mixed_scratchpad_arm = false;
    let mut allowed_vm_hosts: Vec<String> = Vec::new();

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
    }))
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

fn run_prebook_check(
    args: PrebookCheckArgs,
    any_forbidden_local_key: bool,
    redact_ref: Option<&ApiKey>,
    stdout: &mut dyn Write,
) -> Result<CliRunOutcome, RelayError> {
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
    .with_allowed_vm_hosts(effective_vm_hosts);

    validate_scratchpad_setup(&cfg)?;

    let summary = if seats.has_local_seat() || seats.has_openrouter_seat() {
        let mock_transport = MockDirTransport::load_with_key(&mock_dir, redact_ref)?;
        run_turn(&cfg, &mock_transport, redact_ref)?
    } else {
        run_turn(&cfg, &NoopManualTransport, redact_ref)?
    };

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

    if summary.all_clean() {
        let _ = writeln!(
            stdout,
            "prebook-check: OK seats={} wording_arm={} memory_mode={} scratchpad=\"{}\" non_pinned_strings={}",
            seats.seats.len(),
            setup.wording_arm,
            setup.memory_mode,
            setup.scratchpad,
            setup.non_pinned_model_facing_strings.len()
        );
        Ok(CliRunOutcome::PrebookOk)
    } else {
        Ok(CliRunOutcome::PrebookSeatFailed)
    }
}

fn run_cli_inner<I, S>(
    args: I,
    env_key: &Result<ApiKey, RelayError>,
    any_forbidden_local_key: bool,
    redact_ref: Option<&ApiKey>,
    stdout: &mut dyn Write,
) -> Result<CliRunOutcome, RelayError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let turn_args = match parse_args_with_key(args, redact_ref)? {
        CliCommand::Help => {
            let _ = write!(stdout, "{USAGE_TEXT}");
            return Ok(CliRunOutcome::Help);
        }
        CliCommand::PrebookCheck(p) => {
            return run_prebook_check(p, any_forbidden_local_key, redact_ref, stdout);
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
    Ok(CliRunOutcome::Turn(summary))
}
