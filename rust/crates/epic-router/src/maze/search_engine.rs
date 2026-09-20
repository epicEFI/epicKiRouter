//! Java `autoroute/maze/MazeSearchEngine.java` — the maze search CORE:
//! the front ([`Front`]), `findConnection`/`occupyNextElement`
//! (`:300-384`), `expandToRoomDoors` (`:390-626`), the door/target-door
//! expansion (`:629-788`), the cost model in `expandToDoorSection`
//! (`:791-966`), and `init` (`:969-1103`).
//!
//! Seam boundaries (T7-T10 own the production bodies):
//! * ripup: `MazeRipupResolver.checkLeavingRippedItem` /
//!   `checkRipup` — stubbed: `ripupAllowed` is false in the T6
//!   fixtures, so Java takes the same no-ripup path (stub bodies
//!   documented at the call sites).
//! * shove: `shoveTraceRoom` — stubbed "not shoved"; with
//!   `ripupCosts == 0` (no ripup) Java's flow continues identically
//!   past the `if (!shoved)` body.
//! * `DestinationDistance::calculate` (T8) and `ViaLayerChecker`
//!   (T10) — injected traits.
//! * `checkForcedTracePolyline` (the thin-room target-door legality
//!   probe, `:681-689`) — `DrillEngine::check_forced_trace_polyline`
//!   seam; the T6 fixtures keep target expansion in thick rooms where
//!   Java skips the check.
//! * `reduceTraceShapesAtTiePins` (`:154-167`) — SKIPPED: it mutates
//!   trace shapes at MULTI-NET tie pins; the T6 fixtures are
//!   single-net with no tie pins, so Java walks an empty mutation
//!   set. T9 lands it with the trace machinery.

use epic_geometry::float_line::FloatLine;
use epic_geometry::float_point::FloatPoint;
use epic_geometry::tile_shape::TileShape;

use crate::control::{AngleRestriction, AutorouteControl};
use crate::drill::{
    Adjustment, DestinationDistance, DrillEngine, DrillMazeListElement, DrillPageArray,
    ExpansionDrill, ViaLayerChecker, expand_to_other_layers,
};
use crate::expansion::TRACE_WIDTH_TOLERANCE;
use crate::maze::completion::{complete_neighbour_rooms, segment_projection};
use crate::maze::expansion_engine::MazeExpansionEngine;
use crate::maze::list_element::{ExpandableObject, Front, MazeListElement};

/// Java `AutorouteEngine.TRACE_WIDTH_TOLERANCE` is the DOOR-side
/// tolerance already mirrored in `expansion::door`; the maze engine
/// adds it to the half width for the small-door test (`:406`).
///
/// Java `MazeSearchEngine.ALREADY_RIPPED_COSTS` (`:44`).
pub(crate) const ALREADY_RIPPED_COSTS: i32 = 1;

/// Java `MazeSearchEngine.Result` (`:1218-1227`).
#[derive(Clone, Debug)]
pub struct FindConnectionResult {
    /// Java `destinationDoor`.
    pub destination_door: ExpandableObject,
    /// Java `sectionNoOfDoor`.
    pub section_no_of_door: i32,
}

/// The maze search engine (Java `MazeSearchEngine`). Generic over the
/// engine seam ([`DrillEngine`]), the T8 distance seam and the T10
/// via-check seam.
///
/// Construct fresh per search, as Java `getInstance`
/// (`MazeSearchEngine.java:137-151`); state (`front`/`door_sections`/
/// `backtrack_registry`) is intentionally never cleared — [`Self::init`]
/// asserts an empty `front` so a reused engine fails loudly in debug
/// (release builds skip the assert) instead of running on stale search
/// state.
pub struct MazeSearchEngine<'a, E: DrillEngine, D: DestinationDistance, V: ViaLayerChecker> {
    /// Java `autorouteEngine` + `searchTree` + `board` — the engine
    /// seam the maze code reaches through.
    pub(crate) ctx: &'a mut E,
    /// Java `ctrl` — Java MUTATES it around the resolver calls (the
    /// batch router advances `ripupCosts`/`ripupPassNo` per pass; the
    /// capture probes force `traceHalfWidth` for the shove gate), so
    /// the port carries it mutably.
    pub(crate) ctrl: &'a mut AutorouteControl,
    /// Java `mazeExpansionList` (`:52`).
    pub(crate) front: Front,
    /// Java `destinationDistance` (`:58`) — mutable for `init`'s
    /// `join` phase.
    pub(crate) destination_distance: &'a mut D,
    /// The T10 via-check seam for `expandToOtherLayers`.
    pub(crate) checker: &'a mut V,
    /// Java `autorouteEngine.drillPageArray`.
    pub(crate) pages: &'a mut DrillPageArray,
    /// The maze-search sections of the ROOM doors — Java stores them
    /// on `ExpansionDoor.sectionArr`; the Rust door values are
    /// snapshots, so the engine keys the state by the door key
    /// ([`ExpandableObject::key`]).
    pub(crate) door_sections: std::collections::HashMap<u64, Vec<crate::drill::MazeSearchElement>>,
    /// The standalone (ripped-via) drills of the current search — Java
    /// carries the drill OBJECT in the front element; the Rust element
    /// carries only the [`ExpandableObject::StandaloneDrill`] tag, and
    /// the live `ExpansionDrill` (with its maze-search state and
    /// `roomArr`) is owned here, keyed by the Java drill id.
    pub(crate) standalone_drills: std::collections::HashMap<i32, ExpansionDrill>,
    /// Java `destinationDoor` + `sectionNoOfDestinationDoor`
    /// (`:70-72`).
    pub(crate) destination: Option<(ExpandableObject, i32)>,
    /// Key → door object for the T9 locator's backtrack walk. Java's
    /// `FoundConnectionLocator.backtrack` walks REAL door references
    /// (`MazeSearchElement.backtrackDoor` is an object field); the Rust
    /// element stores the D17 opaque key, so the locator resolves keys
    /// back to [`ExpandableObject`] values through this registry. Every
    /// door whose section state was ever written is registered here
    /// (the write in [`Self::occupy_next_element`]), which covers the
    /// full backtrack chain: the destination door's write precedes the
    /// destination check, and every `backtrack_door` key on a written
    /// section was itself the door of an earlier popped element.
    pub(crate) backtrack_registry: std::collections::HashMap<u64, ExpandableObject>,
    /// Java `randomGenerator` (`:63`, seeded with `ctrl.ripupCosts`) —
    /// OWNED BY THE ENGINE, seeded ONCE per construction and shared
    /// with the ripup resolver ([`crate::maze::ripup`]). The state is
    /// ADVANCED across `checkRipup` calls only on randomized passes
    /// (`passNo >= 4 && passNo % 3 != 0` draws) — the capture's
    /// RNG-lifetime pin distinguishes seeded-once (two pass-4 rows
    /// differ) from per-call reseeding (they would not).
    pub(crate) random_generator: epic_geometry::java_random::JavaRandom,
}

impl<'a, E: DrillEngine, D: DestinationDistance, V: ViaLayerChecker> MazeSearchEngine<'a, E, D, V> {
    /// Java ctor equivalent — the field set of `MazeSearchEngine`
    /// (`:75-129`). The `randomGenerator` (`:63`) is seeded here with
    /// `ctrl.ripupCosts`, exactly once per construction.
    pub fn new(
        ctx: &'a mut E,
        ctrl: &'a mut AutorouteControl,
        destination_distance: &'a mut D,
        checker: &'a mut V,
        pages: &'a mut DrillPageArray,
    ) -> Self {
        // Java `this.randomGenerator = new Random(ctrl.ripupCosts)` —
        // read the seed BEFORE the struct literal moves `ctrl`.
        let random_seed = i64::from(ctrl.ripup_costs);
        MazeSearchEngine {
            ctx,
            ctrl,
            front: Front::default(),
            destination_distance,
            checker,
            pages,
            door_sections: std::collections::HashMap::new(),
            standalone_drills: std::collections::HashMap::new(),
            destination: None,
            backtrack_registry: std::collections::HashMap::new(),
            random_generator: epic_geometry::java_random::JavaRandom::new(random_seed),
        }
    }

    /// Java `getInstance` + `init` — `None` is Java's null result
    /// (initialization failed).
    pub fn find_connection_between(
        &mut self,
        start_items: &[u64],
        destination_items: &[u64],
    ) -> Option<FindConnectionResult> {
        if !self.init(start_items, destination_items) {
            return None;
        }
        self.find_connection()
    }

    fn is_stop_requested(&self) -> bool {
        self.ctx.is_stop_requested()
    }

    /// Java `findConnection` (`:300-308`).
    #[must_use]
    pub fn find_connection(&mut self) -> Option<FindConnectionResult> {
        while self.occupy_next_element() {}
        self.destination
            .as_ref()
            .map(|(door, section)| FindConnectionResult {
                destination_door: door.clone(),
                section_no_of_door: *section,
            })
    }

    /// The maze-search element of a door section — Java
    /// `door.getMazeSearchElement(section)`. Room-door state lives in
    /// [`Self::door_sections`] (lazily sized); target doors have
    /// exactly one; pages and drills carry their own.
    pub(crate) fn maze_element_mut(
        &mut self,
        door: &ExpandableObject,
        section: i32,
    ) -> &mut crate::drill::MazeSearchElement {
        match door {
            ExpandableObject::RoomDoor(_) => {
                let key = door.key();
                let vec = self.door_sections.entry(key).or_default();
                let needed = section as usize + 1;
                if vec.len() < needed {
                    vec.resize(needed, crate::drill::MazeSearchElement::default());
                }
                &mut vec[section as usize]
            }
            ExpandableObject::TargetDoor(_) => {
                // Java `mazeSearchElementCount()` is 1 and the only
                // caller passes 0. Target doors own ONE persistent maze
                // element (Java `TargetItemExpansionDoor
                // .mazeSearchElementArr`); the engine keys it in the
                // same map under the target-door key.
                debug_assert_eq!(section, 0, "target doors have a single section");
                let key = door.key();
                let vec = self.door_sections.entry(key).or_default();
                if vec.is_empty() {
                    vec.push(crate::drill::MazeSearchElement::default());
                }
                &mut vec[0]
            }
            ExpandableObject::DrillPage { row, column, .. } => self
                .pages
                .page_mut(*row, *column)
                .maze_search_element_mut(section as usize),
            ExpandableObject::Drill { row, column, d, .. } => self
                .pages
                .page_drill_mut(*row, *column, *d)
                .maze_search_element_mut(section as usize),
            ExpandableObject::StandaloneDrill { id, .. } => self
                .standalone_drills
                .get_mut(id)
                .expect("a standalone drill is registered at construction")
                .maze_search_element_mut(section as usize),
        }
    }

    /// The read side of [`Self::maze_element_mut`]. ABSENT room-door /
    /// target-door state reads as the default element: Java's
    /// `TargetItemExpansionDoor.mazeSearchInfo` is a constructor field
    /// (always default-initialized), and a room door's `sectionArr` is
    /// allocated by [`Self::allocate_sections`] (Java
    /// `getSectionSegments` → `allocateSections`) before any read in
    /// the engine's own flow — so an absent read only sees fresh state.
    pub(crate) fn maze_element(
        &self,
        door: &ExpandableObject,
        section: i32,
    ) -> &crate::drill::MazeSearchElement {
        const UNMARKED: crate::drill::MazeSearchElement = crate::drill::MazeSearchElement {
            is_occupied: false,
            backtrack_door: None,
            section_no_of_backtrack_door: 0,
            room_ripped: false,
            adjustment: crate::drill::Adjustment::None,
            ripup_cost: 0,
        };
        match door {
            ExpandableObject::RoomDoor(_) | ExpandableObject::TargetDoor(_) => self
                .door_sections
                .get(&door.key())
                .and_then(|vec| vec.get(section as usize))
                .unwrap_or(&UNMARKED),
            ExpandableObject::DrillPage { row, column, .. } => self
                .pages
                .page(*row, *column)
                .maze_search_element(section as usize),
            ExpandableObject::Drill { row, column, d, .. } => self
                .pages
                .page_drill(*row, *column, *d)
                .maze_search_element(section as usize),
            ExpandableObject::StandaloneDrill { id, .. } => self
                .standalone_drills
                .get(id)
                .map(|drill| drill.maze_search_element(section as usize))
                .unwrap_or(&UNMARKED),
        }
    }

    /// Java `ExpansionDoor.allocateSections` (`:193-201`): the section
    /// state array of a room door, allocated at the Java site that
    /// triggers it (`getSectionSegments`, called in
    /// [`Self::expand_to_door`]); re-allocation RESETS the state when
    /// the door re-segments to a different count.
    pub(crate) fn allocate_sections(&mut self, door: &ExpandableObject, section_count: usize) {
        debug_assert!(
            matches!(
                door,
                ExpandableObject::RoomDoor(_) | ExpandableObject::TargetDoor(_)
            ),
            "allocate_sections is the room/target-door state array"
        );
        let vec = self.door_sections.entry(door.key()).or_default();
        if vec.len() != section_count {
            *vec = vec![crate::drill::MazeSearchElement::default(); section_count];
        }
    }

    /// Java `ExpandableObject.mazeSearchElementCount()` — the length
    /// of the object's maze-search element array. Room doors read the
    /// `allocateSections` state (Java `sectionArr.length` — an element
    /// can only exist for a door whose sections were allocated, so the
    /// Java NPE-on-unallocated territory is UNREACHABLE ABSENT A
    /// BOOKKEEPING BUG (a room door read before its
    /// [`Self::allocate_sections`]); the `map_or(0)` fallback answers
    /// 0 where Java would throw, a deliberate deviation from the
    /// brief's letter on that claim);
    /// `TargetItemExpansionDoor` answers 1
    /// (`TargetItemExpansionDoor.java:60-63`); a drill page
    /// spans `board.getLayerCount()` entries; a drill spans
    /// `lastLayer - firstLayer + 1`.
    pub(crate) fn maze_search_element_count(&self, door: &ExpandableObject) -> i32 {
        match door {
            ExpandableObject::RoomDoor(_) => {
                let count = self.door_sections.get(&door.key()).map_or(0, Vec::len);
                i32::try_from(count).unwrap_or(i32::MAX)
            }
            ExpandableObject::TargetDoor(_) => 1,
            ExpandableObject::DrillPage { .. } => self.ctx.layer_count(),
            ExpandableObject::Drill { .. } | ExpandableObject::StandaloneDrill { .. } => {
                let drill = self.live_drill(door);
                drill.last_layer - drill.first_layer + 1
            }
        }
    }

    /// Java `occupyNextElement` (`:314-384`).
    pub fn occupy_next_element(&mut self) -> bool {
        if self.destination.is_some() {
            return false; // destination already reached
        }
        // Search the next element, which is not yet expanded
        // (`:318-338`) — the iterator-first pop, occupied sections
        // skipped.
        let mut list_element: Option<MazeListElement> = None;
        while !self.front.is_empty() {
            if self.is_stop_requested() {
                return false;
            }
            let element = self
                .front
                .pop_first()
                .expect("front is non-empty (checked above)");
            if !self
                .maze_element(&element.door, element.section_no_of_door)
                .is_occupied
            {
                list_element = Some(element);
                break;
            }
        }
        let Some(element) = list_element else {
            return false;
        };
        let section_state = self.maze_element_mut(&element.door, element.section_no_of_door);
        section_state.backtrack_door = element.backtrack_door.as_ref().map(ExpandableObject::key);
        section_state.section_no_of_backtrack_door = element.section_no_of_backtrack_door;
        section_state.room_ripped = element.room_ripped;
        section_state.ripup_cost = element.ripup_cost;
        section_state.adjustment = element.adjustment;
        // Register the door object for the locator's key → object
        // resolution (see the field doc). The write happens BEFORE the
        // destination/fanout early returns below, so the destination
        // door is registered too.
        self.backtrack_registry
            .insert(element.door.key(), element.door.clone());

        if matches!(element.door, ExpandableObject::DrillPage { .. }) {
            MazeExpansionEngine::expand_to_drills_of_page(
                self.ctx,
                self.ctrl,
                &mut self.front,
                self.destination_distance,
                self.pages,
                &element,
            );
            return true;
        }

        if let ExpandableObject::TargetDoor(door) = &element.door
            && self.ctx.item_is_destination(door.item_key)
        {
            // The destination is reached (`:353-359`).
            self.destination = Some((element.door.clone(), element.section_no_of_door));
            return false;
        }
        if self.ctrl.is_fanout
            && element.door.is_drill()
            && element
                .backtrack_door
                .as_ref()
                .is_some_and(ExpandableObject::is_drill)
        {
            // Fanout: the algorithm completes after the first drill
            // (`:361-368`).
            self.destination = Some((element.door.clone(), element.section_no_of_door));
            return false;
        }
        let from_drill = element.door.is_drill();
        if self.ctrl.vias_allowed && from_drill {
            let backtrack_is_drill = element
                .backtrack_door
                .as_ref()
                .is_some_and(ExpandableObject::is_drill);
            if !backtrack_is_drill {
                self.expand_to_other_layers_wired(&element);
            }
        }

        if let Some(next_room_key) = element.next_room_key
            && !self.expand_to_room_doors(&element, next_room_key)
        {
            // Occupation by ripup is delayed or nothing was expanded
            // (`:375-381`) — the section stays unoccupied so a
            // different section can claim it.
            return true;
        }
        self.maze_element_mut(&element.door, element.section_no_of_door)
            .is_occupied = true;
        true
    }

    /// The T5 layer-change port, wired to the front (Java
    /// `MazeExpansionEngine.expandToOtherLayers` emits through
    /// `search.mazeExpansionList.add`). Both the door and the
    /// backtrack door of an emitted element are the SAME drill (T5
    /// contract), so the wiring re-anchors both to the current
    /// (row, column, d).
    fn expand_to_other_layers_wired(&mut self, element: &MazeListElement) {
        debug_assert!(
            element.door.is_drill(),
            "expandToOtherLayers requires a drill door"
        );
        let drill = self.live_drill(&element.door);
        let door = element.door.clone();
        let mut emitted: Vec<DrillMazeListElement> = Vec::new();
        expand_to_other_layers(
            self.ctx,
            self.ctrl,
            element.door.key(),
            &drill,
            element.section_no_of_door,
            element.expansion_value,
            &element.shape_entry,
            self.checker,
            self.destination_distance,
            &mut |emit| emitted.push(emit),
        );
        for emit in emitted {
            self.front.add(MazeListElement::new(
                door.clone(),
                emit.section_no_of_door,
                Some(door.clone()),
                emit.section_no_of_backtrack_door,
                emit.expansion_value,
                emit.sorting_value,
                Some(emit.next_room_key),
                emit.shape_entry,
                emit.room_ripped,
                emit.adjustment,
                emit.already_checked,
            ));
        }
    }

    /// The live drill object of a drill door — Java casts
    /// `currElement.door` to `ExpansionDrill` and reads the object.
    /// Page-grid drills resolve through the page array; standalone
    /// (ripped-via) drills live in [`Self::standalone_drills`].
    pub(crate) fn live_drill(&self, door: &ExpandableObject) -> ExpansionDrill {
        match door {
            ExpandableObject::Drill { row, column, d, .. } => {
                self.pages.page_drill(*row, *column, *d).clone()
            }
            ExpandableObject::StandaloneDrill { id, .. } => self
                .standalone_drills
                .get(id)
                .expect("a standalone drill is registered at construction")
                .clone(),
            _ => panic!("expected a drill door"),
        }
    }

    /// Java `expandToRoomDoors` (`:390-626`). Returns true if the from
    /// door section has to be occupied.
    fn expand_to_room_doors(&mut self, element: &MazeListElement, next_room_key: u64) -> bool {
        let layer_index = self.ctx.room_layer(next_room_key);
        let layer_active = self.ctrl.layer_active[layer_index as usize];
        if !layer_active && self.ctx.layer_is_signal(layer_index) {
            return true;
        }

        let mut half_width =
            f64::from(self.ctrl.compensated_trace_half_width[layer_index as usize]);
        let mut current_door_is_small = false;
        if let ExpandableObject::RoomDoor(current_door) = &element.door {
            let mut half_width_add = half_width + TRACE_WIDTH_TOLERANCE;
            if self.ctrl.with_neckdown {
                // try evtl. neckdown at a destination pin (`:408-414`)
                let neck_down_half_width = self.check_neck_down_at_dest_pin(next_room_key);
                if neck_down_half_width > 0.0 {
                    half_width_add = half_width_add.min(neck_down_half_width);
                    half_width = half_width_add;
                }
            }
            current_door_is_small = self.door_is_small(current_door, 2.0 * half_width_add);
        }

        // Complete the neighbour rooms so the doors of this room will
        // not change later on (`:418-419`).
        complete_neighbour_rooms(self.ctx, next_room_key);

        let shape_entry_middle = element.shape_entry.a.middle_point(&element.shape_entry.b);

        if self.ctrl.with_neckdown
            && let ExpandableObject::TargetDoor(door) = &element.door
            && self.ctx.item_is_pin(door.item_key)
        {
            // try evtl. neckdown at a start pin (`:442-451`)
            let neckdown_half_width = self.ctx.pin_neckdown_half_width(door.item_key, layer_index);
            if neckdown_half_width > 0.0 {
                half_width = half_width.min(neckdown_half_width);
            }
        }

        let next_room_is_thick = self.room_is_thick(
            element,
            next_room_key,
            &shape_entry_middle,
            half_width,
            current_door_is_small,
        );

        if !layer_active && element.door.is_drill() {
            // check for a drill to a foreign conduction area on a
            // split plane (`:478-489`)
            let location = &self.live_drill(&element.door).location;
            if self
                .ctx
                .drill_hits_foreign_conduction(location, layer_index, self.ctrl.net_number)
            {
                return true;
            }
        }
        let mut something_expanded = self.expand_to_target_doors(
            element,
            next_room_key,
            next_room_is_thick,
            current_door_is_small,
            &shape_entry_middle,
        );

        if !layer_active {
            return true;
        }

        let mut ripup_costs: i32 = 0;
        // Java `instanceof FreeSpaceExpansionRoom` — the BASE class:
        // complete AND incomplete free-space rooms.
        let next_room_is_free_space = self.ctx.room_is_complete_free_space(next_room_key)
            || self.ctx.room_is_incomplete(next_room_key);
        let next_room_is_obstacle = self.ctx.room_is_obstacle(next_room_key);
        if next_room_is_free_space {
            if !element.already_checked && current_door_is_small {
                let mut enter_through_small_door = false;
                if next_room_is_thick {
                    // Java `ripupResolver.checkLeavingRippedItem`
                    // (`:200-213`, ported in `crate::maze::ripup`).
                    enter_through_small_door = self.check_leaving_ripped_item(element);
                }
                if !enter_through_small_door {
                    return something_expanded;
                }
            }
        } else if next_room_is_obstacle && !element.already_checked {
            let mut room_rippable = false;
            if self.ctrl.ripup_allowed {
                // Java `ripupResolver.checkRipup` (`:72-197`, ported in
                // `crate::maze::ripup`) — the resolver reads the
                // obstacle item off the room; the Rust room is a key,
                // so the item is resolved here.
                let obstacle_item = self
                    .ctx
                    .room_obstacle_item_key(next_room_key)
                    .expect("an obstacle room carries its obstacle item");
                ripup_costs = self.check_ripup(element, obstacle_item, current_door_is_small);
                room_rippable = ripup_costs >= 0;
            }

            if ripup_costs != ALREADY_RIPPED_COSTS && next_room_is_thick {
                let obstacle_is_shoveable_trace = self
                    .ctx
                    .room_obstacle_item_key(next_room_key)
                    .is_some_and(|item| self.ctx.item_is_polyline_trace(item));
                if !current_door_is_small
                    && self.ctrl.max_shove_trace_recursion_depth > 0
                    && obstacle_is_shoveable_trace
                {
                    // Java `shoveTraceRoom` (`:1130-1201`, ported in
                    // `crate::maze::shove_probe`) — the read-only
                    // probe; the board-mutation body is the T10 seam.
                    let shoved = self.shove_trace_room(element, next_room_key);
                    if !shoved && ripup_costs > 0 {
                        // Delay the occupation by ripup to allow
                        // shoving the room by other door sections
                        // (`:530-548`).
                        let mut delayed = element.clone();
                        delayed.expansion_value += f64::from(ripup_costs);
                        delayed.sorting_value += f64::from(ripup_costs);
                        delayed.room_ripped = true;
                        delayed.already_checked = true;
                        delayed.ripup_cost = ripup_costs;
                        self.front.add(delayed);
                    }
                    if !shoved {
                        return something_expanded;
                    }
                }
            }
            if !room_rippable {
                return true;
            }
        }

        let room_doors_snapshot = self.ctx.room_doors(next_room_key);
        for to_door in room_doors_snapshot {
            let door_object = ExpandableObject::RoomDoor(to_door.clone());
            if door_object == element.door {
                continue;
            }
            if self.expand_to_door(
                to_door,
                &door_object,
                element,
                next_room_key,
                ripup_costs,
                next_room_is_thick,
                Adjustment::None,
            ) {
                something_expanded = true;
            }
        }

        // Expand also the drill pages intersecting the room (`:600-623`).
        if self.ctrl.vias_allowed && !element.door.is_drill() {
            if (something_expanded || next_room_is_thick)
                && self.ctx.room_is_complete_free_space(next_room_key)
            {
                // avoid setting somethingExpanded when nextRoom is thin
                // to allow occupying by different sections of the door
                let room_shape = self.ctx.room_shape(next_room_key);
                let overlapping_pages = self.pages.overlapping_pages(&room_shape);
                for (row, column) in overlapping_pages {
                    MazeExpansionEngine::expand_to_drill_page(
                        self.ctx,
                        self.ctrl,
                        &mut self.front,
                        self.destination_distance,
                        self.pages,
                        row,
                        column,
                        element,
                    );
                    something_expanded = true;
                }
            } else if next_room_is_obstacle
                && let Some(item) = self.ctx.room_obstacle_item_key(next_room_key)
                && self.ctx.item_is_via(item)
            {
                // Java `Via.getAutorouteDrillInfo` — SEAM (T7): the
                // drill construction over a ripped via. Java stores the
                // drill OBJECT in the new front element; the Rust
                // element carries the `StandaloneDrill` tag (a
                // page-grid `Drill` tag would resolve state through the
                // page array and panic on the first pop), and the live
                // drill object + its maze state live in
                // [`Self::standalone_drills`]. The T6 fixtures carry no
                // vias; the production `via_drill_info` seam lands
                // with T7.
                if let Some((location, first_layer, last_layer, shape)) =
                    self.ctx.via_drill_info(item)
                {
                    let drill =
                        ExpansionDrill::new(shape.clone(), location, first_layer, last_layer);
                    let drill_id = drill.get_id();
                    let drill_door = ExpandableObject::StandaloneDrill {
                        id: drill_id,
                        shape,
                    };
                    self.standalone_drills.insert(drill_id, drill);
                    MazeExpansionEngine::expand_to_drill(
                        self.ctx,
                        self.ctrl,
                        &mut self.front,
                        self.destination_distance,
                        self.pages,
                        self.standalone_drills
                            .get(&drill_id)
                            .expect("the standalone drill was just inserted"),
                        &drill_door,
                        element,
                        ripup_costs,
                    );
                }
            }
        }

        something_expanded
    }

    /// The thick/thin classification of the next room (`:453-477`).
    fn room_is_thick(
        &mut self,
        element: &MazeListElement,
        next_room_key: u64,
        shape_entry_middle: &FloatPoint,
        half_width: f64,
        current_door_is_small: bool,
    ) -> bool {
        if self.ctx.room_is_obstacle(next_room_key) {
            return self.room_shape_is_thick(next_room_key);
        }
        let next_room_shape = self.ctx.room_shape(next_room_key);
        if next_room_shape.min_width() < 2.0 * half_width {
            return false; // to prevent problems with the opposite side
        }
        if !element.already_checked && element.door.dimension() == 1 && !current_door_is_small {
            // The algorithm below works only if the location is on the
            // border of the room shape — only 1-dimensional doors.
            let nearest_points =
                next_room_shape.nearest_border_points_approx(shape_entry_middle, 2);
            if nearest_points.len() < 2 {
                return false;
            }
            let current_distance = nearest_points[1].distance(shape_entry_middle);
            return current_distance > half_width + 1.0;
        }
        true
    }

    /// Java `roomShapeIsThick` (`:1105-1123`).
    fn room_shape_is_thick(&self, obstacle_room_key: u64) -> bool {
        let layer = self.ctx.room_layer(obstacle_room_key);
        let Some(obstacle_item) = self.ctx.room_obstacle_item_key(obstacle_room_key) else {
            // Java NPE arm — unreachable for a live obstacle room.
            return false;
        };
        let obstacle_half_width = if self.ctx.item_is_trace(obstacle_item) {
            f64::from(self.ctx.item_trace_half_width(obstacle_item))
                + f64::from(self.ctx.clearance_compensation_value(
                    self.ctx.item_clearance_class(obstacle_item),
                    layer,
                ))
        } else if self.ctx.item_is_via(obstacle_item) {
            let via_shape = self
                .ctx
                .item_tree_shape_on_layer(obstacle_item, layer)
                .expect("a via obstacle has a tree shape on its own layer");
            0.5 * via_shape.max_width()
        } else {
            // Java logs "unexpected obstacle item" and uses 0 — e.g. a
            // keepout area.
            0.0
        };
        obstacle_half_width >= f64::from(self.ctrl.compensated_trace_half_width[layer as usize])
    }

    /// Java `expandToTargetDoors` (`:629-704`).
    fn expand_to_target_doors(
        &mut self,
        element: &MazeListElement,
        next_room_key: u64,
        next_room_is_thick: bool,
        current_door_is_small: bool,
        shape_entry_middle: &FloatPoint,
    ) -> bool {
        if current_door_is_small {
            let mut enter_through_small_door = false;
            if let ExpandableObject::RoomDoor(room_door) = &element.door {
                let next_room_id = self.ctx.room_id(next_room_key);
                if let Some(from_room_id) = room_door.other_room_id(next_room_id)
                    && let Some(from_room_key) = self.ctx.room_key_of_id(from_room_id)
                    && self.ctx.room_is_obstacle(from_room_key)
                {
                    // otherwise entering through the small door
                    // may fail, because it was not checked
                    enter_through_small_door = true;
                }
            }
            if !enter_through_small_door {
                return false;
            }
        }
        let mut result = false;
        for to_door in self.ctx.room_target_doors(next_room_key) {
            let door_object = ExpandableObject::TargetDoor(to_door.clone());
            if door_object == element.door {
                continue;
            }
            let tree_shape_count = self.ctx.item_tree_shape_count(to_door.item_key);
            if to_door.tree_entry_no as i32 >= tree_shape_count {
                // Index out of range (trace modified during routing)
                continue;
            }
            let Some(target_shape) = self
                .ctx
                .trace_connection_shape(to_door.item_key, to_door.tree_entry_no)
            else {
                continue;
            };
            let connection_point = target_shape.nearest_point_approx(shape_entry_middle);
            if !next_room_is_thick {
                // check the line from shapeEntryMiddle to the nearest
                // point (`:670-694`). SEAM (T7):
                // `checkForcedTracePolyline` is stubbed "legal" — the
                // decision site is live; the T6 fixtures keep target
                // expansion inside thick rooms, where Java skips the
                // check entirely.
                let current_layer = self.ctx.room_layer(next_room_key);
                let from_point = shape_entry_middle.round();
                let to_point = connection_point.round();
                if from_point != to_point
                    && !self
                        .ctx
                        .check_forced_trace_polyline(&from_point, &to_point, current_layer)
                {
                    continue;
                }
            }
            let new_shape_entry = FloatLine::new(connection_point, connection_point);
            if self.expand_to_door_section(
                door_object,
                0,
                new_shape_entry,
                element,
                0,
                Adjustment::None,
            ) {
                result = true;
            }
        }
        result
    }

    /// Java `expandToDoor` (`:707-760`). `pub(crate)` for the
    /// per-section occupancy pre-check contract pin (maze/pins.rs); the
    /// argument list is the Java-faithful 8-arg shape.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn expand_to_door(
        &mut self,
        to_door: crate::expansion::ExpansionDoor,
        door_object: &ExpandableObject,
        element: &MazeListElement,
        next_room_key: u64,
        add_costs: i32,
        next_room_is_thick: bool,
        adjustment: Adjustment,
    ) -> bool {
        let layer = self.ctx.room_layer(next_room_key);
        let half_width = f64::from(self.ctrl.compensated_trace_half_width[layer as usize]);
        let mut something_expanded = false;
        let (first_key, second_key, first_shape, second_shape) =
            self.door_endpoint_shapes(&to_door);
        let both_complete_free_space = first_key
            .is_some_and(|k| self.ctx.room_is_complete_free_space(k))
            && second_key.is_some_and(|k| self.ctx.room_is_complete_free_space(k));
        let door_shape =
            crate::expansion::ExpansionDoor::shape_between(&first_shape, &second_shape);
        let (section_count, line_sections) = to_door.get_section_segments(
            &door_shape,
            both_complete_free_space,
            &first_shape,
            &second_shape,
            half_width,
        );
        // Java `getSectionSegments` allocates the door's maze-search
        // section state as a side effect (`allocateSections`, idempotent
        // by count, resetting on a re-segmentation) — mirror it here,
        // BEFORE the section loop reads `isOccupied` (`:715` vs `:798`).
        self.allocate_sections(door_object, section_count);

        for i in 0..section_count {
            let section = i as i32;
            if self.maze_element(door_object, section).is_occupied {
                continue;
            }
            let new_shape_entry: FloatLine;
            if next_room_is_thick {
                new_shape_entry = line_sections[i];
                if to_door.dimension == 1 && section_count == 1 && both_complete_free_space {
                    // check entering the toDoor at an acute corner of the
                    // room shape (`:724-740`)
                    let shape_entry_middle = mid_of(&line_sections[i]);
                    let room_shape = self.ctx.room_shape(next_room_key);
                    if room_shape.min_width() < 2.0 * half_width {
                        return false;
                    }
                    let nearest_points =
                        room_shape.nearest_border_points_approx(&shape_entry_middle, 2);
                    if nearest_points.len() < 2
                        || nearest_points[1].distance(&shape_entry_middle) <= half_width + 1.0
                    {
                        return false;
                    }
                }
            } else {
                // expand only doors on the opposite side of the room
                // from the shapeEntry (`:742-753`)
                if to_door.dimension == 1
                    && i == 0
                    && line_sections[0].b.distance_square(&line_sections[0].a) < 1.0
                {
                    // toDoor is small, belonging to a via or thin room
                    continue;
                }
                let Some(projected) = segment_projection(&element.shape_entry, &line_sections[i])
                else {
                    continue;
                };
                new_shape_entry = projected;
            }

            if self.expand_to_door_section(
                door_object.clone(),
                section,
                new_shape_entry,
                element,
                add_costs,
                adjustment,
            ) {
                something_expanded = true;
            }
        }
        something_expanded
    }

    /// The resolved registry keys and live shapes of a room door's two
    /// endpoints (Java reads `firstRoom`/`secondRoom` references; both
    /// are always live for a registered door).
    pub(crate) fn door_endpoint_shapes(
        &self,
        door: &crate::expansion::ExpansionDoor,
    ) -> (Option<u64>, Option<u64>, TileShape, TileShape) {
        let first_key = self.ctx.room_key_of_id(door.first_room_id);
        let second_key = self.ctx.room_key_of_id(door.second_room_id);
        let first_shape = self.ctx.room_shape(first_key.unwrap_or_else(|| {
            panic!(
                "a live room door's first room is registered (door id {}, first room id {})",
                door.id(),
                door.first_room_id
            )
        }));
        let second_shape = self.ctx.room_shape(second_key.unwrap_or_else(|| {
            panic!(
                "a live room door's second room is registered (door id {}, second room id {})",
                door.id(),
                door.second_room_id
            )
        }));
        (first_key, second_key, first_shape, second_shape)
    }

    /// Java `doorIsSmall` (`:763-788`).
    #[must_use]
    pub fn door_is_small(&self, door: &crate::expansion::ExpansionDoor, trace_width: f64) -> bool {
        let first_key = self.ctx.room_key_of_id(door.first_room_id);
        let second_key = self.ctx.room_key_of_id(door.second_room_id);
        if door.dimension == 1
            || (first_key.is_some_and(|k| self.ctx.room_is_complete_free_space(k))
                && second_key.is_some_and(|k| self.ctx.room_is_complete_free_space(k)))
        {
            let (_, _, first_shape, second_shape) = self.door_endpoint_shapes(door);
            let door_shape =
                crate::expansion::ExpansionDoor::shape_between(&first_shape, &second_shape);
            if door_shape.is_empty() {
                return true;
            }
            let door_length = match self.ctx.trace_angle_restriction() {
                AngleRestriction::NinetyDegree => door_shape.bounding_box().max_width(),
                AngleRestriction::FortyfiveDegree => door_shape
                    .bounding_octagon()
                    .map_or(0.0, |oct| oct.max_width()),
                AngleRestriction::None => {
                    let segment = door_shape
                        .diagonal_corner_segment()
                        .expect("a non-empty shape has a diagonal corner segment");
                    segment.b.distance(&segment.a)
                }
            };
            return door_length < trace_width;
        }
        false
    }

    /// Java `expandToDoorSection` (`:791-966`) — THE cost model:
    /// weighted distance + addCosts + bend penalty for the expansion
    /// value; destination distance for the sorting value.
    pub(crate) fn expand_to_door_section(
        &mut self,
        door: ExpandableObject,
        section_index: i32,
        shape_entry: FloatLine,
        from_element: &MazeListElement,
        add_costs: i32,
        adjustment: Adjustment,
    ) -> bool {
        if self.maze_element(&door, section_index).is_occupied {
            return false;
        }
        let Some(from_room_key) = from_element.next_room_key else {
            panic!("expandToDoorSection requires a non-null fromElement.nextRoom");
        };
        // Java `door.otherRoom(fromElement.nextRoom)` — the
        // `CompleteExpansionRoom` overload (`ExpansionDoor.java:79-91`)
        // answers null UNLESS the other room is itself complete
        // (complete free-space or obstacle); the plain overload
        // (`:62-72`) has no such restriction. An incomplete other room
        // therefore yields a `nextRoom`-less element (Java null),
        // exactly like a target door.
        let next_room_key = match &door {
            ExpandableObject::RoomDoor(room_door) => {
                let from_room_id = self.ctx.room_id(from_room_key);
                room_door
                    .other_room_id(from_room_id)
                    .and_then(|id| self.ctx.room_key_of_id(id))
                    .filter(|other_key| {
                        self.ctx.room_is_complete_free_space(*other_key)
                            || self.ctx.room_is_obstacle(*other_key)
                    })
            }
            ExpandableObject::TargetDoor(_) => None,
            ExpandableObject::DrillPage { .. }
            | ExpandableObject::Drill { .. }
            | ExpandableObject::StandaloneDrill { .. } => {
                unreachable!("expandToDoorSection is only called for room and target doors")
            }
        };
        let layer = self.ctx.room_layer(from_room_key);
        let shape_entry_middle = shape_entry.a.middle_point(&shape_entry.b);

        // The bend penalty (`:855-873`).
        let mut bend_cost_penalty = 0.0;
        if self.ctrl.bend_costs[layer as usize] > 0.0
            && let Some(backtrack) = &from_element.backtrack_door
        {
            let from_mid = mid_of(&from_element.shape_entry);
            let backtrack_cog = MazeExpansionEngine::door_shape(self.ctx, self.pages, backtrack)
                .centre_of_gravity();
            let prev_dx = from_mid.x - backtrack_cog.x;
            let prev_dy = from_mid.y - backtrack_cog.y;
            let next_dx = shape_entry_middle.x - from_mid.x;
            let next_dy = shape_entry_middle.y - from_mid.y;
            let cross_product = prev_dx * next_dy - prev_dy * next_dx;
            let sq_len_prev = prev_dx * prev_dx + prev_dy * prev_dy;
            let sq_len_next = next_dx * next_dx + next_dy * next_dy;
            // Normalized threshold (sin^2 > 0.01, about 5.7 degrees).
            if sq_len_prev > 0.0
                && sq_len_next > 0.0
                && (cross_product * cross_product) > 0.01 * sq_len_prev * sq_len_next
            {
                bend_cost_penalty = self.ctrl.bend_costs[layer as usize];
            }
        }

        let costs = &self.ctrl.trace_costs[layer as usize];
        let expansion_value = from_element.expansion_value
            + f64::from(add_costs)
            + bend_cost_penalty
            + shape_entry_middle.weighted_distance(
                &mid_of(&from_element.shape_entry),
                costs.horizontal,
                costs.vertical,
            );
        let sorting_value = expansion_value
            + self
                .destination_distance
                .calculate(&shape_entry_middle, layer);
        let room_ripped = (add_costs > 0 && adjustment == Adjustment::None)
            || (from_element.already_checked && from_element.room_ripped);

        let mut new_element = MazeListElement::new(
            door,
            section_index,
            Some(from_element.door.clone()),
            from_element.section_no_of_door,
            expansion_value,
            sorting_value,
            next_room_key,
            shape_entry,
            room_ripped,
            adjustment,
            false,
        );
        // The direct ripup cost rides only the element that caused it
        // (`:902-906`).
        if add_costs > 0 && adjustment == Adjustment::None {
            new_element.ripup_cost = add_costs;
        }
        self.front.add(new_element);
        true
    }

    /// Java `checkNeckDownAtDestPin` (`:1207-1215`).
    fn check_neck_down_at_dest_pin(&self, room_key: u64) -> f64 {
        for target_door in self.ctx.room_target_doors(room_key) {
            if self.ctx.item_is_pin(target_door.item_key) {
                let layer = self.ctx.room_layer(room_key);
                return self
                    .ctx
                    .pin_neckdown_half_width(target_door.item_key, layer);
            }
        }
        0.0
    }

    /// Java `init` (`:969-1103`).
    pub(crate) fn init(&mut self, start_items: &[u64], destination_items: &[u64]) -> bool {
        // Reuse guard (see the struct doc): Java never reuses an engine
        // (`getInstance` constructs one per search), so the state is
        // never cleared — a non-empty front here means a REUSED engine,
        // which must fail loudly, not search from stale state.
        debug_assert!(self.front.is_empty());
        // `reduceTraceShapesAtTiePins` — SKIPPED, see the module doc.
        // process the destination items (`:972-987`)
        let mut destination_ok = false;
        for &item in destination_items {
            if self.is_stop_requested() {
                return false;
            }
            self.ctx.set_item_start_info(item, false);
            for i in 0..self.ctx.item_tree_shape_count(item) {
                if let Some(tree_shape) = self.ctx.tree_shape(item, i as u32) {
                    let layer = self.ctx.item_shape_layer(item, i as u32);
                    let bounding = tree_shape.bounding_box();
                    self.destination_distance.join(&bounding, layer);
                }
            }
            destination_ok = true;
        }
        if !destination_ok && self.ctrl.is_fanout {
            // destination set is not needed for fanout (`:988-994`)
            let board_bounds = self.ctx.board_bounds();
            self.destination_distance.join(&board_bounds, 0);
            self.destination_distance
                .join(&board_bounds, self.ctrl.layer_count as i32 - 1);
            destination_ok = true;
        }
        if !destination_ok {
            return false;
        }

        // process the start items (`:1006-1023`)
        let mut start_rooms: Vec<(u64, i32)> = Vec::new();
        for &item in start_items {
            if self.is_stop_requested() {
                return false;
            }
            self.ctx.set_item_start_info(item, true);
            if self.ctx.item_is_connectable(item) {
                for i in 0..self.ctx.item_tree_shape_count(item) {
                    let Some(contained_shape) = self.ctx.trace_connection_shape(item, i as u32)
                    else {
                        continue;
                    };
                    let layer = self.ctx.item_shape_layer(item, i as u32);
                    // Java passes a null room shape; the registration is
                    // id-neutral (`addIncompleteExpansionRoom` appends to
                    // a list), and the completion below consumes the
                    // room.
                    let room_key = self.ctx.add_incomplete_expansion_room(
                        contained_shape.clone(),
                        layer,
                        contained_shape,
                    );
                    start_rooms.push((room_key, layer));
                }
            }
        }

        // complete the start rooms (`:1025-1041`). `getRoomsWithTargetItems`
        // (`:1028-1032`) is empty on a fresh engine — the carried-room
        // path lands with the multi-pass machinery (T9/T12).
        let mut completed_start_rooms: Vec<u64> = Vec::new();
        for (room_key, _layer) in &start_rooms {
            if self.is_stop_requested() {
                return false;
            }
            // The start room is a null-shape room for the completion
            // (Java `completeShape` reads the contained shape; the
            // placeholder registration shape must not leak).
            let contained = self.ctx.room_contained_shape(*room_key);
            if let Some(contained) = contained {
                completed_start_rooms.extend(crate::maze::completion::complete_null_shape_room(
                    self.ctx,
                    &contained,
                    self.ctx.room_layer(*room_key),
                ));
            }
            self.ctx.remove_incomplete_room(*room_key);
        }

        // Put the target doors of the completed start rooms into the
        // front (`:1043-1082`).
        let mut start_ok = false;
        for current_room in completed_start_rooms {
            for current_door in self.ctx.room_target_doors(current_room) {
                if self.is_stop_requested() {
                    return false;
                }
                if self.ctx.item_is_destination(current_door.item_key) {
                    continue;
                }
                let Some(connection_shape) = self
                    .ctx
                    .trace_connection_shape(current_door.item_key, current_door.tree_entry_no)
                else {
                    continue;
                };
                let room_shape = self.ctx.room_shape(current_room);
                let connection_shape = connection_shape.intersection(&room_shape);
                let current_center = connection_shape.centre_of_gravity();
                let shape_entry = FloatLine::new(current_center, current_center);
                let sorting_value = self
                    .destination_distance
                    .calculate(&current_center, self.ctx.room_layer(current_room));
                let new_list_element = MazeListElement::new(
                    ExpandableObject::TargetDoor(current_door),
                    0,
                    None,
                    0,
                    0.0,
                    sorting_value,
                    Some(current_room),
                    shape_entry,
                    false,
                    Adjustment::None,
                    false,
                );
                self.front.add(new_list_element);
                start_ok = true;
            }
        }
        start_ok
    }
}

/// The mid-point of a shape entry segment.
fn mid_of(line: &FloatLine) -> FloatPoint {
    line.a.middle_point(&line.b)
}
