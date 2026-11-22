//! Deterministic undealt-mandate scenario draws and per-scenario bot seeds (`§5.1`, `§6.1`).

use std::collections::BTreeMap;

use league_core::{PrivateMandate, PublicTableSpec};
use sha2::{Digest, Sha256};

use crate::error::ScorerError;

/// One scenario's undealt peer mandate assignment and deterministic base bot seed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioAssignment {
    /// Counterfactual bot base seed for scenario `m` (`ClubBot::new(bot_base_seed.wrapping_add(u64::from(seat)))`).
    pub bot_base_seed: u64,
    /// Remaining undealt mandates for every peer seat `s != focal_seat`, keyed by seat index.
    pub peer_mandates: BTreeMap<u8, PrivateMandate>,
    /// Selected entry IDs drawn from `spec.mandate_deck` (matching peer seat order), for invariant tests.
    pub selected_card_ids: Vec<(u8, u16)>,
}

fn sha256_u64_le(domain: &[u8], seed: u64, pair_sha: &str, day: u8, m: usize, ctr: u64) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(seed.to_le_bytes());
    hasher.update(pair_sha.as_bytes());
    hasher.update([day]);
    hasher.update((m as u64).to_le_bytes());
    hasher.update(ctr.to_le_bytes());
    let digest: [u8; 32] = hasher.finalize().into();
    let mut buf = [0_u8; 8];
    buf.copy_from_slice(&digest[..8]);
    u64::from_le_bytes(buf)
}

/// Unbiased integer uniform index in `0..bound` using rejection sampling over 64-bit SHA-256 outputs.
fn draw_uniform_index(
    bound: usize,
    scorer_seed: u64,
    pair_sha: &str,
    day: u8,
    m: usize,
    ctr: &mut u64,
) -> usize {
    if bound <= 1 {
        return 0;
    }
    let n = bound as u128;
    let limit = ((1_u128 << 64) / n) * n;
    loop {
        let sample = u128::from(sha256_u64_le(
            b"league-scorer/mandate-draw/v1|",
            scorer_seed,
            pair_sha,
            day,
            m,
            *ctr,
        ));
        *ctr = ctr.wrapping_add(1);
        if sample < limit {
            return (sample % n) as usize;
        }
    }
}

/// Draws `scenarios_m` counterfactual undealt-mandate assignments from `spec.mandate_deck` (`§5.1`).
///
/// Removes one entry matching `focal_mandate` (`kind` and `bonus_equity`) from `spec.mandate_deck`,
/// then runs uniform reverse Fisher-Yates from domain-separated SHA-256 of
/// `(scorer_seed, pre_act_pair_sha256, day, m)` using pure integer arithmetic.
#[allow(clippy::too_many_arguments)]
pub fn draw_scenarios(
    spec: &PublicTableSpec,
    focal_mandate: PrivateMandate,
    focal_seat: u8,
    scorer_seed: u64,
    pre_act_pair_sha256: &str,
    day: u8,
    scenarios_m: usize,
) -> Result<Vec<ScenarioAssignment>, ScorerError> {
    if focal_seat >= spec.seat_count {
        return Err(ScorerError::InvalidFocalSeat {
            focal_seat,
            seat_count: spec.seat_count,
        });
    }

    let mut undealt_pool: Vec<(u16, PrivateMandate)> = spec
        .mandate_deck
        .iter()
        .map(|entry| {
            (
                entry.card_id,
                PrivateMandate {
                    kind: entry.kind,
                    bonus_equity: entry.bonus_equity,
                },
            )
        })
        .collect();

    let Some(focal_pos) = undealt_pool.iter().position(|(_, m)| *m == focal_mandate) else {
        return Err(ScorerError::FocalMandateNotInDeck);
    };
    undealt_pool.remove(focal_pos);

    let peer_seats: Vec<u8> = (0..spec.seat_count).filter(|&s| s != focal_seat).collect();
    if undealt_pool.len() < peer_seats.len() {
        return Err(ScorerError::InsufficientMandatePool {
            pool_len: undealt_pool.len(),
            needed: peer_seats.len(),
        });
    }

    let mut scenarios = Vec::with_capacity(scenarios_m);
    for m in 0..scenarios_m {
        let mut perm = undealt_pool.clone();
        let mut ctr = 0_u64;
        for i in (1..perm.len()).rev() {
            let j = draw_uniform_index(i + 1, scorer_seed, pre_act_pair_sha256, day, m, &mut ctr);
            perm.swap(i, j);
        }

        let mut peer_mandates = BTreeMap::new();
        let mut selected_card_ids = Vec::with_capacity(peer_seats.len());
        for (slot_idx, &peer_seat) in peer_seats.iter().enumerate() {
            let (card_id, mandate) = perm[slot_idx];
            peer_mandates.insert(peer_seat, mandate);
            selected_card_ids.push((peer_seat, card_id));
        }
        let bot_base_seed = sha256_u64_le(
            b"league-scorer/scenario-bot-seed/v1|",
            scorer_seed,
            pre_act_pair_sha256,
            day,
            m,
            0,
        );
        scenarios.push(ScenarioAssignment {
            bot_base_seed,
            peer_mandates,
            selected_card_ids,
        });
    }
    Ok(scenarios)
}
