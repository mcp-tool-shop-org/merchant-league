//! CLI entrypoint and content-addressed disk I/O for `league-scorer`.

use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use league_core::{StructuredError, ToStructuredError as _};

use crate::candidates::{build_arm_independent_request, build_arm_independent_request_verified};
use crate::error::{EXIT_OK, ScorerError, sanitize_error_text};
use crate::rollout::{
    execute_blind_request, execute_blind_request_verified,
    verify_checkpoint_pair_unchanged_after_batch,
};
use crate::types::{
    ScorerConfig, ScorerFreezeBlindView, VerifiedCheckpointsManifest,
    compute_blind_inputs_manifest_sha256, compute_scorer_tree_sha256, is_valid_sha256_hex,
    parse_blind_scoring_request_strict, parse_verified_checkpoints_manifest_strict, sha256_hex,
    to_sorted_pretty_json,
};

const HELP_TEXT: &str = "\
Merchant League blind decision scorer

USAGE:
  league-scorer [--error-format text|json] score --blind-inputs <dir> --checkpoints <dir> --config <file> --out <dir> [--freeze <file>] [--verified-checkpoints <file>] [--scorer-crate-dir <dir>]
  league-scorer [--error-format text|json] candidates --checkpoints <dir> --save-sha256 <hex> --seal-sha256 <hex> --focal-seat <n> --state-id <id> --config <file> --scorer-seed <u64> --out <dir> [--verified-checkpoints <file>]
  league-scorer --help | -h
  league-scorer --version | -V

NOTES:
  --scorer-crate-dir hashes the given crate source tree on disk, not the running binary.
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

/// Runs the `league-scorer` CLI against explicit arguments and output streams using `CRATE_GIT_SHA`.
pub fn run_cli<I, S>(args: I, stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    run_cli_with_build_id(args, crate::CRATE_GIT_SHA, stdout, stderr)
}

/// Testable entrypoint for [`run_cli`] accepting an explicit `scorer_build_id` seam.
pub fn run_cli_with_build_id<I, S>(
    args: I,
    scorer_build_id: &str,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let args_vec: Vec<OsString> = args.into_iter().map(Into::into).collect();
    let err_fmt = peek_error_format(&args_vec);
    match run_cli_args(&args_vec, scorer_build_id, stdout) {
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

fn take_string<I>(iter: &mut I, flag: &str) -> Result<String, ScorerError>
where
    I: Iterator<Item = OsString>,
{
    let raw = iter
        .next()
        .ok_or_else(|| ScorerError::Usage(format!("{flag} requires a value")))?;
    let s = raw.to_string_lossy().into_owned();
    if s.is_empty() {
        return Err(ScorerError::Usage(format!("{flag} cannot be empty")));
    }
    Ok(s)
}

struct ScoreCliArgs {
    blind_inputs_dir: PathBuf,
    checkpoints_dir: PathBuf,
    config_path: PathBuf,
    freeze_path: Option<PathBuf>,
    verified_checkpoints_path: Option<PathBuf>,
    scorer_crate_dir: Option<PathBuf>,
    out_dir: PathBuf,
}

struct CandidatesCliArgs {
    checkpoints_dir: PathBuf,
    save_sha256: String,
    seal_sha256: String,
    focal_seat: u8,
    state_id: String,
    config_path: PathBuf,
    scorer_seed: u64,
    verified_checkpoints_path: Option<PathBuf>,
    out_dir: PathBuf,
}

fn run_cli_args(
    args: &[OsString],
    scorer_build_id: &str,
    stdout: &mut dyn Write,
) -> Result<(), ScorerError> {
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
            execute_score_command(&parsed, scorer_build_id, stdout)
        }
        Some("candidates") => {
            let parsed = parse_candidates_flags(iter)?;
            execute_candidates_command(&parsed, scorer_build_id, stdout)
        }
        Some(other) => Err(ScorerError::Usage(format!(
            "unknown subcommand `{other}`; expected `score` or `candidates`"
        ))),
        None => Err(ScorerError::Usage(
            "missing subcommand; expected `score` or `candidates` (run `--help` for usage)"
                .to_owned(),
        )),
    }
}

fn set_unique<T>(slot: &mut Option<T>, val: T, flag: &str) -> Result<(), ScorerError> {
    if slot.is_some() {
        return Err(ScorerError::Usage(format!("duplicate flag {flag}")));
    }
    *slot = Some(val);
    Ok(())
}

fn parse_score_flags<I>(mut iter: I) -> Result<ScoreCliArgs, ScorerError>
where
    I: Iterator<Item = OsString>,
{
    let mut blind_inputs_dir: Option<PathBuf> = None;
    let mut checkpoints_dir: Option<PathBuf> = None;
    let mut config_path: Option<PathBuf> = None;
    let mut freeze_path: Option<PathBuf> = None;
    let mut verified_checkpoints_path: Option<PathBuf> = None;
    let mut scorer_crate_dir: Option<PathBuf> = None;
    let mut out_dir: Option<PathBuf> = None;

    while let Some(arg) = iter.next() {
        let s = arg.to_string_lossy();
        match s.as_ref() {
            "--blind-inputs" => set_unique(
                &mut blind_inputs_dir,
                take_path(&mut iter, "--blind-inputs")?,
                "--blind-inputs",
            )?,
            "--checkpoints" => set_unique(
                &mut checkpoints_dir,
                take_path(&mut iter, "--checkpoints")?,
                "--checkpoints",
            )?,
            "--config" => set_unique(
                &mut config_path,
                take_path(&mut iter, "--config")?,
                "--config",
            )?,
            "--freeze" => set_unique(
                &mut freeze_path,
                take_path(&mut iter, "--freeze")?,
                "--freeze",
            )?,
            "--verified-checkpoints" => set_unique(
                &mut verified_checkpoints_path,
                take_path(&mut iter, "--verified-checkpoints")?,
                "--verified-checkpoints",
            )?,
            "--scorer-crate-dir" => set_unique(
                &mut scorer_crate_dir,
                take_path(&mut iter, "--scorer-crate-dir")?,
                "--scorer-crate-dir",
            )?,
            "--out" => set_unique(&mut out_dir, take_path(&mut iter, "--out")?, "--out")?,
            "--error-format" => {
                let val = take_string(&mut iter, "--error-format")?;
                let _ = ErrorFormat::parse(&val)?;
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
        freeze_path,
        verified_checkpoints_path,
        scorer_crate_dir,
        out_dir: out_dir
            .ok_or_else(|| ScorerError::Usage("missing required --out <dir>".to_owned()))?,
    })
}

fn parse_candidates_flags<I>(mut iter: I) -> Result<CandidatesCliArgs, ScorerError>
where
    I: Iterator<Item = OsString>,
{
    let mut checkpoints_dir: Option<PathBuf> = None;
    let mut save_sha256: Option<String> = None;
    let mut seal_sha256: Option<String> = None;
    let mut focal_seat: Option<u8> = None;
    let mut state_id: Option<String> = None;
    let mut config_path: Option<PathBuf> = None;
    let mut scorer_seed: Option<u64> = None;
    let mut verified_checkpoints_path: Option<PathBuf> = None;
    let mut out_dir: Option<PathBuf> = None;

    while let Some(arg) = iter.next() {
        let s = arg.to_string_lossy();
        match s.as_ref() {
            "--checkpoints" => set_unique(
                &mut checkpoints_dir,
                take_path(&mut iter, "--checkpoints")?,
                "--checkpoints",
            )?,
            "--save-sha256" => set_unique(
                &mut save_sha256,
                take_string(&mut iter, "--save-sha256")?,
                "--save-sha256",
            )?,
            "--seal-sha256" => set_unique(
                &mut seal_sha256,
                take_string(&mut iter, "--seal-sha256")?,
                "--seal-sha256",
            )?,
            "--focal-seat" => {
                let raw = take_string(&mut iter, "--focal-seat")?;
                let parsed: u8 = raw.parse().map_err(|_| {
                    ScorerError::Usage(format!("--focal-seat must be an integer, got `{raw}`"))
                })?;
                set_unique(&mut focal_seat, parsed, "--focal-seat")?;
            }
            "--state-id" => set_unique(
                &mut state_id,
                take_string(&mut iter, "--state-id")?,
                "--state-id",
            )?,
            "--config" => set_unique(
                &mut config_path,
                take_path(&mut iter, "--config")?,
                "--config",
            )?,
            "--scorer-seed" => {
                let raw = take_string(&mut iter, "--scorer-seed")?;
                let parsed: u64 = raw.parse().map_err(|_| {
                    ScorerError::Usage(format!(
                        "--scorer-seed must be an unsigned u64, got `{raw}`"
                    ))
                })?;
                set_unique(&mut scorer_seed, parsed, "--scorer-seed")?;
            }
            "--verified-checkpoints" => set_unique(
                &mut verified_checkpoints_path,
                take_path(&mut iter, "--verified-checkpoints")?,
                "--verified-checkpoints",
            )?,
            "--out" => set_unique(&mut out_dir, take_path(&mut iter, "--out")?, "--out")?,
            "--error-format" => {
                let val = take_string(&mut iter, "--error-format")?;
                let _ = ErrorFormat::parse(&val)?;
            }
            other => {
                return Err(ScorerError::Usage(format!(
                    "unknown argument `{other}` for `candidates`"
                )));
            }
        }
    }

    Ok(CandidatesCliArgs {
        checkpoints_dir: checkpoints_dir
            .ok_or_else(|| ScorerError::Usage("missing required --checkpoints <dir>".to_owned()))?,
        save_sha256: save_sha256
            .ok_or_else(|| ScorerError::Usage("missing required --save-sha256 <hex>".to_owned()))?,
        seal_sha256: seal_sha256
            .ok_or_else(|| ScorerError::Usage("missing required --seal-sha256 <hex>".to_owned()))?,
        focal_seat: focal_seat
            .ok_or_else(|| ScorerError::Usage("missing required --focal-seat <n>".to_owned()))?,
        state_id: state_id
            .ok_or_else(|| ScorerError::Usage("missing required --state-id <id>".to_owned()))?,
        config_path: config_path
            .ok_or_else(|| ScorerError::Usage("missing required --config <file>".to_owned()))?,
        scorer_seed: scorer_seed
            .ok_or_else(|| ScorerError::Usage("missing required --scorer-seed <u64>".to_owned()))?,
        verified_checkpoints_path,
        out_dir: out_dir
            .ok_or_else(|| ScorerError::Usage("missing required --out <dir>".to_owned()))?,
    })
}

fn load_and_validate_config(config_path: &Path) -> Result<ScorerConfig, ScorerError> {
    let cfg_bytes = fs::read(config_path).map_err(|e| {
        ScorerError::InvalidScorerConfig(sanitize_error_text(
            &format!("read --config file: {e}"),
            &[config_path],
        ))
    })?;
    let config: ScorerConfig = serde_json::from_slice(&cfg_bytes)
        .map_err(|e| ScorerError::Json(format!("parse ScorerConfig JSON: {e}")))?;
    config.validate()?;
    Ok(config)
}

/// Validates that `--checkpoints` contains strictly `<64-hex>.save.json` or `<64-hex>.seal` files.
fn validate_checkpoints_layout(checkpoints_dir: &Path) -> Result<(), ScorerError> {
    let entries = fs::read_dir(checkpoints_dir).map_err(|e| {
        ScorerError::InvalidCheckpointsLayout(sanitize_error_text(
            &format!("cannot read checkpoints directory: {e}"),
            &[checkpoints_dir],
        ))
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
        ScorerError::NoBlindInputs(sanitize_error_text(
            &format!("cannot read --blind-inputs directory: {e}"),
            &[blind_inputs_dir],
        ))
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

fn load_verified_checkpoints(
    path: &Path,
    scorer_build_id: &str,
) -> Result<(VerifiedCheckpointsManifest, String), ScorerError> {
    let vc_bytes = fs::read(path).map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            ScorerError::MissingVerifiedCheckpoints(format!(
                "missing --verified-checkpoints file {}",
                path.file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("<file>")
            ))
        } else {
            ScorerError::Io(sanitize_error_text(
                &format!("read --verified-checkpoints file: {e}"),
                &[path],
            ))
        }
    })?;
    let vc_sha256 = sha256_hex(&vc_bytes);
    let manifest = parse_verified_checkpoints_manifest_strict(&vc_bytes)?;
    manifest.validate_for_scorer(scorer_build_id)?;
    Ok((manifest, vc_sha256))
}

fn execute_candidates_command(
    args: &CandidatesCliArgs,
    scorer_build_id: &str,
    stdout: &mut dyn Write,
) -> Result<(), ScorerError> {
    let config = load_and_validate_config(&args.config_path)?;
    validate_checkpoints_layout(&args.checkpoints_dir)?;
    if !is_valid_sha256_hex(&args.save_sha256) {
        return Err(ScorerError::InvalidHexDigest {
            field: "save_sha256",
            value: args.save_sha256.clone(),
        });
    }
    if !is_valid_sha256_hex(&args.seal_sha256) {
        return Err(ScorerError::InvalidHexDigest {
            field: "seal_sha256",
            value: args.seal_sha256.clone(),
        });
    }

    let save_path = args
        .checkpoints_dir
        .join(format!("{}.save.json", args.save_sha256));
    let seal_path = args
        .checkpoints_dir
        .join(format!("{}.seal", args.seal_sha256));
    let req = if let Some(ref vc_path) = args.verified_checkpoints_path {
        let (manifest, _) = load_verified_checkpoints(vc_path, scorer_build_id)?;
        build_arm_independent_request_verified(
            &save_path,
            &seal_path,
            &args.save_sha256,
            &args.seal_sha256,
            args.focal_seat,
            &args.state_id,
            &config,
            args.scorer_seed,
            &manifest,
            scorer_build_id,
        )?
    } else {
        build_arm_independent_request(
            &save_path,
            &seal_path,
            &args.save_sha256,
            &args.seal_sha256,
            args.focal_seat,
            &args.state_id,
            &config,
            args.scorer_seed,
        )?
    };

    fs::create_dir_all(&args.out_dir).map_err(|e| {
        ScorerError::Io(sanitize_error_text(
            &format!("create --out dir: {e}"),
            &[&args.out_dir],
        ))
    })?;
    let json_text = to_sorted_pretty_json(&req)?;
    let out_path = args.out_dir.join(format!("{}.json", req.state_id));
    fs::write(&out_path, json_text.as_bytes()).map_err(|e| {
        ScorerError::Io(sanitize_error_text(
            &format!("write blind request JSON: {e}"),
            &[&out_path],
        ))
    })?;

    writeln!(
        stdout,
        "generated_state={} candidates={}",
        req.state_id,
        req.candidates.len()
    )
    .map_err(|e| ScorerError::Io(format!("write stdout: {e}")))?;
    Ok(())
}

fn execute_score_command(
    args: &ScoreCliArgs,
    scorer_build_id: &str,
    stdout: &mut dyn Write,
) -> Result<(), ScorerError> {
    let config = load_and_validate_config(&args.config_path)?;
    validate_checkpoints_layout(&args.checkpoints_dir)?;
    let request_paths = collect_blind_request_files(&args.blind_inputs_dir)?;

    let mut verified_manifest: Option<VerifiedCheckpointsManifest> = None;
    if let Some(freeze_path) = &args.freeze_path {
        let freeze_bytes = fs::read(freeze_path).map_err(|e| {
            ScorerError::Io(sanitize_error_text(
                &format!("read --freeze file: {e}"),
                &[freeze_path],
            ))
        })?;
        let freeze: ScorerFreezeBlindView = serde_json::from_slice(&freeze_bytes)
            .map_err(|e| ScorerError::Json(format!("parse --freeze JSON: {e}")))?;
        freeze.validate()?;

        let actual_cfg_sha = config.sha256_hex()?;
        if actual_cfg_sha != freeze.scorer_config_sha256 {
            return Err(ScorerError::FreezeConfigMismatch(format!(
                "config SHA-256 `{actual_cfg_sha}` != freeze `{}`",
                freeze.scorer_config_sha256
            )));
        }

        let default_crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let crate_dir = args
            .scorer_crate_dir
            .as_deref()
            .unwrap_or(&default_crate_dir);
        let actual_tree_sha = compute_scorer_tree_sha256(crate_dir)?;
        if actual_tree_sha != freeze.scorer_tree_sha256 {
            return Err(ScorerError::FreezeTreeMismatch(format!(
                "scorer_tree_sha256 `{actual_tree_sha}` != freeze `{}`",
                freeze.scorer_tree_sha256
            )));
        }

        let actual_manifest_sha = compute_blind_inputs_manifest_sha256(&args.blind_inputs_dir)?;
        if actual_manifest_sha != freeze.blind_inputs_manifest_sha256 {
            return Err(ScorerError::FreezeManifestMismatch(format!(
                "blind_inputs_manifest_sha256 `{actual_manifest_sha}` != freeze `{}`",
                freeze.blind_inputs_manifest_sha256
            )));
        }

        let vc_path = args.verified_checkpoints_path.as_deref().ok_or_else(|| {
            ScorerError::MissingVerifiedCheckpoints(
                "--freeze requires --verified-checkpoints <file>".to_owned(),
            )
        })?;
        let (manifest, vc_sha256) = load_verified_checkpoints(vc_path, scorer_build_id)?;
        if vc_sha256 != freeze.verified_checkpoints_sha256 {
            return Err(ScorerError::FreezeVerifiedCheckpointsMismatch(format!(
                "verified_checkpoints_sha256 `{vc_sha256}` != freeze `{}`",
                freeze.verified_checkpoints_sha256
            )));
        }
        verified_manifest = Some(manifest);
    } else if let Some(ref vc_path) = args.verified_checkpoints_path {
        let (manifest, _) = load_verified_checkpoints(vc_path, scorer_build_id)?;
        verified_manifest = Some(manifest);
    }

    let mut requests = Vec::with_capacity(request_paths.len());
    let mut seen_state_ids_ci = BTreeSet::new();
    for path in request_paths {
        let raw = fs::read(&path).map_err(|e| {
            ScorerError::Io(sanitize_error_text(
                &format!("read blind input file: {e}"),
                &[&path],
            ))
        })?;
        let req_sha256 = sha256_hex(&raw);
        let req = parse_blind_scoring_request_strict(&raw)?;
        req.validate(&config)?;

        let file_stem = path.file_stem().and_then(|s| s.to_str()).ok_or_else(|| {
            ScorerError::StateIdFilenameMismatch("non-UTF8 request filename".to_owned())
        })?;
        if file_stem != req.state_id {
            return Err(ScorerError::StateIdFilenameMismatch(format!(
                "request file stem `{file_stem}` != state_id `{}`",
                req.state_id
            )));
        }
        let lower_id = req.state_id.to_ascii_lowercase();
        if !seen_state_ids_ci.insert(lower_id) {
            return Err(ScorerError::InvalidStateId(format!(
                "duplicate or case-colliding state_id `{}` across --blind-inputs files",
                req.state_id
            )));
        }
        requests.push((req, req_sha256));
    }

    let scores_dir = args.out_dir.join("scores");
    fs::create_dir_all(&scores_dir).map_err(|e| {
        ScorerError::Io(sanitize_error_text(
            &format!("create output scores dir: {e}"),
            &[&scores_dir],
        ))
    })?;

    let mut pending_outputs = Vec::with_capacity(requests.len());
    for (req, req_sha256) in &requests {
        let save_path = args
            .checkpoints_dir
            .join(format!("{}.save.json", req.pre_act_save_sha256));
        let seal_path = args
            .checkpoints_dir
            .join(format!("{}.seal", req.pre_act_seal_sha256));
        let mut output = match verified_manifest.as_ref() {
            Some(manifest) => execute_blind_request_verified(
                req,
                &config,
                &save_path,
                &seal_path,
                manifest,
                scorer_build_id,
            )?,
            None => execute_blind_request(req, &config, &save_path, &seal_path)?,
        };
        output.blind_request_sha256 = req_sha256.clone();
        let json_text = to_sorted_pretty_json(&output)?;
        let out_path = scores_dir.join(format!("{}.json", req.state_id));
        pending_outputs.push((out_path, json_text.into_bytes()));
    }

    if let Some((last_req, _)) = requests.last() {
        let mut verified_pairs = BTreeSet::new();
        verified_pairs.insert((
            last_req.pre_act_save_sha256.as_str(),
            last_req.pre_act_seal_sha256.as_str(),
            last_req.pre_act_pair_sha256.as_str(),
        ));
        for (req, _) in &requests[..requests.len() - 1] {
            let key = (
                req.pre_act_save_sha256.as_str(),
                req.pre_act_seal_sha256.as_str(),
                req.pre_act_pair_sha256.as_str(),
            );
            if verified_pairs.insert(key) {
                let save_path = args
                    .checkpoints_dir
                    .join(format!("{}.save.json", req.pre_act_save_sha256));
                let seal_path = args
                    .checkpoints_dir
                    .join(format!("{}.seal", req.pre_act_seal_sha256));
                verify_checkpoint_pair_unchanged_after_batch(
                    &save_path,
                    &seal_path,
                    &req.pre_act_save_sha256,
                    &req.pre_act_seal_sha256,
                    &req.pre_act_pair_sha256,
                )?;
            }
        }
    }

    for (out_path, bytes) in pending_outputs {
        fs::write(&out_path, &bytes).map_err(|e| {
            ScorerError::Io(sanitize_error_text(
                &format!("write score JSON: {e}"),
                &[&out_path],
            ))
        })?;
    }

    writeln!(stdout, "scored_states={}", requests.len())
        .map_err(|e| ScorerError::Io(format!("write stdout: {e}")))?;
    Ok(())
}
