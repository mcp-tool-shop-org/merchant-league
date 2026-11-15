//! Deterministic Steward replay from initial `TableSpec`, seed, Turn-1 mandate
//! commitments, per-turn action/commit/reveal logs, and turn beacons.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::commit::{
    CommitmentHash, MandateReveal, OrderReveal, SealedMandateDeal, derive_turn_beacon,
    verify_drand_quicknet_beacon, verify_sealed_mandate_deal,
};
use crate::contracts::MarketAction;
use crate::engine::{SweepReceipt, TurnEngine, TurnPhase};
use crate::scoring::FinalScoreboard;
use crate::types::{EngineError, Seat, TableMode, TableSpec, Turn};

/// Verification mode for per-turn beacons during [`replay_season`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReplayBeaconMode {
    /// Eval / standalone mode: verifies `turn_beacon == derive_turn_beacon(seed, table_id, turn)`.
    #[default]
    SeedDerived,
    /// Game mode with League of Entropy drand `quicknet` beacons: requires `spec.drand_schedule`,
    /// scheduled `drand_round`, and `turn_beacon == SHA-256(drand_signature)`.
    ///
    /// **Deferred verification note (`M2`):** `league-core` does not yet perform BLS12-381
    /// signature verification against the `quicknet` public key (that check runs in the later
    /// live oracle/harness layer). Consequently, `ExternalDrand` replay checks internal log
    /// self-consistency against the pre-named round schedule, not cryptographic authenticity.
    ExternalDrand,
}

/// Recorded ordered inputs for a single simultaneous turn (`TalkAndMarket -> Commit -> Reveal -> Sweep`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoggedTurn {
    /// Expected turn number (`1..=28`).
    pub turn: u8,
    /// Ordered Phase 1 market, check, escrow, and intent actions.
    pub market_actions: Vec<MarketAction>,
    /// Phase 2 cryptographic order commitments keyed by seat index.
    pub commitments: BTreeMap<u8, CommitmentHash>,
    /// Phase 2 order reveals keyed by seat index.
    pub reveals: BTreeMap<u8, OrderReveal>,
    /// Publicly logged randomness beacon (`drand` in Game Mode or deterministic PRNG in Eval Mode).
    pub turn_beacon: [u8; 32],
    /// Optional drand round number required when `beacon_mode == ReplayBeaconMode::ExternalDrand`.
    #[serde(default)]
    pub drand_round: Option<u64>,
    /// Optional 48-byte drand `quicknet` BLS12-381 signature required under `ExternalDrand` (`M2`).
    #[serde(default)]
    pub drand_signature: Option<Vec<u8>>,
}

/// Complete portable audit log replaying an entire season without any ledger I/O.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeasonReplayLog {
    /// Immutable table specification (`4..=6` seats).
    pub spec: TableSpec,
    /// Required execution mode matching `spec.mode` (`Game` or `Eval`; prevents mode switches).
    pub mode: TableMode,
    /// Season replay seed associated with this log.
    pub seed: u64,
    /// Verification rule applied to each logged turn beacon.
    pub beacon_mode: ReplayBeaconMode,
    /// Sealed OS-entropy Turn-1 mandate deal and per-seat salts bound to `spec.dealt_mandate_commits` (`M3`).
    pub sealed_mandate_deal: SealedMandateDeal,
    /// Optional Turn-1 Private Charter Mandate commitments per seat (`0..seat_count`).
    pub mandate_commits: Vec<Option<CommitmentHash>>,
    /// Chronological sequence of turns executed from Turn `1` through closing turn `T in 24..=28`.
    pub turns: Vec<LoggedTurn>,
    /// Terminal Private Charter Mandate reveals per seat (`0..seat_count`).
    pub mandate_reveals: Vec<Option<MandateReveal>>,
}

/// Complete verified artifacts emitted by replaying a [`SeasonReplayLog`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayOutcome {
    /// Per-turn deterministic 8-step Sweep receipts in chronological order.
    pub receipts: Vec<SweepReceipt>,
    /// Canonical SHA-256 post-Sweep state digests (`len == receipts.len()`).
    pub state_digests: Vec<[u8; 32]>,
    /// Terminal Cardinal Net Equity scoreboard (`Completed`).
    pub scoreboard: FinalScoreboard,
}

/// Replays an entire season deterministically from `log`, verifying every phase transition,
/// cryptographic commitment, 8-step Sweep, closing bell roll, and final scoreboard.
pub fn replay_season(log: &SeasonReplayLog) -> Result<ReplayOutcome, EngineError> {
    if log.mode != log.spec.mode {
        return Err(EngineError::InvalidConfig(format!(
            "replay log mode {:?} does not match TableSpec mode {:?}",
            log.mode, log.spec.mode
        )));
    }
    match log.mode {
        TableMode::Game if log.beacon_mode != ReplayBeaconMode::ExternalDrand => {
            return Err(EngineError::InvalidConfig(
                "Game mode replay refuses non-drand beacon_mode; ExternalDrand is required"
                    .to_owned(),
            ));
        }
        TableMode::Eval if log.beacon_mode != ReplayBeaconMode::SeedDerived => {
            return Err(EngineError::InvalidConfig(
                "Eval mode replay requires SeedDerived beacon_mode".to_owned(),
            ));
        }
        _ => {}
    }

    let mut engine = TurnEngine::new(log.spec.clone())?;
    let n = usize::from(log.spec.seat_count);
    if log.mandate_commits.len() != n || log.mandate_reveals.len() != n {
        return Err(EngineError::SeasonEnded(
            "mandate commit/reveal vectors must match seat_count".to_owned(),
        ));
    }
    if !verify_sealed_mandate_deal(
        log.spec.table_id,
        log.spec.seat_count,
        &log.spec.mandate_deck,
        &log.spec.dealt_mandate_commits,
        &log.sealed_mandate_deal,
    )? {
        return Err(EngineError::InvalidConfig(
            "replay log sealed_mandate_deal does not match spec.dealt_mandate_commits and deck"
                .to_owned(),
        ));
    }
    let mut padded_beacon_seed = [0u8; 32];
    padded_beacon_seed[..8].copy_from_slice(&log.seed.to_le_bytes());
    let hashed_beacon_seed: [u8; 32] = Sha256::digest(log.seed.to_le_bytes()).into();
    if log.sealed_mandate_deal.deal_seed == padded_beacon_seed
        || log.sealed_mandate_deal.deal_seed == hashed_beacon_seed
    {
        return Err(EngineError::InvalidConfig(
            "sealed_mandate_deal deal_seed must be independent of replay beacon seed".to_owned(),
        ));
    }

    let drand_schedule = if log.mode == TableMode::Game {
        let Some(schedule) = log.spec.drand_schedule else {
            return Err(EngineError::InvalidConfig(
                "ExternalDrand replay requires TableSpec::drand_schedule".to_owned(),
            ));
        };
        schedule.validate()?;
        Some(schedule)
    } else {
        None
    };
    for (s_idx, commit_opt) in log.mandate_commits.iter().enumerate() {
        if let Some(commit) = commit_opt {
            engine.submit_mandate_commitment(Seat(s_idx as u8), *commit)?;
        }
    }
    if log.turns.is_empty() {
        return Err(EngineError::SeasonEnded(
            "season replay log contains zero turns".to_owned(),
        ));
    }

    let mut receipts = Vec::with_capacity(log.turns.len());
    let mut state_digests = Vec::with_capacity(log.turns.len());
    let mut prev_drand_round: Option<u64> = None;

    for logged in &log.turns {
        if engine.state().phase != TurnPhase::TalkAndMarket {
            return Err(EngineError::SeasonEnded(format!(
                "unexpected turn {} after season already entered {:?}",
                logged.turn,
                engine.state().phase
            )));
        }
        if logged.turn != engine.state().turn {
            return Err(EngineError::SeasonEnded(format!(
                "expected turn {}, logged turn {}",
                engine.state().turn,
                logged.turn
            )));
        }
        let verified_drand = match log.mode {
            TableMode::Eval => {
                if logged.drand_round.is_some() || logged.drand_signature.is_some() {
                    return Err(EngineError::InvalidOrder(format!(
                        "turn {} carries drand fields in Eval mode",
                        logged.turn
                    )));
                }
                let expected_beacon =
                    derive_turn_beacon(log.seed, log.spec.table_id, Turn(logged.turn));
                if logged.turn_beacon != expected_beacon {
                    return Err(EngineError::SeasonEnded(format!(
                        "turn {} beacon does not match seed {} and table {}",
                        logged.turn, log.seed, log.spec.table_id.0
                    )));
                }
                None
            }
            TableMode::Game => {
                let Some(round) = logged.drand_round else {
                    return Err(EngineError::SeasonEnded(format!(
                        "turn {} missing drand_round in ExternalDrand mode",
                        logged.turn
                    )));
                };
                if prev_drand_round.is_some_and(|prev| round <= prev) {
                    return Err(EngineError::SeasonEnded(format!(
                        "turn {} drand_round {round} is not strictly increasing",
                        logged.turn
                    )));
                }
                let Some(ref drand_signature) = logged.drand_signature else {
                    return Err(EngineError::SeasonEnded(format!(
                        "turn {} missing drand_signature in ExternalDrand mode",
                        logged.turn
                    )));
                };
                let Some(ref schedule) = drand_schedule else {
                    return Err(EngineError::InvalidConfig(
                        "ExternalDrand replay requires TableSpec::drand_schedule".to_owned(),
                    ));
                };
                verify_drand_quicknet_beacon(
                    schedule,
                    Turn(logged.turn),
                    round,
                    drand_signature,
                    logged.turn_beacon,
                )?;
                prev_drand_round = Some(round);
                Some((round, drand_signature.as_slice()))
            }
        };
        for action in &logged.market_actions {
            let _ = engine.apply_market_action(action.clone());
        }
        engine.advance_to_commit()?;
        for (&seat_idx, &commit) in &logged.commitments {
            engine.submit_order_commitment(Seat(seat_idx), commit)?;
        }
        engine.advance_to_reveal()?;
        for (&seat_idx, reveal) in &logged.reveals {
            engine.submit_order_reveal(Seat(seat_idx), reveal.clone())?;
        }
        let receipt = match verified_drand {
            Some((round, sig)) => engine.sweep_turn_with_drand(round, sig)?,
            None => engine.sweep_turn(logged.turn_beacon)?,
        };
        state_digests.push(receipt.post_state_digest);
        receipts.push(receipt);
    }

    if engine.state().phase != TurnPhase::AwaitingMandateReveal {
        return Err(EngineError::SeasonEnded(
            "replay log ended before stochastic or Turn-28 close occurred".to_owned(),
        ));
    }

    for (s_idx, rev_opt) in log.mandate_reveals.iter().enumerate() {
        if let Some(rev) = rev_opt {
            engine.submit_mandate_reveal(Seat(s_idx as u8), rev.clone())?;
        }
    }

    let scoreboard = engine.finalize_season()?;
    Ok(ReplayOutcome {
        receipts,
        state_digests,
        scoreboard,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commit::{RevealSalt, SealedOrderBundle, compute_order_commitment};
    use crate::types::{TableId, Turn};

    #[test]
    fn replay_rejects_incomplete_tampered_and_post_close_logs() {
        let Ok((spec, sealed_deal)) =
            TableSpec::standard_chiral_ring_with_mandates(TableId(101), 4, 7)
        else {
            panic!("valid spec");
        };
        let empty = SeasonReplayLog {
            spec: spec.clone(),
            mode: TableMode::Eval,
            seed: 7,
            beacon_mode: ReplayBeaconMode::SeedDerived,
            sealed_mandate_deal: sealed_deal.clone(),
            mandate_commits: vec![None; 4],
            turns: Vec::new(),
            mandate_reveals: vec![None; 4],
        };
        assert!(matches!(
            replay_season(&empty),
            Err(EngineError::SeasonEnded(_))
        ));

        let bad_len = SeasonReplayLog {
            spec: spec.clone(),
            mode: TableMode::Eval,
            seed: 7,
            beacon_mode: ReplayBeaconMode::SeedDerived,
            sealed_mandate_deal: sealed_deal.clone(),
            mandate_commits: vec![None; 3],
            turns: Vec::new(),
            mandate_reveals: vec![None; 4],
        };
        assert!(matches!(
            replay_season(&bad_len),
            Err(EngineError::SeasonEnded(_))
        ));

        // Full minimal 24..=28 turn close via deterministic beacons, checking replay & errors.
        let mut engine = match TurnEngine::new(spec.clone()) {
            Ok(e) => e,
            Err(err) => panic!("init failed: {err}"),
        };
        let mut mandate_commits = vec![None; 4];
        let mut mandate_reveals = vec![None; 4];
        let rev0 = sealed_deal.seat_reveals[0].clone();
        let mc0 = spec.dealt_mandate_commits[0];
        mandate_commits[0] = Some(mc0);
        mandate_reveals[0] = Some(rev0.clone());
        assert!(engine.submit_mandate_commitment(Seat(0), mc0).is_ok());

        let mut turns = Vec::new();
        for t in 1..=28u8 {
            if engine.state().phase != TurnPhase::TalkAndMarket {
                break;
            }
            assert!(engine.advance_to_commit().is_ok());
            let mut commitments = BTreeMap::new();
            let mut reveals = BTreeMap::new();
            for s in 0..4u8 {
                let ord_rev = OrderReveal {
                    bundle: SealedOrderBundle::default(),
                    salt: RevealSalt([s.wrapping_add(t); 32]),
                };
                let Ok(ch) = compute_order_commitment(TableId(101), Turn(t), Seat(s), &ord_rev)
                else {
                    panic!("commit");
                };
                assert!(engine.submit_order_commitment(Seat(s), ch).is_ok());
                commitments.insert(s, ch);
                reveals.insert(s, ord_rev);
            }
            assert!(engine.advance_to_reveal().is_ok());
            for (&s, r) in &reveals {
                assert!(engine.submit_order_reveal(Seat(s), r.clone()).is_ok());
            }
            let beacon = derive_turn_beacon(7, TableId(101), Turn(t));
            let Ok(receipt) = engine.sweep_turn(beacon) else {
                panic!("sweep");
            };
            turns.push(LoggedTurn {
                turn: t,
                market_actions: Vec::new(),
                commitments,
                reveals,
                turn_beacon: beacon,
                drand_round: None,
                drand_signature: None,
            });
            if receipt.season_closed {
                break;
            }
        }
        assert!(engine.submit_mandate_reveal(Seat(0), rev0).is_ok());
        let Ok(live_board) = engine.finalize_season() else {
            panic!("finalize");
        };

        let log = SeasonReplayLog {
            spec,
            mode: TableMode::Eval,
            seed: 7,
            beacon_mode: ReplayBeaconMode::SeedDerived,
            sealed_mandate_deal: sealed_deal,
            mandate_commits,
            turns: turns.clone(),
            mandate_reveals,
        };
        let Ok(out1) = replay_season(&log) else {
            panic!("replay ok");
        };
        assert_eq!(out1.scoreboard, live_board);

        // Truncated log before closing must fail cleanly.
        let mut trunc = log.clone();
        trunc.turns.pop();
        assert!(matches!(
            replay_season(&trunc),
            Err(EngineError::SeasonEnded(_))
        ));

        // Extra turn after close must fail cleanly.
        let mut extra = log.clone();
        let next_t = extra.turns.len() as u8 + 1;
        extra.turns.push(LoggedTurn {
            turn: next_t,
            market_actions: Vec::new(),
            commitments: BTreeMap::new(),
            reveals: BTreeMap::new(),
            turn_beacon: [0; 32],
            drand_round: None,
            drand_signature: None,
        });
        assert!(matches!(
            replay_season(&extra),
            Err(EngineError::SeasonEnded(_))
        ));

        // Wrong turn index must fail cleanly.
        let mut bad_idx = log;
        bad_idx.turns[0].turn = 9;
        assert!(matches!(
            replay_season(&bad_idx),
            Err(EngineError::SeasonEnded(_))
        ));
    }
}
