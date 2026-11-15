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
use same_file::Handle as SameFileHandle;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::any::Any;
#[cfg(any(test, feature = "test-hooks"))]
use std::collections::HashSet;
use std::fmt;
use std::fs::{self, OpenOptions, TryLockError};
use std::io::{Seek, SeekFrom, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
#[cfg(any(test, feature = "test-hooks"))]
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
    /// without a valid reported cost or reconciled from an orphaned pre-call reservation,
    /// or `0.0` when refused pre-call.
    #[serde(default)]
    pub charged_credits: f64,
    /// Wall-clock call duration in milliseconds (`0` when refused or reserved pre-call).
    pub latency_ms: u64,
    /// Call outcome: `"OK"`, `"harness"`, `"NO_REPLY"`, `"LATE_DISCARDED"`, `"RESERVED"`,
    /// `"CAP_REACHED"`, `"COST_UNKNOWN"`, `"COST_OVER_ESTIMATE"`, or `"LEDGER_ERROR"`.
    pub outcome: String,
    /// Redacted failure or refusal reason when `outcome != "OK"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Unique pre-call reservation identifier tying a sent call's final record (or reconciled
    /// `"RESERVED"` entry) to its durable pre-send reservation (`None` on pre-call refusals or
    /// legacy lines).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reservation_id: Option<String>,
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

const TURN_LOCK_FILENAME: &str = "turn.lock";
const SIDE_FILE_LOCK_FILENAME: &str = "calls.side.lock";
const PENDING_CALLS_FILENAME: &str = "calls.pending.jsonl";
const PENDING_CALLS_TMP_FILENAME: &str = "calls.pending.jsonl.tmp";
const RESERVED_CALLS_FILENAME: &str = "calls.reserved.jsonl";
const RESERVED_CALLS_TMP_FILENAME: &str = "calls.reserved.jsonl.tmp";
const SIDE_FILE_LOCK_TIMEOUT: Duration = Duration::from_secs(10);

fn side_file_lock_path(sibling_path: &Path) -> PathBuf {
    sibling_path.with_file_name(SIDE_FILE_LOCK_FILENAME)
}

fn pending_calls_path(calls_path: &Path) -> PathBuf {
    calls_path.with_file_name(PENDING_CALLS_FILENAME)
}

fn pending_calls_tmp_path(pending_path: &Path) -> PathBuf {
    pending_path.with_file_name(PENDING_CALLS_TMP_FILENAME)
}

fn reserved_calls_path(calls_path: &Path) -> PathBuf {
    calls_path.with_file_name(RESERVED_CALLS_FILENAME)
}

fn reserved_calls_tmp_path(reserved_path: &Path) -> PathBuf {
    reserved_path.with_file_name(RESERVED_CALLS_TMP_FILENAME)
}

fn run_reservation_lock_path(calls_path: &Path, run_id: &str) -> PathBuf {
    calls_path.with_file_name(format!("calls.resv-{run_id}.lock"))
}

fn next_unique_run_id() -> String {
    static NEXT_SEQ: AtomicU64 = AtomicU64::new(1);
    let seq = NEXT_SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("{}-{nanos}-{seq}", std::process::id())
}

struct SideFileLockGuard {
    file: fs::File,
}

impl SideFileLockGuard {
    fn acquire(sibling_path: &Path, redaction_key: Option<&ApiKey>) -> Result<Self, RelayError> {
        let lock_path = side_file_lock_path(sibling_path);
        if let Some(parent) = lock_path.parent() {
            fs::create_dir_all(parent).map_err(|e| io_err(parent, e, redaction_key))?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|e| io_err(&lock_path, e, redaction_key))?;
        let start = Instant::now();
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(Self { file }),
                Err(TryLockError::WouldBlock) => {
                    if start.elapsed() >= SIDE_FILE_LOCK_TIMEOUT {
                        return Err(io_err(
                            &lock_path,
                            "timed out waiting for exclusive side-file lock",
                            redaction_key,
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(TryLockError::Error(e)) => {
                    return Err(io_err(&lock_path, e, redaction_key));
                }
            }
        }
    }
}

impl Drop for SideFileLockGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

fn read_lock_holder(lock_path: &Path, key: Option<&ApiKey>) -> Option<String> {
    let raw = fs::read_to_string(lock_path).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(redact_secrets(trimmed, key))
    }
}

struct TurnLock {
    lock_path: PathBuf,
    handle: SameFileHandle,
}

impl TurnLock {
    fn acquire(game_dir: &Path, redaction_key: Option<&ApiKey>) -> Result<Self, RelayError> {
        match fs::metadata(game_dir) {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => {
                return Err(io_err(
                    game_dir,
                    "game directory path is not a directory",
                    redaction_key,
                ));
            }
            Err(e) => {
                return Err(io_err(
                    game_dir,
                    format!("game directory does not exist ({e})"),
                    redaction_key,
                ));
            }
        }

        let relay_dir = game_dir.join("relay");
        fs::create_dir_all(&relay_dir).map_err(|e| io_err(&relay_dir, e, redaction_key))?;
        let lock_path = relay_dir.join(TURN_LOCK_FILENAME);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|e| io_err(&lock_path, e, redaction_key))?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                let holder = read_lock_holder(&lock_path, redaction_key);
                return Err(RelayError::TurnLocked {
                    lock_path: redact_secrets(&lock_path.display().to_string(), redaction_key),
                    holder,
                });
            }
            Err(TryLockError::Error(e)) => {
                return Err(io_err(&lock_path, e, redaction_key));
            }
        }
        let handle =
            SameFileHandle::from_file(file).map_err(|e| io_err(&lock_path, e, redaction_key))?;
        let mut lock = Self { lock_path, handle };
        lock.verify_identity(redaction_key)?;

        let holder_line = format!(
            "pid={} started_utc={}\n",
            std::process::id(),
            now_utc_rfc3339()
        );
        let locked_file = lock.handle.as_file_mut();
        let _ = locked_file.set_len(0);
        let _ = locked_file.seek(SeekFrom::Start(0));
        let _ = locked_file.write_all(holder_line.as_bytes());
        let _ = locked_file.flush();
        Ok(lock)
    }

    fn verify_identity(&self, redaction_key: Option<&ApiKey>) -> Result<(), RelayError> {
        match SameFileHandle::from_path(&self.lock_path) {
            Ok(path_handle) if path_handle == self.handle => Ok(()),
            Ok(_) => Err(RelayError::TurnLocked {
                lock_path: redact_secrets(&self.lock_path.display().to_string(), redaction_key),
                holder: Some(
                    "compromised: lock file at path refers to a different file".to_owned(),
                ),
            }),
            Err(e) => Err(RelayError::TurnLocked {
                lock_path: redact_secrets(&self.lock_path.display().to_string(), redaction_key),
                holder: Some(redact_secrets(
                    &format!("compromised: lock file missing or unreadable ({e})"),
                    redaction_key,
                )),
            }),
        }
    }
}

impl Drop for TurnLock {
    fn drop(&mut self) {
        if self.verify_identity(None).is_ok() {
            let locked_file = self.handle.as_file_mut();
            let _ = locked_file.set_len(0);
            let _ = locked_file.flush();
        }
        let _ = self.handle.as_file().unlock();
    }
}

#[cfg(any(test, feature = "test-hooks"))]
fn fail_pending_temp_write_paths() -> &'static Mutex<HashSet<PathBuf>> {
    static PATHS: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    PATHS.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Test-only fault-injection hook: when `fail` is `true`, writing the pending side
/// file at `pending_path` fails right after creating `calls.pending.jsonl.tmp`,
/// proving atomic rollback leaves `pending_path` untouched.
#[cfg(any(test, feature = "test-hooks"))]
pub fn set_fail_pending_temp_write_for_test(pending_path: &Path, fail: bool) {
    let mut set = fail_pending_temp_write_paths()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if fail {
        set.insert(pending_path.to_path_buf());
    } else {
        set.remove(pending_path);
    }
}

#[cfg(any(test, feature = "test-hooks"))]
fn should_fail_pending_temp_write_for_test(pending_path: &Path) -> bool {
    fail_pending_temp_write_paths()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .contains(pending_path)
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ActiveReservation {
    reservation_id: String,
    run_id: String,
    entry: CallLogEntry,
}

const RUN_RESV_LOCK_PREFIX: &str = "calls.resv-";
const RUN_RESV_LOCK_SUFFIX: &str = ".lock";

fn has_entry_with_reservation_id(entries: &[CallLogEntry], reservation_id: &str) -> bool {
    !reservation_id.is_empty()
        && entries
            .iter()
            .any(|e| e.reservation_id.as_deref() == Some(reservation_id))
}

fn is_run_reservation_lock_live(resv_lock_path: &Path) -> bool {
    let file = match OpenOptions::new()
        .read(true)
        .write(true)
        .open(resv_lock_path)
        .or_else(|_| OpenOptions::new().read(true).open(resv_lock_path))
    {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return false,
        Err(_) => return true,
    };
    match file.try_lock() {
        Ok(()) => {
            let _ = file.unlock();
            drop(file);
            let _ = fs::remove_file(resv_lock_path);
            false
        }
        Err(TryLockError::WouldBlock | TryLockError::Error(_)) => true,
    }
}

fn sweep_stale_reservation_locks(calls_path: &Path, active_reserved: &[ActiveReservation]) {
    let Some(relay_dir) = calls_path.parent() else {
        return;
    };
    let Ok(entries) = fs::read_dir(relay_dir) else {
        return;
    };
    for dir_entry in entries.flatten() {
        let file_name = dir_entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        let Some(run_id) = name
            .strip_prefix(RUN_RESV_LOCK_PREFIX)
            .and_then(|rest| rest.strip_suffix(RUN_RESV_LOCK_SUFFIX))
        else {
            continue;
        };
        if active_reserved.iter().any(|r| r.run_id == run_id) {
            continue;
        }
        let _ = is_run_reservation_lock_live(&dir_entry.path());
    }
}

fn read_reserved_file(
    reserved_path: &Path,
    redaction_key: Option<&ApiKey>,
) -> Result<Vec<ActiveReservation>, RelayError> {
    let raw =
        fs::read_to_string(reserved_path).map_err(|e| io_err(reserved_path, e, redaction_key))?;
    let mut entries = Vec::new();
    for (idx, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let parsed: ActiveReservation = serde_json::from_str(line).map_err(|e| {
            RelayError::CorruptLog(redact_secrets(
                &format!("{} line {}: {e}", reserved_path.display(), idx + 1),
                redaction_key,
            ))
        })?;
        entries.push(parsed);
    }
    Ok(entries)
}

fn write_reserved_entries_unlocked(
    reserved_path: &Path,
    entries: &[ActiveReservation],
    key: Option<&ApiKey>,
) -> Result<(), RelayError> {
    if entries.is_empty() {
        if reserved_path.exists() {
            match fs::remove_file(reserved_path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(io_err(reserved_path, e, key)),
            }
        }
        return Ok(());
    }
    if let Some(parent) = reserved_path.parent() {
        fs::create_dir_all(parent).map_err(|e| io_err(parent, e, key))?;
    }
    if reserved_path.exists() {
        let meta = fs::metadata(reserved_path).map_err(|e| io_err(reserved_path, e, key))?;
        if !meta.is_file() {
            return Err(io_err(
                reserved_path,
                "reservation side file path is not a regular file",
                key,
            ));
        }
        if meta.permissions().readonly() {
            return Err(io_err(
                reserved_path,
                "reservation side file is read-only",
                key,
            ));
        }
    }
    let mut blob = String::new();
    for item in entries {
        let line = serde_json::to_string(item).map_err(|e| io_err(reserved_path, e, key))?;
        blob.push_str(&line);
        blob.push('\n');
    }
    let tmp_path = reserved_calls_tmp_path(reserved_path);
    if tmp_path.exists() {
        fs::remove_file(&tmp_path).map_err(|e| io_err(&tmp_path, e, key))?;
    }
    let mut tmp_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp_path)
        .map_err(|e| io_err(&tmp_path, e, key))?;
    let write_res: Result<(), std::io::Error> = (|| {
        tmp_file.write_all(blob.as_bytes())?;
        tmp_file.flush()?;
        tmp_file.sync_all()
    })();
    drop(tmp_file);
    if let Err(e) = write_res {
        let _ = fs::remove_file(&tmp_path);
        return Err(io_err(&tmp_path, e, key));
    }
    if let Err(e) = fs::rename(&tmp_path, reserved_path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(io_err(reserved_path, e, key));
    }
    #[cfg(unix)]
    if let Some(parent) = reserved_path.parent()
        && let Ok(dir) = fs::File::open(parent)
    {
        let _ = dir.sync_all();
    }
    Ok(())
}

fn write_pending_side_file_unlocked(
    pending_path: &Path,
    new_entries: &[CallLogEntry],
    key: Option<&ApiKey>,
) -> Result<(), RelayError> {
    if let Some(parent) = pending_path.parent() {
        fs::create_dir_all(parent).map_err(|e| io_err(parent, e, key))?;
    }
    if pending_path.exists() {
        let meta = fs::metadata(pending_path).map_err(|e| io_err(pending_path, e, key))?;
        if !meta.is_file() {
            return Err(io_err(
                pending_path,
                "pending side file path is not a regular file",
                key,
            ));
        }
        if meta.permissions().readonly() {
            return Err(io_err(pending_path, "pending side file is read-only", key));
        }
    }
    let mut combined = if pending_path.is_file() {
        read_calls_file(pending_path, key)?
    } else {
        Vec::new()
    };
    let extras = filter_unmigrated_entries(&combined, new_entries);
    if extras.is_empty() && pending_path.is_file() {
        return Ok(());
    }
    combined.extend(extras);

    let mut blob = String::new();
    for entry in &combined {
        let line = serde_json::to_string(entry).map_err(|e| io_err(pending_path, e, key))?;
        blob.push_str(&line);
        blob.push('\n');
    }

    let tmp_path = pending_calls_tmp_path(pending_path);
    if tmp_path.exists() {
        fs::remove_file(&tmp_path).map_err(|e| io_err(&tmp_path, e, key))?;
    }
    let mut tmp_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp_path)
        .map_err(|e| io_err(&tmp_path, e, key))?;

    let write_res: Result<(), std::io::Error> = (|| {
        #[cfg(any(test, feature = "test-hooks"))]
        if should_fail_pending_temp_write_for_test(pending_path) {
            let prefix_len = blob.len().min(16);
            let _ = tmp_file.write_all(&blob.as_bytes()[..prefix_len]);
            return Err(std::io::Error::other(
                "injected write failure after temp file creation",
            ));
        }
        tmp_file.write_all(blob.as_bytes())?;
        tmp_file.flush()?;
        tmp_file.sync_all()
    })();
    drop(tmp_file);
    if let Err(e) = write_res {
        let _ = fs::remove_file(&tmp_path);
        return Err(io_err(&tmp_path, e, key));
    }

    if let Err(e) = fs::rename(&tmp_path, pending_path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(io_err(pending_path, e, key));
    }
    #[cfg(unix)]
    if let Some(parent) = pending_path.parent()
        && let Ok(dir) = fs::File::open(parent)
    {
        let _ = dir.sync_all();
    }
    Ok(())
}

fn write_pending_side_file(
    pending_path: &Path,
    new_entries: &[CallLogEntry],
    key: Option<&ApiKey>,
) -> Result<(), RelayError> {
    let _side_lock = SideFileLockGuard::acquire(pending_path, key)?;
    write_pending_side_file_unlocked(pending_path, new_entries, key)
}

struct LoadedLedgerState {
    active_resvs: Vec<ActiveReservation>,
    persisted_for_sum: Vec<CallLogEntry>,
}

struct CallLedger<'a> {
    turn_lock: &'a TurnLock,
    calls_path: PathBuf,
    pending_path: PathBuf,
    reserved_path: PathBuf,
    run_id: String,
    run_resv_lock_path: PathBuf,
    run_resv_lock_file: Option<fs::File>,
    next_reservation_seq: u64,
    cap_credits: Option<f64>,
    reserved_in_flight: f64,
    prior_unmigrated_pending: Vec<CallLogEntry>,
    pending_unwritten: Vec<(f64, CallLogEntry)>,
    unreleased_reservations: Vec<String>,
    lock_compromised: Option<RelayError>,
    ledger_write_failed_in_turn: bool,
    cost_unknown_in_turn: bool,
    cost_over_estimate_in_turn: bool,
}

enum PreCallDecision {
    Reserved(String),
    LockCompromised(RelayError),
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

impl<'a> CallLedger<'a> {
    fn open(
        turn_lock: &'a TurnLock,
        calls_path: PathBuf,
        day: &str,
        round: Round,
        cap_credits: Option<f64>,
        redaction_key: Option<&ApiKey>,
    ) -> Result<Self, RelayError> {
        turn_lock.verify_identity(redaction_key)?;
        let pending_path = pending_calls_path(&calls_path);
        let reserved_path = reserved_calls_path(&calls_path);
        let run_id = next_unique_run_id();
        let run_resv_lock_path = run_reservation_lock_path(&calls_path, &run_id);

        let _side_lock = SideFileLockGuard::acquire(&calls_path, redaction_key)?;
        let mut effective_entries = if calls_path.exists() {
            read_calls_file(&calls_path, redaction_key)?
        } else {
            Vec::new()
        };
        let mut prior_unmigrated_pending = Vec::new();

        if pending_path.exists() {
            let pending_entries = read_calls_file(&pending_path, redaction_key)?;
            let unmigrated = filter_unmigrated_entries(&effective_entries, &pending_entries);
            turn_lock.verify_identity(redaction_key)?;
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
                } else {
                    prior_unmigrated_pending = unmigrated.clone();
                }
                effective_entries.extend(unmigrated);
            }
        }

        let mut remaining_reserved = Vec::new();
        if reserved_path.exists() {
            let original_reserved = read_reserved_file(&reserved_path, redaction_key)?;
            let base_recorded_entries = effective_entries.clone();
            let mut orphans_to_migrate: Vec<(ActiveReservation, CallLogEntry)> = Vec::new();

            for r in &original_reserved {
                let already_recorded =
                    has_entry_with_reservation_id(&base_recorded_entries, &r.reservation_id);
                let lock_path = run_reservation_lock_path(&calls_path, &r.run_id);
                if is_run_reservation_lock_live(&lock_path) {
                    remaining_reserved.push(r.clone());
                    if !already_recorded {
                        effective_entries.push(r.entry.clone());
                    }
                } else if !already_recorded
                    && !orphans_to_migrate
                        .iter()
                        .any(|(prev, _)| prev.reservation_id == r.reservation_id)
                {
                    turn_lock.verify_identity(redaction_key)?;
                    let mut orphan_entry = r.entry.clone();
                    if orphan_entry.reservation_id.is_none() && !r.reservation_id.is_empty() {
                        orphan_entry.reservation_id = Some(r.reservation_id.clone());
                    }
                    orphans_to_migrate.push((r.clone(), orphan_entry));
                }
            }

            if !orphans_to_migrate.is_empty() {
                let mut orphan_blob = String::new();
                for (_, orphan_entry) in &orphans_to_migrate {
                    let line = serde_json::to_string(orphan_entry)
                        .map_err(|e| io_err(&calls_path, e, redaction_key))?;
                    orphan_blob.push_str(&line);
                    orphan_blob.push('\n');
                }
                if append_text_synced(&calls_path, &orphan_blob, redaction_key).is_ok() {
                    effective_entries
                        .extend(orphans_to_migrate.into_iter().map(|(_, entry)| entry));
                } else {
                    for (r, orphan_entry) in orphans_to_migrate {
                        prior_unmigrated_pending.push(orphan_entry.clone());
                        effective_entries.push(orphan_entry);
                        remaining_reserved.push(r);
                    }
                }
            }

            if remaining_reserved != original_reserved {
                let _ = write_reserved_entries_unlocked(
                    &reserved_path,
                    &remaining_reserved,
                    redaction_key,
                );
            }
        }
        sweep_stale_reservation_locks(&calls_path, &remaining_reserved);
        drop(_side_lock);

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
            turn_lock,
            calls_path,
            pending_path,
            reserved_path,
            run_id,
            run_resv_lock_path,
            run_resv_lock_file: None,
            next_reservation_seq: 1,
            cap_credits,
            reserved_in_flight: 0.0,
            prior_unmigrated_pending,
            pending_unwritten: Vec::new(),
            unreleased_reservations: Vec::new(),
            lock_compromised: None,
            ledger_write_failed_in_turn: false,
            cost_unknown_in_turn,
            cost_over_estimate_in_turn,
        })
    }

    fn check_lock_identity(&mut self, redaction_key: Option<&ApiKey>) {
        if self.lock_compromised.is_none()
            && let Err(lock_err) = self.turn_lock.verify_identity(redaction_key)
        {
            self.lock_compromised = Some(lock_err);
        }
    }

    fn ensure_run_resv_lock(&mut self, redaction_key: Option<&ApiKey>) -> Result<(), RelayError> {
        if self.run_resv_lock_file.is_some() {
            return Ok(());
        }
        if let Some(parent) = self.run_resv_lock_path.parent() {
            fs::create_dir_all(parent).map_err(|e| io_err(parent, e, redaction_key))?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&self.run_resv_lock_path)
            .map_err(|e| io_err(&self.run_resv_lock_path, e, redaction_key))?;
        match file.try_lock() {
            Ok(()) => {
                self.run_resv_lock_file = Some(file);
                Ok(())
            }
            Err(TryLockError::WouldBlock) => Err(io_err(
                &self.run_resv_lock_path,
                "run reservation lock unexpectedly contended",
                redaction_key,
            )),
            Err(TryLockError::Error(e)) => Err(io_err(&self.run_resv_lock_path, e, redaction_key)),
        }
    }

    fn release_reservations_unlocked(
        &self,
        ids: &[String],
        redaction_key: Option<&ApiKey>,
    ) -> Result<(), RelayError> {
        if ids.is_empty() || !self.reserved_path.exists() {
            return Ok(());
        }
        let mut current = read_reserved_file(&self.reserved_path, redaction_key)?;
        let before = current.len();
        current.retain(|r| !ids.iter().any(|id| id == &r.reservation_id));
        if current.len() != before {
            write_reserved_entries_unlocked(&self.reserved_path, &current, redaction_key)?;
        }
        Ok(())
    }

    fn read_state_unlocked(
        &self,
        redaction_key: Option<&ApiKey>,
    ) -> Result<LoadedLedgerState, RelayError> {
        let mut base_entries = if self.calls_path.exists() {
            read_calls_file(&self.calls_path, redaction_key)?
        } else {
            Vec::new()
        };
        if self.pending_path.exists() {
            let pending = read_calls_file(&self.pending_path, redaction_key)?;
            let unmigrated = filter_unmigrated_entries(&base_entries, &pending);
            base_entries.extend(unmigrated);
        }
        let active_resvs = if self.reserved_path.exists() {
            read_reserved_file(&self.reserved_path, redaction_key)?
        } else {
            Vec::new()
        };
        let mut persisted_for_sum = base_entries.clone();
        let mut counted_orphan_ids: Vec<String> = Vec::new();
        for r in &active_resvs {
            if r.run_id == self.run_id {
                continue;
            }
            if has_entry_with_reservation_id(&base_entries, &r.reservation_id) {
                continue;
            }
            let lock_path = run_reservation_lock_path(&self.calls_path, &r.run_id);
            if is_run_reservation_lock_live(&lock_path) {
                persisted_for_sum.push(r.entry.clone());
            } else if !counted_orphan_ids.iter().any(|id| id == &r.reservation_id) {
                counted_orphan_ids.push(r.reservation_id.clone());
                persisted_for_sum.push(r.entry.clone());
            }
        }
        Ok(LoadedLedgerState {
            active_resvs,
            persisted_for_sum,
        })
    }

    fn unpersisted_memory_sum(&self, persisted: &[CallLogEntry]) -> f64 {
        let mut matched = vec![false; persisted.len()];
        let mut sum = 0.0;
        for (held_charge, entry) in &self.pending_unwritten {
            if let Some(pos) = persisted
                .iter()
                .enumerate()
                .position(|(i, p)| !matched[i] && p == entry)
            {
                matched[pos] = true;
            } else {
                sum += *held_charge;
            }
        }
        sum
    }

    fn try_reserve(
        &mut self,
        seat_max_cost: f64,
        reservation_template: &CallLogEntry,
        redaction_key: Option<&ApiKey>,
    ) -> Result<PreCallDecision, RelayError> {
        self.check_lock_identity(redaction_key);
        if let Some(lock_err) = &self.lock_compromised {
            return Ok(PreCallDecision::LockCompromised(lock_err.clone()));
        }
        if self.ledger_write_failed_in_turn {
            return Ok(PreCallDecision::LedgerWriteFailed);
        }
        if self.cost_over_estimate_in_turn {
            return Ok(PreCallDecision::CostOverEstimate);
        }

        let _side_lock = SideFileLockGuard::acquire(&self.calls_path, redaction_key)?;
        let LoadedLedgerState {
            mut active_resvs,
            persisted_for_sum,
        } = self.read_state_unlocked(redaction_key)?;

        if let Some(cap) = self.cap_credits {
            let recorded_sum = sum_charged_credits(&persisted_for_sum)?;
            let charged_sum = recorded_sum + self.unpersisted_memory_sum(&persisted_for_sum);
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
        }

        self.ensure_run_resv_lock(redaction_key)?;
        let reservation_id = format!("{}-{}", self.run_id, self.next_reservation_seq);
        self.next_reservation_seq += 1;
        let mut reserved_entry = reservation_template.clone();
        reserved_entry.reservation_id = Some(reservation_id.clone());
        active_resvs.push(ActiveReservation {
            reservation_id: reservation_id.clone(),
            run_id: self.run_id.clone(),
            entry: reserved_entry,
        });
        write_reserved_entries_unlocked(&self.reserved_path, &active_resvs, redaction_key)?;

        self.check_lock_identity(redaction_key);
        if let Some(lock_err) = &self.lock_compromised {
            let _ = self.release_reservations_unlocked(
                std::slice::from_ref(&reservation_id),
                redaction_key,
            );
            return Ok(PreCallDecision::LockCompromised(lock_err.clone()));
        }

        self.reserved_in_flight += seat_max_cost;
        Ok(PreCallDecision::Reserved(reservation_id))
    }

    fn finish_call_and_record(
        &mut self,
        reservation_id: &str,
        reserved_amount: f64,
        entry: &CallLogEntry,
        over_estimate: bool,
        redaction_key: Option<&ApiKey>,
    ) -> Result<(), RelayError> {
        self.check_lock_identity(redaction_key);
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
        let single_id = [reservation_id.to_owned()];
        match append_res {
            Ok(()) => {
                self.reserved_in_flight = (self.reserved_in_flight - reserved_amount).max(0.0);
                if let Ok(_side_lock) = SideFileLockGuard::acquire(&self.calls_path, redaction_key)
                {
                    let _ = self.release_reservations_unlocked(&single_id, redaction_key);
                }
                Ok(())
            }
            Err(e) => {
                self.reserved_in_flight = (self.reserved_in_flight - reserved_amount).max(0.0);
                let held_charge = entry.charged_credits.max(reserved_amount);
                self.pending_unwritten.push((held_charge, entry.clone()));
                self.ledger_write_failed_in_turn = true;
                self.check_lock_identity(redaction_key);
                let pending_snapshot: Vec<CallLogEntry> = self
                    .pending_unwritten
                    .iter()
                    .map(|(_, item)| item.clone())
                    .collect();
                match SideFileLockGuard::acquire(&self.calls_path, redaction_key).and_then(|_g| {
                    write_pending_side_file_unlocked(
                        &self.pending_path,
                        &pending_snapshot,
                        redaction_key,
                    )?;
                    let _ = self.release_reservations_unlocked(&single_id, redaction_key);
                    Ok(())
                }) {
                    Ok(()) => {}
                    Err(_) => {
                        self.unreleased_reservations.push(reservation_id.to_owned());
                    }
                }
                Err(e)
            }
        }
    }

    fn record_refusal(
        &mut self,
        entry: &CallLogEntry,
        redaction_key: Option<&ApiKey>,
    ) -> Result<(), RelayError> {
        self.check_lock_identity(redaction_key);
        if let Some(lock_err) = &self.lock_compromised {
            return Err(lock_err.clone());
        }
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

    fn clear_unreleased_reservations(&mut self, redaction_key: Option<&ApiKey>) {
        if self.unreleased_reservations.is_empty() {
            return;
        }
        let ids = std::mem::take(&mut self.unreleased_reservations);
        if let Ok(_side_lock) = SideFileLockGuard::acquire(&self.calls_path, redaction_key) {
            let _ = self.release_reservations_unlocked(&ids, redaction_key);
        }
    }

    fn finalize_turn(&mut self, redaction_key: Option<&ApiKey>) -> Result<(), RelayError> {
        self.check_lock_identity(redaction_key);
        if self.pending_unwritten.is_empty() {
            self.clear_unreleased_reservations(redaction_key);
            if let Some(lock_err) = self.lock_compromised.take() {
                return Err(lock_err);
            }
            return Ok(());
        }

        let mut last_ledger_err: Option<RelayError> = None;
        while let Some((_, pending_entry)) = self.pending_unwritten.first().cloned() {
            self.check_lock_identity(redaction_key);
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
                    if let Some(ref resv_id) = pending_entry.reservation_id {
                        self.unreleased_reservations.retain(|id| id != resv_id);
                        if let Ok(_side_lock) =
                            SideFileLockGuard::acquire(&self.calls_path, redaction_key)
                        {
                            let _ = self.release_reservations_unlocked(
                                std::slice::from_ref(resv_id),
                                redaction_key,
                            );
                        }
                    }
                }
                Err(e) => {
                    last_ledger_err = Some(e);
                    break;
                }
            }
        }

        if self.pending_unwritten.is_empty() {
            self.clear_unreleased_reservations(redaction_key);
            if self.lock_compromised.is_none() {
                let _side_lock = SideFileLockGuard::acquire(&self.calls_path, redaction_key)?;
                if self.pending_path.is_file() {
                    let calls_entries = if self.calls_path.exists() {
                        read_calls_file(&self.calls_path, redaction_key)?
                    } else {
                        Vec::new()
                    };
                    let side_entries = read_calls_file(&self.pending_path, redaction_key)?;
                    let unmigrated = filter_unmigrated_entries(&calls_entries, &side_entries);
                    if unmigrated.is_empty() {
                        let _ = fs::remove_file(&self.pending_path);
                        self.prior_unmigrated_pending.clear();
                    } else {
                        let mut migrated_blob = String::new();
                        for entry in &unmigrated {
                            let line = serde_json::to_string(entry)
                                .map_err(|e| io_err(&self.calls_path, e, redaction_key))?;
                            migrated_blob.push_str(&line);
                            migrated_blob.push('\n');
                        }
                        if append_text_synced(&self.calls_path, &migrated_blob, redaction_key)
                            .is_ok()
                        {
                            let _ = fs::remove_file(&self.pending_path);
                            self.prior_unmigrated_pending.clear();
                        }
                    }
                }
            }
            self.check_lock_identity(redaction_key);
            if let Some(lock_err) = self.lock_compromised.take() {
                return Err(lock_err);
            }
            return Ok(());
        }

        self.check_lock_identity(redaction_key);
        let remaining: Vec<CallLogEntry> = self
            .pending_unwritten
            .iter()
            .map(|(_, entry)| entry.clone())
            .collect();
        match write_pending_side_file(&self.pending_path, &remaining, redaction_key) {
            Ok(()) => {
                self.pending_unwritten.clear();
                self.clear_unreleased_reservations(redaction_key);
                self.check_lock_identity(redaction_key);
                if let Some(lock_err) = self.lock_compromised.take() {
                    return Err(lock_err);
                }
                Ok(())
            }
            Err(side_err) => {
                // Keep unreleased reservations on disk in `calls.reserved.jsonl` when neither
                // `calls.jsonl` nor `calls.pending.jsonl` could be written (exit 3), preserving
                // the durable pre-call record so a subsequent run still counts `max_cost_per_call`.
                let mut all_held: Vec<(f64, CallLogEntry)> = self
                    .prior_unmigrated_pending
                    .iter()
                    .filter(|e| {
                        e.charged_credits > 0.0
                            || e.cost.unwrap_or(0.0) > 0.0
                            || e.outcome != "CAP_REACHED"
                    })
                    .map(|e| (e.charged_credits.max(e.cost.unwrap_or(0.0)), e.clone()))
                    .collect();
                let prior_only: Vec<CallLogEntry> =
                    all_held.iter().map(|(_, e)| e.clone()).collect();
                let mut matched = vec![false; prior_only.len()];
                for (held, entry) in &self.pending_unwritten {
                    if let Some(pos) = prior_only
                        .iter()
                        .enumerate()
                        .position(|(i, p)| !matched[i] && p == entry)
                    {
                        matched[pos] = true;
                    } else {
                        all_held.push((*held, entry.clone()));
                    }
                }

                let total_unrecorded: f64 = all_held.iter().map(|(h, _)| *h).sum();
                let entries_summary = all_held
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

impl Drop for CallLedger<'_> {
    fn drop(&mut self) {
        if let Some(file) = self.run_resv_lock_file.take() {
            let _ = file.unlock();
            drop(file);
            let _ = fs::remove_file(&self.run_resv_lock_path);
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

    let turn_lock = TurnLock::acquire(&config.game_dir, redaction_key)?;

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
        &turn_lock,
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

    drop(ledger);
    drop(turn_lock);
    Ok(TurnSummary { reports })
}

#[allow(clippy::too_many_arguments)]
fn run_single_openrouter_seat(
    config: &TurnConfig,
    guild: &str,
    seat: &OpenRouterSeat,
    wrapper_text: Option<&str>,
    transport: &dyn Transport,
    ledger: &Mutex<CallLedger<'_>>,
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
        let reservation_template = reserved_call_entry(
            config,
            guild,
            attempt,
            trigger,
            &seat.model,
            &request_body_sha256,
            seat.max_cost_per_call,
        );

        let reservation_id = {
            let mut guard = ledger.lock().unwrap_or_else(|p| p.into_inner());
            match guard.try_reserve(seat.max_cost_per_call, &reservation_template, redaction_key) {
                Ok(PreCallDecision::Reserved(resv_id)) => resv_id,
                Ok(PreCallDecision::LockCompromised(lock_err)) => {
                    return SeatOutcome::SeatError {
                        detail: lock_err.to_string(),
                    };
                }
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
        };

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
                    reservation_id: Some(reservation_id.clone()),
                };
                {
                    let mut guard = ledger.lock().unwrap_or_else(|p| p.into_inner());
                    if let Err(e) = guard.finish_call_and_record(
                        &reservation_id,
                        seat.max_cost_per_call,
                        &entry,
                        over_estimate,
                        redaction_key,
                    ) {
                        return SeatOutcome::SeatError {
                            detail: e.to_string(),
                        };
                    }
                    if let Some(lock_err) = &guard.lock_compromised {
                        return SeatOutcome::SeatError {
                            detail: lock_err.to_string(),
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
                    reservation_id: Some(reservation_id.clone()),
                };
                {
                    let mut guard = ledger.lock().unwrap_or_else(|p| p.into_inner());
                    if let Err(e) = guard.finish_call_and_record(
                        &reservation_id,
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
fn reserved_call_entry(
    config: &TurnConfig,
    guild: &str,
    attempt: u8,
    trigger: &str,
    model: &str,
    request_body_sha256: &str,
    max_cost_per_call: f64,
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
        charged_credits: max_cost_per_call,
        latency_ms: 0,
        outcome: "RESERVED".to_owned(),
        error: Some("orphaned pre-call reservation from interrupted run".to_owned()),
        reservation_id: None,
    }
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
        reservation_id: None,
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
