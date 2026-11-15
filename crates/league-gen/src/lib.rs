//! Merchant League v2 seeded chiral converter-ring table generator, LP
//! feasibility filter, and static lock-in / HHI model benchmark.
//!
//! This crate is pure Rust with zero ledger or network I/O. Every generated
//! [`RingTable`] uses the fixed `"CRED"` numeraire (`docs/design-v2.md` §1.2),
//! excludes denied ISO 4217 and crypto tickers, embeds its [`GeneratorConfig`]
//! and [`GENERATOR_VERSION`], and is checked by [`check_feasibility`] to prove:
//! 1. **Zero 2-seat self-sufficiency:** every pair `{i, j}` has no internal
//!    converter chain supplying either seat's upkeep (structural check via
//!    [`pair_has_structural_autarky`] and LP check via [`coalition_upkeep_coverage`]).
//! 2. **Strict aggregate converter surplus:** full-table conversion covers all
//!    mandatory upkeep with net Syndicate Project surplus exceeding
//!    `config.min_surplus_ratio`.
//! 3. **Seat expected-value balance:** expected seat values per turn (computed by
//!    [`EvMethod::ShadowPrice`] by default per `docs/design-v2.md` §1.5, or
//!    [`EvMethod::Shapley`] when selected) fall within `±config.ev_tolerance`
//!    of the table mean.
//!
//! Quantitative figures from the seed-26 report (random bilateral lock-in rates
//! and dyadic HHI comparisons) are kept as hypotheses in [`ReportHypotheses`].
//! [`measure_lockin_and_hhi`] measures random-endowment lock-in probabilities,
//! generator filter rejection rates, and static model HHI outputs; played-game
//! HHI belongs to Mode A engine runs.

pub mod empirical;
pub mod feasibility;
pub mod generator;
pub mod lp;
pub mod rng;
pub mod schema;

pub use empirical::{
    EmpiricalComparison, EnsembleStats, HhiMetrics, RandomEndowmentTable, RandomTableMetrics,
    ReportHypotheses, RingTableMetrics, compute_hhi, evaluate_random_table, evaluate_ring_table,
    generate_random_table, measure_lockin_and_hhi,
};
pub use feasibility::{
    FeasibilityReport, check_feasibility, coalition_upkeep_coverage, compute_shadow_price_seat_evs,
    compute_shapley_seat_evs, pair_has_structural_autarky,
};
pub use generator::{PlrChoice, PlrEntry, PlrSampler, generate_ring_table};
pub use lp::{LpError, LpSolution, LpStatus, solve_lp};
pub use rng::SeedStream;
pub use schema::{
    ConverterSpec, CurrencyCode, DENIED_CURRENCY_CODES, EvMethod, FeasibilityError,
    GENERATOR_VERSION, GenError, GeneratorConfig, HorizonShockKind, MAX_ATTEMPTS_CAP,
    NUMERAIRE_CODE, RingTable, ScheduledShock, SeatCount, SeatSpec, is_denied_currency_code,
};
