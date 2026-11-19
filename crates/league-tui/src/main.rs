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
use league_core::{TableId, ToStructuredError as _, scrub_secret_text};
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
    extra_markers: &[String],
    backtrace_text: Option<&str>,
) -> String {
    let collapsed = payload.replace(['\r', '\n'], " ");
    let scrubbed_msg = host::scrub_tui_diagnostic_text(&collapsed, extra_markers);
    let loc = match location {
        Some(raw) => host::scrub_tui_diagnostic_text(raw, extra_markers),
        None => "unknown".to_owned(),
    };
    let mut out = format!("panic at {loc}: {scrubbed_msg}\n");
    if level == host::LogLevel::Debug
        && let Some(bt) = backtrace_text
    {
        let scrubbed_bt = host::scrub_tui_diagnostic_text(bt, extra_markers);
        out.push_str(&format!("stack backtrace:\n{scrubbed_bt}\n"));
    }
    out
}

fn handle_panic_with_restore(
    info: &std::panic::PanicHookInfo<'_>,
    level: host::LogLevel,
    extra_markers: &[String],
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
    let rendered = format_panic_output(
        payload,
        loc_owned.as_deref(),
        level,
        extra_markers,
        bt_owned.as_deref(),
    );
    let _ = err_out.write_all(rendered.as_bytes());
    let _ = err_out.flush();
}

fn install_redacting_panic_hook_with_level(level: host::LogLevel) {
    host::set_panic_hook_log_level(level);
    std::panic::set_hook(Box::new(|info| {
        let level = host::panic_hook_log_level();
        let mut stderr = io::stderr();
        handle_panic_with_restore(info, level, &[], &mut restore_cooked_terminal, &mut stderr);
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
        matches.sort();
        let save = matches.into_iter().next().ok_or_else(|| {
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
    if let Some(resume_target) = &opts.resume_path {
        let (save_path, seal_path) = resolve_resume_paths_checked(resume_target)?;
        let mut session = host::TuiHostSession::read_save_and_seal(&save_path, &seal_path, None)
            .map_err(host::TuiError::SaveLoadFailed)?;
        apply_cli_session_options(&mut session, opts, true);
        if let Some(save_dir) = &opts.save_dir {
            let (s_path, k_path) = default_save_paths(save_dir, session.table_id());
            session.enable_autosave(s_path, k_path);
        } else {
            session.enable_autosave(save_path, seal_path);
        }
        return Ok(session);
    }
    let ring = default_dev_ring_table().map_err(|err| {
        host::TuiError::LobbyValidation(league_core::StructuredError::new(
            "NO_FEASIBLE_TABLE",
            err.to_string(),
            "Try another dev seed.",
            false,
        ))
    })?;
    let mut session = host::TuiHostSession::from_ring_table(&ring, TableId(1), &[0], 1)
        .map_err(|err| host::TuiError::LobbyValidation(err.to_structured_error()))?;
    apply_cli_session_options(&mut session, opts, false);
    let save_dir = opts.save_dir.clone().unwrap_or_else(|| PathBuf::from("."));
    let (save_path, seal_path) = default_save_paths(&save_dir, session.table_id());
    session.enable_autosave(save_path, seal_path);
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
                            match lobby.validate_and_start(TableId(1), 1, save_dir) {
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
                host::TuiError::LobbyValidation(StructuredError::new(
                    "RESERVED_SEED_FORBIDDEN",
                    "reserved seed 1000000",
                    "Use 0..=9999.",
                    false,
                )),
                1,
                "RESERVED_SEED_FORBIDDEN",
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

    #[test]
    fn all_four_log_levels_suppress_alternate_screen_writes_and_redact_all_five_secret_markers()
    -> Result<(), Box<dyn std::error::Error>> {
        use crossterm::event::KeyEventKind;

        let dir = std::env::temp_dir().join(format!("ml_log_levels_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;

        let ring = default_dev_ring_table()?;
        let mut session =
            host::TuiHostSession::from_ring_table(&ring, TableId(81_703), &[0, 1], 19)?;
        session.enable_seat_pins();

        let planted_pin = "4829";
        let planted_note = "SECRET_LOG_NOTE_marker_991";
        let planted_debt_num = "765432";
        let obs0 = session.build_seat_observation(league_core::Seat(0))?;
        let planted_mandate =
            host::mandate_short_marker(obs0.own_mandate().ok_or("s0 mandate")?.kind);
        let planted_salt_hex = session.turn_reveal_salt_hex(league_core::Seat(0))?;
        let planted_xrpl = "sEdSKaCy2JT7JaM7v95H9SxkhP9wS2r";
        let planted_api_key = "sk-or-v1-logSecretKey77";

        // Drive Seat 0 to set PIN `"4829"`, stage private note & debt order `"765432"`, then lock.
        let key_ev = |code: KeyCode, mods: KeyModifiers| {
            let mut k = KeyEvent::new(code, mods);
            k.kind = KeyEventKind::Press;
            Event::Key(k)
        };
        let mut init_events: Vec<(Duration, Option<Event>, Vec<Event>)> = vec![
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
            init_events.push((
                Duration::ZERO,
                Some(key_ev(KeyCode::Char(d), KeyModifiers::NONE)),
                Vec::new(),
            ));
        }
        init_events.push((
            Duration::ZERO,
            Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
            Vec::new(),
        ));
        init_events.push((
            host::CURTAIN_LIFT_DEAD_TIME,
            Some(key_ev(KeyCode::Char('n'), KeyModifiers::NONE)),
            Vec::new(),
        ));
        for ch in planted_note.chars() {
            init_events.push((
                Duration::ZERO,
                Some(key_ev(KeyCode::Char(ch), KeyModifiers::NONE)),
                Vec::new(),
            ));
        }
        init_events.push((
            Duration::ZERO,
            Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
            Vec::new(),
        ));

        let mut init_script = ScriptedSessionEvents::from_timed_with_queued(init_events);
        let mut init_term = Terminal::new(TestBackend::new(100, 30))?;
        run_session_loop(&mut init_term, &mut session, &mut init_script)?;
        session.apply_seat_intent(
            league_core::Seat(0),
            league_seat_types::SeatIntent::update_bundle(
                league_seat_types::BundleDraftAction::SetDebtRepaymentCred(765_432),
            ),
        )?;

        // Exercise all 4 levels via CLI flags + env fallback (`LEAGUE_LOG_LEVEL`).
        for level in [
            host::LogLevel::Silent,
            host::LogLevel::Normal,
            host::LogLevel::Verbose,
            host::LogLevel::Debug,
        ] {
            let log_file_path = dir.join(format!("diag-{}.log", level.as_str()));
            let written = std::rc::Rc::new(std::cell::RefCell::new(Vec::<u8>::new()));
            let mut alt_term = Terminal::new(FixedSizeCrosstermBackend::new(
                SharedVecWriter(written.clone()),
                100,
                30,
            ))?;
            let mut sink = host::TuiDiagnosticSink::new(level, Some(log_file_path.clone()));
            sink.add_redaction_markers([
                planted_pin,
                planted_note,
                planted_mandate.as_str(),
                planted_debt_num,
                planted_salt_hex.as_str(),
            ]);
            sink.set_alternate_screen_active(true);

            let mut step_events = ScriptedSessionEvents::from_timed_with_queued([(
                host::CURTAIN_LIFT_DEAD_TIME,
                Some(key_ev(KeyCode::Char('l'), KeyModifiers::CONTROL)),
                Vec::new(),
            )]);
            run_session_loop_with_sink(&mut alt_term, &mut session, &mut step_events, &mut sink)?;

            // While alternate screen was active, zero diagnostic lines hit stdout/stderr or `SharedVecWriter`.
            let alt_bytes = String::from_utf8_lossy(&written.borrow()).into_owned();
            assert!(
                !alt_bytes.contains("[normal]")
                    && !alt_bytes.contains("[verbose]")
                    && !alt_bytes.contains("[debug]")
            );

            // Also attempt to log all 5 secret markers explicitly inside the active sink across this level.
            sink.emit(
                level,
                &format!(
                    "secret_probe pin={planted_pin} note={planted_note} mandate={planted_mandate} debt={planted_debt_num} salt={planted_salt_hex} seed={planted_xrpl} key={planted_api_key} \"seat_pins\":[\"4829\"] \"preimage_secret\":\"hiddenSaltVal\""
                ),
            );
            sink.emit_error(
                &host::TuiError::TerminalIo(format!(
                    "err pin={planted_pin} note={planted_note} mandate={planted_mandate} debt={planted_debt_num} salt={planted_salt_hex} seed={planted_xrpl} key={planted_api_key}"
                )),
                host::ErrorFormat::Text,
            );

            // Attempting to flush before terminal restore emits nothing.
            let mut pre_restore = Vec::new();
            sink.flush_after_restore(&mut pre_restore);
            assert!(pre_restore.is_empty());

            // Restore cooked mode -> flush diagnostics.
            sink.set_alternate_screen_active(false);
            let mut post_restore = Vec::new();
            sink.flush_after_restore(&mut post_restore);
            let flushed_stderr = String::from_utf8(post_restore)?;
            let file_log = std::fs::read_to_string(&log_file_path).unwrap_or_default();

            for combined in [&flushed_stderr, &file_log] {
                for secret in [
                    planted_pin,
                    planted_note,
                    planted_mandate.as_str(),
                    planted_debt_num,
                    planted_salt_hex.as_str(),
                    planted_xrpl,
                    planted_api_key,
                    "hiddenSaltVal",
                ] {
                    assert!(
                        !combined.contains(secret),
                        "level `{}` leaked `{secret}` in log:\n{combined}",
                        level.as_str()
                    );
                }
            }
            if level == host::LogLevel::Silent {
                assert!(!flushed_stderr.contains("session start"));
            } else {
                assert!(flushed_stderr.contains("session start"));
            }
            if level >= host::LogLevel::Verbose {
                assert!(flushed_stderr.contains("state transition:"));
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

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn panic_hook_restores_terminal_prints_one_redacted_line_and_gates_backtrace_on_debug() {
        let extra = vec![
            "4829".to_owned(),
            "PANIC_SECRET_NOTE_77".to_owned(),
            "MinProjectEquity >=3".to_owned(),
            "765432".to_owned(),
            "deadbeef01234567".to_owned(),
        ];
        let planted_payload = "boom\npin=4829 note=PANIC_SECRET_NOTE_77 mandate=MinProjectEquity >=3 order=765432 salt=deadbeef01234567 seed=sEdSKaCy2JT7JaM7v95H9SxkhP9wS2r key=sk-or-v1-panicKey88";

        // Test 1: real std::panic::catch_unwind with a forced panic invokes terminal restore
        // strictly BEFORE writing to stderr, emits exactly one line under Normal, and leaks zero secrets.
        let prev_hook = std::panic::take_hook();
        let events_log = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let captured_err = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
        let extra_hook = extra.clone();
        let ev_clone = std::sync::Arc::clone(&events_log);
        let err_clone = std::sync::Arc::clone(&captured_err);
        std::panic::set_hook(Box::new(move |info| {
            let mut buf = Vec::new();
            let mut restore = || {
                if let Ok(mut g) = ev_clone.lock() {
                    g.push("restore".to_owned());
                }
            };
            handle_panic_with_restore(
                info,
                host::LogLevel::Normal,
                &extra_hook,
                &mut restore,
                &mut buf,
            );
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
            "PANIC_SECRET_NOTE_77",
            "MinProjectEquity >=3",
            "765432",
            "deadbeef01234567",
            "sEdSKaCy2JT7JaM7v95H9SxkhP9wS2r",
            "sk-or-v1-panicKey88",
        ] {
            assert!(
                !normal_str.contains(secret),
                "panic line leaked `{secret}`:\n{normal_str}"
            );
        }

        // Test 2: Debug level adds scrubbed stack backtrace while Normal/Silent/Verbose omit it.
        let fake_bt = "  0: my_fn\n  secret=PANIC_SECRET_NOTE_77 sk-or-v1-btKey11";
        let no_bt = format_panic_output(
            planted_payload,
            Some("main.rs:10:1"),
            host::LogLevel::Verbose,
            &extra,
            Some(fake_bt),
        );
        assert_eq!(no_bt.lines().count(), 1);
        assert!(!no_bt.contains("stack backtrace:"));

        let with_bt = format_panic_output(
            planted_payload,
            Some("main.rs:10:1"),
            host::LogLevel::Debug,
            &extra,
            Some(fake_bt),
        );
        assert!(with_bt.contains("stack backtrace:"));
        assert!(!with_bt.contains("PANIC_SECRET_NOTE_77"));
        assert!(!with_bt.contains("sk-or-v1-btKey11"));
        assert!(with_bt.contains("[REDACTED]"));
    }
}
