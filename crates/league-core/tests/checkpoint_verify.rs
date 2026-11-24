//! Causal, canonical, binding, and Save-Load-Step verification tests for Evaluation checkpoints.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use league_core::eval::host::checkpoint_verify::{
    CheckpointVerifyError, compute_eval_save_v2_sha256_hex, decode_strict_checkpoint_pair,
    sha256_bytes_hex, verify_and_construct_pre_act_bytes, verify_checkpoint_causal,
    verify_checkpoint_causal_against_chain, verify_checkpoint_causal_with_probe_fault_for_test,
};
use league_core::{
    ClubBot, EVAL_SAVE_FORMAT_VERSION, EVAL_SEAL_FORMAT_VERSION, EngineError, EvalActSubmission,
    EvalHost, EvalSaveSnapshot, HARD_TURN_CAP, Seat, TableId, TurnPhase,
};
use sha2::{Digest, Sha256};

const COMMIT_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const COMMIT_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn must_ok<T, E: std::fmt::Display>(res: Result<T, E>, ctx: &str) -> T {
    match res {
        Ok(v) => v,
        Err(err) => panic!("{ctx}: {err}"),
    }
}

fn must_err<T, E>(res: Result<T, E>, ctx: &str) -> E {
    match res {
        Ok(_) => panic!("{ctx}: expected Err"),
        Err(err) => err,
    }
}

fn must_some<T>(opt: Option<T>, ctx: &str) -> T {
    match opt {
        Some(v) => v,
        None => panic!("{ctx}: expected Some"),
    }
}

fn unique_tmp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ml_ckpt_verify_{label}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    must_ok(fs::create_dir_all(&dir), "create tmp dir");
    dir
}

fn collect_bot_submissions(host: &EvalHost) -> BTreeMap<u8, EvalActSubmission> {
    let mut subs = BTreeMap::new();
    for s in 0..host.seat_count() {
        let obs = must_ok(host.observe_seat(Seat(s)), "observe_seat");
        let mandate = obs
            .own_mandate()
            .copied()
            .unwrap_or_else(|| panic!("seat {s} missing own_mandate"));
        let decision = ClubBot::new(host.bot_tie_seed().wrapping_add(u64::from(s)))
            .decide_full_turn(&obs, &mandate);
        subs.insert(
            s,
            EvalActSubmission {
                market_actions: decision.pass_a_actions,
                pass_b_actions: decision.pass_b_actions,
                bundle: decision.bundle,
            },
        );
    }
    subs
}

fn build_pre_act_checkpoint_at_turn(
    table_id: u64,
    layout_seed: u64,
    bot_tie_seed: u64,
    engine_commit: &str,
    target_turn: u8,
    label: &str,
) -> (PathBuf, EvalHost, Vec<u8>, Vec<u8>) {
    let tmp = unique_tmp_dir(label);
    let ckpt_dir = tmp.join("checkpoints");
    let mut host = must_ok(
        must_ok(
            EvalHost::from_standard_setup(TableId(table_id), 4, layout_seed, bot_tie_seed),
            "from_standard_setup",
        )
        .with_engine_commit(engine_commit),
        "with_engine_commit",
    );
    while host.turn() < target_turn {
        let subs = collect_bot_submissions(&host);
        must_ok(host.apply_act_submissions(&subs), "apply_act_submissions");
        let _ = must_ok(host.sweep_turn(), "sweep_turn");
    }
    assert_eq!(host.turn(), target_turn);
    let (save_path, seal_path) = must_ok(
        host.write_pre_act_checkpoint(&ckpt_dir),
        "write_pre_act_checkpoint",
    );
    let save_bytes = must_ok(fs::read(&save_path), "read save");
    let seal_bytes = must_ok(fs::read(&seal_path), "read seal");
    (tmp, host, save_bytes, seal_bytes)
}

/// Finds the byte span `(value_start, value_end)` of top-level or nested JSON field `key`
/// within compact `json` (preserving all surrounding key ordering and compact formatting).
fn find_json_field_value_span(json: &[u8], search_from: usize, key: &str) -> (usize, usize) {
    let needle = format!("\"{key}\":");
    let haystack = &json[search_from..];
    let rel = haystack
        .windows(needle.len())
        .position(|w| w == needle.as_bytes())
        .unwrap_or_else(|| panic!("key `{key}` not found in JSON"));
    let val_start = search_from + rel + needle.len();
    let val_end = find_json_value_end(json, val_start);
    (val_start, val_end)
}

fn find_json_value_end(json: &[u8], start: usize) -> usize {
    let first = json[start];
    match first {
        b'"' => {
            let mut i = start + 1;
            while i < json.len() {
                if json[i] == b'\\' {
                    i += 2;
                    continue;
                }
                if json[i] == b'"' {
                    return i + 1;
                }
                i += 1;
            }
            panic!("unterminated JSON string");
        }
        b'{' | b'[' => {
            let mut depth = 0_usize;
            let mut i = start;
            let mut in_str = false;
            while i < json.len() {
                let b = json[i];
                if in_str {
                    if b == b'\\' {
                        i += 2;
                        continue;
                    }
                    if b == b'"' {
                        in_str = false;
                    }
                } else {
                    match b {
                        b'"' => in_str = true,
                        b'{' | b'[' => depth += 1,
                        b'}' | b']' => {
                            depth -= 1;
                            if depth == 0 {
                                return i + 1;
                            }
                        }
                        _ => {}
                    }
                }
                i += 1;
            }
            panic!("unterminated JSON container");
        }
        _ => {
            let mut i = start;
            while i < json.len() && !matches!(json[i], b',' | b'}' | b']') {
                i += 1;
            }
            i
        }
    }
}

fn replace_span(buf: &[u8], span: (usize, usize), replacement: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(buf.len() + replacement.len());
    out.extend_from_slice(&buf[..span.0]);
    out.extend_from_slice(replacement);
    out.extend_from_slice(&buf[span.1..]);
    out
}

fn patch_seal_save_sha256(seal_bytes: &[u8], save_bytes: &[u8]) -> Vec<u8> {
    let new_sha = compute_eval_save_v2_sha256_hex(save_bytes);
    let span = find_json_field_value_span(seal_bytes, 0, "save_sha256");
    let repl = format!("\"{new_sha}\"");
    replace_span(seal_bytes, span, repl.as_bytes())
}

#[test]
fn m01_disk_bitflip_rejected_with_outer_file_hash_mismatch() {
    let (tmp, _host, save_bytes, seal_bytes) =
        build_pre_act_checkpoint_at_turn(930_001, 7, 101, COMMIT_A, 11, "m01");

    let expected_save = sha256_bytes_hex(&save_bytes);
    let expected_seal = sha256_bytes_hex(&seal_bytes);

    let mut flipped_save = save_bytes.clone();
    flipped_save[0] ^= 0x01;
    let err_save = must_err(
        verify_and_construct_pre_act_bytes(
            &flipped_save,
            &seal_bytes,
            Some(&expected_save),
            Some(&expected_seal),
            None,
            None,
        ),
        "bitflipped save must fail",
    );
    assert!(matches!(
        err_save,
        CheckpointVerifyError::OuterFileHashMismatch { ref file, .. } if file == "save"
    ));

    let mut flipped_seal = seal_bytes;
    flipped_seal[0] ^= 0x01;
    let err_seal = must_err(
        verify_and_construct_pre_act_bytes(
            &save_bytes,
            &flipped_seal,
            Some(&expected_save),
            Some(&expected_seal),
            None,
            None,
        ),
        "bitflipped seal must fail",
    );
    assert!(matches!(
        err_seal,
        CheckpointVerifyError::OuterFileHashMismatch { ref file, .. } if file == "seal"
    ));

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn m02_torn_tail_rejected_with_unexpected_eof() {
    let (tmp, _host, save_bytes, seal_bytes) =
        build_pre_act_checkpoint_at_turn(930_002, 7, 102, COMMIT_A, 11, "m02");

    let torn_save = &save_bytes[..save_bytes.len().saturating_sub(19)];
    let err_save = must_err(
        verify_checkpoint_causal(torn_save, &seal_bytes),
        "torn save must fail",
    );
    assert!(matches!(
        err_save,
        CheckpointVerifyError::UnexpectedEofOrMalformedEncoding { ref file, .. } if file == "save"
    ));

    let torn_seal = &seal_bytes[..seal_bytes.len().saturating_sub(19)];
    let err_seal = must_err(
        verify_and_construct_pre_act_bytes(
            torn_seal_or_save(&save_bytes),
            torn_seal,
            None,
            None,
            None,
            None,
        ),
        "torn seal must fail",
    );
    assert!(matches!(
        err_seal,
        CheckpointVerifyError::UnexpectedEofOrMalformedEncoding { ref file, .. } if file == "seal"
    ));

    let mut trailing_garbage = seal_bytes;
    trailing_garbage.extend_from_slice(b"trailing");
    let err_trail = must_err(
        verify_checkpoint_causal(&save_bytes, &trailing_garbage),
        "trailing garbage must fail",
    );
    assert!(matches!(
        err_trail,
        CheckpointVerifyError::UnexpectedEofOrMalformedEncoding { ref file, .. } if file == "seal"
    ));

    let _ = fs::remove_dir_all(&tmp);
}

fn torn_seal_or_save(bytes: &[u8]) -> &[u8] {
    bytes
}

#[test]
fn m03_cross_game_and_d2_duplicate_swap_rejected_with_pair_binding_mismatch() {
    // (a) Game A day-11 save paired with Game B day-11 seal (equal turn & logged_turns.len() == 10).
    let (tmp_a, _ha, save_a, _seal_a) =
        build_pre_act_checkpoint_at_turn(930_003, 7, 103, COMMIT_A, 11, "m03a");
    let (tmp_b, _hb, _save_b, seal_b) =
        build_pre_act_checkpoint_at_turn(930_003, 7, 104, COMMIT_A, 11, "m03b");
    let err_a = must_err(
        verify_checkpoint_causal(&save_a, &seal_b),
        "cross-game day-11 swap must fail",
    );
    assert!(matches!(
        err_a,
        CheckpointVerifyError::PairBindingMismatch { .. }
    ));

    // (b) Turn-1 D2 duplicate swap (identical public snapshot except game_id).
    let tmp_dup = unique_tmp_dir("m03_dup");
    let src = must_ok(
        must_ok(
            EvalHost::from_standard_setup(TableId(930_004), 4, 7, 200),
            "src",
        )
        .with_engine_commit(COMMIT_A),
        "commit",
    );
    let src_dir = tmp_dup.join("src");
    let (src_save, src_seal) = must_ok(src.write_game_save_and_seal(&src_dir), "write src");
    let mut dup1 = must_ok(
        EvalHost::fork_duplicate_from_turn_0(&src_save, &src_seal),
        "dup1",
    );
    let mut dup2 = must_ok(
        EvalHost::fork_duplicate_from_turn_0(&src_save, &src_seal),
        "dup2",
    );
    let (d1_save_p, _) = must_ok(
        dup1.write_pre_act_checkpoint(&tmp_dup.join("dup1/ckpts")),
        "d1",
    );
    let (_, d2_seal_p) = must_ok(
        dup2.write_pre_act_checkpoint(&tmp_dup.join("dup2/ckpts")),
        "d2",
    );
    let d1_save = must_ok(fs::read(&d1_save_p), "read d1 save");
    let d2_seal = must_ok(fs::read(&d2_seal_p), "read d2 seal");
    let err_b = must_err(
        verify_and_construct_pre_act_bytes(&d1_save, &d2_seal, None, None, None, None),
        "D2 duplicate swap must fail",
    );
    assert!(matches!(
        err_b,
        CheckpointVerifyError::PairBindingMismatch { .. }
    ));

    // Also test same game_id with stale save_sha256.
    let (_tmp_s, _hs, save_s, seal_s) = (
        tmp_dup.join("same"),
        src,
        d1_save,
        must_ok(
            fs::read(d1_save_p.with_file_name("day-01-pre-act.seal")),
            "read d1 seal",
        ),
    );
    let bad_sha_seal = replace_span(
        &seal_s,
        find_json_field_value_span(&seal_s, 0, "save_sha256"),
        format!("\"{}\"", "0".repeat(64)).as_bytes(),
    );
    let err_sha = must_err(
        verify_and_construct_pre_act_bytes(&save_s, &bad_sha_seal, None, None, None, None),
        "tampered save_sha256 must fail",
    );
    assert!(matches!(
        err_sha,
        CheckpointVerifyError::PairBindingMismatch { .. }
    ));

    let _ = fs::remove_dir_all(&tmp_a);
    let _ = fs::remove_dir_all(&tmp_b);
    let _ = fs::remove_dir_all(&tmp_dup);
}

#[test]
fn m04_turn_offset_and_digest_len_mismatch_rejected_with_tip_state_digest_mismatch() {
    let tmp = unique_tmp_dir("m04");
    let ckpt_dir = tmp.join("checkpoints");
    let mut host = must_ok(
        must_ok(
            EvalHost::from_standard_setup(TableId(930_005), 4, 7, 300),
            "setup",
        )
        .with_engine_commit(COMMIT_A),
        "commit",
    );
    while host.turn() < 10 {
        let subs = collect_bot_submissions(&host);
        must_ok(host.apply_act_submissions(&subs), "apply");
        let _ = must_ok(host.sweep_turn(), "sweep");
    }
    let (day10_save_p, _) = must_ok(host.write_pre_act_checkpoint(&ckpt_dir), "write day-10");
    let subs10 = collect_bot_submissions(&host);
    must_ok(host.apply_act_submissions(&subs10), "apply 10");
    let _ = must_ok(host.sweep_turn(), "sweep 10");
    let (day11_save_p, day11_seal_p) =
        must_ok(host.write_pre_act_checkpoint(&ckpt_dir), "write day-11");

    let day10_save = must_ok(fs::read(&day10_save_p), "read day10 save");
    let day11_save = must_ok(fs::read(&day11_save_p), "read day11 save");
    let day11_seal = must_ok(fs::read(&day11_seal_p), "read day11 seal");

    // (a) Same-game day-10 save paired with day-11 seal.
    let err_offset = must_err(
        verify_checkpoint_causal(&day10_save, &day11_seal),
        "turn offset save+seal pair must fail",
    );
    assert!(matches!(
        err_offset,
        CheckpointVerifyError::TipStateDigestMismatch { turn: 11, .. }
    ));

    // (b) Truncated live_state_digests in seal with recomputed save_sha256.
    let trunc_digests_seal = replace_span(
        &day11_seal,
        find_json_field_value_span(&day11_seal, 0, "live_state_digests"),
        b"[]",
    );
    let err_len = must_err(
        verify_and_construct_pre_act_bytes(
            &day11_save,
            &trunc_digests_seal,
            None,
            None,
            None,
            None,
        ),
        "live_state_digests len mismatch must fail",
    );
    assert!(matches!(
        err_len,
        CheckpointVerifyError::TipStateDigestMismatch { turn: 11, .. }
    ));

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn m05_co_written_state_and_tip_digest_bug_rejected_by_gate2_replay() {
    let (tmp, _host, save_bytes, seal_bytes) =
        build_pre_act_checkpoint_at_turn(930_006, 7, 400, COMMIT_A, 11, "m05");

    // Mutate seat 0's `cred` consistently across BOTH `.save.json` and `seal.engine.state`,
    // update `live_state_digests[9]` to match the mutated state digest, and recompute `save_sha256`.
    let mut save: EvalSaveSnapshot = must_ok(serde_json::from_slice(&save_bytes), "parse save");
    let old_cred = save.seats[0].cred;
    let new_cred = old_cred + 777;
    save.seats[0].cred = new_cred;
    let mutated_save_bytes = must_ok(serde_json::to_vec_pretty(&save), "ser mutated save");

    let state_span = find_json_field_value_span(&seal_bytes, 0, "state");
    let state_slice = &seal_bytes[state_span.0..state_span.1];
    let cred_needle = format!("\"cred\":{old_cred}");
    let cred_rel = must_some(
        state_slice
            .windows(cred_needle.len())
            .position(|w| w == cred_needle.as_bytes()),
        "cred needle in compact EngineState",
    );
    let mut new_state_slice = Vec::with_capacity(state_slice.len() + 8);
    new_state_slice.extend_from_slice(&state_slice[..cred_rel]);
    new_state_slice.extend_from_slice(format!("\"cred\":{new_cred}").as_bytes());
    new_state_slice.extend_from_slice(&state_slice[cred_rel + cred_needle.len()..]);

    let mut hasher = Sha256::new();
    hasher.update(b"league-core/state/v2");
    hasher.update(&new_state_slice);
    let forged_tip_digest: [u8; 32] = hasher.finalize().into();

    let seal_with_state = replace_span(&seal_bytes, state_span, &new_state_slice);
    let digests_span = find_json_field_value_span(&seal_with_state, 0, "live_state_digests");
    let mut digests: Vec<[u8; 32]> = must_ok(
        serde_json::from_slice(&seal_with_state[digests_span.0..digests_span.1]),
        "parse live_state_digests",
    );
    digests[9] = forged_tip_digest;
    let new_digests_json = must_ok(serde_json::to_vec(&digests), "ser forged digests");
    let seal_with_digests = replace_span(&seal_with_state, digests_span, &new_digests_json);
    let forged_seal_bytes = patch_seal_save_sha256(&seal_with_digests, &mutated_save_bytes);

    // Gate 3 tip digest consistency check (against stale forged_tip_digest? Wait: Gate 3 without
    // Gate 2 trusts forged tip digest and succeeds, showing WHY Gate 2 is necessary!).
    assert!(
        verify_and_construct_pre_act_bytes(
            &mutated_save_bytes,
            &forged_seal_bytes,
            None,
            None,
            None,
            None,
        )
        .is_ok()
    );

    // Gate 2 Turn-0 replay catches the co-written bug at turn 10!
    let err = must_err(
        verify_checkpoint_causal(&mutated_save_bytes, &forged_seal_bytes),
        "Gate 2 must catch M05 co-written state bug",
    );
    assert!(matches!(
        err,
        CheckpointVerifyError::ReplayDigestMismatch { turn: 10, .. }
    ));

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn m06_mid_log_edit_and_chain_hash_mismatch_rejected() {
    let (tmp, _host, save_bytes, seal_bytes) =
        build_pre_act_checkpoint_at_turn(930_007, 7, 500, COMMIT_A, 11, "m06");

    let (_, clean_summary) = must_ok(
        verify_checkpoint_causal(&save_bytes, &seal_bytes),
        "clean Gate 2",
    );

    // (a) Flip 1 byte in live_state_digests[3] (Turn 4).
    let digests_span = find_json_field_value_span(&seal_bytes, 0, "live_state_digests");
    let mut digests: Vec<[u8; 32]> = must_ok(
        serde_json::from_slice(&seal_bytes[digests_span.0..digests_span.1]),
        "parse live_state_digests",
    );
    digests[3][0] ^= 0xFF;
    let bad_digests_json = must_ok(serde_json::to_vec(&digests), "ser bad digests");
    let bad_seal = replace_span(&seal_bytes, digests_span, &bad_digests_json);
    let err_turn4 = must_err(
        verify_checkpoint_causal(&save_bytes, &bad_seal),
        "mid-log digest edit must fail at turn 4",
    );
    assert!(matches!(
        err_turn4,
        CheckpointVerifyError::ReplayDigestMismatch { turn: 4, .. }
    ));

    // (b) Tampered expected cumulative turn-chain digest raises ChainHashMismatch.
    let mut skewed_chain = clean_summary.turn_chain_digest;
    let replacement = if skewed_chain.starts_with('0') {
        "1"
    } else {
        "0"
    };
    skewed_chain.replace_range(0..1, replacement);
    let err_chain = must_err(
        verify_checkpoint_causal_against_chain(&save_bytes, &seal_bytes, &skewed_chain),
        "chain digest divergence must raise ChainHashMismatch",
    );
    assert!(matches!(
        err_chain,
        CheckpointVerifyError::ChainHashMismatch { turn: 10, .. }
    ));

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn m07_non_canonical_json_rejected() {
    let (tmp, _host, save_bytes, seal_bytes) =
        build_pre_act_checkpoint_at_turn(930_008, 7, 600, COMMIT_A, 11, "m07");

    // 1. Pretty-printed .seal instead of compact canonical JSON.
    let seal_val: serde_json::Value = must_ok(serde_json::from_slice(&seal_bytes), "parse seal");
    let pretty_seal = must_ok(serde_json::to_vec_pretty(&seal_val), "pretty seal");
    let err_pretty = must_err(
        verify_checkpoint_causal(&save_bytes, &pretty_seal),
        "pretty seal must be rejected as non-canonical",
    );
    assert!(matches!(
        err_pretty,
        CheckpointVerifyError::NonCanonicalEncoding { ref file, .. } if file == "seal"
    ));

    // 2. Key-reordered .save.json (serde_json::Value sorts keys alphabetically).
    let save_val: serde_json::Value = must_ok(serde_json::from_slice(&save_bytes), "parse save");
    let reordered_save = must_ok(serde_json::to_vec_pretty(&save_val), "reordered save");
    let err_reordered = must_err(
        verify_checkpoint_causal(&reordered_save, &seal_bytes),
        "key-reordered save must be rejected",
    );
    assert!(matches!(
        err_reordered,
        CheckpointVerifyError::NonCanonicalEncoding { ref file, .. } if file == "save"
    ));

    // 3. Single trailing newline / whitespace byte appended to .save.json.
    let mut trailing_save = save_bytes.clone();
    trailing_save.push(b'\n');
    let err_trailing = must_err(
        verify_checkpoint_causal(&trailing_save, &seal_bytes),
        "trailing whitespace byte must fail",
    );
    assert!(matches!(
        err_trailing,
        CheckpointVerifyError::NonCanonicalEncoding { ref file, .. } if file == "save"
    ));

    // 4. Duplicate JSON object key nested inside seal.engine.state.
    let dup_key_seal = replace_span(
        &seal_bytes,
        find_json_field_value_span(&seal_bytes, 0, "cancel_locked"),
        b"false,\"cancel_locked\":false",
    );
    let err_dup = must_err(
        verify_checkpoint_causal(&save_bytes, &dup_key_seal),
        "duplicate nested JSON key must fail",
    );
    assert!(matches!(
        err_dup,
        CheckpointVerifyError::NonCanonicalEncoding { ref file, .. } if file == "seal"
    ));

    // 5. Explicitly emitted `skip_serializing_if` optional default (`"pre_act_checkpoint_sha256":null`).
    let mut skipped_seal = seal_bytes;
    let insert_pos = find_json_field_value_span(&skipped_seal, 0, "game_id").1;
    let skipped_field = b",\"pre_act_checkpoint_sha256\":null";
    let mut with_skipped = Vec::with_capacity(skipped_seal.len() + skipped_field.len());
    with_skipped.extend_from_slice(&skipped_seal[..insert_pos]);
    with_skipped.extend_from_slice(skipped_field);
    with_skipped.extend_from_slice(&skipped_seal[insert_pos..]);
    let err_skip = must_err(
        verify_checkpoint_causal(&save_bytes, &with_skipped),
        "explicit null on skip_serializing_if field must fail NonCanonicalEncoding",
    );
    assert!(matches!(
        err_skip,
        CheckpointVerifyError::NonCanonicalEncoding { ref file, .. } if file == "seal"
    ));
    let _ = &mut skipped_seal;

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn m08_unknown_field_rejected_top_level_and_nested() {
    let (tmp, _host, save_bytes, seal_bytes) =
        build_pre_act_checkpoint_at_turn(930_009, 7, 700, COMMIT_A, 11, "m08");

    // 1. Unknown top-level field in .save.json.
    let save_end = save_bytes.len() - 1;
    let mut unk_save = save_bytes[..save_end].to_vec();
    unk_save.extend_from_slice(b",\n  \"extra_probe_key\": 1\n}");
    let err_save = must_err(
        verify_checkpoint_causal(&unk_save, &seal_bytes),
        "unknown save field must fail",
    );
    assert!(matches!(
        err_save,
        CheckpointVerifyError::UnknownFieldRejected { ref file, ref field_path }
            if file == "save" && field_path == "extra_probe_key"
    ));

    // 2. Unknown top-level field in .seal.
    let mut unk_seal_top = seal_bytes[..seal_bytes.len() - 1].to_vec();
    unk_seal_top.extend_from_slice(b",\"extra_probe_key\":1}");
    let err_seal_top = must_err(
        verify_checkpoint_causal(&save_bytes, &unk_seal_top),
        "unknown top-level seal field must fail",
    );
    assert!(matches!(
        err_seal_top,
        CheckpointVerifyError::UnknownFieldRejected { ref file, ref field_path }
            if file == "seal" && field_path == "extra_probe_key"
    ));

    // 3. Unknown nested field inside seal.engine.state.
    let state_span = find_json_field_value_span(&seal_bytes, 0, "state");
    let insert_at = state_span.1 - 1;
    let mut unk_seal_nested = Vec::with_capacity(seal_bytes.len() + 24);
    unk_seal_nested.extend_from_slice(&seal_bytes[..insert_at]);
    unk_seal_nested.extend_from_slice(b",\"extra_nested_key\":99");
    unk_seal_nested.extend_from_slice(&seal_bytes[insert_at..]);
    let err_nested = must_err(
        verify_checkpoint_causal(&save_bytes, &unk_seal_nested),
        "unknown nested seal field must fail",
    );
    assert!(matches!(
        err_nested,
        CheckpointVerifyError::UnknownFieldRejected { ref file, ref field_path }
            if file == "seal" && field_path == "engine.state.extra_nested_key"
    ));

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn m09_version_and_invalid_engine_commit_rejected() {
    let (tmp, _host, save_bytes, seal_bytes) =
        build_pre_act_checkpoint_at_turn(930_010, 7, 800, COMMIT_A, 11, "m09");

    // 1. Legacy v1 in save and seal refused fail-closed.
    let mut save_v1_val: serde_json::Value = must_ok(serde_json::from_slice(&save_bytes), "parse");
    save_v1_val["format_version"] = serde_json::json!(1);
    let save_v1 = must_ok(serde_json::to_vec(&save_v1_val), "ser");
    assert!(matches!(
        verify_checkpoint_causal(&save_v1, &seal_bytes),
        Err(CheckpointVerifyError::IncompatibleEngineOrSchemaVersion { ref field, ref found, .. })
            if field == "save.format_version" && found == "1"
    ));

    let mut seal_v1_val: serde_json::Value = must_ok(serde_json::from_slice(&seal_bytes), "parse");
    seal_v1_val["format_version"] = serde_json::json!(1);
    let seal_v1 = must_ok(serde_json::to_vec(&seal_v1_val), "ser");
    assert!(matches!(
        verify_checkpoint_causal(&save_bytes, &seal_v1),
        Err(CheckpointVerifyError::IncompatibleEngineOrSchemaVersion { ref field, ref found, .. })
            if field == "seal.format_version" && found == "1"
    ));

    // Also verify EvalHost::fork_from_verified_checkpoint_bytes preserves UnsupportedReplayVersion for v1.
    assert_eq!(
        EvalHost::fork_from_verified_checkpoint_bytes(&save_bytes, &seal_v1).err(),
        Some(EngineError::UnsupportedReplayVersion {
            found: 1,
            newest_known: EVAL_SEAL_FORMAT_VERSION,
        })
    );
    assert_eq!(
        EvalHost::fork_from_verified_checkpoint_bytes(&save_v1, &seal_bytes).err(),
        Some(EngineError::UnsupportedReplayVersion {
            found: 1,
            newest_known: EVAL_SAVE_FORMAT_VERSION,
        })
    );

    // 2. Wrong seal.save_format_version.
    let bad_sfv = replace_span(
        &seal_bytes,
        find_json_field_value_span(&seal_bytes, 0, "save_format_version"),
        b"1",
    );
    assert!(matches!(
        verify_checkpoint_causal(&save_bytes, &bad_sfv),
        Err(CheckpointVerifyError::IncompatibleEngineOrSchemaVersion { ref field, .. })
            if field == "seal.save_format_version"
    ));

    // 3. Empty, short, and uppercase seal.engine_commit refused fail-closed.
    for bad_commit in ["", "deadbeef", &"A".repeat(40)] {
        let bad_seal = replace_span(
            &seal_bytes,
            find_json_field_value_span(&seal_bytes, 0, "engine_commit"),
            format!("\"{bad_commit}\"").as_bytes(),
        );
        assert!(matches!(
            verify_checkpoint_causal(&save_bytes, &bad_seal),
            Err(CheckpointVerifyError::IncompatibleEngineOrSchemaVersion { ref field, .. })
                if field == "seal.engine_commit"
        ));
    }

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn engine_commit_is_provenance_not_equality_gate() {
    // 1. A checkpoint written at COMMIT_A verifies cleanly under Gate 2 regardless of the
    //    verifier's own commit, preserving COMMIT_A in VerifiedCheckpointCausalSummary::engine_commit.
    let (tmp, _host, save_bytes, seal_a) =
        build_pre_act_checkpoint_at_turn(930_011, 7, 900, COMMIT_A, 11, "prov_p1");
    let (_, summary_a) = must_ok(
        verify_checkpoint_causal(&save_bytes, &seal_a),
        "verify COMMIT_A",
    );
    assert_eq!(summary_a.engine_commit, COMMIT_A);

    let seal_b = replace_span(
        &seal_a,
        find_json_field_value_span(&seal_a, 0, "engine_commit"),
        format!("\"{COMMIT_B}\"").as_bytes(),
    );
    let (_, summary_b) = must_ok(
        verify_checkpoint_causal(&save_bytes, &seal_b),
        "verify COMMIT_B",
    );
    assert_eq!(summary_b.engine_commit, COMMIT_B);
    assert_eq!(
        summary_a.expected_state_digest,
        summary_b.expected_state_digest
    );

    // 2. When a state/rule change causes replay in the current engine to diverge from historical
    //    transitions, Gate 2 refuses it with ReplayDigestMismatch even if tip digests match.
    let mut save: EvalSaveSnapshot = must_ok(serde_json::from_slice(&save_bytes), "parse save");
    let old_cred = save.seats[1].cred;
    let new_cred = old_cred + 500;
    save.seats[1].cred = new_cred;
    let mutated_save = must_ok(serde_json::to_vec_pretty(&save), "ser mutated");

    let state_span = find_json_field_value_span(&seal_b, 0, "state");
    let state_slice = &seal_b[state_span.0..state_span.1];
    let parities_span = find_json_field_value_span(state_slice, 0, "parities");
    let seats_span = find_json_field_value_span(state_slice, parities_span.1, "seats");
    let seat0_end = find_json_value_end(state_slice, seats_span.0 + 1);
    assert_eq!(state_slice[seat0_end], b',');
    let seat1_start = seat0_end + 1;
    let seat1_end = find_json_value_end(state_slice, seat1_start);
    let needle = format!("\"cred\":{old_cred}");
    let rel = must_some(
        state_slice[seat1_start..seat1_end]
            .windows(needle.len())
            .position(|w| w == needle.as_bytes())
            .map(|p| seat1_start + p),
        "seat 1 cred in state",
    );
    let mut new_state = Vec::with_capacity(state_slice.len() + 8);
    new_state.extend_from_slice(&state_slice[..rel]);
    new_state.extend_from_slice(format!("\"cred\":{new_cred}").as_bytes());
    new_state.extend_from_slice(&state_slice[rel + needle.len()..]);

    let mut hasher = Sha256::new();
    hasher.update(b"league-core/state/v2");
    hasher.update(&new_state);
    let forged_tip: [u8; 32] = hasher.finalize().into();

    let seal_state = replace_span(&seal_b, state_span, &new_state);
    let seal_tip_only = patch_seal_save_sha256(&seal_state, &mutated_save);
    assert!(matches!(
        verify_checkpoint_causal(&mutated_save, &seal_tip_only),
        Err(CheckpointVerifyError::TipStateDigestMismatch { .. })
    ));
    let digests_span = find_json_field_value_span(&seal_state, 0, "live_state_digests");
    let mut digests: Vec<[u8; 32]> = must_ok(
        serde_json::from_slice(&seal_state[digests_span.0..digests_span.1]),
        "parse digests",
    );
    digests[9] = forged_tip;
    let seal_digests = replace_span(
        &seal_state,
        digests_span,
        &must_ok(serde_json::to_vec(&digests), "ser"),
    );
    let diverged_seal = patch_seal_save_sha256(&seal_digests, &mutated_save);

    let err_div = must_err(
        verify_checkpoint_causal(&mutated_save, &diverged_seal),
        "diverged rule/state replay must be refused",
    );
    assert!(
        matches!(
            err_div,
            CheckpointVerifyError::ReplayDigestMismatch { turn: 10, .. }
        ),
        "unexpected err_div: {err_div:?}"
    );

    let _ = fs::remove_dir_all(&tmp);
}

fn fault_loaded_probe(host: &mut EvalHost) {
    let _ = host.begin_cancel_lock();
}

#[test]
fn probe_step_divergence_negative_control_caught_by_gate2() {
    let (tmp, _host, save_bytes, seal_bytes) =
        build_pre_act_checkpoint_at_turn(930_012, 7, 1_000, COMMIT_A, 11, "probe_neg");

    let err = must_err(
        verify_checkpoint_causal_with_probe_fault_for_test(
            &save_bytes,
            &seal_bytes,
            fault_loaded_probe,
        ),
        "injected probe divergence must fail Gate 2",
    );
    assert!(matches!(
        err,
        CheckpointVerifyError::ProbeStepDivergence { turn: 11, .. }
    ));

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn save_load_step_commutativity_across_full_season() {
    let tmp = unique_tmp_dir("save_load_step");
    let mut live_host = must_ok(
        must_ok(
            EvalHost::from_standard_setup(TableId(930_013), 4, 7, 1_100),
            "setup",
        )
        .with_engine_commit(COMMIT_A),
        "commit",
    );

    let mut turns_checked = 0_u8;
    let mut sampled_mid_season_bytes: Option<(Vec<u8>, Vec<u8>)> = None;

    while live_host.phase() == TurnPhase::TalkAndMarket {
        let day = live_host.turn();
        let turn_dir = tmp.join(format!("turn_{day:02}"));
        let (save_p, seal_p) = must_ok(
            live_host.write_pre_act_checkpoint(&turn_dir),
            "write live pre-act",
        );
        let save_bytes = must_ok(fs::read(&save_p), "read save");
        let seal_bytes = must_ok(fs::read(&seal_p), "read seal");
        let (mut reloaded, summary) = must_ok(
            verify_checkpoint_causal(&save_bytes, &seal_bytes),
            "Gate 2 pre-act",
        );
        assert_eq!(summary.turn, day);

        if day == 8 {
            sampled_mid_season_bytes = Some((save_bytes.clone(), seal_bytes.clone()));
        }

        let subs = collect_bot_submissions(&live_host);
        must_ok(live_host.apply_act_submissions(&subs), "live apply");
        let live_receipt = must_ok(live_host.sweep_turn(), "live sweep");

        must_ok(reloaded.apply_act_submissions(&subs), "reloaded apply");
        let reload_receipt = must_ok(reloaded.sweep_turn(), "reloaded sweep");
        assert_eq!(reload_receipt, live_receipt, "turn {day} receipt diverged");
        assert_eq!(
            must_ok(reloaded.canonical_state_digest_hex(), "reloaded digest"),
            must_ok(live_host.canonical_state_digest_hex(), "live digest"),
            "turn {day} state_digest diverged after save-load-step"
        );

        let rt_dir = tmp.join(format!("rt_turn_{day:02}"));
        let (live_post_save, live_post_seal) = must_ok(
            live_host.write_game_save_and_seal(&rt_dir.join("live")),
            "write live post",
        );
        let (reload_post_save, reload_post_seal) = must_ok(
            reloaded.write_game_save_and_seal(&rt_dir.join("reload")),
            "write reload post",
        );
        assert_eq!(
            must_ok(fs::read(&live_post_save), "read live post save"),
            must_ok(fs::read(&reload_post_save), "read reload post save"),
            "turn {day} post-step .save.json diverged"
        );
        assert_eq!(
            must_ok(fs::read(&live_post_seal), "read live post seal"),
            must_ok(fs::read(&reload_post_seal), "read reload post seal"),
            "turn {day} post-step .seal diverged"
        );
        turns_checked = day;
    }

    assert!(
        turns_checked >= 12,
        "expected >= 12 turns before season close, played {turns_checked}"
    );
    assert_eq!(live_host.phase(), TurnPhase::AwaitingMandateReveal);

    // Mid-season Turn-8 reload run to full season close must match live final scoreboard identically.
    let (mid_save, mid_seal) = must_some(sampled_mid_season_bytes, "day 8 checkpoint captured");
    let mut forked_mid = must_ok(
        EvalHost::fork_from_verified_checkpoint_bytes(&mid_save, &mid_seal),
        "fork mid-season",
    );
    while forked_mid.phase() == TurnPhase::TalkAndMarket {
        let subs = collect_bot_submissions(&forked_mid);
        must_ok(forked_mid.apply_act_submissions(&subs), "mid apply");
        let _ = must_ok(forked_mid.sweep_turn(), "mid sweep");
    }
    let (live_board, _, _, _) = must_ok(live_host.finalize_eval_season(), "finalize live");
    let (mid_board, _, _, _) = must_ok(forked_mid.finalize_eval_season(), "finalize forked_mid");
    assert_eq!(mid_board, live_board);

    let _ = fs::remove_dir_all(&tmp);
}

fn write_child_checkpoint(out_dir: &Path) {
    let mut host = must_ok(
        must_ok(
            EvalHost::from_explicit_entropy(
                TableId(930_014),
                4,
                7,
                [0x44; 32],
                &[[0x11; 32], [0x22; 32], [0x33; 32], [0x45; 32]],
                [0x66; 32],
                1_200,
            ),
            "from_explicit_entropy",
        )
        .with_engine_commit(COMMIT_A),
        "with_engine_commit",
    );
    while host.turn() < 6 {
        let subs = collect_bot_submissions(&host);
        must_ok(host.apply_act_submissions(&subs), "child apply");
        let _ = must_ok(host.sweep_turn(), "child sweep");
    }
    must_ok(
        host.write_pre_act_checkpoint(out_dir),
        "child write_pre_act_checkpoint",
    );
}

#[test]
fn cross_process_round_trip_byte_equality() {
    if std::env::var_os("ML_TEST_SUBPROCESS").as_deref() == Some(OsStr::new("write_checkpoint")) {
        let out_dir = PathBuf::from(must_some(
            std::env::var_os("ML_CHILD_CKPT_OUT_DIR"),
            "ML_CHILD_CKPT_OUT_DIR set",
        ));
        write_child_checkpoint(&out_dir);
        return;
    }

    let tmp = unique_tmp_dir("cross_proc_rt");
    let child_dir = tmp.join("child_ckpts");
    let exe = must_ok(std::env::current_exe(), "current_exe");
    let output = must_ok(
        Command::new(exe)
            .args([
                "--exact",
                "cross_process_round_trip_byte_equality",
                "--nocapture",
            ])
            .env("ML_TEST_SUBPROCESS", "write_checkpoint")
            .env("ML_CHILD_CKPT_OUT_DIR", &child_dir)
            .stdin(Stdio::null())
            .output(),
        "spawn child writer process",
    );
    assert!(
        output.status.success(),
        "child writer process failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let save_path = child_dir.join("day-06-pre-act.save.json");
    let seal_path = child_dir.join("day-06-pre-act.seal");
    let save_bytes = must_ok(fs::read(&save_path), "read child save");
    let seal_bytes = must_ok(fs::read(&seal_path), "read child seal");

    let (mut verified_host, summary) = must_ok(
        verify_checkpoint_causal(&save_bytes, &seal_bytes),
        "parent process verify_checkpoint_causal",
    );
    assert_eq!(summary.turn, 6);
    assert_eq!(summary.engine_commit, COMMIT_A);

    let parent_dir = tmp.join("parent_ckpts");
    let (rt_save_path, rt_seal_path) = must_ok(
        verified_host.write_pre_act_checkpoint(&parent_dir),
        "parent re-serialize pre-act checkpoint",
    );
    assert_eq!(
        must_ok(fs::read(&rt_save_path), "read parent save"),
        save_bytes,
        "cross-process .save.json round-trip bytes differed"
    );
    assert_eq!(
        must_ok(fs::read(&rt_seal_path), "read parent seal"),
        seal_bytes,
        "cross-process .seal round-trip bytes differed"
    );

    // Explicitly verify HARD_TURN_CAP boundary constant is exercised by Turn 32 check sanity.
    assert_eq!(HARD_TURN_CAP, 32);

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn strict_error_variants_skip_serializing_fields_and_action_replay_paths() {
    use league_core::ToStructuredError;
    use league_core::eval::host::checkpoint_verify::parse_strict_json_value;

    // 1. Exercise every CheckpointVerifyError variant's code, hint, Display, and StructuredError.
    let all_variants = [
        CheckpointVerifyError::OuterFileHashMismatch {
            file: "save".to_owned(),
            expected: "a".repeat(64),
            actual: "b".repeat(64),
        },
        CheckpointVerifyError::UnexpectedEofOrMalformedEncoding {
            file: "seal".to_owned(),
            line: 1,
            column: 2,
            detail: "eof".to_owned(),
        },
        CheckpointVerifyError::PairBindingMismatch {
            reason: "bad".to_owned(),
        },
        CheckpointVerifyError::TipStateDigestMismatch {
            turn: 3,
            expected: "e".repeat(64),
            actual: "f".repeat(64),
            detail: "tip".to_owned(),
        },
        CheckpointVerifyError::ReplayDigestMismatch {
            turn: 2,
            expected: "c".repeat(64),
            actual: "d".repeat(64),
        },
        CheckpointVerifyError::ChainHashMismatch {
            turn: 2,
            expected: "1".repeat(64),
            actual: "2".repeat(64),
        },
        CheckpointVerifyError::ProbeStepDivergence {
            turn: 3,
            replayed_digest: "3".repeat(64),
            loaded_digest: "4".repeat(64),
        },
        CheckpointVerifyError::NonCanonicalEncoding {
            file: "save".to_owned(),
            detail: "ws".to_owned(),
        },
        CheckpointVerifyError::UnknownFieldRejected {
            file: "seal".to_owned(),
            field_path: "x".to_owned(),
        },
        CheckpointVerifyError::IncompatibleEngineOrSchemaVersion {
            field: "seal.engine_commit".to_owned(),
            found: "bad".to_owned(),
            expected: "good".to_owned(),
        },
        CheckpointVerifyError::InMemoryBaseStateCorrupted {
            expected: "a".repeat(64),
            actual: "b".repeat(64),
        },
    ];
    for err in all_variants {
        assert!(!err.code().is_empty());
        assert!(!err.hint().is_empty());
        assert!(!err.to_string().is_empty());
        let st = err.to_structured_error();
        assert_eq!(st.code, err.code());
    }

    // Exercise strict JSON primitive branches (bool, float, i64, null, trailing bytes, non-empty check).
    assert!(
        parse_strict_json_value("x", br#"{"b":true,"f":1.5,"n":-3,"z":null,"arr":[1]}"#).is_ok()
    );
    assert!(matches!(
        parse_strict_json_value("x", b""),
        Err(CheckpointVerifyError::UnexpectedEofOrMalformedEncoding { .. })
    ));
    assert!(matches!(
        parse_strict_json_value("x", b"{\"a\":1} trailing"),
        Err(CheckpointVerifyError::UnexpectedEofOrMalformedEncoding { .. })
    ));

    // 2. Build a real 4-turn checkpoint, then test inner replay/schema/skip/manifest checks.
    let (tmp, _host, save_bytes, seal_bytes) =
        build_pre_act_checkpoint_at_turn(930_020, 11, 22, COMMIT_A, 4, "err_cov");

    // Trailing bytes on save & seal hit UnexpectedEofOrMalformedEncoding.
    let mut trail_save = save_bytes.clone();
    trail_save.extend_from_slice(b" garbage");
    assert!(matches!(
        verify_checkpoint_causal(&trail_save, &seal_bytes),
        Err(CheckpointVerifyError::UnexpectedEofOrMalformedEncoding { .. })
    ));

    // Explicit `skip_serializing_if` empty/null fields hit NonCanonicalEncoding rather than UnknownFieldRejected.
    let save_str = String::from_utf8(save_bytes.clone()).unwrap_or_default();
    let seal_str = String::from_utf8(seal_bytes.clone()).unwrap_or_default();
    let skip_null_seal = seal_str
        .replacen(
            "\"format_version\":2,",
            "\"format_version\":2,\"pre_act_checkpoint_sha256\":null,",
            1,
        )
        .into_bytes();
    assert!(matches!(
        verify_checkpoint_causal(&save_bytes, &skip_null_seal),
        Err(CheckpointVerifyError::NonCanonicalEncoding { .. })
    ));
    let skip_receipts_seal = seal_str
        .replacen("\"engine\":{", "\"engine\":{\"public_receipts\":[],", 1)
        .into_bytes();
    assert!(matches!(
        verify_checkpoint_causal(&save_bytes, &skip_receipts_seal),
        Err(CheckpointVerifyError::NonCanonicalEncoding { .. })
    ));

    // Schema type error inside save & seal (`"turn": true`) hits UnexpectedEofOrMalformedEncoding.
    let bad_turn_save = save_str
        .replacen("\"turn\": 4,", "\"turn\": \"bad\",", 1)
        .into_bytes();
    assert!(matches!(
        verify_checkpoint_causal(&bad_turn_save, &seal_bytes),
        Err(CheckpointVerifyError::UnexpectedEofOrMalformedEncoding { .. })
    ));
    let bad_turn_seal = seal_str
        .replacen("\"turn\":4,", "\"turn\":\"bad\",", 1)
        .into_bytes();
    assert!(matches!(
        verify_checkpoint_causal(&save_bytes, &bad_turn_seal),
        Err(CheckpointVerifyError::UnexpectedEofOrMalformedEncoding { .. })
    ));

    // Missing / wrong save_format_version and engine_commit in seal -> IncompatibleEngineOrSchemaVersion.
    for (pat, rep) in [
        ("\"save_format_version\":2,", "\"save_format_version\":1,"),
        ("\"save_format_version\":2,", ""),
        (&format!("\"engine_commit\":\"{COMMIT_A}\","), ""),
    ] {
        let patched = seal_str.replacen(pat, rep, 1).into_bytes();
        assert!(matches!(
            verify_checkpoint_causal(&save_bytes, &patched),
            Err(CheckpointVerifyError::IncompatibleEngineOrSchemaVersion { .. })
        ));
    }

    // Tampered logged_turn fields inside canonical compact seal -> ReplayDigestMismatch.
    let bad_drand = seal_str
        .replacen(
            "\"logged_turns\":[{\"turn\":1,",
            "\"logged_turns\":[{\"turn\":1,\"drand_round\":1,",
            1,
        )
        .into_bytes();
    let bad_beacon = {
        let mut s = seal_str.clone();
        if let Some(lt) = s.find("\"logged_turns\":[")
            && let Some(rel) = s[lt..].find("\"turn_beacon\":[")
        {
            let arr_start = lt + rel + "\"turn_beacon\":[".len();
            if let Some(end_rel) = s[arr_start..].find(',') {
                let first = s[arr_start..arr_start + end_rel].trim();
                let next_val = if first == "0" { "1" } else { "0" };
                s.replace_range(arr_start..arr_start + end_rel, next_val);
            }
        }
        s.into_bytes()
    };
    let bad_lock = {
        let mut s = seal_str.clone();
        let lt = s.find("\"logged_turns\":[").unwrap_or(0);
        let obj_end = s[lt..]
            .find("\"commitments\":")
            .map(|o| lt + o)
            .unwrap_or(lt);
        if let Some(rel) = s[lt..obj_end].find("\"cancel_lock_at\":") {
            let v_start = lt + rel + "\"cancel_lock_at\":".len();
            if let Some(v_len) = s[v_start..].find(',') {
                s.replace_range(v_start..v_start + v_len, "9999");
            }
        } else {
            s = s.replacen(
                "\"logged_turns\":[{\"turn\":1,",
                "\"logged_turns\":[{\"turn\":1,\"cancel_lock_at\":9999,",
                1,
            );
        }
        s.into_bytes()
    };
    for bad_seal in [bad_drand, bad_beacon, bad_lock] {
        let res = verify_checkpoint_causal(&save_bytes, &bad_seal);
        assert!(
            matches!(
                res,
                Err(CheckpointVerifyError::ReplayDigestMismatch { .. }
                    | CheckpointVerifyError::NonCanonicalEncoding { .. })
            ),
            "unexpected result: {res:?}"
        );
    }
    let _ = fs::remove_dir_all(&tmp);

    // 3. `verify_and_construct_pre_act_bytes` outer hash and manifest tip mismatch branches.
    let save_sha = sha256_bytes_hex(&save_bytes);
    let seal_sha = sha256_bytes_hex(&seal_bytes);
    let (_, summary) = must_ok(
        verify_checkpoint_causal(&save_bytes, &seal_bytes),
        "clean verify for manifest tests",
    );
    let pair_sha = summary.pre_act_pair_sha256.clone();
    assert!(matches!(
        verify_and_construct_pre_act_bytes(
            &save_bytes,
            &seal_bytes,
            Some(&"0".repeat(64)),
            None,
            None,
            None
        ),
        Err(CheckpointVerifyError::OuterFileHashMismatch { .. })
    ));
    assert!(matches!(
        verify_and_construct_pre_act_bytes(
            &save_bytes,
            &seal_bytes,
            Some(&save_sha),
            Some(&"0".repeat(64)),
            None,
            None
        ),
        Err(CheckpointVerifyError::OuterFileHashMismatch { .. })
    ));
    assert!(matches!(
        verify_and_construct_pre_act_bytes(
            &save_bytes,
            &seal_bytes,
            Some(&save_sha),
            Some(&seal_sha),
            Some(&"0".repeat(64)),
            None
        ),
        Err(CheckpointVerifyError::OuterFileHashMismatch { .. })
    ));
    assert!(matches!(
        verify_and_construct_pre_act_bytes(
            &save_bytes,
            &seal_bytes,
            Some(&save_sha),
            Some(&seal_sha),
            Some(&pair_sha),
            Some((9, 4, &summary.expected_state_digest)),
        ),
        Err(CheckpointVerifyError::TipStateDigestMismatch { .. })
    ));
    assert!(matches!(
        verify_and_construct_pre_act_bytes(
            &save_bytes,
            &seal_bytes,
            Some(&save_sha),
            Some(&seal_sha),
            Some(&pair_sha),
            Some((summary.turn, 3, &summary.expected_state_digest)),
        ),
        Err(CheckpointVerifyError::TipStateDigestMismatch { .. })
    ));
    assert!(matches!(
        verify_and_construct_pre_act_bytes(
            &save_bytes,
            &seal_bytes,
            Some(&save_sha),
            Some(&seal_sha),
            Some(&pair_sha),
            Some((summary.turn, 4, &"0".repeat(64))),
        ),
        Err(CheckpointVerifyError::TipStateDigestMismatch { .. })
    ));
    assert!(
        verify_and_construct_pre_act_bytes(
            &save_bytes,
            &seal_bytes,
            Some(&save_sha),
            Some(&seal_sha),
            Some(&pair_sha),
            Some((summary.turn, 4, &summary.expected_state_digest)),
        )
        .is_ok()
    );

    // 4. Exercise decode_strict_checkpoint_pair invariants & Turn 1 skip_serializing_if branches.
    let (tmp1, _h1, save1, seal1) =
        build_pre_act_checkpoint_at_turn(930_021, 13, 29, COMMIT_A, 1, "t1_cov");
    let save1_str = String::from_utf8(save1.clone()).unwrap_or_default();
    let seal1_str = String::from_utf8(seal1.clone()).unwrap_or_default();
    let empty_receipts_seal1 = seal1_str
        .replacen("\"engine\":{", "\"engine\":{\"public_receipts\":[],", 1)
        .into_bytes();
    assert!(matches!(
        verify_checkpoint_causal(&save1, &empty_receipts_seal1),
        Err(CheckpointVerifyError::NonCanonicalEncoding { .. })
    ));
    let no_ver_save1 = save1_str
        .replacen("  \"format_version\": 2,\n", "", 1)
        .into_bytes();
    assert!(matches!(
        verify_checkpoint_causal(&no_ver_save1, &seal1),
        Err(CheckpointVerifyError::IncompatibleEngineOrSchemaVersion { .. })
    ));
    let bool_ver_save1 = save1_str
        .replacen("\"format_version\": 2,", "\"format_version\": true,", 1)
        .into_bytes();
    assert!(matches!(
        verify_checkpoint_causal(&bool_ver_save1, &seal1),
        Err(CheckpointVerifyError::IncompatibleEngineOrSchemaVersion { .. })
    ));
    let bad_gid_save1 = save1_str
        .replacen(
            &format!("\"game_id\": \"{}\"", _h1.game_id()),
            "\"game_id\": \"abcd\"",
            1,
        )
        .into_bytes();
    assert!(matches!(
        verify_checkpoint_causal(&bad_gid_save1, &seal1),
        Err(CheckpointVerifyError::PairBindingMismatch { .. })
    ));
    let diff_gid_save1 = save1_str
        .replacen(_h1.game_id(), &"e".repeat(32), 1)
        .into_bytes();
    assert!(matches!(
        verify_checkpoint_causal(&diff_gid_save1, &seal1),
        Err(CheckpointVerifyError::PairBindingMismatch { .. })
    ));
    let _ = fs::remove_dir_all(&tmp1);

    // Exercise seal turn mismatch, save turn mismatch, history len checks, and ws canonical checks on Turn 4.
    let mismatch_seal_turn = seal_str
        .replacen(",\"turn\":4,", ",\"turn\":3,", 1)
        .into_bytes();
    assert!(matches!(
        verify_checkpoint_causal(&save_bytes, &mismatch_seal_turn),
        Err(CheckpointVerifyError::TipStateDigestMismatch { .. })
    ));
    let mismatch_save_turn = save_str
        .replacen("\"turn\": 4,", "\"turn\": 3,", 1)
        .into_bytes();
    assert!(matches!(
        verify_checkpoint_causal(&mismatch_save_turn, &seal_bytes),
        Err(CheckpointVerifyError::TipStateDigestMismatch { .. })
    ));
    let ws_save = format!("{save_str} ").into_bytes();
    let ws_sha = compute_eval_save_v2_sha256_hex(&ws_save);
    let ws_save_seal = seal_str
        .replacen(&compute_eval_save_v2_sha256_hex(&save_bytes), &ws_sha, 1)
        .into_bytes();
    assert!(matches!(
        verify_checkpoint_causal(&ws_save, &ws_save_seal),
        Err(CheckpointVerifyError::NonCanonicalEncoding { .. })
    ));
    assert!(matches!(
        verify_checkpoint_causal(&save_bytes, format!("{seal_str} ").as_bytes()),
        Err(CheckpointVerifyError::NonCanonicalEncoding { .. })
    ));
    let bad_seed_save = save_str
        .replacen("\"layout_seed\": 11,", "\"layout_seed\": 99,", 1)
        .into_bytes();
    let bad_seed_sha = compute_eval_save_v2_sha256_hex(&bad_seed_save);
    let bad_seed_seal = seal_str
        .replacen(
            &compute_eval_save_v2_sha256_hex(&save_bytes),
            &bad_seed_sha,
            1,
        )
        .into_bytes();
    assert!(matches!(
        verify_checkpoint_causal(&bad_seed_save, &bad_seed_seal),
        Err(CheckpointVerifyError::PairBindingMismatch { .. })
    ));

    let lt_span = find_json_field_value_span(&seal_bytes, 0, "logged_turns");
    let empty_lt_seal = replace_span(&seal_bytes, lt_span, b"[]");
    assert!(matches!(
        verify_checkpoint_causal(&save_bytes, &empty_lt_seal),
        Err(CheckpointVerifyError::TipStateDigestMismatch { .. })
    ));
    let lsd_span = find_json_field_value_span(&seal_bytes, 0, "live_state_digests");
    let empty_lsd_seal = replace_span(&seal_bytes, lsd_span, b"[]");
    assert!(matches!(
        verify_checkpoint_causal(&save_bytes, &empty_lsd_seal),
        Err(CheckpointVerifyError::TipStateDigestMismatch { .. })
    ));

    // Additional strict JSON / probe divergence / default-action replay coverage cases.
    let _ = parse_strict_json_value("x", b"{\"x\":1e999}");
    assert!(matches!(
        parse_strict_json_value("x", b"{\"x\":NaN}"),
        Err(CheckpointVerifyError::UnexpectedEofOrMalformedEncoding { .. })
    ));

    fn diverge_probe_cleanly(host: &mut EvalHost) {
        let _ = host.apply_market_action(league_core::MarketAction::RecordIntentPledge {
            seat: 0,
            pledge: league_core::IntentPledge {
                pledge_id: 999,
                promisor: 0,
                counterparty: 1,
                target: league_core::IntentTarget::VoteForSeat {
                    office_turn: 4,
                    candidate: 1,
                },
                honor_weight: 50,
                memo_digest: [9; 32],
            },
        });
    }
    assert!(matches!(
        verify_checkpoint_causal_with_probe_fault_for_test(
            &save_bytes,
            &seal_bytes,
            diverge_probe_cleanly,
        ),
        Err(CheckpointVerifyError::ProbeStepDivergence { .. })
    ));

    // Empty-action turn exercises `cancel_lock_at == None` in replay and explicit `"cancel_lock_at":null` refusal.
    let tmp_empty = unique_tmp_dir("empty_turn_cov");
    let mut h_empty = must_ok(
        must_ok(
            EvalHost::from_standard_setup(TableId(930_022), 4, 17, 31),
            "empty turn setup",
        )
        .with_engine_commit(COMMIT_A),
        "empty turn commit",
    );
    let empty_subs: BTreeMap<u8, EvalActSubmission> = (0_u8..4)
        .map(|s| (s, EvalActSubmission::default()))
        .collect();
    must_ok(h_empty.apply_act_submissions(&empty_subs), "empty apply");
    let _ = must_ok(h_empty.sweep_turn(), "empty sweep");
    let (es_p, el_p) = must_ok(
        h_empty.write_pre_act_checkpoint(&tmp_empty),
        "write empty turn ckpt",
    );
    let e_save = must_ok(fs::read(&es_p), "read empty save");
    let e_seal = must_ok(fs::read(&el_p), "read empty seal");
    assert!(verify_checkpoint_causal(&e_save, &e_seal).is_ok());

    let e_seal_str = String::from_utf8(e_seal.clone()).unwrap_or_default();
    let lt_offset = e_seal_str.find("\"logged_turns\":[").unwrap_or(0);
    let null_lock_seal = {
        let mut s = e_seal_str.clone();
        let rel = must_some(s[lt_offset..].find("\"turn\":1,"), "turn:1 in logged_turns");
        let ins = lt_offset + rel + "\"turn\":1,".len();
        s.insert_str(ins, "\"cancel_lock_at\":null,");
        s.into_bytes()
    };
    assert!(matches!(
        verify_checkpoint_causal(&e_save, &null_lock_seal),
        Err(CheckpointVerifyError::NonCanonicalEncoding { .. })
    ));

    // Exercise `drawn_shocks` tampering -> ReplayDigestMismatch.
    let ds_span = find_json_field_value_span(&e_seal, lt_offset, "drawn_shocks");
    let bad_ds_bytes: Vec<u8> = if &e_seal[ds_span.0..ds_span.1] == b"[]" {
        br#"[{"hint_turn":2,"forecast_turn":4,"realize_turn":6,"target_seat":0,"kind":"SupplyFreeze","magnitude_bps":100}]"#.to_vec()
    } else {
        b"[]".to_vec()
    };
    let bad_ds_seal = replace_span(&e_seal, ds_span, &bad_ds_bytes);
    assert!(matches!(
        verify_checkpoint_causal(&e_save, &bad_ds_seal),
        Err(CheckpointVerifyError::ReplayDigestMismatch { .. })
    ));

    // Exercise `pass_b_actions: []` skip_serializing_if rejection on an act-recorded seal
    // and mid-loop `cancel_lock_at == Some(a_idx)` when both pass A and pass B actions exist.
    let tmp_act = unique_tmp_dir("act_pass_b_cov");
    let mut h_act = must_ok(
        must_ok(
            EvalHost::from_standard_setup(TableId(930_023), 4, 19, 37),
            "act setup",
        )
        .with_engine_commit(COMMIT_A),
        "act commit",
    );
    let act_subs: BTreeMap<u8, EvalActSubmission> = (0_u8..4)
        .map(|s| (s, EvalActSubmission::default()))
        .collect();
    let (pre_save_p, pre_seal_p) = must_ok(
        h_act.write_pre_act_checkpoint(&tmp_act),
        "write pre before act",
    );
    let h_fork_pre = must_ok(
        EvalHost::fork_from_checkpoint(&pre_save_p, &pre_seal_p),
        "fork pre",
    );
    must_ok(h_act.apply_act_submissions(&act_subs), "empty act apply");
    let act_seal_p = must_ok(h_act.write_act_recorded_seal(&tmp_act), "write act seal");
    let act_seal_str = must_ok(fs::read_to_string(&act_seal_p), "read act seal");
    let rec_pos = act_seal_str
        .find("\"recorded_act_submissions\":{")
        .unwrap_or(0);
    let bad_act_seal = if let Some(b_rel) = act_seal_str[rec_pos..].find("\"bundle\":") {
        let mut s = act_seal_str;
        s.insert_str(rec_pos + b_rel, "\"pass_b_actions\":[],");
        s
    } else {
        act_seal_str
    };
    must_ok(fs::write(&act_seal_p, bad_act_seal), "write bad act seal");
    assert!(matches!(
        h_fork_pre.read_recorded_act_submissions(&act_seal_p),
        Err(EngineError::InvalidConfig(_))
    ));
    let pre1_save = must_ok(fs::read(&pre_save_p), "read turn1 save");
    let pre1_seal = must_ok(fs::read(&pre_seal_p), "read turn1 seal");
    let _ = fs::remove_dir_all(&tmp_act);
    let mut act_subs = act_subs;

    let tmp_lock = unique_tmp_dir("lock_mid_cov");
    let mut h_lock = must_ok(
        must_ok(
            EvalHost::from_standard_setup(TableId(930_024), 4, 23, 41),
            "lock setup",
        )
        .with_engine_commit(COMMIT_A),
        "lock commit",
    );
    let mk_pledge = |id: u64| league_core::MarketAction::RecordIntentPledge {
        seat: 0,
        pledge: league_core::IntentPledge {
            pledge_id: id,
            promisor: 0,
            counterparty: 1,
            target: league_core::IntentTarget::VoteForSeat {
                office_turn: 4,
                candidate: 1,
            },
            honor_weight: 50,
            memo_digest: [id as u8; 32],
        },
    };
    act_subs.insert(
        0,
        EvalActSubmission {
            market_actions: vec![mk_pledge(1)],
            pass_b_actions: vec![mk_pledge(2)],
            bundle: league_core::SealedOrderBundle::default(),
        },
    );
    must_ok(h_lock.apply_act_submissions(&act_subs), "lock act apply");
    let _ = must_ok(h_lock.sweep_turn(), "lock sweep");
    let (ls_p, ll_p) = must_ok(
        h_lock.write_pre_act_checkpoint(&tmp_lock),
        "write lock ckpt",
    );
    let l_save = must_ok(fs::read(&ls_p), "read lock save");
    let l_seal = must_ok(fs::read(&ll_p), "read lock seal");
    assert!(verify_checkpoint_causal(&l_save, &l_seal).is_ok());

    // Exercise additional strict pre-act decoding and Gate 2 tip-divergence checks:
    // 1. seal_turn == 0 -> TipStateDigestMismatch.
    let turn0_seal = {
        let mut v = e_seal_str.clone();
        let hdr_t = must_some(v.find("\"turn\":2,"), "hdr turn");
        v.replace_range(hdr_t..hdr_t + "\"turn\":2,".len(), "\"turn\":0,");
        let st_off = must_some(v.find("\"state\":{"), "state obj");
        let st_t = must_some(v[st_off..].find("\"turn\":2,"), "state turn");
        v.replace_range(
            st_off + st_t..st_off + st_t + "\"turn\":2,".len(),
            "\"turn\":0,",
        );
        v.into_bytes()
    };
    assert!(matches!(
        decode_strict_checkpoint_pair(&e_save, &turn0_seal),
        Err(CheckpointVerifyError::TipStateDigestMismatch { .. })
    ));

    // 2. Act-recorded header (`pre_act_checkpoint_sha256`) on pre-act pair (both with & without `save_sha256`) -> PairBindingMismatch.
    let pre_act_hex = "c".repeat(64);
    let save_sha_span = find_json_field_value_span(&e_seal, 0, "save_sha256");
    let pre_act_with_save = {
        let mut s = e_seal_str.clone();
        let pos = must_some(s.find("\"save_sha256\":"), "save_sha256 key");
        s.insert_str(
            pos,
            &format!("\"pre_act_checkpoint_sha256\":\"{pre_act_hex}\","),
        );
        s.into_bytes()
    };
    assert!(matches!(
        decode_strict_checkpoint_pair(&e_save, &pre_act_with_save),
        Err(CheckpointVerifyError::PairBindingMismatch { .. })
    ));
    let pre_act_without_save = {
        let old_kv = format!(
            "\"save_sha256\":{}",
            must_ok(
                std::str::from_utf8(&e_seal[save_sha_span.0..save_sha_span.1]),
                "utf8"
            )
        );
        e_seal_str
            .replacen(
                &old_kv,
                &format!("\"pre_act_checkpoint_sha256\":\"{pre_act_hex}\""),
                1,
            )
            .into_bytes()
    };
    assert!(matches!(
        decode_strict_checkpoint_pair(&e_save, &pre_act_without_save),
        Err(CheckpointVerifyError::PairBindingMismatch { .. })
    ));

    // 3. Non-empty `recorded_act_submissions` fails `ensure_pre_act_state` inside `decode_strict_checkpoint_pair`.
    let lock_seal_str = String::from_utf8(l_seal).unwrap_or_default();
    let default_sub_json = must_ok(
        serde_json::to_string(&EvalActSubmission::default()),
        "serialize default EvalActSubmission",
    );
    let bad_cur_lock_s = lock_seal_str.replacen(
        "\"recorded_act_submissions\":{}",
        &format!("\"recorded_act_submissions\":{{\"0\":{default_sub_json}}}"),
        1,
    );
    assert!(matches!(
        decode_strict_checkpoint_pair(&l_save, bad_cur_lock_s.as_bytes()),
        Err(CheckpointVerifyError::PairBindingMismatch { .. })
    ));

    // 4. Turn-1 pre-act checkpoint whose save & engine.state both mutate identically passes
    //    `decode_strict_checkpoint_pair` (since `logged_turns` is empty) and fails at Gate 2 tip replay check.
    let old_p1_sha = compute_eval_save_v2_sha256_hex(&pre1_save);
    let bump_first_cred = |mut s: String, key: &str| -> String {
        let seats_p = must_some(s.find("\"seats\":"), "seats arr");
        let cred_p = seats_p + must_some(s[seats_p..].find(key), "cred key") + key.len();
        let end_p = cred_p
            + s[cred_p..]
                .find(|c: char| !c.is_ascii_digit() && c != '-')
                .unwrap_or(0);
        let old_val: i64 = must_ok(s[cred_p..end_p].parse(), "parse cred");
        s.replace_range(cred_p..end_p, &(old_val + 777).to_string());
        s
    };
    let mut_pre1_save = bump_first_cred(
        String::from_utf8(pre1_save).unwrap_or_default(),
        "\"cred\": ",
    )
    .into_bytes();
    let new_p1_sha = compute_eval_save_v2_sha256_hex(&mut_pre1_save);
    let mut_pre1_seal = {
        let pre1_s = String::from_utf8(pre1_seal).unwrap_or_default();
        let st_pos = must_some(pre1_s.find("\"engine\":{"), "engine obj");
        let seats_rel = must_some(pre1_s[st_pos..].find("\"seats\":["), "engine seats");
        let abs_seats = st_pos + seats_rel;
        let cred_rel =
            must_some(pre1_s[abs_seats..].find("\"cred\":"), "engine cred") + "\"cred\":".len();
        let abs_cred = abs_seats + cred_rel;
        let end_p = abs_cred
            + pre1_s[abs_cred..]
                .find(|c: char| !c.is_ascii_digit() && c != '-')
                .unwrap_or(0);
        let old_val: i64 = must_ok(pre1_s[abs_cred..end_p].parse(), "parse engine cred");
        let mut s = pre1_s;
        s.replace_range(abs_cred..end_p, &(old_val + 777).to_string());
        s.replacen(
            &format!("\"save_sha256\":\"{old_p1_sha}\""),
            &format!("\"save_sha256\":\"{new_p1_sha}\""),
            1,
        )
        .into_bytes()
    };
    assert!(matches!(
        verify_checkpoint_causal(&mut_pre1_save, &mut_pre1_seal),
        Err(CheckpointVerifyError::TipStateDigestMismatch { .. })
    ));

    // 5. Non-numeric `"format_version": "two"` triggers non-u32 `into_engine_error` fallback via `EvalHost::fork_from_checkpoint`.
    let non_num_save_p = tmp_lock.join("bad_ver.save.json");
    let non_num_save = String::from_utf8(e_save.clone())
        .unwrap_or_default()
        .replacen("\"format_version\": 2,", "\"format_version\": \"two\",", 1);
    must_ok(
        fs::write(&non_num_save_p, non_num_save),
        "write bad_ver save",
    );
    assert!(matches!(
        EvalHost::fork_from_checkpoint(&non_num_save_p, &el_p),
        Err(EngineError::InvalidConfig(_))
    ));
    let _ = fs::remove_dir_all(&tmp_lock);

    let vrf_span = find_json_field_value_span(&e_seal, 0, "eval_vrf_proof");
    let bad_vrf_seal = replace_span(&e_seal, vrf_span, b"null");
    assert!(matches!(
        verify_checkpoint_causal(&e_save, &bad_vrf_seal),
        Err(CheckpointVerifyError::ReplayDigestMismatch { .. })
    ));

    let _ = fs::remove_dir_all(&tmp_empty);
}
