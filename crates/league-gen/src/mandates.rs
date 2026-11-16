//! Single canonical Turn-1 mandate deck shared between [`league_core`] and generated
//! [`crate::RingTable`] fixtures (`B-2`, #32 review).
//!
//! `league-gen` delegates mandate card types, deck generation, and CSPRNG deal permutations
//! directly to [`league_core`] so there is never a second diverging deck or shuffle implementation.
//! Note: conversion of provisional mandate points into terminal scoring lives separately in
//! [`league_core::convert_mandate_points_to_equity`].

use crate::schema::RingTable;
pub use league_core::{
    CommitmentHash, CreditTier, EngineError, GradedMandateCard, MandateKind, RevealSalt,
    SealedMandateDeal, TableId, UNIT_SCALE, Units, bonus_points_milli_for_completion_bps,
    deal_graded_mandates, deal_graded_mandates_with_entropy, deal_mandate_deck_indices,
    default_graded_mandate_deck, graded_mandate_deck_for_mean_project_value,
};

/// Returns the mean Tier-3 Syndicate Project point value in milli-units across `table`'s seats
/// (`round(mean(refined_project_value) * 1_000)`).
#[must_use]
pub fn mean_project_value_milli(table: &RingTable) -> Units {
    if table.seats.is_empty() {
        return 0;
    }
    let sum: f64 = table.seats.iter().map(|s| s.refined_project_value).sum();
    let mean = sum / (table.seats.len() as f64);
    ((mean * (UNIT_SCALE as f64)).round() as Units).max(1)
}

/// Builds the single canonical graded Turn-1 mandate deck for `table` directly from
/// [`league_core::graded_mandate_deck_for_mean_project_value`].
#[must_use]
pub fn mandate_deck_for_ring_table(table: &RingTable) -> Vec<GradedMandateCard> {
    let mean_milli = mean_project_value_milli(table);
    graded_mandate_deck_for_mean_project_value(table.seat_count.as_u8(), mean_milli)
}

/// Deals sealed Turn-1 mandates from `mandate_deck_for_ring_table(table)` via [`league_core`]'s
/// canonical CSPRNG-separated dealing routine.
pub fn deal_mandates_for_ring_table(
    table: &RingTable,
    table_id: TableId,
    deal_seed: [u8; 32],
    seat_salts: &[RevealSalt],
) -> Result<(Vec<CommitmentHash>, SealedMandateDeal), EngineError> {
    let deck = mandate_deck_for_ring_table(table);
    deal_graded_mandates_with_entropy(
        table_id,
        table.seat_count.as_u8(),
        deal_seed,
        seat_salts,
        &deck,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate_ring_table;
    use crate::schema::{GeneratorConfig, SeatCount};
    use league_core::{MAX_MANDATE_BONUS_EQUITY, MIN_MANDATE_BONUS_EQUITY};

    #[test]
    fn b2_single_mandate_deck_matches_league_core_card_for_card_and_deal() {
        let cfg = GeneratorConfig {
            seat_count: Some(SeatCount::Four),
            ..GeneratorConfig::default()
        };
        let (table, _) = match generate_ring_table(20261115, &cfg) {
            Ok(pair) => pair,
            Err(err) => panic!("ring table failed: {err}"),
        };

        let gen_deck = mandate_deck_for_ring_table(&table);
        let expected_mean = mean_project_value_milli(&table);
        assert!(
            expected_mean > 0,
            "ring table must expose positive project value"
        );
        let core_deck = graded_mandate_deck_for_mean_project_value(4, expected_mean);
        assert!(!gen_deck.is_empty(), "league-gen deck must not be empty");
        assert_eq!(
            gen_deck, core_deck,
            "league-gen must reuse league-core's single deck"
        );
        let embedded_core_cards: Vec<GradedMandateCard> = table
            .mandate_deck
            .iter()
            .map(|c| c.to_core_card())
            .collect();
        assert_eq!(
            embedded_core_cards, core_deck,
            "RingTable.mandate_deck must match league-core's single deck card for card"
        );
        let schema_deal = match crate::schema::deal_mandate_indices_with_entropy(
            &table.mandate_deck,
            SeatCount::Four,
            [0xA5; 32],
        ) {
            Ok(idxs) => idxs,
            Err(err) => panic!("schema deal failed: {err}"),
        };
        let core_table0_deal =
            match deal_mandate_deck_indices(TableId(0), 4, [0xA5; 32], core_deck.len()) {
                Ok(idxs) => idxs,
                Err(err) => panic!("core deal failed: {err}"),
            };
        assert_eq!(
            schema_deal, core_table0_deal,
            "deal_mandate_indices_with_entropy must use league-core's single shuffle"
        );

        let std_core = default_graded_mandate_deck(4);
        assert_eq!(std_core.len(), gen_deck.len());
        for card in &gen_deck {
            assert!(
                (MIN_MANDATE_BONUS_EQUITY..=MAX_MANDATE_BONUS_EQUITY)
                    .contains(&card.mandate_points_milli())
            );
            assert!(
                !matches!(
                    card.kind,
                    MandateKind::MaintainCleanCredit {
                        min_tier: CreditTier::B | CreditTier::D
                    }
                ),
                "B-2 single deck must never contain B or D credit tiers"
            );
            assert!(card.kind.validate_for_seat_count(4).is_ok());
        }

        // Verify deal permutation uses league-core's single shuffle direction & rejects zero deal_seed.
        assert!(deal_mandate_deck_indices(TableId(44), 4, [0u8; 32], gen_deck.len()).is_err());
        let expected_idxs =
            match deal_mandate_deck_indices(TableId(44), 4, [0xA5; 32], gen_deck.len()) {
                Ok(idxs) => idxs,
                Err(err) => panic!("deal_mandate_deck_indices failed: {err}"),
            };
        let salts: Vec<RevealSalt> = (0..4).map(|i| RevealSalt([0x31 + i; 32])).collect();
        let (commits, deal) =
            match deal_mandates_for_ring_table(&table, TableId(44), [0xA5; 32], &salts) {
                Ok(res) => res,
                Err(err) => panic!("deal_mandates_for_ring_table failed: {err}"),
            };
        assert_eq!(commits.len(), 4);
        assert_eq!(deal.seat_reveals.len(), 4);
        for (seat_idx, rev) in deal.seat_reveals.iter().enumerate() {
            let expected_card = &core_deck[expected_idxs[seat_idx]];
            assert_eq!(rev.mandate.kind, expected_card.kind);
            assert_eq!(rev.mandate.bonus_equity, expected_card.bonus_equity);
        }
    }
}
