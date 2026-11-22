//! Blind input, frozen configuration, and deterministic output schemas for `league-scorer`.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use league_core::{EvalActSubmission, MarketAction};

use crate::error::ScorerError;

/// Fixed schema version accepted by S1 blind scoring requests.
pub const BLIND_SCORING_SCHEMA_VERSION: u32 = 1;

/// Fixed Evaluation Mode table size (`D6`).
pub const EVAL_TABLE_SEAT_COUNT: u8 = 4;

/// Proposed fixed code constant: maximum synthetic converter batches per turn (`§5.2`).
pub const SYNTH_MAX_CONVERTER_BATCHES: usize = 4;

/// Proposed fixed code constant: maximum synthetic project batches per turn (`§5.2`).
pub const SYNTH_MAX_PROJECT_BATCHES: usize = 4;

/// Proposed fixed code constant: rank coverable projects by descending equity then project ID (`§5.2`).
pub const SYNTH_PROJECT_RANK_BY_EQUITY: bool = true;

/// Proposed fixed code constant: include converter input deficit coverage (`§5.2`).
pub const SYNTH_CONVERTER_INPUT_DEFICIT_COVER: bool = true;

/// Proposed fixed code constant: exact maturing CheckCash / EscrowFinish claims (`§5.2`).
pub const SYNTH_CLAIM_MATURING_EXACT: bool = true;

/// Proposed fixed code constant: self-nomination on election/charter turns (`§5.2`).
pub const SYNTH_COUNCIL_SELF_NOMINATION: bool = true;

/// Proposed fixed code constant: break upkeep-risk leg-drop ties by ascending `candidate_id` (`§5.2`).
pub const DROP_TIE_BREAK_BY_CANDIDATE_ID: bool = true;

/// Encodes a 32-byte digest into 64 lowercase hexadecimal ASCII characters.
fn encode_digest_hex(digest: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for &byte in digest {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

/// Computes lowercase 64-character hex SHA-256 of arbitrary bytes.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    encode_digest_hex(&digest)
}

/// Returns `true` iff `s` is exactly 64 lowercase hexadecimal ASCII characters.
#[must_use]
pub fn is_valid_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Computes `pre_act_pair_sha256` matching `league-core::eval::host::compute_pre_act_pair_sha256_hex`.
#[must_use]
pub fn compute_pre_act_pair_sha256_hex(save_bytes: &[u8], seal_bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"league-core/eval-pre-act-pair/v1|");
    hasher.update((save_bytes.len() as u64).to_le_bytes());
    hasher.update(save_bytes);
    hasher.update((seal_bytes.len() as u64).to_le_bytes());
    hasher.update(seal_bytes);
    let digest: [u8; 32] = hasher.finalize().into();
    encode_digest_hex(&digest)
}

/// Computes canonical `candidate_id` as `sha256_hex(serde_json::to_vec(&submission))` (`§8.1`).
pub fn candidate_id_for_submission(sub: &EvalActSubmission) -> Result<String, ScorerError> {
    let bytes = serde_json::to_vec(sub)
        .map_err(|e| ScorerError::Json(format!("serialize submission: {e}")))?;
    Ok(sha256_hex(&bytes))
}

/// Validates that `state_id` is safe as both a canonical identifier and cross-platform file stem.
pub fn validate_state_id(state_id: &str) -> Result<(), ScorerError> {
    if state_id.is_empty() || state_id == "." || state_id == ".." || state_id.len() > 128 {
        return Err(ScorerError::InvalidStateId(format!(
            "state_id `{state_id}` must be 1..=128 safe ASCII characters"
        )));
    }
    if !state_id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err(ScorerError::InvalidStateId(format!(
            "state_id `{state_id}` contains disallowed characters (allowed: [A-Za-z0-9._-])"
        )));
    }
    Ok(())
}

/// Recursively converts any `serde_json::Value` so every object map has lexicographically sorted keys.
fn sort_json_value(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(sort_json_value).collect())
        }
        serde_json::Value::Object(map) => {
            let mut entries: Vec<(String, serde_json::Value)> = map
                .into_iter()
                .map(|(k, v)| (k, sort_json_value(v)))
                .collect();
            entries.sort_by(|(a, _), (b, _)| a.cmp(b));
            let mut sorted = serde_json::Map::new();
            for (k, v) in entries {
                sorted.insert(k, v);
            }
            serde_json::Value::Object(sorted)
        }
        other => other,
    }
}

/// Serializes any `Serialize` value to canonical compact JSON with sorted object keys.
pub fn to_sorted_compact_json<T: Serialize>(value: &T) -> Result<Vec<u8>, ScorerError> {
    let v = serde_json::to_value(value)
        .map_err(|e| ScorerError::Json(format!("serialize value to JSON AST: {e}")))?;
    let sorted = sort_json_value(v);
    serde_json::to_vec(&sorted)
        .map_err(|e| ScorerError::Json(format!("serialize sorted compact JSON: {e}")))
}

/// Serializes any `Serialize` value to 2-space pretty JSON with sorted object keys and trailing newline.
pub fn to_sorted_pretty_json<T: Serialize>(value: &T) -> Result<String, ScorerError> {
    let v = serde_json::to_value(value)
        .map_err(|e| ScorerError::Json(format!("serialize value to JSON AST: {e}")))?;
    let sorted = sort_json_value(v);
    let mut s = serde_json::to_string_pretty(&sorted)
        .map_err(|e| ScorerError::Json(format!("serialize sorted pretty JSON: {e}")))?;
    s.push('\n');
    Ok(s)
}

/// Counterfactual peer policy posture (`§4.2`, `§5.2`). S1 implements [`PostureKind::Standard`] only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PostureKind {
    /// Unmodified `ClubBot::decide_full_turn` (`§4.2`).
    Standard,
    /// Liquidity/converter-active posture (`§4.2`, deferred to S2).
    Active,
    /// Conservative upkeep-first posture (`§4.2`, deferred to S2).
    Conservative,
}

/// Peer policy posture mix weights (`§5.2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PostureWeights {
    /// Active posture weight.
    pub active: u32,
    /// Conservative posture weight.
    pub conservative: u32,
    /// Standard `ClubBot` posture weight.
    pub standard: u32,
}

impl Default for PostureWeights {
    fn default() -> Self {
        Self {
            active: 1,
            conservative: 1,
            standard: 2,
        }
    }
}

/// Complete frozen configuration for decision scoring (`§5.2`).
///
/// [`Default::default`] holds the exact pilot initial values from `docs/decision-scorer.md` §5.2.
/// Fields are ordered alphabetically so struct serialization and sorted-key canonical JSON agree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScorerConfig {
    /// Sole owner of composite greedy combination & leg-drop mode.
    pub composite_greedy_rules: String,
    /// Fractions of spare `CRED` above upkeep reserve offered as `debt_repayment_cred` (bps).
    pub debt_repayment_grid_bps: Vec<u16>,
    /// Whether to include a `pass_b_actions`-cleared `ClubBot` copy inside `k_synth_max`.
    pub include_pass_b_cleared_club_bot: bool,
    /// Cap on synthetic candidates plus the optional cleared-pass-B `ClubBot` copy.
    pub k_synth_max: usize,
    /// Permutation stratification scheme over undealt `spec.mandate_deck` entries.
    pub mandate_stratification_mode: String,
    /// Probability an extracted talk commitment is prepended to peer Turn-`day` actions (S2).
    pub p_honour: f64,
    /// Minimum spread vs shadow parity triggering liquidity posture `PostOffer` posts (bps).
    pub parity_spread_threshold_bps: u16,
    /// Crossing fractions applied to resting top-of-book `CredOffer`s (bps).
    pub partial_crossing_fractions_bps: Vec<u16>,
    /// Deterministic `m % posture_schedule.len()` posture mapping.
    pub posture_schedule: Vec<PostureKind>,
    /// Posture weights (`[Standard, Active, Conservative]`).
    pub posture_weights: PostureWeights,
    /// Exclude states with `v_max_indep == v_min_indep` in the `§3.2` arm-blind pre-screen.
    pub prescreen_exclude_flat_sets: bool,
    /// Minimum distinct feasible candidates required in `C_indep(state)` (`§3.2`).
    pub prescreen_min_arm_indep_candidates: usize,
    /// Minimum arm-independent `v_max_indep - v_min_indep` span in integer equity (`§3.2`).
    pub prescreen_min_arm_indep_span: i64,
    /// Perturbation scalings applied to feasible composite drafts (bps).
    pub quantity_scales_bps: Vec<u16>,
    /// Parity-relative limit-order price offsets for synthetic `PostOffer` generation (bps).
    pub quote_offsets_bps: Vec<i16>,
    /// Number of Common Random Numbers scenarios `M` per state.
    pub scenarios_m: usize,
    /// Checkpoint stratification schedule label.
    pub strata_schedule: String,
    /// Deterministic priority queue truncating excess feasible synthetics at `k_synth_max`.
    pub synth_slot_fill_order: Vec<String>,
    /// 64-hex SHA-256 of the talk commitment extractor rule table.
    pub talk_extractor_rules_digest: String,
    /// Talk commitment extractor version identifier.
    pub talk_extractor_version: String,
    /// Minimum upkeep turns reserved by synthetic rules and conservative posture.
    pub upkeep_cover_turns: u8,
}

impl Default for ScorerConfig {
    fn default() -> Self {
        Self {
            composite_greedy_rules: "top_pass_per_axis_plus_drop_max_upkeep".to_owned(),
            debt_repayment_grid_bps: vec![5000, 10_000],
            include_pass_b_cleared_club_bot: true,
            k_synth_max: 32,
            mandate_stratification_mode: "uniform_undealt_fy_stratified".to_owned(),
            p_honour: 0.75,
            parity_spread_threshold_bps: 500,
            partial_crossing_fractions_bps: vec![5000, 10_000],
            posture_schedule: vec![
                PostureKind::Standard,
                PostureKind::Standard,
                PostureKind::Active,
                PostureKind::Conservative,
            ],
            posture_weights: PostureWeights::default(),
            prescreen_exclude_flat_sets: true,
            prescreen_min_arm_indep_candidates: 2,
            prescreen_min_arm_indep_span: 1,
            quantity_scales_bps: vec![5000, 10_000],
            quote_offsets_bps: vec![-1000, -500, 500, 1000],
            scenarios_m: 16,
            strata_schedule: "2_early_2_mid_2_late_20_seeds".to_owned(),
            synth_slot_fill_order: vec![
                "cleared_pass_b".to_owned(),
                "claims_council".to_owned(),
                "upkeep_inputs".to_owned(),
                "cross_quotes".to_owned(),
                "batches".to_owned(),
                "debt".to_owned(),
                "composite_drops".to_owned(),
                "scaled".to_owned(),
            ],
            talk_extractor_rules_digest: sha256_hex(b"talk_rules_v1"),
            talk_extractor_version: "talk_rules_v1".to_owned(),
            upkeep_cover_turns: 2,
        }
    }
}

impl ScorerConfig {
    /// Builds an S1-valid all-Standard `ScorerConfig` using `scenarios_m` CRN scenarios.
    #[must_use]
    pub fn s1_standard(scenarios_m: usize) -> Self {
        Self {
            posture_schedule: vec![PostureKind::Standard],
            posture_weights: PostureWeights {
                active: 0,
                conservative: 0,
                standard: 1,
            },
            scenarios_m,
            ..Self::default()
        }
    }

    /// Computes canonical compact-JSON SHA-256 of this config (`scorer_config_sha256`).
    pub fn sha256_hex(&self) -> Result<String, ScorerError> {
        let bytes = to_sorted_compact_json(self)?;
        Ok(sha256_hex(&bytes))
    }

    /// Validates all S1 invariants and refuses unbuilt postures (`Active` / `Conservative`).
    pub fn validate_for_s1(&self) -> Result<(), ScorerError> {
        if self.scenarios_m < 2 || !self.scenarios_m.is_multiple_of(2) {
            return Err(ScorerError::InvalidScorerConfig(format!(
                "scenarios_m must be an even integer >= 2 for split-half Winner's curse, got {}",
                self.scenarios_m
            )));
        }
        if self.posture_schedule.is_empty() {
            return Err(ScorerError::InvalidScorerConfig(
                "posture_schedule cannot be empty".to_owned(),
            ));
        }
        for posture in &self.posture_schedule {
            if *posture != PostureKind::Standard {
                return Err(ScorerError::PostureNotImplemented(format!(
                    "posture not built yet: `{posture:?}` in posture_schedule"
                )));
            }
        }
        if self.posture_weights.active > 0 || self.posture_weights.conservative > 0 {
            return Err(ScorerError::PostureNotImplemented(format!(
                "posture not built yet: non-zero active ({}) or conservative ({}) weight",
                self.posture_weights.active, self.posture_weights.conservative
            )));
        }
        if self.posture_weights.standard == 0 {
            return Err(ScorerError::InvalidScorerConfig(
                "posture_weights.standard must be > 0".to_owned(),
            ));
        }
        if !matches!(
            self.mandate_stratification_mode.as_str(),
            "uniform_undealt_fy_stratified"
        ) {
            return Err(ScorerError::InvalidScorerConfig(format!(
                "unsupported mandate_stratification_mode `{}`",
                self.mandate_stratification_mode
            )));
        }
        if !self.p_honour.is_finite() || !(0.0..=1.0).contains(&self.p_honour) {
            return Err(ScorerError::InvalidScorerConfig(format!(
                "p_honour must be in [0.0, 1.0], got {}",
                self.p_honour
            )));
        }
        if self.prescreen_min_arm_indep_span < 0 {
            return Err(ScorerError::InvalidScorerConfig(format!(
                "prescreen_min_arm_indep_span must be >= 0, got {}",
                self.prescreen_min_arm_indep_span
            )));
        }
        if !is_valid_sha256_hex(&self.talk_extractor_rules_digest) {
            return Err(ScorerError::InvalidHexDigest {
                field: "talk_extractor_rules_digest",
                value: self.talk_extractor_rules_digest.clone(),
            });
        }
        if self.talk_extractor_version.trim().is_empty() {
            return Err(ScorerError::InvalidScorerConfig(
                "talk_extractor_version cannot be empty".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Neutral counterfactual talk-phase trade commitment (`§4.3`, `§8.1`). Deferred to S2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedTalkCommitment {
    /// Seat index of the counterparty receiving the offer (`0..4`).
    pub counterparty_seat: u8,
    /// Offered Pass-A `MarketAction`.
    pub offered_action: MarketAction,
    /// Seat index of the proposing peer (`0..4`).
    pub proposer_seat: u8,
}

/// One anonymized candidate submission inside a [`BlindScoringRequest`] (`§8.1`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlindCandidate {
    /// Canonical `sha256_hex(serde_json::to_vec(&submission))`.
    pub candidate_id: String,
    /// Whether this candidate enters `[v_min, v_max]` normalization (`true` for passive, synthetic,
    /// and committed decisions; `false` for tool-arm first drafts).
    pub in_span: bool,
    /// Whether this candidate originated strictly from arm-independent sources (`§3.2`, `§7.1`).
    pub is_arm_independent: bool,
    /// Exact submission applied via `EvalHost::apply_act_submissions`.
    pub submission: EvalActSubmission,
}

/// Arm-blind scoring request for one `(state_id, pre_act_checkpoint, focal_seat)` (`§8.1`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlindScoringRequest {
    /// Sorted, deduplicated candidate submissions.
    pub candidates: Vec<BlindCandidate>,
    /// Zero-based focal seat index (`0..4`).
    pub focal_seat: u8,
    /// Content-addressed domain-separated pair SHA-256 of `.save.json` + `.seal`.
    pub pre_act_pair_sha256: String,
    /// 64-hex SHA-256 of `day-DD-pre-act.save.json` bytes.
    pub pre_act_save_sha256: String,
    /// 64-hex SHA-256 of `day-DD-pre-act.seal` bytes.
    pub pre_act_seal_sha256: String,
    /// Request schema version (`1`).
    pub schema_version: u32,
    /// Canonical SHA-256 of the matching [`ScorerConfig`].
    pub scorer_config_sha256: String,
    /// Explicit seed governing deterministic undealt mandate and scenario bot seeds.
    pub scorer_seed: u64,
    /// Canonical arm-blind state identifier (cross-platform filename stem).
    pub state_id: String,
    /// Neutral counterfactual talk-phase trade commitments (must be empty in S1).
    pub talk_commitments: Vec<ExtractedTalkCommitment>,
}

impl BlindScoringRequest {
    /// Validates every S1 structural, hashing, ordering, and config-binding requirement.
    pub fn validate(&self, config: &ScorerConfig) -> Result<(), ScorerError> {
        if self.schema_version != BLIND_SCORING_SCHEMA_VERSION {
            return Err(ScorerError::UnsupportedSchemaVersion(self.schema_version));
        }
        validate_state_id(&self.state_id)?;
        for (field, val) in [
            ("pre_act_save_sha256", &self.pre_act_save_sha256),
            ("pre_act_seal_sha256", &self.pre_act_seal_sha256),
            ("pre_act_pair_sha256", &self.pre_act_pair_sha256),
            ("scorer_config_sha256", &self.scorer_config_sha256),
        ] {
            if !is_valid_sha256_hex(val) {
                return Err(ScorerError::InvalidHexDigest {
                    field,
                    value: val.clone(),
                });
            }
        }
        if self.focal_seat >= EVAL_TABLE_SEAT_COUNT {
            return Err(ScorerError::InvalidFocalSeat {
                focal_seat: self.focal_seat,
                seat_count: EVAL_TABLE_SEAT_COUNT,
            });
        }
        if !self.talk_commitments.is_empty() {
            return Err(ScorerError::TalkCommitmentsNotImplemented(
                self.talk_commitments.len(),
            ));
        }
        let cfg_hash = config.sha256_hex()?;
        if self.scorer_config_sha256 != cfg_hash {
            return Err(ScorerError::ConfigHashMismatch {
                expected: self.scorer_config_sha256.clone(),
                actual: cfg_hash,
            });
        }
        if self.candidates.is_empty() {
            return Err(ScorerError::EmptyCandidates(format!(
                "request `{}` contains zero candidates",
                self.state_id
            )));
        }

        let mut prev_id: Option<&str> = None;
        let mut any_in_span = false;
        for (idx, candidate) in self.candidates.iter().enumerate() {
            if !is_valid_sha256_hex(&candidate.candidate_id) {
                return Err(ScorerError::InvalidHexDigest {
                    field: "candidate_id",
                    value: candidate.candidate_id.clone(),
                });
            }
            if let Some(prev) = prev_id {
                match prev.cmp(candidate.candidate_id.as_str()) {
                    std::cmp::Ordering::Equal => {
                        return Err(ScorerError::DuplicateCandidateId(
                            candidate.candidate_id.clone(),
                        ));
                    }
                    std::cmp::Ordering::Greater => {
                        return Err(ScorerError::UnsortedCandidates {
                            prev: prev.to_owned(),
                            current: candidate.candidate_id.clone(),
                        });
                    }
                    std::cmp::Ordering::Less => {}
                }
            }
            prev_id = Some(candidate.candidate_id.as_str());

            let computed = candidate_id_for_submission(&candidate.submission)?;
            if computed != candidate.candidate_id {
                return Err(ScorerError::CandidateIdMismatch {
                    index: idx,
                    declared: candidate.candidate_id.clone(),
                    actual: computed,
                });
            }
            if candidate.in_span {
                any_in_span = true;
            }
        }
        if !any_in_span {
            return Err(ScorerError::NoInSpanCandidates(format!(
                "request `{}` has no in_span == true candidate",
                self.state_id
            )));
        }
        Ok(())
    }
}

/// Exact integer season-close outcome for the focal seat in one `(candidate, scenario)` rollout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenarioRolloutOutcome {
    /// Awarded Turn-1 Private Mandate bonus (`Units`).
    pub mandate_bonus: i64,
    /// Terminal marked Net Asset Equity (`Units`).
    pub net_asset_equity: i64,
    /// Irreversible Project Equity points (`Units`).
    pub project_equity: i64,
    /// Sum `project_equity + net_asset_equity + mandate_bonus` (`Units`).
    pub total_equity: i64,
}

/// Arm-blind pre-screen summary computed strictly over `is_arm_independent == true` candidates (`§3.2`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrescreenSummary {
    /// Whether the state fails any pre-screen threshold and is excluded from primary `[0, 1]` analysis.
    pub excluded_by_prescreen: bool,
    /// Whether `v_max_indep == v_min_indep` (or fewer than 1 arm-independent candidate exists).
    pub is_flat_indep: bool,
    /// Number of distinct `is_arm_independent == true` candidates evaluated.
    pub n_feasible_indep: usize,
    /// `v_max_indep - v_min_indep` across `C_indep(state)`.
    pub span_indep: f64,
    /// Maximum expected total equity across `C_indep(state)` (`None` when `C_indep` is empty).
    pub v_max_indep: Option<f64>,
    /// Minimum expected total equity across `C_indep(state)` (`None` when `C_indep` is empty).
    pub v_min_indep: Option<f64>,
}

/// Split-half selection-bias (Winner's curse) measurement across disjoint scenario halves (`§7.1`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WinnersCurseSummary {
    /// Average of `diff_a_to_b_equity` and `diff_b_to_a_equity`.
    pub bias_estimate_equity: f64,
    /// `c*_A`: `in_span` candidate maximizing mean total equity over `0..(M / 2)`.
    pub c_star_half_a: String,
    /// `c*_B`: `in_span` candidate maximizing mean total equity over `(M / 2)..M`.
    pub c_star_half_b: String,
    /// `EV_{H_A}(c*_A) - EV_{H_B}(c*_A)`.
    pub diff_a_to_b_equity: f64,
    /// `EV_{H_B}(c*_B) - EV_{H_A}(c*_B)`.
    pub diff_b_to_a_equity: f64,
    /// Number of scenarios in each disjoint half (`M / 2`).
    pub half_scenarios: usize,
    /// Explicit note that this estimates selection bias at `M / 2` scenarios (upper estimate for `M`).
    pub is_half_m_upper_estimate: bool,
}

/// Deterministic blind scores for one candidate inside a [`StateScoreOutput`] (`§7.1`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateScoreOutput {
    /// Canonical `candidate_id`.
    pub candidate_id: String,
    /// Integer-summed mean total equity across all `M` scenarios.
    pub ev_total_equity: f64,
    /// Whether this candidate enters `[v_min, v_max]`.
    pub in_span: bool,
    /// Whether this candidate is arm-independent.
    pub is_arm_independent: bool,
    /// Mean Turn-1 Private Mandate bonus across `M` scenarios.
    pub mean_mandate_bonus: f64,
    /// Mean terminal Net Asset Equity across `M` scenarios.
    pub mean_net_asset_equity: f64,
    /// Mean Project Equity across `M` scenarios.
    pub mean_project_equity: f64,
    /// Sample standard error across `m = 0..M` of paired per-scenario equity differences vs `c_best`.
    pub paired_se_equity: f64,
    /// `v_max - ev_total_equity` (`>= 0.0` for all `in_span == true` candidates).
    pub regret_equity: f64,
    /// Primary normalized `[0, 1]` score within `[v_min, v_max]`, or `None` (`null`) on flat spans.
    pub score_norm: Option<f64>,
    /// Empirical mid-rank percentile `[0.0, 1.0]` relative strictly to `C_indep(state)`.
    pub sensitivity_indep_percentile: Option<f64>,
    /// 1-based descending rank relative strictly to `C_indep(state)`.
    pub sensitivity_indep_rank: Option<usize>,
}

/// Pure, deterministic blind score output written to `<out>/scores/<state_id>.json`.
///
/// Omits all timestamps, durations, hostnames, filesystem paths, and arm labels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StateScoreOutput {
    /// Winning `in_span` candidate identifier achieving `v_max` (ties broken by ascending `candidate_id`).
    pub c_best: String,
    /// Per-candidate score records in ascending `candidate_id` order.
    pub candidates: Vec<CandidateScoreOutput>,
    /// `true` iff `v_max == v_min` (`score_norm` is `null` for every candidate).
    pub degenerate_flat_set: bool,
    /// Checkpoint turn number (`day`).
    pub day: u8,
    /// Zero-based focal seat index (`0..4`).
    pub focal_seat: u8,
    /// Checkpoint pair hash echoed from [`BlindScoringRequest`].
    pub pre_act_pair_sha256: String,
    /// Checkpoint save hash echoed from [`BlindScoringRequest`].
    pub pre_act_save_sha256: String,
    /// Checkpoint seal hash echoed from [`BlindScoringRequest`].
    pub pre_act_seal_sha256: String,
    /// Arm-blind pre-screen summary (`§3.2`).
    pub prescreen: PrescreenSummary,
    /// Number of CRN scenarios evaluated per candidate.
    pub scenarios_m: usize,
    /// Schema version (`1`).
    pub schema_version: u32,
    /// Hash of the frozen `ScorerConfig`.
    pub scorer_config_sha256: String,
    /// Explicit seed echoed from [`BlindScoringRequest`].
    pub scorer_seed: u64,
    /// Arm-blind canonical state identifier.
    pub state_id: String,
    /// Maximum `ev_total_equity` over `in_span == true` candidates.
    pub v_max: f64,
    /// Minimum `ev_total_equity` over `in_span == true` candidates.
    pub v_min: f64,
    /// Split-half Winner's curse upward-bias estimate (`§7.1`).
    pub winners_curse: WinnersCurseSummary,
}
