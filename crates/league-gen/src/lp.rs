//! Self-certifying Two-Phase Simplex LP solver with Bland's anti-cycling rule.
//!
//! Solves standard inequality form:
//! `maximize c^T x` subject to `A x <= b, x >= 0`.
//! Every optimal solution is verified against primal feasibility, dual
//! feasibility, and strong duality (`|c^T x - b^T y| <= tol`) before returning.

use std::fmt;

const PIVOT_TOL: f64 = 1e-10;
const FEAS_TOL: f64 = 1e-6;
const DUAL_TOL: f64 = 1e-5;
const ZERO_CLAMP: f64 = 1e-13;
const MAX_PIVOTS: usize = 8_192;

/// Outcome of solving a linear program.
#[derive(Debug, Clone, PartialEq)]
pub enum LpStatus {
    /// Certified optimal solution with verified primal and dual witnesses.
    Optimal(LpSolution),
    /// Constraints `A x <= b, x >= 0` have no feasible solution.
    Infeasible,
    /// Objective can grow without bound on the feasible region.
    Unbounded,
}

/// Optimal primal and dual witness vectors returned by [`solve_lp`].
#[derive(Debug, Clone, PartialEq)]
pub struct LpSolution {
    /// Optimal objective value `c^T x == b^T y`.
    pub objective: f64,
    /// Primal decision vector `x >= 0`.
    pub primal: Vec<f64>,
    /// Dual shadow prices `y >= 0` associated with `A x <= b`.
    pub dual: Vec<f64>,
}

/// Numerical or input validation error raised by [`solve_lp`].
#[derive(Debug, Clone, PartialEq)]
pub struct LpError(pub String);

impl fmt::Display for LpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for LpError {}

/// Solves `maximize c^T x` subject to `A x <= b` and `x >= 0`.
pub fn solve_lp(a: &[Vec<f64>], b: &[f64], c: &[f64]) -> Result<LpStatus, LpError> {
    let m = b.len();
    let n = c.len();
    if a.len() != m {
        return Err(LpError(format!(
            "constraint row count {} does not match rhs length {}",
            a.len(),
            m
        )));
    }
    for (row_idx, row) in a.iter().enumerate() {
        if row.len() != n {
            return Err(LpError(format!(
                "constraint row {row_idx} width {} does not match objective length {}",
                row.len(),
                n
            )));
        }
        if row.iter().any(|v| !v.is_finite()) {
            return Err(LpError(format!(
                "constraint row {row_idx} contains non-finite value"
            )));
        }
    }
    if b.iter().any(|v| !v.is_finite()) || c.iter().any(|v| !v.is_finite()) {
        return Err(LpError(
            "objective or rhs contains non-finite value".to_owned(),
        ));
    }

    if n == 0 {
        if b.iter().any(|&rhs| rhs < -FEAS_TOL) {
            return Ok(LpStatus::Infeasible);
        }
        return Ok(LpStatus::Optimal(LpSolution {
            objective: 0.0,
            primal: Vec::new(),
            dual: vec![0.0; m],
        }));
    }

    if m == 0 {
        if c.iter().any(|&coeff| coeff > PIVOT_TOL) {
            return Ok(LpStatus::Unbounded);
        }
        return Ok(LpStatus::Optimal(LpSolution {
            objective: 0.0,
            primal: vec![0.0; n],
            dual: Vec::new(),
        }));
    }

    let min_b_idx = b
        .iter()
        .enumerate()
        .min_by(|(_, x), (_, y)| x.total_cmp(y))
        .map(|(idx, _)| idx)
        .unwrap_or(0);

    let total_vars = n + m;
    let rhs_col = total_vars;
    let stride = rhs_col + 1;

    let (mut tableau, mut basis) = if b[min_b_idx] < -PIVOT_TOL {
        run_phase_one(a, b, m, n, min_b_idx)?
    } else {
        let mut tab = vec![0.0; (m + 1) * stride];
        for i in 0..m {
            let row = i * stride;
            tab[row..row + n].copy_from_slice(&a[i]);
            tab[row + n + i] = 1.0;
            tab[row + rhs_col] = b[i].max(0.0);
        }
        let bas: Vec<usize> = (0..m).map(|i| n + i).collect();
        (tab, bas)
    };

    if tableau.is_empty() {
        return Ok(LpStatus::Infeasible);
    }

    // Build Phase II objective row from original `c` and the feasible basis.
    let obj_off = m * stride;
    tableau[obj_off..obj_off + stride].fill(0.0);
    for (j, &cj) in c.iter().enumerate() {
        tableau[obj_off + j] = -cj;
    }
    for (row_idx, &b_var) in basis.iter().enumerate() {
        if b_var < n {
            let weight = c[b_var];
            if weight.abs() > 0.0 {
                let row_off = row_idx * stride;
                for col in 0..stride {
                    tableau[obj_off + col] += weight * tableau[row_off + col];
                }
            }
        }
    }

    match simplex_iterate(&mut tableau, &mut basis, total_vars, m, rhs_col, stride)? {
        SimplexOutcome::Unbounded => return Ok(LpStatus::Unbounded),
        SimplexOutcome::Optimal => {}
    }

    let mut primal = vec![0.0; n];
    for (row_idx, &b_var) in basis.iter().enumerate() {
        if b_var < n {
            let val = tableau[row_idx * stride + rhs_col];
            primal[b_var] = if val.abs() <= ZERO_CLAMP { 0.0 } else { val };
        }
    }
    let mut dual = vec![0.0; m];
    for (i, d) in dual.iter_mut().enumerate() {
        let val = tableau[obj_off + n + i];
        *d = if val.abs() <= ZERO_CLAMP { 0.0 } else { val };
    }

    let primal_obj: f64 = c.iter().zip(primal.iter()).map(|(&ci, &xi)| ci * xi).sum();
    verify_kkt_certificate(a, b, c, &primal, &dual, primal_obj)?;

    Ok(LpStatus::Optimal(LpSolution {
        objective: primal_obj,
        primal,
        dual,
    }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SimplexOutcome {
    Optimal,
    Unbounded,
}

fn run_phase_one(
    a: &[Vec<f64>],
    b: &[f64],
    m: usize,
    n: usize,
    most_neg_row: usize,
) -> Result<(Vec<f64>, Vec<usize>), LpError> {
    let aux_col = n + m;
    let rhs_col = aux_col + 1;
    let p1_stride = rhs_col + 1;
    let mut tab = vec![0.0; (m + 1) * p1_stride];
    for i in 0..m {
        let row = i * p1_stride;
        tab[row..row + n].copy_from_slice(&a[i]);
        tab[row + n + i] = 1.0;
        tab[row + aux_col] = -1.0;
        tab[row + rhs_col] = b[i];
    }
    tab[m * p1_stride + aux_col] = 1.0;

    let mut basis: Vec<usize> = (0..m).map(|i| n + i).collect();
    pivot(&mut tab, &mut basis, most_neg_row, aux_col, m, p1_stride)?;

    match simplex_iterate(&mut tab, &mut basis, aux_col + 1, m, rhs_col, p1_stride)? {
        SimplexOutcome::Unbounded => {
            return Err(LpError(
                "Phase I auxiliary LP reported unbounded".to_owned(),
            ));
        }
        SimplexOutcome::Optimal => {}
    }

    if tab[m * p1_stride + rhs_col] < -FEAS_TOL {
        return Ok((Vec::new(), Vec::new()));
    }

    for row_idx in 0..m {
        if basis[row_idx] == aux_col {
            let row = row_idx * p1_stride;
            if tab[row + rhs_col].abs() > FEAS_TOL {
                return Ok((Vec::new(), Vec::new()));
            }
            if let Some((col, _)) = (0..aux_col)
                .filter_map(|j| {
                    let v = tab[row + j].abs();
                    if v > PIVOT_TOL { Some((j, v)) } else { None }
                })
                .max_by(|(_, lhs), (_, rhs)| lhs.total_cmp(rhs))
            {
                pivot(&mut tab, &mut basis, row_idx, col, m, p1_stride)?;
            } else {
                basis[row_idx] = usize::MAX;
            }
        }
    }

    let p2_stride = aux_col + 1;
    let mut reduced = vec![0.0; (m + 1) * p2_stride];
    for r in 0..=m {
        let src = r * p1_stride;
        let dst = r * p2_stride;
        reduced[dst..dst + aux_col].copy_from_slice(&tab[src..src + aux_col]);
        reduced[dst + aux_col] = tab[src + rhs_col];
    }

    Ok((reduced, basis))
}

fn simplex_iterate(
    tableau: &mut [f64],
    basis: &mut [usize],
    num_vars: usize,
    m: usize,
    rhs_col: usize,
    stride: usize,
) -> Result<SimplexOutcome, LpError> {
    let obj_off = m * stride;
    for _ in 0..MAX_PIVOTS {
        let entering = (0..num_vars).find(|&j| tableau[obj_off + j] < -PIVOT_TOL);
        let Some(enter_col) = entering else {
            return Ok(SimplexOutcome::Optimal);
        };

        let mut best: Option<(usize, f64, usize)> = None;
        for (row_idx, &leave_var) in basis.iter().enumerate().take(m) {
            let row_off = row_idx * stride;
            let coeff = tableau[row_off + enter_col];
            if coeff > PIVOT_TOL {
                let rhs = tableau[row_off + rhs_col].max(0.0);
                let ratio = rhs / coeff;
                let replace = match best {
                    None => true,
                    Some((_, best_ratio, best_var)) => {
                        ratio < best_ratio - 1e-12
                            || ((ratio - best_ratio).abs() <= 1e-12 && leave_var < best_var)
                    }
                };
                if replace {
                    best = Some((row_idx, ratio, leave_var));
                }
            }
        }

        let Some((leave_row, _, _)) = best else {
            return Ok(SimplexOutcome::Unbounded);
        };

        pivot(tableau, basis, leave_row, enter_col, m, stride)?;
    }

    Err(LpError(format!(
        "simplex exceeded iteration limit ({MAX_PIVOTS})"
    )))
}

fn pivot(
    tableau: &mut [f64],
    basis: &mut [usize],
    pivot_row: usize,
    pivot_col: usize,
    m: usize,
    stride: usize,
) -> Result<(), LpError> {
    let p_off = pivot_row * stride;
    let scale = tableau[p_off + pivot_col];
    if !scale.is_finite() || scale.abs() <= 1e-15 {
        return Err(LpError(format!(
            "degenerate pivot at ({pivot_row}, {pivot_col}) with value {scale}"
        )));
    }
    let inv_scale = 1.0 / scale;
    for cell in &mut tableau[p_off..p_off + stride] {
        *cell *= inv_scale;
        if cell.abs() <= ZERO_CLAMP {
            *cell = 0.0;
        }
    }
    tableau[p_off + pivot_col] = 1.0;

    for row_idx in 0..=m {
        if row_idx == pivot_row {
            continue;
        }
        let r_off = row_idx * stride;
        let factor = tableau[r_off + pivot_col];
        if factor == 0.0 {
            continue;
        }
        for col in 0..stride {
            let updated = tableau[r_off + col] - factor * tableau[p_off + col];
            tableau[r_off + col] = if updated.abs() <= ZERO_CLAMP {
                0.0
            } else {
                updated
            };
        }
        tableau[r_off + pivot_col] = 0.0;
    }

    basis[pivot_row] = pivot_col;
    Ok(())
}

fn verify_kkt_certificate(
    a: &[Vec<f64>],
    b: &[f64],
    c: &[f64],
    primal: &[f64],
    dual: &[f64],
    primal_obj: f64,
) -> Result<(), LpError> {
    for &x in primal {
        if !x.is_finite() || x < -FEAS_TOL {
            return Err(LpError(format!("primal non-negativity violated: {x}")));
        }
    }
    for (i, row) in a.iter().enumerate() {
        let lhs: f64 = row
            .iter()
            .zip(primal.iter())
            .map(|(&aij, &xj)| aij * xj)
            .sum();
        if lhs > b[i] + FEAS_TOL * (1.0 + b[i].abs()) {
            return Err(LpError(format!(
                "primal constraint {i} violated: {lhs} > {}",
                b[i]
            )));
        }
    }

    for &y in dual {
        if !y.is_finite() || y < -FEAS_TOL {
            return Err(LpError(format!("dual non-negativity violated: {y}")));
        }
    }
    for (j, &cj) in c.iter().enumerate() {
        let lhs: f64 = a
            .iter()
            .zip(dual.iter())
            .map(|(row, &yi)| row[j] * yi)
            .sum();
        if lhs < cj - DUAL_TOL * (1.0 + cj.abs()) {
            return Err(LpError(format!(
                "dual constraint {j} violated: {lhs} < {cj}"
            )));
        }
    }

    let dual_obj: f64 = b.iter().zip(dual.iter()).map(|(&bi, &yi)| bi * yi).sum();
    let gap = (primal_obj - dual_obj).abs();
    let gap_tol = DUAL_TOL * (1.0 + primal_obj.abs() + dual_obj.abs());
    if !gap.is_finite() || gap > gap_tol {
        return Err(LpError(format!(
            "strong duality gap {gap} exceeds tolerance {gap_tol} (primal={primal_obj}, dual={dual_obj})"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solves_basic_bounded_and_equality_lp() {
        let a = vec![vec![1.0, 0.0], vec![0.0, 2.0], vec![3.0, 2.0]];
        let b = vec![4.0, 12.0, 18.0];
        let c = vec![3.0, 5.0];
        let Ok(LpStatus::Optimal(sol)) = solve_lp(&a, &b, &c) else {
            panic!("expected optimal solution");
        };
        assert!((sol.objective - 36.0).abs() < 1e-6);
        assert!((sol.primal[0] - 2.0).abs() < 1e-6);
        assert!((sol.primal[1] - 6.0).abs() < 1e-6);
    }

    #[test]
    fn handles_phase_one_negative_rhs_and_infeasibility() {
        let a = vec![vec![-1.0, -1.0], vec![1.0, 0.0], vec![0.0, 1.0]];
        let b = vec![-3.0, 2.0, 4.0];
        let c = vec![1.0, 2.0];
        let Ok(LpStatus::Optimal(sol)) = solve_lp(&a, &b, &c) else {
            panic!("expected optimal solution");
        };
        assert!((sol.objective - 10.0).abs() < 1e-6);

        let a_inf = vec![vec![1.0, 1.0], vec![-1.0, -1.0]];
        let b_inf = vec![1.0, -3.0];
        let c_inf = vec![1.0, 1.0];
        assert_eq!(solve_lp(&a_inf, &b_inf, &c_inf), Ok(LpStatus::Infeasible));
    }

    #[test]
    fn handles_unbounded_zero_dimensions_and_redundant_rows() {
        let a_unb = vec![vec![-1.0, 1.0]];
        let b_unb = vec![0.0];
        let c_unb = vec![1.0, -1.0];
        assert_eq!(solve_lp(&a_unb, &b_unb, &c_unb), Ok(LpStatus::Unbounded));

        assert!(matches!(solve_lp(&[], &[], &[]), Ok(LpStatus::Optimal(_))));
        assert_eq!(solve_lp(&[vec![]], &[-1.0], &[]), Ok(LpStatus::Infeasible));

        assert_eq!(solve_lp(&[], &[], &[1.0]), Ok(LpStatus::Unbounded));
        assert!(matches!(
            solve_lp(&[], &[], &[-2.0]),
            Ok(LpStatus::Optimal(_))
        ));

        let a_eq = vec![vec![1.0, 1.0], vec![-1.0, -1.0], vec![-1.0, -1.0]];
        let b_eq = vec![2.0, -2.0, -2.0];
        let c_eq = vec![1.0, 3.0];
        let Ok(LpStatus::Optimal(sol)) = solve_lp(&a_eq, &b_eq, &c_eq) else {
            panic!("redundant equality LP should solve cleanly");
        };
        assert!((sol.objective - 6.0).abs() < 1e-6);
    }

    #[test]
    fn validates_dimensions_and_finiteness() {
        assert!(solve_lp(&[], &[1.0], &[1.0]).is_err());
        assert!(solve_lp(&[vec![1.0]], &[1.0], &[1.0, 2.0]).is_err());
        assert!(solve_lp(&[vec![f64::NAN]], &[1.0], &[1.0]).is_err());
        let err = LpError("bad".to_owned());
        assert_eq!(err.to_string(), "bad");
    }
}
