use serde::{Deserialize, Serialize};

use crate::types::{Bps, CreditTier, SeatIdentity, ShockKind, Units};

/// Active phase within a simultaneous turn or season close lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnPhase {
    /// Phase 1: open `CRED` market quotes, Track-2 checks/pledges, and Track-1 `TokenEscrow` setup.
    TalkAndMarket,
    /// Phase 2: sealed cryptographic commitment submission for simultaneous actions.
    Commit,
    /// Reveal gate: seats submit plaintext bundles + salts matching Phase 2 commitments.
    Reveal,
    /// Closing bell sounded on Turn `24..=32`; waiting for Turn-1 Private Mandate reveals.
    AwaitingMandateReveal,
    /// Season finalized cleanly; [`crate::FinalScoreboard`] computed.
    Completed,
}

/// Per-seat mutable ledger-mirror state maintained deterministically across turns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatState {
    /// Seat index (`0..seat_count`).
    pub seat: u8,
    /// Current disclosed identity (`Human` or converted `AiSyndicate`).
    pub identity: SeatIdentity,
    /// Free `CRED` balance (excluding DEX buy locks and `TokenEscrow` vaults).
    pub cred: Units,
    /// Unpaid `CRED` auto-debt principal + accrued interest owed to Authority.
    pub cred_debt: Units,
    /// Free raw commodity balances `raw[0..N]` (perishable).
    pub raw: Vec<Units>,
    /// Free refined commodity balances `refined[0..N]`.
    pub refined: Vec<Units>,
    /// Irreversible Tier-3 Syndicate Project Equity points earned.
    pub project_equity: Units,
    /// Current export `TransferRate` tariff on `Raw(seat)` (`0..=3_500 bps`).
    pub raw_tariff_bps: Bps,
    /// Current export `TransferRate` tariff on `Refined(seat)` (`0..=3_500 bps`).
    pub refined_tariff_bps: Bps,
    /// Current Hard Credit Solvency tier (`Aaa..D`).
    pub credit_tier: CreditTier,
    /// Count of `CheckCash` bounces causing Hard Credit Defaults.
    pub hard_credit_defaults: u32,
    /// Count of defaulted Steward-refereed delivery bonds.
    pub bond_breaches: u32,
    /// Count of Soft Diplomatic Honor promise breaches detected by deterministic audit.
    pub broken_promises: u32,
    /// Cumulative honor penalty weight.
    pub honor_penalties: Units,
    /// Consecutive turns with missing/invalid Phase 2 reveal (`3` triggers AI conversion).
    pub consecutive_misses: u8,
    /// Consecutive turns with zero executed converter runs (`3` counts as missed; `C1`, `2026-11-15`).
    #[serde(default)]
    pub consecutive_zero_production_turns: u8,
    /// Turn through which `CustomsInspector` wash-freeze blocks new offers/swaps (`0` = none).
    pub wash_frozen_until_turn: u8,
}

/// Audited broken-promise record emitted when a seat violates an anchored `IntentPledge`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrokenPromiseRecord {
    /// Violated pledge ID.
    pub pledge_id: u64,
    /// Promisor seat penalized.
    pub promisor: u8,
    /// Counterparty affected.
    pub counterparty: u8,
    /// Penalty honor weight charged.
    pub weight: Units,
}

/// Publicly narrowed forecast emitted in Sweep Step 8 (`forecast_turn`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShockForecastEvent {
    /// Seat whose commodity sector is forecasted to shift next turn.
    pub target_seat: u8,
    /// Forecasted shock category.
    pub kind: ShockKind,
    /// Forecasted shock turn (`realize_turn`).
    pub realize_turn: u8,
}

/// Preview breakdown of one [`crate::ConverterRunOrder`] simulated inside preview execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConverterOrderPreview {
    /// Converter recipe index requested.
    pub recipe_idx: usize,
    /// Original run count submitted in the bundle.
    pub requested_runs: u32,
    /// Whole runs executed after capacity and inventory clamping.
    pub executed_runs: u32,
    /// `true` if clamped (`0 < executed_runs < requested_runs`).
    pub clamped: bool,
}

/// Preview breakdown of one [`crate::ProjectDeliveryOrder`] simulated inside preview execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectOrderPreview {
    /// Target Syndicate Project ID requested.
    pub project_id: u16,
    /// Original batch count submitted in the bundle.
    pub requested_batches: u32,
    /// Whole batches executed after refined-inventory clamping.
    pub executed_batches: u32,
    /// `true` if clamped (`0 < executed_batches < requested_batches`).
    pub clamped: bool,
}

/// Preview breakdown of one `EscrowFinish` unlock request in Step 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EscrowOrderPreview {
    /// Escrow vault identifier.
    pub escrow_id: u64,
    /// `true` if the escrow unlocked and released funds to its destination.
    pub unlocked: bool,
}

/// Per-seat Step-2 debt repayment and Step-5 upkeep/interest accounting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatUpkeepPreview {
    /// Refined commodity index consumed as periodic upkeep.
    pub upkeep_good: u8,
    /// Total upkeep units due each turn.
    pub upkeep_required: Units,
    /// Refined units deducted toward upkeep.
    pub paid_with_refined: Units,
    /// Existing `cred_debt` interest accrued this turn before upkeep shortfall.
    pub debt_interest_accrued: Units,
    /// Newly issued shortfall debt from unpaid upkeep.
    pub new_shortfall_debt: Units,
    /// Voluntary debt repaid during Step 2.
    pub debt_repaid_cred: Units,
}
