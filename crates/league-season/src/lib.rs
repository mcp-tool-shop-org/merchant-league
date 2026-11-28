//! Merchant League seasons and XRPL Testnet reset detection.
//!
//! A season is 24 to 32 turns (`docs/design-v2.md` §6.2, Director Decision 3).
//! This crate holds the Testnet's announced reset dates and a check that spots
//! an unannounced reset, ported from `xrpl-lab`'s `resets.py`:
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

use std::fmt;

/// A server can lag behind the network by this many ledgers. A lower index
/// within this distance is lag, not a reset. Same value as xrpl-lab.
pub const LEDGER_LAG_TOLERANCE: u64 = 1_000;

/// Signal indicating that a previously active ledger was wiped or reset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WipeSignal {
    /// Previously funded table Authority/Steward account returned `actNotFound`.
    AuthorityAccountMissing {
        /// Classic `r...` address of the missing Authority account.
        authority_address: String,
    },
    /// Validated ledger index fell below a previously observed index beyond [`LEDGER_LAG_TOLERANCE`].
    LedgerRewind {
        /// Highest validated ledger index seen before the reset.
        previous_ledger: u64,
        /// Newly observed validated ledger index.
        observed_ledger: u64,
    },
    /// Observed ledger-1/genesis hash differs from the hash recorded before disconnect.
    GenesisHashChanged {
        /// Genesis hash previously recorded.
        previous_genesis_hash: String,
        /// Newly observed genesis hash.
        observed_genesis_hash: String,
    },
}

impl WipeSignal {
    /// Stable `SCREAMING_SNAKE_CASE` identifier for the detected wipe reason.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::AuthorityAccountMissing { .. } => "WIPE_AUTHORITY_MISSING",
            Self::LedgerRewind { .. } => "WIPE_LEDGER_REWIND",
            Self::GenesisHashChanged { .. } => "WIPE_GENESIS_CHANGED",
        }
    }
}

impl fmt::Display for WipeSignal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AuthorityAccountMissing { authority_address } => write!(
                f,
                "authority account {authority_address} missing (actNotFound)"
            ),
            Self::LedgerRewind {
                previous_ledger,
                observed_ledger,
            } => write!(
                f,
                "validated ledger rewound from {previous_ledger} to {observed_ledger}"
            ),
            Self::GenesisHashChanged {
                previous_genesis_hash,
                observed_genesis_hash,
            } => write!(
                f,
                "genesis ledger hash changed from {previous_genesis_hash} to {observed_genesis_hash}"
            ),
        }
    }
}

/// Inputs sampled from an XRPL node to check whether a prior table ledger was wiped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WipeProbeInput<'a> {
    /// Classic `r...` address of the table's Authority/Steward account.
    pub authority_address: &'a str,
    /// `Some(false)` when `account_info` on `authority_address` returned `actNotFound`.
    pub authority_exists: Option<bool>,
    /// Highest validated ledger index observed prior to this probe.
    pub previous_ledger: Option<u64>,
    /// Current validated ledger index observed on the node.
    pub observed_ledger: Option<u64>,
    /// Previously recorded genesis/ledger-1 hash, if captured.
    pub previous_genesis_hash: Option<&'a str>,
    /// Current genesis/ledger-1 hash observed on the node, if captured.
    pub observed_genesis_hash: Option<&'a str>,
}

/// Inspect node probe signals and return the highest-priority [`WipeSignal`], if wiped.
#[must_use]
pub fn detect_ledger_wipe(input: &WipeProbeInput<'_>) -> Option<WipeSignal> {
    if input.authority_exists == Some(false) && !input.authority_address.is_empty() {
        return Some(WipeSignal::AuthorityAccountMissing {
            authority_address: input.authority_address.to_string(),
        });
    }
    if ledger_went_backwards(input.previous_ledger, input.observed_ledger)
        && let (Some(previous_ledger), Some(observed_ledger)) =
            (input.previous_ledger, input.observed_ledger)
    {
        return Some(WipeSignal::LedgerRewind {
            previous_ledger,
            observed_ledger,
        });
    }
    if let (Some(prev), Some(obs)) = (input.previous_genesis_hash, input.observed_genesis_hash)
        && !prev.is_empty()
        && !obs.is_empty()
        && !prev.eq_ignore_ascii_case(obs)
    {
        return Some(WipeSignal::GenesisHashChanged {
            previous_genesis_hash: prev.to_string(),
            observed_genesis_hash: obs.to_string(),
        });
    }
    None
}

/// Return the latest announced [`KnownReset`] spanning `[last_seen_unix, now_unix]`, if any.
#[must_use]
pub fn known_reset_spanning(last_seen_unix: i64, now_unix: i64) -> Option<KnownReset> {
    if last_seen_unix > now_unix {
        return None;
    }
    KNOWN_RESETS
        .iter()
        .rev()
        .copied()
        .find(|reset| last_seen_unix < reset.ends_unix() && now_unix >= reset.starts_unix())
}

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

    #[test]
    fn wipe_detection_catches_missing_authority_rewind_and_genesis_change() {
        let missing = detect_ledger_wipe(&WipeProbeInput {
            authority_address: "rStewardAddr",
            authority_exists: Some(false),
            previous_ledger: Some(250),
            observed_ledger: Some(2),
            previous_genesis_hash: Some("AAAA"),
            observed_genesis_hash: Some("AAAA"),
        });
        assert_eq!(
            missing,
            Some(WipeSignal::AuthorityAccountMissing {
                authority_address: "rStewardAddr".to_string(),
            })
        );
        if let Some(ref sig) = missing {
            assert_eq!(sig.code(), "WIPE_AUTHORITY_MISSING");
            assert!(sig.to_string().contains("actNotFound"));
        }

        let rewind = detect_ledger_wipe(&WipeProbeInput {
            authority_address: "rStewardAddr",
            authority_exists: Some(true),
            previous_ledger: Some(22_310_595),
            observed_ledger: Some(12),
            previous_genesis_hash: None,
            observed_genesis_hash: None,
        });
        assert_eq!(
            rewind,
            Some(WipeSignal::LedgerRewind {
                previous_ledger: 22_310_595,
                observed_ledger: 12,
            })
        );
        if let Some(ref sig) = rewind {
            assert_eq!(sig.code(), "WIPE_LEDGER_REWIND");
            assert!(sig.to_string().contains("22310595"));
        }

        let genesis_changed = detect_ledger_wipe(&WipeProbeInput {
            authority_address: "rStewardAddr",
            authority_exists: Some(true),
            previous_ledger: Some(300),
            observed_ledger: Some(301),
            previous_genesis_hash: Some("ABCD"),
            observed_genesis_hash: Some("EF01"),
        });
        assert_eq!(
            genesis_changed,
            Some(WipeSignal::GenesisHashChanged {
                previous_genesis_hash: "ABCD".to_string(),
                observed_genesis_hash: "EF01".to_string(),
            })
        );
        if let Some(ref sig) = genesis_changed {
            assert_eq!(sig.code(), "WIPE_GENESIS_CHANGED");
            assert!(sig.to_string().contains("ABCD"));
        }

        let healthy = detect_ledger_wipe(&WipeProbeInput {
            authority_address: "rStewardAddr",
            authority_exists: Some(true),
            previous_ledger: Some(22_310_595),
            observed_ledger: Some(22_310_000),
            previous_genesis_hash: Some("abcd"),
            observed_genesis_hash: Some("ABCD"),
        });
        assert_eq!(healthy, None);
    }

    #[test]
    fn known_reset_spanning_checks_window_boundaries() {
        let reset = KNOWN_RESETS[0];
        assert_eq!(
            known_reset_spanning(reset.starts_unix() - 10, reset.starts_unix()),
            Some(reset)
        );
        assert_eq!(
            known_reset_spanning(reset.ends_unix() - 1, reset.ends_unix() + 10),
            Some(reset)
        );
        assert_eq!(
            known_reset_spanning(reset.ends_unix(), reset.ends_unix() + 10),
            None
        );
        assert_eq!(
            known_reset_spanning(reset.starts_unix() - 20, reset.starts_unix() - 1),
            None
        );
        assert_eq!(
            known_reset_spanning(reset.ends_unix(), reset.starts_unix()),
            None
        );
    }
}
