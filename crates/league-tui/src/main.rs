//! Merchant League terminal host binary.
//!
//! Owns the Club Mode host session (`host::TuiHostSession`) and drives `league-seat-ui`
//! rendering over an interactive crossterm/ratatui terminal.

mod host;

use std::io;
use std::process::ExitCode;

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
use league_seat_ui::{Action, action_for};
use ratatui::Terminal;
use ratatui::backend::{Backend, CrosstermBackend};

#[derive(Debug, Clone, PartialEq, Eq)]
enum CliAction {
    Help,
    Version,
    RunClub,
    Error(String),
}

fn parse_args<I, S>(args: I) -> CliAction
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    if let Some(arg) = args.into_iter().nth(1) {
        return match arg.as_ref() {
            "-h" | "--help" => CliAction::Help,
            "-V" | "--version" => CliAction::Version,
            other => CliAction::Error(format!("unknown flag `{other}` (try --help)")),
        };
    }
    CliAction::RunClub
}

struct TerminalCleanup;

impl Drop for TerminalCleanup {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = crossterm::execute!(stdout, LeaveAlternateScreen);
    }
}

#[allow(dead_code)]
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
        if let Some(Event::Key(key)) = events()?
            && action_for(host::map_crossterm_key(key)) == Action::Quit
        {
            return Ok(());
        }
    }
}

fn run_session_loop<B: Backend>(
    terminal: &mut Terminal<B>,
    session: &mut host::TuiHostSession,
    events: &mut dyn FnMut() -> io::Result<Option<Event>>,
) -> io::Result<()>
where
    B::Error: std::fmt::Display,
{
    loop {
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
        if session.quit_requested() || session.phase() != TurnPhase::TalkAndMarket {
            return Ok(());
        }
        let area = terminal
            .size()
            .map_err(|err| io::Error::other(scrub_secret_text(&err.to_string())))?;
        let vp = ViewportSize::new(area.width, area.height);
        if let Some(Event::Key(key)) = events()? {
            let mapped = host::map_crossterm_key(key);
            let _ = session.handle_key_event(mapped, vp);
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

fn run_interactive() -> io::Result<()> {
    let ring = default_dev_ring_table()?;
    let mut session = host::TuiHostSession::from_ring_table(&ring, TableId(1), &[0], 1)
        .map_err(|err| io::Error::other(scrub_secret_text(&err.to_string())))?;
    enable_raw_mode()?;
    let _cleanup = TerminalCleanup;
    let mut stdout = io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let res = run_session_loop(&mut terminal, &mut session, &mut || event::read().map(Some));
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
    F: FnOnce() -> io::Result<()>,
{
    match parse_args(args) {
        CliAction::Help => {
            let _ = writeln!(
                out,
                "league-tui {} - Merchant League XRPL Testnet host",
                env!("CARGO_PKG_VERSION")
            );
            let _ = writeln!(out, "Usage: league-tui [-h|--help] [-V|--version]");
            ExitCode::SUCCESS
        }
        CliAction::Version => {
            let _ = writeln!(out, "league-tui {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        CliAction::RunClub => match run_tui() {
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
    use ratatui::backend::TestBackend;

    #[test]
    fn cli_args_parse_and_dispatch_all_paths() {
        assert_eq!(parse_args(["league-tui", "--help"]), CliAction::Help);
        assert_eq!(parse_args(["league-tui", "-h"]), CliAction::Help);
        assert_eq!(parse_args(["league-tui", "--version"]), CliAction::Version);
        assert_eq!(parse_args(["league-tui", "-V"]), CliAction::Version);
        assert_eq!(parse_args(["league-tui"]), CliAction::RunClub);
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
            || Ok(()),
        );
        assert_eq!(code_help, ExitCode::SUCCESS);
        assert!(String::from_utf8_lossy(&out).contains("Usage: league-tui"));

        out.clear();
        let code_ver = dispatch_cli(
            ["league-tui".to_owned(), "--version".to_owned()],
            &mut out,
            &mut err,
            || Ok(()),
        );
        assert_eq!(code_ver, ExitCode::SUCCESS);
        assert!(String::from_utf8_lossy(&out).contains("league-tui"));

        let code_ok = dispatch_cli(["league-tui".to_owned()], &mut out, &mut err, || Ok(()));
        assert_eq!(code_ok, ExitCode::SUCCESS);

        err.clear();
        let code_tui_err = dispatch_cli(["league-tui".to_owned()], &mut out, &mut err, || {
            Err(io::Error::other("no tty"))
        });
        assert_eq!(code_tui_err, ExitCode::from(2));
        assert!(String::from_utf8_lossy(&err).contains("no tty"));

        err.clear();
        let code_bad = dispatch_cli(
            ["league-tui".to_owned(), "--bogus".to_owned()],
            &mut out,
            &mut err,
            || Ok(()),
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
        let mut seq = vec![Event::Key(KeyEvent::new(
            KeyCode::Char('q'),
            KeyModifiers::NONE,
        ))]
        .into_iter();
        run_session_loop(&mut terminal, &mut session, &mut || Ok(seq.next()))?;
        assert!(session.quit_requested());
        Ok(())
    }
}
