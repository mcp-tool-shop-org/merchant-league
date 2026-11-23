//! Sequential checkpoint verification and season-close CRN rollouts (`§6.1`).

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use league_core::{
    ClubBot, ClubBotDecision, EvalActSubmission, EvalHost, PrivateMandate, Seat, TurnPhase,
};

use crate::error::ScorerError;
use crate::scenarios::{ScenarioAssignment, draw_scenarios};
use crate::scoring::score_outcomes;
use crate::types::{
    BlindScoringRequest, EVAL_TABLE_SEAT_COUNT, ScenarioRolloutOutcome, ScorerConfig,
    StateScoreOutput, compute_pre_act_pair_sha256_hex, sha256_hex,
};

fn submission_from_club_bot(decision: ClubBotDecision) -> EvalActSubmission {
    EvalActSubmission {
        market_actions: decision.pass_a_actions,
        pass_b_actions: decision.pass_b_actions,
        bundle: decision.bundle,
    }
}

/// Verifies that on-disk checkpoint bytes match `pre_act_save_sha256`, `pre_act_seal_sha256`,
/// and `pre_act_pair_sha256` declared on `request` before calling `EvalHost::fork_from_checkpoint`.
pub fn verify_checkpoint_pair(
    save_path: &Path,
    seal_path: &Path,
    request: &BlindScoringRequest,
) -> Result<(), ScorerError> {
    let save_bytes = fs::read(save_path).map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            ScorerError::CheckpointNotFound(format!(
                "missing checkpoint save for `{}`",
                request.pre_act_save_sha256
            ))
        } else {
            ScorerError::Io(format!("read checkpoint save: {err}"))
        }
    })?;
    let seal_bytes = fs::read(seal_path).map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            ScorerError::CheckpointNotFound(format!(
                "missing checkpoint seal for `{}`",
                request.pre_act_seal_sha256
            ))
        } else {
            ScorerError::Io(format!("read checkpoint seal: {err}"))
        }
    })?;

    let actual_save = sha256_hex(&save_bytes);
    if actual_save != request.pre_act_save_sha256 {
        return Err(ScorerError::CheckpointHashMismatch(format!(
            "save SHA-256 mismatch: expected `{}`, got `{actual_save}`",
            request.pre_act_save_sha256
        )));
    }
    let actual_seal = sha256_hex(&seal_bytes);
    if actual_seal != request.pre_act_seal_sha256 {
        return Err(ScorerError::CheckpointHashMismatch(format!(
            "seal SHA-256 mismatch: expected `{}`, got `{actual_seal}`",
            request.pre_act_seal_sha256
        )));
    }
    let actual_pair = compute_pre_act_pair_sha256_hex(&save_bytes, &seal_bytes);
    if actual_pair != request.pre_act_pair_sha256 {
        return Err(ScorerError::CheckpointHashMismatch(format!(
            "pair SHA-256 mismatch: expected `{}`, got `{actual_pair}`",
            request.pre_act_pair_sha256
        )));
    }
    Ok(())
}

/// Runs one deterministic fork-to-close rollout from a verified pre-act checkpoint (`§6.1`).
///
/// Forks a fresh [`EvalHost`], applies `focal_submission` alongside `peer_day_submissions`,
/// sweeps turn `day`, plays all remaining turns until natural close with `ClubBot` (using
/// `focal_mandate` at `focal_seat` and scenario mandates at peers), finalizes the season, and
/// returns the focal seat's exact integer equity breakdown.
#[allow(clippy::too_many_arguments)]
pub fn run_single_rollout(
    save_path: &Path,
    seal_path: &Path,
    focal_seat: u8,
    focal_submission: &EvalActSubmission,
    peer_day_submissions: &BTreeMap<u8, EvalActSubmission>,
    focal_mandate: PrivateMandate,
    scenario: &ScenarioAssignment,
) -> Result<ScenarioRolloutOutcome, ScorerError> {
    let mut host = EvalHost::fork_from_checkpoint(save_path, seal_path)
        .map_err(|e| ScorerError::Engine(e.to_string()))?;
    let seat_count = host.seat_count();
    if seat_count != EVAL_TABLE_SEAT_COUNT {
        return Err(ScorerError::InvalidCheckpointState(format!(
            "expected {EVAL_TABLE_SEAT_COUNT}-seat Evaluation table, got {seat_count}"
        )));
    }
    if focal_seat >= seat_count {
        return Err(ScorerError::InvalidFocalSeat {
            focal_seat,
            seat_count,
        });
    }

    let mut day_subs = peer_day_submissions.clone();
    day_subs.insert(focal_seat, focal_submission.clone());
    host.apply_act_submissions(&day_subs)
        .map_err(|e| ScorerError::Engine(format!("apply turn {} submissions: {e}", host.turn())))?;
    host.sweep_turn()
        .map_err(|e| ScorerError::Engine(format!("sweep turn {}: {e}", host.turn())))?;

    while host.phase() == TurnPhase::TalkAndMarket {
        let mut subs = BTreeMap::new();
        for s in 0..seat_count {
            let obs = host
                .observe_seat(Seat(s))
                .map_err(|e| ScorerError::Engine(format!("observe seat {s}: {e}")))?;
            let mandate = if s == focal_seat {
                focal_mandate
            } else {
                scenario
                    .peer_mandates
                    .get(&s)
                    .copied()
                    .ok_or_else(|| ScorerError::Engine(format!("missing peer mandate {s}")))?
            };
            let bot = ClubBot::new(scenario.bot_base_seed.wrapping_add(u64::from(s)));
            subs.insert(
                s,
                submission_from_club_bot(bot.decide_full_turn(&obs, &mandate)),
            );
        }
        host.apply_act_submissions(&subs).map_err(|e| {
            ScorerError::Engine(format!(
                "apply turn {} rollout submissions: {e}",
                host.turn()
            ))
        })?;
        host.sweep_turn()
            .map_err(|e| ScorerError::Engine(format!("sweep turn {} rollout: {e}", host.turn())))?;
    }

    let (scoreboard, _, _, _) = host
        .finalize_eval_season()
        .map_err(|e| ScorerError::Engine(format!("finalize_eval_season: {e}")))?;
    let entry = scoreboard
        .standings
        .iter()
        .find(|e| e.seat == focal_seat)
        .ok_or_else(|| {
            ScorerError::Engine(format!(
                "focal seat {focal_seat} missing from FinalScoreboard"
            ))
        })?;

    Ok(ScenarioRolloutOutcome {
        mandate_bonus: entry.mandate_bonus,
        net_asset_equity: entry.net_asset_equity,
        project_equity: entry.project_equity,
        total_equity: entry.total_equity,
    })
}

/// Verifies the checkpoint pair, draws `scenarios_m` counterfactual scenarios, runs sequential
/// rollouts over `(candidate, scenario)`, and computes deterministic scores (`§6.1`, `§7.1`).
pub fn execute_blind_request(
    request: &BlindScoringRequest,
    config: &ScorerConfig,
    save_path: &Path,
    seal_path: &Path,
) -> Result<StateScoreOutput, ScorerError> {
    config.validate_for_s1()?;
    request.validate(config)?;
    verify_checkpoint_pair(save_path, seal_path, request)?;

    let probe = EvalHost::fork_from_checkpoint(save_path, seal_path)
        .map_err(|e| ScorerError::Engine(e.to_string()))?;
    let seat_count = probe.seat_count();
    if seat_count != EVAL_TABLE_SEAT_COUNT {
        return Err(ScorerError::InvalidCheckpointState(format!(
            "expected {EVAL_TABLE_SEAT_COUNT}-seat Evaluation table, got {seat_count}"
        )));
    }
    let day = probe.turn();
    let focal_obs = probe
        .observe_seat(Seat(request.focal_seat))
        .map_err(|e| ScorerError::Engine(format!("observe focal seat: {e}")))?;
    let focal_mandate = focal_obs.own_mandate().copied().ok_or_else(|| {
        ScorerError::Engine("focal SeatObservation omitted own_mandate".to_owned())
    })?;

    let scenarios = draw_scenarios(
        &focal_obs.spec,
        focal_mandate,
        request.focal_seat,
        request.scorer_seed,
        &request.pre_act_pair_sha256,
        day,
        config.scenarios_m,
    )?;

    // Precompute deterministic peer observations once from the probe host, then compute each
    // scenario `m`'s counterfactual turn-`day` peer submissions once per `(state, m)`.
    let mut peer_observations = Vec::with_capacity(usize::from(seat_count.saturating_sub(1)));
    for s in 0..seat_count {
        if s != request.focal_seat {
            let obs_s = probe
                .observe_seat(Seat(s))
                .map_err(|e| ScorerError::Engine(format!("observe peer seat {s}: {e}")))?;
            peer_observations.push((s, obs_s));
        }
    }

    let mut peer_day_subs_by_m = Vec::with_capacity(scenarios.len());
    for scenario in &scenarios {
        let mut peer_subs = BTreeMap::new();
        for (s, obs_s) in &peer_observations {
            let mandate = scenario
                .peer_mandates
                .get(s)
                .copied()
                .ok_or_else(|| ScorerError::Engine(format!("missing peer mandate {s}")))?;
            let bot = ClubBot::new(scenario.bot_base_seed.wrapping_add(u64::from(*s)));
            peer_subs.insert(
                *s,
                submission_from_club_bot(bot.decide_full_turn(obs_s, &mandate)),
            );
        }
        peer_day_subs_by_m.push(peer_subs);
    }
    drop(probe);

    let mut outcomes = Vec::with_capacity(request.candidates.len());
    for candidate in &request.candidates {
        let mut row = Vec::with_capacity(scenarios.len());
        for (m_idx, scenario) in scenarios.iter().enumerate() {
            let outcome = run_single_rollout(
                save_path,
                seal_path,
                request.focal_seat,
                &candidate.submission,
                &peer_day_subs_by_m[m_idx],
                focal_mandate,
                scenario,
            )?;
            row.push(outcome);
        }
        outcomes.push(row);
    }

    score_outcomes(request, config, day, &outcomes)
}
