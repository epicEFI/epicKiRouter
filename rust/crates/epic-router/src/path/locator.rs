//! Port of Java `app.freerouting.autoroute.path.FoundConnectionLocator`
//! — locates and constructs trace connection geometries from maze
//! search backtrack paths (T9).
//!
//! Java ships the family as one abstract base + two subclasses
//! (`FoundConnectionLocator45Degree`, `FoundConnectionLocatorAnyAngle`)
//! dispatched by `getInstance` on the angle restriction; the port keeps
//! one file per Java file (`locator_45.rs`, `locator_any.rs`) and
//! models the virtual `calculateNextTraceCorners` dispatch with the
//! [`Synthesis`] enum + free functions that take the mutable locator
//! state (Java's `protected` fields).
//!
//! Seam notes (see `SEAM.md` T9):
//! * The maze-search state is read through
//!   [`crate::maze::locator_access::LocatorAccess`]: Java reaches the
//!   door/drill objects and their LIVE room references directly; the
//!   port resolves the D17 opaque door keys through the engine's
//!   backtrack registry and the room reads through
//!   [`crate::expansion::NeighbourEngine`].
//! * `emitDiagnostics` (`:502-516`) is NOT ported — the Rust engine
//!   has no diagnostic sink yet (the same disposition as the drill
//!   `emitDiagnostics`, drill/mod.rs deviations).
//! * The net-gated `FRLogger.trace` hooks (nets 33/66/67/98) are the
//!   CAPTURE FORMAT of the spike oracle, not product behavior — not
//!   ported.
//! * `rippedItemList` is Java's `SortedSet<Item>` whose order is
//!   DESCENDING item id (`Item.compareTo = other.id - id`); the port
//!   carries `BTreeMap<i32, u64>` (id → item key, ASCENDING) — the
//!   consumer must iterate `.rev()`.
//! * Java's `ripupCosts` map parameter is nullable; the engine always
//!   passes non-null maps, so the port takes `&mut HashMap` (deviation
//!   documented at the harvest sites).

use std::collections::{BTreeMap, HashMap};

use epic_geometry::float_point::FloatPoint;
use epic_geometry::int_point::IntPoint;
use epic_geometry::regular_tile_shape::RegularTileShape;
use epic_geometry::tile_shape::TileShape;

use crate::control::{AngleRestriction, AutorouteControl};
use crate::expansion::NeighbourEngine;
use crate::maze::list_element::ExpandableObject;
use crate::maze::locator_access::LocatorAccess;
use crate::maze::search_engine::FindConnectionResult;

use super::locator_45;
use super::locator_any;

/// Java `AutorouteEngine.TRACE_WIDTH_TOLERANCE` (`AutorouteEngine.java:41`)
/// is an INT constant; the 45-degree locator's `traceHalfwidthAdd` and
/// the any-angle locator's `traceHalfwidthMax` consume it in their
/// Java-typed arithmetic (int in the 45-degree file, widened to double
/// in the any-angle file).
pub(crate) const TRACE_WIDTH_TOLERANCE: i32 = 2;

/// The corner-synthesis dispatch (Java's two concrete subclasses).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Synthesis {
    /// Java `FoundConnectionLocator45Degree` — used for BOTH the
    /// ninety-degree and the fortyfive-degree restriction (Java
    /// `getInstance`, `:196-200`); the restriction itself flows into
    /// [`calculate_additional_corner`].
    FortyFive,
    /// Java `FoundConnectionLocatorAnyAngle`.
    AnyAngle,
}

/// The restriction → subclass dispatch (Java `getInstance`,
/// `FoundConnectionLocator.java:196-200`): NINETY and FORTYFIVE share
/// the 45-degree class, NONE takes the any-angle class.
pub(crate) fn synthesis_for(angle_restriction: AngleRestriction) -> Synthesis {
    match angle_restriction {
        AngleRestriction::NinetyDegree | AngleRestriction::FortyfiveDegree => Synthesis::FortyFive,
        AngleRestriction::None => Synthesis::AnyAngle,
    }
}

/// One element of a subclass `calculateNextTraceCorners` result (Java's
/// `Collection<FloatPoint>` of OBJECT references). The base loop
/// filters every element with `currentNextCorner != prevCorner`
/// (`FoundConnectionLocator.java:432`) — a REFERENCE compare, and
/// `prevCorner` is initialized to `currentFromPoint` (`:425`) and both
/// fields are updated together on every kept corner (`:434-436`), so
/// the two names always denote the SAME object inside the loop. The
/// only non-fresh element a subclass can emit is the `currentFromPoint`
/// reference itself — the "door completely passed" arms
/// (`FoundConnectionLocatorAnyAngle.java:101` and `:184`) — which the
/// filter therefore always drops; every other element is a fresh Java
/// allocation that the filter keeps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum NextCorner {
    /// A fresh `FloatPoint` allocation — kept by the base filter.
    Fresh(FloatPoint),
    /// The `currentFromPoint` reference itself — dropped by the base
    /// filter; carrying it (instead of returning an empty list) is what
    /// keeps the trace loop RUNNING from the door index the arm
    /// advanced, so the same trace covers the remaining doors.
    FromPointAlias,
}

/// Type of a single item in the result list `connection_items` (Java
/// `ResultItem`, `:542-551`) — used to create a new `PolylineTrace`
/// (the T10/T11 consumer).
#[derive(Clone, Debug)]
pub struct ResultItem {
    /// Java `corners` — the rounded trace corners.
    pub corners: Vec<IntPoint>,
    /// Java `layer`.
    pub layer: i32,
}

/// Type of the elements of the backtrack list (Java
/// `BacktrackElement`, `:557-569`). `next_room` is the common room of
/// the current door and the next door in the backtrack list (`None` =
/// Java null).
#[derive(Clone, Debug)]
pub(crate) struct BacktrackElement {
    /// Java `door`.
    pub(crate) door: ExpandableObject,
    /// Java `sectionNoOfDoor`.
    pub(crate) section_no_of_door: i32,
    /// Java `nextRoom` — the room key (`None` = Java null).
    pub(crate) next_room: Option<u64>,
}

/// The mutable locator state (Java's `protected` fields,
/// `:53-59`) — the geometry synthesis happens entirely inside
/// [`get_instance`], so the state never escapes into the result.
pub(crate) struct LocatorState {
    /// Java `backtrackArray` — destination-first.
    pub(crate) backtrack_array: Vec<BacktrackElement>,
    /// Java `currentFromPoint` (`None` only on the bail arms where
    /// Java leaves the field unset).
    pub(crate) current_from_point: Option<FloatPoint>,
    /// Java `previousFromPoint`.
    pub(crate) previous_from_point: Option<FloatPoint>,
    /// Java `currentTraceLayer`.
    pub(crate) current_trace_layer: i32,
    /// Java `currentFromDoorIndex`.
    pub(crate) current_from_door_index: i32,
    /// Java `currentToDoorIndex`.
    pub(crate) current_to_door_index: i32,
    /// Java `currentTargetDoorIndex`.
    pub(crate) current_target_door_index: i32,
    /// Java `currentTargetShape` (set fresh every loop iteration).
    pub(crate) current_target_shape: Option<TileShape>,
    /// Java `connectionItems`.
    pub(crate) connection_items: Vec<ResultItem>,
}

/// The located connection (Java's public final locator fields) — the
/// T10/T11 consumer reads `connection_items` + the start/target
/// identification. Lifetimes are gone: every maze-state read happened
/// during construction.
pub struct FoundConnectionLocator {
    /// Java `connectionItems` — the new items implementing the found
    /// connection.
    pub connection_items: Vec<ResultItem>,
    /// Java `startItem` — the start item of the new routed connection
    /// (the item KEY; `None` = Java null on the bail arms).
    pub start_item: Option<u64>,
    /// Java `startLayer` — the layer of the connection to the start
    /// item.
    pub start_layer: i32,
    /// Java `targetItem` — the destination item (the item KEY; `None`
    /// = Java null on the bail and fanout arms).
    pub target_item: Option<u64>,
    /// Java `targetLayer` — the layer of the connection to the target
    /// item.
    pub target_layer: i32,
}

/// Returns a new instance of the locator or `None` if the maze search
/// result is `None` (Java `getInstance`, `:185-207`; the NINETY and
/// FORTYFIVE restrictions share the 45-degree class).
pub fn get_instance<A: LocatorAccess>(
    access: &A,
    maze_search_result: Option<&FindConnectionResult>,
    ctrl: &AutorouteControl,
    angle_restriction: AngleRestriction,
    // Java `rippedItemList` iterates DESCENDING item id
    // (`Item.compareTo = other.id - id`); this map is ASCENDING, so
    // consumers must iterate `.rev()` (module doc "rippedItemList").
    ripped_item_list: &mut BTreeMap<i32, u64>,
    ripup_costs: &mut HashMap<u64, i32>,
) -> Option<FoundConnectionLocator> {
    let maze_search_result = maze_search_result?;
    let synthesis = synthesis_for(angle_restriction);

    // Java ctor `:71-77`: the backtrack walk fills the array
    // destination-first.
    let backtrack_array = backtrack(
        access,
        maze_search_result,
        ripped_item_list,
        ripup_costs,
        ctrl.net_number,
    );

    let mut st = LocatorState {
        backtrack_array,
        current_from_point: None,
        previous_from_point: None,
        current_trace_layer: 0,
        current_from_door_index: 0,
        current_to_door_index: 0,
        current_target_door_index: 0,
        current_target_shape: None,
        connection_items: Vec::new(),
    };

    // Java `:102-114`: the LAST backtrack element is the start door.
    let last = st
        .backtrack_array
        .last()
        .expect("the backtrack list holds at least the destination element");
    let ExpandableObject::TargetDoor(start_door) = &last.door else {
        // Java `:103-111` — warn + null start/target fields; no trace
        // is constructed.
        return Some(FoundConnectionLocator {
            connection_items: st.connection_items,
            start_item: None,
            start_layer: 0,
            target_item: None,
            target_layer: 0,
        });
    };
    let start_door = start_door.clone();
    let start_item_key = start_door.item_key;
    let start_item = Some(start_item_key);
    // Java `startDoor.room` is non-null for a live target door (Java
    // would NPE below otherwise — the M1a/M3 expect-panic discipline).
    let start_room_key = start_door
        .room_id
        .and_then(|id| access.engine().room_key_of_id(id))
        .expect("a live target door carries its room");
    let start_layer = access.engine().room_layer(start_room_key);

    // Java `:116-137` — the destination arms.
    let mut at_fanout_end = false;
    let target_item;
    let target_layer;
    match &maze_search_result.destination_door {
        ExpandableObject::TargetDoor(destination_door) => {
            target_item = Some(destination_door.item_key);
            let dest_room_key = destination_door
                .room_id
                .and_then(|id| access.engine().room_key_of_id(id))
                .expect("a live target door carries its room");
            target_layer = access.engine().room_layer(dest_room_key);
            st.current_from_point = Some(calculate_starting_point(
                access,
                destination_door,
                dest_room_key,
            ));
        }
        ExpandableObject::Drill { .. } | ExpandableObject::StandaloneDrill { .. } => {
            // may happen only in case of fanout (Java `:124-129`).
            target_item = None;
            let info = access
                .drill_info(&maze_search_result.destination_door)
                .expect("the fanout destination is a drill");
            st.current_from_point = Some(info.location.to_float());
            target_layer = info.first_layer + maze_search_result.section_no_of_door;
            at_fanout_end = true;
        }
        _ => {
            // Java `:130-134` — warn; `currentFromPoint` stays unset
            // and no trace loop runs.
            return Some(FoundConnectionLocator {
                connection_items: st.connection_items,
                start_item,
                start_layer,
                target_item: None,
                target_layer: 0,
            });
        }
    }
    st.current_trace_layer = target_layer;
    st.previous_from_point = st.current_from_point;

    // Java `:139-181` — the trace loop.
    let mut connection_done = false;
    while !connection_done {
        let mut layer_changed = false;
        if at_fanout_end {
            // do not increase this.currentTargetDoorIndex (`:142-144`)
            layer_changed = true;
        } else {
            st.current_target_door_index = st.current_from_door_index + 1;
            while st.current_target_door_index < st.backtrack_array.len() as i32 && !layer_changed {
                if st.backtrack_array[st.current_target_door_index as usize]
                    .door
                    .is_drill()
                {
                    layer_changed = true;
                } else {
                    st.current_target_door_index += 1;
                }
            }
        }
        if layer_changed {
            // the next trace leads to a via (`:155-159`).
            let drill = &st.backtrack_array[st.current_target_door_index as usize].door;
            let info = access
                .drill_info(drill)
                .expect("a layer-change door is a drill");
            st.current_target_shape = Some(TileShape::RegularTileShape(RegularTileShape::IntBox(
                TileShape::surrounding_point(&info.location),
            )));
        } else {
            // the next trace leads to the final target (`:160-175`).
            connection_done = true;
            st.current_target_door_index = st.backtrack_array.len() as i32 - 1;
            let connection_shape = access
                .engine()
                .trace_connection_shape(start_item_key, start_door.tree_entry_no)
                .expect("the start item has a trace connection shape");
            let target_shape =
                connection_shape.intersection(&access.engine().room_shape(start_room_key));
            let mut target = target_shape;
            if target.dimension() >= 2 {
                // the target is a conduction area, make a save
                // connection by shrinking the shape by the trace
                // halfwidth.
                let trace_half_width =
                    f64::from(ctrl.compensated_trace_half_width[start_layer as usize]);
                let shrinked = target.offset(-trace_half_width);
                if !shrinked.is_empty() {
                    target = shrinked;
                }
            }
            st.current_target_shape = Some(target);
        }
        st.current_to_door_index = st.current_from_door_index + 1;
        let next_trace = calculate_next_trace(
            &mut st,
            access,
            ctrl,
            angle_restriction,
            synthesis,
            layer_changed,
            at_fanout_end,
        );
        at_fanout_end = false;
        st.connection_items.push(next_trace);
    }

    Some(FoundConnectionLocator {
        connection_items: st.connection_items,
        start_item,
        start_layer,
        target_item,
        target_layer,
    })
}

/// Java `calculateStartingPoint` (`:213-219`) — the starting point of
/// the next trace on `from_door.item`.
fn calculate_starting_point<A: LocatorAccess>(
    access: &A,
    from_door: &crate::expansion::TargetItemExpansionDoor,
    room_key: u64,
) -> FloatPoint {
    let connection_shape = access
        .engine()
        .trace_connection_shape(from_door.item_key, from_door.tree_entry_no)
        .expect("the start item has a trace connection shape");
    let connection_shape = connection_shape.intersection(&access.engine().room_shape(room_key));
    connection_shape.centre_of_gravity().round().to_float()
}

/// Java `otherRoom(CompleteExpansionRoom)` dispatch (`ExpansionDoor
/// .java:79-92`; `TargetItemExpansionDoor.java:51-53`,
/// `DrillPage.java:185-187`, `ExpansionDrill.java:105-107` all answer
/// null): the other endpoint room of a room door, ANSWERED ONLY when
/// it is itself a complete room (complete free space or obstacle);
/// every other input maps to `None`. Java reference-compares the room
/// objects; the port compares the unique room ids.
pub(crate) fn other_room<A: LocatorAccess>(
    access: &A,
    door: &ExpandableObject,
    current_next_room: Option<u64>,
) -> Option<u64> {
    match door {
        ExpandableObject::RoomDoor(room_door) => {
            let current_id = access.engine().room_id(current_next_room?);
            room_door
                .other_room_id(current_id)
                .and_then(|other_id| access.engine().room_key_of_id(other_id))
                .filter(|other_key| {
                    access.engine().room_is_complete_free_space(*other_key)
                        || access.engine().room_is_obstacle(*other_key)
                })
        }
        ExpandableObject::TargetDoor(_)
        | ExpandableObject::DrillPage { .. }
        | ExpandableObject::Drill { .. }
        | ExpandableObject::StandaloneDrill { .. } => None,
    }
}

/// Java `backtrack` (`:225-327`) — creates the list of doors by
/// backtracking from the destination door to the start door.
fn backtrack<A: LocatorAccess>(
    access: &A,
    maze_search_result: &FindConnectionResult,
    ripped_item_list: &mut BTreeMap<i32, u64>,
    ripup_costs: &mut HashMap<u64, i32>,
    net_number: i32,
) -> Vec<BacktrackElement> {
    let _ = net_number; // the net-98 BACKTRACK_* hooks are capture-only (module doc)
    let mut result: Vec<BacktrackElement> = Vec::new();
    let mut current_next_room: Option<u64> = None;
    let mut current_backtrack_door = maze_search_result.destination_door.clone();
    let mut current_maze_search_element = access.maze_search_element(
        &current_backtrack_door,
        maze_search_result.section_no_of_door,
    );
    if let ExpandableObject::TargetDoor(door) = &current_backtrack_door {
        current_next_room = door
            .room_id
            .and_then(|id| access.engine().room_key_of_id(id));
    } else if let Some(current_drill) = access.drill_info(&current_backtrack_door) {
        current_next_room = current_drill.room_arr
            [(current_drill.first_layer + maze_search_result.section_no_of_door) as usize];
        if current_maze_search_element.room_ripped {
            // Java `:256-265` — harvest EVERY obstacle room of the
            // drill's layer rooms (null slots skipped by the
            // instanceof).
            for tmp_room in &current_drill.room_arr {
                if let Some(room_key) = tmp_room
                    && access.engine().room_is_obstacle(*room_key)
                {
                    let item_key = access
                        .engine()
                        .room_obstacle_item_key(*room_key)
                        .expect("an obstacle room carries its obstacle item");
                    ripped_item_list.insert(access.engine().object_id(item_key), item_key);
                    ripup_costs.insert(item_key, current_maze_search_element.ripup_cost);
                }
            }
        }
    }
    let mut current_backtrack_element = BacktrackElement {
        door: current_backtrack_door.clone(),
        section_no_of_door: maze_search_result.section_no_of_door,
        next_room: current_next_room,
    };
    loop {
        result.push(current_backtrack_element.clone());
        let Some(backtrack_key) = current_maze_search_element.backtrack_door else {
            break;
        };
        // The registry covers every door of a finished backtrack chain
        // (the engine registers a door when its section state is
        // written, which precedes every later backtrack_door read).
        current_backtrack_door = access
            .resolve_backtrack_door(backtrack_key)
            .expect("the backtrack chain door is registered by the search");
        let mut current_section_no = current_maze_search_element.section_no_of_backtrack_door;
        let element_count = access.maze_search_element_count(&current_backtrack_door);
        if current_section_no >= element_count {
            // Java `:278-281` — `FRLogger.warn("currentSectionNo to
            // big")` then clamps to elementCount - 1. The warn is
            // diagnostic-only and is NOT ported (same disposition as
            // `emitDiagnostics`, locator.rs module doc); the CLAMP is
            // the behavioral payload and is kept verbatim — the
            // out-of-range section is silently rewritten to the last
            // section, and the element read below uses it.
            current_section_no = element_count - 1;
        }
        if let Some(current_drill) = access.drill_info(&current_backtrack_door) {
            current_next_room = current_drill.room_arr[current_section_no as usize];
        } else {
            current_next_room = other_room(access, &current_backtrack_door, current_next_room);
        }
        current_maze_search_element =
            access.maze_search_element(&current_backtrack_door, current_section_no);
        current_backtrack_element = BacktrackElement {
            door: current_backtrack_door.clone(),
            section_no_of_door: current_section_no,
            next_room: current_next_room,
        };
        if current_maze_search_element.room_ripped
            && let Some(room_key) = current_next_room
            && access.engine().room_is_obstacle(room_key)
        {
            // Java `:316-323` — the step harvest.
            let item_key = access
                .engine()
                .room_obstacle_item_key(room_key)
                .expect("an obstacle room carries its obstacle item");
            ripped_item_list.insert(access.engine().object_id(item_key), item_key);
            ripup_costs.insert(item_key, current_maze_search_element.ripup_cost);
        }
    }
    result
}

/// Java `ninetyDegreeCorner` (`:329-341`).
pub(crate) fn ninety_degree_corner(
    from_point: &FloatPoint,
    to_point: &FloatPoint,
    horizontal_first: bool,
) -> FloatPoint {
    let (x, y) = if horizontal_first {
        (to_point.x, from_point.y)
    } else {
        (from_point.x, to_point.y)
    };
    FloatPoint::new(x, y)
}

/// Java `fortyfiveDegreeCorner` (`:343-384`) — note the asymmetric
/// `>=` / `>` comparators (they are tie-breaks, pinned in the pin
/// suite).
pub(crate) fn fortyfive_degree_corner(
    from_point: &FloatPoint,
    to_point: &FloatPoint,
    horizontal_first: bool,
) -> FloatPoint {
    let abs_dx = (to_point.x - from_point.x).abs();
    let abs_dy = (to_point.y - from_point.y).abs();
    let x;
    let y;

    if abs_dx <= abs_dy {
        if horizontal_first {
            x = to_point.x;
            if to_point.y >= from_point.y {
                y = from_point.y + abs_dx;
            } else {
                y = from_point.y - abs_dx;
            }
        } else {
            x = from_point.x;
            if to_point.y > from_point.y {
                y = to_point.y - abs_dx;
            } else {
                y = to_point.y + abs_dx;
            }
        }
    } else if horizontal_first {
        y = from_point.y;
        if to_point.x > from_point.x {
            x = to_point.x - abs_dy;
        } else {
            x = to_point.x + abs_dy;
        }
    } else {
        y = to_point.y;
        if to_point.x > from_point.x {
            x = from_point.x + abs_dy;
        } else {
            x = from_point.x - abs_dy;
        }
    }
    FloatPoint::new(x, y)
}

/// Java `calculateAdditionalCorner` (`:390-404`) — calculates an
/// additional corner, so that for the lines from fromPoint to the
/// result corner and from the result corner to toPoint the angle
/// restriction is fulfilled.
pub(crate) fn calculate_additional_corner(
    from_point: &FloatPoint,
    to_point: &FloatPoint,
    horizontal_first: bool,
    angle_restriction: AngleRestriction,
) -> FloatPoint {
    match angle_restriction {
        AngleRestriction::NinetyDegree => {
            ninety_degree_corner(from_point, to_point, horizontal_first)
        }
        AngleRestriction::FortyfiveDegree => {
            fortyfive_degree_corner(from_point, to_point, horizontal_first)
        }
        AngleRestriction::None => *to_point,
    }
}

/// Java `adjustStartCorner` (`:522-537`) — adjusts the start corner,
/// so that a trace starting at this corner is completely contained in
/// the start room. `None` models Java returning `this.currentFromPoint`
/// ITSELF (the caller's reference-compare); `Some(point)` is the fresh
/// nearest point.
fn adjust_start_corner<A: LocatorAccess>(
    st: &LocatorState,
    access: &A,
    ctrl: &AutorouteControl,
) -> Option<FloatPoint> {
    let from = st
        .current_from_point
        .expect("adjustStartCorner runs with the from point set");
    if st.current_from_door_index < 0 {
        return None;
    }
    let current_from_info = &st.backtrack_array[st.current_from_door_index as usize];
    let room_key = current_from_info.next_room?;
    let trace_half_width =
        f64::from(ctrl.compensated_trace_half_width[st.current_trace_layer as usize]);
    let room_shape = access.engine().room_shape(room_key);
    let shrinked_room_shape = room_shape.offset(-trace_half_width);
    if shrinked_room_shape.is_empty() || shrinked_room_shape.contains_float(&from) {
        return None;
    }
    Some(
        shrinked_room_shape
            .nearest_point_approx(&from)
            .round()
            .to_float(),
    )
}

/// Java `calculateNextTrace` (`:410-493`) — calculates the next trace
/// of the connection under construction. `pub(crate)`: the passed-door
/// semantic pin (`path::pins`) drives it over a synthetic backtrack
/// chain, which `get_instance`'s registry-keyed backtrack walk cannot
/// express.
pub(crate) fn calculate_next_trace<A: LocatorAccess>(
    st: &mut LocatorState,
    access: &A,
    ctrl: &AutorouteControl,
    angle_restriction: AngleRestriction,
    synthesis: Synthesis,
    layer_changed: bool,
    at_fanout_end: bool,
) -> ResultItem {
    let mut corner_list: Vec<FloatPoint> = Vec::new();
    corner_list.push(
        st.current_from_point
            .expect("the trace loop runs with the from point set"),
    );
    if !at_fanout_end && let Some(adjusted_start_corner) = adjust_start_corner(st, access, ctrl) {
        // Java `:415` `adjustedStartCorner != this.currentFromPoint`
        // is a REFERENCE compare; `adjustStartCorner` returning
        // `Some` is exactly the fresh-object case.
        let add_corner = calculate_additional_corner(
            st.current_from_point.as_ref().expect("set above"),
            &adjusted_start_corner,
            true,
            angle_restriction,
        );
        corner_list.push(add_corner);
        corner_list.push(adjusted_start_corner);
        st.previous_from_point = st.current_from_point;
        st.current_from_point = Some(adjusted_start_corner);
    }
    // Java keeps `prevCorner` (`:425`) as a REFERENCE filter for the
    // corners below (`:432`): `prevCorner === currentFromPoint` through
    // the whole loop (both update together, `:434-436`), so the filter
    // drops exactly the from-point ALIAS singletons the any-angle
    // passed-door arms return (`FoundConnectionLocatorAnyAngle.java
    // :101`, `:184`) — and because the result is non-empty, the loop
    // CONTINUES from the door index the arm advanced. An empty result
    // (`:428`) is the only trace-end signal. [`NextCorner`] models the
    // reference/alias distinction explicitly.
    loop {
        let next_corners = match synthesis {
            Synthesis::FortyFive => {
                locator_45::calculate_next_trace_corners(st, access, ctrl, angle_restriction)
            }
            Synthesis::AnyAngle => locator_any::calculate_next_trace_corners(st, access, ctrl),
        };
        if next_corners.is_empty() {
            break;
        }
        for current_next_corner in next_corners {
            if let NextCorner::Fresh(point) = current_next_corner {
                corner_list.push(point);
                st.previous_from_point = st.current_from_point;
                st.current_from_point = Some(point);
            }
        }
    }

    let mut next_layer = st.current_trace_layer;
    if layer_changed {
        st.current_from_door_index = st.current_target_door_index + 1;
        if let Some(room_key) = st.backtrack_array[st.current_from_door_index as usize].next_room {
            next_layer = access.engine().room_layer(room_key);
        }
    }

    // Round the new trace corners to Integer (`:450-459`) — the
    // round-dedup keeps the FIRST of value-equal consecutive corners.
    let mut rounded_corner_list: Vec<IntPoint> = Vec::new();
    let mut prev_point: Option<IntPoint> = None;
    for corner in corner_list {
        let current_point = corner.round();
        if prev_point != Some(current_point) {
            rounded_corner_list.push(current_point);
            prev_point = Some(current_point);
        }
    }

    // The net-33/66/67 `compare_trace_next_trace_raw` hook is
    // capture-only (module doc).
    let result = ResultItem {
        corners: rounded_corner_list,
        layer: st.current_trace_layer,
    };
    st.current_trace_layer = next_layer;
    result
}
