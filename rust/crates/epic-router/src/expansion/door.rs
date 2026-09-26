//! Java `ExpansionDoor` (`autoroute/expansion/ExpansionDoor.java`, 202
//! lines) — a common edge between two expansion rooms.
//!
//! Java doors hold ROOM REFERENCES and recompute `getShape()` through
//! them (a room's shape can change after the door exists —
//! `tryRemoveEdge`/`tryRemoveEdgeLine` call `setShape` on the
//! from-room). Rust value semantics break reference identity, so the
//! door stores the endpoint rooms' `getId()` values (the formulas are
//! the exact Java bodies, see `crate::expansion::room`); the SHAPE
//! consumers (`shape_between`, `get_section_segments`) take the
//! current room shapes as arguments so callers read them live exactly
//! like the Java field dereferences.
//!
//! REFERENCE IDENTITY (the T6 t7 root cause): Java's doors are
//! OBJECTS — `List.remove(door)` / `equals` fall back to reference
//! identity (`ExpansionDoor` overrides neither), so two DISTINCT doors
//! with equal values stay distinct everywhere. The value-only port
//! breaks exactly when the room-id space collides: an incomplete
//! room's `getId()` is `31 * shape.getId() + layer` on content-derived
//! shape ids, so two live same-shape same-layer rooms share an id and
//! the doors to them share the whole `(first, second, dimension)`
//! value — a value-based door removal or endpoint resolution can then
//! pick the wrong twin (the observed phantom completion). The
//! [`ExpansionDoor::tag`] restores the identity: every CONSTRUCTION
//! draws the next instance number from a process-wide counter; clones
//! carry it (the copies Java has of one door object are one reference
//! spread across lists), distinct constructions never do, and
//! `PartialEq` includes it — so equality is Java's identity semantics.
//! Tags never enter any output row, ordering, or hash key ordering:
//! they feed only equality/membership tests and the maze state-slot
//! key fold (`ExpandableObject::key`), so run-to-run determinism of
//! all printed values is unaffected.
//!
//! Java `sectionArr` (the per-section `MazeSearchElement[]`) is maze
//! state owned by T6; this port does NOT store it (or its length) on
//! the door — [`ExpansionDoor::get_section_segments`] returns the
//! `allocateSections` arity (`ExpansionDoor.java:193-201`) next to the
//! segment list, and the live state/count lives in the maze engine's
//! `door_sections` map.
//!
//! Float arithmetic note: `getSectionSegments` and
//! `calcDoorLineSegment` run in Java `double` arithmetic
//! (`FloatPoint`/`FloatLine`) — mirrored exactly here; the door CORNER
//! walk stays in the integer `Point` domain until the final
//! `toFloat()` conversions (`ExpansionDoor.java:171`).

use std::sync::atomic::{AtomicU64, Ordering};

use epic_geometry::float_line::FloatLine;
use epic_geometry::tile_shape::TileShape;

/// Java `AutorouteEngine.TRACE_WIDTH_TOLERANCE`
/// (`AutorouteEngine.java:41`) — the slack added to the caller's
/// half-width before shrinking a door section line.
pub const TRACE_WIDTH_TOLERANCE: f64 = 2.0;

/// The instance-tag source (see the module doc): Java's door object
/// identity, one draw per construction. Single-threaded parity runs
/// make the draw order deterministic; the values are never printed.
///
/// THREADS STRATEGY (M5-T7, the plan's "reconciled to the sequential
/// counter order" arm): the T7 partitioned executor keeps at most ONE
/// search in flight — the board-side environment is handed off between
/// partition workers per work unit and the coordinator blocks on the
/// reply before the next dispatch (see `pipeline::pass_runner`
/// `SendUnitEnv`). Every door construction therefore draws its tag in
/// exactly the golden sequential order regardless of the thread
/// count, so tag-equality/membership outcomes are thread-count
/// invariant by construction, not by coincidence of timing. (The
/// alternative "hoist tag assignment into the reduction" arm is not
/// needed while the conflicted point — the board — is fully
/// serialized; revisit only if a future executor runs searches
/// concurrently.)
static DOOR_TAG_COUNTER: AtomicU64 = AtomicU64::new(0);

/// An expansion door: a common edge between two expansion rooms
/// (Java `ExpansionDoor`). The dimension may be 1 or 2; 2-dimensional
/// doors can only exist between obstacle expansion rooms (or the
/// corner-overlap case between completed free-space rooms).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpansionDoor {
    /// Java `firstRoom.getId()`.
    pub first_room_id: i32,
    /// Java `secondRoom.getId()`.
    pub second_room_id: i32,
    /// Java `dimension`.
    pub dimension: i32,
    /// The instance tag — Java's reference identity (module doc).
    /// Included in `PartialEq`: clones of one construction are the
    /// copies of one Java object; distinct constructions differ.
    pub(crate) tag: u64,
    // Java also holds `sectionArr` (the per-section maze state) ON the
    // door; this port keeps that state in the maze engine's
    // `door_sections` map (keyed by the door key) — a `section_count`
    // FIELD here would be a constant-0 trap (the only reader in the T6
    // round mis-derived `mazeSearchElementCount()` from it; quality
    // round removed it). The live count is the map entry's length.
}

impl ExpansionDoor {
    /// Java `new ExpansionDoor(firstRoom, secondRoom, dimension)`
    /// (`:28-32`) — the explicit-dimension constructor.
    pub fn new(first_room_id: i32, second_room_id: i32, dimension: i32) -> Self {
        ExpansionDoor {
            first_room_id,
            second_room_id,
            dimension,
            tag: DOOR_TAG_COUNTER.fetch_add(1, Ordering::Relaxed),
        }
    }

    /// Java `new ExpansionDoor(firstRoom, secondRoom)` (`:35-39`) —
    /// the dimension is the intersection dimension of the two room
    /// shapes AT CREATION TIME (Java computes it once here; later
    /// shape changes do not re-derive it).
    pub fn new_between(
        first_room_id: i32,
        second_room_id: i32,
        first_shape: &TileShape,
        second_shape: &TileShape,
    ) -> Self {
        let dimension = first_shape.intersection(second_shape).dimension();
        ExpansionDoor {
            first_room_id,
            second_room_id,
            dimension,
            tag: DOOR_TAG_COUNTER.fetch_add(1, Ordering::Relaxed),
        }
    }

    /// Java `getId()` (`:185-190`) — the order-independent room-id
    /// combination `min * 31 + max`. Java int overflow WRAPS; the
    /// arithmetic is `wrapping_*` on purpose.
    #[must_use]
    pub fn id(&self) -> i32 {
        let (lo, hi) = if self.first_room_id <= self.second_room_id {
            (self.first_room_id, self.second_room_id)
        } else {
            (self.second_room_id, self.first_room_id)
        };
        lo.wrapping_mul(31).wrapping_add(hi)
    }

    /// Java `otherRoom(room)` (`:62-72`) — the other endpoint's room
    /// id, or `None` if `room_id` is neither endpoint.
    #[must_use]
    pub fn other_room_id(&self, room_id: i32) -> Option<i32> {
        if room_id == self.first_room_id {
            Some(self.second_room_id)
        } else if room_id == self.second_room_id {
            Some(self.first_room_id)
        } else {
            None
        }
    }

    /// True if `room_id` is one of the two endpoints (the
    /// `firstRoom == r || secondRoom == r` half of Java
    /// `doorExists`, `FreeSpaceExpansionRoom.java:81-91`).
    #[must_use]
    pub fn touches_room(&self, room_id: i32) -> bool {
        room_id == self.first_room_id || room_id == self.second_room_id
    }

    /// Java `getShape()` (`:42-47`) — the intersection of the CURRENT
    /// shapes of the two rooms (the caller passes them live).
    #[must_use]
    pub fn shape_between(first_shape: &TileShape, second_shape: &TileShape) -> TileShape {
        first_shape.intersection(second_shape)
    }

    /// Java `getSectionSegments(offsetParam)` (`:105-143`) — the line
    /// segments of the door sections at the given trace half-width.
    /// Returns `(section_count, segments)`; the count is the Java
    /// `allocateSections` arity (T6 allocates the maze elements).
    ///
    /// `both_complete_free_space` is the Java
    /// `firstRoom instanceof CompleteFreeSpaceExpansionRoom &&
    /// secondRoom instanceof ...` test (`:118-120`); the corner
    /// restraint line is only calculated in that case. `first_shape` /
    /// `second_shape` are the rooms' live shapes (`calcDoorLineSegment`
    /// reads them, `:149-172`).
    ///
    /// The owned Java-parity surface: this wrapper delegates to
    /// [`Self::get_section_segments_into`] (the production path — the
    /// maze engine's reusable `section_buf`); see the core's doc for
    /// the appending contract.
    #[must_use]
    pub fn get_section_segments(
        &self,
        door_shape: &TileShape,
        both_complete_free_space: bool,
        first_shape: &TileShape,
        second_shape: &TileShape,
        offset_param: f64,
    ) -> (usize, Vec<FloatLine>) {
        let mut out = Vec::new();
        let count = self.get_section_segments_into(
            door_shape,
            both_complete_free_space,
            first_shape,
            second_shape,
            offset_param,
            &mut out,
        );
        (count, out)
    }

    /// The appending core of [`Self::get_section_segments`] (slice C):
    /// the identical sections in the identical order appended to `out`
    /// (cleared first), so the caller's buffer is reused across door
    /// expansions. Returns the section count (the Java
    /// `allocateSections` arity).
    pub fn get_section_segments_into(
        &self,
        door_shape: &TileShape,
        both_complete_free_space: bool,
        first_shape: &TileShape,
        second_shape: &TileShape,
        offset_param: f64,
        out: &mut Vec<FloatLine>,
    ) -> usize {
        out.clear();
        let offset = offset_param + TRACE_WIDTH_TOLERANCE;
        if door_shape.is_empty() {
            // Java :109-112 — the EMPTY door-shape arm fires BEFORE any
            // dimension dispatch.
            return 0;
        }
        let (door_line_segment, shrinked_line_segment) = if self.dimension == 1 {
            // Java :115-117 — the diagonal corner segment of the
            // 1-dimensional door shape (bounded for a live door; Java
            // would NPE on null).
            let door_line_segment = door_shape
                .diagonal_corner_segment()
                .expect("1-dimensional door shape has a diagonal corner segment");
            let shrinked = door_line_segment.shrink_segment(offset);
            (door_line_segment, shrinked)
        } else if self.dimension == 2 && both_complete_free_space {
            // Java :118-132 — overlapping doors at a corner possible
            // in case of 90- or 45-degree routing; the restraint line
            // is the diagonal between the corners on BOTH room
            // borders.
            let Some(door_line_segment) =
                calc_door_line_segment(door_shape, first_shape, second_shape)
            else {
                // Java :124-127 — a complete room inside the other.
                return 0;
            };
            if door_line_segment.b.distance_square(&door_line_segment.a) < 4.0 * offset * offset {
                // Java :128-131 — the door is small; 2-dimensional
                // small doors are not yet expanded.
                return 0;
            }
            let shrinked = door_line_segment.shrink_segment(offset);
            (door_line_segment, shrinked)
        } else {
            // Java :133-137 — the gravity-point degenerate segment
            // (zero length): every section collapses to the centre.
            let gravity_point = door_shape.centre_of_gravity();
            let door_line_segment = FloatLine::new(gravity_point, gravity_point);
            let shrinked = door_line_segment;
            (door_line_segment, shrinked)
        };

        // Java :138-142 — the section count: Java's `(int)` cast
        // truncates toward zero (JLS d2i also SATURATES at
        // Integer.MAX_VALUE, which Rust's `as` mirrors); the +1 keeps
        // a section for every non-negative quotient.
        let max_door_section_width = 10.0 * offset;
        let section_count = (door_line_segment.b.distance(&door_line_segment.a)
            / max_door_section_width) as i32
            + 1;
        shrinked_line_segment.divide_segment_into_sections_into(section_count, out);
        section_count as usize
    }
}

/// Java `calcDoorLineSegment` (`ExpansionDoor.java:149-172`) — the
/// diagonal of the 2-dimensional door shape whose endpoints lie on the
/// border of BOTH room shapes (the restraint line between them).
/// `None` is Java's null (fewer than two distinct common corners).
fn calc_door_line_segment(
    door_shape: &TileShape,
    first_shape: &TileShape,
    second_shape: &TileShape,
) -> Option<FloatLine> {
    let mut first_corner: Option<epic_geometry::point::Point> = None;
    let mut second_corner: Option<epic_geometry::point::Point> = None;
    let corner_count = door_shape.border_line_count();
    for i in 0..corner_count as i32 {
        let current_corner = door_shape.corner(i);
        if !first_shape.contains_inside(&current_corner)
            && !second_shape.contains_inside(&current_corner)
        {
            // currentCorner is on the border of both room shapes.
            match &first_corner {
                None => first_corner = Some(current_corner),
                Some(first) if *first != current_corner => {
                    second_corner = Some(current_corner);
                    break;
                }
                Some(_) => {}
            }
        }
    }
    let first_corner = first_corner?;
    let second_corner = second_corner?;
    Some(FloatLine::new(
        first_corner.to_float(),
        second_corner.to_float(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use epic_geometry::int_octagon::IntOctagon;
    use epic_geometry::regular_tile_shape::RegularTileShape;

    /// An exact rectangle (or octagon) built from raw border
    /// coordinates, normalized like the engine's shapes.
    #[allow(clippy::too_many_arguments)] // the raw border octagon IS 8 coordinates
    fn oct(
        left_x: i32,
        bottom_y: i32,
        right_x: i32,
        top_y: i32,
        ulx: i32,
        lrx: i32,
        llx: i32,
        urx: i32,
    ) -> TileShape {
        TileShape::RegularTileShape(RegularTileShape::IntOctagon(
            IntOctagon::new(left_x, bottom_y, right_x, top_y, ulx, lrx, llx, urx).normalize(),
        ))
    }

    /// The section lines as raw `(ax, ay, bx, by)` f64 tuples, for
    /// bit-exact comparison against the Java-double literals.
    fn section_coords(sections: &[FloatLine]) -> Vec<(f64, f64, f64, f64)> {
        sections
            .iter()
            .map(|line| (line.a.x, line.a.y, line.b.x, line.b.y))
            .collect()
    }

    /// The rot-recovery instruction the T2-P1/P2 literal asserts carry
    /// (the T1 QN-3 convention, route_events.rs `ROT`): a failed face
    /// must say what to do, not just what broke.
    const ROT: &str = "literal mismatch — re-derive, do not edit: world shapes via the committed rust/harness/oracle/RoomSnapProbe.java (e1_ripup at the first net-2 assign; room-label witness too), literals via the JDK-25 jar's FloatLine.shrinkSegment + divideSegmentIntoSections on the pinned inputs, verdicts vs rust/harness/corpus/events-golden.jsonl ordinals 58-64 (full wall) / 65+ (trapezoid)";

    /// M4-T2 PIN (T2-P1) — the e1_ripup net-2 full-wall door, the
    /// events-golden divergence face at 1-based assign ordinal 58.
    ///
    /// Java verdicts: the golden rows 58-64 expand
    /// `ExpansionDoor/bounds=[(216250,238750)..(983750,238750)]/dim=1
    /// /sections=7` through selected sections 0..=6 (seven sections on
    /// the FULL wall); the lazy door shape
    /// `Oct(lx=216250,ly=238750,rx=983750,uy=238750,ulx=-22500,
    /// lrx=745000,llx=455000,urx=1222500)` is the Java-side measured
    /// intersection of complete room 1 with item 20's single straight
    /// tree shape (probed via the oracle at the first net-2 assign).
    ///
    /// The section LINES are the Java `getSectionSegments` double
    /// arithmetic on those witnessed inputs (offsetParam =
    /// compensatedTraceHalfWidth = 11250, tolerance +2, shrink, count
    /// = (int)(767500/112520)+1 = 7, even divide) — every value here
    /// is an exact integer, so the literals are the unique Java
    /// verdict, not a reimplementation.
    #[test]
    fn door_sections_full_wall_e1_net2_ordinal58() {
        let door_shape = oct(
            216250, 238750, 983750, 238750, -22500, 745000, 455000, 1222500,
        );
        let room1 = oct(
            216250, 91250, 983750, 238750, -22500, 892500, 307500, 1222500,
        );
        // dim=1: the room shapes are unread (Java only reaches them in
        // the dim=2 free-space branch).
        let door = ExpansionDoor::new(1, 20480, 1);
        let (count, sections) =
            door.get_section_segments(&door_shape, false, &room1, &room1, 11250.0);
        assert_eq!(count, 7, "golden ords 58-64: sections=7. {ROT}");
        assert_eq!(
            section_coords(&sections),
            vec![
                (227502.0, 238750.0, 333930.0, 238750.0),
                (333930.0, 238750.0, 440358.0, 238750.0),
                (440358.0, 238750.0, 546786.0, 238750.0),
                (546786.0, 238750.0, 653214.0, 238750.0),
                (653214.0, 238750.0, 759642.0, 238750.0),
                (759642.0, 238750.0, 866070.0, 238750.0),
                (866070.0, 238750.0, 972498.0, 238750.0),
            ],
            "Java shrinkSegment + divideSegmentIntoSections on the full wall. {ROT}"
        );
    }

    /// M4-T2 PIN (T2-P2) — the e1_ripup net-2 free-space overlap door
    /// (complete room 1 x complete room 3), the second witnessed wall.
    ///
    /// Java verdicts: the golden rows from ordinal 65 expand
    /// `ExpansionDoor/bounds=[(216250,91250)..(539482,233750)]/dim=2
    /// /sections=4`. The room shapes are the walk-measured octagons
    /// whose COMPLETE_ROOM rows sit inside the matched stream prefix
    /// (complete room 3 carries the cut lower-right/upper-right corners
    /// of the restart-restrained completeShape — its lrx=448232, where
    /// rx-ly would be 538132; the snapshot probe is the label witness):
    /// the door shape is their
    /// intersection — a trapezoid whose restraint line runs
    /// (539482,91250) -> (216250,233750) (the only corners on BOTH
    /// room borders), length sqrt(124785175824) < 4*max section
    /// width, count = 3+1 = 4. The literals are the exact Java-double
    /// shrink/divide results on that line (ulp-faithful: Java's
    /// shrinkSegment computes newB from the ORIGINAL a with the
    /// REDUCED length, and the divide snaps the LAST endpoint to the
    /// exact b).
    #[test]
    fn door_sections_free_space_trapezoid_e1_net2_ordinal65() {
        // Raw max/min intersection coordinates; normalize() relaxes the
        // inactive upper-left diagonal to lx-uy = -17500.
        let door_shape = oct(
            216250, 91250, 539482, 233750, -22500, 448232, 307500, 630732,
        );
        let room1 = oct(
            216250, 91250, 983750, 238750, -22500, 892500, 307500, 1222500,
        );
        let room3 = oct(1350, 1350, 539482, 233750, -232400, 448232, 2700, 630732);
        let door = ExpansionDoor::new(1, 3, 2);
        let (count, sections) =
            door.get_section_segments(&door_shape, true, &room1, &room3, 11250.0);
        assert_eq!(count, 4, "golden ordinal 65+: sections=4. {ROT}");
        assert_eq!(
            section_coords(&sections),
            vec![
                (
                    529186.1412624135,
                    95789.0303871711,
                    453526.07063120673,
                    129144.51519358555
                ),
                (453526.07063120673, 129144.51519358555, 377866.0, 162500.0),
                (377866.0, 162500.0, 302205.9293687933, 195855.48480641446),
                (
                    302205.9293687933,
                    195855.48480641446,
                    226545.8587375866,
                    229210.96961282892
                ),
            ],
            "Java shrink/divide on the restraint line (539482,91250)->(216250,233750). {ROT}"
        );
    }

    /// M4-T2 PIN (T2-P3) — the SMALL dim=2 free-space door is NOT
    /// expanded (Java `:128-131`: distanceSquare < 4*offset*offset →
    /// empty sections; "2 dimensional small doors are not yet
    /// expanded"). Two side-by-side rectangles overlapping in a
    /// 5000x30000 strip: the restraint line is the 5000-long bottom
    /// strip edge, length^2 = 2.5e7 < 5.06430016e8.
    #[test]
    fn small_dim2_free_space_door_not_expanded() {
        // rectangle (0,0)..(20000,30000)
        let room1 = oct(0, 0, 20000, 30000, -30000, 20000, 0, 50000);
        // rectangle (15000,0)..(35000,30000)
        let room2 = oct(15000, 0, 35000, 30000, -15000, 35000, 15000, 65000);
        // intersection (15000,0)..(20000,30000)
        let door_shape = oct(15000, 0, 20000, 30000, -15000, 20000, 15000, 50000);
        let door = ExpansionDoor::new(1, 2, 2);
        let (count, sections) =
            door.get_section_segments(&door_shape, true, &room1, &room2, 11250.0);
        assert_eq!(count, 0, "small door: no sections (Java :128-131)");
        assert!(sections.is_empty());
    }

    /// M4-T2 PIN (T2-P4) — the dim=2 door that is NOT between two
    /// complete free-space rooms takes the gravity-point arm (Java
    /// `:133-137`): a zero-length line at the centre of gravity,
    /// count = (0/max)+1 = 1, the single section collapsed onto it.
    #[test]
    fn dim2_obstacle_door_single_gravity_section() {
        let door_shape = oct(0, 0, 10000, 20000, -20000, 10000, 0, 30000);
        let room = oct(0, 0, 10000, 20000, -20000, 10000, 0, 30000);
        let door = ExpansionDoor::new(1, 2, 2);
        let (count, sections) =
            door.get_section_segments(&door_shape, false, &room, &room, 11250.0);
        assert_eq!(count, 1, "gravity-point arm: exactly one section");
        assert_eq!(sections.len(), 1);
        let line = &sections[0];
        assert_eq!(line.a, line.b, "zero-length section line");
        assert_eq!(
            (line.a.x, line.a.y),
            (5000.0, 10000.0),
            "the rectangle's centre of gravity"
        );
    }

    /// Sanity for the pin worlds: the two witnessed door shapes have
    /// the dimensions the golden rows declare (dim=1 / dim=2).
    #[test]
    fn witnessed_door_shapes_dimensions() {
        let wall1 = oct(
            216250, 238750, 983750, 238750, -22500, 745000, 455000, 1222500,
        );
        assert_eq!(wall1.dimension(), 1, "full-wall door is dim=1");
        let trapezoid = oct(
            216250, 91250, 539482, 233750, -22500, 448232, 307500, 630732,
        );
        assert_eq!(trapezoid.dimension(), 2, "overlap door is dim=2");
    }

    /// M5-T7 PIN (T7-D1) — the EMPTY door-shape arm (Java `:109-112`,
    /// the M4-T2 BACKLOG obligation): an empty door shape answers
    /// `(0, empty)` BEFORE any dimension dispatch, so even a dim=1
    /// door never reaches `diagonalCornerSegment` on an empty shape
    /// (Java would return the empty array; the mis-derived empty-check
    /// arity falls through to the dim dispatch here).
    #[test]
    fn empty_door_shape_answers_empty_before_dimension_dispatch() {
        // Two disjoint rectangles: the intersection IS the empty shape.
        let room1 = oct(0, 0, 1000, 1000, -1000, 1000, 0, 2000);
        let room2 = oct(5000, 5000, 9000, 9000, 0, 9000, 5000, 14000);
        let door_shape = room1.intersection(&room2);
        assert!(
            door_shape.is_empty(),
            "world check: disjoint rooms intersect empty"
        );
        let door = ExpansionDoor::new(1, 2, 1);
        let (count, sections) =
            door.get_section_segments(&door_shape, false, &room1, &room2, 11250.0);
        assert_eq!(count, 0, "EMPTY arm: zero sections (Java :109-112). {ROT}");
        assert!(
            sections.is_empty(),
            "EMPTY arm: no segment lines (Java :109-112). {ROT}"
        );
    }

    /// M5-T7 PIN (T7-D2) — the both-complete-NULL arm (Java `:124-127`,
    /// the M4-T2 BACKLOG obligation): a dim=2 door between two complete
    /// free-space rooms where fewer than two DISTINCT corners lie on
    /// the border of BOTH room shapes — here room 1 strictly contains
    /// the door shape and room 2 IS it, so every door corner sits
    /// strictly inside room 1 — `calcDoorLineSegment` answers null and
    /// the arm answers `(0, empty)` ("a complete room inside the
    /// other"). The shape is deliberately LARGE, so this world cannot
    /// be confused with the small-door arm (T2-P3, Java `:128-131`)
    /// that shares the `(0, empty)` verdict.
    #[test]
    fn dim2_complete_rooms_without_common_border_corners_answer_null_arm() {
        // Door shape: rectangle (0,0)..(10000,20000), dim=2.
        let door_shape = oct(0, 0, 10000, 20000, -20000, 10000, 0, 30000);
        // Room 1 strictly contains the door shape (its corners are
        // strictly inside room 1, so never "on the border of both").
        let room1 = oct(-5000, -5000, 15000, 25000, -30000, 20000, -10000, 40000);
        // Room 2 equals the door shape: its corners sit on room 2's
        // border but strictly inside room 1.
        let room2 = door_shape.clone();
        // World check: each corner is strictly inside room 1.
        for i in 0..door_shape.border_line_count() {
            let corner = door_shape.corner(i as i32);
            assert!(
                room1.contains_inside(&corner),
                "world check: door corner strictly inside room 1"
            );
        }
        let door = ExpansionDoor::new(1, 2, 2);
        let (count, sections) =
            door.get_section_segments(&door_shape, true, &room1, &room2, 11250.0);
        assert_eq!(
            count, 0,
            "NULL arm: zero sections (Java :124-127, a complete room inside the other). {ROT}"
        );
        assert!(
            sections.is_empty(),
            "NULL arm: no segment lines (Java :124-127). {ROT}"
        );
    }
}
