//! Pre-registration freeze writer and fail-closed unblinding join gate (`§7.1`, `§8.2`).
//!
//! - `run_freeze` refuses dirty git worktrees (`DIRTY_GIT_WORKTREE`) and refuses overwriting an
//!   existing freeze file (`FREEZE_FILE_EXISTS`), recording `scorer_freeze.json`.
//! - `run_join` refuses missing freezes (`MISSING_FREEZE_FILE`), blind-input manifest mismatches
//!   (`FREEZE_MANIFEST_MISMATCH`), config hash mismatches (`FREEZE_CONFIG_MISMATCH`), and `control`
//!   wording-arm mismatches (`CONTROL_WORDING_ARM_MISMATCH`), before emitting descriptive-only
//!   per-arm tables.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use league_scorer::{
    BLIND_SCORING_SCHEMA_VERSION, CandidateScoreOutput, ScorerConfig, StateScoreOutput,
    TALK_EXTRACTOR_VERSION_V2, TalkExtractionCounts, TalkSourceCounts,
    compute_blind_inputs_manifest_sha256, compute_blind_inputs_manifest_sha256_from_entries,
    compute_scorer_tree_sha256, is_valid_sha256_hex, parse_blind_scoring_request_strict,
    to_sorted_pretty_json,
};
use serde::{Deserialize, Serialize};

use crate::blind_builder::{ArmMapEntry, ArmMapStateFile};
use crate::error::EvalError;
use crate::mechanisms::{
    DecisionMechanismMetrics, MECHANISM_DATA_LIMITS, PerArmMechanismSummary,
    compute_decision_mechanisms, index_source_game_dirs, summarize_arm_mechanisms,
};
use crate::records::{EvalArm, EvalManifest, EvalSetupFile};
use crate::serving::{CrateGitShaSource, resolve_crate_git_sha_from};
use crate::talk_extract::talk_rules_v2_digest;

/// Descriptive-only inferential tag required by `§7.1`.
pub const DESCRIPTIVE_ONLY_INFERENTIAL_STATUS: &str = "descriptive_only";

/// Options for [`run_freeze`].
#[derive(Debug, Clone)]
pub struct FreezeOptions {
    /// Directory containing anonymized `blind_inputs/*.json` requests (`--blind <dir>`).
    pub blind_dir: PathBuf,
    /// Path to frozen [`ScorerConfig`] JSON (`--config <file>`).
    pub config_path: PathBuf,
    /// Exact `control` wording arm label used identically in pilot and main runs (`--control-wording-arm <label>`).
    pub control_wording_arm: String,
    /// Output path for write-once `scorer_freeze.json` (`--out <file>`).
    pub out_path: PathBuf,
    /// Optional path to `crates/league-scorer` directory (defaults to compile-time sibling path).
    pub scorer_crate_dir: Option<PathBuf>,
}

/// Write-once freeze specification written to `scorer_freeze.json` (`§8.2` step 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScorerFreezeFile {
    /// Sorted manifest SHA-256 over `blind_inputs/*.json`.
    pub blind_inputs_manifest_sha256: String,
    /// Exact control wording arm label (`r2` legality check).
    pub control_wording_arm: String,
    /// Canonical compact-JSON SHA-256 of frozen [`ScorerConfig`].
    pub scorer_config_sha256: String,
    /// Verified 40-hex git commit SHA of clean workspace.
    pub scorer_git_sha: String,
    /// Canonical tree SHA-256 over `crates/league-scorer/**`.
    pub scorer_tree_sha256: String,
    /// Canonical SHA-256 over the `talk_rules_v2` rule table and LF-normalized extractor source.
    pub talk_extractor_rules_digest: String,
    /// Talk commitment extractor version (`talk_rules_v2`).
    pub talk_extractor_version: String,
}

impl ScorerFreezeFile {
    /// Validates freeze field syntax before using it for scoring or joining.
    pub fn validate(&self) -> Result<(), EvalError> {
        if self.scorer_git_sha.len() != 40
            || !self.scorer_git_sha.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(EvalError::InvalidCrateGitSha(format!(
                "invalid scorer_git_sha `{}` in freeze file",
                self.scorer_git_sha
            )));
        }
        for (field, val) in [
            ("scorer_tree_sha256", &self.scorer_tree_sha256),
            ("scorer_config_sha256", &self.scorer_config_sha256),
            (
                "talk_extractor_rules_digest",
                &self.talk_extractor_rules_digest,
            ),
            (
                "blind_inputs_manifest_sha256",
                &self.blind_inputs_manifest_sha256,
            ),
        ] {
            if !is_valid_sha256_hex(val) {
                return Err(EvalError::SchemaValidation(format!(
                    "malformed 64-hex SHA-256 in freeze `{field}`: `{val}`"
                )));
            }
        }
        if self.control_wording_arm.trim().is_empty() {
            return Err(EvalError::ControlWordingArmMismatch(
                "freeze `control_wording_arm` cannot be empty".to_owned(),
            ));
        }
        if self.talk_extractor_version.trim().is_empty() {
            return Err(EvalError::SchemaValidation(
                "freeze `talk_extractor_version` cannot be empty".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Options for [`run_join`].
#[derive(Debug, Clone)]
pub struct JoinOptions {
    /// Path to pre-registered `scorer_freeze.json` (`--freeze <file>`).
    pub freeze_path: PathBuf,
    /// Disjoint `arm_map/` directory written by `build-blind-inputs` (`--arm-map <dir>`).
    pub arm_map_dir: PathBuf,
    /// Directory containing `<state_id>.json` or `scores/<state_id>.json` from `league-scorer` (`--scores <dir>`).
    pub scores_dir: PathBuf,
    /// Output directory where descriptive joined tables are written (`--out <dir>`).
    pub out_dir: PathBuf,
    /// Optional explicit `blind_inputs/` directory to cross-check directly on disk.
    pub blind_dir: Option<PathBuf>,
    /// Optional source game directories carrying `setup.json`, `checkpoints/`, `decisions/`,
    /// `to-seat/`, and optional `scoreboard.json` for §7.2 Measure 5(a)/(b) evaluation.
    pub source_game_dirs: Vec<PathBuf>,
}

/// One joined `(state_id, arm, replicate)` row linking blind scores to provenance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JoinedDecisionRow {
    /// Source canonical `state_id`.
    pub state_id: String,
    /// 1-based checkpoint day.
    pub day: u8,
    /// Zero-based focal seat index.
    pub focal_seat: u8,
    /// Arm (`off`, `control`, `on`).
    pub arm: EvalArm,
    /// Deterministic zero-based replicate index within `(state_id, arm)`.
    pub replicate: u32,
    /// Committed candidate ID.
    pub committed_candidate_id: String,
    /// Optional first-draft candidate ID.
    pub first_draft_candidate_id: Option<String>,
    /// Mean total equity for the committed submission (`Units`).
    pub committed_ev_total_equity: f64,
    /// Primary normalized `[0, 1]` score for the committed submission (`None` on flat span).
    pub committed_score_norm: Option<f64>,
    /// Regret relative to `v_max` (`Units`).
    pub committed_regret_equity: f64,
    /// Sensitivity percentile relative strictly to `C_indep(state)`.
    pub committed_sensitivity_indep_percentile: Option<f64>,
    /// Sensitivity rank relative strictly to `C_indep(state)`.
    pub committed_sensitivity_indep_rank: Option<usize>,
    /// Mean total equity for the first draft, when present (`Units`).
    pub first_draft_ev_total_equity: Option<f64>,
    /// Normalized score for the first draft, when present (`None` on flat span).
    pub first_draft_score_norm: Option<f64>,
    /// Whether this state failed the arm-blind pre-screen (`§3.2`).
    pub excluded_by_prescreen: bool,
    /// Whether `v_max == v_min` (`§7.1`).
    pub degenerate_flat_set: bool,
    /// Per-state talk message extraction counts echoed from `StateScoreOutput`.
    #[serde(default)]
    pub talk_extraction_counts: TalkExtractionCounts,
    /// Unblinded §7.2 descriptive mechanism metrics (`inferential_status: "descriptive_only"`).
    #[serde(default)]
    pub mechanisms: DecisionMechanismMetrics,
    /// Unblinded `decision.json` provenance path.
    pub decision_path: PathBuf,
    /// Unblinded `outcome.json` provenance path.
    pub outcome_path: PathBuf,
}

/// Descriptive distribution of one per-state count across states in an arm (`§7.1`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CountDistribution {
    /// Maximum count observed across `n_states` (`0` when `n_states == 0`).
    pub max: u64,
    /// Arithmetic mean across `n_states` (`None` when `n_states == 0`).
    pub mean: Option<f64>,
    /// Median across `n_states` (`None` when `n_states == 0`).
    pub median: Option<f64>,
    /// Minimum count observed across `n_states` (`0` when `n_states == 0`).
    pub min: u64,
    /// Number of distinct states aggregated.
    pub n_states: usize,
    /// Sum of counts across `n_states`.
    pub total: u64,
}

/// Descriptive per-arm talk message count totals and per-state distributions (`§7.1`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PerArmTalkCountsSummary {
    /// Number of distinct states observed under this arm.
    pub n_states: usize,
    /// Element-wise sums across distinct states in this arm (`negotiation`, `talk`, `total`).
    pub totals: TalkExtractionCounts,
    /// Per-state distribution of `total.seen`.
    pub per_state_seen: CountDistribution,
    /// Per-state distribution of `total.matched`.
    pub per_state_matched: CountDistribution,
    /// Per-state distribution of `total.ambiguous`.
    pub per_state_ambiguous: CountDistribution,
}

/// Descriptive per-arm summary row (`§7.1` last bullet).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PerArmSummaryRow {
    /// Arm (`off`, `control`, `on`).
    pub arm: EvalArm,
    /// Total joined decisions.
    pub n_decisions: usize,
    /// Decisions on states passing the arm-blind pre-screen (`excluded_by_prescreen == false`).
    pub n_non_prescreened: usize,
    /// Decisions having a defined non-`null` `score_norm`.
    pub n_scored_norm: usize,
    /// Decisions carrying a `first_draft_candidate_id`.
    pub n_with_first_draft: usize,
    /// Mean `ev_total_equity` across all `n_decisions` (`None` when `0`).
    pub mean_ev_total_equity: Option<f64>,
    /// Mean `score_norm` across `n_scored_norm` (`None` when `0`).
    pub mean_score_norm: Option<f64>,
    /// Mean `regret_equity` across all `n_decisions` (`None` when `0`).
    pub mean_regret_equity: Option<f64>,
    /// Mean arm-independent percentile sensitivity across scored entries (`None` when `0`).
    pub mean_sensitivity_indep_percentile: Option<f64>,
    /// Descriptive talk message count totals and per-state distributions for this arm (`§7.1`).
    #[serde(default)]
    pub talk_counts: PerArmTalkCountsSummary,
    /// Descriptive §7.2 per-arm mechanism summary (`inferential_status: "descriptive_only"`).
    #[serde(default)]
    pub mechanisms: PerArmMechanismSummary,
}

/// Joined descriptive summary output written to `<out>/per_arm_summary.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PerArmSummaryFile {
    /// Schema version (`1`).
    pub schema_version: u32,
    /// Explicit note that post-unblinding tables are descriptive only (`§7.1`).
    pub inferential_status: String,
    /// Freeze SHA-256 of the verified `scorer_freeze.json`.
    pub freeze_sha256: String,
    /// Sorted per-arm descriptive summaries.
    pub arms: Vec<PerArmSummaryRow>,
    /// Fixed §7.2.3 statement of what the recorded Evaluation data cannot tell.
    #[serde(default)]
    pub data_limits: Vec<String>,
}

/// Complete joined per-decision report written to `<out>/joined_decisions.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JoinedDecisionsFile {
    /// Schema version (`1`).
    pub schema_version: u32,
    /// Explicit note that post-unblinding tables are descriptive only (`§7.1`).
    pub inferential_status: String,
    /// SHA-256 of `scorer_freeze.json`.
    pub freeze_sha256: String,
    /// Joined rows in canonical `(state_id, arm, replicate)` order.
    pub rows: Vec<JoinedDecisionRow>,
}

/// Summary returned by [`run_join`].
#[derive(Debug, Clone, PartialEq)]
pub struct JoinReport {
    /// Number of states joined.
    pub states_joined: usize,
    /// Number of `(state_id, arm, replicate)` decision rows joined.
    pub decisions_joined: usize,
    /// Per-arm descriptive summary rows.
    pub arm_summaries: Vec<PerArmSummaryRow>,
}

fn default_scorer_crate_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../league-scorer")
}

/// Validates clean workspace & blind inputs and writes write-once `scorer_freeze.json` (`§8.2` step 3).
pub fn run_freeze(
    opts: &FreezeOptions,
    git_sha: &CrateGitShaSource<'_>,
) -> Result<ScorerFreezeFile, EvalError> {
    if opts.control_wording_arm.trim().is_empty() {
        return Err(EvalError::Usage(
            "`--control-wording-arm <label>` cannot be empty".to_owned(),
        ));
    }

    let scorer_git_sha = resolve_crate_git_sha_from(git_sha)?;

    let cfg_bytes = fs::read(&opts.config_path)
        .map_err(|e| EvalError::Io(format!("read {}: {e}", opts.config_path.display())))?;
    let config: ScorerConfig = serde_json::from_slice(&cfg_bytes)
        .map_err(|e| EvalError::Io(format!("parse ScorerConfig JSON: {e}")))?;
    config.validate()?;
    let expected_talk_digest = talk_rules_v2_digest()?;
    if config.talk_extractor_version != TALK_EXTRACTOR_VERSION_V2 {
        return Err(EvalError::SchemaValidation(format!(
            "unsupported talk_extractor_version `{}`",
            config.talk_extractor_version
        )));
    }
    if config.talk_extractor_rules_digest != expected_talk_digest {
        return Err(EvalError::SchemaValidation(format!(
            "config talk_extractor_rules_digest `{}` != `{expected_talk_digest}`",
            config.talk_extractor_rules_digest
        )));
    }
    let scorer_config_sha256 = config.sha256_hex()?;

    let scorer_crate_dir = opts
        .scorer_crate_dir
        .clone()
        .unwrap_or_else(default_scorer_crate_dir);
    let scorer_tree_sha256 = compute_scorer_tree_sha256(&scorer_crate_dir)?;

    validate_blind_inputs_dir_against_config(&opts.blind_dir, &config)?;
    let blind_inputs_manifest_sha256 = compute_blind_inputs_manifest_sha256(&opts.blind_dir)?;

    let freeze = ScorerFreezeFile {
        blind_inputs_manifest_sha256,
        control_wording_arm: opts.control_wording_arm.clone(),
        scorer_config_sha256,
        scorer_git_sha,
        scorer_tree_sha256,
        talk_extractor_rules_digest: config.talk_extractor_rules_digest,
        talk_extractor_version: config.talk_extractor_version,
    };
    freeze.validate()?;

    if let Some(parent) = opts.out_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|e| EvalError::Io(format!("create freeze parent dir: {e}")))?;
    }
    let json_text = to_sorted_pretty_json(&freeze)?;
    write_freeze_file_with_writer(&opts.out_path, |file| file.write_all(json_text.as_bytes()))?;
    Ok(freeze)
}

/// Atomically creates a new freeze file (`create_new(true)`) and removes the partial file if
/// writing fails, so subsequent retries are not blocked by `FREEZE_FILE_EXISTS`.
#[doc(hidden)]
pub fn write_freeze_file_with_writer(
    out_path: &Path,
    write_fn: impl FnOnce(&mut fs::File) -> std::io::Result<()>,
) -> Result<(), EvalError> {
    write_freeze_file_with_writer_and_remover(out_path, write_fn, |path| fs::remove_file(path))
}

/// Variant of [`write_freeze_file_with_writer`] accepting a cleanup callback so partial-file
/// removal error paths can be verified cleanly across platforms.
#[doc(hidden)]
pub fn write_freeze_file_with_writer_and_remover(
    out_path: &Path,
    write_fn: impl FnOnce(&mut fs::File) -> std::io::Result<()>,
    remove_fn: impl FnOnce(&Path) -> std::io::Result<()>,
) -> Result<(), EvalError> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(out_path)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                EvalError::FreezeFileExists(format!(
                    "freeze file already exists at {}; refusing to overwrite",
                    out_path.display()
                ))
            } else {
                EvalError::Io(format!("create {}: {e}", out_path.display()))
            }
        })?;
    if let Err(e) = write_fn(&mut file) {
        drop(file);
        if let Err(rm_err) = remove_fn(out_path) {
            return Err(EvalError::Io(format!(
                "write {}: {e} (also failed to remove partial file: {rm_err})",
                out_path.display()
            )));
        }
        return Err(EvalError::Io(format!("write {}: {e}", out_path.display())));
    }
    Ok(())
}

fn validate_blind_inputs_dir_against_config(
    blind_dir: &Path,
    config: &ScorerConfig,
) -> Result<(), EvalError> {
    let read_dir = fs::read_dir(blind_dir).map_err(|e| {
        EvalError::FreezeManifestMismatch(format!(
            "cannot read --blind {}: {e}",
            blind_dir.display()
        ))
    })?;
    let mut paths = Vec::new();
    for entry_res in read_dir {
        let entry = entry_res.map_err(|e| EvalError::Io(format!("read blind entry: {e}")))?;
        let ft = entry
            .file_type()
            .map_err(|e| EvalError::Io(format!("inspect blind entry: {e}")))?;
        if ft.is_file() && entry.path().extension().and_then(|s| s.to_str()) == Some("json") {
            paths.push(entry.path());
        }
    }
    paths.sort();
    if paths.is_empty() {
        return Err(EvalError::FreezeManifestMismatch(format!(
            "no *.json files in --blind {}",
            blind_dir.display()
        )));
    }
    for path in paths {
        let bytes =
            fs::read(&path).map_err(|e| EvalError::Io(format!("read {}: {e}", path.display())))?;
        let req = parse_blind_scoring_request_strict(&bytes)?;
        req.validate(config)?;
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| EvalError::Io("non-UTF-8 blind request filename".to_owned()))?;
        if stem != req.state_id {
            return Err(EvalError::FreezeManifestMismatch(format!(
                "blind request stem `{stem}` != state_id `{}`",
                req.state_id
            )));
        }
    }
    Ok(())
}

fn collect_json_files(dir: &Path, label: &str) -> Result<Vec<PathBuf>, EvalError> {
    let entries = fs::read_dir(dir)
        .map_err(|e| EvalError::Io(format!("read {label} {}: {e}", dir.display())))?;
    let mut paths = Vec::new();
    for entry_res in entries {
        let entry = entry_res.map_err(|e| EvalError::Io(format!("read {label} entry: {e}")))?;
        let ft = entry
            .file_type()
            .map_err(|e| EvalError::Io(format!("inspect {label} entry: {e}")))?;
        if ft.is_file() && entry.path().extension().and_then(|s| s.to_str()) == Some("json") {
            paths.push(entry.path());
        }
    }
    paths.sort();
    Ok(paths)
}

fn resolve_scores_json_dir(scores_dir: &Path) -> PathBuf {
    let nested = scores_dir.join("scores");
    if nested.is_dir() {
        nested
    } else {
        scores_dir.to_path_buf()
    }
}

fn verify_control_entry_wording_arm(
    entry: &ArmMapEntry,
    expected_control_wording_arm: &str,
) -> Result<(), EvalError> {
    let mut verified_any = false;
    if let Some(ref setup_path) = entry.setup_path {
        let raw = fs::read_to_string(setup_path).map_err(|e| {
            EvalError::ControlWordingArmMismatch(format!(
                "failed to read control setup.json {}: {e}",
                setup_path.display()
            ))
        })?;
        let setup: EvalSetupFile = serde_json::from_str(&raw).map_err(|e| {
            EvalError::ControlWordingArmMismatch(format!(
                "failed to parse control setup.json {}: {e}",
                setup_path.display()
            ))
        })?;
        if setup.arm != EvalArm::Control {
            return Err(EvalError::ControlWordingArmMismatch(format!(
                "setup.json {} has arm `{}`, expected `control`",
                setup_path.display(),
                setup.arm.as_str()
            )));
        }
        if setup.relay_wording_arm != expected_control_wording_arm {
            return Err(EvalError::ControlWordingArmMismatch(format!(
                "control setup.json {} has relay_wording_arm `{}`, expected frozen `{expected_control_wording_arm}`",
                setup_path.display(),
                setup.relay_wording_arm
            )));
        }
        verified_any = true;
    }

    if let Some(ref manifest_path) = entry.manifest_path {
        let raw = fs::read_to_string(manifest_path).map_err(|e| {
            EvalError::ControlWordingArmMismatch(format!(
                "failed to read control manifest.json {}: {e}",
                manifest_path.display()
            ))
        })?;
        let manifest: EvalManifest = serde_json::from_str(&raw).map_err(|e| {
            EvalError::ControlWordingArmMismatch(format!(
                "failed to parse control manifest.json {}: {e}",
                manifest_path.display()
            ))
        })?;
        if manifest.relay_wording_arm != expected_control_wording_arm {
            return Err(EvalError::ControlWordingArmMismatch(format!(
                "control manifest.json {} has relay_wording_arm `{}`, expected frozen `{expected_control_wording_arm}`",
                manifest_path.display(),
                manifest.relay_wording_arm
            )));
        }
        verified_any = true;
    }

    if let Some(arm_dir) = entry.decision_path.parent() {
        let relay_setup = arm_dir.join("relay").join("setup.json");
        if relay_setup.is_file() {
            let raw = fs::read_to_string(&relay_setup).map_err(|e| {
                EvalError::ControlWordingArmMismatch(format!(
                    "failed to read {}: {e}",
                    relay_setup.display()
                ))
            })?;
            let val: serde_json::Value = serde_json::from_str(&raw).map_err(|e| {
                EvalError::ControlWordingArmMismatch(format!(
                    "failed to parse {}: {e}",
                    relay_setup.display()
                ))
            })?;
            let Some(wording_arm) = val.get("wording_arm").and_then(|v| v.as_str()) else {
                return Err(EvalError::ControlWordingArmMismatch(format!(
                    "control relay/setup.json {} lacks string wording_arm key",
                    relay_setup.display()
                )));
            };
            if wording_arm != expected_control_wording_arm {
                return Err(EvalError::ControlWordingArmMismatch(format!(
                    "control relay/setup.json {} has wording_arm `{wording_arm}`, expected frozen `{expected_control_wording_arm}`",
                    relay_setup.display()
                )));
            }
            verified_any = true;
        }
    }

    if !verified_any {
        return Err(EvalError::ControlWordingArmMismatch(format!(
            "control entry `{}` lacks setup.json or manifest.json recording relay_wording_arm",
            entry.decision_path.display()
        )));
    }
    Ok(())
}

fn find_candidate_score<'a>(
    score: &'a StateScoreOutput,
    candidate_id: &str,
) -> Result<&'a CandidateScoreOutput, EvalError> {
    score
        .candidates
        .iter()
        .find(|c| c.candidate_id == candidate_id)
        .ok_or_else(|| {
            EvalError::FreezeManifestMismatch(format!(
                "candidate_id `{candidate_id}` missing from scored candidates for `{}`",
                score.state_id
            ))
        })
}

fn verify_score_matches_arm_map(
    score: &StateScoreOutput,
    arm_map: &ArmMapStateFile,
    freeze: &ScorerFreezeFile,
) -> Result<(), EvalError> {
    if score.scorer_config_sha256 != freeze.scorer_config_sha256 {
        return Err(EvalError::FreezeConfigMismatch(format!(
            "score `{}` scorer_config_sha256 `{}` != freeze `{}`",
            score.state_id, score.scorer_config_sha256, freeze.scorer_config_sha256
        )));
    }
    if score.blind_request_sha256 != arm_map.blind_input_sha256 {
        return Err(EvalError::FreezeManifestMismatch(format!(
            "score `{}` blind_request_sha256 `{}` != arm_map blind_input_sha256 `{}`",
            score.state_id, score.blind_request_sha256, arm_map.blind_input_sha256
        )));
    }
    if score.day != arm_map.day
        || score.focal_seat != arm_map.focal_seat
        || score.scorer_seed != arm_map.scorer_seed
        || score.pre_act_save_sha256 != arm_map.pre_act_save_sha256
        || score.pre_act_seal_sha256 != arm_map.pre_act_seal_sha256
        || score.pre_act_pair_sha256 != arm_map.pre_act_pair_sha256
    {
        return Err(EvalError::FreezeManifestMismatch(format!(
            "score metadata for `{}` disagrees with arm_map blind-input provenance",
            score.state_id
        )));
    }
    let scored_cids: Vec<String> = score
        .candidates
        .iter()
        .map(|c| c.candidate_id.clone())
        .collect();
    if scored_cids != arm_map.blind_candidate_ids {
        return Err(EvalError::FreezeManifestMismatch(format!(
            "scored candidate set for `{}` disagrees with frozen blind input candidates",
            score.state_id
        )));
    }
    Ok(())
}

/// Enforces all `§8.2` pre-unblinding gates, joins blind scores with `arm_map/` by
/// `(state_id, candidate_id)`, and writes descriptive-only per-arm tables to `--out`.
pub fn run_join(opts: &JoinOptions) -> Result<JoinReport, EvalError> {
    if !opts.freeze_path.is_file() {
        return Err(EvalError::MissingFreezeFile(format!(
            "missing scorer_freeze.json at {}; cannot join arm_map/ without a freeze file (§8.2 step 2)",
            opts.freeze_path.display()
        )));
    }
    let freeze_bytes = fs::read(&opts.freeze_path)
        .map_err(|e| EvalError::Io(format!("read {}: {e}", opts.freeze_path.display())))?;
    let freeze_sha256 = league_scorer::sha256_hex(&freeze_bytes);
    let freeze: ScorerFreezeFile = serde_json::from_slice(&freeze_bytes)
        .map_err(|e| EvalError::Io(format!("parse {}: {e}", opts.freeze_path.display())))?;
    freeze.validate()?;

    let arm_map_paths = collect_json_files(&opts.arm_map_dir, "--arm-map")?;
    if arm_map_paths.is_empty() {
        return Err(EvalError::FreezeManifestMismatch(format!(
            "no *.json files found in --arm-map {}",
            opts.arm_map_dir.display()
        )));
    }

    let mut arm_maps: BTreeMap<String, ArmMapStateFile> = BTreeMap::new();
    let mut manifest_entries: BTreeMap<String, String> = BTreeMap::new();
    for path in arm_map_paths {
        let raw = fs::read_to_string(&path)
            .map_err(|e| EvalError::Io(format!("read {}: {e}", path.display())))?;
        let state: ArmMapStateFile = serde_json::from_str(&raw)
            .map_err(|e| EvalError::Io(format!("parse {}: {e}", path.display())))?;
        manifest_entries.insert(
            format!("{}.json", state.state_id),
            state.blind_input_sha256.clone(),
        );
        let state_id = state.state_id.clone();
        if arm_maps.insert(state_id.clone(), state).is_some() {
            return Err(EvalError::DuplicateStateId(format!(
                "duplicate state_id `{state_id}` across --arm-map files"
            )));
        }
    }

    let reconstructed_manifest_sha =
        compute_blind_inputs_manifest_sha256_from_entries(&manifest_entries);
    if reconstructed_manifest_sha != freeze.blind_inputs_manifest_sha256 {
        return Err(EvalError::FreezeManifestMismatch(format!(
            "blind_inputs_manifest_sha256 `{reconstructed_manifest_sha}` != freeze `{}`",
            freeze.blind_inputs_manifest_sha256
        )));
    }

    if let Some(ref blind_dir) = opts.blind_dir {
        let actual_blind_manifest = compute_blind_inputs_manifest_sha256(blind_dir)?;
        if actual_blind_manifest != freeze.blind_inputs_manifest_sha256 {
            return Err(EvalError::FreezeManifestMismatch(format!(
                "on-disk --blind manifest `{actual_blind_manifest}` != freeze `{}`",
                freeze.blind_inputs_manifest_sha256
            )));
        }
    }

    let effective_scores_dir = resolve_scores_json_dir(&opts.scores_dir);
    let score_paths = collect_json_files(&effective_scores_dir, "--scores")?;
    if score_paths.is_empty() {
        return Err(EvalError::FreezeManifestMismatch(format!(
            "no *.json score files found in {}",
            effective_scores_dir.display()
        )));
    }

    let mut scores_by_state: BTreeMap<String, StateScoreOutput> = BTreeMap::new();
    let mut scored_manifest_entries: BTreeMap<String, String> = BTreeMap::new();
    for path in score_paths {
        let raw = fs::read_to_string(&path)
            .map_err(|e| EvalError::Io(format!("read {}: {e}", path.display())))?;
        let score: StateScoreOutput = serde_json::from_str(&raw)
            .map_err(|e| EvalError::Io(format!("parse {}: {e}", path.display())))?;
        if !is_valid_sha256_hex(&score.blind_request_sha256) {
            return Err(EvalError::FreezeManifestMismatch(format!(
                "score `{}` has invalid blind_request_sha256 `{}`",
                score.state_id, score.blind_request_sha256
            )));
        }
        scored_manifest_entries.insert(
            format!("{}.json", score.state_id),
            score.blind_request_sha256.clone(),
        );
        let state_id = score.state_id.clone();
        if scores_by_state.insert(state_id.clone(), score).is_some() {
            return Err(EvalError::DuplicateStateId(format!(
                "duplicate state_id `{state_id}` across --scores files"
            )));
        }
    }

    let scored_manifest_sha =
        compute_blind_inputs_manifest_sha256_from_entries(&scored_manifest_entries);
    if scored_manifest_sha != freeze.blind_inputs_manifest_sha256 {
        return Err(EvalError::FreezeManifestMismatch(format!(
            "scored blind_request_sha256 manifest `{scored_manifest_sha}` != freeze `{}`",
            freeze.blind_inputs_manifest_sha256
        )));
    }

    let arm_keys: BTreeSet<&String> = arm_maps.keys().collect();
    let score_keys: BTreeSet<&String> = scores_by_state.keys().collect();
    if arm_keys != score_keys {
        return Err(EvalError::FreezeManifestMismatch(
            "state_id set in --scores disagrees with --arm-map".to_owned(),
        ));
    }

    // Verify every freeze/manifest/config/wording gate BEFORE inspecting unblinded decision or
    // sidecar files for §7.2 mechanism reporting.
    for (state_id, arm_map) in &arm_maps {
        let score = &scores_by_state[state_id];
        verify_score_matches_arm_map(score, arm_map, &freeze)?;
        for entry in &arm_map.entries {
            if entry.arm == EvalArm::Control {
                verify_control_entry_wording_arm(entry, &freeze.control_wording_arm)?;
            }
            let _ = find_candidate_score(score, &entry.committed_candidate_id)?;
            if let Some(cid) = entry.first_draft_candidate_id.as_deref() {
                let _ = find_candidate_score(score, cid)?;
            }
        }
    }

    let source_games = index_source_game_dirs(&opts.source_game_dirs)?;

    let mut rows = Vec::new();
    for (state_id, arm_map) in &arm_maps {
        let score = &scores_by_state[state_id];
        let source_game_dir = source_games
            .get(&arm_map.source_game_id)
            .map(PathBuf::as_path);

        for entry in &arm_map.entries {
            let committed_score = find_candidate_score(score, &entry.committed_candidate_id)?;
            let draft_score = entry
                .first_draft_candidate_id
                .as_deref()
                .map(|cid| find_candidate_score(score, cid))
                .transpose()?;
            let first_draft_ev_total_equity = draft_score.map(|s| s.ev_total_equity);
            let first_draft_score_norm = draft_score.and_then(|s| s.score_norm);
            let mechanisms = compute_decision_mechanisms(
                arm_map,
                &entry.decision_path,
                &entry.outcome_path,
                committed_score.ev_total_equity,
                committed_score.score_norm,
                first_draft_ev_total_equity,
                first_draft_score_norm,
                source_game_dir,
            )?;

            rows.push(JoinedDecisionRow {
                state_id: state_id.clone(),
                day: arm_map.day,
                focal_seat: arm_map.focal_seat,
                arm: entry.arm,
                replicate: entry.replicate,
                committed_candidate_id: entry.committed_candidate_id.clone(),
                first_draft_candidate_id: entry.first_draft_candidate_id.clone(),
                committed_ev_total_equity: committed_score.ev_total_equity,
                committed_score_norm: committed_score.score_norm,
                committed_regret_equity: committed_score.regret_equity,
                committed_sensitivity_indep_percentile: committed_score
                    .sensitivity_indep_percentile,
                committed_sensitivity_indep_rank: committed_score.sensitivity_indep_rank,
                first_draft_ev_total_equity,
                first_draft_score_norm,
                excluded_by_prescreen: score.prescreen.excluded_by_prescreen,
                degenerate_flat_set: score.degenerate_flat_set,
                talk_extraction_counts: score.talk_extraction_counts.clone(),
                mechanisms,
                decision_path: entry.decision_path.clone(),
                outcome_path: entry.outcome_path.clone(),
            });
        }
    }

    let mut by_arm: BTreeMap<EvalArm, Vec<&JoinedDecisionRow>> = BTreeMap::new();
    for row in &rows {
        by_arm.entry(row.arm).or_default().push(row);
    }

    let mut arm_summaries = Vec::with_capacity(by_arm.len());
    for (arm, arm_rows) in by_arm {
        let n_decisions = arm_rows.len();
        let n_non_prescreened = arm_rows.iter().filter(|r| !r.excluded_by_prescreen).count();
        let norm_vals: Vec<f64> = arm_rows
            .iter()
            .filter_map(|r| r.committed_score_norm)
            .collect();
        let pct_vals: Vec<f64> = arm_rows
            .iter()
            .filter_map(|r| r.committed_sensitivity_indep_percentile)
            .collect();
        let n_with_first_draft = arm_rows
            .iter()
            .filter(|r| r.first_draft_candidate_id.is_some())
            .count();
        let sum_ev: f64 = arm_rows.iter().map(|r| r.committed_ev_total_equity).sum();
        let sum_regret: f64 = arm_rows.iter().map(|r| r.committed_regret_equity).sum();
        let sum_norm: f64 = norm_vals.iter().sum();
        let sum_pct: f64 = pct_vals.iter().sum();
        let talk_counts = summarize_arm_talk_counts(&arm_rows);
        let mechanisms = summarize_arm_mechanisms(&arm_rows);

        arm_summaries.push(PerArmSummaryRow {
            arm,
            n_decisions,
            n_non_prescreened,
            n_scored_norm: norm_vals.len(),
            n_with_first_draft,
            mean_ev_total_equity: (n_decisions > 0).then(|| sum_ev / (n_decisions as f64)),
            mean_score_norm: (!norm_vals.is_empty()).then(|| sum_norm / (norm_vals.len() as f64)),
            mean_regret_equity: (n_decisions > 0).then(|| sum_regret / (n_decisions as f64)),
            mean_sensitivity_indep_percentile: (!pct_vals.is_empty())
                .then(|| sum_pct / (pct_vals.len() as f64)),
            talk_counts,
            mechanisms,
        });
    }

    fs::create_dir_all(&opts.out_dir)
        .map_err(|e| EvalError::Io(format!("create join --out {}: {e}", opts.out_dir.display())))?;

    let summary_file = PerArmSummaryFile {
        schema_version: BLIND_SCORING_SCHEMA_VERSION,
        inferential_status: DESCRIPTIVE_ONLY_INFERENTIAL_STATUS.to_owned(),
        freeze_sha256: freeze_sha256.clone(),
        arms: arm_summaries.clone(),
        data_limits: MECHANISM_DATA_LIMITS
            .iter()
            .map(|s| (*s).to_owned())
            .collect(),
    };
    let decisions_file = JoinedDecisionsFile {
        schema_version: BLIND_SCORING_SCHEMA_VERSION,
        inferential_status: DESCRIPTIVE_ONLY_INFERENTIAL_STATUS.to_owned(),
        freeze_sha256,
        rows,
    };

    let summary_json = to_sorted_pretty_json(&summary_file)?;
    fs::write(
        opts.out_dir.join("per_arm_summary.json"),
        summary_json.as_bytes(),
    )
    .map_err(|e| EvalError::Io(format!("write per_arm_summary.json: {e}")))?;

    let decisions_json = to_sorted_pretty_json(&decisions_file)?;
    fs::write(
        opts.out_dir.join("joined_decisions.json"),
        decisions_json.as_bytes(),
    )
    .map_err(|e| EvalError::Io(format!("write joined_decisions.json: {e}")))?;

    Ok(JoinReport {
        states_joined: arm_maps.len(),
        decisions_joined: decisions_file.rows.len(),
        arm_summaries,
    })
}

fn add_source_counts(acc: &mut TalkSourceCounts, rhs: &TalkSourceCounts) {
    acc.ambiguous = acc.ambiguous.saturating_add(rhs.ambiguous);
    acc.matched = acc.matched.saturating_add(rhs.matched);
    acc.seen = acc.seen.saturating_add(rhs.seen);
}

fn compute_count_distribution(values: &[u64]) -> CountDistribution {
    let n_states = values.len();
    if n_states == 0 {
        return CountDistribution::default();
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let total = sorted.iter().copied().fold(0_u64, u64::saturating_add);
    let min = sorted[0];
    let max = sorted[n_states - 1];
    let mean = Some((total as f64) / (n_states as f64));
    let median = if n_states % 2 == 1 {
        Some(sorted[n_states / 2] as f64)
    } else {
        let lo = sorted[n_states / 2 - 1] as f64;
        let hi = sorted[n_states / 2] as f64;
        Some((lo + hi) / 2.0)
    };
    CountDistribution {
        max,
        mean,
        median,
        min,
        n_states,
        total,
    }
}

pub(crate) fn summarize_arm_talk_counts_from_slices(
    pairs: &[(&str, &TalkExtractionCounts)],
) -> PerArmTalkCountsSummary {
    let mut per_state: BTreeMap<&str, &TalkExtractionCounts> = BTreeMap::new();
    for &(state_id, counts) in pairs {
        per_state.entry(state_id).or_insert(counts);
    }
    let mut totals = TalkExtractionCounts::default();
    let mut seen_vals = Vec::with_capacity(per_state.len());
    let mut matched_vals = Vec::with_capacity(per_state.len());
    let mut ambiguous_vals = Vec::with_capacity(per_state.len());
    for counts in per_state.values() {
        add_source_counts(&mut totals.negotiation, &counts.negotiation);
        add_source_counts(&mut totals.talk, &counts.talk);
        add_source_counts(&mut totals.total, &counts.total);
        seen_vals.push(counts.total.seen);
        matched_vals.push(counts.total.matched);
        ambiguous_vals.push(counts.total.ambiguous);
    }
    PerArmTalkCountsSummary {
        n_states: per_state.len(),
        totals,
        per_state_seen: compute_count_distribution(&seen_vals),
        per_state_matched: compute_count_distribution(&matched_vals),
        per_state_ambiguous: compute_count_distribution(&ambiguous_vals),
    }
}

fn summarize_arm_talk_counts(arm_rows: &[&JoinedDecisionRow]) -> PerArmTalkCountsSummary {
    let pairs: Vec<(&str, &TalkExtractionCounts)> = arm_rows
        .iter()
        .map(|row| (row.state_id.as_str(), &row.talk_extraction_counts))
        .collect();
    summarize_arm_talk_counts_from_slices(&pairs)
}
