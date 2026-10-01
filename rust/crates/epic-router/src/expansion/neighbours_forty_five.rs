//! Java `Sorted45DegreeRoomNeighbours`
//! (`autoroute/expansion/Sorted45DegreeRoomNeighbours.java`, 982
//! lines) — the DEGREE_45 room-neighbour sorter: pure `IntOctagon`
//! integer arithmetic.
//!
//! Orientation convention (THE load-bearing detail): the neighbours
//! are sorted COUNTERCLOCKWISE around the room octagon — border side
//! 0 (bottom) runs left→right, side 2 (right) bottom→top, side 4
//! (top) right→left, side 6 (left) top→bottom. The `compareTo` below
//! is the literal Java body: sides 0-3 compare `is1 - is2`
//! (ASCENDING), sides 4-7 compare `is2 - is1` (the REVERSED corner
//! sign) — inverting the sign mirror-images the walk and T6's
//! door-section assignment inherits it silently.
//!
//! The `(objectId, shapeIndexInObject)` v1.9-parity pre-sort
//! (`:104-113`) is the same total order as the base sorter (distinct
//! entries cannot tie on both keys — sort stability is moot); the
//! shared [`super::neighbours::compare_tree_entries`] is pinned by a
//! unit test (the capture pins are blind to a flipped pre-sort).
//!
//! Differences vs the base (any-angle) sorter, all Java-faithful:
//! * target doors are created DURING the walk
//!   (`fsRoom.calculateTargetDoors(currentEntry, ...)`, `:122-126`,
//!   which calls `setNetDependent()` for EVERY non-obstacle entry)
//!   instead of deferred to the end;
//! * a 2-dim overlap `continue`s for BOTH items and rooms — but ONLY
//!   when the COMPLETED room is an obstacle room (`:132-143`); when it
//!   is free space (every routed completion), 2-dim overlaps fall
//!   through to `addSortedNeighbour` + door creation (the F2 room 6 /
//!   room 3 door is a dimension-2 ROOM overlap — capture-proven);
//! * `tryRemoveEdgeLine` shrinks ALL border lines whose interior the
//!   walk left obstacle-free (`removeNotTouchingBorderLines`,
//!   `:174-243` — the `edgeInteriorTouchesObstacle` flags exclusively;
//!   Java has NO contained-shape survival) and passes the biggest
//!   2-dim complete-free-space door as the ignore pair to
//!   `completeShape` (`:373-401`) — the restraint of the enlarged
//!   shape at the tree shapes is what keeps the F2 restart room
//!   bounded (right_x = 197929, upper-right diagonal 620000 through
//!   two restarts, ids 4/5 burned).

use epic_geometry::int_octagon::IntOctagon;
use epic_geometry::limits::CRIT_INT;
use epic_geometry::regular_tile_shape::RegularTileShape;
use epic_geometry::tile_shape::TileShape;
use std::cmp::Ordering;

use super::door::ExpansionDoor;
use super::neighbours::{
    NeighbourEngine, SortedNeighbours, TreeEntry, compare_tree_entries, create_overlap_door,
    insert_door_ok_pair,
};

/// The DEGREE_45 neighbour calculation (Java
/// `Sorted45DegreeRoomNeighbours.calculate`, `:45-79`). Returns the
/// completed room key.
///
/// Java `:53` evaluates `autorouteEngine.generateRoomIdNo()` as an
/// ARGUMENT of `calculateNeighbours` — the id burns at EVERY calculate
/// entry, for EVERY room kind: an obstacle-room door calculation
/// (`completeNeighbourRooms` obstacle arm → `calculateDoors` →
/// `complete`) consumes the id and discards it, and each restart
/// recursion re-enters `calculate` and burns a fresh one. The T7
/// witness: Java ids 4 and 9 vanish into the wire-obstacle room's
/// door calculation during the drain.
#[must_use]
pub fn calculate(ctx: &mut impl NeighbourEngine, from_room_key: u64) -> Option<u64> {
    let net_number = ctx.net_number();
    let mut room_id_no = ctx.generate_room_id_no();
    let mut room_neighbours = calculate_neighbours(ctx, from_room_key, net_number, room_id_no)?;

    // Check, that each side of the room shape has at least one touching
    // neighbour. Otherwise, improve the room shape by enlarging.
    let mut completed_room = room_neighbours.completed_room;
    let mut edge_removed = room_neighbours.try_remove_edge_line(ctx);
    while edge_removed {
        // Java :63-66 — removeAllDoors + RECURSIVE calculate (the
        // re-entry burns another room id).
        ctx.remove_all_doors(completed_room);
        room_id_no = ctx.generate_room_id_no();
        room_neighbours = calculate_neighbours(ctx, from_room_key, net_number, room_id_no)?;
        completed_room = room_neighbours.completed_room;
        edge_removed = room_neighbours.try_remove_edge_line(ctx);
    }

    // Now calculate the new incomplete rooms together with the doors
    // between this room and the sorted neighbours.
    if room_neighbours.sorted_neighbours.is_empty() {
        if ctx.room_is_obstacle(from_room_key) {
            room_neighbours.calculate_edge_incomplete_rooms(0, 7, ctx);
        }
    } else {
        room_neighbours.calculate_new_incomplete_rooms(ctx);
    }
    Some(completed_room)
}

/// Java `calculateNeighbours` (`:85-172`). `room_id_no` is the id
/// burned by the `calculate` entry (Java `:53` argument evaluation);
/// the incomplete-room arm stamps it on the completed room, the
/// obstacle-room arm discards it (Java consumes the argument either
/// way).
fn calculate_neighbours(
    ctx: &mut impl NeighbourEngine,
    from_room_key: u64,
    net_number: i32,
    room_id_no: i32,
) -> Option<FortyfiveRoomNeighbours> {
    let room_shape = ctx.room_shape(from_room_key);
    let completed_room = if ctx.room_is_incomplete(from_room_key) {
        ctx.add_complete_free_space_room(
            room_shape.clone(),
            ctx.room_layer(from_room_key),
            room_id_no,
        )
    } else if ctx.room_is_obstacle(from_room_key) {
        from_room_key
    } else {
        // Java :94-97 — unexpected expansion room type (warn).
        return None;
    };
    let room_oct = room_shape
        .bounding_octagon()
        .expect("a live room shape has a bounding octagon");

    let mut result = FortyfiveRoomNeighbours {
        from_room: from_room_key,
        completed_room,
        room_shape: room_oct,
        sorted_neighbours: SortedNeighbours::default(),
        edge_interior_touches_obstacle: [false; 8],
    };

    let layer = ctx.room_layer(from_room_key);
    let mut overlapping_objects = ctx.overlapping_entries(&room_shape, layer);

    // Sort the overlapping objects deterministically to ensure parity
    // with v1.9 (`:104-113`).
    overlapping_objects.sort_by(|a, b| compare_tree_entries(a, b, |key| ctx.object_id(key)));

    for current_entry in overlapping_objects {
        if ctx.tree_object_room(current_entry.object_key) == Some(from_room_key) {
            // Java `currentObject == room`.
            continue;
        }
        if ctx.room_is_complete_free_space(completed_room)
            && !ctx.is_trace_obstacle(current_entry.object_key, net_number)
        {
            // Java :122-126 — target doors are created NOW (the 45
            // -degree arm's deferred-door difference).
            calculate_target_doors(ctx, completed_room, &current_entry, net_number);
            continue;
        }
        let Some(current_shape) = ctx.tree_shape(
            current_entry.object_key,
            current_entry.shape_index_in_object,
        ) else {
            // Java NPE arm — unreachable for live entries.
            continue;
        };
        let current_oct = current_shape
            .bounding_octagon()
            .expect("a live tree shape has a bounding octagon");
        let intersection = room_oct.intersection(&current_oct);
        let dimension = octagon_dimension(&intersection);
        if dimension > 1 && ctx.room_is_obstacle(completed_room) {
            // Java :132-143 — the 2-dim `continue` is gated on the
            // COMPLETED room kind, not on the entry kind: behind an
            // ObstacleExpansionRoom, items AND rooms are both skipped
            // (only a routable same-net ITEM receives its overlap door
            // first); with a free-space completed room (every routed
            // completion) 2-dim overlaps fall through to
            // addSortedNeighbour (the F2 room 6 / room 3 door is a
            // dimension-2 room overlap — capture-proven).
            if ctx.is_item(current_entry.object_key)
                && ctx.item_is_routable(current_entry.object_key)
                && let Some(current_overlap_room) = ctx.item_expansion_room(
                    current_entry.object_key,
                    current_entry.shape_index_in_object,
                )
            {
                create_overlap_door(ctx, completed_room, current_overlap_room);
            }
            continue;
        }
        if dimension < 0 {
            // may happen at a corner from 2 diagonal lines with non
            // integer coordinates (--.5, ---.5).
            continue;
        }
        result.add_sorted_neighbour(
            current_oct,
            intersection,
            ctx.object_id(current_entry.object_key),
        );
        if dimension > 0 {
            // Java :149-169 — "make sure, that there is a door to the
            // neighbour room": a tree-room neighbour resolves directly;
            // a routable ITEM resolves to its (lazily created)
            // ObstacleExpansionRoom ("expand the item for ripup and
            // pushing purposes"). ObstacleExpansionRoom is NOT a
            // SearchTreeObject, so this branch is the ONLY walk-time
            // origin of room↔obstacle doors — the T7 witness door
            // 14398 (free-space room 2 ↔ wire-14 corner 0) is created
            // exactly here. Non-routable items get no door;
            // insertDoorOk gates the obstacle pairs itself
            // (sharesNet / trace-parallelism).
            let neighbour_room =
                if let Some(room_key) = ctx.tree_object_room(current_entry.object_key) {
                    Some(room_key)
                } else if ctx.is_item(current_entry.object_key)
                    && ctx.item_is_routable(current_entry.object_key)
                {
                    ctx.item_expansion_room(
                        current_entry.object_key,
                        current_entry.shape_index_in_object,
                    )
                } else {
                    None
                };
            if let Some(neighbour_room) = neighbour_room
                && insert_door_ok_pair(
                    ctx,
                    completed_room,
                    neighbour_room,
                    &oct_tile(&intersection),
                )
            {
                // Java :164 — the 2-ARG door constructor (`:35-39`):
                // the dimension is the intersection dimension of the
                // two ROOM shapes.
                let door = ExpansionDoor::new_between(
                    ctx.room_id(completed_room),
                    ctx.room_id(neighbour_room),
                    &ctx.room_shape(completed_room),
                    &ctx.room_shape(neighbour_room),
                );
                ctx.attach_door(completed_room, door.clone());
                ctx.attach_door(neighbour_room, door);
            }
        }
    }
    Some(result)
}

/// Java `CompleteFreeSpaceExpansionRoom.calculateTargetDoors`
/// (`CompleteFreeSpaceExpansionRoom.java:132-149`) — the 45-degree
/// arm's per-entry variant. NOTE `setNetDependent()` fires for EVERY
/// entry BEFORE the Connectable test (the base arm only sets it when
/// the whole own-net list is non-empty).
fn calculate_target_doors(
    ctx: &mut impl NeighbourEngine,
    completed_room: u64,
    entry: &TreeEntry,
    net_number: i32,
) {
    ctx.set_net_dependent(completed_room);
    if !ctx.item_is_connectable(entry.object_key) {
        return;
    }
    if !ctx.item_contains_net(entry.object_key, net_number) {
        return;
    }
    let Some(current_connection_shape) =
        ctx.trace_connection_shape(entry.object_key, entry.shape_index_in_object)
    else {
        return;
    };
    if ctx
        .room_shape(completed_room)
        .intersects(&current_connection_shape)
    {
        let room_id = ctx.room_id(completed_room);
        if let Some(item_shape) = ctx.tree_shape(entry.object_key, entry.shape_index_in_object) {
            let target_door = super::target_door::TargetItemExpansionDoor::new(
                entry.object_key,
                entry.shape_index_in_object,
                Some(room_id),
                &item_shape,
                Some(&ctx.room_shape(completed_room)),
            );
            ctx.add_target_door(completed_room, target_door);
        }
    }
}

/// Java `removeNotTouchingBorderLines` (`:174-243`) — unbounded half
/// planes replace every border line whose interior the walk left
/// obstacle-free. The `edge_interior_touches_obstacle` flags are the
/// ONLY survival criterion; Java has NO contained-shape survival (the
/// restart room's containment comes from the tryRemoveEdgeLine ignore
/// pair plus the completeShape restraint of the enlarged shape).
fn remove_not_touching_border_lines(
    room_oct: &IntOctagon,
    edge_interior_touches_obstacle: &[bool; 8],
) -> IntOctagon {
    let left_x = if edge_interior_touches_obstacle[6] {
        room_oct.left_x
    } else {
        -CRIT_INT
    };
    let bottom_y = if edge_interior_touches_obstacle[0] {
        room_oct.bottom_y
    } else {
        -CRIT_INT
    };
    let right_x = if edge_interior_touches_obstacle[2] {
        room_oct.right_x
    } else {
        CRIT_INT
    };
    let top_y = if edge_interior_touches_obstacle[4] {
        room_oct.top_y
    } else {
        CRIT_INT
    };
    let upper_left_diagonal_x = if edge_interior_touches_obstacle[5] {
        room_oct.upper_left_diagonal_x
    } else {
        -CRIT_INT
    };
    let lower_right_diagonal_x = if edge_interior_touches_obstacle[1] {
        room_oct.lower_right_diagonal_x
    } else {
        CRIT_INT
    };
    let lower_left_diagonal_x = if edge_interior_touches_obstacle[7] {
        room_oct.lower_left_diagonal_x
    } else {
        -CRIT_INT
    };
    let upper_right_diagonal_x = if edge_interior_touches_obstacle[3] {
        room_oct.upper_right_diagonal_x
    } else {
        CRIT_INT
    };
    IntOctagon::new(
        left_x,
        bottom_y,
        right_x,
        top_y,
        upper_left_diagonal_x,
        lower_right_diagonal_x,
        lower_left_diagonal_x,
        upper_right_diagonal_x,
    )
    .normalize()
}

/// The 45-degree sorter state (Java `Sorted45DegreeRoomNeighbours`
/// fields).
pub(crate) struct FortyfiveRoomNeighbours {
    from_room: u64,
    completed_room: u64,
    room_shape: IntOctagon,
    sorted_neighbours: SortedNeighbours<FortyfiveSortedRoomNeighbour>,
    edge_interior_touches_obstacle: [bool; 8],
}

impl FortyfiveRoomNeighbours {
    /// Java `addSortedNeighbour` (`:245-252`) — entries whose sorting
    /// failed (`lastTouchingSide == -1`, room contained in the
    /// neighbour) are NOT added.
    fn add_sorted_neighbour(
        &mut self,
        neighbour_shape: IntOctagon,
        intersection: IntOctagon,
        object_id: i32,
    ) {
        let neighbour = FortyfiveSortedRoomNeighbour::new(
            neighbour_shape,
            intersection,
            &self.room_shape,
            &mut self.edge_interior_touches_obstacle,
            object_id,
        );
        if neighbour.last_touching_side >= 0 {
            self.sorted_neighbours
                .add(neighbour, FortyfiveSortedRoomNeighbour::compare_to);
        }
    }

    /// Java `tryRemoveEdgeLine` (`:316-431`).
    fn try_remove_edge_line(&self, ctx: &mut impl NeighbourEngine) -> bool {
        if !ctx.room_is_incomplete(self.from_room) {
            return false;
        }
        let room_shape = ctx.room_shape(self.from_room);
        let TileShape::RegularTileShape(RegularTileShape::IntOctagon(room_oct)) = room_shape else {
            // Java :320-325 — warn "IntOctagon expected".
            return false;
        };
        let room_area = room_oct.area();

        let mut try_remove_edge_lines = false;
        for i in 0..8i32 {
            if !self.edge_interior_touches_obstacle[i as usize] {
                let prev_corner = self.room_shape.corner(i).to_float();
                let next_corner = self.room_shape.corner((i + 1) % 8).to_float();
                if prev_corner.distance_square(&next_corner) > 1.0 {
                    try_remove_edge_lines = true;
                    break;
                }
            }
        }

        if !try_remove_edge_lines {
            return false;
        }
        // Touching neighbour missing at the edge side with index
        // removeEdgeNo. Remove the edge line and restart the
        // algorithm. (Java :343-371 has diagnostic FRLogger traces
        // here, omitted — log-only, D12.) The contained shape is only
        // the completeShape containment argument: the enlarged shape's
        // border survival comes solely from the walk's edge flags and
        // the restraint at the tree shapes — Java has no
        // contained-touch survival check.
        let Some(contained) = ctx.room_contained_shape(self.from_room) else {
            return false;
        };
        let enlarged_oct =
            remove_not_touching_border_lines(&room_oct, &self.edge_interior_touches_obstacle);
        let layer = ctx.room_layer(self.from_room);

        // Java :373-394 — the biggest 2-dim door to a completed
        // free-space room becomes the ignore pair of completeShape.
        let completed_id = ctx.room_id(self.completed_room);
        let mut ignore_shape: Option<TileShape> = None;
        let mut ignore_object: Option<u64> = None;
        let mut max_door_area = 0.0f64;
        for current_door in ctx.room_doors(self.completed_room) {
            // insert the overlapping doors with
            // CompleteFreeSpaceExpansionRooms for the information in
            // complete_shape about the objects to ignore.
            if current_door.dimension == 2
                && let Some(other_id) = current_door.other_room_id(completed_id)
                && let Some(other_key) = ctx.room_key_of_door(other_id, &current_door)
                && ctx.room_is_complete_free_space(other_key)
            {
                let current_door_shape = ExpansionDoor::shape_between(
                    &ctx.room_shape(self.completed_room),
                    &ctx.room_shape(other_key),
                );
                let current_door_area = current_door_shape.area();
                if current_door_area > max_door_area {
                    max_door_area = current_door_area;
                    ignore_shape = Some(current_door_shape);
                    ignore_object = Some(other_key);
                }
            }
        }
        let enlarged_shape = oct_tile(&enlarged_oct);
        let new_rooms = ctx.complete_shape(
            Some(&enlarged_shape),
            Some(&contained),
            layer,
            ignore_object,
            ignore_shape.as_ref(),
        );
        if new_rooms.len() == 1 {
            // Check, that the area increases to prevent endless loop.
            let new_room = &new_rooms[0];
            if new_room.shape.area() > room_area {
                ctx.set_incomplete_shape(
                    self.from_room,
                    new_room.shape.clone(),
                    new_room.contained_shape.clone(),
                );
                return true;
            }
        }
        false
    }

    /// Java `calculateEdgeIncompleteRoomsOfObstacleExpansionRoom`
    /// (`:255-310`) — an incomplete room behind every side from
    /// `from_side_index` to `to_side_index` whose corner test passes.
    /// JAVA QUIRK (load-bearing, T7-proven): `currentCorner` is
    /// assigned ONCE before the loop (`:264`) and never updated —
    /// `:269` is its only read — so the degenerate-side test compares
    /// `corner(from_side_index)` against `corner(k + 1)` on EVERY
    /// iteration. At the final side of the full walk `(0, 7)` the
    /// next corner wraps to `corner(0) == currentCorner` and the last
    /// wedge is ALWAYS skipped: Java's every-side arm creates at most
    /// 7 border rooms, never the `to_side_index` one. Do not
    /// "refresh" the corner in the loop.
    fn calculate_edge_incomplete_rooms(
        &self,
        from_side_index: i32,
        to_side_index: i32,
        ctx: &mut impl NeighbourEngine,
    ) {
        if !ctx.room_is_obstacle(self.from_room) {
            // Java :257-262 — warn.
            return;
        }
        let board_bounding_oct = ctx.board_bounding_octagon();
        // Java `:264` — assigned once, never refreshed (see the quirk
        // note above).
        let current_corner = self.room_shape.corner(from_side_index);
        let mut current_side_index = from_side_index;
        loop {
            let next_side_no = (current_side_index + 1) % 8;
            let next_corner = self.room_shape.corner(next_side_no);
            if current_corner != next_corner {
                let mut left_x = board_bounding_oct.left_x;
                let mut bottom_y = board_bounding_oct.bottom_y;
                let mut right_x = board_bounding_oct.right_x;
                let mut top_y = board_bounding_oct.top_y;
                let mut upper_left_diagonal_x = board_bounding_oct.upper_left_diagonal_x;
                let mut lower_right_diagonal_x = board_bounding_oct.lower_right_diagonal_x;
                let mut lower_left_diagonal_x = board_bounding_oct.lower_left_diagonal_x;
                let mut upper_right_diagonal_x = board_bounding_oct.upper_right_diagonal_x;
                match current_side_index {
                    0 => top_y = self.room_shape.bottom_y,
                    1 => upper_left_diagonal_x = self.room_shape.lower_right_diagonal_x,
                    2 => left_x = self.room_shape.right_x,
                    3 => lower_left_diagonal_x = self.room_shape.upper_right_diagonal_x,
                    4 => bottom_y = self.room_shape.top_y,
                    5 => lower_right_diagonal_x = self.room_shape.upper_left_diagonal_x,
                    6 => right_x = self.room_shape.left_x,
                    7 => upper_right_diagonal_x = self.room_shape.lower_left_diagonal_x,
                    _ => {
                        // Java :287-292 — warn "illegal".
                        return;
                    }
                }
                insert_incomplete_room(
                    ctx,
                    self.completed_room,
                    self.from_room,
                    &self.room_shape,
                    left_x,
                    bottom_y,
                    right_x,
                    top_y,
                    upper_left_diagonal_x,
                    lower_right_diagonal_x,
                    lower_left_diagonal_x,
                    upper_right_diagonal_x,
                );
            }
            if current_side_index == to_side_index {
                break;
            }
            current_side_index = next_side_no;
        }
    }

    /// Java `calculateNewIncompleteRoomsForObstacleExpansionRoom`
    /// (`:475-609`) — the obstacle-room special walk.
    fn calculate_new_incomplete_rooms_for_obstacle_room(
        &self,
        prev_neighbour: &FortyfiveSortedRoomNeighbour,
        next_neighbour: &FortyfiveSortedRoomNeighbour,
        ctx: &mut impl NeighbourEngine,
    ) {
        let from_side_index = prev_neighbour.last_touching_side;
        let to_side_index = next_neighbour.first_touching_side;
        if from_side_index == to_side_index && !std::ptr::eq(prev_neighbour, next_neighbour) {
            // no return in case of only 1 neighbour.
            return;
        }
        let board_bounding_oct = ctx.board_bounding_octagon();

        // insert the new incomplete room from prevNeighbour to the
        // next corner of the room shape.
        let mut left_x = board_bounding_oct.left_x;
        let mut bottom_y = board_bounding_oct.bottom_y;
        let mut right_x = board_bounding_oct.right_x;
        let mut top_y = board_bounding_oct.top_y;
        let mut upper_left_diagonal_x = board_bounding_oct.upper_left_diagonal_x;
        let mut lower_right_diagonal_x = board_bounding_oct.lower_right_diagonal_x;
        let mut lower_left_diagonal_x = board_bounding_oct.lower_left_diagonal_x;
        let mut upper_right_diagonal_x = board_bounding_oct.upper_right_diagonal_x;
        match from_side_index {
            0 => {
                top_y = self.room_shape.bottom_y;
                upper_left_diagonal_x = prev_neighbour.intersection.lower_right_diagonal_x;
            }
            1 => {
                upper_left_diagonal_x = self.room_shape.lower_right_diagonal_x;
                left_x = prev_neighbour.intersection.right_x;
            }
            2 => {
                left_x = self.room_shape.right_x;
                lower_left_diagonal_x = prev_neighbour.intersection.upper_right_diagonal_x;
            }
            3 => {
                lower_left_diagonal_x = self.room_shape.upper_right_diagonal_x;
                bottom_y = prev_neighbour.intersection.top_y;
            }
            4 => {
                bottom_y = self.room_shape.top_y;
                lower_right_diagonal_x = prev_neighbour.intersection.upper_left_diagonal_x;
            }
            5 => {
                lower_right_diagonal_x = self.room_shape.upper_left_diagonal_x;
                right_x = prev_neighbour.intersection.left_x;
            }
            6 => {
                right_x = self.room_shape.left_x;
                upper_right_diagonal_x = prev_neighbour.intersection.lower_left_diagonal_x;
            }
            7 => {
                upper_right_diagonal_x = self.room_shape.lower_left_diagonal_x;
                top_y = prev_neighbour.intersection.bottom_y;
            }
            _ => {}
        }
        insert_incomplete_room(
            ctx,
            self.completed_room,
            self.from_room,
            &self.room_shape,
            left_x,
            bottom_y,
            right_x,
            top_y,
            upper_left_diagonal_x,
            lower_right_diagonal_x,
            lower_left_diagonal_x,
            upper_right_diagonal_x,
        );

        // insert the new incomplete room from prevNeighbour to the
        // next corner of the room shape. (The Java comment repeats;
        // this is the toNeighbour side.)
        let mut left_x = board_bounding_oct.left_x;
        let mut bottom_y = board_bounding_oct.bottom_y;
        let mut right_x = board_bounding_oct.right_x;
        let mut top_y = board_bounding_oct.top_y;
        let mut upper_left_diagonal_x = board_bounding_oct.upper_left_diagonal_x;
        let mut lower_right_diagonal_x = board_bounding_oct.lower_right_diagonal_x;
        let mut lower_left_diagonal_x = board_bounding_oct.lower_left_diagonal_x;
        let mut upper_right_diagonal_x = board_bounding_oct.upper_right_diagonal_x;
        match to_side_index {
            0 => {
                top_y = self.room_shape.bottom_y;
                upper_right_diagonal_x = next_neighbour.intersection.lower_left_diagonal_x;
            }
            1 => {
                upper_left_diagonal_x = self.room_shape.lower_right_diagonal_x;
                top_y = next_neighbour.intersection.bottom_y;
            }
            2 => {
                left_x = self.room_shape.right_x;
                upper_left_diagonal_x = next_neighbour.intersection.lower_right_diagonal_x;
            }
            3 => {
                lower_left_diagonal_x = self.room_shape.upper_right_diagonal_x;
                left_x = next_neighbour.intersection.right_x;
            }
            4 => {
                bottom_y = self.room_shape.top_y;
                lower_left_diagonal_x = next_neighbour.intersection.upper_right_diagonal_x;
            }
            5 => {
                lower_right_diagonal_x = self.room_shape.upper_left_diagonal_x;
                bottom_y = next_neighbour.intersection.top_y;
            }
            6 => {
                right_x = self.room_shape.left_x;
                lower_right_diagonal_x = next_neighbour.intersection.upper_left_diagonal_x;
            }
            7 => {
                upper_right_diagonal_x = self.room_shape.lower_left_diagonal_x;
                right_x = next_neighbour.intersection.left_x;
            }
            _ => {}
        }
        insert_incomplete_room(
            ctx,
            self.completed_room,
            self.from_room,
            &self.room_shape,
            left_x,
            bottom_y,
            right_x,
            top_y,
            upper_left_diagonal_x,
            lower_right_diagonal_x,
            lower_left_diagonal_x,
            upper_right_diagonal_x,
        );

        // Insert the new incomplete rooms on the intermediate free
        // sides of the obstacle expansion room.
        let current_from_side_no = (from_side_index + 1) % 8;
        if current_from_side_no == to_side_index {
            return;
        }
        let current_to_side_no = (to_side_index + 7) % 8;
        self.calculate_edge_incomplete_rooms(current_from_side_no, current_to_side_no, ctx);
    }

    /// Java `calculateNewIncompleteRooms` (`:611-797`).
    fn calculate_new_incomplete_rooms(&self, ctx: &mut impl NeighbourEngine) {
        let board_bounding_oct = ctx.board_bounding_octagon();
        let count = self.sorted_neighbours.len();
        if ctx.room_is_obstacle(self.from_room) && count == 1 {
            // ObstacleExpansionRoom has only 1 neighbour
            let prev_neighbour = self.sorted_neighbours.last();
            self.calculate_new_incomplete_rooms_for_obstacle_room(
                prev_neighbour,
                prev_neighbour,
                ctx,
            );
            return;
        }

        for index in 0..count {
            let next_neighbour = self.sorted_neighbours.get(index);
            let prev_neighbour = if index == 0 {
                self.sorted_neighbours.last()
            } else {
                self.sorted_neighbours.get(index - 1)
            };
            let insert_room = if ctx.room_is_obstacle(self.completed_room) && count == 2 {
                // check, if this site is touching or open.
                let intersection = next_neighbour
                    .intersection
                    .intersection(&prev_neighbour.intersection);
                if octagon_is_empty(&intersection) {
                    true
                } else if octagon_dimension(&intersection) >= 1 {
                    false
                } else {
                    // touch at a corner of the room shape
                    if prev_neighbour.last_touching_side == next_neighbour.first_touching_side {
                        // touch along the side of the room shape
                        false
                    } else {
                        prev_neighbour.last_touching_side
                            != (next_neighbour.first_touching_side + 1) % 8
                    }
                }
            } else {
                // the 2 neighbours do not touch. (Capture note, F2
                // final walk: the four-entry sorted list produces four
                // pairs — keepout(4,1)->tile(1,2) and pad(3,4)->
                // keepout(4,1) insert (rows 1002583643 / -1219568412);
                // tile(1,2)->room3(1,4) and room3->pad are rejected
                // right here by the intersects gate, their overlaps
                // touching at an edge and a corner. The room3 (1,4)
                // entry is what dissolves the old tile->pad wedge-pair
                // phantom — no extra pair gate exists.)
                !next_neighbour
                    .intersection
                    .intersects(&prev_neighbour.intersection)
            };

            if !insert_room {
                continue;
            }
            // create a door to a new incomplete expansion room between
            // the last corner of the previous neighbour and the first
            // corner of the current neighbour

            if ctx.room_is_obstacle(self.from_room)
                && next_neighbour.first_touching_side != prev_neighbour.last_touching_side
            {
                self.calculate_new_incomplete_rooms_for_obstacle_room(
                    prev_neighbour,
                    next_neighbour,
                    ctx,
                );
                continue;
            }
            let mut lx = board_bounding_oct.left_x;
            let mut ly = board_bounding_oct.bottom_y;
            let mut rx = board_bounding_oct.right_x;
            let mut uy = board_bounding_oct.top_y;
            let mut ulx = board_bounding_oct.upper_left_diagonal_x;
            let mut lrx = board_bounding_oct.lower_right_diagonal_x;
            let mut llx = board_bounding_oct.lower_left_diagonal_x;
            let mut urx = board_bounding_oct.upper_right_diagonal_x;

            let prev_is = &prev_neighbour.intersection;
            let next_is = &next_neighbour.intersection;
            match next_neighbour.first_touching_side {
                0 => {
                    if prev_is.lower_left_diagonal_x < next_is.lower_left_diagonal_x {
                        urx = next_is.lower_left_diagonal_x;
                        uy = prev_is.bottom_y;
                        if prev_neighbour.last_touching_side == 0 {
                            ulx = prev_is.lower_right_diagonal_x;
                        }
                    } else if prev_is.lower_left_diagonal_x > next_is.lower_left_diagonal_x {
                        rx = next_is.left_x;
                        urx = prev_is.lower_left_diagonal_x;
                    } else {
                        // prev.intersection.llx == next.intersection.llx
                        urx = next_is.lower_left_diagonal_x;
                    }
                }
                1 => {
                    if prev_is.bottom_y < next_is.bottom_y {
                        uy = next_is.bottom_y;
                        ulx = prev_is.lower_right_diagonal_x;
                        if prev_neighbour.last_touching_side == 1 {
                            lx = prev_is.right_x;
                        }
                    } else if prev_is.bottom_y > next_is.bottom_y {
                        uy = prev_is.bottom_y;
                        urx = next_is.lower_left_diagonal_x;
                    } else {
                        uy = next_is.bottom_y;
                    }
                }
                2 => {
                    if prev_is.lower_right_diagonal_x > next_is.lower_right_diagonal_x {
                        ulx = next_is.lower_right_diagonal_x;
                        lx = prev_is.right_x;
                        if prev_neighbour.last_touching_side == 2 {
                            llx = prev_is.upper_right_diagonal_x;
                        }
                    } else if prev_is.lower_right_diagonal_x < next_is.lower_right_diagonal_x {
                        uy = next_is.bottom_y;
                        ulx = prev_is.lower_right_diagonal_x;
                    } else {
                        ulx = next_is.lower_right_diagonal_x;
                    }
                }
                3 => {
                    if prev_is.right_x > next_is.right_x {
                        lx = next_is.right_x;
                        llx = prev_is.upper_right_diagonal_x;
                        if prev_neighbour.last_touching_side == 3 {
                            ly = prev_is.top_y;
                        }
                    } else if prev_is.right_x < next_is.right_x {
                        lx = prev_is.right_x;
                        ulx = next_is.lower_right_diagonal_x;
                    } else {
                        lx = next_is.right_x;
                    }
                }
                4 => {
                    if prev_is.upper_right_diagonal_x > next_is.upper_right_diagonal_x {
                        llx = next_is.upper_right_diagonal_x;
                        ly = prev_is.top_y;
                        if prev_neighbour.last_touching_side == 4 {
                            lrx = prev_is.upper_left_diagonal_x;
                        }
                    } else if prev_is.upper_right_diagonal_x < next_is.upper_right_diagonal_x {
                        lx = next_is.right_x;
                        llx = prev_is.upper_right_diagonal_x;
                    } else {
                        llx = next_is.upper_right_diagonal_x;
                    }
                }
                5 => {
                    if prev_is.top_y > next_is.top_y {
                        ly = next_is.top_y;
                        lrx = prev_is.upper_left_diagonal_x;
                        if prev_neighbour.last_touching_side == 5 {
                            rx = prev_is.left_x;
                        }
                    } else if prev_is.top_y < next_is.top_y {
                        ly = prev_is.top_y;
                        llx = next_is.upper_right_diagonal_x;
                    } else {
                        ly = next_is.top_y;
                    }
                }
                6 => {
                    if prev_is.upper_left_diagonal_x < next_is.upper_left_diagonal_x {
                        lrx = next_is.upper_left_diagonal_x;
                        rx = prev_is.left_x;
                        if prev_neighbour.last_touching_side == 6 {
                            urx = prev_is.lower_left_diagonal_x;
                        }
                    } else if prev_is.upper_left_diagonal_x > next_is.upper_left_diagonal_x {
                        ly = next_is.top_y;
                        lrx = prev_is.upper_left_diagonal_x;
                    } else {
                        lrx = next_is.upper_left_diagonal_x;
                    }
                }
                7 => {
                    if prev_is.left_x < next_is.left_x {
                        rx = next_is.left_x;
                        urx = prev_is.lower_left_diagonal_x;
                        if prev_neighbour.last_touching_side == 7 {
                            uy = prev_is.bottom_y;
                        }
                    } else if prev_is.left_x > next_is.left_x {
                        rx = prev_is.left_x;
                        lrx = next_is.upper_left_diagonal_x;
                    } else {
                        rx = next_is.left_x;
                    }
                }
                _ => {
                    // Java :788-790 — warn "illegal touching side".
                }
            }
            insert_incomplete_room(
                ctx,
                self.completed_room,
                self.from_room,
                &self.room_shape,
                lx,
                ly,
                rx,
                uy,
                ulx,
                lrx,
                llx,
                urx,
            );
        }
    }
}

/// Java `insertIncompleteRoom` (`:438-473`) — the new incomplete room
/// behind `nextNeighbour`'s touching side, cut to the board octagon.
#[allow(clippy::too_many_arguments)]
fn insert_incomplete_room(
    ctx: &mut impl NeighbourEngine,
    completed_room: u64,
    from_room: u64,
    room_shape: &IntOctagon,
    left_x: i32,
    bottom_y: i32,
    right_x: i32,
    top_y: i32,
    upper_left_diagonal_x: i32,
    lower_right_diagonal_x: i32,
    lower_left_diagonal_x: i32,
    upper_right_diagonal_x: i32,
) {
    let new_incomplete_room_shape = IntOctagon::new(
        left_x,
        bottom_y,
        right_x,
        top_y,
        upper_left_diagonal_x,
        lower_right_diagonal_x,
        lower_left_diagonal_x,
        upper_right_diagonal_x,
    )
    .normalize();
    if octagon_dimension(&new_incomplete_room_shape) != 2 {
        return;
    }
    let new_contained_shape = room_shape.intersection(&new_incomplete_room_shape);
    if octagon_is_empty(&new_contained_shape) {
        return;
    }
    let door_dimension = octagon_dimension(&new_contained_shape);
    if door_dimension > 0 {
        let layer = ctx.room_layer(from_room);
        let new_room = ctx.add_incomplete_expansion_room(
            oct_tile(&new_incomplete_room_shape),
            layer,
            oct_tile(&new_contained_shape),
        );
        // Java :467 — the explicit-dimension door constructor.
        let door = ExpansionDoor::new(
            ctx.room_id(completed_room),
            ctx.room_id(new_room),
            door_dimension,
        );
        ctx.attach_door(completed_room, door.clone());
        ctx.attach_door(new_room, door);
    }
}

/// The 45-degree sorter's neighbour entry (Java inner class
/// `SortedRoomNeighbour`, `:803-981`).
#[derive(Clone, Debug)]
pub(crate) struct FortyfiveSortedRoomNeighbour {
    /// Java `shape` (the neighbour octagon) — stored by the Java inner
    /// class but never read there; kept for field parity.
    #[allow(dead_code)]
    shape: IntOctagon,
    /// Java `intersection`.
    intersection: IntOctagon,
    /// Java `firstTouchingSide`.
    first_touching_side: i32,
    /// Java `lastTouchingSide`.
    last_touching_side: i32,
    /// Java `searchTreeObject.getId()` — the compareTo tie-break after
    /// equal geometry (Java `:975-978`).
    object_id: i32,
}

impl FortyfiveSortedRoomNeighbour {
    /// Java constructor (`:825-916`) — the first/last touching side
    /// detection against the room octagon plus the
    /// `edgeInteriorTouchesObstacle` updates. `room_shape` is the
    /// sorter's `completedRoom.getShape().boundingOctagon()`.
    /// NOTE the comparator ends in the object-id tie-break (Java
    /// `:975-978`), so comparator-equal now means same object id AND
    /// identical geometry — a duplicate tree shape of ONE object —
    /// and Java's `TreeSet` still DROPS such entries ("only 1
    /// obstacle is needed"); the port must return `Ordering::Equal`
    /// there too.
    fn new(
        shape: IntOctagon,
        intersection: IntOctagon,
        room_shape: &IntOctagon,
        edge_interior_touches_obstacle: &mut [bool; 8],
        object_id: i32,
    ) -> Self {
        let is = &intersection;
        let first_touching_side = if is.bottom_y == room_shape.bottom_y
            && is.lower_left_diagonal_x > room_shape.lower_left_diagonal_x
        {
            0
        } else if is.lower_right_diagonal_x == room_shape.lower_right_diagonal_x
            && is.bottom_y > room_shape.bottom_y
        {
            1
        } else if is.right_x == room_shape.right_x
            && is.lower_right_diagonal_x < room_shape.lower_right_diagonal_x
        {
            2
        } else if is.upper_right_diagonal_x == room_shape.upper_right_diagonal_x
            && is.right_x < room_shape.right_x
        {
            3
        } else if is.top_y == room_shape.top_y
            && is.upper_right_diagonal_x < room_shape.upper_right_diagonal_x
        {
            4
        } else if is.upper_left_diagonal_x == room_shape.upper_left_diagonal_x
            && is.top_y < room_shape.top_y
        {
            5
        } else if is.left_x == room_shape.left_x
            && is.upper_left_diagonal_x > room_shape.upper_left_diagonal_x
        {
            6
        } else if is.lower_left_diagonal_x == room_shape.lower_left_diagonal_x
            && is.left_x > room_shape.left_x
        {
            7
        } else {
            // the roomShape may be contained in the neighbourShape
            return Self {
                shape,
                intersection,
                first_touching_side: -1,
                last_touching_side: -1,
                object_id,
            };
        };

        let last_touching_side = if is.lower_left_diagonal_x == room_shape.lower_left_diagonal_x
            && is.bottom_y > room_shape.bottom_y
        {
            7
        } else if is.left_x == room_shape.left_x
            && is.lower_left_diagonal_x > room_shape.lower_left_diagonal_x
        {
            6
        } else if is.upper_left_diagonal_x == room_shape.upper_left_diagonal_x
            && is.left_x > room_shape.left_x
        {
            5
        } else if is.top_y == room_shape.top_y
            && is.upper_left_diagonal_x > room_shape.upper_left_diagonal_x
        {
            // Java :871-873 — the last chain's top-side arm: equality
            // on the side's own line plus the NEXT side's bounding
            // coordinate strictly past the room's (the same predicate
            // shape as arms 7/6/5 above). Capture witness (F2, every
            // walk on the W1/W2/W3 room shapes): the NET_A pad's
            // degenerate top-border overlap (y = 495000, x in
            // [75000, 125000]) fires this arm — ulx -420000 > room
            // ulx -489900 — with last_touching_side 4, while the
            // boundary-keepout segment fails the strict ulx test and
            // falls through to the lower arms, preserving the
            // capture's two-restart chain (ids 4, 5 burn) and the
            // top-left wedge room (-1219568412, bounded by the pad's
            // lower-left diagonal through (75000, 495000)).
            4
        } else if is.upper_right_diagonal_x == room_shape.upper_right_diagonal_x
            && is.top_y < room_shape.top_y
        {
            3
        } else if is.right_x == room_shape.right_x
            && is.upper_right_diagonal_x < room_shape.upper_right_diagonal_x
        {
            2
        } else if is.lower_right_diagonal_x == room_shape.lower_right_diagonal_x
            && is.right_x < room_shape.right_x
        {
            1
        } else if is.bottom_y == room_shape.bottom_y
            && is.lower_right_diagonal_x < room_shape.lower_right_diagonal_x
        {
            0
        } else {
            // the roomShape may be contained in the neighbourShape
            return Self {
                shape,
                intersection,
                first_touching_side: -1,
                last_touching_side: -1,
                object_id,
            };
        };

        let neighbour = Self {
            shape,
            intersection,
            first_touching_side,
            last_touching_side,
            object_id,
        };

        // Java :892-915 — mark the border sides whose INTERIOR the
        // neighbour touches (corner-only touches stay unmarked).
        let mut next_side_no = neighbour.first_touching_side;
        loop {
            let current_side_index = next_side_no;
            next_side_no = (next_side_no + 1) % 8;
            if !edge_interior_touches_obstacle[current_side_index as usize] {
                let mut touch_only_at_corner = false;
                if current_side_index == neighbour.first_touching_side
                    && neighbour.intersection.corner(current_side_index)
                        == room_shape.corner(next_side_no)
                {
                    touch_only_at_corner = true;
                }
                if current_side_index == neighbour.last_touching_side
                    && neighbour.intersection.corner(next_side_no)
                        == room_shape.corner(current_side_index)
                {
                    touch_only_at_corner = true;
                }
                if !touch_only_at_corner {
                    edge_interior_touches_obstacle[current_side_index as usize] = true;
                }
            }
            if current_side_index == neighbour.last_touching_side {
                break;
            }
        }
        neighbour
    }

    /// Java `compareTo` (`:923-980`) — the counterclockwise order.
    /// Sides 0-3 ASCEND along the walk direction; sides 4-7 carry the
    /// REVERSED corner sign (is2 - is1) — do not "fix" the asymmetry.
    /// Geometry-equal neighbours fall through to the object-id
    /// tie-break (`:975-978`); every subtraction is a wrapping Java
    /// `int` operation.
    fn compare_to(&self, other: &Self) -> Ordering {
        if self.first_touching_side > other.first_touching_side {
            return Ordering::Greater;
        }
        if self.first_touching_side < other.first_touching_side {
            return Ordering::Less;
        }

        // now the first touch of this and other is at the same side
        let is1 = &self.intersection;
        let is2 = &other.intersection;
        let mut cmp_value: i32 = match self.first_touching_side {
            0 => is1.corner(0).x.wrapping_sub(is2.corner(0).x),
            1 => is1.corner(1).x.wrapping_sub(is2.corner(1).x),
            2 => is1.corner(2).y.wrapping_sub(is2.corner(2).y),
            3 => is1.corner(3).y.wrapping_sub(is2.corner(3).y),
            4 => is2.corner(4).x.wrapping_sub(is1.corner(4).x),
            5 => is2.corner(5).x.wrapping_sub(is1.corner(5).x),
            6 => is2.corner(6).y.wrapping_sub(is1.corner(6).y),
            7 => is2.corner(7).y.wrapping_sub(is1.corner(7).y),
            _ => 0, // Java :945-948 — warn "out of range"; equal.
        };

        if cmp_value == 0 {
            // The first touching points of this neighbour and other
            // with the room shape are equal. Compare the last
            // touching points.
            let this_touching_side_diff =
                (self.last_touching_side - self.first_touching_side + 8) % 8;
            let other_touching_side_diff =
                (other.last_touching_side - other.first_touching_side + 8) % 8;
            if this_touching_side_diff > other_touching_side_diff {
                return Ordering::Greater;
            }
            if this_touching_side_diff < other_touching_side_diff {
                return Ordering::Less;
            }
            // now the last touch of this and other is at the same side
            cmp_value = match self.last_touching_side {
                0 => is1.corner(1).x.wrapping_sub(is2.corner(1).x),
                1 => is1.corner(2).x.wrapping_sub(is2.corner(2).x),
                2 => is1.corner(3).y.wrapping_sub(is2.corner(3).y),
                3 => is1.corner(4).y.wrapping_sub(is2.corner(4).y),
                4 => is2.corner(5).x.wrapping_sub(is1.corner(5).x),
                5 => is2.corner(6).x.wrapping_sub(is1.corner(6).x),
                6 => is2.corner(7).y.wrapping_sub(is1.corner(7).y),
                7 => is2.corner(0).y.wrapping_sub(is1.corner(0).y),
                _ => 0,
            };
        }
        if cmp_value == 0 {
            // Java :975-978 — the object-id tie-break after equal
            // geometry (Java int subtraction, wrapping).
            cmp_value = self.object_id.wrapping_sub(other.object_id);
        }
        cmp_value.cmp(&0)
    }
}

/// The octagon wrapped as a `TileShape`.
fn oct_tile(octagon: &IntOctagon) -> TileShape {
    TileShape::RegularTileShape(RegularTileShape::IntOctagon(*octagon))
}

/// `IntOctagon` dimension through the `TileShape` dispatch.
fn octagon_dimension(octagon: &IntOctagon) -> i32 {
    oct_tile(octagon).dimension()
}

/// `IntOctagon` emptiness through the `TileShape` dispatch.
fn octagon_is_empty(octagon: &IntOctagon) -> bool {
    oct_tile(octagon).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An exact rectangle as an `IntOctagon` (diagonals on the corner
    /// lines): corner(0) = (left, bottom), corner(1) = (right,
    /// bottom), corner(4) = (right, top).
    fn neighbour_oct(left_x: i32, bottom_y: i32, right_x: i32, top_y: i32) -> IntOctagon {
        IntOctagon::new(
            left_x,
            bottom_y,
            right_x,
            top_y,
            left_x - top_y,
            right_x - bottom_y,
            left_x + bottom_y,
            right_x + top_y,
        )
        .normalize()
    }

    fn neighbour_at(shape: IntOctagon, first: i32, last: i32) -> FortyfiveSortedRoomNeighbour {
        FortyfiveSortedRoomNeighbour {
            shape,
            intersection: shape,
            first_touching_side: first,
            last_touching_side: last,
            object_id: 0,
        }
    }

    /// The diagonal-band sign pin (the ONLY guard of the sides 4-7
    /// reversed corner sign in `compare_to`): the capture pins cannot
    /// see a sides 4-7 sign flip — no F1/F2 walk neighbours share a
    /// first touching side in that band (mutation-verified) — while a
    /// sides 0-3 flip kills them via the F2 side-1 tile/room3 tie.
    /// Sides 0-3 sort ASCENDING along the walk coordinate; sides 4-7
    /// sort DESCENDING (the counterclockwise top/left sides run
    /// against the axis).
    #[test]
    fn compare_to_diagonal_band_signs() {
        // side 0: corner(0).x = left_x — ascending.
        let a0 = neighbour_at(neighbour_oct(100, 0, 300, 200), 0, 0);
        let b0 = neighbour_at(neighbour_oct(200, 0, 300, 200), 0, 0);
        assert_eq!(a0.compare_to(&b0), Ordering::Less, "side 0: ascending");

        // side 4: corner(4).x = right_x — DESCENDING (reversed sign).
        let a4 = neighbour_at(neighbour_oct(0, 0, 800, 200), 4, 4);
        let b4 = neighbour_at(neighbour_oct(0, 0, 500, 200), 4, 4);
        assert_eq!(a4.compare_to(&b4), Ordering::Less, "side 4: descending");

        // Last-side band arm 0 (equal first corners): corner(1).x =
        // right_x — ascending.
        let al = neighbour_at(neighbour_oct(100, 0, 700, 200), 0, 0);
        let bl = neighbour_at(neighbour_oct(100, 0, 400, 200), 0, 0);
        assert_eq!(
            al.compare_to(&bl),
            Ordering::Greater,
            "last band arm 0: ascending"
        );
    }

    /// The object-id tie-break pin (MINOR-4, quality round): geometry
    /// -equal neighbours with DIFFERENT object ids order by id (Java
    /// `:975-978`), while identical ids on identical geometry compare
    /// Equal — the condition under which Java's `TreeSet` drops a
    /// duplicate tree shape of ONE object ("only 1 obstacle is
    /// needed").
    #[test]
    fn compare_to_object_id_tie_break() {
        let low = neighbour_at(neighbour_oct(100, 0, 300, 200), 0, 0);
        let mut high = neighbour_at(neighbour_oct(100, 0, 300, 200), 0, 0);
        high.object_id = 5;
        assert_eq!(low.compare_to(&high), Ordering::Less, "id 0 < id 5");
        assert_eq!(high.compare_to(&low), Ordering::Greater, "id 5 > id 0");
        assert_eq!(
            low.compare_to(&neighbour_at(neighbour_oct(100, 0, 300, 200), 0, 0)),
            Ordering::Equal,
            "same id, identical geometry: Equal (TreeSet drop)"
        );
    }
}
