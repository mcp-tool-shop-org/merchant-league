#![no_std]
#![forbid(unsafe_code)]
#![forbid(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]
//! Seat-safe `no_std` + `alloc` Merchant League UI view-model builder.
//!
//! Depends strictly on `league-seat-types` (never on `std`, `ratatui`, `crossterm`, `league-core`,
//! `league-gen`, or `league-standalone`). Every public entry point takes only concrete,
//! allow-listed `league-seat-types` values and local UI state, returning plain owned view-model
//! values ([`SeatScreenView`] / [`Action`]) for the trusted `league-tui` host to draw
//! (`docs/club-mode-design.md` §2.3).

extern crate alloc;

mod composer;
mod render;
mod state;

use alloc::borrow::ToOwned;
use alloc::format;
use alloc::string::String;
use alloc::vec;

use league_seat_types::{
    AssetId, CommodityDisplayMap, CurtainPinPrompt, NUMERAIRE_DISPLAY_LABEL, Seat, SeatKeyCode,
    SeatKeyEvent, SeatObservation, SeatScreenView, SeatState, TableId, Turn, TurnPhase, ViewAlign,
    ViewLabel, ViewPanel, ViewRow, ViewportSize,
};

pub use state::{
    BundleSection, CenterTab, FocusZone, MarketActionKind, PassKind, SeatUiSelection, SeatUiState,
    SeatUiStep,
};

/// The title screen's lines, top to bottom.
pub const TITLE_LINES: [&str; 3] = [
    "Merchant League",
    "A trading game on the XRPL Testnet. No real money.",
    "Not playable yet. Press q to quit.",
];

/// What a key press asks the seat UI to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Leave the client.
    Quit,
    /// Nothing: the key has no meaning on this screen.
    Ignore,
}

/// Map a plain seat key press to an [`Action`]. `q`, `Esc` and Ctrl-C quit (never auto-repeats).
#[must_use]
pub const fn action_for(key: SeatKeyEvent) -> Action {
    if key.is_repeat {
        return Action::Ignore;
    }
    match key.code {
        SeatKeyCode::Char('q') | SeatKeyCode::Esc => Action::Quit,
        SeatKeyCode::Char('c') if key.modifiers.ctrl => Action::Quit,
        _ => Action::Ignore,
    }
}

/// Build the title screen view model as plain owned data for the host to draw.
#[must_use]
pub fn title_view() -> SeatScreenView {
    let rows = TITLE_LINES
        .iter()
        .map(|line| ViewRow::from_plain((*line).to_owned()))
        .collect();
    SeatScreenView::single_panel(ViewPanel {
        title: None,
        alignment: ViewAlign::Center,
        bordered: true,
        tabs: None,
        table: None,
        rows,
        selected_row: None,
        scroll: None,
    })
}

/// Renders one turn screen for `obs.seat` using solely [`SeatObservation`], `state`,
/// [`CommodityDisplayMap`], and `viewport`.
#[must_use]
pub fn render_turn_screen(
    obs: &SeatObservation,
    state: &SeatUiState,
    pass: PassKind,
    commodities: Option<&CommodityDisplayMap>,
    viewport: ViewportSize,
) -> SeatScreenView {
    render::build_turn_screen(obs, state, pass, commodities, viewport)
}

/// Applies one single [`SeatKeyEvent`] to `state`, emitting the updated [`SeatScreenView`],
/// updated [`SeatUiState`], and optional seat-to-host [`league_seat_types::SeatIntent`].
#[must_use]
pub fn handle_seat_key(
    obs: &SeatObservation,
    mut state: SeatUiState,
    pass: PassKind,
    commodities: Option<&CommodityDisplayMap>,
    viewport: ViewportSize,
    key: SeatKeyEvent,
) -> SeatUiStep {
    let intent = composer::apply_seat_key(obs, &mut state, viewport, key);
    let view = render::build_turn_screen(obs, &state, pass, commodities, viewport);
    SeatUiStep {
        view,
        state,
        intent,
    }
}

/// Renders the hot-seat Privacy Curtain holding strictly `table_id`, `turn`, `pass`,
/// `next_seat`'s guild name, and any masked PIN prompt until confirmation completes.
#[must_use]
pub fn curtain_view(
    table_id: TableId,
    turn: Turn,
    pass: PassKind,
    next_seat: Seat,
    viewport: ViewportSize,
    space_armed: bool,
    pin_prompt: CurtainPinPrompt,
) -> SeatScreenView {
    render::build_curtain_view(
        table_id,
        turn,
        pass,
        next_seat,
        viewport,
        space_armed,
        pin_prompt,
    )
}

/// Renders a strictly public session-standby view when no human seat controls the keyboard
/// (after quit, at season close, or on a pass-advance error), containing zero seat private state.
#[must_use]
pub fn neutral_session_view(
    table_id: TableId,
    turn: Turn,
    phase: TurnPhase,
    pass: PassKind,
    quit_requested: bool,
    halted_on_error: bool,
    viewport: ViewportSize,
) -> SeatScreenView {
    render::build_neutral_session_view(
        table_id,
        turn,
        phase,
        pass,
        quit_requested,
        halted_on_error,
        viewport,
    )
}

/// Build one seat's minimal observation view model from [`SeatObservation`] alone.
#[must_use]
pub fn render_seat(obs: &SeatObservation) -> SeatScreenView {
    render_seat_view(obs, NUMERAIRE_DISPLAY_LABEL, None, false)
}

/// Build one seat's observation view model alongside table commodity display names from
/// [`CommodityDisplayMap`].
#[must_use]
pub fn render_seat_with_commodities(
    obs: &SeatObservation,
    commodities: &CommodityDisplayMap,
    plain_codes: bool,
) -> SeatScreenView {
    let cred_label = commodities
        .format_asset(AssetId::Cred, plain_codes)
        .unwrap_or_else(|| NUMERAIRE_DISPLAY_LABEL.to_owned());
    render_seat_view(obs, &cred_label, Some(commodities), plain_codes)
}

fn format_own_mandate_line(own_mandate: Option<&league_seat_types::PrivateMandate>) -> String {
    match own_mandate {
        Some(mandate) => format!(
            "Own Mandate: {:?} (+{} eq)",
            mandate.kind, mandate.bonus_equity
        ),
        None => "Own Mandate: none attached".to_owned(),
    }
}

fn render_seat_view(
    obs: &SeatObservation,
    cred_label: &str,
    commodities: Option<&CommodityDisplayMap>,
    plain_codes: bool,
) -> SeatScreenView {
    let seat_state: Option<&SeatState> = obs.seats.get(usize::from(obs.seat));
    let (cred_balance, cred_debt, tier_label) = match seat_state {
        Some(s) => (s.cred, s.cred_debt, format!("{:?}", s.credit_tier)),
        None => (0, 0, "Unknown".to_owned()),
    };
    let upkeep_label = obs
        .spec
        .seats
        .get(usize::from(obs.seat))
        .map(|cfg| {
            let asset = AssetId::Refined(cfg.upkeep_good);
            commodities
                .and_then(|m| m.format_asset(asset, plain_codes))
                .unwrap_or_else(|| format!("Refined({})", cfg.upkeep_good))
        })
        .unwrap_or_else(|| "None".to_owned());
    let mandate_line = format_own_mandate_line(obs.own_mandate());
    let rows = vec![
        ViewRow::from_plain(format!(
            "Table #{} | Turn {} ({:?}) | Seat {}",
            obs.table_id.0, obs.turn, obs.phase, obs.seat
        )),
        ViewRow::from_plain(format!(
            "{cred_label}: {cred_balance} | Debt: {cred_debt} | Tier: {tier_label}"
        )),
        ViewRow::from_plain(format!("Upkeep Good: {upkeep_label}")),
        ViewRow::from_plain(mandate_line),
        ViewRow::from_plain(format!(
            "Receipts: {} | Forecasts: {} | Hints: {}",
            obs.public_receipts.len(),
            obs.public_forecasts.len(),
            obs.auditor_hints().len()
        )),
    ];
    SeatScreenView::single_panel(ViewPanel {
        title: Some(ViewLabel::plain(format!("Seat {} View", obs.seat))),
        alignment: ViewAlign::Left,
        bordered: true,
        tabs: None,
        table: None,
        rows,
        selected_row: None,
        scroll: None,
    })
}

#[cfg(test)]
mod tests {
    use alloc::borrow::ToOwned;
    use alloc::collections::{BTreeMap, BTreeSet};
    use alloc::vec;
    use alloc::vec::Vec;

    use league_seat_types::{
        ActiveCharter, ActiveShock, AssetId, AtomicSwapProposal, AuditorShockHint,
        BondSettlementMode, BondSpec, BundleDraftAction, COMMODITY_DISPLAY_CATALOG, CartelDomain,
        CharterRole, CheckStatus, CoalitionReach, CommitmentHash, CommodityDisplayMap,
        CouncilPlatform, CredOffer, CreditCheck, CreditTier, CurtainPinPrompt, DualEscrowProposal,
        DualEscrowVault, EscrowStatus, FillKind, FillRecord, FundamentalParities, GoodScarcity,
        GradedMandateCard, HoldingMark, IntentPledge, IntentTarget, MandateKind, MarketAction,
        NUMERAIRE_CODE, NUMERAIRE_DISPLAY_LABEL, OfferSide, PreviewTieMode, PrivateMandate,
        PublicSweepReceipt, PublicTableSpec, RecipeCover, RefusedActionPreview, RegentDoctrine,
        STATE_PREDICATES_VERSION, SealedOrderBundle, SeasonEndAndMarks, Seat, SeatCommodityDisplay,
        SeatConfig, SeatIdentity, SeatIntentKind, SeatKeyCode, SeatKeyEvent, SeatKeyModifiers,
        SeatObservation, SeatObservationPublicParts, SeatSpoilagePreview, SeatState,
        SeatStatePredicates, SeatUpkeepPreview, ShockForecastEvent, ShockKind,
        SpoilageAndConversion, Step0To7SweepSummary, TableId, TableMode, TokenEscrowVault, Turn,
        TurnPhase, UpkeepAndDebt, ViewAlign, ViewLayoutMode, ViewZoneId, ViewportSize,
        WhatIfPreview, format_numeraire_display, scrub_secret_text,
    };

    use crate::{
        Action, CenterTab, PassKind, SeatUiState, TITLE_LINES, action_for, curtain_view,
        format_own_mandate_line, handle_seat_key, neutral_session_view, render, render_seat,
        render_seat_with_commodities, render_turn_screen, title_view,
    };

    fn key(code: SeatKeyCode, modifiers: SeatKeyModifiers) -> SeatKeyEvent {
        SeatKeyEvent::new(code, modifiers)
    }

    fn sample_seat_observation() -> SeatObservation {
        let spec = PublicTableSpec {
            table_id: TableId(202_611),
            mode: TableMode::Club,
            seat_count: 4,
            seats: vec![SeatConfig {
                seat: 0,
                identity: SeatIdentity::Human,
                initial_cred: 15_000,
                raw_yield_per_turn: 10_000,
                refined_project_value: 2_000,
                upkeep_good: 3,
                upkeep_amount: 1_800,
                initial_raw_tariff_bps: 800,
                initial_refined_tariff_bps: 600,
                perishable_decay_bps: 2_000,
                regent: Some(RegentDoctrine {
                    risk_tolerance_bps: 5_000,
                    min_raw_reserve: 1_000,
                    auto_cash_due_claims: true,
                }),
                converters: Vec::new(),
            }],
            projects: Vec::new(),
            closing_hazard_bps: [3_500; 8],
            mandate_deck: vec![GradedMandateCard {
                card_id: 1,
                kind: MandateKind::MaintainCleanCredit {
                    min_tier: CreditTier::Bbb,
                },
                difficulty_completion_bps: 10_000,
                bonus_equity: 3_000,
            }],
            drand_schedule: None,
            eval_vrf_pubkey: None,
            town_import_premium_bps: 22_500,
            works_stipend_per_unit_cred: 350,
            mandate_alpha_bps: 250,
            cancel_lock_bps: 2_500,
            wash_round_trip_min_units: 1_000,
            wash_freeze_bond_cred: 500,
            wash_freeze_bounty_cred: 500,
        };
        let predicates = SeatStatePredicates {
            version: STATE_PREDICATES_VERSION,
            table_id: TableId(202_611),
            turn: 1,
            phase: TurnPhase::TalkAndMarket,
            seat: 0,
            upkeep: UpkeepAndDebt {
                upkeep_good: 3,
                owed_units: 1_800,
                held_units: 0,
                held_on_own_resting_sells: 0,
                shortfall_units: 1_800,
                debt_added_at_sweep: 3_600,
                interest_bps: 400,
            },
            spoilage: SpoilageAndConversion {
                own_raw_good: 0,
                own_raw_yield_at_sweep: 10_000,
                spoilage_bps_by_good: vec![2_000; 4],
                spoil_units_held_by_good: vec![2_000, 0, 0, 0],
                spoil_units_on_resting_offers: 0,
                spoil_units_total: 2_000,
                workshop_capacity_runs_left: 4,
                recipes: Vec::new(),
            },
            scarcity: vec![GoodScarcity {
                good: AssetId::Raw(0),
                producer_seat: 0,
                own_units: 0,
                table_units_held: 0,
                open_book_units_offered: 0,
                open_book_units_bid: 0,
                domain_book_units_offered: 0,
                domain_book_units_bid: 0,
                seats_needing: 2,
                open_book_tariff_bps: 800,
            }],
            coalitions: vec![CoalitionReach {
                members: vec![0, 1],
                upkeep_goods_makeable: Vec::new(),
                upkeep_goods_not_makeable: vec![3, 0],
            }],
            season_end: SeasonEndAndMarks {
                season_closed: false,
                turns_until_earliest_close: 23,
                turns_until_latest_close: 31,
                holdings: Vec::new(),
                projects: Vec::new(),
            },
            cancel_locked: false,
        };
        let _ = (
            ActiveShock {
                target_seat: 0,
                kind: ShockKind::DemandSurge,
                magnitude_bps: 0,
                expires_after_turn: 1,
            },
            CharterRole::OracleAuditor,
            CommitmentHash([0xAA; 32]),
            COMMODITY_DISPLAY_CATALOG.len(),
        );
        SeatObservation::unmandated(SeatObservationPublicParts {
            seat: 0,
            table_id: TableId(202_611),
            turn: 1,
            phase: TurnPhase::TalkAndMarket,
            cancel_locked: false,
            trade_minister: 0,
            cartel_chair: None,
            spec,
            parities: FundamentalParities {
                cred_per_unit: 1_000,
                raw_per_unit: vec![1_000; 4],
                refined_per_unit: vec![2_000; 4],
            },
            seats: vec![SeatState {
                seat: 0,
                identity: SeatIdentity::Human,
                cred: 15_000,
                cred_debt: 0,
                raw: vec![0; 4],
                refined: vec![0; 4],
                project_equity: 0,
                raw_tariff_bps: 800,
                refined_tariff_bps: 600,
                credit_tier: CreditTier::Aaa,
                hard_credit_defaults: 0,
                bond_breaches: 0,
                broken_promises: 0,
                honor_penalties: 0,
                consecutive_misses: 0,
                consecutive_zero_production_turns: 0,
                wash_frozen_until_turn: 0,
            }],
            offers: BTreeMap::new(),
            checks: BTreeMap::new(),
            escrows: BTreeMap::new(),
            dual_escrows: BTreeMap::new(),
            pledges: BTreeMap::new(),
            audited_pledges: BTreeSet::new(),
            domains: Vec::new(),
            council_platforms: BTreeMap::new(),
            charters: Vec::new(),
            active_shocks: Vec::new(),
            public_forecasts: Vec::new(),
            auditor_hints: vec![AuditorShockHint {
                hint_turn: 1,
                forecast_turn: 2,
                realize_turn: 3,
                target_seat: 1,
                kind: ShockKind::DemandSurge,
            }],
            next_offer_id: 1,
            next_check_id: 1,
            next_escrow_id: 1,
            delivery_ledger: BTreeMap::new(),
            last_potentials: vec![15_000; 4],
            pending_town_import_cred_spent: 0,
            fill_log: Vec::new(),
            pending_wash_freezes: Vec::new(),
            eval_vrf_pubkey: None,
            baseline_predicates: predicates,
            public_receipts: Vec::new(),
        })
    }

    #[test]
    fn q_esc_and_ctrl_c_quit_and_others_ignore() {
        assert_eq!(
            action_for(key(SeatKeyCode::Char('q'), SeatKeyModifiers::NONE)),
            Action::Quit
        );
        assert_eq!(
            action_for(key(SeatKeyCode::Esc, SeatKeyModifiers::NONE)),
            Action::Quit
        );
        assert_eq!(
            action_for(key(SeatKeyCode::Char('c'), SeatKeyModifiers::CTRL)),
            Action::Quit
        );
        assert_eq!(
            action_for(key(SeatKeyCode::Char('c'), SeatKeyModifiers::NONE)),
            Action::Ignore
        );
        assert_eq!(
            action_for(key(SeatKeyCode::Enter, SeatKeyModifiers::NONE)),
            Action::Ignore
        );
    }

    #[test]
    fn the_title_screen_says_testnet_and_not_playable() {
        let view = title_view();
        let main = view
            .zone(ViewZoneId::SingleFull)
            .unwrap_or_else(|| panic!("SingleFull zone missing"));
        assert_eq!(main.alignment, ViewAlign::Center);
        assert!(main.bordered);
        let screen = view.plain_text();
        assert!(screen.contains(TITLE_LINES[0]));
        assert!(screen.contains(TITLE_LINES[1]));
        assert!(screen.contains(TITLE_LINES[2]));
    }

    #[test]
    fn render_seat_and_commodity_display_render_cleanly() {
        let mandate_line = format_own_mandate_line(Some(&PrivateMandate {
            kind: MandateKind::MinProjectEquity { target: 28_000 },
            bonus_equity: 7_250,
        }));
        assert!(mandate_line.contains("MinProjectEquity"));
        assert!(mandate_line.contains("+7250 eq"));

        let obs_unmandated = sample_seat_observation();
        let screen = render_seat(&obs_unmandated).plain_text();
        assert!(screen.contains("Seat 0 View"));
        assert!(screen.contains("Table #202611 | Turn 1"));
        assert!(screen.contains("Clearinghouse Marks [CRED]: 15000"));
        assert!(screen.contains("Own Mandate: none attached"));

        let cmap = CommodityDisplayMap {
            numeraire_code: NUMERAIRE_CODE.to_owned(),
            numeraire_label: NUMERAIRE_DISPLAY_LABEL.to_owned(),
            seats: vec![
                SeatCommodityDisplay {
                    seat: 0,
                    raw_code: "TMB".to_owned(),
                    raw_label: "Rough Timber [TMB]".to_owned(),
                    refined_code: "TMB".to_owned(),
                    refined_label: "Seasoned Timber [TMB]".to_owned(),
                },
                SeatCommodityDisplay {
                    seat: 1,
                    raw_code: "SLT".to_owned(),
                    raw_label: "Coarse Pan Salt [SLT]".to_owned(),
                    refined_code: "SLT".to_owned(),
                    refined_label: "Cured Fine Salt [SLT]".to_owned(),
                },
                SeatCommodityDisplay {
                    seat: 2,
                    raw_code: "OAK".to_owned(),
                    raw_label: "Green Oak Heartwood [OAK]".to_owned(),
                    refined_code: "OAK".to_owned(),
                    refined_label: "Seasoned Cooper Oak [OAK]".to_owned(),
                },
                SeatCommodityDisplay {
                    seat: 3,
                    raw_code: "IRN".to_owned(),
                    raw_label: "Pig Iron Bloom [IRN]".to_owned(),
                    refined_code: "IRN".to_owned(),
                    refined_label: "Wrought Guild Iron [IRN]".to_owned(),
                },
            ],
        };
        let obs_without = sample_seat_observation();
        let named = render_seat_with_commodities(&obs_without, &cmap, false).plain_text();
        assert!(named.contains("Wrought Guild Iron [IRN]"));
        assert!(named.contains("Own Mandate: none attached"));

        let plain = render_seat_with_commodities(&obs_without, &cmap, true).plain_text();
        assert!(plain.contains("CRED: 15000"));
        assert!(plain.contains("Upkeep Good: IRN"));

        let mut obs_empty = sample_seat_observation();
        obs_empty.seats.clear();
        obs_empty.spec.seats.clear();
        let empty_screen = render_seat(&obs_empty).plain_text();
        assert!(empty_screen.contains("Tier: Unknown"));
        assert!(empty_screen.contains("Upkeep Good: None"));
    }

    #[test]
    fn turn_screens_cover_responsive_viewports_and_four_zones() {
        let obs = sample_seat_observation();
        let state = SeatUiState::new();

        let wide = render_turn_screen(
            &obs,
            &state,
            PassKind::SubPassA,
            None,
            ViewportSize::new(100, 30),
        );
        assert_eq!(wide.layout.mode, ViewLayoutMode::FourZoneWide);
        assert!(wide.zone(ViewZoneId::HeaderBar).is_some());
        assert!(wide.zone(ViewZoneId::PublicLedger).is_some());
        assert!(wide.zone(ViewZoneId::CenterTabs).is_some());
        assert!(wide.zone(ViewZoneId::PrivateDrawer).is_some());
        let wide_txt = wide.plain_text();
        assert!(wide_txt.contains("Clearinghouse Marks [CRED]"));
        assert!(wide_txt.contains("Diplomatic Desk (Disabled)"));
        assert!(wide_txt.contains("OracleAuditor Hints"));
        assert!(wide_txt.contains("T1->T2->T3 S1"));

        let stacked = render_turn_screen(
            &obs,
            &state,
            PassKind::SubPassB,
            None,
            ViewportSize::new(80, 24),
        );
        assert_eq!(stacked.layout.mode, ViewLayoutMode::StackedFallback);
        assert_eq!(stacked.zones.len(), 4);

        let small = render_turn_screen(
            &obs,
            &state,
            PassKind::SubPassA,
            None,
            ViewportSize::new(79, 23),
        );
        assert_eq!(small.layout.mode, ViewLayoutMode::TooSmall);
        assert!(small.plain_text().contains("Terminal too small"));

        let ignored = handle_seat_key(
            &obs,
            state.clone(),
            PassKind::SubPassA,
            None,
            ViewportSize::new(100, 30),
            key(SeatKeyCode::Char('4'), SeatKeyModifiers::NONE),
        );
        assert!(ignored.intent.is_none());
        assert_eq!(ignored.state.active_tab(), CenterTab::MarketAndContracts);

        let curtain_unarmed = curtain_view(
            TableId(202_611),
            Turn(3),
            PassKind::SubPassB,
            Seat(1),
            ViewportSize::new(100, 30),
            false,
            CurtainPinPrompt::default(),
        );
        let c_txt = curtain_unarmed.plain_text();
        assert!(c_txt.contains("Table #202611 | Turn 3"));
        assert!(c_txt.contains("Hand keyboard to Seat 1 (Boreal Guild)"));
        assert!(c_txt.contains("press [Space] then [Enter]"));

        let curtain_armed = curtain_view(
            TableId(202_611),
            Turn(3),
            PassKind::SubPassB,
            Seat(1),
            ViewportSize::new(100, 30),
            true,
            CurtainPinPrompt::default(),
        );
        assert!(curtain_armed.plain_text().contains("HAND-OFF ARMED"));
    }

    #[test]
    fn market_tab_builds_all_ten_variants_and_blocks_cancel_offer_when_locked() {
        let mut obs = sample_seat_observation();
        let mut state = SeatUiState::new();
        let vp = ViewportSize::new(100, 30);
        let mut seen = Vec::new();
        for _ in 0..10 {
            let step = handle_seat_key(
                &obs,
                state.clone(),
                PassKind::SubPassA,
                None,
                vp,
                key(SeatKeyCode::Enter, SeatKeyModifiers::NONE),
            );
            let intent = step
                .intent
                .unwrap_or_else(|| panic!("expected MarketAction intent on Enter"));
            assert_eq!(intent.kind, SeatIntentKind::SubmitMarketAction);
            let act = intent
                .market_action
                .unwrap_or_else(|| panic!("expected market_action payload"));
            seen.push(match act {
                MarketAction::PostOffer { .. } => "PostOffer",
                MarketAction::CancelOffer { .. } => "CancelOffer",
                MarketAction::CreateCheck { .. } => "CreateCheck",
                MarketAction::CancelCheck { .. } => "CancelCheck",
                MarketAction::CreateEscrow { .. } => "CreateEscrow",
                MarketAction::CancelExpiredEscrow { .. } => "CancelExpiredEscrow",
                MarketAction::RecordIntentPledge { .. } => "RecordIntentPledge",
                MarketAction::StandForOffice { .. } => "StandForOffice",
                MarketAction::FlagWashFreeze { .. } => "FlagWashFreeze",
                MarketAction::TownImport { .. } => "TownImport",
            });
            state = handle_seat_key(
                &obs,
                step.state,
                PassKind::SubPassA,
                None,
                vp,
                key(SeatKeyCode::Char(']'), SeatKeyModifiers::NONE),
            )
            .state;
        }
        assert_eq!(
            seen,
            vec![
                "PostOffer",
                "CancelOffer",
                "CreateCheck",
                "CancelCheck",
                "CreateEscrow",
                "CancelExpiredEscrow",
                "RecordIntentPledge",
                "StandForOffice",
                "FlagWashFreeze",
                "TownImport",
            ]
        );

        obs.cancel_locked = true;
        let locked_step = handle_seat_key(
            &obs,
            handle_seat_key(
                &obs,
                SeatUiState::new(),
                PassKind::SubPassB,
                None,
                vp,
                key(SeatKeyCode::Char(']'), SeatKeyModifiers::NONE),
            )
            .state,
            PassKind::SubPassB,
            None,
            vp,
            key(SeatKeyCode::Enter, SeatKeyModifiers::NONE),
        );
        assert!(locked_step.intent.is_none());
        assert!(
            locked_step
                .view
                .plain_text()
                .contains("CancelOffer is BLOCKED")
        );
    }

    #[test]
    fn bundle_tab_gates_vote_to_4n_charters_to_6n_and_builds_all_ten_fields() {
        let mut obs = sample_seat_observation();
        let vp = ViewportSize::new(100, 30);

        let go_tab2 = |o: &SeatObservation| {
            handle_seat_key(
                o,
                SeatUiState::new(),
                PassKind::SubPassB,
                None,
                vp,
                key(SeatKeyCode::Char('2'), SeatKeyModifiers::NONE),
            )
        };

        obs.turn = 1;
        let t1 = go_tab2(&obs).view.plain_text();
        assert!(!t1.contains("Council Vote (Turn 4n)"));
        assert!(!t1.contains("Charter Bids (Turn 6n)"));

        obs.turn = 4;
        let t4 = go_tab2(&obs).view.plain_text();
        assert!(t4.contains("Council Vote (Turn 4n)"));
        assert!(!t4.contains("Charter Bids (Turn 6n)"));

        obs.turn = 6;
        let t6 = go_tab2(&obs).view.plain_text();
        assert!(!t6.contains("Council Vote (Turn 4n)"));
        assert!(t6.contains("Charter Bids (Turn 6n)"));

        obs.turn = 12;
        let mut state = go_tab2(&obs).state;
        let t12 = render_turn_screen(&obs, &state, PassKind::SubPassB, None, vp).plain_text();
        assert!(t12.contains("Council Vote (Turn 4n)"));
        assert!(t12.contains("Charter Bids (Turn 6n)"));

        let mut staged_kinds = 0_usize;
        for _ in 0..10 {
            let step = handle_seat_key(
                &obs,
                state.clone(),
                PassKind::SubPassB,
                None,
                vp,
                key(SeatKeyCode::Enter, SeatKeyModifiers::NONE),
            );
            let intent = step
                .intent
                .unwrap_or_else(|| panic!("expected UpdateBundle intent"));
            assert_eq!(intent.kind, SeatIntentKind::UpdateBundle);
            assert!(intent.bundle_action.is_some());
            staged_kinds += 1;
            state = handle_seat_key(
                &obs,
                step.state,
                PassKind::SubPassB,
                None,
                vp,
                key(SeatKeyCode::Char(']'), SeatKeyModifiers::NONE),
            )
            .state;
        }
        assert_eq!(staged_kinds, 10);

        let cleared = handle_seat_key(
            &obs,
            state,
            PassKind::SubPassB,
            None,
            vp,
            key(SeatKeyCode::Char('X'), SeatKeyModifiers::NONE),
        );
        assert_eq!(
            cleared.intent.and_then(|i| i.bundle_action),
            Some(BundleDraftAction::ClearAll)
        );
    }

    #[test]
    fn what_if_tab_chronicle_30_fields_handoff_clear_and_scrubber_preservation() {
        let mut obs = sample_seat_observation();
        obs.public_receipts.push(PublicSweepReceipt {
            turn: 1,
            non_reveal_seats: vec![2],
            regent_acted_seats: vec![2],
            converted_to_ai: Vec::new(),
            escrows_finished: vec![4],
            escrows_cancelled_or_forfeited: Vec::new(),
            atomic_swaps_settled: 1,
            converter_batches_run: 2,
            converter_orders_clamped: 0,
            checks_cashed: vec![1],
            checks_bounced: Vec::new(),
            cartel_evictions: Vec::new(),
            projects_completed: 1,
            project_orders_clamped: 0,
            total_upkeep_shortfall_debt: 500,
            total_spoilage_clawed_back: 250,
            broken_promises: Vec::new(),
            elected_officers: None,
            charters_awarded: Vec::new(),
            public_forecasts: Vec::new(),
            newly_activated_shocks: Vec::new(),
            cred_debt_repaid: 100,
            charter_cred_burned: 0,
            town_import_cred_spent: 200,
            works_stipend_cred_minted: 300,
            batch_trades_cleared: 2,
            fills: Vec::new(),
            wash_freeze_settlements: Vec::new(),
            season_closed: false,
            pbrs_deltas: vec![10, -5, 0, -5],
        });
        let vp = ViewportSize::new(100, 30);

        let mut state = handle_seat_key(
            &obs,
            SeatUiState::new(),
            PassKind::SubPassA,
            None,
            vp,
            key(SeatKeyCode::Char('3'), SeatKeyModifiers::NONE),
        )
        .state;
        state = handle_seat_key(
            &obs,
            state,
            PassKind::SubPassA,
            None,
            vp,
            key(SeatKeyCode::Char(' '), SeatKeyModifiers::NONE),
        )
        .state;
        let preview_step = handle_seat_key(
            &obs,
            state,
            PassKind::SubPassA,
            None,
            vp,
            key(SeatKeyCode::Char('p'), SeatKeyModifiers::NONE),
        );
        let req_intent = preview_step
            .intent
            .unwrap_or_else(|| panic!("expected RequestWhatIf intent"));
        assert_eq!(req_intent.kind, SeatIntentKind::RequestWhatIf);
        assert_eq!(
            req_intent.what_if.as_ref().map(|r| r.tie_mode),
            Some(PreviewTieMode::PessimisticForSeat)
        );

        state = preview_step.state;
        let seat_state_stub = obs.seats[0].clone();
        state.set_what_if_preview(Some(WhatIfPreview {
            seat: 0,
            turn: 1,
            baseline_predicates: obs.baseline_predicates.clone(),
            pre_seat_state: seat_state_stub.clone(),
            post_market_seat_state: seat_state_stub.clone(),
            post_sweep_seat_state: seat_state_stub,
            post_sweep_predicates: obs.baseline_predicates.clone(),
            close_mark_caveat: league_seat_types::HYPOTHETICAL_CLOSE_MARK_CAVEAT.to_owned(),
            refused_market_actions: vec![RefusedActionPreview {
                action_index: 0,
                error_code: "CANCEL_LOCKED".to_owned(),
                message: "rejected sEd7N5k8mQp2rT9vW3xY6zA4bC1d and OPENROUTER_API_KEY=secret_tok"
                    .to_owned(),
            }],
            converter_orders: Vec::new(),
            project_orders: Vec::new(),
            swap_orders: Vec::new(),
            check_orders: Vec::new(),
            escrow_orders: Vec::new(),
            upkeep: SeatUpkeepPreview {
                upkeep_good: 3,
                upkeep_required: 1800,
                paid_with_refined: 1800,
                debt_interest_accrued: 0,
                new_shortfall_debt: 0,
                debt_repaid_cred: 0,
            },
            spoilage: SeatSpoilagePreview {
                perishable_spoiled_units: 0,
            },
            public_fills: Vec::new(),
            step0_to_7_receipt: Step0To7SweepSummary::from_public_sweep_receipt(
                &obs.public_receipts[0],
            ),
        }));
        let what_if_txt =
            render_turn_screen(&obs, &state, PassKind::SubPassA, None, vp).plain_text();
        assert!(what_if_txt.contains("[REDACTED]"));
        assert!(!what_if_txt.contains("sEd7N5k8mQp2rT9vW3xY6zA4bC1d"));
        assert!(!what_if_txt.contains("secret_tok"));

        // Verify all 61 glossary phrases + numeraire render without false-positive redaction.
        for entry in COMMODITY_DISPLAY_CATALOG {
            let raw = entry.raw_display(false);
            let refined = entry.refined_display(false);
            assert_eq!(scrub_secret_text(&raw), raw);
            assert_eq!(scrub_secret_text(&refined), refined);
        }
        assert_eq!(
            scrub_secret_text(format_numeraire_display(false)),
            format_numeraire_display(false)
        );

        // Open Chronicle modal and verify all 30 seat-safe fields appear.
        let modal_step = handle_seat_key(
            &obs,
            state,
            PassKind::SubPassA,
            None,
            vp,
            key(SeatKeyCode::Char('r'), SeatKeyModifiers::NONE),
        );
        assert_eq!(
            modal_step.intent.as_ref().map(|i| i.kind),
            Some(SeatIntentKind::OpenChronicleModal)
        );
        let modal_txt = modal_step.view.plain_text();
        for field_key in [
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
                modal_txt.contains(field_key),
                "Chronicle modal missing field `{field_key}`"
            );
        }

        // Verify hand-off clears free text while keeping active_tab + selection.
        let mut s = modal_step.state;
        s = handle_seat_key(
            &obs,
            handle_seat_key(
                &obs,
                s,
                PassKind::SubPassA,
                None,
                vp,
                key(SeatKeyCode::Char('r'), SeatKeyModifiers::NONE),
            )
            .state,
            PassKind::SubPassA,
            None,
            vp,
            key(SeatKeyCode::Char('n'), SeatKeyModifiers::NONE),
        )
        .state;
        s = handle_seat_key(
            &obs,
            s,
            PassKind::SubPassA,
            None,
            vp,
            key(SeatKeyCode::Char('X'), SeatKeyModifiers::NONE),
        )
        .state;
        s = handle_seat_key(
            &obs,
            s,
            PassKind::SubPassA,
            None,
            vp,
            key(SeatKeyCode::Enter, SeatKeyModifiers::NONE),
        )
        .state;
        assert_eq!(s.notes_draft(), "X");
        let saved_tab = s.active_tab();
        let saved_sel = s.selection();
        s.clear_for_hand_off();
        assert_eq!(s.composer_draft(), "");
        assert_eq!(s.whisper_draft(), "");
        assert_eq!(s.notes_draft(), "");
        assert_eq!(s.active_tab(), saved_tab);
        assert_eq!(s.selection(), saved_sel);
        assert!(s.what_if_preview().is_none());
        assert_eq!(s.staged_bundle(), &SealedOrderBundle::default());
    }

    #[test]
    fn composer_navigation_field_adjustments_and_ledger_rows_exercise_all_branches() {
        let mut base = sample_seat_observation();
        let parts = SeatObservationPublicParts {
            seat: 0,
            table_id: TableId(202_611),
            turn: 24,
            phase: TurnPhase::TalkAndMarket,
            cancel_locked: false,
            trade_minister: 0,
            cartel_chair: Some(1),
            spec: PublicTableSpec {
                seats: vec![
                    SeatConfig {
                        seat: 0,
                        identity: SeatIdentity::Human,
                        initial_cred: 15_000,
                        raw_yield_per_turn: 10_000,
                        refined_project_value: 2_000,
                        upkeep_good: 3,
                        upkeep_amount: 1_800,
                        initial_raw_tariff_bps: 800,
                        initial_refined_tariff_bps: 600,
                        perishable_decay_bps: 2_000,
                        regent: Some(RegentDoctrine {
                            risk_tolerance_bps: 5_000,
                            min_raw_reserve: 1_000,
                            auto_cash_due_claims: true,
                        }),
                        converters: Vec::new(),
                    },
                    SeatConfig {
                        seat: 1,
                        identity: SeatIdentity::AiSyndicate,
                        initial_cred: 15_000,
                        raw_yield_per_turn: 10_000,
                        refined_project_value: 2_000,
                        upkeep_good: 0,
                        upkeep_amount: 1_800,
                        initial_raw_tariff_bps: 800,
                        initial_refined_tariff_bps: 600,
                        perishable_decay_bps: 2_000,
                        regent: None,
                        converters: Vec::new(),
                    },
                ],
                ..base.spec.clone()
            },
            parities: base.parities.clone(),
            seats: vec![
                SeatState {
                    raw: vec![500, 0, 0, 0],
                    refined: vec![0, 200, 0, 0],
                    wash_frozen_until_turn: 25,
                    ..base.seats[0].clone()
                },
                SeatState {
                    seat: 1,
                    identity: SeatIdentity::AiSyndicate,
                    credit_tier: CreditTier::D,
                    ..base.seats[0].clone()
                },
            ],
            offers: BTreeMap::from([(
                1,
                CredOffer {
                    offer_id: 1,
                    maker: 1,
                    side: OfferSide::Sell,
                    good: AssetId::Raw(1),
                    remaining_units: 800,
                    price_per_unit_cred: 1_100,
                    locked_cred: 0,
                    domain_id: Some(0),
                    is_hybrid: true,
                    created_turn: 23,
                },
            )]),
            checks: BTreeMap::from([(
                1,
                CreditCheck {
                    check_id: 1,
                    drawer: 1,
                    payee: 0,
                    asset: AssetId::Cred,
                    send_max: 1_000,
                    deliver_min: 500,
                    due_turn: 24,
                    expiration_turn: 27,
                    status: CheckStatus::Open,
                },
            )]),
            escrows: BTreeMap::from([(
                1,
                TokenEscrowVault {
                    escrow_id: 1,
                    sender: 1,
                    destination: 0,
                    amount_cred: 1_500,
                    finish_after_turn: Some(24),
                    cancel_after_turn: 28,
                    condition_hash: Some(CommitmentHash([0x5A; 32])),
                    bond: Some(BondSpec {
                        obligor: 0,
                        beneficiary: 1,
                        good: AssetId::Refined(0),
                        required_units: 400,
                        baseline_delivered: 0,
                        deadline_turn: 26,
                        mode: BondSettlementMode::BuyerDvpPayment,
                    }),
                    status: EscrowStatus::Locked,
                },
            )]),
            dual_escrows: BTreeMap::from([(
                2,
                DualEscrowVault {
                    escrow_id: 2,
                    buyer: 0,
                    seller: 1,
                    good: AssetId::Refined(1),
                    required_units: 600,
                    payment_cred: 1_200,
                    buyer_bond_cred: 300,
                    seller_bond_cred: 300,
                    baseline_delivered: 0,
                    deadline_turn: 26,
                    domain_id: None,
                    status: EscrowStatus::Locked,
                },
            )]),
            pledges: BTreeMap::from([
                (
                    1,
                    IntentPledge {
                        pledge_id: 1,
                        promisor: 0,
                        counterparty: 1,
                        target: IntentTarget::VoteForSeat {
                            office_turn: 24,
                            candidate: 1,
                        },
                        honor_weight: 200,
                        memo_digest: [0x11; 32],
                    },
                ),
                (
                    2,
                    IntentPledge {
                        pledge_id: 2,
                        promisor: 1,
                        counterparty: 0,
                        target: IntentTarget::PreserveCartelMembership {
                            office_turn: 24,
                            domain_id: 0,
                            beneficiary: 0,
                        },
                        honor_weight: 200,
                        memo_digest: [0x22; 32],
                    },
                ),
                (
                    3,
                    IntentPledge {
                        pledge_id: 3,
                        promisor: 0,
                        counterparty: 1,
                        target: IntentTarget::RevealPreimage {
                            escrow_id: 1,
                            condition: CommitmentHash([0x5A; 32]),
                        },
                        honor_weight: 200,
                        memo_digest: [0x33; 32],
                    },
                ),
            ]),
            audited_pledges: BTreeSet::from([1]),
            domains: vec![CartelDomain {
                domain_id: 0,
                members: BTreeSet::from([0, 1]),
            }],
            council_platforms: BTreeMap::from([(
                0,
                CouncilPlatform {
                    proposed_tariffs_bps: vec![(0, (800, 600))],
                    cartel_admissions: vec![(0, 1)],
                    cartel_revocations: Vec::new(),
                },
            )]),
            charters: vec![ActiveCharter {
                role: CharterRole::OracleAuditor,
                holder: 0,
                expires_at_turn: 30,
            }],
            active_shocks: vec![ActiveShock {
                target_seat: 1,
                kind: ShockKind::DemandSurge,
                magnitude_bps: 1_500,
                expires_after_turn: 25,
            }],
            public_forecasts: vec![ShockForecastEvent {
                target_seat: 0,
                kind: ShockKind::SupplyFreeze,
                realize_turn: 26,
            }],
            auditor_hints: base.auditor_hints.clone(),
            next_offer_id: 2,
            next_check_id: 2,
            next_escrow_id: 3,
            delivery_ledger: BTreeMap::new(),
            last_potentials: base.last_potentials.clone(),
            pending_town_import_cred_spent: 0,
            fill_log: Vec::new(),
            pending_wash_freezes: Vec::new(),
            eval_vrf_pubkey: None,
            baseline_predicates: SeatStatePredicates {
                spoilage: SpoilageAndConversion {
                    recipes: vec![RecipeCover {
                        recipe_idx: 0,
                        output_good: 0,
                        output_units_per_batch: 1_000,
                        capacity_runs_left: 2,
                        batches_covered_by_inputs: 1,
                    }],
                    ..base.baseline_predicates.spoilage.clone()
                },
                scarcity: vec![GoodScarcity {
                    good: AssetId::Raw(0),
                    producer_seat: 0,
                    own_units: 500,
                    table_units_held: 500,
                    open_book_units_offered: 100,
                    open_book_units_bid: 50,
                    domain_book_units_offered: 0,
                    domain_book_units_bid: 0,
                    seats_needing: 2,
                    open_book_tariff_bps: 800,
                }],
                season_end: SeasonEndAndMarks {
                    holdings: vec![HoldingMark {
                        good: AssetId::Refined(0),
                        units: 200,
                        marked_value: 400,
                        units_marked_by_bids: 100,
                        units_at_floor: 100,
                        value_at_floor: 100,
                    }],
                    ..base.baseline_predicates.season_end.clone()
                },
                ..base.baseline_predicates.clone()
            },
            public_receipts: vec![
                PublicSweepReceipt {
                    turn: 22,
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
                    public_forecasts: Vec::new(),
                    newly_activated_shocks: Vec::new(),
                    cred_debt_repaid: 0,
                    charter_cred_burned: 0,
                    town_import_cred_spent: 0,
                    works_stipend_cred_minted: 0,
                    batch_trades_cleared: 1,
                    fills: vec![FillRecord {
                        turn: 22,
                        kind: FillKind::BatchFill,
                        good: Some(AssetId::Raw(0)),
                        from: Some(0),
                        to: 1,
                        units: 500,
                        net_units: 460,
                        cred_amount: 500,
                        unit_price_cred: Some(1_000),
                    }],
                    wash_freeze_settlements: Vec::new(),
                    season_closed: false,
                    pbrs_deltas: vec![0; 4],
                },
                PublicSweepReceipt {
                    turn: 23,
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
                    public_forecasts: Vec::new(),
                    newly_activated_shocks: Vec::new(),
                    cred_debt_repaid: 0,
                    charter_cred_burned: 0,
                    town_import_cred_spent: 0,
                    works_stipend_cred_minted: 0,
                    batch_trades_cleared: 0,
                    fills: Vec::new(),
                    wash_freeze_settlements: Vec::new(),
                    season_closed: false,
                    pbrs_deltas: vec![0; 4],
                },
            ],
        };
        let obs = SeatObservation::unmandated(parts);
        let vp = ViewportSize::new(100, 30);
        let mut s = SeatUiState::new();
        let press =
            |o: &SeatObservation, st: SeatUiState, code: SeatKeyCode, mods: SeatKeyModifiers| {
                handle_seat_key(o, st, PassKind::SubPassA, None, vp, key(code, mods))
            };

        // 1. Curtain at < 80x24 + Global/Zone scrolling + Tab/BackTab + plain_codes toggle + Quits.
        let small_curtain = curtain_view(
            TableId(202_611),
            Turn(24),
            PassKind::SubPassA,
            Seat(1),
            ViewportSize::new(60, 15),
            false,
            CurtainPinPrompt::default(),
        );
        assert_eq!(small_curtain.layout.mode, ViewLayoutMode::TooSmall);
        for (c, mods, expected_kind) in [
            (
                SeatKeyCode::Char('q'),
                SeatKeyModifiers::NONE,
                SeatIntentKind::Quit,
            ),
            (
                SeatKeyCode::Esc,
                SeatKeyModifiers::NONE,
                SeatIntentKind::Quit,
            ),
            (
                SeatKeyCode::Char('c'),
                SeatKeyModifiers::CTRL,
                SeatIntentKind::Quit,
            ),
            (
                SeatKeyCode::Char('l'),
                SeatKeyModifiers::CTRL,
                SeatIntentKind::LockAndHandOff,
            ),
            (
                SeatKeyCode::Char('L'),
                SeatKeyModifiers::CTRL,
                SeatIntentKind::LockAndHandOff,
            ),
        ] {
            let step = press(&obs, s.clone(), c, mods);
            assert_eq!(step.intent.map(|i| i.kind), Some(expected_kind));
        }
        assert!(
            press(
                &obs,
                s.clone(),
                SeatKeyCode::Char('L'),
                SeatKeyModifiers::NONE
            )
            .intent
            .is_none()
        );

        for zone_steps in 0..2 {
            s = press(&obs, s, SeatKeyCode::F(6), SeatKeyModifiers::NONE).state;
            for kc in [
                SeatKeyCode::Down,
                SeatKeyCode::PageDown,
                SeatKeyCode::Up,
                SeatKeyCode::PageUp,
                SeatKeyCode::Home,
            ] {
                s = press(&obs, s, kc, SeatKeyModifiers::NONE).state;
            }
            let _ = zone_steps;
        }
        s = press(&obs, s, SeatKeyCode::F(6), SeatKeyModifiers::NONE).state; // back to CenterTabs
        s = press(&obs, s, SeatKeyCode::Char('c'), SeatKeyModifiers::NONE).state; // plain_codes=true
        s = press(&obs, s, SeatKeyCode::Char('c'), SeatKeyModifiers::NONE).state; // plain_codes=false
        s = press(&obs, s, SeatKeyCode::Tab, SeatKeyModifiers::NONE).state;
        s = press(&obs, s, SeatKeyCode::BackTab, SeatKeyModifiers::NONE).state;

        // 2. Tab 1: cycle all 10 MarketActionKind variants and adjust every field + and - + Submit.
        for _ in 0..10 {
            for _ in 0..8 {
                s = press(&obs, s, SeatKeyCode::Char('+'), SeatKeyModifiers::NONE).state;
                s = press(&obs, s, SeatKeyCode::Char('-'), SeatKeyModifiers::NONE).state;
                s = press(&obs, s, SeatKeyCode::Down, SeatKeyModifiers::NONE).state;
            }
            for _ in 0..8 {
                s = press(&obs, s, SeatKeyCode::Up, SeatKeyModifiers::NONE).state;
            }
            s = press(&obs, s, SeatKeyCode::PageDown, SeatKeyModifiers::NONE).state;
            s = press(&obs, s, SeatKeyCode::PageUp, SeatKeyModifiers::NONE).state;
            s = press(&obs, s, SeatKeyCode::Home, SeatKeyModifiers::NONE).state;
            let step = press(&obs, s, SeatKeyCode::Enter, SeatKeyModifiers::NONE);
            assert_eq!(
                step.intent.as_ref().map(|i| i.kind),
                Some(SeatIntentKind::SubmitMarketAction)
            );
            s = press(
                &obs,
                step.state,
                SeatKeyCode::Char(']'),
                SeatKeyModifiers::NONE,
            )
            .state;
        }
        s = press(&obs, s, SeatKeyCode::Char('['), SeatKeyModifiers::NONE).state;

        // 3. Tab 2 on Turn 12 (all 10 BundleSections visible): adjust every field + and - + Stage/Clear.
        base.turn = 12;
        s = press(&base, s, SeatKeyCode::Char('2'), SeatKeyModifiers::NONE).state;
        for _ in 0..10 {
            for _ in 0..10 {
                s = press(&base, s, SeatKeyCode::Right, SeatKeyModifiers::NONE).state;
                s = press(&base, s, SeatKeyCode::Left, SeatKeyModifiers::NONE).state;
                s = press(&base, s, SeatKeyCode::Down, SeatKeyModifiers::NONE).state;
            }
            for _ in 0..10 {
                s = press(&base, s, SeatKeyCode::Up, SeatKeyModifiers::NONE).state;
            }
            s = press(&base, s, SeatKeyCode::PageDown, SeatKeyModifiers::NONE).state;
            s = press(&base, s, SeatKeyCode::PageUp, SeatKeyModifiers::NONE).state;
            s = press(&base, s, SeatKeyCode::Home, SeatKeyModifiers::NONE).state;
            let stage_step = press(&base, s, SeatKeyCode::Enter, SeatKeyModifiers::NONE);
            assert_eq!(
                stage_step.intent.as_ref().map(|i| i.kind),
                Some(SeatIntentKind::UpdateBundle)
            );
            let clear_step = press(
                &base,
                stage_step.state,
                SeatKeyCode::Delete,
                SeatKeyModifiers::NONE,
            );
            assert_eq!(
                clear_step.intent.as_ref().map(|i| i.kind),
                Some(SeatIntentKind::UpdateBundle)
            );
            s = press(
                &base,
                clear_step.state,
                SeatKeyCode::Char(']'),
                SeatKeyModifiers::NONE,
            )
            .state;
        }
        s = press(&base, s, SeatKeyCode::Char('['), SeatKeyModifiers::NONE).state;

        // 4. Composer free-text draft overrides & Esc cancel + Notes edit/Backspace/Esc.
        s = press(&obs, s, SeatKeyCode::Char('e'), SeatKeyModifiers::NONE).state;
        for ch in "amt=1500 px=1400 id=3 peer=2 runs=2 batches=2 debt=300 bid=400x".chars() {
            s = press(&obs, s, SeatKeyCode::Char(ch), SeatKeyModifiers::NONE).state;
        }
        s = press(&obs, s, SeatKeyCode::Backspace, SeatKeyModifiers::NONE).state;
        s = press(&obs, s, SeatKeyCode::Enter, SeatKeyModifiers::NONE).state;
        s = press(&obs, s, SeatKeyCode::Char('e'), SeatKeyModifiers::NONE).state;
        s = press(&obs, s, SeatKeyCode::Esc, SeatKeyModifiers::NONE).state;
        s = press(&obs, s, SeatKeyCode::Char('n'), SeatKeyModifiers::NONE).state;
        s = press(&obs, s, SeatKeyCode::Char('a'), SeatKeyModifiers::NONE).state;
        s = press(&obs, s, SeatKeyCode::Backspace, SeatKeyModifiers::NONE).state;
        s = press(&obs, s, SeatKeyCode::Esc, SeatKeyModifiers::NONE).state;

        // 5. Tab 3 What-If Sandbox rows (toggle all 7 rows + preview Close Marks + dual delivery).
        let mut bundle = SealedOrderBundle::default();
        BundleDraftAction::StageAtomicSwap(AtomicSwapProposal {
            swap_id: 9,
            maker: 0,
            taker: 1,
            maker_asset: AssetId::Refined(0),
            maker_amount: 100,
            taker_asset: AssetId::Refined(1),
            taker_amount: 100,
            domain_id: None,
        })
        .apply_to(&mut bundle);
        BundleDraftAction::StageDualEscrow(DualEscrowProposal {
            proposal_id: 9,
            buyer: 0,
            seller: 1,
            good: AssetId::Refined(1),
            required_units: 100,
            payment_cred: 300,
            buyer_bond_cred: 100,
            seller_bond_cred: 100,
            deadline_turn: 26,
            domain_id: None,
        })
        .apply_to(&mut bundle);
        s.set_staged_bundle(bundle);
        s.set_what_if_preview(Some(WhatIfPreview {
            seat: 0,
            turn: 24,
            baseline_predicates: obs.baseline_predicates.clone(),
            pre_seat_state: obs.seats[0].clone(),
            post_market_seat_state: obs.seats[0].clone(),
            post_sweep_seat_state: obs.seats[0].clone(),
            post_sweep_predicates: obs.baseline_predicates.clone(),
            close_mark_caveat: league_seat_types::HYPOTHETICAL_CLOSE_MARK_CAVEAT.to_owned(),
            refused_market_actions: Vec::new(),
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
            step0_to_7_receipt: Step0To7SweepSummary::from_public_sweep_receipt(
                &obs.public_receipts[0],
            ),
        }));
        s = press(&obs, s, SeatKeyCode::Char('3'), SeatKeyModifiers::NONE).state;
        for _ in 0..7 {
            s = press(&obs, s, SeatKeyCode::Char(' '), SeatKeyModifiers::NONE).state;
            s = press(&obs, s, SeatKeyCode::Down, SeatKeyModifiers::NONE).state;
        }
        // Toggle last row off then back on so `toggle_u64` removal is covered too.
        s = press(&obs, s, SeatKeyCode::Char(' '), SeatKeyModifiers::NONE).state;
        s = press(&obs, s, SeatKeyCode::Char(' '), SeatKeyModifiers::NONE).state;
        s = press(&obs, s, SeatKeyCode::PageDown, SeatKeyModifiers::NONE).state;
        s = press(&obs, s, SeatKeyCode::PageUp, SeatKeyModifiers::NONE).state;
        s = press(&obs, s, SeatKeyCode::Up, SeatKeyModifiers::NONE).state;
        s = press(&obs, s, SeatKeyCode::Home, SeatKeyModifiers::NONE).state;
        let wi = press(&obs, s, SeatKeyCode::Enter, SeatKeyModifiers::NONE);
        let req = wi
            .intent
            .and_then(|i| i.what_if)
            .unwrap_or_else(|| panic!("expected what_if"));
        assert_eq!(req.market_actions.len(), 1);
        assert_eq!(req.assumptions.assumed_cosigned_swaps.len(), 1);
        assert_eq!(req.assumptions.assumed_cosigned_dual_escrows.len(), 1);
        assert_eq!(req.assumptions.assumed_peer_deliveries.len(), 1);
        assert_eq!(req.assumptions.assumed_cancelled_offer_ids, vec![1]);
        assert_eq!(req.assumptions.assumed_bounced_check_ids, vec![1]);
        let txt = render_turn_screen(&obs, &wi.state, PassKind::SubPassA, None, vp).plain_text();
        assert!(txt.contains("Close Mark"));
        assert!(txt.contains("Closing Window Active"));

        // 6. Chronicle modal navigation across 2 receipts, FillRecord lines, scroll, plain codes, and close.
        let m0 = press(
            &obs,
            wi.state,
            SeatKeyCode::Char('r'),
            SeatKeyModifiers::NONE,
        );
        assert!(m0.view.plain_text().contains("Receipt 2/2"));
        let m1 = press(
            &obs,
            m0.state,
            SeatKeyCode::Char('['),
            SeatKeyModifiers::NONE,
        );
        assert!(m1.view.plain_text().contains("Fill T22 BatchFill"));
        let mut ms = m1.state;
        for kc in [
            SeatKeyCode::Char(']'),
            SeatKeyCode::Down,
            SeatKeyCode::PageDown,
            SeatKeyCode::Up,
            SeatKeyCode::PageUp,
            SeatKeyCode::Home,
            SeatKeyCode::Char('c'),
            SeatKeyCode::Char('c'),
        ] {
            ms = press(&obs, ms, kc, SeatKeyModifiers::NONE).state;
        }
        let closed = press(&obs, ms, SeatKeyCode::Esc, SeatKeyModifiers::NONE);
        assert_eq!(
            closed.intent.map(|i| i.kind),
            Some(SeatIntentKind::CloseChronicleModal)
        );
        assert!(!closed.state.chronicle_open());
    }

    #[test]
    fn neutral_standby_shift_l_and_repeat_key_filters_hold() {
        let obs = sample_seat_observation();
        let vp = ViewportSize::new(80, 24);
        let neutral = neutral_session_view(
            TableId(202_611),
            Turn(4),
            TurnPhase::AwaitingMandateReveal,
            PassKind::SubPassB,
            false,
            false,
            vp,
        );
        let n_txt = neutral.plain_text();
        assert!(n_txt.contains("Session Standby"));
        assert!(n_txt.contains("Table #202611 | Turn 4 | Phase: AwaitingMandateReveal"));
        assert!(n_txt.contains("Season closed — waiting for host post-season step."));
        assert!(!n_txt.contains("Private Vault"));
        assert!(!n_txt.contains("Private Mandate"));

        let halted_neutral = neutral_session_view(
            TableId(202_611),
            Turn(4),
            TurnPhase::Reveal,
            PassKind::SubPassB,
            false,
            true,
            vp,
        );
        assert!(
            halted_neutral
                .plain_text()
                .contains("Pass advance failed — session halted")
        );

        let small_neutral = neutral_session_view(
            TableId(202_611),
            Turn(4),
            TurnPhase::TalkAndMarket,
            PassKind::SubPassA,
            true,
            false,
            ViewportSize::new(79, 23),
        );
        assert_eq!(small_neutral.layout.mode, ViewLayoutMode::TooSmall);

        // Shift-L alone does not emit LockAndHandOff; Ctrl+L does; repeated Ctrl+L does not.
        let shift_l = handle_seat_key(
            &obs,
            SeatUiState::new(),
            PassKind::SubPassA,
            None,
            vp,
            key(
                SeatKeyCode::Char('L'),
                SeatKeyModifiers {
                    ctrl: false,
                    alt: false,
                    shift: true,
                },
            ),
        );
        assert!(shift_l.intent.is_none());

        let rep_enter = handle_seat_key(
            &obs,
            SeatUiState::new(),
            PassKind::SubPassA,
            None,
            vp,
            SeatKeyEvent::repeated(SeatKeyCode::Enter, SeatKeyModifiers::NONE),
        );
        assert!(rep_enter.intent.is_none());

        let rep_ctrl_l = handle_seat_key(
            &obs,
            SeatUiState::new(),
            PassKind::SubPassA,
            None,
            vp,
            SeatKeyEvent::repeated(SeatKeyCode::Char('l'), SeatKeyModifiers::CTRL),
        );
        assert!(rep_ctrl_l.intent.is_none());

        let rep_quit = action_for(SeatKeyEvent::repeated(
            SeatKeyCode::Char('q'),
            SeatKeyModifiers::NONE,
        ));
        assert_eq!(rep_quit, Action::Ignore);

        let press_ctrl_l = handle_seat_key(
            &obs,
            SeatUiState::new(),
            PassKind::SubPassA,
            None,
            vp,
            key(SeatKeyCode::Char('l'), SeatKeyModifiers::CTRL),
        );
        assert_eq!(
            press_ctrl_l.intent.map(|i| i.kind),
            Some(SeatIntentKind::LockAndHandOff)
        );
    }

    #[test]
    fn ledger_abbreviations_clock_badge_pin_prompts_and_note_replay_work() {
        use league_seat_types::{CurtainPinMode, TurnClockDisplay};

        assert_eq!(render::format_ledger_units(15_000, 6), "15000");
        assert_eq!(render::format_ledger_units(15_000, 4), "15k");
        assert_eq!(render::format_ledger_units(15_500, 5), "15500");
        assert_eq!(render::format_ledger_units(15_500_000, 5), "15.5M");
        assert_eq!(render::format_ledger_units(15_500, 4), "15k+");
        assert_eq!(render::format_ledger_units(1_234_567, 6), "1.2M+");
        assert_eq!(render::format_ledger_units(-15_000, 4), "-15k");
        assert_eq!(render::format_tariff_pair(1_245, 708, 9), "1245/708");
        assert_eq!(render::format_tariff_pair(1_245, 708, 7), "1k+/708");

        let obs = sample_seat_observation();
        let mut st = SeatUiState::new();
        st.set_clock_display(TurnClockDisplay::timed(95, 157));
        assert_eq!(st.clock_display().remaining_secs, 95);
        st.apply_note_key(key(SeatKeyCode::Char('A'), SeatKeyModifiers::NONE));
        st.apply_note_key(key(SeatKeyCode::Char('B'), SeatKeyModifiers::NONE));
        st.apply_note_key(key(SeatKeyCode::Backspace, SeatKeyModifiers::NONE));
        assert_eq!(st.notes_draft(), "A");

        let timed_txt = render_turn_screen(
            &obs,
            &st,
            PassKind::SubPassA,
            None,
            ViewportSize::new(100, 30),
        )
        .plain_text();
        assert!(timed_txt.contains("Clock: 01:35"));

        let pin_set = curtain_view(
            TableId(202_611),
            Turn(1),
            PassKind::SubPassA,
            Seat(0),
            ViewportSize::new(100, 30),
            true,
            CurtainPinPrompt {
                mode: CurtainPinMode::SetInitialPin,
                entered_count: 2,
                failed_attempts: 0,
                locked_out_reset: false,
            },
        )
        .plain_text();
        assert!(pin_set.contains("SET 4-DIGIT SEAT PIN: [**__]"));

        let pin_verify = curtain_view(
            TableId(202_611),
            Turn(2),
            PassKind::SubPassB,
            Seat(0),
            ViewportSize::new(100, 30),
            true,
            CurtainPinPrompt {
                mode: CurtainPinMode::VerifyPin,
                entered_count: 1,
                failed_attempts: 1,
                locked_out_reset: false,
            },
        )
        .plain_text();
        assert!(pin_verify.contains("ENTER 4-DIGIT SEAT PIN: [*___] (wrong PIN; attempt 2/3)"));

        let pin_locked_out = curtain_view(
            TableId(202_611),
            Turn(2),
            PassKind::SubPassB,
            Seat(0),
            ViewportSize::new(100, 30),
            false,
            CurtainPinPrompt {
                mode: CurtainPinMode::Inactive,
                entered_count: 0,
                failed_attempts: 0,
                locked_out_reset: true,
            },
        )
        .plain_text();
        assert!(pin_locked_out.contains("PIN entry failed 3 times — Curtain re-locked."));
    }
}
