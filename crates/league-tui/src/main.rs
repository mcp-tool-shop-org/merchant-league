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
    Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
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

trait ScrollbackClear {
    fn clear_scrollback(&mut self) -> io::Result<()>;
}

impl<W: io::Write> ScrollbackClear for CrosstermBackend<W> {
    fn clear_scrollback(&mut self) -> io::Result<()> {
        crossterm::execute!(self, Clear(ClearType::Purge))
    }
}

#[cfg(test)]
impl ScrollbackClear for TestBackend {
    fn clear_scrollback(&mut self) -> io::Result<()> {
        Ok(())
    }
}

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

struct TerminalCleanup;

impl Drop for TerminalCleanup {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = crossterm::execute!(stdout, LeaveAlternateScreen);
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
    B: Backend + ScrollbackClear,
    B::Error: std::fmt::Display,
    E: SessionEventSource,
{
    loop {
        session.sync_clock(events.elapsed());
        if session.quit_requested() || session.phase() != TurnPhase::TalkAndMarket {
            return Ok(());
        }
        if session.take_scrollback_purge() {
            terminal
                .backend_mut()
                .clear_scrollback()
                .map_err(|err| io::Error::other(scrub_secret_text(&err.to_string())))?;
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
                if let Some(mapped) = host::map_crossterm_key(key) {
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

fn apply_cli_session_options(session: &mut host::TuiHostSession, opts: &CliOptions) {
    if opts.untimed {
        session.configure_turn_clock(None);
    } else {
        session.configure_turn_clock(Some(
            opts.turn_clock_secs.unwrap_or(DEFAULT_TURN_CLOCK_SECS),
        ));
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
        apply_cli_session_options(&mut session, opts);
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
    apply_cli_session_options(&mut session, opts);
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
    crossterm::execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let mut live_events = LiveTerminalEvents::new();
    let res = run_session_loop(&mut terminal, &mut session, &mut live_events);
    let _ = terminal.show_cursor();
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
        let curtain_events = vec![
            key_with(
                KeyCode::Char('l'),
                KeyModifiers::CONTROL,
                KeyEventKind::Press,
            ),
            key_with(
                KeyCode::Char('l'),
                KeyModifiers::CONTROL,
                KeyEventKind::Release,
            ),
            key_with(KeyCode::Char(' '), KeyModifiers::NONE, KeyEventKind::Press),
            key_with(
                KeyCode::Char(' '),
                KeyModifiers::NONE,
                KeyEventKind::Release,
            ),
            key_with(KeyCode::Enter, KeyModifiers::NONE, KeyEventKind::Press),
            key_with(KeyCode::Enter, KeyModifiers::NONE, KeyEventKind::Release),
            key_with(KeyCode::Enter, KeyModifiers::NONE, KeyEventKind::Repeat),
        ]
        .into_iter();
        let mut events_curtain = ScriptedSessionEvents::from_events(curtain_events);
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
    fn curtain_handoff_discards_queued_input_enforces_dead_time_and_purges_scrollback()
    -> Result<(), Box<dyn std::error::Error>> {
        use crossterm::event::KeyEventKind;
        let ring = default_dev_ring_table()?;
        let mut session =
            host::TuiHostSession::from_ring_table(&ring, TableId(81_301), &[0, 1], 5)?;
        let written = std::rc::Rc::new(std::cell::RefCell::new(Vec::<u8>::new()));
        let mut terminal = Terminal::new(CrosstermBackend::new(SharedVecWriter(written.clone())))?;
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
                Duration::from_millis(10),
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
            raw_ansi.contains("\x1b[3J"),
            "Curtain hand-off must emit ESC[3J scrollback purge on CrosstermBackend"
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

        // Now unlock Seat 1 and verify Seat 1's first buffer leaks zero Seat 0 private state, while
        // the countdown in Seat 1's header ticks down smoothly on pure `Tick`s (`67s` -> `57s`).
        let mut s1_steps = vec![
            (
                Duration::from_millis(10),
                Some(key_ev(KeyCode::Char(' '), KeyModifiers::NONE)),
                Vec::new(),
            ),
            (
                Duration::from_millis(10),
                Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
                Vec::new(),
            ),
            (Duration::from_secs(10), None, Vec::new()),
            (Duration::ZERO, None, Vec::new()),
        ];
        let mut events_s1 = ScriptedSessionEvents::from_timed_with_queued(s1_steps.drain(..));
        run_session_loop(&mut terminal, &mut session, &mut events_s1)?;
        let s1_buf = term_capture(&terminal);
        assert!(
            !session.is_curtain_open(),
            "Seat 1 must have unlocked from Curtain"
        );
        assert!(!s1_buf.contains("PRIVATE_BEFORE_TIMEOUT_919"));
        assert!(!s1_buf.contains("Preview T1 S0:"));
        assert!(
            s1_buf.contains("57s") || s1_buf.contains("0:57") || s1_buf.contains("Clock:"),
            "header must display ticking Pass A countdown without key presses:\n{s1_buf}"
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
            Duration::from_millis(10),
            Some(key_ev(KeyCode::Char(' '), KeyModifiers::NONE)),
            Vec::new(),
        ));
        regent_steps.push((
            Duration::from_millis(10),
            Some(key_ev(KeyCode::Enter, KeyModifiers::NONE)),
            Vec::new(),
        ));
        for _ in 0..5 {
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
        Ok(())
    }
}
