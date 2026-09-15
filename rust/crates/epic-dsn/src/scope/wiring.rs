//! Port of `io.specctra.parser.Wiring` — the `(wiring ...)` scope:
//! `readScope` (`:307-354`), `readWireScope` (`:356-577`), `readViaScope`
//! (`:601-714`), `calcFixed` (`:255-275`), `readNetId` (`:281-305`) and
//! `getSubnets` (`:223-236`).
//!
//! ## D11 deferrals (documented at the scope exit)
//!
//! - `board.normalizeAllTraces()` (the `readScope` tail, `:338-354`) is
//!   deliberately NOT ported — normalization is board intelligence (D11).
//!   Only its CATCH ARM survives: Java wraps the call in try/catch and a
//!   null board NPEs inside the try, producing the warning
//!   `"Wiring: normalization of traces failed"` + `true`. The Rust tail
//!   fires exactly that warning when `!sink.has_board()`
//!   (jar `/tmp/epic-t9-fresh.out` CASE npefold: wiring before structure
//!   folds to **Success** with that warning, not a parse error — the
//!   wire itself dropped earlier at the no-shape check because the layer
//!   structure does not exist yet).
//! - `tryCorrectNet` (`:583-599`) is NOT ported (D11): it reassigns
//!   empty-net items to net 0 post-insert; unobservable through the parse
//!   surface in scope.
//!
//! Consequence (phantom id): because normalization is skipped, the
//! t37nofire fixture keeps the In1.Cu trace that Java's
//! `normalizeAllTraces` removes — but Java still burns the item id in the
//! `Item` constructor BEFORE the removal, so jar `GEN_MAX 4` with 3
//! stored items; Rust keeps all 4 items (same max id). Pinned in
//! [`crate::reader`] tests.
//!
//! ## calcFixed desync (jar-verified, `/tmp/epic-t9-{fresh,cfx}.out`)
//!
//! The `(type` token stream is `Open, Keyword(Type), Close`; the wire
//! loop dispatches on the KEYWORD, so `calcFixed` reads `Close(type)` as
//! its first token and the NEXT stream token as its bracket check:
//! - `(type )` at wire end: first = `Close(type)` (≠ shove_fixed/fix/
//!   normal → USER_FIXED), second = the wire's own `Close` → clean USER.
//!   Both closes consumed. The still-active wire loop then reads the next
//!   wire's `Open`+`Keyword(Wire)` into the `_` skip_scope arm and
//!   swallows that wire WHOLE inside the same `readWireScope` (jar cf-b:
//!   3 wires → GEN 3 with the third gone; cf-d: GEN 7).
//! - `(type fix garbage)`: first = `Fix` → SYSTEM, second = the garbage
//!   string → log-only `is_fixed` warn + UNFIXED. `Close(type)` is never
//!   read: the wire loop breaks on IT, and the wire's own `Close` leaks
//!   to the WIRING scope, which ends early — every following wire is
//!   stranded at PCB level and lost (jar calcfixed/cf-e: wires 7-8 gone,
//!   GEN 7; cf-f/g/h: GEN 2).
//! - `(type )(net GND)`: first = `Close(type)` → USER, second = `Open`
//!   → warn + UNFIXED; the loop flies past `Keyword(Net)`/string and
//!   breaks at `Close(net)`; the wire inserts UNFIXED with its ORIGINAL
//!   net, and its close leaks exactly as above (jar cf-j: GEN 2).
//!
//! ## T47 reconciliation (Task-1 trap shape)
//!
//! A `(class ...)` before `(wiring ...)` loses every wire — but NOT here:
//! the loss is the NETWORK scope's bare-class close-eating cascade
//! (t8_deg1/t8_deg2 in `scope/network.rs`), not a wiring veto. Equivalence
//! is pinned at the dispatcher level by the t47 test in `crate::reader`
//! (jar `/tmp/epic-t9fix-t47.out`).
//!
//! ## Scanner alias
//!
//! The flex lexer maps plain `path` to `Keyword.POLYGON_PATH`
//! (`SpecctraFileDescription.flex`: `"path" { yybegin(LAYER_NAME); return
//! Keyword.POLYGON_PATH; }`), so `(path ...)` and `(polygon_path ...)`
//! dispatch to the same arm — the Rust lexer mirrors it
//! ([`crate::keyword`] `b"path" => Keyword::PolygonPath`). `polyline_path`
//! has its own arm.
//!
//! ## Documented divergences
//!
//! - Java NPEs (`board.rules` `:437`, `board.library.padstacks` `:660`,
//!   a null `PolylinePath.layer` `:453`, a null transform) escape
//!   `readBoard` UNCAUGHT — jar stack traces in
//!   `/tmp/epic-t9-{fresh,cfk}.out`. The Rust port classifies them as
//!   read failures (`false`), which surface as `OutlineMissing` or
//!   `ParseError` depending on how far the walk got. Notably
//!   `BasicBoard`'s no-arg `BoardLibrary()` leaves `padstacks` null
//!   until a `(library ...)` scope runs (`Library.java:264`), so a via in
//!   a library-less DSN NPEs even with a live board (jar cf-k) where the
//!   Rust empty registry yields the missing-padstack warning + `false`.
//! - Polyline corners keep exact Rationals in Java; the port rounds at
//!   parse time (`corner_to_int`) — the digest surface is int-valued
//!   either way.
//! - Java's via `netNumbers` loop never increments its write index
//!   (`:684-689`), so a multi-subnet via stores `[lastNet, 0, ...]` —
//!   bug-compatibly reproduced.

use crate::keyword::{Keyword, skip_scope};
use crate::lexer::{Scanner, Token};
use crate::scope::network::{padstack_from_layer, padstack_to_layer};
use crate::scope::structure::read_string_scope;
use crate::shape::{self, Shape};
use crate::sink::{BoardSink, ConductionAreaIr, FixedStateIr, ItemClassIr, TraceIr, ViaIr};
use crate::state::{NetId, ParseState};
use epic_geometry::int_box::IntBox;
use epic_geometry::int_point::IntPoint;
use epic_geometry::line::Line;
use epic_geometry::point::Point;
use epic_geometry::polygon::Polygon;
use epic_geometry::polyline::Polyline;

/// Java `Wiring.readScope` (`:307-354`). Returns `false` only where Java
/// fails the read (IOException arm, EOF arm, a failing via scope); a
/// dropped wire is Java `null` + continue.
pub fn read_scope(scanner: &mut Scanner, state: &mut ParseState, sink: &mut dyn BoardSink) -> bool {
    // Java inits nextToken = null — `None` models that (the first
    // iteration must not take the `prev == Open` arm).
    let mut next_token: Option<Token> = None;
    loop {
        let prev_token = next_token;
        next_token = match scanner.next_token() {
            Token::Error(_) => {
                // `:311-314` IOException catch: log-only error + false —
                // the wiring scope is fatal.
                return false;
            }
            Token::Eof => {
                // `:308-310` null -> log-only warn + false.
                return false;
            }
            Token::Close => break,
            token => Some(token),
        };
        let mut read_ok = true;
        if prev_token == Some(Token::Open) {
            match next_token {
                Some(Token::Keyword(Keyword::Wire)) => {
                    // Java discards the returned Item (`:333-335`); a
                    // `false` here is the Java-thrown NPE class.
                    read_ok = read_wire_scope(scanner, state, sink);
                }
                Some(Token::Keyword(Keyword::Via)) => {
                    // `:336-337`: only the via scope's false propagates.
                    read_ok = read_via_scope(scanner, state, sink);
                }
                _ => {
                    let _ = skip_scope(scanner);
                }
            }
        }
        if !read_ok {
            return false;
        }
    }
    if !sink.has_board() {
        // `:338-354`: Java calls board.normalizeAllTraces() inside
        // try/catch; the null-board NPE lands in the catch -> warning +
        // true. The normalization itself is NOT ported (module docs).
        state
            .warnings
            .push("Wiring: normalization of traces failed".to_string());
    }
    true
}

/// Java `Wiring.readWireScope` (`:356-577`). `true` = processed or
/// dropped (Java returned an Item or null); `false` = Java THREW (the
/// no-board / no-transform NPEs, a failed area transform) and the whole
/// read dies.
fn read_wire_scope(
    scanner: &mut Scanner,
    state: &mut ParseState,
    sink: &mut dyn BoardSink,
) -> bool {
    let mut net_id: Option<NetId> = None;
    let mut clearance_class_name: Option<String> = None;
    let mut fixed = FixedStateIr::Unfixed;
    // `path` holds the PolygonPath/PolylinePath variants; each path arm
    // ASSIGNS (a later shape overwrites an earlier one, `:385-389`).
    let mut path: Option<Shape> = None;
    let mut border_shape: Option<Shape> = None;
    // The WINDOW arm adds the hole UNCONDITIONALLY — even a null (None)
    // entry (`:401-402`).
    let mut hole_list: Vec<Option<Shape>> = Vec::new();
    let mut next_token: Option<Token> = None;
    loop {
        let prev_token = next_token;
        next_token = match scanner.next_token() {
            Token::Error(_) => {
                // `:368-371` IOException catch: log-only error, wire
                // dropped.
                return true;
            }
            Token::Eof => {
                // `:372-379` null -> log-only warn, wire dropped.
                return true;
            }
            Token::Close => break,
            token => Some(token),
        };
        if prev_token == Some(Token::Open) {
            match next_token {
                Some(Token::Keyword(Keyword::PolygonPath)) => {
                    // Plain `path` arrives here too (module docs, the
                    // scanner alias).
                    path = shape::read_polygon_path_scope(scanner, state.layer_structure.as_ref())
                        .map(Shape::PolygonPath);
                }
                Some(Token::Keyword(Keyword::PolylinePath)) => {
                    path = shape::read_polyline_path_scope(scanner, state.layer_structure.as_ref())
                        .map(Shape::PolylinePath);
                }
                Some(Token::Keyword(Keyword::Rectangle)) => {
                    border_shape =
                        shape::read_rectangle_scope(scanner, state.layer_structure.as_ref())
                            .map(Shape::Rectangle);
                }
                Some(Token::Keyword(Keyword::Polygon)) => {
                    border_shape =
                        shape::read_polygon_scope(scanner, state.layer_structure.as_ref())
                            .map(Shape::Polygon);
                }
                Some(Token::Keyword(Keyword::Circle)) => {
                    border_shape =
                        shape::read_circle_scope(scanner, state.layer_structure.as_ref())
                            .map(Shape::Circle);
                }
                Some(Token::Keyword(Keyword::Window)) => {
                    // `:401-402`: Shape.readScope (with its optional-bracket
                    // overread) + the hole added unconditionally.
                    let hole_shape = shape::read_scope(scanner, state.layer_structure.as_ref());
                    hole_list.push(hole_shape);
                    // Overread the closing bracket (`:404-417`); a mismatch
                    // drops the wire (Java returns null).
                    match scanner.next_token() {
                        Token::Error(_) => return true,
                        Token::Close => next_token = Some(Token::Close),
                        _ => {
                            // log-only "closing bracket expected".
                            return true;
                        }
                    }
                }
                Some(Token::Keyword(Keyword::Net)) => {
                    net_id = Some(read_net_id(scanner));
                }
                Some(Token::Keyword(Keyword::ClearanceClass)) => {
                    clearance_class_name = Some(read_string_scope(scanner));
                }
                Some(Token::Keyword(Keyword::Type)) => {
                    fixed = calc_fixed(scanner);
                }
                // Everything else — including a nested `(wire` keyword
                // after a calcFixed desync — is skip-scoped whole
                // (`:427-429`).
                _ => {
                    let _ = skip_scope(scanner);
                }
            }
        }
    }
    if path.is_none() && border_shape.is_none() {
        // `:435-443`: warning + Java null (the wire is dropped; the scope
        // continues).
        state.warnings.push(format!(
            "Wiring: wire has no shape at '{}'",
            scanner.scope_identifier()
        ));
        return true;
    }
    // `:437` `board.rules.getDefaultNetClass()` NPEs on a null board —
    // the NPE propagates uncaught (module docs).
    if !sink.has_board() {
        return false;
    }
    // `:454`/:483/:497/:556/:559 use the transform; None only before
    // create_board (same condition as the board guard).
    let Some(transform) = state.coordinate_transform else {
        return false;
    };
    // `:438-447`: the net class starts at the default (class 0) and is
    // overwritten per found net (last wins).
    let mut net_class_no = 0i32;
    let found_nets = get_subnets(sink, net_id.as_ref());
    let mut net_numbers: Vec<i32> = Vec::with_capacity(found_nets.len());
    for net_no in &found_nets {
        net_numbers.push(*net_no);
        net_class_no = match sink.nets().get((*net_no - 1) as usize) {
            Some(net) => net.net_class,
            // Java `rules.nets.get(int)` would AIOOBE — unreachable (the
            // numbers come from the same table).
            None => return false,
        };
    }
    let mut clearance_class = -1i32;
    if let Some(name) = &clearance_class_name {
        // `ClearanceMatrix.getNo` — case-insensitive, -1 on a miss.
        clearance_class = sink.clearance_class_no(name).unwrap_or(-1);
    }
    // `:449-459`: the path wins for layer/half-width even when a border
    // shape is also present.
    let (layer_no, half_width, layer_name);
    match &path {
        Some(Shape::PolygonPath(polygon_path)) => {
            layer_no = polygon_path.layer.no;
            half_width = epic_geometry::rounding::java_round(
                transform.dsn_to_board_value(polygon_path.width / 2.0),
            ) as i32;
            layer_name = polygon_path.layer.name.clone();
        }
        Some(Shape::PolylinePath(polyline_path)) => {
            // `:453` `layerIndex = path.layer.no` NPEs on a null layer
            // (module docs).
            let Some(layer) = &polyline_path.layer else {
                return false;
            };
            layer_no = layer.no;
            half_width = epic_geometry::rounding::java_round(
                transform.dsn_to_board_value(polyline_path.width / 2.0),
            ) as i32;
            layer_name = layer.name.clone();
        }
        _ => {
            let border = border_shape.as_ref().expect("checked above");
            layer_no = border_shape_layer_no(border);
            half_width = 0;
            layer_name = border_shape_layer_name(border);
        }
    }
    if layer_no < 0 || layer_no >= sink.layer_count() {
        // `:461-476`: warning + Java null (wire dropped).
        state.warnings.push(format!(
            "Wiring: wire ignored — unknown layer '{}' at '{}'",
            layer_name,
            scanner.scope_identifier()
        ));
        return true;
    }
    // `:478` `board.getBoundingBox()` — set by create_board.
    let Some(bounding_box) = sink.board_bounding_box() else {
        return false;
    };
    if border_shape.is_some() {
        // `:481-499`: the border branch wins over the path.
        if clearance_class < 0 {
            clearance_class = net_class_item_clearance(sink, net_class_no, ItemClassIr::Area);
        }
        let mut area: Vec<Option<Shape>> = Vec::with_capacity(1 + hole_list.len());
        area.push(border_shape.clone());
        area.extend(hole_list.iter().cloned());
        match shape::transform_area_to_board(&area, &transform) {
            shape::TransformedArea::Area(area_ir) => {
                sink.insert_conduction_area(ConductionAreaIr {
                    layer_no,
                    area: area_ir,
                    nets: net_numbers,
                    clearance_class,
                    fixed,
                });
            }
            // Java `insertConductionArea(null, ...)` warns log-only and
            // returns null — no item, no id (`BasicBoard.java:552-556`).
            shape::TransformedArea::Null => {}
            // Java transformAreaToBoard THREW — parse-fatal.
            shape::TransformedArea::Threw => return false,
        }
    } else if let Some(Shape::PolygonPath(polygon_path)) = &path {
        // `:500-545`.
        if clearance_class < 0 {
            clearance_class = net_class_item_clearance(sink, net_class_no, ItemClassIr::Trace);
        }
        let corner_count = polygon_path.coordinate_arr.len() / 2;
        let mut corners: Vec<Point> = Vec::with_capacity(corner_count);
        for index in 0..corner_count {
            let current_point = [
                polygon_path.coordinate_arr[2 * index],
                polygon_path.coordinate_arr[2 * index + 1],
            ];
            let current_corner = transform.dsn_to_board_tuple(current_point);
            if !int_box_contains_float(&bounding_box, current_corner.x, current_corner.y) {
                // `:514-526`: RAW DSN coordinates, (int)-truncated, in the
                // message; the wire is dropped.
                state.warnings.push(format!(
                    "Wiring: wire corner ({},{}) is outside board bounds at '{}'",
                    current_point[0] as i32,
                    current_point[1] as i32,
                    scanner.scope_identifier()
                ));
                return true;
            }
            corners.push(Point::Int(current_corner.round()));
        }
        let polygon = Polygon::new(&corners);
        let polygon_corners = polygon.corner_array();
        // `:531-545`: degenerate = fewer than 2 corners, or every corner
        // identical (zero-length trace). Java logs FRLogger.debug and adds
        // the WARNING to the parity list; no item, no id.
        let degenerate = polygon_corners.len() < 2
            || !polygon_corners[1..]
                .iter()
                .any(|corner| corner != &polygon_corners[0]);
        if degenerate {
            state.warnings.push(format!(
                "Wiring: degenerate wire trace skipped (all {} corners are identical — zero-length trace) on layer '{}'. This is likely a DSN export issue in your EDA tool.",
                polygon_corners.len(),
                layer_name
            ));
        } else {
            let trace_polyline = Polyline::from_polygon(&polygon);
            // Traces are not normalized here (module docs, D11).
            insert_trace(
                sink,
                &trace_polyline,
                layer_no,
                half_width,
                net_numbers,
                clearance_class,
                fixed,
            );
        }
    } else if let Some(Shape::PolylinePath(polyline_path)) = &path {
        // `:546-565`.
        if clearance_class < 0 {
            clearance_class = net_class_item_clearance(sink, net_class_no, ItemClassIr::Trace);
        }
        let line_count = polyline_path.coordinate_arr.len() / 4;
        let mut lines: Vec<Line> = Vec::with_capacity(line_count);
        for index in 0..line_count {
            let a = transform
                .dsn_to_board_tuple([
                    polyline_path.coordinate_arr[4 * index],
                    polyline_path.coordinate_arr[4 * index + 1],
                ])
                .round();
            let b = transform
                .dsn_to_board_tuple([
                    polyline_path.coordinate_arr[4 * index + 2],
                    polyline_path.coordinate_arr[4 * index + 3],
                ])
                .round();
            lines.push(Line::new(Point::Int(a), Point::Int(b)));
        }
        let trace_polyline = Polyline::new(lines);
        // A sub-2-corner polyline (e.g. a 1-line path) is silently
        // dropped by the sink — no item, no id, no warning.
        insert_trace(
            sink,
            &trace_polyline,
            layer_no,
            half_width,
            net_numbers,
            clearance_class,
            fixed,
        );
    } else {
        // Java `:566-573` "unexpected Path subclass" — unreachable (path
        // only ever holds the two Path variants).
        return true;
    }
    // `:575-576` `tryCorrectNet` NOT ported (module docs, D11).
    true
}

/// Java `Wiring.readViaScope` (`:601-714`).
fn read_via_scope(scanner: &mut Scanner, state: &mut ParseState, sink: &mut dyn BoardSink) -> bool {
    let fixed_cell = std::cell::Cell::new(FixedStateIr::Unfixed);
    // The padstack name (`:610-621`): any non-string token fails the read.
    let padstack_name = match scanner.next_token() {
        Token::Str(name) => name.to_string(),
        Token::Error(_) => return false,
        _ => {
            // log-only "padstack name expected".
            return false;
        }
    };
    scanner.set_scope_identifier(&padstack_name);
    // The location (`:623-640`): exactly two numbers; the SECOND token
    // seeds the inner loop's prev.
    let mut location = [0.0f64; 2];
    let mut second_location_token = Token::Eof;
    for slot in &mut location {
        let token = scanner.next_token();
        match token {
            Token::Double(value) => *slot = value,
            Token::Int(value) => *slot = f64::from(value),
            Token::Error(_) => return false,
            _ => {
                // log-only "number expected".
                return false;
            }
        }
        second_location_token = token;
    }
    let mut net_id: Option<NetId> = None;
    let mut clearance_class_name: Option<String> = None;
    let mut next_token: Option<Token> = Some(second_location_token);
    loop {
        let prev_token = next_token;
        next_token = match scanner.next_token() {
            Token::Error(_) => return false,
            Token::Eof => {
                // log-only "unexpected end of file".
                return false;
            }
            Token::Close => break,
            token => Some(token),
        };
        if prev_token == Some(Token::Open) {
            match next_token {
                Some(Token::Keyword(Keyword::Net)) => {
                    net_id = Some(read_net_id(scanner));
                }
                Some(Token::Keyword(Keyword::ClearanceClass)) => {
                    clearance_class_name = Some(read_string_scope(scanner));
                }
                Some(Token::Keyword(Keyword::Type)) => {
                    fixed_cell.set(calc_fixed(scanner));
                }
                _ => {
                    let _ = skip_scope(scanner);
                }
            }
        }
    }
    let fixed = fixed_cell.get();
    // `:660` `board.library.padstacks.get(...)`: NPEs on a null board AND
    // on the null padstacks table of a library-less DSN (module docs,
    // jar cf-k).
    if !sink.has_board() {
        return false;
    }
    // `:661-671`: the query strips every `.<digits>` run and matches
    // case-insensitively; a miss warns with the RAW name and fails the
    // whole read.
    let Some(padstack_no) = sink.resolve_padstack_query(&padstack_name) else {
        state.warnings.push(format!(
            "Wiring: via padstack '{}' not found at '{}'",
            padstack_name,
            scanner.scope_identifier()
        ));
        return false;
    };
    // `:673-681`: a named net that resolves to nothing warns and
    // CONTINUES — the via still inserts, with an empty net list.
    let found_nets = get_subnets(sink, net_id.as_ref());
    if found_nets.is_empty()
        && let Some(net_id) = &net_id
    {
        state.warnings.push(format!(
            "Wiring: via net '{}' not found at '{}'",
            net_id.name,
            scanner.scope_identifier()
        ));
    }
    // `:682-689` verbatim — the write index is NEVER incremented, so a
    // multi-subnet via stores [lastNet, 0, ...] (module docs).
    let mut net_class_no = 0i32;
    let mut net_numbers = vec![0i32; found_nets.len()];
    for net_no in &found_nets {
        net_numbers[0] = *net_no;
        net_class_no = match sink.nets().get((*net_no - 1) as usize) {
            Some(net) => net.net_class,
            None => return false,
        };
    }
    let mut clearance_class = -1i32;
    if let Some(name) = &clearance_class_name {
        clearance_class = sink.clearance_class_no(name).unwrap_or(-1);
    }
    if clearance_class < 0 {
        clearance_class = net_class_item_clearance(sink, net_class_no, ItemClassIr::Via);
    }
    // `:698` — the transform exists whenever the board does.
    let Some(transform) = state.coordinate_transform else {
        return false;
    };
    let board_location = transform.dsn_to_board_tuple(location).round();
    // `:699-705`: duplicate via probe; the duplicate is skipped (no item,
    // no id) and the read continues.
    let Some(padstack) = sink.padstack(padstack_no) else {
        return false;
    };
    let from_layer = padstack_from_layer(&padstack.shapes) as i32;
    let to_layer = padstack_to_layer(&padstack.shapes);
    if sink.via_exists(board_location, from_layer, to_layer, &net_numbers) {
        state.warnings.push(format!(
            "Wiring: duplicate via skipped at ({}, {})",
            board_location.x, board_location.y
        ));
    } else {
        // `:706-708`: `Padstack.attachAllowed == is_drillable`
        // (`Padstack.java:57`), so the drillable flag is exact.
        let attach_smd_allowed = state.via_at_smd_allowed && padstack.drillable;
        sink.insert_via(ViaIr {
            padstack_no,
            location: board_location,
            nets: net_numbers,
            clearance_class,
            fixed,
            attach_smd_allowed,
        });
    }
    true
}

/// Java `Wiring.calcFixed` (`:255-275`): the first token picks the state
/// (shove_fixed → SHOVE, fix → SYSTEM, normal → UNFIXED, ANYTHING else —
/// including `Close(type)` — → USER), the second must be the closing
/// bracket (log-only warn + UNFIXED otherwise). The IOException arm maps
/// to UNFIXED on either read.
fn calc_fixed(scanner: &mut Scanner) -> FixedStateIr {
    let result = match scanner.next_token() {
        Token::Keyword(Keyword::ShoveFixed) => FixedStateIr::ShoveFixed,
        Token::Keyword(Keyword::Fix) => FixedStateIr::SystemFixed,
        Token::Keyword(Keyword::Normal) => FixedStateIr::Unfixed,
        Token::Error(_) => return FixedStateIr::Unfixed,
        _ => FixedStateIr::UserFixed,
    };
    match scanner.next_token() {
        Token::Close => result,
        Token::Error(_) => FixedStateIr::Unfixed,
        _ => {
            // log-only "Wiring.is_fixed: ) expected at '{id}'".
            FixedStateIr::Unfixed
        }
    }
}

/// Java `Wiring.readNetId` (`:281-305`): the net name, an optional
/// positional subnet integer (default 0), then the closing bracket. A
/// non-close token only logs (log-only warn); the Id is returned either
/// way. The Task-5 scanner fold routes stream failure to an EMPTY name —
/// the port's IOException-substitute for Java's null (the catch maps the
/// exception to null; the port carries `""` through the same path) — so
/// the return is the bare [`NetId`], never absent. Theoretical edge,
/// unreachable today: Java's null id suppresses the downstream "net not
/// found" warning, while the Rust `""`-id could emit one.
fn read_net_id(scanner: &mut Scanner) -> NetId {
    let net_name = scanner.next_string();
    scanner.set_scope_identifier(&net_name);
    let mut subnet_number = 0i32;
    let mut next_token = scanner.next_token();
    if let Token::Int(value) = next_token {
        subnet_number = value;
        next_token = scanner.next_token();
    }
    if next_token != Token::Close {
        // log-only "Wiring.read_net_id: closing bracket expected".
    }
    NetId {
        name: net_name,
        subnet_number,
    }
}

/// Java `Wiring.getSubnets` (`:223-236`): no id → empty; a positive
/// subnet number → the FIRST case-insensitive name+subnet match (or
/// empty); otherwise EVERY case-insensitive name match in table order.
fn get_subnets(sink: &dyn BoardSink, net_id: Option<&NetId>) -> Vec<i32> {
    let Some(net_id) = net_id else {
        return Vec::new();
    };
    if net_id.subnet_number > 0 {
        match sink.net_no_subnet(&net_id.name, net_id.subnet_number) {
            Some(net_no) => vec![net_no],
            None => Vec::new(),
        }
    } else {
        sink.net_nos(&net_id.name)
    }
}

/// Java `netClass.defaultItemClearanceClasses.get(ItemClass)` for the
/// wire/via's (possibly default) net class.
fn net_class_item_clearance(
    sink: &dyn BoardSink,
    net_class_no: i32,
    item_class: ItemClassIr,
) -> i32 {
    sink.net_classes()
        .get(net_class_no.max(0) as usize)
        .map(|net_class| net_class.default_item_clearance_classes[item_class as usize])
        .unwrap_or_else(|| item_class.default_clearance_class())
}

/// `insertTraceWithoutCleaning` — the sink owns the drop guards:
/// sub-2-corner polylines drop without an id; closed low-fixed loops
/// drop but still burn one (the guard fires after the `Item` ctor,
/// `BasicBoard.java:183-201` / `Item.java:86-89`).
fn insert_trace(
    sink: &mut dyn BoardSink,
    polyline: &Polyline,
    layer_no: i32,
    half_width: i32,
    nets: Vec<i32>,
    clearance_class: i32,
    fixed: FixedStateIr,
) {
    let corners: Vec<IntPoint> = polyline.corners().iter().map(corner_to_int).collect();
    sink.insert_trace(TraceIr {
        layer_no,
        half_width,
        corners,
        // T13: the reader-constructed polyline is carried VERBATIM —
        // the live board must store exactly Java's parse-time polyline,
        // and it is NOT re-derivable from the rounded corner list (the
        // `PolylinePath` branch keeps duplicate corners that
        // `Polyline::from_points`' Polygon dedup would drop; dsn-0061).
        polyline: polyline.clone(),
        nets,
        clearance_class,
        fixed,
    });
}

/// Java keeps exact Rational polyline corners; the port rounds at parse
/// time (module docs).
fn corner_to_int(point: &Point) -> IntPoint {
    point.to_float().round()
}

/// Java `IntBox.contains(FloatPoint)`: closed containment, double
/// comparison against the int bounds.
fn int_box_contains_float(bounding_box: &IntBox, x: f64, y: f64) -> bool {
    f64::from(bounding_box.ll.x) <= x
        && x <= f64::from(bounding_box.ur.x)
        && f64::from(bounding_box.ll.y) <= y
        && y <= f64::from(bounding_box.ur.y)
}

/// `Shape.layer.no` for the three border variants (the path variants are
/// handled in the layer/half-width match above).
fn border_shape_layer_no(shape: &Shape) -> i32 {
    match shape {
        Shape::Rectangle(rectangle) => rectangle.layer.no,
        Shape::Polygon(polygon) => polygon.layer.no,
        Shape::Circle(circle) => circle.layer.no,
        _ => 0,
    }
}

/// `Shape.layer.name` for the three border variants.
fn border_shape_layer_name(shape: &Shape) -> String {
    match shape {
        Shape::Rectangle(rectangle) => rectangle.layer.name.clone(),
        Shape::Polygon(polygon) => polygon.layer.name.clone(),
        Shape::Circle(circle) => circle.layer.name.clone(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::read_board;
    use crate::ses_board::{ItemIr, SesBoard};

    // ==== Fixtures (exact bytes of /tmp/epic-t9-*.dsn, the on-disk
    // evidence of record; every pin below was captured fresh from the
    // frozen jar against these files, /tmp/epic-t9-{fresh,fresh2,cfx,
    // cfi,cf,cf2,cf3,cf4,cfk,warnings}.out). ====

    const WARNINGS_DSN: &str = r#"(pcb t9-warnings.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary
      (path pcb 0  0 0  10000 0  10000 10000  0 10000  0 0)
    )
    (keepout
      (polygon F.Cu 0  1000 1000  3000 1000  3000 3000  1000 3000)
      (rule (clearance 50))
    )
    (rule
      (width 250)
      (clearance 200)
    )
  )
  (placement
    (component PAD
      (place U1 2000 5000 front 0)
      (place U2 8000 5000 front 0)
      (place U3 2000 7000 front 0)
      (place U4 8000 7000 front 0)
    )
  )
  (library
    (image PAD
      (pin CirclePad_F_800_um 1 0 0)
    )
    (padstack ViaPad_F
      (shape (circle F.Cu 600))
      (attach off)
    )
    (padstack CirclePad_F_800_um
      (shape (circle F.Cu 800))
      (attach off)
    )
  )
  (network
    (net PERFECT (pins U1-1 U2-1))
    (net GND 1 (pins U3-1))
    (net GND 2 (pins U4-1))
    (class kicad_default PERFECT
      (via ViaPad_F)
    )
  )
  (wiring
    (wire (path F.Cu 125  2000 1000  3000 1000) (net PERFECT))
    (wire (net PERFECT))
    (wire (path pcb 125  1000 2000  2000 2000) (net PERFECT))
    (wire (path F.Cu 125  1000 3000  99999 99999) (net PERFECT))
    (wire (path F.Cu 125  3000 3000  3000 3000  3000 3000) (net PERFECT))
    (via ViaPad_F 5000 2000 (net NOSUCH))
    (via ViaPad_F 5000 5000 (net PERFECT))
    (via ViaPad_F 5000 5000 (net PERFECT))
    (via ViaPad_F 5000 7000 (net GND))
    (via ViaPad_F.1 5000 8000 (net PERFECT))
    (wire (rectangle F.Cu 4000 4000 6000 6000) (net PERFECT))
  )
)
"#;
    const CALCFIXED_DSN: &str = r#"(pcb t9-calcfixed.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary
      (path pcb 0  0 0  10000 0  10000 10000  0 10000  0 0)
    )
  )
  (network
    (net PERFECT)
  )
  (wiring
    (wire (path F.Cu 125  1000 900  2000 900) (net PERFECT) (type shove_fixed))
    (wire (path F.Cu 125  1000 1000  2000 1000) (net PERFECT) (type normal))
    (wire (path F.Cu 125  1000 2000  2000 2000) (net PERFECT) (type route))
    (wire (path F.Cu 125  1000 3000  2000 3000) (net PERFECT) (type protect))
    (wire (path F.Cu 125  1000 4000  2000 4000) (net PERFECT) (type fix))
    (wire (path F.Cu 125  1000 5000  2000 5000) (net PERFECT) (type fix garbage))
    (wire (path F.Cu 125  1000 7000  2000 7000) (net PERFECT) (type ))
    (wire (path F.Cu 125  1000 8000  2000 8000) (net PERFECT))
  )
)
"#;
    const CF_A_DSN: &str = r#"(pcb t9-cf-a.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary
      (path pcb 0  0 0  10000 0  10000 10000  0 10000  0 0)
    )
  )
  (network
    (net PERFECT)
  )
  (wiring
    (wire (path F.Cu 125  1000 2000  2000 2000) (net PERFECT) (type route))
    (wire (path F.Cu 125  1000 7000  2000 7000) (net PERFECT) (type ))
  )
)
"#;
    const CF_B_DSN: &str = r#"(pcb t9-cf-b.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary
      (path pcb 0  0 0  10000 0  10000 10000  0 10000  0 0)
    )
  )
  (network
    (net PERFECT)
  )
  (wiring
    (wire (path F.Cu 125  1000 2000  2000 2000) (net PERFECT) (type route))
    (wire (path F.Cu 125  1000 7000  2000 7000) (net PERFECT) (type ))
    (wire (path F.Cu 125  1000 8000  2000 8000) (net PERFECT))
  )
)
"#;
    const CF_C_DSN: &str = r#"(pcb t9-cf-c.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary
      (path pcb 0  0 0  10000 0  10000 10000  0 10000  0 0)
    )
  )
  (network
    (net PERFECT)
  )
  (wiring
    (wire (path F.Cu 125  1000 7000  2000 7000) (net PERFECT) (type ))
  )
)
"#;
    const CF_D_DSN: &str = r#"(pcb t9-cf-d.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary
      (path pcb 0  0 0  10000 0  10000 10000  0 10000  0 0)
    )
  )
  (network
    (net PERFECT)
  )
  (wiring
    (wire (path F.Cu 125  1000 900  2000 900) (net PERFECT) (type shove_fixed))
    (wire (path F.Cu 125  1000 1000  2000 1000) (net PERFECT) (type normal))
    (wire (path F.Cu 125  1000 2000  2000 2000) (net PERFECT) (type route))
    (wire (path F.Cu 125  1000 3000  2000 3000) (net PERFECT) (type protect))
    (wire (path F.Cu 125  1000 4000  2000 4000) (net PERFECT) (type fix))
    (wire (path F.Cu 125  1000 7000  2000 7000) (net PERFECT) (type ))
    (wire (path F.Cu 125  1000 8000  2000 8000) (net PERFECT))
  )
)
"#;
    const CF_E_DSN: &str = r#"(pcb t9-cf-e.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary
      (path pcb 0  0 0  10000 0  10000 10000  0 10000  0 0)
    )
  )
  (network
    (net PERFECT)
  )
  (wiring
    (wire (path F.Cu 125  1000 900  2000 900) (net PERFECT) (type shove_fixed))
    (wire (path F.Cu 125  1000 1000  2000 1000) (net PERFECT) (type normal))
    (wire (path F.Cu 125  1000 2000  2000 2000) (net PERFECT) (type route))
    (wire (path F.Cu 125  1000 3000  2000 3000) (net PERFECT) (type protect))
    (wire (path F.Cu 125  1000 4000  2000 4000) (net PERFECT) (type fix))
    (wire (path F.Cu 125  1000 5000  2000 5000) (net PERFECT) (type fix garbage))
    (wire (path F.Cu 125  1000 7000  2000 7000) (net PERFECT) (type ))
  )
)
"#;
    const CF_F_DSN: &str = r#"(pcb t9-cf-f.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary
      (path pcb 0  0 0  10000 0  10000 10000  0 10000  0 0)
    )
  )
  (network
    (net PERFECT)
  )
  (wiring
    (wire (path F.Cu 125  1000 5000  2000 5000) (net PERFECT) (type fix garbage))
    (wire (path F.Cu 125  1000 7000  2000 7000) (net PERFECT) (type ))
    (wire (path F.Cu 125  1000 8000  2000 8000) (net PERFECT))
  )
)
"#;
    const CF_G_DSN: &str = r#"(pcb t9-cf-g.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary
      (path pcb 0  0 0  10000 0  10000 10000  0 10000  0 0)
    )
  )
  (network
    (net PERFECT)
  )
  (wiring
    (wire (path F.Cu 125  1000 5000  2000 5000) (net PERFECT) (type fix garbage))
    (wire (path F.Cu 125  1000 8000  2000 8000) (net PERFECT))
  )
)
"#;
    const CF_H_DSN: &str = r#"(pcb t9-cf-h.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary
      (path pcb 0  0 0  10000 0  10000 10000  0 10000  0 0)
    )
  )
  (network
    (net PERFECT)
  )
  (wiring
    (wire (path F.Cu 125  1000 5000  2000 5000) (net PERFECT) (type route garbage))
    (wire (path F.Cu 125  1000 8000  2000 8000) (net PERFECT))
  )
)
"#;
    const CF_I_DSN: &str = r#"(pcb t9-cf-i.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary
      (path pcb 0  0 0  10000 0  10000 10000  0 10000  0 0)
    )
  )
  (network
    (net PERFECT)
  )
  (wiring
    (wire (path F.Cu 125  1000 5000  2000 5000) (net PERFECT) (type garbage))
    (wire (path F.Cu 125  1000 8000  2000 8000) (net PERFECT))
  )
)
"#;
    const CF_J_DSN: &str = r#"(pcb t9-cf-j.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary
      (path pcb 0  0 0  10000 0  10000 10000  0 10000  0 0)
    )
  )
  (network
    (net PERFECT)
    (net GND)
  )
  (wiring
    (wire (path F.Cu 125  1000 1000  2000 1000) (net PERFECT) (type )(net GND))
    (wire (path F.Cu 125  1000 8000  2000 8000) (net PERFECT))
  )
)
"#;
    const CF_K_DSN: &str = r#"(pcb t9-cf-k.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary
      (path pcb 0  0 0  10000 0  10000 10000  0 10000  0 0)
    )
  )
  (network
    (net PERFECT)
  )
  (wiring
    (via NoSuchPad 5000 5000 (net PERFECT))
  )
)
"#;
    const PLPATH_DSN: &str = r#"(pcb t9-plpath.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary
      (path pcb 0  0 0  10000 0  10000 10000  0 10000  0 0)
    )
  )
  (network
    (net PERFECT)
  )
  (wiring
    (wire (polyline_path F.Cu 125  1000 1000  2000 1000  2000 1000  2000 2000  2000 2000  3000 2000) (net PERFECT))
    (wire (polyline_path F.Cu 125  3000 1000  4000 1000) (net PERFECT) (type route))
  )
)
"#;
    const NPEFOLD_DSN: &str = r#"(pcb t9-npefold.dsn
  (wiring
    (wire (path F.Cu 125  2000 1000  3000 1000) (net GND))
  )
  (structure
    (layer F.Cu (type signal))
    (boundary (path pcb 0  0 0  10000 0  10000 10000  0 10000  0 0))
  )
)
"#;
    const NPEFOLDVIA_DSN: &str = r#"(pcb t9-npefoldvia.dsn
  (wiring
    (via ViaPad_F 5000 5000 (net GND))
  )
  (structure
    (layer F.Cu (type signal))
    (boundary (path pcb 0  0 0  10000 0  10000 10000  0 10000  0 0))
  )
)
"#;

    fn run(fixture: &str) -> (crate::reader::DsnReadResult, SesBoard) {
        let mut board = SesBoard::new();
        let result = read_board(fixture.as_bytes(), &mut board);
        (result, board)
    }

    /// The board traces in insertion order, with ids.
    fn traces(board: &SesBoard) -> Vec<(i32, &TraceIr)> {
        board
            .items
            .iter()
            .filter_map(|item| match item {
                ItemIr::Trace { id, trace } => Some((*id, trace)),
                _ => None,
            })
            .collect()
    }

    /// The board vias in insertion order, with ids.
    fn vias(board: &SesBoard) -> Vec<(i32, &ViaIr)> {
        board
            .items
            .iter()
            .filter_map(|item| match item {
                ItemIr::Via { id, via } => Some((*id, via)),
                _ => None,
            })
            .collect()
    }

    fn item_count(board: &SesBoard) -> usize {
        board.items.len()
    }

    fn max_id(board: &SesBoard) -> i32 {
        board.items.iter().map(|item| item.id()).max().unwrap_or(0)
    }

    // ==== Pins ====

    /// The kitchen-sink warning battery: every D12 warning site fires in
    /// file order, and every insert path coexists. Jar
    /// /tmp/epic-t9-{warnings,fresh2}.out: WARNINGS 6, GEN_MAX 12,
    /// via id10 nets=[3, 0] (the no-increment bug), via id11 resolved
    /// through the `.1`-stripped padstack query.
    #[test]
    fn warnings_battery_pins_all_warning_sites_and_insert_paths() {
        let (result, board) = run(WARNINGS_DSN);
        let warnings = match &result {
            crate::reader::DsnReadResult::Success { warnings } => warnings,
            other => panic!("expected Success, got {other:?}"),
        };
        assert_eq!(
            warnings,
            &vec![
                "Wiring: wire has no shape at 'PERFECT'".to_string(),
                "Wiring: wire ignored — unknown layer 'pcb' at 'PERFECT'".to_string(),
                "Wiring: wire corner (99999,99999) is outside board bounds at 'PERFECT'"
                    .to_string(),
                "Wiring: degenerate wire trace skipped (all 1 corners are identical — zero-length trace) on layer 'F.Cu'. This is likely a DSN export issue in your EDA tool.".to_string(),
                "Wiring: via net 'NOSUCH' not found at 'NOSUCH'".to_string(),
                "Wiring: duplicate via skipped at (50000, 50000)".to_string(),
            ]
        );
        assert_eq!(item_count(&board), 12);
        assert_eq!(max_id(&board), 12);
        // ids 1..12: outline, keepout, 4 pins, trace, 4 vias, area.
        assert_eq!(traces(&board).len(), 1);
        let (id, trace) = traces(&board)[0];
        assert_eq!(id, 7);
        assert_eq!(trace.layer_no, 0);
        assert_eq!(trace.half_width, 625);
        assert_eq!(trace.fixed, FixedStateIr::Unfixed);
        assert_eq!(
            trace.corners,
            vec![IntPoint::new(20000, 10000), IntPoint::new(30000, 10000)]
        );
        assert_eq!(trace.nets, vec![1]);
        assert_eq!(trace.clearance_class, 1);
        let board_vias = vias(&board);
        assert_eq!(board_vias.len(), 4);
        let via_payloads: Vec<(i32, [i32; 2], Vec<i32>)> = board_vias
            .iter()
            .map(|(id, via)| (*id, [via.location.x, via.location.y], via.nets.clone()))
            .collect();
        assert_eq!(
            via_payloads,
            vec![
                (8, [50000, 20000], Vec::<i32>::new()),
                (9, [50000, 50000], vec![1]),
                // The no-increment bug: two GND subnets store [3, 0].
                (10, [50000, 70000], vec![3, 0]),
                (11, [50000, 80000], vec![1]),
            ]
        );
        // All vias resolve to padstack 1 — ViaPad_F is the FIRST
        // `(padstack ...)` scope in the library (the image pin's
        // CirclePad_F_800_um is the second; image pins do not inject
        // registry entries at image-scope time).
        for (_, via) in board_vias {
            assert_eq!(via.padstack_no, 1);
            assert_eq!(via.fixed, FixedStateIr::Unfixed);
        }
        // The rectangle wire became a conduction area on layer 0.
        let areas: Vec<(i32, &ConductionAreaIr)> = board
            .items
            .iter()
            .filter_map(|item| match item {
                ItemIr::ConductionArea { id, area } => Some((*id, area)),
                _ => None,
            })
            .collect();
        assert_eq!(areas.len(), 1);
        assert_eq!(areas[0].0, 12);
        assert_eq!(areas[0].1.layer_no, 0);
        assert_eq!(areas[0].1.fixed, FixedStateIr::Unfixed);
        assert_eq!(areas[0].1.nets, vec![1]);
        // 3 nets: PERFECT/1, GND/1, GND/2.
        assert_eq!(board.nets.len(), 3);
        assert_eq!(board.nets[2].subnet_number, 2);
    }

    /// Jar /tmp/epic-t9-fresh.out CASE calcfixed: 8 wires in, GEN_MAX 7.
    /// The `(type fix garbage)` wire (y5000) inserts UNFIXED and leaks its
    /// scope close, so the wiring scope ends there: wires 7-8 never
    /// produce traces. Exactly ONE log-only is_fixed warning (not a parity
    /// warning).
    #[test]
    fn calcfixed_fix_garbage_leak_ends_the_wiring_scope() {
        let (result, board) = run(CALCFIXED_DSN);
        let warnings = match &result {
            crate::reader::DsnReadResult::Success { warnings } => warnings,
            other => panic!("expected Success, got {other:?}"),
        };
        assert!(warnings.is_empty(), "got {warnings:?}");
        let got: Vec<(i32, FixedStateIr, [i32; 2], [i32; 2])> = traces(&board)
            .iter()
            .map(|(id, t)| {
                (
                    *id,
                    t.fixed,
                    [t.corners[0].x, t.corners[0].y],
                    [t.corners[1].x, t.corners[1].y],
                )
            })
            .collect();
        assert_eq!(
            got,
            vec![
                (2, FixedStateIr::ShoveFixed, [10000, 9000], [20000, 9000]),
                (3, FixedStateIr::Unfixed, [10000, 10000], [20000, 10000]),
                (4, FixedStateIr::UserFixed, [10000, 20000], [20000, 20000]),
                (5, FixedStateIr::UserFixed, [10000, 30000], [20000, 30000]),
                (6, FixedStateIr::SystemFixed, [10000, 40000], [20000, 40000]),
                (7, FixedStateIr::Unfixed, [10000, 50000], [20000, 50000]),
            ]
        );
        assert_eq!(max_id(&board), 7);
    }

    /// The full desync table. Jar captures:
    /// cf-a/cf-b /tmp/epic-t9-{cfx,fresh}.out, cf-c..cf-i
    /// /tmp/epic-t9-{cf,cf2,cf3,cfi}.out, cf-j /tmp/epic-t9-cfx.out.
    #[test]
    fn calcfixed_desync_table_reproduces_every_jar_case() {
        // cf-a: `(type route)` USER + `(type )` USER, GEN 3.
        let (result, board) = run(CF_A_DSN);
        assert!(matches!(
            result,
            crate::reader::DsnReadResult::Success { .. }
        ));
        let fixed: Vec<FixedStateIr> = traces(&board).iter().map(|(_, t)| t.fixed).collect();
        assert_eq!(
            fixed,
            vec![FixedStateIr::UserFixed, FixedStateIr::UserFixed]
        );
        assert_eq!(max_id(&board), 3);

        // cf-b: cf-a plus a trailing plain wire — SWALLOWED by the wire
        // loop's else-skipScope arm (the `(type )` consumed both closes),
        // so GEN stays 3.
        let (result, board) = run(CF_B_DSN);
        assert!(matches!(
            result,
            crate::reader::DsnReadResult::Success { .. }
        ));
        let fixed: Vec<FixedStateIr> = traces(&board).iter().map(|(_, t)| t.fixed).collect();
        assert_eq!(
            fixed,
            vec![FixedStateIr::UserFixed, FixedStateIr::UserFixed]
        );
        assert_eq!(max_id(&board), 3);

        // cf-c: `(type )` alone → the wire itself inserts USER, GEN 2.
        let (result, board) = run(CF_C_DSN);
        assert!(matches!(
            result,
            crate::reader::DsnReadResult::Success { .. }
        ));
        let fixed: Vec<FixedStateIr> = traces(&board).iter().map(|(_, t)| t.fixed).collect();
        assert_eq!(fixed, vec![FixedStateIr::UserFixed]);
        assert_eq!(max_id(&board), 2);

        // cf-d: five typed wires + `(type )` (GEN 7; trailing wire
        // swallowed).
        let (result, board) = run(CF_D_DSN);
        assert!(matches!(
            result,
            crate::reader::DsnReadResult::Success { .. }
        ));
        let fixed: Vec<FixedStateIr> = traces(&board).iter().map(|(_, t)| t.fixed).collect();
        assert_eq!(
            fixed,
            vec![
                FixedStateIr::ShoveFixed,
                FixedStateIr::Unfixed,
                FixedStateIr::UserFixed,
                FixedStateIr::UserFixed,
                FixedStateIr::SystemFixed,
                FixedStateIr::UserFixed,
            ]
        );
        assert_eq!(max_id(&board), 7);

        // cf-e: five typed wires + `(type fix garbage)` + `(type )` — the
        // garbage wire leaks its close (GEN 7; the `(type )` wire lost).
        let (result, board) = run(CF_E_DSN);
        assert!(matches!(
            result,
            crate::reader::DsnReadResult::Success { .. }
        ));
        let fixed: Vec<FixedStateIr> = traces(&board).iter().map(|(_, t)| t.fixed).collect();
        assert_eq!(
            fixed,
            vec![
                FixedStateIr::ShoveFixed,
                FixedStateIr::Unfixed,
                FixedStateIr::UserFixed,
                FixedStateIr::UserFixed,
                FixedStateIr::SystemFixed,
                FixedStateIr::Unfixed,
            ]
        );
        assert_eq!(max_id(&board), 7);

        // cf-f/g/h: a leading garbage-type wire leaks; the other two wires
        // are lost; the garbage wire itself inserts UNFIXED (jar cf-f/g/h
        // + /tmp/epic-t9-cfi.out: GEN 2, single UNFIXED trace).
        for fixture in [CF_F_DSN, CF_G_DSN, CF_H_DSN] {
            let (result, board) = run(fixture);
            assert!(matches!(
                result,
                crate::reader::DsnReadResult::Success { .. }
            ));
            let fixed: Vec<FixedStateIr> = traces(&board).iter().map(|(_, t)| t.fixed).collect();
            assert_eq!(fixed, vec![FixedStateIr::Unfixed]);
            assert_eq!(max_id(&board), 2);
        }

        // cf-i: `(type garbage)` — the bare string maps to USER (the
        // calcFixed default arm), the wire's close is intact, so the
        // trailing plain wire inserts UNFIXED (GEN 3).
        let (result, board) = run(CF_I_DSN);
        assert!(matches!(
            result,
            crate::reader::DsnReadResult::Success { .. }
        ));
        let fixed: Vec<FixedStateIr> = traces(&board).iter().map(|(_, t)| t.fixed).collect();
        assert_eq!(fixed, vec![FixedStateIr::UserFixed, FixedStateIr::Unfixed]);
        assert_eq!(max_id(&board), 3);

        // cf-j: `(type )(net GND)` — USER then warn+UNFIXED; the loop
        // breaks at Close(net) and the wire keeps its ORIGINAL net
        // (PERFECT = [1], not GND); the trailing wire is swallowed.
        let (result, board) = run(CF_J_DSN);
        assert!(matches!(
            result,
            crate::reader::DsnReadResult::Success { .. }
        ));
        let got: Vec<(FixedStateIr, Vec<i32>)> = traces(&board)
            .iter()
            .map(|(_, t)| (t.fixed, t.nets.clone()))
            .collect();
        assert_eq!(got, vec![(FixedStateIr::Unfixed, vec![1])]);
        assert_eq!(max_id(&board), 2);
    }

    /// Jar /tmp/epic-t9-fresh.out CASE plpath: the 3-line polyline_path
    /// collapses to 2 stored corners (Java Polyline ctor semantics; the
    /// Rust Polyline is the M1a java-verbatim port so the jar values are
    /// pinned, not re-derived). The second wire's 1-line path (2 corners)
    /// drops silently in the sink (corner count < 2, no id burned). The
    /// trailing extra `)` in the fixture is absorbed. Plain `(path ...)` /
    /// `(polygon_path ...)` share the POLYGON_PATH arm.
    #[test]
    fn polyline_path_corner_collapse_and_one_line_drop() {
        let (result, board) = run(PLPATH_DSN);
        assert!(matches!(
            result,
            crate::reader::DsnReadResult::Success { .. }
        ));
        let got: Vec<(i32, i32, i32, Vec<[i32; 2]>)> = traces(&board)
            .iter()
            .map(|(id, t)| {
                (
                    *id,
                    t.layer_no,
                    t.half_width,
                    t.corners.iter().map(|c| [c.x, c.y]).collect(),
                )
            })
            .collect();
        assert_eq!(
            got,
            vec![(2, 0, 625, vec![[20000, 10000], [20000, 20000]],)]
        );
        assert_eq!(max_id(&board), 2);
    }

    /// T13 IR invariant: the carried polyline is the source of truth and
    /// the rounded `corners` view is derived from it at the single
    /// construction site (`insert_trace`). The `polyline_path` branch is
    /// the case the digest corpus proved load-bearing (dsn-0061/0063: the
    /// reader's `Polyline(Line[])` keeps corners `from_points` would
    /// dedup, and the live board must store the reader's polyline
    /// verbatim).
    #[test]
    fn trace_ir_corners_are_the_rounded_polyline_view() {
        let (result, board) = run(PLPATH_DSN);
        assert!(matches!(
            result,
            crate::reader::DsnReadResult::Success { .. }
        ));
        let count = traces(&board).len();
        assert_eq!(count, 1, "PLPATH_DSN parses exactly one trace");
        for (_, trace) in traces(&board) {
            let derived: Vec<IntPoint> =
                trace.polyline.corners().iter().map(corner_to_int).collect();
            assert_eq!(trace.corners, derived, "trace {}", trace.layer_no);
        }
    }

    /// Jar /tmp/epic-t9-fresh.out CASE npefold: wiring before structure —
    /// the wire drops at the no-shape check (no layer structure yet), the
    /// wiring tail's null-board normalization NPE lands in the catch arm
    /// ("normalization of traces failed"), and the file still folds to
    /// Success with the structure parsed afterwards (GEN 1, outline only).
    #[test]
    fn wiring_before_structure_folds_to_success() {
        let (result, board) = run(NPEFOLD_DSN);
        let warnings = match &result {
            crate::reader::DsnReadResult::Success { warnings } => warnings,
            other => panic!("expected Success, got {other:?}"),
        };
        assert_eq!(
            warnings,
            &vec![
                "Wiring: wire has no shape at 'GND'".to_string(),
                "Wiring: normalization of traces failed".to_string(),
            ]
        );
        assert_eq!(item_count(&board), 1);
        assert_eq!(max_id(&board), 1);
        assert!(matches!(board.items[0], ItemIr::BoardOutline { id: 1, .. }));
    }

    /// Jar /tmp/epic-t9-fresh.out CASE npefoldvia: a via before structure
    /// NPEs at Wiring.java:660 (null board) and the exception escapes
    /// readBoard UNCAUGHT — Java produces NO result at all (the trailing
    /// "RES Success GEN_MAX 0" lines in the capture are JShell stale-
    /// variable artifacts). The port classifies mechanically: the wiring
    /// scope returns false before the structure scope runs, and
    /// `boardOutlineOk` starts TRUE (`ReadScopeParameter.java:75` — the
    /// Rust default mirrors it), so the failure is a ParseError, not an
    /// OutlineMissing. DOCUMENTED DIVERGENCE (crash vs classified
    /// failure).
    #[test]
    fn via_before_structure_classifies_parse_error() {
        let (result, board) = run(NPEFOLDVIA_DSN);
        match result {
            crate::reader::DsnReadResult::ParseError { location, detail } => {
                assert_eq!(location, "(pcb");
                assert_eq!(detail, "DSN structure parsing failed");
            }
            other => panic!("expected ParseError, got {other:?}"),
        }
        assert_eq!(item_count(&board), 0);
    }

    /// Jar /tmp/epic-t9-cfk.out: a via naming an unknown padstack in a DSN
    /// with NO `(library ...)` scope — Java NPEs because
    /// `board.library.padstacks` is null (BasicBoard's no-arg BoardLibrary
    /// leaves it null until the Library scope assigns it,
    /// Library.java:264). The Rust registry is a non-null Vec, so the
    /// missing-padstack path warns (dropped with the ParseError) and fails
    /// the read → ParseError (structure parsed first, outline ok).
    /// DOCUMENTED DIVERGENCE (crash vs classified failure).
    #[test]
    fn via_with_unknown_padstack_and_no_library_scope_is_parse_error() {
        let (result, board) = run(CF_K_DSN);
        match result {
            crate::reader::DsnReadResult::ParseError { location, detail } => {
                assert_eq!(location, "(pcb");
                assert_eq!(detail, "DSN structure parsing failed");
            }
            other => panic!("expected ParseError, got {other:?}"),
        }
        assert_eq!(item_count(&board), 1);
    }
}
