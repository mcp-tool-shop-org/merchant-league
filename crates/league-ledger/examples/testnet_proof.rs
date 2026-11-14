//! M2b proof: the league's own XRPL layer does every transaction the game
//! needs, on the live XRPL Testnet. The same steps as the M2 xrpl-rust
//! spike (docs/receipts/m2-xrpl-rust-spike-2026-11-14), on our serializer,
//! our signing and our JSON-RPC client.
//!
//! Run: `cargo run -p league-ledger --example testnet_proof`
//!
//! Two faucet accounts (one call each). Every step prints one JSON line with
//! the hash we computed before submitting and the validated result. Seeds
//! are never printed.

use league_dice::{Secret, condition_hex, finish_fee_drops, fulfillment_hex};
use league_ledger::client::{Funded, Receipt, Testnet};
use league_ledger::codec::{Amount, Currency, Transaction, Value, decode_hex};
use serde_json::json;
use std::error::Error;
use std::time::{SystemTime, UNIX_EPOCH};

type Res<T> = Result<T, Box<dyn Error>>;

/// asfDefaultRipple and asfAllowTrustLineClawback (xrpl.org, AccountSet).
const ASF_DEFAULT_RIPPLE: u32 = 8;
const ASF_ALLOW_TRUSTLINE_CLAWBACK: u32 = 16;
/// Seconds between the Unix epoch and the Ripple epoch (2000-01-01).
const RIPPLE_EPOCH_OFFSET: u64 = 946_684_800;

fn mrk(issuer: &[u8; 20], value: &str) -> Res<Amount> {
    Ok(Amount::Issued {
        value: value.to_string(),
        currency: Currency::parse("MRK")?,
        issuer: *issuer,
    })
}

fn report(step: &str, receipt: &Receipt) {
    println!(
        "{}",
        json!({
            "step": step,
            "hash": receipt.hash,
            "result": receipt.result,
            "validated": receipt.validated,
            "ledger_index": receipt.ledger_index,
            "preliminary": receipt.preliminary,
        })
    );
}

fn tx(kind: &str, fields: Vec<(&str, Value)>) -> Res<Transaction> {
    let mut t = Transaction::new(kind)?;
    for (name, value) in fields {
        t.set(name, value)?;
    }
    Ok(t)
}

fn main() -> Res<()> {
    let net = Testnet::connect()?;
    let info = net.call("server_info", json!({}))?;
    println!(
        "{}",
        json!({"step": "network_guard", "network_id": info["info"]["network_id"], "build": info["info"]["build_version"]})
    );

    let issuer: Funded = net.fund_new_account()?;
    let merchant: Funded = net.fund_new_account()?;
    net.wait_for_account(&issuer.address)?;
    net.wait_for_account(&merchant.address)?;
    // The faucet's seed derived to the address it reported (checked inside
    // fund_new_account): a live seed-to-account vector for each account.
    println!(
        "{}",
        json!({"step": "accounts", "issuer": issuer.address, "merchant": merchant.address, "faucet_calls": 2, "seed_derives_to_reported_address": true})
    );
    let (iss, mer) = (issuer.keys.account_id(), merchant.keys.account_id());

    let r = net.submit(
        &issuer.keys,
        tx(
            "AccountSet",
            vec![("SetFlag", Value::U32(ASF_ALLOW_TRUSTLINE_CLAWBACK))],
        )?,
    )?;
    report("issuer_allow_clawback", &r);
    let r = net.submit(
        &issuer.keys,
        tx(
            "AccountSet",
            vec![("SetFlag", Value::U32(ASF_DEFAULT_RIPPLE))],
        )?,
    )?;
    report("issuer_default_ripple", &r);

    let r = net.submit(
        &merchant.keys,
        tx(
            "TrustSet",
            vec![("LimitAmount", Value::Amount(mrk(&iss, "1000")?))],
        )?,
    )?;
    report("merchant_trust_set", &r);
    let r = net.submit(
        &issuer.keys,
        tx(
            "Payment",
            vec![
                ("Amount", Value::Amount(mrk(&iss, "100")?)),
                ("Destination", Value::Account(mer)),
            ],
        )?,
    )?;
    report("issuer_pays_100", &r);

    let offer = net.submit(
        &merchant.keys,
        tx(
            "OfferCreate",
            vec![
                ("TakerGets", Value::Amount(mrk(&iss, "10")?)),
                ("TakerPays", Value::Amount(Amount::Drops(1_000_000))),
            ],
        )?,
    )?;
    report("merchant_offer_create", &offer);
    let offers = net.call(
        "account_offers",
        json!({"account": merchant.address, "ledger_index": "validated"}),
    )?;
    let seq = offers["offers"][0]["seq"]
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .ok_or("no open offer")?;
    let r = net.submit(
        &merchant.keys,
        tx("OfferCancel", vec![("OfferSequence", Value::U32(seq))])?,
    )?;
    report("merchant_offer_cancel", &r);

    let r = net.submit(
        &issuer.keys,
        tx(
            "CheckCreate",
            vec![
                ("Destination", Value::Account(mer)),
                ("SendMax", Value::Amount(mrk(&iss, "5")?)),
            ],
        )?,
    )?;
    report("issuer_check_create", &r);
    let checks = net.call(
        "account_objects",
        json!({"account": issuer.address, "type": "check", "ledger_index": "validated"}),
    )?;
    let check_id: [u8; 32] = decode_hex(
        checks["account_objects"][0]["index"]
            .as_str()
            .ok_or("no check")?,
    )?
    .try_into()
    .map_err(|_| "check id is not 32 bytes")?;
    let r = net.submit(
        &merchant.keys,
        tx(
            "CheckCash",
            vec![
                ("CheckID", Value::Hash256(check_id)),
                ("Amount", Value::Amount(mrk(&iss, "5")?)),
            ],
        )?,
    )?;
    report("merchant_check_cash", &r);

    // The dice commitment, then a reveal with the wrong secret and the right one.
    let mut right = [0u8; 32];
    let mut wrong = [0u8; 32];
    getrandom::fill(&mut right).map_err(|e| e.to_string())?;
    getrandom::fill(&mut wrong).map_err(|e| e.to_string())?;
    let (secret, wrong) = (Secret(right), Secret(wrong));
    let condition = decode_hex(&condition_hex(&secret))?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() - RIPPLE_EPOCH_OFFSET;
    let cancel_after = u32::try_from(now + 900)?;
    let create = net.submit(
        &merchant.keys,
        tx(
            "EscrowCreate",
            vec![
                ("Amount", Value::Amount(Amount::Drops(1))),
                ("Destination", Value::Account(iss)),
                ("CancelAfter", Value::U32(cancel_after)),
                ("Condition", Value::Blob(condition.clone())),
            ],
        )?,
    )?;
    report("merchant_escrow_create", &create);
    let found = net.call("tx", json!({"transaction": create.hash}))?;
    let escrow_seq = found["tx_json"]["Sequence"]
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .ok_or("no escrow sequence")?;
    let fee = Value::Amount(Amount::Drops(finish_fee_drops(32) + 150));
    let finish = |secret: &Secret| -> Res<Transaction> {
        tx(
            "EscrowFinish",
            vec![
                ("Owner", Value::Account(mer)),
                ("OfferSequence", Value::U32(escrow_seq)),
                ("Condition", Value::Blob(condition.clone())),
                (
                    "Fulfillment",
                    Value::Blob(decode_hex(&fulfillment_hex(secret))?),
                ),
                ("Fee", fee.clone()),
            ],
        )
    };
    let r = net.submit(&merchant.keys, finish(&wrong)?)?;
    report("merchant_escrow_finish_wrong_secret", &r);
    let r = net.submit(&merchant.keys, finish(&secret)?)?;
    report("merchant_escrow_finish", &r);

    // Clawback: for a trust line, the amount's issuer field names the holder.
    let r = net.submit(
        &issuer.keys,
        tx("Clawback", vec![("Amount", Value::Amount(mrk(&mer, "3")?))])?,
    )?;
    report("issuer_clawback_3", &r);

    let lines = net.call(
        "account_lines",
        json!({"account": merchant.address, "peer": issuer.address, "ledger_index": "validated"}),
    )?;
    let count = |method: &str, account: &str, kind: Option<&str>, key: &str| -> Res<usize> {
        let mut params = json!({"account": account, "ledger_index": "validated"});
        if let Some(k) = kind {
            params["type"] = json!(k);
        }
        Ok(net.call(method, params)?[key]
            .as_array()
            .map_or(0, |a| a.len()))
    };
    println!(
        "{}",
        json!({
            "step": "final_state",
            "merchant_mrk_balance": lines["lines"][0]["balance"],
            "expected_balance": "102",
            "escrows_left": count("account_objects", &merchant.address, Some("escrow"), "account_objects")?,
            "checks_left": count("account_objects", &issuer.address, Some("check"), "account_objects")?,
            "offers_left": count("account_offers", &merchant.address, None, "offers")?,
        })
    );
    Ok(())
}
