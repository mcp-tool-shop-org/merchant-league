//! Merchant League seasons.
//!
//! A season runs from one reset of the XRPL Testnet to the next: a reset
//! deletes every account and transaction, so it ends the season. A reset is
//! known in one of two ways, ported from xrpl-lab's `resets.py`:
//!
//! 1. **Announced.** The network's operators publish the day. The notice gives
//!    a day with no hour or timezone, so the reset may begin as soon as that
//!    day has begun anywhere (UTC+14) and is certain only once it has ended
//!    everywhere (UTC-12).
//! 2. **Seen.** Ledger indexes only grow on a live network, so a validated
//!    index well below one seen before means the network was reset, announced
//!    or not.
//!
//! The same list lives in xrpl-lab (the source), xrpl-camp and sovereignty.
//! A new announced reset is added in all four places.
//!
//! The season record and its signature arrive in a later milestone.

/// A server can lag behind the network by this many ledgers. A lower index
/// within this distance is lag, not a reset. Same value as xrpl-lab.
pub const LEDGER_LAG_TOLERANCE: u64 = 1_000;

/// An announced reset of the Testnet, on one day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownReset {
    /// The announced day: year, month (1 to 12), day of month.
    pub day: (i32, u8, u8),
    /// Where the reset was announced.
    pub source: &'static str,
}

const HOUR: i64 = 3_600;
const DAY: i64 = 24 * HOUR;

impl KnownReset {
    /// The earliest moment the reset may begin: its day starting at UTC+14,
    /// in Unix seconds.
    pub fn starts_unix(&self) -> i64 {
        let (y, m, d) = self.day;
        days_from_civil(y, m, d) * DAY - 14 * HOUR
    }

    /// The moment the reset day has ended everywhere (UTC-12), in Unix
    /// seconds. Anything recorded before this may have been wiped.
    pub fn ends_unix(&self) -> i64 {
        let (y, m, d) = self.day;
        (days_from_civil(y, m, d) + 1) * DAY + 12 * HOUR
    }

    /// A short label for a player, e.g. "Testnet was reset on 2026-11-25".
    pub fn label(&self) -> String {
        let (y, m, d) = self.day;
        format!("Testnet was reset on {y:04}-{m:02}-{d:02}")
    }
}

/// Announced Testnet resets, oldest first.
pub const KNOWN_RESETS: &[KnownReset] = &[KnownReset {
    day: (2026, 11, 25),
    source: "https://xrpl.org/blog/2026/testnet-devnet-reset",
}];

/// True when the network's validated ledger is well below an index seen
/// before, which only a reset explains. A missing or zero index on either
/// side means nothing to compare, so the answer is no.
pub fn ledger_went_backwards(recorded: Option<u64>, current: Option<u64>) -> bool {
    match (recorded, current) {
        (Some(seen), Some(now)) if seen > 0 && now > 0 => {
            now < seen.saturating_sub(LEDGER_LAG_TOLERANCE)
        }
        _ => false,
    }
}

/// Days from 1970-01-01 to a date in the proleptic Gregorian calendar
/// (Howard Hinnant's `days_from_civil`).
fn days_from_civil(year: i32, month: u8, day: u8) -> i64 {
    let (m, d) = (i64::from(month), i64::from(day));
    let y = i64::from(year) - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_2026_11_25_reset_is_listed() {
        let reset = KNOWN_RESETS[0];
        assert_eq!(reset.day, (2026, 11, 25));
        assert!(reset.source.starts_with("https://xrpl.org/"));
        assert_eq!(reset.label(), "Testnet was reset on 2026-11-25");
    }

    #[test]
    fn the_reset_day_spans_every_timezone() {
        let reset = KNOWN_RESETS[0];
        // 2026-11-24T10:00:00Z and 2026-11-26T12:00:00Z.
        assert_eq!(reset.starts_unix(), 1_795_514_400);
        assert_eq!(reset.ends_unix(), 1_795_694_400);
        assert_eq!(reset.ends_unix() - reset.starts_unix(), 50 * HOUR);
    }

    #[test]
    fn civil_days_match_known_dates() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(days_from_civil(2024, 2, 29), 19_782);
        assert_eq!(days_from_civil(2026, 11, 25), 20_782);
        assert_eq!(days_from_civil(1969, 12, 31), -1);
    }

    #[test]
    fn a_large_fall_is_a_reset() {
        assert!(ledger_went_backwards(Some(22_310_595), Some(5)));
        assert!(ledger_went_backwards(Some(22_310_595), Some(22_309_594)));
    }

    #[test]
    fn lag_within_the_tolerance_is_not_a_reset() {
        assert!(!ledger_went_backwards(Some(22_310_595), Some(22_309_595)));
        assert!(!ledger_went_backwards(Some(22_310_595), Some(22_310_595)));
        assert!(!ledger_went_backwards(Some(22_310_595), Some(22_310_700)));
        assert!(!ledger_went_backwards(Some(500), Some(1)));
    }

    #[test]
    fn nothing_to_compare_is_not_a_reset() {
        assert!(!ledger_went_backwards(None, Some(5)));
        assert!(!ledger_went_backwards(Some(22_310_595), None));
        assert!(!ledger_went_backwards(Some(0), Some(5)));
        assert!(!ledger_went_backwards(Some(22_310_595), Some(0)));
    }
}
