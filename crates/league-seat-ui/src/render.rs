use alloc::borrow::ToOwned;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use league_seat_types::{
    ActiveCharter, AssetId, CLOSING_ROLL_START_TURN, COMMODITY_DISPLAY_CATALOG,
    CommodityDisplayMap, CredOffer, CreditCheck, CreditTier, CurtainPinMode, CurtainPinPrompt,
    DualEscrowVault, FillKind, FillRecord, HYPOTHETICAL_CLOSE_MARK_CAVEAT, IntentPledge,
    IntentTarget, MandateKind, MarketAction, PublicSweepReceipt, ScreenLayout, Seat, SeatIdentity,
    SeatObservation, SeatScreenView, SeatState, TokenEscrowVault, Turn, TurnClockDisplay,
    TurnPhase, ViewAlign, ViewColor, ViewLabel, ViewLayoutMode, ViewModal, ViewPanel, ViewRow,
    ViewScroll, ViewSizeHint, ViewSpan, ViewStatusLine, ViewStyle, ViewTabBar, ViewTabItem,
    ViewTable, ViewTableCell, ViewTableColumn, ViewTableRow, ViewZoneId, ViewZonePanel,
    ViewportSize, format_numeraire_display, scrub_secret_text,
};

use crate::composer::{
    build_what_if_request, bundle_field_count, market_field_count, preview_selected_asset,
};
use crate::state::{
    ALL_MARKET_KINDS, BundleSection, CenterTab, FocusZone, MarketActionKind, PassKind, SeatUiState,
    visible_bundle_sections,
};

fn format_signed_delta(delta: i64) -> String {
    if delta > 0 {
        format!("▲+{delta}")
    } else if delta < 0 {
        format!("▼{delta}")
    } else {
        "=0".to_owned()
    }
}

fn signed_delta_style(delta: i64, positive_is_good: bool) -> ViewStyle {
    if delta == 0 {
        return ViewStyle::plain();
    }
    let good = if positive_is_good {
        delta > 0
    } else {
        delta < 0
    };
    ViewStyle::plain().with_fg(if good {
        ViewColor::Green
    } else {
        ViewColor::Red
    })
}

/// Matches every settlement kind that `league-core` records in `EngineState::delivery_ledger`
/// (`BatchFill`, `ContractDelivery`, `CheckCashed`, and `AtomicSwap`).
fn fill_records_partner_delivery(f: &FillRecord) -> bool {
    match f.kind {
        FillKind::BatchFill | FillKind::ContractDelivery | FillKind::CheckCashed => f.net_units > 0,
        FillKind::AtomicSwap => f.net_units > 0 || f.cred_amount > 0,
        FillKind::TownImport
        | FillKind::EscrowFinished
        | FillKind::EscrowRefunded
        | FillKind::EscrowForfeited
        | FillKind::EscrowBreachRefunded => false,
    }
}

fn strip_style_colors(style: &mut ViewStyle) {
    style.fg = ViewColor::Reset;
    style.bg = ViewColor::Reset;
}

fn strip_panel_colors(panel: &mut ViewPanel) {
    if let Some(title) = &mut panel.title {
        strip_style_colors(&mut title.style);
    }
    if let Some(tab_bar) = &mut panel.tabs {
        for item in &mut tab_bar.tabs {
            strip_style_colors(&mut item.label.style);
        }
    }
    if let Some(table) = &mut panel.table {
        for col in &mut table.columns {
            strip_style_colors(&mut col.header.style);
        }
        for row in &mut table.rows {
            strip_style_colors(&mut row.style);
            for cell in &mut row.cells {
                for sp in &mut cell.spans {
                    strip_style_colors(&mut sp.style);
                }
            }
        }
    }
    for row in &mut panel.rows {
        for sp in &mut row.spans {
            strip_style_colors(&mut sp.style);
        }
    }
}

pub(crate) fn strip_colors_in_view(view: &mut SeatScreenView) {
    for z in &mut view.zones {
        strip_panel_colors(&mut z.panel);
    }
    if let Some(status) = &mut view.status_line {
        strip_style_colors(&mut status.style);
        if let Some(alert) = &mut status.alert {
            for sp in &mut alert.spans {
                strip_style_colors(&mut sp.style);
            }
        }
        for sp in &mut status.left {
            strip_style_colors(&mut sp.style);
        }
        for sp in &mut status.right {
            strip_style_colors(&mut sp.style);
        }
    }
    if let Some(modal) = &mut view.modal {
        strip_panel_colors(&mut modal.panel);
    }
}

const GUILD_NAMES: [&str; 6] = [
    "Aurelia House",
    "Boreal Guild",
    "Cinder Factor",
    "Dusk Consortium",
    "Ember Syndicate",
    "Flint Charter",
];

pub(crate) fn guild_name_for_seat(seat: u8) -> &'static str {
    GUILD_NAMES
        .get(usize::from(seat))
        .copied()
        .unwrap_or("Free Guild")
}

pub(crate) fn format_asset_label(
    asset: AssetId,
    commodities: Option<&CommodityDisplayMap>,
    plain_codes: bool,
) -> String {
    if asset == AssetId::Cred {
        return commodities
            .and_then(|m| m.format_asset(AssetId::Cred, plain_codes))
            .unwrap_or_else(|| format_numeraire_display(plain_codes).to_owned());
    }
    if let Some(found) = commodities.and_then(|m| m.format_asset(asset, plain_codes)) {
        return found;
    }
    let idx = match asset {
        AssetId::Raw(i) => usize::from(i) * 2,
        AssetId::Refined(i) => usize::from(i) * 2 + 1,
        AssetId::Cred => 0,
    };
    let entry = &COMMODITY_DISPLAY_CATALOG[idx % COMMODITY_DISPLAY_CATALOG.len()];
    match asset {
        AssetId::Raw(_) => entry.raw_display(plain_codes),
        AssetId::Refined(_) => entry.refined_display(plain_codes),
        AssetId::Cred => format_numeraire_display(plain_codes).to_owned(),
    }
}

pub(crate) fn apply_view_accessibility(
    mut view: SeatScreenView,
    no_color: bool,
    screen_reader: bool,
) -> SeatScreenView {
    if screen_reader {
        if let Some(modal) = view.modal.take()
            && let Some(first) = view.zones.first_mut()
        {
            let mut prefix = Vec::new();
            if let Some(title) = modal.panel.title {
                prefix.push(ViewRow::from_plain(format!(
                    "=== ACTIVE MODAL: {} ===",
                    title.text
                )));
            }
            prefix.extend(modal.panel.rows);
            prefix.push(ViewRow::from_plain(String::new()));
            prefix.append(&mut first.panel.rows);
            first.panel.rows = prefix;
        }
        for z in &mut view.zones {
            z.panel.bordered = false;
            z.panel.tabs = None;
            z.panel.alignment = ViewAlign::Left;
            if let Some(title) = z.panel.title.take() {
                z.panel.rows.insert(
                    0,
                    ViewRow::from_plain(format!("=== {} ===", title.text.to_uppercase())),
                );
            }
            if let Some(tbl) = z.panel.table.take() {
                for row in tbl.rows {
                    z.panel.rows.push(ViewRow::from_plain(row.plain_text()));
                }
            }
        }
    }
    if no_color || screen_reader {
        strip_colors_in_view(&mut view);
    }
    view
}

pub(crate) fn build_curtain_view(
    table_id: league_seat_types::TableId,
    turn: Turn,
    pass: PassKind,
    next_seat: Seat,
    viewport: ViewportSize,
    space_armed: bool,
    pin_prompt: CurtainPinPrompt,
) -> SeatScreenView {
    let layout = ScreenLayout::layout_for_viewport(viewport);
    if layout.mode == ViewLayoutMode::TooSmall {
        return build_too_small_view_with_options(viewport, layout, false, false);
    }

    let mut rows = vec![
        ViewRow::from_plain(format!("Table #{} | Turn {}", table_id.0, turn.0)),
        ViewRow::from_plain(format!("Pass: {}", pass.label())),
        ViewRow::from_plain(format!(
            "Hand keyboard to Seat {} ({})",
            next_seat.0,
            guild_name_for_seat(next_seat.0)
        )),
        ViewRow::from_plain(String::new()),
    ];

    let (prompt_row, footer_left, footer_right) = match pin_prompt.mode {
        CurtainPinMode::Inactive => {
            if pin_prompt.locked_out_reset {
                rows.push(ViewRow::from_spans(vec![ViewSpan::styled(
                    "▼ PIN entry failed 3 times — Curtain re-locked.".to_owned(),
                    ViewStyle::plain().with_fg(ViewColor::Red).bold(),
                )]));
            }
            let (msg, msg_style) = if space_armed {
                (
                    "▲ HAND-OFF ARMED ([Space] received) — press [Enter] to reveal seat screen",
                    ViewStyle::plain().with_fg(ViewColor::Yellow).bold(),
                )
            } else {
                (
                    "= PRIVACY CURTAIN — press [Space] then [Enter] when ready to reveal seat screen",
                    ViewStyle::plain().with_fg(ViewColor::Cyan),
                )
            };
            (
                ViewRow::from_spans(vec![ViewSpan::styled(msg.to_owned(), msg_style)]),
                "Space:Arm Hand-Off | Enter:Confirm Reveal | q:Quit".to_owned(),
                if space_armed {
                    "▲ Armed".to_owned()
                } else {
                    "= Locked Curtain".to_owned()
                },
            )
        }
        CurtainPinMode::SetInitialPin | CurtainPinMode::VerifyPin => {
            let filled = usize::from(pin_prompt.entered_count.min(4));
            let masked: String = (0..4)
                .map(|idx| if idx < filled { '*' } else { '_' })
                .collect();
            let (heading, heading_style) = match pin_prompt.mode {
                CurtainPinMode::SetInitialPin => (
                    format!("SET 4-DIGIT SEAT PIN: [{masked}] (digits never echoed)"),
                    ViewStyle::plain(),
                ),
                CurtainPinMode::VerifyPin => {
                    if pin_prompt.failed_attempts > 0 {
                        (
                            format!(
                                "▼ ENTER 4-DIGIT SEAT PIN: [{masked}] (wrong PIN; attempt {}/3)",
                                pin_prompt.failed_attempts + 1
                            ),
                            ViewStyle::plain().with_fg(ViewColor::Red).bold(),
                        )
                    } else {
                        (
                            format!("ENTER 4-DIGIT SEAT PIN: [{masked}]"),
                            ViewStyle::plain(),
                        )
                    }
                }
                CurtainPinMode::Inactive => (String::new(), ViewStyle::plain()),
            };
            (
                ViewRow::from_spans(vec![ViewSpan::styled(heading, heading_style)]),
                "0-9:PIN Digit | Backspace:Erase | Enter:Submit PIN | Esc:Cancel".to_owned(),
                "PIN Gate".to_owned(),
            )
        }
    };
    rows.push(prompt_row);
    let panel = ViewPanel {
        title: Some(ViewLabel::styled(
            "Hot-Seat Privacy Curtain".to_owned(),
            ViewStyle {
                fg: if space_armed {
                    ViewColor::Yellow
                } else {
                    ViewColor::Cyan
                },
                bg: ViewColor::Reset,
                bold: true,
                ..ViewStyle::plain()
            },
        )),
        alignment: ViewAlign::Center,
        bordered: true,
        tabs: None,
        table: None,
        rows,
        selected_row: None,
        scroll: None,
    };
    let mut view = SeatScreenView {
        layout: if viewport.height >= 8 {
            ScreenLayout::single_panel_with_status(viewport)
        } else {
            ScreenLayout::single_panel(viewport)
        },
        zones: vec![ViewZonePanel {
            zone: ViewZoneId::SingleFull,
            panel,
        }],
        status_line: None,
        modal: None,
    };
    view.status_line = Some(ViewStatusLine {
        alert: None,
        left: vec![ViewSpan::plain(footer_left)],
        right: vec![ViewSpan::plain(footer_right)],
        style: ViewStyle::plain(),
    });
    view
}

pub(crate) fn build_neutral_session_view(
    table_id: league_seat_types::TableId,
    turn: Turn,
    phase: TurnPhase,
    pass: PassKind,
    quit_requested: bool,
    halted_on_error: bool,
    viewport: ViewportSize,
) -> SeatScreenView {
    let layout = ScreenLayout::layout_for_viewport(viewport);
    if layout.mode == ViewLayoutMode::TooSmall {
        return build_too_small_view_with_options(viewport, layout, false, false);
    }

    let state_note = if quit_requested {
        "Session quit requested — returning to terminal."
    } else if halted_on_error {
        "Pass advance failed — session halted; press q to quit."
    } else if matches!(phase, TurnPhase::Commit | TurnPhase::Reveal) {
        "Mid-turn commit/reveal in progress — session standby."
    } else if phase != TurnPhase::TalkAndMarket {
        "Season closed — waiting for host post-season step."
    } else {
        "No human seat active — session standby."
    };
    let rows = vec![
        ViewRow::from_plain(format!(
            "Table #{} | Turn {} | Phase: {:?} | Pass: {}",
            table_id.0,
            turn.0,
            phase,
            pass.label()
        )),
        ViewRow::from_plain(state_note.to_owned()),
    ];
    let panel = ViewPanel {
        title: Some(ViewLabel::styled(
            "Merchant League — Session Standby".to_owned(),
            ViewStyle {
                fg: ViewColor::Cyan,
                bg: ViewColor::Reset,
                bold: true,
                ..ViewStyle::plain()
            },
        )),
        alignment: ViewAlign::Center,
        bordered: true,
        tabs: None,
        table: None,
        rows,
        selected_row: None,
        scroll: None,
    };
    SeatScreenView {
        layout: if viewport.height >= 8 {
            ScreenLayout::single_panel_with_status(viewport)
        } else {
            ScreenLayout::single_panel(viewport)
        },
        zones: vec![ViewZonePanel {
            zone: ViewZoneId::SingleFull,
            panel,
        }],
        status_line: Some(ViewStatusLine {
            alert: None,
            left: vec![ViewSpan::plain("q / Esc / Ctrl-C: Quit".to_owned())],
            right: vec![ViewSpan::plain("Standby".to_owned())],
            style: ViewStyle::plain(),
        }),
        modal: None,
    }
}

fn turn_mode_tag(state: &SeatUiState) -> &'static str {
    if state.composer_edit_active {
        "EDIT DRAFT (Enter apply | Esc cancel)"
    } else if state.notes_edit_active {
        "EDIT NOTES (Enter/Esc done)"
    } else {
        match state.selection.focus_zone {
            FocusZone::PublicLedger => "Focus: Ledger",
            FocusZone::CenterTabs => "Focus: Center",
            FocusZone::PrivateDrawer => "Focus: Private",
        }
    }
}

pub(crate) fn build_turn_screen(
    obs: &SeatObservation,
    state: &SeatUiState,
    pass: PassKind,
    commodities: Option<&CommodityDisplayMap>,
    viewport: ViewportSize,
) -> SeatScreenView {
    let layout = ScreenLayout::layout_for_viewport(viewport);
    if layout.mode == ViewLayoutMode::TooSmall {
        return build_too_small_view_with_options(
            viewport,
            layout,
            state.no_color,
            state.screen_reader,
        );
    }

    if state.screen_reader {
        return build_screen_reader_turn_screen(obs, state, pass, commodities, viewport);
    }

    let zones = vec![
        ViewZonePanel {
            zone: ViewZoneId::HeaderBar,
            panel: build_header_panel(
                obs,
                pass,
                commodities,
                state.plain_codes,
                state.clock_display,
            ),
        },
        ViewZonePanel {
            zone: ViewZoneId::PublicLedger,
            panel: build_public_ledger_panel(obs, state, commodities, viewport),
        },
        ViewZonePanel {
            zone: ViewZoneId::CenterTabs,
            panel: build_center_tabs_panel(obs, state, commodities),
        },
        ViewZonePanel {
            zone: ViewZoneId::PrivateDrawer,
            panel: build_private_drawer_panel(obs, state, commodities),
        },
    ];

    let modal = if state.chronicle_open {
        Some(build_chronicle_modal(obs, state, commodities))
    } else {
        None
    };

    let mode_tag = turn_mode_tag(state);
    let digest_key = if state.resume_digest_open {
        "d:HideDigest"
    } else {
        "d:Digest"
    };

    let mut view = SeatScreenView {
        layout,
        zones,
        status_line: Some(ViewStatusLine {
            alert: None,
            left: vec![ViewSpan::plain(
                "1-3:Tabs F6:Zone [/]:Item Arrows/+/-:Adj e:Draft n:Notes Enter:Apply".to_owned(),
            )],
            right: vec![ViewSpan::plain(format!(
                "[{mode_tag}] c:Codes {digest_key} r:Chron p:WhatIf Ctrl+L:Lock q:Quit"
            ))],
            style: ViewStyle::plain(),
        }),
        modal,
    };
    if state.no_color {
        strip_colors_in_view(&mut view);
    }
    view
}

fn build_too_small_view_with_options(
    viewport: ViewportSize,
    layout: ScreenLayout,
    no_color: bool,
    screen_reader: bool,
) -> SeatScreenView {
    let warn_style = ViewStyle::plain().with_fg(ViewColor::Yellow).bold();
    let panel = ViewPanel {
        title: if screen_reader {
            None
        } else {
            Some(ViewLabel::styled(
                "▼ Terminal too small".to_owned(),
                warn_style,
            ))
        },
        alignment: if screen_reader {
            ViewAlign::Left
        } else {
            ViewAlign::Center
        },
        bordered: !screen_reader,
        tabs: None,
        table: None,
        rows: vec![
            ViewRow::from_spans(vec![ViewSpan::styled(
                "▼ Terminal too small".to_owned(),
                warn_style,
            )]),
            ViewRow::from_plain(format!(
                "Minimum viewport: 80x24 | Current: {}x{}",
                viewport.width, viewport.height
            )),
            ViewRow::from_plain("Resize terminal or press q to quit.".to_owned()),
        ],
        selected_row: None,
        scroll: None,
    };
    let mut view = SeatScreenView {
        layout,
        zones: vec![ViewZonePanel {
            zone: ViewZoneId::SingleFull,
            panel,
        }],
        status_line: None,
        modal: None,
    };
    if no_color || screen_reader {
        strip_colors_in_view(&mut view);
    }
    view
}

fn format_clock_badge(clock: TurnClockDisplay) -> String {
    if !clock.timed {
        return "Clock: Untimed".to_owned();
    }
    let mins = clock.remaining_secs / 60;
    let secs = clock.remaining_secs % 60;
    format!("Clock: {mins:02}:{secs:02}")
}

fn build_header_panel(
    obs: &SeatObservation,
    pass: PassKind,
    commodities: Option<&CommodityDisplayMap>,
    plain_codes: bool,
    clock: TurnClockDisplay,
) -> ViewPanel {
    let active_role = obs
        .seats
        .get(usize::from(obs.seat))
        .map(|s| match s.identity {
            SeatIdentity::Human => "Human",
            SeatIdentity::AiSyndicate => "ClubBot (AiSyndicate)",
        })
        .unwrap_or("Human");
    let lock_badge = if obs.cancel_locked {
        "[CANCEL LOCKED]"
    } else {
        "[OPEN BOOK]"
    };
    let close_badge = if obs.turn >= CLOSING_ROLL_START_TURN {
        "Closing Window Active: 35.00% roll each Sweep (cap T32)"
    } else {
        "Season Open (Closing rolls begin Turn 24)"
    };
    let chair_label = obs
        .cartel_chair
        .map_or_else(|| "None".to_owned(), |c| format!("S{c}"));
    let shocks_line = if obs.active_shocks.is_empty() {
        "Active Shocks: none".to_owned()
    } else {
        let items: Vec<String> = obs
            .active_shocks
            .iter()
            .map(|s| {
                format!(
                    "S{} {:?} {}bps (exp T{})",
                    s.target_seat, s.kind, s.magnitude_bps, s.expires_after_turn
                )
            })
            .collect();
        format!("Active Shocks: {}", items.join("; "))
    };
    let forecasts_line = if obs.public_forecasts.is_empty() {
        "Forecasts: none".to_owned()
    } else {
        let items: Vec<String> = obs
            .public_forecasts
            .iter()
            .map(|f| format!("S{} {:?} at T{}", f.target_seat, f.kind, f.realize_turn))
            .collect();
        format!("Forecasts: {}", items.join("; "))
    };
    let cred_name = format_asset_label(AssetId::Cred, commodities, plain_codes);

    ViewPanel {
        title: Some(ViewLabel::styled(
            format!(
                "Merchant League — Table #{} | Turn {} | {:?} | Seat {} ({}) [{}]",
                obs.table_id.0,
                obs.turn,
                obs.phase,
                obs.seat,
                guild_name_for_seat(obs.seat),
                active_role
            ),
            ViewStyle {
                fg: ViewColor::Cyan,
                bg: ViewColor::Reset,
                bold: true,
                ..ViewStyle::plain()
            },
        )),
        alignment: ViewAlign::Left,
        bordered: true,
        tabs: None,
        table: None,
        rows: vec![
            ViewRow::from_plain(format!(
                "{} | {} | {} | TM: S{} | Chair: {}",
                pass.label(),
                format_clock_badge(clock),
                lock_badge,
                obs.trade_minister,
                chair_label,
            )),
            ViewRow::from_plain(format!("Unit: {cred_name} | {close_badge}")),
            ViewRow::from_plain(format!("{shocks_line} | {forecasts_line}")),
        ],
        selected_row: None,
        scroll: None,
    }
}

fn ledger_zone_inner_width(viewport: ViewportSize) -> usize {
    if viewport.width >= 100 && viewport.height >= 30 {
        ((usize::from(viewport.width) * 30) / 100).saturating_sub(2)
    } else {
        usize::from(viewport.width).saturating_sub(2)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LedgerTableLayout {
    Compact28,
    Compact34,
    FullStacked,
}

impl LedgerTableLayout {
    pub(crate) fn for_viewport(viewport: ViewportSize) -> Self {
        let inner_w = ledger_zone_inner_width(viewport);
        if inner_w >= 61 {
            Self::FullStacked
        } else if inner_w >= 34 {
            Self::Compact34
        } else {
            Self::Compact28
        }
    }
}

/// Formats an integer ledger value without ever dropping leading digits.
///
/// Values that fit in `max_chars` are returned in full; wider values abbreviate toward zero with
/// SI suffixes (`k`, `M`, `B`, `T`, `E`) and `+` whenever truncated non-zero lower digits remain.
pub(crate) fn format_ledger_units(value: i64, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let exact = format!("{value}");
    if exact.chars().count() <= max_chars {
        return exact;
    }

    let neg = value < 0;
    let abs_val = value.unsigned_abs();
    let sign = if neg { "-" } else { "" };
    for (div, suffix) in [
        (1_000_000_000_000_000_000_u64, 'E'),
        (1_000_000_000_000_000_u64, 'P'),
        (1_000_000_000_000_u64, 'T'),
        (1_000_000_000_u64, 'B'),
        (1_000_000_u64, 'M'),
        (1_000_u64, 'k'),
    ] {
        if abs_val < div {
            continue;
        }
        let whole = abs_val / div;
        let rem = abs_val % div;
        let tenths = (rem.saturating_mul(10)) / div;
        let mut candidates = Vec::new();
        if rem == 0 {
            candidates.push(format!("{sign}{whole}{suffix}"));
        } else {
            let exact_tenth = rem.saturating_mul(10) == tenths.saturating_mul(div);
            if exact_tenth {
                candidates.push(format!("{sign}{whole}.{tenths}{suffix}"));
            } else {
                candidates.push(format!("{sign}{whole}.{tenths}{suffix}+"));
            }
            candidates.push(format!("{sign}{whole}{suffix}+"));
            candidates.push(format!("{sign}{whole}{suffix}"));
        }
        for cand in candidates {
            if cand.chars().count() <= max_chars {
                return cand;
            }
        }
    }

    if max_chars == 1 {
        return "…".to_owned();
    }
    let mut clipped: String = exact.chars().take(max_chars - 1).collect();
    clipped.push('…');
    clipped
}

pub(crate) fn format_tariff_pair(raw_bps: u16, refined_bps: u16, max_chars: usize) -> String {
    let full = format!("{raw_bps}/{refined_bps}");
    if full.chars().count() <= max_chars {
        return full;
    }
    let side_w = (max_chars.saturating_sub(1)) / 2;
    format!(
        "{}/{}",
        format_ledger_units(i64::from(raw_bps), side_w),
        format_ledger_units(i64::from(refined_bps), side_w)
    )
}

fn build_public_ledger_panel(
    obs: &SeatObservation,
    state: &SeatUiState,
    commodities: Option<&CommodityDisplayMap>,
    viewport: ViewportSize,
) -> ViewPanel {
    let table_layout = LedgerTableLayout::for_viewport(viewport);
    let columns = match table_layout {
        LedgerTableLayout::Compact28 => vec![
            ViewTableColumn {
                header: ViewLabel::plain("St".to_owned()),
                width: ViewSizeHint::Fixed(2),
                alignment: ViewAlign::Left,
            },
            ViewTableColumn {
                header: ViewLabel::plain("CRED".to_owned()),
                width: ViewSizeHint::Fixed(6),
                alignment: ViewAlign::Right,
            },
            ViewTableColumn {
                header: ViewLabel::plain("Debt".to_owned()),
                width: ViewSizeHint::Fixed(4),
                alignment: ViewAlign::Right,
            },
            ViewTableColumn {
                header: ViewLabel::plain("Tr".to_owned()),
                width: ViewSizeHint::Fixed(3),
                alignment: ViewAlign::Left,
            },
            ViewTableColumn {
                header: ViewLabel::plain("TarR/M".to_owned()),
                width: ViewSizeHint::Fixed(9),
                alignment: ViewAlign::Right,
            },
        ],
        LedgerTableLayout::Compact34 => vec![
            ViewTableColumn {
                header: ViewLabel::plain("Seat".to_owned()),
                width: ViewSizeHint::Fixed(4),
                alignment: ViewAlign::Left,
            },
            ViewTableColumn {
                header: ViewLabel::plain("CRED".to_owned()),
                width: ViewSizeHint::Fixed(7),
                alignment: ViewAlign::Right,
            },
            ViewTableColumn {
                header: ViewLabel::plain("Debt".to_owned()),
                width: ViewSizeHint::Fixed(6),
                alignment: ViewAlign::Right,
            },
            ViewTableColumn {
                header: ViewLabel::plain("Tier".to_owned()),
                width: ViewSizeHint::Fixed(4),
                alignment: ViewAlign::Left,
            },
            ViewTableColumn {
                header: ViewLabel::plain("Tar(R/M)".to_owned()),
                width: ViewSizeHint::Fixed(9),
                alignment: ViewAlign::Right,
            },
        ],
        LedgerTableLayout::FullStacked => vec![
            ViewTableColumn {
                header: ViewLabel::plain("Seat".to_owned()),
                width: ViewSizeHint::Fixed(4),
                alignment: ViewAlign::Left,
            },
            ViewTableColumn {
                header: ViewLabel::plain("Role".to_owned()),
                width: ViewSizeHint::Fixed(4),
                alignment: ViewAlign::Left,
            },
            ViewTableColumn {
                header: ViewLabel::plain("CRED".to_owned()),
                width: ViewSizeHint::Fixed(8),
                alignment: ViewAlign::Right,
            },
            ViewTableColumn {
                header: ViewLabel::plain("Debt".to_owned()),
                width: ViewSizeHint::Fixed(7),
                alignment: ViewAlign::Right,
            },
            ViewTableColumn {
                header: ViewLabel::plain("Tier".to_owned()),
                width: ViewSizeHint::Fixed(4),
                alignment: ViewAlign::Left,
            },
            ViewTableColumn {
                header: ViewLabel::plain("Eq".to_owned()),
                width: ViewSizeHint::Fixed(7),
                alignment: ViewAlign::Right,
            },
            ViewTableColumn {
                header: ViewLabel::plain("Tar(R/M)".to_owned()),
                width: ViewSizeHint::Fixed(9),
                alignment: ViewAlign::Right,
            },
            ViewTableColumn {
                header: ViewLabel::plain("Def/Br/Frz".to_owned()),
                width: ViewSizeHint::Min(10),
                alignment: ViewAlign::Right,
            },
        ],
    };
    let table_rows: Vec<ViewTableRow> = obs
        .seats
        .iter()
        .map(|s| build_seat_table_row(s, table_layout))
        .collect();
    let table_len = table_rows.len();
    let rows = public_ledger_body_rows(obs, state.plain_codes, commodities, table_layout);
    let total = table_len + rows.len();
    let offset = state.public_scroll.min(total.saturating_sub(1));
    let ledger_sel = state.selection.ledger_row_idx.min(total.saturating_sub(1));
    let table = ViewTable {
        columns,
        rows: table_rows,
        selected_row: (ledger_sel < table_len).then_some(ledger_sel),
    };

    ViewPanel {
        title: Some(ViewLabel::styled(
            "Public Table & Ledger Mirror".to_owned(),
            ViewStyle {
                fg: ViewColor::Green,
                bg: ViewColor::Reset,
                bold: state.selection.focus_zone == FocusZone::PublicLedger,
                ..ViewStyle::plain()
            },
        )),
        alignment: ViewAlign::Left,
        bordered: true,
        tabs: None,
        table: Some(table),
        rows,
        selected_row: ledger_sel.checked_sub(table_len),
        scroll: Some(ViewScroll { offset, total }),
    }
}

pub(crate) fn public_ledger_body_rows(
    obs: &SeatObservation,
    plain_codes: bool,
    commodities: Option<&CommodityDisplayMap>,
    table_layout: LedgerTableLayout,
) -> Vec<ViewRow> {
    let mut rows = Vec::new();
    if table_layout != LedgerTableLayout::FullStacked {
        append_compact_standing_rows(obs, &mut rows);
    }
    append_ledger_inventory_rows(obs, plain_codes, commodities, &mut rows);
    append_ledger_book_rows(obs, plain_codes, commodities, &mut rows);
    rows
}

pub(crate) fn public_ledger_total_rows(obs: &SeatObservation, viewport: ViewportSize) -> usize {
    obs.seats.len()
        + public_ledger_body_rows(obs, false, None, LedgerTableLayout::for_viewport(viewport)).len()
}

fn append_compact_standing_rows(obs: &SeatObservation, rows: &mut Vec<ViewRow>) {
    rows.push(ViewRow::from_plain("--- Standing & Legend ---".to_owned()));
    rows.push(ViewRow::from_plain("Legend: k=1e3 M=1e6 +=rem".to_owned()));
    for s in &obs.seats {
        let role = match s.identity {
            SeatIdentity::Human => "HUM",
            SeatIdentity::AiSyndicate => "BOT",
        };
        let frz = if s.wash_frozen_until_turn > 0 {
            format!("T{}", s.wash_frozen_until_turn)
        } else {
            "-".to_owned()
        };
        rows.push(ViewRow::from_plain(format!(
            "S{} {} Eq:{} D/B/F:{}/{}/{}",
            s.seat, role, s.project_equity, s.hard_credit_defaults, s.broken_promises, frz
        )));
        rows.push(ViewRow::from_plain(format!(
            "  C:{} D:{} T:{}/{}",
            s.cred, s.cred_debt, s.raw_tariff_bps, s.refined_tariff_bps
        )));
    }
}

fn build_seat_table_row(s: &SeatState, table_layout: LedgerTableLayout) -> ViewTableRow {
    let role = match s.identity {
        SeatIdentity::Human => "HUM",
        SeatIdentity::AiSyndicate => "BOT",
    };
    let tier = match s.credit_tier {
        CreditTier::Aaa => "Aaa",
        CreditTier::Bbb => "Bbb",
        CreditTier::B => "B",
        CreditTier::D => "D",
    };
    let frz = if s.wash_frozen_until_turn > 0 {
        format!("T{}", s.wash_frozen_until_turn)
    } else {
        "-".to_owned()
    };
    let cells = match table_layout {
        LedgerTableLayout::Compact28 => vec![
            ViewTableCell::plain(format!("S{}", s.seat)),
            ViewTableCell::aligned(
                format_ledger_units(s.cred, 6),
                ViewAlign::Right,
                ViewStyle::plain(),
            ),
            ViewTableCell::aligned(
                format_ledger_units(s.cred_debt, 4),
                ViewAlign::Right,
                ViewStyle::plain(),
            ),
            ViewTableCell::plain(tier.to_owned()),
            ViewTableCell::aligned(
                format_tariff_pair(s.raw_tariff_bps, s.refined_tariff_bps, 9),
                ViewAlign::Right,
                ViewStyle::plain(),
            ),
        ],
        LedgerTableLayout::Compact34 => vec![
            ViewTableCell::plain(format!("S{}", s.seat)),
            ViewTableCell::aligned(
                format_ledger_units(s.cred, 7),
                ViewAlign::Right,
                ViewStyle::plain(),
            ),
            ViewTableCell::aligned(
                format_ledger_units(s.cred_debt, 6),
                ViewAlign::Right,
                ViewStyle::plain(),
            ),
            ViewTableCell::plain(tier.to_owned()),
            ViewTableCell::aligned(
                format_tariff_pair(s.raw_tariff_bps, s.refined_tariff_bps, 9),
                ViewAlign::Right,
                ViewStyle::plain(),
            ),
        ],
        LedgerTableLayout::FullStacked => vec![
            ViewTableCell::plain(format!("S{}", s.seat)),
            ViewTableCell::plain(role.to_owned()),
            ViewTableCell::aligned(
                format_ledger_units(s.cred, 8),
                ViewAlign::Right,
                ViewStyle::plain(),
            ),
            ViewTableCell::aligned(
                format_ledger_units(s.cred_debt, 7),
                ViewAlign::Right,
                ViewStyle::plain(),
            ),
            ViewTableCell::plain(tier.to_owned()),
            ViewTableCell::aligned(
                format_ledger_units(s.project_equity, 7),
                ViewAlign::Right,
                ViewStyle::plain(),
            ),
            ViewTableCell::aligned(
                format_tariff_pair(s.raw_tariff_bps, s.refined_tariff_bps, 9),
                ViewAlign::Right,
                ViewStyle::plain(),
            ),
            ViewTableCell::aligned(
                format!("{}/{}/{}", s.hard_credit_defaults, s.broken_promises, frz),
                ViewAlign::Right,
                ViewStyle::plain(),
            ),
        ],
    };
    ViewTableRow {
        cells,
        style: ViewStyle::plain(),
    }
}

fn append_ledger_inventory_rows(
    obs: &SeatObservation,
    plain_codes: bool,
    commodities: Option<&CommodityDisplayMap>,
    rows: &mut Vec<ViewRow>,
) {
    rows.push(ViewRow::from_plain("--- Commodity Balances ---".to_owned()));
    for s in &obs.seats {
        let mut parts = Vec::new();
        for (idx, &amt) in s.raw.iter().enumerate() {
            if amt > 0 {
                let g = u8::try_from(idx).unwrap_or(0);
                parts.push(format!(
                    "{}:{}",
                    format_asset_label(AssetId::Raw(g), commodities, plain_codes),
                    amt
                ));
            }
        }
        for (idx, &amt) in s.refined.iter().enumerate() {
            if amt > 0 {
                let g = u8::try_from(idx).unwrap_or(0);
                parts.push(format!(
                    "{}:{}",
                    format_asset_label(AssetId::Refined(g), commodities, plain_codes),
                    amt
                ));
            }
        }
        let regent = obs
            .spec
            .seats
            .get(usize::from(s.seat))
            .and_then(|cfg| cfg.regent.as_ref())
            .map_or("NoRegent", |_| "Regent");
        let joined = if parts.is_empty() {
            "empty stocks".to_owned()
        } else {
            parts.join(", ")
        };
        rows.push(ViewRow::from_plain(format!(
            "S{} ({}) [{}]: {}",
            s.seat,
            guild_name_for_seat(s.seat),
            regent,
            joined
        )));
    }
}

fn append_ledger_book_rows(
    obs: &SeatObservation,
    plain_codes: bool,
    commodities: Option<&CommodityDisplayMap>,
    rows: &mut Vec<ViewRow>,
) {
    rows.push(ViewRow::from_plain(format!(
        "--- DEX Offers ({}) ---",
        obs.offers.len()
    )));
    if obs.offers.is_empty() {
        rows.push(ViewRow::from_plain("  (no resting offers)".to_owned()));
    } else {
        for offer in obs.offers.values() {
            rows.push(ViewRow::from_plain(format_offer_line(
                offer,
                obs.cancel_locked,
                plain_codes,
                commodities,
            )));
        }
    }

    rows.push(ViewRow::from_plain(format!(
        "--- Checks ({}) | Escrows ({}) | Dual ({}) ---",
        obs.checks.len(),
        obs.escrows.len(),
        obs.dual_escrows.len()
    )));
    for chk in obs.checks.values() {
        rows.push(ViewRow::from_plain(format_check_line(
            chk,
            plain_codes,
            commodities,
        )));
    }
    for esc in obs.escrows.values() {
        rows.push(ViewRow::from_plain(format_escrow_line(
            esc,
            plain_codes,
            commodities,
        )));
    }
    for dual in obs.dual_escrows.values() {
        rows.push(ViewRow::from_plain(format_dual_escrow_line(
            dual,
            plain_codes,
            commodities,
        )));
    }

    rows.push(ViewRow::from_plain(format!(
        "--- Pledges ({}) | Domains ({}) | Platforms ({}) | Charters ({}) ---",
        obs.pledges.len(),
        obs.domains.len(),
        obs.council_platforms.len(),
        obs.charters.len()
    )));
    for pledge in obs.pledges.values() {
        rows.push(ViewRow::from_plain(format_pledge_line(
            pledge,
            obs.audited_pledges.contains(&pledge.pledge_id),
        )));
    }
    for dom in &obs.domains {
        rows.push(ViewRow::from_plain(format!(
            "  Domain #{} members: {:?}",
            dom.domain_id, dom.members
        )));
    }
    for (&seat, plat) in &obs.council_platforms {
        rows.push(ViewRow::from_plain(format!(
            "  Platform S{}: tariffs {:?} admit {:?} revoke {:?}",
            seat, plat.proposed_tariffs_bps, plat.cartel_admissions, plat.cartel_revocations
        )));
    }
    for ch in &obs.charters {
        rows.push(ViewRow::from_plain(format_charter_line(ch)));
    }
}

fn format_offer_line(
    offer: &CredOffer,
    cancel_locked: bool,
    plain_codes: bool,
    commodities: Option<&CommodityDisplayMap>,
) -> String {
    let book = offer
        .domain_id
        .map_or_else(|| "Open".to_owned(), |d| format!("Dom#{d}"));
    let hyb = if offer.is_hybrid { " [tfHybrid]" } else { "" };
    let lock = if cancel_locked {
        " [CANCEL LOCKED]"
    } else {
        ""
    };
    format!(
        "  Offer #{} S{} {:?} {} x{} @{} [{}]{}{}",
        offer.offer_id,
        offer.maker,
        offer.side,
        format_asset_label(offer.good, commodities, plain_codes),
        offer.remaining_units,
        offer.price_per_unit_cred,
        book,
        hyb,
        lock
    )
}

fn format_check_line(
    chk: &CreditCheck,
    plain_codes: bool,
    commodities: Option<&CommodityDisplayMap>,
) -> String {
    format!(
        "  Check #{} S{}->S{} {} max:{} min:{} T{}..T{} ({:?})",
        chk.check_id,
        chk.drawer,
        chk.payee,
        format_asset_label(chk.asset, commodities, plain_codes),
        chk.send_max,
        chk.deliver_min,
        chk.due_turn,
        chk.expiration_turn,
        chk.status
    )
}

fn format_escrow_line(
    esc: &TokenEscrowVault,
    plain_codes: bool,
    commodities: Option<&CommodityDisplayMap>,
) -> String {
    let bond_desc = esc.bond.as_ref().map_or_else(
        || "no-bond".to_owned(),
        |b| {
            format!(
                "bond S{}->S{} {} >= {} T{} ({:?})",
                b.obligor,
                b.beneficiary,
                format_asset_label(b.good, commodities, plain_codes),
                b.required_units,
                b.deadline_turn,
                b.mode
            )
        },
    );
    format!(
        "  Escrow #{} S{}->S{} {} CRED (cancel T{}) {} [{:?}]",
        esc.escrow_id,
        esc.sender,
        esc.destination,
        esc.amount_cred,
        esc.cancel_after_turn,
        bond_desc,
        esc.status
    )
}

fn format_dual_escrow_line(
    dual: &DualEscrowVault,
    plain_codes: bool,
    commodities: Option<&CommodityDisplayMap>,
) -> String {
    format!(
        "  DualEscrow #{} Buyer S{} Seller S{} {} x{} pay:{} bonds:{}/{} T{} [{:?}]",
        dual.escrow_id,
        dual.buyer,
        dual.seller,
        format_asset_label(dual.good, commodities, plain_codes),
        dual.required_units,
        dual.payment_cred,
        dual.buyer_bond_cred,
        dual.seller_bond_cred,
        dual.deadline_turn,
        dual.status
    )
}

fn format_pledge_line(pledge: &IntentPledge, resolved: bool) -> String {
    let tag = if resolved { "audited" } else { "active" };
    let target = match pledge.target {
        IntentTarget::VoteForSeat {
            office_turn,
            candidate,
        } => format!("Vote S{candidate} on T{office_turn}"),
        IntentTarget::PreserveCartelMembership {
            office_turn,
            domain_id,
            beneficiary,
        } => format!("Keep S{beneficiary} in Dom#{domain_id} on T{office_turn}"),
        IntentTarget::RevealPreimage { escrow_id, .. } => format!("Reveal Esc#{escrow_id}"),
    };
    format!(
        "  Pledge #{} S{}->S{} {} w:{} [{}]",
        pledge.pledge_id, pledge.promisor, pledge.counterparty, target, pledge.honor_weight, tag
    )
}

fn format_charter_line(ch: &ActiveCharter) -> String {
    format!(
        "  Charter {:?}: holder S{} (until T{})",
        ch.role, ch.holder, ch.expires_at_turn
    )
}

fn styled_tab_item(text: &str, enabled: bool, active: bool) -> ViewTabItem {
    let style = if active {
        ViewStyle::plain().with_fg(ViewColor::Cyan).bold()
    } else if !enabled {
        ViewStyle::plain().with_fg(ViewColor::DarkGray)
    } else {
        ViewStyle::plain()
    };
    ViewTabItem {
        label: ViewLabel::styled(text.to_owned(), style),
        enabled,
    }
}

fn build_center_tabs_panel(
    obs: &SeatObservation,
    state: &SeatUiState,
    commodities: Option<&CommodityDisplayMap>,
) -> ViewPanel {
    let active_idx = state.active_tab.index();
    let tab_bar = ViewTabBar {
        tabs: vec![
            styled_tab_item("1: Market & Contracts", true, active_idx == 0),
            styled_tab_item("2: Workshop, Deals & Politics", true, active_idx == 1),
            styled_tab_item("3: What-If Sandbox", true, active_idx == 2),
            styled_tab_item("4: Diplomatic Desk (Disabled)", false, active_idx == 3),
        ],
        active_index: active_idx,
    };

    let mut rows = if state.resume_digest_open {
        let mut combined = build_resume_digest_rows(obs, state.plain_codes, commodities);
        combined.push(ViewRow::from_plain(
            "----------------------------------------".to_owned(),
        ));
        combined.extend(center_tab_rows(obs, state, commodities));
        combined
    } else {
        center_tab_rows(obs, state, commodities)
    };
    if rows.is_empty() {
        rows.push(ViewRow::from_plain(String::new()));
    }
    let total = rows.len();
    let offset = state.center_scroll.min(total.saturating_sub(1));

    ViewPanel {
        title: Some(ViewLabel::styled(
            "Action & Order Composer".to_owned(),
            ViewStyle {
                fg: ViewColor::Magenta,
                bg: ViewColor::Reset,
                bold: state.selection.focus_zone == FocusZone::CenterTabs,
                ..ViewStyle::plain()
            },
        )),
        alignment: ViewAlign::Left,
        bordered: true,
        tabs: Some(tab_bar),
        table: None,
        rows,
        selected_row: None,
        scroll: Some(ViewScroll { offset, total }),
    }
}

pub(crate) fn center_tab_rows(
    obs: &SeatObservation,
    state: &SeatUiState,
    commodities: Option<&CommodityDisplayMap>,
) -> Vec<ViewRow> {
    match state.active_tab {
        CenterTab::MarketAndContracts => build_market_tab_rows(obs, state, commodities),
        CenterTab::WorkshopDealsPolitics => build_bundle_tab_rows(obs, state, commodities),
        CenterTab::WhatIfSandbox => build_what_if_tab_rows(obs, state, commodities),
    }
}

pub(crate) fn center_tab_total_rows(obs: &SeatObservation, state: &SeatUiState) -> usize {
    center_tab_rows(obs, state, None).len()
}

fn build_market_tab_rows(
    obs: &SeatObservation,
    state: &SeatUiState,
    commodities: Option<&CommodityDisplayMap>,
) -> Vec<ViewRow> {
    let selected_idx = state.selection.market_kind_idx % ALL_MARKET_KINDS.len();
    let kind = ALL_MARKET_KINDS[selected_idx];
    let n = obs.spec.seat_count.max(1);
    let mut rows = Vec::new();
    let kinds_bar: Vec<String> = ALL_MARKET_KINDS
        .iter()
        .enumerate()
        .map(|(i, k)| {
            let locked = *k == MarketActionKind::CancelOffer && obs.cancel_locked;
            let suffix = if locked { " [LOCKED]" } else { "" };
            if i == selected_idx {
                format!("[{}{suffix}]", k.label())
            } else {
                format!("{}{suffix}", k.label())
            }
        })
        .collect();
    rows.push(ViewRow::from_plain(format!(
        "Actions ([/]): {}",
        kinds_bar.join(" | ")
    )));
    if obs.cancel_locked && kind == MarketActionKind::CancelOffer {
        rows.push(ViewRow::from_plain(
            "CancelOffer is BLOCKED because cancel lock is engaged on this turn.".to_owned(),
        ));
    }

    let good_label = format_asset_label(
        preview_selected_asset(&state.market_fields, n),
        commodities,
        state.plain_codes,
    );
    let field_lines = market_field_lines(obs, state, kind, &good_label, commodities);
    let active_f = state.selection.market_field_idx % market_field_count(kind);
    for (idx, line) in field_lines.into_iter().enumerate() {
        let mark = if idx == active_f { ">" } else { " " };
        rows.push(ViewRow::from_plain(format!("{mark} {line}")));
    }

    let built = state.market_fields.build_action(obs, kind);
    rows.push(ViewRow::from_plain(format!(
        "Preview Action (Enter Submit): {}",
        format_market_action(&built, state.plain_codes, commodities)
    )));
    if state.composer_edit_active || !state.composer_draft.is_empty() {
        rows.push(ViewRow::from_plain(format!(
            "Draft Text (e to edit): {}{}",
            state.composer_draft,
            if state.composer_edit_active { "_" } else { "" }
        )));
    }
    rows
}

fn market_field_lines(
    obs: &SeatObservation,
    state: &SeatUiState,
    kind: MarketActionKind,
    good_label: &str,
    commodities: Option<&CommodityDisplayMap>,
) -> Vec<String> {
    let f = &state.market_fields;
    let n = obs.spec.seat_count.max(1);
    match kind {
        MarketActionKind::PostOffer => vec![
            format!("Side: {}", if f.side_buy { "Buy" } else { "Sell" }),
            format!("Kind: {}", if f.good_kind_raw { "Raw" } else { "Refined" }),
            format!("Good: {good_label}"),
            format!("Amount: {}", f.amount),
            format!("Price (CRED/1000): {}", f.price_per_unit_cred),
            format!("Domain ID: {:?}", f.domain_id),
            format!("tfHybrid: {}", f.is_hybrid),
        ],
        MarketActionKind::CancelOffer => vec![format!("Offer ID: {}", f.target_id)],
        MarketActionKind::CancelCheck => vec![format!("Check ID: {}", f.target_id)],
        MarketActionKind::CancelExpiredEscrow => vec![format!("Escrow ID: {}", f.target_id)],
        MarketActionKind::CreateCheck => vec![
            format!("Payee: S{}", f.counterparty % n),
            format!("Asset Mode: {}", f.check_asset_mode % 3),
            format!(
                "Asset: {}",
                format_asset_label(f.selected_check_asset(n), commodities, state.plain_codes)
            ),
            format!("Send Max: {}", f.amount),
            format!("Deliver Min: {}", f.deliver_min),
            format!("Expiration Turn (0=auto): {}", f.expiration_turn),
        ],
        MarketActionKind::CreateEscrow => vec![
            format!("Destination: S{}", f.counterparty % n),
            format!("CRED Collateral: {}", f.amount),
            format!("Preimage Condition: {}", f.has_condition),
            format!("Attach Steward Bond: {}", f.has_bond),
            format!(
                "Bond Mode: {}",
                if f.bond_buyer_dvp {
                    "BuyerDvpPayment"
                } else {
                    "SellerPerformanceBond"
                }
            ),
            format!("Bond Good {good_label} Req: {}", f.bond_required_units),
            format!("Cancel After Turn (0=auto): {}", f.cancel_after_turn),
        ],
        MarketActionKind::RecordIntentPledge => vec![
            format!("Pledge ID: {}", f.target_id.max(1)),
            format!("Counterparty: S{}", f.counterparty % n),
            format!("Target Variant: {}", f.pledge_target_kind % 3),
            format!("Office Turn: {}", f.office_turn.max(4)),
            format!("Honor Weight: {}", f.honor_weight),
        ],
        MarketActionKind::StandForOffice => vec![
            format!("Proposed Raw Tariff (bps): {}", f.platform_raw_tariff_bps),
            format!(
                "Proposed Refined Tariff (bps): {}",
                f.platform_refined_tariff_bps
            ),
            format!("Admit Domain: {:?}", f.platform_admit_domain),
            format!("Revoke Domain: {:?}", f.platform_revoke_domain),
            format!("Target Seat for Admit/Revoke: S{}", f.counterparty % n),
        ],
        MarketActionKind::FlagWashFreeze => {
            vec![format!("Target Seat: S{}", f.counterparty % n)]
        }
        MarketActionKind::TownImport => vec![
            format!("Kind: {}", if f.good_kind_raw { "Raw" } else { "Refined" }),
            format!("Good: {good_label}"),
            format!("Amount: {}", f.amount),
        ],
    }
}

fn format_market_action(
    action: &MarketAction,
    plain_codes: bool,
    commodities: Option<&CommodityDisplayMap>,
) -> String {
    match action {
        MarketAction::PostOffer { seat, order } => format!(
            "PostOffer S{} {:?} {} x{} @{}",
            seat,
            order.side,
            format_asset_label(order.good, commodities, plain_codes),
            order.amount,
            order.price_per_unit_cred
        ),
        MarketAction::CancelOffer { seat, offer_id } => {
            format!("CancelOffer S{} #{}", seat, offer_id)
        }
        MarketAction::CreateCheck { seat, check } => format!(
            "CreateCheck S{}->S{} {} max:{} min:{}",
            seat,
            check.payee,
            format_asset_label(check.asset, commodities, plain_codes),
            check.send_max,
            check.deliver_min
        ),
        MarketAction::CancelCheck { seat, check_id } => {
            format!("CancelCheck S{} #{}", seat, check_id)
        }
        MarketAction::CreateEscrow { seat, escrow } => format!(
            "CreateEscrow S{}->S{} {} CRED (bond:{})",
            seat,
            escrow.destination,
            escrow.amount,
            escrow.bond.is_some()
        ),
        MarketAction::CancelExpiredEscrow { seat, escrow_id } => {
            format!("CancelExpiredEscrow S{} #{}", seat, escrow_id)
        }
        MarketAction::RecordIntentPledge { seat, pledge } => format!(
            "RecordIntentPledge S{} #{} -> S{} (w:{})",
            seat, pledge.pledge_id, pledge.counterparty, pledge.honor_weight
        ),
        MarketAction::FlagWashFreeze {
            inspector,
            target_seat,
        } => format!("FlagWashFreeze S{} -> S{}", inspector, target_seat),
        MarketAction::StandForOffice { seat, platform } => format!(
            "StandForOffice S{} (tariffs: {})",
            seat,
            platform.proposed_tariffs_bps.len()
        ),
        MarketAction::TownImport { seat, good, amount } => format!(
            "TownImport S{} {} x{}",
            seat,
            format_asset_label(*good, commodities, plain_codes),
            amount
        ),
    }
}

fn build_bundle_tab_rows(
    obs: &SeatObservation,
    state: &SeatUiState,
    commodities: Option<&CommodityDisplayMap>,
) -> Vec<ViewRow> {
    let sections = visible_bundle_sections(obs.turn);
    let sec_idx = state.selection.bundle_section_idx % sections.len().max(1);
    let section = sections[sec_idx];

    let bar: Vec<String> = sections
        .iter()
        .enumerate()
        .map(|(i, s)| {
            if i == sec_idx {
                format!("[{}]", s.label())
            } else {
                s.label().to_owned()
            }
        })
        .collect();
    let mut rows = vec![ViewRow::from_plain(format!(
        "Sections ([/]): {}",
        bar.join(" | ")
    ))];

    let active_f = state.selection.bundle_field_idx % bundle_field_count(section);
    for (idx, line) in bundle_field_lines(obs, state, section, commodities)
        .into_iter()
        .enumerate()
    {
        let mark = if idx == active_f { ">" } else { " " };
        rows.push(ViewRow::from_plain(format!("{mark} {line}")));
    }

    rows.push(ViewRow::from_plain(
        "Keys: Enter Stage Section | Del/Bksp Clear Section | X Clear All".to_owned(),
    ));

    let b = &state.staged_bundle;
    rows.push(ViewRow::from_plain(format!(
        "Staged: conv={} proj={} swap={} deliv={}",
        b.converter_runs.len(),
        b.project_deliveries.len(),
        b.atomic_swaps.len(),
        b.contract_deliveries.len(),
    )));
    rows.push(ViewRow::from_plain(format!(
        "  dual={} debt_repay={} fin={} chk={}",
        b.dual_escrows.len(),
        b.debt_repayment_cred,
        b.escrow_finishes.len(),
        b.check_cashes.len()
    )));
    if obs.turn > 0 && obs.turn.is_multiple_of(4) {
        rows.push(ViewRow::from_plain(format!(
            "  Council Vote (Turn 4n): {:?}",
            b.council_vote
        )));
    }
    if obs.turn > 0 && obs.turn.is_multiple_of(6) {
        rows.push(ViewRow::from_plain(format!(
            "  Charter Bids (Turn 6n): {:?}",
            b.charter_bids
        )));
    }
    if state.composer_edit_active || !state.composer_draft.is_empty() {
        rows.push(ViewRow::from_plain(format!(
            "Draft Text (e to edit): {}{}",
            state.composer_draft,
            if state.composer_edit_active { "_" } else { "" }
        )));
    }
    rows
}

fn bundle_field_lines(
    obs: &SeatObservation,
    state: &SeatUiState,
    section: BundleSection,
    commodities: Option<&CommodityDisplayMap>,
) -> Vec<String> {
    let f = &state.bundle_fields;
    let n = obs.spec.seat_count.max(1);
    let plain = state.plain_codes;
    let good_of = |is_raw: bool, idx: u8| -> String {
        let g = idx % n;
        let asset = if is_raw {
            AssetId::Raw(g)
        } else {
            AssetId::Refined(g)
        };
        format_asset_label(asset, commodities, plain)
    };

    match section {
        BundleSection::ConverterRuns => vec![
            format!("Recipe Index: {}", f.recipe_idx),
            format!("Runs: {}", f.runs),
        ],
        BundleSection::ProjectDeliveries => vec![
            format!("Project ID: {}", f.project_id),
            format!("Batches: {}", f.batches),
        ],
        BundleSection::AtomicSwaps => vec![
            format!("Swap ID: {}", f.swap_id),
            format!("Taker Seat: S{}", f.swap_taker % n),
            format!("Maker Raw?: {}", f.swap_maker_raw),
            format!(
                "Maker Asset: {}",
                good_of(f.swap_maker_raw, f.swap_maker_good)
            ),
            format!("Maker Amount: {}", f.swap_maker_amount),
            format!(
                "Taker Asset: {}",
                good_of(f.swap_taker_raw, f.swap_taker_good)
            ),
            format!("Taker Amount: {}", f.swap_taker_amount),
        ],
        BundleSection::ContractDeliveries => vec![
            format!("Counterparty: S{}", f.cd_counterparty % n),
            format!("Raw?: {}", f.cd_raw),
            format!("Good: {}", good_of(f.cd_raw, f.cd_good)),
            format!("Gross Amount: {}", f.cd_amount),
            format!("Dual Escrow ID: {:?}", f.cd_dual_escrow_id),
        ],
        BundleSection::DualEscrows => vec![
            format!("Proposal ID: {}", f.dual_id),
            format!("Seller Seat: S{}", f.dual_seller % n),
            format!("Raw?: {}", f.dual_raw),
            format!("Good: {}", good_of(f.dual_raw, f.dual_good)),
            format!("Required Units: {}", f.dual_required_units),
            format!("Payment CRED: {}", f.dual_payment_cred),
            format!("Buyer Bond CRED: {}", f.dual_buyer_bond_cred),
            format!("Seller Bond CRED: {}", f.dual_seller_bond_cred),
            format!("Deadline Turn (0=auto): {}", f.dual_deadline_turn),
        ],
        BundleSection::DebtRepayment => {
            vec![format!("Debt Repayment CRED: {}", f.debt_repayment_cred)]
        }
        BundleSection::EscrowFinishes => vec![
            format!("Escrow ID: {}", f.escrow_finish_id),
            format!("Supply Preimage?: {}", f.escrow_use_preimage),
            format!("Preimage Seed Byte: 0x{:02x}", f.escrow_preimage_byte),
        ],
        BundleSection::CheckCashes => {
            vec![format!("Present Check ID: {}", f.check_cash_id)]
        }
        BundleSection::CouncilVote => vec![
            format!("Trade Minister Vote: S{}", f.vote_tm % n),
            format!("Cartel Chair Vote: S{}", f.vote_cc % n),
        ],
        BundleSection::CharterBids => vec![
            format!("Charter Role: {:?}", f.charter_role()),
            format!("Bid CRED: {}", f.charter_bid_cred),
        ],
    }
}

fn build_what_if_tab_rows(
    obs: &SeatObservation,
    state: &SeatUiState,
    commodities: Option<&CommodityDisplayMap>,
) -> Vec<ViewRow> {
    let mut rows = Vec::new();
    let req = build_what_if_request(obs, state);
    let sel = state.selection.what_if_row_idx;
    let mark = |idx: usize| -> &'static str { if idx == sel { ">" } else { " " } };

    rows.push(ViewRow::from_plain(format!(
        "{} Tie Mode: {:?} (Space/Arrows toggle, Enter/p run preview_what_if)",
        mark(0),
        req.tie_mode
    )));
    rows.push(ViewRow::from_plain(format!(
        "{} Include Draft Market Action: {}",
        mark(1),
        state.include_draft_market_action
    )));
    rows.push(ViewRow::from_plain(format!(
        "{} Assume Counterparties Co-Sign Staged Swaps ({})",
        mark(2),
        state.assume_cosign_swaps
    )));
    rows.push(ViewRow::from_plain(format!(
        "{} Assume Counterparties Co-Sign Staged Dual Escrows ({})",
        mark(3),
        state.assume_cosign_dual_escrows
    )));
    rows.push(ViewRow::from_plain(format!(
        "{} Assume Peer Deliveries on Open Dual Escrows ({})",
        mark(4),
        state.assume_peer_deliveries
    )));

    let mut row_cursor = 5_usize;
    for offer in obs.offers.values().filter(|o| o.maker != obs.seat) {
        let on = state.assumed_cancelled_offer_ids.contains(&offer.offer_id);
        let lock = if obs.cancel_locked {
            " [ignored: cancel locked]"
        } else {
            ""
        };
        rows.push(ViewRow::from_plain(format!(
            "{} Assume Cancel Peer Offer #{} (S{} {} x{} @{}): {}{}",
            mark(row_cursor),
            offer.offer_id,
            offer.maker,
            format_asset_label(offer.good, commodities, state.plain_codes),
            offer.remaining_units,
            offer.price_per_unit_cred,
            on,
            lock
        )));
        row_cursor += 1;
    }
    for chk in obs.checks.values() {
        let on = state.assumed_bounced_check_ids.contains(&chk.check_id);
        rows.push(ViewRow::from_plain(format!(
            "{} Assume Bounce Check #{} (S{}->S{}): {}",
            mark(row_cursor),
            chk.check_id,
            chk.drawer,
            chk.payee,
            on
        )));
        row_cursor += 1;
    }

    rows.push(ViewRow::from_plain(format!(
        "Caveat: {HYPOTHETICAL_CLOSE_MARK_CAVEAT}"
    )));
    append_what_if_preview_result(state, commodities, &mut rows);
    rows
}

fn append_what_if_preview_result(
    state: &SeatUiState,
    commodities: Option<&CommodityDisplayMap>,
    rows: &mut Vec<ViewRow>,
) {
    let Some(preview) = state.last_what_if.as_ref() else {
        rows.push(ViewRow::from_plain(
            "No What-If preview computed yet — press [p] or [Enter] on Tab 3.".to_owned(),
        ));
        return;
    };

    let cred_delta = preview.post_sweep_seat_state.cred - preview.pre_seat_state.cred;
    let debt_delta = preview.post_sweep_seat_state.cred_debt - preview.pre_seat_state.cred_debt;
    let eq_delta =
        preview.post_sweep_seat_state.project_equity - preview.pre_seat_state.project_equity;
    rows.push(ViewRow::from_spans(vec![
        ViewSpan::plain(format!(
            "Preview T{} S{}: Pre CRED {} -> Post-Mkt {} -> Post-Sweep {} (",
            preview.turn,
            preview.seat,
            preview.pre_seat_state.cred,
            preview.post_market_seat_state.cred,
            preview.post_sweep_seat_state.cred,
        )),
        ViewSpan::styled(
            format_signed_delta(cred_delta),
            signed_delta_style(cred_delta, true),
        ),
        ViewSpan::plain(format!(
            ") (Debt {} ",
            preview.post_sweep_seat_state.cred_debt
        )),
        ViewSpan::styled(
            format_signed_delta(debt_delta),
            signed_delta_style(debt_delta, false),
        ),
        ViewSpan::plain(format!(
            ", Eq {} ",
            preview.post_sweep_seat_state.project_equity
        )),
        ViewSpan::styled(
            format_signed_delta(eq_delta),
            signed_delta_style(eq_delta, true),
        ),
        ViewSpan::plain(")".to_owned()),
    ]));
    rows.push(ViewRow::from_plain(format!(
        "  Upkeep: req {} paid {} interest {} shortfall {} debt_repaid {} | Spoiled: {}",
        preview.upkeep.upkeep_required,
        preview.upkeep.paid_with_refined,
        preview.upkeep.debt_interest_accrued,
        preview.upkeep.new_shortfall_debt,
        preview.upkeep.debt_repaid_cred,
        preview.spoilage.perishable_spoiled_units
    )));
    if !preview.refused_market_actions.is_empty() {
        for r in &preview.refused_market_actions {
            let safe_msg = scrub_secret_text(&r.message);
            rows.push(ViewRow::from_plain(format!(
                "  Refused Action #{}: [{}]",
                r.action_index, r.error_code
            )));
            rows.push(ViewRow::from_plain(format!("    {safe_msg}")));
        }
    }
    rows.push(ViewRow::from_plain(format!(
        "  Orders: conv={} proj={} swaps={} checks={} escrows={} fills={} batch_trades={}",
        preview.converter_orders.len(),
        preview.project_orders.len(),
        preview.swap_orders.len(),
        preview.check_orders.len(),
        preview.escrow_orders.len(),
        preview.public_fills.len(),
        preview.step0_to_7_receipt.batch_trades_cleared
    )));
    for hm in &preview.post_sweep_predicates.season_end.holdings {
        if hm.units > 0 {
            rows.push(ViewRow::from_plain(format!(
                "  Close Mark {}: {}u => {} (bids:{}, floor:{})",
                format_asset_label(hm.good, commodities, state.plain_codes),
                hm.units,
                hm.marked_value,
                hm.units_marked_by_bids,
                hm.units_at_floor
            )));
        }
    }
}

fn build_private_drawer_panel(
    obs: &SeatObservation,
    state: &SeatUiState,
    commodities: Option<&CommodityDisplayMap>,
) -> ViewPanel {
    let rows = private_drawer_rows(obs, state, commodities);
    let total = rows.len();
    let offset = state.drawer_scroll.min(total.saturating_sub(1));

    ViewPanel {
        title: Some(ViewLabel::styled(
            format!("Seat {} Private Vault & Diagnostics", obs.seat),
            ViewStyle {
                fg: ViewColor::Yellow,
                bg: ViewColor::Reset,
                bold: state.selection.focus_zone == FocusZone::PrivateDrawer,
                ..ViewStyle::plain()
            },
        )),
        alignment: ViewAlign::Left,
        bordered: true,
        tabs: None,
        table: None,
        rows,
        selected_row: Some(state.selection.drawer_row_idx.min(total.saturating_sub(1))),
        scroll: Some(ViewScroll { offset, total }),
    }
}

pub(crate) fn private_drawer_rows(
    obs: &SeatObservation,
    state: &SeatUiState,
    commodities: Option<&CommodityDisplayMap>,
) -> Vec<ViewRow> {
    let mut rows = Vec::new();
    match obs.own_mandate() {
        Some(m) => {
            rows.push(ViewRow::from_plain("Private Mandate:".to_owned()));
            rows.push(ViewRow::from_plain(format!(
                "  {}",
                format_mandate_kind(m.kind)
            )));
            rows.push(ViewRow::from_plain(format!(
                "  Bonus: +{} eq",
                m.bonus_equity
            )));
        }
        None => rows.push(ViewRow::from_plain("Mandate: none".to_owned())),
    }

    rows.push(ViewRow::from_plain("- OracleAuditor Hints -".to_owned()));
    if obs.auditor_hints.is_empty() {
        rows.push(ViewRow::from_plain("  none".to_owned()));
    } else {
        for hint in &obs.auditor_hints {
            rows.push(ViewRow::from_plain(format!(
                "  T{}->T{}->T{} S{}",
                hint.hint_turn, hint.forecast_turn, hint.realize_turn, hint.target_seat
            )));
            rows.push(ViewRow::from_plain(format!("  {:?}", hint.kind)));
        }
    }

    append_predicates_rows(obs, state, commodities, &mut rows);
    rows.push(ViewRow::from_plain(
        "--- Personal Notes (n edit) ---".to_owned(),
    ));
    rows.push(ViewRow::from_plain(format!(
        "{}{}",
        if state.notes_draft.is_empty() {
            "(empty)"
        } else {
            &state.notes_draft
        },
        if state.notes_edit_active { "_" } else { "" }
    )));
    rows
}

pub(crate) fn private_drawer_total_rows(obs: &SeatObservation, state: &SeatUiState) -> usize {
    private_drawer_rows(obs, state, None).len()
}

fn format_mandate_kind(kind: MandateKind) -> String {
    match kind {
        MandateKind::MinProjectEquity { target } => {
            format!("MinProjectEquity >={target}")
        }
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

fn append_predicates_rows(
    obs: &SeatObservation,
    state: &SeatUiState,
    commodities: Option<&CommodityDisplayMap>,
    rows: &mut Vec<ViewRow>,
) {
    let p = &obs.baseline_predicates;
    let upkeep_label = format_asset_label(
        AssetId::Refined(p.upkeep.upkeep_good),
        commodities,
        state.plain_codes,
    );
    rows.push(ViewRow::from_plain("--- State Predicates ---".to_owned()));
    rows.push(ViewRow::from_plain(format!(
        "Upkeep ({upkeep_label}): owe {} | free {} | sell {} | short {} -> debt+ {} @{}bps",
        p.upkeep.owed_units,
        p.upkeep.held_units,
        p.upkeep.held_on_own_resting_sells,
        p.upkeep.shortfall_units,
        p.upkeep.debt_added_at_sweep,
        p.upkeep.interest_bps
    )));
    rows.push(ViewRow::from_plain(format!(
        "Spoilage & Workshop: own_yield {} | spoil_total {} (sells {}) | cap_runs {}",
        p.spoilage.own_raw_yield_at_sweep,
        p.spoilage.spoil_units_total,
        p.spoilage.spoil_units_on_resting_offers,
        p.spoilage.workshop_capacity_runs_left
    )));
    for rc in &p.spoilage.recipes {
        rows.push(ViewRow::from_plain(format!(
            "  Recipe #{}: {} x{}/run cap:{} covered:{}",
            rc.recipe_idx,
            format_asset_label(
                AssetId::Refined(rc.output_good),
                commodities,
                state.plain_codes
            ),
            rc.output_units_per_batch,
            rc.capacity_runs_left,
            rc.batches_covered_by_inputs
        )));
    }
    rows.push(ViewRow::from_plain(format!(
        "Season Window: closed={} earliest={}T latest={}T | projects={} coalitions={}",
        p.season_end.season_closed,
        p.season_end.turns_until_earliest_close,
        p.season_end.turns_until_latest_close,
        p.season_end.projects.len(),
        p.coalitions.len()
    )));
    for g in &p.scarcity {
        if g.own_units > 0 || g.open_book_units_offered > 0 || g.open_book_units_bid > 0 {
            rows.push(ViewRow::from_plain(format!(
                "  Scarcity {}: own={} tbl={} ask={} bid={} tariff={}bps",
                format_asset_label(g.good, commodities, state.plain_codes),
                g.own_units,
                g.table_units_held,
                g.open_book_units_offered,
                g.open_book_units_bid,
                g.open_book_tariff_bps
            )));
        }
    }
}

fn build_chronicle_modal(
    obs: &SeatObservation,
    state: &SeatUiState,
    commodities: Option<&CommodityDisplayMap>,
) -> ViewModal {
    let rows = if obs.public_receipts.is_empty() {
        vec![ViewRow::from_plain(
            "Sweep Chronicle: no completed PublicSweepReceipt turns yet.".to_owned(),
        )]
    } else {
        let idx = state
            .chronicle_receipt_idx
            .min(obs.public_receipts.len() - 1);
        chronicle_receipt_rows(
            &obs.public_receipts[idx],
            idx,
            obs.public_receipts.len(),
            state.plain_codes,
            commodities,
        )
    };
    let total = rows.len();
    let offset = state.chronicle_scroll.min(total.saturating_sub(1));

    ViewModal {
        width_pct: 88,
        height_pct: 84,
        panel: ViewPanel {
            title: Some(ViewLabel::styled(
                "Sweep Chronicle (PublicSweepReceipt — 30 Seat-Safe Fields)".to_owned(),
                ViewStyle {
                    fg: ViewColor::Cyan,
                    bg: ViewColor::Reset,
                    bold: true,
                    ..ViewStyle::plain()
                },
            )),
            alignment: ViewAlign::Left,
            bordered: true,
            tabs: None,
            table: None,
            rows,
            selected_row: None,
            scroll: Some(ViewScroll { offset, total }),
        },
    }
}

fn chronicle_receipt_rows(
    r: &PublicSweepReceipt,
    idx: usize,
    count: usize,
    plain_codes: bool,
    commodities: Option<&CommodityDisplayMap>,
) -> Vec<ViewRow> {
    let mut rows = vec![
        ViewRow::from_plain(format!(
            "Receipt {}/{} ([/] Turn | Esc/r Close)",
            idx + 1,
            count
        )),
        ViewRow::from_plain(format!("turn: {}", r.turn)),
        ViewRow::from_plain(format!("non_reveal_seats: {:?}", r.non_reveal_seats)),
        ViewRow::from_plain(format!("regent_acted_seats: {:?}", r.regent_acted_seats)),
        ViewRow::from_plain(format!("converted_to_ai: {:?}", r.converted_to_ai)),
        ViewRow::from_plain(format!("escrows_finished: {:?}", r.escrows_finished)),
        ViewRow::from_plain(format!(
            "escrows_cancelled_or_forfeited: {:?}",
            r.escrows_cancelled_or_forfeited
        )),
        ViewRow::from_plain(format!("atomic_swaps_settled: {}", r.atomic_swaps_settled)),
        ViewRow::from_plain(format!(
            "converter_batches_run: {}",
            r.converter_batches_run
        )),
        ViewRow::from_plain(format!(
            "converter_orders_clamped: {}",
            r.converter_orders_clamped
        )),
        ViewRow::from_plain(format!("checks_cashed: {:?}", r.checks_cashed)),
        ViewRow::from_plain(format!("checks_bounced: {:?}", r.checks_bounced)),
        ViewRow::from_plain(format!("cartel_evictions: {:?}", r.cartel_evictions)),
        ViewRow::from_plain(format!("projects_completed: {}", r.projects_completed)),
        ViewRow::from_plain(format!(
            "project_orders_clamped: {}",
            r.project_orders_clamped
        )),
        ViewRow::from_plain(format!(
            "total_upkeep_shortfall_debt: {}",
            r.total_upkeep_shortfall_debt
        )),
        ViewRow::from_plain(format!(
            "total_spoilage_clawed_back: {}",
            r.total_spoilage_clawed_back
        )),
        ViewRow::from_plain(format!("broken_promises: {:?}", r.broken_promises)),
        ViewRow::from_plain(format!("elected_officers: {:?}", r.elected_officers)),
        ViewRow::from_plain(format!("charters_awarded: {:?}", r.charters_awarded)),
        ViewRow::from_plain(format!("public_forecasts: {:?}", r.public_forecasts)),
        ViewRow::from_plain(format!(
            "newly_activated_shocks: {:?}",
            r.newly_activated_shocks
        )),
        ViewRow::from_plain(format!("cred_debt_repaid: {}", r.cred_debt_repaid)),
        ViewRow::from_plain(format!("charter_cred_burned: {}", r.charter_cred_burned)),
        ViewRow::from_plain(format!(
            "town_import_cred_spent: {}",
            r.town_import_cred_spent
        )),
        ViewRow::from_plain(format!(
            "works_stipend_cred_minted: {}",
            r.works_stipend_cred_minted
        )),
        ViewRow::from_plain(format!("batch_trades_cleared: {}", r.batch_trades_cleared)),
        ViewRow::from_plain(format!("fills: {} entries", r.fills.len())),
        ViewRow::from_plain(format!(
            "wash_freeze_settlements: {:?}",
            r.wash_freeze_settlements
        )),
        ViewRow::from_plain(format!("season_closed: {}", r.season_closed)),
        ViewRow::from_plain(format!("pbrs_deltas: {:?}", r.pbrs_deltas)),
    ];
    for f in &r.fills {
        rows.push(ViewRow::from_plain(format_fill_line(
            f,
            plain_codes,
            commodities,
        )));
    }
    rows
}

fn format_fill_line(
    f: &FillRecord,
    plain_codes: bool,
    commodities: Option<&CommodityDisplayMap>,
) -> String {
    let good = f.good.map_or_else(
        || "none".to_owned(),
        |g| format_asset_label(g, commodities, plain_codes),
    );
    format!(
        "  Fill T{} {:?}: {} {:?}->S{} u:{} net:{} cred:{}",
        f.turn, f.kind, good, f.from, f.to, f.units, f.net_units, f.cred_amount
    )
}

pub(crate) fn build_lobby_screen(
    state: &crate::state::LobbyState,
    overrides: league_seat_types::LobbyCliOverrides,
    viewport: ViewportSize,
) -> SeatScreenView {
    let layout = ScreenLayout::layout_for_viewport(viewport);
    if layout.mode == ViewLayoutMode::TooSmall {
        return build_too_small_view_with_options(
            viewport,
            layout,
            overrides.no_color,
            overrides.screen_reader,
        );
    }

    let sel = |row: usize, text: String| {
        if state.focus_row == row {
            let prefix = if overrides.screen_reader {
                "[SELECTED] "
            } else {
                "> "
            };
            ViewRow::from_spans(vec![ViewSpan::styled(
                format!("{prefix}{text}"),
                ViewStyle::plain().with_fg(ViewColor::Cyan).bold(),
            )])
        } else {
            ViewRow::from_plain(format!("  {text}"))
        }
    };

    let mut rows = Vec::new();
    if overrides.screen_reader {
        rows.push(ViewRow::from_plain(
            "=== CLUB MODE LOBBY (LINEAR) ===".to_owned(),
        ));
    }
    rows.push(ViewRow::from_plain(
        "Configure offline Club Mode table (`dev` seed pool 0..=9999; CLI flags override Lobby):"
            .to_owned(),
    ));
    if overrides.no_color || overrides.screen_reader {
        let mut flags = Vec::new();
        if overrides.no_color {
            flags.push("--no-color / NO_COLOR");
        }
        if overrides.screen_reader {
            flags.push("--screen-reader");
        }
        rows.push(ViewRow::from_plain(format!(
            "Accessibility active: {}",
            flags.join(", ")
        )));
    }
    rows.push(sel(
        0,
        format!("Seat Count: {} (allowed 4..=6)", state.seat_count),
    ));

    for (s, is_human) in state
        .human_seats
        .iter()
        .copied()
        .enumerate()
        .take(usize::from(state.seat_count))
    {
        let role = if is_human {
            "Human (Regent fallback: Auto; editable pre-start in design, default Auto here)"
        } else {
            "ClubBot (AiSyndicate)"
        };
        rows.push(sel(
            1 + s,
            format!("Seat {s} ({}): {role}", guild_name_for_seat(s as u8)),
        ));
    }

    let seed_row = state.seed_row_idx();
    rows.push(sel(
        seed_row,
        format!("Layout Seed (dev 0..=9999): {}", state.seed_text),
    ));

    let effective_untimed =
        overrides.untimed || (overrides.turn_clock_secs.is_none() && state.untimed);
    let effective_secs = overrides.turn_clock_secs.unwrap_or(state.turn_clock_secs);
    let clock_note = if overrides.untimed {
        "Untimed (CLI override --untimed)"
    } else if overrides.turn_clock_secs.is_some() {
        "(CLI override --turn-clock)"
    } else if effective_untimed {
        "Untimed (no pass timeout)"
    } else {
        "75% Pass A / 25% Pass B across humans"
    };
    let clock_val = if effective_untimed {
        "Untimed".to_owned()
    } else {
        format!("{effective_secs}s")
    };
    rows.push(sel(
        state.clock_row_idx(),
        format!("Turn Clock: {clock_val} — {clock_note}"),
    ));

    let effective_pins = overrides.pins_enabled || state.pins_enabled;
    let pin_note = if overrides.pins_enabled {
        "On (CLI override --pins; set 4 digits at first Curtain)"
    } else if effective_pins {
        "On (enter 4-digit table PIN at first Curtain)"
    } else {
        "Off"
    };
    rows.push(sel(
        state.pins_row_idx(),
        format!("Curtain PINs: {pin_note}"),
    ));
    rows.push(sel(
        state.start_row_idx(),
        "[ Start Club Season ] (press Enter)".to_owned(),
    ));

    let total_rows = rows.len();
    let max_vis = usize::from(viewport.height.saturating_sub(6)).max(1);
    let scroll_off = state
        .focus_row
        .saturating_add(2)
        .saturating_sub(max_vis)
        .min(total_rows.saturating_sub(1));

    let panel = ViewPanel {
        title: if overrides.screen_reader {
            None
        } else {
            Some(ViewLabel::styled(
                "Merchant League — Club Mode Lobby".to_owned(),
                ViewStyle {
                    fg: ViewColor::Cyan,
                    bg: ViewColor::Reset,
                    bold: true,
                    ..ViewStyle::plain()
                },
            ))
        },
        alignment: ViewAlign::Left,
        bordered: !overrides.screen_reader,
        tabs: None,
        table: None,
        rows,
        selected_row: None,
        scroll: Some(ViewScroll {
            offset: scroll_off,
            total: total_rows,
        }),
    };

    let mut view = SeatScreenView {
        layout: if viewport.height >= 8 {
            ScreenLayout::single_panel_with_status(viewport)
        } else {
            ScreenLayout::single_panel(viewport)
        },
        zones: vec![ViewZonePanel {
            zone: ViewZoneId::SingleFull,
            panel,
        }],
        status_line: Some(ViewStatusLine {
            alert: None,
            left: vec![ViewSpan::plain(
                "Up/Down: Row | Left/Right/Space: Toggle | Digits: Seed | Enter: Start | q/Esc: Quit"
                    .to_owned(),
            )],
            right: vec![ViewSpan::plain("Lobby".to_owned())],
            style: ViewStyle::plain(),
        }),
        modal: None,
    };
    if overrides.no_color || overrides.screen_reader {
        strip_colors_in_view(&mut view);
    }
    view
}

pub(crate) fn build_awaiting_reveal_screen(
    table_id: league_seat_types::TableId,
    turn: Turn,
    progress: league_seat_types::AwaitingRevealProgress,
    viewport: ViewportSize,
) -> SeatScreenView {
    let layout = ScreenLayout::layout_for_viewport(viewport);
    if layout.mode == ViewLayoutMode::TooSmall {
        return build_too_small_view_with_options(viewport, layout, false, false);
    }

    let rows = vec![
        ViewRow::from_plain(format!(
            "Table #{} | Turn {} | Phase: {:?}",
            table_id.0,
            turn.0,
            TurnPhase::AwaitingMandateReveal
        )),
        ViewRow::from_plain(
            "Season closed — Turn-1 mandates stay sealed until all seats submit reveals."
                .to_owned(),
        ),
        ViewRow::from_plain(format!(
            "Mandate reveals submitted: {}/{} — press Enter to submit next reveal.",
            progress.revealed_count, progress.seat_count
        )),
    ];

    let panel = ViewPanel {
        title: Some(ViewLabel::styled(
            "Merchant League — Session Standby".to_owned(),
            ViewStyle {
                fg: ViewColor::Cyan,
                bg: ViewColor::Reset,
                bold: true,
                ..ViewStyle::plain()
            },
        )),
        alignment: ViewAlign::Center,
        bordered: true,
        tabs: None,
        table: None,
        rows,
        selected_row: None,
        scroll: None,
    };

    SeatScreenView {
        layout: if viewport.height >= 8 {
            ScreenLayout::single_panel_with_status(viewport)
        } else {
            ScreenLayout::single_panel(viewport)
        },
        zones: vec![ViewZonePanel {
            zone: ViewZoneId::SingleFull,
            panel,
        }],
        status_line: Some(ViewStatusLine {
            alert: None,
            left: vec![ViewSpan::plain(
                "Enter/Space: Submit Mandate Reveal | q / Esc / Ctrl-C: Quit".to_owned(),
            )],
            right: vec![ViewSpan::plain("Awaiting Reveal".to_owned())],
            style: ViewStyle::plain(),
        }),
        modal: None,
    }
}

pub(crate) fn build_end_of_season_screen(
    data: &league_seat_types::EndOfSeasonData,
    state: &crate::state::EndOfSeasonUiState,
    commodities: Option<&CommodityDisplayMap>,
    viewport: ViewportSize,
) -> SeatScreenView {
    use crate::state::{ALL_END_OF_SEASON_TABS, EndOfSeasonTab};
    use league_seat_types::ReplayExportState;

    let layout = ScreenLayout::layout_for_viewport(viewport);
    if layout.mode == ViewLayoutMode::TooSmall {
        return build_too_small_view_with_options(
            viewport,
            layout,
            state.no_color,
            state.screen_reader,
        );
    }

    let eos_active_idx = match state.active_tab {
        EndOfSeasonTab::Scoreboard => 0,
        EndOfSeasonTab::Mandates => 1,
        EndOfSeasonTab::SeasonMetrics => 2,
        EndOfSeasonTab::ReplayExport => 3,
    };
    let tabs = ViewTabBar {
        tabs: ALL_END_OF_SEASON_TABS
            .iter()
            .enumerate()
            .map(|(i, tab)| styled_tab_item(tab.label(), true, i == eos_active_idx))
            .collect(),
        active_index: eos_active_idx,
    };

    let mut rows = Vec::new();
    if state.screen_reader {
        rows.push(ViewRow::from_plain(format!(
            "=== END OF SEASON: {} ===",
            state.active_tab.label()
        )));
    }
    rows.push(ViewRow::from_plain(format!(
        "Table #{} | Closed Turn {} | {} seats",
        data.table_id.0,
        data.closed_turn.0,
        data.scoreboard.standings.len()
    )));
    let mut table = None;

    match state.active_tab {
        EndOfSeasonTab::Scoreboard => {
            rows.push(ViewRow::from_plain(
                "Final Standings (sorted by Total Equity, tie-broken cleanly via standings order):"
                    .to_owned(),
            ));
            let columns = vec![
                ViewTableColumn {
                    header: ViewLabel::plain("Rank".to_owned()),
                    width: ViewSizeHint::Fixed(4),
                    alignment: ViewAlign::Right,
                },
                ViewTableColumn {
                    header: ViewLabel::plain("Seat".to_owned()),
                    width: ViewSizeHint::Fixed(4),
                    alignment: ViewAlign::Left,
                },
                ViewTableColumn {
                    header: ViewLabel::plain("TotalEq".to_owned()),
                    width: ViewSizeHint::Min(8),
                    alignment: ViewAlign::Right,
                },
                ViewTableColumn {
                    header: ViewLabel::plain("ProjEq".to_owned()),
                    width: ViewSizeHint::Min(7),
                    alignment: ViewAlign::Right,
                },
                ViewTableColumn {
                    header: ViewLabel::plain("NetEq".to_owned()),
                    width: ViewSizeHint::Min(7),
                    alignment: ViewAlign::Right,
                },
                ViewTableColumn {
                    header: ViewLabel::plain("MandEq".to_owned()),
                    width: ViewSizeHint::Min(6),
                    alignment: ViewAlign::Right,
                },
                ViewTableColumn {
                    header: ViewLabel::plain("Tier".to_owned()),
                    width: ViewSizeHint::Fixed(4),
                    alignment: ViewAlign::Left,
                },
            ];
            let mut trows = Vec::new();
            for (idx, entry) in data.scoreboard.standings.iter().enumerate() {
                let rank = idx + 1;
                trows.push(ViewTableRow {
                    cells: vec![
                        ViewTableCell::aligned(
                            format!("#{rank}"),
                            ViewAlign::Right,
                            ViewStyle::plain(),
                        ),
                        ViewTableCell::plain(format!("S{}", entry.seat)),
                        ViewTableCell::aligned(
                            format!("{}", entry.total_equity),
                            ViewAlign::Right,
                            ViewStyle::plain().bold(),
                        ),
                        ViewTableCell::aligned(
                            format!("{}", entry.project_equity),
                            ViewAlign::Right,
                            ViewStyle::plain(),
                        ),
                        ViewTableCell::aligned(
                            format!("{}", entry.net_asset_equity),
                            ViewAlign::Right,
                            ViewStyle::plain(),
                        ),
                        ViewTableCell::aligned(
                            format!("{}", entry.mandate_bonus),
                            ViewAlign::Right,
                            ViewStyle::plain(),
                        ),
                        ViewTableCell::plain(format!("{:?}", entry.credit_tier)),
                    ],
                    style: ViewStyle::plain(),
                });
            }
            if !state.screen_reader {
                table = Some(ViewTable {
                    columns,
                    rows: trows,
                    selected_row: None,
                });
            }
            for (idx, entry) in data.scoreboard.standings.iter().enumerate() {
                rows.push(ViewRow::from_plain(format!(
                    "  Rank #{} — S{} ({}) | TotalEq={} ProjEq={} NetEq={} MandEq={} Tier={:?} HonorPen={}",
                    idx + 1,
                    entry.seat,
                    guild_name_for_seat(entry.seat),
                    entry.total_equity,
                    entry.project_equity,
                    entry.net_asset_equity,
                    entry.mandate_bonus,
                    entry.credit_tier,
                    entry.honor_penalties
                )));
            }
        }
        EndOfSeasonTab::Mandates => {
            rows.push(ViewRow::from_plain(
                "Unsealed Turn-1 Private Charter Mandates (revealed post-close):".to_owned(),
            ));
            for (idx, mandate) in data.revealed_mandates.iter().enumerate() {
                let seat_u8 = idx as u8;
                let bonus_awarded = data
                    .scoreboard
                    .standings
                    .iter()
                    .find(|e| e.seat == seat_u8)
                    .map_or(0, |e| e.mandate_bonus);
                let status = if bonus_awarded > 0 {
                    format!("▲ AWARDED +{bonus_awarded}")
                } else {
                    "▼ Unmet (+0)".to_owned()
                };
                let status_style = if bonus_awarded > 0 {
                    ViewStyle::plain().with_fg(ViewColor::Green).bold()
                } else {
                    ViewStyle::plain().with_fg(ViewColor::DarkGray)
                };
                rows.push(ViewRow::from_spans(vec![
                    ViewSpan::plain(format!(
                        "S{} ({}): {} | BaseBonus: {} | ",
                        seat_u8,
                        guild_name_for_seat(seat_u8),
                        format_mandate_kind(mandate.kind),
                        mandate.bonus_equity,
                    )),
                    ViewSpan::styled(status, status_style),
                ]));
                rows.push(ViewRow::from_plain(format!(
                    "  {}",
                    format_mandate_provenance(
                        idx,
                        mandate.kind,
                        data,
                        commodities,
                        state.plain_codes
                    )
                )));
            }
        }
        EndOfSeasonTab::SeasonMetrics => {
            rows.push(ViewRow::from_plain(
                "2026-11-16 Close-Marking Splits (HoldingMark) & Solvency / Honor Metrics:"
                    .to_owned(),
            ));
            for (idx, metric) in data.season_metrics.iter().enumerate() {
                let entry = data
                    .scoreboard
                    .standings
                    .iter()
                    .find(|e| e.seat == metric.seat);
                let (defaults, breaches, broken, honor_pen, tier) =
                    entry.map_or((0, 0, 0, 0, "Aaa".to_owned()), |e| {
                        (
                            e.hard_credit_defaults,
                            e.bond_breaches,
                            e.broken_promises,
                            e.honor_penalties,
                            format!("{:?}", e.credit_tier),
                        )
                    });
                rows.push(ViewRow::from_plain(format!(
                    "S{} ({}) | Tier {tier} | Defaults:{defaults} Breaches:{breaches} Broken:{broken} HonorPen:{honor_pen}",
                    metric.seat,
                    guild_name_for_seat(metric.seat),
                )));
                rows.push(ViewRow::from_plain(format!(
                    "  Metrics: held_short={:?} covered_short={:?} spoiled={} stranded_u={} floor_val={}",
                    metric.held_enough_but_short_turns,
                    metric.covered_with_own_offers_but_short_turns,
                    metric.spoiled_units,
                    metric.stranded_units_at_close,
                    metric.stranded_value_at_close
                )));
                if let Some(pred) = data.close_predicates.get(idx) {
                    for h in &pred.season_end.holdings {
                        if h.units > 0 {
                            rows.push(ViewRow::from_plain(format!(
                                "  Mark {}: units={} marked={} (bid_u={} floor_u={} floor_val={})",
                                format_asset_label(h.good, commodities, state.plain_codes),
                                h.units,
                                h.marked_value,
                                h.units_marked_by_bids,
                                h.units_at_floor,
                                h.value_at_floor
                            )));
                        }
                    }
                }
            }
        }
        EndOfSeasonTab::ReplayExport => {
            rows.push(ViewRow::from_plain(
                "SeasonReplayLog Export & Private .seal Cleanup:".to_owned(),
            ));
            let msg = match data.export_state {
                ReplayExportState::Ready => {
                    "Status: Ready — press `e` or `Enter` to verify & export SeasonReplayLog JSON and wipe `.seal`."
                }
                ReplayExportState::Exported => {
                    "Status: Exported & Verified — SeasonReplayLog written cleanly and private `.seal` wiped."
                }
                ReplayExportState::Failed => {
                    "Status: Export Failed — private `.seal` was preserved on disk; see details below."
                }
            };
            rows.push(ViewRow::from_plain(msg.to_owned()));
        }
    }

    let total_rows = rows.len() + table.as_ref().map_or(0, |t| t.rows.len() + 1);
    let panel = ViewPanel {
        title: if state.screen_reader {
            None
        } else {
            Some(ViewLabel::styled(
                "Merchant League — End of Season".to_owned(),
                ViewStyle {
                    fg: ViewColor::Cyan,
                    bg: ViewColor::Reset,
                    bold: true,
                    ..ViewStyle::plain()
                },
            ))
        },
        alignment: ViewAlign::Left,
        bordered: !state.screen_reader,
        tabs: if state.screen_reader {
            None
        } else {
            Some(tabs)
        },
        table,
        rows,
        selected_row: None,
        scroll: Some(ViewScroll {
            offset: state.scroll.min(total_rows.saturating_sub(1)),
            total: total_rows,
        }),
    };

    let mut view = SeatScreenView {
        layout: if viewport.height >= 8 {
            ScreenLayout::single_panel_with_status(viewport)
        } else {
            ScreenLayout::single_panel(viewport)
        },
        zones: vec![ViewZonePanel {
            zone: ViewZoneId::SingleFull,
            panel,
        }],
        status_line: Some(ViewStatusLine {
            alert: None,
            left: vec![ViewSpan::plain(
                "1-4/Tab: Tab | Up/Down/PgDn: Scroll | c: Codes | e/Enter(Tab 4): Export Replay | q/Esc: Quit"
                    .to_owned(),
            )],
            right: vec![ViewSpan::plain(state.active_tab.label().to_owned())],
            style: ViewStyle::plain(),
        }),
        modal: None,
    };
    if state.no_color || state.screen_reader {
        strip_colors_in_view(&mut view);
    }
    view
}

pub(crate) fn build_resume_digest_rows(
    obs: &SeatObservation,
    plain_codes: bool,
    commodities: Option<&CommodityDisplayMap>,
) -> Vec<ViewRow> {
    let mut rows = Vec::new();
    let receipts = &obs.public_receipts;
    if receipts.is_empty() {
        rows.push(ViewRow::from_plain(
            "Previously This Season (T1 opening — 0 prior turns swept) [d: toggle]".to_owned(),
        ));
        rows.push(ViewRow::from_plain(
            "  Season opens at Turn 1; no prior PublicSweepReceipt history yet.".to_owned(),
        ));
        return rows;
    }

    let first_t = receipts.first().map_or(1, |r| r.turn);
    let last_t = receipts.last().map_or(first_t, |r| r.turn);
    let count = receipts.len();
    let seat_u8 = obs.seat;
    let seat_idx = usize::from(seat_u8);

    let mut cum_pbrs: i64 = 0;
    let mut last_pbrs: i64 = 0;
    let mut batch_trades: usize = 0;
    let mut atomic_swaps: usize = 0;
    let mut converter_runs: usize = 0;
    let mut projects_done: usize = 0;
    let mut total_debt: i64 = 0;
    let mut total_spoilage: i64 = 0;
    let mut cashed_checks: usize = 0;
    let mut bounced_checks: usize = 0;
    let mut escrows_finished: usize = 0;
    let mut escrows_forfeited: usize = 0;
    let mut broken_promises: usize = 0;
    let mut charters_count: usize = 0;
    let mut shocks_activated: usize = 0;
    let mut forecasts_count: usize = 0;
    let mut last_officers: Option<(u8, Option<u8>)> = None;
    let mut seat_fills_in: usize = 0;
    let mut seat_fills_out: usize = 0;
    let mut partner_flags = [false; 6];
    let mut regent_turns: Vec<u8> = Vec::new();
    let mut non_reveal_turns: Vec<u8> = Vec::new();
    let mut converted_to_ai_turn: Option<u8> = None;

    for r in receipts {
        if let Some(&delta) = r.pbrs_deltas.get(seat_idx) {
            cum_pbrs = cum_pbrs.saturating_add(delta);
            last_pbrs = delta;
        }
        batch_trades = batch_trades.saturating_add(r.batch_trades_cleared);
        atomic_swaps = atomic_swaps.saturating_add(r.atomic_swaps_settled);
        converter_runs = converter_runs.saturating_add(r.converter_batches_run);
        projects_done = projects_done.saturating_add(r.projects_completed);
        total_debt = total_debt.saturating_add(r.total_upkeep_shortfall_debt);
        total_spoilage = total_spoilage.saturating_add(r.total_spoilage_clawed_back);
        cashed_checks = cashed_checks.saturating_add(r.checks_cashed.len());
        bounced_checks = bounced_checks.saturating_add(r.checks_bounced.len());
        escrows_finished = escrows_finished.saturating_add(r.escrows_finished.len());
        escrows_forfeited =
            escrows_forfeited.saturating_add(r.escrows_cancelled_or_forfeited.len());
        broken_promises = broken_promises.saturating_add(r.broken_promises.len());
        charters_count = charters_count.saturating_add(r.charters_awarded.len());
        shocks_activated = shocks_activated.saturating_add(r.newly_activated_shocks.len());
        forecasts_count = forecasts_count.saturating_add(r.public_forecasts.len());
        if let Some(officers) = r.elected_officers {
            last_officers = Some(officers);
        }
        if r.regent_acted_seats.contains(&seat_u8) {
            regent_turns.push(r.turn);
        }
        if r.non_reveal_seats.contains(&seat_u8) {
            non_reveal_turns.push(r.turn);
        }
        if r.converted_to_ai.contains(&seat_u8) && converted_to_ai_turn.is_none() {
            converted_to_ai_turn = Some(r.turn);
        }
        for f in &r.fills {
            if fill_records_partner_delivery(f) {
                if f.from == Some(seat_u8) {
                    seat_fills_out = seat_fills_out.saturating_add(1);
                    if f.to != seat_u8 && usize::from(f.to) < 6 {
                        partner_flags[usize::from(f.to)] = true;
                    }
                } else if f.to == seat_u8 {
                    seat_fills_in = seat_fills_in.saturating_add(1);
                    if let Some(peer) = f.from
                        && peer != seat_u8
                        && usize::from(peer) < 6
                    {
                        partner_flags[usize::from(peer)] = true;
                    }
                }
            }
        }
    }

    let partner_count = partner_flags.iter().filter(|&&p| p).count();
    let officers_desc = last_officers.map_or_else(
        || "none".to_owned(),
        |(tm, cc)| {
            let cc_str = cc.map_or_else(|| "None".to_owned(), |c| format!("S{c}"));
            format!("TM=S{tm}, CC={cc_str}")
        },
    );
    let (custody_desc, custody_style) = if let Some(t) = converted_to_ai_turn {
        (
            format!("▼ converted_to_ai@T{t}"),
            ViewStyle::plain().with_fg(ViewColor::Red).bold(),
        )
    } else if !regent_turns.is_empty() || !non_reveal_turns.is_empty() {
        (
            format!(
                "▼ regent_turns={:?} non_reveal={:?}",
                regent_turns, non_reveal_turns
            ),
            ViewStyle::plain().with_fg(ViewColor::Yellow).bold(),
        )
    } else {
        (
            "= 0 Regent/non-reveal interventions".to_owned(),
            ViewStyle::plain(),
        )
    };

    rows.push(ViewRow::from_plain(format!(
        "Previously This Season (Turns T{first_t}..=T{last_t}, {count} swept) [d: toggle]"
    )));
    rows.push(ViewRow::from_spans(vec![
        ViewSpan::plain(format!("  Seat S{seat_u8}: PBRS cum ")),
        ViewSpan::styled(
            format_signed_delta(cum_pbrs),
            signed_delta_style(cum_pbrs, true),
        ),
        ViewSpan::plain(format!(" (last T{last_t}: ")),
        ViewSpan::styled(
            format_signed_delta(last_pbrs),
            signed_delta_style(last_pbrs, true),
        ),
        ViewSpan::plain(format!(
            ") | fills in:{seat_fills_in} out:{seat_fills_out} partners:{partner_count} | "
        )),
        ViewSpan::styled(custody_desc, custody_style),
    ]));
    rows.push(ViewRow::from_plain(format!(
        "  Table Totals: trades={batch_trades} swaps={atomic_swaps} conv={converter_runs} proj={projects_done} debt={total_debt} spoil={total_spoilage}"
    )));
    rows.push(ViewRow::from_plain(format!(
        "  Credit/Gov: checks({cashed_checks}/{bounced_checks}) esc({escrows_finished}/{escrows_forfeited}) promises({broken_promises}) officers({officers_desc}) charters={charters_count} shocks={shocks_activated}/fcst={forecasts_count}"
    )));

    let recent_start = receipts.len().saturating_sub(3);
    for r in &receipts[recent_start..] {
        let mut notes = Vec::new();
        for sh in &r.newly_activated_shocks {
            notes.push(format!(
                "shock S{} {:?} {}bps",
                sh.target_seat, sh.kind, sh.magnitude_bps
            ));
        }
        for fc in &r.public_forecasts {
            notes.push(format!(
                "fcst S{} {:?}@T{}",
                fc.target_seat, fc.kind, fc.realize_turn
            ));
        }
        if let Some((tm, cc)) = r.elected_officers {
            let cc_str = cc.map_or_else(|| "None".to_owned(), |c| format!("S{c}"));
            notes.push(format!("elected TM=S{tm}/CC={cc_str}"));
        }
        if !r.charters_awarded.is_empty() {
            notes.push(format!("charters={}", r.charters_awarded.len()));
        }
        if !r.checks_bounced.is_empty() {
            notes.push(format!("bounced={}", r.checks_bounced.len()));
        }
        if !r.broken_promises.is_empty() {
            notes.push(format!("broken={}", r.broken_promises.len()));
        }
        if let Some(first_fill) = r.fills.first()
            && r.fills.len() == 1
        {
            let good_s = first_fill.good.map_or_else(
                || "CRED".to_owned(),
                |g| format_asset_label(g, commodities, plain_codes),
            );
            notes.push(format!("fill {good_s} u={}", first_fill.net_units));
        }
        let rp = r.pbrs_deltas.get(seat_idx).copied().unwrap_or(0);
        let tail = if notes.is_empty() {
            String::new()
        } else {
            format!(" | {}", notes.join("; "))
        };
        rows.push(ViewRow::from_spans(vec![
            ViewSpan::plain(format!("  Recent T{}: pbrs=", r.turn)),
            ViewSpan::styled(format_signed_delta(rp), signed_delta_style(rp, true)),
            ViewSpan::plain(format!(
                " trades={} swaps={} conv={} proj={} debt={} spoil={}{tail}",
                r.batch_trades_cleared,
                r.atomic_swaps_settled,
                r.converter_batches_run,
                r.projects_completed,
                r.total_upkeep_shortfall_debt,
                r.total_spoilage_clawed_back,
            )),
        ]));
    }
    rows
}

fn build_screen_reader_turn_rows(
    obs: &SeatObservation,
    state: &SeatUiState,
    pass: PassKind,
    commodities: Option<&CommodityDisplayMap>,
) -> (Vec<ViewRow>, usize) {
    let mut rows = Vec::new();
    let header_panel = build_header_panel(
        obs,
        pass,
        commodities,
        state.plain_codes,
        state.clock_display,
    );

    if state.chronicle_open {
        rows.push(ViewRow::from_plain(
            "=== ACTIVE MODAL: SWEEP CHRONICLE ([/] Turn | Esc/r Close) ===".to_owned(),
        ));
        if obs.public_receipts.is_empty() {
            rows.push(ViewRow::from_plain(
                "Sweep Chronicle: no completed PublicSweepReceipt turns yet.".to_owned(),
            ));
        } else {
            let idx = state
                .chronicle_receipt_idx
                .min(obs.public_receipts.len() - 1);
            rows.extend(chronicle_receipt_rows(
                &obs.public_receipts[idx],
                idx,
                obs.public_receipts.len(),
                state.plain_codes,
                commodities,
            ));
        }
        rows.push(ViewRow::from_plain(String::new()));
    }

    rows.push(ViewRow::from_plain(
        "=== MERCHANT LEAGUE: SCREEN READER TURN VIEW ===".to_owned(),
    ));
    if let Some(title) = &header_panel.title {
        rows.push(ViewRow::from_plain(title.text.clone()));
    }
    for r in header_panel.rows {
        rows.push(r);
    }
    let mode_tag = turn_mode_tag(state);
    let focus_label = match state.selection.focus_zone {
        FocusZone::PublicLedger => "Public Ledger",
        FocusZone::CenterTabs => "Center Composer",
        FocusZone::PrivateDrawer => "Private Vault",
    };
    let tab_label = match state.active_tab {
        CenterTab::MarketAndContracts => "1: Market & Contracts",
        CenterTab::WorkshopDealsPolitics => "2: Workshop, Deals & Politics",
        CenterTab::WhatIfSandbox => "3: What-If Sandbox",
    };
    rows.push(ViewRow::from_plain(format!(
        "Focus Zone: {focus_label} | Mode: {mode_tag} | Active Tab: {tab_label}"
    )));
    rows.push(ViewRow::from_plain(String::new()));

    rows.push(ViewRow::from_plain(
        "=== PREVIOUSLY THIS SEASON ===".to_owned(),
    ));
    if state.resume_digest_open || obs.public_receipts.is_empty() {
        rows.extend(build_resume_digest_rows(
            obs,
            state.plain_codes,
            commodities,
        ));
    } else {
        let first_t = obs.public_receipts.first().map_or(1, |r| r.turn);
        let last_t = obs.public_receipts.last().map_or(first_t, |r| r.turn);
        rows.push(ViewRow::from_plain(format!(
            "Collapsed summary for Turns T{first_t}..=T{last_t} ({} swept) — press d to expand full digest.",
            obs.public_receipts.len()
        )));
    }
    rows.push(ViewRow::from_plain(String::new()));

    rows.push(ViewRow::from_plain(
        "=== PUBLIC TABLE & LEDGER MIRROR ===".to_owned(),
    ));
    let ledger_start = rows.len();
    for s in &obs.seats {
        let role = match s.identity {
            SeatIdentity::Human => "Human",
            SeatIdentity::AiSyndicate => "ClubBot",
        };
        let frz = if s.wash_frozen_until_turn > 0 {
            format!("frozen until T{}", s.wash_frozen_until_turn)
        } else {
            "clear".to_owned()
        };
        rows.push(ViewRow::from_plain(format!(
            "  S{} ({}, {}) | CRED={} | Debt={} | Tier={:?} | ProjectEq={} | Tariffs={}/{}bps | Defaults={} Broken={} Freeze={frz}",
            s.seat,
            guild_name_for_seat(s.seat),
            role,
            s.cred,
            s.cred_debt,
            s.credit_tier,
            s.project_equity,
            s.raw_tariff_bps,
            s.refined_tariff_bps,
            s.hard_credit_defaults,
            s.broken_promises,
        )));
    }
    rows.extend(public_ledger_body_rows(
        obs,
        state.plain_codes,
        commodities,
        LedgerTableLayout::FullStacked,
    ));
    let ledger_end = rows.len().saturating_sub(1);
    rows.push(ViewRow::from_plain(String::new()));

    rows.push(ViewRow::from_plain(format!(
        "=== ACTIVE WORKSPACE: {tab_label} ==="
    )));
    let workspace_header = rows.len().saturating_sub(1);
    let mut center_selected_row: Option<usize> = None;
    for row in center_tab_rows(obs, state, commodities) {
        let plain = row.plain_text();
        if let Some(rest) = plain.strip_prefix("> ") {
            if center_selected_row.is_none() {
                center_selected_row = Some(rows.len());
            }
            rows.push(ViewRow::from_plain(format!("[SELECTED] {rest}")));
        } else {
            rows.push(row);
        }
    }
    rows.push(ViewRow::from_plain(String::new()));

    rows.push(ViewRow::from_plain(format!(
        "=== PRIVATE VAULT (SEAT S{} ONLY) ===",
        obs.seat
    )));
    let drawer_start = rows.len();
    rows.extend(private_drawer_rows(obs, state, commodities));
    let drawer_end = rows.len().saturating_sub(1);

    let target_row = match state.selection.focus_zone {
        FocusZone::PublicLedger => (ledger_start + state.selection.ledger_row_idx).min(ledger_end),
        FocusZone::CenterTabs => center_selected_row.unwrap_or(workspace_header),
        FocusZone::PrivateDrawer => (drawer_start + state.selection.drawer_row_idx).min(drawer_end),
    };
    (rows, target_row)
}

pub(crate) fn screen_reader_target_and_total(
    obs: &SeatObservation,
    state: &SeatUiState,
) -> (usize, usize) {
    let (rows, target) = build_screen_reader_turn_rows(obs, state, PassKind::SubPassA, None);
    (target, rows.len())
}

fn build_screen_reader_turn_screen(
    obs: &SeatObservation,
    state: &SeatUiState,
    pass: PassKind,
    commodities: Option<&CommodityDisplayMap>,
    viewport: ViewportSize,
) -> SeatScreenView {
    let (rows, _) = build_screen_reader_turn_rows(obs, state, pass, commodities);
    let total = rows.len();
    let offset = state.screen_reader_scroll.min(total.saturating_sub(1));
    let digest_key = if state.resume_digest_open {
        "d:HideDigest"
    } else {
        "d:Digest"
    };
    let mode_tag = turn_mode_tag(state);

    let mut view = SeatScreenView {
        layout: if viewport.height >= 8 {
            ScreenLayout::single_panel_with_status(viewport)
        } else {
            ScreenLayout::single_panel(viewport)
        },
        zones: vec![ViewZonePanel {
            zone: ViewZoneId::SingleFull,
            panel: ViewPanel {
                title: None,
                alignment: ViewAlign::Left,
                bordered: false,
                tabs: None,
                table: None,
                rows,
                selected_row: None,
                scroll: Some(ViewScroll { offset, total }),
            },
        }],
        status_line: Some(ViewStatusLine {
            alert: None,
            left: vec![ViewSpan::plain(
                "Linear Mode | 1-3:Tabs F6:Zone [/]:Item Arrows:Act PgUp/PgDn:Scroll".to_owned(),
            )],
            right: vec![ViewSpan::plain(format!(
                "[{mode_tag}] c:Codes {digest_key} r:Chron p:WhatIf Ctrl+L:Lock q:Quit"
            ))],
            style: ViewStyle::plain(),
        }),
        modal: None,
    };
    strip_colors_in_view(&mut view);
    view
}

fn format_mandate_provenance(
    seat_idx: usize,
    kind: MandateKind,
    data: &league_seat_types::EndOfSeasonData,
    commodities: Option<&CommodityDisplayMap>,
    plain_codes: bool,
) -> String {
    let seat_u8 = seat_idx as u8;
    let close_t = data.closed_turn.0;
    match kind {
        MandateKind::MinProjectEquity { target } => {
            let final_proj = data
                .scoreboard
                .standings
                .iter()
                .find(|e| e.seat == seat_u8)
                .map_or(0, |e| e.project_equity);
            let status_note = if final_proj >= target {
                format!("target met by T{close_t}")
            } else {
                format!("unmet at T{close_t}")
            };
            format!("Provenance: project equity {final_proj}/{target} [{status_note}]")
        }
        MandateKind::MinActivePartners { min_partners } => {
            let mut first_partner_turn: [Option<u8>; 6] = [None; 6];
            let mut partner_order: Vec<(u8, u8)> = Vec::new();
            for r in &data.public_receipts {
                for f in &r.fills {
                    if fill_records_partner_delivery(f) {
                        let peer = if f.from == Some(seat_u8) && f.to != seat_u8 {
                            Some(f.to)
                        } else if f.to == seat_u8 {
                            f.from.filter(|&p| p != seat_u8)
                        } else {
                            None
                        };
                        if let Some(p) = peer
                            && usize::from(p) < 6
                            && first_partner_turn[usize::from(p)].is_none()
                        {
                            first_partner_turn[usize::from(p)] = Some(f.turn);
                            partner_order.push((p, f.turn));
                        }
                    }
                }
            }
            let count = partner_order.len();
            if data.public_receipts.is_empty() {
                return format!(
                    "Provenance: active partners target {min_partners} [evaluated at T{close_t} close]"
                );
            }
            let tags: Vec<String> = partner_order
                .iter()
                .map(|(p, t)| format!("S{p}@T{t}"))
                .collect();
            let list_str = if tags.is_empty() {
                "none".to_owned()
            } else {
                tags.join(", ")
            };
            let target_idx = usize::from(min_partners);
            let met_note = if target_idx == 0 {
                "target met T1".to_owned()
            } else if count >= target_idx {
                format!("target met T{}", partner_order[target_idx - 1].1)
            } else {
                format!("unmet at T{close_t}")
            };
            format!("Provenance: active partners {count}/{min_partners} [{list_str}; {met_note}]")
        }
        MandateKind::MinRefinedVariety {
            distinct_goods,
            min_each,
        } => {
            let Some(close_pred) = data.close_predicates.get(seat_idx) else {
                return format!(
                    "Provenance: refined >= {min_each} x{distinct_goods} evaluated at T{close_t}"
                );
            };
            let mut qualified = Vec::new();
            for sc in &close_pred.scarcity {
                if let AssetId::Refined(_) = sc.good
                    && sc.own_units >= min_each
                {
                    let label = format_asset_label(sc.good, commodities, plain_codes);
                    let since_t = earliest_continuous_refined_turn(
                        seat_idx,
                        sc.good,
                        min_each,
                        close_t,
                        &data.predicate_turns,
                    );
                    qualified.push(format!("{label}={} since T{since_t}", sc.own_units));
                }
            }
            let summary = if qualified.is_empty() {
                "none".to_owned()
            } else {
                qualified.join(", ")
            };
            format!(
                "Provenance: refined >= {min_each} at T{close_t}: {}/{} [{summary}]",
                qualified.len(),
                distinct_goods
            )
        }
        MandateKind::MaintainCleanCredit { min_tier } => {
            let terminal_tier = data
                .scoreboard
                .standings
                .iter()
                .find(|e| e.seat == seat_u8)
                .map_or(CreditTier::Aaa, |e| e.credit_tier);
            let tier_from_bps = |bps: u16| -> CreditTier {
                if bps <= CreditTier::Aaa.interest_bps() {
                    CreditTier::Aaa
                } else if bps <= CreditTier::Bbb.interest_bps() {
                    CreditTier::Bbb
                } else if bps <= CreditTier::B.interest_bps() {
                    CreditTier::B
                } else {
                    CreditTier::D
                }
            };
            let mut drop_turns = Vec::new();
            for tp in &data.predicate_turns {
                if let Some(seat_pred) = tp.at_turn_start.get(seat_idx)
                    && tier_from_bps(seat_pred.upkeep.interest_bps) > min_tier
                {
                    drop_turns.push(tp.turn);
                }
            }
            if terminal_tier <= min_tier && drop_turns.is_empty() {
                format!(
                    "Provenance: credit >= {min_tier:?} held T1..=T{close_t} (terminal {terminal_tier:?})"
                )
            } else if !drop_turns.is_empty() {
                format!(
                    "Provenance: credit below {min_tier:?} at T{}..=T{} (terminal {terminal_tier:?})",
                    drop_turns[0],
                    drop_turns[drop_turns.len() - 1]
                )
            } else {
                format!(
                    "Provenance: terminal credit {terminal_tier:?} below {min_tier:?} at T{close_t}"
                )
            }
        }
    }
}

fn earliest_continuous_refined_turn(
    seat_idx: usize,
    good: AssetId,
    min_each: i64,
    close_t: u8,
    predicate_turns: &[league_seat_types::TurnPredicateRecord],
) -> u8 {
    let mut since = close_t;
    for tp in predicate_turns.iter().rev() {
        let held = tp
            .at_turn_start
            .get(seat_idx)
            .and_then(|sp| sp.scarcity.iter().find(|s| s.good == good))
            .map_or(0, |s| s.own_units);
        if held >= min_each {
            since = tp.turn;
        } else {
            break;
        }
    }
    since
}
