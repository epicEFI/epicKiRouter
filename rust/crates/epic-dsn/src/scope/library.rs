//! The `(library ...)` and `(part_library ...)` scope readers: the port of
//! `io/specctra/parser/Library.java` (`readScope` + `readPadstackScope` +
//! `generateMissingKeepoutNames` + `arePackagePinsIdentical`),
//! `io/specctra/parser/Package.java` (the `(image ...)` scope: pins,
//! outlines, keepouts, side), and `io/specctra/parser/PartLibrary.java`.
//!
//! Java call shape: both scopes dispatch from the pcb-level loop. The
//! `(image ...)` scopes are parsed into an internal [`ParserImage`] during
//! the library loop; AFTER the loop closes, every image is converted to an
//! [`ImageIr`] (pins through the padstack registry, outline/keepout shapes
//! through `transformToBoardRel`) and inserted behind the `Packages.get`
//! dedup loop (`Library.java:309-450`).
//!
//! Jar sessions (repo root, JDK 25 jar): `/tmp/epic-t7-pad.dsn`,
//! `/tmp/epic-t7-image.dsn`, `/tmp/epic-t7-image-bad.dsn`,
//! `/tmp/epic-t7-part.dsn`, `/tmp/epic-t7-part2.dsn`, output
//! `/tmp/epic-t7-probe.out`.
//!
//! Documented divergences:
//! - Java RESETS the board padstack table at library-scope start
//!   (`Library.java:264-265`: `board.library.padstacks = new Padstacks(...)`)
//!   and rebuilds the package table (`:330`). The Rust sink ACCUMULATES
//!   (there is no reset seam); a second library scope in one file would
//!   differ. Corpus-dead: none of the 175 fixtures has two library scopes.
//! - The dedup loop's `catch (Exception)` fallback (insert under the
//!   ORIGINAL name, `Library.java:436-443`) is unreachable in Rust —
//!   [`BoardSink::package_no`] cannot throw.
//! - The plan's Task-2 note claimed `Package.writeScope:175` passes the
//!   RELATIVE pin location through the base-ADDING `boardToDsn(FloatPoint[])`
//!   overload. REFUTED against the source: `:175` calls the `Vector`
//!   overload, which does NOT add the base — matching the reader here
//!   (`readPinInfo` coordinates are relative; only the scale applies).
//!   Relevant to the Task 13 writer, not to this reader.
//! - `Package.Pin` locations are transformed with `dsnToBoard(double)` —
//!   the PURE SCALE overload (`io/CoordinateTransform.java:158-160`), NOT
//!   a point transform; Task 8 expands pins per placement.
//! - BTreeSet ordering vs Java TreeSet (UTF-8 vs UTF-16) — see
//!   [`LogicalPartMapping`].

use epic_geometry::circle::Circle as BoardCircle;
use epic_geometry::int_point::IntPoint;
use epic_geometry::rounding::java_round;
use epic_geometry::tile_shape::TileShape;

use crate::coordinate_transform::CoordinateTransform;
use crate::keyword::{Keyword, skip_scope};
use crate::layer_structure::{Layer, LayerStructure};
use crate::lexer::{LexicalState, Scanner, Token};
use crate::scope::structure::read_on_off_scope;
use crate::ses_board::{strip_package_dedup_suffix, strip_padstack_alias};
use crate::shape::{
    BoardShape, Shape, TransformedAreaRel, read_area_scope, transform_area_to_board_rel,
};
use crate::shape::{number_value, read_scope as read_shape_scope};
use crate::sink::{
    BoardSink, ImageIr, ImageKeepoutIr, ImageOutlineIr, ImagePinIr, PadstackIr, eq_ignore_case,
};
use crate::state::{AreaScopeResult, LogicalPart, LogicalPartMapping, ParseState, PartPin};

/// Java `Library.readScope` (`Library.java:262-452`). The parse loop
/// collects `(padstack ...)` insertions (immediate) and `(image ...)` scopes
/// (deferred); the conversion + dedup insertion pass runs AFTER the loop.
/// EOF inside the loop is FATAL (`:276-282` — the opposite of the
/// placement scope's inherited loop).
///
/// `pub` (like [`crate::scope::structure::read_scope`]): the pcb-level
/// dispatcher that calls it lands with Task 9, and the crate keeps its
/// scope-reader entry points public until then.
pub fn read_library_scope(
    scanner: &mut Scanner,
    state: &mut ParseState,
    sink: &mut dyn BoardSink,
) -> bool {
    // Java `board.library.padstacks = new Padstacks(...)` (`:264-265`) —
    // the RESET divergence is documented in the module docs.
    let Some(layer_structure) = state.layer_structure.clone() else {
        // Java dereferences `layerStructure` unguarded — an NPE there is
        // parse death; the library scope only runs after create_board.
        return false;
    };
    let Some(coordinate_transform) = state.coordinate_transform else {
        return false;
    };
    let mut package_list: Vec<ParserImage> = Vec::new();
    let mut prev_token: Option<Token> = None;
    loop {
        let next_token = scanner.next_token();
        if matches!(next_token, Token::Eof | Token::Error(_)) {
            // Java warns "unexpected end of file" -> return FALSE
            return false;
        }
        if next_token == Token::Close {
            break;
        }
        if prev_token == Some(Token::Open) {
            match next_token {
                Token::Keyword(Keyword::Padstack) => {
                    if !read_padstack_scope(scanner, &layer_structure, &coordinate_transform, sink)
                    {
                        return false;
                    }
                }
                Token::Keyword(Keyword::Image) => {
                    let Some(image) = read_image_scope(scanner, &layer_structure) else {
                        return false;
                    };
                    package_list.push(image);
                }
                _ => {
                    skip_scope(scanner);
                }
            }
        }
        prev_token = Some(next_token);
    }
    // Create the library packages on the board (`Library.java:309-450`).
    for image in package_list {
        if !insert_image_with_dedup(image, &coordinate_transform, sink) {
            return false;
        }
    }
    true
}

/// Java `Library.readPadstackScope` (`Library.java:101-224`). `true` keeps
/// the parse alive — a padstack may legitimately end up NOT added (already
/// in the registry, or an empty shape list); `false` is parse-fatal.
fn read_padstack_scope(
    scanner: &mut Scanner,
    layer_structure: &LayerStructure,
    coordinate_transform: &CoordinateTransform,
    sink: &mut dyn BoardSink,
) -> bool {
    let mut is_drilllable = true;
    let mut placed_absolute = false;
    let mut shape_list: Vec<Shape> = Vec::new();
    // The padstack keyword switches the scanner to its NAME state, so a
    // fully numeric name (`(padstack 1 ...)`, jar t7-image) scans as a
    // string. The name is `.N`-stripped HERE (`:113`), BEFORE the dedup
    // query (`:158`) and the registry add (`:222`).
    let padstack_name = match scanner.next_token() {
        Token::Str(name) => strip_padstack_alias(&name),
        _ => {
            // Java warns "unexpected padstack identifier"
            return false;
        }
    };
    // Java seeds `prevToken` with the name string token itself. The walk
    // never reuses the Box<str>, so a cheap lossy seed token is fine.
    let mut prev_token = Token::Str(padstack_name.clone().into_boxed_str());
    let mut next_token = scanner.next_token();
    while next_token != Token::Close {
        if matches!(next_token, Token::Eof | Token::Error(_)) {
            // Java: IOException -> error log -> false; the Rust scanner
            // reports EOF.
            return false;
        }
        if prev_token == Token::Open {
            match next_token {
                Token::Keyword(Keyword::Shape) => {
                    // A null shape (unknown layer in a layer-less scope) is
                    // simply not added (`:128-131`).
                    if let Some(shape) = read_shape_scope(scanner, Some(layer_structure)) {
                        shape_list.push(shape);
                    }
                    // Overread ONE closing bracket, skipping any unknown
                    // trailing scopes (`:133-137`); anything but CLOSE is
                    // fatal (`:138-144`).
                    let mut current_next_token = scanner.next_token();
                    while current_next_token == Token::Open {
                        skip_scope(scanner);
                        current_next_token = scanner.next_token();
                    }
                    if current_next_token != Token::Close {
                        // Java warns "closing bracket expected"
                        return false;
                    }
                }
                Token::Keyword(Keyword::Attach) => {
                    is_drilllable = read_on_off_scope(scanner);
                }
                Token::Keyword(Keyword::Absolute) => {
                    placed_absolute = read_on_off_scope(scanner);
                }
                _ => {
                    skip_scope(scanner);
                }
            }
        }
        prev_token = next_token;
        next_token = scanner.next_token();
    }
    // Dedup BEFORE the empty-shape check (`:158-161`).
    if sink.resolve_padstack_query(&padstack_name).is_some() {
        return true;
    }
    if shape_list.is_empty() {
        // Java warns "shape not found for padstack ..." — log-only, and the
        // padstack is NOT added (return TRUE, parse survives).
        return true;
    }
    // The fill-vs-assign conversion (`:169-221`): slots start null (Java
    // `new ConvexShape[layerCount]`), a layer-less `(shape ...)` FILLS every
    // slot, a layered one ASSIGNS only its layer, later shapes overwrite.
    let mut padstack_shapes: Vec<Option<BoardShape>> = vec![None; layer_structure.layers.len()];
    for pad_shape in &shape_list {
        // Java dereferences the transform result unguarded — the
        // PolylinePath stub's null NPEs at the `splitToConvex` call
        // (both `instanceof` checks fail first; jar
        // /tmp/epic-t8-partb.out t8-pcpath: uncaught
        // NullPointerException) — parse death.
        let Some(current_shape) = pad_shape.transform_to_board_rel(coordinate_transform) else {
            return false;
        };
        let convex_shape: ConvexBoardShape = match current_shape {
            BoardShape::Tile(tile) => ConvexBoardShape::Tile(tile),
            BoardShape::Circle(circle) => ConvexBoardShape::Circle(circle),
            BoardShape::PolygonShape(polygon) => {
                let hulled = polygon.convex_hull();
                let Some(mut pieces) = hulled.split_to_convex() else {
                    // Java NPEs on the null array at `convexShapes[0]`.
                    return false;
                };
                if pieces.is_empty() {
                    // Java AIOOBE at `convexShapes[0]`.
                    return false;
                }
                if pieces.len() != 1 {
                    // Java warns "convex shape expected" — log-only; [0] is
                    // still taken.
                }
                let mut piece = pieces.swap_remove(0);
                // `instanceof Simplex` -> simplify (the max-int bbox
                // explosion for a degenerate segment hull, jar t7-pad LINE).
                if let TileShape::Simplex(simplex) = piece {
                    piece = simplex.simplify();
                }
                ConvexBoardShape::Tile(piece)
            }
        };
        // The "not an area" workaround (`:192-205`): offset(1) once; a
        // still-1-dimensional result lands as a NULL slot (jar t7-pad DEG1).
        let padstack_shape = if convex_shape.dimension() < 2 {
            let offset_shape = convex_shape.offset(1.0);
            if offset_shape.dimension() < 2 {
                None
            } else {
                Some(offset_shape)
            }
        } else {
            Some(convex_shape.into_board_shape())
        };
        // PCB/signal layer names FILL every slot (`:208-209`); a named
        // layer ASSIGNS one slot after the range check (`:210-220` — an
        // out-of-range layer is parse-fatal).
        let Some(layer_name) = shape_layer_name(pad_shape) else {
            // Dead arm: a PolylinePath layer-less shape died in the
            // transform stub above.
            return false;
        };
        if layer_name == Layer::PCB_NAME || layer_name == Layer::SIGNAL_NAME {
            for slot in &mut padstack_shapes {
                *slot = padstack_shape.clone();
            }
        } else {
            let shape_layer = layer_structure.get_no(layer_name);
            if shape_layer < 0 || shape_layer as usize >= padstack_shapes.len() {
                // Java warns "layer number found" -> return FALSE
                return false;
            }
            padstack_shapes[shape_layer as usize] = padstack_shape;
        }
    }
    sink.append_padstack(PadstackIr {
        name: padstack_name,
        shapes: padstack_shapes,
        drillable: is_drilllable,
        placed_absolute,
    });
    true
}

/// The parser layer NAME of a shape (Java `padShape.layer`); `None` for the
/// PolylinePath unknown-layer case (dead here — the transform stub rejects
/// every PolylinePath first).
fn shape_layer_name(shape: &Shape) -> Option<&str> {
    match shape {
        Shape::Rectangle(s) => Some(&s.layer.name),
        Shape::Polygon(s) => Some(&s.layer.name),
        Shape::Circle(s) => Some(&s.layer.name),
        Shape::PolygonPath(s) => Some(&s.layer.name),
        Shape::PolylinePath(s) => s.layer.as_ref().map(|layer| layer.name.as_str()),
    }
}

/// Java `Package.readScope` (`Package.java:57-150`): parses one `(image
/// ...)` scope. `None` = Java null = parse-fatal at the caller.
fn read_image_scope(
    scanner: &mut Scanner,
    layer_structure: &LayerStructure,
) -> Option<ParserImage> {
    let Token::Str(image_name) = scanner.next_token() else {
        // Java warns "String expected"
        return None;
    };
    let mut image = ParserImage {
        name: image_name.to_string(),
        pin_infos: Vec::new(),
        outlines: Vec::new(),
        keepouts: Vec::new(),
        via_keepouts: Vec::new(),
        place_keepouts: Vec::new(),
        is_front: true,
    };
    // Java seeds `prevToken` with the NAME token itself.
    let mut prev_token = Token::Str(image_name);
    let mut next_token = scanner.next_token();
    loop {
        if matches!(next_token, Token::Eof | Token::Error(_)) {
            // Java warns "unexpected end of file" -> null
            return None;
        }
        if next_token == Token::Close {
            break;
        }
        if prev_token == Token::Open {
            match next_token {
                Token::Keyword(Keyword::Pin) => {
                    let pin_info = read_pin_info(scanner)?;
                    image.pin_infos.push(pin_info);
                }
                Token::Keyword(Keyword::Side) => {
                    image.is_front = read_placement_side(scanner)?;
                }
                Token::Keyword(Keyword::Outline) => {
                    let current_shape = read_shape_scope(scanner, Some(layer_structure));
                    if let Some(shape) = current_shape {
                        image.outlines.push(shape);
                    }
                    // Overread ONE closing bracket — unknown leftovers are
                    // NOT skipped here (unlike the padstack SHAPE branch);
                    // anything but CLOSE is fatal (`:103-112`).
                    if scanner.next_token() != Token::Close {
                        // Java warns "closed bracket expected"
                        return None;
                    }
                }
                Token::Keyword(Keyword::Keepout) => {
                    // A failed area read logs an ERROR (log-only) and the
                    // parse CONTINUES (`:117-127`).
                    if let Some(keepout) = read_area_scope(scanner, Some(layer_structure), false) {
                        image.keepouts.push(keepout);
                    }
                }
                Token::Keyword(Keyword::ViaKeepout) => {
                    // A failed via-keepout read is silently skipped.
                    if let Some(keepout) = read_area_scope(scanner, Some(layer_structure), false) {
                        image.via_keepouts.push(keepout);
                    }
                }
                Token::Keyword(Keyword::PlaceKeepout) => {
                    if let Some(keepout) = read_area_scope(scanner, Some(layer_structure), false) {
                        image.place_keepouts.push(keepout);
                    }
                }
                _ => {
                    skip_scope(scanner);
                }
            }
        }
        prev_token = next_token;
        next_token = scanner.next_token();
    }
    Some(image)
}

/// Java `Package.readPinInfo` (`Package.java:249-330`). `None` = Java null
/// = parse-fatal at the caller.
fn read_pin_info(scanner: &mut Scanner) -> Option<PinInfo> {
    // yybegin(NAME): the padstack name may be numeric (`(pin 1 7 500 500)`
    // — padstack "1", pin "7", jar t7-image PIN 4).
    scanner.set_lexical_state(LexicalState::Name);
    let padstack_name = match scanner.next_token() {
        Token::Str(name) => name.to_string(),
        Token::Int(value) => value.to_string(),
        _ => {
            // Java warns "String or Integer expected"
            return None;
        }
    };
    let mut rotation = 0.0;
    // Optional `(rotate ...)` BEFORE the pin name (again NAME-forced, so a
    // digit-leading pin name scans as a string).
    scanner.set_lexical_state(LexicalState::Name);
    let mut next_token = scanner.next_token();
    if next_token == Token::Open {
        let inner = scanner.next_token();
        if matches!(inner, Token::Error(_)) {
            return None;
        }
        if inner == Token::Keyword(Keyword::Rotate) {
            rotation = read_rotation(scanner)?;
        } else {
            skip_scope(scanner);
        }
        scanner.set_lexical_state(LexicalState::Name);
        next_token = scanner.next_token();
    }
    let pin_name = match &next_token {
        Token::Str(name) => name.to_string(),
        Token::Int(value) => value.to_string(),
        _ => {
            // Java warns "String or Integer expected"
            return None;
        }
    };
    let mut rel_coor = [0.0f64; 2];
    for slot_value in &mut rel_coor {
        match number_value(&scanner.next_token()) {
            Some(value) => *slot_value = value,
            None => {
                // Java warns "number expected" — Error tokens land here too
                return None;
            }
        }
    }
    // Trailing scopes: CLOSE ends the pin; a `(rotate ...)` updates the
    // rotation; anything else is skip-scoped. EOF is fatal.
    loop {
        let prev_token = next_token;
        next_token = scanner.next_token();
        if next_token == Token::Eof {
            // Java warns "unexpected end of file"
            return None;
        }
        if matches!(next_token, Token::Error(_)) {
            return None;
        }
        if next_token == Token::Close {
            break;
        }
        if prev_token == Token::Open {
            if next_token == Token::Keyword(Keyword::Rotate) {
                rotation = read_rotation(scanner)?;
            } else {
                skip_scope(scanner);
            }
        }
    }
    Some(PinInfo {
        padstack_name,
        pin_name,
        rel_coor,
        rotation,
    })
}

/// Java `Package.readRotation` (`Package.java:332-352`): a STRICT
/// `Double.parseDouble` over `nextString()`. `None` mirrors the uncaught
/// NumberFormatException — parse death (jar `/tmp/epic-t7-rot-bad.dsn`:
/// `(rotate 1.5x)` dies at `Package.readRotation:337` BEFORE the closing
/// bracket is consumed). A missing bracket only WARNS (log-only) and keeps
/// the parsed value.
fn read_rotation(scanner: &mut Scanner) -> Option<f64> {
    let next_string = scanner.next_string();
    let result = crate::lexer::parse_double_strict(&next_string)?;
    if scanner.next_token() != Token::Close {
        // Java warns "closing bracket expected" — value is kept
    }
    Some(result)
}

/// Java `Package.readPlacementSide` (`Package.java:389-401`): ANY token
/// that is not `back` means front (including garbage/EOF); a missing
/// bracket only warns. An overflow integer still dies: both tokens are
/// LEXED before the front fallback applies, and the scanner's
/// NumberFormatException escapes every IOException-only catch — parse
/// death (jar `/tmp/epic-t8-errorpin.jsh`, output
/// `/tmp/epic-t8-errorpin.out` t8-errorpin2: `(side 99999999999)` inside
/// an image dies with an uncaught NumberFormatException).
/// `None` = that throw = parse-fatal at the caller.
fn read_placement_side(scanner: &mut Scanner) -> Option<bool> {
    let result = match scanner.next_token() {
        Token::Error(_) => return None,
        token => token != Token::Keyword(Keyword::Back),
    };
    match scanner.next_token() {
        Token::Close => {}
        Token::Error(_) => return None,
        _ => {
            // Java warns "closing bracket expected"
        }
    }
    Some(result)
}

/// One outline/keepout conversion target: the convex shapes that survive
/// the `instanceof ConvexShape` filter (`Library.java:186-205`).
enum ConvexBoardShape {
    Tile(TileShape),
    Circle(BoardCircle),
}

impl ConvexBoardShape {
    fn into_board_shape(self) -> BoardShape {
        match self {
            Self::Tile(tile) => BoardShape::Tile(tile),
            Self::Circle(circle) => BoardShape::Circle(circle),
        }
    }

    /// Java `ConvexShape.dimension()` (2 for every Circle).
    fn dimension(&self) -> i32 {
        match self {
            Self::Tile(tile) => tile.dimension(),
            Self::Circle(circle) => circle.dimension(),
        }
    }

    /// Java `ConvexShape.offset(1)` — the "not an area" workaround.
    fn offset(&self, distance: f64) -> BoardShape {
        match self {
            Self::Tile(tile) => BoardShape::Tile(tile.offset(distance)),
            Self::Circle(circle) => BoardShape::Circle(circle.offset(distance)),
        }
    }
}

/// Java `Library.generateMissingKeepoutNames` (`Library.java:454-472`):
/// PER-LIST — if ANY entry of THIS list lacks a name, EVERY entry of this
/// list is renamed `<prefix><n>` in file order, 1-based, OVERWRITING
/// existing names (jar t7-image: keepouts `K1` + unnamed become
/// `keepout_1`/`keepout_2`; the place-keepout list has no null, so `VK1`
/// SURVIVES — the per-list discriminator).
fn generate_missing_keepout_names(prefix: &str, keepout_list: &mut [AreaScopeResult]) {
    let all_names_existing = keepout_list.iter().all(|k| k.area_name.is_some());
    if all_names_existing {
        return;
    }
    for (index, keepout) in keepout_list.iter_mut().enumerate() {
        keepout.area_name = Some(format!("{}{}", prefix, index + 1));
    }
}

/// Java `Library.arePackagePinsIdentical` (`Library.java:226-259`): pin
/// count, name (CASE-SENSITIVE `String.equals`), padstack id, location
/// within 0.001, rotation within 0.001.
fn package_pins_identical(stored: &[ImagePinIr], new_pins: &[ImagePinIr]) -> bool {
    if stored.len() != new_pins.len() {
        return false;
    }
    for (pin1, pin2) in stored.iter().zip(new_pins.iter()) {
        if pin1.name != pin2.name {
            return false;
        }
        if pin1.padstack_no != pin2.padstack_no {
            return false;
        }
        let loc1 = (
            f64::from(pin1.rel_location.x),
            f64::from(pin1.rel_location.y),
        );
        let loc2 = (
            f64::from(pin2.rel_location.x),
            f64::from(pin2.rel_location.y),
        );
        if (loc1.0 - loc2.0).abs() > 0.001 || (loc1.1 - loc2.1).abs() > 0.001 {
            return false;
        }
        if (pin1.rotation - pin2.rotation).abs() > 0.001 {
            return false;
        }
    }
    true
}

/// The post-loop conversion pass (`Library.java:330-450`): pins through the
/// padstack registry, outlines/keepouts through `transformToBoardRel`, then
/// the `Packages.get` dedup insertion. `false` is parse-fatal (missing
/// padstack, missing keepout layer, or the Threw transform flavor).
fn insert_image_with_dedup(
    mut image: ParserImage,
    coordinate_transform: &CoordinateTransform,
    sink: &mut dyn BoardSink,
) -> bool {
    let mut pins: Vec<ImagePinIr> = Vec::with_capacity(image.pin_infos.len());
    for pin_info in &image.pin_infos {
        // `(int) Math.round(dsnToBoard(relCoor[i]))` — the PURE SCALE
        // overload; jar t7-image: 300.55 -> 3006, 0.55 -> 6 (half-up).
        let rel_x =
            java_round(coordinate_transform.dsn_to_board_value(pin_info.rel_coor[0])) as i32;
        let rel_y =
            java_round(coordinate_transform.dsn_to_board_value(pin_info.rel_coor[1])) as i32;
        let cleaned_lookup_name = strip_padstack_alias(&pin_info.padstack_name);
        let Some(padstack_no) = sink.resolve_padstack_query(&cleaned_lookup_name) else {
            // Java warns "board padstack '<raw>' (cleaned: '<cleaned>') not
            // found" and returns FALSE — parse death (jar t7-image-bad).
            return false;
        };
        pins.push(ImagePinIr {
            name: pin_info.pin_name.clone(),
            padstack_no,
            rel_location: IntPoint::new(rel_x, rel_y),
            rotation: pin_info.rotation,
        });
    }
    let mut outlines: Vec<ImageOutlineIr> = Vec::with_capacity(image.outlines.len());
    for shape in &image.outlines {
        // The transform result and the width/isClosed writes are
        // INDEPENDENT (`Library.java:350-366`): a null transform (the
        // PolylinePath stub) stores a null entry but STILL sets both —
        // jar t7-image OUTLINE 3: `null w=10.0 closed=false`.
        let board_shape = shape.transform_to_board_rel(coordinate_transform);
        let (width, is_closed) = match shape {
            Shape::PolylinePath(path) => (path.width, path_is_closed(&path.coordinate_arr)),
            Shape::PolygonPath(path) => (path.width, path_is_closed(&path.coordinate_arr)),
            // Non-path shapes (polygons/rects) are closed with width 0.0.
            _ => (0.0, true),
        };
        outlines.push(ImageOutlineIr {
            shape: board_shape,
            width,
            is_closed,
        });
    }
    let convert_keepouts = |list: &[AreaScopeResult]| -> Option<Vec<ImageKeepoutIr>> {
        let mut converted = Vec::with_capacity(list.len());
        for keepout in list {
            // The layer comes from the FIRST shape's parser layer BEFORE the
            // transform (`:369` etc.) — a null first layer is Java NPE death.
            let layer_no = first_shape_layer_no(keepout)?;
            match transform_area_to_board_rel(&keepout.shapes, coordinate_transform) {
                TransformedAreaRel::Area(area) => converted.push(ImageKeepoutIr {
                    name: keepout.area_name.clone().unwrap_or_default(),
                    layer_no,
                    area: Some(area),
                }),
                TransformedAreaRel::Null => converted.push(ImageKeepoutIr {
                    name: keepout.area_name.clone().unwrap_or_default(),
                    layer_no,
                    area: None,
                }),
                // A null WINDOW entry dereferenced in the hole loop — uncaught
                // in Library.readScope: parse death.
                TransformedAreaRel::Threw => return None,
            }
        }
        Some(converted)
    };
    // Rename + convert in place — no clones: `image` is owned here.
    generate_missing_keepout_names("keepout_", &mut image.keepouts);
    generate_missing_keepout_names("via_keepout_", &mut image.via_keepouts);
    generate_missing_keepout_names("place_keepout_", &mut image.place_keepouts);
    // A Threw keepout (null window entry) is parse death at every list.
    let Some(keepouts) = convert_keepouts(&image.keepouts) else {
        return false;
    };
    let Some(via_keepouts) = convert_keepouts(&image.via_keepouts) else {
        return false;
    };
    let Some(place_keepouts) = convert_keepouts(&image.place_keepouts) else {
        return false;
    };

    // The dedup loop (`Library.java:409-449`): insert under `NAME` or the
    // first free `NAME::k`, unless an existing SAME-NAME package has
    // identical pins. The Java `catch` fallback (insert under the original
    // name) is unreachable in Rust.
    let base_name = strip_package_dedup_suffix(&image.name);
    let mut suffix: i32 = 0;
    loop {
        let test_name = if suffix == 0 {
            base_name.clone()
        } else {
            format!("{}::{}", base_name, suffix)
        };
        let insert_here = match sink.package_no(&test_name, image.is_front) {
            None => true,
            Some(existing_no) => {
                let exact_match = sink
                    .package_name(existing_no)
                    .is_some_and(|stored| eq_ignore_case(stored, &test_name));
                if !exact_match {
                    true
                } else if package_pins_identical(sink.package_pins(existing_no), &pins) {
                    return true;
                } else {
                    false
                }
            }
        };
        if insert_here {
            sink.insert_package(ImageIr {
                name: test_name,
                pins,
                outline: outlines,
                keepouts,
                via_keepouts,
                place_keepouts,
                is_front: image.is_front,
            });
            return true;
        }
        suffix += 1;
    }
}

/// `Package.java:352-354`: `coords.length >= 4 && first == last` on the
/// RAW doubles (exact `==`).
fn path_is_closed(coordinate_arr: &[f64]) -> bool {
    coordinate_arr.len() >= 4
        && coordinate_arr[0] == coordinate_arr[coordinate_arr.len() - 2]
        && coordinate_arr[1] == coordinate_arr[coordinate_arr.len() - 1]
}

/// The keepout layer: `shapeList.iterator().next().layer.no` — the FIRST
/// shape's parser layer number. `None` = Java NPE (null layer / null first
/// entry): parse death.
fn first_shape_layer_no(keepout: &AreaScopeResult) -> Option<i32> {
    match keepout.shapes.first()? {
        Some(shape) => shape_layer_no(shape),
        None => None,
    }
}

fn shape_layer_no(shape: &Shape) -> Option<i32> {
    match shape {
        Shape::Rectangle(s) => Some(s.layer.no),
        Shape::Polygon(s) => Some(s.layer.no),
        Shape::Circle(s) => Some(s.layer.no),
        Shape::PolygonPath(s) => Some(s.layer.no),
        // The PolylinePath layer is `None` (Java null) for an unknown layer.
        Shape::PolylinePath(s) => s.layer.as_ref().map(|layer| layer.no),
    }
}

/// Java `PartLibrary.readScope` (`PartLibrary.java:95-126`): EOF is fatal;
/// failed mapping/part reads are fatal; unknown inner scopes are skipped.
/// `pub` — the Task 9 pcb dispatcher owns the call site (see
/// [`read_library_scope`]).
pub fn read_part_library_scope(
    scanner: &mut Scanner,
    logical_part_mappings: &mut Vec<LogicalPartMapping>,
    logical_parts: &mut Vec<LogicalPart>,
) -> bool {
    let mut prev_token: Option<Token> = None;
    loop {
        let next_token = scanner.next_token();
        if matches!(next_token, Token::Eof | Token::Error(_)) {
            // Java warns "unexpected end of file"
            return false;
        }
        if next_token == Token::Close {
            break;
        }
        if prev_token == Some(Token::Open) {
            match next_token {
                Token::Keyword(Keyword::LogicalPartMapping) => {
                    let Some(mapping) = read_logical_part_mapping(scanner) else {
                        return false;
                    };
                    logical_part_mappings.push(mapping);
                }
                Token::Keyword(Keyword::LogicalPart) => {
                    let Some(part) = read_logical_part(scanner) else {
                        return false;
                    };
                    logical_parts.push(part);
                }
                _ => {
                    skip_scope(scanner);
                }
            }
        }
        prev_token = Some(next_token);
    }
    true
}

/// Java `PartLibrary.readLogicalPartMapping` (`PartLibrary.java:127-182`):
/// name + OPEN + the COMPONENT keyword + a NAME-state token loop building a
/// sorted set + a SECOND closing bracket.
fn read_logical_part_mapping(scanner: &mut Scanner) -> Option<LogicalPartMapping> {
    let Token::Str(name) = scanner.next_token() else {
        // Java warns "string expected"
        return None;
    };
    if scanner.next_token() != Token::Open {
        // Java warns "open bracket expected"
        return None;
    }
    if scanner.next_token() != Token::Keyword(Keyword::Component) {
        // Java warns "Keyword.COMPONENT_SCOPE expected" — `comp` and
        // `component` both scan as Keyword::Component.
        return None;
    }
    let mut components = std::collections::BTreeSet::new();
    loop {
        // yybegin(NAME) BEFORE EVERY token: component names that collide
        // with keywords stay strings; `)` still closes.
        scanner.set_lexical_state(LexicalState::Name);
        match scanner.next_token() {
            Token::Close => break,
            Token::Str(component) => {
                components.insert(component.to_string());
            }
            Token::Eof | Token::Error(_) => {
                // Java warns "string expected"; IOException -> false
                return None;
            }
            _ => {
                // Java warns "string expected"
                return None;
            }
        }
    }
    if scanner.next_token() != Token::Close {
        // Java warns "closing bracket expected"
        return None;
    }
    Some(LogicalPartMapping {
        name: name.to_string(),
        components,
    })
}

/// Java `PartLibrary.readLogicalPart` (`PartLibrary.java:184-237`): a
/// prev/next walk collecting `(pin ...)` entries; EOF is fatal. (The Java
/// `readOk` flag is dead code — always true — and is not mirrored.)
fn read_logical_part(scanner: &mut Scanner) -> Option<LogicalPart> {
    let Token::Str(part_name) = scanner.next_token() else {
        // Java warns "string expected"
        return None;
    };
    let mut part_pins: Vec<PartPin> = Vec::new();
    let mut prev_token = Token::Str(part_name.clone());
    let mut next_token = scanner.next_token();
    loop {
        if matches!(next_token, Token::Eof | Token::Error(_)) {
            // Java warns "unexpected end of file"
            return None;
        }
        if next_token == Token::Close {
            break;
        }
        if prev_token == Token::Open {
            match next_token {
                Token::Keyword(Keyword::Pin) => {
                    let part_pin = read_part_pin(scanner)?;
                    part_pins.push(part_pin);
                }
                _ => {
                    skip_scope(scanner);
                }
            }
        }
        prev_token = next_token;
        next_token = scanner.next_token();
    }
    Some(LogicalPart {
        name: part_name.to_string(),
        part_pins,
    })
}

/// Java `PartLibrary.readPartPin` (`PartLibrary.java:239-310`): five
/// fields (name, int, name, int, name, int — the second integer is the
/// unused component number), each NAME forced, then subgate tokens DRAINED
/// to the closing bracket.
fn read_part_pin(scanner: &mut Scanner) -> Option<PartPin> {
    scanner.set_lexical_state(LexicalState::Name);
    let Token::Str(pin_name) = scanner.next_token() else {
        // Java warns "string expected"
        return None;
    };
    let Token::Int(_) = scanner.next_token() else {
        // Java warns "integer expected"
        return None;
    };
    scanner.set_lexical_state(LexicalState::Name);
    let Token::Str(gate_name) = scanner.next_token() else {
        return None;
    };
    let Token::Int(gate_swap_code) = scanner.next_token() else {
        return None;
    };
    scanner.set_lexical_state(LexicalState::Name);
    let Token::Str(gate_pin_name) = scanner.next_token() else {
        return None;
    };
    let Token::Int(gate_pin_swap_code) = scanner.next_token() else {
        return None;
    };
    // Overread subgates: do-while to CLOSE or EOF (Java tolerates EOF here
    // — the pin is still returned). An Error token is NOT value-less: the
    // Java scanner throws NumberFormatException at LEX time and the
    // exception escapes every IOException-only catch — parse death
    // (jar /tmp/epic-t8-errorpin.jsh, output /tmp/epic-t8-errorpin.out
    // t8-errorpin1: `(pin 1 0 G1 0 GP1 0 99999999999)` dies with an
    // uncaught NumberFormatException before readBoard returns).
    loop {
        let next_token = scanner.next_token();
        if matches!(next_token, Token::Error(_)) {
            return None;
        }
        if matches!(next_token, Token::Close | Token::Eof) {
            break;
        }
    }
    Some(PartPin {
        pin_name: pin_name.to_string(),
        gate_name: gate_name.to_string(),
        gate_swap_code,
        gate_pin_name: gate_pin_name.to_string(),
        gate_pin_swap_code,
    })
}

/// One `(image ...)` scope as parsed (pre-conversion): the Java
/// `parser/Package` shape returned by `Package.readScope`.
struct ParserImage {
    name: String,
    pin_infos: Vec<PinInfo>,
    outlines: Vec<Shape>,
    keepouts: Vec<AreaScopeResult>,
    via_keepouts: Vec<AreaScopeResult>,
    place_keepouts: Vec<AreaScopeResult>,
    is_front: bool,
}

/// Java `Package.PinInfo` (`Package.java:404-424`).
struct PinInfo {
    padstack_name: String,
    pin_name: String,
    rel_coor: [f64; 2],
    rotation: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scope::placement::read_placement_scope;
    use crate::ses_board::SesBoard;
    use epic_geometry::regular_tile_shape::RegularTileShape;

    /// The two-layer parse state of the probe fixtures
    /// (`(layer F.Cu/B.Cu (type signal))`, `(resolution um 10)` -> scale
    /// 10, base 0).
    fn state_with_layers() -> ParseState {
        ParseState {
            layer_structure: Some(LayerStructure::new(vec![
                Layer::new("F.Cu", 0, true),
                Layer::new("B.Cu", 1, true),
            ])),
            coordinate_transform: Some(CoordinateTransform::new(10.0, 0.0, 0.0)),
            ..ParseState::default()
        }
    }

    /// The IntBox bounds of a slot shape, panicking on any other flavor.
    fn slot_box(shape: &Option<BoardShape>) -> (i32, i32, i32, i32) {
        match shape {
            Some(BoardShape::Tile(TileShape::RegularTileShape(RegularTileShape::IntBox(b)))) => {
                (b.ll.x, b.ll.y, b.ur.x, b.ur.y)
            }
            other => panic!("expected IntBox tile slot, got {other:?}"),
        }
    }

    fn slot_circle(shape: &Option<BoardShape>) -> (i32, i32, i32) {
        match shape {
            Some(BoardShape::Circle(circle)) => (circle.center.x, circle.center.y, circle.radius),
            other => panic!("expected Circle slot, got {other:?}"),
        }
    }

    /// Jar `/tmp/epic-t7-pad.dsn` -> `/tmp/epic-t7-probe.out` t7-pad: the
    /// padstack battery. FILLALL pins the fill-vs-assign difference (the
    /// signal rectangle fills BOTH slots, the later F.Cu rectangle
    /// overwrites slot 0 only), HULL the convex-hull route (a concave
    /// pentagon whose hull is the full rectangle), LINE the
    /// dim<2 workaround (offset(1) -> the max-int Simplex the jar
    /// printed), DEG1 the null-slot outcome (offset did not help), ATTACHON
    /// the drillable flag on a single-layer assign, DUPL/dupl.1 the
    /// `.N`-stripped dedup skip, EMPTYSH the warn-only skip. The three jar
    /// FRLogger.warn lines fire WITHOUT touching the parity warnings
    /// (WARN_COUNT 0) — log-only, never routed to `state.warnings`.
    #[test]
    fn t7_pad_padstack_battery() {
        let input = b"\
(padstack FILLALL (shape (rectangle signal -100 -100 100 100)) \
(shape (rectangle F.Cu -50 -50 50 50)) (attach off)) \
(padstack HULL (shape (polygon B.Cu 0 -300 -200 300 -200 300 200 0 100 -300 200)) (attach off)) \
(padstack CIRC (shape (circle signal 800)) (attach off)) \
(padstack LINE (shape (polygon signal 0 0 0 500 0)) (attach off)) \
(padstack DEG1 (shape (polygon signal 0 500)) (attach off)) \
(padstack ATTACHON (shape (circle F.Cu 300)) (attach on)) \
(padstack DUPL (shape (circle signal 300)) (attach off)) \
(padstack dupl.1 (shape (circle signal 300)) (attach off)) \
(padstack EMPTYSH (attach off)))";
        let mut state = state_with_layers();
        let mut board = SesBoard::new();
        let mut scanner = Scanner::new(input);
        assert!(read_library_scope(&mut scanner, &mut state, &mut board));
        assert_eq!(board.padstacks.len(), 7, "dupl.1 deduped, EMPTYSH skipped");
        assert!(board.packages.is_empty(), "no images here");

        let names: Vec<&str> = board.padstacks.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "FILLALL", "HULL", "CIRC", "LINE", "DEG1", "ATTACHON", "DUPL"
            ]
        );

        // FILLALL: the signal rect (x10 -> +-1000) filled both slots; the
        // F.Cu rect (x10 -> +-500) overwrote ONLY slot 0.
        let fillall = &board.padstacks[0];
        assert_eq!(slot_box(&fillall.shapes[0]), (-500, -500, 500, 500));
        assert_eq!(slot_box(&fillall.shapes[1]), (-1000, -1000, 1000, 1000));
        assert!(!fillall.drillable);

        // HULL: concave pentagon -> hull rectangle on B.Cu (slot 1) only.
        let hull = &board.padstacks[1];
        assert!(hull.shapes[0].is_none());
        assert_eq!(slot_box(&hull.shapes[1]), (-3000, -2000, 3000, 2000));

        // CIRC: circle 800 -> radius 4000 (the halved-radius transform)
        // filled into both slots.
        let circ = &board.padstacks[2];
        assert_eq!(slot_circle(&circ.shapes[0]), (0, 0, 4000));
        assert_eq!(slot_circle(&circ.shapes[1]), (0, 0, 4000));

        // LINE: the 2-point polygon -> segment Simplex (dim 1) -> the
        // offset(1) workaround stores a 2-dimensional Simplex (jar: the
        // max-int Simplex on both slots).
        let line = &board.padstacks[3];
        for slot in &line.shapes {
            match slot {
                Some(BoardShape::Tile(tile @ TileShape::Simplex(_))) => {
                    assert_eq!(tile.dimension(), 2, "offset(1) made it an area");
                }
                other => panic!("expected Simplex slot, got {other:?}"),
            }
        }

        // DEG1: 1-point polygon -> dim 0 before AND after offset(1) ->
        // BOTH slots null (jar PSLOT 5 L0/L1 null).
        let deg1 = &board.padstacks[4];
        assert!(deg1.shapes[0].is_none() && deg1.shapes[1].is_none());

        // ATTACHON: drillable=true, F.Cu-only circle (r 1500), slot 1 null.
        let attachon = &board.padstacks[5];
        assert!(attachon.drillable);
        assert_eq!(slot_circle(&attachon.shapes[0]), (0, 0, 1500));
        assert!(attachon.shapes[1].is_none());

        // DUPL: plain signal circle, not drillable.
        assert!(!board.padstacks[6].drillable);
        assert_eq!(slot_circle(&board.padstacks[6].shapes[0]), (0, 0, 1500));
    }

    /// Jar `/tmp/epic-t7-image.dsn` -> `/tmp/epic-t7-probe.out` t7-image:
    /// the full image read — numeric padstack names, the `.N` pin-padstack
    /// alias, optional pre- and post-name `(rotate ...)` scopes, the
    /// half-up pin rounding (300.55 -> 3006, 0.55 -> 6), the four outline
    /// flavors (closed / open / explicitly-closed path / the PolylinePath
    /// null slot that still carries width and is_closed), and the per-list
    /// keepout renaming (K1 + unnamed -> keepout_1/keepout_2, while the
    /// all-named place-keepout list keeps VK1).
    #[test]
    fn t7_image_full_library() {
        let input = b"\
(padstack P1 (shape (circle signal 400)) (attach off)) \
(padstack 1 (shape (circle signal 400)) (attach off)) \
(image IMG \
(pin P1.1 1 0 0) \
(pin P1 (rotate 1.5e2) 2 300 700) \
(pin P1 3 300.55 0.55) \
(pin P1 4 0 0 (rotate .5)) \
(pin 1 7 500 500) \
(outline (path signal 10 0 0 1000 0 1000 1000 0 1000)) \
(outline (path signal 10 0 0 1000 0 1000 1000)) \
(outline (path signal 10 0 0 1000 0 1000 1000 0 1000 0 0)) \
(outline (polyline_path signal 10 0 0 100 0 100 100)) \
(keepout K1 (rectangle F.Cu 0 0 500 500)) \
(keepout (rectangle B.Cu 0 0 500 500)) \
(via_keepout (circle F.Cu 200 100 100)) \
(place_keepout VK1 (rectangle F.Cu 10 10 100 100)) \
(side front)))";
        let mut state = state_with_layers();
        let mut board = SesBoard::new();
        let mut scanner = Scanner::new(input);
        assert!(read_library_scope(&mut scanner, &mut state, &mut board));
        assert_eq!(board.padstacks.len(), 2, "P1 and the numeric-name padstack");
        assert_eq!(board.padstacks[1].name, "1");

        assert_eq!(board.packages.len(), 1);
        let image = &board.packages[0];
        assert_eq!(image.name, "IMG");
        assert!(image.is_front);

        // Pins, in file order (jar PIN 0..4): the P1.1 alias resolves to
        // padstack 1, pin "7" resolves to the numeric padstack 2, the
        // rotations 150/0.5 survive, and 300.55/0.55 round half-up to
        // 3006/6 at scale 10.
        let plain: Vec<(&str, i32, i32, i32, f64)> = image
            .pins
            .iter()
            .map(|p| {
                (
                    p.name.as_str(),
                    p.padstack_no,
                    p.rel_location.x,
                    p.rel_location.y,
                    p.rotation,
                )
            })
            .collect();
        assert_eq!(
            plain,
            vec![
                ("1", 1, 0, 0, 0.0),
                ("2", 1, 3000, 7000, 150.0),
                ("3", 1, 3006, 6, 0.0),
                ("4", 1, 0, 0, 0.5),
                ("7", 2, 5000, 5000, 0.0),
            ]
        );

        // Outlines: 4 entries; the PolylinePath stub stores a null shape
        // but STILL sets width 10.0 and is_closed=false (jar OUTLINE 3).
        assert_eq!(image.outline.len(), 4);
        let closed: Vec<bool> = image.outline.iter().map(|o| o.is_closed).collect();
        assert_eq!(closed, vec![false, false, true, false]);
        assert!(
            image.outline.iter().all(|o| o.width == 10.0),
            "every outline width is set, even the null one"
        );
        assert!(image.outline[0].shape.is_some());
        assert!(image.outline[1].shape.is_some());
        assert!(image.outline[2].shape.is_some());
        assert!(image.outline[3].shape.is_none(), "PolylinePath stub slot");

        // Keepout renaming is per-list: keepout_1/keepout_2 (K1 is
        // OVERWRITTEN), via_keepout_1, and VK1 kept verbatim.
        let keepout_names: Vec<&str> = image.keepouts.iter().map(|k| k.name.as_str()).collect();
        assert_eq!(keepout_names, vec!["keepout_1", "keepout_2"]);
        assert_eq!(image.keepouts[0].layer_no, 0);
        assert_eq!(image.keepouts[1].layer_no, 1);
        // keepout_1's border: the F.Cu rectangle 0 0 500 500 at scale 10.
        assert_eq!(
            slot_box(&image.keepouts[0].area.as_ref().map(|a| a.border.clone())),
            (0, 0, 5000, 5000)
        );
        assert_eq!(
            image
                .via_keepouts
                .iter()
                .map(|k| k.name.as_str())
                .collect::<Vec<_>>(),
            vec!["via_keepout_1"]
        );
        assert_eq!(
            image
                .place_keepouts
                .iter()
                .map(|k| k.name.as_str())
                .collect::<Vec<_>>(),
            vec!["VK1"],
            "the all-named list is untouched (per-list discriminator)"
        );
    }

    /// Jar `/tmp/epic-t7-image-bad.dsn`: a pin whose padstack resolves to
    /// nothing is FATAL (`Library.read_scope: board padstack 'MISSING'
    /// (cleaned: 'MISSING') not found` -> ParseError). The padstacks added
    /// earlier in the loop STAY (Java adds them immediately); no package
    /// lands.
    #[test]
    fn image_missing_padstack_is_fatal() {
        let input = b"\
(padstack P1 (shape (circle signal 400)) (attach off)) \
(image IMG (pin MISSING 9 0 0)))";
        let mut state = state_with_layers();
        let mut board = SesBoard::new();
        let mut scanner = Scanner::new(input);
        assert!(!read_library_scope(&mut scanner, &mut state, &mut board));
        assert_eq!(board.padstacks.len(), 1);
        assert!(board.packages.is_empty());
    }

    /// The dedup loop over real image reads (jar
    /// `/tmp/epic-t4c-images.out`): an identical-pin duplicate is
    /// SKIPPED (also across a case-variant name), a different-pin
    /// duplicate is inserted as `PAD::1`.
    #[test]
    fn image_dedup_insert_or_skip() {
        let input = b"\
(padstack P (shape (circle signal 400)) (attach off)) \
(image PAD (pin P 1 0 0)) \
(image pad (pin P 1 0 0)) \
(image PAD (pin P 1 0 0) (pin P 2 1000 0)))";
        let mut state = state_with_layers();
        let mut board = SesBoard::new();
        let mut scanner = Scanner::new(input);
        assert!(read_library_scope(&mut scanner, &mut state, &mut board));
        let names: Vec<&str> = board.packages.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["PAD", "PAD::1"]);
        assert_eq!(board.packages[0].pins.len(), 1);
        assert_eq!(board.packages[1].pins.len(), 2);
    }

    /// Jar `/tmp/epic-t7-part.dsn` -> `/tmp/epic-t7-probe.out` t7-part:
    /// the mapping's component set iterates SORTED — `[ACOMP, ZCOMP]`
    /// against the file order `ZCOMP ACOMP` (a file-order-preserving port
    /// diverges; BTreeSet over UTF-8 bytes mirrors the Java TreeSet for
    /// this ASCII input) — and the part pin carries all five fields
    /// (PARTPIN 0 pin=1 gate=G1 gswap=0 gpin=GP1 gpswap=0).
    #[test]
    fn t7_part_library_mapping_and_part_pin() {
        let input = b"\
(logical_part_mapping LP1 (comp ZCOMP ACOMP)) \
(logical_part LP1 (pin 1 0 G1 0 GP1 0)))";
        let mut mappings = Vec::new();
        let mut parts = Vec::new();
        let mut scanner = Scanner::new(input);
        assert!(read_part_library_scope(
            &mut scanner,
            &mut mappings,
            &mut parts
        ));
        assert_eq!(mappings.len(), 1);
        assert_eq!(mappings[0].name, "LP1");
        let components: Vec<&str> = mappings[0].components.iter().map(String::as_str).collect();
        assert_eq!(components, vec!["ACOMP", "ZCOMP"], "sorted, not file order");

        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].name, "LP1");
        assert_eq!(parts[0].part_pins.len(), 1);
        let pin = &parts[0].part_pins[0];
        assert_eq!(pin.pin_name, "1");
        assert_eq!(pin.gate_name, "G1");
        assert_eq!(pin.gate_swap_code, 0);
        assert_eq!(pin.gate_pin_name, "GP1");
        assert_eq!(pin.gate_pin_swap_code, 0);
    }

    /// T46 stress test on the real tier-A fixture bytes
    /// (`fixtures/Issue690-sonde_xilinx.dsn`): 25 `(place ...)` rows
    /// across 10 component groups; R14 pins the `(PN 100)` form with its
    /// exact coordinates and rotation. The scanner is positioned the way
    /// the pcb dispatcher leaves it: right after the `placement` keyword.
    #[test]
    fn t46_real_fixture_placement_scope() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../fixtures/Issue690-sonde_xilinx.dsn"
        );
        let bytes = std::fs::read(path).expect("tier-A fixture readable");
        let keyword = b"(placement";
        let offset = bytes
            .windows(keyword.len())
            .position(|w| w == keyword)
            .expect("fixture has a placement scope");
        let mut state = ParseState::default();
        let mut board = SesBoard::new();
        let mut scanner = Scanner::new(&bytes[offset + keyword.len()..]);
        assert!(read_placement_scope(&mut scanner, &mut state, &mut board));
        assert_eq!(board.placements.len(), 25, "every (place ...) row landed");
        assert_eq!(state.placement_list.len(), 10, "component groups");

        let r14 = board
            .placements
            .iter()
            .find(|p| p.location.name == "R14")
            .expect("R14 present");
        // The fixture's quoted component group name (contains ':').
        assert_eq!(
            r14.lib_name,
            "sonde-xilinx:R_Axial_DIN0207_L6.3mm_D2.5mm_P10.16mm_Horizontal"
        );
        assert_eq!(r14.location.coor, Some([162560.0, -88900.0]));
        assert!(r14.location.is_front);
        assert_eq!(r14.location.rotation, 180.0);
        assert_eq!(r14.location.part_number.as_deref(), Some("100"));
        assert!(!r14.location.position_fixed, "no (lock_type position) here");
    }

    /// Jar `/tmp/epic-t8-errorpin.out` t8-errorpin1: an overflow integer in
    /// the subgate DRAIN of a `(pin ...)` logical-part entry throws
    /// NumberFormatException at LEX time — the drain is NOT value-less;
    /// parse death. The same pin without the extra token parses (the
    /// t7_part pin).
    #[test]
    fn part_pin_drain_error_token_is_fatal() {
        let mut mappings = Vec::new();
        let mut parts = Vec::new();
        let mut scanner = Scanner::new(b"(logical_part LP1 (pin 1 0 G1 0 GP1 0 99999999999)))");
        assert!(!read_part_library_scope(
            &mut scanner,
            &mut mappings,
            &mut parts
        ));
        assert!(parts.is_empty());
    }

    /// Jar `/tmp/epic-t8-errorpin.out` t8-errorpin2: `(side 99999999999)`
    /// inside an image dies (the side token is LEXED before the
    /// front-fallback applies). Controls: a garbage token that lexes
    /// cleanly keeps the front fallback, and `(side back)` flips the side.
    #[test]
    fn image_side_error_token_is_fatal() {
        let common = b"\
(padstack P1 (shape (circle F.Cu 400)) (attach off)) \
(image PA (pin P1 1 0 0) ";

        let mut state = state_with_layers();
        let mut board = SesBoard::new();
        let mut input = common.to_vec();
        input.extend_from_slice(b"(side 99999999999)))");
        let mut scanner = Scanner::new(&input);
        assert!(!read_library_scope(&mut scanner, &mut state, &mut board));
        assert!(board.packages.is_empty());

        let mut state = state_with_layers();
        let mut board = SesBoard::new();
        let mut input = common.to_vec();
        input.extend_from_slice(b"(side garbage)))");
        let mut scanner = Scanner::new(&input);
        assert!(read_library_scope(&mut scanner, &mut state, &mut board));
        assert!(board.packages[0].is_front, "warn-only front fallback");

        let mut state = state_with_layers();
        let mut board = SesBoard::new();
        let mut input = common.to_vec();
        input.extend_from_slice(b"(side back)))");
        let mut scanner = Scanner::new(&input);
        assert!(read_library_scope(&mut scanner, &mut state, &mut board));
        assert!(!board.packages[0].is_front);
    }

    /// Jar `/tmp/epic-t8-partb.out` t8-pcbadlayer + `/tmp/epic-t8-partb2.out`
    /// (REFUTATION of the assumed fatal): an unknown layer inside a padstack
    /// rectangle warns log-only ("Shape.read_circle_scope: layer with name
    /// 'Z.TOP' not found" — the upstream copy-paste warn string) and falls
    /// back to the SIGNAL layer (`Shape.java:290-292`), so the shape
    /// FILL-ALLs and the padstack IS added: jar `PADSTACK 1 name=BAD
    /// attach=false from=0 to=1` with BOTH slots `IntBox -1000 -1000 1000
    /// 1000` (scale 10). The `shapeLayer < 0` fatal in read_padstack_scope
    /// is therefore dead via DSN input (unknown layers never yield a
    /// shape; PolylinePath dies at the transform stub first) — kept as
    /// Java-verbatim dead defense, not pinned as reachable.
    #[test]
    fn padstack_unknown_layer_falls_back_to_signal() {
        let input = b"(padstack BAD (shape (rectangle Z.TOP -100 -100 100 100)) (attach off)))";
        let mut state = state_with_layers();
        let mut board = SesBoard::new();
        let mut scanner = Scanner::new(input);
        assert!(read_library_scope(&mut scanner, &mut state, &mut board));
        assert_eq!(board.padstacks.len(), 1, "jar PADSTACK_COUNT 1");
        assert_eq!(board.padstacks[0].name, "BAD");
        assert!(!board.padstacks[0].drillable);
        assert_eq!(
            slot_box(&board.padstacks[0].shapes[0]),
            (-1000, -1000, 1000, 1000)
        );
        assert_eq!(
            slot_box(&board.padstacks[0].shapes[1]),
            (-1000, -1000, 1000, 1000)
        );
    }

    /// Jar `/tmp/epic-t8-partb.out` t8-pcpath: `(shape (polyline_path ...))`
    /// inside a padstack — the PolylinePath transform stub returns null and
    /// Java dereferences it at the `splitToConvex` call (uncaught
    /// NullPointerException) — parse death.
    #[test]
    fn padstack_polyline_path_shape_is_fatal() {
        let input = b"\
(padstack PATH (shape (polyline_path signal 10 0 0 100 0 100 100)) (attach off)))";
        let mut state = state_with_layers();
        let mut board = SesBoard::new();
        let mut scanner = Scanner::new(input);
        assert!(!read_library_scope(&mut scanner, &mut state, &mut board));
        assert!(board.padstacks.is_empty());
    }

    /// `arePackagePinsIdentical` discriminators, flow-level (the dedup
    /// loop of `Library.java:412-449`): a rotation delta of 0.0005 (within
    /// the 0.001 tolerance) DEDUPES while 0.002 inserts as `PAD::1`, and
    /// pin names compare CASE-SENSITIVE (`String.equals`): image `pad`
    /// with pin `a` against stored pin `A` inserts as `pad::1` instead of
    /// deduping. Jar-anchored on the Task 7 dedup pins
    /// (`/tmp/epic-t4c-images.out`); the tolerance values are the Java
    /// `> 0.001` bound discriminated at 0.0005/0.002.
    #[test]
    fn package_pins_identical_tolerance_and_case() {
        let input = b"\
(padstack P (shape (circle signal 400)) (attach off)) \
(image PAD (pin P A 0 0)) \
(image PAD (pin P A 0 0)) \
(image PAD (pin P (rotate 0.0005) A 0 0)) \
(image PAD (pin P (rotate 0.002) A 0 0)) \
(image pad (pin P a 0 0)))";
        let mut state = state_with_layers();
        let mut board = SesBoard::new();
        let mut scanner = Scanner::new(input);
        assert!(read_library_scope(&mut scanner, &mut state, &mut board));
        let names: Vec<&str> = board.packages.iter().map(|p| p.name.as_str()).collect();
        // The third insert retries "pad::1", but `Packages.get` matches
        // package names CASE-INSENSITIVELY, so it hits the stored "PAD::1"
        // (different pins) and lands as "pad::2" — the case-variant PIN
        // name is still the discriminator: a case-insensitive pin compare
        // would have deduped image 5 into PAD (2 packages, not 3).
        assert_eq!(
            names,
            vec!["PAD", "PAD::1", "pad::2"],
            "dup deduped; 0.0005 deduped; 0.002 renamed; case-variant pin renamed"
        );
        assert!((board.packages[1].pins[0].rotation - 0.002).abs() < 1e-12);
        assert_eq!(board.packages[2].pins[0].name, "a");
    }
}
