//! Multi-seed branch reporting (`league-eval report --branches <dir>`, `docs/eval-harness.md`).
//!
//! Aggregates public branch outcomes across evaluated arms (`off`, `control`, `on`), tracks
//! voided runs (`MODEL_SEAT_REGENT_INTERVENTION`) per arm, and computes paired seed-level
//! comparisons (paired sign-flip test over seeds and seeded studentized cluster bootstrap
//! confidence intervals with default `B = 10,000`).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use league_core::PolicyChaChaRng;
use league_scorer::to_sorted_pretty_json;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::driver::write_text_atomic;
use crate::error::EvalError;
use crate::freeze_gate::JoinedDecisionsFile;
use crate::mechanisms::parse_json_prose_safe;
use crate::records::{
    BranchArmOutcome, BranchManifest, BranchSummary, CommunicationMode, EvalArm, EvalManifest,
    WrongPartCounter,
};

/// Schema version for [`EvalBranchReport`].
pub const REPORT_SCHEMA_VERSION: u32 = 1;

/// Default bootstrap resample count for [`run_report`].
pub const DEFAULT_REPORT_BOOTSTRAP_SAMPLES: usize = 10_000;

/// Default deterministic RNG seed for [`run_report`].
pub const DEFAULT_REPORT_BOOTSTRAP_SEED: u64 = 20_261_127;

/// Nominal two-sided confidence level for studentized cluster bootstrap intervals (`95%`).
pub const DEFAULT_REPORT_CI_LEVEL: f64 = 0.95;

/// Maximum seed cluster count using exact `2^G` sign-flip enumeration before switching to
/// deterministic Monte Carlo sign flips.
pub const EXACT_SIGN_FLIP_MAX_SEEDS: usize = 20;

/// Domain tag for studentized cluster bootstrap resampling in [`run_report`].
pub const REPORT_BOOTSTRAP_RNG_DOMAIN: &[u8] = b"league-eval/report-cluster-bootstrap/v1";

/// Domain tag for Monte Carlo sign-flip permutation draws when `G > 20`.
pub const REPORT_SIGN_FLIP_RNG_DOMAIN: &[u8] = b"league-eval/report-sign-flip/v1";

/// Stdout format selector for `league-eval report`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ReportOutputFormat {
    /// Aligned plain-text summary table.
    #[default]
    Text,
    /// Structured canonical JSON (`EvalBranchReport`).
    Json,
}

impl ReportOutputFormat {
    /// Parse `"text"` or `"json"` (case-insensitive).
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            other => Err(format!(
                "unknown report format '{other}': expected 'text' or 'json'"
            )),
        }
    }
}

/// Options for [`run_report`].
#[derive(Debug, Clone)]
pub struct ReportOptions {
    /// Directories holding `league-eval branch` outputs (`--branches <dir>...`).
    pub branch_dirs: Vec<PathBuf>,
    /// Optional unblinded `joined_decisions.json` written by `league-eval join` (`--joined <file>`).
    pub joined_path: Option<PathBuf>,
    /// Deterministic RNG seed for studentized cluster bootstrap & >20-seed sign-flips (`--bootstrap-seed <u64>`).
    pub bootstrap_seed: u64,
    /// Bootstrap resample count (`--bootstrap-samples <n>`, default `10_000`).
    pub bootstrap_samples: usize,
    /// Optional output directory receiving `report.json` and `report.txt` (`--out <dir>`).
    pub out_dir: Option<PathBuf>,
    /// Optional explicit path receiving canonical JSON report (`--out-json <file>`).
    pub out_json_path: Option<PathBuf>,
    /// Optional explicit path receiving plain-text report (`--out-text <file>`).
    pub out_text_path: Option<PathBuf>,
}

impl Default for ReportOptions {
    fn default() -> Self {
        Self {
            branch_dirs: Vec::new(),
            joined_path: None,
            bootstrap_seed: DEFAULT_REPORT_BOOTSTRAP_SEED,
            bootstrap_samples: DEFAULT_REPORT_BOOTSTRAP_SAMPLES,
            out_dir: None,
            out_json_path: None,
            out_text_path: None,
        }
    }
}

/// One seed-level paired mean difference (`arm_b - arm_a`) inside [`ReportPairwiseContrast`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeedClusterDiff {
    /// Table layout seed defining the cluster.
    pub seed: u64,
    /// Number of paired branch states contributing to this seed's cluster mean.
    pub n_paired_states: usize,
    /// Mean within-seed difference (`arm_b - arm_a`).
    pub mean_diff: f64,
}

/// Pairwise arm contrast (`arm_b - arm_a`) aggregated at the table-seed cluster level.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReportPairwiseContrast {
    /// Metric identifier (`"pbrs_delta"`, `"ev_total_equity"`, or `"score_norm"`).
    pub metric: String,
    /// Baseline arm (`arm_a`).
    pub arm_a: EvalArm,
    /// Comparison arm (`arm_b`; difference is `arm_b - arm_a`).
    pub arm_b: EvalArm,
    /// Count of paired seed clusters (`G`).
    pub n_paired_seeds: usize,
    /// Total paired branch states across those seeds.
    pub n_paired_states: usize,
    /// Mean of seed cluster differences (`sum_g D_g / G`).
    pub mean_seed_diff: Option<f64>,
    /// Pooled unweighted mean across paired states (`sum_s diff_s / n_paired_states`).
    pub mean_state_diff: Option<f64>,
    /// Sample standard error of seed cluster means (`sqrt(s^2 / G)` when `G >= 2`).
    pub se_seed_diff: Option<f64>,
    /// Two-sided paired sign-flip p-value over the `G` seed cluster means.
    pub sign_flip_p_two_sided: Option<f64>,
    /// Sign-flip method used (`"exact"`, `"monte_carlo"`, or `"insufficient_seeds"`).
    pub sign_flip_method: String,
    /// Requested bootstrap replicate count `B`.
    pub bootstrap_samples_requested: usize,
    /// Count of non-degenerate bootstrap draws included in pivotal quantiles.
    pub bootstrap_valid_draws: usize,
    /// Count of degenerate zero-variance bootstrap draws skipped.
    pub bootstrap_degenerate_draws: usize,
    /// Nominal two-sided confidence level (`0.95`).
    pub ci_level: f64,
    /// Lower bound of the seeded studentized cluster bootstrap confidence interval.
    pub studentized_ci_low: Option<f64>,
    /// Upper bound of the seeded studentized cluster bootstrap confidence interval.
    pub studentized_ci_high: Option<f64>,
    /// Ordered per-seed cluster differences used for sign-flip and bootstrap inference.
    pub seed_diffs: Vec<SeedClusterDiff>,
}

/// Per-arm summary counters and descriptive means inside [`EvalBranchReport`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReportArmSummary {
    /// Arm (`off`, `control`, `on`).
    pub arm: EvalArm,
    /// Total discovered leaf branch outcomes for this arm (including voided outcomes).
    pub n_total_outcomes: usize,
    /// Valid non-voided leaf branch outcomes included in descriptive metrics.
    pub n_valid_outcomes: usize,
    /// Voided leaf outcomes excluded from valid metrics (`MODEL_SEAT_REGENT_INTERVENTION`).
    pub n_voided_outcomes: u64,
    /// Voided games reported for this arm (`MODEL_SEAT_REGENT_INTERVENTION`).
    pub voided_games: u64,
    /// Distinct table layout seeds observed among valid outcomes for this arm.
    pub n_seeds: usize,
    /// Distinct canonical `(source_game_id, day, focal_seat)` states observed among valid outcomes.
    pub n_states: usize,
    /// Valid outcomes with `no_reply == true`.
    pub n_no_reply: usize,
    /// Valid outcomes with `parse_failure || refusal_code.is_some()`.
    pub n_reply_refusals: usize,
    /// Valid outcomes with `refused_bundle.is_some() || refused_bundle_legs > 0`.
    pub n_bundle_rejections: usize,
    /// Mean `tool_calls_used` across valid outcomes (`None` when `n_valid_outcomes == 0`).
    pub mean_tool_calls_used: Option<f64>,
    /// Mean focal `PublicSweepReceipt::pbrs_deltas[focal_seat]` across valid outcomes (`None` when `0`).
    pub mean_pbrs_delta: Option<f64>,
    /// Mean `committed_ev_total_equity` from `--joined` over non-prescreened valid states (`None` if omitted).
    pub mean_ev_total_equity: Option<f64>,
    /// Mean `committed_score_norm` from `--joined` over non-degenerate non-prescreened valid states (`None` if omitted).
    pub mean_score_norm: Option<f64>,
}

/// Complete report emitted by [`run_report`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvalBranchReport {
    /// Schema version (`1`).
    pub schema_version: u32,
    /// Homogeneous communication condition across reported runs (`talk` omitted on JSON; `silent` serialized).
    #[serde(default, skip_serializing_if = "CommunicationMode::is_talk")]
    pub communication: CommunicationMode,
    /// Deterministic RNG seed used for bootstrap & Monte Carlo sign-flips.
    pub bootstrap_seed: u64,
    /// Bootstrap resample count `B`.
    pub bootstrap_samples: usize,
    /// Two-sided confidence interval level (`0.95`).
    pub ci_level: f64,
    /// Sorted list of arms discovered across `--branches`.
    pub evaluated_arms: Vec<EvalArm>,
    /// Total distinct table layout seeds observed among valid outcomes.
    pub n_seeds: usize,
    /// Total distinct canonical branch states observed among valid outcomes.
    pub n_states: usize,
    /// Voided games reconciled per evaluated arm (`off`, `control`, `on`).
    pub voided_games_by_arm: BTreeMap<String, u64>,
    /// Aggregated wrong-part refusal counters across valid `Model` branch decisions.
    pub wrong_part_refusals: WrongPartCounter,
    /// Ordered per-arm summaries.
    pub arms: Vec<ReportArmSummary>,
    /// Pairwise seed-clustered contrasts across evaluated arms and metrics.
    pub contrasts: Vec<ReportPairwiseContrast>,
}

/// Structured outputs returned by [`run_report`].
#[derive(Debug, Clone, PartialEq)]
pub struct EvalReportResult {
    /// Structured report data.
    pub report: EvalBranchReport,
    /// Canonical pretty-printed JSON serialization.
    pub json: String,
    /// Aligned plain-text summary table.
    pub text: String,
}

#[derive(Debug, Clone)]
struct DiscoveredLeafOutcome {
    state_id: String,
    seed: u64,
    outcome: BranchArmOutcome,
    is_voided: bool,
}

#[derive(Debug, Default)]
struct ScannedBranches {
    communications: BTreeSet<CommunicationMode>,
    root_voided_by_arm: BTreeMap<String, u64>,
    leaves: Vec<DiscoveredLeafOutcome>,
    game_seeds: BTreeMap<String, u64>,
}

/// Validate `--out`, `--out-json`, `--out-text` targets do not already exist before writing.
fn resolve_output_targets(
    opts: &ReportOptions,
) -> Result<(Option<PathBuf>, Option<PathBuf>), EvalError> {
    let json_target = opts
        .out_json_path
        .clone()
        .or_else(|| opts.out_dir.as_ref().map(|d| d.join("report.json")));
    let text_target = opts
        .out_text_path
        .clone()
        .or_else(|| opts.out_dir.as_ref().map(|d| d.join("report.txt")));

    for path_opt in [&json_target, &text_target] {
        if let Some(p) = path_opt
            && p.exists()
        {
            return Err(EvalError::Usage(format!(
                "refusing to overwrite existing report output file {}",
                p.display()
            )));
        }
    }
    Ok((json_target, text_target))
}

fn scan_branch_dir_recursive(dir: &Path, acc: &mut ScannedBranches) -> Result<(), EvalError> {
    let outcome_path = dir.join("outcome.json");
    let manifest_path = dir.join("manifest.json");
    let summary_path = dir.join("branch_summary.json");

    if outcome_path.is_file() {
        let out_raw = fs::read_to_string(&outcome_path)
            .map_err(|e| EvalError::Io(format!("read {}: {e}", outcome_path.display())))?;
        let outcome: BranchArmOutcome = parse_json_prose_safe(&out_raw, &outcome_path)?;

        if !manifest_path.is_file() {
            return Err(EvalError::CheckpointNotFound(format!(
                "branch arm outcome {} is missing companion manifest.json",
                outcome_path.display()
            )));
        }
        let man_raw = fs::read_to_string(&manifest_path)
            .map_err(|e| EvalError::Io(format!("read {}: {e}", manifest_path.display())))?;
        let arm_manifest: EvalManifest = parse_json_prose_safe(&man_raw, &manifest_path)?;

        if outcome.arm != arm_manifest.arm
            || outcome.source_game_id != arm_manifest.game_id
            || outcome.table_id != arm_manifest.table_id
            || outcome.communication != arm_manifest.communication
            || outcome.decision.communication != outcome.communication
            || outcome.decision.arm != outcome.arm
            || outcome.decision.game_id != outcome.source_game_id
            || outcome.decision.table_id != outcome.table_id
            || outcome.decision.day != outcome.day
            || outcome.decision.seat != outcome.focal_seat
        {
            return Err(EvalError::IncompatibleEvalSetup(format!(
                "branch outcome metadata mismatch between {} and {}",
                outcome_path.display(),
                manifest_path.display()
            )));
        }
        if usize::from(outcome.focal_seat) >= outcome.receipt.pbrs_deltas.len() {
            return Err(EvalError::SchemaValidation(format!(
                "outcome {} focal_seat {} out of pbrs_deltas bounds ({})",
                outcome_path.display(),
                outcome.focal_seat,
                outcome.receipt.pbrs_deltas.len()
            )));
        }
        if let Some(&prev_seed) = acc.game_seeds.get(&outcome.source_game_id) {
            if prev_seed != arm_manifest.seed {
                return Err(EvalError::IncompatibleEvalSetup(format!(
                    "conflicting seeds ({prev_seed} vs {}) for source_game_id '{}'",
                    arm_manifest.seed, outcome.source_game_id
                )));
            }
        } else {
            acc.game_seeds
                .insert(outcome.source_game_id.clone(), arm_manifest.seed);
        }

        acc.communications.insert(outcome.communication);
        let is_voided = arm_manifest.voided
            || arm_manifest
                .voided_games_by_arm
                .get(outcome.arm.as_str())
                .copied()
                .unwrap_or(0)
                > 0;
        let state_id = format!(
            "{}-day-{:02}-seat-{}",
            outcome.source_game_id, outcome.day, outcome.focal_seat
        );
        acc.leaves.push(DiscoveredLeafOutcome {
            state_id,
            seed: arm_manifest.seed,
            outcome,
            is_voided,
        });
        return Ok(());
    }

    if summary_path.is_file() {
        let sum_raw = fs::read_to_string(&summary_path)
            .map_err(|e| EvalError::Io(format!("read {}: {e}", summary_path.display())))?;
        let summary: BranchSummary = parse_json_prose_safe(&sum_raw, &summary_path)?;
        for (arm_key, &count) in &summary.voided_games_by_arm {
            let entry = acc.root_voided_by_arm.entry(arm_key.clone()).or_insert(0);
            *entry = entry.saturating_add(count);
        }
    }

    if manifest_path.is_file() {
        let man_raw = fs::read_to_string(&manifest_path)
            .map_err(|e| EvalError::Io(format!("read {}: {e}", manifest_path.display())))?;
        if let Ok(branch_man) = parse_json_prose_safe::<BranchManifest>(&man_raw, &manifest_path) {
            acc.communications.insert(branch_man.communication);
            if !summary_path.is_file() {
                for (arm_key, &count) in &branch_man.voided_games_by_arm {
                    let entry = acc.root_voided_by_arm.entry(arm_key.clone()).or_insert(0);
                    *entry = entry.saturating_add(count);
                }
            }
        }
    }

    let mut subdirs = Vec::new();
    let entries = fs::read_dir(dir)
        .map_err(|e| EvalError::CheckpointNotFound(format!("read {}: {e}", dir.display())))?;
    for entry_res in entries {
        let entry = entry_res.map_err(|e| EvalError::Io(format!("read dir entry: {e}")))?;
        let ft = entry
            .file_type()
            .map_err(|e| EvalError::Io(format!("inspect entry type: {e}")))?;
        if ft.is_dir() {
            subdirs.push(entry.path());
        }
    }
    subdirs.sort();
    for sub in subdirs {
        let skip = sub
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|name| {
                matches!(
                    name,
                    "relay" | "to-seat" | "from-seat" | "receipts" | "checkpoints" | "decisions"
                )
            });
        if !skip {
            scan_branch_dir_recursive(&sub, acc)?;
        }
    }
    Ok(())
}

fn derive_report_rng(
    domain: &[u8],
    seed: u64,
    metric: &str,
    arm_a: EvalArm,
    arm_b: EvalArm,
) -> PolicyChaChaRng {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(seed.to_le_bytes());
    hasher.update(metric.as_bytes());
    hasher.update([0_u8]);
    hasher.update(arm_a.as_str().as_bytes());
    hasher.update([0_u8]);
    hasher.update(arm_b.as_str().as_bytes());
    let digest: [u8; 32] = hasher.finalize().into();
    PolicyChaChaRng::from_seed_bytes(digest)
}

/// Compute type-7 quantile (`0.0 <= p <= 1.0`) from a slice sorted via `f64::total_cmp`.
fn sorted_quantile_type7(sorted: &[f64], p: f64) -> f64 {
    let n = sorted.len();
    debug_assert!(n > 0);
    if n == 1 {
        return sorted[0];
    }
    let pos = p * ((n - 1) as f64);
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    let frac = pos - (lo as f64);
    sorted[lo] * (1.0 - frac) + sorted[hi] * frac
}

/// Compute exact (when `1 <= G <= 20`) or seeded Monte Carlo two-sided paired sign-flip p-value
/// and seeded studentized cluster bootstrap interval (`ci_level = 0.95`) over `seed_diffs`.
///
/// Replicates inside a seed are expected to already be aggregated into one cluster mean `D_g`
/// per independent seed before calling this function.
#[allow(clippy::too_many_arguments)]
pub fn compute_pairwise_contrast_from_seed_diffs(
    mut seed_diffs: Vec<SeedClusterDiff>,
    metric: &str,
    arm_a: EvalArm,
    arm_b: EvalArm,
    n_paired_states: usize,
    mean_state_diff: Option<f64>,
    bootstrap_seed: u64,
    bootstrap_samples: usize,
) -> ReportPairwiseContrast {
    seed_diffs.sort_by_key(|s| s.seed);
    let g = seed_diffs.len();
    if g == 0 {
        return ReportPairwiseContrast {
            metric: metric.to_owned(),
            arm_a,
            arm_b,
            n_paired_seeds: 0,
            n_paired_states,
            mean_seed_diff: None,
            mean_state_diff,
            se_seed_diff: None,
            sign_flip_p_two_sided: None,
            sign_flip_method: "insufficient_seeds".to_owned(),
            bootstrap_samples_requested: bootstrap_samples,
            bootstrap_valid_draws: 0,
            bootstrap_degenerate_draws: 0,
            ci_level: DEFAULT_REPORT_CI_LEVEL,
            studentized_ci_low: None,
            studentized_ci_high: None,
            seed_diffs,
        };
    }

    let d_vals: Vec<f64> = seed_diffs.iter().map(|s| s.mean_diff).collect();
    let total_sum: f64 = d_vals.iter().copied().sum();
    let bar_d = total_sum / (g as f64);

    let se = if g >= 2 {
        let mut sq_sum = 0.0_f64;
        for &d in &d_vals {
            let delta = d - bar_d;
            sq_sum += delta * delta;
        }
        let sample_var = sq_sum / ((g - 1) as f64);
        Some((sample_var / (g as f64)).sqrt())
    } else {
        None
    };

    // Paired sign-flip test across the `G` independent seed cluster means.
    let obs_abs = total_sum.abs();
    let (sign_flip_p_two_sided, sign_flip_method) = if g <= EXACT_SIGN_FLIP_MAX_SEEDS {
        let n_perms = 1_u64 << g;
        let tol = 1e-12_f64 * obs_abs;
        let p = if obs_abs == 0.0 {
            1.0
        } else {
            let mut exceed = 0_u64;
            for mask in 0..n_perms {
                let mut flipped_sum = 0.0_f64;
                for (i, &val) in d_vals.iter().enumerate() {
                    if ((mask >> i) & 1) == 1 {
                        flipped_sum -= val;
                    } else {
                        flipped_sum += val;
                    }
                }
                if flipped_sum.abs() + tol >= obs_abs {
                    exceed += 1;
                }
            }
            (exceed as f64) / (n_perms as f64)
        };
        (Some(p), "exact".to_owned())
    } else {
        let p = if obs_abs == 0.0 {
            1.0
        } else {
            let tol = 1e-12_f64 * obs_abs;
            let mut rng = derive_report_rng(
                REPORT_SIGN_FLIP_RNG_DOMAIN,
                bootstrap_seed,
                metric,
                arm_a,
                arm_b,
            );
            let mut exceed = 0_usize;
            for _ in 0..bootstrap_samples {
                let mut flipped_sum = 0.0_f64;
                for &val in &d_vals {
                    if (rng.next_u64() & 1) == 1 {
                        flipped_sum -= val;
                    } else {
                        flipped_sum += val;
                    }
                }
                if flipped_sum.abs() + tol >= obs_abs {
                    exceed += 1;
                }
            }
            ((1 + exceed) as f64) / ((1 + bootstrap_samples) as f64)
        };
        (Some(p), "monte_carlo".to_owned())
    };

    // Seeded studentized cluster bootstrap confidence interval over `G` seed clusters.
    let mut t_stats = Vec::with_capacity(bootstrap_samples);
    let mut degenerate_draws = 0_usize;
    let (ci_low, ci_high) = if let Some(se_obs) = se
        && se_obs > 0.0
    {
        let mut rng = derive_report_rng(
            REPORT_BOOTSTRAP_RNG_DOMAIN,
            bootstrap_seed,
            metric,
            arm_a,
            arm_b,
        );
        let se_floor = 1e-15_f64 * se_obs;
        let mut resampled = Vec::with_capacity(g);
        for _ in 0..bootstrap_samples {
            resampled.clear();
            let mut sum_star = 0.0_f64;
            for _ in 0..g {
                let idx = rng.gen_index(g);
                let val = d_vals[idx];
                resampled.push(val);
                sum_star += val;
            }
            let bar_star = sum_star / (g as f64);
            let mut sq_star = 0.0_f64;
            for &val in &resampled {
                let delta = val - bar_star;
                sq_star += delta * delta;
            }
            let se_star = ((sq_star / ((g - 1) as f64)) / (g as f64)).sqrt();
            if !se_star.is_finite() || se_star <= se_floor {
                degenerate_draws += 1;
                continue;
            }
            t_stats.push((bar_star - bar_d) / se_star);
        }
        if t_stats.len() >= 2 {
            t_stats.sort_by(f64::total_cmp);
            let alpha_half = (1.0 - DEFAULT_REPORT_CI_LEVEL) / 2.0;
            let q_low = sorted_quantile_type7(&t_stats, alpha_half);
            let q_high = sorted_quantile_type7(&t_stats, 1.0 - alpha_half);
            (Some(bar_d - q_high * se_obs), Some(bar_d - q_low * se_obs))
        } else {
            (None, None)
        }
    } else {
        (None, None)
    };

    ReportPairwiseContrast {
        metric: metric.to_owned(),
        arm_a,
        arm_b,
        n_paired_seeds: g,
        n_paired_states,
        mean_seed_diff: Some(bar_d),
        mean_state_diff,
        se_seed_diff: se,
        sign_flip_p_two_sided,
        sign_flip_method,
        bootstrap_samples_requested: bootstrap_samples,
        bootstrap_valid_draws: t_stats.len(),
        bootstrap_degenerate_draws: degenerate_draws,
        ci_level: DEFAULT_REPORT_CI_LEVEL,
        studentized_ci_low: ci_low,
        studentized_ci_high: ci_high,
        seed_diffs,
    }
}

fn aggregate_metric_contrast(
    metric: &str,
    arm_a: EvalArm,
    arm_b: EvalArm,
    obs_by_state: &BTreeMap<(String, u64, EvalArm), Vec<f64>>,
    state_to_seed: &BTreeMap<String, u64>,
    bootstrap_seed: u64,
    bootstrap_samples: usize,
) -> Option<ReportPairwiseContrast> {
    let mut paired_by_seed: BTreeMap<u64, Vec<f64>> = BTreeMap::new();
    let mut total_state_diff = 0.0_f64;
    let mut n_paired_states = 0_usize;

    for (state_id, &seed) in state_to_seed {
        let Some(vals_a) = obs_by_state.get(&(state_id.clone(), seed, arm_a)) else {
            continue;
        };
        let Some(vals_b) = obs_by_state.get(&(state_id.clone(), seed, arm_b)) else {
            continue;
        };
        if vals_a.is_empty() || vals_b.is_empty() {
            continue;
        }
        let mean_a = vals_a.iter().copied().sum::<f64>() / (vals_a.len() as f64);
        let mean_b = vals_b.iter().copied().sum::<f64>() / (vals_b.len() as f64);
        let diff = mean_b - mean_a;
        paired_by_seed.entry(seed).or_default().push(diff);
        total_state_diff += diff;
        n_paired_states += 1;
    }

    if n_paired_states == 0 {
        return None;
    }

    let seed_diffs: Vec<SeedClusterDiff> = paired_by_seed
        .into_iter()
        .map(|(seed, diffs)| {
            let n = diffs.len();
            let mean_diff = diffs.iter().copied().sum::<f64>() / (n as f64);
            SeedClusterDiff {
                seed,
                n_paired_states: n,
                mean_diff,
            }
        })
        .collect();
    let mean_state_diff = Some(total_state_diff / (n_paired_states as f64));

    Some(compute_pairwise_contrast_from_seed_diffs(
        seed_diffs,
        metric,
        arm_a,
        arm_b,
        n_paired_states,
        mean_state_diff,
        bootstrap_seed,
        bootstrap_samples,
    ))
}

/// Format an [`EvalBranchReport`] as an aligned plain-text summary table.
#[must_use]
pub fn render_report_text(report: &EvalBranchReport) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "MERCHANT LEAGUE EVAL REPORT (communication={}, seeds={}, states={}, bootstrap_samples={})",
        report.communication.as_str(),
        report.n_seeds,
        report.n_states,
        report.bootstrap_samples
    );
    let _ = writeln!(
        out,
        "wrong_part_refusals: delivered={}/{} ({:.4}) all_rounds={}/{} ({:.4})",
        report.wrong_part_refusals.delivered_wrong_part_refusals,
        report.wrong_part_refusals.delivered_model_replies,
        report.wrong_part_refusals.wrong_part_rate_delivered,
        report.wrong_part_refusals.total_wrong_part_refusals,
        report.wrong_part_refusals.total_model_rounds,
        report.wrong_part_refusals.wrong_part_rate_all_rounds
    );
    let _ = writeln!(out, "\nARMS & VOIDED GAMES");
    let _ = writeln!(
        out,
        "{:<8} {:>6} {:>6} {:>6} {:>6} {:>8} {:>10} {:>10} {:>10}",
        "arm",
        "valid",
        "voided",
        "vgames",
        "seeds",
        "no_reply",
        "mean_pbrs",
        "mean_ev",
        "score_norm"
    );
    for row in &report.arms {
        let pbrs = row
            .mean_pbrs_delta
            .map_or_else(|| "NA".to_owned(), |v| format!("{v:.3}"));
        let ev = row
            .mean_ev_total_equity
            .map_or_else(|| "NA".to_owned(), |v| format!("{v:.3}"));
        let norm = row
            .mean_score_norm
            .map_or_else(|| "NA".to_owned(), |v| format!("{v:.4}"));
        let _ = writeln!(
            out,
            "{:<8} {:>6} {:>6} {:>6} {:>6} {:>8} {:>10} {:>10} {:>10}",
            row.arm.as_str(),
            row.n_valid_outcomes,
            row.n_voided_outcomes,
            row.voided_games,
            row.n_seeds,
            row.n_no_reply,
            pbrs,
            ev,
            norm
        );
    }

    let _ = writeln!(out, "\nPAIRWISE SEED-CLUSTER CONTRASTS (arm_b - arm_a)");
    if report.contrasts.is_empty() {
        let _ = writeln!(out, "(none: fewer than two paired arms)");
    } else {
        let _ = writeln!(
            out,
            "{:<16} {:<16} {:>5} {:>6} {:>11} {:>10} {:>10} {:>25}",
            "metric",
            "contrast",
            "seeds",
            "states",
            "seed_diff",
            "seed_se",
            "sign_p",
            "95%_studentized_ci"
        );
        for c in &report.contrasts {
            let label = format!("{}-{}", c.arm_b.as_str(), c.arm_a.as_str());
            let mean = c
                .mean_seed_diff
                .map_or_else(|| "NA".to_owned(), |v| format!("{v:.4}"));
            let se = c
                .se_seed_diff
                .map_or_else(|| "NA".to_owned(), |v| format!("{v:.4}"));
            let pval = c
                .sign_flip_p_two_sided
                .map_or_else(|| "NA".to_owned(), |v| format!("{v:.5}"));
            let ci = match (c.studentized_ci_low, c.studentized_ci_high) {
                (Some(lo), Some(hi)) => format!("[{lo:.4}, {hi:.4}]"),
                _ => "NA".to_owned(),
            };
            let _ = writeln!(
                out,
                "{:<16} {:<16} {:>5} {:>6} {:>11} {:>10} {:>10} {:>25}",
                c.metric, label, c.n_paired_seeds, c.n_paired_states, mean, se, pval, ci
            );
        }
    }
    out
}

/// Discover branch outputs, compute seed-clustered statistics, and optionally write `report.json` / `report.txt`.
pub fn run_report(opts: &ReportOptions) -> Result<EvalReportResult, EvalError> {
    if opts.branch_dirs.is_empty() {
        return Err(EvalError::Usage(
            "`report` requires at least one `--branches <dir>` directory".to_owned(),
        ));
    }
    if opts.bootstrap_samples == 0 {
        return Err(EvalError::Usage(
            "`--bootstrap-samples` must be >= 1".to_owned(),
        ));
    }
    let (json_target, text_target) = resolve_output_targets(opts)?;

    let mut scanned = ScannedBranches::default();
    for dir in &opts.branch_dirs {
        if !dir.is_dir() {
            return Err(EvalError::CheckpointNotFound(format!(
                "`--branches` path {} does not exist or is not a directory",
                dir.display()
            )));
        }
        scan_branch_dir_recursive(dir, &mut scanned)?;
    }

    if scanned.leaves.is_empty() {
        return Err(EvalError::CheckpointNotFound(
            "no `outcome.json` branch outcomes found under `--branches`".to_owned(),
        ));
    }
    if scanned.communications.len() > 1 {
        return Err(EvalError::MixedCommunicationConditions(
            "cannot mix `talk` and `silent` branch runs in `report`".to_owned(),
        ));
    }
    let communication = scanned
        .communications
        .into_iter()
        .next()
        .unwrap_or(CommunicationMode::Talk);

    let mut valid_state_seed: BTreeMap<String, u64> = BTreeMap::new();
    let mut valid_state_arms: BTreeSet<(String, EvalArm)> = BTreeSet::new();
    let mut valid_outcomes_for_wrong_part = Vec::new();
    let mut evaluated_arms_set: BTreeSet<EvalArm> = BTreeSet::new();
    let mut all_valid_seeds: BTreeSet<u64> = BTreeSet::new();
    let mut all_valid_states: BTreeSet<String> = BTreeSet::new();

    // Per-arm tracking
    let mut total_count_by_arm: BTreeMap<EvalArm, usize> = BTreeMap::new();
    let mut voided_count_by_arm: BTreeMap<EvalArm, u64> = BTreeMap::new();
    let mut voided_games_distinct_by_arm: BTreeMap<EvalArm, BTreeSet<String>> = BTreeMap::new();
    let mut valid_leaves_by_arm: BTreeMap<EvalArm, Vec<&DiscoveredLeafOutcome>> = BTreeMap::new();

    // Metric maps: `(state_id, seed, arm) -> Vec<f64>`
    let mut pbrs_by_state: BTreeMap<(String, u64, EvalArm), Vec<f64>> = BTreeMap::new();
    let mut ev_by_state: BTreeMap<(String, u64, EvalArm), Vec<f64>> = BTreeMap::new();
    let mut norm_by_state: BTreeMap<(String, u64, EvalArm), Vec<f64>> = BTreeMap::new();

    for leaf in &scanned.leaves {
        let arm = leaf.outcome.arm;
        evaluated_arms_set.insert(arm);
        *total_count_by_arm.entry(arm).or_insert(0) += 1;

        if leaf.is_voided {
            *voided_count_by_arm.entry(arm).or_insert(0) += 1;
            voided_games_distinct_by_arm
                .entry(arm)
                .or_default()
                .insert(leaf.outcome.source_game_id.clone());
            continue;
        }

        all_valid_seeds.insert(leaf.seed);
        all_valid_states.insert(leaf.state_id.clone());
        valid_state_seed.insert(leaf.state_id.clone(), leaf.seed);
        valid_state_arms.insert((leaf.state_id.clone(), arm));
        valid_leaves_by_arm.entry(arm).or_default().push(leaf);
        valid_outcomes_for_wrong_part.push(leaf.outcome.clone());

        let pbrs_val =
            leaf.outcome.receipt.pbrs_deltas[usize::from(leaf.outcome.focal_seat)] as f64;
        pbrs_by_state
            .entry((leaf.state_id.clone(), leaf.seed, arm))
            .or_default()
            .push(pbrs_val);
    }

    if let Some(ref joined_path) = opts.joined_path {
        let raw = fs::read_to_string(joined_path)
            .map_err(|e| EvalError::Io(format!("read {}: {e}", joined_path.display())))?;
        let joined: JoinedDecisionsFile = parse_json_prose_safe(&raw, joined_path)?;
        if joined.communication != communication {
            return Err(EvalError::MixedCommunicationConditions(format!(
                "`--joined` communication '{}' disagrees with `--branches` '{}'",
                joined.communication.as_str(),
                communication.as_str()
            )));
        }
        for row in joined.rows {
            evaluated_arms_set.insert(row.arm);
            if !valid_state_arms.contains(&(row.state_id.clone(), row.arm)) {
                continue;
            }
            if row.excluded_by_prescreen {
                continue;
            }
            let Some(&seed) = valid_state_seed.get(&row.state_id) else {
                continue;
            };
            ev_by_state
                .entry((row.state_id.clone(), seed, row.arm))
                .or_default()
                .push(row.committed_ev_total_equity);
            if !row.degenerate_flat_set
                && let Some(norm) = row.committed_score_norm
            {
                norm_by_state
                    .entry((row.state_id.clone(), seed, row.arm))
                    .or_default()
                    .push(norm);
            }
        }
    }

    let evaluated_arms: Vec<EvalArm> = evaluated_arms_set.into_iter().collect();
    let mut voided_games_by_arm = BTreeMap::new();
    let mut arm_rows = Vec::with_capacity(evaluated_arms.len());

    for &arm in &evaluated_arms {
        let root_v = scanned
            .root_voided_by_arm
            .get(arm.as_str())
            .copied()
            .unwrap_or(0);
        let distinct_v = voided_games_distinct_by_arm
            .get(&arm)
            .map_or(0, |s| s.len() as u64);
        let voided_games = root_v.max(distinct_v);
        voided_games_by_arm.insert(arm.as_str().to_owned(), voided_games);

        let valid = valid_leaves_by_arm
            .get(&arm)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let n_valid = valid.len();
        let seeds_arm: BTreeSet<u64> = valid.iter().map(|l| l.seed).collect();
        let states_arm: BTreeSet<&str> = valid.iter().map(|l| l.state_id.as_str()).collect();
        let n_no_reply = valid.iter().filter(|l| l.outcome.decision.no_reply).count();
        let n_reply_refusals = valid
            .iter()
            .filter(|l| {
                l.outcome.decision.parse_failure || l.outcome.decision.refusal_code.is_some()
            })
            .count();
        let n_bundle_rejections = valid
            .iter()
            .filter(|l| {
                l.outcome.decision.refused_bundle.is_some()
                    || l.outcome.decision.refused_bundle_legs > 0
            })
            .count();
        let mean_tool_calls_used = if n_valid > 0 {
            let sum_calls: u64 = valid
                .iter()
                .map(|l| u64::from(l.outcome.decision.tool_calls_used))
                .sum();
            Some((sum_calls as f64) / (n_valid as f64))
        } else {
            None
        };
        let mean_pbrs_delta = if n_valid > 0 {
            let sum_pbrs: i128 = valid
                .iter()
                .map(|l| {
                    i128::from(l.outcome.receipt.pbrs_deltas[usize::from(l.outcome.focal_seat)])
                })
                .sum();
            Some((sum_pbrs as f64) / (n_valid as f64))
        } else {
            None
        };

        let ev_vals: Vec<f64> = ev_by_state
            .iter()
            .filter(|((_, _, a), _)| *a == arm)
            .flat_map(|(_, vs)| vs.iter().copied())
            .collect();
        let mean_ev_total_equity = (!ev_vals.is_empty())
            .then(|| ev_vals.iter().copied().sum::<f64>() / (ev_vals.len() as f64));

        let norm_vals: Vec<f64> = norm_by_state
            .iter()
            .filter(|((_, _, a), _)| *a == arm)
            .flat_map(|(_, vs)| vs.iter().copied())
            .collect();
        let mean_score_norm = (!norm_vals.is_empty())
            .then(|| norm_vals.iter().copied().sum::<f64>() / (norm_vals.len() as f64));

        arm_rows.push(ReportArmSummary {
            arm,
            n_total_outcomes: total_count_by_arm.get(&arm).copied().unwrap_or(0),
            n_valid_outcomes: n_valid,
            n_voided_outcomes: voided_count_by_arm.get(&arm).copied().unwrap_or(0),
            voided_games,
            n_seeds: seeds_arm.len(),
            n_states: states_arm.len(),
            n_no_reply,
            n_reply_refusals,
            n_bundle_rejections,
            mean_tool_calls_used,
            mean_pbrs_delta,
            mean_ev_total_equity,
            mean_score_norm,
        });
    }

    let summary_for_valid =
        crate::records::summarize_branch_outcomes(&valid_outcomes_for_wrong_part);
    let mut wrong_part_refusals = summary_for_valid.wrong_part_refusals;
    wrong_part_refusals.try_recompute_rates()?;

    let mut contrasts = Vec::new();
    for i in 0..evaluated_arms.len() {
        for j in (i + 1)..evaluated_arms.len() {
            let arm_a = evaluated_arms[i];
            let arm_b = evaluated_arms[j];
            for (metric, map) in [
                ("pbrs_delta", &pbrs_by_state),
                ("ev_total_equity", &ev_by_state),
                ("score_norm", &norm_by_state),
            ] {
                if let Some(c) = aggregate_metric_contrast(
                    metric,
                    arm_a,
                    arm_b,
                    map,
                    &valid_state_seed,
                    opts.bootstrap_seed,
                    opts.bootstrap_samples,
                ) {
                    contrasts.push(c);
                }
            }
        }
    }

    let report = EvalBranchReport {
        schema_version: REPORT_SCHEMA_VERSION,
        communication,
        bootstrap_seed: opts.bootstrap_seed,
        bootstrap_samples: opts.bootstrap_samples,
        ci_level: DEFAULT_REPORT_CI_LEVEL,
        evaluated_arms,
        n_seeds: all_valid_seeds.len(),
        n_states: all_valid_states.len(),
        voided_games_by_arm,
        wrong_part_refusals,
        arms: arm_rows,
        contrasts,
    };

    let json = format!("{}\n", to_sorted_pretty_json(&report)?);
    let text = render_report_text(&report);

    if let Some(ref dir) = opts.out_dir {
        fs::create_dir_all(dir)
            .map_err(|e| EvalError::Io(format!("create report out dir {}: {e}", dir.display())))?;
    }
    if let Some(ref p) = json_target {
        if let Some(parent) = p.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|e| {
                EvalError::Io(format!("create report json dir {}: {e}", parent.display()))
            })?;
        }
        write_text_atomic(p, &json)?;
    }
    if let Some(ref p) = text_target {
        if let Some(parent) = p.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|e| {
                EvalError::Io(format!("create report text dir {}: {e}", parent.display()))
            })?;
        }
        write_text_atomic(p, &text)?;
    }

    Ok(EvalReportResult { report, json, text })
}
