use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use serde::{Deserialize, Serialize};

use crate::commit::{
    AtomicSwapProposal, CommitmentHash, ContractDeliveryOrder, CouncilPlatform, DrandSchedule,
    DualEscrowProposal, EscrowFinishUnlock, GradedMandateCard, IntentPledge, PrivateMandate,
    SealedOrderBundle,
};
use crate::contracts::{
    ActiveCharter, CartelDomain, CredOffer, CreditCheck, DualEscrowVault, FillRecord, MarketAction,
    PendingWashFreeze, TokenEscrowVault, WashFreezeSettlement,
};
use crate::engine_types::{
    BrokenPromiseRecord, ConverterOrderPreview, EscrowOrderPreview, ProjectOrderPreview, SeatState,
    SeatUpkeepPreview, ShockForecastEvent, TurnPhase,
};
use crate::predicates_types::SeatStatePredicates;
use crate::scoring_types::{FundamentalParities, ParitySpecSource};
use crate::serde_helpers::{delivery_ledger_serde, opt_hex32};
use crate::types::{
    ActiveShock, AssetId, Bps, CLOSING_ROLL_WINDOW_LEN, CharterRole, ProjectRecipe, SeatConfig,
    ShockKind, TableId, TableMode, Units,
};

/// Deterministic non-zero sentinel replacing private `dealt_mandate_commits` inside sanitized
/// preview engines (`C4`).
pub const DEALT_MANDATE_COMMIT_SENTINEL: CommitmentHash = CommitmentHash([0xAA; 32]);

/// Public neutral seed (`0`) used when previews do not take a caller-supplied beacon.
pub const PREVIEW_NEUTRAL_BEACON_SEED: u64 = 0;

/// Caveat attached to [`WhatIfPreview::close_mark_caveat`] explaining the Steps 0–7 boundary.
pub const HYPOTHETICAL_CLOSE_MARK_CAVEAT: &str =
    "Hypothetical close mark after Steps 0-7; assumes open season and no unforecast shock";

/// Seat-safe 30-field allow-list projection of `SweepReceipt`, unconditionally dropping
/// `newly_drawn_shocks` and `post_state_digest` (`A1`, `C4`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicSweepReceipt {
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
    pub elected_officers: Option<(u8, Option<u8>)>,
    /// Awarded charter `(role, winner_seat)` pairs if `turn % 6 == 0`.
    pub charters_awarded: Vec<(CharterRole, u8)>,
    /// Publicly forecasted Horizon Shocks announced in Step 8 (`forecast_turn`, zero magnitude).
    pub public_forecasts: Vec<ShockForecastEvent>,
    /// Newly activated realized shocks enacted for `turn` (`realize_turn == turn`).
    pub newly_activated_shocks: Vec<ActiveShock>,
    /// Total voluntary `CRED` auto-debt repaid in Step 2.
    pub cred_debt_repaid: Units,
    /// Total `CRED` burned by Syndicate Charter auctions in Step 7.
    pub charter_cred_burned: Units,
    /// Total `CRED` spent by seats on Authority `TownImport` fallback orders this turn.
    pub town_import_cred_spent: Units,
    /// Total working-capital `CRED` stipend minted for Tier-3 Syndicate Project deliveries.
    pub works_stipend_cred_minted: Units,
    /// Count of Phase-1 DEX batch trades matched at Step 0 Sweep.
    pub batch_trades_cleared: usize,
    /// Every settlement record (`FillRecord`) logged during `turn`.
    pub fills: Vec<FillRecord>,
    /// Customs Inspector `FlagWashFreeze` bond settlements finalized on `turn`.
    pub wash_freeze_settlements: Vec<WashFreezeSettlement>,
    /// `true` if the season closed on this Sweep.
    pub season_closed: bool,
    /// Per-seat PBRS shaped returns `F_i = gamma * Psi_{t+1} - Psi_t`.
    pub pbrs_deltas: Vec<i64>,
}

impl PublicSweepReceipt {
    /// Serializes this 30-field public receipt to compact canonical JSON for host comparison.
    pub fn to_json_string(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

/// Private Horizon Shock hint visible solely to the active `OracleAuditor` seat while
/// `hint_turn <= turn < forecast_turn`; `magnitude_bps` is always omitted before realize.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditorShockHint {
    /// Turn when the private hint became visible to `OracleAuditor`.
    pub hint_turn: u8,
    /// Turn when the public forecast posts at Sweep Step 8.
    pub forecast_turn: u8,
    /// Turn when the realized shock activates (`realize_turn`).
    pub realize_turn: u8,
    /// Target commodity seat (`0..seat_count`).
    pub target_seat: u8,
    /// Shock category.
    pub kind: ShockKind,
}

/// Public subset of `TableSpec` omitting pre-seeded `shocks` and private `dealt_mandate_commits`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicTableSpec {
    /// Unique table identifier.
    pub table_id: TableId,
    /// Execution mode (`Game`, `Eval`, or `Club`).
    pub mode: TableMode,
    /// Number of seats (`4`, `5`, or `6`).
    pub seat_count: u8,
    /// Per-seat economic definitions.
    ///
    /// Note: [`SeatConfig::regent`] (`Option<RegentDoctrine>`) is intentionally public table
    /// configuration visible to every seat so all seats see upfront which seats carry Standing
    /// Regent fallback settings (`C4`).
    pub seats: Vec<SeatConfig>,
    /// Available Tier-3 Syndicate Project sinks.
    pub projects: Vec<ProjectRecipe>,
    /// Stochastic season closing hazards (in bps) evaluated on Turns `24..=31`.
    pub closing_hazard_bps: [Bps; CLOSING_ROLL_WINDOW_LEN],
    /// Public graded Turn-1 mandate deck.
    pub mandate_deck: Vec<GradedMandateCard>,
    /// Optional drand `quicknet` close schedule for `Game` tables.
    #[serde(default)]
    pub drand_schedule: Option<DrandSchedule>,
    /// Optional `ristretto255` VRF public key (`PK_eval`) for `Eval` / `Club` tables.
    #[serde(default, with = "opt_hex32")]
    pub eval_vrf_pubkey: Option<[u8; 32]>,
    /// Town-import premium multiplier in basis points.
    pub town_import_premium_bps: Bps,
    /// Works stipend (`CRED` per `1_000` refined units delivered to Works).
    pub works_stipend_per_unit_cred: Units,
    /// Mandate-point conversion factor `alpha` in basis points.
    pub mandate_alpha_bps: Bps,
    /// Phase-1 cancel-lock fraction in basis points (`0` disables the lock).
    pub cancel_lock_bps: Bps,
    /// Minimum per-direction commodity flow triggering a confirmed wash-freeze audit.
    pub wash_round_trip_min_units: Units,
    /// Challenger bond posted upfront by `CustomsInspector` on `FlagWashFreeze`.
    pub wash_freeze_bond_cred: Units,
    /// Bounty transferred from confirmed wash target free `CRED` to `CustomsInspector`.
    pub wash_freeze_bounty_cred: Units,
}

impl ParitySpecSource for PublicTableSpec {
    fn spec_seat_count(&self) -> u8 {
        self.seat_count
    }

    fn spec_seats(&self) -> &[SeatConfig] {
        &self.seats
    }
}

/// Sole seat-safe read boundary constructed from `TurnEngine::observe_seat` or
/// `ClubHost::observe_seat` (`A1`, `C4`).
///
/// Carries only public table state, `seat`'s own `OracleAuditor` hints (without `magnitude_bps`),
/// realized active shocks (`turn >= realize_turn`), announced forecasts (`forecast_turn < turn`),
/// static [`SeatStatePredicates`], [`PublicSweepReceipt`] history, and optional `own_mandate`
/// (populating solely `seat`'s own mandate when emitted by a host). Never carries raw
/// `SweepReceipt`s, unforecast `newly_drawn_shocks`, shock magnitudes before realize,
/// `dealt_mandate_commits`, peer commits/reveals, or `post_state_digest`.
///
/// Intentionally omits `serde::Deserialize` (`B1`): neither save files nor replay logs deserialize
/// `SeatObservation`, so neither seat code nor host code can forge `own_mandate` via a serde
/// round-trip (pinned by the `compile_fail,E0277` doctest on `league_core::ClubHost::observe_seat`,
/// since `league-seat-types` disables doctests via `[lib] doctest = false`).
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct SeatObservation {
    /// Observing seat index (`0..seat_count`).
    pub seat: u8,
    /// Table identifier.
    pub table_id: TableId,
    /// Current turn index (`1..=32`).
    pub turn: u8,
    /// Current turn phase or terminal close state.
    pub phase: TurnPhase,
    /// Whether Phase-1 cancel lock is currently engaged.
    pub cancel_locked: bool,
    /// Current Trade Minister seat index.
    pub trade_minister: u8,
    /// Current Cartel Chair seat index (`None` if vacant).
    pub cartel_chair: Option<u8>,
    /// Public table rule parameters.
    pub spec: PublicTableSpec,
    /// Precomputed fundamental shadow parities.
    pub parities: FundamentalParities,
    /// Per-seat public balances and reputations.
    pub seats: Vec<SeatState>,
    /// Resting `CRED`-priced DEX limit orders keyed by canonical `offer_id`.
    pub offers: BTreeMap<u64, CredOffer>,
    /// Track-2 deferred checks keyed by canonical `check_id`.
    pub checks: BTreeMap<u64, CreditCheck>,
    /// Track-1 `TokenEscrow` vaults keyed by canonical `escrow_id`.
    pub escrows: BTreeMap<u64, TokenEscrowVault>,
    /// Co-signed dual delivery-versus-payment escrows keyed by shared `escrow_id`.
    pub dual_escrows: BTreeMap<u64, DualEscrowVault>,
    /// Recorded Track-2 structured diplomatic pledges keyed by `pledge_id`.
    pub pledges: BTreeMap<u64, IntentPledge>,
    /// Pledge IDs already resolved (honored or flagged broken).
    pub audited_pledges: BTreeSet<u64>,
    /// Permissioned cartel trade domains.
    pub domains: Vec<CartelDomain>,
    /// Public council platforms filed ahead of Turn-`4n` elections.
    pub council_platforms: BTreeMap<u8, CouncilPlatform>,
    /// Active rotating Syndicate Charters.
    pub charters: Vec<ActiveCharter>,
    /// Active realized Horizon Shocks (`turn >= realize_turn`; magnitude is public from realize).
    pub active_shocks: Vec<ActiveShock>,
    /// Publicly announced Horizon Shock forecasts (`forecast_turn <= last_swept_turn`, no magnitude).
    pub public_forecasts: Vec<ShockForecastEvent>,
    /// Private `OracleAuditor` shock hints for `seat` (`hint_turn <= turn < forecast_turn`, no magnitude).
    pub auditor_hints: Vec<AuditorShockHint>,
    /// Monotonic next offer ID.
    pub next_offer_id: u64,
    /// Monotonic next check ID.
    pub next_check_id: u64,
    /// Monotonic next escrow ID.
    pub next_escrow_id: u64,
    /// Cumulative delivered volume `(sender, receiver, asset)`.
    #[serde(serialize_with = "delivery_ledger_serde::serialize")]
    pub delivery_ledger: BTreeMap<(u8, u8, AssetId), Units>,
    /// Previous step PBRS potentials `Psi_i(s_t)`.
    pub last_potentials: Vec<Units>,
    /// Accumulator of `CRED` spent on Phase-1 `TownImport` orders during the current turn.
    pub pending_town_import_cred_spent: Units,
    /// Append-only per-turn settlement log across all turns.
    pub fill_log: Vec<FillRecord>,
    /// Unsettled bonded `FlagWashFreeze` challenges for the current turn.
    pub pending_wash_freezes: Vec<PendingWashFreeze>,
    /// Optional `ristretto255` VRF public key (`PK_eval`) committed on the table.
    #[serde(serialize_with = "opt_hex32::serialize")]
    pub eval_vrf_pubkey: Option<[u8; 32]>,
    /// Static start-of-observation diagnostics for `seat`.
    pub baseline_predicates: SeatStatePredicates,
    /// Seat-safe 30-field historical Sweep receipts through `last_swept_turn`.
    pub public_receipts: Vec<PublicSweepReceipt>,
    /// Dealt Turn-1 [`PrivateMandate`] belonging solely to `seat`, populated exclusively by
    /// `ClubHost::observe_seat` (never carries reveal salts or rival seats' mandates).
    #[serde(skip_serializing_if = "Option::is_none")]
    own_mandate: Option<PrivateMandate>,
}

/// Public non-mandate fields required to assemble a [`SeatObservation`] without exposing
/// `own_mandate` as a writable field outside trusted host construction.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatObservationPublicParts {
    /// Observing seat index (`0..seat_count`).
    pub seat: u8,
    /// Table identifier.
    pub table_id: TableId,
    /// Current turn index (`1..=32`).
    pub turn: u8,
    /// Current turn phase or terminal close state.
    pub phase: TurnPhase,
    /// Whether Phase-1 cancel lock is currently engaged.
    pub cancel_locked: bool,
    /// Current Trade Minister seat index.
    pub trade_minister: u8,
    /// Current Cartel Chair seat index (`None` if vacant).
    pub cartel_chair: Option<u8>,
    /// Public table rule parameters.
    pub spec: PublicTableSpec,
    /// Precomputed fundamental shadow parities.
    pub parities: FundamentalParities,
    /// Per-seat public balances and reputations.
    pub seats: Vec<SeatState>,
    /// Resting `CRED`-priced DEX limit orders keyed by canonical `offer_id`.
    pub offers: BTreeMap<u64, CredOffer>,
    /// Track-2 deferred checks keyed by canonical `check_id`.
    pub checks: BTreeMap<u64, CreditCheck>,
    /// Track-1 `TokenEscrow` vaults keyed by canonical `escrow_id`.
    pub escrows: BTreeMap<u64, TokenEscrowVault>,
    /// Co-signed dual delivery-versus-payment escrows keyed by shared `escrow_id`.
    pub dual_escrows: BTreeMap<u64, DualEscrowVault>,
    /// Recorded Track-2 structured diplomatic pledges keyed by `pledge_id`.
    pub pledges: BTreeMap<u64, IntentPledge>,
    /// Pledge IDs already resolved (honored or flagged broken).
    pub audited_pledges: BTreeSet<u64>,
    /// Permissioned cartel trade domains.
    pub domains: Vec<CartelDomain>,
    /// Public council platforms filed ahead of Turn-`4n` elections.
    pub council_platforms: BTreeMap<u8, CouncilPlatform>,
    /// Active rotating Syndicate Charters.
    pub charters: Vec<ActiveCharter>,
    /// Active realized Horizon Shocks (`turn >= realize_turn`; magnitude is public from realize).
    pub active_shocks: Vec<ActiveShock>,
    /// Publicly announced Horizon Shock forecasts (`forecast_turn <= last_swept_turn`, no magnitude).
    pub public_forecasts: Vec<ShockForecastEvent>,
    /// Private `OracleAuditor` shock hints for `seat` (`hint_turn <= turn < forecast_turn`, no magnitude).
    pub auditor_hints: Vec<AuditorShockHint>,
    /// Monotonic next offer ID.
    pub next_offer_id: u64,
    /// Monotonic next check ID.
    pub next_check_id: u64,
    /// Monotonic next escrow ID.
    pub next_escrow_id: u64,
    /// Cumulative delivered volume `(sender, receiver, asset)`.
    pub delivery_ledger: BTreeMap<(u8, u8, AssetId), Units>,
    /// Previous step PBRS potentials `Psi_i(s_t)`.
    pub last_potentials: Vec<Units>,
    /// Accumulator of `CRED` spent on Phase-1 `TownImport` orders during the current turn.
    pub pending_town_import_cred_spent: Units,
    /// Append-only per-turn settlement log across all turns.
    pub fill_log: Vec<FillRecord>,
    /// Unsettled bonded `FlagWashFreeze` challenges for the current turn.
    pub pending_wash_freezes: Vec<PendingWashFreeze>,
    /// Optional `ristretto255` VRF public key (`PK_eval`) committed on the table.
    pub eval_vrf_pubkey: Option<[u8; 32]>,
    /// Static start-of-observation diagnostics for `seat`.
    pub baseline_predicates: SeatStatePredicates,
    /// Seat-safe 30-field historical Sweep receipts through `last_swept_turn`.
    pub public_receipts: Vec<PublicSweepReceipt>,
}

impl SeatObservation {
    /// Assembles a mandate-free [`SeatObservation`] (`own_mandate == None`) for pure UI/public
    /// test construction without permitting any caller to attach a [`PrivateMandate`].
    #[doc(hidden)]
    #[must_use]
    pub fn unmandated(parts: SeatObservationPublicParts) -> Self {
        Self::from_parts_inner(parts, None)
    }

    /// Assembles a [`SeatObservation`] with optional `own_mandate`; guarded so only
    /// `league-core::observation` (acting solely for `ClubHost::observe_seat`) may invoke it.
    #[doc(hidden)]
    #[must_use]
    pub fn from_host_parts(
        parts: SeatObservationPublicParts,
        own_mandate: Option<PrivateMandate>,
    ) -> Self {
        Self::from_parts_inner(parts, own_mandate)
    }

    fn from_parts_inner(
        parts: SeatObservationPublicParts,
        own_mandate: Option<PrivateMandate>,
    ) -> Self {
        Self {
            seat: parts.seat,
            table_id: parts.table_id,
            turn: parts.turn,
            phase: parts.phase,
            cancel_locked: parts.cancel_locked,
            trade_minister: parts.trade_minister,
            cartel_chair: parts.cartel_chair,
            spec: parts.spec,
            parities: parts.parities,
            seats: parts.seats,
            offers: parts.offers,
            checks: parts.checks,
            escrows: parts.escrows,
            dual_escrows: parts.dual_escrows,
            pledges: parts.pledges,
            audited_pledges: parts.audited_pledges,
            domains: parts.domains,
            council_platforms: parts.council_platforms,
            charters: parts.charters,
            active_shocks: parts.active_shocks,
            public_forecasts: parts.public_forecasts,
            auditor_hints: parts.auditor_hints,
            next_offer_id: parts.next_offer_id,
            next_check_id: parts.next_check_id,
            next_escrow_id: parts.next_escrow_id,
            delivery_ledger: parts.delivery_ledger,
            last_potentials: parts.last_potentials,
            pending_town_import_cred_spent: parts.pending_town_import_cred_spent,
            fill_log: parts.fill_log,
            pending_wash_freezes: parts.pending_wash_freezes,
            eval_vrf_pubkey: parts.eval_vrf_pubkey,
            baseline_predicates: parts.baseline_predicates,
            public_receipts: parts.public_receipts,
            own_mandate,
        }
    }

    /// Returns the seat-safe `OracleAuditor` private shock hints (`hint_turn <= turn < forecast_turn`)
    /// for this observing seat, with `magnitude_bps` redacted until `realize_turn` (`B1`).
    ///
    /// Realized shocks carry public `magnitude_bps` in [`Self::active_shocks`] and
    /// [`PublicSweepReceipt::newly_activated_shocks`] from `realize_turn` onward.
    #[must_use]
    pub fn auditor_hints(&self) -> &[AuditorShockHint] {
        &self.auditor_hints
    }

    /// Returns this seat's own dealt Turn-1 [`PrivateMandate`] when populated by
    /// `ClubHost::observe_seat` (never carries reveal salts or rival seats' mandates).
    #[must_use]
    pub const fn own_mandate(&self) -> Option<&PrivateMandate> {
        self.own_mandate.as_ref()
    }

    /// Exports a sanitized [`PublicPreviewState`] containing strictly public Steps 0–7 simulation
    /// state, omitting Auditor hints, forecasts, receipts, and all hidden secrets.
    #[must_use]
    pub fn to_public_preview_state(&self) -> PublicPreviewState {
        PublicPreviewState {
            seat: self.seat,
            table_id: self.table_id,
            turn: self.turn,
            phase: self.phase,
            cancel_locked: self.cancel_locked,
            trade_minister: self.trade_minister,
            cartel_chair: self.cartel_chair,
            spec: self.spec.clone(),
            parities: self.parities.clone(),
            seats: self.seats.clone(),
            offers: self.offers.clone(),
            checks: self.checks.clone(),
            escrows: self.escrows.clone(),
            dual_escrows: self.dual_escrows.clone(),
            pledges: self.pledges.clone(),
            audited_pledges: self.audited_pledges.clone(),
            domains: self.domains.clone(),
            council_platforms: self.council_platforms.clone(),
            charters: self.charters.clone(),
            active_shocks: self.active_shocks.clone(),
            next_offer_id: self.next_offer_id,
            next_check_id: self.next_check_id,
            next_escrow_id: self.next_escrow_id,
            delivery_ledger: self.delivery_ledger.clone(),
            last_potentials: self.last_potentials.clone(),
            pending_town_import_cred_spent: self.pending_town_import_cred_spent,
            fill_log: self.fill_log.clone(),
            pending_wash_freezes: self.pending_wash_freezes.clone(),
            baseline_predicates: self.baseline_predicates.clone(),
        }
    }
}

impl fmt::Debug for SeatObservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SeatObservation")
            .field("seat", &self.seat)
            .field("table_id", &self.table_id)
            .field("turn", &self.turn)
            .field("phase", &self.phase)
            .field("cancel_locked", &self.cancel_locked)
            .field(
                "own_mandate",
                &self.own_mandate.as_ref().map(|_| "<redacted>"),
            )
            .field("auditor_hints_len", &self.auditor_hints.len())
            .finish()
    }
}

/// Sanitized public snapshot exported from [`SeatObservation::to_public_preview_state`] for
/// side-effect-free Steps 0–7 simulations (`preview_what_if` / `preview_seat_bundle`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicPreviewState {
    /// Observing seat index.
    pub seat: u8,
    /// Table identifier.
    pub table_id: TableId,
    /// Current turn index (`1..=32`).
    pub turn: u8,
    /// Current turn phase.
    pub phase: TurnPhase,
    /// Whether Phase-1 cancel lock is active.
    pub cancel_locked: bool,
    /// Current Trade Minister seat index.
    pub trade_minister: u8,
    /// Current Cartel Chair seat index.
    pub cartel_chair: Option<u8>,
    /// Public table specification.
    pub spec: PublicTableSpec,
    /// Precomputed fundamental shadow parities.
    pub parities: FundamentalParities,
    /// Per-seat public balances and reputations.
    pub seats: Vec<SeatState>,
    /// Resting DEX limit orders.
    pub offers: BTreeMap<u64, CredOffer>,
    /// Track-2 deferred checks.
    pub checks: BTreeMap<u64, CreditCheck>,
    /// Track-1 `TokenEscrow` vaults.
    pub escrows: BTreeMap<u64, TokenEscrowVault>,
    /// Co-signed dual escrows.
    pub dual_escrows: BTreeMap<u64, DualEscrowVault>,
    /// Diplomatic intent pledges.
    pub pledges: BTreeMap<u64, IntentPledge>,
    /// Resolved pledge IDs.
    pub audited_pledges: BTreeSet<u64>,
    /// Cartel domains.
    pub domains: Vec<CartelDomain>,
    /// Public council platforms.
    pub council_platforms: BTreeMap<u8, CouncilPlatform>,
    /// Active Syndicate Charters.
    pub charters: Vec<ActiveCharter>,
    /// Realized active Horizon Shocks (`turn >= realize_turn`).
    pub active_shocks: Vec<ActiveShock>,
    /// Monotonic next offer ID.
    pub next_offer_id: u64,
    /// Monotonic next check ID.
    pub next_check_id: u64,
    /// Monotonic next escrow ID.
    pub next_escrow_id: u64,
    /// Cumulative delivered volume `(sender, receiver, asset)`.
    #[serde(with = "delivery_ledger_serde")]
    pub delivery_ledger: BTreeMap<(u8, u8, AssetId), Units>,
    /// Previous step PBRS potentials.
    pub last_potentials: Vec<Units>,
    /// Pending `TownImport` `CRED` spent this turn.
    pub pending_town_import_cred_spent: Units,
    /// Settlement history.
    pub fill_log: Vec<FillRecord>,
    /// Pending wash-freeze challenges.
    pub pending_wash_freezes: Vec<PendingWashFreeze>,
    /// Baseline start-of-observation predicates.
    pub baseline_predicates: SeatStatePredicates,
}

/// Tie-break policy for `preview_what_if`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewTieMode {
    /// Deterministic neutral preview beacon.
    #[default]
    NeutralSeed,
    /// Evaluates candidate deterministic beacons and selects the tie outcome yielding the lowest
    /// post-Sweep potential for the asking seat.
    PessimisticForSeat,
}

/// Caller-supplied hypothetical counterparty assumptions for [`HumanWhatIfRequest`].
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CounterpartyAssumptions {
    /// Resting offer IDs assumed cancelled before Phase-1 execution (ignored when `cancel_locked`).
    #[serde(default)]
    pub assumed_cancelled_offer_ids: Vec<u64>,
    /// Hypothetical peer Phase-1 [`MarketAction`]s applied during `TalkAndMarket`.
    #[serde(default)]
    pub assumed_peer_market_actions: Vec<MarketAction>,
    /// Atomic swaps assumed co-signed by the counterparty seat.
    #[serde(default)]
    pub assumed_cosigned_swaps: Vec<AtomicSwapProposal>,
    /// Dual escrows assumed co-signed by the counterparty seat.
    #[serde(default)]
    pub assumed_cosigned_dual_escrows: Vec<DualEscrowProposal>,
    /// Peer contract deliveries `(peer_seat, order)` assumed revealed in Phase 2.
    #[serde(default)]
    pub assumed_peer_deliveries: Vec<(u8, ContractDeliveryOrder)>,
    /// Peer escrow unlocks `(peer_seat, unlock)` assumed revealed in Phase 2.
    #[serde(default)]
    pub assumed_peer_escrow_finishes: Vec<(u8, EscrowFinishUnlock)>,
    /// Open check IDs drawn by peers assumed to bounce at Step 3.
    #[serde(default)]
    pub assumed_bounced_check_ids: Vec<u64>,
    /// Explicit per-peer Phase-2 bundles overriding default empty peer bundles.
    #[serde(default)]
    pub assumed_peer_bundles: BTreeMap<u8, SealedOrderBundle>,
}

impl fmt::Debug for CounterpartyAssumptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CounterpartyAssumptions(<redacted>)")
    }
}

/// Human What-If Sandbox request executed by `preview_what_if` over [`PublicPreviewState`].
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanWhatIfRequest {
    /// Asking seat index (`0..seat_count`).
    pub seat: u8,
    /// Phase-1 [`MarketAction`] sequence to simulate.
    pub market_actions: Vec<MarketAction>,
    /// Optional index into `market_actions` at which cancel lock engages during simulation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancel_lock_at: Option<usize>,
    /// Asking seat's Phase-2 [`SealedOrderBundle`].
    pub bundle: SealedOrderBundle,
    /// Tie-break policy (`NeutralSeed` or `PessimisticForSeat`).
    pub tie_mode: PreviewTieMode,
    /// Counterparty action/bundle assumptions.
    pub assumptions: CounterpartyAssumptions,
}

impl fmt::Debug for HumanWhatIfRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HumanWhatIfRequest")
            .field("seat", &self.seat)
            .field("tie_mode", &self.tie_mode)
            .field("bundle", &"<redacted>")
            .field("assumptions", &"<redacted>")
            .finish()
    }
}

/// AI-seat preview request executed by `preview_seat_bundle` (`A2`, zero assumptions).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiPreviewRequest {
    /// Asking AI seat index (`0..seat_count`).
    pub seat: u8,
    /// Own Phase-1 [`MarketAction`]s (every action must act for `seat`).
    pub market_actions: Vec<MarketAction>,
    /// Own Phase-2 [`SealedOrderBundle`].
    pub bundle: SealedOrderBundle,
}

impl fmt::Debug for AiPreviewRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AiPreviewRequest")
            .field("seat", &self.seat)
            .field("market_actions_len", &self.market_actions.len())
            .field("bundle", &"<redacted>")
            .finish()
    }
}

/// Refused Phase-1 [`MarketAction`] entry inside [`WhatIfPreview::refused_market_actions`].
///
/// # Secret-Scrubber Exemption (`B2`)
/// `message` is the sole player-facing string emitted from `league-core` simulation that passes
/// through [`league_error::scrub_secret_text`] (`engine.rs`), because it carries a runtime
/// [`crate::EngineError`] validation refusal string rather than authored display prose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefusedActionPreview {
    /// Zero-based index inside `market_actions`.
    pub action_index: usize,
    /// Stable [`crate::EngineError::code`] identifier.
    pub error_code: String,
    /// Scrubbed human-readable refusal error string (exempt from display-prose ban as an error string).
    pub message: String,
}

/// Outcome of an [`AtomicSwapProposal`] simulated in [`WhatIfPreview`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicSwapOutcome {
    /// Counterparty co-signed and both parties satisfied domain/balance checks.
    Settled,
    /// Counterparty did not co-sign, one party lacked balance, or wash/domain checks blocked it.
    UnmatchedOrInsufficientBalance,
}

/// Preview breakdown of one [`AtomicSwapProposal`] inside [`WhatIfPreview`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicSwapOrderPreview {
    /// Swap identifier from the bundle.
    pub swap_id: u64,
    /// Maker seat index.
    pub maker: u8,
    /// Taker seat index.
    pub taker: u8,
    /// Predicted Step-2 settlement status.
    pub outcome: PublicSwapOutcome,
}

/// Outcome of a `CheckCash` request simulated in [`WhatIfPreview`] (omitting `"note"` tokens).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicCheckOutcome {
    /// Check cleared on Step 3 and credited the payee.
    Cashed,
    /// Drawer lacked `deliver_min`; check bounced and downgraded drawer credit tier.
    Bounced,
    /// Check was missing, closed, premature, expired, or presented by non-payee.
    IneligibleOrIgnored,
}

/// Preview breakdown of one `check_id` inside [`WhatIfPreview`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicCheckOrderPreview {
    /// Check ID presented.
    pub check_id: u64,
    /// Predicted Step-3 clearing status.
    pub outcome: PublicCheckOutcome,
}

/// Per-seat Step-6 perishable spoilage summary inside [`WhatIfPreview`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatSpoilagePreview {
    /// Perishable raw commodity units removed from `seat` at Step 6 `Clawback`.
    pub perishable_spoiled_units: Units,
}

/// Deterministic Steps 0–7 sweep summary matching live execution (`B3`), explicitly excluding
/// Step-8 stochastic/digest fields (`newly_drawn_shocks`, `public_forecasts`,
/// `newly_activated_shocks`, `season_closed`, `pbrs_deltas`, `post_state_digest`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step0To7SweepSummary {
    /// Turn index simulated.
    pub turn: u8,
    /// Seats that missed commit or reveal in Step 1.
    pub non_reveal_seats: Vec<u8>,
    /// Absent human seats for which the Standing Regent acted.
    pub regent_acted_seats: Vec<u8>,
    /// Seats converted from `Human` to `AiSyndicate` after 3 misses.
    pub converted_to_ai: Vec<u8>,
    /// Escrow IDs unlocked cleanly in Step 2.
    pub escrows_finished: Vec<u64>,
    /// Escrow IDs forfeited or refunded in Step 2.
    pub escrows_cancelled_or_forfeited: Vec<u64>,
    /// Count of co-signed atomic swaps settled in Step 2.
    pub atomic_swaps_settled: usize,
    /// Count of valid converter batches executed in Step 2.
    pub converter_batches_run: usize,
    /// Count of converter orders clamped in Step 2.
    pub converter_orders_clamped: usize,
    /// Check IDs cleared via `CheckCash` in Step 3.
    pub checks_cashed: Vec<u64>,
    /// Check IDs that bounced in Step 3.
    pub checks_bounced: Vec<u64>,
    /// Seats evicted from cartel domains due to hitting `CreditTier::D`.
    pub cartel_evictions: Vec<u8>,
    /// Count of Tier-3 Syndicate Project batches burned in Step 4.
    pub projects_completed: usize,
    /// Count of project delivery orders clamped in Step 4.
    pub project_orders_clamped: usize,
    /// Total `CRED` auto-debt newly issued for upkeep shortfalls in Step 5.
    pub total_upkeep_shortfall_debt: Units,
    /// Total perishable raw units removed via `Clawback` spoilage in Step 6.
    pub total_spoilage_clawed_back: Units,
    /// Diplomatic honor violations recorded during Step 7 / Step 2 audits.
    pub broken_promises: Vec<BrokenPromiseRecord>,
    /// Elected `(trade_minister, cartel_chair)` if `turn % 4 == 0`.
    pub elected_officers: Option<(u8, Option<u8>)>,
    /// Awarded charter `(role, winner_seat)` pairs if `turn % 6 == 0`.
    pub charters_awarded: Vec<(CharterRole, u8)>,
    /// Total voluntary `CRED` auto-debt repaid in Step 2.
    pub cred_debt_repaid: Units,
    /// Total `CRED` burned by Syndicate Charter auctions in Step 7.
    pub charter_cred_burned: Units,
    /// Total `CRED` spent on Authority `TownImport` orders this turn.
    pub town_import_cred_spent: Units,
    /// Total working-capital `CRED` stipend minted for project deliveries.
    pub works_stipend_cred_minted: Units,
    /// Count of Phase-1 DEX batch trades matched at Step 0.
    pub batch_trades_cleared: usize,
    /// Step 0–7 settlement fills logged during `turn`.
    pub fills: Vec<FillRecord>,
    /// Customs Inspector `FlagWashFreeze` bond settlements finalized on `turn`.
    pub wash_freeze_settlements: Vec<WashFreezeSettlement>,
}

impl Step0To7SweepSummary {
    /// Projects a [`PublicSweepReceipt`] onto Steps 0–7 fields only.
    #[must_use]
    pub fn from_public_sweep_receipt(receipt: &PublicSweepReceipt) -> Self {
        Self {
            turn: receipt.turn,
            non_reveal_seats: receipt.non_reveal_seats.clone(),
            regent_acted_seats: receipt.regent_acted_seats.clone(),
            converted_to_ai: receipt.converted_to_ai.clone(),
            escrows_finished: receipt.escrows_finished.clone(),
            escrows_cancelled_or_forfeited: receipt.escrows_cancelled_or_forfeited.clone(),
            atomic_swaps_settled: receipt.atomic_swaps_settled,
            converter_batches_run: receipt.converter_batches_run,
            converter_orders_clamped: receipt.converter_orders_clamped,
            checks_cashed: receipt.checks_cashed.clone(),
            checks_bounced: receipt.checks_bounced.clone(),
            cartel_evictions: receipt.cartel_evictions.clone(),
            projects_completed: receipt.projects_completed,
            project_orders_clamped: receipt.project_orders_clamped,
            total_upkeep_shortfall_debt: receipt.total_upkeep_shortfall_debt,
            total_spoilage_clawed_back: receipt.total_spoilage_clawed_back,
            broken_promises: receipt.broken_promises.clone(),
            elected_officers: receipt.elected_officers,
            charters_awarded: receipt.charters_awarded.clone(),
            cred_debt_repaid: receipt.cred_debt_repaid,
            charter_cred_burned: receipt.charter_cred_burned,
            town_import_cred_spent: receipt.town_import_cred_spent,
            works_stipend_cred_minted: receipt.works_stipend_cred_minted,
            batch_trades_cleared: receipt.batch_trades_cleared,
            fills: receipt.fills.clone(),
            wash_freeze_settlements: receipt.wash_freeze_settlements.clone(),
        }
    }
}

/// Scrubbed Steps 0–7 preview response returned by `preview_what_if`,
/// `preview_what_if_with_beacon`, and `preview_seat_bundle`.
///
/// Redacts detailed preview state and order outcomes in `Debug`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WhatIfPreview {
    /// Previewed seat index.
    pub seat: u8,
    /// Turn simulated.
    pub turn: u8,
    /// Baseline start-of-observation predicates before simulated actions.
    pub baseline_predicates: SeatStatePredicates,
    /// Seat holdings and credit state before simulating `market_actions`.
    pub pre_seat_state: SeatState,
    /// Seat holdings after simulated `market_actions` and before Steps 0–7.
    pub post_market_seat_state: SeatState,
    /// Seat holdings and credit state after Steps 0–7 and deterministic Step-8 cleanup.
    pub post_sweep_seat_state: SeatState,
    /// Predicates evaluated on post-Steps-0–7 state.
    pub post_sweep_predicates: SeatStatePredicates,
    /// Caveat explaining that close marks assume open season and no unforecast shock.
    pub close_mark_caveat: String,
    /// Phase-1 market actions refused by engine validation.
    pub refused_market_actions: Vec<RefusedActionPreview>,
    /// Converter execution breakdown.
    pub converter_orders: Vec<ConverterOrderPreview>,
    /// Project delivery execution breakdown.
    pub project_orders: Vec<ProjectOrderPreview>,
    /// Atomic swap settlement outcomes.
    pub swap_orders: Vec<PublicSwapOrderPreview>,
    /// Check clearing outcomes.
    pub check_orders: Vec<PublicCheckOrderPreview>,
    /// Escrow unlock outcomes.
    pub escrow_orders: Vec<EscrowOrderPreview>,
    /// Debt, interest, and upkeep settlement summary for `seat`.
    pub upkeep: SeatUpkeepPreview,
    /// Perishable raw units removed from `seat` at Step 6.
    pub spoilage: SeatSpoilagePreview,
    /// Step 0–7 settlement fills.
    pub public_fills: Vec<FillRecord>,
    /// Full Steps 0–7 deterministic receipt summary.
    pub step0_to_7_receipt: Step0To7SweepSummary,
}

impl fmt::Debug for WhatIfPreview {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WhatIfPreview")
            .field("seat", &self.seat)
            .field("turn", &self.turn)
            .field("preview", &"<redacted>")
            .finish()
    }
}
