//! Merchant League terminal host binary.
//!
//! Owns the Club Mode host session (`host::TuiHostSession`) and drives `league-seat-ui`
//! rendering over an interactive crossterm/ratatui terminal.

mod host;

use std::io;
use std::io::IsTerminal as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
#[cfg(test)]
use league_core::TableId;
use league_core::{ToStructuredError as _, scrub_secret_text};
#[cfg(test)]
use league_gen::{
    GeneratorConfig, RingTable, SeatCount, SeedPool, TablePurpose, TableSetupOptions,
    generate_ring_table_for_setup, is_reserved_test_seed, seed_pool,
};
use league_seat_types::{LobbyCliOverrides, ViewportSize};
use league_seat_ui::LobbyAction;
#[cfg(test)]
use league_seat_ui::{Action, action_for};
use ratatui::Terminal;
#[cfg(test)]
use ratatui::backend::TestBackend;
use ratatui::backend::{Backend, CrosstermBackend};

enum PollEvent {
    Input(Event),
    Tick,
}

trait SessionEventSource {
    fn elapsed(&self) -> Duration;
    fn poll_event(&mut self) -> io::Result<Option<PollEvent>>;
    fn drain_queued_input(&mut self) -> io::Result<()>;
}

struct LiveTerminalEvents {
    started_at: Instant,
}

impl LiveTerminalEvents {
    fn new() -> Self {
        Self {
            started_at: Instant::now(),
        }
    }
}

impl SessionEventSource for LiveTerminalEvents {
    fn elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }

    fn poll_event(&mut self) -> io::Result<Option<PollEvent>> {
        if event::poll(POLL_INTERVAL)? {
            Ok(Some(PollEvent::Input(event::read()?)))
        } else {
            Ok(Some(PollEvent::Tick))
        }
    }

    fn drain_queued_input(&mut self) -> io::Result<()> {
        while event::poll(Duration::ZERO)? {
            let _ = event::read()?;
        }
        Ok(())
    }
}

const DEFAULT_TURN_CLOCK_SECS: u16 = 210;
const MIN_TURN_CLOCK_SECS: u16 = 180;
const MAX_TURN_CLOCK_SECS: u16 = 300;
const POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct CliOptions {
    save_dir: Option<PathBuf>,
    resume_path: Option<PathBuf>,
    turn_clock_secs: Option<u16>,
    untimed: bool,
    pins_enabled: bool,
    error_format: host::ErrorFormat,
    log_level: Option<host::LogLevel>,
    log_file: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CliAction {
    Help,
    Version,
    RunClub(CliOptions),
    Error(host::TuiError),
}

fn take_flag_value<'a>(
    flag: &str,
    inline: Option<&'a str>,
    tokens: &'a [String],
    idx: usize,
) -> Result<(&'a str, usize), host::TuiError> {
    match inline {
        Some(v) if !v.is_empty() => Ok((v, 1)),
        Some(_) => Err(host::TuiError::InvalidUsage(format!(
            "{flag} requires a non-empty value"
        ))),
        None => match tokens.get(idx + 1) {
            Some(v) if !v.is_empty() => Ok((v.as_str(), 2)),
            _ => Err(host::TuiError::InvalidUsage(format!(
                "{flag} requires a value"
            ))),
        },
    }
}

fn set_log_level_checked(
    slot: &mut Option<host::LogLevel>,
    next: host::LogLevel,
) -> Result<(), host::TuiError> {
    if let Some(prev) = *slot
        && prev != next
    {
        return Err(host::TuiError::InvalidUsage(format!(
            "conflicting log-level flags ('{}' and '{}')",
            prev.as_str(),
            next.as_str()
        )));
    }
    *slot = Some(next);
    Ok(())
}

fn infer_error_format_best_effort(tokens: &[String]) -> host::ErrorFormat {
    let mut format = host::ErrorFormat::default();
    let mut idx = 1;
    while idx < tokens.len() {
        let arg = &tokens[idx];
        if let Some(val) = arg.strip_prefix("--error-format=") {
            if let Ok(parsed) = host::ErrorFormat::parse(val) {
                format = parsed;
            }
            idx += 1;
        } else if arg == "--error-format" {
            if let Some(next) = tokens.get(idx + 1)
                && let Ok(parsed) = host::ErrorFormat::parse(next)
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

fn infer_log_level_best_effort<E>(tokens: &[String], mut env_reader: E) -> host::LogLevel
where
    E: FnMut(&str) -> Option<String>,
{
    let mut cli_level = None;
    let mut idx = 1;
    while idx < tokens.len() {
        let arg = &tokens[idx];
        if let Some(val) = arg.strip_prefix("--log-level=") {
            if let Ok(parsed) = host::LogLevel::parse(val) {
                cli_level = Some(parsed);
            }
            idx += 1;
        } else if arg == "--log-level" {
            if let Some(next) = tokens.get(idx + 1)
                && let Ok(parsed) = host::LogLevel::parse(next)
            {
                cli_level = Some(parsed);
            }
            idx += 2;
        } else if matches!(arg.as_str(), "-q" | "--silent") {
            cli_level = Some(host::LogLevel::Silent);
            idx += 1;
        } else if matches!(arg.as_str(), "-v" | "--verbose") {
            cli_level = Some(host::LogLevel::Verbose);
            idx += 1;
        } else if arg == "--debug" {
            cli_level = Some(host::LogLevel::Debug);
            idx += 1;
        } else {
            idx += 1;
        }
    }
    if let Some(lvl) = cli_level {
        return lvl;
    }
    if let Some(raw_env) = env_reader(host::LEAGUE_LOG_LEVEL_ENV)
        && !raw_env.trim().is_empty()
        && let Ok(parsed) = host::LogLevel::parse(&raw_env)
    {
        return parsed;
    }
    host::LogLevel::Normal
}

fn resolve_log_level<E>(
    cli_override: Option<host::LogLevel>,
    mut env_reader: E,
) -> Result<host::LogLevel, host::TuiError>
where
    E: FnMut(&str) -> Option<String>,
{
    if let Some(lvl) = cli_override {
        return Ok(lvl);
    }
    if let Some(raw_env) = env_reader(host::LEAGUE_LOG_LEVEL_ENV)
        && !raw_env.trim().is_empty()
    {
        return host::LogLevel::parse(&raw_env);
    }
    Ok(host::LogLevel::Normal)
}

fn parse_args<I, S>(args: I) -> CliAction
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let tokens: Vec<String> = args.into_iter().map(|s| s.as_ref().to_owned()).collect();
    let mut opts = CliOptions::default();
    let mut seen_error_format: Option<host::ErrorFormat> = None;
    let mut idx = 1;
    while idx < tokens.len() {
        let raw_arg = &tokens[idx];
        let (flag, inline_val) = match raw_arg.split_once('=') {
            Some((f, v)) => (f, Some(v)),
            None => (raw_arg.as_str(), None),
        };
        match flag {
            "-h" | "--help" => return CliAction::Help,
            "-V" | "--version" => return CliAction::Version,
            "--save-dir" => match take_flag_value(flag, inline_val, &tokens, idx) {
                Ok((val, step)) => {
                    opts.save_dir = Some(PathBuf::from(val));
                    idx += step;
                }
                Err(err) => return CliAction::Error(err),
            },
            "--resume" => match take_flag_value(flag, inline_val, &tokens, idx) {
                Ok((val, step)) => {
                    opts.resume_path = Some(PathBuf::from(val));
                    idx += step;
                }
                Err(err) => return CliAction::Error(err),
            },
            "--turn-clock" => {
                if opts.untimed {
                    return CliAction::Error(host::TuiError::InvalidUsage(
                        "--turn-clock cannot be combined with --untimed".to_owned(),
                    ));
                }
                let (val, step) = match take_flag_value(flag, inline_val, &tokens, idx) {
                    Ok(pair) => pair,
                    Err(_) => {
                        return CliAction::Error(host::TuiError::InvalidUsage(format!(
                            "--turn-clock requires seconds in {MIN_TURN_CLOCK_SECS}..={MAX_TURN_CLOCK_SECS}"
                        )));
                    }
                };
                let Ok(secs) = val.parse::<u16>() else {
                    return CliAction::Error(host::TuiError::InvalidUsage(format!(
                        "invalid --turn-clock value `{val}` (expected {MIN_TURN_CLOCK_SECS}..={MAX_TURN_CLOCK_SECS})"
                    )));
                };
                if !(MIN_TURN_CLOCK_SECS..=MAX_TURN_CLOCK_SECS).contains(&secs) {
                    return CliAction::Error(host::TuiError::InvalidUsage(format!(
                        "--turn-clock {secs} out of range ({MIN_TURN_CLOCK_SECS}..={MAX_TURN_CLOCK_SECS})"
                    )));
                }
                opts.turn_clock_secs = Some(secs);
                idx += step;
            }
            "--untimed" => {
                if inline_val.is_some() {
                    return CliAction::Error(host::TuiError::InvalidUsage(
                        "--untimed takes no value".to_owned(),
                    ));
                }
                if opts.turn_clock_secs.is_some() {
                    return CliAction::Error(host::TuiError::InvalidUsage(
                        "--untimed cannot be combined with --turn-clock".to_owned(),
                    ));
                }
                opts.untimed = true;
                idx += 1;
            }
            "--pins" => {
                if inline_val.is_some() {
                    return CliAction::Error(host::TuiError::InvalidUsage(
                        "--pins takes no value".to_owned(),
                    ));
                }
                opts.pins_enabled = true;
                idx += 1;
            }
            "--error-format" => {
                let (val, step) = match take_flag_value(flag, inline_val, &tokens, idx) {
                    Ok(pair) => pair,
                    Err(err) => return CliAction::Error(err),
                };
                let parsed = match host::ErrorFormat::parse(val) {
                    Ok(fmt) => fmt,
                    Err(err) => return CliAction::Error(err),
                };
                if let Some(prev) = seen_error_format
                    && prev != parsed
                {
                    return CliAction::Error(host::TuiError::InvalidUsage(format!(
                        "conflicting --error-format values ('{}' and '{}')",
                        prev.as_str(),
                        parsed.as_str()
                    )));
                }
                seen_error_format = Some(parsed);
                opts.error_format = parsed;
                idx += step;
            }
            "--log-level" => {
                let (val, step) = match take_flag_value(flag, inline_val, &tokens, idx) {
                    Ok(pair) => pair,
                    Err(err) => return CliAction::Error(err),
                };
                let parsed = match host::LogLevel::parse(val) {
                    Ok(lvl) => lvl,
                    Err(err) => return CliAction::Error(err),
                };
                if let Err(err) = set_log_level_checked(&mut opts.log_level, parsed) {
                    return CliAction::Error(err);
                }
                idx += step;
            }
            "-q" | "--silent" => {
                if inline_val.is_some() {
                    return CliAction::Error(host::TuiError::InvalidUsage(format!(
                        "{flag} takes no value"
                    )));
                }
                if let Err(err) = set_log_level_checked(&mut opts.log_level, host::LogLevel::Silent)
                {
                    return CliAction::Error(err);
                }
                idx += 1;
            }
            "-v" | "--verbose" => {
                if inline_val.is_some() {
                    return CliAction::Error(host::TuiError::InvalidUsage(format!(
                        "{flag} takes no value"
                    )));
                }
                if let Err(err) =
                    set_log_level_checked(&mut opts.log_level, host::LogLevel::Verbose)
                {
                    return CliAction::Error(err);
                }
                idx += 1;
            }
            "--debug" => {
                if inline_val.is_some() {
                    return CliAction::Error(host::TuiError::InvalidUsage(
                        "--debug takes no value".to_owned(),
                    ));
                }
                if let Err(err) = set_log_level_checked(&mut opts.log_level, host::LogLevel::Debug)
                {
                    return CliAction::Error(err);
                }
                idx += 1;
            }
            "--log-file" => match take_flag_value(flag, inline_val, &tokens, idx) {
                Ok((val, step)) => {
                    opts.log_file = Some(PathBuf::from(val));
                    idx += step;
                }
                Err(err) => return CliAction::Error(err),
            },
            "--pin" => {
                return CliAction::Error(host::TuiError::PinCliForbidden);
            }
            _ => {
                return CliAction::Error(host::TuiError::InvalidUsage(format!(
                    "unknown flag `{}` (try --help)",
                    scrub_secret_text(raw_arg)
                )));
            }
        }
    }
    CliAction::RunClub(opts)
}

fn enter_alternate_screen<W: io::Write>(out: &mut W) -> io::Result<()> {
    crossterm::execute!(out, EnterAlternateScreen)
}

fn leave_alternate_screen<W: io::Write>(out: &mut W) -> io::Result<()> {
    crossterm::execute!(out, LeaveAlternateScreen)
}

fn finish_terminal_session<B: Backend>(terminal: &mut Terminal<B>) {
    let _ = terminal.clear();
    let _ = terminal.show_cursor();
}

fn restore_cooked_terminal() {
    let _ = disable_raw_mode();
    let mut stdout = io::stdout();
    let _ = leave_alternate_screen(&mut stdout);
}

struct TerminalCleanup;

impl Drop for TerminalCleanup {
    fn drop(&mut self) {
        restore_cooked_terminal();
    }
}

fn format_panic_output(
    payload: &str,
    location: Option<&str>,
    level: host::LogLevel,
    backtrace_text: Option<&str>,
) -> String {
    let collapsed = payload.replace(['\r', '\n'], " ");
    let scrubbed_msg = host::scrub_tui_diagnostic_text(&collapsed);
    let loc = match location {
        Some(raw) => host::scrub_tui_diagnostic_text(raw),
        None => "unknown".to_owned(),
    };
    let mut out = format!("panic at {loc}: {scrubbed_msg}\n");
    if level == host::LogLevel::Debug
        && let Some(bt) = backtrace_text
    {
        let scrubbed_bt = host::scrub_tui_diagnostic_text(bt);
        out.push_str(&format!("stack backtrace:\n{scrubbed_bt}\n"));
    }
    out
}

fn handle_panic_with_restore(
    info: &std::panic::PanicHookInfo<'_>,
    level: host::LogLevel,
    restore_terminal: &mut dyn FnMut(),
    err_out: &mut dyn io::Write,
) {
    restore_terminal();
    let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
        *s
    } else if let Some(s) = info.payload().downcast_ref::<String>() {
        s.as_str()
    } else {
        "panic"
    };
    let loc_owned = info
        .location()
        .map(|loc| format!("{}:{}:{}", loc.file(), loc.line(), loc.column()));
    let bt_owned = (level == host::LogLevel::Debug)
        .then(|| std::backtrace::Backtrace::force_capture().to_string());
    let rendered = format_panic_output(payload, loc_owned.as_deref(), level, bt_owned.as_deref());
    let _ = err_out.write_all(rendered.as_bytes());
    let _ = err_out.flush();
}

fn install_redacting_panic_hook_with_level(level: host::LogLevel) {
    host::set_panic_hook_log_level(level);
    std::panic::set_hook(Box::new(|info| {
        let level = host::panic_hook_log_level();
        let mut stderr = io::stderr();
        handle_panic_with_restore(info, level, &mut restore_cooked_terminal, &mut stderr);
    }));
}

#[cfg(test)]
fn run_title_loop<B: Backend>(
    terminal: &mut Terminal<B>,
    events: &mut dyn FnMut() -> io::Result<Option<Event>>,
) -> io::Result<()>
where
    B::Error: std::fmt::Display,
{
    loop {
        terminal
            .draw(|frame| {
                let view = league_seat_ui::title_view();
                host::draw_screen_view(frame, &view);
            })
            .map_err(|err| io::Error::other(err.to_string()))?;
        match events()? {
            None => return Ok(()),
            Some(Event::Key(key)) => {
                if let Some(mapped) = host::map_crossterm_key(key)
                    && action_for(mapped) == Action::Quit
                {
                    return Ok(());
                }
            }
            Some(_) => {}
        }
    }
}

#[cfg(test)]
fn run_session_loop<B, E>(
    terminal: &mut Terminal<B>,
    session: &mut host::TuiHostSession,
    events: &mut E,
) -> io::Result<()>
where
    B: Backend,
    B::Error: std::fmt::Display,
    E: SessionEventSource,
{
    let mut sink = host::TuiDiagnosticSink::new(host::LogLevel::Silent, None);
    sink.set_alternate_screen_active(true);
    run_session_loop_with_sink(terminal, session, events, &mut sink)
        .map_err(|err| io::Error::other(err.to_string()))
}

fn run_session_loop_with_sink<B, E>(
    terminal: &mut Terminal<B>,
    session: &mut host::TuiHostSession,
    events: &mut E,
    diag_sink: &mut host::TuiDiagnosticSink,
) -> Result<(), host::TuiError>
where
    B: Backend,
    B::Error: std::fmt::Display,
    E: SessionEventSource,
{
    diag_sink.emit(
        host::LogLevel::Normal,
        &format!("session start ({})", session.session_metadata_summary()),
    );
    let mut last_summary = String::new();
    loop {
        session.sync_clock(events.elapsed());
        let summary = session.session_metadata_summary();
        if summary != last_summary {
            diag_sink.emit(
                host::LogLevel::Verbose,
                &format!("state transition: {summary}"),
            );
            last_summary = summary;
        }
        if session.quit_requested() {
            diag_sink.emit(
                host::LogLevel::Normal,
                &format!(
                    "session exit requested ({})",
                    session.session_metadata_summary()
                ),
            );
            return Ok(());
        }
        if session.take_screen_clear() {
            terminal
                .clear()
                .map_err(|err| host::TuiError::TerminalIo(scrub_secret_text(&err.to_string())))?;
            diag_sink.emit(host::LogLevel::Debug, "alternate screen cleared (CSI 2 J)");
        }
        if session.take_input_drain() {
            events
                .drain_queued_input()
                .map_err(|err| host::TuiError::TerminalIo(scrub_secret_text(&err.to_string())))?;
            diag_sink.emit(host::LogLevel::Debug, "queued terminal input drained");
        }
        let mut render_err = None;
        terminal
            .draw(|frame| {
                if let Err(err) = session.render_current_frame(frame) {
                    render_err = Some(err);
                }
            })
            .map_err(|err| host::TuiError::TerminalIo(scrub_secret_text(&err.to_string())))?;
        if let Some(err) = render_err {
            let structured = err.to_structured_error();
            diag_sink.emit(
                host::LogLevel::Normal,
                &format!("frame render error [{}]", structured.code),
            );
            return Err(host::TuiError::SessionHalted(structured));
        }
        let area = terminal
            .size()
            .map_err(|err| host::TuiError::TerminalIo(scrub_secret_text(&err.to_string())))?;
        let vp = ViewportSize::new(area.width, area.height);
        match events
            .poll_event()
            .map_err(|err| host::TuiError::TerminalIo(scrub_secret_text(&err.to_string())))?
        {
            None => {
                diag_sink.emit(
                    host::LogLevel::Normal,
                    &format!(
                        "session stream ended ({})",
                        session.session_metadata_summary()
                    ),
                );
                return Ok(());
            }
            Some(PollEvent::Tick) => {
                session.sync_clock(events.elapsed());
            }
            Some(PollEvent::Input(Event::Key(key))) => {
                session.sync_clock(events.elapsed());
                if session.take_input_drain() {
                    events.drain_queued_input().map_err(|err| {
                        host::TuiError::TerminalIo(scrub_secret_text(&err.to_string()))
                    })?;
                    diag_sink.emit(
                        host::LogLevel::Debug,
                        "queued terminal input drained on key",
                    );
                } else if let Some(mapped) = host::map_crossterm_key(key)
                    && let Err(err) = session.handle_key_event(mapped, vp)
                {
                    diag_sink.emit(
                        host::LogLevel::Debug,
                        &format!("host refused input action [{}]", err.code),
                    );
                }
            }
            Some(PollEvent::Input(_)) => {
                session.sync_clock(events.elapsed());
            }
        }
    }
}

#[cfg(test)]
fn default_dev_ring_table() -> io::Result<RingTable> {
    let cfg = GeneratorConfig {
        seat_count: Some(SeatCount::Four),
        ..GeneratorConfig::default()
    };
    let opts = TableSetupOptions {
        purpose: TablePurpose::Dev,
        training_logged: false,
    };
    for seed in 1_u64..1_000 {
        if is_reserved_test_seed(seed) || seed_pool(seed) != SeedPool::Dev {
            continue;
        }
        if let Ok((checked, _, _)) = generate_ring_table_for_setup(seed, opts, &cfg)
            && let Some(table) = checked.into_clear_table()
        {
            return Ok(table);
        }
    }
    Err(io::Error::other("no feasible dev ring table available"))
}

#[cfg(test)]
fn default_save_paths(save_dir: &Path, table_id: TableId) -> (PathBuf, PathBuf) {
    let base = format!("club-{}", table_id.0);
    (
        save_dir.join(format!("{base}.save.json")),
        save_dir.join(format!("{base}.seal")),
    )
}

fn resolve_resume_paths_checked(target: &Path) -> Result<(PathBuf, PathBuf), host::TuiError> {
    if !target.exists() {
        return Err(host::TuiError::InvalidResumeTarget(format!(
            "resume path does not exist: {}",
            target.display()
        )));
    }
    if target.is_dir() {
        let mut matches = Vec::new();
        let entries = std::fs::read_dir(target).map_err(|err| {
            host::TuiError::InvalidResumeTarget(format!(
                "failed reading resume directory {}: {err}",
                target.display()
            ))
        })?;
        for entry in entries {
            let path = entry
                .map_err(|err| {
                    host::TuiError::InvalidResumeTarget(format!(
                        "failed iterating resume directory {}: {err}",
                        target.display()
                    ))
                })?
                .path();
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("club-") && n.ends_with(".save.json"))
            {
                matches.push(path);
            }
        }
        matches.sort_by(|a, b| {
            let name_a = a
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_owned();
            let name_b = b
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_owned();
            let key_a = (
                host::parse_club_table_id_from_filename(&name_a).unwrap_or(u64::MAX),
                name_a,
            );
            let key_b = (
                host::parse_club_table_id_from_filename(&name_b).unwrap_or(u64::MAX),
                name_b,
            );
            key_a.cmp(&key_b)
        });
        let preferred = matches
            .iter()
            .find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .and_then(|n| n.strip_suffix(".save.json"))
                    .is_some_and(|stem| p.with_file_name(format!("{stem}.seal")).exists())
            })
            .cloned()
            .or_else(|| matches.into_iter().next());
        let save = preferred.ok_or_else(|| {
            host::TuiError::InvalidResumeTarget(format!(
                "no club-*.save.json found in {}",
                target.display()
            ))
        })?;
        return resolve_resume_paths_checked(&save);
    }
    let name = target.file_name().and_then(|n| n.to_str()).ok_or_else(|| {
        host::TuiError::InvalidResumeTarget("resume path has no file name".to_owned())
    })?;
    let stem = name
        .strip_suffix(".save.json")
        .or_else(|| name.strip_suffix(".json"))
        .unwrap_or(name);
    let seal = target.with_file_name(format!("{stem}.seal"));
    if !seal.exists() {
        return Err(host::TuiError::InvalidResumeTarget(format!(
            "missing companion seal file {}",
            seal.display()
        )));
    }
    Ok((target.to_path_buf(), seal))
}

fn apply_cli_session_options(
    session: &mut host::TuiHostSession,
    opts: &CliOptions,
    is_resume: bool,
) {
    if opts.untimed {
        session.configure_turn_clock(None);
    } else if let Some(secs) = opts.turn_clock_secs {
        session.configure_turn_clock(Some(secs));
    } else if !is_resume || !session.has_saved_clock_setting() {
        session.configure_turn_clock(Some(DEFAULT_TURN_CLOCK_SECS));
    }
    if opts.pins_enabled {
        session.enable_seat_pins();
    }
}

fn build_session_for_cli(opts: &CliOptions) -> Result<host::TuiHostSession, host::TuiError> {
    let Some(resume_target) = &opts.resume_path else {
        return Err(host::TuiError::InvalidUsage(
            "--resume <PATH> is required to load a saved session".to_owned(),
        ));
    };
    let (save_path, seal_path) = resolve_resume_paths_checked(resume_target)?;
    let mut session = host::TuiHostSession::read_save_and_seal(&save_path, &seal_path, None)
        .map_err(host::TuiError::SaveLoadFailed)?;
    apply_cli_session_options(&mut session, opts, true);
    if let Some(save_dir) = &opts.save_dir {
        let (s_path, k_path) = host::validate_resume_autosave_target(
            &save_path,
            &seal_path,
            save_dir,
            session.table_id(),
        )
        .map_err(host::TuiError::SaveDirConflict)?;
        session.enable_autosave(s_path, k_path);
    } else {
        session.enable_autosave(save_path, seal_path);
    }
    Ok(session)
}

fn lobby_overrides_from_cli(opts: &CliOptions) -> LobbyCliOverrides {
    LobbyCliOverrides {
        turn_clock_secs: opts.turn_clock_secs,
        untimed: opts.untimed,
        pins_enabled: opts.pins_enabled,
    }
}

#[cfg(test)]
fn run_lobby_loop<B, E>(
    terminal: &mut Terminal<B>,
    opts: &CliOptions,
    events: &mut E,
    save_dir: Option<&Path>,
) -> io::Result<Option<host::TuiHostSession>>
where
    B: Backend,
    B::Error: std::fmt::Display,
    E: SessionEventSource,
{
    let mut sink = host::TuiDiagnosticSink::new(host::LogLevel::Silent, None);
    sink.set_alternate_screen_active(true);
    run_lobby_loop_with_sink(terminal, opts, events, save_dir, &mut sink)
        .map_err(|err| io::Error::other(err.to_string()))
}

fn run_lobby_loop_with_sink<B, E>(
    terminal: &mut Terminal<B>,
    opts: &CliOptions,
    events: &mut E,
    save_dir: Option<&Path>,
    diag_sink: &mut host::TuiDiagnosticSink,
) -> Result<Option<host::TuiHostSession>, host::TuiError>
where
    B: Backend,
    B::Error: std::fmt::Display,
    E: SessionEventSource,
{
    let mut lobby = host::LobbyHostSession::new(lobby_overrides_from_cli(opts));
    diag_sink.emit(host::LogLevel::Normal, "lobby opened");
    loop {
        terminal
            .draw(|frame| lobby.render_frame(frame))
            .map_err(|err| host::TuiError::TerminalIo(scrub_secret_text(&err.to_string())))?;
        if lobby.quit_requested() {
            diag_sink.emit(host::LogLevel::Normal, "lobby quit requested");
            return Ok(None);
        }
        let area = terminal
            .size()
            .map_err(|err| host::TuiError::TerminalIo(scrub_secret_text(&err.to_string())))?;
        let vp = ViewportSize::new(area.width, area.height);
        match events
            .poll_event()
            .map_err(|err| host::TuiError::TerminalIo(scrub_secret_text(&err.to_string())))?
        {
            None => return Ok(None),
            Some(PollEvent::Tick) => {}
            Some(PollEvent::Input(Event::Key(key))) => {
                if let Some(mapped) = host::map_crossterm_key(key) {
                    match lobby.handle_key_event(mapped, vp) {
                        LobbyAction::None => {}
                        LobbyAction::Quit => {
                            diag_sink.emit(host::LogLevel::Normal, "lobby closed cleanly");
                            return Ok(None);
                        }
                        LobbyAction::StartSeason => {
                            match lobby.validate_and_start_in_dir(1, save_dir) {
                                Ok(session) => {
                                    diag_sink.emit(
                                        host::LogLevel::Verbose,
                                        &format!(
                                            "lobby built table ({})",
                                            session.session_metadata_summary()
                                        ),
                                    );
                                    return Ok(Some(session));
                                }
                                Err(err) => {
                                    diag_sink.emit(
                                        host::LogLevel::Debug,
                                        &format!("lobby validation refused [{}]", err.code),
                                    );
                                }
                            }
                        }
                    }
                }
            }
            Some(PollEvent::Input(_)) => {}
        }
    }
}

fn run_interactive(
    opts: &CliOptions,
    is_tty: bool,
    diag_sink: &mut host::TuiDiagnosticSink,
) -> Result<(), host::TuiError> {
    if !is_tty {
        return Err(host::TuiError::TtyRequired);
    }
    let mut preloaded_session = if opts.resume_path.is_some() {
        let session = build_session_for_cli(opts)?;
        diag_sink.emit(
            host::LogLevel::Verbose,
            &format!("resumed session ({})", session.session_metadata_summary()),
        );
        Some(session)
    } else {
        None
    };

    enable_raw_mode()
        .map_err(|err| host::TuiError::TerminalIo(scrub_secret_text(&err.to_string())))?;
    let cleanup = TerminalCleanup;
    let mut stdout = io::stdout();
    enter_alternate_screen(&mut stdout)
        .map_err(|err| host::TuiError::TerminalIo(scrub_secret_text(&err.to_string())))?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)
        .map_err(|err| host::TuiError::TerminalIo(scrub_secret_text(&err.to_string())))?;
    diag_sink.set_alternate_screen_active(true);

    let mut live_events = LiveTerminalEvents::new();
    let loop_res = if let Some(mut session) = preloaded_session.take() {
        let r =
            run_session_loop_with_sink(&mut terminal, &mut session, &mut live_events, diag_sink);
        if r.is_ok()
            && let Some(tui_err) = session.terminal_tui_error()
        {
            Err(tui_err)
        } else {
            r
        }
    } else {
        let save_dir = opts.save_dir.clone().unwrap_or_else(|| PathBuf::from("."));
        match run_lobby_loop_with_sink(
            &mut terminal,
            opts,
            &mut live_events,
            Some(&save_dir),
            diag_sink,
        )? {
            Some(mut session) => {
                let r = run_session_loop_with_sink(
                    &mut terminal,
                    &mut session,
                    &mut live_events,
                    diag_sink,
                );
                if r.is_ok()
                    && let Some(tui_err) = session.terminal_tui_error()
                {
                    Err(tui_err)
                } else {
                    r
                }
            }
            None => Ok(()),
        }
    };

    finish_terminal_session(&mut terminal);
    drop(cleanup);
    diag_sink.set_alternate_screen_active(false);
    loop_res
}

fn dispatch_cli_with_env<E, F>(
    args: impl IntoIterator<Item = String>,
    mut env_reader: E,
    is_tty: bool,
    out: &mut dyn io::Write,
    err_out: &mut dyn io::Write,
    run_tui: F,
) -> ExitCode
where
    E: FnMut(&str) -> Option<String>,
    F: FnOnce(&CliOptions, bool, &mut host::TuiDiagnosticSink) -> Result<(), host::TuiError>,
{
    let tokens: Vec<String> = args.into_iter().collect();
    let best_err_fmt = infer_error_format_best_effort(&tokens);
    let best_log_lvl = infer_log_level_best_effort(&tokens, |k| env_reader(k));
    host::set_panic_hook_log_level(best_log_lvl);

    match parse_args(tokens) {
        CliAction::Help => {
            let _ = writeln!(
                out,
                "league-tui {} - Merchant League offline Club Mode host (prototype, not released)",
                env!("CARGO_PKG_VERSION")
            );
            let _ = writeln!(
                out,
                "Usage: league-tui [-h|--help] [-V|--version] [--save-dir <DIR>] [--resume <PATH>] [--turn-clock <180..=300> | --untimed] [--pins] [--error-format <text|json>] [--log-level <silent|normal|verbose|debug> | -q|--silent | -v|--verbose | --debug] [--log-file <PATH>]"
            );
            let _ = writeln!(
                out,
                "Precedence: CLI flags (--turn-clock, --untimed, --pins, --log-level/-q/-v/--debug) override Lobby choices, saved .seal settings, and env vars ({}); `--pin` values are forbidden on CLI.",
                host::LEAGUE_LOG_LEVEL_ENV
            );
            let _ = writeln!(
                out,
                "Exit codes: 0 ok | 1 user/config error (incl. TTY_REQUIRED) | 2 runtime error (corrupt save/seal, autosave/terminal error) | 3 partial success (season finalized, replay export/.seal wipe failed)"
            );
            ExitCode::from(host::EXIT_OK)
        }
        CliAction::Version => {
            let _ = writeln!(out, "league-tui {}", env!("CARGO_PKG_VERSION"));
            ExitCode::from(host::EXIT_OK)
        }
        CliAction::RunClub(opts) => {
            let level = match resolve_log_level(opts.log_level, env_reader) {
                Ok(lvl) => lvl,
                Err(err) => {
                    let line = opts.error_format.format_line(&err.to_structured_error());
                    let _ = writeln!(err_out, "{line}");
                    return ExitCode::from(err.exit_code());
                }
            };
            host::set_panic_hook_log_level(level);
            let mut diag_sink = host::TuiDiagnosticSink::new(level, opts.log_file.clone());
            let run_res = run_tui(&opts, is_tty, &mut diag_sink);
            diag_sink.set_alternate_screen_active(false);
            diag_sink.flush_after_restore(err_out);
            match run_res {
                Ok(()) => ExitCode::from(host::EXIT_OK),
                Err(err) => {
                    let line = opts.error_format.format_line(&err.to_structured_error());
                    let _ = writeln!(err_out, "{line}");
                    ExitCode::from(err.exit_code())
                }
            }
        }
        CliAction::Error(err) => {
            let line = best_err_fmt.format_line(&err.to_structured_error());
            let _ = writeln!(err_out, "{line}");
            ExitCode::from(err.exit_code())
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let initial_log_level = infer_log_level_best_effort(&args, |name| std::env::var(name).ok());
    install_redacting_panic_hook_with_level(initial_log_level);
    let mut out = io::stdout();
    let mut err_out = io::stderr();
    let is_tty = io::stdin().is_terminal() && io::stdout().is_terminal();
    dispatch_cli_with_env(
        args,
        |name| std::env::var(name).ok(),
        is_tty,
        &mut out,
        &mut err_out,
        run_interactive,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::collections::VecDeque;

    struct ScriptedSessionEvents {
        clock: Duration,
        steps: VecDeque<(Duration, PollEvent, Vec<Event>)>,
        buffered_input: VecDeque<Event>,
        drain_calls: usize,
        on_draw: Option<Box<dyn FnMut()>>,
    }

    impl ScriptedSessionEvents {
        fn from_events(events: impl IntoIterator<Item = Event>) -> Self {
            Self {
                clock: Duration::ZERO,
                steps: events
                    .into_iter()
                    .map(|e| (Duration::ZERO, PollEvent::Input(e), Vec::new()))
                    .collect(),
                buffered_input: VecDeque::new(),
                drain_calls: 0,
                on_draw: None,
            }
        }

        fn from_timed_with_queued(
            steps: impl IntoIterator<Item = (Duration, Option<Event>, Vec<Event>)>,
        ) -> Self {
            Self {
                clock: Duration::ZERO,
                steps: steps
                    .into_iter()
                    .map(|(d, ev_opt, q)| {
                        let ev = match ev_opt {
                            Some(ev) => PollEvent::Input(ev),
                            None => PollEvent::Tick,
                        };
                        (d, ev, q)
                    })
                    .collect(),
                buffered_input: VecDeque::new(),
                drain_calls: 0,
                on_draw: None,
            }
        }
    }

    impl SessionEventSource for ScriptedSessionEvents {
        fn elapsed(&self) -> Duration {
            self.clock
        }

        fn poll_event(&mut self) -> io::Result<Option<PollEvent>> {
            if let Some(cb) = &mut self.on_draw {
                cb();
            }
            if let Some(ev) = self.buffered_input.pop_front() {
                return Ok(Some(PollEvent::Input(ev)));
            }
            let Some((delta, ev, follow_on_queued)) = self.steps.pop_front() else {
                return Ok(None);
            };
            self.clock = self.clock.saturating_add(delta);
            self.buffered_input.extend(follow_on_queued);
            Ok(Some(ev))
        }

        fn drain_queued_input(&mut self) -> io::Result<()> {
            self.drain_calls += 1;
            self.buffered_input.clear();
            Ok(())
        }
    }

    #[inline(never)]
    fn trigger_test_panic_line(msg: String) {
        panic!("{msg}");
    }

    #[derive(Clone)]
    struct SharedVecWriter(std::rc::Rc<std::cell::RefCell<Vec<u8>>>);

    impl io::Write for SharedVecWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct FixedSizeCrosstermBackend<W: io::Write> {
        inner: CrosstermBackend<W>,
        width: u16,
        height: u16,
    }

    impl<W: io::Write> FixedSizeCrosstermBackend<W> {
        fn new(writer: W, width: u16, height: u16) -> Self {
            Self {
                inner: CrosstermBackend::new(writer),
                width,
                height,
            }
        }
    }

    impl<W: io::Write> Backend for FixedSizeCrosstermBackend<W> {
        type Error = io::Error;

        fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
        where
            I: Iterator<Item = (u16, u16, &'a ratatui::buffer::Cell)>,
        {
            self.inner.draw(content)
        }

        fn hide_cursor(&mut self) -> io::Result<()> {
            self.inner.hide_cursor()
        }

        fn show_cursor(&mut self) -> io::Result<()> {
            self.inner.show_cursor()
        }

        fn get_cursor_position(&mut self) -> io::Result<ratatui::layout::Position> {
            Ok(ratatui::layout::Position::new(0, 0))
        }

        fn set_cursor_position<P: Into<ratatui::layout::Position>>(
            &mut self,
            position: P,
        ) -> io::Result<()> {
            self.inner.set_cursor_position(position)
        }

        fn clear(&mut self) -> io::Result<()> {
            self.inner.clear()
        }

        fn clear_region(&mut self, clear_type: ratatui::backend::ClearType) -> io::Result<()> {
            self.inner.clear_region(clear_type)
        }

        fn size(&self) -> io::Result<ratatui::layout::Size> {
            Ok(ratatui::layout::Size::new(self.width, self.height))
        }

        fn window_size(&mut self) -> io::Result<ratatui::backend::WindowSize> {
            Ok(ratatui::backend::WindowSize {
                columns_rows: ratatui::layout::Size::new(self.width, self.height),
                pixels: ratatui::layout::Size::new(0, 0),
            })
        }

        fn flush(&mut self) -> io::Result<()> {
            self.inner.flush()
        }
    }

    #[test]
    fn cli_args_parse_and_dispatch_all_paths() {
        assert_eq!(parse_args(["league-tui", "--help"]), CliAction::Help);
        assert_eq!(parse_args(["league-tui", "-h"]), CliAction::Help);
        assert_eq!(parse_args(["league-tui", "--version"]), CliAction::Version);
        assert_eq!(parse_args(["league-tui", "-V"]), CliAction::Version);
        assert_eq!(
            parse_args(["league-tui"]),
            CliAction::RunClub(CliOptions::default())
        );
        assert_eq!(
            parse_args([
                "league-tui",
                "--save-dir",
                "saves",
                "--resume",
                "saves/club-1.save.json",
                "--turn-clock",
                "180",
                "--pins",
                "--error-format",
                "json",
                "--log-level",
                "verbose",
            ]),
            CliAction::RunClub(CliOptions {
                save_dir: Some(PathBuf::from("saves")),
                resume_path: Some(PathBuf::from("saves/club-1.save.json")),
                turn_clock_secs: Some(180),
                untimed: false,
                pins_enabled: true,
                error_format: host::ErrorFormat::Json,
                log_level: Some(host::LogLevel::Verbose),
                log_file: None,
            })
        );
        assert_eq!(
            parse_args(["league-tui", "--untimed", "--pins", "-q"]),
            CliAction::RunClub(CliOptions {
                save_dir: None,
                resume_path: None,
                turn_clock_secs: None,
                untimed: true,
                pins_enabled: true,
                error_format: host::ErrorFormat::Text,
                log_level: Some(host::LogLevel::Silent),
                log_file: None,
            })
        );
        assert!(matches!(
            parse_args(["league-tui", "--turn-clock", "120"]),
            CliAction::Error(host::TuiError::InvalidUsage(_))
        ));
        assert!(matches!(
            parse_args(["league-tui", "--turn-clock", "301"]),
            CliAction::Error(host::TuiError::InvalidUsage(_))
        ));
        assert!(matches!(
            parse_args(["league-tui", "--turn-clock", "210", "--untimed"]),
            CliAction::Error(host::TuiError::InvalidUsage(_))
        ));
        assert_eq!(
            parse_args(["league-tui", "--pin=1234"]),
            CliAction::Error(host::TuiError::PinCliForbidden)
        );
        assert!(matches!(
            parse_args(["league-tui", "-q", "--debug"]),
            CliAction::Error(host::TuiError::InvalidUsage(_))
        ));
        assert_eq!(
            parse_args(["league-tui", "--bogus"]),
            CliAction::Error(host::TuiError::InvalidUsage(
                "unknown flag `--bogus` (try --help)".to_owned()
            ))
        );

        let mut out = Vec::new();
        let mut err = Vec::new();
        let code_help = dispatch_cli_with_env(
            ["league-tui".to_owned(), "--help".to_owned()],
            |_| None,
            false,
            &mut out,
            &mut err,
            |_, _, _| Ok(()),
        );
        assert_eq!(code_help, ExitCode::SUCCESS);
        let help_txt = String::from_utf8_lossy(&out);
        assert!(help_txt.contains("Usage: league-tui"));
        assert!(help_txt.contains("Precedence:"));
        assert!(help_txt.contains("Exit codes:"));

        out.clear();
        let code_ver = dispatch_cli_with_env(
            ["league-tui".to_owned(), "--version".to_owned()],
            |_| None,
            false,
            &mut out,
            &mut err,
            |_, _, _| Ok(()),
        );
        assert_eq!(code_ver, ExitCode::SUCCESS);
        assert!(String::from_utf8_lossy(&out).contains("league-tui"));

        let code_ok = dispatch_cli_with_env(
            ["league-tui".to_owned()],
            |_| None,
            true,
            &mut out,
            &mut err,
            |_, _, _| Ok(()),
        );
        assert_eq!(code_ok, ExitCode::SUCCESS);

        err.clear();
        let code_tui_err = dispatch_cli_with_env(
            ["league-tui".to_owned()],
            |_| None,
            false,
            &mut out,
            &mut err,
            |_, is_tty, _| {
                if !is_tty {
                    return Err(host::TuiError::TtyRequired);
                }
                Ok(())
            },
        );
        assert_eq!(code_tui_err, ExitCode::from(1));
        assert!(String::from_utf8_lossy(&err).contains("[TTY_REQUIRED]"));

        err.clear();
        let code_bad = dispatch_cli_with_env(
            ["league-tui".to_owned(), "--bogus".to_owned()],
            |_| None,
            true,
            &mut out,
            &mut err,
            |_, _, _| Ok(()),
        );
        assert_eq!(code_bad, ExitCode::from(1));
        assert!(String::from_utf8_lossy(&err).contains("unknown flag"));
    }

    #[test]
    fn title_loop_draws_title_and_quits_on_q() -> Result<(), Box<dyn std::error::Error>> {
        let mut terminal = Terminal::new(TestBackend::new(60, 7))?;
        let mut seq = vec![
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
        ]
        .into_iter();
        run_title_loop(&mut terminal, &mut || Ok(seq.next()))?;
        let buffer = terminal.backend().buffer();
        let screen: String = (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(screen.contains(league_seat_ui::TITLE_LINES[0]));
        assert!(screen.contains(league_seat_ui::TITLE_LINES[1]));
        Ok(())
    }

    #[test]
    fn session_loop_draws_turn_and_quits_on_q() -> Result<(), Box<dyn std::error::Error>> {
        let ring = default_dev_ring_table()?;
        let mut session = host::TuiHostSession::from_ring_table(&ring, TableId(7), &[0], 1)?;
        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
        let mut events = ScriptedSessionEvents::from_events([Event::Key(KeyEvent::new(
            KeyCode::Char('q'),
            KeyModifiers::NONE,
        ))]);
        run_session_loop(&mut terminal, &mut session, &mut events)?;
        assert!(session.quit_requested());
        Ok(())
    }

    #[test]
    fn key_releases_and_action_repeats_are_ignored_on_notes_and_curtain()
    -> Result<(), Box<dyn std::error::Error>> {
        use crossterm::event::KeyEventKind;
        let ring = default_dev_ring_table()?;
        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;

        let key_with = |code: KeyCode, modifiers: KeyModifiers, kind: KeyEventKind| {
            let mut ev = KeyEvent::new(code, modifiers);
            ev.kind = kind;
            Event::Key(ev)
        };

        // 1. Press + Release stream (`n`, `n`, `z`, `Esc`) + Repeat on text/nav vs ignored Repeat on Esc/q/Enter/Ctrl+L.
        let mut session_notes = host::TuiHostSession::from_ring_table(&ring, TableId(78), &[0], 1)?;
        let notes_events = vec![
            key_with(KeyCode::Char('n'), KeyModifiers::NONE, KeyEventKind::Press),
            key_with(
                KeyCode::Char('n'),
                KeyModifiers::NONE,
                KeyEventKind::Release,
            ),
            key_with(KeyCode::Char('n'), KeyModifiers::NONE, KeyEventKind::Press),
            key_with(
                KeyCode::Char('n'),
                KeyModifiers::NONE,
                KeyEventKind::Release,
            ),
            key_with(KeyCode::Char('z'), KeyModifiers::NONE, KeyEventKind::Press),
            key_with(
                KeyCode::Char('z'),
                KeyModifiers::NONE,
                KeyEventKind::Release,
            ),
            key_with(KeyCode::Char('!'), KeyModifiers::NONE, KeyEventKind::Repeat),
            key_with(KeyCode::Backspace, KeyModifiers::NONE, KeyEventKind::Repeat),
            key_with(KeyCode::Esc, KeyModifiers::NONE, KeyEventKind::Repeat),
            key_with(KeyCode::Esc, KeyModifiers::NONE, KeyEventKind::Press),
            key_with(KeyCode::Esc, KeyModifiers::NONE, KeyEventKind::Release),
            key_with(KeyCode::Char('q'), KeyModifiers::NONE, KeyEventKind::Repeat),
            key_with(
                KeyCode::Char('l'),
                KeyModifiers::CONTROL,
                KeyEventKind::Repeat,
            ),
        ];
        let mut events_notes = ScriptedSessionEvents::from_events(notes_events);
        run_session_loop(&mut terminal, &mut session_notes, &mut events_notes)?;
        assert!(
            !session_notes.quit_requested(),
            "Esc/q repeat/release must not quit"
        );
        assert!(
            !session_notes.is_curtain_open(),
            "Ctrl+L repeat must not lock"
        );
        assert_eq!(
            session_notes
                .seat_ui_state(league_core::Seat(0))
                .ok_or("missing s0")?
                .notes_draft(),
            "nz"
        );

        // 2. Curtain handoff (`Ctrl+L`, `Space`, `Enter`) with Press + Release must post zero offers.
        let mut session_curtain =
            host::TuiHostSession::from_ring_table(&ring, TableId(79), &[0], 1)?;
        let mut events_curtain = ScriptedSessionEvents::from_timed_with_queued([
            (
                Duration::ZERO,
                Some(key_with(
                    KeyCode::Char('l'),
                    KeyModifiers::CONTROL,
                    KeyEventKind::Press,
                )),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_with(
                    KeyCode::Char('l'),
                    KeyModifiers::CONTROL,
                    KeyEventKind::Release,
                )),
                Vec::new(),
            ),
            (
                host::CURTAIN_LIFT_DEAD_TIME,
                Some(key_with(
                    KeyCode::Char(' '),
                    KeyModifiers::NONE,
                    KeyEventKind::Press,
                )),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_with(
                    KeyCode::Char(' '),
                    KeyModifiers::NONE,
                    KeyEventKind::Release,
                )),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_with(
                    KeyCode::Enter,
                    KeyModifiers::NONE,
                    KeyEventKind::Press,
                )),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_with(
                    KeyCode::Enter,
                    KeyModifiers::NONE,
                    KeyEventKind::Release,
                )),
                Vec::new(),
            ),
            (
                host::CURTAIN_LIFT_DEAD_TIME,
                Some(key_with(
                    KeyCode::Enter,
                    KeyModifiers::NONE,
                    KeyEventKind::Repeat,
                )),
                Vec::new(),
            ),
        ]);
        run_session_loop(&mut terminal, &mut session_curtain, &mut events_curtain)?;
        assert!(!session_curtain.is_curtain_open());
        let obs_curtain = session_curtain.build_seat_observation(league_core::Seat(0))?;
        assert!(
            !obs_curtain.offers.values().any(|o| o.maker == 0),
            "Enter release/repeat after Curtain must not post an offer for seat 0"
        );

        // 3. Pressing `q` inside `run_session_loop` exits before drawing any post-quit standby frame.
        let mut session_quit = host::TuiHostSession::from_ring_table(&ring, TableId(80), &[0], 1)?;
        let q_iter = vec![
            key_with(KeyCode::Char('q'), KeyModifiers::NONE, KeyEventKind::Press),
            key_with(
                KeyCode::Char('q'),
                KeyModifiers::NONE,
                KeyEventKind::Release,
            ),
        ];
        let draw_count = std::rc::Rc::new(std::cell::Cell::new(0_usize));
        let draw_count_cb = std::rc::Rc::clone(&draw_count);
        let mut q_events = ScriptedSessionEvents::from_events(q_iter);
        q_events.on_draw = Some(Box::new(move || {
            draw_count_cb.set(draw_count_cb.get() + 1);
        }));
        let mut term_counted = Terminal::new(TestBackend::new(100, 30))?;
        run_session_loop(&mut term_counted, &mut session_quit, &mut q_events)?;
        assert!(session_quit.quit_requested());
        assert_eq!(
            draw_count.get(),
            1,
            "run_session_loop must check quit_requested before drawing a second frame"
        );
        Ok(())
    }

    #[test]
    fn curtain_handoff_discards_queued_input_enforces_dead_time_and_clears_alternate_screen()
    -> Result<(), Box<dyn std::error::Error>> {
        use crossterm::event::KeyEventKind;
        let ring = default_dev_ring_table()?;
        let mut session =
            host::TuiHostSession::from_ring_table(&ring, TableId(81_301), &[0, 1], 5)?;
        let written = std::rc::Rc::new(std::cell::RefCell::new(Vec::<u8>::new()));
        let mut terminal = Terminal::new(FixedSizeCrosstermBackend::new(
            SharedVecWriter(written.clone()),
            100,
            30,
        ))?;
        let key_ev = |code: KeyCode, mods: KeyModifiers| {
            let mut k = KeyEvent::new(code, mods);
            k.kind = KeyEventKind::Press;
            Event::Key(k)
        };

        // Trigger Curtain open (`Ctrl+L`), arm (`Space`), lift (`Enter`), then check:
        // - Queued offer-post key (`Enter`) arriving before drain must NOT be read by Seat 1.
        // - Keys inside `CURTAIN_LIFT_DEAD_TIME` (`200 ms < 400 ms`) must be dropped.
        // - Keys at `>= 400 ms` (`n`, `k`, `Enter`) work for Seat 1's personal notes.
        let mut events = ScriptedSessionEvents::from_timed_with_queued([
            (
                Duration::from_millis(10),
                Some(key_ev(KeyCode::Char('l'), KeyModifiers::CONTROL)),
                Vec::new(),
            ),
            (
                host::CURTAIN_LIFT_DEAD_TIME,
                Some(key_ev(KeyCode::Char(' '), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::from_millis(10),
                Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                vec![key_ev(KeyCode::Enter, KeyModifiers::NONE)],
            ),
            (
                Duration::from_millis(200),
                Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Char('n'), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                host::CURTAIN_LIFT_DEAD_TIME,
                Some(key_ev(KeyCode::Char('n'), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::from_millis(10),
                Some(key_ev(KeyCode::Char('k'), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::from_millis(10),
                Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                Vec::new(),
            ),
        ]);

        run_session_loop(&mut terminal, &mut session, &mut events)?;
        assert_eq!(
            events.drain_calls, 2,
            "run_session_loop must drain queued input on Curtain open and Curtain lift"
        );
        let obs1 = session.build_seat_observation(league_core::Seat(1))?;
        assert!(
            !obs1.offers.values().any(|o| o.maker == 1),
            "queued input and dead-time Enter must not post an offer for Seat 1"
        );
        assert_eq!(
            session
                .seat_ui_state(league_core::Seat(1))
                .ok_or("missing s1")?
                .notes_draft(),
            "k",
            "key inside dead time must be ignored, while key after 400 ms dead time must apply"
        );

        let bytes = written.borrow();
        let raw_ansi = String::from_utf8_lossy(&bytes);
        assert!(
            raw_ansi.contains("\x1b[2J"),
            "Curtain hand-off must clear alternate screen cells in place with CSI 2 J"
        );
        assert!(
            !bytes.windows(4).any(|w| w == b"\x1b[3J") && !raw_ansi.contains("\x1b[3J"),
            "must never emit CSI 3 J scrollback purge"
        );
        assert!(
            !bytes.contains(&0x07),
            "must never emit terminal bell (BEL)"
        );
        assert!(!raw_ansi.contains("\x1b]0;") && !raw_ansi.contains("\x1b]2;"));
        assert!(!raw_ansi.contains("\x1b]52;"));
        Ok(())
    }

    #[test]
    fn handoff_and_exit_bytes_never_contain_csi_3j_scrollback_purge()
    -> Result<(), Box<dyn std::error::Error>> {
        use crossterm::event::KeyEventKind;
        let ring = default_dev_ring_table()?;
        let mut session =
            host::TuiHostSession::from_ring_table(&ring, TableId(81_302), &[0, 1], 7)?;
        let written = std::rc::Rc::new(std::cell::RefCell::new(Vec::<u8>::new()));
        let mut screen_writer = SharedVecWriter(written.clone());
        enter_alternate_screen(&mut screen_writer)?;
        let mut terminal = Terminal::new(FixedSizeCrosstermBackend::new(
            SharedVecWriter(written.clone()),
            100,
            30,
        ))?;
        let key_ev = |code: KeyCode, mods: KeyModifiers| {
            let mut k = KeyEvent::new(code, mods);
            k.kind = KeyEventKind::Press;
            Event::Key(k)
        };

        let mut events = ScriptedSessionEvents::from_timed_with_queued([
            (
                Duration::from_millis(10),
                Some(key_ev(KeyCode::Char('l'), KeyModifiers::CONTROL)),
                Vec::new(),
            ),
            (
                host::CURTAIN_LIFT_DEAD_TIME,
                Some(key_ev(KeyCode::Char(' '), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::from_millis(10),
                Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                host::CURTAIN_LIFT_DEAD_TIME,
                Some(key_ev(KeyCode::Char('q'), KeyModifiers::NONE)),
                Vec::new(),
            ),
        ]);

        run_session_loop(&mut terminal, &mut session, &mut events)?;
        let handoff_len = written.borrow().len();
        {
            let bytes = written.borrow();
            let handoff_ansi = String::from_utf8_lossy(&bytes);
            assert!(
                handoff_ansi.contains("\x1b[?1049h"),
                "session startup must enter the alternate screen"
            );
            assert!(
                handoff_ansi.contains("\x1b[2J"),
                "Curtain hand-off must clear only the alternate screen (CSI 2 J)"
            );
            assert!(
                !bytes.windows(4).any(|w| w == b"\x1b[3J") && !handoff_ansi.contains("\x1b[3J"),
                "bytes written at Curtain hand-off must contain no CSI 3 J sequence"
            );
        }

        finish_terminal_session(&mut terminal);
        leave_alternate_screen(&mut screen_writer)?;
        let all_bytes = written.borrow();
        let exit_ansi = String::from_utf8_lossy(&all_bytes[handoff_len..]);
        let full_ansi = String::from_utf8_lossy(&all_bytes);
        assert!(
            exit_ansi.contains("\x1b[?1049l"),
            "session exit must leave the alternate screen"
        );
        assert!(
            !all_bytes.windows(4).any(|w| w == b"\x1b[3J") && !full_ansi.contains("\x1b[3J"),
            "bytes written at hand-off and at exit must never contain CSI 3 J"
        );
        assert!(
            !all_bytes.contains(&0x07),
            "must never emit terminal bell (BEL)"
        );
        assert!(!full_ansi.contains("\x1b]0;") && !full_ansi.contains("\x1b]2;"));
        assert!(!full_ansi.contains("\x1b]52;"));
        Ok(())
    }

    #[test]
    fn fake_clock_poll_loop_updates_countdown_expires_pass_to_clean_curtain_regent_acts_and_untimed_never_expires()
    -> Result<(), Box<dyn std::error::Error>> {
        use crossterm::event::KeyEventKind;
        let ring = default_dev_ring_table()?;
        let mut session =
            host::TuiHostSession::from_ring_table(&ring, TableId(81_502), &[0, 1], 13)?;
        session.configure_turn_clock(Some(180)); // H=2 => Pass A = 67s, Pass B = 22s

        let key_ev = |code: KeyCode, mods: KeyModifiers| {
            let mut k = KeyEvent::new(code, mods);
            k.kind = KeyEventKind::Press;
            Event::Key(k)
        };
        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;

        // Drive Seat 0 note `"PRIVATE_BEFORE_TIMEOUT_919"`, What-If preview (`p`), then let
        // non-blocking `Tick` events advance past `67s` WITHOUT pressing Ctrl+L.
        let mut steps: Vec<(Duration, Option<Event>, Vec<Event>)> = vec![(
            Duration::ZERO,
            Some(key_ev(KeyCode::Char('n'), KeyModifiers::NONE)),
            Vec::new(),
        )];
        for ch in "PRIVATE_BEFORE_TIMEOUT_919".chars() {
            steps.push((
                Duration::ZERO,
                Some(key_ev(KeyCode::Char(ch), KeyModifiers::NONE)),
                Vec::new(),
            ));
        }
        steps.push((
            Duration::ZERO,
            Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
            Vec::new(),
        ));
        steps.push((
            Duration::ZERO,
            Some(key_ev(KeyCode::Char('p'), KeyModifiers::NONE)),
            Vec::new(),
        ));
        // Seven 10s poll Ticks (`70s >= 67s`) expire Pass A automatically into Seat 1's Curtain.
        for _ in 0..8 {
            steps.push((Duration::from_secs(10), None, Vec::new()));
        }

        let mut captured_screens: Vec<String> = Vec::new();
        let term_capture = |term: &Terminal<TestBackend>| {
            let buf = term.backend().buffer();
            (0..buf.area.height)
                .map(|y| {
                    (0..buf.area.width)
                        .map(|x| buf[(x, y)].symbol().to_string())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let mut events = ScriptedSessionEvents::from_timed_with_queued(steps);
        // Capture initial + every tick screen via a thin custom loop driving `run_session_loop` one
        // tick at a time isn't needed: `on_draw` can read terminal? Instead drive first batch via
        // `run_session_loop`, then unlock Curtain and inspect Seat 1!
        run_session_loop(&mut terminal, &mut session, &mut events)?;
        let curtain_buf = term_capture(&terminal);
        captured_screens.push(curtain_buf.clone());
        assert!(
            session.is_curtain_open(),
            "Pass A must lock automatically into Curtain at >= 67s:\n{curtain_buf}"
        );
        assert!(curtain_buf.contains("Hand keyboard to Seat 1"));
        assert!(!curtain_buf.contains("PRIVATE_BEFORE_TIMEOUT_919"));
        assert!(!curtain_buf.contains("Preview T1 S0:"));
        let obs0 = session.build_seat_observation(league_core::Seat(0))?;
        let m0 = obs0.own_mandate().ok_or("s0 mandate")?;
        let m0_dbg = format!("{:?}", m0.kind);
        assert!(!curtain_buf.contains(&m0_dbg));

        // Phase 1: Unlock Seat 1 after the 400 ms Curtain-open dead time and verify the first
        // unlocked frame displays the exact initial `pass_budget_secs` (`67s` => `01:07`) and zero
        // Seat 0 private markers.
        let mut events_s1 = ScriptedSessionEvents::from_timed_with_queued([
            (
                host::CURTAIN_LIFT_DEAD_TIME,
                Some(key_ev(KeyCode::Char(' '), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::from_millis(10),
                Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                Vec::new(),
            ),
        ]);
        events_s1.clock = events.elapsed();
        run_session_loop(&mut terminal, &mut session, &mut events_s1)?;
        let s1_buf_t0 = term_capture(&terminal);
        assert!(
            !session.is_curtain_open(),
            "Seat 1 must have unlocked from Curtain"
        );
        assert!(!s1_buf_t0.contains("PRIVATE_BEFORE_TIMEOUT_919"));
        assert!(!s1_buf_t0.contains("Preview T1 S0:"));
        let budget = session
            .pass_budget_secs()
            .ok_or("missing Seat 1 Pass A budget")?;
        assert_eq!(budget, 67);
        let expected_t0 = format!("Clock: {:02}:{:02}", budget / 60, budget % 60);
        assert!(
            s1_buf_t0.contains(&expected_t0),
            "first unlocked header must show `{expected_t0}` matching `pass_budget_secs == {budget}`:\n{s1_buf_t0}"
        );

        // Phase 2: Advance fake clock past lift dead time + 10s on pure `Tick`s and assert the
        // displayed countdown decrements accurately (`67s` -> `57s` / `00:57`) with no `Clock:`
        // fallback escape.
        let mut events_s1_tick = ScriptedSessionEvents::from_timed_with_queued([
            (
                host::CURTAIN_LIFT_DEAD_TIME + Duration::from_secs(10),
                None,
                Vec::new(),
            ),
            (Duration::ZERO, None, Vec::new()),
        ]);
        events_s1_tick.clock = events_s1.elapsed();
        run_session_loop(&mut terminal, &mut session, &mut events_s1_tick)?;
        let s1_buf_t1 = term_capture(&terminal);
        let remaining = budget.saturating_sub(10);
        assert_eq!(remaining, 57);
        let expected_t1 = format!("Clock: {:02}:{:02}", remaining / 60, remaining % 60);
        assert!(
            s1_buf_t1.contains(&expected_t1) && !s1_buf_t1.contains(&expected_t0),
            "header must display ticking Pass A countdown `{expected_t1}` (`pass_budget_secs - 10s == {remaining}`) without key presses:\n{s1_buf_t1}"
        );

        // Regent doctrine triggers on idle `SubPassB` timeout (`H=1`, 180s => 135s A / 45s B).
        let mut regent_session =
            host::TuiHostSession::from_ring_table(&ring, TableId(81_503), &[0], 17)?;
        regent_session.configure_turn_clock(Some(180));
        let mut regent_steps: Vec<(Duration, Option<Event>, Vec<Event>)> = Vec::new();
        for _ in 0..14 {
            regent_steps.push((Duration::from_secs(10), None, Vec::new()));
        }
        regent_steps.push((
            host::CURTAIN_LIFT_DEAD_TIME + Duration::from_millis(10),
            Some(key_ev(KeyCode::Char(' '), KeyModifiers::NONE)),
            Vec::new(),
        ));
        regent_steps.push((
            Duration::from_millis(10),
            Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
            Vec::new(),
        ));
        for _ in 0..6 {
            regent_steps.push((Duration::from_secs(10), None, Vec::new()));
        }
        let mut regent_events = ScriptedSessionEvents::from_timed_with_queued(regent_steps);
        run_session_loop(&mut terminal, &mut regent_session, &mut regent_events)?;
        assert_eq!(
            regent_session.turn(),
            2,
            "Turn 1 must sweep after Pass B timeout"
        );
        let r1 = &regent_session.public_receipts()[0];
        assert!(
            r1.regent_acted_seats.contains(&0),
            "idle SubPassB human timeout must invoke Standing Regent doctrine: {:?}",
            r1.regent_acted_seats
        );

        // Untimed option (`--untimed`) never expires a pass even after 10_000s of Ticks.
        let mut untimed_session =
            host::TuiHostSession::from_ring_table(&ring, TableId(81_504), &[0], 19)?;
        untimed_session.configure_turn_clock(None);
        let mut untimed_events = ScriptedSessionEvents::from_timed_with_queued([
            (Duration::from_secs(5_000), None, Vec::new()),
            (Duration::from_secs(5_000), None, Vec::new()),
        ]);
        run_session_loop(&mut terminal, &mut untimed_session, &mut untimed_events)?;
        assert_eq!(untimed_session.turn(), 1);
        assert_eq!(untimed_session.pass(), league_seat_ui::PassKind::SubPassA);
        assert!(!untimed_session.is_curtain_open());
        assert!(term_capture(&terminal).contains("Clock: Untimed"));

        // C2: `.seal` checkpoint preserves timed (`240s`) and `--untimed` clock settings across
        // `--resume` when CLI flags are omitted, while explicit CLI flags override `.seal`.
        let ckpt_dir = std::env::temp_dir().join(format!("ml_clock_ckpt_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ckpt_dir);
        std::fs::create_dir_all(&ckpt_dir)?;
        let save_timed = ckpt_dir.join("club-81505.save.json");
        let seal_timed = ckpt_dir.join("club-81505.seal");
        let mut timed_240 =
            host::TuiHostSession::from_ring_table(&ring, TableId(81_505), &[0], 21)?;
        timed_240.configure_turn_clock(Some(240));
        timed_240
            .write_save_and_seal(&save_timed, &seal_timed)
            .map_err(|e| e.to_string())?;
        let resumed_240 = build_session_for_cli(&CliOptions {
            resume_path: Some(save_timed.clone()),
            ..CliOptions::default()
        })?;
        assert_eq!(resumed_240.pass_budget_secs(), Some(180));
        let resumed_300_override = build_session_for_cli(&CliOptions {
            resume_path: Some(save_timed.clone()),
            turn_clock_secs: Some(300),
            ..CliOptions::default()
        })?;
        assert_eq!(resumed_300_override.pass_budget_secs(), Some(225));
        let resumed_untimed_override = build_session_for_cli(&CliOptions {
            resume_path: Some(save_timed),
            untimed: true,
            ..CliOptions::default()
        })?;
        assert_eq!(resumed_untimed_override.pass_budget_secs(), None);

        let save_untimed = ckpt_dir.join("club-81506.save.json");
        let seal_untimed = ckpt_dir.join("club-81506.seal");
        untimed_session
            .write_save_and_seal(&save_untimed, &seal_untimed)
            .map_err(|e| e.to_string())?;
        let resumed_untimed = build_session_for_cli(&CliOptions {
            resume_path: Some(save_untimed.clone()),
            ..CliOptions::default()
        })?;
        assert_eq!(resumed_untimed.pass_budget_secs(), None);
        let resumed_retimed = build_session_for_cli(&CliOptions {
            resume_path: Some(save_untimed),
            turn_clock_secs: Some(180),
            ..CliOptions::default()
        })?;
        assert_eq!(resumed_retimed.pass_budget_secs(), Some(135));
        let _ = std::fs::remove_dir_all(&ckpt_dir);
        Ok(())
    }

    #[test]
    fn resume_with_pins_starts_behind_curtain_hides_private_markers_and_requires_pin()
    -> Result<(), Box<dyn std::error::Error>> {
        use crossterm::event::KeyEventKind;
        use league_core::Seat;
        use league_seat_types::{BundleDraftAction, SeatIntent};

        let key_ev = |code: KeyCode, mods: KeyModifiers| {
            let mut k = KeyEvent::new(code, mods);
            k.kind = KeyEventKind::Press;
            Event::Key(k)
        };
        let term_capture = |term: &Terminal<TestBackend>| {
            let buf = term.backend().buffer();
            (0..buf.area.height)
                .map(|y| {
                    (0..buf.area.width)
                        .map(|x| buf[(x, y)].symbol().to_string())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let ring = default_dev_ring_table()?;
        let base_dir = std::env::temp_dir().join(format!("ml_resume_pins_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base_dir);
        let dir_a = base_dir.join("case_a");
        std::fs::create_dir_all(&dir_a)?;
        let (save_a, seal_a) = default_save_paths(&dir_a, TableId(81_511));
        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;

        // Sub-case A: `--pins` enabled from Turn 1 Curtain. Seat 0 sets PIN `"4829"` -> post-PIN
        // autosave triggers while `curtain == None` -> drop immediately -> `--resume` must reopen
        // behind Seat 0's Curtain and require `"4829"`.
        let mut sess_a =
            host::TuiHostSession::from_ring_table(&ring, TableId(81_511), &[0, 1], 31)?;
        sess_a.enable_autosave(save_a.clone(), seal_a.clone());
        sess_a.enable_seat_pins();
        let s0a_mandate = host::mandate_short_marker(
            sess_a
                .build_seat_observation(Seat(0))?
                .own_mandate()
                .ok_or("s0a mandate")?
                .kind,
        );
        let s0a_mandate_clip: String = s0a_mandate.chars().take(18).collect();
        let mut set_pin_events = ScriptedSessionEvents::from_timed_with_queued([
            (
                host::CURTAIN_LIFT_DEAD_TIME,
                Some(key_ev(KeyCode::Char(' '), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Char('4'), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Char('8'), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Char('2'), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Char('9'), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                Vec::new(),
            ),
        ]);
        run_session_loop(&mut terminal, &mut sess_a, &mut set_pin_events)?;
        assert!(!sess_a.is_curtain_open());
        assert!(save_a.exists() && seal_a.exists());
        drop(sess_a);

        let mut resumed_a = build_session_for_cli(&CliOptions {
            save_dir: Some(dir_a),
            resume_path: Some(save_a),
            turn_clock_secs: None,
            untimed: false,
            pins_enabled: true,
            ..CliOptions::default()
        })?;
        assert!(resumed_a.is_curtain_open());
        terminal.draw(|f| {
            let _ = resumed_a.render_current_frame(f);
        })?;
        assert_eq!(resumed_a.observe_seat_call_count(), 0);
        let first_a = term_capture(&terminal);
        assert!(first_a.contains("Hand keyboard to Seat 0"));
        assert!(!first_a.contains(&s0a_mandate_clip));
        assert!(!first_a.contains("Private Mandate"));
        assert!(!first_a.contains("Seat 0 Private Vault"));

        let mut wrong_pin_events = ScriptedSessionEvents::from_timed_with_queued([
            (
                host::CURTAIN_LIFT_DEAD_TIME,
                Some(key_ev(KeyCode::Char(' '), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Char('0'), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Char('0'), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Char('0'), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Char('0'), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                Vec::new(),
            ),
        ]);
        run_session_loop(&mut terminal, &mut resumed_a, &mut wrong_pin_events)?;
        assert!(resumed_a.is_curtain_open());
        assert_eq!(resumed_a.observe_seat_call_count(), 0);

        let mut right_pin_events = ScriptedSessionEvents::from_timed_with_queued([
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Char('4'), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Char('8'), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Char('2'), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Char('9'), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                Vec::new(),
            ),
        ]);
        right_pin_events.clock = wrong_pin_events.elapsed();
        run_session_loop(&mut terminal, &mut resumed_a, &mut right_pin_events)?;
        assert!(!resumed_a.is_curtain_open());
        let unlocked_a = term_capture(&terminal);
        assert!(
            unlocked_a.contains("Seat 0 Private Vault") && unlocked_a.contains(&s0a_mandate_clip),
            "right PIN must restore Seat 0's view:\n{unlocked_a}"
        );

        // Sub-case B: post-PIN autosave while Seat 0 holds a note + staged order + mandate, then
        // drop immediately -> `--resume` Curtain hides all three markers until right PIN `"5193"`.
        let dir_b = base_dir.join("case_b");
        std::fs::create_dir_all(&dir_b)?;
        let (save_b, seal_b) = default_save_paths(&dir_b, TableId(81_512));
        let mut sess_b =
            host::TuiHostSession::from_ring_table(&ring, TableId(81_512), &[0, 1], 37)?;
        sess_b.enable_autosave(save_b.clone(), seal_b.clone());
        let s0b_mandate = host::mandate_short_marker(
            sess_b
                .build_seat_observation(Seat(0))?
                .own_mandate()
                .ok_or("s0b mandate")?
                .kind,
        );
        let s0b_mandate_clip: String = s0b_mandate.chars().take(18).collect();
        let mut note_steps: Vec<(Duration, Option<Event>, Vec<Event>)> = vec![(
            Duration::ZERO,
            Some(key_ev(KeyCode::Char('n'), KeyModifiers::NONE)),
            Vec::new(),
        )];
        for ch in "PIN_NOTE_731".chars() {
            note_steps.push((
                Duration::ZERO,
                Some(key_ev(KeyCode::Char(ch), KeyModifiers::NONE)),
                Vec::new(),
            ));
        }
        note_steps.push((
            Duration::ZERO,
            Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
            Vec::new(),
        ));
        let mut note_events = ScriptedSessionEvents::from_timed_with_queued(note_steps);
        run_session_loop(&mut terminal, &mut sess_b, &mut note_events)?;
        sess_b
            .apply_seat_intent(
                Seat(0),
                SeatIntent::update_bundle(BundleDraftAction::SetDebtRepaymentCred(731)),
            )
            .map_err(|e| e.to_string())?;
        sess_b.enable_seat_pins();
        let mut pin_b_events = ScriptedSessionEvents::from_timed_with_queued([
            (
                host::CURTAIN_LIFT_DEAD_TIME,
                Some(key_ev(KeyCode::Char(' '), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Char('5'), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Char('1'), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Char('9'), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Char('3'), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                Vec::new(),
            ),
        ]);
        pin_b_events.clock = note_events.elapsed();
        run_session_loop(&mut terminal, &mut sess_b, &mut pin_b_events)?;
        assert!(!sess_b.is_curtain_open());
        drop(sess_b);

        let mut resumed_b = build_session_for_cli(&CliOptions {
            save_dir: Some(dir_b),
            resume_path: Some(save_b),
            turn_clock_secs: None,
            untimed: false,
            pins_enabled: true,
            ..CliOptions::default()
        })?;
        assert!(resumed_b.is_curtain_open());
        terminal.draw(|f| {
            let _ = resumed_b.render_current_frame(f);
        })?;
        assert_eq!(resumed_b.observe_seat_call_count(), 0);
        let first_b = term_capture(&terminal);
        assert!(first_b.contains("Hand keyboard to Seat 0"));
        assert!(!first_b.contains(&s0b_mandate_clip));
        assert!(!first_b.contains("731"));
        assert!(!first_b.contains("PIN_NOTE_731"));
        assert!(!first_b.contains("Private Mandate"));

        let mut unlock_b_events = ScriptedSessionEvents::from_timed_with_queued([
            (
                host::CURTAIN_LIFT_DEAD_TIME,
                Some(key_ev(KeyCode::Char(' '), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Char('5'), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Char('1'), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Char('9'), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Char('3'), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                Vec::new(),
            ),
        ]);
        run_session_loop(&mut terminal, &mut resumed_b, &mut unlock_b_events)?;
        assert!(!resumed_b.is_curtain_open());
        let unlocked_b = term_capture(&terminal);
        assert!(
            unlocked_b.contains("Seat 0 Private Vault")
                && unlocked_b.contains(&s0b_mandate_clip)
                && unlocked_b.contains("PIN_NOTE_731"),
            "right PIN must restore Seat 0's mandate and note:\n{unlocked_b}"
        );
        let mut tab2_b_events = ScriptedSessionEvents::from_timed_with_queued([(
            host::CURTAIN_LIFT_DEAD_TIME,
            Some(key_ev(KeyCode::Char('2'), KeyModifiers::NONE)),
            Vec::new(),
        )]);
        tab2_b_events.clock = unlock_b_events.elapsed();
        run_session_loop(&mut terminal, &mut resumed_b, &mut tab2_b_events)?;
        let tab2_b_buf = term_capture(&terminal);
        assert!(
            tab2_b_buf.contains("debt_repay=731"),
            "right PIN must restore Seat 0's staged bundle value on Tab 2:\n{tab2_b_buf}"
        );

        let _ = std::fs::remove_dir_all(&base_dir);
        Ok(())
    }

    #[test]
    fn resume_without_pins_starts_behind_curtain_and_lifts_on_space_enter()
    -> Result<(), Box<dyn std::error::Error>> {
        use crossterm::event::KeyEventKind;
        use league_core::Seat;
        use league_seat_types::{BundleDraftAction, SeatIntent};

        let key_ev = |code: KeyCode, mods: KeyModifiers| {
            let mut k = KeyEvent::new(code, mods);
            k.kind = KeyEventKind::Press;
            Event::Key(k)
        };
        let term_capture = |term: &Terminal<TestBackend>| {
            let buf = term.backend().buffer();
            (0..buf.area.height)
                .map(|y| {
                    (0..buf.area.width)
                        .map(|x| buf[(x, y)].symbol().to_string())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let ring = default_dev_ring_table()?;
        let dir = std::env::temp_dir().join(format!("ml_resume_nopins_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        let (save_path, seal_path) = default_save_paths(&dir, TableId(81_513));
        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;

        let mut sess = host::TuiHostSession::from_ring_table(&ring, TableId(81_513), &[0, 1], 43)?;
        let s0_mandate = host::mandate_short_marker(
            sess.build_seat_observation(Seat(0))?
                .own_mandate()
                .ok_or("s0 mandate")?
                .kind,
        );
        let s0_mandate_clip: String = s0_mandate.chars().take(18).collect();
        let mut steps: Vec<(Duration, Option<Event>, Vec<Event>)> = vec![(
            Duration::ZERO,
            Some(key_ev(KeyCode::Char('n'), KeyModifiers::NONE)),
            Vec::new(),
        )];
        for ch in "NOPIN_NOTE_842".chars() {
            steps.push((
                Duration::ZERO,
                Some(key_ev(KeyCode::Char(ch), KeyModifiers::NONE)),
                Vec::new(),
            ));
        }
        steps.push((
            Duration::ZERO,
            Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
            Vec::new(),
        ));
        let mut note_events = ScriptedSessionEvents::from_timed_with_queued(steps);
        run_session_loop(&mut terminal, &mut sess, &mut note_events)?;
        sess.apply_seat_intent(
            Seat(0),
            SeatIntent::update_bundle(BundleDraftAction::SetDebtRepaymentCred(842)),
        )
        .map_err(|e| e.to_string())?;
        sess.write_save_and_seal(&save_path, &seal_path)
            .map_err(|e| e.to_string())?;
        drop(sess);

        let mut resumed = build_session_for_cli(&CliOptions {
            save_dir: Some(dir.clone()),
            resume_path: Some(save_path),
            turn_clock_secs: None,
            untimed: false,
            pins_enabled: false,
            ..CliOptions::default()
        })?;
        assert!(resumed.is_curtain_open());
        terminal.draw(|f| {
            let _ = resumed.render_current_frame(f);
        })?;
        assert_eq!(resumed.observe_seat_call_count(), 0);
        let curtain_buf = term_capture(&terminal);
        assert!(curtain_buf.contains("Hand keyboard to Seat 0"));
        assert!(!curtain_buf.contains(&s0_mandate_clip));
        assert!(!curtain_buf.contains("842"));
        assert!(!curtain_buf.contains("NOPIN_NOTE_842"));
        assert!(!curtain_buf.contains("Private Mandate"));

        let mut lift_events = ScriptedSessionEvents::from_timed_with_queued([
            (
                host::CURTAIN_LIFT_DEAD_TIME,
                Some(key_ev(KeyCode::Char(' '), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                Vec::new(),
            ),
        ]);
        run_session_loop(&mut terminal, &mut resumed, &mut lift_events)?;
        assert!(!resumed.is_curtain_open());
        let lifted_buf = term_capture(&terminal);
        assert!(
            lifted_buf.contains("Seat 0 Private Vault")
                && lifted_buf.contains(&s0_mandate_clip)
                && lifted_buf.contains("NOPIN_NOTE_842"),
            "Space/Enter must lift Curtain into Seat 0's resumed view:\n{lifted_buf}"
        );
        let mut tab2_events = ScriptedSessionEvents::from_timed_with_queued([(
            host::CURTAIN_LIFT_DEAD_TIME,
            Some(key_ev(KeyCode::Char('2'), KeyModifiers::NONE)),
            Vec::new(),
        )]);
        tab2_events.clock = lift_events.elapsed();
        run_session_loop(&mut terminal, &mut resumed, &mut tab2_events)?;
        let tab2_buf = term_capture(&terminal);
        assert!(
            tab2_buf.contains("debt_repay=842"),
            "Space/Enter lift must restore Seat 0's staged bundle value on Tab 2:\n{tab2_buf}"
        );

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn resume_at_bot_cursor_advances_bots_to_next_human_curtain_without_input()
    -> Result<(), Box<dyn std::error::Error>> {
        use crossterm::event::KeyEventKind;
        use league_core::club::host::{ClubPauseCursor, ClubSessionCheckpoint};
        use league_core::{ClubHost, SealedOrderBundle, Seat};
        use league_gen::ring_table_to_table_spec;

        let ring = default_dev_ring_table()?;
        let dir = std::env::temp_dir().join(format!("ml_resume_bot_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        let (save_path, seal_path) = default_save_paths(&dir, TableId(81_514));

        // 4 seats where only Seat 2 is human => Turn 1 offset 0 is Seat 0 (bot), offset 1 is Seat 1
        // (bot), offset 2 is Seat 2 (human). Persist a cursor pointing at offset 0 with no Curtain.
        let (spec, _cmap, deal, eval_seed) =
            ring_table_to_table_spec(&ring, TableId(81_514), &[2])?;
        let mut raw_host = ClubHost::new(spec, ring.seed, deal, eval_seed, 47)?;
        ClubSessionCheckpoint::store_on_host(
            &mut raw_host,
            ClubSessionCheckpoint {
                pause_cursor: Some(ClubPauseCursor {
                    sub_pass_b: false,
                    pass_offset: 0,
                    curtain_next_seat: None,
                }),
                pending_bundles: vec![SealedOrderBundle::default(); 4],
                sub_pass_b_committed: vec![false; 4],
                seat_notes: vec![String::new(); 4],
                seat_pins: vec![None; 4],
                pins_enabled: false,
                turn_clock_secs: None,
                untimed: false,
            },
        )?;
        raw_host.write_save_and_seal(&save_path, &seal_path)?;
        drop(raw_host);

        let mut resumed = build_session_for_cli(&CliOptions {
            save_dir: Some(dir.clone()),
            resume_path: Some(save_path),
            ..CliOptions::default()
        })?;
        assert!(resumed.is_curtain_open());
        assert_eq!(resumed.active_human_seat(), None);

        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
        terminal.draw(|f| {
            let _ = resumed.render_current_frame(f);
        })?;
        assert_eq!(resumed.observe_seat_call_count(), 0);
        let buf = terminal.backend().buffer();
        let screen = (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            screen.contains("Table #81514 | Turn 1") && screen.contains("Hand keyboard to Seat 2"),
            "bot-cursor resume must advance bots without input to Seat 2's Curtain:\n{screen}"
        );

        let key_ev = |code: KeyCode, mods: KeyModifiers| {
            let mut k = KeyEvent::new(code, mods);
            k.kind = KeyEventKind::Press;
            Event::Key(k)
        };
        let mut lift_events = ScriptedSessionEvents::from_timed_with_queued([
            (
                host::CURTAIN_LIFT_DEAD_TIME,
                Some(key_ev(KeyCode::Char(' '), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                Vec::new(),
            ),
        ]);
        run_session_loop(&mut terminal, &mut resumed, &mut lift_events)?;
        assert_eq!(resumed.active_human_seat(), Some(Seat(2)));

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn curtain_open_discards_queued_input_and_enforces_dead_time_on_timeout_and_pass_end()
    -> Result<(), Box<dyn std::error::Error>> {
        use crossterm::event::KeyEventKind;
        use league_core::Seat;
        use std::cell::RefCell;
        use std::rc::Rc;

        struct RecordingTestBackend {
            inner: TestBackend,
            frames: Rc<RefCell<Vec<String>>>,
        }

        impl Backend for RecordingTestBackend {
            type Error = <TestBackend as Backend>::Error;

            fn draw<'a, I>(&mut self, content: I) -> Result<(), Self::Error>
            where
                I: Iterator<Item = (u16, u16, &'a ratatui::buffer::Cell)>,
            {
                self.inner.draw(content)
            }

            fn hide_cursor(&mut self) -> Result<(), Self::Error> {
                self.inner.hide_cursor()
            }

            fn show_cursor(&mut self) -> Result<(), Self::Error> {
                self.inner.show_cursor()
            }

            fn get_cursor_position(&mut self) -> Result<ratatui::layout::Position, Self::Error> {
                self.inner.get_cursor_position()
            }

            fn set_cursor_position<P: Into<ratatui::layout::Position>>(
                &mut self,
                position: P,
            ) -> Result<(), Self::Error> {
                self.inner.set_cursor_position(position)
            }

            fn clear(&mut self) -> Result<(), Self::Error> {
                self.inner.clear()
            }

            fn clear_region(
                &mut self,
                clear_type: ratatui::backend::ClearType,
            ) -> Result<(), Self::Error> {
                self.inner.clear_region(clear_type)
            }

            fn size(&self) -> Result<ratatui::layout::Size, Self::Error> {
                self.inner.size()
            }

            fn window_size(&mut self) -> Result<ratatui::backend::WindowSize, Self::Error> {
                self.inner.window_size()
            }

            fn flush(&mut self) -> Result<(), Self::Error> {
                self.inner.flush()?;
                let buf = self.inner.buffer();
                let snap = (0..buf.area.height)
                    .map(|y| {
                        (0..buf.area.width)
                            .map(|x| buf[(x, y)].symbol().to_string())
                            .collect::<String>()
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                self.frames.borrow_mut().push(snap);
                Ok(())
            }
        }

        let key_ev = |code: KeyCode, mods: KeyModifiers| {
            let mut k = KeyEvent::new(code, mods);
            k.kind = KeyEventKind::Press;
            Event::Key(k)
        };

        let ring = default_dev_ring_table()?;
        let mut session =
            host::TuiHostSession::from_ring_table(&ring, TableId(81_515), &[0, 1], 53)?;
        session.configure_turn_clock(Some(180)); // H=2 => Pass A 67s, Pass B 22s
        let s0_mandate = host::mandate_short_marker(
            session
                .build_seat_observation(Seat(0))?
                .own_mandate()
                .ok_or("s0 mandate")?
                .kind,
        );
        let s1_mandate = host::mandate_short_marker(
            session
                .build_seat_observation(Seat(1))?
                .own_mandate()
                .ok_or("s1 mandate")?
                .kind,
        );
        let s0_mandate_clip: String = s0_mandate.chars().take(18).collect();
        let s1_mandate_clip: String = s1_mandate.chars().take(18).collect();
        session.reset_observe_seat_calls();

        let frames = Rc::new(RefCell::new(Vec::<String>::new()));
        let mut terminal = Terminal::new(RecordingTestBackend {
            inner: TestBackend::new(100, 30),
            frames: Rc::clone(&frames),
        })?;

        // Setup Pass A notes on both seats (`S0_NOTE_321` and `S1_NOTE_654`) and reach Pass B
        // with Seat 0 active.
        let mut setup_steps: Vec<(Duration, Option<Event>, Vec<Event>)> = vec![(
            Duration::ZERO,
            Some(key_ev(KeyCode::Char('n'), KeyModifiers::NONE)),
            Vec::new(),
        )];
        for ch in "S0_NOTE_321".chars() {
            setup_steps.push((
                Duration::ZERO,
                Some(key_ev(KeyCode::Char(ch), KeyModifiers::NONE)),
                Vec::new(),
            ));
        }
        setup_steps.extend([
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Char('l'), KeyModifiers::CONTROL)),
                Vec::new(),
            ),
            (
                host::CURTAIN_LIFT_DEAD_TIME,
                Some(key_ev(KeyCode::Char(' '), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                host::CURTAIN_LIFT_DEAD_TIME,
                Some(key_ev(KeyCode::Char('n'), KeyModifiers::NONE)),
                Vec::new(),
            ),
        ]);
        for ch in "S1_NOTE_654".chars() {
            setup_steps.push((
                Duration::ZERO,
                Some(key_ev(KeyCode::Char(ch), KeyModifiers::NONE)),
                Vec::new(),
            ));
        }
        setup_steps.extend([
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Char('l'), KeyModifiers::CONTROL)),
                Vec::new(),
            ),
            (
                host::CURTAIN_LIFT_DEAD_TIME,
                Some(key_ev(KeyCode::Char(' '), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                Vec::new(),
            ),
        ]);
        let mut setup_events = ScriptedSessionEvents::from_timed_with_queued(setup_steps);
        run_session_loop(&mut terminal, &mut session, &mut setup_events)?;
        assert_eq!(session.pass(), league_seat_ui::PassKind::SubPassB);
        assert_eq!(session.active_human_seat(), Some(Seat(0)));

        // Case 1 (timeout hand-off): queue `' '`, `Enter`, `'q'` right as Pass B (22s) expires.
        let timeout_start = frames.borrow().len();
        let mut timeout_events = ScriptedSessionEvents::from_timed_with_queued([(
            Duration::from_secs(25),
            None,
            vec![
                key_ev(KeyCode::Char(' '), KeyModifiers::NONE),
                key_ev(KeyCode::Enter, KeyModifiers::NONE),
                key_ev(KeyCode::Char('q'), KeyModifiers::NONE),
            ],
        )]);
        timeout_events.clock = setup_events.elapsed();
        run_session_loop(&mut terminal, &mut session, &mut timeout_events)?;
        assert_eq!(timeout_events.drain_calls, 1);
        assert!(session.is_curtain_open());
        assert!(!session.quit_requested());
        {
            let recorded = frames.borrow();
            let timeout_slice = &recorded[timeout_start..];
            assert!(
                timeout_slice
                    .iter()
                    .any(|f| f.contains("Hand keyboard to Seat 1"))
            );
            for frame in timeout_slice {
                assert!(!frame.contains("S1_NOTE_654"));
                assert!(!frame.contains("Seat 1 Private Vault"));
                if s1_mandate_clip != s0_mandate_clip {
                    assert!(!frame.contains(&s1_mandate_clip));
                }
            }
        }

        // Send `' '`, `Enter`, `'q'`, `Esc`, `Ctrl+C` during the 400 ms dead time after open.
        session.reset_observe_seat_calls();
        let timeout_dead_start = frames.borrow().len();
        let mut timeout_dead_events = ScriptedSessionEvents::from_timed_with_queued([
            (
                Duration::from_millis(50),
                Some(key_ev(KeyCode::Char(' '), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::from_millis(50),
                Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::from_millis(50),
                Some(key_ev(KeyCode::Char('q'), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::from_millis(50),
                Some(key_ev(KeyCode::Esc, KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::from_millis(50),
                Some(key_ev(KeyCode::Char('c'), KeyModifiers::CONTROL)),
                Vec::new(),
            ),
        ]);
        timeout_dead_events.clock = timeout_events.elapsed();
        run_session_loop(&mut terminal, &mut session, &mut timeout_dead_events)?;
        assert!(session.is_curtain_open());
        assert!(!session.quit_requested());
        assert_eq!(session.observe_seat_call_count(), 0);
        {
            let recorded = frames.borrow();
            for frame in &recorded[timeout_dead_start..] {
                assert!(!frame.contains("S1_NOTE_654"));
                assert!(!frame.contains(&s1_mandate_clip));
                assert!(!frame.contains("Private Mandate"));
                assert!(!frame.contains("Seat 1 Private Vault"));
            }
        }

        // After dead time, one fresh `Space` then `Enter` lifts the Curtain cleanly onto Seat 1.
        let s1_lift_start = frames.borrow().len();
        let mut s1_lift_events = ScriptedSessionEvents::from_timed_with_queued([
            (
                host::CURTAIN_LIFT_DEAD_TIME,
                Some(key_ev(KeyCode::Char(' '), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                Vec::new(),
            ),
        ]);
        s1_lift_events.clock = timeout_dead_events.elapsed();
        run_session_loop(&mut terminal, &mut session, &mut s1_lift_events)?;
        assert!(!session.is_curtain_open());
        assert_eq!(session.active_human_seat(), Some(Seat(1)));
        {
            let recorded = frames.borrow();
            let last_s1 = recorded[s1_lift_start..].last().ok_or("missing s1 frame")?;
            assert!(
                last_s1.contains("Seat 1 Private Vault")
                    && last_s1.contains("S1_NOTE_654")
                    && last_s1.contains(&s1_mandate_clip),
                "fresh Space/Enter after timeout dead time must reveal Seat 1's view:\n{last_s1}"
            );
        }

        // Advance Seat 1 through Sweep into Turn 2 Pass A (Seat 1 active) so Case 2 tests ordinary
        // Ctrl+L hand-off from Seat 1 -> Seat 0.
        let mut to_turn2_s1 = ScriptedSessionEvents::from_timed_with_queued([
            (
                host::CURTAIN_LIFT_DEAD_TIME,
                Some(key_ev(KeyCode::Char('l'), KeyModifiers::CONTROL)),
                Vec::new(),
            ),
            (
                host::CURTAIN_LIFT_DEAD_TIME,
                Some(key_ev(KeyCode::Char(' '), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                Vec::new(),
            ),
        ]);
        to_turn2_s1.clock = s1_lift_events.elapsed();
        run_session_loop(&mut terminal, &mut session, &mut to_turn2_s1)?;
        assert_eq!(session.turn(), 2);
        assert_eq!(session.active_human_seat(), Some(Seat(1)));

        // Case 2 (ordinary end-of-pass Ctrl+L hand-off): queue `' '`, `Enter`, `'q'` on Ctrl+L.
        let pass_end_start = frames.borrow().len();
        let mut pass_end_events = ScriptedSessionEvents::from_timed_with_queued([(
            host::CURTAIN_LIFT_DEAD_TIME,
            Some(key_ev(KeyCode::Char('l'), KeyModifiers::CONTROL)),
            vec![
                key_ev(KeyCode::Char(' '), KeyModifiers::NONE),
                key_ev(KeyCode::Enter, KeyModifiers::NONE),
                key_ev(KeyCode::Char('q'), KeyModifiers::NONE),
            ],
        )]);
        pass_end_events.clock = to_turn2_s1.elapsed();
        run_session_loop(&mut terminal, &mut session, &mut pass_end_events)?;
        assert_eq!(pass_end_events.drain_calls, 1);
        assert!(session.is_curtain_open());
        assert!(!session.quit_requested());
        {
            let recorded = frames.borrow();
            let handoff_slice = &recorded[pass_end_start..];
            assert!(
                handoff_slice
                    .iter()
                    .any(|f| f.contains("Hand keyboard to Seat 0"))
            );
            for frame in handoff_slice {
                assert!(!frame.contains("S0_NOTE_321"));
                assert!(!frame.contains("Seat 0 Private Vault"));
                if s0_mandate_clip != s1_mandate_clip {
                    assert!(!frame.contains(&s0_mandate_clip));
                }
            }
        }

        // Dead-time keys `< 400 ms` after Ctrl+L Curtain open also must not lift or quit.
        session.reset_observe_seat_calls();
        let pass_dead_start = frames.borrow().len();
        let mut pass_dead_events = ScriptedSessionEvents::from_timed_with_queued([
            (
                Duration::from_millis(50),
                Some(key_ev(KeyCode::Char(' '), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::from_millis(50),
                Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::from_millis(50),
                Some(key_ev(KeyCode::Char('q'), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::from_millis(50),
                Some(key_ev(KeyCode::Esc, KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::from_millis(50),
                Some(key_ev(KeyCode::Char('c'), KeyModifiers::CONTROL)),
                Vec::new(),
            ),
        ]);
        pass_dead_events.clock = pass_end_events.elapsed();
        run_session_loop(&mut terminal, &mut session, &mut pass_dead_events)?;
        assert!(session.is_curtain_open());
        assert!(!session.quit_requested());
        assert_eq!(session.observe_seat_call_count(), 0);
        {
            let recorded = frames.borrow();
            for frame in &recorded[pass_dead_start..] {
                assert!(!frame.contains("S0_NOTE_321"));
                assert!(!frame.contains(&s0_mandate_clip));
                assert!(!frame.contains("Private Mandate"));
                assert!(!frame.contains("Seat 0 Private Vault"));
            }
        }

        let s0_lift_start = frames.borrow().len();
        let mut s0_lift_events = ScriptedSessionEvents::from_timed_with_queued([
            (
                host::CURTAIN_LIFT_DEAD_TIME,
                Some(key_ev(KeyCode::Char(' '), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::ZERO,
                Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                Vec::new(),
            ),
        ]);
        s0_lift_events.clock = pass_dead_events.elapsed();
        run_session_loop(&mut terminal, &mut session, &mut s0_lift_events)?;
        assert!(!session.is_curtain_open());
        assert_eq!(session.active_human_seat(), Some(Seat(0)));
        let recorded = frames.borrow();
        let last_s0 = recorded[s0_lift_start..].last().ok_or("missing s0 frame")?;
        assert!(
            last_s0.contains("Seat 0 Private Vault")
                && last_s0.contains("S0_NOTE_321")
                && last_s0.contains(&s0_mandate_clip),
            "fresh Space/Enter after dead time must reveal Seat 0's view:\n{last_s0}"
        );
        Ok(())
    }

    #[test]
    fn lobby_loop_configures_table_starts_behind_curtain_and_quits_cleanly()
    -> Result<(), Box<dyn std::error::Error>> {
        use crossterm::event::KeyEventKind;
        let key_ev = |code: KeyCode, mods: KeyModifiers| {
            let mut k = KeyEvent::new(code, mods);
            k.kind = KeyEventKind::Press;
            Event::Key(k)
        };

        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
        let opts = CliOptions {
            untimed: true,
            ..CliOptions::default()
        };

        // Case 1: Quit directly from Lobby with `q` returns Ok(None).
        let mut quit_events =
            ScriptedSessionEvents::from_events([key_ev(KeyCode::Char('q'), KeyModifiers::NONE)]);
        let quit_res = run_lobby_loop(&mut terminal, &opts, &mut quit_events, None)?;
        assert!(quit_res.is_none());

        // Case 2: Toggle Seat 1 -> Human (`Down`, `Down`, `Space`), jump to Start (`Down` x6), `Enter`
        // starts behind Seat 0 Curtain with `observe_seat_call_count() == 0`.
        let mut start_steps = vec![
            key_ev(KeyCode::Down, KeyModifiers::NONE),
            key_ev(KeyCode::Down, KeyModifiers::NONE),
            key_ev(KeyCode::Char(' '), KeyModifiers::NONE),
        ];
        for _ in 0..6 {
            start_steps.push(key_ev(KeyCode::Down, KeyModifiers::NONE));
        }
        start_steps.push(key_ev(KeyCode::Enter, KeyModifiers::NONE));
        let mut start_events = ScriptedSessionEvents::from_events(start_steps);
        let session = run_lobby_loop(&mut terminal, &opts, &mut start_events, None)?
            .ok_or("lobby StartSeason should produce TuiHostSession")?;
        assert!(session.is_curtain_open());
        assert_eq!(session.observe_seat_call_count(), 0);
        assert_eq!(session.pass_budget_secs(), None);
        Ok(())
    }

    #[test]
    fn tui_error_follows_structured_shape_and_scrubs_secrets_in_text_and_json()
    -> Result<(), Box<dyn std::error::Error>> {
        use league_core::StructuredError;
        let seeded_secret = "sEdSKaCy2JT7JaM7v95H9SxkhP9wS2r";
        let api_secret = "sk-or-v1-plantedTuiErrSecret99";

        let samples: &[(host::TuiError, u8, &str)] = &[
            (
                host::TuiError::InvalidUsage(format!("bad flag with {seeded_secret}")),
                1,
                "INVALID_USAGE",
            ),
            (host::TuiError::PinCliForbidden, 1, "PIN_CLI_FORBIDDEN"),
            (host::TuiError::TtyRequired, 1, "TTY_REQUIRED"),
            (
                host::TuiError::InvalidResumeTarget(format!("missing path {api_secret}")),
                1,
                "INVALID_RESUME_TARGET",
            ),
            (
                host::TuiError::SaveDirConflict(StructuredError::new(
                    "SEASON_SAVE_EXISTS",
                    format!("occupied dir {seeded_secret}"),
                    "Choose another dir.",
                    false,
                )),
                1,
                "SEASON_SAVE_EXISTS",
            ),
            (
                host::TuiError::AutosaveFailed {
                    save_path: PathBuf::from("saves/club-1.save.json"),
                    cause: StructuredError::new(
                        "TUI_IO_ERROR",
                        format!("permission denied {api_secret}"),
                        "check disk",
                        true,
                    ),
                },
                2,
                "TUI_AUTOSAVE_FAILED",
            ),
            (
                host::TuiError::SaveLoadFailed(StructuredError::new(
                    "INVALID_CHECKPOINT_PIN_FORMAT",
                    "bad pin in seal",
                    "fix seal",
                    false,
                )),
                2,
                "INVALID_CHECKPOINT_PIN_FORMAT",
            ),
            (
                host::TuiError::TerminalIo(format!("tty write failed {seeded_secret}")),
                2,
                "TUI_TERMINAL_IO",
            ),
            (
                host::TuiError::SessionHalted(StructuredError::new(
                    "SESSION_WEDGED",
                    format!("wedged with {api_secret}"),
                    "resume",
                    false,
                )),
                2,
                "SESSION_WEDGED",
            ),
            (
                host::TuiError::PostSeasonExportFailed(StructuredError::new(
                    "EXPORT_IO",
                    format!("cannot write replay {seeded_secret}"),
                    "retry",
                    true,
                )),
                3,
                "TUI_REPLAY_EXPORT_FAILED",
            ),
            (
                host::TuiError::PostSeasonExportFailed(StructuredError::new(
                    "REPLAY_TARGET_EXISTS",
                    format!("replay target occupied {api_secret}"),
                    "use --save-dir",
                    false,
                )),
                3,
                "REPLAY_TARGET_EXISTS",
            ),
        ];

        for (err, expected_exit, expected_code) in samples {
            assert_eq!(err.exit_code(), *expected_exit);
            let structured = err.to_structured_error();
            assert_eq!(structured.code, *expected_code);
            assert!(!structured.hint.is_empty());

            let text_line = host::ErrorFormat::Text.format_line(&structured);
            let json_line = host::ErrorFormat::Json.format_line(&structured);
            let disp = format!("{err}");
            let dbg = format!("{err:?}");
            for rendered in [&text_line, &json_line, &disp, &dbg] {
                assert!(
                    !rendered.contains(seeded_secret) && !rendered.contains(api_secret),
                    "TuiError `{expected_code}` leaked secret:\n{rendered}"
                );
                assert!(
                    rendered.contains(expected_code),
                    "rendered missing code `{expected_code}`:\n{rendered}"
                );
            }
            assert!(text_line.starts_with(&format!("error: [{expected_code}] ")));
            assert!(text_line.contains("; hint: "));
            assert!(json_line.starts_with('{') && json_line.ends_with('}'));
            assert!(json_line.contains(&format!("\"code\":\"{expected_code}\"")));
            assert!(json_line.contains("\"hint\":\""));
            assert!(json_line.contains("\"retryable\":"));
        }

        // Verify live failed autosave on `TuiHostSession` surfaces `TuiError::AutosaveFailed` (`exit 2`).
        let dir = std::env::temp_dir().join(format!("ml_tui_err_autosave_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        let blocker = dir.join("not_a_dir");
        std::fs::write(&blocker, b"file")?;
        let ring = default_dev_ring_table()?;
        let mut session = host::TuiHostSession::from_ring_table(&ring, TableId(81_701), &[0], 11)?;
        session.enable_autosave(
            blocker.join("club-81701.save.json"),
            blocker.join("club-81701.seal"),
        );
        let _ = session.handle_key_event(
            league_seat_types::SeatKeyEvent::new(
                league_seat_types::SeatKeyCode::Char('l'),
                league_seat_types::SeatKeyModifiers {
                    ctrl: true,
                    alt: false,
                    shift: false,
                },
            ),
            ViewportSize::new(100, 30),
        );
        let tui_err = session
            .terminal_tui_error()
            .ok_or("failed autosave must populate terminal_tui_error")?;
        assert_eq!(tui_err.exit_code(), 2);
        let st = tui_err.to_structured_error();
        assert_eq!(st.code, "TUI_AUTOSAVE_FAILED");
        assert!(st.message.contains("Autosave failed at "));
        assert!(st.message.contains("club-81701.save.json"));
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn cli_exit_codes_cover_0_1_2_3_and_non_tty_exits_1_before_raw_mode()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut out = Vec::new();
        let mut err = Vec::new();

        // 0: --help and --version succeed without a TTY.
        assert_eq!(
            dispatch_cli_with_env(
                ["league-tui".to_owned(), "--help".to_owned()],
                |_| None,
                false,
                &mut out,
                &mut err,
                run_interactive,
            ),
            ExitCode::from(0)
        );
        out.clear();
        assert_eq!(
            dispatch_cli_with_env(
                ["league-tui".to_owned(), "--version".to_owned()],
                |_| None,
                false,
                &mut out,
                &mut err,
                run_interactive,
            ),
            ExitCode::from(0)
        );

        // 1: real non-TTY invocation using `run_interactive` exits 1 (`TTY_REQUIRED`) BEFORE raw
        // mode or alternate screen is entered, in both `text` and `json` error formats.
        err.clear();
        let non_tty_text = dispatch_cli_with_env(
            ["league-tui".to_owned()],
            |_| None,
            false,
            &mut out,
            &mut err,
            run_interactive,
        );
        assert_eq!(non_tty_text, ExitCode::from(1));
        assert!(String::from_utf8_lossy(&err).contains("[TTY_REQUIRED]"));

        err.clear();
        let non_tty_json = dispatch_cli_with_env(
            [
                "league-tui".to_owned(),
                "--error-format".to_owned(),
                "json".to_owned(),
            ],
            |_| None,
            false,
            &mut out,
            &mut err,
            run_interactive,
        );
        assert_eq!(non_tty_json, ExitCode::from(1));
        let json_str = String::from_utf8_lossy(&err);
        assert!(json_str.contains("\"code\":\"TTY_REQUIRED\""));
        assert!(json_str.contains("\"retryable\":false"));

        // 1: user/config CLI errors (`--pin`, conflicting log levels, invalid env, missing resume).
        for (bad_args, env_val, expected_code) in [
            (vec!["league-tui", "--pin=1234"], None, "PIN_CLI_FORBIDDEN"),
            (
                vec!["league-tui", "--silent", "--verbose"],
                None,
                "INVALID_USAGE",
            ),
            (vec!["league-tui", "-q", "-v"], None, "INVALID_USAGE"),
            (vec!["league-tui"], Some("noisy"), "INVALID_USAGE"),
        ] {
            err.clear();
            let owned: Vec<String> = bad_args.into_iter().map(str::to_owned).collect();
            let code = dispatch_cli_with_env(
                owned,
                |k| (k == host::LEAGUE_LOG_LEVEL_ENV).then(|| env_val.unwrap_or("").to_owned()),
                true,
                &mut out,
                &mut err,
                |_, _, _| Ok(()),
            );
            assert_eq!(code, ExitCode::from(1));
            assert!(
                String::from_utf8_lossy(&err).contains(expected_code),
                "expected `{expected_code}` in stderr: {}",
                String::from_utf8_lossy(&err)
            );
        }

        let dir = std::env::temp_dir().join(format!("ml_cli_exit_codes_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;

        // Missing --resume path -> exit 1 (`INVALID_RESUME_TARGET`) before raw mode.
        let missing_save = dir.join("missing.save.json");
        let missing_res = build_session_for_cli(&CliOptions {
            resume_path: Some(missing_save),
            ..CliOptions::default()
        })
        .err()
        .ok_or("missing save must fail")?;
        assert_eq!(missing_res.exit_code(), 1);
        assert_eq!(
            missing_res.to_structured_error().code,
            "INVALID_RESUME_TARGET"
        );

        // 2: corrupt .seal on --resume -> exit 2 (`SaveLoadFailed`).
        let save_path = dir.join("club-81702.save.json");
        let seal_path = dir.join("club-81702.seal");
        let ring = default_dev_ring_table()?;
        let mut session = host::TuiHostSession::from_ring_table(&ring, TableId(81_702), &[0], 13)?;
        session.write_save_and_seal(&save_path, &seal_path)?;
        std::fs::write(&seal_path, b"{not valid json")?;
        let corrupt_res = build_session_for_cli(&CliOptions {
            resume_path: Some(save_path),
            ..CliOptions::default()
        })
        .err()
        .ok_or("corrupt seal must fail")?;
        assert_eq!(corrupt_res.exit_code(), 2);
        let corrupt_st = corrupt_res.to_structured_error();
        assert_eq!(corrupt_st.code, "INVALID_CONFIG");
        assert!(corrupt_st.message.contains("MALFORMED_SEAL_JSON"));

        // 2: runtime error from runner -> exit 2.
        err.clear();
        let runtime_code = dispatch_cli_with_env(
            ["league-tui".to_owned()],
            |_| None,
            true,
            &mut out,
            &mut err,
            |_, _, _| Err(corrupt_res),
        );
        assert_eq!(runtime_code, ExitCode::from(2));
        assert!(String::from_utf8_lossy(&err).contains("MALFORMED_SEAL_JSON"));

        // 3: partial post-season failure (season finalized, replay export failed) -> exit 3.
        err.clear();
        let partial_code = dispatch_cli_with_env(
            ["league-tui".to_owned(), "--error-format=json".to_owned()],
            |_| None,
            true,
            &mut out,
            &mut err,
            |_, _, _| {
                Err(host::TuiError::PostSeasonExportFailed(
                    league_core::StructuredError::new(
                        "TUI_REPLAY_EXPORT_FAILED",
                        "Replay export failed at /bad/path",
                        "retry export",
                        true,
                    ),
                ))
            },
        );
        assert_eq!(partial_code, ExitCode::from(3));
        assert!(String::from_utf8_lossy(&err).contains("\"code\":\"TUI_REPLAY_EXPORT_FAILED\""));

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    /// Drives a real Club Mode table across all four log levels (`Silent`, `Normal`, `Verbose`,
    /// `Debug`) using a real `TuiDiagnosticSink` (stderr capture + `--log-file`, zero markers
    /// injected into logger or panic hook) and real `handle_panic_with_restore`.
    ///
    /// Paths driven and checked here:
    /// - `run_lobby_loop_with_sink` (`NO_HUMAN_SEATS` refusal + table start);
    /// - `run_session_loop_with_sink` across every pass from Turn 1 through season close (Curtain
    ///   open/lift, `CSI 2 J` clear, input drain, `--pins` set & verify, Vault note entry, in-turn
    ///   `CancelOffer` `UNKNOWN_OBJECT` refusal, staged `SealedOrderBundle` + `PreimageSecret`, and
    ///   every Sweep through Turn 24..=32 close);
    /// - all `AwaitingMandateReveal` reveal steps, one failed (`TUI_REPLAY_EXPORT_FAILED`, exit 3)
    ///   and one succeeded replay export, and clean `q` exit (`run_session_loop_with_sink`);
    /// - alternate-screen suppression, `--debug` vs non-debug panic hook (`{session:?}`,
    ///   `{active_view:?}`, `{:?}` of `active_view.zones`, XRPL seed, API key), plus an explicit
    ///   4-seat `1 Human + 3 ClubBot` `Debug` Sweep + `Debug` panic backtrace check with unique
    ///   5-digit `MinProjectEquity` targets `[91_234..=94_567]` and 4-digit bonuses
    ///   `[6_123..=7_987]` (`> main.rs line count`).
    ///
    /// Paths NOT driven in this single test (covered elsewhere or not automated):
    /// - bare `< 100` 1-2 digit mandate counts (`distinct_goods` / `min_partners`) as standalone
    ///   substring needles (because they collide with `turn=`/`table=#`; full `MandateKind` short
    ///   and `Debug` strings ARE checked), and 4-digit numbers inside `--debug` backtraces of the
    ///   main loop (checked on the panic header line and in the explicit 4-seat `Debug` sub-check);
    /// - `dispatch_cli_with_env` stderr error-line formatting, wrong-PIN tries, timed timeouts,
    ///   `resumed session (...)`, `lobby quit`, `queued terminal input drained on key`,
    ///   `frame render error`, `session stream ended`, 5-/6-seat log runs, and live OS TTY launch.
    #[test]
    fn real_session_all_four_log_levels_and_panic_hook_emit_no_private_values_and_suppress_alt_screen()
    -> Result<(), Box<dyn std::error::Error>> {
        use crossterm::event::KeyEventKind;
        use league_seat_types::{SeatKeyCode, SeatKeyEvent, SeatKeyModifiers};

        let key_ev = |code: KeyCode, mods: KeyModifiers| {
            let mut k = KeyEvent::new(code, mods);
            k.kind = KeyEventKind::Press;
            Event::Key(k)
        };
        let s_key = |c: char| SeatKeyEvent::new(SeatKeyCode::Char(c), SeatKeyModifiers::NONE);

        let collect_mandate_markers = |session: &host::TuiHostSession| -> Result<
            (Vec<String>, Vec<String>),
            Box<dyn std::error::Error>,
        > {
            let mut text_markers = Vec::new();
            let mut num_markers = Vec::new();
            for s in 0..session.seat_count() {
                let obs = session.build_seat_observation(league_core::Seat(s))?;
                let mandate = obs.own_mandate().ok_or("missing seat mandate")?;
                text_markers.push(host::mandate_short_marker(mandate.kind));
                text_markers.push(format!("{:?}", mandate.kind));
                text_markers.push(format!("{:#?}", mandate.kind));
                let variant = match mandate.kind {
                    league_core::MandateKind::MinProjectEquity { target } => {
                        if target >= 100 {
                            num_markers.push(target.to_string());
                        }
                        "MinProjectEquity"
                    }
                    league_core::MandateKind::MinRefinedVariety { min_each, .. } => {
                        if min_each >= 100 {
                            num_markers.push(min_each.to_string());
                        }
                        "MinRefinedVariety"
                    }
                    league_core::MandateKind::MaintainCleanCredit { min_tier } => {
                        text_markers.push(format!("{min_tier:?}"));
                        "MaintainCleanCredit"
                    }
                    league_core::MandateKind::MinActivePartners { .. } => "MinActivePartners",
                };
                text_markers.push(variant.to_owned());
                num_markers.push(mandate.bonus_equity.to_string());
            }
            Ok((text_markers, num_markers))
        };

        let planted_pin = "9483";
        let planted_note = "REAL_PATH_NOTE_991";
        let planted_debt_num = "765432";
        let planted_xrpl = "sEdSKaCy2JT7JaM7v95H9SxkhP9wS2r";
        let planted_api_key = "sk-or-v1-logSecretKey77";

        // Drive real Lobby + session + refusal + Sweep + reveal + failed/succeeded export + panic
        // across all 4 log levels using real `TuiDiagnosticSink` (zero handed-in markers).
        for level in [
            host::LogLevel::Silent,
            host::LogLevel::Normal,
            host::LogLevel::Verbose,
            host::LogLevel::Debug,
        ] {
            let dir = std::env::temp_dir().join(format!(
                "ml_real_log_level_{}_{}",
                level.as_str(),
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir)?;
            let save_dir = dir.join("saves");
            std::fs::create_dir_all(&save_dir)?;
            let log_file_path = dir.join(format!("diag-{}.log", level.as_str()));

            let written = std::rc::Rc::new(std::cell::RefCell::new(Vec::<u8>::new()));
            let mut alt_term = Terminal::new(FixedSizeCrosstermBackend::new(
                SharedVecWriter(written.clone()),
                100,
                30,
            ))?;
            let mut sink = host::TuiDiagnosticSink::new(level, Some(log_file_path.clone()));
            sink.set_alternate_screen_active(true);

            // 1. Real Lobby loop: first trigger a validation refusal (`NO_HUMAN_SEATS`), then
            // re-enable Seat 0 and start with `--untimed --pins --save-dir <save_dir>`.
            let lobby_opts = match parse_args([
                "league-tui".to_owned(),
                "--untimed".to_owned(),
                "--pins".to_owned(),
                "--save-dir".to_owned(),
                save_dir.to_string_lossy().into_owned(),
            ]) {
                CliAction::RunClub(o) => o,
                other => return Err(format!("expected RunClub lobby opts, got {other:?}").into()),
            };
            let mut lobby_events_vec = vec![
                key_ev(KeyCode::Down, KeyModifiers::NONE),
                key_ev(KeyCode::Char(' '), KeyModifiers::NONE),
            ];
            for _ in 0..7 {
                lobby_events_vec.push(key_ev(KeyCode::Down, KeyModifiers::NONE));
            }
            lobby_events_vec.push(key_ev(KeyCode::Enter, KeyModifiers::NONE));
            for _ in 0..7 {
                lobby_events_vec.push(key_ev(KeyCode::Up, KeyModifiers::NONE));
            }
            lobby_events_vec.push(key_ev(KeyCode::Char(' '), KeyModifiers::NONE));
            for _ in 0..7 {
                lobby_events_vec.push(key_ev(KeyCode::Down, KeyModifiers::NONE));
            }
            lobby_events_vec.push(key_ev(KeyCode::Enter, KeyModifiers::NONE));
            let mut lobby_script = ScriptedSessionEvents::from_events(lobby_events_vec);
            let mut session = run_lobby_loop_with_sink(
                &mut alt_term,
                &lobby_opts,
                &mut lobby_script,
                Some(&save_dir),
                &mut sink,
            )?
            .ok_or("lobby must start session")?;

            let (mut all_text_markers, mandate_number_markers) = collect_mandate_markers(&session)?;
            for s in 0..session.seat_count() {
                let salt_hex = session.turn_reveal_salt_hex(league_core::Seat(s))?;
                let upper = salt_hex.to_ascii_uppercase();
                let prefix = salt_hex[..16].to_owned();
                if let (Ok(b0), Ok(b1), Ok(b2)) = (
                    u8::from_str_radix(&salt_hex[0..2], 16),
                    u8::from_str_radix(&salt_hex[2..4], 16),
                    u8::from_str_radix(&salt_hex[4..6], 16),
                ) {
                    all_text_markers.push(format!("[{b0}, {b1}, {b2}"));
                }
                all_text_markers.push(salt_hex);
                all_text_markers.push(upper);
                all_text_markers.push(prefix);
            }

            // 2. Drive Pass A: lift Curtain, set PIN "9483", enter Vault note, switch to Tab 1
            // `CancelOffer` and press Enter to trigger one refused in-turn command.
            let mut pass_a_events_vec: Vec<(Duration, Option<Event>, Vec<Event>)> = vec![
                (
                    host::CURTAIN_LIFT_DEAD_TIME,
                    Some(key_ev(KeyCode::Char(' '), KeyModifiers::NONE)),
                    Vec::new(),
                ),
                (
                    Duration::ZERO,
                    Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                    Vec::new(),
                ),
            ];
            for d in planted_pin.chars() {
                pass_a_events_vec.push((
                    Duration::ZERO,
                    Some(key_ev(KeyCode::Char(d), KeyModifiers::NONE)),
                    Vec::new(),
                ));
            }
            pass_a_events_vec.push((
                Duration::ZERO,
                Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                Vec::new(),
            ));
            pass_a_events_vec.push((
                host::CURTAIN_LIFT_DEAD_TIME,
                Some(key_ev(KeyCode::Char('n'), KeyModifiers::NONE)),
                Vec::new(),
            ));
            for ch in planted_note.chars() {
                pass_a_events_vec.push((
                    Duration::ZERO,
                    Some(key_ev(KeyCode::Char(ch), KeyModifiers::NONE)),
                    Vec::new(),
                ));
            }
            pass_a_events_vec.push((
                Duration::ZERO,
                Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                Vec::new(),
            ));
            pass_a_events_vec.push((
                Duration::ZERO,
                Some(key_ev(KeyCode::Char('1'), KeyModifiers::NONE)),
                Vec::new(),
            ));
            pass_a_events_vec.push((
                Duration::ZERO,
                Some(key_ev(KeyCode::Char(']'), KeyModifiers::NONE)),
                Vec::new(),
            ));
            pass_a_events_vec.push((
                Duration::ZERO,
                Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                Vec::new(),
            ));

            let mut pass_a_script =
                ScriptedSessionEvents::from_timed_with_queued(pass_a_events_vec);
            run_session_loop_with_sink(&mut alt_term, &mut session, &mut pass_a_script, &mut sink)?;

            // Stage debt repayment + preimage escrow order, then lock Pass A -> Pass B -> unlock
            // with PIN -> lock Pass B -> execute Turn 1 -> Turn 2 Sweep.
            session.apply_seat_intent(
                league_core::Seat(0),
                league_seat_types::SeatIntent::update_bundle(
                    league_seat_types::BundleDraftAction::SetDebtRepaymentCred(765_432),
                ),
            )?;
            session.apply_seat_intent(
                league_core::Seat(0),
                league_seat_types::SeatIntent::update_bundle(
                    league_seat_types::BundleDraftAction::StageEscrowFinish(
                        league_seat_types::EscrowFinishUnlock {
                            escrow_id: 88,
                            preimage: Some(league_core::PreimageSecret([0x5A; 32])),
                        },
                    ),
                ),
            )?;

            let mut sweep_events_vec: Vec<(Duration, Option<Event>, Vec<Event>)> = vec![
                (
                    host::CURTAIN_LIFT_DEAD_TIME,
                    Some(key_ev(KeyCode::Char('l'), KeyModifiers::CONTROL)),
                    Vec::new(),
                ),
                (
                    host::CURTAIN_LIFT_DEAD_TIME,
                    Some(key_ev(KeyCode::Char(' '), KeyModifiers::NONE)),
                    Vec::new(),
                ),
                (
                    Duration::ZERO,
                    Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                    Vec::new(),
                ),
            ];
            for d in planted_pin.chars() {
                sweep_events_vec.push((
                    Duration::ZERO,
                    Some(key_ev(KeyCode::Char(d), KeyModifiers::NONE)),
                    Vec::new(),
                ));
            }
            sweep_events_vec.push((
                Duration::ZERO,
                Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                Vec::new(),
            ));
            sweep_events_vec.push((
                host::CURTAIN_LIFT_DEAD_TIME,
                Some(key_ev(KeyCode::Char('l'), KeyModifiers::CONTROL)),
                Vec::new(),
            ));
            let mut sweep_script = ScriptedSessionEvents::from_timed_with_queued(sweep_events_vec);
            sweep_script.clock = pass_a_script.elapsed();
            run_session_loop_with_sink(&mut alt_term, &mut session, &mut sweep_script, &mut sink)?;
            assert_eq!(
                session.turn(),
                2,
                "real Sweep must advance session to Turn 2"
            );

            // Unlock Turn 2 Seat 0 via `run_session_loop_with_sink` to capture a live active
            // `SeatScreenView` holding Seat 0's dealt mandate and Vault note, then drive every
            // remaining pass through season close via `run_session_loop_with_sink`.
            let vp = ViewportSize::new(100, 30);
            let mut loop_clock = sweep_script.elapsed();
            let mut t2_unlock_events: Vec<(Duration, Option<Event>, Vec<Event>)> = vec![
                (
                    host::CURTAIN_LIFT_DEAD_TIME,
                    Some(key_ev(KeyCode::Char(' '), KeyModifiers::NONE)),
                    Vec::new(),
                ),
                (
                    Duration::ZERO,
                    Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                    Vec::new(),
                ),
            ];
            for d in planted_pin.chars() {
                t2_unlock_events.push((
                    Duration::ZERO,
                    Some(key_ev(KeyCode::Char(d), KeyModifiers::NONE)),
                    Vec::new(),
                ));
            }
            t2_unlock_events.push((
                Duration::ZERO,
                Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                Vec::new(),
            ));
            let mut t2_unlock_script =
                ScriptedSessionEvents::from_timed_with_queued(t2_unlock_events);
            t2_unlock_script.clock = loop_clock;
            run_session_loop_with_sink(
                &mut alt_term,
                &mut session,
                &mut t2_unlock_script,
                &mut sink,
            )?;
            loop_clock = t2_unlock_script.elapsed();

            session.apply_seat_intent(
                league_core::Seat(0),
                league_seat_types::SeatIntent::update_bundle(
                    league_seat_types::BundleDraftAction::SetDebtRepaymentCred(765_432),
                ),
            )?;
            let active_view = session.build_current_view(vp)?;

            while session.phase() == league_core::TurnPhase::TalkAndMarket {
                let mut pass_events: Vec<(Duration, Option<Event>, Vec<Event>)> = Vec::new();
                if session.is_curtain_open() {
                    pass_events.push((
                        host::CURTAIN_LIFT_DEAD_TIME,
                        Some(key_ev(KeyCode::Char(' '), KeyModifiers::NONE)),
                        Vec::new(),
                    ));
                    pass_events.push((
                        Duration::ZERO,
                        Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                        Vec::new(),
                    ));
                    for d in planted_pin.chars() {
                        pass_events.push((
                            Duration::ZERO,
                            Some(key_ev(KeyCode::Char(d), KeyModifiers::NONE)),
                            Vec::new(),
                        ));
                    }
                    pass_events.push((
                        Duration::ZERO,
                        Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                        Vec::new(),
                    ));
                }
                pass_events.push((
                    host::CURTAIN_LIFT_DEAD_TIME,
                    Some(key_ev(KeyCode::Char('l'), KeyModifiers::CONTROL)),
                    Vec::new(),
                ));
                let mut pass_script = ScriptedSessionEvents::from_timed_with_queued(pass_events);
                pass_script.clock = loop_clock;
                run_session_loop_with_sink(
                    &mut alt_term,
                    &mut session,
                    &mut pass_script,
                    &mut sink,
                )?;
                loop_clock = pass_script.elapsed();
            }
            assert_eq!(
                session.phase(),
                league_core::TurnPhase::AwaitingMandateReveal
            );

            // 3. Drive all mandate reveals + 1 failed replay export via `run_session_loop_with_sink`.
            let replay_path = save_dir.join("club-1.replay.json");
            let seal_path = save_dir.join("club-1.seal");
            std::fs::create_dir_all(replay_path.join("blocker"))?;
            let mut reveal_and_fail_events = Vec::new();
            for _ in 0..session.seat_count() {
                reveal_and_fail_events.push((
                    Duration::ZERO,
                    Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                    Vec::new(),
                ));
            }
            reveal_and_fail_events.push((
                Duration::ZERO,
                Some(key_ev(KeyCode::Char('4'), KeyModifiers::NONE)),
                Vec::new(),
            ));
            reveal_and_fail_events.push((
                Duration::ZERO,
                Some(key_ev(KeyCode::Char('e'), KeyModifiers::NONE)),
                Vec::new(),
            ));
            let mut fail_script =
                ScriptedSessionEvents::from_timed_with_queued(reveal_and_fail_events);
            fail_script.clock = loop_clock;
            run_session_loop_with_sink(&mut alt_term, &mut session, &mut fail_script, &mut sink)?;
            assert_eq!(session.phase(), league_core::TurnPhase::Completed);
            assert!(seal_path.exists());
            assert_eq!(
                session
                    .terminal_tui_error()
                    .ok_or("failed export must set exit 3 error")?
                    .exit_code(),
                3
            );

            // 4. Unblock replay path and drive successful replay export + clean quit.
            std::fs::remove_dir_all(&replay_path)?;
            let mut export_ok_script = ScriptedSessionEvents::from_timed_with_queued(vec![
                (
                    Duration::ZERO,
                    Some(key_ev(KeyCode::Char('e'), KeyModifiers::NONE)),
                    Vec::new(),
                ),
                (
                    Duration::ZERO,
                    Some(key_ev(KeyCode::Char('q'), KeyModifiers::NONE)),
                    Vec::new(),
                ),
            ]);
            export_ok_script.clock = fail_script.elapsed();
            run_session_loop_with_sink(
                &mut alt_term,
                &mut session,
                &mut export_ok_script,
                &mut sink,
            )?;
            assert!(!seal_path.exists());
            assert!(replay_path.exists());
            assert!(session.terminal_tui_error().is_none());

            // Alternate screen received zero diagnostic lines; restore and inspect flushed output + log file.
            let alt_bytes = String::from_utf8_lossy(&written.borrow()).into_owned();
            assert!(
                !alt_bytes.contains("[normal]")
                    && !alt_bytes.contains("[verbose]")
                    && !alt_bytes.contains("[debug]")
            );
            let mut pre_restore = Vec::new();
            sink.flush_after_restore(&mut pre_restore);
            assert!(pre_restore.is_empty());

            sink.set_alternate_screen_active(false);
            let mut post_restore = Vec::new();
            sink.flush_after_restore(&mut post_restore);
            let flushed_stderr = String::from_utf8(post_restore)?;
            let file_log = std::fs::read_to_string(&log_file_path).unwrap_or_default();

            let mut log_secrets: Vec<&str> = vec![
                planted_pin,
                planted_note,
                planted_debt_num,
                "5a5a5a5a",
                "5A5A5A5A",
                "[90, 90, 90",
            ];
            for m in &all_text_markers {
                log_secrets.push(m.as_str());
            }
            for n in &mandate_number_markers {
                log_secrets.push(n.as_str());
            }
            for combined in [&flushed_stderr, &file_log] {
                for secret in &log_secrets {
                    assert!(
                        !combined.contains(secret),
                        "real session level `{}` wrote secret `{secret}` to log:\n{combined}",
                        level.as_str()
                    );
                }
            }
            if level == host::LogLevel::Silent {
                assert!(flushed_stderr.is_empty());
                assert!(file_log.is_empty());
            } else {
                assert!(flushed_stderr.contains("session start"));
            }
            if level >= host::LogLevel::Verbose {
                assert!(flushed_stderr.contains("lobby built table"));
                assert!(flushed_stderr.contains("phase=Completed"));
            }
            if level == host::LogLevel::Debug {
                assert!(flushed_stderr.contains("lobby validation refused [NO_HUMAN_SEATS]"));
                assert!(flushed_stderr.contains("host refused input action [UNKNOWN_OBJECT]"));
                assert!(
                    flushed_stderr.contains("host refused input action [TUI_REPLAY_EXPORT_FAILED]")
                );
                assert!(flushed_stderr.contains("queued terminal input drained"));
            }

            // Force a real panic carrying `{session:?}`, `{active_view:?}`, `{active_view.zones:?}`
            // plus XRPL seed & API key through `handle_panic_with_restore` at this `level`.
            let session_dbg = format!("{session:?}");
            let view_dbg = format!("{active_view:?}");
            let zones_dbg = format!("{:?}", active_view.zones);
            let panic_msg = format!(
                "forced boom {session_dbg} {view_dbg} {zones_dbg} seed={planted_xrpl} key={planted_api_key}"
            );
            let prev_hook = std::panic::take_hook();
            let ev_log = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
            let cap_err = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
            let ev_clone = std::sync::Arc::clone(&ev_log);
            let err_clone = std::sync::Arc::clone(&cap_err);
            std::panic::set_hook(Box::new(move |info| {
                let mut buf = Vec::new();
                let mut restore = || {
                    if let Ok(mut g) = ev_clone.lock() {
                        g.push("restore".to_owned());
                    }
                };
                handle_panic_with_restore(info, level, &mut restore, &mut buf);
                if let Ok(mut g) = ev_clone.lock() {
                    g.push("write".to_owned());
                }
                if let Ok(mut w) = err_clone.lock() {
                    w.extend_from_slice(&buf);
                }
            }));
            let res = std::panic::catch_unwind(move || {
                trigger_test_panic_line(panic_msg);
            });
            std::panic::set_hook(prev_hook);
            assert!(res.is_err());

            let order = ev_log
                .lock()
                .unwrap_or_else(|e| panic!("lock: {e}"))
                .clone();
            assert_eq!(order, vec!["restore".to_owned(), "write".to_owned()]);
            let panic_bytes = cap_err
                .lock()
                .unwrap_or_else(|e| panic!("lock: {e}"))
                .clone();
            let panic_str = String::from_utf8_lossy(&panic_bytes);
            if level == host::LogLevel::Debug {
                assert!(panic_str.contains("stack backtrace:"));
            } else {
                assert_eq!(panic_str.lines().count(), 1);
                assert!(!panic_str.contains("stack backtrace:"));
            }
            let panic_header = panic_str.lines().next().unwrap_or("");
            let mut panic_text_secrets: Vec<&str> = vec![
                planted_note,
                planted_debt_num,
                "5a5a5a5a",
                "5A5A5A5A",
                "[90, 90, 90",
                planted_xrpl,
                planted_api_key,
            ];
            for m in &all_text_markers {
                panic_text_secrets.push(m.as_str());
            }
            for secret in &panic_text_secrets {
                assert!(
                    !panic_str.contains(secret),
                    "panic hook level `{}` leaked `{secret}`:\n{panic_str}",
                    level.as_str()
                );
            }
            assert!(
                !panic_header.contains(planted_pin),
                "panic header line leaked PIN `{planted_pin}`:\n{panic_header}"
            );
            for num in &mandate_number_markers {
                assert!(
                    !panic_header.contains(num.as_str()),
                    "panic header line leaked mandate number `{num}`:\n{panic_header}"
                );
            }

            let _ = std::fs::remove_dir_all(&dir);
        }

        // Explicit 4-seat (1 Human + 3 ClubBots) LogLevel::Debug run with unique 5-digit
        // MinProjectEquity targets and distinct 4-digit bonuses (within 3000..=8000 and above
        // `main.rs` line count) proving every human and bot mandate target & bonus number stays
        // absent across Sweep & Debug panic.
        {
            let ring = default_dev_ring_table()?;
            let explicit_targets = [91_234, 92_345, 93_456, 94_567];
            let explicit_bonuses = [6_123, 6_789, 7_345, 7_987];
            let mut exp_session = host::TuiHostSession::from_explicit_mandates_for_test(
                &ring,
                TableId(81_704),
                &[0],
                explicit_targets,
                explicit_bonuses,
                19,
            )?;
            exp_session.enable_seat_pins();
            exp_session.open_initial_curtain_for_active_human();

            let (mut exp_text, mut exp_nums) = collect_mandate_markers(&exp_session)?;
            for s in 0..4_u8 {
                let salt_hex = exp_session.turn_reveal_salt_hex(league_core::Seat(s))?;
                exp_text.push(salt_hex[..16].to_owned());
                exp_text.push(salt_hex);
            }
            let vp = ViewportSize::new(100, 30);
            let mut exp_events: Vec<(Duration, Option<Event>, Vec<Event>)> = vec![
                (
                    host::CURTAIN_LIFT_DEAD_TIME,
                    Some(key_ev(KeyCode::Char(' '), KeyModifiers::NONE)),
                    Vec::new(),
                ),
                (
                    Duration::ZERO,
                    Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                    Vec::new(),
                ),
            ];
            for d in planted_pin.chars() {
                exp_events.push((
                    Duration::ZERO,
                    Some(key_ev(KeyCode::Char(d), KeyModifiers::NONE)),
                    Vec::new(),
                ));
            }
            exp_events.push((
                Duration::ZERO,
                Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                Vec::new(),
            ));
            exp_events.push((
                host::CURTAIN_LIFT_DEAD_TIME,
                Some(key_ev(KeyCode::Char('l'), KeyModifiers::CONTROL)),
                Vec::new(),
            ));
            exp_events.push((
                host::CURTAIN_LIFT_DEAD_TIME,
                Some(key_ev(KeyCode::Char(' '), KeyModifiers::NONE)),
                Vec::new(),
            ));
            exp_events.push((
                Duration::ZERO,
                Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                Vec::new(),
            ));
            for d in planted_pin.chars() {
                exp_events.push((
                    Duration::ZERO,
                    Some(key_ev(KeyCode::Char(d), KeyModifiers::NONE)),
                    Vec::new(),
                ));
            }
            exp_events.push((
                Duration::ZERO,
                Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                Vec::new(),
            ));
            exp_events.push((
                host::CURTAIN_LIFT_DEAD_TIME,
                Some(key_ev(KeyCode::Char('l'), KeyModifiers::CONTROL)),
                Vec::new(),
            ));
            let mut exp_term = Terminal::new(TestBackend::new(100, 30))?;
            let mut exp_sink = host::TuiDiagnosticSink::new(host::LogLevel::Debug, None);
            exp_sink.set_alternate_screen_active(true);
            let mut exp_script = ScriptedSessionEvents::from_timed_with_queued(exp_events);
            run_session_loop_with_sink(
                &mut exp_term,
                &mut exp_session,
                &mut exp_script,
                &mut exp_sink,
            )?;
            assert_eq!(exp_session.turn(), 2);

            exp_session.advance_clock(host::CURTAIN_LIFT_DEAD_TIME);
            exp_session
                .handle_key_event(s_key(' '), vp)
                .map_err(|e| e.to_string())?;
            exp_session
                .handle_key_event(
                    SeatKeyEvent::new(SeatKeyCode::Enter, SeatKeyModifiers::NONE),
                    vp,
                )
                .map_err(|e| e.to_string())?;
            for d in planted_pin.chars() {
                exp_session
                    .handle_key_event(s_key(d), vp)
                    .map_err(|e| e.to_string())?;
            }
            exp_session
                .handle_key_event(
                    SeatKeyEvent::new(SeatKeyCode::Enter, SeatKeyModifiers::NONE),
                    vp,
                )
                .map_err(|e| e.to_string())?;
            let exp_view = exp_session.build_current_view(vp)?;

            exp_sink.set_alternate_screen_active(false);
            let mut exp_out = Vec::new();
            exp_sink.flush_after_restore(&mut exp_out);
            let exp_log = String::from_utf8(exp_out)?;
            exp_nums.sort();
            exp_nums.dedup();
            for s in exp_text.iter().chain(exp_nums.iter()) {
                assert!(
                    !exp_log.contains(s.as_str()),
                    "explicit mandate Debug log leaked `{s}`:\n{exp_log}"
                );
            }

            let exp_panic_msg = format!(
                "explicit panic {:?} {:?} {:?}",
                exp_session, exp_view, exp_view.zones
            );
            let prev_hook = std::panic::take_hook();
            let cap_err = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
            let err_clone = std::sync::Arc::clone(&cap_err);
            std::panic::set_hook(Box::new(move |info| {
                let mut buf = Vec::new();
                let mut restore = || {};
                handle_panic_with_restore(info, host::LogLevel::Debug, &mut restore, &mut buf);
                if let Ok(mut w) = err_clone.lock() {
                    w.extend_from_slice(&buf);
                }
            }));
            let res = std::panic::catch_unwind(move || {
                trigger_test_panic_line(exp_panic_msg);
            });
            std::panic::set_hook(prev_hook);
            assert!(res.is_err());
            let panic_str =
                String::from_utf8_lossy(&cap_err.lock().unwrap_or_else(|e| panic!("lock: {e}")))
                    .into_owned();
            for s in exp_text.iter().chain(exp_nums.iter()) {
                assert!(
                    !panic_str.contains(s.as_str()),
                    "explicit Debug panic leaked `{s}`:\n{panic_str}"
                );
            }
        }

        // CLI log flag overrides conflicting `LEAGUE_LOG_LEVEL` env var.
        let env_tokens: Vec<String> =
            vec!["league-tui".to_owned(), "--log-level=silent".to_owned()];
        assert_eq!(
            infer_log_level_best_effort(&env_tokens, |_| Some("debug".to_owned())),
            host::LogLevel::Silent
        );
        assert_eq!(
            infer_log_level_best_effort(&["league-tui".to_owned()], |_| Some("verbose".to_owned())),
            host::LogLevel::Verbose
        );

        Ok(())
    }

    #[test]
    fn panic_hook_restores_terminal_prints_one_redacted_line_and_gates_backtrace_on_debug() {
        let planted_payload = "boom\nseed=sEdSKaCy2JT7JaM7v95H9SxkhP9wS2r key=sk-or-v1-panicKey88 \"seat_pins\":[\"4829\"] \"preimage_secret\":\"hiddenSaltVal\"";

        // Test 1: real std::panic::catch_unwind with a forced panic invokes terminal restore
        // strictly BEFORE writing to stderr, emits one single line under Normal, and scrubs keys/seeds.
        let prev_hook = std::panic::take_hook();
        let events_log = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let captured_err = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
        let ev_clone = std::sync::Arc::clone(&events_log);
        let err_clone = std::sync::Arc::clone(&captured_err);
        std::panic::set_hook(Box::new(move |info| {
            let mut buf = Vec::new();
            let mut restore = || {
                if let Ok(mut g) = ev_clone.lock() {
                    g.push("restore".to_owned());
                }
            };
            handle_panic_with_restore(info, host::LogLevel::Normal, &mut restore, &mut buf);
            if let Ok(mut g) = ev_clone.lock() {
                g.push("write".to_owned());
            }
            if let Ok(mut w) = err_clone.lock() {
                w.extend_from_slice(&buf);
            }
        }));
        let res = std::panic::catch_unwind(move || {
            panic!("{planted_payload}");
        });
        std::panic::set_hook(prev_hook);
        assert!(res.is_err());

        let order = events_log
            .lock()
            .unwrap_or_else(|e| panic!("lock: {e}"))
            .clone();
        assert_eq!(order, vec!["restore".to_owned(), "write".to_owned()]);
        let normal_bytes = captured_err
            .lock()
            .unwrap_or_else(|e| panic!("lock: {e}"))
            .clone();
        let normal_str = String::from_utf8_lossy(&normal_bytes);
        assert_eq!(
            normal_str.lines().count(),
            1,
            "Normal panic must print 1 line:\n{normal_str}"
        );
        assert!(!normal_str.contains("stack backtrace:"));
        for secret in [
            "4829",
            "hiddenSaltVal",
            "sEdSKaCy2JT7JaM7v95H9SxkhP9wS2r",
            "sk-or-v1-panicKey88",
        ] {
            assert!(
                !normal_str.contains(secret),
                "panic line leaked `{secret}`:\n{normal_str}"
            );
        }

        // Test 2: Debug level adds scrubbed stack backtrace while Normal/Silent/Verbose omit it.
        let fake_bt = "  0: my_fn\n  \"seat_notes\":[\"btSecretNote\"] sk-or-v1-btKey11";
        let no_bt = format_panic_output(
            planted_payload,
            Some("main.rs:10:1"),
            host::LogLevel::Verbose,
            Some(fake_bt),
        );
        assert_eq!(no_bt.lines().count(), 1);
        assert!(!no_bt.contains("stack backtrace:"));

        let with_bt = format_panic_output(
            planted_payload,
            Some("main.rs:10:1"),
            host::LogLevel::Debug,
            Some(fake_bt),
        );
        assert!(with_bt.contains("stack backtrace:"));
        assert!(!with_bt.contains("btSecretNote"));
        assert!(!with_bt.contains("sk-or-v1-btKey11"));
        assert!(with_bt.contains("[REDACTED]"));
    }

    #[test]
    fn lobby_never_overwrites_unfinished_season_and_allocates_next_table_id_after_export()
    -> Result<(), Box<dyn std::error::Error>> {
        use crossterm::event::KeyEventKind;
        use league_seat_types::{SeatKeyCode, SeatKeyEvent, SeatKeyModifiers};

        let key_ev = |code: KeyCode, mods: KeyModifiers| {
            let mut k = KeyEvent::new(code, mods);
            k.kind = KeyEventKind::Press;
            Event::Key(k)
        };
        let dir = std::env::temp_dir().join(format!("ml_lobby_overwrite_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        let dir_str = dir.to_string_lossy().into_owned();

        // Parse real CLI flags `--untimed --save-dir <dir>` to start Season 1 from Lobby.
        let opts = match parse_args([
            "league-tui".to_owned(),
            "--untimed".to_owned(),
            "--save-dir".to_owned(),
            dir_str.clone(),
        ]) {
            CliAction::RunClub(o) => o,
            other => return Err(format!("expected RunClub, got {other:?}").into()),
        };

        let lobby_start_steps = || {
            let mut v = vec![key_ev(KeyCode::Enter, KeyModifiers::NONE)];
            for _ in 0..8 {
                v.insert(0, key_ev(KeyCode::Down, KeyModifiers::NONE));
            }
            v
        };
        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
        let mut sink = host::TuiDiagnosticSink::new(host::LogLevel::Normal, None);
        sink.set_alternate_screen_active(true);

        // Season 1 starts as `club-1.save.json` + `club-1.seal`.
        let mut ev1 = ScriptedSessionEvents::from_events(lobby_start_steps());
        let mut s1 =
            run_lobby_loop_with_sink(&mut terminal, &opts, &mut ev1, Some(&dir), &mut sink)?
                .ok_or("Season 1 must start")?;
        assert_eq!(s1.table_id(), TableId(1));
        let s1_save_path = dir.join("club-1.save.json");
        let s1_seal_path = dir.join("club-1.seal");
        let s1_replay_path = dir.join("club-1.replay.json");
        let s1_save_before = std::fs::read(&s1_save_path)?;
        let s1_seal_before = std::fs::read(&s1_seal_path)?;

        // Attempt 2 while Season 1 is unfinished -> refused with `UNFINISHED_SEASON_EXISTS`,
        // showing banner on Lobby and leaving `club-1.*` byte-identical.
        let mut refuse_steps = lobby_start_steps();
        refuse_steps.push(key_ev(KeyCode::Char('q'), KeyModifiers::NONE));
        let mut ev2 = ScriptedSessionEvents::from_events(refuse_steps.clone());
        let s2_attempt =
            run_lobby_loop_with_sink(&mut terminal, &opts, &mut ev2, Some(&dir), &mut sink)?;
        assert!(s2_attempt.is_none());
        let refuse_screen = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(refuse_screen.contains("REFUSED [UNFINISHED_SEASON_EXISTS]:"));
        assert!(!dir.join("club-2.save.json").exists());
        assert_eq!(std::fs::read(&s1_save_path)?, s1_save_before);
        assert_eq!(std::fs::read(&s1_seal_path)?, s1_seal_before);

        // Drive Season 1 to `Completed` without exporting yet -> `club-1.seal` still present, so a
        // new Lobby start MUST still refuse and preserve `club-1.*` byte-identical.
        let vp = ViewportSize::new(100, 30);
        let s_key = |c: char| SeatKeyEvent::new(SeatKeyCode::Char(c), SeatKeyModifiers::NONE);
        while s1.phase() == league_core::TurnPhase::TalkAndMarket {
            if s1.is_curtain_open() {
                s1.advance_clock(host::CURTAIN_LIFT_DEAD_TIME);
                s1.handle_key_event(s_key(' '), vp)
                    .map_err(|e| e.to_string())?;
                s1.handle_key_event(
                    SeatKeyEvent::new(SeatKeyCode::Enter, SeatKeyModifiers::NONE),
                    vp,
                )
                .map_err(|e| e.to_string())?;
                s1.advance_clock(host::CURTAIN_LIFT_DEAD_TIME);
            }
            s1.handle_key_event(
                SeatKeyEvent::new(SeatKeyCode::Char('l'), SeatKeyModifiers::CTRL),
                vp,
            )
            .map_err(|e| e.to_string())?;
        }
        for _ in 0..s1.seat_count() {
            s1.handle_key_event(
                SeatKeyEvent::new(SeatKeyCode::Enter, SeatKeyModifiers::NONE),
                vp,
            )
            .map_err(|e| e.to_string())?;
        }
        assert_eq!(s1.phase(), league_core::TurnPhase::Completed);
        let s1_save_completed = std::fs::read(&s1_save_path)?;
        let s1_seal_completed = std::fs::read(&s1_seal_path)?;

        let mut ev3 = ScriptedSessionEvents::from_events(refuse_steps);
        assert!(
            run_lobby_loop_with_sink(&mut terminal, &opts, &mut ev3, Some(&dir), &mut sink)?
                .is_none()
        );
        assert_eq!(std::fs::read(&s1_save_path)?, s1_save_completed);
        assert_eq!(std::fs::read(&s1_seal_path)?, s1_seal_completed);

        // Export Season 1's replay -> `club-1.seal` is wiped (`club-1.replay.json` written).
        s1.handle_key_event(s_key('4'), vp)
            .map_err(|e| e.to_string())?;
        s1.handle_key_event(s_key('e'), vp)
            .map_err(|e| e.to_string())?;
        assert!(!s1_seal_path.exists());
        assert!(s1_replay_path.exists());
        let s1_replay_bytes = std::fs::read(&s1_replay_path)?;

        // Season 2 starts cleanly in the same `--save-dir` as `TableId(2)` without touching Season 1!
        let mut ev4 = ScriptedSessionEvents::from_events(lobby_start_steps());
        let s2 = run_lobby_loop_with_sink(&mut terminal, &opts, &mut ev4, Some(&dir), &mut sink)?
            .ok_or("Season 2 must start once Season 1 is exported")?;
        assert_eq!(s2.table_id(), TableId(2));
        assert!(dir.join("club-2.save.json").exists());
        assert!(dir.join("club-2.seal").exists());
        assert_eq!(std::fs::read(&s1_save_path)?, s1_save_completed);
        assert_eq!(std::fs::read(&s1_replay_path)?, s1_replay_bytes);

        // Resuming via `--resume <dir>` selects active `club-2` behind Seat 0's Curtain.
        let resume_opts =
            match parse_args(["league-tui".to_owned(), "--resume".to_owned(), dir_str]) {
                CliAction::RunClub(o) => o,
                other => return Err(format!("expected RunClub resume, got {other:?}").into()),
            };
        let resumed_s2 = build_session_for_cli(&resume_opts)?;
        assert_eq!(resumed_s2.table_id(), TableId(2));
        assert!(resumed_s2.is_curtain_open());

        let _ = league_core::SeasonReplayLog::from_club_json_slice(&s1_replay_bytes)?;
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn lobby_cli_flags_match_equivalent_lobby_keys_through_real_parser()
    -> Result<(), Box<dyn std::error::Error>> {
        use crossterm::event::KeyEventKind;

        let key_ev = |code: KeyCode, mods: KeyModifiers| {
            let mut k = KeyEvent::new(code, mods);
            k.kind = KeyEventKind::Press;
            Event::Key(k)
        };
        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;

        let cli_opts = match parse_args([
            "league-tui".to_owned(),
            "--turn-clock".to_owned(),
            "240".to_owned(),
            "--pins".to_owned(),
        ]) {
            CliAction::RunClub(o) => o,
            other => return Err(format!("expected RunClub, got {other:?}").into()),
        };
        let mut cli_lobby_steps = Vec::new();
        for _ in 0..8 {
            cli_lobby_steps.push(key_ev(KeyCode::Down, KeyModifiers::NONE));
        }
        cli_lobby_steps.push(key_ev(KeyCode::Enter, KeyModifiers::NONE));
        let mut ev_cli = ScriptedSessionEvents::from_events(cli_lobby_steps);
        let sess_cli = run_lobby_loop(&mut terminal, &cli_opts, &mut ev_cli, None)?
            .ok_or("CLI flags Lobby start must succeed")?;

        let default_opts = match parse_args(["league-tui".to_owned()]) {
            CliAction::RunClub(o) => o,
            other => return Err(format!("expected RunClub default, got {other:?}").into()),
        };
        let mut ui_steps = Vec::new();
        for _ in 0..6 {
            ui_steps.push(key_ev(KeyCode::Down, KeyModifiers::NONE));
        }
        ui_steps.push(key_ev(KeyCode::Right, KeyModifiers::NONE));
        ui_steps.push(key_ev(KeyCode::Right, KeyModifiers::NONE));
        ui_steps.push(key_ev(KeyCode::Down, KeyModifiers::NONE));
        ui_steps.push(key_ev(KeyCode::Char(' '), KeyModifiers::NONE));
        ui_steps.push(key_ev(KeyCode::Down, KeyModifiers::NONE));
        ui_steps.push(key_ev(KeyCode::Enter, KeyModifiers::NONE));
        let mut ev_ui = ScriptedSessionEvents::from_events(ui_steps);
        let sess_ui = run_lobby_loop(&mut terminal, &default_opts, &mut ev_ui, None)?
            .ok_or("Lobby UI equivalent start must succeed")?;

        let mut spec_cli = sess_cli.public_spec();
        let mut spec_ui = sess_ui.public_spec();
        spec_cli.eval_vrf_pubkey = None;
        spec_ui.eval_vrf_pubkey = None;
        assert_eq!(spec_cli, spec_ui);
        assert_eq!(sess_cli.pass_budget_secs(), Some(180));
        assert_eq!(sess_ui.pass_budget_secs(), Some(180));
        assert!(sess_cli.pins_enabled() && sess_ui.pins_enabled());
        assert!(sess_cli.is_curtain_open() && sess_ui.is_curtain_open());
        assert_eq!(sess_cli.observe_seat_call_count(), 0);
        assert_eq!(sess_ui.observe_seat_call_count(), 0);
        Ok(())
    }

    #[test]
    fn completed_season_failed_export_resumes_via_cli_and_wipes_seal_only_after_retry_succeeds()
    -> Result<(), Box<dyn std::error::Error>> {
        use league_seat_types::{SeatKeyCode, SeatKeyEvent, SeatKeyModifiers};

        let dir = std::env::temp_dir().join(format!("ml_eos_resume_retry_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;

        let ring = default_dev_ring_table()?;
        let save_path = dir.join("club-81710.save.json");
        let seal_path = dir.join("club-81710.seal");
        let replay_path = dir.join("club-81710.replay.json");
        let vp = ViewportSize::new(100, 30);
        let s_key = |c: char| SeatKeyEvent::new(SeatKeyCode::Char(c), SeatKeyModifiers::NONE);

        {
            let mut session =
                host::TuiHostSession::from_ring_table(&ring, TableId(81_710), &[0], 61)?;
            session.enable_autosave(save_path.clone(), seal_path.clone());
            while session.phase() == league_core::TurnPhase::TalkAndMarket {
                if session.is_curtain_open() {
                    session.advance_clock(host::CURTAIN_LIFT_DEAD_TIME);
                    session
                        .handle_key_event(s_key(' '), vp)
                        .map_err(|e| e.to_string())?;
                    session
                        .handle_key_event(
                            SeatKeyEvent::new(SeatKeyCode::Enter, SeatKeyModifiers::NONE),
                            vp,
                        )
                        .map_err(|e| e.to_string())?;
                    session.advance_clock(host::CURTAIN_LIFT_DEAD_TIME);
                }
                session
                    .handle_key_event(
                        SeatKeyEvent::new(SeatKeyCode::Char('l'), SeatKeyModifiers::CTRL),
                        vp,
                    )
                    .map_err(|e| e.to_string())?;
            }
            for _ in 0..session.seat_count() {
                session
                    .handle_key_event(
                        SeatKeyEvent::new(SeatKeyCode::Enter, SeatKeyModifiers::NONE),
                        vp,
                    )
                    .map_err(|e| e.to_string())?;
            }
            assert_eq!(session.phase(), league_core::TurnPhase::Completed);

            // Force ExportReplay failure by blocking `club-81710.replay.json` with a directory.
            std::fs::create_dir_all(replay_path.join("blocker"))?;
            session
                .handle_key_event(s_key('4'), vp)
                .map_err(|e| e.to_string())?;
            let err = session
                .handle_key_event(s_key('e'), vp)
                .err()
                .ok_or("blocked export must fail")?;
            assert_eq!(err.code, "TUI_REPLAY_EXPORT_FAILED");
            let tui_err = session
                .terminal_tui_error()
                .ok_or("failed export must set terminal_tui_error")?;
            assert_eq!(tui_err.exit_code(), 3);
            assert!(
                seal_path.exists(),
                ".seal must stay on disk after failed export"
            );
        }

        // Remove blocker, resume via real `--resume` CLI path, verify End-of-Season restores,
        // export succeeds cleanly, and `.seal` is wiped only then.
        std::fs::remove_dir_all(&replay_path)?;
        assert!(seal_path.exists());
        let resume_opts = match parse_args([
            "league-tui".to_owned(),
            "--resume".to_owned(),
            save_path.to_string_lossy().into_owned(),
        ]) {
            CliAction::RunClub(o) => o,
            other => return Err(format!("expected RunClub resume, got {other:?}").into()),
        };
        let mut resumed = build_session_for_cli(&resume_opts)?;
        assert_eq!(resumed.phase(), league_core::TurnPhase::Completed);
        assert!(!resumed.is_curtain_open());
        assert_eq!(resumed.observe_seat_call_count(), 0);
        assert!(
            seal_path.exists(),
            ".seal must remain until retry export runs"
        );

        resumed
            .handle_key_event(s_key('4'), vp)
            .map_err(|e| e.to_string())?;
        resumed
            .handle_key_event(s_key('e'), vp)
            .map_err(|e| e.to_string())?;
        assert!(
            !seal_path.exists(),
            ".seal must be wiped once retry export succeeds"
        );
        assert!(resumed.terminal_tui_error().is_none());
        let parsed =
            league_core::SeasonReplayLog::from_club_json_slice(&std::fs::read(&replay_path)?)?;
        assert!(parsed.turns.len() >= 24);

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn resume_during_awaiting_mandate_reveal_shows_no_mandates_on_first_frame()
    -> Result<(), Box<dyn std::error::Error>> {
        use league_seat_types::{SeatKeyCode, SeatKeyEvent, SeatKeyModifiers};

        let dir = std::env::temp_dir().join(format!("ml_standby_resume_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;

        let ring = default_dev_ring_table()?;
        let save_path = dir.join("club-81720.save.json");
        let seal_path = dir.join("club-81720.seal");
        let vp = ViewportSize::new(100, 30);
        let s_key = |c: char| SeatKeyEvent::new(SeatKeyCode::Char(c), SeatKeyModifiers::NONE);

        let mut all_markers = Vec::new();
        {
            let mut session =
                host::TuiHostSession::from_ring_table(&ring, TableId(81_720), &[0], 63)?;
            for s in 0..session.seat_count() {
                let m = *session
                    .build_seat_observation(league_core::Seat(s))?
                    .own_mandate()
                    .ok_or("mandate")?;
                all_markers.push(host::mandate_short_marker(m.kind));
            }
            session.enable_autosave(save_path.clone(), seal_path.clone());
            while session.phase() == league_core::TurnPhase::TalkAndMarket {
                if session.is_curtain_open() {
                    session.advance_clock(host::CURTAIN_LIFT_DEAD_TIME);
                    session
                        .handle_key_event(s_key(' '), vp)
                        .map_err(|e| e.to_string())?;
                    session
                        .handle_key_event(
                            SeatKeyEvent::new(SeatKeyCode::Enter, SeatKeyModifiers::NONE),
                            vp,
                        )
                        .map_err(|e| e.to_string())?;
                    session.advance_clock(host::CURTAIN_LIFT_DEAD_TIME);
                }
                session
                    .handle_key_event(
                        SeatKeyEvent::new(SeatKeyCode::Char('l'), SeatKeyModifiers::CTRL),
                        vp,
                    )
                    .map_err(|e| e.to_string())?;
            }
            assert_eq!(
                session.phase(),
                league_core::TurnPhase::AwaitingMandateReveal
            );
            // Submit 2 of 4 reveals and explicitly persist checkpoint mid-reveal.
            for _ in 0..2 {
                session
                    .handle_key_event(
                        SeatKeyEvent::new(SeatKeyCode::Enter, SeatKeyModifiers::NONE),
                        vp,
                    )
                    .map_err(|e| e.to_string())?;
            }
            session
                .write_save_and_seal(&save_path, &seal_path)
                .map_err(|e| e.to_string())?;
        }

        // Resume during `AwaitingMandateReveal` via `--resume` CLI path.
        let resume_opts = match parse_args([
            "league-tui".to_owned(),
            "--resume".to_owned(),
            save_path.to_string_lossy().into_owned(),
        ]) {
            CliAction::RunClub(o) => o,
            other => return Err(format!("expected RunClub resume, got {other:?}").into()),
        };
        let resumed = build_session_for_cli(&resume_opts)?;
        assert_eq!(
            resumed.phase(),
            league_core::TurnPhase::AwaitingMandateReveal
        );
        assert!(!resumed.is_curtain_open());
        assert_eq!(resumed.observe_seat_call_count(), 0);

        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
        terminal.draw(|f| {
            resumed
                .render_current_frame(f)
                .unwrap_or_else(|e| panic!("render resumed standby: {e}"));
        })?;
        let first_frame: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert_eq!(resumed.observe_seat_call_count(), 0);
        assert!(first_frame.contains("Session Standby"));
        assert!(first_frame.contains("AwaitingMandateReveal"));
        assert!(first_frame.contains("2/4"));
        assert!(!first_frame.contains("Private Vault"));
        for marker in &all_markers {
            assert!(
                !first_frame.contains(marker),
                "first resumed standby frame leaked mandate `{marker}`:\n{first_frame}"
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn resume_with_save_dir_never_overwrites_other_season_supports_in_place_and_empty_dir()
    -> Result<(), Box<dyn std::error::Error>> {
        use crossterm::event::KeyEventKind;
        use league_seat_types::{SeatKeyCode, SeatKeyEvent, SeatKeyModifiers};

        let key_ev = |code: KeyCode, mods: KeyModifiers| {
            let mut k = KeyEvent::new(code, mods);
            k.kind = KeyEventKind::Press;
            Event::Key(k)
        };
        let s_key = |c: char| SeatKeyEvent::new(SeatKeyCode::Char(c), SeatKeyModifiers::NONE);
        let vp = ViewportSize::new(100, 30);

        let dir = std::env::temp_dir().join(format!("ml_resume_save_dir_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let dir_a = dir.join("season_a");
        let dir_b_unfin = dir.join("season_b_unfinished");
        let dir_b_fin = dir.join("season_b_finished");
        let dir_c_empty = dir.join("season_c_empty");
        std::fs::create_dir_all(&dir_a)?;
        std::fs::create_dir_all(&dir_b_unfin)?;
        std::fs::create_dir_all(&dir_b_fin)?;
        std::fs::create_dir_all(&dir_c_empty)?;

        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
        let lobby_start_steps = || {
            let mut v = vec![key_ev(KeyCode::Enter, KeyModifiers::NONE)];
            for _ in 0..8 {
                v.insert(0, key_ev(KeyCode::Down, KeyModifiers::NONE));
            }
            v
        };

        // Start two independent TableId(1) seasons in <dir_a> and <dir_b_unfin> via Lobby.
        let start_table_1_in =
            |target_dir: &Path,
             seed_step: Option<u8>,
             term: &mut Terminal<TestBackend>|
             -> Result<host::TuiHostSession, Box<dyn std::error::Error>> {
                let opts = match parse_args([
                    "league-tui".to_owned(),
                    "--untimed".to_owned(),
                    "--save-dir".to_owned(),
                    target_dir.to_string_lossy().into_owned(),
                ]) {
                    CliAction::RunClub(o) => o,
                    other => return Err(format!("expected RunClub, got {other:?}").into()),
                };
                let mut steps = lobby_start_steps();
                if seed_step.is_some() {
                    // Step into Layout Seed row (row 5) and append digit '2' so Season A != Season B.
                    let mut seeded = Vec::new();
                    for _ in 0..5 {
                        seeded.push(key_ev(KeyCode::Down, KeyModifiers::NONE));
                    }
                    seeded.push(key_ev(KeyCode::Char('2'), KeyModifiers::NONE));
                    for _ in 0..3 {
                        seeded.push(key_ev(KeyCode::Down, KeyModifiers::NONE));
                    }
                    seeded.push(key_ev(KeyCode::Enter, KeyModifiers::NONE));
                    steps = seeded;
                }
                let mut sink = host::TuiDiagnosticSink::new(host::LogLevel::Silent, None);
                sink.set_alternate_screen_active(true);
                let mut ev = ScriptedSessionEvents::from_events(steps);
                Ok(
                    run_lobby_loop_with_sink(term, &opts, &mut ev, Some(target_dir), &mut sink)?
                        .ok_or("lobby table start")?,
                )
            };

        let sess_a = start_table_1_in(&dir_a, None, &mut terminal)?;
        let mut sess_b_unfin = start_table_1_in(&dir_b_unfin, Some(2), &mut terminal)?;
        assert_eq!(sess_a.table_id(), TableId(1));
        assert_eq!(sess_b_unfin.table_id(), TableId(1));
        drop(sess_a);

        let a_save = dir_a.join("club-1.save.json");
        let a_seal = dir_a.join("club-1.seal");
        let b_save = dir_b_unfin.join("club-1.save.json");
        let b_seal = dir_b_unfin.join("club-1.seal");

        // Advance Season B through its Curtain and lock Sub-Pass A (`Ctrl+L`) so B autosaves a
        // later Curtain cursor on disk before snapshot capture, and confirm A != B on disk.
        sess_b_unfin.advance_clock(host::CURTAIN_LIFT_DEAD_TIME);
        sess_b_unfin
            .handle_key_event(s_key(' '), vp)
            .map_err(|e| e.to_string())?;
        sess_b_unfin
            .handle_key_event(
                SeatKeyEvent::new(SeatKeyCode::Enter, SeatKeyModifiers::NONE),
                vp,
            )
            .map_err(|e| e.to_string())?;
        sess_b_unfin.advance_clock(host::CURTAIN_LIFT_DEAD_TIME);
        sess_b_unfin
            .handle_key_event(
                SeatKeyEvent::new(SeatKeyCode::Char('l'), SeatKeyModifiers::CTRL),
                vp,
            )
            .map_err(|e| e.to_string())?;
        drop(sess_b_unfin);
        let b_save_bytes = std::fs::read(&b_save)?;
        let b_seal_bytes = std::fs::read(&b_seal)?;
        assert_ne!(b_save_bytes, std::fs::read(&a_save)?);
        assert_ne!(b_seal_bytes, std::fs::read(&a_seal)?);

        // 1. `--resume <A>/club-1.save.json --save-dir <B_unfin>` -> refused with
        // `UNFINISHED_SEASON_EXISTS` (exit 1), leaving `<B_unfin>/club-1.*` byte-identical.
        let mut out = Vec::new();
        let mut err = Vec::new();
        let conflict_exit = dispatch_cli_with_env(
            [
                "league-tui".to_owned(),
                "--resume".to_owned(),
                a_save.to_string_lossy().into_owned(),
                "--save-dir".to_owned(),
                dir_b_unfin.to_string_lossy().into_owned(),
                "--error-format".to_owned(),
                "json".to_owned(),
            ],
            |_| None,
            true,
            &mut out,
            &mut err,
            |opts, _, _| build_session_for_cli(opts).map(|_| ()),
        );
        assert_eq!(conflict_exit, ExitCode::from(1));
        let conflict_json = String::from_utf8(err)?;
        assert!(conflict_json.contains("\"code\":\"UNFINISHED_SEASON_EXISTS\""));
        assert_eq!(std::fs::read(&b_save)?, b_save_bytes);
        assert_eq!(std::fs::read(&b_seal)?, b_seal_bytes);

        // 2. Finish & export a `club-1` season in `<dir_b_fin>` -> `--resume <A>/club-1.save.json
        // --save-dir <dir_b_fin>` AND direct `validate_and_start(TableId(1), ...)` (C1) must refuse
        // with `SEASON_SAVE_EXISTS` and leave `<dir_b_fin>` files byte-identical.
        let mut sess_b_fin = start_table_1_in(&dir_b_fin, Some(2), &mut terminal)?;
        while sess_b_fin.phase() == league_core::TurnPhase::TalkAndMarket {
            if sess_b_fin.is_curtain_open() {
                sess_b_fin.advance_clock(host::CURTAIN_LIFT_DEAD_TIME);
                sess_b_fin
                    .handle_key_event(s_key(' '), vp)
                    .map_err(|e| e.to_string())?;
                sess_b_fin
                    .handle_key_event(
                        SeatKeyEvent::new(SeatKeyCode::Enter, SeatKeyModifiers::NONE),
                        vp,
                    )
                    .map_err(|e| e.to_string())?;
                sess_b_fin.advance_clock(host::CURTAIN_LIFT_DEAD_TIME);
            }
            sess_b_fin
                .handle_key_event(
                    SeatKeyEvent::new(SeatKeyCode::Char('l'), SeatKeyModifiers::CTRL),
                    vp,
                )
                .map_err(|e| e.to_string())?;
        }
        for _ in 0..sess_b_fin.seat_count() {
            sess_b_fin
                .handle_key_event(
                    SeatKeyEvent::new(SeatKeyCode::Enter, SeatKeyModifiers::NONE),
                    vp,
                )
                .map_err(|e| e.to_string())?;
        }
        sess_b_fin
            .handle_key_event(s_key('4'), vp)
            .map_err(|e| e.to_string())?;
        sess_b_fin
            .handle_key_event(s_key('e'), vp)
            .map_err(|e| e.to_string())?;
        drop(sess_b_fin);
        let b_fin_save = dir_b_fin.join("club-1.save.json");
        let b_fin_replay = dir_b_fin.join("club-1.replay.json");
        assert!(!dir_b_fin.join("club-1.seal").exists());
        let b_fin_save_bytes = std::fs::read(&b_fin_save)?;
        let b_fin_replay_bytes = std::fs::read(&b_fin_replay)?;

        let mut err_fin = Vec::new();
        let fin_exit = dispatch_cli_with_env(
            [
                "league-tui".to_owned(),
                "--resume".to_owned(),
                a_save.to_string_lossy().into_owned(),
                "--save-dir".to_owned(),
                dir_b_fin.to_string_lossy().into_owned(),
            ],
            |_| None,
            true,
            &mut Vec::new(),
            &mut err_fin,
            |opts, _, _| build_session_for_cli(opts).map(|_| ()),
        );
        assert_eq!(fin_exit, ExitCode::from(1));
        assert!(String::from_utf8(err_fin)?.contains("SEASON_SAVE_EXISTS"));
        assert_eq!(std::fs::read(&b_fin_save)?, b_fin_save_bytes);
        assert_eq!(std::fs::read(&b_fin_replay)?, b_fin_replay_bytes);

        let mut direct_lobby = host::LobbyHostSession::new(LobbyCliOverrides::default());
        let c1_err = direct_lobby
            .validate_and_start(TableId(1), 1, Some(&dir_b_fin))
            .err()
            .ok_or("direct validate_and_start on existing save must refuse")?;
        assert_eq!(c1_err.code, "SEASON_SAVE_EXISTS");
        assert_eq!(std::fs::read(&b_fin_save)?, b_fin_save_bytes);
        assert_eq!(std::fs::read(&b_fin_replay)?, b_fin_replay_bytes);

        // 3. Resume in place with explicit `--save-dir <dir_a>/.` and with no `--save-dir` keep
        // working and autosave into `<dir_a>` cleanly.
        let a_save_before_inplace = std::fs::read(&a_save)?;
        let a_seal_before_inplace = std::fs::read(&a_seal)?;
        let in_place_opts = match parse_args([
            "league-tui".to_owned(),
            "--resume".to_owned(),
            a_save.to_string_lossy().into_owned(),
            "--save-dir".to_owned(),
            dir_a.join(".").to_string_lossy().into_owned(),
        ]) {
            CliAction::RunClub(o) => o,
            other => return Err(format!("expected RunClub in-place, got {other:?}").into()),
        };
        let mut resumed_inplace = build_session_for_cli(&in_place_opts)?;
        resumed_inplace.advance_clock(host::CURTAIN_LIFT_DEAD_TIME);
        resumed_inplace
            .handle_key_event(s_key(' '), vp)
            .map_err(|e| e.to_string())?;
        resumed_inplace
            .handle_key_event(
                SeatKeyEvent::new(SeatKeyCode::Enter, SeatKeyModifiers::NONE),
                vp,
            )
            .map_err(|e| e.to_string())?;
        resumed_inplace.advance_clock(host::CURTAIN_LIFT_DEAD_TIME);
        resumed_inplace
            .handle_key_event(
                SeatKeyEvent::new(SeatKeyCode::Char('l'), SeatKeyModifiers::CTRL),
                vp,
            )
            .map_err(|e| e.to_string())?;
        assert_ne!(std::fs::read(&a_seal)?, a_seal_before_inplace);
        assert_ne!(std::fs::read(&a_save)?, a_save_before_inplace);

        let no_savedir_opts = match parse_args([
            "league-tui".to_owned(),
            "--resume".to_owned(),
            a_save.to_string_lossy().into_owned(),
        ]) {
            CliAction::RunClub(o) => o,
            other => return Err(format!("expected RunClub no savedir, got {other:?}").into()),
        };
        let mut resumed_no_dir = build_session_for_cli(&no_savedir_opts)?;
        let a_seal_mid = std::fs::read(&a_seal)?;
        resumed_no_dir.advance_clock(host::CURTAIN_LIFT_DEAD_TIME);
        resumed_no_dir
            .handle_key_event(s_key(' '), vp)
            .map_err(|e| e.to_string())?;
        resumed_no_dir
            .handle_key_event(
                SeatKeyEvent::new(SeatKeyCode::Enter, SeatKeyModifiers::NONE),
                vp,
            )
            .map_err(|e| e.to_string())?;
        resumed_no_dir.advance_clock(host::CURTAIN_LIFT_DEAD_TIME);
        resumed_no_dir
            .handle_key_event(
                SeatKeyEvent::new(SeatKeyCode::Char('l'), SeatKeyModifiers::CTRL),
                vp,
            )
            .map_err(|e| e.to_string())?;
        assert_ne!(std::fs::read(&a_seal)?, a_seal_mid);

        // 4. `--resume <A>/club-1.save.json --save-dir <empty_C>` succeeds, autosaves into <empty_C>,
        // and leaves <A> untouched after resume.
        let a_save_snapshot = std::fs::read(&a_save)?;
        let a_seal_snapshot = std::fs::read(&a_seal)?;
        let empty_opts = match parse_args([
            "league-tui".to_owned(),
            "--resume".to_owned(),
            a_save.to_string_lossy().into_owned(),
            "--save-dir".to_owned(),
            dir_c_empty.to_string_lossy().into_owned(),
        ]) {
            CliAction::RunClub(o) => o,
            other => return Err(format!("expected RunClub empty target, got {other:?}").into()),
        };
        let mut resumed_c = build_session_for_cli(&empty_opts)?;
        resumed_c.advance_clock(host::CURTAIN_LIFT_DEAD_TIME);
        resumed_c
            .handle_key_event(s_key(' '), vp)
            .map_err(|e| e.to_string())?;
        resumed_c
            .handle_key_event(
                SeatKeyEvent::new(SeatKeyCode::Enter, SeatKeyModifiers::NONE),
                vp,
            )
            .map_err(|e| e.to_string())?;
        resumed_c.advance_clock(host::CURTAIN_LIFT_DEAD_TIME);
        resumed_c
            .handle_key_event(
                SeatKeyEvent::new(SeatKeyCode::Char('l'), SeatKeyModifiers::CTRL),
                vp,
            )
            .map_err(|e| e.to_string())?;
        assert!(dir_c_empty.join("club-1.save.json").exists());
        assert!(dir_c_empty.join("club-1.seal").exists());
        assert_eq!(std::fs::read(&a_save)?, a_save_snapshot);
        assert_eq!(std::fs::read(&a_seal)?, a_seal_snapshot);

        // 5. `--resume <A>/club-1.save.json --save-dir <missing_D>` (not yet created on disk)
        // creates <missing_D> cleanly on autosave while leaving <A> byte-identical.
        let dir_d_missing = dir.join("season_d_missing");
        assert!(!dir_d_missing.exists());
        let missing_opts = match parse_args([
            "league-tui".to_owned(),
            "--resume".to_owned(),
            a_save.to_string_lossy().into_owned(),
            "--save-dir".to_owned(),
            dir_d_missing.to_string_lossy().into_owned(),
        ]) {
            CliAction::RunClub(o) => o,
            other => return Err(format!("expected RunClub missing target, got {other:?}").into()),
        };
        let mut resumed_d = build_session_for_cli(&missing_opts)?;
        resumed_d.advance_clock(host::CURTAIN_LIFT_DEAD_TIME);
        resumed_d
            .handle_key_event(s_key(' '), vp)
            .map_err(|e| e.to_string())?;
        resumed_d
            .handle_key_event(
                SeatKeyEvent::new(SeatKeyCode::Enter, SeatKeyModifiers::NONE),
                vp,
            )
            .map_err(|e| e.to_string())?;
        resumed_d.advance_clock(host::CURTAIN_LIFT_DEAD_TIME);
        resumed_d
            .handle_key_event(
                SeatKeyEvent::new(SeatKeyCode::Char('l'), SeatKeyModifiers::CTRL),
                vp,
            )
            .map_err(|e| e.to_string())?;
        assert!(dir_d_missing.join("club-1.save.json").exists());
        assert!(dir_d_missing.join("club-1.seal").exists());
        assert_eq!(std::fs::read(&a_save)?, a_save_snapshot);
        assert_eq!(std::fs::read(&a_seal)?, a_seal_snapshot);

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    fn start_lobby_table_1_for_test(
        target_dir: &Path,
        seed_suffix_digit: Option<char>,
        term: &mut Terminal<TestBackend>,
    ) -> Result<host::TuiHostSession, Box<dyn std::error::Error>> {
        use crossterm::event::KeyEventKind;

        let key_ev = |code: KeyCode, mods: KeyModifiers| {
            let mut k = KeyEvent::new(code, mods);
            k.kind = KeyEventKind::Press;
            Event::Key(k)
        };
        let opts = match parse_args([
            "league-tui".to_owned(),
            "--untimed".to_owned(),
            "--save-dir".to_owned(),
            target_dir.to_string_lossy().into_owned(),
        ]) {
            CliAction::RunClub(o) => o,
            other => return Err(format!("expected RunClub, got {other:?}").into()),
        };
        let mut steps = Vec::new();
        if let Some(digit) = seed_suffix_digit {
            for _ in 0..5 {
                steps.push(key_ev(KeyCode::Down, KeyModifiers::NONE));
            }
            steps.push(key_ev(KeyCode::Char(digit), KeyModifiers::NONE));
            for _ in 0..3 {
                steps.push(key_ev(KeyCode::Down, KeyModifiers::NONE));
            }
        } else {
            for _ in 0..8 {
                steps.push(key_ev(KeyCode::Down, KeyModifiers::NONE));
            }
        }
        steps.push(key_ev(KeyCode::Enter, KeyModifiers::NONE));
        let mut sink = host::TuiDiagnosticSink::new(host::LogLevel::Silent, None);
        sink.set_alternate_screen_active(true);
        let mut ev = ScriptedSessionEvents::from_events(steps);
        Ok(
            run_lobby_loop_with_sink(term, &opts, &mut ev, Some(target_dir), &mut sink)?
                .ok_or("lobby table start")?,
        )
    }

    fn advance_session_to_completed_for_test(
        session: &mut host::TuiHostSession,
        vp: ViewportSize,
    ) -> Result<(), Box<dyn std::error::Error>> {
        use league_seat_types::{SeatKeyCode, SeatKeyEvent, SeatKeyModifiers};
        let s_key = |c: char| SeatKeyEvent::new(SeatKeyCode::Char(c), SeatKeyModifiers::NONE);

        while session.phase() == league_core::TurnPhase::TalkAndMarket {
            if session.is_curtain_open() {
                session.advance_clock(host::CURTAIN_LIFT_DEAD_TIME);
                session
                    .handle_key_event(s_key(' '), vp)
                    .map_err(|e| e.to_string())?;
                session
                    .handle_key_event(
                        SeatKeyEvent::new(SeatKeyCode::Enter, SeatKeyModifiers::NONE),
                        vp,
                    )
                    .map_err(|e| e.to_string())?;
                session.advance_clock(host::CURTAIN_LIFT_DEAD_TIME);
            }
            session
                .handle_key_event(
                    SeatKeyEvent::new(SeatKeyCode::Char('l'), SeatKeyModifiers::CTRL),
                    vp,
                )
                .map_err(|e| e.to_string())?;
        }
        for _ in 0..session.seat_count() {
            session
                .handle_key_event(
                    SeatKeyEvent::new(SeatKeyCode::Enter, SeatKeyModifiers::NONE),
                    vp,
                )
                .map_err(|e| e.to_string())?;
        }
        if session.phase() != league_core::TurnPhase::Completed {
            return Err(format!("expected Completed phase, got {:?}", session.phase()).into());
        }
        Ok(())
    }

    #[test]
    fn export_never_overwrites_other_season_replay_on_renamed_resume_and_allows_repeat_same_season_export()
    -> Result<(), Box<dyn std::error::Error>> {
        use league_seat_types::{SeatKeyCode, SeatKeyEvent, SeatKeyModifiers};
        let s_key = |c: char| SeatKeyEvent::new(SeatKeyCode::Char(c), SeatKeyModifiers::NONE);
        let vp = ViewportSize::new(100, 30);

        let dir = std::env::temp_dir().join(format!("ml_replay_target_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let dir_a = dir.join("dir_a");
        let dir_y_tmp = dir.join("dir_y_tmp");
        let dir_y_out = dir.join("dir_y_out");
        std::fs::create_dir_all(&dir_a)?;
        std::fs::create_dir_all(&dir_y_tmp)?;
        std::fs::create_dir_all(&dir_y_out)?;
        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;

        // Season X (`TableId(1)`, default seed) plays to completion in `dir_a` and exports
        // `club-1.replay.json`, wiping `club-1.seal`.
        let mut sess_x = start_lobby_table_1_for_test(&dir_a, None, &mut terminal)?;
        advance_session_to_completed_for_test(&mut sess_x, vp)?;
        sess_x
            .handle_key_event(s_key('4'), vp)
            .map_err(|e| e.to_string())?;
        sess_x
            .handle_key_event(s_key('e'), vp)
            .map_err(|e| e.to_string())?;
        drop(sess_x);
        let x_replay_path = dir_a.join("club-1.replay.json");
        assert!(x_replay_path.exists());
        assert!(!dir_a.join("club-1.seal").exists());
        let x_replay_bytes = std::fs::read(&x_replay_path)?;

        // Season Y (`TableId(1)`, distinct seed '2') reaches Completed without exporting, then
        // moves into `dir_a` under renamed files `y.save.json` / `y.seal`.
        let mut sess_y = start_lobby_table_1_for_test(&dir_y_tmp, Some('2'), &mut terminal)?;
        advance_session_to_completed_for_test(&mut sess_y, vp)?;
        drop(sess_y);
        let y_save = dir_a.join("y.save.json");
        let y_seal = dir_a.join("y.seal");
        let y_seal_snapshot = std::fs::read(dir_y_tmp.join("club-1.seal"))?;
        std::fs::copy(dir_y_tmp.join("club-1.save.json"), &y_save)?;
        std::fs::write(&y_seal, &y_seal_snapshot)?;

        // 1. In-place resume (`--resume dir_a/y.save.json` with no `--save-dir`) refuses export with
        // `REPLAY_TARGET_EXISTS` instead of overwriting Season X's `dir_a/club-1.replay.json`,
        // preserves `y.seal`, sets exit code 3, and displays a usable hint on Tab 4.
        let opts_in_place = match parse_args([
            "league-tui".to_owned(),
            "--resume".to_owned(),
            y_save.to_string_lossy().into_owned(),
        ]) {
            CliAction::RunClub(o) => o,
            other => return Err(format!("expected RunClub, got {other:?}").into()),
        };
        let mut sess_y_inplace = build_session_for_cli(&opts_in_place)?;
        sess_y_inplace
            .handle_key_event(s_key('4'), vp)
            .map_err(|e| e.to_string())?;
        let clash_err = sess_y_inplace
            .handle_key_event(s_key('e'), vp)
            .err()
            .ok_or("export over foreign replay must refuse")?;
        assert_eq!(clash_err.code, "REPLAY_TARGET_EXISTS");
        assert!(!clash_err.hint.is_empty());
        assert_eq!(std::fs::read(&x_replay_path)?, x_replay_bytes);
        assert!(y_seal.exists());
        assert!(sess_y_inplace.has_partial_post_season_failure());
        let tui_err = sess_y_inplace
            .terminal_tui_error()
            .ok_or("clash export must set terminal_tui_error")?;
        assert_eq!(tui_err.exit_code(), 3);
        assert_eq!(tui_err.to_structured_error().code, "REPLAY_TARGET_EXISTS");
        terminal.draw(|f| {
            let _ = sess_y_inplace.render_current_frame(f);
        })?;
        let clash_buf = terminal.backend().buffer();
        let clash_screen = (0..clash_buf.area.height)
            .map(|y| {
                (0..clash_buf.area.width)
                    .map(|x| clash_buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(clash_screen.contains("REPLAY_TARGET_EXISTS"));
        assert!(clash_screen.contains("Hint:"));
        drop(sess_y_inplace);

        // 2. Resuming `dir_a/y.save.json` into `--save-dir <dir_y_out>` exports Season Y's distinct
        // replay, wipes `dir_a/y.seal`, and allows repeated export of the SAME season (`e` again).
        let opts_out = match parse_args([
            "league-tui".to_owned(),
            "--resume".to_owned(),
            y_save.to_string_lossy().into_owned(),
            "--save-dir".to_owned(),
            dir_y_out.to_string_lossy().into_owned(),
        ]) {
            CliAction::RunClub(o) => o,
            other => return Err(format!("expected RunClub, got {other:?}").into()),
        };
        let mut sess_y_out = build_session_for_cli(&opts_out)?;
        sess_y_out
            .handle_key_event(s_key('4'), vp)
            .map_err(|e| e.to_string())?;
        sess_y_out
            .handle_key_event(s_key('e'), vp)
            .map_err(|e| e.to_string())?;
        let y_replay_path = dir_y_out.join("club-1.replay.json");
        assert!(y_replay_path.exists());
        assert!(!y_seal.exists());
        assert!(sess_y_out.terminal_tui_error().is_none());
        let y_replay_bytes = std::fs::read(&y_replay_path)?;
        assert_ne!(y_replay_bytes, x_replay_bytes);
        assert_eq!(std::fs::read(&x_replay_path)?, x_replay_bytes);
        let parsed_y = league_core::SeasonReplayLog::from_club_json_slice(&y_replay_bytes)?;
        assert!(parsed_y.turns.len() >= 24);

        // Repeat export of the same season in the same session keeps working idempotently.
        sess_y_out
            .handle_key_event(s_key('e'), vp)
            .map_err(|e| e.to_string())?;
        assert_eq!(std::fs::read(&y_replay_path)?, y_replay_bytes);
        assert!(sess_y_out.terminal_tui_error().is_none());
        drop(sess_y_out);

        // Simulated retry after partial `.seal`-wipe failure (replay already matches byte-for-byte
        // alongside the in-place resumed `.save.json`/`.seal`): resume and export succeeds cleanly
        // and wipes the `.seal` without treating identical replay bytes as `REPLAY_TARGET_EXISTS`.
        let y_retry_save = dir_y_out.join("y_retry.save.json");
        let y_retry_seal = dir_y_out.join("y_retry.seal");
        std::fs::copy(&y_save, &y_retry_save)?;
        std::fs::write(&y_retry_seal, &y_seal_snapshot)?;
        let opts_retry = match parse_args([
            "league-tui".to_owned(),
            "--resume".to_owned(),
            y_retry_save.to_string_lossy().into_owned(),
        ]) {
            CliAction::RunClub(o) => o,
            other => return Err(format!("expected RunClub retry, got {other:?}").into()),
        };
        let mut sess_y_retry = build_session_for_cli(&opts_retry)?;
        sess_y_retry
            .handle_key_event(s_key('4'), vp)
            .map_err(|e| e.to_string())?;
        sess_y_retry
            .handle_key_event(s_key('e'), vp)
            .map_err(|e| e.to_string())?;
        assert!(!y_retry_seal.exists());
        assert_eq!(std::fs::read(&y_replay_path)?, y_replay_bytes);
        assert!(sess_y_retry.terminal_tui_error().is_none());

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn finished_season_resumed_into_missing_or_empty_save_dir_exports_and_wipes_source_seal()
    -> Result<(), Box<dyn std::error::Error>> {
        use league_seat_types::{SeatKeyCode, SeatKeyEvent, SeatKeyModifiers};
        let s_key = |c: char| SeatKeyEvent::new(SeatKeyCode::Char(c), SeatKeyModifiers::NONE);
        let vp = ViewportSize::new(100, 30);

        let dir = std::env::temp_dir().join(format!("ml_fin_cross_dir_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let dir_a1 = dir.join("src_a1");
        let dir_b_missing = dir.join("out_b_missing");
        let dir_a2 = dir.join("src_a2");
        let dir_b_empty = dir.join("out_b_empty");
        std::fs::create_dir_all(&dir_a1)?;
        std::fs::create_dir_all(&dir_a2)?;
        std::fs::create_dir_all(&dir_b_empty)?;
        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;

        // Case 1: Finished season in <dir_a1> resumed into missing <dir_b_missing>.
        assert!(!dir_b_missing.exists());
        let mut sess_a1 = start_lobby_table_1_for_test(&dir_a1, None, &mut terminal)?;
        advance_session_to_completed_for_test(&mut sess_a1, vp)?;
        drop(sess_a1);
        let a1_save = dir_a1.join("club-1.save.json");
        let a1_seal = dir_a1.join("club-1.seal");
        assert!(a1_seal.exists());

        let opts_missing = match parse_args([
            "league-tui".to_owned(),
            "--resume".to_owned(),
            a1_save.to_string_lossy().into_owned(),
            "--save-dir".to_owned(),
            dir_b_missing.to_string_lossy().into_owned(),
        ]) {
            CliAction::RunClub(o) => o,
            other => return Err(format!("expected RunClub missing dir, got {other:?}").into()),
        };
        let mut res_missing = build_session_for_cli(&opts_missing)?;
        assert_eq!(res_missing.phase(), league_core::TurnPhase::Completed);
        assert!(!res_missing.is_curtain_open());
        assert_eq!(res_missing.observe_seat_call_count(), 0);
        res_missing
            .handle_key_event(s_key('4'), vp)
            .map_err(|e| e.to_string())?;
        res_missing
            .handle_key_event(s_key('e'), vp)
            .map_err(|e| e.to_string())?;
        let missing_replay = dir_b_missing.join("club-1.replay.json");
        assert!(missing_replay.exists());
        assert!(!a1_seal.exists());
        assert!(!dir_b_missing.join("club-1.seal").exists());
        let parsed1 =
            league_core::SeasonReplayLog::from_club_json_slice(&std::fs::read(&missing_replay)?)?;
        assert!(parsed1.turns.len() >= 24);
        let mut lobby1 = host::LobbyHostSession::new(LobbyCliOverrides::default());
        let next1 = lobby1.validate_and_start_in_dir(1, Some(&dir_a1))?;
        assert_eq!(next1.table_id(), TableId(2));

        // Case 2: Finished season in <dir_a2> resumed into existing empty <dir_b_empty>.
        let mut sess_a2 = start_lobby_table_1_for_test(&dir_a2, None, &mut terminal)?;
        advance_session_to_completed_for_test(&mut sess_a2, vp)?;
        drop(sess_a2);
        let a2_save = dir_a2.join("club-1.save.json");
        let a2_seal = dir_a2.join("club-1.seal");
        assert!(a2_seal.exists());

        let opts_empty = match parse_args([
            "league-tui".to_owned(),
            "--resume".to_owned(),
            a2_save.to_string_lossy().into_owned(),
            "--save-dir".to_owned(),
            dir_b_empty.to_string_lossy().into_owned(),
        ]) {
            CliAction::RunClub(o) => o,
            other => return Err(format!("expected RunClub empty dir, got {other:?}").into()),
        };
        let mut res_empty = build_session_for_cli(&opts_empty)?;
        assert_eq!(res_empty.phase(), league_core::TurnPhase::Completed);
        res_empty
            .handle_key_event(s_key('4'), vp)
            .map_err(|e| e.to_string())?;
        res_empty
            .handle_key_event(s_key('e'), vp)
            .map_err(|e| e.to_string())?;
        let empty_replay = dir_b_empty.join("club-1.replay.json");
        assert!(empty_replay.exists());
        assert!(!a2_seal.exists());
        assert!(!dir_b_empty.join("club-1.seal").exists());
        let parsed2 =
            league_core::SeasonReplayLog::from_club_json_slice(&std::fs::read(&empty_replay)?)?;
        assert!(parsed2.turns.len() >= 24);
        let mut lobby2 = host::LobbyHostSession::new(LobbyCliOverrides::default());
        let next2 = lobby2.validate_and_start_in_dir(1, Some(&dir_a2))?;
        assert_eq!(next2.table_id(), TableId(2));

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    fn strip_tests_and_comments_for_panic_scan(source: &str) -> String {
        let mut out = String::with_capacity(source.len());
        let mut iter = source.char_indices().peekable();
        let mut depth: usize = 0;
        let mut in_cfg_test = false;
        let mut cfg_test_depth: Option<usize> = None;

        while let Some((idx, ch)) = iter.next() {
            let rest = &source[idx..];
            if rest.starts_with("//") {
                for (_, nc) in iter.by_ref() {
                    if nc == '\n' {
                        out.push('\n');
                        break;
                    }
                }
                continue;
            }
            if rest.starts_with("/*") {
                iter.next();
                let mut nest = 1_usize;
                while nest > 0 {
                    let Some((_, c)) = iter.next() else {
                        break;
                    };
                    if c == '/' && iter.peek().is_some_and(|&(_, nc)| nc == '*') {
                        iter.next();
                        nest += 1;
                    } else if c == '*' && iter.peek().is_some_and(|&(_, nc)| nc == '/') {
                        iter.next();
                        nest -= 1;
                    }
                }
                out.push(' ');
                continue;
            }
            if rest.starts_with("#[cfg(test)]") {
                in_cfg_test = true;
                for _ in 0..("#[cfg(test)]".len() - 1) {
                    iter.next();
                }
                continue;
            }
            if ch == 'r' && (rest.starts_with("r\"") || rest.starts_with("r#")) {
                let hashes = rest[1..].bytes().take_while(|&b| b == b'#').count();
                if rest[1 + hashes..].starts_with('"') {
                    for _ in 0..(hashes + 1) {
                        iter.next();
                    }
                    while let Some((s_idx, _)) = iter.next() {
                        if source[s_idx..].starts_with('"')
                            && source[s_idx + 1..]
                                .bytes()
                                .take(hashes)
                                .eq(std::iter::repeat_n(b'#', hashes))
                        {
                            for _ in 0..hashes {
                                iter.next();
                            }
                            break;
                        }
                    }
                    out.push_str("\"\"");
                    continue;
                }
            }
            if ch == '"' {
                while let Some((_, sc)) = iter.next() {
                    if sc == '\\' {
                        iter.next();
                    } else if sc == '"' {
                        break;
                    }
                }
                out.push_str("\"\"");
                continue;
            }
            if ch == '\'' {
                let mut cloned = iter.clone();
                if let Some((_, c1)) = cloned.next() {
                    if c1 == '\\' {
                        iter.next();
                        iter.next();
                        for (_, ec) in iter.by_ref() {
                            if ec == '\'' {
                                break;
                            }
                        }
                        out.push_str("''");
                        continue;
                    } else if cloned.next().is_some_and(|(_, c2)| c2 == '\'') {
                        iter.next();
                        iter.next();
                        out.push_str("''");
                        continue;
                    }
                }
            }

            if ch == '{' {
                depth += 1;
                if in_cfg_test && cfg_test_depth.is_none() {
                    cfg_test_depth = Some(depth);
                    in_cfg_test = false;
                }
            } else if ch == '}' {
                if cfg_test_depth == Some(depth) {
                    cfg_test_depth = None;
                }
                depth = depth.saturating_sub(1);
            } else if (ch == ';' || ch == ',') && cfg_test_depth.is_none() {
                in_cfg_test = false;
            }

            if cfg_test_depth.is_none() && !in_cfg_test {
                out.push(ch);
            }
        }
        out
    }

    fn scan_prod_for_panics(source: &str) -> Result<(), String> {
        let filtered = strip_tests_and_comments_for_panic_scan(source);
        for forbidden in [
            "panic!",
            "unreachable!",
            "todo!",
            "unimplemented!",
            "assert!",
            "assert_eq!",
            "assert_ne!",
            "debug_assert!",
            "debug_assert_eq!",
            "debug_assert_ne!",
            ".unwrap(",
            ".expect(",
        ] {
            if filtered.contains(forbidden) {
                return Err(format!("found forbidden `{forbidden}` in non-test code"));
            }
        }
        Ok(())
    }

    #[test]
    fn production_club_seat_and_tui_sources_contain_no_panic_unwrap_expect_or_assert()
    -> Result<(), Box<dyn std::error::Error>> {
        // Negative controls for the scanner itself.
        for bad in [
            "fn bad() { panic!(\"boom\"); }",
            "fn bad(x: Option<u8>) { let _ = x.unwrap(); }",
            "fn bad(x: Option<u8>) { let _ = x.expect(\"missing\"); }",
            "fn bad(ok: bool) { assert!(ok); }",
        ] {
            assert!(
                scan_prod_for_panics(bad).is_err(),
                "scanner missed negative control `{bad}`"
            );
        }
        // Positive controls: test module, comments, and string literals are ignored cleanly.
        assert!(
            scan_prod_for_panics(
                "// panic!()\nfn ok() -> &'static str { \"panic!()\" }\n#[cfg(test)]\nmod tests { fn t() { assert!(true); None::<u8>.unwrap(); } }\n",
            )
            .is_ok()
        );

        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = manifest_dir
            .parent()
            .and_then(|p| p.parent())
            .ok_or("workspace root")?;
        let dirs = [
            root.join("crates/league-error/src"),
            root.join("crates/league-seat-types/src"),
            root.join("crates/league-seat-ui/src"),
            root.join("crates/league-core/src/club"),
            root.join("crates/league-tui/src"),
        ];
        for dir in &dirs {
            let mut stack = vec![dir.clone()];
            while let Some(d) = stack.pop() {
                for entry in std::fs::read_dir(&d)? {
                    let path = entry?.path();
                    if path.is_dir() {
                        stack.push(path);
                    } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                        let content = std::fs::read_to_string(&path)?;
                        scan_prod_for_panics(&content)
                            .map_err(|err| format!("{}: {err}", path.display()))?;
                    }
                }
            }
        }
        Ok(())
    }
}
