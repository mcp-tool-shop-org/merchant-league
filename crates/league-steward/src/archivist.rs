//! Event-sourced Archivist state mirror (`docs/design-v2.md` §7.2,
//! `docs/game-mode-protocol.md`).
//!
//! Records every table transition in a rolling SHA-256 hash-chained log without
//! retaining secret signing keys, verifies post-Sweep `EngineState` digests on
//! fold, and materializes exact pre-wipe state for 3-wave Phoenix rehydration.

use std::collections::{BTreeMap, BTreeSet};

use league_core::{
    CharterRole, CommitmentHash, CreditTier, LoggedTurn, MandateReveal, MarketAction, OrderReveal,
    SealedMandateDeal, Seat, SeatIdentity, TableSpec, TurnEngine, Units,
    compute_mandate_commitment, derive_drand_beacon,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::error::StewardError;
use crate::firewall::IngressDropRecord;
use crate::steward::StewardTurnWindow;

const ARCHIVIST_CHAIN_DOMAIN: &[u8] = b"ml.archivist.v1";

/// Mirrored live native `PREIMAGE-SHA-256` `TokenEscrow` vault tracked for Phoenix remapping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MirroredNativeEscrow {
    /// Canonical engine escrow ID (`next_escrow_id`).
    pub escrow_id: u64,
    /// Owner seat index (`0..seat_count`).
    pub owner_seat: u8,
    /// Destination seat index (`0..seat_count`).
    pub dest_seat: u8,
    /// Escrowed `CRED` amount in milli-units.
    pub amount_milli: Units,
    /// Ledger `Sequence` / `TicketSequence` on the current XRPL node.
    pub xrpl_sequence: u32,
    /// DER crypto-condition hex (`PREIMAGE-SHA-256`).
    pub condition_hex: String,
    /// Optional earliest turn when `EscrowFinish` may run.
    pub finish_after_turn: Option<u8>,
    /// Turn when the unfulfilled escrow expires for `EscrowCancel`.
    pub cancel_after_turn: u8,
    /// Current-node Ripple-epoch `FinishAfter`, if any.
    pub finish_after_ripple: Option<u64>,
    /// Current-node Ripple-epoch `CancelAfter`.
    pub cancel_after_ripple: u64,
}

/// Mirrored live native `CheckCreate` object tracked for Phoenix remapping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MirroredNativeCheck {
    /// Canonical engine check ID (`next_check_id`).
    pub check_id_engine: u64,
    /// Drawer seat index (`0..seat_count`).
    pub drawer_seat: u8,
    /// Payee seat index (`0..seat_count`).
    pub payee_seat: u8,
    /// Currency code (`CRED` or `Rxx`/`Mxx`).
    pub currency: String,
    /// Check `SendMax` in milli-units.
    pub amount_milli: Units,
    /// Earliest turn when `CheckCash` runs.
    pub due_turn: u8,
    /// Expiration turn recorded on the engine check.
    pub expiration_turn: u8,
    /// Current-node 64-hex `CheckID` ledger index.
    pub xrpl_check_id: String,
}

/// Mirrored mid-turn 1-drop `PREIMAGE-SHA-256` Phase-2 commit escrow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MirroredCommitEscrow {
    /// Seat index (`0..seat_count`).
    pub seat: u8,
    /// Ledger `Sequence` / `TicketSequence` of the 1-drop commit escrow.
    pub sequence: u32,
    /// DER crypto-condition hex.
    pub condition_hex: String,
    /// DER fulfillment hex.
    pub fulfillment_hex: String,
}

/// Mirrored active Syndicate Charter credential awarded at Turn `6k`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MirroredCharter {
    /// Charter role (`ClearinghouseBroker`, `CustomsInspector`, `OracleAuditor`).
    pub role: CharterRole,
    /// Holding seat index (`0..seat_count`).
    pub holder: u8,
    /// Turn when the charter expires.
    pub expires_turn: u8,
    /// Credential `URI` hex recorded on-ledger.
    pub uri_hex: String,
}

/// Append-only event emitted by [`crate::GameTableSteward`] into [`ArchivistMirror`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArchivistEvent {
    /// Initial table genesis, flags, credentials, trust lines, starting `CRED`, and Turn-1 deal.
    TableBootstrapped {
        /// Table identifier.
        table_id: u64,
        /// Full deterministic [`TableSpec`].
        spec: Box<TableSpec>,
        /// Public chiral layout seed (`dev` / `train` / `probe` / `held_out`).
        layout_seed: u64,
        /// Sealed Turn-1 mandate deal.
        sealed_deal: SealedMandateDeal,
        /// Pre-computed Turn-1 mandate commitments indexed by seat.
        mandate_commits: Vec<Option<CommitmentHash>>,
        /// Pre-dealt Turn-1 mandate reveals indexed by seat.
        mandate_reveals: Vec<Option<MandateReveal>>,
        /// Configured Ripple-epoch turn span for window calculations.
        turn_span_ripple_secs: u64,
        /// Steward classic XRPL address.
        steward_address: String,
        /// Seated classic XRPL addresses (`0..seat_count`).
        seat_addresses: Vec<String>,
        /// Standing Regent delegate classic addresses (`0..seat_count`).
        regent_addresses: Vec<Option<String>>,
        /// Verified `AccountRoot.Flags` on Steward.
        authority_flags: u32,
        /// Active table `PermissionedDomain` 64-hex ID.
        table_domain_id: String,
        /// Validated ledger index right after bootstrap parity check.
        initial_ledger_index: u64,
        /// Initial Turn-1 [`StewardTurnWindow`].
        initial_window: StewardTurnWindow,
        /// Canonical post-Turn-1-commit `TurnEngine::state_digest` SHA-256.
        initial_state_digest: [u8; 32],
        /// `BatchV1_1` outer batches verified during bootstrap.
        bootstrap_batches_verified: usize,
        /// Inner `BatchV1_1` legs verified during bootstrap.
        bootstrap_inner_legs_verified: usize,
    },
    /// Phase-1 `cancel_lock` engaged from a validated `xrpld` ledger header `close_time`.
    CancelLockEngaged {
        /// Active turn (`1..=32`).
        turn: u8,
        /// Index inside `turn_market_actions` at which `begin_cancel_lock` triggered.
        cancel_lock_at: u32,
        /// Validated ledger index that triggered the lock.
        ledger_index: u64,
        /// Validated Ripple-epoch `close_time`.
        ripple_close: u64,
    },
    /// Validated Phase-1 [`MarketAction`] accepted by `TurnEngine` and mirrored on-ledger.
    MarketActionAccepted {
        /// Active turn (`1..=32`).
        turn: u8,
        /// Accepted market action.
        action: MarketAction,
        /// Newly opened native `TokenEscrow` record, if `CreateEscrow` used native XRPL escrow.
        opened_native_escrow: Option<MirroredNativeEscrow>,
        /// Closed native `TokenEscrow` engine ID, if `CancelExpiredEscrow` closed a native escrow.
        closed_native_escrow_id: Option<u64>,
        /// Newly opened native `CheckCreate` record, if `CreateCheck`.
        opened_native_check: Option<MirroredNativeCheck>,
        /// Closed native `Check` engine ID, if `CancelCheck`.
        closed_native_check_id: Option<u64>,
        /// Verified `BatchV1_1` outer batches added by this action (`TownImport`).
        batches_verified_delta: usize,
        /// Verified inner `BatchV1_1` legs added by this action.
        inner_legs_verified_delta: usize,
    },
    /// Phase-1 [`MarketAction`] rejected by `TurnEngine` (e.g., post-lock `CancelOffer`).
    MarketActionRejected {
        /// Active turn (`1..=32`).
        turn: u8,
        /// Attempted market action (recorded in `LoggedTurn.market_actions`).
        action: MarketAction,
        /// `true` when `EngineError::CancelLocked` caused the rejection.
        post_lock_cancel_refused: bool,
    },
    /// Phase transition `TalkAndMarket -> Commit`.
    AdvancedToCommit {
        /// Active turn (`1..=32`).
        turn: u8,
    },
    /// Phase-2 commitment validated by `MemoFirewall` and `close_time` cutoff.
    SeatCommitAccepted {
        /// Active turn (`1..=32`).
        turn: u8,
        /// Seat index (`0..seat_count`).
        seat: u8,
        /// 32-byte SHA-256 order commitment.
        commitment: CommitmentHash,
        /// Validated commit transaction hash.
        tx_hash: String,
        /// Mid-turn 1-drop commit escrow record, if submitted via `submit_seat_commit_via_xrpld`.
        commit_escrow: Option<MirroredCommitEscrow>,
    },
    /// Sanitized `MemoData` firewall drop record (retains zero raw memo payload).
    FirewallDropped {
        /// Sanitized drop telemetry.
        record: IngressDropRecord,
    },
    /// Phase transition `Commit -> Reveal`.
    AdvancedToReveal {
        /// Active turn (`1..=32`).
        turn: u8,
    },
    /// Validated Phase-2 reveal submitted and verified against commitment.
    SeatRevealAccepted {
        /// Active turn (`1..=32`).
        turn: u8,
        /// Seat index (`0..seat_count`).
        seat: u8,
        /// Revealed order bundle + salt.
        reveal: OrderReveal,
    },
    /// Phase-3 Sweep settled on-ledger with verified `BatchV1_1` + `0` parity divergence.
    TurnSwept {
        /// Completed turn (`1..=32`).
        turn: u8,
        /// Authenticated `drand quicknet` round.
        drand_round: u64,
        /// 48-byte BLS12-381 `drand` signature.
        drand_signature: Vec<u8>,
        /// Post-commit deadline Unix timestamp (`#51`).
        commit_deadline_unix: u64,
        /// Post-Sweep canonical `EngineState` SHA-256 digest.
        post_state_digest: [u8; 32],
        /// Validated ledger index after post-Sweep `account_lines` parity passed.
        settled_ledger_index: u64,
        /// Next turn's [`StewardTurnWindow`] when the season continues.
        next_window: Option<StewardTurnWindow>,
        /// Active cartel `PermissionedDomain` map (`domain_id -> 64-hex DomainID`).
        cartel_domain_ids: BTreeMap<u8, String>,
        /// Newly awarded charters on this turn (Turn `6k`).
        awarded_charters: Vec<MirroredCharter>,
        /// Verified `BatchV1_1` outer envelopes submitted during Sweep.
        batch_count: usize,
        /// Verified inner `BatchV1_1` legs submitted during Sweep.
        batch_inner_legs_verified: usize,
    },
    /// 3-wave Phoenix state rehydration completed against a fresh genesis ledger.
    PhoenixRehydrated {
        /// Active turn at time of wipe (`1..=32`).
        at_turn: u8,
        /// `WipeSignal::code()` that triggered recovery.
        wipe_code: String,
        /// Last validated ledger index seen before the wipe.
        previous_ledger: u64,
        /// Genesis ledger index observed on the fresh node before Wave 1.
        fresh_boot_ledger: u64,
        /// Validated ledger index after Wave 3 completed.
        rehydrated_ledger: u64,
        /// Total `ledger_accept` closes consumed (`<= 3`).
        ledger_closes_used: u64,
        /// Newly created table `PermissionedDomain` 64-hex ID.
        new_table_domain_id: String,
        /// Newly created cartel `PermissionedDomain` IDs (`domain_id -> 64-hex`).
        remapped_cartel_domains: BTreeMap<u8, String>,
        /// Remapped native `TokenEscrow` sequences (`engine_escrow_id -> new_xrpl_sequence`).
        remapped_escrow_sequences: BTreeMap<u64, u32>,
        /// Remapped native `Check` IDs (`engine_check_id -> new_64_hex_check_id`).
        remapped_check_ids: BTreeMap<u64, String>,
        /// Remapped mid-turn commit escrow sequences (`seat -> new_xrpl_sequence`).
        remapped_commit_sequences: BTreeMap<u8, u32>,
        /// Updated [`StewardTurnWindow`] anchored on the fresh ledger's `close_time`.
        new_window: StewardTurnWindow,
        /// Total `BatchV1_1` outer batches verified across Waves 1–3.
        wave_batches_verified: usize,
        /// Total inner `BatchV1_1` legs verified across Waves 1–3.
        wave_inner_legs_verified: usize,
    },
}

/// Complete deterministic table state materialized by replaying [`ArchivistMirror`].
#[derive(Debug, Clone)]
pub struct MaterializedArchivistState {
    /// Reconstructed `TurnEngine` with verified post-Sweep digests.
    pub(crate) engine: TurnEngine,
    /// Public table layout seed.
    pub layout_seed: u64,
    /// Sealed Turn-1 mandate deal.
    pub(crate) sealed_deal: SealedMandateDeal,
    /// Turn-1 mandate commitments per seat.
    pub mandate_commits: Vec<Option<CommitmentHash>>,
    /// Turn-1 mandate reveals per seat.
    pub mandate_reveals: Vec<Option<MandateReveal>>,
    /// Steward classic address.
    pub steward_address: String,
    /// Seated classic addresses (`0..seat_count`).
    pub seat_addresses: Vec<String>,
    /// Standing Regent classic addresses (`0..seat_count`).
    pub regent_addresses: Vec<Option<String>>,
    /// Verified Steward `AccountRoot.Flags`.
    pub authority_flags: u32,
    /// Active table `PermissionedDomain` 64-hex ID.
    pub table_domain_id: String,
    /// Active cartel `PermissionedDomain` IDs (`domain_id -> 64-hex`).
    pub cartel_domain_ids: BTreeMap<u8, String>,
    /// Active Syndicate Charters (`expires_turn > current_turn`).
    pub active_charters: Vec<MirroredCharter>,
    /// Last synchronized `CreditTier` per seat.
    pub last_credit_tiers: Vec<CreditTier>,
    /// Last synchronized `SeatIdentity` per seat.
    pub last_identities: Vec<SeatIdentity>,
    /// Highest validated ledger index observed.
    pub last_observed_ledger: u64,
    /// Configured Ripple-epoch turn span.
    pub turn_span_ripple_secs: u64,
    /// Active [`StewardTurnWindow`].
    pub window: StewardTurnWindow,
    /// Open native `TokenEscrow` vaults keyed by `escrow_id`.
    pub native_escrows: BTreeMap<u64, MirroredNativeEscrow>,
    /// Open Steward-custody escrows keyed by `escrow_id`.
    pub custody_escrows: BTreeSet<u64>,
    /// Open native `CheckCreate` objects keyed by engine `check_id`.
    pub native_checks: BTreeMap<u64, MirroredNativeCheck>,
    /// Active mid-turn 1-drop commit escrows keyed by `seat`.
    pub active_commit_escrows: BTreeMap<u8, MirroredCommitEscrow>,
    /// In-progress turn market actions (for `LoggedTurn`).
    pub turn_market_actions: Vec<MarketAction>,
    /// In-progress turn cancel-lock action index.
    pub turn_cancel_lock_at: Option<u32>,
    /// In-progress turn rejected action count.
    pub turn_rejected_count: usize,
    /// In-progress turn commitments.
    pub turn_commitments: BTreeMap<u8, CommitmentHash>,
    /// In-progress turn reveals.
    pub turn_reveals: BTreeMap<u8, OrderReveal>,
    /// Completed [`LoggedTurn`] records for `replay_season`.
    pub logged_turns: Vec<LoggedTurn>,
    /// Recorded post-Sweep `EngineState` digests.
    pub live_state_digests: Vec<[u8; 32]>,
    /// Sanitized `MemoData` firewall drops recorded across the season.
    pub dropped_records: Vec<IngressDropRecord>,
    /// Cumulative verified `BatchV1_1` outer envelopes.
    pub total_batches_verified: usize,
    /// Cumulative verified inner `BatchV1_1` legs.
    pub total_inner_legs_verified: usize,
    /// Cumulative post-lock `CancelOffer` refusals.
    pub total_post_lock_cancel_refusals: usize,
}

/// Append-only, SHA-256 hash-chained Archivist state mirror (`docs/design-v2.md` §7.2).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchivistMirror {
    events: Vec<ArchivistEvent>,
    chain_tip_digest: [u8; 32],
}

impl ArchivistMirror {
    /// Create an empty Archivist mirror.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Borrow the ordered slice of recorded [`ArchivistEvent`]s.
    #[must_use]
    pub fn events(&self) -> &[ArchivistEvent] {
        &self.events
    }

    /// Number of recorded events.
    #[must_use]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// True when zero events have been recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Current rolling SHA-256 hash-chain tip digest.
    #[must_use]
    pub const fn chain_tip_digest(&self) -> [u8; 32] {
        self.chain_tip_digest
    }

    /// Append `event` and advance `chain_tip_digest`.
    pub fn append(&mut self, event: ArchivistEvent) -> Result<[u8; 32], StewardError> {
        let seq = self.events.len() as u64;
        let next_tip = compute_next_chain_tip(self.chain_tip_digest, seq, &event)?;
        self.events.push(event);
        self.chain_tip_digest = next_tip;
        Ok(next_tip)
    }

    /// Recompute the rolling SHA-256 hash chain from event `0` and reject any tampering.
    pub fn verify_hash_chain(&self) -> Result<(), StewardError> {
        let mut tip = [0u8; 32];
        for (idx, event) in self.events.iter().enumerate() {
            tip = compute_next_chain_tip(tip, idx as u64, event)?;
        }
        if tip != self.chain_tip_digest {
            return Err(StewardError::ArchivistIntegrity(
                "archivist hash chain tip mismatch".into(),
            ));
        }
        Ok(())
    }

    /// Serialize the hash-chained mirror to JSON bytes (never contains signing keys).
    pub fn to_json_bytes(&self) -> Result<Vec<u8>, StewardError> {
        self.verify_hash_chain()?;
        serde_json::to_vec(self).map_err(|err| {
            StewardError::ArchivistIntegrity(format!("archivist serialize failed: {err}"))
        })
    }

    /// Deserialize an [`ArchivistMirror`] from JSON bytes and verify its rolling hash chain.
    pub fn from_json_slice(bytes: &[u8]) -> Result<Self, StewardError> {
        let mirror: Self = serde_json::from_slice(bytes).map_err(|err| {
            StewardError::ArchivistIntegrity(format!("archivist deserialize failed: {err}"))
        })?;
        mirror.verify_hash_chain()?;
        Ok(mirror)
    }

    /// Replay every event through a fresh `TurnEngine`, verify every post-Sweep `EngineState`
    /// digest, and materialize the exact current table and contract state for Phoenix recovery.
    pub fn materialize(&self) -> Result<MaterializedArchivistState, StewardError> {
        self.verify_hash_chain()?;
        let Some(ArchivistEvent::TableBootstrapped {
            spec,
            layout_seed,
            sealed_deal,
            mandate_commits,
            mandate_reveals,
            turn_span_ripple_secs,
            steward_address,
            seat_addresses,
            regent_addresses,
            authority_flags,
            table_domain_id,
            initial_ledger_index,
            initial_window,
            initial_state_digest,
            bootstrap_batches_verified,
            bootstrap_inner_legs_verified,
            ..
        }) = self.events.first()
        else {
            return Err(StewardError::ArchivistIntegrity(
                "archivist mirror must start with TableBootstrapped".into(),
            ));
        };

        let n = usize::from(spec.seat_count);
        if seat_addresses.len() != n || regent_addresses.len() != n {
            return Err(StewardError::ArchivistIntegrity(
                "archivist seat/regent address count does not match TableSpec".into(),
            ));
        }

        let mut engine = TurnEngine::new((**spec).clone())?;
        for (s_idx, commit_opt) in mandate_commits.iter().enumerate().take(n) {
            if let Some(commit) = commit_opt {
                engine.submit_mandate_commitment(Seat(s_idx as u8), *commit)?;
            } else if let Some(reveal) = sealed_deal.seat_reveals.get(s_idx) {
                let computed =
                    compute_mandate_commitment(spec.table_id, Seat(s_idx as u8), reveal)?;
                engine.submit_mandate_commitment(Seat(s_idx as u8), computed)?;
            }
        }
        let got_init_digest = engine.state_digest()?;
        if got_init_digest != *initial_state_digest {
            return Err(StewardError::ArchivistIntegrity(
                "archivist initial state_digest mismatch".into(),
            ));
        }

        let mut st = MaterializedArchivistState {
            last_credit_tiers: engine.state().seats.iter().map(|s| s.credit_tier).collect(),
            last_identities: engine.state().seats.iter().map(|s| s.identity).collect(),
            engine,
            layout_seed: *layout_seed,
            sealed_deal: sealed_deal.clone(),
            mandate_commits: mandate_commits.clone(),
            mandate_reveals: mandate_reveals.clone(),
            steward_address: steward_address.clone(),
            seat_addresses: seat_addresses.clone(),
            regent_addresses: regent_addresses.clone(),
            authority_flags: *authority_flags,
            table_domain_id: table_domain_id.clone(),
            cartel_domain_ids: BTreeMap::new(),
            active_charters: Vec::new(),
            last_observed_ledger: *initial_ledger_index,
            turn_span_ripple_secs: *turn_span_ripple_secs,
            window: *initial_window,
            native_escrows: BTreeMap::new(),
            custody_escrows: BTreeSet::new(),
            native_checks: BTreeMap::new(),
            active_commit_escrows: BTreeMap::new(),
            turn_market_actions: Vec::new(),
            turn_cancel_lock_at: None,
            turn_rejected_count: 0,
            turn_commitments: BTreeMap::new(),
            turn_reveals: BTreeMap::new(),
            logged_turns: Vec::new(),
            live_state_digests: Vec::new(),
            dropped_records: Vec::new(),
            total_batches_verified: *bootstrap_batches_verified,
            total_inner_legs_verified: *bootstrap_inner_legs_verified,
            total_post_lock_cancel_refusals: 0,
        };

        for event in self.events.iter().skip(1) {
            apply_event(&mut st, event)?;
        }
        Ok(st)
    }
}

fn apply_event(
    st: &mut MaterializedArchivistState,
    event: &ArchivistEvent,
) -> Result<(), StewardError> {
    match event {
        ArchivistEvent::TableBootstrapped { .. } => {
            return Err(StewardError::ArchivistIntegrity(
                "duplicate TableBootstrapped event in archivist mirror".into(),
            ));
        }
        ArchivistEvent::CancelLockEngaged {
            turn,
            cancel_lock_at,
            ledger_index,
            ..
        } => {
            ensure_turn(st, *turn)?;
            if !st.engine.state().cancel_locked {
                st.engine.begin_cancel_lock()?;
            }
            if st.turn_cancel_lock_at.is_none() {
                st.turn_cancel_lock_at = Some(*cancel_lock_at);
            }
            st.last_observed_ledger = st.last_observed_ledger.max(*ledger_index);
        }
        ArchivistEvent::MarketActionAccepted {
            turn,
            action,
            opened_native_escrow,
            closed_native_escrow_id,
            opened_native_check,
            closed_native_check_id,
            batches_verified_delta,
            inner_legs_verified_delta,
        } => {
            ensure_turn(st, *turn)?;
            let next_escrow_id = st.engine.state().next_escrow_id;
            st.turn_market_actions.push(action.clone());
            st.engine.apply_market_action(action.clone())?;
            if let Some(esc) = opened_native_escrow {
                st.native_escrows.insert(esc.escrow_id, esc.clone());
            } else if matches!(action, MarketAction::CreateEscrow { .. }) {
                st.custody_escrows.insert(next_escrow_id);
            }
            if let Some(closed_id) = closed_native_escrow_id {
                st.native_escrows.remove(closed_id);
            } else if let MarketAction::CancelExpiredEscrow { escrow_id, .. } = action {
                st.custody_escrows.remove(escrow_id);
            }
            if let Some(chk) = opened_native_check {
                st.native_checks.insert(chk.check_id_engine, chk.clone());
            }
            if let Some(closed_cid) = closed_native_check_id {
                st.native_checks.remove(closed_cid);
            }
            st.total_batches_verified = st
                .total_batches_verified
                .saturating_add(*batches_verified_delta);
            st.total_inner_legs_verified = st
                .total_inner_legs_verified
                .saturating_add(*inner_legs_verified_delta);
        }
        ArchivistEvent::MarketActionRejected {
            turn,
            action,
            post_lock_cancel_refused,
        } => {
            ensure_turn(st, *turn)?;
            st.turn_market_actions.push(action.clone());
            st.turn_rejected_count = st.turn_rejected_count.saturating_add(1);
            if *post_lock_cancel_refused {
                st.total_post_lock_cancel_refusals =
                    st.total_post_lock_cancel_refusals.saturating_add(1);
            }
        }
        ArchivistEvent::AdvancedToCommit { turn } => {
            ensure_turn(st, *turn)?;
            st.engine.advance_to_commit()?;
        }
        ArchivistEvent::SeatCommitAccepted {
            turn,
            seat,
            commitment,
            commit_escrow,
            ..
        } => {
            ensure_turn(st, *turn)?;
            st.engine
                .submit_order_commitment(Seat(*seat), *commitment)?;
            st.turn_commitments.insert(*seat, *commitment);
            if let Some(ce) = commit_escrow {
                st.active_commit_escrows.insert(*seat, ce.clone());
            }
        }
        ArchivistEvent::FirewallDropped { record } => {
            st.dropped_records.push(record.clone());
        }
        ArchivistEvent::AdvancedToReveal { turn } => {
            ensure_turn(st, *turn)?;
            st.engine.advance_to_reveal()?;
        }
        ArchivistEvent::SeatRevealAccepted { turn, seat, reveal } => {
            ensure_turn(st, *turn)?;
            st.active_commit_escrows.remove(seat);
            st.engine.submit_order_reveal(Seat(*seat), reveal.clone())?;
            st.turn_reveals.insert(*seat, reveal.clone());
        }
        ArchivistEvent::TurnSwept {
            turn,
            drand_round,
            drand_signature,
            commit_deadline_unix,
            post_state_digest,
            settled_ledger_index,
            next_window,
            cartel_domain_ids,
            awarded_charters,
            batch_count,
            batch_inner_legs_verified,
        } => {
            ensure_turn(st, *turn)?;
            let turn_beacon = derive_drand_beacon(drand_signature)?;
            let receipt = st
                .engine
                .sweep_turn_with_drand(*drand_round, drand_signature)?;
            let engine_digest = st.engine.state_digest()?;
            if receipt.post_state_digest != *post_state_digest
                || engine_digest != *post_state_digest
            {
                return Err(StewardError::ArchivistIntegrity(format!(
                    "turn {turn} archivist replay state_digest diverged"
                )));
            }
            for eid in &receipt.escrows_finished {
                st.native_escrows.remove(eid);
                st.custody_escrows.remove(eid);
            }
            for eid in &receipt.escrows_cancelled_or_forfeited {
                st.native_escrows.remove(eid);
                st.custody_escrows.remove(eid);
            }
            for cid in &receipt.checks_cashed {
                st.native_checks.remove(cid);
            }
            for cid in &receipt.checks_bounced {
                st.native_checks.remove(cid);
            }

            let state_now = st.engine.state();
            st.last_credit_tiers = state_now.seats.iter().map(|s| s.credit_tier).collect();
            st.last_identities = state_now.seats.iter().map(|s| s.identity).collect();
            st.cartel_domain_ids = cartel_domain_ids.clone();
            st.active_charters
                .retain(|ch| ch.expires_turn > state_now.turn);
            for ch in awarded_charters {
                st.active_charters.retain(|old| old.role != ch.role);
                st.active_charters.push(ch.clone());
            }
            st.live_state_digests.push(*post_state_digest);
            st.logged_turns.push(LoggedTurn {
                turn: *turn,
                market_actions: std::mem::take(&mut st.turn_market_actions),
                cancel_lock_at: st.turn_cancel_lock_at.take(),
                commit_deadline_unix: Some(*commit_deadline_unix),
                commitments: std::mem::take(&mut st.turn_commitments),
                reveals: std::mem::take(&mut st.turn_reveals),
                turn_beacon,
                drand_round: Some(*drand_round),
                drand_signature: Some(drand_signature.clone()),
                eval_vrf_proof: None,
                drawn_shocks: receipt.newly_drawn_shocks,
            });
            st.turn_rejected_count = 0;
            st.total_batches_verified = st.total_batches_verified.saturating_add(*batch_count);
            st.total_inner_legs_verified = st
                .total_inner_legs_verified
                .saturating_add(*batch_inner_legs_verified);
            st.last_observed_ledger = st.last_observed_ledger.max(*settled_ledger_index);
            if let Some(win) = next_window {
                st.window = *win;
            }
        }
        ArchivistEvent::PhoenixRehydrated {
            rehydrated_ledger,
            new_table_domain_id,
            remapped_cartel_domains,
            remapped_escrow_sequences,
            remapped_check_ids,
            remapped_commit_sequences,
            new_window,
            wave_batches_verified,
            wave_inner_legs_verified,
            ..
        } => {
            st.table_domain_id = new_table_domain_id.clone();
            st.cartel_domain_ids = remapped_cartel_domains.clone();
            for (eid, new_seq) in remapped_escrow_sequences {
                if let Some(esc) = st.native_escrows.get_mut(eid) {
                    esc.xrpl_sequence = *new_seq;
                }
            }
            for (cid, new_check_id) in remapped_check_ids {
                if let Some(chk) = st.native_checks.get_mut(cid) {
                    chk.xrpl_check_id = new_check_id.clone();
                }
            }
            for (seat, new_seq) in remapped_commit_sequences {
                if let Some(ce) = st.active_commit_escrows.get_mut(seat) {
                    ce.sequence = *new_seq;
                }
            }
            st.window = *new_window;
            st.last_observed_ledger = *rehydrated_ledger;
            st.total_batches_verified = st
                .total_batches_verified
                .saturating_add(*wave_batches_verified);
            st.total_inner_legs_verified = st
                .total_inner_legs_verified
                .saturating_add(*wave_inner_legs_verified);
        }
    }
    Ok(())
}

fn ensure_turn(st: &MaterializedArchivistState, expected: u8) -> Result<(), StewardError> {
    let actual = st.engine.state().turn;
    if actual != expected {
        return Err(StewardError::ArchivistIntegrity(format!(
            "archivist event turn mismatch: expected {actual}, got {expected}"
        )));
    }
    Ok(())
}

fn compute_next_chain_tip(
    prev_tip: [u8; 32],
    seq: u64,
    event: &ArchivistEvent,
) -> Result<[u8; 32], StewardError> {
    let encoded = serde_json::to_vec(event).map_err(|err| {
        StewardError::ArchivistIntegrity(format!("archivist event serialize failed: {err}"))
    })?;
    let mut hasher = Sha256::new();
    hasher.update(ARCHIVIST_CHAIN_DOMAIN);
    hasher.update(prev_tip);
    hasher.update(seq.to_be_bytes());
    hasher.update(encoded);
    Ok(hasher.finalize().into())
}

#[cfg(test)]
mod tests {
    use league_core::{
        DRAND_QUICKNET_GENESIS_UNIX, DrandSchedule, TableId, UNIT_SCALE, compute_mandate_commitment,
    };

    use super::*;
    use crate::firewall::IngressDropReason;

    fn sample_window(turn: u8) -> StewardTurnWindow {
        StewardTurnWindow {
            turn,
            open_ripple_close: 815_000_000,
            lock_ripple_close: 815_000_300,
            commit_close_ripple: 815_000_400,
            lock_unix: DRAND_QUICKNET_GENESIS_UNIX + 6,
            commit_close_unix: DRAND_QUICKNET_GENESIS_UNIX + 9,
            beacon_unix: DRAND_QUICKNET_GENESIS_UNIX + 12,
            expected_drand_round: 5,
        }
    }

    fn sample_bootstrap_event() -> ArchivistEvent {
        let schedule = DrandSchedule {
            turn_1_close_unix: DRAND_QUICKNET_GENESIS_UNIX + 12,
            turn_period_secs: 12,
        };
        let (mut spec, deal) =
            TableSpec::standard_game_chiral_ring_with_mandates(TableId(301), 4, 7, schedule)
                .unwrap_or_else(|e| panic!("spec: {e}"));
        for s in &mut spec.seats {
            s.initial_cred = 100 * UNIT_SCALE;
        }
        let mut engine = TurnEngine::new(spec.clone()).unwrap_or_else(|e| panic!("engine: {e}"));
        let mut commits = Vec::with_capacity(4);
        let mut reveals = Vec::with_capacity(4);
        for (idx, rev) in deal.seat_reveals.iter().enumerate() {
            let c = compute_mandate_commitment(TableId(301), Seat(idx as u8), rev)
                .unwrap_or_else(|e| panic!("commit: {e}"));
            engine
                .submit_mandate_commitment(Seat(idx as u8), c)
                .unwrap_or_else(|e| panic!("submit mandate: {e}"));
            commits.push(Some(c));
            reveals.push(Some(rev.clone()));
        }
        let initial_state_digest = engine
            .state_digest()
            .unwrap_or_else(|e| panic!("digest: {e}"));

        ArchivistEvent::TableBootstrapped {
            table_id: 301,
            spec: Box::new(spec),
            layout_seed: 7,
            sealed_deal: deal,
            mandate_commits: commits,
            mandate_reveals: reveals,
            turn_span_ripple_secs: 400,
            steward_address: "rSteward".into(),
            seat_addresses: vec![
                "rSeat0".into(),
                "rSeat1".into(),
                "rSeat2".into(),
                "rSeat3".into(),
            ],
            regent_addresses: vec![
                Some("rReg0".into()),
                Some("rReg1".into()),
                Some("rReg2".into()),
                Some("rReg3".into()),
            ],
            authority_flags: 0xc08c_0000,
            table_domain_id: "AA".repeat(32),
            initial_ledger_index: 25,
            initial_window: sample_window(1),
            initial_state_digest,
            bootstrap_batches_verified: 1,
            bootstrap_inner_legs_verified: 4,
        }
    }

    #[test]
    fn archivist_mirror_hash_chain_tamper_and_materialize_guards() {
        let mut mirror = ArchivistMirror::new();
        assert!(mirror.is_empty());
        assert_eq!(mirror.len(), 0);
        assert!(mirror.materialize().is_err());

        let boot = sample_bootstrap_event();
        let tip0 = mirror
            .append(boot.clone())
            .unwrap_or_else(|e| panic!("append boot: {e}"));
        assert_ne!(tip0, [0u8; 32]);
        assert_eq!(mirror.chain_tip_digest(), tip0);

        mirror
            .append(ArchivistEvent::FirewallDropped {
                record: IngressDropRecord {
                    tx_hash: "TX1".into(),
                    sender: "rOutsider".into(),
                    reason: IngressDropReason::UnseatedSender,
                },
            })
            .unwrap_or_else(|e| panic!("append drop: {e}"));

        let bytes = mirror
            .to_json_bytes()
            .unwrap_or_else(|e| panic!("serialize: {e}"));
        let loaded =
            ArchivistMirror::from_json_slice(&bytes).unwrap_or_else(|e| panic!("deserialize: {e}"));
        assert_eq!(loaded.len(), 2);
        let mat = loaded
            .materialize()
            .unwrap_or_else(|e| panic!("materialize: {e}"));
        assert_eq!(mat.dropped_records.len(), 1);
        assert_eq!(mat.engine.state().turn, 1);

        // Tampering with JSON bytes must fail hash-chain verification on load.
        let mut tampered = loaded;
        tampered.chain_tip_digest[0] ^= 0xFF;
        assert!(matches!(
            tampered.verify_hash_chain(),
            Err(StewardError::ArchivistIntegrity(_))
        ));

        // Duplicate TableBootstrapped or turn mismatch must fail materialize.
        let mut bad_dup = ArchivistMirror::new();
        let _ = bad_dup.append(boot.clone());
        let _ = bad_dup.append(boot);
        assert!(matches!(
            bad_dup.materialize(),
            Err(StewardError::ArchivistIntegrity(_))
        ));

        let mut bad_turn = ArchivistMirror::new();
        let _ = bad_turn.append(sample_bootstrap_event());
        let _ = bad_turn.append(ArchivistEvent::AdvancedToCommit { turn: 9 });
        assert!(matches!(
            bad_turn.materialize(),
            Err(StewardError::ArchivistIntegrity(_))
        ));
    }

    #[test]
    fn archivist_fold_variants_and_integrity_error_paths() {
        let def = ArchivistMirror::default();
        assert!(def.is_empty());
        assert!(def.events().is_empty());

        assert!(matches!(
            ArchivistMirror::from_json_slice(b"not-valid-json"),
            Err(StewardError::ArchivistIntegrity(_))
        ));

        // Non-bootstrap first event fails.
        let mut non_boot = ArchivistMirror::new();
        let _ = non_boot.append(ArchivistEvent::AdvancedToCommit { turn: 1 });
        assert!(matches!(
            non_boot.materialize(),
            Err(StewardError::ArchivistIntegrity(_))
        ));

        // Seat address count mismatch fails.
        let mut bad_seats = ArchivistMirror::new();
        let mut ev = sample_bootstrap_event();
        if let ArchivistEvent::TableBootstrapped {
            ref mut seat_addresses,
            ..
        } = ev
        {
            seat_addresses.pop();
        }
        let _ = bad_seats.append(ev);
        assert!(matches!(
            bad_seats.materialize(),
            Err(StewardError::ArchivistIntegrity(_))
        ));

        // Initial state digest mismatch fails.
        let mut bad_digest = ArchivistMirror::new();
        let mut ev2 = sample_bootstrap_event();
        if let ArchivistEvent::TableBootstrapped {
            ref mut initial_state_digest,
            ..
        } = ev2
        {
            initial_state_digest[0] ^= 0xFF;
        }
        let _ = bad_digest.append(ev2);
        assert!(matches!(
            bad_digest.materialize(),
            Err(StewardError::ArchivistIntegrity(_))
        ));

        // Fold CancelLockEngaged, MarketActionAccepted (custody escrow + native check),
        // MarketActionRejected, and PhoenixRehydrated.
        let mut mirror = ArchivistMirror::new();
        let _ = mirror.append(sample_bootstrap_event());
        let _ = mirror.append(ArchivistEvent::CancelLockEngaged {
            turn: 1,
            cancel_lock_at: 0,
            ledger_index: 30,
            ripple_close: 815_000_310,
        });
        let _ = mirror.append(ArchivistEvent::MarketActionRejected {
            turn: 1,
            action: league_core::MarketAction::CancelOffer {
                seat: 0,
                offer_id: 1,
            },
            post_lock_cancel_refused: true,
        });
        let _ = mirror.append(ArchivistEvent::MarketActionAccepted {
            turn: 1,
            action: league_core::MarketAction::CreateCheck {
                seat: 0,
                check: league_core::CheckProposal {
                    payee: 1,
                    asset: league_core::AssetId::Cred,
                    send_max: 1_000,
                    deliver_min: 1_000,
                    due_turn: 2,
                    expiration_turn: 5,
                },
            },
            opened_native_escrow: None,
            closed_native_escrow_id: None,
            opened_native_check: Some(MirroredNativeCheck {
                check_id_engine: 1,
                drawer_seat: 0,
                payee_seat: 1,
                currency: "CRED".into(),
                amount_milli: 1_000,
                due_turn: 2,
                expiration_turn: 5,
                xrpl_check_id: "11".repeat(32),
            }),
            closed_native_check_id: None,
            batches_verified_delta: 0,
            inner_legs_verified_delta: 0,
        });
        let _ = mirror.append(ArchivistEvent::MarketActionAccepted {
            turn: 1,
            action: league_core::MarketAction::CancelCheck {
                seat: 0,
                check_id: 1,
            },
            opened_native_escrow: None,
            closed_native_escrow_id: None,
            opened_native_check: None,
            closed_native_check_id: Some(1),
            batches_verified_delta: 0,
            inner_legs_verified_delta: 0,
        });
        let _ = mirror.append(ArchivistEvent::MarketActionAccepted {
            turn: 1,
            action: league_core::MarketAction::CreateEscrow {
                seat: 0,
                escrow: league_core::EscrowProposal {
                    destination: 1,
                    asset: league_core::AssetId::Cred,
                    amount: 1_000,
                    finish_after_turn: Some(2),
                    cancel_after_turn: 4,
                    condition_hash: Some(league_core::CommitmentHash([0x11; 32])),
                    bond: None,
                },
            },
            opened_native_escrow: None,
            closed_native_escrow_id: None,
            opened_native_check: None,
            closed_native_check_id: None,
            batches_verified_delta: 0,
            inner_legs_verified_delta: 0,
        });
        let _ = mirror.append(ArchivistEvent::PhoenixRehydrated {
            at_turn: 1,
            wipe_code: "WIPE_AUTHORITY_MISSING".into(),
            previous_ledger: 30,
            fresh_boot_ledger: 2,
            rehydrated_ledger: 5,
            ledger_closes_used: 3,
            new_table_domain_id: "BB".repeat(32),
            remapped_cartel_domains: BTreeMap::from([(1_u8, "CC".repeat(32))]),
            remapped_escrow_sequences: BTreeMap::from([(99_u64, 42_u32)]),
            remapped_check_ids: BTreeMap::from([(99_u64, "DD".repeat(32))]),
            remapped_commit_sequences: BTreeMap::from([(0_u8, 43_u32)]),
            new_window: sample_window(1),
            wave_batches_verified: 4,
            wave_inner_legs_verified: 16,
        });

        let mat = mirror.materialize().unwrap_or_else(|e| panic!("mat: {e}"));
        assert_eq!(mat.total_post_lock_cancel_refusals, 1);
        assert!(mat.native_checks.is_empty());
        assert!(mat.custody_escrows.contains(&1));
        assert_eq!(mat.table_domain_id, "BB".repeat(32));
        assert_eq!(mat.last_observed_ledger, 5);
        assert_eq!(mat.total_batches_verified, 5);
    }
}
