//! Release timing benchmark for early, mid, and late fork-to-close rollouts.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use league_core::{ClubBot, EvalActSubmission, EvalHost, Seat, TableId, TurnPhase};
use league_scorer::{
    ScorerConfig, compute_pre_act_pair_sha256_hex, decide_peer_turn, draw_scenarios,
    generate_arm_independent_candidates, posture_for_scenario, run_single_rollout, sha256_hex,
};

static BENCH_TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

struct BenchTmpDir {
    path: PathBuf,
}

impl BenchTmpDir {
    fn new(label: &str) -> Result<Self, Box<dyn Error>> {
        let id = BENCH_TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "league_scorer_bench_{label}_{}_{id}",
            std::process::id()
        ));
        if dir.exists() {
            let _ = fs::remove_dir_all(&dir);
        }
        fs::create_dir_all(&dir)?;
        Ok(Self { path: dir })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for BenchTmpDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
#[ignore]
fn timing_benchmark_early_mid_late_rollouts() -> Result<(), Box<dyn Error>> {
    let tmp = BenchTmpDir::new("timing")?;
    let ckpt_dir = tmp.path().join("ckpts");
    fs::create_dir_all(&ckpt_dir)?;

    let mut host = EvalHost::from_standard_setup(TableId(9001), 4, 1, 42)?;
    let mut saved_by_day: BTreeMap<u8, (PathBuf, PathBuf)> = BTreeMap::new();

    while host.phase() == TurnPhase::TalkAndMarket {
        let day = host.turn();
        let (save_p, seal_p) = host.write_pre_act_checkpoint(&ckpt_dir)?;
        saved_by_day.insert(day, (save_p, seal_p));

        let mut subs = BTreeMap::new();
        for s in 0..4_u8 {
            let obs = host.observe_seat(Seat(s))?;
            let mandate = *obs.own_mandate().ok_or("missing own_mandate")?;
            let dec =
                ClubBot::new(42_u64.wrapping_add(u64::from(s))).decide_full_turn(&obs, &mandate);
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

    let (sb, _, _, _) = host.finalize_eval_season()?;
    let closed_turn = sb.closed_turn;
    let early_day = 1_u8;
    let mid_day = (closed_turn / 2).max(2);
    let late_day = closed_turn;
    eprintln!(
        "TIMING_BENCH_SEASON closed_turn={closed_turn} early_day={early_day} mid_day={mid_day} late_day={late_day}"
    );

    let cfg = ScorerConfig::default();
    const REPEATS: usize = 5;
    for (label, day) in [("early", early_day), ("mid", mid_day), ("late", late_day)] {
        let (save_p, seal_p) = saved_by_day
            .get(&day)
            .ok_or_else(|| format!("missing checkpoint for day {day}"))?;
        let save_bytes = fs::read(save_p)?;
        let seal_bytes = fs::read(seal_p)?;
        let save_sha = sha256_hex(&save_bytes);
        let seal_sha = sha256_hex(&seal_bytes);
        let pair_sha = compute_pre_act_pair_sha256_hex(&save_bytes, &seal_bytes);

        let probe = EvalHost::fork_from_checkpoint(save_p, seal_p)?;
        let focal_obs = probe.observe_seat(Seat(0))?;
        let focal_mandate = *focal_obs.own_mandate().ok_or("missing focal own_mandate")?;

        let gen_t0 = Instant::now();
        let generated =
            generate_arm_independent_candidates(&focal_obs, &focal_mandate, &cfg, 12345)?;
        let gen_ms = gen_t0.elapsed().as_secs_f64() * 1000.0;

        let scenarios =
            draw_scenarios(&focal_obs.spec, focal_mandate, 0, 12345, &pair_sha, day, 4)?;
        let mut peer_day_subs_by_m = Vec::with_capacity(scenarios.len());
        for (m, scenario) in scenarios.iter().enumerate() {
            let posture = posture_for_scenario(&cfg, m);
            let mut peer_day_subs = BTreeMap::new();
            for s in 1..4_u8 {
                let p_obs = probe.observe_seat(Seat(s))?;
                let p_mandate = *scenario
                    .peer_mandates
                    .get(&s)
                    .ok_or("missing peer mandate")?;
                let sub = decide_peer_turn(
                    posture,
                    scenario.bot_base_seed.wrapping_add(u64::from(s)),
                    &p_obs,
                    &p_mandate,
                    &cfg,
                )?;
                peer_day_subs.insert(s, sub);
            }
            peer_day_subs_by_m.push(peer_day_subs);
        }
        let passive = EvalActSubmission::default();
        let digests = (save_sha.as_str(), seal_sha.as_str(), pair_sha.as_str());

        // Warm once then run REPEATS timed fork-to-close rollouts across S2 scenario postures.
        let _warm = run_single_rollout(
            save_p,
            seal_p,
            digests,
            0,
            &passive,
            &peer_day_subs_by_m[0],
            focal_mandate,
            &scenarios[0],
            posture_for_scenario(&cfg, 0),
            &cfg,
        )?;

        let mut samples_ms = Vec::with_capacity(REPEATS);
        let mut checksum = 0_i64;
        for rep in 0..REPEATS {
            let m = rep % scenarios.len();
            let posture = posture_for_scenario(&cfg, m);
            let t0 = Instant::now();
            let out = run_single_rollout(
                save_p,
                seal_p,
                digests,
                0,
                &passive,
                &peer_day_subs_by_m[m],
                focal_mandate,
                &scenarios[m],
                posture,
                &cfg,
            )?;
            let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;
            samples_ms.push(elapsed_ms);
            checksum = checksum.wrapping_add(out.total_equity);
        }
        let mean_ms = samples_ms.iter().sum::<f64>() / (REPEATS as f64);
        let min_ms = samples_ms.iter().copied().fold(f64::INFINITY, f64::min);
        let max_ms = samples_ms.iter().copied().fold(0.0_f64, f64::max);
        eprintln!(
            "TIMING_BENCH {label} day={day} gen_candidates={} gen_ms={gen_ms:.3} repeats={REPEATS} mean_ms={mean_ms:.3} min_ms={min_ms:.3} max_ms={max_ms:.3} samples={samples_ms:?} checksum={checksum}",
            generated.len()
        );
    }
    Ok(())
}
