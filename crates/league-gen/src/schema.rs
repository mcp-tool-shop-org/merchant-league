//! Schema, configuration, and error types for Merchant League v2 chiral tables.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Generator release identifier embedded in every produced [`RingTable`] for cross-run comparability.
pub const GENERATOR_VERSION: &str = "league-gen/v2.4.1";

/// Fixed ledger numeraire currency code specified by `docs/design-v2.md` §1.2.
pub const NUMERAIRE_CODE: &str = "CRED";

/// Provisional upper cap on `GeneratorConfig::max_attempts` preventing unbounded search loops.
pub const MAX_ATTEMPTS_CAP: u32 = 10_000;

/// Reserved tier-3 full-ring Syndicate Concordat project id (`N`-good collective assembly).
pub const PROJECT_CONCORDAT_ID: u8 = 100;

/// Minimum Turn-1 Private Charter Mandate completion bonus in Victory Points (`3.0 VP`).
pub const MIN_MANDATE_BONUS_VP: f64 = 3.0;

/// Maximum Turn-1 Private Charter Mandate completion bonus in Victory Points (`8.0 VP`).
pub const MAX_MANDATE_BONUS_VP: f64 = 8.0;

/// Basis-point scale (`10_000 = 100%`).
pub const BPS_SCALE: u16 = 10_000;

/// Validated seat count for a chiral converter ring (`4`, `5`, or `6`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SeatCount {
    /// Four-seat chiral converter ring.
    Four = 4,
    /// Five-seat chiral converter ring.
    Five = 5,
    /// Six-seat chiral converter ring.
    Six = 6,
}

impl SeatCount {
    /// All supported chiral ring seat counts in ascending order.
    pub const ALL: [Self; 3] = [Self::Four, Self::Five, Self::Six];

    /// Converts a raw integer count into a validated [`SeatCount`], or `None` if unsupported.
    #[must_use]
    pub const fn from_usize(n: usize) -> Option<Self> {
        match n {
            4 => Some(Self::Four),
            5 => Some(Self::Five),
            6 => Some(Self::Six),
            _ => None,
        }
    }

    /// Returns the number of seats as `usize`.
    #[must_use]
    pub const fn as_usize(self) -> usize {
        self as usize
    }

    /// Returns the number of seats as `u8`.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    /// Returns the number of unordered seat pairs `N * (N - 1) / 2`.
    #[must_use]
    pub const fn pair_count(self) -> usize {
        let n = self.as_usize();
        n * (n - 1) / 2
    }

    /// Provisional C6 table-level top-pair share cap (`4p: 0.45`, `5p: 0.35`, `6p: 0.30`).
    ///
    /// Provisional pending playtest calibration: the 2026-11-15 ruling established the pair-share
    /// table check concept without specifying numeric cap values, and these thresholds do not bind
    /// on generated tables today (`static_max_pair_share` and rollout pair shares fall well below).
    #[must_use]
    pub const fn default_max_pair_share_cap(self) -> f64 {
        match self {
            Self::Four => 0.45,
            Self::Five => 0.35,
            Self::Six => 0.30,
        }
    }
}

/// Method used by [`crate::check_feasibility`] to compute per-seat expected value (Invariant 3).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EvMethod {
    /// Competitive/cooperative LP-dual shadow-price allocation (`docs/design-v2.md` §1.5 default).
    #[default]
    ShadowPrice,
    /// Exact `2^N` sub-coalition LP Shapley value allocation (retained for comparison).
    Shapley,
}

/// Three-character uppercase ASCII ledger commodity code (`[A-Z]{3}`) drawn strictly from the
/// curated game-word allow-list ([`ALLOWED_COMMODITY_GLOSSARY`]), never overlapping `"XRP"`,
/// active/historical ISO 4217 currency codes, crypto tickers, or reserved strings.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CurrencyCode([u8; 3]);

/// Curated, strictly sorted allow-list of 61 medieval-guild game-word commodity codes (`[A-Z]{3}`)
/// mapped to their descriptive English trade goods (`#26a`).
///
/// Generating and validating currency codes exclusively from this list guarantees that neither
/// active/historical ISO 4217 codes nor cryptocurrency/XRPL ecosystem tickers can ever appear.
pub const ALLOWED_COMMODITY_GLOSSARY: &[([u8; 3], &str)] = &[
    (*b"ALM", "alum"),
    (*b"ANV", "anvil"),
    (*b"BBL", "barrel"),
    (*b"BLW", "bellows"),
    (*b"BLY", "barley"),
    (*b"BRS", "brass"),
    (*b"CBL", "cobble"),
    (*b"CDR", "cedar"),
    (*b"CHK", "chalk"),
    (*b"CLY", "clay"),
    (*b"CNV", "canvas"),
    (*b"CPR", "copper"),
    (*b"CRL", "coral"),
    (*b"DYE", "dye"),
    (*b"ELM", "elm"),
    (*b"ENM", "enamel"),
    (*b"FIR", "fir"),
    (*b"FLN", "flint"),
    (*b"GLS", "glass"),
    (*b"GLZ", "glaze"),
    (*b"HMP", "hemp"),
    (*b"HNY", "honey"),
    (*b"HPS", "hops"),
    (*b"HRB", "herb"),
    (*b"IRN", "iron"),
    (*b"IVR", "ivory"),
    (*b"JDE", "jade"),
    (*b"KLN", "kiln"),
    (*b"KLP", "kelp"),
    (*b"LOM", "loom"),
    (*b"LTH", "leather"),
    (*b"LYE", "lye"),
    (*b"MLT", "malt"),
    (*b"MRB", "marble"),
    (*b"NTR", "nitre"),
    (*b"OAK", "oak"),
    (*b"OAT", "oat"),
    (*b"OLV", "olive"),
    (*b"PCH", "pitch"),
    (*b"PLK", "plank"),
    (*b"PWT", "pewter"),
    (*b"QRZ", "quartz"),
    (*b"ROP", "rope"),
    (*b"RSN", "resin"),
    (*b"RYE", "rye"),
    (*b"SLG", "slag"),
    (*b"SLK", "silk"),
    (*b"SLT", "salt"),
    (*b"SPN", "spindle"),
    (*b"SUF", "sulfur"),
    (*b"TLW", "tallow"),
    (*b"TMB", "timber"),
    (*b"TNG", "tongs"),
    (*b"TRF", "turf"),
    (*b"VLM", "vellum"),
    (*b"VLV", "velvet"),
    (*b"WHT", "wheat"),
    (*b"WLK", "whelk"),
    (*b"WOL", "wool"),
    (*b"YEW", "yew"),
    (*b"ZNC", "zinc"),
];

/// Returns `true` when `bytes` is present in the curated game-word allow-list [`ALLOWED_COMMODITY_GLOSSARY`].
#[must_use]
pub fn is_allowed_commodity_code(bytes: [u8; 3]) -> bool {
    ALLOWED_COMMODITY_GLOSSARY
        .binary_search_by_key(&bytes, |&(code, _)| code)
        .is_ok()
}

/// Returns the descriptive English trade good for an allow-listed [`CurrencyCode`].
#[must_use]
pub fn commodity_glossary_word(code: CurrencyCode) -> Option<&'static str> {
    ALLOWED_COMMODITY_GLOSSARY
        .binary_search_by_key(&code.0, |&(entry, _)| entry)
        .ok()
        .map(|idx| ALLOWED_COMMODITY_GLOSSARY[idx].1)
}

/// Sorted deny-list of 3-letter currency, precious-metal, crypto/ledger, and reserved codes
/// excluded from procedural commodity code generation (`docs/design-v2.md` Hard Invariant 1).
#[rustfmt::skip]
pub const DENIED_CURRENCY_CODES: &[[u8; 3]] = &[
    *b"AAV", *b"ADA", *b"ADP", *b"AED", *b"AFA", *b"AFN", *b"AKT", *b"ALG", *b"ALK", *b"ALL",
    *b"AMD", *b"ANC", *b"ANG", *b"ANK", *b"ANT", *b"AOA", *b"AOK", *b"AON", *b"AOR", *b"APE",
    *b"APR", *b"APT", *b"APY", *b"ARA", *b"ARB", *b"ARP", *b"ARS", *b"ARY", *b"ASR", *b"ASS",
    *b"ATM", *b"ATS", *b"AUD", *b"AVA", *b"AWG", *b"AXL", *b"AXS", *b"AZM", *b"AZN", *b"BAD",
    *b"BAL", *b"BAM", *b"BAT", *b"BBD", *b"BCH", *b"BDT", *b"BEC", *b"BEF", *b"BEL", *b"BEM",
    *b"BET", *b"BGL", *b"BGM", *b"BGN", *b"BGO", *b"BHD", *b"BIF", *b"BIS", *b"BLR", *b"BMD",
    *b"BNB", *b"BND", *b"BNK", *b"BNT", *b"BOB", *b"BOL", *b"BOP", *b"BOV", *b"BRB", *b"BRC",
    *b"BRE", *b"BRL", *b"BRN", *b"BRR", *b"BRZ", *b"BSD", *b"BSV", *b"BTC", *b"BTN", *b"BTT",
    *b"BUK", *b"BUY", *b"BWP", *b"BYB", *b"BYN", *b"BYR", *b"BZD", *b"CAD", *b"CAK", *b"CDF",
    *b"CEL", *b"CFX", *b"CHE", *b"CHF", *b"CHW", *b"CHZ", *b"CKB", *b"CLF", *b"CLP", *b"CMP",
    *b"CNY", *b"COP", *b"COR", *b"COU", *b"CPI", *b"CRC", *b"CRD", *b"CRO", *b"CRV", *b"CSC",
    *b"CSD", *b"CSJ", *b"CSK", *b"CUC", *b"CUP", *b"CVC", *b"CVE", *b"CYP", *b"CZK", *b"DAI",
    *b"DCR", *b"DDM", *b"DEM", *b"DGB", *b"DIE", *b"DJF", *b"DKK", *b"DOG", *b"DOP", *b"DOT",
    *b"DSH", *b"DYX", *b"DZD", *b"ECS", *b"ECV", *b"EEK", *b"EGL", *b"EGP", *b"EIG", *b"ELG",
    *b"ELS", *b"ENA", *b"ENJ", *b"EOS", *b"ERN", *b"ESA", *b"ESB", *b"ESP", *b"ETB", *b"ETC",
    *b"ETF", *b"ETH", *b"EUR", *b"EVR", *b"FCK", *b"FED", *b"FEE", *b"FET", *b"FIL", *b"FIM",
    *b"FJD", *b"FKP", *b"FLK", *b"FLR", *b"FLW", *b"FRF", *b"FTM", *b"FTT", *b"FUK", *b"GAL",
    *b"GBP", *b"GBX", *b"GDP", *b"GEK", *b"GEL", *b"GHC", *b"GHP", *b"GHS", *b"GIP", *b"GLM",
    *b"GMD", *b"GMT", *b"GMX", *b"GNE", *b"GNF", *b"GNO", *b"GNS", *b"GOD", *b"GQE", *b"GRD",
    *b"GRT", *b"GTQ", *b"GUN", *b"GWE", *b"GWP", *b"GYD", *b"HBR", *b"HKD", *b"HNL", *b"HOT",
    *b"HRD", *b"HRK", *b"HTG", *b"HTX", *b"HUF", *b"HYP", *b"ICP", *b"ICX", *b"IDR", *b"IEP",
    *b"ILP", *b"ILR", *b"ILS", *b"IMF", *b"IMX", *b"INJ", *b"INR", *b"IOT", *b"IOU", *b"IPO",
    *b"IQD", *b"IRR", *b"ISJ", *b"ISK", *b"ITL", *b"JMD", *b"JOD", *b"JPY", *b"JUN", *b"JUP",
    *b"KAS", *b"KAV", *b"KCS", *b"KES", *b"KGS", *b"KHR", *b"KKK", *b"KLY", *b"KMD", *b"KMF",
    *b"KNC", *b"KPW", *b"KRH", *b"KRO", *b"KRW", *b"KSM", *b"KWD", *b"KYD", *b"KZT", *b"LAJ",
    *b"LAK", *b"LBP", *b"LDO", *b"LEO", *b"LKR", *b"LNK", *b"LRC", *b"LRD", *b"LSK", *b"LSL",
    *b"LSM", *b"LTC", *b"LTL", *b"LTT", *b"LUC", *b"LUF", *b"LUL", *b"LUN", *b"LVL", *b"LVR",
    *b"LYD", *b"MAD", *b"MAF", *b"MAN", *b"MAT", *b"MCF", *b"MDC", *b"MDL", *b"MGA", *b"MGF",
    *b"MIN", *b"MIR", *b"MKD", *b"MKN", *b"MKR", *b"MLF", *b"MLN", *b"MMK", *b"MNA", *b"MNT",
    *b"MOP", *b"MRO", *b"MRU", *b"MTL", *b"MTP", *b"MUR", *b"MVP", *b"MVR", *b"MWK", *b"MXN",
    *b"MXP", *b"MXV", *b"MYR", *b"MZE", *b"MZM", *b"MZN", *b"NAD", *b"NEO", *b"NER", *b"NGN",
    *b"NIC", *b"NIO", *b"NLG", *b"NMR", *b"NOK", *b"NOT", *b"NPR", *b"NZD", *b"OGN", *b"OKB",
    *b"OMG", *b"OMR", *b"OND", *b"ONE", *b"ONT", *b"OPX", *b"ORC", *b"OSM", *b"PAB", *b"PAX",
    *b"PAY", *b"PEH", *b"PEI", *b"PEN", *b"PEP", *b"PES", *b"PGK", *b"PHP", *b"PKR", *b"PLN",
    *b"PLZ", *b"PND", *b"POL", *b"PTE", *b"PYG", *b"PYT", *b"QAR", *b"QNT", *b"QTU", *b"RAY",
    *b"RHD", *b"RND", *b"ROI", *b"ROK", *b"ROL", *b"RON", *b"RPL", *b"RSD", *b"RSE", *b"RUB",
    *b"RUN", *b"RUR", *b"RVN", *b"RWF", *b"SAR", *b"SBD", *b"SCR", *b"SDD", *b"SDG", *b"SDP",
    *b"SEC", *b"SEI", *b"SEK", *b"SEX", *b"SGB", *b"SGD", *b"SHB", *b"SHP", *b"SIT", *b"SKK",
    *b"SLE", *b"SLL", *b"SND", *b"SNX", *b"SOL", *b"SOS", *b"SRD", *b"SRG", *b"SRK", *b"SRM",
    *b"SSP", *b"SSV", *b"STD", *b"STN", *b"STX", *b"SUI", *b"SUN", *b"SUR", *b"SUS", *b"SVC",
    *b"SYP", *b"SZL", *b"TAO", *b"TAX", *b"THB", *b"TIA", *b"TJR", *b"TJS", *b"TMM", *b"TMT",
    *b"TND", *b"TON", *b"TOP", *b"TPE", *b"TRL", *b"TRX", *b"TRY", *b"TTD", *b"TWD", *b"TZS",
    *b"UAH", *b"UAK", *b"UGS", *b"UGX", *b"UNI", *b"USC", *b"USD", *b"USN", *b"USS", *b"UST",
    *b"UYI", *b"UYP", *b"UYU", *b"UYW", *b"UZS", *b"VEB", *b"VED", *b"VEF", *b"VEN", *b"VES",
    *b"VET", *b"VNC", *b"VND", *b"VUV", *b"WAR", *b"WAV", *b"WBT", *b"WIF", *b"WIN", *b"WLD",
    *b"WST", *b"WTC", *b"WTF", *b"XAF", *b"XAG", *b"XAH", *b"XAU", *b"XBA", *b"XBB", *b"XBC",
    *b"XBD", *b"XBT", *b"XCD", *b"XCG", *b"XCH", *b"XCN", *b"XDC", *b"XDR", *b"XEC", *b"XEM",
    *b"XEU", *b"XFO", *b"XFU", *b"XLM", *b"XMR", *b"XNO", *b"XOF", *b"XPD", *b"XPF", *b"XPR",
    *b"XPT", *b"XRE", *b"XRP", *b"XSU", *b"XTS", *b"XTZ", *b"XUA", *b"XVG", *b"XXX", *b"YDD",
    *b"YER", *b"YFI", *b"YUD", *b"YUM", *b"YUN", *b"YUR", *b"ZAL", *b"ZAR", *b"ZEC", *b"ZIL",
    *b"ZMK", *b"ZMW", *b"ZRN", *b"ZRX", *b"ZRZ", *b"ZWC", *b"ZWD", *b"ZWG", *b"ZWL", *b"ZWN",
    *b"ZWR",
];

/// Returns `true` when `bytes` is on the real-currency, crypto ticker, or reserved deny-list.
#[must_use]
pub fn is_denied_currency_code(bytes: [u8; 3]) -> bool {
    DENIED_CURRENCY_CODES.binary_search(&bytes).is_ok()
}

impl CurrencyCode {
    /// Constructs a [`CurrencyCode`] if `bytes` are 3 ASCII uppercase letters strictly inside
    /// [`ALLOWED_COMMODITY_GLOSSARY`] and outside [`DENIED_CURRENCY_CODES`].
    pub fn from_bytes(bytes: [u8; 3]) -> Result<Self, FeasibilityError> {
        if !bytes.iter().all(|b| b.is_ascii_uppercase()) {
            return Err(FeasibilityError::InvalidTable(
                "currency code must contain 3 uppercase ASCII letters".to_owned(),
            ));
        }
        if is_denied_currency_code(bytes) {
            return Err(FeasibilityError::InvalidTable(
                "currency code uses a denied real-world currency, crypto ticker, or reserved code"
                    .to_owned(),
            ));
        }
        if !is_allowed_commodity_code(bytes) {
            return Err(FeasibilityError::InvalidTable(
                "currency code is not in the curated game-word allow-list".to_owned(),
            ));
        }
        Ok(Self(bytes))
    }

    /// Returns the 3 ASCII bytes of this currency code.
    #[must_use]
    pub const fn as_bytes(&self) -> [u8; 3] {
        self.0
    }

    /// Formats the 3-character code as a `&str`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        // Construction checks every byte is ASCII uppercase `0x41..=0x5A`, which is valid UTF-8.
        std::str::from_utf8(&self.0).unwrap_or("BAD")
    }
}

impl fmt::Debug for CurrencyCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CurrencyCode({})", self.as_str())
    }
}

impl fmt::Display for CurrencyCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<CurrencyCode> for String {
    fn from(code: CurrencyCode) -> Self {
        code.as_str().to_owned()
    }
}

impl TryFrom<String> for CurrencyCode {
    type Error = FeasibilityError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let bytes = value.as_bytes();
        if bytes.len() != 3 {
            return Err(FeasibilityError::InvalidTable(
                "currency code string must have byte length 3".to_owned(),
            ));
        }
        Self::from_bytes([bytes[0], bytes[1], bytes[2]])
    }
}

/// Kind of 3-stage telegraphed Horizon Shock anchored via `OracleSet`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum HorizonShockKind {
    /// Temporarily cuts Turn `t+2` raw extraction yield on the target seat's good.
    SupplyFreeze,
    /// Temporarily resets the target seat's commodity `TransferRate` tariff to zero.
    TariffHoliday,
    /// Temporarily boosts Turn `t+2`..`t+3` Syndicate Project valuation on the target refined good.
    DemandSurge,
    /// Temporarily elevates end-of-turn perishable `Clawback` spoilage rate on the target good.
    SpoilageSpike,
}

/// One telegraphed shock schedule (`hint_turn -> forecast_turn -> realize_turn`) targeting a seat.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScheduledShock {
    /// Turn when Authority commits the hidden hash and sends the private hint to Oracle Auditor.
    pub hint_turn: u8,
    /// Turn (`hint_turn + 1`) when Authority publishes the narrowed public `OracleSet` forecast.
    pub forecast_turn: u8,
    /// Turn (`hint_turn + 2`) when the shock executes deterministically on-ledger.
    pub realize_turn: u8,
    /// Target seat index (`0..N`) whose raw/refined commodity is affected.
    pub target_seat: u8,
    /// Shock effect category.
    pub kind: HorizonShockKind,
    /// Magnitude parameter in `(0.0, 1.0]` (fraction reduction or bonus weight depending on `kind`).
    pub magnitude: f64,
}

/// One converter recipe owned by a seat on the ring.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConverterSpec {
    /// Seat index (`0..N`) authorized to run this converter.
    pub owner: u8,
    /// Required `(raw_good_index, quantity_per_run)` inputs.
    pub inputs: Vec<(u8, f64)>,
    /// Produced refined good index (`0..N`).
    pub output_good: u8,
    /// Quantity of `output_good` minted per batch run.
    pub output_amount: f64,
    /// Maximum runs per turn for this converter recipe.
    pub max_runs_per_turn: f64,
    /// Whether this is an optional secondary cross-converter (`false` = primary ring converter).
    pub is_secondary: bool,
}

/// Economic parameters for a single seat in a [`RingTable`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeatSpec {
    /// Zero-based seat index (`0..N`).
    pub seat: u8,
    /// Exclusive raw good index extracted by this seat (`seat`).
    pub raw_good: u8,
    /// 3-character ledger IOU currency code for this seat's raw good.
    pub raw_code: CurrencyCode,
    /// Units of `raw_good` extracted per turn.
    pub raw_yield_per_turn: f64,
    /// Exclusive refined good index produced by this seat's converters (`seat`).
    pub refined_good: u8,
    /// 3-character ledger IOU currency code for this seat's refined good.
    pub refined_code: CurrencyCode,
    /// Fundamental Syndicate Project value per unit of `refined_good`.
    pub refined_project_value: f64,
    /// Neighbor refined good owed for upkeep (`(seat + N - 1) % N`).
    pub upkeep_good: u8,
    /// Mandatory units of `upkeep_good` owed at Sweep each turn.
    pub upkeep_amount: f64,
    /// Initial commodity export `TransferRate` tariff (`0.0..=0.35`) on `raw_good`.
    pub raw_export_tariff: f64,
    /// Initial commodity export `TransferRate` tariff (`0.0..=0.35`) on `refined_good`.
    pub refined_export_tariff: f64,
    /// Per-turn perishable `Clawback` spoilage rate (`0.0..1.0`) on unconverted `raw_good`.
    pub perishable_decay_rate: f64,
    /// Converter recipes operated by this seat.
    pub converters: Vec<ConverterSpec>,
}

/// Tier-3 Syndicate Project definition emitted for table setup (`1..=N` chiral pair charters
/// plus reserved id `100` Syndicate Concordat requiring all `N` refined goods).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectSpec {
    /// Project id (`1..=N` for chiral neighbor pairs, `100` for the full-ring Concordat).
    pub id: u8,
    /// Human-readable project charter title.
    pub name: String,
    /// Required refined good inputs `(refined_good_index, code, units_per_batch)`.
    pub required_inputs: Vec<(u8, CurrencyCode, f64)>,
    /// Irreversible Project Equity points minted per completed batch.
    pub output_project_equity: f64,
    /// Optional per-turn completion limit (`None` = unconstrained).
    pub max_completions_per_turn: Option<u32>,
}

/// Terminal hard-credit solvency tier used by Turn-1 Charter Mandates (`MaintainCleanCredit`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CreditRatingTier {
    /// Clean balance sheet with zero hard credit defaults or escrow breaches.
    Aaa = 0,
    /// Minor transitory distress resolved before close.
    Bbb = 1,
    /// Elevated credit stress.
    B = 2,
    /// Hard credit default.
    D = 3,
}

/// Objective predicate for a Turn-1 Private Charter Mandate card, keeping structural thresholds
/// within B1.2's monopoly-safe bounds (`1..=seat_count - 2`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MandateObjective {
    /// Accumulate at least `target` irreversible Project Equity points by season close.
    MinProjectEquity {
        /// Minimum irreversible Project Equity threshold (`> 0`).
        target: f64,
    },
    /// Hold at least `min_each` units of at least `distinct_goods` (`1..=N-2`) refined goods.
    MinRefinedVariety {
        /// Required distinct refined goods held (`1..=seat_count - 2`).
        distinct_goods: u8,
        /// Minimum units required for each counted refined good (`> 0`).
        min_each: f64,
    },
    /// Maintain hard credit solvency at or above `min_tier` (`Aaa` or `Bbb`).
    MaintainCleanCredit {
        /// Cleanest tolerated terminal credit tier.
        min_tier: CreditRatingTier,
    },
    /// Complete positive deliveries with at least `min_partners` (`1..=N-2`) distinct counterparties.
    MinActivePartners {
        /// Required distinct counterparties (`1..=seat_count - 2`).
        min_partners: u8,
    },
}

impl MandateObjective {
    /// Validates objective parameters against `seat_count`, strictly enforcing B1.2's
    /// `1..=seat_count - 2` ceiling on `distinct_goods` and `min_partners`.
    pub fn validate_for_seat_count(&self, seat_count: SeatCount) -> Result<(), FeasibilityError> {
        let max_safe = seat_count.as_u8().saturating_sub(2);
        match *self {
            Self::MinProjectEquity { target } => {
                if !target.is_finite() || target <= 0.0 {
                    return Err(FeasibilityError::InvalidTable(
                        "mandate MinProjectEquity target must be finite and > 0".to_owned(),
                    ));
                }
            }
            Self::MinRefinedVariety {
                distinct_goods,
                min_each,
            } => {
                if distinct_goods == 0 || distinct_goods > max_safe {
                    return Err(FeasibilityError::InvalidTable(format!(
                        "mandate MinRefinedVariety distinct_goods {distinct_goods} outside 1..={max_safe}"
                    )));
                }
                if !min_each.is_finite() || min_each <= 0.0 {
                    return Err(FeasibilityError::InvalidTable(
                        "mandate MinRefinedVariety min_each must be finite and > 0".to_owned(),
                    ));
                }
            }
            Self::MaintainCleanCredit { min_tier } => {
                if matches!(min_tier, CreditRatingTier::B | CreditRatingTier::D) {
                    return Err(FeasibilityError::InvalidTable(
                        "mandate MaintainCleanCredit must require Aaa or Bbb solvency".to_owned(),
                    ));
                }
            }
            Self::MinActivePartners { min_partners } => {
                if min_partners == 0 || min_partners > max_safe {
                    return Err(FeasibilityError::InvalidTable(format!(
                        "mandate MinActivePartners min_partners {min_partners} outside 1..={max_safe}"
                    )));
                }
            }
        }
        Ok(())
    }
}

/// One card in the public Turn-1 graded Charter Mandate deck (`3.0..=8.0 VP` scale per T5 / A7).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MandateCardSpec {
    /// Stable deck slot identifier (`1..=M`).
    pub card_id: u8,
    /// Structural win-condition objective.
    pub kind: MandateObjective,
    /// Provisional or rollout-measured completion frequency (`0..=10_000 bps`).
    pub difficulty_completion_bps: u16,
    /// Graded Victory Point bonus in `[MIN_MANDATE_BONUS_VP, MAX_MANDATE_BONUS_VP]` (`3.0..=8.0`).
    pub bonus_vp: f64,
}

/// Maps empirical/structural completion frequency (`0..=10_000 bps`) monotonically onto
/// `[MIN_MANDATE_BONUS_VP, MAX_MANDATE_BONUS_VP]` (`3.0..=8.0 VP`), delegating directly to
/// [`league_core::bonus_points_milli_for_completion_bps`] (`#32` `B-2`).
#[must_use]
pub fn bonus_vp_for_completion_bps(completion_bps: u16) -> f64 {
    (league_core::bonus_points_milli_for_completion_bps(completion_bps) as f64) / 1_000.0
}

impl From<league_core::CreditTier> for CreditRatingTier {
    fn from(tier: league_core::CreditTier) -> Self {
        match tier {
            league_core::CreditTier::Aaa => Self::Aaa,
            league_core::CreditTier::Bbb => Self::Bbb,
            league_core::CreditTier::B => Self::B,
            league_core::CreditTier::D => Self::D,
        }
    }
}

impl From<CreditRatingTier> for league_core::CreditTier {
    fn from(tier: CreditRatingTier) -> Self {
        match tier {
            CreditRatingTier::Aaa => Self::Aaa,
            CreditRatingTier::Bbb => Self::Bbb,
            CreditRatingTier::B => Self::B,
            CreditRatingTier::D => Self::D,
        }
    }
}

impl From<league_core::MandateKind> for MandateObjective {
    fn from(kind: league_core::MandateKind) -> Self {
        match kind {
            league_core::MandateKind::MinProjectEquity { target } => Self::MinProjectEquity {
                target: (target as f64) / 1_000.0,
            },
            league_core::MandateKind::MinRefinedVariety {
                distinct_goods,
                min_each,
            } => Self::MinRefinedVariety {
                distinct_goods,
                min_each: (min_each as f64) / 1_000.0,
            },
            league_core::MandateKind::MaintainCleanCredit { min_tier } => {
                Self::MaintainCleanCredit {
                    min_tier: CreditRatingTier::from(min_tier),
                }
            }
            league_core::MandateKind::MinActivePartners { min_partners } => {
                Self::MinActivePartners { min_partners }
            }
        }
    }
}

impl From<league_core::GradedMandateCard> for MandateCardSpec {
    fn from(card: league_core::GradedMandateCard) -> Self {
        Self {
            card_id: u8::try_from(card.card_id).unwrap_or(u8::MAX),
            kind: MandateObjective::from(card.kind),
            difficulty_completion_bps: card.difficulty_completion_bps,
            bonus_vp: (card.bonus_equity as f64) / 1_000.0,
        }
    }
}

impl MandateCardSpec {
    /// Converts this schema card back into [`league_core::GradedMandateCard`] on the exact
    /// milli-unit integer grid (`#32` `B-2`).
    #[must_use]
    pub fn to_core_card(&self) -> league_core::GradedMandateCard {
        let kind = match self.kind {
            MandateObjective::MinProjectEquity { target } => {
                league_core::MandateKind::MinProjectEquity {
                    target: (target * 1_000.0).round() as league_core::Units,
                }
            }
            MandateObjective::MinRefinedVariety {
                distinct_goods,
                min_each,
            } => league_core::MandateKind::MinRefinedVariety {
                distinct_goods,
                min_each: (min_each * 1_000.0).round() as league_core::Units,
            },
            MandateObjective::MaintainCleanCredit { min_tier } => {
                league_core::MandateKind::MaintainCleanCredit {
                    min_tier: league_core::CreditTier::from(min_tier),
                }
            }
            MandateObjective::MinActivePartners { min_partners } => {
                league_core::MandateKind::MinActivePartners { min_partners }
            }
        };
        league_core::GradedMandateCard {
            card_id: u16::from(self.card_id),
            kind,
            difficulty_completion_bps: self.difficulty_completion_bps,
            bonus_equity: (self.bonus_vp * 1_000.0).round() as league_core::Units,
        }
    }
}

/// Deals `seat_count` distinct [`MandateCardSpec`] indices using [`league_core::deal_mandate_deck_indices`]
/// (`#32` `B-2`, `B1.3`/`M3`), strictly separate from the public table `seed: u64` and rejecting `[0u8; 32]`.
pub fn deal_mandate_indices_with_entropy(
    mandate_deck: &[MandateCardSpec],
    seat_count: SeatCount,
    deal_seed: [u8; 32],
) -> Result<Vec<usize>, FeasibilityError> {
    league_core::deal_mandate_deck_indices(
        league_core::TableId(0),
        seat_count.as_u8(),
        deal_seed,
        mandate_deck.len(),
    )
    .map_err(|err| FeasibilityError::InvalidTable(err.to_string()))
}

fn round3_schema(v: f64) -> f64 {
    (v * 1_000.0).round() / 1_000.0
}

/// Builds canonical Tier-3 Syndicate Projects for `seats`: `1..=N` adjacent chiral neighbor
/// pairs plus `100` Syndicate Concordat requiring all `N` distinct refined goods (`C1` held rule).
#[must_use]
pub fn default_projects_for_seats(seats: &[SeatSpec]) -> Vec<ProjectSpec> {
    let n = seats.len();
    let mut projects = Vec::with_capacity(n + 1);
    for i in 0..n {
        let j = (i + 1) % n;
        let eq =
            round3_schema((seats[i].refined_project_value + seats[j].refined_project_value) * 1.05);
        projects.push(ProjectSpec {
            id: (i + 1) as u8,
            name: format!("Pair Charter {}-{}", i + 1, j + 1),
            required_inputs: vec![
                (i as u8, seats[i].refined_code, 1.0),
                (j as u8, seats[j].refined_code, 1.0),
            ],
            output_project_equity: eq,
            max_completions_per_turn: None,
        });
    }
    let concordat_eq =
        round3_schema(seats.iter().map(|s| s.refined_project_value).sum::<f64>() * 1.15);
    let concordat_inputs = (0..n)
        .map(|g| (g as u8, seats[g].refined_code, 1.0))
        .collect();
    projects.push(ProjectSpec {
        id: PROJECT_CONCORDAT_ID,
        name: "Syndicate Concordat".to_owned(),
        required_inputs: concordat_inputs,
        output_project_equity: concordat_eq,
        max_completions_per_turn: None,
    });
    projects
}

/// Builds the canonical 8-card Turn-1 public graded Charter Mandate deck (`3.0..=8.0 VP` scale
/// per `T5`/`A7`), delegating directly to [`league_core::graded_mandate_deck_for_mean_project_value`]
/// so `league-gen` and `league-core` share a single mandate deck source (`#32` `B-2`).
#[must_use]
pub fn default_mandate_deck_for_table(
    seat_count: SeatCount,
    seats: &[SeatSpec],
) -> Vec<MandateCardSpec> {
    let mean_milli = if seats.is_empty() {
        0
    } else {
        let sum: f64 = seats.iter().map(|s| s.refined_project_value).sum();
        let mean = sum / (seats.len() as f64);
        ((mean * (league_core::UNIT_SCALE as f64)).round() as league_core::Units).max(1)
    };
    league_core::graded_mandate_deck_for_mean_project_value(seat_count.as_u8(), mean_milli)
        .into_iter()
        .map(MandateCardSpec::from)
        .collect()
}

/// Complete specification for a generated chiral converter-ring table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RingTable {
    /// Generator version string (`GENERATOR_VERSION`) recording exact sampling/filter rules.
    pub generator_version: String,
    /// Full [`GeneratorConfig`] active when this table was sampled so runs stay comparable.
    pub config: GeneratorConfig,
    /// Seed passed to [`crate::generate_ring_table`].
    pub seed: u64,
    /// Zero-based search attempt index that passed the LP feasibility filter.
    pub attempt: u32,
    /// Validated number of seats (`4`, `5`, or `6`).
    pub seat_count: SeatCount,
    /// Fixed table numeraire currency code (`"CRED"` per `docs/design-v2.md` §1.2).
    pub numeraire_code: String,
    /// Inclusive turn bounds `(min_close, max_close)` for the stochastic season close (`(24, 28)`).
    pub season_turn_window: (u8, u8),
    /// Per-seat economic definitions in index order `0..N`.
    pub seats: Vec<SeatSpec>,
    /// Tier-3 Syndicate Projects (`1..=N` chiral pairs plus `100` full-ring Concordat).
    pub projects: Vec<ProjectSpec>,
    /// Public Turn-1 graded Charter Mandate deck (`3.0..=8.0 VP` scale).
    pub mandate_deck: Vec<MandateCardSpec>,
    /// Pre-sampled 3-stage `OracleSet` horizon shock schedule for the season.
    pub shocks: Vec<ScheduledShock>,
}

/// Parameters controlling chiral table generation and LP feasibility filtering.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeneratorConfig {
    /// Optional fixed seat count (`None` draws uniformly once per seed over `4`, `5`, `6`,
    /// per `docs/design-v2.md` §1.5 `N = 4, 5, 6`).
    pub seat_count: Option<SeatCount>,
    /// Maximum allowed relative deviation of any seat's EV from the table mean (`0.03` = ±3%,
    /// from `docs/design-v2.md` §1.5 item 3 and seed-26 report §"Static board layouts…").
    pub ev_tolerance: f64,
    /// Minimum required `(net_converter_surplus_value / aggregate_upkeep_value)` margin (`0.15`;
    /// provisional, calibrate in playtest 1 — `docs/design-v2.md` §1.5 requires a positive margin).
    pub min_surplus_ratio: f64,
    /// Whether 4-, 5-, and 6-seat tables attach binding primary workshops paired with cyclic
    /// secondary cross-converters (`(R_{i+2}, R_{i+3}) -> M_i` on 4p; binding 3-input
    /// `(R_{i+1}, R_{i+2}, R_{i+3})` plus clearing `(R_{i+2}, R_{i+4})` on 5p so every 4-seat bloc
    /// covers at most 2/4 upkeep; binding `(R_{i+1}, R_{i+3})` plus clearing `(R_{i+2}, R_{i+4})`
    /// on 6p; `true` per `docs/design-v2.md` §1.3 & §1.5 and the 2026-11-15 rulings; provisional
    /// calibration, calibrate in playtest 1).
    pub enable_secondary_converters: bool,
    /// Upper bound on initial baseline export `TransferRate` tariffs (`0.20` = 20% from
    /// `docs/design-v2.md` §1.5 item 3 "0%–20% initial baseline", under the §5.1 35% cap).
    pub max_tariff: f64,
    /// Inclusive `(min, max)` bounds for initial perishable `Clawback` decay rates (`(0.15, 0.25)`,
    /// provisional ±5% band around the `20%`/turn baseline in `docs/design-v2.md` §1.4).
    pub decay_range: (f64, f64),
    /// Maximum candidate sampling attempts before returning [`GenError::NoFeasibleTableFound`]
    /// (`512`, capped at [`MAX_ATTEMPTS_CAP`]).
    pub max_attempts: u32,
    /// Seat expected-value allocation method for Invariant 3 ([`EvMethod::ShadowPrice`] default
    /// per `docs/design-v2.md` §1.5 "optimal competitive/cooperative shadow-price allocation").
    pub ev_method: EvMethod,
    /// Optional override for the provisional C6 table-level top-pair share cap (`None` uses
    /// [`SeatCount::default_max_pair_share_cap`]: `4p: 0.45`, `5p: 0.35`, `6p: 0.30`; the 2026-11-15
    /// ruling gave no numeric cap values and these defaults do not bind on generated tables today).
    pub max_pair_share_cap: Option<f64>,
}

impl Default for GeneratorConfig {
    /// Defaults combine `docs/design-v2.md` §1.4–§1.5 baselines with Director v2 rulings (`G1`/`C6`):
    /// - `seat_count: None` (`docs/design-v2.md` §1.5: uniform draw over `N in {4, 5, 6}`)
    /// - `ev_tolerance: 0.03` (`docs/design-v2.md` §1.5 item 3 & seed-26 report)
    /// - `min_surplus_ratio: 0.15` (provisional, calibrate in playtest 1)
    /// - `enable_secondary_converters: true` (`docs/design-v2.md` §1.3 & §1.5)
    /// - `max_tariff: 0.20` (`docs/design-v2.md` §1.5 item 3: 0%–20% initial baseline)
    /// - `decay_range: (0.15, 0.25)` (provisional ±5% band around `docs/design-v2.md` §1.4 20%/turn)
    /// - `max_attempts: 512` (stationary sampling retry cap; G1 removes dispersion annealing)
    /// - `ev_method: EvMethod::ShadowPrice` (`docs/design-v2.md` §1.5 shadow-price allocation)
    /// - `max_pair_share_cap: None` (provisional defaults: `4p: 0.45`, `5p: 0.35`, `6p: 0.30`;
    ///   2026-11-15 ruling gave no values and these caps do not bind on generated tables today)
    fn default() -> Self {
        Self {
            seat_count: None,
            ev_tolerance: 0.03,
            min_surplus_ratio: 0.15,
            enable_secondary_converters: true,
            max_tariff: 0.20,
            decay_range: (0.15, 0.25),
            max_attempts: 512,
            ev_method: EvMethod::ShadowPrice,
            max_pair_share_cap: None,
        }
    }
}

/// Reasons a candidate [`RingTable`] can fail structural or LP feasibility checks.
#[derive(Debug, Clone, PartialEq)]
pub enum FeasibilityError {
    /// Candidate table failed structural validation (indices, codes, positive quantities, shock timing,
    /// converter positive clearing value-added, projects, or mandate bounds).
    InvalidTable(String),
    /// At least one 1-seat or 2-seat subset satisfied non-zero internal upkeep without external trade.
    PairCoversUpkeep {
        /// Ordered pair `(i, j)` of seats (`i <= j`).
        pair: (u8, u8),
        /// Maximum fraction of the pair's combined upkeep satisfied internally.
        coverage_ratio: f64,
    },
    /// Aggregate table production either cannot cover mandatory upkeep or lacks required surplus.
    InsufficientSurplus {
        /// Net refined surplus value remaining after full table upkeep (negative if deficit).
        net_surplus: f64,
        /// Minimum required surplus ratio (`config.min_surplus_ratio`).
        required_ratio: f64,
        /// Measured `(net_surplus / total_upkeep_value)` ratio (or `0.0` if infeasible).
        actual_ratio: f64,
    },
    /// At least one seat's EV falls outside `±ev_tolerance` of the table mean.
    SeatEvOutOfTolerance {
        /// Maximum relative deviation `|(seat_ev - mean_ev) / mean_ev|` observed.
        max_relative_deviation: f64,
        /// Maximum allowed relative deviation (`config.ev_tolerance`).
        tolerance: f64,
        /// Per-seat expected values under the active [`EvMethod`].
        seat_evs: Vec<f64>,
    },
    /// Optimal ring LP trade share on a single unordered pair exceeded the C6 table cap.
    PairShareExceedsCap {
        /// Ordered seat pair `(i, j)` with `i < j`.
        pair: (u8, u8),
        /// Observed share of total bilateral volume carried by `(i, j)`.
        share: f64,
        /// Enforced upper cap (`0.45 / 0.35 / 0.30` or custom override).
        cap: f64,
    },
    /// Deterministic multi-agent rollout grading failed B2.2 acceptance checks.
    RolloutGateFailed(String),
    /// Underlying simplex LP solver hit an unexpected numerical or unbounded state.
    SolverFailure(String),
}

impl fmt::Display for FeasibilityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTable(msg) => write!(f, "invalid ring table: {msg}"),
            Self::PairCoversUpkeep {
                pair,
                coverage_ratio,
            } => write!(
                f,
                "2-seat pair ({}, {}) covers {:.4} of its own upkeep (must be 0.0)",
                pair.0, pair.1, coverage_ratio
            ),
            Self::InsufficientSurplus {
                net_surplus,
                required_ratio,
                actual_ratio,
            } => write!(
                f,
                "insufficient aggregate converter surplus: net={net_surplus:.4}, ratio={actual_ratio:.4} < {required_ratio:.4}"
            ),
            Self::SeatEvOutOfTolerance {
                max_relative_deviation,
                tolerance,
                ..
            } => write!(
                f,
                "seat EV deviation {max_relative_deviation:.4} exceeds tolerance {tolerance:.4}"
            ),
            Self::PairShareExceedsCap { pair, share, cap } => write!(
                f,
                "pair ({}, {}) volume share {share:.4} exceeds cap {cap:.4}",
                pair.0, pair.1
            ),
            Self::RolloutGateFailed(msg) => write!(f, "rollout grading failed: {msg}"),
            Self::SolverFailure(msg) => write!(f, "LP solver error: {msg}"),
        }
    }
}

impl std::error::Error for FeasibilityError {}

/// Errors returned by table generation and feasibility configuration checks.
#[derive(Debug, Clone, PartialEq)]
pub enum GenError {
    /// Generator configuration contained invalid bounds, zero attempts, or exceeded `MAX_ATTEMPTS_CAP`.
    InvalidConfig(String),
    /// No feasible table passed all three LP invariants within `max_attempts`.
    NoFeasibleTableFound {
        /// Number of attempts tested.
        attempts: u32,
        /// Failure recorded on the final attempt.
        last_error: FeasibilityError,
    },
}

impl fmt::Display for GenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(msg) => write!(f, "invalid generator config: {msg}"),
            Self::NoFeasibleTableFound {
                attempts,
                last_error,
            } => write!(
                f,
                "no feasible chiral table found in {attempts} attempts (last: {last_error})"
            ),
        }
    }
}

impl std::error::Error for GenError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seat_count_and_currency_code_validation() {
        assert_eq!(SeatCount::from_usize(3), None);
        assert_eq!(SeatCount::from_usize(4), Some(SeatCount::Four));
        assert_eq!(SeatCount::from_usize(5), Some(SeatCount::Five));
        assert_eq!(SeatCount::from_usize(6), Some(SeatCount::Six));
        assert_eq!(SeatCount::from_usize(7), None);
        assert_eq!(SeatCount::Five.as_u8(), 5);
        assert_eq!(SeatCount::Five.pair_count(), 10);
        assert_eq!(SeatCount::Six.pair_count(), 15);

        let Ok(code) = CurrencyCode::from_bytes(*b"SLT") else {
            panic!("SLT must be valid");
        };
        assert_eq!(code.as_str(), "SLT");
        assert_eq!(code.to_string(), "SLT");
        assert_eq!(format!("{code:?}"), "CurrencyCode(SLT)");
        let s: String = code.into();
        assert_eq!(s, "SLT");
        assert_eq!(CurrencyCode::try_from("SLT".to_owned()), Ok(code));

        assert!(CurrencyCode::from_bytes(*b"slt").is_err());
        assert!(CurrencyCode::from_bytes(*b"XRP").is_err());
        assert!(CurrencyCode::try_from("AB".to_owned()).is_err());
    }

    #[test]
    fn denied_currency_codes_are_strictly_sorted_and_reject_real_and_crypto_tickers() {
        assert!(
            DENIED_CURRENCY_CODES
                .windows(2)
                .all(|pair| pair[0] < pair[1]),
            "DENIED_CURRENCY_CODES must remain strictly sorted for binary_search"
        );
        for ticker in [
            *b"XRP", *b"BTC", *b"ETH", *b"SOL", *b"XLM", *b"DOT", *b"KGS", *b"AZN", *b"BHD",
            *b"TWD", *b"CZK", *b"NGN", *b"DZD", *b"USD", *b"EUR", *b"GBP", *b"JPY", *b"CHF",
            *b"XAU", *b"XAG", *b"CRD", *b"LEO", *b"BSV", *b"FTT", *b"FLR", *b"SGB", *b"EVR",
            *b"GBX", *b"ELS", *b"ANC",
        ] {
            assert!(is_denied_currency_code(ticker));
            assert!(CurrencyCode::from_bytes(ticker).is_err());
        }
        assert_eq!(NUMERAIRE_CODE, "CRED");
        assert_eq!(EvMethod::default(), EvMethod::ShadowPrice);
    }

    #[test]
    fn allowed_commodity_glossary_is_strictly_sorted_disjoint_and_wide() {
        assert!(
            ALLOWED_COMMODITY_GLOSSARY.len() >= 60,
            "glossary must hold at least 60 allow-listed goods (has {})",
            ALLOWED_COMMODITY_GLOSSARY.len()
        );
        assert!(
            ALLOWED_COMMODITY_GLOSSARY
                .windows(2)
                .all(|pair| pair[0].0 < pair[1].0),
            "ALLOWED_COMMODITY_GLOSSARY must remain strictly sorted for binary_search"
        );
        for &(code, word) in ALLOWED_COMMODITY_GLOSSARY {
            assert!(
                code.iter().all(|b| b.is_ascii_uppercase()),
                "allow-list code {code:?} must be uppercase ASCII"
            );
            assert!(
                !word.is_empty(),
                "allow-list code {code:?} must have non-empty word"
            );
            assert!(
                !is_denied_currency_code(code),
                "allow-list code {code:?} collides with DENIED_CURRENCY_CODES"
            );
            let Ok(parsed) = CurrencyCode::from_bytes(code) else {
                panic!("allow-listed code {code:?} must parse");
            };
            assert_eq!(commodity_glossary_word(parsed), Some(word));
        }
        // Arbitrary non-allow-listed 3-letter codes are rejected even when absent from deny-list.
        assert!(CurrencyCode::from_bytes(*b"QRA").is_err());
        assert!(CurrencyCode::from_bytes(*b"ZZQ").is_err());
    }

    #[test]
    fn seat_count_pair_share_caps_and_mandate_validation() {
        assert!((SeatCount::Four.default_max_pair_share_cap() - 0.45).abs() < 1e-12);
        assert!((SeatCount::Five.default_max_pair_share_cap() - 0.35).abs() < 1e-12);
        assert!((SeatCount::Six.default_max_pair_share_cap() - 0.30).abs() < 1e-12);
        assert!((bonus_vp_for_completion_bps(0) - MAX_MANDATE_BONUS_VP).abs() < 1e-12);
        assert!((bonus_vp_for_completion_bps(BPS_SCALE) - MIN_MANDATE_BONUS_VP).abs() < 1e-12);

        for sc in SeatCount::ALL {
            let max_safe = sc.as_u8() - 2;
            assert!(
                MandateObjective::MinRefinedVariety {
                    distinct_goods: max_safe,
                    min_each: 1.5,
                }
                .validate_for_seat_count(sc)
                .is_ok()
            );
            assert!(
                MandateObjective::MinRefinedVariety {
                    distinct_goods: 0,
                    min_each: 1.5,
                }
                .validate_for_seat_count(sc)
                .is_err()
            );
            assert!(
                MandateObjective::MinRefinedVariety {
                    distinct_goods: max_safe + 1,
                    min_each: 1.5,
                }
                .validate_for_seat_count(sc)
                .is_err()
            );
            assert!(
                MandateObjective::MinActivePartners {
                    min_partners: max_safe + 1,
                }
                .validate_for_seat_count(sc)
                .is_err()
            );
        }
    }

    #[test]
    fn mandate_entropy_deal_rejects_zero_seed_and_deals_distinct_cards() {
        let deck: Vec<MandateCardSpec> = (1..=8u8)
            .map(|id| MandateCardSpec {
                card_id: id,
                kind: MandateObjective::MaintainCleanCredit {
                    min_tier: CreditRatingTier::Bbb,
                },
                difficulty_completion_bps: 5_000,
                bonus_vp: bonus_vp_for_completion_bps(5_000),
            })
            .collect();
        assert!(deal_mandate_indices_with_entropy(&deck, SeatCount::Five, [0u8; 32]).is_err());

        let Ok(deal) = deal_mandate_indices_with_entropy(&deck, SeatCount::Six, [0xA5u8; 32])
        else {
            panic!("non-zero 256-bit entropy deals cleanly");
        };
        assert_eq!(deal.len(), 6);
        let uniq: std::collections::BTreeSet<usize> = deal.iter().copied().collect();
        assert_eq!(uniq.len(), 6);
    }

    #[test]
    fn error_display_messages_are_non_empty() {
        let errs = [
            FeasibilityError::InvalidTable("bad".to_owned()),
            FeasibilityError::PairCoversUpkeep {
                pair: (0, 1),
                coverage_ratio: 0.5,
            },
            FeasibilityError::InsufficientSurplus {
                net_surplus: -1.0,
                required_ratio: 0.15,
                actual_ratio: 0.0,
            },
            FeasibilityError::SeatEvOutOfTolerance {
                max_relative_deviation: 0.08,
                tolerance: 0.03,
                seat_evs: vec![10.0, 12.0],
            },
            FeasibilityError::PairShareExceedsCap {
                pair: (0, 1),
                share: 0.52,
                cap: 0.45,
            },
            FeasibilityError::RolloutGateFailed("low completion".to_owned()),
            FeasibilityError::SolverFailure("singular".to_owned()),
        ];
        for err in &errs {
            assert!(!err.to_string().is_empty());
        }
        let g1 = GenError::InvalidConfig("bad cfg".to_owned());
        let g2 = GenError::NoFeasibleTableFound {
            attempts: 5,
            last_error: errs[0].clone(),
        };
        assert!(!g1.to_string().is_empty());
        assert!(!g2.to_string().is_empty());
    }
}
