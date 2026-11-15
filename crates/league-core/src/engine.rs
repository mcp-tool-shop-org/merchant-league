//! Pure-state simultaneous turn engine (`TalkAndMarket -> Commit -> Reveal -> 8-Step Sweep`)
//! for Merchant League v2.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};

use crate::commit::{
    AtomicSwapProposal, CommitmentHash, ConverterRunOrder, CouncilPlatform, CouncilVoteBallot,
    EscrowFinishUnlock, IntentPledge, IntentTarget, MandateReveal, OrderReveal, RevealSalt,
    SealedOrderBundle, beacon_roll_bps, break_tie_with_beacon, compute_order_commitment,
    derive_drand_beacon, derive_turn_beacon, raw_zero_preimage_condition_hash,
    verify_drand_quicknet_beacon, verify_mandate_reveal, verify_order_reveal,
    verify_preimage_condition,
};
use crate::contracts::{
    ActiveCharter, BondSpec, CartelDomain, CheckStatus, CredOffer, CreditCheck, EscrowProposal,
    EscrowStatus, MAX_CARTEL_DOMAINS, MarketAction, OfferProposal, OfferSide, TokenEscrowVault,
    check_offer_domain_access, effective_export_tariff_bps,
};
use crate::scoring::{
    FinalScoreboard, FundamentalParities, compute_final_scoreboard, compute_pbrs_reward,
    compute_potential,
};
use crate::types::{
    ActiveShock, AssetId, Bps, CharterRole, CreditTier, DEFAULT_PBRS_GAMMA_BPS, EngineError,
    MAX_TARIFF_BPS, ScheduledShockSpec, Seat, SeatIdentity, ShockKind, TableId, TableMode,
    TableSpec, Turn, Units, ceil_bps_fee, ceil_units_parity, checked_ceil_units_parity,
    checked_mul_units_parity, floor_bps_part, half_up_bps,
};

type PoliticsOutcome = (Option<(u8, u8)>, Vec<(CharterRole, u8)>, Units);

mod delivery_ledger_serde {
    use super::*;

    type LedgerMap = BTreeMap<(u8, u8, AssetId), Units>;
    type LedgerEntry = ((u8, u8, AssetId), Units);

    pub fn serialize<S: Serializer>(map: &LedgerMap, serializer: S) -> Result<S::Ok, S::Error> {
        let entries: Vec<LedgerEntry> = map.iter().map(|(&k, &v)| (k, v)).collect();
        entries.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<LedgerMap, D::Error> {
        let entries = Vec::<LedgerEntry>::deserialize(deserializer)?;
        let mut map = LedgerMap::new();
        for (k, v) in entries {
            if map.insert(k, v).is_some() {
                return Err(serde::de::Error::custom("duplicate key in delivery_ledger"));
            }
        }
        Ok(map)
    }
}

/// Active phase within a simultaneous turn or season close lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnPhase {
    /// Phase 1: open `CRED` market quotes, Track-2 checks/pledges, and Track-1 `TokenEscrow` setup.
    TalkAndMarket,
    /// Phase 2: sealed cryptographic commitment submission for simultaneous actions.
    Commit,
    /// Reveal gate: seats submit plaintext bundles + salts matching Phase 2 commitments.
    Reveal,
    /// Closing bell sounded on Turn `24..=28`; waiting for Turn-1 Private Mandate reveals.
    AwaitingMandateReveal,
    /// Season finalized cleanly; [`FinalScoreboard`] computed.
    Completed,
}

/// Per-seat mutable ledger-mirror state maintained deterministically across turns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatState {
    /// Seat index (`0..seat_count`).
    pub seat: u8,
    /// Current disclosed identity (`Human` or converted `AiSyndicate`).
    pub identity: SeatIdentity,
    /// Free `CRED` balance (excluding DEX buy locks and `TokenEscrow` vaults).
    pub cred: Units,
    /// Unpaid `CRED` auto-debt principal + accrued interest owed to Authority.
    pub cred_debt: Units,
    /// Free raw commodity balances `raw[0..N]` (perishable).
    pub raw: Vec<Units>,
    /// Free refined commodity balances `refined[0..N]`.
    pub refined: Vec<Units>,
    /// Irreversible Tier-3 Syndicate Project Equity points earned.
    pub project_equity: Units,
    /// Current export `TransferRate` tariff on `Raw(seat)` (`0..=3_500 bps`).
    pub raw_tariff_bps: Bps,
    /// Current export `TransferRate` tariff on `Refined(seat)` (`0..=3_500 bps`).
    pub refined_tariff_bps: Bps,
    /// Current Hard Credit Solvency tier (`Aaa..D`).
    pub credit_tier: CreditTier,
    /// Count of `CheckCash` bounces causing Hard Credit Defaults.
    pub hard_credit_defaults: u32,
    /// Count of defaulted Steward-refereed delivery bonds.
    pub bond_breaches: u32,
    /// Count of Soft Diplomatic Honor promise breaches detected by deterministic audit.
    pub broken_promises: u32,
    /// Cumulative honor penalty weight.
    pub honor_penalties: Units,
    /// Consecutive turns with missing/invalid Phase 2 reveal (`3` triggers AI conversion).
    pub consecutive_misses: u8,
    /// Turn through which `CustomsInspector` wash-freeze blocks new offers/swaps (`0` = none).
    pub wash_frozen_until_turn: u8,
}

/// Complete deterministic engine snapshot serializable for replay and digest checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineState {
    /// Table identifier.
    pub table_id: TableId,
    /// Current turn index (`1..=28`).
    pub turn: u8,
    /// Current turn phase or terminal close state.
    pub phase: TurnPhase,
    /// Current Trade Minister seat index (`0..seat_count`).
    pub trade_minister: u8,
    /// Current Cartel Chair seat index (`0..seat_count`).
    pub cartel_chair: u8,
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
    /// Latest beacon roll basis points on Closing Bell evaluation (turns `24..=28`).
    pub last_closing_roll_bps: Option<Bps>,
}

/// Audited broken-promise record emitted when a seat violates an anchored `IntentPledge`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrokenPromiseRecord {
    /// Violated pledge ID.
    pub pledge_id: u64,
    /// Promisor seat penalized.
    pub promisor: u8,
    /// Counterparty affected.
    pub counterparty: u8,
    /// Penalty honor weight charged.
    pub weight: Units,
}

/// Publicly narrowed forecast emitted in Sweep Step 8 (`forecast_turn`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShockForecastEvent {
    /// Seat whose commodity sector is forecasted to shift next turn.
    pub target_seat: u8,
    /// Forecasted shock category.
    pub kind: ShockKind,
    /// Forecasted shock turn (`realize_turn`).
    pub realize_turn: u8,
}

/// Deterministic audit receipt capturing every sub-step executed during a Turn Sweep.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SweepReceipt {
    /// Turn index that just swept.
    pub turn: u8,
    /// Seats that missed commit or valid reveal in Step 1.
    pub non_reveal_seats: Vec<u8>,
    /// Absent human seats for which the Standing Regent synthesized upkeep actions.
    pub regent_acted_seats: Vec<u8>,
    /// Seats converted from `Human` to `AiSyndicate` after 3 consecutive misses.
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
    /// Elected `(trade_minister, cartel_chair)` if `turn % 4 == 0`.
    pub elected_officers: Option<(u8, u8)>,
    /// Awarded charter `(role, winner_seat)` pairs if `turn % 6 == 0`.
    pub charters_awarded: Vec<(CharterRole, u8)>,
    /// Publicly forecasted Horizon Shocks announced in Step 8 (`forecast_turn`).
    pub public_forecasts: Vec<ShockForecastEvent>,
    /// Newly activated realized shocks enacted for `turn` (`realize_turn == turn`).
    pub newly_activated_shocks: Vec<ActiveShock>,
    /// Total voluntary `CRED` auto-debt repaid in Step 2.
    pub cred_debt_repaid: Units,
    /// Total `CRED` burned by Syndicate Charter auctions in Step 7.
    pub charter_cred_burned: Units,
    /// `true` if the season closed on this Sweep (moving into `AwaitingMandateReveal`).
    pub season_closed: bool,
    /// Per-seat PBRS shaped returns `F_i = gamma * Psi_{t+1} - Psi_t` (`DEFAULT_PBRS_GAMMA_BPS`).
    pub pbrs_deltas: Vec<i64>,
    /// Canonical SHA-256 digest of post-Sweep [`EngineState`].
    pub post_state_digest: [u8; 32],
}

/// Preview breakdown of one [`ConverterRunOrder`] simulated inside [`TurnEngine::preview_bundle`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConverterOrderPreview {
    /// Converter recipe index requested.
    pub recipe_idx: usize,
    /// Original run count submitted in the bundle.
    pub requested_runs: u32,
    /// Whole runs executed after capacity and inventory clamping.
    pub executed_runs: u32,
    /// `true` if clamped (`0 < executed_runs < requested_runs`).
    pub clamped: bool,
}

/// Preview breakdown of one [`ProjectDeliveryOrder`] simulated inside [`TurnEngine::preview_bundle`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectOrderPreview {
    /// Target Syndicate Project ID requested.
    pub project_id: u16,
    /// Original batch count submitted in the bundle.
    pub requested_batches: u32,
    /// Whole batches executed after refined-inventory clamping.
    pub executed_batches: u32,
    /// `true` if clamped (`0 < executed_batches < requested_batches`).
    pub clamped: bool,
}

/// Outcome of an [`AtomicSwapProposal`] simulated in [`TurnEngine::preview_bundle`].
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

/// Outcome of a `CheckCash` request simulated in [`TurnEngine::preview_bundle`].
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

/// Preview breakdown of one `EscrowFinish` unlock request in Step 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EscrowOrderPreview {
    /// Escrow vault identifier.
    pub escrow_id: u64,
    /// `true` if the escrow unlocked and released funds to its destination.
    pub unlocked: bool,
}

/// Per-seat Step-2 debt repayment and Step-5 upkeep/interest accounting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatUpkeepPreview {
    /// Refined commodity index consumed as periodic upkeep.
    pub upkeep_good: u8,
    /// Total upkeep units due each turn.
    pub upkeep_required: Units,
    /// Refined units deducted toward upkeep.
    pub paid_with_refined: Units,
    /// Existing `cred_debt` interest accrued this turn before upkeep shortfall.
    pub debt_interest_accrued: Units,
    /// Newly issued shortfall debt from unpaid upkeep.
    pub new_shortfall_debt: Units,
    /// Voluntary debt repaid during Step 2.
    pub debt_repaid_cred: Units,
}

/// Side-effect-free simulation result returned by [`TurnEngine::preview_bundle`] and
/// [`TurnEngine::preview_bundle_with_context`].
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

struct SweepSeatDetails {
    converter_orders_by_seat: Vec<Vec<ConverterOrderPreview>>,
    project_orders_by_seat: Vec<Vec<ProjectOrderPreview>>,
    settled_swaps: Vec<AtomicSwapProposal>,
    upkeep_by_seat: Vec<SeatUpkeepPreview>,
    spoilage_by_seat: Vec<Units>,
}

/// Deterministic pure-state v2 Turn Engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnEngine {
    state: EngineState,
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
                wash_frozen_until_turn: 0,
            })
            .collect();

        let mut engine = Self {
            state: EngineState {
                table_id: spec.table_id,
                turn: 1,
                phase: TurnPhase::TalkAndMarket,
                trade_minister: 0,
                cartel_chair: 0,
                spec,
                parities,
                seats,
                offers: BTreeMap::new(),
                checks: BTreeMap::new(),
                escrows: BTreeMap::new(),
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
            },
        };
        engine.activate_shocks_for_turn(1);
        let initial_pots: Vec<Units> = (0..n as u8)
            .map(|s| compute_potential(&engine.state, s))
            .collect();
        engine.state.last_potentials = initial_pots;
        Ok(engine)
    }

    /// Returns an immutable reference to the full engine state snapshot.
    #[must_use]
    pub const fn state(&self) -> &EngineState {
        &self.state
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
    pub fn private_hints_for_auditor(
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
                            || office_turn > 28
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
                            || office_turn > 28
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
                self.state.seats[usize::from(target_seat)].wash_frozen_until_turn = self.state.turn;
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
        }
    }

    /// Transitions current turn from `TalkAndMarket` -> `Commit`.
    pub fn advance_to_commit(&mut self) -> Result<(), EngineError> {
        if self.state.phase != TurnPhase::TalkAndMarket {
            return Err(EngineError::InvalidPhase(
                "must be in TalkAndMarket to enter Commit".to_owned(),
            ));
        }
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
        for swap in &reveal.bundle.atomic_swaps {
            self.ensure_valid_asset(swap.maker_asset)?;
            self.ensure_valid_asset(swap.taker_asset)?;
        }
        self.state.turn_reveals.insert(seat.0, reveal);
        Ok(())
    }

    /// Side-effect-free simulation of `market_actions` and `bundle` for `seat` on a cloned engine
    /// assuming empty peer bundles and a deterministic preview beacon (`derive_turn_beacon(0, ...)`).
    /// Never modifies `self`.
    pub fn preview_bundle(
        &self,
        seat: Seat,
        market_actions: &[MarketAction],
        bundle: &SealedOrderBundle,
    ) -> Result<BundlePreview, EngineError> {
        let beacon = derive_turn_beacon(0, self.state.table_id, Turn(self.state.turn));
        self.preview_bundle_with_context(seat, market_actions, bundle, &BTreeMap::new(), beacon)
    }

    /// Side-effect-free simulation of `market_actions` and `bundle` for `seat` given explicit
    /// `peer_bundles` and `turn_beacon`. Leaves `self` untouched.
    pub fn preview_bundle_with_context(
        &self,
        seat: Seat,
        market_actions: &[MarketAction],
        bundle: &SealedOrderBundle,
        peer_bundles: &BTreeMap<u8, SealedOrderBundle>,
        turn_beacon: [u8; 32],
    ) -> Result<BundlePreview, EngineError> {
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
        for swap in &bundle.atomic_swaps {
            self.ensure_valid_asset(swap.maker_asset)?;
            self.ensure_valid_asset(swap.taker_asset)?;
        }
        for peer_bundle in peer_bundles.values() {
            for swap in &peer_bundle.atomic_swaps {
                self.ensure_valid_asset(swap.maker_asset)?;
                self.ensure_valid_asset(swap.taker_asset)?;
            }
        }

        let turn = self.state.turn;
        let seat_idx = usize::from(seat.0);
        let n = self.state.seats.len();
        let was_talk_and_market = self.state.phase == TurnPhase::TalkAndMarket;

        let mut sim = self.clone();
        let pre_seat_state = sim.state.seats[seat_idx].clone();
        let mut refused_market_actions = Vec::new();
        if was_talk_and_market {
            for (idx, action) in market_actions.iter().enumerate() {
                if let Err(err) = sim.apply_market_action(action.clone()) {
                    refused_market_actions.push((idx, err.to_string()));
                }
            }
        } else if !market_actions.is_empty() {
            return Err(EngineError::InvalidPhase(
                "market_actions require TalkAndMarket phase".to_owned(),
            ));
        }
        let post_market_seat_state = sim.state.seats[seat_idx].clone();

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
        if was_talk_and_market {
            let empty = SealedOrderBundle::default();
            for s in 0..n as u8 {
                if s != seat.0 && !peer_bundles.contains_key(&s) {
                    insert_preview_reveal(s, &empty)?;
                }
            }
        }
        sim.state.phase = TurnPhase::Reveal;

        let (sweep_receipt, details) = sim.sweep_turn_detailed(turn_beacon)?;
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

        Ok(BundlePreview {
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
        })
    }

    /// Executes the 8-step deterministic `Sweep` for the current Eval-mode turn using `turn_beacon`.
    ///
    /// Refuses `TableMode::Game` tables, which must sweep via [`TurnEngine::sweep_turn_with_drand`].
    pub fn sweep_turn(&mut self, turn_beacon: [u8; 32]) -> Result<SweepReceipt, EngineError> {
        if self.state.spec.mode != TableMode::Eval {
            return Err(EngineError::InvalidConfig(
                "Game mode requires sweep_turn_with_drand with scheduled round and signature"
                    .to_owned(),
            ));
        }
        Ok(self.sweep_turn_detailed(turn_beacon)?.0)
    }

    /// Executes the 8-step deterministic `Sweep` for an Eval-mode turn using a beacon derived from
    /// the independent `beacon_seed` (`Decision 12` & `2026-11-15` addendum).
    pub fn sweep_turn_seeded(&mut self, beacon_seed: u64) -> Result<SweepReceipt, EngineError> {
        if self.state.spec.mode != TableMode::Eval {
            return Err(EngineError::InvalidConfig(
                "sweep_turn_seeded requires TableMode::Eval".to_owned(),
            ));
        }
        let beacon = derive_turn_beacon(beacon_seed, self.state.table_id, Turn(self.state.turn));
        self.sweep_turn(beacon)
    }

    /// Executes the 8-step deterministic `Sweep` for a Game-mode turn using an `ExternalDrand`
    /// `quicknet` round and 48-byte signature validated against `spec.drand_schedule` (`M2`).
    ///
    /// Derives `turn_beacon = SHA-256(drand_signature)` and requires `drand_round` to equal
    /// `schedule.expected_round_for_turn(current_turn)`. Note: BLS12-381 signature
    /// authentication against the `quicknet` chain public key follows in the live oracle layer.
    pub fn sweep_turn_with_drand(
        &mut self,
        drand_round: u64,
        drand_signature: &[u8],
    ) -> Result<SweepReceipt, EngineError> {
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
        verify_drand_quicknet_beacon(
            &schedule,
            Turn(self.state.turn),
            drand_round,
            drand_signature,
            beacon,
        )?;
        Ok(self.sweep_turn_detailed(beacon)?.0)
    }

    fn sweep_turn_detailed(
        &mut self,
        turn_beacon: [u8; 32],
    ) -> Result<(SweepReceipt, SweepSeatDetails), EngineError> {
        if self.state.phase != TurnPhase::Reveal {
            return Err(EngineError::InvalidPhase(
                "sweep_turn requires Reveal phase".to_owned(),
            ));
        }
        let turn = self.state.turn;
        let n = self.state.seats.len();

        // Step 1: Validate reveals, run Regent fallback (non-committers only), track 3-miss AI conversion.
        let (non_reveal_seats, regent_acted_seats, converted_to_ai, active_bundles) =
            self.step1_validate_reveals_and_regents(turn, n);

        // Step 2: Settle EscrowFinish reveals, co-signed Batch swaps, Steward bond & expiry checks,
        // voluntary auto-debt repayments, and Tier-2 converters.
        let mut broken_promises = Vec::new();
        let mut escrows_finished = self.step2a_finish_preimage_escrows(turn, &active_bundles);
        let (atomic_swaps_settled, settled_swaps) =
            self.step2_settle_atomic_swaps(turn, &active_bundles)?;
        let (bond_finished, escrows_cancelled_or_forfeited) =
            self.step2b_evaluate_bonds_and_expired_escrows(turn, &mut broken_promises);
        escrows_finished.extend(bond_finished);
        let (cred_debt_repaid, repaid_by_seat) =
            self.step2_voluntary_debt_repayment(&active_bundles);
        let (converter_batches_run, converter_orders_clamped, converter_orders_by_seat) =
            self.step2_settle_converters(&active_bundles);

        // Step 3: Clear maturing Track-2 CheckCash requests & apply Check bounce downgrades.
        let (checks_cashed, checks_bounced, cartel_evictions) =
            self.step3_clear_checks(turn, &active_bundles);

        // Step 4: Burn Tier-3 Syndicate Projects & mint Tier-1 raw extraction yields.
        let (projects_completed, project_orders_clamped, project_orders_by_seat) =
            self.step4_projects_and_yields(turn, &active_bundles);

        // Step 5: Accrue auto-debt interest & collect chiral upkeep (issuing shortfall auto-debt).
        let (total_upkeep_shortfall_debt, upkeep_by_seat) =
            self.step5_collect_upkeep_and_debt(&repaid_by_seat);

        // Step 6: Apply perishable Clawback spoilage on remaining raw goods.
        let (total_spoilage_clawed_back, spoilage_by_seat) =
            self.step6_apply_perishable_spoilage(turn);

        // Step 7: Audit vote pledges, settle 4-turn council politics, and 6-turn charters.
        let (elected_officers, charters_awarded, charter_cred_burned) = self
            .step7_politics_and_charters(turn, turn_beacon, &active_bundles, &mut broken_promises);

        // Step 8: Advance Horizon Shocks and evaluate stochastic close roll (Turns 24..=28).
        let (public_forecasts, newly_activated_shocks, season_closed) =
            self.step8_shocks_and_close(turn, turn_beacon);

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
            self.state.turn = turn + 1;
            self.activate_shocks_for_turn(turn + 1);
            self.state.phase = TurnPhase::TalkAndMarket;
        }

        let post_state_digest = self.state_digest()?;
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
            cred_debt_repaid,
            charter_cred_burned,
            season_closed,
            pbrs_deltas,
            post_state_digest,
        };
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

    fn has_active_charter(&self, seat: u8, role: CharterRole) -> bool {
        self.state
            .charters
            .iter()
            .any(|c| c.role == role && c.holder == seat && c.expires_at_turn >= self.state.turn)
    }

    fn base_tariff_bps(&self, asset: AssetId) -> Bps {
        match asset {
            AssetId::Cred => 0,
            AssetId::Raw(g) => self
                .state
                .seats
                .get(usize::from(g))
                .map_or(0, |s| s.raw_tariff_bps),
            AssetId::Refined(m) => self
                .state
                .seats
                .get(usize::from(m))
                .map_or(0, |s| s.refined_tariff_bps),
        }
    }

    fn active_tariff_holiday_bps(&self, asset: AssetId) -> Option<Bps> {
        let owner = match asset {
            AssetId::Cred => return None,
            AssetId::Raw(g) | AssetId::Refined(g) => g,
        };
        self.state
            .active_shocks
            .iter()
            .filter(|s| {
                s.target_seat == owner
                    && s.kind == ShockKind::TariffHoliday
                    && s.expires_after_turn >= self.state.turn
            })
            .map(|s| s.magnitude_bps)
            .max()
    }

    fn effective_asset_tariff_bps(
        &self,
        asset: AssetId,
        domain_id: Option<u8>,
        sender: u8,
        receiver: u8,
    ) -> Bps {
        effective_export_tariff_bps(
            self.base_tariff_bps(asset),
            domain_id,
            &self.state.domains,
            sender,
            receiver,
            self.active_tariff_holiday_bps(asset),
        )
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
        match order.side {
            OfferSide::Buy => {
                if self.free_balance(seat, AssetId::Cred) < max_lock {
                    return Err(EngineError::InsufficientBalance(format!(
                        "seat {seat} needs {max_lock} CRED to post buy offer"
                    )));
                }
            }
            OfferSide::Sell => {
                if self.free_balance(seat, order.good) < order.amount {
                    return Err(EngineError::InsufficientBalance(format!(
                        "seat {seat} lacks {} units to sell",
                        order.amount
                    )));
                }
            }
        }

        let mut remaining = order.amount;
        let matching_ids = self.sorted_crossable_offer_ids(seat, &order, broker);

        for maker_id in matching_ids {
            if remaining <= 0 {
                break;
            }
            let Some(maker) = self.state.offers.get(&maker_id).cloned() else {
                continue;
            };
            let fill = remaining.min(maker.remaining_units);
            let trade_cred = if fill == maker.remaining_units && maker.side == OfferSide::Buy {
                maker.locked_cred
            } else {
                let computed = checked_mul_units_parity(fill, maker.price_per_unit_cred)?;
                if maker.side == OfferSide::Buy {
                    computed.min(maker.locked_cred)
                } else {
                    computed.max(if fill > 0 { 1 } else { 0 })
                }
            };

            let (seller, buyer) = match order.side {
                OfferSide::Buy => (maker.maker, seat),
                OfferSide::Sell => (seat, maker.maker),
            };
            // Verify buyer still has free balance if taker is Buy (for maker Sell).
            if order.side == OfferSide::Buy && self.free_balance(buyer, AssetId::Cred) < trade_cred
            {
                break;
            }

            let tariff_dom = maker.domain_id.or(order.domain_id);
            let tariff_bps = self.effective_asset_tariff_bps(order.good, tariff_dom, seller, buyer);
            let tariff_units = ceil_bps_fee(fill, tariff_bps);
            let net_commodity = fill - tariff_units;

            if order.side == OfferSide::Buy {
                // Taker buys from resting Sell: taker pays `trade_cred` from free CRED, receives net_commodity.
                self.adjust_free_balance(buyer, AssetId::Cred, -trade_cred)?;
                self.adjust_free_balance(seller, AssetId::Cred, trade_cred)?;
                self.adjust_free_balance(buyer, order.good, net_commodity)?;
            } else {
                // Taker sells to resting Buy: maker already locked CRED, taker transfers from free good.
                self.adjust_free_balance(seller, order.good, -fill)?;
                self.adjust_free_balance(buyer, order.good, net_commodity)?;
                self.adjust_free_balance(seller, AssetId::Cred, trade_cred)?;
            }
            self.record_delivery(seller, buyer, order.good, net_commodity);

            remaining -= fill;
            if let Some(m_entry) = self.state.offers.get_mut(&maker_id) {
                m_entry.remaining_units -= fill;
                if m_entry.side == OfferSide::Buy {
                    m_entry.locked_cred -= trade_cred;
                }
                if m_entry.remaining_units <= 0 {
                    self.state.offers.remove(&maker_id);
                }
            }
        }

        if remaining > 0 {
            let offer_id = self.state.next_offer_id;
            self.state.next_offer_id += 1;
            let locked_cred = match order.side {
                OfferSide::Buy => {
                    let lock = checked_ceil_units_parity(remaining, order.price_per_unit_cred)?;
                    self.adjust_free_balance(seat, AssetId::Cred, -lock)?;
                    lock
                }
                OfferSide::Sell => {
                    self.adjust_free_balance(seat, order.good, -remaining)?;
                    0
                }
            };
            self.state.offers.insert(
                offer_id,
                CredOffer {
                    offer_id,
                    maker: seat,
                    side: order.side,
                    good: order.good,
                    remaining_units: remaining,
                    price_per_unit_cred: order.price_per_unit_cred,
                    locked_cred,
                    domain_id: order.domain_id,
                    is_hybrid: order.is_hybrid,
                    created_turn: self.state.turn,
                },
            );
        }

        Ok(())
    }

    fn sorted_crossable_offer_ids(
        &self,
        taker: u8,
        order: &OfferProposal,
        taker_has_broker: bool,
    ) -> Vec<u64> {
        let mut cands: Vec<&CredOffer> = self
            .state
            .offers
            .values()
            .filter(|m| {
                m.good == order.good
                    && m.side != order.side
                    && m.maker != taker
                    && m.remaining_units > 0
                    && match order.side {
                        OfferSide::Buy => m.price_per_unit_cred <= order.price_per_unit_cred,
                        OfferSide::Sell => m.price_per_unit_cred >= order.price_per_unit_cred,
                    }
                    && self.offers_can_cross(
                        taker,
                        order.domain_id,
                        order.is_hybrid,
                        taker_has_broker,
                        m,
                    )
            })
            .collect();

        cands.sort_by(|a, b| {
            let price_cmp = match order.side {
                OfferSide::Buy => a.price_per_unit_cred.cmp(&b.price_per_unit_cred),
                OfferSide::Sell => b.price_per_unit_cred.cmp(&a.price_per_unit_cred),
            };
            price_cmp.then_with(|| a.offer_id.cmp(&b.offer_id))
        });
        cands.into_iter().map(|m| m.offer_id).collect()
    }

    fn offers_can_cross(
        &self,
        taker: u8,
        taker_domain: Option<u8>,
        taker_hybrid: bool,
        taker_broker: bool,
        maker: &CredOffer,
    ) -> bool {
        if self.state.seats[usize::from(maker.maker)].wash_frozen_until_turn >= self.state.turn {
            return false;
        }
        if (taker_domain.is_some() || maker.domain_id.is_some())
            && (!self.state.seats[usize::from(taker)]
                .credit_tier
                .may_access_prime_cartel()
                || !self.state.seats[usize::from(maker.maker)]
                    .credit_tier
                    .may_access_prime_cartel())
        {
            return false;
        }
        match (taker_domain, maker.domain_id) {
            (None, None) => true,
            (Some(td), Some(md)) if td == md => {
                if let Some(dom) = self.state.domains.iter().find(|d| d.domain_id == td) {
                    (dom.members.contains(&taker) || (taker_hybrid && taker_broker))
                        && (dom.members.contains(&maker.maker) || maker.is_hybrid)
                } else {
                    false
                }
            }
            (None, Some(_)) | (Some(_), None) | (Some(_), Some(_)) => {
                (taker_hybrid && taker_broker) || maker.is_hybrid
            }
        }
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
        if seat == proposal.destination
            || proposal.amount <= 0
            || proposal.cancel_after_turn <= self.state.turn
            || (proposal.finish_after_turn.is_none() && proposal.condition_hash.is_none())
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
                self.ensure_valid_asset(bp.good)?;
                if bp.required_units <= 0
                    || bp.deadline_turn < self.state.turn
                    || bp.deadline_turn >= proposal.cancel_after_turn
                {
                    return Err(EngineError::InvalidOrder(
                        "invalid Steward Bond deadline or required_units".to_owned(),
                    ));
                }
                let baseline = self
                    .state
                    .delivery_ledger
                    .get(&(seat, proposal.destination, bp.good))
                    .copied()
                    .unwrap_or(0);
                Some(BondSpec {
                    obligor: seat,
                    beneficiary: proposal.destination,
                    good: bp.good,
                    required_units: bp.required_units,
                    baseline_delivered: baseline,
                    deadline_turn: bp.deadline_turn,
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
        Ok(())
    }

    fn step1_validate_reveals_and_regents(
        &mut self,
        turn: u8,
        n: usize,
    ) -> (Vec<u8>, Vec<u8>, Vec<u8>, Vec<SealedOrderBundle>) {
        let mut non_reveal_seats = Vec::new();
        let mut regent_acted_seats = Vec::new();
        let mut converted_to_ai = Vec::new();
        let mut bundles = vec![SealedOrderBundle::default(); n];

        for s in 0..n as u8 {
            let idx = usize::from(s);
            let did_commit = self.state.turn_commits.contains_key(&s);
            if let Some(rev) = self.state.turn_reveals.get(&s) {
                self.state.seats[idx].consecutive_misses = 0;
                bundles[idx] = rev.bundle.clone();
            } else {
                non_reveal_seats.push(s);
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

    fn step2b_evaluate_bonds_and_expired_escrows(
        &mut self,
        turn: u8,
        broken_promises: &mut Vec<BrokenPromiseRecord>,
    ) -> (Vec<u64>, Vec<u64>) {
        let mut finished = Vec::new();
        let mut cancelled = Vec::new();

        // Evaluate Steward-refereed delivery bonds at deadline (after Step 2 swaps settle).
        let vault_ids: Vec<u64> = self.state.escrows.keys().copied().collect();
        for escrow_id in vault_ids {
            let Some(vault) = self.state.escrows.get(&escrow_id).cloned() else {
                continue;
            };
            if vault.status != EscrowStatus::Locked {
                continue;
            }
            if let Some(bond) = vault.bond
                && bond.deadline_turn <= turn
            {
                let cum_now = self
                    .state
                    .delivery_ledger
                    .get(&(bond.obligor, bond.beneficiary, bond.good))
                    .copied()
                    .unwrap_or(0);
                let delta_delivered = cum_now.saturating_sub(bond.baseline_delivered);
                if delta_delivered >= bond.required_units {
                    if let Some(slot) = self.state.escrows.get_mut(&escrow_id) {
                        slot.status = EscrowStatus::Finished;
                    }
                    let _ =
                        self.adjust_free_balance(vault.sender, AssetId::Cred, vault.amount_cred);
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
                    ob.honor_penalties = ob.honor_penalties.saturating_add(vault.amount_cred);
                    cancelled.push(escrow_id);
                }
                continue;
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

    fn step2_settle_converters(
        &mut self,
        bundles: &[SealedOrderBundle],
    ) -> (usize, usize, Vec<Vec<ConverterOrderPreview>>) {
        let mut total_batches = 0usize;
        let mut clamped_orders = 0usize;
        let mut by_seat = Vec::with_capacity(bundles.len());
        for (s_idx, bundle) in bundles.iter().enumerate() {
            let mut runs_used: BTreeMap<usize, u32> = BTreeMap::new();
            let mut previews = Vec::with_capacity(bundle.converter_runs.len());
            for order in &bundle.converter_runs {
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
                let affordable_by_inputs = recipe
                    .inputs
                    .iter()
                    .map(|&(raw_g, amt)| {
                        if amt <= 0 {
                            0
                        } else {
                            let avail = self.state.seats[s_idx].raw[usize::from(raw_g)].max(0);
                            u32::try_from(avail / amt).unwrap_or(u32::MAX)
                        }
                    })
                    .min()
                    .unwrap_or(0);
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
                    clamped_orders += 1;
                }
                let runs_i64 = i64::from(exec_runs);
                for &(raw_g, amt) in &recipe.inputs {
                    self.state.seats[s_idx].raw[usize::from(raw_g)] -= amt.saturating_mul(runs_i64);
                }
                self.state.seats[s_idx].refined[usize::from(recipe.output_good)] =
                    self.state.seats[s_idx].refined[usize::from(recipe.output_good)]
                        .saturating_add(recipe.output_amount.saturating_mul(runs_i64));
                *runs_used.entry(order.recipe_idx).or_insert(0) += exec_runs;
                total_batches += exec_runs as usize;
            }
            by_seat.push(previews);
        }
        (total_batches, clamped_orders, by_seat)
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

    fn step4_projects_and_yields(
        &mut self,
        turn: u8,
        bundles: &[SealedOrderBundle],
    ) -> (usize, usize, Vec<Vec<ProjectOrderPreview>>) {
        let mut completed = 0usize;
        let mut clamped_orders = 0usize;
        let project_by_id: BTreeMap<u16, crate::types::ProjectRecipe> = self
            .state
            .spec
            .projects
            .iter()
            .map(|p| (p.id, p.clone()))
            .collect();
        let mut by_seat = Vec::with_capacity(bundles.len());

        for (s_idx, bundle) in bundles.iter().enumerate() {
            let mut previews = Vec::with_capacity(bundle.project_deliveries.len());
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
                for &(ref_g, amt) in &recipe.inputs {
                    self.state.seats[s_idx].refined[usize::from(ref_g)] -=
                        amt.saturating_mul(b_i64);
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
            }
            by_seat.push(previews);
        }

        // Mint Turn raw extraction yields with any active SupplyFreeze reduction.
        for s_idx in 0..self.state.seats.len() {
            let base_yield = self.state.spec.seats[s_idx].raw_yield_per_turn;
            let freeze_bps = self
                .state
                .active_shocks
                .iter()
                .filter(|sh| {
                    sh.kind == ShockKind::SupplyFreeze
                        && sh.target_seat == s_idx as u8
                        && sh.expires_after_turn >= turn
                })
                .map(|sh| sh.magnitude_bps)
                .max()
                .unwrap_or(0);
            let actual_yield = base_yield - floor_bps_part(base_yield, freeze_bps);
            self.state.seats[s_idx].raw[s_idx] =
                self.state.seats[s_idx].raw[s_idx].saturating_add(actual_yield);
        }
        (completed, clamped_orders, by_seat)
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
            let paid = avail.min(cfg.upkeep_amount);
            self.state.seats[s_idx].refined[upkeep_g] -= paid;
            let shortfall = cfg.upkeep_amount - paid;
            let mut new_debt = 0;
            if shortfall > 0 {
                let parity = self.state.parities.refined_per_unit[upkeep_g];
                new_debt = ceil_units_parity(shortfall, parity);
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
        let mut rate_by_good = Vec::with_capacity(n);
        for g in 0..n {
            let base = self.state.spec.seats[g].perishable_decay_bps;
            let spike = self
                .state
                .active_shocks
                .iter()
                .filter(|sh| {
                    sh.kind == ShockKind::SpoilageSpike
                        && sh.target_seat == g as u8
                        && sh.expires_after_turn >= turn
                })
                .map(|sh| sh.magnitude_bps)
                .max()
                .unwrap_or(0);
            let headroom = 10_000_i32 - i32::from(base);
            let boost = (headroom * i32::from(spike) + 9_999) / 10_000;
            let eff = (i32::from(base) + boost).min(9_900) as Bps;
            rate_by_good.push(eff);
        }

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

            let (tm, tm_won_vote) = pick_plurality_winner_with_beacon(
                &tm_votes,
                self.state.trade_minister,
                turn_beacon,
                b"trade_minister",
            );
            let (cc, cc_won_vote) = pick_plurality_winner_with_beacon(
                &cc_votes,
                self.state.cartel_chair,
                turn_beacon,
                b"cartel_chair",
            );
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
                && let Some(win_cc_platform) = self.state.council_platforms.get(&cc).cloned()
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
                    if pledge.promisor == cc && revoked_pairs.contains(&(domain_id, beneficiary)) {
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
        for shock in &self.state.spec.shocks {
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
    ) -> (Vec<ShockForecastEvent>, Vec<ActiveShock>, bool) {
        let mut public_forecasts = Vec::new();
        let mut newly_activated = Vec::new();
        for shock in &self.state.spec.shocks {
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
        self.state
            .active_shocks
            .retain(|s| s.expires_after_turn > turn);

        for seat in &mut self.state.seats {
            if seat.wash_frozen_until_turn <= turn {
                seat.wash_frozen_until_turn = 0;
            }
        }

        let season_closed = if turn >= 28 {
            self.state.last_closing_roll_bps = None;
            true
        } else if (24..=27).contains(&turn) {
            let roll = beacon_roll_bps(turn_beacon);
            self.state.last_closing_roll_bps = Some(roll);
            roll < self.state.spec.closing_hazard_bps[usize::from(turn - 24)]
        } else {
            self.state.last_closing_roll_bps = None;
            false
        };
        (public_forecasts, newly_activated, season_closed)
    }
}

fn pick_plurality_winner_with_beacon(
    votes: &[u32],
    incumbent: u8,
    turn_beacon: [u8; 32],
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
    (break_tie_with_beacon(turn_beacon, domain_tag, &tied), true)
}

#[cfg(test)]
mod tests {
    use super::*;
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
