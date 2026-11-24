//! Gate 3 verification, RAM-clone parity, empty-build-ID refusal, and post-batch canary (`M10`)
//! integration tests for `league-scorer`.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use league_core::eval::host::checkpoint_verify::{CheckpointVerifyError, verify_checkpoint_causal};
use league_core::{
    ClubBot, EVAL_SAVE_FORMAT_VERSION, EVAL_SEAL_FORMAT_VERSION, EvalActSubmission, EvalHost, Seat,
    TableId, ToStructuredError as _,
};
use league_scorer::cli::{run_cli_with_build_id, run_cli_with_build_id_and_state_hook};
use league_scorer::{
    CRATE_GIT_SHA, EXIT_OK, EXIT_RUNTIME_ERROR, EXIT_USER_ERROR, ScorerConfig, ScorerError,
    ScorerFreezeBlindView, VERIFIED_CHECKPOINTS_SCHEMA_VERSION, VerifiedCheckpointEntry,
    VerifiedCheckpointsManifest, build_arm_independent_request,
    build_arm_independent_request_verified, compute_blind_inputs_manifest_sha256,
    compute_pre_act_pair_sha256_hex, compute_scorer_tree_sha256, execute_blind_request,
    execute_blind_request_verified, execute_blind_request_verified_with_hook,
    execute_blind_request_verified_with_mid_batch_hook, execute_blind_request_with_mid_batch_hook,
    run_single_rollout_from_host, score_outcomes, sha256_hex, to_sorted_compact_json,
    to_sorted_pretty_json, verify_checkpoint_pair_unchanged_after_batch,
};

type TestResult = Result<(), Box<dyn Error>>;
const COMMIT_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const BUILD_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct TmpDir {
    path: PathBuf,
}

impl TmpDir {
    fn new(label: &str) -> Result<Self, Box<dyn Error>> {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("ml_gate3_{label}_{}_{id}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path)?;
        Ok(Self { path })
    }
}

impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

struct Gate3Fixture {
    ckpts_dir: PathBuf,
    blind_dir: PathBuf,
    cfg_path: PathBuf,
    freeze_path: PathBuf,
    vc_path: PathBuf,
    cas_save: PathBuf,
    cfg: ScorerConfig,
    req: league_scorer::BlindScoringRequest,
    manifest: VerifiedCheckpointsManifest,
}

fn setup_gate3_fixture(
    tmp: &Path,
    engine_commit: &str,
    build_id: &str,
) -> Result<Gate3Fixture, Box<dyn Error>> {
    let mut host = EvalHost::from_standard_setup(TableId(940_001), 4, 7, 15)?
        .with_engine_commit(engine_commit)?;
    for _ in 0..2 {
        let mut subs = BTreeMap::new();
        for s in 0..host.seat_count() {
            let obs = host.observe_seat(Seat(s))?;
            let mandate = *obs.own_mandate().ok_or("missing own_mandate")?;
            let dec = ClubBot::new(host.bot_tie_seed().wrapping_add(u64::from(s)))
                .decide_full_turn(&obs, &mandate);
            subs.insert(
                s,
                EvalActSubmission {
                    market_actions: dec.pass_a_actions,
                    pass_b_actions: dec.pass_b_actions,
                    bundle: dec.bundle,
                },
            );
        }
        host.apply_act_submissions(&subs)?;
        host.sweep_turn()?;
    }

    let raw_dir = tmp.join("raw_ckpt");
    let (raw_save, raw_seal) = host.write_pre_act_checkpoint(&raw_dir)?;
    let save_bytes = fs::read(&raw_save)?;
    let seal_bytes = fs::read(&raw_seal)?;
    let save_sha = sha256_hex(&save_bytes);
    let seal_sha = sha256_hex(&seal_bytes);

    let ckpts_dir = tmp.join("checkpoints");
    fs::create_dir_all(&ckpts_dir)?;
    let cas_save = ckpts_dir.join(format!("{save_sha}.save.json"));
    let cas_seal = ckpts_dir.join(format!("{seal_sha}.seal"));
    fs::write(&cas_save, &save_bytes)?;
    fs::write(&cas_seal, &seal_bytes)?;

    let (_, summary) = verify_checkpoint_causal(&save_bytes, &seal_bytes)?;
    let manifest = VerifiedCheckpointsManifest {
        schema_version: VERIFIED_CHECKPOINTS_SCHEMA_VERSION,
        engine_build_id: build_id.to_owned(),
        save_format_version: EVAL_SAVE_FORMAT_VERSION,
        seal_format_version: EVAL_SEAL_FORMAT_VERSION,
        entries: vec![VerifiedCheckpointEntry {
            pre_act_pair_sha256: summary.pre_act_pair_sha256,
            pre_act_save_sha256: summary.pre_act_save_sha256,
            pre_act_seal_sha256: summary.pre_act_seal_sha256,
            engine_commit: summary.engine_commit,
            turn: summary.turn,
            seat_count: summary.seat_count,
            expected_state_digest: summary.expected_state_digest,
            turn_chain_digest: summary.turn_chain_digest,
            post_probe_state_digest: summary.post_probe_state_digest,
        }],
    };
    let vc_path = tmp.join("verified_checkpoints.json");
    let vc_text = to_sorted_pretty_json(&manifest)?;
    fs::write(&vc_path, vc_text.as_bytes())?;
    let vc_sha = sha256_hex(vc_text.as_bytes());

    let cfg = ScorerConfig {
        k_synth_max: 2,
        scenarios_m: 2,
        ..ScorerConfig::default()
    };
    let cfg_path = tmp.join("cfg.json");
    fs::write(&cfg_path, to_sorted_pretty_json(&cfg)?)?;

    let req = build_arm_independent_request_verified(
        &cas_save,
        &cas_seal,
        &save_sha,
        &seal_sha,
        0,
        "game940001-day-03-seat-0",
        &cfg,
        123,
        &manifest,
        build_id,
    )?;
    let blind_dir = tmp.join("blind_inputs");
    fs::create_dir_all(&blind_dir)?;
    fs::write(
        blind_dir.join(format!("{}.json", req.state_id)),
        to_sorted_compact_json(&req)?,
    )?;

    let tree_sha = compute_scorer_tree_sha256(Path::new(env!("CARGO_MANIFEST_DIR")))?;
    let manifest_sha = compute_blind_inputs_manifest_sha256(&blind_dir)?;
    let freeze_path = tmp.join("scorer_freeze.json");
    fs::write(
        &freeze_path,
        to_sorted_pretty_json(&ScorerFreezeBlindView {
            blind_inputs_manifest_sha256: manifest_sha,
            scorer_config_sha256: cfg.sha256_hex()?,
            scorer_tree_sha256: tree_sha,
            verified_checkpoints_sha256: vc_sha,
        })?,
    )?;

    Ok(Gate3Fixture {
        ckpts_dir,
        blind_dir,
        cfg_path,
        freeze_path,
        vc_path,
        cas_save,
        cfg,
        req,
        manifest,
    })
}

#[test]
fn gate3_verified_score_parity_and_build_id_and_freeze_enforcement() -> TestResult {
    let tmp = TmpDir::new("gate3_parity")?;
    let build_id = if CRATE_GIT_SHA.len() == 40 {
        CRATE_GIT_SHA
    } else {
        BUILD_B
    };
    let Gate3Fixture {
        ckpts_dir,
        blind_dir,
        cfg_path,
        freeze_path,
        vc_path,
        cas_save,
        cfg,
        req,
        manifest,
    } = setup_gate3_fixture(&tmp.path, COMMIT_A, build_id)?;
    let cas_seal = ckpts_dir.join(format!("{}.seal", req.pre_act_seal_sha256));

    // 1. Direct RAM-clone verified score output is bit-for-bit identical to unverified call
    //    even though checkpoint engine_commit (COMMIT_A) != scorer build_id (BUILD_B/HEAD).
    let unverified_out = execute_blind_request(&req, &cfg, &cas_save, &cas_seal)?;
    let verified_out =
        execute_blind_request_verified(&req, &cfg, &cas_save, &cas_seal, &manifest, build_id)?;
    assert_eq!(
        to_sorted_pretty_json(&verified_out)?,
        to_sorted_pretty_json(&unverified_out)?
    );

    // 2. CLI `score --freeze --verified-checkpoints` and `candidates --verified-checkpoints` succeed.
    let score_out = tmp.path.join("score_out");
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    assert_eq!(
        run_cli_with_build_id(
            [
                "score",
                "--blind-inputs",
                blind_dir.to_str().ok_or("utf8")?,
                "--checkpoints",
                ckpts_dir.to_str().ok_or("utf8")?,
                "--config",
                cfg_path.to_str().ok_or("utf8")?,
                "--freeze",
                freeze_path.to_str().ok_or("utf8")?,
                "--verified-checkpoints",
                vc_path.to_str().ok_or("utf8")?,
                "--out",
                score_out.to_str().ok_or("utf8")?,
            ],
            build_id,
            &mut stdout,
            &mut stderr,
        ),
        EXIT_OK,
        "stderr={}",
        String::from_utf8_lossy(&stderr)
    );
    let cand_out = tmp.path.join("cand_out");
    let req_baseline = build_arm_independent_request(
        &cas_save,
        &cas_seal,
        &req.pre_act_save_sha256,
        &req.pre_act_seal_sha256,
        0,
        "game940001-day-03-seat-0",
        &cfg,
        123,
    )?;
    assert_eq!(req, req_baseline);
    assert_eq!(
        run_cli_with_build_id(
            [
                "candidates",
                "--checkpoints",
                ckpts_dir.to_str().ok_or("utf8")?,
                "--save-sha256",
                &req.pre_act_save_sha256,
                "--seal-sha256",
                &req.pre_act_seal_sha256,
                "--focal-seat",
                "0",
                "--state-id",
                "game940001-day-03-seat-0",
                "--config",
                cfg_path.to_str().ok_or("utf8")?,
                "--scorer-seed",
                "123",
                "--verified-checkpoints",
                vc_path.to_str().ok_or("utf8")?,
                "--out",
                cand_out.to_str().ok_or("utf8")?,
            ],
            build_id,
            &mut Vec::new(),
            &mut stderr,
        ),
        EXIT_OK
    );

    // 3. Empty and non-40-hex `scorer_build_id` refused fail-closed at runtime (M09).
    for bad_build_id in ["", "not_a_sha", &"A".repeat(40)] {
        let mut err_buf = Vec::new();
        assert_eq!(
            run_cli_with_build_id(
                [
                    "score",
                    "--blind-inputs",
                    blind_dir.to_str().ok_or("utf8")?,
                    "--checkpoints",
                    ckpts_dir.to_str().ok_or("utf8")?,
                    "--config",
                    cfg_path.to_str().ok_or("utf8")?,
                    "--freeze",
                    freeze_path.to_str().ok_or("utf8")?,
                    "--verified-checkpoints",
                    vc_path.to_str().ok_or("utf8")?,
                    "--out",
                    tmp.path.join("out_bad_build").to_str().ok_or("utf8")?,
                ],
                bad_build_id,
                &mut Vec::new(),
                &mut err_buf,
            ),
            EXIT_USER_ERROR
        );
        assert!(
            String::from_utf8_lossy(&err_buf).contains("INCOMPATIBLE_ENGINE_OR_SCHEMA_VERSION"),
            "expected build id refusal for `{bad_build_id}`, got {}",
            String::from_utf8_lossy(&err_buf)
        );
    }

    // 4. Skewed manifest.engine_build_id != scorer_build_id refused fail-closed (M09).
    let other_sha = "cccccccccccccccccccccccccccccccccccccccc";
    let mut err_skew = Vec::new();
    assert_eq!(
        run_cli_with_build_id(
            [
                "score",
                "--blind-inputs",
                blind_dir.to_str().ok_or("utf8")?,
                "--checkpoints",
                ckpts_dir.to_str().ok_or("utf8")?,
                "--config",
                cfg_path.to_str().ok_or("utf8")?,
                "--freeze",
                freeze_path.to_str().ok_or("utf8")?,
                "--verified-checkpoints",
                vc_path.to_str().ok_or("utf8")?,
                "--out",
                tmp.path.join("out_skew").to_str().ok_or("utf8")?,
            ],
            other_sha,
            &mut Vec::new(),
            &mut err_skew,
        ),
        EXIT_USER_ERROR
    );
    assert!(String::from_utf8_lossy(&err_skew).contains("INCOMPATIBLE_ENGINE_OR_SCHEMA_VERSION"));

    // 5. Missing `--verified-checkpoints` with `--freeze` fails with MISSING_VERIFIED_CHECKPOINTS.
    let mut err_missing_vc = Vec::new();
    assert_eq!(
        run_cli_with_build_id(
            [
                "score",
                "--blind-inputs",
                blind_dir.to_str().ok_or("utf8")?,
                "--checkpoints",
                ckpts_dir.to_str().ok_or("utf8")?,
                "--config",
                cfg_path.to_str().ok_or("utf8")?,
                "--freeze",
                freeze_path.to_str().ok_or("utf8")?,
                "--out",
                tmp.path.join("out_missing_vc").to_str().ok_or("utf8")?,
            ],
            build_id,
            &mut Vec::new(),
            &mut err_missing_vc,
        ),
        EXIT_USER_ERROR
    );
    assert!(String::from_utf8_lossy(&err_missing_vc).contains("MISSING_VERIFIED_CHECKPOINTS"));

    // 6. Modified `verified_checkpoints.json` disagrees with `freeze.verified_checkpoints_sha256`.
    let alt_vc = tmp.path.join("alt_verified_checkpoints.json");
    let mut alt_manifest = manifest.clone();
    alt_manifest.entries[0].turn_chain_digest = "0".repeat(64);
    fs::write(&alt_vc, to_sorted_pretty_json(&alt_manifest)?)?;
    let mut err_mismatch_vc = Vec::new();
    assert_eq!(
        run_cli_with_build_id(
            [
                "score",
                "--blind-inputs",
                blind_dir.to_str().ok_or("utf8")?,
                "--checkpoints",
                ckpts_dir.to_str().ok_or("utf8")?,
                "--config",
                cfg_path.to_str().ok_or("utf8")?,
                "--freeze",
                freeze_path.to_str().ok_or("utf8")?,
                "--verified-checkpoints",
                alt_vc.to_str().ok_or("utf8")?,
                "--out",
                tmp.path.join("out_mismatch_vc").to_str().ok_or("utf8")?,
            ],
            build_id,
            &mut Vec::new(),
            &mut err_mismatch_vc,
        ),
        EXIT_USER_ERROR
    );
    assert!(
        String::from_utf8_lossy(&err_mismatch_vc).contains("FREEZE_VERIFIED_CHECKPOINTS_MISMATCH")
    );

    // 7. Outer disk bitflip in `--checkpoints` fails Gate 3 with OUTER_FILE_HASH_MISMATCH (M01).
    let mut flipped_save = fs::read(&cas_save)?;
    flipped_save[0] ^= 0x01;
    fs::write(&cas_save, &flipped_save)?;
    let err_m01 =
        execute_blind_request_verified(&req, &cfg, &cas_save, &cas_seal, &manifest, build_id)
            .err()
            .ok_or("expected M01 OUTER_FILE_HASH_MISMATCH")?;
    assert_eq!(err_m01.code(), "OUTER_FILE_HASH_MISMATCH");

    let _ = compute_pre_act_pair_sha256_hex(&flipped_save, &[]);
    Ok(())
}

fn mutate_base_host_in_ram(host: &mut EvalHost) {
    let _ = host.begin_cancel_lock();
}

#[test]
fn m10_shared_ref_bleed_caught_by_gate3_post_batch_canary() -> TestResult {
    let tmp = TmpDir::new("m10_canary")?;
    let Gate3Fixture {
        ckpts_dir,
        cas_save,
        cfg,
        req,
        manifest,
        ..
    } = setup_gate3_fixture(&tmp.path, COMMIT_A, BUILD_B)?;
    let cas_seal = ckpts_dir.join(format!("{}.seal", req.pre_act_seal_sha256));

    let err = execute_blind_request_verified_with_hook(
        &req,
        &cfg,
        &cas_save,
        &cas_seal,
        &manifest,
        BUILD_B,
        Some(mutate_base_host_in_ram),
    )
    .err()
    .ok_or("post-batch canary must detect in-memory base_host mutation")?;
    assert_eq!(err.code(), "IN_MEMORY_BASE_STATE_CORRUPTED");
    assert_eq!(err.exit_code(), EXIT_RUNTIME_ERROR);
    assert!(matches!(
        err,
        ScorerError::CheckpointVerify(CheckpointVerifyError::InMemoryBaseStateCorrupted { .. })
    ));
    Ok(())
}

#[test]
fn gate3_manifest_validation_verified_candidate_builder_and_rollout_errors() -> TestResult {
    use league_scorer::{
        build_arm_independent_request_verified, parse_verified_checkpoints_manifest_strict,
    };

    let tmp = TmpDir::new("gate3_cov")?;
    let fix = setup_gate3_fixture(&tmp.path, COMMIT_A, BUILD_B)?;
    let cas_seal = fix
        .ckpts_dir
        .join(format!("{}.seal", fix.req.pre_act_seal_sha256));

    // 1. `VerifiedCheckpointsManifest::validate_for_scorer` and strict parser error branches.
    assert!(parse_verified_checkpoints_manifest_strict(b"{").is_err());
    assert!(parse_verified_checkpoints_manifest_strict(br#"{"schema_version":1}"#).is_err());
    assert!(fix.manifest.validate_for_scorer("bad_sha").is_err());

    let mut m_bad_schema = fix.manifest.clone();
    m_bad_schema.schema_version = VERIFIED_CHECKPOINTS_SCHEMA_VERSION + 1;
    assert_eq!(
        m_bad_schema
            .validate_for_scorer(BUILD_B)
            .err()
            .ok_or("bad schema_version")?
            .code(),
        "INCOMPATIBLE_ENGINE_OR_SCHEMA_VERSION"
    );

    let mut m_bad_save_ver = fix.manifest.clone();
    m_bad_save_ver.save_format_version = 1;
    assert!(m_bad_save_ver.validate_for_scorer(BUILD_B).is_err());

    let mut m_bad_seal_ver = fix.manifest.clone();
    m_bad_seal_ver.seal_format_version = 1;
    assert!(m_bad_seal_ver.validate_for_scorer(BUILD_B).is_err());

    let mut m_empty = fix.manifest.clone();
    m_empty.entries.clear();
    assert!(m_empty.validate_for_scorer(BUILD_B).is_err());

    for idx in 0..6 {
        let mut m = fix.manifest.clone();
        match idx {
            0 => m.entries[0].pre_act_pair_sha256 = "bad".to_owned(),
            1 => m.entries[0].pre_act_save_sha256 = "bad".to_owned(),
            2 => m.entries[0].pre_act_seal_sha256 = "bad".to_owned(),
            3 => m.entries[0].expected_state_digest = "bad".to_owned(),
            4 => m.entries[0].turn_chain_digest = "bad".to_owned(),
            _ => m.entries[0].post_probe_state_digest = "bad".to_owned(),
        }
        assert!(m.validate_for_scorer(BUILD_B).is_err());
    }

    let mut m_bad_commit = fix.manifest.clone();
    m_bad_commit.entries[0].engine_commit = "unknown".to_owned();
    assert_eq!(
        m_bad_commit
            .validate_for_scorer(BUILD_B)
            .err()
            .ok_or("bad entry engine_commit")?
            .code(),
        "INCOMPATIBLE_ENGINE_OR_SCHEMA_VERSION"
    );

    let mut m_bad_turn = fix.manifest.clone();
    m_bad_turn.entries[0].turn = 0;
    assert!(m_bad_turn.validate_for_scorer(BUILD_B).is_err());

    let mut m_bad_seats = fix.manifest.clone();
    m_bad_seats.entries[0].seat_count = 3;
    assert!(m_bad_seats.validate_for_scorer(BUILD_B).is_err());

    let mut m_unsorted = fix.manifest.clone();
    let e0 = m_unsorted.entries[0].clone();
    m_unsorted.entries.push(e0);
    assert!(m_unsorted.validate_for_scorer(BUILD_B).is_err());

    // `ScorerFreezeBlindView::validate` empty & malformed `verified_checkpoints_sha256`.
    let mut freeze_view: ScorerFreezeBlindView =
        serde_json::from_slice(&fs::read(&fix.freeze_path)?)?;
    freeze_view.verified_checkpoints_sha256.clear();
    assert_eq!(
        freeze_view
            .validate()
            .err()
            .ok_or("empty verified_checkpoints_sha256")?
            .code(),
        "MISSING_VERIFIED_CHECKPOINTS"
    );
    freeze_view.verified_checkpoints_sha256 = "not_hex".to_owned();
    assert!(freeze_view.validate().is_err());

    // 2. `build_arm_independent_request_verified` happy + validation + manifest mismatch paths.
    let built = build_arm_independent_request_verified(
        &fix.cas_save,
        &cas_seal,
        &fix.req.pre_act_save_sha256,
        &fix.req.pre_act_seal_sha256,
        0,
        "gate3_state_01",
        &fix.cfg,
        7,
        &fix.manifest,
        BUILD_B,
    )?;
    assert_eq!(built.pre_act_pair_sha256, fix.req.pre_act_pair_sha256);

    assert!(
        build_arm_independent_request_verified(
            &fix.cas_save,
            &cas_seal,
            "bad",
            &fix.req.pre_act_seal_sha256,
            0,
            "gate3_state_01",
            &fix.cfg,
            7,
            &fix.manifest,
            BUILD_B,
        )
        .is_err()
    );
    assert!(
        build_arm_independent_request_verified(
            &fix.cas_save,
            &cas_seal,
            &fix.req.pre_act_save_sha256,
            "bad",
            0,
            "gate3_state_01",
            &fix.cfg,
            7,
            &fix.manifest,
            BUILD_B,
        )
        .is_err()
    );
    assert!(
        build_arm_independent_request_verified(
            &fix.cas_save,
            &cas_seal,
            &fix.req.pre_act_save_sha256,
            &fix.req.pre_act_seal_sha256,
            9,
            "gate3_state_01",
            &fix.cfg,
            7,
            &fix.manifest,
            BUILD_B,
        )
        .is_err()
    );
    assert!(matches!(
        build_arm_independent_request_verified(
            &tmp.path.join("no_such.save.json"),
            &cas_seal,
            &fix.req.pre_act_save_sha256,
            &fix.req.pre_act_seal_sha256,
            0,
            "gate3_state_01",
            &fix.cfg,
            7,
            &fix.manifest,
            BUILD_B,
        ),
        Err(ScorerError::CheckpointNotFound(_))
    ));
    assert!(matches!(
        build_arm_independent_request_verified(
            &fix.cas_save,
            &tmp.path.join("no_such.seal"),
            &fix.req.pre_act_save_sha256,
            &fix.req.pre_act_seal_sha256,
            0,
            "gate3_state_01",
            &fix.cfg,
            7,
            &fix.manifest,
            BUILD_B,
        ),
        Err(ScorerError::CheckpointNotFound(_))
    ));

    let mut m_diff_pair = fix.manifest.clone();
    m_diff_pair.entries[0].pre_act_pair_sha256 = "c".repeat(64);
    assert_eq!(
        build_arm_independent_request_verified(
            &fix.cas_save,
            &cas_seal,
            &fix.req.pre_act_save_sha256,
            &fix.req.pre_act_seal_sha256,
            0,
            "gate3_state_01",
            &fix.cfg,
            7,
            &m_diff_pair,
            BUILD_B,
        )
        .err()
        .ok_or("missing pair in manifest")?
        .code(),
        "OUTER_FILE_HASH_MISMATCH"
    );

    let mut m_diff_save = fix.manifest.clone();
    m_diff_save.entries[0].pre_act_save_sha256 = "d".repeat(64);
    assert!(
        build_arm_independent_request_verified(
            &fix.cas_save,
            &cas_seal,
            &fix.req.pre_act_save_sha256,
            &fix.req.pre_act_seal_sha256,
            0,
            "gate3_state_01",
            &fix.cfg,
            7,
            &m_diff_save,
            BUILD_B,
        )
        .is_err()
    );
    let mut m_diff_seal = fix.manifest.clone();
    m_diff_seal.entries[0].pre_act_seal_sha256 = "e".repeat(64);
    assert!(
        build_arm_independent_request_verified(
            &fix.cas_save,
            &cas_seal,
            &fix.req.pre_act_save_sha256,
            &fix.req.pre_act_seal_sha256,
            0,
            "gate3_state_01",
            &fix.cfg,
            7,
            &m_diff_seal,
            BUILD_B,
        )
        .is_err()
    );

    // 3. `execute_blind_request_verified` against mismatched manifest entry.
    for m in [&m_diff_pair, &m_diff_save, &m_diff_seal] {
        assert_eq!(
            execute_blind_request_verified(
                &fix.req,
                &fix.cfg,
                &fix.cas_save,
                &cas_seal,
                m,
                BUILD_B
            )
            .err()
            .ok_or("expected OUTER_FILE_HASH_MISMATCH")?
            .code(),
            "OUTER_FILE_HASH_MISMATCH"
        );
    }

    // 4. Scorer CLI `--verified-checkpoints` missing / nonexistent / unfrozen paths.
    let mut cli_out = Vec::new();
    let mut cli_err = Vec::new();
    let code_missing_vc = run_cli_with_build_id(
        [
            "score",
            "--blind-inputs",
            fix.blind_dir.to_str().ok_or("utf8")?,
            "--checkpoints",
            fix.ckpts_dir.to_str().ok_or("utf8")?,
            "--config",
            fix.cfg_path.to_str().ok_or("utf8")?,
            "--out",
            tmp.path.join("out_missing_vc").to_str().ok_or("utf8")?,
            "--verified-checkpoints",
            tmp.path
                .join("nonexistent_vc.json")
                .to_str()
                .ok_or("utf8")?,
        ],
        BUILD_B,
        &mut cli_out,
        &mut cli_err,
    );
    assert_eq!(code_missing_vc, EXIT_USER_ERROR);
    assert!(
        String::from_utf8_lossy(&cli_err).contains("MISSING_VERIFIED_CHECKPOINTS"),
        "{}",
        String::from_utf8_lossy(&cli_err)
    );

    let mut cli_ok_out = Vec::new();
    let mut cli_ok_err = Vec::new();
    let code_unfrozen_vc = run_cli_with_build_id(
        [
            "score",
            "--blind-inputs",
            fix.blind_dir.to_str().ok_or("utf8")?,
            "--checkpoints",
            fix.ckpts_dir.to_str().ok_or("utf8")?,
            "--config",
            fix.cfg_path.to_str().ok_or("utf8")?,
            "--out",
            tmp.path.join("out_unfrozen_vc").to_str().ok_or("utf8")?,
            "--verified-checkpoints",
            fix.vc_path.to_str().ok_or("utf8")?,
        ],
        BUILD_B,
        &mut cli_ok_out,
        &mut cli_ok_err,
    );
    assert_eq!(
        code_unfrozen_vc,
        EXIT_OK,
        "{}",
        String::from_utf8_lossy(&cli_ok_err)
    );

    Ok(())
}

#[test]
fn post_batch_unchanged_checkpoint_matches_baseline_byte_for_byte() -> TestResult {
    use league_scorer::{
        apply_talk_commitments, canonicalize_talk_commitments, decide_peer_turn, draw_scenarios,
        posture_for_scenario,
    };

    let tmp = TmpDir::new("post_batch_unchanged")?;
    let fix = setup_gate3_fixture(&tmp.path, COMMIT_A, BUILD_B)?;
    let cas_seal = fix
        .ckpts_dir
        .join(format!("{}.seal", fix.req.pre_act_seal_sha256));

    // Reconstruct expected baseline score output directly from the verified host.
    let save_bytes = fs::read(&fix.cas_save)?;
    let seal_bytes = fs::read(&cas_seal)?;
    let base_host = EvalHost::fork_from_verified_checkpoint_bytes(&save_bytes, &seal_bytes)?;
    let day = base_host.turn();
    let focal_obs = base_host.observe_seat(Seat(fix.req.focal_seat))?;
    let focal_mandate = *focal_obs.own_mandate().ok_or("missing focal own_mandate")?;
    let scenarios = draw_scenarios(
        &focal_obs.spec,
        focal_mandate,
        fix.req.focal_seat,
        fix.req.scorer_seed,
        &fix.req.pre_act_pair_sha256,
        day,
        fix.cfg.scenarios_m,
    )?;
    let canonical_commitments = canonicalize_talk_commitments(&fix.req.talk_commitments)?;
    let mut peer_day_subs_by_m = Vec::with_capacity(scenarios.len());
    for (m_idx, scenario) in scenarios.iter().enumerate() {
        let posture = posture_for_scenario(&fix.cfg, m_idx);
        let mut peer_subs = BTreeMap::new();
        for s in 0..base_host.seat_count() {
            if s != fix.req.focal_seat {
                let obs_s = base_host.observe_seat(Seat(s))?;
                let mandate = *scenario
                    .peer_mandates
                    .get(&s)
                    .ok_or("missing peer mandate")?;
                let seed = scenario.bot_base_seed.wrapping_add(u64::from(s));
                let base_sub = decide_peer_turn(posture, seed, &obs_s, &mandate, &fix.cfg)?;
                let final_sub = apply_talk_commitments(
                    &obs_s,
                    base_sub,
                    &canonical_commitments,
                    fix.req.scorer_seed,
                    &fix.req.pre_act_pair_sha256,
                    m_idx,
                    fix.cfg.p_honour,
                )?;
                peer_subs.insert(s, final_sub);
            }
        }
        peer_day_subs_by_m.push(peer_subs);
    }
    let mut outcomes = Vec::with_capacity(fix.req.candidates.len());
    for candidate in &fix.req.candidates {
        let mut row = Vec::with_capacity(scenarios.len());
        for (m_idx, scenario) in scenarios.iter().enumerate() {
            let posture = posture_for_scenario(&fix.cfg, m_idx);
            let (outcome, _) = run_single_rollout_from_host(
                &base_host,
                fix.req.focal_seat,
                &candidate.submission,
                &peer_day_subs_by_m[m_idx],
                focal_mandate,
                scenario,
                posture,
                &fix.cfg,
            )?;
            row.push(outcome);
        }
        outcomes.push(row);
    }
    let baseline = score_outcomes(&fix.req, &fix.cfg, day, &outcomes)?;
    let baseline_json = to_sorted_pretty_json(&baseline)?;

    let direct = execute_blind_request(&fix.req, &fix.cfg, &fix.cas_save, &cas_seal)?;
    let verified = execute_blind_request_verified(
        &fix.req,
        &fix.cfg,
        &fix.cas_save,
        &cas_seal,
        &fix.manifest,
        BUILD_B,
    )?;
    assert_eq!(to_sorted_pretty_json(&direct)?, baseline_json);
    assert_eq!(to_sorted_pretty_json(&verified)?, baseline_json);
    Ok(())
}

fn assert_changed_during_batch_error(err: &ScorerError, tmp_path: &Path) {
    assert_eq!(err.code(), "CHECKPOINT_CHANGED_DURING_BATCH");
    assert_eq!(err.exit_code(), EXIT_RUNTIME_ERROR);
    assert!(!err.hint().is_empty());
    let msg = err.to_string();
    assert!(
        msg.starts_with("CHECKPOINT_CHANGED_DURING_BATCH: "),
        "msg={msg}"
    );
    let tmp_str = tmp_path.to_string_lossy();
    assert!(
        !msg.contains(tmp_str.as_ref()),
        "temp dir leaked in error: {msg}"
    );
    let structured = err.to_structured_error();
    assert_eq!(structured.code, "CHECKPOINT_CHANGED_DURING_BATCH");
    assert_eq!(structured.hint, err.hint());
    assert!(matches!(err, ScorerError::CheckpointChangedDuringBatch(_)));
}

#[test]
fn post_batch_refuses_mid_batch_save_or_seal_mutation_and_deletion() -> TestResult {
    let tmp = TmpDir::new("mid_batch_change")?;
    let fix = setup_gate3_fixture(&tmp.path, COMMIT_A, BUILD_B)?;
    let cas_seal = fix
        .ckpts_dir
        .join(format!("{}.seal", fix.req.pre_act_seal_sha256));
    let orig_save = fs::read(&fix.cas_save)?;
    let orig_seal = fs::read(&cas_seal)?;

    // 1. Mutating `.save.json` mid-batch refuses unverified execution.
    let mut fired = false;
    let mut hook_save = |c_idx: usize, m_idx: usize| {
        if c_idx == 0 && m_idx == 0 && !fired {
            fired = true;
            let mut tampered = orig_save.clone();
            tampered[0] ^= 0x01;
            if let Err(err) = fs::write(&fix.cas_save, &tampered) {
                panic!("tamper save failed: {err}");
            }
        }
    };
    let err_save = execute_blind_request_with_mid_batch_hook(
        &fix.req,
        &fix.cfg,
        &fix.cas_save,
        &cas_seal,
        Some(&mut hook_save),
    )
    .err()
    .ok_or("expected mid-batch save change refusal")?;
    assert_changed_during_batch_error(&err_save, &tmp.path);
    assert!(err_save.to_string().contains("save changed during batch"));
    fs::write(&fix.cas_save, &orig_save)?;

    // 2. Mutating `.seal` mid-batch refuses verified execution.
    let mut fired_seal = false;
    let mut hook_seal = |c_idx: usize, m_idx: usize| {
        if c_idx == 0 && m_idx == 0 && !fired_seal {
            fired_seal = true;
            let mut tampered = orig_seal.clone();
            tampered[0] ^= 0x01;
            if let Err(err) = fs::write(&cas_seal, &tampered) {
                panic!("tamper seal failed: {err}");
            }
        }
    };
    let err_seal = execute_blind_request_verified_with_mid_batch_hook(
        &fix.req,
        &fix.cfg,
        &fix.cas_save,
        &cas_seal,
        &fix.manifest,
        BUILD_B,
        Some(&mut hook_seal),
        None,
    )
    .err()
    .ok_or("expected mid-batch seal change refusal")?;
    assert_changed_during_batch_error(&err_seal, &tmp.path);
    assert!(err_seal.to_string().contains("seal changed during batch"));
    fs::write(&cas_seal, &orig_seal)?;

    // 3. Removing `.save.json` mid-batch refuses execution without leaking directories.
    let mut fired_rm_save = false;
    let mut hook_rm_save = |c_idx: usize, m_idx: usize| {
        if c_idx == 0 && m_idx == 0 && !fired_rm_save {
            fired_rm_save = true;
            if let Err(err) = fs::remove_file(&fix.cas_save) {
                panic!("remove save failed: {err}");
            }
        }
    };
    let err_rm_save = execute_blind_request_with_mid_batch_hook(
        &fix.req,
        &fix.cfg,
        &fix.cas_save,
        &cas_seal,
        Some(&mut hook_rm_save),
    )
    .err()
    .ok_or("expected mid-batch missing save refusal")?;
    assert_changed_during_batch_error(&err_rm_save, &tmp.path);
    fs::write(&fix.cas_save, &orig_save)?;

    // 4. Removing `.seal` mid-batch refuses verified execution.
    let mut fired_rm_seal = false;
    let mut hook_rm_seal = |c_idx: usize, m_idx: usize| {
        if c_idx == 0 && m_idx == 0 && !fired_rm_seal {
            fired_rm_seal = true;
            if let Err(err) = fs::remove_file(&cas_seal) {
                panic!("remove seal failed: {err}");
            }
        }
    };
    let err_rm_seal = execute_blind_request_verified_with_mid_batch_hook(
        &fix.req,
        &fix.cfg,
        &fix.cas_save,
        &cas_seal,
        &fix.manifest,
        BUILD_B,
        Some(&mut hook_rm_seal),
        None,
    )
    .err()
    .ok_or("expected mid-batch missing seal refusal")?;
    assert_changed_during_batch_error(&err_rm_seal, &tmp.path);
    fs::write(&cas_seal, &orig_seal)?;

    // 5. Direct post-batch helper covers unchanged pass plus save/seal/pair mismatch & missing file.
    verify_checkpoint_pair_unchanged_after_batch(
        &fix.cas_save,
        &cas_seal,
        &fix.req.pre_act_save_sha256,
        &fix.req.pre_act_seal_sha256,
        &fix.req.pre_act_pair_sha256,
    )?;
    let bad_save = verify_checkpoint_pair_unchanged_after_batch(
        &fix.cas_save,
        &cas_seal,
        &"f".repeat(64),
        &fix.req.pre_act_seal_sha256,
        &fix.req.pre_act_pair_sha256,
    )
    .err()
    .ok_or("expected save digest mismatch")?;
    assert_changed_during_batch_error(&bad_save, &tmp.path);

    let bad_seal = verify_checkpoint_pair_unchanged_after_batch(
        &fix.cas_save,
        &cas_seal,
        &fix.req.pre_act_save_sha256,
        &"e".repeat(64),
        &fix.req.pre_act_pair_sha256,
    )
    .err()
    .ok_or("expected seal digest mismatch")?;
    assert_changed_during_batch_error(&bad_seal, &tmp.path);

    let bad_pair = verify_checkpoint_pair_unchanged_after_batch(
        &fix.cas_save,
        &cas_seal,
        &fix.req.pre_act_save_sha256,
        &fix.req.pre_act_seal_sha256,
        &"d".repeat(64),
    )
    .err()
    .ok_or("expected pair digest mismatch")?;
    assert_changed_during_batch_error(&bad_pair, &tmp.path);
    assert!(bad_pair.to_string().contains("pair changed during batch"));

    Ok(())
}

struct TwoStateCliFixture {
    ckpts_dir: PathBuf,
    blind_dir: PathBuf,
    cfg_path: PathBuf,
    vc_path: PathBuf,
    pairs: Vec<(String, String, Vec<u8>, Vec<u8>)>,
}

fn setup_two_state_cli_fixture(
    tmp: &Path,
    table_id: u64,
) -> Result<TwoStateCliFixture, Box<dyn Error>> {
    let mut host = EvalHost::from_standard_setup(TableId(table_id), 4, 11, 21)?
        .with_engine_commit(COMMIT_A)?;

    let mut pairs = Vec::new();
    let mut manifest_entries = Vec::new();
    for day_idx in 0..2_u8 {
        let mut subs = BTreeMap::new();
        for s in 0..host.seat_count() {
            let obs = host.observe_seat(Seat(s))?;
            let mandate = *obs.own_mandate().ok_or("missing own_mandate")?;
            let dec = ClubBot::new(host.bot_tie_seed().wrapping_add(u64::from(s)))
                .decide_full_turn(&obs, &mandate);
            subs.insert(
                s,
                EvalActSubmission {
                    market_actions: dec.pass_a_actions,
                    pass_b_actions: dec.pass_b_actions,
                    bundle: dec.bundle,
                },
            );
        }
        host.apply_act_submissions(&subs)?;
        host.sweep_turn()?;
        let raw_dir = tmp.join(format!("raw_{day_idx}"));
        let (raw_save, raw_seal) = host.write_pre_act_checkpoint(&raw_dir)?;
        let save_bytes = fs::read(&raw_save)?;
        let seal_bytes = fs::read(&raw_seal)?;
        let (_, summary) = verify_checkpoint_causal(&save_bytes, &seal_bytes)?;
        manifest_entries.push(VerifiedCheckpointEntry {
            pre_act_pair_sha256: summary.pre_act_pair_sha256,
            pre_act_save_sha256: summary.pre_act_save_sha256,
            pre_act_seal_sha256: summary.pre_act_seal_sha256,
            engine_commit: summary.engine_commit,
            turn: summary.turn,
            seat_count: summary.seat_count,
            expected_state_digest: summary.expected_state_digest,
            turn_chain_digest: summary.turn_chain_digest,
            post_probe_state_digest: summary.post_probe_state_digest,
        });
        pairs.push((
            sha256_hex(&save_bytes),
            sha256_hex(&seal_bytes),
            save_bytes,
            seal_bytes,
        ));
    }

    manifest_entries.sort_by(|a, b| a.pre_act_pair_sha256.cmp(&b.pre_act_pair_sha256));
    let manifest = VerifiedCheckpointsManifest {
        schema_version: VERIFIED_CHECKPOINTS_SCHEMA_VERSION,
        engine_build_id: BUILD_B.to_owned(),
        save_format_version: EVAL_SAVE_FORMAT_VERSION,
        seal_format_version: EVAL_SEAL_FORMAT_VERSION,
        entries: manifest_entries,
    };
    let vc_path = tmp.join("verified_checkpoints.json");
    fs::write(&vc_path, to_sorted_pretty_json(&manifest)?)?;

    let ckpts_dir = tmp.join("checkpoints");
    let blind_dir = tmp.join("blind_inputs");
    fs::create_dir_all(&ckpts_dir)?;
    fs::create_dir_all(&blind_dir)?;
    let cfg = ScorerConfig {
        k_synth_max: 1,
        scenarios_m: 2,
        ..ScorerConfig::default()
    };
    let cfg_path = tmp.join("cfg.json");
    fs::write(&cfg_path, to_sorted_pretty_json(&cfg)?)?;

    for (idx, (save_sha, seal_sha, save_bytes, seal_bytes)) in pairs.iter().enumerate() {
        let cas_save = ckpts_dir.join(format!("{save_sha}.save.json"));
        let cas_seal = ckpts_dir.join(format!("{seal_sha}.seal"));
        fs::write(&cas_save, save_bytes)?;
        fs::write(&cas_seal, seal_bytes)?;
        let state_id = format!("game{table_id}-day-0{}-seat-0", idx + 2);
        let req = build_arm_independent_request(
            &cas_save, &cas_seal, save_sha, seal_sha, 0, &state_id, &cfg, 77,
        )?;
        fs::write(
            blind_dir.join(format!("{state_id}.json")),
            to_sorted_compact_json(&req)?,
        )?;
    }

    Ok(TwoStateCliFixture {
        ckpts_dir,
        blind_dir,
        cfg_path,
        vc_path,
        pairs,
    })
}

#[test]
fn cli_score_multi_state_batch_refuses_whole_batch_without_partial_files_on_checkpoint_change()
-> TestResult {
    let tmp = TmpDir::new("cli_multi_state")?;
    let fix = setup_two_state_cli_fixture(&tmp.path, 940_002)?;

    // 1. Clean 2-state CLI batch writes both score files and re-verifies prior checkpoint pairs.
    let ok_out = tmp.path.join("out_ok");
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let code_ok = run_cli_with_build_id(
        [
            "score",
            "--blind-inputs",
            fix.blind_dir.to_str().ok_or("utf8")?,
            "--checkpoints",
            fix.ckpts_dir.to_str().ok_or("utf8")?,
            "--config",
            fix.cfg_path.to_str().ok_or("utf8")?,
            "--out",
            ok_out.to_str().ok_or("utf8")?,
        ],
        BUILD_B,
        &mut stdout,
        &mut stderr,
    );
    assert_eq!(
        code_ok,
        EXIT_OK,
        "stderr={}",
        String::from_utf8_lossy(&stderr)
    );
    assert_eq!(String::from_utf8_lossy(&stdout).trim(), "scored_states=2");
    assert!(
        ok_out
            .join("scores/game940002-day-02-seat-0.json")
            .is_file()
    );
    assert!(
        ok_out
            .join("scores/game940002-day-03-seat-0.json")
            .is_file()
    );

    // 2. Corrupting the second state's seal fails the whole batch with zero partial files written.
    let second_seal_path = fix.ckpts_dir.join(format!("{}.seal", fix.pairs[1].1));
    let mut tampered = fix.pairs[1].3.clone();
    tampered[0] ^= 0x01;
    fs::write(&second_seal_path, &tampered)?;

    let fail_out = tmp.path.join("out_fail");
    let mut fail_stdout = Vec::new();
    let mut fail_stderr = Vec::new();
    let code_fail = run_cli_with_build_id(
        [
            "score",
            "--blind-inputs",
            fix.blind_dir.to_str().ok_or("utf8")?,
            "--checkpoints",
            fix.ckpts_dir.to_str().ok_or("utf8")?,
            "--config",
            fix.cfg_path.to_str().ok_or("utf8")?,
            "--out",
            fail_out.to_str().ok_or("utf8")?,
        ],
        BUILD_B,
        &mut fail_stdout,
        &mut fail_stderr,
    );
    assert_ne!(code_fail, EXIT_OK);
    let written: Vec<_> = fs::read_dir(fail_out.join("scores"))?.collect();
    assert!(
        written.is_empty(),
        "failed multi-state batch must emit zero partial score files, found {}",
        written.len()
    );

    Ok(())
}

#[test]
fn cli_score_cross_state_recheck_refuses_earlier_pair_mutated_after_own_batch() -> TestResult {
    let tmp = TmpDir::new("cli_cross_state_recheck")?;
    let fix = setup_two_state_cli_fixture(&tmp.path, 940_003)?;

    let first_save_path = fix.ckpts_dir.join(format!("{}.save.json", fix.pairs[0].0));
    let first_seal_path = fix.ckpts_dir.join(format!("{}.seal", fix.pairs[0].1));
    let orig_first_save = fix.pairs[0].2.clone();
    let orig_first_seal = fix.pairs[0].3.clone();

    // 1. Unverified CLI run: mutate state 0's `.save.json` right after state 0 finishes its own
    //    batch (while state 1 is about to score). Only the CLI cross-state re-check inspects
    //    earlier pairs before emitting `<out>/scores/*.json`.
    let out_save = tmp.path.join("out_mutated_save");
    let mut seen_states = Vec::new();
    let mut hook_save = |state_idx: usize, state_id: &str| {
        seen_states.push((state_idx, state_id.to_owned()));
        if state_idx == 0 {
            let mut tampered = orig_first_save.clone();
            tampered[0] ^= 0x01;
            if let Err(err) = fs::write(&first_save_path, &tampered) {
                panic!("failed to tamper state 0 save: {err}");
            }
        }
    };
    let mut stdout_save = Vec::new();
    let mut stderr_save = Vec::new();
    let code_save = run_cli_with_build_id_and_state_hook(
        [
            "score",
            "--blind-inputs",
            fix.blind_dir.to_str().ok_or("utf8")?,
            "--checkpoints",
            fix.ckpts_dir.to_str().ok_or("utf8")?,
            "--config",
            fix.cfg_path.to_str().ok_or("utf8")?,
            "--out",
            out_save.to_str().ok_or("utf8")?,
        ],
        BUILD_B,
        Some(&mut hook_save),
        &mut stdout_save,
        &mut stderr_save,
    );
    assert_eq!(seen_states.len(), 2, "both states must have scored first");
    assert_eq!(code_save, EXIT_RUNTIME_ERROR);
    let err_save_text = String::from_utf8_lossy(&stderr_save);
    assert!(
        err_save_text.contains("CHECKPOINT_CHANGED_DURING_BATCH"),
        "stderr={err_save_text}"
    );
    assert!(
        !err_save_text.contains(tmp.path.to_string_lossy().as_ref()),
        "temp path leaked in CLI error: {err_save_text}"
    );
    assert!(stdout_save.is_empty());
    let written_save: Vec<_> = fs::read_dir(out_save.join("scores"))?.collect();
    assert!(
        written_save.is_empty(),
        "cross-state re-check refusal must emit zero output files, found {}",
        written_save.len()
    );

    // Restore state 0's `.save.json` before testing `--verified-checkpoints` + `.seal` deletion.
    fs::write(&first_save_path, &orig_first_save)?;

    // 2. Verified CLI run: remove state 0's `.seal` right after state 0 finishes its own batch.
    let out_seal = tmp.path.join("out_removed_seal");
    let mut removed = false;
    let mut hook_seal = |state_idx: usize, _state_id: &str| {
        if state_idx == 0 && !removed {
            removed = true;
            if let Err(err) = fs::remove_file(&first_seal_path) {
                panic!("failed to remove state 0 seal: {err}");
            }
        }
    };
    let mut stdout_seal = Vec::new();
    let mut stderr_seal = Vec::new();
    let code_seal = run_cli_with_build_id_and_state_hook(
        [
            "score",
            "--blind-inputs",
            fix.blind_dir.to_str().ok_or("utf8")?,
            "--checkpoints",
            fix.ckpts_dir.to_str().ok_or("utf8")?,
            "--config",
            fix.cfg_path.to_str().ok_or("utf8")?,
            "--verified-checkpoints",
            fix.vc_path.to_str().ok_or("utf8")?,
            "--out",
            out_seal.to_str().ok_or("utf8")?,
        ],
        BUILD_B,
        Some(&mut hook_seal),
        &mut stdout_seal,
        &mut stderr_seal,
    );
    assert!(removed);
    assert_eq!(code_seal, EXIT_RUNTIME_ERROR);
    let err_seal_text = String::from_utf8_lossy(&stderr_seal);
    assert!(
        err_seal_text.contains("CHECKPOINT_CHANGED_DURING_BATCH"),
        "stderr={err_seal_text}"
    );
    assert!(stdout_seal.is_empty());
    let written_seal: Vec<_> = fs::read_dir(out_seal.join("scores"))?.collect();
    assert!(
        written_seal.is_empty(),
        "verified cross-state re-check refusal must emit zero output files, found {}",
        written_seal.len()
    );
    fs::write(&first_seal_path, &orig_first_seal)?;

    Ok(())
}
