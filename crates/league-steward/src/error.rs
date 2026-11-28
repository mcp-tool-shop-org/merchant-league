//! Structured error types for `league-steward` (`SHIP_GATE.md` `:26`, `scrub_secret_text`).

use std::fmt;

use league_core::{EngineError, Units};
use league_error::{StructuredError, ToStructuredError, scrub_secret_text};
use league_standalone::client::StandAloneError;

use crate::firewall::IngressDropRecord;

/// Errors returned by [`crate::GameTableSteward`], codec builders, and ingress firewall checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StewardError {
    /// Underlying stand-alone `xrpld` harness or RPC error.
    StandAlone(StandAloneError),
    /// Pure-state `TurnEngine` or replay verification error.
    Engine(EngineError),
    /// Ledger index fell backwards beyond [`league_season::LEDGER_LAG_TOLERANCE`].
    LedgerRewind {
        /// Highest validated ledger index previously seen.
        previous_ledger: u64,
        /// Newly observed validated ledger index.
        observed_ledger: u64,
    },
    /// Authority genesis flags or `TransferRate` failed verification on `xrpld`.
    GenesisFlagsMismatch(String),
    /// Pre-allocated `TicketCreate` pool is exhausted or could not be replenished.
    TicketPoolExhausted(String),
    /// `CancelOffer` refused because validated ledger `close_time` reached `cancel_lock`.
    CancelLocked {
        /// Active turn (`1..=32`).
        turn: u8,
        /// Current table Unix timestamp derived from validated `close_time`.
        close_unix: u64,
        /// Configured cancel-lock cutoff timestamp for `turn`.
        lock_unix: u64,
    },
    /// Commit rejected because validated ledger `close_time` exceeded `commit_close_unix`.
    CommitWindowClosed {
        /// Active turn (`1..=32`).
        turn: u8,
        /// Validated commit transaction ledger timestamp.
        close_unix: u64,
        /// Configured commit cutoff timestamp for `turn`.
        deadline_unix: u64,
    },
    /// Scheduled `drand` round was not strictly after `commit_close_unix` (`#51`).
    BeaconNotPostCommit {
        /// Active turn (`1..=32`).
        turn: u8,
        /// Scheduled `drand` quicknet round.
        round: u64,
        /// Round publication Unix timestamp `t_round(round)`.
        round_unix: u64,
        /// Commit cutoff Unix timestamp `T_commit_close(turn)`.
        deadline_unix: u64,
    },
    /// Transaction dropped by the `MemoData` ingress firewall (never retains raw payload bytes).
    FirewallDropped(IngressDropRecord),
    /// Outer `Batch` (`XLS-56`) succeeded or rolled back while an inner transaction failed validation.
    BatchInnerFailed {
        /// Outer `Batch` transaction hash.
        outer_hash: String,
        /// Pre-computed inner transaction hash that failed or lacked `ParentBatchID`.
        inner_hash: String,
        /// Machine-readable reason without secret/memo material.
        detail: String,
    },
    /// Seat trust-line balance on `xrpld` is smaller than the planned `Clawback` debit.
    InsufficientLineForClawback {
        /// Seat index (`0..seat_count`).
        seat: u8,
        /// XRPL currency code.
        currency: String,
        /// Observed live balance in milli-units.
        have_milli: Units,
        /// Required clawback amount in milli-units.
        need_milli: Units,
    },
    /// Live `xrpld` `account_lines` or scoreboard diverged from `TurnEngine` integer milli-units.
    BalanceDivergence {
        /// Seat or Steward description.
        target: String,
        /// XRPL currency code or metric label.
        currency: String,
        /// Expected integer milli-unit balance from `TurnEngine`.
        expected_milli: Units,
        /// Observed integer milli-unit balance from `xrpld`.
        observed_milli: Units,
    },
    /// Event-sourced Archivist mirror integrity, replay digest, or deserialization error.
    ArchivistIntegrity(String),
    /// 3-wave Phoenix state rehydration failed or exceeded the 3-ledger-close budget.
    PhoenixRehydrationFailed(String),
    /// Protocol or encoding invariant failure.
    Protocol(String),
}

impl From<StandAloneError> for StewardError {
    fn from(value: StandAloneError) -> Self {
        Self::StandAlone(value)
    }
}

impl From<EngineError> for StewardError {
    fn from(value: EngineError) -> Self {
        Self::Engine(value)
    }
}

impl fmt::Display for StewardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StandAlone(err) => write!(f, "{err}"),
            Self::Engine(err) => write!(f, "{err}"),
            Self::LedgerRewind {
                previous_ledger,
                observed_ledger,
            } => write!(
                f,
                "validated ledger went backwards from {previous_ledger} to {observed_ledger}"
            ),
            Self::GenesisFlagsMismatch(msg) => {
                write!(f, "authority genesis flags mismatch: {msg}")
            }
            Self::TicketPoolExhausted(msg) => write!(f, "ticket pool error: {msg}"),
            Self::CancelLocked {
                turn,
                close_unix,
                lock_unix,
            } => write!(
                f,
                "turn {turn} cancel lock active at validated close_time {close_unix} (cutoff {lock_unix})"
            ),
            Self::CommitWindowClosed {
                turn,
                close_unix,
                deadline_unix,
            } => write!(
                f,
                "turn {turn} commit window closed at {close_unix} (deadline {deadline_unix})"
            ),
            Self::BeaconNotPostCommit {
                turn,
                round,
                round_unix,
                deadline_unix,
            } => write!(
                f,
                "turn {turn} drand round {round} ({round_unix}) is not strictly after commit deadline {deadline_unix}"
            ),
            Self::FirewallDropped(rec) => write!(
                f,
                "memo firewall dropped tx {} from {} ({})",
                rec.tx_hash, rec.sender, rec.reason
            ),
            Self::BatchInnerFailed {
                outer_hash,
                inner_hash,
                detail,
            } => write!(
                f,
                "batch {outer_hash} inner tx {inner_hash} verification failed: {detail}"
            ),
            Self::InsufficientLineForClawback {
                seat,
                currency,
                have_milli,
                need_milli,
            } => write!(
                f,
                "seat {seat} trust line {currency} holds {have_milli} milli-units, below clawback {need_milli}"
            ),
            Self::BalanceDivergence {
                target,
                currency,
                expected_milli,
                observed_milli,
            } => write!(
                f,
                "balance divergence on {target} {currency}: expected {expected_milli} milli, observed {observed_milli} milli"
            ),
            Self::ArchivistIntegrity(msg) => write!(f, "archivist mirror error: {msg}"),
            Self::PhoenixRehydrationFailed(msg) => {
                write!(f, "phoenix state rehydration failed: {msg}")
            }
            Self::Protocol(msg) => write!(f, "steward protocol error: {msg}"),
        }
    }
}

impl std::error::Error for StewardError {}

impl StewardError {
    /// Stable `SCREAMING_SNAKE_CASE` error identifier (`SHIP_GATE.md` `:26`).
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::StandAlone(err) => err.code(),
            Self::Engine(err) => err.code(),
            Self::LedgerRewind { .. } => "STEWARD_LEDGER_REWIND",
            Self::GenesisFlagsMismatch(_) => "STEWARD_GENESIS_FLAGS",
            Self::TicketPoolExhausted(_) => "STEWARD_TICKET_POOL",
            Self::CancelLocked { .. } => "STEWARD_CANCEL_LOCKED",
            Self::CommitWindowClosed { .. } => "STEWARD_COMMIT_CLOSED",
            Self::BeaconNotPostCommit { .. } => "STEWARD_BEACON_PRE_COMMIT",
            Self::FirewallDropped(_) => "STEWARD_MEMO_DROPPED",
            Self::BatchInnerFailed { .. } => "STEWARD_BATCH_INNER_FAILED",
            Self::InsufficientLineForClawback { .. } => "STEWARD_CLAWBACK_SHORTFALL",
            Self::BalanceDivergence { .. } => "STEWARD_BALANCE_DIVERGENCE",
            Self::ArchivistIntegrity(_) => "STEWARD_ARCHIVIST_INTEGRITY",
            Self::PhoenixRehydrationFailed(_) => "STEWARD_PHOENIX_FAILED",
            Self::Protocol(_) => "STEWARD_PROTOCOL",
        }
    }
}

impl ToStructuredError for StewardError {
    fn to_structured_error(&self) -> StructuredError {
        match self {
            Self::StandAlone(err) => err.to_structured_error(),
            Self::Engine(err) => err.to_structured_error(),
            Self::LedgerRewind { .. } => StructuredError::new(
                self.code(),
                scrub_secret_text(&self.to_string()),
                "halt turn execution and trigger Phoenix state rehydration against the fresh ledger",
                false,
            ),
            Self::GenesisFlagsMismatch(detail) => StructuredError::new(
                self.code(),
                scrub_secret_text(&self.to_string()),
                "enable RequireAuth, Clawback, Locking, DefaultRipple, and DisallowXRP on Authority with TransferRate unset before issuing trust lines",
                false,
            )
            .with_cause(detail),
            Self::TicketPoolExhausted(detail) => StructuredError::new(
                self.code(),
                scrub_secret_text(&self.to_string()),
                "allocate fresh tickets with TicketCreate before submitting concurrent turn transactions",
                true,
            )
            .with_cause(detail),
            Self::CancelLocked { .. } => StructuredError::new(
                self.code(),
                scrub_secret_text(&self.to_string()),
                "cancel open offers before validated close_time reaches the turn cancel-lock cutoff",
                false,
            ),
            Self::CommitWindowClosed { .. } => StructuredError::new(
                self.code(),
                scrub_secret_text(&self.to_string()),
                "submit Phase-2 commitments in a validated ledger at or before commit_close_unix",
                false,
            ),
            Self::BeaconNotPostCommit { .. } => StructuredError::new(
                self.code(),
                scrub_secret_text(&self.to_string()),
                "schedule DrandSchedule so t_round(drand_round) is strictly greater than commit_close_unix",
                false,
            ),
            Self::FirewallDropped(rec) => StructuredError::new(
                self.code(),
                scrub_secret_text(&self.to_string()),
                "send Phase-2 commits solely from seated accounts using MemoType ml.commit.v2 and 64-hex SHA-256 MemoData",
                false,
            )
            .with_cause(rec.reason.code()),
            Self::BatchInnerFailed { detail, .. } => StructuredError::new(
                self.code(),
                scrub_secret_text(&self.to_string()),
                "inspect every inner Batch transaction result and ParentBatchID; aborted tfAllOrNothing batches leave balances untouched",
                false,
            )
            .with_cause(detail),
            Self::InsufficientLineForClawback { .. } => StructuredError::new(
                self.code(),
                scrub_secret_text(&self.to_string()),
                "read live account_lines balances before constructing Clawback settlement legs",
                false,
            ),
            Self::BalanceDivergence { .. } => StructuredError::new(
                self.code(),
                scrub_secret_text(&self.to_string()),
                "reconcile account_lines against TurnEngine integer milli-units before advancing the turn",
                false,
            ),
            Self::ArchivistIntegrity(msg) => StructuredError::new(
                self.code(),
                scrub_secret_text(&self.to_string()),
                "verify the event-sourced Archivist mirror hash chain and post-sweep state digests",
                false,
            )
            .with_cause(msg),
            Self::PhoenixRehydrationFailed(msg) => StructuredError::new(
                self.code(),
                scrub_secret_text(&self.to_string()),
                "inspect the 3-wave Phoenix rehydration batches, tickets, and 3-ledger close budget",
                false,
            )
            .with_cause(msg),
            Self::Protocol(msg) => StructuredError::new(
                self.code(),
                scrub_secret_text(&self.to_string()),
                "check Steward protocol invariants and transaction encoding parameters",
                false,
            )
            .with_cause(msg),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::firewall::IngressDropReason;

    #[test]
    fn steward_error_codes_structured_shape_and_secret_scrubbing() {
        let ed_seed = "sEdSKaCy2JT7JaM7v95H9SxkhP9wS2r";
        let secp_seed = "sp5fghtJtpUorTwvof1NpDXAzNwf5";
        let drop_rec = IngressDropRecord {
            tx_hash: "ABCD".into(),
            sender: "rOutsider".into(),
            reason: IngressDropReason::UnseatedSender,
        };
        let cases = [
            (
                StewardError::StandAlone(StandAloneError::Transport(format!("Bearer {ed_seed}"))),
                "STANDALONE_TRANSPORT",
                true,
            ),
            (
                StewardError::Engine(EngineError::CancelLocked),
                "CANCEL_LOCKED",
                false,
            ),
            (
                StewardError::LedgerRewind {
                    previous_ledger: 2000,
                    observed_ledger: 5,
                },
                "STEWARD_LEDGER_REWIND",
                false,
            ),
            (
                StewardError::GenesisFlagsMismatch(format!("bad {ed_seed}")),
                "STEWARD_GENESIS_FLAGS",
                false,
            ),
            (
                StewardError::TicketPoolExhausted("empty".into()),
                "STEWARD_TICKET_POOL",
                true,
            ),
            (
                StewardError::CancelLocked {
                    turn: 3,
                    close_unix: 100,
                    lock_unix: 90,
                },
                "STEWARD_CANCEL_LOCKED",
                false,
            ),
            (
                StewardError::CommitWindowClosed {
                    turn: 3,
                    close_unix: 105,
                    deadline_unix: 100,
                },
                "STEWARD_COMMIT_CLOSED",
                false,
            ),
            (
                StewardError::BeaconNotPostCommit {
                    turn: 3,
                    round: 10,
                    round_unix: 100,
                    deadline_unix: 100,
                },
                "STEWARD_BEACON_PRE_COMMIT",
                false,
            ),
            (
                StewardError::FirewallDropped(drop_rec),
                "STEWARD_MEMO_DROPPED",
                false,
            ),
            (
                StewardError::BatchInnerFailed {
                    outer_hash: "OUT".into(),
                    inner_hash: "IN".into(),
                    detail: format!("tecFAIL {secp_seed}"),
                },
                "STEWARD_BATCH_INNER_FAILED",
                false,
            ),
            (
                StewardError::InsufficientLineForClawback {
                    seat: 1,
                    currency: "R00".into(),
                    have_milli: 500,
                    need_milli: 1_000,
                },
                "STEWARD_CLAWBACK_SHORTFALL",
                false,
            ),
            (
                StewardError::BalanceDivergence {
                    target: "seat 0".into(),
                    currency: "CRED".into(),
                    expected_milli: 1_000,
                    observed_milli: 999,
                },
                "STEWARD_BALANCE_DIVERGENCE",
                false,
            ),
            (
                StewardError::ArchivistIntegrity(format!("tampered {ed_seed}")),
                "STEWARD_ARCHIVIST_INTEGRITY",
                false,
            ),
            (
                StewardError::PhoenixRehydrationFailed(format!("budget exceeded {secp_seed}")),
                "STEWARD_PHOENIX_FAILED",
                false,
            ),
            (
                StewardError::Protocol(format!("OPENROUTER_API_KEY=sk-or-secret999 {ed_seed}")),
                "STEWARD_PROTOCOL",
                false,
            ),
        ];

        for (err, expected_code, expect_retryable) in cases {
            assert_eq!(err.code(), expected_code);
            let se = err.to_structured_error();
            assert_eq!(se.code, expected_code);
            assert!(!se.hint.is_empty());
            assert_eq!(se.retryable, expect_retryable);
            let json = se
                .to_json_string()
                .unwrap_or_else(|e| panic!("json serialize: {e}"));
            let text = se.format_text();
            for rendered in [&json, &text, &se.message] {
                assert!(!rendered.contains(ed_seed), "leaked ed seed: {rendered}");
                assert!(
                    !rendered.contains(secp_seed),
                    "leaked secp seed: {rendered}"
                );
                assert!(
                    !rendered.contains("sk-or-secret999"),
                    "leaked api key: {rendered}"
                );
            }
        }
    }
}
