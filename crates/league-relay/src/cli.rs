//! Command-line parsing and execution for the `league-relay` binary.
//!
//! Kept in the library crate so every CLI validation, redaction, and exit-code
//! path is covered by both unit and integration tests.

use crate::error::RelayError;
use crate::relay::{Round, TurnConfig, TurnSummary, run_turn, validate_day};
use crate::seats::SeatFile;
use crate::secret::{
    ApiKey, LiveGate, MAX_REDACTION_SCAN_BYTES, MAX_UNRECORDED_CHARGE_SCAN_BYTES, redact_secrets,
    redact_secrets_with_limit, truncate_after_redaction,
};
use crate::transport::{
    MockDirTransport, RawHttpResponse, Transport, TransportError, UreqTransport,
};
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

/// Usage text printed for `--help`.
pub const USAGE_TEXT: &str = "\
Usage:
  league-relay turn --game-dir <dir> --day <DD> --round <talk|act> --seats <file> [options]

Options:
  --wrapper <path>       First-turn wrapper markdown prepended on a seat's first turn
  --live                 Permit live OpenRouter calls (requires OPENROUTER_API_KEY and --cap-credits)
  --cap-credits <n>      Cumulative credit cap checked against relay/calls.jsonl before each call
  --mock-dir <dir>       Run against canned offline responses in <dir>/responses.json
  -h, --help             Print this help message
";

/// Parsed command-line request.
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
    /// `--live`
    pub live: bool,
    /// `--cap-credits <n>`
    pub cap_credits: Option<f64>,
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
        let sub_name = sanitize_flag_name(&tokens[0], redaction_key);
        return Err(RelayError::Usage(format!(
            "unknown subcommand '{sub_name}' (expected 'turn')"
        )));
    }

    let mut game_dir: Option<PathBuf> = None;
    let mut day: Option<String> = None;
    let mut round: Option<Round> = None;
    let mut seats_path: Option<PathBuf> = None;
    let mut wrapper_path: Option<PathBuf> = None;
    let mut live = false;
    let mut cap_credits: Option<f64> = None;
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

        if is_forbidden_key_flag(flag) {
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
            "--mock-dir" => {
                let (val, step) = take_flag_value(flag, inline_val, &tokens, idx)?;
                mock_dir = Some(PathBuf::from(val));
                idx += step;
            }
            other => {
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

    Ok(CliCommand::Turn(TurnArgs {
        game_dir: game_dir
            .ok_or_else(|| RelayError::Usage("missing required --game-dir <dir>".to_owned()))?,
        day: day.ok_or_else(|| RelayError::Usage("missing required --day <DD>".to_owned()))?,
        round: round
            .ok_or_else(|| RelayError::Usage("missing required --round <talk|act>".to_owned()))?,
        seats_path: seats_path
            .ok_or_else(|| RelayError::Usage("missing required --seats <file>".to_owned()))?,
        wrapper_path,
        live,
        cap_credits,
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
    matches!(
        flag.to_ascii_lowercase().as_str(),
        "-k" | "--key"
            | "--api-key"
            | "--apikey"
            | "--openrouter-key"
            | "--openrouter-api-key"
            | "--token"
            | "--secret"
            | "--bearer"
    )
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
/// `3` if a sent call's charge could not be persisted to either `calls.jsonl`
/// or `calls.pending.jsonl`, `4` if `<game-dir>/relay/turn.lock` is held by
/// another run or deleted/replaced mid-turn, or `1` on invalid CLI,
/// configuration, live-gate, or file I/O refusal.
pub fn execute_cli<I, S, E>(
    args: I,
    env_reader: E,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
    E: FnOnce(&str) -> Option<String>,
{
    let env_key = ApiKey::from_env_reader(env_reader);
    let redact_ref = env_key.as_ref().ok();

    match run_cli_inner(args, &env_key, stdout) {
        Ok(Some(summary)) => {
            let rendered = redact_secrets_with_limit(
                &summary.format_paths_and_hashes(),
                redact_ref,
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
                redact_secrets_with_limit(&e.to_string(), redact_ref, scan_limit)
            );
            exit_code
        }
    }
}

fn run_cli_inner<I, S>(
    args: I,
    env_key: &Result<ApiKey, RelayError>,
    stdout: &mut dyn Write,
) -> Result<Option<TurnSummary>, RelayError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let redact_ref = env_key.as_ref().ok();
    let turn_args = match parse_args_with_key(args, redact_ref)? {
        CliCommand::Help => {
            let _ = write!(stdout, "{USAGE_TEXT}");
            return Ok(None);
        }
        CliCommand::Turn(t) => t,
    };

    let seats = SeatFile::load(&turn_args.seats_path, redact_ref)?;
    let summary = if let Some(mock_dir) = &turn_args.mock_dir {
        let mock_transport = MockDirTransport::load_with_key(mock_dir, redact_ref)?;
        let cfg = TurnConfig::new(
            turn_args.game_dir,
            turn_args.day,
            turn_args.round,
            seats,
            turn_args.wrapper_path,
            false,
            turn_args.cap_credits,
        );
        run_turn(&cfg, &mock_transport, redact_ref)?
    } else if turn_args.live || seats.has_openrouter_seat() {
        let gate = LiveGate::acquire(turn_args.live, turn_args.cap_credits, env_key.clone())?;
        let cfg = TurnConfig::new(
            turn_args.game_dir,
            turn_args.day,
            turn_args.round,
            seats,
            turn_args.wrapper_path,
            true,
            Some(gate.cap_credits()),
        );
        let live_transport = UreqTransport::new(gate);
        run_turn(&cfg, &live_transport, redact_ref)?
    } else {
        let cfg = TurnConfig::new(
            turn_args.game_dir,
            turn_args.day,
            turn_args.round,
            seats,
            turn_args.wrapper_path,
            false,
            turn_args.cap_credits,
        );
        run_turn(&cfg, &NoopManualTransport, redact_ref)?
    };
    Ok(Some(summary))
}
