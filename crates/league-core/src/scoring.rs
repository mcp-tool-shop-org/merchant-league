//! Cardinal Net Equity scoring, fundamental shadow parities, Potential-Based Reward
//! Shaping (PBRS), Turn-1 Private Mandate evaluation, and terminal scoreboards.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::commit::{MandateKind, MandateReveal, standings_tie_key, verify_mandate_reveal};
use crate::contracts::{CheckStatus, CredOffer, EscrowStatus, OfferSide};
use crate::engine::EngineState;
use crate::types::{
    AssetId, BPS_SCALE, Bps, CharterRole, CreditTier, EngineError, MANDATE_STARTING_NW_FLOOR_BPS,
    MAX_MANDATE_BONUS_EQUITY, MIN_MANDATE_BONUS_EQUITY, Seat, TableSpec, Turn, UNIT_SCALE,
    UNSOLD_EXCESS_PARITY_BPS, Units, half_up_bps, mul_units_parity, scale_bps_signed,
};

/// Fundamental shadow parities derived deterministically from table converter stoichiometry
/// so thin late-turn DEX quotes cannot manipulate mark-to-market valuations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FundamentalParities {
    /// Numeraire `CRED` parity per unit (`1_000` milli-CRED per `1.000` unit).
    pub cred_per_unit: Units,
    /// Per-seat raw commodity fundamental parity (`Raw(g)` -> milli-CRED per `1_000` units).
    pub raw_per_unit: Vec<Units>,
    /// Per-seat refined commodity fundamental parity (`Refined(m)` -> milli-CRED per `1_000` units).
    pub refined_per_unit: Vec<Units>,
}

impl FundamentalParities {
    /// Derives [`FundamentalParities`] deterministically from `spec` using primary converter yields.
    #[must_use]
    pub fn from_spec(spec: &TableSpec) -> Self {
        let n = usize::from(spec.seat_count);
        let refined_per_unit: Vec<Units> =
            spec.seats.iter().map(|s| s.refined_project_value).collect();
        let mean_refined = if n > 0 {
            refined_per_unit.iter().map(|&v| v as i128).sum::<i128>() / (n as i128)
        } else {
            UNIT_SCALE as i128
        };

        let mut num = vec![0_i128; n];
        let mut den = vec![0_i128; n];
        for seat in &spec.seats {
            for conv in &seat.converters {
                if conv.is_secondary {
                    continue;
                }
                let total_in: i128 = conv.inputs.iter().map(|&(_, amt)| amt as i128).sum();
                if total_in <= 0 {
                    continue;
                }
                let out_ref = refined_per_unit[usize::from(conv.output_good)] as i128;
                let unit_parity = ((conv.output_amount as i128) * out_ref) / total_in;
                for &(raw_g, amt) in &conv.inputs {
                    let g = usize::from(raw_g);
                    if g < n && amt > 0 {
                        num[g] += unit_parity * (amt as i128);
                        den[g] += amt as i128;
                    }
                }
            }
        }

        let raw_per_unit = (0..n)
            .map(|g| {
                if den[g] > 0 {
                    (num[g] / den[g]) as Units
                } else {
                    mean_refined as Units
                }
            })
            .collect();

        Self {
            cred_per_unit: UNIT_SCALE,
            raw_per_unit,
            refined_per_unit,
        }
    }

    /// Returns the fundamental parity (milli-CRED per `1_000` units) for `asset`.
    #[must_use]
    pub fn asset_parity(&self, asset: AssetId) -> Units {
        match asset {
            AssetId::Cred => self.cred_per_unit,
            AssetId::Raw(g) => self.raw_per_unit.get(usize::from(g)).copied().unwrap_or(0),
            AssetId::Refined(m) => self
                .refined_per_unit
                .get(usize::from(m))
                .copied()
                .unwrap_or(0),
        }
    }
}

/// Computes Seat `seat`'s fundamental mark-to-market Net Asset Equity `Phi_i(s_t)`.
#[must_use]
pub fn compute_net_asset_equity(state: &EngineState, seat: u8) -> Units {
    let idx = usize::from(seat);
    let Some(seat_state) = state.seats.get(idx) else {
        return 0;
    };
    let n = state.parities.raw_per_unit.len();
    let mut equity = seat_state.cred as i128;

    let mut total_raw: Vec<i128> = seat_state.raw.iter().map(|&v| v as i128).collect();
    let mut total_refined: Vec<i128> = seat_state.refined.iter().map(|&v| v as i128).collect();

    for offer in state.offers.values() {
        if offer.maker != seat || offer.remaining_units <= 0 {
            continue;
        }
        match offer.side {
            OfferSide::Buy => equity += offer.locked_cred as i128,
            OfferSide::Sell => match offer.good {
                AssetId::Raw(g) if usize::from(g) < n => {
                    total_raw[usize::from(g)] += offer.remaining_units as i128;
                }
                AssetId::Refined(m) if usize::from(m) < n => {
                    total_refined[usize::from(m)] += offer.remaining_units as i128;
                }
                _ => {}
            },
        }
    }

    for escrow in state.escrows.values() {
        if escrow.sender == seat && escrow.status == EscrowStatus::Locked {
            equity += escrow.amount_cred as i128;
        }
    }
    for dual in state.dual_escrows.values() {
        if dual.status == EscrowStatus::Locked {
            if dual.buyer == seat {
                equity += dual.buyer_bond_cred as i128;
            }
            if dual.seller == seat {
                equity += dual.seller_bond_cred as i128;
            }
        }
    }

    for g in 0..n {
        let raw_units = total_raw[g].clamp(i64::MIN as i128, i64::MAX as i128) as Units;
        let ref_units = total_refined[g].clamp(i64::MIN as i128, i64::MAX as i128) as Units;
        equity += mul_units_parity(raw_units, state.parities.raw_per_unit[g]) as i128;
        equity += mul_units_parity(ref_units, state.parities.refined_per_unit[g]) as i128;
    }

    equity -= seat_state.cred_debt as i128;

    for check in state.checks.values() {
        if check.drawer == seat && check.status == CheckStatus::Open {
            let liab = mul_units_parity(check.send_max, state.parities.asset_parity(check.asset));
            equity -= liab as i128;
        }
    }

    equity.clamp(i64::MIN as i128, i64::MAX as i128) as Units
}

/// Computes total shaping potential `Psi_i(s_t) = ProjectEquity_i(s_t) + Phi_i(s_t)`.
#[must_use]
pub fn compute_potential(state: &EngineState, seat: u8) -> Units {
    let idx = usize::from(seat);
    let proj = state.seats.get(idx).map_or(0, |s| s.project_equity);
    proj.saturating_add(compute_net_asset_equity(state, seat))
}

/// Computes dense Potential-Based Reward Shaping step `F_i = gamma * Psi_{t+1} - Psi_t`.
#[must_use]
pub const fn compute_pbrs_reward(
    prev_potential: Units,
    next_potential: Units,
    gamma_bps: Bps,
) -> i64 {
    scale_bps_signed(next_potential, gamma_bps).saturating_sub(prev_potential)
}

fn seat_has_active_broker(state: &EngineState, seat: u8) -> bool {
    state.charters.iter().any(|c| {
        c.role == CharterRole::ClearinghouseBroker
            && c.holder == seat
            && c.expires_at_turn >= state.turn
    })
}

fn bid_executable_by_seller(state: &EngineState, seller: u8, bid: &CredOffer) -> bool {
    if bid.side != OfferSide::Buy || bid.remaining_units <= 0 || bid.maker == seller {
        return false;
    }
    let Some(maker_state) = state.seats.get(usize::from(bid.maker)) else {
        return false;
    };
    if maker_state.wash_frozen_until_turn >= state.turn {
        return false;
    }
    match bid.domain_id {
        None => true,
        Some(dom_id) => {
            if !maker_state.credit_tier.may_access_prime_cartel() {
                return false;
            }
            let seller_broker = seat_has_active_broker(state, seller);
            let maker_hybrid_broker = bid.is_hybrid && seat_has_active_broker(state, bid.maker);
            let both_in_domain = state
                .seats
                .get(usize::from(seller))
                .is_some_and(|s| s.credit_tier.may_access_prime_cartel())
                && state
                    .domains
                    .iter()
                    .find(|d| d.domain_id == dom_id)
                    .is_some_and(|d| {
                        d.members.contains(&seller)
                            && (d.members.contains(&bid.maker) || maker_hybrid_broker)
                    });
            both_in_domain || seller_broker || maker_hybrid_broker
        }
    }
}

/// Computes `floor(units * parity * UNSOLD_EXCESS_PARITY_BPS / (UNIT_SCALE * BPS_SCALE))`
/// (`50%` of fixed base value / `parity` for `units`), applied once to all pooled non-winning and
/// uncovered units at Season Close.
#[must_use]
fn half_base_value_for_units(units: Units, parity: Units) -> Units {
    if units <= 0 || parity <= 0 {
        return 0;
    }
    let raw = (i128::from(units) * i128::from(parity) * i128::from(UNSOLD_EXCESS_PARITY_BPS))
        / (i128::from(UNIT_SCALE) * BPS_SCALE);
    raw.clamp(0, i128::from(Units::MAX)) as Units
}

/// Marks `units` of `good` held by `seat` at Season Close (`2026-11-16` final close-marking rule):
///
/// 1. **Trade prices do not count:** no traded price, volume, window, or qualification test enters
///    the close mark.
/// 2. **Standing bids first, compared against the unrounded `50%` floor:** units that executable
///    non-self resting `Buy` bids would buy are walked highest-bid-first (`offer_id` ascending on
///    price ties; `seat`'s own bids skipped). A slice of `take` units whose `bid_cred`
///    (`min(take * bid_price, locked_cred)`) strictly exceeds the unrounded `50%` base floor
///    (`bid_cred * UNIT_SCALE * BPS_SCALE > take * parity * UNSOLD_EXCESS_PARITY_BPS`) adds
///    `bid_cred`; any slice that does not beat that unrounded floor adds nothing during the walk
///    and instead joins the pooled floored unit count.
/// 3. **One pooled `50%` floor over the rest:** all non-winning bid slices and remaining uncovered
///    units are pooled together and valued once via [`half_base_value_for_units`] at
///    [`UNSOLD_EXCESS_PARITY_BPS`] (`50%`) of fixed base value (`parity`), so a holding with no
///    winning bid marks identically to a holding with no bids at all.
///
/// Note: each holder is valued against the full executable resting bid depth independently (so a
/// single resting bid can lift multiple holders); this is a known, accepted residual.
#[must_use]
pub fn mark_good_holding_at_close(
    state: &EngineState,
    seat: u8,
    good: AssetId,
    units: Units,
) -> Units {
    if units <= 0 {
        return 0;
    }
    let parity = state.parities.asset_parity(good).max(0);
    let mut bids: Vec<&CredOffer> = state
        .offers
        .values()
        .filter(|o| o.good == good && bid_executable_by_seller(state, seat, o))
        .collect();
    bids.sort_by(|a, b| {
        b.price_per_unit_cred
            .cmp(&a.price_per_unit_cred)
            .then_with(|| a.offer_id.cmp(&b.offer_id))
    });

    let mut rem_units = units;
    let mut floored_units: i128 = 0;
    let mut proceeds: i128 = 0;
    let scale = i128::from(UNIT_SCALE) * BPS_SCALE;
    for bid in bids {
        if rem_units <= 0 {
            break;
        }
        let take = rem_units.min(bid.remaining_units);
        let bid_cred = mul_units_parity(take, bid.price_per_unit_cred).min(bid.locked_cred);
        if i128::from(bid_cred) * scale
            > i128::from(take) * i128::from(parity) * i128::from(UNSOLD_EXCESS_PARITY_BPS)
        {
            proceeds += i128::from(bid_cred);
        } else {
            floored_units += i128::from(take);
        }
        rem_units -= take;
    }
    let pooled_floor_units =
        (floored_units + i128::from(rem_units.max(0))).clamp(0, i128::from(Units::MAX)) as Units;
    if pooled_floor_units > 0 {
        proceeds += i128::from(half_base_value_for_units(pooled_floor_units, parity));
    }
    proceeds.clamp(i64::MIN as i128, i64::MAX as i128) as Units
}

/// Computes `seat`'s terminal marked Net Asset Equity at Season Close (`2026-11-16` ruling),
/// valuing commodity holdings by [`mark_good_holding_at_close`] (executable non-self standing bids
/// floored at `50%` of base value first, and remaining units at `50%` of base value; trade prices
/// never enter) while keeping `CRED`, escrows, debt, and open check liabilities exact.
#[must_use]
pub fn compute_final_marked_net_asset_equity(state: &EngineState, seat: u8) -> Units {
    let idx = usize::from(seat);
    let Some(seat_state) = state.seats.get(idx) else {
        return 0;
    };
    let n = state.parities.raw_per_unit.len();
    let mut equity = seat_state.cred as i128;

    let mut total_raw: Vec<i128> = seat_state.raw.iter().map(|&v| v as i128).collect();
    let mut total_refined: Vec<i128> = seat_state.refined.iter().map(|&v| v as i128).collect();

    for offer in state.offers.values() {
        if offer.maker != seat || offer.remaining_units <= 0 {
            continue;
        }
        match offer.side {
            OfferSide::Buy => equity += offer.locked_cred as i128,
            OfferSide::Sell => match offer.good {
                AssetId::Raw(g) if usize::from(g) < n => {
                    total_raw[usize::from(g)] += offer.remaining_units as i128;
                }
                AssetId::Refined(m) if usize::from(m) < n => {
                    total_refined[usize::from(m)] += offer.remaining_units as i128;
                }
                _ => {}
            },
        }
    }

    for escrow in state.escrows.values() {
        if escrow.sender == seat && escrow.status == EscrowStatus::Locked {
            equity += escrow.amount_cred as i128;
        }
    }
    for dual in state.dual_escrows.values() {
        if dual.status == EscrowStatus::Locked {
            if dual.buyer == seat {
                equity += dual.buyer_bond_cred as i128;
            }
            if dual.seller == seat {
                equity += dual.seller_bond_cred as i128;
            }
        }
    }

    for g in 0..n {
        let raw_units = total_raw[g].clamp(i64::MIN as i128, i64::MAX as i128) as Units;
        let ref_units = total_refined[g].clamp(i64::MIN as i128, i64::MAX as i128) as Units;
        equity += i128::from(mark_good_holding_at_close(
            state,
            seat,
            AssetId::Raw(g as u8),
            raw_units,
        ));
        equity += i128::from(mark_good_holding_at_close(
            state,
            seat,
            AssetId::Refined(g as u8),
            ref_units,
        ));
    }

    equity -= seat_state.cred_debt as i128;

    for check in state.checks.values() {
        if check.drawer == seat && check.status == CheckStatus::Open {
            let liab = mul_units_parity(check.send_max, state.parities.asset_parity(check.asset));
            equity -= liab as i128;
        }
    }

    equity.clamp(i64::MIN as i128, i64::MAX as i128) as Units
}

/// Computes the table median over `net_worths` in integer milli-units.
///
/// For odd `N` (`N = 5`), returns the middle element `sorted[N / 2]`.
/// For even `N` (`N = 4, 6`), returns the half-up integer midpoint
/// `(sorted[N / 2 - 1] + sorted[N / 2] + 1).div_euclid(2)`.
#[must_use]
pub fn table_median_net_worth(net_worths: &[Units]) -> Units {
    if net_worths.is_empty() {
        return 0;
    }
    let mut sorted = net_worths.to_vec();
    sorted.sort_unstable();
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        sorted[mid]
    } else {
        let lo = i128::from(sorted[mid - 1]);
        let hi = i128::from(sorted[mid]);
        ((lo + hi + 1).div_euclid(2)).clamp(i64::MIN as i128, i64::MAX as i128) as Units
    }
}

/// Computes `W_0`, the table-median starting net worth (`median_j(spec.seats[j].initial_cred)`),
/// used as the `0.5 * W_0` floor inside Formula 3 mandate scaling (`Seed-28 Report` §6).
#[must_use]
pub fn compute_table_median_starting_net_worth(spec: &TableSpec) -> Units {
    let initial_nws: Vec<Units> = spec.seats.iter().map(|s| s.initial_cred).collect();
    table_median_net_worth(&initial_nws)
}

/// Computes `median_j(ProjectEquity_j + FinalMarkedNetAssetEquity_j)` across all seats in `state`.
#[must_use]
pub fn compute_table_median_pre_mandate_net_worth(state: &EngineState) -> Units {
    let nws: Vec<Units> = state
        .seats
        .iter()
        .enumerate()
        .map(|(idx, s)| {
            s.project_equity
                .saturating_add(compute_final_marked_net_asset_equity(state, idx as u8))
        })
        .collect();
    table_median_net_worth(&nws)
}

/// Converts `mandate_points_milli` (`3_000..=8_000` = `3..=8` mandate points) into terminal equity
/// milli-units against an already-floored `effective_median_nw`.
#[must_use]
pub const fn convert_mandate_points_to_equity_with_median(
    mandate_points_milli: Units,
    table_median_nw: Units,
    alpha_bps: Bps,
) -> Units {
    if mandate_points_milli <= 0 || table_median_nw <= 0 || alpha_bps == 0 {
        return 0;
    }
    let den = (UNIT_SCALE as i128) * BPS_SCALE;
    let num = (mandate_points_milli as i128) * (alpha_bps as i128) * (table_median_nw as i128);
    let rounded = (num + (den / 2)) / den;
    if rounded > Units::MAX as i128 {
        Units::MAX
    } else {
        rounded as Units
    }
}

/// Converts `mandate_points_milli` into terminal equity milli-units per `Seed-28 Report` §6
/// Formula 3 (`B_i = round_half_up(m_i * alpha * max(Med(W), 0.5 * W_0))`, `A-3`).
#[must_use]
pub const fn convert_mandate_points_to_equity_with_floor(
    mandate_points_milli: Units,
    table_median_nw: Units,
    starting_median_nw: Units,
    alpha_bps: Bps,
) -> Units {
    let floor_nw = half_up_bps(starting_median_nw, MANDATE_STARTING_NW_FLOOR_BPS);
    let effective_nw = if table_median_nw > floor_nw {
        table_median_nw
    } else {
        floor_nw
    };
    convert_mandate_points_to_equity_with_median(mandate_points_milli, effective_nw, alpha_bps)
}

/// Converts a card's mandate point value (`3_000..=8_000` milli-points) into terminal equity units
/// via `points * alpha * max(Med(W), 0.5 * W_0)` (`DEFAULT_MANDATE_ALPHA_BPS = 250` = `2.5%`).
#[must_use]
pub fn convert_mandate_points_to_equity(mandate_points_milli: Units, state: &EngineState) -> Units {
    let median_nw = compute_table_median_pre_mandate_net_worth(state);
    let w0 = compute_table_median_starting_net_worth(&state.spec);
    convert_mandate_points_to_equity_with_floor(
        mandate_points_milli,
        median_nw,
        w0,
        state.spec.mandate_alpha_bps,
    )
}

fn evaluate_mandate_points_milli(
    state: &EngineState,
    seat: u8,
    reveal: &MandateReveal,
) -> Option<Units> {
    let idx = usize::from(seat);
    let seat_state = state.seats.get(idx)?;
    if reveal.salt.is_degenerate() {
        return None;
    }
    let &dealt_commit = state.spec.dealt_mandate_commits.get(idx)?;
    if let Some(Some(logged_commit)) = state.mandate_commits.get(idx)
        && *logged_commit != dealt_commit
    {
        return None;
    }
    if !matches!(
        verify_mandate_reveal(state.table_id, Seat(seat), dealt_commit, reveal),
        Ok(true)
    ) {
        return None;
    }
    let card =
        state.spec.mandate_deck.iter().find(|c| {
            c.kind == reveal.mandate.kind && c.bonus_equity == reveal.mandate.bonus_equity
        })?;
    card.kind
        .validate_for_seat_count(state.spec.seat_count)
        .ok()?;
    if !(MIN_MANDATE_BONUS_EQUITY..=MAX_MANDATE_BONUS_EQUITY).contains(&card.bonus_equity) {
        return None;
    }
    let ok = match card.kind {
        MandateKind::MinProjectEquity { target } => seat_state.project_equity >= target,
        MandateKind::MinRefinedVariety {
            distinct_goods,
            min_each,
        } => {
            let count = seat_state
                .refined
                .iter()
                .filter(|&&v| v >= min_each)
                .count();
            count >= usize::from(distinct_goods)
        }
        MandateKind::MaintainCleanCredit { min_tier } => {
            credit_rank(seat_state.credit_tier) <= credit_rank(min_tier)
        }
        MandateKind::MinActivePartners { min_partners } => {
            let mut partners = BTreeSet::new();
            for (&(from, to, _), &vol) in &state.delivery_ledger {
                if vol > 0 {
                    if from == seat && to != seat {
                        partners.insert(to);
                    } else if to == seat && from != seat {
                        partners.insert(from);
                    }
                }
            }
            partners.len() >= usize::from(min_partners)
        }
    };
    ok.then(|| card.mandate_points_milli())
}

/// Evaluates whether `seat` satisfied its revealed Turn-1 Private Charter Mandate and returns
/// `(satisfied, awarded_bonus_equity)` scaled by `points * alpha * table_median_net_worth`.
#[must_use]
pub fn evaluate_mandate_bonus(
    state: &EngineState,
    seat: u8,
    reveal: &MandateReveal,
) -> (bool, Units) {
    match evaluate_mandate_points_milli(state, seat, reveal) {
        Some(pts_milli) => (true, convert_mandate_points_to_equity(pts_milli, state)),
        None => (false, 0),
    }
}

const fn credit_rank(tier: CreditTier) -> u8 {
    match tier {
        CreditTier::Aaa => 0,
        CreditTier::Bbb => 1,
        CreditTier::B => 2,
        CreditTier::D => 3,
    }
}

/// Per-seat breakdown at Season Close (`Turn T in 24..=32`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatScoreEntry {
    /// Seat index (`0..seat_count`).
    pub seat: u8,
    /// Irreversible Project Equity points burned via Tier-3 Syndicate Projects.
    pub project_equity: Units,
    /// Terminal marked Net Asset Equity `Phi_i(T)` (standing bids floored at `50%` of base value, rest at `50%` of base value).
    pub net_asset_equity: Units,
    /// Awarded Turn-1 Private Mandate bonus (`0` if unrevealed, invalid hash, or unmet).
    pub mandate_bonus: Units,
    /// Sum `project_equity + net_asset_equity + mandate_bonus`.
    pub total_equity: Units,
    /// Terminal Hard Credit Solvency tier (`Aaa..D`).
    pub credit_tier: CreditTier,
    /// Number of Hard Credit Defaults (`CheckCash` bounces).
    pub hard_credit_defaults: u32,
    /// Number of Steward-refereed `TokenEscrow` bond breaches.
    pub bond_breaches: u32,
    /// Number of Soft Diplomatic Honor promise breaches audited against `IntentPledge`s.
    pub broken_promises: u32,
    /// Cumulative honor penalty points accumulated over the season.
    pub honor_penalties: Units,
}

/// Complete sorted final scoreboard produced when a season closes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FinalScoreboard {
    /// Table identifier.
    pub table_id: crate::types::TableId,
    /// Terminal turn number (`24..=32`) when the closing bell sounded.
    pub closed_turn: u8,
    /// Seat scores sorted descending by `total_equity` (ties broken by the closing turn's HKDF
    /// `RngDomain::Tiebreak` stream per Decision 1, `B-7`).
    pub standings: Vec<SeatScoreEntry>,
}

/// Computes the deterministic [`FinalScoreboard`] from terminal `state` and verified mandate reveals.
pub fn compute_final_scoreboard(
    state: &EngineState,
    verified_mandates: &[Option<MandateReveal>],
) -> Result<FinalScoreboard, EngineError> {
    if verified_mandates.len() != state.seats.len() {
        return Err(EngineError::SeasonEnded(
            "verified mandate slice length mismatches seat count".to_owned(),
        ));
    }
    let marked_equities: Vec<Units> = (0..state.seats.len())
        .map(|idx| compute_final_marked_net_asset_equity(state, idx as u8))
        .collect();
    let pre_mandate_nws: Vec<Units> = state
        .seats
        .iter()
        .enumerate()
        .map(|(idx, s)| s.project_equity.saturating_add(marked_equities[idx]))
        .collect();
    let median_nw = table_median_net_worth(&pre_mandate_nws);
    let starting_median_nw = compute_table_median_starting_net_worth(&state.spec);

    let mut standings = Vec::with_capacity(state.seats.len());
    for (idx, seat_state) in state.seats.iter().enumerate() {
        let seat = idx as u8;
        let net_asset_equity = marked_equities[idx];
        let mandate_bonus = match verified_mandates[idx] {
            Some(ref reveal) => match evaluate_mandate_points_milli(state, seat, reveal) {
                Some(pts_milli) => convert_mandate_points_to_equity_with_floor(
                    pts_milli,
                    median_nw,
                    starting_median_nw,
                    state.spec.mandate_alpha_bps,
                ),
                None => 0,
            },
            None => 0,
        };
        let total_equity = seat_state
            .project_equity
            .saturating_add(net_asset_equity)
            .saturating_add(mandate_bonus);
        standings.push(SeatScoreEntry {
            seat,
            project_equity: seat_state.project_equity,
            net_asset_equity,
            mandate_bonus,
            total_equity,
            credit_tier: seat_state.credit_tier,
            hard_credit_defaults: seat_state.hard_credit_defaults,
            bond_breaches: seat_state.bond_breaches,
            broken_promises: seat_state.broken_promises,
            honor_penalties: seat_state.honor_penalties,
        });
    }
    let table_id = state.table_id;
    let closed_turn = Turn(state.turn);
    let closing_beacon = state.last_turn_beacon;
    standings.sort_by(|a, b| {
        b.total_equity
            .cmp(&a.total_equity)
            .then_with(|| {
                standings_tie_key(closing_beacon, table_id, closed_turn, a.seat).cmp(
                    &standings_tie_key(closing_beacon, table_id, closed_turn, b.seat),
                )
            })
            .then_with(|| a.seat.cmp(&b.seat))
    });
    Ok(FinalScoreboard {
        table_id: state.table_id,
        closed_turn: state.turn,
        standings,
    })
}
