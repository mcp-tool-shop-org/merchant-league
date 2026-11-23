//! Arm-independent candidate generators and blind request builder (`§3.1` sources 1 & 4, `§5.2`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use league_core::{
    AssetId, BPS_SCALE, CharterBidOrder, CharterRole, CheckStatus, ClubBot, ConverterRunOrder,
    CouncilVoteBallot, EscrowStatus, EvalActSubmission, MarketAction, OfferProposal, OfferSide,
    PrivateMandate, ProjectDeliveryOrder, SealedOrderBundle, Seat, SeatObservation, TurnPhase,
    Units, checked_ceil_units_parity, mul_units_parity, town_import_cost_cred,
};

use crate::error::ScorerError;
use crate::postures::{preview_submission, submission_from_club_bot};
use crate::rollout::verify_and_fork_checkpoint;
use crate::types::{
    BLIND_SCORING_SCHEMA_VERSION, BlindCandidate, BlindScoringRequest,
    DROP_TIE_BREAK_BY_CANDIDATE_ID, EVAL_TABLE_SEAT_COUNT, SYNTH_CLAIM_MATURING_EXACT,
    SYNTH_CONVERTER_INPUT_DEFICIT_COVER, SYNTH_COUNCIL_SELF_NOMINATION,
    SYNTH_MAX_CONVERTER_BATCHES, SYNTH_MAX_PROJECT_BATCHES, SYNTH_PROJECT_RANK_BY_EQUITY,
    ScorerConfig, candidate_id_for_submission, is_valid_sha256_hex, validate_state_id,
};

/// Deduplicates `candidates` by canonical `candidate_id` (`sha256_hex` of compact submission JSON),
/// merging duplicate flags via logical OR and sorting ascending by `candidate_id` (`§3.1` item 5).
pub fn dedupe_candidates(
    candidates: Vec<BlindCandidate>,
) -> Result<Vec<BlindCandidate>, ScorerError> {
    let mut merged: BTreeMap<String, BlindCandidate> = BTreeMap::new();
    for mut cand in candidates {
        let canonical_id = candidate_id_for_submission(&cand.submission)?;
        cand.candidate_id = canonical_id.clone();
        merged
            .entry(canonical_id)
            .and_modify(|existing| {
                existing.in_span = existing.in_span || cand.in_span;
                existing.is_arm_independent =
                    existing.is_arm_independent || cand.is_arm_independent;
            })
            .or_insert(cand);
    }
    Ok(merged.into_values().collect())
}

fn free_asset(obs: &SeatObservation, seat: u8, asset: AssetId) -> Units {
    let Some(state) = obs.seats.get(usize::from(seat)) else {
        return 0;
    };
    match asset {
        AssetId::Cred => state.cred,
        AssetId::Raw(g) => state.raw.get(usize::from(g)).copied().unwrap_or(0),
        AssetId::Refined(m) => state.refined.get(usize::from(m)).copied().unwrap_or(0),
    }
}

fn is_legal_synthetic(obs: &SeatObservation, sub: &EvalActSubmission) -> bool {
    let Some(preview) = preview_submission(obs, &sub.market_actions, &sub.bundle) else {
        return false;
    };
    if !preview.refused_market_actions.is_empty() {
        return false;
    }
    let baseline = &obs.baseline_predicates.upkeep;
    !(baseline.held_units >= baseline.owed_units && preview.upkeep.new_shortfall_debt > 0)
}

fn to_sorted_bucket_entries(
    obs: &SeatObservation,
    drafts: Vec<EvalActSubmission>,
) -> Result<Vec<(String, EvalActSubmission)>, ScorerError> {
    let mut unique: BTreeMap<String, EvalActSubmission> = BTreeMap::new();
    for sub in drafts {
        if !is_legal_synthetic(obs, &sub) {
            continue;
        }
        let cid = candidate_id_for_submission(&sub)?;
        unique.entry(cid).or_insert(sub);
    }
    Ok(unique.into_iter().collect())
}

fn gen_claims_and_council(obs: &SeatObservation) -> Vec<EvalActSubmission> {
    let mut drafts = Vec::new();
    let seat = obs.seat;
    let mut bundle_claims = SealedOrderBundle::default();

    if SYNTH_CLAIM_MATURING_EXACT {
        for (&check_id, check) in &obs.checks {
            if check.payee == seat
                && check.status == CheckStatus::Open
                && obs.turn >= check.due_turn
                && obs.turn < check.expiration_turn
                && free_asset(obs, check.drawer, check.asset) >= check.deliver_min
            {
                bundle_claims.check_cashes.push(check_id);
            }
        }
        for (&escrow_id, escrow) in &obs.escrows {
            if escrow.destination == seat
                && escrow.status == EscrowStatus::Locked
                && escrow.finish_after_turn.is_none_or(|t| obs.turn >= t)
                && obs.turn < escrow.cancel_after_turn
                && escrow.condition_hash.is_none()
            {
                bundle_claims
                    .escrow_finishes
                    .push(league_core::EscrowFinishUnlock {
                        escrow_id,
                        preimage: None,
                    });
            }
        }
        if !bundle_claims.check_cashes.is_empty() || !bundle_claims.escrow_finishes.is_empty() {
            drafts.push(EvalActSubmission {
                market_actions: Vec::new(),
                pass_b_actions: Vec::new(),
                bundle: bundle_claims.clone(),
            });
        }
    }

    if SYNTH_COUNCIL_SELF_NOMINATION {
        if obs.turn.is_multiple_of(4) {
            let vote = CouncilVoteBallot {
                trade_minister: seat,
                cartel_chair: seat,
            };
            let mut b = bundle_claims.clone();
            b.council_vote = Some(vote);
            drafts.push(EvalActSubmission {
                market_actions: Vec::new(),
                pass_b_actions: Vec::new(),
                bundle: b.clone(),
            });
            if obs.phase == TurnPhase::TalkAndMarket && !obs.cancel_locked {
                let platform = obs
                    .council_platforms
                    .get(&seat)
                    .cloned()
                    .unwrap_or_default();
                drafts.push(EvalActSubmission {
                    market_actions: vec![MarketAction::StandForOffice { seat, platform }],
                    pass_b_actions: Vec::new(),
                    bundle: b,
                });
            }
        }
        if obs.turn.is_multiple_of(6) {
            for role in [
                CharterRole::ClearinghouseBroker,
                CharterRole::OracleAuditor,
                CharterRole::CustomsInspector,
            ] {
                let mut b = bundle_claims.clone();
                b.charter_bids.push(CharterBidOrder {
                    role,
                    bid_cred: 150,
                });
                drafts.push(EvalActSubmission {
                    market_actions: Vec::new(),
                    pass_b_actions: Vec::new(),
                    bundle: b,
                });
            }
        }
    }

    drafts
}

fn gen_upkeep_and_converter_imports(
    obs: &SeatObservation,
    config: &ScorerConfig,
) -> Vec<EvalActSubmission> {
    if obs.phase != TurnPhase::TalkAndMarket || obs.cancel_locked {
        return Vec::new();
    }
    let seat_idx = usize::from(obs.seat);
    let Some(seat_cfg) = obs.spec.seats.get(seat_idx) else {
        return Vec::new();
    };
    let mut drafts = Vec::new();

    let upkeep_good = AssetId::Refined(seat_cfg.upkeep_good);
    let upkeep_held = free_asset(obs, obs.seat, upkeep_good);
    let upkeep_parity = obs.parities.asset_parity(upkeep_good).max(1);
    let cover_target = seat_cfg
        .upkeep_amount
        .saturating_mul(i64::from(config.upkeep_cover_turns));
    for target in [seat_cfg.upkeep_amount, cover_target] {
        let shortfall = target.saturating_sub(upkeep_held);
        if shortfall > 0
            && town_import_cost_cred(shortfall, upkeep_parity, obs.spec.town_import_premium_bps)
                .is_ok_and(|c| c > 0 && c <= free_asset(obs, obs.seat, AssetId::Cred))
        {
            drafts.push(EvalActSubmission {
                market_actions: vec![MarketAction::TownImport {
                    seat: obs.seat,
                    good: upkeep_good,
                    amount: shortfall,
                }],
                pass_b_actions: Vec::new(),
                bundle: SealedOrderBundle::default(),
            });
        }
    }

    if SYNTH_CONVERTER_INPUT_DEFICIT_COVER {
        for recipe in &seat_cfg.converters {
            let mut input_actions = Vec::new();
            for &(in_g, per_run) in &recipe.inputs {
                let raw_good = AssetId::Raw(in_g);
                let held = free_asset(obs, obs.seat, raw_good);
                let deficit = per_run.saturating_sub(held);
                if deficit > 0 {
                    input_actions.push(MarketAction::TownImport {
                        seat: obs.seat,
                        good: raw_good,
                        amount: deficit,
                    });
                }
            }
            if !input_actions.is_empty() {
                drafts.push(EvalActSubmission {
                    market_actions: input_actions,
                    pass_b_actions: Vec::new(),
                    bundle: SealedOrderBundle::default(),
                });
            }
        }
    }

    drafts
}

fn gen_cross_and_quotes(obs: &SeatObservation, config: &ScorerConfig) -> Vec<EvalActSubmission> {
    if obs.phase != TurnPhase::TalkAndMarket || obs.cancel_locked {
        return Vec::new();
    }
    let seat_idx = usize::from(obs.seat);
    let (Some(seat_state), Some(seat_cfg)) =
        (obs.seats.get(seat_idx), obs.spec.seats.get(seat_idx))
    else {
        return Vec::new();
    };
    if seat_state.wash_frozen_until_turn >= obs.turn {
        return Vec::new();
    }

    let mut drafts = Vec::new();
    let upkeep_good = AssetId::Refined(seat_cfg.upkeep_good);
    let upkeep_reserve = seat_cfg
        .upkeep_amount
        .saturating_mul(i64::from(config.upkeep_cover_turns));

    // Collect top-of-book non-self public Buy bids and Sell asks.
    let mut best_buy: BTreeMap<AssetId, (Units, Units)> = BTreeMap::new();
    let mut best_sell: BTreeMap<AssetId, (Units, Units)> = BTreeMap::new();
    for offer in obs.offers.values() {
        if offer.maker == obs.seat || offer.domain_id.is_some() || offer.remaining_units <= 0 {
            continue;
        }
        match offer.side {
            OfferSide::Buy => {
                best_buy
                    .entry(offer.good)
                    .and_modify(|(p, u)| {
                        if offer.price_per_unit_cred > *p {
                            *p = offer.price_per_unit_cred;
                            *u = offer.remaining_units;
                        }
                    })
                    .or_insert((offer.price_per_unit_cred, offer.remaining_units));
            }
            OfferSide::Sell => {
                best_sell
                    .entry(offer.good)
                    .and_modify(|(p, u)| {
                        if offer.price_per_unit_cred < *p {
                            *p = offer.price_per_unit_cred;
                            *u = offer.remaining_units;
                        }
                    })
                    .or_insert((offer.price_per_unit_cred, offer.remaining_units));
            }
        }
    }

    // 1. Crossing resting top-of-book offers at configured partial crossing fractions.
    for &frac_bps in &config.partial_crossing_fractions_bps {
        if frac_bps == 0 {
            continue;
        }
        for (&good, &(bid_price, bid_avail)) in &best_buy {
            let held = free_asset(obs, obs.seat, good);
            let sellable = if good == upkeep_good {
                held.saturating_sub(upkeep_reserve)
            } else {
                held
            };
            let target =
                ((i128::from(bid_avail.min(sellable)) * i128::from(frac_bps)) / BPS_SCALE) as Units;
            if target > 0 && bid_price > 0 {
                drafts.push(EvalActSubmission {
                    market_actions: vec![MarketAction::PostOffer {
                        seat: obs.seat,
                        order: OfferProposal {
                            side: OfferSide::Sell,
                            good,
                            amount: target,
                            price_per_unit_cred: bid_price,
                            domain_id: None,
                            is_hybrid: false,
                        },
                    }],
                    pass_b_actions: Vec::new(),
                    bundle: SealedOrderBundle::default(),
                });
            }
        }
        for (&good, &(ask_price, ask_avail)) in &best_sell {
            if ask_price <= 0 || seat_state.cred <= 0 {
                continue;
            }
            let max_by_cred =
                ((i128::from(seat_state.cred) * 1_000_i128) / i128::from(ask_price)) as Units;
            let cap = ask_avail.min(max_by_cred);
            let mut amount = ((i128::from(cap) * i128::from(frac_bps)) / BPS_SCALE) as Units;
            while amount > 0
                && checked_ceil_units_parity(amount, ask_price)
                    .is_ok_and(|lock| lock > seat_state.cred)
            {
                amount = amount.saturating_sub(1);
            }
            if amount > 0 {
                drafts.push(EvalActSubmission {
                    market_actions: vec![MarketAction::PostOffer {
                        seat: obs.seat,
                        order: OfferProposal {
                            side: OfferSide::Buy,
                            good,
                            amount,
                            price_per_unit_cred: ask_price,
                            domain_id: None,
                            is_hybrid: false,
                        },
                    }],
                    pass_b_actions: Vec::new(),
                    bundle: SealedOrderBundle::default(),
                });
            }
        }
    }

    // 2. Parity-anchored limit quote postings inside `parity_spread_threshold_bps`.
    let n_seats = u8::try_from(obs.spec.seats.len()).unwrap_or(0);
    for &offset_bps in &config.quote_offsets_bps {
        if offset_bps.unsigned_abs() > config.parity_spread_threshold_bps {
            continue;
        }
        if offset_bps < 0 {
            let mut buy_targets = vec![upkeep_good];
            for recipe in &seat_cfg.converters {
                for &(in_g, _) in &recipe.inputs {
                    if in_g != obs.seat {
                        buy_targets.push(AssetId::Raw(in_g));
                    }
                }
            }
            for good in buy_targets {
                let parity = obs.parities.asset_parity(good).max(1);
                let price = ((i128::from(parity) * (BPS_SCALE + i128::from(offset_bps)))
                    / BPS_SCALE)
                    .max(1) as Units;
                let max_units = ((i128::from(seat_state.cred / 2) * 1_000_i128) / i128::from(price))
                    .min(1_000) as Units;
                if max_units >= 100
                    && checked_ceil_units_parity(max_units, price)
                        .is_ok_and(|lock| lock <= seat_state.cred)
                {
                    drafts.push(EvalActSubmission {
                        market_actions: vec![MarketAction::PostOffer {
                            seat: obs.seat,
                            order: OfferProposal {
                                side: OfferSide::Buy,
                                good,
                                amount: max_units,
                                price_per_unit_cred: price,
                                domain_id: None,
                                is_hybrid: false,
                            },
                        }],
                        pass_b_actions: Vec::new(),
                        bundle: SealedOrderBundle::default(),
                    });
                }
            }
        } else {
            for g in 0..n_seats {
                for good in [AssetId::Raw(g), AssetId::Refined(g)] {
                    let held = free_asset(obs, obs.seat, good);
                    let sellable = if good == upkeep_good {
                        held.saturating_sub(upkeep_reserve)
                    } else {
                        held
                    };
                    if sellable < 100 {
                        continue;
                    }
                    let parity = obs.parities.asset_parity(good).max(1);
                    let price = ((i128::from(parity) * (BPS_SCALE + i128::from(offset_bps)))
                        / BPS_SCALE)
                        .max(1) as Units;
                    drafts.push(EvalActSubmission {
                        market_actions: vec![MarketAction::PostOffer {
                            seat: obs.seat,
                            order: OfferProposal {
                                side: OfferSide::Sell,
                                good,
                                amount: sellable,
                                price_per_unit_cred: price,
                                domain_id: None,
                                is_hybrid: false,
                            },
                        }],
                        pass_b_actions: Vec::new(),
                        bundle: SealedOrderBundle::default(),
                    });
                }
            }
        }
    }

    drafts
}

fn ranked_converter_batches(obs: &SeatObservation) -> Vec<ConverterRunOrder> {
    let seat_idx = usize::from(obs.seat);
    let (Some(seat_state), Some(seat_cfg)) =
        (obs.seats.get(seat_idx), obs.spec.seats.get(seat_idx))
    else {
        return Vec::new();
    };
    let mut scored: Vec<(usize, Units)> = seat_cfg
        .converters
        .iter()
        .enumerate()
        .map(|(idx, recipe)| {
            let out_val = mul_units_parity(
                recipe.output_amount,
                obs.parities
                    .asset_parity(AssetId::Refined(recipe.output_good)),
            );
            let in_val: Units = recipe
                .inputs
                .iter()
                .map(|&(g, units)| {
                    mul_units_parity(units, obs.parities.asset_parity(AssetId::Raw(g)))
                })
                .sum();
            (idx, out_val.saturating_sub(in_val))
        })
        .collect();
    scored.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let mut avail_raw = seat_state.raw.clone();
    let mut orders = Vec::new();
    for (recipe_idx, _) in scored {
        if orders.len() >= SYNTH_MAX_CONVERTER_BATCHES {
            break;
        }
        let recipe = &seat_cfg.converters[recipe_idx];
        let mut max_possible = recipe.max_runs_per_turn;
        for &(in_g, per_run) in &recipe.inputs {
            if per_run <= 0 {
                max_possible = 0;
                break;
            }
            let held = avail_raw.get(usize::from(in_g)).copied().unwrap_or(0);
            let by_input = u32::try_from((held / per_run).max(0)).unwrap_or(u32::MAX);
            max_possible = max_possible.min(by_input);
        }
        if max_possible > 0 {
            for &(in_g, per_run) in &recipe.inputs {
                if let Some(slot) = avail_raw.get_mut(usize::from(in_g)) {
                    *slot = slot.saturating_sub(per_run.saturating_mul(i64::from(max_possible)));
                }
            }
            orders.push(ConverterRunOrder {
                recipe_idx,
                runs: max_possible,
            });
        }
    }
    orders
}

fn ranked_project_batches(obs: &SeatObservation) -> Vec<ProjectDeliveryOrder> {
    let seat_idx = usize::from(obs.seat);
    let (Some(seat_state), Some(seat_cfg)) =
        (obs.seats.get(seat_idx), obs.spec.seats.get(seat_idx))
    else {
        return Vec::new();
    };
    let mut ranked: Vec<(usize, Units, u16)> = obs
        .spec
        .projects
        .iter()
        .enumerate()
        .map(|(idx, p)| (idx, p.equity_points, p.id))
        .collect();
    if SYNTH_PROJECT_RANK_BY_EQUITY {
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.2.cmp(&b.2)));
    } else {
        ranked.sort_by_key(|a| a.2);
    }

    let mut spendable = seat_state.refined.clone();
    let upkeep_idx = usize::from(seat_cfg.upkeep_good);
    if let Some(slot) = spendable.get_mut(upkeep_idx) {
        *slot = slot.saturating_sub(seat_cfg.upkeep_amount);
    }

    let mut orders = Vec::new();
    for (proj_idx, _, _) in ranked {
        if orders.len() >= SYNTH_MAX_PROJECT_BATCHES {
            break;
        }
        let proj = &obs.spec.projects[proj_idx];
        let mut batches = u32::MAX;
        for &(g_idx, per_batch) in &proj.inputs {
            if per_batch <= 0 {
                batches = 0;
                break;
            }
            let held = spendable.get(usize::from(g_idx)).copied().unwrap_or(0);
            let cover = u32::try_from((held / per_batch).max(0)).unwrap_or(u32::MAX);
            batches = batches.min(cover);
        }
        if batches > 0 && batches < u32::MAX {
            for &(g_idx, per_batch) in &proj.inputs {
                if let Some(slot) = spendable.get_mut(usize::from(g_idx)) {
                    *slot = slot.saturating_sub(per_batch.saturating_mul(i64::from(batches)));
                }
            }
            orders.push(ProjectDeliveryOrder {
                project_id: proj.id,
                batches,
            });
        }
    }
    orders
}

fn gen_batches(obs: &SeatObservation) -> Vec<EvalActSubmission> {
    let mut drafts = Vec::new();
    let conv = ranked_converter_batches(obs);
    for c in &conv {
        drafts.push(EvalActSubmission {
            market_actions: Vec::new(),
            pass_b_actions: Vec::new(),
            bundle: SealedOrderBundle {
                converter_runs: vec![c.clone()],
                ..SealedOrderBundle::default()
            },
        });
    }
    if conv.len() > 1 {
        drafts.push(EvalActSubmission {
            market_actions: Vec::new(),
            pass_b_actions: Vec::new(),
            bundle: SealedOrderBundle {
                converter_runs: conv.clone(),
                ..SealedOrderBundle::default()
            },
        });
    }

    let projs = ranked_project_batches(obs);
    for p in &projs {
        drafts.push(EvalActSubmission {
            market_actions: Vec::new(),
            pass_b_actions: Vec::new(),
            bundle: SealedOrderBundle {
                project_deliveries: vec![p.clone()],
                ..SealedOrderBundle::default()
            },
        });
    }
    if !conv.is_empty() || !projs.is_empty() {
        drafts.push(EvalActSubmission {
            market_actions: Vec::new(),
            pass_b_actions: Vec::new(),
            bundle: SealedOrderBundle {
                converter_runs: conv,
                project_deliveries: projs,
                ..SealedOrderBundle::default()
            },
        });
    }
    drafts
}

fn gen_debt(obs: &SeatObservation, config: &ScorerConfig) -> Vec<EvalActSubmission> {
    let seat_idx = usize::from(obs.seat);
    let (Some(seat_state), Some(seat_cfg)) =
        (obs.seats.get(seat_idx), obs.spec.seats.get(seat_idx))
    else {
        return Vec::new();
    };
    if seat_state.cred_debt <= 0 {
        return Vec::new();
    }
    let upkeep_good = AssetId::Refined(seat_cfg.upkeep_good);
    let upkeep_parity = obs.parities.asset_parity(upkeep_good).max(1);
    let reserve = mul_units_parity(
        seat_cfg
            .upkeep_amount
            .saturating_mul(i64::from(config.upkeep_cover_turns)),
        upkeep_parity,
    );
    let spare = seat_state.cred.saturating_sub(reserve).max(0);
    let mut drafts = Vec::new();
    for &bps in &config.debt_repayment_grid_bps {
        let repay = ((i128::from(spare) * i128::from(bps)) / BPS_SCALE) as Units;
        let repay = repay.min(seat_state.cred_debt);
        if repay > 0 {
            drafts.push(EvalActSubmission {
                market_actions: Vec::new(),
                pass_b_actions: Vec::new(),
                bundle: SealedOrderBundle {
                    debt_repayment_cred: repay,
                    ..SealedOrderBundle::default()
                },
            });
        }
    }
    drafts
}

fn build_single_dropped_variant(
    obs: &SeatObservation,
    base: &EvalActSubmission,
) -> Option<EvalActSubmission> {
    let seat_idx = usize::from(obs.seat);
    let seat_cfg = obs.spec.seats.get(seat_idx)?;
    let upkeep_good = AssetId::Refined(seat_cfg.upkeep_good);
    let mut variants: Vec<(Units, String, EvalActSubmission)> = Vec::new();

    for (idx, act) in base.market_actions.iter().enumerate() {
        let mut copy = base.clone();
        copy.market_actions.remove(idx);
        if !is_legal_synthetic(obs, &copy) {
            continue;
        }
        let risk = match act {
            MarketAction::PostOffer { order, .. } if order.side == OfferSide::Buy => {
                checked_ceil_units_parity(order.amount, order.price_per_unit_cred).unwrap_or(0)
            }
            MarketAction::PostOffer { order, .. } if order.good == upkeep_good => {
                order.amount.saturating_mul(2)
            }
            MarketAction::TownImport { good, amount, .. } => {
                let p = obs.parities.asset_parity(*good).max(1);
                town_import_cost_cred(*amount, p, obs.spec.town_import_premium_bps).unwrap_or(0)
            }
            _ => 1,
        };
        if let Ok(cid) = candidate_id_for_submission(&copy) {
            variants.push((risk, cid, copy));
        }
    }

    if !base.bundle.project_deliveries.is_empty() {
        let mut copy = base.clone();
        copy.bundle.project_deliveries.clear();
        if is_legal_synthetic(obs, &copy)
            && let Ok(cid) = candidate_id_for_submission(&copy)
        {
            variants.push((i64::MAX / 2, cid, copy));
        }
    }
    if base.bundle.debt_repayment_cred > 0 {
        let mut copy = base.clone();
        let cred = copy.bundle.debt_repayment_cred;
        copy.bundle.debt_repayment_cred = 0;
        if is_legal_synthetic(obs, &copy)
            && let Ok(cid) = candidate_id_for_submission(&copy)
        {
            variants.push((cred, cid, copy));
        }
    }

    if DROP_TIE_BREAK_BY_CANDIDATE_ID {
        variants.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    }
    variants.into_iter().next().map(|(_, _, sub)| sub)
}

fn gen_composite_and_drops(
    obs: &SeatObservation,
    base_axes: &[&[EvalActSubmission]],
) -> Vec<EvalActSubmission> {
    let mut combined = EvalActSubmission::default();
    for axis in base_axes {
        for sub in *axis {
            let mut trial = combined.clone();
            trial
                .market_actions
                .extend(sub.market_actions.iter().cloned());
            trial
                .bundle
                .converter_runs
                .extend(sub.bundle.converter_runs.iter().cloned());
            trial
                .bundle
                .project_deliveries
                .extend(sub.bundle.project_deliveries.iter().cloned());
            trial
                .bundle
                .check_cashes
                .extend(sub.bundle.check_cashes.iter().copied());
            trial
                .bundle
                .escrow_finishes
                .extend(sub.bundle.escrow_finishes.iter().cloned());
            if trial.bundle.council_vote.is_none() {
                trial.bundle.council_vote = sub.bundle.council_vote.clone();
            }
            trial
                .bundle
                .charter_bids
                .extend(sub.bundle.charter_bids.iter().cloned());
            trial.bundle.debt_repayment_cred = trial
                .bundle
                .debt_repayment_cred
                .max(sub.bundle.debt_repayment_cred);
            if is_legal_synthetic(obs, &trial) {
                combined = trial;
                break;
            }
        }
    }
    let mut drafts = Vec::new();
    if combined != EvalActSubmission::default() && is_legal_synthetic(obs, &combined) {
        drafts.push(combined.clone());
        let mut cursor = combined;
        while let Some(dropped) = build_single_dropped_variant(obs, &cursor) {
            if dropped == EvalActSubmission::default() {
                break;
            }
            drafts.push(dropped.clone());
            cursor = dropped;
        }
    }
    drafts
}

fn scale_submission(sub: &EvalActSubmission, scale_bps: u16) -> EvalActSubmission {
    let s = i128::from(scale_bps);
    let mut scaled = sub.clone();
    for act in &mut scaled.market_actions {
        match act {
            MarketAction::PostOffer { order, .. } => {
                order.amount = ((i128::from(order.amount) * s) / BPS_SCALE).max(1) as Units;
            }
            MarketAction::TownImport { amount, .. } => {
                *amount = ((i128::from(*amount) * s) / BPS_SCALE).max(1) as Units;
            }
            _ => {}
        }
    }
    for cr in &mut scaled.bundle.converter_runs {
        let r = ((i128::from(cr.runs) * s) / BPS_SCALE).max(1);
        cr.runs = u32::try_from(r).unwrap_or(1);
    }
    for pr in &mut scaled.bundle.project_deliveries {
        let b = ((i128::from(pr.batches) * s) / BPS_SCALE).max(1);
        pr.batches = u32::try_from(b).unwrap_or(1);
    }
    if scaled.bundle.debt_repayment_cred > 0 {
        scaled.bundle.debt_repayment_cred =
            ((i128::from(scaled.bundle.debt_repayment_cred) * s) / BPS_SCALE).max(1) as Units;
    }
    scaled
}

fn gen_scaled(
    obs: &SeatObservation,
    config: &ScorerConfig,
    seed_pool: &[EvalActSubmission],
) -> Vec<EvalActSubmission> {
    let mut drafts = Vec::new();
    for &scale_bps in &config.quantity_scales_bps {
        if scale_bps == 0 || scale_bps == 10_000 {
            continue;
        }
        for sub in seed_pool.iter().take(6) {
            let scaled = scale_submission(sub, scale_bps);
            if scaled != *sub && is_legal_synthetic(obs, &scaled) {
                drafts.push(scaled);
            }
        }
    }
    drafts
}

/// Generates the frozen arm-independent candidate set (`passive`, `ClubBot`, optional cleared-pass-B
/// copy, and up to `config.k_synth_max` legal synthetic drafts) strictly from `obs`, `own_mandate`,
/// and `config` (`§3.1` sources 1 & 4, `§5.2`).
pub fn generate_arm_independent_candidates(
    obs: &SeatObservation,
    own_mandate: &PrivateMandate,
    config: &ScorerConfig,
    bot_tie_seed: u64,
) -> Result<Vec<BlindCandidate>, ScorerError> {
    config.validate()?;

    let passive = EvalActSubmission::default();
    let club_bot_decision = ClubBot::new(bot_tie_seed).decide_full_turn(obs, own_mandate);
    let club_bot = submission_from_club_bot(club_bot_decision);

    let cleared_pass_b_drafts = if config.include_pass_b_cleared_club_bot {
        let cleared = EvalActSubmission {
            market_actions: club_bot.market_actions.clone(),
            pass_b_actions: Vec::new(),
            bundle: club_bot.bundle.clone(),
        };
        vec![cleared]
    } else {
        Vec::new()
    };
    let claims_drafts = gen_claims_and_council(obs);
    let upkeep_drafts = gen_upkeep_and_converter_imports(obs, config);
    let cross_drafts = gen_cross_and_quotes(obs, config);
    let batch_drafts = gen_batches(obs);
    let debt_drafts = gen_debt(obs, config);
    let composite_drafts = gen_composite_and_drops(
        obs,
        &[
            &claims_drafts,
            &upkeep_drafts,
            &cross_drafts,
            &batch_drafts,
            &debt_drafts,
        ],
    );
    let mut scale_seed_pool = Vec::new();
    scale_seed_pool.extend(composite_drafts.iter().take(2).cloned());
    scale_seed_pool.extend(batch_drafts.iter().take(2).cloned());
    scale_seed_pool.extend(cross_drafts.iter().take(2).cloned());
    let scaled_drafts = gen_scaled(obs, config, &scale_seed_pool);

    let buckets: BTreeMap<&str, Vec<EvalActSubmission>> = BTreeMap::from([
        ("cleared_pass_b", cleared_pass_b_drafts),
        ("claims_council", claims_drafts),
        ("upkeep_inputs", upkeep_drafts),
        ("cross_quotes", cross_drafts),
        ("batches", batch_drafts),
        ("debt", debt_drafts),
        ("composite_drops", composite_drafts),
        ("scaled", scaled_drafts),
    ]);

    let mut guaranteed_ids = BTreeSet::new();
    let passive_id = candidate_id_for_submission(&passive)?;
    let club_bot_id = candidate_id_for_submission(&club_bot)?;
    guaranteed_ids.insert(passive_id.clone());
    guaranteed_ids.insert(club_bot_id.clone());

    let mut selected_synth_ids = BTreeSet::new();
    let mut synth_candidates = Vec::with_capacity(config.k_synth_max);

    for slot in &config.synth_slot_fill_order {
        if synth_candidates.len() >= config.k_synth_max {
            break;
        }
        let Some(raw_drafts) = buckets.get(slot.as_str()) else {
            continue;
        };
        let sorted_entries = to_sorted_bucket_entries(obs, raw_drafts.clone())?;
        for (cid, sub) in sorted_entries {
            if synth_candidates.len() >= config.k_synth_max {
                break;
            }
            if guaranteed_ids.contains(&cid) || !selected_synth_ids.insert(cid.clone()) {
                continue;
            }
            synth_candidates.push(BlindCandidate {
                candidate_id: cid,
                in_span: true,
                is_arm_independent: true,
                submission: sub,
            });
        }
    }

    let mut all = Vec::with_capacity(2 + synth_candidates.len());
    all.push(BlindCandidate {
        candidate_id: passive_id,
        in_span: true,
        is_arm_independent: true,
        submission: passive,
    });
    all.push(BlindCandidate {
        candidate_id: club_bot_id,
        in_span: true,
        is_arm_independent: true,
        submission: club_bot,
    });
    all.extend(synth_candidates);

    dedupe_candidates(all)
}

/// Verifies the content-addressed checkpoint pair, forks `EvalHost`, generates the arm-independent
/// candidate set for `focal_seat`, and returns a validated [`BlindScoringRequest`] with empty
/// `talk_commitments` (`§3.1`, `§8.1`).
#[allow(clippy::too_many_arguments)]
pub fn build_arm_independent_request(
    save_path: &Path,
    seal_path: &Path,
    expected_save_sha256: &str,
    expected_seal_sha256: &str,
    focal_seat: u8,
    state_id: &str,
    config: &ScorerConfig,
    scorer_seed: u64,
) -> Result<BlindScoringRequest, ScorerError> {
    config.validate()?;
    validate_state_id(state_id)?;
    if !is_valid_sha256_hex(expected_save_sha256) {
        return Err(ScorerError::InvalidHexDigest {
            field: "save_sha256",
            value: expected_save_sha256.to_owned(),
        });
    }
    if !is_valid_sha256_hex(expected_seal_sha256) {
        return Err(ScorerError::InvalidHexDigest {
            field: "seal_sha256",
            value: expected_seal_sha256.to_owned(),
        });
    }
    if focal_seat >= EVAL_TABLE_SEAT_COUNT {
        return Err(ScorerError::InvalidFocalSeat {
            focal_seat,
            seat_count: EVAL_TABLE_SEAT_COUNT,
        });
    }

    let (host, pair_sha) = verify_and_fork_checkpoint(
        save_path,
        seal_path,
        expected_save_sha256,
        expected_seal_sha256,
        None,
    )?;
    let seat_count = host.seat_count();
    if seat_count != EVAL_TABLE_SEAT_COUNT {
        return Err(ScorerError::InvalidCheckpointState(format!(
            "expected {EVAL_TABLE_SEAT_COUNT}-seat Evaluation table, got {seat_count}"
        )));
    }
    let focal_obs = host
        .observe_seat(Seat(focal_seat))
        .map_err(|e| ScorerError::Engine(format!("observe focal seat: {e}")))?;
    let focal_mandate = focal_obs.own_mandate().copied().ok_or_else(|| {
        ScorerError::Engine("focal SeatObservation omitted own_mandate".to_owned())
    })?;

    let candidates =
        generate_arm_independent_candidates(&focal_obs, &focal_mandate, config, scorer_seed)?;
    let req = BlindScoringRequest {
        candidates,
        focal_seat,
        pre_act_pair_sha256: pair_sha,
        pre_act_save_sha256: expected_save_sha256.to_owned(),
        pre_act_seal_sha256: expected_seal_sha256.to_owned(),
        schema_version: BLIND_SCORING_SCHEMA_VERSION,
        scorer_config_sha256: config.sha256_hex()?,
        scorer_seed,
        state_id: state_id.to_owned(),
        talk_commitments: Vec::new(),
    };
    req.validate(config)?;
    Ok(req)
}
