//! The F1 pin auto-assignment core: a self-contained min-cost
//! bipartite assignment solver (the Hungarian algorithm, Jonker-
//! Volgenant potentials form, O(k^3)) plus the pure cost-model
//! types. Tyler's ask (2026-10-02, connector board screenshot):
//! "these blue areas are fixed pins — assign the lower pins and
//! route to make the neatest routing" — for a connector's
//! interchangeable pins, the ROUTER chooses the net->pin mapping
//! that minimizes crossing/length, instead of inheriting whatever
//! order the schematic happened to leave.
//!
//! Module law (the shell.rs census-pinnability idiom): every pure
//! face the census pins lives here ungated; the board-mutating
//! integration (cost-matrix building from live pins, the apply face)
//! composes these and is tested through the engine/session faces.
//!
//! This file: the solver half. The cost model is plain data —
//! `cost[pin_index][net_index]` as FINITE f64 board-units (DBU);
//! there is deliberately NO infinity/NaN handling in the solver
//! (subtraction on non-finite values is not a total arithmetic —
//! `inf - inf` poisons the potentials): a pair the caller wants to
//! FORBID is encoded as a large finite penalty constant at the
//! cost-builder, never `f64::INFINITY` here.

/// The solver's answer for a k x k cost matrix: `assignment[row] =
/// col` (a permutation of `0..k`) and the TOTAL cost RECOMPUTED from
/// the matrix by summation over the returned permutation (never the
/// accumulated potential drift — the float sum of the chosen cells
/// is the honest face).
#[derive(Debug, Clone, PartialEq)]
pub struct Assignment {
    /// `assignment[row] = col` — which column each row is matched to
    /// (a permutation of `0..k`).
    pub assignment: Vec<usize>,
    /// The total cost: `sum(cost[r][assignment[r]])` recomputed from
    /// the input matrix.
    pub total_cost: f64,
}

/// Min-cost perfect bipartite matching on a SQUARE cost matrix
/// (Hungarian / Kuhn-Munkres, the Jonker-Volgenant potentials form —
/// O(k^3) time, O(k) extra space per row). Rows and columns are
/// symmetric to the caller; the pin-assignment use reads rows as
/// PINS and columns as NETS (the cost-builder pads to square with a
/// constant — documented there).
///
/// Contract:
///
/// * `cost` is k x k with FINITE entries (NaN/inf are a caller bug;
///   the potentials arithmetic is only total on finite values — see
///   the module docs for the forbidden-pair encoding).
/// * `k == 0` returns the empty assignment at total 0.0 (the
///   nothing-to-assign face — a component with no interchangeable
///   pins).
/// * The result is a PERMUTATION (each column used exactly once) and
///   the total is MINIMAL among all permutations.
///
/// Implementation note: the classic 1-indexed e-maxx port with the
/// sentinel row/column 0 (`p[0]`, `way` backpointers). Correctness
/// is pinned two ways: the hand-checked 3x3 optimum and a
/// brute-force cross-check over every permutation for k <= 6 (720
/// permutations — an optimality mutant cannot survive that pin).
#[must_use]
pub fn hungarian(cost: &[Vec<f64>]) -> Assignment {
    let k = cost.len();
    if k == 0 {
        return Assignment {
            assignment: Vec::new(),
            total_cost: 0.0,
        };
    }
    // The 1-indexed matrix (row/col 0 is the sentinel).
    let a = |i: usize, j: usize| cost[i - 1][j - 1];
    let inf = f64::INFINITY;
    // u/v: the row/column potentials; p[j]: the ROW matched to column
    // j (p[0] carries the current row during the augmenting search).
    let mut u = vec![0.0_f64; k + 1];
    let mut v = vec![0.0_f64; k + 1];
    let mut p = vec![0_usize; k + 1];
    let mut way = vec![0_usize; k + 1];
    for i in 1..=k {
        p[0] = i;
        let mut j0 = 0_usize;
        let mut minv = vec![inf; k + 1];
        let mut used = vec![false; k + 1];
        loop {
            used[j0] = true;
            let i0 = p[j0];
            let mut delta = inf;
            let mut j1 = usize::MAX;
            for j in 1..=k {
                if used[j] {
                    continue;
                }
                let cur = a(i0, j) - u[i0] - v[j];
                if cur < minv[j] {
                    minv[j] = cur;
                    way[j] = j0;
                }
                if minv[j] < delta {
                    delta = minv[j];
                    j1 = j;
                }
            }
            // Grow the tree by the tightest column and shift the
            // potentials so every tree edge stays tight.
            for j in 0..=k {
                if used[j] {
                    u[p[j]] += delta;
                    v[j] -= delta;
                } else {
                    minv[j] -= delta;
                }
            }
            j0 = j1;
            if p[j0] == 0 {
                break; // reached an unmatched column — augment
            }
        }
        // Walk the way[] backpointers to flip the augmenting path.
        loop {
            let j1 = way[j0];
            p[j0] = p[j1];
            j0 = j1;
            if j0 == 0 {
                break;
            }
        }
    }
    // p[j] = row matched to column j (0-indexed rows/cols outside).
    let mut assignment = vec![0_usize; k];
    for j in 1..=k {
        assignment[p[j] - 1] = j - 1;
    }
    let total_cost: f64 = (0..k).map(|r| cost[r][assignment[r]]).sum();
    Assignment {
        assignment,
        total_cost,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic pseudo-random f64 in (0, 1000) — an LCG (the
    /// pins must be reproducible; no external rand dep in the engine).
    fn lcg(state: &mut u64) -> f64 {
        *state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((*state >> 11) % 100_000) as f64 / 100.0
    }

    /// Brute-force minimum over ALL k! permutations (k <= 6 — 720
    /// rows; the optimality oracle for the pins).
    fn brute_force_min(cost: &[Vec<f64>]) -> f64 {
        let k = cost.len();
        let mut perm: Vec<usize> = (0..k).collect();
        let mut best = f64::INFINITY;
        loop {
            let total: f64 = (0..k).map(|r| cost[r][perm[r]]).sum();
            if total < best {
                best = total;
            }
            if !next_permutation(&mut perm) {
                break;
            }
        }
        best
    }

    fn next_permutation(perm: &mut [usize]) -> bool {
        let n = perm.len();
        let mut i = n;
        loop {
            if i <= 1 {
                return false;
            }
            i -= 1;
            if perm[i - 1] < perm[i] {
                break;
            }
        }
        let mut j = n;
        while perm[j - 1] <= perm[i - 1] {
            j -= 1;
        }
        perm.swap(i - 1, j - 1);
        perm[i..].reverse();
        true
    }

    /// THE OPTIMUM PIN, hand-checked face: the classic 3x3 with
    /// optimum 5.0 (two optimal permutations exist; the pin asserts
    /// the TOTAL and the permutation PROPERTY — kills the
    /// off-by-one mapping mutant and the accumulated-total mutant).
    #[test]
    fn hungarian_finds_the_classic_3x3_optimum() {
        let cost = vec![
            vec![4.0, 1.0, 3.0],
            vec![2.0, 0.0, 5.0],
            vec![3.0, 2.0, 2.0],
        ];
        let Assignment {
            assignment,
            total_cost,
        } = hungarian(&cost);
        // The permutation property: every column used exactly once.
        let mut seen = [false; 3];
        for &col in &assignment {
            assert!(col < 3);
            assert!(!seen[col], "column {col} used twice: {assignment:?}");
            seen[col] = true;
        }
        // The hand-derived optimum (r0c1 + r1c0 + r2c2 = 1+2+2, or
        // r0c2 + r1c0 + r2c1 = 3+0+2 — both 5.0).
        assert!((total_cost - 5.0).abs() < 1e-9, "got {total_cost}");
        // The total is the recomputed sum of the CHOSEN cells (never
        // a potential-drift carry).
        let sum: f64 = (0..3).map(|r| cost[r][assignment[r]]).sum();
        assert!((sum - total_cost).abs() < 1e-12);
    }

    /// THE OPTIMALITY PIN, brute-force cross-check: on deterministic
    /// pseudo-random matrices (k = 2..=6, 3 seeds each) the solver's
    /// total equals the exhaustive k! minimum to float tolerance AND
    /// the result is a permutation. An optimality mutant cannot
    /// survive 15 exhaustive oracle comparisons.
    #[test]
    fn hungarian_matches_brute_force_on_random_matrices() {
        for k in 2..=6 {
            for seed in [1_u64, 42, 9_181] {
                let mut state = seed;
                let cost: Vec<Vec<f64>> = (0..k)
                    .map(|_| (0..k).map(|_| lcg(&mut state)).collect())
                    .collect();
                let Assignment {
                    assignment,
                    total_cost,
                } = hungarian(&cost);
                let mut seen = vec![false; k];
                for &col in &assignment {
                    assert!(!seen[col], "k={k} seed={seed} dup col: {assignment:?}");
                    seen[col] = true;
                }
                let brute = brute_force_min(&cost);
                assert!(
                    (total_cost - brute).abs() < 1e-6,
                    "k={k} seed={seed}: solver {total_cost} vs brute {brute}"
                );
            }
        }
    }

    /// The structure pins: identity on a zero diagonal (the trivial
    /// optimum — total exactly 0.0), and a large off-diagonal block
    /// cannot pull the solver off the diagonal.
    #[test]
    fn hungarian_identity_face() {
        let k = 4;
        let cost: Vec<Vec<f64>> = (0..k)
            .map(|r| {
                (0..k)
                    .map(|c| if r == c { 0.0 } else { 10_000.0 })
                    .collect()
            })
            .collect();
        let Assignment {
            assignment,
            total_cost,
        } = hungarian(&cost);
        assert_eq!(assignment, vec![0, 1, 2, 3]);
        assert_eq!(total_cost, 0.0);
    }

    /// The degenerate faces: k=0 (the nothing-to-assign face — a
    /// component with no interchangeable pins) and k=1 (the forced
    /// assignment). Both are total — never a panic.
    #[test]
    fn hungarian_empty_and_single_faces() {
        let empty = hungarian(&[]);
        assert!(empty.assignment.is_empty());
        assert_eq!(empty.total_cost, 0.0);
        let single = hungarian(&[vec![7.5]]);
        assert_eq!(single.assignment, vec![0]);
        assert!((single.total_cost - 7.5).abs() < 1e-12);
    }

    /// The forbidden-pair encoding face: a large FINITE penalty
    /// steers the optimum away from the penalized cells unless every
    /// perfect matching needs one (the forced face — the penalty is
    /// advisory cost, never an infeasibility; see the module docs
    /// for why the solver refuses inf/NaN inputs).
    #[test]
    fn hungarian_penalty_encoding_face() {
        // 2x2: the cheap diagonal except one penalized cell — the
        // optimum crosses to avoid the penalty.
        let penalty = 1.0e12_f64;
        let cost = vec![vec![1.0, 2.0], vec![penalty, 1.0]];
        let Assignment {
            assignment,
            total_cost,
        } = hungarian(&cost);
        // r0c1 + r1c0 = 2.0 + 1.0e12? NO — r0c1(2.0) + r1c0(penalty)
        // vs r0c0(1.0)+r1c1(1.0)=2.0: the identity wins.
        assert_eq!(assignment, vec![0, 1]);
        assert!((total_cost - 2.0).abs() < 1e-6);
        // The FORCED face: both cheap cells share a row, so one
        // penalty must be taken — the smaller total keeps the 1.0.
        let forced = vec![vec![1.0, 3.0], vec![1.0, penalty]];
        let Assignment { assignment, .. } = hungarian(&forced);
        // r0c1=3.0 + r1c0=1.0 (total 4.0) beats r0c0=1.0 +
        // r1c1=penalty — the assignment must be the crossed one.
        assert_eq!(assignment, vec![1, 0]);
    }
}
