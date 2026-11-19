//! Checked bridge promoting a validated [`RingTable`] into a `TableMode::Club`
//! [`league_core::TableSpec`] plus role-qualified [`TableCommodityMap`] (`docs/club-mode-design.md` §1.8 & §7 Card 3).
//!
//! Enforces `SeedPool::Dev` (`0..=9_999`), refuses [`is_reserved_test_seed`], checks
//! [`GENERATOR_VERSION`] (`"league-gen/v2.5.0"`), re-verifies LP/structural feasibility via
//! [`check_feasibility`], initializes [`DEFAULT_CLUB_HUMAN_REGENT_DOCTRINE`] on requested human
//! seats, leaves `TableSpec::shocks` empty so Horizon Shocks draw dynamically from verified
//! `EvalVrf` beacons, and returns freshly dealt sealed mandates plus [`SealedEvalSeed`].

use std::collections::BTreeSet;

use league_core::{
    AssetId, ConverterRecipe, DEFAULT_CANCEL_LOCK_BPS, DEFAULT_CLOSING_HAZARD_BPS,
    DEFAULT_CLUB_HUMAN_REGENT_DOCTRINE, DEFAULT_MANDATE_ALPHA_BPS, DEFAULT_TOWN_IMPORT_PREMIUM_BPS,
    DEFAULT_WASH_FREEZE_BOND_CRED, DEFAULT_WASH_FREEZE_BOUNTY_CRED,
    DEFAULT_WASH_ROUND_TRIP_MIN_UNITS, DEFAULT_WORKS_STIPEND_PER_UNIT_CRED, ProjectRecipe,
    RevealSalt, SealedEvalSeed, SealedMandateDeal, SeatConfig, SeatIdentity, TableId, TableMode,
    TableSpec, Units, deal_graded_mandates, deal_graded_mandates_with_entropy,
    validate_club_dev_seed,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::feasibility::check_feasibility;
use crate::glossary::{CommodityCatalog, NUMERAIRE_DISPLAY_LABEL};
use crate::mandates::mandate_deck_for_ring_table;
use crate::schema::{
    CurrencyCode, GENERATOR_VERSION, GenError, NUMERAIRE_CODE, RingTable, SeedPool, TablePurpose,
    is_reserved_test_seed, seed_pool,
};

/// Default starting `CRED` allocation (`15.000 CRED` in milli-units) for Club Mode tables.
pub const DEFAULT_CLUB_INITIAL_CRED: Units = 15_000;

/// Per-seat commodity code and in-world role-qualified display mapping for a Club table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatCommodityAssignment {
    /// Zero-based seat index (`0..seat_count`).
    pub seat: u8,
    /// Perishable extracted raw commodity ticker.
    pub raw_code: CurrencyCode,
    /// In-world display label for `Raw(seat)` (for example `"Rough Timber [TMB]"`).
    pub raw_label: String,
    /// Manufactured refined commodity ticker.
    pub refined_code: CurrencyCode,
    /// In-world display label for `Refined(seat)` (for example `"Seasoned Timber [TMB]"`).
    pub refined_label: String,
}

/// Complete table-level numeraire + per-seat commodity glossary mapping produced alongside
/// [`ring_table_to_table_spec`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableCommodityMap {
    /// Fixed numeraire ticker (`"CRED"`).
    pub numeraire_code: String,
    /// Default in-world display label for `AssetId::Cred` (`"Clearinghouse Marks [CRED]"`).
    pub numeraire_label: String,
    /// Per-seat assignments in seat order `0..seat_count`.
    pub seats: Vec<SeatCommodityAssignment>,
}

impl TableCommodityMap {
    /// Builds the [`TableCommodityMap`] for a validated [`RingTable`] using [`CommodityCatalog`].
    #[must_use]
    pub fn from_ring_table(table: &RingTable) -> Self {
        let seats = table
            .seats
            .iter()
            .map(|seat| SeatCommodityAssignment {
                seat: seat.seat,
                raw_code: seat.raw_code,
                raw_label: CommodityCatalog::format_raw(seat.raw_code, false),
                refined_code: seat.refined_code,
                refined_label: CommodityCatalog::format_refined(seat.refined_code, false),
            })
            .collect();
        Self {
            numeraire_code: NUMERAIRE_CODE.to_owned(),
            numeraire_label: NUMERAIRE_DISPLAY_LABEL.to_owned(),
            seats,
        }
    }

    /// Formats an [`AssetId`] using in-world names alongside `[CODE]`, or plain codes when
    /// `plain_codes` is `true`. Returns `None` if the asset references an out-of-range seat.
    #[must_use]
    pub fn format_asset(&self, asset: AssetId, plain_codes: bool) -> Option<String> {
        match asset {
            AssetId::Cred => Some(CommodityCatalog::format_numeraire(plain_codes).to_owned()),
            AssetId::Raw(idx) => self
                .seats
                .get(usize::from(idx))
                .map(|s| CommodityCatalog::format_raw(s.raw_code, plain_codes)),
            AssetId::Refined(idx) => self
                .seats
                .get(usize::from(idx))
                .map(|s| CommodityCatalog::format_refined(s.refined_code, plain_codes)),
        }
    }

    /// Exports a seat-safe [`league_seat_types::CommodityDisplayMap`] without depending on
    /// `league-gen` inside UI crates.
    #[must_use]
    pub fn to_display_map(&self) -> league_seat_types::CommodityDisplayMap {
        league_seat_types::CommodityDisplayMap {
            numeraire_code: self.numeraire_code.clone(),
            numeraire_label: self.numeraire_label.clone(),
            seats: self
                .seats
                .iter()
                .map(|s| league_seat_types::SeatCommodityDisplay {
                    seat: s.seat,
                    raw_code: s.raw_code.as_str().to_owned(),
                    raw_label: s.raw_label.clone(),
                    refined_code: s.refined_code.as_str().to_owned(),
                    refined_label: s.refined_label.clone(),
                })
                .collect(),
        }
    }
}

impl From<&TableCommodityMap> for league_seat_types::CommodityDisplayMap {
    fn from(map: &TableCommodityMap) -> Self {
        map.to_display_map()
    }
}

fn validate_club_bridge_inputs(table: &RingTable, human_seats: &[u8]) -> Result<(), GenError> {
    if is_reserved_test_seed(table.seed) {
        if seed_pool(table.seed) != SeedPool::Dev {
            return Err(GenError::seed_pool_mismatch(table.seed, TablePurpose::Dev));
        }
        return Err(GenError::InvalidConfig(format!(
            "RESERVED_SEED_FORBIDDEN: layout seed {} is reserved for tests",
            table.seed
        )));
    }
    if seed_pool(table.seed) != SeedPool::Dev {
        return Err(GenError::seed_pool_mismatch(table.seed, TablePurpose::Dev));
    }
    validate_club_dev_seed(table.seed).map_err(|err| GenError::InvalidConfig(err.to_string()))?;

    if table.generator_version != GENERATOR_VERSION {
        return Err(GenError::InvalidConfig(format!(
            "expected generator_version `{GENERATOR_VERSION}`, got `{}`",
            table.generator_version
        )));
    }
    if table.numeraire_code != NUMERAIRE_CODE {
        return Err(GenError::InvalidConfig(format!(
            "expected numeraire_code `{NUMERAIRE_CODE}`, got `{}`",
            table.numeraire_code
        )));
    }

    let n = table.seat_count.as_u8();
    let mut seen_humans = BTreeSet::new();
    for &seat in human_seats {
        if seat >= n {
            return Err(GenError::InvalidConfig(format!(
                "human seat index {seat} out of range 0..{n}"
            )));
        }
        if !seen_humans.insert(seat) {
            return Err(GenError::InvalidConfig(format!(
                "duplicate human seat index {seat}"
            )));
        }
    }

    check_feasibility(table).map_err(|last_error| GenError::NoFeasibleTableFound {
        attempts: 1,
        last_error,
    })?;

    Ok(())
}

fn build_club_table_spec(
    table: &RingTable,
    table_id: TableId,
    human_seats: &[u8],
    dealt_mandate_commits: Vec<league_core::CommitmentHash>,
    eval_vrf_pubkey: [u8; 32],
) -> Result<(TableSpec, TableCommodityMap), GenError> {
    let human_set: BTreeSet<u8> = human_seats.iter().copied().collect();
    let n = table.seat_count.as_u8();

    let seats: Vec<SeatConfig> = table
        .seats
        .iter()
        .map(|s| {
            let is_human = human_set.contains(&s.seat);
            let identity = if is_human {
                SeatIdentity::Human
            } else {
                SeatIdentity::AiSyndicate
            };
            let regent = is_human.then_some(DEFAULT_CLUB_HUMAN_REGENT_DOCTRINE);
            SeatConfig {
                seat: s.seat,
                identity,
                initial_cred: DEFAULT_CLUB_INITIAL_CRED,
                raw_yield_per_turn: ((s.raw_yield_per_turn * 1_000.0).round() as Units).max(1),
                refined_project_value: ((s.refined_project_value * 1_000.0).round() as Units)
                    .max(1),
                upkeep_good: s.upkeep_good,
                upkeep_amount: ((s.upkeep_amount * 1_000.0).round() as Units).max(1),
                initial_raw_tariff_bps: (s.raw_export_tariff * 10_000.0).round() as u16,
                initial_refined_tariff_bps: (s.refined_export_tariff * 10_000.0).round() as u16,
                perishable_decay_bps: ((s.perishable_decay_rate * 10_000.0).round() as u16)
                    .min(9_999),
                regent,
                converters: s
                    .converters
                    .iter()
                    .map(|c| ConverterRecipe {
                        owner: c.owner,
                        inputs: c
                            .inputs
                            .iter()
                            .map(|&(g, amt)| (g, ((amt * 1_000.0).round() as Units).max(1)))
                            .collect(),
                        output_good: c.output_good,
                        output_amount: ((c.output_amount * 1_000.0).round() as Units).max(1),
                        max_runs_per_turn: (c.max_runs_per_turn.ceil() as u32).max(1),
                        is_secondary: c.is_secondary,
                    })
                    .collect(),
            }
        })
        .collect();

    let projects: Vec<ProjectRecipe> = table
        .projects
        .iter()
        .map(|p| ProjectRecipe {
            id: u16::from(p.id),
            inputs: p
                .required_inputs
                .iter()
                .map(|&(g, _, amt)| (g, ((amt * 1_000.0).round() as Units).max(1)))
                .collect(),
            equity_points: ((p.output_project_equity * 1_000.0).round() as Units).max(1),
        })
        .collect();

    let spec = TableSpec {
        table_id,
        mode: TableMode::Club,
        seat_count: n,
        seats,
        projects,
        // Club mode always draws Horizon Shocks dynamically from per-turn `EvalVrf` beacons.
        shocks: Vec::new(),
        closing_hazard_bps: DEFAULT_CLOSING_HAZARD_BPS,
        mandate_deck: mandate_deck_for_ring_table(table),
        dealt_mandate_commits,
        drand_schedule: None,
        eval_vrf_pubkey: Some(eval_vrf_pubkey),
        town_import_premium_bps: DEFAULT_TOWN_IMPORT_PREMIUM_BPS,
        works_stipend_per_unit_cred: DEFAULT_WORKS_STIPEND_PER_UNIT_CRED,
        mandate_alpha_bps: DEFAULT_MANDATE_ALPHA_BPS,
        cancel_lock_bps: DEFAULT_CANCEL_LOCK_BPS,
        wash_round_trip_min_units: DEFAULT_WASH_ROUND_TRIP_MIN_UNITS,
        wash_freeze_bond_cred: DEFAULT_WASH_FREEZE_BOND_CRED,
        wash_freeze_bounty_cred: DEFAULT_WASH_FREEZE_BOUNTY_CRED,
    };
    spec.validate()
        .map_err(|err| GenError::InvalidConfig(err.to_string()))?;

    let commodity_map = TableCommodityMap::from_ring_table(table);
    Ok((spec, commodity_map))
}

/// Promotes a validated `dev`-pool [`RingTable`] (`GENERATOR_VERSION == "league-gen/v2.5.0"`)
/// into a playable `TableMode::Club` [`TableSpec`] plus [`TableCommodityMap`], freshly dealt
/// Turn-1 mandates ([`SealedMandateDeal`]), and ephemeral [`SealedEvalSeed`] from OS CSPRNG.
///
/// Seats listed in `human_seats` become [`SeatIdentity::Human`] with
/// [`DEFAULT_CLUB_HUMAN_REGENT_DOCTRINE`]; remaining seats become [`SeatIdentity::AiSyndicate`]
/// (`ClubBot` seats with `regent: None`). Refuses reserved test seeds and non-`dev` pools.
pub fn ring_table_to_table_spec(
    table: &RingTable,
    table_id: TableId,
    human_seats: &[u8],
) -> Result<
    (
        TableSpec,
        TableCommodityMap,
        SealedMandateDeal,
        SealedEvalSeed,
    ),
    GenError,
> {
    validate_club_bridge_inputs(table, human_seats)?;
    let mandate_deck = mandate_deck_for_ring_table(table);
    let (dealt_mandate_commits, mandate_deal) =
        deal_graded_mandates(table_id, table.seat_count.as_u8(), &mandate_deck)
            .map_err(|err| GenError::InvalidConfig(err.to_string()))?;
    let eval_seed =
        SealedEvalSeed::generate().map_err(|err| GenError::InvalidConfig(err.to_string()))?;
    let eval_pk = eval_seed
        .public_key_bytes()
        .map_err(|err| GenError::InvalidConfig(err.to_string()))?;
    let (spec, map) =
        build_club_table_spec(table, table_id, human_seats, dealt_mandate_commits, eval_pk)?;
    Ok((spec, map, mandate_deal, eval_seed))
}

/// Deterministic explicit-entropy sibling of [`ring_table_to_table_spec`] for tests and
/// reproducible fixtures, still enforcing `SeedPool::Dev`, `!is_reserved_test_seed`,
/// [`check_feasibility`], and independence of `deal_seed` / `eval_seed` from `table.seed`.
pub fn ring_table_to_table_spec_with_explicit_entropy(
    table: &RingTable,
    table_id: TableId,
    human_seats: &[u8],
    deal_seed: [u8; 32],
    seat_salts: &[RevealSalt],
    eval_seed: SealedEvalSeed,
) -> Result<
    (
        TableSpec,
        TableCommodityMap,
        SealedMandateDeal,
        SealedEvalSeed,
    ),
    GenError,
> {
    validate_club_bridge_inputs(table, human_seats)?;
    let mut padded_seed = [0u8; 32];
    padded_seed[..8].copy_from_slice(&table.seed.to_le_bytes());
    let hashed_seed: [u8; 32] = Sha256::digest(table.seed.to_le_bytes()).into();
    if deal_seed == padded_seed || deal_seed == hashed_seed {
        return Err(GenError::InvalidConfig(
            "deal_seed must not derive from public layout table.seed".to_owned(),
        ));
    }
    let eval_pk = eval_seed
        .public_key_bytes()
        .map_err(|err| GenError::InvalidConfig(err.to_string()))?;
    for cand in [padded_seed, hashed_seed] {
        if let Ok(derived_eval) = SealedEvalSeed::try_from_bytes(cand)
            && derived_eval.public_key_bytes().ok() == Some(eval_pk)
        {
            return Err(GenError::InvalidConfig(
                "eval_seed must not derive from public layout table.seed".to_owned(),
            ));
        }
    }

    let mandate_deck = mandate_deck_for_ring_table(table);
    let (dealt_mandate_commits, mandate_deal) = deal_graded_mandates_with_entropy(
        table_id,
        table.seat_count.as_u8(),
        deal_seed,
        seat_salts,
        &mandate_deck,
    )
    .map_err(|err| GenError::InvalidConfig(err.to_string()))?;

    let (spec, map) =
        build_club_table_spec(table, table_id, human_seats, dealt_mandate_commits, eval_pk)?;
    Ok((spec, map, mandate_deal, eval_seed))
}
