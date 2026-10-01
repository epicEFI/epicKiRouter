//! Canonical geometry digest for the DSN parse-parity gate (M1b Task 4;
//! the formats below are the DOC-OF-RECORD the Task 11 Java oracle
//! implements — refinements land here first, in this crate's commits).
//!
//! ## Canonical geometry text
//!
//! One line per Trace/Via/Keepout/ConductionArea item in DESCENDING board
//! id order (T39: Java `board.getItems()` enumerates a descending-id view
//! — jar session `/tmp/epic-t4-ids.jsh`, output `/tmp/epic-t4-ids.out`,
//! enumerates id 8 first and id 1 last). Pins, component outlines and the
//! board outline CONSUME ids but emit NO line — id gaps are expected and
//! load-bearing (they prove the Java id assignment is being mirrored; see
//! the `epic_dsn::sink` module docs).
//!
//! ```text
//! T <id> <layer> <halfwidth> <fixed> <x0> <y0> <x1> <y1> ...
//! V <id> <padstack> <x> <y> <fixed>
//! K <id> <layer> <class> <fixed> <shape-encoding>
//! A <id> <layer> <class> <fixed> <net-count> <nets...> <shape-encoding>
//! ```
//!
//! - All numbers are decimal i64 in BOARD space (parse output is
//!   already rounded; nothing re-rounds here). The shape of a `K` line is
//!   the item's ABSOLUTE area — Java `ObstacleArea.getArea()`
//!   (`ObstacleArea.java:119-144`), i.e. the placement transform applied
//!   (T49) — while structure-scope areas transform identically.
//! - `<fixed>` is one of `unfixed`, `shove_fixed`, `user_fixed`,
//!   `system_fixed` (the `FixedState` ordinal order).
//! - `<padstack>` is the RAW registry name of the resolved padstack,
//!   through [`quote_java_identifier`] with the board's string quote and
//!   the SES reserved set (`SesWriter.java:62`). An out-of-range via
//!   padstack number (unreachable from a correct parse — Java
//!   dereferences the Padstack object at `Wiring.java:706`) emits
//!   `<unresolved <no>>` in that field instead of panicking, so one
//!   reader bug surfaces as a digest mismatch on its fixture rather than
//!   killing a whole soak batch.
//! - `<layer>`/`<class>` are the 0-based layer number and the clearance
//!   class index.
//! - `<shape-encoding>` kinds:
//!   - `box <llx> <lly> <urx> <ury>` (IntBox corners),
//!   - `octagon <left_x> <bottom_y> <right_x> <top_y> <upper_left_diag>
//!     <lower_right_diag> <lower_left_diag> <upper_right_diag>` (the Java
//!     `IntOctagon` field order),
//!   - `simplex <n> <x0> <y0> ...` (corner approximations, `Math.round`
//!     to int; only `n = 0` is reachable from a DSN parse — the only
//!     producer is `Polygon.transformToBoardRel` with under 2
//!     coordinates, jar T4a/B5),
//!   - `polygon <n> <x0> <y0> ...` (PolygonShape border corners),
//!   - `circle <cx> <cy> <r>`,
//!   - `(window)` holes append ` win <shape-encoding>` per hole in file
//!     order.
//! - Kind blind spot: the `K` line erases the keepout kind — keepout,
//!   via_keepout and place_keepout (`KeepoutKindIr`) all render as `K` —
//!   so a kind-swap regression is invisible to the digest. Changing the
//!   line format requires updating BOTH this file and
//!   `rust/harness/oracle/DsnParseOracle.java` together (doc-of-record
//!   format).
//! - A zero-corner trace would encode as `T <id> <layer> <halfwidth>
//!   <fixed>` with no coordinate pairs (the encoder does not special-case
//!   or panic), but it is UNREACHABLE from a parse: `readWire` skips any
//!   wire with fewer than 2 corners or with all corners equal (warning,
//!   then drop, `Wiring.java:518-545`), so every parse-produced trace
//!   carries at least one pair.
//! - Every line ends `\n` INCLUDING the last; an item-free board yields
//!   the empty text.
//!
//! `geometry_sha256` is the lowercase SHA-256 hex over the UTF-8 text.
//!
//! ## The Board-side twin (T13)
//!
//! The Java oracle digests `success.board()` — the LIVE BOARD after the
//! read, i.e. AFTER the in-wiring-scope `normalizeAllTraces()`
//! (`Wiring.java:343-353`) has run. The SesBoard-side
//! `canonical_geometry_text` below digests the parser IR — PRE — and
//! matches it only on fixtures where normalize is a no-op (1,331 of
//! 1,332; dsn-0151 is the exception the M2 divergence ledger carried).
//! T13 moves the Rust `stats` + `geometry_sha256` to the POST-normalize
//! [`epic_board::board::Board`]: the digest pipeline builds the Board,
//! fills the trees the read-path way
//! (`SearchTreeManager::insert_items_creation_order` — ASCENDING), runs
//! `epic_board::normalize_all::normalize_all_traces`, and digests
//! through [`canonical_geometry_text_board`] — the same line format,
//! over the Board's LIVE items (`entry.on_the_board`; Java
//! `board.getItems()` is live-only, which is why the oracle's `items`
//! stat reads 2 on dsn-0151 while the Rust arena keeps every parsed
//! id). The SesBoard twin stays: the IR remains the source for every
//! other golden field, and the two twins must agree byte-for-byte on a
//! no-op-normalize board — the 1,332 compare asserts it empirically.
//!
//! ## Warning normalization (D12)
//!
//! [`normalize_warning_digits`] replaces every `[0-9]+` run with `#` —
//! the exact Java-side oracle rule (`replaceAll("[0-9]+", "#")`); raw
//! warning strings are captured in goldens but compared only in
//! normalized form.

use epic_board::board::Board;
use epic_board::items::ItemData;
// Test-only since T13: the SesBoard-twin renderer below is the pins'
// PRE-normalize comparison source; the pipeline digests the Board twin.
#[cfg(test)]
use epic_dsn::ses_board::{ItemIr, SesBoard};
use epic_dsn::shape::BoardShape;
use epic_dsn::sink::AreaIr;
#[cfg(test)]
use epic_dsn::sink::FixedStateIr;
use epic_dsn::write_scope::quote_java_identifier;
use epic_geometry::float_point::FloatPoint;
use epic_geometry::point::Point;
use epic_geometry::polygon_shape::PolygonShape;
use epic_geometry::regular_tile_shape::RegularTileShape;
use epic_geometry::rounding::java_round;
use epic_geometry::tile_shape::TileShape;
use sha2::{Digest, Sha256};

/// D12: every `[0-9]+` run becomes `#` (Java `replaceAll("[0-9]+", "#")`).
pub fn normalize_warning_digits(warning: &str) -> String {
    let mut out = String::with_capacity(warning.len());
    let mut chars = warning.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() {
            while chars.peek().is_some_and(|c| c.is_ascii_digit()) {
                chars.next();
            }
            out.push('#');
        } else {
            out.push(c);
        }
    }
    out
}

/// `FixedState` token (the enum's ordinal order). TEST-ONLY since T13:
/// the only remaining caller is the SesBoard twin below (the compare
/// pipeline digests the Board twin's `board_fixed_token`).
#[cfg(test)]
fn fixed_token(fixed: FixedStateIr) -> &'static str {
    match fixed {
        FixedStateIr::Unfixed => "unfixed",
        FixedStateIr::ShoveFixed => "shove_fixed",
        FixedStateIr::UserFixed => "user_fixed",
        FixedStateIr::SystemFixed => "system_fixed",
    }
}

/// The `<shape-encoding>` of one border shape (module docs).
fn encode_shape(shape: &BoardShape) -> String {
    match shape {
        BoardShape::Tile(TileShape::RegularTileShape(RegularTileShape::IntBox(r#box))) => {
            format!(
                "box {} {} {} {}",
                r#box.ll.x, r#box.ll.y, r#box.ur.x, r#box.ur.y
            )
        }
        BoardShape::Tile(TileShape::RegularTileShape(RegularTileShape::IntOctagon(o))) => {
            format!(
                "octagon {} {} {} {} {} {} {} {}",
                o.left_x,
                o.bottom_y,
                o.right_x,
                o.top_y,
                o.upper_left_diagonal_x,
                o.lower_right_diagonal_x,
                o.lower_left_diagonal_x,
                o.upper_right_diagonal_x
            )
        }
        BoardShape::Tile(TileShape::Simplex(simplex)) => {
            format!(
                "simplex {}",
                encode_corner_points(&simplex.corner_approx_arr())
            )
        }
        BoardShape::PolygonShape(polygon) => {
            format!("polygon {}", encode_corner_polygon(polygon))
        }
        BoardShape::Circle(circle) => {
            format!(
                "circle {} {} {}",
                circle.center.x, circle.center.y, circle.radius
            )
        }
    }
}

/// `polygon <n> <2n>` over the border corners (parse-produced corners are
/// exact ints; `to_float` is the identity on them).
fn encode_corner_polygon(polygon: &PolygonShape) -> String {
    let corners: Vec<FloatPoint> = (0..polygon.border_line_count() as i32)
        .map(|i| polygon.corner(i).to_float())
        .collect();
    encode_corner_points(&corners)
}

/// `simplex|polygon <n> <2n>` body: corner count, then x/y pairs rounded
/// with `Math.round` (identity on int corners; the simplex corner
/// approximations are the only real rounding case).
fn encode_corner_points(corners: &[FloatPoint]) -> String {
    let mut out = format!("{}", corners.len());
    for corner in corners {
        out.push(' ');
        out.push_str(&java_round(corner.x).to_string());
        out.push(' ');
        out.push_str(&java_round(corner.y).to_string());
    }
    out
}

/// The border encoding plus ` win ...` per hole in file order.
fn encode_area(area: &AreaIr) -> String {
    let mut out = encode_shape(&area.border);
    for hole in &area.holes {
        out.push_str(" win ");
        out.push_str(&encode_shape(hole));
    }
    out
}

/// The canonical geometry text over the SesBoard IR (module docs):
/// descending board id, one line per trace/via/keepout/conduction area,
/// `\n`-terminated including the last line. TEST-ONLY since T13 — the
/// PRE-normalize twin the pins compare against (the digest pipeline and
/// every other consumer moved to [`canonical_geometry_text_board`]).
#[cfg(test)]
pub fn canonical_geometry_text(board: &SesBoard) -> String {
    let mut text = String::new();
    for item in board.items.iter().rev() {
        match item {
            ItemIr::Trace { id, trace } => {
                text.push_str(&format!(
                    "T {id} {} {} {}",
                    trace.layer_no,
                    trace.half_width,
                    fixed_token(trace.fixed)
                ));
                for corner in &trace.corners {
                    text.push_str(&format!(" {} {}", corner.x, corner.y));
                }
                text.push('\n');
            }
            ItemIr::Via { id, via } => {
                // An out-of-range via padstack number cannot come from a
                // correct parse (Java dereferences the Padstack object at
                // `Wiring.java:706`); emit a marked line instead of
                // panicking so one reader bug cannot kill a whole soak
                // batch (the divergence still shows up as a digest
                // mismatch on that fixture).
                let padstack: String = board
                    .padstack_name(via.padstack_no)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("<unresolved {}>", via.padstack_no));
                text.push_str(&format!(
                    "V {id} {} {} {} {}\n",
                    quote_java_identifier(&padstack, &board.metadata.string_quote),
                    via.location.x,
                    via.location.y,
                    fixed_token(via.fixed)
                ));
            }
            ItemIr::Keepout { id, keepout } => {
                // T49: the Java oracle reads `keepout.getArea()` — the
                // LAZY placement-transformed form (`ObstacleArea.java:
                // 119-144`); the port applies the same transform here so
                // component keepouts digest at their absolute geometry.
                let absolute = board.obstacle_absolute_area(
                    &keepout.area,
                    keepout.translation,
                    keepout.rotation,
                    keepout.side_changed,
                );
                text.push_str(&format!(
                    "K {id} {} {} {} {}\n",
                    keepout.layer_no,
                    keepout.clearance_class,
                    fixed_token(keepout.fixed),
                    encode_area(&absolute)
                ));
            }
            ItemIr::ConductionArea { id, area } => {
                text.push_str(&format!(
                    "A {id} {} {} {} {}",
                    area.layer_no,
                    area.clearance_class,
                    fixed_token(area.fixed),
                    area.nets.len()
                ));
                for net in &area.nets {
                    text.push(' ');
                    text.push_str(&net.to_string());
                }
                text.push(' ');
                text.push_str(&encode_area(&area.area));
                text.push('\n');
            }
            // id-consuming, line-free (module docs)
            ItemIr::Pin { .. } | ItemIr::ComponentOutline { .. } | ItemIr::BoardOutline { .. } => {}
        }
    }
    text
}

/// Lowercase SHA-256 hex over the UTF-8 canonical text. TEST-ONLY since
/// T13 (the SesBoard twin's hash — see [`canonical_geometry_text`]).
#[cfg(test)]
pub fn geometry_sha256(board: &SesBoard) -> String {
    sha256_hex(canonical_geometry_text(board).as_bytes())
}

/// epic-board's `BoardShape` -> the IR `BoardShape` the encoders take —
/// a mechanical variant mirror over SHARED geometry types (`TileShape`,
/// `PolygonShape`, `Circle` are `epic_geometry` types in both crates; no
/// coordinate or structural conversion happens).
fn board_shape_to_ir(shape: &epic_board::items::BoardShape) -> BoardShape {
    match shape {
        epic_board::items::BoardShape::Tile(tile) => BoardShape::Tile(tile.clone()),
        epic_board::items::BoardShape::PolygonShape(polygon) => {
            BoardShape::PolygonShape(polygon.clone())
        }
        epic_board::items::BoardShape::Circle(circle) => BoardShape::Circle(*circle),
    }
}

/// epic-board's `Area` -> the IR `AreaIr` (same border + holes order).
fn board_area_to_ir(area: &epic_board::items::Area) -> AreaIr {
    AreaIr {
        border: board_shape_to_ir(&area.border),
        holes: area.holes.iter().map(board_shape_to_ir).collect(),
    }
}

/// The `FixedState` token over the BOARD's enum (same ordinal order as
/// the IR enum — both mirror Java's `FixedState`).
fn board_fixed_token(fixed: epic_board::items::FixedState) -> &'static str {
    match fixed {
        epic_board::items::FixedState::Unfixed => "unfixed",
        epic_board::items::FixedState::ShoveFixed => "shove_fixed",
        epic_board::items::FixedState::UserFixed => "user_fixed",
        epic_board::items::FixedState::SystemFixed => "system_fixed",
    }
}

/// The Board-side twin of `canonical_geometry_text` — the POST-normalize
/// digest source (module docs, "The Board-side twin"). The same T/V/K/A
/// line format over the Board's LIVE items in descending-id order; a
/// trace's corners come off the stored `Polyline`, a keepout's shape is
/// the T49 ABSOLUTE area (`Board::obstacle_area` — Java
/// `ObstacleArea.getArea()`), a conduction area's shape is the STORED
/// area verbatim (identity placement at parse — the `conduction_area`
/// accessor docs), and a via's padstack name resolves through the board
/// library (`<unresolved <no>>` for the parse-unreachable out-of-range
/// number, same discipline as the SesBoard twin).
pub fn canonical_geometry_text_board(board: &Board, string_quote: &str) -> String {
    let mut text = String::new();
    for entry in board.iter_descending() {
        if !entry.on_the_board {
            continue; // Java getItems() is live-only
        }
        match &entry.data {
            ItemData::Trace {
                layer,
                half_width,
                lines,
            } => {
                text.push_str(&format!(
                    "T {} {} {} {}",
                    entry.id.get(),
                    layer,
                    half_width,
                    board_fixed_token(entry.fixed)
                ));
                for corner in lines.corners() {
                    match corner {
                        Point::Int(point) => {
                            text.push_str(&format!(" {} {}", point.x, point.y));
                        }
                        Point::Rational(_) => {
                            // Parse-produced corners are exact ints; the
                            // canon's rounding contract is Math.round on
                            // the float view (java_round), as the oracle's
                            // coordinateOut does.
                            let float = corner.to_float();
                            text.push_str(&format!(
                                " {} {}",
                                java_round(float.x),
                                java_round(float.y)
                            ));
                        }
                    }
                }
                text.push('\n');
            }
            ItemData::Via {
                center,
                padstack_no,
                ..
            } => {
                let padstack: String = board
                    .library()
                    .padstack(*padstack_no)
                    .map(|pad| pad.name.clone())
                    .unwrap_or_else(|| format!("<unresolved {}>", padstack_no));
                text.push_str(&format!(
                    "V {} {} {} {} {}\n",
                    entry.id.get(),
                    quote_java_identifier(&padstack, string_quote),
                    center.x,
                    center.y,
                    board_fixed_token(entry.fixed)
                ));
            }
            ItemData::ObstacleArea { layer, .. } => {
                // The T49 absolute area (`Board::obstacle_area`); a None
                // here would be a Board/entry invariant break — the item
                // passed the on-the-board + kind gate, so the area is
                // computable. expect() keeps the harness fail-loud.
                let absolute = board.obstacle_area(entry.id).expect("keepout area");
                text.push_str(&format!(
                    "K {} {} {} {} {}\n",
                    entry.id.get(),
                    layer,
                    entry.clearance_class,
                    board_fixed_token(entry.fixed),
                    encode_area(&board_area_to_ir(&absolute))
                ));
            }
            ItemData::ConductionArea { layer, area, .. } => {
                // Parse-time conduction areas are identity-placed; the
                // STORED area is the Java getArea() (accessor docs).
                text.push_str(&format!(
                    "A {} {} {} {}",
                    entry.id.get(),
                    layer,
                    entry.clearance_class,
                    board_fixed_token(entry.fixed)
                ));
                text.push_str(&format!(" {}", entry.nets.len()));
                for net in &entry.nets {
                    text.push(' ');
                    text.push_str(&net.to_string());
                }
                text.push(' ');
                text.push_str(&encode_area(&board_area_to_ir(area)));
                text.push('\n');
            }
            // id-consuming, line-free (module docs)
            ItemData::Pin { .. }
            | ItemData::ComponentOutline { .. }
            | ItemData::BoardOutline { .. }
            | ItemData::Other => {}
        }
    }
    text
}

/// Lowercase SHA-256 hex over the Board-side canonical text (T13).
pub fn geometry_sha256_board(board: &Board, string_quote: &str) -> String {
    sha256_hex(canonical_geometry_text_board(board, string_quote).as_bytes())
}

/// Lowercase SHA-256 hex over raw bytes — shared by the digest and the
/// SES golden capture (cross-check against the oracle's own digest).
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;
    use epic_dsn::coordinate_transform::CoordinateTransform;
    use epic_dsn::layer_structure::{Layer, LayerStructure};
    use epic_dsn::sink::{
        BoardSink, ComponentIr, ComponentOutlineIr, ConductionAreaIr, CreateBoardIr, KeepoutIr,
        KeepoutKindIr, MetadataIr, NetIr, PadstackIr, PinIr, TraceIr, ViaIr,
    };
    use epic_dsn::state::Unit;
    use epic_geometry::circle::Circle as BoardCircle;
    use epic_geometry::int_box::IntBox;
    use epic_geometry::int_point::IntPoint;
    use epic_geometry::point::Point;
    use epic_geometry::simplex::Simplex;

    /// Replays `/tmp/epic-t4-ids.dsn` (scale 10, base 0) through the
    /// sink in Java scope order (identical to the
    /// `ses_board::tests::spike_board` construction; kept local so this
    /// module's pins stay self-contained).
    fn spike_board() -> SesBoard {
        let mut board = SesBoard::new();
        board.set_metadata(MetadataIr {
            unit: Unit::Um,
            resolution: 10,
            string_quote: "\"".to_string(),
            ..MetadataIr::default()
        });
        board.create_board(CreateBoardIr {
            bounding_box: IntBox::new(IntPoint::new(0, 0), IntPoint::new(100_000, 100_000)),
            layer_structure: LayerStructure::new(vec![
                Layer::new("F.Cu", 0, true),
                Layer::new("B.Cu", 1, true),
            ]),
            outline_shapes: Vec::new(),
            outline_clearance_class: Some("default".to_string()),
            rules: epic_dsn::sink::BoardRulesIr::new(2),
            transform: CoordinateTransform::new(10.0, 0.0, 0.0),
        });
        board.append_padstack(PadstackIr {
            name: "ViaPad_F".to_string(),
            shapes: Vec::new(),
            drillable: true,
            placed_absolute: false,
        });
        board.append_padstack(PadstackIr {
            name: "CirclePad_F_800_um".to_string(),
            shapes: Vec::new(),
            drillable: false,
            placed_absolute: false,
        });
        assert_eq!(
            board.append_net(NetIr {
                name: "PERFECT".to_string(),
                subnet_number: 1,
                contains_plane: false,
                net_class: 0,
            }),
            1
        );

        let corners: Vec<Point> = [
            (10_000, 10_000),
            (30_000, 10_000),
            (30_000, 30_000),
            (10_000, 30_000),
        ]
        .iter()
        .map(|&(x, y)| Point::Int(IntPoint::new(x, y)))
        .collect();
        board.insert_keepout(KeepoutIr {
            kind: KeepoutKindIr::Keepout,
            layer_no: 0,
            area: AreaIr::simple(BoardShape::PolygonShape(PolygonShape::new(&corners))),
            clearance_class: 1,
            fixed: FixedStateIr::SystemFixed,
            component_id: 0,
            translation: IntPoint::new(0, 0),
            rotation: 0.0,
            side_changed: false,
            name: None,
        });

        let u1 = board.insert_component(ComponentIr {
            name: "U1".to_string(),
            package_front: 1,
            package_back: 1,
            location: Some(IntPoint::new(20_000, 50_000)),
            rotation: 0.0,
            is_front: true,
            fixed: FixedStateIr::Unfixed,
            part_number: None,
            logical_part: None,
        });
        let u2 = board.insert_component(ComponentIr {
            name: "U2".to_string(),
            package_front: 1,
            package_back: 1,
            location: Some(IntPoint::new(80_000, 50_000)),
            rotation: 0.0,
            is_front: true,
            fixed: FixedStateIr::Unfixed,
            part_number: None,
            logical_part: None,
        });
        board.insert_pin(PinIr {
            component_id: u1,
            pin_index: 0,
            padstack_no: 2,
            nets: vec![1],
            clearance_class: 1,
            fixed: FixedStateIr::Unfixed,
        });
        board.insert_pin(PinIr {
            component_id: u2,
            pin_index: 0,
            padstack_no: 2,
            nets: vec![1],
            clearance_class: 1,
            fixed: FixedStateIr::Unfixed,
        });

        board.insert_trace(TraceIr {
            layer_no: 0,
            half_width: 625,
            corners: vec![IntPoint::new(20_000, 50_000), IntPoint::new(80_000, 50_000)],
            polyline: TraceIr::polyline_of_corners(&[
                IntPoint::new(20_000, 50_000),
                IntPoint::new(80_000, 50_000),
            ]),
            nets: vec![1],
            clearance_class: 1,
            fixed: FixedStateIr::UserFixed,
        });
        board.insert_trace(TraceIr {
            layer_no: 0,
            half_width: 625,
            corners: vec![IntPoint::new(20_000, 51_000), IntPoint::new(80_000, 51_000)],
            polyline: TraceIr::polyline_of_corners(&[
                IntPoint::new(20_000, 51_000),
                IntPoint::new(80_000, 51_000),
            ]),
            nets: vec![1],
            clearance_class: 1,
            fixed: FixedStateIr::SystemFixed,
        });
        board.insert_via(ViaIr {
            padstack_no: 1,
            location: IntPoint::new(50_000, 50_000),
            nets: vec![1],
            clearance_class: 1,
            fixed: FixedStateIr::Unfixed,
            attach_smd_allowed: false,
        });
        board.insert_conduction_area(ConductionAreaIr {
            layer_no: 0,
            area: AreaIr::simple(BoardShape::Tile(TileShape::RegularTileShape(
                RegularTileShape::IntBox(IntBox::new(
                    IntPoint::new(40_000, 40_000),
                    IntPoint::new(60_000, 60_000),
                )),
            ))),
            nets: vec![1],
            clearance_class: 1,
            fixed: FixedStateIr::Unfixed,
        });
        board
    }

    /// D12 digit normalization (Java `replaceAll("[0-9]+", "#")`).
    #[test]
    fn warning_digit_normalization() {
        assert_eq!(
            normalize_warning_digits("keepout skipped in DsnFile 123 at x 45"),
            "keepout skipped in DsnFile # at x #"
        );
        assert_eq!(normalize_warning_digits("no digits here"), "no digits here");
        assert_eq!(normalize_warning_digits("a1b2c"), "a#b#c");
    }

    /// THE spike pin: the exact canonical text of the
    /// `/tmp/epic-t4-ids.dsn` replay (module docs format; descending id
    /// order per the jar enumeration of `/tmp/epic-t4-ids.out`) and its
    /// SHA-256. The two `_`-carrying padstack name is quoted; pins and
    /// the board outline consume ids 1/3/4 without emitting lines.
    #[test]
    fn canonical_text_matches_spike_fixture() {
        let board = spike_board();
        let expected = "A 8 0 1 unfixed 1 1 box 40000 40000 60000 60000\n\
                        V 7 \"ViaPad_F\" 50000 50000 unfixed\n\
                        T 6 0 625 system_fixed 20000 51000 80000 51000\n\
                        T 5 0 625 user_fixed 20000 50000 80000 50000\n\
                        K 2 0 1 system_fixed polygon 4 10000 10000 30000 10000 30000 30000 10000 30000\n";
        assert_eq!(canonical_geometry_text(&board), expected);
    }

    /// The SHA-256 over the pinned text (verified independently with
    /// `printf '%s' <text> | sha256sum`; recomputed the same way whenever
    /// the format evolves).
    #[test]
    fn geometry_hash_matches_pinned_digest() {
        let board = spike_board();
        assert_eq!(
            geometry_sha256(&board),
            "5039a0ee94150698e55d334a0632c224c27eef5a3db8bf63e956c6937be56ad3"
        );
    }

    /// Order + gap discipline: 8 items but exactly 5 lines; the first
    /// line is the HIGHEST id (the conduction area, T39) and the last is
    /// the keepout (id 2 — the outline id 1 is line-free).
    #[test]
    fn descending_order_with_id_gaps() {
        let board = spike_board();
        let text = canonical_geometry_text(&board);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(board.items.len(), 8);
        assert_eq!(lines.len(), 5);
        assert!(lines[0].starts_with("A 8 "), "first line: {}", lines[0]);
        assert!(lines[1].starts_with("V 7 "), "second line: {}", lines[1]);
        assert!(lines[2].starts_with("T 6 "), "third line: {}", lines[2]);
        assert!(lines[3].starts_with("T 5 "), "fourth line: {}", lines[3]);
        assert!(lines[4].starts_with("K 2 "), "fifth line: {}", lines[4]);
        assert!(text.ends_with('\n'), "last line is newline-terminated");
    }

    /// An item-free board digests to the empty text and the well-known
    /// SHA-256 of the empty string.
    #[test]
    fn empty_board_digests_to_empty_text() {
        let board = SesBoard::new();
        assert_eq!(canonical_geometry_text(&board), "");
        assert_eq!(
            geometry_sha256(&board),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    /// THE T13 twin pin: the SesBoard twin ([`canonical_geometry_text`],
    /// the PRE-normalize IR view) and the Board twin
    /// ([`canonical_geometry_text_board`], the POST-normalize live-board
    /// view) must agree byte-for-byte on a no-op-normalize board (module
    /// docs, "The Board-side twin"; the 1,332-fixture compare asserts it
    /// empirically, this pin locks the spike). The spike board's parse
    /// traces are parallel (no combine), pin-anchored at distinct pins
    /// (no cycle), and the via is a mid-span contact (no crossing split),
    /// so `normalize_all_traces` is a no-op — asserted.
    #[test]
    fn board_twin_matches_ses_twin_on_a_noop_normalize_board() {
        let ses = spike_board();
        let mut board = epic_board::board::Board::from_ses_board(&ses);
        let mut manager = epic_board::tree_manager::SearchTreeManager::new();
        manager.insert_items_creation_order(&mut board);
        assert!(
            !epic_board::normalize_all::normalize_all_traces(&mut manager, &mut board),
            "the spike board must be a normalize no-op for the twin pin"
        );
        assert_eq!(
            canonical_geometry_text(&ses),
            canonical_geometry_text_board(&board, &ses.metadata.string_quote),
            "the twins must render the same geometry on a no-op board"
        );
        assert_eq!(
            geometry_sha256(&ses),
            geometry_sha256_board(&board, &ses.metadata.string_quote)
        );
    }

    /// The Point::Rational corner branch of [`canonical_geometry_text_board`]
    /// (:376-387, the only canon branch with no local pin before the T13
    /// quality round). No public RationalPoint constructor exists (the
    /// `pub(crate)` one mirrors Java's package-private constructor), so the
    /// Rational corner is built INDIRECTLY: a `Polyline::new` of three
    /// integer-endpoint lines whose CONSECUTIVE INTERSECTIONS are
    /// non-integer rationals (`corners()` is exact intersection math),
    /// stored verbatim through the public `insert_trace_without_cleaning`.
    /// Java renders such a corner in `DsnParseOracle.coordinateOut`
    /// (`:330-336`): `Math.round(corner.toFloat().x) + " " +
    /// Math.round(corner.toFloat().y)` — floor(x + 0.5) per coordinate,
    /// halves rounding UP (1.5 -> 2, 2.5 -> 3, 0.5 -> 1), which the pinned
    /// line exercises in all three positions.
    #[test]
    fn board_twin_renders_rational_corners_via_java_round() {
        use epic_board::items::FixedState;
        use epic_board::trace_ops::insert_trace_without_cleaning;
        use epic_geometry::line::Line;
        use epic_geometry::polyline::Polyline;

        let ses = spike_board();
        let mut board = epic_board::board::Board::from_ses_board(&ses);
        let mut manager = epic_board::tree_manager::SearchTreeManager::new();
        manager.insert_items_creation_order(&mut board);
        // L1 through (0,0)-(4,4): y = x; L2 through (3,0)-(0,3):
        // x + y = 3; L3 through (4,2)-(2,0): y = x - 2. Pairwise
        // non-parallel (Polyline::new's filters keep all three), so
        // corners() = [L1∩L2, L2∩L3] = [(3/2, 3/2), (5/2, 1/2)].
        let polyline = Polyline::new(vec![
            Line::from_int_coords(0, 0, 4, 4),
            Line::from_int_coords(3, 0, 0, 3),
            Line::from_int_coords(4, 2, 2, 0),
        ]);
        assert_eq!(polyline.lines.len(), 3, "all three lines survive");
        let id = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            polyline,
            0,
            0,
            &[1],
            0,
            FixedState::Unfixed,
        )
        .expect("the crossing-line trace inserts");
        // The corners REALLY are Rational — the branch under test is
        // exercised, not silently rendering coincidentally-integral
        // IntPoints.
        let stored = board.trace_polyline(id).expect("stored polyline");
        let corners = stored.corners();
        assert_eq!(corners.len(), 2);
        assert!(
            corners
                .iter()
                .all(|corner| matches!(corner, Point::Rational(_))),
            "both corners are Rational intersections"
        );
        // The pinned canon line: Math.round rendering of
        // (3/2, 3/2) -> "2 2" and (5/2, 1/2) -> "3 1" (layer 0,
        // half_width 0, unfixed).
        let text = canonical_geometry_text_board(&board, &ses.metadata.string_quote);
        let line = text
            .lines()
            .find(|line| line.starts_with(&format!("T {} ", id.get())))
            .expect("the inserted trace renders");
        assert_eq!(
            line,
            format!("T {} 0 0 unfixed 2 2 3 1", id.get()),
            "coordinateOut form: Math.round per float coordinate"
        );
    }

    /// Every shape-encoding branch (module docs): box, octagon (Java
    /// field order), empty simplex (`n = 0`, the only parse-reachable
    /// simplex), polygon, circle, and a `(window)` hole. Format-defined
    /// (no jar pin needed — the Task 11 goldens pin cross-language
    /// equality); this test pins the Rust side so format drift breaks
    /// HERE first.
    #[test]
    fn shape_encoding_coverage() {
        let mut board = SesBoard::new();
        board.create_board(CreateBoardIr {
            bounding_box: IntBox::new(IntPoint::new(0, 0), IntPoint::new(1000, 1000)),
            layer_structure: LayerStructure::new(vec![Layer::new("Top", 0, true)]),
            outline_shapes: Vec::new(),
            outline_clearance_class: Some("default".to_string()),
            rules: epic_dsn::sink::BoardRulesIr::new(1),
            transform: CoordinateTransform::new(1.0, 0.0, 0.0),
        });

        let octagon = epic_geometry::float_point::FloatPoint::bounding_octagon(&[
            FloatPoint::new(10.0, 20.0),
            FloatPoint::new(110.0, 120.0),
        ]);
        let with_window = AreaIr {
            border: BoardShape::Tile(TileShape::RegularTileShape(RegularTileShape::IntBox(
                IntBox::new(IntPoint::new(0, 0), IntPoint::new(100, 100)),
            ))),
            holes: vec![BoardShape::Circle(BoardCircle::new(
                IntPoint::new(50, 50),
                7,
            ))],
        };

        board.insert_conduction_area(ConductionAreaIr {
            layer_no: 0,
            area: with_window,
            nets: vec![2, 5],
            clearance_class: 1,
            fixed: FixedStateIr::UserFixed,
        });
        board.insert_keepout(KeepoutIr {
            kind: KeepoutKindIr::ViaKeepout,
            layer_no: 0,
            area: AreaIr::simple(BoardShape::Tile(TileShape::RegularTileShape(
                RegularTileShape::IntOctagon(octagon),
            ))),
            clearance_class: 0,
            fixed: FixedStateIr::SystemFixed,
            component_id: 0,
            translation: IntPoint::new(0, 0),
            rotation: 0.0,
            side_changed: false,
            name: None,
        });
        board.insert_keepout(KeepoutIr {
            kind: KeepoutKindIr::Keepout,
            layer_no: 0,
            area: AreaIr::simple(BoardShape::Tile(TileShape::Simplex(Box::new(
                Simplex::empty(),
            )))),
            clearance_class: 1,
            fixed: FixedStateIr::SystemFixed,
            component_id: 0,
            translation: IntPoint::new(0, 0),
            rotation: 0.0,
            side_changed: false,
            name: None,
        });
        board.insert_component_outline(ComponentOutlineIr {
            component_id: 1,
            layer_no: 0,
            area: Some(AreaIr::simple(BoardShape::Circle(BoardCircle::new(
                IntPoint::new(1, 1),
                3,
            )))),
            nets: vec![],
            clearance_class: 1,
            fixed: FixedStateIr::Unfixed,
            is_front: true,
            translation: IntPoint::new(0, 0),
            rotation: 0.0,
            is_courtyard: false,
            is_fabrication: false,
            is_closed: true,
        });
        board.insert_pin(PinIr {
            component_id: 1,
            pin_index: 0,
            padstack_no: 1,
            nets: vec![],
            clearance_class: 1,
            fixed: FixedStateIr::Unfixed,
        });

        let text = canonical_geometry_text(&board);
        let lines: Vec<&str> = text.lines().collect();
        // ids run in insertion order: board outline 1, conduction area 2,
        // octagon keepout 3, simplex keepout 4, component outline 5,
        // pin 6 — the pin and the two outlines are line-free, so 3 lines
        // in DESCENDING id order (T39).
        assert_eq!(board.last_assigned_item_id(), 6);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "K 4 0 1 system_fixed simplex 0");
        // Octagon of bounding_octagon([(10,20),(110,120)]): axis borders
        // at the min/max coordinates; uld/lrd are the x-intercepts of the
        // +45-degree border y = x + (y-x) = x + 10 (both points share
        // y-x = 10) -> -10; lld/urd the x-intercepts of the -45-degree
        // borders y = -x + (x+y): 30 and 230. Consistent with the
        // jar-pinned T6 octagon math (shape.rs / float_point.rs pins).
        assert_eq!(
            lines[1],
            "K 3 0 0 system_fixed octagon 10 20 110 120 -10 -10 30 230"
        );
        assert_eq!(
            lines[2],
            "A 2 0 1 user_fixed 2 2 5 box 0 0 100 100 win circle 50 50 7"
        );
    }
}
