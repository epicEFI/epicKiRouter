//! Java `autoroute/maze/MazeTraceShover.java` (`checkShoveTraceLine`,
//! `endPointsMatching`, `DoorSection`) plus the
//! `MazeSearchEngine.shoveTraceRoom` caller (`:1130-1201`).
//!
//! READ-ONLY PROBE — the T7 scope boundary: the probe consults the two
//! shove seams [`DrillEngine::check_trace_segment`] and
//! [`DrillEngine::shove_trace_check`] but never MUTATES a board trace.
//! What IS ported beyond the verdicts is the maze-side state the Java
//! call chain produces: `ExpansionDoor.getSectionSegments` allocates
//! the door's maze-search section state as a side effect, mirrored
//! here through [`MazeSearchEngine::allocate_sections`] at the same
//! call sites. The mutation body (`TraceShover.check` recursion and
//! the trace insertion) is the T10 remainder.

use epic_geometry::float_line::FloatLine;
use epic_geometry::line::Line;
use epic_geometry::line_segment::LineSegment;
use epic_geometry::point::Point;
use epic_geometry::side::Side;

use crate::drill::{Adjustment, DestinationDistance, DrillEngine, ViaLayerChecker};
use crate::expansion::ExpansionDoor;
use crate::maze::list_element::{ExpandableObject, MazeListElement};
use crate::maze::search_engine::MazeSearchEngine;

/// Java `MazeTraceShover.DoorSection` — a candidate door section to
/// expand after a successful shove. Identity is the door's value
/// (Java compares door references; a door is uniquely determined by
/// its room pair, dimension and shape id, so value equality coincides
/// with reference identity for live doors).
#[derive(Clone, Debug)]
pub(crate) struct DoorSection {
    pub door: ExpansionDoor,
    pub section_index: i32,
    pub section_line: FloatLine,
}

impl<E: DrillEngine, D: DestinationDistance, V: ViaLayerChecker> MazeSearchEngine<'_, E, D, V> {
    /// Java `MazeSearchEngine.shoveTraceRoom` (`:1130-1201`) — shoves a
    /// trace room and expands the corresponding doors. Returns false
    /// only when NO door was expanded (inner sections return true: no
    /// delay of occupation is necessary because inner sections of a
    /// door are currently not shoved).
    pub(crate) fn shove_trace_room(
        &mut self,
        element: &MazeListElement,
        obstacle_room_key: u64,
    ) -> bool {
        if element.section_no_of_door != 0
            && element.section_no_of_door != self.maze_search_element_count(&element.door) - 1
        {
            // No delay of occupation necessary because inner sections
            // of a door are currently not shoved.
            return true;
        }
        let mut result = false;
        if element.adjustment != Adjustment::Right {
            let mut left_to_door_section_list: Vec<DoorSection> = Vec::new();

            if self.check_shove_trace_line(
                element,
                obstacle_room_key,
                false,
                &mut left_to_door_section_list,
            ) {
                result = true;
            }

            for current_left_door_section in left_to_door_section_list {
                let current_adjustment = if current_left_door_section.door.dimension == 2 {
                    // the door is the link door to the next room
                    Adjustment::Left
                } else {
                    Adjustment::None
                };

                let _ = self.expand_to_door_section(
                    ExpandableObject::RoomDoor(current_left_door_section.door),
                    current_left_door_section.section_index,
                    current_left_door_section.section_line,
                    element,
                    0,
                    current_adjustment,
                );
            }
        }

        if element.adjustment != Adjustment::Left {
            let mut right_to_door_section_list: Vec<DoorSection> = Vec::new();

            if self.check_shove_trace_line(
                element,
                obstacle_room_key,
                true,
                &mut right_to_door_section_list,
            ) {
                result = true;
            }
            for current_right_door_section in right_to_door_section_list {
                let current_adjustment = if current_right_door_section.door.dimension == 2 {
                    // the door is the link door to the next room
                    Adjustment::Right
                } else {
                    Adjustment::None
                };
                let _ = self.expand_to_door_section(
                    ExpandableObject::RoomDoor(current_right_door_section.door),
                    current_right_door_section.section_index,
                    current_right_door_section.section_line,
                    element,
                    0,
                    current_adjustment,
                );
            }
        }
        result
    }

    /// Java `MazeTraceShover.checkShoveTraceLine` (`:32-314`). Returns
    /// false if the algorithm did not succeed and trying to shove from
    /// another door section may be more successful; newly shovable
    /// door sections are appended to `to_door_list`.
    pub(crate) fn check_shove_trace_line(
        &mut self,
        element: &MazeListElement,
        obstacle_room_key: u64,
        shove_to_the_left: bool,
        to_door_list: &mut Vec<DoorSection>,
    ) -> bool {
        let ExpandableObject::RoomDoor(from_door) = element.door.clone() else {
            return true;
        };
        let Some(obstacle_item) = self.ctx.room_obstacle_item_key(obstacle_room_key) else {
            return true;
        };
        if !self.ctx.item_is_polyline_trace(obstacle_item) {
            return true;
        }
        let trace_layer = self.ctx.room_layer(obstacle_room_key);
        // only traces with the same halfwidth and the same clearance
        // class can be shoved.
        if self.ctx.item_trace_half_width(obstacle_item)
            != self.ctrl.trace_half_width[trace_layer as usize]
            || self.ctx.item_clearance_class(obstacle_item) != self.ctrl.trace_clearance_class_index
        {
            return true;
        }
        let compensated_trace_half_width =
            self.ctrl.compensated_trace_half_width[trace_layer as usize];
        let (_, _, first_shape, second_shape) = self.door_endpoint_shapes(&from_door);
        let from_door_shape =
            crate::expansion::ExpansionDoor::shape_between(&first_shape, &second_shape);
        if from_door_shape.max_width() < 2.0 * f64::from(compensated_trace_half_width) {
            return true;
        }
        let trace_corner_no = i32::try_from(
            self.ctx
                .room_obstacle_index_in_item(obstacle_room_key)
                .expect("an obstacle room carries its index in the item"),
        )
        .expect("corner index fits i32");

        let trace_polyline = self
            .ctx
            .item_trace_polyline(obstacle_item)
            .expect("a polyline trace carries its polyline");

        // Check if traceCornerNo allows access to indices up to
        // traceCornerNo + 2. Stale indices can occur when traces are
        // modified during routing (pull-tight, shoving, etc.)
        if trace_corner_no < 0 || trace_corner_no >= trace_polyline.lines.len() as i32 - 2 {
            return false;
        }
        let room_doors = self.ctx.room_doors(obstacle_room_key);
        // The side of the trace line seen from the doors to expand.
        // Used to determine, if a door is on the right side to put it
        // into the doorList.
        let shove_line_segment: LineSegment;
        if from_door.dimension == 2 {
            // shove from a link door into the direction of the other
            // link door.
            let obstacle_room_id = self.ctx.room_id(obstacle_room_key);
            let Some(other_room_key) = from_door
                .other_room_id(obstacle_room_id)
                .and_then(|id| self.ctx.room_key_of_id(id))
            else {
                return false;
            };
            if !self.ctx.room_is_obstacle(other_room_key) {
                return false;
            }
            let other_item = self
                .ctx
                .room_obstacle_item_key(other_room_key)
                .expect("an obstacle room carries its obstacle item");
            if !end_points_matching(self.ctx, obstacle_item, other_item) {
                return false;
            }
            let door_center = from_door_shape.centre_of_gravity();
            let corner1 = trace_polyline.corner_approx(trace_corner_no);
            let corner2 = trace_polyline.corner_approx(trace_corner_no + 1);
            if corner1.distance_square(&corner2) < 1.0 {
                // shoveLineSegment may be reduced to a point
                return false;
            }
            let shove_into_direction_of_trace_start =
                door_center.distance_square(&corner2) < door_center.distance_square(&corner1);
            let mut segment = LineSegment::from_polyline(&trace_polyline, trace_corner_no + 1);
            if shove_into_direction_of_trace_start {
                // shove from the endpoint to the start point of the
                // line segment
                segment = segment.opposite();
            }
            shove_line_segment = segment;
        } else {
            let obstacle_room_id = self.ctx.room_id(obstacle_room_key);
            let from_room_key = from_door
                .other_room_id(obstacle_room_id)
                .and_then(|id| self.ctx.room_key_of_id(id))
                .expect("a live room door's other room is registered");
            let from_point = self.ctx.room_shape(from_room_key).centre_of_gravity();
            let shove_trace_line = trace_polyline.lines[(trace_corner_no + 1) as usize].clone();
            let door_line_segment = from_door_shape
                .diagonal_corner_segment()
                .expect("the from-door shape passed the width gate");
            let side_of_trace_line = shove_trace_line.side_of_float_zero(&door_line_segment.a);

            let polar_line_segment = from_door_shape
                .polar_line_segment(&from_point)
                .expect("the from-door shape passed the width gate");

            let door_line_swapped = polar_line_segment.b.distance_square(&door_line_segment.a)
                < polar_line_segment.a.distance_square(&door_line_segment.a);

            // shove only from the right most section to the right or
            // from the left most section to the left.
            let shape_entry_check_distance = f64::from(compensated_trace_half_width) + 5.0;
            let check_dist_square = shape_entry_check_distance * shape_entry_check_distance;

            let section_ok = if (shove_to_the_left && !door_line_swapped)
                || (!shove_to_the_left && door_line_swapped)
            {
                element.section_no_of_door == self.maze_search_element_count(&element.door) - 1
                    && (element.shape_entry.a.distance_square(&door_line_segment.b)
                        <= check_dist_square
                        || element.shape_entry.b.distance_square(&door_line_segment.b)
                            <= check_dist_square)
            } else {
                element.section_no_of_door == 0
                    && (element.shape_entry.a.distance_square(&door_line_segment.a)
                        <= check_dist_square
                        || element.shape_entry.b.distance_square(&door_line_segment.a)
                            <= check_dist_square)
            };
            if !section_ok {
                return false;
            }

            // create the line segment for shoving

            let shrinked_line_segment =
                polar_line_segment.shrink_segment(f64::from(compensated_trace_half_width));
            let perpendicular_direction = shove_trace_line.direction().clone().turn_45_degree(2);
            if side_of_trace_line == Side::Positive {
                if shove_to_the_left {
                    let start_closing_line = Line::new_with_direction(
                        Point::get_instance(
                            shrinked_line_segment.b.round().x,
                            shrinked_line_segment.b.round().y,
                        ),
                        perpendicular_direction,
                    );
                    shove_line_segment = LineSegment::new(
                        start_closing_line,
                        trace_polyline.lines[(trace_corner_no + 1) as usize].clone(),
                        trace_polyline.lines[(trace_corner_no + 2) as usize].clone(),
                    );
                } else {
                    let start_closing_line = Line::new_with_direction(
                        Point::get_instance(
                            shrinked_line_segment.a.round().x,
                            shrinked_line_segment.a.round().y,
                        ),
                        perpendicular_direction,
                    );
                    shove_line_segment = LineSegment::new(
                        start_closing_line,
                        trace_polyline.lines[(trace_corner_no + 1) as usize].opposite(),
                        trace_polyline.lines[trace_corner_no as usize].opposite(),
                    );
                }
            } else if shove_to_the_left {
                let start_closing_line = Line::new_with_direction(
                    Point::get_instance(
                        shrinked_line_segment.b.round().x,
                        shrinked_line_segment.b.round().y,
                    ),
                    perpendicular_direction,
                );
                shove_line_segment = LineSegment::new(
                    start_closing_line,
                    trace_polyline.lines[(trace_corner_no + 1) as usize].opposite(),
                    trace_polyline.lines[trace_corner_no as usize].opposite(),
                );
            } else {
                let start_closing_line = Line::new_with_direction(
                    Point::get_instance(
                        shrinked_line_segment.a.round().x,
                        shrinked_line_segment.a.round().y,
                    ),
                    perpendicular_direction,
                );
                shove_line_segment = LineSegment::new(
                    start_closing_line,
                    trace_polyline.lines[(trace_corner_no + 1) as usize].clone(),
                    trace_polyline.lines[(trace_corner_no + 2) as usize].clone(),
                );
            }
        }
        let trace_half_width = self.ctrl.trace_half_width[trace_layer as usize];
        let net_numbers = [self.ctrl.net_number];

        let mut shove_width = self.ctx.check_trace_segment(
            &shove_line_segment,
            trace_layer,
            &net_numbers,
            trace_half_width,
            self.ctrl.trace_clearance_class_index,
            true,
        );
        let mut shove_line_segment = shove_line_segment;
        let mut segment_shortened = false;
        if shove_width < 2147483647.0 {
            // shorten shoveLineSegment
            shove_width -= 1.0;
            if shove_width <= 0.0 {
                return true;
            }
            shove_line_segment = shove_line_segment.change_length_approx(shove_width);
            segment_shortened = true;
        }

        let from_corner = shove_line_segment.start_point_approx();
        let to_corner = shove_line_segment.end_point_approx();
        let segment_is_point = from_corner.distance_square(&to_corner) < 0.1;

        if !segment_is_point {
            shove_width = self.ctx.shove_trace_check(
                &shove_line_segment,
                shove_to_the_left,
                trace_layer,
                &net_numbers,
                trace_half_width,
                self.ctrl.trace_clearance_class_index,
                self.ctrl.max_shove_trace_recursion_depth,
                self.ctrl.max_shove_via_recursion_depth,
            );

            if shove_width <= 0.0 {
                return true;
            }
        }

        // Put the doors on this side of the room into toDoorList with
        if segment_shortened {
            shove_width = shove_width.min(from_corner.distance(&to_corner));
        }

        let shove_line = shove_line_segment.get_line().clone();

        // From_door_compare_distance is used to check, that a door is
        // between fromDoor and the end point of the shove line.
        let from_door_compare_distance = if from_door.dimension == 2 || segment_is_point {
            f64::MAX
        } else {
            to_corner.distance_square(
                &from_door_shape
                    .corner_approx(0)
                    .expect("the from-door shape passed the width gate"),
            )
        };

        for current_door in &room_doors {
            if *current_door == from_door {
                continue;
            }
            let first_key = self.ctx.room_key_of_id(current_door.first_room_id);
            let second_key = self.ctx.room_key_of_id(current_door.second_room_id);
            if first_key.is_some_and(|k| self.ctx.room_is_obstacle(k))
                && second_key.is_some_and(|k| self.ctx.room_is_obstacle(k))
            {
                let first_room_item = self
                    .ctx
                    .room_obstacle_item_key(first_key.expect("checked above"));
                let second_room_item = self
                    .ctx
                    .room_obstacle_item_key(second_key.expect("checked above"));
                if first_room_item != second_room_item {
                    // there may be topological problems at a trace fork
                    continue;
                }
            }
            let (_, _, current_first_shape, current_second_shape) =
                self.door_endpoint_shapes(current_door);
            let current_door_shape = crate::expansion::ExpansionDoor::shape_between(
                &current_first_shape,
                &current_second_shape,
            );
            if current_door.dimension == 2 && shove_width >= 2147483647.0 {
                let add_link_door = current_door_shape.contains_float(&to_corner);

                if add_link_door {
                    let (section_count, line_sections) = self
                        .door_sections_of(current_door, f64::from(compensated_trace_half_width));
                    let first_section = line_sections
                        .into_iter()
                        .next()
                        .expect("Java indexes lineSections[0] of a non-empty section list");
                    debug_assert!(section_count > 0);
                    to_door_list.push(DoorSection {
                        door: current_door.clone(),
                        section_index: 0,
                        section_line: first_section,
                    });
                }
            } else if !segment_is_point {
                // now currentDoor is 1-dimensional

                // check, that currentDoor is on the same borderline as
                // fromDoor.
                let Some(current_door_segment) = current_door_shape.diagonal_corner_segment()
                else {
                    // Java traces "check_shove_trace_line: door shape
                    // is empty" (FRLogger, diagnostic only) and skips.
                    continue;
                };
                let start_corner_side_of_trace_line =
                    shove_line.side_of_float_zero(&current_door_segment.a);
                let end_corner_side_of_trace_line =
                    shove_line.side_of_float_zero(&current_door_segment.b);
                if shove_to_the_left {
                    if start_corner_side_of_trace_line != Side::Positive
                        || end_corner_side_of_trace_line != Side::Positive
                    {
                        continue;
                    }
                } else if start_corner_side_of_trace_line != Side::Negative
                    || end_corner_side_of_trace_line != Side::Negative
                {
                    continue;
                }
                let current_door_line = current_door_shape
                    .polar_line_segment(&from_corner)
                    .expect("a non-empty door shape has a polar line segment");
                let current_door_nearest_corner =
                    if current_door_line.a.distance_square(&from_corner)
                        <= current_door_line.b.distance_square(&from_corner)
                    {
                        current_door_line.a
                    } else {
                        current_door_line.b
                    };
                if to_corner.distance_square(&current_door_nearest_corner)
                    >= from_door_compare_distance
                {
                    // currentDoor is not located into the direction of
                    // toCorner.
                    continue;
                }
                let current_door_projection =
                    current_door_nearest_corner.projection_approx(&shove_line);

                if current_door_projection.distance(&from_corner)
                    + f64::from(compensated_trace_half_width)
                    <= shove_width
                {
                    let (_, line_sections) = self
                        .door_sections_of(current_door, f64::from(compensated_trace_half_width));
                    for (i, current_line_section) in line_sections.into_iter().enumerate() {
                        let current_section_nearest_corner =
                            if current_line_section.a.distance_square(&from_corner)
                                <= current_line_section.b.distance_square(&from_corner)
                            {
                                current_line_section.a
                            } else {
                                current_line_section.b
                            };
                        let current_section_projection =
                            current_section_nearest_corner.projection_approx(&shove_line);
                        if current_section_projection.distance(&from_corner) <= shove_width {
                            to_door_list.push(DoorSection {
                                door: current_door.clone(),
                                section_index: i as i32,
                                section_line: current_line_section,
                            });
                        }
                    }
                }
            }
        }
        true
    }

    /// Java `currentDoor.getSectionSegments(compensatedTraceHalfWidth)`
    /// including its maze-state side effect: Java's
    /// `getSectionSegments` allocates the door's section state
    /// (`allocateSections`) except on its early-return arms, which the
    /// pure computation mirrors by returning 0 sections
    /// ([`Self::allocate_sections`] with 0 is a no-op on the empty
    /// default entry).
    fn door_sections_of(
        &mut self,
        door: &crate::expansion::ExpansionDoor,
        compensated_trace_half_width: f64,
    ) -> (usize, Vec<FloatLine>) {
        let (first_key, second_key, first_shape, second_shape) = self.door_endpoint_shapes(door);
        let both_complete_free_space = first_key
            .is_some_and(|k| self.ctx.room_is_complete_free_space(k))
            && second_key.is_some_and(|k| self.ctx.room_is_complete_free_space(k));
        let door_shape =
            crate::expansion::ExpansionDoor::shape_between(&first_shape, &second_shape);
        let result = door.get_section_segments(
            &door_shape,
            both_complete_free_space,
            &first_shape,
            &second_shape,
            compensated_trace_half_width,
        );
        self.allocate_sections(&ExpandableObject::RoomDoor(door.clone()), result.0);
        result
    }
}

/// Java `MazeTraceShover.endPointsMatching` (`:320-342`) — check if
/// the endpoints of trace and fromItem are matching, so that the shove
/// can continue through a link door.
pub(crate) fn end_points_matching<E: DrillEngine>(ctx: &E, trace_key: u64, from_item: u64) -> bool {
    if from_item == trace_key {
        return true;
    }
    if !ctx.item_shares_net(trace_key, from_item) {
        return false;
    }
    let trace_polyline = ctx
        .item_trace_polyline(trace_key)
        .expect("a polyline trace carries its polyline");
    if ctx.item_is_via(from_item) {
        // Java `fromItem instanceof DrillItem` — a via is the only
        // DrillItem the maze search produces here. The comparison
        // reads `item.getCenter()` (MazeTraceShover.java:329-332) —
        // the STORED center, total and always present. It never
        // touches `getAutorouteDrillInfo` (Via.java:204-216), the
        // transient the maze EXPANSION computes lazily from the tree;
        // the old port demanded it here (buglog 170: bm06/bm07
        // panicked) — an expect on data Java never reads at this
        // site. `None` is unreachable for a live via; the defensive
        // arm answers "no match".
        let Some(from_center) = ctx.via_center(from_item) else {
            return false;
        };
        let Some(first_corner) = trace_polyline.first_corner() else {
            return false;
        };
        let Some(last_corner) = trace_polyline.last_corner() else {
            return false;
        };
        points_equal(&from_center, &first_corner) || points_equal(&from_center, &last_corner)
    } else if ctx.item_is_polyline_trace(from_item) {
        let Some(from_trace) = ctx.item_trace_polyline(from_item) else {
            return false;
        };
        let (Some(trace_first), Some(trace_last)) =
            (trace_polyline.first_corner(), trace_polyline.last_corner())
        else {
            return false;
        };
        match (from_trace.first_corner(), from_trace.last_corner()) {
            (Some(from_first), Some(from_last)) => {
                points_equal(&trace_first, &from_first)
                    || points_equal(&trace_first, &from_last)
                    || points_equal(&trace_last, &from_first)
                    || points_equal(&trace_last, &from_last)
            }
            _ => false,
        }
    } else {
        false
    }
}

/// Java `Point.equals` for the corner comparisons (see
/// `crate::path::connection::points_equal`).
fn points_equal(a: &Point, b: &Point) -> bool {
    match (a, b) {
        (Point::Int(x), Point::Int(y)) => x == y,
        _ => a.to_float() == b.to_float(),
    }
}
