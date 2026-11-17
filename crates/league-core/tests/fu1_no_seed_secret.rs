//! FU1 regression guard: `league-core` must not expose seed-derived secret constructors
//! (`derive_from_seed`) outside `#[cfg(test)]`, and secret/entropy types must reject `[0u8; 32]`
//! and redact bytes in `Debug` output.

use std::fs;
use std::path::{Path, PathBuf};

use league_core::{PreimageSecret, RevealSalt, SealedEvalSeed};

fn collect_rs_files_recursive(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries =
        fs::read_dir(dir).unwrap_or_else(|err| panic!("failed to read {}: {err}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|err| panic!("failed to read dir entry: {err}"));
        let path = entry.path();
        let file_type = entry
            .file_type()
            .unwrap_or_else(|err| panic!("failed to read file type {}: {err}", path.display()));
        if file_type.is_dir() {
            collect_rs_files_recursive(&path, out);
        } else if file_type.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("rs")
        {
            out.push(path);
        }
    }
}

fn scan_src_tree_for_derive_from_seed(root: &Path) -> Result<Vec<String>, PathBuf> {
    let mut paths = Vec::new();
    collect_rs_files_recursive(root, &mut paths);
    paths.sort();

    let mut visited = Vec::with_capacity(paths.len());
    for path in paths {
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
        let prod_part = source.split("#[cfg(test)]").next().unwrap_or(&source);
        if prod_part.contains("derive_from_seed") {
            return Err(path);
        }
        if let Some(rel) = path.strip_prefix(root).ok().and_then(|p| p.to_str()) {
            visited.push(rel.replace('\\', "/"));
        }
    }
    Ok(visited)
}

#[test]
fn fu1_league_core_secrets_have_no_public_seed_derivation() {
    let src_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let scanned_files = match scan_src_tree_for_derive_from_seed(&src_dir) {
        Ok(files) => files,
        Err(path) => panic!(
            "FU1 violated: {} still exposes derive_from_seed in non-test code",
            path.display()
        ),
    };

    for required in [
        "commit.rs",
        "contracts.rs",
        "engine.rs",
        "lib.rs",
        "predicates.rs",
        "replay.rs",
        "scoring.rs",
        "types.rs",
    ] {
        assert!(
            scanned_files.iter().any(|name| name == required),
            "expected {required} in recursively scanned league-core src files: {scanned_files:?}"
        );
    }

    // Verify nested subdirectory recursion catches a non-test `derive_from_seed` inside a module
    // nested below `src/`.
    let temp_root = std::env::temp_dir().join(format!(
        "ml_fu1_recursive_guard_check_{}",
        std::process::id()
    ));
    let nested_dir = temp_root.join("nested").join("deep");
    let nested_file = nested_dir.join("leaky.rs");
    let create_res = fs::create_dir_all(&nested_dir)
        .and_then(|()| fs::write(&nested_file, "pub fn derive_from_seed() {}\n"));
    let scan_res = create_res
        .as_ref()
        .map(|()| scan_src_tree_for_derive_from_seed(&temp_root));
    let _ = fs::remove_dir_all(&temp_root);
    assert!(
        create_res.is_ok(),
        "failed to create nested temp fixture: {create_res:?}"
    );
    assert_eq!(scan_res.ok(), Some(Err(nested_file)));

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
