//! Cardinal Net Equity scoring, fundamental shadow parities, Potential-Based Reward
//! Shaping (PBRS), Turn-1 Private Mandate evaluation, and terminal scoreboards.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::commit::{MandateKind, MandateReveal};
use crate::contracts::{CheckStatus, EscrowStatus, OfferSide};
use crate::engine::EngineState;
use crate::types::{
    AssetId, Bps, CreditTier, EngineError, MAX_MANDATE_BONUS_EQUITY, MIN_MANDATE_BONUS_EQUITY,
    TableSpec, UNIT_SCALE, Units, mul_units_parity, scale_bps_signed,
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

/// Evaluates whether `seat` satisfied its revealed Turn-1 Private Charter Mandate and returns
/// `(satisfied, awarded_bonus_equity)`.
#[must_use]
pub fn evaluate_mandate_bonus(
    state: &EngineState,
    seat: u8,
    reveal: &MandateReveal,
) -> (bool, Units) {
    let idx = usize::from(seat);
    let Some(seat_state) = state.seats.get(idx) else {
        return (false, 0);
    };
    if reveal.salt.is_degenerate() {
        return (false, 0);
    }
    let Some(card) =
        state.spec.mandate_deck.iter().find(|c| {
            c.kind == reveal.mandate.kind && c.bonus_equity == reveal.mandate.bonus_equity
        })
    else {
        return (false, 0);
    };
    if card
        .kind
        .validate_for_seat_count(state.spec.seat_count)
        .is_err()
    {
        return (false, 0);
    }
    if !(MIN_MANDATE_BONUS_EQUITY..=MAX_MANDATE_BONUS_EQUITY).contains(&card.bonus_equity) {
        return (false, 0);
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
    if ok {
        (true, card.bonus_equity)
    } else {
        (false, 0)
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

/// Per-seat breakdown at Season Close (`Turn T in 24..=28`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatScoreEntry {
    /// Seat index (`0..seat_count`).
    pub seat: u8,
    /// Irreversible Project Equity points burned via Tier-3 Syndicate Projects.
    pub project_equity: Units,
    /// Fundamental mark-to-market Net Asset Equity `Phi_i(T)`.
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
    /// Terminal turn number (`24..=28`) when the closing bell sounded.
    pub closed_turn: u8,
    /// Seat scores sorted descending by `total_equity` (ties broken ascending by `seat`).
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
    let mut standings = Vec::with_capacity(state.seats.len());
    for (idx, seat_state) in state.seats.iter().enumerate() {
        let seat = idx as u8;
        let net_asset_equity = compute_net_asset_equity(state, seat);
        let mandate_bonus = match verified_mandates[idx] {
            Some(ref reveal) => evaluate_mandate_bonus(state, seat, reveal).1,
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
    standings.sort_by(|a, b| {
        b.total_equity
            .cmp(&a.total_equity)
            .then_with(|| a.seat.cmp(&b.seat))
    });
    Ok(FinalScoreboard {
        table_id: state.table_id,
        closed_turn: state.turn,
        standings,
    })
}
