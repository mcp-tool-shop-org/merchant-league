//! Trusted Club Mode host session wrapper in the `league-tui` binary crate.
//!
//! Owns [`ClubHost`], two-pass hot-seat sequencing, `ClubBot` automation, strict host validation
//! of [`SeatIntent`]s, and generic [`ScreenLayout`] rendering via `ratatui` without putting any
//! seat view-model logic or forbidden engine symbols into `league-tui`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use league_core::SeatIdentity;
use league_core::club::host::{ClubPauseCursor, ClubSessionCheckpoint};
use league_core::{
    AiPreviewRequest, ClubBot, ClubHost, EngineError, MarketAction, OrderReveal, SealedOrderBundle,
    Seat, SeatObservation, StructuredError, TableId, ToStructuredError, Turn, TurnPhase,
    preview_seat_bundle, preview_what_if, scrub_secret_text,
};
#[cfg(test)]
use league_core::{PublicSweepReceipt, RevealSalt, SealedEvalSeed, SealedMandateDeal, TableSpec};
use league_gen::{GenError, RingTable, ring_table_to_table_spec};
use league_seat_types::{
    CommodityDisplayMap, CurtainPinMode, CurtainPinPrompt, SeatIntent, SeatIntentKind, SeatKeyCode,
    SeatKeyEvent, SeatKeyModifiers, SeatScreenView, TurnClockDisplay, ViewAlign, ViewAxis,
    ViewColor, ViewLayoutMode, ViewPanel, ViewRow, ViewSizeHint, ViewSpan, ViewStyle, ViewTabBar,
    ViewTable, ViewZoneId, ViewportSize, WhatIfPreview,
};
use league_seat_ui::{PassKind, SeatUiState};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table};

/// Post-lift input dead time enforced after any Privacy Curtain unlocks so buffered or
/// held keystrokes immediately following `[Enter]` do not leak into the arriving seat.
pub(crate) const CURTAIN_LIFT_DEAD_TIME: Duration = Duration::from_millis(400);

#[derive(Debug, Clone, PartialEq, Eq)]
struct CurtainPinEntry {
    mode: CurtainPinMode,
    digits: String,
    wrong_tries: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CurtainState {
    next_seat: Seat,
    space_armed: bool,
    pin_entry: Option<CurtainPinEntry>,
    locked_out_reset: bool,
}

impl CurtainState {
    const fn fresh(next_seat: Seat) -> Self {
        Self {
            next_seat,
            space_armed: false,
            pin_entry: None,
            locked_out_reset: false,
        }
    }
}

fn strip_terminal_controls(text: &str) -> String {
    text.chars().filter(|ch| !ch.is_control()).collect()
}

fn clip_text_right(text: &str, max_width: usize) -> String {
    let sanitized = strip_terminal_controls(text);
    if max_width == 0 {
        return String::new();
    }
    let count = sanitized.chars().count();
    if count <= max_width {
        return sanitized;
    }
    if max_width == 1 {
        return sanitized.chars().take(1).collect();
    }
    let mut out: String = sanitized.chars().take(max_width - 1).collect();
    out.push('~');
    out
}

/// Converts a host `crossterm` [`KeyEvent`] into a pure seat key event value, dropping
/// `KeyEventKind::Release` and blocking `KeyEventKind::Repeat` on commit/lock/quit keys.
#[must_use]
pub(crate) fn map_crossterm_key(key: KeyEvent) -> Option<SeatKeyEvent> {
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
    let mods = SeatKeyModifiers {
        ctrl: key.modifiers.contains(KeyModifiers::CONTROL),
        alt: key.modifiers.contains(KeyModifiers::ALT),
        shift: key.modifiers.contains(KeyModifiers::SHIFT),
    };
    match key.kind {
        KeyEventKind::Press => Some(SeatKeyEvent::new(code, mods)),
        KeyEventKind::Repeat => {
            if matches!(code, SeatKeyCode::Enter | SeatKeyCode::Esc)
                || (mods.ctrl && matches!(code, SeatKeyCode::Char('l' | 'L' | 'c' | 'C')))
            {
                None
            } else {
                Some(SeatKeyEvent::repeated(code, mods))
            }
        }
        KeyEventKind::Release => None,
    }
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

const fn map_constraint(hint: ViewSizeHint) -> Constraint {
    match hint {
        ViewSizeHint::Fixed(n) => Constraint::Length(n),
        ViewSizeHint::Min(n) => Constraint::Min(n),
        ViewSizeHint::Percentage(p) => Constraint::Percentage(p),
        ViewSizeHint::Ratio { num, den } => Constraint::Ratio(num, if den == 0 { 1 } else { den }),
    }
}

const fn map_axis(axis: ViewAxis) -> Direction {
    match axis {
        ViewAxis::Vertical => Direction::Vertical,
        ViewAxis::Horizontal => Direction::Horizontal,
    }
}

fn render_tab_bar(frame: &mut Frame<'_>, area: Rect, tab_bar: &ViewTabBar) -> u16 {
    if area.width == 0 || area.height == 0 || tab_bar.tabs.is_empty() {
        return 0;
    }
    let mut lines: Vec<Vec<Span<'_>>> = vec![Vec::new()];
    let mut line_width = 0_usize;
    let max_width = usize::from(area.width);

    for (idx, item) in tab_bar.tabs.iter().enumerate() {
        let chip = if idx == tab_bar.active_index {
            format!("[{}]", item.label.text)
        } else {
            format!(" {} ", item.label.text)
        };
        let sep_len = usize::from(!lines.last().is_some_and(Vec::is_empty)) * 3;
        if !lines.last().is_some_and(Vec::is_empty)
            && line_width + sep_len + chip.chars().count() > max_width
            && lines.len() < usize::from(area.height)
        {
            lines.push(Vec::new());
            line_width = 0;
        }
        if let Some(current) = lines.last_mut() {
            if !current.is_empty() {
                current.push(Span::raw(" | "));
                line_width += 3;
            }
            let mut st = map_style(item.label.style);
            if idx == tab_bar.active_index {
                st = st
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD | Modifier::REVERSED);
            } else if !item.enabled {
                st = st.fg(Color::DarkGray).add_modifier(Modifier::DIM);
            }
            let safe_chip = strip_terminal_controls(&chip);
            line_width += safe_chip.chars().count();
            current.push(Span::styled(safe_chip, st));
        }
    }

    let used_rows = u16::try_from(lines.len()).unwrap_or(1).min(area.height);
    let rendered_lines: Vec<Line<'_>> = lines
        .into_iter()
        .take(usize::from(used_rows))
        .map(Line::from)
        .collect();
    let tab_rect = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: used_rows,
    };
    frame.render_widget(Paragraph::new(Text::from(rendered_lines)), tab_rect);
    used_rows
}

fn render_view_table(
    frame: &mut Frame<'_>,
    area: Rect,
    table: &ViewTable,
    skip_rows: usize,
) -> u16 {
    if area.width == 0
        || area.height < 2
        || table.columns.is_empty()
        || skip_rows >= table.rows.len()
    {
        return 0;
    }
    let visible_rows = &table.rows[skip_rows..];
    let desired_rows = 1_usize.saturating_add(visible_rows.len());
    let allocated = u16::try_from(desired_rows)
        .unwrap_or(u16::MAX)
        .min(area.height);
    let widths: Vec<Constraint> = table
        .columns
        .iter()
        .map(|c| map_constraint(c.width))
        .collect();
    let table_rect = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: allocated,
    };
    let col_rects = Layout::default()
        .direction(Direction::Horizontal)
        .spacing(1)
        .constraints(widths.clone())
        .split(table_rect);
    let col_widths: Vec<usize> = col_rects.iter().map(|r| usize::from(r.width)).collect();

    let header_cells: Vec<Cell<'_>> = table
        .columns
        .iter()
        .enumerate()
        .map(|(col_idx, col)| {
            let max_w = col_widths
                .get(col_idx)
                .copied()
                .unwrap_or_else(|| usize::from(area.width));
            let text = clip_text_right(&col.header.text, max_w);
            Cell::from(
                Line::from(Span::styled(
                    text,
                    map_style(col.header.style).add_modifier(Modifier::BOLD),
                ))
                .alignment(map_align(col.alignment)),
            )
        })
        .collect();
    let header = Row::new(header_cells);
    let data_limit = usize::from(allocated.saturating_sub(1));
    let data_rows: Vec<Row<'_>> = visible_rows
        .iter()
        .enumerate()
        .take(data_limit)
        .map(|(rel_idx, row)| {
            let selected = table.selected_row == Some(skip_rows + rel_idx);
            let cells: Vec<Cell<'_>> = row
                .cells
                .iter()
                .enumerate()
                .map(|(col_idx, cell)| {
                    let max_w = col_widths
                        .get(col_idx)
                        .copied()
                        .unwrap_or_else(|| usize::from(area.width));
                    let total_chars: usize = cell
                        .spans
                        .iter()
                        .map(|sp| strip_terminal_controls(&sp.text).chars().count())
                        .sum();
                    let spans: Vec<Span<'_>> = if total_chars <= max_w {
                        cell.spans
                            .iter()
                            .map(|sp| {
                                let mut st = map_style(sp.style);
                                if selected {
                                    st = st.add_modifier(Modifier::REVERSED);
                                }
                                Span::styled(strip_terminal_controls(&sp.text), st)
                            })
                            .collect()
                    } else {
                        let combined: String =
                            cell.spans.iter().map(|sp| sp.text.as_str()).collect();
                        let mut st = cell
                            .spans
                            .first()
                            .map_or_else(Style::default, |sp| map_style(sp.style));
                        if selected {
                            st = st.add_modifier(Modifier::REVERSED);
                        }
                        vec![Span::styled(clip_text_right(&combined, max_w), st)]
                    };
                    Cell::from(Line::from(spans).alignment(map_align(cell.alignment)))
                })
                .collect();
            let mut r = Row::new(cells);
            if selected {
                r = r.style(map_style(row.style).add_modifier(Modifier::BOLD));
            }
            r
        })
        .collect();
    let widget = Table::new(data_rows, widths).header(header);
    frame.render_widget(widget, table_rect);
    allocated
}

fn render_view_panel(frame: &mut Frame<'_>, area: Rect, panel: &ViewPanel) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let mut inner = if panel.bordered {
        if area.width < 2 || area.height < 2 {
            return;
        }
        let mut block = Block::default().borders(Borders::ALL);
        if let Some(title) = &panel.title {
            block = block.title(Span::styled(
                strip_terminal_controls(&title.text),
                map_style(title.style),
            ));
        }
        let inner_rect = block.inner(area);
        frame.render_widget(block, area);
        inner_rect
    } else {
        area
    };

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    if let Some(tab_bar) = &panel.tabs {
        let used = render_tab_bar(frame, inner, tab_bar);
        inner.y = inner.y.saturating_add(used);
        inner.height = inner.height.saturating_sub(used);
    }
    if inner.height == 0 {
        return;
    }

    let offset = panel.scroll.map_or(0, |s| s.offset);
    let row_offset = if let Some(table) = &panel.table {
        let t_len = table.rows.len();
        if offset < t_len {
            let used = render_view_table(frame, inner, table, offset);
            inner.y = inner.y.saturating_add(used);
            inner.height = inner.height.saturating_sub(used);
            0
        } else {
            offset - t_len
        }
    } else {
        offset
    };
    if inner.height == 0 || panel.rows.is_empty() {
        return;
    }

    let lines: Vec<Line<'_>> = panel
        .rows
        .iter()
        .enumerate()
        .skip(row_offset)
        .take(usize::from(inner.height))
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
                    Span::styled(strip_terminal_controls(&span.text), st)
                })
                .collect();
            Line::from(spans)
        })
        .collect();
    let paragraph = Paragraph::new(Text::from(lines)).alignment(map_align(panel.alignment));
    frame.render_widget(paragraph, inner);
}

fn render_status_line(frame: &mut Frame<'_>, area: Rect, view: &SeatScreenView) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let Some(status) = &view.status_line else {
        return;
    };
    let alert_spans: Option<Vec<Span<'_>>> = status.alert.as_ref().map(|alert| {
        alert
            .spans
            .iter()
            .map(|sp| Span::styled(strip_terminal_controls(&sp.text), map_style(sp.style)))
            .collect()
    });
    let left_spans: Vec<Span<'_>> = status
        .left
        .iter()
        .map(|sp| Span::styled(strip_terminal_controls(&sp.text), map_style(sp.style)))
        .collect();
    let right_spans: Vec<Span<'_>> = status
        .right
        .iter()
        .map(|sp| Span::styled(strip_terminal_controls(&sp.text), map_style(sp.style)))
        .collect();

    let lines = if area.height >= 2 {
        let top = if let Some(alert) = alert_spans {
            Line::from(alert)
        } else {
            Line::from(right_spans)
        };
        vec![top, Line::from(left_spans)]
    } else {
        let mut combined = Vec::new();
        if let Some(alert) = alert_spans {
            combined.extend(alert);
            if !left_spans.is_empty() {
                combined.push(Span::styled(" | ", map_style(status.style)));
                combined.extend(left_spans);
            }
        } else {
            combined.extend(left_spans);
            if !right_spans.is_empty() {
                if !combined.is_empty() {
                    combined.push(Span::styled(" | ", map_style(status.style)));
                }
                combined.extend(right_spans);
            }
        }
        vec![Line::from(combined)]
    };

    frame.render_widget(
        Paragraph::new(Text::from(lines)).style(map_style(status.style)),
        area,
    );
}

/// Draws a plain [`SeatScreenView`] view model emitted by `league-seat-ui` using `ratatui`.
pub(crate) fn draw_screen_view(frame: &mut Frame<'_>, view: &SeatScreenView) {
    let full_area = frame.area();
    frame.render_widget(Clear, full_area);

    if view.layout.mode == ViewLayoutMode::TooSmall || view.layout.bands.is_empty() {
        if let Some(zone) = view.zones.first() {
            render_view_panel(frame, full_area, &zone.panel);
        }
        return;
    }

    let band_constraints: Vec<Constraint> = view
        .layout
        .bands
        .iter()
        .map(|band| map_constraint(band.hint))
        .collect();
    let band_rects = Layout::default()
        .direction(Direction::Vertical)
        .constraints(band_constraints)
        .split(full_area);

    for (band_rect, band) in band_rects.iter().copied().zip(view.layout.bands.iter()) {
        if band.zones.is_empty() || band_rect.width == 0 || band_rect.height == 0 {
            continue;
        }
        let zone_constraints: Vec<Constraint> = band
            .zones
            .iter()
            .map(|spec| map_constraint(spec.hint))
            .collect();
        let zone_rects = Layout::default()
            .direction(map_axis(band.axis))
            .constraints(zone_constraints)
            .split(band_rect);

        for (zone_rect, spec) in zone_rects.iter().copied().zip(band.zones.iter()) {
            if spec.zone == ViewZoneId::StatusLine {
                render_status_line(frame, zone_rect, view);
            } else if let Some(panel) = view.zone(spec.zone) {
                render_view_panel(frame, zone_rect, panel);
            }
        }
    }

    if let Some(modal) = &view.modal {
        let w =
            ((u32::from(full_area.width) * u32::from(modal.width_pct.clamp(10, 100))) / 100) as u16;
        let h = ((u32::from(full_area.height) * u32::from(modal.height_pct.clamp(10, 100))) / 100)
            as u16;
        let w = w.max(20.min(full_area.width)).min(full_area.width);
        let h = h.max(6.min(full_area.height)).min(full_area.height);
        let modal_rect = Rect {
            x: full_area.x + full_area.width.saturating_sub(w) / 2,
            y: full_area.y + full_area.height.saturating_sub(h) / 2,
            width: w,
            height: h,
        };
        frame.render_widget(Clear, modal_rect);
        render_view_panel(frame, modal_rect, &modal.panel);
    }
}

fn host_error(code: &'static str, raw_msg: &str, hint: Option<&'static str>) -> StructuredError {
    StructuredError::new(
        code,
        scrub_secret_text(raw_msg),
        hint.unwrap_or("Submit intents only for the active seat in TalkAndMarket."),
        false,
    )
}

fn scrub_engine_error(err: &EngineError) -> StructuredError {
    let mut s = err.to_structured_error();
    s.message = scrub_secret_text(&s.message);
    s
}

/// Host session bridging a [`ClubHost`] table to seat-isolated `league-seat-ui` views and
/// executing sequential hot-seat passes (`SubPassA -> begin_cancel_lock -> SubPassB -> Sweep`).
#[derive(Debug)]
pub struct TuiHostSession {
    host: ClubHost,
    commodities: Option<CommodityDisplayMap>,
    human_seats: Vec<bool>,
    seat_states: Vec<SeatUiState>,
    pending_bundles: Vec<SealedOrderBundle>,
    sub_pass_b_committed: Vec<bool>,
    seat_notes: Vec<String>,
    seat_pins: Vec<Option<String>>,
    pins_enabled: bool,
    last_errors: Vec<Option<StructuredError>>,
    pass: PassKind,
    pass_offset: usize,
    curtain: Option<CurtainState>,
    quit_requested: bool,
    advance_halted_error: Option<StructuredError>,
    autosave_paths: Option<(PathBuf, PathBuf)>,
    turn_clock_budget_secs: Option<u16>,
    active_pass_started_at: Option<Duration>,
    now: Duration,
    curtain_dead_until: Option<Duration>,
    pending_scrollback_purge: bool,
    pending_input_drain: bool,
    #[cfg(test)]
    injected_raw_what_if: Option<WhatIfPreview>,
    #[cfg(test)]
    injected_bot_action: Option<MarketAction>,
    #[cfg(test)]
    observe_seat_calls: std::cell::Cell<usize>,
}

impl TuiHostSession {
    fn from_host(
        host: ClubHost,
        commodities: Option<CommodityDisplayMap>,
    ) -> Result<Self, StructuredError> {
        Self::from_host_with_extra_bot_action(
            host,
            commodities,
            #[cfg(test)]
            None,
        )
    }

    fn from_host_with_extra_bot_action(
        host: ClubHost,
        commodities: Option<CommodityDisplayMap>,
        #[cfg(test)] extra_bot_action: Option<MarketAction>,
    ) -> Result<Self, StructuredError> {
        #[cfg(not(test))]
        let _ = ();
        let n = usize::from(host.seat_count());
        let spec = host.public_spec();
        let human_seats: Vec<bool> = spec
            .seats
            .iter()
            .map(|cfg| cfg.identity == SeatIdentity::Human)
            .collect();
        let ckpt = ClubSessionCheckpoint::load_from_host(&host);
        let pending_bundles = if ckpt.pending_bundles.len() == n {
            ckpt.pending_bundles
        } else {
            vec![SealedOrderBundle::default(); n]
        };
        let sub_pass_b_committed = if ckpt.sub_pass_b_committed.len() == n {
            ckpt.sub_pass_b_committed
        } else {
            vec![false; n]
        };
        let seat_notes = if ckpt.seat_notes.len() == n {
            ckpt.seat_notes
        } else {
            vec![String::new(); n]
        };
        let seat_pins = if ckpt.seat_pins.len() == n {
            ckpt.seat_pins
        } else {
            vec![None; n]
        };
        let mut session = Self {
            host,
            commodities,
            human_seats,
            seat_states: vec![SeatUiState::new(); n],
            pending_bundles,
            sub_pass_b_committed,
            seat_notes,
            seat_pins,
            pins_enabled: ckpt.pins_enabled,
            last_errors: vec![None; n],
            pass: PassKind::SubPassA,
            pass_offset: 0,
            curtain: None,
            quit_requested: false,
            advance_halted_error: None,
            autosave_paths: None,
            turn_clock_budget_secs: None,
            active_pass_started_at: None,
            now: Duration::ZERO,
            curtain_dead_until: None,
            pending_scrollback_purge: false,
            pending_input_drain: false,
            #[cfg(test)]
            injected_raw_what_if: None,
            #[cfg(test)]
            injected_bot_action: extra_bot_action,
            #[cfg(test)]
            observe_seat_calls: std::cell::Cell::new(0),
        };
        if let Some(cursor) = ckpt.pause_cursor {
            session.pass = if cursor.sub_pass_b {
                PassKind::SubPassB
            } else {
                PassKind::SubPassA
            };
            session.pass_offset = cursor.pass_offset;
            session.curtain = cursor
                .curtain_next_seat
                .map(|s| CurtainState::fresh(Seat(s)));
            session.pending_scrollback_purge = session.curtain.is_some();
            if let Some(active) = session.active_human_seat() {
                session.active_pass_started_at = Some(session.now);
                session.restore_seat_ui_from_host(active);
            }
        } else if session.host.phase() == TurnPhase::TalkAndMarket
            && session.human_seats.iter().any(|&h| h)
        {
            session.advance_bots_without_curtain()?;
            if session.active_human_seat().is_some() {
                session.active_pass_started_at = Some(session.now);
            }
        }
        Ok(session)
    }

    fn observe_seat_checked(&self, seat: Seat) -> Result<SeatObservation, EngineError> {
        #[cfg(test)]
        self.observe_seat_calls
            .set(self.observe_seat_calls.get().saturating_add(1));
        self.host.observe_seat(seat)
    }

    #[cfg(test)]
    pub(crate) fn reset_observe_seat_calls(&self) {
        self.observe_seat_calls.set(0);
    }

    #[cfg(test)]
    pub(crate) fn observe_seat_call_count(&self) -> usize {
        self.observe_seat_calls.get()
    }

    fn pass_budget_secs(&self) -> Option<u16> {
        let budget_secs = self.turn_clock_budget_secs?;
        let human_count =
            u64::try_from(self.human_seats.iter().filter(|&&h| h).count().max(1)).unwrap_or(1);
        let pct = match self.pass {
            PassKind::SubPassA => 75_u64,
            PassKind::SubPassB => 25_u64,
        };
        let raw = (u64::from(budget_secs) * pct) / (100 * human_count);
        Some(u16::try_from(raw.max(1)).unwrap_or(u16::MAX))
    }

    fn refresh_active_clock_display(&mut self) {
        let Some(active) = self.active_human_seat() else {
            return;
        };
        let idx = usize::from(active.0);
        let display = if let Some(budget_secs) = self.pass_budget_secs() {
            let elapsed_secs = self
                .active_pass_started_at
                .map_or(0_u64, |start| self.now.saturating_sub(start).as_secs());
            let rem =
                u16::try_from(u64::from(budget_secs).saturating_sub(elapsed_secs)).unwrap_or(0);
            TurnClockDisplay::timed(rem, budget_secs)
        } else {
            TurnClockDisplay::untimed()
        };
        self.seat_states[idx].set_clock_display(display);
    }

    /// Enables timed per-turn budgeting (`Some(180..=300)` seconds split 75%/25% across human
    /// seats) or disables the countdown clock (`None` for Untimed Hot-Seat).
    pub(crate) fn configure_turn_clock(&mut self, budget_secs: Option<u16>) {
        self.turn_clock_budget_secs = budget_secs;
        if budget_secs.is_some()
            && self.active_human_seat().is_some()
            && self.active_pass_started_at.is_none()
        {
            self.active_pass_started_at = Some(self.now);
        }
        self.refresh_active_clock_display();
    }

    /// Enables optional 4-digit table-local seat PINs at every Curtain; if the table is on its
    /// initial Turn-1 human seat with no Curtain yet, opens the Curtain so that seat sets its PIN
    /// before any seat observation is built.
    pub(crate) fn enable_seat_pins(&mut self) {
        self.pins_enabled = true;
        if self.curtain.is_none()
            && let Some(active) = self.active_human_seat()
            && self.seat_pins[usize::from(active.0)].is_none()
        {
            self.curtain = Some(CurtainState::fresh(active));
            self.active_pass_started_at = None;
            self.curtain_dead_until = None;
            self.pending_scrollback_purge = true;
        }
    }

    fn expire_active_seat_pass(&mut self, seat: Seat) {
        let idx = usize::from(seat.0);
        self.seat_notes[idx] = self.seat_states[idx].notes_draft().to_owned();
        if self.pass == PassKind::SubPassB {
            let staged_non_empty = self.pending_bundles[idx] != SealedOrderBundle::default();
            self.sub_pass_b_committed[idx] = staged_non_empty;
        }
        self.seat_states[idx].clear_for_hand_off();
        self.last_errors[idx] = None;
        self.active_pass_started_at = None;
        self.pending_scrollback_purge = true;
        self.pass_offset += 1;
        if let Err(err) = self.advance_after_human_lock() {
            self.advance_halted_error = Some(err);
        }
    }

    fn curtain_pin_prompt(curtain: &CurtainState) -> CurtainPinPrompt {
        match &curtain.pin_entry {
            Some(entry) => CurtainPinPrompt {
                mode: entry.mode,
                entered_count: u8::try_from(entry.digits.len()).unwrap_or(4),
                failed_attempts: entry.wrong_tries,
                locked_out_reset: curtain.locked_out_reset,
            },
            None => CurtainPinPrompt {
                mode: CurtainPinMode::Inactive,
                entered_count: 0,
                failed_attempts: 0,
                locked_out_reset: curtain.locked_out_reset,
            },
        }
    }

    fn restore_seat_ui_from_host(&mut self, seat: Seat) {
        let idx = usize::from(seat.0);
        self.seat_states[idx].clear_for_hand_off();
        let synced = self.pending_bundles[idx].clone();
        self.seat_states[idx].set_staged_bundle(synced);
        let note = self.seat_notes[idx].clone();
        for ch in note.chars() {
            self.seat_states[idx].apply_note_key(SeatKeyEvent::new(
                SeatKeyCode::Char(ch),
                SeatKeyModifiers::NONE,
            ));
        }
        self.refresh_active_clock_display();
    }

    fn sync_checkpoint_to_host(&mut self) -> Result<(), StructuredError> {
        let pause_cursor =
            (self.host.phase() == TurnPhase::TalkAndMarket).then(|| ClubPauseCursor {
                sub_pass_b: self.pass == PassKind::SubPassB,
                pass_offset: self.pass_offset,
                curtain_next_seat: self.curtain.as_ref().map(|c| c.next_seat.0),
            });
        ClubSessionCheckpoint::store_on_host(
            &mut self.host,
            ClubSessionCheckpoint {
                pause_cursor,
                pending_bundles: self.pending_bundles.clone(),
                sub_pass_b_committed: self.sub_pass_b_committed.clone(),
                seat_notes: self.seat_notes.clone(),
                seat_pins: self.seat_pins.clone(),
                pins_enabled: self.pins_enabled,
            },
        )
        .map_err(|err| scrub_engine_error(&err))
    }

    fn maybe_autosave(&mut self) -> Result<(), StructuredError> {
        if self.is_wedged() {
            return Ok(());
        }
        if let Some((save_path, seal_path)) = self.autosave_paths.clone() {
            self.write_save_and_seal(&save_path, &seal_path)?;
        }
        Ok(())
    }

    /// Configures automatic `.save.json` / `.seal` checkpointing at every Curtain and Sweep.
    pub(crate) fn enable_autosave(&mut self, save_path: PathBuf, seal_path: PathBuf) {
        self.autosave_paths = Some((save_path, seal_path));
    }

    /// Bootstraps a Club Mode session on the canonical `TableSpec::standard_club_chiral_ring_with_setup`.
    #[cfg(test)]
    pub fn from_standard_setup(
        table_id: TableId,
        seat_count: u8,
        layout_seed: u64,
        bot_tie_seed: u64,
    ) -> Result<Self, StructuredError> {
        let host = ClubHost::from_standard_setup(table_id, seat_count, layout_seed, bot_tie_seed)
            .map_err(|err| scrub_engine_error(&err))?;
        Self::from_host(host, None)
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
            .map_err(|err| GenError::InvalidConfig(scrub_secret_text(&err.to_string())))?;
        Self::from_host(host, Some(cmap.to_display_map()))
            .map_err(|err| GenError::InvalidConfig(scrub_secret_text(&err.to_string())))
    }

    /// Creates a Club Mode session from explicit [`TableSpec`], [`SealedMandateDeal`], and
    /// [`SealedEvalSeed`] values for deterministic tests.
    #[cfg(test)]
    pub fn from_explicit_setup(
        spec: TableSpec,
        layout_seed: u64,
        commodities: Option<CommodityDisplayMap>,
        mandate_deal: SealedMandateDeal,
        eval_seed: SealedEvalSeed,
        bot_tie_seed: u64,
    ) -> Result<Self, StructuredError> {
        let host = ClubHost::new(spec, layout_seed, mandate_deal, eval_seed, bot_tie_seed)
            .map_err(|err| scrub_engine_error(&err))?;
        Self::from_host(host, commodities)
    }

    /// Advances the monotonic session clock (`elapsed` since session loop start), retires expired
    /// post-Curtain dead time, updates the active seat's header countdown, and locks timed-out
    /// human passes through the Privacy Curtain.
    pub(crate) fn sync_clock(&mut self, elapsed: Duration) {
        self.now = self.now.max(elapsed);
        if self
            .curtain_dead_until
            .is_some_and(|until| self.now >= until)
        {
            self.curtain_dead_until = None;
        }
        let Some(active) = self.active_human_seat() else {
            return;
        };
        if let (Some(budget_secs), Some(started_at)) =
            (self.pass_budget_secs(), self.active_pass_started_at)
            && self.now.saturating_sub(started_at) >= Duration::from_secs(u64::from(budget_secs))
        {
            self.expire_active_seat_pass(active);
            return;
        }
        self.refresh_active_clock_display();
    }

    /// Advances the session clock by `delta` for deterministic tests.
    #[cfg(test)]
    pub fn advance_clock(&mut self, delta: Duration) {
        self.sync_clock(self.now.saturating_add(delta));
    }

    /// Consumes a pending terminal scrollback purge request emitted on Curtain open or lift.
    pub(crate) fn take_scrollback_purge(&mut self) -> bool {
        std::mem::take(&mut self.pending_scrollback_purge)
    }

    /// Consumes a pending input-queue drain request emitted right as the Curtain lifts.
    pub(crate) fn take_input_drain(&mut self) -> bool {
        std::mem::take(&mut self.pending_input_drain)
    }

    /// Returns whether the session halted on a pass-advance error.
    pub(crate) const fn is_wedged(&self) -> bool {
        self.advance_halted_error.is_some()
    }

    /// Returns the number of seats at the table (`4..=6`).
    #[cfg(test)]
    #[must_use]
    pub const fn seat_count(&self) -> u8 {
        self.host.seat_count()
    }

    /// Returns the active turn number (`1..=32`).
    #[cfg(test)]
    #[must_use]
    pub const fn turn(&self) -> u8 {
        self.host.turn()
    }

    /// Returns the current [`TurnPhase`].
    #[must_use]
    pub const fn phase(&self) -> TurnPhase {
        self.host.phase()
    }

    /// Returns the active sub-pass (`SubPassA` or `SubPassB`).
    #[cfg(test)]
    #[must_use]
    pub const fn pass(&self) -> PassKind {
        self.pass
    }

    /// Returns whether the Privacy Curtain is currently displayed between seat hand-offs.
    #[cfg(test)]
    #[must_use]
    pub const fn is_curtain_open(&self) -> bool {
        self.curtain.is_some()
    }

    /// Returns whether the current session received a quit request.
    #[must_use]
    pub const fn quit_requested(&self) -> bool {
        self.quit_requested
    }

    /// Returns the seat-safe 30-field live [`PublicSweepReceipt`] history.
    #[cfg(test)]
    #[must_use]
    pub fn public_receipts(&self) -> &[PublicSweepReceipt] {
        self.host.public_receipts()
    }

    /// Returns an immutable reference to `seat`'s host-owned [`SeatUiState`].
    #[cfg(test)]
    #[must_use]
    pub fn seat_ui_state(&self, seat: Seat) -> Option<&SeatUiState> {
        self.seat_states.get(usize::from(seat.0))
    }

    /// Returns the pending [`SealedOrderBundle`] staged for `seat`.
    #[cfg(test)]
    #[must_use]
    pub fn pending_bundle(&self, seat: Seat) -> Option<&SealedOrderBundle> {
        self.pending_bundles.get(usize::from(seat.0))
    }

    /// Returns the seat currently controlling the keyboard (`None` when Curtain is up, season is
    /// closed, a pass advance failed, or quit was requested).
    #[must_use]
    pub fn active_human_seat(&self) -> Option<Seat> {
        if self.curtain.is_some()
            || self.quit_requested
            || self.is_wedged()
            || self.host.phase() != TurnPhase::TalkAndMarket
        {
            return None;
        }
        let s = self.seat_at_offset(self.pass_offset)?;
        if self.human_seats.get(usize::from(s.0)).copied() == Some(true) {
            Some(s)
        } else {
            None
        }
    }

    /// Builds a single [`SeatObservation`] **by value** for `seat`, carrying strictly `seat`'s
    /// own dealt `PrivateMandate` and never rival seats' mandates or hidden engine state.
    #[cfg(test)]
    pub fn build_seat_observation(&self, seat: Seat) -> Result<SeatObservation, EngineError> {
        self.host.observe_seat(seat)
    }

    /// Builds one [`SeatObservation`] **by value** per seat `0..seat_count` in ascending order,
    /// each carrying strictly that seat's own `PrivateMandate`.
    #[cfg(test)]
    pub fn build_all_seat_observations(&self) -> Result<Vec<SeatObservation>, EngineError> {
        let count = self.seat_count();
        let mut out = Vec::with_capacity(usize::from(count));
        for s in 0..count {
            out.push(self.build_seat_observation(Seat(s))?);
        }
        Ok(out)
    }

    /// Runs `TalkAndMarket -> Commit -> Reveal -> sweep_turn` atomically on the host.
    #[cfg(test)]
    pub fn commit_reveal_and_sweep(
        &mut self,
        reveals: &BTreeMap<u8, OrderReveal>,
    ) -> Result<PublicSweepReceipt, EngineError> {
        self.host.commit_and_reveal_all(reveals)?;
        self.host.sweep_turn()
    }

    /// Atomically persists `.save.json` and `.seal` sidecar files on the host, refusing wedged
    /// sessions (`SESSION_WEDGED`).
    pub fn write_save_and_seal(
        &mut self,
        save_path: &Path,
        seal_path: &Path,
    ) -> Result<(), StructuredError> {
        if self.is_wedged() {
            return Err(host_error(
                "SESSION_WEDGED",
                "refused saving a wedged Club session after a pass advance error",
                Some("Resume from the last clean Curtain or Sweep autosave."),
            ));
        }
        self.sync_checkpoint_to_host()?;
        self.host
            .write_save_and_seal(save_path, seal_path)
            .map_err(|err| scrub_engine_error(&err))
    }

    /// Reloads a persisted Club Mode host session from `.save.json` and `.seal`, restoring any
    /// paused pass/Curtain cursor, uncommitted bundles, private notes, and PIN state.
    pub fn read_save_and_seal(
        save_path: &Path,
        seal_path: &Path,
        commodities: Option<CommodityDisplayMap>,
    ) -> Result<Self, StructuredError> {
        let host = ClubHost::read_save_and_seal(save_path, seal_path)
            .map_err(|err| scrub_engine_error(&err))?;
        Self::from_host(host, commodities)
    }

    /// Wipes the private `.seal` sidecar cleanly once the season finishes and replay log is
    /// exported, refusing wedged or in-progress sessions.
    #[cfg(test)]
    pub(crate) fn wipe_seal_after_season_export(
        &self,
        seal_path: &Path,
    ) -> Result<(), StructuredError> {
        if self.is_wedged() {
            return Err(host_error(
                "SESSION_WEDGED",
                "refused wiping .seal for a wedged Club session",
                Some("Resolve or resume from a clean autosave before wiping .seal."),
            ));
        }
        if self.host.phase() == TurnPhase::TalkAndMarket {
            return Err(host_error(
                "SEASON_NOT_FINISHED",
                "refused wiping .seal while season is still open",
                None,
            ));
        }
        ClubHost::wipe_seal_file(seal_path).map_err(|err| scrub_engine_error(&err))
    }

    /// Wipes a finished `.seal` sidecar cleanly.
    #[cfg(test)]
    pub fn wipe_seal_file(seal_path: &Path) -> Result<(), EngineError> {
        ClubHost::wipe_seal_file(seal_path)
    }

    /// Returns this table's [`TableId`].
    #[must_use]
    pub const fn table_id(&self) -> TableId {
        self.host.table_id()
    }

    /// Records a scrubbed host-level error banner for the currently active seat.
    #[cfg(test)]
    pub fn record_scrubbed_host_error(&mut self, code: &'static str, raw_msg: &str) {
        if let Some(active) = self.active_human_seat() {
            self.last_errors[usize::from(active.0)] = Some(host_error(code, raw_msg, None));
        }
    }

    /// Builds the plain owned [`SeatScreenView`] for either the Curtain hand-off, the active
    /// seat's [`SeatObservation`], or a strictly public standby view when no human seat is active.
    pub fn build_current_view(
        &self,
        viewport: ViewportSize,
    ) -> Result<SeatScreenView, EngineError> {
        if let Some(curtain) = &self.curtain {
            return Ok(league_seat_ui::curtain_view(
                self.host.table_id(),
                Turn(self.host.turn()),
                self.pass,
                curtain.next_seat,
                viewport,
                curtain.space_armed,
                Self::curtain_pin_prompt(curtain),
            ));
        }
        let Some(target_seat) = self.active_human_seat() else {
            let mut view = league_seat_ui::neutral_session_view(
                self.host.table_id(),
                Turn(self.host.turn()),
                self.host.phase(),
                self.pass,
                self.quit_requested,
                self.is_wedged(),
                viewport,
            );
            if let Some(err) = &self.advance_halted_error
                && let Some(status) = &mut view.status_line
            {
                let banner = scrub_secret_text(&format!("REFUSED [{}]: {}", err.code, err.message));
                status.alert = Some(ViewRow::from_spans(vec![ViewSpan::styled(
                    banner,
                    ViewStyle::plain().with_fg(ViewColor::Red).bold(),
                )]));
            }
            return Ok(view);
        };
        let obs = self.observe_seat_checked(target_seat)?;
        let idx = usize::from(target_seat.0);
        let mut view = league_seat_ui::render_turn_screen(
            &obs,
            &self.seat_states[idx],
            self.pass,
            self.commodities.as_ref(),
            viewport,
        );
        if let Some(err) = &self.last_errors[idx]
            && let Some(status) = &mut view.status_line
        {
            let banner = scrub_secret_text(&format!("REFUSED [{}]: {}", err.code, err.message));
            status.alert = Some(ViewRow::from_spans(vec![ViewSpan::styled(
                banner,
                ViewStyle::plain().with_fg(ViewColor::Red).bold(),
            )]));
        }
        Ok(view)
    }

    /// Renders the active view (Curtain, neutral standby, or active seat's turn screen) onto `frame`.
    pub fn render_current_frame(&self, frame: &mut Frame<'_>) -> Result<(), EngineError> {
        let viewport = ViewportSize::new(frame.area().width, frame.area().height);
        let view = self.build_current_view(viewport)?;
        draw_screen_view(frame, &view);
        Ok(())
    }

    /// Routes one [`SeatKeyEvent`] through either Curtain confirmation or `league-seat-ui` and
    /// applies any emitted [`SeatIntent`] after host validation.
    pub fn handle_key_event(
        &mut self,
        key: SeatKeyEvent,
        viewport: ViewportSize,
    ) -> Result<(), StructuredError> {
        if let Some(curtain) = &mut self.curtain {
            if key.is_repeat {
                return Ok(());
            }
            let next = curtain.next_seat;
            let next_idx = usize::from(next.0);
            if let Some(entry) = &mut curtain.pin_entry {
                match key.code {
                    SeatKeyCode::Char('c') if key.modifiers.ctrl => {
                        self.quit_requested = true;
                    }
                    SeatKeyCode::Esc => {
                        curtain.pin_entry = None;
                        curtain.space_armed = false;
                        curtain.locked_out_reset = false;
                    }
                    SeatKeyCode::Backspace => {
                        entry.digits.pop();
                    }
                    SeatKeyCode::Char(ch)
                        if ch.is_ascii_digit()
                            && !key.modifiers.ctrl
                            && !key.modifiers.alt
                            && entry.digits.len() < 4 =>
                    {
                        entry.digits.push(ch);
                    }
                    SeatKeyCode::Enter if entry.digits.len() == 4 => {
                        let pin_matches_or_set = match entry.mode {
                            CurtainPinMode::SetInitialPin => {
                                self.seat_pins[next_idx] = Some(entry.digits.clone());
                                true
                            }
                            CurtainPinMode::VerifyPin => {
                                self.seat_pins[next_idx].as_deref() == Some(entry.digits.as_str())
                            }
                            CurtainPinMode::Inactive => true,
                        };
                        if pin_matches_or_set {
                            let need_autosave = entry.mode == CurtainPinMode::SetInitialPin;
                            self.curtain = None;
                            self.active_pass_started_at = Some(self.now);
                            self.curtain_dead_until =
                                Some(self.now.saturating_add(CURTAIN_LIFT_DEAD_TIME));
                            self.pending_input_drain = true;
                            self.pending_scrollback_purge = true;
                            self.restore_seat_ui_from_host(next);
                            if need_autosave {
                                self.maybe_autosave()?;
                            }
                        } else {
                            entry.digits.clear();
                            entry.wrong_tries = entry.wrong_tries.saturating_add(1);
                            if entry.wrong_tries >= 3 {
                                curtain.pin_entry = None;
                                curtain.space_armed = false;
                                curtain.locked_out_reset = true;
                            }
                        }
                    }
                    _ => {}
                }
                return Ok(());
            }

            match key.code {
                SeatKeyCode::Char('c') if key.modifiers.ctrl => {
                    self.quit_requested = true;
                }
                SeatKeyCode::Char('q') | SeatKeyCode::Esc => {
                    self.quit_requested = true;
                }
                SeatKeyCode::Char(' ') => {
                    curtain.space_armed = true;
                    curtain.locked_out_reset = false;
                }
                SeatKeyCode::Enter if curtain.space_armed => {
                    if self.pins_enabled {
                        let mode = if self.seat_pins[next_idx].is_some() {
                            CurtainPinMode::VerifyPin
                        } else {
                            CurtainPinMode::SetInitialPin
                        };
                        curtain.locked_out_reset = false;
                        curtain.pin_entry = Some(CurtainPinEntry {
                            mode,
                            digits: String::new(),
                            wrong_tries: 0,
                        });
                    } else {
                        self.curtain = None;
                        self.active_pass_started_at = Some(self.now);
                        self.curtain_dead_until =
                            Some(self.now.saturating_add(CURTAIN_LIFT_DEAD_TIME));
                        self.pending_input_drain = true;
                        self.pending_scrollback_purge = true;
                        self.restore_seat_ui_from_host(next);
                    }
                }
                _ => {
                    curtain.space_armed = false;
                }
            }
            return Ok(());
        }

        if let Some(until) = self.curtain_dead_until {
            if self.now < until {
                return Ok(());
            }
            self.curtain_dead_until = None;
        }

        let Some(active) = self.active_human_seat() else {
            if key.is_repeat {
                return Ok(());
            }
            if matches!(key.code, SeatKeyCode::Char('q') | SeatKeyCode::Esc)
                || (key.code == SeatKeyCode::Char('c') && key.modifiers.ctrl)
            {
                self.quit_requested = true;
            }
            return Ok(());
        };

        let idx = usize::from(active.0);
        self.last_errors[idx] = None;
        let obs = self
            .observe_seat_checked(active)
            .map_err(|err| scrub_engine_error(&err))?;
        let state = self.seat_states[idx].clone();
        let step = league_seat_ui::handle_seat_key(
            &obs,
            state,
            self.pass,
            self.commodities.as_ref(),
            viewport,
            key,
        );
        drop(obs);
        self.seat_states[idx] = step.state;
        self.seat_notes[idx] = self.seat_states[idx].notes_draft().to_owned();
        if let Some(intent) = step.intent
            && let Err(err) = self.apply_seat_intent(active, intent)
        {
            self.last_errors[idx] = Some(err.clone());
            return Err(err);
        }
        Ok(())
    }

    fn compute_raw_what_if(
        &mut self,
        obs: &SeatObservation,
        req: &league_core::HumanWhatIfRequest,
    ) -> Result<WhatIfPreview, StructuredError> {
        #[cfg(test)]
        if let Some(injected) = self.injected_raw_what_if.take() {
            return Ok(injected);
        }
        preview_what_if(&obs.to_public_preview_state(), req).map_err(|err| scrub_engine_error(&err))
    }

    /// Validates and applies a plain owned [`SeatIntent`] strictly if `submitting_seat` is the
    /// active keyboard seat and all payload seat indices and engine checks succeed.
    ///
    /// Any refusal returns a scrubbed [`StructuredError`] with a named error code and **mutates
    /// zero engine or UI state**.
    pub fn apply_seat_intent(
        &mut self,
        submitting_seat: Seat,
        intent: SeatIntent,
    ) -> Result<(), StructuredError> {
        let Some(active) = self.active_human_seat() else {
            return Err(host_error(
                "INACTIVE_SEAT_INTENT",
                &format!(
                    "refused {:?} for seat {}: no active human keyboard turn",
                    intent.kind, submitting_seat.0
                ),
                Some("Wait until Curtain hand-off finishes for that seat."),
            ));
        };
        if active != submitting_seat {
            return Err(host_error(
                "INACTIVE_SEAT_INTENT",
                &format!(
                    "refused {:?} for seat {} while seat {} is active",
                    intent.kind, submitting_seat.0, active.0
                ),
                Some("Submit actions only for the currently active seat."),
            ));
        }

        let unexpected_payload = match intent.kind {
            SeatIntentKind::SubmitMarketAction => {
                intent.bundle_action.is_some() || intent.what_if.is_some()
            }
            SeatIntentKind::UpdateBundle => {
                intent.market_action.is_some() || intent.what_if.is_some()
            }
            SeatIntentKind::RequestWhatIf => {
                intent.market_action.is_some() || intent.bundle_action.is_some()
            }
            SeatIntentKind::LockAndHandOff
            | SeatIntentKind::OpenChronicleModal
            | SeatIntentKind::CloseChronicleModal
            | SeatIntentKind::Quit => {
                intent.market_action.is_some()
                    || intent.bundle_action.is_some()
                    || intent.what_if.is_some()
            }
        };
        if unexpected_payload {
            return Err(host_error(
                "UNEXPECTED_INTENT_PAYLOAD",
                &format!(
                    "refused {:?}: extra payload fields not matching intent kind",
                    intent.kind
                ),
                Some("Populate only the payload field named by SeatIntentKind."),
            ));
        }

        let idx = usize::from(submitting_seat.0);
        match intent.kind {
            SeatIntentKind::SubmitMarketAction => {
                let action = intent.market_action.ok_or_else(|| {
                    host_error(
                        "MISSING_MARKET_ACTION",
                        "missing market_action payload",
                        None,
                    )
                })?;
                if action.actor_seat() != submitting_seat.0 {
                    return Err(host_error(
                        "SEAT_MISMATCH",
                        &format!(
                            "refused MarketAction targeting actor seat {} while seat {} is active",
                            action.actor_seat(),
                            submitting_seat.0
                        ),
                        None,
                    ));
                }
                if matches!(action, MarketAction::CancelOffer { .. }) && self.host.cancel_locked() {
                    return Err(host_error(
                        "CANCEL_LOCKED",
                        "CancelOffer is blocked after Phase-1 cancel lock engaged",
                        None,
                    ));
                }
                self.host
                    .apply_market_action(action)
                    .map_err(|err| scrub_engine_error(&err))?;
                Ok(())
            }
            SeatIntentKind::UpdateBundle => {
                let op = intent.bundle_action.ok_or_else(|| {
                    host_error(
                        "MISSING_BUNDLE_ACTION",
                        "missing bundle_action payload",
                        None,
                    )
                })?;
                let mut candidate = self.pending_bundles[idx].clone();
                op.apply_to(&mut candidate);
                let turn = self.host.turn();
                if candidate.council_vote.is_some() && !turn.is_multiple_of(4) {
                    return Err(host_error(
                        "INVALID_VOTE_TURN",
                        &format!(
                            "council_vote is allowed only on turns divisible by 4 (turn {turn})"
                        ),
                        None,
                    ));
                }
                if !candidate.charter_bids.is_empty() && !turn.is_multiple_of(6) {
                    return Err(host_error(
                        "INVALID_CHARTER_TURN",
                        &format!(
                            "charter_bids are allowed only on turns divisible by 6 (turn {turn})"
                        ),
                        None,
                    ));
                }
                let obs = self
                    .observe_seat_checked(submitting_seat)
                    .map_err(|err| scrub_engine_error(&err))?;
                let _preview = preview_seat_bundle(
                    &obs.to_public_preview_state(),
                    &AiPreviewRequest {
                        seat: submitting_seat.0,
                        market_actions: Vec::new(),
                        bundle: candidate.clone(),
                    },
                )
                .map_err(|err| scrub_engine_error(&err))?;
                self.pending_bundles[idx] = candidate.clone();
                self.seat_states[idx].set_staged_bundle(candidate);
                Ok(())
            }
            SeatIntentKind::RequestWhatIf => {
                let req = intent.what_if.ok_or_else(|| {
                    host_error(
                        "MISSING_WHAT_IF_REQUEST",
                        "missing what_if request payload",
                        None,
                    )
                })?;
                if req.seat != submitting_seat.0 {
                    return Err(host_error(
                        "SEAT_MISMATCH",
                        &format!(
                            "refused What-If request for seat {} while seat {} is active",
                            req.seat, submitting_seat.0
                        ),
                        None,
                    ));
                }
                let obs = self
                    .observe_seat_checked(submitting_seat)
                    .map_err(|err| scrub_engine_error(&err))?;
                let mut preview = self.compute_raw_what_if(&obs, &req)?;
                for refused in &mut preview.refused_market_actions {
                    refused.message = scrub_secret_text(&refused.message);
                }
                self.seat_states[idx].set_what_if_preview(Some(preview));
                Ok(())
            }
            SeatIntentKind::OpenChronicleModal => {
                self.seat_states[idx].set_chronicle_open(true);
                Ok(())
            }
            SeatIntentKind::CloseChronicleModal => {
                self.seat_states[idx].set_chronicle_open(false);
                Ok(())
            }
            SeatIntentKind::Quit => {
                self.quit_requested = true;
                Ok(())
            }
            SeatIntentKind::LockAndHandOff => {
                self.seat_notes[idx] = self.seat_states[idx].notes_draft().to_owned();
                if self.pass == PassKind::SubPassB {
                    self.sub_pass_b_committed[idx] = true;
                }
                self.seat_states[idx].clear_for_hand_off();
                self.last_errors[idx] = None;
                self.active_pass_started_at = None;
                self.pending_scrollback_purge = true;
                self.pass_offset += 1;
                if let Err(err) = self.advance_after_human_lock() {
                    self.advance_halted_error = Some(err.clone());
                    return Err(err);
                }
                Ok(())
            }
        }
    }

    /// Builds one [`SeatObservation`] **by value** for `seat`, requests the Part A view model, and
    /// draws it onto `frame` via `ratatui`.
    #[cfg(test)]
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

    fn seat_at_offset(&self, offset: usize) -> Option<Seat> {
        let n = usize::from(self.host.seat_count());
        if n == 0 || offset >= n {
            return None;
        }
        let start = usize::from(self.host.turn().saturating_sub(1)) % n;
        let idx = (start + offset) % n;
        u8::try_from(idx).ok().map(Seat)
    }

    fn run_bot_for_current_pass(&mut self, seat: Seat) -> Result<(), StructuredError> {
        #[cfg(test)]
        if let Some(action) = self.injected_bot_action.take() {
            self.host
                .apply_market_action(action)
                .map_err(|err| scrub_engine_error(&err))?;
        }
        let obs = self
            .host
            .observe_seat(seat)
            .map_err(|err| scrub_engine_error(&err))?;
        let mandate = *obs
            .own_mandate()
            .ok_or_else(|| host_error("MISSING_MANDATE", "missing bot mandate", None))?;
        let bot = ClubBot::new(self.host.bot_tie_seed());
        match self.pass {
            PassKind::SubPassA => {
                let actions = bot.plan_pass_a(&obs, &mandate);
                drop(obs);
                for action in actions {
                    self.host
                        .apply_market_action(action)
                        .map_err(|err| scrub_engine_error(&err))?;
                }
            }
            PassKind::SubPassB => {
                let (actions, bundle) = bot.plan_pass_b_and_bundle(&obs, &mandate);
                drop(obs);
                for action in actions {
                    self.host
                        .apply_market_action(action)
                        .map_err(|err| scrub_engine_error(&err))?;
                }
                let idx = usize::from(seat.0);
                self.pending_bundles[idx] = bundle;
                self.sub_pass_b_committed[idx] = true;
            }
        }
        Ok(())
    }

    fn advance_bots_without_curtain(&mut self) -> Result<(), StructuredError> {
        let n = usize::from(self.host.seat_count());
        while self.pass_offset < n {
            let Some(seat) = self.seat_at_offset(self.pass_offset) else {
                break;
            };
            if self.human_seats.get(usize::from(seat.0)).copied() == Some(true) {
                return Ok(());
            }
            self.run_bot_for_current_pass(seat)?;
            self.pass_offset += 1;
        }
        Ok(())
    }

    fn advance_after_human_lock(&mut self) -> Result<(), StructuredError> {
        let n = usize::from(self.host.seat_count());
        let has_any_human = self.human_seats.iter().any(|&h| h);
        if !has_any_human {
            return Ok(());
        }

        loop {
            while self.pass_offset < n {
                let seat = self.seat_at_offset(self.pass_offset).ok_or_else(|| {
                    host_error("INVALID_OFFSET", "seat offset out of range", None)
                })?;
                if self.human_seats[usize::from(seat.0)] {
                    self.curtain = Some(CurtainState::fresh(seat));
                    self.active_pass_started_at = None;
                    self.curtain_dead_until = None;
                    self.pending_scrollback_purge = true;
                    self.maybe_autosave()?;
                    return Ok(());
                }
                self.run_bot_for_current_pass(seat)?;
                self.pass_offset += 1;
            }

            match self.pass {
                PassKind::SubPassA => {
                    self.host
                        .begin_cancel_lock()
                        .map_err(|err| scrub_engine_error(&err))?;
                    self.pass = PassKind::SubPassB;
                    self.pass_offset = 0;
                }
                PassKind::SubPassB => {
                    let mut reveals = BTreeMap::new();
                    for s in 0..self.host.seat_count() {
                        let idx = usize::from(s);
                        let committed = std::mem::take(&mut self.sub_pass_b_committed[idx]);
                        let bundle = std::mem::take(&mut self.pending_bundles[idx]);
                        if committed {
                            let salt = ClubSessionCheckpoint::turn_reveal_salt(&self.host, Seat(s))
                                .map_err(|err| scrub_engine_error(&err))?;
                            reveals.insert(s, OrderReveal { bundle, salt });
                        }
                    }
                    self.host
                        .commit_and_reveal_all(&reveals)
                        .map_err(|err| scrub_engine_error(&err))?;
                    self.host
                        .sweep_turn()
                        .map_err(|err| scrub_engine_error(&err))?;
                    for state in &mut self.seat_states {
                        state.set_staged_bundle(SealedOrderBundle::default());
                    }
                    self.pass = PassKind::SubPassA;
                    self.pass_offset = 0;
                    if self.host.phase() != TurnPhase::TalkAndMarket {
                        self.curtain = None;
                        self.maybe_autosave()?;
                        return Ok(());
                    }
                    self.maybe_autosave()?;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use league_core::{
        CharterBidOrder, CharterRole, ConverterRunOrder, CouncilVoteBallot, HumanWhatIfRequest,
        MandateKind, OfferProposal, OfferSide, PreviewTieMode, TableMode,
        deal_graded_mandates_with_entropy, replay_season_with_state_predicates,
    };
    use league_gen::{
        GeneratorConfig, SeatCount, SeedPool, TablePurpose, TableSetupOptions,
        generate_ring_table_for_setup, is_reserved_test_seed,
        ring_table_to_table_spec_with_explicit_entropy, seed_pool,
    };
    use league_seat_types::{
        AssetId, BundleDraftAction, COMMODITY_DISPLAY_CATALOG, HYPOTHETICAL_CLOSE_MARK_CAVEAT,
        RefusedActionPreview, SeatConfig, SeatKeyCode, SeatKeyEvent, SeatKeyModifiers,
        SeatSpoilagePreview, SeatUpkeepPreview, Step0To7SweepSummary, ViewAxis, ViewLabel,
        ViewLayoutMode, ViewModal, ViewRow, ViewScroll, ViewSizeHint, ViewSpan, ViewStatusLine,
        ViewTabBar, ViewTabItem, ViewZoneGroup, ViewZoneId, ViewZonePanel, ViewZoneSpec,
        WhatIfPreview, format_numeraire_display,
    };
    use league_seat_ui::CenterTab;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::collections::BTreeSet;

    fn dev_ring_table_for_seats(start_seed: u64, seat_count: SeatCount) -> RingTable {
        let cfg = GeneratorConfig {
            seat_count: Some(seat_count),
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

    fn dev_ring_table(start_seed: u64) -> RingTable {
        dev_ring_table_for_seats(start_seed, SeatCount::Four)
    }

    fn distinct_mandate_deal_seed(ring: &RingTable, human_seats: &[u8]) -> [u8; 32] {
        let salts: Vec<RevealSalt> = (0..ring.seat_count.as_u8())
            .map(|i| RevealSalt([0x41_u8.wrapping_add(i); 32]))
            .collect();
        for seed_byte in 1_u8..=255 {
            let eval_seed = must_ok(SealedEvalSeed::try_from_bytes([0x77; 32]), "eval seed");
            let deal_seed = [seed_byte; 32];
            let (spec, _cmap, deal, _eval) = must_ok(
                ring_table_to_table_spec_with_explicit_entropy(
                    ring,
                    TableId(91_000),
                    human_seats,
                    deal_seed,
                    &salts,
                    eval_seed,
                ),
                "bridge",
            );
            let mut fulls = BTreeSet::new();
            let mut clips = BTreeSet::new();
            for rev in &deal.seat_reveals {
                let s = mandate_short_marker(rev.mandate.kind);
                clips.insert(s.chars().take(22).collect::<String>());
                fulls.insert(s);
            }
            let n = usize::from(spec.seat_count);
            if fulls.len() == n && clips.len() == n {
                return deal_seed;
            }
        }
        panic!("could not find pairwise-distinct mandate deal within 255 seeds");
    }

    fn build_explicit_sessions(
        ring: &RingTable,
        table_id: TableId,
        human_seats: &[u8],
        deal_seed: [u8; 32],
        eval_seed_bytes: [u8; 32],
        bot_tie_seed: u64,
    ) -> (TuiHostSession, ClubHost, CommodityDisplayMap) {
        let salts: Vec<RevealSalt> = (0..ring.seat_count.as_u8())
            .map(|i| RevealSalt([0x41_u8.wrapping_add(i); 32]))
            .collect();
        let eval_1 = must_ok(SealedEvalSeed::try_from_bytes(eval_seed_bytes), "eval_1");
        let eval_2 = must_ok(SealedEvalSeed::try_from_bytes(eval_seed_bytes), "eval_2");
        let (spec, cmap, deal, eval_1) = must_ok(
            ring_table_to_table_spec_with_explicit_entropy(
                ring,
                table_id,
                human_seats,
                deal_seed,
                &salts,
                eval_1,
            ),
            "bridge 1",
        );
        let dmap = cmap.to_display_map();
        let tui = must_ok(
            TuiHostSession::from_explicit_setup(
                spec.clone(),
                ring.seed,
                Some(dmap.clone()),
                deal.clone(),
                eval_1,
                bot_tie_seed,
            ),
            "tui setup",
        );
        let headless = must_ok(
            ClubHost::new(spec, ring.seed, deal, eval_2, bot_tie_seed),
            "headless setup",
        );
        (tui, headless, dmap)
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

    fn find_text_coords(terminal: &Terminal<TestBackend>, needle: &str) -> Option<(u16, u16)> {
        let txt = screen_text(terminal);
        for (y, line) in txt.lines().enumerate() {
            if let Some(byte_idx) = line.find(needle) {
                let x = line[..byte_idx].chars().count();
                return Some((u16::try_from(x).unwrap_or(0), u16::try_from(y).unwrap_or(0)));
            }
        }
        None
    }

    fn key(ch: char) -> SeatKeyEvent {
        SeatKeyEvent::new(SeatKeyCode::Char(ch), SeatKeyModifiers::NONE)
    }

    fn key_code(code: SeatKeyCode) -> SeatKeyEvent {
        SeatKeyEvent::new(code, SeatKeyModifiers::NONE)
    }

    fn ctrl_key(ch: char) -> SeatKeyEvent {
        SeatKeyEvent::new(SeatKeyCode::Char(ch), SeatKeyModifiers::CTRL)
    }

    fn type_text_in_mode(
        session: &mut TuiHostSession,
        mode_key: char,
        text: &str,
        vp: ViewportSize,
    ) {
        must_ok(
            session.handle_key_event(key(mode_key), vp),
            "enter edit mode",
        );
        for ch in text.chars() {
            must_ok(session.handle_key_event(key(ch), vp), "type char");
        }
        must_ok(
            session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
            "commit edit text",
        );
    }

    #[derive(Debug, Clone)]
    struct CapturedSeasonDraw {
        turn: u8,
        active_seat: Option<u8>,
        view_text: String,
        buffer_text: String,
    }

    #[derive(Debug, Clone)]
    struct MandateLeakSpec {
        short_full: String,
        clipped_drawer: String,
        debug_kind: String,
        bonus_line: String,
        target_tokens: Vec<String>,
    }

    fn build_mandate_leak_spec(mandate: &league_core::PrivateMandate) -> MandateLeakSpec {
        let short_full = mandate_short_marker(mandate.kind);
        let clipped_drawer: String = short_full.chars().take(22).collect();
        let debug_kind = format!("{:?}", mandate.kind);
        let bonus_line = format!("Bonus: +{} eq", mandate.bonus_equity);
        let target_tokens = match mandate.kind {
            MandateKind::MinProjectEquity { target } => {
                let full_t = format!(">={target}");
                let clipped_t: String = full_t.chars().take(5).collect();
                vec![full_t, clipped_t]
            }
            MandateKind::MinRefinedVariety {
                distinct_goods,
                min_each,
            } => vec![format!("{distinct_goods}x{min_each}")],
            MandateKind::MaintainCleanCredit { min_tier } => {
                vec![format!("CleanCredit >= {min_tier:?}")]
            }
            MandateKind::MinActivePartners { min_partners } => {
                vec![format!("ActivePartners >= {min_partners}")]
            }
        };
        MandateLeakSpec {
            short_full,
            clipped_drawer,
            debug_kind,
            bonus_line,
            target_tokens,
        }
    }

    fn draw_and_capture(
        session: &TuiHostSession,
        terminal: &mut Terminal<TestBackend>,
        vp: ViewportSize,
        turn: u8,
        active_seat: Option<u8>,
        frames: &mut Vec<CapturedSeasonDraw>,
    ) {
        let view = must_ok(session.build_current_view(vp), "build current view");
        let view_text = view.plain_text();
        must_ok(
            terminal.draw(|f| draw_screen_view(f, &view)),
            "draw captured view",
        );
        frames.push(CapturedSeasonDraw {
            turn,
            active_seat,
            view_text,
            buffer_text: screen_text(terminal),
        });
    }

    fn unlock_curtain_and_capture(
        session: &mut TuiHostSession,
        terminal: &mut Terminal<TestBackend>,
        vp: ViewportSize,
        frames: &mut Vec<CapturedSeasonDraw>,
    ) {
        if !session.is_curtain_open() {
            return;
        }
        draw_and_capture(session, terminal, vp, session.turn(), None, frames);
        must_ok(session.handle_key_event(key(' '), vp), "curtain space");
        draw_and_capture(session, terminal, vp, session.turn(), None, frames);
        must_ok(
            session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
            "curtain enter",
        );
        session.advance_clock(CURTAIN_LIFT_DEAD_TIME);
    }

    fn mandate_short_marker(kind: MandateKind) -> String {
        match kind {
            MandateKind::MinProjectEquity { target } => format!("MinProjectEquity >={target}"),
            MandateKind::MinRefinedVariety {
                distinct_goods,
                min_each,
            } => format!("MinRefinedVar {distinct_goods}x{min_each}"),
            MandateKind::MaintainCleanCredit { min_tier } => {
                format!("CleanCredit >= {min_tier:?}")
            }
            MandateKind::MinActivePartners { min_partners } => {
                format!("ActivePartners >= {min_partners}")
            }
        }
    }

    fn check_season_leaks(
        frames: &[CapturedSeasonDraw],
        mandates: &[MandateLeakSpec],
        salt_hexes: &[String],
        digests: &[[u8; 32]],
        drawn_shock_bps_by_turn: &[(u8, u8, league_core::Bps)],
    ) -> Result<(), String> {
        if frames.is_empty() {
            return Err("expected at least one captured frame".to_owned());
        }
        let digest_hexes: Vec<(String, String)> = digests
            .iter()
            .map(|d| {
                let full: String = d.iter().map(|b| format!("{b:02x}")).collect();
                let short = full.chars().take(8).collect();
                (full, short)
            })
            .collect();
        let salt_prefixes: Vec<(String, String)> = salt_hexes
            .iter()
            .map(|s| {
                let low = s.to_ascii_lowercase();
                let short = low.chars().take(16).collect();
                (low, short)
            })
            .collect();

        for frame in frames {
            let v_low = frame.view_text.to_ascii_lowercase();
            let b_low = frame.buffer_text.to_ascii_lowercase();
            if frame.active_seat.is_none()
                && (frame.view_text.contains("Private Vault")
                    || frame.view_text.contains("Private Mandate")
                    || frame.buffer_text.contains("Private Vault")
                    || frame.buffer_text.contains("Private Mandate"))
            {
                return Err(format!(
                    "Turn {} non-active frame exposed Private Vault/Mandate",
                    frame.turn
                ));
            }
            let own = frame.active_seat.and_then(|s| mandates.get(usize::from(s)));
            for (seat_idx, spec) in mandates.iter().enumerate() {
                if frame
                    .active_seat
                    .is_some_and(|s| usize::from(s) == seat_idx)
                {
                    continue;
                }
                if own.is_none_or(|o| o.short_full != spec.short_full) {
                    if frame.view_text.contains(&spec.short_full) {
                        return Err(format!(
                            "Turn {} seat {:?} pre-draw view leaked seat {seat_idx}'s mandate `{}`",
                            frame.turn, frame.active_seat, spec.short_full
                        ));
                    }
                    if frame.buffer_text.contains(&spec.short_full) {
                        return Err(format!(
                            "Turn {} seat {:?} buffer leaked seat {seat_idx}'s mandate `{}`",
                            frame.turn, frame.active_seat, spec.short_full
                        ));
                    }
                }
                if own.is_none_or(|o| o.clipped_drawer != spec.clipped_drawer)
                    && frame.buffer_text.contains(&spec.clipped_drawer)
                {
                    return Err(format!(
                        "Turn {} seat {:?} buffer leaked seat {seat_idx}'s clipped mandate `{}`",
                        frame.turn, frame.active_seat, spec.clipped_drawer
                    ));
                }
                if own.is_none_or(|o| o.debug_kind != spec.debug_kind)
                    && (frame.view_text.contains(&spec.debug_kind)
                        || frame.buffer_text.contains(&spec.debug_kind))
                {
                    return Err(format!(
                        "Turn {} seat {:?} leaked seat {seat_idx}'s Debug mandate `{}`",
                        frame.turn, frame.active_seat, spec.debug_kind
                    ));
                }
                if own.is_none_or(|o| o.bonus_line != spec.bonus_line)
                    && (frame.view_text.contains(&spec.bonus_line)
                        || frame.buffer_text.contains(&spec.bonus_line))
                {
                    return Err(format!(
                        "Turn {} seat {:?} leaked seat {seat_idx}'s mandate bonus `{}`",
                        frame.turn, frame.active_seat, spec.bonus_line
                    ));
                }
                for tok in &spec.target_tokens {
                    if own.is_none_or(|o| !o.target_tokens.contains(tok))
                        && (frame.view_text.contains(tok) || frame.buffer_text.contains(tok))
                    {
                        return Err(format!(
                            "Turn {} seat {:?} leaked seat {seat_idx}'s mandate target `{tok}`",
                            frame.turn, frame.active_seat,
                        ));
                    }
                }
            }
            for (salt_full, salt_short) in &salt_prefixes {
                if v_low.contains(salt_full)
                    || b_low.contains(salt_full)
                    || (!salt_short.is_empty()
                        && (v_low.contains(salt_short) || b_low.contains(salt_short)))
                {
                    return Err(format!(
                        "Turn {} seat {:?} leaked mandate salt hex",
                        frame.turn, frame.active_seat
                    ));
                }
            }
            for (d_full, d_short) in &digest_hexes {
                if v_low.contains(d_full)
                    || b_low.contains(d_full)
                    || (!d_short.is_empty() && (v_low.contains(d_short) || b_low.contains(d_short)))
                {
                    return Err(format!(
                        "Turn {} seat {:?} leaked post_state_digest `{d_full}`",
                        frame.turn, frame.active_seat
                    ));
                }
            }
            for &(realize_turn, _forecast_turn, mag_bps) in drawn_shock_bps_by_turn {
                if frame.turn < realize_turn {
                    let tight = format!("{mag_bps}bps");
                    let spaced = format!("{mag_bps} bps");
                    let pct = format!("{:.2}%", f64::from(mag_bps) / 100.0);
                    let view_no_closing = frame
                        .view_text
                        .replace("35.00% roll each Sweep", "<closing_roll>");
                    let buf_no_closing = frame
                        .buffer_text
                        .replace("35.00% roll each Sweep", "<closing_roll>");
                    if frame.view_text.contains(&tight)
                        || frame.buffer_text.contains(&tight)
                        || frame.view_text.contains(&spaced)
                        || frame.buffer_text.contains(&spaced)
                    {
                        return Err(format!(
                            "Turn {} seat {:?} leaked unrealized shock magnitude `{tight}` (realizes T{realize_turn})",
                            frame.turn, frame.active_seat
                        ));
                    }
                    if view_no_closing.contains(&pct) || buf_no_closing.contains(&pct) {
                        return Err(format!(
                            "Turn {} seat {:?} leaked unrealized shock percentage `{pct}` (realizes T{realize_turn})",
                            frame.turn, frame.active_seat
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    fn assert_no_season_leaks(
        frames: &[CapturedSeasonDraw],
        mandates: &[MandateLeakSpec],
        salt_hexes: &[String],
        digests: &[[u8; 32]],
        drawn_shock_bps_by_turn: &[(u8, u8, league_core::Bps)],
    ) {
        if let Err(err) = check_season_leaks(
            frames,
            mandates,
            salt_hexes,
            digests,
            drawn_shock_bps_by_turn,
        ) {
            panic!("{err}");
        }
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

    fn must_err<T: std::fmt::Debug, E>(res: Result<T, E>, ctx: &str) -> E {
        match res {
            Ok(v) => panic!("expected Err for {ctx}, got Ok({v:?})"),
            Err(err) => err,
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
            let ev = must_some(
                map_crossterm_key(KeyEvent::new(
                    raw_code,
                    KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT,
                )),
                "KeyEventKind::Press must map",
            );
            assert_eq!(ev.code, expected);
            assert!(ev.modifiers.ctrl && ev.modifiers.alt && ev.modifiers.shift);
            assert!(!ev.is_repeat);
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
        let layout = league_seat_types::ScreenLayout {
            viewport: ViewportSize::new(80, 24),
            mode: ViewLayoutMode::StackedFallback,
            bands: vec![
                ViewZoneGroup {
                    axis: ViewAxis::Vertical,
                    hint: ViewSizeHint::Fixed(3),
                    zones: vec![ViewZoneSpec {
                        zone: ViewZoneId::HeaderBar,
                        hint: ViewSizeHint::Fixed(3),
                    }],
                },
                ViewZoneGroup {
                    axis: ViewAxis::Horizontal,
                    hint: ViewSizeHint::Min(8),
                    zones: vec![
                        ViewZoneSpec {
                            zone: ViewZoneId::PublicLedger,
                            hint: ViewSizeHint::Ratio { num: 1, den: 2 },
                        },
                        ViewZoneSpec {
                            zone: ViewZoneId::PrivateDrawer,
                            hint: ViewSizeHint::Min(10),
                        },
                    ],
                },
                ViewZoneGroup {
                    axis: ViewAxis::Vertical,
                    hint: ViewSizeHint::Fixed(1),
                    zones: vec![ViewZoneSpec {
                        zone: ViewZoneId::StatusLine,
                        hint: ViewSizeHint::Fixed(1),
                    }],
                },
            ],
        };
        let rich = SeatScreenView {
            layout,
            zones: vec![
                ViewZonePanel {
                    zone: ViewZoneId::HeaderBar,
                    panel: ViewPanel {
                        title: None,
                        alignment: ViewAlign::Right,
                        bordered: false,
                        tabs: None,
                        table: None,
                        rows: vec![ViewRow::from_plain("TOP HEADER".to_owned())],
                        selected_row: None,
                        scroll: None,
                    },
                },
                ViewZonePanel {
                    zone: ViewZoneId::PublicLedger,
                    panel: ViewPanel {
                        title: Some(ViewLabel::styled("Main Panel".to_owned(), styled)),
                        alignment: ViewAlign::Left,
                        bordered: true,
                        tabs: Some(ViewTabBar {
                            tabs: vec![ViewTabItem {
                                label: ViewLabel::plain("TabA".to_owned()),
                                enabled: true,
                            }],
                            active_index: 0,
                        }),
                        table: None,
                        rows: vec![
                            ViewRow::from_spans(vec![ViewSpan::styled(
                                "SELECTED".to_owned(),
                                styled,
                            )]),
                            ViewRow::from_plain("SECOND".to_owned()),
                        ],
                        selected_row: Some(0),
                        scroll: Some(ViewScroll {
                            offset: 0,
                            total: 2,
                        }),
                    },
                },
                ViewZonePanel {
                    zone: ViewZoneId::PrivateDrawer,
                    panel: ViewPanel {
                        title: Some(ViewLabel::plain("Side".to_owned())),
                        alignment: ViewAlign::Center,
                        bordered: true,
                        tabs: None,
                        table: None,
                        rows: vec![ViewRow::from_plain("SIDE TEXT".to_owned())],
                        selected_row: None,
                        scroll: None,
                    },
                },
            ],
            status_line: Some(ViewStatusLine {
                alert: None,
                left: vec![ViewSpan::plain("STATUS-L".to_owned())],
                right: vec![ViewSpan::plain("STATUS-R".to_owned())],
                style: ViewStyle::plain(),
            }),
            modal: Some(ViewModal {
                width_pct: 75,
                height_pct: 60,
                panel: ViewPanel {
                    title: Some(ViewLabel::plain("Modal".to_owned())),
                    alignment: ViewAlign::Center,
                    bordered: true,
                    tabs: None,
                    table: None,
                    rows: vec![ViewRow::from_plain("MODAL BODY".to_owned())],
                    selected_row: None,
                    scroll: None,
                },
            }),
        };

        let mut terminal = must_ok(Terminal::new(TestBackend::new(80, 24)), "TestBackend");
        must_ok(
            terminal.draw(|frame| draw_screen_view(frame, &rich)),
            "draw rich view model",
        );
        let txt = screen_text(&terminal);
        assert!(txt.contains("TOP HEADER"));
        assert!(txt.contains("MODAL BODY"));
        assert!(txt.contains("STATUS-L | STATUS-R"));
    }

    #[test]
    fn intent_for_other_seat_is_refused_with_named_code_and_zero_state_change() {
        let ring = dev_ring_table(1);
        let mut session = must_ok(
            TuiHostSession::from_ring_table(&ring, TableId(81_003), &[0, 1], 19),
            "2-human session",
        );
        assert_eq!(session.active_human_seat(), Some(Seat(0)));

        let obs_before = must_ok(session.build_all_seat_observations(), "obs before");
        let ui0_before = must_some(session.seat_ui_state(Seat(0)), "ui0 before").clone();
        let ui1_before = must_some(session.seat_ui_state(Seat(1)), "ui1 before").clone();
        let b0_before = must_some(session.pending_bundle(Seat(0)), "b0 before").clone();
        let b1_before = must_some(session.pending_bundle(Seat(1)), "b1 before").clone();

        let err_inactive = must_err(
            session.apply_seat_intent(
                Seat(1),
                SeatIntent::submit_market_action(MarketAction::PostOffer {
                    seat: 1,
                    order: OfferProposal {
                        side: OfferSide::Buy,
                        good: AssetId::Raw(0),
                        amount: 500,
                        price_per_unit_cred: 1_000,
                        domain_id: None,
                        is_hybrid: false,
                    },
                }),
            ),
            "seat 1 intent while seat 0 active must fail",
        );
        assert_eq!(err_inactive.code, "INACTIVE_SEAT_INTENT");

        let err_mismatch = must_err(
            session.apply_seat_intent(
                Seat(0),
                SeatIntent::submit_market_action(MarketAction::PostOffer {
                    seat: 1,
                    order: OfferProposal {
                        side: OfferSide::Buy,
                        good: AssetId::Raw(0),
                        amount: 500,
                        price_per_unit_cred: 1_000,
                        domain_id: None,
                        is_hybrid: false,
                    },
                }),
            ),
            "actor seat 1 while seat 0 active must fail",
        );
        assert_eq!(err_mismatch.code, "SEAT_MISMATCH");

        let err_what_if = must_err(
            session.apply_seat_intent(
                Seat(0),
                SeatIntent::request_what_if(HumanWhatIfRequest {
                    seat: 1,
                    market_actions: Vec::new(),
                    cancel_lock_at: None,
                    bundle: SealedOrderBundle::default(),
                    tie_mode: PreviewTieMode::NeutralSeed,
                    assumptions: league_core::CounterpartyAssumptions::default(),
                }),
            ),
            "what-if for seat 1 while seat 0 active must fail",
        );
        assert_eq!(err_what_if.code, "SEAT_MISMATCH");

        let obs_after = must_ok(session.build_all_seat_observations(), "obs after");
        assert_eq!(obs_before, obs_after);
        assert_eq!(
            must_some(session.seat_ui_state(Seat(0)), "ui0 after"),
            &ui0_before
        );
        assert_eq!(
            must_some(session.seat_ui_state(Seat(1)), "ui1 after"),
            &ui1_before
        );
        assert_eq!(
            must_some(session.pending_bundle(Seat(0)), "b0 after"),
            &b0_before
        );
        assert_eq!(
            must_some(session.pending_bundle(Seat(1)), "b1 after"),
            &b1_before
        );
    }

    #[test]
    fn per_seat_state_never_crosses_seats_and_handoff_clears_all_free_text() {
        let ring = dev_ring_table(1);
        let mut session = must_ok(
            TuiHostSession::from_ring_table(&ring, TableId(81_004), &[0, 1], 23),
            "2-human session",
        );
        let vp = ViewportSize::new(100, 30);
        let mut terminal = must_ok(Terminal::new(TestBackend::new(100, 30)), "TestBackend");

        must_ok(session.handle_key_event(key('2'), vp), "tab 2");
        must_ok(session.handle_key_event(key(']'), vp), "move section");
        type_text_in_mode(&mut session, 'e', "S0_DRAFT_ONLY_991", vp);
        type_text_in_mode(&mut session, 'n', "S0_NOTE_ONLY_992", vp);

        must_ok(
            terminal.draw(|f| must_ok(session.render_current_frame(f), "draw seat 0")),
            "draw seat 0",
        );
        let s0_txt = screen_text(&terminal);
        assert!(s0_txt.contains("S0_DRAFT_ONLY_991"));
        assert!(s0_txt.contains("S0_NOTE_ONLY_992"));

        must_ok(
            session.handle_key_event(ctrl_key('l'), vp),
            "lock and handoff",
        );
        assert!(session.is_curtain_open());
        let s0_after = must_some(session.seat_ui_state(Seat(0)), "seat 0 state");
        assert_eq!(s0_after.active_tab(), CenterTab::WorkshopDealsPolitics);
        assert_eq!(s0_after.selection().bundle_section_idx, 1);
        assert_eq!(s0_after.composer_draft(), "");
        assert_eq!(s0_after.whisper_draft(), "");
        assert_eq!(s0_after.notes_draft(), "");

        must_ok(
            terminal.draw(|f| must_ok(session.render_current_frame(f), "draw curtain")),
            "draw curtain",
        );
        let curtain_txt = screen_text(&terminal);
        assert!(!curtain_txt.contains("S0_DRAFT_ONLY_991"));
        assert!(!curtain_txt.contains("S0_NOTE_ONLY_992"));

        must_ok(session.handle_key_event(key(' '), vp), "curtain space");
        must_ok(
            session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
            "curtain enter",
        );
        session.advance_clock(CURTAIN_LIFT_DEAD_TIME);
        assert_eq!(session.active_human_seat(), Some(Seat(1)));

        for tab_key in ['1', '2', '3'] {
            must_ok(session.handle_key_event(key(tab_key), vp), "switch tab");
            must_ok(
                terminal.draw(|f| must_ok(session.render_current_frame(f), "draw seat 1")),
                "draw seat 1",
            );
            let s1_txt = screen_text(&terminal);
            assert!(!s1_txt.contains("S0_DRAFT_ONLY_991"));
            assert!(!s1_txt.contains("S0_NOTE_ONLY_992"));
        }
    }

    #[test]
    fn layout_renders_four_zones_at_100x30_stacked_at_80x24_and_too_small_below_80x24() {
        let ring = dev_ring_table(1);
        let session = must_ok(
            TuiHostSession::from_ring_table(&ring, TableId(81_005), &[0], 29),
            "session",
        );

        let mut term_wide = must_ok(Terminal::new(TestBackend::new(100, 30)), "100x30");
        must_ok(
            term_wide.draw(|f| must_ok(session.render_current_frame(f), "draw 100x30")),
            "draw 100x30",
        );
        let ledger_xy = must_some(
            find_text_coords(&term_wide, "Public Table & Ledger Mirror"),
            "ledger title",
        );
        let center_xy = must_some(
            find_text_coords(&term_wide, "Action & Order Composer"),
            "center title",
        );
        let drawer_xy = must_some(
            find_text_coords(&term_wide, "Private Vault"),
            "drawer title",
        );
        let header_xy = must_some(
            find_text_coords(&term_wide, "Merchant League — Table"),
            "header title",
        );
        assert!(header_xy.1 < ledger_xy.1);
        assert!(ledger_xy.0 < center_xy.0 && center_xy.0 < drawer_xy.0);
        let wide_txt = screen_text(&term_wide);
        assert!(wide_txt.contains("1-3:Tabs"));

        let mut term_stacked = must_ok(Terminal::new(TestBackend::new(80, 24)), "80x24");
        must_ok(
            term_stacked.draw(|f| must_ok(session.render_current_frame(f), "draw 80x24")),
            "draw 80x24",
        );
        let h80 = must_some(
            find_text_coords(&term_stacked, "Merchant League"),
            "80 header",
        );
        let l80 = must_some(find_text_coords(&term_stacked, "Public Table"), "80 ledger");
        let c80 = must_some(
            find_text_coords(&term_stacked, "Action & Order"),
            "80 center",
        );
        let p80 = must_some(
            find_text_coords(&term_stacked, "Private Vault"),
            "80 drawer",
        );
        assert!(h80.1 < l80.1 && l80.1 < c80.1 && c80.1 < p80.1);

        for (w, h) in [(79_u16, 23_u16), (55, 14)] {
            let mut term_small = must_ok(Terminal::new(TestBackend::new(w, h)), "small");
            must_ok(
                term_small.draw(|f| must_ok(session.render_current_frame(f), "draw small")),
                "draw small",
            );
            let small_txt = screen_text(&term_small);
            assert!(small_txt.contains("Terminal too small"));
            assert!(!small_txt.contains("Public Table & Ledger Mirror"));
            let lines: Vec<&str> = small_txt.lines().collect();
            let top_border = lines[0].trim();
            let bot_border = lines[lines.len() - 1].trim();
            assert!(top_border.starts_with('┌') && top_border.ends_with('┐'));
            assert!(bot_border.starts_with('└') && bot_border.ends_with('┘'));
        }
    }

    #[test]
    fn turn_zones_tabs_cancel_lock_and_30_field_chronicle_modal_work_end_to_end() {
        let ring = dev_ring_table(1);
        let mut session = must_ok(
            TuiHostSession::from_ring_table(&ring, TableId(81_006), &[0], 31),
            "session",
        );
        let vp = ViewportSize::new(120, 36);
        let mut terminal = must_ok(Terminal::new(TestBackend::new(120, 36)), "TestBackend");

        must_ok(
            terminal.draw(|f| must_ok(session.render_current_frame(f), "draw t1 a")),
            "draw t1 a",
        );
        let pass_a_txt = screen_text(&terminal);
        assert!(pass_a_txt.contains("Diplomatic Desk (Disabled)"));
        assert!(pass_a_txt.contains("Clearinghouse Marks [CRED]"));
        assert!(pass_a_txt.contains("[OPEN BOOK]"));

        // Submit PostOffer in Pass A, then What-If Sandbox preview.
        must_ok(
            session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
            "submit PostOffer",
        );
        must_ok(session.handle_key_event(key('p'), vp), "run what-if");
        must_ok(
            terminal.draw(|f| must_ok(session.render_current_frame(f), "draw what-if")),
            "draw what-if",
        );
        assert!(screen_text(&terminal).contains("Preview T1 S0:"));

        // Lock Pass A -> Curtain -> Unlock into Sub-Pass B (`cancel_locked == true`).
        must_ok(session.handle_key_event(ctrl_key('l'), vp), "lock pass A");
        assert!(session.is_curtain_open());
        must_ok(session.handle_key_event(key(' '), vp), "curtain space");
        must_ok(
            session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
            "curtain enter",
        );
        session.advance_clock(CURTAIN_LIFT_DEAD_TIME);
        assert_eq!(session.pass(), PassKind::SubPassB);
        must_ok(session.handle_key_event(key('1'), vp), "tab 1");
        must_ok(session.handle_key_event(key(']'), vp), "select CancelOffer");
        must_ok(
            terminal.draw(|f| must_ok(session.render_current_frame(f), "draw t1 b")),
            "draw t1 b",
        );
        assert!(screen_text(&terminal).contains("[CANCEL LOCKED]"));
        assert!(screen_text(&terminal).contains("CancelOffer is BLOCKED"));

        let direct_cancel = must_err(
            session.apply_seat_intent(
                Seat(0),
                SeatIntent::submit_market_action(MarketAction::CancelOffer {
                    seat: 0,
                    offer_id: 1,
                }),
            ),
            "direct CancelOffer while cancel_locked must be refused by host",
        );
        assert_eq!(direct_cancel.code, "CANCEL_LOCKED");

        // Complete Sub-Pass B -> Sweep -> Turn 2 Curtain -> Open Sweep Chronicle modal.
        must_ok(session.handle_key_event(ctrl_key('l'), vp), "lock pass B");
        must_ok(session.handle_key_event(key(' '), vp), "curtain space t2");
        must_ok(
            session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
            "curtain enter t2",
        );
        session.advance_clock(CURTAIN_LIFT_DEAD_TIME);
        assert_eq!(session.turn(), 2);
        assert_eq!(session.public_receipts().len(), 1);

        must_ok(
            session.handle_key_event(key('r'), vp),
            "open chronicle modal",
        );
        let mut combined_modal = String::new();
        for _ in 0..4 {
            must_ok(
                terminal.draw(|f| must_ok(session.render_current_frame(f), "draw modal")),
                "draw modal",
            );
            combined_modal.push_str(&screen_text(&terminal));
            combined_modal.push('\n');
            must_ok(
                session.handle_key_event(key_code(SeatKeyCode::PageDown), vp),
                "scroll modal",
            );
        }
        for key_name in [
            "turn:",
            "non_reveal_seats:",
            "regent_acted_seats:",
            "converted_to_ai:",
            "escrows_finished:",
            "escrows_cancelled_or_forfeited:",
            "atomic_swaps_settled:",
            "converter_batches_run:",
            "converter_orders_clamped:",
            "checks_cashed:",
            "checks_bounced:",
            "cartel_evictions:",
            "projects_completed:",
            "project_orders_clamped:",
            "total_upkeep_shortfall_debt:",
            "total_spoilage_clawed_back:",
            "broken_promises:",
            "elected_officers:",
            "charters_awarded:",
            "public_forecasts:",
            "newly_activated_shocks:",
            "cred_debt_repaid:",
            "charter_cred_burned:",
            "town_import_cred_spent:",
            "works_stipend_cred_minted:",
            "batch_trades_cleared:",
            "fills:",
            "wash_freeze_settlements:",
            "season_closed:",
            "pbrs_deltas:",
        ] {
            assert!(
                combined_modal.contains(key_name),
                "Chronicle modal on TestBackend missed `{key_name}`"
            );
        }
        assert!(!combined_modal.contains("newly_drawn_shocks"));
        assert!(!combined_modal.contains("post_state_digest"));
    }

    #[test]
    fn curtain_and_next_seat_first_buffer_exclude_all_previous_seat_private_values() {
        let ring = dev_ring_table(1);
        let deal_seed = distinct_mandate_deal_seed(&ring, &[0, 1]);
        let (mut session, _headless, _dmap) =
            build_explicit_sessions(&ring, TableId(81_007), &[0, 1], deal_seed, [0x77; 32], 37);
        let vp = ViewportSize::new(100, 30);
        let mut terminal = must_ok(Terminal::new(TestBackend::new(100, 30)), "100x30");

        let obs0_t1 = must_ok(session.build_seat_observation(Seat(0)), "obs0 t1");
        let obs1_t1 = must_ok(session.build_seat_observation(Seat(1)), "obs1 t1");
        let s0_mandate = mandate_short_marker(must_some(obs0_t1.own_mandate(), "m0").kind);
        let s1_mandate = mandate_short_marker(must_some(obs1_t1.own_mandate(), "m1").kind);
        assert_ne!(s0_mandate, s1_mandate);

        // Advance cleanly until Turn 8 while winning OracleAuditor for Seat 0 on Turn 6 (`bid = 2500`).
        while session.turn() < 8 {
            unlock_curtain_and_capture(&mut session, &mut terminal, vp, &mut Vec::new());
            if let Some(active) = session.active_human_seat() {
                if active == Seat(0) && session.turn() == 6 && session.pass() == PassKind::SubPassB
                {
                    must_ok(
                        session.apply_seat_intent(
                            Seat(0),
                            SeatIntent::update_bundle(BundleDraftAction::StageCharterBid(
                                CharterBidOrder {
                                    role: CharterRole::OracleAuditor,
                                    bid_cred: 2_500,
                                },
                            )),
                        ),
                        "bid OracleAuditor turn 6",
                    );
                }
                must_ok(
                    session.handle_key_event(ctrl_key('l'), vp),
                    "advance handoff",
                );
            }
        }
        // On Turn 8, `(8 - 1 + 0) % 4 == 3` (Bot), `offset=1` -> Seat(0) in SubPassA.
        unlock_curtain_and_capture(&mut session, &mut terminal, vp, &mut Vec::new());
        assert_eq!(session.turn(), 8);
        assert_eq!(session.active_human_seat(), Some(Seat(0)));

        let obs0_t8 = must_ok(session.build_seat_observation(Seat(0)), "obs0 t8");
        assert!(
            !obs0_t8.auditor_hints.is_empty(),
            "Seat 0 must hold an active Turn-8 OracleAuditor hint from Turn-7 draw"
        );
        let hint = obs0_t8.auditor_hints[0];
        let hint_marker = format!(
            "T{}->T{}->T{} S{}",
            hint.hint_turn, hint.forecast_turn, hint.realize_turn, hint.target_seat
        );

        // Stage a distinctive bundle entry (`debt_repayment_cred: 4321`) + What-If preview on Seat 0.
        must_ok(
            session.apply_seat_intent(
                Seat(0),
                SeatIntent::update_bundle(BundleDraftAction::SetDebtRepaymentCred(4_321)),
            ),
            "stage distinctive debt repayment",
        );
        must_ok(session.handle_key_event(key('p'), vp), "run What-If T8");
        must_ok(session.handle_key_event(key('2'), vp), "tab 2 check");
        must_ok(
            terminal.draw(|f| must_ok(session.render_current_frame(f), "draw s0 t8")),
            "draw s0 t8",
        );
        let s0_buf = screen_text(&terminal);
        assert!(s0_buf.contains(&s0_mandate));
        assert!(s0_buf.contains(&hint_marker));
        assert!(s0_buf.contains("debt_repay=4321"));

        must_ok(session.handle_key_event(ctrl_key('l'), vp), "lock seat 0");
        assert!(session.is_curtain_open());
        // Pressing another key or Enter without Space must NOT unlock Curtain.
        must_ok(
            session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
            "enter without space stays locked",
        );
        assert!(session.is_curtain_open());
        must_ok(
            terminal.draw(|f| must_ok(session.render_current_frame(f), "draw curtain")),
            "draw curtain",
        );
        let curtain_buf = screen_text(&terminal);
        assert!(curtain_buf.contains("Table #81007 | Turn 8"));
        assert!(curtain_buf.contains("Hand keyboard to Seat 1 (Boreal Guild)"));
        assert!(!curtain_buf.contains(&s0_mandate));
        assert!(!curtain_buf.contains(&s1_mandate));
        assert!(!curtain_buf.contains(&hint_marker));
        assert!(!curtain_buf.contains("4321"));
        assert!(!curtain_buf.contains("Preview T8 S0:"));
        assert!(!curtain_buf.contains("locked at"));

        // Confirm Space then Enter and verify Seat 1's first buffer leaks zero Seat 0 private items.
        must_ok(session.handle_key_event(key(' '), vp), "curtain space");
        must_ok(
            session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
            "curtain enter",
        );
        session.advance_clock(CURTAIN_LIFT_DEAD_TIME);
        assert_eq!(session.active_human_seat(), Some(Seat(1)));
        for tab in ['1', '2', '3'] {
            must_ok(session.handle_key_event(key(tab), vp), "seat 1 tab");
            must_ok(
                terminal.draw(|f| must_ok(session.render_current_frame(f), "draw seat 1")),
                "draw seat 1",
            );
            let s1_buf = screen_text(&terminal);
            assert!(!s1_buf.contains(&s0_mandate));
            assert!(!s1_buf.contains(&hint_marker));
            assert!(!s1_buf.contains("4321"));
            assert!(!s1_buf.contains("Preview T8 S0:"));
        }
    }

    #[test]
    fn one_human_plus_club_bots_plays_dev_seed_turn1_to_awaiting_mandate_reveal_leak_free() {
        let ring = dev_ring_table(1);
        let deal_seed = distinct_mandate_deal_seed(&ring, &[0]);
        let (mut session, _headless, _dmap) =
            build_explicit_sessions(&ring, TableId(81_008), &[0], deal_seed, [0x77; 32], 41);
        let vp = ViewportSize::new(100, 30);
        let mut terminal = must_ok(Terminal::new(TestBackend::new(100, 30)), "100x30");
        let mut frames: Vec<CapturedSeasonDraw> = Vec::new();
        let mut opened_chronicle = false;

        while session.phase() == TurnPhase::TalkAndMarket {
            unlock_curtain_and_capture(&mut session, &mut terminal, vp, &mut frames);
            let seat = must_some(session.active_human_seat(), "active seat 0");
            assert_eq!(seat, Seat(0));

            let t = session.turn();
            if session.pass() == PassKind::SubPassA {
                if t == 1 {
                    must_ok(
                        session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
                        "submit PostOffer T1",
                    );
                }
                must_ok(session.handle_key_event(key('p'), vp), "what-if preview");
                if !opened_chronicle && !session.public_receipts().is_empty() {
                    opened_chronicle = true;
                    must_ok(
                        session.handle_key_event(key('r'), vp),
                        "open chronicle modal",
                    );
                    draw_and_capture(&session, &mut terminal, vp, t, Some(0), &mut frames);
                    must_ok(
                        session.handle_key_event(key_code(SeatKeyCode::PageDown), vp),
                        "scroll chronicle modal",
                    );
                    draw_and_capture(&session, &mut terminal, vp, t, Some(0), &mut frames);
                    must_ok(
                        session.handle_key_event(key('r'), vp),
                        "close chronicle modal",
                    );
                }
            } else {
                if t <= 20 {
                    must_ok(session.handle_key_event(key('2'), vp), "tab 2");
                    must_ok(
                        session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
                        "stage converter",
                    );
                }
                if t.is_multiple_of(4) {
                    must_ok(
                        session.apply_seat_intent(
                            Seat(0),
                            SeatIntent::update_bundle(BundleDraftAction::SetCouncilVote(Some(
                                CouncilVoteBallot {
                                    trade_minister: 0,
                                    cartel_chair: 1,
                                },
                            ))),
                        ),
                        "stage turn 4n vote",
                    );
                }
                if t == 6 {
                    must_ok(
                        session.apply_seat_intent(
                            Seat(0),
                            SeatIntent::update_bundle(BundleDraftAction::StageCharterBid(
                                CharterBidOrder {
                                    role: CharterRole::OracleAuditor,
                                    bid_cred: 2_500,
                                },
                            )),
                        ),
                        "stage turn 6 charter bid",
                    );
                }
            }
            draw_and_capture(&session, &mut terminal, vp, t, Some(0), &mut frames);
            must_ok(
                session.handle_key_event(ctrl_key('l'), vp),
                "lock and handoff",
            );
        }

        assert!(
            opened_chronicle,
            "Chronicle modal must be opened during season"
        );
        assert_eq!(session.phase(), TurnPhase::AwaitingMandateReveal);
        assert!(session.public_receipts().len() >= 24);
        draw_and_capture(
            &session,
            &mut terminal,
            vp,
            session.turn(),
            None,
            &mut frames,
        );
        let post_txt = screen_text(&terminal);
        assert!(post_txt.contains("Session Standby"));
        assert!(!post_txt.contains("Private Vault"));

        let (_sb, log, _sec, _met) =
            must_ok(session.host.finalize_club_season(), "finalize club season");
        let (replay_out, _) = must_ok(replay_season_with_state_predicates(&log), "replay log");
        let mandates: Vec<MandateLeakSpec> = log
            .mandate_reveals
            .iter()
            .map(|r| build_mandate_leak_spec(&must_some(r.as_ref(), "revealed mandate").mandate))
            .collect();
        let salt_hexes: Vec<String> = log
            .mandate_reveals
            .iter()
            .map(|r| {
                must_some(r.as_ref(), "rev")
                    .salt
                    .0
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect()
            })
            .collect();
        let mut drawn_shocks = Vec::new();
        for lt in &log.turns {
            for shock in &lt.drawn_shocks {
                drawn_shocks.push((shock.realize_turn, shock.forecast_turn, shock.magnitude_bps));
            }
        }
        assert_no_season_leaks(
            &frames,
            &mandates,
            &salt_hexes,
            &replay_out.state_digests,
            &drawn_shocks,
        );
    }

    #[test]
    fn two_humans_plus_bots_season_matches_headless_public_receipts_byte_for_byte() {
        let ring = dev_ring_table(1);
        let deal_seed = distinct_mandate_deal_seed(&ring, &[0, 1]);
        let (mut session, mut headless, _dmap) =
            build_explicit_sessions(&ring, TableId(81_009), &[0, 1], deal_seed, [0x77; 32], 43);
        let vp = ViewportSize::new(100, 30);
        let mut terminal = must_ok(Terminal::new(TestBackend::new(100, 30)), "100x30");
        let mut frames: Vec<CapturedSeasonDraw> = Vec::new();
        let n = usize::from(headless.seat_count());
        let mut opened_chronicle = false;

        while session.phase() == TurnPhase::TalkAndMarket {
            let turn = headless.turn();
            let mut headless_bundles = vec![SealedOrderBundle::default(); n];

            // Sub-Pass A across rotating seat order.
            for offset in 0..n {
                let s = u8::try_from((usize::from(turn - 1) + offset) % n).unwrap_or(0);
                if s < 2 {
                    unlock_curtain_and_capture(&mut session, &mut terminal, vp, &mut frames);
                    assert_eq!(session.active_human_seat(), Some(Seat(s)));
                    if s == 0 {
                        // Drive Seat 0 via keyboard events on Tab 1 and Sweep Chronicle modal.
                        if turn == 1 {
                            must_ok(
                                session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
                                "tui key submit PostOffer",
                            );
                            let default_action = MarketAction::PostOffer {
                                seat: 0,
                                order: OfferProposal {
                                    side: OfferSide::Buy,
                                    good: AssetId::Raw(1),
                                    amount: 1_000,
                                    price_per_unit_cred: 1_200,
                                    domain_id: None,
                                    is_hybrid: false,
                                },
                            };
                            must_ok(
                                headless.apply_market_action(default_action),
                                "headless PostOffer matching Seat 0 key submit",
                            );
                        }
                        if !opened_chronicle && !session.public_receipts().is_empty() {
                            opened_chronicle = true;
                            must_ok(
                                session.handle_key_event(key('r'), vp),
                                "open chronicle modal",
                            );
                            draw_and_capture(
                                &session,
                                &mut terminal,
                                vp,
                                turn,
                                Some(s),
                                &mut frames,
                            );
                            must_ok(
                                session.handle_key_event(key_code(SeatKeyCode::PageDown), vp),
                                "scroll chronicle modal",
                            );
                            draw_and_capture(
                                &session,
                                &mut terminal,
                                vp,
                                turn,
                                Some(s),
                                &mut frames,
                            );
                            must_ok(
                                session.handle_key_event(key_code(SeatKeyCode::Esc), vp),
                                "close chronicle modal",
                            );
                        }
                    } else if turn == 1 {
                        let action = MarketAction::PostOffer {
                            seat: s,
                            order: OfferProposal {
                                side: OfferSide::Buy,
                                good: AssetId::Raw((s + 1) % 4),
                                amount: 500,
                                price_per_unit_cred: 1_000,
                                domain_id: None,
                                is_hybrid: false,
                            },
                        };
                        must_ok(
                            session.apply_seat_intent(
                                Seat(s),
                                SeatIntent::submit_market_action(action.clone()),
                            ),
                            "tui pass a action",
                        );
                        must_ok(
                            headless.apply_market_action(action),
                            "headless pass a action",
                        );
                    }
                    draw_and_capture(&session, &mut terminal, vp, turn, Some(s), &mut frames);
                    must_ok(session.handle_key_event(ctrl_key('l'), vp), "lock human A");
                } else {
                    let obs = must_ok(headless.observe_seat(Seat(s)), "bot obs A");
                    let m = *must_some(obs.own_mandate(), "bot mandate A");
                    for act in ClubBot::new(headless.bot_tie_seed()).plan_pass_a(&obs, &m) {
                        must_ok(headless.apply_market_action(act), "headless bot A");
                    }
                }
            }

            must_ok(headless.begin_cancel_lock(), "headless cancel lock");

            // Sub-Pass B across rotating seat order.
            for offset in 0..n {
                let s = u8::try_from((usize::from(turn - 1) + offset) % n).unwrap_or(0);
                if s < 2 {
                    unlock_curtain_and_capture(&mut session, &mut terminal, vp, &mut frames);
                    assert_eq!(session.active_human_seat(), Some(Seat(s)));
                    if turn <= 12 {
                        let op = BundleDraftAction::SetConverterRun(ConverterRunOrder {
                            recipe_idx: 0,
                            runs: 1,
                        });
                        op.apply_to(&mut headless_bundles[usize::from(s)]);
                        if s == 0 {
                            must_ok(session.handle_key_event(key('2'), vp), "seat 0 tab 2 key");
                            must_ok(
                                session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
                                "seat 0 key stage converter",
                            );
                        } else {
                            must_ok(
                                session.apply_seat_intent(Seat(s), SeatIntent::update_bundle(op)),
                                "tui stage converter",
                            );
                        }
                    }
                    if turn.is_multiple_of(4) {
                        let op = BundleDraftAction::SetCouncilVote(Some(CouncilVoteBallot {
                            trade_minister: 0,
                            cartel_chair: 1,
                        }));
                        op.apply_to(&mut headless_bundles[usize::from(s)]);
                        must_ok(
                            session.apply_seat_intent(Seat(s), SeatIntent::update_bundle(op)),
                            "tui stage vote",
                        );
                    }
                    if turn.is_multiple_of(6) {
                        let op = BundleDraftAction::StageCharterBid(CharterBidOrder {
                            role: CharterRole::OracleAuditor,
                            bid_cred: if s == 0 { 1_500 } else { 200 },
                        });
                        op.apply_to(&mut headless_bundles[usize::from(s)]);
                        must_ok(
                            session.apply_seat_intent(Seat(s), SeatIntent::update_bundle(op)),
                            "tui stage charter bid",
                        );
                    }
                    draw_and_capture(&session, &mut terminal, vp, turn, Some(s), &mut frames);
                    must_ok(session.handle_key_event(ctrl_key('l'), vp), "lock human B");
                } else {
                    let obs = must_ok(headless.observe_seat(Seat(s)), "bot obs B");
                    let m = *must_some(obs.own_mandate(), "bot mandate B");
                    let (acts, bundle) =
                        ClubBot::new(headless.bot_tie_seed()).plan_pass_b_and_bundle(&obs, &m);
                    for act in acts {
                        must_ok(headless.apply_market_action(act), "headless bot B");
                    }
                    headless_bundles[usize::from(s)] = bundle;
                }
            }

            let mut reveals = BTreeMap::new();
            for s in 0..headless.seat_count() {
                reveals.insert(
                    s,
                    OrderReveal {
                        bundle: std::mem::take(&mut headless_bundles[usize::from(s)]),
                        salt: must_ok(
                            ClubSessionCheckpoint::turn_reveal_salt(&headless, Seat(s)),
                            "headless salt",
                        ),
                    },
                );
            }
            must_ok(headless.commit_and_reveal_all(&reveals), "headless commit");
            must_ok(headless.sweep_turn(), "headless sweep");
        }

        assert!(opened_chronicle, "2-human season must open Chronicle modal");
        assert_eq!(session.phase(), TurnPhase::AwaitingMandateReveal);
        assert_eq!(headless.phase(), TurnPhase::AwaitingMandateReveal);
        draw_and_capture(
            &session,
            &mut terminal,
            vp,
            headless.turn(),
            None,
            &mut frames,
        );
        let post_txt = screen_text(&terminal);
        assert!(post_txt.contains("Session Standby"));
        assert!(!post_txt.contains("Private Vault"));

        let tui_receipts = session.public_receipts();
        let headless_receipts = headless.public_receipts();
        assert_eq!(tui_receipts.len(), headless_receipts.len());
        assert!(tui_receipts.len() >= 24);
        for (idx, (tr, hr)) in tui_receipts
            .iter()
            .zip(headless_receipts.iter())
            .enumerate()
        {
            let tj = must_ok(tr.to_json_string(), "serialize tui receipt");
            let hj = must_ok(hr.to_json_string(), "serialize headless receipt");
            assert_eq!(
                tj.as_bytes(),
                hj.as_bytes(),
                "receipt {} diverged between TUI and headless",
                idx + 1
            );
        }

        let (_sb, log, _sec, _met) = must_ok(
            session.host.finalize_club_season(),
            "finalize 2-human season",
        );
        let (replay_out, _) = must_ok(replay_season_with_state_predicates(&log), "replay 2-human");
        let mandates: Vec<MandateLeakSpec> = log
            .mandate_reveals
            .iter()
            .map(|r| build_mandate_leak_spec(&must_some(r.as_ref(), "rev").mandate))
            .collect();
        let salt_hexes: Vec<String> = log
            .mandate_reveals
            .iter()
            .map(|r| {
                must_some(r.as_ref(), "rev")
                    .salt
                    .0
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect()
            })
            .collect();
        let mut drawn_shocks = Vec::new();
        for lt in &log.turns {
            for shock in &lt.drawn_shocks {
                drawn_shocks.push((shock.realize_turn, shock.forecast_turn, shock.magnitude_bps));
            }
        }
        assert_no_season_leaks(
            &frames,
            &mandates,
            &salt_hexes,
            &replay_out.state_digests,
            &drawn_shocks,
        );
    }

    #[test]
    fn tui_buffers_scrub_error_and_refusal_secrets_while_preserving_display_phrases() {
        let ring = dev_ring_table(1);
        let mut session = must_ok(
            TuiHostSession::from_ring_table(&ring, TableId(81_010), &[0], 47),
            "session",
        );
        let vp_small = ViewportSize::new(80, 24);
        let vp_wide = ViewportSize::new(100, 30);
        let mut term_80 = must_ok(Terminal::new(TestBackend::new(80, 24)), "80x24");
        let mut term_100 = must_ok(Terminal::new(TestBackend::new(100, 30)), "100x30");

        // 1. Secret-bearing host error banner reaches buffer strictly scrubbed at 80x24 and 100x30.
        session.record_scrubbed_host_error(
            "HOST_ERR",
            "leaked sEd7N5k8mQp2rT9vW3xY6zA4bC1d OPENROUTER_API_KEY=sk-secret-value Bearer tok_live_999 sk-proj-secret123",
        );
        for (term, label) in [(&mut term_80, "80x24"), (&mut term_100, "100x30")] {
            must_ok(
                term.draw(|f| must_ok(session.render_current_frame(f), label)),
                label,
            );
            let err_buf = screen_text(term);
            assert!(
                err_buf.contains("REFUSED [HOST_ERR]:"),
                "missing banner at {label}"
            );
            assert!(
                err_buf.contains("[REDACTED]"),
                "missing [REDACTED] at {label}"
            );
            for secret in [
                "sEd7N5k8mQp2rT9vW3xY6zA4bC1d",
                "sk-secret-value",
                "tok_live_999",
                "sk-proj-secret123",
            ] {
                assert!(
                    !err_buf.contains(secret),
                    "TUI leaked `{secret}` at {label}"
                );
            }
        }

        // 2. Display-text paths + Part A over-redaction phrases render verbatim on TestBackend
        // at 80x24 (full 78-col stacked drawer) and 100x30 without triggering false-positive `[REDACTED]`.
        let obs = must_ok(session.build_seat_observation(Seat(0)), "obs");
        let over_redaction_phrases = [
            "risk-or-reward",
            "ask-or-tell",
            "desk-proj-alpha",
            "SKU SK-PROJ-1234",
            "class=\"btn-bearer active\"",
            "the standard bearer of",
        ];
        for phrase in over_redaction_phrases {
            let mut s = SeatUiState::new();
            s = league_seat_ui::handle_seat_key(
                &obs,
                s,
                PassKind::SubPassA,
                None,
                vp_small,
                key('n'),
            )
            .state;
            for ch in phrase.chars() {
                s = league_seat_ui::handle_seat_key(
                    &obs,
                    s,
                    PassKind::SubPassA,
                    None,
                    vp_small,
                    key(ch),
                )
                .state;
            }
            s = league_seat_ui::handle_seat_key(
                &obs,
                s,
                PassKind::SubPassA,
                None,
                vp_small,
                key_code(SeatKeyCode::Enter),
            )
            .state;
            for _ in 0..20 {
                s = league_seat_ui::handle_seat_key(
                    &obs,
                    s,
                    PassKind::SubPassA,
                    None,
                    vp_small,
                    key_code(SeatKeyCode::Down),
                )
                .state;
            }
            let view_80 =
                league_seat_ui::render_turn_screen(&obs, &s, PassKind::SubPassA, None, vp_small);
            must_ok(
                term_80.draw(|f| draw_screen_view(f, &view_80)),
                "draw over-redaction phrase 80x24",
            );
            let txt_80 = screen_text(&term_80);
            assert!(
                txt_80.contains(phrase),
                "80x24 TUI buffer altered over-redaction phrase `{phrase}`:\n{txt_80}"
            );
            assert!(!txt_80.contains("[REDACTED]"));

            let view_100 =
                league_seat_ui::render_turn_screen(&obs, &s, PassKind::SubPassA, None, vp_wide);
            must_ok(
                term_100.draw(|f| draw_screen_view(f, &view_100)),
                "draw over-redaction phrase 100x30",
            );
            let txt_100 = screen_text(&term_100);
            let clip_24: String = phrase.chars().take(24).collect();
            assert!(
                txt_100.contains(&clip_24),
                "100x30 TUI buffer altered clipped over-redaction phrase `{clip_24}`:\n{txt_100}"
            );
            assert!(!txt_100.contains("[REDACTED]"));
        }

        // 3. Raw secret injected into What-If result is scrubbed inside `apply_seat_intent(RequestWhatIf)`
        // and reaches TestBackend at supported sizes (80x24 and 100x30) strictly scrubbed.
        let dummy_state = obs.seats[0].clone();
        session.last_errors[0] = None;
        session.injected_raw_what_if = Some(WhatIfPreview {
            seat: 0,
            turn: 1,
            baseline_predicates: obs.baseline_predicates.clone(),
            pre_seat_state: dummy_state.clone(),
            post_market_seat_state: dummy_state.clone(),
            post_sweep_seat_state: dummy_state,
            post_sweep_predicates: obs.baseline_predicates.clone(),
            close_mark_caveat: HYPOTHETICAL_CLOSE_MARK_CAVEAT.to_owned(),
            refused_market_actions: vec![RefusedActionPreview {
                action_index: 0,
                error_code: "CANCEL_LOCKED".to_owned(),
                message: "bad key sk-ant-secret_999 leaked sEd7N5k8mQp2rT9vW3xY6zA4bC1d".to_owned(),
            }],
            converter_orders: Vec::new(),
            project_orders: Vec::new(),
            swap_orders: Vec::new(),
            check_orders: Vec::new(),
            escrow_orders: Vec::new(),
            upkeep: SeatUpkeepPreview {
                upkeep_good: 3,
                upkeep_required: 1_800,
                paid_with_refined: 1_800,
                debt_interest_accrued: 0,
                new_shortfall_debt: 0,
                debt_repaid_cred: 0,
            },
            spoilage: SeatSpoilagePreview {
                perishable_spoiled_units: 0,
            },
            public_fills: Vec::new(),
            step0_to_7_receipt: Step0To7SweepSummary {
                turn: 1,
                non_reveal_seats: Vec::new(),
                regent_acted_seats: Vec::new(),
                converted_to_ai: Vec::new(),
                escrows_finished: Vec::new(),
                escrows_cancelled_or_forfeited: Vec::new(),
                atomic_swaps_settled: 0,
                converter_batches_run: 0,
                converter_orders_clamped: 0,
                checks_cashed: Vec::new(),
                checks_bounced: Vec::new(),
                cartel_evictions: Vec::new(),
                projects_completed: 0,
                project_orders_clamped: 0,
                total_upkeep_shortfall_debt: 0,
                total_spoilage_clawed_back: 0,
                broken_promises: Vec::new(),
                elected_officers: None,
                charters_awarded: Vec::new(),
                cred_debt_repaid: 0,
                charter_cred_burned: 0,
                town_import_cred_spent: 0,
                works_stipend_cred_minted: 0,
                batch_trades_cleared: 0,
                fills: Vec::new(),
                wash_freeze_settlements: Vec::new(),
            },
        });
        must_ok(
            session.apply_seat_intent(
                Seat(0),
                SeatIntent::request_what_if(HumanWhatIfRequest {
                    seat: 0,
                    market_actions: Vec::new(),
                    cancel_lock_at: None,
                    bundle: SealedOrderBundle::default(),
                    tie_mode: PreviewTieMode::NeutralSeed,
                    assumptions: league_core::CounterpartyAssumptions::default(),
                }),
            ),
            "apply RequestWhatIf through host scrub path",
        );
        let stored_wi = must_some(
            must_some(session.seat_ui_state(Seat(0)), "ui0").what_if_preview(),
            "stored what-if preview",
        );
        assert!(
            stored_wi.refused_market_actions[0]
                .message
                .contains("[REDACTED]")
        );
        assert!(
            !stored_wi.refused_market_actions[0]
                .message
                .contains("sk-ant-secret_999")
        );
        assert!(
            !stored_wi.refused_market_actions[0]
                .message
                .contains("sEd7N5k8mQp2rT9vW3xY6zA4bC1d")
        );

        must_ok(session.handle_key_event(key('3'), vp_wide), "tab 3");
        let mut r_txt_80 = String::new();
        for _ in 0..4 {
            must_ok(
                term_80.draw(|f| must_ok(session.render_current_frame(f), "draw refusal 80x24")),
                "draw refusal 80x24",
            );
            r_txt_80.push_str(&screen_text(&term_80));
            r_txt_80.push('\n');
            must_ok(
                session.handle_key_event(key_code(SeatKeyCode::PageDown), vp_small),
                "scroll down to preview refusal",
            );
        }
        assert!(r_txt_80.contains("Refused Action #0: [CANCEL_LOCKED]"));
        assert!(r_txt_80.contains("[REDACTED]"));
        assert!(!r_txt_80.contains("sk-ant-secret_999"));
        assert!(!r_txt_80.contains("sEd7N5k8mQp2rT9vW3xY6zA4bC1d"));

        must_ok(
            session.handle_key_event(key_code(SeatKeyCode::Home), vp_wide),
            "reset scroll",
        );
        must_ok(
            term_100.draw(|f| must_ok(session.render_current_frame(f), "draw refusal 100x30")),
            "draw refusal 100x30",
        );
        let r_txt_100 = screen_text(&term_100);
        assert!(r_txt_100.contains("Refused Action #0: [CANCEL_LOCKED]"));
        assert!(r_txt_100.contains("[REDACTED]"));
        assert!(!r_txt_100.contains("sk-ant-secret_999"));

        // 4. All 61 commodity display catalog pairs + numeraire render verbatim without redaction at 80x24 & 100x30.
        let ui_raw = SeatUiState::new();
        let ui_refined = league_seat_ui::handle_seat_key(
            &obs,
            ui_raw.clone(),
            PassKind::SubPassA,
            None,
            vp_wide,
            key_code(SeatKeyCode::Down),
        )
        .state;
        let ui_refined = league_seat_ui::handle_seat_key(
            &obs,
            ui_refined,
            PassKind::SubPassA,
            None,
            vp_wide,
            key_code(SeatKeyCode::Right),
        )
        .state;
        for entry in COMMODITY_DISPLAY_CATALOG {
            let cmap = CommodityDisplayMap {
                numeraire_code: "CRED".to_owned(),
                numeraire_label: format_numeraire_display(false).to_owned(),
                seats: (0..4_u8)
                    .map(|seat| league_seat_types::SeatCommodityDisplay {
                        seat,
                        raw_code: entry.code.to_owned(),
                        raw_label: entry.raw_display(false),
                        refined_code: entry.code.to_owned(),
                        refined_label: entry.refined_display(false),
                    })
                    .collect(),
            };
            let v_raw = league_seat_ui::render_turn_screen(
                &obs,
                &ui_raw,
                PassKind::SubPassA,
                Some(&cmap),
                vp_wide,
            );
            must_ok(term_100.draw(|f| draw_screen_view(f, &v_raw)), "draw raw");
            let buf_raw = screen_text(&term_100);
            assert!(
                buf_raw.contains(&entry.raw_display(false)),
                "raw commodity `{}` missing from 100x30 TUI buffer",
                entry.raw_display(false)
            );
            assert!(!buf_raw.contains("[REDACTED]"));

            let v_ref = league_seat_ui::render_turn_screen(
                &obs,
                &ui_refined,
                PassKind::SubPassA,
                Some(&cmap),
                vp_wide,
            );
            must_ok(term_100.draw(|f| draw_screen_view(f, &v_ref)), "draw ref");
            let buf_ref = screen_text(&term_100);
            assert!(
                buf_ref.contains(&entry.refined_display(false)),
                "refined commodity `{}` missing from 100x30 TUI buffer",
                entry.refined_display(false)
            );
            assert!(!buf_ref.contains("[REDACTED]"));
        }

        // Verify unused imports aren't flagged by referencing TableMode / SeatConfig / deal_graded_mandates_with_entropy.
        let _ = (
            TableMode::Club,
            SeatConfig {
                seat: 0,
                identity: SeatIdentity::Human,
                initial_cred: 10_000,
                raw_yield_per_turn: 10_000,
                refined_project_value: 2_000,
                upkeep_good: 3,
                upkeep_amount: 1_000,
                initial_raw_tariff_bps: 800,
                initial_refined_tariff_bps: 600,
                perishable_decay_bps: 2_000,
                regent: None,
                converters: Vec::new(),
            },
            deal_graded_mandates_with_entropy,
        );
    }

    #[test]
    fn no_active_human_seat_draws_public_only_standby_across_quit_season_end_and_error() {
        let mut failures = Vec::new();
        let vp = ViewportSize::new(100, 30);

        // (a) Non-first human seat (Seat 1) presses q on Turn 1.
        {
            let ring = dev_ring_table(1);
            let deal_seed = distinct_mandate_deal_seed(&ring, &[0, 1]);
            let (mut session, _, _) =
                build_explicit_sessions(&ring, TableId(81_101), &[0, 1], deal_seed, [0x77; 32], 51);
            let mut terminal = must_ok(Terminal::new(TestBackend::new(100, 30)), "100x30");
            must_ok(session.handle_key_event(ctrl_key('l'), vp), "lock seat 0");
            must_ok(session.handle_key_event(key(' '), vp), "curtain space");
            must_ok(
                session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
                "curtain enter",
            );
            session.advance_clock(CURTAIN_LIFT_DEAD_TIME);
            assert_eq!(session.active_human_seat(), Some(Seat(1)));
            must_ok(session.handle_key_event(key('q'), vp), "seat 1 quit");
            must_ok(
                terminal.draw(|f| must_ok(session.render_current_frame(f), "draw post-q a")),
                "draw post-q a",
            );
            let txt = screen_text(&terminal);
            if txt.contains("Private Vault") || txt.contains("Private Mandate") {
                failures.push(format!("(a) post-quit leaked Private Vault:\n{txt}"));
            }
            if !txt.contains("Session Standby") {
                failures.push("(a) post-quit missing neutral Session Standby view".to_owned());
            }
        }

        // (b) Single human Seat 0 quits on Turn 2 (where offset 0 is Seat 1 ClubBot).
        {
            let ring = dev_ring_table(1);
            let deal_seed = distinct_mandate_deal_seed(&ring, &[0]);
            let (mut session, _, _) =
                build_explicit_sessions(&ring, TableId(81_102), &[0], deal_seed, [0x77; 32], 53);
            let mut terminal = must_ok(Terminal::new(TestBackend::new(100, 30)), "100x30");
            must_ok(session.handle_key_event(ctrl_key('l'), vp), "lock T1 A");
            must_ok(session.handle_key_event(key(' '), vp), "space T1 B");
            must_ok(
                session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
                "enter T1 B",
            );
            session.advance_clock(CURTAIN_LIFT_DEAD_TIME);
            must_ok(session.handle_key_event(ctrl_key('l'), vp), "lock T1 B");
            must_ok(session.handle_key_event(key(' '), vp), "space T2 A");
            must_ok(
                session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
                "enter T2 A",
            );
            session.advance_clock(CURTAIN_LIFT_DEAD_TIME);
            assert_eq!(session.turn(), 2);
            must_ok(session.handle_key_event(key('q'), vp), "seat 0 quit T2");
            must_ok(
                terminal.draw(|f| must_ok(session.render_current_frame(f), "draw post-q b")),
                "draw post-q b",
            );
            let txt = screen_text(&terminal);
            if txt.contains("Private Vault") || txt.contains("Private Mandate") {
                failures.push(format!("(b) T2 post-quit leaked Private Vault:\n{txt}"));
            }
            if !txt.contains("Session Standby") {
                failures.push("(b) T2 post-quit missing neutral Session Standby view".to_owned());
            }
        }

        // (c) Season end (`TurnPhase::AwaitingMandateReveal`) after final lock.
        {
            let ring = dev_ring_table(1);
            let deal_seed = distinct_mandate_deal_seed(&ring, &[0]);
            let (mut session, _, _) =
                build_explicit_sessions(&ring, TableId(81_103), &[0], deal_seed, [0x77; 32], 55);
            let mut terminal = must_ok(Terminal::new(TestBackend::new(100, 30)), "100x30");
            while session.phase() == TurnPhase::TalkAndMarket {
                unlock_curtain_and_capture(&mut session, &mut terminal, vp, &mut Vec::new());
                must_ok(session.handle_key_event(ctrl_key('l'), vp), "advance turn");
            }
            assert_eq!(session.phase(), TurnPhase::AwaitingMandateReveal);
            must_ok(
                terminal.draw(|f| must_ok(session.render_current_frame(f), "draw season end")),
                "draw season end",
            );
            let txt = screen_text(&terminal);
            if txt.contains("Private Vault") || txt.contains("Private Mandate") {
                failures.push(format!("(c) season-end leaked Private Vault:\n{txt}"));
            }
            if !txt.contains("AwaitingMandateReveal") {
                failures
                    .push("(c) season-end missing AwaitingMandateReveal standby status".to_owned());
            }
        }

        // (d) Forced error inside `advance_after_human_lock` (invalid counterparty bundle in SubPassB).
        {
            let ring = dev_ring_table(1);
            let deal_seed = distinct_mandate_deal_seed(&ring, &[0]);
            let (mut session, _, _) =
                build_explicit_sessions(&ring, TableId(81_104), &[0], deal_seed, [0x77; 32], 57);
            let mut terminal = must_ok(Terminal::new(TestBackend::new(100, 30)), "100x30");
            must_ok(session.handle_key_event(ctrl_key('l'), vp), "lock T1 A");
            must_ok(session.handle_key_event(key(' '), vp), "space T1 B");
            must_ok(
                session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
                "enter T1 B",
            );
            session.advance_clock(CURTAIN_LIFT_DEAD_TIME);
            assert_eq!(session.pass(), PassKind::SubPassB);
            session.pending_bundles[0].contract_deliveries.push(
                league_seat_types::ContractDeliveryOrder {
                    counterparty: 99,
                    good: AssetId::Raw(0),
                    gross_amount: 100,
                    domain_id: None,
                    dual_escrow_id: None,
                },
            );
            let err = session.handle_key_event(ctrl_key('l'), vp);
            assert!(
                err.is_err(),
                "forced advance_after_human_lock error must return Err"
            );
            assert!(session.active_human_seat().is_none());
            must_ok(
                terminal.draw(|f| must_ok(session.render_current_frame(f), "draw after error")),
                "draw after error",
            );
            let txt = screen_text(&terminal);
            if txt.contains("Private Vault") || txt.contains("Private Mandate") {
                failures.push(format!("(d) forced error leaked Private Vault:\n{txt}"));
            }
            if !txt.contains("Session Standby") || !txt.contains("REFUSED [") {
                failures.push("(d) forced error missing standby or REFUSED banner".to_owned());
            }
        }

        assert!(
            failures.is_empty(),
            "active-seat-less draws leaked private data:\n{}",
            failures.join("\n---\n")
        );
    }

    #[test]
    fn season_leak_assertion_catches_clipped_mandate_unrealized_shock_and_state_digest_plants() {
        let vp = ViewportSize::new(100, 30);
        let ring = dev_ring_table(1);
        let deal_seed = distinct_mandate_deal_seed(&ring, &[0, 1]);
        let (session, _, _) =
            build_explicit_sessions(&ring, TableId(81_109), &[0, 1], deal_seed, [0x77; 32], 59);
        let mandates = vec![
            build_mandate_leak_spec(&league_core::PrivateMandate {
                kind: MandateKind::MinProjectEquity { target: 24_225 },
                bonus_equity: 7_250,
            }),
            build_mandate_leak_spec(&league_core::PrivateMandate {
                kind: MandateKind::MinRefinedVariety {
                    distinct_goods: 2,
                    min_each: 1_500,
                },
                bonus_equity: 4_500,
            }),
        ];
        let _ = session.turn();

        // 1. Clipped mandate plant in 24-col drawer at 100x30 (`>=24225` clips to `>=242`).
        let clipped_view = SeatScreenView {
            layout: league_seat_types::ScreenLayout::layout_for_viewport(vp),
            zones: vec![
                ViewZonePanel {
                    zone: ViewZoneId::HeaderBar,
                    panel: ViewPanel {
                        title: Some(ViewLabel::plain("Header".to_owned())),
                        bordered: true,
                        ..ViewPanel::default()
                    },
                },
                ViewZonePanel {
                    zone: ViewZoneId::PublicLedger,
                    panel: ViewPanel {
                        title: Some(ViewLabel::plain("Ledger".to_owned())),
                        bordered: true,
                        ..ViewPanel::default()
                    },
                },
                ViewZonePanel {
                    zone: ViewZoneId::CenterTabs,
                    panel: ViewPanel {
                        title: Some(ViewLabel::plain("Center".to_owned())),
                        bordered: true,
                        ..ViewPanel::default()
                    },
                },
                ViewZonePanel {
                    zone: ViewZoneId::PrivateDrawer,
                    panel: ViewPanel {
                        title: Some(ViewLabel::plain(
                            "Seat 0 Private Vault & Diagnostics".to_owned(),
                        )),
                        bordered: true,
                        alignment: ViewAlign::Left,
                        rows: vec![
                            ViewRow::from_plain("Private Mandate:".to_owned()),
                            ViewRow::from_plain("  MinProjectEquity >=24225".to_owned()),
                        ],
                        ..ViewPanel::default()
                    },
                },
            ],
            status_line: Some(ViewStatusLine::default()),
            modal: None,
        };
        let mut terminal = must_ok(Terminal::new(TestBackend::new(100, 30)), "100x30");
        must_ok(
            terminal.draw(|f| draw_screen_view(f, &clipped_view)),
            "draw clipped mandate",
        );
        let clipped_buf = screen_text(&terminal);
        assert!(clipped_buf.contains("MinProjectEquity >=242"));
        assert!(!clipped_buf.contains("MinProjectEquity >=24225"));

        // Verify buffer-only check catches the clipped prefix even with view_text empty.
        let err_mandate = must_err(
            check_season_leaks(
                &[CapturedSeasonDraw {
                    turn: 1,
                    active_seat: Some(1),
                    view_text: String::new(),
                    buffer_text: clipped_buf,
                }],
                &mandates,
                &[],
                &[],
                &[],
            ),
            "clipped mandate plant must fail",
        );
        assert_eq!(
            err_mandate,
            "Turn 1 seat Some(1) buffer leaked seat 0's clipped mandate `MinProjectEquity >=242`"
        );

        // 2. Unrealized shock magnitude plant (`6002bps` and `60.02%` before realize_turn = 5).
        let err_shock = must_err(
            check_season_leaks(
                &[CapturedSeasonDraw {
                    turn: 1,
                    active_seat: Some(0),
                    view_text: "Shock 6002bps".to_owned(),
                    buffer_text: "Shock 6002bps".to_owned(),
                }],
                &mandates,
                &[],
                &[],
                &[(5, 3, 6_002)],
            ),
            "unrealized shock plant must fail",
        );
        assert_eq!(
            err_shock,
            "Turn 1 seat Some(0) leaked unrealized shock magnitude `6002bps` (realizes T5)"
        );

        let err_shock_pct = must_err(
            check_season_leaks(
                &[CapturedSeasonDraw {
                    turn: 1,
                    active_seat: Some(0),
                    view_text: "Shock 60.02%".to_owned(),
                    buffer_text: "Shock 60.02%".to_owned(),
                }],
                &mandates,
                &[],
                &[],
                &[(5, 3, 6_002)],
            ),
            "unrealized shock percentage plant must fail",
        );
        assert_eq!(
            err_shock_pct,
            "Turn 1 seat Some(0) leaked unrealized shock percentage `60.02%` (realizes T5)"
        );

        // Static Closing Window 35.00% caption is NOT flagged as an unrealized 3500 bps shock.
        must_ok(
            check_season_leaks(
                &[CapturedSeasonDraw {
                    turn: 24,
                    active_seat: Some(0),
                    view_text: "Closing Window Active: 35.00% roll each Sweep".to_owned(),
                    buffer_text: "Closing Window Active: 35.00% roll each Sweep".to_owned(),
                }],
                &mandates,
                &[],
                &[],
                &[(26, 25, 3_500)],
            ),
            "closing window 35.00% caption must not false-positive",
        );

        // MinActivePartners rendered token (`ActivePartners >= N`) is caught when leaked to rival seat.
        let partner_mandate = vec![build_mandate_leak_spec(&league_core::PrivateMandate {
            kind: MandateKind::MinActivePartners { min_partners: 3 },
            bonus_equity: 3_000,
        })];
        let err_partner = must_err(
            check_season_leaks(
                &[CapturedSeasonDraw {
                    turn: 1,
                    active_seat: Some(1),
                    view_text: "ActivePartners >= 3".to_owned(),
                    buffer_text: "ActivePartners >= 3".to_owned(),
                }],
                &partner_mandate,
                &[],
                &[],
                &[],
            ),
            "MinActivePartners rendered token must fail",
        );
        assert!(err_partner.contains("ActivePartners >= 3"));

        // 3. State digest plant (`[0xAB; 32]`) using an 8-hex-character prefix (`abababab`).
        let planted_digest = [0xAB_u8; 32];
        let digest_prefix: String = planted_digest
            .iter()
            .take(4)
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(digest_prefix.len(), 8);
        let err_digest = must_err(
            check_season_leaks(
                &[CapturedSeasonDraw {
                    turn: 1,
                    active_seat: Some(0),
                    view_text: format!("digest {digest_prefix}"),
                    buffer_text: format!("digest {digest_prefix}"),
                }],
                &mandates,
                &[],
                &[planted_digest],
                &[],
            ),
            "8-hex state digest prefix plant must fail",
        );
        assert_eq!(
            err_digest,
            "Turn 1 seat Some(0) leaked post_state_digest `abababababababababababababababababababababababababababababababab`"
        );
    }

    #[test]
    fn every_zone_row_is_drawn_or_scrollable_across_supported_viewports_and_seat_counts() {
        fn trimmed_prefix(s: &str, max_chars: usize) -> String {
            s.chars().take(max_chars).collect::<String>()
        }

        for seat_count in [SeatCount::Four, SeatCount::Six] {
            let ring = dev_ring_table_for_seats(1, seat_count);
            for (w, h) in [(80_u16, 24_u16), (99, 30), (100, 29), (100, 30)] {
                let mut session = must_ok(
                    TuiHostSession::from_ring_table(&ring, TableId(81_110), &[0], 61),
                    "session",
                );
                let vp = ViewportSize::new(w, h);
                let mut terminal = must_ok(Terminal::new(TestBackend::new(w, h)), "tb");
                // Run a What-If first so Tab 3 includes caveat + preview rows too.
                must_ok(session.handle_key_event(key('p'), vp), "seed what-if");
                must_ok(session.handle_key_event(key('1'), vp), "back to tab 1");

                let wide = w >= 100 && h >= 30;
                let inner_w_ledger = if wide {
                    ((usize::from(w) * 30) / 100).saturating_sub(3)
                } else {
                    usize::from(w).saturating_sub(2)
                };
                let inner_w_center = if wide {
                    ((usize::from(w) * 44) / 100).saturating_sub(3)
                } else {
                    usize::from(w).saturating_sub(2)
                };
                let inner_w_drawer = if wide {
                    ((usize::from(w) * 26) / 100).saturating_sub(3)
                } else {
                    usize::from(w).saturating_sub(2)
                };

                // HeaderBar and StatusLine must appear in full without scrolling.
                let view0 = must_ok(session.build_current_view(vp), "view0");
                must_ok(terminal.draw(|f| draw_screen_view(f, &view0)), "draw view0");
                let buf0 = screen_text(&terminal);
                let hdr = must_some(view0.zone(ViewZoneId::HeaderBar), "HeaderBar");
                for row in &hdr.rows {
                    let prefix =
                        trimmed_prefix(&row.plain_text(), usize::from(w).saturating_sub(2));
                    assert!(
                        buf0.contains(&prefix),
                        "{w}x{h} ({seat_count:?}) missing HeaderBar row `{prefix}`:\n{buf0}"
                    );
                }
                let st = must_some(view0.status_line.as_ref(), "StatusLine");
                for span in &st.left {
                    let prefix = trimmed_prefix(&span.text, usize::from(w));
                    assert!(
                        buf0.contains(&prefix),
                        "{w}x{h} missing StatusLine left `{prefix}`"
                    );
                }
                for span in &st.right {
                    let prefix = trimmed_prefix(&span.text, usize::from(w));
                    assert!(
                        buf0.contains(&prefix),
                        "{w}x{h} missing StatusLine right `{prefix}`"
                    );
                }

                // Focus PublicLedger (1 F6 press from CenterTabs -> PrivateDrawer, 2 -> PublicLedger).
                must_ok(
                    session.handle_key_event(key_code(SeatKeyCode::F(6)), vp),
                    "f6->drawer",
                );
                must_ok(
                    session.handle_key_event(key_code(SeatKeyCode::F(6)), vp),
                    "f6->ledger",
                );
                let ledger_panel =
                    must_some(view0.zone(ViewZoneId::PublicLedger), "PublicLedger").clone();
                let ledger_total = ledger_panel
                    .scroll
                    .map_or(ledger_panel.rows.len(), |s| s.total);
                let mut ledger_combined = String::new();
                for _ in 0..=ledger_total {
                    must_ok(
                        terminal.draw(|f| must_ok(session.render_current_frame(f), "draw ledger")),
                        "draw ledger",
                    );
                    ledger_combined.push_str(&screen_text(&terminal));
                    ledger_combined.push('\n');
                    must_ok(
                        session.handle_key_event(key_code(SeatKeyCode::Down), vp),
                        "scroll ledger down",
                    );
                }
                for s_idx in 0..usize::from(session.seat_count()) {
                    assert!(
                        ledger_combined.contains(&format!("S{s_idx}")),
                        "{w}x{h} ({seat_count:?}) PublicLedger unreachable seat row S{s_idx}"
                    );
                }
                for row in &ledger_panel.rows {
                    let prefix = trimmed_prefix(&row.plain_text(), inner_w_ledger.min(24));
                    if !prefix.trim().is_empty() {
                        assert!(
                            ledger_combined.contains(&prefix),
                            "{w}x{h} ({seat_count:?}) PublicLedger unreachable row `{prefix}`"
                        );
                    }
                }
                must_ok(
                    session.handle_key_event(key_code(SeatKeyCode::Home), vp),
                    "ledger home",
                );

                // Focus PrivateDrawer (2 F6 presses from PublicLedger -> CenterTabs -> PrivateDrawer).
                must_ok(
                    session.handle_key_event(key_code(SeatKeyCode::F(6)), vp),
                    "f6->center",
                );
                must_ok(
                    session.handle_key_event(key_code(SeatKeyCode::F(6)), vp),
                    "f6->drawer",
                );
                let drawer_panel =
                    must_some(view0.zone(ViewZoneId::PrivateDrawer), "PrivateDrawer").clone();
                let drawer_total = drawer_panel
                    .scroll
                    .map_or(drawer_panel.rows.len(), |s| s.total);
                let mut drawer_combined = String::new();
                for _ in 0..=drawer_total {
                    must_ok(
                        terminal.draw(|f| must_ok(session.render_current_frame(f), "draw drawer")),
                        "draw drawer",
                    );
                    drawer_combined.push_str(&screen_text(&terminal));
                    drawer_combined.push('\n');
                    must_ok(
                        session.handle_key_event(key_code(SeatKeyCode::Down), vp),
                        "scroll drawer down",
                    );
                }
                for row in &drawer_panel.rows {
                    let prefix = trimmed_prefix(&row.plain_text(), inner_w_drawer.min(20));
                    if !prefix.trim().is_empty() {
                        assert!(
                            drawer_combined.contains(&prefix),
                            "{w}x{h} ({seat_count:?}) PrivateDrawer unreachable row `{prefix}`"
                        );
                    }
                }
                must_ok(
                    session.handle_key_event(key_code(SeatKeyCode::Home), vp),
                    "drawer home",
                );

                // Focus CenterTabs and verify Tabs 1, 2, 3 rows are all reachable by scroll.
                must_ok(
                    session.handle_key_event(key_code(SeatKeyCode::F(6)), vp),
                    "f6->center",
                );
                for tab_char in ['1', '2', '3'] {
                    must_ok(session.handle_key_event(key(tab_char), vp), "switch tab");
                    must_ok(
                        session.handle_key_event(key_code(SeatKeyCode::Home), vp),
                        "tab home",
                    );
                    let tab_view = must_ok(session.build_current_view(vp), "tab view");
                    let tab_panel =
                        must_some(tab_view.zone(ViewZoneId::CenterTabs), "CenterTabs").clone();
                    let tab_total = tab_panel.scroll.map_or(tab_panel.rows.len(), |s| s.total);
                    let mut tab_combined = String::new();
                    for _ in 0..=(tab_total + 4) {
                        must_ok(
                            terminal.draw(|f| must_ok(session.render_current_frame(f), "draw tab")),
                            "draw tab",
                        );
                        tab_combined.push_str(&screen_text(&terminal));
                        tab_combined.push('\n');
                        must_ok(
                            session.handle_key_event(key_code(SeatKeyCode::Down), vp),
                            "scroll tab down",
                        );
                    }
                    for row in &tab_panel.rows {
                        let raw = row.plain_text();
                        let trimmed = raw
                            .strip_prefix("> ")
                            .or_else(|| raw.strip_prefix("  "))
                            .unwrap_or(&raw);
                        let prefix =
                            trimmed_prefix(trimmed, inner_w_center.saturating_sub(2).min(26));
                        if !prefix.trim().is_empty() {
                            assert!(
                                tab_combined.contains(&prefix),
                                "{w}x{h} ({seat_count:?}) CenterTab {tab_char} unreachable row `{prefix}`"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn shift_l_does_not_lock_and_handoff_while_ctrl_l_does() {
        let ring = dev_ring_table(1);
        let mut session = must_ok(
            TuiHostSession::from_ring_table(&ring, TableId(81_106), &[0, 1], 63),
            "2-human",
        );
        let vp = ViewportSize::new(100, 30);
        must_ok(
            session.handle_key_event(
                SeatKeyEvent::new(
                    SeatKeyCode::Char('L'),
                    SeatKeyModifiers {
                        ctrl: false,
                        alt: false,
                        shift: true,
                    },
                ),
                vp,
            ),
            "Shift-L",
        );
        assert!(
            !session.is_curtain_open(),
            "Shift-L alone must not lock and open the Curtain"
        );
        must_ok(session.handle_key_event(ctrl_key('l'), vp), "Ctrl-L");
        assert!(
            session.is_curtain_open(),
            "Ctrl-L must lock and open the Curtain"
        );
    }

    #[test]
    fn intents_with_mismatched_extra_payloads_are_refused_with_zero_state_change() {
        let ring = dev_ring_table(1);
        let mut session = must_ok(
            TuiHostSession::from_ring_table(&ring, TableId(81_107), &[0, 1], 65),
            "2-human",
        );
        let obs_before = must_ok(session.build_all_seat_observations(), "obs before");
        let ui0_before = must_some(session.seat_ui_state(Seat(0)), "ui0 before").clone();
        let ui1_before = must_some(session.seat_ui_state(Seat(1)), "ui1 before").clone();
        let b0_before = must_some(session.pending_bundle(Seat(0)), "b0 before").clone();
        let b1_before = must_some(session.pending_bundle(Seat(1)), "b1 before").clone();
        let errs_before = session.last_errors.clone();

        let dummy_action = MarketAction::PostOffer {
            seat: 0,
            order: OfferProposal {
                side: OfferSide::Buy,
                good: AssetId::Raw(1),
                amount: 100,
                price_per_unit_cred: 1_000,
                domain_id: None,
                is_hybrid: false,
            },
        };
        let dummy_bundle = BundleDraftAction::SetDebtRepaymentCred(100);
        let dummy_what_if = HumanWhatIfRequest {
            seat: 0,
            market_actions: Vec::new(),
            cancel_lock_at: None,
            bundle: SealedOrderBundle::default(),
            tie_mode: PreviewTieMode::NeutralSeed,
            assumptions: league_core::CounterpartyAssumptions::default(),
        };

        let bad_intents = vec![
            SeatIntent {
                kind: SeatIntentKind::SubmitMarketAction,
                market_action: Some(dummy_action.clone()),
                bundle_action: Some(dummy_bundle.clone()),
                what_if: None,
            },
            SeatIntent {
                kind: SeatIntentKind::SubmitMarketAction,
                market_action: Some(dummy_action.clone()),
                bundle_action: None,
                what_if: Some(dummy_what_if.clone()),
            },
            SeatIntent {
                kind: SeatIntentKind::UpdateBundle,
                market_action: Some(dummy_action.clone()),
                bundle_action: Some(dummy_bundle.clone()),
                what_if: None,
            },
            SeatIntent {
                kind: SeatIntentKind::RequestWhatIf,
                market_action: None,
                bundle_action: Some(dummy_bundle.clone()),
                what_if: Some(dummy_what_if.clone()),
            },
            SeatIntent {
                kind: SeatIntentKind::LockAndHandOff,
                market_action: Some(dummy_action.clone()),
                bundle_action: None,
                what_if: None,
            },
            SeatIntent {
                kind: SeatIntentKind::OpenChronicleModal,
                market_action: None,
                bundle_action: Some(dummy_bundle),
                what_if: None,
            },
            SeatIntent {
                kind: SeatIntentKind::CloseChronicleModal,
                market_action: None,
                bundle_action: None,
                what_if: Some(dummy_what_if),
            },
            SeatIntent {
                kind: SeatIntentKind::Quit,
                market_action: Some(dummy_action),
                bundle_action: None,
                what_if: None,
            },
        ];

        for intent in bad_intents {
            let err = must_err(
                session.apply_seat_intent(Seat(0), intent),
                "intent with extra payload must fail",
            );
            assert_eq!(err.code, "UNEXPECTED_INTENT_PAYLOAD");
        }

        assert_eq!(
            must_ok(session.build_all_seat_observations(), "obs after"),
            obs_before
        );
        assert_eq!(
            must_some(session.seat_ui_state(Seat(0)), "ui0 after"),
            &ui0_before
        );
        assert_eq!(
            must_some(session.seat_ui_state(Seat(1)), "ui1 after"),
            &ui1_before
        );
        assert_eq!(
            must_some(session.pending_bundle(Seat(0)), "b0 after"),
            &b0_before
        );
        assert_eq!(
            must_some(session.pending_bundle(Seat(1)), "b1 after"),
            &b1_before
        );
        assert_eq!(session.last_errors, errs_before);
        assert!(!session.is_curtain_open());
        assert!(!session.quit_requested());
    }

    #[test]
    fn refusal_banner_is_visible_and_scrubbed_at_80x24_100x30_and_120x40() {
        let ring = dev_ring_table(1);
        let mut session = must_ok(
            TuiHostSession::from_ring_table(&ring, TableId(81_108), &[0], 67),
            "1-human",
        );
        session.record_scrubbed_host_error(
            "HOST_ERR",
            "leaked sEd7N5k8mQp2rT9vW3xY6zA4bC1d sk-proj-secret123",
        );
        for (w, h) in [(80_u16, 24_u16), (100, 30), (120, 40)] {
            let mut terminal = must_ok(Terminal::new(TestBackend::new(w, h)), "tb");
            must_ok(
                terminal.draw(|f| must_ok(session.render_current_frame(f), "draw banner")),
                "draw banner",
            );
            let txt = screen_text(&terminal);
            assert!(
                txt.contains("REFUSED [HOST_ERR]:"),
                "refusal banner missing at {w}x{h}:\n{txt}"
            );
            assert!(txt.contains("[REDACTED]"), "missing scrub at {w}x{h}");
            assert!(!txt.contains("sEd7N5k8mQp2rT9vW3xY6zA4bC1d"));
            assert!(!txt.contains("sk-proj-secret123"));
        }
    }

    fn format_expected_ledger_cells(
        st: &league_seat_types::SeatState,
        w: u16,
        h: u16,
    ) -> (String, String, &'static str, String) {
        let tier = match st.credit_tier {
            league_seat_types::CreditTier::Aaa => "Aaa",
            league_seat_types::CreditTier::Bbb => "Bbb",
            league_seat_types::CreditTier::B => "B",
            league_seat_types::CreditTier::D => "D",
        };
        let format_units = |v: i64, max_chars: usize| -> String {
            let exact = v.to_string();
            if exact.chars().count() <= max_chars {
                return exact;
            }
            let neg = v < 0;
            let abs = v.unsigned_abs();
            let sign = if neg { "-" } else { "" };
            for (div, suffix) in [
                (1_000_000_000_u64, 'B'),
                (1_000_000_u64, 'M'),
                (1_000_u64, 'k'),
            ] {
                if abs < div {
                    continue;
                }
                let whole = abs / div;
                let rem = abs % div;
                let tenths = (rem * 10) / div;
                let mut cands = Vec::new();
                if rem == 0 {
                    cands.push(format!("{sign}{whole}{suffix}"));
                } else {
                    if rem * 10 == tenths * div {
                        cands.push(format!("{sign}{whole}.{tenths}{suffix}"));
                    } else {
                        cands.push(format!("{sign}{whole}.{tenths}{suffix}+"));
                    }
                    cands.push(format!("{sign}{whole}{suffix}+"));
                    cands.push(format!("{sign}{whole}{suffix}"));
                }
                for cand in cands {
                    if cand.chars().count() <= max_chars {
                        return cand;
                    }
                }
            }
            exact
        };
        let (cw, dw, tw) = if w >= 100 && h >= 30 {
            if ((usize::from(w) * 30) / 100).saturating_sub(2) >= 34 {
                (7, 6, 9)
            } else {
                (6, 4, 9)
            }
        } else {
            (8, 7, 9)
        };
        let cred = format_units(st.cred, cw);
        let debt = format_units(st.cred_debt, dw);
        let raw_tar = format!("{}/{}", st.raw_tariff_bps, st.refined_tariff_bps);
        let tar = if raw_tar.chars().count() <= tw {
            raw_tar
        } else {
            let side = (tw - 1) / 2;
            format!(
                "{}/{}",
                format_units(i64::from(st.raw_tariff_bps), side),
                format_units(i64::from(st.refined_tariff_bps), side)
            )
        };
        (cred, debt, tier, tar)
    }

    #[test]
    fn public_ledger_cells_show_whole_or_abbreviated_values_at_80x24_100x30_and_120x40() {
        for seat_count in [SeatCount::Four, SeatCount::Six] {
            let ring = dev_ring_table_for_seats(1, seat_count);
            let mut session = must_ok(
                TuiHostSession::from_ring_table(&ring, TableId(81_201), &[0], 71),
                "session",
            );
            // Drive one market trade + Turn 1 sweep so credits, tariffs, and potentially debt
            // reflect realistic non-zero figures across all seats.
            must_ok(
                session.handle_key_event(key_code(SeatKeyCode::Enter), ViewportSize::new(100, 30)),
                "submit default offer",
            );
            must_ok(
                session.handle_key_event(ctrl_key('l'), ViewportSize::new(100, 30)),
                "lock T1 pass A",
            );
            unlock_curtain_and_capture(
                &mut session,
                &mut must_ok(Terminal::new(TestBackend::new(100, 30)), "tb"),
                ViewportSize::new(100, 30),
                &mut Vec::new(),
            );
            must_ok(
                session.handle_key_event(ctrl_key('l'), ViewportSize::new(100, 30)),
                "lock T1 pass B",
            );
            unlock_curtain_and_capture(
                &mut session,
                &mut must_ok(Terminal::new(TestBackend::new(100, 30)), "tb"),
                ViewportSize::new(100, 30),
                &mut Vec::new(),
            );

            let obs = must_ok(session.build_seat_observation(Seat(0)), "seat 0 obs");
            let mut seat0_100x30_row = None;
            for (w, h) in [(80_u16, 24_u16), (100, 30), (120, 40)] {
                let vp = ViewportSize::new(w, h);
                let mut terminal = must_ok(Terminal::new(TestBackend::new(w, h)), "tb");
                let mut combined_buf = String::new();
                if w >= 100 && h >= 30 {
                    must_ok(
                        terminal.draw(|f| must_ok(session.render_current_frame(f), "draw wide")),
                        "draw wide",
                    );
                    combined_buf = screen_text(&terminal);
                    if (w, h) == (100, 30) && seat_count == SeatCount::Four {
                        seat0_100x30_row = combined_buf
                            .lines()
                            .find(|l| l.contains("S0 ") && (l.contains("Aaa") || l.contains("Bbb")))
                            .map(|l| l.trim().to_owned());
                    }
                } else {
                    // At 80x24 StackedFallback the PublicLedger pane shows 2 table rows per scroll
                    // step; focus PublicLedger (F6 x 2) and scroll through every seat row.
                    must_ok(
                        session.handle_key_event(key_code(SeatKeyCode::F(6)), vp),
                        "f6 1",
                    );
                    must_ok(
                        session.handle_key_event(key_code(SeatKeyCode::F(6)), vp),
                        "f6 2",
                    );
                    for _ in 0..obs.seats.len() {
                        must_ok(
                            terminal
                                .draw(|f| must_ok(session.render_current_frame(f), "draw 80x24")),
                            "draw 80x24",
                        );
                        combined_buf.push_str(&screen_text(&terminal));
                        combined_buf.push('\n');
                        must_ok(
                            session.handle_key_event(key_code(SeatKeyCode::Down), vp),
                            "scroll ledger down",
                        );
                    }
                    must_ok(
                        session.handle_key_event(key_code(SeatKeyCode::Home), vp),
                        "home",
                    );
                    must_ok(
                        session.handle_key_event(key_code(SeatKeyCode::F(6)), vp),
                        "f6 back",
                    );
                }
                assert!(
                    combined_buf.lines().any(|l| l.contains("S0")),
                    "{w}x{h} ({seat_count:?}) missing S0 ledger row:\n{combined_buf}"
                );

                for st in &obs.seats {
                    let (cred, debt, tier, tar) = format_expected_ledger_cells(st, w, h);
                    let row_line = combined_buf
                        .lines()
                        .find(|l| {
                            l.contains(&format!("S{} ", st.seat))
                                && l.contains(tier)
                                && l.contains(&tar)
                        })
                        .unwrap_or_else(|| {
                            panic!(
                                "{w}x{h} ({seat_count:?}) missing complete S{} row (expected cred=`{cred}`, debt=`{debt}`, tier=`{tier}`, tar=`{tar}`):\n{combined_buf}",
                                st.seat
                            )
                        });
                    assert!(
                        row_line.contains(&cred),
                        "{w}x{h} ({seat_count:?}) S{} expected cred `{cred}` in row `{row_line}`",
                        st.seat
                    );
                    assert!(
                        row_line.contains(&debt),
                        "{w}x{h} ({seat_count:?}) S{} expected debt `{debt}` in row `{row_line}`",
                        st.seat
                    );
                    assert!(
                        row_line.contains(&tar),
                        "{w}x{h} ({seat_count:?}) S{} expected tariff `{tar}` in row `{row_line}`",
                        st.seat
                    );
                    // Ensure no truncation drops leading digits on large credit balances.
                    let full_cred = st.cred.to_string();
                    if cred == full_cred {
                        assert!(
                            row_line.contains(&full_cred),
                            "{w}x{h} ({seat_count:?}) S{} lost digits from `{full_cred}` in `{row_line}`",
                            st.seat
                        );
                    }
                }
            }
            if seat_count == SeatCount::Four {
                let captured_row = must_some(seat0_100x30_row, "seat 0 100x30 row");
                eprintln!("VERIFIED_100X30_SEAT0_ROW={captured_row}");
            }
        }

        // Explicitly verify exact 15000 credits and 1245/708 tariff cells from the card brief at
        // 80x24, 100x30, and 120x40 (even when tariffs/balances sit right on column limits).
        let ring = dev_ring_table(1);
        let session = must_ok(
            TuiHostSession::from_ring_table(&ring, TableId(81_202), &[0], 73),
            "ring session",
        );
        let mut obs = must_ok(session.build_seat_observation(Seat(0)), "obs");
        obs.seats[0].cred = 15_000;
        obs.seats[0].cred_debt = 2_450;
        obs.seats[0].raw_tariff_bps = 1_245;
        obs.seats[0].refined_tariff_bps = 708;
        obs.seats[1].cred = 1_500_000;
        obs.seats[1].cred_debt = 15_000;
        obs.seats[1].raw_tariff_bps = 500;
        obs.seats[1].refined_tariff_bps = 250;
        let state = SeatUiState::new();
        for (w, h) in [(80_u16, 24_u16), (100, 30), (120, 40)] {
            let vp = ViewportSize::new(w, h);
            let view =
                league_seat_ui::render_turn_screen(&obs, &state, PassKind::SubPassA, None, vp);
            let mut terminal = must_ok(Terminal::new(TestBackend::new(w, h)), "tb");
            must_ok(
                terminal.draw(|f| draw_screen_view(f, &view)),
                "draw synthetic",
            );
            let buf = screen_text(&terminal);
            assert!(
                buf.contains("15000"),
                "{w}x{h} must display whole `15000`:\n{buf}"
            );
            assert!(
                buf.contains("1245/708"),
                "{w}x{h} must display whole `1245/708`:\n{buf}"
            );
            assert!(
                buf.contains("15k") || buf.contains("15000"),
                "{w}x{h} must show debt 15000 whole or as unambiguous `15k`:\n{buf}"
            );
            assert!(
                buf.contains("1500k") || buf.contains("1.5M") || buf.contains("1500000"),
                "{w}x{h} must show 1.5M whole or unambiguous abbreviation:\n{buf}"
            );
            assert!(
                !buf.contains("│ S0   00 ") && !buf.contains("│S0   00 "),
                "{w}x{h} cut leading digits to `00`:\n{buf}"
            );
        }
    }

    #[test]
    fn halted_session_shows_accurate_standby_notes_refuses_wedged_save_and_from_host_surfaces_bot_errors()
     {
        let vp = ViewportSize::new(100, 30);
        let ring = dev_ring_table(1);
        let deal_seed = distinct_mandate_deal_seed(&ring, &[0]);
        let (mut session, _, _) =
            build_explicit_sessions(&ring, TableId(81_203), &[0], deal_seed, [0x77; 32], 75);
        must_ok(session.handle_key_event(ctrl_key('l'), vp), "lock A");
        unlock_curtain_and_capture(
            &mut session,
            &mut must_ok(Terminal::new(TestBackend::new(100, 30)), "tb"),
            vp,
            &mut Vec::new(),
        );
        session.pending_bundles[0].contract_deliveries.push(
            league_seat_types::ContractDeliveryOrder {
                counterparty: 99,
                good: AssetId::Raw(0),
                gross_amount: 100,
                domain_id: None,
                dual_escrow_id: None,
            },
        );
        let lock_err = must_err(
            session.handle_key_event(ctrl_key('l'), vp),
            "wedging session via invalid delivery counterparty",
        );
        assert!(!lock_err.code.is_empty());
        assert!(session.is_wedged());

        let standby = must_ok(session.build_current_view(vp), "wedged standby");
        let txt = standby.plain_text();
        assert!(
            !txt.contains("bots or sweep idle"),
            "halted session must not show misleading idle standby note:\n{txt}"
        );
        assert!(
            txt.contains("Pass advance failed") && txt.contains("REFUSED ["),
            "halted session must describe error standby truthfully:\n{txt}"
        );

        let dir = std::env::temp_dir().join(format!("ml_wedged_save_{}", std::process::id()));
        let save_path = dir.join("wedged.save.json");
        let seal_path = dir.join("wedged.seal");
        let save_err = must_err(
            session.write_save_and_seal(&save_path, &seal_path),
            "wedged session must refuse save",
        );
        assert_eq!(save_err.code, "SESSION_WEDGED");
        assert!(!save_path.exists());
        assert!(!seal_path.exists());

        // `TuiHostSession::from_host` propagates bot-advance / host startup errors scrubbed instead
        // of silently discarding with `let _ =`.
        let ring2 = dev_ring_table(1);
        let (spec, _, deal, eval_seed) =
            league_gen::ring_table_to_table_spec(&ring2, TableId(81_204), &[1])
                .unwrap_or_else(|e| panic!("ring spec: {e}"));
        let bad_host = ClubHost::new(spec, ring2.seed, deal, eval_seed, 77)
            .unwrap_or_else(|e| panic!("bad_host: {e}"));
        let err_from_host = must_err(
            TuiHostSession::from_host_with_extra_bot_action(
                bad_host,
                None,
                Some(MarketAction::CancelOffer {
                    seat: 0,
                    offer_id: 999_999,
                }),
            ),
            "from_host must preserve bot/host advance error",
        );
        assert_eq!(err_from_host.code, "UNKNOWN_OBJECT");
    }

    #[test]
    fn mid_turn_save_and_resume_at_curtains_matches_uninterrupted_receipts_keeps_secrets_in_seal_and_wipes_seal()
     {
        let vp = ViewportSize::new(100, 30);
        let ring = dev_ring_table(1);
        let deal_seed = distinct_mandate_deal_seed(&ring, &[0, 1, 2]);
        let salts: Vec<RevealSalt> = (0..ring.seat_count.as_u8())
            .map(|i| RevealSalt([0x31_u8.wrapping_add(i); 32]))
            .collect();
        let seed_eval = must_ok(SealedEvalSeed::try_from_bytes([0x61; 32]), "eval seed");
        let (spec, cmap, deal, eval_seed) = ring_table_to_table_spec_with_explicit_entropy(
            &ring,
            TableId(81_401),
            &[0, 1, 2],
            deal_seed,
            &salts,
            seed_eval,
        )
        .unwrap_or_else(|e| panic!("ring spec: {e}"));

        // 1. Baseline uninterrupted run across 3 human seats + 1 bot (`Seat(3)`).
        let mut baseline = must_ok(
            TuiHostSession::from_explicit_setup(
                spec.clone(),
                ring.seed,
                Some(cmap.to_display_map()),
                deal.clone(),
                eval_seed.clone(),
                83,
            ),
            "baseline",
        );
        let mut term = must_ok(Terminal::new(TestBackend::new(100, 30)), "tb");

        // Sub-Pass A Seat 0 -> post default offer + private note -> Lock -> Seat 1 -> Lock -> Seat 2 Curtain.
        type_text_in_mode(&mut baseline, 'n', "NOTE_S0_SECRET_771", vp);
        must_ok(
            baseline.handle_key_event(key_code(SeatKeyCode::Enter), vp),
            "baseline s0 offer",
        );
        must_ok(
            baseline.handle_key_event(ctrl_key('l'), vp),
            "baseline s0 lock",
        );
        unlock_curtain_and_capture(&mut baseline, &mut term, vp, &mut Vec::new());
        must_ok(
            baseline.handle_key_event(ctrl_key('l'), vp),
            "baseline s1 lock",
        );
        unlock_curtain_and_capture(&mut baseline, &mut term, vp, &mut Vec::new());
        must_ok(
            baseline.handle_key_event(ctrl_key('l'), vp),
            "baseline s2 lock",
        );
        // Now at Turn 1 Sub-Pass B Seat 0 Curtain.
        unlock_curtain_and_capture(&mut baseline, &mut term, vp, &mut Vec::new());
        assert_eq!(baseline.pass(), PassKind::SubPassB);
        assert_eq!(baseline.active_human_seat(), Some(Seat(0)));
        must_ok(
            baseline.apply_seat_intent(
                Seat(0),
                SeatIntent::update_bundle(BundleDraftAction::SetDebtRepaymentCred(731)),
            ),
            "baseline s0 debt bundle",
        );
        must_ok(
            baseline.handle_key_event(ctrl_key('l'), vp),
            "baseline s0 lock B",
        );
        // Now at Turn 1 Sub-Pass B Seat 1 Curtain -> unlock & finish rest of season cleanly.
        while baseline.phase() == TurnPhase::TalkAndMarket {
            unlock_curtain_and_capture(&mut baseline, &mut term, vp, &mut Vec::new());
            if baseline.active_human_seat().is_some() {
                must_ok(
                    baseline.handle_key_event(ctrl_key('l'), vp),
                    "baseline lock",
                );
            }
        }
        assert_eq!(baseline.phase(), TurnPhase::AwaitingMandateReveal);

        // 2. Paused run: enable autosave, pause at Sub-Pass A Seat 2 Curtain, resume, then pause at
        // Sub-Pass B Seat 1 Curtain, resume, and drive to season close.
        let dir = std::env::temp_dir().join(format!("ml_partB2a_resume_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let save_path = dir.join("club-81401.save.json");
        let seal_path = dir.join("club-81401.seal");

        let mut run1 = must_ok(
            TuiHostSession::from_explicit_setup(
                spec,
                ring.seed,
                Some(cmap.to_display_map()),
                deal,
                eval_seed,
                83,
            ),
            "run1",
        );
        run1.enable_autosave(save_path.clone(), seal_path.clone());
        type_text_in_mode(&mut run1, 'n', "NOTE_S0_SECRET_771", vp);
        must_ok(
            run1.handle_key_event(key_code(SeatKeyCode::Enter), vp),
            "run1 s0 offer",
        );
        must_ok(run1.handle_key_event(ctrl_key('l'), vp), "run1 s0 lock A");
        unlock_curtain_and_capture(&mut run1, &mut term, vp, &mut Vec::new());
        must_ok(run1.handle_key_event(ctrl_key('l'), vp), "run1 s1 lock A");
        // Paused right at Sub-Pass A Seat 2 Curtain (`pass_offset == 2`).
        assert!(run1.is_curtain_open());
        assert_eq!(run1.pass(), PassKind::SubPassA);
        assert!(save_path.exists() && seal_path.exists());

        let save1_txt =
            std::fs::read_to_string(&save_path).unwrap_or_else(|e| panic!("read save1: {e}"));
        let seal1_txt =
            std::fs::read_to_string(&seal_path).unwrap_or_else(|e| panic!("read seal1: {e}"));
        assert!(!save1_txt.contains("NOTE_S0_SECRET_771"));
        assert!(seal1_txt.contains("NOTE_S0_SECRET_771"));

        let mut run2 = must_ok(
            TuiHostSession::read_save_and_seal(&save_path, &seal_path, Some(cmap.to_display_map())),
            "resume at Pass A Seat 2 Curtain",
        );
        run2.enable_autosave(save_path.clone(), seal_path.clone());
        assert_eq!(run2.turn(), 1);
        assert_eq!(run2.pass(), PassKind::SubPassA);
        assert!(run2.is_curtain_open());
        unlock_curtain_and_capture(&mut run2, &mut term, vp, &mut Vec::new());
        assert_eq!(run2.active_human_seat(), Some(Seat(2)));
        must_ok(run2.handle_key_event(ctrl_key('l'), vp), "run2 s2 lock A");

        // Now in Sub-Pass B Seat 0 Curtain -> unlock Seat 0 -> verify note replayed + stage 731.
        unlock_curtain_and_capture(&mut run2, &mut term, vp, &mut Vec::new());
        assert_eq!(run2.pass(), PassKind::SubPassB);
        assert_eq!(run2.active_human_seat(), Some(Seat(0)));
        assert_eq!(
            must_some(run2.seat_ui_state(Seat(0)), "s0 ui").notes_draft(),
            "NOTE_S0_SECRET_771",
            "Seat 0 private note must survive hand-offs and save/resume via .seal"
        );
        must_ok(
            run2.apply_seat_intent(
                Seat(0),
                SeatIntent::update_bundle(BundleDraftAction::SetDebtRepaymentCred(731)),
            ),
            "run2 s0 debt bundle",
        );
        must_ok(run2.handle_key_event(ctrl_key('l'), vp), "run2 s0 lock B");
        // Now paused at Sub-Pass B Seat 1 Curtain! Autosave ran when Curtain opened.
        assert_eq!(run2.pass(), PassKind::SubPassB);
        assert!(run2.is_curtain_open());

        let save2_txt =
            std::fs::read_to_string(&save_path).unwrap_or_else(|e| panic!("read save2: {e}"));
        let seal2_txt =
            std::fs::read_to_string(&seal_path).unwrap_or_else(|e| panic!("read seal2: {e}"));
        let s0_mandate_dbg = format!(
            "{:?}",
            must_some(
                run2.build_seat_observation(Seat(0))
                    .ok()
                    .and_then(|o| o.own_mandate().copied()),
                "s0 m"
            )
        );
        let _ = &s0_mandate_dbg;
        assert!(!save2_txt.contains("NOTE_S0_SECRET_771"));
        assert!(!save2_txt.contains("731"));
        assert!(!save2_txt.contains("debt_repayment_cred"));
        assert!(!save2_txt.contains("sealed_mandate_deal"));
        assert!(!save2_txt.contains("seat_reveals"));
        assert!(!save2_txt.contains("mandate_commits"));
        assert!(!save2_txt.contains("mandate_reveals"));
        assert!(!save2_txt.contains("vrf_proofs"));
        assert!(!save2_txt.contains("current_turn_salts"));
        assert!(seal2_txt.contains("NOTE_S0_SECRET_771"));
        assert!(seal2_txt.contains("731"));
        assert!(seal2_txt.contains("sealed_mandate_deal"));

        let mut run3 = must_ok(
            TuiHostSession::read_save_and_seal(&save_path, &seal_path, Some(cmap.to_display_map())),
            "resume at Pass B Seat 1 Curtain",
        );
        run3.enable_autosave(save_path.clone(), seal_path.clone());
        assert_eq!(run3.turn(), 1);
        assert_eq!(run3.pass(), PassKind::SubPassB);
        assert!(run3.is_curtain_open());
        assert_eq!(
            must_some(run3.pending_bundle(Seat(0)), "s0 staged bundle").debt_repayment_cred,
            731
        );
        // Active season cannot wipe `.seal`.
        let active_wipe_err = must_err(
            run3.wipe_seal_after_season_export(&seal_path),
            "cannot wipe .seal mid-season",
        );
        assert_eq!(active_wipe_err.code, "SEASON_NOT_FINISHED");

        while run3.phase() == TurnPhase::TalkAndMarket {
            unlock_curtain_and_capture(&mut run3, &mut term, vp, &mut Vec::new());
            if run3.active_human_seat().is_some() {
                must_ok(run3.handle_key_event(ctrl_key('l'), vp), "run3 lock");
            }
        }
        assert_eq!(run3.phase(), TurnPhase::AwaitingMandateReveal);
        assert_eq!(
            run3.public_receipts(),
            baseline.public_receipts(),
            "mid-turn paused/resumed season must produce byte-identical PublicSweepReceipts"
        );

        must_ok(
            run3.wipe_seal_after_season_export(&seal_path),
            "wipe .seal after season close/export",
        );
        assert!(
            !seal_path.exists(),
            ".seal must be deleted after season export"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn commit_salts_draw_from_sealed_entropy_differ_across_identical_public_tables_stay_out_of_save_json_and_replay_verifies()
     {
        let ring = dev_ring_table(1);
        let deal_seed = distinct_mandate_deal_seed(&ring, &[0]);
        let salts: Vec<RevealSalt> = (0..ring.seat_count.as_u8())
            .map(|i| RevealSalt([0x21_u8.wrapping_add(i); 32]))
            .collect();
        let seed_eval = must_ok(SealedEvalSeed::try_from_bytes([0x55; 32]), "eval seed");
        let (spec, _, deal, eval_seed) = ring_table_to_table_spec_with_explicit_entropy(
            &ring,
            TableId(81_402),
            &[0],
            deal_seed,
            &salts,
            seed_eval,
        )
        .unwrap_or_else(|e| panic!("ring spec: {e}"));

        let mut session_a = must_ok(
            TuiHostSession::from_explicit_setup(
                spec.clone(),
                ring.seed,
                None,
                deal.clone(),
                eval_seed.clone(),
                91,
            ),
            "session A",
        );
        let session_b = must_ok(
            TuiHostSession::from_explicit_setup(spec, ring.seed, None, deal, eval_seed, 91),
            "session B",
        );

        let salt_a = must_ok(
            ClubSessionCheckpoint::turn_reveal_salt(&session_a.host, Seat(0)),
            "salt A",
        );
        let salt_b = must_ok(
            ClubSessionCheckpoint::turn_reveal_salt(&session_b.host, Seat(0)),
            "salt B",
        );
        assert_ne!(salt_a, RevealSalt([0u8; 32]));
        assert_ne!(salt_b, RevealSalt([0u8; 32]));
        assert_ne!(
            salt_a, salt_b,
            "identical public tables must draw independent commit salts from .seal entropy"
        );

        let salt_a_hex: String = salt_a.0.iter().map(|b| format!("{b:02x}")).collect();
        let dir = std::env::temp_dir().join(format!("ml_salt_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let save_path = dir.join("salt.save.json");
        let seal_path = dir.join("salt.seal");
        must_ok(
            session_a.write_save_and_seal(&save_path, &seal_path),
            "save salt table",
        );
        let save_txt =
            std::fs::read_to_string(&save_path).unwrap_or_else(|e| panic!("read salt save: {e}"));
        let seal_txt =
            std::fs::read_to_string(&seal_path).unwrap_or_else(|e| panic!("read salt seal: {e}"));
        assert!(
            seal_txt.contains(&salt_a_hex),
            "sealed turn reveal salt hex must be persisted in .seal"
        );
        assert!(
            !save_txt.contains(&salt_a_hex),
            "commit salt hex must never appear in .save.json"
        );

        // Drive session A to season close and verify replay log verifies cleanly.
        let vp = ViewportSize::new(100, 30);
        let mut term = must_ok(Terminal::new(TestBackend::new(100, 30)), "tb");
        while session_a.phase() == TurnPhase::TalkAndMarket {
            unlock_curtain_and_capture(&mut session_a, &mut term, vp, &mut Vec::new());
            if session_a.active_human_seat().is_some() {
                must_ok(session_a.handle_key_event(ctrl_key('l'), vp), "lock");
            }
        }
        let (board, log, _, _) = must_ok(
            session_a.host.finalize_club_season(),
            "finalize club season",
        );
        let replayed = must_ok(league_core::replay_season(&log), "replay_season");
        assert_eq!(replayed.scoreboard, board);
        assert!(!log.turns.is_empty());
        assert_eq!(
            log.turns[0].reveals.get(&0).map(|r| r.salt.clone()),
            Some(salt_a),
            "Turn 1 reveal salt must match .seal-derived salt"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn optional_seat_pins_never_echo_build_zero_observations_on_wrong_tries_return_to_curtain_on_third_miss_and_stay_in_seal_only()
     {
        let vp = ViewportSize::new(100, 30);
        let ring = dev_ring_table(1);
        let dir = std::env::temp_dir().join(format!("ml_pins_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let save_path = dir.join("club-81501.save.json");
        let seal_path = dir.join("club-81501.seal");

        let mut session = must_ok(
            TuiHostSession::from_ring_table(&ring, TableId(81_501), &[0, 1], 97),
            "pins session",
        );
        session.enable_autosave(save_path.clone(), seal_path.clone());
        session.enable_seat_pins();

        let mut term = must_ok(Terminal::new(TestBackend::new(100, 30)), "tb");
        assert!(
            session.is_curtain_open(),
            "first seat must open at Curtain to set its PIN before any observation is built"
        );
        session.reset_observe_seat_calls();

        // Arm Seat 0's Curtain and set PIN `"4829"`. Verify digits never echo in the buffer.
        must_ok(session.handle_key_event(key(' '), vp), "arm s0 curtain");
        must_ok(
            session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
            "open s0 pin prompt",
        );
        for d in ['4', '8', '2', '9'] {
            must_ok(session.handle_key_event(key(d), vp), "type s0 pin digit");
            must_ok(
                term.draw(|f| must_ok(session.render_current_frame(f), "draw pin entry")),
                "draw pin entry",
            );
            let buf = screen_text(&term);
            assert!(
                !buf.contains("4829"),
                "PIN digits must never echo in Curtain buffer:\n{buf}"
            );
        }
        assert!(
            screen_text(&term).contains("[****]"),
            "Curtain must show 4 masked stars while digits stay hidden"
        );
        assert_eq!(
            session.observe_seat_call_count(),
            0,
            "setting Curtain PIN must build zero SeatObservations before Enter completes"
        );
        must_ok(
            session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
            "confirm s0 pin",
        );
        assert!(!session.is_curtain_open());
        session.advance_clock(CURTAIN_LIFT_DEAD_TIME);
        let obs0 = must_ok(session.build_seat_observation(Seat(0)), "s0 obs");
        let s0_mandate = mandate_short_marker(must_some(obs0.own_mandate(), "s0 mandate").kind);
        let s0_mandate_clip: String = s0_mandate.chars().take(20).collect();

        // Lock Seat 0 -> Seat 1 sets its PIN `"7135"`.
        must_ok(
            session.handle_key_event(ctrl_key('l'), vp),
            "lock s0 pass A",
        );
        must_ok(session.handle_key_event(key(' '), vp), "arm s1 curtain");
        must_ok(
            session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
            "open s1 pin",
        );
        for d in ['7', '1', '3', '5'] {
            must_ok(session.handle_key_event(key(d), vp), "type s1 pin");
        }
        must_ok(
            term.draw(|f| must_ok(session.render_current_frame(f), "draw s1 pin")),
            "draw s1 pin",
        );
        assert!(!screen_text(&term).contains("7135"));
        must_ok(
            session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
            "confirm s1 pin",
        );
        session.advance_clock(CURTAIN_LIFT_DEAD_TIME);

        // Verify `.save.json` contains neither PIN while `.seal` holds both PINs.
        let save_txt =
            std::fs::read_to_string(&save_path).unwrap_or_else(|e| panic!("read pin save: {e}"));
        let seal_txt =
            std::fs::read_to_string(&seal_path).unwrap_or_else(|e| panic!("read pin seal: {e}"));
        assert!(!save_txt.contains("4829") && !save_txt.contains("7135"));
        assert!(!save_txt.contains("session_seat_pins"));
        assert!(seal_txt.contains("4829") && seal_txt.contains("7135"));

        // Seat 1 locks Pass A -> Pass B opens Curtain for Seat 0 (`VerifyPin`).
        must_ok(
            session.handle_key_event(ctrl_key('l'), vp),
            "lock s1 pass A",
        );
        assert!(session.is_curtain_open());
        assert_eq!(session.pass(), PassKind::SubPassB);

        // Reset observation counter: 3 wrong PIN attempts must build ZERO observations and return
        // to the unarmed Curtain after the 3rd miss.
        session.reset_observe_seat_calls();
        must_ok(session.handle_key_event(key(' '), vp), "arm s0 pass B");
        must_ok(
            session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
            "enter verify pin",
        );
        for bad_pin in ["0001", "0002", "0003"] {
            for ch in bad_pin.chars() {
                must_ok(session.handle_key_event(key(ch), vp), "type wrong pin");
            }
            must_ok(
                session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
                "submit wrong pin",
            );
            must_ok(
                term.draw(|f| must_ok(session.render_current_frame(f), "draw after wrong pin")),
                "draw after wrong pin",
            );
            let buf = screen_text(&term);
            assert!(
                !buf.contains(bad_pin),
                "wrong PIN digits must never appear in buffer:\n{buf}"
            );
            assert!(
                !buf.contains(&s0_mandate_clip) && !buf.contains("Private Mandate"),
                "wrong PIN try must leak zero seat private data:\n{buf}"
            );
            assert_eq!(
                session.observe_seat_call_count(),
                0,
                "wrong PIN `{bad_pin}` must build zero SeatObservations"
            );
            assert!(session.is_curtain_open());
        }
        let reset_buf = screen_text(&term);
        assert!(
            reset_buf.contains("PIN entry failed 3 times"),
            "3rd wrong PIN must reset back to unarmed Curtain notice:\n{reset_buf}"
        );
        // After 3rd miss, Curtain is unarmed (typing `Enter` or digits directly does not unlock).
        must_ok(
            session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
            "unarmed enter stays on Curtain",
        );
        assert!(session.is_curtain_open());
        assert_eq!(session.observe_seat_call_count(), 0);

        // Saving/resuming at this Curtain retains `.seal` PINs and enforces `VerifyPin` on resume.
        must_ok(
            session.write_save_and_seal(&save_path, &seal_path),
            "save with pin",
        );
        let mut resumed_pin_session = must_ok(
            TuiHostSession::read_save_and_seal(&save_path, &seal_path, None),
            "resume pin session",
        );
        resumed_pin_session.reset_observe_seat_calls();
        must_ok(
            resumed_pin_session.handle_key_event(key(' '), vp),
            "re-arm resumed after 3-miss",
        );
        must_ok(
            resumed_pin_session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
            "re-open verify pin",
        );
        for ch in "4829".chars() {
            must_ok(
                resumed_pin_session.handle_key_event(key(ch), vp),
                "type correct pin",
            );
        }
        must_ok(
            resumed_pin_session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
            "submit correct pin",
        );
        assert!(!resumed_pin_session.is_curtain_open());
        resumed_pin_session.advance_clock(CURTAIN_LIFT_DEAD_TIME);
        must_ok(
            term.draw(|f| {
                must_ok(
                    resumed_pin_session.render_current_frame(f),
                    "draw unlocked s0",
                )
            }),
            "draw unlocked s0",
        );
        assert!(
            resumed_pin_session.observe_seat_call_count() >= 1,
            "correct PIN unlock must build active SeatObservation"
        );
        let unlocked_buf = screen_text(&term);
        assert!(
            unlocked_buf.contains("Seat 0 Private Vault")
                && unlocked_buf.contains(&s0_mandate_clip),
            "unlocked Seat 0 must display own private vault:\n{unlocked_buf}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
