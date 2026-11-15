//! Deterministic Steward replay from initial `TableSpec`, seed, Turn-1 mandate
//! commitments, per-turn action/commit/reveal logs, and turn beacons.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::commit::{CommitmentHash, MandateReveal, OrderReveal, derive_turn_beacon};
use crate::contracts::MarketAction;
use crate::engine::{SweepReceipt, TurnEngine, TurnPhase};
use crate::scoring::FinalScoreboard;
use crate::types::{EngineError, Seat, TableSpec, Turn};

/// Verification mode for per-turn beacons during [`replay_season`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReplayBeaconMode {
    /// Eval / standalone mode: verifies `turn_beacon == derive_turn_beacon(seed, table_id, turn)`.
    #[default]
    SeedDerived,
    /// Game mode with public drand beacon: requires strictly increasing `drand_round` and non-zero beacon.
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
}

/// Complete portable audit log replaying an entire season without any ledger I/O.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeasonReplayLog {
    /// Immutable table specification (`4..=6` seats).
    pub spec: TableSpec,
    /// Season replay seed associated with this log.
    pub seed: u64,
    /// Verification rule applied to each logged turn beacon.
    #[serde(default)]
    pub beacon_mode: ReplayBeaconMode,
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
    let mut engine = TurnEngine::new(log.spec.clone())?;
    let n = usize::from(log.spec.seat_count);
    if log.mandate_commits.len() != n || log.mandate_reveals.len() != n {
        return Err(EngineError::SeasonEnded(
            "mandate commit/reveal vectors must match seat_count".to_owned(),
        ));
    }
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
        match log.beacon_mode {
            ReplayBeaconMode::SeedDerived => {
                let expected_beacon =
                    derive_turn_beacon(log.seed, log.spec.table_id, Turn(logged.turn));
                if logged.turn_beacon != expected_beacon {
                    return Err(EngineError::SeasonEnded(format!(
                        "turn {} beacon does not match seed {} and table {}",
                        logged.turn, log.seed, log.spec.table_id.0
                    )));
                }
            }
            ReplayBeaconMode::ExternalDrand => {
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
                if logged.turn_beacon == [0u8; 32] {
                    return Err(EngineError::SeasonEnded(format!(
                        "turn {} has all-zero external drand beacon",
                        logged.turn
                    )));
                }
                prev_drand_round = Some(round);
            }
        }
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
        let receipt = engine.sweep_turn(logged.turn_beacon)?;
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
        let Ok((spec, dealt_reveals)) =
            TableSpec::standard_chiral_ring_with_mandates(TableId(101), 4, 7)
        else {
            panic!("valid spec");
        };
        let empty = SeasonReplayLog {
            spec: spec.clone(),
            seed: 7,
            beacon_mode: ReplayBeaconMode::SeedDerived,
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
            seed: 7,
            beacon_mode: ReplayBeaconMode::SeedDerived,
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
        let rev0 = dealt_reveals[0].clone();
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
            seed: 7,
            beacon_mode: ReplayBeaconMode::SeedDerived,
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
