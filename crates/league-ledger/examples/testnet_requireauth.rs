//! A design check (docs/design.md, "Check before building"): can one issuer
//! use RequireAuth, Allow Trust Line Clawback and Allow Trust Line Locking
//! together, the way the steward would for a table's goods?
//!
//! Run: `cargo run -p league-ledger --example testnet_requireauth`
//!
//! Four faucet accounts: the issuer, two seated guilds (alice, bob) and an
//! outsider whose trust line is never authorized. Each step prints one JSON
//! line with the hash computed before submitting, the validated result and
//! what was expected (or "recorded" where the ledger's answer is the finding).
//! Seeds are never printed.

use league_ledger::client::{Account, Receipt, Testnet};
use league_ledger::{ESCROW_FINISH_DEFAULT_FEE_DROPS, escrow_condition};
use serde::de::DeserializeOwned;
use serde_json::{Value as Json, json};
use std::error::Error;
use std::time::{SystemTime, UNIX_EPOCH};
use xrpl::models::transactions::account_set::AccountSet;
use xrpl::models::transactions::clawback::Clawback;
use xrpl::models::transactions::escrow_create::EscrowCreate;
use xrpl::models::transactions::escrow_finish::EscrowFinish;
use xrpl::models::transactions::payment::Payment;
use xrpl::models::transactions::trust_set::TrustSet;

type Res<T> = Result<T, Box<dyn Error>>;

/// AccountSet flags (xrpl.org, AccountSet).
const ASF_REQUIRE_AUTH: u32 = 2;
const ASF_DEFAULT_RIPPLE: u32 = 8;
const ASF_ALLOW_TRUSTLINE_CLAWBACK: u32 = 16;
const ASF_ALLOW_TRUSTLINE_LOCKING: u32 = 17;
/// TrustSet tfSetfAuth: the issuer authorizes the holder's line.
const TF_SET_AUTH: u32 = 0x0001_0000;
const RIPPLE_EPOCH_OFFSET: u64 = 946_684_800;

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

struct Probe {
    net: Testnet,
    unexpected: Vec<String>,
}

impl Probe {
    fn report(&mut self, step: &str, r: &Receipt, expect: Option<&str>) {
        let as_expected = expect.map(|e| r.validated && r.result == e);
        if as_expected == Some(false) {
            self.unexpected.push(step.to_string());
        }
        println!(
            "{}",
            json!({"step": step, "hash": r.hash, "result": r.result, "validated": r.validated,
                   "ledger_index": r.ledger_index, "preliminary": r.preliminary, "sequence": r.sequence,
                   "expected": expect, "as_expected": as_expected})
        );
    }

    fn line(&self, holder: &Account, issuer: &Account) -> Res<Json> {
        let lines = self.net.call(
            "account_lines",
            json!({"account": holder.address(), "peer": issuer.address(), "ledger_index": "validated"}),
        )?;
        let l = &lines["lines"][0];
        Ok(
            json!({"balance": l["balance"], "authorized_by_issuer": l["peer_authorized"],
                  "locked": l["locked_balance"]}),
        )
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
    let mut p = Probe {
        net,
        unexpected: Vec::new(),
    };
    let issuer = p.net.fund_new_account()?.account;
    let alice = p.net.fund_new_account()?.account;
    let bob = p.net.fund_new_account()?.account;
    let outsider = p.net.fund_new_account()?.account;
    for a in [&issuer, &alice, &bob, &outsider] {
        p.net.wait_for_account(a.address())?;
    }
    println!(
        "{}",
        json!({"step": "accounts", "issuer": issuer.address(), "alice": alice.address(), "bob": bob.address(),
               "outsider": outsider.address(), "faucet_calls": 4, "ed25519": true})
    );
    let iss = issuer.address().to_string();
    let ok = Some("tesSUCCESS");

    // 1. All four issuer flags, before any trust line exists.
    for (step, flag) in [
        ("issuer_require_auth", ASF_REQUIRE_AUTH),
        ("issuer_allow_clawback", ASF_ALLOW_TRUSTLINE_CLAWBACK),
        ("issuer_allow_locking", ASF_ALLOW_TRUSTLINE_LOCKING),
        ("issuer_default_ripple", ASF_DEFAULT_RIPPLE),
    ] {
        send!(
            p,
            step,
            &issuer,
            AccountSet<'static>,
            "AccountSet",
            json!({"SetFlag": flag}),
            ok
        );
    }
    let flags = p.net.call(
        "account_info",
        json!({"account": iss, "ledger_index": "validated"}),
    )?["account_flags"]
        .clone();
    println!(
        "{}",
        json!({"step": "issuer_flags", "account_flags": flags})
    );

    // 2. Trust lines; only alice and bob are authorized.
    for (step, who) in [
        ("alice_trust", &alice),
        ("bob_trust", &bob),
        ("outsider_trust", &outsider),
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
        "issuer_pays_unauthorized_outsider",
        &issuer,
        Payment<'static>,
        "Payment",
        json!({"Amount": mrk(&iss, "10"), "Destination": outsider.address()}),
        None
    );
    for (step, who) in [
        ("issuer_authorizes_alice", &alice),
        ("issuer_authorizes_bob", &bob),
    ] {
        send!(
            p,
            step,
            &issuer,
            TrustSet<'static>,
            "TrustSet",
            json!({"LimitAmount": mrk(who.address(), "0"), "Flags": TF_SET_AUTH}),
            ok
        );
    }
    send!(
        p,
        "issuer_pays_alice_100",
        &issuer,
        Payment<'static>,
        "Payment",
        json!({"Amount": mrk(&iss, "100"), "Destination": alice.address()}),
        ok
    );
    send!(
        p,
        "alice_pays_unauthorized_outsider",
        &alice,
        Payment<'static>,
        "Payment",
        json!({"Amount": mrk(&iss, "1"), "Destination": outsider.address()}),
        None
    );

    // 3. Token escrows on authorized lines, and to an unauthorized line.
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() - RIPPLE_EPOCH_OFFSET;
    let cancel_after = u32::try_from(now + 900)?;
    let (cond, fulfil) = escrow_condition(&random_secret()?);
    let escrow = |to: &Account, amount: &str| {
        json!({"Amount": mrk(&iss, amount), "Destination": to.address(), "Condition": cond,
               "CancelAfter": cancel_after})
    };
    let finish = |owner: &Account, seq: u32| {
        json!({"Owner": owner.address(), "OfferSequence": seq, "Condition": cond,
               "Fulfillment": fulfil, "Fee": ESCROW_FINISH_DEFAULT_FEE_DROPS})
    };
    let first = send!(
        p,
        "alice_escrows_10_to_bob",
        &alice,
        EscrowCreate<'static>,
        "EscrowCreate",
        escrow(&bob, "10"),
        ok
    );
    send!(
        p,
        "bob_finishes_10",
        &bob,
        EscrowFinish<'static>,
        "EscrowFinish",
        finish(&alice, first.sequence),
        ok
    );
    send!(
        p,
        "alice_escrows_5_to_unauthorized_outsider",
        &alice,
        EscrowCreate<'static>,
        "EscrowCreate",
        escrow(&outsider, "5"),
        None
    );

    // 4. Clawback on an authorized line, then with tokens locked in escrow.
    send!(
        p,
        "issuer_claws_3_from_alice",
        &issuer,
        Clawback<'static>,
        "Clawback",
        json!({"Amount": mrk(alice.address(), "3")}),
        ok
    );
    let held = send!(
        p,
        "alice_escrows_20_to_bob",
        &alice,
        EscrowCreate<'static>,
        "EscrowCreate",
        escrow(&bob, "20"),
        ok
    );
    println!(
        "{}",
        json!({"step": "alice_line_with_20_locked", "line": p.line(&alice, &issuer)?})
    );
    // Alice holds 87 with 20 of it in escrow. Ask for all 87 back: the
    // ledger's answer says whether clawback reaches escrowed tokens.
    send!(
        p,
        "issuer_claws_87_from_alice",
        &issuer,
        Clawback<'static>,
        "Clawback",
        json!({"Amount": mrk(alice.address(), "87")}),
        None
    );
    println!(
        "{}",
        json!({"step": "alice_line_after_clawback", "line": p.line(&alice, &issuer)?})
    );
    send!(
        p,
        "bob_finishes_20_after_clawback",
        &bob,
        EscrowFinish<'static>,
        "EscrowFinish",
        finish(&alice, held.sequence),
        None
    );

    println!(
        "{}",
        json!({"step": "final_state",
               "alice": p.line(&alice, &issuer)?, "bob": p.line(&bob, &issuer)?,
               "outsider": p.line(&outsider, &issuer)?})
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

    /// Every shape the probe sends builds, validates and encodes, before any
    /// faucet call: `cargo test -p league-ledger --example testnet_requireauth`.
    #[test]
    fn every_shape_builds_validates_and_encodes() -> Res<()> {
        let w = xrpl::wallet::Wallet::create(Some(xrpl::constants::CryptoAlgorithm::ED25519))
            .map_err(|e| format!("{e}"))?;
        let a = Account::from_seed(&w.seed)?;
        let iss = a.address();
        let (cond, fulfil) = escrow_condition(&[7u8; 32]);
        fn check<T: serde::Serialize + Model>(tx: T) -> Res<()> {
            tx.get_errors().map_err(|e| format!("{e}"))?;
            xrpl::core::binarycodec::encode(&tx).map_err(|e| format!("{e}"))?;
            Ok(())
        }
        check(build::<AccountSet>(
            "AccountSet",
            &a,
            json!({"SetFlag": ASF_REQUIRE_AUTH}),
        )?)?;
        let auth: TrustSet = build(
            "TrustSet",
            &a,
            json!({"LimitAmount": mrk(iss, "0"), "Flags": TF_SET_AUTH}),
        )?;
        let blob = xrpl::core::binarycodec::encode(&auth).map_err(|e| format!("{e}"))?;
        assert!(
            blob.contains("2200010000"),
            "tfSetfAuth must encode: {blob}"
        );
        check(auth)?;
        check(build::<EscrowCreate>(
            "EscrowCreate",
            &a,
            json!({"Amount": mrk(iss, "5"), "Destination": iss, "Condition": cond, "CancelAfter": 1}),
        )?)?;
        check(build::<EscrowFinish>(
            "EscrowFinish",
            &a,
            json!({"Owner": iss, "OfferSequence": 3, "Condition": cond, "Fulfillment": fulfil, "Fee": ESCROW_FINISH_DEFAULT_FEE_DROPS}),
        )?)?;
        // For a trust line, the clawed amount's issuer field names the holder.
        let holder = xrpl::wallet::Wallet::create(Some(xrpl::constants::CryptoAlgorithm::ED25519))
            .map_err(|e| format!("{e}"))?
            .classic_address
            .clone();
        check(build::<Clawback>(
            "Clawback",
            &a,
            json!({"Amount": mrk(&holder, "87")}),
        )?)?;
        Ok(())
    }
}
