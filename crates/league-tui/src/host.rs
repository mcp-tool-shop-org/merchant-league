//! Trusted Club Mode host session wrapper in the `league-tui` binary crate.
//!
//! Owns [`ClubHost`], save/seal persistence, and `EvalVrf` sweeps, builds one per-seat
//! [`SeatObservation`] by value carrying strictly that seat's `own_mandate`, and renders via
//! `league-seat-ui` without placing any seat logic in `league-tui`.

use std::collections::BTreeMap;
use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use league_core::{
    ClubHost, EngineError, OrderReveal, PublicSweepReceipt, Seat, SeatObservation, TableId,
    TurnPhase,
};
use league_gen::{GenError, RingTable, ring_table_to_table_spec};
use league_seat_types::{
    CommodityDisplayMap, SeatKeyCode, SeatKeyEvent, SeatKeyModifiers, SeatScreenView, ViewAlign,
    ViewColor, ViewPanel, ViewStyle,
};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

/// Converts a host `crossterm` [`KeyEvent`] into a pure seat key event value.
#[must_use]
pub(crate) fn map_crossterm_key(key: KeyEvent) -> SeatKeyEvent {
    let code = match key.code {
        KeyCode::Char(ch) => SeatKeyCode::Char(ch),
        KeyCode::Enter => SeatKeyCode::Enter,
        KeyCode::Esc => SeatKeyCode::Esc,
        KeyCode::Tab => SeatKeyCode::Tab,
        KeyCode::BackTab => SeatKeyCode::BackTab,
        KeyCode::Backspace => SeatKeyCode::Backspace,
        KeyCode::Delete => SeatKeyCode::Delete,
        KeyCode::Up => SeatKeyCode::Up,
        KeyCode::Down => SeatKeyCode::Down,
        KeyCode::Left => SeatKeyCode::Left,
        KeyCode::Right => SeatKeyCode::Right,
        KeyCode::Home => SeatKeyCode::Home,
        KeyCode::End => SeatKeyCode::End,
        KeyCode::PageUp => SeatKeyCode::PageUp,
        KeyCode::PageDown => SeatKeyCode::PageDown,
        KeyCode::F(n) => SeatKeyCode::F(n),
        _ => SeatKeyCode::Unmapped,
    };
    SeatKeyEvent::new(
        code,
        SeatKeyModifiers {
            ctrl: key.modifiers.contains(KeyModifiers::CONTROL),
            alt: key.modifiers.contains(KeyModifiers::ALT),
            shift: key.modifiers.contains(KeyModifiers::SHIFT),
        },
    )
}

const fn map_color(color: ViewColor) -> Color {
    match color {
        ViewColor::Reset => Color::Reset,
        ViewColor::Black => Color::Black,
        ViewColor::Red => Color::Red,
        ViewColor::Green => Color::Green,
        ViewColor::Yellow => Color::Yellow,
        ViewColor::Blue => Color::Blue,
        ViewColor::Magenta => Color::Magenta,
        ViewColor::Cyan => Color::Cyan,
        ViewColor::Gray => Color::Gray,
        ViewColor::DarkGray => Color::DarkGray,
        ViewColor::LightRed => Color::LightRed,
        ViewColor::LightGreen => Color::LightGreen,
        ViewColor::LightYellow => Color::LightYellow,
        ViewColor::LightBlue => Color::LightBlue,
        ViewColor::LightMagenta => Color::LightMagenta,
        ViewColor::LightCyan => Color::LightCyan,
        ViewColor::White => Color::White,
        ViewColor::Rgb { r, g, b } => Color::Rgb(r, g, b),
    }
}

fn map_style(style: ViewStyle) -> Style {
    let mut out = Style::default()
        .fg(map_color(style.fg))
        .bg(map_color(style.bg));
    if style.bold {
        out = out.add_modifier(Modifier::BOLD);
    }
    if style.dim {
        out = out.add_modifier(Modifier::DIM);
    }
    if style.italic {
        out = out.add_modifier(Modifier::ITALIC);
    }
    if style.underlined {
        out = out.add_modifier(Modifier::UNDERLINED);
    }
    if style.reversed {
        out = out.add_modifier(Modifier::REVERSED);
    }
    out
}

const fn map_align(align: ViewAlign) -> Alignment {
    match align {
        ViewAlign::Left => Alignment::Left,
        ViewAlign::Center => Alignment::Center,
        ViewAlign::Right => Alignment::Right,
    }
}

fn render_view_panel(frame: &mut Frame<'_>, area: Rect, panel: &ViewPanel) {
    let lines: Vec<Line<'_>> = panel
        .rows
        .iter()
        .enumerate()
        .map(|(idx, row)| {
            let is_selected = panel.selected_row == Some(idx);
            let spans: Vec<Span<'_>> = row
                .spans
                .iter()
                .map(|span| {
                    let mut st = map_style(span.style);
                    if is_selected {
                        st = st.add_modifier(Modifier::REVERSED);
                    }
                    Span::styled(span.text.as_str(), st)
                })
                .collect();
            Line::from(spans)
        })
        .collect();
    let mut paragraph = Paragraph::new(Text::from(lines)).alignment(map_align(panel.alignment));
    if panel.bordered {
        let mut block = Block::default().borders(Borders::ALL);
        if let Some(title) = &panel.title {
            block = block.title(Span::styled(title.text.as_str(), map_style(title.style)));
        }
        paragraph = paragraph.block(block);
    }
    frame.render_widget(paragraph, area);
}

/// Draws a plain [`SeatScreenView`] view model emitted by `league-seat-ui` using `ratatui`.
pub(crate) fn draw_screen_view(frame: &mut Frame<'_>, view: &SeatScreenView) {
    let full_area = frame.area();
    let body_area = if let Some(header) = &view.header {
        let border_rows = if header.bordered { 2_u16 } else { 0_u16 };
        let header_height = u16::try_from(header.rows.len())
            .unwrap_or(1)
            .saturating_add(border_rows)
            .min(full_area.height);
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(header_height), Constraint::Min(0)])
            .split(full_area);
        render_view_panel(frame, chunks[0], header);
        chunks[1]
    } else {
        full_area
    };

    if view.side_panels.is_empty() {
        render_view_panel(frame, body_area, &view.main);
    } else {
        let panes = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
            .split(body_area);
        render_view_panel(frame, panes[0], &view.main);
        let side_constraints =
            vec![
                Constraint::Ratio(1, u32::try_from(view.side_panels.len()).unwrap_or(1));
                view.side_panels.len()
            ];
        let side_areas = Layout::default()
            .direction(Direction::Vertical)
            .constraints(side_constraints)
            .split(panes[1]);
        for (side_area, panel) in side_areas.iter().copied().zip(view.side_panels.iter()) {
            render_view_panel(frame, side_area, panel);
        }
    }

    if let Some(modal) = &view.modal {
        let w = (full_area.width * 3 / 4).max(20).min(full_area.width);
        let h = (full_area.height * 3 / 4).max(5).min(full_area.height);
        let modal_rect = Rect {
            x: full_area.x + full_area.width.saturating_sub(w) / 2,
            y: full_area.y + full_area.height.saturating_sub(h) / 2,
            width: w,
            height: h,
        };
        frame.render_widget(Clear, modal_rect);
        render_view_panel(frame, modal_rect, modal);
    }
}

/// Host session bridging a `ClubHost` table to seat-isolated `league-seat-ui` views.
#[derive(Debug)]
#[allow(dead_code)]
pub struct TuiHostSession {
    host: ClubHost,
    commodities: Option<CommodityDisplayMap>,
}

#[allow(dead_code)]
impl TuiHostSession {
    /// Bootstraps a Club Mode session on the canonical `TableSpec::standard_club_chiral_ring_with_setup`.
    pub fn from_standard_setup(
        table_id: TableId,
        seat_count: u8,
        layout_seed: u64,
        bot_tie_seed: u64,
    ) -> Result<Self, EngineError> {
        let host = ClubHost::from_standard_setup(table_id, seat_count, layout_seed, bot_tie_seed)?;
        Ok(Self {
            host,
            commodities: None,
        })
    }

    /// Promotes a feasibility-checked `SeedPool::Dev` [`RingTable`] into a live Club Mode session
    /// with its derived [`CommodityDisplayMap`].
    pub fn from_ring_table(
        table: &RingTable,
        table_id: TableId,
        human_seats: &[u8],
        bot_tie_seed: u64,
    ) -> Result<Self, GenError> {
        let (spec, cmap, deal, eval_seed) = ring_table_to_table_spec(table, table_id, human_seats)?;
        let host = ClubHost::new(spec, table.seed, deal, eval_seed, bot_tie_seed)
            .map_err(|err| GenError::InvalidConfig(err.to_string()))?;
        Ok(Self {
            host,
            commodities: Some(cmap.to_display_map()),
        })
    }

    /// Returns the number of seats at the table (`4..=6`).
    #[must_use]
    pub const fn seat_count(&self) -> u8 {
        self.host.seat_count()
    }

    /// Returns the active turn number (`1..=32`).
    #[must_use]
    pub const fn turn(&self) -> u8 {
        self.host.turn()
    }

    /// Returns the current [`TurnPhase`].
    #[must_use]
    pub const fn phase(&self) -> TurnPhase {
        self.host.phase()
    }

    /// Returns the seat-safe 30-field live [`PublicSweepReceipt`] history.
    #[must_use]
    pub fn public_receipts(&self) -> &[PublicSweepReceipt] {
        self.host.public_receipts()
    }

    /// Builds a single [`SeatObservation`] **by value** for `seat`, carrying strictly `seat`'s
    /// own dealt `PrivateMandate` and never rival seats' mandates or hidden engine state.
    pub fn build_seat_observation(&self, seat: Seat) -> Result<SeatObservation, EngineError> {
        self.host.observe_seat(seat)
    }

    /// Builds one [`SeatObservation`] **by value** per seat `0..seat_count` in ascending order,
    /// each carrying strictly that seat's own `PrivateMandate`.
    pub fn build_all_seat_observations(&self) -> Result<Vec<SeatObservation>, EngineError> {
        let count = self.seat_count();
        let mut out = Vec::with_capacity(usize::from(count));
        for s in 0..count {
            out.push(self.build_seat_observation(Seat(s))?);
        }
        Ok(out)
    }

    /// Runs `TalkAndMarket -> Commit -> Reveal -> sweep_turn` atomically on the host.
    pub fn commit_reveal_and_sweep(
        &mut self,
        reveals: &BTreeMap<u8, OrderReveal>,
    ) -> Result<PublicSweepReceipt, EngineError> {
        self.host.commit_and_reveal_all(reveals)?;
        self.host.sweep_turn()
    }

    /// Atomically persists `.save.json` and `.seal` sidecar files on the host.
    pub fn write_save_and_seal(
        &self,
        save_path: &Path,
        seal_path: &Path,
    ) -> Result<(), EngineError> {
        self.host.write_save_and_seal(save_path, seal_path)
    }

    /// Reloads a persisted Club Mode host session from `.save.json` and `.seal`.
    pub fn read_save_and_seal(
        save_path: &Path,
        seal_path: &Path,
        commodities: Option<CommodityDisplayMap>,
    ) -> Result<Self, EngineError> {
        let host = ClubHost::read_save_and_seal(save_path, seal_path)?;
        Ok(Self { host, commodities })
    }

    /// Wipes a finished `.seal` sidecar cleanly.
    pub fn wipe_seal_file(seal_path: &Path) -> Result<(), EngineError> {
        ClubHost::wipe_seal_file(seal_path)
    }

    /// Builds one [`SeatObservation`] **by value** for `seat`, requests the plain view model from
    /// `league-seat-ui`, and draws it onto `frame` via `ratatui`.
    pub fn render_seat_frame(
        &self,
        frame: &mut Frame<'_>,
        seat: Seat,
        plain_codes: bool,
    ) -> Result<(), EngineError> {
        let obs = self.build_seat_observation(seat)?;
        let view = match &self.commodities {
            Some(map) => league_seat_ui::render_seat_with_commodities(&obs, map, plain_codes),
            None => league_seat_ui::render_seat(&obs),
        };
        draw_screen_view(frame, &view);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use league_gen::{
        GeneratorConfig, SeatCount, SeedPool, TablePurpose, TableSetupOptions,
        generate_ring_table_for_setup, is_reserved_test_seed, seed_pool,
    };
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn dev_ring_table(start_seed: u64) -> RingTable {
        let cfg = GeneratorConfig {
            seat_count: Some(SeatCount::Four),
            ..GeneratorConfig::default()
        };
        let opts = TableSetupOptions {
            purpose: TablePurpose::Dev,
            training_logged: false,
        };
        for seed in start_seed..1_000 {
            if is_reserved_test_seed(seed) || seed_pool(seed) != SeedPool::Dev {
                continue;
            }
            if let Ok((checked, _, _)) = generate_ring_table_for_setup(seed, opts, &cfg)
                && let Some(table) = checked.into_clear_table()
            {
                return table;
            }
        }
        panic!("no feasible dev table found from seed {start_seed}");
    }

    fn screen_text(terminal: &Terminal<TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn must_ok<T, E: std::fmt::Display>(res: Result<T, E>, ctx: &str) -> T {
        match res {
            Ok(v) => v,
            Err(err) => panic!("{ctx}: {err}"),
        }
    }

    fn must_some<T>(opt: Option<T>, ctx: &str) -> T {
        match opt {
            Some(v) => v,
            None => panic!("expected Some for {ctx}"),
        }
    }

    #[test]
    fn host_session_isolates_per_seat_observations_sweeps_persists_and_renders() {
        let mut session = must_ok(
            TuiHostSession::from_standard_setup(TableId(81_001), 4, 1, 7),
            "standard club setup",
        );
        assert_eq!(session.seat_count(), 4);
        assert_eq!(session.turn(), 1);
        assert_eq!(session.phase(), TurnPhase::TalkAndMarket);
        assert!(session.public_receipts().is_empty());

        let all_obs = must_ok(
            session.build_all_seat_observations(),
            "all seat observations",
        );
        assert_eq!(all_obs.len(), 4);
        for (s, obs) in all_obs.iter().enumerate() {
            assert_eq!(usize::from(obs.seat), s);
            let own = must_some(obs.own_mandate(), "own_mandate attached");
            let own_dbg = format!("{own:?}");
            let obs_dbg = format!("{obs:?}");
            assert!(obs_dbg.contains(&own_dbg));
            for (r, rival_obs) in all_obs.iter().enumerate() {
                if r != s {
                    let rival_mandate = must_some(rival_obs.own_mandate(), "rival mandate");
                    if rival_mandate != own {
                        let rival_dbg = format!("{rival_mandate:?}");
                        assert!(
                            !obs_dbg.contains(&rival_dbg),
                            "seat {s} observation leaked rival seat {r}'s mandate"
                        );
                    }
                }
            }
        }

        let mut terminal = must_ok(Terminal::new(TestBackend::new(80, 10)), "TestBackend");
        must_ok(
            terminal.draw(|frame| {
                must_ok(
                    session.render_seat_frame(frame, Seat(0), false),
                    "render seat 0",
                );
            }),
            "draw seat 0",
        );
        let text_std = screen_text(&terminal);
        assert!(text_std.contains("Table #81001 | Turn 1"));
        assert!(text_std.contains("Seat 0 View"));

        let receipt = must_ok(
            session.commit_reveal_and_sweep(&BTreeMap::new()),
            "commit reveal sweep",
        );
        assert_eq!(receipt.turn, 1);
        assert_eq!(session.turn(), 2);
        assert_eq!(session.public_receipts().len(), 1);

        let dir = std::env::temp_dir().join(format!("ml_tui_host_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let save_path = dir.join("tui-session.save.json");
        let seal_path = dir.join("tui-session.seal");
        must_ok(
            session.write_save_and_seal(&save_path, &seal_path),
            "write save and seal",
        );
        let reloaded = must_ok(
            TuiHostSession::read_save_and_seal(&save_path, &seal_path, None),
            "reload",
        );
        assert_eq!(reloaded.turn(), 2);
        assert_eq!(reloaded.public_receipts().len(), 1);
        must_ok(TuiHostSession::wipe_seal_file(&seal_path), "wipe seal");
        assert!(!seal_path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn host_session_from_ring_table_renders_commodity_names_and_plain_codes() {
        let ring = dev_ring_table(1);
        let session = must_ok(
            TuiHostSession::from_ring_table(&ring, TableId(81_002), &[0], 11),
            "ring table session",
        );
        let mut terminal = must_ok(Terminal::new(TestBackend::new(84, 10)), "TestBackend");
        must_ok(
            terminal.draw(|frame| {
                must_ok(
                    session.render_seat_frame(frame, Seat(0), false),
                    "render in-world names",
                );
            }),
            "draw in-world names",
        );
        let in_world = screen_text(&terminal);
        assert!(in_world.contains("Clearinghouse Marks [CRED]"));
        assert!(in_world.contains('['));

        must_ok(
            terminal.draw(|frame| {
                must_ok(
                    session.render_seat_frame(frame, Seat(0), true),
                    "render plain codes",
                );
            }),
            "draw plain codes",
        );
        let plain = screen_text(&terminal);
        assert!(plain.contains("CRED:"));
        assert!(!plain.contains("Clearinghouse Marks [CRED]"));
    }

    #[test]
    fn host_draws_rich_view_models_and_maps_crossterm_keys() {
        use league_seat_types::{ViewLabel, ViewRow, ViewSpan};

        for (raw_code, expected) in [
            (KeyCode::Char('q'), SeatKeyCode::Char('q')),
            (KeyCode::Enter, SeatKeyCode::Enter),
            (KeyCode::Esc, SeatKeyCode::Esc),
            (KeyCode::Tab, SeatKeyCode::Tab),
            (KeyCode::BackTab, SeatKeyCode::BackTab),
            (KeyCode::Backspace, SeatKeyCode::Backspace),
            (KeyCode::Delete, SeatKeyCode::Delete),
            (KeyCode::Up, SeatKeyCode::Up),
            (KeyCode::Down, SeatKeyCode::Down),
            (KeyCode::Left, SeatKeyCode::Left),
            (KeyCode::Right, SeatKeyCode::Right),
            (KeyCode::Home, SeatKeyCode::Home),
            (KeyCode::End, SeatKeyCode::End),
            (KeyCode::PageUp, SeatKeyCode::PageUp),
            (KeyCode::PageDown, SeatKeyCode::PageDown),
            (KeyCode::F(12), SeatKeyCode::F(12)),
            (KeyCode::Insert, SeatKeyCode::Unmapped),
        ] {
            let ev = map_crossterm_key(KeyEvent::new(
                raw_code,
                KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT,
            ));
            assert_eq!(ev.code, expected);
            assert!(ev.modifiers.ctrl && ev.modifiers.alt && ev.modifiers.shift);
        }

        let all_colors = [
            ViewColor::Reset,
            ViewColor::Black,
            ViewColor::Red,
            ViewColor::Green,
            ViewColor::Yellow,
            ViewColor::Blue,
            ViewColor::Magenta,
            ViewColor::Cyan,
            ViewColor::Gray,
            ViewColor::DarkGray,
            ViewColor::LightRed,
            ViewColor::LightGreen,
            ViewColor::LightYellow,
            ViewColor::LightBlue,
            ViewColor::LightMagenta,
            ViewColor::LightCyan,
            ViewColor::White,
            ViewColor::Rgb { r: 1, g: 2, b: 3 },
        ];
        for color in all_colors {
            let mut st = ViewStyle::plain().with_fg(color).with_bg(color);
            st.italic = true;
            let mapped = map_style(st);
            assert_eq!(mapped.fg, Some(map_color(color)));
        }

        let styled = ViewStyle::plain()
            .with_fg(ViewColor::Cyan)
            .with_bg(ViewColor::Black)
            .bold()
            .dim()
            .underlined()
            .reversed();
        let rich = SeatScreenView {
            header: Some(ViewPanel {
                title: None,
                alignment: ViewAlign::Right,
                bordered: false,
                rows: vec![ViewRow::from_plain("TOP HEADER".to_owned())],
                selected_row: None,
            }),
            main: ViewPanel {
                title: Some(ViewLabel::styled("Main Panel".to_owned(), styled)),
                alignment: ViewAlign::Left,
                bordered: true,
                rows: vec![
                    ViewRow::from_spans(vec![ViewSpan::styled("SELECTED".to_owned(), styled)]),
                    ViewRow::from_plain("SECOND".to_owned()),
                ],
                selected_row: Some(0),
            },
            side_panels: vec![ViewPanel {
                title: Some(ViewLabel::plain("Side".to_owned())),
                alignment: ViewAlign::Center,
                bordered: true,
                rows: vec![ViewRow::from_plain("SIDE TEXT".to_owned())],
                selected_row: None,
            }],
            modal: Some(ViewPanel {
                title: Some(ViewLabel::plain("Modal".to_owned())),
                alignment: ViewAlign::Center,
                bordered: true,
                rows: vec![ViewRow::from_plain("MODAL BODY".to_owned())],
                selected_row: None,
            }),
        };

        let mut terminal = must_ok(Terminal::new(TestBackend::new(80, 16)), "TestBackend");
        must_ok(
            terminal.draw(|frame| draw_screen_view(frame, &rich)),
            "draw rich view model",
        );
        let txt = screen_text(&terminal);
        assert!(txt.contains("TOP HEADER"));
        assert!(txt.contains("MODAL BODY"));
    }
}
