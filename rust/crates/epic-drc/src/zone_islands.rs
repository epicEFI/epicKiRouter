//! The 152-G zone-island DRC face (upstream `3011e6e60` +
//! `DesignRulesChecker` summary wiring at the review-feedback tip):
//! pour fragmentation reported as DRC violations.
//!
//! Upstream decomposes each conduction area's detailed fill into
//! islands, maps same-net connectable items onto them by center
//! containment, and reports a non-primary island whose items cannot
//! reach the primary's connected set as `isolated_island_unconnected`
//! (error) and a zero-item island whose bbox area is at least
//! 1 mm² as `isolated_island_dead_copper` (warning).
//!
//! This port builds on the M6-T6 detector ([`epic_board::islands`]),
//! which partitions the pour's metal lattice exactly (foreign
//! same-layer disjoint-net copper carved BEFORE partitioning —
//! stronger than upstream's drawn fill) and already flags every
//! region without same-net seed copper as floating. The mapping:
//!
//! * floating region ≡ upstream zero-ITEMS island → the
//!   [`ZoneIslandKind::DeadCopper`] arm, ported faithfully: bbox
//!   area ≥ `1.0 × resolution(MM)²` board units², kind string, and
//!   the area-in-mm² reporting sentence.
//! * [`epic_board::islands::PourIslands::region_count`] ≤ 1 mirrors
//!   upstream's `islands.size() <= 1` fragmentation gate.
//!
//! FACE NOTE (documented divergence): upstream's
//! `isolated_island_unconnected` ERROR class needs Java's per-item
//! `Item.getConnectedSet(netNumber, true)` walk — whether an
//! island's items reach the primary island through traces/vias
//! elsewhere on the board. The M6-T6 detector has no such walk: its
//! seeds are the connectivity proxy (any same-net pin/via/trace
//! copper overlapping a region counts as connected), so a seeded
//! island never reports here. Only the dead-copper class is
//! portable at this scope; the unconnected class would need the
//! connectivity graph (an M-sized face, out of the S add-on's
//! scope per the intake classification).
//!
//! Area convention: upstream reads the CONTINUOUS bbox
//! (`getBounds2D` width × height) of the AWT island; this port
//! reads the INCLUSIVE lattice bbox ((x1−x0+1) · (y1−y0+1) unit
//! cells — each cell a unit square, so the inclusive dims are the
//! region's geometric extent under the lattice convention). The two
//! differ by at most one cell row/column at the boundary edges —
//! immaterial except exactly at the 1 mm² threshold, where the
//! boundary fixture pins this port's convention.

use epic_board::board::{Board, BoardCommunication};
use epic_board::islands::PourIslands;

/// The violation kind (upstream `ZoneIslandViolation.type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneIslandKind {
    /// Upstream `isolated_island_dead_copper`: a floating pour region
    /// with no same-net seed copper whose bbox area is at least
    /// 1 mm². Upstream severity: warning.
    DeadCopper,
}

impl ZoneIslandKind {
    /// The upstream type string (reported verbatim).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        "isolated_island_dead_copper"
    }
}

/// One dead-copper violation — upstream `ZoneIslandViolation`, the
/// zero-items arm (the unconnected arm is the documented divergence
/// above; floating regions carry no items by construction).
#[derive(Debug, Clone, PartialEq)]
pub struct ZoneIslandViolation {
    /// The ConductionArea item id.
    pub pour_item_id: u32,
    /// The pour's first net's name (empty when the net row is absent
    /// — upstream's `netNumber <= 0` case).
    pub net: String,
    /// The pour's 0-based layer.
    pub layer: i32,
    /// The layer's name (upstream's `layers[layer].name` with the
    /// `Layer {n}` out-of-range fallback).
    pub layer_name: String,
    /// The violation kind (always [`ZoneIslandKind::DeadCopper`]
    /// today; carried for the reporting face's stability).
    pub kind: ZoneIslandKind,
    /// The floating region's inclusive lattice bbox (x0, y0, x1, y1).
    pub bbox: (i64, i64, i64, i64),
    /// The inclusive-bbox area in board units².
    pub area_board_units: i64,
}

/// The dead-copper violations over precomputed pour faces: every
/// floating island of a FRAGMENTED pour (`region_count > 1`, the
/// upstream `islands.size() <= 1` gate) whose inclusive bbox area is
/// at least 1 mm² (`1.0 × resolution(MM)²` board units², upstream's
/// `minDeadCopperArea` with the `>=` comparison). Pours without
/// regions (fully covered — upstream's empty-fill skip) and
/// floating regions below the threshold report nothing.
///
/// Deterministic order: pours in board-ascending id order (the
/// detector's iteration), islands in canonical region order.
#[must_use]
pub fn zone_island_violations(
    communication: &BoardCommunication,
    board: &Board,
    faces: &[PourIslands],
) -> Vec<ZoneIslandViolation> {
    // Upstream: `1.0 * resolution(MM) * resolution(MM)` — the 1.0
    // multiplier is an IEEE-754 no-op, so `res * res` is bit-identical.
    let resolution_mm = communication.resolution_mm();
    let min_dead_copper_area = resolution_mm * resolution_mm;
    let mut out = Vec::new();
    for face in faces {
        if face.region_count <= 1 {
            continue; // single continuous pour, no fragmentation
        }
        for island in &face.islands {
            let width = island.x1 - island.x0 + 1;
            let height = island.y1 - island.y0 + 1;
            let area = width * height;
            if area as f64 >= min_dead_copper_area {
                out.push(ZoneIslandViolation {
                    pour_item_id: face.pour_item_id,
                    net: face.net.clone(),
                    layer: face.layer,
                    layer_name: layer_name(board, face.layer),
                    kind: ZoneIslandKind::DeadCopper,
                    bbox: (island.x0, island.y0, island.x1, island.y1),
                    area_board_units: area,
                });
            }
        }
    }
    out
}

/// Upstream's layer-name resolution: the physical-order layer name,
/// `Layer {n}` when the index is out of range.
fn layer_name(board: &Board, layer: i32) -> String {
    board
        .layers()
        .layers
        .get(layer as usize)
        .map_or_else(|| format!("Layer {layer}"), |entry| entry.name.clone())
}

/// The CLI warning block for the violations (upstream's
/// per-violation explanation sentence, one line per island under a
/// count header — the #930 warning-block shape). Empty input is the
/// empty string; the caller warns only when violations exist.
#[must_use]
pub fn format_dead_copper_warning(
    violations: &[ZoneIslandViolation],
    communication: &BoardCommunication,
) -> String {
    if violations.is_empty() {
        return String::new();
    }
    let resolution_mm = communication.resolution_mm();
    let mut lines = Vec::with_capacity(violations.len() + 1);
    lines.push(format!(
        "Design Warning: {} dead copper island(s) detected in copper pours \
         (floating region with area >= 1 mm2)",
        violations.len()
    ));
    for violation in violations {
        // Upstream: area = getAreaInBoardUnits / (resMM * resMM),
        // `%.2f` (Locale.US — the en-US decimal point).
        let area_mm2 = violation.area_board_units as f64 / (resolution_mm * resolution_mm);
        lines.push(format!(
            "  - Dead copper island detected on {} [net {}]: area {:.2} mm2 \
             with no electrical connections.",
            violation.layer_name, violation.net, area_mm2
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::parse;
    use epic_board::islands::detect_pour_islands;

    /// The committed island-spike worlds (M6-T6): `(resolution um 10)`
    /// — one internal unit is 0.1 um, so 1 mm² = 10000*10000 = 1e8
    /// board units². All expectations below were DERIVED by running
    /// the harness `islands` face on the fixtures (derive-then-pin;
    /// buglog 266's discipline — never pin a remembered summary).
    fn spike(name: &str) -> &'static str {
        match name {
            "t11_island_clean" => {
                include_str!("../../../harness/fixtures/island-spike/t11_island_clean.dsn")
            }
            "t11_island_covered" => {
                include_str!("../../../harness/fixtures/island-spike/t11_island_covered.dsn")
            }
            "t11_island_diagonal" => {
                include_str!("../../../harness/fixtures/island-spike/t11_island_diagonal.dsn")
            }
            "t11_island_floating" => {
                include_str!("../../../harness/fixtures/island-spike/t11_island_floating.dsn")
            }
            "t11_island_gap0" => {
                include_str!("../../../harness/fixtures/island-spike/t11_island_gap0.dsn")
            }
            "t11_island_gap1" => {
                include_str!("../../../harness/fixtures/island-spike/t11_island_gap1.dsn")
            }
            "t11_island_gap2" => {
                include_str!("../../../harness/fixtures/island-spike/t11_island_gap2.dsn")
            }
            other => panic!("unknown spike fixture {other}"),
        }
    }

    fn violations_for(dsn: &str) -> Vec<ZoneIslandViolation> {
        let (_manager, board) = parse(dsn);
        let faces = detect_pour_islands(&board);
        zone_island_violations(board.communication(), &board, &faces)
    }

    /// The fully-floating world: ONE pour, ONE floating region,
    /// bbox (0,0)-(200000,100000) — inclusive area 200001*100001 =
    /// 20,000,300,001 units² = 200.0030001 mm², far over the 1 mm²
    /// threshold. Yet NO violation: `region_count == 1`, and
    /// upstream's `islands.size() <= 1` gate (mirrored here) skips
    /// single-region pours entirely — a port that dropped the gate
    /// would report this world.
    #[test]
    fn single_region_floating_pour_stays_behind_the_fragmentation_gate() {
        let violations = violations_for(spike("t11_island_floating"));
        assert!(
            violations.is_empty(),
            "region_count <= 1 gate: the single-region floating pour reports nothing"
        );
    }

    /// The diagonal staircase: 11 regions, 10 floating — nine
    /// SINGLE-CELL islands (area 1 unit², far below 1 mm² = 1e8)
    /// and the big floating body. Only the big body reports: the
    /// sub-threshold fragments stay silent — the threshold arm's
    /// natural witness.
    #[test]
    fn sub_threshold_single_cell_islands_stay_silent() {
        let violations = violations_for(spike("t11_island_diagonal"));
        assert_eq!(violations.len(), 1, "only the big body reports");
        let violation = &violations[0];
        assert_eq!(violation.kind, ZoneIslandKind::DeadCopper);
        assert_eq!(violation.kind.as_str(), "isolated_island_dead_copper");
        assert_eq!(violation.net, "PLANE");
        assert_eq!(violation.layer, 0);
        assert_eq!(violation.layer_name, "F.Cu");
        // Derived: bbox (0,49969)-(200000,100000) inclusive ->
        // 200001 * 50032.
        assert_eq!(violation.bbox, (0, 49_969, 200_000, 100_000));
        assert_eq!(violation.area_board_units, 200_001 * 50_032);
    }

    /// The carved pour (gap0): TWO regions, one seeded (the pin's
    /// half), one floating — the fragmentation gate ARMED and the
    /// floating half above threshold reports.
    #[test]
    fn carved_pour_reports_its_floating_half() {
        let violations = violations_for(spike("t11_island_gap0"));
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].bbox, (0, 50_051, 200_000, 100_000));
        assert_eq!(violations[0].area_board_units, 200_001 * 49_950);
    }

    /// The quiet worlds: clean (seeded single region), gap1/gap2
    /// (bridge-width merges — single region), covered (zero
    /// regions, upstream's empty-fill skip). No violations.
    #[test]
    fn seeded_continuous_and_covered_pours_report_nothing() {
        for name in [
            "t11_island_clean",
            "t11_island_gap1",
            "t11_island_gap2",
            "t11_island_covered",
        ] {
            let violations = violations_for(spike(name));
            assert!(violations.is_empty(), "{name} must stay quiet");
        }
    }

    /// A synthetic face for the pure arms (pub fields — no board
    /// walk needed): fragmented pour (region_count 2) carrying ONE
    /// floating island of the given inclusive bbox.
    fn synthetic_face(x1: i64, y1: i64) -> PourIslands {
        PourIslands {
            pour_item_id: 7,
            net: "NB".to_string(),
            layer: 0,
            region_count: 2,
            island_count: 1,
            islands: vec![epic_board::islands::Island {
                x0: 0,
                y0: 0,
                x1,
                y1,
                cells: (x1 + 1) * (y1 + 1),
            }],
            digest: String::new(),
        }
    }

    /// The exact 1 mm² boundary at `(resolution um 1)` (threshold =
    /// 1000*1000 = 1e6 units²): an island of inclusive bbox
    /// 1000*1000 == threshold REPORTS (upstream's `>=`), one cell
    /// short stays silent. A `>` port drops the exact-boundary
    /// island; a non-inclusive bbox convention (x1-x0)*(y1-y0)
    /// mis-areas BOTH. The empty-layer board also pins the
    /// `Layer {n}` out-of-range name fallback.
    #[test]
    fn threshold_boundary_is_inclusive_at_one_square_millimeter() {
        use epic_board::board::BoardCommunication;
        use epic_dsn::state::Unit as DsnUnit;
        let um1 = BoardCommunication {
            unit: DsnUnit::Um,
            resolution: 1,
            host_cad: None,
        };
        let board = epic_board::board::Board::new();
        let violations = zone_island_violations(
            &um1,
            &board,
            &[synthetic_face(999, 999), synthetic_face(998, 998)],
        );
        assert_eq!(
            violations.len(),
            1,
            "exact-boundary reports, one-short silent"
        );
        assert_eq!(violations[0].area_board_units, 1_000_000);
        assert_eq!(violations[0].layer_name, "Layer 0");
    }

    /// The warning text: the upstream explanation sentence (area
    /// `%.2f` in mm²) under the #930-style count header, and the
    /// empty-input contract. Area derived from the diagonal run:
    /// bbox 200001*50032 units² at `(resolution um 10)` (1 mm =
    /// 10000 units) = 100.06450032 mm² -> `100.06`.
    #[test]
    fn warning_text_pins_the_upstream_sentence() {
        use epic_board::board::BoardCommunication;
        use epic_dsn::state::Unit as DsnUnit;
        let um10 = BoardCommunication {
            unit: DsnUnit::Um,
            resolution: 10,
            host_cad: None,
        };
        let board = epic_board::board::Board::new();
        let face = PourIslands {
            pour_item_id: 4,
            net: "PLANE".to_string(),
            layer: 0,
            region_count: 11,
            island_count: 10,
            islands: vec![epic_board::islands::Island {
                x0: 0,
                y0: 49_969,
                x1: 200_000,
                y1: 100_000,
                cells: 10_006_050_032,
            }],
            digest: String::new(),
        };
        let violations = zone_island_violations(&um10, &board, &[face]);
        assert_eq!(violations.len(), 1);
        assert_eq!(
            format_dead_copper_warning(&violations, &um10),
            "Design Warning: 1 dead copper island(s) detected in copper pours \
             (floating region with area >= 1 mm2)\n  \
             - Dead copper island detected on Layer 0 [net PLANE]: area 100.06 mm2 \
             with no electrical connections."
        );
        assert_eq!(format_dead_copper_warning(&[], &um10), "");
    }
}
