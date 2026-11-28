//! Low-level XRPL `BatchV1_1`, `DelegateSet`, `TicketCreate`, `Credentials`, `PermissionedDomains`,
//! and `OracleSet` transaction builders and verification helpers.

use std::collections::VecDeque;

use league_core::Units;
use league_ledger::client::{Transport, decode_hex, transaction_hash};
use league_standalone::STANDALONE_NETWORK_ID;
use league_standalone::client::{BASE_FEE_DROPS, StandAlone, StandAloneAccount, StandAloneError};
use league_standalone::conformance::build_tx;
use serde_json::{Value, json};
use xrpl::core::addresscodec::decode_classic_address;
use xrpl::core::binarycodec::encode;
use xrpl::models::transactions::Credential;
use xrpl::models::transactions::account_set::AccountSet;
use xrpl::models::transactions::check_cancel::CheckCancel;
use xrpl::models::transactions::check_cash::CheckCash;
use xrpl::models::transactions::check_create::CheckCreate;
use xrpl::models::transactions::clawback::Clawback;
use xrpl::models::transactions::credential_accept::CredentialAccept;
use xrpl::models::transactions::credential_create::CredentialCreate;
use xrpl::models::transactions::credential_delete::CredentialDelete;
use xrpl::models::transactions::escrow_cancel::EscrowCancel;
use xrpl::models::transactions::escrow_finish::EscrowFinish;
use xrpl::models::transactions::payment::Payment;
use xrpl::models::transactions::permissioned_domain_set::PermissionedDomainSet;
use xrpl::models::transactions::ticket_create::TicketCreate;
use xrpl::models::transactions::trust_set::TrustSet;

use crate::amount::format_milli_units;
use crate::error::StewardError;

/// `AccountSet` flag enabling `RequireAuth` (`lsfRequireAuth`).
pub const ASF_REQUIRE_AUTH: u32 = 2;
/// `AccountSet` flag disabling incoming XRP (`lsfDisallowXRP`).
pub const ASF_DISALLOW_XRP: u32 = 3;
/// `AccountSet` flag enabling `DefaultRipple` (`lsfDefaultRipple`).
pub const ASF_DEFAULT_RIPPLE: u32 = 8;
/// `AccountSet` flag enabling issuer `Clawback` (`lsfAllowTrustLineClawback`).
pub const ASF_ALLOW_TRUSTLINE_CLAWBACK: u32 = 16;
/// `AccountSet` flag enabling `TokenEscrow` locking (`lsfAllowTrustLineLocking`).
pub const ASF_ALLOW_TRUSTLINE_LOCKING: u32 = 17;

/// `AccountRoot` ledger flag bit for `lsfRequireAuth`.
pub const LSF_REQUIRE_AUTH: u32 = 0x0004_0000;
/// `AccountRoot` ledger flag bit for `lsfDisallowXRP`.
pub const LSF_DISALLOW_XRP: u32 = 0x0008_0000;
/// `AccountRoot` ledger flag bit for `lsfDefaultRipple`.
pub const LSF_DEFAULT_RIPPLE: u32 = 0x0080_0000;
/// `AccountRoot` ledger flag bit for `lsfAllowTrustLineLocking`.
pub const LSF_ALLOW_TRUSTLINE_LOCKING: u32 = 0x4000_0000;
/// `AccountRoot` ledger flag bit for `lsfAllowTrustLineClawback`.
pub const LSF_ALLOW_TRUSTLINE_CLAWBACK: u32 = 0x8000_0000;

/// `TrustSet` flag authorizing a counterparty's trust line (`tfSetfAuth`).
pub const TF_SET_AUTH: u32 = 0x0001_0000;
/// `TrustSet` flag disabling rippling on a seat's trust line (`tfSetNoRipple`).
pub const TF_SET_NO_RIPPLE: u32 = 0x0002_0000;

/// `Batch` (`XLS-56`) flag requiring atomic all-or-nothing execution across `2..=8` inner txs.
pub const TF_BATCH_ALL_OR_NOTHING: u32 = 0x0001_0000;
/// Universal transaction flag marking an inner `Batch` transaction (`tfInnerBatchTxn`).
pub const TF_INNER_BATCH_TXN: u32 = 0x4000_0000;

/// Maximum inner transactions permitted inside one `BatchV1_1` envelope.
pub const MAX_BATCH_INNER_TXS: usize = 8;

/// Scoped transaction permissions granted to a seat's Standing Regent (`PermissionDelegationV1_1`).
pub const REGENT_DELEGATE_PERMISSIONS: &[&str] = &[
    "Payment",
    "EscrowCreate",
    "EscrowFinish",
    "CheckCash",
    "OfferCreate",
    "OfferCancel",
];

/// Pre-allocated XRPL `TicketCreate` sequence queue for one account.
#[derive(Debug, Clone, Default)]
pub struct TicketPool {
    available: VecDeque<u32>,
}

impl TicketPool {
    /// Number of unspent ticket sequences currently cached locally.
    #[must_use]
    pub fn remaining(&self) -> usize {
        self.available.len()
    }

    /// Pop the next pre-allocated ticket sequence if one is available.
    pub fn pop(&mut self) -> Option<u32> {
        self.available.pop_front()
    }

    /// Allocate `count` new tickets via `TicketCreate` (`1..=250`) and synchronize from
    /// `account_objects` (`type = "ticket"`).
    pub fn allocate<T: Transport>(
        &mut self,
        client: &StandAlone<T>,
        account: &StandAloneAccount,
        count: u32,
    ) -> Result<(), StewardError> {
        if count == 0 || count > 250 {
            return Err(StewardError::TicketPoolExhausted(format!(
                "TicketCount {count} must be in 1..=250"
            )));
        }
        let mut tx: TicketCreate<'static> =
            build_tx("TicketCreate", account, json!({ "TicketCount": count }))?;
        let _ = client.submit_and_accept_ok(&mut tx, account, None)?;
        self.refresh_from_ledger(client, account.address())
    }

    /// Reload live unused ticket sequences from validated `account_objects`.
    pub fn refresh_from_ledger<T: Transport>(
        &mut self,
        client: &StandAlone<T>,
        address: &str,
    ) -> Result<(), StewardError> {
        let reply = client.call(
            "account_objects",
            json!({
                "account": address,
                "type": "ticket",
                "ledger_index": "validated",
                "limit": 400
            }),
        )?;
        let Some(objs) = reply.get("account_objects").and_then(Value::as_array) else {
            return Err(StewardError::TicketPoolExhausted(
                "account_objects reply missing account_objects array".into(),
            ));
        };
        let mut seqs = Vec::with_capacity(objs.len());
        for obj in objs {
            if let Some(seq_u64) = obj.get("TicketSequence").and_then(Value::as_u64)
                && let Ok(seq) = u32::try_from(seq_u64)
            {
                seqs.push(seq);
            }
        }
        seqs.sort_unstable();
        seqs.dedup();
        self.available = VecDeque::from(seqs);
        Ok(())
    }

    /// Ensure at least `min_remaining` tickets exist, replenishing `replenish_count` if low.
    pub fn ensure_headroom<T: Transport>(
        &mut self,
        client: &StandAlone<T>,
        account: &StandAloneAccount,
        min_remaining: usize,
        replenish_count: u32,
    ) -> Result<(), StewardError> {
        if self.available.len() < min_remaining {
            self.allocate(client, account, replenish_count)?;
        }
        if self.available.is_empty() {
            return Err(StewardError::TicketPoolExhausted(format!(
                "no tickets available for {} after TicketCreate",
                account.address()
            )));
        }
        Ok(())
    }
}

/// Verified inner transaction metadata returned for every leg of a `BatchV1_1` execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedBatchInner {
    /// Pre-computed inner transaction hash (uppercase 64-hex).
    pub inner_hash: String,
    /// Outer `Batch` transaction hash reported in `ParentBatchID`.
    pub parent_batch_id: String,
    /// Validated `meta.TransactionResult` (always `"tesSUCCESS"`).
    pub result: String,
    /// Validated ledger index holding this inner transaction.
    pub ledger_index: u64,
}

/// Verified result of an atomic `BatchV1_1` (`tfAllOrNothing`) submission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedBatchResult {
    /// Outer `Batch` transaction hash (uppercase 64-hex).
    pub outer_hash: String,
    /// Validated ledger index holding the outer `Batch`.
    pub ledger_index: u64,
    /// Verified metadata proof for every inner transaction (`len == 2..=8`).
    pub inners: Vec<VerifiedBatchInner>,
}

/// Split `N >= 2` settlement legs into `BatchV1_1` chunks of `2..=8` legs (never emitting a
/// length-`1` chunk because XRPL `Batch` requires at least `2` `RawTransactions`).
#[must_use]
pub fn split_batch_chunks<T: Clone>(items: &[T]) -> Vec<Vec<T>> {
    if items.len() <= MAX_BATCH_INNER_TXS {
        return vec![items.to_vec()];
    }
    let mut chunks = Vec::new();
    let mut start = 0usize;
    while start < items.len() {
        let remaining = items.len() - start;
        let take = if remaining <= MAX_BATCH_INNER_TXS {
            remaining
        } else if remaining == MAX_BATCH_INNER_TXS + 1 {
            // Split 9 -> 7 + 2 so the final chunk still satisfies `>= 2`.
            MAX_BATCH_INNER_TXS - 1
        } else {
            MAX_BATCH_INNER_TXS
        };
        chunks.push(items[start..start + take].to_vec());
        start += take;
    }
    chunks
}

/// Submit an arbitrary JSON transaction (such as `DelegateSet` or delegated execution), using
/// `signer` to sign (`Account` may differ from `signer` when `Delegate` is set).
pub fn submit_raw_json_tx<T: Transport>(
    client: &StandAlone<T>,
    signer: &StandAloneAccount,
    mut tx_json: Value,
    fee_drops: Option<&str>,
    ticket_sequence: Option<u32>,
) -> Result<(String, Value), StewardError> {
    let account_addr = tx_json
        .get("Account")
        .and_then(Value::as_str)
        .ok_or_else(|| StewardError::Protocol("raw transaction JSON missing Account".into()))?
        .to_string();
    let (validated_seq, _) = client.validated_close_time()?;
    let last_ledger = u32::try_from(validated_seq.saturating_add(20))
        .map_err(|_| StewardError::Protocol("ledger sequence exceeds u32".into()))?;
    let obj = tx_json
        .as_object_mut()
        .ok_or_else(|| StewardError::Protocol("raw transaction must be JSON object".into()))?;
    obj.insert("NetworkID".to_string(), Value::from(STANDALONE_NETWORK_ID));
    obj.entry("Flags").or_insert_with(|| Value::from(0_u32));
    obj.insert(
        "Fee".to_string(),
        Value::String(fee_drops.unwrap_or(BASE_FEE_DROPS).to_string()),
    );
    obj.insert("LastLedgerSequence".to_string(), Value::from(last_ledger));
    if let Some(ticket) = ticket_sequence {
        obj.insert("Sequence".to_string(), Value::from(0_u32));
        obj.insert("TicketSequence".to_string(), Value::from(ticket));
    } else if !obj.contains_key("Sequence") {
        let seq = client.account_sequence(&account_addr)?;
        obj.insert("Sequence".to_string(), Value::from(seq));
    }

    let (blob_hex, local_hash) = signer.sign_tx_json(&mut tx_json)?;
    let submit_reply = client.call("submit", json!({ "tx_blob": blob_hex }))?;
    let preliminary = submit_reply
        .get("engine_result")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let accepted = submit_reply
        .get("accepted")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    if !accepted {
        return Ok((
            local_hash,
            json!({
                "validated": false,
                "preliminary": preliminary,
                "meta": { "TransactionResult": preliminary }
            }),
        ));
    }
    let _ = client.ledger_accept()?;
    match client.call("tx", json!({ "transaction": local_hash })) {
        Ok(tx_reply) => Ok((local_hash, tx_reply)),
        Err(StandAloneError::Rpc { error, .. }) if error == "txnNotFound" => Ok((
            local_hash,
            json!({
                "validated": false,
                "preliminary": preliminary,
                "meta": { "TransactionResult": preliminary }
            }),
        )),
        Err(err) => Err(StewardError::StandAlone(err)),
    }
}

/// Submit an atomic `BatchV1_1` (`tfAllOrNothing`) envelope over `2..=8` inner transactions and
/// verify **both** the outer `Batch` and **every pre-computed inner transaction hash** (`validated`,
/// `ParentBatchID == outer_hash`, `meta.TransactionResult == "tesSUCCESS"`).
pub fn submit_batch_all_or_nothing<T: Transport>(
    client: &StandAlone<T>,
    outer_signer: &StandAloneAccount,
    inner_templates: Vec<Value>,
    extra_signers: &[&StandAloneAccount],
    outer_ticket: Option<u32>,
) -> Result<VerifiedBatchResult, StewardError> {
    if !(2..=MAX_BATCH_INNER_TXS).contains(&inner_templates.len()) {
        return Err(StewardError::Protocol(format!(
            "BatchV1_1 requires 2..={MAX_BATCH_INNER_TXS} inner transactions, got {}",
            inner_templates.len()
        )));
    }

    let outer_seq = if outer_ticket.is_some() {
        0
    } else {
        client.account_sequence(outer_signer.address())?
    };
    let mut next_outer_acct_seq = if outer_ticket.is_some() {
        client.account_sequence(outer_signer.address())?
    } else {
        outer_seq.saturating_add(1)
    };
    let mut per_account_next_seq = std::collections::BTreeMap::<String, u32>::new();

    let mut raw_transactions = Vec::with_capacity(inner_templates.len());
    let mut inner_hashes = Vec::with_capacity(inner_templates.len());
    let mut distinct_inner_accounts = std::collections::BTreeSet::<String>::new();

    for mut inner in inner_templates {
        let acct = inner
            .get("Account")
            .and_then(Value::as_str)
            .ok_or_else(|| StewardError::Protocol("inner Batch tx missing Account".into()))?
            .to_string();
        distinct_inner_accounts.insert(acct.clone());

        let base_flags = inner.get("Flags").and_then(Value::as_u64).unwrap_or(0) as u32;
        let obj = inner
            .as_object_mut()
            .ok_or_else(|| StewardError::Protocol("inner Batch tx must be JSON object".into()))?;
        obj.insert(
            "Flags".to_string(),
            Value::from(base_flags | TF_INNER_BATCH_TXN),
        );
        obj.insert("Fee".to_string(), Value::String("0".to_string()));
        obj.insert("SigningPubKey".to_string(), Value::String(String::new()));
        obj.insert("NetworkID".to_string(), Value::from(STANDALONE_NETWORK_ID));
        obj.remove("TxnSignature");
        obj.remove("LastLedgerSequence");

        if !obj.contains_key("Sequence") && !obj.contains_key("TicketSequence") {
            let seq = if acct == outer_signer.address() {
                let s = next_outer_acct_seq;
                next_outer_acct_seq = next_outer_acct_seq.saturating_add(1);
                s
            } else {
                let entry = match per_account_next_seq.get(&acct).copied() {
                    Some(s) => s,
                    None => client.account_sequence(&acct)?,
                };
                per_account_next_seq.insert(acct.clone(), entry.saturating_add(1));
                entry
            };
            obj.insert("Sequence".to_string(), Value::from(seq));
        }

        let inner_blob = encode(&inner)
            .map_err(|err| StewardError::Protocol(format!("inner Batch binary encode: {err}")))?;
        let inner_bytes = decode_hex(&inner_blob).map_err(StewardError::Protocol)?;
        let inner_hash = transaction_hash(&inner_bytes);
        inner_hashes.push(inner_hash);
        raw_transactions.push(json!({ "RawTransaction": inner }));
    }

    let mut batch_signers_with_keys: Vec<(Vec<u8>, Value)> = Vec::new();
    for signer_addr in &distinct_inner_accounts {
        if signer_addr == outer_signer.address() {
            continue;
        }
        let signer = extra_signers
            .iter()
            .find(|acc| acc.address() == signer_addr)
            .ok_or_else(|| {
                StewardError::Protocol(format!(
                    "missing BatchSigner keypair for inner account {signer_addr}"
                ))
            })?;
        let raw_id = decode_classic_address(signer_addr).map_err(|err| {
            StewardError::Protocol(format!("invalid BatchSigner classic address: {err}"))
        })?;
        let signer_entry = signer.sign_batch_payload(TF_BATCH_ALL_OR_NOTHING, &inner_hashes)?;
        batch_signers_with_keys.push((raw_id.to_vec(), signer_entry));
    }
    batch_signers_with_keys.sort_by(|a, b| a.0.cmp(&b.0));

    let base_fee = client.fee_drops().unwrap_or(10).max(10);
    let total_fee = ((2 + batch_signers_with_keys.len() + inner_hashes.len()) as u64)
        .saturating_mul(base_fee)
        .saturating_mul(2)
        .max(200);
    let (validated_seq, _) = client.validated_close_time()?;
    let last_ledger = u32::try_from(validated_seq.saturating_add(20))
        .map_err(|_| StewardError::Protocol("ledger sequence exceeds u32".into()))?;

    let mut batch_tx = json!({
        "TransactionType": "Batch",
        "Account": outer_signer.address(),
        "Flags": TF_BATCH_ALL_OR_NOTHING,
        "Fee": total_fee.to_string(),
        "LastLedgerSequence": last_ledger,
        "NetworkID": STANDALONE_NETWORK_ID,
        "RawTransactions": raw_transactions
    });
    if let Some(ticket) = outer_ticket {
        batch_tx["Sequence"] = Value::from(0_u32);
        batch_tx["TicketSequence"] = Value::from(ticket);
    } else {
        batch_tx["Sequence"] = Value::from(outer_seq);
    }
    if !batch_signers_with_keys.is_empty() {
        let entries: Vec<Value> = batch_signers_with_keys
            .into_iter()
            .map(|(_, entry)| entry)
            .collect();
        batch_tx["BatchSigners"] = Value::Array(entries);
    }

    let (outer_blob, outer_hash) = outer_signer.sign_tx_json(&mut batch_tx)?;
    let submit_reply = client.call("submit", json!({ "tx_blob": outer_blob }))?;
    let accepted = submit_reply
        .get("accepted")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let preliminary = submit_reply
        .get("engine_result")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    if !accepted {
        return Err(StewardError::StandAlone(StandAloneError::TxFailed {
            tx_type: "Batch".to_string(),
            result: preliminary.to_string(),
        }));
    }
    let _ = client.ledger_accept()?;

    let outer_tx = client.call("tx", json!({ "transaction": outer_hash }))?;
    let outer_validated = outer_tx
        .get("validated")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let outer_result = outer_tx
        .get("meta")
        .and_then(|m| m.get("TransactionResult"))
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let outer_ledger_index = outer_tx
        .get("ledger_index")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if !outer_validated || outer_result != "tesSUCCESS" {
        return Err(StewardError::StandAlone(StandAloneError::TxFailed {
            tx_type: "Batch".to_string(),
            result: outer_result.to_string(),
        }));
    }

    let mut inners = Vec::with_capacity(inner_hashes.len());
    for inner_hash in inner_hashes {
        let inner_tx = match client.call("tx", json!({ "transaction": inner_hash })) {
            Ok(v) => v,
            Err(err) => {
                return Err(StewardError::BatchInnerFailed {
                    outer_hash,
                    inner_hash,
                    detail: format!("inner tx lookup failed: {err}"),
                });
            }
        };
        let validated = inner_tx
            .get("validated")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let parent_id = inner_tx
            .get("meta")
            .and_then(|m| m.get("ParentBatchID"))
            .or_else(|| inner_tx.get("tx_json").and_then(|j| j.get("ParentBatchID")))
            .or_else(|| inner_tx.get("tx").and_then(|j| j.get("ParentBatchID")))
            .or_else(|| inner_tx.get("ParentBatchID"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let inner_result = inner_tx
            .get("meta")
            .and_then(|m| m.get("TransactionResult"))
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let inner_ledger = inner_tx
            .get("ledger_index")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if !validated
            || !parent_id.eq_ignore_ascii_case(&outer_hash)
            || inner_result != "tesSUCCESS"
        {
            return Err(StewardError::BatchInnerFailed {
                outer_hash,
                inner_hash,
                detail: format!("validated={validated} parent={parent_id} result={inner_result}"),
            });
        }
        inners.push(VerifiedBatchInner {
            inner_hash,
            parent_batch_id: parent_id,
            result: inner_result,
            ledger_index: inner_ledger,
        });
    }

    Ok(VerifiedBatchResult {
        outer_hash,
        ledger_index: outer_ledger_index,
        inners,
    })
}

/// Construct an unsigned inner/standalone `Payment` JSON object for an issued currency.
pub fn payment_iou_json(
    sender: &str,
    destination: &str,
    issuer: &str,
    currency: &str,
    amount_milli: Units,
) -> Result<Value, StewardError> {
    let value = format_milli_units(amount_milli)?;
    Ok(json!({
        "TransactionType": "Payment",
        "Account": sender,
        "Destination": destination,
        "Amount": {
            "currency": currency,
            "issuer": issuer,
            "value": value
        }
    }))
}

/// Construct an unsigned inner/standalone `Clawback` JSON object for an issued trust line.
pub fn clawback_iou_json(
    issuer: &str,
    holder: &str,
    currency: &str,
    amount_milli: Units,
) -> Result<Value, StewardError> {
    let value = format_milli_units(amount_milli)?;
    Ok(json!({
        "TransactionType": "Clawback",
        "Account": issuer,
        "Amount": {
            "currency": currency,
            "issuer": holder,
            "value": value
        }
    }))
}

/// Submit a single `Payment` (used when a single transfer leg does not require `Batch`).
pub fn submit_single_payment<T: Transport>(
    client: &StandAlone<T>,
    sender: &StandAloneAccount,
    destination: &str,
    issuer: &str,
    currency: &str,
    amount_milli: Units,
    ticket: Option<u32>,
) -> Result<(), StewardError> {
    let value = format_milli_units(amount_milli)?;
    let mut tx: Payment<'static> = build_tx(
        "Payment",
        sender,
        json!({
            "Destination": destination,
            "Amount": {
                "currency": currency,
                "issuer": issuer,
                "value": value
            }
        }),
    )?;
    if let Some(t) = ticket {
        tx.common_fields.sequence = Some(0);
        tx.common_fields.ticket_sequence = Some(t);
    }
    let _ = client.submit_and_accept_ok(&mut tx, sender, None)?;
    Ok(())
}

/// Submit a single `Clawback` (used when locking a single Phase-1 `PostOffer` / bond).
pub fn submit_single_clawback<T: Transport>(
    client: &StandAlone<T>,
    issuer: &StandAloneAccount,
    holder: &str,
    currency: &str,
    amount_milli: Units,
    ticket: Option<u32>,
) -> Result<(), StewardError> {
    let value = format_milli_units(amount_milli)?;
    let mut tx: Clawback<'static> = build_tx(
        "Clawback",
        issuer,
        json!({
            "Amount": {
                "currency": currency,
                "issuer": holder,
                "value": value
            }
        }),
    )?;
    if let Some(t) = ticket {
        tx.common_fields.sequence = Some(0);
        tx.common_fields.ticket_sequence = Some(t);
    }
    let _ = client.submit_and_accept_ok(&mut tx, issuer, None)?;
    Ok(())
}

/// Configure Authority genesis flags in exact order (`RequireAuth` -> `Clawback` -> `Locking` ->
/// `DefaultRipple` -> `DisallowXRP`) before any trust line exists, and verify on `account_info`.
pub fn configure_steward_authority<T: Transport>(
    client: &StandAlone<T>,
    steward: &StandAloneAccount,
) -> Result<u32, StewardError> {
    for flag in [
        ASF_REQUIRE_AUTH,
        ASF_ALLOW_TRUSTLINE_CLAWBACK,
        ASF_ALLOW_TRUSTLINE_LOCKING,
        ASF_DEFAULT_RIPPLE,
        ASF_DISALLOW_XRP,
    ] {
        let mut tx: AccountSet<'static> =
            build_tx("AccountSet", steward, json!({ "SetFlag": flag }))?;
        let _ = client.submit_and_accept_ok(&mut tx, steward, None)?;
    }

    let info = client.call(
        "account_info",
        json!({
            "account": steward.address(),
            "ledger_index": "validated"
        }),
    )?;
    let Some(acct) = info.get("account_data") else {
        return Err(StewardError::GenesisFlagsMismatch(
            "account_info missing account_data".into(),
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
    Ok(flags)
}

/// Open a `tfSetNoRipple` trust line on `holder` and authorize it (`tfSetfAuth`) from `issuer`.
pub fn open_authorized_trust_line<T: Transport>(
    client: &StandAlone<T>,
    issuer: &StandAloneAccount,
    holder: &StandAloneAccount,
    currency: &str,
    limit: &str,
    issuer_ticket: Option<u32>,
    holder_ticket: Option<u32>,
) -> Result<(), StewardError> {
    let mut trust: TrustSet<'static> = build_tx(
        "TrustSet",
        holder,
        json!({
            "Flags": TF_SET_NO_RIPPLE,
            "LimitAmount": {
                "currency": currency,
                "issuer": issuer.address(),
                "value": limit
            }
        }),
    )?;
    if let Some(t) = holder_ticket {
        trust.common_fields.sequence = Some(0);
        trust.common_fields.ticket_sequence = Some(t);
    }
    let _ = client.submit_and_accept_ok(&mut trust, holder, None)?;

    let mut auth: TrustSet<'static> = build_tx(
        "TrustSet",
        issuer,
        json!({
            "Flags": TF_SET_AUTH,
            "LimitAmount": {
                "currency": currency,
                "issuer": holder.address(),
                "value": "0"
            }
        }),
    )?;
    if let Some(t) = issuer_ticket {
        auth.common_fields.sequence = Some(0);
        auth.common_fields.ticket_sequence = Some(t);
    }
    let _ = client.submit_and_accept_ok(&mut auth, issuer, None)?;
    Ok(())
}

/// Issue and accept an `XLS-70` `Credential` (`CredentialCreate` + `CredentialAccept`).
pub fn issue_and_accept_credential<T: Transport>(
    client: &StandAlone<T>,
    issuer: &StandAloneAccount,
    subject: &StandAloneAccount,
    credential_type_ascii: &str,
    expiration_ripple_secs: Option<u32>,
    uri_hex: Option<&str>,
) -> Result<(), StewardError> {
    let cred_hex = hex::encode_upper(credential_type_ascii.as_bytes());
    let mut create_fields = json!({
        "Subject": subject.address(),
        "CredentialType": cred_hex
    });
    if let Some(exp) = expiration_ripple_secs {
        create_fields["Expiration"] = Value::from(exp);
    }
    if let Some(uri) = uri_hex {
        create_fields["URI"] = Value::String(uri.to_string());
    }
    let mut create_tx: CredentialCreate<'static> =
        build_tx("CredentialCreate", issuer, create_fields)?;
    let _ = client.submit_and_accept_ok(&mut create_tx, issuer, None)?;

    let mut accept_tx: CredentialAccept<'static> = build_tx(
        "CredentialAccept",
        subject,
        json!({
            "Issuer": issuer.address(),
            "CredentialType": cred_hex
        }),
    )?;
    let _ = client.submit_and_accept_ok(&mut accept_tx, subject, None)?;
    Ok(())
}

/// Revoke an `XLS-70` `Credential` via `CredentialDelete`.
pub fn delete_credential<T: Transport>(
    client: &StandAlone<T>,
    issuer: &StandAloneAccount,
    subject_address: &str,
    credential_type_ascii: &str,
) -> Result<(), StewardError> {
    let cred_hex = hex::encode_upper(credential_type_ascii.as_bytes());
    let mut tx: CredentialDelete<'static> = build_tx(
        "CredentialDelete",
        issuer,
        json!({
            "Subject": subject_address,
            "Issuer": issuer.address(),
            "CredentialType": cred_hex
        }),
    )?;
    let _ = client.submit_and_accept_ok(&mut tx, issuer, None)?;
    Ok(())
}

/// Create or update an `XLS-80` `PermissionedDomain` and return its 64-hex `DomainID`.
pub fn set_permissioned_domain<T: Transport>(
    client: &StandAlone<T>,
    owner: &StandAloneAccount,
    existing_domain_id: Option<&str>,
    accepted_credential_types_ascii: &[&str],
) -> Result<String, StewardError> {
    let credentials: Vec<Credential> = accepted_credential_types_ascii
        .iter()
        .map(|ty| {
            Credential::new(
                owner.address().to_string(),
                hex::encode_upper(ty.as_bytes()),
            )
        })
        .collect();
    let mut tx = PermissionedDomainSet::new(
        owner.address().to_string().into(),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        existing_domain_id.map(|s| s.to_string().into()),
        credentials,
    );
    let validated = client.submit_and_accept_ok(&mut tx, owner, None)?;
    if let Some(id) = existing_domain_id {
        return Ok(id.to_string());
    }
    extract_created_ledger_index(&validated.meta, "PermissionedDomain")
}

/// Authorize a Standing Regent via `DelegateSet` (`XLS-75`, `PermissionDelegationV1_1`).
pub fn submit_delegate_set<T: Transport>(
    client: &StandAlone<T>,
    seat: &StandAloneAccount,
    regent: &StandAloneAccount,
    permissions: &[&str],
) -> Result<Value, StewardError> {
    let perms: Vec<Value> = permissions
        .iter()
        .map(|p| json!({ "Permission": { "PermissionValue": p } }))
        .collect();
    let tx_json = json!({
        "TransactionType": "DelegateSet",
        "Account": seat.address(),
        "Authorize": regent.address(),
        "Permissions": perms
    });
    let (_hash, reply) = submit_raw_json_tx(client, seat, tx_json, None, None)?;
    Ok(reply)
}

/// Submit a delegated transaction on behalf of `seat` signed strictly by `regent`.
pub fn submit_delegated_tx<T: Transport>(
    client: &StandAlone<T>,
    seat_address: &str,
    regent: &StandAloneAccount,
    mut tx_json: Value,
    fee_drops: Option<&str>,
) -> Result<(String, Value), StewardError> {
    let obj = tx_json
        .as_object_mut()
        .ok_or_else(|| StewardError::Protocol("delegated tx must be JSON object".into()))?;
    obj.insert(
        "Account".to_string(),
        Value::String(seat_address.to_string()),
    );
    obj.insert(
        "Delegate".to_string(),
        Value::String(regent.address().to_string()),
    );
    submit_raw_json_tx(client, regent, tx_json, fee_drops, None)
}

/// Submit an `EscrowCreate` for a `CRED` IOU or 1-drop XRP commit lock and return `(sequence, tx_hash, meta)`.
pub fn submit_escrow_create<T: Transport>(
    client: &StandAlone<T>,
    owner: &StandAloneAccount,
    mut fields: Value,
    ticket: Option<u32>,
) -> Result<(u32, String, Value), StewardError> {
    let obj = fields
        .as_object_mut()
        .ok_or_else(|| StewardError::Protocol("EscrowCreate fields must be JSON object".into()))?;
    obj.insert(
        "TransactionType".to_string(),
        Value::String("EscrowCreate".to_string()),
    );
    obj.insert(
        "Account".to_string(),
        Value::String(owner.address().to_string()),
    );
    let seq = if ticket.is_none() {
        let s = client.account_sequence(owner.address())?;
        obj.insert("Sequence".to_string(), Value::from(s));
        s
    } else {
        0
    };
    let (tx_hash, reply) = submit_raw_json_tx(client, owner, fields, None, ticket)?;
    let validated = reply
        .get("validated")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let result = reply
        .get("meta")
        .and_then(|m| m.get("TransactionResult"))
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    if !validated || result != "tesSUCCESS" {
        return Err(StewardError::StandAlone(StandAloneError::TxFailed {
            tx_type: "EscrowCreate".to_string(),
            result: result.to_string(),
        }));
    }
    let meta = reply.get("meta").cloned().unwrap_or(Value::Null);
    Ok((ticket.unwrap_or(seq), tx_hash, meta))
}

/// Submit an `EscrowFinish` with preimage condition/fulfillment and explicit drop fee floor.
pub fn submit_escrow_finish<T: Transport>(
    client: &StandAlone<T>,
    signer: &StandAloneAccount,
    fields: Value,
    fee_drops: &str,
    ticket: Option<u32>,
) -> Result<String, StewardError> {
    let mut tx: EscrowFinish<'static> = build_tx("EscrowFinish", signer, fields)?;
    if let Some(t) = ticket {
        tx.common_fields.sequence = Some(0);
        tx.common_fields.ticket_sequence = Some(t);
    }
    let receipt = client.submit_and_accept(&mut tx, signer, Some(fee_drops))?;
    Ok(receipt.final_result)
}

/// Submit an `EscrowCancel` and return its validated `TransactionResult`.
pub fn submit_escrow_cancel<T: Transport>(
    client: &StandAlone<T>,
    signer: &StandAloneAccount,
    owner_address: &str,
    offer_sequence: u32,
    ticket_sequence: Option<u32>,
) -> Result<String, StewardError> {
    let fields = json!({
        "Owner": owner_address,
        "OfferSequence": offer_sequence
    });
    let mut tx: EscrowCancel<'static> = build_tx("EscrowCancel", signer, fields)?;
    if let Some(t) = ticket_sequence {
        tx.common_fields.sequence = Some(0);
        tx.common_fields.ticket_sequence = Some(t);
    }
    let receipt = client.submit_and_accept(&mut tx, signer, None)?;
    Ok(receipt.final_result)
}

/// Submit a `CheckCreate` for an issued currency and return the created 64-hex `CheckID`.
pub fn submit_check_create<T: Transport>(
    client: &StandAlone<T>,
    drawer: &StandAloneAccount,
    destination: &str,
    issuer: &str,
    currency: &str,
    send_max_milli: Units,
    expiration_ripple: Option<u32>,
) -> Result<String, StewardError> {
    let val = format_milli_units(send_max_milli)?;
    let mut fields = json!({
        "Destination": destination,
        "SendMax": {
            "currency": currency,
            "issuer": issuer,
            "value": val
        }
    });
    if let Some(exp) = expiration_ripple {
        fields["Expiration"] = Value::from(exp);
    }
    let mut tx: CheckCreate<'static> = build_tx("CheckCreate", drawer, fields)?;
    let receipt = client.submit_and_accept_ok(&mut tx, drawer, None)?;
    extract_created_ledger_index(&receipt.meta, "Check")
}

/// Submit a `CheckCash` (`Amount`) and return its validated `TransactionResult` (`tesSUCCESS` or
/// bounce code such as `tecPATH_PARTIAL`/`tecEXPIRED`/`tecUNFUNDED_PAYMENT`).
pub fn submit_check_cash<T: Transport>(
    client: &StandAlone<T>,
    payee: &StandAloneAccount,
    check_id: &str,
    issuer: &str,
    currency: &str,
    amount_milli: Units,
) -> Result<String, StewardError> {
    let val = format_milli_units(amount_milli)?;
    let mut tx: CheckCash<'static> = build_tx(
        "CheckCash",
        payee,
        json!({
            "CheckID": check_id,
            "Amount": {
                "currency": currency,
                "issuer": issuer,
                "value": val
            }
        }),
    )?;
    let receipt = client.submit_and_accept(&mut tx, payee, None)?;
    Ok(receipt.final_result)
}

/// Submit a `CheckCancel` for an open check.
pub fn submit_check_cancel<T: Transport>(
    client: &StandAlone<T>,
    signer: &StandAloneAccount,
    check_id: &str,
) -> Result<String, StewardError> {
    let mut tx: CheckCancel<'static> =
        build_tx("CheckCancel", signer, json!({ "CheckID": check_id }))?;
    let receipt = client.submit_and_accept(&mut tx, signer, None)?;
    Ok(receipt.final_result)
}

/// Publish one or more `OracleSet` (`XLS-47`) documents (splitting into `<= 10` entries per tx).
pub fn publish_oracle_prices<T: Transport>(
    client: &StandAlone<T>,
    steward: &StandAloneAccount,
    quote_currency: &str,
    last_update_time: u32,
    uri_hex: &str,
    prices_milli: &[(String, Units)],
) -> Result<Vec<String>, StewardError> {
    let provider_hex = hex::encode_upper(b"merchant-league");
    let asset_class_hex = hex::encode_upper(b"commodity");
    let mut tx_hashes = Vec::new();

    for (chunk_idx, chunk) in prices_milli.chunks(10).enumerate() {
        let doc_id = u32::try_from(chunk_idx + 1)
            .map_err(|_| StewardError::Protocol("OracleDocumentID overflow".into()))?;
        let series: Vec<Value> = chunk
            .iter()
            .map(|(base, price)| {
                let abs_price = (*price).max(1) as u64;
                json!({
                    "PriceData": {
                        "BaseAsset": base,
                        "QuoteAsset": quote_currency,
                        "AssetPrice": format!("{abs_price:X}"),
                        "Scale": 3_u8
                    }
                })
            })
            .collect();
        let tx_json = json!({
            "TransactionType": "OracleSet",
            "Account": steward.address(),
            "OracleDocumentID": doc_id,
            "Provider": provider_hex,
            "URI": uri_hex,
            "AssetClass": asset_class_hex,
            "LastUpdateTime": last_update_time,
            "PriceDataSeries": series
        });
        let (tx_hash, reply) = submit_raw_json_tx(client, steward, tx_json, None, None)?;
        let validated = reply
            .get("validated")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let result = reply
            .get("meta")
            .and_then(|m| m.get("TransactionResult"))
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        if !validated || result != "tesSUCCESS" {
            return Err(StewardError::StandAlone(StandAloneError::TxFailed {
                tx_type: "OracleSet".to_string(),
                result: result.to_string(),
            }));
        }
        tx_hashes.push(tx_hash);
    }
    Ok(tx_hashes)
}

/// Extract the `LedgerIndex` of a newly created ledger object of `entry_type` from `meta.AffectedNodes`.
pub fn extract_created_ledger_index(
    meta: &Value,
    entry_type: &str,
) -> Result<String, StewardError> {
    let nodes = meta
        .get("AffectedNodes")
        .and_then(Value::as_array)
        .ok_or_else(|| StewardError::Protocol("tx meta missing AffectedNodes".into()))?;
    for node in nodes {
        if let Some(created) = node.get("CreatedNode")
            && created.get("LedgerEntryType").and_then(Value::as_str) == Some(entry_type)
            && let Some(idx) = created.get("LedgerIndex").and_then(Value::as_str)
        {
            return Ok(idx.to_string());
        }
    }
    Err(StewardError::Protocol(format!(
        "CreatedNode {entry_type} not found in AffectedNodes"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use league_standalone::STANDALONE_NETWORK_ID;
    use std::cell::RefCell;

    struct FnTransport<F>(RefCell<F>);
    impl<F: FnMut(&str, &Value) -> Result<Value, String>> Transport for FnTransport<F> {
        fn post(&self, _url: &str, body: &Value) -> Result<Value, String> {
            let method = body
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let params = body
                .get("params")
                .and_then(|p| p.get(0))
                .cloned()
                .unwrap_or_else(|| json!({}));
            (self.0.borrow_mut())(method, &params)
        }
    }

    #[test]
    fn split_batch_chunks_never_emits_singleton_chunk() {
        for len in 2..=30_usize {
            let items: Vec<usize> = (0..len).collect();
            let chunks = split_batch_chunks(&items);
            let flattened: Vec<usize> = chunks.iter().flatten().copied().collect();
            assert_eq!(flattened, items, "len {len}");
            for chunk in &chunks {
                assert!(
                    (2..=MAX_BATCH_INNER_TXS).contains(&chunk.len()),
                    "len {len} produced invalid chunk size {}",
                    chunk.len()
                );
            }
        }
    }

    #[test]
    fn codec_error_paths_multi_party_batch_and_authority_checks() {
        assert!(extract_created_ledger_index(&json!({}), "Check").is_err());
        assert!(extract_created_ledger_index(&json!({ "AffectedNodes": [] }), "Check").is_err());
        assert!(payment_iou_json("rA", "rB", "rS", "CRED", -1).is_err());
        assert!(clawback_iou_json("rS", "rA", "CRED", -1).is_err());

        let mut seq = 10_u32;
        let t_mode = std::cell::Cell::new(0_u8);
        let t = FnTransport(RefCell::new(|method: &str, _params: &Value| match method {
            "server_info" => Ok(json!({
                "result": {
                    "status": "success",
                    "info": {
                        "network_id": STANDALONE_NETWORK_ID,
                        "validation_quorum": 0,
                        "peers": 0,
                        "server_state": "full"
                    }
                }
            })),
            "ledger" => Ok(json!({
                "result": { "status": "success", "ledger": { "ledger_index": 5, "close_time": 100 } }
            })),
            "ledger_accept" => {
                Ok(json!({ "result": { "status": "success", "ledger_current_index": 6 } }))
            }
            "fee" => {
                Ok(json!({ "result": { "status": "success", "drops": { "base_fee": "10" } } }))
            }
            "account_info" => {
                seq += 1;
                let flags = if t_mode.get() == 1 {
                    0_u32
                } else {
                    LSF_REQUIRE_AUTH
                        | LSF_ALLOW_TRUSTLINE_CLAWBACK
                        | LSF_ALLOW_TRUSTLINE_LOCKING
                        | LSF_DEFAULT_RIPPLE
                        | LSF_DISALLOW_XRP
                };
                let mut data = json!({ "Sequence": seq, "Flags": flags, "Balance": "100000" });
                if t_mode.get() == 2 {
                    data["TransferRate"] = json!(1_001_000_000_u64);
                }
                Ok(json!({ "result": { "status": "success", "account_data": data } }))
            }
            "account_objects" => Ok(json!({
                "result": { "status": "success", "account_objects": [{ "TicketSequence": 77 }] }
            })),
            "submit" => {
                if t_mode.get() == 3 {
                    Ok(
                        json!({ "result": { "status": "success", "accepted": false, "engine_result": "temBAD" } }),
                    )
                } else {
                    Ok(
                        json!({ "result": { "status": "success", "accepted": true, "engine_result": "tesSUCCESS" } }),
                    )
                }
            }
            "tx" => {
                if t_mode.get() == 4 {
                    Ok(json!({ "result": { "status": "error", "error": "txnNotFound" } }))
                } else {
                    Ok(json!({
                        "result": {
                            "status": "success",
                            "validated": true,
                            "ledger_index": 5,
                            "meta": {
                                "TransactionResult": "tesSUCCESS",
                                "ParentBatchID": "00".repeat(32)
                            },
                            "ParentBatchID": "ANY"
                        }
                    }))
                }
            }
            _ => Err(format!("unexpected {method}")),
        }));
        let client = StandAlone::with_transport("http://127.0.0.1:5005", t)
            .unwrap_or_else(|e| panic!("client: {e}"));
        let acct_a = StandAloneAccount::generate_ed25519().unwrap_or_else(|e| panic!("gen a: {e}"));
        let acct_b = StandAloneAccount::generate_ed25519().unwrap_or_else(|e| panic!("gen b: {e}"));

        let mut pool = TicketPool::default();
        assert!(pool.allocate(&client, &acct_a, 0).is_err());
        assert!(pool.allocate(&client, &acct_a, 251).is_err());
        assert!(pool.allocate(&client, &acct_a, 1).is_ok());
        assert_eq!(pool.remaining(), 1);
        assert!(pool.ensure_headroom(&client, &acct_a, 2, 1).is_ok());

        t_mode.set(1);
        assert!(matches!(
            configure_steward_authority(&client, &acct_a),
            Err(StewardError::GenesisFlagsMismatch(_))
        ));
        t_mode.set(2);
        assert!(matches!(
            configure_steward_authority(&client, &acct_a),
            Err(StewardError::GenesisFlagsMismatch(_))
        ));
        t_mode.set(0);

        let p_a = payment_iou_json(
            acct_a.address(),
            acct_b.address(),
            acct_a.address(),
            "R00",
            1_000,
        )
        .unwrap_or_else(|e| panic!("p_a: {e}"));
        let p_b = payment_iou_json(
            acct_b.address(),
            acct_a.address(),
            acct_a.address(),
            "R00",
            1_000,
        )
        .unwrap_or_else(|e| panic!("p_b: {e}"));
        assert!(
            submit_batch_all_or_nothing(&client, &acct_a, vec![p_a.clone()], &[], None).is_err()
        );
        assert!(
            submit_batch_all_or_nothing(
                &client,
                &acct_a,
                vec![p_a.clone(), p_b.clone()],
                &[],
                Some(55)
            )
            .is_err()
        );
        // Valid 2-party BatchV1_1 outer + BatchSigners with outer_ticket fails only at ParentBatchID check below,
        // exercising both BatchSigners sorting/signing and ParentBatchID mismatch branch.
        let batch_res =
            submit_batch_all_or_nothing(&client, &acct_a, vec![p_a, p_b], &[&acct_b], Some(55));
        assert!(matches!(
            batch_res,
            Err(StewardError::BatchInnerFailed { .. })
        ));

        t_mode.set(3);
        let rej = submit_raw_json_tx(
            &client,
            &acct_a,
            json!({ "TransactionType": "AccountSet", "Account": acct_a.address() }),
            None,
            None,
        )
        .unwrap_or_else(|e| panic!("rej: {e}"));
        assert_eq!(rej.1.get("validated").and_then(Value::as_bool), Some(false));
        assert!(
            submit_batch_all_or_nothing(
                &client,
                &acct_a,
                vec![
                    payment_iou_json(
                        acct_a.address(),
                        acct_b.address(),
                        acct_a.address(),
                        "R00",
                        500
                    )
                    .unwrap_or_else(|e| panic!("{e}")),
                    payment_iou_json(
                        acct_a.address(),
                        acct_b.address(),
                        acct_a.address(),
                        "R00",
                        500
                    )
                    .unwrap_or_else(|e| panic!("{e}")),
                ],
                &[],
                None,
            )
            .is_err()
        );
        t_mode.set(4);
        let nf = submit_raw_json_tx(
            &client,
            &acct_a,
            json!({ "TransactionType": "AccountSet", "Account": acct_a.address() }),
            None,
            Some(77),
        )
        .unwrap_or_else(|e| panic!("nf: {e}"));
        assert_eq!(nf.1.get("validated").and_then(Value::as_bool), Some(false));
    }
}
