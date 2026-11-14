//! The serializer, the hash and the key derivation, checked against twelve
//! transactions signed and validated on the live XRPL Testnet on 2026-11-14
//! (the M2 xrpl-rust spike, merchant-league PR #2). The ledger's own binary
//! form and hash are the reference: our encoding must match them exactly.
//!
//! `tests/vectors/testnet-spike-2026-11-14.json` holds, for each
//! transaction, the `tx_json` and the binary `tx` the Testnet returned, and
//! the hash it reported. These are our own captures; the Testnet is wiped on
//! 2026-11-25, after which they can no longer be fetched.

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use league_ledger::codec::{Value, decode_hex, encode_hex, from_json, transaction_hash};
use league_ledger::keys::{account_id_from_public, encode_address};
use serde_json::Value as Json;

type Res = Result<(), Box<dyn std::error::Error>>;

fn vectors() -> Result<Vec<Json>, Box<dyn std::error::Error>> {
    let text = include_str!("vectors/testnet-spike-2026-11-14.json");
    let doc: Json = serde_json::from_str(text)?;
    let list = doc["transactions"]
        .as_array()
        .ok_or("no transactions array")?
        .clone();
    Ok(list)
}

#[test]
fn there_are_twelve_transactions_covering_all_ten_types() -> Res {
    let list = vectors()?;
    assert_eq!(list.len(), 12);
    let mut types: Vec<&str> = list
        .iter()
        .filter_map(|t| t["tx_json"]["TransactionType"].as_str())
        .collect();
    types.sort_unstable();
    types.dedup();
    assert_eq!(
        types,
        [
            "AccountSet",
            "CheckCash",
            "CheckCreate",
            "Clawback",
            "EscrowCreate",
            "EscrowFinish",
            "OfferCancel",
            "OfferCreate",
            "Payment",
            "TrustSet"
        ]
    );
    Ok(())
}

#[test]
fn every_transaction_serializes_to_the_ledgers_own_bytes() -> Res {
    for t in vectors()? {
        let tx = from_json(&t["tx_json"])?;
        let ours = encode_hex(&tx.serialize()?);
        let ledger = t["tx_blob"].as_str().ok_or("no tx_blob")?;
        assert_eq!(
            ours,
            ledger.to_ascii_uppercase(),
            "{} at ledger {}",
            t["tx_json"]["TransactionType"],
            t["ledger_index"]
        );
    }
    Ok(())
}

#[test]
fn every_hash_matches_the_one_the_testnet_reported() -> Res {
    for t in vectors()? {
        let tx = from_json(&t["tx_json"])?;
        let hash = encode_hex(&transaction_hash(&tx.serialize()?));
        assert_eq!(
            hash,
            t["hash"].as_str().ok_or("no hash")?,
            "{}",
            t["tx_json"]["TransactionType"]
        );
    }
    Ok(())
}

#[test]
fn the_failed_reveal_has_its_hash_too() -> Res {
    // The tec result that xrpl-rust returned with no hash: ours computes it.
    let list = vectors()?;
    let failed = list
        .iter()
        .find(|t| t["result"] == "tecCRYPTOCONDITION_ERROR")
        .ok_or("no failed reveal")?;
    let tx = from_json(&failed["tx_json"])?;
    assert_eq!(
        encode_hex(&transaction_hash(&tx.serialize()?)),
        "ECD872CA38D44F682A0C228977C165C0A871BDCFAE62A1F749A1311CFA72414A"
    );
    Ok(())
}

#[test]
fn every_signing_key_derives_to_its_account() -> Res {
    for t in vectors()? {
        let raw = decode_hex(t["tx_json"]["SigningPubKey"].as_str().ok_or("no key")?)?;
        let key: [u8; 33] = raw.try_into().map_err(|_| "not 33 bytes")?;
        assert_eq!(key[0], 0xED, "ed25519 only");
        let address = encode_address(&account_id_from_public(&key));
        assert_eq!(
            address,
            t["tx_json"]["Account"].as_str().ok_or("no account")?
        );
    }
    Ok(())
}

#[test]
fn every_signature_verifies_over_our_signing_data() -> Res {
    for t in vectors()? {
        let tx = from_json(&t["tx_json"])?;
        let message = tx.signing_data()?;
        let key_bytes = decode_hex(t["tx_json"]["SigningPubKey"].as_str().ok_or("no key")?)?;
        let mut public = [0u8; 32];
        public.copy_from_slice(&key_bytes[1..]);
        let sig_bytes = match tx.get("TxnSignature") {
            Some(Value::Blob(b)) => b.clone(),
            _ => return Err("no signature".into()),
        };
        let sig: [u8; 64] = sig_bytes.try_into().map_err(|_| "not 64 bytes")?;
        VerifyingKey::from_bytes(&public)?.verify(&message, &Signature::from_bytes(&sig))?;
    }
    Ok(())
}
