use alloc::borrow::ToOwned;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use league_seat_types::{
    ActiveCharter, AssetId, CLOSING_ROLL_START_TURN, COMMODITY_DISPLAY_CATALOG,
    CommodityDisplayMap, CredOffer, CreditCheck, DualEscrowVault, FillRecord,
    HYPOTHETICAL_CLOSE_MARK_CAVEAT, IntentPledge, IntentTarget, MandateKind, MarketAction,
    PublicSweepReceipt, ScreenLayout, Seat, SeatIdentity, SeatObservation, SeatScreenView,
    SeatState, TokenEscrowVault, Turn, TurnPhase, ViewAlign, ViewColor, ViewLabel, ViewLayoutMode,
    ViewModal, ViewPanel, ViewRow, ViewScroll, ViewSizeHint, ViewSpan, ViewStatusLine, ViewStyle,
    ViewTabBar, ViewTabItem, ViewTable, ViewTableCell, ViewTableColumn, ViewTableRow, ViewZoneId,
    ViewZonePanel, ViewportSize, format_numeraire_display, scrub_secret_text,
};

use crate::composer::{
    build_what_if_request, bundle_field_count, market_field_count, preview_selected_asset,
};
use crate::state::{
    ALL_MARKET_KINDS, BundleSection, CenterTab, FocusZone, MarketActionKind, PassKind, SeatUiState,
    visible_bundle_sections,
};

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

pub(crate) fn build_curtain_view(
    table_id: league_seat_types::TableId,
    turn: Turn,
    pass: PassKind,
    next_seat: Seat,
    viewport: ViewportSize,
    space_armed: bool,
) -> SeatScreenView {
    let layout = ScreenLayout::layout_for_viewport(viewport);
    if layout.mode == ViewLayoutMode::TooSmall {
        return build_too_small_view(viewport, layout);
    }

    let prompt = if space_armed {
        "HAND-OFF ARMED ([Space] received) — press [Enter] to reveal seat screen"
    } else {
        "PRIVACY CURTAIN — press [Space] then [Enter] when ready to reveal seat screen"
    };
    let rows = vec![
        ViewRow::from_plain(format!("Table #{} | Turn {}", table_id.0, turn.0)),
        ViewRow::from_plain(format!("Pass: {}", pass.label())),
        ViewRow::from_plain(format!(
            "Hand keyboard to Seat {} ({})",
            next_seat.0,
            guild_name_for_seat(next_seat.0)
        )),
        ViewRow::from_plain(String::new()),
        ViewRow::from_plain(prompt.to_owned()),
    ];
    let panel = ViewPanel {
        title: Some(ViewLabel::styled(
            "Hot-Seat Privacy Curtain".to_owned(),
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
        left: vec![ViewSpan::plain(
            "Space:Arm Hand-Off | Enter:Confirm Reveal | q:Quit".to_owned(),
        )],
        right: vec![ViewSpan::plain(if space_armed {
            "Armed".to_owned()
        } else {
            "Locked Curtain".to_owned()
        })],
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
    viewport: ViewportSize,
) -> SeatScreenView {
    let layout = ScreenLayout::layout_for_viewport(viewport);
    if layout.mode == ViewLayoutMode::TooSmall {
        return build_too_small_view(viewport, layout);
    }

    let state_note = if quit_requested {
        "Session quit requested — returning to terminal."
    } else if phase != TurnPhase::TalkAndMarket {
        "Turn closed — waiting for host post-season step."
    } else {
        "No human seat active — session standby."
    };
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
        rows: vec![
            ViewRow::from_plain(format!(
                "Table #{} | Turn {} | Phase: {:?} | Pass: {}",
                table_id.0,
                turn.0,
                phase,
                pass.label()
            )),
            ViewRow::from_plain(state_note.to_owned()),
        ],
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

pub(crate) fn build_turn_screen(
    obs: &SeatObservation,
    state: &SeatUiState,
    pass: PassKind,
    commodities: Option<&CommodityDisplayMap>,
    viewport: ViewportSize,
) -> SeatScreenView {
    let layout = ScreenLayout::layout_for_viewport(viewport);
    if layout.mode == ViewLayoutMode::TooSmall {
        return build_too_small_view(viewport, layout);
    }

    let zones = vec![
        ViewZonePanel {
            zone: ViewZoneId::HeaderBar,
            panel: build_header_panel(obs, pass, commodities, state.plain_codes),
        },
        ViewZonePanel {
            zone: ViewZoneId::PublicLedger,
            panel: build_public_ledger_panel(obs, state, commodities),
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

    let mode_tag = if state.composer_edit_active {
        "EDIT DRAFT (Enter apply | Esc cancel)"
    } else if state.notes_edit_active {
        "EDIT NOTES (Enter/Esc done)"
    } else {
        match state.selection.focus_zone {
            FocusZone::PublicLedger => "Focus: Ledger",
            FocusZone::CenterTabs => "Focus: Center",
            FocusZone::PrivateDrawer => "Focus: Private",
        }
    };

    SeatScreenView {
        layout,
        zones,
        status_line: Some(ViewStatusLine {
            alert: None,
            left: vec![ViewSpan::plain(
                "1-3:Tabs F6:Zone [/]:Item Arrows/+/-:Adj e:Draft n:Notes Enter:Apply".to_owned(),
            )],
            right: vec![ViewSpan::plain(format!(
                "[{mode_tag}] c:Codes r:Chron p:WhatIf Ctrl+L:Lock q:Quit"
            ))],
            style: ViewStyle::plain(),
        }),
        modal,
    }
}

fn build_too_small_view(viewport: ViewportSize, layout: ScreenLayout) -> SeatScreenView {
    let panel = ViewPanel {
        title: Some(ViewLabel::styled(
            "Terminal too small".to_owned(),
            ViewStyle {
                fg: ViewColor::Yellow,
                bg: ViewColor::Reset,
                bold: true,
                ..ViewStyle::plain()
            },
        )),
        alignment: ViewAlign::Center,
        bordered: true,
        tabs: None,
        table: None,
        rows: vec![
            ViewRow::from_plain("Terminal too small".to_owned()),
            ViewRow::from_plain(format!(
                "Minimum viewport: 80x24 | Current: {}x{}",
                viewport.width, viewport.height
            )),
            ViewRow::from_plain("Resize terminal or press q to quit.".to_owned()),
        ],
        selected_row: None,
        scroll: None,
    };
    SeatScreenView {
        layout,
        zones: vec![ViewZonePanel {
            zone: ViewZoneId::SingleFull,
            panel,
        }],
        status_line: None,
        modal: None,
    }
}

fn build_header_panel(
    obs: &SeatObservation,
    pass: PassKind,
    commodities: Option<&CommodityDisplayMap>,
    plain_codes: bool,
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
                "{} | {} | TM: S{} | Chair: {}",
                pass.label(),
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

fn build_public_ledger_panel(
    obs: &SeatObservation,
    state: &SeatUiState,
    commodities: Option<&CommodityDisplayMap>,
) -> ViewPanel {
    let columns = vec![
        ViewTableColumn {
            header: ViewLabel::plain("Seat".to_owned()),
            width: ViewSizeHint::Fixed(5),
            alignment: ViewAlign::Left,
        },
        ViewTableColumn {
            header: ViewLabel::plain("Role".to_owned()),
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
            header: ViewLabel::plain("Eq".to_owned()),
            width: ViewSizeHint::Fixed(6),
            alignment: ViewAlign::Right,
        },
        ViewTableColumn {
            header: ViewLabel::plain("Tar(R/M)".to_owned()),
            width: ViewSizeHint::Fixed(9),
            alignment: ViewAlign::Right,
        },
        ViewTableColumn {
            header: ViewLabel::plain("Def/Br/Frz".to_owned()),
            width: ViewSizeHint::Min(8),
            alignment: ViewAlign::Right,
        },
    ];
    let table_rows: Vec<ViewTableRow> = obs.seats.iter().map(build_seat_table_row).collect();
    let table_len = table_rows.len();
    let rows = public_ledger_body_rows(obs, state.plain_codes, commodities);
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
) -> Vec<ViewRow> {
    let mut rows = Vec::new();
    append_ledger_inventory_rows(obs, plain_codes, commodities, &mut rows);
    append_ledger_book_rows(obs, plain_codes, commodities, &mut rows);
    rows
}

pub(crate) fn public_ledger_total_rows(obs: &SeatObservation) -> usize {
    obs.seats.len() + public_ledger_body_rows(obs, false, None).len()
}

fn build_seat_table_row(s: &SeatState) -> ViewTableRow {
    let role = match s.identity {
        SeatIdentity::Human => "HUM",
        SeatIdentity::AiSyndicate => "BOT",
    };
    let tier = match s.credit_tier {
        league_seat_types::CreditTier::Aaa => "Aaa",
        league_seat_types::CreditTier::Bbb => "Bbb",
        league_seat_types::CreditTier::B => "B",
        league_seat_types::CreditTier::D => "D",
    };
    let frz = if s.wash_frozen_until_turn > 0 {
        format!("T{}", s.wash_frozen_until_turn)
    } else {
        "-".to_owned()
    };
    ViewTableRow {
        cells: vec![
            ViewTableCell::plain(format!("S{}", s.seat)),
            ViewTableCell::plain(role.to_owned()),
            ViewTableCell::aligned(format!("{}", s.cred), ViewAlign::Right, ViewStyle::plain()),
            ViewTableCell::aligned(
                format!("{}", s.cred_debt),
                ViewAlign::Right,
                ViewStyle::plain(),
            ),
            ViewTableCell::plain(tier.to_owned()),
            ViewTableCell::aligned(
                format!("{}", s.project_equity),
                ViewAlign::Right,
                ViewStyle::plain(),
            ),
            ViewTableCell::aligned(
                format!("{}/{}", s.raw_tariff_bps, s.refined_tariff_bps),
                ViewAlign::Right,
                ViewStyle::plain(),
            ),
            ViewTableCell::aligned(
                format!("{}/{}/{}", s.hard_credit_defaults, s.broken_promises, frz),
                ViewAlign::Right,
                ViewStyle::plain(),
            ),
        ],
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

fn build_center_tabs_panel(
    obs: &SeatObservation,
    state: &SeatUiState,
    commodities: Option<&CommodityDisplayMap>,
) -> ViewPanel {
    let tab_bar = ViewTabBar {
        tabs: vec![
            ViewTabItem {
                label: ViewLabel::plain("1: Market & Contracts".to_owned()),
                enabled: true,
            },
            ViewTabItem {
                label: ViewLabel::plain("2: Workshop, Deals & Politics".to_owned()),
                enabled: true,
            },
            ViewTabItem {
                label: ViewLabel::plain("3: What-If Sandbox".to_owned()),
                enabled: true,
            },
            ViewTabItem {
                label: ViewLabel::plain("4: Diplomatic Desk (Disabled)".to_owned()),
                enabled: false,
            },
        ],
        active_index: state.active_tab.index(),
    };

    let rows = center_tab_rows(obs, state, commodities);
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

    rows.push(ViewRow::from_plain(format!(
        "Preview T{} S{}: Pre CRED {} -> Post-Mkt {} -> Post-Sweep {} (Debt {}, Eq {})",
        preview.turn,
        preview.seat,
        preview.pre_seat_state.cred,
        preview.post_market_seat_state.cred,
        preview.post_sweep_seat_state.cred,
        preview.post_sweep_seat_state.cred_debt,
        preview.post_sweep_seat_state.project_equity,
    )));
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
