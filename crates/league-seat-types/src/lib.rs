#![no_std]
#![forbid(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]
//! Seat-safe value, observation, preview, error, and commodity display types for Merchant League v2.
//!
//! This `no_std` + `alloc` leaf crate depends solely on `serde`, `serde_json`, and
//! `league-error` — never on `league-core`, `league-gen`, or `league-standalone` — so `rustc`
//! and target builds (`thumbv7em-none-eabi`) enforce the host/seat boundary at compile time
//! (`docs/club-mode-design.md` §2.3).

extern crate alloc;

mod commit;
mod contracts;
mod display;
mod engine_types;
mod observation;
mod predicates_types;
mod scoring_types;
mod serde_helpers;
mod types;
mod view;

pub use commit::{
    AtomicSwapProposal, CharterBidOrder, CommitmentHash, ContractDeliveryOrder, ConverterRunOrder,
    CouncilPlatform, CouncilVoteBallot, DRAND_QUICKNET_GENESIS_UNIX, DRAND_QUICKNET_PERIOD_SECS,
    DrandSchedule, DualEscrowProposal, EscrowFinishUnlock, GradedMandateCard, IntentPledge,
    IntentTarget, MandateKind, PreimageSecret, PrivateMandate, ProjectDeliveryOrder,
    SealedOrderBundle, drand_quicknet_round_for_close,
};
pub use contracts::{
    ActiveCharter, BondProposal, BondSettlementMode, BondSpec, CartelDomain, CheckProposal,
    CheckStatus, CredOffer, CreditCheck, DualEscrowVault, EscrowProposal, EscrowStatus, FillKind,
    FillRecord, MAX_CARTEL_DOMAINS, MarketAction, OfferProposal, OfferSide, PendingWashFreeze,
    TokenEscrowVault, WashFreezeSettlement,
};
pub use display::{
    COMMODITY_DISPLAY_CATALOG, CommodityDisplayEntry, CommodityDisplayMap, NUMERAIRE_CODE,
    NUMERAIRE_DISPLAY_LABEL, NUMERAIRE_PLAIN_LABEL, SeatCommodityDisplay, format_numeraire_display,
    lookup_commodity_display, lookup_commodity_display_bytes,
};
pub use engine_types::{
    BrokenPromiseRecord, ConverterOrderPreview, EscrowOrderPreview, ProjectOrderPreview, SeatState,
    SeatUpkeepPreview, ShockForecastEvent, TurnPhase,
};
pub use league_error::{StructuredError, ToStructuredError, scrub_secret_text};
#[doc(hidden)]
pub use observation::SeatObservationPublicParts;
pub use observation::{
    AiPreviewRequest, AuditorShockHint, CounterpartyAssumptions, DEALT_MANDATE_COMMIT_SENTINEL,
    HYPOTHETICAL_CLOSE_MARK_CAVEAT, HumanWhatIfRequest, PREVIEW_NEUTRAL_BEACON_SEED,
    PreviewTieMode, PublicCheckOrderPreview, PublicCheckOutcome, PublicPreviewState,
    PublicSwapOrderPreview, PublicSwapOutcome, PublicSweepReceipt, PublicTableSpec,
    RefusedActionPreview, SeatObservation, SeatSpoilagePreview, Step0To7SweepSummary,
    WhatIfPreview,
};
pub use predicates_types::{
    CoalitionReach, GoodScarcity, HoldingMark, ProjectCover, ProjectInputCover, RecipeCover,
    STATE_PREDICATE_SECTION_VERSION, STATE_PREDICATES_VERSION, SeasonEndAndMarks,
    SeatSeasonMetrics, SeatStatePredicates, SeatSweepOutcome, SpoilageAndConversion,
    StatePredicateSection, TurnPredicateRecord, UpkeepAndDebt,
};
pub use scoring_types::{FinalScoreboard, FundamentalParities, ParitySpecSource, SeatScoreEntry};
pub use types::{
    ActiveShock, AssetId, BPS_SCALE, Bps, CLOSING_ROLL_START_TURN, CLOSING_ROLL_WINDOW_LEN,
    CharterRole, ConverterRecipe, CreditTier, EngineError, HARD_TURN_CAP,
    MANDATE_STARTING_NW_FLOOR_BPS, MAX_MANDATE_BONUS_EQUITY, MIN_MANDATE_BONUS_EQUITY, MarketDay,
    ProjectRecipe, RegentDoctrine, SeasonId, Seat, SeatConfig, SeatIdentity, ShockKind, TableId,
    TableMode, Turn, UNIT_SCALE, UNSOLD_EXCESS_PARITY_BPS, Units,
};
pub use view::{
    SeatKeyCode, SeatKeyEvent, SeatKeyModifiers, SeatScreenView, ViewAlign, ViewColor, ViewLabel,
    ViewPanel, ViewRow, ViewSpan, ViewStyle, ViewportSize,
};

/// Internal serde helpers re-exported within `crates/` via explicit crate usage where identical
/// `EngineState` / `SeasonReplayLog` JSON encodings are shared.
pub mod wire_serde {
    use alloc::collections::BTreeMap;

    use serde::{Deserializer, Serializer};

    use crate::serde_helpers;
    use crate::types::{AssetId, Units};

    /// Cumulative bilateral delivery ledger type `(sender, receiver, asset) -> units`.
    pub type DeliveryLedgerMap = BTreeMap<(u8, u8, AssetId), Units>;

    /// Serializes a 32-byte digest or commitment as 64 lowercase ASCII hex characters.
    pub fn serialize_hex32<S: Serializer>(
        bytes: &[u8; 32],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serde_helpers::hex32::serialize(bytes, serializer)
    }

    /// Deserializes a 64-character hex string into 32 bytes.
    pub fn deserialize_hex32<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<[u8; 32], D::Error> {
        serde_helpers::hex32::deserialize(deserializer)
    }

    /// Serializes an optional 32-byte digest as optional 64 lowercase ASCII hex characters.
    pub fn serialize_opt_hex32<S: Serializer>(
        value: &Option<[u8; 32]>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serde_helpers::opt_hex32::serialize(value, serializer)
    }

    /// Deserializes an optional 64-character hex string into optional 32 bytes.
    pub fn deserialize_opt_hex32<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<[u8; 32]>, D::Error> {
        serde_helpers::opt_hex32::deserialize(deserializer)
    }

    /// Serializes the cumulative bilateral delivery ledger map as key-value entries.
    pub fn serialize_delivery_ledger<S: Serializer>(
        map: &DeliveryLedgerMap,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serde_helpers::delivery_ledger_serde::serialize(map, serializer)
    }

    /// Deserializes bilateral delivery ledger entries while rejecting duplicate tuple keys.
    pub fn deserialize_delivery_ledger<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<DeliveryLedgerMap, D::Error> {
        serde_helpers::delivery_ledger_serde::deserialize(deserializer)
    }
}

#[cfg(test)]
mod tests {
    use alloc::borrow::ToOwned;
    use alloc::collections::{BTreeMap, BTreeSet};
    use alloc::format;
    use alloc::vec;
    use alloc::vec::Vec;

    use alloc::string::ToString;

    use crate::{
        AssetId, AuditorShockHint, Bps, COMMODITY_DISPLAY_CATALOG, CoalitionReach, CommitmentHash,
        CommodityDisplayMap, ConverterRecipe, CounterpartyAssumptions, CreditTier,
        DRAND_QUICKNET_GENESIS_UNIX, DrandSchedule, EngineError, FundamentalParities, GoodScarcity,
        GradedMandateCard, MandateKind, MarketAction, MarketDay, NUMERAIRE_CODE,
        NUMERAIRE_DISPLAY_LABEL, NUMERAIRE_PLAIN_LABEL, ParitySpecSource, PreimageSecret,
        PreviewTieMode, PrivateMandate, PublicSweepReceipt, PublicTableSpec, RegentDoctrine,
        STATE_PREDICATES_VERSION, SealedOrderBundle, SeasonEndAndMarks, SeasonId, Seat,
        SeatCommodityDisplay, SeatConfig, SeatIdentity, SeatKeyCode, SeatKeyEvent,
        SeatKeyModifiers, SeatObservation, SeatObservationPublicParts, SeatScreenView, SeatState,
        SeatStatePredicates, ShockKind, SpoilageAndConversion, Step0To7SweepSummary, TableId,
        TableMode, ToStructuredError, Turn, TurnPhase, UpkeepAndDebt, ViewAlign, ViewColor,
        ViewLabel, ViewPanel, ViewRow, ViewSpan, ViewStyle, ViewportSize,
        drand_quicknet_round_for_close, format_numeraire_display, lookup_commodity_display,
        lookup_commodity_display_bytes,
    };

    #[test]
    fn ids_assets_credit_and_commit_value_methods_work() {
        assert!(Seat(0) < Seat(1));
        assert!(MarketDay(1) < MarketDay(2));
        assert!(Turn(1) < Turn(2));
        assert!(TableId(1) < TableId(2));
        assert!(SeasonId(1) < SeasonId(2));

        assert!(!AssetId::Cred.is_perishable());
        assert!(AssetId::Raw(0).is_perishable());
        assert!(!AssetId::Refined(0).is_perishable());

        assert_eq!(CreditTier::Aaa.interest_bps(), 400);
        assert_eq!(CreditTier::Bbb.interest_bps(), 1_000);
        assert_eq!(CreditTier::B.interest_bps(), 1_800);
        assert_eq!(CreditTier::D.interest_bps(), 3_000);
        assert_eq!(CreditTier::Aaa.downgrade_one(), CreditTier::Bbb);
        assert_eq!(CreditTier::Bbb.downgrade_one(), CreditTier::B);
        assert_eq!(CreditTier::B.downgrade_one(), CreditTier::D);
        assert_eq!(CreditTier::D.downgrade_one(), CreditTier::D);
        assert!(CreditTier::Aaa.may_access_prime_cartel());
        assert!(CreditTier::Bbb.may_access_prime_cartel());
        assert!(CreditTier::B.may_access_prime_cartel());
        assert!(!CreditTier::D.may_access_prime_cartel());

        let zero_secret = PreimageSecret([0u8; 32]);
        assert!(zero_secret.is_degenerate());
        assert!(zero_secret.validate().is_err());
        let good_secret = PreimageSecret([0x5A; 32]);
        assert!(!good_secret.is_degenerate());
        assert!(good_secret.validate().is_ok());
        assert_eq!(format!("{good_secret:?}"), "PreimageSecret(<hidden>)");

        let zero_hash = CommitmentHash([0u8; 32]);
        assert!(zero_hash.is_degenerate());
        let hash = CommitmentHash([0xAB; 32]);
        assert!(!hash.is_degenerate());
        assert_eq!(format!("{hash}"), "ab".repeat(32));
        assert_eq!(
            format!("{hash:?}"),
            format!("CommitmentHash({})", "ab".repeat(32))
        );
    }

    #[test]
    fn mandate_drand_action_error_and_display_catalog_work() {
        assert!(
            MandateKind::MinProjectEquity { target: 0 }
                .validate_for_seat_count(4)
                .is_err()
        );
        assert!(
            MandateKind::MinProjectEquity { target: 100 }
                .validate_for_seat_count(2)
                .is_err()
        );
        assert!(
            MandateKind::MinRefinedVariety {
                distinct_goods: 3,
                min_each: 1_000,
            }
            .validate_for_seat_count(4)
            .is_err()
        );
        assert!(
            MandateKind::MaintainCleanCredit {
                min_tier: CreditTier::D,
            }
            .validate_for_seat_count(4)
            .is_err()
        );
        assert!(
            MandateKind::MinActivePartners { min_partners: 3 }
                .validate_for_seat_count(4)
                .is_err()
        );
        assert!(
            MandateKind::MinActivePartners { min_partners: 2 }
                .validate_for_seat_count(4)
                .is_ok()
        );
        let card = GradedMandateCard {
            card_id: 1,
            kind: MandateKind::MaintainCleanCredit {
                min_tier: CreditTier::Aaa,
            },
            difficulty_completion_bps: 5_000,
            bonus_equity: 5_500,
        };
        assert_eq!(card.mandate_points_milli(), 5_500);

        let ds = DrandSchedule {
            turn_1_close_unix: DRAND_QUICKNET_GENESIS_UNIX + 60,
            turn_period_secs: 600,
        };
        assert!(ds.validate().is_ok());
        assert!(ds.close_unix_for_turn(Turn(0)).is_err());
        assert!(ds.close_unix_for_turn(Turn(1)).is_ok());
        assert!(ds.expected_round_for_turn(Turn(1)).is_ok());
        assert!(drand_quicknet_round_for_close(0).is_err());
        assert!(drand_quicknet_round_for_close(DRAND_QUICKNET_GENESIS_UNIX + 6).is_ok());
        assert!(
            DrandSchedule {
                turn_1_close_unix: DRAND_QUICKNET_GENESIS_UNIX - 1,
                turn_period_secs: 60,
            }
            .validate()
            .is_err()
        );
        assert!(
            DrandSchedule {
                turn_1_close_unix: DRAND_QUICKNET_GENESIS_UNIX + 60,
                turn_period_secs: u64::MAX,
            }
            .validate()
            .is_err()
        );
        assert!(
            DrandSchedule {
                turn_1_close_unix: u64::MAX,
                turn_period_secs: 3,
            }
            .validate()
            .is_err()
        );

        let bad_ds = DrandSchedule {
            turn_1_close_unix: 10,
            turn_period_secs: 1,
        };
        assert!(bad_ds.validate().is_err());

        let act = MarketAction::FlagWashFreeze {
            inspector: 3,
            target_seat: 1,
        };
        assert_eq!(act.actor_seat(), 3);
        assert_eq!(
            MarketAction::TownImport {
                seat: 2,
                good: AssetId::Raw(0),
                amount: 100
            }
            .actor_seat(),
            2
        );

        let err = EngineError::CancelLocked;
        let se = err.to_structured_error();
        assert_eq!(se.code, "CANCEL_LOCKED");
        for e in [
            EngineError::InvalidConfig("x".to_owned()),
            EngineError::InvalidPhase("x".to_owned()),
            EngineError::InvalidSeat(9),
            EngineError::InvalidOrder("x".to_owned()),
            EngineError::UnknownObject("x".to_owned()),
            EngineError::Unauthorized("x".to_owned()),
            EngineError::PerishableEscrowForbidden,
            EngineError::InsufficientBalance("x".to_owned()),
            EngineError::AlreadySubmitted(0),
            EngineError::CommitmentMismatch(0),
            EngineError::DomainAccessDenied("x".to_owned()),
            EngineError::SeasonEnded("x".to_owned()),
            EngineError::UnsupportedReplayVersion {
                found: 0,
                newest_known: 3,
            },
        ] {
            let s = e.to_structured_error();
            assert_eq!(s.code, e.code());
            assert!(!s.message.is_empty());
        }

        assert_eq!(COMMODITY_DISPLAY_CATALOG.len(), 61);
        assert_eq!(format_numeraire_display(false), NUMERAIRE_DISPLAY_LABEL);
        assert_eq!(format_numeraire_display(true), NUMERAIRE_PLAIN_LABEL);
        assert!(lookup_commodity_display_bytes(*b"TMB").is_some());
        assert!(lookup_commodity_display_bytes(*b"ZZZ").is_none());
        let tmb_opt = lookup_commodity_display("TMB");
        assert!(tmb_opt.is_some());
        if let Some(tmb) = tmb_opt {
            assert_eq!(tmb.raw_display(false), "Rough Timber [TMB]");
            assert_eq!(tmb.raw_display(true), "TMB");
            assert_eq!(tmb.refined_display(false), "Seasoned Timber [TMB]");
            assert_eq!(tmb.refined_display(true), "TMB");
        }
        assert!(lookup_commodity_display("CRED").is_none());
        assert!(lookup_commodity_display("ZWZ").is_none());
        assert!(lookup_commodity_display("AB").is_none());

        let map = CommodityDisplayMap {
            numeraire_code: NUMERAIRE_CODE.to_owned(),
            numeraire_label: NUMERAIRE_DISPLAY_LABEL.to_owned(),
            seats: vec![SeatCommodityDisplay {
                seat: 0,
                raw_code: "TMB".to_owned(),
                raw_label: "Rough Timber [TMB]".to_owned(),
                refined_code: "SLT".to_owned(),
                refined_label: "Cured Fine Salt [SLT]".to_owned(),
            }],
        };
        assert_eq!(
            map.format_asset(AssetId::Cred, false).as_deref(),
            Some("Clearinghouse Marks [CRED]")
        );
        assert_eq!(
            map.format_asset(AssetId::Cred, true).as_deref(),
            Some("CRED")
        );
        assert_eq!(
            map.format_asset(AssetId::Raw(0), false).as_deref(),
            Some("Rough Timber [TMB]")
        );
        assert_eq!(
            map.format_asset(AssetId::Raw(0), true).as_deref(),
            Some("TMB")
        );
        assert_eq!(
            map.format_asset(AssetId::Refined(0), false).as_deref(),
            Some("Cured Fine Salt [SLT]")
        );
        assert_eq!(
            map.format_asset(AssetId::Refined(0), true).as_deref(),
            Some("SLT")
        );
        assert!(map.format_asset(AssetId::Raw(9), false).is_none());
        assert!(map.format_asset(AssetId::Refined(9), false).is_none());
    }

    #[test]
    fn observation_preview_summary_and_parities_cover_helpers_cleanly() {
        struct EmptySpec;
        impl ParitySpecSource for EmptySpec {
            fn spec_seat_count(&self) -> u8 {
                0
            }

            fn spec_seats(&self) -> &[SeatConfig] {
                &[]
            }
        }
        let empty_p = FundamentalParities::from_spec(&EmptySpec);
        assert_eq!(empty_p.cred_per_unit, 1_000);
        assert!(empty_p.raw_per_unit.is_empty());
        assert!(empty_p.refined_per_unit.is_empty());
        assert_eq!(empty_p.asset_parity(AssetId::Cred), 1_000);
        assert_eq!(empty_p.asset_parity(AssetId::Raw(5)), 0);
        assert_eq!(empty_p.asset_parity(AssetId::Refined(5)), 0);

        let seat_cfg = SeatConfig {
            seat: 0,
            identity: SeatIdentity::Human,
            initial_cred: 15_000,
            raw_yield_per_turn: 10_000,
            refined_project_value: 2_400,
            upkeep_good: 1,
            upkeep_amount: 1_800,
            initial_raw_tariff_bps: 800,
            initial_refined_tariff_bps: 600,
            perishable_decay_bps: 2_000,
            regent: Some(RegentDoctrine {
                risk_tolerance_bps: 5_000,
                min_raw_reserve: 1_000,
                auto_cash_due_claims: true,
            }),
            converters: vec![
                ConverterRecipe {
                    owner: 0,
                    inputs: vec![(0, 1_000)],
                    output_good: 0,
                    output_amount: 2_000,
                    max_runs_per_turn: 2,
                    is_secondary: true,
                },
                ConverterRecipe {
                    owner: 0,
                    inputs: vec![],
                    output_good: 0,
                    output_amount: 1_000,
                    max_runs_per_turn: 1,
                    is_secondary: false,
                },
            ],
        };
        let pub_spec = PublicTableSpec {
            table_id: TableId(55),
            mode: TableMode::Club,
            seat_count: 1,
            seats: vec![seat_cfg],
            projects: Vec::new(),
            closing_hazard_bps: [3_500; 8],
            mandate_deck: Vec::new(),
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
        assert_eq!(pub_spec.spec_seat_count(), 1);
        assert_eq!(pub_spec.spec_seats().len(), 1);
        let parities = FundamentalParities::from_spec(&pub_spec);
        assert_eq!(parities.raw_per_unit, vec![2_400]);
        assert_eq!(parities.refined_per_unit, vec![2_400]);

        let predicates = SeatStatePredicates {
            version: STATE_PREDICATES_VERSION,
            table_id: TableId(55),
            turn: 2,
            phase: TurnPhase::TalkAndMarket,
            seat: 0,
            upkeep: UpkeepAndDebt {
                upkeep_good: 1,
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
                spoilage_bps_by_good: vec![2_000],
                spoil_units_held_by_good: vec![0],
                spoil_units_on_resting_offers: 0,
                spoil_units_total: 0,
                workshop_capacity_runs_left: 2,
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
                seats_needing: 1,
                open_book_tariff_bps: 800,
            }],
            coalitions: vec![CoalitionReach {
                members: vec![0],
                upkeep_goods_makeable: Vec::new(),
                upkeep_goods_not_makeable: vec![1],
            }],
            season_end: SeasonEndAndMarks {
                season_closed: false,
                turns_until_earliest_close: 22,
                turns_until_latest_close: 30,
                holdings: Vec::new(),
                projects: Vec::new(),
            },
            cancel_locked: false,
        };
        let receipt = PublicSweepReceipt {
            turn: 1,
            non_reveal_seats: vec![],
            regent_acted_seats: vec![],
            converted_to_ai: vec![],
            escrows_finished: vec![],
            escrows_cancelled_or_forfeited: vec![],
            atomic_swaps_settled: 0,
            converter_batches_run: 1,
            converter_orders_clamped: 0,
            checks_cashed: vec![],
            checks_bounced: vec![],
            cartel_evictions: vec![],
            projects_completed: 0,
            project_orders_clamped: 0,
            total_upkeep_shortfall_debt: 0,
            total_spoilage_clawed_back: 0,
            broken_promises: vec![],
            elected_officers: None,
            charters_awarded: vec![],
            public_forecasts: vec![],
            newly_activated_shocks: vec![],
            cred_debt_repaid: 0,
            charter_cred_burned: 0,
            town_import_cred_spent: 0,
            works_stipend_cred_minted: 0,
            batch_trades_cleared: 0,
            fills: vec![],
            wash_freeze_settlements: vec![],
            season_closed: false,
            pbrs_deltas: vec![10],
        };
        let summary = Step0To7SweepSummary::from_public_sweep_receipt(&receipt);
        assert_eq!(summary.turn, 1);
        assert_eq!(summary.converter_batches_run, 1);

        let hint = AuditorShockHint {
            hint_turn: 2,
            forecast_turn: 3,
            realize_turn: 4,
            target_seat: 0,
            kind: ShockKind::DemandSurge,
        };
        let obs = SeatObservation::unmandated(SeatObservationPublicParts {
            seat: 0,
            table_id: TableId(55),
            turn: 2,
            phase: TurnPhase::TalkAndMarket,
            cancel_locked: false,
            trade_minister: 0,
            cartel_chair: None,
            spec: pub_spec,
            parities,
            seats: vec![SeatState {
                seat: 0,
                identity: SeatIdentity::Human,
                cred: 15_000,
                cred_debt: 0,
                raw: vec![0],
                refined: vec![0],
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
            auditor_hints: vec![hint],
            next_offer_id: 1,
            next_check_id: 1,
            next_escrow_id: 1,
            delivery_ledger: BTreeMap::new(),
            last_potentials: vec![15_000],
            pending_town_import_cred_spent: 0,
            fill_log: Vec::new(),
            pending_wash_freezes: Vec::new(),
            eval_vrf_pubkey: None,
            baseline_predicates: predicates,
            public_receipts: vec![receipt],
        });
        assert!(obs.own_mandate().is_none());
        assert_eq!(obs.auditor_hints(), &[hint]);
        let preview_state = obs.to_public_preview_state();
        assert_eq!(preview_state.table_id, TableId(55));
        assert_eq!(preview_state.turn, 2);
        let default_bundle = SealedOrderBundle::default();
        assert!(default_bundle.converter_runs.is_empty());
        let default_assumptions = CounterpartyAssumptions::default();
        assert!(default_assumptions.assumed_peer_bundles.is_empty());
        assert_eq!(PreviewTieMode::default(), PreviewTieMode::NeutralSeed);
        let _ = (
            None::<Bps>,
            PrivateMandate {
                kind: MandateKind::MinProjectEquity { target: 1_000 },
                bonus_equity: 3_000,
            },
        );
    }

    #[test]
    fn view_model_types_and_key_events_cover_plain_owned_helpers() {
        let style = ViewStyle::plain()
            .with_fg(ViewColor::Cyan)
            .with_bg(ViewColor::Rgb {
                r: 10,
                g: 20,
                b: 30,
            })
            .bold()
            .dim()
            .underlined()
            .reversed();
        assert_eq!(style.fg, ViewColor::Cyan);
        assert!(style.bold && style.dim && style.underlined && style.reversed);

        let row = ViewRow::from_spans(vec![
            ViewSpan::plain("CRED: ".to_owned()),
            ViewSpan::styled("15000".to_owned(), style),
        ]);
        assert_eq!(row.plain_text(), "CRED: 15000");

        let panel = ViewPanel {
            title: Some(ViewLabel::styled("Header".to_owned(), style)),
            alignment: ViewAlign::Center,
            bordered: true,
            rows: vec![ViewRow::from_plain("Row 1".to_owned()), row],
            selected_row: Some(0),
        };
        assert_eq!(panel.plain_text(), "Row 1\nCRED: 15000");

        let view = SeatScreenView {
            header: Some(ViewPanel {
                title: Some(ViewLabel::plain("Top".to_owned())),
                ..ViewPanel::default()
            }),
            main: panel.clone(),
            side_panels: vec![ViewPanel {
                rows: vec![ViewRow::from_plain("Side".to_owned())],
                ..ViewPanel::default()
            }],
            modal: Some(ViewPanel {
                title: Some(ViewLabel::plain("Chronicle".to_owned())),
                rows: vec![ViewRow::from_plain("Body".to_owned())],
                ..ViewPanel::default()
            }),
        };
        let flat = view.plain_text();
        assert!(flat.contains("Top"));
        assert!(flat.contains("Header"));
        assert!(flat.contains("Row 1\nCRED: 15000"));
        assert!(flat.contains("Side"));
        assert!(flat.contains("Chronicle\nBody"));

        let single = SeatScreenView::single_panel(panel);
        assert!(single.header.is_none());
        assert!(single.side_panels.is_empty());
        assert!(single.modal.is_none());

        let vp = ViewportSize::new(100, 30);
        assert_eq!(vp.width, 100);
        assert_eq!(vp.height, 30);

        let ev_q = SeatKeyEvent::new(SeatKeyCode::Char('q'), SeatKeyModifiers::NONE);
        let ev_ctrl = SeatKeyEvent::new(SeatKeyCode::Char('c'), SeatKeyModifiers::CTRL);
        assert_eq!(ev_q.code, SeatKeyCode::Char('q'));
        assert!(!ev_q.modifiers.ctrl);
        assert!(ev_ctrl.modifiers.ctrl);
        assert_eq!(SeatKeyEvent::default().code, SeatKeyCode::Unmapped);
        assert_eq!(ViewColor::default().to_owned(), ViewColor::Reset);
        assert_eq!(ViewLabel::plain("ok".to_owned()).text.to_string(), "ok");
    }
}
