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
//! FACE NOTE (the unconnected class, ported at 152-H): upstream's
//! `isolated_island_unconnected` ERROR arm asks whether a seeded
//! island's items reach the PRIMARY island's items through
//! traces/vias elsewhere — `rep.getConnectedSet(netNumber, true)`,
//! the `stopAtPlane = true` overload (component-less conduction
//! areas are severed by the walk, so the fragmented pour itself is
//! never a bridge). This port runs the mirror walk
//! ([`epic_board::contacts::item_connected_set_stopping_at_plane`]
//! with `true`) over the detector's per-region seed attribution
//! ([`epic_board::islands::PourIslands::region_seeds`]).
//!
//! Mapping divergences (documented, deliberate):
//!
//! * upstream maps items onto AWT islands by CENTER containment
//!   (DrillItem center / Trace firstCorner); this port maps by
//!   LATTICE overlap — a seed item's copper UNIONED into a region
//!   claims it. A region here is therefore a maximal same-layer
//!   same-net copper union: a bridging pin/trace MERGES what
//!   upstream would still call two islands, so this face fires
//!   strictly less often, and every firing is a severance real in
//!   ANY same-layer topology (never a center-mapping false
//!   positive). The residual question the walk answers is exactly
//!   the inter-layer one: can the region's seed reach the primary's
//!   through vias/traces on other layers, with pours severed.
//! * upstream's primary representative is `primary.items[0]` in
//!   `board.getItems()` walk order; this port uses the ascending-id
//!   first (deterministic here, arbitrary there — connected-set
//!   equivalence across a region's items is guaranteed by neither;
//!   pinned, not accidental).
//! * upstream folds the unconnected class into the summary's
//!   net-deduped `unconnectedNetsCount`; this port is
//!   REPORTING-ONLY (stderr) — no score or count face reads it.
//!
//! Area convention: upstream reads the CONTINUOUS bbox
//! (`getBounds2D` width × height) of the AWT island; this port
//! reads the INCLUSIVE lattice bbox ((x1−x0+1) · (y1−y0+1) unit
//! cells — each cell a unit square, so the inclusive dims are the
//! region's geometric extent under the lattice convention). The two
//! differ by at most one cell row/column at the boundary edges —
//! immaterial except exactly at the 1 mm² threshold, where the
//! boundary fixture pins this port's convention.

use std::cmp::Reverse;

use epic_board::board::{Board, BoardCommunication};
use epic_board::contacts::item_connected_set_stopping_at_plane;
use epic_board::id::ItemId;
use epic_board::islands::PourIslands;
use epic_board::tree_manager::SearchTreeManager;

/// The violation kind (upstream `ZoneIslandViolation.type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneIslandKind {
    /// Upstream `isolated_island_unconnected`: a seeded region of a
    /// fragmented pour whose items cannot reach the primary region's
    /// through traces/vias with pours severed (the
    /// `stopAtPlane = true` walk). Upstream severity: error.
    Unconnected,
    /// Upstream `isolated_island_dead_copper`: a floating pour region
    /// with no same-net seed copper whose bbox area is at least
    /// 1 mm². Upstream severity: warning.
    DeadCopper,
}

impl ZoneIslandKind {
    /// The upstream type string (reported verbatim).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unconnected => "isolated_island_unconnected",
            Self::DeadCopper => "isolated_island_dead_copper",
        }
    }
}

/// One zone-island violation — upstream `ZoneIslandViolation`.
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
    /// The violation kind.
    pub kind: ZoneIslandKind,
    /// The region's inclusive lattice bbox (x0, y0, x1, y1).
    pub bbox: (i64, i64, i64, i64),
    /// The inclusive-bbox area in board units².
    pub area_board_units: i64,
    /// Upstream `itemsInIsland` — the severed region's seed item ids
    /// (empty for dead copper: floating regions carry no items by
    /// construction).
    pub items: Vec<u32>,
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
                    items: Vec::new(),
                });
            }
        }
    }
    out
}

/// The `isolated_island_unconnected` violations over precomputed pour
/// faces (upstream `3011e6e60`, the seeded-island arm): for every
/// FRAGMENTED pour (`region_count > 1`), the primary region is the one
/// carrying the most seed items (first-max in canonical scan order —
/// upstream's strict `>` over `maxConnectedItems` starting at −1), and
/// every OTHER seeded region whose items are ALL outside the primary's
/// representative's connected set reports one violation.
///
/// The walk is upstream's exact overload —
/// `getConnectedSet(netNumber, stopAtPlane = true)`: component-less
/// conduction areas are severed, so neither the pour under test nor
/// any other free pour can bridge the two regions; only traces/vias
/// (same-layer direct contacts included) can. ONE walk per pour
/// (upstream computes the primary's set once, then tests every island
/// against it). Regions without seeds stay silent here (the
/// dead-copper arm's subject); a netless pour seeds nothing, so the
/// arm is naturally inert for it.
///
/// Deterministic order: pours in board-ascending id order, regions in
/// canonical scan order.
#[must_use]
pub fn zone_island_unconnected_violations(
    manager: &SearchTreeManager,
    board: &mut Board,
    faces: &[PourIslands],
) -> Vec<ZoneIslandViolation> {
    let mut out = Vec::new();
    for face in faces {
        if face.region_count <= 1 {
            continue; // single continuous pour, no fragmentation
        }
        // First-max primary (upstream's strict `>` keeps the FIRST of
        // equal counts).
        let mut primary_idx = 0usize;
        let mut primary_len = 0usize;
        for (idx, region) in face.region_seeds.iter().enumerate() {
            if idx == 0 || region.items.len() > primary_len {
                primary_idx = idx;
                primary_len = region.items.len();
            }
        }
        let Some(&rep) = face.region_seeds[primary_idx].items.first() else {
            // Upstream's defensive guard (unreachable-dead there via
            // the argmax, and here too: an empty primary implies every
            // region is empty, so the loop below is a no-op anyway).
            continue;
        };
        let connected = item_connected_set_stopping_at_plane(
            manager,
            board,
            ItemId::new(rep),
            face.net_number,
            true,
        );
        for (idx, region) in face.region_seeds.iter().enumerate() {
            if idx == primary_idx || region.items.is_empty() {
                continue;
            }
            let severed = region
                .items
                .iter()
                .all(|id| !connected.contains(&Reverse(ItemId::new(*id))));
            if severed {
                let width = region.x1 - region.x0 + 1;
                let height = region.y1 - region.y0 + 1;
                out.push(ZoneIslandViolation {
                    pour_item_id: face.pour_item_id,
                    net: face.net.clone(),
                    layer: face.layer,
                    layer_name: layer_name(board, face.layer),
                    kind: ZoneIslandKind::Unconnected,
                    bbox: (region.x0, region.y0, region.x1, region.y1),
                    area_board_units: width * height,
                    items: region.items.clone(),
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

/// The CLI warning block for the unconnected violations (upstream's
/// per-violation explanation sentence, one line per island under a
/// count header — the #930 warning-block shape; upstream severity is
/// ERROR, hence the header word). Empty input is the empty string;
/// the caller warns only when violations exist.
#[must_use]
pub fn format_unconnected_warning(violations: &[ZoneIslandViolation]) -> String {
    if violations.is_empty() {
        return String::new();
    }
    let mut lines = Vec::with_capacity(violations.len() + 1);
    lines.push(format!(
        "Design Error: {} isolated island(s) detected in copper pours \
         (seeded island severed from the primary region)",
        violations.len()
    ));
    for violation in violations {
        // Upstream: "Copper pour on %s [net %s] is fragmented: island
        // contains %d pin(s)/via(s) disconnected from main pour."
        lines.push(format!(
            "  - Copper pour on {} [net {}] is fragmented: island contains {} \
             pin(s)/via(s) disconnected from main pour.",
            violation.layer_name,
            violation.net,
            violation.items.len()
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
            "t11_island_severed" => {
                include_str!("../../../harness/fixtures/island-spike/t11_island_severed.dsn")
            }
            "t11_island_bridged" => {
                include_str!("../../../harness/fixtures/island-spike/t11_island_bridged.dsn")
            }
            other => panic!("unknown spike fixture {other}"),
        }
    }

    fn violations_for(dsn: &str) -> Vec<ZoneIslandViolation> {
        let (_manager, board) = parse(dsn);
        let faces = detect_pour_islands(&board);
        zone_island_violations(board.communication(), &board, &faces)
    }

    /// The 152-H unconnected arm over a parsed world: the faces feed
    /// the walk (the board is mutably walked through the manager).
    fn unconnected_for(dsn: &str) -> (Vec<ZoneIslandViolation>, Vec<PourIslands>) {
        let (manager, mut board) = parse(dsn);
        let faces = detect_pour_islands(&board);
        let violations = zone_island_unconnected_violations(&manager, &mut board, &faces);
        (violations, faces)
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

    /// 152-H: the severed world — two seeded regions, no inter-layer
    /// copper. The lower region is primary (first-max in scan order,
    /// both carry one item), and its pin's stop-at-plane walk cannot
    /// reach the upper pin (the fragmented pour itself is severed by
    /// the walk), so the upper region reports ONE error violation
    /// carrying its pin.
    #[test]
    fn severed_world_reports_the_unconnected_island() {
        let (violations, faces) = unconnected_for(spike("t11_island_severed"));
        assert_eq!(violations.len(), 1);
        let violation = &violations[0];
        assert_eq!(violation.kind, ZoneIslandKind::Unconnected);
        assert_eq!(violation.kind.as_str(), "isolated_island_unconnected");
        assert_eq!(violation.net, "PLANE");
        assert_eq!(violation.layer, 0);
        assert_eq!(violation.layer_name, "F.Cu");
        // The upper region's geometry == the gap0 floating half (the
        // carve wires are identical; the seed pin's copper sits
        // strictly inside the region, extending no bbox edge).
        assert_eq!(violation.bbox, (0, 50_051, 200_000, 100_000));
        assert_eq!(violation.area_board_units, 200_001 * 49_950);
        // The island's items == the non-primary region's seed items
        // (the attribution itself is pinned in epic-board's islands
        // world tests, geometry-resolved).
        assert_eq!(violation.items, faces[0].region_seeds[1].items);
        assert_eq!(violation.items.len(), 1);
    }

    /// 152-H control: the bridged world — IDENTICAL F.Cu partition,
    /// but the through-hole pads + B.Cu PLANE wire let the walk cross
    /// pin → wire → pin with pours severed (`stop_at_plane` skips the
    /// pour, never traces/vias). NO violation: this is exactly the
    /// inter-layer connectivity the arm exists to see through.
    #[test]
    fn bridged_world_crosses_layers_and_stays_quiet() {
        let (violations, faces) = unconnected_for(spike("t11_island_bridged"));
        assert_eq!(
            faces[0].region_count, 2,
            "the F.Cu partition is still fragmented"
        );
        assert!(
            violations.is_empty(),
            "the B.Cu wire bridges the two seeded regions through the pads"
        );
    }

    /// The unconnected warning text: the upstream explanation sentence
    /// (d0d876e30, verbatim modulo the #930 block shape) under the
    /// ERROR header, and the empty-input contract.
    #[test]
    fn unconnected_warning_text_pins_the_upstream_sentence() {
        let violation = ZoneIslandViolation {
            pour_item_id: 9,
            net: "PLANE".to_string(),
            layer: 0,
            layer_name: "F.Cu".to_string(),
            kind: ZoneIslandKind::Unconnected,
            bbox: (0, 50_051, 200_000, 100_000),
            area_board_units: 200_001 * 49_950,
            items: vec![12, 47],
        };
        assert_eq!(
            format_unconnected_warning(&[violation]),
            "Design Error: 1 isolated island(s) detected in copper pours \
             (seeded island severed from the primary region)\n  \
             - Copper pour on F.Cu [net PLANE] is fragmented: island contains 2 \
             pin(s)/via(s) disconnected from main pour."
        );
        assert_eq!(format_unconnected_warning(&[]), "");
    }

    /// A synthetic face for the pure arms (pub fields — no board
    /// walk needed): fragmented pour (region_count 2) carrying ONE
    /// floating island of the given inclusive bbox.
    fn synthetic_face(x1: i64, y1: i64) -> PourIslands {
        PourIslands {
            pour_item_id: 7,
            net: "NB".to_string(),
            net_number: 3,
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
            region_seeds: Vec::new(),
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
            net_number: 2,
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
            region_seeds: Vec::new(),
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
