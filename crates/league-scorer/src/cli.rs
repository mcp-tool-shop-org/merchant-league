//! CLI entrypoint and content-addressed disk I/O for `league-scorer`.

use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use league_core::{StructuredError, ToStructuredError as _};

use crate::error::{EXIT_OK, ScorerError};
use crate::rollout::execute_blind_request;
use crate::types::{BlindScoringRequest, ScorerConfig, is_valid_sha256_hex, to_sorted_pretty_json};

const HELP_TEXT: &str = "\
Merchant League blind decision scorer (S1)

USAGE:
  league-scorer [--error-format text|json] score --blind-inputs <dir> --checkpoints <dir> --config <file> --out <dir>
  league-scorer --help | -h
  league-scorer --version | -V
";

/// CLI error output mode (`--error-format text|json`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ErrorFormat {
    /// Human-readable single-line stderr message.
    #[default]
    Text,
    /// Structured JSON on stderr (`league_core::StructuredError`).
    Json,
}

impl ErrorFormat {
    fn parse(raw: &str) -> Result<Self, ScorerError> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            other => Err(ScorerError::Usage(format!(
                "--error-format must be 'text' or 'json', got '{other}'"
            ))),
        }
    }
}

/// Process entrypoint invoked by the `league-scorer` binary.
#[must_use]
pub fn main_entry() -> i32 {
    let raw_args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    run_cli(raw_args, &mut stdout, &mut stderr)
}

/// Runs the `league-scorer` CLI against explicit arguments and output streams.
pub fn run_cli<I, S>(args: I, stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let args_vec: Vec<OsString> = args.into_iter().map(Into::into).collect();
    let err_fmt = peek_error_format(&args_vec);
    match run_cli_args(&args_vec, stdout) {
        Ok(()) => EXIT_OK,
        Err(err) => {
            write_cli_error(&err, err_fmt, stderr);
            err.exit_code()
        }
    }
}

fn peek_error_format(args: &[OsString]) -> ErrorFormat {
    let mut i = 0_usize;
    while i < args.len() {
        if args[i].to_string_lossy() == "--error-format"
            && let Some(next) = args.get(i + 1)
            && let Ok(parsed) = ErrorFormat::parse(&next.to_string_lossy())
        {
            return parsed;
        }
        i += 1;
    }
    ErrorFormat::Text
}

fn write_cli_error(err: &ScorerError, fmt: ErrorFormat, stderr: &mut dyn Write) {
    let structured = err.to_structured_error();
    let _ = match fmt {
        ErrorFormat::Text => writeln!(stderr, "{}", structured.format_text()),
        ErrorFormat::Json => {
            let line = structured.to_json_string().unwrap_or_else(|_| {
                StructuredError::new("INTERNAL", "json format failed", "retry", false).format_text()
            });
            writeln!(stderr, "{line}")
        }
    };
}

fn take_path<I>(iter: &mut I, flag: &str) -> Result<PathBuf, ScorerError>
where
    I: Iterator<Item = OsString>,
{
    let raw = iter
        .next()
        .ok_or_else(|| ScorerError::Usage(format!("{flag} requires a value")))?;
    if raw.is_empty() {
        return Err(ScorerError::Usage(format!("{flag} cannot be empty")));
    }
    Ok(PathBuf::from(raw))
}

struct ScoreCliArgs {
    blind_inputs_dir: PathBuf,
    checkpoints_dir: PathBuf,
    config_path: PathBuf,
    out_dir: PathBuf,
}

fn run_cli_args(args: &[OsString], stdout: &mut dyn Write) -> Result<(), ScorerError> {
    let mut iter = args.iter().cloned().peekable();
    let mut _error_format = ErrorFormat::Text;
    let mut command: Option<String> = None;

    while let Some(arg) = iter.peek() {
        let s = arg.to_string_lossy();
        if s == "--help" || s == "-h" {
            writeln!(stdout, "{HELP_TEXT}")
                .map_err(|e| ScorerError::Io(format!("write help: {e}")))?;
            return Ok(());
        }
        if s == "--version" || s == "-V" {
            writeln!(stdout, "league-scorer {}", env!("CARGO_PKG_VERSION"))
                .map_err(|e| ScorerError::Io(format!("write version: {e}")))?;
            return Ok(());
        }
        if s == "--error-format" {
            let _ = iter.next();
            let val = iter
                .next()
                .ok_or_else(|| ScorerError::Usage("--error-format requires a value".to_owned()))?;
            _error_format = ErrorFormat::parse(&val.to_string_lossy())?;
            continue;
        }
        command = Some(s.into_owned());
        let _ = iter.next();
        break;
    }

    match command.as_deref() {
        Some("score") => {
            let parsed = parse_score_flags(iter)?;
            execute_score_command(&parsed, stdout)
        }
        Some(other) => Err(ScorerError::Usage(format!(
            "unknown subcommand `{other}`; expected `score`"
        ))),
        None => Err(ScorerError::Usage(
            "missing subcommand; expected `score` (run `--help` for usage)".to_owned(),
        )),
    }
}

fn parse_score_flags<I>(mut iter: I) -> Result<ScoreCliArgs, ScorerError>
where
    I: Iterator<Item = OsString>,
{
    let mut blind_inputs_dir: Option<PathBuf> = None;
    let mut checkpoints_dir: Option<PathBuf> = None;
    let mut config_path: Option<PathBuf> = None;
    let mut out_dir: Option<PathBuf> = None;

    while let Some(arg) = iter.next() {
        let s = arg.to_string_lossy();
        match s.as_ref() {
            "--blind-inputs" => {
                if blind_inputs_dir.is_some() {
                    return Err(ScorerError::Usage(
                        "duplicate flag --blind-inputs".to_owned(),
                    ));
                }
                blind_inputs_dir = Some(take_path(&mut iter, "--blind-inputs")?);
            }
            "--checkpoints" => {
                if checkpoints_dir.is_some() {
                    return Err(ScorerError::Usage(
                        "duplicate flag --checkpoints".to_owned(),
                    ));
                }
                checkpoints_dir = Some(take_path(&mut iter, "--checkpoints")?);
            }
            "--config" => {
                if config_path.is_some() {
                    return Err(ScorerError::Usage("duplicate flag --config".to_owned()));
                }
                config_path = Some(take_path(&mut iter, "--config")?);
            }
            "--out" => {
                if out_dir.is_some() {
                    return Err(ScorerError::Usage("duplicate flag --out".to_owned()));
                }
                out_dir = Some(take_path(&mut iter, "--out")?);
            }
            "--error-format" => {
                let val = iter.next().ok_or_else(|| {
                    ScorerError::Usage("--error-format requires a value".to_owned())
                })?;
                let _ = ErrorFormat::parse(&val.to_string_lossy())?;
            }
            other => {
                return Err(ScorerError::Usage(format!(
                    "unknown argument `{other}` for `score`"
                )));
            }
        }
    }

    Ok(ScoreCliArgs {
        blind_inputs_dir: blind_inputs_dir.ok_or_else(|| {
            ScorerError::Usage("missing required --blind-inputs <dir>".to_owned())
        })?,
        checkpoints_dir: checkpoints_dir
            .ok_or_else(|| ScorerError::Usage("missing required --checkpoints <dir>".to_owned()))?,
        config_path: config_path
            .ok_or_else(|| ScorerError::Usage("missing required --config <file>".to_owned()))?,
        out_dir: out_dir
            .ok_or_else(|| ScorerError::Usage("missing required --out <dir>".to_owned()))?,
    })
}

/// Validates that `--checkpoints` contains strictly `<64-hex>.save.json` or `<64-hex>.seal` files.
fn validate_checkpoints_layout(checkpoints_dir: &Path) -> Result<(), ScorerError> {
    let entries = fs::read_dir(checkpoints_dir).map_err(|e| {
        ScorerError::InvalidCheckpointsLayout(format!("cannot read checkpoints directory: {e}"))
    })?;
    for entry_res in entries {
        let entry = entry_res.map_err(|e| {
            ScorerError::InvalidCheckpointsLayout(format!("read checkpoint entry: {e}"))
        })?;
        let file_type = entry.file_type().map_err(|e| {
            ScorerError::InvalidCheckpointsLayout(format!("inspect checkpoint entry type: {e}"))
        })?;
        let name_os = entry.file_name();
        let name = name_os.to_string_lossy();
        if !file_type.is_file() {
            return Err(ScorerError::InvalidCheckpointsLayout(format!(
                "disallowed non-file entry `{name}` inside --checkpoints"
            )));
        }
        let stem_opt = name
            .strip_suffix(".save.json")
            .or_else(|| name.strip_suffix(".seal"));
        let Some(stem) = stem_opt else {
            return Err(ScorerError::InvalidCheckpointsLayout(format!(
                "disallowed filename `{name}` in --checkpoints (expected <sha256>.save.json or <sha256>.seal)"
            )));
        };
        if !is_valid_sha256_hex(stem) {
            return Err(ScorerError::InvalidCheckpointsLayout(format!(
                "non-hash filename `{name}` in --checkpoints (expected 64-hex stem)"
            )));
        }
    }
    Ok(())
}

fn collect_blind_request_files(blind_inputs_dir: &Path) -> Result<Vec<PathBuf>, ScorerError> {
    let entries = fs::read_dir(blind_inputs_dir).map_err(|e| {
        ScorerError::NoBlindInputs(format!("cannot read --blind-inputs directory: {e}"))
    })?;
    let mut paths = Vec::new();
    for entry_res in entries {
        let entry =
            entry_res.map_err(|e| ScorerError::Io(format!("read --blind-inputs entry: {e}")))?;
        let file_type = entry
            .file_type()
            .map_err(|e| ScorerError::Io(format!("inspect --blind-inputs entry: {e}")))?;
        if file_type.is_file() && entry.path().extension() == Some(OsStr::new("json")) {
            paths.push(entry.path());
        }
    }
    paths.sort();
    if paths.is_empty() {
        return Err(ScorerError::NoBlindInputs(
            "no *.json request files found in --blind-inputs".to_owned(),
        ));
    }
    Ok(paths)
}

fn execute_score_command(args: &ScoreCliArgs, stdout: &mut dyn Write) -> Result<(), ScorerError> {
    let cfg_bytes = fs::read(&args.config_path)
        .map_err(|e| ScorerError::InvalidScorerConfig(format!("read --config file: {e}")))?;
    let config: ScorerConfig = serde_json::from_slice(&cfg_bytes)
        .map_err(|e| ScorerError::Json(format!("parse ScorerConfig JSON: {e}")))?;
    config.validate_for_s1()?;

    validate_checkpoints_layout(&args.checkpoints_dir)?;
    let request_paths = collect_blind_request_files(&args.blind_inputs_dir)?;

    let mut requests = Vec::with_capacity(request_paths.len());
    let mut seen_state_ids = BTreeSet::new();
    for path in request_paths {
        let raw =
            fs::read(&path).map_err(|e| ScorerError::Io(format!("read blind input file: {e}")))?;
        let req: BlindScoringRequest = serde_json::from_slice(&raw)
            .map_err(|e| ScorerError::Json(format!("parse BlindScoringRequest JSON: {e}")))?;
        req.validate(&config)?;
        if !seen_state_ids.insert(req.state_id.clone()) {
            return Err(ScorerError::InvalidStateId(format!(
                "duplicate state_id `{}` across --blind-inputs files",
                req.state_id
            )));
        }
        requests.push(req);
    }

    let scores_dir = args.out_dir.join("scores");
    fs::create_dir_all(&scores_dir)
        .map_err(|e| ScorerError::Io(format!("create output scores dir: {e}")))?;

    for req in &requests {
        let save_path = args
            .checkpoints_dir
            .join(format!("{}.save.json", req.pre_act_save_sha256));
        let seal_path = args
            .checkpoints_dir
            .join(format!("{}.seal", req.pre_act_seal_sha256));
        let output = execute_blind_request(req, &config, &save_path, &seal_path)?;
        let json_text = to_sorted_pretty_json(&output)?;
        let out_path = scores_dir.join(format!("{}.json", req.state_id));
        fs::write(&out_path, json_text.as_bytes())
            .map_err(|e| ScorerError::Io(format!("write score JSON: {e}")))?;
    }

    writeln!(stdout, "scored_states={}", requests.len())
        .map_err(|e| ScorerError::Io(format!("write stdout: {e}")))?;
    Ok(())
}
