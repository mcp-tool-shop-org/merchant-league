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
use league_seat_ui::{Action, action_for, draw_title};
use ratatui::Terminal;
use ratatui::backend::{Backend, CrosstermBackend};

#[derive(Debug, Clone, PartialEq, Eq)]
enum CliAction {
    Help,
    Version,
    RunTitle,
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
    CliAction::RunTitle
}

fn run_title_loop<B: Backend>(
    terminal: &mut Terminal<B>,
    events: &mut dyn FnMut() -> io::Result<Option<Event>>,
) -> io::Result<()>
where
    B::Error: std::fmt::Display,
{
    loop {
        terminal
            .draw(draw_title)
            .map_err(|err| io::Error::other(err.to_string()))?;
        if let Some(Event::Key(key)) = events()?
            && action_for(key) == Action::Quit
        {
            return Ok(());
        }
    }
}

fn run_interactive() -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let res = run_title_loop(&mut terminal, &mut || event::read().map(Some));
    disable_raw_mode()?;
    crossterm::execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
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
        CliAction::RunTitle => match run_tui() {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                let _ = writeln!(err_out, "error: {err}");
                ExitCode::from(2)
            }
        },
        CliAction::Error(msg) => {
            let _ = writeln!(err_out, "error: {msg}");
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
        assert_eq!(parse_args(["league-tui"]), CliAction::RunTitle);
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
}
