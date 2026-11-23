//! Blind input, frozen configuration, and deterministic output schemas for `league-scorer`.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use league_core::{EvalActSubmission, MarketAction, Units};
use league_seat_types::UNIT_SCALE;

use crate::error::{ScorerError, sanitize_error_text};

/// Fixed schema version accepted by S1 blind scoring requests.
pub const BLIND_SCORING_SCHEMA_VERSION: u32 = 1;

/// Fixed Evaluation Mode table size (`D6`).
pub const EVAL_TABLE_SEAT_COUNT: u8 = 4;

/// Default and only supported `ScorerConfig::composite_greedy_rules` rule identifier (`§5.2`).
pub const COMPOSITE_GREEDY_RULES_DEFAULT: &str = "top_pass_per_axis_plus_drop_max_upkeep";

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

/// Proposed fixed code constant: fixed charter auction `bid_cred` matching `ClubBot` (`bot.rs:554`).
pub const SYNTH_CHARTER_BID_CRED: Units = 150;

/// Proposed fixed code constant: fraction of spare `CRED` allocated per parity-anchored buy quote (bps).
pub const SYNTH_QUOTE_CRED_SHARE_BPS: u16 = 5_000;

/// Proposed fixed code constant: maximum commodity units per parity-anchored buy quote (`UNIT_SCALE`).
pub const SYNTH_QUOTE_MAX_UNITS: Units = UNIT_SCALE;

/// Proposed fixed code constant: minimum commodity units for parity-anchored synthetic quotes (`>= 100`).
pub const SYNTH_QUOTE_MIN_UNITS: Units = 100;

/// Proposed fixed code constant: parity-value multiplier applied when pricing `upkeep_good` sell drop risk.
pub const SYNTH_DROP_UPKEEP_SELL_RISK_MULTIPLIER: i64 = 2;

/// Proposed fixed code constant: minimum non-zero milli-`CRED` drop risk assigned to other market actions.
pub const SYNTH_DROP_DEFAULT_ACTION_RISK_CRED: Units = 1;

/// Proposed fixed code constant: milli-`CRED` sentinel drop risk prioritizing `project_deliveries` drops first.
pub const SYNTH_DROP_PROJECT_RISK_CRED: Units = i64::MAX / 2;

/// Proposed fixed code constant: maximum seed drafts drawn per axis into the `scaled` pool.
pub const SYNTH_SCALE_SEED_PER_AXIS: usize = 2;

/// Proposed fixed code constant: maximum combined seed drafts scaled per `quantity_scales_bps` entry.
pub const SYNTH_SCALE_SEED_POOL_MAX: usize = 6;

/// Proposed fixed code constant: integer parts-per-million denominator for Bernoulli talk-honour draws (`§4.3`, `§5.2`).
pub const HONOUR_PPM_SCALE: u64 = 1_000_000;

/// Frozen talk-commitment extraction rules version identifier (`§5.2`, `§8.2`).
pub const TALK_EXTRACTOR_VERSION_V2: &str = "talk_rules_v2";

/// Frozen canonical SHA-256 of `talk_rules_v2` (canonical `TalkRulesV2Table` JSON + LF-normalized `crates/league-eval/src/talk_extract.rs` source bytes; enforces per-message `allowed_ascii_chars` validation, thousands-comma grouping, whole-clause closed grammar matching, and fail-closed pair latching; upstream reply/record/whisper loaders are pinned by `scorer_git_sha` and `blind_inputs_manifest_sha256`, `§5.2`, `§8.2`).
pub const TALK_EXTRACTOR_RULES_DIGEST_V2: &str =
    "8edf70e7a60f3550f79b82dd354af84185094addd0333a9724bf701b0f56bfaa";

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
    /// Minimum distinct feasible candidates required in `C_indep(state)` (`§3.2`, placeholder until pilot calibration).
    pub prescreen_min_arm_indep_candidates: usize,
    /// Minimum arm-independent `v_max_indep - v_min_indep` span in integer equity (`§3.2`, placeholder until pilot calibration).
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
    /// 64-hex SHA-256 over the talk commitment extractor rule table and source code.
    pub talk_extractor_rules_digest: String,
    /// Talk commitment extractor version identifier.
    pub talk_extractor_version: String,
    /// Minimum upkeep turns reserved by synthetic rules and conservative posture.
    pub upkeep_cover_turns: u8,
}

impl Default for ScorerConfig {
    fn default() -> Self {
        Self {
            composite_greedy_rules: COMPOSITE_GREEDY_RULES_DEFAULT.to_owned(),
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
            // Placeholder defaults until arm-blind pilot calibration (§3.2, §5.2).
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
            talk_extractor_rules_digest: TALK_EXTRACTOR_RULES_DIGEST_V2.to_owned(),
            talk_extractor_version: TALK_EXTRACTOR_VERSION_V2.to_owned(),
            upkeep_cover_turns: 2,
        }
    }
}

/// Allowed bucket identifiers for [`ScorerConfig::synth_slot_fill_order`].
pub const SYNTH_SLOT_BUCKET_NAMES: [&str; 8] = [
    "cleared_pass_b",
    "claims_council",
    "upkeep_inputs",
    "cross_quotes",
    "batches",
    "debt",
    "composite_drops",
    "scaled",
];

impl ScorerConfig {
    /// Builds an all-Standard `ScorerConfig` using `scenarios_m` CRN scenarios (useful for baseline tests).
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

    /// Validates all `ScorerConfig` invariants (`§5.2`, `§7.1`).
    pub fn validate(&self) -> Result<(), ScorerError> {
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
        let weight_sum = u64::from(self.posture_weights.standard)
            + u64::from(self.posture_weights.active)
            + u64::from(self.posture_weights.conservative);
        if weight_sum == 0 {
            return Err(ScorerError::InvalidScorerConfig(
                "posture_weights sum must be > 0".to_owned(),
            ));
        }
        let sched_std = self
            .posture_schedule
            .iter()
            .filter(|&&p| p == PostureKind::Standard)
            .count() as u128;
        let sched_act = self
            .posture_schedule
            .iter()
            .filter(|&&p| p == PostureKind::Active)
            .count() as u128;
        let sched_cons = self
            .posture_schedule
            .iter()
            .filter(|&&p| p == PostureKind::Conservative)
            .count() as u128;
        let w_std = u128::from(self.posture_weights.standard);
        let w_act = u128::from(self.posture_weights.active);
        let w_cons = u128::from(self.posture_weights.conservative);
        if sched_std * w_act != sched_act * w_std
            || sched_std * w_cons != sched_cons * w_std
            || sched_act * w_cons != sched_cons * w_act
        {
            return Err(ScorerError::InvalidScorerConfig(format!(
                "posture_weights ({}, {}, {}) disagrees with posture_schedule counts ({sched_std}, {sched_act}, {sched_cons})",
                self.posture_weights.standard,
                self.posture_weights.active,
                self.posture_weights.conservative
            )));
        }
        if self.composite_greedy_rules != COMPOSITE_GREEDY_RULES_DEFAULT {
            return Err(ScorerError::InvalidScorerConfig(format!(
                "unsupported composite_greedy_rules `{}` (expected `{COMPOSITE_GREEDY_RULES_DEFAULT}`)",
                self.composite_greedy_rules
            )));
        }
        if self.synth_slot_fill_order.is_empty() {
            return Err(ScorerError::InvalidScorerConfig(
                "synth_slot_fill_order cannot be empty".to_owned(),
            ));
        }
        let mut seen_slots = std::collections::BTreeSet::new();
        for slot in &self.synth_slot_fill_order {
            if !SYNTH_SLOT_BUCKET_NAMES.contains(&slot.as_str()) {
                return Err(ScorerError::InvalidScorerConfig(format!(
                    "unknown synth_slot_fill_order bucket `{slot}`"
                )));
            }
            if !seen_slots.insert(slot.as_str()) {
                return Err(ScorerError::InvalidScorerConfig(format!(
                    "duplicate synth_slot_fill_order bucket `{slot}`"
                )));
            }
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

/// Neutral counterfactual talk-phase trade commitment (`§4.3`, `§8.1`).
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

/// Validates a single [`ExtractedTalkCommitment`] against table bounds and counterparty consistency (`§4.3`).
pub fn validate_talk_commitment(
    commitment: &ExtractedTalkCommitment,
    focal_seat: u8,
    seat_count: u8,
) -> Result<(), ScorerError> {
    if commitment.proposer_seat >= seat_count {
        return Err(ScorerError::InvalidTalkCommitment(format!(
            "proposer_seat {} out of range 0..{seat_count}",
            commitment.proposer_seat
        )));
    }
    if commitment.counterparty_seat >= seat_count {
        return Err(ScorerError::InvalidTalkCommitment(format!(
            "counterparty_seat {} out of range 0..{seat_count}",
            commitment.counterparty_seat
        )));
    }
    if commitment.proposer_seat == commitment.counterparty_seat {
        return Err(ScorerError::InvalidTalkCommitment(format!(
            "proposer_seat {} equals counterparty_seat",
            commitment.proposer_seat
        )));
    }
    if commitment.proposer_seat == focal_seat {
        return Err(ScorerError::InvalidTalkCommitment(format!(
            "proposer_seat {focal_seat} cannot match focal_seat"
        )));
    }
    let actor = commitment.offered_action.actor_seat();
    if actor != commitment.proposer_seat {
        return Err(ScorerError::InvalidTalkCommitment(format!(
            "offered_action actor_seat {actor} != proposer_seat {}",
            commitment.proposer_seat
        )));
    }
    match &commitment.offered_action {
        MarketAction::CreateCheck { check, .. } if check.payee != commitment.counterparty_seat => {
            return Err(ScorerError::InvalidTalkCommitment(format!(
                "CreateCheck payee {} != counterparty_seat {}",
                check.payee, commitment.counterparty_seat
            )));
        }
        MarketAction::CreateEscrow { escrow, .. }
            if escrow.destination != commitment.counterparty_seat =>
        {
            return Err(ScorerError::InvalidTalkCommitment(format!(
                "CreateEscrow destination {} != counterparty_seat {}",
                escrow.destination, commitment.counterparty_seat
            )));
        }
        MarketAction::FlagWashFreeze { target_seat, .. }
            if *target_seat != commitment.counterparty_seat =>
        {
            return Err(ScorerError::InvalidTalkCommitment(format!(
                "FlagWashFreeze target_seat {target_seat} != counterparty_seat {}",
                commitment.counterparty_seat
            )));
        }
        MarketAction::RecordIntentPledge { pledge, .. }
            if pledge.promisor != commitment.proposer_seat
                || pledge.counterparty != commitment.counterparty_seat =>
        {
            return Err(ScorerError::InvalidTalkCommitment(format!(
                "RecordIntentPledge ({}, {}) != ({}, {})",
                pledge.promisor,
                pledge.counterparty,
                commitment.proposer_seat,
                commitment.counterparty_seat
            )));
        }
        _ => {}
    }
    Ok(())
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

/// Per-source talk message extraction counts included in [`BlindScoringRequest`] (`§4.3`, `§8.1`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TalkSourceCounts {
    /// Messages failing closed as ambiguous for their `(proposer, counterparty)` pair.
    pub ambiguous: u64,
    /// Messages in which every clause matched a closed-grammar template cleanly.
    pub matched: u64,
    /// Total peer messages targeting `focal_seat` inspected from this source (`matched + ambiguous`).
    pub seen: u64,
}

impl TalkSourceCounts {
    /// Checks `matched + ambiguous == seen`.
    pub fn validate(&self, label: &str) -> Result<(), ScorerError> {
        let sum = self.matched.checked_add(self.ambiguous).ok_or_else(|| {
            ScorerError::InvalidTalkCommitment(format!("{label} talk counts overflow u64"))
        })?;
        if sum != self.seen {
            return Err(ScorerError::InvalidTalkCommitment(format!(
                "{label} seen count {} != matched {} + ambiguous {}",
                self.seen, self.matched, self.ambiguous
            )));
        }
        Ok(())
    }
}

/// Per-state message extraction counts covered by `blind_request_sha256` (`§4.3`, `§7.1`, `§8.1`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TalkExtractionCounts {
    /// Counts from `to-seat/day-DD-act-<Guild>.negotiation.json` whispers targeting `focal_seat`.
    pub negotiation: TalkSourceCounts,
    /// Counts from `decisions/day-DD-talk-<Guild>.json` replies targeting `focal_seat`.
    pub talk: TalkSourceCounts,
    /// Element-wise sum of `negotiation` and `talk` counts.
    pub total: TalkSourceCounts,
}

impl TalkExtractionCounts {
    /// Constructs verified totals from per-source `negotiation` and `talk` counts.
    pub fn from_sources(
        negotiation: TalkSourceCounts,
        talk: TalkSourceCounts,
    ) -> Result<Self, ScorerError> {
        negotiation.validate("negotiation")?;
        talk.validate("talk")?;
        let seen = negotiation.seen.checked_add(talk.seen).ok_or_else(|| {
            ScorerError::InvalidTalkCommitment("total seen talk count overflow".to_owned())
        })?;
        let matched = negotiation
            .matched
            .checked_add(talk.matched)
            .ok_or_else(|| {
                ScorerError::InvalidTalkCommitment("total matched talk count overflow".to_owned())
            })?;
        let ambiguous = negotiation
            .ambiguous
            .checked_add(talk.ambiguous)
            .ok_or_else(|| {
                ScorerError::InvalidTalkCommitment("total ambiguous talk count overflow".to_owned())
            })?;
        let out = Self {
            negotiation,
            talk,
            total: TalkSourceCounts {
                ambiguous,
                matched,
                seen,
            },
        };
        out.validate()?;
        Ok(out)
    }

    /// Verifies per-source `seen == matched + ambiguous` and `total == negotiation + talk`.
    pub fn validate(&self) -> Result<(), ScorerError> {
        self.negotiation.validate("negotiation")?;
        self.talk.validate("talk")?;
        self.total.validate("total")?;
        if self.negotiation.seen.checked_add(self.talk.seen) != Some(self.total.seen)
            || self.negotiation.matched.checked_add(self.talk.matched) != Some(self.total.matched)
            || self.negotiation.ambiguous.checked_add(self.talk.ambiguous)
                != Some(self.total.ambiguous)
        {
            return Err(ScorerError::InvalidTalkCommitment(
                "talk_extraction_counts.total disagrees with negotiation + talk".to_owned(),
            ));
        }
        Ok(())
    }
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
    /// Neutral counterfactual talk-phase trade commitments (`§4.3`).
    pub talk_commitments: Vec<ExtractedTalkCommitment>,
    /// Per-source and total talk message classification counts (`§4.3`, `§8.1`).
    #[serde(default)]
    pub talk_extraction_counts: TalkExtractionCounts,
}

/// Parses raw JSON bytes into [`BlindScoringRequest`], refusing unknown or non-canonical fields
/// inside nested `candidate.submission` structs without modifying `league-core` (`§8.1`).
pub fn parse_blind_scoring_request_strict(
    bytes: &[u8],
) -> Result<BlindScoringRequest, ScorerError> {
    let raw_val: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|e| ScorerError::Json(format!("parse BlindScoringRequest JSON: {e}")))?;
    let parsed: BlindScoringRequest = serde_json::from_value(raw_val.clone())
        .map_err(|e| ScorerError::Json(format!("parse BlindScoringRequest schema: {e}")))?;
    let Some(raw_cands) = raw_val.get("candidates").and_then(|v| v.as_array()) else {
        return Err(ScorerError::UnknownSubmissionField(
            "missing candidates array in raw JSON".to_owned(),
        ));
    };
    if raw_cands.len() != parsed.candidates.len() {
        return Err(ScorerError::UnknownSubmissionField(
            "candidates array length mismatch".to_owned(),
        ));
    }
    for (idx, (raw_cand, parsed_cand)) in raw_cands.iter().zip(parsed.candidates.iter()).enumerate()
    {
        let Some(raw_sub) = raw_cand.get("submission") else {
            return Err(ScorerError::UnknownSubmissionField(format!(
                "candidate[{idx}] missing submission object"
            )));
        };
        let canonical_sub = serde_json::to_value(&parsed_cand.submission)
            .map_err(|e| ScorerError::Json(format!("re-serialize submission: {e}")))?;
        if *raw_sub != canonical_sub {
            return Err(ScorerError::UnknownSubmissionField(format!(
                "candidate[{idx}] submission contains unknown or non-canonical JSON fields"
            )));
        }
    }
    Ok(parsed)
}

impl BlindScoringRequest {
    /// Validates every structural, hashing, ordering, talk-commitment, and config-binding invariant.
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
        for commitment in &self.talk_commitments {
            validate_talk_commitment(commitment, self.focal_seat, EVAL_TABLE_SEAT_COUNT)?;
        }
        self.talk_extraction_counts.validate()?;
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
    /// SHA-256 hex digest of the blind scoring request (`§8.2`). `league-scorer score` records the
    /// exact file bytes read from `--blind-inputs/<state_id>.json`; direct library calls to
    /// `execute_blind_request` default to the SHA-256 of `to_sorted_pretty_json(request)`.
    pub blind_request_sha256: String,
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
    /// Per-source and total talk message classification counts echoed from [`BlindScoringRequest`].
    #[serde(default)]
    pub talk_extraction_counts: TalkExtractionCounts,
    /// Maximum `ev_total_equity` over `in_span == true` candidates.
    pub v_max: f64,
    /// Minimum `ev_total_equity` over `in_span == true` candidates.
    pub v_min: f64,
    /// Split-half Winner's curse upward-bias estimate (`§7.1`).
    pub winners_curse: WinnersCurseSummary,
}

/// Blinded projection of `scorer_freeze.json` read by `league-scorer score --freeze` (`§8.2` step 4).
///
/// Intentionally omits `deny_unknown_fields` so unblinded fields (such as `control_wording_arm`)
/// remain invisible to `league-scorer`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScorerFreezeBlindView {
    /// Sorted manifest SHA-256 of `blind_inputs/*.json`.
    pub blind_inputs_manifest_sha256: String,
    /// Canonical SHA-256 of the frozen [`ScorerConfig`].
    pub scorer_config_sha256: String,
    /// Canonical SHA-256 over `crates/league-scorer/**`.
    pub scorer_tree_sha256: String,
}

impl ScorerFreezeBlindView {
    /// Validates all SHA-256 digest strings on the freeze view.
    pub fn validate(&self) -> Result<(), ScorerError> {
        if !is_valid_sha256_hex(&self.scorer_tree_sha256) {
            return Err(ScorerError::InvalidHexDigest {
                field: "scorer_tree_sha256",
                value: self.scorer_tree_sha256.clone(),
            });
        }
        if !is_valid_sha256_hex(&self.scorer_config_sha256) {
            return Err(ScorerError::InvalidHexDigest {
                field: "scorer_config_sha256",
                value: self.scorer_config_sha256.clone(),
            });
        }
        if !is_valid_sha256_hex(&self.blind_inputs_manifest_sha256) {
            return Err(ScorerError::InvalidHexDigest {
                field: "blind_inputs_manifest_sha256",
                value: self.blind_inputs_manifest_sha256.clone(),
            });
        }
        Ok(())
    }
}

/// Computes `blind_inputs_manifest_sha256` from a filename-sorted map of `filename -> file_sha256` (`§8.2`).
#[must_use]
pub fn compute_blind_inputs_manifest_sha256_from_entries(
    entries: &BTreeMap<String, String>,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"merchant-league/blind-inputs-manifest/v1\n");
    for (filename, file_sha256) in entries {
        hasher.update(filename.as_bytes());
        hasher.update(b":");
        hasher.update(file_sha256.as_bytes());
        hasher.update(b"\n");
    }
    let digest: [u8; 32] = hasher.finalize().into();
    encode_digest_hex(&digest)
}

/// Computes the sorted SHA-256 manifest digest over all `*.json` files in `blind_inputs_dir` (`§8.2`).
pub fn compute_blind_inputs_manifest_sha256(
    blind_inputs_dir: &Path,
) -> Result<String, ScorerError> {
    let read_dir = fs::read_dir(blind_inputs_dir).map_err(|e| {
        ScorerError::NoBlindInputs(sanitize_error_text(
            &format!("cannot read blind inputs directory: {e}"),
            &[blind_inputs_dir],
        ))
    })?;
    let mut entries = BTreeMap::new();
    for entry_res in read_dir {
        let entry = entry_res.map_err(|e| {
            ScorerError::Io(sanitize_error_text(
                &format!("read blind inputs entry: {e}"),
                &[blind_inputs_dir],
            ))
        })?;
        let ft = entry.file_type().map_err(|e| {
            ScorerError::Io(sanitize_error_text(
                &format!("inspect blind inputs entry: {e}"),
                &[blind_inputs_dir, &entry.path()],
            ))
        })?;
        if ft.is_file() && entry.path().extension() == Some(OsStr::new("json")) {
            let name = entry.file_name().into_string().map_err(|_| {
                ScorerError::InvalidStateId("non-UTF-8 blind input filename".to_owned())
            })?;
            let bytes = fs::read(entry.path()).map_err(|e| {
                ScorerError::Io(sanitize_error_text(
                    &format!("read blind input bytes: {e}"),
                    &[blind_inputs_dir, &entry.path()],
                ))
            })?;
            entries.insert(name, sha256_hex(&bytes));
        }
    }
    if entries.is_empty() {
        return Err(ScorerError::NoBlindInputs(
            "no *.json request files found in --blind-inputs".to_owned(),
        ));
    }
    Ok(compute_blind_inputs_manifest_sha256_from_entries(&entries))
}

fn collect_scorer_tree_entries(
    root: &Path,
    dir: &Path,
    rel_prefix: &str,
    entries: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), ScorerError> {
    let read_dir = fs::read_dir(dir).map_err(|e| {
        ScorerError::Io(sanitize_error_text(
            &format!("read scorer crate dir: {e}"),
            &[root, dir],
        ))
    })?;
    for entry_res in read_dir {
        let entry = entry_res.map_err(|e| {
            ScorerError::Io(sanitize_error_text(
                &format!("read scorer crate entry: {e}"),
                &[root, dir],
            ))
        })?;
        let name_os = entry.file_name();
        let name = name_os.to_str().ok_or_else(|| {
            ScorerError::Io("non-UTF-8 filename in scorer crate directory".to_owned())
        })?;
        let ft = entry.file_type().map_err(|e| {
            ScorerError::Io(sanitize_error_text(
                &format!("inspect scorer crate entry: {e}"),
                &[root, &entry.path()],
            ))
        })?;
        let rel = if rel_prefix.is_empty() {
            name.to_owned()
        } else {
            format!("{rel_prefix}/{name}")
        };
        if ft.is_dir() {
            if name == "target" || name == ".git" {
                continue;
            }
            collect_scorer_tree_entries(root, &entry.path(), &rel, entries)?;
        } else if ft.is_file() {
            let bytes = fs::read(entry.path()).map_err(|e| {
                ScorerError::Io(sanitize_error_text(
                    &format!("read scorer crate file: {e}"),
                    &[root, &entry.path()],
                ))
            })?;
            entries.insert(rel, bytes);
        }
    }
    Ok(())
}

/// Computes `scorer_tree_sha256` across `crate_dir` (`crates/league-scorer/**`), walking files
/// in ascending POSIX relative-path order (excluding `target/` and `.git/`).
pub fn compute_scorer_tree_sha256(crate_dir: &Path) -> Result<String, ScorerError> {
    let mut entries = BTreeMap::new();
    collect_scorer_tree_entries(crate_dir, crate_dir, "", &mut entries)?;
    let mut hasher = Sha256::new();
    hasher.update(b"merchant-league/scorer-tree/v1\n");
    for (rel, bytes) in entries {
        hasher.update(rel.as_bytes());
        hasher.update(b"\0");
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(b"\0");
        hasher.update(&bytes);
    }
    let digest: [u8; 32] = hasher.finalize().into();
    Ok(encode_digest_hex(&digest))
}
