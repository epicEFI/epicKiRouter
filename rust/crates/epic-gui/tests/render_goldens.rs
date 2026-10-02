//! M9-T4: the render-golden pins (the dispatch charter for
//! `epic-gui`):
//!
//! 1. **the golden verify x3** — bm08 (the Tier A parity fixture),
//!    kit2-led-cube (the probed moderate PCBench reference board),
//!    e1_ripup (the T3 crafted world): the in-process re-derivation is
//!    byte-identical (serde_json) to each committed golden;
//! 2. **layer-visibility exclusion** — hide a populated layer and the
//!    projection loses EXACTLY that layer's ops (crafted prefix/suffix
//!    proof + the bm08 count-moved face + the all-hidden ==
//!    outline-only face);
//! 3. **the cull boundary ±1 (DNR-16)** — a viewport edge placed so a
//!    pad is 1-inside (kept), exactly-on (kept), 1-outside (culled);
//!    `culled` moves by exactly the right amount;
//! 4. **the overlay flags** — all four exist; all 16 combinations
//!    change NOTHING in T4 output (the plumbing pin; T5 extends the
//!    one site in `render.rs`);
//! 5. **the pan/zoom round trips** —
//!    `screen_to_world(world_to_screen(p)) == p` at the i64 faces and
//!    the symmetric screen face, exact per `ScreenTransform`'s
//!    documented exactness contract (full-domain at 1:1 and at every
//!    integer px-per-DBU zoom; lattice-exact otherwise, quantization
//!    bounded).
//!
//! All runs are IN-PROCESS (no spawned binaries). The goldens are
//! captured/regenerated via `capture_render_goldens` (`#[ignore]`ged,
//! in-process — run with `--ignored`, `EPIC_GUI_REGEN` not required).

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use epic_engine::session::Session;
use epic_engine::settings::CliLayer;
use epic_engine::settings::SessionLayer;
use epic_engine::snapshot::{BoardSnapshot, board_snapshot};
use epic_geometry::int_box::IntBox;
use epic_geometry::int_point::IntPoint;
use epic_gui::render::{RenderList, project};
use epic_gui::view::{OverlayFlags, ScreenTransform, ViewModel};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("rust/ is two parents up from crates/epic-gui")
        .to_path_buf()
}

const BM08: &str = "../scripts/benchmark/fixtures/DAC2020_boards/DAC2020_bm08.dsn";
const KIT2: &str = "../scripts/benchmark/fixtures/PCBench/kit2-led-cube_led_cube/unrouted.dsn";
const E1_RIPUP: &str = "harness/fixtures/event-stream/e1_ripup.dsn";

/// Loads a fixture through the session face (the T4 golden path:
/// `Session::load_dsn` at defaults -> post-load `board_snapshot`).
fn load_snapshot(repo_rel: &str) -> BoardSnapshot {
    let bytes = fs::read(repo_root().join(repo_rel))
        .unwrap_or_else(|error| panic!("{repo_rel} is readable: {error}"));
    let session = match Session::load_dsn(&bytes, SessionLayer::default()) {
        Ok(session) => session,
        Err(error) => panic!("{repo_rel} loads at defaults: {error:?}"),
    };
    board_snapshot(session.board())
}

/// THE FIXED TEST TRANSFORM (the golden transform, with its
/// derivation — DNR-19): `pan = snapshot.bounds.ll` (the fixture's
/// lower-left anchored at the screen origin) and zoom 1/1.
///
/// DERIVATION of zoom 1/1: it is the unique px-per-DBU ratio whose
/// world<->screen map is a full BIJECTION (both round trips exact on
/// the entire in-range domain — `ScreenTransform`'s exactness
/// contract), which is what the charter's round-trip pin demands at
/// the i64 faces; the dispatch letter's "1:1000-ish" example would
/// make that exactness lattice-only. The projected extents then
/// equal the fixture's DBU extents (probed < 2^30 for all three
/// fixtures — they fit i32 with margin and exercise both screen
/// axes).
fn fixed_test_transform(snapshot: &BoardSnapshot) -> ScreenTransform {
    ScreenTransform::new(
        epic_engine::snapshot::PointPrimitive {
            x: snapshot.bounds.ll_x,
            y: snapshot.bounds.ll_y,
        },
        1,
        1,
    )
}

/// The wide-open viewport (every primitive is inside; the golden
/// renders are cull-free by construction — `culled == 0`).
fn full_viewport() -> IntBox {
    IntBox::new(
        IntPoint::new(i32::MIN, i32::MIN),
        IntPoint::new(i32::MAX, i32::MAX),
    )
}

/// Every layer the snapshot's primitives name (the "all layers
/// visible" face; the set is sorted — BTreeSet).
fn populated_layers(snapshot: &BoardSnapshot) -> BTreeSet<i32> {
    let mut layers = BTreeSet::new();
    for trace in &snapshot.traces {
        layers.insert(trace.layer);
    }
    for pad in &snapshot.pads {
        layers.insert(pad.layer);
    }
    for area in &snapshot.areas {
        layers.insert(area.layer);
    }
    for via in &snapshot.vias {
        layers.extend(via.layers.iter().copied());
    }
    layers
}

/// The golden view: the fixed test transform + every populated layer
/// visible + the documented default color table + default overlays.
fn golden_view(snapshot: &BoardSnapshot) -> ViewModel {
    let mut view = ViewModel::new(fixed_test_transform(snapshot));
    view.visible_layers = populated_layers(snapshot);
    view
}

/// Serializes a `RenderList` exactly as the goldens store it (the
/// golden-format convention: `serde_json::to_string_pretty`; the
/// header lines ride separately in the file).
fn render_json(list: &RenderList) -> String {
    serde_json::to_string_pretty(list).expect("RenderList serializes")
}

/// Reads a committed golden: strips the `#` header lines, trims the
/// trailing newline.
fn read_golden(rel: &str) -> String {
    let text = fs::read_to_string(
        repo_root()
            .join("harness/fixtures/gui-render/golden")
            .join(rel),
    )
    .unwrap_or_else(|error| panic!("golden {rel} is readable: {error}"));
    let body: String = text
        .lines()
        .filter(|line| !line.starts_with('#'))
        .collect::<Vec<&str>>()
        .join("\n");
    body.trim_end().to_string()
}

/// The shared golden-verify body (determinism double-derivation +
/// byte-compare against the committed file).
fn assert_golden_matches(fixture_rel: &str, golden_rel: &str) {
    let snapshot = load_snapshot(fixture_rel);
    let view = golden_view(&snapshot);

    let run_one = project(&snapshot, &view, full_viewport());
    let run_two = project(&snapshot, &view, full_viewport());
    assert_eq!(
        render_json(&run_one),
        render_json(&run_two),
        "two projections of the same inputs are byte-identical (determinism)"
    );
    assert_eq!(run_one.culled, 0, "the wide-open viewport culls nothing");

    let golden = read_golden(golden_rel);
    assert!(!golden.is_empty(), "the golden body is non-empty");
    assert_eq!(
        render_json(&run_one),
        golden,
        "the re-derivation is byte-identical to the committed golden"
    );
}

// --- PIN 1 — the golden verify x3 -----------------------------------

#[test]
fn bm08_golden_matches_in_process_rederivation() {
    assert_golden_matches(BM08, "bm08.render.json");
}

#[test]
fn kit2_led_cube_golden_matches_in_process_rederivation() {
    assert_golden_matches(KIT2, "kit2-led-cube_led_cube.render.json");
}

#[test]
fn e1_ripup_golden_matches_in_process_rederivation() {
    assert_golden_matches(E1_RIPUP, "e1_ripup.render.json");
}

// --- PIN 2 — layer-visibility exclusion -----------------------------

/// A crafted two-trace + one-pad + outline snapshot (layers 0/1) for
/// the EXACT by-construction exclusion proof: with layer-ascending
/// emission, hiding a layer removes exactly that layer's op group —
/// a PREFIX (lower layer) or SUFFIX (higher layer) of the full op
/// list.
fn two_layer_snapshot() -> BoardSnapshot {
    let point = |x: i64, y: i64| epic_engine::snapshot::PointPrimitive { x, y };
    let trace = |layer: i32, y: i64| epic_engine::snapshot::TracePrimitive {
        polyline_points: vec![point(0, y), point(500, y)],
        layer,
        half_width: 50,
        net: 1,
    };
    BoardSnapshot {
        traces: vec![trace(0, 0), trace(1, 500)],
        vias: Vec::new(),
        pads: vec![epic_engine::snapshot::PadPrimitive {
            outline_points: vec![
                point(200, 200),
                point(300, 200),
                point(300, 300),
                point(200, 300),
            ],
            layer: 0,
            net: 1,
        }],
        areas: Vec::new(),
        outline: vec![
            point(0, 0),
            point(1000, 0),
            point(1000, 1000),
            point(0, 1000),
        ],
        nets: vec![epic_engine::snapshot::NetInfo {
            id: 1,
            name: "n".to_string(),
        }],
        bounds: epic_engine::snapshot::BoxPrimitive {
            ll_x: 0,
            ll_y: 0,
            ur_x: 1000,
            ur_y: 1000,
        },
        revision: 0,
        overlays: epic_engine::snapshot::OverlayData::default(),
    }
}

/// A view over `snapshot` with exactly `layers` visible (zoom 1:1,
/// pan at the origin — the crafted boards' coordinates ARE screen px).
fn crafted_view(snapshot: &BoardSnapshot, layers: &[i32]) -> ViewModel {
    let mut view = ViewModel::new(fixed_test_transform(snapshot));
    view.visible_layers = layers.iter().copied().collect();
    view
}

#[test]
fn layer_visibility_exclusion_by_construction() {
    let snapshot = two_layer_snapshot();
    let all = project(
        &snapshot,
        &crafted_view(&snapshot, &[0, 1]),
        full_viewport(),
    );
    // The full face: outline (2) + pad (2) + two trace groups (4+4).
    assert_eq!(all.ops.len(), 12, "the crafted full render has 12 ops");

    // Hide the LOWER layer: layer 0's groups (pad + trace) are gone;
    // the remainder is the outline pair plus the layer-1 trace group
    // (an exact splice of the full list — zero ops reference hidden
    // items, by construction).
    let hide_zero = project(&snapshot, &crafted_view(&snapshot, &[1]), full_viewport());
    assert_eq!(hide_zero.ops.len(), 6, "hiding layer 0 drops 6 ops");
    let mut expected: Vec<epic_gui::render::RenderOp> = all.ops[..2].to_vec();
    expected.extend_from_slice(&all.ops[8..]);
    assert_eq!(
        render_json(&RenderList {
            ops: expected,
            culled: 0,
        }),
        render_json(&hide_zero),
        "the hidden-layer render keeps exactly the outline pair + the layer-1 group"
    );

    // Hide the HIGHER layer: the remainder is the exact PREFIX (the
    // layer-1 trace group was last — layer-ascending emission).
    let hide_one = project(&snapshot, &crafted_view(&snapshot, &[0]), full_viewport());
    assert_eq!(hide_one.ops.len(), 8, "hiding layer 1 drops 4 ops");
    assert_eq!(
        render_json(&RenderList {
            ops: all.ops[..8].to_vec(),
            culled: 0,
        }),
        render_json(&hide_one),
        "the hidden-layer render is the exact op prefix"
    );

    // The count moved on the REAL fixture too: bm08 minus layer 0 has
    // strictly fewer ops, and hiding EVERY layer leaves exactly the
    // layer-less outline group.
    let bm08 = load_snapshot(BM08);
    let bm08_view = golden_view(&bm08);
    let bm08_all = project(&bm08, &bm08_view.clone(), full_viewport());
    let mut minus_zero = bm08_view.clone();
    minus_zero.visible_layers.remove(&0);
    let bm08_minus = project(&bm08, &minus_zero, full_viewport());
    assert!(
        bm08_minus.ops.len() < bm08_all.ops.len(),
        "hiding a populated layer moves the op count"
    );
    let empty = project(&bm08, &crafted_view(&bm08, &[]), full_viewport());
    // The outline-only face pinned for real: hiding EVERY layer leaves
    // EXACTLY the layer-less outline group — one SetColor + one
    // FillPolygon and nothing else (a hidden-layer-leaking mutant
    // cannot pass a bare count).
    assert_eq!(
        empty.ops.len(),
        2,
        "hiding every layer renders exactly SetColor + FillPolygon for the outline"
    );
}

// --- PIN 2b — the per-layer trace color override (quality Q5, fix
// round t4-2) ------------------------------------------

/// The `ColorTable` per-layer face: a trace on layer 1 (which HAS a
/// `per_layer` entry) renders `SetColor` = [77,127,196] — the
/// override, NOT the class [200,52,52]; a trace on layer 2 (NO
/// entry) falls to the class color. Kills the override-dropped
/// mutant (`trace_color` -> always `self.trace`).
#[test]
fn trace_color_per_layer_override_face() {
    let point = |x: i64, y: i64| epic_engine::snapshot::PointPrimitive { x, y };
    let trace = |layer: i32, y: i64| epic_engine::snapshot::TracePrimitive {
        polyline_points: vec![point(0, y), point(500, y)],
        layer,
        half_width: 50,
        net: 1,
    };
    let snapshot = BoardSnapshot {
        traces: vec![trace(1, 0), trace(2, 500)],
        vias: Vec::new(),
        pads: Vec::new(),
        areas: Vec::new(),
        outline: Vec::new(),
        nets: vec![epic_engine::snapshot::NetInfo {
            id: 1,
            name: "n".to_string(),
        }],
        bounds: epic_engine::snapshot::BoxPrimitive {
            ll_x: 0,
            ll_y: 0,
            ur_x: 500,
            ur_y: 500,
        },
        revision: 0,
        overlays: epic_engine::snapshot::OverlayData::default(),
    };
    let render = project(
        &snapshot,
        &crafted_view(&snapshot, &[1, 2]),
        full_viewport(),
    );
    // Emission is layer-ascending: the layer-1 group is ops[0..4]
    // (SetColor, SetWidth, MoveTo, LineTo), the layer-2 group
    // ops[4..8].
    assert_eq!(render.ops.len(), 8, "two trace groups, 4 ops each");
    assert_eq!(
        render.ops[0],
        epic_gui::render::RenderOp::SetColor([77, 127, 196, 255]),
        "a trace on layer 1 uses the per-layer OVERRIDE color"
    );
    assert_eq!(
        render.ops[4],
        epic_gui::render::RenderOp::SetColor([200, 52, 52, 255]),
        "a trace on layer 2 (no per_layer entry) falls to the CLASS color"
    );
}

// --- PIN 3 — the cull boundary ±1 (DNR-16) ---------------------------

#[test]
fn cull_boundary_plus_minus_one_dbu() {
    let point = |x: i64, y: i64| epic_engine::snapshot::PointPrimitive { x, y };
    // One pad, square (1000,1000)-(1100,1100); nothing else.
    let snapshot = BoardSnapshot {
        traces: Vec::new(),
        vias: Vec::new(),
        pads: vec![epic_engine::snapshot::PadPrimitive {
            outline_points: vec![
                point(1000, 1000),
                point(1100, 1000),
                point(1100, 1100),
                point(1000, 1100),
            ],
            layer: 0,
            net: 1,
        }],
        areas: Vec::new(),
        outline: Vec::new(),
        nets: vec![epic_engine::snapshot::NetInfo {
            id: 1,
            name: "n".to_string(),
        }],
        bounds: epic_engine::snapshot::BoxPrimitive {
            ll_x: 1000,
            ll_y: 1000,
            ur_x: 1100,
            ur_y: 1100,
        },
        revision: 0,
        overlays: epic_engine::snapshot::OverlayData::default(),
    };
    let view = crafted_view(&snapshot, &[0]);

    // The viewport spans y fully; its ur_x edge moves across the
    // pad's ur_x = 1100.
    let viewport_at = |ur_x: i32| IntBox::new(IntPoint::new(0, 0), IntPoint::new(ur_x, 2000));

    // 1 DBU INSIDE the edge (the pad fully in, 1 DBU of margin):
    // kept.
    let inside = project(&snapshot, &view.clone(), viewport_at(1101));
    assert_eq!(
        (inside.ops.len(), inside.culled),
        (2, 0),
        "1-inside renders"
    );

    // EXACTLY ON the edge: the viewport's right edge at the pad's
    // ll_x — a zero-area touch. KEPT (the inclusive DNR-16 face).
    let on_edge = project(&snapshot, &view.clone(), viewport_at(1000));
    assert_eq!(
        (on_edge.ops.len(), on_edge.culled),
        (2, 0),
        "on-edge renders (inclusive)"
    );

    // 1 DBU OUTSIDE (the pad entirely beyond the edge): culled;
    // `culled` moves by exactly 1 and no ops remain.
    let outside = project(&snapshot, &view.clone(), viewport_at(999));
    assert_eq!(
        (outside.ops.len(), outside.culled),
        (0, 1),
        "1-outside is culled"
    );
}

// --- PIN 3b — the via mapping row (spec-review F1, fix round t4-1) --

/// A crafted one-via snapshot: two drill shapes on DIFFERENT layers
/// (padstack order — a small 80x80 SQUARE on layer 0, a wider 160x120
/// RECTANGLE on layer 1 — the rectangle is what pins the min(w,h)
/// radius discriminant, quality-review Q3: a min→max mutant changes
/// the expected radius), centered at (500, 500); nothing else renders
/// (no outline/pads/traces/areas), so the projection's op list is
/// EXACTLY the via's op sequence and every assertion below is
/// op-by-op.
///
/// Radius arithmetic at the 1:1 test transform (scale = 1): both
/// shapes visible ⇒ union bbox (420,420)-(580,540), w=160, h=120 ⇒
/// min(w,h) = 120 ⇒ radius = 120/4 = 30 DBU ⇒ 30 px (the documented
/// `max(1, min(w,h)/4 x scale)` face, the 1 px floor not reached; a
/// min→max mutant would compute 160/4 = 40 and die).
/// Only layer 0 visible ⇒ visible-union bbox (460,460)-(540,540),
/// min(w,h) = 80 ⇒ radius = 80/4 = 20 px.
fn one_via_snapshot() -> BoardSnapshot {
    let point = |x: i64, y: i64| epic_engine::snapshot::PointPrimitive { x, y };
    BoardSnapshot {
        traces: Vec::new(),
        vias: vec![epic_engine::snapshot::ViaPrimitive {
            center: point(500, 500),
            drill_shapes: vec![
                // Padstack index 0 — layer 0, the small square.
                vec![
                    point(460, 460),
                    point(540, 460),
                    point(540, 540),
                    point(460, 540),
                ],
                // Padstack index 1 — layer 1, the wider RECTANGLE
                // (160 x 120 — w != h pins the min face, Q3).
                vec![
                    point(420, 420),
                    point(580, 420),
                    point(580, 540),
                    point(420, 540),
                ],
            ],
            layers: vec![0, 1],
            net: 1,
        }],
        pads: Vec::new(),
        areas: Vec::new(),
        outline: Vec::new(),
        nets: vec![epic_engine::snapshot::NetInfo {
            id: 1,
            name: "n".to_string(),
        }],
        bounds: epic_engine::snapshot::BoxPrimitive {
            ll_x: 420,
            ll_y: 420,
            ur_x: 580,
            ur_y: 540,
        },
        revision: 0,
        overlays: epic_engine::snapshot::OverlayData::default(),
    }
}

/// The doc table's via row, asserted op-by-op: `SetColor(via)` →
/// `FillPolygon(pad rectangle)` per visible shape in PADSTACK order →
/// the drill `Circle` LAST. Kills the drop-Circle mutant (the exact
/// 4-op sequence fails), the Circle-before-pads mutant (same assert),
/// and the per-shape-bbox cull mutant (the union-overlap face keeps
/// the FULL 4-op sequence when the layer-1 shape sticks out of the
/// viewport).
#[test]
fn via_mapping_ops_match_the_doc_table() {
    let snapshot = one_via_snapshot();

    // Both layers visible, wide-open viewport: exactly the doc-table
    // op sequence (screen == world at the 1:1 pan-origin transform).
    let all = project(
        &snapshot,
        &crafted_view(&snapshot, &[0, 1]),
        full_viewport(),
    );
    let point = |x: i64, y: i64| epic_engine::snapshot::PointPrimitive { x, y };
    let shape0 = [
        point(460, 460),
        point(540, 460),
        point(540, 540),
        point(460, 540),
    ];
    let shape1 = [
        point(420, 420),
        point(580, 420),
        point(580, 540),
        point(420, 540),
    ];
    // Screen == world − pan (pan = bounds.ll = (420, 420)) at zoom
    // 1:1 — the expected literals below are the TRANSFORMED corners.
    let screen = |points: &[epic_engine::snapshot::PointPrimitive]| {
        points
            .iter()
            .map(|p| IntPoint::new((p.x - 420) as i32, (p.y - 420) as i32))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        all.ops,
        vec![
            epic_gui::render::RenderOp::SetColor([227, 183, 46, 255]),
            epic_gui::render::RenderOp::FillPolygon(screen(&shape0)), // (40,40)-(120,120)
            epic_gui::render::RenderOp::FillPolygon(screen(&shape1)), // (0,0)-(160,120)
            epic_gui::render::RenderOp::Circle {
                center: IntPoint::new(80, 80), // world (500,500) − pan
                radius: 30, // min(160,120)/4 x scale 1 — a min→max mutant computes 40 and dies
            },
        ],
        "the via emits SetColor -> FillPolygon per visible shape in padstack order -> the drill Circle LAST"
    );

    // UNION-bbox cull: the layer-0 shape is fully inside the viewport
    // while the layer-1 shape STICKS OUT past the right edge — the
    // union still overlaps, so the via renders its FULL 4-op sequence
    // (a per-shape-bbox cull mutant would drop the via or its
    // layer-1 polygon here).
    let sticking_out = IntBox::new(IntPoint::new(0, 0), IntPoint::new(560, 2000));
    let kept = project(&snapshot, &crafted_view(&snapshot, &[0, 1]), sticking_out);
    assert_eq!(
        kept.ops.len(),
        4,
        "the union bbox overlaps (layer-1 shape sticks out) — the via renders completely"
    );
    assert_eq!(kept.culled, 0, "a union overlap is a keep — nothing culled");

    // The same via fully ONE DBU outside: culled, no ops.
    let one_out = IntBox::new(IntPoint::new(0, 0), IntPoint::new(419, 2000));
    let culled_render = project(&snapshot, &crafted_view(&snapshot, &[0, 1]), one_out);
    assert_eq!(
        (culled_render.ops.len(), culled_render.culled),
        (0, 1),
        "the via 1-DBU outside the viewport is culled with no ops"
    );

    // ALL layers hidden: the via contributes NOTHING — no ops AND
    // `culled` unchanged (a visibility-skipped primitive never reaches
    // the precheck; the count-in-culled mutant dies here).
    let hidden = project(&snapshot, &crafted_view(&snapshot, &[]), full_viewport());
    assert_eq!(
        (hidden.ops.len(), hidden.culled),
        (0, 0),
        "an all-layers-hidden via contributes nothing and is NOT counted in culled"
    );

    // ONE layer hidden: per-shape visibility, not all-or-nothing —
    // only the layer-1 FillPolygon drops, and the drill radius
    // re-derives from the VISIBLE shapes' union (80/4 = 20 px).
    let half = project(&snapshot, &crafted_view(&snapshot, &[0]), full_viewport());
    assert_eq!(
        half.ops,
        vec![
            epic_gui::render::RenderOp::SetColor([227, 183, 46, 255]),
            epic_gui::render::RenderOp::FillPolygon(screen(&shape0)),
            epic_gui::render::RenderOp::Circle {
                center: IntPoint::new(80, 80),
                radius: 20, // min(80,80)/4 x scale 1 — the visible-union arithmetic
            },
        ],
        "hiding layer 1 drops exactly its FillPolygon and re-derives the drill radius"
    );
}

// --- PIN 3c — the trace half-width cull expansion (quality Q4, fix
// round t4-2) ------------------------------------------

/// A crafted trace whose CORNER bbox is fully OUTSIDE the viewport's
/// right edge while its half-width-EXPANDED bbox still reaches in:
/// corner bbox (1000,1000)-(1200,1000), half_width 51 => expanded
/// (949,949)-(1251,1051). The expanded-bbox face pinned BOTH
/// directions at ±1 (the DNR-16 discipline):
/// * right edge at 975 => the corner bbox alone says OUTSIDE
///   (1000 > 975) but the expansion touches (949 <= 975) => KEPT —
///   renders only BECAUSE of the expansion (the drop-expand mutant
///   culls here and dies);
/// * right edge at 949 => exactly ON the expanded edge => KEPT
///   (inclusive);
/// * right edge at 948 => 1 DBU beyond the expansion => CULLED,
///   `culled` moves by 1.
#[test]
fn trace_half_width_expansion_cull_face() {
    let point = |x: i64, y: i64| epic_engine::snapshot::PointPrimitive { x, y };
    let snapshot = BoardSnapshot {
        traces: vec![epic_engine::snapshot::TracePrimitive {
            polyline_points: vec![point(1000, 1000), point(1200, 1000)],
            layer: 0,
            half_width: 51,
            net: 1,
        }],
        vias: Vec::new(),
        pads: Vec::new(),
        areas: Vec::new(),
        outline: Vec::new(),
        nets: vec![epic_engine::snapshot::NetInfo {
            id: 1,
            name: "n".to_string(),
        }],
        bounds: epic_engine::snapshot::BoxPrimitive {
            ll_x: 949,
            ll_y: 949,
            ur_x: 1251,
            ur_y: 1051,
        },
        revision: 0,
        overlays: epic_engine::snapshot::OverlayData::default(),
    };
    let view = crafted_view(&snapshot, &[0]);
    let viewport = |ur_x: i32| IntBox::new(IntPoint::new(0, 0), IntPoint::new(ur_x, 5000));

    // Kept ONLY because of the expansion (corner bbox says outside).
    let kept = project(&snapshot, &view.clone(), viewport(975));
    assert_eq!(
        (kept.ops.len(), kept.culled),
        (4, 0),
        "the expansion reaches the viewport (949 <= 975) — the trace renders (SetColor, SetWidth, MoveTo, LineTo)"
    );

    // Exactly ON the expanded edge: KEPT (inclusive).
    let on_edge = project(&snapshot, &view.clone(), viewport(949));
    assert_eq!(
        (on_edge.ops.len(), on_edge.culled),
        (4, 0),
        "the viewport edge exactly ON the expanded bbox edge keeps the trace (inclusive)"
    );

    // 1 DBU beyond the expansion: CULLED; `culled` moves by exactly 1.
    let out = project(&snapshot, &view.clone(), viewport(948));
    assert_eq!(
        (out.ops.len(), out.culled),
        (0, 1),
        "1 DBU beyond the expanded bbox the trace is culled"
    );
}

// --- PIN 4 (REWRITTEN, M9-T5) — the overlay flags --------------------

/// A fully-populated crafted OVERLAY snapshot (empty geometry, all
/// four overlay faces populated) for the exact family-arithmetic
/// proof. Coordinates are screen px (pan = bounds.ll = (0,0), zoom
/// 1:1). Airlines: A from (0,0) to (5,0) — 5 px <= the 8 px dash-on,
/// renders SOLID (2 ops); B from (100,0) to (140,0) — 40 px, dashes
/// [0,8) on, [8,14) off, [14,22) on, [22,28) off, [28,36) on,
/// [36,40) off-tail => 3 dashes => 6 ops. Marker: center (3000,0),
/// depth 0 => radius max(4, min(4+0, 64)) = 4 (2 ops). Heatmap: one
/// 100x100 cell at (0,0), overflow 5, capacity 10 => ratio 0.5
/// (2 ops). Tuning: net 1, min 100, max 500, actual 250 => in-band,
/// anchored at airline A's from (6 ops).
fn populated_overlay_snapshot() -> BoardSnapshot {
    let point = |x: i64, y: i64| epic_engine::snapshot::PointPrimitive { x, y };
    let mut snapshot = two_layer_snapshot();
    snapshot.traces.clear();
    snapshot.pads.clear();
    snapshot.outline.clear();
    snapshot.overlays.airlines = vec![
        epic_engine::snapshot::AirLinePrimitive {
            from: point(0, 0),
            to: point(5, 0),
            net: 1,
        },
        epic_engine::snapshot::AirLinePrimitive {
            from: point(100, 0),
            to: point(140, 0),
            net: 1,
        },
    ];
    snapshot.overlays.violation_markers = vec![epic_engine::snapshot::ViolationMarker {
        center: point(3000, 0),
        depth: 0.0,
        pair_kind: (0, 2),
        phase: epic_engine::snapshot::MarkerPhase::Parse,
    }];
    snapshot.overlays.congestion = Some(epic_engine::snapshot::CongestionHeatmap {
        cell_size: 100,
        origin: point(0, 0),
        dims: (1, 1),
        cells: vec![epic_engine::snapshot::CongestionCell {
            ix: 0,
            iy: 0,
            signal_layer: 0,
            overflow: 5,
            capacity: 10,
        }],
    });
    snapshot.overlays.tuning = Some(vec![epic_engine::snapshot::NetTuningInfo {
        net: 1,
        min: 100.0,
        max: 500.0,
        actual: 250.0,
    }]);
    snapshot.bounds = epic_engine::snapshot::BoxPrimitive {
        ll_x: 0,
        ll_y: 0,
        ur_x: 3000,
        ur_y: 0,
    };
    snapshot
}

/// The ratsnest dash arithmetic at the crafted segment B (40 px):
/// [0,8) on, [8,14) off, [14,22) on, [22,28) off, [28,36) on,
/// [36,40) off-tail => 3 dashes => 6 ops.
#[test]
fn overlay_flags_off_is_inert_and_each_flag_adds_its_own_family() {
    // (a) ALL-OFF inertia: the flags-OFF e1_ripup projection is
    // byte-identical to the COMMITTED T4 golden body (the T4-trio
    // inertia pin 0 — the trio is NOT regenerated; flags-OFF emits
    // zero overlay ops by construction).
    let e1 = load_snapshot(E1_RIPUP);
    let e1_view = golden_view(&e1);
    let base = render_json(&project(&e1, &e1_view, full_viewport()));
    assert_eq!(
        base,
        read_golden("e1_ripup.render.json"),
        "flags-OFF output is byte-identical to the committed T4 golden"
    );

    // (b) the EMPTY-data face: a PURE snapshot carries the empty
    // OverlayData, so every single flag ON adds exactly ZERO ops.
    for name in ["ratsnest", "drc", "congestion", "tuning"] {
        let mut view = e1_view.clone();
        match name {
            "ratsnest" => view.overlays.ratsnest = true,
            "drc" => view.overlays.drc = true,
            "congestion" => view.overlays.congestion = true,
            _ => view.overlays.tuning = true,
        }
        let rendered = project(&e1, &view, full_viewport());
        assert_eq!(
            render_json(&rendered),
            base,
            "{name} ON over EMPTY overlay data adds no ops"
        );
    }

    // (c) the populated family arithmetic: each single flag ON adds
    // EXACTLY its own op family (count + variant sequence + colors).
    let snapshot = populated_overlay_snapshot();
    let view = crafted_view(&snapshot, &[0, 1]);
    let all_off = project(&snapshot, &view, full_viewport());
    assert_eq!(all_off.ops.len(), 0, "empty geometry + all flags OFF");

    // ratsnest ON: header (2) + airline A solid (2) + airline B
    // dashed (6) = 10.
    let mut ratsnest_view = view.clone();
    ratsnest_view.overlays.ratsnest = true;
    let ratsnest = project(&snapshot, &ratsnest_view, full_viewport());
    assert_eq!(ratsnest.ops.len(), 10, "the ratsnest family arithmetic");
    assert_eq!(
        ratsnest.ops[0],
        epic_gui::render::RenderOp::SetColor([255, 255, 255, 255]),
        "the family header colors the ratsnest class (Java INCOMPLETES white)"
    );
    assert_eq!(
        ratsnest.ops[1],
        epic_gui::render::RenderOp::SetWidth(1.0),
        "the family header width is Java's drawWidth 1"
    );
    assert_eq!(
        &ratsnest.ops[2..4],
        &[
            epic_gui::render::RenderOp::MoveTo(IntPoint::new(0, 0)),
            epic_gui::render::RenderOp::LineTo(IntPoint::new(5, 0)),
        ],
        "airline A (5 px) renders SOLID"
    );

    // drc ON: SetColor(violation) + Circle{r=4}.
    let mut drc_view = view.clone();
    drc_view.overlays.drc = true;
    let drc = project(&snapshot, &drc_view, full_viewport());
    assert_eq!(
        drc.ops,
        vec![
            epic_gui::render::RenderOp::SetColor([255, 0, 255, 255]),
            epic_gui::render::RenderOp::Circle {
                center: IntPoint::new(3000, 0),
                radius: 4,
            },
        ],
        "the drc family: SetColor(violation) + Circle at the documented radius"
    );

    // congestion ON: ramp(0.5) = level 8, t = 8/15, per-channel lerp
    // between [0,150,0] and [255,0,255] (the documented formula,
    // computed here from the SPEC, not the impl).
    let level: f64 = 8.0;
    let t = level / 15.0;
    let lerp = |a: u8, b: u8| (f64::from(a) + (f64::from(b) - f64::from(a)) * t).round() as u8;
    let expected_ramp = [
        lerp(0, 255), // red 0 -> 255
        lerp(150, 0), // green 150 -> 0 (magenta carries NO green)
        lerp(0, 255), // blue 0 -> 255
        255u8,
    ];
    let mut congestion_view = view.clone();
    congestion_view.overlays.congestion = true;
    let congestion = project(&snapshot, &congestion_view, full_viewport());
    assert_eq!(
        congestion.ops,
        vec![
            epic_gui::render::RenderOp::SetColor(expected_ramp),
            epic_gui::render::RenderOp::FillPolygon(vec![
                IntPoint::new(0, 0),
                IntPoint::new(100, 0),
                IntPoint::new(100, 100),
                IntPoint::new(0, 100),
            ]),
        ],
        "the congestion family: ramp(0.5) color + the cell rect corners (ll, lr, ur, ul)\
         — world (0,0)-(100,100), screen == world at the 1:1 transform"
    );

    // tuning ON: in-band green bracket at airline A's from (0,0),
    // half-side 12: SetColor + MoveTo + 4 LineTo = 6 ops.
    let mut tuning_view = view.clone();
    tuning_view.overlays.tuning = true;
    let tuning = project(&snapshot, &tuning_view, full_viewport());
    assert_eq!(
        tuning.ops,
        vec![
            epic_gui::render::RenderOp::SetColor([0, 150, 0, 255]),
            epic_gui::render::RenderOp::MoveTo(IntPoint::new(-12, -12)),
            epic_gui::render::RenderOp::LineTo(IntPoint::new(12, -12)),
            epic_gui::render::RenderOp::LineTo(IntPoint::new(12, 12)),
            epic_gui::render::RenderOp::LineTo(IntPoint::new(-12, 12)),
            epic_gui::render::RenderOp::LineTo(IntPoint::new(-12, -12)),
        ],
        "the tuning family: in-band green closed bracket at the anchor"
    );

    // (d) determinism: all four ON twice => byte-equal.
    let mut all_on = view.clone();
    all_on.overlays = OverlayFlags {
        ratsnest: true,
        drc: true,
        congestion: true,
        tuning: true,
    };
    let one = project(&snapshot, &all_on, full_viewport());
    let two = project(&snapshot, &all_on, full_viewport());
    assert_eq!(
        render_json(&one),
        render_json(&two),
        "same flags twice => byte-equal (determinism)"
    );
    assert_eq!(one.ops.len(), 10 + 2 + 2 + 6, "the all-ON count");

    // (e) THE FAMILY ORDER (spec-review F3, mutant M4): the all-ON
    // projection must equal the four per-family renders (each
    // asserted op-exactly above) CONCATENATED in the chartered fixed
    // order ratsnest -> drc -> congestion -> tuning. The single-flag
    // renders are order-independent (each family emits alone), so an
    // order-permutation mutant (e.g. drc emitted before ratsnest)
    // fails THIS compare while the per-family asserts stay green.
    let mut expected: Vec<epic_gui::render::RenderOp> = Vec::new();
    for family in [&ratsnest, &drc, &congestion, &tuning] {
        expected.extend(family.ops.iter().cloned());
    }
    assert_eq!(one.ops, expected, "the chartered family order");
}
#[test]
fn overlay_flags_round_trip_through_the_view_model() {
    let snapshot = load_snapshot(E1_RIPUP);
    let mut view = golden_view(&snapshot);

    // Each flag carries through the ViewModel (the plumbing face T5
    // consumes) and the projection ACCEPTS the view at every state.
    type FlagSetter = (&'static str, fn(&mut OverlayFlags));
    let setters: [FlagSetter; 4] = [
        ("ratsnest", |f| f.ratsnest = true),
        ("drc", |f| f.drc = true),
        ("congestion", |f| f.congestion = true),
        ("tuning", |f| f.tuning = true),
    ];
    for (name, setter) in setters {
        setter(&mut view.overlays);
        assert!(
            matches!(
                (&view.overlays, name),
                (OverlayFlags { ratsnest: true, .. }, "ratsnest")
                    | (OverlayFlags { drc: true, .. }, "drc")
                    | (
                        OverlayFlags {
                            congestion: true,
                            ..
                        },
                        "congestion"
                    )
                    | (OverlayFlags { tuning: true, .. }, "tuning")
            ),
            "the {name} flag round-trips through the ViewModel"
        );
        let list = project(&snapshot, &view.clone(), full_viewport());
        assert!(
            !list.ops.is_empty(),
            "project accepts a view with overlays set"
        );
        view.overlays = OverlayFlags::default();
    }
}

// --- PIN 5 — the pan/zoom round trips --------------------------------

/// Samples a grid of world points across `snapshot`'s bounds (plus
/// the corners and the first pad's outline points — points across the
/// fixture's extent).
fn sample_world_points(
    snapshot: &BoardSnapshot,
    steps: usize,
) -> Vec<epic_engine::snapshot::PointPrimitive> {
    let mut points = Vec::new();
    let b = &snapshot.bounds;
    let w = b.ur_x - b.ll_x;
    let h = b.ur_y - b.ll_y;
    for i in 0..=steps {
        for j in 0..=steps {
            let x = b.ll_x + w * i as i64 / steps as i64;
            let y = b.ll_y + h * j as i64 / steps as i64;
            points.push(epic_engine::snapshot::PointPrimitive { x, y });
        }
    }
    if let Some(pad) = snapshot.pads.first() {
        points.extend(pad.outline_points.iter().copied());
    }
    points
}

#[test]
fn world_round_trip_exact_at_the_i64_faces() {
    let snapshot = load_snapshot(BM08);
    let samples = sample_world_points(&snapshot, 20);
    let pan = epic_engine::snapshot::PointPrimitive {
        x: snapshot.bounds.ll_x,
        y: snapshot.bounds.ll_y,
    };

    // The fixed test transform (1:1): FULL-DOMAIN exact — the golden
    // transform's own contract.
    let identity = fixed_test_transform(&snapshot);
    for p in &samples {
        assert_eq!(
            identity.screen_to_world(identity.world_to_screen(*p)),
            *p,
            "1:1 world round trip is exact at the i64 faces for {p:?}"
        );
    }

    // The integer px-per-DBU family (num % den == 0, here 7/1):
    // full-domain exact per the exactness contract.
    let zoom_in = ScreenTransform::new(pan, 7, 1);
    for p in &samples {
        assert_eq!(
            zoom_in.screen_to_world(zoom_in.world_to_screen(*p)),
            *p,
            "7/1 world round trip is exact at the i64 faces for {p:?}"
        );
    }

    // The fractional zoom (1/1000): exact ON the world lattice
    // {pan + k*1000}, and the off-lattice round trip is quantized
    // within ONE den per axis (the documented floor face).
    let zoom_out = ScreenTransform::new(pan, 1, 1000);
    for p in &samples {
        let round_tripped = zoom_out.screen_to_world(zoom_out.world_to_screen(*p));
        if (p.x - pan.x) % 1000 == 0 && (p.y - pan.y) % 1000 == 0 {
            assert_eq!(
                round_tripped, *p,
                "1/1000 world round trip is exact on the lattice for {p:?}"
            );
        } else {
            let dx = (round_tripped.x - p.x).abs();
            let dy = (round_tripped.y - p.y).abs();
            assert!(
                dx < 1000 && dy < 1000,
                "1/1000 off-lattice round trip quantizes within one den for {p:?} (d=({dx},{dy}))"
            );
        }
    }
}

#[test]
fn screen_round_trip_exact_at_the_i32_faces() {
    let snapshot = load_snapshot(BM08);
    let b = &snapshot.bounds;
    let pan = epic_engine::snapshot::PointPrimitive {
        x: b.ll_x,
        y: b.ll_y,
    };

    // Screen samples: a grid across the projected bounds (the 1:1
    // projection IS the bounds extents; the zoom faces scale it).
    let mut samples = Vec::new();
    let steps: i32 = 20;
    let w = i32::try_from(b.ur_x - b.ll_x).expect("bm08 fits i32");
    let h = i32::try_from(b.ur_y - b.ll_y).expect("bm08 fits i32");
    for i in 0..=steps {
        for j in 0..=steps {
            samples.push(IntPoint::new((w / steps) * i, (h / steps) * j));
        }
    }

    // The fixed test transform (1:1): bijective — FULL-DOMAIN exact.
    let identity = fixed_test_transform(&snapshot);
    for s in &samples {
        assert_eq!(
            identity.world_to_screen(identity.screen_to_world(*s)),
            *s,
            "1:1 screen round trip is exact at the i32 faces for {s:?}"
        );
    }

    // 1/1000 (den % num == 0): full-domain exact screen round trip.
    let zoom_out = ScreenTransform::new(pan, 1, 1000);
    for s in &samples {
        assert_eq!(
            zoom_out.world_to_screen(zoom_out.screen_to_world(*s)),
            *s,
            "1/1000 screen round trip is exact at the i32 faces for {s:?}"
        );
    }

    // 7/1: exact on the screen lattice {k*7}, quantized within one
    // num DBU otherwise (the documented floor face).
    let zoom_in = ScreenTransform::new(pan, 7, 1);
    for s in &samples {
        let round_tripped = zoom_in.world_to_screen(zoom_in.screen_to_world(*s));
        if s.x % 7 == 0 && s.y % 7 == 0 {
            assert_eq!(
                round_tripped, *s,
                "7/1 screen round trip is exact on the lattice for {s:?}"
            );
        } else {
            let dx = i64::from(round_tripped.x - s.x);
            let dy = i64::from(round_tripped.y - s.y);
            assert!(
                dx.abs() < 7 && dy.abs() < 7,
                "7/1 off-lattice screen round trip quantizes within one num for {s:?}"
            );
        }
    }
}

// --- PIN 5c — the narrowing saturation face (quality Q2, fix round
// t4-2) ------------------------------------------------

/// The documented saturating i32 narrowing (`view.rs`): at a 1:1
/// transform with pan (0,0), world x = i32::MAX + 1 saturates to
/// i32::MAX (the raw-cast mutant wraps to i32::MIN and dies); the MIN
/// side symmetric; both axes pinned. The i64-side narrowing (the
/// screen_to_world twin) is exercised at its range faces too.
#[test]
fn narrowing_saturates_at_the_i32_faces() {
    let zero = epic_engine::snapshot::PointPrimitive { x: 0, y: 0 };
    let t = ScreenTransform::new(zero, 1, 1);

    let overflow = epic_engine::snapshot::PointPrimitive {
        x: i64::from(i32::MAX) + 1,
        y: i64::from(i32::MIN) - 1,
    };
    let s = t.world_to_screen(overflow);
    assert_eq!(s.x, i32::MAX, "world x past i32::MAX saturates, not wraps");
    assert_eq!(s.y, i32::MIN, "world y past i32::MIN saturates, not wraps");

    // The i64-side narrowing: at 1:1 the round trip is exact, so the
    // extreme SCREEN points map back to themselves at the i64 faces.
    let w = t.screen_to_world(IntPoint::new(i32::MAX, i32::MIN));
    assert_eq!(w.x, i64::from(i32::MAX));
    assert_eq!(w.y, i64::from(i32::MIN));
}

// --- PIN 6 (M9-T5) — overlay cull participation ±1 (DNR-16) ----------

/// One 100-px airline (0,0)-(100,0), pan (0,0), zoom 1:1. The
/// viewport walks across the segment's ur_x edge BOTH directions:
/// with ll_x = w and a far ur, w = 100 touches the segment end =>
/// KEPT (inclusive), w = 101 leaves it entirely left => CULLED
/// (`culled` moves by exactly 1); symmetric on the ll side with
/// ur_x = 0 => KEPT, ur_x = -1 => CULLED. Kills the
/// overlays-don't-cull and exclusive-boundary mutants.
#[test]
fn overlay_airline_cull_participation_both_directions() {
    let point = |x: i64, y: i64| epic_engine::snapshot::PointPrimitive { x, y };
    let mut snapshot = two_layer_snapshot();
    snapshot.traces.clear();
    snapshot.pads.clear();
    snapshot.outline.clear();
    snapshot.areas.clear();
    snapshot.overlays.airlines = vec![epic_engine::snapshot::AirLinePrimitive {
        from: point(0, 0),
        to: point(100, 0),
        net: 1,
    }];
    snapshot.bounds = epic_engine::snapshot::BoxPrimitive {
        ll_x: 0,
        ll_y: 0,
        ur_x: 100,
        ur_y: 0,
    };
    let mut view = crafted_view(&snapshot, &[0, 1]);
    view.overlays.ratsnest = true;

    // Right side: the viewport's LEFT edge crosses the segment's
    // ur_x = 100.
    let kept_on = project(
        &snapshot,
        &view,
        IntBox::new(IntPoint::new(100, -10), IntPoint::new(2000, 10)),
    );
    assert_eq!(
        (kept_on.ops.len(), kept_on.culled),
        (18, 0),
        "the viewport edge exactly ON the segment end keeps it (inclusive): header (2)\
         + the 100-px dashed walk (8 on-dashes at 8/6 px = 16 ops)"
    );
    let culled_right = project(
        &snapshot,
        &view,
        IntBox::new(IntPoint::new(101, -10), IntPoint::new(2000, 10)),
    );
    assert_eq!(
        (culled_right.ops.len(), culled_right.culled),
        (0, 1),
        "1 DBU past the segment end it is culled, counted once"
    );

    // Left side: the viewport's RIGHT edge crosses the segment's
    // ll_x = 0.
    let kept_left = project(
        &snapshot,
        &view,
        IntBox::new(IntPoint::new(-2000, -10), IntPoint::new(0, 10)),
    );
    assert_eq!(
        (kept_left.ops.len(), kept_left.culled),
        (18, 0),
        "touching at the segment start keeps it (inclusive): the same 18 ops"
    );
    let culled_left = project(
        &snapshot,
        &view,
        IntBox::new(IntPoint::new(-2000, -10), IntPoint::new(-1, 10)),
    );
    assert_eq!(
        (culled_left.ops.len(), culled_left.culled),
        (0, 1),
        "1 DBU before the segment start it is culled, counted once"
    );
}

// --- The M9-T5 overlay goldens (NEW files; session-boundary
// snapshots, flags ON) -------------------------------------------------

/// The silent sink for the routed capture/verify faces (the
/// session/tee forwarding laws live in epic-engine's own pins — the
/// golden door only needs a `DriverSink` to hand `Session::route`).
struct SilentSink;

impl epic_router::pipeline::event_sink::DriverSink for SilentSink {}

/// The overlay-golden entries: (fixture, committed golden, flags,
/// boundary, viewport kind). `boundary`: Load = `snapshot_with_overlays`
/// pre-route (Parse markers); Route = route at defaults first;
/// CongestionOn = route with `SessionLayer.congestion_global = Some(true)`.
struct OverlayGoldenCase {
    fixture: &'static str,
    golden: &'static str,
    flags: OverlayFlags,
    boundary: Boundary,
    restricted_viewport: bool,
}

enum Boundary {
    Load,
    Route,
    CongestionOn,
}

const fn overlay_flags(ratsnest: bool, drc: bool, congestion: bool, tuning: bool) -> OverlayFlags {
    OverlayFlags {
        ratsnest,
        drc,
        congestion,
        tuning,
    }
}

/// The golden case set (the dispatch's minimum set 1-6):
const OVERLAY_GOLDEN_CASES: [OverlayGoldenCase; 6] = [
    // 1. airlines golden: LOAD boundary on bm08 (unrouted => airlines
    //    plentiful), ratsnest flag ON only.
    OverlayGoldenCase {
        fixture: BM08,
        golden: "bm08.overlay-airlines.json",
        flags: overlay_flags(true, false, false, false),
        boundary: Boundary::Load,
        restricted_viewport: false,
    },
    // 2. markers golden: the crafted DRC world (a probed >= 1
    //    violation bearer — the drc corpus craft), drc flag ON only,
    //    Parse phase. PARITY DECISION (upstream #935 pin-gap cap,
    //    freerouting@a917044ff): this is the one golden family that
    //    drives Session::load_dsn, so the cap fires here — P10's pad
    //    crosses the outline edge (minimumPinGap 0) and the
    //    outline x P10 marker depth collapses 250 -> 0 (radius 64 -> 4;
    //    count, centers, and both copper-copper rows unchanged).
    //    Upstream HEAD applies the same cap on the same board; golden
    //    re-captured same-commit through capture_overlay_goldens.
    //    SECOND PARITY DECISION (P3, upstream #925a clearance-shortfall
    //    tolerance, freerouting@14b28b6ff): the #935-collapsed
    //    outline x P10 row carries a rule cell of 0 (the
    //    copper-to-edge override writes board_edge cells for classes
    //    1 and up only) against a measured 0 — shortfall exactly
    //    0.0, which the STRICT gate drops at EVERY tolerance — so
    //    the marker leaves the set entirely (markers 3 -> 2; the two
    //    copper-copper trace rows unchanged). Upstream HEAD's gate
    //    behaves identically; golden re-captured same-commit through
    //    capture_overlay_goldens (the other five goldens byte-stable).
    OverlayGoldenCase {
        fixture: "harness/corpus/craft/drc-main.dsn",
        golden: "drc-craft.overlay-markers.json",
        flags: overlay_flags(false, true, false, false),
        boundary: Boundary::Load,
        restricted_viewport: false,
    },
    // 3. tuning golden: the crafted tuning fixture (constraints
    //    declared — probed), tuning flag ON only.
    OverlayGoldenCase {
        fixture: "harness/fixtures/tuning/min_stair_tuning.dsn",
        golden: "min-stair-tuning.overlay-tuning.json",
        flags: overlay_flags(false, false, false, true),
        boundary: Boundary::Load,
        restricted_viewport: false,
    },
    // 4. AM4(a) MANDATE: the restricted-viewport face (culled > 0) —
    //    the OBS-3/Q2 closure.
    OverlayGoldenCase {
        fixture: BM08,
        golden: "bm08.overlay-culled.json",
        flags: overlay_flags(true, false, false, false),
        boundary: Boundary::Load,
        restricted_viewport: true,
    },
    // 5. AM4(a) MANDATE: the routed face (post-route snapshot, all
    //    four flags ON) — the via-bearing + layer-1-trace closure
    //    (probed: bm08 routes to vias and B.Cu traces at defaults).
    OverlayGoldenCase {
        fixture: BM08,
        golden: "bm08.overlay-routed.json",
        flags: overlay_flags(true, true, false, true),
        boundary: Boundary::Route,
        restricted_viewport: false,
    },
    // 6. the congestion ON face (the reachable ON path:
    //    SessionLayer congestion_global), congestion flag ON only.
    OverlayGoldenCase {
        fixture: BM08,
        golden: "bm08.overlay-congestion.json",
        flags: overlay_flags(false, false, true, false),
        boundary: Boundary::CongestionOn,
        restricted_viewport: false,
    },
];

/// The restricted viewport for the culled face: the lower-left
/// quadrant of bm08's bounds, widened to i32 — a viewport that
/// provably culls (the golden header records `culled`).
fn restricted_viewport(snapshot: &BoardSnapshot) -> IntBox {
    let mid_x = snapshot.bounds.ll_x + (snapshot.bounds.ur_x - snapshot.bounds.ll_x) / 2;
    let mid_y = snapshot.bounds.ll_y + (snapshot.bounds.ur_y - snapshot.bounds.ll_y) / 2;
    let clamp = |v: i64| i32::try_from(v).unwrap_or(i32::MAX);
    IntBox::new(
        IntPoint::new(clamp(snapshot.bounds.ll_x), clamp(snapshot.bounds.ll_y)),
        IntPoint::new(clamp(mid_x), clamp(mid_y)),
    )
}

/// Derives the projection for one golden case (the shared face of
/// capture + verify).
fn project_overlay_case(case: &OverlayGoldenCase) -> (RenderList, BoardSnapshot) {
    let bytes = fs::read(repo_root().join(case.fixture))
        .unwrap_or_else(|error| panic!("{} is readable: {error}", case.fixture));
    let session_layer = match case.boundary {
        Boundary::CongestionOn => SessionLayer {
            congestion_global: Some(true),
            ..SessionLayer::default()
        },
        _ => SessionLayer::default(),
    };
    let mut session = match Session::load_dsn(&bytes, session_layer) {
        Ok(session) => session,
        Err(error) => panic!("{} loads at defaults: {error:?}", case.fixture),
    };
    match case.boundary {
        Boundary::Load => {}
        Boundary::Route | Boundary::CongestionOn => {
            session
                .route(&CliLayer::default(), &mut SilentSink)
                .expect("route succeeds");
        }
    }
    let snapshot = session.snapshot_with_overlays();
    let mut view = golden_view(&snapshot);
    view.overlays = case.flags;
    let viewport = if case.restricted_viewport {
        restricted_viewport(&snapshot)
    } else {
        full_viewport()
    };
    (project(&snapshot, &view, viewport), snapshot)
}

/// Every overlay golden verifies: in-process double derivation
/// byte-identical to each other AND to the committed bytes; the
/// structural asserts per case (culled > 0 on the culled face,
/// congestion Some on the ON face, >= 1 marker on the markers face).
#[test]
fn overlay_goldens_verify_in_process() {
    for case in &OVERLAY_GOLDEN_CASES {
        let (run_one, snapshot) = project_overlay_case(case);
        let (run_two, _) = project_overlay_case(case);
        assert_eq!(
            render_json(&run_one),
            render_json(&run_two),
            "{}: two projections of the same inputs are byte-identical",
            case.golden
        );
        let golden = read_golden(case.golden);
        assert!(!golden.is_empty(), "{} non-empty", case.golden);
        assert_eq!(
            render_json(&run_one),
            golden,
            "{}: the re-derivation is byte-identical to the committed golden",
            case.golden
        );
        match case.golden {
            "bm08.overlay-culled.json" => {
                assert!(run_one.culled > 0, "the restricted viewport CULLS");
            }
            "bm08.overlay-congestion.json" => {
                assert!(
                    snapshot.overlays.congestion.is_some(),
                    "the congestion ON face carries Some"
                );
            }
            "drc-craft.overlay-markers.json" => {
                assert!(
                    !snapshot.overlays.violation_markers.is_empty(),
                    "the markers golden carries >= 1 marker"
                );
            }
            "min-stair-tuning.overlay-tuning.json" => {
                assert!(
                    snapshot.overlays.tuning.is_some(),
                    "the tuning golden carries Some"
                );
            }
            _ => {}
        }
    }
}

// --- The sanctioned overlay capture door ------------------------------

/// Regenerates ALL THREE goldens in-process (the sanctioned capture
/// path: committed goldens are regenerated ONLY through this door,
/// and the regeneration reason goes in the commit). `#[ignore]`ged —
/// run with `cargo test -p epic-gui --test render_goldens -- --ignored
/// capture_render_goldens`.
#[test]
#[ignore = "the golden capture door — run explicitly with --ignored"]
fn capture_render_goldens() {
    for (fixture_rel, golden_rel) in [
        (BM08, "bm08.render.json"),
        (KIT2, "kit2-led-cube_led_cube.render.json"),
        (E1_RIPUP, "e1_ripup.render.json"),
    ] {
        let snapshot = load_snapshot(fixture_rel);
        let view = golden_view(&snapshot);
        let list = project(&snapshot, &view, full_viewport());
        let header = format!(
            "# fixture: {fixture_rel}\n\
             # snapshot revision: {rev}\n\
             # snapshot items: traces={traces} vias={vias} pads={pads} areas={areas} outline_pts={outline} nets={nets}\n\
             # transform: pan=({px}, {py}) zoom=1/1\n\
             # transform derivation: pan = snapshot.bounds.ll (the fixture's lower-left anchored at\n\
             #   the screen origin); zoom 1/1 is the unique px-per-DBU ratio whose world<->screen map\n\
             #   is a full bijection (both round trips exact on the entire in-range domain —\n\
             #   view.rs's exactness contract), chosen over the dispatch letter's 1:1000-ish example\n\
             #   because the round-trip pin demands world->screen->world exactness at the i64 faces.\n\
             #   The projected extents equal the fixture's DBU extents (all three fixtures probed\n\
             #   < 2^30; they fit i32 with margin and exercise both screen axes).\n\
             # viewport: the wide-open IntBox (ll=(i32::MIN,i32::MIN) ur=(i32::MAX,i32::MAX)); culled=0.\n\
             # format: serde_json::to_string_pretty of the RenderList; RenderOp externally tagged with\n\
             #   Rust variant spelling; points {{\"x\":i32,\"y\":i32}}; colors [r,g,b,a] u8; SetWidth f32 px.\n",
            rev = snapshot.revision,
            traces = snapshot.traces.len(),
            vias = snapshot.vias.len(),
            pads = snapshot.pads.len(),
            areas = snapshot.areas.len(),
            outline = snapshot.outline.len(),
            nets = snapshot.nets.len(),
            px = snapshot.bounds.ll_x,
            py = snapshot.bounds.ll_y,
        );
        let path = repo_root()
            .join("harness/fixtures/gui-render/golden")
            .join(golden_rel);
        fs::write(&path, format!("{header}{}\n", render_json(&list)))
            .unwrap_or_else(|error| panic!("write {path:?}: {error}"));
        println!(
            "captured {golden_rel}: ops={} culled={}",
            list.ops.len(),
            list.culled
        );
    }
}

/// Captures ALL SIX overlay goldens in-process (the sanctioned NEW-
/// goldens-only door; the T4 trio above is deliberately NOT touched —
/// flags-OFF output is byte-identical, pinned). `#[ignore]`ged — run
/// with `cargo test -p epic-gui --test render_goldens -- --ignored
/// capture_overlay_goldens`.
#[test]
#[ignore = "the overlay golden capture door — run explicitly with --ignored"]
fn capture_overlay_goldens() {
    for case in &OVERLAY_GOLDEN_CASES {
        let (list, snapshot) = project_overlay_case(case);
        let overlay_counts = format!(
            "airlines={} markers={} congestion={} tuning={}",
            snapshot.overlays.airlines.len(),
            snapshot.overlays.violation_markers.len(),
            snapshot
                .overlays
                .congestion
                .as_ref()
                .map(|heatmap| heatmap.cells.len())
                .map(|cells| format!("Some({cells} cells)",))
                .unwrap_or_else(|| "None".to_string()),
            snapshot
                .overlays
                .tuning
                .as_ref()
                .map(|infos| format!("Some({} infos)", infos.len()))
                .unwrap_or_else(|| "None".to_string()),
        );
        let (viewport_desc, culled_desc) = if case.restricted_viewport {
            let viewport = restricted_viewport(&snapshot);
            (
                format!(
                    "restricted (ll=({},{}) ur=({},{}))",
                    viewport.ll.x, viewport.ll.y, viewport.ur.x, viewport.ur.y
                ),
                format!("{}", list.culled),
            )
        } else {
            (
                "the wide-open IntBox (ll=(i32::MIN,i32::MIN) ur=(i32::MAX,i32::MAX))".to_string(),
                "0".to_string(),
            )
        };
        let header = format!(
            "# fixture: {fixture}\n\
             # boundary: {boundary}\n\
             # snapshot revision: {rev}\n\
             # snapshot items: traces={traces} vias={vias} pads={pads} areas={areas} outline_pts={outline} nets={nets}\n\
             # overlay counts: {overlay_counts}\n\
             # transform: pan=({px}, {py}) zoom=1/1 (the T4 derivation: 1/1 is the unique\n\
             #   full-bijection px-per-DBU ratio — view.rs's exactness contract; pan = bounds.ll)\n\
             # viewport: {viewport_desc}; culled={culled_desc}\n\
             # overlay families: fixed order ratsnest -> drc -> congestion -> tuning, each in wire\n\
             #   order; constants' derivations in render.rs's module-doc family table\n\
             # format: serde_json::to_string_pretty of the RenderList; RenderOp externally tagged with\n\
             #   Rust variant spelling; points {{\"x\":i32,\"y\":i32}}; colors [r,g,b,a] u8; SetWidth f32 px.\n",
            fixture = case.fixture,
            boundary = match case.boundary {
                Boundary::Load => "load (snapshot_with_overlays pre-route; markers carry Parse)",
                Boundary::Route =>
                    "post-route (route at defaults, then snapshot_with_overlays; markers carry PostRoute)",
                Boundary::CongestionOn =>
                    "congestion-on (route with SessionLayer.congestion_global = Some(true), then snapshot_with_overlays)",
            },
            rev = snapshot.revision,
            traces = snapshot.traces.len(),
            vias = snapshot.vias.len(),
            pads = snapshot.pads.len(),
            areas = snapshot.areas.len(),
            outline = snapshot.outline.len(),
            nets = snapshot.nets.len(),
            px = snapshot.bounds.ll_x,
            py = snapshot.bounds.ll_y,
            viewport_desc = viewport_desc,
            culled_desc = culled_desc,
        );
        let path = repo_root()
            .join("harness/fixtures/gui-render/golden")
            .join(case.golden);
        fs::write(&path, format!("{header}{}\n", render_json(&list)))
            .unwrap_or_else(|error| panic!("write {path:?}: {error}"));
        println!(
            "captured {}: ops={} culled={}",
            case.golden,
            list.ops.len(),
            list.culled
        );
    }
}
