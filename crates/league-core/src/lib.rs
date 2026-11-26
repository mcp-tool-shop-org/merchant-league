//! Merchant League v2 game rules and pure-state simultaneous turn engine.
//!
//! Core rules and engine state are pure Rust with zero ledger or network I/O; [`club::host`]
//! provides atomic local `.save.json` / `.seal` persistence for offline Club Mode. Every
//! simultaneous turn runs `TalkAndMarket -> Commit -> Reveal -> Sweep (8 deterministic steps)` and
//! replays byte-for-byte identically from a table's [`TableSpec`], seed, and ordered log of
//! actions, commitments, reveals, and randomness beacons.

pub mod bot;
pub mod club;
pub mod commit;
pub mod contracts;
pub mod engine;
pub mod error;
pub mod eval;
pub mod observation;
pub mod predicates;
pub mod replay;
pub mod scoring;
pub mod types;

pub use bot::{ClubBot, ClubBotDecision};
pub use club::{
    CLUB_SAVE_FORMAT_VERSION, CLUB_SEAL_FORMAT_VERSION, ClubHost, ClubSaveSnapshot,
    DEFAULT_CLUB_HUMAN_REGENT_DOCTRINE,
};
pub use eval::{
    EVAL_SAVE_FORMAT_VERSION, EVAL_SEAL_FORMAT_VERSION, EvalActApplyOutcome, EvalActSubmission,
    EvalHost, EvalSaveSnapshot, EvalSeatApplyReport,
};

pub use commit::{
    AtomicSwapProposal, CharterBidOrder, CommitmentHash, ContractDeliveryOrder, ConverterRunOrder,
    CouncilPlatform, CouncilVoteBallot, DEFAULT_SHOCK_DRAW_TURNS, DRAND_QUICKNET_GENESIS_UNIX,
    DRAND_QUICKNET_PERIOD_SECS, DRAND_QUICKNET_SIGNATURE_BYTES, DrandBeaconAuthStatus,
    DrandSchedule, DualEscrowProposal, EVAL_VRF_PROOF_BYTES, EVAL_VRF_PUBKEY_BYTES,
    EscrowFinishUnlock, GradedMandateCard, IntentPledge, IntentTarget, MandateKind, MandateReveal,
    OrderReveal, PolicyChaChaRng, PreimageSecret, PrivateMandate, ProjectDeliveryOrder, RevealSalt,
    RngDomain, SealedEvalSeed, SealedMandateDeal, SealedOrderBundle, beacon_roll_bps,
    bonus_equity_for_completion_bps, bonus_points_milli_for_completion_bps, break_tie_with_beacon,
    break_tie_with_turn_beacon, close_roll_bps_from_beacon, compute_mandate_commitment,
    compute_order_commitment, compute_preimage_condition, deal_graded_mandates,
    deal_graded_mandates_with_entropy, deal_mandate_deck_indices, default_graded_mandate_deck,
    derive_drand_beacon, derive_turn_beacon, drand_quicknet_round_for_close,
    drand_quicknet_signature_status, draw_shocks_for_turn, eval_vrf_alpha, expand_turn_stream,
    graded_mandate_deck_for_mean_project_value, standings_tie_key, validate_eval_vrf_pubkey,
    verify_and_extract_eval_vrf_beacon, verify_drand_quicknet_beacon, verify_eval_vrf_beacon,
    verify_mandate_reveal, verify_order_reveal, verify_preimage_condition,
    verify_sealed_mandate_deal, width_independent_index,
};
pub use contracts::{
    ActiveCharter, BondProposal, BondSettlementMode, BondSpec, CartelDomain, CheckProposal,
    CheckStatus, CredOffer, CreditCheck, DualEscrowVault, EscrowProposal, EscrowStatus, FillKind,
    FillRecord, MAX_CARTEL_DOMAINS, MarketAction, OfferProposal, OfferSide, PendingWashFreeze,
    TokenEscrowVault, WashFreezeSettlement, audit_wash_freeze, check_offer_domain_access,
    effective_export_tariff_bps,
};
pub use engine::{
    BrokenPromiseRecord, BundlePreview, CheckOrderPreview, CheckPreviewOutcome,
    ConverterOrderPreview, EngineState, EscrowOrderPreview, ProjectOrderPreview, SeatState,
    SeatUpkeepPreview, ShockForecastEvent, SwapOrderPreview, SwapPreviewOutcome, SweepReceipt,
    TurnEngine, TurnPhase,
};
pub use error::{StructuredError, ToStructuredError, scrub_secret_text};
pub use observation::{
    AiPreviewRequest, AuditorShockHint, CounterpartyAssumptions, DEALT_MANDATE_COMMIT_SENTINEL,
    HYPOTHETICAL_CLOSE_MARK_CAVEAT, HumanWhatIfRequest, PREVIEW_NEUTRAL_BEACON_SEED,
    PreviewTieMode, PublicCheckOrderPreview, PublicCheckOutcome, PublicPreviewState,
    PublicSwapOrderPreview, PublicSwapOutcome, PublicSweepReceipt, PublicTableSpec,
    RefusedActionPreview, SeatObservation, SeatSpoilagePreview, Step0To7SweepSummary,
    WhatIfPreview, preview_neutral_beacon, preview_seat_bundle, preview_what_if,
    preview_what_if_with_beacon,
};
pub use predicates::{
    CoalitionReach, GoodScarcity, HoldingMark, ProjectCover, ProjectInputCover, RecipeCover,
    STATE_PREDICATE_SECTION_VERSION, STATE_PREDICATES_VERSION, SeasonEndAndMarks,
    SeatSeasonMetrics, SeatStatePredicates, SeatSweepOutcome, SpoilageAndConversion,
    StatePredicateSection, TurnPredicateRecord, UpkeepAndDebt, all_coalitions_reach, cancel_locked,
    coalition_reach, compute_season_metrics, compute_seat_state_predicates,
    covered_with_own_offers_but_short_turns, held_enough_but_short_turns, scarcity_and_tariffs,
    season_end_and_marks, spoilage_and_conversion, spoiled_units_over_season,
    stranded_stock_at_close, upkeep_and_debt,
};
pub use replay::{
    LoggedTurn, REPLAY_FORMAT_VERSION, RejectedMarketAction, ReplayBeaconMode, ReplayLayoutSeed,
    ReplayLayoutSeedError, ReplayOutcome, ReplayRecord, SEALED_LAYOUT_SEED_MIN,
    SEALED_SEED_POOL_NAME, SealedReplaySeed, SeasonReplayLog, TurnRejectedActions,
    attach_state_predicates, check_replay_format_version_slice, layout_seed_digest,
    replay_club_season, replay_eval_season, replay_season, replay_season_with_state_predicates,
};
pub use scoring::{
    FinalScoreboard, FundamentalParities, SeatScoreEntry, compute_final_marked_net_asset_equity,
    compute_final_scoreboard, compute_net_asset_equity, compute_pbrs_reward, compute_potential,
    compute_table_median_pre_mandate_net_worth, compute_table_median_starting_net_worth,
    convert_mandate_points_to_equity, convert_mandate_points_to_equity_with_floor,
    convert_mandate_points_to_equity_with_median, evaluate_mandate_bonus,
    mark_good_holding_at_close, table_median_net_worth,
};
pub use types::{
    ActiveShock, AssetId, BPS_SCALE, Bps, CLOSING_ROLL_START_TURN, CLOSING_ROLL_WINDOW_LEN,
    CharterRole, ConverterRecipe, CreditTier, DEFAULT_CANCEL_LOCK_BPS, DEFAULT_CLOSING_HAZARD_BPS,
    DEFAULT_MANDATE_ALPHA_BPS, DEFAULT_PBRS_GAMMA_BPS, DEFAULT_TOWN_IMPORT_PREMIUM_BPS,
    DEFAULT_WASH_FREEZE_BOND_CRED, DEFAULT_WASH_FREEZE_BOUNTY_CRED,
    DEFAULT_WASH_ROUND_TRIP_MIN_UNITS, DEFAULT_WORKS_STIPEND_PER_UNIT_CRED, DEV_LAYOUT_SEED_MAX,
    EngineError, HARD_TURN_CAP, IDLE_MISS_TAKEOVER_TURNS, MANDATE_STARTING_NW_FLOOR_BPS,
    MAX_MANDATE_BONUS_EQUITY, MAX_TARIFF_BPS, MIN_MANDATE_BONUS_EQUITY, MarketDay, ProjectRecipe,
    RESERVED_TEST_LAYOUT_SEEDS, RegentDoctrine, ScheduledShockSpec, SeasonId, Seat, SeatConfig,
    SeatIdentity, ShockKind, TableId, TableMode, TableSpec, Turn, UNIT_SCALE,
    UNSOLD_EXCESS_PARITY_BPS, Units, cap_reach_probability, ceil_bps_fee, ceil_units_parity,
    checked_ceil_units_parity, checked_mul_units_parity, expected_season_length_turns,
    floor_bps_part, half_up_bps, is_reserved_layout_test_seed, mul_units_parity, scale_bps_signed,
    town_import_cost_cred, validate_club_dev_seed,
};

/// Host- and conformance-only raw engine inspection and full-Sweep preview entry points.
///
/// Gated behind the non-default `engine-internals` Cargo feature and forbidden in all
/// seat-facing crates (`league-tui`, `league-relay`, etc.). Seat-facing code must read
/// `OracleAuditor` hints via [`TurnEngine::observe_seat`] -> [`SeatObservation::auditor_hints`]
/// and run previews via [`preview_what_if`], [`preview_what_if_with_beacon`], or
/// [`preview_seat_bundle`].
#[cfg(feature = "engine-internals")]
#[doc(hidden)]
pub mod engine_internals {
    use std::collections::BTreeMap;

    use crate::commit::SealedOrderBundle;
    use crate::contracts::MarketAction;
    use crate::engine::{BundlePreview, TurnEngine};
    use crate::types::{EngineError, ScheduledShockSpec, Seat};

    pub use crate::engine::sweep_mode::SweepMode;

    /// Host/test-only raw Auditor hint accessor returning full [`ScheduledShockSpec`]s
    /// (including `magnitude_bps` before `realize_turn`). Forbidden in seat-facing code (`B1`).
    pub fn private_hints_for_auditor(
        engine: &TurnEngine,
        seat: Seat,
    ) -> Result<Vec<ScheduledShockSpec>, EngineError> {
        engine.private_hints_for_auditor(seat)
    }

    /// Host/conformance-only raw live-Sweep solo preview (`B2`).
    pub fn preview_bundle_live(
        engine: &TurnEngine,
        seat: Seat,
        market_actions: &[MarketAction],
        bundle: &SealedOrderBundle,
    ) -> Result<BundlePreview, EngineError> {
        engine.preview_bundle_live(seat, market_actions, bundle)
    }

    /// Host/test-only raw solo preview in an explicit [`SweepMode`] (`B2`).
    pub fn preview_bundle_in_mode(
        engine: &TurnEngine,
        seat: Seat,
        market_actions: &[MarketAction],
        bundle: &SealedOrderBundle,
        mode: SweepMode,
    ) -> Result<BundlePreview, EngineError> {
        engine.preview_bundle_in_mode(seat, market_actions, bundle, mode)
    }

    /// Host/conformance-only raw live-Sweep context-aware preview (`B2`).
    pub fn preview_bundle_with_context_live(
        engine: &TurnEngine,
        seat: Seat,
        market_actions: &[MarketAction],
        bundle: &SealedOrderBundle,
        peer_bundles: &BTreeMap<u8, SealedOrderBundle>,
        turn_beacon: [u8; 32],
    ) -> Result<BundlePreview, EngineError> {
        engine.preview_bundle_with_context_live(
            seat,
            market_actions,
            bundle,
            peer_bundles,
            turn_beacon,
        )
    }

    /// Host/test-only raw context-aware preview in an explicit [`SweepMode`] (`B2`).
    pub fn preview_bundle_with_context_and_mode(
        engine: &TurnEngine,
        seat: Seat,
        market_actions: &[MarketAction],
        bundle: &SealedOrderBundle,
        peer_bundles: &BTreeMap<u8, SealedOrderBundle>,
        turn_beacon: [u8; 32],
        mode: SweepMode,
    ) -> Result<BundlePreview, EngineError> {
        engine.preview_bundle_with_context_and_mode(
            seat,
            market_actions,
            bundle,
            peer_bundles,
            turn_beacon,
            mode,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_order_by_their_number() {
        assert!(Seat(0) < Seat(1));
        assert!(MarketDay(1) < MarketDay(12));
        assert!(Turn(1) < Turn(28));
        assert_eq!(TableId(7), TableId(7));
        assert_ne!(SeasonId(1), SeasonId(2));
    }
}
