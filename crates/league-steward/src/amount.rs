//! Exact integer milli-unit <-> XRPL IOU amount encoding and parsing (`B3.3`).
//!
//! Resolves `B3.3` (`docs/game-mode-protocol.md` §1): all `TurnEngine` balances are exact `i64`
//! milli-units (`UNIT_SCALE = 1_000`). Outgoing IOU strings use `.3` fixed decimals with zero
//! floating-point math, and incoming `account_lines` strings are parsed exactly to integer
//! milli-units (rejecting any non-zero digit past `10^-3`).

use league_core::{AssetId, UNIT_SCALE, Units};
use league_standalone::encode_currency_code;

use crate::error::StewardError;

const _: () = assert!(UNIT_SCALE == 1_000);

/// Map a [`league_core::AssetId`] to its canonical XRPL issued currency code (`CRED` hex or
/// 3-char `R00..`/`M00..`).
pub fn asset_currency_code(asset: AssetId) -> Result<String, StewardError> {
    match asset {
        AssetId::Cred => {
            encode_currency_code("CRED").map_err(|err| StewardError::Protocol(err.to_string()))
        }
        AssetId::Raw(idx) => {
            if idx >= 100 {
                return Err(StewardError::Protocol(format!(
                    "raw asset index {idx} exceeds 2-digit currency code"
                )));
            }
            Ok(format!("R{idx:02}"))
        }
        AssetId::Refined(idx) => {
            if idx >= 100 {
                return Err(StewardError::Protocol(format!(
                    "refined asset index {idx} exceeds 2-digit currency code"
                )));
            }
            Ok(format!("M{idx:02}"))
        }
    }
}

/// Format non-negative integer milli-units as an exact XRPL decimal string (e.g. `1_250` -> `"1.250"`).
pub fn format_milli_units(units: Units) -> Result<String, StewardError> {
    if units < 0 {
        return Err(StewardError::Protocol(format!(
            "XRPL IOU amount cannot be negative: {units}"
        )));
    }
    let whole = units / UNIT_SCALE;
    let frac = units % UNIT_SCALE;
    if frac == 0 {
        Ok(whole.to_string())
    } else {
        Ok(format!("{whole}.{frac:03}"))
    }
}

/// Parse an XRPL decimal or scientific-notation IOU balance string into exact integer milli-units.
///
/// Rejects malformed strings, fractional digits beyond `10^-3` (except trailing zeros), and
/// `i64` overflow. Never uses IEEE-754 floating point.
pub fn parse_iou_milli_exact(text: &str) -> Result<Units, StewardError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(StewardError::Protocol(
            "empty XRPL IOU amount string".into(),
        ));
    }
    let (neg, unsigned) = if let Some(rest) = trimmed.strip_prefix('-') {
        (true, rest)
    } else if let Some(rest) = trimmed.strip_prefix('+') {
        (false, rest)
    } else {
        (false, trimmed)
    };
    if unsigned.is_empty() {
        return Err(StewardError::Protocol(format!(
            "invalid XRPL IOU amount {text:?}"
        )));
    }

    let (mantissa_str, exp_shift) = match unsigned.split_once(['e', 'E']) {
        Some((m, e)) => {
            let exp: i32 = e.parse().map_err(|_| {
                StewardError::Protocol(format!("invalid exponent in XRPL IOU amount {text:?}"))
            })?;
            (m, exp)
        }
        None => (unsigned, 0_i32),
    };

    let (whole_part, frac_part) = match mantissa_str.split_once('.') {
        Some((w, f)) => {
            if w.is_empty() && f.is_empty() {
                return Err(StewardError::Protocol(format!(
                    "invalid mantissa in XRPL IOU amount {text:?}"
                )));
            }
            (w, f)
        }
        None => (mantissa_str, ""),
    };

    if !whole_part.bytes().all(|b| b.is_ascii_digit())
        || !frac_part.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(StewardError::Protocol(format!(
            "non-digit in XRPL IOU amount {text:?}"
        )));
    }

    let mut digits = String::with_capacity(whole_part.len() + frac_part.len());
    digits.push_str(whole_part);
    digits.push_str(frac_part);
    let frac_len = i32::try_from(frac_part.len())
        .map_err(|_| StewardError::Protocol(format!("fractional length overflow in {text:?}")))?;

    // Target value in milli-units is `digits_int * 10^(3 + exp_shift - frac_len)`.
    let net_pow10 = 3_i32
        .checked_add(exp_shift)
        .and_then(|v| v.checked_sub(frac_len))
        .ok_or_else(|| StewardError::Protocol(format!("exponent overflow in {text:?}")))?;

    if digits.bytes().all(|b| b == b'0') {
        return Ok(0);
    }

    let mut value: i128 = 0;
    if net_pow10 >= 0 {
        for b in digits.bytes() {
            value = value
                .checked_mul(10)
                .and_then(|v| v.checked_add(i128::from(b - b'0')))
                .ok_or_else(|| StewardError::Protocol(format!("IOU overflow in {text:?}")))?;
        }
        for _ in 0..net_pow10 {
            value = value
                .checked_mul(10)
                .ok_or_else(|| StewardError::Protocol(format!("IOU scale overflow in {text:?}")))?;
        }
    } else {
        let drop_right = usize::try_from(-net_pow10)
            .map_err(|_| StewardError::Protocol(format!("negative shift error in {text:?}")))?;
        if drop_right > digits.len() {
            // Every digit sits below 10^-3, and we already checked not all digits are zero.
            return Err(StewardError::Protocol(format!(
                "sub-milli-unit precision in XRPL IOU amount {text:?}"
            )));
        }
        let split_at = digits.len() - drop_right;
        let (keep, dropped) = digits.split_at(split_at);
        if dropped.bytes().any(|b| b != b'0') {
            return Err(StewardError::Protocol(format!(
                "sub-milli-unit precision in XRPL IOU amount {text:?}"
            )));
        }
        for b in keep.bytes() {
            value = value
                .checked_mul(10)
                .and_then(|v| v.checked_add(i128::from(b - b'0')))
                .ok_or_else(|| StewardError::Protocol(format!("IOU overflow in {text:?}")))?;
        }
    }

    let signed = if neg { -value } else { value };
    Units::try_from(signed)
        .map_err(|_| StewardError::Protocol(format!("IOU amount {text:?} exceeds i64 milli-units")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn currency_codes_and_milli_roundtrip_are_exact() {
        assert_eq!(
            asset_currency_code(AssetId::Cred).ok().as_deref(),
            Some("4352454400000000000000000000000000000000")
        );
        assert_eq!(
            asset_currency_code(AssetId::Raw(0)).ok().as_deref(),
            Some("R00")
        );
        assert_eq!(
            asset_currency_code(AssetId::Refined(5)).ok().as_deref(),
            Some("M05")
        );
        assert!(asset_currency_code(AssetId::Raw(100)).is_err());
        assert!(asset_currency_code(AssetId::Refined(100)).is_err());

        for &val in &[0_i64, 1, 333, 1_000, 1_250, 999_999] {
            let encoded = format_milli_units(val).unwrap_or_else(|e| panic!("encode: {e}"));
            let parsed = parse_iou_milli_exact(&encoded).unwrap_or_else(|e| panic!("parse: {e}"));
            assert_eq!(parsed, val);
        }
        assert!(format_milli_units(-1).is_err());

        assert_eq!(parse_iou_milli_exact("+12.500"), Ok(12_500));
        assert_eq!(parse_iou_milli_exact("1e-3"), Ok(1));
        assert_eq!(parse_iou_milli_exact("2.5e2"), Ok(250_000));
        assert_eq!(parse_iou_milli_exact("-0.0000"), Ok(0));
        assert_eq!(parse_iou_milli_exact("-12.34500"), Ok(-12_345));
        assert!(parse_iou_milli_exact("+").is_err());
        assert!(parse_iou_milli_exact("-").is_err());
        assert!(parse_iou_milli_exact(".").is_err());
        assert!(parse_iou_milli_exact("1eX").is_err());
        assert!(parse_iou_milli_exact("1.a").is_err());
        assert!(parse_iou_milli_exact("0.00005").is_err());
        assert!(parse_iou_milli_exact("0.0001").is_err());
        assert!(parse_iou_milli_exact("1e-4").is_err());
        assert!(parse_iou_milli_exact("1e-5").is_err());
        assert!(parse_iou_milli_exact("1e25").is_err());
        assert!(parse_iou_milli_exact("1e40").is_err());
        assert!(parse_iou_milli_exact("9999999999999999999").is_err());
        assert!(parse_iou_milli_exact("9999999999999999999999999999999999999999").is_err());
        assert!(
            parse_iou_milli_exact(&format!("1.{}", "0".repeat(30))).is_err()
                || parse_iou_milli_exact(&format!("0.{}1", "0".repeat(4))).is_err()
        );
        assert!(parse_iou_milli_exact("").is_err());
        assert!(parse_iou_milli_exact("abc").is_err());
    }
}
