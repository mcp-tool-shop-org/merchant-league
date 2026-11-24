//! Sequential per-fork checkpoint verification and season-close CRN rollouts (`§6.1`).

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use league_core::eval::host::checkpoint_verify::{
    CheckpointVerifyError, verify_and_construct_pre_act_bytes,
};
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
    StateScoreOutput, VerifiedCheckpointsManifest, compute_pre_act_pair_sha256_hex, sha256_hex,
};

/// Fixed schema version for preflight `verified_checkpoints.json` manifests.
pub const VERIFIED_CHECKPOINTS_SCHEMA_VERSION: u32 = 1;

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

fn read_checkpoint_pair_once(
    save_path: &Path,
    seal_path: &Path,
    save_label: &str,
    seal_label: &str,
) -> Result<(Vec<u8>, Vec<u8>), ScorerError> {
    let save_bytes = fs::read(save_path).map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            ScorerError::CheckpointNotFound(format!("missing checkpoint save for `{save_label}`"))
        } else {
            ScorerError::Io(sanitize_error_text(
                &format!("read checkpoint save: {err}"),
                &[save_path],
            ))
        }
    })?;
    let seal_bytes = fs::read(seal_path).map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            ScorerError::CheckpointNotFound(format!("missing checkpoint seal for `{seal_label}`"))
        } else {
            ScorerError::Io(sanitize_error_text(
                &format!("read checkpoint seal: {err}"),
                &[seal_path],
            ))
        }
    })?;
    Ok((save_bytes, seal_bytes))
}

/// Reads and verifies `.save.json`, `.seal`, and optional pair SHA-256 digests, returning the pair hex digest.
pub fn verify_checkpoint_digests(
    save_path: &Path,
    seal_path: &Path,
    expected_save_sha256: &str,
    expected_seal_sha256: &str,
    expected_pair_sha256: Option<&str>,
) -> Result<String, ScorerError> {
    let (save_bytes, seal_bytes) = read_checkpoint_pair_once(
        save_path,
        seal_path,
        expected_save_sha256,
        expected_seal_sha256,
    )?;
    let (_, _, actual_pair) = verify_checkpoint_byte_digests(
        &save_bytes,
        &seal_bytes,
        expected_save_sha256,
        expected_seal_sha256,
        expected_pair_sha256,
    )?;
    Ok(actual_pair)
}

fn verify_checkpoint_byte_digests(
    save_bytes: &[u8],
    seal_bytes: &[u8],
    expected_save_sha256: &str,
    expected_seal_sha256: &str,
    expected_pair_sha256: Option<&str>,
) -> Result<(String, String, String), ScorerError> {
    let actual_save = sha256_hex(save_bytes);
    if actual_save != expected_save_sha256 {
        return Err(ScorerError::CheckpointHashMismatch(format!(
            "save SHA-256 mismatch: expected `{expected_save_sha256}`, got `{actual_save}`"
        )));
    }
    let actual_seal = sha256_hex(seal_bytes);
    if actual_seal != expected_seal_sha256 {
        return Err(ScorerError::CheckpointHashMismatch(format!(
            "seal SHA-256 mismatch: expected `{expected_seal_sha256}`, got `{actual_seal}`"
        )));
    }
    let actual_pair = compute_pre_act_pair_sha256_hex(save_bytes, seal_bytes);
    if let Some(expected_pair) = expected_pair_sha256
        && actual_pair != expected_pair
    {
        return Err(ScorerError::CheckpointHashMismatch(format!(
            "pair SHA-256 mismatch: expected `{expected_pair}`, got `{actual_pair}`"
        )));
    }
    Ok((actual_save, actual_seal, actual_pair))
}

/// Reads the checkpoint pair once, verifies outer SHA-256 digests, and forks `EvalHost` directly
/// from the verified bytes without re-reading disk (closing double-read TOCTOU), returning the
/// verified byte buffers alongside `(host, pair_sha)` for post-batch verification.
fn verify_and_fork_checkpoint_with_bytes(
    save_path: &Path,
    seal_path: &Path,
    expected_save_sha256: &str,
    expected_seal_sha256: &str,
    expected_pair_sha256: Option<&str>,
) -> Result<(EvalHost, String, Vec<u8>, Vec<u8>), ScorerError> {
    let (save_bytes, seal_bytes) = read_checkpoint_pair_once(
        save_path,
        seal_path,
        expected_save_sha256,
        expected_seal_sha256,
    )?;
    let (_, _, pair_sha) = verify_checkpoint_byte_digests(
        &save_bytes,
        &seal_bytes,
        expected_save_sha256,
        expected_seal_sha256,
        expected_pair_sha256,
    )?;
    let host =
        EvalHost::fork_from_verified_checkpoint_bytes(&save_bytes, &seal_bytes).map_err(|e| {
            ScorerError::Engine(sanitize_error_text(&e.to_string(), &[save_path, seal_path]))
        })?;
    Ok((host, pair_sha, save_bytes, seal_bytes))
}

/// Reads the checkpoint pair once, verifies outer SHA-256 digests, and forks `EvalHost` directly
/// from the verified bytes without re-reading disk (closing double-read TOCTOU).
pub(crate) fn verify_and_fork_checkpoint(
    save_path: &Path,
    seal_path: &Path,
    expected_save_sha256: &str,
    expected_seal_sha256: &str,
    expected_pair_sha256: Option<&str>,
) -> Result<(EvalHost, String), ScorerError> {
    let (host, pair_sha, _, _) = verify_and_fork_checkpoint_with_bytes(
        save_path,
        seal_path,
        expected_save_sha256,
        expected_seal_sha256,
        expected_pair_sha256,
    )?;
    Ok((host, pair_sha))
}

struct KeptCheckpointPair<'a> {
    save_path: &'a Path,
    seal_path: &'a Path,
    save_bytes: Vec<u8>,
    seal_bytes: Vec<u8>,
    expected_save_sha256: &'a str,
    expected_seal_sha256: &'a str,
    expected_pair_sha256: &'a str,
}

fn read_post_batch_file(path: &Path, role: &str) -> Result<Vec<u8>, ScorerError> {
    fs::read(path).map_err(|err| {
        ScorerError::CheckpointChangedDuringBatch(sanitize_error_text(
            &format!("post-batch checkpoint {role} read failed: {err}"),
            &[path],
        ))
    })
}

fn check_post_batch_pair_bytes(
    save_path: &Path,
    seal_path: &Path,
    expected_save_sha256: &str,
    expected_seal_sha256: &str,
    expected_pair_sha256: &str,
    kept_bytes: Option<(&[u8], &[u8])>,
) -> Result<(), ScorerError> {
    let post_save = read_post_batch_file(save_path, "save")?;
    let post_seal = read_post_batch_file(seal_path, "seal")?;
    let actual_save = sha256_hex(&post_save);
    if actual_save != expected_save_sha256
        || kept_bytes.is_some_and(|(kept_save, _)| post_save.as_slice() != kept_save)
    {
        return Err(ScorerError::CheckpointChangedDuringBatch(format!(
            "save changed during batch: expected `{expected_save_sha256}`, got `{actual_save}`"
        )));
    }
    let actual_seal = sha256_hex(&post_seal);
    if actual_seal != expected_seal_sha256
        || kept_bytes.is_some_and(|(_, kept_seal)| post_seal.as_slice() != kept_seal)
    {
        return Err(ScorerError::CheckpointChangedDuringBatch(format!(
            "seal changed during batch: expected `{expected_seal_sha256}`, got `{actual_seal}`"
        )));
    }
    let actual_pair = compute_pre_act_pair_sha256_hex(&post_save, &post_seal);
    if actual_pair != expected_pair_sha256 {
        return Err(ScorerError::CheckpointChangedDuringBatch(format!(
            "pair changed during batch: expected `{expected_pair_sha256}`, got `{actual_pair}`"
        )));
    }
    Ok(())
}

/// Re-verifies after a batch that the on-disk `.save.json` and `.seal` pair still matches the
/// digests and bytes verified before the batch ran, refusing any pair that differs at the post-batch check.
#[doc(hidden)]
pub fn verify_checkpoint_pair_unchanged_after_batch(
    save_path: &Path,
    seal_path: &Path,
    expected_save_sha256: &str,
    expected_seal_sha256: &str,
    expected_pair_sha256: &str,
) -> Result<(), ScorerError> {
    check_post_batch_pair_bytes(
        save_path,
        seal_path,
        expected_save_sha256,
        expected_seal_sha256,
        expected_pair_sha256,
        None,
    )
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
    let (host, _) =
        verify_and_fork_checkpoint(save_path, seal_path, save_sha, seal_sha, Some(pair_sha))?;
    run_single_rollout_from_host(
        &host,
        focal_seat,
        focal_submission,
        peer_day_submissions,
        focal_mandate,
        scenario,
        posture,
        config,
    )
}

/// Clones `base_host` in memory and executes one deterministic turn-to-close counterfactual rollout.
#[allow(clippy::too_many_arguments)]
pub fn run_single_rollout_from_host(
    base_host: &EvalHost,
    focal_seat: u8,
    focal_submission: &EvalActSubmission,
    peer_day_submissions: &BTreeMap<u8, EvalActSubmission>,
    focal_mandate: PrivateMandate,
    scenario: &ScenarioAssignment,
    posture: PostureKind,
    config: &ScorerConfig,
) -> Result<(ScenarioRolloutOutcome, SeasonReplayLog), ScorerError> {
    let mut host = base_host.clone();
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
    host.apply_act_submissions(&day_subs).map_err(|e| {
        ScorerError::Engine(format!(
            "apply turn {} submissions: {e} ({e:?})",
            host.turn()
        ))
    })?;
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
        host.apply_act_submissions(&subs).map_err(|e| {
            ScorerError::Engine(format!(
                "apply turn {} rollout submissions: {e}",
                host.turn()
            ))
        })?;
        host.sweep_turn()
            .map_err(|e| ScorerError::Engine(format!("sweep turn {} rollout: {e}", host.turn())))?;
    }

    let (scoreboard, replay_log, _, _) = host
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

/// Loads the checkpoint once via single-read verification, clones `base_host` in memory across all
/// `(c, m)` rollouts (`K * M`), checks the post-batch base-state canary and post-batch on-disk pair
/// digests/bytes, and computes deterministic scores (`§4.2`, `§4.3`, `§6.1`, `§7.1`).
pub fn execute_blind_request(
    request: &BlindScoringRequest,
    config: &ScorerConfig,
    save_path: &Path,
    seal_path: &Path,
) -> Result<StateScoreOutput, ScorerError> {
    execute_blind_request_with_mid_batch_hook(request, config, save_path, seal_path, None)
}

/// Variant of [`execute_blind_request`] exposing a per-rollout mid-batch hook for fault-injection testing.
#[doc(hidden)]
pub fn execute_blind_request_with_mid_batch_hook(
    request: &BlindScoringRequest,
    config: &ScorerConfig,
    save_path: &Path,
    seal_path: &Path,
    mid_batch_hook: Option<&mut dyn FnMut(usize, usize)>,
) -> Result<StateScoreOutput, ScorerError> {
    config.validate()?;
    request.validate(config)?;

    let (base_host, _, save_bytes, seal_bytes) = verify_and_fork_checkpoint_with_bytes(
        save_path,
        seal_path,
        &request.pre_act_save_sha256,
        &request.pre_act_seal_sha256,
        Some(&request.pre_act_pair_sha256),
    )?;
    let kept = KeptCheckpointPair {
        save_path,
        seal_path,
        save_bytes,
        seal_bytes,
        expected_save_sha256: &request.pre_act_save_sha256,
        expected_seal_sha256: &request.pre_act_seal_sha256,
        expected_pair_sha256: &request.pre_act_pair_sha256,
    };
    execute_blind_request_on_loaded_host(request, config, base_host, &kept, mid_batch_hook, None)
}

/// Executes Gate 3 verified scoring against `VerifiedCheckpointsManifest` and `scorer_build_id`:
/// validates build-ID parity, checks outer file hashes on a single disk read, verifies tip canonical
/// digest against the manifest, clones `base_host` in memory per rollout, and runs the post-batch
/// base-state canary (`InMemoryBaseStateCorrupted`) and on-disk checkpoint check
/// (`CheckpointChangedDuringBatch`).
pub fn execute_blind_request_verified(
    request: &BlindScoringRequest,
    config: &ScorerConfig,
    save_path: &Path,
    seal_path: &Path,
    verified_manifest: &VerifiedCheckpointsManifest,
    scorer_build_id: &str,
) -> Result<StateScoreOutput, ScorerError> {
    execute_blind_request_verified_with_hook(
        request,
        config,
        save_path,
        seal_path,
        verified_manifest,
        scorer_build_id,
        None,
    )
}

/// Variant of [`execute_blind_request_verified`] exposing an in-memory post-batch canary hook for
/// Gate 3 fault-injection testing (`M10`).
#[doc(hidden)]
pub fn execute_blind_request_verified_with_hook(
    request: &BlindScoringRequest,
    config: &ScorerConfig,
    save_path: &Path,
    seal_path: &Path,
    verified_manifest: &VerifiedCheckpointsManifest,
    scorer_build_id: &str,
    post_batch_hook: Option<fn(&mut EvalHost)>,
) -> Result<StateScoreOutput, ScorerError> {
    execute_blind_request_verified_with_mid_batch_hook(
        request,
        config,
        save_path,
        seal_path,
        verified_manifest,
        scorer_build_id,
        None,
        post_batch_hook,
    )
}

/// Variant of [`execute_blind_request_verified`] exposing both per-rollout mid-batch and post-batch hooks.
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn execute_blind_request_verified_with_mid_batch_hook(
    request: &BlindScoringRequest,
    config: &ScorerConfig,
    save_path: &Path,
    seal_path: &Path,
    verified_manifest: &VerifiedCheckpointsManifest,
    scorer_build_id: &str,
    mid_batch_hook: Option<&mut dyn FnMut(usize, usize)>,
    post_batch_hook: Option<fn(&mut EvalHost)>,
) -> Result<StateScoreOutput, ScorerError> {
    config.validate()?;
    request.validate(config)?;
    verified_manifest.validate_for_scorer(scorer_build_id)?;

    let entry = verified_manifest
        .find_entry(&request.pre_act_pair_sha256)
        .ok_or_else(|| {
            ScorerError::CheckpointVerify(CheckpointVerifyError::OuterFileHashMismatch {
                file: "pair".to_owned(),
                expected: request.pre_act_pair_sha256.clone(),
                actual: "missing_from_verified_checkpoints_manifest".to_owned(),
            })
        })?;
    if entry.pre_act_save_sha256 != request.pre_act_save_sha256 {
        return Err(ScorerError::CheckpointVerify(
            CheckpointVerifyError::OuterFileHashMismatch {
                file: "save".to_owned(),
                expected: entry.pre_act_save_sha256.clone(),
                actual: request.pre_act_save_sha256.clone(),
            },
        ));
    }
    if entry.pre_act_seal_sha256 != request.pre_act_seal_sha256 {
        return Err(ScorerError::CheckpointVerify(
            CheckpointVerifyError::OuterFileHashMismatch {
                file: "seal".to_owned(),
                expected: entry.pre_act_seal_sha256.clone(),
                actual: request.pre_act_seal_sha256.clone(),
            },
        ));
    }

    let (save_bytes, seal_bytes) = read_checkpoint_pair_once(
        save_path,
        seal_path,
        &request.pre_act_save_sha256,
        &request.pre_act_seal_sha256,
    )?;
    let base_host = verify_and_construct_pre_act_bytes(
        &save_bytes,
        &seal_bytes,
        Some(&entry.pre_act_save_sha256),
        Some(&entry.pre_act_seal_sha256),
        Some(&entry.pre_act_pair_sha256),
        Some((entry.turn, entry.seat_count, &entry.expected_state_digest)),
    )?;
    let kept = KeptCheckpointPair {
        save_path,
        seal_path,
        save_bytes,
        seal_bytes,
        expected_save_sha256: &entry.pre_act_save_sha256,
        expected_seal_sha256: &entry.pre_act_seal_sha256,
        expected_pair_sha256: &entry.pre_act_pair_sha256,
    };
    execute_blind_request_on_loaded_host(
        request,
        config,
        base_host,
        &kept,
        mid_batch_hook,
        post_batch_hook,
    )
}

fn execute_blind_request_on_loaded_host(
    request: &BlindScoringRequest,
    config: &ScorerConfig,
    mut base_host: EvalHost,
    kept: &KeptCheckpointPair<'_>,
    mut mid_batch_hook: Option<&mut dyn FnMut(usize, usize)>,
    post_batch_hook: Option<fn(&mut EvalHost)>,
) -> Result<StateScoreOutput, ScorerError> {
    let seat_count = base_host.seat_count();
    if seat_count != EVAL_TABLE_SEAT_COUNT {
        return Err(ScorerError::InvalidCheckpointState(format!(
            "expected {EVAL_TABLE_SEAT_COUNT}-seat Evaluation table, got {seat_count}"
        )));
    }
    let day = base_host.turn();
    let pre_batch_digest = base_host
        .canonical_state_digest_hex()
        .map_err(|e| ScorerError::Engine(format!("digest pre-batch base_host: {e}")))?;
    let pre_batch_save = base_host
        .public_save_snapshot()
        .map_err(|e| ScorerError::Engine(format!("snapshot pre-batch base_host: {e}")))?;

    let focal_obs = base_host
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
            let obs_s = base_host
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

    let mut outcomes = Vec::with_capacity(request.candidates.len());
    for (c_idx, candidate) in request.candidates.iter().enumerate() {
        let mut row = Vec::with_capacity(scenarios.len());
        for (m_idx, scenario) in scenarios.iter().enumerate() {
            let posture = posture_for_scenario(config, m_idx);
            let (outcome, _) = run_single_rollout_from_host(
                &base_host,
                request.focal_seat,
                &candidate.submission,
                &peer_day_subs_by_m[m_idx],
                focal_mandate,
                scenario,
                posture,
                config,
            )?;
            row.push(outcome);
            if let Some(hook) = mid_batch_hook.as_deref_mut() {
                hook(c_idx, m_idx);
            }
        }
        outcomes.push(row);
    }

    if let Some(hook) = post_batch_hook {
        hook(&mut base_host);
    }
    let post_batch_digest = base_host
        .canonical_state_digest_hex()
        .map_err(|e| ScorerError::Engine(format!("digest post-batch base_host: {e}")))?;
    let post_batch_save = base_host
        .public_save_snapshot()
        .map_err(|e| ScorerError::Engine(format!("snapshot post-batch base_host: {e}")))?;
    if post_batch_digest != pre_batch_digest || post_batch_save != pre_batch_save {
        return Err(ScorerError::CheckpointVerify(
            CheckpointVerifyError::InMemoryBaseStateCorrupted {
                expected: pre_batch_digest,
                actual: post_batch_digest,
            },
        ));
    }

    check_post_batch_pair_bytes(
        kept.save_path,
        kept.seal_path,
        kept.expected_save_sha256,
        kept.expected_seal_sha256,
        kept.expected_pair_sha256,
        Some((&kept.save_bytes, &kept.seal_bytes)),
    )?;

    score_outcomes(request, config, day, &outcomes)
}
