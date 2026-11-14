//! XRPL binary serialization for the transactions Merchant League uses.
//!
//! Exactly the ten transaction types and the fields they need, in XRPL's
//! canonical order (by type code, then field number). Checked byte for byte
//! against transactions captured on the live Testnet (`tests/vectors/`), and
//! against the official codec fixtures once their licence is confirmed.
//!
//! Signing data is `STX\0` followed by every signing field; a transaction's
//! hash is the SHA-512Half of `TXN\0` followed by the signed blob.

use crate::keys::{KeyError, decode_address, encode_address};
use sha2::{Digest, Sha512};
use std::fmt;

/// XRPL serialized types used here, with their type codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FieldType {
    /// 16-bit unsigned integer.
    UInt16 = 1,
    /// 32-bit unsigned integer.
    UInt32 = 2,
    /// 256-bit hash.
    Hash256 = 5,
    /// XRP drops or an issued-token amount.
    Amount = 6,
    /// Variable-length bytes.
    Blob = 7,
    /// A 20-byte account ID, length-prefixed.
    AccountId = 8,
}

/// One field: its name, type and number within the type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Field {
    /// The field's name, as in transaction JSON.
    pub name: &'static str,
    /// The field's serialized type.
    pub kind: FieldType,
    /// The field's number within its type.
    pub nth: u8,
}

impl Field {
    fn order(&self) -> (u8, u8) {
        (self.kind as u8, self.nth)
    }

    fn signing(&self) -> bool {
        self.name != "TxnSignature"
    }
}

macro_rules! field {
    ($name:literal, $kind:ident, $nth:literal) => {
        Field {
            name: $name,
            kind: FieldType::$kind,
            nth: $nth,
        }
    };
}

/// Every field this codec handles.
pub const FIELDS: &[Field] = &[
    field!("TransactionType", UInt16, 2),
    field!("Flags", UInt32, 2),
    field!("Sequence", UInt32, 4),
    field!("Expiration", UInt32, 10),
    field!("OfferSequence", UInt32, 25),
    field!("LastLedgerSequence", UInt32, 27),
    field!("SetFlag", UInt32, 33),
    field!("ClearFlag", UInt32, 34),
    field!("CancelAfter", UInt32, 36),
    field!("FinishAfter", UInt32, 37),
    field!("CheckID", Hash256, 24),
    field!("Amount", Amount, 1),
    field!("LimitAmount", Amount, 3),
    field!("TakerPays", Amount, 4),
    field!("TakerGets", Amount, 5),
    field!("Fee", Amount, 8),
    field!("SendMax", Amount, 9),
    field!("DeliverMin", Amount, 10),
    field!("SigningPubKey", Blob, 3),
    field!("TxnSignature", Blob, 4),
    field!("Fulfillment", Blob, 16),
    field!("Condition", Blob, 17),
    field!("Account", AccountId, 1),
    field!("Owner", AccountId, 2),
    field!("Destination", AccountId, 3),
];

/// The field with this name, if the codec handles it.
pub fn field(name: &str) -> Option<Field> {
    FIELDS.iter().copied().find(|f| f.name == name)
}

/// The transaction types this codec handles, with their codes.
pub const TRANSACTION_TYPES: &[(&str, u16)] = &[
    ("Payment", 0),
    ("EscrowCreate", 1),
    ("EscrowFinish", 2),
    ("AccountSet", 3),
    ("OfferCreate", 7),
    ("OfferCancel", 8),
    ("CheckCreate", 16),
    ("CheckCash", 17),
    ("TrustSet", 20),
    ("Clawback", 30),
];

/// The code for a transaction type name.
pub fn transaction_type_code(name: &str) -> Option<u16> {
    TRANSACTION_TYPES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, c)| *c)
}

/// Something the codec cannot encode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodecError {
    /// A field this codec does not handle.
    UnknownField(String),
    /// A transaction type this codec does not handle.
    UnknownTransactionType(String),
    /// A field's value has the wrong shape for its type.
    BadValue(&'static str, String),
    /// An issued amount that cannot be represented exactly.
    BadAmount(String),
    /// An address or other key material that does not decode.
    Key(KeyError),
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CodecError::UnknownField(n) => write!(f, "field {n} is not handled by this codec"),
            CodecError::UnknownTransactionType(n) => {
                write!(f, "transaction type {n} is not handled by this codec")
            }
            CodecError::BadValue(field, why) => write!(f, "{field}: {why}"),
            CodecError::BadAmount(why) => write!(f, "amount: {why}"),
            CodecError::Key(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for CodecError {}

impl From<KeyError> for CodecError {
    fn from(e: KeyError) -> Self {
        CodecError::Key(e)
    }
}

/// A three-letter currency code, or a 160-bit non-standard code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Currency(pub [u8; 20]);

impl Currency {
    /// A currency from its code: three ASCII characters (not "XRP"), or 40
    /// hex digits.
    pub fn parse(code: &str) -> Result<Self, CodecError> {
        let mut bytes = [0u8; 20];
        if code.len() == 3 && code.is_ascii() && code != "XRP" {
            bytes[12..15].copy_from_slice(code.as_bytes());
            return Ok(Currency(bytes));
        }
        if code.len() == 40 {
            let raw =
                decode_hex(code).map_err(|_| CodecError::BadAmount(format!("currency {code}")))?;
            bytes.copy_from_slice(&raw);
            return Ok(Currency(bytes));
        }
        Err(CodecError::BadAmount(format!("currency {code}")))
    }
}

/// XRP in drops, or an amount of an issued token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Amount {
    /// Drops of XRP (one millionth of an XRP).
    Drops(u64),
    /// An issued token: a decimal value, its currency and its issuer.
    Issued {
        /// The value, as a decimal string ("100", "0.5", "1e3").
        value: String,
        /// The currency.
        currency: Currency,
        /// The issuer's account ID.
        issuer: [u8; 20],
    },
}

/// One field's value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// A 16-bit integer.
    U16(u16),
    /// A 32-bit integer.
    U32(u32),
    /// A 256-bit hash.
    Hash256([u8; 32]),
    /// An amount.
    Amount(Amount),
    /// Variable-length bytes.
    Blob(Vec<u8>),
    /// An account ID.
    Account([u8; 20]),
}

/// A transaction as a set of fields, ready to serialize.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Transaction {
    fields: Vec<(Field, Value)>,
}

impl Transaction {
    /// An empty transaction of the given type.
    pub fn new(transaction_type: &str) -> Result<Self, CodecError> {
        let code = transaction_type_code(transaction_type)
            .ok_or_else(|| CodecError::UnknownTransactionType(transaction_type.to_string()))?;
        let mut tx = Transaction::default();
        tx.set("TransactionType", Value::U16(code))?;
        Ok(tx)
    }

    /// Set a field, replacing any earlier value. The value must match the
    /// field's type.
    pub fn set(&mut self, name: &str, value: Value) -> Result<&mut Self, CodecError> {
        let f = field(name).ok_or_else(|| CodecError::UnknownField(name.to_string()))?;
        let fits = matches!(
            (f.kind, &value),
            (FieldType::UInt16, Value::U16(_))
                | (FieldType::UInt32, Value::U32(_))
                | (FieldType::Hash256, Value::Hash256(_))
                | (FieldType::Amount, Value::Amount(_))
                | (FieldType::Blob, Value::Blob(_))
                | (FieldType::AccountId, Value::Account(_))
        );
        if !fits {
            return Err(CodecError::BadValue(
                f.name,
                "wrong type for this field".into(),
            ));
        }
        self.fields.retain(|(g, _)| g.name != name);
        self.fields.push((f, value));
        Ok(self)
    }

    /// A field's value, if set.
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.fields
            .iter()
            .find(|(f, _)| f.name == name)
            .map(|(_, v)| v)
    }

    /// The serialized transaction, every field in canonical order.
    pub fn serialize(&self) -> Result<Vec<u8>, CodecError> {
        self.encode(false)
    }

    /// The data to sign: `STX\0` and every field except the signature.
    pub fn signing_data(&self) -> Result<Vec<u8>, CodecError> {
        let mut out = vec![0x53, 0x54, 0x58, 0x00];
        out.extend(self.encode(true)?);
        Ok(out)
    }

    fn encode(&self, signing_only: bool) -> Result<Vec<u8>, CodecError> {
        let mut sorted: Vec<&(Field, Value)> = self
            .fields
            .iter()
            .filter(|(f, _)| !signing_only || f.signing())
            .collect();
        sorted.sort_by_key(|(f, _)| f.order());
        let mut out = Vec::new();
        for (f, v) in sorted {
            write_field_id(&mut out, f.kind as u8, f.nth);
            write_value(&mut out, f, v)?;
        }
        Ok(out)
    }
}

/// The transaction hash of a signed blob: SHA-512Half of `TXN\0` + blob.
pub fn transaction_hash(signed_blob: &[u8]) -> [u8; 32] {
    let mut data = vec![0x54, 0x58, 0x4E, 0x00];
    data.extend_from_slice(signed_blob);
    sha512_half(&data)
}

/// The first 32 bytes of SHA-512.
pub fn sha512_half(data: &[u8]) -> [u8; 32] {
    let digest = Sha512::digest(data);
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest[..32]);
    out
}

fn write_field_id(out: &mut Vec<u8>, kind: u8, nth: u8) {
    match (kind < 16, nth < 16) {
        (true, true) => out.push((kind << 4) | nth),
        (true, false) => {
            out.push(kind << 4);
            out.push(nth);
        }
        (false, true) => {
            out.push(nth);
            out.push(kind);
        }
        (false, false) => {
            out.push(0);
            out.push(kind);
            out.push(nth);
        }
    }
}

fn write_length(out: &mut Vec<u8>, len: usize, field: &'static str) -> Result<(), CodecError> {
    if len <= 192 {
        out.push(len as u8);
    } else if len <= 12_480 {
        let rest = len - 193;
        out.push(193 + (rest >> 8) as u8);
        out.push((rest & 0xFF) as u8);
    } else if len <= 918_744 {
        let rest = len - 12_481;
        out.push(241 + (rest >> 16) as u8);
        out.push(((rest >> 8) & 0xFF) as u8);
        out.push((rest & 0xFF) as u8);
    } else {
        return Err(CodecError::BadValue(
            field,
            format!("{len} bytes is too long"),
        ));
    }
    Ok(())
}

fn write_value(out: &mut Vec<u8>, f: &Field, v: &Value) -> Result<(), CodecError> {
    match v {
        Value::U16(n) => out.extend_from_slice(&n.to_be_bytes()),
        Value::U32(n) => out.extend_from_slice(&n.to_be_bytes()),
        Value::Hash256(h) => out.extend_from_slice(h),
        Value::Amount(a) => write_amount(out, a)?,
        Value::Blob(b) => {
            write_length(out, b.len(), f.name)?;
            out.extend_from_slice(b);
        }
        Value::Account(id) => {
            out.push(20);
            out.extend_from_slice(id);
        }
    }
    Ok(())
}

/// Largest number of drops the ledger allows (100 billion XRP).
const MAX_DROPS: u64 = 100_000_000_000_000_000;

fn write_amount(out: &mut Vec<u8>, amount: &Amount) -> Result<(), CodecError> {
    match amount {
        Amount::Drops(drops) => {
            if *drops > MAX_DROPS {
                return Err(CodecError::BadAmount(format!("{drops} drops is too many")));
            }
            out.extend_from_slice(&(drops | 0x4000_0000_0000_0000).to_be_bytes());
        }
        Amount::Issued {
            value,
            currency,
            issuer,
        } => {
            out.extend_from_slice(&issued_value_bits(value)?.to_be_bytes());
            out.extend_from_slice(&currency.0);
            out.extend_from_slice(issuer);
        }
    }
    Ok(())
}

const MIN_MANTISSA: u128 = 1_000_000_000_000_000;
const MAX_MANTISSA: u128 = 9_999_999_999_999_999;
const MIN_EXPONENT: i32 = -96;
const MAX_EXPONENT: i32 = 80;

/// The 64 bits of an issued value: not-XRP bit, sign bit, exponent + 97 in
/// eight bits, and a 54-bit mantissa in [10^15, 10^16). Values that would
/// need more than 16 significant digits are refused, not rounded.
pub fn issued_value_bits(value: &str) -> Result<u64, CodecError> {
    let bad = |why: &str| CodecError::BadAmount(format!("{value}: {why}"));
    let text = value.trim();
    let (negative, text) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let (number, exp_part) = match text.find(['e', 'E']) {
        Some(i) => (&text[..i], Some(&text[i + 1..])),
        None => (text, None),
    };
    let mut exponent: i32 = match exp_part {
        Some(e) => e.parse().map_err(|_| bad("bad exponent"))?,
        None => 0,
    };
    let (int_part, frac_part) = match number.split_once('.') {
        Some((i, f)) => (i, f),
        None => (number, ""),
    };
    if int_part.is_empty() && frac_part.is_empty() {
        return Err(bad("no digits"));
    }
    if !int_part
        .chars()
        .chain(frac_part.chars())
        .all(|c| c.is_ascii_digit())
    {
        return Err(bad("not a decimal number"));
    }
    let digits: String = format!("{int_part}{frac_part}");
    exponent -= i32::try_from(frac_part.len()).map_err(|_| bad("too many digits"))?;
    let digits = digits.trim_start_matches('0');
    if digits.is_empty() {
        return Ok(0x8000_0000_0000_0000);
    }
    let trimmed = digits.trim_end_matches('0');
    exponent += i32::try_from(digits.len() - trimmed.len()).map_err(|_| bad("too many digits"))?;
    if trimmed.len() > 16 {
        return Err(bad("more than 16 significant digits"));
    }
    let mut mantissa: u128 = trimmed.parse().map_err(|_| bad("not a decimal number"))?;
    while mantissa < MIN_MANTISSA {
        mantissa *= 10;
        exponent -= 1;
    }
    if mantissa > MAX_MANTISSA {
        return Err(bad("more than 16 significant digits"));
    }
    if !(MIN_EXPONENT..=MAX_EXPONENT).contains(&exponent) {
        return Err(bad("out of range"));
    }
    let mut bits: u64 = 0x8000_0000_0000_0000;
    if !negative {
        bits |= 0x4000_0000_0000_0000;
    }
    bits |= ((exponent + 97) as u64) << 54;
    bits |= mantissa as u64;
    Ok(bits)
}

/// Decode uppercase or lowercase hex.
pub fn decode_hex(text: &str) -> Result<Vec<u8>, String> {
    if !text.len().is_multiple_of(2) {
        return Err("odd length".into());
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).map_err(|_| format!("bad hex at {i}")))
        .collect()
}

/// Uppercase hex.
pub fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

/// Build a transaction from its JSON form, as the ledger returns it in
/// `tx_json`. Fields this codec does not handle are refused, except
/// `DeliverMax`, which the API adds to a Payment as a copy of `Amount`.
pub fn from_json(tx: &serde_json::Value) -> Result<Transaction, CodecError> {
    let obj = tx
        .as_object()
        .ok_or_else(|| CodecError::BadValue("transaction", "not a JSON object".into()))?;
    let type_name = obj
        .get("TransactionType")
        .and_then(|v| v.as_str())
        .ok_or_else(|| CodecError::BadValue("TransactionType", "missing".into()))?;
    let mut out = Transaction::new(type_name)?;
    for (name, value) in obj {
        if name == "TransactionType"
            || name == "DeliverMax"
            || name.starts_with(|c: char| c.is_ascii_lowercase())
        {
            continue;
        }
        let f = field(name).ok_or_else(|| CodecError::UnknownField(name.clone()))?;
        let parsed = match f.kind {
            FieldType::UInt16 => Value::U16(json_uint(f.name, value)? as u16),
            FieldType::UInt32 => Value::U32(
                u32::try_from(json_uint(f.name, value)?)
                    .map_err(|_| CodecError::BadValue(f.name, "too large".into()))?,
            ),
            FieldType::Hash256 => {
                let raw = json_hex(f.name, value)?;
                let arr: [u8; 32] = raw
                    .try_into()
                    .map_err(|_| CodecError::BadValue(f.name, "not 32 bytes".into()))?;
                Value::Hash256(arr)
            }
            FieldType::Amount => Value::Amount(json_amount(value)?),
            FieldType::Blob => Value::Blob(json_hex(f.name, value)?),
            FieldType::AccountId => Value::Account(decode_address(
                value
                    .as_str()
                    .ok_or_else(|| CodecError::BadValue(f.name, "not a string".into()))?,
            )?),
        };
        out.set(f.name, parsed)?;
    }
    Ok(out)
}

fn json_uint(field: &'static str, v: &serde_json::Value) -> Result<u64, CodecError> {
    v.as_u64()
        .ok_or_else(|| CodecError::BadValue(field, "not an unsigned integer".into()))
}

fn json_hex(field: &'static str, v: &serde_json::Value) -> Result<Vec<u8>, CodecError> {
    let s = v
        .as_str()
        .ok_or_else(|| CodecError::BadValue(field, "not a hex string".into()))?;
    decode_hex(s).map_err(|e| CodecError::BadValue(field, e))
}

fn json_amount(v: &serde_json::Value) -> Result<Amount, CodecError> {
    if let Some(drops) = v.as_str() {
        return drops
            .parse::<u64>()
            .map(Amount::Drops)
            .map_err(|_| CodecError::BadAmount(format!("{drops} drops")));
    }
    let get = |k: &str| {
        v.get(k)
            .and_then(|x| x.as_str())
            .ok_or_else(|| CodecError::BadAmount(format!("missing {k}")))
    };
    Ok(Amount::Issued {
        value: get("value")?.to_string(),
        currency: Currency::parse(get("currency")?)?,
        issuer: decode_address(get("issuer")?)?,
    })
}

/// The classic address for an account-ID value, for display.
pub fn account_text(id: &[u8; 20]) -> String {
    encode_address(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_ids_use_the_short_and_long_forms() {
        let mut out = Vec::new();
        write_field_id(&mut out, 1, 2);
        write_field_id(&mut out, 2, 27);
        write_field_id(&mut out, 16, 1);
        write_field_id(&mut out, 16, 17);
        assert_eq!(out, vec![0x12, 0x20, 27, 0x01, 16, 0, 16, 17]);
    }

    #[test]
    fn lengths_use_one_two_or_three_bytes() -> Result<(), CodecError> {
        let mut out = Vec::new();
        write_length(&mut out, 192, "x")?;
        write_length(&mut out, 193, "x")?;
        write_length(&mut out, 12_480, "x")?;
        write_length(&mut out, 12_481, "x")?;
        assert_eq!(out, vec![192, 193, 0, 240, 255, 241, 0, 0]);
        assert!(write_length(&mut out, 918_745, "x").is_err());
        Ok(())
    }

    #[test]
    fn issued_values_normalise() -> Result<(), CodecError> {
        // 1 = 10^15 x 10^-15.
        assert_eq!(issued_value_bits("1")?, 0xD4838D7EA4C68000);
        assert_eq!(issued_value_bits("1.0")?, issued_value_bits("1")?);
        assert_eq!(issued_value_bits("10")?, issued_value_bits("1e1")?);
        assert_eq!(issued_value_bits("0.5")?, issued_value_bits("5e-1")?);
        assert_eq!(issued_value_bits("0")?, 0x8000_0000_0000_0000);
        assert_eq!(issued_value_bits("0.000")?, 0x8000_0000_0000_0000);
        let neg = issued_value_bits("-1")?;
        assert_eq!(neg & 0x4000_0000_0000_0000, 0);
        assert_eq!(neg | 0x4000_0000_0000_0000, issued_value_bits("1")?);
        assert_eq!(issued_value_bits("+2")?, issued_value_bits("2")?);
        Ok(())
    }

    #[test]
    fn issued_values_that_cannot_be_exact_are_refused() {
        for bad in [
            "",
            ".",
            "1.2.3",
            "abc",
            "1e",
            "12345678901234567",
            "1e200",
            "1e-200",
        ] {
            assert!(issued_value_bits(bad).is_err(), "{bad}");
        }
        assert!(issued_value_bits("1234567890123456").is_ok());
        assert!(issued_value_bits("1234567890123456000").is_ok());
    }

    #[test]
    fn currencies_parse_and_xrp_is_refused() -> Result<(), CodecError> {
        let c = Currency::parse("MRK")?;
        assert_eq!(&c.0[12..15], b"MRK");
        assert!(c.0[..12].iter().all(|b| *b == 0));
        assert!(Currency::parse("XRP").is_err());
        assert!(Currency::parse("TOOLONG").is_err());
        let hex = "0158415500000000C1F76FF6ECB0BAC600000000";
        assert_eq!(encode_hex(&Currency::parse(hex)?.0), hex);
        assert!(Currency::parse(&"Z".repeat(40)).is_err());
        Ok(())
    }

    #[test]
    fn values_must_fit_their_field() {
        let mut tx = Transaction::default();
        assert!(tx.set("Sequence", Value::U16(1)).is_err());
        assert!(tx.set("Nonsense", Value::U32(1)).is_err());
        assert!(Transaction::new("AMMCreate").is_err());
        assert!(
            tx.set("Fee", Value::Amount(Amount::Drops(MAX_DROPS + 1)))
                .is_ok()
        );
        assert!(tx.serialize().is_err());
    }

    #[test]
    fn setting_a_field_twice_keeps_the_last_value() -> Result<(), CodecError> {
        let mut tx = Transaction::new("AccountSet")?;
        tx.set("Sequence", Value::U32(1))?;
        tx.set("Sequence", Value::U32(2))?;
        assert_eq!(tx.get("Sequence"), Some(&Value::U32(2)));
        Ok(())
    }

    #[test]
    fn hex_round_trips_and_rejects_bad_input() {
        assert_eq!(decode_hex("00ff10").ok(), Some(vec![0, 255, 16]));
        assert_eq!(encode_hex(&[0, 255, 16]), "00FF10");
        assert!(decode_hex("abc").is_err());
        assert!(decode_hex("zz").is_err());
    }

    #[test]
    fn json_with_unhandled_or_bad_fields_is_refused() {
        use serde_json::json;
        assert!(from_json(&json!("x")).is_err());
        assert!(from_json(&json!({})).is_err());
        assert!(from_json(&json!({"TransactionType": "AccountSet", "Memos": []})).is_err());
        assert!(from_json(&json!({"TransactionType": "AccountSet", "Sequence": "1"})).is_err());
        assert!(
            from_json(&json!({"TransactionType": "AccountSet", "Sequence": 5_000_000_000u64}))
                .is_err()
        );
        assert!(from_json(&json!({"TransactionType": "CheckCash", "CheckID": "AB"})).is_err());
        assert!(from_json(&json!({"TransactionType": "AccountSet", "SigningPubKey": 7})).is_err());
        assert!(from_json(&json!({"TransactionType": "AccountSet", "Account": 7})).is_err());
        assert!(from_json(&json!({"TransactionType": "Payment", "Amount": "ten"})).is_err());
        assert!(
            from_json(&json!({"TransactionType": "Payment", "Amount": {"value": "1"}})).is_err()
        );
        assert!(
            from_json(&json!({"TransactionType": "AccountSet", "date": 1, "DeliverMax": "1"}))
                .is_ok()
        );
    }

    #[test]
    fn errors_say_what_is_wrong() {
        let all = [
            CodecError::UnknownField("X".into()),
            CodecError::UnknownTransactionType("Y".into()),
            CodecError::BadValue("Fee", "why".into()),
            CodecError::BadAmount("z".into()),
            CodecError::Key(KeyError::NotBase58),
        ];
        for e in all {
            assert!(!e.to_string().is_empty());
        }
        assert_eq!(account_text(&[0u8; 20]), "rrrrrrrrrrrrrrrrrrrrrhoLvTp");
    }
}
