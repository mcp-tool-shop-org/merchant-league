//! Merchant League v2 game rules and pure-state simultaneous turn engine.
//!
//! Pure Rust with zero ledger or network I/O. Every simultaneous turn runs
//! `TalkAndMarket -> Commit -> Reveal -> Sweep (8 deterministic steps)` and replays
//! byte-for-byte identically from a table's [`TableSpec`], seed, and ordered log of
//! actions, commitments, reveals, and randomness beacons.

pub mod commit;
pub mod contracts;
pub mod engine;
pub mod replay;
pub mod scoring;
pub mod types;

pub use commit::{
    AtomicSwapProposal, CharterBidOrder, CommitmentHash, ConverterRunOrder, CouncilVoteBallot,
    EscrowFinishUnlock, IntentPledge, IntentTarget, MandateKind, MandateReveal, OrderReveal,
    PreimageSecret, PrivateMandate, ProjectDeliveryOrder, RevealSalt, SealedOrderBundle,
    beacon_roll_bps, compute_mandate_commitment, compute_order_commitment,
    compute_preimage_condition, derive_turn_beacon, verify_mandate_reveal, verify_order_reveal,
};
pub use contracts::{
    ActiveCharter, BondProposal, BondSpec, CartelDomain, CheckProposal, CheckStatus, CredOffer,
    CreditCheck, EscrowProposal, EscrowStatus, MarketAction, OfferProposal, OfferSide,
    TokenEscrowVault, check_offer_domain_access, effective_export_tariff_bps,
};
pub use engine::{
    BrokenPromiseRecord, EngineState, SeatState, ShockForecastEvent, SweepReceipt, TurnEngine,
    TurnPhase,
};
pub use replay::{LoggedTurn, ReplayOutcome, SeasonReplayLog, replay_season};
pub use scoring::{
    FinalScoreboard, FundamentalParities, SeatScoreEntry, compute_final_scoreboard,
    compute_net_asset_equity, compute_pbrs_reward, compute_potential, evaluate_mandate_bonus,
};
pub use types::{
    ActiveShock, AssetId, BPS_SCALE, Bps, CharterRole, ConverterRecipe, CreditTier,
    DEFAULT_CLOSING_HAZARD_BPS, DEFAULT_PBRS_GAMMA_BPS, EngineError, MAX_TARIFF_BPS, MarketDay,
    ProjectRecipe, RegentDoctrine, ScheduledShockSpec, SeasonId, Seat, SeatConfig, SeatIdentity,
    ShockKind, TableId, TableSpec, Turn, UNIT_SCALE, Units, ceil_bps_fee, ceil_units_parity,
    floor_bps_part, half_up_bps, mul_units_parity, scale_bps_signed,
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
