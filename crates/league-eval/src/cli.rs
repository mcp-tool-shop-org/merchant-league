//! CLI argument parsing and dispatch for `league-eval` (`play`, `branch`, `select`).

use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use league_core::{HARD_TURN_CAP, StructuredError, ToStructuredError as _, scrub_secret_text};
use league_relay::{
    ApiKey, FORBIDDEN_LOCAL_KEY_ENVS, LiveGate, LocalGate, LocalOpenAiTransport, MemoryMode,
    MockDirTransport, OPENROUTER_API_KEY_ENV, SeatFile, UreqTransport, collect_env_redaction_keys,
    redact_secrets_multi_with_limit, validate_vm_host_allowance,
};

use crate::driver::{
    BranchOptions, NoopTransport, PlayOptions, run_branch_with_transport, run_play_with_transport,
    validate_eval_seats_file,
};
use crate::records::{EvalArm, summarize_branch_outcomes};
use crate::select::{SelectOptions, run_select};
use crate::{EXIT_OK, EvalError};

const HELP_TEXT: &str = "\
Merchant League Evaluation Mode harness (Card 10)

USAGE:
  league-eval [--error-format text|json] <COMMAND> [OPTIONS]
  league-eval --help | -h
  league-eval --version | -V

COMMANDS:
  play     Play up to `--turns` full turns from standard setup or turn-0 fork under `--arm <off|on>`
  branch   Replay a frozen pre-act checkpoint across `--arms <off,on>` holding peer actions fixed
  select   Deterministically select stratified early/mid/late pre-act checkpoints across games
";

/// CLI error output mode (`--error-format text|json`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ErrorFormat {
    /// Human-readable single-line stderr message.
    #[default]
    Text,
    /// Structured JSON on stderr (`league_error::StructuredError`).
    Json,
}

impl ErrorFormat {
    fn parse(raw: &str) -> Result<Self, EvalError> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            other => Err(EvalError::Usage(format!(
                "--error-format must be 'text' or 'json', got '{other}'"
            ))),
        }
    }
}

fn take_value<I>(args: &mut I, flag: &str) -> Result<String, EvalError>
where
    I: Iterator<Item = OsString>,
{
    let raw = args
        .next()
        .ok_or_else(|| EvalError::Usage(format!("{flag} requires a value")))?;
    raw.into_string()
        .map_err(|_| EvalError::Usage(format!("{flag} contains non-UTF-8 characters")))
}

fn peek_error_format(args: &[OsString]) -> ErrorFormat {
    let mut i = 0;
    while i < args.len() {
        let s = args[i].to_string_lossy();
        if s == "--error-format"
            && let Some(val) = args.get(i + 1)
            && val.to_string_lossy().eq_ignore_ascii_case("json")
        {
            return ErrorFormat::Json;
        }
        if let Some(val) = s.strip_prefix("--error-format=")
            && val.eq_ignore_ascii_case("json")
        {
            return ErrorFormat::Json;
        }
        i += 1;
    }
    ErrorFormat::Text
}

/// Execute `league-eval` from OS arguments, returning the process exit code.
pub fn main_entry() -> i32 {
    let raw_args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    execute_cli(raw_args, &mut stdout, &mut stderr)
}

/// Run the `league-eval` CLI over explicit arguments and output sinks (for deterministic tests).
pub fn execute_cli<I>(args: I, stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32
where
    I: IntoIterator<Item = OsString>,
{
    let args_vec: Vec<OsString> = args.into_iter().collect();
    let err_fmt = peek_error_format(&args_vec);
    let redaction_keys = collect_env_redaction_keys(|k| std::env::var(k).ok());
    match run_cli_args(&args_vec, &redaction_keys, stdout) {
        Ok(()) => EXIT_OK,
        Err(err) => {
            write_cli_error(&err, err_fmt, &redaction_keys, stderr);
            err.exit_code()
        }
    }
}

fn write_cli_error(
    err: &EvalError,
    fmt: ErrorFormat,
    redaction_keys: &[ApiKey],
    stderr: &mut dyn Write,
) {
    let mut structured = err.to_structured_error();
    structured.message = redact_secrets_multi_with_limit(
        &scrub_secret_text(&structured.message),
        redaction_keys,
        8192,
    );
    structured.cause = structured
        .cause
        .map(|c| redact_secrets_multi_with_limit(&scrub_secret_text(&c), redaction_keys, 8192));
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

fn run_cli_args(
    args: &[OsString],
    redaction_keys: &[ApiKey],
    stdout: &mut dyn Write,
) -> Result<(), EvalError> {
    let mut iter = args.iter().cloned().peekable();
    let mut _error_format = ErrorFormat::Text;

    while let Some(arg) = iter.peek() {
        let s = arg.to_string_lossy();
        if s == "--help" || s == "-h" {
            writeln!(stdout, "{HELP_TEXT}")
                .map_err(|e| EvalError::Io(format!("write help: {e}")))?;
            return Ok(());
        }
        if s == "--version" || s == "-V" {
            writeln!(stdout, "league-eval {}", env!("CARGO_PKG_VERSION"))
                .map_err(|e| EvalError::Io(format!("write version: {e}")))?;
            return Ok(());
        }
        if s == "--error-format" {
            let _ = iter.next();
            let val = take_value(&mut iter, "--error-format")?;
            _error_format = ErrorFormat::parse(&val)?;
            continue;
        }
        if let Some(val) = s.strip_prefix("--error-format=") {
            _error_format = ErrorFormat::parse(val)?;
            let _ = iter.next();
            continue;
        }
        break;
    }

    let Some(subcmd_os) = iter.next() else {
        return Err(EvalError::Usage(
            "missing subcommand (`play`, `branch`, or `select`); run with `--help`".to_owned(),
        ));
    };
    let subcmd = subcmd_os.to_string_lossy();
    let rest: Vec<OsString> = iter.collect();
    if rest.iter().any(|a| a == "--help" || a == "-h") {
        writeln!(stdout, "{HELP_TEXT}").map_err(|e| EvalError::Io(format!("write help: {e}")))?;
        return Ok(());
    }
    if rest.iter().any(|a| a == "--version" || a == "-V") {
        writeln!(stdout, "league-eval {}", env!("CARGO_PKG_VERSION"))
            .map_err(|e| EvalError::Io(format!("write version: {e}")))?;
        return Ok(());
    }

    match subcmd.as_ref() {
        "play" => cmd_play(&rest, redaction_keys, stdout),
        "branch" => cmd_branch(&rest, redaction_keys, stdout),
        "select" => cmd_select(&rest, stdout),
        other => Err(EvalError::Usage(format!(
            "unknown subcommand '{other}'; expected `play`, `branch`, or `select`"
        ))),
    }
}

fn cmd_play(
    args: &[OsString],
    redaction_keys: &[ApiKey],
    stdout: &mut dyn Write,
) -> Result<(), EvalError> {
    let mut game_dir: Option<PathBuf> = None;
    let mut seats_path: Option<PathBuf> = None;
    let mut wrapper_path: Option<PathBuf> = None;
    let mut wrapper_version: Option<String> = None;
    let mut arm: Option<EvalArm> = None;
    let mut table_id: u64 = 1;
    let mut seed: Option<u64> = None;
    let mut turns: u8 = HARD_TURN_CAP;
    let mut memory_mode = MemoryMode::Snapshot;
    let mut live = false;
    let mut cap_credits: Option<f64> = None;
    let mut cap_gpu_secs: Option<f64> = None;
    let mut mock_dir: Option<PathBuf> = None;
    let mut allowed_vm_hosts: Vec<String> = Vec::new();

    let mut iter = args.iter().cloned();
    while let Some(arg_os) = iter.next() {
        let arg = arg_os
            .into_string()
            .map_err(|_| EvalError::Usage("argument contains non-UTF-8 characters".to_owned()))?;
        match arg.as_str() {
            "--error-format" => {
                let val = take_value(&mut iter, "--error-format")?;
                let _ = ErrorFormat::parse(&val)?;
            }
            "--game-dir" => game_dir = Some(PathBuf::from(take_value(&mut iter, "--game-dir")?)),
            "--seats" => seats_path = Some(PathBuf::from(take_value(&mut iter, "--seats")?)),
            "--wrapper" => wrapper_path = Some(PathBuf::from(take_value(&mut iter, "--wrapper")?)),
            "--wrapper-version" => {
                wrapper_version = Some(take_value(&mut iter, "--wrapper-version")?);
            }
            "--arm" => {
                let raw = take_value(&mut iter, "--arm")?;
                arm = Some(EvalArm::parse(&raw).map_err(EvalError::Usage)?);
            }
            "--table-id" => {
                let raw = take_value(&mut iter, "--table-id")?;
                table_id = raw
                    .parse()
                    .map_err(|_| EvalError::Usage(format!("invalid --table-id '{raw}'")))?;
            }
            "--seed" => {
                let raw = take_value(&mut iter, "--seed")?;
                seed = Some(
                    raw.parse()
                        .map_err(|_| EvalError::Usage(format!("invalid --seed '{raw}'")))?,
                );
            }
            "--turns" => {
                let raw = take_value(&mut iter, "--turns")?;
                turns = raw
                    .parse()
                    .map_err(|_| EvalError::Usage(format!("invalid --turns '{raw}'")))?;
            }
            "--memory-mode" => {
                let raw = take_value(&mut iter, "--memory-mode")?;
                memory_mode = parse_memory_mode(&raw)?;
            }
            "--live" => live = true,
            "--cap-credits" => {
                let raw = take_value(&mut iter, "--cap-credits")?;
                cap_credits = Some(
                    raw.parse()
                        .map_err(|_| EvalError::Usage(format!("invalid --cap-credits '{raw}'")))?,
                );
            }
            "--cap-gpu-secs" => {
                let raw = take_value(&mut iter, "--cap-gpu-secs")?;
                cap_gpu_secs =
                    Some(raw.parse().map_err(|_| {
                        EvalError::Usage(format!("invalid --cap-gpu-secs '{raw}'"))
                    })?);
            }
            "--mock-dir" => mock_dir = Some(PathBuf::from(take_value(&mut iter, "--mock-dir")?)),
            "--allow-vm-host" => allowed_vm_hosts.push(take_value(&mut iter, "--allow-vm-host")?),
            other => {
                return Err(EvalError::Usage(format!("unknown `play` flag '{other}'")));
            }
        }
    }

    let opts = PlayOptions {
        game_dir: game_dir
            .ok_or_else(|| EvalError::Usage("`play` requires `--game-dir`".to_owned()))?,
        seats_path: seats_path
            .ok_or_else(|| EvalError::Usage("`play` requires `--seats`".to_owned()))?,
        wrapper_path: wrapper_path
            .ok_or_else(|| EvalError::Usage("`play` requires `--wrapper`".to_owned()))?,
        wrapper_version,
        arm: arm.ok_or_else(|| EvalError::Usage("`play` requires `--arm <off|on>`".to_owned()))?,
        table_id,
        seed: seed.ok_or_else(|| EvalError::Usage("`play` requires `--seed <u64>`".to_owned()))?,
        turns,
        memory_mode,
        live,
        cap_credits,
        cap_gpu_secs,
        allowed_vm_hosts: allowed_vm_hosts.clone(),
    };

    let manifest = execute_with_transport(
        &opts.seats_path,
        opts.live,
        opts.cap_credits,
        opts.cap_gpu_secs,
        mock_dir.as_deref(),
        &allowed_vm_hosts,
        redaction_keys,
        |transport, eff_live, eff_credits, eff_gpu, key| {
            let mut run_opts = opts.clone();
            run_opts.live = eff_live;
            run_opts.cap_credits = eff_credits;
            run_opts.cap_gpu_secs = eff_gpu;
            run_play_with_transport(&run_opts, transport, key)
        },
    )?;

    writeln!(
        stdout,
        "game_id={} table_id={} arm={} played_turns={} wrong_part_refusals={}/{} all_rounds={}",
        manifest.game_id,
        manifest.table_id,
        manifest.arm.as_str(),
        manifest.played_turns,
        manifest.wrong_part_refusals.total_wrong_part_refusals,
        manifest.wrong_part_refusals.delivered_model_replies,
        manifest.wrong_part_refusals.total_model_rounds
    )
    .map_err(|e| EvalError::Io(format!("write stdout: {e}")))?;
    Ok(())
}

fn cmd_branch(
    args: &[OsString],
    redaction_keys: &[ApiKey],
    stdout: &mut dyn Write,
) -> Result<(), EvalError> {
    let mut checkpoint_prefix: Option<PathBuf> = None;
    let mut list_path: Option<PathBuf> = None;
    let mut focal_seat: Option<String> = None;
    let mut seats_path: Option<PathBuf> = None;
    let mut wrapper_path: Option<PathBuf> = None;
    let mut arms: Vec<EvalArm> = Vec::new();
    let mut out_dir: Option<PathBuf> = None;
    let mut memory_mode = MemoryMode::Snapshot;
    let mut live = false;
    let mut cap_credits: Option<f64> = None;
    let mut cap_gpu_secs: Option<f64> = None;
    let mut mock_dir: Option<PathBuf> = None;
    let mut allowed_vm_hosts: Vec<String> = Vec::new();

    let mut iter = args.iter().cloned();
    while let Some(arg_os) = iter.next() {
        let arg = arg_os
            .into_string()
            .map_err(|_| EvalError::Usage("argument contains non-UTF-8 characters".to_owned()))?;
        match arg.as_str() {
            "--error-format" => {
                let val = take_value(&mut iter, "--error-format")?;
                let _ = ErrorFormat::parse(&val)?;
            }
            "--checkpoint" => {
                checkpoint_prefix = Some(PathBuf::from(take_value(&mut iter, "--checkpoint")?));
            }
            "--list" => list_path = Some(PathBuf::from(take_value(&mut iter, "--list")?)),
            "--seat" => focal_seat = Some(take_value(&mut iter, "--seat")?),
            "--seats" => seats_path = Some(PathBuf::from(take_value(&mut iter, "--seats")?)),
            "--wrapper" => wrapper_path = Some(PathBuf::from(take_value(&mut iter, "--wrapper")?)),
            "--arm" | "--arms" => {
                let raw = take_value(&mut iter, &arg)?;
                for part in raw.split(',').filter(|p| !p.trim().is_empty()) {
                    arms.push(EvalArm::parse(part).map_err(EvalError::Usage)?);
                }
            }
            "--out" => out_dir = Some(PathBuf::from(take_value(&mut iter, "--out")?)),
            "--memory-mode" => {
                let raw = take_value(&mut iter, "--memory-mode")?;
                memory_mode = parse_memory_mode(&raw)?;
            }
            "--live" => live = true,
            "--cap-credits" => {
                let raw = take_value(&mut iter, "--cap-credits")?;
                cap_credits = Some(
                    raw.parse()
                        .map_err(|_| EvalError::Usage(format!("invalid --cap-credits '{raw}'")))?,
                );
            }
            "--cap-gpu-secs" => {
                let raw = take_value(&mut iter, "--cap-gpu-secs")?;
                cap_gpu_secs =
                    Some(raw.parse().map_err(|_| {
                        EvalError::Usage(format!("invalid --cap-gpu-secs '{raw}'"))
                    })?);
            }
            "--mock-dir" => mock_dir = Some(PathBuf::from(take_value(&mut iter, "--mock-dir")?)),
            "--allow-vm-host" => allowed_vm_hosts.push(take_value(&mut iter, "--allow-vm-host")?),
            other => {
                return Err(EvalError::Usage(format!("unknown `branch` flag '{other}'")));
            }
        }
    }

    if arms.is_empty() {
        arms = vec![EvalArm::Off, EvalArm::On];
    }

    let opts = BranchOptions {
        checkpoint_prefix,
        list_path,
        focal_seat,
        seats_path: seats_path
            .ok_or_else(|| EvalError::Usage("`branch` requires `--seats`".to_owned()))?,
        wrapper_path: wrapper_path
            .ok_or_else(|| EvalError::Usage("`branch` requires `--wrapper`".to_owned()))?,
        arms,
        out_dir: out_dir.ok_or_else(|| EvalError::Usage("`branch` requires `--out`".to_owned()))?,
        memory_mode,
        live,
        cap_credits,
        cap_gpu_secs,
        allowed_vm_hosts: allowed_vm_hosts.clone(),
    };

    let outcomes = execute_with_transport(
        &opts.seats_path,
        opts.live,
        opts.cap_credits,
        opts.cap_gpu_secs,
        mock_dir.as_deref(),
        &allowed_vm_hosts,
        redaction_keys,
        |transport, eff_live, eff_credits, eff_gpu, key| {
            let mut run_opts = opts.clone();
            run_opts.live = eff_live;
            run_opts.cap_credits = eff_credits;
            run_opts.cap_gpu_secs = eff_gpu;
            run_branch_with_transport(&run_opts, transport, key)
        },
    )?;

    let summary = summarize_branch_outcomes(&outcomes);
    writeln!(
        stdout,
        "branched_outcomes={} wrong_part_refusals={}/{} all_rounds={}",
        outcomes.len(),
        summary.wrong_part_refusals.total_wrong_part_refusals,
        summary.wrong_part_refusals.delivered_model_replies,
        summary.wrong_part_refusals.total_model_rounds
    )
    .map_err(|e| EvalError::Io(format!("write stdout: {e}")))?;
    Ok(())
}

fn cmd_select(args: &[OsString], stdout: &mut dyn Write) -> Result<(), EvalError> {
    let mut game_dirs: Vec<PathBuf> = Vec::new();
    let mut per_game: usize = 6;
    let mut strata: [usize; 3] = [2, 2, 2];
    let mut selection_seed: Option<u64> = None;
    let mut out_path: Option<PathBuf> = None;

    let mut iter = args.iter().cloned();
    while let Some(arg_os) = iter.next() {
        let arg = arg_os
            .into_string()
            .map_err(|_| EvalError::Usage("argument contains non-UTF-8 characters".to_owned()))?;
        match arg.as_str() {
            "--error-format" => {
                let val = take_value(&mut iter, "--error-format")?;
                let _ = ErrorFormat::parse(&val)?;
            }
            "--games" => {
                let val = take_value(&mut iter, "--games")?;
                for part in val.split(',').filter(|p| !p.trim().is_empty()) {
                    game_dirs.push(PathBuf::from(part.trim()));
                }
            }
            "--per-game" => {
                let raw = take_value(&mut iter, "--per-game")?;
                per_game = raw
                    .parse()
                    .map_err(|_| EvalError::Usage(format!("invalid --per-game '{raw}'")))?;
            }
            "--strata" => {
                let raw = take_value(&mut iter, "--strata")?;
                let parts: Vec<&str> = raw.split(',').collect();
                if parts.len() != 3 {
                    return Err(EvalError::InvalidStrata(format!(
                        "`--strata` must have 3 comma-separated integers (`early,mid,late`), got '{raw}'"
                    )));
                }
                for (i, part) in parts.iter().enumerate() {
                    strata[i] = part.trim().parse().map_err(|_| {
                        EvalError::InvalidStrata(format!(
                            "invalid `--strata` count '{part}' in '{raw}'"
                        ))
                    })?;
                }
            }
            "--selection-seed" => {
                let raw = take_value(&mut iter, "--selection-seed")?;
                selection_seed =
                    Some(raw.parse().map_err(|_| {
                        EvalError::Usage(format!("invalid --selection-seed '{raw}'"))
                    })?);
            }
            "--out" => out_path = Some(PathBuf::from(take_value(&mut iter, "--out")?)),
            other => {
                return Err(EvalError::Usage(format!("unknown `select` flag '{other}'")));
            }
        }
    }

    let opts = SelectOptions {
        game_dirs,
        per_game,
        strata,
        selection_seed: selection_seed.ok_or_else(|| {
            EvalError::Usage("`select` requires `--selection-seed <u64>`".to_owned())
        })?,
        out_path: out_path
            .ok_or_else(|| EvalError::Usage("`select` requires `--out <list.json>`".to_owned()))?,
    };
    let list = run_select(&opts)?;
    writeln!(
        stdout,
        "selected_states={} out={}",
        list.states.len(),
        opts.out_path.display()
    )
    .map_err(|e| EvalError::Io(format!("write stdout: {e}")))?;
    Ok(())
}

fn parse_memory_mode(raw: &str) -> Result<MemoryMode, EvalError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "snapshot" => Ok(MemoryMode::Snapshot),
        "emc" => Ok(MemoryMode::Emc),
        "full" => Ok(MemoryMode::Full),
        other => Err(EvalError::Usage(format!(
            "--memory-mode must be 'snapshot', 'emc', or 'full', got '{other}'"
        ))),
    }
}

#[allow(clippy::too_many_arguments)]
fn execute_with_transport<F, T>(
    seats_path: &Path,
    live: bool,
    cap_credits: Option<f64>,
    cap_gpu_secs: Option<f64>,
    mock_dir: Option<&Path>,
    allowed_vm_hosts: &[String],
    redaction_keys: &[ApiKey],
    action: F,
) -> Result<T, EvalError>
where
    F: FnOnce(
        &dyn league_relay::Transport,
        bool,
        Option<f64>,
        Option<f64>,
        Option<&ApiKey>,
    ) -> Result<T, EvalError>,
{
    let env_key = ApiKey::from_env().ok();
    let redact_ref = env_key.as_ref().or_else(|| redaction_keys.first());
    let any_forbidden_local_key = FORBIDDEN_LOCAL_KEY_ENVS
        .iter()
        .any(|var| std::env::var_os(var).is_some());
    for host in allowed_vm_hosts {
        let _ = validate_vm_host_allowance(host)?;
    }

    let seats = SeatFile::load_with_allowed_hosts(seats_path, redact_ref, allowed_vm_hosts)?;
    validate_eval_seats_file(&seats)?;

    if let Some(dir) = mock_dir {
        let mock = MockDirTransport::load_with_key(dir, redact_ref)?;
        action(&mock, false, cap_credits, cap_gpu_secs, redact_ref)
    } else if seats.has_local_seat() {
        let local_gate = LocalGate::acquire(
            live,
            cap_gpu_secs,
            any_forbidden_local_key,
            allowed_vm_hosts,
        )?;
        let gpu_cap = Some(local_gate.cap_gpu_secs());
        let local_transport = LocalOpenAiTransport::new(local_gate);
        action(&local_transport, live, cap_credits, gpu_cap, None)
    } else if seats.has_openrouter_seat() {
        let key_res = ApiKey::from_env_reader(|k| {
            if k == OPENROUTER_API_KEY_ENV {
                std::env::var(k).ok()
            } else {
                None
            }
        });
        let gate = LiveGate::acquire(live, cap_credits, key_res)?;
        let credits = Some(gate.cap_credits());
        let ureq = UreqTransport::new(gate);
        action(&ureq, true, credits, cap_gpu_secs, redact_ref)
    } else {
        action(&NoopTransport, false, None, None, redact_ref)
    }
}
