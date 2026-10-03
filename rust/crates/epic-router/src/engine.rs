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
//!   The SAME fold absorbs Java's THIRD arm (`AutorouteEngine.java
//!   :215-219`): when the locator returns null after a found search,
//!   Java emits a plain `"Failed to route connection between X."`
//!   (no because-clause). That arm is dead-defensive in Java itself —
//!   `FoundConnectionLocator.getInstance` (`FoundConnectionLocator.java
//!   :192-194`) answers null ONLY for a null maze search result, which
//!   the earlier `:206-213` arm already handled — and the ported
//!   locator mirrors it: `locator::get_instance` returns None only
//!   through the `maze_search_result?` guard (`locator.rs:188`); on a
//!   Some search every path returns Some (the bail arms carry the
//!   null-ish FIELDS, not a None). The single Rust arm therefore
//!   always serves the reachable search-null face and keeps the
//!   no-connection literal; the plain-literal arm is unreachable in
//!   the port outright (no locator exceptions exist to swallow), and
//!   in Java reachable ONLY through the swallowed `getInstance`
//!   exception (`AutorouteEngine.java:180-191`: `autorouteResult`
//!   null-initialized, any exception logged and absorbed, leaving the
//!   null that `:215-219` fires on) — a path the port does not
//!   reproduce (quality-review F-Q2; precision fix R-1).
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
//!   earlier); a key whose item is OFF the board degrades to
//!   empty/None — Java's `board == null` faces in `treeShapeCount`
//!   and `getTreeShape` (bug-229 lifecycle: ripped items stay in
//!   connection sets and maze doors); the `touchingSides[1]` index
//!   and the layer index reads keep Java's AIOOBE face as panics.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use epic_board::board::Board;
use epic_board::forced_pad_router::CheckDrillResult as BoardCheckDrillResult;
use epic_board::id::ItemId;
use epic_board::items::{FixedState, ItemData};
use epic_board::routing_board_insert::{PullTightSeam, remove_trace_tails};
use epic_board::routing_board_search::check_trace_segment as check_trace_segment_facade;
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
use crate::expansion::neighbours::{
    ROOM_KEY_BASE, board_item_is_trace_obstacle, ignore_nets_key_is_obstacle,
};
use crate::expansion::{
    ExpansionDoor, ExpansionRoom, NeighbourEngine, RoomKind, TargetItemExpansionDoor, TreeEntry,
};
use crate::maze::destination_distance::DestinationDistance as ProdDistance;
use crate::maze::list_element::ExpandableObject;
use crate::maze::search_engine::MazeSearchEngine;
use crate::path::inserter::get_instance as insert_found_connection;
use crate::path::inserter::{InserterEventSink, router_angle_restriction};
use crate::path::locator;

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
// the ripped-item row seed (T12)
// ---------------------------------------------------------------------------

/// The per-ripped-item row faces (`AutoroutePassRunner.logRippedItems`,
/// `:360-394`): Java reads `getClass().getSimpleName()`, `netCount()`
/// and `getNetNo(ix)` off the ripped items AFTER the engine has removed
/// them from the board — legal there because the ripped list is a
/// `TreeSet<Item>` of live objects that survives the removal. The
/// port's list carries keys, not object identity, so
/// [`AutorouteEngine::autoroute_connection`] snapshots the faces into
/// this seed at the last moment they are readable (right after the
/// locator harvest, BEFORE the ripup removal walk and the FAILED early
/// returns — Java's map keeps its entries on those paths too).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RippedItemSeed {
    /// The item key (Java `Item.getId()` — the row's `ripped_id`; the
    /// map key is the same id as an `i32`).
    pub key: u64,
    /// Java `item.getClass().getSimpleName()`.
    pub simple_name: &'static str,
    /// Java `item.netCount()` / `getNetNo(ix)` at harvest time.
    pub nets: Vec<i32>,
}

// ---------------------------------------------------------------------------
// the per-connection budget (T12)
// ---------------------------------------------------------------------------

/// The per-connection stop budget (Java `datastructures.TimeLimit` as
/// consumed by `AutorouteEngine.isStopRequested`, `:287-295`).
///
/// Java builds a wall-clock `TimeLimit` PER CONNECTION ROUTE with the
/// exact arithmetic `maxMilliseconds = 100000 * 2^(ripupPassNo - 1)`
/// capped at `Integer.MAX_VALUE` (`AutorouteConnectionRouter.java:72-74`),
/// and `isStopRequested` answers `elapsed > limit` BEFORE reading the
/// stoppable-thread flag. The wall clock is the only run-to-run
/// variance source in the whole engine, so the deterministic corpus
/// profile replaces its CONSUMPTION with a call-tick counter
/// ([`RouteBudget::Deterministic`]) carrying the SAME limit value,
/// spent at the SAME call sites with the SAME strict `>`; the
/// wall-clock face stays available as [`RouteBudget::Wall`]
/// (SEAM: the units deviation — Java spends milliseconds, the
/// deterministic variant spends `is_stop_requested` calls).
#[derive(Debug)]
pub enum RouteBudget {
    /// The Java wall-clock face (`new TimeLimit(limitMillis)`).
    Wall(TimeLimit),
    /// Deterministic consumption: `limit` ticks, one per
    /// `is_stop_requested` consultation; stops when `spent > limit`
    /// (the strict `>` mirrors `TimeLimit.limitExceeded`).
    Deterministic { limit: u64, spent: Cell<u64> },
}

impl RouteBudget {
    /// M11-T4 fix round (2026-10-03): the deterministic profile's
    /// per-search tick CEILING — `min(ladder, CEILING)`.
    ///
    /// Java's `TimeLimit` ladder saturates at `Integer.MAX_VALUE` ms
    /// (~24 days), so in Java the ladder only ever bounds a search in
    /// wall time it can actually exceed — an early pass's 100 s. The
    /// deterministic profile spends the SAME ladder as call ticks,
    /// where `Integer.MAX_VALUE` is UNREACHABLE work, not unreachable
    /// time: measured on gv-iu (EPIC_T4DIAG tick rows, 2026-10-03),
    /// the largest HEALTHY search on the face is 44,181 ticks while
    /// the #931-cluster-F pass-35 explosion (net 70, item 230, a
    /// re-attempt after two same-pass rips) ran >16.7M frontier pops
    /// — >380x healthy — without approaching the ladder, at ~84% of
    /// one core. The ceiling bounds every deterministic search
    /// regardless of pass: 2^22 = 4,194,304 ticks = 95x the measured
    /// healthy max (margin against a larger healthy max on some
    /// unmeasured board of the 1332-fixture corpus), and at the
    /// explosion's measured ~100k pops/s it trips in ~40 s. The
    /// [`RouteBudget::Wall`] face consumes the SAME capped value as
    /// its millisecond budget (pass >= 7: ~70 min instead of Java's
    /// ~107-min rung — beyond any healthy single search; a real
    /// clock needs no further ceiling). A tripped search stops
    /// exactly as Java's expired
    /// `TimeLimit` — the attempt fails, the item re-queues; see the
    /// fix-round report for the pass-wall measurement at the fixed
    /// world.
    pub(crate) const SEARCH_TICK_CEILING: u64 = 1 << 22;

    /// Java `AutorouteConnectionRouter.route` (`:72-74`):
    /// `double maxMilliseconds = 100000 * Math.pow(2, ripupPassNo - 1);
    /// maxMilliseconds = Math.min(maxMilliseconds, Integer.MAX_VALUE);
    /// new TimeLimit((int) maxMilliseconds)`. The limit VALUE is
    /// Java-exact; only the consumption units differ (see the type
    /// docs). The deterministic variant additionally carries the
    /// [`SEARCH_TICK_CEILING`] backstop (`min` of the ladder).
    #[must_use]
    pub fn deterministic_for_pass(ripup_pass_no: i32) -> RouteBudget {
        let mut max_milliseconds = 100_000.0f64 * (2.0f64).powi(ripup_pass_no - 1);
        max_milliseconds = max_milliseconds.min(f64::from(i32::MAX));
        // Java's (int) cast of an in-range double truncates; every
        // value on this ladder is integral, so the cast is exact.
        let limit = i64::from(max_milliseconds as i32);
        let limit = u64::try_from(limit)
            .unwrap_or(0)
            .min(Self::SEARCH_TICK_CEILING);
        RouteBudget::Deterministic {
            limit,
            spent: Cell::new(0),
        }
    }

    /// Java `TimeLimit.limitExceeded` — strict `>`. Each deterministic
    /// consultation burns one tick.
    fn limit_exceeded(&self) -> bool {
        match self {
            RouteBudget::Wall(time_limit) => time_limit.limit_exceeded(),
            RouteBudget::Deterministic { limit, spent } => {
                let ticks = spent.get().saturating_add(1);
                spent.set(ticks);
                ticks > *limit
            }
        }
    }

    /// The construction value (Java `(int) maxMilliseconds`).
    #[must_use]
    pub fn limit_value(&self) -> u64 {
        match self {
            RouteBudget::Wall(time_limit) => u64::try_from(time_limit.limit_millis()).unwrap_or(0),
            RouteBudget::Deterministic { limit, .. } => *limit,
        }
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
    key_index: &'e HashMap<u64, usize>,
    graveyard: &'e BTreeMap<u64, ExpansionRoom>,
    /// The autoroute tree's object id — the warm-cache peek key.
    tree_object_id: u64,
}

/// The live-then-graveyard room read (Java reads removed room OBJECTS
/// through the live references held by in-flight maze elements; the
/// graveyard is that object identity). Free-standing so the view and
/// the engine share it.
///
/// The live-registry arm is a HASH lookup (slice C): the key→index
/// side map answers with the same index the linear
/// `keys.iter().position()` scan used to find — registry keys are
/// unique by construction (monotone `alloc_key`) — so the resolved
/// room, or the miss, is identical for every key in every registry
/// state. The graveyard arm is untouched.
fn registry_resolve<'r>(
    rooms: &'r [ExpansionRoom],
    key_index: &'r HashMap<u64, usize>,
    graveyard: &'r BTreeMap<u64, ExpansionRoom>,
    key: u64,
) -> Option<&'r ExpansionRoom> {
    if let Some(room) = graveyard.get(&key) {
        return Some(room);
    }
    key_index.get(&key).map(|&index| &rooms[index])
}

fn is_room_key(key: u64) -> bool {
    key >= ROOM_KEY_BASE
}

impl CompleteShapeObjects for EngineShapeView<'_> {
    fn is_trace_obstacle(&self, object_key: u64, net_number: i32) -> bool {
        if is_room_key(object_key) {
            let Some(room) =
                registry_resolve(self.rooms, self.key_index, self.graveyard, object_key)
            else {
                return false;
            };
            if room.is_complete_free_space() {
                // Java CompleteFreeSpaceExpansionRoom participates in
                // completeShape's restraint/ignore arms.
                return true;
            }
            if let RoomKind::Obstacle { item_key, .. } = room.kind {
                // Java queries the room's CONTAINED ITEM (obstacle rooms
                // are not tree objects — `SortedRoomNeighbours.java:217-223`
                // casts the tree entry to the item), so the VIRTUAL
                // `Item.isTraceObstacle` face applies (CA flag, keepout
                // kinds) via [`Board::item_is_trace_obstacle`].
                return board_item_is_trace_obstacle(self.board, item_key, net_number);
            }
            return false;
        }
        board_item_is_trace_obstacle(self.board, object_key, net_number)
    }

    fn shape_layer(&self, object_key: u64, shape_index: u32) -> i32 {
        if is_room_key(object_key) {
            return registry_resolve(self.rooms, self.key_index, self.graveyard, object_key)
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
            return registry_resolve(self.rooms, self.key_index, self.graveyard, object_key)
                .map(|room| room.shape().clone());
        }
        let id = ItemId::new(u32::try_from(object_key).expect("item key fits u32"));
        // Java `getTreeShape` on a removed item: `this.board == null`
        // → null. The key survives in doors/connection sets after a
        // ripup (bug-229 lifecycle); no shapes remain to read.
        self.board.get(id)?;
        self.board
            .tree_shape_precalc_peek(id, self.tree_object_id)
            .expect("warm-cache peek miss")
            .get(shape_index as usize)
            .cloned()
            .flatten()
    }

    fn is_complete_free_space(&self, object_key: u64) -> bool {
        is_room_key(object_key)
            && registry_resolve(self.rooms, self.key_index, self.graveyard, object_key)
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
    /// Java `timeLimit` — the [`RouteBudget`] replaces the raw
    /// `TimeLimit` (wall-clock + deterministic consumption faces).
    budget: Option<RouteBudget>,
    /// Java `drillPageArray`.
    drill_page_array: DrillPageArray,
    /// The room registry (Java's room objects; creation-ordered like
    /// the Java `completeExpansionRooms`/`incompleteExpansionRooms`
    /// ArrayLists).
    rooms: Vec<ExpansionRoom>,
    keys: Vec<u64>,
    /// The live-registry key→index side map (slice C): answers
    /// `registry_resolve`/`room_mut` with the same index the linear
    /// `keys.iter().position()` scan found — keys are unique
    /// (monotone `alloc_key`) — at O(1) instead of O(n). Maintained at
    /// every registry push/removal/clear; rebuilt on removal (the
    /// order-preserving `Vec::remove` shifts every higher index down
    /// by one; removals are rare next to resolves — bm06 measured
    /// 77.3M resolves against 145k graveyard insertions).
    key_index: HashMap<u64, usize>,
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
            budget: None,
            drill_page_array: DrillPageArray::default(),
            rooms: Vec::new(),
            keys: Vec::new(),
            key_index: HashMap::new(),
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
            key_index: &self.key_index,
            graveyard: &self.graveyard,
            tree_object_id: self.tree_object_id(),
        }
    }

    /// The warm-cache item shapes (Java `item.getTreeShape` reads the
    /// same stored shapes; a live on-board item always has them — a
    /// miss on a LIVE item is an engine-contract bug, hence the panic).
    /// A key whose item is no longer on the board degrades to EMPTY:
    /// Java `Item.treeShapeCount` opens with `if (this.board == null)
    /// return 0;` and `getTreeShape` answers null — connection sets and
    /// maze target doors can still carry the key after a same-net
    /// ripup (the bug-229 lifecycle), and the arena has dropped the
    /// data, so the warm cache cannot answer (E2E witness:
    /// Issue420-contribution-board, fanout pass 1, maze init counting
    /// shapes of a ripped connection item).
    fn item_tree_shapes(&self, item_key: u64) -> &[Option<TileShape>] {
        let id = ItemId::new(u32::try_from(item_key).expect("item key fits u32"));
        if self.board.get(id).is_none() {
            return &[];
        }
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

    /// Java `isStopRequested` (`:287-295`): the budget first, then the
    /// stoppable-thread flag. The T12 pass driver hands in a
    /// [`RouteBudget`]: the wall-clock face mirrors Java's `TimeLimit`
    /// directly; the deterministic face spends Java's exact
    /// construction value as call ticks (SEAM: units deviation).
    #[must_use]
    pub fn is_stop_requested(&self) -> bool {
        if self
            .budget
            .as_ref()
            .is_some_and(RouteBudget::limit_exceeded)
        {
            return true;
        }
        self.stoppable_flag
            .as_deref()
            .is_some_and(|flag| flag.load(Ordering::Relaxed))
    }

    /// Reclaim the budget for the NECKED RETRY: Java hands the SAME
    /// `TimeLimit` instance to the second
    /// `board.initAutoroute` call (`AutorouteConnectionRouter.java`,
    /// `retryConnectionNecked`'s last argument), so the deadline (and
    /// the deterministic tick count) carries across both attempts.
    /// The engine owns its budget, so the retry takes it out of the
    /// spent engine and re-lends it to the fresh one.
    pub fn take_budget(&mut self) -> Option<RouteBudget> {
        self.budget.take()
    }

    /// Java `initConnection` (`:96-124`): under a maintained database,
    /// a NET SWITCH first removes the net-dependent complete rooms
    /// (creation order — Java's `completeExpansionRooms` ArrayList),
    /// then runs [`Self::additional_update_after_change`] for every
    /// on-board item of the new net (Java walks `board.getItems()` in
    /// insertion order = ascending id; off-board items carry no tree
    /// shapes, so the Java loop no-ops for them and the port skips
    /// them). Always assigns net/stop/budget state.
    pub fn init_connection(
        &mut self,
        net_number: i32,
        stoppable_flag: Option<Arc<AtomicBool>>,
        budget: Option<RouteBudget>,
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
        self.budget = budget;
    }

    /// Java `RoutingBoard.initAutoroute`'s reuse condition
    /// (`:888-897`): the HELD engine serves the next connection when
    /// the database is maintained AND this engine's compensated tree
    /// already carries the wanted clearance class. True → the caller
    /// only re-runs [`Self::init_connection`] (Java's reuse arm: the
    /// room registry and the Java room ids carry over; a same-net
    /// init purges nothing). The slot-parameter form of this branch is
    /// unrepresentable in the port's ownership model — see
    /// [`init_autoroute`].
    #[must_use]
    pub fn is_reusable_for(&self, trace_clearance_class_index: i32, retain_database: bool) -> bool {
        retain_database
            && self.manager.trees()[self.autoroute_tree].compensated_clearance_class
                == trace_clearance_class_index
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
        if let Some(&index) = self.key_index.get(&room_key) {
            let room = self.rooms.remove(index);
            self.keys.remove(index);
            self.graveyard.insert(room_key, room);
            // The order-preserving Vec::remove shifted every higher
            // index down by one — rebuild the side map (removals are
            // rare next to resolves; bm06 measured 77.3M resolves
            // against 145k graveyard insertions). clear() KEEPS the
            // table's capacity: the fresh-`collect()` form allocated a
            // full new table per removal and DOUBLED bm06's byte churn
            // (measured, 4.61→8.95 GB) before being replaced here.
            self.key_index.clear();
            for (i, &k) in self.keys.iter().enumerate() {
                self.key_index.insert(k, i);
            }
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
        self.key_index.clear();
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
            let Some(other_key) = self.room_key_of_door(other_id, &door) else {
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
            let Some(other_key) = self.room_key_of_door(other_id, &door) else {
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
        registry_resolve(&self.rooms, &self.key_index, &self.graveyard, key)
    }

    fn room_mut(&mut self, key: u64) -> &mut ExpansionRoom {
        let index = *self
            .key_index
            .get(&key)
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

    /// The door-driven exact endpoint resolution: the live room of
    /// `id` connected by `door` — holds THE door (the endpoint's copy
    /// shares the instance tag, Java's reference identity). Java
    /// resolves the other endpoint by OBJECT REFERENCE
    /// (`door.otherRoom(room)`); the bare id scan of
    /// [`Self::room_key_of_id`] is unfaithful for that when two live
    /// incomplete rooms share `getId()`: the formula
    /// `31 * shape.getId() + layer` (`IncompleteFreeSpaceExpansionRoom
    /// .java:38-41`) runs on content-derived shape ids, so
    /// equal-shape same-layer rooms collide (the room.rs collision
    /// note). No graveyard fallback: Java only reaches a removed room
    /// through a door that `removeAllDoors` has already stripped from
    /// the walking room, so a miss mirrors an unreachable reference.
    fn room_key_of_door(&self, id: i32, door: &ExpansionDoor) -> Option<u64> {
        self.rooms
            .iter()
            .zip(&self.keys)
            .find(|(room, _)| room.id() == id && room.doors().iter().any(|d| d == door))
            .map(|(_, key)| *key)
    }

    fn item_nets(&self, key: u64) -> &[i32] {
        let id = ItemId::new(u32::try_from(key).expect("item key fits u32"));
        self.board
            .get(id)
            .map(|entry| entry.nets.as_slice())
            .unwrap_or(&[])
    }

    /// The [`RippedItemSeed`] snapshot of a just-harvested ripped item
    /// (Java reads the same faces off the live object after removal —
    /// see the type docs).
    fn ripped_item_seed(&self, item_key: u64) -> RippedItemSeed {
        let id = ItemId::new(u32::try_from(item_key).expect("item key fits u32"));
        let entry = self
            .board
            .get(id)
            .expect("a harvested ripped item is still on the board");
        RippedItemSeed {
            key: item_key,
            simple_name: entry.data.java_simple_name(),
            nets: entry.nets.clone(),
        }
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
        ripped_item_list: &mut BTreeMap<i32, RippedItemSeed>,
        ripup_costs: &mut HashMap<u64, i32>,
        seam: &mut S,
        sink: &mut E,
    ) -> AutorouteAttemptResult {
        // Pre-maze reads (the maze mutably borrows the engine).
        let net = ctrl.net_number;
        let describe = describe_connection(self.board, ripped_item_list, start_items, dest_items);
        let angle_restriction =
            router_angle_restriction(self.board.rules().trace_angle_restriction);

        // The page array is swapped out for the search (the maze holds
        // it alongside the engine borrow) and restored before cleanup.
        let mut pages = std::mem::take(&mut self.drill_page_array);

        // The locator harvests into a KEY-only scratch map: its generic
        // access layer reads through `NeighbourEngine` and cannot
        // snapshot board faces. Right after it returns — while every
        // listed item is still on the board, and BEFORE the cleanup
        // gates and the ripup removal below — the keys are converted to
        // the row seeds the caller observes (Java reads the same faces
        // off the live `TreeSet<Item>` objects, which keep the entries
        // readable on the FAILED early-return paths too).
        let mut harvested: BTreeMap<i32, u64> = BTreeMap::new();

        let autoroute_result = {
            let mut destination_distance = ProdDistance::from_ctrl(ctrl);
            let mut checker = BoardLayerChecker;
            // Java installs the fanout frontier filter on the expansion
            // TreeSet at MAZE-ENGINE CONSTRUCTION
            // (`MazeSearchEngine.java:87-124`), gated on
            // `ctrl.isFanout && ctrl.fanoutStartPinCenter != null` —
            // the port snapshots the search-invariant gate constants
            // here (pin anchor/layer, the escape-length ternaries, the
            // board resolution) and hands them to the front below. A
            // detail-route search installs nothing (the gate is inert).
            let fanout_gate = match (ctrl.is_fanout, ctrl.fanout_start_pin_center.as_ref()) {
                (true, Some(pin_center)) => {
                    let comm = self.board.communication();
                    let resolution = epic_dsn::state::Unit::scale(
                        f64::from(comm.resolution),
                        epic_dsn::state::Unit::Um,
                        comm.unit,
                    );
                    Some(crate::maze::list_element::FanoutFrontGate::new(
                        &ctrl.settings.fanout,
                        pin_center,
                        ctrl.fanout_start_pin_layer,
                        resolution,
                    ))
                }
                _ => None,
            };
            let mut maze = MazeSearchEngine::new(
                self,
                ctrl,
                &mut destination_distance,
                &mut checker,
                &mut pages,
            );
            if let Some(gate) = fanout_gate {
                maze.front.set_fanout_gate(gate);
            }
            // T16: attach the one-arg trace backend (Java
            // `FRLogger.trace(String)`). The maze engine's RAW_SECTION
            // rows ride the connection's own event sink — the capture
            // sink records them, `NullSink` drops them (Java's silent
            // backend). The sink is borrowed for the search's whole
            // life, so the post-search row below forwards through the
            // maze's own emit face (`emit_raw_row`) — identical
            // backend, identical stream position.
            maze.trace_sink(&mut *sink);
            let search_result = maze.find_connection_between(start_items, dest_items);

            // The raw maze-result row (Java `:163-178`) — log-only.
            if let Some(result) = &search_result
                && (net == 33 || net == 66 || net == 67)
            {
                let destination_type = expandable_object_type_name(&result.destination_door);
                maze.emit_raw_row(&format!(
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
                &mut harvested,
                ripup_costs,
            )
        };
        self.drill_page_array = pages;
        for (item_id, item_key) in harvested {
            ripped_item_list.insert(item_id, self.ripped_item_seed(item_key));
        }

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
        let ripped_keys: Vec<u64> = ripped_item_list.values().map(|seed| seed.key).collect();
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
        // INVARIANT (slice C): the side-map insert MUST follow this
        // push (registry key→index coupling — see the key_index field
        // doc; the pin t6_registry_side_map_tracks_removal_rebuild
        // witnesses the rebuild half).
        self.keys.push(key);
        self.key_index.insert(key, self.keys.len() - 1);
        key
    }

    fn remove_all_doors(&mut self, room_key: u64) {
        self.remove_all_doors_impl(room_key);
    }

    fn add_complete_free_space_room(&mut self, shape: TileShape, layer: i32, id: i32) -> u64 {
        let key = self.alloc_key();
        self.rooms
            .push(ExpansionRoom::new_complete_free_space(shape, layer, id));
        // INVARIANT (slice C): the side-map insert MUST follow this
        // push (registry key→index coupling — see the key_index field
        // doc; the pin t6_registry_side_map_tracks_removal_rebuild
        // witnesses the rebuild half).
        self.keys.push(key);
        self.key_index.insert(key, self.keys.len() - 1);
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
        // INVARIANT (slice C): the side-map insert MUST follow this
        // push (registry key→index coupling — see the key_index field
        // doc; the pin t6_registry_side_map_tracks_removal_rebuild
        // witnesses the rebuild half).
        self.keys.push(key);
        self.key_index.insert(key, self.keys.len() - 1);
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

    fn room_key_of_door(&self, id: i32, door: &ExpansionDoor) -> Option<u64> {
        AutorouteEngine::room_key_of_door(self, id, door)
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

    fn is_stop_requested(&self) -> bool {
        // The inherent method (budget BEFORE flag — Java `:287-295`);
        // qualified call to avoid trait-impl recursion.
        AutorouteEngine::is_stop_requested(self)
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

    fn via_center(&self, item_key: u64) -> Option<Point> {
        let id = ItemId::new(u32::try_from(item_key).expect("item key fits u32"));
        // Java `DrillItem.getCenter()` — the stored center (buglog
        // 170: endPointsMatching reads it directly, never the lazy
        // drill-info transient).
        self.board.drill_center(id)
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
            // Java `:412-413` filters on the BASE `isObstacle(int)` face
            // (`Item.java:162-164`, overridden by NO item subclass):
            // keep an object when it does not CONTAIN every ignore net —
            // the trace-obstacle overrides (ConductionArea `:398`,
            // Component/ViaObstacleArea `:71`/`:100`) do NOT apply to
            // this walk, so a foreign NON-obstacle conduction area STAYS
            // in the results. (Pre-fix this called `is_trace_obstacle`,
            // which coincided with the base face before the virtual
            // dispatch was ported.) Rooms keep the trait face (Java tree
            // rooms are CompleteFreeSpaceExpansionRoom, whose
            // `isObstacle(int)` is unconditionally true — `:77-79`).
            // ONE shared branch with the drill replay harness
            // (quality-review T17b M-Q1) — [`ignore_nets_key_is_obstacle`].
            if ignore_nets
                .iter()
                .all(|&net| ignore_nets_key_is_obstacle(self, self.board, leaf.object_key, net))
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
        // Java `RoutingBoard.checkTraceSegment(LineSegment, ...)`
        // (`RoutingBoard.java:223-233`) delegates the SEGMENT straight
        // through to the facade overload — `lineSegment.toPolyline()`
        // over the segment's OWN start/middle/end lines; it never
        // materializes the corner points. The old port decomposed to
        // `startPoint()`/`endPoint()` and re-ran the POINTS overload:
        // `middle.intersection(end)` is a RationalPoint whenever the
        // corner is not int-divisible (Polyline.java:11-13 — the
        // designed-for case), and `Polyline(Point, Point)` then carries
        // rational endpoints into `Line.intersectionApprox`, whose
        // `(IntPoint)` casts Java never executes on this path (a CCE
        // Java never hits, because Java never builds that
        // reconstruction — buglog 169; the decomposition also skew
        // rebuilt closing lines). The last flag is the Java
        // `onlyNotShovableObstacles` arm of the same overload family.
        check_trace_segment_facade(
            self.manager,
            self.board,
            line_segment,
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

    fn shove_trace_check(
        &mut self,
        line_segment: &LineSegment,
        shove_to_the_left: bool,
        layer: i32,
        net_numbers: &[i32],
        half_width: i32,
        clearance_class: i32,
        max_shove_trace_recursion_depth: i32,
        max_shove_via_recursion_depth: i32,
    ) -> f64 {
        // Java `MazeTraceShover.checkShoveTraceLine` (:202-212) calls
        // the STATIC `TraceShover.check` (board/optimize/TraceShover.java
        // :57-225) — the read-only shove-feasibility probe running on
        // the DEFAULT search tree (it recurses through substitute trace
        // pieces and obstacle vias but never mutates the board, so the
        // maze probe stays within its read-only contract). The port is
        // `epic_board::trace_shover::check_max_length` — ported
        // arm-for-arm from `TraceShover.check`; observed end-to-end by
        // the events compare and the router battery (no probe-replay
        // pin exists). The T10 0.0 stub short-circuited every maze
        // shove to "impossible" (all door lists empty, ripup only) —
        // the t7 events divergence (M4-T6).
        epic_board::trace_shover::check_max_length(
            &mut *self.manager,
            self.board,
            line_segment,
            shove_to_the_left,
            layer,
            net_numbers,
            half_width,
            clearance_class,
            max_shove_trace_recursion_depth,
            max_shove_via_recursion_depth,
        )
    }

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
/// `PolylineTrace` → `polylinetrace`. `pub(crate)`: the batch driver's
/// unrouted-connections report (`AutorouteUnroutedReport.describeItem`)
/// falls back to the same face — one port, two consumers (T12).
pub(crate) fn item_to_string(board: &Board, key: u64) -> String {
    let id = ItemId::new(u32::try_from(key).expect("item key fits u32"));
    let Some(entry) = board.get(id) else {
        // Java reads `Item.toString()` on the OBJECT REFERENCES held by
        // the connection sets — an item removed from the board (a
        // fanout retry after a same-net ripup removed it) still prints
        // there, because GC keeps the object alive. The arena drops the
        // data on removal, so a dead key degrades to a marker instead
        // of aborting the run (E2E witness: Issue420-contribution-board,
        // fanout attempt 2, panic at this line). Callers that can
        // recover the ripped name use `describe_connection`'s seed
        // lookup. Intentional log-face divergence from Java: the marker
        // is truthful about removal where Java would silently print a
        // stale-but-valid name.
        return format!("item #{key} (removed)");
    };
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
/// `", "`, then `" and "`, then the destination set. A key whose item
/// the board no longer holds falls back to the ripped-item seeds —
/// `RippedItemSeed::simple_name` IS the Java class face, harvested
/// while the item was live — and only then to the plain removed
/// marker. The seed lookup covers the case that makes a key dead in
/// practice: attempt 1 of a fanout pin rips a same-net item out of the
/// connection set, and attempt 2 (sharing `ripped_item_list`) describes
/// the set at entry.
fn describe_connection(
    board: &Board,
    ripped_items: &BTreeMap<i32, RippedItemSeed>,
    start_items: &[u64],
    dest_items: &[u64],
) -> String {
    let describe_key = |&key: &u64| {
        let live = board
            .get(ItemId::new(u32::try_from(key).expect("item key fits u32")))
            .is_some();
        if !live && let Some(seed) = ripped_items.values().find(|seed| seed.key == key) {
            return format!("{} (ripped up)", seed.simple_name);
        }
        item_to_string(board, key)
    };
    let start = start_items
        .iter()
        .map(&describe_key)
        .collect::<Vec<_>>()
        .join(", ");
    let dest = dest_items
        .iter()
        .map(&describe_key)
        .collect::<Vec<_>>()
        .join(", ");
    format!("{start} and {dest}")
}

// ---------------------------------------------------------------------------
// the lifecycle faces (Java RoutingBoard.initAutoroute/finishAutoroute)
// ---------------------------------------------------------------------------

/// Java `RoutingBoard.initAutoroute` (`:882-905`) — the REBUILD arm of
/// the lifecycle: construct a fresh engine over the class-resolved
/// compensated tree and prime it with
/// [`AutorouteEngine::init_connection`]. A replaced engine is dropped
/// WITHOUT `clear()` (the caller's choice — see [`finish_autoroute`]):
/// its completed rooms legitimately remain in the shared (same-class)
/// tree, exactly as in Java, and the tree-level continuity is carried
/// by the manager (the same class resolves to the same tree index).
///
/// OWNERSHIP NOTE (quality-review F-Q3, upgraded to a signature fix):
/// Java's reuse branch (`:888-897` — keep the engine iff
/// `maintainDatabase` AND its tree is already compensated for the same
/// class, then only `initConnection`) works by mutating the
/// `this.autorouteEngine` FIELD. The port's engine OWNS its `&mut`
/// manager/board borrows, so a `slot: Option<AutorouteEngine<'a>>`
/// parameter could never be called — passing the slot holds those
/// borrows while the same call re-lends them (E0499; verified by
/// compile probe; no caller can exist). The reuse face therefore lives
/// on the HELD engine: [`AutorouteEngine::is_reusable_for`] is the
/// verbatim `:888-897` condition, and the T12 pass loop expresses the
/// Java branch as
/// `if engine.is_reusable_for(class, retain) {
///     engine.init_connection(..)            // the reuse arm
/// } else {
///     engine = init_autoroute(..)           // this rebuild arm
/// }`.
pub fn init_autoroute<'a>(
    manager: &'a mut SearchTreeManager,
    board: &'a mut Board,
    net_number: i32,
    trace_clearance_class_index: i32,
    retain_database: bool,
    stoppable_flag: Option<Arc<AtomicBool>>,
    budget: Option<RouteBudget>,
) -> AutorouteEngine<'a> {
    let mut engine =
        AutorouteEngine::new(manager, board, trace_clearance_class_index, retain_database);
    engine.init_connection(net_number, stoppable_flag, budget);
    engine
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
            fanout: Default::default(),
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

    /// The room-key-space boundary pin. Before quality-review T17b
    /// M-Q1 this compared three module-local consts for equality
    /// (engine, drill harness, expansion harness); they are now ONE
    /// definition re-imported everywhere, so the equality face is
    /// enforced by construction and this pin guards the BOUNDARY VALUE
    /// itself: keys `ROOM_KEY_BASE - 1` are items, `ROOM_KEY_BASE` and
    /// above are rooms (the key-space partition the tree dispatch and
    /// the ignore-nets walk branch on).
    #[test]
    fn t11_room_key_base_const_equality() {
        assert_eq!(ROOM_KEY_BASE, 1u64 << 40);
        assert!(!is_room_key(ROOM_KEY_BASE - 1));
        assert!(is_room_key(ROOM_KEY_BASE));
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

    /// M11-T4 fix round (2026-10-03): the deterministic ladder carries
    /// the [`RouteBudget::SEARCH_TICK_CEILING`] backstop —
    /// `min(ladder, 2^22)`. Boundary derivation: the ladder
    /// `100000 * 2^(N-1)` crosses `2^22 = 4,194,304` between N=6
    /// (3,200,000 — the ladder face survives) and N=7 (6,400,000 —
    /// the ceiling face); every later pass including the i32
    /// saturation region pins the ceiling (the gv-iu pass-35
    /// explosion measured an INT_MAX ladder — buglog 256).
    #[test]
    fn t12_deterministic_ladder_carries_the_tick_ceiling() {
        assert_eq!(
            RouteBudget::deterministic_for_pass(1).limit_value(),
            100_000,
            "pass 1: the Java ladder value, below the ceiling"
        );
        assert_eq!(
            RouteBudget::deterministic_for_pass(6).limit_value(),
            3_200_000,
            "pass 6: the LAST uncapped ladder rung (boundary - 1)"
        );
        assert_eq!(
            RouteBudget::deterministic_for_pass(7).limit_value(),
            RouteBudget::SEARCH_TICK_CEILING,
            "pass 7: the FIRST ceiling rung (boundary)"
        );
        assert_eq!(
            RouteBudget::deterministic_for_pass(35).limit_value(),
            RouteBudget::SEARCH_TICK_CEILING,
            "pass 35: the old INT_MAX saturation now the ceiling"
        );
    }

    /// The stop faces: no flag/limit → false; an already-exceeded time
    /// limit → true; a raised stoppable flag → true; the deterministic
    /// budget answers true strictly AFTER its limit-th tick (the `>`
    /// mirrors `TimeLimit.limitExceeded`), consuming one tick per
    /// consultation. The flag is CLEAR during the deterministic ticks:
    /// Java's budget only short-circuits on its own true
    /// (`AutorouteEngine.java:291-294`); while unspent the walk falls
    /// through to the flag (`:295-296`), so a raised flag answers true
    /// under a live budget — pinned by the both-raised arm at the end
    /// (a budget that suppressed the flag while unspent fails it).
    #[test]
    fn t11_is_stop_requested_faces() {
        let (mut manager, mut board) = parse_fixture();
        let (mut engine, _ctrl) = build_engine(&mut manager, &mut board, 94, true);
        assert!(!engine.is_stop_requested(), "no limit, no flag");
        engine.budget = Some(RouteBudget::Wall(TimeLimit::new(-1)));
        assert!(engine.is_stop_requested(), "an expired limit stops");
        engine.budget = None;
        let flag = Arc::new(AtomicBool::new(false));
        engine.stoppable_flag = Some(Arc::clone(&flag));
        assert!(!engine.is_stop_requested(), "a clear flag does not stop");
        flag.store(true, Ordering::Relaxed);
        assert!(engine.is_stop_requested(), "a raised flag stops");

        // The deterministic face (SEAM: units deviation — Java's
        // wall-clock `TimeLimit` becomes a call-tick budget): limit 2
        // → ticks 1 and 2 answer false, tick 3 answers true (the
        // strict `>`); construction mirrors the Java
        // `(int) min(100000 * 2^(pass-1), Integer.MAX_VALUE)` ladder
        // — then min's [`RouteBudget::SEARCH_TICK_CEILING`] (buglog
        // 256: Java's i32::MAX rung left late deterministic passes
        // effectively unbudgeted; the ceiling, 4,194,304 < i32::MAX,
        // makes the Java cap unreachable through it). The flag is
        // cleared FIRST — Java still consults it while the
        // budget says not-stop (`:295-296`), so a raised flag would
        // answer true under a live budget.
        flag.store(false, Ordering::Relaxed);
        assert_eq!(
            RouteBudget::deterministic_for_pass(1).limit_value(),
            100_000,
            "pass 1: 100000 ms"
        );
        assert_eq!(
            RouteBudget::deterministic_for_pass(2).limit_value(),
            200_000,
            "pass 2: 200000 ms"
        );
        assert_eq!(
            RouteBudget::deterministic_for_pass(30).limit_value(),
            RouteBudget::SEARCH_TICK_CEILING,
            "the ladder caps at the search-tick ceiling (i32::MAX only beneath it)"
        );
        engine.budget = Some(RouteBudget::Deterministic {
            limit: 2,
            spent: std::cell::Cell::new(0),
        });
        assert!(!engine.is_stop_requested(), "tick 1 of 2");
        assert!(!engine.is_stop_requested(), "tick 2 of 2");
        assert!(engine.is_stop_requested(), "tick 3 exceeds the strict >");
        // The spent budget keeps stopping (flag clear — its own true).
        assert!(
            engine.is_stop_requested(),
            "the spent budget keeps stopping"
        );
        // BOTH raised, budget NOT spent: Java still answers true,
        // through the FLAG (`:295-296` — the budget short-circuits
        // only on its own true). A budget that suppressed the flag
        // while unspent answers false here and fails this arm.
        engine.budget = Some(RouteBudget::Deterministic {
            limit: 5,
            spent: std::cell::Cell::new(0),
        });
        flag.store(true, Ordering::Relaxed);
        assert!(
            engine.is_stop_requested(),
            "both raised, budget unspent: the flag answers"
        );
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

    /// Bug-232 pin (the second contribution-board crash): keys of items
    /// no longer on the board — the post-ripup state carried by maze
    /// init's connection items and target doors — answer Java's
    /// `board == null` faces (zero shapes / None) instead of panicking.
    /// LIVE items keep the warm-cache invariant (a live miss still
    /// panics; t11_peek_and_layer_read_match_fresh_derivation pins the
    /// live answers against fresh derivation).
    #[test]
    fn t11_dead_item_keys_answer_no_shapes_not_panic() {
        use crate::drill::DrillEngine;
        let (mut manager, mut board) = parse_fixture();
        let (engine, _ctrl) = build_engine(&mut manager, &mut board, 94, true);
        for dead in [424_242u64, 646_464] {
            assert!(
                engine.item_tree_shapes(dead).is_empty(),
                "dead key {dead}: no shapes remain"
            );
            assert_eq!(
                DrillEngine::item_tree_shape_count(&engine, dead),
                0,
                "dead key {dead}: Java treeShapeCount board == null → 0"
            );
            assert!(
                NeighbourEngine::tree_shape(&engine, dead, 0).is_none(),
                "dead key {dead}: Java getTreeShape board == null → null"
            );
        }
        let pin_key = net_pin_key(engine.board, 94);
        assert!(
            !engine.item_tree_shapes(pin_key).is_empty(),
            "live pin {pin_key}: the warm cache still answers"
        );
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
        // The id-burn row FIELDS at discriminating values (quality
        // OBS-Q5): the delta is not constant — the first three forced
        // inserts of the route's FIRST leg burn 3/1/1 — and the
        // maxItemId watermark CHAINS across rows (105 → 108 → 109 →
        // 110; the leg itself runs 7 segments to watermark 114, then
        // the two junction via inserts + further legs continue past
        // it), so a constant-delta or before/after-swap mutant cannot
        // satisfy all three literals. #931 cluster-F rotation
        // (2026-10-03): the old world's first leg ran 3 segments
        // (5/1/0, watermark 105 → 110 → 111 → 111) — the ported
        // door-centering walks a different room chain, so leg 1 now
        // burns 3 then 1s across seven segments.
        assert!(
            joined.contains(
                "compare_trace_insert_segment_ids net=94, i=1, \
                 maxItemIdBefore=105, maxItemIdAfter=108, delta=3"
            ),
            "i=1 id-burn row. rows:\n{joined}"
        );
        assert!(
            joined.contains(
                "compare_trace_insert_segment_ids net=94, i=2, \
                 maxItemIdBefore=108, maxItemIdAfter=109, delta=1"
            ),
            "i=2 id-burn row. rows:\n{joined}"
        );
        assert!(
            joined.contains(
                "compare_trace_insert_segment_ids net=94, i=3, \
                 maxItemIdBefore=109, maxItemIdAfter=110, delta=1"
            ),
            "i=3 id-burn row. rows:\n{joined}"
        );
        // The route reached the anchor: the anchor's seeded east end
        // (620000,300000) survives on its split east half (108) — the
        // splice itself lands at the anchor midpoint (#931 rotation,
        // see the canon block below).
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
        // corners are literally the leg's own okPoints; the layer-0
        // head/tail corners are the splice point on the anchor and
        // the far okPoint), the jar-exact pin leg (no staircase to
        // tighten), and the anchor SPLIT at the splice point.
        //
        // #931 cluster-F rotation (2026-10-03): the hunk-2
        // door-section point rule (locator_45
        // `calculate_next_trace_corners` — degenerate-short section
        // takes the midpoint; long sections shrink by
        // trace_halfwidth_add before the nearest-point pick) moved the
        // route's root on the anchor from its EAST END (620000,300000)
        // to the anchor MIDPOINT (610000,300000). The anchor now
        // SPLICES at its middle and survives as its two collinear
        // halves (107 west, 108 east); the pre-rotation world re-rooted
        // at the anchor's far corner and the west stub was dropped. The
        // jar capture above is PRE-#931 Java; the splice-point face is
        // decided BEFORE the tightener, so it is jar-adjudicable
        // against post-#931 Java (the bisect tree survives at
        // logs/java-oracle-m11/post) — not probed in T4, whose
        // adjudication ran on optimizer-score faces (buglog 251
        // dossier).
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
            "107 trace layer=0 (600000,300000) (610000,300000)\n\
             108 trace layer=0 (610000,300000) (620000,300000)\n\
             114 trace layer=0 (610000,300000) (607248,302752) (521250,302752) (521248,302754) \
             (487359,302754) (480738,309375)\n\
             115 via center=(480738,309375)\n\
             119 trace layer=1 (480738,309375) (502752,309375) (667957,144170) (667957,9800)\n\
             120 via center=(667957,9800)\n\
             122 trace layer=0 (667957,9800) (663500,14257) (663500,20000)",
            "net-94 board canon diverged from the NoPullTight engine verdict. rows:\n{joined}"
        );
        // The anchor SPLITS at the splice point (#931 rotation; was:
        // "consumed" — no net-94 item retained the anchor's west stub
        // corner). The route roots at the anchor midpoint
        // (610000,300000), so BOTH collinear halves survive — 107 west
        // (still carrying the anchor's seeded west corner 600000,300000)
        // and 108 east (carrying the seeded east corner 620000,300000);
        // a normalize step that dropped or merged either half would
        // move this pin.
        let anchor_halves_present = board.iter_descending().any(|entry| {
            entry.nets.contains(&94)
                && matches!(entry.data, ItemData::Trace { .. })
                && board.trace_polyline(entry.id).is_some_and(|polyline| {
                    polyline
                        .corners()
                        .contains(&Point::Int(corner(600000, 300000)))
                })
        }) && board.iter_descending().any(|entry| {
            entry.nets.contains(&94)
                && matches!(entry.data, ItemData::Trace { .. })
                && board.trace_polyline(entry.id).is_some_and(|polyline| {
                    polyline
                        .corners()
                        .contains(&Point::Int(corner(620000, 300000)))
                })
        });
        assert!(
            anchor_halves_present,
            "both anchor halves survive the midpoint splice. rows:\n{joined}"
        );
        // The search populated the room registry (maintained database
        // survives the reset_all_doors cleanup).
        assert!(
            rooms_present,
            "completed rooms persist under maintain_database"
        );
    }

    /// THE buglog-169 pin (T17a, RED-BEFORE-GREEN): the engine's
    /// segment-face `checkTraceSegment` passes the segment's OWN
    /// start/middle/end lines through, like Java
    /// `RoutingBoard.checkTraceSegment(LineSegment, ...)`
    /// (`RoutingBoard.java:223-233` → facade overload →
    /// `lineSegment.toPolyline()`); it must NOT decompose to
    /// `startPoint()`/`endPoint()` and re-run the points overload.
    /// This segment's end corner `middle ∩ end` is RATIONAL — x =
    /// 9_799_970_000 / 20_000 = 489998.5, the Polyline.java:11-13
    /// designed-for case: the decomposition materializes it and
    /// re-enters `Polyline(Point, Point)` with rational endpoints,
    /// where `Line.intersectionApprox`'s int cast panics — a cast
    /// Java NEVER executes on this path (its segment face never
    /// builds that reconstruction; Java's own points overload would
    /// CCE there too, and the oracle completes these boards). The
    /// corridor is the one the epic-board jar-literal world pins
    /// clear (`t11_check_trace_segment_jar_literals` S0): the verdict
    /// is Java's no-conflict `Integer.MAX_VALUE`.
    #[test]
    fn t17_segment_face_survives_a_rational_corner() {
        let (mut manager, mut board) = parse_fixture();
        // vertical start closing line at x=480000, horizontal middle
        // along y=300000, slope-2 end closing line crossing the
        // middle at (489998.5, 300000).
        let start = epic_geometry::line::Line::from_int_coords(480000, 300000, 480000, 301000);
        let middle = epic_geometry::line::Line::from_int_coords(480000, 300000, 490000, 300000);
        let end = epic_geometry::line::Line::from_int_coords(489999, 300001, 490000, 300003);
        let segment = LineSegment::new(start, middle, end);
        // The pin's premise: the end corner really is rational (an
        // int-corner world cannot witness the decomposition panic).
        assert!(
            matches!(segment.end_point(), Point::Rational(_)),
            "the crafted end corner is the designed-for rational case"
        );
        let (mut engine, _ctrl) = build_engine(&mut manager, &mut board, 94, true);
        let verdict = engine.check_trace_segment(&segment, 0, &[94], 1500, 0, false);
        assert_eq!(verdict, 2147483647.0, "clear corridor answers MAX_VALUE");
    }

    /// The buglog-169 int-world literal pin: the engine segment face
    /// answers the SAME jar-captured literals as the free-function
    /// face (epic-board `t11_check_trace_segment_jar_literals`,
    /// S1_unfixed_false / S2_fixed_true / S3_unfixed_true) on the
    /// identical world — the unfixed wall at x=505000 and the
    /// USER_FIXED wall at x=545000, ids replaying 105/106. S3 is the
    /// FLAG CROSSING the S1/S2 pair never makes (fix round F-2 /
    /// SR-3: S1 is unfixed×false, S2 fixed×true — neither crosses
    /// flag × shovability): the S1 span with
    /// `onlyNotShovableObstacles=true` makes the UNFIXED wall
    /// INVISIBLE (RoutingBoard.java:196 — "unfixed traces and vias
    /// are ignored"; facade `:67-71` skips `isRoutable() &&
    /// !isShoveFixed()` obstacles), so the corridor answers
    /// MAX_VALUE. Oracle row `S1_unfixed_true` =
    /// 2.147483647E9/max_value=true
    /// (logs/M3-T11/captures/autoroute_engine_rows_run1.jsonl —
    /// also the empty-case answer NO other fixed obstacle hides in
    /// this span). This is the no-int-world-behavior-change witness
    /// the events corpus posture rides on (int corners are
    /// equivalent through both faces; rational corners are not).
    #[test]
    fn t17_segment_face_answers_the_jar_literals() {
        let (mut manager, mut board) = parse_fixture();
        let unfixed_wall = epic_board::trace_ops::insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(
                &Point::Int(corner(505000, 100000)),
                &Point::Int(corner(505000, 500000)),
            ),
            0,
            1000,
            &[2],
            0,
            FixedState::Unfixed,
        )
        .expect("unfixed wall");
        assert_eq!(unfixed_wall.get(), 105, "id replay (the t11 jar world)");
        let fixed_wall = epic_board::trace_ops::insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(
                &Point::Int(corner(545000, 100000)),
                &Point::Int(corner(545000, 500000)),
            ),
            0,
            1000,
            &[2],
            0,
            FixedState::UserFixed,
        )
        .expect("fixed wall");
        assert_eq!(fixed_wall.get(), 106, "id replay (the t11 jar world)");

        let s1 = Polyline::from_two_corners(
            &Point::Int(corner(500000, 300000)),
            &Point::Int(corner(520000, 300000)),
        );
        let s2 = Polyline::from_two_corners(
            &Point::Int(corner(535000, 300000)),
            &Point::Int(corner(555000, 300000)),
        );
        let (mut engine, _ctrl) = build_engine(&mut manager, &mut board, 94, true);
        assert_eq!(
            engine.check_trace_segment(
                &LineSegment::from_polyline(&s1, 1),
                0,
                &[94],
                1500,
                0,
                false
            ),
            2483.0,
            "S1_unfixed_false through the engine segment face"
        );
        // The flag crossing (fix round F-2): the SAME S1 span, the
        // flag FLIPPED — the only difference between this assert and
        // S1 is `onlyNotShovableObstacles`, so the two asserts
        // together observe the flag's passthrough on the engine face.
        assert_eq!(
            engine.check_trace_segment(
                &LineSegment::from_polyline(&s1, 1),
                0,
                &[94],
                1500,
                0,
                true
            ),
            2147483647.0,
            "S3_unfixed_true through the engine segment face: the \
             unfixed wall is ignored, the corridor answers MAX_VALUE"
        );
        assert_eq!(
            engine.check_trace_segment(
                &LineSegment::from_polyline(&s2, 1),
                0,
                &[94],
                1500,
                0,
                true
            ),
            7483.0,
            "S2_fixed_true through the engine segment face"
        );
    }

    /// The buglog-170 PRODUCTION-impl pin (T17a fix round, review
    /// F-3 / SR-4): P4 (`maze/pins.rs`) drives the pins-Harness
    /// `DrillEngine` impl; the impl the battery actually routes
    /// through is THIS engine's `via_center` (the
    /// `impl DrillEngine for AutorouteEngine` body →
    /// `Board::drill_center`, the STORED center — Java
    /// `DrillItem.getCenter()`, `DrillItem.java:229`, never the lazy
    /// `getAutorouteDrillInfo` transient of `Via.java:204-216`).
    /// The P4 craft replayed through the PRODUCTION handle:
    /// `parse` → `build_engine` → the premise (drill info ABSENT —
    /// exactly the world whose old `expect` panicked the battery's
    /// bm06/bm07 at this face), the literal stored center, and the
    /// `endPointsMatching` TRUE / FALSE / foreign-net arms through
    /// the production engine's own trait impl.
    #[test]
    fn t17_production_engine_via_center_answers_the_stored_center() {
        let (mut manager, mut board) = parse(crate::maze::pins::T17_VIA_MATCH_DSN);
        let mine = crate::test_util::net_no(&board, "MINE");
        let mut via_key = None;
        let mut w1_key = None;
        let mut w2_key = None;
        let mut pin_key = None;
        for entry in board.iter_ascending() {
            let key = u64::from(entry.id.get());
            match &entry.data {
                ItemData::Via { .. } => via_key = Some(key),
                ItemData::Pin { .. } => pin_key = Some(key),
                ItemData::Trace { lines, .. } => {
                    // The P4 discovery rule: W1's far corner sits AT
                    // the via center; W2 is the far trace.
                    let near_via = lines.corners().iter().any(|corner| {
                        matches!(corner, Point::Int(center) if (center.x, center.y) == (40000, 40000))
                    });
                    if near_via {
                        w1_key = Some(key);
                    } else {
                        w2_key = Some(key);
                    }
                }
                _ => {}
            }
        }
        let (via_key, w1_key, w2_key, pin_key) = (
            via_key.expect("the craft carries a via"),
            w1_key.expect("W1 ends at the via center"),
            w2_key.expect("W2 is the far trace"),
            pin_key.expect("the OTHER-net pin"),
        );
        let (engine, _ctrl) = build_engine(&mut manager, &mut board, mine, false);
        use crate::drill::DrillEngine;
        assert!(
            engine.via_drill_info(via_key).is_none(),
            "the witness world leaves drill info absent (the buglog-170 world)"
        );
        assert_eq!(
            engine.via_center(via_key).map(|point| point.to_float()),
            Some(epic_geometry::float_point::FloatPoint::new(
                40000.0, 40000.0
            )),
            "the PRODUCTION via_center reads the stored center"
        );
        assert!(
            crate::maze::shove_probe::end_points_matching(&engine, w1_key, via_key),
            "via center == W1 end corner: Java's TRUE arm, production engine"
        );
        assert!(
            !crate::maze::shove_probe::end_points_matching(&engine, w2_key, via_key),
            "W2's corners are not at the via center: FALSE, production engine"
        );
        assert!(
            !crate::maze::shove_probe::end_points_matching(&engine, w1_key, pin_key),
            "foreign net: the sharesNet gate answers FALSE, production engine"
        );
    }

    /// Quality-review T17b M-Q1(3) — the PRODUCTION ignore-nets walk
    /// pin (pin mode 13(c): a trait with multiple impls needs its pin
    /// driven through EACH impl the callers actually use; the M-2 pin
    /// in `drill/pins.rs` drives the harness copy, THIS one drives
    /// the `impl DrillEngine for AutorouteEngine` copy the ripup
    /// resolver consumes — `maze/ripup.rs` — through
    /// `build_engine`, the real `settings_ir` cost table, and the
    /// real manager tree). The SAME `PLANE_WALK_DSN` world and the
    /// SAME flag × net discriminator as the harness pin: a foreign
    /// NON-obstacle plane STAYS in the walk's results (the base-face
    /// crossing cell), an own-net object drops, the netless keepout
    /// stays on both. The trait-face-for-all-keys mutant at the
    /// shared helper (`ignore_nets_key_is_obstacle`) must kill BOTH
    /// this pin and the harness pin.
    #[test]
    fn t17b_production_ignore_nets_walk_filters_items_on_the_base_obstacle_face() {
        let (mut manager, mut board) = parse(crate::drill::pins::PLANE_WALK_DSN);
        let sig = crate::test_util::net_no(&board, "SIG");

        // The world, read off the parsed board BEFORE the engine
        // borrows it: the plane is a NON-obstacle ConductionArea on
        // GND (parse flag), the keepout a plain netless ObstacleArea.
        let plane_entry = board
            .iter_descending()
            .find(|e| matches!(e.data, ItemData::ConductionArea { .. }))
            .expect("the plane parsed as a ConductionArea");
        let plane_id = plane_entry.id;
        let (gnd, plane_flag) = match &plane_entry.data {
            ItemData::ConductionArea { is_obstacle, .. } => (plane_entry.nets[0], *is_obstacle),
            other => panic!("not a plane: {other:?}"),
        };
        let keepout_key = u64::from(
            board
                .iter_descending()
                .find(|e| matches!(e.data, ItemData::ObstacleArea { .. }))
                .expect("the keepout parsed as an ObstacleArea")
                .id
                .get(),
        );
        let plane_key = u64::from(plane_id.get());
        assert!(
            !plane_flag,
            "a parse-time plane is NON-obstacle (Structure.java:1113)"
        );
        assert_ne!(sig, gnd, "SIG is foreign to the plane");
        // The premise — the two faces genuinely DISAGREE on the
        // crossing item (base keeps the foreign plane, the virtual
        // face never does).
        assert!(
            board.item_is_obstacle(plane_id, sig),
            "base face: the foreign plane is an obstacle w.r.t. SIG"
        );
        assert!(
            !board.item_is_trace_obstacle(plane_id, sig),
            "virtual face: a non-obstacle plane never is"
        );

        let (engine, _ctrl) = build_engine(&mut manager, &mut board, sig, false);
        use crate::drill::DrillEngine;

        // The probe over the plane + keepout on In1.Cu (index 1), in
        // INTERNAL units — the same probe as the harness pin.
        let probe = TileShape::RegularTileShape(
            epic_geometry::regular_tile_shape::RegularTileShape::IntBox(
                epic_geometry::int_box::IntBox::new(
                    epic_geometry::int_point::IntPoint::new(30000, 30000),
                    epic_geometry::int_point::IntPoint::new(70000, 70000),
                ),
            ),
        );

        // Sanity (observability): with NO ignore nets everything
        // stays — both items reach the production walk at all (the
        // engine's manager tree, not the harness's).
        let all = DrillEngine::overlapping_objects_ignore_nets(&engine, &probe, 1, &[]);
        assert!(
            all.contains(&plane_key),
            "no ignore nets: the plane is in the production walk"
        );
        assert!(
            all.contains(&keepout_key),
            "no ignore nets: the keepout is in the production walk"
        );

        // THE CROSSING CELL (plane × foreign): the BASE face keeps
        // the NON-obstacle plane — the trait-face-for-all-keys mutant
        // drops it exactly here.
        let foreign = DrillEngine::overlapping_objects_ignore_nets(&engine, &probe, 1, &[sig]);
        assert!(
            foreign.contains(&plane_key),
            "foreign net: a NON-obstacle plane STAYS (base face, :412-413)"
        );
        assert!(foreign.contains(&keepout_key), "foreign net: keepout stays");

        // Plane × own: drops (`!containsNet` false under every face).
        // Keepout × own: the both-faces-agree control arm.
        let own = DrillEngine::overlapping_objects_ignore_nets(&engine, &probe, 1, &[gnd]);
        assert!(!own.contains(&plane_key), "own net: the plane drops");
        assert!(
            own.contains(&keepout_key),
            "own net: the netless keepout stays"
        );
    }

    /// The ripup-removal block (Java `:230-249`, quality-review F-Q1):
    /// a caller-seeded `ripped_item_list` drives the walk even when the
    /// maze never needed a rip (the list is caller-owned input — the
    /// locator only ever INSERTS into it). Witnesses: the LISTED
    /// foreign trace vanishes (the block runs: accumulation →
    /// `remove_item_through_repository`), and the per-changed-net
    /// `remove_trace_tails` sweep takes the UNLISTED lone net-2 tail
    /// (fired by the listed item's net set alone). Mutation matrix:
    /// block-drop KILLED (listed survives), tail-sweep-drop KILLED
    /// (unlisted survives while listed is gone); accumulation-drop is
    /// SWEEP-SUBSUMED in every single-net trace world — a chain always
    /// dies from its 1-contact ends through the sweep's own
    /// connection walk, so dropping the accumulation is
    /// board-final-state-equivalent there (applied + survived;
    /// the SEAM bank carries the multi-net-via discriminating design).
    /// The two `.rev()` ORDER faces are board-final-state-invariant
    /// (set removal is order-independent) — their bank also lives in
    /// SEAM.md (the discriminator would be the undo-record order).
    /// WORLD-DESIGN HAZARD (learned the hard way): never shape the
    /// listed item's connection as a CLOSED LOOP — both engines'
    /// `getConnectionItems` walk has no cycle termination (Java
    /// `Item.java:746-817` spins the same way; `remove_if_cycle` is
    /// the only cycle-safe walk) and the pin hangs.
    #[test]
    fn t11_ripup_removal_walk_removes_listed_connection_and_net_tails() {
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
        // Two foreign net-2 traces in free space far west of the route
        // corridor (the canon path stays at x >= 480738): A is LISTED
        // (its whole connection accumulates through
        // `get_connection_items` and is removed), B is not — a lone
        // zero-contact trace only the per-changed-net sweep can take.
        let listed = epic_board::trace_ops::insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(
                &Point::Int(corner(200000, 100000)),
                &Point::Int(corner(240000, 100000)),
            ),
            0,
            1500,
            &[2],
            0,
            FixedState::Unfixed,
        )
        .expect("listed foreign trace");
        let unlisted = epic_board::trace_ops::insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(
                &Point::Int(corner(300000, 100000)),
                &Point::Int(corner(340000, 100000)),
            ),
            0,
            1500,
            &[2],
            0,
            FixedState::Unfixed,
        )
        .expect("unlisted foreign tail");
        let listed_key = u64::from(listed.get());
        let unlisted_key = u64::from(unlisted.get());
        // World self-check: both seeded traces are on the board (the
        // fixture carries a pre-existing net-2 item too — id 6 — which
        // the keyed asserts below simply ignore).
        let net2_before: Vec<u64> = board
            .iter_descending()
            .filter(|entry| entry.on_the_board && entry.nets.contains(&2))
            .map(|entry| u64::from(entry.id.get()))
            .collect();
        assert!(
            net2_before.contains(&listed_key) && net2_before.contains(&unlisted_key),
            "the world seeds the two net-2 traces; before: {net2_before:?}"
        );
        // The ripped list is keyed by the Java item id (= the key for
        // board items; `object_id` is `i32::try_from`). A caller
        // pre-seed carries the full seed face — production callers
        // always start from an empty map, the engine fills the seeds.
        let mut ripped = BTreeMap::new();
        ripped.insert(
            i32::try_from(listed_key).expect("item key fits i32"),
            RippedItemSeed {
                key: listed_key,
                simple_name: "PolylineTrace",
                nets: vec![2],
            },
        );
        let (routed, joined) = {
            let (mut engine, mut ctrl) = build_engine(&mut manager, &mut board, 94, true);
            engine.init_connection(94, None, None);
            let mut sink = CaptureSink::default();
            let result = engine.autoroute_connection(
                &[pin_key],
                &[anchor_key],
                &mut ctrl,
                &mut ripped,
                &mut HashMap::new(),
                &mut NoPullTight,
                &mut sink,
            );
            (
                result.state == AutorouteAttemptState::Routed,
                sink.rows.join("\n"),
            )
        };
        assert!(
            routed,
            "the off-corridor rip leaves the route untouched. rows:\n{joined}"
        );
        let net2_after: Vec<u64> = board
            .iter_descending()
            .filter(|entry| entry.on_the_board && entry.nets.contains(&2))
            .map(|entry| u64::from(entry.id.get()))
            .collect();
        assert!(
            !net2_after.contains(&listed_key),
            "the LISTED connection must be removed by the ripup block; \
             remaining: {net2_after:?}. rows:\n{joined}"
        );
        assert!(
            !net2_after.contains(&unlisted_key),
            "the UNLISTED lone net-2 tail must be swept by \
             remove_trace_tails(2, _); remaining: {net2_after:?}. rows:\n{joined}"
        );
        // The route itself still landed on the anchor end.
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
    }

    /// The `init_autoroute` reuse faces (Java `RoutingBoard.initAutoroute`
    /// `:888-897`, quality-review F-Q3): the reuse CONDITION
    /// (`is_reusable_for` — maintain AND same compensated class) is
    /// true for the held engine, and honoring it carries the registry
    /// through `init_connection` (same net purges nothing); both
    /// mismatch faces are false. The rebuild arm (a fresh engine) gets
    /// an empty registry and zeroed room ids; the manager carries the
    /// TREE continuity (same class → same tree index) while a
    /// different class resolves a different tree. Both directions kill
    /// the retain-flip mutant (`retain_database &&` →
    /// `!retain_database &&`): face 1 (maintain) must answer true,
    /// face 2 (no maintain) must answer false.
    #[test]
    fn t11_init_autoroute_reuse_faces() {
        let (mut manager, mut board) = parse_fixture();
        // Resolve the class the way build_engine does, then populate a
        // first engine's registry with a real route.
        let layer_count = board.layers().layers.len();
        let probe = AutorouteControl::new(&mut board, 94, &settings_ir(layer_count));
        let class = probe.trace_clearance_class_index;
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
            &mut CaptureSink::default(),
        );
        assert_eq!(result.state, AutorouteAttemptState::Routed);
        let tree_a = engine.autoroute_tree;
        let counter_a = engine.id_counter;
        assert!(counter_a > 0, "the route created rooms (Java room ids)");
        assert!(!engine.keys.is_empty(), "the registry is populated");

        // Face 1 — the reuse condition, HELD engine, same class,
        // maintain: TRUE, and honoring it carries the registry.
        assert!(
            engine.is_reusable_for(class, true),
            "maintain + same compensated class reuses the engine"
        );
        engine.init_connection(94, None, None);
        assert_eq!(
            engine.id_counter, counter_a,
            "the registry carries over (reuse, not rebuild)"
        );
        assert!(!engine.keys.is_empty(), "the rooms survive the re-init");

        // Face 2 — same class, NOT maintained: FALSE (fresh engine in
        // Java's branch).
        assert!(
            !engine.is_reusable_for(class, false),
            "a not-maintained database never reuses"
        );
        // Face 3 — maintained, DIFFERENT class: FALSE (the tree is not
        // compensated for it).
        assert!(
            !engine.is_reusable_for(0, true),
            "a different class forces the rebuild arm"
        );
        drop(engine);

        // Face 4 — the REBUILD arm, same class: fresh registry, but
        // the MANAGER carries the tree continuity.
        let engine = init_autoroute(&mut manager, &mut board, 94, class, true, None, None);
        assert_eq!(
            engine.id_counter, 0,
            "a replaced engine starts an empty registry (Java room ids restart)"
        );
        assert!(engine.keys.is_empty(), "a fresh registry");
        assert_eq!(engine.autoroute_tree, tree_a, "same class, same tree");
        drop(engine);

        // Face 5 — the REBUILD arm, different class: a different
        // compensated tree.
        let engine = init_autoroute(&mut manager, &mut board, 94, 0, true, None, None);
        assert_ne!(
            engine.autoroute_tree, tree_a,
            "the different class re-resolves the tree"
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
        let describe = describe_connection(
            &board,
            &BTreeMap::new(),
            &[pin_key],
            &[u64::from(trace.get())],
        );
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
            &BTreeMap::new(),
            &[pin_key, u64::from(trace.get())],
            &[u64::from(second.get())],
        );
        assert!(
            multi.contains(", polylinetrace and polylinetrace"),
            "describe: {multi}"
        );
    }

    /// The dead-key degradation (E2E witness: Issue420-contribution-board
    /// panicked at the old `expect("a routed set holds live items")`
    /// during fanout attempt 2, after attempt 1's same-net ripup removed
    /// a start-set item): a key the board no longer holds renders from
    /// the ripped-item seed when one exists (the Java class face,
    /// harvested while live) and as the plain removed marker otherwise —
    /// never a panic. Java keeps such items printable via GC'd object
    /// references; the marker is the deliberate truthfulness divergence.
    #[test]
    fn t11_describe_connection_dead_keys_degrade() {
        let (_manager, board) = parse_fixture();
        let pin_key = net_pin_key(&board, 94);
        // A seed harvested for a key that is NOT on the board (the
        // post-ripup state of attempt 2) and a bare dead key with no
        // seed (removed by an earlier fanout pin).
        let ripped = BTreeMap::from([(
            94,
            RippedItemSeed {
                key: 424_242,
                simple_name: "via",
                nets: vec![94],
            },
        )]);
        let describe = describe_connection(&board, &ripped, &[pin_key, 424_242], &[646_464]);
        assert!(
            describe.contains("pin of component #93, via (ripped up) and item #646464 (removed)"),
            "describe: {describe}"
        );
        // Live items are unaffected by the seed map's presence.
        let live = describe_connection(&board, &ripped, &[pin_key], &[pin_key]);
        assert!(
            live.starts_with("pin") && live.contains(" and pin"),
            "describe: {live}"
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
        let mut engine = init_autoroute(&mut manager, &mut board, 94, class, true, None, None);
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

    /// PIN (M5 slice C) — the registry key→index side map tracks the
    /// order-preserving removal + rebuild (bm06 measured 77.3M
    /// registry resolves against 145k graveyard insertions, so the
    /// map's rebuild-on-removal is the correctness-critical half).
    /// Three rooms registered with GEOMETRICALLY DISTINCT shapes
    /// (lengths 1/2/3 — the quality-round fix: equal-length keys made
    /// the survivors shape-equal and collapsed the kill arms), the
    /// FIRST removed (every higher index shifts down by one): the
    /// removed room must still resolve (graveyard arm — Java's
    /// live-reference semantics) and each survivor must resolve to ITS
    /// OWN room. Skip-the-rebuild kill faces, each independently
    /// discriminative: `k2`'s stale index 1 lands on `k3`'s room
    /// (wrong-shape assert), `k3`'s stale index 2 points past the
    /// post-removal registry end (index-out-of-bounds panic).
    #[test]
    fn t6_registry_side_map_tracks_removal_rebuild() {
        let (mut manager, mut board) = parse_fixture();
        let (mut engine, _ctrl) = build_engine(&mut manager, &mut board, 94, true);
        let shape_of = |s: &str| -> TileShape {
            TileShape::RegularTileShape(
                epic_geometry::regular_tile_shape::RegularTileShape::IntBox(
                    epic_geometry::int_box::IntBox::new(
                        corner(100_000 * s.len() as i32, 0),
                        corner(200_000 * s.len() as i32, 100_000),
                    ),
                ),
            )
        };
        let k1 = NeighbourEngine::add_incomplete_expansion_room(
            &mut engine,
            shape_of("a"),
            0,
            shape_of("aa"),
        );
        let k2 = NeighbourEngine::add_incomplete_expansion_room(
            &mut engine,
            shape_of("bb"),
            0,
            shape_of("bbb"),
        );
        let k3 = NeighbourEngine::add_incomplete_expansion_room(
            &mut engine,
            shape_of("ccc"),
            0,
            shape_of("cccc"),
        );
        assert!(k1 != k2 && k2 != k3 && k1 != k3);
        // Pre-removal resolutions answer through the map.
        assert!(engine.resolve(k2).is_some(), "k2 resolves pre-removal");
        // Remove the FIRST room: the shift face.
        NeighbourEngine::remove_incomplete_room(&mut engine, k1);
        // The removed room stays reachable (graveyard arm).
        assert!(engine.resolve(k1).is_some(), "graveyard arm live");
        // Both survivors resolve — each to its OWN room, not a shifted
        // slot. Each survivor is ASSERTED before the next is resolved,
        // so either kill face fires independently (the k2 wrong-shape
        // assert is not masked by a later out-of-bounds panic).
        let r2 = engine.resolve(k2).expect("k2 resolves after shift");
        assert_eq!(*r2.shape(), shape_of("bb"), "k2 resolved to its own room");
        let r3 = engine.resolve(k3).expect("k3 resolves after shift");
        assert_eq!(*r3.shape(), shape_of("ccc"), "k3 resolved to its own room");
        finish_autoroute(Some(engine));
    }
    /// The fanout-via protection's 2-corner boundary (buglog 181, the
    /// bm06 fanout-stage fork origin). Java
    /// `MazeRipupResolver.calcFanoutViaRipupCostFactor` (:35-66)
    /// protects an obstacle trace whose single end contact is a
    /// SHOVE_FIXED trace with `cornerCount() == 2` — where Java
    /// `PolylineTrace.cornerCount()` = `lines.length - 1`
    /// (Polyline.java:178-180). The port's corner-count seam counted
    /// LINES (`lines.len()`), so every 2-corner contact answered 3 and
    /// the arm never fired: freshly inserted fanout escapes were
    /// rippable at base ripup cost in Rust while Java priced them at
    /// the `Integer.MAX_VALUE/100` clamp — the exact fork the bm06
    /// instrument captured (jar CHECK_RIPUP net=15 obstacle=177
    /// halfWidth=1000.0 ripupCosts=100 detour=1.0 result=21474836 —
    /// the clamp — where the pre-fix Rust computed 100000; evidence
    /// logs/M6-T2/). Pinned through the PRODUCTION
    /// `impl DrillEngine for AutorouteEngine` (DNR-13), both
    /// directions of the boundary: a 2-corner shove-fixed end contact
    /// protects (factor > 1, the formula literal), a 3-corner contact
    /// one step over the boundary does not (factor == 1.0).
    #[test]
    fn fanout_via_protection_two_corner_boundary() {
        let (mut manager, mut board) = parse_fixture();
        // The probed obstacle trace P: (600000,300000)->(620000,300000),
        // layer 0, halfWidth 1500, net 94. lengthApprox = 20000.
        let p_key = u64::from(
            epic_board::trace_ops::insert_trace_without_cleaning(
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
            .expect("probe trace")
            .get(),
        );
        // The contact trace C (2 corners: 3 lines, SHOVE_FIXED) whose
        // first corner touches P's end corner.
        let c_key = u64::from(
            epic_board::trace_ops::insert_trace_without_cleaning(
                &mut manager,
                &mut board,
                Polyline::from_two_corners(
                    &Point::Int(corner(620000, 300000)),
                    &Point::Int(corner(640000, 300000)),
                ),
                0,
                1000,
                &[94],
                0,
                FixedState::ShoveFixed,
            )
            .expect("2-corner contact trace")
            .get(),
        );
        // The second probe pair, far from the first: P2 with a 3-corner
        // SHOVE_FIXED dogleg touching its end corner only.
        let p2_key = u64::from(
            epic_board::trace_ops::insert_trace_without_cleaning(
                &mut manager,
                &mut board,
                Polyline::from_two_corners(
                    &Point::Int(corner(800000, 300000)),
                    &Point::Int(corner(820000, 300000)),
                ),
                0,
                1500,
                &[94],
                0,
                FixedState::Unfixed,
            )
            .expect("second probe trace")
            .get(),
        );
        let c4_key = u64::from(
            epic_board::trace_ops::insert_trace_without_cleaning(
                &mut manager,
                &mut board,
                Polyline::from_points(&[
                    Point::Int(corner(820000, 300000)),
                    Point::Int(corner(840000, 300000)),
                    Point::Int(corner(840000, 320000)),
                ]),
                0,
                1000,
                &[94],
                0,
                FixedState::ShoveFixed,
            )
            .expect("3-corner contact trace 2")
            .get(),
        );
        let (mut engine, _ctrl) = build_engine(&mut manager, &mut board, 94, false);
        // The seam itself: a 3-line polyline = 2 corners (Java
        // Polyline.cornerCount = lines.length - 1); a 4-line polyline
        // = 3 corners.
        assert_eq!(
            engine.item_trace_corner_count(c_key),
            2,
            "corner count = lines - 1 (the fixed face)"
        );
        assert_eq!(
            engine.item_trace_corner_count(c4_key),
            3,
            "4-line polyline = 3 corners (boundary +1 face)"
        );
        // The protect arm: factor = max((halfWidth/length)^2 * 20000, 1)
        // = max(0.075^2 * 20000, 1) = 112.5 (Java MazeRipupResolver
        // :59-62). PRE-FIX (the mutation), the seam counted LINES so the
        // 2-corner contact answered 3, the arm did not fire, and this
        // assert read 1.0 — the pin kills it.
        let factor = crate::maze::ripup::calc_fanout_via_ripup_cost_factor(&mut engine, p_key);
        assert_eq!(
            factor, 112.5,
            "2-corner shove-fixed end contact protects (Java factor formula)"
        );
        let factor2 = crate::maze::ripup::calc_fanout_via_ripup_cost_factor(&mut engine, p2_key);
        assert_eq!(
            factor2, 1.0,
            "3-corner shove-fixed end contact does not protect (boundary +1)"
        );
    }
}
