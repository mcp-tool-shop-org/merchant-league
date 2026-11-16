//! FU1 regression guard: `league-dice` must not expose seed-derived `Secret` constructors
//! outside `#[cfg(test)]`.

use league_dice::{DiceError, Secret};

#[test]
fn fu1_league_dice_secret_has_no_public_seed_derivation() {
    let lib_rs = include_str!("../src/lib.rs");
    let prod_part = lib_rs.split("#[cfg(test)]").next().unwrap_or(lib_rs);
    assert!(
        !prod_part.contains("derive_from_seed"),
        "FU1 violated: crates/league-dice/src/lib.rs still exposes derive_from_seed in non-test code"
    );

    assert_eq!(
        Secret::try_from_bytes([0u8; 32]),
        Err(DiceError::DegenerateSecret)
    );
    let Ok(generated) = Secret::generate() else {
        panic!("OS CSPRNG Secret::generate must succeed");
    };
    assert!(!generated.is_degenerate());
}
