//! Merchant League terminal client, built on ratatui and crossterm.
//!
//! **A stub.** It draws the title screen and maps keys to actions, so the
//! client's shape is fixed and tested. The game screens arrive with the rules,
//! which wait for the outside design review to be ruled on.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line, Text};
use ratatui::widgets::{Block, Borders, Paragraph};

/// The title screen's lines, top to bottom.
pub const TITLE_LINES: [&str; 3] = [
    "Merchant League",
    "A trading game on the XRPL Testnet. No real money.",
    "Not playable yet. Press q to quit.",
];

/// What a key press asks the client to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Leave the client.
    Quit,
    /// Nothing: the key has no meaning on this screen.
    Ignore,
}

/// Map a key press to an action. `q`, `Esc` and Ctrl-C quit.
pub fn action_for(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => Action::Quit,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Action::Quit,
        _ => Action::Ignore,
    }
}

/// Draw the title screen across the whole frame.
pub fn draw_title(frame: &mut Frame<'_>) {
    let area: Rect = frame.area();
    let text = Text::from(
        TITLE_LINES
            .iter()
            .map(|l| Line::from(*l))
            .collect::<Vec<_>>(),
    );
    let title = Paragraph::new(text)
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL));
    frame.render_widget(title, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn q_esc_and_ctrl_c_quit() {
        assert_eq!(
            action_for(key(KeyCode::Char('q'), KeyModifiers::NONE)),
            Action::Quit
        );
        assert_eq!(
            action_for(key(KeyCode::Esc, KeyModifiers::NONE)),
            Action::Quit
        );
        assert_eq!(
            action_for(key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Action::Quit
        );
    }

    #[test]
    fn other_keys_are_ignored() {
        assert_eq!(
            action_for(key(KeyCode::Char('c'), KeyModifiers::NONE)),
            Action::Ignore
        );
        assert_eq!(
            action_for(key(KeyCode::Enter, KeyModifiers::NONE)),
            Action::Ignore
        );
    }

    #[test]
    fn the_title_screen_says_testnet_and_not_playable() -> Result<(), Box<dyn std::error::Error>> {
        let mut terminal = Terminal::new(TestBackend::new(60, 7))?;
        terminal.draw(draw_title)?;
        let buffer = terminal.backend().buffer();
        let screen: String = (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(screen.contains("Merchant League"));
        assert!(screen.contains("XRPL Testnet. No real money."));
        assert!(screen.contains("Not playable yet"));
        Ok(())
    }
}
