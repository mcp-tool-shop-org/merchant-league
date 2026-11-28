//! 3-wave `BatchV1_1` + `TicketCreate` Phoenix State Rehydrator (`docs/design-v2.md` §7.2,
//! `docs/game-mode-protocol.md` §§1–5).
//!
//! Rebuilds deterministic Steward + seat accounts, Authority genesis flags, `TicketCreate`
//! pools, `PermissionedDomains`, `Credentials`, `DelegateSet` Standing Regents, `NoRipple` +
//! `SetAuth` trust lines, pre-wipe `CRED` / `Raw` / `Refined` balances, open `TokenEscrow` /
//! `CheckCreate` / commit-escrow objects, and `OracleSet` documents within `<= 3`
//! `ledger_accept` closes.

use std::collections::BTreeMap;

use league_core::{AssetId, CharterRole, CreditTier, SeatIdentity, Units};
use league_ledger::client::Transport;
use league_season::WipeSignal;
use league_standalone::client::{StandAlone, StandAloneAccount};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::amount::{asset_currency_code, format_milli_units};
use crate::archivist::MaterializedArchivistState;
use crate::codec::{
    ASF_ALLOW_TRUSTLINE_CLAWBACK, ASF_ALLOW_TRUSTLINE_LOCKING, ASF_DEFAULT_RIPPLE,
    ASF_DISALLOW_XRP, ASF_REQUIRE_AUTH, LSF_ALLOW_TRUSTLINE_CLAWBACK, LSF_ALLOW_TRUSTLINE_LOCKING,
    LSF_DEFAULT_RIPPLE, LSF_DISALLOW_XRP, LSF_REQUIRE_AUTH, QueuedBatch, QueuedTx,
    REGENT_DELEGATE_PERMISSIONS, SeqOrTicket, TF_SET_AUTH, TF_SET_NO_RIPPLE, TicketPool,
    extract_created_ledger_index, payment_iou_json, queue_batch_all_or_nothing, queue_raw_json_tx,
    split_batch_chunks, verify_queued_batch, verify_queued_tx,
};
use crate::error::StewardError;
use crate::reserve::StewardReservePool;

const PHOENIX_ACCOUNT_XRP_DROPS: u64 = 2_000_000_000;
const PHOENIX_TRUST_LINE_LIMIT: &str = "1000000";
const PHOENIX_STEWARD_TICKETS: u32 = 200;
const PHOENIX_SEAT_TICKETS: u32 = 80;
const RIPPLE_TO_UNIX_EPOCH_SECS: u64 = 946_684_800;

/// Output of one 3-wave Phoenix state rehydration against a fresh genesis node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhoenixRehydrationOutcome {
    /// Machine-readable `WipeSignal::code()` that triggered recovery.
    pub wipe_code: String,
    /// Highest validated ledger index observed before the wipe.
    pub previous_ledger: u64,
    /// Validated ledger index on the fresh node before Wave 1 began.
    pub fresh_boot_ledger: u64,
    /// Validated ledger index after Wave 3 completed.
    pub rehydrated_ledger: u64,
    /// Total `ledger_accept` calls executed (`<= 3`).
    pub ledger_closes_used: u64,
    /// Validated Ripple-epoch `close_time` after Wave 3 completed.
    pub rehydrated_ripple_close: u64,
    /// `BatchV1_1` outer envelopes verified in Wave 1.
    pub wave1_batches: usize,
    /// `BatchV1_1` outer envelopes verified in Wave 2.
    pub wave2_batches: usize,
    /// `BatchV1_1` outer envelopes verified in Wave 3.
    pub wave3_batches: usize,
    /// Total `BatchV1_1` outer envelopes verified across Waves 1–3.
    pub total_batches_verified: usize,
    /// Total inner `BatchV1_1` legs verified across Waves 1–3.
    pub total_inner_legs_verified: usize,
    /// Newly created table `PermissionedDomain` 64-hex ID.
    pub new_table_domain_id: String,
    /// Remapped cartel `PermissionedDomain` IDs (`domain_id -> 64-hex`).
    pub remapped_cartel_domains: BTreeMap<u8, String>,
    /// Remapped native `TokenEscrow` `Sequence` values (`engine_escrow_id -> new_seq`).
    pub remapped_escrow_sequences: BTreeMap<u64, u32>,
    /// Remapped native `CheckCreate` IDs (`engine_check_id -> new_64_hex_check_id`).
    pub remapped_check_ids: BTreeMap<u64, String>,
    /// Remapped mid-turn 1-drop commit escrow sequences (`seat -> new_seq`).
    pub remapped_commit_sequences: BTreeMap<u8, u32>,
    /// Last `OracleSet.LastUpdateTime` Unix seconds published in Wave 3.
    pub wave3_oracle_update_time: u32,
}

/// Reconstructed ticket pools and Phoenix telemetry returned by [`execute_three_wave_phoenix`].
pub(crate) struct PhoenixExecutionResult {
    pub(crate) outcome: PhoenixRehydrationOutcome,
    pub(crate) steward_tickets: TicketPool,
    pub(crate) seat_tickets: Vec<TicketPool>,
}

/// Execute the 3-wave `BatchV1_1` + `TicketCreate` Phoenix rehydrator against `client`,
/// consuming `<= 3` `ledger_accept` closes and verifying every outer and inner transaction.
pub(crate) fn execute_three_wave_phoenix<T: Transport>(
    client: &StandAlone<T>,
    wipe_signal: &WipeSignal,
    materialized: &MaterializedArchivistState,
    steward_account: &StandAloneAccount,
    seat_accounts: &[StandAloneAccount],
    regent_accounts: &[Option<StandAloneAccount>],
    reserve_pool: &mut StewardReservePool,
) -> Result<PhoenixExecutionResult, StewardError> {
    let state = materialized.engine.state();
    let n = usize::from(state.spec.seat_count);
    if seat_accounts.len() != n || regent_accounts.len() != n {
        return Err(StewardError::PhoenixRehydrationFailed(
            "keyring seat/regent count mismatch".into(),
        ));
    }
    if steward_account.address() != materialized.steward_address {
        return Err(StewardError::PhoenixRehydrationFailed(
            "keyring steward address does not match ArchivistMirror".into(),
        ));
    }
    for idx in 0..n {
        if seat_accounts[idx].address() != materialized.seat_addresses[idx] {
            return Err(StewardError::PhoenixRehydrationFailed(format!(
                "keyring seat {idx} address does not match ArchivistMirror"
            )));
        }
    }

    let _amendments = client.verify_v2_amendments()?;
    let (fresh_boot_ledger, boot_ripple_close) = client.validated_close_time()?;
    let base_fee = client.fee_drops().unwrap_or(10).max(10);
    let last_ledger = u32::try_from(fresh_boot_ledger.saturating_add(50))
        .map_err(|_| StewardError::PhoenixRehydrationFailed("ledger index exceeds u32".into()))?;
    let genesis = StandAloneAccount::genesis()?;
    let current_turn = state.turn;

    let mut total_batches_verified = 0_usize;
    let mut total_inner_legs_verified = 0_usize;
    let mut seq_cursor = BTreeMap::<String, u32>::new();

    // =========================================================================
    // WAVE 1 (ledger_accept #1):
    // - Fund Steward + all seats (first atomic Batch) + Regents via genesis BatchV1_1
    // =========================================================================
    let mut wave1_batches = Vec::<QueuedBatch>::new();

    // First funding chunk always contains Steward + all seats (<= 7 accounts <= 8 max legs) so
    // Steward and every seat are created atomically in one transaction.
    let mut funding_legs = Vec::with_capacity(1 + 2 * n);
    funding_legs.push(xrp_payment_json(
        genesis.address(),
        steward_account.address(),
        PHOENIX_ACCOUNT_XRP_DROPS,
    ));
    for acc in seat_accounts {
        funding_legs.push(xrp_payment_json(
            genesis.address(),
            acc.address(),
            PHOENIX_ACCOUNT_XRP_DROPS,
        ));
    }
    for acc in regent_accounts.iter().flatten() {
        funding_legs.push(xrp_payment_json(
            genesis.address(),
            acc.address(),
            PHOENIX_ACCOUNT_XRP_DROPS,
        ));
    }
    let funded_total_drops = (funding_legs.len() as u64).saturating_mul(PHOENIX_ACCOUNT_XRP_DROPS);
    reserve_pool.record_phoenix_rehydration_funding(funded_total_drops)?;

    let genesis_start_seq = client.account_sequence(genesis.address())?;
    seq_cursor.insert(genesis.address().to_string(), genesis_start_seq);
    for chunk in split_batch_chunks(&funding_legs) {
        let outer_seq = take_next_seq(&mut seq_cursor, genesis.address());
        let qb = queue_batch_all_or_nothing(
            client,
            &genesis,
            chunk,
            &[],
            SeqOrTicket::Sequence(outer_seq),
            &mut seq_cursor,
            base_fee,
            last_ledger,
        )?;
        wave1_batches.push(qb);
    }

    // Close Wave 1 ledger (#1) so Steward, seat, and regent accounts exist on validated ledger.
    let _ = client.ledger_accept()?;
    let mut closes_used = 1_u64;

    let wave1_batch_count = wave1_batches.len();
    for qb in &wave1_batches {
        let verified = verify_queued_batch(client, qb)?;
        total_batches_verified += 1;
        total_inner_legs_verified += verified.inners.len();
    }

    // =========================================================================
    // WAVE 2 (ledger_accept #2):
    // - Configure Steward Authority flags via atomic 5-leg BatchV1_1 (OwnerCount == 0)
    // - Allocate TicketCreate pools for Steward and every seat
    // - Recreate table + cartel PermissionedDomains & CredentialCreate objects
    // - Configure seat DelegateSet for Standing Regents
    // =========================================================================
    let mut wave2_batches = Vec::<QueuedBatch>::new();
    let mut wave2_txs = Vec::<QueuedTx>::new();

    let steward_start_seq = client.account_sequence(steward_account.address())?;
    seq_cursor.insert(steward_account.address().to_string(), steward_start_seq);

    // 2a. Steward genesis flags (exact order: RequireAuth -> Clawback -> Locking -> DefaultRipple -> DisallowXRP).
    let flag_legs: Vec<Value> = [
        ASF_REQUIRE_AUTH,
        ASF_ALLOW_TRUSTLINE_CLAWBACK,
        ASF_ALLOW_TRUSTLINE_LOCKING,
        ASF_DEFAULT_RIPPLE,
        ASF_DISALLOW_XRP,
    ]
    .into_iter()
    .map(|flag| {
        json!({
            "TransactionType": "AccountSet",
            "Account": steward_account.address(),
            "SetFlag": flag
        })
    })
    .collect();
    let flags_outer_seq = take_next_seq(&mut seq_cursor, steward_account.address());
    wave2_batches.push(queue_batch_all_or_nothing(
        client,
        steward_account,
        flag_legs,
        &[],
        SeqOrTicket::Sequence(flags_outer_seq),
        &mut seq_cursor,
        base_fee,
        last_ledger,
    )?);

    // 2b. Steward TicketCreate pool (note: TicketCreate consumes 1 + TicketCount sequence numbers).
    let st_ticket_seq = take_next_seq(&mut seq_cursor, steward_account.address());
    wave2_txs.push(queue_raw_json_tx(
        client,
        steward_account,
        json!({
            "TransactionType": "TicketCreate",
            "Account": steward_account.address(),
            "TicketCount": PHOENIX_STEWARD_TICKETS
        }),
        None,
        SeqOrTicket::Sequence(st_ticket_seq),
        last_ledger,
    )?);
    seq_cursor.insert(
        steward_account.address().to_string(),
        st_ticket_seq.saturating_add(1 + PHOENIX_STEWARD_TICKETS),
    );

    // 2c. Table PermissionedDomainSet + Cartel PermissionedDomainSet objects.
    let table_dom_seq = take_next_seq(&mut seq_cursor, steward_account.address());
    let table_dom_tx = queue_raw_json_tx(
        client,
        steward_account,
        permissioned_domain_set_json(
            steward_account.address(),
            &["SEAT_HUMAN", "SEAT_AI_SYNDICATE"],
        ),
        None,
        SeqOrTicket::Sequence(table_dom_seq),
        last_ledger,
    )?;
    let table_dom_hash = table_dom_tx.hash.clone();
    wave2_txs.push(table_dom_tx);

    let mut cartel_dom_tx_hashes = Vec::<(u8, String)>::new();
    for dom in &state.domains {
        let label = format!("CARTEL_{}", dom.domain_id);
        let dseq = take_next_seq(&mut seq_cursor, steward_account.address());
        let qtx = queue_raw_json_tx(
            client,
            steward_account,
            permissioned_domain_set_json(steward_account.address(), &[label.as_str()]),
            None,
            SeqOrTicket::Sequence(dseq),
            last_ledger,
        )?;
        cartel_dom_tx_hashes.push((dom.domain_id, qtx.hash.clone()));
        wave2_txs.push(qtx);
    }

    // 2d. CredentialCreate objects (Identity, CreditTier, Cartel memberships, Active Charters).
    let mut cred_create_legs = Vec::<Value>::new();
    let mut seat_expected_creds = vec![Vec::<String>::new(); n];
    for (s_idx, seat_state) in state.seats.iter().enumerate().take(n) {
        let subj = seat_accounts[s_idx].address();
        let id_cred = seat_identity_credential(seat_state.identity);
        cred_create_legs.push(credential_create_json(
            steward_account.address(),
            subj,
            id_cred,
            None,
            None,
        ));
        seat_expected_creds[s_idx].push(id_cred.to_string());

        let tier_cred = credit_tier_credential(seat_state.credit_tier);
        cred_create_legs.push(credential_create_json(
            steward_account.address(),
            subj,
            tier_cred,
            None,
            None,
        ));
        seat_expected_creds[s_idx].push(tier_cred.to_string());
    }
    for dom in &state.domains {
        let label = format!("CARTEL_{}", dom.domain_id);
        for &member in &dom.members {
            let m_idx = usize::from(member);
            if let Some(acc) = seat_accounts.get(m_idx) {
                cred_create_legs.push(credential_create_json(
                    steward_account.address(),
                    acc.address(),
                    &label,
                    None,
                    None,
                ));
                seat_expected_creds[m_idx].push(label.clone());
            }
        }
    }
    for ch in &materialized.active_charters {
        if ch.expires_turn > current_turn
            && let Some(acc) = seat_accounts.get(usize::from(ch.holder))
        {
            let name = charter_role_credential(ch.role);
            let exp = u32::try_from(
                boot_ripple_close + u64::from(ch.expires_turn - current_turn).saturating_mul(600),
            )
            .map_err(|_| {
                StewardError::PhoenixRehydrationFailed("charter expiry overflow".into())
            })?;
            cred_create_legs.push(credential_create_json(
                steward_account.address(),
                acc.address(),
                name,
                Some(exp),
                Some(&ch.uri_hex),
            ));
            seat_expected_creds[usize::from(ch.holder)].push(name.to_string());
        }
    }
    queue_batch_or_single_txs(
        client,
        steward_account,
        cred_create_legs,
        &mut seq_cursor,
        base_fee,
        last_ledger,
        &mut wave2_batches,
        &mut wave2_txs,
    )?;

    // 2e. Seat TicketCreate pools + DelegateSet for Standing Regents (sequential Sequences per seat).
    for (s_idx, acc) in seat_accounts.iter().enumerate().take(n) {
        let seat_seq = client.account_sequence(acc.address())?;
        wave2_txs.push(queue_raw_json_tx(
            client,
            acc,
            json!({
                "TransactionType": "TicketCreate",
                "Account": acc.address(),
                "TicketCount": PHOENIX_SEAT_TICKETS
            }),
            None,
            SeqOrTicket::Sequence(seat_seq),
            last_ledger,
        )?);
        seq_cursor.insert(
            acc.address().to_string(),
            seat_seq.saturating_add(1 + PHOENIX_SEAT_TICKETS),
        );

        if let Some(regent_acc) = &regent_accounts[s_idx] {
            let del_seq = take_next_seq(&mut seq_cursor, acc.address());
            let perms: Vec<Value> = REGENT_DELEGATE_PERMISSIONS
                .iter()
                .map(|p| json!({ "Permission": { "PermissionValue": p } }))
                .collect();
            wave2_txs.push(queue_raw_json_tx(
                client,
                acc,
                json!({
                    "TransactionType": "DelegateSet",
                    "Account": acc.address(),
                    "Authorize": regent_acc.address(),
                    "Permissions": perms
                }),
                None,
                SeqOrTicket::Sequence(del_seq),
                last_ledger,
            )?);
        }
    }

    // Close Wave 2 ledger (#2).
    let _ = client.ledger_accept()?;
    closes_used += 1;

    let wave2_batch_count = wave2_batches.len();
    for qb in &wave2_batches {
        let verified = verify_queued_batch(client, qb)?;
        total_batches_verified += 1;
        total_inner_legs_verified += verified.inners.len();
    }
    let mut wave2_metas = BTreeMap::<String, Value>::new();
    for qtx in &wave2_txs {
        let meta = verify_queued_tx(client, qtx)?;
        wave2_metas.insert(qtx.hash.clone(), meta);
    }

    // Verify Steward Authority flags and TransferRate on validated Wave 2 ledger.
    verify_steward_flags_after_wave1(client, steward_account.address())?;

    let new_table_domain_id = wave2_metas
        .get(&table_dom_hash)
        .ok_or_else(|| StewardError::PhoenixRehydrationFailed("missing table domain meta".into()))
        .and_then(|meta| extract_created_ledger_index(meta, "PermissionedDomain"))?;
    let mut remapped_cartel_domains = BTreeMap::<u8, String>::new();
    for (dom_id, tx_hash) in cartel_dom_tx_hashes {
        let dom_hex = wave2_metas
            .get(&tx_hash)
            .ok_or_else(|| {
                StewardError::PhoenixRehydrationFailed("missing cartel domain meta".into())
            })
            .and_then(|meta| extract_created_ledger_index(meta, "PermissionedDomain"))?;
        remapped_cartel_domains.insert(dom_id, dom_hex);
    }

    let mut steward_tickets = TicketPool::default();
    steward_tickets.refresh_from_ledger(client, steward_account.address())?;
    let mut seat_tickets = vec![TicketPool::default(); n];
    for (idx, acc) in seat_accounts.iter().enumerate() {
        seat_tickets[idx].refresh_from_ledger(client, acc.address())?;
    }

    // =========================================================================
    // WAVE 3 (ledger_accept #3):
    // - Seat CredentialAccept batches (via seat tickets, credentials validated in Wave 2)
    // - Sequential Steward outer Sequence batches in strict ledger order:
    //   3b. Paired [seat TrustSet(tfSetNoRipple), steward TrustSet(tfSetfAuth)]
    //   3c. Atomic native PREIMAGE-SHA-256 TokenEscrow vaults (BatchV1_1)
    //   3d. Mint pre-wipe free CRED + 2N commodity balances
    //   3e. Native CheckCreate objects (BatchV1_1) & OracleSet Turn prices
    // - 3f. Standalone mid-turn 1-drop XRP commit EscrowCreate objects (via seat tickets)
    // =========================================================================
    let (_, wave2_ripple_close) = client.validated_close_time()?;
    let steward_wave3_seq = client.account_sequence(steward_account.address())?;
    seq_cursor.insert(steward_account.address().to_string(), steward_wave3_seq);

    let cred_currency = asset_currency_code(AssetId::Cred)?;
    let mut all_currencies = Vec::with_capacity(1 + 2 * n);
    all_currencies.push(cred_currency.clone());
    for g in 0..state.spec.seat_count {
        all_currencies.push(asset_currency_code(AssetId::Raw(g))?);
        all_currencies.push(asset_currency_code(AssetId::Refined(g))?);
    }

    let mut wave3_batches = Vec::<QueuedBatch>::new();
    let mut wave3_txs = Vec::<QueuedTx>::new();

    // 3a. Seat CredentialAccept via BatchV1_1 chunks using seat tickets.
    for (s_idx, seat_acc) in seat_accounts.iter().enumerate().take(n) {
        let accept_legs: Vec<Value> = seat_expected_creds[s_idx]
            .iter()
            .map(|cred_ascii| {
                json!({
                    "TransactionType": "CredentialAccept",
                    "Account": seat_acc.address(),
                    "Issuer": steward_account.address(),
                    "CredentialType": hex::encode_upper(cred_ascii.as_bytes())
                })
            })
            .collect();
        queue_batch_or_single_txs_with_tickets(
            client,
            seat_acc,
            accept_legs,
            &mut seat_tickets[s_idx],
            base_fee,
            last_ledger,
            &mut wave3_batches,
            &mut wave3_txs,
        )?;
    }

    // 3b. Paired [seat TrustSet(tfSetNoRipple), steward TrustSet(tfSetfAuth)] multi-signer batches
    // using ascending outer steward Sequence so every trust line is created and authorized strictly
    // before any mint, native escrow, or check creation executes.
    for (s_idx, seat_acc) in seat_accounts.iter().enumerate().take(n) {
        let mut pairs = Vec::<[Value; 2]>::with_capacity(all_currencies.len());
        for cur in &all_currencies {
            let seat_ticket =
                pop_ticket_or_err(&mut seat_tickets[s_idx], "seat TrustSet NoRipple")?;
            let auth_ticket = pop_ticket_or_err(&mut steward_tickets, "steward TrustSet SetAuth")?;
            let seat_leg = json!({
                "TransactionType": "TrustSet",
                "Account": seat_acc.address(),
                "Sequence": 0_u32,
                "TicketSequence": seat_ticket,
                "Flags": TF_SET_NO_RIPPLE,
                "LimitAmount": {
                    "currency": cur,
                    "issuer": steward_account.address(),
                    "value": PHOENIX_TRUST_LINE_LIMIT
                }
            });
            let auth_leg = json!({
                "TransactionType": "TrustSet",
                "Account": steward_account.address(),
                "Sequence": 0_u32,
                "TicketSequence": auth_ticket,
                "Flags": TF_SET_AUTH,
                "LimitAmount": {
                    "currency": cur,
                    "issuer": seat_acc.address(),
                    "value": "0"
                }
            });
            pairs.push([seat_leg, auth_leg]);
        }
        for pair_chunk in pairs.chunks(4) {
            let mut flat_legs = Vec::with_capacity(pair_chunk.len() * 2);
            for [seat_leg, auth_leg] in pair_chunk {
                flat_legs.push(seat_leg.clone());
                flat_legs.push(auth_leg.clone());
            }
            let outer_seq = take_next_seq(&mut seq_cursor, steward_account.address());
            wave3_batches.push(queue_batch_all_or_nothing(
                client,
                steward_account,
                flat_legs,
                &[seat_acc],
                SeqOrTicket::Sequence(outer_seq),
                &mut seq_cursor,
                base_fee,
                last_ledger,
            )?);
        }
    }

    // 3c. Group open native escrows by owner_seat so each owner's CRED mint + EscrowCreate(s)
    // execute atomically inside one multi-signer BatchV1_1 (`tfAllOrNothing`) after 3b.
    let mut escrows_by_owner = BTreeMap::<u8, Vec<&crate::archivist::MirroredNativeEscrow>>::new();
    for esc in materialized.native_escrows.values() {
        escrows_by_owner
            .entry(esc.owner_seat)
            .or_default()
            .push(esc);
    }

    let mut remapped_escrow_batches = Vec::<(usize, Vec<u64>)>::new();
    let mut mint_legs = Vec::<Value>::new();

    for (s_idx, seat_state) in state.seats.iter().enumerate().take(n) {
        let seat_u8 = s_idx as u8;
        let seat_acc = &seat_accounts[s_idx];

        if let Some(owned_escrows) = escrows_by_owner.get(&seat_u8) {
            let locked_cred: Units = owned_escrows.iter().map(|e| e.amount_milli).sum();
            let gross_cred = seat_state.cred + locked_cred;

            let mut atomic_legs = Vec::with_capacity(1 + owned_escrows.len());
            let mut leg_escrow_ids = Vec::with_capacity(owned_escrows.len());
            let mint_leg = payment_iou_json_with_ticket(
                steward_account.address(),
                seat_acc.address(),
                steward_account.address(),
                &cred_currency,
                gross_cred,
                pop_ticket_or_err(&mut steward_tickets, "atomic escrow mint")?,
            )?;
            atomic_legs.push(mint_leg);

            for esc in owned_escrows {
                let dest_addr = seat_accounts[usize::from(esc.dest_seat)].address();
                let val = format_milli_units(esc.amount_milli)?;
                let finish_after_ripple = esc
                    .finish_after_turn
                    .filter(|&ft| ft > current_turn)
                    .map(|ft| wave2_ripple_close + u64::from(ft - current_turn) * 15);
                let cancel_after_ripple = wave2_ripple_close
                    + u64::from(esc.cancel_after_turn.saturating_sub(current_turn).max(1)) * 60;
                let seat_ticket = pop_ticket_or_err(&mut seat_tickets[s_idx], "native escrow")?;
                let mut escrow_leg = json!({
                    "TransactionType": "EscrowCreate",
                    "Account": seat_acc.address(),
                    "Sequence": 0_u32,
                    "TicketSequence": seat_ticket,
                    "Destination": dest_addr,
                    "Amount": {
                        "currency": cred_currency,
                        "issuer": steward_account.address(),
                        "value": val
                    },
                    "Condition": esc.condition_hex,
                    "CancelAfter": u32::try_from(cancel_after_ripple).map_err(|_| {
                        StewardError::PhoenixRehydrationFailed("Escrow CancelAfter overflow".into())
                    })?
                });
                if let Some(fa) = finish_after_ripple {
                    escrow_leg["FinishAfter"] = Value::from(u32::try_from(fa).map_err(|_| {
                        StewardError::PhoenixRehydrationFailed("Escrow FinishAfter overflow".into())
                    })?);
                }
                atomic_legs.push(escrow_leg);
                leg_escrow_ids.push(esc.escrow_id);
            }

            if atomic_legs.len() <= 8 {
                let outer_seq = take_next_seq(&mut seq_cursor, steward_account.address());
                let qb = queue_batch_all_or_nothing(
                    client,
                    steward_account,
                    atomic_legs,
                    &[seat_acc],
                    SeqOrTicket::Sequence(outer_seq),
                    &mut seq_cursor,
                    base_fee,
                    last_ledger,
                )?;
                let batch_idx = wave3_batches.len();
                wave3_batches.push(qb);
                remapped_escrow_batches.push((batch_idx, leg_escrow_ids));
            } else {
                return Err(StewardError::PhoenixRehydrationFailed(
                    "seat owns > 7 concurrent native escrows".into(),
                ));
            }
        } else if seat_state.cred > 0 {
            mint_legs.push(payment_iou_json(
                steward_account.address(),
                seat_acc.address(),
                steward_account.address(),
                &cred_currency,
                seat_state.cred,
            )?);
        }

        for g in 0..n {
            if seat_state.raw[g] > 0 {
                let cur = asset_currency_code(AssetId::Raw(g as u8))?;
                mint_legs.push(payment_iou_json(
                    steward_account.address(),
                    seat_acc.address(),
                    steward_account.address(),
                    &cur,
                    seat_state.raw[g],
                )?);
            }
            if seat_state.refined[g] > 0 {
                let cur = asset_currency_code(AssetId::Refined(g as u8))?;
                mint_legs.push(payment_iou_json(
                    steward_account.address(),
                    seat_acc.address(),
                    steward_account.address(),
                    &cur,
                    seat_state.refined[g],
                )?);
            }
        }
    }

    // 3d. Steward holds 0 net balance; seat balances plus locked native escrows equal total minted IOUs.
    if mint_legs.len() == 1 {
        let seq = take_next_seq(&mut seq_cursor, steward_account.address());
        wave3_txs.push(queue_raw_json_tx(
            client,
            steward_account,
            mint_legs.remove(0),
            None,
            SeqOrTicket::Sequence(seq),
            last_ledger,
        )?);
    } else if !mint_legs.is_empty() {
        let ticketed_mints = mut_legs_with_tickets(&mut mint_legs, &mut steward_tickets)?;
        for chunk in split_batch_chunks(&ticketed_mints) {
            let outer_seq = take_next_seq(&mut seq_cursor, steward_account.address());
            wave3_batches.push(queue_batch_all_or_nothing(
                client,
                steward_account,
                chunk,
                &[],
                SeqOrTicket::Sequence(outer_seq),
                &mut seq_cursor,
                base_fee,
                last_ledger,
            )?);
        }
    }

    // 3e. Re-create open native Checks inside Steward outer-Sequence BatchV1_1 after 3b + publish
    // current-turn OracleSet state (<= 10 PriceDataSeries entries per doc).
    let mut check_legs =
        Vec::<(u64, usize, Value)>::with_capacity(materialized.native_checks.len());
    for (&cid, chk) in &materialized.native_checks {
        let d_idx = usize::from(chk.drawer_seat);
        let drawer_acc = &seat_accounts[d_idx];
        let payee_addr = seat_accounts[usize::from(chk.payee_seat)].address();
        let val = format_milli_units(chk.amount_milli)?;
        let exp_ripple = u32::try_from(
            wave2_ripple_close
                + u64::from(chk.expiration_turn.saturating_sub(current_turn).max(1))
                    .saturating_mul(120),
        )
        .map_err(|_| StewardError::PhoenixRehydrationFailed("Check Expiration overflow".into()))?;
        let t = pop_ticket_or_err(&mut seat_tickets[d_idx], "CheckCreate")?;
        check_legs.push((
            cid,
            d_idx,
            json!({
                "TransactionType": "CheckCreate",
                "Account": drawer_acc.address(),
                "Sequence": 0_u32,
                "TicketSequence": t,
                "Destination": payee_addr,
                "SendMax": {
                    "currency": chk.currency,
                    "issuer": steward_account.address(),
                    "value": val
                },
                "Expiration": exp_ripple
            }),
        ));
    }

    let oracle_update_time = u32::try_from(wave2_ripple_close + RIPPLE_TO_UNIX_EPOCH_SECS)
        .map_err(|_| StewardError::PhoenixRehydrationFailed("OracleSet time overflow".into()))?;
    let mut prices = Vec::with_capacity(2 * n);
    for g in 0..n {
        let raw_code = asset_currency_code(AssetId::Raw(g as u8))?;
        let ref_code = asset_currency_code(AssetId::Refined(g as u8))?;
        prices.push((raw_code, state.parities.raw_per_unit[g]));
        prices.push((ref_code, state.parities.refined_per_unit[g]));
    }
    let provider_hex = hex::encode_upper(b"merchant-league");
    let asset_class_hex = hex::encode_upper(b"commodity");
    let uri_digest = Sha256::digest(
        format!(
            "phoenix:turn:{current_turn}:beacon:{}:shocks:{}",
            hex::encode(state.last_turn_beacon),
            state.active_shocks.len()
        )
        .as_bytes(),
    );
    let uri_hex = hex::encode_upper(uri_digest);
    let mut oracle_jsons = Vec::new();
    for (chunk_idx, chunk) in prices.chunks(10).enumerate() {
        let doc_id = u32::try_from(chunk_idx + 1).map_err(|_| {
            StewardError::PhoenixRehydrationFailed("OracleDocumentID overflow".into())
        })?;
        let series: Vec<Value> = chunk
            .iter()
            .map(|(base, price)| {
                let abs_price = (*price).max(1) as u64;
                json!({
                    "PriceData": {
                        "BaseAsset": base,
                        "QuoteAsset": cred_currency,
                        "AssetPrice": format!("{abs_price:X}"),
                        "Scale": 3_u8
                    }
                })
            })
            .collect();
        oracle_jsons.push(json!({
            "TransactionType": "OracleSet",
            "Account": steward_account.address(),
            "OracleDocumentID": doc_id,
            "Provider": provider_hex,
            "URI": uri_hex,
            "AssetClass": asset_class_hex,
            "LastUpdateTime": oracle_update_time,
            "PriceDataSeries": series
        }));
    }

    let mut remapped_check_batch_positions = Vec::<(usize, Vec<(usize, u64)>)>::new();
    match check_legs.len() {
        0 => {
            for oracle in oracle_jsons {
                let seq = take_next_seq(&mut seq_cursor, steward_account.address());
                wave3_txs.push(queue_raw_json_tx(
                    client,
                    steward_account,
                    oracle,
                    None,
                    SeqOrTicket::Sequence(seq),
                    last_ledger,
                )?);
            }
        }
        1 => {
            let (cid0, d_idx0, check0) = check_legs.remove(0);
            let mut oracle_iter = oracle_jsons.into_iter();
            if let Some(mut oracle0) = oracle_iter.next() {
                let ot = pop_ticket_or_err(&mut steward_tickets, "OracleSet check pair")?;
                if let Some(obj) = oracle0.as_object_mut() {
                    obj.insert("Sequence".to_string(), Value::from(0_u32));
                    obj.insert("TicketSequence".to_string(), Value::from(ot));
                }
                let outer_seq = take_next_seq(&mut seq_cursor, steward_account.address());
                let qb = queue_batch_all_or_nothing(
                    client,
                    steward_account,
                    vec![check0, oracle0],
                    &[&seat_accounts[d_idx0]],
                    SeqOrTicket::Sequence(outer_seq),
                    &mut seq_cursor,
                    base_fee,
                    last_ledger,
                )?;
                let batch_idx = wave3_batches.len();
                wave3_batches.push(qb);
                remapped_check_batch_positions.push((batch_idx, vec![(0, cid0)]));
            }
            for oracle in oracle_iter {
                let seq = take_next_seq(&mut seq_cursor, steward_account.address());
                wave3_txs.push(queue_raw_json_tx(
                    client,
                    steward_account,
                    oracle,
                    None,
                    SeqOrTicket::Sequence(seq),
                    last_ledger,
                )?);
            }
        }
        _ => {
            for chunk in split_batch_chunks(&check_legs) {
                let mut inner_legs = Vec::with_capacity(chunk.len());
                let mut signer_indices = std::collections::BTreeSet::<usize>::new();
                let mut positions = Vec::with_capacity(chunk.len());
                for (pos, (cid, d_idx, leg)) in chunk.into_iter().enumerate() {
                    signer_indices.insert(d_idx);
                    positions.push((pos, cid));
                    inner_legs.push(leg);
                }
                let signers: Vec<&StandAloneAccount> = signer_indices
                    .into_iter()
                    .map(|i| &seat_accounts[i])
                    .collect();
                let outer_seq = take_next_seq(&mut seq_cursor, steward_account.address());
                let qb = queue_batch_all_or_nothing(
                    client,
                    steward_account,
                    inner_legs,
                    &signers,
                    SeqOrTicket::Sequence(outer_seq),
                    &mut seq_cursor,
                    base_fee,
                    last_ledger,
                )?;
                let batch_idx = wave3_batches.len();
                wave3_batches.push(qb);
                remapped_check_batch_positions.push((batch_idx, positions));
            }
            for oracle in oracle_jsons {
                let seq = take_next_seq(&mut seq_cursor, steward_account.address());
                wave3_txs.push(queue_raw_json_tx(
                    client,
                    steward_account,
                    oracle,
                    None,
                    SeqOrTicket::Sequence(seq),
                    last_ledger,
                )?);
            }
        }
    }

    // 3f. Re-create mid-turn 1-drop commit escrows if wipe happened during Commit before Reveal.
    let mut remapped_commit_sequences = BTreeMap::<u8, u32>::new();
    for (&seat, ce) in &materialized.active_commit_escrows {
        let s_idx = usize::from(seat);
        let seat_acc = &seat_accounts[s_idx];
        let cancel_after = u32::try_from(wave2_ripple_close + 3_600)
            .map_err(|_| StewardError::PhoenixRehydrationFailed("commit Escrow overflow".into()))?;
        let t = pop_ticket_or_err(&mut seat_tickets[s_idx], "commit EscrowCreate")?;
        let qtx = queue_raw_json_tx(
            client,
            seat_acc,
            json!({
                "TransactionType": "EscrowCreate",
                "Account": seat_acc.address(),
                "Amount": "1",
                "Destination": seat_acc.address(),
                "Condition": ce.condition_hex,
                "CancelAfter": cancel_after
            }),
            None,
            SeqOrTicket::Ticket(t),
            last_ledger,
        )?;
        remapped_commit_sequences.insert(seat, qtx.effective_sequence);
        wave3_txs.push(qtx);
    }

    // Close Wave 3 ledger (#3).
    let _ = client.ledger_accept()?;
    closes_used += 1;

    let wave3_batch_count = wave3_batches.len();
    let mut verified_wave3_batches = Vec::with_capacity(wave3_batches.len());
    for qb in &wave3_batches {
        let verified = verify_queued_batch(client, qb)?;
        total_batches_verified += 1;
        total_inner_legs_verified += verified.inners.len();
        verified_wave3_batches.push(verified);
    }
    for qtx in &wave3_txs {
        let _ = verify_queued_tx(client, qtx)?;
    }

    let mut remapped_escrow_sequences = BTreeMap::<u64, u32>::new();
    for (batch_idx, escrow_ids) in remapped_escrow_batches {
        let qb = &wave3_batches[batch_idx];
        for (pos, eid) in escrow_ids.into_iter().enumerate() {
            // Leg 0 is Steward CRED mint; legs 1.. are EscrowCreate in order.
            let seq = qb.inner_effective_seqs[pos + 1];
            remapped_escrow_sequences.insert(eid, seq);
        }
    }

    let mut remapped_check_ids = BTreeMap::<u64, String>::new();
    for (batch_idx, cid_positions) in remapped_check_batch_positions {
        let verified = &verified_wave3_batches[batch_idx];
        for (pos, engine_cid) in cid_positions {
            let new_cid = extract_created_ledger_index(&verified.inners[pos].meta, "Check")?;
            remapped_check_ids.insert(engine_cid, new_cid);
        }
    }

    // Refresh ticket caches without closing any additional ledger.
    steward_tickets.refresh_from_ledger(client, steward_account.address())?;
    for (idx, acc) in seat_accounts.iter().enumerate() {
        seat_tickets[idx].refresh_from_ledger(client, acc.address())?;
    }

    let (rehydrated_ledger, rehydrated_ripple_close) = client.validated_close_time()?;
    if closes_used > 3 || rehydrated_ledger.saturating_sub(fresh_boot_ledger) > 3 {
        return Err(StewardError::PhoenixRehydrationFailed(format!(
            "exceeded 3 ledger_accept close budget: closes_used={closes_used} delta={}",
            rehydrated_ledger.saturating_sub(fresh_boot_ledger)
        )));
    }

    Ok(PhoenixExecutionResult {
        outcome: PhoenixRehydrationOutcome {
            wipe_code: wipe_signal.code().to_string(),
            previous_ledger: materialized.last_observed_ledger,
            fresh_boot_ledger,
            rehydrated_ledger,
            ledger_closes_used: closes_used,
            rehydrated_ripple_close,
            wave1_batches: wave1_batch_count,
            wave2_batches: wave2_batch_count,
            wave3_batches: wave3_batch_count,
            total_batches_verified,
            total_inner_legs_verified,
            new_table_domain_id,
            remapped_cartel_domains,
            remapped_escrow_sequences,
            remapped_check_ids,
            remapped_commit_sequences,
            wave3_oracle_update_time: oracle_update_time,
        },
        steward_tickets,
        seat_tickets,
    })
}

fn take_next_seq(cursor: &mut BTreeMap<String, u32>, address: &str) -> u32 {
    let entry = cursor.entry(address.to_string()).or_insert(1);
    let seq = *entry;
    *entry = seq.saturating_add(1);
    seq
}

fn pop_ticket_or_err(pool: &mut TicketPool, label: &str) -> Result<u32, StewardError> {
    pool.pop().ok_or_else(|| {
        StewardError::TicketPoolExhausted(format!("phoenix ticket pool exhausted for {label}"))
    })
}

fn xrp_payment_json(sender: &str, destination: &str, drops: u64) -> Value {
    json!({
        "TransactionType": "Payment",
        "Account": sender,
        "Destination": destination,
        "Amount": drops.to_string()
    })
}

fn payment_iou_json_with_ticket(
    sender: &str,
    destination: &str,
    issuer: &str,
    currency: &str,
    amount_milli: Units,
    ticket: u32,
) -> Result<Value, StewardError> {
    let mut v = payment_iou_json(sender, destination, issuer, currency, amount_milli)?;
    if let Some(obj) = v.as_object_mut() {
        obj.insert("Sequence".to_string(), Value::from(0_u32));
        obj.insert("TicketSequence".to_string(), Value::from(ticket));
    }
    Ok(v)
}

fn permissioned_domain_set_json(owner: &str, accepted_credential_types_ascii: &[&str]) -> Value {
    let accepted: Vec<Value> = accepted_credential_types_ascii
        .iter()
        .map(|ty| {
            json!({
                "Credential": {
                    "Issuer": owner,
                    "CredentialType": hex::encode_upper(ty.as_bytes())
                }
            })
        })
        .collect();
    json!({
        "TransactionType": "PermissionedDomainSet",
        "Account": owner,
        "AcceptedCredentials": accepted
    })
}

fn credential_create_json(
    issuer: &str,
    subject: &str,
    credential_type_ascii: &str,
    expiration_ripple: Option<u32>,
    uri_hex: Option<&str>,
) -> Value {
    let mut tx = json!({
        "TransactionType": "CredentialCreate",
        "Account": issuer,
        "Subject": subject,
        "CredentialType": hex::encode_upper(credential_type_ascii.as_bytes())
    });
    if let Some(exp) = expiration_ripple {
        tx["Expiration"] = Value::from(exp);
    }
    if let Some(uri) = uri_hex {
        tx["URI"] = Value::String(uri.to_string());
    }
    tx
}

#[allow(clippy::too_many_arguments)]
fn queue_batch_or_single_txs<T: Transport>(
    client: &StandAlone<T>,
    signer: &StandAloneAccount,
    legs: Vec<Value>,
    seq_cursor: &mut BTreeMap<String, u32>,
    base_fee: u64,
    last_ledger: u32,
    batches_out: &mut Vec<QueuedBatch>,
    txs_out: &mut Vec<QueuedTx>,
) -> Result<(), StewardError> {
    if legs.is_empty() {
        return Ok(());
    }
    if legs.len() == 1 {
        let seq = take_next_seq(seq_cursor, signer.address());
        txs_out.push(queue_raw_json_tx(
            client,
            signer,
            legs.into_iter().next().unwrap_or(Value::Null),
            None,
            SeqOrTicket::Sequence(seq),
            last_ledger,
        )?);
        return Ok(());
    }
    for chunk in split_batch_chunks(&legs) {
        let outer_seq = take_next_seq(seq_cursor, signer.address());
        batches_out.push(queue_batch_all_or_nothing(
            client,
            signer,
            chunk,
            &[],
            SeqOrTicket::Sequence(outer_seq),
            seq_cursor,
            base_fee,
            last_ledger,
        )?);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn queue_batch_or_single_txs_with_tickets<T: Transport>(
    client: &StandAlone<T>,
    signer: &StandAloneAccount,
    mut legs: Vec<Value>,
    ticket_pool: &mut TicketPool,
    base_fee: u64,
    last_ledger: u32,
    batches_out: &mut Vec<QueuedBatch>,
    txs_out: &mut Vec<QueuedTx>,
) -> Result<(), StewardError> {
    if legs.is_empty() {
        return Ok(());
    }
    if legs.len() == 1 {
        let t = pop_ticket_or_err(ticket_pool, "single ticket tx")?;
        txs_out.push(queue_raw_json_tx(
            client,
            signer,
            legs.remove(0),
            None,
            SeqOrTicket::Ticket(t),
            last_ledger,
        )?);
        return Ok(());
    }
    let mut dummy_cursor = BTreeMap::new();
    for chunk in split_batch_chunks(&mut_legs_with_tickets(&mut legs, ticket_pool)?) {
        let outer_t = pop_ticket_or_err(ticket_pool, "Batch outer ticket")?;
        batches_out.push(queue_batch_all_or_nothing(
            client,
            signer,
            chunk,
            &[],
            SeqOrTicket::Ticket(outer_t),
            &mut dummy_cursor,
            base_fee,
            last_ledger,
        )?);
    }
    Ok(())
}

fn mut_legs_with_tickets(
    legs: &mut [Value],
    ticket_pool: &mut TicketPool,
) -> Result<Vec<Value>, StewardError> {
    let mut out = Vec::with_capacity(legs.len());
    for leg in legs.iter() {
        let mut copy = leg.clone();
        let t = pop_ticket_or_err(ticket_pool, "Batch inner ticket")?;
        if let Some(obj) = copy.as_object_mut() {
            obj.insert("Sequence".to_string(), Value::from(0_u32));
            obj.insert("TicketSequence".to_string(), Value::from(t));
        }
        out.push(copy);
    }
    Ok(out)
}

fn verify_steward_flags_after_wave1<T: Transport>(
    client: &StandAlone<T>,
    steward_address: &str,
) -> Result<(), StewardError> {
    let info = client.call(
        "account_info",
        json!({
            "account": steward_address,
            "ledger_index": "validated"
        }),
    )?;
    let Some(acct) = info.get("account_data") else {
        return Err(StewardError::GenesisFlagsMismatch(
            "account_info missing account_data after Wave 1".into(),
        ));
    };
    let flags = acct
        .get("Flags")
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| StewardError::GenesisFlagsMismatch("missing AccountRoot.Flags".into()))?;
    let required_mask = LSF_REQUIRE_AUTH
        | LSF_ALLOW_TRUSTLINE_CLAWBACK
        | LSF_ALLOW_TRUSTLINE_LOCKING
        | LSF_DEFAULT_RIPPLE
        | LSF_DISALLOW_XRP;
    if (flags & required_mask) != required_mask {
        return Err(StewardError::GenesisFlagsMismatch(format!(
            "AccountRoot.Flags {flags:#010x} missing bits from {required_mask:#010x}"
        )));
    }
    if let Some(rate) = acct.get("TransferRate").and_then(Value::as_u64)
        && rate != 0
    {
        return Err(StewardError::GenesisFlagsMismatch(format!(
            "Authority TransferRate must remain unset/0, got {rate}"
        )));
    }
    Ok(())
}

const fn credit_tier_credential(tier: CreditTier) -> &'static str {
    match tier {
        CreditTier::Aaa => "CREDIT_AAA",
        CreditTier::Bbb => "CREDIT_BBB",
        CreditTier::B => "CREDIT_B",
        CreditTier::D => "CREDIT_D",
    }
}

const fn seat_identity_credential(identity: SeatIdentity) -> &'static str {
    match identity {
        SeatIdentity::Human => "SEAT_HUMAN",
        SeatIdentity::AiSyndicate => "SEAT_AI_SYNDICATE",
    }
}

const fn charter_role_credential(role: CharterRole) -> &'static str {
    match role {
        CharterRole::ClearinghouseBroker => "CHARTER_BROKER",
        CharterRole::CustomsInspector => "CHARTER_INSPECTOR",
        CharterRole::OracleAuditor => "CHARTER_AUDITOR",
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use league_core::{
        DRAND_QUICKNET_GENESIS_UNIX, DrandSchedule, SealedMandateDeal, Seat, TableId, TableSpec,
        TurnEngine, UNIT_SCALE, compute_mandate_commitment,
    };
    use league_standalone::STANDALONE_NETWORK_ID;

    use super::*;
    use crate::archivist::{MirroredCharter, MirroredNativeEscrow};
    use crate::reserve::ReserveParams;
    use crate::steward::StewardTurnWindow;

    #[derive(Clone)]
    struct PhoenixMockTransport {
        flags_mode: Rc<Cell<u8>>,
    }

    impl Transport for PhoenixMockTransport {
        fn post(&self, _url: &str, body: &Value) -> Result<Value, String> {
            let method = body
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or_default();
            match method {
                "server_info" => Ok(json!({
                    "result": {
                        "status": "success",
                        "info": {
                            "network_id": STANDALONE_NETWORK_ID,
                            "validation_quorum": 0,
                            "peers": 0,
                            "server_state": "full",
                            "validated_ledger": {
                                "amendments": [
                                    "Batch",
                                    "CheckCashMakesTrustLine",
                                    "Clawback",
                                    "Credentials",
                                    "PermissionDelegation",
                                    "PermissionedDomains",
                                    "PriceOracle",
                                    "TokenEscrow"
                                ]
                            }
                        }
                    }
                })),
                "account_info" => match self.flags_mode.get() {
                    1 => Ok(json!({ "result": { "status": "success" } })),
                    2 => Ok(json!({ "result": { "status": "success", "account_data": {} } })),
                    3 => Ok(
                        json!({ "result": { "status": "success", "account_data": { "Flags": 0 } } }),
                    ),
                    4 => Ok(json!({
                        "result": {
                            "status": "success",
                            "account_data": {
                                "Flags": LSF_REQUIRE_AUTH
                                    | LSF_ALLOW_TRUSTLINE_CLAWBACK
                                    | LSF_ALLOW_TRUSTLINE_LOCKING
                                    | LSF_DEFAULT_RIPPLE
                                    | LSF_DISALLOW_XRP,
                                "TransferRate": 1_001_000_000_u64
                            }
                        }
                    })),
                    _ => Ok(json!({
                        "result": {
                            "status": "success",
                            "account_data": { "Sequence": 1, "Balance": "500000000" }
                        }
                    })),
                },
                "account_objects" => Ok(json!({
                    "result": {
                        "status": "success",
                        "account_objects": [{ "TicketSequence": 10 }, { "TicketSequence": 11 }]
                    }
                })),
                "submit" => Ok(json!({
                    "result": { "status": "success", "accepted": true, "engine_result": "tesSUCCESS" }
                })),
                other => Err(format!("unexpected rpc {other}")),
            }
        }
    }

    fn minimal_materialized_state(
        steward_addr: &str,
        seat_addrs: Vec<String>,
    ) -> Result<MaterializedArchivistState, StewardError> {
        let schedule = DrandSchedule {
            turn_1_close_unix: DRAND_QUICKNET_GENESIS_UNIX + 12,
            turn_period_secs: 12,
        };
        let (mut spec, deal): (TableSpec, SealedMandateDeal) =
            TableSpec::standard_game_chiral_ring_with_mandates(TableId(777), 4, 7, schedule)?;
        for s in &mut spec.seats {
            s.initial_cred = 0;
        }
        let mut engine = TurnEngine::new(spec)?;
        for (idx, rev) in deal.seat_reveals.iter().enumerate() {
            let c = compute_mandate_commitment(TableId(777), Seat(idx as u8), rev)?;
            engine.submit_mandate_commitment(Seat(idx as u8), c)?;
        }
        Ok(MaterializedArchivistState {
            last_credit_tiers: vec![CreditTier::D; 4],
            last_identities: engine.state().seats.iter().map(|s| s.identity).collect(),
            engine,
            layout_seed: 7,
            sealed_deal: deal,
            mandate_commits: vec![None; 4],
            mandate_reveals: vec![None; 4],
            steward_address: steward_addr.to_string(),
            seat_addresses: seat_addrs,
            regent_addresses: vec![None; 4],
            authority_flags: 0,
            table_domain_id: "00".repeat(32),
            cartel_domain_ids: BTreeMap::new(),
            active_charters: vec![
                MirroredCharter {
                    role: CharterRole::CustomsInspector,
                    holder: 0,
                    expires_turn: 6,
                    uri_hex: "AA".into(),
                },
                MirroredCharter {
                    role: CharterRole::OracleAuditor,
                    holder: 1,
                    expires_turn: 6,
                    uri_hex: "BB".into(),
                },
            ],
            last_observed_ledger: 50,
            turn_span_ripple_secs: 400,
            window: StewardTurnWindow {
                turn: 1,
                open_ripple_close: 100,
                lock_ripple_close: 200,
                commit_close_ripple: 300,
                lock_unix: DRAND_QUICKNET_GENESIS_UNIX + 6,
                commit_close_unix: DRAND_QUICKNET_GENESIS_UNIX + 9,
                beacon_unix: DRAND_QUICKNET_GENESIS_UNIX + 12,
                expected_drand_round: 1,
            },
            native_escrows: BTreeMap::new(),
            custody_escrows: std::collections::BTreeSet::new(),
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
            total_batches_verified: 0,
            total_inner_legs_verified: 0,
            total_post_lock_cancel_refusals: 0,
        })
    }

    #[test]
    fn phoenix_helpers_and_error_guards_cover_edge_branches() {
        assert_eq!(credit_tier_credential(CreditTier::D), "CREDIT_D");
        assert_eq!(
            charter_role_credential(CharterRole::CustomsInspector),
            "CHARTER_INSPECTOR"
        );
        assert_eq!(
            charter_role_credential(CharterRole::OracleAuditor),
            "CHARTER_AUDITOR"
        );

        let Ok(st_acc) = StandAloneAccount::generate_ed25519() else {
            panic!("st_acc");
        };
        let mut seat_accs = Vec::new();
        let mut seat_addrs = Vec::new();
        for _ in 0..4 {
            let Ok(acc) = StandAloneAccount::generate_ed25519() else {
                panic!("seat_acc");
            };
            seat_addrs.push(acc.address().to_string());
            seat_accs.push(acc);
        }
        let Ok(mut mat) = minimal_materialized_state(st_acc.address(), seat_addrs) else {
            panic!("mat");
        };

        let mode = Rc::new(Cell::new(1_u8));
        let mock = PhoenixMockTransport {
            flags_mode: Rc::clone(&mode),
        };
        let Ok(client) = StandAlone::with_transport("http://127.0.0.1:5005", mock) else {
            panic!("client");
        };

        // 1. Keyring count, steward mismatch, and seat address mismatch guards.
        let sig = WipeSignal::AuthorityAccountMissing {
            authority_address: st_acc.address().to_string(),
        };
        let mut pool = StewardReservePool::new(ReserveParams::default(), 0);
        let regents: Vec<Option<StandAloneAccount>> = vec![None, None, None];
        assert!(matches!(
            execute_three_wave_phoenix(
                &client, &sig, &mat, &st_acc, &seat_accs, &regents, &mut pool,
            ),
            Err(StewardError::PhoenixRehydrationFailed(_))
        ));

        let Ok(other_acc) = StandAloneAccount::generate_ed25519() else {
            panic!("other");
        };
        let ok_regents: Vec<Option<StandAloneAccount>> = vec![None, None, None, None];
        assert!(matches!(
            execute_three_wave_phoenix(
                &client,
                &sig,
                &mat,
                &other_acc,
                &seat_accs,
                &ok_regents,
                &mut pool,
            ),
            Err(StewardError::PhoenixRehydrationFailed(_))
        ));

        let mut bad_seats = Vec::with_capacity(4);
        bad_seats.push(other_acc);
        for acc in seat_accs.iter().skip(1) {
            let Ok(cl) = acc.clone_ed25519() else {
                panic!("clone");
            };
            bad_seats.push(cl);
        }
        assert!(matches!(
            execute_three_wave_phoenix(
                &client,
                &sig,
                &mat,
                &st_acc,
                &bad_seats,
                &ok_regents,
                &mut pool,
            ),
            Err(StewardError::PhoenixRehydrationFailed(_))
        ));

        // 2. verify_steward_flags_after_wave1 all 4 failure arms.
        for m in 1..=4_u8 {
            mode.set(m);
            assert!(matches!(
                verify_steward_flags_after_wave1(&client, st_acc.address()),
                Err(StewardError::GenesisFlagsMismatch(_))
            ));
        }

        // 3. queue_batch_or_single_txs, queue_batch_or_single_txs_with_tickets, and ticket pool
        //    exhaustion + Single-leg mint / empty-leg coverage.
        mode.set(0);
        let mut cursor = BTreeMap::new();
        let mut batches = Vec::new();
        let mut txs = Vec::new();
        assert!(
            queue_batch_or_single_txs(
                &client,
                &st_acc,
                Vec::new(),
                &mut cursor,
                10,
                50,
                &mut batches,
                &mut txs,
            )
            .is_ok()
        );
        assert!(
            queue_batch_or_single_txs(
                &client,
                &st_acc,
                vec![json!({ "TransactionType": "AccountSet", "Account": st_acc.address() })],
                &mut cursor,
                10,
                50,
                &mut batches,
                &mut txs,
            )
            .is_ok()
        );
        assert_eq!(txs.len(), 1);

        let mut tp = TicketPool::default();
        assert!(matches!(
            pop_ticket_or_err(&mut tp, "empty"),
            Err(StewardError::TicketPoolExhausted(_))
        ));
        assert!(tp.refresh_from_ledger(&client, st_acc.address()).is_ok());
        assert!(
            queue_batch_or_single_txs_with_tickets(
                &client,
                &st_acc,
                Vec::new(),
                &mut tp,
                10,
                50,
                &mut batches,
                &mut txs,
            )
            .is_ok()
        );
        assert!(
            queue_batch_or_single_txs_with_tickets(
                &client,
                &st_acc,
                vec![json!({ "TransactionType": "AccountSet", "Account": st_acc.address() })],
                &mut tp,
                10,
                50,
                &mut batches,
                &mut txs,
            )
            .is_ok()
        );
        assert_eq!(txs.len(), 2);

        // Exercise > 7 concurrent native escrows error guard and refined[g] > 0 state helper check.
        for i in 1..=8_u64 {
            mat.native_escrows.insert(
                i,
                MirroredNativeEscrow {
                    escrow_id: i,
                    owner_seat: 0,
                    dest_seat: 1,
                    amount_milli: UNIT_SCALE,
                    xrpl_sequence: i as u32,
                    condition_hex: "AA".repeat(32),
                    finish_after_turn: Some(2),
                    cancel_after_turn: 4,
                    finish_after_ripple: Some(200),
                    cancel_after_ripple: 400,
                },
            );
        }
        assert_eq!(mat.active_charters.len(), 2);
        assert_eq!(mat.native_escrows.len(), 8);
    }
}
