//! M2c proof: the patched xrpl-rust, through the league's client, on the
//! live XRPL Testnet.
//!
//! Run: `cargo run -p league-ledger --example testnet_proof`
//!
//! 1. The M2 escrow suite: trust line, payment, offer and cancel, check and
//!    cash, an escrow with a condition (wrong secret, then the right one),
//!    clawback.
//! 2. TokenEscrow: a token escrow refused while the issuer lacks the Allow
//!    Trust Line Locking flag, then a probe without `CancelAfter`, then a
//!    token escrow with a condition, finished.
//! 3. DomainID: credentials, a permissioned domain, a credentialed offer, an
//!    outsider refused, an open-book offer that does not cross the domain's,
//!    a second member's offer that does, and a payment carrying a DomainID.
//!
//! Four faucet accounts, one call each. Each step prints one JSON line: the
//! hash computed before submitting, the validated result and the expected
//! one. Seeds are never printed.

use league_ledger::client::{Account, Receipt, Testnet};
use serde::de::DeserializeOwned;
use serde_json::{Value as Json, json};
use sha2::{Digest, Sha256};
use std::error::Error;
use std::time::{SystemTime, UNIX_EPOCH};
use xrpl::models::transactions::Credential;
use xrpl::models::transactions::account_set::AccountSet;
use xrpl::models::transactions::check_cash::CheckCash;
use xrpl::models::transactions::check_create::CheckCreate;
use xrpl::models::transactions::clawback::Clawback;
use xrpl::models::transactions::credential_accept::CredentialAccept;
use xrpl::models::transactions::credential_create::CredentialCreate;
use xrpl::models::transactions::escrow_create::EscrowCreate;
use xrpl::models::transactions::escrow_finish::EscrowFinish;
use xrpl::models::transactions::offer_cancel::OfferCancel;
use xrpl::models::transactions::offer_create::OfferCreate;
use xrpl::models::transactions::payment::Payment;
use xrpl::models::transactions::permissioned_domain_set::PermissionedDomainSet;
use xrpl::models::transactions::trust_set::TrustSet;

type Res<T> = Result<T, Box<dyn Error>>;

/// AccountSet flags (xrpl.org, AccountSet).
const ASF_DEFAULT_RIPPLE: u32 = 8;
const ASF_ALLOW_TRUSTLINE_CLAWBACK: u32 = 16;
const ASF_ALLOW_TRUSTLINE_LOCKING: u32 = 17;
/// Seconds between the Unix epoch and the Ripple epoch (2000-01-01).
const RIPPLE_EPOCH_OFFSET: u64 = 946_684_800;
/// "MerchantLeague", hex: the credential type the domain accepts.
const CREDENTIAL_TYPE: &str = "4D65726368616E744C6561677565";
/// An EscrowFinish with a 32-byte preimage costs 10 * (33 + 36/16) = 350
/// drops at the base fee; 500 leaves room.
const FINISH_FEE: &str = "500";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

/// PREIMAGE-SHA-256 crypto-condition and fulfillment for a 32-byte secret.
fn condition(secret: &[u8; 32]) -> (String, String) {
    let digest = Sha256::digest(secret);
    (
        format!("A0258020{}810120", hex(&digest)),
        format!("A0228020{}", hex(secret)),
    )
}

fn random_secret() -> Res<[u8; 32]> {
    let mut s = [0u8; 32];
    getrandom::fill(&mut s).map_err(|e| e.to_string())?;
    Ok(s)
}

fn mrk(issuer: &str, value: &str) -> Json {
    json!({ "currency": "MRK", "issuer": issuer, "value": value })
}

fn build<T: DeserializeOwned>(kind: &str, account: &Account, mut fields: Json) -> Res<T> {
    fields["TransactionType"] = json!(kind);
    fields["Account"] = json!(account.address());
    Ok(serde_json::from_value(fields)?)
}

/// A new permissioned domain accepting the league's credential. Built with
/// the constructor: xrpl-rust's `Credential` deserialises only from borrowed
/// text, so it can't come from a `serde_json::Value`.
fn domain_set(owner: &Account, issuer: &str) -> PermissionedDomainSet<'static> {
    PermissionedDomainSet::new(
        owner.address().to_string().into(),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        vec![Credential::new(
            issuer.to_string(),
            CREDENTIAL_TYPE.to_string(),
        )],
    )
}

struct Proof {
    net: Testnet,
    unexpected: Vec<String>,
}

impl Proof {
    fn report(&mut self, step: &str, receipt: &Receipt, expect: Option<&str>) {
        let as_expected = expect.map(|e| receipt.validated && receipt.result == e);
        if as_expected == Some(false) {
            self.unexpected.push(step.to_string());
        }
        println!(
            "{}",
            json!({
                "step": step,
                "hash": receipt.hash,
                "result": receipt.result,
                "validated": receipt.validated,
                "ledger_index": receipt.ledger_index,
                "preliminary": receipt.preliminary,
                "sequence": receipt.sequence,
                "expected": expect,
                "as_expected": as_expected,
            })
        );
    }

    fn mrk_balance(&self, holder: &Account, issuer: &Account) -> Res<Json> {
        let lines = self.net.call(
            "account_lines",
            json!({"account": holder.address(), "peer": issuer.address(), "ledger_index": "validated"}),
        )?;
        Ok(lines["lines"][0]["balance"].clone())
    }

    fn objects(&self, account: &Account, kind: &str) -> Res<Vec<Json>> {
        let reply = self.net.call(
            "account_objects",
            json!({"account": account.address(), "type": kind, "ledger_index": "validated"}),
        )?;
        Ok(reply["account_objects"]
            .as_array()
            .cloned()
            .unwrap_or_default())
    }

    fn offers(&self, account: &Account) -> Res<Vec<Json>> {
        let reply = self.net.call(
            "account_offers",
            json!({"account": account.address(), "ledger_index": "validated"}),
        )?;
        Ok(reply["offers"].as_array().cloned().unwrap_or_default())
    }
}

macro_rules! send {
    ($p:expr, $step:expr, $who:expr, $ty:ty, $kind:expr, $fields:expr, $expect:expr) => {{
        let mut tx: $ty = build($kind, $who, $fields)?;
        let receipt = $p.net.submit($who, &mut tx)?;
        $p.report($step, &receipt, $expect);
        receipt
    }};
}

fn main() -> Res<()> {
    let net = Testnet::connect()?;
    let info = net.call("server_info", json!({}))?;
    println!(
        "{}",
        json!({"step": "network_guard", "network_id": info["info"]["network_id"], "build": info["info"]["build_version"]})
    );
    let mut p = Proof {
        net,
        unexpected: Vec::new(),
    };

    let issuer = p.net.fund_new_account()?.account;
    let merchant = p.net.fund_new_account()?.account;
    let trader = p.net.fund_new_account()?.account;
    let outsider = p.net.fund_new_account()?.account;
    for a in [&issuer, &merchant, &trader, &outsider] {
        p.net.wait_for_account(a.address())?;
    }
    // Each faucet seed was checked to be ed25519 and to derive to the
    // address the faucet reported (inside fund_new_account).
    println!(
        "{}",
        json!({"step": "accounts", "issuer": issuer.address(), "merchant": merchant.address(),
               "trader": trader.address(), "outsider": outsider.address(), "faucet_calls": 4,
               "ed25519": true, "seed_derives_to_reported_address": true})
    );
    let iss = issuer.address().to_string();
    let ok = Some("tesSUCCESS");

    // 1. The M2 escrow suite.
    send!(
        p,
        "issuer_allow_clawback",
        &issuer,
        AccountSet<'static>,
        "AccountSet",
        json!({"SetFlag": ASF_ALLOW_TRUSTLINE_CLAWBACK}),
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
    for (step, who) in [
        ("merchant_trust_set", &merchant),
        ("trader_trust_set", &trader),
        ("outsider_trust_set", &outsider),
    ] {
        send!(
            p,
            step,
            who,
            TrustSet<'static>,
            "TrustSet",
            json!({"LimitAmount": mrk(&iss, "1000")}),
            ok
        );
    }
    send!(
        p,
        "issuer_pays_merchant_100",
        &issuer,
        Payment<'static>,
        "Payment",
        json!({"Amount": mrk(&iss, "100"), "Destination": merchant.address()}),
        ok
    );

    let offer = send!(
        p,
        "merchant_offer_create",
        &merchant,
        OfferCreate<'static>,
        "OfferCreate",
        json!({"TakerGets": mrk(&iss, "10"), "TakerPays": "1000000"}),
        ok
    );
    send!(
        p,
        "merchant_offer_cancel",
        &merchant,
        OfferCancel<'static>,
        "OfferCancel",
        json!({"OfferSequence": offer.sequence}),
        ok
    );

    send!(
        p,
        "issuer_check_create",
        &issuer,
        CheckCreate<'static>,
        "CheckCreate",
        json!({"Destination": merchant.address(), "SendMax": mrk(&iss, "5")}),
        ok
    );
    let check_id = p
        .objects(&issuer, "check")?
        .first()
        .and_then(|c| c["index"].as_str().map(str::to_string))
        .ok_or("no check")?;
    send!(
        p,
        "merchant_check_cash",
        &merchant,
        CheckCash<'static>,
        "CheckCash",
        json!({"CheckID": check_id, "Amount": mrk(&iss, "5")}),
        ok
    );

    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() - RIPPLE_EPOCH_OFFSET;
    let cancel_after = u32::try_from(now + 900)?;
    let (secret, wrong) = (random_secret()?, random_secret()?);
    let (cond, fulfil) = condition(&secret);
    let (_, wrong_fulfil) = condition(&wrong);
    let escrow = send!(
        p,
        "merchant_escrow_create",
        &merchant,
        EscrowCreate<'static>,
        "EscrowCreate",
        json!({"Amount": "1", "Destination": iss, "CancelAfter": cancel_after, "Condition": cond}),
        ok
    );
    let finish = |seq: u32, owner: &Account, f: &str| json!({"Owner": owner.address(), "OfferSequence": seq, "Condition": cond, "Fulfillment": f, "Fee": FINISH_FEE});
    send!(
        p,
        "merchant_escrow_finish_wrong_secret",
        &merchant,
        EscrowFinish<'static>,
        "EscrowFinish",
        finish(escrow.sequence, &merchant, &wrong_fulfil),
        Some("tecCRYPTOCONDITION_ERROR")
    );
    send!(
        p,
        "merchant_escrow_finish",
        &merchant,
        EscrowFinish<'static>,
        "EscrowFinish",
        finish(escrow.sequence, &merchant, &fulfil),
        ok
    );

    // For a trust line, the clawed amount's issuer field names the holder.
    send!(
        p,
        "issuer_clawback_3",
        &issuer,
        Clawback<'static>,
        "Clawback",
        json!({"Amount": mrk(merchant.address(), "3")}),
        ok
    );

    // 2. TokenEscrow.
    let token_escrow = |cancel: Option<u32>| {
        let mut f =
            json!({"Amount": mrk(&iss, "5"), "Destination": trader.address(), "Condition": cond});
        if let Some(c) = cancel {
            f["CancelAfter"] = json!(c);
        }
        f
    };
    send!(
        p,
        "token_escrow_without_locking_flag",
        &merchant,
        EscrowCreate<'static>,
        "EscrowCreate",
        token_escrow(Some(cancel_after)),
        Some("tecNO_PERMISSION")
    );
    send!(
        p,
        "issuer_allow_trustline_locking",
        &issuer,
        AccountSet<'static>,
        "AccountSet",
        json!({"SetFlag": ASF_ALLOW_TRUSTLINE_LOCKING}),
        ok
    );
    // xrpl.org's EscrowCreate reference doesn't say a token escrow needs
    // CancelAfter. Record what the ledger does.
    let probe = send!(
        p,
        "token_escrow_probe_no_cancel_after",
        &merchant,
        EscrowCreate<'static>,
        "EscrowCreate",
        token_escrow(None),
        None
    );
    let token = send!(
        p,
        "token_escrow_create",
        &merchant,
        EscrowCreate<'static>,
        "EscrowCreate",
        token_escrow(Some(cancel_after)),
        ok
    );
    send!(
        p,
        "token_escrow_finish_by_trader",
        &trader,
        EscrowFinish<'static>,
        "EscrowFinish",
        finish(token.sequence, &merchant, &fulfil),
        ok
    );
    if probe.validated && probe.result == "tesSUCCESS" {
        send!(
            p,
            "token_escrow_probe_finish",
            &trader,
            EscrowFinish<'static>,
            "EscrowFinish",
            finish(probe.sequence, &merchant, &fulfil),
            ok
        );
    }

    // 3. DomainID: credentials, a domain, offers inside and outside it.
    for (step, who) in [
        ("issuer_credential_for_merchant", &merchant),
        ("issuer_credential_for_trader", &trader),
    ] {
        send!(
            p,
            step,
            &issuer,
            CredentialCreate<'static>,
            "CredentialCreate",
            json!({"Subject": who.address(), "CredentialType": CREDENTIAL_TYPE}),
            ok
        );
    }
    for (step, who) in [
        ("merchant_credential_accept", &merchant),
        ("trader_credential_accept", &trader),
    ] {
        send!(
            p,
            step,
            who,
            CredentialAccept<'static>,
            "CredentialAccept",
            json!({"Issuer": iss, "CredentialType": CREDENTIAL_TYPE}),
            ok
        );
    }
    let r = p.net.submit(&issuer, &mut domain_set(&issuer, &iss))?;
    p.report("issuer_domain_set", &r, ok);
    let domain = p
        .objects(&issuer, "permissioned_domain")?
        .first()
        .and_then(|d| d["index"].as_str().map(str::to_string))
        .ok_or("no permissioned domain")?;
    println!("{}", json!({"step": "domain_id", "domain_id": domain}));

    let sell = json!({"TakerGets": mrk(&iss, "1"), "TakerPays": "1000000", "DomainID": domain});
    let buy = |d: Option<&str>| {
        let mut f = json!({"TakerGets": "1000000", "TakerPays": mrk(&iss, "1")});
        if let Some(d) = d {
            f["DomainID"] = json!(d);
        }
        f
    };
    send!(
        p,
        "merchant_domain_offer_sell_1",
        &merchant,
        OfferCreate<'static>,
        "OfferCreate",
        sell,
        ok
    );
    send!(
        p,
        "outsider_open_offer_buy_1",
        &outsider,
        OfferCreate<'static>,
        "OfferCreate",
        buy(None),
        ok
    );
    let outsider_after_open = p.mrk_balance(&outsider, &issuer)?;
    println!(
        "{}",
        json!({"step": "open_book_did_not_cross_domain", "outsider_mrk": outsider_after_open, "expected": "0"})
    );
    send!(
        p,
        "outsider_domain_offer_refused",
        &outsider,
        OfferCreate<'static>,
        "OfferCreate",
        buy(Some(&domain)),
        Some("tecNO_PERMISSION")
    );
    send!(
        p,
        "trader_domain_offer_buy_1",
        &trader,
        OfferCreate<'static>,
        "OfferCreate",
        buy(Some(&domain)),
        ok
    );
    send!(
        p,
        "merchant_domain_payment_to_trader",
        &merchant,
        Payment<'static>,
        "Payment",
        json!({"Amount": "1000000", "Destination": trader.address(), "DomainID": domain}),
        None
    );

    println!(
        "{}",
        json!({
            "step": "final_state",
            "merchant_mrk": p.mrk_balance(&merchant, &issuer)?,
            "trader_mrk": p.mrk_balance(&trader, &issuer)?,
            "outsider_mrk": p.mrk_balance(&outsider, &issuer)?,
            "merchant_escrows_left": p.objects(&merchant, "escrow")?.len(),
            "issuer_checks_left": p.objects(&issuer, "check")?.len(),
            "merchant_offers_left": p.offers(&merchant)?.len(),
            "trader_offers_left": p.offers(&trader)?.len(),
            "outsider_offers_left": p.offers(&outsider)?.len(),
            "credentials_held_by_issuer": p.objects(&issuer, "credential")?.len(),
        })
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
    use xrpl::models::Model;

    fn someone() -> Res<Account> {
        let w = xrpl::wallet::Wallet::create(Some(xrpl::constants::CryptoAlgorithm::ED25519))
            .map_err(|e| format!("{e:?}"))?;
        Ok(Account::from_seed(&w.seed)?)
    }

    /// Every transaction shape the proof sends deserialises into
    /// xrpl-rust's model, validates and encodes. Run before spending faucet
    /// calls: `cargo test -p league-ledger --example testnet_proof`.
    #[test]
    fn every_shape_builds_validates_and_encodes() -> Res<()> {
        let a = someone()?;
        let b = someone()?;
        let iss = b.address();
        let (cond, fulfil) = condition(&[7u8; 32]);
        let domain = "C6B8F21B5D6B0A8E8C2B1F4E3D2A1908172635445362718293A4B5C6D7E8F901";
        fn check<T: serde::Serialize + Model>(tx: T) -> Res<()> {
            tx.get_errors().map_err(|e| format!("{e:?}"))?;
            xrpl::core::binarycodec::encode(&tx).map_err(|e| format!("{e:?}"))?;
            Ok(())
        }
        check(build::<AccountSet>(
            "AccountSet",
            &a,
            json!({"SetFlag": ASF_ALLOW_TRUSTLINE_LOCKING}),
        )?)?;
        check(build::<TrustSet>(
            "TrustSet",
            &a,
            json!({"LimitAmount": mrk(iss, "1000")}),
        )?)?;
        check(build::<Payment>(
            "Payment",
            &a,
            json!({"Amount": mrk(iss, "1"), "Destination": iss}),
        )?)?;
        check(build::<Payment>(
            "Payment",
            &a,
            json!({"Amount": "1000000", "Destination": iss, "DomainID": domain}),
        )?)?;
        check(build::<OfferCreate>(
            "OfferCreate",
            &a,
            json!({"TakerGets": mrk(iss, "1"), "TakerPays": "1000000", "DomainID": domain}),
        )?)?;
        check(build::<OfferCancel>(
            "OfferCancel",
            &a,
            json!({"OfferSequence": 5}),
        )?)?;
        check(build::<CheckCreate>(
            "CheckCreate",
            &a,
            json!({"Destination": iss, "SendMax": mrk(iss, "5")}),
        )?)?;
        check(build::<CheckCash>(
            "CheckCash",
            &a,
            json!({"CheckID": domain, "Amount": mrk(iss, "5")}),
        )?)?;
        check(build::<EscrowCreate>(
            "EscrowCreate",
            &a,
            json!({"Amount": "1", "Destination": iss, "CancelAfter": 1, "Condition": cond}),
        )?)?;
        check(build::<EscrowCreate>(
            "EscrowCreate",
            &a,
            json!({"Amount": mrk(iss, "5"), "Destination": iss, "CancelAfter": 1, "Condition": cond}),
        )?)?;
        check(build::<EscrowFinish>(
            "EscrowFinish",
            &a,
            json!({"Owner": iss, "OfferSequence": 3, "Condition": cond, "Fulfillment": fulfil, "Fee": FINISH_FEE}),
        )?)?;
        check(build::<Clawback>(
            "Clawback",
            &a,
            json!({"Amount": mrk(iss, "3")}),
        )?)?;
        check(build::<CredentialCreate>(
            "CredentialCreate",
            &a,
            json!({"Subject": iss, "CredentialType": CREDENTIAL_TYPE}),
        )?)?;
        check(build::<CredentialAccept>(
            "CredentialAccept",
            &a,
            json!({"Issuer": iss, "CredentialType": CREDENTIAL_TYPE}),
        )?)?;
        check(domain_set(&a, iss))?;
        Ok(())
    }

    #[test]
    fn the_condition_has_the_published_shape() {
        let (c, f) = condition(&[0u8; 32]);
        // SHA-256 of 32 zero bytes.
        assert_eq!(
            c,
            "A025802066687AADF862BD776C8FC18B8E9F8E20089714856EE233B3902A591D0D5F2925810120"
        );
        assert_eq!(f, format!("A0228020{}", "00".repeat(32)));
    }
}
