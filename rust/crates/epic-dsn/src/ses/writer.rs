//! Port of `io/specctra/SesWriter.java` (Task 13, `:37-551`): serializes
//! a parse-time [`SesBoard`] to Specctra session text, byte-for-byte as
//! the Java oracle emits it (the harness's `SesEmitOracle` captures
//! goldens; `ses-compare` is the gate).
//!
//! ## Deliberate omissions (plan `:285`, decisions T40/D11)
//!
//! - **T40 endpoint snapping is NOT ported.** Java `snappedEndpoint`
//!   (`:426-453`) re-centers wire endpoints on contacted drill items —
//!   a ROUTER-state query (`wire.getStartContacts()`) a parse-time board
//!   has no equivalent of (no contact sets, D9). Emitted endpoints are
//!   exactly `java_round(session_transform(parsed_corner))`; the T40
//!   round-trip assertion is the harness compare against the oracle
//!   goldens (the oracle emits the same parse-time board, so its
//!   `snappedEndpoint` also always returns `null` there — `contacts` is
//!   empty before routing). Snapping is scheduled for M2+ when trace
//!   contacts exist.
//! - **`writeWasIs` emits an empty scope unconditionally.** Pin swaps
//!   (`Pin.getChangedTo() != this`) are a router-time mutation; a
//!   parse-time board has none. `(was_is` + `)` with nothing between —
//!   the golden compare pins this.
//! - **`(constant ...)`/`(write_resolution ...)` parser sub-scopes are
//!   not emitted**: `MetadataIr` carries neither (no corpus fixture
//!   sets them; the reader skips those sub-scopes). If a fixture ever
//!   does, the B3 golden capture diverges loudly.
//!
//! ## Item emission order (T39)
//!
//! `writeNet` walks `board.getConnectableItems(netNumber)`
//! (`BoardConnectivityQueries`) — the item list in DESCENDING id order
//! (the `UndoableObjects` ConcurrentSkipListMap iterates by
//! `Item.compareTo = other.id - id`), filtered to `Connectable` items
//! containing the net. The port walks [`SesBoard::items`] (ascending
//! id, parse order) in REVERSE, dispatching only on the three
//! emittable kinds — observationally identical because pins are the
//! only other Connectable-and-net-carrying kind and every branch they
//! reach writes nothing (`isWire`/`isVia`/`isConductionArea` all
//! false, so they neither open the net scope nor emit).
//!
//! ## Coordinate scale (T43)
//!
//! The session transform is `CoordinateTransform(dsnToBoard(1) /
//! resolution, 0, 0)` (`SesWriter.java:80-82`) — the BOARD transform's
//! scale divided by the DSN resolution, with ZERO base (placement
//! offsets do NOT shift session coordinates). Composed with the board
//! transform, an emitted coordinate is `source_dsn × resolution`: a
//! `um 10` board (scale 10) has session scale `10/10 = 1`, so the
//! emitted value is the BOARD integer (= dsn × 10). Real-world pin:
//! `fixtures/Issue313-FastTest.ses` — design `(resolution mil 1000)`,
//! the session's `(via via0 45016 …)` coordinates are board ints
//! (= dsn × 1000).
//!
//! ## Java-write quirks kept verbatim
//!
//! - `routes `, `library_out `, `network_out `, `session `,
//!   `component `, `net `, `via `, `padstack ` all carry a TRAILING
//!   SPACE that `newLine()`/identifier writes then follow — the goldens
//!   contain `(routes \n` with the space before the newline. `wire`,
//!   `placement`, `was_is`, `parser` have none.
//! - Path widths inside conduction-area shapes print as JAVA DOUBLES
//!   (`String.valueOf(double)`: `125.0`), while the wire `path` width
//!   is a Java `int` (`125`) — [`java_double_to_string`] for the
//!   former, integer display for the latter.
//! - `(place ...)` and the fixed-state/`(attach off)`/`(lock_type
//!   position)` sub-scopes are INLINE writes (raw `"("` … `")"`), never
//!   `startScope` — they close on the SAME line they open.

use crate::coordinate_transform::CoordinateTransform;
use crate::layer_structure::Layer;
use crate::ses_board::{ItemIr, SesBoard};
use crate::shape::Shape;
use crate::sink::{BoardSink, FixedStateIr};
use crate::write_scope::{
    IndentFileWriter, java_double_to_string, quote_java_identifier, unit_to_dsn_string,
};
use epic_geometry::rounding::java_round;
use std::collections::HashSet;

/// Java `SesWriter.write(board, out, designName)` (`:55-67`): the whole
/// session document as a `String`. `design_name` is the design file name
/// the harness passes (e.g. `board.dsn`); the session name is
/// `design_name.replace(".dsn", ".ses")` — Java `String.replace` swaps
/// EVERY literal occurrence, as does [`str::replace`].
///
/// A board that never completed `create_board` (no transform/layers)
/// has no Java-reachable counterpart (`SesWriter` requires a built
/// board); the port returns an empty string for it — unreachable for
/// every board the harness emits (parse Success implies create_board).
pub fn write_session(board: &SesBoard, design_name: &str) -> String {
    let (Some(board_transform), Some(layers)) = (&board.transform, &board.layers) else {
        return String::new();
    };
    let session_name = design_name.replace(".dsn", ".ses");
    let string_quote = board.metadata.string_quote.as_str();
    // T43: session scale = board scale / resolution, base 0.
    let scale_factor =
        board_transform.dsn_to_board_value(1.0) / f64::from(board.metadata.resolution);
    let session_transform = CoordinateTransform::new(scale_factor, 0.0, 0.0);
    let mut file = IndentFileWriter::new();

    // writeSessionScope (:73-94)
    file.start_scope_with_newline(false);
    file.write("session ");
    file.write(&quote_java_identifier(&session_name, string_quote));
    file.new_line();
    file.write("(base_design ");
    file.write(&quote_java_identifier(design_name, string_quote));
    file.write(")");
    write_placement(board, string_quote, &session_transform, &mut file);
    write_was_is(&mut file);
    write_routes(board, string_quote, &session_transform, layers, &mut file);
    file.end_scope();
    file.into_string()
}

/// `IdentifierType.write(name, file)`: the processed (possibly quoted)
/// name appended raw.
fn write_identifier(file: &mut IndentFileWriter, name: &str, string_quote: &str) {
    file.write(&quote_java_identifier(name, string_quote));
}

/// Java `writePlacement` (`:96-114`): `(placement` + resolution scope +
/// components grouped per PACKAGE in LIBRARY order.
fn write_placement(
    board: &SesBoard,
    string_quote: &str,
    session_transform: &CoordinateTransform,
    file: &mut IndentFileWriter,
) {
    file.start_scope();
    file.write("placement");
    write_resolution_scope(board, file);
    for package_index in 0..board.packages.len() {
        write_components(board, string_quote, session_transform, package_index, file);
    }
    file.end_scope();
}

/// Java `writeComponents` (`:117-151`): all components whose CURRENT-SIDE
/// package is `packages[package_index]` (Java `getPackage() == pkg` is
/// reference identity — the same package number), COMPONENT-TABLE order
/// (ids ascending), gated on the component having at least one
/// non-deleted item (`getComponentId() == component.id` over all items:
/// pins, keepouts and component outlines are the only id-carrying
/// kinds; traces/vias/conduction areas never match since ids start at
/// 1). The `(component <pkg>` scope opens at the FIRST surviving
/// component and closes after the last.
fn write_components(
    board: &SesBoard,
    string_quote: &str,
    session_transform: &CoordinateTransform,
    package_index: usize,
    file: &mut IndentFileWriter,
) {
    let package_no = package_index as i32 + 1;
    let mut component_found = false;
    for (component_index, component) in board.components.iter().enumerate() {
        let current_package_no = if component.is_front {
            component.package_front
        } else {
            component.package_back
        };
        if current_package_no != package_no {
            continue;
        }
        let component_id = component_index as i32 + 1;
        let undeleted_item_found = board.items.iter().any(|item| match item {
            ItemIr::Pin { pin, .. } => pin.component_id == component_id,
            ItemIr::Keepout { keepout, .. } => keepout.component_id == component_id,
            ItemIr::ComponentOutline { outline, .. } => outline.component_id == component_id,
            _ => false,
        });
        if !undeleted_item_found {
            continue;
        }
        if !component_found {
            file.start_scope();
            file.write("component ");
            write_identifier(file, &board.packages[package_index].name, string_quote);
            component_found = true;
        }
        write_component(component, string_quote, session_transform, file);
    }
    if component_found {
        file.end_scope();
    }
}

/// Java `writeComponent` (`:153-181`): the inline `(place ...)` line.
/// `positionFixed` is reconstructed as `fixed == SYSTEM_FIXED` — the
/// reader derives the component fixed state 1:1 from
/// `location.positionFixed` (`Network.java:972-977`), so the mapping
/// round-trips. A component without a location cannot occur in the
/// component table (the reader never inserts unplaced components,
/// `Network.java:968-970`); the port skips its place line.
fn write_component(
    component: &crate::sink::ComponentIr,
    string_quote: &str,
    session_transform: &CoordinateTransform,
    file: &mut IndentFileWriter,
) {
    let Some(location) = component.location else {
        return;
    };
    file.new_line();
    file.write("(place ");
    write_identifier(file, &component.name, string_quote);
    let location_dsn = session_transform.board_to_dsn_point(location.to_float());
    let xcoordinate = java_round(location_dsn[0]);
    let ycoordinate = java_round(location_dsn[1]);
    file.write(" ");
    file.write(&xcoordinate.to_string());
    file.write(" ");
    file.write(&ycoordinate.to_string());
    if component.is_front {
        file.write(" front ");
    } else {
        file.write(" back ");
    }
    file.write(&format_placement_rotation(component.rotation));
    if component.fixed == FixedStateIr::SystemFixed {
        file.new_line();
        file.write(" (lock_type position)");
    }
    file.write(")");
}

/// Java `writeWasIs` (`:183-218`): the `(was_is` scope — empty on a
/// parse-time board (module docs).
fn write_was_is(file: &mut IndentFileWriter) {
    file.start_scope();
    file.write("was_is");
    file.end_scope();
}

/// Java `writeRoutes` (`:220-233`).
fn write_routes(
    board: &SesBoard,
    string_quote: &str,
    session_transform: &CoordinateTransform,
    layers: &crate::layer_structure::LayerStructure,
    file: &mut IndentFileWriter,
) {
    file.start_scope();
    file.write("routes ");
    write_resolution_scope(board, file);
    write_parser_scope_reduced(board, string_quote, file);
    write_library(board, string_quote, session_transform, file);
    write_network(board, string_quote, session_transform, layers, file);
    file.end_scope();
}

/// Java `Resolution.writeScope` (`Resolution.java:17-25`): one inline
/// line `(resolution <unit> <n>)`.
fn write_resolution_scope(board: &SesBoard, file: &mut IndentFileWriter) {
    file.new_line();
    file.write("(resolution ");
    file.write(unit_to_dsn_string(board.metadata.unit));
    file.write(" ");
    file.write(&board.metadata.resolution.to_string());
    file.write(")");
}

/// Java `Parser.writeScope(file, parserInfo, identifierType, reduced =
/// true)` (`Parser.java:92-137`): `reduced` skips
/// `string_quote`/`space_in_quoted_tokens`; `host_cad`/`host_version`
/// emit when non-null. `constants`/`write_resolution` sub-scopes are
/// omitted (module docs: `MetadataIr` carries neither).
fn write_parser_scope_reduced(board: &SesBoard, string_quote: &str, file: &mut IndentFileWriter) {
    file.start_scope();
    file.write("parser");
    if let Some(host_cad) = &board.metadata.host_cad {
        file.new_line();
        file.write("(host_cad ");
        write_identifier(file, host_cad, string_quote);
        file.write(")");
    }
    if let Some(host_version) = &board.metadata.host_version {
        file.new_line();
        file.write("(host_version ");
        write_identifier(file, host_version, string_quote);
        file.write(")");
    }
    file.end_scope();
}

/// Java `writeLibrary` (`:235-252`): `(library_out` + one padstack per
/// VIA-PADSTACK-LIST entry, deduped FIRST-NAME-WINS (Java
/// `LinkedHashSet.add`). The list is walked in list order, not registry
/// order ([`SesBoard::via_padstacks`] holds 1-based registry numbers).
fn write_library(
    board: &SesBoard,
    string_quote: &str,
    session_transform: &CoordinateTransform,
    file: &mut IndentFileWriter,
) {
    file.start_scope();
    file.write("library_out ");
    let mut written_padstack_names: HashSet<&str> = HashSet::new();
    for padstack_no in &board.via_padstacks {
        let Some(padstack) = board.padstack(*padstack_no) else {
            continue; // Java getViaPadstack(int) null out of range
        };
        if !written_padstack_names.insert(padstack.name.as_str()) {
            continue;
        }
        write_padstack(padstack, board, string_quote, session_transform, file);
    }
    file.end_scope();
}

/// Java `writePadstack` (`:271-313`): `(padstack <name>` + one
/// `(shape ...)` per layer between the first and last non-null shape
/// (inclusive scan; null layers INSIDE the range are skipped), then
/// `(attach off)` iff `!attachAllowed` (= `!isDrillable`,
/// `Padstack.java:57`). A padstack with no shape on any layer warns and
/// is skipped entirely (Java `return`).
fn write_padstack(
    padstack: &crate::sink::PadstackIr,
    board: &SesBoard,
    string_quote: &str,
    session_transform: &CoordinateTransform,
    file: &mut IndentFileWriter,
) {
    let layer_count = board.layers.as_ref().map_or(0, |ls| ls.layers.len());
    let shape_at = |no: i32| -> Option<&crate::shape::BoardShape> {
        padstack
            .shapes
            .get(usize::try_from(no).ok()?)
            .and_then(|slot| slot.as_ref())
    };
    let mut first_layer_no = 0usize;
    while first_layer_no < layer_count && shape_at(first_layer_no as i32).is_none() {
        first_layer_no += 1;
    }
    let mut last_layer_no = layer_count as i32 - 1;
    while last_layer_no >= 0 && shape_at(last_layer_no).is_none() {
        last_layer_no -= 1;
    }
    if first_layer_no >= layer_count || last_layer_no < 0 {
        return; // Java warns + returns (no padstack scope)
    }
    file.start_scope();
    file.write("padstack ");
    write_identifier(file, &padstack.name, string_quote);
    let layers = board.layers.as_ref().map_or(&[] as &[_], |ls| &ls.layers);
    for i in first_layer_no..=last_layer_no as usize {
        let Some(current_board_shape) = shape_at(i as i32) else {
            continue;
        };
        // Java board.layerStructure.layers[i] — in range by the scan
        // bounds above (first/last both < layer_count).
        let layer = layers.get(i).map_or_else(
            || Layer::new("", i as i32, true),
            |board_layer| Layer::new(&board_layer.name, i as i32, board_layer.is_signal),
        );
        // Java boardToDsnRel null (IntOctagon/Simplex mismatch) would
        // NPE at writeScopeInt — unreachable for reader-produced
        // padstack shapes (rect/circle/polygon); the port skips.
        if let Some(current_shape) =
            session_transform.board_to_dsn_rel_shape(current_board_shape, &layer)
        {
            file.start_scope();
            file.write("shape");
            shape_write_scope_int(&current_shape, string_quote, file);
            file.end_scope();
        }
    }
    if !padstack.drillable {
        file.new_line();
        file.write("(attach off)");
    }
    file.end_scope();
}

/// Java `writeNetwork` (`:315-327`): nets `1..=maxNetNumber` — the
/// board net table is dense (`Nets.add` numbers sequentially), so the
/// max is the table length.
fn write_network(
    board: &SesBoard,
    string_quote: &str,
    session_transform: &CoordinateTransform,
    layers: &crate::layer_structure::LayerStructure,
    file: &mut IndentFileWriter,
) {
    file.start_scope();
    file.write("network_out ");
    for net_number in 1..=board.nets.len() as i32 {
        write_net(
            net_number,
            board,
            string_quote,
            session_transform,
            layers,
            file,
        );
    }
    file.end_scope();
}

/// Java `writeNet` (`:329-370`) — descending-id walk (T39, module
/// docs). SYSTEM_FIXED items are skipped BEFORE dispatch (so a net of
/// only fixed items emits nothing — no scope). A conduction area on a
/// NON-SIGNAL layer does not count as one (neither opens the header
/// nor emits).
fn write_net(
    net_number: i32,
    board: &SesBoard,
    string_quote: &str,
    session_transform: &CoordinateTransform,
    layers: &crate::layer_structure::LayerStructure,
    file: &mut IndentFileWriter,
) {
    let mut header_written = false;
    for item in board.items.iter().rev() {
        let kind = match item {
            ItemIr::Trace { trace, .. } if trace.nets.contains(&net_number) => {
                (trace.fixed, &trace.nets)
            }
            ItemIr::Via { via, .. } if via.nets.contains(&net_number) => (via.fixed, &via.nets),
            ItemIr::ConductionArea { area, .. }
                if area.nets.contains(&net_number)
                    && usize::try_from(area.layer_no)
                        .ok()
                        .and_then(|slot| layers.layers.get(slot))
                        .is_some_and(|layer| layer.is_signal) =>
            {
                (area.fixed, &area.nets)
            }
            _ => continue,
        };
        let (fixed, _) = kind;
        if fixed == FixedStateIr::SystemFixed {
            continue;
        }
        if !header_written {
            file.start_scope();
            file.write("net ");
            if let Some(name) = board.net_name(net_number) {
                write_identifier(file, name, string_quote);
            }
            header_written = true;
        }
        match item {
            ItemIr::Trace { trace, .. } => {
                write_wire(trace, layers, string_quote, session_transform, file)
            }
            ItemIr::Via { via, .. } => write_via(via, board, string_quote, session_transform, file),
            ItemIr::ConductionArea { area, .. } => {
                write_conduction_area(area, layers, string_quote, session_transform, file)
            }
            _ => unreachable!("the kind match above filtered to these three"),
        }
    }
    if header_written {
        file.end_scope();
    }
}

/// Java `writeWire` (`:372-415`): `(wire` + `(path <layer> <width>`
/// with per-line corners + optional fixed-state line. Width =
/// `java_round(boardToDsn(2 * halfWidth))` (an INT). Corners dedup
/// against the PREVIOUS emitted corner only (Java `prevCoors`
/// comparison; runs of identical corners collapse to one).
fn write_wire(
    wire: &crate::sink::TraceIr,
    layers: &crate::layer_structure::LayerStructure,
    string_quote: &str,
    session_transform: &CoordinateTransform,
    file: &mut IndentFileWriter,
) {
    let wire_width =
        java_round(session_transform.board_to_dsn_value(2.0 * f64::from(wire.half_width)));
    file.start_scope();
    file.write("wire");
    let corners = &wire.corners;
    let mut coors: Vec<i64> = Vec::with_capacity(2 * corners.len());
    let mut prev_coors: Option<[i64; 2]> = None;
    for (i, corner) in corners.iter().enumerate() {
        let current_float_coors = session_transform.board_to_dsn_point(corner.to_float());
        let current_coors = [
            java_round(current_float_coors[0]),
            java_round(current_float_coors[1]),
        ];
        if i == 0 || Some(current_coors) != prev_coors {
            coors.push(current_coors[0]);
            coors.push(current_coors[1]);
            prev_coors = Some(current_coors);
        }
    }
    // Java board.layerStructure.layers[layerIndex] — an out-of-range
    // trace layer would AIOOBE (unreachable: traces insert on board
    // layers); the port falls back to the number string, which cannot
    // reach the goldens.
    let layer_name = layers
        .layers
        .get(usize::try_from(wire.layer_no).ok().unwrap_or(usize::MAX))
        .map_or_else(|| wire.layer_no.to_string(), |layer| layer.name.clone());
    write_path(&layer_name, wire_width, &coors, string_quote, file);
    write_fixed_state(wire.fixed, file);
    file.end_scope();
}

/// Java `writeVia` (`:455-476`).
fn write_via(
    via: &crate::sink::ViaIr,
    board: &SesBoard,
    string_quote: &str,
    session_transform: &CoordinateTransform,
    file: &mut IndentFileWriter,
) {
    let padstack_name = board
        .padstack(via.padstack_no)
        .map(|padstack| padstack.name.clone())
        .unwrap_or_default();
    file.start_scope();
    file.write("via ");
    write_identifier(file, &padstack_name, string_quote);
    file.write(" ");
    let location = session_transform.board_to_dsn_point(via.location.to_float());
    file.write(&java_round(location[0]).to_string());
    file.write(" ");
    file.write(&java_round(location[1]).to_string());
    write_fixed_state(via.fixed, file);
    file.end_scope();
}

/// Java `writeFixedState` (`:478-490`): nothing at/below SHOVE_FIXED;
/// `fix` for SYSTEM_FIXED (dead in this path — `writeNet` skips
/// SYSTEM_FIXED items first, so only `protect` is reachable); else
/// `protect`. Inline, never a scope.
fn write_fixed_state(fixed_state: FixedStateIr, file: &mut IndentFileWriter) {
    match fixed_state {
        FixedStateIr::Unfixed | FixedStateIr::ShoveFixed => {}
        FixedStateIr::SystemFixed => {
            file.new_line();
            file.write("(type fix)");
        }
        FixedStateIr::UserFixed => {
            file.new_line();
            file.write("(type protect)");
        }
    }
}

/// Java `writeConductionArea` (`:514-551`): `(wire ` + the area border
/// (int-rounded) + one `(window ...)` per hole (RAW DOUBLES via
/// [`java_double_to_string`]). A net count != 1 warns and emits NOTHING
/// (no scope). The shape transform uses the ABSOLUTE `boardToDsn`
/// flavor (the session transform's base is 0, so absolute == relative
/// numerically — the flavor distinction is ported for fidelity).
fn write_conduction_area(
    conduction_area: &crate::sink::ConductionAreaIr,
    layers: &crate::layer_structure::LayerStructure,
    string_quote: &str,
    session_transform: &CoordinateTransform,
    file: &mut IndentFileWriter,
) {
    if conduction_area.nets.len() != 1 {
        return; // Java warns + returns
    }
    let Some(board_layer) = layers.layers.get(
        usize::try_from(conduction_area.layer_no)
            .ok()
            .unwrap_or(usize::MAX),
    ) else {
        return; // Java AIOOBE — unreachable: areas insert on board layers
    };
    let layer = Layer::new(
        &board_layer.name,
        conduction_area.layer_no,
        board_layer.is_signal,
    );
    file.start_scope();
    file.write("wire ");
    if let Some(dsn_shape) =
        session_transform.board_to_dsn_shape(&conduction_area.area.border, &layer)
    {
        shape_write_scope_int(&dsn_shape, string_quote, file);
    }
    for hole in &conduction_area.area.holes {
        // Java writeHoleScope has NO null check (NPE on a
        // null-transforming hole — unreachable for reader-produced
        // polygon windows); the port skips such a hole.
        if let Some(dsn_hole) = session_transform.board_to_dsn_shape(hole, &layer) {
            shape_write_hole_scope(&dsn_hole, string_quote, file);
        }
    }
    file.end_scope();
}

/// Java `writePath` (`:492-512`): `(path <layer> <width>` + per-line
/// corner pairs.
fn write_path(
    layer_name: &str,
    width: i64,
    coors: &[i64],
    string_quote: &str,
    file: &mut IndentFileWriter,
) {
    file.start_scope();
    file.write("path ");
    write_identifier(file, layer_name, string_quote);
    file.write(" ");
    file.write(&width.to_string());
    let corner_count = coors.len() / 2;
    for i in 0..corner_count {
        file.new_line();
        file.write(&coors[2 * i].to_string());
        file.write(" ");
        file.write(&coors[2 * i + 1].to_string());
    }
    file.end_scope();
}

/// `Shape.writeScopeInt` dispatch (the five parser leaves, rounded
/// int coordinates; `Circle.java:79-90`, `Rectangle.java:85-95`,
/// `Polygon.java:93-110`, `PolygonPath.java:39-56`,
/// `PolylinePath.java:38-54`).
fn shape_write_scope_int(shape: &Shape, string_quote: &str, file: &mut IndentFileWriter) {
    match shape {
        Shape::Rectangle(rectangle) => {
            file.new_line();
            file.write("(rect ");
            write_identifier(file, &rectangle.layer.name, string_quote);
            for coor in rectangle.coor {
                file.write(" ");
                file.write(&java_round(coor).to_string());
            }
            file.write(")");
        }
        Shape::Circle(circle) => {
            file.new_line();
            file.write("(circle ");
            write_identifier(file, &circle.layer.name, string_quote);
            for coor in circle.coor {
                file.write(" ");
                file.write(&java_round(coor).to_string());
            }
            file.write(")");
        }
        Shape::Polygon(polygon) => {
            file.start_scope();
            file.write("polygon ");
            write_identifier(file, &polygon.layer.name, string_quote);
            file.write(" 0");
            let corner_count = polygon.coor.len() / 2;
            for i in 0..corner_count {
                file.new_line();
                file.write(&java_round(polygon.coor[2 * i]).to_string());
                file.write(" ");
                file.write(&java_round(polygon.coor[2 * i + 1]).to_string());
            }
            file.end_scope();
        }
        Shape::PolygonPath(path) => {
            file.start_scope();
            file.write("path ");
            write_identifier(file, &path.layer.name, string_quote);
            file.write(" ");
            // Java width is a double here: String.valueOf(double)
            file.write(&java_double_to_string(path.width));
            let corner_count = path.coordinate_arr.len() / 2;
            for i in 0..corner_count {
                file.new_line();
                file.write(&java_round(path.coordinate_arr[2 * i]).to_string());
                file.write(" ");
                file.write(&java_round(path.coordinate_arr[2 * i + 1]).to_string());
            }
            file.end_scope();
        }
        Shape::PolylinePath(path) => {
            let Some(layer) = &path.layer else {
                return; // unreachable: writer-built shapes carry layers
            };
            file.start_scope();
            file.write("polyline_path ");
            write_identifier(file, &layer.name, string_quote);
            file.write(" ");
            file.write(&java_double_to_string(path.width));
            let line_count = path.coordinate_arr.len() / 4;
            for i in 0..line_count {
                file.new_line();
                for j in 0..4 {
                    file.write(&java_round(path.coordinate_arr[4 * i + j]).to_string());
                    file.write(" ");
                }
            }
            file.end_scope();
        }
    }
}

/// `Shape.writeScope` dispatch (RAW DOUBLES — hole windows only,
/// `Circle.java:67-76`, `Rectangle.java:73-82`, `Polygon.java:76-90`,
/// `PolygonPath.java:22-36`, `PolylinePath.java:20-35`).
fn shape_write_scope(shape: &Shape, string_quote: &str, file: &mut IndentFileWriter) {
    match shape {
        Shape::Rectangle(rectangle) => {
            file.new_line();
            file.write("(rect ");
            write_identifier(file, &rectangle.layer.name, string_quote);
            for coor in rectangle.coor {
                file.write(" ");
                file.write(&java_double_to_string(coor));
            }
            file.write(")");
        }
        Shape::Circle(circle) => {
            file.new_line();
            file.write("(circle ");
            write_identifier(file, &circle.layer.name, string_quote);
            for coor in circle.coor {
                file.write(" ");
                file.write(&java_double_to_string(coor));
            }
            file.write(")");
        }
        Shape::Polygon(polygon) => {
            file.start_scope();
            file.write("polygon ");
            write_identifier(file, &polygon.layer.name, string_quote);
            file.write(" 0");
            let corner_count = polygon.coor.len() / 2;
            for i in 0..corner_count {
                file.new_line();
                file.write(&java_double_to_string(polygon.coor[2 * i]));
                file.write(" ");
                file.write(&java_double_to_string(polygon.coor[2 * i + 1]));
            }
            file.end_scope();
        }
        Shape::PolygonPath(path) => {
            file.start_scope();
            file.write("path ");
            write_identifier(file, &path.layer.name, string_quote);
            file.write(" ");
            file.write(&java_double_to_string(path.width));
            let corner_count = path.coordinate_arr.len() / 2;
            for i in 0..corner_count {
                file.new_line();
                file.write(&java_double_to_string(path.coordinate_arr[2 * i]));
                file.write(" ");
                file.write(&java_double_to_string(path.coordinate_arr[2 * i + 1]));
            }
            file.end_scope();
        }
        Shape::PolylinePath(path) => {
            let Some(layer) = &path.layer else {
                return;
            };
            file.start_scope();
            file.write("polyline_path ");
            write_identifier(file, &layer.name, string_quote);
            file.write(" ");
            file.write(&java_double_to_string(path.width));
            let line_count = path.coordinate_arr.len() / 4;
            for i in 0..line_count {
                file.new_line();
                for j in 0..4 {
                    file.write(&java_double_to_string(path.coordinate_arr[4 * i + j]));
                    file.write(" ");
                }
            }
            file.end_scope();
        }
    }
}

/// `Shape.writeHoleScope` (`Shape.java:607-613`): `(window` + the
/// double-precision shape scope + `)`.
fn shape_write_hole_scope(shape: &Shape, string_quote: &str, file: &mut IndentFileWriter) {
    file.start_scope();
    file.write("window");
    shape_write_scope(shape, string_quote, file);
    file.end_scope();
}

/// Java `formatPlacementRotation` (`:259-269`): KiCad-style rotation
/// text — `rint(degrees * 1000) / 1000` first, then `%.0f` when the
/// rounded value is (within 1e-9 of) an integer, else `%.3f` with
/// trailing zeros and a trailing dot trimmed. `rint` is ties-to-even
/// (`f64::round_ties_even`); a tie at the 4th decimal is unreachable
/// (the rounded value is `n/1000`), so Rust `{:.3}` matches Java's
/// HALF_UP `%.3f` on every reachable input.
pub fn format_placement_rotation(degrees: f64) -> String {
    let rounded = (degrees * 1000.0).round_ties_even() / 1000.0;
    if (rounded - rounded.round_ties_even()).abs() < 1e-9 {
        return format!("{rounded:.0}");
    }
    let mut formatted = format!("{rounded:.3}");
    if formatted.contains('.') {
        while formatted.ends_with('0') {
            formatted.pop();
        }
        if formatted.ends_with('.') {
            formatted.pop();
        }
    }
    formatted
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer_structure::LayerStructure;
    use crate::sink::{
        AreaIr, BoardRulesIr, CreateBoardIr, ImageIr, ImagePinIr, NetIr, PadstackIr, PinIr,
    };
    use crate::state::Unit;
    use epic_geometry::circle::Circle as BoardCircle;
    use epic_geometry::int_box::IntBox;
    use epic_geometry::int_point::IntPoint;
    use epic_geometry::point::Point;
    use epic_geometry::polygon_shape::PolygonShape;

    /// `formatPlacementRotation` jar pins — every expected string captured
    /// from `SesWriter.formatPlacementRotation` on
    /// `build/libs/freerouting-current-executable.jar` (jshell
    /// `/tmp/epic-t13-rot.jsh` + `/tmp/epic-t13-rot2.jsh`, reflection, JDK
    /// 25). The `0.0625`/`0.5625` rows are the `rint` TIES-TO-EVEN
    /// discriminators (0.0625*1000 = 62.5 EXACTLY — HALF_UP would print
    /// `0.063`); `359.9995 -> 360` pins rounding BEFORE the integer test;
    /// `0.0004 -> 0` and `0.0006 -> 0.001` pin the sub-milli-degree collapse;
    /// `12.3456789 -> 12.346` pins `%.3f` HALF_UP at the 4th decimal (not a
    /// tie, so it cannot discriminate the tie rule — the 0.0625 rows do).
    #[test]
    fn format_placement_rotation_jar_pins() {
        assert_eq!(format_placement_rotation(0.0), "0");
        assert_eq!(format_placement_rotation(270.0), "270");
        assert_eq!(format_placement_rotation(90.0), "90");
        assert_eq!(format_placement_rotation(338.5), "338.5");
        assert_eq!(format_placement_rotation(339.9994), "339.999");
        assert_eq!(format_placement_rotation(0.5004), "0.5");
        assert_eq!(format_placement_rotation(-45.5), "-45.5");
        assert_eq!(format_placement_rotation(45.123456), "45.123");
        assert_eq!(format_placement_rotation(359.9995), "360");
        assert_eq!(format_placement_rotation(0.0004), "0");
        assert_eq!(format_placement_rotation(0.0006), "0.001");
        assert_eq!(format_placement_rotation(359.99949), "359.999");
        assert_eq!(format_placement_rotation(1_000_000.5), "1000000.5");
        assert_eq!(format_placement_rotation(12.3456789), "12.346");
        assert_eq!(format_placement_rotation(0.0015), "0.002");
        assert_eq!(format_placement_rotation(2.0005), "2.001");
        assert_eq!(format_placement_rotation(179.9999), "180");
        // rint ties-to-even discriminators (0.0625 = 1/16 is dyadic, so the
        // x1000 product is the exact tie 62.5; HALF_UP prints "0.063").
        assert_eq!(format_placement_rotation(0.0625), "0.062");
        assert_eq!(format_placement_rotation(0.5625), "0.562");
        assert_eq!(format_placement_rotation(-0.0625), "-0.062");
        assert_eq!(format_placement_rotation(0.1875), "0.188");
    }

    /// `writeFixedState` (`SesWriter.java:478-490`): nothing at/below
    /// SHOVE_FIXED, `(type fix)` for SYSTEM_FIXED (dead through
    /// `write_session` — `write_net` skips those first — but the arm is the
    /// Java mapping), `(type protect)` for USER_FIXED.
    #[test]
    fn write_fixed_state_mapping() {
        let render = |fixed: FixedStateIr| {
            let mut file = IndentFileWriter::new();
            file.start_scope();
            file.write("wire");
            write_fixed_state(fixed, &mut file);
            file.end_scope();
            file.into_string()
        };
        // start_scope on a fresh writer newline-firsts at level 0, hence the
        // leading "\n(" in every render.
        assert_eq!(render(FixedStateIr::Unfixed), "\n(wire\n)");
        assert_eq!(render(FixedStateIr::ShoveFixed), "\n(wire\n)");
        assert_eq!(
            render(FixedStateIr::UserFixed),
            "\n(wire\n  (type protect)\n)"
        );
        assert_eq!(
            render(FixedStateIr::SystemFixed),
            "\n(wire\n  (type fix)\n)"
        );
    }

    /// Two um/10 signal layers (the spike-fixture layer structure).
    fn two_layers() -> LayerStructure {
        LayerStructure::new(vec![
            crate::layer_structure::Layer::new("F.Cu", 0, true),
            crate::layer_structure::Layer::new("B.Cu", 1, true),
        ])
    }

    /// `create_board` + um/10 metadata on a fresh board.
    fn create_two_layer_board() -> SesBoard {
        let mut board = SesBoard::new();
        board.metadata.unit = Unit::Um;
        board.metadata.resolution = 10;
        board.create_board(CreateBoardIr {
            bounding_box: IntBox::new(IntPoint::new(0, 0), IntPoint::new(100_000, 100_000)),
            layer_structure: two_layers(),
            outline_shapes: Vec::new(),
            outline_clearance_class: Some("default".to_string()),
            rules: BoardRulesIr::new(2),
            transform: CoordinateTransform::new(10.0, 0.0, 0.0),
        });
        board
    }

    fn circle_padstack_layer(radius: i32) -> Option<crate::shape::BoardShape> {
        Some(crate::shape::BoardShape::Circle(BoardCircle::new(
            IntPoint::new(0, 0),
            radius,
        )))
    }

    /// The full-session scaffold pin: every scope of `SesWriter` on a
    /// hand-built parse-time board, asserted as ONE exact string. Layout
    /// cross-checked line-for-line against the real Freerouting session
    /// `fixtures/Issue313-FastTest.ses` (`(routes `/`(library_out `/
    /// `(network_out ` TRAILING SPACES at :13/:23/:189; `(was_is` + empty;
    /// the via/wire/path block at :194-208 with `(type protect)` inline and
    /// the path width `1772` an INT). Pins in one shot:
    ///
    /// - session name `.dsn -> .ses` + `(base_design ...)`;
    /// - T43 scale 1 (board 10 / resolution 10): emitted coordinates are the
    ///   BOARD integers (place `20000 50000` for board (20000,50000));
    /// - placement groups by CURRENT-SIDE package (U3 is back-side with
    ///   `package_back` = the same package 1 and lands in the SAME
    ///   `(component PAD` scope), components in TABLE order;
    /// - the undeleted gate: U4 (package 1, NO pin) never places;
    /// - `(lock_type position)` inline sub-line for the SYSTEM_FIXED U2
    ///   (SesWriter.java:167-181 raw writes: newLine + one leading space);
    /// - library_out walks the VIA-PADSTACK LIST (1,2,1,3,4): the second
    ///   entry `1` dedups by name, EmptyPad (no shapes) is skipped whole;
    /// - `(attach off)` for the non-drillable RoundPad;
    /// - the padstack shape arms: circle (diameter = 2 * r / session scale)
    ///   and polygon (with the literal ` 0` aperture);
    /// - network_out nets 1..=len with items in DESCENDING id (T39): via 8,
    ///   dedup trace 7, protect trace 6, route trace 5;
    /// - wire corner dedup (trace 7 repeats (10000,60000) — emitted once)
    ///   and the INT path width `1250` (= java_round(2*625/1));
    /// - USER_FIXED -> `(type protect)`, UNFIXED -> nothing.
    #[test]
    fn session_scaffold_full_text() {
        let mut board = create_two_layer_board();
        board.append_padstack(PadstackIr {
            name: "ViaPad_F".to_string(),
            shapes: vec![circle_padstack_layer(500), circle_padstack_layer(500)],
            drillable: true,
            placed_absolute: false,
        });
        board.append_padstack(PadstackIr {
            name: "RoundPad".to_string(),
            shapes: vec![circle_padstack_layer(400), None],
            drillable: false,
            placed_absolute: false,
        });
        board.append_padstack(PadstackIr {
            name: "PolyPad".to_string(),
            shapes: vec![
                Some(crate::shape::BoardShape::PolygonShape(PolygonShape::new(
                    &[
                        Point::Int(IntPoint::new(0, 0)),
                        Point::Int(IntPoint::new(3000, 0)),
                        Point::Int(IntPoint::new(3000, 3000)),
                        Point::Int(IntPoint::new(0, 3000)),
                    ],
                ))),
                None,
            ],
            drillable: true,
            placed_absolute: false,
        });
        board.append_padstack(PadstackIr {
            name: "EmptyPad".to_string(),
            shapes: vec![None, None],
            drillable: true,
            placed_absolute: false,
        });
        // LIST order 1,2,1,3,4: ViaPad_F, RoundPad, dup-1 (name-dedup skip),
        // PolyPad, EmptyPad (shape-less skip).
        board.set_via_padstacks(vec![1, 2, 1, 3, 4]);
        board.append_net(NetIr {
            name: "PERFECT".to_string(),
            subnet_number: 1,
            contains_plane: false,
            net_class: 0,
        });
        board.insert_package(ImageIr {
            name: "PAD".to_string(),
            pins: vec![ImagePinIr {
                name: "1".to_string(),
                padstack_no: 2,
                rel_location: IntPoint::new(0, 0),
                rotation: 0.0,
            }],
            outline: Vec::new(),
            keepouts: Vec::new(),
            via_keepouts: Vec::new(),
            place_keepouts: Vec::new(),
            is_front: true,
        });
        board.insert_component(crate::sink::ComponentIr {
            name: "U1".to_string(),
            package_front: 1,
            package_back: 1,
            location: Some(IntPoint::new(20_000, 50_000)),
            rotation: 90.0,
            is_front: true,
            fixed: FixedStateIr::Unfixed,
            part_number: None,
            logical_part: None,
        });
        board.insert_component(crate::sink::ComponentIr {
            name: "U2".to_string(),
            package_front: 1,
            package_back: 1,
            location: Some(IntPoint::new(60_000, 50_000)),
            rotation: 338.5,
            is_front: true,
            fixed: FixedStateIr::SystemFixed,
            part_number: None,
            logical_part: None,
        });
        board.insert_component(crate::sink::ComponentIr {
            name: "U3".to_string(),
            package_front: 1,
            package_back: 1,
            location: Some(IntPoint::new(60_000, 60_000)),
            rotation: 0.0,
            is_front: false,
            fixed: FixedStateIr::Unfixed,
            part_number: None,
            logical_part: None,
        });
        board.insert_component(crate::sink::ComponentIr {
            name: "U4".to_string(),
            package_front: 1,
            package_back: 1,
            location: Some(IntPoint::new(10_000, 10_000)),
            rotation: 0.0,
            is_front: true,
            fixed: FixedStateIr::Unfixed,
            part_number: None,
            logical_part: None,
        });
        for component_id in 1..=3 {
            board.insert_pin(PinIr {
                component_id,
                pin_index: 0,
                padstack_no: 2,
                nets: vec![1],
                clearance_class: 1,
                fixed: FixedStateIr::Unfixed,
            });
        }
        board.insert_trace(crate::sink::TraceIr {
            layer_no: 0,
            half_width: 625,
            corners: vec![
                IntPoint::new(10_000, 10_000),
                IntPoint::new(30_000, 10_000),
                IntPoint::new(30_000, 30_000),
            ],
            nets: vec![1],
            clearance_class: 1,
            fixed: FixedStateIr::Unfixed,
        });
        board.insert_trace(crate::sink::TraceIr {
            layer_no: 0,
            half_width: 625,
            corners: vec![IntPoint::new(10_000, 40_000), IntPoint::new(30_000, 40_000)],
            nets: vec![1],
            clearance_class: 1,
            fixed: FixedStateIr::UserFixed,
        });
        board.insert_trace(crate::sink::TraceIr {
            layer_no: 0,
            half_width: 625,
            corners: vec![
                IntPoint::new(10_000, 60_000),
                IntPoint::new(10_000, 60_000),
                IntPoint::new(30_000, 60_000),
            ],
            nets: vec![1],
            clearance_class: 1,
            fixed: FixedStateIr::Unfixed,
        });
        board.insert_via(crate::sink::ViaIr {
            padstack_no: 1,
            location: IntPoint::new(50_000, 50_000),
            nets: vec![1],
            clearance_class: 1,
            fixed: FixedStateIr::Unfixed,
            attach_smd_allowed: false,
        });
        // ids: 1 outline, 2-4 pins, 5-7 traces, 8 via (insertion order).
        assert_eq!(board.last_assigned_item_id(), 8);

        let expected = concat!(
            "(session board.ses\n",
            "  (base_design board.dsn)\n",
            "  (placement\n",
            "    (resolution um 10)\n",
            "    (component PAD\n",
            "      (place U1 20000 50000 front 90)\n",
            "      (place U2 60000 50000 front 338.5\n",
            "       (lock_type position))\n",
            "      (place U3 60000 60000 back 0)\n",
            "    )\n",
            "  )\n",
            "  (was_is\n",
            "  )\n",
            "  (routes \n",
            "    (resolution um 10)\n",
            "    (parser\n",
            "    )\n",
            "    (library_out \n",
            "      (padstack \"ViaPad_F\"\n",
            "        (shape\n",
            "          (circle F.Cu 1000 0 0)\n",
            "        )\n",
            "        (shape\n",
            "          (circle B.Cu 1000 0 0)\n",
            "        )\n",
            "      )\n",
            "      (padstack RoundPad\n",
            "        (shape\n",
            "          (circle F.Cu 800 0 0)\n",
            "        )\n",
            "        (attach off)\n",
            "      )\n",
            "      (padstack PolyPad\n",
            "        (shape\n",
            "          (polygon F.Cu 0\n",
            "            0 0\n",
            "            3000 0\n",
            "            3000 3000\n",
            "            0 3000\n",
            "          )\n",
            "        )\n",
            "      )\n",
            "    )\n",
            "    (network_out \n",
            "      (net PERFECT\n",
            "        (via \"ViaPad_F\" 50000 50000\n",
            "        )\n",
            "        (wire\n",
            "          (path F.Cu 1250\n",
            "            10000 60000\n",
            "            30000 60000\n",
            "          )\n",
            "        )\n",
            "        (wire\n",
            "          (path F.Cu 1250\n",
            "            10000 40000\n",
            "            30000 40000\n",
            "          )\n",
            "          (type protect)\n",
            "        )\n",
            "        (wire\n",
            "          (path F.Cu 1250\n",
            "            10000 10000\n",
            "            30000 10000\n",
            "            30000 30000\n",
            "          )\n",
            "        )\n",
            "      )\n",
            "    )\n",
            "  )\n",
            ")", // the session ends after the root `)` — NO trailing newline
        );
        assert_eq!(write_session(&board, "board.dsn"), expected);
    }

    /// The skip paths of `writeNet`/`writeConductionArea`: net 1's ONLY item
    /// is a SYSTEM_FIXED trace — the skip precedes the header, so net 1
    /// emits NOTHING (a port that dispatched SYSTEM_FIXED items would emit
    /// an empty net scope or a `(type fix)` wire). The conduction area
    /// carries TWO nets (2, 3): each opens its net scope but the wire
    /// warns-and-returns (`SesWriter.java:517-519`) — header-only nets, the
    /// bug-compatible Java behavior (a port that pre-filtered the net list
    /// would drop both scopes).
    #[test]
    fn system_fixed_net_skipped_and_multi_net_area_header_only() {
        let mut board = create_two_layer_board();
        let net = |name: &str| NetIr {
            name: name.to_string(),
            subnet_number: 1,
            contains_plane: false,
            net_class: 0,
        };
        board.append_net(net("ONLYFIX"));
        board.append_net(net("AREA2"));
        board.append_net(net("AREA3"));
        board.insert_trace(crate::sink::TraceIr {
            layer_no: 0,
            half_width: 625,
            corners: vec![IntPoint::new(10_000, 10_000), IntPoint::new(30_000, 10_000)],
            nets: vec![1],
            clearance_class: 1,
            fixed: FixedStateIr::SystemFixed,
        });
        board.insert_conduction_area(crate::sink::ConductionAreaIr {
            layer_no: 0,
            area: AreaIr::simple(crate::shape::BoardShape::Tile(
                epic_geometry::tile_shape::TileShape::RegularTileShape(
                    epic_geometry::regular_tile_shape::RegularTileShape::IntBox(IntBox::new(
                        IntPoint::new(40_000, 40_000),
                        IntPoint::new(60_000, 60_000),
                    )),
                ),
            )),
            nets: vec![2, 3],
            clearance_class: 1,
            fixed: FixedStateIr::Unfixed,
        });

        let expected = concat!(
            "(session board.ses\n",
            "  (base_design board.dsn)\n",
            "  (placement\n",
            "    (resolution um 10)\n",
            "  )\n",
            "  (was_is\n",
            "  )\n",
            "  (routes \n",
            "    (resolution um 10)\n",
            "    (parser\n",
            "    )\n",
            "    (library_out \n",
            "    )\n",
            "    (network_out \n",
            "      (net AREA2\n",
            "      )\n",
            "      (net AREA3\n",
            "      )\n",
            "    )\n",
            "  )\n",
            ")", // the session ends after the root `)` — NO trailing newline
        );
        assert_eq!(write_session(&board, "board.dsn"), expected);
    }

    /// The conduction-area wire: `(wire ` WITH trailing space (unlike trace
    /// wires), the border rectangle through the INT scope (`java_round`
    /// values), and each hole as a `(window` scope carrying the RAW-DOUBLE
    /// scope (`java_double_to_string`: `45000.0`, not `45000`) — the
    /// int-vs-double quirk of `writeConductionArea`
    /// (`SesWriter.java:514-551`).
    #[test]
    fn conduction_area_window_holes_emit_doubles() {
        let mut board = create_two_layer_board();
        board.append_net(NetIr {
            name: "GND".to_string(),
            subnet_number: 1,
            contains_plane: false,
            net_class: 0,
        });
        let rect = |x0: i32, y0: i32, x1: i32, y1: i32| {
            crate::shape::BoardShape::Tile(epic_geometry::tile_shape::TileShape::RegularTileShape(
                epic_geometry::regular_tile_shape::RegularTileShape::IntBox(IntBox::new(
                    IntPoint::new(x0, y0),
                    IntPoint::new(x1, y1),
                )),
            ))
        };
        board.insert_conduction_area(crate::sink::ConductionAreaIr {
            layer_no: 0,
            area: AreaIr {
                border: rect(40_000, 40_000, 60_000, 60_000),
                holes: vec![rect(45_000, 45_000, 55_000, 55_000)],
            },
            nets: vec![1],
            clearance_class: 1,
            fixed: FixedStateIr::Unfixed,
        });

        let expected = concat!(
            "(session board.ses\n",
            "  (base_design board.dsn)\n",
            "  (placement\n",
            "    (resolution um 10)\n",
            "  )\n",
            "  (was_is\n",
            "  )\n",
            "  (routes \n",
            "    (resolution um 10)\n",
            "    (parser\n",
            "    )\n",
            "    (library_out \n",
            "    )\n",
            "    (network_out \n",
            "      (net GND\n",
            "        (wire \n",
            "          (rect F.Cu 40000 40000 60000 60000)\n",
            "          (window\n",
            "            (rect F.Cu 45000.0 45000.0 55000.0 55000.0)\n",
            "          )\n",
            "        )\n",
            "      )\n",
            "    )\n",
            "  )\n",
            ")", // the session ends after the root `)` — NO trailing newline
        );
        assert_eq!(write_session(&board, "board.dsn"), expected);
    }

    /// T40 round-trip: a minimal DSN through the READER, then the writer —
    /// emitted wire endpoints are exactly the parsed corners (NO endpoint
    /// snapping, module docs) and the T43 composed scale shows up as
    /// board integers: source DSN `5000 1000 -> 5000 2000` (um/10, board
    /// scale 10, session scale 1) emits `50000 10000 -> 50000 20000`, and
    /// the DSN width `125` (half-width 625 board) emits as INT `1250`.
    #[test]
    fn t40_round_trip_emits_parsed_corners_untouched() {
        const ROUND_TRIP_DSN: &str = r#"(pcb board.dsn
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
    (wire (path F.Cu 125  5000 1000  5000 2000) (net PERFECT))
  )
)
"#;
        let mut board = SesBoard::new();
        let result = crate::reader::read_board(ROUND_TRIP_DSN.as_bytes(), &mut board);
        assert!(
            matches!(result, crate::reader::DsnReadResult::Success { .. }),
            "expected Success, got {result:?}"
        );

        let expected = concat!(
            "(session board.ses\n",
            "  (base_design board.dsn)\n",
            "  (placement\n",
            "    (resolution um 10)\n",
            "  )\n",
            "  (was_is\n",
            "  )\n",
            "  (routes \n",
            "    (resolution um 10)\n",
            "    (parser\n",
            "    )\n",
            "    (library_out \n",
            "    )\n",
            "    (network_out \n",
            "      (net PERFECT\n",
            "        (wire\n",
            "          (path F.Cu 1250\n",
            "            50000 10000\n",
            "            50000 20000\n",
            "          )\n",
            "        )\n",
            "      )\n",
            "    )\n",
            "  )\n",
            ")", // the session ends after the root `)` — NO trailing newline
        );
        assert_eq!(write_session(&board, "board.dsn"), expected);
    }
}
