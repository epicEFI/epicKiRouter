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

// ===========================================================================
// The integration half (F1): the board-mutating apply face.
// ===========================================================================

use std::collections::BTreeMap;

use epic_board::board::Board;
use epic_board::id::ItemId;
use epic_board::items::{FixedState, ItemData};
use epic_board::tree_manager::SearchTreeManager;
use epic_geometry::point::Point;

/// One pin's re-netting, for the report (the CLI manifest rows and
/// the session telemetry render these verbatim).
#[derive(Debug, Clone, PartialEq)]
pub struct PinSwapRow {
    /// The component REF the caller named.
    pub component: String,
    /// The pin's NAME in its package (`#<index>` fallback when the
    /// package pin cannot be resolved).
    pub pin: String,
    /// The net number the pin carried at entry (1-based into the
    /// append-only table).
    pub old_net_number: i32,
    pub old_net_name: String,
    pub new_net_number: i32,
    pub new_net_name: String,
}

/// The apply face's answer: the CHANGED rows only (a pin whose
/// optimal assignment equals its parsed net is untouched and
/// unreported) plus the refs that could not be honored, as
/// human-readable reasons (unknown ref, fewer than 2 interchangeable
/// pins).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PinAssignReport {
    /// The performed re-nettings, in candidate (ascending-id) order.
    pub rows: Vec<PinSwapRow>,
    /// Unhonored refs with reasons — never fatal, never guessed.
    pub unresolved: Vec<String>,
}

/// The net NAME (`#number` fallback for a number outside the table —
/// unreachable from a parsed board, whose item nets came from the
/// same append-only table).
fn net_name(board: &Board, net_number: i32) -> String {
    board
        .rules()
        .nets
        .get(net_number)
        .map_or_else(|| format!("#{net_number}"), |net| net.name.clone())
}

/// A drill center as plain f64 coordinates (pins live on the integer
/// lattice; the Rational arm is unreachable for drill centers).
fn point_xy(p: Point) -> (f64, f64) {
    match p {
        Point::Int(ip) => (ip.x as f64, ip.y as f64),
        Point::Rational(_) => (0.0, 0.0),
    }
}

/// One interchangeable pin of the named component — the snapshot the
/// cost matrix is built from (owned, so the board borrow ends before
/// any mutation).
struct Candidate {
    id: ItemId,
    pin_name: String,
    old_net: i32,
    /// The drill center (f64 pair).
    center: (f64, f64),
}

/// F1: for each named component REF, re-net the component's
/// INTERCHANGEABLE pins to the min-total-cost assignment
/// ([`hungarian`]), Tyler's ask: "assign the lower pins and route to
/// make the neatest routing" — the router chooses the net->pin
/// mapping that minimizes crossing/length instead of inheriting the
/// schematic's order.
///
/// Semantics:
///
/// * Candidates: the component's ON-BOARD pin items with exactly ONE
///   net and `FixedState::Unfixed` — the DSN parse's own
///   interchangeability declaration (parsed pins are Unfixed
///   (`Pin.fixedState`, network scope); a `lock_type` placement is
///   SystemFixed and a GUI-locked pin is UserFixed — both excluded).
/// * The column multiset: the candidates' CURRENT nets — square by
///   construction; the assignment PERMUTES the multiset, so every net
///   keeps its pin count and no connectivity is invented or dropped.
/// * `cost[pin][net]`: the mean Euclidean distance from the pin's
///   drill center to the net's OTHER pin centers, EXCLUDING every pin
///   of this component (the in-component partners are the ones being
///   permuted — including them would charge a pin against its own
///   permutation). A net with no foreign pins costs 0.0 in EVERY
///   column — a constant row, invisible to the argmin.
/// * Fewer than 2 candidates leaves the component untouched (nothing
///   to optimize) and reports the reason.
/// * Changed pins mutate through the canonical pre-route recipe (the
///   `apply_copper_to_edge_clearance_override` choreography): tree
///   remove -> `set_item_nets` -> `clear_derived_data` -> tree insert.
///
/// The caller runs this BEFORE any routing pass (the route head /
/// CLI pre-route stage); the mutated netlist then routes as if the
/// schematic had declared the assignment.
pub fn apply_pin_assignments(
    board: &mut Board,
    manager: &mut SearchTreeManager,
    refs: &[String],
) -> PinAssignReport {
    let mut report = PinAssignReport::default();
    for component_ref in refs {
        let Some((component_id, component)) = board.components().get_by_name(component_ref) else {
            report
                .unresolved
                .push(format!("{component_ref}: no such component"));
            continue;
        };
        let package = board.library().package(component.package_no());
        let candidates: Vec<Candidate> = board
            .iter_ascending()
            .filter(|entry| entry.on_the_board)
            .filter(|entry| u32::try_from(entry.component_id).ok() == Some(component_id))
            .filter(|entry| matches!(entry.data, ItemData::Pin { .. }))
            .filter(|entry| entry.nets.len() == 1 && entry.fixed == FixedState::Unfixed)
            .filter_map(|entry| {
                let ItemData::Pin { pin_index, .. } = &entry.data else {
                    return None;
                };
                let pin_name = package
                    .and_then(|pkg| pkg.get_pin(*pin_index))
                    .map_or_else(|| format!("#{pin_index}"), |pin| pin.name.clone());
                let center = point_xy(board.drill_center(entry.id)?);
                Some(Candidate {
                    id: entry.id,
                    pin_name,
                    old_net: entry.nets[0],
                    center,
                })
            })
            .collect();
        if candidates.len() < 2 {
            report.unresolved.push(format!(
                "{component_ref}: fewer than 2 interchangeable pins (single-net, unfixed)"
            ));
            continue;
        }
        // The foreign target centers per net — every pin OUTSIDE this
        // component, keyed by each net it carries.
        let mut targets: BTreeMap<i32, Vec<(f64, f64)>> = BTreeMap::new();
        for entry in board.iter_ascending() {
            if !matches!(entry.data, ItemData::Pin { .. })
                || u32::try_from(entry.component_id).ok() == Some(component_id)
            {
                continue;
            }
            let Some(center) = board.drill_center(entry.id).map(point_xy) else {
                continue;
            };
            for &net in &entry.nets {
                targets.entry(net).or_default().push(center);
            }
        }
        // cost[row][col]: candidate row vs the net of candidate col
        // (the square multiset matrix).
        let columns: Vec<i32> = candidates.iter().map(|c| c.old_net).collect();
        let cost: Vec<Vec<f64>> = candidates
            .iter()
            .map(|candidate| {
                columns
                    .iter()
                    .map(
                        |&net| match targets.get(&net).filter(|list| !list.is_empty()) {
                            None => 0.0,
                            Some(list) => {
                                let sum: f64 = list
                                    .iter()
                                    .map(|&(tx, ty)| {
                                        let (dx, dy) =
                                            (candidate.center.0 - tx, candidate.center.1 - ty);
                                        (dx * dx + dy * dy).sqrt()
                                    })
                                    .sum();
                                sum / list.len() as f64
                            }
                        },
                    )
                    .collect()
            })
            .collect();
        let solved = hungarian(&cost);
        for (row, candidate) in candidates.iter().enumerate() {
            let new_net = columns[solved.assignment[row]];
            if new_net == candidate.old_net {
                continue;
            }
            let swap = PinSwapRow {
                component: component_ref.clone(),
                pin: candidate.pin_name.clone(),
                old_net_number: candidate.old_net,
                old_net_name: net_name(board, candidate.old_net),
                new_net_number: new_net,
                new_net_name: net_name(board, new_net),
            };
            manager.remove(board, candidate.id);
            board.set_item_nets(candidate.id, vec![new_net]);
            board.clear_derived_data(candidate.id);
            manager.insert(board, candidate.id);
            report.rows.push(swap);
        }
    }
    report
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

/// The F1 integration tests: the apply face on a CRAFT DSN (the
/// g4_fanout.dsn template's grammar) — a 4-pin connector column at
/// the left, four single-pin targets at the right in REVERSED
/// vertical order, so the straight-through netlist is four crossing
/// diagonals and the optimum is the horizontal reversal.
#[cfg(test)]
mod assign_tests {
    use super::*;
    use epic_dsn::reader::{DsnReadResult, read_board};
    use epic_dsn::ses_board::SesBoard;

    /// Connector CONN1: pins CA1..CA4 top-to-bottom at x=20000,
    /// y = 64000..40000 (step -8000). Targets T4..T1 at x=100000 with
    /// y = 64000..40000 — T4 TOP, T1 BOTTOM. Nets N1..N4 pair each
    /// pin with the target at the OPPOSITE height (N1 = CA1-T1: the
    /// top pin to the bottom target): the parsed netlist is fully
    /// crossed, and the unique min-cost assignment is the reversal
    /// (every term of `sqrt(80000^2 + dy^2)` is minimized at dy=0,
    /// which only the reversal achieves for all four rows).
    const CROSSED_CONN_DSN: &str = r#"(pcb conn-uncross.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal)(property(index 0)))
    (layer B.Cu (type signal)(property(index 1)))
    (boundary (path pcb 0  0 0  128000 0  128000 128000  0 128000  0 0))
    (rule (clearance 250))
  )
  (placement
    (component "CONN" (place "CONN1" 20000 64000 Front 0.000000))
    (component "TGT" (place "T4" 100000 64000 Front 0.000000))
    (component "TGT" (place "T3" 100000 56000 Front 0.000000))
    (component "TGT" (place "T2" 100000 48000 Front 0.000000))
    (component "TGT" (place "T1" 100000 40000 Front 0.000000))
  )
  (library
    (image "CONN"
      (pin "PAD" "CA1" 0 0)
      (pin "PAD" "CA2" 0 -8000)
      (pin "PAD" "CA3" 0 -16000)
      (pin "PAD" "CA4" 0 -24000)
    )
    (image "TGT"
      (pin "PAD" "TA" 0 0)
    )
    (padstack "PAD"
      (shape (circle F.Cu 2000))
      (attach off)
    )
  )
  (network
    (net "N1" (pins "CONN1"-"CA1" "T1"-"TA"))
    (net "N2" (pins "CONN1"-"CA2" "T2"-"TA"))
    (net "N3" (pins "CONN1"-"CA3" "T3"-"TA"))
    (net "N4" (pins "CONN1"-"CA4" "T4"-"TA"))
    (class kicad_default "N1" "N2" "N3" "N4"
      (rule (clearance 250)(width 2000))
    )
  )
)
"#;

    /// Parses the craft DSN into a live board + filled tree manager
    /// (the `Session::load_dsn` recipe's first half).
    fn crossed_board() -> (Board, SearchTreeManager) {
        let mut ses = SesBoard::new();
        match read_board(CROSSED_CONN_DSN.as_bytes(), &mut ses) {
            DsnReadResult::Success { warnings } => {
                assert!(warnings.is_empty(), "WARN_COUNT 0, got {warnings:?}");
            }
            other => panic!("expected Success, got {other:?}"),
        }
        let mut board = Board::from_ses_board(&ses);
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        (board, manager)
    }

    /// The net NUMBER for a net NAME (nets are 1-based append-only;
    /// the test resolves by name, never by assumed number).
    fn net_no(board: &Board, name: &str) -> i32 {
        board
            .rules()
            .nets
            .iter()
            .find(|(_, net)| net.name == name)
            .unwrap_or_else(|| panic!("net {name} exists"))
            .0
    }

    /// CONN1's pin nets TOP-TO-BOTTOM (by drill-center y, descending).
    fn conn_pin_nets_top_to_bottom(board: &Board) -> Vec<i32> {
        let conn_id = board
            .components()
            .get_by_name("CONN1")
            .expect("CONN1 exists")
            .0;
        let mut pins: Vec<(f64, i32)> = board
            .iter_ascending()
            .filter(|entry| u32::try_from(entry.component_id).ok() == Some(conn_id))
            .filter(|entry| matches!(entry.data, ItemData::Pin { .. }))
            .map(|entry| {
                let center = point_xy(board.drill_center(entry.id).expect("pin center"));
                (center.1, entry.nets[0])
            })
            .collect();
        pins.sort_by(|a, b| b.0.partial_cmp(&a.0).expect("finite pin centers"));
        pins.into_iter().map(|(_, net)| net).collect()
    }

    /// THE F1 PIN: the fully-crossed straight-through netlist
    /// uncrosses into horizontals — every pin takes the net whose
    /// foreign target sits at ITS OWN height, the board's arena
    /// carries the new nets (not just the report), and the rows
    /// report all four swaps with names both sides.
    #[test]
    fn crossed_connector_uncrosses_into_horizontals() {
        let (mut board, mut manager) = crossed_board();
        let n1 = net_no(&board, "N1");
        let n2 = net_no(&board, "N2");
        let n3 = net_no(&board, "N3");
        let n4 = net_no(&board, "N4");
        // Sanity: the parsed netlist is the crossed one (top pin
        // carries the BOTTOM target's net).
        assert_eq!(
            conn_pin_nets_top_to_bottom(&board),
            vec![n1, n2, n3, n4],
            "parsed order: CA1..CA4 top-to-bottom"
        );

        let report =
            apply_pin_assignments(&mut board, &mut manager, ["CONN1".to_string()].as_slice());

        assert!(report.unresolved.is_empty(), "{:?}", report.unresolved);
        assert_eq!(report.rows.len(), 4, "every pin re-netted");
        // The arena carries the reversal: top-to-bottom N4..N1.
        assert_eq!(
            conn_pin_nets_top_to_bottom(&board),
            vec![n4, n3, n2, n1],
            "the unique min-cost assignment is the horizontal reversal"
        );
        // The rows carry NAMES on both sides: the top pin CA1 swaps
        // N1 -> N4 (the top target T4's net).
        let ca1 = report
            .rows
            .iter()
            .find(|row| row.pin == "CA1")
            .expect("CA1 row");
        assert_eq!(ca1.component, "CONN1");
        assert_eq!(ca1.old_net_number, n1);
        assert_eq!(ca1.old_net_name, "N1");
        assert_eq!(ca1.new_net_number, n4);
        assert_eq!(ca1.new_net_name, "N4");
        // The multiset is preserved: {N1..N4} both sides.
        let mut old_sorted: Vec<i32> = report.rows.iter().map(|r| r.old_net_number).collect();
        let mut new_sorted: Vec<i32> = report.rows.iter().map(|r| r.new_net_number).collect();
        old_sorted.sort_unstable();
        new_sorted.sort_unstable();
        assert_eq!(old_sorted, new_sorted, "a permutation, never a net change");
    }

    /// The off face: NO refs leaves every item byte-identical (empty
    /// report, unchanged arena) — the default path never fires.
    #[test]
    fn no_refs_leaves_the_board_unchanged() {
        let (mut board, mut manager) = crossed_board();
        let before = conn_pin_nets_top_to_bottom(&board);
        let before_count = board.item_count();

        let report = apply_pin_assignments(&mut board, &mut manager, &[]);

        assert!(report.rows.is_empty());
        assert!(report.unresolved.is_empty());
        assert_eq!(conn_pin_nets_top_to_bottom(&board), before);
        assert_eq!(board.item_count(), before_count);
    }

    /// Unknown refs and non-interchangeable components are REPORTED,
    /// never fatal, never guessed: an unknown name lands in
    /// `unresolved`, and T1 (a REAL single-pin component) lands there
    /// too — fewer than 2 interchangeable pins leaves it untouched.
    #[test]
    fn unknown_and_single_pin_refs_are_reported_not_fatal() {
        let (mut board, mut manager) = crossed_board();
        let before = conn_pin_nets_top_to_bottom(&board);
        let refs = ["NOPE1".to_string(), "T1".to_string()];

        let report = apply_pin_assignments(&mut board, &mut manager, &refs);

        assert_eq!(report.unresolved.len(), 2, "{:?}", report.unresolved);
        assert!(report.unresolved[0].contains("NOPE1"));
        assert!(report.unresolved[1].contains("T1"));
        assert!(report.rows.is_empty());
        assert_eq!(conn_pin_nets_top_to_bottom(&board), before);
    }
}
