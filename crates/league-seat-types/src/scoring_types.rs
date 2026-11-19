use alloc::vec;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

use crate::types::{AssetId, CreditTier, SeatConfig, TableId, UNIT_SCALE, Units};

/// Public seat-config source needed to derive deterministic [`FundamentalParities`].
pub trait ParitySpecSource {
    /// Returns the number of seats at the table (`4..=6`).
    fn spec_seat_count(&self) -> u8;

    /// Returns the ordered seat economic configurations (`0..spec_seat_count()`).
    fn spec_seats(&self) -> &[SeatConfig];
}

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
    pub fn from_spec<S: ParitySpecSource + ?Sized>(spec: &S) -> Self {
        let seat_configs = spec.spec_seats();
        let n = usize::from(spec.spec_seat_count());
        let refined_per_unit: Vec<Units> = seat_configs
            .iter()
            .map(|s| s.refined_project_value)
            .collect();
        let mean_refined = if n > 0 {
            refined_per_unit.iter().map(|&v| v as i128).sum::<i128>() / (n as i128)
        } else {
            UNIT_SCALE as i128
        };

        let mut num = vec![0_i128; n];
        let mut den = vec![0_i128; n];
        for seat in seat_configs {
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
    pub table_id: TableId,
    /// Terminal turn number (`24..=32`) when the closing bell sounded.
    pub closed_turn: u8,
    /// Seat scores sorted descending by `total_equity` (ties broken by the closing turn's HKDF
    /// `RngDomain::Tiebreak` stream).
    pub standings: Vec<SeatScoreEntry>,
}
