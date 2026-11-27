use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

use crate::commit::PrivateMandate;
use crate::observation::PublicSweepReceipt;
use crate::predicates_types::{SeatSeasonMetrics, SeatStatePredicates, TurnPredicateRecord};
use crate::scoring_types::FinalScoreboard;
use crate::types::{TableId, Turn};

/// Plain terminal palette color carried in a seat UI view model without depending on `ratatui`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
pub enum ViewColor {
    /// Inherit or reset to the host terminal default color.
    #[default]
    Reset,
    /// ANSI black.
    Black,
    /// ANSI red.
    Red,
    /// ANSI green.
    Green,
    /// ANSI yellow.
    Yellow,
    /// ANSI blue.
    Blue,
    /// ANSI magenta.
    Magenta,
    /// ANSI cyan.
    Cyan,
    /// ANSI gray.
    Gray,
    /// ANSI dark gray.
    DarkGray,
    /// ANSI bright red.
    LightRed,
    /// ANSI bright green.
    LightGreen,
    /// ANSI bright yellow.
    LightYellow,
    /// ANSI bright blue.
    LightBlue,
    /// ANSI bright magenta.
    LightMagenta,
    /// ANSI bright cyan.
    LightCyan,
    /// ANSI bright white.
    White,
    /// Explicit 24-bit RGB color.
    Rgb {
        /// Red channel (`0..=255`).
        r: u8,
        /// Green channel (`0..=255`).
        g: u8,
        /// Blue channel (`0..=255`).
        b: u8,
    },
}

/// Text style flags and colors carried as plain owned data.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
pub struct ViewStyle {
    /// Foreground color.
    pub fg: ViewColor,
    /// Background color.
    pub bg: ViewColor,
    /// Bold weight flag.
    pub bold: bool,
    /// Dimmed intensity flag.
    pub dim: bool,
    /// Italic text flag.
    pub italic: bool,
    /// Underlined text flag.
    pub underlined: bool,
    /// Reversed foreground/background colors flag.
    pub reversed: bool,
}

impl ViewStyle {
    /// Returns a plain unstyled [`ViewStyle`] with default terminal colors and no modifiers.
    #[must_use]
    pub const fn plain() -> Self {
        Self {
            fg: ViewColor::Reset,
            bg: ViewColor::Reset,
            bold: false,
            dim: false,
            italic: false,
            underlined: false,
            reversed: false,
        }
    }

    /// Sets the foreground color.
    #[must_use]
    pub const fn with_fg(mut self, fg: ViewColor) -> Self {
        self.fg = fg;
        self
    }

    /// Sets the background color.
    #[must_use]
    pub const fn with_bg(mut self, bg: ViewColor) -> Self {
        self.bg = bg;
        self
    }

    /// Enables the bold modifier flag.
    #[must_use]
    pub const fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    /// Enables the dim modifier flag.
    #[must_use]
    pub const fn dim(mut self) -> Self {
        self.dim = true;
        self
    }

    /// Enables the underlined modifier flag.
    #[must_use]
    pub const fn underlined(mut self) -> Self {
        self.underlined = true;
        self
    }

    /// Enables the reversed-colors modifier flag.
    #[must_use]
    pub const fn reversed(mut self) -> Self {
        self.reversed = true;
        self
    }
}

/// Horizontal alignment hint for a [`ViewPanel`] or table column/cell.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
pub enum ViewAlign {
    /// Flush left (default).
    #[default]
    Left,
    /// Centered horizontally.
    Center,
    /// Flush right.
    Right,
}

/// Styled owned text fragment within a [`ViewRow`], [`ViewTableCell`], or [`ViewStatusLine`].
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub struct ViewSpan {
    /// UTF-8 label or text fragment.
    pub text: String,
    /// Plain style attributes applied by the host renderer.
    pub style: ViewStyle,
}

impl ViewSpan {
    /// Wraps an unstyled text string as a [`ViewSpan`].
    #[must_use]
    pub const fn plain(text: String) -> Self {
        Self {
            text,
            style: ViewStyle::plain(),
        }
    }

    /// Wraps an owned text string together with an explicit [`ViewStyle`].
    #[must_use]
    pub const fn styled(text: String, style: ViewStyle) -> Self {
        Self { text, style }
    }
}

/// Owned sequence of styled [`ViewSpan`]s rendered on one line.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub struct ViewRow {
    /// Ordered spans concatenated left-to-right on a single line.
    pub spans: Vec<ViewSpan>,
}

impl ViewRow {
    /// Constructs a single-span unstyled row.
    #[must_use]
    pub fn from_plain(text: String) -> Self {
        Self {
            spans: vec![ViewSpan::plain(text)],
        }
    }

    /// Constructs a row from owned styled spans.
    #[must_use]
    pub const fn from_spans(spans: Vec<ViewSpan>) -> Self {
        Self { spans }
    }

    /// Concatenates the plain text of every span on this row.
    #[must_use]
    pub fn plain_text(&self) -> String {
        let mut out = String::new();
        for span in &self.spans {
            out.push_str(&span.text);
        }
        out
    }
}

/// Owned label string with optional style (used for panel titles, tabs, and badges).
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub struct ViewLabel {
    /// Displayed label text.
    pub text: String,
    /// Style applied to the label.
    pub style: ViewStyle,
}

impl ViewLabel {
    /// Creates an unstyled owned label.
    #[must_use]
    pub const fn plain(text: String) -> Self {
        Self {
            text,
            style: ViewStyle::plain(),
        }
    }

    /// Creates a styled owned label.
    #[must_use]
    pub const fn styled(text: String, style: ViewStyle) -> Self {
        Self { text, style }
    }
}

/// Named screen zone rendered by the host layout engine without guessing splits.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
pub enum ViewZoneId {
    /// Top turn/phase/shock/seat header bar.
    HeaderBar,
    /// Left public table & ledger mirror pane.
    PublicLedger,
    /// Center action & order composer tabs.
    CenterTabs,
    /// Right seat diagnostics & private vault drawer.
    PrivateDrawer,
    /// Bottom status and key-hint line.
    StatusLine,
    /// Single full-viewport panel (used by title screen, Curtain, and too-small warning).
    #[default]
    SingleFull,
}

/// Layout split axis for a [`ViewZoneGroup`].
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
pub enum ViewAxis {
    /// Top-to-bottom stack.
    #[default]
    Vertical,
    /// Left-to-right split.
    Horizontal,
}

/// Constraint hint for a layout zone, band, or table column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ViewSizeHint {
    /// Fixed number of rows or columns.
    Fixed(u16),
    /// Minimum rows or columns, expanding to take remaining space.
    Min(u16),
    /// Percentage (`0..=100`) of the enclosing axis.
    Percentage(u16),
    /// Exact rational share `num / den`.
    Ratio {
        /// Numerator.
        num: u32,
        /// Denominator (`>= 1`).
        den: u32,
    },
}

/// One named zone together with its size hint inside a [`ViewZoneGroup`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ViewZoneSpec {
    /// Target zone identifier.
    pub zone: ViewZoneId,
    /// Size hint along the parent group's axis.
    pub hint: ViewSizeHint,
}

/// One horizontal or vertical band of named zones in a [`ScreenLayout`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ViewZoneGroup {
    /// Split axis across [`Self::zones`].
    pub axis: ViewAxis,
    /// Height hint (or width hint) of this band in the top-level vertical layout.
    pub hint: ViewSizeHint,
    /// Ordered child zones laid out along `axis`.
    pub zones: Vec<ViewZoneSpec>,
}

/// Responsive layout mode chosen from [`ViewportSize`].
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
pub enum ViewLayoutMode {
    /// Single full-area view (title screen or Curtain).
    #[default]
    SinglePanel,
    /// Four-zone desktop layout (`>= 100x30`).
    FourZoneWide,
    /// Stacked four-zone fallback layout (`>= 80x24`, smaller than `100x30`).
    StackedFallback,
    /// Terminal below `80x24` showing a single unclipped "terminal too small" view.
    TooSmall,
}

/// Complete host-agnostic layout specification calculated from [`ViewportSize`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub struct ScreenLayout {
    /// Viewport dimensions used to derive this layout.
    pub viewport: ViewportSize,
    /// Responsive layout mode selected for `viewport`.
    pub mode: ViewLayoutMode,
    /// Top-level vertical bands of named zones.
    pub bands: Vec<ViewZoneGroup>,
}

impl ScreenLayout {
    /// Builds a single full-viewport layout for title, Curtain, or fallback screens.
    #[must_use]
    pub fn single_panel(viewport: ViewportSize) -> Self {
        Self {
            viewport,
            mode: ViewLayoutMode::SinglePanel,
            bands: vec![ViewZoneGroup {
                axis: ViewAxis::Vertical,
                hint: ViewSizeHint::Min(1),
                zones: vec![ViewZoneSpec {
                    zone: ViewZoneId::SingleFull,
                    hint: ViewSizeHint::Min(1),
                }],
            }],
        }
    }

    /// Builds a single full-viewport layout with a two-row bottom [`ViewZoneId::StatusLine`]
    /// for screens such as the Privacy Curtain and neutral standby view.
    #[must_use]
    pub fn single_panel_with_status(viewport: ViewportSize) -> Self {
        Self {
            viewport,
            mode: ViewLayoutMode::SinglePanel,
            bands: vec![
                ViewZoneGroup {
                    axis: ViewAxis::Vertical,
                    hint: ViewSizeHint::Min(1),
                    zones: vec![ViewZoneSpec {
                        zone: ViewZoneId::SingleFull,
                        hint: ViewSizeHint::Min(1),
                    }],
                },
                ViewZoneGroup {
                    axis: ViewAxis::Vertical,
                    hint: ViewSizeHint::Fixed(2),
                    zones: vec![ViewZoneSpec {
                        zone: ViewZoneId::StatusLine,
                        hint: ViewSizeHint::Fixed(2),
                    }],
                },
            ],
        }
    }

    /// Computes the responsive Club Mode turn-screen layout from `viewport`:
    /// - `>= 100x30`: four-zone desktop layout (`HeaderBar`, `PublicLedger` / `CenterTabs` / `PrivateDrawer`, `StatusLine`);
    /// - `>= 80x24`: stacked four-zone layout where every zone row is drawn in full height or reachable via that zone's scroll (lines wider than the zone cut at the zone's width);
    /// - `< 80x24`: single `TooSmall` warning layout.
    #[must_use]
    pub fn layout_for_viewport(viewport: ViewportSize) -> Self {
        if viewport.width >= 100 && viewport.height >= 30 {
            Self {
                viewport,
                mode: ViewLayoutMode::FourZoneWide,
                bands: vec![
                    ViewZoneGroup {
                        axis: ViewAxis::Vertical,
                        hint: ViewSizeHint::Fixed(5),
                        zones: vec![ViewZoneSpec {
                            zone: ViewZoneId::HeaderBar,
                            hint: ViewSizeHint::Fixed(5),
                        }],
                    },
                    ViewZoneGroup {
                        axis: ViewAxis::Horizontal,
                        hint: ViewSizeHint::Min(20),
                        zones: vec![
                            ViewZoneSpec {
                                zone: ViewZoneId::PublicLedger,
                                hint: ViewSizeHint::Percentage(30),
                            },
                            ViewZoneSpec {
                                zone: ViewZoneId::CenterTabs,
                                hint: ViewSizeHint::Percentage(44),
                            },
                            ViewZoneSpec {
                                zone: ViewZoneId::PrivateDrawer,
                                hint: ViewSizeHint::Percentage(26),
                            },
                        ],
                    },
                    ViewZoneGroup {
                        axis: ViewAxis::Vertical,
                        hint: ViewSizeHint::Fixed(2),
                        zones: vec![ViewZoneSpec {
                            zone: ViewZoneId::StatusLine,
                            hint: ViewSizeHint::Fixed(2),
                        }],
                    },
                ],
            }
        } else if viewport.width >= 80 && viewport.height >= 24 {
            Self {
                viewport,
                mode: ViewLayoutMode::StackedFallback,
                bands: vec![
                    ViewZoneGroup {
                        axis: ViewAxis::Vertical,
                        hint: ViewSizeHint::Fixed(5),
                        zones: vec![ViewZoneSpec {
                            zone: ViewZoneId::HeaderBar,
                            hint: ViewSizeHint::Fixed(5),
                        }],
                    },
                    ViewZoneGroup {
                        axis: ViewAxis::Vertical,
                        hint: ViewSizeHint::Fixed(5),
                        zones: vec![ViewZoneSpec {
                            zone: ViewZoneId::PublicLedger,
                            hint: ViewSizeHint::Fixed(5),
                        }],
                    },
                    ViewZoneGroup {
                        axis: ViewAxis::Vertical,
                        hint: ViewSizeHint::Min(7),
                        zones: vec![ViewZoneSpec {
                            zone: ViewZoneId::CenterTabs,
                            hint: ViewSizeHint::Min(7),
                        }],
                    },
                    ViewZoneGroup {
                        axis: ViewAxis::Vertical,
                        hint: ViewSizeHint::Fixed(5),
                        zones: vec![ViewZoneSpec {
                            zone: ViewZoneId::PrivateDrawer,
                            hint: ViewSizeHint::Fixed(5),
                        }],
                    },
                    ViewZoneGroup {
                        axis: ViewAxis::Vertical,
                        hint: ViewSizeHint::Fixed(2),
                        zones: vec![ViewZoneSpec {
                            zone: ViewZoneId::StatusLine,
                            hint: ViewSizeHint::Fixed(2),
                        }],
                    },
                ],
            }
        } else {
            Self {
                viewport,
                mode: ViewLayoutMode::TooSmall,
                bands: vec![ViewZoneGroup {
                    axis: ViewAxis::Vertical,
                    hint: ViewSizeHint::Min(1),
                    zones: vec![ViewZoneSpec {
                        zone: ViewZoneId::SingleFull,
                        hint: ViewSizeHint::Min(1),
                    }],
                }],
            }
        }
    }
}

/// Scroll offset and item count carried for each scrollable [`ViewPanel`].
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
pub struct ViewScroll {
    /// Zero-based first visible item/row offset.
    pub offset: usize,
    /// Total number of scrollable items/rows in the panel.
    pub total: usize,
}

/// Column definition inside a [`ViewTable`].
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ViewTableColumn {
    /// Column header label.
    pub header: ViewLabel,
    /// Column width hint.
    pub width: ViewSizeHint,
    /// Default alignment of values in this column.
    pub alignment: ViewAlign,
}

/// Owned styled cell in a [`ViewTableRow`].
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub struct ViewTableCell {
    /// Styled spans within this cell.
    pub spans: Vec<ViewSpan>,
    /// Cell alignment.
    pub alignment: ViewAlign,
}

impl ViewTableCell {
    /// Creates a plain left-aligned cell.
    #[must_use]
    pub fn plain(text: String) -> Self {
        Self {
            spans: vec![ViewSpan::plain(text)],
            alignment: ViewAlign::Left,
        }
    }

    /// Creates a plain cell with explicit alignment.
    #[must_use]
    pub fn aligned(text: String, alignment: ViewAlign, style: ViewStyle) -> Self {
        Self {
            spans: vec![ViewSpan::styled(text, style)],
            alignment,
        }
    }

    /// Concatenates the plain text of every span inside this cell.
    #[must_use]
    pub fn plain_text(&self) -> String {
        let mut out = String::new();
        for span in &self.spans {
            out.push_str(&span.text);
        }
        out
    }
}

/// One row of [`ViewTableCell`]s inside a [`ViewTable`].
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub struct ViewTableRow {
    /// Ordered cells matching [`ViewTable::columns`].
    pub cells: Vec<ViewTableCell>,
    /// Row-level style applied by the host.
    pub style: ViewStyle,
}

impl ViewTableRow {
    /// Joins every cell's plain text with `" | "`.
    #[must_use]
    pub fn plain_text(&self) -> String {
        let mut out = String::new();
        for (idx, cell) in self.cells.iter().enumerate() {
            if idx > 0 {
                out.push_str(" | ");
            }
            out.push_str(&cell.plain_text());
        }
        out
    }
}

/// Structured tabular view model with typed columns, alignment, and rows.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub struct ViewTable {
    /// Ordered column headers, widths, and alignments.
    pub columns: Vec<ViewTableColumn>,
    /// Data rows.
    pub rows: Vec<ViewTableRow>,
    /// Optional 0-based selected row index.
    pub selected_row: Option<usize>,
}

impl ViewTable {
    /// Formats column headers and rows as plain text for inspection and tests.
    #[must_use]
    pub fn plain_text(&self) -> String {
        let mut out = String::new();
        if !self.columns.is_empty() {
            for (idx, col) in self.columns.iter().enumerate() {
                if idx > 0 {
                    out.push_str(" | ");
                }
                out.push_str(&col.header.text);
            }
        }
        for row in &self.rows {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&row.plain_text());
        }
        out
    }
}

/// Single tab entry inside a [`ViewTabBar`].
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub struct ViewTabItem {
    /// Tab title label.
    pub label: ViewLabel,
    /// Whether this tab can be selected (`false` for disabled future tabs like Diplomatic Desk).
    pub enabled: bool,
}

/// Tab strip metadata carried inside a tabbed [`ViewPanel`].
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub struct ViewTabBar {
    /// Ordered tabs left-to-right.
    pub tabs: Vec<ViewTabItem>,
    /// Zero-based index of the active tab.
    pub active_index: usize,
}

impl ViewTabBar {
    /// Concatenates tab labels with an active marker for plain-text inspection.
    #[must_use]
    pub fn plain_text(&self) -> String {
        let mut out = String::new();
        for (idx, tab) in self.tabs.iter().enumerate() {
            if idx > 0 {
                out.push_str(" | ");
            }
            if idx == self.active_index {
                out.push('[');
                out.push_str(&tab.label.text);
                out.push(']');
            } else {
                out.push_str(&tab.label.text);
            }
        }
        out
    }
}

/// Modal overlay layer rendered centered over the screen when present.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub struct ViewModal {
    /// Target modal width percentage (`1..=100`).
    pub width_pct: u16,
    /// Target modal height percentage (`1..=100`).
    pub height_pct: u16,
    /// Panel rendered inside the modal dialog.
    pub panel: ViewPanel,
}

/// Bottom status line with optional prominent refusal/alert row plus key-hint and mode spans.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub struct ViewStatusLine {
    /// Prominent top-row refusal/error banner rendered ahead of shortcuts so it stays visible at 80x24+.
    pub alert: Option<ViewRow>,
    /// Left-aligned spans.
    pub left: Vec<ViewSpan>,
    /// Right-aligned spans.
    pub right: Vec<ViewSpan>,
    /// Base style for the status bar.
    pub style: ViewStyle,
}

impl ViewStatusLine {
    /// Joins alert, left, and right spans as plain text.
    #[must_use]
    pub fn plain_text(&self) -> String {
        let mut out = String::new();
        if let Some(alert) = &self.alert {
            out.push_str(&alert.plain_text());
        }
        let mut bar = String::new();
        for span in &self.left {
            bar.push_str(&span.text);
        }
        if !self.right.is_empty() {
            if !bar.is_empty() {
                bar.push_str(" | ");
            }
            for span in &self.right {
                bar.push_str(&span.text);
            }
        }
        if !bar.is_empty() {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&bar);
        }
        out
    }
}

/// Panel bound to a named [`ViewZoneId`] inside [`SeatScreenView::zones`].
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub struct ViewZonePanel {
    /// Zone identifier matching a [`ViewZoneSpec`] inside [`ScreenLayout`].
    pub zone: ViewZoneId,
    /// Panel contents to render inside that zone.
    pub panel: ViewPanel,
}

/// Self-contained bordered or plain panel made purely of owned view-model data.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub struct ViewPanel {
    /// Optional header title rendered in the panel border.
    pub title: Option<ViewLabel>,
    /// Horizontal alignment of body rows within the panel.
    pub alignment: ViewAlign,
    /// Whether the host draws an outer box border around the panel.
    pub bordered: bool,
    /// Optional tab bar rendered at the top of the panel.
    pub tabs: Option<ViewTabBar>,
    /// Optional structured table rendered inside the panel.
    pub table: Option<ViewTable>,
    /// Ordered text rows rendered inside the panel.
    pub rows: Vec<ViewRow>,
    /// Optional 0-based selected row index highlighted by the host.
    pub selected_row: Option<usize>,
    /// Optional scroll position and total row count for scrollable panels.
    pub scroll: Option<ViewScroll>,
}

impl ViewPanel {
    /// Joins tabs, rows, and table text with `"\n"` for inspection and tests.
    #[must_use]
    pub fn plain_text(&self) -> String {
        let mut out = String::new();
        if let Some(tabs) = &self.tabs {
            let t = tabs.plain_text();
            if !t.is_empty() {
                out.push_str(&t);
            }
        }
        if let Some(table) = &self.table {
            let tbl = table.plain_text();
            if !tbl.is_empty() {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&tbl);
            }
        }
        for row in &self.rows {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&row.plain_text());
        }
        out
    }
}

/// Complete plain owned view model emitted by `league-seat-ui` for `league-tui` to draw.
///
/// Redacting `core::fmt::Debug` is implemented in `crate` root so rendered mandates, notes, and
/// staged orders stay redacted while keeping `view.rs` free of trait `impl` blocks.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub struct SeatScreenView {
    /// Layout specification mapping viewport size onto named zones.
    pub layout: ScreenLayout,
    /// Ordered zone panels referenced by `layout`.
    pub zones: Vec<ViewZonePanel>,
    /// Optional bottom status line rendered in [`ViewZoneId::StatusLine`].
    pub status_line: Option<ViewStatusLine>,
    /// Optional modal overlay panel rendered on top of the screen when present.
    pub modal: Option<ViewModal>,
}

impl SeatScreenView {
    /// Wraps a single primary [`ViewPanel`] in a full-viewport [`ScreenLayout::single_panel`].
    #[must_use]
    pub fn single_panel(main: ViewPanel) -> Self {
        Self::single_panel_at(main, ViewportSize::new(80, 24))
    }

    /// Wraps a single primary [`ViewPanel`] sized for an explicit [`ViewportSize`].
    #[must_use]
    pub fn single_panel_at(main: ViewPanel, viewport: ViewportSize) -> Self {
        Self {
            layout: ScreenLayout::single_panel(viewport),
            zones: vec![ViewZonePanel {
                zone: ViewZoneId::SingleFull,
                panel: main,
            }],
            status_line: None,
            modal: None,
        }
    }

    /// Looks up the rendered [`ViewPanel`] for `zone_id`, if present.
    #[must_use]
    pub fn zone(&self, zone_id: ViewZoneId) -> Option<&ViewPanel> {
        self.zones
            .iter()
            .find(|entry| entry.zone == zone_id)
            .map(|entry| &entry.panel)
    }

    /// Concatenates plain text across all zones, status line, and modal.
    #[must_use]
    pub fn plain_text(&self) -> String {
        let mut out = String::new();
        for entry in &self.zones {
            let panel = &entry.panel;
            if let Some(title) = &panel.title {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&title.text);
            }
            let body = panel.plain_text();
            if !body.is_empty() {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&body);
            }
        }
        if let Some(status) = &self.status_line {
            let st = status.plain_text();
            if !st.is_empty() {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&st);
            }
        }
        if let Some(modal) = &self.modal {
            if let Some(title) = &modal.panel.title {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&title.text);
            }
            let body = modal.panel.plain_text();
            if !body.is_empty() {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&body);
            }
        }
        out
    }
}

/// Terminal viewport dimensions in character cells passed by value from host to seat UI.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
pub struct ViewportSize {
    /// Terminal width in columns.
    pub width: u16,
    /// Terminal height in rows.
    pub height: u16,
}

impl ViewportSize {
    /// Creates a [`ViewportSize`] in columns and rows.
    #[must_use]
    pub const fn new(width: u16, height: u16) -> Self {
        Self { width, height }
    }
}

/// Plain key identifier mapped by the host from terminal events without exposing `crossterm`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
pub enum SeatKeyCode {
    /// Printable character key.
    Char(char),
    /// Enter / Return key.
    Enter,
    /// Escape key.
    Esc,
    /// Forward Tab key.
    Tab,
    /// Shift+Tab / BackTab key.
    BackTab,
    /// Backspace key.
    Backspace,
    /// Delete key.
    Delete,
    /// Up arrow key.
    Up,
    /// Down arrow key.
    Down,
    /// Left arrow key.
    Left,
    /// Right arrow key.
    Right,
    /// Home key.
    Home,
    /// End key.
    End,
    /// PageUp key.
    PageUp,
    /// PageDown key.
    PageDown,
    /// Function key `F(n)`.
    F(u8),
    /// Any terminal key not mapped onto a named navigation/character key.
    #[default]
    Unmapped,
}

/// Plain key modifier flags passed alongside [`SeatKeyCode`].
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
pub struct SeatKeyModifiers {
    /// Control modifier held.
    pub ctrl: bool,
    /// Alt / Option modifier held.
    pub alt: bool,
    /// Shift modifier held.
    pub shift: bool,
}

impl SeatKeyModifiers {
    /// Empty modifier set (`ctrl = false`, `alt = false`, `shift = false`).
    pub const NONE: Self = Self {
        ctrl: false,
        alt: false,
        shift: false,
    };

    /// Control modifier set (`ctrl = true`).
    pub const CTRL: Self = Self {
        ctrl: true,
        alt: false,
        shift: false,
    };
}

/// Single key press or key auto-repeat passed by value from the trusted host into `league-seat-ui`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
pub struct SeatKeyEvent {
    /// Pressed or repeated key code.
    pub code: SeatKeyCode,
    /// Active key modifier flags.
    pub modifiers: SeatKeyModifiers,
    /// `true` when emitted from terminal key auto-repeat rather than an initial key press.
    pub is_repeat: bool,
}

impl SeatKeyEvent {
    /// Constructs an initial key-press [`SeatKeyEvent`] (`is_repeat = false`).
    #[must_use]
    pub const fn new(code: SeatKeyCode, modifiers: SeatKeyModifiers) -> Self {
        Self {
            code,
            modifiers,
            is_repeat: false,
        }
    }

    /// Constructs a repeated [`SeatKeyEvent`] (`is_repeat = true`).
    #[must_use]
    pub const fn repeated(code: SeatKeyCode, modifiers: SeatKeyModifiers) -> Self {
        Self {
            code,
            modifiers,
            is_repeat: true,
        }
    }
}

/// Plain turn-clock state rendered in the turn header bar without holding wall-clock handles.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
pub struct TurnClockDisplay {
    /// `true` when the table runs under a timed turn budget; `false` when untimed.
    pub timed: bool,
    /// Whole seconds remaining for the active human seat's current sub-pass.
    pub remaining_secs: u16,
    /// Total whole-second budget allocated to this sub-pass for one human seat.
    pub pass_budget_secs: u16,
}

impl TurnClockDisplay {
    /// Creates an untimed [`TurnClockDisplay`].
    #[must_use]
    pub const fn untimed() -> Self {
        Self {
            timed: false,
            remaining_secs: 0,
            pass_budget_secs: 0,
        }
    }

    /// Creates a timed [`TurnClockDisplay`] with remaining and budget seconds.
    #[must_use]
    pub const fn timed(remaining_secs: u16, pass_budget_secs: u16) -> Self {
        Self {
            timed: true,
            remaining_secs,
            pass_budget_secs,
        }
    }
}

/// Stage of optional 4-digit table-local PIN gating on the Privacy Curtain.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
pub enum CurtainPinMode {
    /// No PIN prompt active (either PINs disabled or Curtain is waiting for `[Space]` + `[Enter]`).
    #[default]
    Inactive,
    /// Seat has not yet set its 4-digit PIN and is entering one for the first time.
    SetInitialPin,
    /// Seat already has a PIN and must enter the matching 4 digits before observation builds.
    VerifyPin,
}

/// Masked PIN prompt status passed to `curtain_view` without carrying PIN digits.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
pub struct CurtainPinPrompt {
    /// Current Curtain PIN stage (`Inactive`, `SetInitialPin`, or `VerifyPin`).
    pub mode: CurtainPinMode,
    /// Number of digits currently typed (`0..=4`); the digits themselves stay host-private.
    pub entered_count: u8,
    /// Number of failed verification attempts in this Curtain visit (`0..=2`).
    pub failed_attempts: u8,
    /// `true` after three wrong PIN tries reset the prompt back to the unarmed Curtain.
    pub locked_out_reset: bool,
}

/// Command-line override flags displayed cleanly in the Club Mode Lobby (`CLI > Lobby`).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
pub struct LobbyCliOverrides {
    /// Whole-turn timed budget forced by `--turn-clock <180..=300>`, if passed on CLI.
    pub turn_clock_secs: Option<u16>,
    /// `true` when `--untimed` was passed on CLI.
    pub untimed: bool,
    /// `true` when `--pins` was passed on CLI.
    pub pins_enabled: bool,
    /// `true` when `--no-color` or a non-empty `NO_COLOR` env var suppresses ANSI palette colors.
    #[serde(default)]
    pub no_color: bool,
    /// `true` when `--screen-reader` enables linear plain-text rendering without box borders.
    #[serde(default)]
    pub screen_reader: bool,
}

/// Public progress counter rendered during `TurnPhase::AwaitingMandateReveal` before finalization.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
pub struct AwaitingRevealProgress {
    /// Number of seats whose Turn-1 mandate reveal has been submitted (`0..=seat_count`).
    pub revealed_count: u8,
    /// Total seats at the table (`4..=6`).
    pub seat_count: u8,
}

/// Public status of interactive `SeasonReplayLog` export on End-of-Season Tab 4.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
pub enum ReplayExportState {
    /// Replay file has not been exported from End-of-Season Tab 4 yet.
    #[default]
    Ready,
    /// Replay log was verified, written to disk, and `.seal` was wiped.
    Exported,
    /// Replay write or verification failed; `.seal` was preserved on disk.
    Failed,
}

/// Plain post-finalize season summary rendered by `league-seat-ui` only in `TurnPhase::Completed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndOfSeasonData {
    /// Closed table identifier.
    pub table_id: TableId,
    /// Terminal turn number (`24..=32`).
    pub closed_turn: Turn,
    /// Final ranked scoreboard across all seats.
    pub scoreboard: FinalScoreboard,
    /// All seats' unsealed Turn-1 Private Charter Mandates (`0..seat_count`), populated strictly
    /// after `finalize_season()`.
    pub revealed_mandates: Vec<PrivateMandate>,
    /// Closing `SeatStatePredicates` (`at_close`) holding terminal `HoldingMark` splits.
    pub close_predicates: Vec<SeatStatePredicates>,
    /// Per-seat paper-test solvency and holding metrics (`SeatSeasonMetrics`).
    pub season_metrics: Vec<SeatSeasonMetrics>,
    /// Replay export status for Tab 4 (`Ready`, `Exported`, `Failed`).
    pub export_state: ReplayExportState,
    /// Ordered public sweep receipts (`T1..=closed_turn`) for mandate delivery/bounce turn provenance.
    #[serde(default)]
    pub public_receipts: Vec<PublicSweepReceipt>,
    /// Ordered per-turn public/post-close state predicate snapshots (`StatePredicateSection.turns`).
    #[serde(default)]
    pub predicate_turns: Vec<TurnPredicateRecord>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_and_curtain_pin_dtos_preserve_defaults_and_constructors() {
        let untimed = TurnClockDisplay::untimed();
        assert!(!untimed.timed);
        assert_eq!(untimed.remaining_secs, 0);
        assert_eq!(untimed.pass_budget_secs, 0);

        let timed = TurnClockDisplay::timed(39, 39);
        assert!(timed.timed);
        assert_eq!(timed.remaining_secs, 39);
        assert_eq!(timed.pass_budget_secs, 39);

        let prompt = CurtainPinPrompt::default();
        assert_eq!(prompt.mode, CurtainPinMode::Inactive);
        assert_eq!(prompt.entered_count, 0);
        assert_eq!(prompt.failed_attempts, 0);
        assert!(!prompt.locked_out_reset);

        let ov = LobbyCliOverrides::default();
        assert_eq!(ov.turn_clock_secs, None);
        assert!(!ov.untimed);
        assert!(!ov.pins_enabled);
        assert!(!ov.no_color);
        assert!(!ov.screen_reader);

        let prog = AwaitingRevealProgress {
            revealed_count: 1,
            seat_count: 4,
        };
        assert_eq!(prog.revealed_count, 1);
        assert_eq!(ReplayExportState::default(), ReplayExportState::Ready);
    }
}
