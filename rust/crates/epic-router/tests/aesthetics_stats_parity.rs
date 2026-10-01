//! The M8-T1 parity pins tying `epic_board::aesthetics::BoardTally` to
//! the REAL `BoardStatistics` counting walk (the measurer's tally is a
//! REPLICATION — the ctor lives in epic-router behind `&mut` + the
//! tree manager and is deliberately not moved), plus the DNR-18
//! reconciliation faces on real fixtures. Java-free; lives HERE (not
//! in epic-board) because `BoardStatistics` cannot be imported into
//! its own dependency.

use epic_board::board::Board;
use epic_board::tree_manager::SearchTreeManager;
use epic_dsn::reader::{DsnReadResult, read_board};

fn parse_bytes(bytes: &[u8]) -> Board {
    let mut ses = epic_dsn::ses_board::SesBoard::new();
    match read_board(bytes, &mut ses) {
        DsnReadResult::Success { .. } | DsnReadResult::OutlineMissing { .. } => {}
        other => panic!("expected a parseable DSN, got {other:?}"),
    }
    Board::from_ses_board(&ses)
}

fn parse_path(path: &str) -> Board {
    let bytes = std::fs::read(path).expect("fixture present");
    parse_bytes(&bytes)
}

fn assert_tally_equals_stats(board: &mut Board) {
    let metrics = epic_board::aesthetics::aesthetics_metrics(board, board.rules());
    let mut manager = SearchTreeManager::new();
    manager.reinsert_tree_items(board);
    let stats = epic_router::pipeline::board_statistics::BoardStatistics::new(&mut manager, board);
    let tally = &metrics.tally;
    assert_eq!(tally.trace_count, stats.traces.total_count, "trace count");
    // VALUE equality on the length faces (bit equality would false-fail
    // on the ±0.0 face: the real walk yields -0.0 on a trace-free board
    // — bits 2147483648 — where the replication yields +0.0; zero sign
    // carries no metric meaning).
    assert_eq!(
        tally.total_length, stats.traces.total_length,
        "total length f32"
    );
    assert_eq!(
        tally.total_length_mm, stats.traces.total_length_mm,
        "total length mm f32"
    );
    assert_eq!(tally.bend_total, stats.bends.total_count, "bend total");
    assert_eq!(
        tally.bend_ninety, stats.bends.ninety_degree_count,
        "90° bends"
    );
    assert_eq!(
        tally.bend_forty_five, stats.bends.forty_five_degree_count,
        "45° bends"
    );
    assert_eq!(
        tally.bend_other, stats.bends.other_angle_count,
        "other bends"
    );
    assert_eq!(tally.via_total, stats.vias.total_count, "via total");
    assert_eq!(
        tally.via_through, stats.vias.through_hole_count,
        "through vias"
    );
    assert_eq!(tally.via_blind, stats.vias.blind_count, "blind vias");
    assert_eq!(tally.via_buried, stats.vias.buried_count, "buried vias");
}

/// The crafted routed world (traces + a via + two nets): the
/// replication is field-equal to the real walk.
#[test]
fn t1_tally_equals_stats_crafted_world() {
    let dsn = r#"(pcb par.dsn
  (resolution um 1)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary (rect pcb 0 0 400000 300000))
    (rule (width 200) (clearance 200))
  )
  (placement
    (component CMP_A (place CMP_A 20000 30000 front 0))
    (component CMP_B (place CMP_B 120000 30000 front 0))
    (component CMP_C (place CMP_C 20000 120000 front 0))
    (component CMP_D (place CMP_D 120000 120000 front 0))
  )
  (library
    (padstack PAD_SMD (shape (circle F.Cu 600 0 0)))
    (padstack PAD_VIA
      (shape (circle F.Cu 800 0 0))
      (shape (circle B.Cu 800 0 0))
    )
    (image CMP_A (pin PAD_SMD P1 0 0))
    (image CMP_B (pin PAD_SMD P1 0 0))
    (image CMP_C (pin PAD_SMD P1 0 0))
    (image CMP_D (pin PAD_SMD P1 0 0))
  )
  (network
    (via V1 PAD_VIA default)
    (via_rule R1 V1)
    (net NET1 (pins CMP_A-P1 CMP_B-P1))
    (net NET2 (pins CMP_C-P1 CMP_D-P1))
  )
  (wiring
    (wire (path F.Cu 200 20000 30000 70000 30000 70000 75000)(net NET1)(type route))
    (wire (path B.Cu 200 70000 75000 120000 120000)(net NET1)(type route))
    (via PAD_VIA 70000 75000 (net NET1)(type route))
    (wire (path F.Cu 200 20000 120000 120000 120000)(net NET2)(type route))
  )
)
"#;
    let mut board = parse_bytes(dsn.as_bytes());
    assert_tally_equals_stats(&mut board);
}

/// bm08 (Tier A): the pins-only face — trace/via/bend faces all zero
/// and equal; the DNR-18 MST reconciliation: Σ per-net Euclidean MST ≤
/// Σ per-net Manhattan MST (the bounds calculator's total, converted
/// back from its mm face) — per-edge Euclidean ≤ Manhattan, same
/// terminal sets, same seed order.
#[test]
fn t1_tally_equals_stats_bm08_and_mst_bounds() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../scripts/benchmark/fixtures/DAC2020_boards/DAC2020_bm08.dsn"
    );
    let mut board = parse_path(path);
    assert_tally_equals_stats(&mut board);
    let metrics = epic_board::aesthetics::aesthetics_metrics(&board, board.rules());
    assert_eq!(metrics.nets_measured, 0, "pins-only board routes nothing");
    let mut manager = SearchTreeManager::new();
    manager.reinsert_tree_items(&mut board);
    let stats =
        epic_router::pipeline::board_statistics::BoardStatistics::new(&mut manager, &mut board);
    let bounds_mm = stats.bounds.min_trace_length_mm.expect("bm08 bounds set");
    assert!(bounds_mm > 0.0);
    // Σ euclid-MST (DBU) × mm-factor ≤ Σ manhattan-MST (the bounds mm
    // face, f32) + a f32-rounding tolerance.
    let communication = board.communication();
    let resolution = i64::from(communication.resolution);
    let resolution = if resolution > 0 { resolution } else { 1 };
    let mm_factor =
        epic_dsn::state::Unit::scale(1.0, communication.unit, epic_dsn::state::Unit::Mm)
            / resolution as f64;
    let euclid_mm = metrics.mst_lb_dbu * mm_factor;
    assert!(
        euclid_mm <= f64::from(bounds_mm) + 1e-3,
        "Σ euclid-MST {euclid_mm} mm must be ≤ the bounds Manhattan total {bounds_mm} mm"
    );
}

/// The 1Bitsy reference board (the real routed population): the
/// replication is field-equal on a fully routed professional board,
/// the DNR-18 routed-length reconciliation is exact-to-f64-accumulation
/// (Σ per-net vs the walk total within a relative epsilon), and the
/// same Euclidean ≤ Manhattan MST bound holds.
#[test]
fn t1_tally_equals_stats_1bitsy_reference() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../scripts/benchmark/fixtures/PCBench/1Bitsy_1bitsy/reference-routed.dsn"
    );
    let mut board = parse_path(path);
    assert_tally_equals_stats(&mut board);
    let metrics = epic_board::aesthetics::aesthetics_metrics(&board, board.rules());
    assert!(metrics.nets_measured > 0, "a routed board qualifies nets");
    // routed_length_dbu accumulates per net (then per net ascending);
    // total_length accumulates per trace ascending — different f64
    // association, so the reconciliation carries a relative tolerance
    // (measured drift on this board: 2.2e-8 — pure summation order).
    let total = f64::from(metrics.tally.total_length);
    let relative = (metrics.routed_length_dbu - total).abs() / total;
    assert!(
        relative < 1e-6,
        "Σ per-net routed {} vs board total {}: relative drift {relative}",
        metrics.routed_length_dbu,
        total
    );
    let mut manager = SearchTreeManager::new();
    manager.reinsert_tree_items(&mut board);
    let stats =
        epic_router::pipeline::board_statistics::BoardStatistics::new(&mut manager, &mut board);
    let bounds_mm = stats.bounds.min_trace_length_mm.expect("1Bitsy bounds set");
    let communication = board.communication();
    let resolution = i64::from(communication.resolution);
    let resolution = if resolution > 0 { resolution } else { 1 };
    let mm_factor =
        epic_dsn::state::Unit::scale(1.0, communication.unit, epic_dsn::state::Unit::Mm)
            / resolution as f64;
    let euclid_mm = metrics.mst_lb_dbu * mm_factor;
    assert!(
        euclid_mm <= f64::from(bounds_mm) + 1e-3,
        "Σ euclid-MST {euclid_mm} mm must be ≤ the bounds Manhattan total {bounds_mm} mm"
    );
}

/// The via-kind world (Q1-2): a 4-layer crafted board carrying one
/// through (F..B), TWO BLIND (F..In1), and one BURIED (In1..In2) via —
/// nonzero values in every ladder arm with blind ≠ buried (the swap
/// armor), field-equal against the real walk. A transposed arm pair
/// rotates the blind/buried fields here (fix2-mutation-viakinds.log).
#[test]
fn t1_tally_equals_stats_via_kinds_world() {
    let dsn = r#"(pcb viakinds.dsn
  (resolution um 1)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer In1.Cu (type signal))
    (layer In2.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary (rect pcb 0 0 400000 300000))
    (rule (width 200) (clearance 200))
  )
  (placement
    (component CMP_A (place CMP_A 20000 30000 front 0))
    (component CMP_B (place CMP_B 120000 30000 front 0))
  )
  (library
    (padstack PAD_SMD (shape (circle F.Cu 600 0 0)))
    (padstack PAD_TH
      (shape (circle F.Cu 800 0 0))
      (shape (circle In1.Cu 800 0 0))
      (shape (circle In2.Cu 800 0 0))
      (shape (circle B.Cu 800 0 0))
    )
    (padstack PAD_BL
      (shape (circle F.Cu 800 0 0))
      (shape (circle In1.Cu 800 0 0))
    )
    (padstack PAD_BU
      (shape (circle In1.Cu 800 0 0))
      (shape (circle In2.Cu 800 0 0))
    )
    (image CMP_A (pin PAD_SMD P1 0 0))
    (image CMP_B (pin PAD_SMD P1 0 0))
  )
  (network
    (via V_TH PAD_TH default)
    (via V_BL PAD_BL default)
    (via V_BU PAD_BU default)
    (via_rule R1 V_TH)
    (net NET1 (pins CMP_A-P1 CMP_B-P1))
  )
  (wiring
    (via PAD_TH 40000 30000 (net NET1)(type route))
    (via PAD_BL 70000 30000 (net NET1)(type route))
    (via PAD_BL 85000 30000 (net NET1)(type route))
    (via PAD_BU 100000 30000 (net NET1)(type route))
  )
)
"#;
    let mut board = parse_bytes(dsn.as_bytes());
    let metrics = epic_board::aesthetics::aesthetics_metrics(&board, board.rules());
    // blind (2) != buried (1) is LOAD-BEARING: a blind/buried swap with
    // equal counts is invisible (the first cut of this world had 1/1 and
    // the swap mutant SURVIVED it — fix2-mutation-viakinds.log round 2).
    assert_eq!(metrics.tally.via_total, 4);
    assert_eq!(
        metrics.tally.via_through, 1,
        "the F..B padstack spans the full stack: through"
    );
    assert_eq!(
        metrics.tally.via_blind, 2,
        "the two F..In1 padstacks are exposed at exactly one end: blind"
    );
    assert_eq!(
        metrics.tally.via_buried, 1,
        "the In1..In2 padstack is exposed at neither end: buried"
    );
    assert_tally_equals_stats(&mut board);
}
