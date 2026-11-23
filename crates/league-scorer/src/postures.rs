//! Counterfactual peer postures (`§4.2`) and talk-commitment compliance (`§4.3`).

use std::collections::BTreeMap;

use league_core::{
    AiPreviewRequest, AssetId, BPS_SCALE, ClubBot, ClubBotDecision, EvalActSubmission,
    MarketAction, OfferProposal, OfferSide, PrivateMandate, SealedOrderBundle, SeatObservation,
    TurnPhase, Units, WhatIfPreview, checked_ceil_units_parity, mul_units_parity,
    preview_seat_bundle,
};
use sha2::{Digest, Sha256};

use crate::error::ScorerError;
use crate::types::{ExtractedTalkCommitment, PostureKind, ScorerConfig, to_sorted_compact_json};

const HONOUR_PPM_SCALE: u64 = 1_000_000;

/// Converts [`ClubBotDecision`] into an [`EvalActSubmission`].
#[must_use]
pub fn submission_from_club_bot(decision: ClubBotDecision) -> EvalActSubmission {
    EvalActSubmission {
        market_actions: decision.pass_a_actions,
        pass_b_actions: decision.pass_b_actions,
        bundle: decision.bundle,
    }
}

/// Runs `preview_seat_bundle` against `obs.to_public_preview_state()` for legality and upkeep checks.
#[must_use]
pub fn preview_submission(
    obs: &SeatObservation,
    market_actions: &[MarketAction],
    bundle: &SealedOrderBundle,
) -> Option<WhatIfPreview> {
    let pub_state = obs.to_public_preview_state();
    let req = AiPreviewRequest {
        seat: obs.seat,
        market_actions: market_actions.to_vec(),
        bundle: bundle.clone(),
    };
    preview_seat_bundle(&pub_state, &req).ok()
}

/// Iteratively removes any Phase-1 actions rejected by `preview_seat_bundle`.
#[must_use]
pub fn prune_refused_market_actions(
    obs: &SeatObservation,
    mut actions: Vec<MarketAction>,
    bundle: &SealedOrderBundle,
) -> Vec<MarketAction> {
    for _ in 0..=actions.len() {
        if actions.is_empty() {
            break;
        }
        let Some(preview) = preview_submission(obs, &actions, bundle) else {
            return Vec::new();
        };
        if preview.refused_market_actions.is_empty() {
            return actions;
        }
        let mut refused_indices: Vec<usize> = preview
            .refused_market_actions
            .iter()
            .map(|r| r.action_index)
            .collect();
        refused_indices.sort_unstable();
        refused_indices.dedup();
        if refused_indices.is_empty() {
            break;
        }
        for idx in refused_indices.into_iter().rev() {
            if idx < actions.len() {
                actions.remove(idx);
            }
        }
    }
    actions
}

/// Maps scenario index `m` deterministically to its counterfactual peer [`PostureKind`] (`§4.2`, `§5.2`).
#[must_use]
pub fn posture_for_scenario(config: &ScorerConfig, scenario_m: usize) -> PostureKind {
    config.posture_schedule[scenario_m % config.posture_schedule.len()]
}

/// Sorts and deduplicates [`ExtractedTalkCommitment`] entries by canonical compact JSON (`§4.3`).
pub fn canonicalize_talk_commitments(
    commitments: &[ExtractedTalkCommitment],
) -> Result<Vec<ExtractedTalkCommitment>, ScorerError> {
    let mut keyed: Vec<(Vec<u8>, ExtractedTalkCommitment)> = Vec::with_capacity(commitments.len());
    for item in commitments {
        keyed.push((to_sorted_compact_json(item)?, item.clone()));
    }
    keyed.sort_by(|(a, _), (b, _)| a.cmp(b));
    keyed.dedup_by(|(a, _), (b, _)| a == b);
    Ok(keyed.into_iter().map(|(_, item)| item).collect())
}

fn sha256_u64_honour(
    scorer_seed: u64,
    pre_act_pair_sha256: &str,
    day: u8,
    scenario_m: usize,
    canonical_commitment: &[u8],
    ctr: u64,
) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(b"league-scorer/talk-honour/v1|");
    hasher.update(scorer_seed.to_le_bytes());
    hasher.update(pre_act_pair_sha256.as_bytes());
    hasher.update([day]);
    hasher.update((scenario_m as u64).to_le_bytes());
    hasher.update((canonical_commitment.len() as u64).to_le_bytes());
    hasher.update(canonical_commitment);
    hasher.update(ctr.to_le_bytes());
    let digest: [u8; 32] = hasher.finalize().into();
    let mut buf = [0_u8; 8];
    buf.copy_from_slice(&digest[..8]);
    u64::from_le_bytes(buf)
}

/// Draws `Bernoulli(p_honour)` deterministically with pure integer arithmetic in `[0, 1_000_000)` (`§4.3`).
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn draw_honour_commitment(
    scorer_seed: u64,
    pre_act_pair_sha256: &str,
    day: u8,
    scenario_m: usize,
    commitment_canonical_json: &[u8],
    p_honour: f64,
) -> bool {
    if p_honour <= 0.0 || !p_honour.is_finite() {
        return false;
    }
    if p_honour >= 1.0 {
        return true;
    }
    let threshold_ppm = (p_honour * (HONOUR_PPM_SCALE as f64)).round() as u64;
    if threshold_ppm == 0 {
        return false;
    }
    if threshold_ppm >= HONOUR_PPM_SCALE {
        return true;
    }
    let scale = u128::from(HONOUR_PPM_SCALE);
    let limit = ((1_u128 << 64) / scale) * scale;
    let mut ctr = 0_u64;
    loop {
        let sample = u128::from(sha256_u64_honour(
            scorer_seed,
            pre_act_pair_sha256,
            day,
            scenario_m,
            commitment_canonical_json,
            ctr,
        ));
        ctr = ctr.wrapping_add(1);
        if sample < limit {
            let draw_ppm = (sample % scale) as u64;
            return draw_ppm < threshold_ppm;
        }
    }
}

fn free_commodity_balance(obs: &SeatObservation, good: AssetId) -> Units {
    let Some(state) = obs.seats.get(usize::from(obs.seat)) else {
        return 0;
    };
    match good {
        AssetId::Cred => state.cred,
        AssetId::Raw(g) => state.raw.get(usize::from(g)).copied().unwrap_or(0),
        AssetId::Refined(m) => state.refined.get(usize::from(m)).copied().unwrap_or(0),
    }
}

fn raw_recipe_reserve(obs: &SeatObservation, raw_idx: u8) -> Units {
    let Some(seat_cfg) = obs.spec.seats.get(usize::from(obs.seat)) else {
        return 0;
    };
    seat_cfg
        .converters
        .iter()
        .filter_map(|r| {
            r.inputs
                .iter()
                .find(|&&(g, _)| g == raw_idx)
                .map(|&(_, amt)| amt.saturating_mul(i64::from(r.max_runs_per_turn)))
        })
        .sum()
}

fn build_active_turn(
    mut base: EvalActSubmission,
    obs: &SeatObservation,
    config: &ScorerConfig,
) -> EvalActSubmission {
    if obs.phase != TurnPhase::TalkAndMarket || obs.cancel_locked {
        return base;
    }
    let seat_idx = usize::from(obs.seat);
    let (Some(seat_state), Some(seat_cfg)) =
        (obs.seats.get(seat_idx), obs.spec.seats.get(seat_idx))
    else {
        return base;
    };
    if seat_state.wash_frozen_until_turn >= obs.turn {
        return base;
    }

    // Account for balances already committed by `ClubBot`'s Pass-A sells.
    let mut planned_sell_by_good: BTreeMap<AssetId, Units> = BTreeMap::new();
    for act in &base.market_actions {
        if let MarketAction::PostOffer { order, .. } = act
            && order.side == OfferSide::Sell
        {
            *planned_sell_by_good.entry(order.good).or_insert(0) += order.amount;
        }
    }

    // Track public best non-self bid (with size) and ask per commodity.
    let mut best_bid: BTreeMap<AssetId, (Units, Units)> = BTreeMap::new();
    let mut best_ask: BTreeMap<AssetId, Units> = BTreeMap::new();
    for offer in obs.offers.values() {
        if offer.maker == obs.seat || offer.domain_id.is_some() || offer.remaining_units <= 0 {
            continue;
        }
        match offer.side {
            OfferSide::Buy => {
                best_bid
                    .entry(offer.good)
                    .and_modify(|(best_p, best_u)| {
                        if offer.price_per_unit_cred > *best_p {
                            *best_p = offer.price_per_unit_cred;
                            *best_u = offer.remaining_units;
                        } else if offer.price_per_unit_cred == *best_p {
                            *best_u = best_u.saturating_add(offer.remaining_units);
                        }
                    })
                    .or_insert((offer.price_per_unit_cred, offer.remaining_units));
            }
            OfferSide::Sell => {
                best_ask
                    .entry(offer.good)
                    .and_modify(|best_p| {
                        if offer.price_per_unit_cred < *best_p {
                            *best_p = offer.price_per_unit_cred;
                        }
                    })
                    .or_insert(offer.price_per_unit_cred);
            }
        }
    }

    let upkeep_good = AssetId::Refined(seat_cfg.upkeep_good);
    let upkeep_reserve = seat_cfg
        .upkeep_amount
        .saturating_mul(i64::from(config.upkeep_cover_turns));
    let n_seats = u8::try_from(obs.spec.seats.len()).unwrap_or(0);
    let mut goods = Vec::with_capacity(usize::from(n_seats) * 2);
    for g in 0..n_seats {
        goods.push(AssetId::Raw(g));
    }
    for m in 0..n_seats {
        goods.push(AssetId::Refined(m));
    }

    let threshold_bps = i128::from(config.parity_spread_threshold_bps);
    for good in goods {
        let Some(&(bid_price, bid_units)) = best_bid.get(&good) else {
            continue;
        };
        let parity = obs.parities.asset_parity(good).max(1);
        let spread_bps = match best_ask.get(&good) {
            Some(&ask_price) => {
                (i128::from(ask_price.saturating_sub(bid_price)) * BPS_SCALE) / i128::from(parity)
            }
            None => i128::MAX,
        };
        if spread_bps <= threshold_bps {
            continue;
        }
        let held = free_commodity_balance(obs, good);
        let reserve = match good {
            AssetId::Refined(m) if AssetId::Refined(m) == upkeep_good => upkeep_reserve,
            AssetId::Raw(g) => raw_recipe_reserve(obs, g),
            _ => 0,
        };
        let already_selling = planned_sell_by_good.get(&good).copied().unwrap_or(0);
        let excess = held.saturating_sub(reserve).saturating_sub(already_selling);
        let cross_amount = excess.min(bid_units);
        if cross_amount > 0 && bid_price > 0 {
            *planned_sell_by_good.entry(good).or_insert(0) += cross_amount;
            base.market_actions.push(MarketAction::PostOffer {
                seat: obs.seat,
                order: OfferProposal {
                    side: OfferSide::Sell,
                    good,
                    amount: cross_amount,
                    price_per_unit_cred: bid_price,
                    domain_id: None,
                    is_hybrid: false,
                },
            });
        }
    }

    base.market_actions = prune_refused_market_actions(obs, base.market_actions, &base.bundle);
    base
}

fn build_conservative_turn(
    mut base: EvalActSubmission,
    obs: &SeatObservation,
    config: &ScorerConfig,
) -> EvalActSubmission {
    let seat_idx = usize::from(obs.seat);
    let (Some(seat_state), Some(seat_cfg)) =
        (obs.seats.get(seat_idx), obs.spec.seats.get(seat_idx))
    else {
        return base;
    };

    let upkeep_good = AssetId::Refined(seat_cfg.upkeep_good);
    let upkeep_parity = obs.parities.asset_parity(upkeep_good).max(1);
    let upkeep_units_floor = seat_cfg
        .upkeep_amount
        .saturating_mul(i64::from(config.upkeep_cover_turns));
    let upkeep_cred_floor = mul_units_parity(upkeep_units_floor, upkeep_parity).max(0);

    let mut cred_avail = seat_state.cred;
    let mut upkeep_avail = free_commodity_balance(obs, upkeep_good);

    let mut filtered_pass_a = Vec::with_capacity(base.market_actions.len());
    for action in base.market_actions {
        match &action {
            MarketAction::CancelOffer { offer_id, .. } => {
                if let Some(offer) = obs.offers.get(offer_id)
                    && offer.maker == obs.seat
                {
                    match offer.side {
                        OfferSide::Buy => {
                            cred_avail = cred_avail.saturating_add(offer.locked_cred);
                        }
                        OfferSide::Sell if offer.good == upkeep_good => {
                            upkeep_avail = upkeep_avail.saturating_add(offer.remaining_units);
                        }
                        OfferSide::Sell => {}
                    }
                }
                filtered_pass_a.push(action);
            }
            MarketAction::PostOffer { order, .. } => match order.side {
                OfferSide::Buy => {
                    let Ok(lock) =
                        checked_ceil_units_parity(order.amount, order.price_per_unit_cred)
                    else {
                        continue;
                    };
                    if order.good == upkeep_good {
                        // Bidding for upkeep itself advances upkeep security as long as free CRED stays >= 0.
                        if cred_avail >= lock {
                            cred_avail -= lock;
                            filtered_pass_a.push(action);
                        }
                    } else if cred_avail.saturating_sub(lock) >= upkeep_cred_floor {
                        cred_avail -= lock;
                        filtered_pass_a.push(action);
                    }
                }
                OfferSide::Sell => {
                    if order.good == upkeep_good {
                        if upkeep_avail.saturating_sub(order.amount) >= upkeep_units_floor {
                            upkeep_avail -= order.amount;
                            filtered_pass_a.push(action);
                        }
                    } else {
                        filtered_pass_a.push(action);
                    }
                }
            },
            _ => filtered_pass_a.push(action),
        }
    }

    // Also spare `upkeep_cred_floor` when voluntarily repaying CRED debt.
    let excess_cred = seat_state.cred.saturating_sub(upkeep_cred_floor).max(0);
    base.bundle.debt_repayment_cred = base.bundle.debt_repayment_cred.min(excess_cred);
    base.market_actions = prune_refused_market_actions(obs, filtered_pass_a, &base.bundle);
    base
}

/// Computes a deterministic counterfactual peer turn submission under `posture` (`§4.2`).
#[must_use]
pub fn decide_peer_turn(
    posture: PostureKind,
    bot_seed: u64,
    obs: &SeatObservation,
    mandate: &PrivateMandate,
    config: &ScorerConfig,
) -> EvalActSubmission {
    let base = submission_from_club_bot(ClubBot::new(bot_seed).decide_full_turn(obs, mandate));
    match posture {
        PostureKind::Standard => base,
        PostureKind::Active => build_active_turn(base, obs, config),
        PostureKind::Conservative => build_conservative_turn(base, obs, config),
    }
}

/// Applies probabilistic `Bernoulli(p_honour)` talk-commitment compliance to a peer seat's
/// turn-`day` submission, prepending honoured actions and pruning via `preview_seat_bundle` (`§4.3`).
pub fn apply_talk_commitments(
    obs: &SeatObservation,
    mut sub: EvalActSubmission,
    canonical_commitments: &[ExtractedTalkCommitment],
    scorer_seed: u64,
    pre_act_pair_sha256: &str,
    scenario_m: usize,
    p_honour: f64,
) -> Result<EvalActSubmission, ScorerError> {
    if obs.phase != TurnPhase::TalkAndMarket || obs.cancel_locked {
        return Ok(sub);
    }
    let mut honoured = Vec::new();
    for commitment in canonical_commitments {
        if commitment.proposer_seat != obs.seat {
            continue;
        }
        let key_bytes = to_sorted_compact_json(commitment)?;
        if draw_honour_commitment(
            scorer_seed,
            pre_act_pair_sha256,
            obs.turn,
            scenario_m,
            &key_bytes,
            p_honour,
        ) {
            honoured.push(commitment.offered_action.clone());
        }
    }
    if honoured.is_empty() {
        return Ok(sub);
    }
    honoured.append(&mut sub.market_actions);
    sub.market_actions = prune_refused_market_actions(obs, honoured, &sub.bundle);
    Ok(sub)
}
