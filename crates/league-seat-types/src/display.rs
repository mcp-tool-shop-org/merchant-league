use alloc::borrow::ToOwned;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

use crate::types::AssetId;

/// Fixed ledger numeraire currency code (`docs/design-v2.md` §1.2).
pub const NUMERAIRE_CODE: &str = "CRED";

/// Default numeraire display label shown when in-world names are enabled.
pub const NUMERAIRE_DISPLAY_LABEL: &str = "Clearinghouse Marks [CRED]";

/// Plain-code numeraire label shown when `--plain-codes` / plain-code mode is toggled on.
pub const NUMERAIRE_PLAIN_LABEL: &str = "CRED";

/// Role-qualified display metadata for one allow-listed 3-letter commodity code (`ALM..ZNC`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommodityDisplayEntry {
    /// Three uppercase ASCII bytes (`[A-Z]{3}`).
    pub code_bytes: [u8; 3],
    /// Three-letter uppercase commodity ticker as `&'static str`.
    pub code: &'static str,
    /// Underlying curated glossary noun.
    pub glossary_word: &'static str,
    /// In-world display name for the perishable extracted `Raw` form.
    pub raw_name: &'static str,
    /// In-world display name for the manufactured `Refined` form.
    pub refined_name: &'static str,
}

impl CommodityDisplayEntry {
    /// Formats `Raw` display label (`"Rough Timber [TMB]"` or `"TMB"` when `plain_codes` is set).
    #[must_use]
    pub fn raw_display(&self, plain_codes: bool) -> String {
        if plain_codes {
            self.code.to_owned()
        } else {
            format!("{} [{}]", self.raw_name, self.code)
        }
    }

    /// Formats `Refined` display label (`"Seasoned Timber [TMB]"` or `"TMB"` when `plain_codes` is set).
    #[must_use]
    pub fn refined_display(&self, plain_codes: bool) -> String {
        if plain_codes {
            self.code.to_owned()
        } else {
            format!("{} [{}]", self.refined_name, self.code)
        }
    }
}

const fn entry(
    code_bytes: [u8; 3],
    code: &'static str,
    glossary_word: &'static str,
    raw_name: &'static str,
    refined_name: &'static str,
) -> CommodityDisplayEntry {
    CommodityDisplayEntry {
        code_bytes,
        code,
        glossary_word,
        raw_name,
        refined_name,
    }
}

/// Canonical compile-time catalog of all `61` allow-listed commodity display names (`ALM..ZNC`).
pub const COMMODITY_DISPLAY_CATALOG: [CommodityDisplayEntry; 61] = [
    entry(*b"ALM", "ALM", "alum", "Raw Alum", "Refined Alum"),
    entry(
        *b"ANV",
        "ANV",
        "anvil",
        "Forged Iron Anvil",
        "Tempered Steel Anvil",
    ),
    entry(
        *b"BBL",
        "BBL",
        "barrel",
        "Unbound Barrel Staves",
        "Hooped Guild Barrel",
    ),
    entry(
        *b"BLW",
        "BLW",
        "bellows",
        "Hide Bellows",
        "Brass-Nozzle Bellows",
    ),
    entry(*b"BLY", "BLY", "barley", "Field Barley", "Kilned Barley"),
    entry(
        *b"BRS",
        "BRS",
        "brass",
        "Raw Brass Ingot",
        "Burnished Brass Plate",
    ),
    entry(
        *b"CBL",
        "CBL",
        "cobble",
        "Rough Quarry Cobble",
        "Dressed Road Cobble",
    ),
    entry(
        *b"CDR",
        "CDR",
        "cedar",
        "Felled Cedar Log",
        "Planed Cedar Beam",
    ),
    entry(
        *b"CHK",
        "CHK",
        "chalk",
        "Pit Chalk Lump",
        "Sifted Mason Chalk",
    ),
    entry(*b"CLY", "CLY", "clay", "River Clay", "Wedged Potter Clay"),
    entry(
        *b"CNV",
        "CNV",
        "canvas",
        "Unbleached Canvas",
        "Waxed Sail Canvas",
    ),
    entry(
        *b"CPR",
        "CPR",
        "copper",
        "Raw Copper Ore",
        "Smelted Copper Bar",
    ),
    entry(
        *b"CRL",
        "CRL",
        "coral",
        "Raw Sea Coral",
        "Polished Coral Bead",
    ),
    entry(
        *b"DYE",
        "DYE",
        "dye",
        "Crushed Woad Dye",
        "Steeped Fast Dye",
    ),
    entry(
        *b"ELM",
        "ELM",
        "elm",
        "Green Elm Log",
        "Seasoned Elm Wheelwood",
    ),
    entry(
        *b"ENM",
        "ENM",
        "enamel",
        "Ground Enamel Frit",
        "Vitrified Guild Enamel",
    ),
    entry(
        *b"FIR",
        "FIR",
        "fir",
        "Green Fir Timber",
        "Seasoned Fir Spar",
    ),
    entry(
        *b"FLN",
        "FLN",
        "flint",
        "Rough Flint Nodule",
        "Knapped Strike Flint",
    ),
    entry(
        *b"GLS",
        "GLS",
        "glass",
        "Cullet Glass",
        "Crown Window Glass",
    ),
    entry(
        *b"GLZ",
        "GLZ",
        "glaze",
        "Raw Ash Glaze",
        "Sieved Ceramic Glaze",
    ),
    entry(
        *b"HMP",
        "HMP",
        "hemp",
        "Retted Hemp Fiber",
        "Hackled Guild Hemp",
    ),
    entry(
        *b"HNY",
        "HNY",
        "honey",
        "Comb Honey",
        "Strained Amber Honey",
    ),
    entry(*b"HPS", "HPS", "hops", "Green Hop Cones", "Kiln-Dried Hops"),
    entry(
        *b"HRB",
        "HRB",
        "herb",
        "Wild Bundle Herb",
        "Dried Apothecary Herb",
    ),
    entry(
        *b"IRN",
        "IRN",
        "iron",
        "Pig Iron Bloom",
        "Wrought Guild Iron",
    ),
    entry(*b"IVR", "IVR", "ivory", "Unworked Ivory", "Carved Ivory"),
    entry(
        *b"JDE",
        "JDE",
        "jade",
        "Rough Jade Pebble",
        "Carved Jade Seal",
    ),
    entry(
        *b"KLN",
        "KLN",
        "kiln",
        "Firebrick Kiln Clay",
        "Refractory Kiln Brick",
    ),
    entry(
        *b"KLP",
        "KLP",
        "kelp",
        "Wet Shore Kelp",
        "Calcined Kelp Soda",
    ),
    entry(
        *b"LOM",
        "LOM",
        "loom",
        "Hewn Loom Frame",
        "Strung Broad Loom",
    ),
    entry(
        *b"LTH",
        "LTH",
        "leather",
        "Brined Raw Leather",
        "Curried Harness Leather",
    ),
    entry(
        *b"LYE",
        "LYE",
        "lye",
        "Leached Wood Ash Lye",
        "Clarified Soapmaker Lye",
    ),
    entry(
        *b"MLT",
        "MLT",
        "malt",
        "Steeped Green Malt",
        "Pale Kilned Malt",
    ),
    entry(
        *b"MRB",
        "MRB",
        "marble",
        "Rough Marble Block",
        "Honed Ashlar Marble",
    ),
    entry(
        *b"NTR",
        "NTR",
        "nitre",
        "Crude Nitre Crust",
        "Recrystallized Nitre",
    ),
    entry(
        *b"OAK",
        "OAK",
        "oak",
        "Green Oak Heartwood",
        "Seasoned Cooper Oak",
    ),
    entry(
        *b"OAT",
        "OAT",
        "oat",
        "Threshed Field Oat",
        "Rolled Mill Oat",
    ),
    entry(
        *b"OLV",
        "OLV",
        "olive",
        "Pressed Green Olive",
        "Cured Brine Olive",
    ),
    entry(
        *b"PCH",
        "PCH",
        "pitch",
        "Pine Tar Pitch",
        "Boiled Shipwright Pitch",
    ),
    entry(
        *b"PLK",
        "PLK",
        "plank",
        "Rough-Sawn Plank",
        "Planed Deck Plank",
    ),
    entry(
        *b"PWT",
        "PWT",
        "pewter",
        "Crude Pewter Alloy",
        "Spun Guild Pewter",
    ),
    entry(
        *b"QRZ",
        "QRZ",
        "quartz",
        "Vein Quartz Chert",
        "Crushed Glassmaker Quartz",
    ),
    entry(
        *b"ROP",
        "ROP",
        "rope",
        "Spun Yarn Cordage",
        "Tarred Hawser Rope",
    ),
    entry(
        *b"RSN",
        "RSN",
        "resin",
        "Scraped Pine Resin",
        "Distilled Clear Resin",
    ),
    entry(
        *b"RYE",
        "RYE",
        "rye",
        "Winnowed Winter Rye",
        "Bolted Rye Meal",
    ),
    entry(
        *b"SLG",
        "SLG",
        "slag",
        "Furnace Iron Slag",
        "Crushed Roadbed Slag",
    ),
    entry(
        *b"SLK",
        "SLK",
        "silk",
        "Reeled Raw Silk",
        "Thrown Guild Silk",
    ),
    entry(*b"SLT", "SLT", "salt", "Coarse Pan Salt", "Cured Fine Salt"),
    entry(
        *b"SPN",
        "SPN",
        "spindle",
        "Turned Oak Spindle",
        "Weighted Guild Spindle",
    ),
    entry(
        *b"SUF",
        "SUF",
        "sulfur",
        "Brimstone Sulfur Ore",
        "Sublimed Flour Sulfur",
    ),
    entry(
        *b"TLW",
        "TLW",
        "tallow",
        "Rendered Rough Tallow",
        "Clarified Candle Tallow",
    ),
    entry(*b"TMB", "TMB", "timber", "Rough Timber", "Seasoned Timber"),
    entry(
        *b"TNG",
        "TNG",
        "tongs",
        "Rough-Forged Tongs",
        "Balanced Smithy Tongs",
    ),
    entry(
        *b"TRF",
        "TRF",
        "turf",
        "Cut Moorland Turf",
        "Stacked Dried Turf",
    ),
    entry(
        *b"VLM",
        "VLM",
        "vellum",
        "Limed Calfskin Vellum",
        "Pumiced Scribe Vellum",
    ),
    entry(
        *b"VLV",
        "VLV",
        "velvet",
        "Loom-Cut Velvet",
        "Sheared Guild Velvet",
    ),
    entry(
        *b"WHT",
        "WHT",
        "wheat",
        "Sheaved Red Wheat",
        "Milled Fine Wheat",
    ),
    entry(
        *b"WLK",
        "WLK",
        "whelk",
        "Fresh Tidal Whelk",
        "Steeped Tyrian Whelk",
    ),
    entry(
        *b"WOL",
        "WOL",
        "wool",
        "Washed Fleece Wool",
        "Combed Worsted Wool",
    ),
    entry(*b"YEW", "YEW", "yew", "Green Yew Bough", "Cured Bowyer Yew"),
    entry(
        *b"ZNC",
        "ZNC",
        "zinc",
        "Roasted Zinc Ore",
        "Cast Guild Zinc",
    ),
];

/// Looks up a commodity display entry by 3 uppercase ASCII bytes, or `None` if unlisted.
#[must_use]
pub fn lookup_commodity_display_bytes(bytes: [u8; 3]) -> Option<&'static CommodityDisplayEntry> {
    COMMODITY_DISPLAY_CATALOG
        .binary_search_by_key(&bytes, |e| e.code_bytes)
        .ok()
        .map(|idx| &COMMODITY_DISPLAY_CATALOG[idx])
}

/// Looks up a commodity display entry by 3-letter code string (`"TMB"` -> `Some(...)`).
#[must_use]
pub fn lookup_commodity_display(code: &str) -> Option<&'static CommodityDisplayEntry> {
    let bytes: [u8; 3] = code.as_bytes().try_into().ok()?;
    lookup_commodity_display_bytes(bytes)
}

/// Formats the numeraire label (`"Clearinghouse Marks [CRED]"` vs `"CRED"`).
#[must_use]
pub const fn format_numeraire_display(plain_codes: bool) -> &'static str {
    if plain_codes {
        NUMERAIRE_PLAIN_LABEL
    } else {
        NUMERAIRE_DISPLAY_LABEL
    }
}

/// Seat-safe commodity ticker and display labels passed from host to seat UI without depending
/// on `league-gen` or `league-core`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatCommodityDisplay {
    /// Zero-based seat index (`0..seat_count`).
    pub seat: u8,
    /// Three-letter commodity ticker for `Raw(seat)`.
    pub raw_code: String,
    /// In-world display label for `Raw(seat)` (e.g., `"Rough Timber [TMB]"`).
    pub raw_label: String,
    /// Three-letter commodity ticker for `Refined(seat)`.
    pub refined_code: String,
    /// In-world display label for `Refined(seat)` (e.g., `"Seasoned Timber [TMB]"`).
    pub refined_label: String,
}

/// Table-level commodity display mapping consumed by seat UI crates (`league-seat-ui`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommodityDisplayMap {
    /// Fixed numeraire ticker (`"CRED"`).
    pub numeraire_code: String,
    /// Default in-world display label for `AssetId::Cred` (`"Clearinghouse Marks [CRED]"`).
    pub numeraire_label: String,
    /// Per-seat commodity display labels in seat order `0..seat_count`.
    pub seats: Vec<SeatCommodityDisplay>,
}

impl CommodityDisplayMap {
    /// Returns the display label for `asset`, selecting plain tickers when `plain_codes` is `true`.
    #[must_use]
    pub fn format_asset(&self, asset: AssetId, plain_codes: bool) -> Option<String> {
        match asset {
            AssetId::Cred => Some(if plain_codes {
                self.numeraire_code.clone()
            } else {
                self.numeraire_label.clone()
            }),
            AssetId::Raw(idx) => self.seats.get(usize::from(idx)).map(|s| {
                if plain_codes {
                    s.raw_code.clone()
                } else {
                    s.raw_label.clone()
                }
            }),
            AssetId::Refined(idx) => self.seats.get(usize::from(idx)).map(|s| {
                if plain_codes {
                    s.refined_code.clone()
                } else {
                    s.refined_label.clone()
                }
            }),
        }
    }
}
