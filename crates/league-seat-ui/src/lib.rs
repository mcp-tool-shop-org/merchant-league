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

use alloc::borrow::ToOwned;
use alloc::format;
use alloc::string::String;
use alloc::vec;

use league_seat_types::{
    AssetId, CommodityDisplayMap, NUMERAIRE_DISPLAY_LABEL, SeatKeyCode, SeatKeyEvent,
    SeatObservation, SeatScreenView, SeatState, ViewAlign, ViewLabel, ViewPanel, ViewRow,
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

/// Map a plain seat key press to an [`Action`]. `q`, `Esc` and Ctrl-C quit.
#[must_use]
pub const fn action_for(key: SeatKeyEvent) -> Action {
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
        rows,
        selected_row: None,
    })
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
        rows,
        selected_row: None,
    })
}

#[cfg(test)]
mod tests {
    use alloc::borrow::ToOwned;
    use alloc::collections::{BTreeMap, BTreeSet};
    use alloc::vec;
    use alloc::vec::Vec;

    use league_seat_types::{
        ActiveShock, AssetId, AuditorShockHint, COMMODITY_DISPLAY_CATALOG, CharterRole,
        CoalitionReach, CommitmentHash, CommodityDisplayMap, CreditTier, FundamentalParities,
        GoodScarcity, GradedMandateCard, MandateKind, NUMERAIRE_CODE, NUMERAIRE_DISPLAY_LABEL,
        PrivateMandate, PublicTableSpec, RegentDoctrine, STATE_PREDICATES_VERSION,
        SeasonEndAndMarks, SeatCommodityDisplay, SeatConfig, SeatIdentity, SeatKeyCode,
        SeatKeyEvent, SeatKeyModifiers, SeatObservation, SeatObservationPublicParts, SeatState,
        SeatStatePredicates, ShockKind, SpoilageAndConversion, TableId, TableMode, TurnPhase,
        UpkeepAndDebt, ViewAlign,
    };

    use crate::{
        Action, TITLE_LINES, action_for, format_own_mandate_line, render_seat,
        render_seat_with_commodities, title_view,
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
        assert_eq!(view.main.alignment, ViewAlign::Center);
        assert!(view.main.bordered);
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
}
