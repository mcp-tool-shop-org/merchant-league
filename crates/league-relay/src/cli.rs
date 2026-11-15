//! Command-line parsing and execution for the `league-relay` binary.
//!
//! Kept in the library crate so every CLI validation, redaction, and exit-code
//! path is covered by both unit and integration tests.

use crate::error::RelayError;
use crate::relay::{Round, TurnConfig, TurnSummary, run_turn, validate_day};
use crate::seats::{SeatBackend, SeatFile};
use crate::secret::{ApiKey, LiveGate, redact_secrets};
use crate::transport::{MockDirTransport, Transport, UreqTransport};
use std::io::Write;
use std::path::PathBuf;

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
        let sub_name = sanitize_flag_name(&tokens[0]);
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
                let parsed: f64 = val.parse().map_err(|_| {
                    RelayError::InvalidCap(
                        "--cap-credits must be a finite non-negative number".to_owned(),
                    )
                })?;
                if !parsed.is_finite() || parsed < 0.0 {
                    return Err(RelayError::InvalidCap(
                        "--cap-credits must be a finite non-negative number".to_owned(),
                    ));
                }
                cap_credits = Some(parsed);
                idx += step;
            }
            "--mock-dir" => {
                let (val, step) = take_flag_value(flag, inline_val, &tokens, idx)?;
                mock_dir = Some(PathBuf::from(val));
                idx += step;
            }
            other => {
                let safe_flag = sanitize_flag_name(other);
                return Err(RelayError::Usage(format!("unknown option '{safe_flag}'")));
            }
        }
    }

    if live && mock_dir.is_some() {
        return Err(RelayError::Usage(
            "--live and --mock-dir cannot be used together".to_owned(),
        ));
    }

    let game_dir = game_dir
        .ok_or_else(|| RelayError::Usage("missing required --game-dir <dir>".to_owned()))?;
    let day = day.ok_or_else(|| RelayError::Usage("missing required --day <DD>".to_owned()))?;
    let round =
        round.ok_or_else(|| RelayError::Usage("missing required --round <talk|act>".to_owned()))?;
    let seats_path = seats_path
        .ok_or_else(|| RelayError::Usage("missing required --seats <file>".to_owned()))?;

    Ok(CliCommand::Turn(TurnArgs {
        game_dir,
        day,
        round,
        seats_path,
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
    if let Some(v) = inline {
        if v.is_empty() {
            return Err(RelayError::Usage(format!(
                "option '{flag}' requires a non-empty value"
            )));
        }
        return Ok((v, 1));
    }
    let Some(next) = tokens.get(idx + 1) else {
        return Err(RelayError::Usage(format!(
            "option '{flag}' requires a value"
        )));
    };
    if next.is_empty() {
        return Err(RelayError::Usage(format!(
            "option '{flag}' requires a non-empty value"
        )));
    }
    Ok((next.as_str(), 2))
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
            | "--token"
            | "--secret"
            | "--bearer"
    )
}

fn sanitize_flag_name(raw: &str) -> String {
    let head = raw.split('=').next().unwrap_or(raw);
    let scrubbed = redact_secrets(head, None);
    scrubbed.chars().take(48).collect()
}

/// Transport used when every seat in the file uses the `file` backend.
struct NoopManualTransport;

impl Transport for NoopManualTransport {
    fn post_chat_completions(
        &self,
        _url: &str,
        _body: &str,
        _timeout: std::time::Duration,
    ) -> Result<crate::transport::RawHttpResponse, crate::transport::TransportError> {
        Err(crate::transport::TransportError::Network(
            "manual-only transport cannot send network requests".to_owned(),
        ))
    }
}

/// Run CLI command with an injected environment lookup and output streams.
///
/// Returns process exit code: `0` on success, `2` if one or more seats failed
/// to deliver a reply (`NO_REPLY`, `CAP_REACHED`, `COST_UNKNOWN`, or seat error),
/// or `1` on invalid CLI/configuration/gate refusal.
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
    let env_key_opt = ApiKey::from_env_reader(env_reader);
    let redaction_ref = env_key_opt.as_ref().ok();

    let cmd = match parse_args(args) {
        Ok(c) => c,
        Err(e) => {
            let msg = redact_secrets(&e.to_string(), redaction_ref);
            let _ = writeln!(stderr, "error: {msg}");
            return 1;
        }
    };

    let turn_args = match cmd {
        CliCommand::Help => {
            let _ = write!(stdout, "{USAGE_TEXT}");
            return 0;
        }
        CliCommand::Turn(t) => t,
    };

    let seats = match SeatFile::load(&turn_args.seats_path, redaction_ref) {
        Ok(s) => s,
        Err(e) => {
            let msg = redact_secrets(&e.to_string(), redaction_ref);
            let _ = writeln!(stderr, "error: {msg}");
            return 1;
        }
    };

    let summary_res: Result<TurnSummary, RelayError> = if let Some(mock_dir) = &turn_args.mock_dir {
        match MockDirTransport::load(mock_dir) {
            Ok(mock_transport) => {
                let cfg = TurnConfig::new(
                    turn_args.game_dir,
                    turn_args.day,
                    turn_args.round,
                    seats,
                    turn_args.wrapper_path,
                    false,
                    turn_args.cap_credits,
                );
                run_turn(&cfg, &mock_transport, redaction_ref)
            }
            Err(e) => Err(e),
        }
    } else if turn_args.live || seats.has_openrouter_seat() {
        match LiveGate::acquire(turn_args.live, turn_args.cap_credits, env_key_opt.clone()) {
            Ok(gate) => {
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
                run_turn(&cfg, &live_transport, env_key_opt.as_ref().ok())
            }
            Err(e) => Err(e),
        }
    } else {
        debug_assert!(
            seats
                .seats
                .iter()
                .all(|s| matches!(s.backend, SeatBackend::File { .. }))
        );
        let cfg = TurnConfig::new(
            turn_args.game_dir,
            turn_args.day,
            turn_args.round,
            seats,
            turn_args.wrapper_path,
            false,
            turn_args.cap_credits,
        );
        run_turn(&cfg, &NoopManualTransport, redaction_ref)
    };

    match summary_res {
        Ok(summary) => {
            let _ = write!(stdout, "{}", summary.format_paths_and_hashes());
            if summary.all_clean() { 0 } else { 2 }
        }
        Err(e) => {
            let msg = redact_secrets(&e.to_string(), env_key_opt.as_ref().ok());
            let _ = writeln!(stderr, "error: {msg}");
            1
        }
    }
}
