const HEX_LOWER: &[u8; 16] = b"0123456789abcdef";

pub(crate) fn hex_encode_32(bytes: &[u8; 32]) -> String {
    let mut buf = [0u8; 64];
    for (idx, &byte) in bytes.iter().enumerate() {
        buf[2 * idx] = HEX_LOWER[usize::from(byte >> 4)];
        buf[2 * idx + 1] = HEX_LOWER[usize::from(byte & 0x0f)];
    }
    // Every byte is ASCII hex (`0..=9` / `a..=f`), which is always valid UTF-8.
    String::from_utf8(buf.to_vec()).unwrap_or_default()
}

pub(crate) fn hex_decode_32(input: &str) -> Result<[u8; 32], String> {
    let bytes = input.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return Err("Odd number of digits".to_owned());
    }
    if bytes.len() != 64 {
        return Err("expected 32 hex-encoded bytes".to_owned());
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        let hi = decode_nibble(bytes[2 * i], 2 * i)?;
        let lo = decode_nibble(bytes[2 * i + 1], 2 * i + 1)?;
        out[i] = (hi << 4) | lo;
    }
    Ok(out)
}

fn decode_nibble(byte: u8, index: usize) -> Result<u8, String> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        other => Err(format!(
            "Invalid character '{}' at position {index}",
            other as char
        )),
    }
}

pub(crate) mod hex32 {
    use serde::{Deserialize, Deserializer, Serializer};

    use super::{hex_decode_32, hex_encode_32};

    pub(crate) fn serialize<S: Serializer>(
        bytes: &[u8; 32],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&hex_encode_32(bytes))
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<[u8; 32], D::Error> {
        let s = String::deserialize(deserializer)?;
        hex_decode_32(&s).map_err(serde::de::Error::custom)
    }
}

pub(crate) mod opt_hex32 {
    use serde::{Deserialize, Deserializer, Serializer};

    use super::{hex_decode_32, hex_encode_32};

    pub(crate) fn serialize<S: Serializer>(
        value: &Option<[u8; 32]>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            Some(bytes) => serializer.serialize_some(&hex_encode_32(bytes)),
            None => serializer.serialize_none(),
        }
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<[u8; 32]>, D::Error> {
        let opt = Option::<String>::deserialize(deserializer)?;
        match opt {
            None => Ok(None),
            Some(s) => hex_decode_32(&s)
                .map(Some)
                .map_err(serde::de::Error::custom),
        }
    }
}

pub(crate) mod delivery_ledger_serde {
    use std::collections::BTreeMap;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use crate::types::{AssetId, Units};

    type LedgerMap = BTreeMap<(u8, u8, AssetId), Units>;
    type LedgerEntry = ((u8, u8, AssetId), Units);

    pub(crate) fn serialize<S: Serializer>(
        map: &LedgerMap,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let entries: Vec<LedgerEntry> = map.iter().map(|(&k, &v)| (k, v)).collect();
        entries.serialize(serializer)
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<LedgerMap, D::Error> {
        let entries = Vec::<LedgerEntry>::deserialize(deserializer)?;
        let mut map = LedgerMap::new();
        for (k, v) in entries {
            if map.insert(k, v).is_some() {
                return Err(serde::de::Error::custom("duplicate key in delivery_ledger"));
            }
        }
        Ok(map)
    }
}
