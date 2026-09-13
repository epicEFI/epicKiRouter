//! The `(structure ...)` scope reader: the port of
//! `io/specctra/parser/Structure.java` (the layer/boundary/rules/clearance
//! halves — keepout, plane and control scopes are Task 6) plus the
//! `Rule.java` scope readers it drives.
//!
//! ## Java call shape
//!
//! `Structure.readScope` (`:936-1137`) loops over the scope body,
//! dispatching `(layer ...)`, `(boundary ...)`, `(rule ...)`, `(via ...)`
//! into [`BoardConstructionInfo`] (the Java private class, `:1294-1303`)
//! and the parse state; after the loop it builds the board once —
//! `if (getRoutingBoard() == null)` (`:1034-1036`, mirrored by
//! [`BoardSink::has_board`]) — via `createBoard` (`:1139-1286`): the T25
//! scale-factor loop, the T43 `offset(1000)` bounds, the outline transform
//! plus the `dimension() > 0` filter (T31), `separateHoles`, the
//! clearance-matrix default instance and `updateBoardRules`.
//!
//! ## Error discipline (Java crash parity)
//!
//! Java readers throw on some inputs where they merely return `false` on
//! others, and nothing between the reader and `readBoard` catches: every
//! throw is a hard parse failure. The port maps throws to `false` (the
//! [`read_scope`] contract) and `false` stays `false`:
//!
//! - `Rule.readScope` returning `null` (EOF inside the rule scope) hits
//!   `defaultRules.addAll(null)` (`Structure.java:982`) — NPE, parse dies.
//! - `nextDouble` returning `null` (`(width abc)` — an unparseable number
//!   reaches the reader as a String) NPEs on unboxing inside
//!   `readWidthRule`/`readClearanceRule` — parse dies.
//! - A `PolylinePath` boundary shape (`(polyline_path ...)`) makes
//!   `boundingBox()`/`transformToBoard` return `null`
//!   (`PolylinePath.java:56-72` warn stubs), NPE-ing in the
//!   `createBoard` union loop (`:1167`) or the outline transform
//!   (`:1222`) — parse dies (jar `/tmp/epic-t5-boundary-polyline`).
//! - A `Circle` boundary shape transforms to a `geometry.planar.Circle`,
//!   which is a `ConvexShape`, NOT a `PolylineShape`: the
//!   `(PolylineShape)` casts at `:1221`/`:1233` throw
//!   `ClassCastException` — parse dies.
//! - Everything else (bad layer type, missing outline, degenerate shapes)
//!   is a logged skip or a `false` return, never a throw.
//!
//! ## Documented divergences
//!
//! - EOF or a null token INSIDE the `(boundary` scope makes Java loop
//!   FOREVER (`readBoundaryScope` only breaks on `CLOSED_BRACKET`;
//!   empirically verified — the only `false` exit is the IOException
//!   handler `:294-296`). The port returns `false` instead of hanging.

use super::rule::{Rule, read_rule_scope};
use crate::coordinate_transform::{
    CoordinateTransform, calc_scale_factor, max_abs_scaled_coordinate,
};
use crate::keyword::{Keyword, skip_scope};
use crate::layer_structure::{Layer, LayerStructure};
use crate::lexer::{LexicalState, Scanner, Token};
use crate::shape::{BoardShape, PolygonPath, Shape};
use crate::sink::{
    AreaIr, BoardRulesIr, BoardSink, CreateBoardIr, FixedStateIr, ItemClassIr, KeepoutIr,
    KeepoutKindIr,
};
use crate::state::ParseState;
use epic_geometry::int_box::IntBox;
use epic_geometry::regular_tile_shape::RegularTileShape;
use epic_geometry::rounding::java_round;
use epic_geometry::tile_shape::TileShape;

/// Java `Structure.LayerRule` (`:1305-1314`): the rules of one
/// `(layer <name> (rule ...))` scope, remembered until the layers are
/// fully known.
#[derive(Clone, Debug, PartialEq)]
struct LayerRule {
    /// Java `layerName`.
    layer_name: String,
    /// Java `rule`.
    rules: Vec<Rule>,
}

/// Java `Structure.BoardConstructionInfo` (`:1294-1303`): the structure
/// scope content, accumulated while the scope body is read and consumed
/// by `createBoard` afterwards.
#[derive(Default)]
struct BoardConstructionInfo {
    /// Java `layerInfo`, file order.
    layer_info: Vec<Layer>,
    /// Java `boundingShape` — the FIRST pcb-layer boundary shape.
    bounding_shape: Option<Shape>,
    /// Java `outlineShapes` — every other boundary shape (path shapes on
    /// any layer, signal-layer shapes, surplus pcb shapes).
    outline_shapes: Vec<Shape>,
    /// Java `outlineClearanceClassName` — the boundary
    /// `(clearance_class ...)` value, `None` until one is seen.
    outline_clearance_class_name: Option<String>,
    /// Java `foundLayerCount` — the `no` handed to the next accepted
    /// layer.
    found_layer_count: i32,
    /// Java `defaultRules`.
    default_rules: Vec<Rule>,
    /// Java `layerDependentRules`.
    layer_dependent_rules: Vec<LayerRule>,
}

/// Java `Structure.readBoundaryScope` (`:273-304`). The FIRST shape is
/// read with a NULL layer structure (boundary shapes only understand
/// pcb/signal layers; `Shape.readScope(scanner, null)`); further shape
/// scopes append to the outline, `(clearance_class ...)` lands in
/// [`BoardConstructionInfo::outline_clearance_class_name`]. The Java
/// return value is DISCARDED at the dispatch site (`:970`) — and here.
fn read_boundary_scope(scanner: &mut Scanner, info: &mut BoardConstructionInfo) -> bool {
    // fully qualified: this module has its own top-level `read_scope`
    let current_shape = crate::shape::read_scope(scanner, None);
    let mut prev_token: Option<Token> = None;
    loop {
        let next_token = scanner.next_token();
        if matches!(next_token, Token::Eof | Token::Error(_)) {
            // Java INFINITE-LOOPS here (only CLOSED_BRACKET breaks; the
            // sole false exit is the IOException handler `:294-296`) —
            // documented divergence, the port returns false (module
            // docs, "Documented divergences").
            return false;
        }
        if next_token == Token::Close {
            break;
        }
        if prev_token == Some(Token::Open) {
            if next_token == Token::Keyword(Keyword::ClearanceClass) {
                info.outline_clearance_class_name = Some(read_string_scope(scanner));
            } else {
                let additional_shape =
                    crate::shape::read_scope_from_keyword(scanner, &next_token, None);
                add_boundary_shape(info, additional_shape);
            }
        }
        prev_token = Some(next_token);
    }
    match current_shape {
        None => {
            // Java warns "shape is null" and returns TRUE (`:297-301`) —
            // an empty boundary scope is swallowed without a shape.
            true
        }
        Some(shape) => {
            add_boundary_shape(info, Some(shape));
            true
        }
    }
}

/// Java `DsnFile.readStringScope` (`DsnFile.java:190-214`): the value of a
/// one-string scope such as `(clearance_class <name>)`, tolerant of a
/// missing closing bracket (drains to the bracket; a null string scans as
/// `""`, which the only Task 5 caller stores verbatim).
fn read_string_scope(scanner: &mut Scanner) -> String {
    let result = scanner.next_string_with(true, b' ');
    let mut next_token = scanner.next_token();
    if next_token != Token::Close {
        // Java warns "closing bracket expected" and drains to the bracket.
        while !matches!(next_token, Token::Eof | Token::Error(_)) {
            next_token = scanner.next_token();
            if next_token == Token::Close {
                break;
            }
        }
    }
    result
}

/// Java `Structure.addBoundaryShape` (`:306-325`): path shapes
/// (`PolylinePath`/`PolygonPath`) are outline shapes on ANY layer; a
/// pcb-layer shape becomes the bounding shape (only the FIRST — later pcb
/// shapes are outline shapes); a signal-layer shape is an outline shape;
/// anything else is dropped with a log-only warning. The layer comparison
/// is the NAME (`Layer.PCB`/`Layer.SIGNAL` are the parser's interning
/// constants).
fn add_boundary_shape(info: &mut BoardConstructionInfo, shape: Option<Shape>) {
    let Some(shape) = shape else {
        return;
    };
    if matches!(shape, Shape::PolylinePath(_) | Shape::PolygonPath(_)) {
        info.outline_shapes.push(shape);
        return;
    }
    let layer_name = match &shape {
        Shape::Rectangle(rectangle) => rectangle.layer.name.as_str(),
        Shape::Polygon(polygon) => polygon.layer.name.as_str(),
        Shape::Circle(circle) => circle.layer.name.as_str(),
        // handled above
        Shape::PolylinePath(_) | Shape::PolygonPath(_) => unreachable!("path shapes handled above"),
    };
    if layer_name == Layer::PCB_NAME {
        if info.bounding_shape.is_none() {
            info.bounding_shape = Some(shape);
        } else {
            info.outline_shapes.push(shape);
        }
    } else if layer_name == Layer::SIGNAL_NAME {
        info.outline_shapes.push(shape);
    }
}

/// Java `Structure.readLayerScope` (`:327-409`): one `(layer <name> ...)`
/// scope. Bug-compat notes:
///
/// - an unknown `(type ...)` sets `layerOk = false` — the layer is DROPPED
///   (`foundLayerCount` not incremented), but a LATER type-less
///   `(layer <same name> (rule ...))` re-adds it with the default
///   `isSignal = true` (jar `/tmp/epic-t5-boundary-d.out`: `Bad` dropped
///   by `(type flex)`, re-added by `(layer Bad (rule (width 900)))` as
///   layer 3);
/// - `(rule ...)` layer rules are remembered EVEN when `layerOk == false`
///   (`:374-376`, outside the `layerOk` guard);
/// - repeated layer NAMES are accepted: each accepted scope gets the next
///   `foundLayerCount`, so duplicates survive as separate layers (jar:
///   `B.Cu` twice in the layer table);
/// - `(use_net ...)` re-enters the NAME lexical state before EVERY token
///   (`:379`), so every word — keywords included — scans as a plain
///   string until the closing bracket;
/// - `(property ...)` and friends are skipped generically (`:393-395`,
///   trap T46).
fn read_layer_scope(scanner: &mut Scanner, info: &mut BoardConstructionInfo) -> bool {
    let mut layer_ok = true;
    let mut is_signal = true;

    let layer_string = scanner.next_string();

    let mut net_names = Vec::new();
    let mut next_token = scanner.next_token();
    while next_token != Token::Close {
        if next_token != Token::Open {
            // Java also lands here on EOF (`null != OPEN`) and warns
            // (`:340-344`); both are parse failures.
            return false;
        }
        let keyword = scanner.next_token();
        match keyword {
            Token::Keyword(Keyword::Type) => {
                let type_token = scanner.next_token();
                match type_token {
                    Token::Keyword(Keyword::Power) => is_signal = false,
                    Token::Keyword(Keyword::Signal) => {}
                    // Java: `nextToken != SIGNAL && !toString().equals("jumper")`
                    // — "jumper" is accepted as a keyword OR a plain string
                    // (`:350-357`).
                    Token::Str(s) if &*s == "jumper" => {}
                    _ => layer_ok = false,
                }
                if scanner.next_token() != Token::Close {
                    return false;
                }
            }
            Token::Keyword(Keyword::Rule) => {
                match read_rule_scope(scanner) {
                    // Java `:375-376`: a null readScope NPEs (fatal).
                    Some(rules) => info.layer_dependent_rules.push(LayerRule {
                        layer_name: layer_string.clone(),
                        rules,
                    }),
                    None => return false,
                }
            }
            Token::Keyword(Keyword::UseNet) => {
                // Java `:379`: yybegin(NAME) before EVERY token, so the
                // net names — keywords included — all scan as strings.
                loop {
                    scanner.set_lexical_state(LexicalState::Name);
                    let net_token = scanner.next_token();
                    if net_token == Token::Close {
                        break;
                    }
                    if let Token::Str(name) = net_token {
                        net_names.push(name.to_string());
                    }
                }
            }
            _ => {
                skip_scope(scanner);
            }
        }
        next_token = scanner.next_token();
    }
    if layer_ok {
        info.layer_info.push(Layer::with_net_names(
            &layer_string,
            info.found_layer_count,
            is_signal,
            net_names,
        ));
        info.found_layer_count += 1;
    }
    true
}

/// Java `Structure.readViaPadstacks` (`:411-444`): the `(via <names>)`
/// scope; `(spare ...)` recurses and its list REPLACES then APPENDS at the
/// end (`:437-443`). `None` mirrors the Java `null` return (EOF/error
/// token, non-string entry) — the dispatch assigns it verbatim (`:980`),
/// so a failed read clears any earlier list.
fn read_via_padstacks(scanner: &mut Scanner) -> Option<Vec<String>> {
    let mut normal_vias = Vec::new();
    let mut spare_vias = Vec::new();
    loop {
        let next_token = scanner.next_token();
        if matches!(next_token, Token::Eof | Token::Error(_)) {
            return None;
        }
        if next_token == Token::Close {
            break;
        }
        if next_token == Token::Open {
            if scanner.next_token() == Token::Keyword(Keyword::Spare) {
                match read_via_padstacks(scanner) {
                    Some(vias) => spare_vias = vias,
                    None => return None,
                }
            } else {
                skip_scope(scanner);
            }
        } else if let Token::Str(name) = next_token {
            normal_vias.push(name.to_string());
        } else {
            // Java warns "padstack name expected" and returns null.
            return None;
        }
    }
    normal_vias.extend(spare_vias);
    Some(normal_vias)
}

/// Java `Structure.readScope` (`:936-1137`): the `(structure ...)` body
/// loop + the post-loop board construction. `state` collects the parse
/// state (`layerStructure`, `coordinateTransform`, `viaPadstackNames`,
/// `boardOutlineOk`), `sink` receives the board.
///
/// Task 6 stubs: the keepout/plane/control arms consume their scopes to
/// keep the scanner aligned, but store nothing — each carries the Java
/// line range it must grow into. `(autoroute_settings ...)` mirrors the
/// Java quirk of doing NOTHING AT ALL (not even reading) when the layer
/// structure already exists (`:1006-1011`).
pub fn read_scope(scanner: &mut Scanner, state: &mut ParseState, sink: &mut dyn BoardSink) -> bool {
    let mut info = BoardConstructionInfo::default();

    let mut prev_token: Option<Token> = None;
    loop {
        let next_token = scanner.next_token();
        if matches!(next_token, Token::Eof | Token::Error(_)) {
            // Java: warn "structure scope incomplete" (`:956-961`) on the
            // null token; the Error token is the IOException family —
            // both `false`.
            return false;
        }
        if next_token == Token::Close {
            break;
        }
        let mut read_ok = true;
        if prev_token == Some(Token::Open) {
            match next_token {
                Token::Keyword(Keyword::Boundary) => {
                    // the return value is discarded (`:970`)
                    read_boundary_scope(scanner, &mut info);
                }
                Token::Keyword(Keyword::Layer) => {
                    read_ok = read_layer_scope(scanner, &mut info);
                    if state.layer_structure.is_some() {
                        // "correct the layerStructure because another layer
                        // is read" (`:975-978`)
                        state.layer_structure = Some(LayerStructure::new(info.layer_info.clone()));
                    }
                }
                Token::Keyword(Keyword::Via) => {
                    state.via_padstack_names = read_via_padstacks(scanner);
                }
                Token::Keyword(Keyword::Rule) => match read_rule_scope(scanner) {
                    Some(rules) => info.default_rules.extend(rules),
                    // Java `:982` addAll(null) NPE — crash parity.
                    None => return false,
                },
                // ---- Task 6 stubs (keepout/plane/control family) -------
                Token::Keyword(Keyword::Keepout)
                | Token::Keyword(Keyword::ViaKeepout)
                | Token::Keyword(Keyword::PlaceKeepout) => {
                    if state.layer_structure.is_none() {
                        state.layer_structure = Some(LayerStructure::new(info.layer_info.clone()));
                    }
                    // Java `:983-1000`: Shape.readAreaScope into
                    // keepoutList/viaKeepoutList/placeKeepoutList, inserted
                    // through insertKeepout after createBoard
                    // (`:1045-1065`).
                    skip_scope(scanner);
                }
                Token::Keyword(Keyword::Plane) => {
                    if state.layer_structure.is_none() {
                        state.layer_structure = Some(LayerStructure::new(info.layer_info.clone()));
                    }
                    // Java `:1001-1005`: Keyword.PLANE_SCOPE.readScope into
                    // planeList, inserted as conduction areas after
                    // createBoard (`:1067-1122`).
                    skip_scope(scanner);
                }
                Token::Keyword(Keyword::AutorouteSettings) => {
                    // Java `:1006-1011`: read ONLY when the layer structure
                    // is still null — otherwise the scope is left UNREAD
                    // (bug-compat; the scanner desyncs exactly like Java).
                    if state.layer_structure.is_none() {
                        state.layer_structure = Some(LayerStructure::new(info.layer_info.clone()));
                        // AutorouteSettings.readScope -> parse state
                        // (Task 9); consumed as a scope for now.
                        skip_scope(scanner);
                    }
                }
                Token::Keyword(Keyword::Control) => {
                    // Java `:1012-1013` readControlScope (`:446-476`,
                    // `(via_at_smd on|off)` into the parse state) — Task 6.
                    skip_scope(scanner);
                }
                Token::Keyword(Keyword::FlipStyle) => {
                    // Java `:1014-1015`
                    // PlaceControl.readFlipStyleRotateFirst — Task 6.
                    skip_scope(scanner);
                }
                Token::Keyword(Keyword::SnapAngle) => {
                    // Java `:1016-1021` readSnapAngle (`:478-508`) into the
                    // parse state — Task 6 wires it through; consumed for
                    // scanner alignment until then.
                    skip_scope(scanner);
                }
                _ => {
                    skip_scope(scanner);
                }
            }
        }
        if !read_ok {
            return false;
        }
        prev_token = Some(next_token);
    }

    // Post loop (`:1031-1040`): build the board once; a failed build (or
    // a sink that still has no board) is a parse failure.
    let mut result = true;
    if !sink.has_board() {
        result = create_board(state, &mut info, sink);
    }
    if !sink.has_board() {
        return false;
    }
    // Java `:1041-1134` (flip style, keepout/plane insertion,
    // insertMissingPowerPlanes, autoroute job) — Task 6/9.
    result
}

/// Java `Structure.createBoard` (`:1139-1286`). Emits [`CreateBoardIr`]
/// through the sink and inserts the outline holes as keepouts on every
/// layer (`:1279-1283`).
fn create_board(
    state: &mut ParseState,
    info: &mut BoardConstructionInfo,
    sink: &mut dyn BoardSink,
) -> bool {
    let layer_count = info.layer_info.len() as i32;
    if layer_count == 0 {
        // happens if no layers are defined (`:1142-1148`); the Java
        // warning is FRLogger-only — boardOutlineOk stays true, so the
        // Task 9 assembly classifies this as a parse error.
        return false;
    }
    if info.bounding_shape.is_none() {
        // happens if the boundary shape with layer pcb is missing
        if info.outline_shapes.is_empty() {
            state.board_outline_ok = false;
            return false;
        }
        // the union of the DSN-side bounding boxes (`:1152-1165`); a
        // PolylinePath boundingBox() is Java null -> NPE parity.
        let mut bounding_box = match info.outline_shapes[0].bounding_box() {
            Some(rectangle) => rectangle,
            None => return false,
        };
        for shape in &info.outline_shapes[1..] {
            let Some(rectangle) = shape.bounding_box() else {
                return false;
            };
            bounding_box = bounding_box.union(&rectangle);
        }
        info.bounding_shape = Some(Shape::Rectangle(bounding_box));
    }
    // `:1167`: the bounding shape's DSN box (a PolylinePath can never BE
    // the bounding shape — addBoundaryShape routes paths to the outline —
    // so the None arm is unreachable-in-Java crash parity).
    let Some(bounding_shape_box) = info.bounding_shape.as_ref().and_then(Shape::bounding_box)
    else {
        return false;
    };

    // per-layer number range check (`:1171-1179`)
    for layer in &info.layer_info {
        if layer.no < 0 || layer.no >= layer_count {
            return false;
        }
    }
    let board_layer_structure = LayerStructure::new(info.layer_info.clone());
    // `:1185`: the PARSER layer structure is set unconditionally, before
    // the scale factor exists.
    state.layer_structure = Some(LayerStructure::new(info.layer_info.clone()));

    // T25 scale-factor loop (`:1189-1203`)
    let max_coor = max_abs_scaled_coordinate(&bounding_shape_box.coor, state.resolution);
    if max_coor == 0.0 {
        state.board_outline_ok = false;
        return false;
    }
    let scale_factor = calc_scale_factor(state.resolution, max_coor);
    let coordinate_transform = CoordinateTransform::new(f64::from(scale_factor), 0.0, 0.0);
    state.coordinate_transform = Some(coordinate_transform);

    // bounds: the transformed bounding box + offset(1000) (T43,
    // `:1207-1208`). The bounding shape is a Rectangle, so the transform
    // result is always an IntBox (the mismatch arm mirrors the Java cast).
    let bounds_shape = Shape::Rectangle(bounding_shape_box);
    let bounds = match bounds_shape.transform_to_board(&coordinate_transform) {
        Some(BoardShape::Tile(TileShape::RegularTileShape(RegularTileShape::IntBox(r#box)))) => {
            r#box
        }
        _ => {
            // Java ClassCastException family — unreachable for a Rectangle.
            return false;
        }
    };
    let bounds = bounds.offset(1000.0);

    // outline loop (`:1210-1226`): PolygonPath width reset to 0 (the
    // offset used in transformToBoard is not implemented for non-convex
    // shapes), transform, keep dimension > 0 only (T31).
    let mut board_outline_shapes: Vec<BoardShape> = Vec::new();
    for current_shape in &info.outline_shapes {
        let shape = match current_shape {
            Shape::PolygonPath(path) if path.width != 0.0 => Shape::PolygonPath(PolygonPath {
                layer: path.layer.clone(),
                width: 0.0,
                coordinate_arr: path.coordinate_arr.clone(),
            }),
            other => other.clone(),
        };
        match shape.transform_to_board(&coordinate_transform) {
            // a Circle is a ConvexShape, NOT a PolylineShape: the Java
            // `(PolylineShape)` cast at `:1221` throws
            // ClassCastException — parse-fatal parity.
            Some(BoardShape::Circle(_)) => return false,
            Some(board_shape) => {
                if board_shape.dimension() > 0 {
                    board_outline_shapes.push(board_shape);
                }
            }
            // PolylinePath transformToBoard is Java null -> the cast
            // passes null and dimension() NPEs (`:1221-1223`) — crash
            // parity (jar /tmp/epic-t5-boundary-polyline).
            None => return false,
        }
    }
    if board_outline_shapes.is_empty() {
        // construct an outline from the bounding shape (`:1227-1234`) —
        // NO dimension filter on this fallback; the same
        // `(PolylineShape)` cast rejects circles.
        let Some(board_shape) = info
            .bounding_shape
            .as_ref()
            .and_then(|shape| shape.transform_to_board(&coordinate_transform))
        else {
            return false;
        };
        if matches!(board_shape, BoardShape::Circle(_)) {
            return false;
        }
        board_outline_shapes.push(board_shape);
    }

    // separateHoles MUTATES the outline list (`:1235`)
    let hole_shapes = separate_holes(&mut board_outline_shapes);

    // rules (`:1236-1237`, `:1266-1267`): default matrix + net-class
    // state, updateBoardRules, then the trace angle restriction.
    let mut rules = BoardRulesIr::new(board_layer_structure.layers.len());
    update_board_rules(state, info, &coordinate_transform, &mut rules);
    rules.trace_angle_restriction = state.snap_angle;

    sink.create_board(CreateBoardIr {
        bounding_box: bounds,
        layer_structure: board_layer_structure,
        outline_shapes: board_outline_shapes,
        outline_clearance_class: info.outline_clearance_class_name.clone(),
        rules,
        transform: coordinate_transform,
    });

    // insert the holes in the board outline as keepouts (`:1279-1283`):
    // clearance class 0 (the `null` class), SYSTEM_FIXED, one insertion
    // PER LAYER (jar `/tmp/epic-t5-boundary-a.out`: ids 2 and 3, class 0).
    let Ok(layer_slots) = usize::try_from(layer_count) else {
        return false;
    };
    for hole in &hole_shapes {
        for layer in 0..layer_slots {
            sink.insert_keepout(KeepoutIr {
                kind: KeepoutKindIr::Keepout,
                layer_no: layer as i32,
                area: AreaIr::simple(hole.clone()),
                clearance_class: 0,
                fixed: FixedStateIr::SystemFixed,
            });
        }
    }
    true
}

/// Java `Structure.updateBoardRules` (`:611-670`): default clearance
/// rules first (their `smd_to_turn_gap` result ORs into
/// `smdToTurnGapFound`), then default width rules, then the per-layer
/// rules (layer rules resolve through the PARSER layer structure — the
/// duplicate/`Bad` layers of `/tmp/epic-t5-boundary-d.out` resolve, the
/// dropped layer's number does not), and finally the pin-edge fallback
/// `setPinEdgeToTurnDist(getMinTraceHalfWidth())`.
fn update_board_rules(
    state: &ParseState,
    info: &BoardConstructionInfo,
    coordinate_transform: &CoordinateTransform,
    rules: &mut BoardRulesIr,
) {
    let mut smd_to_turn_gap_found = false;
    for rule in &info.default_rules {
        if let Rule::Clearance { value, pairs } = rule
            && set_clearance_rule(
                *value,
                pairs,
                -1,
                coordinate_transform,
                rules,
                &state.string_quote,
            )
        {
            smd_to_turn_gap_found = true;
        }
    }
    for rule in &info.default_rules {
        if let Rule::Width { value } = rule {
            let trace_half_width =
                java_round(coordinate_transform.dsn_to_board_value(*value) / 2.0) as i32;
            rules.set_default_trace_half_widths(trace_half_width);
        }
    }
    let Some(layer_structure) = &state.layer_structure else {
        // unreachable: create_board sets it before update_board_rules
        return;
    };
    for layer_rule in &info.layer_dependent_rules {
        let layer_index = layer_structure.get_no(&layer_rule.layer_name);
        if layer_index < 0 {
            continue;
        }
        for rule in &layer_rule.rules {
            match rule {
                Rule::Width { value } => {
                    let trace_half_width =
                        java_round(coordinate_transform.dsn_to_board_value(*value) / 2.0) as i32;
                    rules.set_default_trace_half_width(layer_index, trace_half_width);
                }
                Rule::Clearance { value, pairs } => {
                    // the smd_to_turn_gap result is IGNORED here
                    // (`:657-664`)
                    set_clearance_rule(
                        *value,
                        pairs,
                        layer_index,
                        coordinate_transform,
                        rules,
                        &state.string_quote,
                    );
                }
            }
        }
    }
    if !smd_to_turn_gap_found {
        rules.set_pin_edge_to_turn_dist(f64::from(rules.get_min_trace_half_width()));
    }
}

/// Java `Structure.setClearanceRule` (`:676-808`). Returns whether the
/// string `smd_to_turn_gap` was found. The per-string pair syntax
/// (trap T33):
///
/// - a two-element `(type a-b)` list re-derives the SAME pair on EVERY
///   loop iteration; each iteration strips quotes off both elements and
///   strips ONE leading `_` off element 1 — the strip runs inside the
///   `i in 0..2` loop, so it fires up to TWICE (`c-__d` -> `c`, `d`);
/// - a single element starting with the string quote is split at the
///   second quote, requiring `_` after it (`"a"_b` syntax);
/// - any other single element splits at the FIRST `_` (limit 2):
///   `c__d` -> `c` + `_d` (one underscore SURVIVES — jar), `_x` -> an
///   EMPTY-NAME class + `x`, `smd_via_same_net` -> `smd` +
///   `via_same_net` (the "not implemented" comment at `:744` is STALE —
///   the code creates the classes), `nosplit` -> silently skipped.
///
/// `wire` resolves to class 1; every other missing name is appended
/// (which also retargets the default item class for via/pin/smd/area).
fn set_clearance_rule(
    value: f64,
    pairs: &[String],
    layer_index: i32,
    coordinate_transform: &CoordinateTransform,
    rules: &mut BoardRulesIr,
    string_quote: &str,
) -> bool {
    let mut result = false;
    let current_clearance = java_round(coordinate_transform.dsn_to_board_value(value)) as i32;
    if pairs.is_empty() {
        // no type scope: the default value entry (`:695-701`); the return
        // is ALWAYS false.
        if layer_index < 0 {
            rules
                .clearance
                .set_default_value_all_layers(current_clearance);
        } else {
            rules
                .clearance
                .set_default_value(layer_index, current_clearance);
        }
        return result;
    }
    if contains_wire_clearance_pair(pairs) {
        create_default_clearance_classes(rules);
    }
    for current_string in pairs {
        if crate::sink::eq_ignore_case(current_string, "smd_to_turn_gap") {
            rules.set_pin_edge_to_turn_dist(f64::from(current_clearance));
            result = true;
            continue;
        }
        // Java `split(regex, 2)` regex semantics; exact for the quote
        // characters DSN files use (the quote is `"` in every known
        // corpus; a regex-metacharacter quote would diverge).
        let current_pair: [String; 2];
        if pairs.len() == 2 {
            // the pair is re-derived from the FULL list on every loop
            // iteration (`:718-728`); the underscore strip runs up to
            // TWICE.
            let mut pair = [pairs[0].clone(), pairs[1].clone()];
            for i in 0..2 {
                pair[i] = pair[i].replace('"', "");
                if pair[1].starts_with('_') {
                    pair[1] = pair[1][1..].to_string();
                }
            }
            current_pair = pair;
        } else if let Some(stripped) = current_string.strip_prefix(string_quote) {
            // split at the second occurrence of the string quote
            // (`:731-740`)
            let split: Vec<&str> = stripped.splitn(2, string_quote).collect();
            if split.len() != 2 || !split[1].starts_with('_') {
                // two log-only warnings (`:734-738`)
                continue;
            }
            current_pair = [split[0].to_string(), split[1][1..].to_string()];
        } else {
            // split at the first "_" (`:741-752`)
            let split: Vec<&str> = current_string.splitn(2, '_').collect();
            if split.len() != 2 {
                continue;
            }
            current_pair = [split[0].to_string(), split[1].to_string()];
        }

        let mut first_class_no = if current_pair[0] == "wire" {
            1
        } else {
            rules.clearance.get_no(&current_pair[0])
        };
        if first_class_no < 0 {
            first_class_no = append_clearance_class(rules, &current_pair[0]);
        }
        let mut second_class_no = if current_pair[1] == "wire" {
            1
        } else {
            rules.clearance.get_no(&current_pair[1])
        };
        if second_class_no < 0 {
            second_class_no = append_clearance_class(rules, &current_pair[1]);
        }
        if layer_index < 0 {
            rules.clearance.set_value_all_layers(
                first_class_no,
                second_class_no,
                current_clearance,
            );
            rules.clearance.set_value_all_layers(
                second_class_no,
                first_class_no,
                current_clearance,
            );
        } else {
            rules.clearance.set_value(
                first_class_no,
                second_class_no,
                layer_index,
                current_clearance,
            );
            rules.clearance.set_value(
                second_class_no,
                first_class_no,
                layer_index,
                current_clearance,
            );
        }
    }
    result
}

/// Java `Structure.containsWireClearancePair` (`:810-817`): the RAW
/// strings, before any splitting — a `wire_` prefix or `_wire` suffix
/// triggers the default class creation.
fn contains_wire_clearance_pair(pairs: &[String]) -> bool {
    pairs
        .iter()
        .any(|pair| pair.starts_with("wire_") || pair.ends_with("_wire"))
}

/// Java `Structure.createDefaultClearanceClasses` (`:819-824`): via, smd,
/// pin, area — IN THIS ORDER.
fn create_default_clearance_classes(rules: &mut BoardRulesIr) {
    append_clearance_class(rules, "via");
    append_clearance_class(rules, "smd");
    append_clearance_class(rules, "pin");
    append_clearance_class(rules, "area");
}

/// Java `Structure.appendClearanceClass` (`:826-840`): append (dedup
/// inside `appendClass`), then retarget the DEFAULT NET CLASS item class
/// for the four supported names (the Java switch is case-SENSITIVE).
fn append_clearance_class(rules: &mut BoardRulesIr, name: &str) -> i32 {
    rules.clearance.append_class(name);
    let result = rules.clearance.get_no(name);
    let item_class = match name {
        "via" => ItemClassIr::Via,
        "pin" => ItemClassIr::Pin,
        "smd" => ItemClassIr::Smd,
        "area" => ItemClassIr::Area,
        _ => return result,
    };
    rules.default_item_clearance_classes[item_class as usize] = result;
    result
}

/// Java `Structure.separateHoles` (`:577-605`) + the `OutlineShape`
/// helper (`:1318-1353`): a shape is a hole when some non-hole shape's
/// bounding box CONTAINS its bounding box and all of its corners land in
/// the other shape's convex decomposition. Bug-compat notes:
///
/// - the flag is ASSIGNED (`currentShape.isHole = ...`), not OR-ed — a
///   later non-containing pair RESETS an earlier hole verdict;
/// - the convex split can over-cover a concave shape (jar
///   `/tmp/epic-t5-holeprobe.out`: the L's second tile spans the full
///   right column, so the notch square "is contained" and becomes a
///   hole);
/// - `shapes[j].isHole` is read LIVE: the verdicts of earlier outer
///   iterations gate the inner scan.
fn separate_holes(outline_shapes: &mut Vec<BoardShape>) -> Vec<BoardShape> {
    let mut shapes: Vec<OutlineShape> = outline_shapes
        .iter()
        .map(|shape| OutlineShape::new(shape.clone()))
        .collect();
    for i in 0..shapes.len() {
        for j in 0..shapes.len() {
            // check if shapes[i] may be contained in shapes[j]
            if i == j || shapes[j].is_hole {
                continue;
            }
            if !shapes[i]
                .bounding_box
                .is_contained_in(&shapes[j].bounding_box)
            {
                continue;
            }
            shapes[i].is_hole = shapes[j].contains_all_corners(&shapes[i]);
        }
    }
    let mut hole_list = Vec::new();
    let mut kept = Vec::new();
    for shape in shapes {
        if shape.is_hole {
            hole_list.push(shape.shape);
        } else {
            kept.push(shape.shape);
        }
    }
    *outline_shapes = kept;
    hole_list
}

/// Java `Structure.OutlineShape` (`:1317-1352`): a transformed outline
/// shape (always a `PolylineShape` — the createBoard casts reject
/// circles) with its bounding box, convex decomposition and hole flag.
struct OutlineShape {
    shape: BoardShape,
    bounding_box: IntBox,
    convex_shapes: Option<Vec<TileShape>>,
    is_hole: bool,
}

impl OutlineShape {
    fn new(shape: BoardShape) -> Self {
        let bounding_box = shape.bounding_box();
        let convex_shapes = shape.split_to_convex();
        Self {
            shape,
            bounding_box,
            convex_shapes,
            is_hole: false,
        }
    }

    /// Java `containsAllCorners` (`:1332-1350`): `false` when the convex
    /// split failed (Java null); otherwise every `corner(i)` of the other
    /// shape (`i < borderLineCount()`) must land in SOME tile.
    fn contains_all_corners(&self, other: &OutlineShape) -> bool {
        let Some(convex_shapes) = &self.convex_shapes else {
            return false;
        };
        for i in 0..other.shape.border_line_count() {
            let current_corner = other.shape.corner(i);
            if !convex_shapes
                .iter()
                .any(|tile| tile.contains_point(&current_corner))
            {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ses_board::SesBoard;
    use crate::state::AngleRestriction;

    /// Positions the scanner after the `(structure` opener (the dispatch
    /// site consumes the OPEN bracket + the keyword before `read_scope`)
    /// and sets the parse state the jar fixtures resolve to
    /// (`(resolution um 10)` -> scale 10). Returns the reader triple.
    fn run_structure(body: &str) -> (bool, ParseState, SesBoard) {
        let input = format!("(structure {body})");
        let mut scanner = Scanner::new(input.as_bytes());
        assert_eq!(scanner.next_token(), Token::Open);
        assert_eq!(scanner.next_token(), Token::Keyword(Keyword::Structure));
        let mut state = ParseState::default();
        state.unit = crate::state::Unit::Um;
        state.resolution = 10;
        let mut board = SesBoard::new();
        let ok = read_scope(&mut scanner, &mut state, &mut board);
        (ok, state, board)
    }

    /// Jar `/tmp/epic-t5-probe.out` FILE 1 (`/tmp/epic-t5-clearance.dsn`):
    /// the full rules battery. Pins T33 (all pair syntaxes), the
    /// `smd_to_turn_gap` pin edge, the `wire` class-1 alias with the
    /// default class creation, the stale-comment `c__d` split, the
    /// `""a"_b` end-to-end lexer behavior, per-layer layer rules over a
    /// DUPLICATE layer name, the T43 bounds and the Issue558 outline
    /// class via the appended `area` class.
    #[test]
    fn clearance_rules_full_battery_jar_probe() {
        let (ok, state, board) = run_structure(
            "(layer F.Cu (type signal))\
             (layer B.Cu (type signal))\
             (boundary (rect pcb 0 0 5000 4000))\
             (rule\
               (clearance 500)\
               (clearance 900 (type power-ground))\
               (clearance 120 (type smd_to_turn_gap))\
               (clearance 1300 (type wire_signal))\
               (clearance 140 (type c__d))\
               (clearance 150 (type \"\"a\"_b))\
               (clearance 160 (type x_y))\
               (width 600))\
             (layer B.Cu (rule (width 800) (clearance 200 (type power-ground))))",
        );
        assert!(ok);
        assert!(state.board_outline_ok);
        // BOUNDS -1000 -1000 51000 41000 (T43: transform + offset(1000))
        let bounds = board.bounding_box.expect("bounds stored");
        assert_eq!(
            [bounds.ll.x, bounds.ll.y, bounds.ur.x, bounds.ur.y],
            [-1000, -1000, 51_000, 41_000]
        );
        // 3 layers: the duplicate B.Cu survives
        let layers = board.layers.as_ref().expect("layers");
        let names: Vec<&str> = layers.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["F.Cu", "B.Cu", "B.Cu"]);
        assert!(layers.layers.iter().all(|l| l.is_signal));
        // 15 classes in append order
        assert_eq!(
            board.rules.clearance.names,
            [
                "null", "default", "power", "ground", "via", "smd", "pin", "area", "signal", "c",
                "_d", "a\"", "b", "x", "y"
            ]
        );
        // matrix on all three layers (L1 differs only in (2,3))
        let expect: [([i32; 2], i32); 10] = [
            ([1, 1], 5000),
            ([2, 3], 9000),
            ([1, 2], 5000),
            ([2, 1], 5000),
            ([3, 1], 5000),
            ([1, 8], 13000),
            ([9, 10], 1400),
            ([11, 12], 1500),
            ([13, 14], 1600),
            ([4, 4], 5000),
        ];
        for layer in 0..3 {
            for (pair, value) in expect {
                let got = board.rules.clearance.get_value(pair[0], pair[1], layer);
                let want = if layer == 1 && pair == [2, 3] {
                    2000
                } else {
                    value
                };
                assert_eq!(got, want, "({},{}) on layer {}", pair[0], pair[1], layer);
            }
        }
        // default trace half widths + pin edge + angle
        assert_eq!(board.rules.default_trace_half_widths, [3000, 4000, 3000]);
        assert_eq!(board.rules.pin_edge_to_turn_dist, 1200.0);
        assert_eq!(
            board.rules.trace_angle_restriction,
            AngleRestriction::FortyfiveDegree
        );
        // outline: one shape (the transformed rect fallback — a single pcb
        // rect never separates holes), bbox 0 0 .. 50000 40000, class 7 =
        // the AREA item class after the wire rule appended `area`.
        assert_eq!(board.items.len(), 1);
        match &board.items[0] {
            crate::ses_board::ItemIr::BoardOutline { id, outline } => {
                assert_eq!(*id, 1);
                assert_eq!(outline.shapes.len(), 1);
                let bbox = outline.shapes[0].bounding_box();
                assert_eq!(
                    [bbox.ll.x, bbox.ll.y, bbox.ur.x, bbox.ur.y],
                    [0, 0, 50_000, 40_000]
                );
                assert_eq!(outline.clearance_class, 7);
                assert_eq!(outline.fixed, FixedStateIr::SystemFixed);
            }
            other => panic!("expected BoardOutline, got {other:?}"),
        }
    }

    /// Jar `/tmp/epic-t5-probe2.out` (`/tmp/epic-t5-clearance2.dsn`): the
    /// DOUBLE underscore strip (`c-__d` -> `c`,`d` via the two-element
    /// branch), the even-ize of odd board values (0.5 -> 6, 0.7 -> 8),
    /// the empty-name class (`_x`), the stale-comment `smd_via_same_net`
    /// class pair, and the silent `nosplit` skip.
    #[test]
    fn clearance_pair_syntax_variants_jar_probe() {
        let (ok, _state, board) = run_structure(
            "(layer F.Cu (type signal))\
             (layer B.Cu (type signal))\
             (boundary (rect pcb 0 0 5000 4000))\
             (rule\
               (clearance 0.5)\
               (clearance 170 (type c-__d))\
               (clearance 180 (type e-f))\
               (clearance 0.7 (type m-n))\
               (clearance 195 (type _x))\
               (clearance 196 (type smd_via_same_net))\
               (clearance 197 (type nosplit)))",
        );
        assert!(ok);
        assert_eq!(
            board.rules.clearance.names,
            [
                "null",
                "default",
                "c",
                "d",
                "e",
                "f",
                "m",
                "n",
                "",
                "x",
                "smd",
                "via_same_net"
            ]
        );
        assert_eq!(board.rules.clearance.get_value(1, 1, 0), 6);
        assert_eq!(board.rules.clearance.get_value(2, 3, 0), 1700);
        assert_eq!(board.rules.clearance.get_value(4, 5, 0), 1800);
        assert_eq!(board.rules.clearance.get_value(6, 7, 0), 8);
        assert_eq!(board.rules.clearance.get_value(8, 9, 0), 1950);
        // all-layers rules: layer 1 identical
        assert_eq!(board.rules.clearance.get_value(1, 1, 1), 6);
        assert_eq!(board.rules.clearance.get_value(2, 3, 1), 1700);
        // no smd_to_turn_gap -> the pin-edge fallback is
        // getMinTraceHalfWidth() = 100000 (no width rule touched it)
        assert_eq!(board.rules.pin_edge_to_turn_dist, 100000.0);
    }

    /// Jar `/tmp/epic-t5-boundary-a.out`: pcb rect bounding shape + two
    /// signal paths -> outer square outline, inner square HOLE (inserted
    /// per layer as keepout class 0 SYSTEM_FIXED, ids 2/3), the named
    /// `clearance_class` resolving to class 2 even though the class is
    /// created by a LATER rule scope.
    #[test]
    fn boundary_hole_and_named_clearance_class_jar_probe() {
        let (ok, _state, board) = run_structure(
            "(layer F.Cu (type signal))\
             (layer B.Cu (type signal))\
             (boundary\
               (rect pcb 0 0 5000 4000)\
               (path signal 0 1000 1000 2000 1000 2000 2000 1000 2000 1000 1000)\
               (path signal 0 1200 1200 1800 1200 1800 1800 1200 1800 1200 1200)\
               (clearance_class power))\
             (rule (clearance 900 (type power-ground)))",
        );
        assert!(ok);
        let layers = board.layers.as_ref().expect("layers");
        assert_eq!(layers.layers.len(), 2);
        assert_eq!(
            board.rules.clearance.names,
            ["null", "default", "power", "ground"]
        );
        assert_eq!(board.rules.clearance.get_value(1, 1, 0), 0);
        assert_eq!(board.rules.clearance.get_value(2, 3, 0), 9000);
        assert_eq!(board.rules.default_trace_half_widths, [1500, 1500]);
        assert_eq!(board.rules.pin_edge_to_turn_dist, 100000.0);
        // 1 outline (the outer square), 2 keepouts (the hole, one per layer)
        assert_eq!(board.items.len(), 3);
        let mut keepouts = Vec::new();
        for item in &board.items {
            if let crate::ses_board::ItemIr::Keepout { id, keepout } = item {
                keepouts.push((*id, keepout));
            }
        }
        keepouts.sort_by_key(|(id, _)| *id);
        assert_eq!(keepouts.len(), 2);
        for (index, (id, keepout)) in keepouts.iter().enumerate() {
            assert_eq!(*id, index as i32 + 2);
            assert_eq!(keepout.layer_no, index as i32);
            assert_eq!(keepout.clearance_class, 0);
            assert_eq!(keepout.fixed, FixedStateIr::SystemFixed);
            let bbox = keepout.area.border.bounding_box();
            assert_eq!(
                [bbox.ll.x, bbox.ll.y, bbox.ur.x, bbox.ur.y],
                [12_000, 12_000, 18_000, 18_000]
            );
        }
        match &board.items[0] {
            crate::ses_board::ItemIr::BoardOutline { outline, .. } => {
                assert_eq!(outline.clearance_class, 2);
                assert_eq!(outline.shapes.len(), 1);
                let bbox = outline.shapes[0].bounding_box();
                assert_eq!(
                    [bbox.ll.x, bbox.ll.y, bbox.ur.x, bbox.ur.y],
                    [10_000, 10_000, 20_000, 20_000]
                );
            }
            other => panic!("expected BoardOutline first, got {other:?}"),
        }
    }

    /// Jar `/tmp/epic-t5-boundary-b.out`: a NONEXISTENT
    /// `clearance_class` name resolves to class 0 (`max(0, -1)`) — the
    /// `null` class, NOT the default class.
    #[test]
    fn boundary_unknown_clearance_class_resolves_to_null_class() {
        let (ok, _state, board) = run_structure(
            "(layer F.Cu (type signal))\
             (layer B.Cu (type signal))\
             (boundary (rect pcb 0 0 5000 4000) (clearance_class nonexistent))",
        );
        assert!(ok);
        match &board.items[0] {
            crate::ses_board::ItemIr::BoardOutline { outline, .. } => {
                assert_eq!(outline.clearance_class, 0);
            }
            other => panic!("expected BoardOutline, got {other:?}"),
        }
    }

    /// Jar `/tmp/epic-t5-boundary-c.out` + `/tmp/epic-t5-holeprobe.out`:
    /// the L-shaped outline keeps its NOTCH SQUARE as a hole — Java's
    /// convex split over-covers (tiles `[10000..35000]x[10000..15000]`
    /// and `[35000..40000]x[10000..40000]`), so all notch corners land in
    /// the second tile. Also pins the absent-`clearance_class` default
    /// (class 1, the AREA item class).
    #[test]
    fn boundary_l_shape_notch_becomes_hole_jar_probe() {
        let (ok, _state, board) = run_structure(
            "(layer F.Cu (type signal))\
             (layer B.Cu (type signal))\
             (boundary\
               (rect pcb 0 0 5000 5000)\
               (path signal 0 1000 1000 4000 1000 4000 4000 3500 4000 3500 1500 1000 1500 1000 1000)\
               (path signal 0 3600 2000 3900 2000 3900 3000 3600 3000 3600 2000))",
        );
        assert!(ok);
        assert_eq!(
            board.rules.clearance.names,
            ["null", "default"],
            "no rule scopes: only the built-ins"
        );
        match &board.items[0] {
            crate::ses_board::ItemIr::BoardOutline { outline, .. } => {
                assert_eq!(outline.clearance_class, 1);
                assert_eq!(outline.shapes.len(), 1);
                let bbox = outline.shapes[0].bounding_box();
                assert_eq!(
                    [bbox.ll.x, bbox.ll.y, bbox.ur.x, bbox.ur.y],
                    [10_000, 10_000, 40_000, 40_000]
                );
            }
            other => panic!("expected BoardOutline, got {other:?}"),
        }
        // the notch square is the hole on both layers
        for item in &board.items[1..] {
            let crate::ses_board::ItemIr::Keepout { keepout, .. } = item else {
                panic!("expected keepouts after the outline, got {item:?}");
            };
            let bbox = keepout.area.border.bounding_box();
            assert_eq!(
                [bbox.ll.x, bbox.ll.y, bbox.ur.x, bbox.ur.y],
                [36_000, 20_000, 39_000, 30_000]
            );
        }
        assert_eq!(board.items.len(), 3);
    }

    /// Jar `/tmp/epic-t5-boundary-d.out`: the unknown layer type drops
    /// the layer, a later type-less `(layer Bad (rule ...))` re-adds it
    /// (signal default), duplicate `B.Cu` layers survive, the
    /// `(property (index 1))` scope is skipped generically (T46), and the
    /// layer rules resolve through the parser layer structure — the FIRST
    /// `B.Cu` (layer 1) and the re-added `Bad` (layer 3). The pin-edge
    /// fallback is the MIN default half width (3000).
    #[test]
    fn layer_scope_drop_readd_duplicate_jar_probe() {
        let (ok, _state, board) = run_structure(
            "(layer F.Cu (type signal))\
             (layer Bad (type flex))\
             (layer B.Cu (type signal) (property (index 1)))\
             (boundary (rect pcb 0 0 5000 4000))\
             (rule (width 600))\
             (layer B.Cu (rule (width 800)))\
             (layer Bad (rule (width 900)))",
        );
        assert!(ok);
        let layers = board.layers.as_ref().expect("layers");
        let names: Vec<&str> = layers.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["F.Cu", "B.Cu", "B.Cu", "Bad"]);
        assert!(layers.layers.iter().all(|l| l.is_signal));
        assert_eq!(
            board.rules.default_trace_half_widths,
            [3000, 4000, 3000, 4500]
        );
        assert_eq!(board.rules.pin_edge_to_turn_dist, 3000.0);
        assert_eq!(board.items.len(), 1, "no holes: single rect boundary");
    }

    /// Jar `/tmp/epic-t5-boundary-e.out`: the degenerate path (five
    /// identical corners) is dropped by the `dimension() > 0` filter and
    /// the outline falls back to the transformed bounding shape — WITHOUT
    /// any warning or failure.
    #[test]
    fn degenerate_path_falls_back_to_bounding_shape() {
        let (ok, state, board) = run_structure(
            "(layer F.Cu (type signal))\
             (layer B.Cu (type signal))\
             (boundary\
               (rect pcb 0 0 5000 4000)\
               (path signal 0 2000 2000 2000 2000 2000 2000 2000 2000 2000 2000))",
        );
        assert!(ok);
        assert!(state.board_outline_ok);
        assert_eq!(board.items.len(), 1);
        match &board.items[0] {
            crate::ses_board::ItemIr::BoardOutline { outline, .. } => {
                assert_eq!(outline.shapes.len(), 1);
                let bbox = outline.shapes[0].bounding_box();
                assert_eq!(
                    [bbox.ll.x, bbox.ll.y, bbox.ur.x, bbox.ur.y],
                    [0, 0, 50_000, 40_000]
                );
            }
            other => panic!("expected BoardOutline, got {other:?}"),
        }
    }

    /// Jar `/tmp/epic-t5-boundary-missing.out`: no boundary at all ->
    /// `boardOutlineOk = false` and a failed structure read (the Task 9
    /// assembly classifies it as `OutlineMissing`).
    #[test]
    fn missing_boundary_sets_outline_missing() {
        let (ok, state, board) = run_structure(
            "(layer F.Cu (type signal))\
             (layer B.Cu (type signal))",
        );
        assert!(!ok);
        assert!(!state.board_outline_ok);
        assert!(!board.has_board());
    }

    /// Jar `/tmp/epic-t5-boundary-polyline.out`: a `polyline_path`
    /// boundary NPEs the Java reader at `Structure.java:1167` (the null
    /// `boundingBox()` stub in the union loop) — the port fails the parse
    /// with `board_outline_ok` still TRUE (the flag is never set on the
    /// crash path, so the Task 9 assembly classifies this as a parse
    /// error, not OutlineMissing).
    #[test]
    fn polyline_path_boundary_is_a_parse_failure() {
        let (ok, state, board) = run_structure(
            "(layer F.Cu (type signal))\
             (layer B.Cu (type signal))\
             (boundary (polyline_path pcb 0 0 0 5000 0 5000 4000 0 4000 0 0))",
        );
        assert!(!ok);
        assert!(state.board_outline_ok);
        assert!(!board.has_board());
    }

    /// `(use_net ...)` re-enters the NAME lexical state before EVERY
    /// token, so keywords scan as plain strings; the names land on the
    /// layer (the plane heuristic consumes them, Task 6).
    #[test]
    fn use_net_reads_names_in_name_state() {
        let (ok, state, _board) = run_structure(
            "(layer F.Cu (type signal))\
             (layer B.Cu (type power) (use_net GND VDD rule))\
             (boundary (rect pcb 0 0 5000 4000))",
        );
        assert!(ok);
        let layers = state.layer_structure.as_ref().expect("parser layers");
        assert_eq!(layers.layers[1].name, "B.Cu");
        assert!(!layers.layers[1].is_signal);
        assert_eq!(layers.layers[1].net_names, ["GND", "VDD", "rule"]);
    }

    /// `(via a b (spare c))` -> `["a", "b", "c"]` on the parse state
    /// (`Structure.java:979-980`, spares appended).
    #[test]
    fn via_padstacks_with_spares() {
        let (ok, state, _board) = run_structure(
            "(layer F.Cu (type signal))\
             (via via_a via_b (spare spare_a))\
             (boundary (rect pcb 0 0 5000 4000))",
        );
        assert!(ok);
        assert_eq!(
            state.via_padstack_names,
            Some(vec![
                "via_a".to_string(),
                "via_b".to_string(),
                "spare_a".to_string()
            ])
        );
    }

    /// Java `:1034-1036`: a second `(structure ...)` scope re-parses but
    /// does NOT rebuild the board (the `has_board` guard) — jar
    /// `/tmp/epic-t4c-twostruct.out`. The parser layer structure still
    /// follows the LAST scope (the `:975-978` correction).
    #[test]
    fn second_structure_scope_does_not_rebuild_board() {
        let mut scanner = Scanner::new(
            b"(structure (layer F.Cu (type signal)) (boundary (rect pcb 0 0 5000 4000)))\
              (structure (layer G.Cu (type signal)) (boundary (rect pcb 0 0 6000 4000)))"
                .as_slice(),
        );
        let mut state = ParseState::default();
        let mut board = SesBoard::new();
        for expected in ["F.Cu", "G.Cu"] {
            assert_eq!(scanner.next_token(), Token::Open);
            assert_eq!(scanner.next_token(), Token::Keyword(Keyword::Structure));
            assert!(read_scope(&mut scanner, &mut state, &mut board));
            let parser_layers = state.layer_structure.as_ref().expect("parser layers");
            assert_eq!(parser_layers.layers[0].name, expected);
        }
        // the board was built ONCE, from the FIRST scope
        let board_layers = board.layers.as_ref().expect("layers");
        assert_eq!(board_layers.layers[0].name, "F.Cu");
        assert_eq!(board.items.len(), 1, "ONE outline — no rebuild");
    }

    // ---- unit pins for the IR ops (ClearanceMatrix.java verbatim) ------

    /// `setValue` even-ize (`ClearanceMatrix.java:107-124`): negatives
    /// clamp to 0, odd values round UP (jar: 5 -> 6, 7 -> 8 in
    /// `/tmp/epic-t5-probe2.out`).
    #[test]
    fn matrix_set_value_evenizes() {
        let mut matrix = crate::sink::ClearanceIr::default_instance(1);
        matrix.set_value(1, 1, 0, 5);
        assert_eq!(matrix.get_value(1, 1, 0), 6);
        matrix.set_value(1, 1, 0, 7);
        assert_eq!(matrix.get_value(1, 1, 0), 8);
        matrix.set_value(1, 1, 0, -3);
        assert_eq!(matrix.get_value(1, 1, 0), 0);
        matrix.set_value(1, 1, 0, 4);
        assert_eq!(matrix.get_value(1, 1, 0), 4, "even values untouched");
    }

    /// `setDefaultValue` skips class 0 (`:90-94`) and the out-of-bounds
    /// `getValue` returns 0 (`:126-160`).
    #[test]
    fn matrix_default_value_skips_class_zero() {
        let mut matrix = crate::sink::ClearanceIr::default_instance(2);
        matrix.append_class("x");
        matrix.set_default_value_all_layers(500);
        assert_eq!(matrix.get_value(0, 0, 0), 0, "class 0 untouched");
        assert_eq!(matrix.get_value(0, 1, 0), 0);
        assert_eq!(matrix.get_value(1, 1, 0), 500);
        assert_eq!(matrix.get_value(2, 2, 1), 500);
        assert_eq!(matrix.get_value(9, 9, 0), 0, "out of bounds reads 0");
        assert_eq!(matrix.get_value(1, 1, 5), 0, "out of bounds layer reads 0");
    }

    /// `appendClass` (`:283-325`): the new row/column copies class 1 per
    /// layer, including values set BEFORE the append (jar:
    /// `power`/`ground` rows read 5000 in `/tmp/epic-t5-probe.out` — the
    /// default was set by `(clearance 500)` before the append).
    #[test]
    fn matrix_append_class_copies_class_one() {
        let mut matrix = crate::sink::ClearanceIr::default_instance(2);
        matrix.set_default_value_all_layers(5000);
        assert!(matrix.append_class("power"));
        assert!(matrix.append_class("ground"));
        assert!(!matrix.append_class("POWER"), "case-insensitive dedup");
        assert_eq!(matrix.class_count(), 4);
        assert_eq!(
            matrix.get_value(2, 3, 0),
            5000,
            "appended row copies class 1"
        );
        assert_eq!(matrix.get_value(3, 2, 1), 5000);
        assert_eq!(matrix.get_value(3, 3, 0), 5000, "diagonal copies (1,1)");
        matrix.set_value(2, 3, 0, 9000);
        assert_eq!(matrix.get_value(2, 3, 0), 9000);
        assert_eq!(matrix.get_value(2, 3, 1), 5000, "other layers untouched");
    }

    /// `BoardRulesIr` (`BoardRules.java:52-63` + the lazy default net
    /// class `:206-213`): widths 1500, min 100000, max 100, item classes
    /// all 1; the width setters update min/max and the
    /// `setDefaultTraceHalfWidths` guard rejects `value <= 0`.
    #[test]
    fn board_rules_ir_defaults_and_width_setters() {
        let mut rules = BoardRulesIr::new(2);
        assert_eq!(rules.default_trace_half_widths, [1500, 1500]);
        assert_eq!(rules.min_trace_half_width, 100_000);
        assert_eq!(rules.max_trace_half_width, 100);
        assert_eq!(rules.pin_edge_to_turn_dist, 0.0);
        assert_eq!(
            rules.trace_angle_restriction,
            AngleRestriction::FortyfiveDegree
        );
        assert_eq!(rules.default_item_clearance_classes, [1; 6]);
        rules.set_default_trace_half_widths(3000);
        assert_eq!(rules.default_trace_half_widths, [3000, 3000]);
        assert_eq!(rules.min_trace_half_width, 3000);
        assert_eq!(rules.max_trace_half_width, 3000);
        rules.set_default_trace_half_width(1, 4000);
        assert_eq!(rules.default_trace_half_widths, [3000, 4000]);
        assert_eq!(rules.min_trace_half_width, 3000);
        assert_eq!(rules.max_trace_half_width, 4000);
        rules.set_default_trace_half_widths(0);
        assert_eq!(
            rules.default_trace_half_widths,
            [3000, 4000],
            "0 is a no-op"
        );
        rules.set_default_trace_half_widths(-5);
        assert_eq!(
            rules.default_trace_half_widths,
            [3000, 4000],
            "negative is a no-op"
        );
    }

    /// `appendClearanceClass` (`:826-840`) retargets the default item
    /// classes for via/pin/smd/area (case-sensitive) — the wire rule's
    /// `area` append is what makes the outline class 7 in the jar.
    #[test]
    fn append_clearance_class_retargets_item_classes() {
        let mut rules = BoardRulesIr::new(1);
        assert_eq!(append_clearance_class(&mut rules, "area"), 2);
        assert_eq!(
            rules.default_item_clearance_classes[ItemClassIr::Area as usize],
            2
        );
        assert_eq!(append_clearance_class(&mut rules, "via"), 3);
        assert_eq!(
            rules.default_item_clearance_classes[ItemClassIr::Via as usize],
            3
        );
        assert_eq!(
            rules.default_item_clearance_classes[ItemClassIr::Pin as usize],
            1
        );
        // the retarget switch is case-sensitive, but the LOOKUP is not:
        // Java appendClearanceClass("AREA") dedups to the existing `area`
        // class and returns getNo("AREA") = 2 without retargeting anything
        assert_eq!(append_clearance_class(&mut rules, "AREA"), 2);
        assert_eq!(
            rules.default_item_clearance_classes[ItemClassIr::Area as usize],
            2
        );
    }

    /// The T25 scale-factor loop through `calc_scale_factor`
    /// (`Structure.java:1189-1203`): the fixture scale stays 10
    /// (5 * 50000 < CRIT_INT); a huge coordinate shrinks the scale.
    #[test]
    fn scale_factor_loop_matches_task2_pin() {
        assert_eq!(calc_scale_factor(10, 50_000.0), 10);
        assert_eq!(calc_scale_factor(100, 10_000_000.0), 10);
    }

    /// A `(rule (width abc))` NPEs the Java reader (`nextDouble` null
    /// unboxing) — the port fails the parse.
    #[test]
    fn unparsable_width_value_is_a_parse_failure() {
        let (ok, _state, board) = run_structure(
            "(layer F.Cu (type signal))\
             (boundary (rect pcb 0 0 5000 4000))\
             (rule (width abc))",
        );
        assert!(!ok);
        assert!(!board.has_board());
    }

    /// An unterminated `(rule` scope (EOF) makes Java's
    /// `Rule.readScope` return null and `Structure.java:982` NPE on
    /// `addAll(null)` — parse failure parity.
    #[test]
    fn eof_inside_rule_scope_is_a_parse_failure() {
        let input = "(structure (layer F.Cu (type signal)) (rule (width 600)";
        let mut scanner = Scanner::new(input.as_bytes());
        assert_eq!(scanner.next_token(), Token::Open);
        assert_eq!(scanner.next_token(), Token::Keyword(Keyword::Structure));
        let mut state = ParseState::default();
        let mut board = SesBoard::new();
        assert!(!read_scope(&mut scanner, &mut state, &mut board));
    }

    /// A `circle` boundary shape transforms to a `geometry.planar.Circle`
    /// — NOT a `PolylineShape` — so the Java `(PolylineShape)` cast at
    /// `Structure.java:1221` throws ClassCastException; parse-fatal parity.
    #[test]
    fn circle_boundary_is_a_parse_failure() {
        let (ok, state, board) = run_structure(
            "(layer F.Cu (type signal))\
             (layer B.Cu (type signal))\
             (boundary (rect pcb 0 0 5000 4000) (circle signal 100 2500 2000))",
        );
        assert!(!ok);
        assert!(state.board_outline_ok, "crash path: flag never set");
        assert!(!board.has_board());
    }
}
