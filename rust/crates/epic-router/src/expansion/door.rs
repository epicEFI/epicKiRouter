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
//! Java `sectionArr` (the per-section `MazeSearchElement[]`) is maze
//! state owned by T6; this port keeps only the section COUNT (the
//! `allocateSections` arity, `ExpansionDoor.java:193-201`) and returns
//! it next to the segment list.
//!
//! Float arithmetic note: `getSectionSegments` and
//! `calcDoorLineSegment` run in Java `double` arithmetic
//! (`FloatPoint`/`FloatLine`) — mirrored exactly here; the door CORNER
//! walk stays in the integer `Point` domain until the final
//! `toFloat()` conversions (`ExpansionDoor.java:171`).

use epic_geometry::float_line::FloatLine;
use epic_geometry::tile_shape::TileShape;

/// Java `AutorouteEngine.TRACE_WIDTH_TOLERANCE`
/// (`AutorouteEngine.java:41`) — the slack added to the caller's
/// half-width before shrinking a door section line.
pub const TRACE_WIDTH_TOLERANCE: f64 = 2.0;

/// An expansion door: a common edge between two expansion rooms
/// (Java `ExpansionDoor`). The dimension may be 1 or 2; 2-dimensional
/// doors can only exist between obstacle expansion rooms (or the
/// corner-overlap case between completed free-space rooms).
#[derive(Clone, Debug)]
pub struct ExpansionDoor {
    /// Java `firstRoom.getId()`.
    pub first_room_id: i32,
    /// Java `secondRoom.getId()`.
    pub second_room_id: i32,
    /// Java `dimension`.
    pub dimension: i32,
    /// Java `sectionArr.length` after the last `allocateSections`.
    pub section_count: usize,
}

impl ExpansionDoor {
    /// Java `new ExpansionDoor(firstRoom, secondRoom, dimension)`
    /// (`:28-32`) — the explicit-dimension constructor.
    pub fn new(first_room_id: i32, second_room_id: i32, dimension: i32) -> Self {
        ExpansionDoor {
            first_room_id,
            second_room_id,
            dimension,
            section_count: 0,
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
            section_count: 0,
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
    #[must_use]
    pub fn get_section_segments(
        &self,
        door_shape: &TileShape,
        both_complete_free_space: bool,
        first_shape: &TileShape,
        second_shape: &TileShape,
        offset_param: f64,
    ) -> (usize, Vec<FloatLine>) {
        let offset = offset_param + TRACE_WIDTH_TOLERANCE;
        if door_shape.is_empty() {
            // Java :108-112.
            return (0, Vec::new());
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
                return (0, Vec::new());
            };
            if door_line_segment.b.distance_square(&door_line_segment.a) < 4.0 * offset * offset {
                // Java :128-131 — the door is small; 2-dimensional
                // small doors are not yet expanded.
                return (0, Vec::new());
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
        (
            section_count as usize,
            shrinked_line_segment.divide_segment_into_sections(section_count),
        )
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
