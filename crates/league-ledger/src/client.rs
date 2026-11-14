//! A small JSON-RPC client for the XRPL Testnet, and nothing else.
//!
//! [`Testnet::connect`] checks the server's network id before anything is
//! sent ([`require_testnet`](crate::require_testnet)). [`Testnet::submit`]
//! fills in the account's sequence, the fee and an expiry, signs, computes
//! the transaction's hash **locally**, submits, and waits for the result in
//! a validated ledger. The hash is known before submission, so a `tec`
//! result has its hash too.
//!
//! The HTTP layer sits behind [`Transport`], so everything above it can be
//! tested without a network.

use crate::codec::{Amount, CodecError, Transaction, Value, encode_hex, transaction_hash};
use crate::keys::{KeyError, Keypair};
use crate::{NotTestnet, TESTNET_FAUCET, TESTNET_JSON_RPC, require_testnet};
use serde_json::{Value as Json, json};
use std::fmt;
use std::time::Duration;

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
    /// The transaction could not be encoded.
    Codec(CodecError),
    /// A key or address problem.
    Key(KeyError),
}

impl fmt::Display for LedgerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LedgerError::NotTestnet(e) => write!(f, "{e}"),
            LedgerError::Transport(e) => write!(f, "could not reach the Testnet: {e}"),
            LedgerError::Rpc { method, error } => write!(f, "{method} failed: {error}"),
            LedgerError::Reply(e) => write!(f, "unexpected reply: {e}"),
            LedgerError::Codec(e) => write!(f, "{e}"),
            LedgerError::Key(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for LedgerError {}

impl From<CodecError> for LedgerError {
    fn from(e: CodecError) -> Self {
        LedgerError::Codec(e)
    }
}

impl From<KeyError> for LedgerError {
    fn from(e: KeyError) -> Self {
        LedgerError::Key(e)
    }
}

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

/// A newly funded Testnet account.
#[derive(Debug)]
pub struct Funded {
    /// The account's keys.
    pub keys: Keypair,
    /// The account's seed, as the faucet issued it.
    pub seed: Seed,
    /// The address, as the faucet reported it and as we derived it.
    pub address: String,
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

    /// Fund a new account from the Testnet faucet: one call. The seed it
    /// returns is checked to derive to the address it reports.
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
        let keys = Keypair::from_seed(seed)?;
        if keys.address() != reported {
            return Err(LedgerError::Reply(format!(
                "faucet: the seed derives to {}, not the reported {reported}",
                keys.address()
            )));
        }
        Ok(Funded {
            address: keys.address(),
            keys,
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

    /// Fill in, sign, hash, submit and wait for `tx` from `keys`'s account.
    ///
    /// `Account`, `SigningPubKey`, `Sequence`, `LastLedgerSequence` and (if
    /// not set) `Fee` are filled in here. A result that never validates
    /// comes back with `validated: false`.
    pub fn submit(&self, keys: &Keypair, mut tx: Transaction) -> Result<Receipt, LedgerError> {
        let address = keys.address();
        let account = self.call(
            "account_info",
            json!({ "account": address, "ledger_index": "current" }),
        )?;
        let sequence = account["account_data"]["Sequence"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| LedgerError::Reply("account_info: no Sequence".into()))?;
        let current = self.call("ledger_current", json!({}))?["ledger_current_index"]
            .as_u64()
            .ok_or_else(|| LedgerError::Reply("ledger_current: no index".into()))?;
        let last = u32::try_from(current + EXPIRY_LEDGERS)
            .map_err(|_| LedgerError::Reply("ledger index too large".into()))?;
        if tx.get("Fee").is_none() {
            let fee = self.call("fee", json!({}))?;
            let open = fee["drops"]["open_ledger_fee"]
                .as_str()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(MIN_FEE_DROPS);
            tx.set("Fee", Value::Amount(Amount::Drops(open.max(MIN_FEE_DROPS))))?;
        }
        tx.set("Account", Value::Account(keys.account_id()))?;
        tx.set("SigningPubKey", Value::Blob(keys.public_key().to_vec()))?;
        tx.set("Sequence", Value::U32(sequence))?;
        tx.set("LastLedgerSequence", Value::U32(last))?;
        let signature = keys.sign(&tx.signing_data()?);
        tx.set("TxnSignature", Value::Blob(signature.to_vec()))?;
        let blob = tx.serialize()?;
        let hash = encode_hex(&transaction_hash(&blob));

        let submitted = self.call("submit", json!({ "tx_blob": encode_hex(&blob) }))?;
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
    use crate::codec::Value;
    use crate::keys::encode_seed;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    /// A scripted transport: each URL+method gets its next queued reply.
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
            // The last reply repeats once the queue is down to one.
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

    #[test]
    fn a_server_that_is_not_the_testnet_is_refused_before_anything_else() {
        let fake = Fake::new().on("server_info", ok(json!({ "info": { "network_id": 0 } })));
        let refused = Testnet::with_transport(fake, "rpc", "faucet");
        assert!(matches!(refused, Err(LedgerError::NotTestnet(_))));
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
    fn a_submit_fills_in_signs_and_returns_the_validated_result() -> Result<(), LedgerError> {
        let keys = Keypair::from_entropy(&[5u8; 16]);
        let net = connect(submit_script(ok(json!({
            "validated": true,
            "ledger_index": 101,
            "meta": { "TransactionResult": "tesSUCCESS" }
        }))))?;
        let mut tx = Transaction::new("AccountSet")?;
        tx.set("SetFlag", Value::U32(8))?;
        let receipt = net.submit(&keys, tx)?;
        assert!(receipt.validated);
        assert_eq!(receipt.result, "tesSUCCESS");
        assert_eq!(receipt.ledger_index, Some(101));
        assert_eq!(receipt.sequence, 7);
        assert_eq!(receipt.hash.len(), 64);
        // The hash submitted and the hash looked up are the same one.
        let sent = net.transport.sent.borrow();
        let looked_up = sent
            .iter()
            .find(|b| b["method"] == "tx")
            .map(|b| b["params"][0]["transaction"].clone());
        assert_eq!(looked_up, Some(json!(receipt.hash)));
        // The fee is never below the minimum.
        let blob = sent
            .iter()
            .find(|b| b["method"] == "submit")
            .and_then(|b| b["params"][0]["tx_blob"].as_str().map(str::to_string))
            .unwrap_or_default();
        assert!(
            blob.contains("68400000000000000C"),
            "fee 12 drops in {blob}"
        );
        Ok(())
    }

    #[test]
    fn a_failed_result_keeps_its_hash() -> Result<(), LedgerError> {
        let keys = Keypair::from_entropy(&[6u8; 16]);
        let net = connect(submit_script(ok(json!({
            "validated": true,
            "ledger_index": 102,
            "meta": { "TransactionResult": "tecCRYPTOCONDITION_ERROR" }
        }))))?;
        let receipt = net.submit(&keys, Transaction::new("EscrowFinish")?)?;
        assert_eq!(receipt.result, "tecCRYPTOCONDITION_ERROR");
        assert!(receipt.validated);
        assert_eq!(receipt.hash.len(), 64);
        Ok(())
    }

    #[test]
    fn a_transaction_that_never_validates_says_so() -> Result<(), LedgerError> {
        let keys = Keypair::from_entropy(&[7u8; 16]);
        let fake = submit_script(ok(json!({ "error": "txnNotFound" })))
            .on("ledger_current", ok(json!({ "ledger_current_index": 100 })))
            .on("ledger_current", ok(json!({ "ledger_current_index": 200 })));
        let net = connect(fake)?;
        let mut tx = Transaction::new("AccountSet")?;
        tx.set("Fee", Value::Amount(Amount::Drops(15)))?;
        let receipt = net.submit(&keys, tx)?;
        assert!(!receipt.validated);
        assert_eq!(receipt.result, "tesSUCCESS");
        assert_eq!(receipt.ledger_index, None);
        Ok(())
    }

    #[test]
    fn polling_gives_up_after_its_limit() -> Result<(), LedgerError> {
        let keys = Keypair::from_entropy(&[8u8; 16]);
        let net = connect(submit_script(ok(json!({ "validated": false }))))?;
        let receipt = net.submit(&keys, Transaction::new("AccountSet")?)?;
        assert!(!receipt.validated);
        Ok(())
    }

    #[test]
    fn a_lookup_error_other_than_not_found_is_returned() -> Result<(), LedgerError> {
        let keys = Keypair::from_entropy(&[9u8; 16]);
        let net = connect(submit_script(ok(json!({ "error": "tooBusy" }))))?;
        assert!(matches!(
            net.submit(&keys, Transaction::new("AccountSet")?),
            Err(LedgerError::Rpc { .. })
        ));
        Ok(())
    }

    #[test]
    fn a_submit_without_a_sequence_or_ledger_is_refused() -> Result<(), LedgerError> {
        let keys = Keypair::from_entropy(&[10u8; 16]);
        let fake = Fake::new()
            .on("server_info", testnet_info())
            .on("account_info", ok(json!({ "account_data": {} })));
        let net = connect(fake)?;
        assert!(matches!(
            net.submit(&keys, Transaction::new("AccountSet")?),
            Err(LedgerError::Reply(_))
        ));
        let fake = Fake::new()
            .on("server_info", testnet_info())
            .on(
                "account_info",
                ok(json!({ "account_data": { "Sequence": 1 } })),
            )
            .on("ledger_current", ok(json!({})));
        let net = connect(fake)?;
        assert!(matches!(
            net.submit(&keys, Transaction::new("AccountSet")?),
            Err(LedgerError::Reply(_))
        ));
        Ok(())
    }

    #[test]
    fn the_faucet_seed_must_derive_to_the_reported_address() -> Result<(), LedgerError> {
        let seed = encode_seed(&[3u8; 16]);
        let address = Keypair::from_seed(&seed)?.address();
        let fake = Fake::new().on("server_info", testnet_info()).on(
            "faucet",
            json!({ "seed": seed, "account": { "classicAddress": address } }),
        );
        let funded = connect(fake)?.fund_new_account()?;
        assert_eq!(funded.address, address);
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
    fn errors_say_what_is_wrong() {
        let all = [
            LedgerError::NotTestnet(NotTestnet { reported: Some(0) }),
            LedgerError::Transport("down".into()),
            LedgerError::Rpc {
                method: "m".into(),
                error: "e".into(),
            },
            LedgerError::Reply("r".into()),
            LedgerError::Codec(CodecError::BadAmount("x".into())),
            LedgerError::Key(KeyError::NotBase58),
        ];
        for e in all {
            assert!(!e.to_string().is_empty());
        }
        assert_eq!(format!("{:?}", Seed("s".into())), "Seed(<hidden>)");
    }
}
