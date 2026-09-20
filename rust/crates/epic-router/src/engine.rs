//! Java `autoroute/maze/AutorouteEngine.java` — the per-net routing
//! database and the connection-routing entry point (the M3-T11
//! assembly): the room registry over the manager's autoroute tree,
//! the drill-page array, the maze search + locator + inserter walk
//! ([`AutorouteEngine::autoroute_connection`]), and the
//! [`init_autoroute`]/[`finish_autoroute`] lifecycle faces (Java
//! `RoutingBoard.initAutoroute`/`finishAutoroute`).
//!
//! The engine implements [`NeighbourEngine`] + [`DrillEngine`]
//! (production bodies — the maze, drill and locator code reaches the
//! board through these seams) and [`CompleteShapeObjects`] (the
//! completion query face). The trait bodies are lifted from the T5
//! replay harness (`drill::pins::Harness`) with three documented
//! substitutions: the engine queries the MANAGER's autoroute tree
//! (Java has one tree; the harness kept a parallel copy), item tree
//! shapes read through the board's warm cache
//! ([`Board::tree_shape_precalc_peek`]) instead of a frozen snapshot,
//! and the lifecycle mutators ([`Self::init_connection`],
//! [`Self::clear`]) are real (the harness had none).
//!
//! ## Documented substitutions and banks (SEAM carries the dossier)
//!
//! * **Registry keys vs Java ids.** Rooms live in a parallel
//!   `rooms`/`keys` registry keyed by `ROOM_KEY_BASE + n`; Java keys
//!   rooms by object identity and orders them by their numeric room
//!   id. The graveyard keeps removed rooms reachable (in-flight maze
//!   elements hold Java references to discarded objects). `next_key`
//!   is deliberately NOT reset by [`Self::clear`] (Java's room ids are
//!   reset instead — `expansionRoomInstanceCount = 0` — and are
//!   independent of the keys).
//! * **`ItemAutorouteInfo` faces.** `start_infos` mirrors
//!   `setStartInfo`; `obstacle_rooms` mirrors
//!   `expansionRoomArr`; clearing them ([`Self::clear_item_autoroute_info`],
//!   called by [`Self::clear`] for EVERY item — Java
//!   `board.clearAllItemTemporaryAutorouteData`) is the
//!   temporary-data face. `obstacle_doors_calculated` keeps stale
//!   entries after an item update: Java drops them with the info
//!   object, but a rebuilt obstacle room gets a FRESH registry key, so
//!   a stale flag can never be observed.
//! * **`resetAllDoors` is a structural no-op plus the page reset.**
//!   Java's `resetDoors()` resets each door's `MazeSearchElement`
//!   state (occupied/backtrack/ripup marks); that state lives in the
//!   per-search [`MazeSearchEngine`] here (`door_sections`,
//!   `standalone_drills`), never on the persistent rooms — the door
//!   arms clear nothing. Only `drillPageArray.reset()` is live.
//! * **The init-failure FAILED row collapses.** Java distinguishes
//!   `MazeSearchEngine.getInstance` returning null (init failed →
//!   "…because the maze search algorithm could not be created.") from
//!   `findConnection` returning null ("…because no connection was
//!   found between their nets."); the Rust maze folds both into one
//!   `None`. [`Self::autoroute_connection`] always emits the
//!   no-connection row. State and downstream handling are identical;
//!   only the details string of a degenerate/aborted init differs.
//! * **The SKIPPED arm is unrepresentable.** Java's
//!   `autorouteResult.connectionItems == null` arm cannot occur (the
//!   locator owns its item list; no null exists).
//! * **The layers-disabled gate is a dead-defensive arm in BOTH
//!   engines.** Java's maze itself reads `ctrl.layerActive`
//!   (`MazeSearchEngine.java:396-397`, `:478`, `:493`;
//!   `DestinationDistance.java:57-63`), so an inactive endpoint layer
//!   starves the search before the post-locator gate
//!   (`AutorouteEngine.java:221-222`) can fire — and Java's batch
//!   router pre-checks the layers besides
//!   (`AutorouteConnectionRouter.java:189`). The gate is ported
//!   verbatim; its FAILED literal is pinned by inspection only.
//! * **Observers are a no-op seam.** Java brackets the ripup removals
//!   and the insertion with `startNotifyObservers`/`endNotifyObservers`
//!   when no observer is active; the Rust board has no observer
//!   system yet, so the bracket is absent (log-only face).
//! * **The `COMPLETE_ROOM added` trace row is omitted** (log-only,
//!   no digest impact per the bug-118 convention; not in the T11
//!   event-row surface).
//! * **Java-crash mappings.** A warm-cache peek miss on a live
//!   on-board item panics (Java would have the shape or crash
//!   earlier); the `touchingSides[1]` index and the layer index reads
//!   keep Java's AIOOBE face as panics.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use epic_board::board::Board;
use epic_board::forced_pad_router::CheckDrillResult as BoardCheckDrillResult;
use epic_board::id::ItemId;
use epic_board::items::{FixedState, ItemData};
use epic_board::routing_board_insert::{PullTightSeam, remove_trace_tails};
use epic_board::routing_board_search::check_trace_segment_points;
use epic_board::time_limit::TimeLimit;
use epic_board::trace_ops::{
    StopConnectionOption, get_connection_items, remove_item_through_repository,
};
use epic_board::tree_manager::SearchTreeManager;
use epic_geometry::float_point::FloatPoint;
use epic_geometry::int_box::IntBox;
use epic_geometry::line_segment::LineSegment;
use epic_geometry::point::Point;
use epic_geometry::simplex::Simplex;
use epic_geometry::tile_shape::TileShape;
use epic_index::NodeIdx;
use epic_index::SearchTreeVariant;
use epic_index::complete_shape::{CompleteShapeObjects, CompleteShapeQuery, complete_shape};
use epic_index::search_tree::SearchTree;

use crate::control::{AngleRestriction, AutorouteControl};
use crate::drill::DrillPageArray;
use crate::drill::expand_other_layers::CheckDrillResult as RouterCheckDrillResult;
use crate::drill::{DrillEngine, ViaLayerChecker, ViaRuleVia, max_drill_page_width};
use crate::expansion::{
    ExpansionDoor, ExpansionRoom, NeighbourEngine, RoomKind, TargetItemExpansionDoor, TreeEntry,
};
use crate::maze::destination_distance::DestinationDistance as ProdDistance;
use crate::maze::list_element::ExpandableObject;
use crate::maze::search_engine::MazeSearchEngine;
use crate::path::inserter::get_instance as insert_found_connection;
use crate::path::inserter::{InserterEventSink, router_angle_restriction};
use crate::path::locator;

/// Registry keys at/above this bound are rooms (items keep their raw
/// numeric item id as key). Const-equal to the T5 harness copy
/// (`drill::pins::ROOM_KEY_BASE`, pinned).
pub(crate) const ROOM_KEY_BASE: u64 = 1 << 40;

// ---------------------------------------------------------------------------
// the attempt result
// ---------------------------------------------------------------------------

/// Java `autoroute/AutorouteAttemptState.java` — the outcome kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutorouteAttemptState {
    Unknown,
    Skipped,
    NoUnconnectedNets,
    ConnectedToPlane,
    AlreadyConnected,
    NoConnections,
    Routed,
    Failed,
    InsertError,
}

impl AutorouteAttemptState {
    /// Java `toString()` — the constant name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "UNKNOWN",
            Self::Skipped => "SKIPPED",
            Self::NoUnconnectedNets => "NO_UNCONNECTED_NETS",
            Self::ConnectedToPlane => "CONNECTED_TO_PLANE",
            Self::AlreadyConnected => "ALREADY_CONNECTED",
            Self::NoConnections => "NO_CONNECTIONS",
            Self::Routed => "ROUTED",
            Self::Failed => "FAILED",
            Self::InsertError => "INSERT_ERROR",
        }
    }
}

/// Java `autoroute/AutorouteAttemptResult.java` — state + details,
/// `toString` = `STATE + ": " + details`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutorouteAttemptResult {
    pub state: AutorouteAttemptState,
    pub details: String,
}

impl AutorouteAttemptResult {
    /// Java `(state)` — empty details.
    #[must_use]
    pub fn new(state: AutorouteAttemptState) -> Self {
        Self {
            state,
            details: String::new(),
        }
    }

    /// Java `(state, details)`.
    #[must_use]
    pub fn with_details(state: AutorouteAttemptState, details: impl Into<String>) -> Self {
        Self {
            state,
            details: details.into(),
        }
    }
}

impl std::fmt::Display for AutorouteAttemptResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.state.as_str(), self.details)
    }
}

// ---------------------------------------------------------------------------
// the via checker + shape view
// ---------------------------------------------------------------------------

/// The production [`ViaLayerChecker`] placeholder: the real probe is
/// [`epic_board::forced_via_inserter::check_layer`], which needs
/// `&mut manager` + `&mut board` and therefore runs inside
/// [`DrillEngine::via_layer_check`] (where the engine holds both). The
/// maze calls the engine's override, never this checker — reaching it
/// is an engine-contract bug.
struct BoardLayerChecker;

impl ViaLayerChecker for BoardLayerChecker {
    fn check_layer(
        &mut self,
        _required_radius: f64,
        _clearance_class: i32,
        _attach_smd_allowed: bool,
        _room_shape: &TileShape,
        _location: &Point,
        _layer: i32,
        _net_number: i32,
    ) -> RouterCheckDrillResult {
        unreachable!("production checks route through DrillEngine::via_layer_check")
    }
}

/// The shared-borrow face of the engine for [`complete_shape`]: Java
/// walks the live objects through the tree; the Rust engine owns the
/// registry AND the board mutably during a search, so the query gets a
/// disjoint-field snapshot view instead.
struct EngineShapeView<'e> {
    board: &'e Board,
    rooms: &'e [ExpansionRoom],
    keys: &'e [u64],
    graveyard: &'e BTreeMap<u64, ExpansionRoom>,
    /// The autoroute tree's object id — the warm-cache peek key.
    tree_object_id: u64,
}

/// The live-then-graveyard room read (Java reads removed room OBJECTS
/// through the live references held by in-flight maze elements; the
/// graveyard is that object identity). Free-standing so the view and
/// the engine share it.
fn registry_resolve<'r>(
    rooms: &'r [ExpansionRoom],
    keys: &'r [u64],
    graveyard: &'r BTreeMap<u64, ExpansionRoom>,
    key: u64,
) -> Option<&'r ExpansionRoom> {
    if let Some(room) = graveyard.get(&key) {
        return Some(room);
    }
    keys.iter()
        .position(|&k| k == key)
        .map(|index| &rooms[index])
}

fn is_room_key(key: u64) -> bool {
    key >= ROOM_KEY_BASE
}

/// The nets of an item key (empty for rooms and unknown keys).
fn entry_nets(board: &Board, key: u64) -> Vec<i32> {
    u32::try_from(key)
        .ok()
        .and_then(|raw| board.get(ItemId::new(raw)))
        .map(|entry| entry.nets.clone())
        .unwrap_or_default()
}

impl CompleteShapeObjects for EngineShapeView<'_> {
    fn is_trace_obstacle(&self, object_key: u64, net_number: i32) -> bool {
        if is_room_key(object_key) {
            let Some(room) = registry_resolve(self.rooms, self.keys, self.graveyard, object_key)
            else {
                return false;
            };
            if room.is_complete_free_space() {
                // Java CompleteFreeSpaceExpansionRoom participates in
                // completeShape's restraint/ignore arms.
                return true;
            }
            if let RoomKind::Obstacle { item_key, .. } = room.kind {
                return !entry_nets(self.board, item_key).contains(&net_number);
            }
            return false;
        }
        !entry_nets(self.board, object_key).contains(&net_number)
    }

    fn shape_layer(&self, object_key: u64, shape_index: u32) -> i32 {
        if is_room_key(object_key) {
            return registry_resolve(self.rooms, self.keys, self.graveyard, object_key)
                .map(ExpansionRoom::layer)
                .unwrap_or(0);
        }
        let id = ItemId::new(u32::try_from(object_key).expect("item key fits u32"));
        self.board
            .item_shape_layer_read(id, shape_index as i32)
            .expect("live tree shapes carry a layer")
    }

    fn tree_shape(&self, object_key: u64, shape_index: u32) -> Option<TileShape> {
        if is_room_key(object_key) {
            return registry_resolve(self.rooms, self.keys, self.graveyard, object_key)
                .map(|room| room.shape().clone());
        }
        let id = ItemId::new(u32::try_from(object_key).expect("item key fits u32"));
        self.board
            .tree_shape_precalc_peek(id, self.tree_object_id)
            .expect("warm-cache peek miss")
            .get(shape_index as usize)
            .cloned()
            .flatten()
    }

    fn is_complete_free_space(&self, object_key: u64) -> bool {
        is_room_key(object_key)
            && registry_resolve(self.rooms, self.keys, self.graveyard, object_key)
                .is_some_and(ExpansionRoom::is_complete_free_space)
    }
}

// ---------------------------------------------------------------------------
// the engine
// ---------------------------------------------------------------------------

/// Java `AutorouteEngine` — one instance per routing session; the room
/// registry + drill pages + net/stop state, with the board and the
/// search-tree manager borrowed for the engine's lifetime.
pub struct AutorouteEngine<'a> {
    manager: &'a mut SearchTreeManager,
    board: &'a mut Board,
    /// The index of this engine's tree in the manager (Java
    /// `autorouteSearchTree`).
    autoroute_tree: usize,
    /// The manager-resolved tree variant (the completion dilations).
    resolved_variant: SearchTreeVariant,
    /// The live board bounding box (Java `board.boundingBox`).
    board_box: IntBox,
    maintain_database: bool,
    /// Java `netNumber` — `-1` until the first [`Self::init_connection`].
    net_number: i32,
    /// Java `stoppableThread` — the split-walk abort flag.
    stoppable_flag: Option<Arc<AtomicBool>>,
    /// Java `timeLimit`.
    time_limit: Option<TimeLimit>,
    /// Java `drillPageArray`.
    drill_page_array: DrillPageArray,
    /// The room registry (Java's room objects; creation-ordered like
    /// the Java `completeExpansionRooms`/`incompleteExpansionRooms`
    /// ArrayLists).
    rooms: Vec<ExpansionRoom>,
    keys: Vec<u64>,
    /// Java removed rooms remain REACHABLE: in-flight maze elements
    /// hold live references to discarded objects.
    graveyard: BTreeMap<u64, ExpansionRoom>,
    next_key: u64,
    /// Java `expansionRoomInstanceCount` — reset by [`Self::clear`].
    id_counter: i32,
    pending_completed: Vec<u64>,
    /// Java `ItemAutorouteInfo.expansionRoomArr`.
    obstacle_rooms: BTreeMap<(u64, u32), u64>,
    /// Java `ItemAutorouteInfo.startInfo` (`item_is_destination` is
    /// the negation).
    start_infos: BTreeSet<u64>,
    /// Java `ObstacleExpansionRoom.doorsCalculated`.
    obstacle_doors_calculated: BTreeSet<u64>,
    /// The autoroute-tree leaves of the COMPLETED rooms (Java stores
    /// the representation on the room object; the map is the same
    /// bookkeeping for [`Self::clear`]/removals).
    room_tree_entries: BTreeMap<u64, Vec<Option<NodeIdx>>>,
}

impl<'a> AutorouteEngine<'a> {
    /// Java ctor (`AutorouteEngine.java:80-94`): resolve (or create)
    /// the class-compensated autoroute tree, then build the drill-page
    /// array FROM the engine (Java's final-field two-step).
    pub fn new(
        manager: &'a mut SearchTreeManager,
        board: &'a mut Board,
        trace_clearance_class_index: i32,
        maintain_database: bool,
    ) -> AutorouteEngine<'a> {
        let autoroute_tree = manager.get_autoroute_tree(board, trace_clearance_class_index);
        let resolved_variant = manager.trees()[autoroute_tree].variant;
        let board_box = board
            .bounding_box()
            .unwrap_or_else(|| panic!("a routed board carries a bounding box"));
        let mut engine = AutorouteEngine {
            manager,
            board,
            autoroute_tree,
            resolved_variant,
            board_box,
            maintain_database,
            net_number: -1,
            stoppable_flag: None,
            time_limit: None,
            drill_page_array: DrillPageArray::default(),
            rooms: Vec::new(),
            keys: Vec::new(),
            graveyard: BTreeMap::new(),
            next_key: 0,
            id_counter: 0,
            pending_completed: Vec::new(),
            obstacle_rooms: BTreeMap::new(),
            start_infos: BTreeSet::new(),
            obstacle_doors_calculated: BTreeSet::new(),
            room_tree_entries: BTreeMap::new(),
        };
        let max_drill_page_width = max_drill_page_width(engine.board.default_via_diameter());
        engine.drill_page_array = DrillPageArray::new(&engine, max_drill_page_width);
        engine
    }

    // ---- internal accessors ----

    fn tree(&self) -> &SearchTree {
        &self.manager.trees()[self.autoroute_tree]
    }

    fn tree_mut(&mut self) -> &mut SearchTree {
        &mut self.manager.trees_mut()[self.autoroute_tree]
    }

    fn tree_object_id(&self) -> u64 {
        self.tree().object_id()
    }

    /// The disjoint-field snapshot view for the completion query.
    fn view(&self) -> EngineShapeView<'_> {
        EngineShapeView {
            board: self.board,
            rooms: &self.rooms,
            keys: &self.keys,
            graveyard: &self.graveyard,
            tree_object_id: self.tree_object_id(),
        }
    }

    /// The warm-cache item shapes (Java `item.getTreeShape` reads the
    /// same stored shapes; a live on-board item always has them — a
    /// miss is an engine-contract bug, hence the panic).
    fn item_tree_shapes(&self, item_key: u64) -> &[Option<TileShape>] {
        let id = ItemId::new(u32::try_from(item_key).expect("item key fits u32"));
        self.board
            .tree_shape_precalc_peek(id, self.tree_object_id())
            .expect("warm-cache peek miss")
    }

    // ---- lifecycle ----

    /// Java `getNetNumber`.
    #[must_use]
    pub fn net_number(&self) -> i32 {
        self.net_number
    }

    /// Java `isStopRequested` (`:287-295`): the time limit first, then
    /// the stoppable-thread flag.
    #[must_use]
    pub fn is_stop_requested(&self) -> bool {
        if self
            .time_limit
            .as_ref()
            .is_some_and(TimeLimit::limit_exceeded)
        {
            return true;
        }
        self.stoppable_flag
            .as_deref()
            .is_some_and(|flag| flag.load(Ordering::Relaxed))
    }

    /// Java `initConnection` (`:96-124`): under a maintained database,
    /// a NET SWITCH first removes the net-dependent complete rooms
    /// (creation order — Java's `completeExpansionRooms` ArrayList),
    /// then runs [`Self::additional_update_after_change`] for every
    /// on-board item of the new net (Java walks `board.getItems()` in
    /// insertion order = ascending id; off-board items carry no tree
    /// shapes, so the Java loop no-ops for them and the port skips
    /// them). Always assigns net/stop/time state.
    pub fn init_connection(
        &mut self,
        net_number: i32,
        stoppable_flag: Option<Arc<AtomicBool>>,
        time_limit: Option<TimeLimit>,
    ) {
        if self.maintain_database && net_number != self.net_number {
            if !self.keys.is_empty() {
                // Java guards on `completeExpansionRooms != null`; the
                // registry replaces the (lazily allocated) ArrayList.
                let keys = self.keys.clone();
                for key in keys {
                    let remove = self.resolve(key).is_some_and(|room| {
                        room.is_complete_free_space() && room.is_net_dependent()
                    });
                    if remove {
                        self.remove_complete_expansion_room_key(key);
                    }
                }
            }
            let net_item_keys: Vec<u64> = self
                .board
                .iter_ascending()
                .filter(|entry| entry.on_the_board && entry.nets.contains(&net_number))
                .map(|entry| u64::from(entry.id.get()))
                .collect();
            for key in net_item_keys {
                self.additional_update_after_change(key);
            }
        }
        self.net_number = net_number;
        self.stoppable_flag = stoppable_flag;
        self.time_limit = time_limit;
    }

    /// Java `RoutingBoard.additionalUpdateAfterChange`
    /// (`RoutingBoard.java:96-118`): invalidate the drill pages of and
    /// remove the complete rooms touching every tree shape of the
    /// item, then clear the item's autoroute info. No-op unless the
    /// database is maintained (Java's `autorouteEngine == null ||
    /// !maintainDatabase` guard).
    pub fn additional_update_after_change(&mut self, item_key: u64) {
        if !self.maintain_database {
            return;
        }
        let shapes: Vec<Option<TileShape>> = self.item_tree_shapes(item_key).to_vec();
        for (shape_index, shape) in shapes.iter().enumerate() {
            let Some(shape) = shape else {
                // Java tree shapes are never null; a None slot holds no
                // leaf and cannot touch anything.
                continue;
            };
            self.drill_page_array.invalidate(shape);
            let layer = CompleteShapeObjects::shape_layer(self, item_key, shape_index as u32);
            for entry in crate::drill::java_ordered_entries(self, shape, layer) {
                if self
                    .resolve(entry.object_key)
                    .is_some_and(ExpansionRoom::is_complete_free_space)
                {
                    self.remove_complete_expansion_room_key(entry.object_key);
                }
            }
        }
        self.clear_item_autoroute_info(item_key);
    }

    /// Java `Item.clearAutorouteInfo()`: drop the start-info flag and
    /// the obstacle-room cache, and graveyard the item's obstacle
    /// rooms (Java drops the info OBJECT; the rooms stay reachable
    /// through in-flight references — no door cascade here).
    fn clear_item_autoroute_info(&mut self, item_key: u64) {
        self.start_infos.remove(&item_key);
        let stale: Vec<u64> = self
            .obstacle_rooms
            .range((item_key, 0)..=(item_key, u32::MAX))
            .map(|(_, &room_key)| room_key)
            .collect();
        for room_key in stale {
            self.drop_room_reference(room_key);
        }
        self.obstacle_rooms
            .range((item_key, 0)..=(item_key, u32::MAX))
            .map(|(k, _)| *k)
            .collect::<Vec<_>>()
            .iter()
            .for_each(|k| {
                self.obstacle_rooms.remove(k);
            });
        // Java drops `doorsCalculated` with the info object; a stale
        // flag here is unobservable (a rebuilt obstacle room gets a
        // fresh registry key).
    }

    /// Registry → graveyard without door work (the Java
    /// object-still-referenced face).
    fn drop_room_reference(&mut self, room_key: u64) {
        if let Some(index) = self.keys.iter().position(|&k| k == room_key) {
            let room = self.rooms.remove(index);
            self.keys.remove(index);
            self.graveyard.insert(room_key, room);
        }
    }

    /// Java `clear` (`:306-318`): remove every completed room's tree
    /// entries, reset the registries and the room-id counter, and
    /// clear every item's temporary autoroute data (the
    /// `start_infos`/`obstacle_rooms` clears). The drill pages are NOT
    /// reset (Java `clear` does not touch them) and `next_key` stays
    /// monotone (the registry-key replacement for Java's room ids,
    /// which ARE reset via `expansionRoomInstanceCount`).
    pub fn clear(&mut self) {
        let entries = std::mem::take(&mut self.room_tree_entries);
        for room_entries in entries.into_values() {
            self.tree_mut().remove(&room_entries);
        }
        self.rooms.clear();
        self.keys.clear();
        self.graveyard.clear();
        self.pending_completed.clear();
        self.obstacle_rooms.clear();
        self.start_infos.clear();
        self.obstacle_doors_calculated.clear();
        self.id_counter = 0;
    }

    /// Java `resetAllDoors` (`:652-669`): ONLY the drill-page reset is
    /// live — Java's door-reset arms reset per-door `MazeSearchElement`
    /// state, which lives in the per-search [`MazeSearchEngine`] here
    /// (see the module docs).
    pub fn reset_all_doors(&mut self) {
        self.drill_page_array.reset();
    }

    // ---- room database ----

    /// Java `removeCompleteExpansionRoom` (`:376-412`): detach the
    /// doors, grow incomplete rooms into every dimension-1 touching
    /// neighbour, remove the rest of the doors (with the incomplete
    /// cascade), drop the tree entries, and invalidate the drill pages
    /// over the room shape.
    fn remove_complete_expansion_room_key(&mut self, room_key: u64) {
        let room_id = self.resolve(room_key).expect("room key live").id();
        let room_shape = self
            .resolve(room_key)
            .expect("room key live")
            .shape()
            .clone();
        let room_layer = self.resolve(room_key).expect("room key live").layer();
        for door in self
            .resolve(room_key)
            .expect("room key live")
            .doors()
            .to_vec()
        {
            let Some(other_id) = door.other_room_id(room_id) else {
                continue;
            };
            let Some(other_key) = self.room_key_of_id(other_id) else {
                continue;
            };
            let Some(other_index) = self.keys.iter().position(|&k| k == other_key) else {
                // Java mutates the dead neighbour object — unobservable.
                continue;
            };
            self.rooms[other_index].remove_door(&door);
            let neighbour_shape = self.rooms[other_index].shape().clone();
            let intersection = room_shape.intersection(&neighbour_shape);
            if intersection.dimension() == 1 {
                // Add a new incomplete room to the neighbour.
                let touching_sides = room_shape.touching_sides(&neighbour_shape);
                let border_line = neighbour_shape.border_line(touching_sides[1]).opposite();
                let new_shape = TileShape::Simplex(Box::new(Simplex::get_instance(&[border_line])));
                let new_key =
                    self.add_incomplete_expansion_room(new_shape, room_layer, intersection);
                let new_id = self.resolve(new_key).expect("fresh room live").id();
                // Java `new ExpansionDoor(currentNeighbour, newIncompleteRoom, 1)`.
                let new_door = ExpansionDoor::new(other_id, new_id, 1);
                self.rooms[other_index].add_door(new_door.clone());
                self.room_mut(new_key).add_door(new_door);
            }
        }
        self.remove_all_doors_impl(room_key);
        if let Some(entries) = self.room_tree_entries.remove(&room_key) {
            self.tree_mut().remove(&entries);
        }
        // Java's `completeExpansionRooms.remove(room)` — a no-op when
        // the set does not contain the room; the registry removal is
        // idempotent the same way.
        self.drop_room_reference(room_key);
        self.drill_page_array.invalidate(&room_shape);
    }

    /// Java `removeIncompleteExpansionRoom` (`:360-371`): doors first,
    /// then the registry removal (bug-123 discipline; idempotent).
    fn remove_room(&mut self, room_key: u64) {
        if !self.keys.contains(&room_key) {
            return;
        }
        self.remove_all_doors_impl(room_key);
        self.drop_room_reference(room_key);
    }

    /// Java `AutorouteEngine.removeAllDoors` (`:603-614`): every door
    /// of `room_key` is removed from BOTH endpoint rooms, and an
    /// incomplete free-space room discovered through a removed door is
    /// removed from the engine entirely (the `:611` cascade — in
    /// practice one level deep, because two incomplete rooms never
    /// share a door: incomplete rooms are not in the tree, so the
    /// sorter only attaches doors between a candidate and
    /// tree-resident neighbours). Mutating a graveyard'd other is
    /// skipped: Java mutates the dead object there, which is
    /// unobservable.
    fn remove_all_doors_impl(&mut self, room_key: u64) {
        if !self.keys.contains(&room_key) {
            return;
        }
        let room_id = self.resolve(room_key).expect("room key live").id();
        let doors = self
            .resolve(room_key)
            .expect("room key live")
            .doors()
            .to_vec();
        for door in doors {
            let Some(other_id) = door.other_room_id(room_id) else {
                continue;
            };
            let Some(other_key) = self.room_key_of_id(other_id) else {
                continue;
            };
            if let Some(index) = self.keys.iter().position(|&k| k == other_key) {
                self.rooms[index].remove_door(&door);
            }
            let cascade = self
                .resolve(other_key)
                .is_some_and(ExpansionRoom::is_incomplete)
                && self.keys.contains(&other_key);
            if cascade {
                self.remove_room(other_key);
            }
        }
        self.room_mut(room_key).clear_doors();
    }

    /// Inserts the SURVIVING completed room of a completion call and
    /// discards the abandoned restart attempts (they burned their ids
    /// but never reached the tree — the T4 capture's gap semantics).
    /// The accepted room's leaves are recorded for [`Self::clear`].
    fn flush_completed_inserts(&mut self, accepted: Option<u64>) {
        for key in std::mem::take(&mut self.pending_completed) {
            if Some(key) == accepted {
                let shape = self
                    .resolve(key)
                    .expect("completed room live")
                    .shape()
                    .clone();
                let entries = self.tree_mut().insert(key, &[Some(shape)]);
                self.room_tree_entries.insert(key, entries);
            } else {
                self.remove_room(key);
            }
        }
    }

    /// Java `getRoomsWithTargetItems` (`:620-634`): the live complete
    /// free-space rooms with a target door to an item of the set, in
    /// Java's `TreeSet` order (room id DESCENDING — `compareTo` is
    /// `other.id - this.id`). Java's `completeExpansionRooms == null`
    /// guard (no maintained database) is subsumed: with no maintained
    /// database the caller never asks.
    #[must_use]
    pub fn get_rooms_with_target_items(&self, items: &[u64]) -> Vec<u64> {
        let mut result: Vec<u64> = self
            .rooms
            .iter()
            .zip(&self.keys)
            .filter(|(room, _)| room.is_complete_free_space())
            .filter(|(room, _)| {
                room.target_doors()
                    .iter()
                    .any(|door| items.contains(&door.item_key))
            })
            .map(|(_, key)| *key)
            .collect();
        result
            .sort_by_key(|&key| std::cmp::Reverse(self.resolve(key).map_or(0, ExpansionRoom::id)));
        result
    }

    // ---- registry helpers ----

    fn alloc_key(&mut self) -> u64 {
        self.next_key += 1;
        ROOM_KEY_BASE + self.next_key
    }

    /// The live-then-graveyard room read.
    fn resolve(&self, key: u64) -> Option<&ExpansionRoom> {
        registry_resolve(&self.rooms, &self.keys, &self.graveyard, key)
    }

    fn room_mut(&mut self, key: u64) -> &mut ExpansionRoom {
        let index = self
            .keys
            .iter()
            .position(|&k| k == key)
            .unwrap_or_else(|| panic!("room key {key} not registered"));
        &mut self.rooms[index]
    }

    fn room_key_of_id(&self, id: i32) -> Option<u64> {
        self.rooms
            .iter()
            .zip(&self.keys)
            .find(|(room, _)| room.id() == id)
            .map(|(_, key)| *key)
            .or_else(|| {
                self.graveyard
                    .iter()
                    .find(|(_, room)| room.id() == id)
                    .map(|(key, _)| *key)
            })
    }

    fn item_nets(&self, key: u64) -> &[i32] {
        let id = ItemId::new(u32::try_from(key).expect("item key fits u32"));
        self.board
            .get(id)
            .map(|entry| entry.nets.as_slice())
            .unwrap_or(&[])
    }

    // ---- the connection entry point ----

    /// Java `autorouteConnection` (`:131-282`): run the maze search,
    /// emit the raw maze-result row (nets 33/66/67), reconstruct the
    /// connection with the locator, clean up the expansion rooms
    /// BEFORE any early return, then ripup the obstructing connections
    /// and insert the new one.
    #[allow(clippy::too_many_lines)] // the Java method is one flat walk
    #[allow(clippy::too_many_arguments)] // the Java signature, kept 1:1
    pub fn autoroute_connection<S: PullTightSeam, E: InserterEventSink>(
        &mut self,
        start_items: &[u64],
        dest_items: &[u64],
        ctrl: &mut AutorouteControl,
        ripped_item_list: &mut BTreeMap<i32, u64>,
        ripup_costs: &mut HashMap<u64, i32>,
        seam: &mut S,
        sink: &mut E,
    ) -> AutorouteAttemptResult {
        // Pre-maze reads (the maze mutably borrows the engine).
        let net = ctrl.net_number;
        let describe = describe_connection(self.board, start_items, dest_items);
        let angle_restriction =
            router_angle_restriction(self.board.rules().trace_angle_restriction);

        // The page array is swapped out for the search (the maze holds
        // it alongside the engine borrow) and restored before cleanup.
        let mut pages = std::mem::take(&mut self.drill_page_array);

        let autoroute_result = {
            let mut destination_distance = ProdDistance::from_ctrl(ctrl);
            let mut checker = BoardLayerChecker;
            let mut maze = MazeSearchEngine::new(
                self,
                ctrl,
                &mut destination_distance,
                &mut checker,
                &mut pages,
            );
            let search_result = maze.find_connection_between(start_items, dest_items);

            // The raw maze-result row (Java `:163-178`) — log-only.
            if let Some(result) = &search_result
                && (net == 33 || net == 66 || net == 67)
            {
                let destination_type = expandable_object_type_name(&result.destination_door);
                sink.trace(&format!(
                    "compare_trace_maze_result_raw net={net}, section={}, \
                         destination_type={destination_type}",
                    result.section_no_of_door
                ));
            }

            // Java calls the locator only when the search found
            // something; `locator::get_instance` answers None for a
            // None search the same way.
            locator::get_instance(
                &maze,
                search_result.as_ref(),
                &*maze.ctrl,
                angle_restriction,
                ripped_item_list,
                ripup_costs,
            )
        };
        self.drill_page_array = pages;

        // Always clean up expansion rooms from the search tree,
        // regardless of search outcome (Java `:204-209`, the v1.9
        // mirror comment).
        if !self.maintain_database {
            self.clear();
        } else {
            self.reset_all_doors();
        }

        let Some(autoroute_result) = autoroute_result else {
            // Java folds the maze-init failure into the same None; the
            // details literal of THIS arm is the no-connection one (the
            // init-failure literal is unreachable through the ported
            // composition — see the module docs).
            return AutorouteAttemptResult::with_details(
                AutorouteAttemptState::Failed,
                format!(
                    "Failed to route connection between {describe}, \
                     because no connection was found between their nets."
                ),
            );
        };

        if !ctrl.layer_active
            [usize::try_from(autoroute_result.start_layer).expect("start layer index")]
        {
            return AutorouteAttemptResult::with_details(
                AutorouteAttemptState::Failed,
                format!(
                    "Failed to route connection between {describe}, \
                     because some of their layers are disabled."
                ),
            );
        }
        if !ctrl.layer_active
            [usize::try_from(autoroute_result.target_layer).expect("target layer index")]
        {
            return AutorouteAttemptResult::with_details(
                AutorouteAttemptState::Failed,
                format!(
                    "Failed to route connection between {describe}, \
                     because some of their layers are disabled."
                ),
            );
        }

        // Java's `connectionItems == null` SKIPPED arm is
        // unrepresentable (the locator owns its item list).

        // Delete the ripped connections (Java `:230-249`).
        let stop_connection_option = if ctrl.remove_unconnected_vias {
            StopConnectionOption::None
        } else {
            StopConnectionOption::FanoutVia
        };
        // Java's `rippedItemList` is a TreeSet<Item> (descending id);
        // the map is ascending-keyed, so the walk is reversed.
        let ripped_keys: Vec<u64> = ripped_item_list.values().copied().collect();
        let mut ripped_connections: BTreeSet<ItemId> = BTreeSet::new();
        let mut changed_nets: BTreeSet<i32> = BTreeSet::new();
        for key in ripped_keys.iter().rev() {
            let id = ItemId::new(u32::try_from(*key).expect("item key fits u32"));
            for contact in
                get_connection_items(&*self.manager, self.board, id, stop_connection_option)
            {
                ripped_connections.insert(contact);
            }
            for net_number in self.item_nets(*key) {
                changed_nets.insert(*net_number);
            }
        }

        // Java brackets the mutations with the observer notifications
        // (`observersActivated = !board.observersActive()`); the Rust
        // board has no observer system — documented no-op seam.
        for id in ripped_connections.iter().rev() {
            remove_item_through_repository(self.manager, self.board, *id);
        }
        for net_number in &changed_nets {
            remove_trace_tails(
                self.manager,
                self.board,
                *net_number,
                stop_connection_option,
            );
        }

        let inserted = insert_found_connection(
            Some(&autoroute_result),
            self.manager,
            self.board,
            seam,
            ctrl,
            sink,
        );

        if inserted.is_none() {
            return AutorouteAttemptResult::with_details(
                AutorouteAttemptState::Failed,
                format!(
                    "Failed to route connection between {describe}, \
                     because the new connection could not be inserted."
                ),
            );
        }

        AutorouteAttemptResult::new(AutorouteAttemptState::Routed)
    }
}

// ---------------------------------------------------------------------------
// the NeighbourEngine impl
// ---------------------------------------------------------------------------

impl NeighbourEngine for AutorouteEngine<'_> {
    fn net_number(&self) -> i32 {
        self.net_number
    }

    fn generate_room_id_no(&mut self) -> i32 {
        self.id_counter += 1;
        self.id_counter
    }

    fn board_bounding_octagon(&self) -> epic_geometry::int_octagon::IntOctagon {
        TileShape::RegularTileShape(epic_geometry::regular_tile_shape::RegularTileShape::IntBox(
            self.board_box,
        ))
        .bounding_octagon()
        .expect("an IntBox always has a bounding octagon")
    }

    fn add_incomplete_expansion_room(
        &mut self,
        shape: TileShape,
        layer: i32,
        contained_shape: TileShape,
    ) -> u64 {
        let key = self.alloc_key();
        self.rooms
            .push(ExpansionRoom::new_incomplete(shape, layer, contained_shape));
        self.keys.push(key);
        key
    }

    fn remove_all_doors(&mut self, room_key: u64) {
        self.remove_all_doors_impl(room_key);
    }

    fn add_complete_free_space_room(&mut self, shape: TileShape, layer: i32, id: i32) -> u64 {
        let key = self.alloc_key();
        self.rooms
            .push(ExpansionRoom::new_complete_free_space(shape, layer, id));
        self.keys.push(key);
        self.pending_completed.push(key);
        key
    }

    fn overlapping_entries(&mut self, shape: &TileShape, layer: i32) -> Vec<TreeEntry> {
        let Some(leaves) = self.tree().query_candidates(shape, |a, b| a.cmp(&b)) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for leaf in leaves {
            let Some(stored) =
                NeighbourEngine::tree_shape(self, leaf.object_key, leaf.shape_index_in_object)
            else {
                continue;
            };
            if layer >= 0 && self.shape_layer(leaf.object_key, leaf.shape_index_in_object) != layer
            {
                continue;
            }
            if !SearchTree::entry_intersects(shape, &stored) {
                continue;
            }
            out.push(TreeEntry {
                object_key: leaf.object_key,
                shape_index_in_object: leaf.shape_index_in_object,
            });
        }
        out
    }

    fn object_id(&self, object_key: u64) -> i32 {
        if is_room_key(object_key) {
            self.resolve(object_key).map_or(0, ExpansionRoom::id)
        } else {
            i32::try_from(object_key).expect("item keys are u32 ids")
        }
    }

    fn is_trace_obstacle(&self, object_key: u64, net_number: i32) -> bool {
        self.view().is_trace_obstacle(object_key, net_number)
    }

    fn tree_shape(&self, object_key: u64, shape_index: u32) -> Option<TileShape> {
        if is_room_key(object_key) {
            return self.resolve(object_key).map(|room| room.shape().clone());
        }
        self.item_tree_shapes(object_key)
            .get(shape_index as usize)
            .cloned()
            .flatten()
    }

    fn complete_shape(
        &mut self,
        room_shape: Option<&TileShape>,
        contained: Option<&TileShape>,
        layer: i32,
        ignore_object: Option<u64>,
        ignore_shape: Option<&TileShape>,
    ) -> Vec<epic_index::complete_shape::IncompleteRoom> {
        let query = CompleteShapeQuery {
            room_shape,
            contained,
            layer,
            net_number: self.net_number,
            ignore_object,
            ignore_shape,
        };
        complete_shape(self.tree(), &self.view(), &query, &self.board_box)
    }

    fn tree_object_room(&self, object_key: u64) -> Option<u64> {
        is_room_key(object_key).then_some(object_key)
    }

    fn is_item(&self, object_key: u64) -> bool {
        !is_room_key(object_key)
    }

    /// Java `Item.isRoutable`: base FALSE; `Trace`/`Via` override with
    /// `!isUserFixed() && netCount() > 0`; `Pin` has NO override.
    fn item_is_routable(&self, object_key: u64) -> bool {
        let id = ItemId::new(u32::try_from(object_key).expect("item key fits u32"));
        self.board
            .get(id)
            .map(|entry| {
                matches!(entry.data, ItemData::Trace { .. } | ItemData::Via { .. })
                    && !matches!(entry.fixed, FixedState::UserFixed)
                    && !entry.nets.is_empty()
            })
            .unwrap_or(false)
    }

    /// Java `Item.isConnectable`: `(this instanceof Connectable) &&
    /// netCount() > 0` with `Connectable` implemented by
    /// {Pin, Via, Trace, ConductionArea}. INDEPENDENT of `isRoutable`.
    fn item_is_connectable(&self, object_key: u64) -> bool {
        let id = ItemId::new(u32::try_from(object_key).expect("item key fits u32"));
        self.board
            .get(id)
            .map(|entry| {
                matches!(
                    entry.data,
                    ItemData::Pin { .. } | ItemData::Trace { .. } | ItemData::Via { .. }
                ) && !entry.nets.is_empty()
            })
            .unwrap_or(false)
    }

    fn item_contains_net(&self, object_key: u64, net_number: i32) -> bool {
        self.item_nets(object_key).contains(&net_number)
    }

    fn item_shares_net(&self, first_key: u64, second_key: u64) -> bool {
        let first = self.item_nets(first_key);
        let second = self.item_nets(second_key);
        first.iter().any(|net| second.contains(net))
    }

    fn item_is_polyline_trace(&self, object_key: u64) -> bool {
        let id = ItemId::new(u32::try_from(object_key).expect("item key fits u32"));
        self.board
            .get(id)
            .is_some_and(|entry| matches!(entry.data, ItemData::Trace { .. }))
    }

    fn item_expansion_room(&mut self, object_key: u64, shape_index: u32) -> Option<u64> {
        if let Some(&cached) = self.obstacle_rooms.get(&(object_key, shape_index)) {
            return Some(cached);
        }
        let shape = NeighbourEngine::tree_shape(self, object_key, shape_index)?;
        let layer = self.shape_layer(object_key, shape_index);
        let key = self.alloc_key();
        self.rooms.push(ExpansionRoom::new_obstacle(
            object_key,
            shape_index,
            shape.clone(),
            layer,
        ));
        self.keys.push(key);
        // Java does NOT put ObstacleExpansionRoom into the search tree
        // (it is not a SearchTreeObject) — see the T7 pins doc.
        self.obstacle_rooms.insert((object_key, shape_index), key);
        Some(key)
    }

    fn trace_connection_shape(&self, object_key: u64, shape_index: u32) -> Option<TileShape> {
        // Java `Connectable.getTraceConnectionShape` dispatch: the base
        // `Item` answers the raw tree shape, `DrillItem` (Pin, Via) the
        // degenerate center box, `PolylineTrace` the corner connection
        // shape, `ConductionArea` the tree shape.
        let id = ItemId::new(u32::try_from(object_key).expect("item key fits u32"));
        let Some(entry) = self.board.get(id) else {
            return NeighbourEngine::tree_shape(self, object_key, shape_index);
        };
        match &entry.data {
            ItemData::Pin { .. } => {
                let center = self.board.pin_center(id)?;
                Some(TileShape::RegularTileShape(
                    epic_geometry::regular_tile_shape::RegularTileShape::IntBox(
                        TileShape::surrounding_point(&center),
                    ),
                ))
            }
            ItemData::Via { center, .. } => {
                let point = Point::int(*center);
                Some(TileShape::RegularTileShape(
                    epic_geometry::regular_tile_shape::RegularTileShape::IntBox(
                        TileShape::surrounding_point(&point),
                    ),
                ))
            }
            ItemData::Trace { lines, .. } => {
                epic_board::items::trace::connection_shape(lines, shape_index as i32)
            }
            ItemData::ConductionArea { .. } => {
                NeighbourEngine::tree_shape(self, object_key, shape_index)
            }
            _ => NeighbourEngine::tree_shape(self, object_key, shape_index),
        }
    }

    fn trace_first_or_last_parallel(
        &self,
        item_key: u64,
        index_in_item: u32,
        door_line: &epic_geometry::line::Line,
    ) -> Option<bool> {
        // Java `SortedRoomNeighbours.insertDoorOk` (`:375-389`): a
        // trace obstacle section answers the door-line parallelism
        // only for the first and the last tile shape; everything else
        // falls through to `true` (the `None` here).
        let id = ItemId::new(u32::try_from(item_key).expect("item key fits u32"));
        let entry = self.board.get(id)?;
        let ItemData::Trace { lines, .. } = &entry.data else {
            return None;
        };
        // Java `PolylineTrace.tileShapeCount()` = corner count - 1;
        // the stored polyline carries BOTH perpendicular placeholders,
        // so the section count is `lines.len() - 2`.
        let tile_shape_count = lines.lines.len().saturating_sub(2);
        let index = index_in_item as usize;
        if index != 0 && index + 1 != tile_shape_count {
            return None;
        }
        let trace_line = lines
            .lines
            .get(index + 1)
            .expect("a first/last section index has a polyline line");
        Some(trace_line.is_parallel(door_line))
    }

    fn room_shape(&self, room_key: u64) -> TileShape {
        self.resolve(room_key)
            .unwrap_or_else(|| panic!("room key {room_key} not registered"))
            .shape()
            .clone()
    }

    fn room_layer(&self, room_key: u64) -> i32 {
        self.resolve(room_key)
            .unwrap_or_else(|| panic!("room key {room_key} not registered"))
            .layer()
    }

    fn room_id(&self, room_key: u64) -> i32 {
        self.resolve(room_key)
            .unwrap_or_else(|| panic!("room key {room_key} not registered"))
            .id()
    }

    fn room_is_incomplete(&self, room_key: u64) -> bool {
        self.resolve(room_key)
            .is_some_and(ExpansionRoom::is_incomplete)
    }

    fn room_is_obstacle(&self, room_key: u64) -> bool {
        self.resolve(room_key)
            .is_some_and(ExpansionRoom::is_obstacle)
    }

    fn room_is_complete_free_space(&self, room_key: u64) -> bool {
        self.resolve(room_key)
            .is_some_and(ExpansionRoom::is_complete_free_space)
    }

    fn room_contained_shape(&self, room_key: u64) -> Option<TileShape> {
        self.resolve(room_key)
            .and_then(ExpansionRoom::contained_shape)
            .cloned()
    }

    fn room_obstacle_item_key(&self, room_key: u64) -> Option<u64> {
        match self.resolve(room_key).map(|room| &room.kind) {
            Some(RoomKind::Obstacle { item_key, .. }) => Some(*item_key),
            _ => None,
        }
    }

    fn room_obstacle_index_in_item(&self, room_key: u64) -> Option<u32> {
        match self.resolve(room_key).map(|room| &room.kind) {
            Some(RoomKind::Obstacle { index_in_item, .. }) => Some(*index_in_item),
            _ => None,
        }
    }

    fn room_has_door_to(&self, room_key: u64, other_room_id: i32) -> bool {
        self.resolve(room_key)
            .is_some_and(|room| room.door_exists(other_room_id))
    }

    fn room_doors(&self, room_key: u64) -> Vec<ExpansionDoor> {
        self.resolve(room_key)
            .map(|room| room.doors().to_vec())
            .unwrap_or_default()
    }

    fn room_key_of_id(&self, id: i32) -> Option<u64> {
        AutorouteEngine::room_key_of_id(self, id)
    }

    fn set_incomplete_shape(
        &mut self,
        room_key: u64,
        shape: TileShape,
        contained_shape: TileShape,
    ) {
        let room = self.room_mut(room_key);
        room.set_shape(shape);
        room.set_contained_shape(contained_shape);
    }

    fn set_room_shape(&mut self, room_key: u64, shape: TileShape) {
        self.room_mut(room_key).set_shape(shape);
    }

    fn attach_door(&mut self, room_key: u64, door: ExpansionDoor) {
        self.room_mut(room_key).add_door(door);
    }

    fn add_target_door(&mut self, room_key: u64, door: TargetItemExpansionDoor) {
        self.room_mut(room_key).add_target_door(door);
    }

    fn set_net_dependent(&mut self, room_key: u64) {
        self.room_mut(room_key).set_net_dependent(true);
    }

    // ---- T6 seams ----
    fn tree_variant(&self) -> SearchTreeVariant {
        self.resolved_variant
    }

    fn remove_incomplete_room(&mut self, room_key: u64) {
        self.remove_room(room_key);
    }

    fn flush_completed_inserts(&mut self, accepted: Option<u64>) {
        // The inherent helper (inherent resolution wins over the trait
        // method — not recursion).
        AutorouteEngine::flush_completed_inserts(self, accepted);
    }

    fn room_target_doors(&self, room_key: u64) -> Vec<TargetItemExpansionDoor> {
        self.resolve(room_key)
            .map(|room| room.target_doors().to_vec())
            .unwrap_or_default()
    }

    fn room_obstacle_doors_calculated(&self, room_key: u64) -> bool {
        self.obstacle_doors_calculated.contains(&room_key)
    }

    fn set_room_doors_calculated(&mut self, room_key: u64) {
        self.obstacle_doors_calculated.insert(room_key);
    }
}

impl CompleteShapeObjects for AutorouteEngine<'_> {
    fn is_trace_obstacle(&self, object_key: u64, net_number: i32) -> bool {
        self.view().is_trace_obstacle(object_key, net_number)
    }

    fn shape_layer(&self, object_key: u64, shape_index: u32) -> i32 {
        self.view().shape_layer(object_key, shape_index)
    }

    fn tree_shape(&self, object_key: u64, shape_index: u32) -> Option<TileShape> {
        self.view().tree_shape(object_key, shape_index)
    }

    fn is_complete_free_space(&self, object_key: u64) -> bool {
        self.view().is_complete_free_space(object_key)
    }
}

// ---------------------------------------------------------------------------
// the DrillEngine impl
// ---------------------------------------------------------------------------

impl DrillEngine for AutorouteEngine<'_> {
    fn board_bounds(&self) -> IntBox {
        self.board_box
    }

    fn layer_count(&self) -> i32 {
        i32::try_from(self.board.layers().layers.len()).expect("layer count fits i32")
    }

    fn stop_flag(&self) -> Option<&AtomicBool> {
        self.stoppable_flag.as_deref()
    }

    fn overlapping_items(&self, shape: &TileShape, layer: i32) -> Vec<u64> {
        // Java `board.overlappingItems` — the tree's TreeSet leaf
        // order (object id DESCENDING), items only.
        let Some(leaves) = self.tree().query_candidates(shape, |a, b| a.cmp(&b)) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for leaf in leaves {
            if is_room_key(leaf.object_key) {
                continue;
            }
            let Some(stored) = self
                .item_tree_shapes(leaf.object_key)
                .get(leaf.shape_index_in_object as usize)
                .cloned()
                .flatten()
            else {
                continue;
            };
            if layer >= 0
                && CompleteShapeObjects::shape_layer(
                    self,
                    leaf.object_key,
                    leaf.shape_index_in_object,
                ) != layer
            {
                continue;
            }
            if !SearchTree::entry_intersects(shape, &stored) {
                continue;
            }
            out.push(leaf.object_key);
        }
        out.sort_by(|a, b| b.cmp(a));
        out.dedup();
        out
    }

    fn item_is_drillable(&self, item_key: u64, net_number: i32) -> bool {
        // Java `Item.isDrillable(netNumber)`: base false; Trace
        // overrides to `containsNet`; ConductionArea to
        // `!isObstacle || containsNet`.
        let id = ItemId::new(u32::try_from(item_key).expect("item key fits u32"));
        let Some(entry) = self.board.get(id) else {
            return false;
        };
        match &entry.data {
            ItemData::Trace { .. } => entry.nets.contains(&net_number),
            ItemData::ConductionArea { is_obstacle, .. } => {
                !*is_obstacle || entry.nets.contains(&net_number)
            }
            _ => false,
        }
    }

    fn item_is_pin(&self, item_key: u64) -> bool {
        let id = ItemId::new(u32::try_from(item_key).expect("item key fits u32"));
        self.board
            .get(id)
            .is_some_and(|entry| matches!(entry.data, ItemData::Pin { .. }))
    }

    fn pin_drill_allowed(&self, item_key: u64) -> bool {
        // Java `Pin.drillAllowed()`: `firstLayer() == lastLayer()` — a
        // single-layer (SMD) pin.
        let id = ItemId::new(u32::try_from(item_key).expect("item key fits u32"));
        let Some(entry) = self.board.get(id) else {
            return false;
        };
        match &entry.data {
            ItemData::Pin { padstack_no, .. } => {
                let (first, last) = self.padstack_layer_span(*padstack_no);
                first == last
            }
            _ => false,
        }
    }

    fn pin_center(&self, item_key: u64) -> Option<Point> {
        let id = ItemId::new(u32::try_from(item_key).expect("item key fits u32"));
        self.board.pin_center(id)
    }

    fn item_is_via(&self, item_key: u64) -> bool {
        let id = ItemId::new(u32::try_from(item_key).expect("item key fits u32"));
        self.board
            .get(id)
            .is_some_and(|entry| matches!(entry.data, ItemData::Via { .. }))
    }

    fn item_padstack_no(&self, item_key: u64) -> Option<i32> {
        let id = ItemId::new(u32::try_from(item_key).expect("item key fits u32"));
        self.board.get(id).and_then(|entry| match &entry.data {
            ItemData::Via { padstack_no, .. } => Some(*padstack_no),
            _ => None,
        })
    }

    fn item_clearance_class(&self, item_key: u64) -> i32 {
        let id = ItemId::new(u32::try_from(item_key).expect("item key fits u32"));
        self.board
            .get(id)
            .map(|entry| entry.clearance_class)
            .unwrap_or(0)
    }

    fn padstack_layer_span(&self, padstack_no: i32) -> (i32, i32) {
        let padstack = self
            .board
            .library()
            .padstack(padstack_no)
            .unwrap_or_else(|| panic!("padstack {padstack_no} absent"));
        (
            i32::try_from(padstack.from_layer()).unwrap_or(0),
            padstack.to_layer(),
        )
    }

    fn padstack_shape(
        &self,
        padstack_no: i32,
        layer: i32,
    ) -> Option<epic_board::items::BoardShape> {
        self.board
            .library()
            .padstack(padstack_no)
            .and_then(|padstack| padstack.get_shape(usize::try_from(layer).unwrap_or(0)))
            .cloned()
    }

    fn via_rule_vias(&self) -> Vec<ViaRuleVia> {
        let rules = self.board.rules();
        let class_index = rules
            .nets
            .get(self.net_number)
            .map(|net| net.net_class)
            .unwrap_or(0);
        let Some(class) = rules.net_class(class_index) else {
            return Vec::new();
        };
        let Some(rule_id) = class.via_rule else {
            return Vec::new();
        };
        let Some(rule) = rules.via_rule_by_id(rule_id) else {
            return Vec::new();
        };
        rule.via_infos
            .iter()
            .filter_map(|&index| {
                rules
                    .via_infos
                    .get(usize::try_from(index).ok()?)
                    .map(|info| ViaRuleVia {
                        padstack_no: info.padstack_no,
                        clearance_class: info.clearance_class,
                        attach_smd_allowed: info.attach_smd_allowed,
                    })
            })
            .collect()
    }

    // ---- T6 seams (the maze engine's item/layer reads) ----
    fn item_is_destination(&self, item_key: u64) -> bool {
        // Java `!ItemAutorouteInfo.isStartInfo()`.
        !self.start_infos.contains(&item_key)
    }

    fn set_item_start_info(&mut self, item_key: u64, start: bool) {
        if start {
            self.start_infos.insert(item_key);
        } else {
            self.start_infos.remove(&item_key);
        }
    }

    fn item_tree_shape_count(&self, item_key: u64) -> i32 {
        i32::try_from(self.item_tree_shapes(item_key).len()).expect("shape count fits i32")
    }

    fn pin_neckdown_half_width(&self, item_key: u64, layer: i32) -> f64 {
        // Java `Pin.getTraceNeckdownHalfwidth(layer)` (`:511-515`):
        // `(int) Math.max(0.5*getMinWidth(layer) - 1, 1)`; the caller
        // casts to Pin after the instanceof gate, so the non-pin arm
        // is unreachable (Java would ClassCastException).
        let id = ItemId::new(u32::try_from(item_key).expect("item key fits u32"));
        let is_pin = self
            .board
            .get(id)
            .is_some_and(|entry| matches!(entry.data, ItemData::Pin { .. }));
        if is_pin {
            f64::from(self.board.pin_trace_neckdown_halfwidth(id, layer))
        } else {
            0.0
        }
    }

    fn layer_is_signal(&self, layer: i32) -> bool {
        let index = usize::try_from(layer).expect("layer index");
        self.board.layers().layers[index].is_signal
    }

    fn drill_hits_foreign_conduction(
        &self,
        _location: &Point,
        _layer: i32,
        _net_number: i32,
    ) -> bool {
        // SEAM bank (T6): the inactive-layer drill-vs-plane probe —
        // the fixtures route without conduction areas; the plane
        // milestone owns the real body.
        false
    }

    fn pin_nearest_trace_exit_corner(
        &self,
        item_key: u64,
        from_point: &FloatPoint,
        trace_half_width: i32,
        layer: i32,
    ) -> Option<FloatPoint> {
        // Java `MazeExpansionEngine.expandToDrill:56-65` ->
        // `Pin.nearestTraceExitCorner` (`Pin.java:636-668`) — the
        // production port (pinned against the MazeExitProbe capture in
        // the T6/T10 pins).
        let id = ItemId::new(u32::try_from(item_key).expect("item key fits u32"));
        let entry = self.board.get(id)?;
        let ItemData::Pin {
            pin_index,
            padstack_no,
        } = &entry.data
        else {
            return None;
        };
        let component_id = u32::try_from(entry.component_id).ok()?;
        let component = self.board.components().get(component_id)?;
        let package = self.board.library().package(component.package_no())?;
        let package_pin = package.get_pin(*pin_index)?;
        let padstack = self.board.library().padstack(*padstack_no)?;
        // Java `DrillItem.firstLayer()`.
        let first_layer = if component.placed_on_front() || padstack.placed_absolute {
            padstack.from_layer() as i32
        } else {
            padstack.board_layer_count() as i32 - padstack.to_layer() - 1
        };
        let shape_index = layer - first_layer;
        // Java `Pin.getTraceExitRestrictions(layer)` — only the
        // DIRECTIONS reach the corner loop. The padstack layer shape
        // must be an IntBox or IntOctagon.
        let padstack_layer = if component.placed_on_front() || padstack.placed_absolute {
            shape_index + first_layer
        } else {
            padstack.board_layer_count() as i32 - shape_index - first_layer - 1
        };
        let raw = padstack.get_shape(usize::try_from(padstack_layer).ok()?)?;
        let tile = match raw {
            epic_board::items::BoardShape::Tile(tile_shape)
                if !matches!(tile_shape, TileShape::Simplex(_)) =>
            {
                tile_shape
            }
            _ => return None,
        };
        let bounds = tile.bounding_box();
        let width = f64::from(bounds.ur.x - bounds.ll.x);
        let height = f64::from(bounds.ur.y - bounds.ll.y);
        let mut pad_xy_factor = 1.5;
        if package.pins.len() <= 3 {
            pad_xy_factor *= 2.0; // allow the longer side also for short pads
        }
        let all_dirs = width.max(height) < pad_xy_factor * width.min(height);
        let mut directions = Vec::new();
        if all_dirs || width >= height {
            directions.push(epic_geometry::direction::Direction::RIGHT);
            directions.push(epic_geometry::direction::Direction::LEFT);
        }
        if all_dirs || width <= height {
            directions.push(epic_geometry::direction::Direction::UP);
            directions.push(epic_geometry::direction::Direction::DOWN);
        }
        // The component+package rotation arm (`Pin.java:302-313`).
        let rotation = component.rotation_in_degree + package_pin.rotation;
        let directions: Vec<epic_geometry::direction::Direction> = directions
            .iter()
            .map(|base| {
                if rotation % 45.0 == 0.0 {
                    base.clone().turn_45_degree((rotation as i32) / 45)
                } else {
                    epic_geometry::direction::Direction::get_instance_approx(
                        rotation.to_radians() + base.angle_approx(),
                    )
                }
            })
            .collect();
        if directions.is_empty() {
            return None;
        }
        let pin_shape = self.board.drill_shape(id, shape_index)?;
        let epic_board::items::BoardShape::Tile(tile_shape) = pin_shape else {
            return None;
        };
        let edge_to_turn_dist = self.board.rules().pin_edge_to_turn_dist;
        if edge_to_turn_dist < 0.0 {
            return None;
        }
        let offset_shape = tile_shape.offset(edge_to_turn_dist + f64::from(trace_half_width));
        let center = self.board.pin_center(id)?;
        let mut best: Option<(f64, FloatPoint)> = None;
        for direction in &directions {
            let border_no = offset_shape.intersecting_border_line_no(&center, direction);
            if border_no < 0 {
                continue;
            }
            let ray =
                epic_geometry::line::Line::new_with_direction(center.clone(), direction.clone());
            let corner = ray.intersection_approx(&offset_shape.border_line(border_no));
            let distance = corner.distance_square(from_point);
            if best.is_none_or(|(best_distance, _)| distance < best_distance) {
                best = Some((distance, corner));
            }
        }
        best.map(|(_, corner)| corner)
    }

    fn item_is_trace(&self, item_key: u64) -> bool {
        let id = ItemId::new(u32::try_from(item_key).expect("item key fits u32"));
        self.board
            .get(id)
            .is_some_and(|entry| matches!(entry.data, ItemData::Trace { .. }))
    }

    fn item_trace_half_width(&self, item_key: u64) -> i32 {
        let id = ItemId::new(u32::try_from(item_key).expect("item key fits u32"));
        self.board.get(id).map_or(0, |entry| match &entry.data {
            ItemData::Trace { half_width, .. } => *half_width,
            _ => 0,
        })
    }

    fn clearance_compensation_value(&self, clearance_class: i32, layer: i32) -> i32 {
        epic_board::tree_shapes::clearance_compensation_value(
            self.board.rules(),
            clearance_class,
            self.tree().compensated_clearance_class,
            layer,
        )
    }

    fn item_shape_layer(&self, item_key: u64, shape_index: u32) -> i32 {
        CompleteShapeObjects::shape_layer(self, item_key, shape_index)
    }

    fn item_tree_shape_on_layer(&self, item_key: u64, layer: i32) -> Option<TileShape> {
        let shapes = self.item_tree_shapes(item_key);
        for (index, shape) in shapes.iter().enumerate() {
            let index = u32::try_from(index).ok()?;
            if shape.is_some() && CompleteShapeObjects::shape_layer(self, item_key, index) == layer
            {
                return shape.clone();
            }
        }
        None
    }

    fn trace_angle_restriction(&self) -> AngleRestriction {
        router_angle_restriction(self.board.rules().trace_angle_restriction)
    }

    // ---- T7 seams (the ripup resolver + the read-only shove probe) ----

    fn item_normal_contacts(&mut self, item_key: u64) -> Vec<u64> {
        epic_board::contacts::item_normal_contacts(
            &*self.manager,
            self.board,
            ItemId::new(u32::try_from(item_key).expect("item key fits u32")),
        )
        .into_iter()
        .map(|id| u64::from(id.get()))
        .collect()
    }

    fn trace_normal_contacts_at(
        &mut self,
        item_key: u64,
        point: &Point,
        ignore_net: bool,
    ) -> Vec<u64> {
        epic_board::contacts::normal_contacts(
            &*self.manager,
            self.board,
            ItemId::new(u32::try_from(item_key).expect("item key fits u32")),
            point,
            ignore_net,
        )
        .into_iter()
        .map(|id| u64::from(id.get()))
        .collect()
    }

    fn trace_start_contacts(&mut self, item_key: u64) -> Vec<u64> {
        epic_board::contacts::start_contacts(
            &*self.manager,
            self.board,
            ItemId::new(u32::try_from(item_key).expect("item key fits u32")),
        )
        .into_iter()
        .map(|id| u64::from(id.get()))
        .collect()
    }

    fn trace_end_contacts(&mut self, item_key: u64) -> Vec<u64> {
        epic_board::contacts::end_contacts(
            &*self.manager,
            self.board,
            ItemId::new(u32::try_from(item_key).expect("item key fits u32")),
        )
        .into_iter()
        .map(|id| u64::from(id.get()))
        .collect()
    }

    fn normal_contact_point(&mut self, first_key: u64, second_key: u64) -> Option<Point> {
        epic_board::trace_ops::normal_contact_point(
            self.board,
            ItemId::new(u32::try_from(first_key).expect("item key fits u32")),
            ItemId::new(u32::try_from(second_key).expect("item key fits u32")),
        )
    }

    fn first_common_layer(&mut self, first_key: u64, second_key: u64) -> i32 {
        epic_board::trace_ops::first_common_layer(
            self.board,
            ItemId::new(u32::try_from(first_key).expect("item key fits u32")),
            ItemId::new(u32::try_from(second_key).expect("item key fits u32")),
        )
    }

    fn item_is_user_fixed(&self, item_key: u64) -> bool {
        let id = ItemId::new(u32::try_from(item_key).expect("item key fits u32"));
        self.board
            .get(id)
            .is_some_and(|entry| matches!(entry.fixed, FixedState::UserFixed))
    }

    fn item_is_shove_fixed(&self, item_key: u64) -> bool {
        let id = ItemId::new(u32::try_from(item_key).expect("item key fits u32"));
        self.board
            .get(id)
            .is_some_and(|entry| matches!(entry.fixed, FixedState::ShoveFixed))
    }

    fn item_trace_corner_count(&self, item_key: u64) -> i32 {
        // Java `Polyline.size()` — `lines.length`, the CORNER count.
        i32::try_from(
            self.item_trace_polyline(item_key)
                .map_or(0, |polyline| polyline.lines.len()),
        )
        .expect("corner count fits i32")
    }

    fn item_trace_length(&self, item_key: u64) -> f64 {
        self.item_trace_polyline(item_key)
            .map_or(0.0, |polyline| polyline.length_approx_total())
    }

    fn item_trace_polyline(&self, item_key: u64) -> Option<epic_geometry::polyline::Polyline> {
        self.board
            .trace_polyline(ItemId::new(
                u32::try_from(item_key).expect("item key fits u32"),
            ))
            .cloned()
    }

    fn overlapping_objects_ignore_nets(
        &self,
        shape: &TileShape,
        layer: i32,
        ignore_nets: &[i32],
    ) -> Vec<u64> {
        // Java `ShapeSearchTree.overlappingObjects(shape, layer,
        // ignoreNetNos)` (`:404-478`): the descending-id leaf walk,
        // layer filter, intersection test, then the `:412-413`
        // obstacle filter — an object is kept only when it is an
        // obstacle w.r.t. EVERY ignore net.
        let Some(leaves) = self.tree().query_candidates(shape, |a, b| a.cmp(&b)) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for leaf in leaves {
            // The harness lift reads BOTH kinds through the shared
            // tree-shape seam (rooms answer their registry shape).
            let Some(stored) =
                NeighbourEngine::tree_shape(self, leaf.object_key, leaf.shape_index_in_object)
            else {
                continue;
            };
            if layer >= 0
                && CompleteShapeObjects::shape_layer(
                    self,
                    leaf.object_key,
                    leaf.shape_index_in_object,
                ) != layer
            {
                continue;
            }
            if !SearchTree::entry_intersects(shape, &stored) {
                continue;
            }
            if ignore_nets
                .iter()
                .all(|&net| NeighbourEngine::is_trace_obstacle(self, leaf.object_key, net))
            {
                out.push(leaf.object_key);
            }
        }
        out.sort_by(|a, b| b.cmp(a));
        out.dedup();
        out
    }

    fn check_trace_segment(
        &mut self,
        line_segment: &LineSegment,
        layer: i32,
        net_numbers: &[i32],
        half_width: i32,
        clearance_class: i32,
        cushions_enabled: bool,
    ) -> f64 {
        // The PRODUCTION body (Java `RoutingBoard.checkTraceSegment`):
        // the read-only clearance probe. The last flag is the Java
        // `onlyNotShovableObstacles` arm of the same overload family.
        check_trace_segment_points(
            self.manager,
            self.board,
            &line_segment.start_point(),
            &line_segment.end_point(),
            layer,
            net_numbers,
            half_width,
            clearance_class,
            cushions_enabled,
        )
    }

    fn via_layer_check(
        &mut self,
        ctrl: &crate::control::AutorouteControl,
        _checker: &mut impl ViaLayerChecker,
        required_radius: f64,
        clearance_class: i32,
        attach_smd_allowed: bool,
        room_shape: &TileShape,
        location: &Point,
        layer: i32,
        net_number: i32,
    ) -> RouterCheckDrillResult {
        // The PRODUCTION body: Java calls ForcedViaInserter.checkLayer
        // directly on the board; the checker parameter is ignored (the
        // capture/test engines keep the trait default). The literal
        // maxViaRecursionDepth 0 is the Java call-site literal
        // (MazeExpansionEngine.java:391-403) — NOT
        // ctrl.max_shove_via_recursion_depth.
        let net_numbers = [net_number];
        let _ = ctrl;
        match epic_board::forced_via_inserter::check_layer(
            self.manager,
            self.board,
            required_radius,
            clearance_class,
            attach_smd_allowed,
            room_shape,
            location,
            layer,
            &net_numbers,
            ctrl.max_shove_trace_recursion_depth,
            0, // Java literal — see the comment above
            ctrl.trace_half_width[layer as usize],
            ctrl.trace_clearance_class_index,
        ) {
            BoardCheckDrillResult::Drillable => RouterCheckDrillResult::Drillable,
            BoardCheckDrillResult::DrillableWithAttachSmd => {
                RouterCheckDrillResult::DrillableWithAttachSmd
            }
            BoardCheckDrillResult::NotDrillable => RouterCheckDrillResult::NotDrillable,
        }
    }

    // `shove_trace_check` — the trait DEFAULT (0.0, the MazeTraceShover
    // bank) is kept: the maze probe short-circuits to an empty shove
    // list, matching a board with nothing shovable. The MazeTraceShover
    // port is a banked SEAM debt.
    // `complete_expansion_room` — the trait DEFAULT composing the
    // completion primitives (`crate::maze::completion`).
}

// ---------------------------------------------------------------------------
// string helpers
// ---------------------------------------------------------------------------

/// Java `ExpandableObject.getClass().getSimpleName()` for the
/// maze-result row: `ExpansionDoor`, `TargetItemExpansionDoor`,
/// `DrillPage`, and — for BOTH drill tags — `ExpansionDrill` (Java has
/// one `ExpansionDrill` class; the Rust `StandaloneDrill` tag is the
/// ripped-via drill).
fn expandable_object_type_name(object: &ExpandableObject) -> &'static str {
    match object {
        ExpandableObject::RoomDoor(_) => "ExpansionDoor",
        ExpandableObject::TargetDoor(_) => "TargetItemExpansionDoor",
        ExpandableObject::DrillPage { .. } => "DrillPage",
        ExpandableObject::Drill { .. } | ExpandableObject::StandaloneDrill { .. } => {
            "ExpansionDrill"
        }
    }
}

/// Java `Item.toString()` (`Item.java:1302-1313`) + the `Pin` override
/// (`Pin.java:676-690`): the lowercase class simple name, for a pin
/// with a positive index the ` #index` suffix, and for
/// `componentId > 0` the ` of component #id` suffix. Traces are
/// `PolylineTrace` → `polylinetrace`.
fn item_to_string(board: &Board, key: u64) -> String {
    let id = ItemId::new(u32::try_from(key).expect("item key fits u32"));
    let entry = board.get(id).expect("a routed set holds live items");
    let mut name = match &entry.data {
        ItemData::Pin { pin_index, .. } => {
            let mut name = String::from("pin");
            if *pin_index > 0 {
                name.push_str(&format!(" #{pin_index}"));
            }
            name
        }
        ItemData::Trace { .. } => "polylinetrace".to_string(),
        ItemData::Via { .. } => "via".to_string(),
        ItemData::ObstacleArea { .. } => "obstaclearea".to_string(),
        ItemData::ConductionArea { .. } => "conductionarea".to_string(),
        ItemData::ComponentOutline { .. } => "componentoutline".to_string(),
        ItemData::BoardOutline { .. } => "boardoutline".to_string(),
        // Java prints the concrete class lowercase; the ported model
        // folds the non-routed residue into `Other` — unreachable for
        // the routed start/destination sets the describe reads.
        ItemData::Other => "item".to_string(),
    };
    if entry.component_id > 0 {
        name.push_str(&format!(" of component #{}", entry.component_id));
    }
    name
}

/// Java `describeConnection`: the start set's items joined with
/// `", "`, then `" and "`, then the destination set.
fn describe_connection(board: &Board, start_items: &[u64], dest_items: &[u64]) -> String {
    let start = start_items
        .iter()
        .map(|&key| item_to_string(board, key))
        .collect::<Vec<_>>()
        .join(", ");
    let dest = dest_items
        .iter()
        .map(|&key| item_to_string(board, key))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{start} and {dest}")
}

// ---------------------------------------------------------------------------
// the lifecycle faces (Java RoutingBoard.initAutoroute/finishAutoroute)
// ---------------------------------------------------------------------------

/// Java `RoutingBoard.initAutoroute` (`:882-897`): reuse the slot's
/// engine iff the database is retained AND its tree is compensated for
/// the same clearance class; otherwise replace it. NOTE the Java face:
/// a replaced engine is dropped WITHOUT `clear()` — its completed
/// rooms legitimately remain in the shared (same-class) tree, exactly
/// as in Java; the pipeline always calls [`finish_autoroute`] before
/// re-initing, so the phantom-room state is unreachable there. Either
/// way the returned engine is [`Self::init_connection`]-primed.
#[allow(clippy::too_many_arguments)] // the Java signature, kept 1:1
pub fn init_autoroute<'a>(
    manager: &'a mut SearchTreeManager,
    board: &'a mut Board,
    slot: Option<AutorouteEngine<'a>>,
    net_number: i32,
    trace_clearance_class_index: i32,
    retain_database: bool,
    stoppable_flag: Option<Arc<AtomicBool>>,
    time_limit: Option<TimeLimit>,
) -> AutorouteEngine<'a> {
    let reuse = match &slot {
        Some(engine) => {
            retain_database
                && manager.trees()[engine.autoroute_tree].compensated_clearance_class
                    == trace_clearance_class_index
        }
        None => false,
    };
    match slot {
        Some(mut engine) if reuse => {
            engine.init_connection(net_number, stoppable_flag, time_limit);
            engine
        }
        Some(engine) => {
            // Java: the old engine is replaced without clear(); its
            // rooms stay in the shared tree (see the doc above).
            drop(engine);
            let mut engine =
                AutorouteEngine::new(manager, board, trace_clearance_class_index, retain_database);
            engine.init_connection(net_number, stoppable_flag, time_limit);
            engine
        }
        None => {
            let mut engine =
                AutorouteEngine::new(manager, board, trace_clearance_class_index, retain_database);
            engine.init_connection(net_number, stoppable_flag, time_limit);
            engine
        }
    }
}

/// Java `RoutingBoard.finishAutoroute` (`:900-905`): clear the
/// temporary database and drop the engine.
pub fn finish_autoroute(engine: Option<AutorouteEngine<'_>>) {
    if let Some(mut engine) = engine {
        engine.clear();
    }
}

// ---------------------------------------------------------------------------
// structural tests (the jar-probe pin battery lands with Phase 5)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::RouterSettingsIr;
    use crate::path::inserter::{CaptureSink, NullSink};
    use crate::test_util::parse;
    use epic_board::routing_board_insert::NoPullTight;
    use epic_geometry::polyline::Polyline;

    /// The T9/T10c locator-world fixture (net number 94's single pin
    /// — name D093 — at (663500, 20000) DB, the y=300000 corridor,
    /// F.Cu keepouts at x 480000-520000).
    fn parse_fixture() -> (SearchTreeManager, Board) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../harness/fixtures/locator-spike/t9_locator45.dsn");
        let text = std::fs::read_to_string(&path).expect("fixture present");
        parse(&text)
    }

    fn settings_ir(layer_count: usize) -> RouterSettingsIr {
        // The jar world's cost table (capture row `ctrl_costs`,
        // logs/M3-T11/captures/autoroute_engine_rows_run1.jsonl): the
        // probe builds ctrl from `new RouterSettings(board)`, whose
        // applyBoardSpecificOptimizations derives preferred cost 1.0
        // and undesired 1.0 + 0.1*round(10*W/H) per direction,
        // alternating preferred direction per signal layer starting
        // horizontal on F.Cu. For this fixture: F.Cu 1.0/2.7,
        // B.Cu 1.6/1.0, via 1, bend 0, neckdown off. (The pre-capture
        // default `ExpansionCostFactor::default()` = 0/0 made every
        // path free and the maze kept a first-found non-Jar route.)
        assert_eq!(layer_count, 2, "the capture table is 2-layer");
        RouterSettingsIr {
            trace_costs: vec![
                crate::control::ExpansionCostFactor {
                    horizontal: 1.0,
                    vertical: 2.7,
                },
                crate::control::ExpansionCostFactor {
                    horizontal: 1.6,
                    vertical: 1.0,
                },
            ],
            via_costs: 1,
            vias_allowed: true,
            bend_costs: vec![0.0; layer_count],
            layer_active: vec![true; layer_count],
            automatic_neckdown: false,
            start_ripup_costs: 1,
        }
    }

    fn corner(x: i32, y: i32) -> epic_geometry::int_point::IntPoint {
        epic_geometry::int_point::IntPoint::new(x, y)
    }

    /// The net-N pin key (for number 94 that is name D093 — the
    /// fixture's only item of that net).
    fn net_pin_key(board: &Board, net: i32) -> u64 {
        board
            .iter_ascending()
            .find(|entry| entry.nets.contains(&net) && matches!(entry.data, ItemData::Pin { .. }))
            .map(|entry| u64::from(entry.id.get()))
            .unwrap_or_else(|| panic!("net {net} has a pin"))
    }

    /// The engine + ctrl for one net (retain = maintain).
    fn build_engine<'a>(
        manager: &'a mut SearchTreeManager,
        board: &'a mut Board,
        net: i32,
        maintain: bool,
    ) -> (AutorouteEngine<'a>, AutorouteControl) {
        let ctrl = AutorouteControl::new(board, net, &settings_ir(board.layers().layers.len()));
        let class = ctrl.trace_clearance_class_index;
        let engine = AutorouteEngine::new(manager, board, class, maintain);
        (engine, ctrl)
    }

    /// The const-equality pin for the shared room-key space (the
    /// engine's registry and the T5 harness replay must never disagree
    /// on the item/room boundary).
    #[test]
    fn t11_room_key_base_const_equality() {
        assert_eq!(ROOM_KEY_BASE, crate::drill::pins::ROOM_KEY_BASE);
        assert_eq!(ROOM_KEY_BASE, 1u64 << 40);
    }

    /// The ctor faces: net -1 before the first init_connection, the
    /// page array built FROM the engine at the clamped width, and the
    /// registry empty (items live in the MANAGER tree only).
    #[test]
    fn t11_ctor_faces() {
        let (mut manager, mut board) = parse_fixture();
        let width = max_drill_page_width(board.default_via_diameter());
        let (engine, _ctrl) = build_engine(&mut manager, &mut board, 94, true);
        assert_eq!(engine.net_number(), -1, "Java ctor leaves netNumber -1");
        assert!(
            engine.rooms.is_empty() && engine.keys.is_empty(),
            "no rooms yet"
        );
        assert!(engine.maintain_database);
        let page_width = engine.drill_page_array.page_width;
        assert!(page_width > 0 && page_width <= width, "page grid built");
        // init_connection assigns the net.
        let mut engine = engine;
        engine.init_connection(94, None, None);
        assert_eq!(engine.net_number(), 94);
    }

    /// The stop faces: no flag/limit → false; an already-exceeded time
    /// limit → true; a raised stoppable flag → true.
    #[test]
    fn t11_is_stop_requested_faces() {
        let (mut manager, mut board) = parse_fixture();
        let (mut engine, _ctrl) = build_engine(&mut manager, &mut board, 94, true);
        assert!(!engine.is_stop_requested(), "no limit, no flag");
        engine.time_limit = Some(TimeLimit::new(-1));
        assert!(engine.is_stop_requested(), "an expired limit stops");
        engine.time_limit = None;
        let flag = Arc::new(AtomicBool::new(false));
        engine.stoppable_flag = Some(Arc::clone(&flag));
        assert!(!engine.is_stop_requested(), "a clear flag does not stop");
        flag.store(true, Ordering::Relaxed);
        assert!(engine.is_stop_requested(), "a raised flag stops");
    }

    /// The warm-cache equality pin: the engine's `tree_shape` and
    /// `shape_layer` faces agree with the board's INDEPENDENT shape
    /// derivation (`item_tree_shapes`) and layer read for every live
    /// on-board item — the peek faces are pinned against a different
    /// producer, not against themselves. The fresh derivation must run
    /// at the ENGINE's tree variant (bug-137: the manager keeps one
    /// tree per (class, variant); `trees().first()` is a different
    /// variant here, whose compensation produces boxes, not octagons).
    #[test]
    fn t11_peek_and_layer_read_match_fresh_derivation() {
        let (mut manager, mut board) = parse_fixture();
        // Resolve the engine's tree identity the way the ctor does.
        let layer_count = board.layers().layers.len();
        let probe_ctrl = AutorouteControl::new(&mut board, 94, &settings_ir(layer_count));
        let class = probe_ctrl.trace_clearance_class_index;
        let tree_index = manager.get_autoroute_tree(&mut board, class);
        let variant = manager.trees()[tree_index].variant;
        // Fresh derivations for every live item BEFORE the engine
        // exists (item_tree_shapes needs `&mut Board`).
        let keys: Vec<u64> = board
            .iter_ascending()
            .filter(|entry| entry.on_the_board)
            .map(|entry| u64::from(entry.id.get()))
            .collect();
        assert!(keys.len() > 50, "the fixture is populated");
        let mut fresh: HashMap<u64, Vec<Option<TileShape>>> = HashMap::new();
        for &key in &keys {
            let id = ItemId::new(u32::try_from(key).expect("fits"));
            fresh.insert(
                key,
                epic_board::tree_shapes::item_tree_shapes(&mut board, variant, class, id),
            );
        }
        // The engine's reads must agree, shape for shape and layer for
        // layer (the layer read goes through the engine's board — the
        // engine holds the only `&mut Board` borrow).
        let (engine, _ctrl) = build_engine(&mut manager, &mut board, 94, true);
        for &key in &keys {
            let id = ItemId::new(u32::try_from(key).expect("fits"));
            let shapes = fresh.get(&key).expect("fresh derivation recorded");
            for (index, shape) in shapes.iter().enumerate() {
                assert_eq!(
                    NeighbourEngine::tree_shape(&engine, key, index as u32).as_ref(),
                    shape.as_ref(),
                    "item {key} shape {index}: peek vs fresh derivation"
                );
                assert_eq!(
                    CompleteShapeObjects::shape_layer(&engine, key, index as u32),
                    engine
                        .board
                        .item_shape_layer_read(id, index as i32)
                        .expect("live shapes carry a layer"),
                    "item {key} shape {index}: layer read"
                );
            }
        }
    }

    /// THE end-to-end world: net 94's single pin routes to a seeded
    /// net-94 anchor trace in the y=300000 corridor. ROUTED, the route
    /// lands on the anchor corner, and no maze-result row fires (the
    /// {33,66,67} gate is closed for net 94).
    #[test]
    fn t11_autoroute_connection_routes_end_to_end() {
        let (mut manager, mut board) = parse_fixture();
        let pin_key = net_pin_key(&board, 94);
        let anchor = epic_board::trace_ops::insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(
                &Point::Int(corner(600000, 300000)),
                &Point::Int(corner(620000, 300000)),
            ),
            0,
            1500,
            &[94],
            0,
            FixedState::Unfixed,
        )
        .expect("anchor trace");
        let anchor_key = u64::from(anchor.get());
        // The engine holds `&mut board` for its lifetime; the
        // post-route board read happens after the scope drops it.
        let (routed, joined, rooms_present) = {
            let (mut engine, mut ctrl) = build_engine(&mut manager, &mut board, 94, true);
            engine.init_connection(94, None, None);
            let mut sink = CaptureSink::default();
            let result = engine.autoroute_connection(
                &[pin_key],
                &[anchor_key],
                &mut ctrl,
                &mut BTreeMap::new(),
                &mut HashMap::new(),
                &mut NoPullTight,
                &mut sink,
            );
            let joined = sink.rows.join("\n");
            let routed = result.state == AutorouteAttemptState::Routed
                && result.details.is_empty()
                && !joined.contains("compare_trace_maze_result_raw");
            let rooms_present = engine.keys.iter().any(|&key| is_room_key(key));
            (routed, joined, rooms_present)
        };
        assert!(
            routed,
            "ROUTED + empty details + closed gate. rows:\n{joined}"
        );
        // The route landed on the anchor: some net-94 trace now has a
        // corner at the anchor's end point (the target splice +
        // normalize).
        let anchor_end_reached = board.iter_descending().any(|entry| {
            entry.nets.contains(&94)
                && matches!(entry.data, ItemData::Trace { .. })
                && board.trace_polyline(entry.id).is_some_and(|polyline| {
                    polyline
                        .corners()
                        .contains(&Point::Int(corner(620000, 300000)))
                })
        });
        assert!(
            anchor_end_reached,
            "the route reached the anchor. rows:\n{joined}"
        );
        // THE NET-94 BOARD CANON — pinnable subset (see SEAM.md T11:
        // "the tightener-attributed geometry delta"). The jar capture
        // (logs/M3-T11/captures/autoroute_engine_rows_run1.jsonl,
        // phase route_routed:done) holds the same five-item structure
        // — ids 116/117/121/122/124, trace layers 0/1/0 around the
        // F.Cu keepout wall, vias at the layer-change junctions — but
        // its trace GEOMETRY is the raw staircase passed through the
        // per-segment TraceTightener
        // (RoutingBoard.insertForcedTracePolyline:862 runs
        // newTrace.pullTight on every segment; this world runs the
        // documented NoPullTight seam). The tightener attribution is
        // captured (AutorouteEngineProbe runPullTightWorld): the jar
        // tightener collapses the layer-1 staircase below EXACTLY to
        // the jar canon 121 (480738,309375)(662957,127156)
        // (662957,29188); the layer-0 staircase to a path-dependent
        // 45-degree local optimum. No normalize/combine step can
        // produce those reshapes, so the surviving delta is uniquely
        // the un-ported tightener. Pinned here: the structure (ids,
        // kinds, layers), the row-verbatim geometry (the layer-1
        // corners are literally the leg's own i=1..4 okPoints; the
        // layer-0 head/tail corners are the anchor end and the i=6
        // okPoint), the jar-exact pin leg (no staircase to tighten),
        // and the anchor absorption.
        let mut actual: Vec<String> = board
            .iter_descending()
            .filter(|entry| entry.nets.contains(&94) && !matches!(entry.data, ItemData::Pin { .. }))
            .map(|entry| match &entry.data {
                ItemData::Trace { layer, lines, .. } => {
                    let corners: Vec<String> = lines
                        .corners()
                        .iter()
                        .map(|point| match point {
                            Point::Int(c) => format!("({},{})", c.x, c.y),
                            // Rational corners never occur on inserted
                            // routes; render a marker that would fail
                            // the canon comparison if one appeared.
                            Point::Rational(_) => "rational".to_string(),
                        })
                        .collect();
                    format!(
                        "{} trace layer={layer} {}",
                        u64::from(entry.id.get()),
                        corners.join(" ")
                    )
                }
                ItemData::Via { center, .. } => format!(
                    "{} via center=({},{})",
                    u64::from(entry.id.get()),
                    center.x,
                    center.y
                ),
                _ => format!("{} UNEXPECTED-KIND", u64::from(entry.id.get())),
            })
            .collect();
        actual.sort();
        assert_eq!(
            actual.join("\n"),
            "115 trace layer=0 (620000,300000) (521250,300000) (521248,300002) \
             (516679,300002) (516664,300017) (490096,300017) (480738,309375)\n\
             117 via center=(480738,309375)\n\
             121 trace layer=1 (480738,309375) (502752,309375) (662957,149170) (662957,29188)\n\
             122 via center=(662957,29188)\n\
             124 trace layer=0 (662957,29188) (663500,28645) (663500,20000)",
            "net-94 board canon diverged from the NoPullTight engine verdict. rows:\n{joined}"
        );
        // The anchor is consumed: no net-94 item retains the anchor's
        // west stub corner (the jar canon drops it the same way — the
        // route re-roots at the anchor's far corner (620000,300000),
        // which trace 115 above still carries).
        let west_stub_present = board.iter_descending().any(|entry| {
            entry.nets.contains(&94)
                && matches!(entry.data, ItemData::Trace { .. })
                && board.trace_polyline(entry.id).is_some_and(|polyline| {
                    polyline
                        .corners()
                        .contains(&Point::Int(corner(600000, 300000)))
                })
        });
        assert!(!west_stub_present, "the anchor west stub must be absorbed");
        // The search populated the room registry (maintained database
        // survives the reset_all_doors cleanup).
        assert!(
            rooms_present,
            "completed rooms persist under maintain_database"
        );
    }

    /// The layer-active faces. With the route's ONLY layer (0) inactive
    /// the MAZE starves first (Java's maze reads ctrl.layerActive at
    /// `:396-397`/`:478`/`:493` and `DestinationDistance:57`) — FAILED
    /// "no connection was found", board unchanged. With an UNUSED layer
    /// (1) inactive the search still routes — the post-locator gate
    /// (`:221-222`) checks only the FOUND connection's start/target
    /// layers. (The gate's own FAILED literal is a defensive arm: Java's
    /// batch router pre-checks layers at
    /// `AutorouteConnectionRouter.java:189`, so an endpoint layer that
    /// is inactive always starves the search first — both engines.)
    #[test]
    fn t11_layer_active_faces() {
        // World A: the route's ONLY layer (0) inactive. The maze itself
        // starves (Java MazeSearchEngine.java:396-397/478/493 and
        // DestinationDistance.java:57-63 read ctrl.layerActive), so the
        // FAILED row is the NO-CONNECTION one and nothing is inserted —
        // the post-locator layers-disabled gate never fires.
        let (mut manager, mut board) = parse_fixture();
        let pin_key = net_pin_key(&board, 94);
        let anchor = epic_board::trace_ops::insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(
                &Point::Int(corner(600000, 300000)),
                &Point::Int(corner(620000, 300000)),
            ),
            0,
            1500,
            &[94],
            0,
            FixedState::Unfixed,
        )
        .expect("anchor trace");
        let anchor_key = u64::from(anchor.get());
        let net94_traces_before = board
            .iter_descending()
            .filter(|entry| {
                entry.nets.contains(&94) && matches!(entry.data, ItemData::Trace { .. })
            })
            .count();
        let (state, details) = {
            let (mut engine, mut ctrl) = build_engine(&mut manager, &mut board, 94, true);
            engine.init_connection(94, None, None);
            ctrl.layer_active[0] = false;
            let result = engine.autoroute_connection(
                &[pin_key],
                &[anchor_key],
                &mut ctrl,
                &mut BTreeMap::new(),
                &mut HashMap::new(),
                &mut NoPullTight,
                &mut NullSink,
            );
            (result.state, result.details)
        };
        let net94_traces_after = board
            .iter_descending()
            .filter(|entry| {
                entry.nets.contains(&94) && matches!(entry.data, ItemData::Trace { .. })
            })
            .count();
        assert_eq!(state, AutorouteAttemptState::Failed);
        assert!(
            details.ends_with("because no connection was found between their nets."),
            "the starved search answers the no-connection row: {details}"
        );
        assert_eq!(
            net94_traces_before, net94_traces_after,
            "the starved search inserts nothing"
        );

        // World B: an UNUSED layer (1) inactive — the search still
        // routes: the post-locator gate consults only the FOUND
        // connection's start/target layers (both 0 here), not the
        // whole mask.
        let (mut manager, mut board) = parse_fixture();
        let pin_key = net_pin_key(&board, 94);
        let anchor = epic_board::trace_ops::insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(
                &Point::Int(corner(600000, 300000)),
                &Point::Int(corner(620000, 300000)),
            ),
            0,
            1500,
            &[94],
            0,
            FixedState::Unfixed,
        )
        .expect("anchor trace");
        let anchor_key = u64::from(anchor.get());
        let routed = {
            let (mut engine, mut ctrl) = build_engine(&mut manager, &mut board, 94, true);
            engine.init_connection(94, None, None);
            ctrl.layer_active[1] = false;
            let result = engine.autoroute_connection(
                &[pin_key],
                &[anchor_key],
                &mut ctrl,
                &mut BTreeMap::new(),
                &mut HashMap::new(),
                &mut NoPullTight,
                &mut NullSink,
            );
            result.state == AutorouteAttemptState::Routed && result.details.is_empty()
        };
        assert!(routed, "an unused inactive layer does not block the route");
    }

    /// The degenerate start==dest world: the maze init finds no
    /// destinations, the search answers None, the cleanup runs, and
    /// the FAILED no-connection row is emitted. A second attempt on
    /// the same engine behaves identically (the cleanup is
    /// re-runnable).
    #[test]
    fn t11_failed_no_connection_and_cleanup_is_repeatable() {
        let (mut manager, mut board) = parse_fixture();
        let pin_key = net_pin_key(&board, 94);
        let (mut engine, mut ctrl) = build_engine(&mut manager, &mut board, 94, true);
        engine.init_connection(94, None, None);
        for attempt in 0..2 {
            let result = engine.autoroute_connection(
                &[pin_key],
                &[pin_key],
                &mut ctrl,
                &mut BTreeMap::new(),
                &mut HashMap::new(),
                &mut NoPullTight,
                &mut NullSink,
            );
            assert_eq!(
                result.state,
                AutorouteAttemptState::Failed,
                "attempt {attempt}"
            );
            assert!(
                result
                    .details
                    .ends_with("because no connection was found between their nets."),
                "attempt {attempt}: {}",
                result.details
            );
            assert!(
                result.details.contains("pin"),
                "attempt {attempt}: the describe faces are present: {}",
                result.details
            );
        }
    }

    /// The no-maintain cleanup: with `maintain_database = false` the
    /// same routing attempt still reaches ROUTED, and `clear()` leaves
    /// the room registry AND the room tree entries empty afterwards.
    #[test]
    fn t11_maintain_false_clears_registry_and_tree() {
        let (mut manager, mut board) = parse_fixture();
        let pin_key = net_pin_key(&board, 94);
        let anchor = epic_board::trace_ops::insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(
                &Point::Int(corner(600000, 300000)),
                &Point::Int(corner(620000, 300000)),
            ),
            0,
            1500,
            &[94],
            0,
            FixedState::Unfixed,
        )
        .expect("anchor trace");
        let anchor_key = u64::from(anchor.get());
        let (mut engine, mut ctrl) = build_engine(&mut manager, &mut board, 94, false);
        engine.init_connection(94, None, None);
        let mut sink = CaptureSink::default();
        let result = engine.autoroute_connection(
            &[pin_key],
            &[anchor_key],
            &mut ctrl,
            &mut BTreeMap::new(),
            &mut HashMap::new(),
            &mut NoPullTight,
            &mut sink,
        );
        let joined = sink.rows.join("\n");
        assert_eq!(
            result.state,
            AutorouteAttemptState::Routed,
            "rows:\n{joined}"
        );
        assert!(
            engine.rooms.is_empty() && engine.keys.is_empty(),
            "clear() emptied the registry"
        );
        assert!(
            engine.room_tree_entries.is_empty(),
            "clear() dropped the room tree entries"
        );
        // The item temporary data went too.
        assert!(engine.start_infos.is_empty(), "start infos cleared");
    }

    /// The describe face: the FAILED details carry the Java
    /// `Item.toString` faces — `pin` for a pin and `polylinetrace` for
    /// a trace — joined with `" and "`.
    #[test]
    fn t11_describe_connection_names() {
        let (mut manager, mut board) = parse_fixture();
        let pin_key = net_pin_key(&board, 94);
        let trace = epic_board::trace_ops::insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(
                &Point::Int(corner(600000, 300000)),
                &Point::Int(corner(620000, 300000)),
            ),
            0,
            1500,
            &[94],
            0,
            FixedState::Unfixed,
        )
        .expect("anchor trace");
        let describe = describe_connection(&board, &[pin_key], &[u64::from(trace.get())]);
        assert!(
            describe.starts_with("pin") && describe.contains(" and polylinetrace"),
            "describe: {describe}"
        );
        // The multi-element join faces (Java `describeConnection`
        // joins each set with ", " before the " and "): a second start
        // trace must render as `pin, polylinetrace and polylinetrace`.
        let second = epic_board::trace_ops::insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(
                &Point::Int(corner(610000, 310000)),
                &Point::Int(corner(620000, 310000)),
            ),
            0,
            1500,
            &[94],
            0,
            FixedState::Unfixed,
        )
        .expect("second trace");
        let multi = describe_connection(
            &board,
            &[pin_key, u64::from(trace.get())],
            &[u64::from(second.get())],
        );
        assert!(
            multi.contains(", polylinetrace and polylinetrace"),
            "describe: {multi}"
        );
    }

    /// The maze-result row gate (Java `:163-178`): for the gate nets
    /// {33, 66, 67} the raw row fires with the found section and the
    /// destination object's class name; for every other net it stays
    /// silent (pinned by the end-to-end test's closed-gate assert on
    /// net 94). The jar's row in this exact world
    /// (AutorouteEngineProbe route_maze_row_net33, logs/M3-T11/
    /// captures, attempt ROUTED): `net=33, section=0,
    /// destination_type=TargetItemExpansionDoor`.
    #[test]
    fn t11_maze_result_row_gate() {
        let (mut manager, mut board) = parse_fixture();
        let pin_key = net_pin_key(&board, 33);
        let anchor = epic_board::trace_ops::insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(
                &Point::Int(corner(600000, 300000)),
                &Point::Int(corner(620000, 300000)),
            ),
            0,
            1500,
            &[33],
            0,
            FixedState::Unfixed,
        )
        .expect("anchor trace");
        let anchor_key = u64::from(anchor.get());
        let (routed, joined) = {
            let (mut engine, mut ctrl) = build_engine(&mut manager, &mut board, 33, true);
            engine.init_connection(33, None, None);
            let mut sink = CaptureSink::default();
            let result = engine.autoroute_connection(
                &[pin_key],
                &[anchor_key],
                &mut ctrl,
                &mut BTreeMap::new(),
                &mut HashMap::new(),
                &mut NoPullTight,
                &mut sink,
            );
            (
                result.state == AutorouteAttemptState::Routed,
                sink.rows.join("\n"),
            )
        };
        assert!(routed, "the net-33 world routes. rows:\n{joined}");
        assert!(
            joined.contains(
                "compare_trace_maze_result_raw net=33, section=0, \
                 destination_type=TargetItemExpansionDoor"
            ),
            "the maze-result row fires verbatim for the gate net. rows:\n{joined}"
        );
    }

    /// The target-room index: after the end-to-end route (maintained
    /// database) the completed rooms with a target door to the anchor
    /// are listed in DESCENDING room-id order, and a net switch
    /// (init_connection to a foreign net) removes every net-dependent
    /// room — the anchor's target rooms among them.
    #[test]
    fn t11_get_rooms_with_target_items_descending_and_net_switch() {
        let (mut manager, mut board) = parse_fixture();
        let pin_key = net_pin_key(&board, 94);
        let anchor = epic_board::trace_ops::insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(
                &Point::Int(corner(600000, 300000)),
                &Point::Int(corner(620000, 300000)),
            ),
            0,
            1500,
            &[94],
            0,
            FixedState::Unfixed,
        )
        .expect("anchor trace");
        let anchor_key = u64::from(anchor.get());
        let (mut engine, mut ctrl) = build_engine(&mut manager, &mut board, 94, true);
        engine.init_connection(94, None, None);
        let result = engine.autoroute_connection(
            &[pin_key],
            &[anchor_key],
            &mut ctrl,
            &mut BTreeMap::new(),
            &mut HashMap::new(),
            &mut NoPullTight,
            &mut NullSink,
        );
        assert_eq!(result.state, AutorouteAttemptState::Routed);
        let rooms = engine.get_rooms_with_target_items(&[anchor_key]);
        assert!(!rooms.is_empty(), "the anchor has target rooms");
        let ids: Vec<i32> = rooms
            .iter()
            .map(|&key| engine.resolve(key).expect("live").id())
            .collect();
        let mut sorted = ids.clone();
        sorted.sort_by(|a, b| b.cmp(a));
        assert_eq!(ids, sorted, "descending room-id order (Java TreeSet)");
        for &key in &rooms {
            assert!(
                engine
                    .resolve(key)
                    .is_some_and(ExpansionRoom::is_complete_free_space),
                "only complete free-space rooms are listed"
            );
        }
        // The net switch: every net-dependent room (the anchor's
        // target rooms among them) is removed.
        engine.init_connection(2, None, None);
        let after = engine.get_rooms_with_target_items(&[anchor_key]);
        assert!(
            after.is_empty(),
            "the net switch dropped the net-94 target rooms"
        );
        // Java's initConnection walk covers only the NEW net's items
        // (`initConnection` filters `board.getItems()` by the new net)
        // — the net-94 pin's start info legitimately SURVIVES the
        // switch to net 2 (the persistence face).
        assert!(
            engine.start_infos.contains(&pin_key),
            "the pin's start info persists across the net switch"
        );
    }

    /// The split/combine bisect: the layer-0 leg's FIRST per-segment
    /// insert (P1 (610000,300000)→(521250,300000), collinear over the
    /// seeded anchor), replayed standalone — the mirror of the jar's
    /// net-49 debug world (AutorouteEngineProbe runSplitCombineWorld).
    /// Java's verdict (debugNet49 rows + splitcombine:done inventory):
    /// the insert piece is split at the anchor's west endpoint, the
    /// duplicated middle span is removed as a cycle, and everything
    /// combines into ONE trace spanning (620000,300000)→
    /// (521250,300000) — anchor absorbed, west stub gone.
    #[test]
    fn t11_p1_over_anchor_split_combine() {
        let (mut manager, mut board) = parse_fixture();
        epic_board::trace_ops::insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(
                &Point::Int(corner(600000, 300000)),
                &Point::Int(corner(620000, 300000)),
            ),
            0,
            1500,
            &[49],
            0,
            FixedState::Unfixed,
        )
        .expect("anchor trace");
        let ok = epic_board::routing_board_insert::insert_forced_trace_polyline(
            &mut manager,
            &mut board,
            &mut NoPullTight,
            &Polyline::from_two_corners(
                &Point::Int(corner(610000, 300000)),
                &Point::Int(corner(521250, 300000)),
            ),
            1500,
            0,
            Some(&[49]),
            0,
            20,
            5,
            5,
            i32::MAX,
            500,
            true,
            None::<&epic_board::time_limit::TimeLimit>,
        );
        let ok = ok.expect("P1 insert advanced");
        assert_eq!(
            ok,
            Point::Int(corner(521250, 300000)),
            "the jar's okPoint for P1"
        );
        let net49: Vec<String> = board
            .iter_descending()
            .filter(|entry| {
                entry.nets.contains(&49) && matches!(entry.data, ItemData::Trace { .. })
            })
            .map(|entry| match &entry.data {
                ItemData::Trace { lines, .. } => {
                    let corners: Vec<String> = lines
                        .corners()
                        .iter()
                        .map(|point| match point {
                            Point::Int(c) => format!("({},{})", c.x, c.y),
                            Point::Rational(_) => "rational".to_string(),
                        })
                        .collect();
                    format!("{} trace {}", u64::from(entry.id.get()), corners.join(" "))
                }
                _ => format!("{} UNEXPECTED-KIND", u64::from(entry.id.get())),
            })
            .collect();
        assert_eq!(
            net49.join("\n"),
            "109 trace (620000,300000) (521250,300000)",
            "the jar's splitcombine:done canon: one trace, anchor absorbed"
        );
    }

    /// THE STRAIGHT-RUN CANON (the dispatch's mandated
    /// tightener-stable world): net 94's pin (663500,20000) routes to
    /// a seeded net-94 VERTICAL anchor immediately north of the pin
    /// ((663500,26000)-(663500,28000), hw 1500, class 0). The optimal
    /// path is forced-straight (no keepout interaction, no
    /// alternative), so the jar's per-segment TraceTightener is a
    /// no-op and the FULL board canon (ids + geometry) is
    /// Java-identical — the corridor world above is NOT
    /// tightener-stable (its staircase is pull-tight rework; see the
    /// SEAM tightener-attribution bank and the pulltight probe
    /// captures). Jar verdict (AutorouteEngineProbe route_straight94,
    /// logs/M3-T11/captures): ROUTED; one ADVANCE segment
    /// (663500,27000)→(663500,20000) over the anchor (pickedSize=1);
    /// normalize absorbs the piece + anchor into ONE straight trace
    /// (id 109); before/after pull-tight rows equal; the anchor's
    /// north end survives as the trace's first corner.
    #[test]
    fn t11_straight_corridor_canon() {
        let (mut manager, mut board) = parse_fixture();
        let pin_key = net_pin_key(&board, 94);
        let anchor = epic_board::trace_ops::insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(
                &Point::Int(corner(663500, 26000)),
                &Point::Int(corner(663500, 28000)),
            ),
            0,
            1500,
            &[94],
            0,
            FixedState::Unfixed,
        )
        .expect("anchor trace");
        assert_eq!(anchor.get(), 105, "id replay");
        let anchor_key = u64::from(anchor.get());
        let (joined, net94) = {
            let (mut engine, mut ctrl) = build_engine(&mut manager, &mut board, 94, true);
            engine.init_connection(94, None, None);
            let mut sink = CaptureSink::default();
            let result = engine.autoroute_connection(
                &[pin_key],
                &[anchor_key],
                &mut ctrl,
                &mut BTreeMap::new(),
                &mut HashMap::new(),
                &mut NoPullTight,
                &mut sink,
            );
            assert_eq!(
                result.state,
                AutorouteAttemptState::Routed,
                "rows:\n{}",
                sink.rows.join("\n")
            );
            let joined = sink.rows.join("\n");
            let net94: Vec<String> = board
                .iter_descending()
                .filter(|entry| {
                    entry.nets.contains(&94) && !matches!(entry.data, ItemData::Pin { .. })
                })
                .map(|entry| match &entry.data {
                    ItemData::Trace { layer, lines, .. } => {
                        let corners: Vec<String> = lines
                            .corners()
                            .iter()
                            .map(|point| match point {
                                Point::Int(c) => format!("({},{})", c.x, c.y),
                                Point::Rational(_) => "rational".to_string(),
                            })
                            .collect();
                        format!(
                            "{} trace layer={layer} {}",
                            u64::from(entry.id.get()),
                            corners.join(" ")
                        )
                    }
                    ItemData::Via { center, .. } => format!(
                        "{} via center=({},{})",
                        u64::from(entry.id.get()),
                        center.x,
                        center.y
                    ),
                    _ => format!("{} UNEXPECTED-KIND", u64::from(entry.id.get())),
                })
                .collect();
            (joined, net94)
        };
        // THE JAR CANON, byte-exact (route_straight94:done).
        assert_eq!(
            net94.join("\n"),
            "109 trace layer=0 (663500,28000) (663500,20000)",
            "the straight-run board canon diverged from the jar. rows:\n{joined}"
        );
        // The event-row surface, jar-verbatim (the connection item and
        // the single-segment walk).
        assert!(
            joined.contains(
                "compare_trace_connection_item_raw net=94, item_layer=0, cornerCount=2, \
                 start=(663500,27000), end=(663500,20000)"
            ),
            "connection item row. rows:\n{joined}"
        );
        assert!(
            joined.contains(
                "compare_trace_insert_segment_raw net=94, layer=0, i=1, fromCornerNo=1, \
                 decision=ADVANCE, neckdown=false, micro_neckdown=false, \
                 okPoint=(663500,20000), first=(663500,27000), last=(663500,20000)"
            ),
            "segment row. rows:\n{joined}"
        );
        assert!(
            joined.contains("compare_trace_insert_segment_ids net=94, i=1, maxItemIdBefore=105, maxItemIdAfter=110, delta=5"),
            "id-burn row. rows:\n{joined}"
        );
        assert!(
            joined.contains("compare_trace_stub_cleanup")
                && joined.contains("removed_stubs=0, trace_enabled=true"),
            "stub cleanup row. rows:\n{joined}"
        );
    }

    /// The fixpoint-stability face of the insert tail: the board the
    /// engine returns from `autoroute_connection` must be STABLE under
    /// one more `normalize_traces_of_net` pass (Java's insert_trace
    /// exits `board.normalizeTraces(net)` only when a full pass makes
    /// no change, so its exit state is a fixpoint by construction).
    /// A second Rust pass that changes anything means the tail's walk
    /// exited prematurely.
    #[test]
    fn t11_board_is_normalize_fixpoint_after_route() {
        let (mut manager, mut board) = parse_fixture();
        let pin_key = net_pin_key(&board, 94);
        let anchor = epic_board::trace_ops::insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(
                &Point::Int(corner(600000, 300000)),
                &Point::Int(corner(620000, 300000)),
            ),
            0,
            1500,
            &[94],
            0,
            FixedState::Unfixed,
        )
        .expect("anchor trace");
        let anchor_key = u64::from(anchor.get());
        {
            let (mut engine, mut ctrl) = build_engine(&mut manager, &mut board, 94, true);
            engine.init_connection(94, None, None);
            let mut sink = CaptureSink::default();
            let result = engine.autoroute_connection(
                &[pin_key],
                &[anchor_key],
                &mut ctrl,
                &mut BTreeMap::new(),
                &mut HashMap::new(),
                &mut NoPullTight,
                &mut sink,
            );
            assert_eq!(result.state, AutorouteAttemptState::Routed);
        }
        let before: Vec<String> = board
            .iter_descending()
            .filter(|entry| entry.nets.contains(&94))
            .map(|entry| format!("{} {:?}", u64::from(entry.id.get()), entry.data))
            .collect();
        let changed =
            epic_board::normalize_all::normalize_traces_of_net(&mut manager, &mut board, 94);
        let after: Vec<String> = board
            .iter_descending()
            .filter(|entry| entry.nets.contains(&94))
            .map(|entry| format!("{} {:?}", u64::from(entry.id.get()), entry.data))
            .collect();
        assert!(
            !changed && before == after,
            "the post-route board is NOT a normalize fixpoint (changed={changed})\nbefore:\n{}\nafter:\n{}",
            before.join("\n"),
            after.join("\n")
        );
    }

    /// The finish face: after a route with a maintained database,
    /// `finish_autoroute` clears the room leaves — the class tree is
    /// back to its items-only count, which now INCLUDES the route's
    /// own traces (they are items; the route legitimately grew the
    /// tree beyond the pre-route baseline).
    #[test]
    fn t11_finish_autoroute_restores_tree_leaf_count() {
        let (mut manager, mut board) = parse_fixture();
        let pin_key = net_pin_key(&board, 94);
        let anchor = epic_board::trace_ops::insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(
                &Point::Int(corner(600000, 300000)),
                &Point::Int(corner(620000, 300000)),
            ),
            0,
            1500,
            &[94],
            0,
            FixedState::Unfixed,
        )
        .expect("anchor trace");
        let anchor_key = u64::from(anchor.get());
        let layer_count = board.layers().layers.len();
        let mut ctrl = AutorouteControl::new(&mut board, 94, &settings_ir(layer_count));
        let class = ctrl.trace_clearance_class_index;
        let baseline = {
            let engine = AutorouteEngine::new(&mut manager, &mut board, class, true);
            engine.tree().leaf_count()
        };
        let mut engine =
            init_autoroute(&mut manager, &mut board, None, 94, class, true, None, None);
        assert_eq!(engine.net_number(), 94, "init_autoroute primes the net");
        let tree_index = engine.autoroute_tree;
        let result = engine.autoroute_connection(
            &[pin_key],
            &[anchor_key],
            &mut ctrl,
            &mut BTreeMap::new(),
            &mut HashMap::new(),
            &mut NoPullTight,
            &mut NullSink,
        );
        assert_eq!(result.state, AutorouteAttemptState::Routed);
        let leaves_with_rooms = engine.tree().leaf_count();
        assert!(
            leaves_with_rooms > baseline,
            "the route added room leaves (baseline {baseline})"
        );
        finish_autoroute(Some(engine));
        // Address the tree BY INDEX: the manager keeps several trees
        // for the same class (one per variant — bug-137); `find` by
        // class alone can answer a different tree.
        let leaves_after = manager.trees()[tree_index].leaf_count();
        assert!(
            leaves_after < leaves_with_rooms,
            "finish removed the room leaves ({leaves_with_rooms} -> {leaves_after})"
        );
        assert!(
            leaves_after > baseline,
            "the route's own traces remain in the tree ({leaves_after} vs baseline {baseline})"
        );
        // The exact items-only restoration: a fresh engine (same class
        // tree selection) counts exactly the post-finish leaves — this
        // also pins that the ctor and init_autoroute resolve the SAME
        // tree.
        let post_items = {
            let probe = AutorouteEngine::new(&mut manager, &mut board, class, true);
            probe.tree().leaf_count()
        };
        assert_eq!(
            leaves_after, post_items,
            "finish restored the items-only tree"
        );
    }
}
