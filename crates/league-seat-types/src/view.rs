use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

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

    /// Enables or sets the bold modifier flag.
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

/// Horizontal alignment hint for a [`ViewPanel`].
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

/// Styled owned text fragment within a [`ViewRow`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
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

/// Owned label string with optional style (used for panel titles and badges).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
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

/// Self-contained bordered or plain panel made purely of owned view-model data.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub struct ViewPanel {
    /// Optional header title rendered in the panel border.
    pub title: Option<ViewLabel>,
    /// Horizontal alignment of body rows within the panel.
    pub alignment: ViewAlign,
    /// Whether the host draws an outer box border around the panel.
    pub bordered: bool,
    /// Ordered text rows rendered inside the panel.
    pub rows: Vec<ViewRow>,
    /// Optional 0-based selected row index highlighted by the host.
    pub selected_row: Option<usize>,
}

impl ViewPanel {
    /// Joins every row's plain text with `"\n"` for inspection and tests.
    #[must_use]
    pub fn plain_text(&self) -> String {
        let mut out = String::new();
        for (idx, row) in self.rows.iter().enumerate() {
            if idx > 0 {
                out.push('\n');
            }
            out.push_str(&row.plain_text());
        }
        out
    }
}

/// Complete plain owned view model emitted by `league-seat-ui` for `league-tui` to draw.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub struct SeatScreenView {
    /// Optional top status header panel.
    pub header: Option<ViewPanel>,
    /// Primary content panel rendered across the main area.
    pub main: ViewPanel,
    /// Optional secondary side panels (e.g., diagnostics, receipts, or commodity legend).
    pub side_panels: Vec<ViewPanel>,
    /// Optional modal overlay panel rendered on top of the screen when present.
    pub modal: Option<ViewPanel>,
}

impl SeatScreenView {
    /// Wraps a single primary [`ViewPanel`] with no header, side panels, or modal.
    #[must_use]
    pub const fn single_panel(main: ViewPanel) -> Self {
        Self {
            header: None,
            main,
            side_panels: Vec::new(),
            modal: None,
        }
    }

    /// Concatenates plain text across header, main, side panels, and modal.
    #[must_use]
    pub fn plain_text(&self) -> String {
        let mut out = String::new();
        let mut first = true;
        for panel in self
            .header
            .iter()
            .chain(core::iter::once(&self.main))
            .chain(self.side_panels.iter())
            .chain(self.modal.iter())
        {
            if let Some(title) = &panel.title {
                if !first {
                    out.push('\n');
                }
                first = false;
                out.push_str(&title.text);
            }
            let body = panel.plain_text();
            if !body.is_empty() {
                if !first {
                    out.push('\n');
                }
                first = false;
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

/// Single key press passed by value from the trusted host into `league-seat-ui`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
pub struct SeatKeyEvent {
    /// Pressed key code.
    pub code: SeatKeyCode,
    /// Active key modifier flags.
    pub modifiers: SeatKeyModifiers,
}

impl SeatKeyEvent {
    /// Constructs a [`SeatKeyEvent`] from its key code and modifiers.
    #[must_use]
    pub const fn new(code: SeatKeyCode, modifiers: SeatKeyModifiers) -> Self {
        Self { code, modifiers }
    }
}
