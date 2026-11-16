//! Deterministic Steward replay from initial `TableSpec`, public layout seed, Turn-1 mandate
//! commitments, per-turn action/commit/reveal logs, and cryptographically verified turn beacons.

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};

use crate::commit::{
    CommitmentHash, MandateReveal, OrderReveal, SealedEvalSeed, SealedMandateDeal,
    draw_shocks_for_turn, validate_eval_vrf_pubkey, verify_drand_quicknet_beacon,
    verify_eval_vrf_beacon, verify_sealed_mandate_deal,
};
use crate::contracts::MarketAction;
use crate::engine::{SweepReceipt, TurnEngine, TurnPhase};
use crate::scoring::FinalScoreboard;
use crate::types::{EngineError, ScheduledShockSpec, Seat, TableMode, TableSpec, Turn};

mod opt_hex32_pub {
    use super::*;

    pub fn serialize<S: Serializer>(
        value: &Option<[u8; 32]>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            Some(bytes) => serializer.serialize_some(&hex::encode(bytes)),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<[u8; 32]>, D::Error> {
        let opt = Option::<String>::deserialize(deserializer)?;
        match opt {
            None => Ok(None),
            Some(s) => {
                let decoded = hex::decode(&s).map_err(serde::de::Error::custom)?;
                if decoded.len() != 32 {
                    return Err(serde::de::Error::custom("expected 32 hex-encoded bytes"));
                }
                let mut out = [0u8; 32];
                out.copy_from_slice(&decoded);
                Ok(Some(out))
            }
        }
    }
}

/// Verification mode for per-turn beacons during [`replay_season`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReplayBeaconMode {
    /// Eval mode with proofs under the RFC 9381 ECVRF construction with C2SP's `ristretto255`
    /// ciphersuite (`c2sp.org/vrf-r255`, `vrf-r255 0.1.0`; not an IANA-registered suite in RFC 9381)
    /// verified against `eval_vrf_pubkey` (`#31` comment, `2026-11-15` ruling).
    #[default]
    EvalVrf,
    /// Game mode with League of Entropy drand `quicknet` beacons: requires `spec.drand_schedule`,
    /// scheduled `drand_round`, and `turn_beacon == SHA-256(drand_signature)`.
    ///
    /// **Deferred verification note (`M2`):** `league-core` does not yet perform BLS12-381
    /// signature verification against the `quicknet` public key (that check runs in `#29`).
    /// Consequently, `ExternalDrand` replay checks internal log self-consistency against the
    /// pre-named round schedule, not cryptographic authenticity.
    ExternalDrand,
    /// Legacy seed-derived beacon mode — refused by [`replay_season`] in both `Game` and `Eval`
    /// modes because randomness must never derive from the public layout seed (`#31`).
    SeedDerived,
}

/// Recorded ordered inputs for a single simultaneous turn (`TalkAndMarket -> Commit -> Reveal -> Sweep`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoggedTurn {
    /// Expected turn number (`1..=32`).
    pub turn: u8,
    /// Ordered Phase 1 market, check, escrow, and intent actions.
    pub market_actions: Vec<MarketAction>,
    /// Phase 2 cryptographic order commitments keyed by seat index.
    pub commitments: BTreeMap<u8, CommitmentHash>,
    /// Phase 2 order reveals keyed by seat index.
    pub reveals: BTreeMap<u8, OrderReveal>,
    /// Publicly logged randomness beacon (`SHA-256(drand_signature)` in Game Mode or VRF `beta[..32]` in Eval Mode).
    pub turn_beacon: [u8; 32],
    /// Optional drand round number required when `beacon_mode == ReplayBeaconMode::ExternalDrand`.
    #[serde(default)]
    pub drand_round: Option<u64>,
    /// Optional 48-byte drand `quicknet` BLS12-381 signature required under `ExternalDrand` (`M2`).
    #[serde(default)]
    pub drand_signature: Option<Vec<u8>>,
    /// Optional 80-byte VRF proof (RFC 9381 ECVRF construction with C2SP's `ristretto255`
    /// ciphersuite, `vrf-r255 0.1.0`) required under `EvalVrf` (`#31`).
    #[serde(default)]
    pub eval_vrf_proof: Option<Vec<u8>>,
    /// 3-stage Horizon Shocks drawn from this turn's HKDF `"shocks"` stream (`#31`).
    #[serde(default)]
    pub drawn_shocks: Vec<ScheduledShockSpec>,
}

/// Complete portable audit log replaying an entire season without any ledger I/O.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeasonReplayLog {
    /// Immutable table specification (`4..=6` seats).
    pub spec: TableSpec,
    /// Required execution mode matching `spec.mode` (`Game` or `Eval`; prevents mode switches).
    pub mode: TableMode,
    /// Public table layout seed associated with this log (never used to derive randomness or deals).
    pub seed: u64,
    /// Verification rule applied to each logged turn beacon.
    pub beacon_mode: ReplayBeaconMode,
    /// Optional 32-byte `ristretto255` VRF public key (RFC 9381 ECVRF construction with C2SP's
    /// `c2sp.org/vrf-r255` ciphersuite, `vrf-r255 0.1.0`) required in `Eval` mode.
    #[serde(default, with = "opt_hex32_pub")]
    pub eval_vrf_pubkey: Option<[u8; 32]>,
    /// Sealed OS-entropy Turn-1 mandate deal and per-seat salts bound to `spec.dealt_mandate_commits` (`M3`).
    pub sealed_mandate_deal: SealedMandateDeal,
    /// Optional Turn-1 Private Charter Mandate commitments per seat (`0..seat_count`).
    pub mandate_commits: Vec<Option<CommitmentHash>>,
    /// Chronological sequence of turns executed from Turn `1` through closing turn `T in 24..=32`.
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
    if !log.spec.shocks.is_empty() {
        return Err(EngineError::InvalidConfig(
            "replay_season refuses pre-seeded spec.shocks; shocks must draw from verified per-turn beacons"
                .to_owned(),
        ));
    }
    match log.mode {
        TableMode::Game if log.beacon_mode != ReplayBeaconMode::ExternalDrand => {
            return Err(EngineError::InvalidConfig(
                "Game mode replay refuses non-drand beacon_mode; ExternalDrand is required"
                    .to_owned(),
            ));
        }
        TableMode::Eval if log.beacon_mode != ReplayBeaconMode::EvalVrf => {
            return Err(EngineError::InvalidConfig(
                "Eval mode replay refuses non-VRF beacon_mode; EvalVrf is required".to_owned(),
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
            "sealed_mandate_deal deal_seed must be independent of public layout seed".to_owned(),
        ));
    }

    let (drand_schedule, eval_vrf_pk) = match log.mode {
        TableMode::Game => {
            if log.eval_vrf_pubkey.is_some() {
                return Err(EngineError::InvalidConfig(
                    "Game mode replay log must not carry eval_vrf_pubkey".to_owned(),
                ));
            }
            let Some(schedule) = log.spec.drand_schedule else {
                return Err(EngineError::InvalidConfig(
                    "ExternalDrand replay requires TableSpec::drand_schedule".to_owned(),
                ));
            };
            schedule.validate()?;
            (Some(schedule), None)
        }
        TableMode::Eval => {
            let Some(spec_pk) = log.spec.eval_vrf_pubkey else {
                return Err(EngineError::InvalidConfig(
                    "EvalVrf replay requires TableSpec::eval_vrf_pubkey".to_owned(),
                ));
            };
            if log.eval_vrf_pubkey != Some(spec_pk) {
                return Err(EngineError::InvalidConfig(
                    "SeasonReplayLog::eval_vrf_pubkey must match TableSpec::eval_vrf_pubkey"
                        .to_owned(),
                ));
            }
            validate_eval_vrf_pubkey(spec_pk)?;
            for candidate in [padded_beacon_seed, hashed_beacon_seed] {
                if let Ok(derived_seed) = SealedEvalSeed::try_from_bytes(candidate)
                    && derived_seed.public_key_bytes().ok() == Some(spec_pk)
                {
                    return Err(EngineError::InvalidConfig(
                        "Eval VRF key must be independent of public layout seed".to_owned(),
                    ));
                }
            }
            (None, Some(spec_pk))
        }
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

    enum TurnSource<'a> {
        Drand(u64, &'a [u8]),
        EvalVrf(&'a [u8]),
    }

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
        let verified_source = match log.mode {
            TableMode::Eval => {
                if logged.drand_round.is_some() || logged.drand_signature.is_some() {
                    return Err(EngineError::InvalidOrder(format!(
                        "turn {} carries drand fields in Eval mode",
                        logged.turn
                    )));
                }
                let Some(ref vrf_proof) = logged.eval_vrf_proof else {
                    return Err(EngineError::SeasonEnded(format!(
                        "turn {} missing eval_vrf_proof in EvalVrf mode",
                        logged.turn
                    )));
                };
                let Some(pk) = eval_vrf_pk else {
                    return Err(EngineError::InvalidConfig(
                        "EvalVrf replay requires eval_vrf_pubkey".to_owned(),
                    ));
                };
                verify_eval_vrf_beacon(
                    pk,
                    log.spec.table_id,
                    Turn(logged.turn),
                    vrf_proof,
                    logged.turn_beacon,
                )?;
                TurnSource::EvalVrf(vrf_proof.as_slice())
            }
            TableMode::Game => {
                if logged.eval_vrf_proof.is_some() {
                    return Err(EngineError::InvalidOrder(format!(
                        "turn {} carries eval_vrf_proof in Game mode",
                        logged.turn
                    )));
                }
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
                TurnSource::Drand(round, drand_signature.as_slice())
            }
        };

        let expected_drawn_shocks = draw_shocks_for_turn(
            log.spec.table_id,
            log.spec.seat_count,
            Turn(logged.turn),
            logged.turn_beacon,
        );
        if logged.drawn_shocks != expected_drawn_shocks {
            return Err(EngineError::InvalidOrder(format!(
                "turn {} logged drawn_shocks do not match verified beacon HKDF shocks stream",
                logged.turn
            )));
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
        let receipt = match verified_source {
            TurnSource::Drand(round, sig) => engine.sweep_turn_with_drand(round, sig)?,
            TurnSource::EvalVrf(proof) => engine.sweep_turn_with_eval_vrf(proof)?,
        };
        state_digests.push(receipt.post_state_digest);
        receipts.push(receipt);
    }

    if engine.state().phase != TurnPhase::AwaitingMandateReveal {
        return Err(EngineError::SeasonEnded(
            "replay log ended before stochastic or Turn-32 close occurred".to_owned(),
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
        let Ok((spec, sealed_deal, eval_seed)) =
            TableSpec::standard_eval_chiral_ring_with_setup(TableId(101), 4, 7)
        else {
            panic!("valid spec");
        };
        let eval_pk = spec.eval_vrf_pubkey;
        let empty = SeasonReplayLog {
            spec: spec.clone(),
            mode: TableMode::Eval,
            seed: 7,
            beacon_mode: ReplayBeaconMode::EvalVrf,
            eval_vrf_pubkey: eval_pk,
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
            beacon_mode: ReplayBeaconMode::EvalVrf,
            eval_vrf_pubkey: eval_pk,
            sealed_mandate_deal: sealed_deal.clone(),
            mandate_commits: vec![None; 3],
            turns: Vec::new(),
            mandate_reveals: vec![None; 4],
        };
        assert!(matches!(
            replay_season(&bad_len),
            Err(EngineError::SeasonEnded(_))
        ));

        // Full minimal 24..=32 turn close via VRF-verified beacons, checking replay & errors.
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
        for t in 1..=32u8 {
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
            let Ok((beacon, proof)) = eval_seed.prove_turn(TableId(101), Turn(t)) else {
                panic!("vrf prove");
            };
            let Ok(receipt) = engine.sweep_turn_with_eval_vrf(&proof) else {
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
                eval_vrf_proof: Some(proof.to_vec()),
                drawn_shocks: receipt.newly_drawn_shocks.clone(),
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
            beacon_mode: ReplayBeaconMode::EvalVrf,
            eval_vrf_pubkey: eval_pk,
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
            eval_vrf_proof: None,
            drawn_shocks: Vec::new(),
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
