//! Persisted game setup, manifest, refusal counters, and per-decision records (`1.8`–`1.11`, `R7`, `R8`).

use std::collections::BTreeMap;

use league_core::{EvalActSubmission, EvalSeatApplyReport, PublicSweepReceipt, SealedOrderBundle};
use league_relay::{Round, sha256_hex};
use serde::{Deserialize, Serialize};

use crate::error::EvalError;
use crate::reply::ReplyRefusalCode;

/// Stated limit included in every Evaluation manifest (`Item 9` addendum 1.10).
pub const PASS_B_LIMIT_STATEMENT: &str = "ClubBot seats get two passes (Pass A and Pass B); model seats get one pass (Pass A only, pass_b_actions empty), identically across all arms.";

/// Pre-flight wrong-part refusal rate threshold (`5%`, `Item 9` addendum 1.8).
pub const PREFLIGHT_WRONG_PART_STOP_THRESHOLD: f64 = 0.05;

/// Requested Evaluation arm for `league-eval play` and `league-eval branch` (`D1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvalArm {
    /// No preview or draft tool (`v2_neutral_2026_11_15_r6`).
    Off,
    /// Matched inert draft tool (lands in PR 3).
    Control,
    /// Full own-orders rules-engine preview tool (`v2_preview_2026_11_20_r2`).
    On,
}

impl EvalArm {
    /// Parse `"off"`, `"control"`, or `"on"` (case-insensitive).
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "off" => Ok(Self::Off),
            "control" => Ok(Self::Control),
            "on" => Ok(Self::On),
            other => Err(format!(
                "unknown Eval arm '{other}': expected 'off', 'control', or 'on'"
            )),
        }
    }

    /// Canonical lowercase CLI / manifest string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Control => "control",
            Self::On => "on",
        }
    }
}

/// Cumulative counts of structured-reply refusal codes and refused bundles (`1.9`, `R7`, `A1`, `B1`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefusalCodeCounts {
    /// Count of `ACT_REPLY_INVALID` refusals.
    pub act_reply_invalid: u64,
    /// Count of `TALK_REPLY_INVALID` refusals (`B1`).
    #[serde(default)]
    pub talk_reply_invalid: u64,
    /// Count of `REPLY_SCHEMA_INVALID` no-reply failures (`A1.2`).
    #[serde(default)]
    pub reply_schema_invalid: u64,
    /// Count of `TALK_REPLY_HAS_ORDERS` refusals.
    pub talk_reply_has_orders: u64,
    /// Count of `ACT_REPLY_HAS_MESSAGES` refusals.
    pub act_reply_has_messages: u64,
    /// Count of `MESSAGE_RECIPIENT_INVALID` diagnostics.
    pub message_recipient_invalid: u64,
    /// Count of whole-bundle refusals (`R7`).
    pub refused_bundles: u64,
}

impl RefusalCodeCounts {
    /// Increment the counter corresponding to `code`.
    pub fn record(&mut self, code: ReplyRefusalCode) {
        match code {
            ReplyRefusalCode::ActReplyInvalid => self.act_reply_invalid += 1,
            ReplyRefusalCode::TalkReplyInvalid => self.talk_reply_invalid += 1,
            ReplyRefusalCode::ReplySchemaInvalid => self.reply_schema_invalid += 1,
            ReplyRefusalCode::TalkReplyHasOrders => self.talk_reply_has_orders += 1,
            ReplyRefusalCode::ActReplyHasMessages => self.act_reply_has_messages += 1,
            ReplyRefusalCode::MessageRecipientInvalid => self.message_recipient_invalid += 1,
        }
    }

    /// Increment counts for all entries in `other`.
    pub fn merge_from(&mut self, other: &Self) {
        self.act_reply_invalid += other.act_reply_invalid;
        self.talk_reply_invalid += other.talk_reply_invalid;
        self.reply_schema_invalid += other.reply_schema_invalid;
        self.talk_reply_has_orders += other.talk_reply_has_orders;
        self.act_reply_has_messages += other.act_reply_has_messages;
        self.message_recipient_invalid += other.message_recipient_invalid;
        self.refused_bundles += other.refused_bundles;
    }
}

/// Cumulative counter of wrong-part refusals (`TALK_REPLY_HAS_ORDERS` + `ACT_REPLY_HAS_MESSAGES`)
/// tracking delivered wrong-part replies separately from `no_reply` wrong-round refusals and
/// reporting both rates: `wrong_part_rate_delivered` (delivered wrong-part replies over
/// `delivered_model_replies`) and `wrong_part_rate_all_rounds` (all wrong-round refusals over
/// `total_model_rounds`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WrongPartCounter {
    /// Count of `TALK_REPLY_HAS_ORDERS`.
    pub talk_reply_has_orders: u64,
    /// Count of `ACT_REPLY_HAS_MESSAGES`.
    pub act_reply_has_messages: u64,
    /// Count of wrong-part refusals across delivered replies (`delivered_model_replies`).
    #[serde(default)]
    pub delivered_wrong_part_refusals: u64,
    /// Count of wrong-round refusals across `no_reply` rounds whose discarded raw JSON carried wrong-round content.
    #[serde(default)]
    pub no_reply_wrong_part_refusals: u64,
    /// Sum of delivered and no-reply wrong-round refusals (`talk_reply_has_orders + act_reply_has_messages`).
    pub total_wrong_part_refusals: u64,
    /// Total delivered model replies evaluated (both rounds, any seat, any arm).
    #[serde(alias = "total_model_replies")]
    pub delivered_model_replies: u64,
    /// Total evaluated model rounds including `no_reply` rounds.
    #[serde(default)]
    pub total_model_rounds: u64,
    /// True delivered wrong-part rate (`delivered_wrong_part_refusals / delivered_model_replies`, `0.0` when `delivered_model_replies == 0`). Never clamped.
    #[serde(alias = "wrong_part_rate")]
    pub wrong_part_rate_delivered: f64,
    /// All-rounds wrong-part rate (`(delivered_wrong_part_refusals + no_reply_wrong_part_refusals) / total_model_rounds`, `0.0` when `total_model_rounds == 0`). Never clamped.
    #[serde(default)]
    pub wrong_part_rate_all_rounds: f64,
    /// `true` iff `wrong_part_rate_delivered > 0.05` (`1.8` stop line).
    #[serde(alias = "exceeds_five_percent_preflight_stop_line")]
    pub exceeds_five_percent_preflight_stop_line_delivered: bool,
    /// `true` iff `wrong_part_rate_all_rounds > 0.05` (`1.8` stop line).
    #[serde(default)]
    pub exceeds_five_percent_preflight_stop_line_all_rounds: bool,
}

impl WrongPartCounter {
    /// Record one delivered model reply (`wrong_part_code` if wrong-part refusal occurred).
    pub fn record_delivered_reply(&mut self, wrong_part_code: Option<ReplyRefusalCode>) {
        self.delivered_model_replies += 1;
        self.total_model_rounds += 1;
        match wrong_part_code {
            Some(ReplyRefusalCode::TalkReplyHasOrders) => {
                self.talk_reply_has_orders += 1;
                self.delivered_wrong_part_refusals += 1;
                self.total_wrong_part_refusals += 1;
            }
            Some(ReplyRefusalCode::ActReplyHasMessages) => {
                self.act_reply_has_messages += 1;
                self.delivered_wrong_part_refusals += 1;
                self.total_wrong_part_refusals += 1;
            }
            _ => {}
        }
        self.recompute_rate();
    }

    /// Record one evaluated model round that yielded no delivered reply (`no_reply == true`,
    /// without wrong-round content).
    pub fn record_no_reply(&mut self) {
        self.total_model_rounds += 1;
        self.recompute_rate();
    }

    /// Record one evaluated model round with `no_reply == true` whose discarded raw JSON
    /// held a wrong-round payload (`TALK_REPLY_HAS_ORDERS` or `ACT_REPLY_HAS_MESSAGES`).
    pub fn record_wrong_part_no_reply(&mut self, wrong_part_code: ReplyRefusalCode) {
        match wrong_part_code {
            ReplyRefusalCode::TalkReplyHasOrders => {
                self.talk_reply_has_orders += 1;
                self.no_reply_wrong_part_refusals += 1;
                self.total_wrong_part_refusals += 1;
            }
            ReplyRefusalCode::ActReplyHasMessages => {
                self.act_reply_has_messages += 1;
                self.no_reply_wrong_part_refusals += 1;
                self.total_wrong_part_refusals += 1;
            }
            _ => {}
        }
        self.total_model_rounds += 1;
        self.recompute_rate();
    }

    /// Backwards-compatible alias for [`Self::record_delivered_reply`].
    pub fn record_reply(&mut self, wrong_part_code: Option<ReplyRefusalCode>) {
        self.record_delivered_reply(wrong_part_code);
    }

    /// Merge counts from another counter (e.g., from a branch run).
    pub fn merge_from(&mut self, other: &Self) {
        self.talk_reply_has_orders += other.talk_reply_has_orders;
        self.act_reply_has_messages += other.act_reply_has_messages;
        self.delivered_wrong_part_refusals += other.delivered_wrong_part_refusals;
        self.no_reply_wrong_part_refusals += other.no_reply_wrong_part_refusals;
        self.total_wrong_part_refusals += other.total_wrong_part_refusals;
        self.delivered_model_replies += other.delivered_model_replies;
        self.total_model_rounds += other.total_model_rounds;
        self.recompute_rate();
    }

    /// Validate that computed rates and numerators remain within `[0.0, 1.0]` without clamping.
    pub fn validate_rates(&self) -> Result<(), EvalError> {
        let all_wrong_part = self
            .delivered_wrong_part_refusals
            .saturating_add(self.no_reply_wrong_part_refusals);
        if self.delivered_wrong_part_refusals > self.delivered_model_replies
            || !self.wrong_part_rate_delivered.is_finite()
            || !(0.0..=1.0).contains(&self.wrong_part_rate_delivered)
        {
            return Err(EvalError::InvalidWrongPartRate(format!(
                "wrong_part_rate_delivered={} out of bounds for {}/{} delivered replies",
                self.wrong_part_rate_delivered,
                self.delivered_wrong_part_refusals,
                self.delivered_model_replies
            )));
        }
        if all_wrong_part > self.total_model_rounds
            || !self.wrong_part_rate_all_rounds.is_finite()
            || !(0.0..=1.0).contains(&self.wrong_part_rate_all_rounds)
        {
            return Err(EvalError::InvalidWrongPartRate(format!(
                "wrong_part_rate_all_rounds={} out of bounds for {}/{} total model rounds",
                self.wrong_part_rate_all_rounds, all_wrong_part, self.total_model_rounds
            )));
        }
        Ok(())
    }

    /// Recompute both wrong-part rates and refuse if any computed rate falls outside `[0.0, 1.0]`.
    pub fn try_recompute_rates(&mut self) -> Result<(), EvalError> {
        self.recompute_rate();
        self.validate_rates()
    }

    fn recompute_rate(&mut self) {
        self.wrong_part_rate_delivered = if self.delivered_model_replies == 0 {
            0.0
        } else {
            (self.delivered_wrong_part_refusals as f64) / (self.delivered_model_replies as f64)
        };
        let all_wrong_part = self
            .delivered_wrong_part_refusals
            .saturating_add(self.no_reply_wrong_part_refusals);
        self.total_wrong_part_refusals = all_wrong_part;
        self.wrong_part_rate_all_rounds = if self.total_model_rounds == 0 {
            0.0
        } else {
            (all_wrong_part as f64) / (self.total_model_rounds as f64)
        };
        self.exceeds_five_percent_preflight_stop_line_delivered =
            self.wrong_part_rate_delivered > PREFLIGHT_WRONG_PART_STOP_THRESHOLD;
        self.exceeds_five_percent_preflight_stop_line_all_rounds =
            self.wrong_part_rate_all_rounds > PREFLIGHT_WRONG_PART_STOP_THRESHOLD;
    }
}

/// Diagnostic for one failed attempt recorded by `league-relay` in `relay/calls.jsonl` (`A1.1`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailedRelayAttempt {
    /// 1-based attempt number (`CallLogEntry::attempt`).
    pub attempt: u8,
    /// Outcome string written by `league-relay` (e.g. `"harness"`, `"NO_REPLY"`, `"LATE_DISCARDED"`).
    pub outcome: String,
    /// Error or failure reason recorded by `league-relay`.
    pub reason: String,
}

/// Whole refused bundle record required by `R7` and `Item 9` addendum 1.11.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefusedBundleRecord {
    /// Complete [`SealedOrderBundle`] submitted by the seat (all legs preserved).
    pub submitted_bundle: SealedOrderBundle,
    /// Engine error code string returned by reveal validation (for example `"INVALID_ORDER"`).
    pub error_code: String,
    /// Failing leg index when the engine names one, or `"leg not named"` (`engine.rs:1614-1655`).
    pub bad_leg_index: String,
}

/// Per-decision structured record written for every round and branch evaluation (`1.3`, `1.4`, `1.9`, `1.11`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvalDecisionRecord {
    /// Per-game identifier from `EvalHost::game_id()`.
    pub game_id: String,
    /// Table identifier.
    pub table_id: u64,
    /// 1-based day/turn index.
    pub day: u8,
    /// Round (`"talk"` or `"act"`).
    pub round: Round,
    /// Zero-based seat index.
    pub seat: u8,
    /// Guild name.
    pub guild: String,
    /// Active arm (`"off"`, `"control"`, `"on"`).
    pub arm: EvalArm,
    /// `"ClubBot"` or `"Model"`.
    pub seat_kind: String,
    /// Whether this seat was configured to keep `"notes"`.
    pub keeps_notes: bool,
    /// `true` iff `keeps_notes` is true and a string `"notes"` key appeared in the raw reply (`1.4`).
    pub notes_present: bool,
    /// Raw delivered `from-seat/` text (or discarded reply `assistant_content` when refused; `None` if timeout/transport failure or ClubBot).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_reply: Option<String>,
    /// Relative path (`relay/seats/...`) to the discarded-reply sidecar when `league-relay` discarded the reply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discarded_reply_path: Option<String>,
    /// Lowercase hex SHA-256 of the discarded-reply sidecar file (`CallLogEntry::reply_sha256`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discarded_reply_sha256: Option<String>,
    /// Total attempts recorded in `relay/calls.jsonl` for this seat-round (`A1.1`).
    #[serde(default)]
    pub attempts: u32,
    /// Per-attempt failure diagnostics extracted from `relay/calls.jsonl` (`A1.1`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed_attempts: Vec<FailedRelayAttempt>,
    /// Source history file SHA-256 digests loaded when reconstructing a frozen `branch` prompt (`B4`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub loaded_history_sha256: BTreeMap<String, String>,
    /// Reply-level refusal code, if refused (`1.3`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refusal_code: Option<String>,
    /// Reply-level refusal explanation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refusal_reason: Option<String>,
    /// Per-message invalid-recipient diagnostics (`MESSAGE_RECIPIENT_INVALID`, `1.3`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub invalid_recipients: Vec<String>,
    /// `true` when `league-relay` returned no delivered reply (`NoReply` / timeout / error).
    pub no_reply: bool,
    /// Act submission apply report returned by `EvalHost::apply_act_submissions` (act rounds only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apply_report: Option<EvalSeatApplyReport>,
    /// Detailed record if the seat's submitted bundle was refused whole (`R7`, `1.11`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refused_bundle: Option<RefusedBundleRecord>,
    /// Optional `ClubBot::decide_full_turn` baseline decision at this seat/state (prepared for D4 / Card 11).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub club_bot_baseline: Option<EvalActSubmission>,
}

/// Top-level `<game-dir>/setup.json` written by `league-eval play` (`1.7`, `R5`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvalSetupFile {
    /// Per-game identifier (`R5`).
    pub game_id: String,
    /// Table identifier.
    pub table_id: u64,
    /// Layout seed passed to `league-eval play`.
    pub seed: u64,
    /// Planned turn cap passed via `--turns`.
    pub turns: u8,
    /// Seat count (strictly `4` for `D6` / `R9`).
    pub seat_count: u8,
    /// Arm (`"off"`, `"control"`, `"on"`).
    pub arm: EvalArm,
    /// Evaluation reply wording arm identifier (`v2_eval_reply_2026_11_21_r1`).
    pub eval_reply_wording_arm: String,
    /// Relay wording arm locked in `relay/setup.json`.
    pub relay_wording_arm: String,
    /// Wrapper version string (`1.7`).
    pub wrapper_version: String,
    /// SHA-256 hex digest of wrapper file bytes (`1.7`).
    pub wrapper_sha256: String,
    /// Prompt memory mode (`"snapshot"`, `"emc"`, or `"full"`, `C11`).
    pub memory_mode: String,
    /// SHA-256 hex digest of raw `seats.json` bytes (`C11`).
    pub seats_sha256: String,
    /// SHA-256 hex digest of the unified `ResponseFormatConfig`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_schema_sha256: Option<String>,
}

/// Game-level `<game-dir>/manifest.json` written by `league-eval play` (`1.7`–`1.10`, `R5`, `R7`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvalManifest {
    /// Per-game identifier (`R5`, assigned at game dir creation and distinct across D2 duplicates).
    pub game_id: String,
    /// Table identifier.
    pub table_id: u64,
    /// Table layout seed.
    pub seed: u64,
    /// Arm active during this game.
    pub arm: EvalArm,
    /// Evaluation reply wording arm (`v2_eval_reply_2026_11_21_r1`).
    pub eval_reply_wording_arm: String,
    /// Relay wording arm.
    pub relay_wording_arm: String,
    /// Wrapper brief version (`1.7`).
    pub wrapper_version: String,
    /// SHA-256 hex digest of wrapper brief bytes (`1.7`).
    pub wrapper_sha256: String,
    /// Stated Pass B limit (`1.10`).
    pub pass_b_limit: String,
    /// Number of played turns.
    pub played_turns: u8,
    /// Per-seat summary metadata (`0..4`).
    pub seats: Vec<EvalManifestSeat>,
    /// Cumulative refusal counts keyed by seat guild (`1.9`).
    pub refusal_counts_by_seat: BTreeMap<String, RefusalCodeCounts>,
    /// Cumulative refusal counts keyed by arm string (`1.9`, `R7`).
    pub refusal_counts_by_arm: BTreeMap<String, RefusalCodeCounts>,
    /// Pre-flight wrong-part counter over all evaluated model replies (`1.8`).
    pub wrong_part_refusals: WrongPartCounter,
}

/// One seat entry in [`EvalManifest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvalManifestSeat {
    /// Zero-based seat index.
    pub seat: u8,
    /// Guild identifier.
    pub guild: String,
    /// Backend label (`"local"`, `"openrouter"`, or `"file"`).
    pub backend: String,
    /// `"ClubBot"` or model slug / identifier.
    pub model: String,
    /// Whether this seat keeps `"notes"`.
    pub keeps_notes: bool,
}

/// Output written by `league-eval branch` per arm (`<out_dir>/<arm>/outcome.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchArmOutcome {
    /// Source `game_id` (`R5`).
    pub source_game_id: String,
    /// Table identifier.
    pub table_id: u64,
    /// Branched turn / day (`1..=32`).
    pub day: u8,
    /// Focal seat zero-based index.
    pub focal_seat: u8,
    /// Focal seat guild.
    pub focal_guild: String,
    /// Arm evaluated (`"off"`, `"control"`, `"on"`).
    pub arm: EvalArm,
    /// Focal decision record (`raw_reply`, `refusal_code`, `notes_present`, `refused_bundle`, `club_bot_baseline`).
    pub decision: EvalDecisionRecord,
    /// Public sweep receipt produced when replaying this arm from the pre-act checkpoint.
    pub receipt: PublicSweepReceipt,
}

/// Aggregated refusal and wrong-part summary written by `league-eval branch` to `<out_dir>/branch_summary.json` (`B1`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BranchSummary {
    /// Total arm outcomes summarized.
    pub evaluated_arms: usize,
    /// Refusal code counts keyed by focal seat guild (`1.9`, `A1.2`).
    pub refusal_counts_by_seat: BTreeMap<String, RefusalCodeCounts>,
    /// Refusal code counts keyed by evaluated arm (`off`, `control`, `on`).
    pub refusal_counts_by_arm: BTreeMap<String, RefusalCodeCounts>,
    /// Pre-flight wrong-part counter across all `Model` branch decisions (`B1`).
    pub wrong_part_refusals: WrongPartCounter,
}

/// Summarize refusal and wrong-part counts across all `Model` outcomes in `outcomes` (`B1`).
#[must_use]
pub fn summarize_branch_outcomes(outcomes: &[BranchArmOutcome]) -> BranchSummary {
    let mut summary = BranchSummary {
        evaluated_arms: outcomes.len(),
        ..BranchSummary::default()
    };
    for outcome in outcomes {
        let dec = &outcome.decision;
        let seat_counts = summary
            .refusal_counts_by_seat
            .entry(outcome.focal_guild.clone())
            .or_default();
        let arm_counts = summary
            .refusal_counts_by_arm
            .entry(outcome.arm.as_str().to_owned())
            .or_default();
        if let Some(code_str) = dec.refusal_code.as_deref()
            && let Some(code) = ReplyRefusalCode::from_code_str(code_str)
        {
            seat_counts.record(code);
            arm_counts.record(code);
        }
        if dec.refused_bundle.is_some() {
            seat_counts.refused_bundles += 1;
            arm_counts.refused_bundles += 1;
        }
        if dec.seat_kind == "Model" {
            let wrong_part = dec
                .refusal_code
                .as_deref()
                .and_then(ReplyRefusalCode::from_code_str)
                .filter(|&c| {
                    matches!(
                        c,
                        ReplyRefusalCode::TalkReplyHasOrders
                            | ReplyRefusalCode::ActReplyHasMessages
                    )
                });
            if dec.no_reply {
                if let Some(wp_code) = wrong_part {
                    summary
                        .wrong_part_refusals
                        .record_wrong_part_no_reply(wp_code);
                } else {
                    summary.wrong_part_refusals.record_no_reply();
                }
            } else {
                summary
                    .wrong_part_refusals
                    .record_delivered_reply(wrong_part);
            }
        }
    }
    summary
}

/// Extract deterministic wrapper brief version and SHA-256 digest from raw wrapper bytes (`1.7`).
#[must_use]
pub fn extract_wrapper_metadata(
    raw_bytes: &[u8],
    explicit_version: Option<&str>,
) -> (String, String) {
    let digest = sha256_hex(raw_bytes);
    if let Some(v) = explicit_version
        && !v.trim().is_empty()
    {
        return (v.trim().to_owned(), digest);
    }
    let text = String::from_utf8_lossy(raw_bytes);
    let version = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .and_then(|first_line| {
            let trimmed = first_line.trim_start_matches('#').trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.chars().take(64).collect::<String>())
            }
        })
        .unwrap_or_else(|| format!("sha256:{}", &digest[..12]));
    (version, digest)
}
