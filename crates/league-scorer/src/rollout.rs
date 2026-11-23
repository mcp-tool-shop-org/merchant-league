//! Sequential per-fork checkpoint verification and season-close CRN rollouts (`§6.1`).

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use league_core::{
    ClubBot, EvalActSubmission, EvalHost, PrivateMandate, SeasonReplayLog, Seat, TurnPhase,
};

use crate::error::{ScorerError, sanitize_error_text};
use crate::postures::{
    apply_talk_commitments, canonicalize_talk_commitments, decide_peer_turn, posture_for_scenario,
    submission_from_club_bot,
};
use crate::scenarios::{ScenarioAssignment, draw_scenarios};
use crate::scoring::score_outcomes;
use crate::types::{
    BlindScoringRequest, EVAL_TABLE_SEAT_COUNT, PostureKind, ScenarioRolloutOutcome, ScorerConfig,
    StateScoreOutput, compute_pre_act_pair_sha256_hex, sha256_hex,
};

/// Forks `EvalHost` from `(save_path, seal_path)`, stripping directory paths from any engine error (`§6.1`).
pub(crate) fn fork_checkpoint_sanitized(
    save_path: &Path,
    seal_path: &Path,
) -> Result<EvalHost, ScorerError> {
    EvalHost::fork_from_checkpoint(save_path, seal_path).map_err(|e| {
        ScorerError::Engine(sanitize_error_text(&e.to_string(), &[save_path, seal_path]))
    })
}

/// Validates checkpoint forkability via `EvalHost::fork_from_checkpoint`, stripping directory paths from any error.
pub fn probe_checkpoint_sanitized(save_path: &Path, seal_path: &Path) -> Result<(), ScorerError> {
    let _host = fork_checkpoint_sanitized(save_path, seal_path)?;
    Ok(())
}

/// Reads and verifies `.save.json`, `.seal`, and optional pair SHA-256 digests, returning the pair hex digest.
pub fn verify_checkpoint_digests(
    save_path: &Path,
    seal_path: &Path,
    expected_save_sha256: &str,
    expected_seal_sha256: &str,
    expected_pair_sha256: Option<&str>,
) -> Result<String, ScorerError> {
    let save_bytes = fs::read(save_path).map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            ScorerError::CheckpointNotFound(format!(
                "missing checkpoint save for `{expected_save_sha256}`"
            ))
        } else {
            ScorerError::Io(sanitize_error_text(
                &format!("read checkpoint save: {err}"),
                &[save_path],
            ))
        }
    })?;
    let seal_bytes = fs::read(seal_path).map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            ScorerError::CheckpointNotFound(format!(
                "missing checkpoint seal for `{expected_seal_sha256}`"
            ))
        } else {
            ScorerError::Io(sanitize_error_text(
                &format!("read checkpoint seal: {err}"),
                &[seal_path],
            ))
        }
    })?;

    let actual_save = sha256_hex(&save_bytes);
    if actual_save != expected_save_sha256 {
        return Err(ScorerError::CheckpointHashMismatch(format!(
            "save SHA-256 mismatch: expected `{expected_save_sha256}`, got `{actual_save}`"
        )));
    }
    let actual_seal = sha256_hex(&seal_bytes);
    if actual_seal != expected_seal_sha256 {
        return Err(ScorerError::CheckpointHashMismatch(format!(
            "seal SHA-256 mismatch: expected `{expected_seal_sha256}`, got `{actual_seal}`"
        )));
    }
    let actual_pair = compute_pre_act_pair_sha256_hex(&save_bytes, &seal_bytes);
    if let Some(expected_pair) = expected_pair_sha256
        && actual_pair != expected_pair
    {
        return Err(ScorerError::CheckpointHashMismatch(format!(
            "pair SHA-256 mismatch: expected `{expected_pair}`, got `{actual_pair}`"
        )));
    }
    Ok(actual_pair)
}

/// Re-verifies checkpoint bytes against digests immediately before calling `EvalHost::fork_from_checkpoint`.
pub(crate) fn verify_and_fork_checkpoint(
    save_path: &Path,
    seal_path: &Path,
    expected_save_sha256: &str,
    expected_seal_sha256: &str,
    expected_pair_sha256: Option<&str>,
) -> Result<(EvalHost, String), ScorerError> {
    let pair_sha = verify_checkpoint_digests(
        save_path,
        seal_path,
        expected_save_sha256,
        expected_seal_sha256,
        expected_pair_sha256,
    )?;
    let host = fork_checkpoint_sanitized(save_path, seal_path)?;
    Ok((host, pair_sha))
}

/// Verifies that on-disk checkpoint bytes match `pre_act_save_sha256`, `pre_act_seal_sha256`,
/// and `pre_act_pair_sha256` declared on `request`.
pub fn verify_checkpoint_pair(
    save_path: &Path,
    seal_path: &Path,
    request: &BlindScoringRequest,
) -> Result<(), ScorerError> {
    let _ = verify_checkpoint_digests(
        save_path,
        seal_path,
        &request.pre_act_save_sha256,
        &request.pre_act_seal_sha256,
        Some(&request.pre_act_pair_sha256),
    )?;
    Ok(())
}

/// Runs one deterministic fork-to-close rollout from a verified pre-act checkpoint (`§6.1`, `§4.2`).
///
/// Re-verifies save, seal, and pair SHA-256 bytes before every `EvalHost::fork_from_checkpoint`,
/// applies `focal_submission` alongside `peer_day_submissions`, sweeps turn `day`, plays all
/// remaining turns until natural close using `posture` for peers and unmodified `ClubBot` at
/// `focal_seat`, finalizes the season, and returns the focal seat's exact integer equity breakdown.
#[allow(clippy::too_many_arguments)]
pub fn run_single_rollout(
    save_path: &Path,
    seal_path: &Path,
    expected_digests: (&str, &str, &str),
    focal_seat: u8,
    focal_submission: &EvalActSubmission,
    peer_day_submissions: &BTreeMap<u8, EvalActSubmission>,
    focal_mandate: PrivateMandate,
    scenario: &ScenarioAssignment,
    posture: PostureKind,
    config: &ScorerConfig,
) -> Result<ScenarioRolloutOutcome, ScorerError> {
    let (outcome, _) = run_single_rollout_with_replay_log(
        save_path,
        seal_path,
        expected_digests,
        focal_seat,
        focal_submission,
        peer_day_submissions,
        focal_mandate,
        scenario,
        posture,
        config,
    )?;
    Ok(outcome)
}

/// Runs one deterministic fork-to-close rollout and also returns the finalized `SeasonReplayLog`
/// so tests can independently verify engine replay parity across postures and talk commitments.
#[allow(clippy::too_many_arguments)]
pub fn run_single_rollout_with_replay_log(
    save_path: &Path,
    seal_path: &Path,
    expected_digests: (&str, &str, &str),
    focal_seat: u8,
    focal_submission: &EvalActSubmission,
    peer_day_submissions: &BTreeMap<u8, EvalActSubmission>,
    focal_mandate: PrivateMandate,
    scenario: &ScenarioAssignment,
    posture: PostureKind,
    config: &ScorerConfig,
) -> Result<(ScenarioRolloutOutcome, SeasonReplayLog), ScorerError> {
    let (save_sha, seal_sha, pair_sha) = expected_digests;
    let (mut host, _) =
        verify_and_fork_checkpoint(save_path, seal_path, save_sha, seal_sha, Some(pair_sha))?;
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
            let seed = scenario.bot_base_seed.wrapping_add(u64::from(s));
            let sub = if s == focal_seat {
                submission_from_club_bot(ClubBot::new(seed).decide_full_turn(&obs, &focal_mandate))
            } else {
                let mandate = scenario
                    .peer_mandates
                    .get(&s)
                    .copied()
                    .ok_or_else(|| ScorerError::Engine(format!("missing peer mandate {s}")))?;
                decide_peer_turn(posture, seed, &obs, &mandate, config)?
            };
            subs.insert(s, sub);
        }
        let turn = host.turn();
        host.apply_act_submissions(&subs)
            .map_err(|e| ScorerError::Engine(format!("apply turn {turn}: {e}")))?;
        host.sweep_turn()
            .map_err(|e| ScorerError::Engine(format!("sweep turn {turn}: {e}")))?;
    }

    let (scoreboard, replay_log, _, _) = host
        .finalize_eval_season()
        .map_err(|e| ScorerError::Engine(format!("finalize_eval_season: {e}")))?;
    let entry = scoreboard
        .standings
        .iter()
        .find(|e| e.seat == focal_seat)
        .ok_or_else(|| ScorerError::Engine(format!("focal seat {focal_seat} missing")))?;

    Ok((
        ScenarioRolloutOutcome {
            mandate_bonus: entry.mandate_bonus,
            net_asset_equity: entry.net_asset_equity,
            project_equity: entry.project_equity,
            total_equity: entry.total_equity,
        },
        replay_log,
    ))
}

/// Verifies checkpoint hashes before every fork, draws `scenarios_m` counterfactual scenarios with
/// configured postures and talk-commitment compliance, runs sequential rollouts over `(c, m)`, and
/// computes deterministic scores (`§4.2`, `§4.3`, `§6.1`, `§7.1`).
pub fn execute_blind_request(
    request: &BlindScoringRequest,
    config: &ScorerConfig,
    save_path: &Path,
    seal_path: &Path,
) -> Result<StateScoreOutput, ScorerError> {
    config.validate()?;
    request.validate(config)?;

    let (probe, _) = verify_and_fork_checkpoint(
        save_path,
        seal_path,
        &request.pre_act_save_sha256,
        &request.pre_act_seal_sha256,
        Some(&request.pre_act_pair_sha256),
    )?;
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
    let canonical_commitments = canonicalize_talk_commitments(&request.talk_commitments)?;

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
    for (m_idx, scenario) in scenarios.iter().enumerate() {
        let posture = posture_for_scenario(config, m_idx);
        let mut peer_subs = BTreeMap::new();
        for (s, obs_s) in &peer_observations {
            let mandate = scenario
                .peer_mandates
                .get(s)
                .copied()
                .ok_or_else(|| ScorerError::Engine(format!("missing peer mandate {s}")))?;
            let seed = scenario.bot_base_seed.wrapping_add(u64::from(*s));
            let base_sub = decide_peer_turn(posture, seed, obs_s, &mandate, config)?;
            let final_sub = apply_talk_commitments(
                obs_s,
                base_sub,
                &canonical_commitments,
                request.scorer_seed,
                &request.pre_act_pair_sha256,
                m_idx,
                config.p_honour,
            )?;
            peer_subs.insert(*s, final_sub);
        }
        peer_day_subs_by_m.push(peer_subs);
    }
    drop(probe);

    let digests = (
        request.pre_act_save_sha256.as_str(),
        request.pre_act_seal_sha256.as_str(),
        request.pre_act_pair_sha256.as_str(),
    );
    let mut outcomes = Vec::with_capacity(request.candidates.len());
    for candidate in &request.candidates {
        let mut row = Vec::with_capacity(scenarios.len());
        for (m_idx, scenario) in scenarios.iter().enumerate() {
            let posture = posture_for_scenario(config, m_idx);
            let outcome = run_single_rollout(
                save_path,
                seal_path,
                digests,
                request.focal_seat,
                &candidate.submission,
                &peer_day_subs_by_m[m_idx],
                focal_mandate,
                scenario,
                posture,
                config,
            )?;
            row.push(outcome);
        }
        outcomes.push(row);
    }

    score_outcomes(request, config, day, &outcomes)
}
