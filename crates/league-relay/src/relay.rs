//! Core single-turn relay execution: wrapper prepending, transcript replay,
//! concurrent seat dispatch, wall-clock timeouts, one identical `harness`
//! re-send on delivery failure, verbatim reply writes, and credit-cap gating.

use crate::error::RelayError;
use crate::seats::{OpenRouterSeat, SeatBackend, SeatFile};
use crate::secret::{ApiKey, redact_and_truncate, redact_secrets};
use crate::transport::{
    OPENROUTER_CHAT_COMPLETIONS_URL, RawHttpResponse, Transport, TransportError,
};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::any::Any;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};

const CAP_EPSILON: f64 = 1e-12;

/// Market-day round selector (`talk` or `act`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Round {
    /// Morning whisper round (`talk`).
    Talk,
    /// Sealed action round (`act`).
    Act,
}

impl Round {
    /// Parse `"talk"` or `"act"`.
    pub fn parse(raw: &str) -> Result<Self, RelayError> {
        match raw {
            "talk" => Ok(Self::Talk),
            "act" => Ok(Self::Act),
            _ => Err(RelayError::Usage(
                "--round must be 'talk' or 'act'".to_owned(),
            )),
        }
    }

    /// Lowercase wire and filename label (`"talk"` or `"act"`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Talk => "talk",
            Self::Act => "act",
        }
    }
}

impl fmt::Display for Round {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Validate a two-digit day identifier (`"01"`..=`"99"`).
pub fn validate_day(day: &str) -> Result<(), RelayError> {
    if day.len() != 2 || !day.bytes().all(|b| b.is_ascii_digit()) || day == "00" {
        return Err(RelayError::Usage(
            "--day must be two digits between 01 and 99 (for example --day 01)".to_owned(),
        ));
    }
    Ok(())
}

/// Parameters for one `league-relay turn` invocation.
#[derive(Debug, Clone)]
pub struct TurnConfig {
    /// Root directory for the game (`to-seat/`, `from-seat/`, `relay/`).
    pub game_dir: PathBuf,
    /// Two-digit day label (`"01"`..=`"99"`).
    pub day: String,
    /// Round within the day (`talk` or `act`).
    pub round: Round,
    /// Validated seat configuration.
    pub seats: SeatFile,
    /// Optional path to first-turn wrapper markdown.
    pub wrapper_path: Option<PathBuf>,
    /// Whether this run is operating in live mode (`--live`).
    pub live: bool,
    /// Maximum cumulative recorded credits allowed before refusing calls.
    pub cap_credits: Option<f64>,
    /// Target chat-completions URL.
    pub endpoint_url: String,
}

impl TurnConfig {
    /// Create a configuration targeting [`OPENROUTER_CHAT_COMPLETIONS_URL`].
    pub fn new(
        game_dir: PathBuf,
        day: String,
        round: Round,
        seats: SeatFile,
        wrapper_path: Option<PathBuf>,
        live: bool,
        cap_credits: Option<f64>,
    ) -> Self {
        Self {
            game_dir,
            day,
            round,
            seats,
            wrapper_path,
            live,
            cap_credits,
            endpoint_url: OPENROUTER_CHAT_COMPLETIONS_URL.to_owned(),
        }
    }
}

/// One line in `<game-dir>/relay/seats/<Guild>.jsonl`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptEntry {
    /// Day when this turn was recorded (`"01"`..=`"99"`).
    pub day: String,
    /// Round when this turn was recorded (`talk` or `act`).
    pub round: Round,
    /// Chat role (`"user"` or `"assistant"` only; never `"system"`).
    pub role: String,
    /// Exact message content sent or received.
    pub content: String,
}

/// One line in `<game-dir>/relay/calls.jsonl`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallLogEntry {
    /// UTC timestamp in RFC 3339 millisecond format.
    pub time_utc: String,
    /// Two-digit game day (`"01"`..=`"99"`).
    pub day: String,
    /// Round (`talk` or `act`).
    pub round: Round,
    /// Seated guild name.
    pub guild: String,
    /// Attempt number (`1` for initial send, `2` for the single `harness` re-send).
    pub attempt: u8,
    /// `"turn"` on attempt 1, `"harness"` on attempt 2.
    pub trigger: String,
    /// Exact pinned model slug requested for the seat.
    pub model_requested: String,
    /// Model identifier reported by the response, if any.
    pub model_returned: Option<String>,
    /// Lowercase hex SHA-256 of the serialized UTF-8 request JSON body.
    pub request_body_sha256: String,
    /// Lowercase hex SHA-256 of the verbatim assistant reply text, if delivered.
    pub reply_sha256: Option<String>,
    /// Prompt token count reported by `usage.prompt_tokens`, if any.
    pub prompt_tokens: Option<u64>,
    /// Completion token count reported by `usage.completion_tokens`, if any.
    pub completion_tokens: Option<u64>,
    /// Total token count reported by `usage.total_tokens`, if any.
    pub total_tokens: Option<u64>,
    /// Call cost in OpenRouter credits if carried by `usage.cost`.
    pub cost: Option<f64>,
    /// Credits charged against `--cap-credits` for this entry: `usage.cost` when a valid
    /// non-negative finite cost was reported, `max_cost_per_call` when the call was sent
    /// without a valid reported cost, or `0.0` when refused pre-call.
    #[serde(default)]
    pub charged_credits: f64,
    /// Wall-clock call duration in milliseconds (`0` when refused pre-call).
    pub latency_ms: u64,
    /// Call outcome: `"OK"`, `"harness"`, `"NO_REPLY"`, `"LATE_DISCARDED"`, `"CAP_REACHED"`,
    /// `"COST_UNKNOWN"`, `"COST_OVER_ESTIMATE"`, or `"LEDGER_ERROR"`.
    pub outcome: String,
    /// Redacted failure or refusal reason when `outcome != "OK"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Per-seat outcome produced by [`run_turn`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeatOutcome {
    /// `file` backend seat; left for manual relay.
    RelayedByHand,
    /// Reply received, written verbatim to `from-seat/`, and appended to transcript.
    Delivered {
        /// Number of attempts made (`1` or `2`).
        attempts: u8,
        /// Path written under `<game-dir>/from-seat/`.
        from_seat_path: PathBuf,
        /// Path appended under `<game-dir>/relay/seats/<Guild>.jsonl`.
        transcript_path: PathBuf,
        /// Path appended under `<game-dir>/relay/calls.jsonl`.
        calls_path: PathBuf,
        /// Hex SHA-256 of the serialized request body.
        request_body_sha256: String,
        /// Hex SHA-256 of the verbatim reply bytes.
        reply_sha256: String,
    },
    /// Initial send and single `harness` re-send both failed; no `from-seat` file written.
    NoReply {
        /// Path to `<game-dir>/relay/calls.jsonl`.
        calls_path: PathBuf,
        /// Hex SHA-256 of the serialized request body.
        request_body_sha256: String,
    },
    /// Refused before calling transport because recorded costs reached/passed `--cap-credits`.
    CapReached {
        /// Path to `<game-dir>/relay/calls.jsonl`.
        calls_path: PathBuf,
        /// Hex SHA-256 of the serialized request body.
        request_body_sha256: String,
    },
    /// Refused fail-closed because a prior response in this turn carried no cost.
    CostUnknownRefused {
        /// Path to `<game-dir>/relay/calls.jsonl`.
        calls_path: PathBuf,
        /// Hex SHA-256 of the serialized request body.
        request_body_sha256: String,
    },
    /// Refused fail-closed because a prior response in this turn exceeded `max_cost_per_call`.
    CostOverEstimateRefused {
        /// Path to `<game-dir>/relay/calls.jsonl`.
        calls_path: PathBuf,
        /// Hex SHA-256 of the serialized request body.
        request_body_sha256: String,
    },
    /// Per-seat input/transcript error (redacted); did not stop sibling seats.
    SeatError {
        /// Redacted diagnostic detail.
        detail: String,
    },
}

/// Report for a single seat in seat-file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatReport {
    /// Guild name.
    pub guild: String,
    /// Outcome for this guild.
    pub outcome: SeatOutcome,
}

/// Summary of all seats after [`run_turn`] completes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnSummary {
    /// Per-seat reports in the same order as the seat file.
    pub reports: Vec<SeatReport>,
}

impl TurnSummary {
    /// `true` if every seat either succeeded (`Delivered`) or was `RelayedByHand`.
    pub fn all_clean(&self) -> bool {
        self.reports.iter().all(|r| {
            matches!(
                r.outcome,
                SeatOutcome::RelayedByHand | SeatOutcome::Delivered { .. }
            )
        })
    }

    /// Format a plain paths-and-hashes report (never prints prompt or reply bodies).
    pub fn format_paths_and_hashes(&self) -> String {
        let mut out = String::new();
        for report in &self.reports {
            let g = &report.guild;
            let line = match &report.outcome {
                SeatOutcome::RelayedByHand => {
                    format!("seat={g} outcome=FILE_MANUAL note=relayed_by_hand")
                }
                SeatOutcome::Delivered {
                    attempts,
                    from_seat_path,
                    transcript_path,
                    calls_path,
                    request_body_sha256,
                    reply_sha256,
                } => format!(
                    "seat={g} outcome=OK attempts={attempts} from_seat={} transcript={} calls={} request_sha256={request_body_sha256} reply_sha256={reply_sha256}",
                    from_seat_path.display(),
                    transcript_path.display(),
                    calls_path.display(),
                ),
                SeatOutcome::NoReply {
                    calls_path,
                    request_body_sha256,
                } => format!(
                    "seat={g} outcome=NO_REPLY calls={} request_sha256={request_body_sha256}",
                    calls_path.display()
                ),
                SeatOutcome::CapReached {
                    calls_path,
                    request_body_sha256,
                } => format!(
                    "seat={g} outcome=CAP_REACHED calls={} request_sha256={request_body_sha256}",
                    calls_path.display()
                ),
                SeatOutcome::CostUnknownRefused {
                    calls_path,
                    request_body_sha256,
                } => format!(
                    "seat={g} outcome=COST_UNKNOWN calls={} request_sha256={request_body_sha256}",
                    calls_path.display()
                ),
                SeatOutcome::CostOverEstimateRefused {
                    calls_path,
                    request_body_sha256,
                } => format!(
                    "seat={g} outcome=COST_OVER_ESTIMATE calls={} request_sha256={request_body_sha256}",
                    calls_path.display()
                ),
                SeatOutcome::SeatError { detail } => {
                    format!("seat={g} outcome=ERROR detail={detail}")
                }
            };
            out.push_str(&line);
            out.push('\n');
        }
        out
    }
}

#[derive(Debug, Serialize)]
struct ChatMessageWire<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Debug, Serialize)]
struct ChatCompletionRequestWire<'a> {
    model: &'a str,
    messages: Vec<ChatMessageWire<'a>>,
    temperature: f64,
    max_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponseWire {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    choices: Vec<ChatChoiceWire>,
    #[serde(default)]
    usage: Option<UsageWire>,
    #[serde(default)]
    error: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct ChatChoiceWire {
    #[serde(default)]
    message: Option<ChatChoiceMessageWire>,
}

#[derive(Debug, Deserialize)]
struct ChatChoiceMessageWire {
    #[serde(default)]
    content: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct UsageWire {
    #[serde(default)]
    prompt_tokens: Option<u64>,
    #[serde(default)]
    completion_tokens: Option<u64>,
    #[serde(default)]
    total_tokens: Option<u64>,
    #[serde(default)]
    cost: Option<f64>,
}

fn io_err(path: &Path, err: impl fmt::Display, key: Option<&ApiKey>) -> RelayError {
    RelayError::Io(redact_secrets(&format!("{}: {err}", path.display()), key))
}

const PENDING_CALLS_FILENAME: &str = "calls.pending.jsonl";

fn pending_calls_path(calls_path: &Path) -> PathBuf {
    calls_path.with_file_name(PENDING_CALLS_FILENAME)
}

fn append_text(path: &Path, text: &str, key: Option<&ApiKey>) -> Result<(), RelayError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| io_err(parent, e, key))?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| io_err(path, e, key))?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.flush())
        .map_err(|e| io_err(path, e, key))
}

fn append_text_synced(path: &Path, text: &str, key: Option<&ApiKey>) -> Result<(), RelayError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| io_err(parent, e, key))?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| io_err(path, e, key))?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.flush())
        .and_then(|()| file.sync_all())
        .map_err(|e| io_err(path, e, key))
}

fn filter_unmigrated_entries(
    ledger_entries: &[CallLogEntry],
    pending_entries: &[CallLogEntry],
) -> Vec<CallLogEntry> {
    let mut matched = vec![false; ledger_entries.len()];
    let mut out = Vec::new();
    for p in pending_entries {
        if let Some(pos) = ledger_entries
            .iter()
            .enumerate()
            .position(|(i, l)| !matched[i] && l == p)
        {
            matched[pos] = true;
        } else {
            out.push(p.clone());
        }
    }
    out
}

fn write_pending_side_file(
    pending_path: &Path,
    new_entries: &[CallLogEntry],
    key: Option<&ApiKey>,
) -> Result<(), RelayError> {
    if let Some(parent) = pending_path.parent() {
        fs::create_dir_all(parent).map_err(|e| io_err(parent, e, key))?;
    }
    let mut combined = if pending_path.is_file() {
        read_calls_file(pending_path, key)?
    } else {
        Vec::new()
    };
    let extras = filter_unmigrated_entries(&combined, new_entries);
    combined.extend(extras);

    let mut blob = String::new();
    for entry in &combined {
        let line = serde_json::to_string(entry).map_err(|e| io_err(pending_path, e, key))?;
        blob.push_str(&line);
        blob.push('\n');
    }

    let mut file = if pending_path.exists() {
        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(pending_path)
            .map_err(|e| io_err(pending_path, e, key))?
    } else {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(pending_path)
            .map_err(|e| io_err(pending_path, e, key))?
    };
    file.write_all(blob.as_bytes())
        .and_then(|()| file.flush())
        .and_then(|()| file.sync_all())
        .map_err(|e| io_err(pending_path, e, key))
}

struct CallLedger {
    calls_path: PathBuf,
    pending_path: PathBuf,
    cap_credits: Option<f64>,
    reserved_in_flight: f64,
    unrecorded_charged_in_memory: f64,
    pending_unwritten: Vec<(f64, CallLogEntry)>,
    ledger_write_failed_in_turn: bool,
    cost_unknown_in_turn: bool,
    cost_over_estimate_in_turn: bool,
}

enum PreCallDecision {
    Reserved,
    CapReached {
        charged_sum: f64,
        reserved_in_flight: f64,
        requested: f64,
        cap: f64,
    },
    LedgerWriteFailed,
    CostUnknown,
    CostOverEstimate,
}

impl CallLedger {
    fn open(
        calls_path: PathBuf,
        day: &str,
        round: Round,
        cap_credits: Option<f64>,
        redaction_key: Option<&ApiKey>,
    ) -> Result<Self, RelayError> {
        let pending_path = pending_calls_path(&calls_path);
        let mut effective_entries = if calls_path.exists() {
            read_calls_file(&calls_path, redaction_key)?
        } else {
            Vec::new()
        };

        if pending_path.exists() {
            let pending_entries = read_calls_file(&pending_path, redaction_key)?;
            let unmigrated = filter_unmigrated_entries(&effective_entries, &pending_entries);
            if unmigrated.is_empty() {
                let _ = fs::remove_file(&pending_path);
            } else {
                let mut migrated_blob = String::new();
                for entry in &unmigrated {
                    let line = serde_json::to_string(entry)
                        .map_err(|e| io_err(&calls_path, e, redaction_key))?;
                    migrated_blob.push_str(&line);
                    migrated_blob.push('\n');
                }
                if append_text_synced(&calls_path, &migrated_blob, redaction_key).is_ok() {
                    let _ = fs::remove_file(&pending_path);
                }
                effective_entries.extend(unmigrated);
            }
        }

        let mut cost_unknown_in_turn = false;
        let mut cost_over_estimate_in_turn = false;
        for entry in &effective_entries {
            if entry.day == day && entry.round == round {
                if entry_marks_cost_unknown(entry) {
                    cost_unknown_in_turn = true;
                }
                if entry.outcome == "COST_OVER_ESTIMATE" {
                    cost_over_estimate_in_turn = true;
                }
            }
        }
        Ok(Self {
            calls_path,
            pending_path,
            cap_credits,
            reserved_in_flight: 0.0,
            unrecorded_charged_in_memory: 0.0,
            pending_unwritten: Vec::new(),
            ledger_write_failed_in_turn: false,
            cost_unknown_in_turn,
            cost_over_estimate_in_turn,
        })
    }

    fn read_persisted_entries(
        &self,
        redaction_key: Option<&ApiKey>,
    ) -> Result<Vec<CallLogEntry>, RelayError> {
        let mut entries = if self.calls_path.exists() {
            read_calls_file(&self.calls_path, redaction_key)?
        } else {
            Vec::new()
        };
        if self.pending_path.exists() {
            let pending = read_calls_file(&self.pending_path, redaction_key)?;
            let unmigrated = filter_unmigrated_entries(&entries, &pending);
            entries.extend(unmigrated);
        }
        Ok(entries)
    }

    fn try_reserve(
        &mut self,
        seat_max_cost: f64,
        redaction_key: Option<&ApiKey>,
    ) -> Result<PreCallDecision, RelayError> {
        if self.ledger_write_failed_in_turn {
            return Ok(PreCallDecision::LedgerWriteFailed);
        }
        if self.cost_over_estimate_in_turn {
            return Ok(PreCallDecision::CostOverEstimate);
        }
        let Some(cap) = self.cap_credits else {
            self.reserved_in_flight += seat_max_cost;
            return Ok(PreCallDecision::Reserved);
        };
        let recorded_sum = sum_charged_credits(&self.read_persisted_entries(redaction_key)?)?;
        let charged_sum = recorded_sum + self.unrecorded_charged_in_memory;
        if charged_sum + self.reserved_in_flight + seat_max_cost > cap + CAP_EPSILON {
            return Ok(PreCallDecision::CapReached {
                charged_sum,
                reserved_in_flight: self.reserved_in_flight,
                requested: seat_max_cost,
                cap,
            });
        }
        if self.cost_unknown_in_turn {
            return Ok(PreCallDecision::CostUnknown);
        }
        self.reserved_in_flight += seat_max_cost;
        Ok(PreCallDecision::Reserved)
    }

    fn finish_call_and_record(
        &mut self,
        reserved_amount: f64,
        entry: &CallLogEntry,
        over_estimate: bool,
        redaction_key: Option<&ApiKey>,
    ) -> Result<(), RelayError> {
        if entry_marks_cost_unknown(entry) {
            self.cost_unknown_in_turn = true;
        }
        if over_estimate || entry.outcome == "COST_OVER_ESTIMATE" {
            self.cost_over_estimate_in_turn = true;
        }
        let append_res = serde_json::to_string(entry)
            .map_err(|e| io_err(&self.calls_path, e, redaction_key))
            .and_then(|mut line| {
                line.push('\n');
                append_text(&self.calls_path, &line, redaction_key)
            });
        match append_res {
            Ok(()) => {
                self.reserved_in_flight = (self.reserved_in_flight - reserved_amount).max(0.0);
                Ok(())
            }
            Err(e) => {
                self.reserved_in_flight = (self.reserved_in_flight - reserved_amount).max(0.0);
                let held_charge = entry.charged_credits.max(reserved_amount);
                self.unrecorded_charged_in_memory += held_charge;
                self.pending_unwritten.push((held_charge, entry.clone()));
                self.ledger_write_failed_in_turn = true;
                Err(e)
            }
        }
    }

    fn record_refusal(
        &mut self,
        entry: &CallLogEntry,
        redaction_key: Option<&ApiKey>,
    ) -> Result<(), RelayError> {
        if entry_marks_cost_unknown(entry) {
            self.cost_unknown_in_turn = true;
        }
        if entry.outcome == "COST_OVER_ESTIMATE" {
            self.cost_over_estimate_in_turn = true;
        }
        let mut line =
            serde_json::to_string(entry).map_err(|e| io_err(&self.calls_path, e, redaction_key))?;
        line.push('\n');
        append_text(&self.calls_path, &line, redaction_key)
    }

    fn finalize_turn(&mut self, redaction_key: Option<&ApiKey>) -> Result<(), RelayError> {
        if self.pending_unwritten.is_empty() {
            return Ok(());
        }

        let mut last_ledger_err: Option<RelayError> = None;
        while let Some((held_charge, pending_entry)) = self.pending_unwritten.first().cloned() {
            let mut line = match serde_json::to_string(&pending_entry) {
                Ok(s) => s,
                Err(e) => {
                    last_ledger_err = Some(io_err(&self.calls_path, e, redaction_key));
                    break;
                }
            };
            line.push('\n');
            match append_text_synced(&self.calls_path, &line, redaction_key) {
                Ok(()) => {
                    self.pending_unwritten.remove(0);
                    self.unrecorded_charged_in_memory =
                        (self.unrecorded_charged_in_memory - held_charge).max(0.0);
                }
                Err(e) => {
                    last_ledger_err = Some(e);
                    break;
                }
            }
        }

        if self.pending_unwritten.is_empty() {
            return Ok(());
        }

        let remaining: Vec<CallLogEntry> = self
            .pending_unwritten
            .iter()
            .map(|(_, entry)| entry.clone())
            .collect();
        match write_pending_side_file(&self.pending_path, &remaining, redaction_key) {
            Ok(()) => {
                self.pending_unwritten.clear();
                self.unrecorded_charged_in_memory = 0.0;
                Ok(())
            }
            Err(side_err) => {
                let total_unrecorded: f64 = self.pending_unwritten.iter().map(|(h, _)| *h).sum();
                let entries_summary = self
                    .pending_unwritten
                    .iter()
                    .map(|(held, e)| {
                        redact_secrets(
                            &format!(
                                "seat={} day={} round={} attempt={} charged_credits={held:.6}",
                                e.guild, e.day, e.round, e.attempt
                            ),
                            redaction_key,
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let ledger_detail = last_ledger_err
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "ledger append failed".to_owned());
                let detail = redact_secrets(
                    &format!("ledger ({ledger_detail}); side file ({side_err})"),
                    redaction_key,
                );
                Err(RelayError::UnrecordedCharge {
                    charged_credits: format!("{total_unrecorded:.6}"),
                    entries_summary,
                    detail,
                })
            }
        }
    }
}

fn entry_marks_cost_unknown(entry: &CallLogEntry) -> bool {
    if entry.outcome == "COST_UNKNOWN" {
        return true;
    }
    if entry.cost.is_some() {
        return false;
    }
    if entry.outcome == "OK" {
        return true;
    }
    let has_tokens = entry.prompt_tokens.is_some() || entry.total_tokens.is_some();
    let is_http_status_err = entry
        .error
        .as_deref()
        .is_some_and(|e| e.starts_with("http ") || e.starts_with("late http "));
    has_tokens && !is_http_status_err
}

fn read_calls_file(
    calls_path: &Path,
    redaction_key: Option<&ApiKey>,
) -> Result<Vec<CallLogEntry>, RelayError> {
    let raw = fs::read_to_string(calls_path).map_err(|e| io_err(calls_path, e, redaction_key))?;
    let mut entries = Vec::new();
    for (idx, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let parsed: CallLogEntry = serde_json::from_str(line).map_err(|e| {
            RelayError::CorruptLog(redact_secrets(
                &format!("{} line {}: {e}", calls_path.display(), idx + 1),
                redaction_key,
            ))
        })?;
        entries.push(parsed);
    }
    Ok(entries)
}

fn sum_charged_credits(entries: &[CallLogEntry]) -> Result<f64, RelayError> {
    let mut total = 0.0_f64;
    for entry in entries {
        if let Some(c) = entry.cost
            && (!c.is_finite() || c < 0.0)
        {
            return Err(RelayError::CorruptLog(
                "calls.jsonl contains a negative or non-finite cost value".to_owned(),
            ));
        }
        if !entry.charged_credits.is_finite() || entry.charged_credits < 0.0 {
            return Err(RelayError::CorruptLog(
                "calls.jsonl contains a negative or non-finite charged_credits value".to_owned(),
            ));
        }
        total += entry.charged_credits.max(entry.cost.unwrap_or(0.0));
    }
    Ok(total)
}

/// Compute lowercase hexadecimal SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Prepend `wrapper` before `block` with a clean paragraph boundary.
pub fn prepend_wrapper(wrapper: &str, block: &str) -> String {
    if wrapper.is_empty() {
        return block.to_owned();
    }
    if wrapper.ends_with("\n\n") || wrapper.ends_with("\r\n\r\n") {
        format!("{wrapper}{block}")
    } else if wrapper.ends_with('\n') {
        format!("{wrapper}\n{block}")
    } else {
        format!("{wrapper}\n\n{block}")
    }
}

/// Run one paper-test turn across all configured seats.
///
/// `openrouter` seats execute concurrently; a failure or panic in one seat
/// never halts another seat.
pub fn run_turn(
    config: &TurnConfig,
    transport: &dyn Transport,
    redaction_key: Option<&ApiKey>,
) -> Result<TurnSummary, RelayError> {
    validate_day(&config.day)?;

    if config.live && config.cap_credits.is_none() {
        return Err(RelayError::InvalidCap(
            "--cap-credits <n> is required when --live is passed".to_owned(),
        ));
    }
    if let Some(cap) = config.cap_credits
        && (!cap.is_finite() || cap < 0.0)
    {
        return Err(RelayError::InvalidCap(
            "--cap-credits must be a finite non-negative number".to_owned(),
        ));
    }

    let wrapper_text = match &config.wrapper_path {
        Some(path) => Some(fs::read_to_string(path).map_err(|e| io_err(path, e, redaction_key))?),
        None => None,
    };

    for seat in &config.seats.seats {
        if matches!(seat.backend, SeatBackend::Openrouter(_)) {
            let transcript_path = transcript_file_path(&config.game_dir, &seat.guild);
            if !transcript_has_entries(&transcript_path, redaction_key)? && wrapper_text.is_none() {
                return Err(RelayError::MissingWrapper {
                    guild: seat.guild.clone(),
                });
            }
        }
    }

    let calls_path = config.game_dir.join("relay").join("calls.jsonl");
    let ledger = Mutex::new(CallLedger::open(
        calls_path.clone(),
        &config.day,
        config.round,
        config.cap_credits,
        redaction_key,
    )?);

    let reports: Vec<SeatReport> = std::thread::scope(|scope| {
        let mut slots: Vec<(usize, SeatReport)> = Vec::with_capacity(config.seats.seats.len());
        let mut handles = Vec::new();

        for (idx, seat) in config.seats.seats.iter().enumerate() {
            match &seat.backend {
                SeatBackend::File { .. } => slots.push((
                    idx,
                    SeatReport {
                        guild: seat.guild.clone(),
                        outcome: SeatOutcome::RelayedByHand,
                    },
                )),
                SeatBackend::Openrouter(or_seat) => {
                    let guild = seat.guild.clone();
                    let wrapper_ref = wrapper_text.as_deref();
                    let ledger_ref = &ledger;
                    let calls_ref = &calls_path;
                    handles.push(scope.spawn(move || {
                        let outcome = run_single_openrouter_seat(
                            config,
                            &guild,
                            or_seat,
                            wrapper_ref,
                            transport,
                            ledger_ref,
                            calls_ref,
                            redaction_key,
                        );
                        (idx, SeatReport { guild, outcome })
                    }));
                }
            }
        }

        for handle in handles {
            if let Ok(pair) = handle.join() {
                slots.push(pair);
            }
        }
        slots.sort_by_key(|(idx, _)| *idx);
        slots.into_iter().map(|(_, r)| r).collect()
    });

    {
        let mut guard = ledger.lock().unwrap_or_else(|p| p.into_inner());
        guard.finalize_turn(redaction_key)?;
    }

    Ok(TurnSummary { reports })
}

#[allow(clippy::too_many_arguments)]
fn run_single_openrouter_seat(
    config: &TurnConfig,
    guild: &str,
    seat: &OpenRouterSeat,
    wrapper_text: Option<&str>,
    transport: &dyn Transport,
    ledger: &Mutex<CallLedger>,
    calls_path: &Path,
    redaction_key: Option<&ApiKey>,
) -> SeatOutcome {
    let filename = format!("day-{}-{}-{guild}.md", config.day, config.round);
    let to_seat_path = config.game_dir.join("to-seat").join(&filename);
    let from_seat_path = config.game_dir.join("from-seat").join(&filename);
    let transcript_path = transcript_file_path(&config.game_dir, guild);

    if from_seat_path.exists() {
        return SeatOutcome::SeatError {
            detail: redact_secrets(
                &format!(
                    "from-seat file already exists: {}",
                    from_seat_path.display()
                ),
                redaction_key,
            ),
        };
    }

    let raw_block = match fs::read_to_string(&to_seat_path) {
        Ok(text) => text,
        Err(e) => {
            return SeatOutcome::SeatError {
                detail: io_err(&to_seat_path, e, redaction_key).to_string(),
            };
        }
    };

    let history = match load_transcript(&transcript_path, redaction_key) {
        Ok(h) => h,
        Err(e) => {
            return SeatOutcome::SeatError {
                detail: e.to_string(),
            };
        }
    };

    let user_content = match (history.is_empty(), wrapper_text) {
        (true, Some(w)) => prepend_wrapper(w, &raw_block),
        _ => raw_block,
    };

    let mut messages: Vec<ChatMessageWire<'_>> = history
        .iter()
        .map(|entry| ChatMessageWire {
            role: &entry.role,
            content: &entry.content,
        })
        .collect();
    messages.push(ChatMessageWire {
        role: "user",
        content: &user_content,
    });

    let request_wire = ChatCompletionRequestWire {
        model: &seat.model,
        messages,
        temperature: seat.temperature,
        max_tokens: seat.max_output_tokens,
    };
    let Ok(request_body) = serde_json::to_string(&request_wire) else {
        return SeatOutcome::SeatError {
            detail: "failed to encode chat-completion request".to_owned(),
        };
    };
    let request_body_sha256 = sha256_hex(request_body.as_bytes());

    for attempt in [1_u8, 2_u8] {
        let trigger = if attempt == 1 { "turn" } else { "harness" };

        {
            let mut guard = ledger.lock().unwrap_or_else(|p| p.into_inner());
            match guard.try_reserve(seat.max_cost_per_call, redaction_key) {
                Ok(PreCallDecision::Reserved) => {}
                Ok(PreCallDecision::CapReached {
                    charged_sum,
                    reserved_in_flight,
                    requested,
                    cap,
                }) => {
                    let entry = refused_call_entry(
                        config,
                        guild,
                        attempt,
                        trigger,
                        &seat.model,
                        &request_body_sha256,
                        "CAP_REACHED",
                        format!(
                            "charged {charged_sum:.6} + reserved {reserved_in_flight:.6} + max_cost_per_call {requested:.6} exceeds cap {cap:.6}"
                        ),
                    );
                    if let Err(e) = guard.record_refusal(&entry, redaction_key) {
                        return SeatOutcome::SeatError {
                            detail: e.to_string(),
                        };
                    }
                    return SeatOutcome::CapReached {
                        calls_path: calls_path.to_path_buf(),
                        request_body_sha256,
                    };
                }
                Ok(PreCallDecision::LedgerWriteFailed) => {
                    let detail =
                        "prior call ledger write failed in turn; failing closed".to_owned();
                    let entry = refused_call_entry(
                        config,
                        guild,
                        attempt,
                        trigger,
                        &seat.model,
                        &request_body_sha256,
                        "LEDGER_ERROR",
                        detail.clone(),
                    );
                    let _ = guard.record_refusal(&entry, redaction_key);
                    return SeatOutcome::SeatError { detail };
                }
                Ok(PreCallDecision::CostUnknown) => {
                    let entry = refused_call_entry(
                        config,
                        guild,
                        attempt,
                        trigger,
                        &seat.model,
                        &request_body_sha256,
                        "COST_UNKNOWN",
                        "prior call in turn carried no cost; failing closed".to_owned(),
                    );
                    if let Err(e) = guard.record_refusal(&entry, redaction_key) {
                        return SeatOutcome::SeatError {
                            detail: e.to_string(),
                        };
                    }
                    return SeatOutcome::CostUnknownRefused {
                        calls_path: calls_path.to_path_buf(),
                        request_body_sha256,
                    };
                }
                Ok(PreCallDecision::CostOverEstimate) => {
                    let entry = refused_call_entry(
                        config,
                        guild,
                        attempt,
                        trigger,
                        &seat.model,
                        &request_body_sha256,
                        "COST_OVER_ESTIMATE",
                        "prior call in turn exceeded max_cost_per_call; failing closed".to_owned(),
                    );
                    if let Err(e) = guard.record_refusal(&entry, redaction_key) {
                        return SeatOutcome::SeatError {
                            detail: e.to_string(),
                        };
                    }
                    return SeatOutcome::CostOverEstimateRefused {
                        calls_path: calls_path.to_path_buf(),
                        request_body_sha256,
                    };
                }
                Err(e) => {
                    return SeatOutcome::SeatError {
                        detail: e.to_string(),
                    };
                }
            }
        }

        let (call_result, latency_ms) = call_with_wall_timeout(
            transport,
            &config.endpoint_url,
            &request_body,
            seat.timeout,
            redaction_key,
        );

        match evaluate_call_result(call_result, redaction_key) {
            EvaluatedCall::Success {
                reply_content,
                model_returned,
                usage,
            } => {
                let valid_cost = sanitize_cost(usage.cost);
                let charged_credits = valid_cost.unwrap_or(seat.max_cost_per_call);
                let over_estimate =
                    valid_cost.is_some_and(|c| c > seat.max_cost_per_call + CAP_EPSILON);
                let reply_sha256 = sha256_hex(reply_content.as_bytes());
                let (outcome_str, err_opt) = match (over_estimate, valid_cost) {
                    (true, Some(c)) => (
                        "COST_OVER_ESTIMATE".to_owned(),
                        Some(format!(
                            "returned cost {c:.6} exceeded max_cost_per_call {:.6}",
                            seat.max_cost_per_call
                        )),
                    ),
                    _ => ("OK".to_owned(), None),
                };
                let entry = CallLogEntry {
                    time_utc: now_utc_rfc3339(),
                    day: config.day.clone(),
                    round: config.round,
                    guild: guild.to_owned(),
                    attempt,
                    trigger: trigger.to_owned(),
                    model_requested: seat.model.clone(),
                    model_returned,
                    request_body_sha256: request_body_sha256.clone(),
                    reply_sha256: Some(reply_sha256.clone()),
                    prompt_tokens: usage.prompt_tokens,
                    completion_tokens: usage.completion_tokens,
                    total_tokens: usage.total_tokens,
                    cost: valid_cost,
                    charged_credits,
                    latency_ms,
                    outcome: outcome_str,
                    error: err_opt,
                };
                {
                    let mut guard = ledger.lock().unwrap_or_else(|p| p.into_inner());
                    if let Err(e) = guard.finish_call_and_record(
                        seat.max_cost_per_call,
                        &entry,
                        over_estimate,
                        redaction_key,
                    ) {
                        return SeatOutcome::SeatError {
                            detail: e.to_string(),
                        };
                    }
                }
                if let Err(e) = write_verbatim_reply(&from_seat_path, &reply_content, redaction_key)
                {
                    return SeatOutcome::SeatError {
                        detail: e.to_string(),
                    };
                }
                if let Err(e) = append_transcript_pair(
                    &transcript_path,
                    &config.day,
                    config.round,
                    &user_content,
                    &reply_content,
                    redaction_key,
                ) {
                    return SeatOutcome::SeatError {
                        detail: e.to_string(),
                    };
                }
                return SeatOutcome::Delivered {
                    attempts: attempt,
                    from_seat_path,
                    transcript_path,
                    calls_path: calls_path.to_path_buf(),
                    request_body_sha256,
                    reply_sha256,
                };
            }
            EvaluatedCall::DeliveryFailure {
                reason,
                model_returned,
                usage,
                is_late_reply,
            } => {
                let valid_cost = sanitize_cost(usage.cost);
                let charged_credits = valid_cost.unwrap_or(seat.max_cost_per_call);
                let over_estimate =
                    valid_cost.is_some_and(|c| c > seat.max_cost_per_call + CAP_EPSILON);
                let outcome_label = if is_late_reply {
                    "LATE_DISCARDED"
                } else if over_estimate {
                    "COST_OVER_ESTIMATE"
                } else if attempt == 1 {
                    "harness"
                } else {
                    "NO_REPLY"
                };
                let entry = CallLogEntry {
                    time_utc: now_utc_rfc3339(),
                    day: config.day.clone(),
                    round: config.round,
                    guild: guild.to_owned(),
                    attempt,
                    trigger: trigger.to_owned(),
                    model_requested: seat.model.clone(),
                    model_returned,
                    request_body_sha256: request_body_sha256.clone(),
                    reply_sha256: None,
                    prompt_tokens: usage.prompt_tokens,
                    completion_tokens: usage.completion_tokens,
                    total_tokens: usage.total_tokens,
                    cost: valid_cost,
                    charged_credits,
                    latency_ms,
                    outcome: outcome_label.to_owned(),
                    error: Some(reason),
                };
                {
                    let mut guard = ledger.lock().unwrap_or_else(|p| p.into_inner());
                    if let Err(e) = guard.finish_call_and_record(
                        seat.max_cost_per_call,
                        &entry,
                        over_estimate,
                        redaction_key,
                    ) {
                        return SeatOutcome::SeatError {
                            detail: e.to_string(),
                        };
                    }
                }
                if attempt == 2 {
                    return SeatOutcome::NoReply {
                        calls_path: calls_path.to_path_buf(),
                        request_body_sha256,
                    };
                }
            }
        }
    }

    unreachable!("two-attempt loop always returns on attempt 2")
}

#[allow(clippy::too_many_arguments)]
fn refused_call_entry(
    config: &TurnConfig,
    guild: &str,
    attempt: u8,
    trigger: &str,
    model: &str,
    request_body_sha256: &str,
    outcome: &str,
    reason: String,
) -> CallLogEntry {
    CallLogEntry {
        time_utc: now_utc_rfc3339(),
        day: config.day.clone(),
        round: config.round,
        guild: guild.to_owned(),
        attempt,
        trigger: trigger.to_owned(),
        model_requested: model.to_owned(),
        model_returned: None,
        request_body_sha256: request_body_sha256.to_owned(),
        reply_sha256: None,
        prompt_tokens: None,
        completion_tokens: None,
        total_tokens: None,
        cost: None,
        charged_credits: 0.0,
        latency_ms: 0,
        outcome: outcome.to_owned(),
        error: Some(reason),
    }
}

enum CallTransportResult {
    OnTime(Result<RawHttpResponse, TransportError>),
    TimedOut,
    Late(RawHttpResponse),
}

enum EvaluatedCall {
    Success {
        reply_content: String,
        model_returned: Option<String>,
        usage: UsageWire,
    },
    DeliveryFailure {
        reason: String,
        model_returned: Option<String>,
        usage: UsageWire,
        is_late_reply: bool,
    },
}

fn evaluate_call_result(
    call_result: CallTransportResult,
    redaction_key: Option<&ApiKey>,
) -> EvaluatedCall {
    let raw = match call_result {
        CallTransportResult::TimedOut
        | CallTransportResult::OnTime(Err(TransportError::Timeout)) => {
            return EvaluatedCall::DeliveryFailure {
                reason: "timeout".to_owned(),
                model_returned: None,
                usage: UsageWire::default(),
                is_late_reply: false,
            };
        }
        CallTransportResult::OnTime(Err(TransportError::Network(msg))) => {
            return EvaluatedCall::DeliveryFailure {
                reason: redact_secrets(&format!("transport: {msg}"), redaction_key),
                model_returned: None,
                usage: UsageWire::default(),
                is_late_reply: false,
            };
        }
        CallTransportResult::Late(late_raw) => {
            let parsed_opt: Result<ChatCompletionResponseWire, _> =
                serde_json::from_str(&late_raw.body);
            let (model_returned, usage) = match parsed_opt {
                Ok(p) => (
                    p.model.map(|m| redact_secrets(&m, redaction_key)),
                    p.usage.unwrap_or_default(),
                ),
                Err(_) => (None, UsageWire::default()),
            };
            let reason = if late_raw.status == 200 {
                "late reply after timeout discarded".to_owned()
            } else {
                format!("late http {} after timeout discarded", late_raw.status)
            };
            return EvaluatedCall::DeliveryFailure {
                reason,
                model_returned,
                usage,
                is_late_reply: true,
            };
        }
        CallTransportResult::OnTime(Ok(r)) => r,
    };

    let parsed_opt: Result<ChatCompletionResponseWire, _> = serde_json::from_str(&raw.body);

    if raw.status != 200 {
        let (model_returned, usage) = match parsed_opt {
            Ok(p) => (
                p.model.map(|m| redact_secrets(&m, redaction_key)),
                p.usage.unwrap_or_default(),
            ),
            Err(_) => (None, UsageWire::default()),
        };
        let snippet = redact_and_truncate(raw.body.trim(), redaction_key, 200);
        return EvaluatedCall::DeliveryFailure {
            reason: format!("http {}: {snippet}", raw.status),
            model_returned,
            usage,
            is_late_reply: false,
        };
    }

    let parsed = match parsed_opt {
        Ok(p) => p,
        Err(e) => {
            return EvaluatedCall::DeliveryFailure {
                reason: redact_secrets(&format!("invalid response JSON: {e}"), redaction_key),
                model_returned: None,
                usage: UsageWire::default(),
                is_late_reply: false,
            };
        }
    };

    let usage = parsed.usage.unwrap_or_default();
    let model_returned = parsed.model.map(|m| redact_secrets(&m, redaction_key));

    if let Some(err_val) = parsed.error {
        let snippet = redact_and_truncate(&err_val.to_string(), redaction_key, 200);
        return EvaluatedCall::DeliveryFailure {
            reason: format!("provider error: {snippet}"),
            model_returned,
            usage,
            is_late_reply: false,
        };
    }

    let content_opt = parsed
        .choices
        .into_iter()
        .next()
        .and_then(|c| c.message)
        .and_then(|m| m.content);

    match content_opt {
        Some(content) if !content.trim().is_empty() => EvaluatedCall::Success {
            reply_content: content,
            model_returned,
            usage,
        },
        _ => EvaluatedCall::DeliveryFailure {
            reason: "empty reply".to_owned(),
            model_returned,
            usage,
            is_late_reply: false,
        },
    }
}

fn sanitize_cost(cost: Option<f64>) -> Option<f64> {
    cost.filter(|c| c.is_finite() && *c >= 0.0)
}

fn call_with_wall_timeout(
    transport: &dyn Transport,
    url: &str,
    body: &str,
    timeout: Duration,
    redaction_key: Option<&ApiKey>,
) -> (CallTransportResult, u64) {
    let start = Instant::now();
    let (tx, rx) = mpsc::channel();
    std::thread::scope(|scope| {
        scope.spawn(move || {
            let res = match catch_unwind(AssertUnwindSafe(|| {
                transport.post_chat_completions(url, body, timeout)
            })) {
                Ok(Ok(resp)) => Ok(resp),
                Ok(Err(TransportError::Timeout)) => Err(TransportError::Timeout),
                Ok(Err(TransportError::Network(msg))) => {
                    Err(TransportError::Network(redact_secrets(&msg, redaction_key)))
                }
                Err(payload) => Err(TransportError::Network(redact_secrets(
                    &format!("transport panicked: {}", panic_message(&*payload)),
                    redaction_key,
                ))),
            };
            let _ = tx.send(res);
        });

        let outcome = match rx.recv_timeout(timeout) {
            Ok(res) => CallTransportResult::OnTime(res),
            Err(mpsc::RecvTimeoutError::Timeout) => match rx.recv() {
                Ok(Ok(late_resp)) => CallTransportResult::Late(late_resp),
                _ => CallTransportResult::TimedOut,
            },
            Err(mpsc::RecvTimeoutError::Disconnected) => CallTransportResult::TimedOut,
        };
        let elapsed = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
        (outcome, elapsed)
    })
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "panic".to_owned()
    }
}

fn transcript_file_path(game_dir: &Path, guild: &str) -> PathBuf {
    game_dir
        .join("relay")
        .join("seats")
        .join(format!("{guild}.jsonl"))
}

fn transcript_has_entries(
    transcript_path: &Path,
    redaction_key: Option<&ApiKey>,
) -> Result<bool, RelayError> {
    if !transcript_path.exists() {
        return Ok(false);
    }
    Ok(!load_transcript(transcript_path, redaction_key)?.is_empty())
}

fn load_transcript(
    transcript_path: &Path,
    redaction_key: Option<&ApiKey>,
) -> Result<Vec<TranscriptEntry>, RelayError> {
    if !transcript_path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(transcript_path)
        .map_err(|e| io_err(transcript_path, e, redaction_key))?;
    let mut entries = Vec::new();
    for (idx, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let parsed: TranscriptEntry = serde_json::from_str(line).map_err(|e| {
            RelayError::CorruptLog(redact_secrets(
                &format!("{} line {}: {e}", transcript_path.display(), idx + 1),
                redaction_key,
            ))
        })?;
        let expected_role = if entries.len() % 2 == 0 {
            "user"
        } else {
            "assistant"
        };
        if parsed.role != expected_role {
            return Err(RelayError::CorruptLog(redact_secrets(
                &format!(
                    "{} line {} has role '{}', expected '{expected_role}'",
                    transcript_path.display(),
                    idx + 1,
                    parsed.role
                ),
                redaction_key,
            )));
        }
        entries.push(parsed);
    }
    if entries.len() % 2 != 0 {
        return Err(RelayError::CorruptLog(redact_secrets(
            &format!(
                "{} ends on an unpaired user turn",
                transcript_path.display()
            ),
            redaction_key,
        )));
    }
    Ok(entries)
}

fn write_verbatim_reply(
    from_seat_path: &Path,
    reply_content: &str,
    redaction_key: Option<&ApiKey>,
) -> Result<(), RelayError> {
    if let Some(parent) = from_seat_path.parent() {
        fs::create_dir_all(parent).map_err(|e| io_err(parent, e, redaction_key))?;
    }
    fs::write(from_seat_path, reply_content.as_bytes())
        .map_err(|e| io_err(from_seat_path, e, redaction_key))
}

fn append_transcript_pair(
    transcript_path: &Path,
    day: &str,
    round: Round,
    user_content: &str,
    assistant_content: &str,
    redaction_key: Option<&ApiKey>,
) -> Result<(), RelayError> {
    let user_entry = TranscriptEntry {
        day: day.to_owned(),
        round,
        role: "user".to_owned(),
        content: user_content.to_owned(),
    };
    let assistant_entry = TranscriptEntry {
        day: day.to_owned(),
        round,
        role: "assistant".to_owned(),
        content: assistant_content.to_owned(),
    };
    let u_json = serde_json::to_string(&user_entry)
        .map_err(|e| io_err(transcript_path, e, redaction_key))?;
    let a_json = serde_json::to_string(&assistant_entry)
        .map_err(|e| io_err(transcript_path, e, redaction_key))?;
    let payload = format!("{u_json}\n{a_json}\n");
    append_text(transcript_path, &payload, redaction_key)
}

fn now_utc_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}
