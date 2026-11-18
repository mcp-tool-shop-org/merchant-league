//! Compile-time role-qualified commodity glossary (`CommodityCatalog`) covering all `61`
//! allow-listed 3-letter commodity codes alongside the fixed `"CRED"` numeraire (`docs/club-mode-design.md` §3).
//!
//! Displays evocative in-world raw and refined guild trade names together with their `[CODE]`
//! ticker (for example `"Rough Timber [TMB]"` / `"Seasoned Timber [TMB]"` and
//! `"Clearinghouse Marks [CRED]"`), keeping ledger/engine codes and `GENERATOR_VERSION` unchanged.

use crate::schema::{ALLOWED_COMMODITY_GLOSSARY, CurrencyCode};
pub use league_seat_types::{
    COMMODITY_DISPLAY_CATALOG, NUMERAIRE_DISPLAY_LABEL, NUMERAIRE_PLAIN_LABEL,
};

/// Role-qualified display entry for one allow-listed 3-letter commodity code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommodityEntry {
    /// Three-letter uppercase commodity ticker.
    pub code: CurrencyCode,
    /// Underlying glossary noun in [`ALLOWED_COMMODITY_GLOSSARY`].
    pub glossary_word: &'static str,
    /// In-world display name for the perishable extracted `Raw` form.
    pub raw_name: &'static str,
    /// In-world display name for the manufactured `Refined` form.
    pub refined_name: &'static str,
}

impl CommodityEntry {
    /// Formats `Raw` display label (`"Rough Timber [TMB]"` or `"TMB"` when `plain_codes` is set).
    #[must_use]
    pub fn raw_display(&self, plain_codes: bool) -> String {
        if plain_codes {
            self.code.as_str().to_owned()
        } else {
            format!("{} [{}]", self.raw_name, self.code)
        }
    }

    /// Formats `Refined` display label (`"Seasoned Timber [TMB]"` or `"TMB"` when `plain_codes` is set).
    #[must_use]
    pub fn refined_display(&self, plain_codes: bool) -> String {
        if plain_codes {
            self.code.as_str().to_owned()
        } else {
            format!("{} [{}]", self.refined_name, self.code)
        }
    }
}

const fn entry(
    bytes: [u8; 3],
    glossary_word: &'static str,
    raw_name: &'static str,
    refined_name: &'static str,
) -> CommodityEntry {
    CommodityEntry {
        code: CurrencyCode::from_allowed_bytes_unchecked(bytes),
        glossary_word,
        raw_name,
        refined_name,
    }
}

const ENTRIES: [CommodityEntry; 61] = [
    entry(*b"ALM", "alum", "Raw Alum", "Refined Alum"),
    entry(
        *b"ANV",
        "anvil",
        "Forged Iron Anvil",
        "Tempered Steel Anvil",
    ),
    entry(
        *b"BBL",
        "barrel",
        "Unbound Barrel Staves",
        "Hooped Guild Barrel",
    ),
    entry(*b"BLW", "bellows", "Hide Bellows", "Brass-Nozzle Bellows"),
    entry(*b"BLY", "barley", "Field Barley", "Kilned Barley"),
    entry(*b"BRS", "brass", "Raw Brass Ingot", "Burnished Brass Plate"),
    entry(
        *b"CBL",
        "cobble",
        "Rough Quarry Cobble",
        "Dressed Road Cobble",
    ),
    entry(*b"CDR", "cedar", "Felled Cedar Log", "Planed Cedar Beam"),
    entry(*b"CHK", "chalk", "Pit Chalk Lump", "Sifted Mason Chalk"),
    entry(*b"CLY", "clay", "River Clay", "Wedged Potter Clay"),
    entry(*b"CNV", "canvas", "Unbleached Canvas", "Waxed Sail Canvas"),
    entry(*b"CPR", "copper", "Raw Copper Ore", "Smelted Copper Bar"),
    entry(*b"CRL", "coral", "Raw Sea Coral", "Polished Coral Bead"),
    entry(*b"DYE", "dye", "Crushed Woad Dye", "Steeped Fast Dye"),
    entry(*b"ELM", "elm", "Green Elm Log", "Seasoned Elm Wheelwood"),
    entry(
        *b"ENM",
        "enamel",
        "Ground Enamel Frit",
        "Vitrified Guild Enamel",
    ),
    entry(*b"FIR", "fir", "Green Fir Timber", "Seasoned Fir Spar"),
    entry(
        *b"FLN",
        "flint",
        "Rough Flint Nodule",
        "Knapped Strike Flint",
    ),
    entry(*b"GLS", "glass", "Cullet Glass", "Crown Window Glass"),
    entry(*b"GLZ", "glaze", "Raw Ash Glaze", "Sieved Ceramic Glaze"),
    entry(*b"HMP", "hemp", "Retted Hemp Fiber", "Hackled Guild Hemp"),
    entry(*b"HNY", "honey", "Comb Honey", "Strained Amber Honey"),
    entry(*b"HPS", "hops", "Green Hop Cones", "Kiln-Dried Hops"),
    entry(*b"HRB", "herb", "Wild Bundle Herb", "Dried Apothecary Herb"),
    entry(*b"IRN", "iron", "Pig Iron Bloom", "Wrought Guild Iron"),
    entry(*b"IVR", "ivory", "Unworked Ivory", "Carved Ivory"),
    entry(*b"JDE", "jade", "Rough Jade Pebble", "Carved Jade Seal"),
    entry(
        *b"KLN",
        "kiln",
        "Firebrick Kiln Clay",
        "Refractory Kiln Brick",
    ),
    entry(*b"KLP", "kelp", "Wet Shore Kelp", "Calcined Kelp Soda"),
    entry(*b"LOM", "loom", "Hewn Loom Frame", "Strung Broad Loom"),
    entry(
        *b"LTH",
        "leather",
        "Brined Raw Leather",
        "Curried Harness Leather",
    ),
    entry(
        *b"LYE",
        "lye",
        "Leached Wood Ash Lye",
        "Clarified Soapmaker Lye",
    ),
    entry(*b"MLT", "malt", "Steeped Green Malt", "Pale Kilned Malt"),
    entry(
        *b"MRB",
        "marble",
        "Rough Marble Block",
        "Honed Ashlar Marble",
    ),
    entry(
        *b"NTR",
        "nitre",
        "Crude Nitre Crust",
        "Recrystallized Nitre",
    ),
    entry(*b"OAK", "oak", "Green Oak Heartwood", "Seasoned Cooper Oak"),
    entry(*b"OAT", "oat", "Threshed Field Oat", "Rolled Mill Oat"),
    entry(*b"OLV", "olive", "Pressed Green Olive", "Cured Brine Olive"),
    entry(
        *b"PCH",
        "pitch",
        "Pine Tar Pitch",
        "Boiled Shipwright Pitch",
    ),
    entry(*b"PLK", "plank", "Rough-Sawn Plank", "Planed Deck Plank"),
    entry(*b"PWT", "pewter", "Crude Pewter Alloy", "Spun Guild Pewter"),
    entry(
        *b"QRZ",
        "quartz",
        "Vein Quartz Chert",
        "Crushed Glassmaker Quartz",
    ),
    entry(*b"ROP", "rope", "Spun Yarn Cordage", "Tarred Hawser Rope"),
    entry(
        *b"RSN",
        "resin",
        "Scraped Pine Resin",
        "Distilled Clear Resin",
    ),
    entry(*b"RYE", "rye", "Winnowed Winter Rye", "Bolted Rye Meal"),
    entry(*b"SLG", "slag", "Furnace Iron Slag", "Crushed Roadbed Slag"),
    entry(*b"SLK", "silk", "Reeled Raw Silk", "Thrown Guild Silk"),
    entry(*b"SLT", "salt", "Coarse Pan Salt", "Cured Fine Salt"),
    entry(
        *b"SPN",
        "spindle",
        "Turned Oak Spindle",
        "Weighted Guild Spindle",
    ),
    entry(
        *b"SUF",
        "sulfur",
        "Brimstone Sulfur Ore",
        "Sublimed Flour Sulfur",
    ),
    entry(
        *b"TLW",
        "tallow",
        "Rendered Rough Tallow",
        "Clarified Candle Tallow",
    ),
    entry(*b"TMB", "timber", "Rough Timber", "Seasoned Timber"),
    entry(
        *b"TNG",
        "tongs",
        "Rough-Forged Tongs",
        "Balanced Smithy Tongs",
    ),
    entry(*b"TRF", "turf", "Cut Moorland Turf", "Stacked Dried Turf"),
    entry(
        *b"VLM",
        "vellum",
        "Limed Calfskin Vellum",
        "Pumiced Scribe Vellum",
    ),
    entry(*b"VLV", "velvet", "Loom-Cut Velvet", "Sheared Guild Velvet"),
    entry(*b"WHT", "wheat", "Sheaved Red Wheat", "Milled Fine Wheat"),
    entry(
        *b"WLK",
        "whelk",
        "Fresh Tidal Whelk",
        "Steeped Tyrian Whelk",
    ),
    entry(*b"WOL", "wool", "Washed Fleece Wool", "Combed Worsted Wool"),
    entry(*b"YEW", "yew", "Green Yew Bough", "Cured Bowyer Yew"),
    entry(*b"ZNC", "zinc", "Roasted Zinc Ore", "Cast Guild Zinc"),
];

const fn str_eq(a: &str, b: &str) -> bool {
    let ab = a.as_bytes();
    let bb = b.as_bytes();
    if ab.len() != bb.len() {
        return false;
    }
    let mut i = 0;
    while i < ab.len() {
        if ab[i] != bb[i] {
            return false;
        }
        i += 1;
    }
    true
}

const fn catalog_aligned_with_glossary() -> bool {
    if ENTRIES.len() != ALLOWED_COMMODITY_GLOSSARY.len()
        || ENTRIES.len() != COMMODITY_DISPLAY_CATALOG.len()
    {
        return false;
    }
    let mut i = 0;
    while i < ENTRIES.len() {
        let a = ENTRIES[i].code.as_bytes();
        let b = ALLOWED_COMMODITY_GLOSSARY[i].0;
        let c = &COMMODITY_DISPLAY_CATALOG[i];
        if a[0] != b[0] || a[1] != b[1] || a[2] != b[2] {
            return false;
        }
        if a[0] != c.code_bytes[0] || a[1] != c.code_bytes[1] || a[2] != c.code_bytes[2] {
            return false;
        }
        if !str_eq(ENTRIES[i].glossary_word, c.glossary_word)
            || !str_eq(ENTRIES[i].raw_name, c.raw_name)
            || !str_eq(ENTRIES[i].refined_name, c.refined_name)
        {
            return false;
        }
        i += 1;
    }
    true
}

const _: () = {
    assert!(
        catalog_aligned_with_glossary(),
        "CommodityCatalog ENTRIES must stay 1-for-1 aligned with ALLOWED_COMMODITY_GLOSSARY"
    );
};

/// Compile-time lookup catalog over all `61` allow-listed commodity codes plus `CRED`.
pub struct CommodityCatalog;

impl CommodityCatalog {
    /// Returns all `61` compile-time commodity entries in canonical code order (`ALM..ZNC`).
    #[must_use]
    pub const fn entries() -> &'static [CommodityEntry; 61] {
        &ENTRIES
    }

    /// Returns the compile-time entry for a validated [`CurrencyCode`].
    #[must_use]
    pub fn lookup(code: CurrencyCode) -> &'static CommodityEntry {
        match Self::lookup_bytes(code.as_bytes()) {
            Some(found) => found,
            None => unreachable!("validated CurrencyCode is always present in ENTRIES"),
        }
    }

    /// Looks up a commodity entry by 3 uppercase ASCII bytes, or `None` if unlisted.
    #[must_use]
    pub fn lookup_bytes(bytes: [u8; 3]) -> Option<&'static CommodityEntry> {
        ENTRIES
            .binary_search_by_key(&bytes, |entry| entry.code.as_bytes())
            .ok()
            .map(|idx| &ENTRIES[idx])
    }

    /// Looks up a commodity entry by 3-letter code string (`"TMB"` -> `Some(...)`, `"CRED"` -> `None`).
    #[must_use]
    pub fn lookup_str(code: &str) -> Option<&'static CommodityEntry> {
        let bytes: [u8; 3] = code.as_bytes().try_into().ok()?;
        Self::lookup_bytes(bytes)
    }

    /// Formats the numeraire label (`"Clearinghouse Marks [CRED]"` vs `"CRED"`).
    #[must_use]
    pub const fn format_numeraire(plain_codes: bool) -> &'static str {
        if plain_codes {
            NUMERAIRE_PLAIN_LABEL
        } else {
            NUMERAIRE_DISPLAY_LABEL
        }
    }

    /// Formats the `Raw` in-world name beside `[CODE]` (or bare `[CODE]` when `plain_codes` is set).
    #[must_use]
    pub fn format_raw(code: CurrencyCode, plain_codes: bool) -> String {
        Self::lookup(code).raw_display(plain_codes)
    }

    /// Formats the `Refined` in-world name beside `[CODE]` (or bare `CODE` when `plain_codes` is set).
    #[must_use]
    pub fn format_refined(code: CurrencyCode, plain_codes: bool) -> String {
        Self::lookup(code).refined_display(plain_codes)
    }
}
