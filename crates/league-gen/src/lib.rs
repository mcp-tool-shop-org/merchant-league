//! Merchant League v2 seeded chiral converter-ring table generator, LP
//! feasibility filter, and empirical lock-in / HHI evaluator.
//!
//! This crate is pure Rust with zero ledger or network I/O. Every generated
//! [`RingTable`] is checked by [`check_feasibility`] to prove:
//! 1. **Zero 2-seat self-sufficiency:** every pair `{i, j}` satisfies `0.0` of
//!    its own turn upkeep internally via [`coalition_upkeep_coverage`].
//! 2. **Strict aggregate converter surplus:** full-table conversion covers all
//!    mandatory upkeep with net Syndicate Project surplus exceeding
//!    `config.min_surplus_ratio`.
//! 3. **Seat expected-value balance:** Shapley season expected values across
//!    the `2^N` coalition LPs fall within `±config.ev_tolerance` of the mean.
//!
//! Quantitative priors from the seed-26 report (random bilateral lock-in rates
//! and dyadic HHI comparisons) are treated as hypotheses and measured directly
//! by [`measure_lockin_and_hhi`].

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
pub use feasibility::{FeasibilityReport, check_feasibility, coalition_upkeep_coverage};
pub use generator::{PlrChoice, PlrEntry, PlrSampler, generate_ring_table};
pub use lp::{LpError, LpSolution, LpStatus, solve_lp};
pub use rng::SeedStream;
pub use schema::{
    ConverterSpec, CurrencyCode, FeasibilityError, GenError, GeneratorConfig, HorizonShockKind,
    RingTable, ScheduledShock, SeatCount, SeatSpec,
};
