//! Release timing benchmark and determinism verification for early, mid, and late rollouts.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::error::Error;
use std::fs;
use std::io::Write as _;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use league_core::{ClubBot, EvalActSubmission, EvalHost, PrivateMandate, Seat, TableId, TurnPhase};
use league_scorer::{
    BLIND_SCORING_SCHEMA_VERSION, BlindCandidate, BlindScoringRequest, PostureKind,
    ScenarioAssignment, ScenarioRolloutOutcome, ScorerConfig, StateScoreOutput,
    TalkExtractionCounts, apply_talk_commitments, candidate_id_for_submission,
    canonicalize_talk_commitments, compute_pre_act_pair_sha256_hex, decide_peer_turn,
    draw_scenarios, execute_blind_request, generate_arm_independent_candidates,
    posture_for_scenario, run_single_rollout_from_host, score_outcomes, sha256_hex,
    to_sorted_pretty_json, verify_checkpoint_pair_unchanged_after_batch,
};
use serde::{Deserialize, Serialize};

const STAGE_SUM_REL_TOLERANCE: f64 = 0.02;
const STAGE_SUM_ABS_FLOOR_MS: f64 = 1.0;
const BENCH_ENGINE_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
const BENCH_FOCAL_SEAT: u8 = 0;
const BENCH_SCORER_SEED: u64 = 12345;
const PROCESS_JSON_PREFIX: &str = "TIMING_BENCH_PROCESS_JSON:";

const PROVIDER_KEY_ENVS: [&str; 4] = [
    "OPENROUTER_API_KEY",
    "OPENAI_API_KEY",
    "ANTHROPIC_API_KEY",
    "GEMINI_API_KEY",
];

static BENCH_TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

struct BenchTmpDir {
    path: PathBuf,
}

impl BenchTmpDir {
    fn new(label: &str) -> Result<Self, Box<dyn Error>> {
        let id = BENCH_TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = env::temp_dir().join(format!(
            "league_scorer_bench_{label}_{}_{id}",
            std::process::id()
        ));
        if dir.exists() {
            let _ = fs::remove_dir_all(&dir);
        }
        fs::create_dir_all(&dir)?;
        Ok(Self { path: dir })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for BenchTmpDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct StageBreakdownMs {
    file_read_ms: f64,
    sha256_verify_ms: f64,
    strict_decode_parse_ms: f64,
    policy_and_candidates_ms: f64,
    engine_steps_to_close_ms: f64,
}

impl StageBreakdownMs {
    fn sum_ms(&self) -> f64 {
        self.file_read_ms
            + self.sha256_verify_ms
            + self.strict_decode_parse_ms
            + self.policy_and_candidates_ms
            + self.engine_steps_to_close_ms
    }

    fn add_assign(&mut self, other: &Self) {
        self.file_read_ms += other.file_read_ms;
        self.sha256_verify_ms += other.sha256_verify_ms;
        self.strict_decode_parse_ms += other.strict_decode_parse_ms;
        self.policy_and_candidates_ms += other.policy_and_candidates_ms;
        self.engine_steps_to_close_ms += other.engine_steps_to_close_ms;
    }

    fn scaled(&self, divisor: f64) -> Self {
        Self {
            file_read_ms: self.file_read_ms / divisor,
            sha256_verify_ms: self.sha256_verify_ms / divisor,
            strict_decode_parse_ms: self.strict_decode_parse_ms / divisor,
            policy_and_candidates_ms: self.policy_and_candidates_ms / divisor,
            engine_steps_to_close_ms: self.engine_steps_to_close_ms / divisor,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StageToleranceCheck {
    stage_sum_ms: f64,
    total_ms: f64,
    abs_diff_ms: f64,
    stated_tolerance_ms: f64,
    within_tolerance: bool,
}

fn check_stage_sum_tolerance(
    stages: &StageBreakdownMs,
    total_ms: f64,
) -> Result<StageToleranceCheck, String> {
    let stage_sum_ms = stages.sum_ms();
    let abs_diff_ms = (total_ms - stage_sum_ms).abs();
    let stated_tolerance_ms = (total_ms * STAGE_SUM_REL_TOLERANCE).max(STAGE_SUM_ABS_FLOOR_MS);
    let within_tolerance = abs_diff_ms <= stated_tolerance_ms;
    if !within_tolerance {
        return Err(format!(
            "stage sum {stage_sum_ms:.3} ms differed from total {total_ms:.3} ms by {abs_diff_ms:.3} ms (> {stated_tolerance_ms:.3} ms)"
        ));
    }
    Ok(StageToleranceCheck {
        stage_sum_ms,
        total_ms,
        abs_diff_ms,
        stated_tolerance_ms,
        within_tolerance,
    })
}

struct StageClock {
    wall_start: Instant,
    stages: StageBreakdownMs,
}

impl StageClock {
    fn new() -> Self {
        Self {
            wall_start: Instant::now(),
            stages: StageBreakdownMs::default(),
        }
    }

    fn time_file_read<T, F: FnOnce() -> T>(&mut self, f: F) -> T {
        let t0 = Instant::now();
        let out = f();
        self.stages.file_read_ms += t0.elapsed().as_secs_f64() * 1000.0;
        out
    }

    fn time_sha256_verify<T, F: FnOnce() -> T>(&mut self, f: F) -> T {
        let t0 = Instant::now();
        let out = f();
        self.stages.sha256_verify_ms += t0.elapsed().as_secs_f64() * 1000.0;
        out
    }

    fn time_strict_decode_parse<T, F: FnOnce() -> T>(&mut self, f: F) -> T {
        let t0 = Instant::now();
        let out = f();
        self.stages.strict_decode_parse_ms += t0.elapsed().as_secs_f64() * 1000.0;
        out
    }

    fn time_policy_and_candidates<T, F: FnOnce() -> T>(&mut self, f: F) -> T {
        let t0 = Instant::now();
        let out = f();
        self.stages.policy_and_candidates_ms += t0.elapsed().as_secs_f64() * 1000.0;
        out
    }

    fn time_engine_steps_to_close<T, F: FnOnce() -> T>(&mut self, f: F) -> T {
        let t0 = Instant::now();
        let out = f();
        self.stages.engine_steps_to_close_ms += t0.elapsed().as_secs_f64() * 1000.0;
        out
    }

    fn finish(self) -> Result<(StageBreakdownMs, f64, StageToleranceCheck), String> {
        let total_ms = self.wall_start.elapsed().as_secs_f64() * 1000.0;
        let check = check_stage_sum_tolerance(&self.stages, total_ms)?;
        Ok((self.stages, total_ms, check))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CheckpointSpec {
    label: String,
    day: u8,
    save_path: String,
    seal_path: String,
    save_sha256: String,
    seal_sha256: String,
    pair_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FixtureManifest {
    closed_turn: u8,
    checkpoints: Vec<CheckpointSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TolerancePolicy {
    relative_max: f64,
    abs_floor_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ColdRolloutRecord {
    label: String,
    day: u8,
    stages: StageBreakdownMs,
    stage_sum_ms: f64,
    total_ms: f64,
    abs_diff_ms: f64,
    stated_tolerance_ms: f64,
    within_tolerance: bool,
    total_equity: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DecisionPointTotals {
    stages: StageBreakdownMs,
    stage_sum_ms: f64,
    total_ms: f64,
    abs_diff_ms: f64,
    stated_tolerance_ms: f64,
    within_tolerance: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RolloutMeans {
    stages: StageBreakdownMs,
    stage_sum_ms: f64,
    total_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DecisionPointTimingRecord {
    label: String,
    day: u8,
    candidate_mode: String,
    candidate_count: usize,
    scenarios_m: usize,
    rollout_count: usize,
    repeats: usize,
    first_warmup_decision_point_ms: f64,
    per_decision_point: DecisionPointTotals,
    per_rollout: RolloutMeans,
    score_json_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ThreadScalingPoint {
    threads: usize,
    wall_ms: f64,
    speedup_vs_1: f64,
    byte_identical_to_1: bool,
    score_json_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ThreadScalingReport {
    checkpoint_label: String,
    day: u8,
    candidate_mode: String,
    candidate_count: usize,
    scenarios_m: usize,
    rollout_count: usize,
    oracle_score_json_sha256: String,
    runs: Vec<ThreadScalingPoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProcessTimingRecord {
    schema_version: u32,
    process_index: usize,
    pid: u32,
    smoke: bool,
    closed_turn: u8,
    scenarios_m: usize,
    k_synth_max: usize,
    logical_cores: usize,
    oversubscribed_threads: usize,
    tolerance_policy: TolerancePolicy,
    cold_first_rollout: ColdRolloutRecord,
    decision_points: Vec<DecisionPointTimingRecord>,
    thread_scaling: ThreadScalingReport,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SpreadStatsMs {
    n_processes: usize,
    median_ms: f64,
    min_ms: f64,
    max_ms: f64,
    range_ms: f64,
    iqr_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RatioSpreadStats {
    n_processes: usize,
    median: f64,
    min: f64,
    max: f64,
    range: f64,
    iqr: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StageSpreadMs {
    file_read_ms: SpreadStatsMs,
    sha256_verify_ms: SpreadStatsMs,
    strict_decode_parse_ms: SpreadStatsMs,
    policy_and_candidates_ms: SpreadStatsMs,
    engine_steps_to_close_ms: SpreadStatsMs,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ColdSummary {
    label: String,
    day: u8,
    per_stage: StageSpreadMs,
    total_ms: SpreadStatsMs,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UnitSummary {
    per_stage: StageSpreadMs,
    total_ms: SpreadStatsMs,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DecisionPointSummary {
    label: String,
    day: u8,
    candidate_mode: String,
    candidate_count: usize,
    scenarios_m: usize,
    rollout_count: usize,
    first_warmup_decision_point_ms: SpreadStatsMs,
    per_decision_point: UnitSummary,
    per_rollout: UnitSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ThreadScalingCountSummary {
    threads: usize,
    wall_ms: SpreadStatsMs,
    speedup_vs_1: RatioSpreadStats,
    median_wall_ratio_speedup: f64,
    all_byte_identical_to_1: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ThreadScalingSummary {
    checkpoint_label: String,
    day: u8,
    candidate_mode: String,
    candidate_count: usize,
    scenarios_m: usize,
    rollout_count: usize,
    counts: Vec<ThreadScalingCountSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TimingSummaryReport {
    schema_version: u32,
    n_processes: usize,
    smoke: bool,
    logical_cores: usize,
    oversubscribed_threads: usize,
    tolerance_policy: TolerancePolicy,
    cold_first_rollout: ColdSummary,
    decision_points: Vec<DecisionPointSummary>,
    thread_scaling: ThreadScalingSummary,
}

fn generate_bench_fixture(dir: &Path) -> Result<FixtureManifest, Box<dyn Error>> {
    let ckpt_dir = dir.join("ckpts");
    fs::create_dir_all(&ckpt_dir)?;

    let mut host = EvalHost::from_standard_setup(TableId(9001), 4, 1, 42)?
        .with_engine_commit(BENCH_ENGINE_COMMIT)?;
    let mut saved_by_day: BTreeMap<u8, (PathBuf, PathBuf)> = BTreeMap::new();

    while host.phase() == TurnPhase::TalkAndMarket {
        let day = host.turn();
        let (save_p, seal_p) = host.write_pre_act_checkpoint(&ckpt_dir)?;
        saved_by_day.insert(day, (save_p, seal_p));

        let mut subs = BTreeMap::new();
        for s in 0..4_u8 {
            let obs = host.observe_seat(Seat(s))?;
            let mandate = *obs.own_mandate().ok_or("missing own_mandate")?;
            let dec =
                ClubBot::new(42_u64.wrapping_add(u64::from(s))).decide_full_turn(&obs, &mandate);
            subs.insert(
                s,
                EvalActSubmission {
                    market_actions: dec.pass_a_actions,
                    pass_b_actions: dec.pass_b_actions,
                    bundle: dec.bundle,
                },
            );
        }
        host.apply_act_submissions(&subs)?;
        host.sweep_turn()?;
    }

    let (sb, _, _, _) = host.finalize_eval_season()?;
    let closed_turn = sb.closed_turn;
    let early_day = 1_u8;
    let mid_day = (closed_turn / 2).max(2);
    let late_day = closed_turn;

    let mut checkpoints = Vec::with_capacity(3);
    for (label, day) in [("early", early_day), ("mid", mid_day), ("late", late_day)] {
        let (save_p, seal_p) = saved_by_day
            .get(&day)
            .ok_or_else(|| format!("missing checkpoint for day {day}"))?;
        let save_bytes = fs::read(save_p)?;
        let seal_bytes = fs::read(seal_p)?;
        let save_sha256 = sha256_hex(&save_bytes);
        let seal_sha256 = sha256_hex(&seal_bytes);
        let pair_sha256 = compute_pre_act_pair_sha256_hex(&save_bytes, &seal_bytes);
        let cas_save = ckpt_dir.join(format!("{save_sha256}.save.json"));
        let cas_seal = ckpt_dir.join(format!("{seal_sha256}.seal"));
        fs::write(&cas_save, &save_bytes)?;
        fs::write(&cas_seal, &seal_bytes)?;
        checkpoints.push(CheckpointSpec {
            label: label.to_owned(),
            day,
            save_path: cas_save.to_string_lossy().into_owned(),
            seal_path: cas_seal.to_string_lossy().into_owned(),
            save_sha256,
            seal_sha256,
            pair_sha256,
        });
    }

    let manifest = FixtureManifest {
        closed_turn,
        checkpoints,
    };
    fs::write(
        dir.join("fixture_manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    Ok(manifest)
}

fn verify_checkpoint_bytes_for_bench(
    save_bytes: &[u8],
    seal_bytes: &[u8],
    spec: &CheckpointSpec,
) -> Result<(), String> {
    let actual_save = sha256_hex(save_bytes);
    if actual_save != spec.save_sha256 {
        return Err(format!(
            "save SHA-256 mismatch: expected {}, got {actual_save}",
            spec.save_sha256
        ));
    }
    let actual_seal = sha256_hex(seal_bytes);
    if actual_seal != spec.seal_sha256 {
        return Err(format!(
            "seal SHA-256 mismatch: expected {}, got {actual_seal}",
            spec.seal_sha256
        ));
    }
    let actual_pair = compute_pre_act_pair_sha256_hex(save_bytes, seal_bytes);
    if actual_pair != spec.pair_sha256 {
        return Err(format!(
            "pair SHA-256 mismatch: expected {}, got {actual_pair}",
            spec.pair_sha256
        ));
    }
    Ok(())
}

fn build_request_from_host(
    base_host: &EvalHost,
    spec: &CheckpointSpec,
    cfg: &ScorerConfig,
    candidate_mode: &str,
) -> Result<(BlindScoringRequest, PrivateMandate), Box<dyn Error>> {
    let focal_obs = base_host.observe_seat(Seat(BENCH_FOCAL_SEAT))?;
    let focal_mandate = *focal_obs.own_mandate().ok_or("missing focal own_mandate")?;
    let candidates = match candidate_mode {
        "passive_1" => {
            let sub = EvalActSubmission::default();
            let cid = candidate_id_for_submission(&sub)?;
            vec![BlindCandidate {
                candidate_id: cid,
                in_span: true,
                is_arm_independent: true,
                submission: sub,
            }]
        }
        "arm_indep" => {
            generate_arm_independent_candidates(&focal_obs, &focal_mandate, cfg, BENCH_SCORER_SEED)?
        }
        other => return Err(format!("unknown candidate_mode `{other}`").into()),
    };
    let req = BlindScoringRequest {
        candidates,
        focal_seat: BENCH_FOCAL_SEAT,
        pre_act_pair_sha256: spec.pair_sha256.clone(),
        pre_act_save_sha256: spec.save_sha256.clone(),
        pre_act_seal_sha256: spec.seal_sha256.clone(),
        schema_version: BLIND_SCORING_SCHEMA_VERSION,
        scorer_config_sha256: cfg.sha256_hex()?,
        scorer_seed: BENCH_SCORER_SEED,
        state_id: format!("bench_{}_{}", spec.label, candidate_mode),
        talk_commitments: Vec::new(),
        talk_extraction_counts: TalkExtractionCounts::default(),
    };
    req.validate(cfg)?;
    Ok((req, focal_mandate))
}

#[allow(clippy::too_many_arguments)]
fn step_timed_rollout(
    clock: &mut StageClock,
    base_host: &EvalHost,
    focal_seat: u8,
    focal_submission: &EvalActSubmission,
    peer_day_subs: &BTreeMap<u8, EvalActSubmission>,
    focal_mandate: PrivateMandate,
    scenario: &ScenarioAssignment,
    posture: PostureKind,
    cfg: &ScorerConfig,
) -> Result<ScenarioRolloutOutcome, Box<dyn Error>> {
    let mut host = clock.time_engine_steps_to_close(|| -> Result<EvalHost, Box<dyn Error>> {
        let mut host = base_host.clone();
        let mut day_subs = peer_day_subs.clone();
        day_subs.insert(focal_seat, focal_submission.clone());
        host.apply_act_submissions(&day_subs)?;
        host.sweep_turn()?;
        Ok(host)
    })?;
    let seat_count = host.seat_count();

    while host.phase() == TurnPhase::TalkAndMarket {
        let subs = clock.time_policy_and_candidates(
            || -> Result<BTreeMap<u8, EvalActSubmission>, Box<dyn Error>> {
                let mut subs = BTreeMap::new();
                for s in 0..seat_count {
                    let obs = host.observe_seat(Seat(s))?;
                    let seed = scenario.bot_base_seed.wrapping_add(u64::from(s));
                    let sub = if s == focal_seat {
                        let dec = ClubBot::new(seed).decide_full_turn(&obs, &focal_mandate);
                        EvalActSubmission {
                            market_actions: dec.pass_a_actions,
                            pass_b_actions: dec.pass_b_actions,
                            bundle: dec.bundle,
                        }
                    } else {
                        let mandate = *scenario
                            .peer_mandates
                            .get(&s)
                            .ok_or("missing peer mandate")?;
                        decide_peer_turn(posture, seed, &obs, &mandate, cfg)?
                    };
                    subs.insert(s, sub);
                }
                Ok(subs)
            },
        )?;

        clock.time_engine_steps_to_close(|| -> Result<(), Box<dyn Error>> {
            host.apply_act_submissions(&subs)?;
            host.sweep_turn()?;
            Ok(())
        })?;
    }

    clock.time_engine_steps_to_close(|| -> Result<ScenarioRolloutOutcome, Box<dyn Error>> {
        let (scoreboard, _, _, _) = host.finalize_eval_season()?;
        let entry = scoreboard
            .standings
            .iter()
            .find(|e| e.seat == focal_seat)
            .ok_or("focal seat missing from FinalScoreboard")?;
        Ok(ScenarioRolloutOutcome {
            mandate_bonus: entry.mandate_bonus,
            net_asset_equity: entry.net_asset_equity,
            project_equity: entry.project_equity,
            total_equity: entry.total_equity,
        })
    })
}

fn build_peer_day_subs_by_m(
    base_host: &EvalHost,
    req: &BlindScoringRequest,
    scenarios: &[ScenarioAssignment],
    cfg: &ScorerConfig,
) -> Result<Vec<BTreeMap<u8, EvalActSubmission>>, Box<dyn Error>> {
    let seat_count = base_host.seat_count();
    let canonical_commitments = canonicalize_talk_commitments(&req.talk_commitments)?;
    let mut peer_observations = Vec::with_capacity(usize::from(seat_count.saturating_sub(1)));
    for s in 0..seat_count {
        if s != req.focal_seat {
            peer_observations.push((s, base_host.observe_seat(Seat(s))?));
        }
    }
    let mut peer_day_subs_by_m = Vec::with_capacity(scenarios.len());
    for (m_idx, scenario) in scenarios.iter().enumerate() {
        let posture = posture_for_scenario(cfg, m_idx);
        let mut peer_subs = BTreeMap::new();
        for (s, obs_s) in &peer_observations {
            let mandate = *scenario
                .peer_mandates
                .get(s)
                .ok_or("missing peer mandate")?;
            let seed = scenario.bot_base_seed.wrapping_add(u64::from(*s));
            let base_sub = decide_peer_turn(posture, seed, obs_s, &mandate, cfg)?;
            let final_sub = apply_talk_commitments(
                obs_s,
                base_sub,
                &canonical_commitments,
                req.scorer_seed,
                &req.pre_act_pair_sha256,
                m_idx,
                cfg.p_honour,
            )?;
            peer_subs.insert(*s, final_sub);
        }
        peer_day_subs_by_m.push(peer_subs);
    }
    Ok(peer_day_subs_by_m)
}

fn time_cold_first_rollout(
    spec: &CheckpointSpec,
    cfg: &ScorerConfig,
) -> Result<ColdRolloutRecord, Box<dyn Error>> {
    time_cold_first_rollout_with_untimed_gap(spec, cfg, None)
}

fn time_cold_first_rollout_with_untimed_gap(
    spec: &CheckpointSpec,
    cfg: &ScorerConfig,
    untimed_gap: Option<Duration>,
) -> Result<ColdRolloutRecord, Box<dyn Error>> {
    let save_path = Path::new(&spec.save_path);
    let seal_path = Path::new(&spec.seal_path);
    let mut clock = StageClock::new();

    let (save_bytes, seal_bytes) =
        clock.time_file_read(|| -> Result<(Vec<u8>, Vec<u8>), Box<dyn Error>> {
            Ok((fs::read(save_path)?, fs::read(seal_path)?))
        })?;

    clock
        .time_sha256_verify(|| verify_checkpoint_bytes_for_bench(&save_bytes, &seal_bytes, spec))?;

    let base_host = clock.time_strict_decode_parse(|| {
        EvalHost::fork_from_verified_checkpoint_bytes(&save_bytes, &seal_bytes)
    })?;

    let (req, focal_mandate, scenarios, peer_day_subs_by_m) =
        clock.time_policy_and_candidates(|| -> Result<_, Box<dyn Error>> {
            let day = base_host.turn();
            let (req, focal_mandate) = build_request_from_host(&base_host, spec, cfg, "passive_1")?;
            let focal_obs = base_host.observe_seat(Seat(BENCH_FOCAL_SEAT))?;
            let scenarios = draw_scenarios(
                &focal_obs.spec,
                focal_mandate,
                req.focal_seat,
                req.scorer_seed,
                &req.pre_act_pair_sha256,
                day,
                cfg.scenarios_m,
            )?;
            let peer_day_subs_by_m = build_peer_day_subs_by_m(&base_host, &req, &scenarios, cfg)?;
            Ok((req, focal_mandate, scenarios, peer_day_subs_by_m))
        })?;

    let posture = posture_for_scenario(cfg, 0);
    let outcome = step_timed_rollout(
        &mut clock,
        &base_host,
        req.focal_seat,
        &req.candidates[0].submission,
        &peer_day_subs_by_m[0],
        focal_mandate,
        &scenarios[0],
        posture,
        cfg,
    )?;

    if let Some(gap) = untimed_gap {
        std::thread::sleep(gap);
    }

    let (stages, total_ms, tol) = clock.finish()?;
    Ok(ColdRolloutRecord {
        label: spec.label.clone(),
        day: spec.day,
        stages,
        stage_sum_ms: tol.stage_sum_ms,
        total_ms,
        abs_diff_ms: tol.abs_diff_ms,
        stated_tolerance_ms: tol.stated_tolerance_ms,
        within_tolerance: tol.within_tolerance,
        total_equity: outcome.total_equity,
    })
}

struct SingleDecisionPointSample {
    stages: StageBreakdownMs,
    total_ms: f64,
    request: BlindScoringRequest,
    score_json: String,
}

fn time_decision_point_once(
    spec: &CheckpointSpec,
    cfg: &ScorerConfig,
    candidate_mode: &str,
) -> Result<SingleDecisionPointSample, Box<dyn Error>> {
    let save_path = Path::new(&spec.save_path);
    let seal_path = Path::new(&spec.seal_path);
    let mut clock = StageClock::new();

    let (save_bytes, seal_bytes) =
        clock.time_file_read(|| -> Result<(Vec<u8>, Vec<u8>), Box<dyn Error>> {
            Ok((fs::read(save_path)?, fs::read(seal_path)?))
        })?;

    clock
        .time_sha256_verify(|| verify_checkpoint_bytes_for_bench(&save_bytes, &seal_bytes, spec))?;

    let base_host = clock.time_strict_decode_parse(|| {
        EvalHost::fork_from_verified_checkpoint_bytes(&save_bytes, &seal_bytes)
    })?;

    let day = base_host.turn();
    let (pre_digest, pre_save) =
        clock.time_engine_steps_to_close(|| -> Result<_, Box<dyn Error>> {
            Ok((
                base_host.canonical_state_digest_hex()?,
                base_host.public_save_snapshot()?,
            ))
        })?;

    let (req, focal_mandate, scenarios, peer_day_subs_by_m) =
        clock.time_policy_and_candidates(|| -> Result<_, Box<dyn Error>> {
            let (req, focal_mandate) =
                build_request_from_host(&base_host, spec, cfg, candidate_mode)?;
            let focal_obs = base_host.observe_seat(Seat(BENCH_FOCAL_SEAT))?;
            let scenarios = draw_scenarios(
                &focal_obs.spec,
                focal_mandate,
                req.focal_seat,
                req.scorer_seed,
                &req.pre_act_pair_sha256,
                day,
                cfg.scenarios_m,
            )?;
            let peer_day_subs_by_m = build_peer_day_subs_by_m(&base_host, &req, &scenarios, cfg)?;
            Ok((req, focal_mandate, scenarios, peer_day_subs_by_m))
        })?;

    let mut outcomes = Vec::with_capacity(req.candidates.len());
    for candidate in &req.candidates {
        let mut row = Vec::with_capacity(scenarios.len());
        for (m_idx, scenario) in scenarios.iter().enumerate() {
            let posture = posture_for_scenario(cfg, m_idx);
            let outcome = step_timed_rollout(
                &mut clock,
                &base_host,
                req.focal_seat,
                &candidate.submission,
                &peer_day_subs_by_m[m_idx],
                focal_mandate,
                scenario,
                posture,
                cfg,
            )?;
            row.push(outcome);
        }
        outcomes.push(row);
    }

    clock.time_engine_steps_to_close(|| -> Result<(), Box<dyn Error>> {
        let post_digest = base_host.canonical_state_digest_hex()?;
        let post_save = base_host.public_save_snapshot()?;
        if post_digest != pre_digest || post_save != pre_save {
            return Err("in-memory base_host diverged during timed decision-point batch".into());
        }
        Ok(())
    })?;

    let (post_save_bytes, post_seal_bytes) =
        clock.time_file_read(|| -> Result<(Vec<u8>, Vec<u8>), Box<dyn Error>> {
            Ok((fs::read(save_path)?, fs::read(seal_path)?))
        })?;

    clock.time_sha256_verify(|| -> Result<(), Box<dyn Error>> {
        verify_checkpoint_bytes_for_bench(&post_save_bytes, &post_seal_bytes, spec)?;
        if post_save_bytes != save_bytes || post_seal_bytes != seal_bytes {
            return Err("checkpoint bytes changed during timed decision-point batch".into());
        }
        Ok(())
    })?;

    let score_json = clock.time_engine_steps_to_close(|| -> Result<String, Box<dyn Error>> {
        let score_out = score_outcomes(&req, cfg, day, &outcomes)?;
        Ok(to_sorted_pretty_json(&score_out)?)
    })?;

    let (stages, total_ms, _tol) = clock.finish()?;
    Ok(SingleDecisionPointSample {
        stages,
        total_ms,
        request: req,
        score_json,
    })
}

fn measure_decision_point_warm(
    spec: &CheckpointSpec,
    cfg: &ScorerConfig,
    candidate_mode: &str,
    repeats: usize,
) -> Result<DecisionPointTimingRecord, Box<dyn Error>> {
    let save_path = Path::new(&spec.save_path);
    let seal_path = Path::new(&spec.seal_path);

    let warm = time_decision_point_once(spec, cfg, candidate_mode)?;
    let oracle_out = execute_blind_request(&warm.request, cfg, save_path, seal_path)?;
    let oracle_json = to_sorted_pretty_json(&oracle_out)?;
    if warm.score_json != oracle_json {
        return Err(format!(
            "timed decision point {} ({candidate_mode}) diverged from execute_blind_request",
            spec.label
        )
        .into());
    }
    let oracle_sha = sha256_hex(oracle_json.as_bytes());

    let mut sum_stages = StageBreakdownMs::default();
    let mut sum_total_ms = 0.0_f64;
    for _ in 0..repeats {
        let rep = time_decision_point_once(spec, cfg, candidate_mode)?;
        if rep.score_json != oracle_json {
            return Err("steady-state repeat diverged from oracle score JSON".into());
        }
        sum_stages.add_assign(&rep.stages);
        sum_total_ms += rep.total_ms;
    }

    let rep_f = repeats as f64;
    let dp_stages = sum_stages.scaled(rep_f);
    let dp_total_ms = sum_total_ms / rep_f;
    let dp_tol = check_stage_sum_tolerance(&dp_stages, dp_total_ms)?;

    let candidate_count = warm.request.candidates.len();
    let rollout_count = candidate_count * cfg.scenarios_m;
    let rollout_stages = dp_stages.scaled(rollout_count as f64);
    let rollout_total_ms = dp_total_ms / (rollout_count as f64);

    Ok(DecisionPointTimingRecord {
        label: spec.label.clone(),
        day: spec.day,
        candidate_mode: candidate_mode.to_owned(),
        candidate_count,
        scenarios_m: cfg.scenarios_m,
        rollout_count,
        repeats,
        first_warmup_decision_point_ms: warm.total_ms,
        per_decision_point: DecisionPointTotals {
            stages: dp_stages,
            stage_sum_ms: dp_tol.stage_sum_ms,
            total_ms: dp_total_ms,
            abs_diff_ms: dp_tol.abs_diff_ms,
            stated_tolerance_ms: dp_tol.stated_tolerance_ms,
            within_tolerance: dp_tol.within_tolerance,
        },
        per_rollout: RolloutMeans {
            stage_sum_ms: rollout_stages.sum_ms(),
            stages: rollout_stages,
            total_ms: rollout_total_ms,
        },
        score_json_sha256: oracle_sha,
    })
}

fn thread_counts_to_measure() -> (usize, usize, Vec<usize>) {
    let logical_cores = std::thread::available_parallelism()
        .map(NonZeroUsize::get)
        .unwrap_or(1);
    let oversubscribed = logical_cores.saturating_mul(2).max(32);
    let mut set = BTreeSet::new();
    for n in [1, 2, 4, 7, 8, logical_cores, oversubscribed] {
        set.insert(n);
    }
    (logical_cores, oversubscribed, set.into_iter().collect())
}

fn run_scoped_thread_batch_for_bench(
    req: &BlindScoringRequest,
    cfg: &ScorerConfig,
    save_path: &Path,
    seal_path: &Path,
    thread_count: usize,
    panic_on_task: Option<usize>,
) -> Result<StateScoreOutput, Box<dyn Error>> {
    if thread_count == 0 {
        return Err("thread_count must be >= 1".into());
    }
    cfg.validate()?;
    req.validate(cfg)?;

    let save_bytes = fs::read(save_path)?;
    let seal_bytes = fs::read(seal_path)?;
    if sha256_hex(&save_bytes) != req.pre_act_save_sha256 {
        return Err("save SHA-256 mismatch before scoped batch".into());
    }
    if sha256_hex(&seal_bytes) != req.pre_act_seal_sha256 {
        return Err("seal SHA-256 mismatch before scoped batch".into());
    }
    if compute_pre_act_pair_sha256_hex(&save_bytes, &seal_bytes) != req.pre_act_pair_sha256 {
        return Err("pair SHA-256 mismatch before scoped batch".into());
    }

    let base_host = EvalHost::fork_from_verified_checkpoint_bytes(&save_bytes, &seal_bytes)?;
    let day = base_host.turn();
    let pre_digest = base_host.canonical_state_digest_hex()?;
    let pre_save = base_host.public_save_snapshot()?;
    let focal_obs = base_host.observe_seat(Seat(req.focal_seat))?;
    let focal_mandate = *focal_obs
        .own_mandate()
        .ok_or("focal SeatObservation omitted own_mandate")?;
    let scenarios = draw_scenarios(
        &focal_obs.spec,
        focal_mandate,
        req.focal_seat,
        req.scorer_seed,
        &req.pre_act_pair_sha256,
        day,
        cfg.scenarios_m,
    )?;
    let peer_day_subs_by_m = build_peer_day_subs_by_m(&base_host, req, &scenarios, cfg)?;

    let mut tasks = Vec::with_capacity(req.candidates.len() * scenarios.len());
    for c_idx in 0..req.candidates.len() {
        for m_idx in 0..scenarios.len() {
            tasks.push((c_idx, m_idx));
        }
    }

    let cursor = AtomicUsize::new(0);
    let mut slots: Vec<Option<ScenarioRolloutOutcome>> = vec![None; tasks.len()];
    let cursor_ref = &cursor;
    let tasks_ref = &tasks;
    let base_host_ref = &base_host;
    let peer_subs_ref = &peer_day_subs_by_m;
    let scenarios_ref = &scenarios;

    std::thread::scope(|s| -> Result<(), String> {
        let mut handles = Vec::with_capacity(thread_count);
        for worker_id in 0..thread_count {
            let handle = std::thread::Builder::new()
                .name(format!("scorer-bench-{worker_id}"))
                .spawn_scoped(
                    s,
                    move || -> Result<Vec<(usize, ScenarioRolloutOutcome)>, String> {
                        let mut local = Vec::new();
                        loop {
                            let idx = cursor_ref.fetch_add(1, Ordering::Relaxed);
                            if idx >= tasks_ref.len() {
                                break;
                            }
                            if panic_on_task == Some(idx) {
                                panic!("injected scoped worker panic at task {idx}");
                            }
                            let (c_idx, m_idx) = tasks_ref[idx];
                            let posture = posture_for_scenario(cfg, m_idx);
                            let (outcome, _) = run_single_rollout_from_host(
                                base_host_ref,
                                req.focal_seat,
                                &req.candidates[c_idx].submission,
                                &peer_subs_ref[m_idx],
                                focal_mandate,
                                &scenarios_ref[m_idx],
                                posture,
                                cfg,
                            )
                            .map_err(|e| e.to_string())?;
                            local.push((idx, outcome));
                        }
                        Ok(local)
                    },
                )
                .map_err(|e| format!("spawn scoped worker {worker_id}: {e}"))?;
            handles.push(handle);
        }

        for handle in handles {
            let worker_results = handle
                .join()
                .map_err(|_| "scoped worker thread panicked".to_owned())??;
            for (idx, outcome) in worker_results {
                if slots[idx].replace(outcome).is_some() {
                    return Err(format!("duplicate rollout write at task {idx}"));
                }
            }
        }
        Ok(())
    })?;

    let mut outcomes_opt = vec![vec![None; scenarios.len()]; req.candidates.len()];
    for (idx, maybe_outcome) in slots.into_iter().enumerate() {
        let outcome = maybe_outcome.ok_or_else(|| format!("missing rollout result {idx}"))?;
        let (c_idx, m_idx) = tasks[idx];
        outcomes_opt[c_idx][m_idx] = Some(outcome);
    }
    let mut outcomes = Vec::with_capacity(req.candidates.len());
    for row in outcomes_opt {
        let mut clean_row = Vec::with_capacity(scenarios.len());
        for entry in row {
            clean_row.push(entry.ok_or("missing rollout cell after scatter")?);
        }
        outcomes.push(clean_row);
    }

    let post_digest = base_host.canonical_state_digest_hex()?;
    let post_save = base_host.public_save_snapshot()?;
    if post_digest != pre_digest || post_save != pre_save {
        return Err("in-memory base_host mutated during scoped batch".into());
    }
    verify_checkpoint_pair_unchanged_after_batch(
        save_path,
        seal_path,
        &req.pre_act_save_sha256,
        &req.pre_act_seal_sha256,
        &req.pre_act_pair_sha256,
    )?;

    Ok(score_outcomes(req, cfg, day, &outcomes)?)
}

fn measure_thread_scaling(
    spec: &CheckpointSpec,
    cfg: &ScorerConfig,
    candidate_mode: &str,
    counts: &[usize],
) -> Result<ThreadScalingReport, Box<dyn Error>> {
    let save_bytes = fs::read(&spec.save_path)?;
    let seal_bytes = fs::read(&spec.seal_path)?;
    let base_host = EvalHost::fork_from_verified_checkpoint_bytes(&save_bytes, &seal_bytes)?;
    let (req, _) = build_request_from_host(&base_host, spec, cfg, candidate_mode)?;
    let save_path = Path::new(&spec.save_path);
    let seal_path = Path::new(&spec.seal_path);

    let oracle = execute_blind_request(&req, cfg, save_path, seal_path)?;
    let oracle_json = to_sorted_pretty_json(&oracle)?;
    let oracle_sha = sha256_hex(oracle_json.as_bytes());

    let mut baseline_json: Option<String> = None;
    let mut baseline_ms = 0.0_f64;
    let mut runs = Vec::with_capacity(counts.len());

    for &threads in counts {
        let t0 = Instant::now();
        let out =
            run_scoped_thread_batch_for_bench(&req, cfg, save_path, seal_path, threads, None)?;
        let wall_ms = t0.elapsed().as_secs_f64() * 1000.0;
        let out_json = to_sorted_pretty_json(&out)?;
        let out_sha = sha256_hex(out_json.as_bytes());
        if out_json != oracle_json {
            return Err(
                format!("scoped thread count {threads} diverged from sequential oracle").into(),
            );
        }
        let byte_identical_to_1 = match baseline_json.as_ref() {
            None => {
                baseline_json = Some(out_json);
                baseline_ms = wall_ms;
                true
            }
            Some(ref_json) => {
                if &out_json != ref_json {
                    return Err(format!(
                        "scoped thread count {threads} was not byte-identical to 1-thread output"
                    )
                    .into());
                }
                true
            }
        };
        let speedup_vs_1 = if wall_ms > 0.0 {
            baseline_ms / wall_ms
        } else {
            1.0
        };
        runs.push(ThreadScalingPoint {
            threads,
            wall_ms,
            speedup_vs_1,
            byte_identical_to_1,
            score_json_sha256: out_sha,
        });
    }

    Ok(ThreadScalingReport {
        checkpoint_label: spec.label.clone(),
        day: spec.day,
        candidate_mode: candidate_mode.to_owned(),
        candidate_count: req.candidates.len(),
        scenarios_m: cfg.scenarios_m,
        rollout_count: req.candidates.len() * cfg.scenarios_m,
        oracle_score_json_sha256: oracle_sha,
        runs,
    })
}

fn bench_config(smoke: bool) -> (ScorerConfig, usize, usize) {
    if smoke {
        let cfg = ScorerConfig {
            k_synth_max: 1,
            scenarios_m: 2,
            ..ScorerConfig::default()
        };
        (cfg, 1, 1)
    } else {
        (ScorerConfig::default(), 3, 1)
    }
}

fn run_worker(
    manifest: &FixtureManifest,
    process_index: usize,
    smoke: bool,
    micro: bool,
) -> Result<ProcessTimingRecord, Box<dyn Error>> {
    let (cfg, passive_repeats, arm_indep_repeats) = bench_config(smoke);
    let cold_spec = manifest
        .checkpoints
        .last()
        .filter(|_| micro)
        .or_else(|| manifest.checkpoints.first())
        .ok_or("fixture manifest has zero checkpoints")?;

    // Must run first in this process before any warmup or steady-state work.
    let cold_first_rollout = time_cold_first_rollout(cold_spec, &cfg)?;

    let selected_checkpoints: Vec<&CheckpointSpec> = if micro {
        vec![cold_spec]
    } else {
        manifest.checkpoints.iter().collect()
    };

    let mut decision_points = Vec::with_capacity(selected_checkpoints.len() * 2);
    for spec in &selected_checkpoints {
        decision_points.push(measure_decision_point_warm(
            spec,
            &cfg,
            "passive_1",
            passive_repeats,
        )?);
        decision_points.push(measure_decision_point_warm(
            spec,
            &cfg,
            "arm_indep",
            arm_indep_repeats,
        )?);
    }

    let (logical_cores, oversubscribed_threads, counts) = if micro {
        (
            std::thread::available_parallelism()
                .map(NonZeroUsize::get)
                .unwrap_or(1),
            32,
            vec![1, 2],
        )
    } else {
        thread_counts_to_measure()
    };
    let scaling_spec = selected_checkpoints
        .first()
        .copied()
        .ok_or("no checkpoint available for thread scaling")?;
    let thread_scaling = measure_thread_scaling(scaling_spec, &cfg, "arm_indep", &counts)?;

    Ok(ProcessTimingRecord {
        schema_version: 1,
        process_index,
        pid: std::process::id(),
        smoke,
        closed_turn: manifest.closed_turn,
        scenarios_m: cfg.scenarios_m,
        k_synth_max: cfg.k_synth_max,
        logical_cores,
        oversubscribed_threads,
        tolerance_policy: TolerancePolicy {
            relative_max: STAGE_SUM_REL_TOLERANCE,
            abs_floor_ms: STAGE_SUM_ABS_FLOOR_MS,
        },
        cold_first_rollout,
        decision_points,
        thread_scaling,
    })
}

fn median_sorted(sorted: &[f64]) -> f64 {
    let n = sorted.len();
    if n == 0 {
        0.0
    } else if n % 2 == 1 {
        sorted[n / 2]
    } else {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    }
}

fn compute_spread_stats(values: &[f64]) -> Result<SpreadStatsMs, String> {
    if values.is_empty() {
        return Err("cannot compute spread stats on empty slice".to_owned());
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let n = sorted.len();
    let min_ms = sorted[0];
    let max_ms = sorted[n - 1];
    let range_ms = max_ms - min_ms;
    let median_ms = median_sorted(&sorted);
    let iqr_ms = if n == 1 {
        0.0
    } else {
        let q1 = median_sorted(&sorted[..n / 2]);
        let q3 = median_sorted(&sorted[n.div_ceil(2)..]);
        q3 - q1
    };
    Ok(SpreadStatsMs {
        n_processes: n,
        median_ms,
        min_ms,
        max_ms,
        range_ms,
        iqr_ms,
    })
}

fn compute_ratio_spread(values: &[f64]) -> Result<RatioSpreadStats, String> {
    let s = compute_spread_stats(values)?;
    Ok(RatioSpreadStats {
        n_processes: s.n_processes,
        median: s.median_ms,
        min: s.min_ms,
        max: s.max_ms,
        range: s.range_ms,
        iqr: s.iqr_ms,
    })
}

fn compute_stage_spread<'a, I>(stages_iter: I) -> Result<StageSpreadMs, String>
where
    I: Iterator<Item = &'a StageBreakdownMs>,
{
    let mut file_read = Vec::new();
    let mut sha256_verify = Vec::new();
    let mut strict_decode_parse = Vec::new();
    let mut policy_and_candidates = Vec::new();
    let mut engine_steps_to_close = Vec::new();
    for s in stages_iter {
        file_read.push(s.file_read_ms);
        sha256_verify.push(s.sha256_verify_ms);
        strict_decode_parse.push(s.strict_decode_parse_ms);
        policy_and_candidates.push(s.policy_and_candidates_ms);
        engine_steps_to_close.push(s.engine_steps_to_close_ms);
    }
    Ok(StageSpreadMs {
        file_read_ms: compute_spread_stats(&file_read)?,
        sha256_verify_ms: compute_spread_stats(&sha256_verify)?,
        strict_decode_parse_ms: compute_spread_stats(&strict_decode_parse)?,
        policy_and_candidates_ms: compute_spread_stats(&policy_and_candidates)?,
        engine_steps_to_close_ms: compute_spread_stats(&engine_steps_to_close)?,
    })
}

fn summarize_process_records(
    records: &[ProcessTimingRecord],
) -> Result<TimingSummaryReport, String> {
    let first = records
        .first()
        .ok_or_else(|| "cannot summarize empty process record list".to_owned())?;

    let cold_totals: Vec<f64> = records
        .iter()
        .map(|r| r.cold_first_rollout.total_ms)
        .collect();
    let cold_summary = ColdSummary {
        label: first.cold_first_rollout.label.clone(),
        day: first.cold_first_rollout.day,
        per_stage: compute_stage_spread(records.iter().map(|r| &r.cold_first_rollout.stages))?,
        total_ms: compute_spread_stats(&cold_totals)?,
    };

    let mut dp_summaries = Vec::with_capacity(first.decision_points.len());
    for (idx, dp_template) in first.decision_points.iter().enumerate() {
        let mut warmups = Vec::with_capacity(records.len());
        let mut dp_totals = Vec::with_capacity(records.len());
        let mut ro_totals = Vec::with_capacity(records.len());
        let mut dp_stages_list = Vec::with_capacity(records.len());
        let mut ro_stages_list = Vec::with_capacity(records.len());

        for rec in records {
            let dp = rec.decision_points.get(idx).ok_or_else(|| {
                format!("process {} missing decision_point {idx}", rec.process_index)
            })?;
            if dp.label != dp_template.label || dp.candidate_mode != dp_template.candidate_mode {
                return Err(format!(
                    "decision_point mismatch at {idx}: expected ({}, {}), got ({}, {})",
                    dp_template.label, dp_template.candidate_mode, dp.label, dp.candidate_mode
                ));
            }
            warmups.push(dp.first_warmup_decision_point_ms);
            dp_totals.push(dp.per_decision_point.total_ms);
            ro_totals.push(dp.per_rollout.total_ms);
            dp_stages_list.push(&dp.per_decision_point.stages);
            ro_stages_list.push(&dp.per_rollout.stages);
        }

        dp_summaries.push(DecisionPointSummary {
            label: dp_template.label.clone(),
            day: dp_template.day,
            candidate_mode: dp_template.candidate_mode.clone(),
            candidate_count: dp_template.candidate_count,
            scenarios_m: dp_template.scenarios_m,
            rollout_count: dp_template.rollout_count,
            first_warmup_decision_point_ms: compute_spread_stats(&warmups)?,
            per_decision_point: UnitSummary {
                per_stage: compute_stage_spread(dp_stages_list.into_iter())?,
                total_ms: compute_spread_stats(&dp_totals)?,
            },
            per_rollout: UnitSummary {
                per_stage: compute_stage_spread(ro_stages_list.into_iter())?,
                total_ms: compute_spread_stats(&ro_totals)?,
            },
        });
    }

    let mut count_summaries = Vec::with_capacity(first.thread_scaling.runs.len());
    let mut median_wall_1 = 0.0_f64;
    for (idx, run_template) in first.thread_scaling.runs.iter().enumerate() {
        let mut walls = Vec::with_capacity(records.len());
        let mut speedups = Vec::with_capacity(records.len());
        let mut all_identical = true;
        for rec in records {
            let pt = rec.thread_scaling.runs.get(idx).ok_or_else(|| {
                format!("process {} missing thread_scaling {idx}", rec.process_index)
            })?;
            if pt.threads != run_template.threads {
                return Err(format!(
                    "thread count mismatch at {idx}: expected {}, got {}",
                    run_template.threads, pt.threads
                ));
            }
            walls.push(pt.wall_ms);
            speedups.push(pt.speedup_vs_1);
            all_identical &= pt.byte_identical_to_1;
        }
        let wall_ms = compute_spread_stats(&walls)?;
        if idx == 0 {
            median_wall_1 = wall_ms.median_ms;
        }
        let median_wall_ratio_speedup = if wall_ms.median_ms > 0.0 {
            median_wall_1 / wall_ms.median_ms
        } else {
            1.0
        };
        count_summaries.push(ThreadScalingCountSummary {
            threads: run_template.threads,
            wall_ms,
            speedup_vs_1: compute_ratio_spread(&speedups)?,
            median_wall_ratio_speedup,
            all_byte_identical_to_1: all_identical,
        });
    }

    Ok(TimingSummaryReport {
        schema_version: 1,
        n_processes: records.len(),
        smoke: first.smoke,
        logical_cores: first.logical_cores,
        oversubscribed_threads: first.oversubscribed_threads,
        tolerance_policy: first.tolerance_policy.clone(),
        cold_first_rollout: cold_summary,
        decision_points: dp_summaries,
        thread_scaling: ThreadScalingSummary {
            checkpoint_label: first.thread_scaling.checkpoint_label.clone(),
            day: first.thread_scaling.day,
            candidate_mode: first.thread_scaling.candidate_mode.clone(),
            candidate_count: first.thread_scaling.candidate_count,
            scenarios_m: first.thread_scaling.scenarios_m,
            rollout_count: first.thread_scaling.rollout_count,
            counts: count_summaries,
        },
    })
}

fn print_human_summary(summary: &TimingSummaryReport) {
    let cold = &summary.cold_first_rollout;
    println!(
        "TIMING_SUMMARY n_processes={} smoke={} logical_cores={} oversubscribed_threads={}",
        summary.n_processes, summary.smoke, summary.logical_cores, summary.oversubscribed_threads
    );
    println!(
        "COLD_FIRST_ROLLOUT label={} day={} total_median_ms={:.3} (min={:.3}, max={:.3}, range={:.3}, iqr={:.3}) stages_median_ms=[read={:.3}, sha256={:.3}, decode={:.3}, policy={:.3}, engine={:.3}]",
        cold.label,
        cold.day,
        cold.total_ms.median_ms,
        cold.total_ms.min_ms,
        cold.total_ms.max_ms,
        cold.total_ms.range_ms,
        cold.total_ms.iqr_ms,
        cold.per_stage.file_read_ms.median_ms,
        cold.per_stage.sha256_verify_ms.median_ms,
        cold.per_stage.strict_decode_parse_ms.median_ms,
        cold.per_stage.policy_and_candidates_ms.median_ms,
        cold.per_stage.engine_steps_to_close_ms.median_ms,
    );
    for dp in &summary.decision_points {
        println!(
            "DECISION_POINT label={} day={} mode={} K={} M={} rollouts={} warmup_median_ms={:.3} dp_median_ms={:.3} (range={:.3}, iqr={:.3}) dp_stages_median_ms=[read={:.3}, sha256={:.3}, decode={:.3}, policy={:.3}, engine={:.3}] per_rollout_median_ms={:.3} per_rollout_stages_median_ms=[read={:.3}, sha256={:.3}, decode={:.3}, policy={:.3}, engine={:.3}]",
            dp.label,
            dp.day,
            dp.candidate_mode,
            dp.candidate_count,
            dp.scenarios_m,
            dp.rollout_count,
            dp.first_warmup_decision_point_ms.median_ms,
            dp.per_decision_point.total_ms.median_ms,
            dp.per_decision_point.total_ms.range_ms,
            dp.per_decision_point.total_ms.iqr_ms,
            dp.per_decision_point.per_stage.file_read_ms.median_ms,
            dp.per_decision_point.per_stage.sha256_verify_ms.median_ms,
            dp.per_decision_point
                .per_stage
                .strict_decode_parse_ms
                .median_ms,
            dp.per_decision_point
                .per_stage
                .policy_and_candidates_ms
                .median_ms,
            dp.per_decision_point
                .per_stage
                .engine_steps_to_close_ms
                .median_ms,
            dp.per_rollout.total_ms.median_ms,
            dp.per_rollout.per_stage.file_read_ms.median_ms,
            dp.per_rollout.per_stage.sha256_verify_ms.median_ms,
            dp.per_rollout.per_stage.strict_decode_parse_ms.median_ms,
            dp.per_rollout.per_stage.policy_and_candidates_ms.median_ms,
            dp.per_rollout.per_stage.engine_steps_to_close_ms.median_ms,
        );
    }
    let ts = &summary.thread_scaling;
    for pt in &ts.counts {
        println!(
            "THREAD_SCALING label={} day={} mode={} rollouts={} threads={} wall_median_ms={:.3} speedup_median={:.3}x wall_ratio_speedup={:.3}x byte_identical={}",
            ts.checkpoint_label,
            ts.day,
            ts.candidate_mode,
            ts.rollout_count,
            pt.threads,
            pt.wall_ms.median_ms,
            pt.speedup_vs_1.median,
            pt.median_wall_ratio_speedup,
            pt.all_byte_identical_to_1,
        );
    }
}

fn spawn_hermetic_bench_command(exe: &Path) -> Command {
    let mut cmd = Command::new(exe);
    cmd.stdin(Stdio::null());
    for key in PROVIDER_KEY_ENVS {
        cmd.env_remove(key);
    }
    cmd
}

#[test]
#[ignore]
fn timing_benchmark_early_mid_late_rollouts() -> Result<(), Box<dyn Error>> {
    let mode = env::var("LEAGUE_TIMING_MODE").unwrap_or_default();
    let smoke = env::var("LEAGUE_TIMING_SMOKE").as_deref() == Ok("1");
    let micro = env::var("LEAGUE_TIMING_MICRO").as_deref() == Ok("1");

    match mode.as_str() {
        "setup" => {
            let dir = PathBuf::from(env::var("LEAGUE_TIMING_FIXTURE_DIR")?);
            fs::create_dir_all(&dir)?;
            let manifest = generate_bench_fixture(&dir)?;
            println!("TIMING_BENCH_SETUP closed_turn={}", manifest.closed_turn);
            Ok(())
        }
        "worker" => {
            let dir = PathBuf::from(env::var("LEAGUE_TIMING_FIXTURE_DIR")?);
            let proc_idx: usize = env::var("LEAGUE_TIMING_PROCESS_INDEX")
                .unwrap_or_else(|_| "0".to_owned())
                .parse()?;
            let manifest: FixtureManifest =
                serde_json::from_slice(&fs::read(dir.join("fixture_manifest.json"))?)?;
            let record = run_worker(&manifest, proc_idx, smoke, micro)?;
            let line = serde_json::to_string(&record)?;
            if let Ok(out_path) = env::var("LEAGUE_TIMING_WORKER_OUT") {
                fs::write(out_path, format!("{line}\n"))?;
            } else {
                println!("{PROCESS_JSON_PREFIX}{line}");
            }
            Ok(())
        }
        "aggregate" => {
            let jsonl_in = PathBuf::from(env::var("LEAGUE_TIMING_JSONL_IN")?);
            let summary_out = PathBuf::from(env::var("LEAGUE_TIMING_SUMMARY_OUT")?);
            let raw = fs::read_to_string(&jsonl_in)?;
            let mut records = Vec::new();
            for line in raw.lines() {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    records.push(serde_json::from_str::<ProcessTimingRecord>(trimmed)?);
                }
            }
            let summary = summarize_process_records(&records)?;
            fs::write(&summary_out, serde_json::to_vec_pretty(&summary)?)?;
            print_human_summary(&summary);
            Ok(())
        }
        "" => {
            let tmp = BenchTmpDir::new("standalone")?;
            let manifest = generate_bench_fixture(tmp.path())?;
            let record = run_worker(&manifest, 0, smoke, micro)?;
            let jsonl_line = serde_json::to_string(&record)?;
            let summary = summarize_process_records(&[record])?;
            println!("{PROCESS_JSON_PREFIX}{jsonl_line}");
            print_human_summary(&summary);
            Ok(())
        }
        other => Err(format!("unsupported LEAGUE_TIMING_MODE `{other}`").into()),
    }
}

#[test]
fn stage_times_sum_to_total_within_stated_tolerance() -> Result<(), Box<dyn Error>> {
    let tmp = BenchTmpDir::new("stage_tol")?;
    let manifest = generate_bench_fixture(tmp.path())?;
    let late_spec = manifest
        .checkpoints
        .last()
        .ok_or("missing late checkpoint")?;
    let cfg = ScorerConfig {
        k_synth_max: 1,
        scenarios_m: 2,
        ..ScorerConfig::default()
    };

    let cold = time_cold_first_rollout(late_spec, &cfg)?;
    assert!(cold.within_tolerance);
    assert!(cold.stages.file_read_ms >= 0.0);
    assert!(cold.stages.sha256_verify_ms >= 0.0);
    assert!(cold.stages.strict_decode_parse_ms >= 0.0);
    assert!(cold.stages.policy_and_candidates_ms >= 0.0);
    assert!(cold.stages.engine_steps_to_close_ms >= 0.0);

    let dp = measure_decision_point_warm(late_spec, &cfg, "arm_indep", 1)?;
    assert!(dp.per_decision_point.within_tolerance);
    assert!(
        dp.per_decision_point.abs_diff_ms <= dp.per_decision_point.stated_tolerance_ms,
        "abs_diff={} > tol={}",
        dp.per_decision_point.abs_diff_ms,
        dp.per_decision_point.stated_tolerance_ms
    );

    // Negative check: mismatching stage sum outside tolerance fails cleanly.
    let bad_stages = StageBreakdownMs {
        file_read_ms: 1.0,
        sha256_verify_ms: 1.0,
        strict_decode_parse_ms: 1.0,
        policy_and_candidates_ms: 1.0,
        engine_steps_to_close_ms: 1.0,
    };
    assert!(check_stage_sum_tolerance(&bad_stages, 50.0).is_err());
    Ok(())
}

#[test]
fn stage_sum_check_fails_on_untimed_gap() -> Result<(), Box<dyn Error>> {
    // 1. Direct StageClock span: an untimed sleep inside the independent outer timer makes
    //    `finish()` reject the stage sum against `wall_start.elapsed()`.
    let gap_clock = StageClock::new();
    std::thread::sleep(Duration::from_millis(25));
    let clock_err = gap_clock
        .finish()
        .err()
        .ok_or("expected untimed sleep in StageClock to fail stage-sum check")?;
    assert!(
        clock_err.contains("differed from total"),
        "clock_err={clock_err}"
    );

    // 2. Real timed rollout: an untimed 200 ms gap inside `StageClock` exceeds the
    //    `max(2%, 1.0 ms)` tolerance even under unoptimized/coverage builds.
    let tmp = BenchTmpDir::new("stage_untimed_gap")?;
    let manifest = generate_bench_fixture(tmp.path())?;
    let late_spec = manifest
        .checkpoints
        .last()
        .ok_or("missing late checkpoint")?;
    let cfg = ScorerConfig {
        k_synth_max: 1,
        scenarios_m: 2,
        ..ScorerConfig::default()
    };

    let clean = time_cold_first_rollout(late_spec, &cfg)?;
    assert!(clean.within_tolerance);

    let rollout_gap_err =
        time_cold_first_rollout_with_untimed_gap(late_spec, &cfg, Some(Duration::from_millis(200)))
            .err()
            .ok_or("expected untimed gap inside rollout to fail stage-sum check")?;
    let rollout_gap_msg = rollout_gap_err.to_string();
    assert!(
        rollout_gap_msg.contains("differed from total"),
        "rollout_gap_msg={rollout_gap_msg}"
    );
    Ok(())
}

#[test]
fn scoped_thread_batch_is_byte_identical_across_counts_and_propagates_worker_panic()
-> Result<(), Box<dyn Error>> {
    let tmp = BenchTmpDir::new("scoped_threads")?;
    let manifest = generate_bench_fixture(tmp.path())?;
    let late_spec = manifest
        .checkpoints
        .last()
        .ok_or("missing late checkpoint")?;
    let cfg = ScorerConfig {
        k_synth_max: 1,
        scenarios_m: 2,
        ..ScorerConfig::default()
    };

    let (logical_cores, oversubscribed, counts) = thread_counts_to_measure();
    assert!(counts.contains(&1));
    assert!(counts.contains(&2));
    assert!(counts.contains(&4));
    assert!(counts.contains(&7));
    assert!(counts.contains(&8));
    assert!(counts.contains(&logical_cores));
    assert!(counts.contains(&oversubscribed));

    let report = measure_thread_scaling(late_spec, &cfg, "arm_indep", &counts)?;
    assert_ne!(
        report.rollout_count % 7,
        0,
        "rollout_count ({}) must exercise non-divisor 7",
        report.rollout_count
    );
    assert_eq!(report.runs.len(), counts.len());
    for pt in &report.runs {
        assert!(pt.byte_identical_to_1);
        assert_eq!(pt.score_json_sha256, report.oracle_score_json_sha256);
    }

    // Worker panic must fail the entire scoped batch rather than returning partial results.
    let save_bytes = fs::read(&late_spec.save_path)?;
    let seal_bytes = fs::read(&late_spec.seal_path)?;
    let base_host = EvalHost::fork_from_verified_checkpoint_bytes(&save_bytes, &seal_bytes)?;
    let (req, _) = build_request_from_host(&base_host, late_spec, &cfg, "arm_indep")?;
    let panic_res = run_scoped_thread_batch_for_bench(
        &req,
        &cfg,
        Path::new(&late_spec.save_path),
        Path::new(&late_spec.seal_path),
        4,
        Some(1),
    );
    assert!(panic_res.is_err(), "worker panic must fail the whole batch");
    Ok(())
}

#[test]
fn cold_vs_warm_worker_subprocess_and_multiprocess_median_spread_summary()
-> Result<(), Box<dyn Error>> {
    let tmp = BenchTmpDir::new("subproc_summary")?;
    let fixture_dir = tmp.path().join("fixture");
    fs::create_dir_all(&fixture_dir)?;
    let _ = generate_bench_fixture(&fixture_dir)?;

    let worker_out = tmp.path().join("worker_0.json");
    let exe = env::current_exe()?;
    let output = spawn_hermetic_bench_command(&exe)
        .args([
            "--exact",
            "timing_benchmark_early_mid_late_rollouts",
            "--ignored",
            "--nocapture",
        ])
        .env("LEAGUE_TIMING_MODE", "worker")
        .env("LEAGUE_TIMING_FIXTURE_DIR", &fixture_dir)
        .env("LEAGUE_TIMING_PROCESS_INDEX", "0")
        .env("LEAGUE_TIMING_WORKER_OUT", &worker_out)
        .env("LEAGUE_TIMING_SMOKE", "1")
        .env("LEAGUE_TIMING_MICRO", "1")
        .output()?;
    assert!(
        output.status.success(),
        "worker subprocess failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let line = fs::read_to_string(&worker_out)?;
    let rec: ProcessTimingRecord = serde_json::from_str(line.trim())?;
    assert!(rec.smoke);
    assert!(rec.cold_first_rollout.within_tolerance);
    assert_eq!(rec.decision_points.len(), 2);
    assert!(rec.decision_points[0].per_decision_point.within_tolerance);
    assert!(rec.decision_points[1].per_decision_point.within_tolerance);
    assert!(
        rec.thread_scaling
            .runs
            .iter()
            .all(|r| r.byte_identical_to_1)
    );

    // Verify median and spread calculations over a synthetic 5-process vector.
    let mut recs = Vec::with_capacity(5);
    for idx in 0..5_usize {
        let mut r = rec.clone();
        let mult = (idx + 1) as f64;
        r.process_index = idx;
        r.cold_first_rollout.total_ms = 10.0 * mult;
        r.cold_first_rollout.stages.file_read_ms = 2.0 * mult;
        r.decision_points[0].per_decision_point.total_ms = 20.0 * mult;
        r.decision_points[0].per_rollout.total_ms = 10.0 * mult;
        r.decision_points[0]
            .per_decision_point
            .stages
            .engine_steps_to_close_ms = 5.0 * mult;
        r.thread_scaling.runs[0].wall_ms = 100.0;
        r.thread_scaling.runs[1].wall_ms = 50.0;
        recs.push(r);
    }

    let jsonl_path = tmp.path().join("procs.jsonl");
    let mut jsonl_buf = Vec::new();
    for r in &recs {
        serde_json::to_writer(&mut jsonl_buf, r)?;
        jsonl_buf.write_all(b"\n")?;
    }
    fs::write(&jsonl_path, &jsonl_buf)?;

    let summary_path = tmp.path().join("summary.json");
    let agg_out = spawn_hermetic_bench_command(&exe)
        .args([
            "--exact",
            "timing_benchmark_early_mid_late_rollouts",
            "--ignored",
            "--nocapture",
        ])
        .env("LEAGUE_TIMING_MODE", "aggregate")
        .env("LEAGUE_TIMING_JSONL_IN", &jsonl_path)
        .env("LEAGUE_TIMING_SUMMARY_OUT", &summary_path)
        .output()?;
    assert!(agg_out.status.success());

    let summary: TimingSummaryReport = serde_json::from_slice(&fs::read(&summary_path)?)?;
    assert_eq!(summary.n_processes, 5);
    assert!((summary.cold_first_rollout.total_ms.median_ms - 30.0).abs() < 1e-9);
    assert!((summary.cold_first_rollout.total_ms.min_ms - 10.0).abs() < 1e-9);
    assert!((summary.cold_first_rollout.total_ms.max_ms - 50.0).abs() < 1e-9);
    assert!((summary.cold_first_rollout.total_ms.range_ms - 40.0).abs() < 1e-9);
    assert!((summary.cold_first_rollout.total_ms.iqr_ms - 30.0).abs() < 1e-9);
    assert!(
        (summary.decision_points[0]
            .per_decision_point
            .total_ms
            .median_ms
            - 60.0)
            .abs()
            < 1e-9
    );
    assert!((summary.decision_points[0].per_rollout.total_ms.median_ms - 30.0).abs() < 1e-9);
    assert!((summary.thread_scaling.counts[1].median_wall_ratio_speedup - 2.0).abs() < 1e-9);

    Ok(())
}
