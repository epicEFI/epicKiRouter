//! Java `SortedRoomNeighbours`
//! (`autoroute/expansion/SortedRoomNeighbours.java`, 807 lines) — the
//! ANY_ANGLE room-neighbour sorter plus the dispatch
//! (`complete`/`selectCalculationMode`) the maze engine calls.
//!
//! Dispatch reality (the brief's anchor correction, capture-proven by
//! the spike `dispatch` row): `selectCalculationMode` has THREE arms —
//! ORTHOGONAL (`ShapeSearchTree90Degree` →
//! `SortedOrthogonalRoomNeighbours`, a third class NOT ported in T4 —
//! Tier A is all-45-degree and the corpus has no 90-degree fixture,
//! so the arm is debug-asserted unreachable), DEGREE_45
//! (`ShapeSearchTree45Degree` →
//! [`super::neighbours_forty_five`]) and ANY_ANGLE (the plain
//! `ShapeSearchTree` → THIS module's [`calculate`]). "Any angle" is
//! not a separate sorter file — the base class IS the any-angle arm;
//! there is no third variant beyond these.
//!
//! The engine seam ([`NeighbourEngine`]) is the T4/T6 boundary: the
//! sorters own the ALGORITHM (neighbour walk, edge removal, incomplete
//! room creation, door and target-door creation); the engine owns the
//! STATE (room registry, search tree, id generator, item semantics).
//! Java reaches everything through object references; here every
//! registry access is a trait call keyed by room key. NO maze state
//! (`MazeSearchElement`), NO ripup, NO insertion — the engine itself
//! is T6.
//!
//! Determinism notes:
//! * The overlapping tree entries are pre-sorted by the explicit
//!   `(objectId, shapeIndexInObject)` comparator
//!   (`SortedRoomNeighbours.java:204-213`, the "parity with v1.9"
//!   order). Java `List.sort` is stable, but the comparator is a TOTAL
//!   order on distinct entries — two entries tie on both keys only if
//!   they are the same tree entry — so sort stability is moot and the
//!   result equals a `BTreeSet` order. NOT relied upon silently: the
//!   comparator is extracted ([`compare_tree_entries`]) and pinned by
//!   a unit test — the capture pins are blind to a flipped pre-sort
//!   (the neighbour set re-sorts by geometry; pin-failure mode 9).
//! * The neighbour set is Java `TreeSet<SortedRoomNeighbour>`: ordered
//!   by the counterclockwise `compareTo` AND DE-DUPLICATED on
//!   `compareTo == 0` (`TreeSet.add` drops equals-by-comparator
//!   entries). The [`SortedNeighbours`] Vec below reproduces both
//!   behaviours with a binary-search insert.
//! * The counterclockwise orientation of the base comparator is
//!   geometry-derived (corner distances from the side's start corner,
//!   `:720-762`); an inverted sign mirror-images the walk.

use std::cmp::Ordering;

use epic_board::board::Board;
use epic_board::id::ItemId;
use epic_geometry::line::Line;
use epic_geometry::point::Point;
use epic_geometry::side::Side;
use epic_geometry::simplex::Simplex;
use epic_geometry::tile_shape::TileShape;
use epic_index::SearchTreeVariant;

use super::door::ExpansionDoor;
use super::target_door::TargetItemExpansionDoor;

/// Java `SortedRoomNeighbours.CalculationMode` (`:37-41`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalculationMode {
    /// Java `ORTHOGONAL` — `ShapeSearchTree90Degree`.
    Orthogonal,
    /// Java `DEGREE_45` — `ShapeSearchTree45Degree`.
    Degree45,
    /// Java `ANY_ANGLE` — the plain `ShapeSearchTree`.
    AnyAngle,
}

/// Java `selectCalculationMode` (`:80-88`) — the arm selected for a
/// search-tree variant. The spike `dispatch` row pins all three
/// mappings (plain → ANY_ANGLE, 45-degree → DEGREE_45, 90-degree →
/// ORTHOGONAL); the ORTHOGONAL arm is unreachable on the current
/// corpus by construction (no 90-degree fixture exists).
#[must_use]
pub const fn select_calculation_mode(tree: SearchTreeVariant) -> CalculationMode {
    match tree {
        SearchTreeVariant::NinetyDegree => CalculationMode::Orthogonal,
        SearchTreeVariant::FortyfiveDegree => CalculationMode::Degree45,
        SearchTreeVariant::Generic => CalculationMode::AnyAngle,
    }
}

/// Java `ShapeTree.TreeEntry` — the tree-side half of a neighbour
/// candidate (the object plus the shape index that overlapped).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TreeEntry {
    /// Java `TreeEntry.object` — the `SearchTreeObject` key (D17).
    pub object_key: u64,
    /// Java `TreeEntry.shapeIndexInObject`.
    pub shape_index_in_object: u32,
}

/// The v1.9-parity pre-sort of overlapping tree entries (Java
/// `SortedRoomNeighbours.java:204-213`, base `:104-113`): primary key
/// `objectId` ASCENDING via the WRAPPING i32 delta (Java `int`
/// subtraction), secondary key `shapeIndexInObject` ASCENDING. Shared
/// by the base and 45-degree sorters. Guarded by a unit test, not the
/// capture pins: the neighbour set re-sorts by geometry, so a flipped
/// pre-sort is invisible on the corpus (pin-failure mode 9).
pub(crate) fn compare_tree_entries(
    a: &TreeEntry,
    b: &TreeEntry,
    object_id: impl Fn(u64) -> i32,
) -> Ordering {
    let id_diff = object_id(a.object_key).wrapping_sub(object_id(b.object_key));
    if id_diff != 0 {
        return id_diff.cmp(&0);
    }
    (a.shape_index_in_object as i32).cmp(&(b.shape_index_in_object as i32))
}

/// Registry keys at/above this bound are rooms (items keep their raw
/// numeric item id as key). ONE definition shared by the production
/// engine ([`crate::engine`]) and both replay harnesses
/// (quality-review T17b M-Q1: the const previously lived in THREE
/// copies — engine.rs, drill/pins.rs, expansion/pins.rs — held
/// together only by a test asserting their equality; the room/item
/// key-space boundary is now single-sourced by construction).
pub(crate) const ROOM_KEY_BASE: u64 = 1 << 40;

/// Java's VIRTUAL `Item.isTraceObstacle(int)` face for a tree key —
/// [`Board::item_is_trace_obstacle`] (CA flag + keepout kinds) with
/// the unknown-key verdict `true` the pre-fix net-membership arm
/// produced (`!nets.contains` on an empty net list). The u32 key
/// dispatch is shared by the production engine's shape view
/// ([`crate::engine`]) and both replay harnesses (quality-review
/// T17b M-Q1: three byte-identical copies collapsed into this one).
pub(crate) fn board_item_is_trace_obstacle(board: &Board, key: u64, net_number: i32) -> bool {
    u32::try_from(key)
        .ok()
        .map(ItemId::new)
        .is_none_or(|id| board.item_is_trace_obstacle(id, net_number))
}

/// Java `ShapeSearchTree.overlappingObjects(shape, layer, ignoreNetNos)`
/// `:412-413`: an object is kept only when it is an obstacle w.r.t.
/// EVERY ignore net — the BASE `Item.isObstacle(int)` face
/// (`Item.java:162-164`, overridden by NO item subclass — the
/// trace-obstacle overrides do NOT apply to that walk) for ITEM keys,
/// so a foreign NON-obstacle conduction area STAYS in the results
/// while an own-net object drops out; ROOM keys keep the trait face
/// (Java tree rooms are `CompleteFreeSpaceExpansionRoom`, whose
/// `isObstacle(int)` is unconditionally true, `:77-79`). ONE shared
/// branch for the production walk ([`crate::engine`], consumed by the
/// ripup resolver) and the drill replay harness — quality-review
/// T17b M-Q1: the two hand copies drifted apart once already (the
/// T17b-D fix silently flipped the harness's ITEM keys to the virtual
/// face, spec-review M-2); both impls are now pinned on the
/// PLANE_WALK_DSN crossing cell.
pub(crate) fn ignore_nets_key_is_obstacle<E: NeighbourEngine>(
    engine: &E,
    board: &Board,
    key: u64,
    net: i32,
) -> bool {
    if key >= ROOM_KEY_BASE {
        engine.is_trace_obstacle(key, net)
    } else {
        // The BASE-face twin of the dispatch above — same u32 key
        // unwrap, a DIFFERENT board method (`item_is_obstacle`, not
        // the virtual `item_is_trace_obstacle`).
        u32::try_from(key)
            .ok()
            .map(ItemId::new)
            .is_none_or(|id| board.item_is_obstacle(id, net))
    }
}

/// The engine seam behind the sorters (the
/// `autorouteEngine.*`/room-registry/item reads and writes the Java
/// code reaches through references). The maze engine (T6) implements
/// this; the pin suite implements it over a test world.
pub trait NeighbourEngine {
    // ---- engine context (AutorouteEngine.java) ----
    /// Java `AutorouteEngine.getNetNumber()`.
    fn net_number(&self) -> i32;
    /// Java `AutorouteEngine.generateRoomIdNo()` (`:672-674`) — the
    /// pre-incremented instance counter (first id 1; restarts BURN an
    /// id — the capture's id gaps).
    fn generate_room_id_no(&mut self) -> i32;
    /// Java `board.getBoundingBox().boundingOctagon()` — the infinite
    /// half planes the 45-degree incomplete rooms are cut with.
    fn board_bounding_octagon(&self) -> epic_geometry::int_octagon::IntOctagon;
    /// Java `AutorouteEngine.addIncompleteExpansionRoom`
    /// (`:341-353`) — register + append to the incomplete list;
    /// returns the room key.
    fn add_incomplete_expansion_room(
        &mut self,
        shape: TileShape,
        layer: i32,
        contained_shape: TileShape,
    ) -> u64;
    /// Java `AutorouteEngine.removeAllDoors` (`:603`).
    fn remove_all_doors(&mut self, room_key: u64);
    /// Java `addCompleteRoom` registration half (`:534-555`): create
    /// the `CompleteFreeSpaceExpansionRoom` registry entry (tree
    /// insertion stays engine-side, Java inserts after
    /// `calculateDoors` returns).
    fn add_complete_free_space_room(&mut self, shape: TileShape, layer: i32, id: i32) -> u64;

    // ---- search-tree reads (ShapeSearchTree.java) ----
    /// Java `overlappingTreeEntries(shape, layer, coll)` — the RAW
    /// (unsorted) overlapping entries; the sorters apply the
    /// v1.9-parity order themselves.
    fn overlapping_entries(&mut self, shape: &TileShape, layer: i32) -> Vec<TreeEntry>;
    /// Java `SearchTreeObject.getId()`.
    fn object_id(&self, object_key: u64) -> i32;
    /// Java `SearchTreeObject.isTraceObstacle(netNumber)`.
    fn is_trace_obstacle(&self, object_key: u64, net_number: i32) -> bool;
    /// Java `SearchTreeObject.getTreeShape(tree, shapeIndex)` — `None`
    /// is unreachable for live entries (Java would NPE).
    fn tree_shape(&self, object_key: u64, shape_index: u32) -> Option<TileShape>;
    /// Java `ShapeSearchTree.completeShape(room, netNumber,
    /// ignoreObject, ignoreShape)` — the room shape/layer/contained
    /// triple in, the restrained rooms out.
    fn complete_shape(
        &mut self,
        room_shape: Option<&TileShape>,
        contained: Option<&TileShape>,
        layer: i32,
        ignore_object: Option<u64>,
        ignore_shape: Option<&TileShape>,
    ) -> Vec<epic_index::complete_shape::IncompleteRoom>;

    // ---- object kinds ----
    /// Java `currentObject instanceof ExpansionRoom` — the room key if
    /// the tree object IS an expansion room.
    fn tree_object_room(&self, object_key: u64) -> Option<u64>;
    /// Java `currentObject instanceof Item`.
    fn is_item(&self, object_key: u64) -> bool;
    /// Java `Item.isRoutable()`.
    fn item_is_routable(&self, object_key: u64) -> bool;
    /// Java `currentObject instanceof Connectable`.
    fn item_is_connectable(&self, object_key: u64) -> bool;
    /// Java `Connectable.containsNet(netNumber)`.
    fn item_contains_net(&self, object_key: u64, net_number: i32) -> bool;
    /// Java `Item.sharesNet(other)`.
    fn item_shares_net(&self, first_key: u64, second_key: u64) -> bool;
    /// Java `currentObject instanceof PolylineTrace`.
    fn item_is_polyline_trace(&self, object_key: u64) -> bool;
    /// Java `ItemAutorouteInfo.getExpansionRoom(shapeIndex, tree)`
    /// — get or lazily create the obstacle expansion room of the item
    /// shape; returns its key. A PURE side allocation: the room id is
    /// `(item id << 10) | shapeIndex` and `ObstacleExpansionRoom` is
    /// NOT a `SearchTreeObject`, so nothing is inserted into the tree
    /// — the room is reachable only through the item's autoroute info.
    fn item_expansion_room(&mut self, object_key: u64, shape_index: u32) -> Option<u64>;
    /// Java `Item.getTraceConnectionShape(tree, shapeIndex)`.
    fn trace_connection_shape(&self, object_key: u64, shape_index: u32) -> Option<TileShape>;
    /// Java `insertDoorOk(ObstacleExpansionRoom, Line)`
    /// (`SortedRoomNeighbours.java:375-389`) read side: `None` when
    /// the check falls through to `true` (not a `PolylineTrace`, or
    /// the room is not the trace's first/last section);
    /// `Some(parallel)` is the first/last-section parallelism verdict.
    fn trace_first_or_last_parallel(
        &self,
        item_key: u64,
        index_in_item: u32,
        door_line: &Line,
    ) -> Option<bool>;

    // ---- room registry reads ----
    /// Java `room.getShape()` (live).
    fn room_shape(&self, room_key: u64) -> TileShape;
    /// Java `room.getLayer()`.
    fn room_layer(&self, room_key: u64) -> i32;
    /// Java `room.getId()`.
    fn room_id(&self, room_key: u64) -> i32;
    /// Java `room instanceof IncompleteFreeSpaceExpansionRoom`.
    fn room_is_incomplete(&self, room_key: u64) -> bool;
    /// Java `room instanceof ObstacleExpansionRoom`.
    fn room_is_obstacle(&self, room_key: u64) -> bool;
    /// Java `room instanceof CompleteFreeSpaceExpansionRoom`.
    fn room_is_complete_free_space(&self, room_key: u64) -> bool;
    /// Java `IncompleteFreeSpaceExpansionRoom.getContainedShape()`.
    fn room_contained_shape(&self, room_key: u64) -> Option<TileShape>;
    /// Java `ObstacleExpansionRoom.getItem()`.
    fn room_obstacle_item_key(&self, room_key: u64) -> Option<u64>;
    /// Java `ObstacleExpansionRoom.getIndexInItem()`.
    fn room_obstacle_index_in_item(&self, room_key: u64) -> Option<u32>;
    /// Java `room.doorExists(other)` — `other_room_id` is the other
    /// room's `getId()`; consults THIS room's door list only (the
    /// Java semantics).
    fn room_has_door_to(&self, room_key: u64, other_room_id: i32) -> bool;
    /// Java `room.getDoors()` — a snapshot of THIS room's door list
    /// in append order.
    fn room_doors(&self, room_key: u64) -> Vec<ExpansionDoor>;
    /// The registry key of the room with `getId() == id` (Java
    /// compares references; the engine resolves its id space).
    fn room_key_of_id(&self, id: i32) -> Option<u64>;
    /// The door-driven EXACT endpoint resolution — Java
    /// `door.otherRoom(room)` returns the room OBJECT connected by
    /// this door. The bare id scan of [`Self::room_key_of_id`] is
    /// unfaithful for that when two live incomplete rooms share
    /// `getId()`: the formula `31 * shape.getId() + layer`
    /// (`IncompleteFreeSpaceExpansionRoom.java:38-41`) runs on
    /// content-derived shape ids, so equal-shape same-layer rooms
    /// collide (the room.rs collision note). The endpoint is exactly
    /// the live room whose door list holds THE door — door equality
    /// carries the instance tag (Java's reference identity, door.rs
    /// module doc), and doors attach to BOTH endpoints as clones of
    /// one construction. No graveyard fallback: Java only reaches a
    /// removed room through a door that `removeAllDoors` has already
    /// stripped from the walking room, so a miss mirrors an
    /// unreachable reference.
    fn room_key_of_door(&self, id: i32, door: &ExpansionDoor) -> Option<u64>;

    // ---- room registry writes ----
    /// Java `setShape` + `setContainedShape` on the incomplete
    /// from-room (the `tryRemoveEdge` restart).
    fn set_incomplete_shape(&mut self, room_key: u64, shape: TileShape, contained_shape: TileShape);
    /// Java `CompleteFreeSpaceExpansionRoom.setShape` (the corner
    /// cut-off inside `calculateNewIncompleteRooms`).
    fn set_room_shape(&mut self, room_key: u64, shape: TileShape);
    /// Java `room.addDoor(door)` — append to THIS room's door list.
    fn attach_door(&mut self, room_key: u64, door: ExpansionDoor);
    /// Java `CompleteFreeSpaceExpansionRoom.addTargetDoor` (`:110-113`).
    fn add_target_door(&mut self, room_key: u64, door: TargetItemExpansionDoor);
    /// Java `CompleteFreeSpaceExpansionRoom.setNetDependent` (`:87-88`).
    fn set_net_dependent(&mut self, room_key: u64);

    // ---- T6 seams (the maze engine's completion/dispatch needs) ----
    /// The resolved `SearchTreeVariant` of the engine's autoroute tree
    /// — the dispatch key of [`complete`]. Java carries the concrete
    /// tree; the port needs the variant to pick the 45-degree sorter.
    fn tree_variant(&self) -> epic_index::SearchTreeVariant;
    /// Java `AutorouteEngine.removeIncompleteExpansionRoom`
    /// (`:368`; `:355-367` is `getFirstIncompleteExpansionRoom`) —
    /// drop the room from the registry (the completion flow removes
    /// the consumed incomplete room).
    fn remove_incomplete_room(&mut self, room_key: u64);
    /// Java `addCompleteRoom`'s tail (`:534-535`): the accepted
    /// completed room is INSERTED into the search tree; the restart
    /// attempts of the same [`complete`] call (they burned ids but
    /// never reached the tree) are discarded. `None` discards all.
    fn flush_completed_inserts(&mut self, accepted: Option<u64>);
    /// Java `CompleteExpansionRoom.getTargetDoors()` — the room's
    /// target doors in append order.
    fn room_target_doors(&self, room_key: u64) -> Vec<TargetItemExpansionDoor>;
    /// Java `ObstacleExpansionRoom.allDoorsCalculated()`.
    fn room_obstacle_doors_calculated(&self, room_key: u64) -> bool;
    /// Java `ObstacleExpansionRoom.setDoorsCalculated(true)`.
    fn set_room_doors_calculated(&mut self, room_key: u64);
}

/// The counterclockwise-sorted neighbour set (Java
/// `TreeSet<SortedRoomNeighbour>`): ascending by the comparator,
/// duplicates (comparator `Equal`) DROPPED.
#[derive(Clone, Debug)]
pub(crate) struct SortedNeighbours<T> {
    entries: Vec<T>,
}

impl<T> Default for SortedNeighbours<T> {
    fn default() -> Self {
        SortedNeighbours {
            entries: Vec::new(),
        }
    }
}

impl<T> SortedNeighbours<T> {
    /// Java `TreeSet.add` — insert in comparator order, drop on
    /// `Equal` (returns false like the Java boolean).
    pub(crate) fn add(&mut self, item: T, compare: impl Fn(&T, &T) -> Ordering) -> bool {
        match self.entries.binary_search_by(|probe| compare(probe, &item)) {
            Ok(_) => false,
            Err(pos) => {
                self.entries.insert(pos, item);
                true
            }
        }
    }

    /// Java `isEmpty()`.
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Java `size()`.
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Java `getLast()`.
    pub(crate) fn last(&self) -> &T {
        self.entries
            .last()
            .expect("sorted neighbour set is non-empty")
    }

    /// The ascending iteration order (Java `TreeSet` iterator).
    pub(crate) fn iter(&self) -> impl Iterator<Item = &T> {
        self.entries.iter()
    }

    /// Indexed access for the prev/next walk.
    pub(crate) fn get(&self, index: usize) -> &T {
        &self.entries[index]
    }
}

/// `PolylineShape.equalsCorner` (`polyline_shape.rs` port of
/// `PolylineShape.equalsCorner`) — the corner index equal to `point`,
/// or -1. TileShape does not forward it; the local walk is the exact
/// Java body.
fn equals_corner(shape: &TileShape, point: &Point) -> i32 {
    let count = shape.border_line_count() as i32;
    for i in 0..count {
        if shape.corner(i) == *point {
            return i;
        }
    }
    -1
}

/// `PolylineShape.prevNo` (`PolylineShape.java`) — `(no - 1) mod n`.
fn prev_no(shape: &TileShape, no: i32) -> i32 {
    side_prev_no(no, shape.border_line_count() as i32)
}

/// `PolylineShape.nextNo` — `(no + 1) mod n`.
fn next_no(shape: &TileShape, no: i32) -> i32 {
    side_next_no(no, shape.border_line_count() as i32)
}

/// Count-based `prevNo` (the `Simplex` border walk uses it directly).
fn side_prev_no(no: i32, count: i32) -> i32 {
    (no + count - 1) % count
}

/// Count-based `nextNo`.
fn side_next_no(no: i32, count: i32) -> i32 {
    (no + 1) % count
}

/// `Signum.asInt(double)` (the merged `Side::of`).
fn signum(value: f64) -> i32 {
    match Side::of(value) {
        Side::Positive => 1,
        Side::Collinear => 0,
        Side::Negative => -1,
    }
}

/// Java `SortedRoomNeighbours.complete` (`:65-72`) — the dispatch.
/// Returns the completed room key. The ORTHOGONAL arm
/// (`SortedOrthogonalRoomNeighbours`) is NOT ported in T4: Tier A is
/// verified all-45-degree and no 90-degree fixture exists, so the arm
/// is debug-asserted unreachable and returns `None`.
pub fn complete(
    ctx: &mut impl NeighbourEngine,
    from_room_key: u64,
    tree_variant: SearchTreeVariant,
) -> Option<u64> {
    match select_calculation_mode(tree_variant) {
        CalculationMode::Orthogonal => {
            debug_assert!(
                false,
                "ORTHOGONAL neighbour sorter (SortedOrthogonalRoomNeighbours) is out of T4 scope; \
                 Tier A is all-45-degree"
            );
            None
        }
        CalculationMode::Degree45 => super::neighbours_forty_five::calculate(ctx, from_room_key),
        CalculationMode::AnyAngle => calculate(ctx, from_room_key),
    }
}

/// Java `SortedRoomNeighbours.calculate` (`:95-136`) — the ANY_ANGLE
/// neighbour calculation. Returns the completed room key.
///
/// Java `:102-105` evaluates `generateRoomIdNo()` as an ARGUMENT of
/// `calculateNeighbours` — the id burns at EVERY calculate entry for
/// every room kind (obstacle-room door calculations discard it); each
/// restart recursion re-enters and burns again.
#[must_use]
pub fn calculate(ctx: &mut impl NeighbourEngine, from_room_key: u64) -> Option<u64> {
    let net_number = ctx.net_number();
    let mut room_id_no = ctx.generate_room_id_no();
    let mut room_neighbours = calculate_neighbours(ctx, from_room_key, net_number, room_id_no)?;

    // Check, that each side of the room shape has at least one touching
    // neighbour. Otherwise, improve the room shape by enlarging.
    let mut completed_room = room_neighbours.completed_room;
    let mut edge_removed = room_neighbours.try_remove_edge(ctx);
    while edge_removed {
        // Java :112-115 — removeAllDoors + RECURSIVE calculate (the
        // re-entry burns another room id).
        ctx.remove_all_doors(completed_room);
        room_id_no = ctx.generate_room_id_no();
        room_neighbours = calculate_neighbours(ctx, from_room_key, net_number, room_id_no)?;
        completed_room = room_neighbours.completed_room;
        edge_removed = room_neighbours.try_remove_edge(ctx);
    }

    // Now calculate the new incomplete rooms together with the doors
    // between this room and the sorted neighbours.
    if room_neighbours.sorted_neighbours.is_empty() {
        if ctx.room_is_obstacle(from_room_key) {
            calculate_incomplete_rooms_with_empty_neighbours(ctx, from_room_key);
        }
    } else {
        room_neighbours.calculate_new_incomplete_rooms(ctx);
        // Java :125-129 — the unexpected-dimension trace is
        // diagnostic only.
    }

    if ctx.room_is_complete_free_space(completed_room) {
        calculate_target_doors(ctx, completed_room, &room_neighbours.own_net_objects);
    }
    Some(completed_room)
}

/// Java `calculateIncompleteRoomsWithEmptyNeighbours` (`:138-156`) —
/// an obstacle room with no neighbours gets an incomplete room behind
/// every door-ok border line.
fn calculate_incomplete_rooms_with_empty_neighbours(
    ctx: &mut impl NeighbourEngine,
    from_room_key: u64,
) {
    let room_shape = ctx.room_shape(from_room_key);
    for i in 0..room_shape.border_line_count() as i32 {
        let current_line = room_shape.border_line(i);
        if !insert_door_ok_obstacle(ctx, from_room_key, Some(&current_line)) {
            continue;
        }
        let new_room_shape =
            TileShape::Simplex(Box::new(Simplex::get_instance(&[current_line.opposite()])));
        let new_contained_shape = room_shape.intersection(&new_room_shape);
        let layer = ctx.room_layer(from_room_key);
        let new_room =
            ctx.add_incomplete_expansion_room(new_room_shape, layer, new_contained_shape);
        let door = ExpansionDoor::new(ctx.room_id(from_room_key), ctx.room_id(new_room), 1);
        ctx.attach_door(from_room_key, door.clone());
        ctx.attach_door(new_room, door);
    }
}

/// Java `calculateTargetDoors` (`:158-185`) — the END-of-walk target
/// doors of the base sorter (the 45-degree sorter creates them DURING
/// the walk instead).
fn calculate_target_doors(
    ctx: &mut impl NeighbourEngine,
    completed_room: u64,
    own_net_objects: &[TreeEntry],
) {
    if !own_net_objects.is_empty() {
        ctx.set_net_dependent(completed_room);
    }
    let net_number = ctx.net_number();
    for entry in own_net_objects {
        if !ctx.item_is_connectable(entry.object_key) {
            continue;
        }
        if !ctx.item_contains_net(entry.object_key, net_number) {
            continue;
        }
        let Some(current_connection_shape) =
            ctx.trace_connection_shape(entry.object_key, entry.shape_index_in_object)
        else {
            continue;
        };
        if ctx
            .room_shape(completed_room)
            .intersects(&current_connection_shape)
        {
            // Java `new TargetItemExpansionDoor(item, shapeIndex,
            // room, tree)` — the door SHAPE is the item's TREE shape
            // (not the connection shape) intersected with the room
            // shape.
            let room_id = ctx.room_id(completed_room);
            if let Some(item_shape) = ctx.tree_shape(entry.object_key, entry.shape_index_in_object)
            {
                let target_door = TargetItemExpansionDoor::new(
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
}

/// Java `calculateNeighbours` (`:187-329`) — the touching-neighbour
/// walk with the v1.9-parity pre-sort. `room_id_no` is the id burned
/// by the `calculate` entry (Java `:105` argument evaluation); the
/// incomplete-room arm stamps it on the completed room, the
/// obstacle-room arm discards it.
fn calculate_neighbours(
    ctx: &mut impl NeighbourEngine,
    from_room_key: u64,
    net_number: i32,
    room_id_no: i32,
) -> Option<BaseRoomNeighbours> {
    let room_shape = ctx.room_shape(from_room_key);
    let completed_room = if ctx.room_is_incomplete(from_room_key) {
        ctx.add_complete_free_space_room(
            room_shape.clone(),
            ctx.room_layer(from_room_key),
            room_id_no,
        )
    } else if ctx.room_is_obstacle(from_room_key) {
        // Java: the obstacle room is its own completed room.
        from_room_key
    } else {
        // Java :195-197 — unexpected expansion room type (warn).
        return None;
    };

    let mut result = BaseRoomNeighbours {
        from_room: from_room_key,
        completed_room,
        room_shape: room_shape.clone(),
        sorted_neighbours: SortedNeighbours::default(),
        own_net_objects: Vec::new(),
    };

    let layer = ctx.room_layer(from_room_key);
    let mut overlapping_objects = ctx.overlapping_entries(&room_shape, layer);

    // Sort the overlapping objects deterministically to ensure parity
    // with v1.9 (`:204-213`). Total order on distinct entries — sort
    // stability is moot (documented in the module header).
    overlapping_objects.sort_by(|a, b| compare_tree_entries(a, b, |key| ctx.object_id(key)));

    for current_entry in overlapping_objects {
        if ctx.tree_object_room(current_entry.object_key) == Some(from_room_key) {
            // Java `currentObject == room`.
            continue;
        }
        if ctx.room_is_incomplete(from_room_key)
            && !ctx.is_trace_obstacle(current_entry.object_key, net_number)
        {
            // delay processing the target doors until the room shape
            // will not change anymore
            result.own_net_objects.push(current_entry);
            continue;
        }
        let Some(current_shape) = ctx.tree_shape(
            current_entry.object_key,
            current_entry.shape_index_in_object,
        ) else {
            // Java NPE arm — unreachable for live entries.
            continue;
        };
        let intersection = room_shape.intersection(&current_shape);
        let dimension = intersection.dimension();
        if dimension > 1 {
            if ctx.room_is_obstacle(completed_room)
                && ctx.is_item(current_entry.object_key)
                && ctx.item_is_routable(current_entry.object_key)
                && let Some(current_overlap_room) = ctx.item_expansion_room(
                    current_entry.object_key,
                    current_entry.shape_index_in_object,
                )
            {
                // only Obstacle expansion room may have a 2-dim overlap
                create_overlap_door(ctx, completed_room, current_overlap_room);
            }
            // Java :243-247 — the unexpected-free-space-overlap trace
            // is diagnostic only.
            continue;
        }
        if dimension < 0 {
            // Java :249-252 — debug "dimension >= 0 expected".
            continue;
        }
        if dimension == 1 {
            let touching_sides = room_shape.touching_sides(&current_shape);
            if touching_sides.len() != 2 {
                // Java :255-258 — debug "touchingSides length 2 expected".
                continue;
            }
            result.add_sorted_neighbour(
                ctx,
                current_entry.object_key,
                current_shape,
                intersection.clone(),
                touching_sides[0],
                touching_sides[1],
                false,
                false,
            );
            // make sure, that there is a door to the neighbour room.
            let neighbour_room =
                if let Some(room_key) = ctx.tree_object_room(current_entry.object_key) {
                    Some(room_key)
                } else if ctx.is_item(current_entry.object_key)
                    && ctx.item_is_routable(current_entry.object_key)
                {
                    // expand the item for ripup and pushing purposes
                    ctx.item_expansion_room(
                        current_entry.object_key,
                        current_entry.shape_index_in_object,
                    )
                } else {
                    None
                };
            if let Some(neighbour_room) = neighbour_room
                && insert_door_ok_pair(ctx, completed_room, neighbour_room, &intersection)
            {
                let door =
                    ExpansionDoor::new(ctx.room_id(completed_room), ctx.room_id(neighbour_room), 1);
                ctx.attach_door(completed_room, door.clone());
                ctx.attach_door(neighbour_room, door);
            }
        } else {
            // dimension = 0
            let touching_point = intersection.corner(0);
            let room_corner_no = equals_corner(&room_shape, &touching_point);
            let (room_touch_is_corner, touching_side_no_of_room) = if room_corner_no >= 0 {
                (true, room_corner_no)
            } else {
                let line_no = room_shape.contains_on_border_line_no(&touching_point);
                // Java :298-300 — debug ">= 0 expected"; the -1 value
                // flows through (a sorter entry with a non-border
                // touch sorts by the -1 side).
                (false, line_no)
            };
            let neighbour_room_corner_no = equals_corner(&current_shape, &touching_point);
            let (neighbour_room_touch_is_corner, touching_side_no_of_neighbour_room) =
                if neighbour_room_corner_no >= 0 {
                    // The previous border line is preferred to make
                    // the shape of the incomplete room as big as
                    // possible
                    (true, prev_no(&current_shape, neighbour_room_corner_no))
                } else {
                    let line_no = current_shape.contains_on_border_line_no(&touching_point);
                    (false, line_no)
                };
            result.add_sorted_neighbour(
                ctx,
                current_entry.object_key,
                current_shape,
                intersection,
                touching_side_no_of_room,
                touching_side_no_of_neighbour_room,
                room_touch_is_corner,
                neighbour_room_touch_is_corner,
            );
        }
    }
    Some(result)
}

/// Java `insertDoorOk(room1, room2, doorShape)` (`:332-368`) — the
/// door-shape-dimension-1 gate between two existing rooms.
pub(crate) fn insert_door_ok_pair(
    ctx: &impl NeighbourEngine,
    room1: u64,
    room2: u64,
    door_shape: &TileShape,
) -> bool {
    if ctx.room_has_door_to(room1, ctx.room_id(room2)) {
        return false;
    }
    let obstacle1 = ctx.room_is_obstacle(room1);
    let obstacle2 = ctx.room_is_obstacle(room2);
    if obstacle1 && obstacle2 {
        let first_item = ctx.room_obstacle_item_key(room1);
        let second_item = ctx.room_obstacle_item_key(room2);
        // insert only overlap_doors between items of the same net for
        // performance reasons.
        return match (first_item, second_item) {
            (Some(a), Some(b)) => ctx.item_shares_net(a, b),
            _ => false,
        };
    }
    if !obstacle1 && !obstacle2 {
        return true;
    }
    // Insert 1 dimensional doors of trace rooms only, if they are
    // parallel to the trace line. Otherwise, there may be check ripup
    // problems with entering at the wrong side at a fork.
    let mut door_line: Option<Line> = None;
    let mut prev_corner = door_shape.corner(0);
    let corner_count = door_shape.border_line_count() as i32;
    for i in 1..corner_count {
        let current_corner = door_shape.corner(i);
        if current_corner != prev_corner {
            door_line = Some(door_shape.border_line(i - 1));
            break;
        }
        prev_corner = current_corner;
    }
    if obstacle1 && !insert_door_ok_obstacle(ctx, room1, door_line.as_ref()) {
        return false;
    }
    if obstacle2 {
        return insert_door_ok_obstacle(ctx, room2, door_line.as_ref());
    }
    true
}

/// Java `insertDoorOk(ObstacleExpansionRoom, Line)` (`:375-389`).
fn insert_door_ok_obstacle(
    ctx: &impl NeighbourEngine,
    room: u64,
    door_line: Option<&Line>,
) -> bool {
    let Some(door_line) = door_line else {
        // Java :377-379 — warn "doorLine is null".
        return false;
    };
    let Some(item_key) = ctx.room_obstacle_item_key(room) else {
        return true;
    };
    if !ctx.item_is_polyline_trace(item_key) {
        return true;
    }
    let Some(index_in_item) = ctx.room_obstacle_index_in_item(room) else {
        return true;
    };
    ctx.trace_first_or_last_parallel(item_key, index_in_item, door_line)
        .unwrap_or(true)
}

/// Java `ObstacleExpansionRoom.createOverlapDoor`
/// (`ObstacleExpansionRoom.java:77-106`) — the 2-dim door between two
/// obstacle rooms, ported sorter-side over the context reads (Java
/// puts it on the room class; the state it touches lives engine-side).
pub(crate) fn create_overlap_door(ctx: &mut impl NeighbourEngine, room: u64, other: u64) -> bool {
    if ctx.room_has_door_to(room, ctx.room_id(other)) {
        return false;
    }
    let (Some(item), Some(other_item)) = (
        ctx.room_obstacle_item_key(room),
        ctx.room_obstacle_item_key(other),
    ) else {
        return false;
    };
    if !(ctx.item_is_routable(item) && ctx.item_is_routable(other_item)) {
        return false;
    }
    if !ctx.item_shares_net(item, other_item) {
        return false;
    }
    if item == other_item {
        if !ctx.item_is_polyline_trace(item) {
            return false;
        }
        // create only doors between consecutive trace segments
        let (Some(index), Some(other_index)) = (
            ctx.room_obstacle_index_in_item(room),
            ctx.room_obstacle_index_in_item(other),
        ) else {
            return false;
        };
        if index != other_index + 1 && index != other_index.wrapping_sub(1) {
            return false;
        }
    }
    let door = ExpansionDoor::new(ctx.room_id(room), ctx.room_id(other), 2);
    ctx.attach_door(room, door.clone());
    ctx.attach_door(other, door);
    true
}

/// The base sorter state (Java `SortedRoomNeighbours` fields).
pub(crate) struct BaseRoomNeighbours {
    from_room: u64,
    completed_room: u64,
    room_shape: TileShape,
    sorted_neighbours: SortedNeighbours<BaseSortedRoomNeighbour>,
    own_net_objects: Vec<TreeEntry>,
}

impl BaseRoomNeighbours {
    /// Java `addSortedNeighbour` (`:391-409`) — TreeSet add (dedup on
    /// comparator equality).
    #[allow(clippy::too_many_arguments)]
    fn add_sorted_neighbour(
        &mut self,
        ctx: &impl NeighbourEngine,
        object_key: u64,
        neighbour_shape: TileShape,
        intersection: TileShape,
        touching_side_no_of_room: i32,
        touching_side_no_of_neighbour_room: i32,
        room_touch_is_corner: bool,
        neighbour_room_touch_is_corner: bool,
    ) {
        let neighbour = BaseSortedRoomNeighbour {
            object_id: ctx.object_id(object_key),
            neighbour_shape,
            intersection,
            touching_side_no_of_room,
            touching_side_no_of_neighbour_room,
            room_touch_is_corner,
            neighbour_room_touch_is_corner,
        };
        let room_shape = self.room_shape.clone();
        self.sorted_neighbours
            .add(neighbour, |a, b| a.compare_to(b, &room_shape));
    }

    /// Java `tryRemoveEdge` (`:415-507`).
    fn try_remove_edge(&self, ctx: &mut impl NeighbourEngine) -> bool {
        if !ctx.room_is_incomplete(self.from_room) {
            return false;
        }
        let room_shape = ctx.room_shape(self.from_room);
        let room_simplex = room_shape.to_simplex();
        let room_shape_area = room_shape.area();

        let mut remove_edge_no: i32 = -1;
        let mut prev_edge_no: i32 = -1;
        let mut current_edge_no: i32 = 0;
        for next_neighbour in self.sorted_neighbours.iter() {
            if next_neighbour.touching_side_no_of_room == prev_edge_no {
                continue;
            }
            if next_neighbour.touching_side_no_of_room == current_edge_no {
                prev_edge_no = current_edge_no;
                current_edge_no += 1;
            } else {
                // On the edge side with index currentEdgeNo is no
                // touching neighbour.
                remove_edge_no = current_edge_no;
                break;
            }
        }

        if remove_edge_no < 0 && current_edge_no < room_simplex.border_line_count() as i32 {
            // missing touching neighbour at the last edge side.
            remove_edge_no = current_edge_no;
        }

        if remove_edge_no < 0 {
            return false;
        }
        // Touching neighbour missing at the edge side with index
        // removeEdgeNo. Remove the edge line and restart the
        // algorithm. (Java :448-457 has diagnostic FRLogger traces
        // here, omitted — log-only, D12.)
        let enlarged_shape = TileShape::Simplex(Box::new(
            room_simplex.remove_border_line(remove_edge_no as usize),
        ));
        let Some(contained) = ctx.room_contained_shape(self.from_room) else {
            return false;
        };
        let layer = ctx.room_layer(self.from_room);
        let new_rooms =
            ctx.complete_shape(Some(&enlarged_shape), Some(&contained), layer, None, None);
        if new_rooms.len() != 1 {
            // Java :476-479 — trace "1 completed shape expected".
            return false;
        }
        // Check, that the area increases to prevent endless loop.
        let new_room = &new_rooms[0];
        if new_room.shape.area() > room_shape_area {
            ctx.set_incomplete_shape(
                self.from_room,
                new_room.shape.clone(),
                new_room.contained_shape.clone(),
            );
            return true;
        }
        false
    }

    /// Java `calculateNewIncompleteRooms` (`:510-659`) — the
    /// Simplex-room walk between consecutive sorted neighbours.
    fn calculate_new_incomplete_rooms(&self, ctx: &mut impl NeighbourEngine) {
        let count = self.sorted_neighbours.len();
        let room_simplex = ctx.room_shape(self.from_room).to_simplex();
        for index in 0..count {
            let next_neighbour = self.sorted_neighbours.get(index);
            let prev_neighbour = if index == 0 {
                self.sorted_neighbours.last()
            } else {
                self.sorted_neighbours.get(index - 1)
            };
            let prev_is_last = index == 0; // prev == getLast() iff the single wrap
            let first_touching_side_no = prev_neighbour.touching_side_no_of_room;
            let last_touching_side_no = next_neighbour.touching_side_no_of_room;

            let simplex_line_count = room_simplex.border_line_count() as i32;
            let current_next_no = side_next_no(first_touching_side_no, simplex_line_count);
            let intersection_with_prev_ends_at_corner =
                (first_touching_side_no != last_touching_side_no || prev_is_last)
                    && prev_neighbour.last_corner(&self.room_shape)
                        == room_simplex.corner(current_next_no);
            let intersection_with_next_starts_at_corner =
                (first_touching_side_no != last_touching_side_no || prev_is_last)
                    && next_neighbour.first_corner(&self.room_shape)
                        == room_simplex.corner(last_touching_side_no);

            let mut first_touching_side_no = first_touching_side_no;
            let mut last_touching_side_no = last_touching_side_no;
            if intersection_with_prev_ends_at_corner {
                first_touching_side_no = current_next_no;
            }
            if intersection_with_next_starts_at_corner {
                last_touching_side_no = side_prev_no(last_touching_side_no, simplex_line_count);
            }
            let mut neighbours_touch = false;
            if count > 1 {
                neighbours_touch = prev_neighbour.last_corner(&self.room_shape)
                    == next_neighbour.first_corner(&self.room_shape);
            }

            if neighbours_touch {
                continue;
            }
            // create a door to a new incomplete expansion room between
            // the last corner of the previous neighbour and the first
            // corner of the current neighbour.
            let mut last_bounding_line_no = prev_neighbour.touching_side_no_of_neighbour_room;
            if !(intersection_with_prev_ends_at_corner || prev_neighbour.room_touch_is_corner) {
                last_bounding_line_no =
                    prev_no(&prev_neighbour.neighbour_shape, last_bounding_line_no);
            }

            let mut first_bounding_line_no = next_neighbour.touching_side_no_of_neighbour_room;
            if !(intersection_with_next_starts_at_corner
                || next_neighbour.neighbour_room_touch_is_corner)
            {
                first_bounding_line_no =
                    next_no(&next_neighbour.neighbour_shape, first_bounding_line_no);
            }
            let mut start_edge_line: Option<Line> = Some(
                next_neighbour
                    .neighbour_shape
                    .border_line(first_bounding_line_no)
                    .opposite(),
            );
            // startEdgeLine is only used for the first new incomplete room.
            let mut middle_edge_line: Option<Line> = None;
            let mut current_touching_side_no = last_touching_side_no;
            let mut first_time = true;
            // The loop goes backwards from the edge line of
            // nextNeighbour to the edge line of prevNeighbour.
            loop {
                let mut corner_cut_off = false;
                if ctx.room_is_incomplete(self.from_room)
                    && current_touching_side_no == last_touching_side_no
                    && first_touching_side_no != last_touching_side_no
                {
                    // Create a new line approximately from the last
                    // corner of the previous neighbour to the first
                    // corner of the next neighbour to cut off the
                    // outstanding corners of the room shape in the
                    // empty space. That is only tried in the first
                    // pass of the loop.
                    let cut_line_start = prev_neighbour
                        .last_corner(&self.room_shape)
                        .to_float()
                        .round();
                    let cut_line_end = next_neighbour
                        .first_corner(&self.room_shape)
                        .to_float()
                        .round();
                    let cut_line = Line::new(
                        epic_geometry::point::Point::int(cut_line_start),
                        epic_geometry::point::Point::int(cut_line_end),
                    );
                    let cut_half_plane = TileShape::get_instance(std::slice::from_ref(&cut_line));
                    let completed_shape = ctx.room_shape(self.completed_room);
                    ctx.set_room_shape(
                        self.completed_room,
                        completed_shape.intersection(&cut_half_plane),
                    );
                    // Otherwise room.containedShape would no longer be
                    // contained in the shape after cutting of the corner.
                    if let Some(contained) = ctx.room_contained_shape(self.from_room) {
                        corner_cut_off = contained.side_of_line(&cut_line) == Side::Positive;
                        if corner_cut_off {
                            middle_edge_line = Some(cut_line.opposite());
                        }
                    }
                }
                let next_touching_side_no =
                    side_prev_no(current_touching_side_no, simplex_line_count);

                if !corner_cut_off {
                    middle_edge_line = Some(
                        room_simplex
                            .border_line(current_touching_side_no)
                            .opposite(),
                    );
                }
                let middle_edge_line = middle_edge_line
                    .take()
                    .expect("middle edge line set by both arms");
                let middle_line_dir = middle_edge_line.direction().clone();

                let last_time = current_touching_side_no == first_touching_side_no
                    && !(prev_is_last && first_time)
                    // The expression above handles the case, when all
                    // neighbours are on 1 edge line.
                    || corner_cut_off;

                let mut end_edge_line: Option<Line> = None;
                // endEdgeLine is only used for the last new incomplete room.
                if last_time {
                    let candidate = prev_neighbour
                        .neighbour_shape
                        .border_line(last_bounding_line_no)
                        .opposite();
                    if candidate.direction().side_of(&middle_line_dir) == Side::Positive {
                        end_edge_line = Some(candidate);
                    }
                    // Java :603-607 — the concave-corner (1-point
                    // touch) arm drops the end line.
                }

                if start_edge_line
                    .as_ref()
                    .is_some_and(|sel| middle_line_dir.side_of(sel.direction()) != Side::Positive)
                {
                    // concave corner between the first and the
                    // middle line. May be there is a 1 point touch.
                    start_edge_line = None;
                }
                let mut new_edge_lines: Vec<Line> = Vec::with_capacity(3);
                if let Some(start_edge_line) = start_edge_line {
                    new_edge_lines.push(start_edge_line);
                }
                new_edge_lines.push(middle_edge_line);
                if let Some(end_edge_line) = end_edge_line {
                    new_edge_lines.push(end_edge_line);
                }
                let new_room_shape =
                    TileShape::Simplex(Box::new(Simplex::get_instance(&new_edge_lines)));
                if !new_room_shape.is_empty() {
                    let completed_shape = ctx.room_shape(self.completed_room);
                    let new_contained_shape = completed_shape.intersection(&new_room_shape);
                    if !new_contained_shape.is_empty() {
                        let layer = ctx.room_layer(self.from_room);
                        let new_room = ctx.add_incomplete_expansion_room(
                            new_room_shape,
                            layer,
                            new_contained_shape,
                        );
                        let door = ExpansionDoor::new(
                            ctx.room_id(self.completed_room),
                            ctx.room_id(new_room),
                            1,
                        );
                        ctx.attach_door(self.completed_room, door.clone());
                        ctx.attach_door(new_room, door);
                    }
                }
                if last_time {
                    break;
                }
                current_touching_side_no = next_touching_side_no;
                start_edge_line = None;
                first_time = false;
            }
        }
    }
}

/// The base sorter's neighbour entry (Java inner class
/// `SortedRoomNeighbour`, `:665-806`).
#[derive(Clone, Debug)]
pub(crate) struct BaseSortedRoomNeighbour {
    /// Java `searchTreeObject.getId()` — cached for the comparator.
    object_id: i32,
    /// Java `neighbourShape`.
    neighbour_shape: TileShape,
    /// Java `intersection` (SortedRoomNeighbours.java:676) — stored by
    /// the Java class but never read there; kept for field parity.
    #[allow(dead_code)]
    intersection: TileShape,
    /// Java `touchingSideNoOfRoom`.
    touching_side_no_of_room: i32,
    /// Java `touchingSideNoOfNeighbourRoom`.
    touching_side_no_of_neighbour_room: i32,
    /// Java `roomTouchIsCorner`.
    room_touch_is_corner: bool,
    /// Java `neighbourRoomTouchIsCorner`.
    neighbour_room_touch_is_corner: bool,
}

/// Java `c_dist_tolerance` (`:667`).
const C_DIST_TOLERANCE: f64 = 1.0;

impl BaseSortedRoomNeighbour {
    /// Java `compareTo` (`:720-762`) — counterclockwise around the
    /// room shape in ascending order. `room_shape` is the enclosing
    /// sorter's `completedRoom.getShape()`.
    fn compare_to(&self, other: &Self, room_shape: &TileShape) -> Ordering {
        let compare_value = self
            .touching_side_no_of_room
            .wrapping_sub(other.touching_side_no_of_room);
        if compare_value != 0 {
            return compare_value.cmp(&0);
        }
        let compare_corner = room_shape
            .corner_approx(self.touching_side_no_of_room)
            .expect("a sorter side number has a corner approximation");
        let this_distance = self
            .first_corner(room_shape)
            .to_float()
            .distance(&compare_corner);
        let other_distance = other
            .first_corner(room_shape)
            .to_float()
            .distance(&compare_corner);
        let mut delta_distance = this_distance - other_distance;
        if delta_distance.abs() <= C_DIST_TOLERANCE {
            // check corners for equality
            if self.first_corner(room_shape) == other.first_corner(room_shape) {
                // in this case compare the last corners
                let this_distance2 = self
                    .last_corner(room_shape)
                    .to_float()
                    .distance(&compare_corner);
                let other_distance2 = other
                    .last_corner(room_shape)
                    .to_float()
                    .distance(&compare_corner);
                delta_distance = this_distance2 - other_distance2;
                if delta_distance.abs() <= C_DIST_TOLERANCE
                    && self.neighbour_room_touch_is_corner
                    && other.neighbour_room_touch_is_corner
                {
                    // Otherwise there may be a short 1 dim. touch at a
                    // link between 2 trace lines. In this case equality
                    // is ok, because the 2 intersection pieces with the
                    // expansion room are identical, so that only 1
                    // obstacle is needed.
                    let mut compare_line_no = self.touching_side_no_of_room;
                    if self.room_touch_is_corner {
                        compare_line_no = prev_no(room_shape, compare_line_no);
                    }
                    let compare_dir = room_shape
                        .border_line(compare_line_no)
                        .direction()
                        .opposite();
                    let this_compare_line = self
                        .neighbour_shape
                        .border_line(self.touching_side_no_of_neighbour_room);
                    let other_compare_line = other
                        .neighbour_shape
                        .border_line(other.touching_side_no_of_neighbour_room);
                    delta_distance = f64::from(compare_dir.compare_from(
                        this_compare_line.direction(),
                        other_compare_line.direction(),
                    ));
                }
            }
        }
        let mut res = signum(delta_distance);
        if res == 0 {
            // Deterministic tie-breaker for identical geometry
            res = self.object_id.wrapping_sub(other.object_id).signum();
        }
        res.cmp(&0)
    }

    /// Java `firstCorner()` (`:765-784`) — pure (the Java
    /// precalculation is a cache; the computation is deterministic).
    fn first_corner(&self, room_shape: &TileShape) -> Point {
        if self.room_touch_is_corner {
            room_shape.corner(self.touching_side_no_of_room)
        } else if self.neighbour_room_touch_is_corner {
            self.neighbour_shape
                .corner(self.touching_side_no_of_neighbour_room)
        } else {
            let current_first_corner = self.neighbour_shape.corner(next_no(
                &self.neighbour_shape,
                self.touching_side_no_of_neighbour_room,
            ));
            let prev_line =
                room_shape.border_line(prev_no(room_shape, self.touching_side_no_of_room));
            if prev_line.side_of(&current_first_corner) == Side::Negative {
                current_first_corner
            } else {
                // currentFirstCorner is outside the door shape
                room_shape.corner(self.touching_side_no_of_room)
            }
        }
    }

    /// Java `lastCorner()` (`:787-805`) — pure.
    fn last_corner(&self, room_shape: &TileShape) -> Point {
        if self.room_touch_is_corner {
            room_shape.corner(self.touching_side_no_of_room)
        } else if self.neighbour_room_touch_is_corner {
            self.neighbour_shape
                .corner(self.touching_side_no_of_neighbour_room)
        } else {
            let current_last_corner = self
                .neighbour_shape
                .corner(self.touching_side_no_of_neighbour_room);
            let next_line =
                room_shape.border_line(next_no(room_shape, self.touching_side_no_of_room));
            if next_line.side_of(&current_last_corner) == Side::Negative {
                current_last_corner
            } else {
                // currentLastCorner is outside the door shape
                room_shape.corner(next_no(room_shape, self.touching_side_no_of_room))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(object_key: u64, shape_index_in_object: u32) -> TreeEntry {
        TreeEntry {
            object_key,
            shape_index_in_object,
        }
    }

    /// The pre-sort pin (the ONLY guard of [`compare_tree_entries`]):
    /// primary key objectId ASCENDING via the wrapping i32 delta,
    /// secondary key shapeIndexInObject ASCENDING. The capture pins
    /// are blind to this order — the neighbour set re-sorts by
    /// geometry and the corpus has no comparator-equal entries whose
    /// identity matters (pin-failure mode 9) — so the order is
    /// asserted here directly.
    #[test]
    fn pre_sort_object_id_then_shape_index() {
        let id_of = |key: u64| key as i32;
        let mut entries = [entry(7, 1), entry(3, 9), entry(7, 0), entry(0, 4)];
        entries.sort_by(|a, b| compare_tree_entries(a, b, id_of));
        let order: Vec<(u64, u32)> = entries
            .iter()
            .map(|t| (t.object_key, t.shape_index_in_object))
            .collect();
        assert_eq!(order, [(0, 4), (3, 9), (7, 0), (7, 1)]);

        // The wrapping delta is Java int subtraction: i32::MAX - (-1)
        // wraps to i32::MIN (negative), so the maximal id sorts
        // BEFORE id -1. Pinned on a pair — a full sort over a
        // wrap-spanning id set is not transitive and thus not a
        // well-defined total order.
        let max_id = entry(i32::MAX as u64, 0);
        let neg_one = entry(u64::from(u32::MAX), 0);
        assert_eq!(
            compare_tree_entries(&max_id, &neg_one, id_of),
            Ordering::Less
        );
    }
}
