//! Pure-state simultaneous turn engine (`TalkAndMarket -> Commit -> Reveal -> 8-Step Sweep`)
//! for Merchant League v2.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use league_seat_types::{
    BrokenPromiseRecord, ConverterOrderPreview, EscrowOrderPreview, ProjectOrderPreview, SeatState,
    SeatUpkeepPreview, ShockForecastEvent, TurnPhase,
};

use crate::commit::{
    AtomicSwapProposal, CommitmentHash, ContractDeliveryOrder, ConverterRunOrder, CouncilPlatform,
    CouncilVoteBallot, DrandBeaconAuthStatus, DualEscrowProposal, EscrowFinishUnlock, IntentPledge,
    IntentTarget, MandateReveal, OrderReveal, PrivateMandate, RevealSalt, RngDomain,
    SealedOrderBundle, break_tie_with_turn_beacon, close_roll_bps_from_beacon,
    compute_order_commitment, derive_drand_beacon, derive_turn_beacon, draw_shocks_for_turn,
    expand_turn_stream, raw_zero_preimage_condition_hash, verify_and_extract_eval_vrf_beacon,
    verify_drand_quicknet_beacon, verify_mandate_reveal, verify_order_reveal,
    verify_preimage_condition,
};
use crate::contracts::{
    ActiveCharter, BondSettlementMode, BondSpec, CartelDomain, CheckStatus, CredOffer, CreditCheck,
    DualEscrowVault, EscrowProposal, EscrowStatus, FillKind, FillRecord, MAX_CARTEL_DOMAINS,
    MarketAction, OfferProposal, OfferSide, PendingWashFreeze, TokenEscrowVault,
    WashFreezeSettlement, audit_wash_freeze, check_offer_domain_access,
    effective_export_tariff_bps,
};
use crate::observation::{
    PublicSweepReceipt, RefusedActionPreview, SeatObservation,
    build_seat_observation_from_engine_state, public_sweep_receipt_from_raw,
};
use crate::predicates::compute_seat_state_predicates;
use crate::scoring::{
    FinalScoreboard, FundamentalParities, compute_final_scoreboard, compute_pbrs_reward,
    compute_potential,
};
use crate::types::{
    ActiveShock, AssetId, Bps, CLOSING_ROLL_START_TURN, CharterRole, ConverterRecipe, CreditTier,
    DEFAULT_PBRS_GAMMA_BPS, EngineError, HARD_TURN_CAP, IDLE_MISS_TAKEOVER_TURNS, MAX_TARIFF_BPS,
    ScheduledShockSpec, Seat, SeatIdentity, ShockKind, TableId, TableMode, TableSpec, Turn,
    UNIT_SCALE, Units, ceil_bps_fee, ceil_units_parity, checked_ceil_units_parity,
    checked_mul_units_parity, floor_bps_part, half_up_bps, mul_units_parity, town_import_cost_cred,
};

type PoliticsOutcome = (Option<(u8, Option<u8>)>, Vec<(CharterRole, u8)>, Units);
type Step1Outcome = (Vec<u8>, Vec<u8>, Vec<u8>, Vec<SealedOrderBundle>, Vec<bool>);

pub(crate) mod delivery_ledger_serde {
    pub(crate) use league_seat_types::wire_serde::{
        deserialize_delivery_ledger as deserialize, serialize_delivery_ledger as serialize,
    };
}

/// Complete deterministic engine snapshot serializable for replay and digest checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineState {
    /// Table identifier.
    pub table_id: TableId,
    /// Current turn index (`1..=32`).
    pub turn: u8,
    /// Current turn phase or terminal close state.
    pub phase: TurnPhase,
    /// Current Trade Minister seat index (`0..seat_count`).
    pub trade_minister: u8,
    /// Current Cartel Chair seat index (`None` when vacant at table start or after an election where
    /// the incumbent Cartel Chair becomes Trade Minister and no other seat receives chair votes).
    pub cartel_chair: Option<u8>,
    /// Immutable table rule parameters.
    pub spec: TableSpec,
    /// Precomputed fundamental shadow parities.
    pub parities: FundamentalParities,
    /// Per-seat mutable balances and reputations.
    pub seats: Vec<SeatState>,
    /// Resting `CRED`-priced DEX limit orders keyed by canonical `offer_id`.
    pub offers: BTreeMap<u64, CredOffer>,
    /// Track-2 deferred checks keyed by canonical `check_id`.
    pub checks: BTreeMap<u64, CreditCheck>,
    /// Track-1 `TokenEscrow` vaults keyed by canonical `escrow_id`.
    pub escrows: BTreeMap<u64, TokenEscrowVault>,
    /// Co-signed dual delivery-versus-payment escrows keyed by shared canonical `escrow_id` (`C2`).
    #[serde(default)]
    pub dual_escrows: BTreeMap<u64, DualEscrowVault>,
    /// Recorded Track-2 structured diplomatic pledges keyed by `pledge_id`.
    pub pledges: BTreeMap<u64, IntentPledge>,
    /// Pledge IDs already resolved (either fulfilled or flagged broken).
    pub audited_pledges: BTreeSet<u64>,
    /// Permissioned cartel trade domains (empty at Turn 1; founded and updated by elected Cartel Chairs).
    pub domains: Vec<CartelDomain>,
    /// Public council platforms filed ahead of Turn-`4n` elections via `StandForOffice`, keyed by seat.
    pub council_platforms: BTreeMap<u8, CouncilPlatform>,
    /// Active rotating Syndicate Charters.
    pub charters: Vec<ActiveCharter>,
    /// Active realized Horizon Shocks modifying turn behavior.
    pub active_shocks: Vec<ActiveShock>,
    /// Monotonic counters generating canonical object IDs.
    pub next_offer_id: u64,
    /// Next check ID.
    pub next_check_id: u64,
    /// Next escrow ID.
    pub next_escrow_id: u64,
    /// Cumulative delivered volume `(sender, receiver, asset)` used for deterministic bond & partner audits.
    #[serde(with = "delivery_ledger_serde")]
    pub delivery_ledger: BTreeMap<(u8, u8, AssetId), Units>,
    /// Optional Turn-1 Private Mandate commitments per seat.
    pub mandate_commits: Vec<Option<CommitmentHash>>,
    /// Verified Season-Close Private Mandate reveals per seat.
    pub mandate_reveals: Vec<Option<MandateReveal>>,
    /// Current turn Phase 2 commitments.
    pub turn_commits: BTreeMap<u8, CommitmentHash>,
    /// Current turn verified Phase 2 reveals.
    pub turn_reveals: BTreeMap<u8, OrderReveal>,
    /// Previous step PBRS potentials `Psi_i(s_t)` for step-wise reward deltas.
    pub last_potentials: Vec<Units>,
    /// Latest beacon roll basis points on Closing Bell evaluation (turns `24..=31`).
    pub last_closing_roll_bps: Option<Bps>,
    /// Accumulator of `CRED` spent on Phase-1 `TownImport` orders during the current turn (`C1`).
    #[serde(default)]
    pub pending_town_import_cred_spent: Units,
    /// 3-stage Horizon Shocks drawn dynamically from per-turn HKDF `"shocks"` streams (`#31`).
    #[serde(default)]
    pub drawn_shocks: Vec<ScheduledShockSpec>,
    /// Latest verified turn beacon swept (`[0u8; 32]` prior to Turn 1 Sweep), used to break exact
    /// `total_equity` ties on `FinalScoreboard` (`Decision 1`, `B-7`).
    #[serde(default)]
    pub last_turn_beacon: [u8; 32],
    /// Phase-1 cancel lock flag (`C4` amended `2026-11-17`): blocks `CancelOffer` once engaged
    /// via [`TurnEngine::begin_cancel_lock`], cleared when `TalkAndMarket` ends.
    #[serde(default)]
    pub cancel_locked: bool,
    /// Append-only per-turn settlement log across all turns (`§5.2` amended `2026-11-17`).
    #[serde(default)]
    pub fill_log: Vec<FillRecord>,
    /// Unsettled bonded `FlagWashFreeze` challenges for the current turn (`§5.2` amended `2026-11-17`).
    #[serde(default)]
    pub pending_wash_freezes: Vec<PendingWashFreeze>,
}

/// Deterministic audit receipt capturing every sub-step executed during a Turn Sweep.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SweepReceipt {
    /// Turn index that just swept.
    pub turn: u8,
    /// Seats that missed commit or valid reveal in Step 1.
    pub non_reveal_seats: Vec<u8>,
    /// Seats for which the Standing Regent acted during Sweep (after a missed reveal or 3
    /// consecutive zero-production turns from Turn 2 onward).
    pub regent_acted_seats: Vec<u8>,
    /// Seats converted from `Human` to `AiSyndicate` after 3 consecutive missed reveals or 3
    /// consecutive zero-production turns from Turn 2 onward.
    pub converted_to_ai: Vec<u8>,
    /// Escrow IDs unlocked cleanly via `EscrowFinish` or compliant bond evaluation in Step 2.
    pub escrows_finished: Vec<u64>,
    /// Escrow IDs forfeited on bond breach or refunded on cancellation in Step 2.
    pub escrows_cancelled_or_forfeited: Vec<u64>,
    /// Count of `BatchV1_1` co-signed atomic swaps settled in Step 2.
    pub atomic_swaps_settled: usize,
    /// Count of valid converter batches executed in Step 2.
    pub converter_batches_run: usize,
    /// Count of converter orders clamped (`0 < executed < requested`) in Step 2.
    pub converter_orders_clamped: usize,
    /// Check IDs cleared via `CheckCash` in Step 3.
    pub checks_cashed: Vec<u64>,
    /// Check IDs that bounced in Step 3 (triggering `HardCreditDefault` downgrades).
    pub checks_bounced: Vec<u64>,
    /// Seats evicted from cartel domains due to hitting `CreditTier::D`.
    pub cartel_evictions: Vec<u8>,
    /// Count of Tier-3 Syndicate Project batches burned in Step 4.
    pub projects_completed: usize,
    /// Count of project delivery orders clamped (`0 < executed < requested`) in Step 4.
    pub project_orders_clamped: usize,
    /// Total `CRED` auto-debt newly issued across seats for upkeep shortfalls in Step 5.
    pub total_upkeep_shortfall_debt: Units,
    /// Total perishable raw commodity units removed via `Clawback` spoilage in Step 6.
    pub total_spoilage_clawed_back: Units,
    /// Diplomatic honor violations recorded during Step 7 / Step 2 audits.
    pub broken_promises: Vec<BrokenPromiseRecord>,
    /// Elected `(trade_minister, cartel_chair)` if `turn % 4 == 0` (`cartel_chair` may be
    /// `None` if vacant after its incumbent becomes `trade_minister` with no other chair votes).
    pub elected_officers: Option<(u8, Option<u8>)>,
    /// Awarded charter `(role, winner_seat)` pairs if `turn % 6 == 0`.
    pub charters_awarded: Vec<(CharterRole, u8)>,
    /// Publicly forecasted Horizon Shocks announced in Step 8 (`forecast_turn`).
    pub public_forecasts: Vec<ShockForecastEvent>,
    /// Newly activated realized shocks enacted for `turn` (`realize_turn == turn`).
    pub newly_activated_shocks: Vec<ActiveShock>,
    /// 3-stage Horizon Shocks newly sampled from this turn's HKDF `"shocks"` stream (`#31`).
    #[serde(default)]
    pub newly_drawn_shocks: Vec<ScheduledShockSpec>,
    /// Total voluntary `CRED` auto-debt repaid in Step 2.
    pub cred_debt_repaid: Units,
    /// Total `CRED` burned by Syndicate Charter auctions in Step 7.
    pub charter_cred_burned: Units,
    /// Total `CRED` spent by seats on Authority `TownImport` fallback orders this turn (`C1`).
    #[serde(default)]
    pub town_import_cred_spent: Units,
    /// Total working-capital `CRED` stipend minted for Tier-3 Syndicate Project deliveries (`C3`).
    #[serde(default)]
    pub works_stipend_cred_minted: Units,
    /// Count of Phase-1 DEX batch trades matched at Step 0 Sweep (`C4`).
    #[serde(default)]
    pub batch_trades_cleared: usize,
    /// Every settlement record (`FillRecord`) logged during `turn` (`§5.2` amended `2026-11-17`).
    #[serde(default)]
    pub fills: Vec<FillRecord>,
    /// Customs Inspector `FlagWashFreeze` bond settlements finalized on `turn` (`§5.2` amended `2026-11-17`).
    #[serde(default)]
    pub wash_freeze_settlements: Vec<WashFreezeSettlement>,
    /// `true` if the season closed on this Sweep (moving into `AwaitingMandateReveal`).
    pub season_closed: bool,
    /// Per-seat PBRS shaped returns `F_i = gamma * Psi_{t+1} - Psi_t` (`DEFAULT_PBRS_GAMMA_BPS`).
    pub pbrs_deltas: Vec<i64>,
    /// Canonical SHA-256 digest of post-Sweep [`EngineState`].
    pub post_state_digest: [u8; 32],
}

/// Outcome of an [`AtomicSwapProposal`] simulated in [`crate::preview_seat_bundle`] or
/// [`crate::preview_what_if`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SwapPreviewOutcome {
    /// Counterparty co-signed and both parties satisfied domain/balance checks.
    Settled,
    /// Counterparty did not co-sign, one party lacked free balance, or wash/domain checks blocked it.
    UnmatchedOrInsufficientBalance,
}

/// Preview breakdown of one [`AtomicSwapProposal`] included in a previewed bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwapOrderPreview {
    /// Swap identifier from the bundle.
    pub swap_id: u64,
    /// Maker seat index.
    pub maker: u8,
    /// Taker seat index.
    pub taker: u8,
    /// Predicted Step-2 settlement status.
    pub outcome: SwapPreviewOutcome,
}

/// Outcome of a `CheckCash` request simulated in [`crate::preview_seat_bundle`] or
/// [`crate::preview_what_if`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckPreviewOutcome {
    /// Check cleared on Step 3 and credited the payee.
    Cashed,
    /// Drawer lacked `deliver_min`; check bounced and downgraded drawer credit tier.
    Bounced,
    /// Check was missing, already closed, not yet due, expired, or presented by non-payee.
    NotEligibleOrIgnored,
}

/// Preview breakdown of one `check_id` listed in `check_cashes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckOrderPreview {
    /// Check ID presented.
    pub check_id: u64,
    /// Predicted Step-3 check clearing status.
    pub outcome: CheckPreviewOutcome,
}

/// Side-effect-free simulation result returned by the internal preview helpers behind
/// `league_core::engine_internals` (host/conformance use only). Seat-facing callers must use
/// [`crate::WhatIfPreview`] via [`crate::preview_what_if`] or [`crate::preview_seat_bundle`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BundlePreview {
    /// Previewed seat index.
    pub seat: u8,
    /// Turn number simulated.
    pub turn: u8,
    /// `(action_index, error_message)` for any Phase-1 action that failed validation.
    pub refused_market_actions: Vec<(usize, String)>,
    /// Seat holdings and credit state before simulating `market_actions`.
    pub pre_seat_state: SeatState,
    /// Seat holdings after simulated `market_actions` and before Step 1–8 `Sweep`.
    pub post_market_seat_state: SeatState,
    /// Seat holdings and credit state immediately after `Sweep`.
    pub post_seat_state: SeatState,
    /// Converter execution breakdown (including clamped run counts).
    pub converter_orders: Vec<ConverterOrderPreview>,
    /// Project delivery execution breakdown (including clamped batch counts).
    pub project_orders: Vec<ProjectOrderPreview>,
    /// Atomic swap settlement outcomes for swaps listed in the previewed bundle.
    pub swap_orders: Vec<SwapOrderPreview>,
    /// Check clearing outcomes for `check_cashes` listed in the previewed bundle.
    pub check_orders: Vec<CheckOrderPreview>,
    /// Escrow unlock outcomes for `escrow_finishes` listed in the previewed bundle.
    pub escrow_orders: Vec<EscrowOrderPreview>,
    /// Debt, interest, and upkeep settlement summary for `seat`.
    pub upkeep: SeatUpkeepPreview,
    /// Perishable raw inventory + resting raw sell units removed from `seat` via Step-6 `Clawback`.
    pub seat_spoilage_clawed_back: Units,
    /// Predicted PBRS delta `F_i` for `seat` on this Sweep.
    pub seat_pbrs_delta: i64,
    /// Full post-Sweep receipt matching [`TurnEngine::sweep_turn`] for identical inputs.
    pub sweep_receipt: SweepReceipt,
}

/// Per-seat breakdown the Sweep computes alongside its [`SweepReceipt`].
pub(crate) struct SweepSeatDetails {
    converter_orders_by_seat: Vec<Vec<ConverterOrderPreview>>,
    project_orders_by_seat: Vec<Vec<ProjectOrderPreview>>,
    settled_swaps: Vec<AtomicSwapProposal>,
    /// Step-5 upkeep settlement per seat.
    pub(crate) upkeep_by_seat: Vec<SeatUpkeepPreview>,
    /// Step-6 raw units removed per seat (free balances plus resting raw sell offers).
    pub(crate) spoilage_by_seat: Vec<Units>,
}

pub(crate) mod sweep_mode {
    use serde::{Deserialize, Serialize};

    /// Controls whether a Turn Sweep executes the full 8-step live turn (including Step-8
    /// stochastic shock draws, forecasts, next-turn shock activation, closing bell roll, and
    /// `state_digest`) or only deterministic Steps 0–7 plus Step-8 end-of-turn cleanup for
    /// previews (`B2`, `B3`). Defaults to [`Self::PreviewSteps0To7`].
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
    pub enum SweepMode {
        /// Full live 8-step Sweep (draws shocks, emits forecasts, activates next-turn shocks,
        /// evaluates Closing Bell, computes `post_state_digest`, and records a
        /// [`crate::PublicSweepReceipt`]).
        Live,
        /// Preview-only Steps 0–7 plus Step-8 deterministic expiry/wash-freeze cleanup (omits shock
        /// draws, `forecast_turn == turn` forecast emission, closing roll, and next-turn activation).
        #[default]
        PreviewSteps0To7,
    }
}

pub(crate) use sweep_mode::SweepMode;

/// Deterministic pure-state v2 Turn Engine.
///
/// Serializes `state` together with `public_receipts` when non-empty (`#[serde(default,
/// skip_serializing_if = "Vec::is_empty")]`) so engine round-trips preserve
/// [`TurnEngine::public_receipts`], while [`TurnEngine::state_digest`] hashes [`EngineState`]
/// alone (keeping all state digests unchanged and pre-Card-2 JSON payloads readable).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnEngine {
    state: EngineState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    public_receipts: Vec<PublicSweepReceipt>,
}

impl TurnEngine {
    /// Initializes a new [`TurnEngine`] at Turn 1 `TalkAndMarket` after validating `spec`.
    pub fn new(spec: TableSpec) -> Result<Self, EngineError> {
        spec.validate()?;
        let n = usize::from(spec.seat_count);
        let parities = FundamentalParities::from_spec(&spec);

        let seats: Vec<SeatState> = spec
            .seats
            .iter()
            .map(|cfg| SeatState {
                seat: cfg.seat,
                identity: cfg.identity,
                cred: cfg.initial_cred,
                cred_debt: 0,
                raw: vec![0; n],
                refined: vec![0; n],
                project_equity: 0,
                raw_tariff_bps: cfg.initial_raw_tariff_bps,
                refined_tariff_bps: cfg.initial_refined_tariff_bps,
                credit_tier: CreditTier::Aaa,
                hard_credit_defaults: 0,
                bond_breaches: 0,
                broken_promises: 0,
                honor_penalties: 0,
                consecutive_misses: 0,
                consecutive_zero_production_turns: 0,
                wash_frozen_until_turn: 0,
            })
            .collect();

        let mut engine = Self {
            state: EngineState {
                table_id: spec.table_id,
                turn: 1,
                phase: TurnPhase::TalkAndMarket,
                trade_minister: 0,
                cartel_chair: None,
                spec,
                parities,
                seats,
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
                next_offer_id: 1,
                next_check_id: 1,
                next_escrow_id: 1,
                delivery_ledger: BTreeMap::new(),
                mandate_commits: vec![None; n],
                mandate_reveals: vec![None; n],
                turn_commits: BTreeMap::new(),
                turn_reveals: BTreeMap::new(),
                last_potentials: Vec::new(),
                last_closing_roll_bps: None,
                pending_town_import_cred_spent: 0,
                drawn_shocks: Vec::new(),
                last_turn_beacon: [0u8; 32],
                cancel_locked: false,
                fill_log: Vec::new(),
                pending_wash_freezes: Vec::new(),
            },
            public_receipts: Vec::new(),
        };
        engine.activate_shocks_for_turn(1);
        let initial_pots: Vec<Units> = (0..n as u8)
            .map(|s| compute_potential(&engine.state, s))
            .collect();
        engine.state.last_potentials = initial_pots;
        Ok(engine)
    }

    pub(crate) fn from_sanitized_state(
        state: EngineState,
        public_receipts: Vec<PublicSweepReceipt>,
    ) -> Self {
        Self {
            state,
            public_receipts,
        }
    }

    /// Returns an immutable reference to the full engine state snapshot.
    #[must_use]
    pub const fn state(&self) -> &EngineState {
        &self.state
    }

    /// Returns the recorded seat-safe [`PublicSweepReceipt`] history from live sweeps.
    #[must_use]
    pub fn public_receipts(&self) -> &[PublicSweepReceipt] {
        &self.public_receipts
    }

    /// Projects the sole seat-safe read boundary [`SeatObservation`] for `seat` from live state
    /// and this engine's recorded public receipts (`A1`, `C4`).
    ///
    /// Raw engine inspection (`private_hints_for_auditor`) and raw live previews (`preview_bundle`)
    /// are non-public on [`TurnEngine`]:
    ///
    /// ```compile_fail
    /// use league_core::{Seat, TableId, TableSpec, TurnEngine};
    /// let spec = TableSpec::standard_chiral_ring(TableId(1), 4, 1).unwrap();
    /// let engine = TurnEngine::new(spec).unwrap();
    /// let _ = engine.private_hints_for_auditor(Seat(0));
    /// ```
    ///
    /// ```compile_fail
    /// use league_core::{SealedOrderBundle, Seat, TableId, TableSpec, TurnEngine};
    /// let spec = TableSpec::standard_chiral_ring(TableId(1), 4, 1).unwrap();
    /// let engine = TurnEngine::new(spec).unwrap();
    /// let _ = engine.preview_bundle(Seat(0), &[], &SealedOrderBundle::default());
    /// ```
    pub fn observe_seat(&self, seat: Seat) -> Result<SeatObservation, EngineError> {
        self.observe_seat_internal(seat, None, None)
    }

    /// Host-only constructor stamping `own_mandate` into [`SeatObservation`] solely for
    /// [`crate::ClubHost::observe_seat`].
    pub(crate) fn observe_seat_with_mandate(
        &self,
        seat: Seat,
        own_mandate: PrivateMandate,
    ) -> Result<SeatObservation, EngineError> {
        self.observe_seat_internal(seat, None, Some(own_mandate))
    }

    /// Projects [`SeatObservation`] for `seat`, filtering caller-supplied raw [`SweepReceipt`]s
    /// through `public_sweep_receipt_from_raw` (retaining only `r.turn <= last_swept_turn`
    /// and dropping `newly_drawn_shocks` and `post_state_digest`).
    pub fn observe_seat_with_receipts(
        &self,
        seat: Seat,
        receipts: &[SweepReceipt],
    ) -> Result<SeatObservation, EngineError> {
        self.observe_seat_internal(seat, Some(receipts), None)
    }

    fn observe_seat_internal(
        &self,
        seat: Seat,
        caller_receipts: Option<&[SweepReceipt]>,
        own_mandate: Option<PrivateMandate>,
    ) -> Result<SeatObservation, EngineError> {
        self.ensure_seat(seat.0)?;
        let baseline_predicates = compute_seat_state_predicates(&self.state, seat)?;
        let is_auditor = self.has_active_charter(seat.0, CharterRole::OracleAuditor);
        Ok(build_seat_observation_from_engine_state(
            &self.state,
            seat,
            is_auditor,
            baseline_predicates,
            &self.public_receipts,
            caller_receipts,
            own_mandate,
        ))
    }

    /// Computes canonical SHA-256 digest of the serialized [`EngineState`].
    pub fn state_digest(&self) -> Result<[u8; 32], EngineError> {
        let encoded = serde_json::to_vec(&self.state).map_err(|err| {
            EngineError::InvalidConfig(format!("failed to serialize state digest: {err}"))
        })?;
        let mut hasher = Sha256::new();
        hasher.update(b"league-core/state/v2");
        hasher.update(encoded);
        Ok(hasher.finalize().into())
    }

    /// Returns any Turn `t` private Horizon Shock hints visible exclusively to the `OracleAuditor`.
    /// Non-public to seat code (`B1`); host/test callers go through `league_core::engine_internals`
    /// while seats use [`TurnEngine::observe_seat`] and [`SeatObservation::auditor_hints`].
    #[cfg(feature = "engine-internals")]
    pub(crate) fn private_hints_for_auditor(
        &self,
        seat: Seat,
    ) -> Result<Vec<ScheduledShockSpec>, EngineError> {
        self.ensure_seat(seat.0)?;
        if !self.has_active_charter(seat.0, CharterRole::OracleAuditor) {
            return Ok(Vec::new());
        }
        Ok(self
            .state
            .spec
            .shocks
            .iter()
            .chain(self.state.drawn_shocks.iter())
            .copied()
            .filter(|s| s.hint_turn <= self.state.turn && self.state.turn < s.forecast_turn)
            .collect())
    }

    /// Records a Turn-1 Private Charter Mandate hash commitment (`TalkAndMarket` or `Commit` of Turn 1).
    pub fn submit_mandate_commitment(
        &mut self,
        seat: Seat,
        commitment: CommitmentHash,
    ) -> Result<(), EngineError> {
        self.ensure_seat(seat.0)?;
        if commitment.is_degenerate() {
            return Err(EngineError::InvalidOrder(
                "degenerate all-zero mandate commitment is forbidden".to_owned(),
            ));
        }
        if self.state.turn != 1
            || !matches!(
                self.state.phase,
                TurnPhase::TalkAndMarket | TurnPhase::Commit
            )
        {
            return Err(EngineError::InvalidPhase(
                "Turn-1 Private Mandate commitments only open in Turn 1".to_owned(),
            ));
        }
        let idx = usize::from(seat.0);
        if self.state.mandate_commits[idx].is_some() {
            return Err(EngineError::AlreadySubmitted(seat.0));
        }
        if commitment != self.state.spec.dealt_mandate_commits[idx] {
            return Err(EngineError::CommitmentMismatch(seat.0));
        }
        self.state.mandate_commits[idx] = Some(commitment);
        Ok(())
    }

    /// Engages Phase-1 cancel lock (`C4` amended `2026-11-17`) once `(10_000 - cancel_lock_bps) /
    /// 10_000` of Phase 1 has elapsed.
    ///
    /// Valid only in [`TurnPhase::TalkAndMarket`]. A zero `spec.cancel_lock_bps` disables the
    /// lock so calling this is a valid no-op. Automatically clears when Phase 1 ends.
    pub fn begin_cancel_lock(&mut self) -> Result<(), EngineError> {
        if self.state.phase != TurnPhase::TalkAndMarket {
            return Err(EngineError::InvalidPhase(
                "cancel lock requires TalkAndMarket phase".to_owned(),
            ));
        }
        if self.state.spec.cancel_lock_bps > 0 {
            self.state.cancel_locked = true;
        }
        Ok(())
    }

    /// Applies one Phase 1 (`TalkAndMarket`) market, contract, escrow, or intent action atomically.
    pub fn apply_market_action(&mut self, action: MarketAction) -> Result<(), EngineError> {
        if self.state.phase != TurnPhase::TalkAndMarket {
            return Err(EngineError::InvalidPhase(
                "market actions require TalkAndMarket phase".to_owned(),
            ));
        }
        let snapshot = self.state.clone();
        let outcome = self.apply_market_action_inner(action);
        if outcome.is_err() {
            self.state = snapshot;
        }
        outcome
    }

    fn apply_market_action_inner(&mut self, action: MarketAction) -> Result<(), EngineError> {
        match action {
            MarketAction::PostOffer { seat, order } => self.handle_post_offer(seat, order),
            MarketAction::CancelOffer { seat, offer_id } => {
                self.ensure_seat(seat)?;
                if self.state.cancel_locked {
                    return Err(EngineError::CancelLocked);
                }
                self.handle_cancel_offer(seat, offer_id)
            }
            MarketAction::CreateCheck { seat, check } => {
                self.ensure_seat(seat)?;
                self.ensure_seat(check.payee)?;
                self.ensure_valid_asset(check.asset)?;
                if seat == check.payee
                    || check.send_max <= 0
                    || check.deliver_min <= 0
                    || check.deliver_min > check.send_max
                    || check.due_turn < self.state.turn
                    || check.expiration_turn <= check.due_turn
                {
                    return Err(EngineError::InvalidOrder(
                        "invalid CheckCreate terms".to_owned(),
                    ));
                }
                let id = self.state.next_check_id;
                self.state.next_check_id += 1;
                self.state.checks.insert(
                    id,
                    CreditCheck {
                        check_id: id,
                        drawer: seat,
                        payee: check.payee,
                        asset: check.asset,
                        send_max: check.send_max,
                        deliver_min: check.deliver_min,
                        due_turn: check.due_turn,
                        expiration_turn: check.expiration_turn,
                        status: CheckStatus::Open,
                    },
                );
                Ok(())
            }
            MarketAction::CancelCheck { seat, check_id } => {
                self.ensure_seat(seat)?;
                let check = self
                    .state
                    .checks
                    .get_mut(&check_id)
                    .ok_or_else(|| EngineError::UnknownObject(format!("check {check_id}")))?;
                if check.drawer != seat {
                    return Err(EngineError::Unauthorized(format!(
                        "seat {seat} does not own check {check_id}"
                    )));
                }
                if check.status != CheckStatus::Open {
                    return Err(EngineError::InvalidOrder(format!(
                        "check {check_id} is not open"
                    )));
                }
                check.status = CheckStatus::Cancelled;
                Ok(())
            }
            MarketAction::CreateEscrow { seat, escrow } => self.handle_create_escrow(seat, escrow),
            MarketAction::CancelExpiredEscrow { seat, escrow_id } => {
                self.ensure_seat(seat)?;
                self.refund_expired_escrow(escrow_id, self.state.turn, Some(seat))
            }
            MarketAction::RecordIntentPledge { seat, pledge } => {
                self.ensure_seat(seat)?;
                self.ensure_seat(pledge.counterparty)?;
                if pledge.promisor != seat
                    || seat == pledge.counterparty
                    || pledge.honor_weight <= 0
                {
                    return Err(EngineError::InvalidOrder(
                        "invalid Track-2 IntentPledge fields".to_owned(),
                    ));
                }
                match pledge.target {
                    IntentTarget::VoteForSeat {
                        office_turn,
                        candidate,
                    } => {
                        self.ensure_seat(candidate)?;
                        if office_turn < self.state.turn
                            || office_turn > HARD_TURN_CAP
                            || !office_turn.is_multiple_of(4)
                        {
                            return Err(EngineError::InvalidOrder(format!(
                                "invalid VoteForSeat office_turn {office_turn}"
                            )));
                        }
                    }
                    IntentTarget::PreserveCartelMembership {
                        office_turn,
                        beneficiary,
                        ..
                    } => {
                        self.ensure_seat(beneficiary)?;
                        if office_turn < self.state.turn
                            || office_turn > HARD_TURN_CAP
                            || !office_turn.is_multiple_of(4)
                        {
                            return Err(EngineError::InvalidOrder(format!(
                                "invalid PreserveCartelMembership office_turn {office_turn}"
                            )));
                        }
                    }
                    IntentTarget::RevealPreimage { escrow_id, .. } => {
                        if escrow_id == 0 {
                            return Err(EngineError::InvalidOrder(
                                "RevealPreimage escrow_id must be positive".to_owned(),
                            ));
                        }
                    }
                }
                if self.state.pledges.contains_key(&pledge.pledge_id) {
                    return Err(EngineError::InvalidOrder(format!(
                        "duplicate pledge_id {}",
                        pledge.pledge_id
                    )));
                }
                self.state.pledges.insert(pledge.pledge_id, pledge);
                Ok(())
            }
            MarketAction::FlagWashFreeze {
                inspector,
                target_seat,
            } => {
                self.ensure_seat(inspector)?;
                self.ensure_seat(target_seat)?;
                if !self.has_active_charter(inspector, CharterRole::CustomsInspector) {
                    return Err(EngineError::Unauthorized(format!(
                        "seat {inspector} does not hold CustomsInspector charter"
                    )));
                }
                if inspector == target_seat {
                    return Err(EngineError::InvalidOrder(
                        "CustomsInspector cannot freeze self".to_owned(),
                    ));
                }
                if self.state.seats[usize::from(target_seat)].wash_frozen_until_turn
                    >= self.state.turn
                {
                    return Err(EngineError::InvalidOrder(format!(
                        "seat {target_seat} is already frozen on turn {}",
                        self.state.turn
                    )));
                }
                let bond = self.state.spec.wash_freeze_bond_cred;
                self.adjust_free_balance(inspector, AssetId::Cred, -bond)?;
                self.state.seats[usize::from(target_seat)].wash_frozen_until_turn = self.state.turn;
                self.state.pending_wash_freezes.push(PendingWashFreeze {
                    turn: self.state.turn,
                    inspector,
                    target_seat,
                    bond_cred: bond,
                });
                Ok(())
            }
            MarketAction::StandForOffice { seat, platform } => {
                self.ensure_seat(seat)?;
                for &(target, _) in &platform.proposed_tariffs_bps {
                    self.ensure_seat(target)?;
                }
                let mut known_domains: BTreeSet<u8> =
                    self.state.domains.iter().map(|d| d.domain_id).collect();
                for &(dom_id, member) in &platform.cartel_admissions {
                    self.ensure_seat(member)?;
                    known_domains.insert(dom_id);
                }
                if known_domains.len() > MAX_CARTEL_DOMAINS {
                    return Err(EngineError::InvalidOrder(format!(
                        "platform would exceed MAX_CARTEL_DOMAINS ({MAX_CARTEL_DOMAINS})"
                    )));
                }
                for &(_, member) in &platform.cartel_revocations {
                    self.ensure_seat(member)?;
                }
                self.state.council_platforms.insert(seat, platform);
                Ok(())
            }
            MarketAction::TownImport { seat, good, amount } => {
                self.handle_town_import(seat, good, amount)
            }
        }
    }

    /// Transitions current turn from `TalkAndMarket` -> `Commit`, clearing any Phase-1 cancel lock.
    pub fn advance_to_commit(&mut self) -> Result<(), EngineError> {
        if self.state.phase != TurnPhase::TalkAndMarket {
            return Err(EngineError::InvalidPhase(
                "must be in TalkAndMarket to enter Commit".to_owned(),
            ));
        }
        self.state.cancel_locked = false;
        self.state.phase = TurnPhase::Commit;
        Ok(())
    }

    /// Records a seat's sealed SHA-256 Phase 2 commitment.
    pub fn submit_order_commitment(
        &mut self,
        seat: Seat,
        commitment: CommitmentHash,
    ) -> Result<(), EngineError> {
        if self.state.phase != TurnPhase::Commit {
            return Err(EngineError::InvalidPhase(
                "order commitments require Commit phase".to_owned(),
            ));
        }
        self.ensure_seat(seat.0)?;
        if commitment.is_degenerate() {
            return Err(EngineError::InvalidOrder(
                "degenerate all-zero order commitment is forbidden".to_owned(),
            ));
        }
        if self.state.turn_commits.contains_key(&seat.0) {
            return Err(EngineError::AlreadySubmitted(seat.0));
        }
        self.state.turn_commits.insert(seat.0, commitment);
        Ok(())
    }

    /// Transitions current turn from `Commit` -> `Reveal`.
    pub fn advance_to_reveal(&mut self) -> Result<(), EngineError> {
        if self.state.phase != TurnPhase::Commit {
            return Err(EngineError::InvalidPhase(
                "must be in Commit to enter Reveal".to_owned(),
            ));
        }
        self.state.phase = TurnPhase::Reveal;
        Ok(())
    }

    /// Verifies and stores a seat's Phase 2 order reveal during `Reveal`.
    ///
    /// Rejects the whole reveal (`Err`) before recording it if any `atomic_swaps` entry
    /// references an unknown asset ID for this table. Individual malformed orders inside an
    /// otherwise well-typed bundle are refused separately during the 8-step Sweep.
    pub fn submit_order_reveal(
        &mut self,
        seat: Seat,
        reveal: OrderReveal,
    ) -> Result<(), EngineError> {
        if self.state.phase != TurnPhase::Reveal {
            return Err(EngineError::InvalidPhase(
                "order reveals require Reveal phase".to_owned(),
            ));
        }
        self.ensure_seat(seat.0)?;
        let Some(&expected) = self.state.turn_commits.get(&seat.0) else {
            return Err(EngineError::CommitmentMismatch(seat.0));
        };
        if !verify_order_reveal(
            self.state.table_id,
            Turn(self.state.turn),
            seat,
            expected,
            &reveal,
        )? {
            return Err(EngineError::CommitmentMismatch(seat.0));
        }
        if self.state.turn_reveals.contains_key(&seat.0) {
            return Err(EngineError::AlreadySubmitted(seat.0));
        }
        self.validate_bundle_orders(seat.0, &reveal.bundle)?;
        self.state.turn_reveals.insert(seat.0, reveal);
        Ok(())
    }

    /// Computes the deterministic Standing Regent [`SealedOrderBundle`] for `seat` at `turn`
    /// without mutating `self`, returning `Ok(None)` when `seat` is `AiSyndicate` or has `regent: None`
    /// (`B3`).
    pub fn preview_regent_bundle_for_seat(
        &self,
        turn: Turn,
        seat: Seat,
    ) -> Result<Option<SealedOrderBundle>, EngineError> {
        self.ensure_seat(seat.0)?;
        if turn.0 == 0 || turn.0 > HARD_TURN_CAP {
            return Err(EngineError::InvalidOrder(format!(
                "regent preview turn must lie in 1..={HARD_TURN_CAP}, got {}",
                turn.0
            )));
        }
        let idx = usize::from(seat.0);
        if self.state.seats[idx].identity != SeatIdentity::Human {
            return Ok(None);
        }
        let Some(doctrine) = self.state.spec.seats[idx].regent else {
            return Ok(None);
        };
        Ok(Some(
            self.synthesize_regent_bundle(turn.0, seat.0, &doctrine),
        ))
    }

    /// Computes the Standing Regent [`SealedOrderBundle`] for `seat` at `turn` after applying
    /// valid `market_actions` and Step-0 DEX batch clearing on a cloned engine (`B3`).
    pub fn preview_regent_bundle_after_step0(
        &self,
        turn: Turn,
        seat: Seat,
        market_actions: &[MarketAction],
        turn_beacon: [u8; 32],
    ) -> Result<Option<SealedOrderBundle>, EngineError> {
        self.ensure_seat(seat.0)?;
        if turn.0 == 0 || turn.0 > HARD_TURN_CAP {
            return Err(EngineError::InvalidOrder(format!(
                "regent preview turn must lie in 1..={HARD_TURN_CAP}, got {}",
                turn.0
            )));
        }
        let mut sim = self.clone();
        if sim.state.phase == TurnPhase::TalkAndMarket {
            for action in market_actions {
                let _ = sim.apply_market_action(action.clone());
            }
        }
        sim.step0_clear_phase1_offer_batch(turn.0, turn_beacon)?;
        sim.preview_regent_bundle_for_seat(turn, seat)
    }

    /// Side-effect-free live-mode simulation of `market_actions` and `bundle` for `seat` on a
    /// cloned engine assuming empty peer bundles and the neutral preview beacon. Non-public to seat
    /// code (`B2`); host/conformance callers use `league_core::engine_internals`.
    #[cfg(feature = "engine-internals")]
    pub(crate) fn preview_bundle_live(
        &self,
        seat: Seat,
        market_actions: &[MarketAction],
        bundle: &SealedOrderBundle,
    ) -> Result<BundlePreview, EngineError> {
        self.preview_bundle_in_mode(seat, market_actions, bundle, SweepMode::Live)
    }

    /// Side-effect-free simulation of `market_actions` and `bundle` for `seat` in the specified
    /// [`SweepMode`], assuming empty peer bundles and the neutral beacon (`derive_turn_beacon(0, ...)`).
    #[cfg(feature = "engine-internals")]
    pub(crate) fn preview_bundle_in_mode(
        &self,
        seat: Seat,
        market_actions: &[MarketAction],
        bundle: &SealedOrderBundle,
        mode: SweepMode,
    ) -> Result<BundlePreview, EngineError> {
        let beacon = derive_turn_beacon(0, self.state.table_id, Turn(self.state.turn));
        self.preview_bundle_with_context_and_mode(
            seat,
            market_actions,
            bundle,
            &BTreeMap::new(),
            beacon,
            mode,
        )
    }

    /// Side-effect-free live-mode simulation of `market_actions` and `bundle` for `seat` given
    /// explicit `peer_bundles` and `turn_beacon`. Leaves `self` untouched. Non-public to seat code
    /// (`B2`); host/conformance callers use `league_core::engine_internals`.
    #[cfg(feature = "engine-internals")]
    pub(crate) fn preview_bundle_with_context_live(
        &self,
        seat: Seat,
        market_actions: &[MarketAction],
        bundle: &SealedOrderBundle,
        peer_bundles: &BTreeMap<u8, SealedOrderBundle>,
        turn_beacon: [u8; 32],
    ) -> Result<BundlePreview, EngineError> {
        self.preview_bundle_with_context_and_mode(
            seat,
            market_actions,
            bundle,
            peer_bundles,
            turn_beacon,
            SweepMode::Live,
        )
    }

    /// Side-effect-free simulation of `market_actions` and `bundle` for `seat` given explicit
    /// `peer_bundles`, `turn_beacon`, and [`SweepMode`], normalizing peer commits and reveals
    /// across `TalkAndMarket`, `Commit`, and `Reveal` (`B1`, `B3`).
    #[cfg(feature = "engine-internals")]
    pub(crate) fn preview_bundle_with_context_and_mode(
        &self,
        seat: Seat,
        market_actions: &[MarketAction],
        bundle: &SealedOrderBundle,
        peer_bundles: &BTreeMap<u8, SealedOrderBundle>,
        turn_beacon: [u8; 32],
        mode: SweepMode,
    ) -> Result<BundlePreview, EngineError> {
        let (preview, _, _) = self.preview_bundle_with_context_internal(
            seat,
            market_actions,
            None,
            &[],
            &[],
            bundle,
            peer_bundles,
            &[],
            turn_beacon,
            mode,
        )?;
        Ok(preview)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn preview_bundle_with_context_internal(
        &self,
        seat: Seat,
        market_actions: &[MarketAction],
        cancel_lock_at: Option<usize>,
        assumed_cancelled_offer_ids: &[u64],
        assumed_peer_market_actions: &[MarketAction],
        bundle: &SealedOrderBundle,
        peer_bundles: &BTreeMap<u8, SealedOrderBundle>,
        assumed_bounced_check_ids: &[u64],
        turn_beacon: [u8; 32],
        mode: SweepMode,
    ) -> Result<(BundlePreview, EngineState, Vec<RefusedActionPreview>), EngineError> {
        self.ensure_seat(seat.0)?;
        for &peer in peer_bundles.keys() {
            self.ensure_seat(peer)?;
        }
        if !matches!(
            self.state.phase,
            TurnPhase::TalkAndMarket | TurnPhase::Commit | TurnPhase::Reveal
        ) {
            return Err(EngineError::InvalidPhase(
                "preview_bundle requires an active turn phase".to_owned(),
            ));
        }
        self.validate_bundle_orders(seat.0, bundle)?;
        for (&peer_s, peer_bundle) in peer_bundles {
            self.validate_bundle_orders(peer_s, peer_bundle)?;
        }

        let turn = self.state.turn;
        let seat_idx = usize::from(seat.0);
        let n = self.state.seats.len();
        let was_talk_and_market = self.state.phase == TurnPhase::TalkAndMarket;

        let mut sim = self.clone();
        let pre_seat_state = sim.state.seats[seat_idx].clone();
        let mut refused_market_actions = Vec::new();
        let mut refused_details = Vec::new();
        if was_talk_and_market {
            if !sim.state.cancel_locked {
                for &offer_id in assumed_cancelled_offer_ids {
                    if let Some(maker) = sim.state.offers.get(&offer_id).map(|o| o.maker) {
                        let _ = sim.apply_market_action(MarketAction::CancelOffer {
                            seat: maker,
                            offer_id,
                        });
                    }
                }
            }
            for (idx, action) in market_actions.iter().enumerate() {
                if cancel_lock_at == Some(idx) {
                    let _ = sim.begin_cancel_lock();
                }
                if let Err(err) = sim.apply_market_action(action.clone()) {
                    let msg = err.to_string();
                    refused_details.push(RefusedActionPreview {
                        action_index: idx,
                        error_code: err.code().to_owned(),
                        message: crate::error::scrub_secret_text(&msg),
                    });
                    refused_market_actions.push((idx, msg));
                }
            }
            if cancel_lock_at == Some(market_actions.len()) {
                let _ = sim.begin_cancel_lock();
            }
            for action in assumed_peer_market_actions {
                let _ = sim.apply_market_action(action.clone());
            }
        } else if !market_actions.is_empty() {
            return Err(EngineError::InvalidPhase(
                "market_actions require TalkAndMarket phase".to_owned(),
            ));
        }
        let post_market_seat_state = sim.state.seats[seat_idx].clone();

        for &check_id in assumed_bounced_check_ids {
            if let Some(check) = sim.state.checks.get_mut(&check_id)
                && check.status == CheckStatus::Open
                && check.drawer != seat.0
            {
                check.deliver_min = i64::MAX;
            }
        }

        // B1 normalization: clear any existing Phase-2 commits or reveals across TalkAndMarket,
        // Commit, and Reveal so uncommitted peers or partial revealers never distort preview state.
        sim.state.turn_commits.clear();
        sim.state.turn_reveals.clear();

        let preview_salt = RevealSalt([0xA5; 32]);
        let mut insert_preview_reveal = |s: u8, b: &SealedOrderBundle| -> Result<(), EngineError> {
            let rev = OrderReveal {
                bundle: b.clone(),
                salt: preview_salt.clone(),
            };
            let ch = compute_order_commitment(sim.state.table_id, Turn(turn), Seat(s), &rev)?;
            sim.state.turn_commits.insert(s, ch);
            sim.state.turn_reveals.insert(s, rev);
            Ok(())
        };

        insert_preview_reveal(seat.0, bundle)?;
        for (&peer_s, peer_bundle) in peer_bundles {
            if peer_s != seat.0 {
                insert_preview_reveal(peer_s, peer_bundle)?;
            }
        }
        let empty = SealedOrderBundle::default();
        for s in 0..n as u8 {
            if s != seat.0 && !peer_bundles.contains_key(&s) {
                insert_preview_reveal(s, &empty)?;
            }
        }
        sim.state.cancel_locked = false;
        sim.state.phase = TurnPhase::Reveal;

        let (sweep_receipt, details) = sim.sweep_turn_detailed_with_mode(turn_beacon, mode)?;
        let post_seat_state = sim.state.seats[seat_idx].clone();
        let swap_orders = bundle
            .atomic_swaps
            .iter()
            .map(|swap| SwapOrderPreview {
                swap_id: swap.swap_id,
                maker: swap.maker,
                taker: swap.taker,
                outcome: if details.settled_swaps.contains(swap) {
                    SwapPreviewOutcome::Settled
                } else {
                    SwapPreviewOutcome::UnmatchedOrInsufficientBalance
                },
            })
            .collect();
        let check_orders = bundle
            .check_cashes
            .iter()
            .map(|&cid| CheckOrderPreview {
                check_id: cid,
                outcome: if sweep_receipt.checks_cashed.contains(&cid) {
                    CheckPreviewOutcome::Cashed
                } else if sweep_receipt.checks_bounced.contains(&cid) {
                    CheckPreviewOutcome::Bounced
                } else {
                    CheckPreviewOutcome::NotEligibleOrIgnored
                },
            })
            .collect();
        let escrow_orders = bundle
            .escrow_finishes
            .iter()
            .map(|fin| EscrowOrderPreview {
                escrow_id: fin.escrow_id,
                unlocked: sweep_receipt.escrows_finished.contains(&fin.escrow_id),
            })
            .collect();

        let preview = BundlePreview {
            seat: seat.0,
            turn,
            refused_market_actions,
            pre_seat_state,
            post_market_seat_state,
            post_seat_state,
            converter_orders: details.converter_orders_by_seat[seat_idx].clone(),
            project_orders: details.project_orders_by_seat[seat_idx].clone(),
            swap_orders,
            check_orders,
            escrow_orders,
            upkeep: details.upkeep_by_seat[seat_idx],
            seat_spoilage_clawed_back: details.spoilage_by_seat[seat_idx],
            seat_pbrs_delta: sweep_receipt.pbrs_deltas[seat_idx],
            sweep_receipt,
        };
        Ok((preview, sim.state, refused_details))
    }

    /// Executes the 8-step deterministic `Sweep` for the current standalone Eval-mode turn using
    /// `turn_beacon`.
    ///
    /// Refuses `TableMode::Game` tables (which must sweep via [`TurnEngine::sweep_turn_with_drand`]),
    /// refuses `TableMode::Club` tables (which must sweep via [`TurnEngine::sweep_turn_with_eval_vrf`]),
    /// and refuses `TableMode::Eval` tables bound to an `eval_vrf_pubkey` (`B-3`).
    pub fn sweep_turn(&mut self, turn_beacon: [u8; 32]) -> Result<SweepReceipt, EngineError> {
        match self.state.spec.mode {
            TableMode::Game => {
                return Err(EngineError::InvalidConfig(
                    "Game mode requires sweep_turn_with_drand with scheduled round and signature"
                        .to_owned(),
                ));
            }
            TableMode::Club => {
                return Err(EngineError::InvalidConfig(
                    "Club mode requires sweep_turn_with_eval_vrf".to_owned(),
                ));
            }
            TableMode::Eval => {}
        }
        if self.state.spec.eval_vrf_pubkey.is_some() {
            return Err(EngineError::InvalidConfig(
                "Eval table with eval_vrf_pubkey requires sweep_turn_with_eval_vrf".to_owned(),
            ));
        }
        Ok(self.sweep_turn_detailed(turn_beacon)?.0)
    }

    /// Executes the 8-step deterministic `Sweep` for a standalone Eval-mode turn (`eval_vrf_pubkey == None`)
    /// using a beacon derived from `beacon_seed`. Refuses tables carrying `eval_vrf_pubkey` (`B-3`).
    pub fn sweep_turn_seeded(&mut self, beacon_seed: u64) -> Result<SweepReceipt, EngineError> {
        if self.state.spec.mode != TableMode::Eval {
            return Err(EngineError::InvalidConfig(
                "sweep_turn_seeded requires TableMode::Eval".to_owned(),
            ));
        }
        if self.state.spec.eval_vrf_pubkey.is_some() {
            return Err(EngineError::InvalidConfig(
                "Eval table with eval_vrf_pubkey requires sweep_turn_with_eval_vrf".to_owned(),
            ));
        }
        let beacon = derive_turn_beacon(beacon_seed, self.state.table_id, Turn(self.state.turn));
        self.sweep_turn(beacon)
    }

    /// Executes the 8-step deterministic `Sweep` for an Eval- or Club-mode turn by verifying an
    /// `EVAL_VRF_PROOF_BYTES` proof (RFC 9381 ECVRF construction with C2SP's `ristretto255`
    /// ciphersuite, `vrf-r255 0.1.0`) against `spec.eval_vrf_pubkey` (`#31` comment).
    pub fn sweep_turn_with_eval_vrf(
        &mut self,
        vrf_proof: &[u8],
    ) -> Result<SweepReceipt, EngineError> {
        Ok(self.sweep_turn_with_eval_vrf_detailed(vrf_proof)?.0)
    }

    /// [`TurnEngine::sweep_turn_with_eval_vrf`], also returning the Sweep's per-seat breakdown.
    pub(crate) fn sweep_turn_with_eval_vrf_detailed(
        &mut self,
        vrf_proof: &[u8],
    ) -> Result<(SweepReceipt, SweepSeatDetails), EngineError> {
        if !matches!(self.state.spec.mode, TableMode::Eval | TableMode::Club) {
            return Err(EngineError::InvalidConfig(
                "sweep_turn_with_eval_vrf requires TableMode::Eval or TableMode::Club".to_owned(),
            ));
        }
        if self.state.phase != TurnPhase::Reveal {
            return Err(EngineError::InvalidPhase(
                "sweep_turn_with_eval_vrf requires Reveal phase".to_owned(),
            ));
        }
        let Some(eval_pk) = self.state.spec.eval_vrf_pubkey else {
            return Err(EngineError::InvalidConfig(
                "sweep_turn_with_eval_vrf requires TableSpec::eval_vrf_pubkey".to_owned(),
            ));
        };
        let beacon = verify_and_extract_eval_vrf_beacon(
            eval_pk,
            self.state.table_id,
            Turn(self.state.turn),
            vrf_proof,
        )?;
        self.sweep_turn_detailed(beacon)
    }

    /// Executes the 8-step deterministic `Sweep` for a Game-mode turn using an `ExternalDrand`
    /// `quicknet` round and 48-byte signature validated against `spec.drand_schedule` (`M2`) and
    /// verified offline against [`crate::commit::DRAND_QUICKNET_PUBLIC_KEY`] (`#29`).
    ///
    /// Derives `turn_beacon = SHA-256(drand_signature)` and requires `drand_round` to equal
    /// `schedule.expected_round_for_turn(current_turn)`.
    pub fn sweep_turn_with_drand(
        &mut self,
        drand_round: u64,
        drand_signature: &[u8],
    ) -> Result<SweepReceipt, EngineError> {
        Ok(self
            .sweep_turn_with_drand_detailed(drand_round, drand_signature)?
            .0)
    }

    /// [`TurnEngine::sweep_turn_with_drand`], also returning the Sweep's per-seat breakdown.
    pub(crate) fn sweep_turn_with_drand_detailed(
        &mut self,
        drand_round: u64,
        drand_signature: &[u8],
    ) -> Result<(SweepReceipt, SweepSeatDetails), EngineError> {
        if self.state.spec.mode != TableMode::Game {
            return Err(EngineError::InvalidConfig(
                "sweep_turn_with_drand requires TableMode::Game".to_owned(),
            ));
        }
        if self.state.phase != TurnPhase::Reveal {
            return Err(EngineError::InvalidPhase(
                "sweep_turn_with_drand requires Reveal phase".to_owned(),
            ));
        }
        let Some(schedule) = self.state.spec.drand_schedule else {
            return Err(EngineError::InvalidConfig(
                "sweep_turn_with_drand requires TableSpec::drand_schedule".to_owned(),
            ));
        };
        let beacon = derive_drand_beacon(drand_signature)?;
        let DrandBeaconAuthStatus::Authenticated = verify_drand_quicknet_beacon(
            &schedule,
            Turn(self.state.turn),
            drand_round,
            drand_signature,
            beacon,
        )?;
        self.sweep_turn_detailed(beacon)
    }

    fn sweep_turn_detailed(
        &mut self,
        turn_beacon: [u8; 32],
    ) -> Result<(SweepReceipt, SweepSeatDetails), EngineError> {
        self.sweep_turn_detailed_with_mode(turn_beacon, SweepMode::Live)
    }

    fn sweep_turn_detailed_with_mode(
        &mut self,
        turn_beacon: [u8; 32],
        sweep_mode: SweepMode,
    ) -> Result<(SweepReceipt, SweepSeatDetails), EngineError> {
        if self.state.phase != TurnPhase::Reveal {
            return Err(EngineError::InvalidPhase(
                "sweep_turn requires Reveal phase".to_owned(),
            ));
        }
        let turn = self.state.turn;
        let n = self.state.seats.len();
        self.state.last_turn_beacon = turn_beacon;
        let prev_outbound_delivered = self.outbound_delivered_by_seat();

        // Step 0: Simultaneously clear Phase 1 resting DEX offers in batch without arrival-order priority.
        let (batch_trades_cleared, batch_filled_seats) =
            self.step0_clear_phase1_offer_batch(turn, turn_beacon)?;
        let town_import_cred_spent = std::mem::take(&mut self.state.pending_town_import_cred_spent);

        // Step 1: Validate reveals, run Regent fallback (non-committers only), track 3-miss AI conversion.
        let (
            non_reveal_seats,
            mut regent_acted_seats,
            mut converted_to_ai,
            active_bundles,
            committed_non_revealers,
        ) = self.step1_validate_reveals_and_regents(turn, n);

        // Step 2: Settle EscrowFinish reveals, co-signed DualEscrows, Batch swaps, Contract deliveries,
        // Steward bond & expiry checks, voluntary auto-debt repayments, and Tier-2 converters.
        let mut broken_promises = Vec::new();
        let mut escrows_finished = self.step2a_finish_preimage_escrows(turn, &active_bundles);
        self.step2_open_dual_escrows(turn, &active_bundles);
        let (atomic_swaps_settled, settled_swaps) =
            self.step2_settle_atomic_swaps(turn, &active_bundles)?;
        let mut escrows_cancelled_or_forfeited = Vec::new();
        self.step2_settle_contract_deliveries(
            turn,
            &active_bundles,
            &mut escrows_finished,
            &mut escrows_cancelled_or_forfeited,
        );
        let (bond_finished, bond_cancelled) =
            self.step2b_evaluate_bonds_and_expired_escrows(turn, &mut broken_promises);
        escrows_finished.extend(bond_finished);
        escrows_cancelled_or_forfeited.extend(bond_cancelled);
        let (cred_debt_repaid, repaid_by_seat) =
            self.step2_voluntary_debt_repayment(&active_bundles);
        let (
            mut converter_batches_run,
            mut converter_orders_clamped,
            mut converter_orders_by_seat,
            seat_converter_batches,
        ) = self.step2_settle_converters(&active_bundles);

        // Step 3: Clear maturing Track-2 CheckCash requests, apply Check bounce downgrades,
        // and settle FlagWashFreeze challenger bonds against previous-turn fills.
        let (mut checks_cashed, mut checks_bounced, mut cartel_evictions) =
            self.step3_clear_checks(turn, &active_bundles);
        let wash_freeze_settlements = self.step3_settle_wash_freezes(turn);

        // Step 4: Burn Tier-3 Syndicate Projects (+ Works CRED stipend), evaluate C1a idle-seat
        // Regent takeover (after converters, deliveries, batch fills, and works deliveries are all
        // known, skipping Turn 1), and mint Tier-1 raw extraction yields.
        let (
            projects_completed,
            project_orders_clamped,
            works_stipend_cred_minted,
            project_orders_by_seat,
            seat_works_batches,
        ) = self.step4a_project_deliveries(turn, &active_bundles);
        self.evaluate_c1a_idle_takeover(
            turn,
            &committed_non_revealers,
            &seat_converter_batches,
            &prev_outbound_delivered,
            &batch_filled_seats,
            &seat_works_batches,
            &mut converter_batches_run,
            &mut converter_orders_clamped,
            &mut converter_orders_by_seat,
            &mut checks_cashed,
            &mut checks_bounced,
            &mut cartel_evictions,
            &mut regent_acted_seats,
            &mut converted_to_ai,
        );
        self.step4b_mint_raw_yields(turn);

        // Step 5: Accrue auto-debt interest & collect chiral upkeep (issuing shortfall auto-debt).
        let (total_upkeep_shortfall_debt, upkeep_by_seat) =
            self.step5_collect_upkeep_and_debt(&repaid_by_seat);

        // Step 6: Apply perishable Clawback spoilage on remaining raw goods.
        let (total_spoilage_clawed_back, spoilage_by_seat) =
            self.step6_apply_perishable_spoilage(turn);

        // Step 7: Audit vote pledges, settle 4-turn council politics, and 6-turn charters.
        let (elected_officers, charters_awarded, charter_cred_burned) = self
            .step7_politics_and_charters(turn, turn_beacon, &active_bundles, &mut broken_promises);

        // Step 8: Advance Horizon Shocks and evaluate stochastic close roll (Turns 24..=32).
        let (newly_drawn_shocks, public_forecasts, newly_activated_shocks, season_closed) =
            match sweep_mode {
                SweepMode::Live => self.step8_shocks_and_close(turn, turn_beacon),
                SweepMode::PreviewSteps0To7 => {
                    self.step8_deterministic_cleanup_only(turn);
                    (Vec::new(), Vec::new(), Vec::new(), false)
                }
            };

        // Compute dense PBRS deltas using updated post-Sweep potentials.
        let mut pbrs_deltas = Vec::with_capacity(n);
        let mut new_potentials = Vec::with_capacity(n);
        for s in 0..n as u8 {
            let prev = self.state.last_potentials[usize::from(s)];
            let next = compute_potential(&self.state, s);
            pbrs_deltas.push(compute_pbrs_reward(prev, next, DEFAULT_PBRS_GAMMA_BPS));
            new_potentials.push(next);
        }
        self.state.last_potentials = new_potentials;

        // Clear turn phase buffers and transition phase cleanly.
        self.state.turn_commits.clear();
        self.state.turn_reveals.clear();
        if season_closed {
            self.state.phase = TurnPhase::AwaitingMandateReveal;
        } else {
            self.state.turn = turn.saturating_add(1);
            if sweep_mode == SweepMode::Live {
                self.activate_shocks_for_turn(turn + 1);
            }
            self.state.phase = TurnPhase::TalkAndMarket;
        }

        let fills: Vec<FillRecord> = self
            .state
            .fill_log
            .iter()
            .filter(|r| r.turn == turn)
            .cloned()
            .collect();
        let post_state_digest = match sweep_mode {
            SweepMode::Live => self.state_digest()?,
            SweepMode::PreviewSteps0To7 => [0u8; 32],
        };
        let receipt = SweepReceipt {
            turn,
            non_reveal_seats,
            regent_acted_seats,
            converted_to_ai,
            escrows_finished,
            escrows_cancelled_or_forfeited,
            atomic_swaps_settled,
            converter_batches_run,
            converter_orders_clamped,
            checks_cashed,
            checks_bounced,
            cartel_evictions,
            projects_completed,
            project_orders_clamped,
            total_upkeep_shortfall_debt,
            total_spoilage_clawed_back,
            broken_promises,
            elected_officers,
            charters_awarded,
            public_forecasts,
            newly_activated_shocks,
            newly_drawn_shocks,
            cred_debt_repaid,
            charter_cred_burned,
            town_import_cred_spent,
            works_stipend_cred_minted,
            batch_trades_cleared,
            fills,
            wash_freeze_settlements,
            season_closed,
            pbrs_deltas,
            post_state_digest,
        };
        if sweep_mode == SweepMode::Live {
            self.public_receipts
                .push(public_sweep_receipt_from_raw(&receipt));
        }
        let details = SweepSeatDetails {
            converter_orders_by_seat,
            project_orders_by_seat,
            settled_swaps,
            upkeep_by_seat,
            spoilage_by_seat,
        };
        Ok((receipt, details))
    }

    /// Reveals a seat's Turn-1 Private Charter Mandate after Season Close.
    pub fn submit_mandate_reveal(
        &mut self,
        seat: Seat,
        reveal: MandateReveal,
    ) -> Result<(), EngineError> {
        if self.state.phase != TurnPhase::AwaitingMandateReveal {
            return Err(EngineError::InvalidPhase(
                "mandate reveals require AwaitingMandateReveal phase".to_owned(),
            ));
        }
        self.ensure_seat(seat.0)?;
        reveal
            .mandate
            .kind
            .validate_for_seat_count(self.state.spec.seat_count)?;
        let idx = usize::from(seat.0);
        let dealt_commitment = self.state.spec.dealt_mandate_commits[idx];
        let commitment = match self.state.mandate_commits[idx] {
            Some(echoed) if echoed != dealt_commitment => {
                return Err(EngineError::CommitmentMismatch(seat.0));
            }
            Some(echoed) => echoed,
            // F2: Turn-1 commitment echo is optional; fall back to the public dealt commitment.
            None => dealt_commitment,
        };
        if !verify_mandate_reveal(self.state.table_id, seat, commitment, &reveal)? {
            return Err(EngineError::CommitmentMismatch(seat.0));
        }
        if !self
            .state
            .spec
            .mandate_deck
            .iter()
            .any(|c| c.kind == reveal.mandate.kind && c.bonus_equity == reveal.mandate.bonus_equity)
        {
            return Err(EngineError::InvalidOrder(format!(
                "seat {} mandate reveal is not a card in table mandate_deck",
                seat.0
            )));
        }
        self.state.mandate_reveals[idx] = Some(reveal);
        Ok(())
    }

    /// Finalizes closed season standings (`Completed`) and returns [`FinalScoreboard`].
    pub fn finalize_season(&mut self) -> Result<FinalScoreboard, EngineError> {
        if self.state.phase != TurnPhase::AwaitingMandateReveal {
            return Err(EngineError::InvalidPhase(
                "finalize_season requires AwaitingMandateReveal".to_owned(),
            ));
        }
        let board = compute_final_scoreboard(&self.state, &self.state.mandate_reveals)?;
        self.state.phase = TurnPhase::Completed;
        Ok(board)
    }

    fn ensure_seat(&self, seat: u8) -> Result<(), EngineError> {
        if usize::from(seat) < self.state.seats.len() {
            Ok(())
        } else {
            Err(EngineError::InvalidSeat(seat))
        }
    }

    fn ensure_valid_asset(&self, asset: AssetId) -> Result<(), EngineError> {
        let n = self.state.seats.len() as u8;
        match asset {
            AssetId::Cred => Ok(()),
            AssetId::Raw(g) | AssetId::Refined(g) if g < n => Ok(()),
            AssetId::Raw(g) | AssetId::Refined(g) => Err(EngineError::InvalidSeat(g)),
        }
    }

    fn ensure_valid_good(&self, asset: AssetId) -> Result<(), EngineError> {
        let n = self.state.seats.len() as u8;
        match asset {
            AssetId::Cred => Err(EngineError::InvalidOrder(
                "DEX book requires a commodity asset, not CRED".to_owned(),
            )),
            AssetId::Raw(g) | AssetId::Refined(g) if g < n => Ok(()),
            AssetId::Raw(g) | AssetId::Refined(g) => Err(EngineError::InvalidSeat(g)),
        }
    }

    fn validate_bundle_orders(
        &self,
        actor: u8,
        bundle: &SealedOrderBundle,
    ) -> Result<(), EngineError> {
        for swap in &bundle.atomic_swaps {
            self.ensure_valid_asset(swap.maker_asset)?;
            self.ensure_valid_asset(swap.taker_asset)?;
        }
        for cd in &bundle.contract_deliveries {
            self.ensure_seat(cd.counterparty)?;
            self.ensure_valid_good(cd.good)?;
            if cd.counterparty == actor || cd.gross_amount <= 0 {
                return Err(EngineError::InvalidOrder(
                    "invalid contract delivery counterparty or amount".to_owned(),
                ));
            }
        }
        for de in &bundle.dual_escrows {
            self.ensure_seat(de.buyer)?;
            self.ensure_seat(de.seller)?;
            self.ensure_valid_good(de.good)?;
            let combined_within_cap = de
                .buyer_bond_cred
                .checked_add(de.seller_bond_cred)
                .is_some_and(|sum| sum <= de.payment_cred);
            if de.buyer == de.seller
                || de.required_units <= 0
                || de.payment_cred <= 0
                || de.buyer_bond_cred <= 0
                || de.seller_bond_cred <= 0
                || !combined_within_cap
                || de.deadline_turn < self.state.turn
                || de.deadline_turn > HARD_TURN_CAP
            {
                return Err(EngineError::InvalidOrder(
                    "invalid or over-capped DualEscrowProposal (combined bond must stay <= 100% of payment_cred)".to_owned(),
                ));
            }
        }
        Ok(())
    }

    fn has_active_charter(&self, seat: u8, role: CharterRole) -> bool {
        self.state
            .charters
            .iter()
            .any(|c| c.role == role && c.holder == seat && c.expires_at_turn >= self.state.turn)
    }

    fn effective_asset_tariff_bps(
        &self,
        asset: AssetId,
        domain_id: Option<u8>,
        sender: u8,
        receiver: u8,
    ) -> Bps {
        effective_transfer_tariff_bps(&self.state, asset, domain_id, sender, receiver)
    }

    fn free_balance(&self, seat: u8, asset: AssetId) -> Units {
        let Some(s) = self.state.seats.get(usize::from(seat)) else {
            return 0;
        };
        match asset {
            AssetId::Cred => s.cred,
            AssetId::Raw(g) => s.raw.get(usize::from(g)).copied().unwrap_or(0),
            AssetId::Refined(m) => s.refined.get(usize::from(m)).copied().unwrap_or(0),
        }
    }

    fn adjust_free_balance(
        &mut self,
        seat: u8,
        asset: AssetId,
        delta: Units,
    ) -> Result<(), EngineError> {
        let s = self
            .state
            .seats
            .get_mut(usize::from(seat))
            .ok_or(EngineError::InvalidSeat(seat))?;
        let slot = match asset {
            AssetId::Cred => &mut s.cred,
            AssetId::Raw(g) => s
                .raw
                .get_mut(usize::from(g))
                .ok_or(EngineError::InvalidSeat(g))?,
            AssetId::Refined(m) => s
                .refined
                .get_mut(usize::from(m))
                .ok_or(EngineError::InvalidSeat(m))?,
        };
        let next = (*slot)
            .checked_add(delta)
            .ok_or_else(|| EngineError::InvalidOrder("balance overflow".to_owned()))?;
        if next < 0 {
            return Err(EngineError::InsufficientBalance(format!(
                "seat {seat} insufficient {:?}",
                asset
            )));
        }
        *slot = next;
        Ok(())
    }

    fn outbound_delivered_by_seat(&self) -> Vec<i128> {
        let mut out = vec![0_i128; self.state.seats.len()];
        for (&(from, _, _), &amt) in &self.state.delivery_ledger {
            if let Some(slot) = out.get_mut(usize::from(from)) {
                *slot += i128::from(amt);
            }
        }
        out
    }

    fn record_delivery(&mut self, from: u8, to: u8, asset: AssetId, net_units: Units) {
        if net_units > 0 {
            let slot = self
                .state
                .delivery_ledger
                .entry((from, to, asset))
                .or_insert(0);
            *slot = slot.saturating_add(net_units);
        }
    }

    fn implied_unit_price_cred(cred_amount: Units, commodity_units: Units) -> Option<Units> {
        if cred_amount <= 0 || commodity_units <= 0 {
            return None;
        }
        let scaled = i128::from(cred_amount).checked_mul(i128::from(UNIT_SCALE))?;
        Units::try_from(scaled / i128::from(commodity_units)).ok()
    }

    fn handle_town_import(
        &mut self,
        seat: u8,
        good: AssetId,
        amount: Units,
    ) -> Result<(), EngineError> {
        self.ensure_seat(seat)?;
        self.ensure_valid_good(good)?;
        if amount <= 0 {
            return Err(EngineError::InvalidOrder(
                "town import amount must be positive".to_owned(),
            ));
        }
        let parity = self.state.parities.asset_parity(good);
        let cost = town_import_cost_cred(amount, parity, self.state.spec.town_import_premium_bps)?;
        self.adjust_free_balance(seat, AssetId::Cred, -cost)?;
        if let Err(err) = self.adjust_free_balance(seat, good, amount) {
            let _ = self.adjust_free_balance(seat, AssetId::Cred, cost);
            return Err(err);
        }
        self.state.pending_town_import_cred_spent = self
            .state
            .pending_town_import_cred_spent
            .saturating_add(cost);
        self.state.fill_log.push(FillRecord {
            turn: self.state.turn,
            kind: FillKind::TownImport,
            good: Some(good),
            from: None,
            to: seat,
            units: amount,
            net_units: amount,
            cred_amount: cost,
            unit_price_cred: Self::implied_unit_price_cred(cost, amount),
        });
        Ok(())
    }

    fn handle_post_offer(&mut self, seat: u8, order: OfferProposal) -> Result<(), EngineError> {
        self.ensure_seat(seat)?;
        self.ensure_valid_good(order.good)?;
        if order.amount <= 0 || order.price_per_unit_cred <= 0 {
            return Err(EngineError::InvalidOrder(
                "offer amount and price must be positive".to_owned(),
            ));
        }
        if self.state.seats[usize::from(seat)].wash_frozen_until_turn >= self.state.turn {
            return Err(EngineError::Unauthorized(format!(
                "seat {seat} frozen by CustomsInspector this turn"
            )));
        }
        let broker = self.has_active_charter(seat, CharterRole::ClearinghouseBroker);
        check_offer_domain_access(
            seat,
            order.domain_id,
            order.is_hybrid,
            self.state.seats[usize::from(seat)].credit_tier,
            &self.state.domains,
            broker,
        )?;

        let max_lock = checked_ceil_units_parity(order.amount, order.price_per_unit_cred)?;
        let locked_cred = match order.side {
            OfferSide::Buy => {
                if self.free_balance(seat, AssetId::Cred) < max_lock {
                    return Err(EngineError::InsufficientBalance(format!(
                        "seat {seat} needs {max_lock} CRED to post buy offer"
                    )));
                }
                self.adjust_free_balance(seat, AssetId::Cred, -max_lock)?;
                max_lock
            }
            OfferSide::Sell => {
                if self.free_balance(seat, order.good) < order.amount {
                    return Err(EngineError::InsufficientBalance(format!(
                        "seat {seat} lacks {} units to sell",
                        order.amount
                    )));
                }
                self.adjust_free_balance(seat, order.good, -order.amount)?;
                0
            }
        };

        // C4: PostOffer only escrows funds onto the resting book; all crossing happens in
        // simultaneous uniform-price batch clearing at Sweep Step 0.
        let offer_id = self.state.next_offer_id;
        self.state.next_offer_id += 1;
        self.state.offers.insert(
            offer_id,
            CredOffer {
                offer_id,
                maker: seat,
                side: order.side,
                good: order.good,
                remaining_units: order.amount,
                price_per_unit_cred: order.price_per_unit_cred,
                locked_cred,
                domain_id: order.domain_id,
                is_hybrid: order.is_hybrid,
                created_turn: self.state.turn,
            },
        );
        Ok(())
    }

    fn batch_tie_key(
        table_id: TableId,
        turn_beacon: &[u8; 32],
        turn: u8,
        good: AssetId,
        side: OfferSide,
        maker: u8,
    ) -> [u8; 32] {
        let (gk, gi) = match good {
            AssetId::Cred => (0u8, 0u8),
            AssetId::Raw(g) => (1u8, g),
            AssetId::Refined(m) => (2u8, m),
        };
        let sb = match side {
            OfferSide::Buy => 0u8,
            OfferSide::Sell => 1u8,
        };
        expand_turn_stream(
            *turn_beacon,
            table_id,
            Turn(turn),
            RngDomain::BatchTie,
            &[gk, gi, sb, maker],
        )
    }

    fn offers_pair_compatible(&self, buy: &CredOffer, sell: &CredOffer) -> bool {
        if buy.maker == sell.maker {
            return false;
        }
        let buy_broker =
            buy.is_hybrid && self.has_active_charter(buy.maker, CharterRole::ClearinghouseBroker);
        let sell_broker =
            sell.is_hybrid && self.has_active_charter(sell.maker, CharterRole::ClearinghouseBroker);
        match (buy.domain_id, sell.domain_id) {
            (None, None) => true,
            (Some(bd), Some(sd)) if bd == sd => self
                .state
                .domains
                .iter()
                .find(|d| d.domain_id == bd)
                .is_some_and(|d| {
                    (d.members.contains(&buy.maker) || buy_broker)
                        && (d.members.contains(&sell.maker) || sell_broker)
                }),
            (None, Some(_)) | (Some(_), None) | (Some(_), Some(_)) => buy_broker || sell_broker,
        }
    }

    fn step0_clear_phase1_offer_batch(
        &mut self,
        turn: u8,
        turn_beacon: [u8; 32],
    ) -> Result<(usize, Vec<bool>), EngineError> {
        #[derive(Clone, Copy)]
        struct BatchFill {
            buy_id: u64,
            sell_id: u64,
            buy_maker: u8,
            sell_maker: u8,
            buy_limit: Units,
            sell_limit: Units,
            fill_units: Units,
            tariff_dom: Option<u8>,
        }

        let n = self.state.seats.len() as u8;
        let goods: Vec<AssetId> = (0..n)
            .map(AssetId::Raw)
            .chain((0..n).map(AssetId::Refined))
            .collect();
        let mut trades_cleared = 0usize;
        let mut batch_filled_seats = vec![false; usize::from(n)];

        for good in goods {
            let mut buys: Vec<CredOffer> = Vec::new();
            let mut sells: Vec<CredOffer> = Vec::new();
            for offer in self.state.offers.values() {
                if offer.good != good || offer.remaining_units <= 0 {
                    continue;
                }
                let maker_state = &self.state.seats[usize::from(offer.maker)];
                if maker_state.wash_frozen_until_turn >= turn {
                    continue;
                }
                if offer.domain_id.is_some() && !maker_state.credit_tier.may_access_prime_cartel() {
                    continue;
                }
                match offer.side {
                    OfferSide::Buy => buys.push(offer.clone()),
                    OfferSide::Sell => sells.push(offer.clone()),
                }
            }
            if buys.is_empty() || sells.is_empty() {
                continue;
            }

            // Arrival-order invariant sorting: price priority, then HKDF batch_tie tie-break across distinct makers.
            let table_id = self.state.table_id;
            buys.sort_by(|a, b| {
                b.price_per_unit_cred
                    .cmp(&a.price_per_unit_cred)
                    .then_with(|| {
                        Self::batch_tie_key(
                            table_id,
                            &turn_beacon,
                            turn,
                            good,
                            OfferSide::Buy,
                            a.maker,
                        )
                        .cmp(&Self::batch_tie_key(
                            table_id,
                            &turn_beacon,
                            turn,
                            good,
                            OfferSide::Buy,
                            b.maker,
                        ))
                    })
                    .then_with(|| a.maker.cmp(&b.maker))
                    .then_with(|| b.remaining_units.cmp(&a.remaining_units))
                    .then_with(|| a.created_turn.cmp(&b.created_turn))
                    .then_with(|| a.domain_id.cmp(&b.domain_id))
                    .then_with(|| a.is_hybrid.cmp(&b.is_hybrid))
                    .then_with(|| a.offer_id.cmp(&b.offer_id))
            });
            sells.sort_by(|a, b| {
                a.price_per_unit_cred
                    .cmp(&b.price_per_unit_cred)
                    .then_with(|| {
                        Self::batch_tie_key(
                            table_id,
                            &turn_beacon,
                            turn,
                            good,
                            OfferSide::Sell,
                            a.maker,
                        )
                        .cmp(&Self::batch_tie_key(
                            table_id,
                            &turn_beacon,
                            turn,
                            good,
                            OfferSide::Sell,
                            b.maker,
                        ))
                    })
                    .then_with(|| a.maker.cmp(&b.maker))
                    .then_with(|| b.remaining_units.cmp(&a.remaining_units))
                    .then_with(|| a.created_turn.cmp(&b.created_turn))
                    .then_with(|| a.domain_id.cmp(&b.domain_id))
                    .then_with(|| a.is_hybrid.cmp(&b.is_hybrid))
                    .then_with(|| a.offer_id.cmp(&b.offer_id))
            });

            let mut b_rem: Vec<Units> = buys.iter().map(|o| o.remaining_units).collect();
            let mut s_rem: Vec<Units> = sells.iter().map(|o| o.remaining_units).collect();
            let mut by_dom: BTreeMap<Option<u8>, Vec<BatchFill>> = BTreeMap::new();

            for (bi, buy) in buys.iter().enumerate() {
                while b_rem[bi] > 0 {
                    let Some(si) = sells.iter().enumerate().position(|(idx, sell)| {
                        s_rem[idx] > 0
                            && buy.price_per_unit_cred >= sell.price_per_unit_cred
                            && self.offers_pair_compatible(buy, sell)
                    }) else {
                        break;
                    };
                    let sell = &sells[si];
                    let fill_units = b_rem[bi].min(s_rem[si]);
                    b_rem[bi] -= fill_units;
                    s_rem[si] -= fill_units;
                    let tariff_dom = sell.domain_id.or(buy.domain_id);
                    by_dom.entry(tariff_dom).or_default().push(BatchFill {
                        buy_id: buy.offer_id,
                        sell_id: sell.offer_id,
                        buy_maker: buy.maker,
                        sell_maker: sell.maker,
                        buy_limit: buy.price_per_unit_cred,
                        sell_limit: sell.price_per_unit_cred,
                        fill_units,
                        tariff_dom,
                    });
                }
            }

            for fills in by_dom.into_values() {
                let max_sell = fills.iter().map(|f| f.sell_limit).max().unwrap_or(0);
                let min_buy = fills.iter().map(|f| f.buy_limit).min().unwrap_or(max_sell);
                let raw_mid = ((i128::from(max_sell) + i128::from(min_buy)) / 2) as Units;
                for fill in fills {
                    let clearing_price = raw_mid.clamp(fill.sell_limit, fill.buy_limit);
                    let computed_cred = checked_mul_units_parity(fill.fill_units, clearing_price)?;
                    let (trade_cred, buyer_refund) = {
                        let Some(buy_entry) = self.state.offers.get_mut(&fill.buy_id) else {
                            continue;
                        };
                        buy_entry.remaining_units -= fill.fill_units;
                        let new_req_lock = if buy_entry.remaining_units <= 0 {
                            0
                        } else {
                            checked_ceil_units_parity(
                                buy_entry.remaining_units,
                                buy_entry.price_per_unit_cred,
                            )?
                        };
                        let unlocked = buy_entry.locked_cred.saturating_sub(new_req_lock);
                        let paid = computed_cred.min(unlocked).max(
                            if fill.fill_units > 0 && clearing_price > 0 && unlocked > 0 {
                                1
                            } else {
                                0
                            },
                        );
                        let refund = unlocked.saturating_sub(paid);
                        buy_entry.locked_cred = new_req_lock;
                        if buy_entry.remaining_units <= 0 {
                            self.state.offers.remove(&fill.buy_id);
                        }
                        (paid, refund)
                    };
                    if let Some(sell_entry) = self.state.offers.get_mut(&fill.sell_id) {
                        sell_entry.remaining_units -= fill.fill_units;
                        if sell_entry.remaining_units <= 0 {
                            self.state.offers.remove(&fill.sell_id);
                        }
                    }

                    self.adjust_free_balance(fill.sell_maker, AssetId::Cred, trade_cred)?;
                    if buyer_refund > 0 {
                        self.adjust_free_balance(fill.buy_maker, AssetId::Cred, buyer_refund)?;
                    }
                    let tariff_bps = self.effective_asset_tariff_bps(
                        good,
                        fill.tariff_dom,
                        fill.sell_maker,
                        fill.buy_maker,
                    );
                    let net_commodity = fill.fill_units - ceil_bps_fee(fill.fill_units, tariff_bps);
                    self.adjust_free_balance(fill.buy_maker, good, net_commodity)?;
                    self.record_delivery(fill.sell_maker, fill.buy_maker, good, net_commodity);
                    if fill.fill_units > 0 {
                        batch_filled_seats[usize::from(fill.buy_maker)] = true;
                        batch_filled_seats[usize::from(fill.sell_maker)] = true;
                        self.state.fill_log.push(FillRecord {
                            turn,
                            kind: FillKind::BatchFill,
                            good: Some(good),
                            from: Some(fill.sell_maker),
                            to: fill.buy_maker,
                            units: fill.fill_units,
                            net_units: net_commodity,
                            cred_amount: trade_cred,
                            unit_price_cred: Some(clearing_price),
                        });
                    }
                    trades_cleared += 1;
                }
            }
        }
        Ok((trades_cleared, batch_filled_seats))
    }

    fn handle_cancel_offer(&mut self, seat: u8, offer_id: u64) -> Result<(), EngineError> {
        self.ensure_seat(seat)?;
        let Some(offer) = self.state.offers.remove(&offer_id) else {
            return Err(EngineError::UnknownObject(format!("offer {offer_id}")));
        };
        if offer.maker != seat {
            self.state.offers.insert(offer_id, offer);
            return Err(EngineError::Unauthorized(format!(
                "seat {seat} does not own offer {offer_id}"
            )));
        }
        match offer.side {
            OfferSide::Buy => self.adjust_free_balance(seat, AssetId::Cred, offer.locked_cred),
            OfferSide::Sell => self.adjust_free_balance(seat, offer.good, offer.remaining_units),
        }
    }

    fn handle_create_escrow(
        &mut self,
        seat: u8,
        proposal: EscrowProposal,
    ) -> Result<(), EngineError> {
        self.ensure_seat(seat)?;
        self.ensure_seat(proposal.destination)?;
        if proposal.asset != AssetId::Cred {
            return Err(EngineError::PerishableEscrowForbidden);
        }
        let has_bond = proposal.bond.is_some();
        if seat == proposal.destination
            || proposal.amount <= 0
            || proposal.cancel_after_turn <= self.state.turn
            || (!has_bond
                && proposal.finish_after_turn.is_none()
                && proposal.condition_hash.is_none())
        {
            return Err(EngineError::InvalidOrder(
                "invalid TokenEscrow vault parameters".to_owned(),
            ));
        }
        if let Some(cond) = proposal.condition_hash
            && (cond.is_degenerate() || cond == raw_zero_preimage_condition_hash())
        {
            return Err(EngineError::InvalidOrder(
                "degenerate escrow condition_hash is forbidden".to_owned(),
            ));
        }
        if let Some(fa) = proposal.finish_after_turn
            && fa >= proposal.cancel_after_turn
        {
            return Err(EngineError::InvalidOrder(
                "TokenEscrow finish_after must precede cancel_after".to_owned(),
            ));
        }
        let bond = match proposal.bond {
            Some(bp) => {
                self.ensure_valid_good(bp.good)?;
                if bp.required_units <= 0
                    || bp.deadline_turn < self.state.turn
                    || bp.deadline_turn >= proposal.cancel_after_turn
                {
                    return Err(EngineError::InvalidOrder(
                        "invalid Steward Bond deadline or required_units".to_owned(),
                    ));
                }
                let (obligor, beneficiary) = match bp.mode {
                    BondSettlementMode::SellerPerformanceBond => (seat, proposal.destination),
                    BondSettlementMode::BuyerDvpPayment => (proposal.destination, seat),
                };
                let baseline = self
                    .state
                    .delivery_ledger
                    .get(&(obligor, beneficiary, bp.good))
                    .copied()
                    .unwrap_or(0);
                Some(BondSpec {
                    obligor,
                    beneficiary,
                    good: bp.good,
                    required_units: bp.required_units,
                    baseline_delivered: baseline,
                    deadline_turn: bp.deadline_turn,
                    mode: bp.mode,
                })
            }
            None => None,
        };

        self.adjust_free_balance(seat, AssetId::Cred, -proposal.amount)?;
        let id = self.state.next_escrow_id;
        self.state.next_escrow_id += 1;
        self.state.escrows.insert(
            id,
            TokenEscrowVault {
                escrow_id: id,
                sender: seat,
                destination: proposal.destination,
                amount_cred: proposal.amount,
                finish_after_turn: proposal.finish_after_turn,
                cancel_after_turn: proposal.cancel_after_turn,
                condition_hash: proposal.condition_hash,
                bond,
                status: EscrowStatus::Locked,
            },
        );
        Ok(())
    }

    fn refund_expired_escrow(
        &mut self,
        escrow_id: u64,
        turn: u8,
        requester: Option<u8>,
    ) -> Result<(), EngineError> {
        let Some(escrow) = self.state.escrows.get(&escrow_id).cloned() else {
            return Err(EngineError::UnknownObject(format!("escrow {escrow_id}")));
        };
        if let Some(req) = requester
            && req != escrow.sender
            && req != escrow.destination
        {
            return Err(EngineError::Unauthorized(format!(
                "seat {req} cannot cancel escrow {escrow_id}"
            )));
        }
        if escrow.status != EscrowStatus::Locked {
            return Err(EngineError::InvalidOrder(format!(
                "escrow {escrow_id} is not locked"
            )));
        }
        if turn < escrow.cancel_after_turn {
            return Err(EngineError::InvalidPhase(format!(
                "escrow {escrow_id} has not reached cancel_after_turn"
            )));
        }
        if let Some(slot) = self.state.escrows.get_mut(&escrow_id) {
            slot.status = EscrowStatus::Cancelled;
        }
        self.adjust_free_balance(escrow.sender, AssetId::Cred, escrow.amount_cred)?;
        if escrow.amount_cred > 0 {
            self.state.fill_log.push(FillRecord {
                turn: self.state.turn,
                kind: FillKind::EscrowRefunded,
                good: Some(AssetId::Cred),
                from: Some(escrow.sender),
                to: escrow.sender,
                units: escrow.amount_cred,
                net_units: escrow.amount_cred,
                cred_amount: escrow.amount_cred,
                unit_price_cred: None,
            });
        }
        Ok(())
    }

    fn step1_validate_reveals_and_regents(&mut self, turn: u8, n: usize) -> Step1Outcome {
        let mut non_reveal_seats = Vec::new();
        let mut regent_acted_seats = Vec::new();
        let mut converted_to_ai = Vec::new();
        let mut bundles = vec![SealedOrderBundle::default(); n];
        let mut committed_non_revealers = vec![false; n];

        for s in 0..n as u8 {
            let idx = usize::from(s);
            let did_commit = self.state.turn_commits.contains_key(&s);
            if let Some(rev) = self.state.turn_reveals.get(&s) {
                self.state.seats[idx].consecutive_misses = 0;
                bundles[idx] = rev.bundle.clone();
            } else {
                non_reveal_seats.push(s);
                committed_non_revealers[idx] = did_commit;
                self.state.seats[idx].consecutive_misses =
                    self.state.seats[idx].consecutive_misses.saturating_add(1);
                if !did_commit
                    && self.state.seats[idx].identity == SeatIdentity::Human
                    && let Some(doctrine) = self.state.spec.seats[idx].regent
                {
                    bundles[idx] = self.synthesize_regent_bundle(turn, s, &doctrine);
                    regent_acted_seats.push(s);
                }
                if self.state.seats[idx].identity == SeatIdentity::Human
                    && self.state.seats[idx].consecutive_misses >= 3
                {
                    self.state.seats[idx].identity = SeatIdentity::AiSyndicate;
                    converted_to_ai.push(s);
                }
            }
        }
        (
            non_reveal_seats,
            regent_acted_seats,
            converted_to_ai,
            bundles,
            committed_non_revealers,
        )
    }

    fn synthesize_regent_bundle(
        &self,
        turn: u8,
        seat: u8,
        doctrine: &crate::types::RegentDoctrine,
    ) -> SealedOrderBundle {
        let mut check_cashes = Vec::new();
        if doctrine.auto_cash_due_claims {
            for check in self.state.checks.values() {
                if check.status == CheckStatus::Open
                    && check.payee == seat
                    && check.due_turn <= turn
                    && turn < check.expiration_turn
                {
                    check_cashes.push(check.check_id);
                }
            }
        }
        let mut converter_runs = Vec::new();
        let seat_cfg = &self.state.spec.seats[usize::from(seat)];
        let seat_state = &self.state.seats[usize::from(seat)];
        let mut avail_raw = seat_state.raw.clone();
        for (recipe_idx, conv) in seat_cfg.converters.iter().enumerate() {
            let mut max_possible = conv.max_runs_per_turn;
            for &(raw_g, amt) in &conv.inputs {
                let avail = avail_raw
                    .get(usize::from(raw_g))
                    .copied()
                    .unwrap_or(0)
                    .saturating_sub(doctrine.min_raw_reserve)
                    .max(0);
                let possible = (avail / amt).max(0) as u32;
                max_possible = max_possible.min(possible);
            }
            if max_possible > 0 {
                for &(raw_g, amt) in &conv.inputs {
                    if let Some(slot) = avail_raw.get_mut(usize::from(raw_g)) {
                        *slot = slot.saturating_sub(amt.saturating_mul(i64::from(max_possible)));
                    }
                }
                converter_runs.push(ConverterRunOrder {
                    recipe_idx,
                    runs: max_possible,
                });
            }
        }
        SealedOrderBundle {
            converter_runs,
            check_cashes,
            ..SealedOrderBundle::default()
        }
    }

    fn step2a_finish_preimage_escrows(
        &mut self,
        turn: u8,
        bundles: &[SealedOrderBundle],
    ) -> Vec<u64> {
        let mut finished = Vec::new();

        // Collect all EscrowFinish unlocks deterministically by (escrow_id, unlock_seat).
        let mut requests: Vec<(u64, u8, EscrowFinishUnlock)> = Vec::new();
        for (s_idx, b) in bundles.iter().enumerate() {
            for fin in &b.escrow_finishes {
                requests.push((fin.escrow_id, s_idx as u8, fin.clone()));
            }
        }
        requests.sort_by_key(|r| (r.0, r.1));

        for (escrow_id, requester, fin) in requests {
            let Some(vault) = self.state.escrows.get(&escrow_id).cloned() else {
                continue;
            };
            if vault.status != EscrowStatus::Locked
                || vault.bond.is_some()
                || (requester != vault.sender && requester != vault.destination)
                || turn < vault.finish_after_turn.unwrap_or(1)
                || turn >= vault.cancel_after_turn
            {
                continue;
            }
            let cond_ok = match (&vault.condition_hash, &fin.preimage) {
                (None, _) => true,
                (Some(expected), Some(preimage)) => {
                    verify_preimage_condition(*expected, preimage).unwrap_or(false)
                }
                (Some(_), None) => false,
            };
            if !cond_ok {
                continue;
            }
            if let Some(slot) = self.state.escrows.get_mut(&escrow_id) {
                slot.status = EscrowStatus::Finished;
            }
            let _ = self.adjust_free_balance(vault.destination, AssetId::Cred, vault.amount_cred);
            self.state.fill_log.push(FillRecord {
                turn,
                kind: FillKind::EscrowFinished,
                good: Some(AssetId::Cred),
                from: Some(vault.sender),
                to: vault.destination,
                units: vault.amount_cred,
                net_units: vault.amount_cred,
                cred_amount: vault.amount_cred,
                unit_price_cred: None,
            });
            finished.push(escrow_id);

            // Resolve matching RevealPreimage pledges cleanly as honored.
            for pledge in self.state.pledges.values() {
                if let IntentTarget::RevealPreimage { escrow_id: pid, .. } = pledge.target
                    && pid == escrow_id
                {
                    self.state.audited_pledges.insert(pledge.pledge_id);
                }
            }
        }
        finished
    }

    fn dual_parties_share_domain(&self, dom_id: u8, a: u8, b: u8) -> bool {
        self.state
            .domains
            .iter()
            .find(|d| d.domain_id == dom_id)
            .is_some_and(|d| {
                d.members.contains(&a)
                    && d.members.contains(&b)
                    && self.state.seats[usize::from(a)]
                        .credit_tier
                        .may_access_prime_cartel()
                    && self.state.seats[usize::from(b)]
                        .credit_tier
                        .may_access_prime_cartel()
            })
    }

    fn step2_open_dual_escrows(&mut self, turn: u8, bundles: &[SealedOrderBundle]) {
        let mut signers_map: BTreeMap<DualEscrowProposal, BTreeSet<u8>> = BTreeMap::new();
        for (seat_idx, bundle) in bundles.iter().enumerate() {
            let s = seat_idx as u8;
            for prop in &bundle.dual_escrows {
                let cap_ok = prop
                    .buyer_bond_cred
                    .checked_add(prop.seller_bond_cred)
                    .is_some_and(|sum| sum <= prop.payment_cred);
                if (s == prop.buyer || s == prop.seller)
                    && prop.buyer != prop.seller
                    && self.ensure_seat(prop.buyer).is_ok()
                    && self.ensure_seat(prop.seller).is_ok()
                    && self.ensure_valid_good(prop.good).is_ok()
                    && prop.required_units > 0
                    && prop.payment_cred > 0
                    && prop.buyer_bond_cred > 0
                    && prop.seller_bond_cred > 0
                    && cap_ok
                    && prop.deadline_turn >= turn
                    && prop.deadline_turn <= HARD_TURN_CAP
                {
                    signers_map.entry(prop.clone()).or_default().insert(s);
                }
            }
        }

        for (prop, signers) in signers_map {
            if !signers.contains(&prop.buyer) || !signers.contains(&prop.seller) {
                continue;
            }
            if self.state.seats[usize::from(prop.buyer)].wash_frozen_until_turn >= turn
                || self.state.seats[usize::from(prop.seller)].wash_frozen_until_turn >= turn
            {
                continue;
            }
            if let Some(dom_id) = prop.domain_id
                && !self.dual_parties_share_domain(dom_id, prop.buyer, prop.seller)
            {
                continue;
            }
            if self.free_balance(prop.buyer, AssetId::Cred) < prop.buyer_bond_cred
                || self.free_balance(prop.seller, AssetId::Cred) < prop.seller_bond_cred
            {
                continue;
            }
            let _ = self.adjust_free_balance(prop.buyer, AssetId::Cred, -prop.buyer_bond_cred);
            let _ = self.adjust_free_balance(prop.seller, AssetId::Cred, -prop.seller_bond_cred);
            let escrow_id = self.state.next_escrow_id;
            self.state.next_escrow_id += 1;
            let baseline_delivered = self
                .state
                .delivery_ledger
                .get(&(prop.seller, prop.buyer, prop.good))
                .copied()
                .unwrap_or(0);
            self.state.dual_escrows.insert(
                escrow_id,
                DualEscrowVault {
                    escrow_id,
                    buyer: prop.buyer,
                    seller: prop.seller,
                    good: prop.good,
                    required_units: prop.required_units,
                    payment_cred: prop.payment_cred,
                    buyer_bond_cred: prop.buyer_bond_cred,
                    seller_bond_cred: prop.seller_bond_cred,
                    baseline_delivered,
                    deadline_turn: prop.deadline_turn,
                    domain_id: prop.domain_id,
                    status: EscrowStatus::Locked,
                },
            );
        }
    }

    fn step2_settle_contract_deliveries(
        &mut self,
        turn: u8,
        bundles: &[SealedOrderBundle],
        finished: &mut Vec<u64>,
        cancelled: &mut Vec<u64>,
    ) {
        let mut orders: Vec<(u8, ContractDeliveryOrder)> = Vec::new();
        for (s_idx, bundle) in bundles.iter().enumerate() {
            let seller = s_idx as u8;
            for cd in &bundle.contract_deliveries {
                orders.push((seller, cd.clone()));
            }
        }
        orders.sort();

        for (seller, cd) in orders {
            if seller == cd.counterparty
                || cd.gross_amount <= 0
                || self.ensure_seat(cd.counterparty).is_err()
                || self.ensure_valid_good(cd.good).is_err()
            {
                continue;
            }
            if self.state.seats[usize::from(seller)].wash_frozen_until_turn >= turn
                || self.state.seats[usize::from(cd.counterparty)].wash_frozen_until_turn >= turn
            {
                continue;
            }
            if let Some(dom_id) = cd.domain_id
                && !self.dual_parties_share_domain(dom_id, seller, cd.counterparty)
            {
                continue;
            }
            if self.free_balance(seller, cd.good) < cd.gross_amount {
                continue;
            }
            let tariff_bps =
                self.effective_asset_tariff_bps(cd.good, cd.domain_id, seller, cd.counterparty);
            let net_units = cd.gross_amount - ceil_bps_fee(cd.gross_amount, tariff_bps);

            if let Some(eid) = cd.dual_escrow_id {
                let Some(dual) = self.state.dual_escrows.get(&eid).cloned() else {
                    continue;
                };
                if dual.status != EscrowStatus::Locked
                    || dual.seller != seller
                    || dual.buyer != cd.counterparty
                    || dual.good != cd.good
                    || turn > dual.deadline_turn
                    || net_units < dual.required_units
                {
                    continue;
                }
                let buyer_top_up = dual.payment_cred.saturating_sub(dual.buyer_bond_cred);
                if self.free_balance(dual.buyer, AssetId::Cred) >= buyer_top_up {
                    let _ = self.adjust_free_balance(seller, cd.good, -cd.gross_amount);
                    let _ = self.adjust_free_balance(cd.counterparty, cd.good, net_units);
                    self.record_delivery(seller, cd.counterparty, cd.good, net_units);
                    let _ = self.adjust_free_balance(dual.buyer, AssetId::Cred, -buyer_top_up);
                    let _ = self.adjust_free_balance(
                        dual.seller,
                        AssetId::Cred,
                        dual.payment_cred.saturating_add(dual.seller_bond_cred),
                    );
                    if let Some(slot) = self.state.dual_escrows.get_mut(&eid) {
                        slot.status = EscrowStatus::Finished;
                    }
                    self.state.fill_log.push(FillRecord {
                        turn,
                        kind: FillKind::ContractDelivery,
                        good: Some(cd.good),
                        from: Some(seller),
                        to: cd.counterparty,
                        units: cd.gross_amount,
                        net_units,
                        cred_amount: dual.payment_cred,
                        unit_price_cred: Self::implied_unit_price_cred(
                            dual.payment_cred,
                            cd.gross_amount,
                        ),
                    });
                    finished.push(eid);
                } else {
                    // Buyer lacks top-up CRED at delivery: Seller keeps goods & collects buyer bond.
                    self.settle_dual_escrow_default(
                        turn,
                        eid,
                        dual.buyer,
                        dual.seller,
                        dual.buyer_bond_cred,
                        dual.seller_bond_cred,
                    );
                    cancelled.push(eid);
                }
            } else {
                let _ = self.adjust_free_balance(seller, cd.good, -cd.gross_amount);
                let _ = self.adjust_free_balance(cd.counterparty, cd.good, net_units);
                self.record_delivery(seller, cd.counterparty, cd.good, net_units);
                self.state.fill_log.push(FillRecord {
                    turn,
                    kind: FillKind::ContractDelivery,
                    good: Some(cd.good),
                    from: Some(seller),
                    to: cd.counterparty,
                    units: cd.gross_amount,
                    net_units,
                    cred_amount: 0,
                    unit_price_cred: None,
                });
            }
        }
    }

    fn settle_dual_escrow_default(
        &mut self,
        turn: u8,
        eid: u64,
        defaulter: u8,
        counterparty: u8,
        forfeited_bond: Units,
        refunded_bond: Units,
    ) {
        let _ = self.adjust_free_balance(
            counterparty,
            AssetId::Cred,
            refunded_bond.saturating_add(forfeited_bond),
        );
        if let Some(slot) = self.state.dual_escrows.get_mut(&eid) {
            slot.status = EscrowStatus::Cancelled;
        }
        let d_seat = &mut self.state.seats[usize::from(defaulter)];
        d_seat.bond_breaches += 1;
        d_seat.honor_penalties = d_seat.honor_penalties.saturating_add(forfeited_bond);
        self.state.fill_log.push(FillRecord {
            turn,
            kind: FillKind::EscrowForfeited,
            good: Some(AssetId::Cred),
            from: Some(defaulter),
            to: counterparty,
            units: forfeited_bond,
            net_units: forfeited_bond,
            cred_amount: forfeited_bond,
            unit_price_cred: None,
        });
        if refunded_bond > 0 {
            self.state.fill_log.push(FillRecord {
                turn,
                kind: FillKind::EscrowRefunded,
                good: Some(AssetId::Cred),
                from: Some(counterparty),
                to: counterparty,
                units: refunded_bond,
                net_units: refunded_bond,
                cred_amount: refunded_bond,
                unit_price_cred: None,
            });
        }
    }

    fn step2b_evaluate_bonds_and_expired_escrows(
        &mut self,
        turn: u8,
        broken_promises: &mut Vec<BrokenPromiseRecord>,
    ) -> (Vec<u64>, Vec<u64>) {
        let mut finished = Vec::new();
        let mut cancelled = Vec::new();

        // Evaluate unilateral TokenEscrow vaults & Steward-refereed bonds.
        let vault_ids: Vec<u64> = self.state.escrows.keys().copied().collect();
        for escrow_id in vault_ids {
            let Some(vault) = self.state.escrows.get(&escrow_id).cloned() else {
                continue;
            };
            if vault.status != EscrowStatus::Locked {
                continue;
            }
            if let Some(bond) = vault.bond {
                let cum_now = self
                    .state
                    .delivery_ledger
                    .get(&(bond.obligor, bond.beneficiary, bond.good))
                    .copied()
                    .unwrap_or(0);
                let delta_delivered = cum_now.saturating_sub(bond.baseline_delivered);
                match bond.mode {
                    BondSettlementMode::SellerPerformanceBond => {
                        if bond.deadline_turn <= turn {
                            if delta_delivered >= bond.required_units {
                                if let Some(slot) = self.state.escrows.get_mut(&escrow_id) {
                                    slot.status = EscrowStatus::Finished;
                                }
                                let _ = self.adjust_free_balance(
                                    vault.sender,
                                    AssetId::Cred,
                                    vault.amount_cred,
                                );
                                self.state.fill_log.push(FillRecord {
                                    turn,
                                    kind: FillKind::EscrowFinished,
                                    good: Some(AssetId::Cred),
                                    from: Some(vault.sender),
                                    to: vault.sender,
                                    units: vault.amount_cred,
                                    net_units: vault.amount_cred,
                                    cred_amount: vault.amount_cred,
                                    unit_price_cred: None,
                                });
                                finished.push(escrow_id);
                            } else {
                                if let Some(slot) = self.state.escrows.get_mut(&escrow_id) {
                                    slot.status = EscrowStatus::Cancelled;
                                }
                                let _ = self.adjust_free_balance(
                                    vault.destination,
                                    AssetId::Cred,
                                    vault.amount_cred,
                                );
                                let ob = &mut self.state.seats[usize::from(bond.obligor)];
                                ob.bond_breaches += 1;
                                ob.honor_penalties =
                                    ob.honor_penalties.saturating_add(vault.amount_cred);
                                self.state.fill_log.push(FillRecord {
                                    turn,
                                    kind: FillKind::EscrowForfeited,
                                    good: Some(AssetId::Cred),
                                    from: Some(vault.sender),
                                    to: vault.destination,
                                    units: vault.amount_cred,
                                    net_units: vault.amount_cred,
                                    cred_amount: vault.amount_cred,
                                    unit_price_cred: None,
                                });
                                cancelled.push(escrow_id);
                            }
                        }
                        continue;
                    }
                    BondSettlementMode::BuyerDvpPayment => {
                        if delta_delivered >= bond.required_units && turn <= bond.deadline_turn {
                            if let Some(slot) = self.state.escrows.get_mut(&escrow_id) {
                                slot.status = EscrowStatus::Finished;
                            }
                            let _ = self.adjust_free_balance(
                                vault.destination,
                                AssetId::Cred,
                                vault.amount_cred,
                            );
                            self.state.fill_log.push(FillRecord {
                                turn,
                                kind: FillKind::EscrowFinished,
                                good: Some(AssetId::Cred),
                                from: Some(vault.sender),
                                to: vault.destination,
                                units: vault.amount_cred,
                                net_units: vault.amount_cred,
                                cred_amount: vault.amount_cred,
                                unit_price_cred: None,
                            });
                            finished.push(escrow_id);
                        } else if bond.deadline_turn <= turn {
                            if let Some(slot) = self.state.escrows.get_mut(&escrow_id) {
                                slot.status = EscrowStatus::Cancelled;
                            }
                            let _ = self.adjust_free_balance(
                                vault.sender,
                                AssetId::Cred,
                                vault.amount_cred,
                            );
                            let ob = &mut self.state.seats[usize::from(bond.obligor)];
                            ob.bond_breaches += 1;
                            ob.honor_penalties =
                                ob.honor_penalties.saturating_add(vault.amount_cred);
                            self.state.fill_log.push(FillRecord {
                                turn,
                                kind: FillKind::EscrowBreachRefunded,
                                good: Some(AssetId::Cred),
                                from: Some(vault.sender),
                                to: vault.sender,
                                units: vault.amount_cred,
                                net_units: vault.amount_cred,
                                cred_amount: vault.amount_cred,
                                unit_price_cred: None,
                            });
                            cancelled.push(escrow_id);
                        }
                        continue;
                    }
                }
            }
            if vault.cancel_after_turn <= turn
                && self.refund_expired_escrow(escrow_id, turn, None).is_ok()
            {
                cancelled.push(escrow_id);
                let expired_pledges: Vec<IntentPledge> = self
                    .state
                    .pledges
                    .values()
                    .filter(|p| {
                        !self.state.audited_pledges.contains(&p.pledge_id)
                            && matches!(
                                p.target,
                                IntentTarget::RevealPreimage {
                                    escrow_id: eid,
                                    ..
                                } if eid == escrow_id
                            )
                    })
                    .cloned()
                    .collect();
                for pledge in expired_pledges {
                    self.record_broken_promise(&pledge, broken_promises);
                }
            }
        }

        // Evaluate open co-signed DualEscrow vaults against ledger deliveries and deadlines.
        let dual_ids: Vec<u64> = self.state.dual_escrows.keys().copied().collect();
        for eid in dual_ids {
            let Some(dual) = self.state.dual_escrows.get(&eid).cloned() else {
                continue;
            };
            if dual.status != EscrowStatus::Locked {
                continue;
            }
            let cum_now = self
                .state
                .delivery_ledger
                .get(&(dual.seller, dual.buyer, dual.good))
                .copied()
                .unwrap_or(0);
            let delta = cum_now.saturating_sub(dual.baseline_delivered);
            if delta >= dual.required_units && turn <= dual.deadline_turn {
                let buyer_top_up = dual.payment_cred.saturating_sub(dual.buyer_bond_cred);
                if self.free_balance(dual.buyer, AssetId::Cred) >= buyer_top_up {
                    let _ = self.adjust_free_balance(dual.buyer, AssetId::Cred, -buyer_top_up);
                    let _ = self.adjust_free_balance(
                        dual.seller,
                        AssetId::Cred,
                        dual.payment_cred.saturating_add(dual.seller_bond_cred),
                    );
                    if let Some(slot) = self.state.dual_escrows.get_mut(&eid) {
                        slot.status = EscrowStatus::Finished;
                    }
                    self.state.fill_log.push(FillRecord {
                        turn,
                        kind: FillKind::EscrowFinished,
                        good: Some(AssetId::Cred),
                        from: Some(dual.buyer),
                        to: dual.seller,
                        units: dual.payment_cred,
                        net_units: dual.payment_cred,
                        cred_amount: dual.payment_cred,
                        unit_price_cred: None,
                    });
                    finished.push(eid);
                } else if dual.deadline_turn <= turn {
                    self.settle_dual_escrow_default(
                        turn,
                        eid,
                        dual.buyer,
                        dual.seller,
                        dual.buyer_bond_cred,
                        dual.seller_bond_cred,
                    );
                    cancelled.push(eid);
                }
            } else if dual.deadline_turn <= turn {
                self.settle_dual_escrow_default(
                    turn,
                    eid,
                    dual.seller,
                    dual.buyer,
                    dual.seller_bond_cred,
                    dual.buyer_bond_cred,
                );
                cancelled.push(eid);
            }
        }
        (finished, cancelled)
    }

    fn step2_settle_atomic_swaps(
        &mut self,
        turn: u8,
        bundles: &[SealedOrderBundle],
    ) -> Result<(usize, Vec<AtomicSwapProposal>), EngineError> {
        let n = self.state.seats.len() as u8;
        let mut maker_map: BTreeMap<AtomicSwapProposal, BTreeSet<u8>> = BTreeMap::new();
        for (seat_idx, bundle) in bundles.iter().enumerate() {
            let s = seat_idx as u8;
            for prop in &bundle.atomic_swaps {
                if prop.maker < n
                    && prop.taker < n
                    && prop.maker != prop.taker
                    && self.ensure_valid_asset(prop.maker_asset).is_ok()
                    && self.ensure_valid_asset(prop.taker_asset).is_ok()
                    && prop.maker_amount > 0
                    && prop.taker_amount > 0
                    && (s == prop.maker || s == prop.taker)
                {
                    maker_map.entry(prop.clone()).or_default().insert(s);
                }
            }
        }

        let mut settled = 0usize;
        let mut settled_swaps = Vec::new();
        for (swap, signers) in maker_map {
            if !signers.contains(&swap.maker) || !signers.contains(&swap.taker) {
                continue;
            }
            if self.state.seats[usize::from(swap.maker)].wash_frozen_until_turn >= turn
                || self.state.seats[usize::from(swap.taker)].wash_frozen_until_turn >= turn
            {
                continue;
            }
            if let Some(dom_id) = swap.domain_id {
                let valid_dom = self
                    .state
                    .domains
                    .iter()
                    .find(|d| d.domain_id == dom_id)
                    .is_some_and(|d| {
                        d.members.contains(&swap.maker)
                            && d.members.contains(&swap.taker)
                            && self.state.seats[usize::from(swap.maker)]
                                .credit_tier
                                .may_access_prime_cartel()
                            && self.state.seats[usize::from(swap.taker)]
                                .credit_tier
                                .may_access_prime_cartel()
                    });
                if !valid_dom {
                    continue;
                }
            }
            if self.free_balance(swap.maker, swap.maker_asset) < swap.maker_amount
                || self.free_balance(swap.taker, swap.taker_asset) < swap.taker_amount
            {
                continue;
            }

            let m_tariff_bps = self.effective_asset_tariff_bps(
                swap.maker_asset,
                swap.domain_id,
                swap.maker,
                swap.taker,
            );
            let t_tariff_bps = self.effective_asset_tariff_bps(
                swap.taker_asset,
                swap.domain_id,
                swap.taker,
                swap.maker,
            );
            let m_net = swap.maker_amount - ceil_bps_fee(swap.maker_amount, m_tariff_bps);
            let t_net = swap.taker_amount - ceil_bps_fee(swap.taker_amount, t_tariff_bps);

            if self
                .free_balance(swap.taker, swap.maker_asset)
                .checked_add(m_net)
                .is_none()
                || self
                    .free_balance(swap.maker, swap.taker_asset)
                    .checked_add(t_net)
                    .is_none()
            {
                return Err(EngineError::InvalidOrder("balance overflow".to_owned()));
            }
            let _ = self.adjust_free_balance(swap.maker, swap.maker_asset, -swap.maker_amount);
            let _ = self.adjust_free_balance(swap.taker, swap.taker_asset, -swap.taker_amount);
            let _ = self.adjust_free_balance(swap.taker, swap.maker_asset, m_net);
            let _ = self.adjust_free_balance(swap.maker, swap.taker_asset, t_net);
            self.record_delivery(swap.maker, swap.taker, swap.maker_asset, m_net);
            self.record_delivery(swap.taker, swap.maker, swap.taker_asset, t_net);
            if swap.maker_asset != AssetId::Cred && swap.taker_asset == AssetId::Cred {
                self.state.fill_log.push(FillRecord {
                    turn,
                    kind: FillKind::AtomicSwap,
                    good: Some(swap.maker_asset),
                    from: Some(swap.maker),
                    to: swap.taker,
                    units: swap.maker_amount,
                    net_units: m_net,
                    cred_amount: swap.taker_amount,
                    unit_price_cred: Self::implied_unit_price_cred(
                        swap.taker_amount,
                        swap.maker_amount,
                    ),
                });
            } else if swap.taker_asset != AssetId::Cred && swap.maker_asset == AssetId::Cred {
                self.state.fill_log.push(FillRecord {
                    turn,
                    kind: FillKind::AtomicSwap,
                    good: Some(swap.taker_asset),
                    from: Some(swap.taker),
                    to: swap.maker,
                    units: swap.taker_amount,
                    net_units: t_net,
                    cred_amount: swap.maker_amount,
                    unit_price_cred: Self::implied_unit_price_cred(
                        swap.maker_amount,
                        swap.taker_amount,
                    ),
                });
            } else {
                self.state.fill_log.push(FillRecord {
                    turn,
                    kind: FillKind::AtomicSwap,
                    good: Some(swap.maker_asset),
                    from: Some(swap.maker),
                    to: swap.taker,
                    units: swap.maker_amount,
                    net_units: m_net,
                    cred_amount: 0,
                    unit_price_cred: None,
                });
                self.state.fill_log.push(FillRecord {
                    turn,
                    kind: FillKind::AtomicSwap,
                    good: Some(swap.taker_asset),
                    from: Some(swap.taker),
                    to: swap.maker,
                    units: swap.taker_amount,
                    net_units: t_net,
                    cred_amount: 0,
                    unit_price_cred: None,
                });
            }
            settled += 1;
            settled_swaps.push(swap);
        }
        Ok((settled, settled_swaps))
    }

    fn step2_voluntary_debt_repayment(
        &mut self,
        bundles: &[SealedOrderBundle],
    ) -> (Units, Vec<Units>) {
        let mut total_repaid: Units = 0;
        let mut repaid_by_seat = vec![0; bundles.len()];
        for (s_idx, b) in bundles.iter().enumerate() {
            if b.debt_repayment_cred <= 0 {
                continue;
            }
            let seat = &mut self.state.seats[s_idx];
            let repay = b.debt_repayment_cred.min(seat.cred).min(seat.cred_debt);
            if repay > 0 {
                seat.cred -= repay;
                seat.cred_debt -= repay;
                repaid_by_seat[s_idx] = repay;
                total_repaid = total_repaid.saturating_add(repay);
            }
        }
        (total_repaid, repaid_by_seat)
    }

    fn execute_seat_converter_runs(
        &mut self,
        s_idx: usize,
        orders: &[ConverterRunOrder],
    ) -> (usize, usize, Vec<ConverterOrderPreview>) {
        let mut seat_batches = 0usize;
        let mut seat_clamped = 0usize;
        let mut runs_used: BTreeMap<usize, u32> = BTreeMap::new();
        let mut previews = Vec::with_capacity(orders.len());
        for order in orders {
            if order.runs == 0 {
                previews.push(ConverterOrderPreview {
                    recipe_idx: order.recipe_idx,
                    requested_runs: order.runs,
                    executed_runs: 0,
                    clamped: false,
                });
                continue;
            }
            let Some(recipe) = self.state.spec.seats[s_idx]
                .converters
                .get(order.recipe_idx)
                .cloned()
            else {
                previews.push(ConverterOrderPreview {
                    recipe_idx: order.recipe_idx,
                    requested_runs: order.runs,
                    executed_runs: 0,
                    clamped: false,
                });
                continue;
            };
            let used = runs_used.get(&order.recipe_idx).copied().unwrap_or(0);
            let remaining_cap = recipe.max_runs_per_turn.saturating_sub(used);
            let affordable_by_inputs =
                batches_covered_by_inputs(&recipe, &self.state.seats[s_idx].raw);
            let exec_runs = order.runs.min(remaining_cap).min(affordable_by_inputs);
            let clamped = exec_runs > 0 && exec_runs < order.runs;
            previews.push(ConverterOrderPreview {
                recipe_idx: order.recipe_idx,
                requested_runs: order.runs,
                executed_runs: exec_runs,
                clamped,
            });
            if exec_runs == 0 {
                continue;
            }
            if clamped {
                seat_clamped += 1;
            }
            let runs_i64 = i64::from(exec_runs);
            for &(raw_g, amt) in &recipe.inputs {
                self.state.seats[s_idx].raw[usize::from(raw_g)] -= amt.saturating_mul(runs_i64);
            }
            self.state.seats[s_idx].refined[usize::from(recipe.output_good)] =
                self.state.seats[s_idx].refined[usize::from(recipe.output_good)]
                    .saturating_add(recipe.output_amount.saturating_mul(runs_i64));
            *runs_used.entry(order.recipe_idx).or_insert(0) += exec_runs;
            seat_batches += exec_runs as usize;
        }
        (seat_batches, seat_clamped, previews)
    }

    fn step2_settle_converters(
        &mut self,
        bundles: &[SealedOrderBundle],
    ) -> (usize, usize, Vec<Vec<ConverterOrderPreview>>, Vec<usize>) {
        let mut total_batches = 0usize;
        let mut clamped_orders = 0usize;
        let mut by_seat = Vec::with_capacity(bundles.len());
        let mut seat_converter_batches = Vec::with_capacity(bundles.len());
        for (s_idx, bundle) in bundles.iter().enumerate() {
            let (seat_batches, seat_clamped, previews) =
                self.execute_seat_converter_runs(s_idx, &bundle.converter_runs);
            clamped_orders += seat_clamped;
            total_batches += seat_batches;
            seat_converter_batches.push(seat_batches);
            by_seat.push(previews);
        }
        (
            total_batches,
            clamped_orders,
            by_seat,
            seat_converter_batches,
        )
    }

    fn step3_clear_checks(
        &mut self,
        turn: u8,
        bundles: &[SealedOrderBundle],
    ) -> (Vec<u64>, Vec<u64>, Vec<u8>) {
        let mut check_ids = BTreeSet::new();
        for (s_idx, b) in bundles.iter().enumerate() {
            let presenter = s_idx as u8;
            for &cid in &b.check_cashes {
                if self
                    .state
                    .checks
                    .get(&cid)
                    .is_some_and(|c| c.payee == presenter)
                {
                    check_ids.insert(cid);
                }
            }
        }

        let mut cashed = Vec::new();
        let mut bounced = Vec::new();
        let mut evictions = Vec::new();

        for check_id in check_ids {
            let Some(check) = self.state.checks.get(&check_id).cloned() else {
                continue;
            };
            if check.status != CheckStatus::Open
                || check.due_turn > turn
                || turn >= check.expiration_turn
            {
                continue;
            }
            let avail = self.free_balance(check.drawer, check.asset);
            if avail >= check.deliver_min {
                let gross = avail.min(check.send_max);
                let tariff_bps =
                    self.effective_asset_tariff_bps(check.asset, None, check.drawer, check.payee);
                let net = gross - ceil_bps_fee(gross, tariff_bps);
                let _ = self.adjust_free_balance(check.drawer, check.asset, -gross);
                let _ = self.adjust_free_balance(check.payee, check.asset, net);
                self.record_delivery(check.drawer, check.payee, check.asset, net);
                if let Some(slot) = self.state.checks.get_mut(&check_id) {
                    slot.status = CheckStatus::Cashed(net);
                }
                self.state.fill_log.push(FillRecord {
                    turn,
                    kind: FillKind::CheckCashed,
                    good: Some(check.asset),
                    from: Some(check.drawer),
                    to: check.payee,
                    units: gross,
                    net_units: net,
                    cred_amount: if check.asset == AssetId::Cred { net } else { 0 },
                    unit_price_cred: None,
                });
                cashed.push(check_id);
            } else {
                if let Some(slot) = self.state.checks.get_mut(&check_id) {
                    slot.status = CheckStatus::Bounced;
                }
                bounced.push(check_id);
                let drawer_state = &mut self.state.seats[usize::from(check.drawer)];
                drawer_state.hard_credit_defaults += 1;
                drawer_state.credit_tier = drawer_state.credit_tier.downgrade_one();
                if !drawer_state.credit_tier.may_access_prime_cartel() {
                    let evicted = self.evict_seat_from_all_cartels(check.drawer);
                    if evicted {
                        evictions.push(check.drawer);
                    }
                }
            }
        }

        for check in self.state.checks.values_mut() {
            if check.status == CheckStatus::Open && check.expiration_turn <= turn {
                check.status = CheckStatus::Cancelled;
            }
        }
        (cashed, bounced, evictions)
    }

    fn step3_settle_wash_freezes(&mut self, turn: u8) -> Vec<WashFreezeSettlement> {
        let mut remaining = Vec::new();
        let mut due = Vec::new();
        for wf in std::mem::take(&mut self.state.pending_wash_freezes) {
            if wf.turn == turn {
                due.push(wf);
            } else {
                remaining.push(wf);
            }
        }
        self.state.pending_wash_freezes = remaining;
        if due.is_empty() {
            return Vec::new();
        }

        let prev_turn_fills: Vec<FillRecord> = if turn >= 2 {
            self.state
                .fill_log
                .iter()
                .filter(|r| r.turn == turn - 1)
                .cloned()
                .collect()
        } else {
            Vec::new()
        };

        let mut settlements = Vec::with_capacity(due.len());
        for wf in due {
            let confirmed = audit_wash_freeze(
                &prev_turn_fills,
                wf.target_seat,
                &self.state.parities,
                self.state.spec.town_import_premium_bps,
                self.state.spec.wash_round_trip_min_units,
            );
            if confirmed {
                let _ = self.adjust_free_balance(wf.inspector, AssetId::Cred, wf.bond_cred);
                let target_free = self.free_balance(wf.target_seat, AssetId::Cred).max(0);
                let bounty = self.state.spec.wash_freeze_bounty_cred.min(target_free);
                if bounty > 0 {
                    let _ = self.adjust_free_balance(wf.target_seat, AssetId::Cred, -bounty);
                    let _ = self.adjust_free_balance(wf.inspector, AssetId::Cred, bounty);
                }
                settlements.push(WashFreezeSettlement {
                    turn,
                    inspector: wf.inspector,
                    target_seat: wf.target_seat,
                    confirmed: true,
                    bond_returned_cred: wf.bond_cred,
                    bounty_paid_cred: bounty,
                    damages_paid_cred: 0,
                });
            } else {
                // Clean audit (§5.2 amended 2026-11-17): the whole held bond goes to the frozen
                // seat as damages; nothing returns to the Inspector.
                let damages = wf.bond_cred;
                if damages > 0 {
                    let _ = self.adjust_free_balance(wf.target_seat, AssetId::Cred, damages);
                }
                settlements.push(WashFreezeSettlement {
                    turn,
                    inspector: wf.inspector,
                    target_seat: wf.target_seat,
                    confirmed: false,
                    bond_returned_cred: 0,
                    bounty_paid_cred: 0,
                    damages_paid_cred: damages,
                });
            }
        }
        settlements
    }

    fn evict_seat_from_all_cartels(&mut self, seat: u8) -> bool {
        let mut any = false;
        for dom in &mut self.state.domains {
            if dom.members.remove(&seat) {
                any = true;
            }
        }
        self.cancel_domain_offers_for_seat(seat, None);
        any
    }

    fn cancel_domain_offers_for_seat(&mut self, seat: u8, domain_filter: Option<u8>) {
        let cancel_ids: Vec<u64> = self
            .state
            .offers
            .values()
            .filter(|o| {
                o.maker == seat
                    && match (domain_filter, o.domain_id) {
                        (Some(df), Some(od)) => df == od,
                        (None, Some(_)) => true,
                        (_, None) => false,
                    }
            })
            .map(|o| o.offer_id)
            .collect();
        for oid in cancel_ids {
            let _ = self.handle_cancel_offer(seat, oid);
        }
    }

    fn step4a_project_deliveries(
        &mut self,
        turn: u8,
        bundles: &[SealedOrderBundle],
    ) -> (
        usize,
        usize,
        Units,
        Vec<Vec<ProjectOrderPreview>>,
        Vec<usize>,
    ) {
        let mut completed = 0usize;
        let mut clamped_orders = 0usize;
        let mut works_stipend_cred_minted: Units = 0;
        let project_by_id: BTreeMap<u16, crate::types::ProjectRecipe> = self
            .state
            .spec
            .projects
            .iter()
            .map(|p| (p.id, p.clone()))
            .collect();
        let mut by_seat = Vec::with_capacity(bundles.len());
        let mut seat_works_batches = Vec::with_capacity(bundles.len());

        for (s_idx, bundle) in bundles.iter().enumerate() {
            let mut previews = Vec::with_capacity(bundle.project_deliveries.len());
            let mut seat_completed = 0usize;
            for proj_ord in &bundle.project_deliveries {
                if proj_ord.batches == 0 {
                    previews.push(ProjectOrderPreview {
                        project_id: proj_ord.project_id,
                        requested_batches: proj_ord.batches,
                        executed_batches: 0,
                        clamped: false,
                    });
                    continue;
                }
                let Some(recipe) = project_by_id.get(&proj_ord.project_id) else {
                    previews.push(ProjectOrderPreview {
                        project_id: proj_ord.project_id,
                        requested_batches: proj_ord.batches,
                        executed_batches: 0,
                        clamped: false,
                    });
                    continue;
                };
                let affordable_batches = recipe
                    .inputs
                    .iter()
                    .map(|&(ref_g, amt)| {
                        if amt <= 0 {
                            0
                        } else {
                            let avail = self.state.seats[s_idx].refined[usize::from(ref_g)].max(0);
                            u32::try_from(avail / amt).unwrap_or(u32::MAX)
                        }
                    })
                    .min()
                    .unwrap_or(0);
                let exec_batches = proj_ord.batches.min(affordable_batches);
                let clamped = exec_batches > 0 && exec_batches < proj_ord.batches;
                previews.push(ProjectOrderPreview {
                    project_id: proj_ord.project_id,
                    requested_batches: proj_ord.batches,
                    executed_batches: exec_batches,
                    clamped,
                });
                if exec_batches == 0 {
                    continue;
                }
                if clamped {
                    clamped_orders += 1;
                }
                let b_i64 = i64::from(exec_batches);
                let mut delivered_units: Units = 0;
                for &(ref_g, amt) in &recipe.inputs {
                    let qty = amt.saturating_mul(b_i64);
                    self.state.seats[s_idx].refined[usize::from(ref_g)] -= qty;
                    delivered_units = delivered_units.saturating_add(qty);
                }
                let stipend =
                    mul_units_parity(delivered_units, self.state.spec.works_stipend_per_unit_cred);
                if stipend > 0 {
                    self.state.seats[s_idx].cred =
                        self.state.seats[s_idx].cred.saturating_add(stipend);
                    works_stipend_cred_minted = works_stipend_cred_minted.saturating_add(stipend);
                }
                let surge_bps = self
                    .state
                    .active_shocks
                    .iter()
                    .filter(|sh| {
                        sh.kind == ShockKind::DemandSurge
                            && sh.expires_after_turn >= turn
                            && recipe.inputs.iter().any(|&(g, _)| g == sh.target_seat)
                    })
                    .map(|sh| sh.magnitude_bps)
                    .max()
                    .unwrap_or(0);
                let base_pts = recipe.equity_points.saturating_mul(b_i64);
                let bonus = half_up_bps(base_pts, surge_bps);
                self.state.seats[s_idx].project_equity = self.state.seats[s_idx]
                    .project_equity
                    .saturating_add(base_pts.saturating_add(bonus));
                completed += exec_batches as usize;
                seat_completed += exec_batches as usize;
            }
            seat_works_batches.push(seat_completed);
            by_seat.push(previews);
        }
        (
            completed,
            clamped_orders,
            works_stipend_cred_minted,
            by_seat,
            seat_works_batches,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn evaluate_c1a_idle_takeover(
        &mut self,
        turn: u8,
        committed_non_revealers: &[bool],
        seat_converter_batches: &[usize],
        prev_outbound_delivered: &[i128],
        batch_filled_seats: &[bool],
        seat_works_batches: &[usize],
        converter_batches_run: &mut usize,
        converter_orders_clamped: &mut usize,
        converter_orders_by_seat: &mut [Vec<ConverterOrderPreview>],
        checks_cashed: &mut Vec<u64>,
        checks_bounced: &mut Vec<u64>,
        cartel_evictions: &mut Vec<u8>,
        regent_acted_seats: &mut Vec<u8>,
        converted_to_ai: &mut Vec<u8>,
    ) {
        // Turn 1 starts with CRED only (`types.rs:495`) before raw yields are minted at Step 4b,
        // so seats cannot yet hold foreign converter inputs (`A-2`).
        if turn < 2 {
            return;
        }
        let post_outbound_delivered = self.outbound_delivered_by_seat();
        let n = self.state.seats.len();
        let mut regent_check_bundles = vec![SealedOrderBundle::default(); n];
        let mut any_regent_checks = false;

        for s_idx in 0..n {
            let made_delivery = post_outbound_delivered[s_idx] > prev_outbound_delivered[s_idx];
            let had_batch_fill = batch_filled_seats[s_idx];
            let had_works_delivery = seat_works_batches[s_idx] > 0;
            let ran_converter = seat_converter_batches[s_idx] > 0;

            if ran_converter || made_delivery || had_batch_fill || had_works_delivery {
                self.state.seats[s_idx].consecutive_zero_production_turns = 0;
                continue;
            }

            self.state.seats[s_idx].consecutive_zero_production_turns = self.state.seats[s_idx]
                .consecutive_zero_production_turns
                .saturating_add(1);
            let s_u8 = s_idx as u8;
            if self.state.seats[s_idx].consecutive_zero_production_turns >= IDLE_MISS_TAKEOVER_TURNS
                && !committed_non_revealers[s_idx]
            {
                self.state.seats[s_idx].consecutive_misses = self.state.seats[s_idx]
                    .consecutive_misses
                    .max(IDLE_MISS_TAKEOVER_TURNS);
                if let Some(doctrine) = self.state.spec.seats[s_idx].regent {
                    let was_human = self.state.seats[s_idx].identity == SeatIdentity::Human;
                    if was_human {
                        self.state.seats[s_idx].identity = SeatIdentity::AiSyndicate;
                        if !converted_to_ai.contains(&s_u8) {
                            converted_to_ai.push(s_u8);
                        }
                    }
                    if !regent_acted_seats.contains(&s_u8) {
                        let regent_bundle = self.synthesize_regent_bundle(turn, s_u8, &doctrine);
                        if !regent_bundle.check_cashes.is_empty() {
                            regent_check_bundles[s_idx].check_cashes = regent_bundle.check_cashes;
                            any_regent_checks = true;
                        }
                        let (r_batches, r_clamped, r_previews) =
                            self.execute_seat_converter_runs(s_idx, &regent_bundle.converter_runs);
                        *converter_batches_run += r_batches;
                        *converter_orders_clamped += r_clamped;
                        converter_orders_by_seat[s_idx].extend(r_previews);
                        if was_human || r_batches > 0 {
                            regent_acted_seats.push(s_u8);
                        }
                        if r_batches > 0 {
                            self.state.seats[s_idx].consecutive_zero_production_turns = 0;
                        }
                    }
                }
            }
        }
        if any_regent_checks {
            let (r_cashed, r_bounced, r_evicted) =
                self.step3_clear_checks(turn, &regent_check_bundles);
            checks_cashed.extend(r_cashed);
            checks_bounced.extend(r_bounced);
            for ev in r_evicted {
                if !cartel_evictions.contains(&ev) {
                    cartel_evictions.push(ev);
                }
            }
        }
        regent_acted_seats.sort_unstable();
        converted_to_ai.sort_unstable();
    }

    fn step4b_mint_raw_yields(&mut self, turn: u8) {
        for s_idx in 0..self.state.seats.len() {
            let actual_yield = raw_yield_at_sweep(&self.state, s_idx, turn);
            self.state.seats[s_idx].raw[s_idx] =
                self.state.seats[s_idx].raw[s_idx].saturating_add(actual_yield);
        }
    }

    fn step5_collect_upkeep_and_debt(
        &mut self,
        repaid_by_seat: &[Units],
    ) -> (Units, Vec<SeatUpkeepPreview>) {
        let mut total_shortfall_debt: Units = 0;
        let mut upkeep_by_seat = Vec::with_capacity(self.state.seats.len());
        for s_idx in 0..self.state.seats.len() {
            let rate = self.state.seats[s_idx].credit_tier.interest_bps();
            let interest = half_up_bps(self.state.seats[s_idx].cred_debt, rate);
            self.state.seats[s_idx].cred_debt =
                self.state.seats[s_idx].cred_debt.saturating_add(interest);

            let cfg = &self.state.spec.seats[s_idx];
            let upkeep_g = usize::from(cfg.upkeep_good);
            let avail = self.state.seats[s_idx].refined[upkeep_g];
            let parities = &self.state.parities;
            let settlement = settle_upkeep(avail, cfg.upkeep_amount, || {
                parities.refined_per_unit[upkeep_g]
            });
            let paid = settlement.paid_with_refined;
            self.state.seats[s_idx].refined[upkeep_g] -= paid;
            let new_debt = settlement.new_shortfall_debt;
            if settlement.shortfall > 0 {
                self.state.seats[s_idx].cred_debt =
                    self.state.seats[s_idx].cred_debt.saturating_add(new_debt);
                total_shortfall_debt = total_shortfall_debt.saturating_add(new_debt);
            }
            upkeep_by_seat.push(SeatUpkeepPreview {
                upkeep_good: cfg.upkeep_good,
                upkeep_required: cfg.upkeep_amount,
                paid_with_refined: paid,
                debt_interest_accrued: interest,
                new_shortfall_debt: new_debt,
                debt_repaid_cred: repaid_by_seat.get(s_idx).copied().unwrap_or(0),
            });
        }
        (total_shortfall_debt, upkeep_by_seat)
    }

    fn step6_apply_perishable_spoilage(&mut self, turn: u8) -> (Units, Vec<Units>) {
        let n = self.state.seats.len();
        let rate_by_good: Vec<Bps> = (0..n)
            .map(|g| spoilage_bps_at_sweep(&self.state, g, turn))
            .collect();

        let mut total_clawed: Units = 0;
        let mut by_seat: Vec<Units> = vec![0; n];
        for (seat_idx, seat_state) in self.state.seats.iter_mut().take(n).enumerate() {
            for (g, &eff_rate) in rate_by_good.iter().enumerate() {
                let free = seat_state.raw[g];
                let decay = floor_bps_part(free, eff_rate);
                seat_state.raw[g] -= decay;
                total_clawed = total_clawed.saturating_add(decay);
                by_seat[seat_idx] = by_seat[seat_idx].saturating_add(decay);
            }
        }
        // Raw sell offers stay in custody of the issuer's perishable trust line; shrink them
        // proportionally so resting open Raw sell orders cannot shield inventory from Clawback.
        let mut to_remove = Vec::new();
        for (&oid, offer) in &mut self.state.offers {
            if offer.side == OfferSide::Sell
                && let AssetId::Raw(g) = offer.good
            {
                let eff_rate = rate_by_good[usize::from(g)];
                let decay = floor_bps_part(offer.remaining_units, eff_rate);
                offer.remaining_units -= decay;
                total_clawed = total_clawed.saturating_add(decay);
                if let Some(slot) = by_seat.get_mut(usize::from(offer.maker)) {
                    *slot = slot.saturating_add(decay);
                }
                if offer.remaining_units <= 0 {
                    to_remove.push(oid);
                }
            }
        }
        for oid in to_remove {
            self.state.offers.remove(&oid);
        }
        (total_clawed, by_seat)
    }

    fn step7_politics_and_charters(
        &mut self,
        turn: u8,
        turn_beacon: [u8; 32],
        bundles: &[SealedOrderBundle],
        broken_promises: &mut Vec<BrokenPromiseRecord>,
    ) -> PoliticsOutcome {
        let mut elected = None;
        if turn.is_multiple_of(4) {
            let n = self.state.seats.len();
            let mut tm_votes = vec![0u32; n];
            let mut cc_votes = vec![0u32; n];
            let mut valid_ballots: Vec<(u8, &CouncilVoteBallot)> = Vec::new();
            for (s_idx, b) in bundles.iter().enumerate() {
                if let Some(ballot) = &b.council_vote
                    && usize::from(ballot.trade_minister) < n
                    && usize::from(ballot.cartel_chair) < n
                {
                    tm_votes[usize::from(ballot.trade_minister)] += 1;
                    cc_votes[usize::from(ballot.cartel_chair)] += 1;
                    valid_ballots.push((s_idx as u8, ballot));
                }
            }

            // Audit VoteForSeat pledges against cast ballots.
            let vote_pledges: Vec<IntentPledge> = self
                .state
                .pledges
                .values()
                .filter(|p| {
                    !self.state.audited_pledges.contains(&p.pledge_id)
                        && matches!(
                            p.target,
                            IntentTarget::VoteForSeat { office_turn, .. } if office_turn == turn
                        )
                })
                .cloned()
                .collect();
            for pledge in vote_pledges {
                if let IntentTarget::VoteForSeat { candidate, .. } = pledge.target {
                    let honored = valid_ballots.iter().any(|&(voter, b)| {
                        voter == pledge.promisor
                            && (b.trade_minister == candidate || b.cartel_chair == candidate)
                    });
                    if honored {
                        self.state.audited_pledges.insert(pledge.pledge_id);
                    } else {
                        self.record_broken_promise(&pledge, broken_promises);
                    }
                }
            }

            // One elected office per seat (2026-11-17): decide Trade Minister first and exclude the
            // winner from the Cartel Chair tally.
            let (tm, tm_won_vote) = pick_plurality_winner_with_beacon(
                &tm_votes,
                self.state.trade_minister,
                turn_beacon,
                self.state.table_id,
                Turn(turn),
                b"trade_minister",
            );
            cc_votes[usize::from(tm)] = 0;
            let max_cc = cc_votes.iter().copied().max().unwrap_or(0);
            let (cc, cc_won_vote) = if max_cc > 0 {
                let tied: Vec<u8> = cc_votes
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, &v)| (v == max_cc).then_some(idx as u8))
                    .collect();
                let winner = break_tie_with_turn_beacon(
                    turn_beacon,
                    self.state.table_id,
                    Turn(turn),
                    b"cartel_chair",
                    &tied,
                );
                (Some(winner), true)
            } else if self.state.cartel_chair == Some(tm) {
                // Incumbent Cartel Chair became Trade Minister with no eligible chair votes -> vacant.
                (None, false)
            } else {
                // M1 zero-vote rule retains incumbent (or keeps None if already vacant).
                (self.state.cartel_chair, false)
            };
            self.state.trade_minister = tm;
            self.state.cartel_chair = cc;

            // T3 & M1: Only seats elected with >= 1 vote enact their publicly filed StandForOffice platforms.
            if tm_won_vote
                && let Some(win_tm_platform) = self.state.council_platforms.get(&tm).cloned()
            {
                for &(target, (raw_bps, ref_bps)) in &win_tm_platform.proposed_tariffs_bps {
                    if usize::from(target) < n {
                        self.state.seats[usize::from(target)].raw_tariff_bps =
                            raw_bps.min(MAX_TARIFF_BPS);
                        self.state.seats[usize::from(target)].refined_tariff_bps =
                            ref_bps.min(MAX_TARIFF_BPS);
                    }
                }
            }

            let mut revoked_pairs = BTreeSet::new();
            if cc_won_vote
                && let Some(cc_seat) = cc
                && let Some(win_cc_platform) = self.state.council_platforms.get(&cc_seat).cloned()
            {
                for &(dom_id, seat_idx) in &win_cc_platform.cartel_admissions {
                    if usize::from(seat_idx) < n
                        && self.state.seats[usize::from(seat_idx)]
                            .credit_tier
                            .may_access_prime_cartel()
                    {
                        if let Some(dom) = self
                            .state
                            .domains
                            .iter_mut()
                            .find(|d| d.domain_id == dom_id)
                        {
                            dom.members.insert(seat_idx);
                        } else if self.state.domains.len() < MAX_CARTEL_DOMAINS {
                            let mut members = BTreeSet::new();
                            members.insert(seat_idx);
                            self.state.domains.push(CartelDomain {
                                domain_id: dom_id,
                                members,
                            });
                            self.state.domains.sort_by_key(|d| d.domain_id);
                        }
                    }
                }
                for &(dom_id, seat_idx) in &win_cc_platform.cartel_revocations {
                    if usize::from(seat_idx) < n
                        && let Some(dom) = self
                            .state
                            .domains
                            .iter_mut()
                            .find(|d| d.domain_id == dom_id)
                        && dom.members.remove(&seat_idx)
                    {
                        self.cancel_domain_offers_for_seat(seat_idx, Some(dom_id));
                        revoked_pairs.insert((dom_id, seat_idx));
                    }
                }
            }
            self.state.council_platforms.clear();

            let cartel_pledges: Vec<IntentPledge> = self
                .state
                .pledges
                .values()
                .filter(|p| {
                    !self.state.audited_pledges.contains(&p.pledge_id)
                        && matches!(
                            p.target,
                            IntentTarget::PreserveCartelMembership { office_turn, .. }
                                if office_turn == turn
                        )
                })
                .cloned()
                .collect();
            for pledge in cartel_pledges {
                if let IntentTarget::PreserveCartelMembership {
                    domain_id,
                    beneficiary,
                    ..
                } = pledge.target
                {
                    if Some(pledge.promisor) == cc
                        && revoked_pairs.contains(&(domain_id, beneficiary))
                    {
                        self.record_broken_promise(&pledge, broken_promises);
                    } else {
                        self.state.audited_pledges.insert(pledge.pledge_id);
                    }
                }
            }
            elected = (tm_won_vote || cc_won_vote).then_some((tm, cc));
        }

        let mut awarded = Vec::new();
        let mut charter_cred_burned: Units = 0;
        if turn.is_multiple_of(6) {
            self.state.charters.retain(|c| c.expires_at_turn > turn);
            let roles = [
                CharterRole::ClearinghouseBroker,
                CharterRole::CustomsInspector,
                CharterRole::OracleAuditor,
            ];
            for role in roles {
                let mut best: Option<(Units, u8)> = None;
                for (s_idx, b) in bundles.iter().enumerate() {
                    let s = s_idx as u8;
                    for bid in &b.charter_bids {
                        if bid.role == role
                            && bid.bid_cred > 0
                            && bid.bid_cred <= self.state.seats[s_idx].cred
                        {
                            let replace = match best {
                                None => true,
                                Some((best_bid, best_s)) => {
                                    bid.bid_cred > best_bid
                                        || (bid.bid_cred == best_bid && s < best_s)
                                }
                            };
                            if replace {
                                best = Some((bid.bid_cred, s));
                            }
                        }
                    }
                }
                if let Some((win_bid, winner)) = best {
                    self.state.seats[usize::from(winner)].cred -= win_bid;
                    charter_cred_burned = charter_cred_burned.saturating_add(win_bid);
                    self.state.charters.retain(|c| c.role != role);
                    self.state.charters.push(ActiveCharter {
                        role,
                        holder: winner,
                        expires_at_turn: turn + 6,
                    });
                    awarded.push((role, winner));
                }
            }
        }
        (elected, awarded, charter_cred_burned)
    }

    fn record_broken_promise(
        &mut self,
        pledge: &IntentPledge,
        broken_promises: &mut Vec<BrokenPromiseRecord>,
    ) {
        self.state.audited_pledges.insert(pledge.pledge_id);
        let p = &mut self.state.seats[usize::from(pledge.promisor)];
        p.broken_promises += 1;
        p.honor_penalties = p.honor_penalties.saturating_add(pledge.honor_weight);
        broken_promises.push(BrokenPromiseRecord {
            pledge_id: pledge.pledge_id,
            promisor: pledge.promisor,
            counterparty: pledge.counterparty,
            weight: pledge.honor_weight,
        });
    }

    fn activate_shocks_for_turn(&mut self, turn: u8) {
        for shock in self
            .state
            .spec
            .shocks
            .iter()
            .chain(self.state.drawn_shocks.iter())
        {
            if shock.realize_turn == turn {
                let dur = match shock.kind {
                    ShockKind::DemandSurge => 2,
                    ShockKind::SupplyFreeze
                    | ShockKind::TariffHoliday
                    | ShockKind::SpoilageSpike => 1,
                };
                self.state.active_shocks.push(ActiveShock {
                    target_seat: shock.target_seat,
                    kind: shock.kind,
                    magnitude_bps: shock.magnitude_bps,
                    expires_after_turn: turn + dur - 1,
                });
            }
        }
    }

    fn step8_shocks_and_close(
        &mut self,
        turn: u8,
        turn_beacon: [u8; 32],
    ) -> (
        Vec<ScheduledShockSpec>,
        Vec<ShockForecastEvent>,
        Vec<ActiveShock>,
        bool,
    ) {
        let newly_drawn = if self.state.spec.shocks.is_empty() {
            let drawn = draw_shocks_for_turn(
                self.state.table_id,
                self.state.spec.seat_count,
                Turn(turn),
                turn_beacon,
            );
            self.state.drawn_shocks.extend(drawn.iter().copied());
            drawn
        } else {
            Vec::new()
        };

        let mut public_forecasts = Vec::new();
        let mut newly_activated = Vec::new();
        for shock in self
            .state
            .spec
            .shocks
            .iter()
            .chain(self.state.drawn_shocks.iter())
        {
            if shock.forecast_turn == turn {
                public_forecasts.push(ShockForecastEvent {
                    target_seat: shock.target_seat,
                    kind: shock.kind,
                    realize_turn: shock.realize_turn,
                });
            }
            if shock.realize_turn == turn {
                let dur = match shock.kind {
                    ShockKind::DemandSurge => 2,
                    ShockKind::SupplyFreeze
                    | ShockKind::TariffHoliday
                    | ShockKind::SpoilageSpike => 1,
                };
                newly_activated.push(ActiveShock {
                    target_seat: shock.target_seat,
                    kind: shock.kind,
                    magnitude_bps: shock.magnitude_bps,
                    expires_after_turn: turn + dur - 1,
                });
            }
        }
        self.step8_deterministic_cleanup_only(turn);

        let season_closed = if turn >= HARD_TURN_CAP {
            self.state.last_closing_roll_bps = None;
            true
        } else if (CLOSING_ROLL_START_TURN..HARD_TURN_CAP).contains(&turn) {
            let roll = close_roll_bps_from_beacon(turn_beacon, self.state.table_id, Turn(turn));
            self.state.last_closing_roll_bps = Some(roll);
            roll < self.state.spec.closing_hazard_bps[usize::from(turn - CLOSING_ROLL_START_TURN)]
        } else {
            self.state.last_closing_roll_bps = None;
            false
        };
        (
            newly_drawn,
            public_forecasts,
            newly_activated,
            season_closed,
        )
    }

    fn step8_deterministic_cleanup_only(&mut self, turn: u8) {
        self.state
            .active_shocks
            .retain(|s| s.expires_after_turn > turn);

        for seat in &mut self.state.seats {
            if seat.wash_frozen_until_turn <= turn {
                seat.wash_frozen_until_turn = 0;
            }
        }
        self.state.last_closing_roll_bps = None;
    }
}

/// Raw units of `Raw(seat_idx)` minted to `seat_idx` at Sweep Step 4b of `turn`: the base yield less
/// the strongest active `SupplyFreeze` on that seat's good.
pub(crate) fn raw_yield_at_sweep(state: &EngineState, seat_idx: usize, turn: u8) -> Units {
    let base_yield = state.spec.seats[seat_idx].raw_yield_per_turn;
    let freeze_bps = state
        .active_shocks
        .iter()
        .filter(|sh| {
            sh.kind == ShockKind::SupplyFreeze
                && sh.target_seat == seat_idx as u8
                && sh.expires_after_turn >= turn
        })
        .map(|sh| sh.magnitude_bps)
        .max()
        .unwrap_or(0);
    base_yield - floor_bps_part(base_yield, freeze_bps)
}

/// Effective Sweep Step 6 decay rate (bps) on `Raw(good)` for `turn`: the good's base rate raised by
/// the strongest active `SpoilageSpike` on it, capped at `9_900`.
pub(crate) fn spoilage_bps_at_sweep(state: &EngineState, good: usize, turn: u8) -> Bps {
    let base = state.spec.seats[good].perishable_decay_bps;
    let spike = state
        .active_shocks
        .iter()
        .filter(|sh| {
            sh.kind == ShockKind::SpoilageSpike
                && sh.target_seat == good as u8
                && sh.expires_after_turn >= turn
        })
        .map(|sh| sh.magnitude_bps)
        .max()
        .unwrap_or(0);
    let headroom = 10_000_i32 - i32::from(base);
    let boost = (headroom * i32::from(spike) + 9_999) / 10_000;
    (i32::from(base) + boost).min(9_900) as Bps
}

/// Sweep Step 5 upkeep settlement for one seat, before any state is changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UpkeepSettlement {
    /// Units of the upkeep good taken from the seat's free balance.
    pub(crate) paid_with_refined: Units,
    /// Units owed and not covered (`0` when covered).
    pub(crate) shortfall: Units,
    /// `CRED` auto-debt the shortfall adds, at the upkeep good's fundamental parity.
    pub(crate) new_shortfall_debt: Units,
}

/// Settles `owed` units of upkeep against `held` units, converting any shortfall to auto-debt at
/// the upkeep good's parity (milli-`CRED` per `1_000` units), exactly as Sweep Step 5 does. The
/// parity is read only when there is a shortfall, as Step 5 always did.
pub(crate) fn settle_upkeep(
    held: Units,
    owed: Units,
    parity: impl FnOnce() -> Units,
) -> UpkeepSettlement {
    let paid = held.min(owed);
    let shortfall = owed - paid;
    let new_shortfall_debt = if shortfall > 0 {
        ceil_units_parity(shortfall, parity())
    } else {
        0
    };
    UpkeepSettlement {
        paid_with_refined: paid,
        shortfall,
        new_shortfall_debt,
    }
}

/// Whole converter batches of `recipe` that the free raw balances `raw` cover, ignoring the
/// per-turn run cap (the same inventory clamp the Sweep applies to a converter order).
pub(crate) fn batches_covered_by_inputs(recipe: &ConverterRecipe, raw: &[Units]) -> u32 {
    recipe
        .inputs
        .iter()
        .map(|&(raw_g, amt)| {
            if amt <= 0 {
                0
            } else {
                let avail = raw.get(usize::from(raw_g)).copied().unwrap_or(0).max(0);
                u32::try_from(avail / amt).unwrap_or(u32::MAX)
            }
        })
        .min()
        .unwrap_or(0)
}

/// Base export tariff (bps) on `asset` set by its issuing seat (`0` for `CRED`).
pub(crate) fn base_tariff_bps(state: &EngineState, asset: AssetId) -> Bps {
    match asset {
        AssetId::Cred => 0,
        AssetId::Raw(g) => state
            .seats
            .get(usize::from(g))
            .map_or(0, |s| s.raw_tariff_bps),
        AssetId::Refined(m) => state
            .seats
            .get(usize::from(m))
            .map_or(0, |s| s.refined_tariff_bps),
    }
}

/// Strongest active `TariffHoliday` reduction (bps) on `asset`'s issuing seat, if any.
pub(crate) fn active_tariff_holiday_bps(state: &EngineState, asset: AssetId) -> Option<Bps> {
    let owner = match asset {
        AssetId::Cred => return None,
        AssetId::Raw(g) | AssetId::Refined(g) => g,
    };
    state
        .active_shocks
        .iter()
        .filter(|s| {
            s.target_seat == owner
                && s.kind == ShockKind::TariffHoliday
                && s.expires_after_turn >= state.turn
        })
        .map(|s| s.magnitude_bps)
        .max()
}

/// Export tariff (bps) in force on a transfer of `asset` from `sender` to `receiver`, in
/// `domain_id`'s book (`None` for the open book).
pub(crate) fn effective_transfer_tariff_bps(
    state: &EngineState,
    asset: AssetId,
    domain_id: Option<u8>,
    sender: u8,
    receiver: u8,
) -> Bps {
    effective_export_tariff_bps(
        base_tariff_bps(state, asset),
        domain_id,
        &state.domains,
        sender,
        receiver,
        active_tariff_holiday_bps(state, asset),
    )
}

fn pick_plurality_winner_with_beacon(
    votes: &[u32],
    incumbent: u8,
    turn_beacon: [u8; 32],
    table_id: TableId,
    turn: Turn,
    domain_tag: &[u8],
) -> (u8, bool) {
    let max_votes = votes.iter().copied().max().unwrap_or(0);
    if max_votes == 0 {
        return (incumbent, false);
    }
    let tied: Vec<u8> = votes
        .iter()
        .enumerate()
        .filter_map(|(idx, &v)| (v == max_votes).then_some(idx as u8))
        .collect();
    (
        break_tie_with_turn_beacon(turn_beacon, table_id, turn, domain_tag, &tied),
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upkeep_settlement_reads_parity_only_on_a_shortfall_and_input_cover_never_panics() {
        // A covered upkeep never reads the parity (the review's input: 2_000 held, 1_800 owed,
        // no parity at all), and settles as Step 5 always did.
        let covered = settle_upkeep(2_000, 1_800, || panic!("parity read without a shortfall"));
        assert_eq!(
            covered,
            UpkeepSettlement {
                paid_with_refined: 1_800,
                shortfall: 0,
                new_shortfall_debt: 0,
            }
        );
        let short = settle_upkeep(500, 1_800, || 2_000);
        assert_eq!((short.shortfall, short.new_shortfall_debt), (1_300, 2_600));

        // Step 5 itself on a state whose parity table is empty but every seat is covered.
        let Ok(spec) = TableSpec::standard_chiral_ring(TableId(77), 4, 1) else {
            panic!("spec");
        };
        let Ok(mut engine) = TurnEngine::new(spec) else {
            panic!("engine");
        };
        engine.state.parities.refined_per_unit = Vec::new();
        for seat in &mut engine.state.seats {
            seat.refined = vec![2_000; 4];
        }
        let (debt, by_seat) = engine.step5_collect_upkeep_and_debt(&[0; 4]);
        assert_eq!(debt, 0);
        assert!(
            by_seat
                .iter()
                .all(|u| u.paid_with_refined == 1_800 && u.new_shortfall_debt == 0)
        );

        // The review's second input: one raw balance, a recipe input on good 2 -> 0 batches.
        let recipe = ConverterRecipe {
            owner: 0,
            inputs: vec![(2, 2_000)],
            output_good: 0,
            output_amount: 3_500,
            max_runs_per_turn: 4,
            is_secondary: false,
        };
        assert_eq!(batches_covered_by_inputs(&recipe, &[1_000]), 0);
        assert_eq!(batches_covered_by_inputs(&recipe, &[0, 0, 4_100]), 2);
    }
    use crate::commit::{RevealSalt, compute_order_commitment, derive_turn_beacon};

    #[test]
    fn turn_engine_handles_commit_reveal_errors_and_three_miss_ai_conversion() {
        let Ok(spec) = TableSpec::standard_chiral_ring(TableId(7), 4, 42) else {
            panic!("valid spec");
        };
        let m_hash = spec.dealt_mandate_commits[0];
        let Ok(mut engine) = TurnEngine::new(spec) else {
            panic!("init ok");
        };
        assert!(engine.submit_mandate_commitment(Seat(0), m_hash).is_ok());
        assert!(matches!(
            engine.submit_mandate_commitment(Seat(0), m_hash),
            Err(EngineError::AlreadySubmitted(0))
        ));

        for t in 1..=3 {
            assert!(engine.advance_to_commit().is_ok());
            // Seat 0 skips commit for 3 turns -> triggers Regent + converts to AiSyndicate.
            let dummy = OrderReveal {
                bundle: SealedOrderBundle::default(),
                salt: RevealSalt([t; 32]),
            };
            for s in 1..4 {
                let Ok(c) = compute_order_commitment(TableId(7), Turn(t), Seat(s), &dummy) else {
                    panic!("commit hash");
                };
                assert!(engine.submit_order_commitment(Seat(s), c).is_ok());
            }
            assert!(engine.advance_to_reveal().is_ok());
            for s in 1..4 {
                assert!(engine.submit_order_reveal(Seat(s), dummy.clone()).is_ok());
            }
            let beacon = derive_turn_beacon(42, TableId(7), Turn(t));
            let Ok(receipt) = engine.sweep_turn(beacon) else {
                panic!("sweep ok");
            };
            assert!(receipt.non_reveal_seats.contains(&0));
            assert!(receipt.regent_acted_seats.contains(&0));
            if t == 3 {
                assert!(receipt.converted_to_ai.contains(&0));
            }
        }
        assert_eq!(engine.state().seats[0].identity, SeatIdentity::AiSyndicate);
    }

    #[test]
    fn escrow_rejects_perishable_and_credits_bond_compliance_vs_breach() {
        let Ok(spec) = TableSpec::standard_chiral_ring(TableId(9), 4, 11) else {
            panic!("valid spec");
        };
        let Ok(mut engine) = TurnEngine::new(spec) else {
            panic!("init ok");
        };

        // Perishable Escrow must fail cleanly.
        let bad = engine.apply_market_action(MarketAction::CreateEscrow {
            seat: 0,
            escrow: EscrowProposal {
                destination: 1,
                asset: AssetId::Raw(0),
                amount: 1_000,
                finish_after_turn: Some(1),
                cancel_after_turn: 3,
                condition_hash: None,
                bond: None,
            },
        });
        assert_eq!(bad, Err(EngineError::PerishableEscrowForbidden));

        let preimage = crate::commit::PreimageSecret([0x5A; 32]);
        let Ok(cond) = crate::commit::compute_preimage_condition(&preimage) else {
            panic!("valid non-zero preimage");
        };
        assert!(format!("{preimage:?}").contains("<hidden>"));
        assert!(!format!("{preimage:?}").contains("5a"));

        // Conditional escrow unlocked in Turn 1 Reveal + unfulfilled Bond breaching on Turn 2.
        assert!(
            engine
                .apply_market_action(MarketAction::CreateEscrow {
                    seat: 0,
                    escrow: EscrowProposal {
                        destination: 1,
                        asset: AssetId::Cred,
                        amount: 1_000,
                        finish_after_turn: Some(1),
                        cancel_after_turn: 4,
                        condition_hash: Some(cond),
                        bond: None,
                    },
                })
                .is_ok()
        );
        assert!(
            engine
                .apply_market_action(MarketAction::CreateEscrow {
                    seat: 2,
                    escrow: EscrowProposal {
                        destination: 3,
                        asset: AssetId::Cred,
                        amount: 2_000,
                        finish_after_turn: None,
                        cancel_after_turn: 3,
                        condition_hash: Some(cond),
                        bond: Some(crate::contracts::BondProposal {
                            good: AssetId::Raw(2),
                            required_units: 5_000,
                            deadline_turn: 2,
                            mode: BondSettlementMode::SellerPerformanceBond,
                        }),
                    },
                })
                .is_ok()
        );

        let rev0 = OrderReveal {
            bundle: SealedOrderBundle {
                escrow_finishes: vec![EscrowFinishUnlock {
                    escrow_id: 1,
                    preimage: Some(preimage),
                }],
                ..SealedOrderBundle::default()
            },
            salt: RevealSalt([1; 32]),
        };
        let Ok(c0) = compute_order_commitment(TableId(9), Turn(1), Seat(0), &rev0) else {
            panic!("hash");
        };
        assert!(engine.advance_to_commit().is_ok());
        assert!(engine.submit_order_commitment(Seat(0), c0).is_ok());
        assert!(engine.advance_to_reveal().is_ok());
        assert!(engine.submit_order_reveal(Seat(0), rev0).is_ok());
        let Ok(r1) = engine.sweep_turn(derive_turn_beacon(1, TableId(9), Turn(1))) else {
            panic!("sweep 1");
        };
        assert!(r1.escrows_finished.contains(&1));

        assert!(engine.advance_to_commit().is_ok());
        assert!(engine.advance_to_reveal().is_ok());
        let Ok(r2) = engine.sweep_turn(derive_turn_beacon(1, TableId(9), Turn(2))) else {
            panic!("sweep 2");
        };
        assert!(r2.escrows_cancelled_or_forfeited.contains(&2));
        assert_eq!(engine.state().seats[2].bond_breaches, 1);
        assert_eq!(engine.state().seats[2].honor_penalties, 2_000);
    }
}
