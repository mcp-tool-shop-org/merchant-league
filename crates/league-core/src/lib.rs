//! Merchant League v2 game rules and pure-state simultaneous turn engine.
//!
//! Pure Rust with zero ledger or network I/O. Every simultaneous turn runs
//! `TalkAndMarket -> Commit -> Reveal -> Sweep (8 deterministic steps)` and replays
//! byte-for-byte identically from a table's [`TableSpec`], seed, and ordered log of
//! actions, commitments, reveals, and randomness beacons.

pub mod commit;
pub mod contracts;
pub mod engine;
pub mod error;
pub mod observation;
pub mod predicates;
pub mod replay;
pub mod scoring;
pub mod types;

pub use commit::{
    AtomicSwapProposal, CharterBidOrder, CommitmentHash, ContractDeliveryOrder, ConverterRunOrder,
    CouncilPlatform, CouncilVoteBallot, DEFAULT_SHOCK_DRAW_TURNS, DRAND_QUICKNET_GENESIS_UNIX,
    DRAND_QUICKNET_PERIOD_SECS, DRAND_QUICKNET_SIGNATURE_BYTES, DrandBeaconAuthStatus,
    DrandSchedule, DualEscrowProposal, EVAL_VRF_PROOF_BYTES, EVAL_VRF_PUBKEY_BYTES,
    EscrowFinishUnlock, GradedMandateCard, IntentPledge, IntentTarget, MandateKind, MandateReveal,
    OrderReveal, PreimageSecret, PrivateMandate, ProjectDeliveryOrder, RevealSalt, RngDomain,
    SealedEvalSeed, SealedMandateDeal, SealedOrderBundle, beacon_roll_bps,
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
    SeatUpkeepPreview, ShockForecastEvent, SwapOrderPreview, SwapPreviewOutcome, SweepMode,
    SweepReceipt, TurnEngine, TurnPhase,
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
