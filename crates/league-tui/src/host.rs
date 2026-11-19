//! Trusted Club Mode host session wrapper in the `league-tui` binary crate.
//!
//! Owns [`ClubHost`], two-pass hot-seat sequencing, `ClubBot` automation, strict host validation
//! of [`SeatIntent`]s, and generic [`ScreenLayout`] rendering via `ratatui` without putting any
//! seat view-model logic or forbidden engine symbols into `league-tui`.

use std::collections::BTreeMap;
use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use league_core::{
    AiPreviewRequest, ClubBot, ClubHost, EngineError, MarketAction, OrderReveal,
    PublicSweepReceipt, RevealSalt, SealedEvalSeed, SealedMandateDeal, SealedOrderBundle, Seat,
    SeatIdentity, SeatObservation, StructuredError, TableId, TableSpec, ToStructuredError, Turn,
    TurnPhase, preview_seat_bundle, preview_what_if, scrub_secret_text,
};
use league_gen::{GenError, RingTable, ring_table_to_table_spec};
use league_seat_types::{
    CommodityDisplayMap, SeatIntent, SeatIntentKind, SeatKeyCode, SeatKeyEvent, SeatKeyModifiers,
    SeatScreenView, ViewAlign, ViewAxis, ViewColor, ViewLayoutMode, ViewPanel, ViewSizeHint,
    ViewStyle, ViewTabBar, ViewTable, ViewZoneId, ViewportSize,
};
use league_seat_ui::{PassKind, SeatUiState};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table};

#[derive(Debug, Clone, PartialEq, Eq)]
struct CurtainState {
    next_seat: Seat,
    space_armed: bool,
}

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
            line_width += chip.chars().count();
            current.push(Span::styled(chip, st));
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

fn render_view_table(frame: &mut Frame<'_>, area: Rect, table: &ViewTable) -> u16 {
    if area.width == 0 || area.height < 2 || table.columns.is_empty() {
        return 0;
    }
    let desired_rows = 1_usize.saturating_add(table.rows.len());
    let allocated = u16::try_from(desired_rows)
        .unwrap_or(u16::MAX)
        .min(area.height);
    let widths: Vec<Constraint> = table
        .columns
        .iter()
        .map(|c| map_constraint(c.width))
        .collect();
    let header_cells: Vec<Cell<'_>> = table
        .columns
        .iter()
        .map(|col| {
            Cell::from(
                Line::from(Span::styled(
                    col.header.text.as_str(),
                    map_style(col.header.style).add_modifier(Modifier::BOLD),
                ))
                .alignment(map_align(col.alignment)),
            )
        })
        .collect();
    let header = Row::new(header_cells);
    let data_limit = usize::from(allocated.saturating_sub(1));
    let data_rows: Vec<Row<'_>> = table
        .rows
        .iter()
        .enumerate()
        .take(data_limit)
        .map(|(idx, row)| {
            let selected = table.selected_row == Some(idx);
            let cells: Vec<Cell<'_>> = row
                .cells
                .iter()
                .map(|cell| {
                    let spans: Vec<Span<'_>> = cell
                        .spans
                        .iter()
                        .map(|sp| {
                            let mut st = map_style(sp.style);
                            if selected {
                                st = st.add_modifier(Modifier::REVERSED);
                            }
                            Span::styled(sp.text.as_str(), st)
                        })
                        .collect();
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
    let table_rect = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: allocated,
    };
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
            block = block.title(Span::styled(title.text.as_str(), map_style(title.style)));
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

    if let Some(table) = &panel.table {
        let used = render_view_table(frame, inner, table);
        inner.y = inner.y.saturating_add(used);
        inner.height = inner.height.saturating_sub(used);
    }
    if inner.height == 0 || panel.rows.is_empty() {
        return;
    }

    let offset = panel.scroll.map_or(0, |s| s.offset);
    let lines: Vec<Line<'_>> = panel
        .rows
        .iter()
        .enumerate()
        .skip(offset)
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
                    Span::styled(span.text.as_str(), st)
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
    let mut spans: Vec<Span<'_>> = status
        .left
        .iter()
        .map(|sp| Span::styled(sp.text.as_str(), map_style(sp.style)))
        .collect();
    if !status.right.is_empty() {
        if !spans.is_empty() {
            spans.push(Span::styled(" | ", map_style(status.style)));
        }
        for sp in &status.right {
            spans.push(Span::styled(sp.text.as_str(), map_style(sp.style)));
        }
    }
    frame.render_widget(
        Paragraph::new(Text::from(vec![Line::from(spans)])).style(map_style(status.style)),
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

fn deterministic_reveal_salt(table_id: TableId, turn: u8, seat: u8) -> RevealSalt {
    let mut bytes = [0x5Au8; 32];
    let tid = table_id.0.to_le_bytes();
    bytes[0] = turn.max(1);
    bytes[1] = seat.saturating_add(1);
    bytes[2] = tid[0] ^ turn ^ 0xA5;
    bytes[3] = tid[1] ^ seat ^ 0x3C;
    bytes[31] = (turn.wrapping_add(seat)).max(1);
    RevealSalt(bytes)
}

/// Host session bridging a [`ClubHost`] table to seat-isolated `league-seat-ui` views and
/// executing sequential hot-seat passes (`SubPassA -> begin_cancel_lock -> SubPassB -> Sweep`).
#[derive(Debug)]
#[allow(dead_code)]
pub struct TuiHostSession {
    host: ClubHost,
    commodities: Option<CommodityDisplayMap>,
    human_seats: Vec<bool>,
    seat_states: Vec<SeatUiState>,
    pending_bundles: Vec<SealedOrderBundle>,
    last_errors: Vec<Option<StructuredError>>,
    pass: PassKind,
    pass_offset: usize,
    curtain: Option<CurtainState>,
    quit_requested: bool,
}

#[allow(dead_code)]
impl TuiHostSession {
    fn from_host(host: ClubHost, commodities: Option<CommodityDisplayMap>) -> Self {
        let spec = host.public_spec();
        let n = usize::from(host.seat_count());
        let human_seats: Vec<bool> = spec
            .seats
            .iter()
            .map(|cfg| cfg.identity == SeatIdentity::Human)
            .collect();
        let mut session = Self {
            host,
            commodities,
            human_seats,
            seat_states: vec![SeatUiState::new(); n],
            pending_bundles: vec![SealedOrderBundle::default(); n],
            last_errors: vec![None; n],
            pass: PassKind::SubPassA,
            pass_offset: 0,
            curtain: None,
            quit_requested: false,
        };
        if session.host.phase() == TurnPhase::TalkAndMarket
            && session.human_seats.iter().any(|&h| h)
        {
            let _ = session.advance_bots_without_curtain();
        }
        session
    }

    /// Bootstraps a Club Mode session on the canonical `TableSpec::standard_club_chiral_ring_with_setup`.
    pub fn from_standard_setup(
        table_id: TableId,
        seat_count: u8,
        layout_seed: u64,
        bot_tie_seed: u64,
    ) -> Result<Self, EngineError> {
        let host = ClubHost::from_standard_setup(table_id, seat_count, layout_seed, bot_tie_seed)?;
        Ok(Self::from_host(host, None))
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
        Ok(Self::from_host(host, Some(cmap.to_display_map())))
    }

    /// Creates a Club Mode session from explicit [`TableSpec`], [`SealedMandateDeal`], and
    /// [`SealedEvalSeed`] values for deterministic tests.
    pub fn from_explicit_setup(
        spec: TableSpec,
        layout_seed: u64,
        commodities: Option<CommodityDisplayMap>,
        mandate_deal: SealedMandateDeal,
        eval_seed: SealedEvalSeed,
        bot_tie_seed: u64,
    ) -> Result<Self, EngineError> {
        let host = ClubHost::new(spec, layout_seed, mandate_deal, eval_seed, bot_tie_seed)?;
        Ok(Self::from_host(host, commodities))
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

    /// Returns the active sub-pass (`SubPassA` or `SubPassB`).
    #[must_use]
    pub const fn pass(&self) -> PassKind {
        self.pass
    }

    /// Returns whether the Privacy Curtain is currently displayed between seat hand-offs.
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
    #[must_use]
    pub fn public_receipts(&self) -> &[PublicSweepReceipt] {
        self.host.public_receipts()
    }

    /// Returns an immutable reference to `seat`'s host-owned [`SeatUiState`].
    #[must_use]
    pub fn seat_ui_state(&self, seat: Seat) -> Option<&SeatUiState> {
        self.seat_states.get(usize::from(seat.0))
    }

    /// Returns the pending [`SealedOrderBundle`] staged for `seat`.
    #[must_use]
    pub fn pending_bundle(&self, seat: Seat) -> Option<&SealedOrderBundle> {
        self.pending_bundles.get(usize::from(seat.0))
    }

    /// Returns the seat currently controlling the keyboard (`None` when Curtain is up, season is
    /// closed, or quit was requested).
    #[must_use]
    pub fn active_human_seat(&self) -> Option<Seat> {
        if self.curtain.is_some()
            || self.quit_requested
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
        Ok(Self::from_host(host, commodities))
    }

    /// Wipes a finished `.seal` sidecar cleanly.
    pub fn wipe_seal_file(seal_path: &Path) -> Result<(), EngineError> {
        ClubHost::wipe_seal_file(seal_path)
    }

    /// Records a scrubbed host-level error banner for the currently active seat.
    pub fn record_scrubbed_host_error(&mut self, code: &'static str, raw_msg: &str) {
        if let Some(active) = self.active_human_seat() {
            self.last_errors[usize::from(active.0)] = Some(host_error(code, raw_msg, None));
        }
    }

    /// Builds the plain owned [`SeatScreenView`] for either the Curtain hand-off or the active
    /// seat's [`SeatObservation`].
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
            ));
        }
        let target_seat = self
            .active_human_seat()
            .unwrap_or_else(|| self.seat_at_offset(0).unwrap_or(Seat(0)));
        let obs = self.host.observe_seat(target_seat)?;
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
            status.right.push(league_seat_types::ViewSpan::styled(
                format!(" | {banner}"),
                ViewStyle::plain().with_fg(ViewColor::Red).bold(),
            ));
        }
        Ok(view)
    }

    /// Renders the active view (Curtain or active seat's turn screen) onto `frame`.
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
            match key.code {
                SeatKeyCode::Char('c') if key.modifiers.ctrl => {
                    self.quit_requested = true;
                }
                SeatKeyCode::Char('q') | SeatKeyCode::Esc => {
                    self.quit_requested = true;
                }
                SeatKeyCode::Char(' ') => {
                    curtain.space_armed = true;
                }
                SeatKeyCode::Enter if curtain.space_armed => {
                    let next = curtain.next_seat;
                    self.curtain = None;
                    let idx = usize::from(next.0);
                    let synced = self.pending_bundles[idx].clone();
                    self.seat_states[idx].set_staged_bundle(synced);
                }
                _ => {
                    curtain.space_armed = false;
                }
            }
            return Ok(());
        }

        let Some(active) = self.active_human_seat() else {
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
            .host
            .observe_seat(active)
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
        if let Some(intent) = step.intent
            && let Err(err) = self.apply_seat_intent(active, intent)
        {
            self.last_errors[idx] = Some(err.clone());
            return Err(err);
        }
        Ok(())
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
                    .host
                    .observe_seat(submitting_seat)
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
                    .host
                    .observe_seat(submitting_seat)
                    .map_err(|err| scrub_engine_error(&err))?;
                let mut preview = preview_what_if(&obs.to_public_preview_state(), &req)
                    .map_err(|err| scrub_engine_error(&err))?;
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
                self.seat_states[idx].clear_for_hand_off();
                self.last_errors[idx] = None;
                self.pass_offset += 1;
                self.advance_after_human_lock()
            }
        }
    }

    /// Builds one [`SeatObservation`] **by value** for `seat`, requests the Part A view model, and
    /// draws it onto `frame` via `ratatui`.
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
                self.pending_bundles[usize::from(seat.0)] = bundle;
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
                    self.curtain = Some(CurtainState {
                        next_seat: seat,
                        space_armed: false,
                    });
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
                    let table_id = self.host.table_id();
                    let turn = self.host.turn();
                    let mut reveals = BTreeMap::new();
                    for s in 0..self.host.seat_count() {
                        let bundle = std::mem::take(&mut self.pending_bundles[usize::from(s)]);
                        reveals.insert(
                            s,
                            OrderReveal {
                                bundle,
                                salt: deterministic_reveal_salt(table_id, turn, s),
                            },
                        );
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
                    if self.host.phase() != TurnPhase::TalkAndMarket {
                        self.curtain = None;
                        return Ok(());
                    }
                    self.pass = PassKind::SubPassA;
                    self.pass_offset = 0;
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
            let mut tags = BTreeSet::new();
            for rev in &deal.seat_reveals {
                tags.insert(format!("{:?}", rev.mandate.kind));
            }
            if tags.len() == usize::from(spec.seat_count) {
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

    fn unlock_curtain_and_capture(
        session: &mut TuiHostSession,
        terminal: &mut Terminal<TestBackend>,
        vp: ViewportSize,
        frames: &mut Vec<(u8, Option<u8>, String)>,
    ) {
        if !session.is_curtain_open() {
            return;
        }
        must_ok(
            terminal.draw(|f| must_ok(session.render_current_frame(f), "curtain draw 1")),
            "draw curtain 1",
        );
        frames.push((session.turn(), None, screen_text(terminal)));
        must_ok(session.handle_key_event(key(' '), vp), "curtain space");
        must_ok(
            terminal.draw(|f| must_ok(session.render_current_frame(f), "curtain draw 2")),
            "draw curtain armed",
        );
        frames.push((session.turn(), None, screen_text(terminal)));
        must_ok(
            session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
            "curtain enter",
        );
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

    fn assert_no_season_leaks(
        frames: &[(u8, Option<u8>, String)],
        mandate_markers: &[String],
        salt_hexes: &[String],
        digests: &[[u8; 32]],
        drawn_shock_bps_by_turn: &[(u8, u8, league_core::Bps)],
    ) {
        assert!(!frames.is_empty(), "expected at least one captured frame");
        let digest_hexes: Vec<String> = digests
            .iter()
            .map(|d| d.iter().map(|b| format!("{b:02x}")).collect::<String>())
            .collect();

        for (frame_turn, active_seat, txt) in frames {
            for (seat_idx, marker) in mandate_markers.iter().enumerate() {
                let allowed = active_seat.is_some_and(|s| usize::from(s) == seat_idx);
                if !allowed {
                    assert!(
                        !txt.contains(marker),
                        "Turn {frame_turn} seat {active_seat:?} leaked seat {seat_idx}'s mandate `{marker}`"
                    );
                }
            }
            for salt_hex in salt_hexes {
                assert!(
                    !txt.contains(salt_hex),
                    "Turn {frame_turn} seat {active_seat:?} leaked mandate salt hex"
                );
            }
            for dhex in &digest_hexes {
                assert!(
                    !txt.contains(dhex),
                    "Turn {frame_turn} seat {active_seat:?} leaked post_state_digest `{dhex}`"
                );
            }
            for &(realize_turn, _forecast_turn, mag_bps) in drawn_shock_bps_by_turn {
                if *frame_turn < realize_turn {
                    let needle = format!("{mag_bps}bps");
                    assert!(
                        !txt.contains(&needle),
                        "Turn {frame_turn} seat {active_seat:?} leaked unrealized shock magnitude `{needle}` (realizes T{realize_turn})"
                    );
                }
            }
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
        assert!(wide_txt.contains("1-3 Tabs"));

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
        let mut frames: Vec<(u8, Option<u8>, String)> = Vec::new();

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
            must_ok(
                terminal.draw(|f| must_ok(session.render_current_frame(f), "draw active")),
                "draw active",
            );
            frames.push((session.turn(), Some(0), screen_text(&terminal)));
            must_ok(
                session.handle_key_event(ctrl_key('l'), vp),
                "lock and handoff",
            );
        }

        assert_eq!(session.phase(), TurnPhase::AwaitingMandateReveal);
        assert!(session.public_receipts().len() >= 24);

        let (_sb, log, _sec, _met) =
            must_ok(session.host.finalize_club_season(), "finalize club season");
        let (replay_out, _) = must_ok(replay_season_with_state_predicates(&log), "replay log");
        let mandate_markers: Vec<String> = log
            .mandate_reveals
            .iter()
            .map(|r| mandate_short_marker(must_some(r.as_ref(), "revealed mandate").mandate.kind))
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
            &mandate_markers,
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
        let mut frames: Vec<(u8, Option<u8>, String)> = Vec::new();
        let n = usize::from(headless.seat_count());

        while session.phase() == TurnPhase::TalkAndMarket {
            let turn = headless.turn();
            let mut headless_bundles = vec![SealedOrderBundle::default(); n];

            // Sub-Pass A across rotating seat order.
            for offset in 0..n {
                let s = u8::try_from((usize::from(turn - 1) + offset) % n).unwrap_or(0);
                if s < 2 {
                    unlock_curtain_and_capture(&mut session, &mut terminal, vp, &mut frames);
                    assert_eq!(session.active_human_seat(), Some(Seat(s)));
                    if turn == 1 {
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
                    must_ok(
                        terminal.draw(|f| must_ok(session.render_current_frame(f), "draw human A")),
                        "draw human A",
                    );
                    frames.push((turn, Some(s), screen_text(&terminal)));
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
                        must_ok(
                            session.apply_seat_intent(Seat(s), SeatIntent::update_bundle(op)),
                            "tui stage converter",
                        );
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
                    must_ok(
                        terminal.draw(|f| must_ok(session.render_current_frame(f), "draw human B")),
                        "draw human B",
                    );
                    frames.push((turn, Some(s), screen_text(&terminal)));
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
                        salt: deterministic_reveal_salt(headless.table_id(), turn, s),
                    },
                );
            }
            must_ok(headless.commit_and_reveal_all(&reveals), "headless commit");
            must_ok(headless.sweep_turn(), "headless sweep");
        }

        assert_eq!(session.phase(), TurnPhase::AwaitingMandateReveal);
        assert_eq!(headless.phase(), TurnPhase::AwaitingMandateReveal);
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
        let mandate_markers: Vec<String> = log
            .mandate_reveals
            .iter()
            .map(|r| mandate_short_marker(must_some(r.as_ref(), "rev").mandate.kind))
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
            &mandate_markers,
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
        let vp = ViewportSize::new(220, 36);
        let mut terminal = must_ok(Terminal::new(TestBackend::new(220, 36)), "220x36");

        // 1. Secret-bearing host error banner reaches buffer strictly scrubbed.
        session.record_scrubbed_host_error(
            "HOST_ERR",
            "leaked sEd7N5k8mQp2rT9vW3xY6zA4bC1d OPENROUTER_API_KEY=sk-secret-value Bearer tok_live_999 sk-proj-secret123",
        );
        must_ok(
            terminal.draw(|f| must_ok(session.render_current_frame(f), "draw error banner")),
            "draw error banner",
        );
        let err_buf = screen_text(&terminal);
        assert!(err_buf.contains("REFUSED [HOST_ERR]:"));
        assert!(err_buf.contains("[REDACTED]"));
        for secret in [
            "sEd7N5k8mQp2rT9vW3xY6zA4bC1d",
            "sk-secret-value",
            "tok_live_999",
            "sk-proj-secret123",
        ] {
            assert!(!err_buf.contains(secret), "TUI leaked `{secret}`");
        }

        // 2. Display-text paths + Part A over-redaction phrases render verbatim on TestBackend
        // without triggering false-positive `[REDACTED]` scrubbing.
        let obs = must_ok(session.build_seat_observation(Seat(0)), "obs");
        let over_redaction_phrases = [
            "risk-or-reward",
            "ask-or-tell",
            "desk-proj-alpha",
            "SKU SK-PROJ-1234",
            "class=\"btn-bearer active\"",
            "the standard bearer of",
        ];
        let mut ui = SeatUiState::new();
        for phrase in over_redaction_phrases {
            let mut s = ui.clone();
            let step_n =
                league_seat_ui::handle_seat_key(&obs, s, PassKind::SubPassA, None, vp, key('n'));
            s = step_n.state;
            for ch in phrase.chars() {
                s = league_seat_ui::handle_seat_key(&obs, s, PassKind::SubPassA, None, vp, key(ch))
                    .state;
            }
            s = league_seat_ui::handle_seat_key(
                &obs,
                s,
                PassKind::SubPassA,
                None,
                vp,
                key_code(SeatKeyCode::Enter),
            )
            .state;
            let view = league_seat_ui::render_turn_screen(&obs, &s, PassKind::SubPassA, None, vp);
            must_ok(
                terminal.draw(|f| draw_screen_view(f, &view)),
                "draw over-redaction phrase",
            );
            let txt = screen_text(&terminal);
            assert!(
                txt.contains(phrase),
                "TUI buffer altered over-redaction phrase `{phrase}`"
            );
            assert!(!txt.contains("[REDACTED]"));
            ui = SeatUiState::new();
        }

        // All 61 commodity display catalog pairs + numeraire render verbatim without redaction.
        let dummy_state = obs.seats[0].clone();
        ui.set_what_if_preview(Some(WhatIfPreview {
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
                message: scrub_secret_text("bad key sk-ant-secret_999"),
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
        }));
        let ui =
            league_seat_ui::handle_seat_key(&obs, ui, PassKind::SubPassA, None, vp, key('3')).state;
        let view_refusal =
            league_seat_ui::render_turn_screen(&obs, &ui, PassKind::SubPassA, None, vp);
        must_ok(
            terminal.draw(|f| draw_screen_view(f, &view_refusal)),
            "draw refusal preview",
        );
        let r_txt = screen_text(&terminal);
        assert!(r_txt.contains("[REDACTED]"));
        assert!(!r_txt.contains("sk-ant-secret_999"));

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
            let v = league_seat_ui::render_turn_screen(
                &obs,
                &SeatUiState::new(),
                PassKind::SubPassA,
                Some(&cmap),
                vp,
            );
            must_ok(terminal.draw(|f| draw_screen_view(f, &v)), "draw commodity");
            let buf = screen_text(&terminal);
            assert!(
                buf.contains(&entry.refined_display(false)),
                "commodity `{}` missing from TUI buffer",
                entry.refined_display(false)
            );
            assert!(!buf.contains("[REDACTED]"));
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
}
