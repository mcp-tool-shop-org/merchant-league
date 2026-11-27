//! Integration and statistical-golden tests for `league-eval report` and `league-eval seed-block`.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use league_core::{EvalHost, PolicyChaChaRng, TableId};
use league_eval::{
    BuildBlindInputsOptions, CRATE_GIT_SHA, CommunicationMode, CrateGitShaSource,
    DEFAULT_REPORT_BOOTSTRAP_SAMPLES, DEFAULT_REPORT_BOOTSTRAP_SEED, DEFAULT_SEED_BLOCK_RNG_SEED,
    EvalArm, EvalBranchReport, EvalManifest, FreezeOptions, JoinOptions, NoopTransport,
    PlayOptions, REPORT_BOOTSTRAP_RNG_DOMAIN, REPORT_SCHEMA_VERSION, ReportOptions,
    SEED_BLOCK_SCHEMA_VERSION, SeedBlockManifest, SeedBlockOptions, SeedClusterDiff,
    VerifyCheckpointsOptions, compute_pairwise_contrast_from_seed_diffs, execute_cli_with_git_sha,
    run_branch_with_communication_and_git_sha, run_build_blind_inputs, run_freeze, run_join,
    run_play_with_communication_and_git_sha, run_report, run_seed_block, run_verify_checkpoints,
    verify_latin_square_4x4,
};
use league_scorer::ScorerConfig;
use league_scorer::cli::run_cli as run_scorer_cli;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

type TestResult = Result<(), Box<dyn std::error::Error>>;

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);
const FALLBACK_GIT_SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

struct TmpDir(PathBuf);

impl TmpDir {
    fn new(label: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let n = TMP_SEQ.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("ml_report_sb_{label}_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir)?;
        Ok(Self(dir))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

#[cfg(unix)]
fn clear_readonly_recursive(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    if let Ok(meta) = fs::symlink_metadata(path) {
        let mode = if meta.is_dir() { 0o755 } else { 0o644 };
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode));
        if meta.is_dir()
            && let Ok(entries) = fs::read_dir(path)
        {
            for entry in entries.flatten() {
                clear_readonly_recursive(&entry.path());
            }
        }
    }
}

#[cfg(not(unix))]
fn clear_readonly_recursive(path: &Path) {
    if let Ok(meta) = fs::symlink_metadata(path) {
        let mut perms = meta.permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        if perms.readonly() {
            perms.set_readonly(false);
            let _ = fs::set_permissions(path, perms);
        }
        if meta.is_dir()
            && let Ok(entries) = fs::read_dir(path)
        {
            for entry in entries.flatten() {
                clear_readonly_recursive(&entry.path());
            }
        }
    }
}

impl Drop for TmpDir {
    fn drop(&mut self) {
        clear_readonly_recursive(&self.0);
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn clean_git_sha_str() -> &'static str {
    if CRATE_GIT_SHA.len() == 40 && CRATE_GIT_SHA.bytes().all(|b| b.is_ascii_hexdigit()) {
        CRATE_GIT_SHA
    } else {
        FALLBACK_GIT_SHA
    }
}

fn clean_git_sha() -> CrateGitShaSource<'static> {
    let sha = clean_git_sha_str();
    CrateGitShaSource::Injected {
        built_sha: sha,
        live_head: Some(sha),
        is_dirty: Some(false),
    }
}

fn run_cli(args: &[&str]) -> (i32, String, String) {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = execute_cli_with_git_sha(
        args.iter().copied().map(OsString::from),
        &mut out,
        &mut err,
        &clean_git_sha(),
    );
    (
        code,
        String::from_utf8_lossy(&out).into_owned(),
        String::from_utf8_lossy(&err).into_owned(),
    )
}

fn copy_pinned_wrapper(
    dir: &Path,
    comm: CommunicationMode,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let file_name = match comm {
        CommunicationMode::Talk => "wrappers/v2_seat_brief_2026_11_21_r3.md",
        CommunicationMode::Silent => "wrappers/v2_seat_brief_silent_2026_11_26_r1.md",
    };
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(file_name);
    let dst = dir.join(format!("wrapper_{}.md", comm.as_str()));
    fs::write(&dst, fs::read(&src)?)?;
    Ok(dst)
}

fn write_seats_file(
    dir: &Path,
    name: &str,
    value: Value,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let p = dir.join(name);
    fs::write(&p, serde_json::to_string_pretty(&value)?)?;
    Ok(p)
}

fn bot_seats_json() -> Value {
    json!({
        "scratchpad": false,
        "seats": [
            { "guild": "Foundry", "backend": "file", "model": "ClubBot" },
            { "guild": "Quarry", "backend": "file", "model": "ClubBot" },
            { "guild": "Harbor", "backend": "file", "model": "ClubBot" },
            { "guild": "Mint", "backend": "file", "model": "ClubBot" }
        ]
    })
}

fn mixed_baselines_seats_json() -> Value {
    json!({
        "scratchpad": false,
        "seats": [
            { "guild": "Foundry", "backend": "file", "model": "ClubBot" },
            { "guild": "Quarry", "backend": "file", "model": "random-legal" },
            { "guild": "Harbor", "backend": "file", "model": "always-defect" },
            { "guild": "Mint", "backend": "file", "model": "tit-for-tat" }
        ]
    })
}

fn silent_baselines_seats_json() -> Value {
    json!({
        "scratchpad": false,
        "seats": [
            { "guild": "Foundry", "backend": "file", "model": "ClubBot" },
            { "guild": "Quarry", "backend": "file", "model": "random-legal" },
            { "guild": "Harbor", "backend": "file", "model": "always-defect" },
            { "guild": "Mint", "backend": "file", "model": "ClubBot" }
        ]
    })
}

// ============================================================================
// 1. Golden analytical tests for exact sign-flip p-values & studentized CIs
// ============================================================================

#[test]
fn golden_exact_sign_flip_and_studentized_cluster_bootstrap() -> TestResult {
    // Case A: G = 4 clusters [1, 3, 5, 7] => bar_d = 4.0, se = sqrt(5/3), exact 2/16 = 0.125.
    let diffs_a = vec![
        SeedClusterDiff {
            seed: 101,
            n_paired_states: 2,
            mean_diff: 1.0,
        },
        SeedClusterDiff {
            seed: 102,
            n_paired_states: 3,
            mean_diff: 3.0,
        },
        SeedClusterDiff {
            seed: 103,
            n_paired_states: 2,
            mean_diff: 5.0,
        },
        SeedClusterDiff {
            seed: 104,
            n_paired_states: 1,
            mean_diff: 7.0,
        },
    ];
    let c_a = compute_pairwise_contrast_from_seed_diffs(
        diffs_a.clone(),
        "ev_total_equity",
        EvalArm::Off,
        EvalArm::On,
        8,
        Some(3.75),
        DEFAULT_REPORT_BOOTSTRAP_SEED,
        DEFAULT_REPORT_BOOTSTRAP_SAMPLES,
    );
    assert_eq!(c_a.n_paired_seeds, 4);
    assert_eq!(c_a.n_paired_states, 8);
    assert_eq!(c_a.sign_flip_method, "exact");
    assert!((c_a.mean_seed_diff.ok_or("mean")? - 4.0).abs() < 1e-12);
    let expected_se = (5.0_f64 / 3.0).sqrt();
    assert!((c_a.se_seed_diff.ok_or("se")? - expected_se).abs() < 1e-12);
    assert!((c_a.sign_flip_p_two_sided.ok_or("p")? - 0.125).abs() < 1e-12);

    // Independent oracle verification of the seeded studentized bootstrap quantiles at B = 10,000.
    let mut hasher = Sha256::new();
    hasher.update(REPORT_BOOTSTRAP_RNG_DOMAIN);
    hasher.update(DEFAULT_REPORT_BOOTSTRAP_SEED.to_le_bytes());
    hasher.update(b"ev_total_equity");
    hasher.update([0_u8]);
    hasher.update(b"off");
    hasher.update([0_u8]);
    hasher.update(b"on");
    let digest: [u8; 32] = hasher.finalize().into();
    let mut rng = PolicyChaChaRng::from_seed_bytes(digest);
    let d_vals = [1.0_f64, 3.0, 5.0, 7.0];
    let mut t_stats = Vec::new();
    let mut deg = 0_usize;
    for _ in 0..10_000 {
        let mut draw = [0.0_f64; 4];
        for slot in &mut draw {
            *slot = d_vals[rng.gen_index(4)];
        }
        let bar_s: f64 = draw.iter().sum::<f64>() / 4.0;
        let sq: f64 = draw.iter().map(|x| (x - bar_s).powi(2)).sum();
        let se_s = ((sq / 3.0) / 4.0).sqrt();
        if se_s <= 1e-15 * expected_se {
            deg += 1;
            continue;
        }
        t_stats.push((bar_s - 4.0) / se_s);
    }
    t_stats.sort_by(f64::total_cmp);
    let q = |p: f64| -> f64 {
        let pos = p * ((t_stats.len() - 1) as f64);
        let lo = pos.floor() as usize;
        let hi = pos.ceil() as usize;
        let f = pos - (lo as f64);
        t_stats[lo] * (1.0 - f) + t_stats[hi] * f
    };
    let oracle_lo = 4.0 - q(0.975) * expected_se;
    let oracle_hi = 4.0 - q(0.025) * expected_se;
    assert_eq!(c_a.bootstrap_samples_requested, 10_000);
    assert_eq!(c_a.bootstrap_degenerate_draws, deg);
    assert_eq!(c_a.bootstrap_valid_draws, t_stats.len());
    assert!(
        deg > 0,
        "expected all-equal bootstrap draws among B=10,000 for G=4"
    );
    assert!((c_a.studentized_ci_low.ok_or("ci_lo")? - oracle_lo).abs() < 1e-12);
    assert!((c_a.studentized_ci_high.ok_or("ci_hi")? - oracle_hi).abs() < 1e-12);

    // Repeatability check
    let c_a_repeat = compute_pairwise_contrast_from_seed_diffs(
        diffs_a,
        "ev_total_equity",
        EvalArm::Off,
        EvalArm::On,
        8,
        Some(3.75),
        DEFAULT_REPORT_BOOTSTRAP_SEED,
        DEFAULT_REPORT_BOOTSTRAP_SAMPLES,
    );
    assert_eq!(c_a, c_a_repeat);

    // Case B: G = 4 [-1, 2, 3, 4] => exact 4/16 = 0.25 two-sided p-value.
    let diffs_b = vec![
        SeedClusterDiff {
            seed: 201,
            n_paired_states: 2,
            mean_diff: -1.0,
        },
        SeedClusterDiff {
            seed: 202,
            n_paired_states: 2,
            mean_diff: 2.0,
        },
        SeedClusterDiff {
            seed: 203,
            n_paired_states: 2,
            mean_diff: 3.0,
        },
        SeedClusterDiff {
            seed: 204,
            n_paired_states: 2,
            mean_diff: 4.0,
        },
    ];
    let c_b = compute_pairwise_contrast_from_seed_diffs(
        diffs_b,
        "pbrs_delta",
        EvalArm::Control,
        EvalArm::On,
        8,
        Some(2.0),
        777,
        2_000,
    );
    assert!((c_b.sign_flip_p_two_sided.ok_or("p")? - 0.25).abs() < 1e-12);

    // Case C: Asymmetric right-skewed seed diffs => studentized interval is asymmetric around bar_d.
    let diffs_skew = vec![
        SeedClusterDiff {
            seed: 1,
            n_paired_states: 1,
            mean_diff: 0.5,
        },
        SeedClusterDiff {
            seed: 2,
            n_paired_states: 1,
            mean_diff: 0.6,
        },
        SeedClusterDiff {
            seed: 3,
            n_paired_states: 1,
            mean_diff: 0.7,
        },
        SeedClusterDiff {
            seed: 4,
            n_paired_states: 1,
            mean_diff: 0.8,
        },
        SeedClusterDiff {
            seed: 5,
            n_paired_states: 1,
            mean_diff: 10.0,
        },
    ];
    let c_skew = compute_pairwise_contrast_from_seed_diffs(
        diffs_skew,
        "ev_total_equity",
        EvalArm::Off,
        EvalArm::Control,
        5,
        None,
        42,
        10_000,
    );
    let m = c_skew.mean_seed_diff.ok_or("mean")?;
    let lo = c_skew.studentized_ci_low.ok_or("lo")?;
    let hi = c_skew.studentized_ci_high.ok_or("hi")?;
    assert!(
        ((hi - m) - (m - lo)).abs() > 1e-3,
        "studentized CI should reflect skewness"
    );

    // Case D: G = 22 triggers Monte Carlo sign flip (`> EXACT_SIGN_FLIP_MAX_SEEDS`),
    // G = 0 returns insufficient_seeds, G = 1 returns None CI and p = 1.0, zero diff returns p = 1.0.
    let diffs_22: Vec<SeedClusterDiff> = (1_u64..=22)
        .map(|s| SeedClusterDiff {
            seed: s,
            n_paired_states: 1,
            mean_diff: s as f64,
        })
        .collect();
    let c_22 = compute_pairwise_contrast_from_seed_diffs(
        diffs_22,
        "ev_total_equity",
        EvalArm::Off,
        EvalArm::On,
        22,
        Some(11.5),
        99,
        1_000,
    );
    assert_eq!(c_22.sign_flip_method, "monte_carlo");
    assert!(c_22.sign_flip_p_two_sided.ok_or("mc p")? < 0.01);
    assert!(c_22.studentized_ci_low.ok_or("ci lo")? > 0.0);

    let c_zero_g = compute_pairwise_contrast_from_seed_diffs(
        Vec::new(),
        "score_norm",
        EvalArm::Off,
        EvalArm::On,
        0,
        None,
        1,
        100,
    );
    assert_eq!(c_zero_g.sign_flip_method, "insufficient_seeds");
    assert!(c_zero_g.sign_flip_p_two_sided.is_none());

    let c_one = compute_pairwise_contrast_from_seed_diffs(
        vec![SeedClusterDiff {
            seed: 9,
            n_paired_states: 2,
            mean_diff: 3.5,
        }],
        "score_norm",
        EvalArm::Off,
        EvalArm::On,
        2,
        Some(3.5),
        1,
        100,
    );
    assert_eq!(c_one.sign_flip_p_two_sided, Some(1.0));
    assert!(c_one.se_seed_diff.is_none());
    assert!(c_one.studentized_ci_low.is_none());

    let c_flat = compute_pairwise_contrast_from_seed_diffs(
        vec![
            SeedClusterDiff {
                seed: 1,
                n_paired_states: 1,
                mean_diff: 0.0,
            },
            SeedClusterDiff {
                seed: 2,
                n_paired_states: 1,
                mean_diff: 0.0,
            },
        ],
        "score_norm",
        EvalArm::Off,
        EvalArm::On,
        2,
        Some(0.0),
        1,
        100,
    );
    assert_eq!(c_flat.sign_flip_p_two_sided, Some(1.0));
    assert!(c_flat.studentized_ci_low.is_none());
    Ok(())
}

// ============================================================================
// 2. End-to-end multi-seed `play -> branch -> blind/score/join -> report` + voided handling
// ============================================================================

#[test]
fn end_to_end_multi_seed_report_with_joined_and_voided_games() -> TestResult {
    let tmp = TmpDir::new("e2e_report")?;
    let wrapper = copy_pinned_wrapper(tmp.path(), CommunicationMode::Talk)?;
    let seats_path = write_seats_file(tmp.path(), "seats.json", bot_seats_json())?;
    let git_sha = clean_git_sha();

    let mut branch_dirs = Vec::new();
    let mut game_dirs = Vec::new();
    for (idx, seed) in [101_u64, 102, 103, 104].iter().enumerate() {
        let gdir = tmp.path().join(format!("game_{seed}"));
        run_play_with_communication_and_git_sha(
            &PlayOptions {
                game_dir: gdir.clone(),
                seats_path: seats_path.clone(),
                wrapper_path: wrapper.clone(),
                wrapper_version: None,
                serving_manifest_path: None,
                arm: EvalArm::Off,
                table_id: 700 + (idx as u64),
                seed: *seed,
                turns: 2,
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            CommunicationMode::Talk,
            &NoopTransport,
            None,
            &git_sha,
        )?;
        let bdir = tmp.path().join(format!("branch_{seed}"));
        run_branch_with_communication_and_git_sha(
            &league_eval::BranchOptions {
                checkpoint_prefix: Some(gdir.join("checkpoints/day-02-pre-act")),
                list_path: None,
                focal_seat: Some("Foundry".to_owned()),
                seats_path: seats_path.clone(),
                wrapper_path: wrapper.clone(),
                serving_manifest_path: None,
                arms: vec![EvalArm::Off, EvalArm::Control, EvalArm::On],
                out_dir: bdir.clone(),
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            CommunicationMode::Talk,
            &NoopTransport,
            None,
            &git_sha,
        )?;
        branch_dirs.push(bdir);
        game_dirs.push(gdir);
    }

    // Mark seed 104's `on` branch leaf + root voided to verify voided_games_by_arm & exclusion.
    let b104 = &branch_dirs[3];
    let mut leaf_man: EvalManifest =
        serde_json::from_str(&fs::read_to_string(b104.join("on/manifest.json"))?)?;
    leaf_man.voided = true;
    leaf_man.void_error_code = Some("MODEL_SEAT_REGENT_INTERVENTION".to_owned());
    leaf_man.voided_games_by_arm = BTreeMap::from([("on".to_owned(), 1)]);
    fs::write(
        b104.join("on/manifest.json"),
        serde_json::to_string_pretty(&leaf_man)?,
    )?;

    // Run standalone `report` without `--joined` first.
    let branches_root = tmp.path();
    let res_plain = run_report(&ReportOptions {
        branch_dirs: branch_dirs.clone(),
        joined_path: None,
        bootstrap_seed: 20_261_127,
        bootstrap_samples: 500,
        out_dir: None,
        out_json_path: None,
        out_text_path: None,
    })?;
    assert_eq!(res_plain.report.schema_version, REPORT_SCHEMA_VERSION);
    assert_eq!(res_plain.report.communication, CommunicationMode::Talk);
    assert_eq!(res_plain.report.voided_games_by_arm.get("on"), Some(&1));
    assert_eq!(res_plain.report.voided_games_by_arm.get("off"), Some(&0));
    assert_eq!(res_plain.report.n_seeds, 4);
    let on_row = res_plain
        .report
        .arms
        .iter()
        .find(|r| r.arm == EvalArm::On)
        .ok_or("missing on arm")?;
    assert_eq!(on_row.n_total_outcomes, 4);
    assert_eq!(on_row.n_valid_outcomes, 3);
    assert_eq!(on_row.n_voided_outcomes, 1);
    assert_eq!(on_row.voided_games, 1);
    assert!(on_row.mean_ev_total_equity.is_none());

    // Now run S3a/S3b pipeline on the first 3 non-voided games+branches and pass `--joined`.
    let scorer_cfg_path = tmp.path().join("scorer_cfg.json");
    let cfg = ScorerConfig {
        scenarios_m: 4,
        k_synth_max: 2,
        ..ScorerConfig::default()
    };
    fs::write(&scorer_cfg_path, serde_json::to_string_pretty(&cfg)?)?;
    let mut run_inputs = Vec::new();
    for i in 0..3 {
        run_inputs.push(game_dirs[i].clone());
        run_inputs.push(branch_dirs[i].clone());
    }
    let blind_dir = tmp.path().join("blind_inputs");
    let arm_map_dir = tmp.path().join("arm_map");
    let ckpts_dir = tmp.path().join("ckpts");
    run_build_blind_inputs(&BuildBlindInputsOptions {
        run_dirs: run_inputs,
        out_blind_dir: blind_dir.clone(),
        out_arm_map_dir: arm_map_dir.clone(),
        out_checkpoints_dir: Some(ckpts_dir.clone()),
        config_path: scorer_cfg_path.clone(),
        scorer_seed: 919,
    })?;
    let verified_path = tmp.path().join("verified_checkpoints.json");
    let _ = run_verify_checkpoints(
        &VerifyCheckpointsOptions {
            checkpoints_dir: ckpts_dir.clone(),
            out_path: verified_path.clone(),
            blind_dir: Some(blind_dir.clone()),
        },
        &git_sha,
    )?;
    let freeze_path = tmp.path().join("scorer_freeze.json");
    run_freeze(
        &FreezeOptions {
            blind_dir: blind_dir.clone(),
            config_path: scorer_cfg_path.clone(),
            control_wording_arm: "v2_control_2026_11_22_r2".to_owned(),
            out_path: freeze_path.clone(),
            verified_checkpoints_path: verified_path.clone(),
            scorer_crate_dir: None,
        },
        &git_sha,
    )?;
    let scores_dir = tmp.path().join("scores");
    let scorer_args = vec![
        OsString::from("score"),
        OsString::from("--blind-inputs"),
        blind_dir.clone().into_os_string(),
        OsString::from("--checkpoints"),
        ckpts_dir.into_os_string(),
        OsString::from("--config"),
        scorer_cfg_path.into_os_string(),
        OsString::from("--freeze"),
        freeze_path.clone().into_os_string(),
        OsString::from("--verified-checkpoints"),
        verified_path.clone().into_os_string(),
        OsString::from("--out"),
        scores_dir.clone().into_os_string(),
    ];
    let mut s_out = Vec::new();
    let mut s_err = Vec::new();
    let s_code = run_scorer_cli(scorer_args, &mut s_out, &mut s_err);
    assert_eq!(
        s_code,
        0,
        "scorer failed: {}",
        String::from_utf8_lossy(&s_err)
    );
    let join_dir = tmp.path().join("joined");
    run_join(&JoinOptions {
        freeze_path,
        arm_map_dir,
        scores_dir,
        out_dir: join_dir.clone(),
        verified_checkpoints_path: Some(verified_path),
        blind_dir: Some(blind_dir),
        source_game_dirs: game_dirs[..3].to_vec(),
    })?;

    let report_out_dir = tmp.path().join("report_out");
    let joined_file = join_dir.join("joined_decisions.json");
    let (code_cli, stdout_json, stderr_cli) = run_cli(&[
        "report",
        "--branches",
        &branch_dirs[0].to_string_lossy(),
        &branch_dirs[1].to_string_lossy(),
        "--branches",
        &format!("{},{}", branch_dirs[2].display(), branch_dirs[3].display()),
        "--joined",
        &joined_file.to_string_lossy(),
        "--bootstrap-seed",
        "77",
        "--bootstrap-samples",
        "400",
        "--format",
        "json",
        "--out",
        &report_out_dir.to_string_lossy(),
    ]);
    assert_eq!(code_cli, 0, "CLI report failed: {stderr_cli}");
    assert!(report_out_dir.join("report.json").is_file());
    assert!(report_out_dir.join("report.txt").is_file());
    let parsed_cli: EvalBranchReport = serde_json::from_str(&stdout_json)?;
    assert_eq!(parsed_cli.bootstrap_samples, 400);
    assert!(
        parsed_cli
            .arms
            .iter()
            .any(|a| a.mean_ev_total_equity.is_some())
    );
    assert!(
        parsed_cli
            .contrasts
            .iter()
            .any(|c| c.metric == "ev_total_equity")
    );
    assert!(
        fs::read_to_string(report_out_dir.join("report.txt"))?
            .contains("PAIRWISE SEED-CLUSTER CONTRASTS")
    );

    // Overwrite refusal on existing `--out`
    let (code_dup, _, err_dup) = run_cli(&[
        "report",
        "--branches",
        &branches_root.to_string_lossy(),
        "--out",
        &report_out_dir.to_string_lossy(),
    ]);
    assert_eq!(code_dup, 1);
    assert!(err_dup.contains("INVALID_USAGE"));

    Ok(())
}

#[test]
fn report_refuses_mixed_communication_missing_branches_and_bad_flags() -> TestResult {
    let tmp = TmpDir::new("report_refusals")?;
    let w_talk = copy_pinned_wrapper(tmp.path(), CommunicationMode::Talk)?;
    let w_silent = copy_pinned_wrapper(tmp.path(), CommunicationMode::Silent)?;
    let seats_path = write_seats_file(tmp.path(), "seats.json", bot_seats_json())?;
    let git_sha = clean_git_sha();

    let mut bdirs = Vec::new();
    for (label, comm, w, seed) in [
        ("talk", CommunicationMode::Talk, &w_talk, 11_u64),
        ("silent", CommunicationMode::Silent, &w_silent, 12_u64),
    ] {
        let gdir = tmp.path().join(format!("g_{label}"));
        run_play_with_communication_and_git_sha(
            &PlayOptions {
                game_dir: gdir.clone(),
                seats_path: seats_path.clone(),
                wrapper_path: w.clone(),
                wrapper_version: None,
                serving_manifest_path: None,
                arm: EvalArm::Off,
                table_id: seed,
                seed,
                turns: 1,
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            comm,
            &NoopTransport,
            None,
            &git_sha,
        )?;
        let bdir = tmp.path().join(format!("b_{label}"));
        run_branch_with_communication_and_git_sha(
            &league_eval::BranchOptions {
                checkpoint_prefix: Some(gdir.join("checkpoints/day-01-pre-act")),
                list_path: None,
                focal_seat: Some("Foundry".to_owned()),
                seats_path: seats_path.clone(),
                wrapper_path: w.clone(),
                serving_manifest_path: None,
                arms: vec![EvalArm::Off],
                out_dir: bdir.clone(),
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            comm,
            &NoopTransport,
            None,
            &git_sha,
        )?;
        bdirs.push(bdir);
    }

    // Pure silent report succeeds and serializes `"communication": "silent"`.
    let res_silent = run_report(&ReportOptions {
        branch_dirs: vec![bdirs[1].clone()],
        ..ReportOptions::default()
    })?;
    assert_eq!(res_silent.report.communication, CommunicationMode::Silent);
    assert!(res_silent.json.contains("\"communication\": \"silent\""));

    // Mixed talk + silent refuses cleanly.
    let Err(err_mix) = run_report(&ReportOptions {
        branch_dirs: bdirs,
        ..ReportOptions::default()
    }) else {
        panic!("mixed talk + silent branches must fail");
    };
    assert_eq!(err_mix.code(), "MIXED_COMMUNICATION_CONDITIONS");

    // Empty dir / zero outcomes refuses cleanly.
    let empty_dir = tmp.path().join("empty");
    fs::create_dir_all(&empty_dir)?;
    let Err(err_empty) = run_report(&ReportOptions {
        branch_dirs: vec![empty_dir],
        ..ReportOptions::default()
    }) else {
        panic!("empty branches dir must fail");
    };
    assert_eq!(err_empty.code(), "CHECKPOINT_NOT_FOUND");

    let (code_bad_fmt, _, err_fmt) = run_cli(&[
        "report",
        "--branches",
        &tmp.path().join("b_silent").to_string_lossy(),
        "--format",
        "xml",
    ]);
    assert_eq!(code_bad_fmt, 1);
    assert!(err_fmt.contains("INVALID_USAGE"));

    let (code_zero_boot, _, _) = run_cli(&[
        "report",
        "--branches",
        &tmp.path().join("b_silent").to_string_lossy(),
        "--bootstrap-samples",
        "0",
    ]);
    assert_eq!(code_zero_boot, 1);

    Ok(())
}

// ============================================================================
// 3. Latin-square `seed-block` manifest generator + real `EvalHost::fork` execution
// ============================================================================

#[test]
fn seed_block_latin_square_generates_valid_rotations_and_runs_end_to_end() -> TestResult {
    let tmp = TmpDir::new("seed_block")?;
    let wrapper = copy_pinned_wrapper(tmp.path(), CommunicationMode::Talk)?;
    let seats_path = write_seats_file(tmp.path(), "seats.json", mixed_baselines_seats_json())?;
    let sb_out = tmp.path().join("seed_block_out");

    let manifest = run_seed_block(
        &SeedBlockOptions {
            seeds: vec![101, 102, 103],
            seats_path: seats_path.clone(),
            arm: EvalArm::Off,
            communication: CommunicationMode::Talk,
            turns: 1,
            table_id_base: 500,
            block_seed: DEFAULT_SEED_BLOCK_RNG_SEED,
            out_dir: sb_out.clone(),
            allowed_vm_hosts: Vec::new(),
        },
        None,
    )?;

    assert_eq!(manifest.schema_version, SEED_BLOCK_SCHEMA_VERSION);
    assert_eq!(manifest.blocks.len(), 3);
    assert_eq!(
        manifest.table_guilds,
        ["Foundry", "Quarry", "Harbor", "Mint"]
    );

    let mut distinct_squares = std::collections::BTreeSet::new();
    for (idx, block) in manifest.blocks.iter().enumerate() {
        assert_eq!(block.table_id, 500 + (idx as u64));
        assert!(verify_latin_square_4x4(&block.latin_square));
        distinct_squares.insert(block.latin_square);
        assert_eq!(block.games.len(), 4);
    }
    assert!(
        distinct_squares.len() >= 2,
        "distinct layout seeds should vary Latin square rotations"
    );

    // Execute all 4 rotations of seed 101 from a single Turn-0 duplicate fork via `league-eval play`.
    let turn0_dir = sb_out.join(&manifest.blocks[0].turn0_dir);
    fs::create_dir_all(&turn0_dir)?;
    let base_host = EvalHost::from_standard_setup_with_model_seats(
        TableId(manifest.blocks[0].table_id),
        4,
        manifest.blocks[0].seed,
        9001,
        &[true, true, true, true],
    )?
    .with_engine_commit(clean_git_sha_str())?;
    let (turn0_save, turn0_seal) = base_host.write_game_save_and_seal(&turn0_dir)?;

    let git_sha = clean_git_sha();
    let mut receipt_jsons = Vec::new();
    for game in &manifest.blocks[0].games {
        let game_dir = sb_out.join(&game.game_dir);
        let rot_seats = sb_out.join(&game.seats_file);
        fs::create_dir_all(&game_dir)?;
        let forked = EvalHost::fork_duplicate_from_turn_0(&turn0_save, &turn0_seal)?;
        assert_ne!(forked.game_id(), base_host.game_id());
        forked.write_game_save_and_seal(&game_dir)?;
        let eval_man = run_play_with_communication_and_git_sha(
            &PlayOptions {
                game_dir: game_dir.clone(),
                seats_path: rot_seats,
                wrapper_path: wrapper.clone(),
                wrapper_version: None,
                serving_manifest_path: None,
                arm: EvalArm::Off,
                table_id: manifest.blocks[0].table_id,
                seed: manifest.blocks[0].seed,
                turns: 1,
                memory_mode: league_relay::MemoryMode::Snapshot,
                live: false,
                cap_credits: None,
                cap_gpu_secs: None,
                allowed_vm_hosts: Vec::new(),
            },
            CommunicationMode::Talk,
            &NoopTransport,
            None,
            &git_sha,
        )?;
        assert_eq!(eval_man.played_turns, 1);
        receipt_jsons.push(fs::read_to_string(
            game_dir.join("receipts/day-01.receipt.json"),
        )?);
    }
    assert_eq!(receipt_jsons.len(), 4);

    // Determinism on repeat run in a fresh directory.
    let sb_out_2 = tmp.path().join("seed_block_out_2");
    let manifest_2 = run_seed_block(
        &SeedBlockOptions {
            seeds: vec![101, 102, 103],
            seats_path: seats_path.clone(),
            arm: EvalArm::Off,
            communication: CommunicationMode::Talk,
            turns: 1,
            table_id_base: 500,
            block_seed: DEFAULT_SEED_BLOCK_RNG_SEED,
            out_dir: sb_out_2.clone(),
            allowed_vm_hosts: Vec::new(),
        },
        None,
    )?;
    assert_eq!(manifest, manifest_2);

    // Overwrite refusal in `sb_out` via CLI
    let (code_dup, _, err_dup) = run_cli(&[
        "seed-block",
        "--seats",
        &seats_path.to_string_lossy(),
        "--seeds",
        "101,102",
        "--out",
        &sb_out.to_string_lossy(),
    ]);
    assert_eq!(code_dup, 1);
    assert!(err_dup.contains("INVALID_USAGE"));

    // Silent condition refuses `tit-for-tat` cleanly, succeeds on silent-compatible baselines via CLI.
    let (code_tft, _, err_tft) = run_cli(&[
        "seed-block",
        "--seats",
        &seats_path.to_string_lossy(),
        "--seeds",
        "101",
        "--communication",
        "silent",
        "--out",
        &tmp.path().join("sb_tft_silent").to_string_lossy(),
    ]);
    assert_eq!(code_tft, 1);
    assert!(err_tft.contains("BASELINE_REQUIRES_TALK"));

    let silent_seats = write_seats_file(
        tmp.path(),
        "silent_seats.json",
        silent_baselines_seats_json(),
    )?;
    let sb_silent_dir = tmp.path().join("sb_silent_ok");
    let (code_ok, stdout_ok, stderr_ok) = run_cli(&[
        "seed-block",
        "--seats",
        &silent_seats.to_string_lossy(),
        "--seeds",
        "201,202",
        "--communication",
        "silent",
        "--arm",
        "control",
        "--turns",
        "4",
        "--out",
        &sb_silent_dir.to_string_lossy(),
    ]);
    assert_eq!(code_ok, 0, "CLI silent seed-block failed: {stderr_ok}");
    assert!(stdout_ok.contains("seed_blocks=2 rotated_games=8"));
    let parsed_silent: SeedBlockManifest = serde_json::from_str(&fs::read_to_string(
        sb_silent_dir.join("seed_block_manifest.json"),
    )?)?;
    assert_eq!(parsed_silent.communication, CommunicationMode::Silent);
    assert_eq!(parsed_silent.arm, EvalArm::Control);
    assert_eq!(parsed_silent.turns, 4);

    // Invalid Latin-square check
    assert!(!verify_latin_square_4x4(&[
        [0, 1, 2, 3],
        [0, 1, 2, 3],
        [2, 3, 0, 1],
        [3, 2, 1, 0],
    ]));

    Ok(())
}

// ============================================================================
// 4. Edge-case validation, CLI flag variants, and synthetic CI rendering tests
// ============================================================================

#[test]
fn cli_and_validation_edge_cases_for_report_and_seed_block() -> TestResult {
    let tmp = TmpDir::new("edge_cases")?;

    // 4a. `verify_latin_square_4x4` false paths: >=4, duplicate in row, duplicate in column.
    assert!(!verify_latin_square_4x4(&[
        [4, 1, 2, 3],
        [1, 2, 3, 0],
        [2, 3, 0, 1],
        [3, 0, 1, 2],
    ]));
    assert!(!verify_latin_square_4x4(&[
        [0, 0, 2, 3],
        [1, 2, 3, 0],
        [2, 3, 0, 1],
        [3, 0, 1, 2],
    ]));
    assert!(!verify_latin_square_4x4(&[
        [0, 1, 2, 3],
        [0, 2, 3, 1],
        [2, 3, 0, 1],
        [3, 0, 1, 2],
    ]));

    // 4b. Top-level JSON array seats file + CLI `seed-block` full flags.
    let array_seats = write_seats_file(
        tmp.path(),
        "array_seats.json",
        json!([
            { "guild": "Foundry", "backend": "file", "model": "ClubBot" },
            { "guild": "Quarry", "backend": "file", "model": "random-legal" },
            { "guild": "Harbor", "backend": "file", "model": "always-defect" },
            { "guild": "Mint", "backend": "file", "model": "ClubBot" }
        ]),
    )?;
    let sb_arr_out = tmp.path().join("sb_arr_out");
    let (code_arr, stdout_arr, stderr_arr) = run_cli(&[
        "seed-block",
        "--error-format",
        "json",
        "--seats",
        &array_seats.to_string_lossy(),
        "--seeds",
        "11,12",
        "--arm",
        "on",
        "--communication",
        "silent",
        "--turns",
        "4",
        "--table-id-base",
        "10",
        "--block-seed",
        "99",
        "--out",
        &sb_arr_out.to_string_lossy(),
    ]);
    assert_eq!(code_arr, 0, "array seats seed-block failed: {stderr_arr}");
    assert!(stdout_arr.contains("seed_blocks=2 rotated_games=8"));

    let (code_allow_unused, _, err_allow_unused) = run_cli(&[
        "seed-block",
        "--seats",
        &array_seats.to_string_lossy(),
        "--seeds",
        "11",
        "--allow-vm-host",
        "localhost",
        "--out",
        &tmp.path().join("sb_allow_unused").to_string_lossy(),
    ]);
    assert_eq!(code_allow_unused, 1);
    assert!(err_allow_unused.contains("INVALID_SEAT_CONFIG"));

    // 4c. `run_seed_block` validation error paths.
    let base_sb = SeedBlockOptions {
        seeds: vec![1],
        seats_path: array_seats.clone(),
        arm: EvalArm::Off,
        communication: CommunicationMode::Silent,
        turns: 1,
        table_id_base: 1,
        block_seed: 1,
        out_dir: tmp.path().join("sb_unused"),
        allowed_vm_hosts: Vec::new(),
    };
    let mut empty_seeds = base_sb.clone();
    empty_seeds.seeds = Vec::new();
    let Err(e) = run_seed_block(&empty_seeds, None) else {
        panic!("empty seeds must fail");
    };
    assert_eq!(e.code(), "INVALID_USAGE");

    let mut zero_turns = base_sb.clone();
    zero_turns.turns = 0;
    let Err(e) = run_seed_block(&zero_turns, None) else {
        panic!("turns=0 must fail");
    };
    assert_eq!(e.code(), "INVALID_USAGE");

    let mut big_turns = base_sb.clone();
    big_turns.turns = 33;
    let Err(e) = run_seed_block(&big_turns, None) else {
        panic!("turns=33 must fail");
    };
    assert_eq!(e.code(), "INVALID_USAGE");

    let mut dup_seeds = base_sb.clone();
    dup_seeds.seeds = vec![7, 7];
    let Err(e) = run_seed_block(&dup_seeds, None) else {
        panic!("duplicate seeds must fail");
    };
    assert_eq!(e.code(), "INVALID_USAGE");

    let mut overflow_tid = base_sb;
    overflow_tid.table_id_base = u64::MAX;
    let Err(e) = run_seed_block(&overflow_tid, None) else {
        panic!("table_id_base overflow must fail");
    };
    assert_eq!(e.code(), "INVALID_USAGE");

    // 4d. `cmd_seed_block` CLI flag error paths.
    let arr_s = array_seats.to_string_lossy().into_owned();
    let dummy_out = tmp.path().join("sb_cli_err").to_string_lossy().into_owned();
    for bad_args in [
        vec![
            "seed-block",
            "--seats",
            &arr_s,
            "--seeds",
            "abc",
            "--out",
            &dummy_out,
        ],
        vec![
            "seed-block",
            "--seats",
            &arr_s,
            "--seeds",
            "1",
            "--turns",
            "abc",
            "--out",
            &dummy_out,
        ],
        vec![
            "seed-block",
            "--seats",
            &arr_s,
            "--seeds",
            "1",
            "--table-id-base",
            "abc",
            "--out",
            &dummy_out,
        ],
        vec![
            "seed-block",
            "--seats",
            &arr_s,
            "--seeds",
            "1",
            "--block-seed",
            "abc",
            "--out",
            &dummy_out,
        ],
        vec![
            "seed-block",
            "--seats",
            &arr_s,
            "--seeds",
            "1",
            "--bogus",
            "--out",
            &dummy_out,
        ],
        vec!["seed-block", "--seeds", "1", "--out", &dummy_out],
        vec!["seed-block", "--seats", &arr_s, "--seeds", "1"],
    ] {
        let (code, _, _) = run_cli(&bad_args);
        assert_eq!(code, 1, "expected CLI failure for {bad_args:?}");
    }

    // 4e. Build one real branch leaf pair so synthetic multi-seed fixtures match exact schemas.
    let wrapper = copy_pinned_wrapper(tmp.path(), CommunicationMode::Talk)?;
    let seats_path = write_seats_file(tmp.path(), "bot_seats.json", bot_seats_json())?;
    let git_sha = clean_git_sha();
    let tpl_game = tmp.path().join("tpl_game");
    run_play_with_communication_and_git_sha(
        &PlayOptions {
            game_dir: tpl_game.clone(),
            seats_path: seats_path.clone(),
            wrapper_path: wrapper.clone(),
            wrapper_version: None,
            serving_manifest_path: None,
            arm: EvalArm::Off,
            table_id: 700,
            seed: 700,
            turns: 1,
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        CommunicationMode::Talk,
        &NoopTransport,
        None,
        &git_sha,
    )?;
    let tpl_branch = tmp.path().join("tpl_branch");
    run_branch_with_communication_and_git_sha(
        &league_eval::BranchOptions {
            checkpoint_prefix: Some(tpl_game.join("checkpoints/day-01-pre-act")),
            list_path: None,
            focal_seat: Some("Foundry".to_owned()),
            seats_path,
            wrapper_path: wrapper,
            serving_manifest_path: None,
            arms: vec![EvalArm::Off, EvalArm::On],
            out_dir: tpl_branch.clone(),
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        CommunicationMode::Talk,
        &NoopTransport,
        None,
        &git_sha,
    )?;

    let tpl_off_outcome: Value =
        serde_json::from_str(&fs::read_to_string(tpl_branch.join("off/outcome.json"))?)?;
    let tpl_off_manifest: Value =
        serde_json::from_str(&fs::read_to_string(tpl_branch.join("off/manifest.json"))?)?;
    let tpl_on_outcome: Value =
        serde_json::from_str(&fs::read_to_string(tpl_branch.join("on/outcome.json"))?)?;
    let tpl_on_manifest: Value =
        serde_json::from_str(&fs::read_to_string(tpl_branch.join("on/manifest.json"))?)?;
    let tpl_root_manifest: Value =
        serde_json::from_str(&fs::read_to_string(tpl_branch.join("manifest.json"))?)?;

    // Helper closure writing a `(outcome.json, manifest.json)` leaf pair with overrides.
    let write_leaf = |dir: &Path,
                      base_out: &Value,
                      base_man: &Value,
                      game_id: &str,
                      seed: u64,
                      pbrs0: i64|
     -> Result<(), Box<dyn std::error::Error>> {
        fs::create_dir_all(dir)?;
        let mut out_v = base_out.clone();
        out_v["source_game_id"] = json!(game_id);
        out_v["decision"]["game_id"] = json!(game_id);
        if let Some(deltas) = out_v["receipt"]["pbrs_deltas"].as_array_mut()
            && !deltas.is_empty()
        {
            deltas[0] = json!(pbrs0);
        }
        let mut man_v = base_man.clone();
        man_v["game_id"] = json!(game_id);
        man_v["seed"] = json!(seed);
        fs::write(
            dir.join("outcome.json"),
            serde_json::to_string_pretty(&out_v)?,
        )?;
        fs::write(
            dir.join("manifest.json"),
            serde_json::to_string_pretty(&man_v)?,
        )?;
        Ok(())
    };

    // 4f. 3-seed synthetic branch tree with positive spread (`+10, +25, +45` -> non-zero SE + `[lo, hi]` CI),
    // plus a root `manifest.json` (`BranchManifest`) without `branch_summary.json`.
    let synth_root = tmp.path().join("synth_branches");
    fs::create_dir_all(&synth_root)?;
    fs::write(
        synth_root.join("manifest.json"),
        serde_json::to_string_pretty(&tpl_root_manifest)?,
    )?;
    for (seed, gid, delta) in [
        (1_u64, "g1", 10_i64),
        (2_u64, "g2", 25_i64),
        (3_u64, "g3", 45_i64),
    ] {
        write_leaf(
            &synth_root.join(format!("seed_{seed}/off")),
            &tpl_off_outcome,
            &tpl_off_manifest,
            gid,
            seed,
            100,
        )?;
        write_leaf(
            &synth_root.join(format!("seed_{seed}/on")),
            &tpl_on_outcome,
            &tpl_on_manifest,
            gid,
            seed,
            100 + delta,
        )?;
    }

    let out_json_nested = tmp.path().join("nested_json/sub/r.json");
    let out_text_nested = tmp.path().join("nested_text/sub/r.txt");
    let (code_rep_txt, stdout_rep_txt, stderr_rep_txt) = run_cli(&[
        "report",
        "--error-format",
        "json",
        "--branches",
        &synth_root.to_string_lossy(),
        "--bootstrap-seed",
        "7",
        "--bootstrap-samples",
        "200",
        "--format",
        "text",
        "--out-json",
        &out_json_nested.to_string_lossy(),
        "--out-text",
        &out_text_nested.to_string_lossy(),
    ]);
    assert_eq!(code_rep_txt, 0, "report text CLI failed: {stderr_rep_txt}");
    assert!(stdout_rep_txt.contains('[') && stdout_rep_txt.contains(']'));
    assert!(out_json_nested.is_file());
    assert!(out_text_nested.is_file());

    let (code_rep_json, stdout_rep_json, stderr_rep_json) = run_cli(&[
        "report",
        "--branches",
        &synth_root.to_string_lossy(),
        "--format",
        "json",
    ]);
    assert_eq!(
        code_rep_json, 0,
        "report json CLI failed: {stderr_rep_json}"
    );
    let parsed_cli_rep: EvalBranchReport = serde_json::from_str(&stdout_rep_json)?;
    assert_eq!(parsed_cli_rep.n_seeds, 3);

    // 4g. `run_report` error paths.
    let Err(e) = run_report(&ReportOptions::default()) else {
        panic!("empty branch_dirs must fail");
    };
    assert_eq!(e.code(), "INVALID_USAGE");

    let Err(e) = run_report(&ReportOptions {
        branch_dirs: vec![tmp.path().join("does_not_exist")],
        ..ReportOptions::default()
    }) else {
        panic!("missing branch dir must fail");
    };
    assert_eq!(e.code(), "CHECKPOINT_NOT_FOUND");

    let no_man_dir = tmp.path().join("err_no_man");
    fs::create_dir_all(&no_man_dir)?;
    fs::write(
        no_man_dir.join("outcome.json"),
        serde_json::to_string_pretty(&tpl_off_outcome)?,
    )?;
    let Err(e) = run_report(&ReportOptions {
        branch_dirs: vec![no_man_dir],
        ..ReportOptions::default()
    }) else {
        panic!("outcome without manifest must fail");
    };
    assert_eq!(e.code(), "CHECKPOINT_NOT_FOUND");

    let arm_mismatch_dir = tmp.path().join("err_arm_mismatch");
    fs::create_dir_all(&arm_mismatch_dir)?;
    fs::write(
        arm_mismatch_dir.join("outcome.json"),
        serde_json::to_string_pretty(&tpl_on_outcome)?,
    )?;
    fs::write(
        arm_mismatch_dir.join("manifest.json"),
        serde_json::to_string_pretty(&tpl_off_manifest)?,
    )?;
    let Err(e) = run_report(&ReportOptions {
        branch_dirs: vec![arm_mismatch_dir],
        ..ReportOptions::default()
    }) else {
        panic!("arm mismatch must fail");
    };
    assert_eq!(e.code(), "INCOMPATIBLE_EVAL_SETUP");

    let seed_conflict_dir = tmp.path().join("err_seed_conflict");
    write_leaf(
        &seed_conflict_dir.join("a"),
        &tpl_off_outcome,
        &tpl_off_manifest,
        "same_gid",
        1,
        10,
    )?;
    write_leaf(
        &seed_conflict_dir.join("b"),
        &tpl_on_outcome,
        &tpl_on_manifest,
        "same_gid",
        2,
        20,
    )?;
    let Err(e) = run_report(&ReportOptions {
        branch_dirs: vec![seed_conflict_dir],
        ..ReportOptions::default()
    }) else {
        panic!("conflicting seed for same source_game_id must fail");
    };
    assert_eq!(e.code(), "INCOMPATIBLE_EVAL_SETUP");

    let bad_joined_path = tmp.path().join("silent_joined.json");
    fs::write(
        &bad_joined_path,
        serde_json::to_string_pretty(&json!({
            "schema_version": 1,
            "communication": "silent",
            "inferential_status": "descriptive_only",
            "freeze_sha256": "0".repeat(64),
            "verified_checkpoints_sha256": "0".repeat(64),
            "scorer_engine_build_id": clean_git_sha_str(),
            "checkpoint_engine_commits": [],
            "scenario_randomness_scope": "independent_per_state",
            "rows": []
        }))?,
    )?;
    let Err(e) = run_report(&ReportOptions {
        branch_dirs: vec![synth_root.clone()],
        joined_path: Some(bad_joined_path),
        ..ReportOptions::default()
    }) else {
        panic!("mismatched --joined communication must fail");
    };
    assert_eq!(e.code(), "MIXED_COMMUNICATION_CONDITIONS");

    // 4h. `cmd_report` CLI flag error paths.
    let sr = synth_root.to_string_lossy().into_owned();
    for bad_args in [
        vec!["report", "--branches", &sr, "--bootstrap-seed", "abc"],
        vec!["report", "--branches", &sr, "--bootstrap-samples", "abc"],
        vec!["report", "--branches", &sr, "--bogus"],
    ] {
        let (code, _, _) = run_cli(&bad_args);
        assert_eq!(code, 1, "expected CLI failure for {bad_args:?}");
    }

    Ok(())
}
