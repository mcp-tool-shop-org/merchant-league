//! Seat-safe observation boundary ([`SeatObservation`], [`PublicSweepReceipt`],
//! [`PublicPreviewState`]) and scrubbed simulation previews ([`preview_what_if`],
//! [`preview_what_if_with_beacon`], [`preview_seat_bundle`]).

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::commit::{
    AtomicSwapProposal, CommitmentHash, ContractDeliveryOrder, CouncilPlatform, DrandSchedule,
    DualEscrowProposal, EscrowFinishUnlock, IntentPledge, PrivateMandate, SealedOrderBundle,
    derive_turn_beacon,
};
use crate::contracts::{
    ActiveCharter, CartelDomain, CredOffer, CreditCheck, DualEscrowVault, FillRecord, MarketAction,
    PendingWashFreeze, TokenEscrowVault, WashFreezeSettlement,
};
use crate::engine::{
    BrokenPromiseRecord, CheckPreviewOutcome, ConverterOrderPreview, EngineState,
    EscrowOrderPreview, ProjectOrderPreview, SeatState, SeatUpkeepPreview, ShockForecastEvent,
    SwapPreviewOutcome, SweepMode, SweepReceipt, TurnEngine, TurnPhase,
};
use crate::predicates::{SeatStatePredicates, compute_seat_state_predicates};
use crate::scoring::{FundamentalParities, compute_potential};
use crate::types::{
    ActiveShock, AssetId, Bps, CLOSING_ROLL_WINDOW_LEN, CharterRole, EngineError, ProjectRecipe,
    Seat, SeatConfig, ShockKind, TableId, TableMode, TableSpec, Turn, Units,
};

/// Deterministic non-zero sentinel replacing private `dealt_mandate_commits` inside sanitized
/// [`PublicPreviewState`] engines (`C4`).
pub const DEALT_MANDATE_COMMIT_SENTINEL: CommitmentHash = CommitmentHash([0xAA; 32]);

/// Public neutral seed (`0`) used when previews do not take a caller-supplied beacon.
pub const PREVIEW_NEUTRAL_BEACON_SEED: u64 = 0;

/// Caveat attached to [`WhatIfPreview::close_mark_caveat`] explaining the Steps 0–7 boundary.
pub const HYPOTHETICAL_CLOSE_MARK_CAVEAT: &str =
    "Hypothetical close mark after Steps 0-7; assumes open season and no unforecast shock";

/// Computes the deterministic neutral preview beacon for `(table_id, turn)`.
#[must_use]
pub fn preview_neutral_beacon(table_id: TableId, turn: Turn) -> [u8; 32] {
    derive_turn_beacon(PREVIEW_NEUTRAL_BEACON_SEED, table_id, turn)
}

/// Seat-safe 30-field allow-list projection of [`SweepReceipt`], unconditionally dropping
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
    /// Projects a raw [`SweepReceipt`] onto the 30-field seat-safe [`PublicSweepReceipt`].
    #[must_use]
    pub fn from_sweep_receipt(receipt: &SweepReceipt) -> Self {
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
            public_forecasts: receipt.public_forecasts.clone(),
            newly_activated_shocks: receipt.newly_activated_shocks.clone(),
            cred_debt_repaid: receipt.cred_debt_repaid,
            charter_cred_burned: receipt.charter_cred_burned,
            town_import_cred_spent: receipt.town_import_cred_spent,
            works_stipend_cred_minted: receipt.works_stipend_cred_minted,
            batch_trades_cleared: receipt.batch_trades_cleared,
            fills: receipt.fills.clone(),
            wash_freeze_settlements: receipt.wash_freeze_settlements.clone(),
            season_closed: receipt.season_closed,
            pbrs_deltas: receipt.pbrs_deltas.clone(),
        }
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

/// Public subset of [`TableSpec`] omitting pre-seeded `shocks` and private `dealt_mandate_commits`.
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
    pub mandate_deck: Vec<crate::commit::GradedMandateCard>,
    /// Optional drand `quicknet` close schedule for `Game` tables.
    #[serde(default)]
    pub drand_schedule: Option<DrandSchedule>,
    /// Optional `ristretto255` VRF public key (`PK_eval`) for `Eval` / `Club` tables.
    #[serde(default, with = "crate::types::opt_hex32")]
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

impl PublicTableSpec {
    /// Projects a full [`TableSpec`] onto [`PublicTableSpec`] without `shocks` or
    /// `dealt_mandate_commits`.
    #[must_use]
    pub fn from_table_spec(spec: &TableSpec) -> Self {
        Self {
            table_id: spec.table_id,
            mode: spec.mode,
            seat_count: spec.seat_count,
            seats: spec.seats.clone(),
            projects: spec.projects.clone(),
            closing_hazard_bps: spec.closing_hazard_bps,
            mandate_deck: spec.mandate_deck.clone(),
            drand_schedule: spec.drand_schedule,
            eval_vrf_pubkey: spec.eval_vrf_pubkey,
            town_import_premium_bps: spec.town_import_premium_bps,
            works_stipend_per_unit_cred: spec.works_stipend_per_unit_cred,
            mandate_alpha_bps: spec.mandate_alpha_bps,
            cancel_lock_bps: spec.cancel_lock_bps,
            wash_round_trip_min_units: spec.wash_round_trip_min_units,
            wash_freeze_bond_cred: spec.wash_freeze_bond_cred,
            wash_freeze_bounty_cred: spec.wash_freeze_bounty_cred,
        }
    }

    /// Reconstructs a valid sanitized [`TableSpec`] for isolated preview execution, using
    /// empty `shocks` and [`DEALT_MANDATE_COMMIT_SENTINEL`] commitments (`C4`).
    #[must_use]
    pub fn to_sanitized_table_spec(&self) -> TableSpec {
        TableSpec {
            table_id: self.table_id,
            mode: self.mode,
            seat_count: self.seat_count,
            seats: self.seats.clone(),
            projects: self.projects.clone(),
            shocks: Vec::new(),
            closing_hazard_bps: self.closing_hazard_bps,
            mandate_deck: self.mandate_deck.clone(),
            dealt_mandate_commits: vec![
                DEALT_MANDATE_COMMIT_SENTINEL;
                usize::from(self.seat_count)
            ],
            drand_schedule: self.drand_schedule,
            eval_vrf_pubkey: self.eval_vrf_pubkey,
            town_import_premium_bps: self.town_import_premium_bps,
            works_stipend_per_unit_cred: self.works_stipend_per_unit_cred,
            mandate_alpha_bps: self.mandate_alpha_bps,
            cancel_lock_bps: self.cancel_lock_bps,
            wash_round_trip_min_units: self.wash_round_trip_min_units,
            wash_freeze_bond_cred: self.wash_freeze_bond_cred,
            wash_freeze_bounty_cred: self.wash_freeze_bounty_cred,
        }
    }
}

/// Sole seat-safe read boundary constructed from [`TurnEngine::observe_seat`] (`A1`, `C4`).
///
/// Carries only public table state, `seat`'s own `OracleAuditor` hints (without `magnitude_bps`),
/// realized active shocks (`turn >= realize_turn`), announced forecasts (`forecast_turn < turn`),
/// static [`SeatStatePredicates`], and [`PublicSweepReceipt`] history. Never carries raw
/// `SweepReceipt`s, unforecast `newly_drawn_shocks`, shock magnitudes before realize,
/// `dealt_mandate_commits`, peer commits/reveals, or `post_state_digest`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    #[serde(with = "crate::engine::delivery_ledger_serde")]
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
    #[serde(default, with = "crate::types::opt_hex32")]
    pub eval_vrf_pubkey: Option<[u8; 32]>,
    /// Static start-of-observation diagnostics for `seat`.
    pub baseline_predicates: SeatStatePredicates,
    /// Seat-safe 30-field historical Sweep receipts through `last_swept_turn`.
    pub public_receipts: Vec<PublicSweepReceipt>,
    /// Dealt Turn-1 [`PrivateMandate`] belonging solely to `seat`, populated by
    /// [`crate::ClubHost::observe_seat`] (never carries reveal salts or rival seats' mandates).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub own_mandate: Option<PrivateMandate>,
}

pub(crate) fn compute_public_forecasts(state: &EngineState) -> (u8, bool, Vec<ShockForecastEvent>) {
    let is_active_turn = matches!(
        state.phase,
        TurnPhase::TalkAndMarket | TurnPhase::Commit | TurnPhase::Reveal
    );
    let last_swept_turn = if is_active_turn {
        state.turn.saturating_sub(1)
    } else {
        state.turn
    };
    let public_forecasts = state
        .spec
        .shocks
        .iter()
        .chain(state.drawn_shocks.iter())
        .filter(|shock| {
            shock.forecast_turn <= last_swept_turn
                && (!is_active_turn || state.turn <= shock.realize_turn)
        })
        .map(|shock| ShockForecastEvent {
            target_seat: shock.target_seat,
            kind: shock.kind,
            realize_turn: shock.realize_turn,
        })
        .collect();
    (last_swept_turn, is_active_turn, public_forecasts)
}

impl SeatObservation {
    pub(crate) fn from_engine_state(
        state: &EngineState,
        seat: Seat,
        is_auditor: bool,
        baseline_predicates: SeatStatePredicates,
        engine_receipts: &[PublicSweepReceipt],
        caller_receipts: Option<&[SweepReceipt]>,
    ) -> Self {
        let (last_swept_turn, is_active_turn, public_forecasts) = compute_public_forecasts(state);

        let auditor_hints = if is_active_turn && is_auditor {
            state
                .spec
                .shocks
                .iter()
                .chain(state.drawn_shocks.iter())
                .filter(|shock| shock.hint_turn <= state.turn && state.turn < shock.forecast_turn)
                .map(|shock| AuditorShockHint {
                    hint_turn: shock.hint_turn,
                    forecast_turn: shock.forecast_turn,
                    realize_turn: shock.realize_turn,
                    target_seat: shock.target_seat,
                    kind: shock.kind,
                })
                .collect()
        } else {
            Vec::new()
        };

        let public_receipts: Vec<PublicSweepReceipt> = match caller_receipts {
            Some(raw) => raw
                .iter()
                .filter(|r| r.turn <= last_swept_turn)
                .map(PublicSweepReceipt::from_sweep_receipt)
                .collect(),
            None => engine_receipts
                .iter()
                .filter(|r| r.turn <= last_swept_turn)
                .cloned()
                .collect(),
        };

        Self {
            seat: seat.0,
            table_id: state.table_id,
            turn: state.turn,
            phase: state.phase,
            cancel_locked: state.cancel_locked,
            trade_minister: state.trade_minister,
            cartel_chair: state.cartel_chair,
            spec: PublicTableSpec::from_table_spec(&state.spec),
            parities: state.parities.clone(),
            seats: state.seats.clone(),
            offers: state.offers.clone(),
            checks: state.checks.clone(),
            escrows: state.escrows.clone(),
            dual_escrows: state.dual_escrows.clone(),
            pledges: state.pledges.clone(),
            audited_pledges: state.audited_pledges.clone(),
            domains: state.domains.clone(),
            council_platforms: state.council_platforms.clone(),
            charters: state.charters.clone(),
            active_shocks: state.active_shocks.clone(),
            public_forecasts,
            auditor_hints,
            next_offer_id: state.next_offer_id,
            next_check_id: state.next_check_id,
            next_escrow_id: state.next_escrow_id,
            delivery_ledger: state.delivery_ledger.clone(),
            last_potentials: state.last_potentials.clone(),
            pending_town_import_cred_spent: state.pending_town_import_cred_spent,
            fill_log: state.fill_log.clone(),
            pending_wash_freezes: state.pending_wash_freezes.clone(),
            eval_vrf_pubkey: state.spec.eval_vrf_pubkey,
            baseline_predicates,
            public_receipts,
            own_mandate: None,
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
    /// [`crate::ClubHost::observe_seat`] (never carries reveal salts or rival seats' mandates).
    #[must_use]
    pub const fn own_mandate(&self) -> Option<&PrivateMandate> {
        self.own_mandate.as_ref()
    }

    /// Attaches `mandate` as this seat's own [`PrivateMandate`] inside this observation.
    #[must_use]
    pub const fn with_own_mandate(mut self, mandate: PrivateMandate) -> Self {
        self.own_mandate = Some(mandate);
        self
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
    #[serde(with = "crate::engine::delivery_ledger_serde")]
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

impl PublicPreviewState {
    /// Materializes a sanitized [`TurnEngine`] holding zero hidden secrets or peer Phase-2 buffers.
    pub fn to_engine(&self) -> Result<TurnEngine, EngineError> {
        let spec = self.spec.to_sanitized_table_spec();
        spec.validate()?;
        let n = usize::from(spec.seat_count);
        if self.seats.len() != n {
            return Err(EngineError::InvalidConfig(format!(
                "preview state seat count {} does not match TableSpec seat_count {n}",
                self.seats.len()
            )));
        }
        if self.last_potentials.len() != n {
            return Err(EngineError::InvalidConfig(format!(
                "preview state last_potentials len {} does not match TableSpec seat_count {n}",
                self.last_potentials.len()
            )));
        }
        let state = EngineState {
            table_id: self.table_id,
            turn: self.turn,
            phase: self.phase,
            trade_minister: self.trade_minister,
            cartel_chair: self.cartel_chair,
            spec,
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
            mandate_commits: vec![None; n],
            mandate_reveals: vec![None; n],
            turn_commits: BTreeMap::new(),
            turn_reveals: BTreeMap::new(),
            last_potentials: self.last_potentials.clone(),
            last_closing_roll_bps: None,
            pending_town_import_cred_spent: self.pending_town_import_cred_spent,
            drawn_shocks: Vec::new(),
            last_turn_beacon: [0u8; 32],
            cancel_locked: self.cancel_locked,
            fill_log: self.fill_log.clone(),
            pending_wash_freezes: self.pending_wash_freezes.clone(),
        };
        Ok(TurnEngine::from_sanitized_state(state, Vec::new()))
    }
}

/// Tie-break policy for [`preview_what_if`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewTieMode {
    /// Deterministic neutral preview beacon (`derive_turn_beacon(0, table_id, turn)`).
    #[default]
    NeutralSeed,
    /// Evaluates candidate deterministic beacons and selects the tie outcome yielding the lowest
    /// post-Sweep potential for the asking seat.
    PessimisticForSeat,
}

/// Caller-supplied hypothetical counterparty assumptions for [`HumanWhatIfRequest`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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

/// Human What-If Sandbox request executed by [`preview_what_if`] over [`PublicPreviewState`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

/// AI-seat preview request executed by [`preview_seat_bundle`] (`A2`, zero assumptions).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiPreviewRequest {
    /// Asking AI seat index (`0..seat_count`).
    pub seat: u8,
    /// Own Phase-1 [`MarketAction`]s (every action must act for `seat`).
    pub market_actions: Vec<MarketAction>,
    /// Own Phase-2 [`SealedOrderBundle`].
    pub bundle: SealedOrderBundle,
}

/// Refused Phase-1 [`MarketAction`] entry inside [`WhatIfPreview::refused_market_actions`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefusedActionPreview {
    /// Zero-based index inside `market_actions`.
    pub action_index: usize,
    /// Stable [`EngineError::code`] identifier.
    pub error_code: String,
    /// Scrubbed human-readable refusal description.
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
    /// Projects a raw [`SweepReceipt`] onto Steps 0–7 fields only.
    #[must_use]
    pub fn from_sweep_receipt(receipt: &SweepReceipt) -> Self {
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

/// Scrubbed Steps 0–7 preview response returned by [`preview_what_if`],
/// [`preview_what_if_with_beacon`], and [`preview_seat_bundle`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

fn build_peer_bundles(
    seat: u8,
    assumptions: &CounterpartyAssumptions,
) -> BTreeMap<u8, SealedOrderBundle> {
    let mut peer_bundles = assumptions.assumed_peer_bundles.clone();
    for swap in &assumptions.assumed_cosigned_swaps {
        if swap.maker == swap.taker {
            continue;
        }
        if swap.maker == seat {
            peer_bundles
                .entry(swap.taker)
                .or_default()
                .atomic_swaps
                .push(swap.clone());
        } else if swap.taker == seat {
            peer_bundles
                .entry(swap.maker)
                .or_default()
                .atomic_swaps
                .push(swap.clone());
        } else {
            peer_bundles
                .entry(swap.maker)
                .or_default()
                .atomic_swaps
                .push(swap.clone());
            peer_bundles
                .entry(swap.taker)
                .or_default()
                .atomic_swaps
                .push(swap.clone());
        }
    }
    for dual in &assumptions.assumed_cosigned_dual_escrows {
        if dual.buyer != seat {
            peer_bundles
                .entry(dual.buyer)
                .or_default()
                .dual_escrows
                .push(dual.clone());
        }
        if dual.seller != seat && dual.seller != dual.buyer {
            peer_bundles
                .entry(dual.seller)
                .or_default()
                .dual_escrows
                .push(dual.clone());
        }
    }
    for (peer, order) in &assumptions.assumed_peer_deliveries {
        if *peer != seat {
            peer_bundles
                .entry(*peer)
                .or_default()
                .contract_deliveries
                .push(order.clone());
        }
    }
    for (peer, unlock) in &assumptions.assumed_peer_escrow_finishes {
        if *peer != seat {
            peer_bundles
                .entry(*peer)
                .or_default()
                .escrow_finishes
                .push(unlock.clone());
        }
    }
    peer_bundles
}

fn run_what_if_inner(
    state: &PublicPreviewState,
    request: &HumanWhatIfRequest,
    turn_beacon: [u8; 32],
) -> Result<(WhatIfPreview, Units, bool), EngineError> {
    let engine = state.to_engine()?;
    let baseline_predicates = compute_seat_state_predicates(engine.state(), Seat(request.seat))?;
    let peer_bundles = build_peer_bundles(request.seat, &request.assumptions);

    let (bundle_preview, post_state, refused_market_actions) = engine
        .preview_bundle_with_context_internal(
            Seat(request.seat),
            &request.market_actions,
            request.cancel_lock_at,
            &request.assumptions.assumed_cancelled_offer_ids,
            &request.assumptions.assumed_peer_market_actions,
            &request.bundle,
            &peer_bundles,
            &request.assumptions.assumed_bounced_check_ids,
            turn_beacon,
            SweepMode::PreviewSteps0To7,
        )?;
    let post_sweep_predicates = compute_seat_state_predicates(&post_state, Seat(request.seat))?;
    let post_potential = compute_potential(&post_state, request.seat);
    let elected_seat = bundle_preview
        .sweep_receipt
        .elected_officers
        .is_some_and(|(tm, cc)| tm == request.seat || cc == Some(request.seat));

    let swap_orders = bundle_preview
        .swap_orders
        .iter()
        .map(|s| PublicSwapOrderPreview {
            swap_id: s.swap_id,
            maker: s.maker,
            taker: s.taker,
            outcome: match s.outcome {
                SwapPreviewOutcome::Settled => PublicSwapOutcome::Settled,
                SwapPreviewOutcome::UnmatchedOrInsufficientBalance => {
                    PublicSwapOutcome::UnmatchedOrInsufficientBalance
                }
            },
        })
        .collect();
    let check_orders = bundle_preview
        .check_orders
        .iter()
        .map(|c| PublicCheckOrderPreview {
            check_id: c.check_id,
            outcome: match c.outcome {
                CheckPreviewOutcome::Cashed => PublicCheckOutcome::Cashed,
                CheckPreviewOutcome::Bounced => PublicCheckOutcome::Bounced,
                CheckPreviewOutcome::NotEligibleOrIgnored => {
                    PublicCheckOutcome::IneligibleOrIgnored
                }
            },
        })
        .collect();

    let step0_to_7_receipt =
        Step0To7SweepSummary::from_sweep_receipt(&bundle_preview.sweep_receipt);
    let public_fills = step0_to_7_receipt.fills.clone();
    let preview = WhatIfPreview {
        seat: request.seat,
        turn: bundle_preview.turn,
        baseline_predicates,
        pre_seat_state: bundle_preview.pre_seat_state,
        post_market_seat_state: bundle_preview.post_market_seat_state,
        post_sweep_seat_state: bundle_preview.post_seat_state,
        post_sweep_predicates,
        close_mark_caveat: HYPOTHETICAL_CLOSE_MARK_CAVEAT.to_owned(),
        refused_market_actions,
        converter_orders: bundle_preview.converter_orders,
        project_orders: bundle_preview.project_orders,
        swap_orders,
        check_orders,
        escrow_orders: bundle_preview.escrow_orders,
        upkeep: bundle_preview.upkeep,
        spoilage: SeatSpoilagePreview {
            perishable_spoiled_units: bundle_preview.seat_spoilage_clawed_back,
        },
        public_fills,
        step0_to_7_receipt,
    };
    Ok((preview, post_potential, elected_seat))
}

/// Runs a human What-If Sandbox simulation over [`PublicPreviewState`] using an explicit
/// `turn_beacon` (primarily used by the §2.5 Steps 0–7 equivalence proof).
pub fn preview_what_if_with_beacon(
    state: &PublicPreviewState,
    request: &HumanWhatIfRequest,
    turn_beacon: [u8; 32],
) -> Result<WhatIfPreview, EngineError> {
    Ok(run_what_if_inner(state, request, turn_beacon)?.0)
}

/// Runs a human What-If Sandbox simulation over [`PublicPreviewState`] in
/// `SweepMode::PreviewSteps0To7` (counterparty assumptions + pre/post [`SeatStatePredicates`])
/// under `request.tie_mode`.
pub fn preview_what_if(
    state: &PublicPreviewState,
    request: &HumanWhatIfRequest,
) -> Result<WhatIfPreview, EngineError> {
    match request.tie_mode {
        PreviewTieMode::NeutralSeed => preview_what_if_with_beacon(
            state,
            request,
            preview_neutral_beacon(state.table_id, Turn(state.turn)),
        ),
        PreviewTieMode::PessimisticForSeat => {
            let mut worst: Option<(Units, bool, u64, WhatIfPreview)> = None;
            for seed_idx in 0..8_u64 {
                let beacon = derive_turn_beacon(seed_idx, state.table_id, Turn(state.turn));
                let (preview, pot, elected) = run_what_if_inner(state, request, beacon)?;
                let candidate_key = (pot, elected, seed_idx);
                let replace = match &worst {
                    None => true,
                    Some((best_pot, best_elected, best_seed, _)) => {
                        candidate_key < (*best_pot, *best_elected, *best_seed)
                    }
                };
                if replace {
                    worst = Some((pot, elected, seed_idx, preview));
                }
            }
            match worst {
                Some((_, _, _, preview)) => Ok(preview),
                None => Err(EngineError::InvalidConfig(
                    "pessimistic preview candidate set cannot be empty".to_owned(),
                )),
            }
        }
    }
}

/// Runs an AI-seat simulation over [`PublicPreviewState`] in `SweepMode::PreviewSteps0To7`
/// (asking seat's own `market_actions` and `bundle` only, empty peers, neutral beacon, zero
/// assumptions).
pub fn preview_seat_bundle(
    state: &PublicPreviewState,
    request: &AiPreviewRequest,
) -> Result<WhatIfPreview, EngineError> {
    for action in &request.market_actions {
        if action.actor_seat() != request.seat {
            return Err(EngineError::Unauthorized(format!(
                "AI preview seat {} cannot submit Phase-1 action for seat {}",
                request.seat,
                action.actor_seat()
            )));
        }
    }
    let human_req = HumanWhatIfRequest {
        seat: request.seat,
        market_actions: request.market_actions.clone(),
        cancel_lock_at: None,
        bundle: request.bundle.clone(),
        tie_mode: PreviewTieMode::NeutralSeed,
        assumptions: CounterpartyAssumptions::default(),
    };
    preview_what_if_with_beacon(
        state,
        &human_req,
        preview_neutral_beacon(state.table_id, Turn(state.turn)),
    )
}
