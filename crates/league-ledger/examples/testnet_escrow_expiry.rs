//! Which expiry an escrow needs, measured. rippled 3.4.1's EscrowCreate
//! preflight (src/libxrpl/tx/transactors/escrow/EscrowCreate.cpp) refuses an
//! escrow with neither FinishAfter nor CancelAfter (temBAD_EXPIRATION), for XRP
//! and tokens alike, and one with no FinishAfter needs a Condition. The M2c
//! probe saw only the first refusal and read it as "a token escrow needs
//! CancelAfter". This checks the general rule on the Testnet.
//!
//! **Re-running this now stops before the server.** Since merchant-league #7,
//! the vendored model refuses an escrow with no expiry, or with no
//! FinishAfter and no Condition, and signing runs that check
//! (`xrpl::signing::sign` calls `validate()`). The three refusal steps below
//! would fail locally with a model error. Their purpose was the server's
//! answer, which the receipt records. No way around the local check is
//! added to the library.
//!
//! Run: `cargo run -p league-ledger --example testnet_escrow_expiry`
//!
//! Two faucet accounts: an issuer (with Allow Trust Line Locking) and a
//! holder. Seeds are never printed.

use league_ledger::client::{Account, Receipt, Testnet};
use serde::de::DeserializeOwned;
use serde_json::{Value as Json, json};
use sha2::{Digest, Sha256};
use std::error::Error;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use xrpl::models::transactions::account_set::AccountSet;
use xrpl::models::transactions::escrow_create::EscrowCreate;
use xrpl::models::transactions::escrow_finish::EscrowFinish;
use xrpl::models::transactions::payment::Payment;
use xrpl::models::transactions::trust_set::TrustSet;

type Res<T> = Result<T, Box<dyn Error>>;

const ASF_DEFAULT_RIPPLE: u32 = 8;
const ASF_ALLOW_TRUSTLINE_LOCKING: u32 = 17;
const RIPPLE_EPOCH_OFFSET: u64 = 946_684_800;

fn mrk(issuer: &str, value: &str) -> Json {
    json!({ "currency": "MRK", "issuer": issuer, "value": value })
}

fn condition(secret: &[u8; 32]) -> String {
    let digest = Sha256::digest(secret);
    let hex: String = digest.iter().map(|b| format!("{b:02X}")).collect();
    format!("A0258020{hex}810120")
}

fn build<T: DeserializeOwned>(kind: &str, account: &Account, mut fields: Json) -> Res<T> {
    fields["TransactionType"] = json!(kind);
    fields["Account"] = json!(account.address());
    Ok(serde_json::from_value(fields)?)
}

fn ripple_now() -> Res<u32> {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() - RIPPLE_EPOCH_OFFSET;
    Ok(u32::try_from(secs)?)
}

struct Probe {
    net: Testnet,
    unexpected: Vec<String>,
}

impl Probe {
    fn report(&mut self, step: &str, r: &Receipt, expect: &str) {
        // A tem result never reaches a ledger: its result is the preliminary one.
        let got = if r.validated {
            r.result.as_str()
        } else {
            r.preliminary.as_str()
        };
        let as_expected = got == expect;
        if !as_expected {
            self.unexpected.push(step.to_string());
        }
        println!(
            "{}",
            json!({"step": step, "hash": r.hash, "result": r.result, "validated": r.validated,
                   "ledger_index": r.ledger_index, "preliminary": r.preliminary, "sequence": r.sequence,
                   "expected": expect, "as_expected": as_expected})
        );
    }
}

macro_rules! send {
    ($p:expr, $step:expr, $who:expr, $ty:ty, $kind:expr, $fields:expr, $expect:expr) => {{
        let mut tx: $ty = build($kind, $who, $fields)?;
        let r = $p.net.submit($who, &mut tx)?;
        $p.report($step, &r, $expect);
        r
    }};
}

fn main() -> Res<()> {
    let net = Testnet::connect()?;
    let info = net.call("server_info", json!({}))?;
    println!(
        "{}",
        json!({"step": "network_guard", "network_id": info["info"]["network_id"], "build": info["info"]["build_version"]})
    );
    // A refused (tem) transaction never validates, so polling for it would
    // wait out the whole expiry. Poll briefly.
    let mut p = Probe {
        net: net.with_polling(Duration::from_secs(1), 12),
        unexpected: Vec::new(),
    };
    let issuer = p.net.fund_new_account()?.account;
    let holder = p.net.fund_new_account()?.account;
    for a in [&issuer, &holder] {
        p.net.wait_for_account(a.address())?;
    }
    println!(
        "{}",
        json!({"step": "accounts", "issuer": issuer.address(), "holder": holder.address(), "faucet_calls": 2})
    );
    let iss = issuer.address().to_string();
    let ok = "tesSUCCESS";

    send!(
        p,
        "issuer_allow_locking",
        &issuer,
        AccountSet<'static>,
        "AccountSet",
        json!({"SetFlag": ASF_ALLOW_TRUSTLINE_LOCKING}),
        ok
    );
    send!(
        p,
        "issuer_default_ripple",
        &issuer,
        AccountSet<'static>,
        "AccountSet",
        json!({"SetFlag": ASF_DEFAULT_RIPPLE}),
        ok
    );
    send!(
        p,
        "holder_trust",
        &holder,
        TrustSet<'static>,
        "TrustSet",
        json!({"LimitAmount": mrk(&iss, "1000")}),
        ok
    );
    send!(
        p,
        "issuer_pays_holder_50",
        &issuer,
        Payment<'static>,
        "Payment",
        json!({"Amount": mrk(&iss, "50"), "Destination": holder.address()}),
        ok
    );

    let cond = condition(&[9u8; 32]);
    // Neither expiry: refused for XRP and for a token alike.
    send!(
        p,
        "xrp_escrow_condition_only",
        &holder,
        EscrowCreate<'static>,
        "EscrowCreate",
        json!({"Amount": "1000", "Destination": iss, "Condition": cond}),
        "temBAD_EXPIRATION"
    );
    send!(
        p,
        "token_escrow_condition_only",
        &holder,
        EscrowCreate<'static>,
        "EscrowCreate",
        json!({"Amount": mrk(&iss, "5"), "Destination": iss, "Condition": cond}),
        "temBAD_EXPIRATION"
    );
    // No FinishAfter and no Condition: refused (it could finish at once).
    send!(
        p,
        "token_escrow_cancel_after_only",
        &holder,
        EscrowCreate<'static>,
        "EscrowCreate",
        json!({"Amount": mrk(&iss, "5"), "Destination": iss, "CancelAfter": ripple_now()? + 600}),
        "temMALFORMED"
    );
    // FinishAfter only, no CancelAfter and no Condition: accepted, for a token.
    let finish_after = ripple_now()? + 8;
    let held = send!(
        p,
        "token_escrow_finish_after_only",
        &holder,
        EscrowCreate<'static>,
        "EscrowCreate",
        json!({"Amount": mrk(&iss, "5"), "Destination": iss, "FinishAfter": finish_after}),
        ok
    );
    // Finish it once FinishAfter has passed (a validated ledger's close time
    // must be past it).
    std::thread::sleep(Duration::from_secs(20));
    send!(
        p,
        "token_escrow_finished_after_finish_after",
        &holder,
        EscrowFinish<'static>,
        "EscrowFinish",
        json!({"Owner": holder.address(), "OfferSequence": held.sequence}),
        ok
    );

    println!("{}", json!({"step": "summary", "unexpected": p.unexpected}));
    if p.unexpected.is_empty() {
        Ok(())
    } else {
        Err(format!("unexpected results: {:?}", p.unexpected).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shapes build into xrpl-rust's model (the expiry cases are
    /// deliberately not run through get_errors: some are invalid on purpose).
    #[test]
    fn every_shape_builds_and_encodes() -> Res<()> {
        let w = xrpl::wallet::Wallet::create(Some(xrpl::constants::CryptoAlgorithm::ED25519))
            .map_err(|e| format!("{e}"))?;
        let a = Account::from_seed(&w.seed)?;
        let iss = a.address();
        for fields in [
            json!({"Amount": "1000", "Destination": iss, "Condition": condition(&[1u8; 32])}),
            json!({"Amount": mrk(iss, "5"), "Destination": iss, "CancelAfter": 600}),
            json!({"Amount": mrk(iss, "5"), "Destination": iss, "FinishAfter": 5}),
        ] {
            let tx: EscrowCreate = build("EscrowCreate", &a, fields)?;
            xrpl::core::binarycodec::encode(&tx).map_err(|e| format!("{e}"))?;
        }
        let fin: EscrowFinish = build(
            "EscrowFinish",
            &a,
            json!({"Owner": iss, "OfferSequence": 3}),
        )?;
        xrpl::core::binarycodec::encode(&fin).map_err(|e| format!("{e}"))?;
        Ok(())
    }
}
