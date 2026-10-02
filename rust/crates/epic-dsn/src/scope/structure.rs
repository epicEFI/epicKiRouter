//! The `(structure ...)` scope reader: the port of
//! `io/specctra/parser/Structure.java` — the layer/boundary/rules/
//! clearance halves, the `createBoard` construction, and the keepout,
//! plane and control scopes (`insertKeepout`, the `insertConductionArea`
//! loop, `insertMissingPowerPlanes`, the `adjustPlaneAutorouteSettings`
//! heuristic) — plus the `Rule.java` scope readers it drives.
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

use super::plane::read_plane_scope;
use super::rule::{Rule, read_rule_scope};
use crate::coordinate_transform::{
    CoordinateTransform, calc_scale_factor, max_abs_scaled_coordinate,
};
use crate::keyword::{Keyword, skip_scope};
use crate::layer_structure::{Layer, LayerStructure};
use crate::lexer::{LexicalState, Scanner, Token};
use crate::shape::{
    BoardShape, PolygonPath, Shape, TransformedArea, box_shape, read_area_scope,
    transform_area_to_board,
};
use crate::sink::{
    AreaIr, BoardRulesIr, BoardSink, ConductionAreaIr, CreateBoardIr, FixedStateIr, ItemClassIr,
    KeepoutIr, KeepoutKindIr, NetIr,
};
use crate::state::{AngleRestriction, AreaScopeResult, NetId, ParseState, PlaneInfo};
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
/// `""`, which both callers — the rule-scope readers and `read_area_scope`'s
/// `(clearance_class ...)` arm — store verbatim).
pub(crate) fn read_string_scope(scanner: &mut Scanner) -> String {
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

/// Java `DsnFile.readIntegerScope` (`DsnFile.java:131-154`): a strict
/// Integer then the closing bracket; 0 on either mismatch. NOTE: the
/// non-bracket token is CONSUMED AND DISCARDED (no drain-to-bracket
/// here).
pub(crate) fn read_integer_scope(scanner: &mut Scanner) -> i32 {
    let value = match scanner.next_token() {
        Token::Int(value) => value,
        // Java: warn "DsnFile.read_integer_scope: number expected at
        // '<id>'" (log-only).
        _ => return 0,
    };
    if scanner.next_token() != Token::Close {
        // Java: warn "DsnFile.read_integer_scope: closing bracket expected
        // at '<id>'" (log-only).
        return 0;
    }
    value
}

/// Java `DsnFile.readFloatScope` (`DsnFile.java:159-182`): a Double OR an
/// Integer (widened) then the closing bracket; 0.0 on mismatch — again
/// WITHOUT a drain loop.
pub(crate) fn read_float_scope(scanner: &mut Scanner) -> f64 {
    let value = match scanner.next_token() {
        Token::Double(value) => value,
        Token::Int(value) => f64::from(value),
        // Java: warn "DsnFile.read_float_scope: number expected at '<id>'"
        // (log-only).
        _ => return 0.0,
    };
    if scanner.next_token() != Token::Close {
        // Java: warn "DsnFile.read_float_scope: closing bracket expected at
        // '<id>'" (log-only).
        return 0.0;
    }
    value
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
/// so a failed read clears any earlier list. `pub(crate)`: the network
/// scope's `(circuit (use_via ...))` reader (`Circuit.java:44-50` via
/// `NetClass.java`) calls the SAME method.
pub(crate) fn read_via_padstacks(scanner: &mut Scanner) -> Option<Vec<String>> {
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
/// loop + the post-loop board construction and keepout/plane/power-plane
/// insertions. `state` collects the parse state (`layerStructure`,
/// `coordinateTransform`, `viaPadstackNames`, `boardOutlineOk`, planes,
/// control values), `sink` receives the board and items.
///
/// `(autoroute_settings ...)` mirrors the Java quirk of doing NOTHING AT
/// ALL (not even reading) when the layer structure already exists
/// (`:1006-1011`); the Task 9 assembly owns the reader.
///
/// Warnings: every FRLogger.warn in the Task 6 halves (degenerate
/// keepouts, unknown clearance classes, bad plane layers, via_at_smd
/// garbage, ...) is log-only — none reach `state.warnings`, so the
/// parity-warnings count stays 0 on every fixture that parses.
pub fn read_scope(scanner: &mut Scanner, state: &mut ParseState, sink: &mut dyn BoardSink) -> bool {
    let mut info = BoardConstructionInfo::default();

    // If true, components on the back side are rotated before mirroring
    // (`:939-941` — the correct place is the place_control scope, but
    // Electra writes it here).
    let mut flip_style_rotate_first = false;
    // Java `:943-945`: LinkedLists that tolerate null entries.
    let mut keepout_list: Vec<Option<AreaScopeResult>> = Vec::new();
    let mut via_keepout_list: Vec<Option<AreaScopeResult>> = Vec::new();
    let mut place_keepout_list: Vec<Option<AreaScopeResult>> = Vec::new();

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
                // ---- keepout/plane/control family (Task 6) --------------
                Token::Keyword(Keyword::Keepout)
                | Token::Keyword(Keyword::ViaKeepout)
                | Token::Keyword(Keyword::PlaceKeepout) => {
                    if state.layer_structure.is_none() {
                        state.layer_structure = Some(LayerStructure::new(info.layer_info.clone()));
                    }
                    // Java `:983-1000`: the area scope is read and APPENDED
                    // even when the read fails (Java LinkedList of possibly
                    // null entries) — the failure surfaces as the
                    // `area.shapeList` NPE at insertion time.
                    let area = read_area_scope(scanner, state.layer_structure.as_ref(), false);
                    match next_token {
                        Token::Keyword(Keyword::Keepout) => keepout_list.push(area),
                        Token::Keyword(Keyword::ViaKeepout) => via_keepout_list.push(area),
                        _ => place_keepout_list.push(area),
                    }
                }
                Token::Keyword(Keyword::Plane) => {
                    if state.layer_structure.is_none() {
                        state.layer_structure = Some(LayerStructure::new(info.layer_info.clone()));
                    }
                    // Java `:1005`: the return value is DISCARDED — a failed
                    // read appends no plane and does not fail the parse.
                    read_plane_scope(scanner, state);
                }
                Token::Keyword(Keyword::AutorouteSettings) => {
                    // Java `:1006-1011`: read ONLY when the layer structure
                    // is still null — otherwise the scope is left UNREAD
                    // (bug-compat; the scanner desyncs exactly like Java).
                    if state.layer_structure.is_none() {
                        let layer_structure = LayerStructure::new(info.layer_info.clone());
                        // Java assigns the readScope return UNCONDITIONALLY
                        // — a failed read stores null (D15: the raw IR, no
                        // RouterSettings merger until M3).
                        state.autoroute_settings =
                            crate::scope::autoroute_settings::read_scope(scanner, &layer_structure);
                        state.layer_structure = Some(layer_structure);
                    }
                }
                Token::Keyword(Keyword::Control) => {
                    read_ok = read_control_scope(scanner, state);
                }
                Token::Keyword(Keyword::FlipStyle) => {
                    // Java `:1015`: the local is OVERWRITTEN — a second
                    // `(flip_style rotate_first)` after a failed read
                    // clears it again.
                    flip_style_rotate_first = read_flip_style_rotate_first(scanner);
                }
                Token::Keyword(Keyword::SnapAngle) => {
                    // Java `:1018-1021`: a FAILED read (None) keeps the
                    // previous value.
                    if let Some(snap_angle) = read_snap_angle(scanner) {
                        state.snap_angle = snap_angle;
                    }
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

    // Post loop (`:1031-1123`): build the board once, then flip style,
    // keepouts, planes and the missing-power-plane fallback. The
    // `:1126-1134` autoroute-settings job block needs a RoutingJob (Task
    // 9 assembly); the plane heuristic itself runs from there.
    let mut result = true;
    if !sink.has_board() {
        result = create_board(state, &mut info, sink);
    }
    if !sink.has_board() {
        return false;
    }
    if flip_style_rotate_first {
        // Java `board.components.setFlipStyleRotateFirst(true)`
        // (`:1041-1043`) — only TRUE is ever stored.
        sink.set_flip_style("rotate_first".to_string());
    }
    // insert the keepouts (`:1045-1065`); a false from insertKeepout is a
    // parse failure.
    for area in &keepout_list {
        if !insert_keepout(
            area.as_ref(),
            state,
            sink,
            KeepoutKindIr::Keepout,
            FixedStateIr::SystemFixed,
        ) {
            return false;
        }
    }
    for area in &via_keepout_list {
        if !insert_keepout(
            area.as_ref(),
            state,
            sink,
            KeepoutKindIr::ViaKeepout,
            FixedStateIr::SystemFixed,
        ) {
            return false;
        }
    }
    for area in &place_keepout_list {
        if !insert_keepout(
            area.as_ref(),
            state,
            sink,
            KeepoutKindIr::PlaceKeepout,
            FixedStateIr::SystemFixed,
        ) {
            return false;
        }
    }
    // insert the planes (`:1067-1122`) — over the ACCUMULATED plane list,
    // so a second structure scope re-inserts them (Java bug-compat; the
    // clone breaks the state borrow for `add_plane_net`).
    let plane_infos = state.plane_list.clone();
    for plane_info in &plane_infos {
        if !insert_plane(plane_info, state, sink) {
            return false;
        }
    }
    insert_missing_power_planes(&info.layer_info, state, sink);
    result
}

/// Java `DsnFile.readOnOffScope` (`DsnFile.java:116-132`): one value
/// token — `on` -> true; anything that is not `off` warns (log-only) and
/// stays false; then the scope is ALWAYS drained. Never fails: the
/// primitive `boolean` return is why `via_at_smd_allowed` is a plain
/// `bool` on the parse state.
pub(crate) fn read_on_off_scope(scanner: &mut Scanner) -> bool {
    let mut result = false;
    if scanner.next_token() == Token::Keyword(Keyword::On) {
        result = true;
    }
    skip_scope(scanner);
    result
}

/// Java `Structure.readControlScope` (`:446-476`): scans the control
/// scope body; `(via_at_smd <value>)` feeds
/// [`ParseState::via_at_smd_allowed`], any other inner scope is skipped.
/// EOF (or an error token) is a `false` return, the closing bracket a
/// `true`.
fn read_control_scope(scanner: &mut Scanner, state: &mut ParseState) -> bool {
    let mut prev_token: Option<Token> = None;
    loop {
        let next_token = scanner.next_token();
        if matches!(next_token, Token::Eof | Token::Error(_)) {
            // Java warns "unexpected end of file" (`:456-462`)
            return false;
        }
        if next_token == Token::Close {
            break;
        }
        if prev_token == Some(Token::Open) {
            if next_token == Token::Keyword(Keyword::ViaAtSmd) {
                state.via_at_smd_allowed = read_on_off_scope(scanner);
            } else {
                skip_scope(scanner);
            }
        }
        prev_token = Some(next_token);
    }
    true
}

/// Java `PlaceControl.readFlipStyleRotateFirst` (`PlaceControl.java:16-36`):
/// the first token decides (`rotate_first` -> true), then the closing
/// bracket is required — a missing one returns false even after a
/// `rotate_first`.
pub(crate) fn read_flip_style_rotate_first(scanner: &mut Scanner) -> bool {
    let mut result = false;
    if scanner.next_token() == Token::Keyword(Keyword::RotateFirst) {
        result = true;
    }
    if scanner.next_token() != Token::Close {
        // Java warns "closing bracket expected"
        return false;
    }
    result
}

/// Java `Structure.readSnapAngle` (`:478-508`): one of the three angle
/// keywords, then the closing bracket. `None` (Java null) on a bad token
/// or a missing bracket — the caller KEEPS the previous value then
/// (`:1018-1021`).
fn read_snap_angle(scanner: &mut Scanner) -> Option<AngleRestriction> {
    let snap_angle = match scanner.next_token() {
        Token::Keyword(Keyword::NinetyDegree) => AngleRestriction::NinetyDegree,
        Token::Keyword(Keyword::FortyfiveDegree) => AngleRestriction::FortyfiveDegree,
        Token::Keyword(Keyword::None) => AngleRestriction::None,
        _ => {
            // Java warns "unexpected token"
            return None;
        }
    };
    if scanner.next_token() != Token::Close {
        // Java warns "closing bracket expected"
        return None;
    }
    Some(snap_angle)
}

/// The first shape's parser layer (Java `area.shapeList.iterator().next()
/// .layer`, `:882`/`:1086`). The `PolylinePath` arm is only reachable
/// from the PLANE path (a keepout dies earlier on the null transform);
/// `None` (Java null layer from an unknown layer name, `Shape.java:
/// 107-162` has no check) maps to `None` for the caller's NPE parity.
fn area_first_layer(area: &AreaScopeResult) -> Option<&Layer> {
    match area.shapes[0].as_ref() {
        None => None, // unreachable for a successful read (resultOk gate)
        Some(Shape::Rectangle(shape)) => Some(&shape.layer),
        Some(Shape::Polygon(shape)) => Some(&shape.layer),
        Some(Shape::Circle(shape)) => Some(&shape.layer),
        Some(Shape::PolylinePath(shape)) => shape.layer.as_ref(),
        Some(Shape::PolygonPath(shape)) => Some(&shape.layer),
    }
}

/// Java `Structure.insertKeepout` (`:857-901`). `area` is `None` (Java
/// null, a failed `readAreaScope` stored in the list unchecked) exactly
/// where Java NPEs on `area.shapeList` — parse-fatal parity, as is the
/// null transform result (`keepoutArea.dimension()` NPE at `:864`, jar
/// `/tmp/epic-t6-board.out` THROWN lines). A degenerate area (dimension
/// < 2) is SKIPPED with a log-only warning and does not fail the parse
/// (`:864-876`). A `signal`-layer keepout inserts on every PARSER signal
/// layer (indexed up to the BOARD layer count — a shorter parser
/// structure is the AIOOBE parity `false`); a `pcb`/unknown-layer
/// keepout (no < 0) warns and FAILS the parse (`:892-898`, jar
/// t6-pcb-keep).
fn insert_keepout(
    area: Option<&AreaScopeResult>,
    state: &ParseState,
    sink: &mut dyn BoardSink,
    kind: KeepoutKindIr,
    fixed: FixedStateIr,
) -> bool {
    let Some(area) = area else {
        return false; // Java NPE at `area.shapeList` (`:862`, jar stack)
    };
    let Some(coordinate_transform) = &state.coordinate_transform else {
        // Java NPE inside transformToBoard — unreachable post-create_board
        return false;
    };
    let keepout_area = match transform_area_to_board(&area.shapes, coordinate_transform) {
        TransformedArea::Area(area) => area,
        // BOTH Java failure flavors kill the parse at the keepout site:
        // the null return NPEs at `keepoutArea.dimension()` (`:864`, jar
        // t6-keep-circle-hole) and the null hole ENTRY throws inside
        // transformAreaToBoard (jar t6-keep-nullhole)
        TransformedArea::Null | TransformedArea::Threw => return false,
    };
    if keepout_area.border.dimension() < 2 {
        // Java `:864-876`: FRLogger.warn "Keepout zone '<name>' was
        // skipped because its geometry is degenerate ..." — log-only.
        return true;
    }
    let Some(current_layer) = area_first_layer(area) else {
        return false; // Java NPE at `:882` — unreachable (see area_first_layer)
    };
    if current_layer.name == Layer::SIGNAL_NAME {
        // `currentLayer == Layer.SIGNAL` — the getLayer interning makes
        // the exact-case name the identity test (shape.rs module docs).
        let board_layer_count = sink.layer_count();
        let Some(layer_structure) = &state.layer_structure else {
            // unreachable: the keepout arm guarantees the structure
            return false;
        };
        for i in 0..board_layer_count {
            match layer_structure.layers.get(i as usize) {
                // Java `:884-888` indexes the PARSER structure up to the
                // BOARD layer count — a shorter parser structure AIOOBEs
                // (parse dies).
                None => return false,
                Some(parser_layer) => {
                    if parser_layer.is_signal {
                        insert_keepout_item(
                            sink,
                            &keepout_area,
                            i,
                            area.clearance_class.as_deref(),
                            kind,
                            fixed,
                        );
                    }
                }
            }
        }
    } else if current_layer.no >= 0 {
        insert_keepout_item(
            sink,
            &keepout_area,
            current_layer.no,
            area.clearance_class.as_deref(),
            kind,
            fixed,
        );
    } else {
        // Java `:892-898`: "Structure.insert_keepout: unknown layer name"
        // — the Layer.PCB arm (jar t6-pcb-keep: ParseError).
        return false;
    }
    true
}

/// Java `Structure.insertKeepout` inner overload (`:903-933`): the
/// clearance class resolves from the name (a miss warns and falls back
/// to class 0, the `null` class) or from the default net class's AREA
/// item class when no name is present; then one of the three board
/// obstacle insertions per keepout kind.
fn insert_keepout_item(
    sink: &mut dyn BoardSink,
    area: &AreaIr,
    layer: i32,
    clearance_class_name: Option<&str>,
    kind: KeepoutKindIr,
    fixed: FixedStateIr,
) {
    let clearance_class_index = match clearance_class_name {
        None => sink.default_item_clearance_class(ItemClassIr::Area),
        Some(name) => {
            // Java `getNo` -> `< 0` warns "Keepout.insert_keepout:
            // clearance class not found" (log-only) and falls back to
            // BoardRules.clearanceClassNone() == 0.
            sink.clearance_class_no(name).unwrap_or(0)
        }
    };
    sink.insert_keepout(KeepoutIr {
        kind,
        layer_no: layer,
        area: area.clone(),
        clearance_class: clearance_class_index,
        fixed,
        component_id: 0,
        translation: epic_geometry::int_point::IntPoint::new(0, 0),
        rotation: 0.0,
        side_changed: false,
        name: None,
    });
}

/// The netlist-guard block of the plane and power-plane insertions
/// (`Structure.java:1070-1075` and `:543-548`): the CASE-SENSITIVE
/// parser `NetList` decides is-new; a new net is appended to the BOARD
/// table with `contains_plane = true` (`Nets.add(..., true)`).
fn add_plane_net(state: &mut ParseState, sink: &mut dyn BoardSink, name: &str) {
    let net_id = NetId {
        name: name.to_string(),
        subnet_number: 1,
    };
    if !state.netlist.contains(&net_id) && state.netlist.add_net(net_id).is_some() {
        sink.append_net(NetIr {
            name: name.to_string(),
            subnet_number: 1,
            contains_plane: true,
            net_class: 0,
        });
    }
}

/// One iteration of the plane insertion loop (`Structure.java:1068-1122`).
/// Order is load-bearing: the net list entry is created BEFORE the area
/// is transformed, and the layer check comes last — a plane on an
/// unknown layer still added its net before the parse fails
/// (`:1115-1120`, jar t6-plane-signal). `plane_info.area == None` is the
/// `Structure.java:1084` NPE (a failed area read stored unchecked).
/// The transform's failure flavors split here (`Shape.java:543-548`):
/// a null hole ENTRY throws inside `transformAreaToBoard` — parse-fatal
/// (jar t6-plane-nullhole) — while the null-RETURN flavor (circle or
/// failed-transform hole) only skips the plane, `BasicBoard` rejecting
/// the null area with a log-only warning BEFORE burning an id
/// (`BasicBoard.java:553-556`; jar t6-plane-circwin, t6-plane-mixed-holes).
fn insert_plane(plane_info: &PlaneInfo, state: &mut ParseState, sink: &mut dyn BoardSink) -> bool {
    add_plane_net(state, sink, &plane_info.net_name);
    let Some(current_net_no) = sink.net_no_subnet(&plane_info.net_name, 1) else {
        // Java `:1077-1083`: "Plane.read_scope: net not found" (log-only)
        return true; // continue with the next plane
    };
    let Some(area) = &plane_info.area else {
        return false; // Java NPE at `planeInfo.area.shapeList` (`:1084`, jar stack)
    };
    let Some(coordinate_transform) = &state.coordinate_transform else {
        // unreachable post-create_board (Java NPE inside transformToBoard)
        return false;
    };
    let plane_area = match transform_area_to_board(&area.shapes, coordinate_transform) {
        TransformedArea::Area(area) => Some(area),
        // Java null return: BasicBoard.insertConductionArea warns
        // (log-only) and the parse CONTINUES without the plane
        TransformedArea::Null => None,
        // Java THREW in the hole loop — `it.next().transformToBoard` NPEs
        // on the null hole ENTRY (`Shape.java:544-545`, jar
        // t6-plane-nullhole THROWN) — uncaught, parse-fatal
        TransformedArea::Threw => return false,
    };
    let Some(current_layer) = area_first_layer(area) else {
        // Java `currentLayer.no` NPE at `:1086-1087` — reachable ONLY via
        // a PolylinePath first shape with a null layer (unknown layer
        // name); both the port and the jar die on the parse.
        return false;
    };
    if current_layer.no >= 0 {
        let clearance_class_index = match &area.clearance_class {
            Some(name) => {
                // a miss warns (log-only) and falls back to class 0
                sink.clearance_class_no(name).unwrap_or(0)
            }
            None => {
                // Java `currentNet.getNetClass().defaultItemClearanceClasses
                // .get(AREA)` (`:1100-1104`): the NET's class — for a
                // Task 6 parse the net was just created into the default
                // net class, so the sink's default AREA value matches;
                // the general net->class linkage lands with Task 8.
                sink.default_item_clearance_class(ItemClassIr::Area)
            }
        };
        if let Some(plane_area) = plane_area {
            sink.insert_conduction_area(ConductionAreaIr {
                layer_no: current_layer.no,
                area: plane_area,
                nets: vec![current_net_no],
                clearance_class: clearance_class_index,
                fixed: FixedStateIr::SystemFixed,
            });
        }
        // else (the Null flavor): BasicBoard.insert_conduction_area
        // rejects the null area with a log-only warning BEFORE inserting
        // (`BasicBoard.java:553-556`) — no item, no id burned, parse
        // continues.
        true
    } else {
        // Java `:1115-1120`: "Plane.read_scope: unexpected layer name" —
        // parse failure (jar t6-plane-signal: ParseError).
        false
    }
}

/// Java `Structure.insertMissingPowerPlanes` (`:526-571`): every
/// PARSER non-signal layer that carries `(use_net ...)` names and no
/// conduction area yet gets a full-bounding-box plane for its FIRST net
/// name (class 0, SYSTEM_FIXED, `contains_plane = true`). Missing net
/// names warn (log-only) and skip.
fn insert_missing_power_planes(
    layer_info: &[Layer],
    state: &mut ParseState,
    sink: &mut dyn BoardSink,
) {
    for current_layer in layer_info {
        if current_layer.is_signal {
            continue;
        }
        if sink.has_conduction_area_on_layer(current_layer.no) {
            continue;
        }
        let Some(current_net_name) = current_layer.net_names.first() else {
            continue;
        };
        add_plane_net(state, sink, current_net_name);
        let Some(current_net_no) = sink.net_no_subnet(current_net_name, 1) else {
            // Java `:552-558`: "insert_missing_power_planes: net not
            // found" (log-only), continue with the next layer.
            continue;
        };
        let Some(bounding_box) = sink.board_bounding_box() else {
            // Java `board.boundingBox` is never null post-create_board
            continue;
        };
        sink.insert_conduction_area(ConductionAreaIr {
            layer_no: current_layer.no,
            area: AreaIr::simple(box_shape(bounding_box)),
            nets: vec![current_net_no],
            clearance_class: 0, // BoardRules.clearanceClassNone()
            fixed: FixedStateIr::SystemFixed,
        });
    }
}

/// The layer of one parsed plane shape (`area_first_layer`'s ALL-shapes
/// twin — upstream's promotion set walks every shape, not just the
/// first). A failed shape read contributes nothing (Java would NPE on
/// the null entry; the port degrades — a null AREA is already
/// parse-fatal later at `insert_plane`, so skipping here only reorders
/// which face fires).
fn plane_shape_layers(area: &AreaScopeResult) -> impl Iterator<Item = &Layer> {
    area.shapes.iter().filter_map(|shape| match shape.as_ref() {
        Some(Shape::Rectangle(shape)) => Some(&shape.layer),
        Some(Shape::Polygon(shape)) => Some(&shape.layer),
        Some(Shape::Circle(shape)) => Some(&shape.layer),
        Some(Shape::PolylinePath(shape)) => shape.layer.as_ref(),
        Some(Shape::PolygonPath(shape)) => Some(&shape.layer),
        None => None,
    })
}

/// Upstream #935 `Structure.promotePowerLayersWithoutPlane`
/// (a917044ff, called from `createBoard` right after the layer-count
/// check): a non-signal layer is only meaningfully unroutable when it
/// really carries a plane — a `(plane ...)` scope covering it OR
/// `(use_net ...)` names on the layer. KiCad exports layers as
/// `(type power)` without the matching zone; keeping such a layer
/// unroutable silently removes it from the router, so every OTHER
/// non-signal layer is promoted to a signal layer here. Java logs
/// `FRLogger.warn("Layer '...' is declared as a power layer but no
/// plane is defined for it. It will be treated as a signal layer.")`
/// — log-only in Java, and this port keeps `state.warnings` at zero on
/// healthy parses (the parity-warnings law), so the message lives in
/// this comment. INTENTIONAL divergence from the e7f9bdf1 parity
/// point: digest-oracle records for planeless-power boards flip their
/// `layer_table.signal` field (see the porting protocol).
fn promote_power_layers_without_plane(plane_list: &[PlaneInfo], layer_info: &mut [Layer]) {
    let layers_with_plane: std::collections::BTreeSet<i32> = plane_list
        .iter()
        .filter_map(|plane| plane.area.as_ref())
        .flat_map(plane_shape_layers)
        .map(|layer| layer.no)
        .collect();
    for layer in layer_info {
        if !layer.is_signal && layer.net_names.is_empty() && !layers_with_plane.contains(&layer.no)
        {
            layer.is_signal = true;
        }
    }
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
    // Upstream #935 (`createBoard :1191-1194`): promote planeless
    // power layers to signal BEFORE the board is built, so the layer
    // table the sink receives already carries the fix.
    promote_power_layers_without_plane(&state.plane_list, &mut info.layer_info);
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
                component_id: 0,
                translation: epic_geometry::int_point::IntPoint::new(0, 0),
                rotation: 0.0,
                side_changed: false,
                name: None,
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
/// triggers the default class creation. Shared with the network reader
/// (`Network.addClearanceRule`, Task 8).
pub(crate) fn contains_wire_clearance_pair(pairs: &[String]) -> bool {
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
    use crate::sink::TraceIr;
    use crate::state::AngleRestriction;
    use epic_geometry::int_point::IntPoint;

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

    /// Upstream #935 (`promotePowerLayersWithoutPlane`, a917044ff): a
    /// planeless `(type power)` layer is promoted to signal — KiCad
    /// exports power layers without the matching zone, and keeping them
    /// unroutable silently removes them from the router. A power layer
    /// that CARRIES a plane stays non-signal. The `(use_net ...)` control
    /// arm is `use_net_reads_names_in_name_state` (net names keep
    /// `is_signal == false`). Intentional divergence from the e7f9bdf1
    /// parity point — the promotion's doc comment carries the
    /// classification.
    #[test]
    fn planeless_power_layer_promoted_plane_carrying_stays() {
        let (ok, state, board) = run_structure(
            "(layer F.Cu (type signal))\
             (layer In1.Cu (type power))\
             (layer B.Cu (type power))\
             (boundary (rect pcb 0 0 10000 8000))\
             (plane GND (rect In1.Cu 1000 1000 9000 7000))",
        );
        assert!(ok);
        let layers = board.layers.as_ref().expect("layers");
        assert!(layers.layers[0].is_signal, "F.Cu is signal");
        assert!(
            !layers.layers[1].is_signal,
            "In1.Cu carries the GND plane — stays a power layer"
        );
        assert!(
            layers.layers[2].is_signal,
            "B.Cu is planeless power — promoted to signal (upstream #935)"
        );
        assert!(
            state.warnings.is_empty(),
            "the promotion warns via FRLogger in Java (log-only); the \
             parity-warnings law keeps state.warnings empty on healthy parses"
        );
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

    // ---- Task 6: keepouts/planes/control (jar /tmp/epic-t6-board.out) ----

    /// The keepout payload by board id (ids are dense from 1, outline = 1).
    fn keepout_by_id(board: &SesBoard, id: i32) -> &KeepoutIr {
        for item in &board.items {
            if let crate::ses_board::ItemIr::Keepout {
                id: item_id,
                keepout,
            } = item
                && *item_id == id
            {
                return keepout;
            }
        }
        panic!("no keepout with id {id}");
    }

    fn bbox_of(area: &AreaIr) -> [i32; 4] {
        let bbox = area.border.bounding_box();
        [bbox.ll.x, bbox.ll.y, bbox.ur.x, bbox.ur.y]
    }

    fn conduction_by_id(board: &SesBoard, id: i32) -> &ConductionAreaIr {
        for item in &board.items {
            if let crate::ses_board::ItemIr::ConductionArea { id: item_id, area } = item
                && *item_id == id
            {
                return area;
            }
        }
        panic!("no conduction area with id {id}");
    }

    /// Jar FILE `/tmp/epic-t6-keep.dsn` (16 items): the signal keepout with
    /// a named class expands over all four PARSER signal layers (ids 2-5,
    /// class 2 = `power`), the nonexistent class falls back to 0 with a
    /// log-only warn (ids 6-9 — WARN_COUNT still 0), the named-layer and
    /// windowed keepouts insert per kind (`via_keepout` -> ViaObstacleArea
    /// id 15, `place_keepout` -> ComponentObstacleArea id 16), and the
    /// keepout loop order is keepout/via_keepout/place_keepout.
    #[test]
    fn keepout_full_battery_jar_probe() {
        let (ok, _state, board) = run_structure(
            "(layer F.Cu (type signal))\
             (layer IN1 (type signal))\
             (layer IN2 (type signal))\
             (layer B.Cu (type signal))\
             (boundary (rect pcb 0 0 10000 8000))\
             (rule (clearance 800 (type power-ground)))\
             (keepout (rect signal 1000 1000 2000 2000) (clearance_class power))\
             (keepout (rect signal 300 300 400 400) (clearance_class nonexistent))\
             (keepout (rect F.Cu 3000 1000 4000 2000))\
             (via_keepout (rect B.Cu 5000 1000 6000 2000))\
             (place_keepout (rect IN1 7000 1000 8000 2000))\
             (keepout (poly signal 0 500 5000 1500 5000 1500 6000 500 6000)\
                      (window (rect signal 800 5300 1200 5700)))",
        );
        assert!(ok);
        let layers = board.layers.as_ref().expect("layers");
        assert_eq!(layers.layers.len(), 4);
        assert_eq!(
            board.rules.clearance.names,
            ["null", "default", "power", "ground"]
        );
        assert_eq!(board.items.len(), 16);
        // ids 2-5: the `signal` keepout folded over the parser layers
        for (id, layer) in [(2, 0), (3, 1), (4, 2), (5, 3)] {
            let keepout = keepout_by_id(&board, id);
            assert_eq!(keepout.kind, KeepoutKindIr::Keepout);
            assert_eq!(keepout.layer_no, layer);
            assert_eq!(keepout.clearance_class, 2, "the power class");
            assert_eq!(keepout.fixed, FixedStateIr::SystemFixed);
            assert_eq!(bbox_of(&keepout.area), [10_000, 10_000, 20_000, 20_000]);
        }
        // ids 6-9: `nonexistent` class -> clearanceClassNone() = 0
        for (id, layer) in [(6, 0), (7, 1), (8, 2), (9, 3)] {
            let keepout = keepout_by_id(&board, id);
            assert_eq!(keepout.layer_no, layer);
            assert_eq!(keepout.clearance_class, 0, "class-not-found fallback");
            assert_eq!(bbox_of(&keepout.area), [3_000, 3_000, 4_000, 4_000]);
        }
        // id 10: the F.Cu keepout, default AREA item class
        let keepout = keepout_by_id(&board, 10);
        assert_eq!(keepout.layer_no, 0);
        assert_eq!(keepout.clearance_class, 1);
        assert_eq!(bbox_of(&keepout.area), [30_000, 10_000, 40_000, 20_000]);
        // ids 11-14: the windowed polygon keepout, hole preserved
        for (id, layer) in [(11, 0), (12, 1), (13, 2), (14, 3)] {
            let keepout = keepout_by_id(&board, id);
            assert_eq!(keepout.layer_no, layer);
            assert_eq!(keepout.clearance_class, 1);
            assert_eq!(bbox_of(&keepout.area), [5_000, 50_000, 15_000, 60_000]);
            assert_eq!(keepout.area.holes.len(), 1);
        }
        // id 15 / id 16: the via and place keepout kinds, in that order
        let keepout = keepout_by_id(&board, 15);
        assert_eq!(keepout.kind, KeepoutKindIr::ViaKeepout);
        assert_eq!(keepout.layer_no, 3);
        assert_eq!(bbox_of(&keepout.area), [50_000, 10_000, 60_000, 20_000]);
        let keepout = keepout_by_id(&board, 16);
        assert_eq!(keepout.kind, KeepoutKindIr::PlaceKeepout);
        assert_eq!(keepout.layer_no, 1);
        assert_eq!(bbox_of(&keepout.area), [70_000, 10_000, 80_000, 20_000]);
    }

    /// Jar `/tmp/epic-t6-degen.dsn`: the zero-area polygon (name `dz`) and
    /// the zero-width rectangle (no name -> 'null' in the warn) are
    /// SKIPPED with log-only warnings; only the healthy keepout survives
    /// (ids 2-3, both layers) and the parse still succeeds.
    #[test]
    fn degenerate_keepouts_are_skipped() {
        let (ok, _state, board) = run_structure(
            "(layer F.Cu (type signal))\
             (layer B.Cu (type signal))\
             (boundary (rect pcb 0 0 10000 8000))\
             (keepout dz (poly signal 0 2000 2000 2000 2000 2000 2000))\
             (keepout (rect signal 2000 2000 2000 3000))\
             (keepout ok (rect signal 1000 1000 2000 2000))",
        );
        assert!(ok);
        assert_eq!(board.items.len(), 3);
        for id in [2, 3] {
            let keepout = keepout_by_id(&board, id);
            assert_eq!(keepout.layer_no, id - 2);
            assert_eq!(keepout.clearance_class, 1);
            assert_eq!(bbox_of(&keepout.area), [10_000, 10_000, 20_000, 20_000]);
        }
    }

    /// Jar `/tmp/epic-t6-keep-circle.dsn`: a single-shape circle keepout
    /// is INSERTED, not degenerate-skipped — `transformAreaToBoard`
    /// returns it uncast and `Circle.dimension()` is 2 (`Circle.java:
    /// 36-42`). Bbox center +/- radius: jar `48500 38500 .. 51500 41500`.
    #[test]
    fn circle_keepout_is_inserted_not_skipped() {
        let (ok, _state, board) = run_structure(
            "(layer F.Cu (type signal))\
             (layer B.Cu (type signal))\
             (boundary (rect pcb 0 0 10000 8000))\
             (keepout (circle signal 300 5000 4000))",
        );
        assert!(ok);
        assert_eq!(board.items.len(), 3);
        for (id, layer) in [(2, 0), (3, 1)] {
            let keepout = keepout_by_id(&board, id);
            assert_eq!(keepout.layer_no, layer);
            assert_eq!(keepout.clearance_class, 1);
            assert!(matches!(
                keepout.area.border,
                crate::shape::BoardShape::Circle(_)
            ));
            assert_eq!(
                bbox_of(&keepout.area),
                [48_500, 38_500, 51_500, 41_500],
                "center (50000,40000) +/- radius 1500"
            );
        }
    }

    /// Jar `/tmp/epic-t6-keep-circle-hole.dsn`: a circle WINDOW makes the
    /// `PolylineArea` construction fail (warn "PolylineShape expected"),
    /// the null `keepoutArea` then NPEs at `:864` — parse-fatal parity.
    #[test]
    fn circle_window_keepout_is_a_parse_failure() {
        let (ok, _state, _board) = run_structure(
            "(layer F.Cu (type signal))\
             (layer B.Cu (type signal))\
             (boundary (rect pcb 0 0 10000 8000))\
             (keepout (poly signal 0 500 5000 1500 5000 1500 6000 500 6000)\
                      (window (circle signal 100 1000 5500)))",
        );
        assert!(!ok);
    }

    /// Jar `/tmp/epic-t6-keep-badshape.dsn`: a failed FIRST shape is
    /// appended as null (Java LinkedList) and NPEs at `area.shapeList`
    /// (`:862`, jar stack "Cannot read field \"shapeList\" because
    /// \"area\" is null") — parse-fatal parity.
    #[test]
    fn bad_first_shape_keepout_is_a_parse_failure() {
        let (ok, _state, _board) = run_structure(
            "(layer F.Cu (type signal))\
             (layer B.Cu (type signal))\
             (boundary (rect pcb 0 0 10000 8000))\
             (keepout (nosuchshape x))",
        );
        assert!(!ok);
    }

    /// Jar `/tmp/epic-t6-keep-signal.dsn` + `-signal-fold.dsn`: the exact
    /// lowercase `signal` name takes the expand branch, and the UPPERCASE
    /// `SIGNAL` (unknown -> signal FALLBACK in the shape reader) lands on
    /// the same expanded result — the fallback normalizes the case BEFORE
    /// the insert-time name test (jar: identical items for both files).
    #[test]
    fn signal_layer_keepout_expands_and_folds() {
        for layer_token in ["signal", "SIGNAL"] {
            let (ok, _state, board) = run_structure(&format!(
                "(layer F.Cu (type signal))\
                 (layer B.Cu (type signal))\
                 (boundary (rect pcb 0 0 10000 8000))\
                 (keepout (rect {layer_token} 1 1 2 2))"
            ));
            assert!(ok, "{layer_token}");
            assert_eq!(board.items.len(), 3, "{layer_token}");
            for (id, layer) in [(2, 0), (3, 1)] {
                let keepout = keepout_by_id(&board, id);
                assert_eq!(keepout.layer_no, layer, "{layer_token}");
                assert_eq!(bbox_of(&keepout.area), [10, 10, 20, 20]);
            }
        }
    }

    /// Jar `/tmp/epic-t6-keep-unknown.dsn`: an unknown layer name falls
    /// back to signal in the shape reader -> the keepout still EXPANDS
    /// over both layers (a wrong port failing the parse here diverges).
    #[test]
    fn unknown_layer_keepout_falls_back_to_signal() {
        let (ok, _state, board) = run_structure(
            "(layer F.Cu (type signal))\
             (layer B.Cu (type signal))\
             (boundary (rect pcb 0 0 10000 8000))\
             (keepout (rect nosuch 1 1 2 2))",
        );
        assert!(ok);
        assert_eq!(board.items.len(), 3);
        for (id, layer) in [(2, 0), (3, 1)] {
            let keepout = keepout_by_id(&board, id);
            assert_eq!(keepout.layer_no, layer);
            assert_eq!(bbox_of(&keepout.area), [10, 10, 20, 20]);
        }
    }

    /// Jar `/tmp/epic-t6-probe3.out` (`t6-pcb-keep.dsn`): the pcb
    /// pseudo-layer has no -1; `insertKeepout` warns "unknown layer name"
    /// and the parse FAILS (`:892-898`).
    #[test]
    fn pcb_layer_keepout_is_a_parse_error() {
        let (ok, _state, _board) = run_structure(
            "(layer F.Cu (type signal))\
             (layer B.Cu (type signal))\
             (boundary (rect pcb 0 0 10000 8000))\
             (keepout (rect pcb 1 1 2 2))",
        );
        assert!(!ok);
    }

    /// Jar FILE `/tmp/epic-t6-plane.dsn`: the plane battery. Parser nets
    /// are CASE-SENSITIVE (GND and gnd both exist) while the board net
    /// lookup is case-INSENSITIVE (`nets.get("gnd", 1)` -> net 1 — the
    /// second plane carries nets=[1]); all plane nets get
    /// `contains_plane = true`; the control/flip_style/snap_angle values
    /// land on the parse state and the board; the VDD window survives as
    /// a hole.
    #[test]
    fn plane_battery_jar_probe() {
        let (ok, state, board) = run_structure(
            "(layer F.Cu (type signal))\
             (layer B.Cu (type signal))\
             (boundary (rect pcb 0 0 10000 8000))\
             (control (via_at_smd on))\
             (flip_style rotate_first)\
             (snap_angle ninety_degree)\
             (keepout (rect F.Cu 100 100 200 200))\
             (plane GND (rect B.Cu 1000 1000 3000 3000))\
             (plane gnd (rect B.Cu 4000 1000 6000 3000))\
             (plane VDD (poly F.Cu 0 7000 1000 9000 1000 9000 3000 7000 3000)\
                  (window (rect F.Cu 7500 1500 8500 2500)))",
        );
        assert!(ok);
        assert!(state.via_at_smd_allowed, "(control (via_at_smd on))");
        assert_eq!(state.snap_angle, AngleRestriction::NinetyDegree);
        assert_eq!(board.metadata.flip_style.as_deref(), Some("rotate_first"));
        // NET_COUNT 3, all contains_plane=true (jar NET lines)
        assert_eq!(board.nets.len(), 3);
        assert_eq!(board.nets[0].name, "GND");
        assert_eq!(board.nets[1].name, "gnd", "case-sensitive parser list");
        assert_eq!(board.nets[2].name, "VDD");
        assert!(board.nets.iter().all(|net| net.contains_plane));
        assert_eq!(board.items.len(), 5);
        // keepout id 2 (F.Cu, default item class)
        let keepout = keepout_by_id(&board, 2);
        assert_eq!(keepout.layer_no, 0);
        assert_eq!(keepout.clearance_class, 1);
        assert_eq!(bbox_of(&keepout.area), [1_000, 1_000, 2_000, 2_000]);
        // conduction ids 3/4: GND and gnd BOTH resolve to board net 1
        let area = conduction_by_id(&board, 3);
        assert_eq!(area.layer_no, 1);
        assert_eq!(area.nets, [1]);
        assert_eq!(area.clearance_class, 1);
        assert_eq!(area.fixed, FixedStateIr::SystemFixed);
        assert_eq!(bbox_of(&area.area), [10_000, 10_000, 30_000, 30_000]);
        let area = conduction_by_id(&board, 4);
        assert_eq!(area.layer_no, 1);
        assert_eq!(area.nets, [1], "case-insensitive board lookup of 'gnd'");
        assert_eq!(bbox_of(&area.area), [40_000, 10_000, 60_000, 30_000]);
        // conduction id 5: the VDD polygon with its window hole
        let area = conduction_by_id(&board, 5);
        assert_eq!(area.layer_no, 0);
        assert_eq!(area.nets, [3]);
        assert_eq!(bbox_of(&area.area), [70_000, 10_000, 90_000, 30_000]);
        assert_eq!(area.area.holes.len(), 1);
    }

    /// Jar `/tmp/epic-t6-plane-badnet.dsn`: `(plane 123 ...)` — the NAME
    /// lexical state scans digits as a string, so the net "123" is a
    /// successful parse with a conduction area on B.Cu.
    #[test]
    fn plane_net_name_numeric_string() {
        let (ok, _state, board) = run_structure(
            "(layer F.Cu (type signal))\
             (layer B.Cu (type signal))\
             (boundary (rect pcb 0 0 10000 8000))\
             (keepout (rect F.Cu 100 100 200 200))\
             (plane 123 (rect B.Cu 1 1 2 2))",
        );
        assert!(ok);
        assert_eq!(board.nets.len(), 1);
        assert_eq!(board.nets[0].name, "123");
        assert!(board.nets[0].contains_plane);
        let area = conduction_by_id(&board, 3);
        assert_eq!(area.layer_no, 1);
        assert_eq!(area.nets, [1]);
        assert_eq!(bbox_of(&area.area), [10, 10, 20, 20]);
    }

    /// Jar `/tmp/epic-t6-plane-signal.dsn`: a plane on the `signal`
    /// pseudo-layer (no >= 0 fails) warns "unexpected layer name" and the
    /// parse FAILS (`:1115-1120`) — the net was still added first.
    #[test]
    fn plane_on_signal_layer_is_a_parse_error() {
        let (ok, _state, _board) = run_structure(
            "(layer F.Cu (type signal))\
             (layer B.Cu (type signal))\
             (boundary (rect pcb 0 0 10000 8000))\
             (plane GND (rect signal 1 1 2 2))",
        );
        assert!(!ok);
    }

    /// Jar `/tmp/epic-t6-plane-badlayer.dsn`: an unknown plane layer falls
    /// back to signal in the shape reader, so the plane dies on the SAME
    /// unexpected-layer branch (`:1115-1120`).
    #[test]
    fn plane_unknown_layer_falls_back_to_signal_then_fails() {
        let (ok, _state, _board) = run_structure(
            "(layer F.Cu (type signal))\
             (layer B.Cu (type signal))\
             (boundary (rect pcb 0 0 10000 8000))\
             (plane STRAY (rect nosuch 1 1 2 2))",
        );
        assert!(!ok);
    }

    /// Jar `/tmp/epic-t6-plane-badshape.dsn`: a failed plane area is
    /// stored UNCHECKED (`Plane.java:79-81`) and NPEs at
    /// `planeInfo.area.shapeList` (`:1084`, jar stack) — parse-fatal
    /// parity, but only AFTER the net was added.
    #[test]
    fn plane_bad_shape_is_a_parse_failure() {
        let (ok, _state, _board) = run_structure(
            "(layer F.Cu (type signal))\
             (layer B.Cu (type signal))\
             (boundary (rect pcb 0 0 10000 8000))\
             (plane GND (nosuchshape x))",
        );
        assert!(!ok);
    }

    /// Jar `/tmp/epic-t6-probe3.out` (`t6-plane-circwin.dsn`): a circle
    /// border + window fails the PolylineArea construction, the null area
    /// is rejected by `BasicBoard.insertConductionArea` BEFORE burning an
    /// id — Success, NO item, but the plane net still exists.
    #[test]
    fn plane_circle_border_with_window_inserts_nothing() {
        let (ok, _state, board) = run_structure(
            "(layer F.Cu (type signal))\
             (layer B.Cu (type signal))\
             (boundary (rect pcb 0 0 10000 8000))\
             (plane GND (circle F.Cu 300 5000 4000) (window (rect F.Cu 4900 3900 5100 4100)))",
        );
        assert!(ok);
        assert_eq!(board.items.len(), 1, "outline only — no conduction item");
        assert_eq!(board.nets.len(), 1);
        assert_eq!(board.nets[0].name, "GND");
        assert!(board.nets[0].contains_plane);
    }

    /// Jar `/tmp/epic-t6-nullhole.out` (`t6-plane-nullhole.dsn`): a
    /// FAILED window shape is appended as a null hole ENTRY
    /// (`Shape.java:222-223`) and THROWS at
    /// `it.next().transformToBoard` (`:544-545`) — parse-fatal at the
    /// plane site too, unlike the circle/null-transform flavor that only
    /// skips the plane.
    #[test]
    fn plane_failed_window_hole_is_a_parse_failure() {
        let (ok, _state, _board) = run_structure(
            "(layer F.Cu (type signal))\
             (layer B.Cu (type signal))\
             (boundary (rect pcb 0 0 10000 8000))\
             (plane GND (rect F.Cu 1000 1000 3000 3000)\
                (window (nosuchshape x)))",
        );
        assert!(!ok);
    }

    /// Jar `/tmp/epic-t6-nullhole.out` (`t6-keep-nullhole.dsn`): the same
    /// null hole ENTRY at the keepout site — THROWN NPE
    /// (`Shape.java:544-545`), parse-fatal parity.
    #[test]
    fn keepout_failed_window_hole_is_a_parse_failure() {
        let (ok, _state, _board) = run_structure(
            "(layer F.Cu (type signal))\
             (layer B.Cu (type signal))\
             (boundary (rect pcb 0 0 10000 8000))\
             (keepout (poly signal 0 500 5000 1500 5000 1500 6000 500 6000)\
                      (window (nosuchshape x)))",
        );
        assert!(!ok);
    }

    /// Jar `/tmp/epic-t6-nullhole.out` (`t6-plane-mixed-holes.dsn` vs
    /// `-mixed2-holes.dsn`): the hole loop stops at the FIRST failing
    /// hole, so the flavors are ORDER-SENSITIVE — circle-then-bad
    /// survives (the null-return flavor: Success, ITEM_COUNT 1, NET 1 GND
    /// contains_plane=true) while bad-then-circle THROWS the NPE. A port
    /// folding the flavors — or scanning the holes order-blindly —
    /// diverges on exactly one of the two.
    #[test]
    fn plane_hole_flavors_are_order_sensitive() {
        let layers = "(layer F.Cu (type signal))\
                      (layer B.Cu (type signal))\
                      (boundary (rect pcb 0 0 10000 8000))";
        // circle hole first: instanceof fails at hole 0, null return,
        // BasicBoard skips the plane — parse continues
        let (ok, _state, board) = run_structure(&format!(
            "{layers}\
             (plane GND (rect F.Cu 1000 1000 5000 5000)\
                (window (circle F.Cu 100 2000 2000))\
                (window (nosuchshape x)))"
        ));
        assert!(ok, "circle-first must survive");
        assert_eq!(board.items.len(), 1, "outline only — plane skipped");
        assert_eq!(board.nets.len(), 1);
        assert_eq!(board.nets[0].name, "GND");
        assert!(board.nets[0].contains_plane);

        // null entry first: the NPE at hole 0 wins — parse dies
        let (ok, _state, _board) = run_structure(&format!(
            "{layers}\
             (plane GND (rect F.Cu 1000 1000 5000 5000)\
                (window (nosuchshape x))\
                (window (circle F.Cu 100 2000 2000)))"
        ));
        assert!(!ok, "null-entry-first must die");
    }

    /// Jar `/tmp/epic-t6-probe3.out` (`t6-ctrl-garbage.dsn`): the
    /// `(via_at_smd garbage)` value is not `on` -> false (the readOnOffScope
    /// warn is log-only), the scope is drained, and the later
    /// `(snap_angle ninety_degree)` still parses: Success, ANGLE
    /// NINETY_DEGREE.
    #[test]
    fn control_via_at_smd_garbage_drains_scope() {
        let (ok, state, board) = run_structure(
            "(layer F.Cu (type signal))\
             (layer B.Cu (type signal))\
             (boundary (rect pcb 0 0 10000 8000))\
             (control (via_at_smd garbage))\
             (snap_angle ninety_degree)",
        );
        assert!(ok);
        assert!(!state.via_at_smd_allowed);
        assert_eq!(state.snap_angle, AngleRestriction::NinetyDegree);
        assert_eq!(board.items.len(), 1);
    }

    /// Dispatch pins read verbatim (`Structure.java:1015`, `:1018-1021`):
    /// a FAILED snap_angle read keeps the PREVIOUS value (a port resetting
    /// to the fortyfive default reads FORTYFIVE here and fails), and a
    /// second flip_style scope OVERWRITES the local — a failed read after
    /// a successful one clears it.
    #[test]
    fn snap_angle_keeps_previous_and_flip_style_overwrites() {
        let (ok, state, board) = run_structure(
            "(layer F.Cu (type signal))\
             (layer B.Cu (type signal))\
             (boundary (rect pcb 0 0 10000 8000))\
             (snap_angle ninety_degree)\
             (snap_angle bogus)\
             (flip_style rotate_first)\
             (flip_style rotate_first extra)",
        );
        assert!(ok);
        assert_eq!(state.snap_angle, AngleRestriction::NinetyDegree);
        assert_eq!(
            board.metadata.flip_style, None,
            "the failed second flip_style scope overwrote the first"
        );
    }

    /// Jar FILE `/tmp/epic-t6-power.dsn`: `insertMissingPowerPlanes` —
    /// every non-signal layer with `(use_net ...)` and no conduction area
    /// gets a full-bounds plane (class 0, SYSTEM_FIXED) for its FIRST net
    /// name only; VDD is shared by PWR and PWR2 (one net, the second
    /// netlist guard hits), PWR3 (no names) is skipped, GND2 never added.
    #[test]
    fn insert_missing_power_planes_jar_probe() {
        let (ok, _state, board) = run_structure(
            "(layer F.Cu (type signal))\
             (layer PWR (type power) (use_net VDD))\
             (layer PWR2 (type power) (use_net VDD GND2))\
             (layer PWR3 (type power))\
             (layer B.Cu (type signal))\
             (boundary (rect pcb 0 0 10000 8000))",
        );
        assert!(ok);
        assert_eq!(board.nets.len(), 1);
        assert_eq!(board.nets[0].name, "VDD");
        assert!(board.nets[0].contains_plane);
        assert_eq!(board.items.len(), 3);
        for (id, layer) in [(2, 1), (3, 2)] {
            let area = conduction_by_id(&board, id);
            assert_eq!(area.layer_no, layer);
            assert_eq!(area.nets, [1]);
            assert_eq!(area.clearance_class, 0);
            assert_eq!(area.fixed, FixedStateIr::SystemFixed);
            assert_eq!(
                bbox_of(&area.area),
                [-1000, -1000, 101_000, 81_000],
                "board.boundingBox = bounds incl. offset(1000)"
            );
        }
    }

    /// Jar FILE `/tmp/epic-t6-t37-fire.dsn` (T37, seed mirrors the jar
    /// wiring order): only the IN1 plane fires — interior signal layer,
    /// no wires, 5.525E9 >= 0.5 * 8.0E9 board area — promoting its net to
    /// `contains_plane` and the area to USER_FIXED; the IN2 area is vetoed
    /// by the trace on its layer, IN3 by the area gate (2.25E8), F.Cu and
    /// B.Cu by the exterior/last-layer gate. Jar: NET 1 cp=true, INFO
    /// "Layer 'IN1' has been automatically configured ...", conduction
    /// id 6 fixed=USER_FIXED, ids 8-11 UNFIXED.
    #[test]
    fn adjust_plane_autoroute_settings_jar_probe() {
        let (ok, _state, mut board) = run_structure(
            "(layer F.Cu (type signal))\
             (layer IN1 (type signal))\
             (layer IN2 (type signal))\
             (layer IN3 (type signal))\
             (layer B.Cu (type signal))\
             (boundary (path pcb 0 0 0 10000 0 10000 8000 0 8000 0 0))",
        );
        assert!(ok);
        for name in ["PERFECT", "NET2", "NET3", "NET4"] {
            board.append_net(NetIr {
                name: name.to_string(),
                subnet_number: 1,
                contains_plane: false,
                net_class: 0,
            });
        }
        let big = AreaIr::simple(box_shape(IntBox::new(
            IntPoint::new(10_000, 10_000),
            IntPoint::new(95_000, 75_000),
        )));
        let small = AreaIr::simple(box_shape(IntBox::new(
            IntPoint::new(10_000, 10_000),
            IntPoint::new(25_000, 25_000),
        )));
        fn plane(board: &mut SesBoard, layer_no: i32, nets: Vec<i32>, area: &AreaIr) {
            board.insert_conduction_area(ConductionAreaIr {
                layer_no,
                area: area.clone(),
                nets,
                clearance_class: 1,
                fixed: FixedStateIr::Unfixed,
            });
        }
        // the t37 wiring order: IN1 area, IN2 trace + area, IN3 area,
        // F.Cu area, B.Cu area
        plane(&mut board, 1, vec![1], &big);
        board.insert_trace(TraceIr {
            layer_no: 2,
            half_width: 500,
            corners: vec![IntPoint::new(20_000, 20_000), IntPoint::new(30_000, 20_000)],
            polyline: TraceIr::polyline_of_corners(&[
                IntPoint::new(20_000, 20_000),
                IntPoint::new(30_000, 20_000),
            ]),
            nets: vec![2],
            clearance_class: 1,
            fixed: FixedStateIr::UserFixed,
        });
        plane(&mut board, 2, vec![2], &big);
        plane(&mut board, 3, vec![3], &small);
        plane(&mut board, 0, vec![4], &big);
        plane(&mut board, 4, vec![4], &big);
        assert!(board.adjust_plane_autoroute_settings());
        assert!(board.nets[0].contains_plane, "PERFECT promoted");
        assert!(!board.nets[1].contains_plane);
        assert!(!board.nets[2].contains_plane);
        assert!(!board.nets[3].contains_plane);
        let fixed_by_layer: Vec<(i32, FixedStateIr)> = board
            .items
            .iter()
            .filter_map(|item| match item {
                crate::ses_board::ItemIr::ConductionArea { area, .. } => {
                    Some((area.layer_no, area.fixed))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            fixed_by_layer,
            [
                (1, FixedStateIr::UserFixed),
                (2, FixedStateIr::Unfixed),
                (3, FixedStateIr::Unfixed),
                (0, FixedStateIr::Unfixed),
                (4, FixedStateIr::Unfixed),
            ]
        );
    }

    /// The heuristic early gates (code-read `DsnFile.java:33-45`): no
    /// board at all (Java `routingBoard == null`), <= 2 layers, and any
    /// non-signal layer all return false before scanning items.
    #[test]
    fn adjust_plane_autoroute_settings_early_gates() {
        let mut board = SesBoard::new();
        assert!(!board.adjust_plane_autoroute_settings(), "no board");

        let (ok, _state, mut two_layer) = run_structure(
            "(layer F.Cu (type signal))\
             (layer B.Cu (type signal))\
             (boundary (rect pcb 0 0 10000 8000))",
        );
        assert!(ok);
        assert!(!two_layer.adjust_plane_autoroute_settings(), "2 layers");

        let (ok, _state, mut power_layer) = run_structure(
            "(layer F.Cu (type signal))\
             (layer PWR (type power) (use_net VDD))\
             (layer B.Cu (type signal))\
             (boundary (rect pcb 0 0 10000 8000))",
        );
        assert!(ok);
        // PARITY DECISION (upstream #935, a917044ff): this arm originally
        // used a planeless nameless `(type power)` layer, which the
        // promotion now turns into a signal layer — the re-pinned form
        // keeps the layer non-signal the way Java would see ANY power
        // layer: via `(use_net ...)` names (net names block the
        // promotion, see `promote_power_layers_without_plane`).
        assert!(
            !power_layer.adjust_plane_autoroute_settings(),
            "non-signal layer"
        );
    }
}
