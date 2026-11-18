//! Seat-safe observation boundary ([`SeatObservation`], [`PublicSweepReceipt`],
//! [`PublicPreviewState`]) and scrubbed simulation previews ([`preview_what_if`],
//! [`preview_what_if_with_beacon`], [`preview_seat_bundle`]).

use std::collections::BTreeMap;

pub use league_seat_types::{
    AiPreviewRequest, AuditorShockHint, CounterpartyAssumptions, DEALT_MANDATE_COMMIT_SENTINEL,
    HYPOTHETICAL_CLOSE_MARK_CAVEAT, HumanWhatIfRequest, PREVIEW_NEUTRAL_BEACON_SEED,
    PreviewTieMode, PublicCheckOrderPreview, PublicCheckOutcome, PublicPreviewState,
    PublicSwapOrderPreview, PublicSwapOutcome, PublicSweepReceipt, PublicTableSpec,
    RefusedActionPreview, SeatObservation, SeatSpoilagePreview, Step0To7SweepSummary,
    WhatIfPreview,
};

use league_seat_types::SeatObservationPublicParts;

use crate::commit::{PrivateMandate, SealedOrderBundle, derive_turn_beacon};
use crate::engine::{
    CheckPreviewOutcome, EngineState, ShockForecastEvent, SwapPreviewOutcome, SweepMode,
    SweepReceipt, TurnEngine, TurnPhase,
};
use crate::predicates::{SeatStatePredicates, compute_seat_state_predicates};
use crate::scoring::compute_potential;
use crate::types::{EngineError, Seat, TableId, TableSpec, Turn, Units};

/// Computes the deterministic neutral preview beacon for `(table_id, turn)`.
#[must_use]
pub fn preview_neutral_beacon(table_id: TableId, turn: Turn) -> [u8; 32] {
    derive_turn_beacon(PREVIEW_NEUTRAL_BEACON_SEED, table_id, turn)
}

/// Projects a raw [`SweepReceipt`] onto the 30-field seat-safe [`PublicSweepReceipt`].
#[must_use]
pub(crate) fn public_sweep_receipt_from_raw(receipt: &SweepReceipt) -> PublicSweepReceipt {
    PublicSweepReceipt {
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

impl From<&SweepReceipt> for PublicSweepReceipt {
    fn from(receipt: &SweepReceipt) -> Self {
        public_sweep_receipt_from_raw(receipt)
    }
}

/// Projects a full [`TableSpec`] onto [`PublicTableSpec`] without `shocks` or `dealt_mandate_commits`.
#[must_use]
pub(crate) fn public_table_spec_from_raw(spec: &TableSpec) -> PublicTableSpec {
    PublicTableSpec {
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
pub(crate) fn sanitized_table_spec_from_public(spec: &PublicTableSpec) -> TableSpec {
    TableSpec {
        table_id: spec.table_id,
        mode: spec.mode,
        seat_count: spec.seat_count,
        seats: spec.seats.clone(),
        projects: spec.projects.clone(),
        shocks: Vec::new(),
        closing_hazard_bps: spec.closing_hazard_bps,
        mandate_deck: spec.mandate_deck.clone(),
        dealt_mandate_commits: vec![DEALT_MANDATE_COMMIT_SENTINEL; usize::from(spec.seat_count)],
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

pub(crate) fn build_seat_observation_from_engine_state(
    state: &EngineState,
    seat: Seat,
    is_auditor: bool,
    baseline_predicates: SeatStatePredicates,
    engine_receipts: &[PublicSweepReceipt],
    caller_receipts: Option<&[SweepReceipt]>,
    own_mandate: Option<PrivateMandate>,
) -> SeatObservation {
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
            .map(public_sweep_receipt_from_raw)
            .collect(),
        None => engine_receipts
            .iter()
            .filter(|r| r.turn <= last_swept_turn)
            .cloned()
            .collect(),
    };

    SeatObservation::from_host_parts(
        SeatObservationPublicParts {
            seat: seat.0,
            table_id: state.table_id,
            turn: state.turn,
            phase: state.phase,
            cancel_locked: state.cancel_locked,
            trade_minister: state.trade_minister,
            cartel_chair: state.cartel_chair,
            spec: public_table_spec_from_raw(&state.spec),
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
        },
        own_mandate,
    )
}

/// Materializes a sanitized [`TurnEngine`] holding zero hidden secrets or peer Phase-2 buffers.
pub(crate) fn preview_state_to_engine(
    preview_state: &PublicPreviewState,
) -> Result<TurnEngine, EngineError> {
    let spec = sanitized_table_spec_from_public(&preview_state.spec);
    spec.validate()?;
    let n = usize::from(spec.seat_count);
    if preview_state.seats.len() != n {
        return Err(EngineError::InvalidConfig(format!(
            "preview state seat count {} does not match TableSpec seat_count {n}",
            preview_state.seats.len()
        )));
    }
    if preview_state.last_potentials.len() != n {
        return Err(EngineError::InvalidConfig(format!(
            "preview state last_potentials len {} does not match TableSpec seat_count {n}",
            preview_state.last_potentials.len()
        )));
    }
    let state = EngineState {
        table_id: preview_state.table_id,
        turn: preview_state.turn,
        phase: preview_state.phase,
        trade_minister: preview_state.trade_minister,
        cartel_chair: preview_state.cartel_chair,
        spec,
        parities: preview_state.parities.clone(),
        seats: preview_state.seats.clone(),
        offers: preview_state.offers.clone(),
        checks: preview_state.checks.clone(),
        escrows: preview_state.escrows.clone(),
        dual_escrows: preview_state.dual_escrows.clone(),
        pledges: preview_state.pledges.clone(),
        audited_pledges: preview_state.audited_pledges.clone(),
        domains: preview_state.domains.clone(),
        council_platforms: preview_state.council_platforms.clone(),
        charters: preview_state.charters.clone(),
        active_shocks: preview_state.active_shocks.clone(),
        next_offer_id: preview_state.next_offer_id,
        next_check_id: preview_state.next_check_id,
        next_escrow_id: preview_state.next_escrow_id,
        delivery_ledger: preview_state.delivery_ledger.clone(),
        mandate_commits: vec![None; n],
        mandate_reveals: vec![None; n],
        turn_commits: BTreeMap::new(),
        turn_reveals: BTreeMap::new(),
        last_potentials: preview_state.last_potentials.clone(),
        last_closing_roll_bps: None,
        pending_town_import_cred_spent: preview_state.pending_town_import_cred_spent,
        drawn_shocks: Vec::new(),
        last_turn_beacon: [0u8; 32],
        cancel_locked: preview_state.cancel_locked,
        fill_log: preview_state.fill_log.clone(),
        pending_wash_freezes: preview_state.pending_wash_freezes.clone(),
    };
    Ok(TurnEngine::from_sanitized_state(state, Vec::new()))
}

/// Projects a raw [`SweepReceipt`] onto Steps 0–7 fields only.
#[must_use]
pub(crate) fn step0_to_7_summary_from_raw(receipt: &SweepReceipt) -> Step0To7SweepSummary {
    Step0To7SweepSummary {
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

impl From<&SweepReceipt> for Step0To7SweepSummary {
    fn from(receipt: &SweepReceipt) -> Self {
        step0_to_7_summary_from_raw(receipt)
    }
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
    let engine = preview_state_to_engine(state)?;
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

    let step0_to_7_receipt = step0_to_7_summary_from_raw(&bundle_preview.sweep_receipt);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_state_to_engine_rejects_mismatched_seat_and_potential_lengths() {
        let Ok(spec) = TableSpec::standard_chiral_ring(TableId(401), 4, 1) else {
            panic!("standard_chiral_ring failed");
        };
        let Ok(engine) = TurnEngine::new(spec) else {
            panic!("TurnEngine::new failed");
        };
        let Ok(obs) = engine.observe_seat(Seat(0)) else {
            panic!("observe_seat failed");
        };
        let mut bad_seats = obs.to_public_preview_state();
        bad_seats.seats.pop();
        assert!(preview_state_to_engine(&bad_seats).is_err());

        let mut bad_potentials = obs.to_public_preview_state();
        bad_potentials.last_potentials.pop();
        assert!(preview_state_to_engine(&bad_potentials).is_err());
    }
}
