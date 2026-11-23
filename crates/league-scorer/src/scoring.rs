//! Pure deterministic scoring math, arm-blind pre-screen, and Winner's curse estimation (`§7.1`, `§3.2`).

use crate::error::ScorerError;
use crate::types::{
    BLIND_SCORING_SCHEMA_VERSION, BlindScoringRequest, CandidateScoreOutput, PrescreenSummary,
    ScenarioRolloutOutcome, ScorerConfig, StateScoreOutput, WinnersCurseSummary, sha256_hex,
    to_sorted_pretty_json,
};

struct CandidateTotals {
    sum_total: i128,
    sum_project: i128,
    sum_net_asset: i128,
    sum_mandate: i128,
    half_a_total: i128,
    half_b_total: i128,
}

fn sum_candidate_outcomes(outcomes: &[ScenarioRolloutOutcome], half: usize) -> CandidateTotals {
    let mut totals = CandidateTotals {
        sum_total: 0,
        sum_project: 0,
        sum_net_asset: 0,
        sum_mandate: 0,
        half_a_total: 0,
        half_b_total: 0,
    };
    for (m, out) in outcomes.iter().enumerate() {
        let tot = i128::from(out.total_equity);
        totals.sum_total += tot;
        totals.sum_project += i128::from(out.project_equity);
        totals.sum_net_asset += i128::from(out.net_asset_equity);
        totals.sum_mandate += i128::from(out.mandate_bonus);
        if m < half {
            totals.half_a_total += tot;
        } else {
            totals.half_b_total += tot;
        }
    }
    totals
}

/// Computes sample standard error of paired per-scenario differences `outcomes_a[m] - outcomes_b[m]`
/// in ascending `0..M` order (`§7.1`).
fn compute_paired_se(
    outcomes_a: &[ScenarioRolloutOutcome],
    outcomes_b: &[ScenarioRolloutOutcome],
) -> f64 {
    let m_len = outcomes_a.len();
    if m_len < 2 {
        return 0.0;
    }
    let mut diffs = Vec::with_capacity(m_len);
    let mut sum_d = 0_i128;
    for (a, b) in outcomes_a.iter().zip(outcomes_b.iter()) {
        let d = i128::from(a.total_equity) - i128::from(b.total_equity);
        sum_d += d;
        diffs.push(d);
    }
    let m_f64 = m_len as f64;
    let mean_d = (sum_d as f64) / m_f64;
    let mut sse = 0.0_f64;
    for d in diffs {
        let delta = (d as f64) - mean_d;
        sse += delta * delta;
    }
    let sample_var = sse / ((m_len - 1) as f64);
    (sample_var.sqrt()) / m_f64.sqrt()
}

/// Evaluates arm-blind pre-screen metrics strictly over `is_arm_independent == true` candidates (`§3.2`).
fn compute_prescreen(
    request: &BlindScoringRequest,
    config: &ScorerConfig,
    totals: &[CandidateTotals],
) -> PrescreenSummary {
    let m_len = config.scenarios_m;
    let m_f64 = m_len as f64;
    let mut indep_sums = Vec::new();
    for (cand, tot) in request.candidates.iter().zip(totals.iter()) {
        if cand.is_arm_independent {
            indep_sums.push(tot.sum_total);
        }
    }
    let n_feasible_indep = indep_sums.len();
    let Some(min_sum) = indep_sums.iter().copied().min() else {
        return PrescreenSummary {
            excluded_by_prescreen: true,
            is_flat_indep: true,
            n_feasible_indep: 0,
            span_indep: 0.0,
            v_max_indep: None,
            v_min_indep: None,
        };
    };
    let max_sum = indep_sums.iter().copied().max().unwrap_or(min_sum);
    let diff_sum = max_sum - min_sum;
    let is_flat_indep = diff_sum == 0;
    let v_min_indep = Some((min_sum as f64) / m_f64);
    let v_max_indep = Some((max_sum as f64) / m_f64);
    let span_indep = (diff_sum as f64) / m_f64;
    let threshold_scaled =
        i128::from(config.prescreen_min_arm_indep_span) * i128::from(m_len as u64);
    let excluded_by_prescreen = (config.prescreen_exclude_flat_sets && is_flat_indep)
        || diff_sum < threshold_scaled
        || n_feasible_indep < config.prescreen_min_arm_indep_candidates;

    PrescreenSummary {
        excluded_by_prescreen,
        is_flat_indep,
        n_feasible_indep,
        span_indep,
        v_max_indep,
        v_min_indep,
    }
}

/// Computes split-half Winner's curse selection-bias estimate over `in_span == true` candidates (`§7.1`).
fn compute_winners_curse(
    request: &BlindScoringRequest,
    totals: &[CandidateTotals],
    half: usize,
) -> Result<WinnersCurseSummary, ScorerError> {
    let mut best_a: Option<(usize, i128)> = None;
    let mut best_b: Option<(usize, i128)> = None;
    for (idx, (cand, tot)) in request.candidates.iter().zip(totals.iter()).enumerate() {
        if !cand.in_span {
            continue;
        }
        if best_a.is_none_or(|(_, best_val)| tot.half_a_total > best_val) {
            best_a = Some((idx, tot.half_a_total));
        }
        if best_b.is_none_or(|(_, best_val)| tot.half_b_total > best_val) {
            best_b = Some((idx, tot.half_b_total));
        }
    }
    let (Some((idx_a, _)), Some((idx_b, _))) = (best_a, best_b) else {
        return Err(ScorerError::NoInSpanCandidates(format!(
            "cannot compute split-half Winner's curse without an in_span candidate in `{}`",
            request.state_id
        )));
    };
    let half_f64 = half as f64;
    let ev_ha_a = (totals[idx_a].half_a_total as f64) / half_f64;
    let ev_hb_a = (totals[idx_a].half_b_total as f64) / half_f64;
    let ev_hb_b = (totals[idx_b].half_b_total as f64) / half_f64;
    let ev_ha_b = (totals[idx_b].half_a_total as f64) / half_f64;
    let diff_a_to_b_equity = ev_ha_a - ev_hb_a;
    let diff_b_to_a_equity = ev_hb_b - ev_ha_b;
    let bias_estimate_equity = 0.5 * (diff_a_to_b_equity + diff_b_to_a_equity);
    Ok(WinnersCurseSummary {
        bias_estimate_equity,
        c_star_half_a: request.candidates[idx_a].candidate_id.clone(),
        c_star_half_b: request.candidates[idx_b].candidate_id.clone(),
        diff_a_to_b_equity,
        diff_b_to_a_equity,
        half_scenarios: half,
        is_half_m_upper_estimate: true,
    })
}

/// Computes complete deterministic [`StateScoreOutput`] from indexed per-scenario rollout outcomes.
///
/// `outcomes` must have shape `[request.candidates.len()][config.scenarios_m]`.
pub fn score_outcomes(
    request: &BlindScoringRequest,
    config: &ScorerConfig,
    day: u8,
    outcomes: &[Vec<ScenarioRolloutOutcome>],
) -> Result<StateScoreOutput, ScorerError> {
    config.validate()?;
    request.validate(config)?;
    let k = request.candidates.len();
    let m_len = config.scenarios_m;
    if outcomes.len() != k {
        return Err(ScorerError::InvalidScorerConfig(format!(
            "outcomes candidate count {} != request candidates {k}",
            outcomes.len()
        )));
    }
    for (idx, row) in outcomes.iter().enumerate() {
        if row.len() != m_len {
            return Err(ScorerError::InvalidScorerConfig(format!(
                "outcomes[{idx}] scenario count {} != scenarios_m {m_len}",
                row.len()
            )));
        }
    }

    let half = m_len / 2;
    let m_f64 = m_len as f64;
    let totals: Vec<CandidateTotals> = outcomes
        .iter()
        .map(|row| sum_candidate_outcomes(row, half))
        .collect();

    let mut span_min: Option<i128> = None;
    let mut span_max: Option<(usize, i128)> = None;
    for (idx, (cand, tot)) in request.candidates.iter().zip(totals.iter()).enumerate() {
        if !cand.in_span {
            continue;
        }
        span_min = Some(match span_min {
            Some(cur) if cur <= tot.sum_total => cur,
            _ => tot.sum_total,
        });
        if span_max.is_none_or(|(_, best)| tot.sum_total > best) {
            span_max = Some((idx, tot.sum_total));
        }
    }
    let (Some(min_sum), Some((best_idx, max_sum))) = (span_min, span_max) else {
        return Err(ScorerError::NoInSpanCandidates(format!(
            "request `{}` has no in_span == true candidate",
            request.state_id
        )));
    };

    let v_min = (min_sum as f64) / m_f64;
    let v_max = (max_sum as f64) / m_f64;
    let span_sum = max_sum - min_sum;
    let degenerate_flat_set = span_sum == 0;
    let c_best = request.candidates[best_idx].candidate_id.clone();

    let indep_sums: Vec<i128> = request
        .candidates
        .iter()
        .zip(totals.iter())
        .filter(|(c, _)| c.is_arm_independent)
        .map(|(_, t)| t.sum_total)
        .collect();
    let n_indep = indep_sums.len();

    let mut candidate_outputs = Vec::with_capacity(k);
    for (idx, (cand, tot)) in request.candidates.iter().zip(totals.iter()).enumerate() {
        let ev_total_equity = (tot.sum_total as f64) / m_f64;
        let mean_project_equity = (tot.sum_project as f64) / m_f64;
        let mean_net_asset_equity = (tot.sum_net_asset as f64) / m_f64;
        let mean_mandate_bonus = (tot.sum_mandate as f64) / m_f64;
        let score_norm = if degenerate_flat_set {
            None
        } else {
            Some(((tot.sum_total - min_sum) as f64) / (span_sum as f64))
        };
        let regret_equity = ((max_sum - tot.sum_total) as f64) / m_f64;
        let paired_se_equity = compute_paired_se(&outcomes[best_idx], &outcomes[idx]);

        let (sensitivity_indep_rank, sensitivity_indep_percentile) = if n_indep == 0 {
            (None, None)
        } else {
            let greater = indep_sums.iter().filter(|&&s| s > tot.sum_total).count();
            let less = indep_sums.iter().filter(|&&s| s < tot.sum_total).count();
            let equal = n_indep - greater - less;
            let pct = ((less as f64) + 0.5 * (equal as f64)) / (n_indep as f64);
            (Some(greater + 1), Some(pct))
        };

        candidate_outputs.push(CandidateScoreOutput {
            candidate_id: cand.candidate_id.clone(),
            ev_total_equity,
            in_span: cand.in_span,
            is_arm_independent: cand.is_arm_independent,
            mean_mandate_bonus,
            mean_net_asset_equity,
            mean_project_equity,
            paired_se_equity,
            regret_equity,
            score_norm,
            sensitivity_indep_percentile,
            sensitivity_indep_rank,
        });
    }

    let prescreen = compute_prescreen(request, config, &totals);
    let winners_curse = compute_winners_curse(request, &totals, half)?;

    Ok(StateScoreOutput {
        // Default for direct library callers; the `score` CLI overwrites this with `sha256_hex`
        // over the exact request file bytes read from `--blind-inputs/<state_id>.json`.
        blind_request_sha256: sha256_hex(to_sorted_pretty_json(request)?.as_bytes()),
        c_best,
        candidates: candidate_outputs,
        degenerate_flat_set,
        day,
        focal_seat: request.focal_seat,
        pre_act_pair_sha256: request.pre_act_pair_sha256.clone(),
        pre_act_save_sha256: request.pre_act_save_sha256.clone(),
        pre_act_seal_sha256: request.pre_act_seal_sha256.clone(),
        prescreen,
        scenarios_m: m_len,
        schema_version: BLIND_SCORING_SCHEMA_VERSION,
        scorer_config_sha256: request.scorer_config_sha256.clone(),
        scorer_seed: request.scorer_seed,
        state_id: request.state_id.clone(),
        talk_extraction_counts: request.talk_extraction_counts.clone(),
        v_max,
        v_min,
        winners_curse,
    })
}
