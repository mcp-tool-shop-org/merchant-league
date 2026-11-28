//! Authenticated off-ledger whisper relay, closed typed [`IntentHeader`] ingress,
//! `[UNTRUSTED_MESSAGE]` escaping, and discrete sub-round tick-barrier release for
//! `TableMode::Game` and opt-in `TableMode::Club` (`docs/game-mode-protocol.md` §4).
//!
//! Track-1 `TableMode::Eval` stays strictly frozen on `talk_rules_v2` and is refused.

use crate::error::RelayError;
use crate::wording::{
    MAX_INTENT_HEADER_DEPTH, MAX_WHISPER_CHARS, MAX_WHISPER_WORDS, NegotiationInputs,
    UntrustedWhisper, json_escape_untrusted_str, sanitize_whisper,
};
use league_seat_types::{IntentHeader, TableId, TableMode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// Domain separator prefixed to every Ed25519 whisper envelope signing preimage.
pub const WHISPER_ENVELOPE_DOMAIN_TAG: &[u8] = b"merchant-league/v2/whisper-envelope\0";

/// Exact keys accepted on an incoming signed whisper envelope JSON object.
const ENVELOPE_KEYS: [&str; 9] = [
    "table_id",
    "turn",
    "sub_round",
    "sender_seat",
    "recipient_seat",
    "envelope_seq",
    "header",
    "untrusted_body",
    "signature",
];

/// Registered Ed25519 session public key for one seated player at a table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeatSessionPubkey {
    /// Zero-based seat index (`0..seat_count`).
    pub seat: u8,
    /// Seated guild identifier matching `seats.json`.
    pub guild: String,
    /// 64-hex Ed25519 verifying key registered for this seat's off-ledger whispers.
    pub ed25519_pubkey_hex: String,
}

/// Table configuration and Ed25519 session key registry for [`WhisperRelayCoordinator`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WhisperTableConfig {
    /// Concrete table identifier (`TableId`).
    pub table_id: TableId,
    /// Table mode (`Game` or opt-in `Club`; `Eval` is refused).
    pub mode: TableMode,
    /// Number of seated players (`2..=6`).
    pub seat_count: u8,
    /// Registered seat public keys covering `0..seat_count`.
    pub seats: Vec<SeatSessionPubkey>,
}

impl WhisperTableConfig {
    /// Validate mode (`Eval` refused), seat count, unique guild names, and Ed25519 pubkeys.
    pub fn validate(&self) -> Result<(), RelayError> {
        if self.mode == TableMode::Eval {
            return Err(RelayError::WhisperRelayEvalForbidden(
                "TableMode::Eval is frozen on talk_rules_v2 and cannot use whisper_relay"
                    .to_owned(),
            ));
        }
        if !(2..=6).contains(&self.seat_count) {
            return Err(RelayError::SeatConfig(format!(
                "whisper table seat_count must be in 2..=6, got {}",
                self.seat_count
            )));
        }
        if self.seats.len() != usize::from(self.seat_count) {
            return Err(RelayError::SeatConfig(format!(
                "whisper table expected {} seats, got {}",
                self.seat_count,
                self.seats.len()
            )));
        }
        let mut seen_seats = BTreeSet::new();
        let mut seen_guilds = BTreeSet::new();
        for entry in &self.seats {
            if entry.seat >= self.seat_count {
                return Err(RelayError::SeatConfig(format!(
                    "seat index {} out of range for seat_count {}",
                    entry.seat, self.seat_count
                )));
            }
            if !seen_seats.insert(entry.seat) {
                return Err(RelayError::SeatConfig(format!(
                    "duplicate seat index {} in whisper table keys",
                    entry.seat
                )));
            }
            let trimmed = entry.guild.trim();
            if trimmed.is_empty() || trimmed != entry.guild {
                return Err(RelayError::SeatConfig(
                    "whisper seat guild must be non-empty without leading/trailing whitespace"
                        .to_owned(),
                ));
            }
            let lower = trimmed.to_ascii_lowercase();
            if !seen_guilds.insert(lower) {
                return Err(RelayError::SeatConfig(format!(
                    "duplicate guild '{}' in whisper table keys",
                    entry.guild
                )));
            }
            decode_ed25519_pubkey_bytes(&entry.ed25519_pubkey_hex)?;
        }
        Ok(())
    }

    fn seat_entry(&self, seat: u8) -> Option<&SeatSessionPubkey> {
        self.seats.iter().find(|s| s.seat == seat)
    }
}

/// Unsigned off-ledger whisper payload before Ed25519 signing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnsignedWhisperEnvelope {
    /// Target table identifier.
    pub table_id: TableId,
    /// Turn number (`>= 1`).
    pub turn: u8,
    /// Discrete communication sub-round (`>= 1`).
    pub sub_round: u8,
    /// Sending seat index (`0..seat_count`).
    pub sender_seat: u8,
    /// Receiving seat index (`0..seat_count`, `!= sender_seat`).
    pub recipient_seat: u8,
    /// Strictly monotonic per-`(sender_seat, recipient_seat)` sequence number (`>= 1`).
    pub envelope_seq: u64,
    /// Closed 5-variant typed [`IntentHeader`].
    pub header: IntentHeader,
    /// Untrusted free-text body (`<= 120` words, `<= 1_800` chars combined with header).
    pub untrusted_body: String,
}

/// Signed off-ledger whisper envelope carrying the sender's Ed25519 hex signature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedWhisperEnvelope {
    /// Target table identifier.
    pub table_id: TableId,
    /// Turn number (`>= 1`).
    pub turn: u8,
    /// Discrete communication sub-round (`>= 1`).
    pub sub_round: u8,
    /// Sending seat index (`0..seat_count`).
    pub sender_seat: u8,
    /// Receiving seat index (`0..seat_count`, `!= sender_seat`).
    pub recipient_seat: u8,
    /// Strictly monotonic per-`(sender_seat, recipient_seat)` sequence number (`>= 1`).
    pub envelope_seq: u64,
    /// Closed 5-variant typed [`IntentHeader`].
    pub header: IntentHeader,
    /// Untrusted free-text body.
    pub untrusted_body: String,
    /// 128-hex lowercase or uppercase Ed25519 signature over [`whisper_envelope_signing_bytes`].
    pub signature: String,
}

/// Structured reason an incoming whisper envelope was silently dropped before queuing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum WhisperDropReason {
    /// Envelope JSON missed required fields, added extra fields, or used wrong scalar types.
    MalformedEnvelope,
    /// `header` JSON nesting exceeded [`MAX_INTENT_HEADER_DEPTH`] (`8`).
    HeaderDepthExceeded,
    /// `header` failed closed 5-variant [`IntentHeader`] parsing, had extra nested keys, or failed semantic validation.
    InvalidIntentHeader,
    /// `(table_id, turn, sub_round)` did not match the coordinator's active sub-round window.
    WindowMismatch,
    /// `sender_seat` or `recipient_seat` was out of range or equal.
    InvalidSeat,
    /// Ed25519 signature failed strict verification against `sender_seat`'s registered public key.
    InvalidSignature,
    /// `envelope_seq` was zero, replayed, or out of order for `(sender_seat, recipient_seat)`.
    NonMonotonicSeq,
    /// Sender already queued a validated whisper for this recipient in the current sub-round.
    DuplicateSubRoundPair,
}

/// Sanitized telemetry record for one dropped envelope (retains zero untrusted payload text).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WhisperDropRecord {
    /// Sender seat index if present and syntactically valid.
    pub sender_seat: Option<u8>,
    /// Recipient seat index if present and syntactically valid.
    pub recipient_seat: Option<u8>,
    /// Envelope sequence number if present and syntactically valid.
    pub envelope_seq: Option<u64>,
    /// Fail-closed drop classification.
    pub reason: WhisperDropReason,
}

/// One validated, capped whisper released at a discrete sub-round tick barrier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveredWhisper {
    /// Validated sending seat index.
    pub sender_seat: u8,
    /// Validated receiving seat index.
    pub recipient_seat: u8,
    /// Validated sender guild name.
    pub from_guild: String,
    /// Turn number.
    pub turn: u8,
    /// Sub-round number.
    pub sub_round: u8,
    /// Verified monotonic `envelope_seq`.
    pub envelope_seq: u64,
    /// Validated closed 5-variant [`IntentHeader`].
    pub header: IntentHeader,
    /// Word- and character-capped body text (before prompt-line escaping).
    pub capped_body: String,
    /// Combined word count across `header` and `capped_body`.
    pub word_count: usize,
    /// `true` if `header` + `untrusted_body` exceeded `MAX_WHISPER_WORDS` or `MAX_WHISPER_CHARS`.
    pub truncated: bool,
}

/// Canonical batch released to one recipient seat at a discrete sub-round tick barrier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecipientTickBatch {
    /// Receiving seat index.
    pub recipient_seat: u8,
    /// Receiving guild name.
    pub recipient_guild: String,
    /// Validated messages ordered deterministically by `sender_seat`.
    pub messages: Vec<DeliveredWhisper>,
    /// Escaped `[UNTRUSTED_MESSAGE] ... [/UNTRUSTED_MESSAGE]` wire lines matching `messages`.
    pub rendered_frames: Vec<String>,
}

/// Complete output produced when a sub-round tick barrier releases queued whispers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubRoundTickRelease {
    /// Concrete table identifier.
    pub table_id: TableId,
    /// Released turn number.
    pub turn: u8,
    /// Released sub-round number.
    pub sub_round: u8,
    /// Number of authenticated whispers released.
    pub accepted_count: usize,
    /// Number of envelopes silently dropped before queuing.
    pub dropped_count: usize,
    /// Sanitized drop records for this sub-round.
    pub drops: Vec<WhisperDropRecord>,
    /// Per-recipient batches in canonical seat order `0..seat_count`.
    pub batches: Vec<RecipientTickBatch>,
}

/// Persisted per-pair monotonic sequence watermark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeqWatermarkEntry {
    /// Sending seat index.
    pub sender_seat: u8,
    /// Receiving seat index.
    pub recipient_seat: u8,
    /// Highest verified `envelope_seq` accepted for `(sender_seat, recipient_seat)`.
    pub last_seq: u64,
}

/// Persisted closed `(turn, sub_round)` identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WhisperWindowKey {
    /// Turn number.
    pub turn: u8,
    /// Sub-round number.
    pub sub_round: u8,
}

/// Serializable coordinator watermark state persisted across CLI sub-round invocations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WhisperRelayStateSnapshot {
    /// Concrete table identifier.
    pub table_id: TableId,
    /// Highest accepted `envelope_seq` per `(sender_seat, recipient_seat)`.
    pub last_seq: Vec<SeqWatermarkEntry>,
    /// Sub-round windows already released at tick barriers.
    pub closed_windows: Vec<WhisperWindowKey>,
}

/// Stateful off-ledger whisper relay coordinator enforcing Ed25519 signatures, monotonic
/// `envelope_seq`, closed 5-variant [`IntentHeader`] parsing, and tick-barrier batching.
pub struct WhisperRelayCoordinator {
    config: WhisperTableConfig,
    active_turn: u8,
    active_sub_round: u8,
    last_seq: BTreeMap<(u8, u8), u64>,
    closed_windows: BTreeSet<WhisperWindowKey>,
    pending: Vec<DeliveredWhisper>,
    pending_pairs: BTreeSet<(u8, u8)>,
    drops: Vec<WhisperDropRecord>,
}

impl WhisperRelayCoordinator {
    /// Create a new coordinator for `config` at `(initial_turn, initial_sub_round)`.
    ///
    /// Refuses `TableMode::Eval` (`WhisperRelayEvalForbidden`) and zero turn/sub-round numbers.
    pub fn new(
        config: WhisperTableConfig,
        initial_turn: u8,
        initial_sub_round: u8,
    ) -> Result<Self, RelayError> {
        config.validate()?;
        if initial_turn == 0 || initial_sub_round == 0 {
            return Err(RelayError::Usage(
                "whisper relay turn and sub_round must both be >= 1".to_owned(),
            ));
        }
        Ok(Self {
            config,
            active_turn: initial_turn,
            active_sub_round: initial_sub_round,
            last_seq: BTreeMap::new(),
            closed_windows: BTreeSet::new(),
            pending: Vec::new(),
            pending_pairs: BTreeSet::new(),
            drops: Vec::new(),
        })
    }

    /// Restore a coordinator from a persisted [`WhisperRelayStateSnapshot`] for `(turn, sub_round)`.
    pub fn from_snapshot(
        config: WhisperTableConfig,
        turn: u8,
        sub_round: u8,
        snapshot: &WhisperRelayStateSnapshot,
    ) -> Result<Self, RelayError> {
        if snapshot.table_id != config.table_id {
            return Err(RelayError::IncompatibleSetup(format!(
                "whisper state table_id {} != configured {}",
                snapshot.table_id.0, config.table_id.0
            )));
        }
        let mut coord = Self::new(config, turn, sub_round)?;
        for entry in &snapshot.last_seq {
            if entry.sender_seat >= coord.config.seat_count
                || entry.recipient_seat >= coord.config.seat_count
                || entry.sender_seat == entry.recipient_seat
                || entry.last_seq == 0
            {
                return Err(RelayError::CorruptLog(
                    "whisper state snapshot contains invalid sequence watermark".to_owned(),
                ));
            }
            if coord
                .last_seq
                .insert((entry.sender_seat, entry.recipient_seat), entry.last_seq)
                .is_some()
            {
                return Err(RelayError::CorruptLog(
                    "whisper state snapshot contains duplicate pair watermark".to_owned(),
                ));
            }
        }
        for win in &snapshot.closed_windows {
            if win.turn == 0 || win.sub_round == 0 {
                return Err(RelayError::CorruptLog(
                    "whisper state snapshot contains zero turn/sub_round".to_owned(),
                ));
            }
            coord.closed_windows.insert(*win);
        }
        let current_win = WhisperWindowKey { turn, sub_round };
        if coord.closed_windows.contains(&current_win) {
            return Err(RelayError::Usage(format!(
                "whisper sub-round (turn={}, sub_round={}) has already been released",
                turn, sub_round
            )));
        }
        Ok(coord)
    }

    /// Export monotonic sequence watermarks and closed sub-round windows.
    #[must_use]
    pub fn to_snapshot(&self) -> WhisperRelayStateSnapshot {
        let last_seq = self
            .last_seq
            .iter()
            .map(
                |(&(sender_seat, recipient_seat), &last_seq)| SeqWatermarkEntry {
                    sender_seat,
                    recipient_seat,
                    last_seq,
                },
            )
            .collect();
        let closed_windows = self.closed_windows.iter().copied().collect();
        WhisperRelayStateSnapshot {
            table_id: self.config.table_id,
            last_seq,
            closed_windows,
        }
    }

    /// Table configuration bound to this coordinator.
    #[must_use]
    pub fn config(&self) -> &WhisperTableConfig {
        &self.config
    }

    /// Active `(turn, sub_round)` window currently accepting buffered whispers.
    #[must_use]
    pub const fn active_window(&self) -> (u8, u8) {
        (self.active_turn, self.active_sub_round)
    }

    /// Number of validated whispers currently held behind the sub-round tick barrier.
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Drop telemetry recorded in the current un-released sub-round.
    #[must_use]
    pub fn current_drops(&self) -> &[WhisperDropRecord] {
        &self.drops
    }

    /// Open a new `(turn, sub_round)` window after the previous sub-round barrier released.
    pub fn advance_window(&mut self, turn: u8, sub_round: u8) -> Result<(), RelayError> {
        if turn == 0 || sub_round == 0 {
            return Err(RelayError::Usage(
                "whisper relay turn and sub_round must both be >= 1".to_owned(),
            ));
        }
        if !self.pending.is_empty() {
            return Err(RelayError::Usage(
                "cannot advance whisper window before releasing buffered sub-round whispers"
                    .to_owned(),
            ));
        }
        let win = WhisperWindowKey { turn, sub_round };
        if self.closed_windows.contains(&win) {
            return Err(RelayError::Usage(format!(
                "whisper sub-round (turn={turn}, sub_round={sub_round}) was already closed"
            )));
        }
        self.active_turn = turn;
        self.active_sub_round = sub_round;
        self.drops.clear();
        Ok(())
    }

    /// Inspect and buffer one typed [`SignedWhisperEnvelope`] through the exact same firewall as
    /// raw untrusted JSON ingress.
    pub fn ingest_signed_envelope(
        &mut self,
        envelope: &SignedWhisperEnvelope,
    ) -> Result<(), WhisperDropRecord> {
        let Ok(raw) = serde_json::to_value(envelope) else {
            return Err(self.record_drop(
                Some(envelope.sender_seat),
                Some(envelope.recipient_seat),
                Some(envelope.envelope_seq),
                WhisperDropReason::MalformedEnvelope,
            ));
        };
        self.ingest_raw_envelope_value(&raw)
    }

    /// Validate one raw untrusted JSON whisper envelope against schema, depth cap, closed 5-variant
    /// [`IntentHeader`], active window, seat allowlist, Ed25519 signature, and monotonic `envelope_seq`.
    ///
    /// Any invalid packet is dropped silently with a sanitized [`WhisperDropRecord`] and never
    /// advances `last_seq` or enters the pending delivery queue.
    pub fn ingest_raw_envelope_value(
        &mut self,
        raw: &serde_json::Value,
    ) -> Result<(), WhisperDropRecord> {
        let Some(obj) = raw.as_object() else {
            return Err(self.record_drop(None, None, None, WhisperDropReason::MalformedEnvelope));
        };
        let hint_sender = obj
            .get("sender_seat")
            .and_then(|v| v.as_u64())
            .and_then(|n| u8::try_from(n).ok());
        let hint_recipient = obj
            .get("recipient_seat")
            .and_then(|v| v.as_u64())
            .and_then(|n| u8::try_from(n).ok());
        let hint_seq = obj.get("envelope_seq").and_then(|v| v.as_u64());

        if obj.len() != ENVELOPE_KEYS.len() || !ENVELOPE_KEYS.iter().all(|k| obj.contains_key(*k)) {
            return Err(self.record_drop(
                hint_sender,
                hint_recipient,
                hint_seq,
                WhisperDropReason::MalformedEnvelope,
            ));
        }

        let Some(table_id_num) = obj.get("table_id").and_then(|v| v.as_u64()) else {
            return Err(self.record_drop(
                hint_sender,
                hint_recipient,
                hint_seq,
                WhisperDropReason::MalformedEnvelope,
            ));
        };
        let Some(turn) = obj
            .get("turn")
            .and_then(|v| v.as_u64())
            .and_then(|n| u8::try_from(n).ok())
        else {
            return Err(self.record_drop(
                hint_sender,
                hint_recipient,
                hint_seq,
                WhisperDropReason::MalformedEnvelope,
            ));
        };
        let Some(sub_round) = obj
            .get("sub_round")
            .and_then(|v| v.as_u64())
            .and_then(|n| u8::try_from(n).ok())
        else {
            return Err(self.record_drop(
                hint_sender,
                hint_recipient,
                hint_seq,
                WhisperDropReason::MalformedEnvelope,
            ));
        };
        let (Some(sender_seat), Some(recipient_seat), Some(envelope_seq)) =
            (hint_sender, hint_recipient, hint_seq)
        else {
            return Err(self.record_drop(
                hint_sender,
                hint_recipient,
                hint_seq,
                WhisperDropReason::MalformedEnvelope,
            ));
        };
        let Some(untrusted_body) = obj.get("untrusted_body").and_then(|v| v.as_str()) else {
            return Err(self.record_drop(
                Some(sender_seat),
                Some(recipient_seat),
                Some(envelope_seq),
                WhisperDropReason::MalformedEnvelope,
            ));
        };
        let Some(sig_hex) = obj.get("signature").and_then(|v| v.as_str()) else {
            return Err(self.record_drop(
                Some(sender_seat),
                Some(recipient_seat),
                Some(envelope_seq),
                WhisperDropReason::MalformedEnvelope,
            ));
        };
        let Some(raw_header) = obj.get("header") else {
            return Err(self.record_drop(
                Some(sender_seat),
                Some(recipient_seat),
                Some(envelope_seq),
                WhisperDropReason::MalformedEnvelope,
            ));
        };

        if json_nesting_depth(raw_header) > MAX_INTENT_HEADER_DEPTH {
            return Err(self.record_drop(
                Some(sender_seat),
                Some(recipient_seat),
                Some(envelope_seq),
                WhisperDropReason::HeaderDepthExceeded,
            ));
        }

        let Ok(header) = serde_json::from_value::<IntentHeader>(raw_header.clone()) else {
            return Err(self.record_drop(
                Some(sender_seat),
                Some(recipient_seat),
                Some(envelope_seq),
                WhisperDropReason::InvalidIntentHeader,
            ));
        };
        let Ok(canon_header_val) = serde_json::to_value(&header) else {
            return Err(self.record_drop(
                Some(sender_seat),
                Some(recipient_seat),
                Some(envelope_seq),
                WhisperDropReason::InvalidIntentHeader,
            ));
        };
        if !intent_header_nested_keys_exact(raw_header, &canon_header_val) {
            return Err(self.record_drop(
                Some(sender_seat),
                Some(recipient_seat),
                Some(envelope_seq),
                WhisperDropReason::InvalidIntentHeader,
            ));
        }
        if header.validate(self.config.seat_count).is_err() {
            return Err(self.record_drop(
                Some(sender_seat),
                Some(recipient_seat),
                Some(envelope_seq),
                WhisperDropReason::InvalidIntentHeader,
            ));
        }

        if table_id_num != self.config.table_id.0
            || turn != self.active_turn
            || sub_round != self.active_sub_round
        {
            return Err(self.record_drop(
                Some(sender_seat),
                Some(recipient_seat),
                Some(envelope_seq),
                WhisperDropReason::WindowMismatch,
            ));
        }

        if sender_seat >= self.config.seat_count
            || recipient_seat >= self.config.seat_count
            || sender_seat == recipient_seat
        {
            return Err(self.record_drop(
                Some(sender_seat),
                Some(recipient_seat),
                Some(envelope_seq),
                WhisperDropReason::InvalidSeat,
            ));
        }
        let Some(sender_entry) = self.config.seat_entry(sender_seat) else {
            return Err(self.record_drop(
                Some(sender_seat),
                Some(recipient_seat),
                Some(envelope_seq),
                WhisperDropReason::InvalidSeat,
            ));
        };

        let unsigned = UnsignedWhisperEnvelope {
            table_id: TableId(table_id_num),
            turn,
            sub_round,
            sender_seat,
            recipient_seat,
            envelope_seq,
            header: header.clone(),
            untrusted_body: untrusted_body.to_owned(),
        };
        if verify_envelope_signature(&unsigned, &sender_entry.ed25519_pubkey_hex, sig_hex).is_err()
        {
            return Err(self.record_drop(
                Some(sender_seat),
                Some(recipient_seat),
                Some(envelope_seq),
                WhisperDropReason::InvalidSignature,
            ));
        }

        let pair = (sender_seat, recipient_seat);
        let prev_seq = self.last_seq.get(&pair).copied().unwrap_or(0);
        if envelope_seq == 0 || envelope_seq <= prev_seq {
            return Err(self.record_drop(
                Some(sender_seat),
                Some(recipient_seat),
                Some(envelope_seq),
                WhisperDropReason::NonMonotonicSeq,
            ));
        }
        if self.pending_pairs.contains(&pair) {
            return Err(self.record_drop(
                Some(sender_seat),
                Some(recipient_seat),
                Some(envelope_seq),
                WhisperDropReason::DuplicateSubRoundPair,
            ));
        }

        // Enforce word/char limits across `header` + `untrusted_body`.
        let sanitized = sanitize_whisper(&UntrustedWhisper {
            from_guild: sender_entry.guild.clone(),
            intent_header: Some(canon_header_val),
            body: untrusted_body.to_owned(),
        });

        self.last_seq.insert(pair, envelope_seq);
        self.pending_pairs.insert(pair);
        self.pending.push(DeliveredWhisper {
            sender_seat,
            recipient_seat,
            from_guild: sender_entry.guild.clone(),
            turn,
            sub_round,
            envelope_seq,
            header,
            capped_body: sanitized.capped_body,
            word_count: sanitized.word_count,
            truncated: sanitized.truncated,
        });
        Ok(())
    }

    /// Release all validated whispers held for the current discrete sub-round tick barrier,
    /// sorted deterministically by `sender_seat` per recipient seat.
    pub fn release_sub_round(&mut self) -> Result<SubRoundTickRelease, RelayError> {
        let win = WhisperWindowKey {
            turn: self.active_turn,
            sub_round: self.active_sub_round,
        };
        if self.closed_windows.contains(&win) {
            return Err(RelayError::Usage(format!(
                "whisper sub-round (turn={}, sub_round={}) was already released",
                self.active_turn, self.active_sub_round
            )));
        }
        self.closed_windows.insert(win);

        let pending = std::mem::take(&mut self.pending);
        self.pending_pairs.clear();
        let drops = std::mem::take(&mut self.drops);

        let mut by_recipient: BTreeMap<u8, Vec<DeliveredWhisper>> = BTreeMap::new();
        for msg in pending {
            by_recipient
                .entry(msg.recipient_seat)
                .or_default()
                .push(msg);
        }

        let mut seat_entries: Vec<&SeatSessionPubkey> = self.config.seats.iter().collect();
        seat_entries.sort_by_key(|s| s.seat);

        let mut accepted_count = 0usize;
        let mut batches = Vec::with_capacity(seat_entries.len());
        for seat_entry in seat_entries {
            let mut msgs = by_recipient.remove(&seat_entry.seat).unwrap_or_default();
            msgs.sort_by_key(|m| m.sender_seat);
            accepted_count += msgs.len();
            let rendered_frames = msgs.iter().map(render_untrusted_message_frame).collect();
            batches.push(RecipientTickBatch {
                recipient_seat: seat_entry.seat,
                recipient_guild: seat_entry.guild.clone(),
                messages: msgs,
                rendered_frames,
            });
        }

        Ok(SubRoundTickRelease {
            table_id: self.config.table_id,
            turn: self.active_turn,
            sub_round: self.active_sub_round,
            accepted_count,
            dropped_count: drops.len(),
            drops,
            batches,
        })
    }

    fn record_drop(
        &mut self,
        sender_seat: Option<u8>,
        recipient_seat: Option<u8>,
        envelope_seq: Option<u64>,
        reason: WhisperDropReason,
    ) -> WhisperDropRecord {
        let rec = WhisperDropRecord {
            sender_seat,
            recipient_seat,
            envelope_seq,
            reason,
        };
        self.drops.push(rec.clone());
        rec
    }
}

/// Convert one recipient's released tick batch into [`NegotiationInputs`] for `league-relay turn`.
///
/// Neutralizes `[` / `]` / `===` markers in `body` without inflating word count so `sanitize_whisper`
/// enforces the exact [`MAX_WHISPER_WORDS`] / [`MAX_WHISPER_CHARS`] budget.
pub fn batch_to_negotiation_inputs(
    batch: &RecipientTickBatch,
    batna: &str,
) -> Result<NegotiationInputs, RelayError> {
    if batna.trim().is_empty() {
        return Err(RelayError::Io(
            "negotiation inputs require a non-empty 'batna' fallback line".to_owned(),
        ));
    }
    let mut whispers = Vec::with_capacity(batch.messages.len());
    for msg in &batch.messages {
        let header_val = serde_json::to_value(&msg.header).map_err(|e| {
            RelayError::Io(format!(
                "failed to serialize IntentHeader for negotiation: {e}"
            ))
        })?;
        let neutralized_body = neutralize_frame_markers_preserving_words(&msg.capped_body);
        whispers.push(UntrustedWhisper {
            from_guild: msg.from_guild.clone(),
            intent_header: Some(header_val),
            body: neutralized_body,
        });
    }
    Ok(NegotiationInputs {
        batna: batna.trim().to_owned(),
        whispers,
    })
}

/// Derive the 64-hex Ed25519 verifying key corresponding to a 32-byte secret key.
#[must_use]
pub fn ed25519_pubkey_hex_from_secret(secret_key_bytes: &[u8; 32]) -> String {
    use ed25519_dalek::SigningKey;
    let signing_key = SigningKey::from_bytes(secret_key_bytes);
    hex::encode(signing_key.verifying_key().to_bytes())
}

/// Compute the deterministic domain-separated signing preimage for [`UnsignedWhisperEnvelope`].
pub fn whisper_envelope_signing_bytes(
    unsigned: &UnsignedWhisperEnvelope,
) -> Result<Vec<u8>, RelayError> {
    if unsigned.turn == 0 || unsigned.sub_round == 0 {
        return Err(RelayError::Usage(
            "whisper envelope turn and sub_round must be >= 1".to_owned(),
        ));
    }
    let header_json = serde_json::to_vec(&unsigned.header)
        .map_err(|e| RelayError::Usage(format!("failed to encode IntentHeader: {e}")))?;
    let header_digest = Sha256::digest(&header_json);
    let body_bytes = unsigned.untrusted_body.as_bytes();
    let body_len = u64::try_from(body_bytes.len())
        .map_err(|_| RelayError::Usage("whisper body length exceeds u64".to_owned()))?;

    let mut preimage = Vec::with_capacity(
        WHISPER_ENVELOPE_DOMAIN_TAG.len() + 8 + 4 + 8 + 32 + 8 + body_bytes.len(),
    );
    preimage.extend_from_slice(WHISPER_ENVELOPE_DOMAIN_TAG);
    preimage.extend_from_slice(&unsigned.table_id.0.to_be_bytes());
    preimage.push(unsigned.turn);
    preimage.push(unsigned.sub_round);
    preimage.push(unsigned.sender_seat);
    preimage.push(unsigned.recipient_seat);
    preimage.extend_from_slice(&unsigned.envelope_seq.to_be_bytes());
    preimage.extend_from_slice(&header_digest);
    preimage.extend_from_slice(&body_len.to_be_bytes());
    preimage.extend_from_slice(body_bytes);
    Ok(preimage)
}

/// Sign an [`UnsignedWhisperEnvelope`] with the sender seat's 32-byte Ed25519 session secret key.
pub fn sign_whisper_envelope(
    unsigned: &UnsignedWhisperEnvelope,
    secret_key_bytes: &[u8; 32],
) -> Result<SignedWhisperEnvelope, RelayError> {
    use ed25519_dalek::{Signer, SigningKey};
    let preimage = whisper_envelope_signing_bytes(unsigned)?;
    let signing_key = SigningKey::from_bytes(secret_key_bytes);
    let sig = signing_key.sign(&preimage);
    Ok(SignedWhisperEnvelope {
        table_id: unsigned.table_id,
        turn: unsigned.turn,
        sub_round: unsigned.sub_round,
        sender_seat: unsigned.sender_seat,
        recipient_seat: unsigned.recipient_seat,
        envelope_seq: unsigned.envelope_seq,
        header: unsigned.header.clone(),
        untrusted_body: unsigned.untrusted_body.clone(),
        signature: hex::encode(sig.to_bytes()),
    })
}

/// JSON-escape an untrusted string while neutralizing `[UNTRUSTED_MESSAGE]` / `[/UNTRUSTED_MESSAGE]`
/// square brackets, `<` / `>` / full-width angle brackets, `===` section headers, line separators,
/// bidirectional controls, and Unicode 18.0.0 `Default_Ignorable_Code_Point` characters.
#[must_use]
pub fn escape_untrusted_message_json_str(raw: &str) -> String {
    let escaped = json_escape_untrusted_str(raw);
    escaped
        .replace('[', "\\u005b")
        .replace(']', "\\u005d")
        .replace("===", "\\u003d\\u003d\\u003d")
}

/// Render one [`DeliveredWhisper`] wrapped strictly inside `[UNTRUSTED_MESSAGE] ... [/UNTRUSTED_MESSAGE]`.
#[must_use]
pub fn render_untrusted_message_frame(delivered: &DeliveredWhisper) -> String {
    let from_json = escape_untrusted_message_json_str(&delivered.from_guild);
    let body_json = escape_untrusted_message_json_str(&delivered.capped_body);
    let header_json = serde_json::to_string(&delivered.header)
        .map(|s| {
            s.replace('[', "\\u005b")
                .replace(']', "\\u005d")
                .replace('<', "\\u003c")
                .replace('>', "\\u003e")
                .replace("===", "\\u003d\\u003d\\u003d")
        })
        .unwrap_or_else(|_| "null".to_owned());
    let _bounds = (MAX_WHISPER_WORDS, MAX_WHISPER_CHARS);
    format!(
        "[UNTRUSTED_MESSAGE] {{\"from_guild\":{from_json},\"sender_seat\":{},\"sub_round\":{},\"envelope_seq\":{},\"intent_header\":{header_json},\"body\":{body_json},\"truncated\":{}}} [/UNTRUSTED_MESSAGE]",
        delivered.sender_seat, delivered.sub_round, delivered.envelope_seq, delivered.truncated
    )
}

fn neutralize_frame_markers_preserving_words(raw: &str) -> String {
    raw.replace('[', "\u{FF3B}")
        .replace(']', "\u{FF3D}")
        .replace("===", "\u{FF1D}\u{FF1D}\u{FF1D}")
}

fn decode_ed25519_pubkey_bytes(hex_str: &str) -> Result<[u8; 32], RelayError> {
    if hex_str.len() != 64 || !hex_str.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(RelayError::SeatConfig(
            "Ed25519 session public key must be 64 ASCII hex characters".to_owned(),
        ));
    }
    let decoded = hex::decode(hex_str).map_err(|_| {
        RelayError::SeatConfig("Ed25519 session public key contains invalid hex".to_owned())
    })?;
    let bytes: [u8; 32] = decoded.try_into().map_err(|_| {
        RelayError::SeatConfig("Ed25519 session public key must be 32 bytes".to_owned())
    })?;
    {
        use ed25519_dalek::VerifyingKey;
        VerifyingKey::from_bytes(&bytes).map_err(|_| {
            RelayError::SeatConfig(
                "Ed25519 session public key is not a valid curve point".to_owned(),
            )
        })?;
    }
    Ok(bytes)
}

fn verify_envelope_signature(
    unsigned: &UnsignedWhisperEnvelope,
    pubkey_hex: &str,
    sig_hex: &str,
) -> Result<(), ()> {
    if sig_hex.len() != 128 || !sig_hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(());
    }
    let pub_bytes = decode_ed25519_pubkey_bytes(pubkey_hex).map_err(|_| ())?;
    let sig_vec = hex::decode(sig_hex).map_err(|_| ())?;
    let sig_bytes: [u8; 64] = sig_vec.try_into().map_err(|_| ())?;
    let preimage = whisper_envelope_signing_bytes(unsigned).map_err(|_| ())?;

    use ed25519_dalek::{Signature, VerifyingKey};
    let vk = VerifyingKey::from_bytes(&pub_bytes).map_err(|_| ())?;
    let sig = Signature::from_bytes(&sig_bytes);
    vk.verify_strict(&preimage, &sig).map_err(|_| ())
}

fn json_nesting_depth(val: &serde_json::Value) -> usize {
    match val {
        serde_json::Value::Array(items) => {
            1 + items.iter().map(json_nesting_depth).max().unwrap_or(0)
        }
        serde_json::Value::Object(map) => {
            1 + map.values().map(json_nesting_depth).max().unwrap_or(0)
        }
        _ => 0,
    }
}

fn intent_header_nested_keys_exact(raw: &serde_json::Value, canon: &serde_json::Value) -> bool {
    let (Some(raw_outer), Some(canon_outer)) = (raw.as_object(), canon.as_object()) else {
        return false;
    };
    if raw_outer.len() != 1 || canon_outer.len() != 1 {
        return false;
    }
    let Some((variant_key, raw_inner)) = raw_outer.iter().next() else {
        return false;
    };
    let Some(canon_inner) = canon_outer.get(variant_key) else {
        return false;
    };
    let (Some(raw_fields), Some(canon_fields)) = (raw_inner.as_object(), canon_inner.as_object())
    else {
        return false;
    };
    for (k, raw_v) in raw_fields {
        if let Some(canon_v) = canon_fields.get(k)
            && !nested_json_keys_subset(raw_v, canon_v)
        {
            return false;
        }
    }
    true
}

fn nested_json_keys_subset(raw: &serde_json::Value, canon: &serde_json::Value) -> bool {
    match (raw, canon) {
        (serde_json::Value::Object(rm), serde_json::Value::Object(cm)) => {
            for (k, rv) in rm {
                let Some(cv) = cm.get(k) else {
                    return false;
                };
                if !nested_json_keys_subset(rv, cv) {
                    return false;
                }
            }
            true
        }
        (serde_json::Value::Array(ra), serde_json::Value::Array(ca)) => {
            ra.len() == ca.len()
                && ra
                    .iter()
                    .zip(ca.iter())
                    .all(|(rv, cv)| nested_json_keys_subset(rv, cv))
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use league_seat_types::{
        AssetId, BondProposal, BondSettlementMode, CommitmentHash, IntentTarget, OfferSide, SwapLeg,
    };

    fn test_keys() -> ([u8; 32], [u8; 32], [u8; 32]) {
        ([0x11; 32], [0x22; 32], [0x33; 32])
    }

    fn test_config() -> WhisperTableConfig {
        let (k0, k1, k2) = test_keys();
        WhisperTableConfig {
            table_id: TableId(77),
            mode: TableMode::Game,
            seat_count: 3,
            seats: vec![
                SeatSessionPubkey {
                    seat: 0,
                    guild: "Foundry".to_owned(),
                    ed25519_pubkey_hex: ed25519_pubkey_hex_from_secret(&k0),
                },
                SeatSessionPubkey {
                    seat: 1,
                    guild: "Saltworks".to_owned(),
                    ed25519_pubkey_hex: ed25519_pubkey_hex_from_secret(&k1),
                },
                SeatSessionPubkey {
                    seat: 2,
                    guild: "Weavers".to_owned(),
                    ed25519_pubkey_hex: ed25519_pubkey_hex_from_secret(&k2),
                },
            ],
        }
    }

    #[test]
    fn eval_mode_is_refused_for_whisper_relay() {
        let mut cfg = test_config();
        cfg.mode = TableMode::Eval;
        let err = WhisperRelayCoordinator::new(cfg, 1, 1)
            .err()
            .unwrap_or_else(|| panic!("Eval mode must be refused"));
        assert_eq!(err.code(), "WHISPER_RELAY_EVAL_FORBIDDEN");
    }

    #[test]
    fn tick_barrier_releases_all_five_variants_in_canonical_sender_order() {
        let cfg = test_config();
        let (k0, k1, k2) = test_keys();
        let mut coord =
            WhisperRelayCoordinator::new(cfg, 1, 1).unwrap_or_else(|e| panic!("new coord: {e}"));

        // Submit from seat 2 first, then seat 1, both addressed to seat 0.
        let env2 = sign_whisper_envelope(
            &UnsignedWhisperEnvelope {
                table_id: TableId(77),
                turn: 1,
                sub_round: 1,
                sender_seat: 2,
                recipient_seat: 0,
                envelope_seq: 1,
                header: IntentHeader::CreditCheck {
                    send_max_cred: 2_000,
                    deliver_min_cred: Some(1_500),
                    due_turn: Some(2),
                    expiration_turn: 4,
                },
                untrusted_body: "Check for 2000 CRED due turn 2.".to_owned(),
            },
            &k2,
        )
        .unwrap_or_else(|e| panic!("sign env2: {e}"));
        let env1 = sign_whisper_envelope(
            &UnsignedWhisperEnvelope {
                table_id: TableId(77),
                turn: 1,
                sub_round: 1,
                sender_seat: 1,
                recipient_seat: 0,
                envelope_seq: 1,
                header: IntentHeader::SpotOffer {
                    side: OfferSide::Sell,
                    good: AssetId::Raw(1),
                    whole_units: 2,
                    unit_price_cred: 1_800,
                    domain_id: None,
                },
                untrusted_body: "Selling 2 units of Raw(1).".to_owned(),
            },
            &k1,
        )
        .unwrap_or_else(|e| panic!("sign env1: {e}"));

        assert!(coord.ingest_signed_envelope(&env2).is_ok());
        assert!(coord.ingest_signed_envelope(&env1).is_ok());
        assert_eq!(coord.pending_count(), 2);

        let release = coord
            .release_sub_round()
            .unwrap_or_else(|e| panic!("release: {e}"));
        assert_eq!(release.accepted_count, 2);
        assert_eq!(release.dropped_count, 0);
        let seat0_batch = &release.batches[0];
        assert_eq!(seat0_batch.messages.len(), 2);
        // Canonical sender_seat order [1, 2] despite arrival order [2, 1].
        assert_eq!(seat0_batch.messages[0].sender_seat, 1);
        assert_eq!(seat0_batch.messages[1].sender_seat, 2);
        assert!(seat0_batch.rendered_frames[0].starts_with("[UNTRUSTED_MESSAGE] "));
        assert!(seat0_batch.rendered_frames[0].ends_with(" [/UNTRUSTED_MESSAGE]"));

        // Sub-round 2 exercises remaining variants + snapshot round-trip.
        let snap = coord.to_snapshot();
        let mut coord2 = WhisperRelayCoordinator::from_snapshot(test_config(), 1, 2, &snap)
            .unwrap_or_else(|e| panic!("from_snapshot: {e}"));
        for (seq, hdr) in [
            (
                1_u64,
                IntentHeader::AtomicSwap {
                    give_legs: vec![SwapLeg {
                        asset: AssetId::Raw(0),
                        units: 1_000,
                    }],
                    receive_legs: vec![SwapLeg {
                        asset: AssetId::Refined(1),
                        units: 1_000,
                    }],
                    swap_id: Some(5),
                    domain_id: None,
                },
            ),
            (
                2_u64,
                IntentHeader::EscrowDeal {
                    amount_cred: 3_000,
                    finish_after_turn: Some(2),
                    cancel_after_turn: 6,
                    condition_hex: Some(CommitmentHash([0xAB; 32])),
                    bond: Some(BondProposal {
                        good: AssetId::Refined(0),
                        required_units: 1_000,
                        deadline_turn: 5,
                        mode: BondSettlementMode::BuyerDvpPayment,
                    }),
                },
            ),
            (
                3_u64,
                IntentHeader::DiplomaticPledge {
                    target: IntentTarget::VoteForSeat {
                        office_turn: 4,
                        candidate: 1,
                    },
                    honor_weight: Some(400),
                },
            ),
        ] {
            let recipient = if seq == 1 { 1 } else { 2 };
            let e = sign_whisper_envelope(
                &UnsignedWhisperEnvelope {
                    table_id: TableId(77),
                    turn: 1,
                    sub_round: 2,
                    sender_seat: 0,
                    recipient_seat: recipient,
                    envelope_seq: seq,
                    header: hdr,
                    untrusted_body: "Proposal attached.".to_owned(),
                },
                &k0,
            )
            .unwrap_or_else(|e| panic!("sign: {e}"));
            if seq == 3 {
                // Seat 0 -> Seat 2 already used seq 2 in this same sub-round (duplicate pair).
                let err = coord2.ingest_signed_envelope(&e).err();
                assert_eq!(
                    err.map(|d| d.reason),
                    Some(WhisperDropReason::DuplicateSubRoundPair)
                );
            } else {
                assert!(coord2.ingest_signed_envelope(&e).is_ok());
            }
        }
    }

    #[test]
    fn whisper_config_snapshot_window_and_malformed_envelope_guards() {
        let mut cfg = test_config();
        assert!(cfg.validate().is_ok());
        cfg.seat_count = 1;
        assert!(cfg.validate().is_err());

        let mut cfg = test_config();
        cfg.seats.pop();
        assert!(cfg.validate().is_err());

        let mut cfg = test_config();
        cfg.seats[0].seat = 9;
        assert!(cfg.validate().is_err());

        let mut cfg = test_config();
        cfg.seats[1].seat = 0;
        assert!(cfg.validate().is_err());

        let mut cfg = test_config();
        cfg.seats[0].guild = " Foundry ".to_owned();
        assert!(cfg.validate().is_err());

        let mut cfg = test_config();
        cfg.seats[1].guild = "foundry".to_owned();
        assert!(cfg.validate().is_err());

        let mut cfg = test_config();
        cfg.seats[0].ed25519_pubkey_hex = "not_hex".to_owned();
        assert!(cfg.validate().is_err());

        assert!(WhisperRelayCoordinator::new(test_config(), 0, 1).is_err());
        assert!(WhisperRelayCoordinator::new(test_config(), 1, 0).is_err());

        let bad_tid_snap = WhisperRelayStateSnapshot {
            table_id: TableId(999),
            last_seq: Vec::new(),
            closed_windows: Vec::new(),
        };
        assert!(
            WhisperRelayCoordinator::from_snapshot(test_config(), 1, 1, &bad_tid_snap).is_err()
        );

        let bad_seq_snap = WhisperRelayStateSnapshot {
            table_id: TableId(77),
            last_seq: vec![SeqWatermarkEntry {
                sender_seat: 0,
                recipient_seat: 0,
                last_seq: 1,
            }],
            closed_windows: Vec::new(),
        };
        assert!(
            WhisperRelayCoordinator::from_snapshot(test_config(), 1, 1, &bad_seq_snap).is_err()
        );

        let dup_seq_snap = WhisperRelayStateSnapshot {
            table_id: TableId(77),
            last_seq: vec![
                SeqWatermarkEntry {
                    sender_seat: 0,
                    recipient_seat: 1,
                    last_seq: 1,
                },
                SeqWatermarkEntry {
                    sender_seat: 0,
                    recipient_seat: 1,
                    last_seq: 2,
                },
            ],
            closed_windows: Vec::new(),
        };
        assert!(
            WhisperRelayCoordinator::from_snapshot(test_config(), 1, 1, &dup_seq_snap).is_err()
        );

        let zero_win_snap = WhisperRelayStateSnapshot {
            table_id: TableId(77),
            last_seq: Vec::new(),
            closed_windows: vec![WhisperWindowKey {
                turn: 0,
                sub_round: 1,
            }],
        };
        assert!(
            WhisperRelayCoordinator::from_snapshot(test_config(), 1, 1, &zero_win_snap).is_err()
        );

        let mut coord = WhisperRelayCoordinator::new(test_config(), 1, 1)
            .unwrap_or_else(|e| panic!("coord: {e}"));
        assert_eq!(coord.config().table_id, TableId(77));
        assert_eq!(coord.active_window(), (1, 1));
        assert!(coord.current_drops().is_empty());

        for bad_raw in [
            serde_json::json!("not_an_object"),
            serde_json::json!({ "table_id": 77 }),
            serde_json::json!({
                "table_id": "bad",
                "turn": 1,
                "sub_round": 1,
                "sender_seat": 1,
                "recipient_seat": 0,
                "envelope_seq": 1,
                "header": {"SpotOffer":{"side":"Buy","good":{"Raw":0},"whole_units":1,"unit_price_cred":1000}},
                "untrusted_body": "hi",
                "signature": "00"
            }),
            serde_json::json!({
                "table_id": 77,
                "turn": "bad",
                "sub_round": 1,
                "sender_seat": 1,
                "recipient_seat": 0,
                "envelope_seq": 1,
                "header": {"SpotOffer":{"side":"Buy","good":{"Raw":0},"whole_units":1,"unit_price_cred":1000}},
                "untrusted_body": "hi",
                "signature": "00"
            }),
            serde_json::json!({
                "table_id": 77,
                "turn": 1,
                "sub_round": "bad",
                "sender_seat": 1,
                "recipient_seat": 0,
                "envelope_seq": 1,
                "header": {"SpotOffer":{"side":"Buy","good":{"Raw":0},"whole_units":1,"unit_price_cred":1000}},
                "untrusted_body": "hi",
                "signature": "00"
            }),
            serde_json::json!({
                "table_id": 77,
                "turn": 1,
                "sub_round": 1,
                "sender_seat": 1,
                "recipient_seat": 0,
                "envelope_seq": "bad",
                "header": {"SpotOffer":{"side":"Buy","good":{"Raw":0},"whole_units":1,"unit_price_cred":1000}},
                "untrusted_body": "hi",
                "signature": "00"
            }),
            serde_json::json!({
                "table_id": 77,
                "turn": 1,
                "sub_round": 1,
                "sender_seat": 1,
                "recipient_seat": 0,
                "envelope_seq": 1,
                "header": {"SpotOffer":{"side":"Buy","good":{"Raw":0},"whole_units":1,"unit_price_cred":1000}},
                "untrusted_body": 123,
                "signature": "00"
            }),
            serde_json::json!({
                "table_id": 77,
                "turn": 1,
                "sub_round": 1,
                "sender_seat": 1,
                "recipient_seat": 0,
                "envelope_seq": 1,
                "header": {"SpotOffer":{"side":"Buy","good":{"Raw":0},"whole_units":1,"unit_price_cred":1000}},
                "untrusted_body": "hi",
                "signature": 123
            }),
        ] {
            let err = coord.ingest_raw_envelope_value(&bad_raw).err();
            assert_eq!(
                err.map(|d| d.reason),
                Some(WhisperDropReason::MalformedEnvelope)
            );
        }
        assert_eq!(coord.current_drops().len(), 8);

        let (_, k1, _) = test_keys();
        let env1 = sign_whisper_envelope(
            &UnsignedWhisperEnvelope {
                table_id: TableId(77),
                turn: 1,
                sub_round: 1,
                sender_seat: 1,
                recipient_seat: 0,
                envelope_seq: 1,
                header: IntentHeader::SpotOffer {
                    side: OfferSide::Buy,
                    good: AssetId::Raw(0),
                    whole_units: 1,
                    unit_price_cred: 1_000,
                    domain_id: None,
                },
                untrusted_body: "hi".to_owned(),
            },
            &k1,
        )
        .unwrap_or_else(|e| panic!("sign: {e}"));
        assert!(coord.ingest_signed_envelope(&env1).is_ok());
        assert!(coord.advance_window(1, 2).is_err());

        let rel = coord
            .release_sub_round()
            .unwrap_or_else(|e| panic!("release: {e}"));
        assert!(coord.release_sub_round().is_err());
        assert!(coord.advance_window(0, 2).is_err());
        assert!(coord.advance_window(1, 1).is_err());
        assert!(coord.advance_window(1, 2).is_ok());
        assert!(batch_to_negotiation_inputs(&rel.batches[0], "   ").is_err());
        assert!(
            whisper_envelope_signing_bytes(&UnsignedWhisperEnvelope {
                table_id: TableId(77),
                turn: 0,
                sub_round: 1,
                sender_seat: 1,
                recipient_seat: 0,
                envelope_seq: 1,
                header: IntentHeader::SpotOffer {
                    side: OfferSide::Buy,
                    good: AssetId::Raw(0),
                    whole_units: 1,
                    unit_price_cred: 1_000,
                    domain_id: None,
                },
                untrusted_body: "hi".to_owned(),
            })
            .is_err()
        );
    }
}
