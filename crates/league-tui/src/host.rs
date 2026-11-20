//! Trusted Club Mode host session wrapper in the `league-tui` binary crate.
//!
//! Owns [`ClubHost`], two-pass hot-seat sequencing, `ClubBot` automation, strict host validation
//! of [`SeatIntent`]s, and generic [`ScreenLayout`] rendering via `ratatui` without putting any
//! seat view-model logic or forbidden engine symbols into `league-tui`.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use league_core::club::host::{ClubPauseCursor, ClubSessionCheckpoint};
use league_core::{
    AiPreviewRequest, ClubBot, ClubHost, EngineError, MarketAction, OrderReveal, PrivateMandate,
    SealedOrderBundle, SeasonReplayLog, Seat, SeatIdentity, SeatObservation, StructuredError,
    TableId, ToStructuredError, Turn, TurnPhase, preview_seat_bundle, preview_what_if,
    scrub_secret_text,
};
#[cfg(test)]
use league_core::{PublicSweepReceipt, RevealSalt, SealedEvalSeed, SealedMandateDeal, TableSpec};
use league_gen::{
    GenError, GeneratorConfig, RingTable, SeatCount, SeedPool, TablePurpose, TableSetupOptions,
    generate_ring_table_for_setup, is_reserved_test_seed, ring_table_to_table_spec, seed_pool,
};
use league_seat_types::{
    AwaitingRevealProgress, CommodityDisplayMap, CurtainPinMode, CurtainPinPrompt, EndOfSeasonData,
    LobbyCliOverrides, ReplayExportState, SeatIntent, SeatIntentKind, SeatKeyCode, SeatKeyEvent,
    SeatKeyModifiers, SeatScreenView, TurnClockDisplay, ViewAlign, ViewAxis, ViewColor,
    ViewLayoutMode, ViewPanel, ViewRow, ViewSizeHint, ViewSpan, ViewStyle, ViewTabBar, ViewTable,
    ViewZoneId, ViewportSize, WhatIfPreview,
};
use league_seat_ui::{
    EndOfSeasonTab, EndOfSeasonUiState, LobbyAction, LobbyState, PassKind, SeatUiState,
};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table};

/// Input dead time enforced whenever any Privacy Curtain opens or unlocks so queued or held
/// keystrokes during hand-off or immediately after `[Enter]` do not leak into the Curtain or seat.
pub(crate) const CURTAIN_LIFT_DEAD_TIME: Duration = Duration::from_millis(400);

/// Exit code `0`: clean completion (`SHIP_GATE.md` `:27`).
pub(crate) const EXIT_OK: u8 = 0;
/// Exit code `1`: user, CLI, configuration, or non-TTY error (`SHIP_GATE.md` `:27`).
pub(crate) const EXIT_USER_ERROR: u8 = 1;
/// Exit code `2`: runtime failure (save/load corruption, autosave/I/O error, wedged session, `SHIP_GATE.md` `:27`).
pub(crate) const EXIT_RUNTIME_ERROR: u8 = 2;
/// Exit code `3`: partial success (season finalized cleanly, but replay export or `.seal` cleanup failed, `SHIP_GATE.md` `:27`).
pub(crate) const EXIT_PARTIAL_SUCCESS: u8 = 3;

/// Environment variable selecting default diagnostic log verbosity for `league-tui`.
pub(crate) const LEAGUE_LOG_LEVEL_ENV: &str = "LEAGUE_LOG_LEVEL";

/// CLI error output encoding selected via `--error-format` (`SHIP_GATE.md` `:26`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ErrorFormat {
    /// Human-readable single-line redacted [`StructuredError`] text (`default`).
    #[default]
    Text,
    /// Compact machine-readable [`StructuredError`] JSON on `stderr`.
    Json,
}

impl ErrorFormat {
    /// Parses `"text"` or `"json"` (case-insensitive).
    pub fn parse(raw: &str) -> Result<Self, TuiError> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            _ => Err(TuiError::InvalidUsage(
                "--error-format must be 'text' or 'json'".to_owned(),
            )),
        }
    }

    /// Returns the lowercase wire label (`"text"` or `"json"`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Json => "json",
        }
    }

    /// Formats `err` on `stderr` in either scrubbed plain text or compact [`StructuredError`] JSON.
    #[must_use]
    pub(crate) fn format_line(self, err: &StructuredError) -> String {
        match self {
            Self::Text => format!("error: {}", err.format_text()),
            Self::Json => err.to_json_string().unwrap_or_else(|_| {
                format!(
                    "{{\"code\":\"{}\",\"message\":\"{}\",\"hint\":\"{}\",\"retryable\":{}}}",
                    err.code,
                    scrub_secret_text(&err.message).replace('"', "'"),
                    err.hint.replace('"', "'"),
                    err.retryable
                )
            }),
        }
    }
}

/// Diagnostic log verbosity level for `league-tui` (`SHIP_GATE.md` `:40`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    /// Suppress non-error diagnostics while still printing scrubbed fatal errors after restore.
    Silent = 0,
    /// Default level: lifecycle start/exit summary lines after terminal restore.
    #[default]
    Normal = 1,
    /// Adds turn/pass transitions and clock configuration diagnostics after terminal restore.
    Verbose = 2,
    /// Adds Curtain hand-off state diagnostics and unlocks redacted panic backtraces (`SHIP_GATE.md` `:28`).
    Debug = 3,
}

impl LogLevel {
    /// Parses `"silent"`, `"normal"`, `"verbose"`, or `"debug"` (case-insensitive).
    pub fn parse(raw: &str) -> Result<Self, TuiError> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "silent" => Ok(Self::Silent),
            "normal" => Ok(Self::Normal),
            "verbose" => Ok(Self::Verbose),
            "debug" => Ok(Self::Debug),
            _ => Err(TuiError::InvalidUsage(
                "--log-level must be 'silent', 'normal', 'verbose', or 'debug'".to_owned(),
            )),
        }
    }

    /// Returns the lowercase CLI label (`"silent"`, `"normal"`, `"verbose"`, or `"debug"`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Silent => "silent",
            Self::Normal => "normal",
            Self::Verbose => "verbose",
            Self::Debug => "debug",
        }
    }

    pub(crate) const fn as_u8(self) -> u8 {
        match self {
            Self::Silent => 0,
            Self::Normal => 1,
            Self::Verbose => 2,
            Self::Debug => 3,
        }
    }

    pub(crate) const fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Silent,
            2 => Self::Verbose,
            3 => Self::Debug,
            _ => Self::Normal,
        }
    }
}

impl fmt::Display for LogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

static PANIC_HOOK_LOG_LEVEL: AtomicU8 = AtomicU8::new(LogLevel::Normal.as_u8());

pub(crate) fn set_panic_hook_log_level(level: LogLevel) {
    PANIC_HOOK_LOG_LEVEL.store(level.as_u8(), Ordering::Release);
}

pub(crate) fn panic_hook_log_level() -> LogLevel {
    LogLevel::from_u8(PANIC_HOOK_LOG_LEVEL.load(Ordering::Acquire))
}

const SEALED_TUI_KEY_MARKERS: [&str; 10] = [
    "seat_pins",
    "session_seat_pins",
    "seat_notes",
    "session_seat_notes",
    "pending_bundles",
    "preimage_secret",
    "reveal_salt",
    "private_mandate",
    "mandate_reveal",
    "sealed_mandate_deal",
];

fn scrub_sealed_key_values(input: &str) -> String {
    let mut current = input.to_owned();
    for key in SEALED_TUI_KEY_MARKERS {
        let needle = format!("\"{key}\"");
        let mut out = String::with_capacity(current.len());
        let mut cursor = 0;
        while let Some(rel) = current[cursor..].find(&needle) {
            let start = cursor + rel;
            let after_key = start + needle.len();
            out.push_str(&current[cursor..after_key]);
            let rest = &current[after_key..];
            let trimmed_start = rest.find(':');
            if let Some(colon_rel) = trimmed_start
                && rest[..colon_rel].chars().all(|c| c == ' ')
            {
                let after_colon = after_key + colon_rel + 1;
                out.push_str(&current[after_key..after_colon]);
                let mut val_idx = after_colon;
                while let Some(ch) = current[val_idx..].chars().next()
                    && ch == ' '
                {
                    out.push(ch);
                    val_idx += ch.len_utf8();
                }
                if let Some(ch) = current[val_idx..].chars().next() {
                    if ch == '"' {
                        out.push_str("\"[REDACTED]\"");
                        val_idx += 1;
                        let mut escaped = false;
                        while let Some(c) = current[val_idx..].chars().next() {
                            val_idx += c.len_utf8();
                            if escaped {
                                escaped = false;
                            } else if c == '\\' {
                                escaped = true;
                            } else if c == '"' {
                                break;
                            }
                        }
                        cursor = val_idx;
                        continue;
                    } else if ch == '[' || ch == '{' {
                        let open = ch;
                        let close = if open == '[' { ']' } else { '}' };
                        let mut depth = 0_usize;
                        let mut in_str = false;
                        let mut escaped = false;
                        while let Some(c) = current[val_idx..].chars().next() {
                            val_idx += c.len_utf8();
                            if in_str {
                                if escaped {
                                    escaped = false;
                                } else if c == '\\' {
                                    escaped = true;
                                } else if c == '"' {
                                    in_str = false;
                                }
                            } else if c == '"' {
                                in_str = true;
                            } else if c == open {
                                depth += 1;
                            } else if c == close {
                                depth = depth.saturating_sub(1);
                                if depth == 0 {
                                    break;
                                }
                            }
                        }
                        out.push_str("\"[REDACTED]\"");
                        cursor = val_idx;
                        continue;
                    }
                }
            }
            cursor = after_key;
        }
        out.push_str(&current[cursor..]);
        current = out;
    }
    current
}

/// Scrubs credentials, sealed JSON payloads, and raw terminal control sequences so diagnostic
/// logs and panic lines remain safe across all [`LogLevel`]s.
pub(crate) fn scrub_tui_diagnostic_text(raw: &str) -> String {
    let mut scrubbed = scrub_secret_text(raw);
    scrubbed = scrub_sealed_key_values(&scrubbed);
    strip_terminal_controls(&scrubbed)
}

/// Canonical structured error type for `league-tui` (`SHIP_GATE.md` `:26`).
///
/// Maps cleanly onto the workspace unified `0/1/2/3` exit-code contract (`SHIP_GATE.md` `:27`) and
/// scrubs every human- or machine-readable message before display, JSON encoding, or [`fmt::Debug`].
#[derive(Clone, PartialEq, Eq)]
pub enum TuiError {
    /// Invalid CLI flag, conflicting option, or invalid `LEAGUE_LOG_LEVEL` (`exit 1`).
    InvalidUsage(String),
    /// Caller attempted to pass PIN digits on the CLI instead of entering them at the Curtain (`exit 1`).
    PinCliForbidden,
    /// Interactive session launched without an attached TTY on stdin/stdout (`exit 1`).
    TtyRequired,
    /// `--resume <PATH>` pointed to a missing or unreadable save target (`exit 1`).
    InvalidResumeTarget(String),
    /// Conflicting or non-empty `--save-dir` target would overwrite another season (`exit 1`).
    SaveDirConflict(StructuredError),
    /// Atomic mid-turn `.save.json` / `.seal` autosave failed and halted the session (`exit 2`).
    AutosaveFailed {
        save_path: PathBuf,
        cause: StructuredError,
    },
    /// Resuming `.save.json` / `.seal` failed due to corruption or seal validation mismatch (`exit 2`).
    SaveLoadFailed(StructuredError),
    /// Terminal backend or event stream I/O failure (`exit 2`).
    TerminalIo(String),
    /// Session wedged on pass-advance failure (`exit 2`).
    SessionHalted(StructuredError),
    /// Season reached `TurnPhase::Completed`, but replay export or `.seal` wipe failed (`exit 3`).
    PostSeasonExportFailed(StructuredError),
}

impl TuiError {
    /// Returns the unified process exit code (`1`, `2`, or `3`) for this [`TuiError`].
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::InvalidUsage(_)
            | Self::PinCliForbidden
            | Self::TtyRequired
            | Self::InvalidResumeTarget(_)
            | Self::SaveDirConflict(_) => EXIT_USER_ERROR,
            Self::AutosaveFailed { .. }
            | Self::SaveLoadFailed(_)
            | Self::TerminalIo(_)
            | Self::SessionHalted(_) => EXIT_RUNTIME_ERROR,
            Self::PostSeasonExportFailed(_) => EXIT_PARTIAL_SUCCESS,
        }
    }
}

impl ToStructuredError for TuiError {
    fn to_structured_error(&self) -> StructuredError {
        match self {
            Self::InvalidUsage(msg) => StructuredError::new(
                "INVALID_USAGE",
                msg,
                "Run `league-tui --help` for usage, precedence, and exit codes.",
                false,
            ),
            Self::PinCliForbidden => StructuredError::new(
                "PIN_CLI_FORBIDDEN",
                "PIN digits must never be passed on the command line or via env vars; pass --pins and enter each seat's 4-digit PIN at its first Curtain",
                "Remove `--pin` and pass `--pins` to set each human seat's PIN interactively at its Curtain.",
                false,
            ),
            Self::TtyRequired => StructuredError::new(
                "TTY_REQUIRED",
                "interactive terminal (TTY) required to launch Club Mode",
                "Re-run in an interactive terminal (or `docker run --rm -it <image>`), or pass `--help` / `--version`.",
                false,
            ),
            Self::InvalidResumeTarget(msg) => StructuredError::new(
                "INVALID_RESUME_TARGET",
                msg,
                "Pass an existing `club-<id>.save.json` file or directory containing one to `--resume <PATH>`.",
                false,
            ),
            Self::SaveDirConflict(err) => StructuredError {
                code: err.code,
                message: scrub_secret_text(&err.message),
                hint: err.hint,
                cause: err.cause.as_deref().map(scrub_secret_text),
                retryable: false,
            },
            Self::AutosaveFailed { save_path, cause } => StructuredError::new(
                "TUI_AUTOSAVE_FAILED",
                format!(
                    "Autosave failed at {}: {}",
                    save_path.display(),
                    cause.message
                ),
                "Verify `--save-dir` exists, is writable, and has free disk space before resuming.",
                true,
            )
            .with_cause(cause.code),
            Self::SaveLoadFailed(err) => StructuredError {
                code: err.code,
                message: scrub_secret_text(&err.message),
                hint: err.hint,
                cause: err.cause.as_deref().map(scrub_secret_text),
                retryable: err.retryable,
            },
            Self::TerminalIo(msg) => StructuredError::new(
                "TUI_TERMINAL_IO",
                msg,
                "Verify interactive terminal state and stream health before re-running.",
                true,
            ),
            Self::SessionHalted(err) => StructuredError {
                code: err.code,
                message: scrub_secret_text(&err.message),
                hint: err.hint,
                cause: err.cause.as_deref().map(scrub_secret_text),
                retryable: err.retryable,
            },
            Self::PostSeasonExportFailed(err) => {
                if err.code == "TUI_REPLAY_EXPORT_FAILED" {
                    StructuredError {
                        code: err.code,
                        message: scrub_secret_text(&err.message),
                        hint: err.hint,
                        cause: err.cause.as_deref().map(scrub_secret_text),
                        retryable: true,
                    }
                } else {
                    StructuredError::new(
                        "TUI_REPLAY_EXPORT_FAILED",
                        format!("Season finalized cleanly, but replay export failed: {}", err.message),
                        "Check directory permissions/path; `.seal` was kept so replay export can be retried.",
                        true,
                    )
                    .with_cause(err.code)
                }
            }
        }
    }
}

impl fmt::Display for TuiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_structured_error())
    }
}

impl fmt::Debug for TuiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("TuiError")
            .field(&self.to_structured_error())
            .finish()
    }
}

impl std::error::Error for TuiError {}

/// Diagnostic logger that suppresses alternate-screen writes while a TUI session is drawn and
/// redacts secrets across all four [`LogLevel`]s (`silent`, `normal`, `verbose`, `debug`).
pub(crate) struct TuiDiagnosticSink {
    level: LogLevel,
    alternate_screen_active: bool,
    buffered_lines: Vec<String>,
    log_file: Option<PathBuf>,
}

impl TuiDiagnosticSink {
    pub(crate) fn new(level: LogLevel, log_file: Option<PathBuf>) -> Self {
        Self {
            level,
            alternate_screen_active: false,
            buffered_lines: Vec::new(),
            log_file,
        }
    }

    pub(crate) fn set_alternate_screen_active(&mut self, active: bool) {
        self.alternate_screen_active = active;
    }

    pub(crate) fn emit(&mut self, msg_level: LogLevel, raw_msg: &str) {
        if self.level == LogLevel::Silent || self.level < msg_level {
            return;
        }
        let scrubbed = scrub_tui_diagnostic_text(raw_msg);
        let line = format!("[{}] {scrubbed}", msg_level.as_str());
        if let Some(path) = &self.log_file
            && let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path)
        {
            let _ = writeln!(file, "{line}");
        }
        self.buffered_lines.push(line);
    }

    pub(crate) fn flush_after_restore(&mut self, err_out: &mut dyn Write) {
        if self.alternate_screen_active {
            return;
        }
        for line in self.buffered_lines.drain(..) {
            let _ = writeln!(err_out, "{line}");
        }
    }
}

#[cfg(test)]
pub(crate) fn mandate_short_marker(kind: league_core::MandateKind) -> String {
    match kind {
        league_core::MandateKind::MinProjectEquity { target } => {
            format!("MinProjectEquity >={target}")
        }
        league_core::MandateKind::MinRefinedVariety {
            distinct_goods,
            min_each,
        } => format!("MinRefinedVar {distinct_goods}x{min_each}"),
        league_core::MandateKind::MaintainCleanCredit { min_tier } => {
            format!("CleanCredit >= {min_tier:?}")
        }
        league_core::MandateKind::MinActivePartners { min_partners } => {
            format!("ActivePartners >= {min_partners}")
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
struct CurtainPinEntry {
    mode: CurtainPinMode,
    digits: String,
    wrong_tries: u8,
}

impl std::fmt::Debug for CurtainPinEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CurtainPinEntry")
            .field("mode", &self.mode)
            .field("entered_count", &self.digits.len())
            .field("digits", &"<redacted>")
            .field("wrong_tries", &self.wrong_tries)
            .finish()
    }
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

fn push_wrapped_plain_rows(rows: &mut Vec<ViewRow>, label: &str, value: &str, max_width: usize) {
    let full = format!("{label}: {value}");
    let chars: Vec<char> = full.chars().collect();
    if chars.is_empty() {
        return;
    }
    let step = max_width.max(16);
    for (idx, chunk) in chars.chunks(step).enumerate() {
        let line: String = chunk.iter().collect();
        if idx == 0 {
            rows.push(ViewRow::from_plain(line));
        } else {
            rows.push(ViewRow::from_plain(format!("  {line}")));
        }
    }
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

fn extract_revealed_mandates(
    log: &SeasonReplayLog,
    seat_count: usize,
) -> Result<Vec<PrivateMandate>, StructuredError> {
    if log.mandate_reveals.len() != seat_count {
        return Err(host_error(
            "MISSING_MANDATE_REVEAL",
            &format!(
                "expected {seat_count} revealed mandates in finalized log, got {}",
                log.mandate_reveals.len()
            ),
            None,
        ));
    }
    let mut out = Vec::with_capacity(seat_count);
    for (idx, item) in log.mandate_reveals.iter().enumerate() {
        let rev = item.as_ref().ok_or_else(|| {
            host_error(
                "MISSING_MANDATE_REVEAL",
                &format!("seat {idx} mandate reveal missing after finalize"),
                None,
            )
        })?;
        out.push(rev.mandate);
    }
    Ok(out)
}

fn write_atomic_file_bytes(target: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("replay.json");
    let tmp_path = parent.join(format!("{file_name}.tmp.{}", std::process::id()));
    let write_res = (|| -> Result<(), String> {
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp_path)
            .map_err(|err| format!("failed to open {}: {err}", tmp_path.display()))?;
        f.write_all(bytes)
            .map_err(|err| format!("failed to write {}: {err}", tmp_path.display()))?;
        f.flush()
            .map_err(|err| format!("failed to flush {}: {err}", tmp_path.display()))?;
        f.sync_all()
            .map_err(|err| format!("failed to sync {}: {err}", tmp_path.display()))?;
        drop(f);
        if let Err(rename_err) = std::fs::rename(&tmp_path, target) {
            return Err(format!(
                "failed to replace {}: {rename_err}",
                target.display()
            ));
        }
        Ok(())
    })();
    if write_res.is_err() {
        let _ = std::fs::remove_file(&tmp_path);
    }
    write_res
}

#[derive(Clone, PartialEq, Eq)]
struct EndOfSeasonSessionState {
    data: EndOfSeasonData,
    ui_state: EndOfSeasonUiState,
    replay_log: SeasonReplayLog,
    export_error: Option<StructuredError>,
}

/// Host session bridging a [`ClubHost`] table to seat-isolated `league-seat-ui` views and
/// executing sequential hot-seat passes (`SubPassA -> begin_cancel_lock -> SubPassB -> Sweep`).
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
    autosave_halted_info: Option<(PathBuf, StructuredError)>,
    halted_tui_error: Option<TuiError>,
    autosave_paths: Option<(PathBuf, PathBuf)>,
    end_of_season: Option<EndOfSeasonSessionState>,
    post_season_error: Option<StructuredError>,
    turn_clock_budget_secs: Option<u16>,
    clock_explicitly_untimed: bool,
    active_pass_started_at: Option<Duration>,
    now: Duration,
    curtain_dead_until: Option<Duration>,
    pending_screen_clear: bool,
    pending_input_drain: bool,
    #[cfg(test)]
    injected_raw_what_if: Option<WhatIfPreview>,
    #[cfg(test)]
    injected_bot_action: Option<MarketAction>,
    #[cfg(test)]
    observe_seat_calls: std::cell::Cell<usize>,
}

impl std::fmt::Debug for TuiHostSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TuiHostSession")
            .field("host", &self.host)
            .field("pass", &self.pass)
            .field("pass_offset", &self.pass_offset)
            .field("curtain", &self.curtain)
            .field("pins_enabled", &self.pins_enabled)
            .field("quit_requested", &self.quit_requested)
            .field("is_wedged", &self.is_wedged())
            .field("has_end_of_season", &self.end_of_season.is_some())
            .field("turn_clock_budget_secs", &self.turn_clock_budget_secs)
            .field("clock_explicitly_untimed", &self.clock_explicitly_untimed)
            .field("seat_pins", &"<redacted>")
            .field("seat_notes", &"<redacted>")
            .field("pending_bundles", &"<redacted>")
            .field("seat_states", &"<redacted>")
            .finish()
    }
}

impl TuiHostSession {
    fn from_host(
        host: ClubHost,
        commodities: Option<CommodityDisplayMap>,
        resuming: bool,
    ) -> Result<Self, StructuredError> {
        Self::from_host_with_extra_bot_action(
            host,
            commodities,
            resuming,
            #[cfg(test)]
            None,
        )
    }

    fn from_host_with_extra_bot_action(
        host: ClubHost,
        commodities: Option<CommodityDisplayMap>,
        resuming: bool,
        #[cfg(test)] extra_bot_action: Option<MarketAction>,
    ) -> Result<Self, StructuredError> {
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
            autosave_halted_info: None,
            halted_tui_error: None,
            autosave_paths: None,
            end_of_season: None,
            post_season_error: None,
            turn_clock_budget_secs: ckpt.turn_clock_secs,
            clock_explicitly_untimed: ckpt.untimed,
            active_pass_started_at: None,
            now: Duration::ZERO,
            curtain_dead_until: None,
            pending_screen_clear: false,
            pending_input_drain: false,
            #[cfg(test)]
            injected_raw_what_if: None,
            #[cfg(test)]
            injected_bot_action: extra_bot_action,
            #[cfg(test)]
            observe_seat_calls: std::cell::Cell::new(0),
        };
        if session.host.phase() == TurnPhase::Completed {
            let (scoreboard, replay_log, section, season_metrics) =
                ClubSessionCheckpoint::load_completed_season_artifacts(&session.host)
                    .map_err(|err| scrub_engine_error(&err))?;
            let revealed_mandates = extract_revealed_mandates(&replay_log, n)?;
            session.end_of_season = Some(EndOfSeasonSessionState {
                data: EndOfSeasonData {
                    table_id: session.host.table_id(),
                    closed_turn: Turn(scoreboard.closed_turn),
                    scoreboard,
                    revealed_mandates,
                    close_predicates: section.at_close,
                    season_metrics,
                    export_state: ReplayExportState::Ready,
                },
                ui_state: EndOfSeasonUiState::new(),
                replay_log,
                export_error: None,
            });
        }
        if let Some(cursor) = ckpt.pause_cursor {
            session.pass = if cursor.sub_pass_b {
                PassKind::SubPassB
            } else {
                PassKind::SubPassA
            };
            session.pass_offset = cursor.pass_offset;
            if session.host.phase() == TurnPhase::TalkAndMarket
                && session.human_seats.iter().any(|&h| h)
            {
                if let Some(s) = cursor.curtain_next_seat
                    && usize::from(s) < n
                    && session.human_seats[usize::from(s)]
                    && session.seat_at_offset(session.pass_offset) == Some(Seat(s))
                {
                    session.open_curtain_for_seat(Seat(s));
                } else {
                    session.advance_after_human_lock()?;
                }
            }
        } else if resuming {
            if session.host.phase() == TurnPhase::TalkAndMarket
                && session.human_seats.iter().any(|&h| h)
            {
                session.advance_after_human_lock()?;
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

    fn open_curtain_for_seat(&mut self, seat: Seat) {
        self.curtain = Some(CurtainState::fresh(seat));
        self.active_pass_started_at = None;
        self.curtain_dead_until = Some(self.now.saturating_add(CURTAIN_LIFT_DEAD_TIME));
        self.pending_input_drain = true;
        self.pending_screen_clear = true;
    }

    fn record_halt_error(&mut self, err: StructuredError) {
        self.curtain = None;
        self.curtain_dead_until = None;
        self.advance_halted_error = Some(err);
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

    pub(crate) fn pass_budget_secs(&self) -> Option<u16> {
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
        self.clock_explicitly_untimed = budget_secs.is_none();
        if budget_secs.is_some()
            && self.active_human_seat().is_some()
            && self.active_pass_started_at.is_none()
        {
            self.active_pass_started_at = Some(self.now);
        }
        self.refresh_active_clock_display();
    }

    /// Returns whether the session already carries an explicit turn-clock configuration from
    /// `.seal` or caller options.
    pub(crate) const fn has_saved_clock_setting(&self) -> bool {
        self.turn_clock_budget_secs.is_some() || self.clock_explicitly_untimed
    }

    /// Enables optional 4-digit table-local seat PINs at every Curtain; if an active human seat has
    /// no Curtain open yet, opens the Curtain behind dead-time & drain protection.
    pub(crate) fn enable_seat_pins(&mut self) {
        self.pins_enabled = true;
        if self.curtain.is_none()
            && let Some(active) = self.active_human_seat()
        {
            self.open_curtain_for_seat(active);
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
        self.pending_screen_clear = true;
        self.pass_offset += 1;
        if let Err(err) = self.advance_after_human_lock() {
            self.record_halt_error(err);
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
                turn_clock_secs: self.turn_clock_budget_secs,
                untimed: self.clock_explicitly_untimed,
            },
        )
        .map_err(|err| scrub_engine_error(&err))
    }

    fn maybe_autosave(&mut self) -> Result<(), StructuredError> {
        if self.is_wedged() {
            return Ok(());
        }
        if let Some((save_path, seal_path)) = self.autosave_paths.clone()
            && let Err(err) = self.write_save_and_seal(&save_path, &seal_path)
        {
            let tui_err = TuiError::AutosaveFailed {
                save_path: save_path.clone(),
                cause: err,
            };
            let structured = tui_err.to_structured_error();
            self.autosave_halted_info = Some((save_path, structured.clone()));
            self.halted_tui_error = Some(tui_err);
            self.record_halt_error(structured.clone());
            return Err(structured);
        }
        Ok(())
    }

    /// Returns the CLI-level [`TuiError`] corresponding to a halted autosave, wedged pass advance,
    /// or post-season export partial failure (`None` on clean exit).
    #[must_use]
    pub fn terminal_tui_error(&self) -> Option<TuiError> {
        if let Some(tui_err) = &self.halted_tui_error {
            return Some(tui_err.clone());
        }
        if let Some(err) = &self.advance_halted_error {
            return Some(TuiError::SessionHalted(err.clone()));
        }
        if self.has_partial_post_season_failure() {
            if let Some(eos) = &self.end_of_season
                && let Some(err) = &eos.export_error
            {
                return Some(TuiError::PostSeasonExportFailed(err.clone()));
            }
            if let Some(err) = &self.post_season_error {
                return Some(TuiError::PostSeasonExportFailed(err.clone()));
            }
        }
        None
    }

    /// Public non-secret metadata snapshot formatted for diagnostic logs.
    #[must_use]
    pub fn session_metadata_summary(&self) -> String {
        format!(
            "table=#{} turn={} phase={:?} pass={:?} curtain={}",
            self.host.table_id().0,
            self.host.turn(),
            self.host.phase(),
            self.pass,
            self.curtain.is_some()
        )
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
        Self::from_host(host, None, false)
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
        Self::from_host(host, Some(cmap.to_display_map()), false)
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
        Self::from_host(host, commodities, false)
    }

    #[cfg(test)]
    pub(crate) fn from_explicit_mandates_for_test(
        ring: &RingTable,
        table_id: TableId,
        human_seats: &[u8],
        targets: [league_core::Units; 4],
        bonuses: [league_core::Units; 4],
        bot_tie_seed: u64,
    ) -> Result<Self, StructuredError> {
        use league_core::{
            GradedMandateCard, MandateKind, RevealSalt, SealedEvalSeed,
            deal_graded_mandates_with_entropy,
        };
        use league_gen::ring_table_to_table_spec_with_explicit_entropy;

        let salts: Vec<RevealSalt> = (0..4).map(|i| RevealSalt([0x31_u8 + i; 32])).collect();
        let eval_seed =
            SealedEvalSeed::try_from_bytes([0x63; 32]).map_err(|err| scrub_engine_error(&err))?;
        let (mut spec, cmap, _deal, eval_seed) = ring_table_to_table_spec_with_explicit_entropy(
            ring,
            table_id,
            human_seats,
            [0x44; 32],
            &salts,
            eval_seed,
        )
        .map_err(|err| err.to_structured_error())?;
        spec.mandate_deck = (0..4)
            .map(|i| GradedMandateCard {
                card_id: u16::try_from(i + 1).unwrap_or(1),
                kind: MandateKind::MinProjectEquity { target: targets[i] },
                difficulty_completion_bps: 5_000,
                bonus_equity: bonuses[i],
            })
            .collect();
        let (commits, deal) =
            deal_graded_mandates_with_entropy(table_id, 4, [0x55; 32], &salts, &spec.mandate_deck)
                .map_err(|err| scrub_engine_error(&err))?;
        spec.dealt_mandate_commits = commits;
        Self::from_explicit_setup(
            spec,
            ring.seed,
            Some(cmap.to_display_map()),
            deal,
            eval_seed,
            bot_tie_seed,
        )
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

    /// Consumes a pending alternate-screen clear request emitted on Curtain open or lift.
    pub(crate) fn take_screen_clear(&mut self) -> bool {
        std::mem::take(&mut self.pending_screen_clear)
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
    #[cfg(test)]
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

    #[cfg(test)]
    pub(crate) fn turn_reveal_salt_hex(&self, seat: Seat) -> Result<String, EngineError> {
        let salt = ClubSessionCheckpoint::turn_reveal_salt(&self.host, seat)?;
        Ok(salt.0.iter().map(|b| format!("{b:02x}")).collect())
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
    /// paused pass/Curtain cursor, uncommitted bundles, private notes, and PIN state behind a
    /// Privacy Curtain whenever a human seat would otherwise be active.
    pub fn read_save_and_seal(
        save_path: &Path,
        seal_path: &Path,
        commodities: Option<CommodityDisplayMap>,
    ) -> Result<Self, StructuredError> {
        let host = ClubHost::read_save_and_seal(save_path, seal_path)
            .map_err(|err| scrub_engine_error(&err))?;
        Self::from_host(host, commodities, true)
    }

    /// Opens the Privacy Curtain for the currently active human seat using `open_curtain_for_seat`.
    pub(crate) fn open_initial_curtain_for_active_human(&mut self) {
        if self.curtain.is_none()
            && let Some(active) = self.active_human_seat()
        {
            self.open_curtain_for_seat(active);
        }
    }

    /// Returns `true` when post-season finalization succeeded while replay export or `.seal` cleanup
    /// failed (signaling exit code `3` partial success in CLI).
    #[must_use]
    pub fn has_partial_post_season_failure(&self) -> bool {
        self.host.phase() == TurnPhase::Completed
            && self
                .end_of_season
                .as_ref()
                .is_some_and(|eos| eos.export_error.is_some())
    }

    fn resolve_replay_and_seal_paths(&self) -> (PathBuf, PathBuf) {
        let tid = self.host.table_id().0;
        let default_name = format!("club-{tid}.replay.json");
        if let Some((save_path, seal_path)) = &self.autosave_paths {
            let parent = save_path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
            return (parent.join(default_name), seal_path.clone());
        }
        (
            PathBuf::from(default_name),
            PathBuf::from(format!("club-{tid}.seal")),
        )
    }

    /// Wipes the private `.seal` sidecar cleanly once the season finishes and replay log is
    /// exported, refusing wedged or in-progress sessions.
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
        if self.host.phase() != TurnPhase::Completed {
            return Err(host_error(
                "SEASON_NOT_FINISHED",
                "refused wiping .seal before season reaches Completed phase",
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

    #[cfg(test)]
    #[must_use]
    pub(crate) fn public_spec(&self) -> league_core::PublicTableSpec {
        self.host.public_spec()
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) const fn pins_enabled(&self) -> bool {
        self.pins_enabled
    }

    /// Records a scrubbed host-level error banner for the currently active seat.
    #[cfg(test)]
    pub fn record_scrubbed_host_error(&mut self, code: &'static str, raw_msg: &str) {
        if let Some(active) = self.active_human_seat() {
            self.last_errors[usize::from(active.0)] = Some(host_error(code, raw_msg, None));
        }
    }

    /// Builds the plain owned [`SeatScreenView`] for either the Curtain hand-off, the active
    /// seat's [`SeatObservation`], post-season reveal/summary screens, or a strictly public
    /// standby view when no human seat is active.
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
            if !self.quit_requested && !self.is_wedged() {
                if self.host.phase() == TurnPhase::AwaitingMandateReveal {
                    let mut view = league_seat_ui::render_awaiting_reveal_screen(
                        self.host.table_id(),
                        Turn(self.host.turn()),
                        AwaitingRevealProgress {
                            revealed_count: ClubSessionCheckpoint::revealed_mandate_count(
                                &self.host,
                            ),
                            seat_count: self.host.seat_count(),
                        },
                        viewport,
                    );
                    if let Some(err) = &self.post_season_error
                        && let Some(status) = &mut view.status_line
                    {
                        let banner =
                            scrub_secret_text(&format!("REFUSED [{}]: {}", err.code, err.message));
                        status.alert = Some(ViewRow::from_spans(vec![ViewSpan::styled(
                            banner,
                            ViewStyle::plain().with_fg(ViewColor::Red).bold(),
                        )]));
                    }
                    return Ok(view);
                }
                if self.host.phase() == TurnPhase::Completed
                    && let Some(eos) = &self.end_of_season
                {
                    let mut view = league_seat_ui::render_end_of_season_screen(
                        &eos.data,
                        &eos.ui_state,
                        self.commodities.as_ref(),
                        viewport,
                    );
                    if eos.ui_state.active_tab() == EndOfSeasonTab::ReplayExport
                        && let Some(panel) = view.zones.first_mut().map(|z| &mut z.panel)
                    {
                        let (replay_path, _) = self.resolve_replay_and_seal_paths();
                        push_wrapped_plain_rows(
                            &mut panel.rows,
                            "Replay path",
                            &scrub_secret_text(&replay_path.display().to_string()),
                            74,
                        );
                        if let Some(err) = &eos.export_error {
                            panel
                                .rows
                                .push(ViewRow::from_plain(format!("Export error [{}]", err.code)));
                            push_wrapped_plain_rows(
                                &mut panel.rows,
                                "Reason",
                                &scrub_secret_text(&err.message),
                                74,
                            );
                        }
                    }
                    if let Some(err) = &eos.export_error
                        && let Some(status) = &mut view.status_line
                    {
                        let banner =
                            scrub_secret_text(&format!("REFUSED [{}]: {}", err.code, err.message));
                        status.alert = Some(ViewRow::from_spans(vec![ViewSpan::styled(
                            banner,
                            ViewStyle::plain().with_fg(ViewColor::Red).bold(),
                        )]));
                    }
                    return Ok(view);
                }
            }
            let mut view = league_seat_ui::neutral_session_view(
                self.host.table_id(),
                Turn(self.host.turn()),
                self.host.phase(),
                self.pass,
                self.quit_requested,
                self.is_wedged(),
                viewport,
            );
            if let Some((save_path, save_err)) = &self.autosave_halted_info
                && let Some(panel) = view.zones.first_mut().map(|z| &mut z.panel)
            {
                let mut rows = Vec::new();
                if let Some(header_row) = panel.rows.first().cloned() {
                    rows.push(header_row);
                }
                rows.push(ViewRow::from_plain(format!(
                    "Autosave failed [{}] — session halted; press q to quit.",
                    save_err.code
                )));
                push_wrapped_plain_rows(
                    &mut rows,
                    "Save path",
                    &scrub_secret_text(&save_path.display().to_string()),
                    74,
                );
                push_wrapped_plain_rows(
                    &mut rows,
                    "Reason",
                    &scrub_secret_text(&save_err.message),
                    74,
                );
                panel.rows = rows;
            }
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
        if self.is_wedged() {
            self.curtain_dead_until = None;
        } else if let Some(until) = self.curtain_dead_until {
            if self.now < until {
                return Ok(());
            }
            self.curtain_dead_until = None;
        }

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
                            self.active_pass_started_at =
                                Some(self.now.saturating_add(CURTAIN_LIFT_DEAD_TIME));
                            self.curtain_dead_until =
                                Some(self.now.saturating_add(CURTAIN_LIFT_DEAD_TIME));
                            self.pending_input_drain = true;
                            self.pending_screen_clear = true;
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
                        self.active_pass_started_at =
                            Some(self.now.saturating_add(CURTAIN_LIFT_DEAD_TIME));
                        self.curtain_dead_until =
                            Some(self.now.saturating_add(CURTAIN_LIFT_DEAD_TIME));
                        self.pending_input_drain = true;
                        self.pending_screen_clear = true;
                        self.restore_seat_ui_from_host(next);
                    }
                }
                _ => {
                    curtain.space_armed = false;
                }
            }
            return Ok(());
        }

        let Some(active) = self.active_human_seat() else {
            if key.is_repeat
                && !(self.host.phase() == TurnPhase::Completed
                    && !self.is_wedged()
                    && !self.quit_requested)
            {
                return Ok(());
            }
            if !self.is_wedged() && !self.quit_requested {
                if self.host.phase() == TurnPhase::AwaitingMandateReveal {
                    self.post_season_error = None;
                    if let Some(intent) = league_seat_ui::handle_awaiting_reveal_key(key)
                        && let Err(err) = self.apply_post_season_intent(intent)
                    {
                        self.post_season_error = Some(err.clone());
                        return Err(err);
                    }
                    return Ok(());
                }
                if self.host.phase() == TurnPhase::Completed
                    && let Some((data, ui_state)) = self
                        .end_of_season
                        .as_ref()
                        .map(|eos| (eos.data.clone(), eos.ui_state.clone()))
                {
                    let step = league_seat_ui::handle_end_of_season_key(
                        &data,
                        ui_state,
                        self.commodities.as_ref(),
                        viewport,
                        key,
                    );
                    if let Some(eos) = &mut self.end_of_season {
                        eos.ui_state = step.state;
                    }
                    if let Some(intent) = step.intent {
                        self.apply_post_season_intent(intent)?;
                    }
                    return Ok(());
                }
            }
            if !key.is_repeat
                && (matches!(key.code, SeatKeyCode::Char('q') | SeatKeyCode::Esc)
                    || (key.code == SeatKeyCode::Char('c') && key.modifiers.ctrl))
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
            | SeatIntentKind::SubmitMandateReveal
            | SeatIntentKind::ExportReplay
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
            SeatIntentKind::SubmitMandateReveal | SeatIntentKind::ExportReplay => Err(host_error(
                "INVALID_PHASE",
                &format!(
                    "refused {:?}: only valid after season closes in AwaitingMandateReveal or Completed",
                    intent.kind
                ),
                Some("Wait until the season closes to reveal mandates or export the replay."),
            )),
            SeatIntentKind::LockAndHandOff => {
                self.seat_notes[idx] = self.seat_states[idx].notes_draft().to_owned();
                if self.pass == PassKind::SubPassB {
                    self.sub_pass_b_committed[idx] = true;
                }
                self.seat_states[idx].clear_for_hand_off();
                self.last_errors[idx] = None;
                self.active_pass_started_at = None;
                self.pending_screen_clear = true;
                self.pass_offset += 1;
                if let Err(err) = self.advance_after_human_lock() {
                    self.record_halt_error(err.clone());
                    return Err(err);
                }
                Ok(())
            }
        }
    }

    /// Validates and applies a post-season [`SeatIntent`] (`SubmitMandateReveal`, `ExportReplay`, or
    /// `Quit`) strictly outside `TalkAndMarket`.
    pub fn apply_post_season_intent(&mut self, intent: SeatIntent) -> Result<(), StructuredError> {
        let unexpected_payload = intent.market_action.is_some()
            || intent.bundle_action.is_some()
            || intent.what_if.is_some();
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
        match intent.kind {
            SeatIntentKind::Quit => {
                self.quit_requested = true;
                Ok(())
            }
            SeatIntentKind::SubmitMandateReveal => {
                if self.host.phase() != TurnPhase::AwaitingMandateReveal {
                    return Err(host_error(
                        "INVALID_PHASE",
                        "refused SubmitMandateReveal outside AwaitingMandateReveal",
                        None,
                    ));
                }
                let revealed = ClubSessionCheckpoint::submit_next_mandate_reveal(&mut self.host)
                    .map_err(|err| scrub_engine_error(&err))?;
                if revealed == self.host.seat_count() {
                    let (scoreboard, replay_log, section, season_metrics) = self
                        .host
                        .finalize_club_season()
                        .map_err(|err| scrub_engine_error(&err))?;
                    let revealed_mandates = extract_revealed_mandates(
                        &replay_log,
                        usize::from(self.host.seat_count()),
                    )?;
                    self.end_of_season = Some(EndOfSeasonSessionState {
                        data: EndOfSeasonData {
                            table_id: self.host.table_id(),
                            closed_turn: Turn(scoreboard.closed_turn),
                            scoreboard,
                            revealed_mandates,
                            close_predicates: section.at_close,
                            season_metrics,
                            export_state: ReplayExportState::Ready,
                        },
                        ui_state: EndOfSeasonUiState::new(),
                        replay_log,
                        export_error: None,
                    });
                    self.maybe_autosave()?;
                }
                Ok(())
            }
            SeatIntentKind::ExportReplay => {
                if self.host.phase() != TurnPhase::Completed {
                    return Err(host_error(
                        "INVALID_PHASE",
                        "refused ExportReplay outside Completed phase",
                        Some("Reveal all mandates to finalize the season before exporting."),
                    ));
                }
                let (replay_path, seal_path) = self.resolve_replay_and_seal_paths();
                let bytes = match self
                    .end_of_season
                    .as_ref()
                    .ok_or_else(|| {
                        host_error(
                            "MISSING_END_OF_SEASON_DATA",
                            "End-of-Season replay log unavailable",
                            None,
                        )
                    })
                    .and_then(|eos| {
                        eos.replay_log
                            .to_json_pretty_bytes()
                            .map_err(|err| scrub_engine_error(&err))
                    }) {
                    Ok(bytes) => bytes,
                    Err(err) => {
                        if let Some(eos) = &mut self.end_of_season {
                            eos.data.export_state = ReplayExportState::Failed;
                            eos.export_error = Some(err.clone());
                        }
                        return Err(err);
                    }
                };
                if let Err(io_err) = write_atomic_file_bytes(&replay_path, &bytes) {
                    let err = StructuredError::new(
                        "TUI_REPLAY_EXPORT_FAILED",
                        scrub_secret_text(&format!(
                            "Replay export failed at {}: {io_err}",
                            replay_path.display()
                        )),
                        "Check directory permissions/path and press `e` or `Enter` to retry replay export.",
                        true,
                    );
                    if let Some(eos) = &mut self.end_of_season {
                        eos.data.export_state = ReplayExportState::Failed;
                        eos.export_error = Some(err.clone());
                    }
                    return Err(err);
                }
                if let Err(wipe_err) = self.wipe_seal_after_season_export(&seal_path) {
                    let err = StructuredError::new(
                        "TUI_REPLAY_EXPORT_FAILED",
                        scrub_secret_text(&format!(
                            "Replay written at {}, but failed removing {}: {}",
                            replay_path.display(),
                            seal_path.display(),
                            wipe_err.message
                        )),
                        "Ensure `.seal` can be removed and retry replay export.",
                        true,
                    )
                    .with_cause(wipe_err.code);
                    if let Some(eos) = &mut self.end_of_season {
                        eos.data.export_state = ReplayExportState::Failed;
                        eos.export_error = Some(err.clone());
                    }
                    return Err(err);
                }
                if let Some(eos) = &mut self.end_of_season {
                    eos.data.export_state = ReplayExportState::Exported;
                    eos.export_error = None;
                }
                Ok(())
            }
            _ => Err(host_error(
                "INVALID_PHASE",
                &format!(
                    "refused {:?}: not supported in post-season phase {:?}",
                    intent.kind,
                    self.host.phase()
                ),
                None,
            )),
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
                    self.open_curtain_for_seat(seat);
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
                }
            }
        }
    }
}

pub(crate) fn parse_club_table_id_from_filename(name: &str) -> Option<u64> {
    let rest = name.strip_prefix("club-")?;
    let digits = rest
        .strip_suffix(".save.json")
        .or_else(|| rest.strip_suffix(".replay.json"))
        .or_else(|| rest.strip_suffix(".seal"))?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<u64>().ok()
}

pub(crate) fn is_same_existing_file(a: &Path, b: &Path) -> bool {
    std::fs::canonicalize(a)
        .ok()
        .zip(std::fs::canonicalize(b).ok())
        .is_some_and(|(ca, cb)| ca == cb)
}

pub(crate) fn validate_resume_autosave_target(
    resumed_save: &Path,
    resumed_seal: &Path,
    target_dir: &Path,
    table_id: TableId,
) -> Result<(PathBuf, PathBuf), StructuredError> {
    let base = format!("club-{}", table_id.0);
    let s_path = target_dir.join(format!("{base}.save.json"));
    let k_path = target_dir.join(format!("{base}.seal"));
    let r_path = target_dir.join(format!("{base}.replay.json"));

    // Decide whether the target is the same season by actual file identity on disk, never by file name.
    if is_same_existing_file(&s_path, resumed_save) && is_same_existing_file(&k_path, resumed_seal)
    {
        return Ok((resumed_save.to_path_buf(), resumed_seal.to_path_buf()));
    }

    if target_dir.exists() {
        let entries = std::fs::read_dir(target_dir).map_err(|err| {
            StructuredError::new(
                "SAVE_DIR_UNREADABLE",
                scrub_secret_text(&format!(
                    "failed inspecting save directory {}: {err}",
                    target_dir.display()
                )),
                "Check `--save-dir` permissions or pass a different directory.",
                false,
            )
        })?;
        let mut unfinished_seals: Vec<String> = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|err| {
                StructuredError::new(
                    "SAVE_DIR_UNREADABLE",
                    scrub_secret_text(&format!(
                        "failed iterating save directory {}: {err}",
                        target_dir.display()
                    )),
                    "Check `--save-dir` permissions or pass a different directory.",
                    false,
                )
            })?;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("club-")
                && name.ends_with(".seal")
                && !is_same_existing_file(&path, resumed_seal)
            {
                unfinished_seals.push(name);
            }
        }
        if !unfinished_seals.is_empty() {
            unfinished_seals.sort();
            let seal_name = &unfinished_seals[0];
            return Err(StructuredError::new(
                "UNFINISHED_SEASON_EXISTS",
                scrub_secret_text(&format!(
                    "unfinished season `{seal_name}` already exists in {}; refusing to overwrite",
                    target_dir.display()
                )),
                "Resume directly from that directory with `--resume <PATH>`, finish & export its replay if completed, or choose another `--save-dir`.",
                false,
            ));
        }
        if (k_path.exists() && !is_same_existing_file(&k_path, resumed_seal))
            || (s_path.exists() && !is_same_existing_file(&s_path, resumed_save))
            || r_path.exists()
        {
            return Err(StructuredError::new(
                "SEASON_SAVE_EXISTS",
                scrub_secret_text(&format!(
                    "season archive for `{base}` already exists in {}; refusing to overwrite",
                    target_dir.display()
                )),
                "Resume without `--save-dir` to save in place, or choose an empty/different `--save-dir`.",
                false,
            ));
        }
    }

    Ok((s_path, k_path))
}

pub(crate) fn inspect_save_dir_for_new_season(dir: &Path) -> Result<TableId, StructuredError> {
    if !dir.exists() {
        return Ok(TableId(1));
    }
    let entries = std::fs::read_dir(dir).map_err(|err| {
        StructuredError::new(
            "SAVE_DIR_UNREADABLE",
            scrub_secret_text(&format!(
                "failed inspecting save directory {}: {err}",
                dir.display()
            )),
            "Check `--save-dir` permissions or pass a different directory.",
            false,
        )
    })?;
    let mut unfinished_seals: Vec<String> = Vec::new();
    let mut max_id: Option<u64> = None;
    for entry in entries {
        let entry = entry.map_err(|err| {
            StructuredError::new(
                "SAVE_DIR_UNREADABLE",
                scrub_secret_text(&format!(
                    "failed iterating save directory {}: {err}",
                    dir.display()
                )),
                "Check `--save-dir` permissions or pass a different directory.",
                false,
            )
        })?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("club-") && name.ends_with(".seal") {
            unfinished_seals.push(name.clone());
        }
        if let Some(id) = parse_club_table_id_from_filename(&name) {
            max_id = Some(max_id.map_or(id, |m| m.max(id)));
        }
    }
    if !unfinished_seals.is_empty() {
        unfinished_seals.sort();
        let seal_name = &unfinished_seals[0];
        return Err(StructuredError::new(
            "UNFINISHED_SEASON_EXISTS",
            scrub_secret_text(&format!(
                "unfinished season `{seal_name}` already exists in {}; refusing to overwrite",
                dir.display()
            )),
            "Resume it with `--resume <PATH>`, finish & export its replay if completed, or choose another `--save-dir`.",
            false,
        ));
    }
    let next = max_id.map_or(1, |m| m.saturating_add(1)).max(1);
    Ok(TableId(next))
}

fn check_explicit_table_slot_in_dir(dir: &Path, table_id: TableId) -> Result<(), StructuredError> {
    let _ = inspect_save_dir_for_new_season(dir)?;
    if dir.exists() {
        let base = format!("club-{}", table_id.0);
        if dir.join(format!("{base}.save.json")).exists()
            || dir.join(format!("{base}.seal")).exists()
            || dir.join(format!("{base}.replay.json")).exists()
        {
            return Err(StructuredError::new(
                "SEASON_SAVE_EXISTS",
                scrub_secret_text(&format!(
                    "table `{base}` already exists in {}; refusing to overwrite",
                    dir.display()
                )),
                "Choose another `--save-dir` or start without an explicit occupied TableId.",
                false,
            ));
        }
    }
    Ok(())
}

/// Interactive Club Mode pre-game Lobby host session (`docs/club-mode-design.md` §1.2).
///
/// Holds zero `ClubHost`, mandate, salt, or VRF state until `validate_and_start` validates every
/// Lobby parameter, enforces CLI flag precedence (`CLI > Lobby`), refuses overwriting any existing
/// unfinished `.seal` in `save_dir`, and opens the first human seat's Privacy Curtain strictly via
/// [`TuiHostSession::open_initial_curtain_for_active_human`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LobbyHostSession {
    state: LobbyState,
    overrides: LobbyCliOverrides,
    quit_requested: bool,
    last_error: Option<StructuredError>,
}

impl LobbyHostSession {
    /// Creates a new Club Mode Lobby session displaying any CLI override flags.
    #[must_use]
    pub fn new(overrides: LobbyCliOverrides) -> Self {
        Self {
            state: LobbyState::new(),
            overrides,
            quit_requested: false,
            last_error: None,
        }
    }

    /// Returns the pure Lobby UI state.
    #[cfg(test)]
    #[must_use]
    pub const fn state(&self) -> &LobbyState {
        &self.state
    }

    /// Mutable access to Lobby UI state for deterministic host tests.
    #[cfg(test)]
    pub(crate) fn state_mut(&mut self) -> &mut LobbyState {
        &mut self.state
    }

    /// Returns the latest scrubbed validation refusal, if any.
    #[cfg(test)]
    #[must_use]
    pub const fn last_error(&self) -> Option<&StructuredError> {
        self.last_error.as_ref()
    }

    /// Returns whether the user asked to quit the Lobby without starting a table.
    #[must_use]
    pub const fn quit_requested(&self) -> bool {
        self.quit_requested
    }

    fn record_refusal(&mut self, mut err: StructuredError) -> StructuredError {
        err.message = scrub_secret_text(&err.message);
        self.last_error = Some(err.clone());
        err
    }

    fn fail(&mut self, code: &'static str, raw_msg: &str, hint: &'static str) -> StructuredError {
        self.record_refusal(StructuredError::new(
            code,
            scrub_secret_text(raw_msg),
            hint,
            false,
        ))
    }

    /// Builds the current Lobby [`SeatScreenView`], overlaying any scrubbed refusal banner.
    #[must_use]
    pub fn build_view(&self, viewport: ViewportSize) -> SeatScreenView {
        let mut view = league_seat_ui::render_lobby_screen(&self.state, self.overrides, viewport);
        if let Some(err) = &self.last_error
            && let Some(status) = &mut view.status_line
        {
            let banner = scrub_secret_text(&format!("REFUSED [{}]: {}", err.code, err.message));
            status.alert = Some(ViewRow::from_spans(vec![ViewSpan::styled(
                banner,
                ViewStyle::plain().with_fg(ViewColor::Red).bold(),
            )]));
        }
        view
    }

    /// Renders the Lobby screen onto `frame`.
    pub fn render_frame(&self, frame: &mut Frame<'_>) {
        let vp = ViewportSize::new(frame.area().width, frame.area().height);
        let view = self.build_view(vp);
        draw_screen_view(frame, &view);
    }

    /// Processes one key event inside the Lobby and returns the resulting [`LobbyAction`].
    pub fn handle_key_event(&mut self, key: SeatKeyEvent, viewport: ViewportSize) -> LobbyAction {
        self.last_error = None;
        let step =
            league_seat_ui::handle_lobby_key(self.state.clone(), self.overrides, viewport, key);
        self.state = step.state;
        if step.action == LobbyAction::Quit {
            self.quit_requested = true;
        }
        step.action
    }

    /// Validates all Lobby settings (`dev` seed only, `4..=6` seats, `>= 1` human, valid clock),
    /// allocates the next free `TableId` inside `save_dir` (refusing `UNFINISHED_SEASON_EXISTS` if
    /// any `club-*.seal` still exists there), enforces CLI overrides, builds [`TuiHostSession`],
    /// and opens Seat 0's Curtain cleanly via `open_curtain_for_seat`.
    pub(crate) fn validate_and_start_in_dir(
        &mut self,
        bot_tie_seed: u64,
        save_dir: Option<&Path>,
    ) -> Result<TuiHostSession, StructuredError> {
        let table_id = match save_dir {
            Some(dir) => {
                inspect_save_dir_for_new_season(dir).map_err(|err| self.record_refusal(err))?
            }
            None => TableId(1),
        };
        self.validate_state_and_start(table_id, bot_tie_seed, save_dir, false)
    }

    /// Validates all Lobby settings (`dev` seed only, `4..=6` seats, `>= 1` human, valid clock),
    /// refuses overwriting any existing unfinished `.seal` or `club-<table_id>.*` files in `save_dir`,
    /// enforces CLI overrides (`--turn-clock`, `--untimed`, `--pins`), builds the [`TuiHostSession`],
    /// and opens the first human seat's Curtain cleanly via `open_curtain_for_seat`.
    #[cfg(test)]
    pub fn validate_and_start(
        &mut self,
        table_id: TableId,
        bot_tie_seed: u64,
        save_dir: Option<&Path>,
    ) -> Result<TuiHostSession, StructuredError> {
        self.validate_state_and_start(table_id, bot_tie_seed, save_dir, true)
    }

    fn validate_state_and_start(
        &mut self,
        table_id: TableId,
        bot_tie_seed: u64,
        save_dir: Option<&Path>,
        check_explicit_dir: bool,
    ) -> Result<TuiHostSession, StructuredError> {
        let seat_count = self.state.seat_count();
        let humans = self.state.human_seat_indices();
        let seed_raw = self.state.seed_text().trim().to_owned();
        let lobby_untimed = self.state.untimed();
        let lobby_clock_secs = self.state.turn_clock_secs();
        let lobby_pins = self.state.pins_enabled();
        self.validate_params_and_start_inner(
            seat_count,
            &humans,
            &seed_raw,
            lobby_untimed,
            lobby_clock_secs,
            lobby_pins,
            table_id,
            bot_tie_seed,
            save_dir,
            check_explicit_dir,
        )
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn validate_params_and_start(
        &mut self,
        seat_count: u8,
        humans: &[u8],
        seed_raw: &str,
        lobby_untimed: bool,
        lobby_clock_secs: u16,
        lobby_pins: bool,
        table_id: TableId,
        bot_tie_seed: u64,
        save_dir: Option<&Path>,
    ) -> Result<TuiHostSession, StructuredError> {
        self.validate_params_and_start_inner(
            seat_count,
            humans,
            seed_raw,
            lobby_untimed,
            lobby_clock_secs,
            lobby_pins,
            table_id,
            bot_tie_seed,
            save_dir,
            true,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn validate_params_and_start_inner(
        &mut self,
        seat_count: u8,
        humans: &[u8],
        seed_raw: &str,
        lobby_untimed: bool,
        lobby_clock_secs: u16,
        lobby_pins: bool,
        table_id: TableId,
        bot_tie_seed: u64,
        save_dir: Option<&Path>,
        check_explicit_dir: bool,
    ) -> Result<TuiHostSession, StructuredError> {
        if check_explicit_dir && let Some(dir) = save_dir {
            check_explicit_table_slot_in_dir(dir, table_id)
                .map_err(|err| self.record_refusal(err))?;
        }
        let sc = match seat_count {
            4 => SeatCount::Four,
            5 => SeatCount::Five,
            6 => SeatCount::Six,
            other => {
                return Err(self.fail(
                    "INVALID_SEAT_COUNT",
                    &format!("invalid seat_count {other}: expected 4, 5, or 6"),
                    "Set Lobby seat count to 4, 5, or 6.",
                ));
            }
        };
        if humans.is_empty() {
            return Err(self.fail(
                "NO_HUMAN_SEATS",
                "at least one seat must be Human to play Club Mode in league-tui",
                "Toggle at least one seat from ClubBot to Human before starting.",
            ));
        }

        let seed_trimmed = seed_raw.trim();
        let seed = seed_trimmed.parse::<u64>().map_err(|_| {
            self.fail(
                "INVALID_SEED_SYNTAX",
                &format!(
                    "invalid layout seed `{seed_trimmed}` (expected decimal integer in 0..=9999)"
                ),
                "Enter a clear integer seed in the dev pool (0..=9999).",
            )
        })?;
        if is_reserved_test_seed(seed) {
            let desc = if seed >= (1_u64 << 63) {
                format!(
                    "sealed-range digest={}",
                    league_core::layout_seed_digest(seed)
                )
            } else {
                seed.to_string()
            };
            return Err(self.fail(
                "RESERVED_SEED_FORBIDDEN",
                &format!("reserved test seed {desc} is forbidden for Club Mode tables"),
                "Select a non-reserved dev pool seed in 0..=9999.",
            ));
        }
        if seed_pool(seed) != SeedPool::Dev {
            return Err(self.fail(
                "SEED_POOL_MISMATCH",
                &format!(
                    "seed {seed} belongs to {:?}; Club Mode accepts SeedPool::Dev (0..=9999) only",
                    seed_pool(seed)
                ),
                "Select a dev pool seed in 0..=9999.",
            ));
        }

        let effective_clock = if self.overrides.untimed {
            None
        } else if let Some(cli_secs) = self.overrides.turn_clock_secs {
            if !(180..=300).contains(&cli_secs) {
                return Err(self.fail(
                    "INVALID_TURN_CLOCK",
                    &format!("turn clock {cli_secs} out of range (expected 180..=300 seconds)"),
                    "Specify a whole-turn budget in 180..=300 seconds or enable Untimed.",
                ));
            }
            Some(cli_secs)
        } else if lobby_untimed {
            None
        } else {
            if !(180..=300).contains(&lobby_clock_secs) {
                return Err(self.fail(
                    "INVALID_TURN_CLOCK",
                    &format!(
                        "turn clock {lobby_clock_secs} out of range (expected 180..=300 seconds)"
                    ),
                    "Choose a whole-turn budget in 180..=300 seconds or toggle Untimed.",
                ));
            }
            Some(lobby_clock_secs)
        };

        let cfg = GeneratorConfig {
            seat_count: Some(sc),
            ..GeneratorConfig::default()
        };
        let opts = TableSetupOptions {
            purpose: TablePurpose::Dev,
            training_logged: false,
        };
        let (checked, _, _) = generate_ring_table_for_setup(seed, opts, &cfg).map_err(|err| {
            let structured = err.to_structured_error();
            self.record_refusal(structured)
        })?;
        let ring = checked.into_clear_table().ok_or_else(|| {
            self.fail(
                "SEED_POOL_MISMATCH",
                "generated RingTable missing clear dev seed",
                "Select a clear dev pool seed in 0..=9999.",
            )
        })?;

        let mut session = TuiHostSession::from_ring_table(&ring, table_id, humans, bot_tie_seed)
            .map_err(|err| self.record_refusal(err.to_structured_error()))?;
        session.configure_turn_clock(effective_clock);
        if self.overrides.pins_enabled || lobby_pins {
            session.enable_seat_pins();
        }
        session.open_initial_curtain_for_active_human();
        if let Some(dir) = save_dir {
            let base = format!("club-{}", table_id.0);
            session.enable_autosave(
                dir.join(format!("{base}.save.json")),
                dir.join(format!("{base}.seal")),
            );
            session
                .maybe_autosave()
                .map_err(|err| self.record_refusal(err))?;
        }
        Ok(session)
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
        session.advance_clock(CURTAIN_LIFT_DEAD_TIME);
        must_ok(session.handle_key_event(key(' '), vp), "curtain space");
        draw_and_capture(session, terminal, vp, session.turn(), None, frames);
        must_ok(
            session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
            "curtain enter",
        );
        session.advance_clock(CURTAIN_LIFT_DEAD_TIME);
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
            assert_eq!(obs.own_mandate(), Some(own));
            let obs_dbg = format!("{obs:?}");
            assert!(!obs_dbg.contains(&format!("{}", own.bonus_equity)));
            for (r, rival_obs) in all_obs.iter().enumerate() {
                if r != s {
                    let rival_mandate = must_some(rival_obs.own_mandate(), "rival mandate");
                    if rival_mandate != own {
                        assert_ne!(
                            obs.own_mandate(),
                            Some(rival_mandate),
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

        session.advance_clock(CURTAIN_LIFT_DEAD_TIME);
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
        session.advance_clock(CURTAIN_LIFT_DEAD_TIME);
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
        session.advance_clock(CURTAIN_LIFT_DEAD_TIME);
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
        session.advance_clock(CURTAIN_LIFT_DEAD_TIME);
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
        let (mut session, mut headless, _dmap) =
            build_explicit_sessions(&ring, TableId(81_008), &[0], deal_seed, [0x77; 32], 41);
        let dir = std::env::temp_dir().join(format!("ml_season_1h_e2e_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        must_ok(std::fs::create_dir_all(&dir), "mkdir 1h");
        let save_path = dir.join("club-81008.save.json");
        let seal_path = dir.join("club-81008.seal");
        let replay_path = dir.join("club-81008.replay.json");
        session.enable_autosave(save_path.clone(), seal_path.clone());

        let vp = ViewportSize::new(100, 30);
        let mut terminal = must_ok(Terminal::new(TestBackend::new(100, 30)), "100x30");
        let mut frames: Vec<CapturedSeasonDraw> = Vec::new();
        let mut opened_chronicle = false;
        let mut did_mid_resume = false;
        let n = usize::from(headless.seat_count());

        while session.phase() == TurnPhase::TalkAndMarket {
            let t = session.turn();
            if t == 5 && session.pass() == PassKind::SubPassA && !did_mid_resume {
                did_mid_resume = true;
                assert!(
                    session.is_curtain_open(),
                    "Turn 5 Pass A must start behind Curtain"
                );
                must_ok(
                    session.write_save_and_seal(&save_path, &seal_path),
                    "mid-season 1h save",
                );
                let mut resumed = must_ok(
                    TuiHostSession::read_save_and_seal(
                        &save_path,
                        &seal_path,
                        session.commodities.clone(),
                    ),
                    "mid-season 1h resume",
                );
                assert!(
                    resumed.is_curtain_open(),
                    "mid-season resume must land behind a Curtain"
                );
                resumed.enable_autosave(save_path.clone(), seal_path.clone());
                session = resumed;
            }
            let cur_pass = session.pass();

            unlock_curtain_and_capture(&mut session, &mut terminal, vp, &mut frames);
            let seat = must_some(session.active_human_seat(), "active seat 0");
            assert_eq!(seat, Seat(0));

            if cur_pass == PassKind::SubPassA {
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
                for offset in 0..n {
                    let s = u8::try_from((usize::from(t - 1) + offset) % n).unwrap_or(0);
                    if s == 0 {
                        if t == 1 {
                            must_ok(
                                headless.apply_market_action(MarketAction::PostOffer {
                                    seat: 0,
                                    order: OfferProposal {
                                        side: OfferSide::Buy,
                                        good: AssetId::Raw(1),
                                        amount: 1_000,
                                        price_per_unit_cred: 1_200,
                                        domain_id: None,
                                        is_hybrid: false,
                                    },
                                }),
                                "headless PostOffer T1",
                            );
                        }
                    } else {
                        let obs = must_ok(headless.observe_seat(Seat(s)), "headless bot obs A");
                        let m = *must_some(obs.own_mandate(), "bot mandate A");
                        for act in ClubBot::new(headless.bot_tie_seed()).plan_pass_a(&obs, &m) {
                            must_ok(headless.apply_market_action(act), "headless bot A");
                        }
                    }
                }
                must_ok(headless.begin_cancel_lock(), "headless cancel lock 1h");
            } else {
                let mut headless_bundles = vec![SealedOrderBundle::default(); n];
                if t <= 20 {
                    must_ok(session.handle_key_event(key('2'), vp), "tab 2");
                    must_ok(
                        session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
                        "stage converter",
                    );
                    BundleDraftAction::SetConverterRun(ConverterRunOrder {
                        recipe_idx: 0,
                        runs: 1,
                    })
                    .apply_to(&mut headless_bundles[0]);
                }
                if t.is_multiple_of(4) {
                    let op = BundleDraftAction::SetCouncilVote(Some(CouncilVoteBallot {
                        trade_minister: 0,
                        cartel_chair: 1,
                    }));
                    op.apply_to(&mut headless_bundles[0]);
                    must_ok(
                        session.apply_seat_intent(Seat(0), SeatIntent::update_bundle(op)),
                        "stage turn 4n vote",
                    );
                }
                if t == 6 {
                    let op = BundleDraftAction::StageCharterBid(CharterBidOrder {
                        role: CharterRole::OracleAuditor,
                        bid_cred: 2_500,
                    });
                    op.apply_to(&mut headless_bundles[0]);
                    must_ok(
                        session.apply_seat_intent(Seat(0), SeatIntent::update_bundle(op)),
                        "stage turn 6 charter bid",
                    );
                }
                for offset in 0..n {
                    let s = u8::try_from((usize::from(t - 1) + offset) % n).unwrap_or(0);
                    if s != 0 {
                        let obs = must_ok(headless.observe_seat(Seat(s)), "headless bot obs B");
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
                must_ok(
                    headless.commit_and_reveal_all(&reveals),
                    "headless commit 1h",
                );
                must_ok(headless.sweep_turn(), "headless sweep 1h");
            }
            draw_and_capture(&session, &mut terminal, vp, t, Some(0), &mut frames);
            must_ok(
                session.handle_key_event(ctrl_key('l'), vp),
                "lock and handoff",
            );
        }

        assert!(did_mid_resume, "1h season must exercise mid-season resume");
        assert!(
            opened_chronicle,
            "Chronicle modal must be opened during season"
        );
        assert_eq!(session.phase(), TurnPhase::AwaitingMandateReveal);
        assert_eq!(headless.phase(), TurnPhase::AwaitingMandateReveal);
        assert!(session.public_receipts().len() >= 24);
        assert_eq!(
            session.public_receipts().len(),
            headless.public_receipts().len()
        );
        for (idx, (tr, hr)) in session
            .public_receipts()
            .iter()
            .zip(headless.public_receipts().iter())
            .enumerate()
        {
            assert_eq!(
                must_ok(tr.to_json_string(), "tr json").as_bytes(),
                must_ok(hr.to_json_string(), "hr json").as_bytes(),
                "1h receipt {} diverged from headless",
                idx + 1
            );
        }

        // Capture AwaitingMandateReveal frames through N-1 reveals (all public standby, 0 mandates).
        for step_idx in 0..n {
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
            assert!(post_txt.contains(&format!("{step_idx}/{n}")));
            if step_idx + 1 < n {
                must_ok(
                    session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
                    "submit intermediate reveal",
                );
                assert_eq!(session.phase(), TurnPhase::AwaitingMandateReveal);
            }
        }

        // Final reveal triggers finalize_season() -> Completed 4-tab End-of-Season screen.
        must_ok(
            session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
            "submit final reveal",
        );
        assert_eq!(session.phase(), TurnPhase::Completed);

        let (_sb, log, _sec, _met) =
            must_ok(headless.finalize_club_season(), "finalize headless 1h");
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

        // Inspect all 4 End-of-Season tabs and verify secrets/salts/digests stay absent, then export replay.
        for (tab_key, expected_text) in [
            ('1', "Final Standings"),
            ('2', "Unsealed Turn-1 Private Charter Mandates"),
            ('3', "HoldingMark"),
            ('4', "Replay Export"),
        ] {
            must_ok(session.handle_key_event(key(tab_key), vp), "eos tab");
            must_ok(
                terminal.draw(|f| must_ok(session.render_current_frame(f), "draw eos")),
                "draw eos",
            );
            let eos_txt = screen_text(&terminal);
            assert!(
                eos_txt.contains(expected_text),
                "missing {expected_text}:\n{eos_txt}"
            );
            for salt in &salt_hexes {
                assert!(!eos_txt.contains(salt));
                assert!(!eos_txt.contains(&salt[..8]));
            }
            for dig in &replay_out.state_digests {
                let dhex: String = dig[..4].iter().map(|b| format!("{b:02x}")).collect();
                assert!(
                    !eos_txt.contains(&dhex),
                    "End-of-Season leaked digest {dhex}"
                );
            }
        }
        assert!(seal_path.exists());
        must_ok(session.handle_key_event(key('e'), vp), "export replay 1h");
        assert!(!seal_path.exists(), "export must wipe .seal");
        assert!(replay_path.exists(), "export must write replay json");
        let parsed = must_ok(
            SeasonReplayLog::from_club_json_slice(&must_ok(std::fs::read(&replay_path), "read")),
            "parse exported club replay 1h",
        );
        assert_eq!(parsed.turns.len(), log.turns.len());
        let _ = std::fs::remove_dir_all(&dir);
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
        let mut did_mid_resume = false;
        let dir = std::env::temp_dir().join(format!("ml_season_2h_e2e_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        must_ok(std::fs::create_dir_all(&dir), "mkdir 2h");
        let save_path = dir.join("club-81009.save.json");
        let seal_path = dir.join("club-81009.seal");
        let replay_path = dir.join("club-81009.replay.json");
        session.enable_autosave(save_path.clone(), seal_path.clone());

        while session.phase() == TurnPhase::TalkAndMarket {
            let turn = headless.turn();
            let mut headless_bundles = vec![SealedOrderBundle::default(); n];
            if turn == 5 && !did_mid_resume {
                did_mid_resume = true;
                assert!(
                    session.is_curtain_open(),
                    "Turn 5 must open behind Seat 0's Curtain"
                );
                must_ok(
                    session.write_save_and_seal(&save_path, &seal_path),
                    "mid-season 2h save",
                );
                let mut resumed = must_ok(
                    TuiHostSession::read_save_and_seal(
                        &save_path,
                        &seal_path,
                        session.commodities.clone(),
                    ),
                    "mid-season 2h resume",
                );
                assert!(
                    resumed.is_curtain_open(),
                    "mid-season 2h resume must land behind a Curtain"
                );
                resumed.enable_autosave(save_path.clone(), seal_path.clone());
                session = resumed;
            }

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

        assert!(
            did_mid_resume,
            "2-human season must exercise mid-season resume"
        );
        assert!(opened_chronicle, "2-human season must open Chronicle modal");
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

        // Capture every AwaitingMandateReveal frame before the final reveal -> standby only.
        for step_idx in 0..n {
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
            assert!(post_txt.contains(&format!("{step_idx}/{n}")));
            if step_idx + 1 < n {
                must_ok(
                    session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
                    "2h reveal step",
                );
                assert_eq!(session.phase(), TurnPhase::AwaitingMandateReveal);
            }
        }

        must_ok(
            session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
            "2h final reveal",
        );
        assert_eq!(session.phase(), TurnPhase::Completed);

        let (_sb, log, _sec, _met) =
            must_ok(headless.finalize_club_season(), "finalize 2-human headless");
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

        // Render all 4 End-of-Season tabs, check 0 salt/digest leaks, then export replay & wipe .seal.
        for (tab_key, expected) in [
            ('1', "Final Standings"),
            ('2', "Unsealed Turn-1 Private Charter Mandates"),
            ('3', "HoldingMark"),
            ('4', "Replay Export"),
        ] {
            must_ok(session.handle_key_event(key(tab_key), vp), "2h eos tab");
            must_ok(
                terminal.draw(|f| must_ok(session.render_current_frame(f), "draw 2h eos")),
                "draw 2h eos",
            );
            let txt = screen_text(&terminal);
            assert!(txt.contains(expected), "2h eos missing {expected}:\n{txt}");
            for salt in &salt_hexes {
                assert!(!txt.contains(salt));
                assert!(!txt.contains(&salt[..8]));
            }
            for dig in &replay_out.state_digests {
                let dhex: String = dig[..4].iter().map(|b| format!("{b:02x}")).collect();
                assert!(!txt.contains(&dhex), "2h eos leaked digest {dhex}");
            }
        }
        assert!(seal_path.exists());
        must_ok(session.handle_key_event(key('e'), vp), "2h export replay");
        assert!(!seal_path.exists(), "2h export must wipe .seal");
        assert!(replay_path.exists(), "2h export must write replay");
        let parsed = must_ok(
            SeasonReplayLog::from_club_json_slice(&must_ok(std::fs::read(&replay_path), "read")),
            "parse 2h replay",
        );
        assert_eq!(parsed.turns.len(), log.turns.len());
        let _ = std::fs::remove_dir_all(&dir);
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
            session.advance_clock(CURTAIN_LIFT_DEAD_TIME);
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
            session.advance_clock(CURTAIN_LIFT_DEAD_TIME);
            must_ok(session.handle_key_event(key(' '), vp), "space T1 B");
            must_ok(
                session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
                "enter T1 B",
            );
            session.advance_clock(CURTAIN_LIFT_DEAD_TIME);
            must_ok(session.handle_key_event(ctrl_key('l'), vp), "lock T1 B");
            session.advance_clock(CURTAIN_LIFT_DEAD_TIME);
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
            session.advance_clock(CURTAIN_LIFT_DEAD_TIME);
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

    fn extract_ledger_cells_from_buffer(
        buf: &ratatui::buffer::Buffer,
        w: u16,
        h: u16,
    ) -> std::collections::BTreeMap<u8, (String, String, String, String)> {
        let cell_str = |y: u16, x_start: usize, x_end: usize| -> String {
            (x_start..x_end)
                .map(|x| buf[(u16::try_from(x).unwrap_or(0), y)].symbol())
                .collect::<String>()
                .trim()
                .to_owned()
        };
        let (hdr_seat, hdr_tier, hdr_tar, seat_c, cred_c, debt_c, tier_c, tar_c) =
            if w >= 100 && h >= 30 {
                let inner_w = ((usize::from(w) * 30) / 100).saturating_sub(2);
                if inner_w >= 34 {
                    (
                        "Seat",
                        "Tier",
                        "Tar(R/M)",
                        (1usize, 5usize),
                        (6, 13),
                        (14, 20),
                        (21, 25),
                        (26, 35),
                    )
                } else {
                    (
                        "St",
                        "Tr",
                        "TarR/M",
                        (1usize, 3usize),
                        (4, 10),
                        (11, 15),
                        (16, 19),
                        (20, 29),
                    )
                }
            } else {
                (
                    "Seat",
                    "Tier",
                    "Tar(R/M)",
                    (1usize, 5usize),
                    (11, 19),
                    (20, 27),
                    (28, 32),
                    (41, 50),
                )
            };

        let hdr_y = (0..h)
            .find(|&y| {
                cell_str(y, seat_c.0, seat_c.1) == hdr_seat
                    && cell_str(y, cred_c.0, cred_c.1) == "CRED"
                    && cell_str(y, debt_c.0, debt_c.1) == "Debt"
                    && cell_str(y, tier_c.0, tier_c.1) == hdr_tier
                    && cell_str(y, tar_c.0, tar_c.1) == hdr_tar
            })
            .unwrap_or_else(|| {
                panic!(
                    "{w}x{h}: ledger column headers `{hdr_seat}|CRED|Debt|{hdr_tier}|{hdr_tar}` missing at expected x-ranges"
                )
            });

        let mut out = std::collections::BTreeMap::new();
        for y in (hdr_y + 1)..h {
            let s_txt = cell_str(y, seat_c.0, seat_c.1);
            let Some(rest) = s_txt.strip_prefix('S') else {
                break;
            };
            let Ok(seat_idx) = rest.parse::<u8>() else {
                break;
            };
            out.insert(
                seat_idx,
                (
                    cell_str(y, cred_c.0, cred_c.1),
                    cell_str(y, debt_c.0, debt_c.1),
                    cell_str(y, tier_c.0, tier_c.1),
                    cell_str(y, tar_c.0, tar_c.1),
                ),
            );
        }
        out
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
                let mut extracted_cells = std::collections::BTreeMap::new();
                if w >= 100 && h >= 30 {
                    must_ok(
                        terminal.draw(|f| must_ok(session.render_current_frame(f), "draw wide")),
                        "draw wide",
                    );
                    let wide_txt = screen_text(&terminal);
                    if (w, h) == (100, 30) && seat_count == SeatCount::Four {
                        seat0_100x30_row = wide_txt
                            .lines()
                            .find(|l| l.contains("S0 ") && (l.contains("Aaa") || l.contains("Bbb")))
                            .map(|l| l.trim().to_owned());
                    }
                    extracted_cells.extend(extract_ledger_cells_from_buffer(
                        terminal.backend().buffer(),
                        w,
                        h,
                    ));
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
                        extracted_cells.extend(extract_ledger_cells_from_buffer(
                            terminal.backend().buffer(),
                            w,
                            h,
                        ));
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

                for st in &obs.seats {
                    let (exp_cred, exp_debt, exp_tier, exp_tar) =
                        format_expected_ledger_cells(st, w, h);
                    let (got_cred, got_debt, got_tier, got_tar) =
                        extracted_cells.get(&st.seat).unwrap_or_else(|| {
                            panic!(
                                "{w}x{h} ({seat_count:?}) missing ledger cell row for S{}",
                                st.seat
                            )
                        });
                    assert_eq!(
                        got_cred, &exp_cred,
                        "{w}x{h} ({seat_count:?}) S{} CRED cell mismatch",
                        st.seat
                    );
                    assert_eq!(
                        got_debt, &exp_debt,
                        "{w}x{h} ({seat_count:?}) S{} Debt cell mismatch",
                        st.seat
                    );
                    assert_eq!(
                        got_tier, exp_tier,
                        "{w}x{h} ({seat_count:?}) S{} Tier cell mismatch",
                        st.seat
                    );
                    assert_eq!(
                        got_tar, &exp_tar,
                        "{w}x{h} ({seat_count:?}) S{} Tariff cell mismatch",
                        st.seat
                    );
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
            let cells = extract_ledger_cells_from_buffer(terminal.backend().buffer(), w, h);
            let (s0_cred, s0_debt, _, s0_tar) = cells
                .get(&0)
                .unwrap_or_else(|| panic!("{w}x{h} missing synthetic S0 cells"));
            let (s1_cred, s1_debt, _, s1_tar) = cells
                .get(&1)
                .unwrap_or_else(|| panic!("{w}x{h} missing synthetic S1 cells"));
            assert_eq!(s0_cred, "15000", "{w}x{h} S0 CRED cell");
            assert_eq!(s0_debt, "2450", "{w}x{h} S0 Debt cell");
            assert_eq!(s0_tar, "1245/708", "{w}x{h} S0 Tariff cell");
            assert!(
                s1_cred == "1500k" || s1_cred == "1.5M" || s1_cred == "1500000",
                "{w}x{h} S1 CRED cell must show 1.5M whole or unambiguous abbreviation, got `{s1_cred}`"
            );
            assert!(
                s1_debt == "15k" || s1_debt == "15000",
                "{w}x{h} S1 Debt cell must show 15000 whole or `15k`, got `{s1_debt}`"
            );
            assert_eq!(s1_tar, "500/250", "{w}x{h} S1 Tariff cell");
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
                false,
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
    fn failed_autosave_halts_and_reports_save_path_and_error_code() {
        let dir = std::env::temp_dir().join(format!("ml_autosave_fail_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("mkdir: {e}"));
        let not_a_dir = dir.join("unwritable_parent_file");
        std::fs::write(&not_a_dir, b"blocking file").unwrap_or_else(|e| panic!("write file: {e}"));
        let bad_save = not_a_dir.join("club-81205.save.json");
        let bad_seal = not_a_dir.join("club-81205.seal");

        let ring = dev_ring_table(1);
        let mut session = must_ok(
            TuiHostSession::from_ring_table(&ring, TableId(81_205), &[0], 79),
            "autosave fail session",
        );
        session.enable_autosave(bad_save.clone(), bad_seal);

        let err = must_err(
            session.handle_key_event(ctrl_key('l'), ViewportSize::new(100, 30)),
            "unwritable autosave target must fail and wedge session",
        );
        assert!(session.is_wedged(), "failed autosave must wedge session");
        assert!(
            !session.is_curtain_open(),
            "failed autosave must exit Curtain to halt screen"
        );
        assert!(
            err.message.contains("Autosave failed") && err.message.contains("club-81205.save.json"),
            "error must name autosave failure and path: {:?}",
            err
        );

        for (w, h) in [(80_u16, 24_u16), (100, 30)] {
            let mut terminal = must_ok(Terminal::new(TestBackend::new(w, h)), "tb");
            must_ok(
                terminal.draw(|f| must_ok(session.render_current_frame(f), "draw autosave halt")),
                "draw autosave halt",
            );
            let txt = screen_text(&terminal);
            assert!(
                txt.contains("Autosave failed") && txt.contains(err.code),
                "{w}x{h} halt screen must say Autosave failed and error code:\n{txt}"
            );
            assert!(
                txt.contains("club-81205.save.json"),
                "{w}x{h} halt screen must include save path:\n{txt}"
            );
            assert!(
                !txt.contains("Pass advance failed"),
                "{w}x{h} halt screen must not misreport save failure as pass advance failure:\n{txt}"
            );
        }

        // Pressing `q` on the autosave halt screen works immediately (not blocked by dead time).
        must_ok(
            session.handle_key_event(key('q'), ViewportSize::new(100, 30)),
            "q quits halted autosave screen",
        );
        assert!(session.quit_requested());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mid_turn_save_and_resume_at_curtains_matches_uninterrupted_receipts_keeps_secrets_in_seal_and_wipes_seal()
     {
        let vp = ViewportSize::new(100, 30);
        let ring = dev_ring_table(1);
        let deal_seed = distinct_mandate_deal_seed(&ring, &[0, 1, 2]);
        let mut salts: Vec<RevealSalt> = (0..ring.seat_count.as_u8())
            .map(|i| RevealSalt([0x31_u8.wrapping_add(i); 32]))
            .collect();
        salts[0] = RevealSalt([0xC7; 32]);
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
        let baseline_mandate0 = deal.seat_reveals[0].mandate;

        // Also build an alternate deal on the exact same TableId + mandate deck that assigns Seat 0
        // a different PrivateMandate, so we can verify `.save.json` is byte-identical across deals.
        let mut alt_seed_opt = None;
        for byte_seed in 2_u8..=255 {
            let cand_seed = [byte_seed; 32];
            if let Ok((_, _, cand_deal, _)) = ring_table_to_table_spec_with_explicit_entropy(
                &ring,
                TableId(81_401),
                &[0, 1, 2],
                cand_seed,
                &salts,
                eval_seed.clone(),
            ) && cand_deal.seat_reveals[0].mandate != baseline_mandate0
            {
                alt_seed_opt = Some(cand_seed);
                break;
            }
        }
        let alt_deal_seed = must_some(alt_seed_opt, "distinct alternate deal_seed");
        let (alt_spec, _, alt_deal, alt_eval_seed) =
            ring_table_to_table_spec_with_explicit_entropy(
                &ring,
                TableId(81_401),
                &[0, 1, 2],
                alt_deal_seed,
                &salts,
                eval_seed.clone(),
            )
            .unwrap_or_else(|e| panic!("alt ring spec: {e}"));

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
        let baseline_receipt_bytes: Vec<Vec<u8>> = baseline
            .public_receipts()
            .iter()
            .map(|r| must_ok(r.to_json_string(), "baseline receipt json").into_bytes())
            .collect();

        // 2. Paused run: enable autosave, pause at Sub-Pass A Seat 2 Curtain, resume, then pause at
        // Sub-Pass B Seat 1 Curtain, resume, and drive to season close.
        let dir = std::env::temp_dir().join(format!("ml_partB2a_resume_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let save_path = dir.join("club-81401.save.json");
        let seal_path = dir.join("club-81401.seal");

        let mut run1 = must_ok(
            TuiHostSession::from_explicit_setup(
                spec.clone(),
                ring.seed,
                Some(cmap.to_display_map()),
                deal.clone(),
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

        // Verify the alternate mandate deal on identical public table produces byte-identical `.save.json`
        // before Seat 3 (ClubBot) runs.
        let alt_save = dir.join("club-81401-alt.save.json");
        let alt_seal = dir.join("club-81401-alt.seal");
        let mut alt_run = must_ok(
            TuiHostSession::from_explicit_setup(
                alt_spec,
                ring.seed,
                Some(cmap.to_display_map()),
                alt_deal,
                alt_eval_seed,
                83,
            ),
            "alt_run",
        );
        alt_run.enable_autosave(alt_save.clone(), alt_seal);
        type_text_in_mode(&mut alt_run, 'n', "NOTE_S0_SECRET_771", vp);
        must_ok(
            alt_run.handle_key_event(key_code(SeatKeyCode::Enter), vp),
            "alt_run s0 offer",
        );
        must_ok(alt_run.handle_key_event(ctrl_key('l'), vp), "alt s0 lock A");
        unlock_curtain_and_capture(&mut alt_run, &mut term, vp, &mut Vec::new());
        must_ok(alt_run.handle_key_event(ctrl_key('l'), vp), "alt s1 lock A");
        let save1_alt_txt =
            std::fs::read_to_string(&alt_save).unwrap_or_else(|e| panic!("read alt save1: {e}"));
        assert_eq!(
            save1_alt_txt, save1_txt,
            ".save.json must be byte-identical across different dealt mandates on the same table"
        );

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
        let s0_mandate = must_some(
            run2.build_seat_observation(Seat(0))
                .ok()
                .and_then(|o| o.own_mandate().copied()),
            "s0 m",
        );
        let s0_mandate_dbg = format!(
            "PrivateMandate {{ kind: {:?}, bonus_equity: {} }}",
            s0_mandate.kind, s0_mandate.bonus_equity
        );
        let kind_json = match s0_mandate.kind {
            MandateKind::MinProjectEquity { target } => {
                format!("{{\"MinProjectEquity\":{{\"target\":{target}}}}}")
            }
            MandateKind::MinRefinedVariety {
                distinct_goods,
                min_each,
            } => format!(
                "{{\"MinRefinedVariety\":{{\"distinct_goods\":{distinct_goods},\"min_each\":{min_each}}}}}"
            ),
            MandateKind::MaintainCleanCredit { min_tier } => {
                format!("{{\"MaintainCleanCredit\":{{\"min_tier\":\"{min_tier:?}\"}}}}")
            }
            MandateKind::MinActivePartners { min_partners } => {
                format!("{{\"MinActivePartners\":{{\"min_partners\":{min_partners}}}}}")
            }
        };
        let s0_mandate_json = format!(
            "{{\"kind\":{kind_json},\"bonus_equity\":{}}}",
            s0_mandate.bonus_equity
        );
        let s0_salt_hex: String = deal.seat_reveals[0]
            .salt
            .0
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let deal_seed_hex: String = deal.deal_seed.iter().map(|b| format!("{b:02x}")).collect();
        let s0_commit_hex = spec.dealt_mandate_commits[0].to_string();
        assert!(!save2_txt.contains("NOTE_S0_SECRET_771"));
        assert!(!save2_txt.contains("731"));
        assert!(!save2_txt.contains("debt_repayment_cred"));
        assert!(!save2_txt.contains(&s0_mandate_dbg));
        assert!(!save2_txt.contains(&s0_mandate_json));
        assert!(!save2_txt.contains(&s0_salt_hex));
        assert!(!save2_txt.contains(&deal_seed_hex));
        assert!(!save2_txt.contains(&s0_commit_hex));
        assert!(!save2_txt.contains("sealed_mandate_deal"));
        assert!(!save2_txt.contains("seat_reveals"));
        assert!(!save2_txt.contains("mandate_commits"));
        assert!(!save2_txt.contains("mandate_reveals"));
        assert!(!save2_txt.contains("vrf_proofs"));
        assert!(!save2_txt.contains("current_turn_salts"));
        assert!(seal2_txt.contains("NOTE_S0_SECRET_771"));
        assert!(seal2_txt.contains("731"));
        assert!(seal2_txt.contains(&s0_mandate_json));
        assert!(seal2_txt.contains(&s0_salt_hex));
        assert!(seal2_txt.contains(&deal_seed_hex));
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
        let run3_receipt_bytes: Vec<Vec<u8>> = run3
            .public_receipts()
            .iter()
            .map(|r| must_ok(r.to_json_string(), "run3 receipt json").into_bytes())
            .collect();
        assert_eq!(
            run3_receipt_bytes, baseline_receipt_bytes,
            "mid-turn paused/resumed season must produce byte-identical PublicSweepReceipts"
        );
        // C1: `wipe_seal_after_season_export` must still refuse in `AwaitingMandateReveal` and
        // succeed only once `TurnPhase::Completed` is reached.
        let standby_wipe_err = must_err(
            run3.wipe_seal_after_season_export(&seal_path),
            "cannot wipe .seal during AwaitingMandateReveal",
        );
        assert_eq!(standby_wipe_err.code, "SEASON_NOT_FINISHED");
        for _ in 0..run3.seat_count() {
            must_ok(
                run3.apply_post_season_intent(SeatIntent::submit_mandate_reveal()),
                "reveal mandate before C1 wipe",
            );
        }
        assert_eq!(run3.phase(), TurnPhase::Completed);

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
        session.advance_clock(CURTAIN_LIFT_DEAD_TIME);
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
        session.advance_clock(CURTAIN_LIFT_DEAD_TIME);
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
        session.advance_clock(CURTAIN_LIFT_DEAD_TIME);
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
        resumed_pin_session.advance_clock(CURTAIN_LIFT_DEAD_TIME);
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

    #[test]
    fn debug_formatting_redacts_seat_pins_pin_entry_digits_and_notes() {
        let vp = ViewportSize::new(100, 30);
        let ring = dev_ring_table(1);
        let mut session = must_ok(
            TuiHostSession::from_ring_table(&ring, TableId(81_509), &[0, 1], 97),
            "debug redaction session",
        );
        session.enable_seat_pins();
        session.advance_clock(CURTAIN_LIFT_DEAD_TIME);
        must_ok(session.handle_key_event(key(' '), vp), "arm curtain");
        must_ok(
            session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
            "enter initial pin prompt",
        );
        for d in ['4', '8', '2'] {
            must_ok(
                session.handle_key_event(key(d), vp),
                "type partial pin digit",
            );
        }
        let partial_dbg = format!("{session:?}");
        let partial_pretty = format!("{session:#?}");
        assert!(
            !partial_dbg.contains("482") && !partial_pretty.contains("482"),
            "partial CurtainPinEntry digits must be redacted from Debug:\n{partial_dbg}"
        );
        assert!(
            partial_dbg.contains("<redacted>") && partial_pretty.contains("<redacted>"),
            "Debug output must visibly mark redacted secrets:\n{partial_dbg}"
        );

        must_ok(
            session.handle_key_event(key('9'), vp),
            "type final pin digit",
        );
        must_ok(
            session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
            "confirm pin",
        );
        assert!(!session.is_curtain_open());
        assert_eq!(session.seat_pins[0].as_deref(), Some("4829"));

        session.advance_clock(CURTAIN_LIFT_DEAD_TIME);
        must_ok(session.handle_key_event(key('n'), vp), "open notes");
        for ch in "SECRET_NOTE_DEBUG_902".chars() {
            must_ok(session.handle_key_event(key(ch), vp), "type note char");
        }
        must_ok(
            session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
            "save note",
        );
        assert_eq!(session.seat_notes[0], "SECRET_NOTE_DEBUG_902");

        let post_dbg = format!("{session:?}");
        let post_pretty = format!("{session:#?}");
        for text in [&post_dbg, &post_pretty] {
            assert!(
                !text.contains("4829"),
                "seat_pins must be redacted from Debug:\n{text}"
            );
            assert!(
                !text.contains("SECRET_NOTE_DEBUG_902"),
                "seat_notes must be redacted from Debug:\n{text}"
            );
            assert!(text.contains("<redacted>"));
        }

        // Verify staged bundle & CurtainPinEntry Debug formatting also redact planted secrets.
        must_ok(
            session.apply_seat_intent(
                Seat(0),
                league_seat_types::SeatIntent::update_bundle(
                    league_seat_types::BundleDraftAction::SetDebtRepaymentCred(765_432),
                ),
            ),
            "stage debt repayment marker",
        );
        session.seat_states[0].apply_note_key(key('Z'));
        let pin_entry_only = CurtainPinEntry {
            mode: CurtainPinMode::VerifyPin,
            digits: "7391".to_owned(),
            wrong_tries: 1,
        };
        let curtain_only = CurtainState {
            next_seat: Seat(0),
            space_armed: true,
            pin_entry: Some(pin_entry_only.clone()),
            locked_out_reset: false,
        };
        let all_dbg = [
            format!("{pin_entry_only:?}"),
            format!("{pin_entry_only:#?}"),
            format!("{curtain_only:?}"),
            format!("{curtain_only:#?}"),
            format!("{session:?}"),
            format!("{session:#?}"),
        ];
        for text in &all_dbg {
            assert!(text.contains("<redacted>"), "missing <redacted>:\n{text}");
            for secret in ["4829", "7391", "SECRET_NOTE_DEBUG_902", "765432"] {
                assert!(
                    !text.contains(secret),
                    "planted secret `{secret}` leaked in Debug:\n{text}"
                );
            }
        }

        // Also build a real `SeatScreenView` for unlocked Seat 0 (carrying Seat 0's mandate,
        // Vault note, and staged debt repayment) and verify `{:?}`/`{:#?}` of `&view`, `&view.zones`,
        // each inner `ViewPanel`, `&view.status_line`, and `&view.modal` contain zero secret text.
        must_ok(
            session.handle_key_event(key('2'), vp),
            "switch composer to bundle tab so staged debt_repay renders",
        );
        must_ok(
            session.handle_key_event(key('r'), vp),
            "open chronicle modal",
        );
        let real_view = must_ok(
            session.build_current_view(vp),
            "build real seat view with planted note/mandate/bundle",
        );
        let obs0 = must_ok(session.build_seat_observation(Seat(0)), "obs0 debug view");
        let mandate0_marker =
            mandate_short_marker(must_some(obs0.own_mandate(), "s0 mandate").kind);
        let plain = real_view.plain_text();
        assert!(
            plain.contains(&mandate0_marker),
            "view plain_text must hold mandate:\n{plain}"
        );
        assert!(
            plain.contains("SECRET_NOTE_DEBUG_902"),
            "view plain_text must hold note:\n{plain}"
        );
        assert!(
            plain.contains("765432"),
            "view plain_text must hold staged debt repayment:\n{plain}"
        );
        let mut inner_view_dbg = vec![
            format!("{real_view:?}"),
            format!("{real_view:#?}"),
            format!("{:?}", real_view.zones),
            format!("{:#?}", real_view.zones),
            format!("{:?}", real_view.status_line),
            format!("{:#?}", real_view.status_line),
            format!("{:?}", real_view.modal),
            format!("{:#?}", real_view.modal),
        ];
        for zp in &real_view.zones {
            inner_view_dbg.push(format!("{:?}", zp.panel));
            inner_view_dbg.push(format!("{:#?}", zp.panel));
        }
        for text in &inner_view_dbg {
            assert!(text.contains("<redacted>"), "missing <redacted>:\n{text}");
            for secret in [&mandate0_marker, "SECRET_NOTE_DEBUG_902", "765432"] {
                assert!(
                    !text.contains(secret),
                    "real view building block leaked `{secret}` in Debug:\n{text}"
                );
            }
        }
    }

    #[test]
    fn read_save_and_seal_refuses_every_bad_checkpoint_rule_by_name_and_builds_no_session() {
        let dir =
            std::env::temp_dir().join(format!("ml_tui_bad_seal_rules_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        must_ok(std::fs::create_dir_all(&dir), "mkdir bad_seal");
        let save_path = dir.join("club-81520.save.json");
        let seal_path = dir.join("club-81520.seal");

        let ring = dev_ring_table(1);
        let mut session = must_ok(
            TuiHostSession::from_ring_table(&ring, TableId(81_520), &[0, 1], 31),
            "setup bad_seal session",
        );
        must_ok(
            session.write_save_and_seal(&save_path, &seal_path),
            "write valid save/seal",
        );
        let base_save = must_ok(std::fs::read_to_string(&save_path), "read valid save");
        let base_seal = must_ok(std::fs::read_to_string(&seal_path), "read valid seal");

        let cases: &[(&str, &str, &str)] = &[
            (
                "INVALID_CHECKPOINT_COMMITTED_LEN",
                "\"session_sub_pass_b_committed\":[false,false,false,false]",
                "\"session_sub_pass_b_committed\":[false,false]",
            ),
            (
                "INVALID_CHECKPOINT_NOTES_LEN",
                "\"session_seat_notes\":[\"\",\"\",\"\",\"\"]",
                "\"session_seat_notes\":[\"\",\"\"]",
            ),
            (
                "INVALID_CHECKPOINT_PINS_LEN",
                "\"session_seat_pins\":[null,null,null,null]",
                "\"session_seat_pins\":[null,null]",
            ),
            (
                "INVALID_CHECKPOINT_PINS_LEN",
                "\"session_seat_pins\":[null,null,null,null],\"session_pins_enabled\":false",
                "\"session_seat_pins\":[],\"session_pins_enabled\":true",
            ),
            (
                "INVALID_CHECKPOINT_PIN_FORMAT",
                "\"session_seat_pins\":[null,null,null,null]",
                "\"session_seat_pins\":[\"12a4\",null,null,null]",
            ),
            (
                "CONFLICTING_CHECKPOINT_CLOCK",
                "\"session_turn_clock_secs\":null,\"session_untimed\":false",
                "\"session_turn_clock_secs\":210,\"session_untimed\":true",
            ),
            (
                "INVALID_CHECKPOINT_TURN_CLOCK",
                "\"session_turn_clock_secs\":null,\"session_untimed\":false",
                "\"session_turn_clock_secs\":179,\"session_untimed\":false",
            ),
            (
                "INVALID_CHECKPOINT_TURN_CLOCK",
                "\"session_turn_clock_secs\":null,\"session_untimed\":false",
                "\"session_turn_clock_secs\":301,\"session_untimed\":false",
            ),
            (
                "INVALID_CHECKPOINT_PASS_OFFSET",
                "\"pass_offset\":0,",
                "\"pass_offset\":9,",
            ),
            (
                "INVALID_SEAT",
                "\"curtain_next_seat\":null",
                "\"curtain_next_seat\":9",
            ),
            (
                "SEAL_LAYOUT_SEED_MISMATCH",
                "\"layout_seed\":1,",
                "\"layout_seed\":2,",
            ),
        ];

        for (idx, &(expected_code, from, to)) in cases.iter().enumerate() {
            assert!(
                base_seal.contains(from),
                "case {idx} ({expected_code}) pattern `{from}` not found in base seal"
            );
            let bad_seal = base_seal.replacen(from, to, 1);
            let bad_seal_path = dir.join(format!("bad-{idx}.seal"));
            must_ok(std::fs::write(&bad_seal_path, bad_seal), "write bad seal");
            let err = match TuiHostSession::read_save_and_seal(&save_path, &bad_seal_path, None) {
                Ok(_) => panic!("case {idx} ({expected_code}) unexpectedly constructed session"),
                Err(err) => err,
            };
            let combined = format!("{} | {}", err.code, err.message);
            assert!(
                combined.contains(expected_code),
                "case {idx}: expected `{expected_code}` in `{combined}`"
            );
        }

        // Also shrink `session_pending_bundles` array from 4 to 2 elements without serde_json.
        let needle = "\"session_pending_bundles\":[";
        let start = base_seal
            .find(needle)
            .unwrap_or_else(|| panic!("missing {needle}"))
            + needle.len();
        let after = &base_seal[start..];
        let end_rel = after
            .find("],\"session_sub_pass_b_committed\"")
            .unwrap_or_else(|| panic!("missing bundle list end"));
        let mut depth = 0_i32;
        let mut first_end = None;
        for (offset, b) in after[..end_rel].bytes().enumerate() {
            match b {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        first_end = Some(offset + 1);
                        break;
                    }
                }
                _ => {}
            }
        }
        let first_elem = &after[..first_end.unwrap_or_else(|| panic!("first bundle"))];
        let bad_bundles_seal = format!(
            "{}{first_elem},{first_elem}{}",
            &base_seal[..start],
            &after[end_rel..]
        );
        let bad_bundles_path = dir.join("bad-bundles.seal");
        must_ok(
            std::fs::write(&bad_bundles_path, bad_bundles_seal),
            "write bad bundles seal",
        );
        let bundles_err =
            match TuiHostSession::read_save_and_seal(&save_path, &bad_bundles_path, None) {
                Ok(_) => panic!("short session_pending_bundles unexpectedly constructed session"),
                Err(err) => err,
            };
        assert!(
            bundles_err
                .message
                .contains("INVALID_CHECKPOINT_BUNDLES_LEN"),
            "expected INVALID_CHECKPOINT_BUNDLES_LEN, got: {}",
            bundles_err.message
        );

        // Also test tampered `.save.json` mismatch -> SAVE_SEAL_SNAPSHOT_MISMATCH.
        let bad_save = base_save.replacen("\"turn\": 1,", "\"turn\": 9,", 1);
        assert_ne!(bad_save, base_save);
        let bad_save_path = dir.join("bad-save.json");
        must_ok(std::fs::write(&bad_save_path, bad_save), "write bad save");
        let err = match TuiHostSession::read_save_and_seal(&bad_save_path, &seal_path, None) {
            Ok(_) => panic!("tampered save unexpectedly constructed session"),
            Err(err) => err,
        };
        assert!(
            err.message.contains("SAVE_SEAL_SNAPSHOT_MISMATCH"),
            "expected SAVE_SEAL_SNAPSHOT_MISMATCH, got: {}",
            err.message
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lobby_refuses_every_invalid_seed_seat_and_clock_input_by_name_and_holds_no_private_state() {
        let vp = ViewportSize::new(100, 30);
        let mut terminal = must_ok(Terminal::new(TestBackend::new(100, 30)), "100x30");
        let mut lobby = LobbyHostSession::new(LobbyCliOverrides::default());

        // Before start: Debug & view contain zero mandates, salts, VRF state, or Private Vault.
        let dbg = format!("{lobby:?} | {lobby:#?}");
        must_ok(
            terminal.draw(|f| lobby.render_frame(f)),
            "draw initial lobby",
        );
        let view_txt = screen_text(&terminal);
        for surface in [&dbg, &view_txt] {
            for secret in [
                "PrivateMandate",
                "Private Vault",
                "SealedMandateDeal",
                "eval_vrf",
                "commit_salt_entropy",
            ] {
                assert!(
                    !surface.contains(secret),
                    "Lobby leaked `{secret}` before start:\n{surface}"
                );
            }
        }
        assert!(view_txt.contains("Club Mode Lobby") && view_txt.contains("Seat Count: 4"));

        // 1. ZERO HUMANS via Space on Seat 0 row -> NO_HUMAN_SEATS + scrubbed REFUSED banner.
        lobby.handle_key_event(key_code(SeatKeyCode::Down), vp);
        lobby.handle_key_event(key(' '), vp);
        assert!(lobby.state().human_seat_indices().is_empty());
        let err_no_human = must_err(
            lobby.validate_and_start(TableId(81_601), 1, None),
            "0 humans must fail",
        );
        assert_eq!(err_no_human.code, "NO_HUMAN_SEATS");
        assert_eq!(lobby.last_error().map(|e| e.code), Some("NO_HUMAN_SEATS"));
        must_ok(
            terminal.draw(|f| lobby.render_frame(f)),
            "draw lobby error banner",
        );
        let err_screen = screen_text(&terminal);
        assert!(err_screen.contains("REFUSED [NO_HUMAN_SEATS]:"));

        // Restore Seat 0 = Human.
        lobby.handle_key_event(key(' '), vp);
        assert_eq!(lobby.state().human_seat_indices(), vec![0]);

        // 2. INVALID_SEAT_COUNT (`3` and `7`, defensive host bounds outside UI range).
        for bad_n in [3_u8, 7] {
            let err = must_err(
                lobby.validate_params_and_start(
                    bad_n,
                    &[0],
                    "1",
                    false,
                    210,
                    false,
                    TableId(81_601),
                    1,
                    None,
                ),
                "bad seat count",
            );
            assert_eq!(err.code, "INVALID_SEAT_COUNT");
        }

        // Navigate Lobby down to the Seed row (`focus_row == 5` when seat_count == 4).
        for _ in 0..4 {
            lobby.handle_key_event(key_code(SeatKeyCode::Down), vp);
        }
        assert_eq!(lobby.state().focus_row(), 5);

        let clear_and_type_seed = |l: &mut LobbyHostSession, text: &str| {
            for _ in 0..24 {
                l.handle_key_event(key_code(SeatKeyCode::Backspace), vp);
            }
            for ch in text.chars() {
                l.handle_key_event(key(ch), vp);
            }
        };

        // 3. INVALID_SEED_SYNTAX (`""` via UI Backspace and `"abc"`).
        lobby.handle_key_event(key_code(SeatKeyCode::Backspace), vp);
        assert_eq!(lobby.state().seed_text(), "");
        assert_eq!(
            must_err(
                lobby.validate_and_start(TableId(81_601), 1, None),
                "empty seed"
            )
            .code,
            "INVALID_SEED_SYNTAX"
        );
        assert_eq!(
            must_err(
                lobby.validate_params_and_start(
                    4,
                    &[0],
                    "abc",
                    false,
                    210,
                    false,
                    TableId(81_601),
                    1,
                    None,
                ),
                "alpha seed"
            )
            .code,
            "INVALID_SEED_SYNTAX"
        );

        // 4. RESERVED_SEED_FORBIDDEN (probe/held-out/sealed endpoints typed into Lobby UI).
        for reserved in [
            "1000000",
            "1099999",
            "2000000",
            "2099999",
            &format!("{}", 1_u64 << 63),
        ] {
            clear_and_type_seed(&mut lobby, reserved);
            assert_eq!(lobby.state().seed_text(), reserved);
            let err = must_err(
                lobby.validate_and_start(TableId(81_601), 1, None),
                "reserved seed",
            );
            assert_eq!(
                err.code, "RESERVED_SEED_FORBIDDEN",
                "seed `{reserved}` got `{}`",
                err.code
            );
        }

        // 5. SEED_POOL_MISMATCH (Train / Probe non-reserved / HeldOut non-reserved typed into UI).
        for non_dev in ["10000", "1000050", "2000050"] {
            clear_and_type_seed(&mut lobby, non_dev);
            assert_eq!(lobby.state().seed_text(), non_dev);
            let err = must_err(
                lobby.validate_and_start(TableId(81_601), 1, None),
                "non-dev seed",
            );
            assert_eq!(
                err.code, "SEED_POOL_MISMATCH",
                "seed `{non_dev}` got `{}`",
                err.code
            );
        }

        // 6. INVALID_TURN_CLOCK (`179` and `301`, defensive host bounds outside UI step 180..=300).
        clear_and_type_seed(&mut lobby, "1");
        assert_eq!(lobby.state().seed_text(), "1");
        for bad_clock in [179_u16, 301] {
            let err = must_err(
                lobby.validate_params_and_start(
                    4,
                    &[0],
                    "1",
                    false,
                    bad_clock,
                    false,
                    TableId(81_601),
                    1,
                    None,
                ),
                "bad clock",
            );
            assert_eq!(err.code, "INVALID_TURN_CLOCK");
        }

        // 7. Valid 5-seat Lobby start (`Humans = [0, 2]`, dev seed `1`): starts behind Seat 0
        // Curtain via `open_curtain_for_seat`, draws 0 SeatObservations beforehand, and sets
        // DEFAULT_CLUB_HUMAN_REGENT_DOCTRINE on all human seats.
        lobby.state_mut().set_seat_count_step(true);
        lobby.state_mut().toggle_seat_role(Seat(2));
        let session = must_ok(
            lobby.validate_and_start(TableId(81_602), 13, None),
            "valid 5-seat lobby start",
        );
        assert!(session.is_curtain_open(), "Lobby start must open Curtain");
        assert_eq!(session.seat_count(), 5);
        let spec = session.host.public_spec();
        assert_eq!(spec.seats[0].identity, SeatIdentity::Human);
        assert_eq!(
            spec.seats[0].regent,
            Some(league_core::DEFAULT_CLUB_HUMAN_REGENT_DOCTRINE)
        );
        assert_eq!(spec.seats[1].identity, SeatIdentity::AiSyndicate);
        assert_eq!(spec.seats[1].regent, None);
        assert_eq!(spec.seats[2].identity, SeatIdentity::Human);
        assert_eq!(
            spec.seats[2].regent,
            Some(league_core::DEFAULT_CLUB_HUMAN_REGENT_DOCTRINE)
        );
        must_ok(
            terminal.draw(|f| must_ok(session.render_current_frame(f), "draw lobby curtain")),
            "draw lobby curtain",
        );
        assert_eq!(session.observe_seat_call_count(), 0);
        let ctxt = screen_text(&terminal);
        assert!(ctxt.contains("Hand keyboard to Seat 0"));
        assert!(!ctxt.contains("Private Vault"));
    }

    #[test]
    fn lobby_built_table_equals_equivalent_cli_flag_table_and_cli_overrides_win() {
        let vp = ViewportSize::new(100, 30);
        let mut lobby = LobbyHostSession::new(LobbyCliOverrides::default());
        // Navigate down to clock row (`focus_row == 6`) and bump `210 -> 240`, then Pins (`7`).
        for _ in 0..6 {
            lobby.handle_key_event(key_code(SeatKeyCode::Down), vp);
        }
        lobby.handle_key_event(key_code(SeatKeyCode::Right), vp);
        lobby.handle_key_event(key_code(SeatKeyCode::Right), vp);
        assert_eq!(lobby.state().turn_clock_secs(), 240);
        lobby.handle_key_event(key_code(SeatKeyCode::Down), vp);
        lobby.handle_key_event(key(' '), vp);
        assert!(lobby.state().pins_enabled());

        let from_lobby = must_ok(
            lobby.validate_and_start(TableId(81_610), 19, None),
            "lobby default table",
        );

        let ring = dev_ring_table(1);
        let mut from_flags = must_ok(
            TuiHostSession::from_ring_table(&ring, TableId(81_610), &[0], 19),
            "equivalent cli table",
        );
        from_flags.configure_turn_clock(Some(240));
        from_flags.enable_seat_pins();

        let mut spec_a = from_lobby.host.public_spec();
        let mut spec_b = from_flags.host.public_spec();
        // `eval_vrf_pubkey` is fresh OS CSPRNG per table start; compare all table parameters cleanly.
        spec_a.eval_vrf_pubkey = None;
        spec_b.eval_vrf_pubkey = None;
        assert_eq!(spec_a, spec_b);
        assert_eq!(from_lobby.pass_budget_secs(), from_flags.pass_budget_secs());
        assert_eq!(from_lobby.pins_enabled, from_flags.pins_enabled);
        assert_eq!(from_lobby.turn(), from_flags.turn());
        assert_eq!(from_lobby.phase(), from_flags.phase());

        // CLI overrides (`--turn-clock 180 --pins`) beat conflicting Lobby choices (`untimed`, `pins: false`).
        let mut cli_wins = LobbyHostSession::new(LobbyCliOverrides {
            turn_clock_secs: Some(180),
            untimed: false,
            pins_enabled: true,
        });
        cli_wins.state_mut().set_untimed(true);
        cli_wins.state_mut().set_pins_enabled(false);
        let sess_override = must_ok(
            cli_wins.validate_and_start(TableId(81_611), 19, None),
            "cli overrides win timed+pins",
        );
        assert_eq!(sess_override.pass_budget_secs(), Some(135));
        assert!(sess_override.pins_enabled);

        // CLI `--untimed` beats Lobby `turn_clock_secs: 300`.
        let mut cli_untimed = LobbyHostSession::new(LobbyCliOverrides {
            turn_clock_secs: None,
            untimed: true,
            pins_enabled: false,
        });
        for _ in 0..6 {
            cli_untimed.handle_key_event(key_code(SeatKeyCode::Down), vp);
        }
        for _ in 0..6 {
            cli_untimed.handle_key_event(key_code(SeatKeyCode::Right), vp);
        }
        let sess_untimed = must_ok(
            cli_untimed.validate_and_start(TableId(81_612), 19, None),
            "cli untimed wins",
        );
        assert_eq!(sess_untimed.pass_budget_secs(), None);
    }

    #[test]
    fn awaiting_mandate_reveal_frames_before_last_reveal_show_public_standby_and_zero_mandates() {
        let ring = dev_ring_table(1);
        let deal_seed = distinct_mandate_deal_seed(&ring, &[0]);
        let (mut session, headless, _) =
            build_explicit_sessions(&ring, TableId(81_620), &[0], deal_seed, [0x77; 32], 61);
        let vp = ViewportSize::new(100, 30);
        let mut terminal = must_ok(Terminal::new(TestBackend::new(100, 30)), "100x30");
        let n = usize::from(session.seat_count());

        let mut all_markers = Vec::new();
        for s in 0..n as u8 {
            let m = *must_some(
                must_ok(headless.observe_seat(Seat(s)), "obs").own_mandate(),
                "mandate",
            );
            all_markers.push(mandate_short_marker(m.kind));
        }

        while session.phase() == TurnPhase::TalkAndMarket {
            if session.is_curtain_open() {
                session.advance_clock(CURTAIN_LIFT_DEAD_TIME);
                must_ok(session.handle_key_event(key(' '), vp), "curtain space");
                must_ok(
                    session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
                    "curtain enter",
                );
                session.advance_clock(CURTAIN_LIFT_DEAD_TIME);
            }
            must_ok(session.handle_key_event(ctrl_key('l'), vp), "advance turn");
        }
        assert_eq!(session.phase(), TurnPhase::AwaitingMandateReveal);

        // Between the last Sweep (0/N) and the N-th reveal (N-1/N), every frame shows standby and
        // zero mandates for any seat (even seats whose individual reveal was already submitted).
        for count in 0..n {
            must_ok(
                terminal.draw(|f| must_ok(session.render_current_frame(f), "draw standby")),
                "draw standby",
            );
            let buf = screen_text(&terminal);
            assert!(buf.contains("Session Standby"));
            assert!(buf.contains("AwaitingMandateReveal"));
            assert!(buf.contains(&format!("{count}/{n}")));
            assert!(!buf.contains("End of Season"));
            assert!(!buf.contains("Private Vault"));
            for marker in &all_markers {
                assert!(
                    !buf.contains(marker),
                    "frame at {count}/{n} reveals leaked mandate `{marker}`:\n{buf}"
                );
            }
            if count + 1 < n {
                must_ok(
                    session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
                    "submit reveal",
                );
                assert_eq!(session.phase(), TurnPhase::AwaitingMandateReveal);
            }
        }

        // N-th reveal finalizes the season -> Completed 4-tab screen displays every mandate on Tab 2.
        must_ok(
            session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
            "submit final N-th reveal",
        );
        assert_eq!(session.phase(), TurnPhase::Completed);
        must_ok(session.handle_key_event(key('2'), vp), "tab 2 mandates");
        must_ok(
            terminal.draw(|f| must_ok(session.render_current_frame(f), "draw tab 2")),
            "draw tab 2",
        );
        let tab2 = screen_text(&terminal);
        assert!(tab2.contains("End of Season"));
        for marker in &all_markers {
            assert!(
                tab2.contains(marker),
                "post-finalize Tab 2 missing mandate `{marker}`:\n{tab2}"
            );
        }
    }

    #[test]
    fn replay_export_writes_verified_log_and_wipes_seal_while_failed_export_keeps_seal_and_shows_reason()
     {
        let dir =
            std::env::temp_dir().join(format!("ml_replay_export_gate_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        must_ok(std::fs::create_dir_all(&dir), "mkdir export test");
        let vp = ViewportSize::new(100, 30);
        let mut terminal = must_ok(Terminal::new(TestBackend::new(100, 30)), "100x30");
        let ring = dev_ring_table(1);

        let drive_to_completed = |tid: TableId, save_dir: &Path| {
            let deal_seed = distinct_mandate_deal_seed(&ring, &[0]);
            let (mut session, _headless, _) =
                build_explicit_sessions(&ring, tid, &[0], deal_seed, [0x77; 32], 67);
            let sp = save_dir.join(format!("club-{}.save.json", tid.0));
            let sl = save_dir.join(format!("club-{}.seal", tid.0));
            let rp = save_dir.join(format!("club-{}.replay.json", tid.0));
            session.enable_autosave(sp, sl.clone());
            while session.phase() == TurnPhase::TalkAndMarket {
                if session.is_curtain_open() {
                    session.advance_clock(CURTAIN_LIFT_DEAD_TIME);
                    must_ok(session.handle_key_event(key(' '), vp), "space");
                    must_ok(
                        session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
                        "enter",
                    );
                    session.advance_clock(CURTAIN_LIFT_DEAD_TIME);
                }
                must_ok(session.handle_key_event(ctrl_key('l'), vp), "advance");
            }
            for _ in 0..session.seat_count() {
                must_ok(
                    session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
                    "reveal",
                );
            }
            assert_eq!(session.phase(), TurnPhase::Completed);
            (session, sl, rp)
        };

        // Case A: successful real export writes verified replay JSON and wipes `.seal`.
        let dir_ok = dir.join("ok");
        must_ok(std::fs::create_dir_all(&dir_ok), "mkdir ok");
        let (mut sess_ok, seal_ok, replay_ok) = drive_to_completed(TableId(81_631), &dir_ok);
        assert!(seal_ok.exists(), ".seal must exist before export");
        must_ok(sess_ok.handle_key_event(key('4'), vp), "tab 4 ok");
        must_ok(sess_ok.handle_key_event(key('e'), vp), "export replay ok");
        assert!(!seal_ok.exists(), "successful export must wipe .seal");
        assert!(replay_ok.exists(), "successful export must write replay");
        assert!(!sess_ok.has_partial_post_season_failure());
        let parsed = must_ok(
            SeasonReplayLog::from_club_json_slice(&must_ok(std::fs::read(&replay_ok), "read")),
            "verify exported replay json",
        );
        assert!(parsed.turns.len() >= 24);
        must_ok(
            terminal.draw(|f| must_ok(sess_ok.render_current_frame(f), "draw tab 4 ok")),
            "draw tab 4 ok",
        );
        let ok_screen = screen_text(&terminal);
        assert!(ok_screen.contains("Exported & Verified"));

        // Case B: failed real export (target replay path is an existing non-empty directory) preserves
        // `.seal` on disk, flags partial failure (exit 3), and displays why on screen.
        let dir_fail = dir.join("fail");
        must_ok(std::fs::create_dir_all(&dir_fail), "mkdir fail");
        let (mut sess_fail, seal_fail, replay_fail) =
            drive_to_completed(TableId(81_632), &dir_fail);
        must_ok(
            std::fs::create_dir_all(replay_fail.join("blocking_subdir")),
            "make replay target an unreplaceable directory",
        );
        assert!(seal_fail.exists());
        must_ok(sess_fail.handle_key_event(key('4'), vp), "tab 4 fail");
        let export_err = must_err(
            sess_fail.handle_key_event(key('e'), vp),
            "failing export must return error",
        );
        assert_eq!(export_err.code, "TUI_REPLAY_EXPORT_FAILED");
        assert!(seal_fail.exists(), "failed export must keep .seal intact");
        assert!(sess_fail.has_partial_post_season_failure());
        must_ok(
            terminal.draw(|f| must_ok(sess_fail.render_current_frame(f), "draw tab 4 fail")),
            "draw tab 4 fail",
        );
        let fail_screen = screen_text(&terminal);
        assert!(fail_screen.contains("Export Failed"));
        assert!(fail_screen.contains("TUI_REPLAY_EXPORT_FAILED"));
        assert!(fail_screen.contains("Replay export failed"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn six_seat_mixed_humans_and_bots_end_of_season_hides_mandates_until_final_reveal() {
        let dir = std::env::temp_dir().join(format!("ml_six_seat_eos_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        must_ok(std::fs::create_dir_all(&dir), "mkdir 6-seat eos");
        let save_path = dir.join("club-81640.save.json");
        let seal_path = dir.join("club-81640.seal");
        let replay_path = dir.join("club-81640.replay.json");

        let ring = dev_ring_table_for_seats(1, SeatCount::Six);
        let deal_seed = distinct_mandate_deal_seed(&ring, &[0, 2, 4]);
        let (mut session, mut headless, _cmap) = build_explicit_sessions(
            &ring,
            TableId(81_640),
            &[0, 2, 4],
            deal_seed,
            [0x79; 32],
            71,
        );
        assert_eq!(session.seat_count(), 6);
        session.enable_autosave(save_path, seal_path.clone());

        let vp = ViewportSize::new(100, 30);
        let mut terminal = must_ok(Terminal::new(TestBackend::new(100, 30)), "100x30");
        let mut frames = Vec::new();

        // Play all 24..=32 turns through AwaitingMandateReveal on 6 seats (Humans=[0, 2, 4], Bots=[1, 3, 5]).
        while session.phase() == TurnPhase::TalkAndMarket {
            let t = session.turn();
            let mut headless_bundles = vec![SealedOrderBundle::default(); 6];
            for offset in 0..6 {
                let s = u8::try_from((usize::from(t - 1) + offset) % 6).unwrap_or(0);
                if matches!(s, 0 | 2 | 4) {
                    unlock_curtain_and_capture(&mut session, &mut terminal, vp, &mut frames);
                    assert_eq!(session.active_human_seat(), Some(Seat(s)));
                    draw_and_capture(&session, &mut terminal, vp, t, Some(s), &mut frames);
                    must_ok(session.handle_key_event(ctrl_key('l'), vp), "6-seat lock A");
                } else {
                    let obs = must_ok(headless.observe_seat(Seat(s)), "6-seat bot obs A");
                    let m = *must_some(obs.own_mandate(), "6-seat bot m A");
                    for act in ClubBot::new(headless.bot_tie_seed()).plan_pass_a(&obs, &m) {
                        must_ok(headless.apply_market_action(act), "6-seat bot act A");
                    }
                }
            }
            must_ok(headless.begin_cancel_lock(), "6-seat headless cancel lock");
            for offset in 0..6 {
                let s = u8::try_from((usize::from(t - 1) + offset) % 6).unwrap_or(0);
                if matches!(s, 0 | 2 | 4) {
                    unlock_curtain_and_capture(&mut session, &mut terminal, vp, &mut frames);
                    assert_eq!(session.active_human_seat(), Some(Seat(s)));
                    draw_and_capture(&session, &mut terminal, vp, t, Some(s), &mut frames);
                    must_ok(session.handle_key_event(ctrl_key('l'), vp), "6-seat lock B");
                } else {
                    let obs = must_ok(headless.observe_seat(Seat(s)), "6-seat bot obs B");
                    let m = *must_some(obs.own_mandate(), "6-seat bot m B");
                    let (acts, bundle) =
                        ClubBot::new(headless.bot_tie_seed()).plan_pass_b_and_bundle(&obs, &m);
                    for act in acts {
                        must_ok(headless.apply_market_action(act), "6-seat bot act B");
                    }
                    headless_bundles[usize::from(s)] = bundle;
                }
            }
            let mut reveals = BTreeMap::new();
            for s in 0..6_u8 {
                reveals.insert(
                    s,
                    OrderReveal {
                        bundle: std::mem::take(&mut headless_bundles[usize::from(s)]),
                        salt: must_ok(
                            ClubSessionCheckpoint::turn_reveal_salt(&headless, Seat(s)),
                            "6-seat headless salt",
                        ),
                    },
                );
            }
            must_ok(headless.commit_and_reveal_all(&reveals), "6-seat commit");
            must_ok(headless.sweep_turn(), "6-seat sweep");
        }

        assert_eq!(session.phase(), TurnPhase::AwaitingMandateReveal);
        let (_sb, log, _sec, _met) =
            must_ok(headless.finalize_club_season(), "6-seat finalize headless");
        let (replay_out, _) = must_ok(replay_season_with_state_predicates(&log), "6-seat replay");
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

        // Every AwaitingMandateReveal frame from 0/6 through 5/6 shows strictly public standby
        // and ZERO mandates of any of the 6 seats.
        for count in 0..6_usize {
            draw_and_capture(
                &session,
                &mut terminal,
                vp,
                headless.turn(),
                None,
                &mut frames,
            );
            let txt = screen_text(&terminal);
            assert!(txt.contains("Session Standby"));
            assert!(txt.contains(&format!("{count}/6")));
            assert!(!txt.contains("End of Season"));
            for m in &mandates {
                assert!(
                    !txt.contains(&m.short_full) && !txt.contains(&m.bonus_line),
                    "6-seat reveal frame {count}/6 leaked mandate `{}`:\n{txt}",
                    m.short_full
                );
            }
            if count + 1 < 6 {
                must_ok(
                    session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
                    "6-seat submit reveal",
                );
                assert_eq!(session.phase(), TurnPhase::AwaitingMandateReveal);
            }
        }
        assert_no_season_leaks(
            &frames,
            &mandates,
            &salt_hexes,
            &replay_out.state_digests,
            &drawn_shocks,
        );

        // 6th reveal transitions to Completed 4-tab End-of-Season showing all 6 seats & mandates.
        must_ok(
            session.handle_key_event(key_code(SeatKeyCode::Enter), vp),
            "6-seat final reveal",
        );
        assert_eq!(session.phase(), TurnPhase::Completed);
        must_ok(
            terminal.draw(|f| must_ok(session.render_current_frame(f), "6-seat draw tab 1")),
            "6-seat draw tab 1",
        );
        let tab1 = screen_text(&terminal);
        for s in 0..6 {
            assert!(
                tab1.contains(&format!("S{s} ")),
                "Tab 1 missing S{s}:\n{tab1}"
            );
        }
        must_ok(session.handle_key_event(key('2'), vp), "6-seat tab 2");
        must_ok(
            terminal.draw(|f| must_ok(session.render_current_frame(f), "6-seat draw tab 2")),
            "6-seat draw tab 2",
        );
        let tab2 = screen_text(&terminal);
        for m in &mandates {
            assert!(
                tab2.contains(&m.short_full),
                "6-seat Tab 2 missing mandate `{}`:\n{tab2}",
                m.short_full
            );
        }
        must_ok(session.handle_key_event(key('4'), vp), "6-seat tab 4");
        must_ok(session.handle_key_event(key('e'), vp), "6-seat export");
        assert!(!seal_path.exists());
        let exported = must_ok(
            SeasonReplayLog::from_club_json_slice(&must_ok(std::fs::read(&replay_path), "read")),
            "verify 6-seat exported replay",
        );
        assert_eq!(exported.turns.len(), log.turns.len());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
