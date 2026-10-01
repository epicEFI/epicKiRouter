//! The PRODUCTION completion seam (T6): Java
//! `AutorouteEngine.completeExpansionRoom` (`AutorouteEngine.java:418-522`),
//! `addCompleteRoom` (`:525-545`) and `completeNeighbourRooms`
//! (`:567-592`), composed from the T4 completion primitives.
//!
//! Written from the Java source (the T5 `Harness::complete_expansion_room`
//! was a capture-replay mirror; it now delegates here and the T5 probe
//! burn-count pins verify THIS code).
//!
//! The structure the burn counts hang on (probe oracle
//! `rust/harness/oracle/DrillSpikeProbe.java`, `/tmp/drill_probe.rows`):
//! only the FIRST 2-dim candidate of `completeShape` is completed
//! directly; candidates 2..k are RE-completed first (`:496-498`) —
//! the first room's insertion may have consumed their region and
//! `restrainShape`'s contained-point arm drops them — and only the
//! recalc survivors are completed. Every candidate registration and
//! every surviving completed room burns one room id; the abandoned
//! recalc candidates burn theirs and are dropped by
//! [`NeighbourEngine::flush_completed_inserts`].

use epic_geometry::float_line::FloatLine;
use epic_geometry::tile_shape::TileShape;

use crate::drill::DrillEngine;
use crate::expansion::{NeighbourEngine, complete};

/// Java `addCompleteRoom` (`:525-545`) on a REGISTERED incomplete
/// room: `calculateDoors(room)` (the sorted-room-neighbour completion,
/// T4 [`complete`]), then the tree insert / restart-discard flush.
/// Java's `null`/non-2-dim guard is upstream: `complete` returns the
/// accepted key only for a surviving 2-dim room.
fn add_complete_room(ctx: &mut impl NeighbourEngine, room_key: u64) -> Option<u64> {
    let variant = ctx.tree_variant();
    let accepted = complete(ctx, room_key, variant);
    ctx.flush_completed_inserts(accepted);
    accepted
}

/// Java `completeExpansionRoom`'s candidate walk (`:449-517`) over the
/// `completeShape` output, shared by the registered-room and
/// null-shape entries. `ignore_object` / `from_door_shape` are the
/// door-scan results (`:423-434`).
fn complete_candidates(
    ctx: &mut impl DrillEngine,
    room_shape: Option<&TileShape>,
    contained_shape: Option<&TileShape>,
    layer: i32,
    ignore_object: Option<u64>,
    from_door_shape: Option<&TileShape>,
) -> Vec<u64> {
    let cells = ctx.complete_shape(
        room_shape,
        contained_shape,
        layer,
        ignore_object,
        from_door_shape,
    );
    let mut completed = Vec::new();
    let mut first_done = false;
    let mut temp_keys = Vec::new();
    for cell in cells {
        if cell.shape.dimension() != 2 {
            continue;
        }
        if !first_done {
            first_done = true;
            // The first 2-dim candidate is completed directly — Java's
            // one-element `batch`, built here WITHOUT the one-element
            // `Vec` (slice C).
            register_candidate(ctx, cell, &mut temp_keys, &mut completed);
            continue;
        }
        // The shape of the first completed room may have changed and
        // may intersect the other shapes now — re-complete
        // (`:492-498`).
        let recalc = ctx.complete_shape(
            Some(&cell.shape),
            Some(&cell.contained_shape),
            cell.layer,
            ignore_object,
            from_door_shape,
        );
        for tmp in recalc {
            register_candidate(ctx, tmp, &mut temp_keys, &mut completed);
        }
    }
    for key in temp_keys {
        ctx.remove_incomplete_room(key);
    }
    completed
}

/// The per-candidate registration body (slice C): Java's
/// `addIncompleteExpansionRoom(...)` + the completion call on the
/// candidate. The candidate is CONSUMED — Java clones the two shapes
/// into the new room object; here the candidate's own shapes MOVE
/// (the candidate value has no other reader), constructing the
/// field-for-field identical room without the clones.
fn register_candidate(
    ctx: &mut impl DrillEngine,
    tmp: epic_index::complete_shape::IncompleteRoom,
    temp_keys: &mut Vec<u64>,
    completed: &mut Vec<u64>,
) {
    let epic_index::complete_shape::IncompleteRoom {
        shape,
        layer,
        contained_shape,
    } = tmp;
    let key = ctx.add_incomplete_expansion_room(shape, layer, contained_shape);
    temp_keys.push(key);
    if let Some(done_key) = add_complete_room(ctx, key) {
        completed.push(done_key);
    }
}

/// The door scan (`:423-434`): the FIRST 2-dim door of the room that
/// leads to a COMPLETE free-space room donates its shape as
/// `fromDoorShape` and that neighbour as `ignoreObject`. Java reads
/// `currentDoor.getShape()` — the door line between the two live room
/// shapes (`ExpansionDoor::shape_between`).
fn scan_from_door_shape(ctx: &impl DrillEngine, room_key: u64) -> (Option<u64>, Option<TileShape>) {
    let room_shape = ctx.room_shape(room_key);
    let room_id = ctx.room_id(room_key);
    for door in ctx.room_doors(room_key) {
        if door.dimension != 2 {
            continue;
        }
        let Some(other_id) = door.other_room_id(room_id) else {
            continue;
        };
        let Some(other_key) = ctx.room_key_of_door(other_id, &door) else {
            continue;
        };
        if ctx.room_is_complete_free_space(other_key) {
            let other_shape = ctx.room_shape(other_key);
            let door_shape =
                crate::expansion::ExpansionDoor::shape_between(&room_shape, &other_shape);
            return (Some(other_key), Some(door_shape));
        }
    }
    (None, None)
}

/// Java `completeExpansionRoom(IncompleteFreeSpaceExpansionRoom)`
/// for a room already in the registry (the `completeNeighbourRooms`
/// and init-carried-room flow): door scan, complete, REMOVE the
/// consumed room (`:469`), walk the candidates.
pub fn complete_expansion_room(ctx: &mut impl DrillEngine, room_key: u64) -> Vec<u64> {
    let (ignore_object, from_door_shape) = scan_from_door_shape(ctx, room_key);
    let room_shape = ctx.room_shape(room_key);
    let contained_shape = ctx.room_contained_shape(room_key);
    let layer = ctx.room_layer(room_key);
    // Java removes the consumed room BEFORE the candidate walk
    // (`:469`): `removeIncompleteExpansionRoom` strips the room's
    // doors from both endpoints (AutorouteEngine `:603-614`), so the
    // candidate completions' sorter runs see the post-cleanup door
    // topology.
    ctx.remove_incomplete_room(room_key);
    complete_candidates(
        ctx,
        Some(&room_shape),
        contained_shape.as_ref(),
        layer,
        ignore_object,
        from_door_shape.as_ref(),
    )
}

/// The same Java flow for a FRESH null-shape room — Java
/// `new IncompleteFreeSpaceExpansionRoom(null, layer, shape)` — whose
/// door list is empty, so the scan yields `(null, null)`
/// (`:423-434`). This is the [`DrillEngine::complete_expansion_room`]
/// drill seam AND the init start-room completion
/// (`MazeSearchEngine.init:1017-1021` passes `null` as the room
/// shape).
pub fn complete_null_shape_room(
    ctx: &mut impl DrillEngine,
    contained_shape: &TileShape,
    layer: i32,
) -> Vec<u64> {
    complete_candidates(ctx, None, Some(contained_shape), layer, None, None)
}

/// Java `completeNeighbourRooms` (`:567-592`): complete the shapes of
/// the neighbour rooms so the doors of `room_key` will not change
/// later on. Completing a neighbour mutates door topology, so the
/// door iteration RESTARTS after every completion (the v1.9 semantics
/// comment); obstacle neighbours get their door set calculated once.
pub fn complete_neighbour_rooms(ctx: &mut impl DrillEngine, room_key: u64) {
    'restart: loop {
        let room_id = ctx.room_id(room_key);
        let doors = ctx.room_doors(room_key);
        for door in doors {
            let Some(other_id) = door.other_room_id(room_id) else {
                continue;
            };
            let Some(other_key) = ctx.room_key_of_door(other_id, &door) else {
                continue;
            };
            if ctx.room_is_incomplete(other_key) {
                complete_expansion_room(ctx, other_key);
                continue 'restart;
            }
            if ctx.room_is_obstacle(other_key) && !ctx.room_obstacle_doors_calculated(other_key) {
                add_complete_room(ctx, other_key);
                ctx.set_room_doors_calculated(other_key);
            }
        }
        return;
    }
}

/// Java `MazeSearchEngine.segmentProjection` (`:173-204`) — the
/// perpendicular projection of `from_segment` onto `to_segment`,
/// `None` when empty. Static helper of the maze engine kept beside the
/// completion module only for module layout; it is pure geometry.
#[must_use]
pub fn segment_projection(from_segment: &FloatLine, to_segment: &FloatLine) -> Option<FloatLine> {
    let check_segment = from_segment.adjust_direction(to_segment);
    let first_projection = to_segment.segment_projection(&check_segment);
    let second_projection = to_segment.segment_projection2(&check_segment);
    let Some(first) = first_projection else {
        // Java: firstProjection == null → result = secondProjection
        // (possibly null).
        return second_projection;
    };
    let Some(second) = second_projection else {
        return Some(first);
    };
    let result_a = if first.a == to_segment.a || second.a == to_segment.a {
        to_segment.a
    } else if first.a.distance_square(&to_segment.a) <= second.a.distance_square(&to_segment.a) {
        first.a
    } else {
        second.a
    };
    let result_b = if first.b == to_segment.b || second.b == to_segment.b {
        to_segment.b
    } else if first.b.distance_square(&to_segment.b) <= second.b.distance_square(&to_segment.b) {
        first.b
    } else {
        second.b
    };
    Some(FloatLine::new(result_a, result_b))
}
