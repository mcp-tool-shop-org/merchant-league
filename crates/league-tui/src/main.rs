//! Merchant League terminal host binary.
//!
//! Owns the Club Mode host session (`host::TuiHostSession`) and drives `league-seat-ui`
//! rendering over an interactive crossterm/ratatui terminal.

mod host;

use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use league_core::{TableId, TurnPhase, scrub_secret_text};
use league_gen::{
    GeneratorConfig, RingTable, SeatCount, SeedPool, TablePurpose, TableSetupOptions,
    generate_ring_table_for_setup, is_reserved_test_seed, seed_pool,
};
use league_seat_types::ViewportSize;
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CliAction {
    Help,
    Version,
    RunClub(CliOptions),
    Error(String),
}

fn parse_args<I, S>(args: I) -> CliAction
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut opts = CliOptions::default();
    let mut iter = args.into_iter().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_ref() {
            "-h" | "--help" => return CliAction::Help,
            "-V" | "--version" => return CliAction::Version,
            "--save-dir" => {
                let Some(val) = iter.next() else {
                    return CliAction::Error("--save-dir requires a directory path".to_owned());
                };
                opts.save_dir = Some(PathBuf::from(val.as_ref()));
            }
            "--resume" => {
                let Some(val) = iter.next() else {
                    return CliAction::Error(
                        "--resume requires a .save.json file or directory".to_owned(),
                    );
                };
                opts.resume_path = Some(PathBuf::from(val.as_ref()));
            }
            "--turn-clock" => {
                if opts.untimed {
                    return CliAction::Error(
                        "--turn-clock cannot be combined with --untimed".to_owned(),
                    );
                }
                let Some(val) = iter.next() else {
                    return CliAction::Error(format!(
                        "--turn-clock requires seconds in {MIN_TURN_CLOCK_SECS}..={MAX_TURN_CLOCK_SECS}"
                    ));
                };
                let Ok(secs) = val.as_ref().parse::<u16>() else {
                    return CliAction::Error(format!(
                        "invalid --turn-clock value `{}` (expected {MIN_TURN_CLOCK_SECS}..={MAX_TURN_CLOCK_SECS})",
                        val.as_ref()
                    ));
                };
                if !(MIN_TURN_CLOCK_SECS..=MAX_TURN_CLOCK_SECS).contains(&secs) {
                    return CliAction::Error(format!(
                        "--turn-clock {secs} out of range ({MIN_TURN_CLOCK_SECS}..={MAX_TURN_CLOCK_SECS})"
                    ));
                }
                opts.turn_clock_secs = Some(secs);
            }
            "--untimed" => {
                if opts.turn_clock_secs.is_some() {
                    return CliAction::Error(
                        "--untimed cannot be combined with --turn-clock".to_owned(),
                    );
                }
                opts.untimed = true;
            }
            "--pins" => {
                opts.pins_enabled = true;
            }
            other if other == "--pin" || other.starts_with("--pin=") => {
                return CliAction::Error(
                    "PIN digits must never be passed on the command line or via env vars; pass --pins and enter each seat's 4-digit PIN at its first Curtain"
                        .to_owned(),
                );
            }
            other => {
                return CliAction::Error(format!("unknown flag `{other}` (try --help)"));
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

struct TerminalCleanup;

impl Drop for TerminalCleanup {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = leave_alternate_screen(&mut stdout);
    }
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
    loop {
        session.sync_clock(events.elapsed());
        if session.quit_requested() || session.phase() != TurnPhase::TalkAndMarket {
            return Ok(());
        }
        if session.take_screen_clear() {
            terminal
                .clear()
                .map_err(|err| io::Error::other(scrub_secret_text(&err.to_string())))?;
        }
        if session.take_input_drain() {
            events.drain_queued_input()?;
        }
        let mut render_err = None;
        terminal
            .draw(|frame| {
                if let Err(err) = session.render_current_frame(frame) {
                    render_err = Some(err);
                }
            })
            .map_err(|err| io::Error::other(scrub_secret_text(&err.to_string())))?;
        if let Some(err) = render_err {
            return Err(io::Error::other(scrub_secret_text(&err.to_string())));
        }
        let area = terminal
            .size()
            .map_err(|err| io::Error::other(scrub_secret_text(&err.to_string())))?;
        let vp = ViewportSize::new(area.width, area.height);
        match events.poll_event()? {
            None => return Ok(()),
            Some(PollEvent::Tick) => {
                session.sync_clock(events.elapsed());
            }
            Some(PollEvent::Input(Event::Key(key))) => {
                session.sync_clock(events.elapsed());
                if session.take_input_drain() {
                    events.drain_queued_input()?;
                } else if let Some(mapped) = host::map_crossterm_key(key) {
                    let _ = session.handle_key_event(mapped, vp);
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

fn resolve_resume_paths(target: &Path) -> io::Result<(PathBuf, PathBuf)> {
    if target.is_dir() {
        let mut matches = Vec::new();
        for entry in std::fs::read_dir(target)? {
            let path = entry?.path();
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
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("no club-*.save.json found in {}", target.display()),
            )
        })?;
        return resolve_resume_paths(&save);
    }
    let name = target
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| io::Error::other("resume path has no file name"))?;
    let stem = name
        .strip_suffix(".save.json")
        .or_else(|| name.strip_suffix(".json"))
        .unwrap_or(name);
    let seal = target.with_file_name(format!("{stem}.seal"));
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

fn build_session_for_cli(opts: &CliOptions) -> io::Result<host::TuiHostSession> {
    if let Some(resume_target) = &opts.resume_path {
        let (save_path, seal_path) = resolve_resume_paths(resume_target)?;
        let mut session = host::TuiHostSession::read_save_and_seal(&save_path, &seal_path, None)
            .map_err(|err| io::Error::other(scrub_secret_text(&err.to_string())))?;
        apply_cli_session_options(&mut session, opts, true);
        if let Some(save_dir) = &opts.save_dir {
            let (s_path, k_path) = default_save_paths(save_dir, session.table_id());
            session.enable_autosave(s_path, k_path);
        } else {
            session.enable_autosave(save_path, seal_path);
        }
        return Ok(session);
    }
    let ring = default_dev_ring_table()?;
    let mut session = host::TuiHostSession::from_ring_table(&ring, TableId(1), &[0], 1)
        .map_err(|err| io::Error::other(scrub_secret_text(&err.to_string())))?;
    apply_cli_session_options(&mut session, opts, false);
    let save_dir = opts.save_dir.clone().unwrap_or_else(|| PathBuf::from("."));
    let (save_path, seal_path) = default_save_paths(&save_dir, session.table_id());
    session.enable_autosave(save_path, seal_path);
    Ok(session)
}

fn run_interactive(opts: &CliOptions) -> io::Result<()> {
    let mut session = build_session_for_cli(opts)?;
    enable_raw_mode()?;
    let _cleanup = TerminalCleanup;
    let mut stdout = io::stdout();
    enter_alternate_screen(&mut stdout)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let mut live_events = LiveTerminalEvents::new();
    let res = run_session_loop(&mut terminal, &mut session, &mut live_events);
    finish_terminal_session(&mut terminal);
    res
}

fn dispatch_cli<F>(
    args: impl IntoIterator<Item = String>,
    out: &mut dyn io::Write,
    err_out: &mut dyn io::Write,
    run_tui: F,
) -> ExitCode
where
    F: FnOnce(&CliOptions) -> io::Result<()>,
{
    match parse_args(args) {
        CliAction::Help => {
            let _ = writeln!(
                out,
                "league-tui {} - Merchant League XRPL Testnet host",
                env!("CARGO_PKG_VERSION")
            );
            let _ = writeln!(
                out,
                "Usage: league-tui [-h|--help] [-V|--version] [--save-dir <DIR>] [--resume <PATH>] [--turn-clock <180..=300> | --untimed] [--pins]"
            );
            ExitCode::SUCCESS
        }
        CliAction::Version => {
            let _ = writeln!(out, "league-tui {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        CliAction::RunClub(opts) => match run_tui(&opts) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                let _ = writeln!(err_out, "error: {}", scrub_secret_text(&err.to_string()));
                ExitCode::from(2)
            }
        },
        CliAction::Error(msg) => {
            let _ = writeln!(err_out, "error: {}", scrub_secret_text(&msg));
            ExitCode::from(1)
        }
    }
}

fn main() -> ExitCode {
    let mut out = io::stdout();
    let mut err_out = io::stderr();
    dispatch_cli(std::env::args(), &mut out, &mut err_out, run_interactive)
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
            ]),
            CliAction::RunClub(CliOptions {
                save_dir: Some(PathBuf::from("saves")),
                resume_path: Some(PathBuf::from("saves/club-1.save.json")),
                turn_clock_secs: Some(180),
                untimed: false,
                pins_enabled: true,
            })
        );
        assert_eq!(
            parse_args(["league-tui", "--untimed", "--pins"]),
            CliAction::RunClub(CliOptions {
                save_dir: None,
                resume_path: None,
                turn_clock_secs: None,
                untimed: true,
                pins_enabled: true,
            })
        );
        assert!(matches!(
            parse_args(["league-tui", "--turn-clock", "120"]),
            CliAction::Error(_)
        ));
        assert!(matches!(
            parse_args(["league-tui", "--turn-clock", "301"]),
            CliAction::Error(_)
        ));
        assert!(matches!(
            parse_args(["league-tui", "--turn-clock", "210", "--untimed"]),
            CliAction::Error(_)
        ));
        assert!(matches!(
            parse_args(["league-tui", "--pin=1234"]),
            CliAction::Error(_)
        ));
        assert_eq!(
            parse_args(["league-tui", "--bogus"]),
            CliAction::Error("unknown flag `--bogus` (try --help)".to_owned())
        );

        let mut out = Vec::new();
        let mut err = Vec::new();
        let code_help = dispatch_cli(
            ["league-tui".to_owned(), "--help".to_owned()],
            &mut out,
            &mut err,
            |_| Ok(()),
        );
        assert_eq!(code_help, ExitCode::SUCCESS);
        assert!(String::from_utf8_lossy(&out).contains("Usage: league-tui"));

        out.clear();
        let code_ver = dispatch_cli(
            ["league-tui".to_owned(), "--version".to_owned()],
            &mut out,
            &mut err,
            |_| Ok(()),
        );
        assert_eq!(code_ver, ExitCode::SUCCESS);
        assert!(String::from_utf8_lossy(&out).contains("league-tui"));

        let code_ok = dispatch_cli(["league-tui".to_owned()], &mut out, &mut err, |_| Ok(()));
        assert_eq!(code_ok, ExitCode::SUCCESS);

        err.clear();
        let code_tui_err = dispatch_cli(["league-tui".to_owned()], &mut out, &mut err, |_| {
            Err(io::Error::other("no tty"))
        });
        assert_eq!(code_tui_err, ExitCode::from(2));
        assert!(String::from_utf8_lossy(&err).contains("no tty"));

        err.clear();
        let code_bad = dispatch_cli(
            ["league-tui".to_owned(), "--bogus".to_owned()],
            &mut out,
            &mut err,
            |_| Ok(()),
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
        assert!(
            events.drain_calls >= 1,
            "run_session_loop must drain queued input on Curtain lift"
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
        })?;
        assert!(resumed_a.is_curtain_open());
        assert_eq!(resumed_a.observe_seat_call_count(), 0);
        terminal.draw(|f| {
            let _ = resumed_a.render_current_frame(f);
        })?;
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
        })?;
        assert!(resumed_b.is_curtain_open());
        assert_eq!(resumed_b.observe_seat_call_count(), 0);
        terminal.draw(|f| {
            let _ = resumed_b.render_current_frame(f);
        })?;
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
        })?;
        assert!(resumed.is_curtain_open());
        assert_eq!(resumed.observe_seat_call_count(), 0);
        terminal.draw(|f| {
            let _ = resumed.render_current_frame(f);
        })?;
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
        assert_eq!(resumed.observe_seat_call_count(), 0);

        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
        terminal.draw(|f| {
            let _ = resumed.render_current_frame(f);
        })?;
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
        assert!(timeout_events.drain_calls >= 1);
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
        assert!(pass_end_events.drain_calls >= 1);
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
}
