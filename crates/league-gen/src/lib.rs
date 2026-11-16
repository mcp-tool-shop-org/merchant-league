//! Merchant League v2 seeded chiral converter-ring table generator, LP
//! feasibility filter, and static lock-in / HHI / rollout grading suite.
//!
//! This crate is pure Rust with zero ledger or network I/O. Every generated
//! [`RingTable`] uses the fixed `"CRED"` numeraire (`docs/design-v2.md` §1.2),
//! draws commodity codes strictly from the curated game-word allow-list
//! [`ALLOWED_COMMODITY_GLOSSARY`] (`#26a`), embeds its [`GeneratorConfig`],
//! [`GENERATOR_VERSION`], Tier-3 [`ProjectSpec`]s (`1..=N` pairs plus `100`
//! full-ring Concordat), and graded `3.0..=8.0 VP` [`MandateCardSpec`] deck (`B2.2`),
//! and is verified by [`check_feasibility`] (which also executes [`compute_rollout_grading`], a
//! deterministic scripted proxy rollout inside `league-gen`, rather than `league-core`'s turn
//! engine) to prove:
//! 1. **Zero 2-seat self-sufficiency:** every pair `{i, j}` has no internal
//!    converter chain supplying either seat's upkeep (structural check via
//!    [`pair_has_structural_autarky`] and LP check via [`coalition_upkeep_coverage`]).
//! 2. **Strict aggregate & per-converter surplus:** full-table conversion covers all
//!    mandatory upkeep with net Syndicate Project surplus exceeding
//!    `config.min_surplus_ratio`, and every converter maintains positive value-added
//!    evaluated at the shadow LP's objective/salvage floor coefficients (`B2.1`).
//! 3. **Seat expected-value balance & top-pair cap:** expected seat values per turn
//!    (computed by [`EvMethod::ShadowPrice`] by default per `docs/design-v2.md` §1.5,
//!    conserving [`FeasibilityReport::season_effective_net_surplus`], or
//!    [`EvMethod::Shapley`]) fall within `±config.ev_tolerance` of the table mean,
//!    and no unordered pair exceeds the provisional C6 table pair-share cap.
//!
//! Quantitative figures from the seed-26 report (random bilateral lock-in rates
//! and dyadic HHI comparisons) are kept as hypotheses in [`ReportHypotheses`].
//! [`measure_lockin_and_hhi`] measures random-endowment lock-in probabilities,
//! generator filter rejection rates, and static model HHI / `max_pair_share` outputs;
//! played-game HHI belongs to Mode A engine runs.

pub mod empirical;
pub mod feasibility;
pub mod generator;
pub mod lp;
pub mod mandates;
pub mod rng;
pub mod schema;

pub use empirical::{
    EmpiricalComparison, EnsembleStats, HhiMetrics, RandomEndowmentTable, RandomTableMetrics,
    ReportHypotheses, RingTableMetrics, RolloutGradingReport, compute_hhi, compute_rollout_grading,
    evaluate_random_table, evaluate_ring_table, generate_random_table, measure_lockin_and_hhi,
    verify_rollout_grading,
};
pub use feasibility::{
    ConverterMarginDiag, FeasibilityReport, ShadowPriceDetails, TableTradeOrderings,
    TradeOptionLabel, TradeOptionRole, check_feasibility, coalition_upkeep_coverage,
    compute_converter_clearing_margins, compute_seat_trade_orderings,
    compute_shadow_price_allocation, compute_shadow_price_seat_evs, compute_shapley_seat_evs,
    pair_has_structural_autarky, solve_shadow_price_details,
};
pub use generator::{PlrChoice, PlrEntry, PlrSampler, generate_ring_table};
pub use lp::{LpError, LpSolution, LpStatus, solve_lp};
pub use mandates::{
    CreditTier, GradedMandateCard, MandateKind, deal_graded_mandates,
    deal_graded_mandates_with_entropy, deal_mandate_deck_indices, deal_mandates_for_ring_table,
    default_graded_mandate_deck, graded_mandate_deck_for_mean_project_value,
    mandate_deck_for_ring_table, mean_project_value_milli,
};
pub use rng::SeedStream;
pub use schema::{
    ALLOWED_COMMODITY_GLOSSARY, BPS_SCALE, ConverterSpec, CreditRatingTier, CurrencyCode,
    DENIED_CURRENCY_CODES, EvMethod, FeasibilityError, GENERATOR_VERSION, GenError,
    GeneratorConfig, HorizonShockKind, MAX_ATTEMPTS_CAP, MAX_MANDATE_BONUS_VP,
    MIN_MANDATE_BONUS_VP, MandateCardSpec, MandateObjective, NUMERAIRE_CODE, PROJECT_CONCORDAT_ID,
    ProjectSpec, RingTable, ScheduledShock, SeatCount, SeatSpec, bonus_vp_for_completion_bps,
    commodity_glossary_word, deal_mandate_indices_with_entropy, default_mandate_deck_for_table,
    default_projects_for_seats, is_allowed_commodity_code, is_denied_currency_code,
};
