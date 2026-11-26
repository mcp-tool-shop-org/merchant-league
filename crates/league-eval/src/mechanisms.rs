//! Unblinded D5 mechanism measures and report schema (`docs/decision-scorer.md` §7.2).
//!
//! Computes descriptive-only per-decision and per-arm diagnostics inside `league-eval`'s `join`
//! reporting step only after `scorer_freeze.json` verification succeeds. All text-bearing
//! sidecars (reasoning fields and talk/negotiation whispers) are processed in one pass in memory
//! and immediately dropped; helper functions reading those files return strictly numeric counts
//! or structured numeric action descriptors, never prose strings.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use league_core::{
    AssetId, EvalHost, MarketAction, OfferProposal, OfferSide, Seat, SeatObservation,
};
use league_relay::{
    CallLogEntry, NegotiationInputs, Round, SavedHttpReplyRecord, SavedToolRoundRecord, sha256_hex,
};
use league_scorer::{ExtractedTalkCommitment, TalkExtractionCounts, TalkSourceCounts};
use league_seat_types::{FillKind, FinalScoreboard};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::blind_builder::ArmMapStateFile;
use crate::error::EvalError;
use crate::freeze_gate::{
    JoinedDecisionRow, PerArmTalkCountsSummary, summarize_arm_talk_counts_from_slices,
};
use crate::records::{BranchArmOutcome, EvalDecisionRecord, EvalSetupFile};
use crate::reply::{parse_eval_act_reply_for_communication, parse_eval_talk_reply};
use crate::talk_extract::{extract_talk_commitments_from_messages, resolve_eval_seat_guilds};

/// Minimum `EvalDecisionRecord::tool_calls_used` for a decision to count as having used at least
/// one tool call (`§7.2.1` Measure 1).
pub const TOOL_CALL_USED_MIN_COUNT: u32 = 1;

/// Exact HTTP status required for a non-late `SavedHttpReplyRecord` to contribute HTTP-reply
/// reasoning text (`§7.2.1` Measure 3).
pub const HTTP_REPLY_OK_STATUS: u16 = 200;

/// Minimum `EvalDecisionRecord::refused_bundle_legs` counting as a whole-bundle reveal rejection
/// alongside `refused_bundle.is_some()` (`§7.2.1` Measure 4).
pub const REFUSED_BUNDLE_LEGS_MIN_COUNT: u32 = 1;

/// Maximum turn offset `fill.turn - day` checked for Measure 5(b) fulfillment (`§7.2.1`).
pub const TALK_COMMITMENT_HONOUR_WINDOW_DAYS: u8 = 0;

/// Minimum qualifying bilateral fill share in basis points (`10_000` = 100.00%, full committed
/// `OrderProposal::amount`) required for `honoured` in Measure 5(b) (`§7.2.1`).
pub const TALK_COMMITMENT_MIN_FILL_BPS: u32 = 10_000;

/// Exact integer tolerance in milli-`CRED` per `1_000` commodity units around
/// `committed.price_per_unit_cred` when testing counterparty at-or-better price in Measure 5(b)
/// (`§7.2.1`, `0` = strict at-or-better).
pub const TALK_COMMITMENT_PRICE_TOLERANCE_MILLI_CRED: i64 = 0;

/// Fixed plain statement of what the recorded Evaluation data cannot tell (`§7.2.3` item 5),
/// emitted verbatim into `per_arm_summary.json::data_limits`.
pub const MECHANISM_DATA_LIMITS: [&str; 7] = [
    "**Public DEX order provenance vs bilateral intent (third-party fills):** `MarketAction::PostOffer` / `OfferProposal` (`contracts.rs:258-271`) carries no counterparty field, and Step 0 uniform-price batch clearing (`BatchFill`, `engine.rs:2102-2112`) matches anonymous limit orders on price/tie-break across the entire table. If `focal_seat` posts the promised public order and a third seat crosses it ahead of `counterparty_seat`, `PublicSweepReceipt::fills` records `other_seat` rather than `counterparty_seat`. `join` reports `third_party_fill_commitments` separately and does not guess whether `focal_seat` kept faith or traded away.",
    "**Resting unfilled orders vs counterparty refusal (`posted_unfilled_commitments`):** Because single-day `outcome.json` stores no peer private rationale and `PublicSweepReceipt::fills` logs only executed trades, a resting accepted `PostOffer` that went unfilled cannot distinguish a counterparty that walked away from a toxic/mispriced commitment designed not to clear; reported separately in `posted_unfilled_commitments`.",
    "**Partial bilateral fills (`partial_bilateral_fill_commitments`):** Partial settlement due to counterparty size limits vs `focal_seat` posting less than promised is reported in `partial_bilateral_fill_commitments` and never rounded up to `honoured`.",
    "**Multi-day horizon beyond `day`:** Single-day `branch` runs stop after `receipt.turn == day`, so fulfillment occurring on `day + 1..` does not exist in `outcome.json`.",
    "**Free-text promises outside `talk_rules_v2`:** Vague prose, conditional clauses, or unlisted phrasing fail closed into `focal_talk_counts.ambiguous` (`seen`, `matched`, `ambiguous` reported alongside 5(b)) and are never interpreted post hoc.",
    "**Terminal season `broken_promises` in single-day branches (5(a)):** `SeatScoreEntry::broken_promises` only exists when a full-season `scoreboard.json` is produced (`terminal_broken_promises_count = null` on single-day branches).",
    "**Reasoning tokens when sidecars/logprobs are absent (Measure 3):** If a backend omits `reasoning`/`reasoning_content` or per-token `logprobs`, `completion_tokens` alone cannot separate internal reasoning tokens from final JSON answer tokens.",
];

/// Strictly numeric return type produced by single-pass in-memory reasoning-length readers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReasoningLengthCounts {
    /// Total character length of `SavedToolRoundRecord::reasoning` when `calls.jsonl` and matching
    /// tool-round sidecars parsed cleanly with non-omitted `reasoning` (`None` otherwise).
    pub tool_round_reasoning_chars: Option<u64>,
    /// Total character length of HTTP 200 non-late `reasoning` / `reasoning_content` when
    /// `calls.jsonl` and matching HTTP-reply sidecars parsed cleanly with non-omitted fields.
    pub http_reply_reasoning_chars: Option<u64>,
    /// Deduplicated character sum across tool-round and HTTP-reply streams by reservation ID when
    /// `calls.jsonl` and all matching sidecars parsed cleanly with non-omitted reasoning fields.
    pub total_reasoning_chars: Option<u64>,
    /// Count of unparseable optional `calls.jsonl` lines or reasoning sidecar files encountered.
    pub unparseable_sidecars: u32,
}

/// Purely numeric stack-only per-sidecar reasoning count returned without holding any text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SidecarReasoningCount {
    pub round_index: usize,
    pub attempt: u8,
    pub reservation_digest: [u8; 32],
    pub reasoning_chars: Option<u64>,
}

/// Purely numeric stack-only Measure 5(b) counts returned after dropping all whisper text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct TalkCommitmentEvalCounts {
    extracted_outgoing_commitments: u32,
    applicable_commitments: u32,
    inapplicable_commitments: u32,
    honoured_commitments: u32,
    third_party_fill_commitments: u32,
    posted_unfilled_commitments: u32,
    partial_bilateral_fill_commitments: u32,
}

/// Descriptive per-decision §7.2 mechanism metrics embedded in `JoinedDecisionRow::mechanisms`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DecisionMechanismMetrics {
    /// Measure 1: count of sent draft/preview tool calls (`EvalDecisionRecord::tool_calls_used`).
    pub tool_calls_used: u32,
    /// Measure 1: `tool_calls_used >= TOOL_CALL_USED_MIN_COUNT`.
    pub used_any_tool_call: bool,
    /// Measure 1: `EvalDecisionRecord::no_reply`.
    pub no_reply: bool,

    /// Measure 2: `committed_ev_total_equity - first_draft_ev_total_equity` (`None` if no draft).
    pub revision_gain_ev_total_equity: Option<f64>,
    /// Measure 2: `committed_score_norm - first_draft_score_norm` (`None` if no draft or flat span).
    pub revision_gain_score_norm: Option<f64>,

    /// Measure 3: character count across `SavedToolRoundRecord::reasoning` (`None` when
    /// `relay/seats/` or `relay/calls.jsonl` is absent, reasoning was excluded, or sidecars failed to parse).
    pub tool_round_reasoning_chars: Option<u64>,
    /// Measure 3: character count across HTTP 200 non-late `message.reasoning` / `reasoning_content`
    /// (`None` when absent, excluded, or unparseable; `Some(0)` only for explicit empty fields).
    pub http_reply_reasoning_chars: Option<u64>,
    /// Measure 3: reservation-deduplicated reasoning character sum (`None` when absent, excluded,
    /// or unparseable; `Some(0)` only when sidecars parsed cleanly with explicit empty fields).
    pub total_reasoning_chars: Option<u64>,
    /// Measure 3 companion: `EvalDecisionRecord::completion_tokens`.
    pub completion_tokens: u64,
    /// Measure 3 companion: total `reply_token_ids` length (`None` when empty).
    pub reply_token_ids_len: Option<u64>,

    /// Measure 4: `parse_failure || refusal_code.is_some()`.
    pub has_reply_refusal: bool,
    /// Measure 4: count of refused Pass-A market actions (`EvalDecisionRecord::refused_market_actions`).
    pub refused_market_actions: u32,
    /// Measure 4: `refused_bundle.is_some() || refused_bundle_legs >= REFUSED_BUNDLE_LEGS_MIN_COUNT`.
    pub has_bundle_rejection: bool,

    /// Measure 5(a): pre-act engine pledges where `promisor == focal_seat` (`None` if snapshot absent).
    pub pre_act_pledges_count: Option<u32>,
    /// Measure 5(a): pre-act focal pledges already in `audited_pledges` (`None` if snapshot absent).
    pub pre_act_audited_pledges_count: Option<u32>,
    /// Measure 5(a): broken promises for `promisor == focal_seat` on this day's sweep receipt.
    pub sweep_broken_promises_count: u32,
    /// Measure 5(a): terminal `SeatScoreEntry::broken_promises` when `scoreboard.json` is present.
    pub terminal_broken_promises_count: Option<u32>,

    /// Measure 5(b): `talk_rules_v2` outgoing commitments where `proposer_seat == focal_seat`.
    pub extracted_outgoing_commitments: Option<u32>,
    /// Measure 5(b): subset legally postable from the pre-act checkpoint.
    pub applicable_commitments: Option<u32>,
    /// Measure 5(b): `extracted - applicable`.
    pub inapplicable_commitments: Option<u32>,
    /// Measure 5(b): applicable commitments meeting bilateral fill + price + window criteria.
    pub honoured_commitments: Option<u32>,
    /// Measure 5(b): unhonoured applicable commitments with qualifying third-party fills.
    pub third_party_fill_commitments: Option<u32>,
    /// Measure 5(b): unhonoured applicable commitments with qualifying accepted `PostOffer`.
    pub posted_unfilled_commitments: Option<u32>,
    /// Measure 5(b): unhonoured applicable commitments with partial (`0 < units < 100%`) bilateral fills.
    pub partial_bilateral_fill_commitments: Option<u32>,
    /// Measure 5(b): `honoured / applicable` when `applicable > 0`, else `None` (never `0/0`).
    pub honour_rate: Option<f64>,
    /// Measure 5(b): outgoing focal-to-peer `talk_rules_v2` message classification counts.
    pub focal_talk_counts: Option<TalkExtractionCounts>,
    /// Count of unparseable optional sidecars encountered for this decision (`0` when clean).
    #[serde(default)]
    pub unparseable_sidecars: u32,
}

/// Descriptive per-arm §7.2 mechanism summary embedded in `PerArmSummaryRow::mechanisms`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PerArmMechanismSummary {
    /// Measure 1: total joined decisions in arm.
    pub n_decisions: usize,
    /// Measure 1: decisions with `tool_calls_used >= TOOL_CALL_USED_MIN_COUNT`.
    pub n_with_tool_call: usize,
    /// Measure 1: mean `tool_calls_used` across `n_decisions` (`None` when `0`).
    pub mean_tool_calls_used: Option<f64>,
    /// Measure 1: `n_with_tool_call / n_decisions` (`None` when `0`).
    pub tool_call_rate: Option<f64>,

    /// Measure 2: decisions with a `first_draft_candidate_id`.
    pub n_with_first_draft: usize,
    /// Measure 2: decisions with defined `revision_gain_score_norm`.
    pub n_with_revision_gain_score_norm: usize,
    /// Measure 2: mean `revision_gain_ev_total_equity` over `n_with_first_draft` (`None` when `0`).
    pub mean_revision_gain_ev_total_equity: Option<f64>,
    /// Measure 2: mean `revision_gain_score_norm` over `n_with_revision_gain_score_norm` (`None` when `0`).
    pub mean_revision_gain_score_norm: Option<f64>,

    /// Measure 3: decisions with cleanly parsed non-null `total_reasoning_chars`.
    pub n_with_reasoning_sidecars: usize,
    /// Measure 3: decisions with non-empty `reply_token_ids`.
    pub n_with_reply_token_ids: usize,
    /// Measure 3: mean `tool_round_reasoning_chars` over decisions with non-null counts.
    pub mean_tool_round_reasoning_chars: Option<f64>,
    /// Measure 3: mean `http_reply_reasoning_chars` over decisions with non-null counts.
    pub mean_http_reply_reasoning_chars: Option<f64>,
    /// Measure 3: mean `total_reasoning_chars` over `n_with_reasoning_sidecars`.
    pub mean_total_reasoning_chars: Option<f64>,
    /// Measure 3: mean `completion_tokens` across `n_decisions`.
    pub mean_completion_tokens: Option<f64>,
    /// Measure 3: mean `reply_token_ids_len` over `n_with_reply_token_ids`.
    pub mean_reply_token_ids_len: Option<f64>,

    /// Measure 4: decisions with `parse_failure || refusal_code.is_some()`.
    pub n_reply_refusals: usize,
    /// Measure 4: `n_reply_refusals / n_decisions` (`None` when `0`).
    pub reply_refusal_rate: Option<f64>,
    /// Measure 4: mean `refused_market_actions` across `n_decisions` (`None` when `0`).
    pub mean_refused_market_actions: Option<f64>,
    /// Measure 4: decisions with whole-bundle reveal rejection.
    pub n_bundle_rejections: usize,
    /// Measure 4: `n_bundle_rejections / n_decisions` (`None` when `0`).
    pub bundle_rejection_rate: Option<f64>,

    /// Measure 5(a): decisions with a verified pre-act pledge snapshot.
    pub n_with_pre_act_pledge_snapshot: usize,
    /// Measure 5(a): decisions with a source `scoreboard.json`.
    pub n_with_terminal_scoreboard: usize,
    /// Measure 5(a): summed pre-act focal pledges (`None` when `n_with_pre_act_pledge_snapshot == 0`).
    pub total_pre_act_pledges: Option<u64>,
    /// Measure 5(a): summed audited pre-act focal pledges (`None` when `n_with_pre_act_pledge_snapshot == 0`).
    pub total_pre_act_audited_pledges: Option<u64>,
    /// Measure 5(a): summed sweep `broken_promises` where `promisor == focal_seat`.
    pub total_sweep_broken_promises: u64,
    /// Measure 5(a): summed terminal `broken_promises` (`None` when `n_with_terminal_scoreboard == 0`).
    pub total_terminal_broken_promises: Option<u64>,

    /// Measure 5(b): decisions where outgoing talk commitment inputs were present on disk.
    pub n_with_talk_commitment_inputs: usize,
    /// Measure 5(b): summed outgoing focal commitments (`None` when `n_with_talk_commitment_inputs == 0`).
    pub extracted_outgoing_commitments_total: Option<u64>,
    /// Measure 5(b): summed applicable focal commitments (`None` when `n_with_talk_commitment_inputs == 0`).
    pub applicable_commitments_total: Option<u64>,
    /// Measure 5(b): summed inapplicable focal commitments (`None` when `n_with_talk_commitment_inputs == 0`).
    pub inapplicable_commitments_total: Option<u64>,
    /// Measure 5(b): summed honoured focal commitments (`None` when `n_with_talk_commitment_inputs == 0`).
    pub honoured_commitments_total: Option<u64>,
    /// Measure 5(b): summed unresolved third-party fill commitments.
    pub third_party_fill_commitments_total: Option<u64>,
    /// Measure 5(b): summed unresolved resting unfilled commitments.
    pub posted_unfilled_commitments_total: Option<u64>,
    /// Measure 5(b): summed unresolved partial bilateral fill commitments.
    pub partial_bilateral_fill_commitments_total: Option<u64>,
    /// Measure 5(b): `honoured_total / applicable_total` iff `applicable_total > 0`, else `None`.
    pub honour_rate: Option<f64>,
    /// Measure 5(b): per-arm outgoing focal-to-peer talk classification counts (`None` when `0` states).
    pub focal_talk_counts: Option<PerArmTalkCountsSummary>,
    /// Total unparseable optional sidecars encountered across `n_decisions` in this arm.
    #[serde(default)]
    pub n_unparseable_sidecars: u64,
}

/// Parses JSON without quoting any string content from the payload in errors.
pub(crate) fn parse_json_prose_safe<T: DeserializeOwned>(
    raw: &str,
    path: &Path,
) -> Result<T, EvalError> {
    serde_json::from_str::<T>(raw).map_err(|e| {
        let clean = path.display();
        EvalError::Io(format!(
            "parse {clean}: json {:?} line {} column {}",
            e.classify(),
            e.line(),
            e.column()
        ))
    })
}

/// Parses JSON bytes without quoting any content from the payload in errors.
fn parse_json_slice_prose_safe<T: DeserializeOwned>(
    bytes: &[u8],
    path: &Path,
) -> Result<T, EvalError> {
    serde_json::from_slice::<T>(bytes).map_err(|e| {
        let clean = path.display();
        EvalError::Io(format!(
            "parse {clean}: json {:?} line {} column {}",
            e.classify(),
            e.line(),
            e.column()
        ))
    })
}

fn sha256_bytes32(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = [0_u8; 32];
    out.copy_from_slice(&digest);
    out
}

enum CallsFilterState {
    Missing,
    Corrupt(u32),
    Ready(BTreeSet<[u8; 32]>),
}

pub(crate) enum SidecarReadOutcome {
    Counted(SidecarReasoningCount),
    FilteredOut,
    Unparseable,
}

fn collect_act_reservation_digests(
    calls_path: &Path,
    day_str: &str,
    guild: &str,
) -> CallsFilterState {
    if !calls_path.is_file() {
        return CallsFilterState::Missing;
    }
    let Ok(raw) = fs::read_to_string(calls_path) else {
        return CallsFilterState::Corrupt(1);
    };
    let mut digests = BTreeSet::new();
    let mut bad_lines: u32 = 0;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(entry) = parse_json_prose_safe::<CallLogEntry>(trimmed, calls_path) else {
            bad_lines = bad_lines.saturating_add(1);
            continue;
        };
        if entry.day == day_str
            && entry.round == Round::Act
            && entry.guild.eq_ignore_ascii_case(guild)
            && let Some(resv_id) = entry.reservation_id
        {
            digests.insert(sha256_bytes32(resv_id.as_bytes()));
        }
    }
    if bad_lines > 0 {
        CallsFilterState::Corrupt(bad_lines)
    } else {
        CallsFilterState::Ready(digests)
    }
}

/// Single-pass in-memory reader for one `SavedToolRoundRecord` JSON file.
///
/// Holds `SavedToolRoundRecord::reasoning` only inside this stack frame, drops it upon counting,
/// and returns strictly fixed-size numbers and digests (`FilteredOut` / `Unparseable` otherwise).
pub(crate) fn read_tool_round_reasoning_chars(
    path: &Path,
    day_str: &str,
    guild: &str,
    resv_filter: &BTreeSet<[u8; 32]>,
) -> SidecarReadOutcome {
    let Ok(bytes) = fs::read(path) else {
        return SidecarReadOutcome::Unparseable;
    };
    let Ok(rec) = parse_json_slice_prose_safe::<SavedToolRoundRecord>(&bytes, path) else {
        return SidecarReadOutcome::Unparseable;
    };
    drop(bytes);
    let resv_digest = sha256_bytes32(rec.reservation_id.as_bytes());
    let matches_round =
        rec.day == day_str && rec.round == Round::Act && rec.guild.eq_ignore_ascii_case(guild);
    if !matches_round || !resv_filter.contains(&resv_digest) {
        return SidecarReadOutcome::FilteredOut;
    }
    let reasoning_chars = rec.reasoning.as_deref().map(|s| s.chars().count() as u64);
    let round_index = rec.round_index;
    let attempt = rec.attempt;
    drop(rec);
    SidecarReadOutcome::Counted(SidecarReasoningCount {
        round_index,
        attempt,
        reservation_digest: resv_digest,
        reasoning_chars,
    })
}

/// Single-pass in-memory reader for one `SavedHttpReplyRecord` JSON file.
///
/// Parses `body` only inside this stack frame, extracts `choices[0].message.reasoning` or
/// fallback `reasoning_content`, drops all strings/JSON values immediately, and returns strictly
/// fixed-size numbers and digests (`FilteredOut` on non-200/late/error JSON, `Unparseable` on
/// invalid JSON envelope or non-JSON 200 body).
pub(crate) fn read_http_reply_reasoning_chars(
    path: &Path,
    day_str: &str,
    guild: &str,
    resv_filter: &BTreeSet<[u8; 32]>,
) -> SidecarReadOutcome {
    let Ok(bytes) = fs::read(path) else {
        return SidecarReadOutcome::Unparseable;
    };
    let Ok(rec) = parse_json_slice_prose_safe::<SavedHttpReplyRecord>(&bytes, path) else {
        return SidecarReadOutcome::Unparseable;
    };
    drop(bytes);
    let resv_digest = sha256_bytes32(rec.reservation_id.as_bytes());
    let matches_round =
        rec.day == day_str && rec.round == Round::Act && rec.guild.eq_ignore_ascii_case(guild);
    if !matches_round
        || !resv_filter.contains(&resv_digest)
        || rec.status != HTTP_REPLY_OK_STATUS
        || rec.is_late
    {
        return SidecarReadOutcome::FilteredOut;
    }
    let Ok(body_val) = parse_json_prose_safe::<serde_json::Value>(&rec.body, path) else {
        return SidecarReadOutcome::Unparseable;
    };
    if !body_val.get("error").is_none_or(serde_json::Value::is_null) {
        return SidecarReadOutcome::FilteredOut;
    }
    let msg = body_val
        .get("choices")
        .and_then(serde_json::Value::as_array)
        .and_then(|arr| arr.first())
        .and_then(|c| c.get("message"));
    let reasoning_chars = msg.and_then(|m| {
        m.get("reasoning")
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                m.get("reasoning_content")
                    .and_then(serde_json::Value::as_str)
            })
            .map(|s| s.chars().count() as u64)
    });
    let round_index = rec.round_index;
    let attempt = rec.attempt;
    drop(body_val);
    drop(rec);
    SidecarReadOutcome::Counted(SidecarReasoningCount {
        round_index,
        attempt,
        reservation_digest: resv_digest,
        reasoning_chars,
    })
}

/// Computes Measure 3 character counts in a single directory pass over `<arm_dir>/relay/seats/`.
///
/// Returns strictly numeric counts (`ReasoningLengthCounts`) with zero retained prose text.
pub fn measure_reasoning_length_in_memory(
    arm_dir: &Path,
    day: u8,
    guild: &str,
) -> Result<ReasoningLengthCounts, EvalError> {
    let seats_dir = arm_dir.join("relay").join("seats");
    if !seats_dir.is_dir() {
        return Ok(ReasoningLengthCounts::default());
    }
    let day_str = format!("{day:02}");
    let calls_path = arm_dir.join("relay").join("calls.jsonl");
    let resv_filter = match collect_act_reservation_digests(&calls_path, &day_str, guild) {
        CallsFilterState::Missing => return Ok(ReasoningLengthCounts::default()),
        CallsFilterState::Corrupt(bad_lines) => {
            return Ok(ReasoningLengthCounts {
                unparseable_sidecars: bad_lines,
                ..ReasoningLengthCounts::default()
            });
        }
        CallsFilterState::Ready(digests) => digests,
    };

    let tool_prefix = format!("{guild}.day-{day_str}-act.tool-round-");
    let http_prefix = format!("{guild}.day-{day_str}-act.http-reply-round-");

    let mut files = Vec::new();
    let entries = fs::read_dir(&seats_dir)
        .map_err(|e| EvalError::Io(format!("read {}: {e}", seats_dir.display())))?;
    for entry_res in entries {
        let entry = entry_res.map_err(|e| EvalError::Io(format!("read seats entry: {e}")))?;
        let ft = entry
            .file_type()
            .map_err(|e| EvalError::Io(format!("inspect seats entry: {e}")))?;
        if ft.is_file() {
            files.push(entry.path());
        }
    }
    files.sort();

    let mut tool_seen: BTreeMap<(usize, u8, [u8; 32]), Option<u64>> = BTreeMap::new();
    let mut http_seen: BTreeMap<(usize, u8, [u8; 32]), Option<u64>> = BTreeMap::new();
    let mut tool_unparseable = false;
    let mut http_unparseable = false;
    let mut unparseable_sidecars: u32 = 0;

    for path in files {
        let Some(fname) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if !fname.ends_with(".json") {
            continue;
        }
        if fname.starts_with(&tool_prefix) {
            match read_tool_round_reasoning_chars(&path, &day_str, guild, &resv_filter) {
                SidecarReadOutcome::Counted(c) => {
                    tool_seen.insert(
                        (c.round_index, c.attempt, c.reservation_digest),
                        c.reasoning_chars,
                    );
                }
                SidecarReadOutcome::FilteredOut => {}
                SidecarReadOutcome::Unparseable => {
                    tool_unparseable = true;
                    unparseable_sidecars = unparseable_sidecars.saturating_add(1);
                }
            }
        } else if fname.starts_with(&http_prefix) {
            match read_http_reply_reasoning_chars(&path, &day_str, guild, &resv_filter) {
                SidecarReadOutcome::Counted(c) => {
                    http_seen.insert(
                        (c.round_index, c.attempt, c.reservation_digest),
                        c.reasoning_chars,
                    );
                }
                SidecarReadOutcome::FilteredOut => {}
                SidecarReadOutcome::Unparseable => {
                    http_unparseable = true;
                    unparseable_sidecars = unparseable_sidecars.saturating_add(1);
                }
            }
        }
    }

    let tool_has_field = tool_seen.values().any(Option::is_some);
    let http_has_field = http_seen.values().any(Option::is_some);

    let tool_round_reasoning_chars = (!tool_unparseable && tool_has_field).then(|| {
        tool_seen
            .values()
            .flatten()
            .copied()
            .fold(0_u64, u64::saturating_add)
    });
    let http_reply_reasoning_chars = (!http_unparseable && http_has_field).then(|| {
        http_seen
            .values()
            .flatten()
            .copied()
            .fold(0_u64, u64::saturating_add)
    });

    let mut by_resv: BTreeMap<[u8; 32], (Option<u64>, Option<u64>)> = BTreeMap::new();
    for ((_, _, resv), chars_opt) in tool_seen {
        if let Some(c) = chars_opt {
            let slot = by_resv.entry(resv).or_insert((None, None));
            slot.0 = Some(slot.0.unwrap_or(0).saturating_add(c));
        }
    }
    for ((_, _, resv), chars_opt) in http_seen {
        if let Some(c) = chars_opt {
            let slot = by_resv.entry(resv).or_insert((None, None));
            slot.1 = Some(slot.1.unwrap_or(0).saturating_add(c));
        }
    }
    let total_reasoning_chars = (!tool_unparseable && !http_unparseable && !by_resv.is_empty())
        .then(|| {
            by_resv
                .into_values()
                .map(|(tool_chars, http_chars)| tool_chars.or(http_chars).unwrap_or(0))
                .fold(0_u64, u64::saturating_add)
        });

    Ok(ReasoningLengthCounts {
        tool_round_reasoning_chars,
        http_reply_reasoning_chars,
        total_reasoning_chars,
        unparseable_sidecars,
    })
}

/// Scans explicit `--source-game` directories (recursively finding `setup.json`) keyed by `game_id`.
pub(crate) fn index_source_game_dirs(
    dirs: &[PathBuf],
) -> Result<BTreeMap<String, PathBuf>, EvalError> {
    let mut index = BTreeMap::new();
    for dir in dirs {
        scan_source_game_dir(dir, &mut index)?;
    }
    Ok(index)
}

fn scan_source_game_dir(dir: &Path, out: &mut BTreeMap<String, PathBuf>) -> Result<(), EvalError> {
    let setup_path = dir.join("setup.json");
    if setup_path.is_file() {
        let raw = fs::read_to_string(&setup_path)
            .map_err(|e| EvalError::Io(format!("read {}: {e}", setup_path.display())))?;
        let setup: EvalSetupFile = parse_json_prose_safe(&raw, &setup_path)?;
        if let Some(prev) = out.insert(setup.game_id.clone(), dir.to_path_buf())
            && prev != dir
        {
            return Err(EvalError::DuplicateStateId(format!(
                "duplicate source game_id `{}` across --source-game directories",
                setup.game_id
            )));
        }
        return Ok(());
    }
    if !dir.is_dir() {
        return Err(EvalError::MissingSourceSetup(format!(
            "missing --source-game directory {}",
            dir.display()
        )));
    }
    let mut subdirs = Vec::new();
    let entries =
        fs::read_dir(dir).map_err(|e| EvalError::Io(format!("read {}: {e}", dir.display())))?;
    for entry_res in entries {
        let entry = entry_res.map_err(|e| EvalError::Io(format!("read dir entry: {e}")))?;
        let ft = entry
            .file_type()
            .map_err(|e| EvalError::Io(format!("inspect entry: {e}")))?;
        if ft.is_dir() {
            subdirs.push(entry.path());
        }
    }
    subdirs.sort();
    for sub in subdirs {
        scan_source_game_dir(&sub, out)?;
    }
    Ok(())
}

fn verify_and_fork_pre_act_host(
    source_dir: &Path,
    arm_map: &ArmMapStateFile,
) -> Result<Option<(PathBuf, PathBuf, EvalHost, SeatObservation)>, EvalError> {
    let day_str = format!("{:02}", arm_map.day);
    let save_path = source_dir
        .join("checkpoints")
        .join(format!("day-{day_str}-pre-act.save.json"));
    let seal_path = source_dir
        .join("checkpoints")
        .join(format!("day-{day_str}-pre-act.seal"));
    if !save_path.is_file() || !seal_path.is_file() {
        return Ok(None);
    }
    let save_bytes = fs::read(&save_path)
        .map_err(|e| EvalError::Io(format!("read {}: {e}", save_path.display())))?;
    let seal_bytes = fs::read(&seal_path)
        .map_err(|e| EvalError::Io(format!("read {}: {e}", seal_path.display())))?;
    if sha256_hex(&save_bytes) != arm_map.pre_act_save_sha256
        || sha256_hex(&seal_bytes) != arm_map.pre_act_seal_sha256
    {
        return Err(EvalError::FreezeManifestMismatch(format!(
            "source pre-act checkpoint SHA-256 for `{}` disagrees with arm_map",
            arm_map.state_id
        )));
    }
    let host = EvalHost::fork_from_checkpoint(&save_path, &seal_path)
        .map_err(|e| EvalError::SchemaValidation(format!("fork pre-act checkpoint: {e}")))?;
    let obs = host
        .observe_seat(Seat(arm_map.focal_seat))
        .map_err(|e| EvalError::SchemaValidation(format!("observe focal seat: {e}")))?;
    Ok(Some((save_path, seal_path, host, obs)))
}

fn asset_phrase_token(good: AssetId) -> Option<String> {
    match good {
        AssetId::Raw(idx) => Some(format!("raw {idx}")),
        AssetId::Refined(idx) => Some(format!("refined {idx}")),
        AssetId::Cred => None,
    }
}

/// Classifies one outgoing message `(focal_seat -> peer_seat)` under frozen `talk_rules_v2`
/// without holding raw prose beyond this call.
fn classify_outgoing_whisper_matched(
    focal_seat: u8,
    peer_seat: u8,
    body: &str,
    seat_guilds: &[String],
    obs: &SeatObservation,
) -> Result<bool, EvalError> {
    let single = [(focal_seat, peer_seat, body.to_owned())];
    if !extract_talk_commitments_from_messages(peer_seat, seat_guilds, &single, obs)?.is_empty() {
        return Ok(true);
    }
    let seat_count = obs.spec.seat_count;
    for idx in 0..seat_count {
        for candidate in [format!("raw {idx}"), format!("refined {idx}")] {
            let probe = [
                (focal_seat, peer_seat, body.to_owned()),
                (
                    focal_seat,
                    peer_seat,
                    format!("i will sell 1 {candidate} at 1."),
                ),
            ];
            if !extract_talk_commitments_from_messages(peer_seat, seat_guilds, &probe, obs)?
                .is_empty()
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Checks whether `source_msgs` kept its pair outcome non-retracted and non-contradicted on `good`.
fn probe_source_keeps_asset_unretracted(
    focal_seat: u8,
    peer_seat: u8,
    good: AssetId,
    source_msgs: &[(u8, u8, String)],
    seat_guilds: &[String],
    obs: &SeatObservation,
) -> Result<bool, EvalError> {
    let Some(token) = asset_phrase_token(good) else {
        return Ok(false);
    };
    let mut seq = Vec::with_capacity(source_msgs.len() + 1);
    seq.push((
        focal_seat,
        peer_seat,
        format!("i will sell 1 {token} at 1."),
    ));
    seq.extend(source_msgs.iter().cloned());
    let out = extract_talk_commitments_from_messages(peer_seat, seat_guilds, &seq, obs)?;
    Ok(out.iter().any(|c| match c.offered_action {
        MarketAction::PostOffer { ref order, .. } => {
            order.good == good && order.side == OfferSide::Sell && order.amount == 1_000
        }
        _ => false,
    }))
}

fn extract_outgoing_for_pair(
    focal_seat: u8,
    peer_seat: u8,
    neg_msgs: &[(u8, u8, String)],
    talk_msgs: &[(u8, u8, String)],
    seat_guilds: &[String],
    obs: &SeatObservation,
) -> Result<
    (
        Vec<ExtractedTalkCommitment>,
        TalkSourceCounts,
        TalkSourceCounts,
    ),
    EvalError,
> {
    let mut neg_counts = TalkSourceCounts::default();
    let mut neg_ambiguous = false;
    for (_, _, body) in neg_msgs {
        neg_counts.seen = neg_counts.seen.saturating_add(1);
        if classify_outgoing_whisper_matched(focal_seat, peer_seat, body, seat_guilds, obs)? {
            neg_counts.matched = neg_counts.matched.saturating_add(1);
        } else {
            neg_counts.ambiguous = neg_counts.ambiguous.saturating_add(1);
            neg_ambiguous = true;
        }
    }

    let mut talk_counts = TalkSourceCounts::default();
    let mut talk_ambiguous = false;
    for (_, _, body) in talk_msgs {
        talk_counts.seen = talk_counts.seen.saturating_add(1);
        if classify_outgoing_whisper_matched(focal_seat, peer_seat, body, seat_guilds, obs)? {
            talk_counts.matched = talk_counts.matched.saturating_add(1);
        } else {
            talk_counts.ambiguous = talk_counts.ambiguous.saturating_add(1);
            talk_ambiguous = true;
        }
    }

    if neg_ambiguous || talk_ambiguous {
        return Ok((Vec::new(), neg_counts, talk_counts));
    }

    let neg_active = extract_talk_commitments_from_messages(peer_seat, seat_guilds, neg_msgs, obs)?;
    let talk_active =
        extract_talk_commitments_from_messages(peer_seat, seat_guilds, talk_msgs, obs)?;

    let mut by_good_neg: BTreeMap<AssetId, ExtractedTalkCommitment> = BTreeMap::new();
    for c in neg_active {
        if let MarketAction::PostOffer { ref order, .. } = c.offered_action {
            by_good_neg.insert(order.good, c);
        }
    }
    let mut by_good_talk: BTreeMap<AssetId, ExtractedTalkCommitment> = BTreeMap::new();
    for c in talk_active {
        if let MarketAction::PostOffer { ref order, .. } = c.offered_action {
            by_good_talk.insert(order.good, c);
        }
    }

    let mut all_goods = BTreeSet::new();
    all_goods.extend(by_good_neg.keys().copied());
    all_goods.extend(by_good_talk.keys().copied());

    let mut merged = Vec::new();
    for good in all_goods {
        match (by_good_neg.get(&good), by_good_talk.get(&good)) {
            (Some(a), Some(b)) => {
                if a == b {
                    merged.push(a.clone());
                }
            }
            (Some(a), None) => {
                if probe_source_keeps_asset_unretracted(
                    focal_seat,
                    peer_seat,
                    good,
                    talk_msgs,
                    seat_guilds,
                    obs,
                )? {
                    merged.push(a.clone());
                }
            }
            (None, Some(b)) => {
                if probe_source_keeps_asset_unretracted(
                    focal_seat,
                    peer_seat,
                    good,
                    neg_msgs,
                    seat_guilds,
                    obs,
                )? {
                    merged.push(b.clone());
                }
            }
            (None, None) => {}
        }
    }
    Ok((merged, neg_counts, talk_counts))
}

struct OutgoingCommitmentScan {
    extracted: Option<(Vec<ExtractedTalkCommitment>, TalkExtractionCounts)>,
    unparseable_files: u32,
}

/// Extracts outgoing focal-to-peer talk commitments in memory and drops all raw prose immediately.
fn extract_outgoing_commitments_in_memory(
    source_dir: &Path,
    day: u8,
    focal_seat: u8,
    obs: &SeatObservation,
) -> Result<OutgoingCommitmentScan, EvalError> {
    let seat_count = obs.spec.seat_count;
    let seat_guilds = resolve_eval_seat_guilds(source_dir, seat_count)?;
    let Some(focal_guild) = seat_guilds.get(usize::from(focal_seat)) else {
        return Err(EvalError::UnknownFocalSeat(format!(
            "focal_seat {focal_seat} out of range 0..{seat_count}"
        )));
    };
    let day_str = format!("{day:02}");

    let mut any_neg_file = false;
    let mut unparseable_files: u32 = 0;
    let mut neg_by_peer: BTreeMap<u8, Vec<(u8, u8, String)>> = BTreeMap::new();
    for (peer_idx, peer_guild) in seat_guilds.iter().enumerate() {
        let Ok(peer_seat) = u8::try_from(peer_idx) else {
            continue;
        };
        if peer_seat == focal_seat {
            continue;
        }
        let neg_path = source_dir
            .join("to-seat")
            .join(format!("day-{day_str}-act-{peer_guild}.negotiation.json"));
        if !neg_path.is_file() {
            continue;
        }
        any_neg_file = true;
        let Ok(raw) = fs::read_to_string(&neg_path) else {
            unparseable_files = unparseable_files.saturating_add(1);
            continue;
        };
        let Ok(neg) = parse_json_prose_safe::<NegotiationInputs>(&raw, &neg_path) else {
            drop(raw);
            unparseable_files = unparseable_files.saturating_add(1);
            continue;
        };
        drop(raw);
        for w in neg.whispers {
            if w.from_guild == *focal_guild {
                neg_by_peer
                    .entry(peer_seat)
                    .or_default()
                    .push((focal_seat, peer_seat, w.body));
            }
        }
    }

    let guild_to_seat: BTreeMap<&str, u8> = seat_guilds
        .iter()
        .enumerate()
        .filter_map(|(i, g)| u8::try_from(i).ok().map(|s| (g.as_str(), s)))
        .collect();

    let mut talk_by_peer: BTreeMap<u8, Vec<(u8, u8, String)>> = BTreeMap::new();
    let talk_dec_path = source_dir
        .join("decisions")
        .join(format!("day-{day_str}-talk-{focal_guild}.json"));
    let talk_file_exists = talk_dec_path.is_file();
    if talk_file_exists {
        let parsed_rec = fs::read_to_string(&talk_dec_path)
            .ok()
            .and_then(|raw| parse_json_prose_safe::<EvalDecisionRecord>(&raw, &talk_dec_path).ok());
        match parsed_rec {
            Some(rec) => {
                if !rec.no_reply
                    && let Some(raw_reply) = rec.raw_reply.as_deref()
                {
                    let parsed = parse_eval_talk_reply(
                        raw_reply,
                        focal_seat,
                        focal_guild,
                        &seat_guilds,
                        rec.keeps_notes,
                    );
                    if parsed.refusal_code.is_none() {
                        for (recipient_guild, w) in parsed.accepted_whispers {
                            if let Some(&peer_seat) = guild_to_seat.get(recipient_guild.as_str())
                                && peer_seat != focal_seat
                            {
                                talk_by_peer
                                    .entry(peer_seat)
                                    .or_default()
                                    .push((focal_seat, peer_seat, w.body));
                            }
                        }
                    }
                }
            }
            None => {
                unparseable_files = unparseable_files.saturating_add(1);
            }
        }
    }

    if unparseable_files > 0 || (!any_neg_file && !talk_file_exists) {
        drop(neg_by_peer);
        drop(talk_by_peer);
        return Ok(OutgoingCommitmentScan {
            extracted: None,
            unparseable_files,
        });
    }

    let mut all_commitments = Vec::new();
    let mut neg_total = TalkSourceCounts::default();
    let mut talk_total = TalkSourceCounts::default();
    for peer_idx in 0..seat_count {
        if peer_idx == focal_seat {
            continue;
        }
        let empty = Vec::new();
        let neg_msgs = neg_by_peer.get(&peer_idx).unwrap_or(&empty);
        let talk_msgs = talk_by_peer.get(&peer_idx).unwrap_or(&empty);
        let (pair_commitments, n_cnt, t_cnt) = extract_outgoing_for_pair(
            focal_seat,
            peer_idx,
            neg_msgs,
            talk_msgs,
            &seat_guilds,
            obs,
        )?;
        neg_total.seen = neg_total.seen.saturating_add(n_cnt.seen);
        neg_total.matched = neg_total.matched.saturating_add(n_cnt.matched);
        neg_total.ambiguous = neg_total.ambiguous.saturating_add(n_cnt.ambiguous);
        talk_total.seen = talk_total.seen.saturating_add(t_cnt.seen);
        talk_total.matched = talk_total.matched.saturating_add(t_cnt.matched);
        talk_total.ambiguous = talk_total.ambiguous.saturating_add(t_cnt.ambiguous);
        all_commitments.extend(pair_commitments);
    }

    let counts = TalkExtractionCounts::from_sources(neg_total, talk_total)
        .map_err(|e| EvalError::SchemaValidation(format!("outgoing talk counts: {e}")))?;
    Ok(OutgoingCommitmentScan {
        extracted: Some((all_commitments, counts)),
        unparseable_files: 0,
    })
}

fn price_at_or_better_for_counterparty(order: &OfferProposal, unit_price_cred: i64) -> bool {
    let promised = order.price_per_unit_cred;
    match order.side {
        OfferSide::Sell => {
            unit_price_cred <= promised.saturating_add(TALK_COMMITMENT_PRICE_TOLERANCE_MILLI_CRED)
        }
        OfferSide::Buy => {
            unit_price_cred >= promised.saturating_sub(TALK_COMMITMENT_PRICE_TOLERANCE_MILLI_CRED)
        }
    }
}

fn accepted_pass_a_offers(decision: &EvalDecisionRecord) -> Vec<OfferProposal> {
    if decision.no_reply || decision.refusal_code.is_some() {
        return Vec::new();
    }
    let Some(raw_reply) = decision.raw_reply.as_deref() else {
        return Vec::new();
    };
    let parsed = parse_eval_act_reply_for_communication(
        decision.communication,
        raw_reply,
        decision.seat,
        decision.keeps_notes,
    );
    if parsed.refusal_code.is_some() {
        return Vec::new();
    }
    let refused_indices: BTreeSet<usize> = decision
        .apply_report
        .as_ref()
        .map(|r| {
            r.refused_market_actions
                .iter()
                .map(|(idx, _)| *idx)
                .collect()
        })
        .unwrap_or_default();

    let mut accepted = Vec::new();
    for (idx, action) in parsed.submission.market_actions.into_iter().enumerate() {
        if refused_indices.contains(&idx) {
            continue;
        }
        if let MarketAction::PostOffer { seat, order } = action
            && seat == decision.seat
        {
            accepted.push(order);
        }
    }
    accepted
}

#[allow(clippy::too_many_arguments)]
fn evaluate_talk_commitments(
    commitments: &[ExtractedTalkCommitment],
    save_path: &Path,
    seal_path: &Path,
    day: u8,
    focal_seat: u8,
    decision: &EvalDecisionRecord,
    outcome: &BranchArmOutcome,
) -> Result<TalkCommitmentEvalCounts, EvalError> {
    let mut counts = TalkCommitmentEvalCounts {
        extracted_outgoing_commitments: commitments.len() as u32,
        ..TalkCommitmentEvalCounts::default()
    };
    let max_turn = day.saturating_add(TALK_COMMITMENT_HONOUR_WINDOW_DAYS);
    let accepted_offers = accepted_pass_a_offers(decision);

    for commitment in commitments {
        let MarketAction::PostOffer { ref order, .. } = commitment.offered_action else {
            continue;
        };
        let mut fork = EvalHost::fork_from_checkpoint(save_path, seal_path)
            .map_err(|e| EvalError::SchemaValidation(format!("fork applicability check: {e}")))?;
        let is_legal = fork
            .apply_market_action(commitment.offered_action.clone())
            .is_ok();
        if !is_legal {
            counts.inapplicable_commitments = counts.inapplicable_commitments.saturating_add(1);
            continue;
        }
        counts.applicable_commitments = counts.applicable_commitments.saturating_add(1);

        let mut bilateral_units: i128 = 0;
        let mut has_third_party_fill = false;

        for fill in &outcome.receipt.fills {
            if fill.turn < day || fill.turn > max_turn {
                continue;
            }
            if !matches!(
                fill.kind,
                FillKind::BatchFill | FillKind::AtomicSwap | FillKind::ContractDelivery
            ) {
                continue;
            }
            if fill.good != Some(order.good) {
                continue;
            }
            let Some(fill_price) = fill.unit_price_cred else {
                continue;
            };
            if !price_at_or_better_for_counterparty(order, fill_price) {
                continue;
            }
            let (is_bilateral, is_third_party) = match order.side {
                OfferSide::Sell => {
                    let bilateral =
                        fill.from == Some(focal_seat) && fill.to == commitment.counterparty_seat;
                    let third = fill.from == Some(focal_seat)
                        && fill.to != commitment.counterparty_seat
                        && fill.to != focal_seat;
                    (bilateral, third)
                }
                OfferSide::Buy => {
                    let bilateral =
                        fill.from == Some(commitment.counterparty_seat) && fill.to == focal_seat;
                    let third = fill.from.is_some_and(|seller| {
                        seller != commitment.counterparty_seat && seller != focal_seat
                    }) && fill.to == focal_seat;
                    (bilateral, third)
                }
            };
            if is_bilateral {
                bilateral_units = bilateral_units.saturating_add(i128::from(fill.units));
            } else if is_third_party {
                has_third_party_fill = true;
            }
        }

        let needed_scaled =
            i128::from(order.amount).saturating_mul(i128::from(TALK_COMMITMENT_MIN_FILL_BPS));
        let fulfilled_scaled = bilateral_units.saturating_mul(10_000);
        if fulfilled_scaled >= needed_scaled {
            counts.honoured_commitments = counts.honoured_commitments.saturating_add(1);
        } else if bilateral_units > 0 {
            counts.partial_bilateral_fill_commitments =
                counts.partial_bilateral_fill_commitments.saturating_add(1);
        } else if has_third_party_fill {
            counts.third_party_fill_commitments =
                counts.third_party_fill_commitments.saturating_add(1);
        } else {
            let posted_amount: i128 = accepted_offers
                .iter()
                .filter(|posted| {
                    posted.side == order.side
                        && posted.good == order.good
                        && price_at_or_better_for_counterparty(order, posted.price_per_unit_cred)
                })
                .map(|posted| i128::from(posted.amount))
                .sum();
            if posted_amount >= i128::from(order.amount) {
                counts.posted_unfilled_commitments =
                    counts.posted_unfilled_commitments.saturating_add(1);
            }
        }
    }

    Ok(counts)
}

fn read_terminal_broken_promises(source_dir: &Path, focal_seat: u8) -> (Option<u32>, u32) {
    let sb_path = source_dir.join("scoreboard.json");
    if !sb_path.is_file() {
        return (None, 0);
    }
    let Ok(raw) = fs::read_to_string(&sb_path) else {
        return (None, 1);
    };
    let Ok(sb) = parse_json_prose_safe::<FinalScoreboard>(&raw, &sb_path) else {
        return (None, 1);
    };
    let count = sb
        .standings
        .into_iter()
        .find(|entry| entry.seat == focal_seat)
        .map(|entry| entry.broken_promises);
    (count, 0)
}

/// Computes `DecisionMechanismMetrics` for one joined `(state_id, arm, replicate)` decision.
#[allow(clippy::too_many_arguments)]
pub(crate) fn compute_decision_mechanisms(
    arm_map: &ArmMapStateFile,
    decision_path: &Path,
    outcome_path: &Path,
    committed_ev_total_equity: f64,
    committed_score_norm: Option<f64>,
    first_draft_ev_total_equity: Option<f64>,
    first_draft_score_norm: Option<f64>,
    source_game_dir: Option<&Path>,
) -> Result<DecisionMechanismMetrics, EvalError> {
    let dec_raw = fs::read_to_string(decision_path)
        .map_err(|e| EvalError::Io(format!("read {}: {e}", decision_path.display())))?;
    let decision: EvalDecisionRecord = parse_json_prose_safe(&dec_raw, decision_path)?;
    drop(dec_raw);

    let out_raw = fs::read_to_string(outcome_path)
        .map_err(|e| EvalError::Io(format!("read {}: {e}", outcome_path.display())))?;
    let outcome: BranchArmOutcome = parse_json_prose_safe(&out_raw, outcome_path)?;
    drop(out_raw);

    let arm_dir = decision_path.parent().ok_or_else(|| {
        EvalError::Io(format!(
            "decision_path lacks parent dir: {}",
            decision_path.display()
        ))
    })?;

    // Measure 1: tool-call rate
    let tool_calls_used = decision.tool_calls_used;
    let used_any_tool_call = tool_calls_used >= TOOL_CALL_USED_MIN_COUNT;
    let no_reply = decision.no_reply;

    // Measure 2: draft-to-commit revision gain
    let revision_gain_ev_total_equity =
        first_draft_ev_total_equity.map(|draft_ev| committed_ev_total_equity - draft_ev);
    let revision_gain_score_norm = committed_score_norm
        .zip(first_draft_score_norm)
        .map(|(comm, draft)| comm - draft);

    // Measure 3: single-pass in-memory reasoning length + field-only token companions
    let reasoning_counts =
        measure_reasoning_length_in_memory(arm_dir, arm_map.day, &decision.guild)?;
    let completion_tokens = decision.completion_tokens;
    let reply_token_ids_len = if decision.reply_token_ids.is_empty() {
        None
    } else {
        Some(
            decision
                .reply_token_ids
                .iter()
                .map(|r| r.len() as u64)
                .fold(0_u64, u64::saturating_add),
        )
    };

    // Measure 4: illegal-order rate
    let has_reply_refusal = decision.parse_failure || decision.refusal_code.is_some();
    let refused_market_actions = decision.refused_market_actions;
    let has_bundle_rejection = decision.refused_bundle.is_some()
        || decision.refused_bundle_legs >= REFUSED_BUNDLE_LEGS_MIN_COUNT;

    // Measure 5(a): engine-recorded pledges + sweep/terminal broken_promises
    let sweep_broken_promises_count = outcome
        .receipt
        .broken_promises
        .iter()
        .filter(|bp| bp.promisor == arm_map.focal_seat)
        .count() as u32;

    let mut pre_act_pledges_count = None;
    let mut pre_act_audited_pledges_count = None;
    let mut terminal_broken_promises_count = None;

    // Measure 5(b): text-parsed outgoing talk commitments
    let mut extracted_outgoing_commitments = None;
    let mut applicable_commitments = None;
    let mut inapplicable_commitments = None;
    let mut honoured_commitments = None;
    let mut third_party_fill_commitments = None;
    let mut posted_unfilled_commitments = None;
    let mut partial_bilateral_fill_commitments = None;
    let mut honour_rate = None;
    let mut focal_talk_counts = None;

    let mut unparseable_sidecars = reasoning_counts.unparseable_sidecars;

    if let Some(src_dir) = source_game_dir {
        let (term_broken, sb_bad) = read_terminal_broken_promises(src_dir, arm_map.focal_seat);
        terminal_broken_promises_count = term_broken;
        unparseable_sidecars = unparseable_sidecars.saturating_add(sb_bad);
        if let Some((save_path, seal_path, _host, obs)) =
            verify_and_fork_pre_act_host(src_dir, arm_map)?
        {
            let focal_pledges: u32 = obs
                .pledges
                .values()
                .filter(|p| p.promisor == arm_map.focal_seat)
                .count() as u32;
            let focal_audited: u32 = obs
                .pledges
                .values()
                .filter(|p| {
                    p.promisor == arm_map.focal_seat && obs.audited_pledges.contains(&p.pledge_id)
                })
                .count() as u32;
            pre_act_pledges_count = Some(focal_pledges);
            pre_act_audited_pledges_count = Some(focal_audited);

            let outgoing_scan = extract_outgoing_commitments_in_memory(
                src_dir,
                arm_map.day,
                arm_map.focal_seat,
                &obs,
            )?;
            unparseable_sidecars =
                unparseable_sidecars.saturating_add(outgoing_scan.unparseable_files);
            if let Some((outgoing, out_counts)) = outgoing_scan.extracted {
                let eval_counts = evaluate_talk_commitments(
                    &outgoing,
                    &save_path,
                    &seal_path,
                    arm_map.day,
                    arm_map.focal_seat,
                    &decision,
                    &outcome,
                )?;
                extracted_outgoing_commitments = Some(eval_counts.extracted_outgoing_commitments);
                applicable_commitments = Some(eval_counts.applicable_commitments);
                inapplicable_commitments = Some(eval_counts.inapplicable_commitments);
                honoured_commitments = Some(eval_counts.honoured_commitments);
                third_party_fill_commitments = Some(eval_counts.third_party_fill_commitments);
                posted_unfilled_commitments = Some(eval_counts.posted_unfilled_commitments);
                partial_bilateral_fill_commitments =
                    Some(eval_counts.partial_bilateral_fill_commitments);
                honour_rate = (eval_counts.applicable_commitments > 0).then(|| {
                    (eval_counts.honoured_commitments as f64)
                        / (eval_counts.applicable_commitments as f64)
                });
                focal_talk_counts = Some(out_counts);
            }
        }
    }

    Ok(DecisionMechanismMetrics {
        tool_calls_used,
        used_any_tool_call,
        no_reply,
        revision_gain_ev_total_equity,
        revision_gain_score_norm,
        tool_round_reasoning_chars: reasoning_counts.tool_round_reasoning_chars,
        http_reply_reasoning_chars: reasoning_counts.http_reply_reasoning_chars,
        total_reasoning_chars: reasoning_counts.total_reasoning_chars,
        completion_tokens,
        reply_token_ids_len,
        has_reply_refusal,
        refused_market_actions,
        has_bundle_rejection,
        pre_act_pledges_count,
        pre_act_audited_pledges_count,
        sweep_broken_promises_count,
        terminal_broken_promises_count,
        extracted_outgoing_commitments,
        applicable_commitments,
        inapplicable_commitments,
        honoured_commitments,
        third_party_fill_commitments,
        posted_unfilled_commitments,
        partial_bilateral_fill_commitments,
        honour_rate,
        focal_talk_counts,
        unparseable_sidecars,
    })
}

fn opt_mean_u64(vals: impl Iterator<Item = Option<u64>>) -> (usize, Option<f64>) {
    let mut count = 0_usize;
    let mut sum = 0_u128;
    for v in vals.flatten() {
        count = count.saturating_add(1);
        sum = sum.saturating_add(u128::from(v));
    }
    let mean = (count > 0).then(|| (sum as f64) / (count as f64));
    (count, mean)
}

fn opt_sum_u32(vals: impl Iterator<Item = Option<u32>>, available: usize) -> Option<u64> {
    (available > 0).then(|| {
        vals.flatten()
            .map(u64::from)
            .fold(0_u64, u64::saturating_add)
    })
}

/// Aggregates `DecisionMechanismMetrics` across all decisions belonging to one arm.
pub fn summarize_arm_mechanisms(arm_rows: &[&JoinedDecisionRow]) -> PerArmMechanismSummary {
    let n_decisions = arm_rows.len();

    // Measure 1
    let n_with_tool_call = arm_rows
        .iter()
        .filter(|r| r.mechanisms.used_any_tool_call)
        .count();
    let sum_tool_calls: u64 = arm_rows
        .iter()
        .map(|r| u64::from(r.mechanisms.tool_calls_used))
        .fold(0_u64, u64::saturating_add);
    let mean_tool_calls_used =
        (n_decisions > 0).then(|| (sum_tool_calls as f64) / (n_decisions as f64));
    let tool_call_rate =
        (n_decisions > 0).then(|| (n_with_tool_call as f64) / (n_decisions as f64));

    // Measure 2
    let ev_gains: Vec<f64> = arm_rows
        .iter()
        .filter_map(|r| r.mechanisms.revision_gain_ev_total_equity)
        .collect();
    let norm_gains: Vec<f64> = arm_rows
        .iter()
        .filter_map(|r| r.mechanisms.revision_gain_score_norm)
        .collect();
    let n_with_first_draft = ev_gains.len();
    let n_with_revision_gain_score_norm = norm_gains.len();
    let mean_revision_gain_ev_total_equity =
        (!ev_gains.is_empty()).then(|| ev_gains.iter().sum::<f64>() / (ev_gains.len() as f64));
    let mean_revision_gain_score_norm = (!norm_gains.is_empty())
        .then(|| norm_gains.iter().sum::<f64>() / (norm_gains.len() as f64));

    // Measure 3
    let (n_with_reasoning_sidecars, mean_total_reasoning_chars) =
        opt_mean_u64(arm_rows.iter().map(|r| r.mechanisms.total_reasoning_chars));
    let (_, mean_tool_round_reasoning_chars) = opt_mean_u64(
        arm_rows
            .iter()
            .map(|r| r.mechanisms.tool_round_reasoning_chars),
    );
    let (_, mean_http_reply_reasoning_chars) = opt_mean_u64(
        arm_rows
            .iter()
            .map(|r| r.mechanisms.http_reply_reasoning_chars),
    );
    let sum_completion_tokens: u128 = arm_rows
        .iter()
        .map(|r| u128::from(r.mechanisms.completion_tokens))
        .sum();
    let mean_completion_tokens =
        (n_decisions > 0).then(|| (sum_completion_tokens as f64) / (n_decisions as f64));
    let (n_with_reply_token_ids, mean_reply_token_ids_len) =
        opt_mean_u64(arm_rows.iter().map(|r| r.mechanisms.reply_token_ids_len));

    // Measure 4
    let n_reply_refusals = arm_rows
        .iter()
        .filter(|r| r.mechanisms.has_reply_refusal)
        .count();
    let reply_refusal_rate =
        (n_decisions > 0).then(|| (n_reply_refusals as f64) / (n_decisions as f64));
    let sum_refused_actions: u64 = arm_rows
        .iter()
        .map(|r| u64::from(r.mechanisms.refused_market_actions))
        .fold(0_u64, u64::saturating_add);
    let mean_refused_market_actions =
        (n_decisions > 0).then(|| (sum_refused_actions as f64) / (n_decisions as f64));
    let n_bundle_rejections = arm_rows
        .iter()
        .filter(|r| r.mechanisms.has_bundle_rejection)
        .count();
    let bundle_rejection_rate =
        (n_decisions > 0).then(|| (n_bundle_rejections as f64) / (n_decisions as f64));

    // Measure 5(a)
    let n_with_pre_act_pledge_snapshot = arm_rows
        .iter()
        .filter(|r| r.mechanisms.pre_act_pledges_count.is_some())
        .count();
    let n_with_terminal_scoreboard = arm_rows
        .iter()
        .filter(|r| r.mechanisms.terminal_broken_promises_count.is_some())
        .count();
    let total_pre_act_pledges = opt_sum_u32(
        arm_rows.iter().map(|r| r.mechanisms.pre_act_pledges_count),
        n_with_pre_act_pledge_snapshot,
    );
    let total_pre_act_audited_pledges = opt_sum_u32(
        arm_rows
            .iter()
            .map(|r| r.mechanisms.pre_act_audited_pledges_count),
        n_with_pre_act_pledge_snapshot,
    );
    let total_sweep_broken_promises: u64 = arm_rows
        .iter()
        .map(|r| u64::from(r.mechanisms.sweep_broken_promises_count))
        .fold(0_u64, u64::saturating_add);
    let total_terminal_broken_promises = opt_sum_u32(
        arm_rows
            .iter()
            .map(|r| r.mechanisms.terminal_broken_promises_count),
        n_with_terminal_scoreboard,
    );

    // Measure 5(b)
    let n_with_talk_commitment_inputs = arm_rows
        .iter()
        .filter(|r| r.mechanisms.applicable_commitments.is_some())
        .count();
    let extracted_outgoing_commitments_total = opt_sum_u32(
        arm_rows
            .iter()
            .map(|r| r.mechanisms.extracted_outgoing_commitments),
        n_with_talk_commitment_inputs,
    );
    let applicable_commitments_total = opt_sum_u32(
        arm_rows.iter().map(|r| r.mechanisms.applicable_commitments),
        n_with_talk_commitment_inputs,
    );
    let inapplicable_commitments_total = opt_sum_u32(
        arm_rows
            .iter()
            .map(|r| r.mechanisms.inapplicable_commitments),
        n_with_talk_commitment_inputs,
    );
    let honoured_commitments_total = opt_sum_u32(
        arm_rows.iter().map(|r| r.mechanisms.honoured_commitments),
        n_with_talk_commitment_inputs,
    );
    let third_party_fill_commitments_total = opt_sum_u32(
        arm_rows
            .iter()
            .map(|r| r.mechanisms.third_party_fill_commitments),
        n_with_talk_commitment_inputs,
    );
    let posted_unfilled_commitments_total = opt_sum_u32(
        arm_rows
            .iter()
            .map(|r| r.mechanisms.posted_unfilled_commitments),
        n_with_talk_commitment_inputs,
    );
    let partial_bilateral_fill_commitments_total = opt_sum_u32(
        arm_rows
            .iter()
            .map(|r| r.mechanisms.partial_bilateral_fill_commitments),
        n_with_talk_commitment_inputs,
    );
    let honour_rate = honoured_commitments_total
        .zip(applicable_commitments_total)
        .and_then(|(h, a)| (a > 0).then(|| (h as f64) / (a as f64)));

    let focal_talk_counts = if n_with_talk_commitment_inputs == 0 {
        None
    } else {
        let mut pairs = Vec::new();
        for r in arm_rows {
            if let Some(ref counts) = r.mechanisms.focal_talk_counts {
                pairs.push((r.state_id.as_str(), counts));
            }
        }
        Some(summarize_arm_talk_counts_from_slices(&pairs))
    };

    let n_unparseable_sidecars: u64 = arm_rows
        .iter()
        .map(|r| u64::from(r.mechanisms.unparseable_sidecars))
        .fold(0_u64, u64::saturating_add);

    PerArmMechanismSummary {
        n_decisions,
        n_with_tool_call,
        mean_tool_calls_used,
        tool_call_rate,
        n_with_first_draft,
        n_with_revision_gain_score_norm,
        mean_revision_gain_ev_total_equity,
        mean_revision_gain_score_norm,
        n_with_reasoning_sidecars,
        n_with_reply_token_ids,
        mean_tool_round_reasoning_chars,
        mean_http_reply_reasoning_chars,
        mean_total_reasoning_chars,
        mean_completion_tokens,
        mean_reply_token_ids_len,
        n_reply_refusals,
        reply_refusal_rate,
        mean_refused_market_actions,
        n_bundle_rejections,
        bundle_rejection_rate,
        n_with_pre_act_pledge_snapshot,
        n_with_terminal_scoreboard,
        total_pre_act_pledges,
        total_pre_act_audited_pledges,
        total_sweep_broken_promises,
        total_terminal_broken_promises,
        n_with_talk_commitment_inputs,
        extracted_outgoing_commitments_total,
        applicable_commitments_total,
        inapplicable_commitments_total,
        honoured_commitments_total,
        third_party_fill_commitments_total,
        posted_unfilled_commitments_total,
        partial_bilateral_fill_commitments_total,
        honour_rate,
        focal_talk_counts,
        n_unparseable_sidecars,
    }
}
