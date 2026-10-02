//! F3 (Rust-only, no Java counterpart): the ground-pour ask.
//!
//! Tyler's scenario (2026-10-01): a board whose GND net carries no
//! copper pour routes GND as ordinary traces, silently — the router
//! never surfaces that a pour was probably intended. Two faces here:
//!
//! * [`pour_candidates`] — the DETECTION face (pure): which
//!   ground-LIKE nets (name heuristic) have no pour anywhere and at
//!   least one on-board pin. The host surfaces these BEFORE routing
//!   (the CLI prints the ask; the F4 interview layer will make it a
//!   dialog). Detection deliberately emits no sink events and no
//!   manifest rows by default — the events goldens and the digest
//!   faces (det1 canaries, threads, the global goldens' normalized
//!   manifest SHAs) stay byte-identical on every existing board.
//! * [`synthesize_pours`] — the OPT-IN answer: one full-board
//!   [`epic_board::items::ItemData::ConductionArea`] per requested
//!   net, the same construction the Java parse-time
//!   missing-power-plane fallback uses (`Structure.java:1067-1122`:
//!   `isObstacle=false`, ctor default `isFilled=true`, clearance
//!   class none, `SYSTEM_FIXED`), plus `contains_plane` on the net —
//!   the flag the router already reads to lower via costs and gate
//!   completion through the plane
//!   (`pipeline/connection_router.rs:167-221`). A synthesized pour
//!   changes real routing behavior with ZERO router edits.
//!
//! The pour is a plain board INSERT (arena + search tree), not a
//! mutation of an existing item — the F1 remove/re-insert dance is
//! for changed items; a fresh id has no derived data to clear.

use epic_board::board::{Board, ItemEntry};
use epic_board::items::{Area, BoardShape, FixedState, ItemData};
use epic_board::tree_manager::SearchTreeManager;
use epic_geometry::regular_tile_shape::RegularTileShape;
use epic_geometry::tile_shape::TileShape;

/// One detected "probably wanted a pour" net — the ask the host
/// surfaces pre-route (the CLI note and the F4 interview read these
/// verbatim).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PourCandidate {
    /// The net's board number (1-based).
    pub net_number: i32,
    /// The board's own net spelling (canonical, never the request's).
    pub net_name: String,
    /// On-board single-pin items carrying the net (the pour's
    /// would-be load).
    pub pin_count: usize,
}

/// The ground-name heuristic: the uppercased name contains `GND`
/// (GND, AGND, DGND, GNDA, PGND, GND_1, …) or starts with `VSS`
/// (VSS, VSSA, VSS_3V3, …) — the standard KiCad ground families.
/// Deliberately broad: a false positive costs one notice line; the
/// synthesis is explicit opt-in by exact net name regardless.
#[must_use]
pub fn is_ground_like(name: &str) -> bool {
    let upper = name.to_uppercase();
    upper.contains("GND") || upper.starts_with("VSS")
}

/// The on-board pin count for one net (the pour candidate's load
/// measure — a pour for a 0-pin net connects nothing and is not a
/// candidate).
fn pin_count(board: &Board, net_number: i32) -> usize {
    board
        .iter_ascending()
        .filter(|entry| entry.on_the_board)
        .filter(|entry| matches!(entry.data, ItemData::Pin { .. }))
        .filter(|entry| entry.nets.contains(&net_number))
        .count()
}

/// The DETECTION face (pure): every ground-like net with NO pour
/// anywhere (`contains_plane` false — the parse sets it exactly when
/// a plane claims the net) and at least one on-board pin, in net
/// order. No events, no mutation — the host decides how to ask.
#[must_use]
pub fn pour_candidates(board: &Board) -> Vec<PourCandidate> {
    board
        .rules()
        .nets
        .iter()
        .filter(|(_, net)| is_ground_like(&net.name) && !net.contains_plane)
        .filter_map(|(net_number, net)| {
            let pins = pin_count(board, net_number);
            (pins > 0).then(|| PourCandidate {
                net_number,
                net_name: net.name.clone(),
                pin_count: pins,
            })
        })
        .collect()
}

/// One requested pour (the `router.pour.nets` grammar's entry): the
/// net by name plus an optional layer; `None` pours on the LAST
/// signal layer (the bottom-copper default of classic 2-layer
/// practice).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PourRequest {
    /// The net's name (any case; resolved case-insensitively like
    /// every net-naming face in the engine).
    pub net: String,
    /// An explicit layer NAME, or `None` for the last signal layer.
    pub layer: Option<String>,
}

/// One synthesized pour, for the report (the manifest rows and the
/// session telemetry render these verbatim).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PourRow {
    /// The net's board number (1-based).
    pub net_number: i32,
    /// The board's own net spelling (the canonical-name law).
    pub net_name: String,
    /// The poured layer (0-based board index).
    pub layer_no: i32,
    /// The layer's own name.
    pub layer_name: String,
    /// The net's on-board pin count at synthesis time.
    pub pin_count: usize,
}

/// The synthesis report — the F2 shape: rows for what landed,
/// `unresolved` (naming the net) for what did not and why. Reported,
/// never fatal, never guessed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PourReport {
    /// One row per synthesized pour, in request order.
    pub rows: Vec<PourRow>,
    /// The honest refusals: unknown net, unknown layer, already
    /// poured, no board outline — each naming the net.
    pub unresolved: Vec<String>,
}

/// Resolve a layer NAME to its 0-based index: the exact
/// case-sensitive `LayerStructure.getNo` first (the Java face), then
/// a case-insensitive fallback (liberal in what the setting accepts —
/// a wrong-case layer name is a typo, not a different layer).
fn resolve_layer(board: &Board, name: &str) -> Option<i32> {
    let exact = board.layers().get_no(name);
    if exact >= 0 {
        return Some(exact);
    }
    board
        .layers()
        .layers
        .iter()
        .position(|layer| layer.name.eq_ignore_ascii_case(name))
        .and_then(|index| i32::try_from(index).ok())
}

/// The default pour layer: the LAST signal (routing) layer — the
/// bottom copper of a classic 2-layer stack. `None` only on a board
/// with no signal layers at all (unroutable; every request lands in
/// `unresolved`).
fn default_pour_layer(board: &Board) -> Option<i32> {
    board
        .layers()
        .layers
        .iter()
        .enumerate()
        .filter(|&(_, layer)| layer.is_signal)
        .filter_map(|(index, _)| i32::try_from(index).ok())
        .next_back()
}

/// The OPT-IN answer to the ask (the `router.pour.nets` face): for
/// each request, insert one full-board conduction area carrying the
/// net on the resolved layer and set `contains_plane` — the parse's
/// own plane construction (`Structure.java:1108`, the
/// missing-power-plane fallback `:559-566`): border = the board's
/// bounding box, `is_obstacle=false`, `is_filled=true`, clearance
/// class 0, `SYSTEM_FIXED` (the optimizer never moves a synthesized
/// pour). Arena insert + search-tree insert — the pour must be in
/// the tree or every clearance/overlap query would miss it.
///
/// Runs at the route head (after pin assignment and current-driven
/// widths, before the geometry pass) so the pipeline routes the
/// poured board. Unresolved requests warn through the sink and never
/// fail the run.
pub fn synthesize_pours(
    board: &mut Board,
    manager: &mut SearchTreeManager,
    requests: &[PourRequest],
) -> PourReport {
    let mut report = PourReport::default();
    for request in requests {
        let Some((net_number, net)) = board
            .rules()
            .nets
            .iter()
            .find(|(_, net)| net.name.to_lowercase() == request.net.to_lowercase())
        else {
            report
                .unresolved
                .push(format!("{}: no such net", request.net));
            continue;
        };
        let net_name = net.name.clone(); // the CANONICAL name (the
        // request may carry any case; reports name the board's own
        // spelling).
        if net.contains_plane {
            report
                .unresolved
                .push(format!("{net_name}: already has a copper pour"));
            continue;
        }
        let layer_no = match &request.layer {
            Some(name) => match resolve_layer(board, name) {
                Some(no) => no,
                None => {
                    report
                        .unresolved
                        .push(format!("{net_name}: no such layer {name}"));
                    continue;
                }
            },
            None => match default_pour_layer(board) {
                Some(no) => no,
                None => {
                    report
                        .unresolved
                        .push(format!("{net_name}: no signal layer to pour"));
                    continue;
                }
            },
        };
        let Some(layer_name) = board
            .layers()
            .layers
            .get(usize::try_from(layer_no).ok().unwrap_or(usize::MAX))
            .map(|layer| layer.name.clone())
        else {
            report
                .unresolved
                .push(format!("{net_name}: layer {layer_no} missing"));
            continue;
        };
        let Some(bounding_box) = board.bounding_box() else {
            report
                .unresolved
                .push(format!("{net_name}: board has no outline to pour"));
            continue;
        };
        let pins = pin_count(board, net_number);
        let id = board.alloc_id();
        board.insert_item(ItemEntry {
            id,
            data: ItemData::ConductionArea {
                layer: layer_no,
                area: Area::simple(BoardShape::Tile(TileShape::RegularTileShape(
                    RegularTileShape::IntBox(bounding_box),
                ))),
                // The parse-time plane flags (Structure.java:1108 +
                // ConductionArea.java:30): not an obstacle, filled.
                is_obstacle: false,
                is_filled: true,
            },
            nets: vec![net_number],
            // BoardRules.clearanceClassNone() — the fallback's class.
            clearance_class: 0,
            component_id: 0,
            fixed: FixedState::SystemFixed,
            // insert_item flips it (the on_the_board contract).
            on_the_board: false,
        });
        manager.insert(board, id);
        board.rules_mut().nets.set_contains_plane(net_number, true);
        report.rows.push(PourRow {
            net_number,
            net_name,
            layer_no,
            layer_name,
            pin_count: pins,
        });
    }
    report
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ground_like_names_cover_the_families() {
        for name in [
            "GND", "gnd", "AGND", "DGND", "GNDA", "PGND", "GND_1", "+3V3_GND",
        ] {
            assert!(is_ground_like(name), "{name} must be ground-like");
        }
        assert!(!is_ground_like("PWR"), "PWR is not ground");
        assert!(!is_ground_like("VCC"), "VCC is not ground");
        assert!(!is_ground_like("N1"), "signal names are not ground");
        assert!(is_ground_like("VSS"), "VSS is ground");
        assert!(is_ground_like("vssa"), "vssa is ground");
    }
}

/// The F3 integration tests: detection and synthesis on a CRAFT DSN
/// (the F2 grammar) with the standard ground families live — GND
/// (3 pins), AGND, VSS, plus the non-ground controls PWR and N1.
#[cfg(test)]
mod apply_tests {
    use super::*;
    use epic_board::id::ItemId;
    use epic_dsn::reader::{DsnReadResult, read_board};
    use epic_dsn::ses_board::SesBoard;

    const POUR_CRAFT_DSN: &str = r#"(pcb pour-craft.dsn
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
    (component "TGT" (place "T7" 36000 12000 Front 0.000000))
    (component "TGT" (place "T6" 36000 20000 Front 0.000000))
    (component "TGT" (place "T5" 100000 32000 Front 0.000000))
    (component "TGT" (place "T4" 100000 40000 Front 0.000000))
    (component "TGT" (place "T3" 100000 48000 Front 0.000000))
    (component "TGT" (place "T2" 100000 56000 Front 0.000000))
    (component "TGT" (place "T1" 100000 64000 Front 0.000000))
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
    (net "GND" (pins "CONN1"-"CA1" "T1"-"TA" "T2"-"TA"))
    (net "AGND" (pins "CONN1"-"CA2" "T3"-"TA"))
    (net "VSS" (pins "CONN1"-"CA3" "T4"-"TA"))
    (net "PWR" (pins "CONN1"-"CA4" "T5"-"TA"))
    (net "N1" (pins "T6"-"TA" "T7"-"TA"))
    (class kicad_default "GND" "AGND" "VSS" "PWR" "N1"
      (rule (clearance 250)(width 200))
    )
  )
)
"#;

    /// Parses the craft DSN into a live board + tree manager (the
    /// pour face INSERTS items — the tree must exist, the F1 helper
    /// pattern).
    fn pour_board() -> (Board, SearchTreeManager) {
        let mut ses = SesBoard::new();
        match read_board(POUR_CRAFT_DSN.as_bytes(), &mut ses) {
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

    fn net_no(board: &Board, name: &str) -> i32 {
        board
            .rules()
            .nets
            .iter()
            .find(|(_, net)| net.name == name)
            .unwrap_or_else(|| panic!("net {name} exists"))
            .0
    }

    fn conduction_areas(board: &Board) -> Vec<(ItemId, &ItemEntry)> {
        board
            .iter_ascending()
            .filter(|entry| entry.on_the_board)
            .filter(|entry| matches!(entry.data, ItemData::ConductionArea { .. }))
            .map(|entry| (entry.id, entry))
            .collect()
    }

    /// THE DETECTION PIN: the ground families surface as candidates
    /// with their pin counts (GND 3, AGND 2, VSS 2, in net order);
    /// PWR and N1 never do; a synthesized pour RETIRES its candidate
    /// (the ask is answered once).
    #[test]
    fn detects_unpoured_ground_nets_and_synthesis_retires_them() {
        let (mut board, mut manager) = pour_board();
        let candidates = pour_candidates(&board);
        let names: Vec<&str> = candidates.iter().map(|c| c.net_name.as_str()).collect();
        assert_eq!(
            names,
            ["GND", "AGND", "VSS"],
            "the ground families, in net order"
        );
        assert_eq!(candidates[0].pin_count, 3, "GND carries 3 pins");
        assert_eq!(candidates[1].pin_count, 2, "AGND carries 2 pins");
        assert_eq!(candidates[0].net_number, net_no(&board, "GND"));

        let report = synthesize_pours(
            &mut board,
            &mut manager,
            &[PourRequest {
                net: "GND".to_string(),
                layer: None,
            }],
        );
        assert_eq!(report.rows.len(), 1, "the GND pour lands");
        assert!(report.unresolved.is_empty());

        let remaining = pour_candidates(&board);
        let names: Vec<&str> = remaining.iter().map(|c| c.net_name.as_str()).collect();
        assert_eq!(
            names,
            ["AGND", "VSS"],
            "GND is poured — no longer a candidate"
        );
    }

    /// THE SYNTHESIS PIN: the default pour lands on the LAST signal
    /// layer (B.Cu) as the parse's own plane construction — a filled,
    /// non-obstacle, SYSTEM_FIXED conduction area carrying exactly
    /// the net over the board's bounding box — and flips the net's
    /// `contains_plane` (the flag the router reads).
    #[test]
    fn synthesizes_pour_on_last_signal_layer() {
        let (mut board, mut manager) = pour_board();
        let gnd = net_no(&board, "GND");
        let item_count = board.iter_ascending().filter(|e| e.on_the_board).count();
        let report = synthesize_pours(
            &mut board,
            &mut manager,
            &[PourRequest {
                net: "gnd".to_string(), // any case resolves
                layer: None,
            }],
        );
        assert_eq!(
            report.rows,
            vec![PourRow {
                net_number: gnd,
                net_name: "GND".to_string(), // canonical spelling
                layer_no: 1,                 // B.Cu — the last signal layer
                layer_name: "B.Cu".to_string(),
                pin_count: 3,
            }],
            "the row names the board's own spelling and the poured layer"
        );

        let areas = conduction_areas(&board);
        assert_eq!(areas.len(), 1, "exactly one conduction area is inserted");
        let (_, entry) = &areas[0];
        let ItemData::ConductionArea {
            layer,
            area,
            is_obstacle,
            is_filled,
        } = &entry.data
        else {
            unreachable!("filtered above");
        };
        assert_eq!(*layer, 1, "B.Cu");
        assert_eq!(entry.nets, [gnd], "carries exactly the net");
        assert!(!is_obstacle, "the parse-time plane flag");
        assert!(*is_filled, "the ctor default");
        assert_eq!(
            entry.fixed,
            FixedState::SystemFixed,
            "the fallback's fix state"
        );
        assert_eq!(entry.clearance_class, 0, "clearanceClassNone");
        // The pour covers the board's bounding box (the fallback's
        // border — computed from the same face, no literal to drift).
        let expected_box = board.bounding_box().expect("bbox");
        match &area.border {
            BoardShape::Tile(TileShape::RegularTileShape(RegularTileShape::IntBox(b))) => {
                assert_eq!(*b, expected_box, "the pour spans the board");
            }
            other => panic!("a box pour, got {other:?}"),
        }
        assert!(
            board.iter_ascending().filter(|e| e.on_the_board).count() == item_count + 1,
            "exactly one item was added"
        );
        assert!(
            board.rules().nets.get(gnd).expect("GND").contains_plane,
            "the router's plane flag is set"
        );
        assert!(
            !board
                .rules()
                .nets
                .get(net_no(&board, "AGND"))
                .expect("AGND")
                .contains_plane,
            "sibling nets are untouched"
        );
    }

    /// THE HONEST-REFUSAL PIN: an explicit layer name lands there
    /// (case-insensitively); an unknown net, an already-poured net,
    /// and an unknown layer land in `unresolved` naming the net — and
    /// nothing else mutates.
    #[test]
    fn explicit_layer_and_unresolved_faces() {
        let (mut board, mut manager) = pour_board();
        let item_count = board.iter_ascending().filter(|e| e.on_the_board).count();
        let report = synthesize_pours(
            &mut board,
            &mut manager,
            &[
                PourRequest {
                    net: "VSS".to_string(),
                    layer: Some("f.cu".to_string()), // case-insensitive
                },
                PourRequest {
                    net: "NOPE".to_string(),
                    layer: None,
                },
                PourRequest {
                    net: "VSS".to_string(), // already poured by row 1
                    layer: None,
                },
                PourRequest {
                    net: "AGND".to_string(),
                    layer: Some("MARS.Cu".to_string()),
                },
            ],
        );
        assert_eq!(report.rows.len(), 1, "only the VSS pour lands");
        assert_eq!(report.rows[0].layer_no, 0, "F.Cu — the explicit layer");
        assert_eq!(report.rows[0].layer_name, "F.Cu");
        assert_eq!(
            report.unresolved,
            vec![
                "NOPE: no such net".to_string(),
                "VSS: already has a copper pour".to_string(),
                "AGND: no such layer MARS.Cu".to_string(),
            ],
            "the honest refusals, each naming the net"
        );
        assert!(
            board.iter_ascending().filter(|e| e.on_the_board).count() == item_count + 1,
            "exactly one item was added (refusals mutate nothing)"
        );
        assert!(
            !board
                .rules()
                .nets
                .get(net_no(&board, "GND"))
                .expect("GND")
                .contains_plane,
            "an unresolved request never pours"
        );
    }
}
