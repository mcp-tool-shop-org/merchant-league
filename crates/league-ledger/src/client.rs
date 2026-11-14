//! A small JSON-RPC client for the XRPL Testnet, and nothing else.
//!
//! Transactions are built and signed with xrpl-rust (vendored, patched; see
//! `vendor/xrpl-rust/VENDORED.md`). xrpl-rust's own network clients are left
//! out of the build, so submission goes through this client:
//!
//! - [`Testnet::connect`] checks the server's network id before anything is
//!   sent ([`require_testnet`](crate::require_testnet)).
//! - [`Testnet::submit`] fills in the sequence, fee and expiry, signs,
//!   computes the transaction's hash **locally**, submits, and waits for the
//!   result in a validated ledger. The hash is known before submission and
//!   is the receipt's key, so a `tec` result has its hash too.
//!
//! The HTTP layer sits behind [`Transport`], so everything above it can be
//! tested without a network.

use crate::{NotTestnet, TESTNET_FAUCET, TESTNET_JSON_RPC, require_testnet};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value as Json, json};
use sha2::{Digest, Sha512};
use std::fmt;
use std::time::Duration;
use strum::IntoEnumIterator;
use xrpl::core::binarycodec::encode;
use xrpl::models::transactions::Transaction;
use xrpl::models::{Model, XRPAmount};
use xrpl::wallet::Wallet;

/// Posts a JSON body to a URL and returns the JSON reply.
pub trait Transport {
    /// Send `body` to `url`.
    fn post(&self, url: &str, body: &Json) -> Result<Json, String>;
}

/// The real transport: HTTPS through `ureq`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Ureq;

impl Transport for Ureq {
    fn post(&self, url: &str, body: &Json) -> Result<Json, String> {
        ureq::post(url)
            .send_json(body)
            .map_err(|e| e.to_string())?
            .body_mut()
            .read_json()
            .map_err(|e| e.to_string())
    }
}

/// Something went wrong talking to the ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedgerError {
    /// The server is not the Testnet.
    NotTestnet(NotTestnet),
    /// The request could not be sent or the reply read.
    Transport(String),
    /// The server answered with an error.
    Rpc {
        /// The method called.
        method: String,
        /// The server's error code.
        error: String,
    },
    /// The reply did not have the expected shape.
    Reply(String),
    /// xrpl-rust could not build, sign or encode the transaction.
    Build(String),
    /// A seed the game will not use: only ed25519 (`sEd...`) seeds.
    NotEd25519,
}

impl fmt::Display for LedgerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LedgerError::NotTestnet(e) => write!(f, "{e}"),
            LedgerError::Transport(e) => write!(f, "could not reach the Testnet: {e}"),
            LedgerError::Rpc { method, error } => write!(f, "{method} failed: {error}"),
            LedgerError::Reply(e) => write!(f, "unexpected reply: {e}"),
            LedgerError::Build(e) => write!(f, "could not build the transaction: {e}"),
            LedgerError::NotEd25519 => {
                f.write_str("not an ed25519 seed (sEd...); the game uses ed25519 accounts only")
            }
        }
    }
}

impl std::error::Error for LedgerError {}

/// A seed returned by the faucet. Never printed.
pub struct Seed(String);

impl Seed {
    /// The seed text. Handle with care: it controls the account.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Seed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Seed(<hidden>)")
    }
}

/// An ed25519 Testnet account. Its keys are derived once, when it is made,
/// never per move. Never prints its seed or private key.
pub struct Account {
    wallet: Wallet,
}

impl fmt::Debug for Account {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Account({}, <keys hidden>)", self.wallet.classic_address)
    }
}

impl Account {
    /// The account for an ed25519 seed. secp256k1 seeds are refused.
    pub fn from_seed(seed: &str) -> Result<Self, LedgerError> {
        if !seed.starts_with("sEd") {
            return Err(LedgerError::NotEd25519);
        }
        let wallet = Wallet::new(seed, 0).map_err(|e| LedgerError::Build(format!("{e:?}")))?;
        if !wallet.public_key.to_ascii_uppercase().starts_with("ED") {
            return Err(LedgerError::NotEd25519);
        }
        Ok(Account { wallet })
    }

    /// The classic address (`r...`).
    pub fn address(&self) -> &str {
        &self.wallet.classic_address
    }

    /// The public key, as hex (`ED...`).
    pub fn public_key(&self) -> &str {
        &self.wallet.public_key
    }
}

/// A newly funded Testnet account.
#[derive(Debug)]
pub struct Funded {
    /// The account, keys derived.
    pub account: Account,
    /// The seed, as the faucet issued it.
    pub seed: Seed,
}

/// The outcome of a submitted transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    /// The transaction hash, computed before submission.
    pub hash: String,
    /// The result code from a validated ledger, or the preliminary one if
    /// the transaction never validated.
    pub result: String,
    /// Whether `result` comes from a validated ledger.
    pub validated: bool,
    /// The validated ledger that holds it.
    pub ledger_index: Option<u64>,
    /// The result the server gave on submission (not final).
    pub preliminary: String,
    /// The account sequence the transaction used. An escrow or offer is
    /// later named by it (`OfferSequence`).
    pub sequence: u32,
}

/// The hash of a signed transaction blob: SHA-512Half of `TXN\0` + blob.
pub fn transaction_hash(signed_blob: &[u8]) -> String {
    let mut data = vec![0x54, 0x58, 0x4E, 0x00];
    data.extend_from_slice(signed_blob);
    let digest = Sha512::digest(&data);
    upper_hex(&digest[..32])
}

fn upper_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

/// Decode hex, upper or lower case.
pub fn decode_hex(text: &str) -> Result<Vec<u8>, String> {
    if !text.len().is_multiple_of(2) {
        return Err("odd length".into());
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).map_err(|_| format!("bad hex at {i}")))
        .collect()
}

/// How long a transaction may wait to be included, in ledgers.
const EXPIRY_LEDGERS: u64 = 20;
/// The smallest fee sent, in drops.
const MIN_FEE_DROPS: u64 = 12;

/// The XRPL Testnet, through one JSON-RPC endpoint.
pub struct Testnet<T: Transport = Ureq> {
    transport: T,
    rpc_url: String,
    faucet_url: String,
    poll: Duration,
    max_polls: u32,
}

impl Testnet<Ureq> {
    /// Connect to the public Testnet, checking the network id first.
    pub fn connect() -> Result<Self, LedgerError> {
        Testnet::with_transport(Ureq, TESTNET_JSON_RPC, TESTNET_FAUCET)
    }
}

impl<T: Transport> Testnet<T> {
    /// Connect through a given transport. The server must report network id
    /// 1, or nothing else is ever sent.
    pub fn with_transport(
        transport: T,
        rpc_url: &str,
        faucet_url: &str,
    ) -> Result<Self, LedgerError> {
        let me = Testnet {
            transport,
            rpc_url: rpc_url.to_string(),
            faucet_url: faucet_url.to_string(),
            poll: Duration::from_secs(1),
            max_polls: 60,
        };
        let info = me.call("server_info", json!({}))?;
        let reported = info["info"]["network_id"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok());
        require_testnet(reported).map_err(LedgerError::NotTestnet)?;
        Ok(me)
    }

    /// How often, and how many times, [`submit`](Self::submit) checks for a
    /// validated result.
    pub fn with_polling(mut self, every: Duration, times: u32) -> Self {
        self.poll = every;
        self.max_polls = times;
        self
    }

    /// Call a JSON-RPC method and return its `result`.
    pub fn call(&self, method: &str, params: Json) -> Result<Json, LedgerError> {
        let body = json!({ "method": method, "params": [params] });
        let reply = self
            .transport
            .post(&self.rpc_url, &body)
            .map_err(LedgerError::Transport)?;
        let result = reply
            .get("result")
            .cloned()
            .ok_or_else(|| LedgerError::Reply(format!("{method}: no result")))?;
        if let Some(error) = result.get("error").and_then(|e| e.as_str()) {
            return Err(LedgerError::Rpc {
                method: method.to_string(),
                error: error.to_string(),
            });
        }
        Ok(result)
    }

    /// Fund a new account from the Testnet faucet: one call. The seed must
    /// be ed25519 and derive to the address the faucet reports.
    pub fn fund_new_account(&self) -> Result<Funded, LedgerError> {
        let reply = self
            .transport
            .post(&self.faucet_url, &json!({ "userAgent": "merchant-league" }))
            .map_err(LedgerError::Transport)?;
        let seed = reply["seed"]
            .as_str()
            .or_else(|| reply["account"]["secret"].as_str())
            .ok_or_else(|| LedgerError::Reply("faucet: no seed".into()))?;
        let reported = reply["account"]["classicAddress"]
            .as_str()
            .or_else(|| reply["account"]["address"].as_str())
            .ok_or_else(|| LedgerError::Reply("faucet: no address".into()))?;
        let account = Account::from_seed(seed)?;
        if account.address() != reported {
            return Err(LedgerError::Reply(format!(
                "faucet: the seed derives to {}, not the reported {reported}",
                account.address()
            )));
        }
        Ok(Funded {
            account,
            seed: Seed(seed.to_string()),
        })
    }

    /// Wait until `address` exists in a validated ledger.
    pub fn wait_for_account(&self, address: &str) -> Result<(), LedgerError> {
        for _ in 0..self.max_polls {
            match self.call(
                "account_info",
                json!({ "account": address, "ledger_index": "validated" }),
            ) {
                Ok(_) => return Ok(()),
                Err(LedgerError::Rpc { error, .. }) if error == "actNotFound" => {
                    std::thread::sleep(self.poll)
                }
                Err(e) => return Err(e),
            }
        }
        Err(LedgerError::Reply(format!("{address} did not appear")))
    }

    /// Fill in, sign, hash, submit and wait for `tx` from `account`.
    ///
    /// `Sequence`, `LastLedgerSequence` and (if not set) `Fee` are filled in
    /// here; xrpl-rust's `sign` sets `SigningPubKey` and `TxnSignature`. A
    /// result that never validates comes back with `validated: false`.
    pub fn submit<'a, Tx, F>(&self, account: &Account, tx: &mut Tx) -> Result<Receipt, LedgerError>
    where
        F: IntoEnumIterator + Serialize + fmt::Debug + PartialEq,
        Tx: Transaction<'a, F> + Model + Serialize + DeserializeOwned + Clone + fmt::Debug,
    {
        let info = self.call(
            "account_info",
            json!({ "account": account.address(), "ledger_index": "current" }),
        )?;
        let sequence = info["account_data"]["Sequence"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| LedgerError::Reply("account_info: no Sequence".into()))?;
        let current = self.call("ledger_current", json!({}))?["ledger_current_index"]
            .as_u64()
            .ok_or_else(|| LedgerError::Reply("ledger_current: no index".into()))?;
        let last = u32::try_from(current + EXPIRY_LEDGERS)
            .map_err(|_| LedgerError::Reply("ledger index too large".into()))?;
        let fee_needed = tx.get_common_fields().fee.is_none();
        let fee = if fee_needed {
            let reply = self.call("fee", json!({}))?;
            let open = reply["drops"]["open_ledger_fee"]
                .as_str()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(MIN_FEE_DROPS);
            Some(open.max(MIN_FEE_DROPS))
        } else {
            None
        };
        {
            let common = tx.get_mut_common_fields();
            common.sequence = Some(sequence);
            common.last_ledger_sequence = Some(last);
            if let Some(drops) = fee {
                common.fee = Some(XRPAmount::from(drops.to_string()));
            }
        }
        xrpl::signing::sign(tx, &account.wallet, false)
            .map_err(|e| LedgerError::Build(format!("{e:?}")))?;
        let blob = encode(tx).map_err(|e| LedgerError::Build(format!("{e:?}")))?;
        let hash = transaction_hash(&decode_hex(&blob).map_err(LedgerError::Build)?);

        let submitted = self.call("submit", json!({ "tx_blob": blob }))?;
        let preliminary = submitted["engine_result"]
            .as_str()
            .unwrap_or_default()
            .to_string();

        for _ in 0..self.max_polls {
            match self.call("tx", json!({ "transaction": hash })) {
                Ok(found) if found["validated"] == true => {
                    let result = found["meta"]["TransactionResult"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string();
                    return Ok(Receipt {
                        hash,
                        result,
                        validated: true,
                        ledger_index: found["ledger_index"].as_u64(),
                        preliminary,
                        sequence,
                    });
                }
                Ok(_) => {}
                Err(LedgerError::Rpc { error, .. }) if error == "txnNotFound" => {}
                Err(e) => return Err(e),
            }
            let now = self.call("ledger_current", json!({}))?["ledger_current_index"]
                .as_u64()
                .unwrap_or(0);
            if now > u64::from(last) + 1 {
                break;
            }
            std::thread::sleep(self.poll);
        }
        Ok(Receipt {
            hash,
            result: preliminary.clone(),
            validated: false,
            ledger_index: None,
            preliminary,
            sequence,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use xrpl::models::transactions::account_set::{AccountSet, AccountSetFlag};

    /// A fixed ed25519 seed, for offline tests only.
    fn test_seed() -> String {
        // Generated once by xrpl-rust's own seed generator.
        Wallet::create(Some(xrpl::constants::CryptoAlgorithm::ED25519))
            .map(|w| w.seed.clone())
            .unwrap_or_default()
    }

    /// A scripted transport: each method gets its next queued reply.
    struct Fake {
        replies: RefCell<Vec<(String, VecDeque<Json>)>>,
        sent: RefCell<Vec<Json>>,
    }

    impl Fake {
        fn new() -> Self {
            Fake {
                replies: RefCell::new(Vec::new()),
                sent: RefCell::new(Vec::new()),
            }
        }

        fn on(self, key: &str, reply: Json) -> Self {
            {
                let mut all = self.replies.borrow_mut();
                match all.iter_mut().find(|(k, _)| k == key) {
                    Some((_, q)) => q.push_back(reply),
                    None => all.push((key.to_string(), VecDeque::from([reply]))),
                }
            }
            self
        }
    }

    impl Transport for Fake {
        fn post(&self, url: &str, body: &Json) -> Result<Json, String> {
            self.sent.borrow_mut().push(body.clone());
            let key = body["method"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| url.to_string());
            let mut all = self.replies.borrow_mut();
            let queue = all
                .iter_mut()
                .find(|(k, _)| *k == key)
                .map(|(_, q)| q)
                .ok_or_else(|| format!("no reply scripted for {key}"))?;
            if queue.len() > 1 {
                queue.pop_front().ok_or_else(|| "empty".to_string())
            } else {
                queue.front().cloned().ok_or_else(|| "empty".to_string())
            }
        }
    }

    fn ok(result: Json) -> Json {
        json!({ "result": result })
    }

    fn testnet_info() -> Json {
        ok(json!({ "info": { "network_id": 1 } }))
    }

    fn connect(fake: Fake) -> Result<Testnet<Fake>, LedgerError> {
        Ok(Testnet::with_transport(fake, "rpc", "faucet")?.with_polling(Duration::ZERO, 5))
    }

    fn account_set(account: &Account) -> AccountSet<'static> {
        AccountSet::new(
            account.address().to_string().into(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(AccountSetFlag::AsfDefaultRipple),
            None,
            None,
            None,
        )
    }

    #[test]
    fn a_server_that_is_not_the_testnet_is_refused_before_anything_else() {
        // testnet-check: allow (a refused network id, on purpose)
        let fake = Fake::new().on("server_info", ok(json!({ "info": { "network_id": 0 } }))); // testnet-check: allow
        assert!(matches!(
            Testnet::with_transport(fake, "rpc", "faucet"),
            Err(LedgerError::NotTestnet(_))
        ));
        let fake = Fake::new().on("server_info", ok(json!({ "info": {} })));
        assert!(matches!(
            Testnet::with_transport(fake, "rpc", "faucet"),
            Err(LedgerError::NotTestnet(NotTestnet { reported: None }))
        ));
    }

    #[test]
    fn rpc_errors_and_bad_replies_are_reported() -> Result<(), LedgerError> {
        let fake = Fake::new()
            .on("server_info", testnet_info())
            .on("ledger_current", ok(json!({ "error": "noNetwork" })))
            .on("fee", json!({ "nothing": true }));
        let net = connect(fake)?;
        assert_eq!(
            net.call("ledger_current", json!({})),
            Err(LedgerError::Rpc {
                method: "ledger_current".into(),
                error: "noNetwork".into()
            })
        );
        assert!(matches!(
            net.call("fee", json!({})),
            Err(LedgerError::Reply(_))
        ));
        assert!(matches!(
            net.call("unscripted", json!({})),
            Err(LedgerError::Transport(_))
        ));
        Ok(())
    }

    fn submit_script(tx_reply: Json) -> Fake {
        Fake::new()
            .on("server_info", testnet_info())
            .on(
                "account_info",
                ok(json!({ "account_data": { "Sequence": 7 } })),
            )
            .on("ledger_current", ok(json!({ "ledger_current_index": 100 })))
            .on("fee", ok(json!({ "drops": { "open_ledger_fee": "10" } })))
            .on("submit", ok(json!({ "engine_result": "tesSUCCESS" })))
            .on("tx", tx_reply)
    }

    #[test]
    fn a_submit_signs_hashes_locally_and_returns_the_validated_result() -> Result<(), LedgerError> {
        let account = Account::from_seed(&test_seed())?;
        let net = connect(submit_script(ok(json!({
            "validated": true,
            "ledger_index": 101,
            "meta": { "TransactionResult": "tesSUCCESS" }
        }))))?;
        let receipt = net.submit(&account, &mut account_set(&account))?;
        assert!(receipt.validated);
        assert_eq!(receipt.result, "tesSUCCESS");
        assert_eq!(receipt.ledger_index, Some(101));
        assert_eq!(receipt.sequence, 7);
        let sent = net.transport.sent.borrow();
        let blob = sent
            .iter()
            .find(|b| b["method"] == "submit")
            .and_then(|b| b["params"][0]["tx_blob"].as_str().map(str::to_string))
            .unwrap_or_default();
        // The hash looked up is the hash of the blob that was submitted.
        let expected = transaction_hash(&decode_hex(&blob).map_err(LedgerError::Build)?);
        assert_eq!(receipt.hash, expected);
        let looked_up = sent
            .iter()
            .find(|b| b["method"] == "tx")
            .map(|b| b["params"][0]["transaction"].clone());
        assert_eq!(looked_up, Some(json!(expected)));
        // Fee 12 drops (never below the minimum), sequence 7, expiry 120.
        assert!(blob.contains("68400000000000000C"), "{blob}");
        assert!(blob.contains("2400000007"), "{blob}");
        assert!(blob.contains("201B00000078"), "{blob}");
        Ok(())
    }

    #[test]
    fn a_failed_result_keeps_its_hash() -> Result<(), LedgerError> {
        let account = Account::from_seed(&test_seed())?;
        let net = connect(submit_script(ok(json!({
            "validated": true,
            "ledger_index": 102,
            "meta": { "TransactionResult": "tecNO_PERMISSION" }
        }))))?;
        let receipt = net.submit(&account, &mut account_set(&account))?;
        assert_eq!(receipt.result, "tecNO_PERMISSION");
        assert!(receipt.validated);
        assert_eq!(receipt.hash.len(), 64);
        Ok(())
    }

    #[test]
    fn a_set_fee_is_kept_and_a_transaction_that_never_validates_says_so() -> Result<(), LedgerError>
    {
        let account = Account::from_seed(&test_seed())?;
        let fake = submit_script(ok(json!({ "error": "txnNotFound" })))
            .on("ledger_current", ok(json!({ "ledger_current_index": 100 })))
            .on("ledger_current", ok(json!({ "ledger_current_index": 200 })));
        let net = connect(fake)?;
        let mut tx = account_set(&account);
        tx.common_fields.fee = Some(XRPAmount::from("15"));
        let receipt = net.submit(&account, &mut tx)?;
        assert!(!receipt.validated);
        assert_eq!(receipt.ledger_index, None);
        let sent = net.transport.sent.borrow();
        assert!(!sent.iter().any(|b| b["method"] == "fee"));
        Ok(())
    }

    #[test]
    fn polling_gives_up_and_other_lookup_errors_are_returned() -> Result<(), LedgerError> {
        let account = Account::from_seed(&test_seed())?;
        let net = connect(submit_script(ok(json!({ "validated": false }))))?;
        assert!(!net.submit(&account, &mut account_set(&account))?.validated);
        let net = connect(submit_script(ok(json!({ "error": "tooBusy" }))))?;
        assert!(matches!(
            net.submit(&account, &mut account_set(&account)),
            Err(LedgerError::Rpc { .. })
        ));
        Ok(())
    }

    #[test]
    fn a_submit_without_a_sequence_or_ledger_is_refused() -> Result<(), LedgerError> {
        let account = Account::from_seed(&test_seed())?;
        let fake = Fake::new()
            .on("server_info", testnet_info())
            .on("account_info", ok(json!({ "account_data": {} })));
        assert!(matches!(
            connect(fake)?.submit(&account, &mut account_set(&account)),
            Err(LedgerError::Reply(_))
        ));
        let fake = Fake::new()
            .on("server_info", testnet_info())
            .on(
                "account_info",
                ok(json!({ "account_data": { "Sequence": 1 } })),
            )
            .on("ledger_current", ok(json!({})));
        assert!(matches!(
            connect(fake)?.submit(&account, &mut account_set(&account)),
            Err(LedgerError::Reply(_))
        ));
        Ok(())
    }

    #[test]
    fn the_faucet_seed_must_be_ed25519_and_derive_to_the_reported_address()
    -> Result<(), LedgerError> {
        let seed = test_seed();
        let address = Account::from_seed(&seed)?.address().to_string();
        let fake = Fake::new().on("server_info", testnet_info()).on(
            "faucet",
            json!({ "seed": seed, "account": { "classicAddress": address } }),
        );
        let funded = connect(fake)?.fund_new_account()?;
        assert_eq!(funded.account.address(), address);
        assert_eq!(funded.seed.expose(), seed);
        assert!(!format!("{funded:?}").contains(&seed));

        let fake = Fake::new().on("server_info", testnet_info()).on(
            "faucet",
            json!({ "account": { "secret": seed, "address": "rrrrrrrrrrrrrrrrrrrrrhoLvTp" } }),
        );
        assert!(matches!(
            connect(fake)?.fund_new_account(),
            Err(LedgerError::Reply(_))
        ));
        for reply in [json!({}), json!({ "seed": seed })] {
            let fake = Fake::new()
                .on("server_info", testnet_info())
                .on("faucet", reply);
            assert!(matches!(
                connect(fake)?.fund_new_account(),
                Err(LedgerError::Reply(_))
            ));
        }
        Ok(())
    }

    #[test]
    fn secp256k1_and_bad_seeds_are_refused() {
        let secp = Wallet::create(Some(xrpl::constants::CryptoAlgorithm::SECP256K1))
            .map(|w| w.seed.clone())
            .unwrap_or_default();
        assert!(matches!(
            Account::from_seed(&secp),
            Err(LedgerError::NotEd25519)
        ));
        assert!(matches!(
            Account::from_seed("sEdnotarealseed"),
            Err(LedgerError::Build(_))
        ));
    }

    #[test]
    fn an_account_never_prints_its_keys() -> Result<(), LedgerError> {
        let seed = test_seed();
        let account = Account::from_seed(&seed)?;
        let shown = format!("{account:?}");
        assert!(shown.contains(account.address()));
        assert!(!shown.contains(&seed));
        assert!(account.public_key().starts_with("ED"));
        Ok(())
    }

    #[test]
    fn waiting_for_an_account_retries_until_it_appears() -> Result<(), LedgerError> {
        let fake = Fake::new()
            .on("server_info", testnet_info())
            .on("account_info", ok(json!({ "error": "actNotFound" })))
            .on("account_info", ok(json!({ "account_data": {} })));
        assert_eq!(connect(fake)?.wait_for_account("r"), Ok(()));
        let fake = Fake::new()
            .on("server_info", testnet_info())
            .on("account_info", ok(json!({ "error": "actNotFound" })));
        assert!(matches!(
            connect(fake)?.wait_for_account("r"),
            Err(LedgerError::Reply(_))
        ));
        let fake = Fake::new()
            .on("server_info", testnet_info())
            .on("account_info", ok(json!({ "error": "tooBusy" })));
        assert!(matches!(
            connect(fake)?.wait_for_account("r"),
            Err(LedgerError::Rpc { .. })
        ));
        Ok(())
    }

    #[test]
    fn hex_and_errors() {
        assert_eq!(decode_hex("00ff10").ok(), Some(vec![0, 255, 16]));
        assert!(decode_hex("abc").is_err());
        assert!(decode_hex("zz").is_err());
        let all = [
            LedgerError::NotTestnet(NotTestnet { reported: Some(2) }),
            LedgerError::Transport("down".into()),
            LedgerError::Rpc {
                method: "m".into(),
                error: "e".into(),
            },
            LedgerError::Reply("r".into()),
            LedgerError::Build("b".into()),
            LedgerError::NotEd25519,
        ];
        for e in all {
            assert!(!e.to_string().is_empty());
        }
        assert_eq!(format!("{:?}", Seed("s".into())), "Seed(<hidden>)");
    }
}
