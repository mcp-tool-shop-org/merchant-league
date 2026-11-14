//! The league's encoding and keys, checked against published vectors.
//!
//! - `testnet-spike-2026-11-14.json`: twelve transactions the league signed
//!   and the Testnet validated on 2026-11-14, as the server returned them
//!   (`tx_json`, `tx_blob`, `hash`). The binary must re-encode exactly, and
//!   the hash must be the one the ledger holds.
//! - `xrpljs-codec-fixtures.json` and `xrpljs-keypairs-api.json`: xrpl.js's
//!   own test fixtures (ISC; see `vectors/README.md` and
//!   `vectors/LICENSE-xrpl.js`).

use league_ledger::client::{Account, decode_hex, transaction_hash};
use serde_json::Value;
use xrpl::core::binarycodec::encode;

fn load(name: &str) -> Value {
    let path = format!("{}/tests/vectors/{name}", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

#[test]
fn every_validated_testnet_transaction_re_encodes_to_its_blob_and_hash() {
    let spike = load("testnet-spike-2026-11-14.json");
    assert_eq!(spike["network_id"], 1);
    let txs = spike["transactions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(txs.len(), 12);
    for tx in txs {
        let kind = tx["tx_json"]["TransactionType"].clone();
        let blob = encode(&tx["tx_json"]).unwrap_or_default();
        assert_eq!(Value::from(blob.clone()), tx["tx_blob"], "{kind}");
        let hash = transaction_hash(&decode_hex(&blob).unwrap_or_default());
        assert_eq!(Value::from(hash), tx["hash"], "{kind}");
    }
}

#[test]
fn every_xrpljs_codec_fixture_encodes_to_its_published_binary() {
    let fixtures = load("xrpljs-codec-fixtures.json");
    let txs = fixtures["transactions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(txs.len(), 39);
    for tx in txs {
        let kind = tx["json"]["TransactionType"].clone();
        let blob = encode(&tx["json"]).unwrap_or_default();
        assert_eq!(Value::from(blob), tx["binary"], "{kind}");
    }
}

/// No published vector carries a DomainID, so its encoding is checked by
/// hand: Hash256 is type 5 and DomainID field 34, so the field header is
/// `50 22` (type in the high nibble, field number in the next byte), then
/// the 32 bytes. The live proof checks the ledger accepts it.
#[test]
fn a_domain_id_encodes_as_hash256_field_34() {
    let domain = "C6B8F21B5D6B0A8E8C2B1F4E3D2A1908172635445362718293A4B5C6D7E8F901";
    let tx = serde_json::json!({
        "TransactionType": "OfferCreate",
        "Account": "rLUEXYuLiQptky37CqLcm9USQpPiz5rkpD",
        "TakerGets": "1000000",
        "TakerPays": "2000000",
        "DomainID": domain,
    });
    let blob = encode(&tx).unwrap_or_default();
    assert!(blob.contains(&format!("5022{domain}")), "{blob}");
}

#[test]
fn the_xrpljs_ed25519_seed_derives_its_published_key_and_address() {
    let api = load("xrpljs-keypairs-api.json");
    let ed = &api["ed25519"];
    let account = Account::from_seed(ed["seed"].as_str().unwrap_or_default());
    let account = account.ok();
    assert_eq!(
        account.as_ref().map(|a| a.address().to_string()),
        ed["address"].as_str().map(str::to_string)
    );
    assert_eq!(
        account.as_ref().map(|a| a.public_key().to_string()),
        ed["keypair"]["publicKey"].as_str().map(str::to_string)
    );
    // The published secp256k1 seed is refused: the game uses ed25519 only.
    assert!(Account::from_seed(api["secp256k1"]["seed"].as_str().unwrap_or_default()).is_err());
}

/// The TokenEscrow and DomainID transactions the Testnet validated in the
/// M2c proof: each re-encodes to its validated blob and hash, and parses
/// into the patched model with the new field intact.
#[test]
fn the_validated_token_escrow_and_domain_transactions_round_trip() {
    use xrpl::models::Amount;
    use xrpl::models::transactions::escrow_create::EscrowCreate;
    use xrpl::models::transactions::offer_create::OfferCreate;
    use xrpl::models::transactions::payment::Payment;

    let proof = load("testnet-m2c-2026-11-14.json");
    assert_eq!(proof["network_id"], 1);
    let txs = proof["transactions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(txs.len(), 9);
    let mut token_escrows = 0;
    let mut with_domain = 0;
    for tx in txs {
        let mut json = tx["tx_json"].clone();
        // API v2 returns a Payment's Amount as DeliverMax; the signed field is Amount.
        if let Some(deliver_max) = json.as_object_mut().and_then(|o| o.remove("DeliverMax")) {
            json["Amount"] = deliver_max;
        }
        let kind = json["TransactionType"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let blob = encode(&json).unwrap_or_default();
        assert_eq!(Value::from(blob.clone()), tx["tx_blob"], "{kind}");
        let hash = transaction_hash(&decode_hex(&blob).unwrap_or_default());
        assert_eq!(Value::from(hash), tx["hash"], "{kind}");
        match kind.as_str() {
            "EscrowCreate" => {
                let e: Option<EscrowCreate> = serde_json::from_value(json).ok();
                assert!(matches!(
                    e.map(|e| e.amount),
                    Some(Amount::IssuedCurrencyAmount(_))
                ));
                token_escrows += 1;
            }
            "OfferCreate" => {
                let o: Option<OfferCreate> = serde_json::from_value(json.clone()).ok();
                assert_eq!(
                    o.and_then(|o| o.domain_id.map(|d| d.to_string())),
                    json["DomainID"].as_str().map(str::to_string)
                );
                with_domain += 1;
            }
            "Payment" => {
                let p: Option<Payment> = serde_json::from_value(json.clone()).ok();
                assert_eq!(
                    p.and_then(|p| p.domain_id.map(|d| d.to_string())),
                    json["DomainID"].as_str().map(str::to_string)
                );
                with_domain += 1;
            }
            _ => {}
        }
    }
    assert_eq!((token_escrows, with_domain), (2, 4));
}
