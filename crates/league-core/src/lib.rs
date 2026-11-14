//! Merchant League game rules.
//!
//! **Types only.** The rules (towns, goods, market days, road events,
//! contracts, scoring) are not written yet: the game design is under an
//! outside review, and the rules wait until that review has been ruled on.
//! Nothing here should be read as a settled rule.
//!
//! The crate has no I/O. When the rules arrive they will be pure and
//! deterministic: the same seed and the same moves give the same table.

/// Identifies a table: one game of 2 to 6 merchants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TableId(pub u64);

/// A merchant's seat at a table, counted from 0 in seating order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Seat(pub u8);

/// A market day within a table, counted from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MarketDay(pub u16);

/// A season: the play between two resets of the test network.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SeasonId(pub u32);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_order_by_their_number() {
        assert!(Seat(0) < Seat(1));
        assert!(MarketDay(1) < MarketDay(12));
        assert_eq!(TableId(7), TableId(7));
        assert_ne!(SeasonId(1), SeasonId(2));
    }
}
