//! FU1 regression guard: `league-core` must not expose seed-derived secret constructors
//! (`derive_from_seed`) outside `#[cfg(test)]`, and secret/entropy types must reject `[0u8; 32]`
//! and redact bytes in `Debug` output.

use std::fs;
use std::path::Path;

use league_core::{PreimageSecret, RevealSalt, SealedEvalSeed};

#[test]
fn fu1_league_core_secrets_have_no_public_seed_derivation() {
    let src_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let entries = fs::read_dir(&src_dir)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", src_dir.display()));

    let mut scanned_files = Vec::new();
    for entry in entries {
        let entry = entry.unwrap_or_else(|err| panic!("failed to read dir entry: {err}"));
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
        let prod_part = source.split("#[cfg(test)]").next().unwrap_or(&source);
        assert!(
            !prod_part.contains("derive_from_seed"),
            "FU1 violated: {} still exposes derive_from_seed in non-test code",
            path.display()
        );
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            scanned_files.push(name.to_owned());
        }
    }

    for required in ["commit.rs", "lib.rs", "engine.rs", "replay.rs"] {
        assert!(
            scanned_files.iter().any(|name| name == required),
            "expected {required} in scanned league-core src files: {scanned_files:?}"
        );
    }

    let zero_preimage = PreimageSecret([0u8; 32]);
    assert!(zero_preimage.is_degenerate());
    assert!(zero_preimage.validate().is_err());
    assert_eq!(
        format!("{:?}", PreimageSecret([0xAB; 32])),
        "PreimageSecret(<hidden>)"
    );

    let zero_salt = RevealSalt([0u8; 32]);
    assert!(zero_salt.is_degenerate());
    assert!(zero_salt.validate().is_err());
    assert_eq!(
        format!("{:?}", RevealSalt([0xAB; 32])),
        "RevealSalt(<hidden>)"
    );

    assert!(SealedEvalSeed::try_from_bytes([0u8; 32]).is_err());
    let Ok(generated) = SealedEvalSeed::generate() else {
        panic!("OS CSPRNG SealedEvalSeed::generate must succeed");
    };
    assert!(!generated.is_degenerate());
    assert_eq!(format!("{generated:?}"), "SealedEvalSeed(<hidden>)");
}
