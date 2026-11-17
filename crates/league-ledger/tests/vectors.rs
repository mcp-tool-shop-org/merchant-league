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
use league_ledger::{
    ESCROW_FINISH_DEFAULT_FEE_DROPS, condition_hex, escrow_condition, escrow_finish_min_fee_drops,
    fulfillment_hex,
};
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

/// Every validated Testnet transaction (M2b's 12 and M2c's 9), parsed into
/// xrpl-rust's typed model for its kind and encoded from that model,
/// reproduces the validated blob and hash. That proves the typed models carry
/// every signed field, not only the raw JSON. (Carbon's review of #5, item 5.)
/// Parsed from text, so a borrowed field like `Credential` deserialises.
#[test]
fn every_validated_transaction_round_trips_through_its_typed_model() {
    use xrpl::models::transactions::{
        account_set::AccountSet, check_cash::CheckCash, check_create::CheckCreate,
        clawback::Clawback, escrow_create::EscrowCreate, escrow_finish::EscrowFinish,
        offer_cancel::OfferCancel, offer_create::OfferCreate, payment::Payment,
        permissioned_domain_set::PermissionedDomainSet, trust_set::TrustSet,
    };

    fn typed<'a, T: serde::Deserialize<'a> + serde::Serialize + xrpl::models::Model>(
        text: &'a str,
    ) -> String {
        let model: T = serde_json::from_str(text).unwrap_or_else(|e| panic!("{e}: {text}"));
        // Each was a validated transaction, so the model's own checks pass too.
        model
            .validate()
            .unwrap_or_else(|e| panic!("validate: {e}: {text}"));
        encode(&model).unwrap_or_default()
    }

    let mut seen = 0;
    for file in [
        "testnet-spike-2026-11-14.json",
        "testnet-m2c-2026-11-14.json",
    ] {
        let txs = load(file)["transactions"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for tx in txs {
            let mut json = tx["tx_json"].clone();
            // API v2 returns a Payment's Amount as DeliverMax.
            if let Some(deliver_max) = json.as_object_mut().and_then(|o| o.remove("DeliverMax")) {
                json["Amount"] = deliver_max;
            }
            let text = json.to_string();
            let kind = json["TransactionType"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            let blob = match kind.as_str() {
                "AccountSet" => typed::<AccountSet>(&text),
                "TrustSet" => typed::<TrustSet>(&text),
                "Payment" => typed::<Payment>(&text),
                "OfferCreate" => typed::<OfferCreate>(&text),
                "OfferCancel" => typed::<OfferCancel>(&text),
                "CheckCreate" => typed::<CheckCreate>(&text),
                "CheckCash" => typed::<CheckCash>(&text),
                "EscrowCreate" => typed::<EscrowCreate>(&text),
                "EscrowFinish" => typed::<EscrowFinish>(&text),
                "Clawback" => typed::<Clawback>(&text),
                "PermissionedDomainSet" => typed::<PermissionedDomainSet>(&text),
                other => panic!("no typed model mapped for {other}"),
            };
            assert_eq!(Value::from(blob.clone()), tx["tx_blob"], "{file}: {kind}");
            let hash = transaction_hash(&decode_hex(&blob).unwrap_or_default());
            assert_eq!(Value::from(hash), tx["hash"], "{file}: {kind}");
            seen += 1;
        }
    }
    assert_eq!(seen, 21);
}

fn decode_fulfillment_secret(fulfillment: &str) -> [u8; 32] {
    let Some(preimage_hex) = fulfillment.strip_prefix("A0228020") else {
        panic!("expected PREIMAGE-SHA-256 fulfillment prefix A0228020, got {fulfillment}");
    };
    let bytes = decode_hex(preimage_hex).unwrap_or_else(|e| panic!("invalid hex: {e}"));
    bytes
        .try_into()
        .unwrap_or_else(|v: Vec<u8>| panic!("expected 32-byte secret, got {}", v.len()))
}

#[test]
fn escrow_condition_matches_published_and_validated_testnet_vectors() {
    // Published zero-secret vector (examples/testnet_proof.rs:629 / pre-retirement shape).
    let (zero_cond, zero_fulfil) = escrow_condition(&[0u8; 32]);
    assert_eq!(
        zero_cond,
        "A025802066687AADF862BD776C8FC18B8E9F8E20089714856EE233B3902A591D0D5F2925810120"
    );
    assert_eq!(
        zero_fulfil,
        "A02280200000000000000000000000000000000000000000000000000000000000000000"
    );

    // Validated Testnet spike vectors (testnet-spike-2026-11-14.json).
    let spike = load("testnet-spike-2026-11-14.json");
    let spike_txs = spike["transactions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut spike_finishes = 0;
    let mut spike_creates = 0;
    for tx in &spike_txs {
        let tx_json = &tx["tx_json"];
        match tx_json["TransactionType"].as_str() {
            Some("EscrowCreate") => {
                assert_eq!(
                    tx_json["Condition"].as_str(),
                    Some(
                        "A0258020D9439BB41E98851DA594B3D3979CD16D89EE184EE30AB25196EA14116044B8D2810120"
                    )
                );
                spike_creates += 1;
            }
            Some("EscrowFinish") => {
                let cond = tx_json["Condition"].as_str().unwrap_or_default();
                let fulfil = tx_json["Fulfillment"].as_str().unwrap_or_default();
                let fee_str = tx_json["Fee"].as_str().unwrap_or_default();
                assert_eq!(fee_str, ESCROW_FINISH_DEFAULT_FEE_DROPS);
                let fee: u64 = fee_str
                    .parse()
                    .unwrap_or_else(|e| panic!("fee parse failed: {e}"));
                assert!(fee >= escrow_finish_min_fee_drops(10));
                let secret = decode_fulfillment_secret(fulfil);
                match tx["result"].as_str() {
                    Some("tesSUCCESS") => {
                        assert_eq!(
                            escrow_condition(&secret),
                            (cond.to_owned(), fulfil.to_owned())
                        );
                    }
                    Some("tecCRYPTOCONDITION_ERROR") => {
                        assert_eq!(fulfillment_hex(&secret), fulfil);
                        assert_ne!(condition_hex(&secret), cond);
                    }
                    other => panic!("unexpected EscrowFinish result {other:?}"),
                }
                spike_finishes += 1;
            }
            _ => {}
        }
    }
    assert_eq!((spike_creates, spike_finishes), (1, 2));

    // Validated TokenEscrow vectors (testnet-m2c-2026-11-14.json).
    let m2c = load("testnet-m2c-2026-11-14.json");
    let m2c_txs = m2c["transactions"].as_array().cloned().unwrap_or_default();
    let mut m2c_escrows = 0;
    let mut m2c_finishes = 0;
    for tx in &m2c_txs {
        let tx_json = &tx["tx_json"];
        match tx_json["TransactionType"].as_str() {
            Some("EscrowCreate") => {
                assert_eq!(
                    tx_json["Condition"].as_str(),
                    Some(
                        "A02580201739FA62E9BB5608151BDB9EC353BD12D1C902BC910CBD599EFF23467B01F7B9810120"
                    )
                );
                m2c_escrows += 1;
            }
            Some("EscrowFinish") => {
                let cond = tx_json["Condition"].as_str().unwrap_or_default();
                let fulfil = tx_json["Fulfillment"].as_str().unwrap_or_default();
                let fee_str = tx_json["Fee"].as_str().unwrap_or_default();
                assert_eq!(fee_str, ESCROW_FINISH_DEFAULT_FEE_DROPS);
                let fee: u64 = fee_str
                    .parse()
                    .unwrap_or_else(|e| panic!("fee parse failed: {e}"));
                assert!(fee >= escrow_finish_min_fee_drops(12));
                let secret = decode_fulfillment_secret(fulfil);
                assert_eq!(
                    escrow_condition(&secret),
                    (cond.to_owned(), fulfil.to_owned())
                );
                m2c_finishes += 1;
            }
            _ => {}
        }
    }
    assert_eq!((m2c_escrows, m2c_finishes), (2, 1));
}
