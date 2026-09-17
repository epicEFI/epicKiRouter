//! M3-T5 drill-subsystem pins — literal replay of the DrillSpike
//! capture (`/tmp/drill_capture_{1,2}.rows`, 479 JSON rows,
//! byte-identical double run of the committed oracle
//! `rust/harness/oracle/DrillSpike.java` over the committed fixture
//! `rust/harness/fixtures/drill-spike/t5_drill_pages.dsn`).
//!
//! The spike protocol: the fixture (4 signal layers, resolution um 10,
//! one F.Cu keepout, two SMD pins, via rule T5_RULE = VT VA VH over
//! padstacks VIA_T/VIA_H) parses in Java; BOTH nets sit in the
//! kicad_default clearance class — the fixture deliberately avoids the
//! empty-auto-via-rule trap (a net whose class differs from every via
//! info's class gets a via rule with ZERO vias and silently never
//! drills; see the bug log). ONE AutorouteEngine per net shares the
//! same completed-room database: the Rust replay is one
//! [`Harness`] replaying the spike's exact phase order —
//! 416-page attach=false enumeration (rooms 1.. burned in page order),
//! fresh-array attach=true battery on the three full pages, via A +
//! ripped run, then the NET_B section reusing the same rooms (the
//! capture's `[42,8,19,26]` reuse proves the shared database).
//!
//! The two SEAM stubs are justified by the capture rows themselves:
//!
//! * the checker stub answers `attach ? DRILLABLE_WITH_ATTACH_SMD :
//!   NOT_DRILLABLE` — jointly FORCED by the A mask battery: A-mask3-
//!   noattach emitting 0 while A-full emits {1,2,3} requires the
//!   layer-0 verdict to set the component-side attach flag (a
//!   plain-Drillable stub emits 1 on A-mask3-noattach), and A-mask01-
//!   attach emitting exactly {1} requires every layer 1..3 to be
//!   non-NOT_DRILLABLE through the attach-allowed rule via;
//! * the distance stub returns 2147483647.0 — the capture's four dist
//!   rows ARE that value (`initConnection(net, null, null)` leaves the
//!   real heuristic targetless), and the emission rows pin
//!   sortingValue 100.0 + 2147483647.0 = 2.147483747E9 exactly.
//!
//! Item ids: the capture's `items` rows pin the parse ids (1 outline,
//! 2 keepout, pins 3/4) and the `insertedVia` rows pin the monotone
//! post-parse allocation (A=5, B=6, B2=7, B3=8) — replayed verbatim
//! (cerebrum mode 8: absolute post-parse ids demand the full
//! insertion sequence).
//!
//! The completion STREAM has its own capture: the companion probe
//! oracle `rust/harness/oracle/DrillSpikeProbe.java`
//! (`/tmp/drill_probe.rows`) isolates single fresh-engine completions
//! — the exact drill-seam call — pinning the first-candidate/recalc
//! structure of [`Harness::complete_expansion_room`] row by row (the
//! main spike cannot host those probes: the autoroute search tree is
//! shared through `board.searchTreeManager`, so a probe before the
//! page walk would shift every room id).

use epic_board::id::ItemId;
use epic_board::items::{FixedState, ItemData};
use epic_board::tree_shapes::item_tree_shapes;
use epic_geometry::float_line::FloatLine;
use epic_geometry::float_point::FloatPoint;
use epic_geometry::int_box::IntBox;
use epic_geometry::int_point::IntPoint;
use epic_geometry::point::Point;
use epic_geometry::regular_tile_shape::RegularTileShape;
use epic_geometry::tile_shape::TileShape;
use epic_index::SearchTreeVariant;
use epic_index::complete_shape::{CompleteShapeObjects, CompleteShapeQuery, complete_shape};
use epic_index::search_tree::SearchTree;

use super::expand_other_layers::{CheckDrillResult, DrillMazeListElement};
use super::{
    DestinationDistance, DrillEngine, DrillPageArray, ExpansionDrill, ViaLayerChecker, ViaRuleVia,
    expand_to_other_layers, max_drill_page_width,
};
use crate::control::{AutorouteControl, RouterSettingsIr, ViaMask};
use crate::expansion::{
    ExpansionDoor, ExpansionRoom, NeighbourEngine, RoomKind, TargetItemExpansionDoor, TreeEntry,
    complete,
};

/// The spike fixture (a committed deliverable of this task).
const FIXTURE_DSN: &str =
    include_str!("../../../../harness/fixtures/drill-spike/t5_drill_pages.dsn");

/// The board box in DB units (capture `meta` row).
const BOARD_BOX: IntBox = IntBox {
    ll: IntPoint::new(-1000, -1000),
    ur: IntPoint::new(1001000, 601000),
};

/// Registry keys at/above this bound are rooms (items keep their raw
/// numeric item id as key).
const ROOM_KEY_BASE: u64 = 1 << 40;

/// The engines' via clearance class (capture `ctrl` row
/// `viaClearanceClass:1`) — the tree the AutorouteEngine builds is
/// compensated for THIS class.
const TREE_CLASS: i32 = 1;

/// The opaque drill key handed to [`expand_to_other_layers`] (Java
/// passes the drill object; the emitted `door_key` is opaque here).
const DRILL_KEY: u64 = 900_000;

/// The layer-change expansion value the spike seeds every run with.
const EXPANSION_VALUE: f64 = 100.0;

/// The capture's dist/sorting distance value (targetless heuristic).
const TARGETLESS_DISTANCE: f64 = 2147483647.0;

// ---- literal row tables (verbatim from the capture rows) ----

/// The 18 pages whose candidate enumeration produced zero drills
/// (`page` rows `drillCount:0`, capture scan order).
const ZERO_PAGES: [(i32, i32); 18] = [
    (0, 1),
    (0, 2),
    (6, 11),
    (6, 12),
    (6, 13),
    (6, 14),
    (7, 11),
    (7, 12),
    (7, 13),
    (7, 14),
    (8, 11),
    (8, 12),
    (8, 13),
    (8, 14),
    (9, 11),
    (9, 12),
    (9, 13),
    (9, 14),
];

/// The full-drill dump pages: keepout page + the two pin pages.
const FULL_PAGES: [(i32, i32); 3] = [(5, 10), (8, 5), (8, 20)];

/// (j, i, box) — the `pageBounds` rows.
const PAGE_BOUNDS: [(i32, i32, [i32; 4]); 3] = [
    (5, 10, [384390, 187125, 422929, 224750]),
    (8, 5, [191695, 300000, 230234, 337625]),
    (8, 20, [769780, 300000, 808319, 337625]),
];

/// (j, i, d, locX, locY, [room ids over layers 0..=3]) — the attach=
/// false `drill` rows.
const DRILLS_ATTACH_FALSE: [(i32, i32, i32, i32, i32, [i32; 4]); 7] = [
    (5, 10, 0, 391570, 205938, [1, 2, 13, 20]),
    (5, 10, 1, 411206, 192938, [27, 2, 13, 20]),
    (5, 10, 2, 399025, 193212, [27, 2, 13, 20]),
    (8, 5, 0, 225742, 318813, [1, 8, 19, 26]),
    (8, 5, 1, 206473, 304375, [1, 8, 19, 26]),
    (8, 20, 0, 774265, 318813, [32, 4, 15, 22]),
    (8, 20, 1, 793535, 304375, [34, 4, 15, 22]),
];

/// The attach=true `drill` rows: the keepout page keeps its three
/// gravity-anchored drills (no pins), page (8,5) collapses to ONE
/// drill anchored at the KA pin center (the pin stops being a cutout
/// and its center is inside NET_A's own-net room 1), and page (8,20)
/// produces ZERO rows — its only anchor would be the KB pin center,
/// which no NET_A room covers (foreign-net pin) and whose completion
/// dies against the pin's own dilated shape (drill rejected).
const DRILLS_ATTACH_TRUE: [(i32, i32, i32, i32, i32, [i32; 4]); 4] = [
    (5, 10, 0, 391570, 205938, [1, 2, 13, 20]),
    (5, 10, 1, 411206, 192938, [27, 2, 13, 20]),
    (5, 10, 2, 399025, 193212, [27, 2, 13, 20]),
    (8, 5, 0, 200000, 330000, [1, 8, 19, 26]),
];

/// The pin drill's `drillId` row: `31*(31*Point.getId(200000,330000)
/// + 0) + 3` with Java wrapping — validates the whole id chain.
const PIN_DRILL_ID: i32 = 1_980_362_707;

/// (x, y, rip, [room ids]) — the `ripDiscovery` rows in spike order.
const RIP_DISCOVERY: [(i32, i32, bool, [i32; 4]); 5] = [
    (300000, 150000, true, [1, 2, 13, 20]),
    (620000, 450000, false, [32, 8, 19, 26]),
    (500000, 450000, true, [42, 8, 19, 26]),
    (520000, 450000, true, [42, 8, 19, 26]),
    (540000, 450000, true, [42, 8, 19, 26]),
];

/// The `insertedVia` rows: the monotone post-parse id allocation.
const VIA_ID_A: u32 = 5;
const VIA_ID_B: u32 = 6;
const VIA_ID_B2: u32 = 7;
const VIA_ID_B3: u32 = 8;

/// (section, nextRoomId, roomRipped) of every emission, plus the
/// shared expansionValue/sortingValue/backtrack/alreadyChecked
/// literals — the `emit` rows of the five emitting runs. Runs absent
/// here emitted NOTHING (A-mask3-noattach, A-mask01-noattach,
/// B-rip-ripup-off, B-rip-wrong-class, B-rip-foreign-padstack).
const EMITS_A_FULL: [(i32, i32, bool); 3] = [(1, 8, false), (2, 19, false), (3, 26, false)];
const EMITS_A_MASK01_ATTACH: [(i32, i32, bool); 1] = [(1, 8, false)];
const EMITS_A_RIP: [(i32, i32, bool); 3] = [(1, 2, true), (2, 13, true), (3, 20, true)];
const EMITS_B_FREE: [(i32, i32, bool); 3] = [(1, 8, false), (2, 19, false), (3, 26, false)];
const EMITS_B_RIP: [(i32, i32, bool); 3] = [(1, 8, true), (2, 19, true), (3, 26, true)];

/// The shared `emit`-row literals: expansionValue 100.0 (zero via
/// costs), sortingValue 100 + 2147483647 (the targetless distance),
/// backtrack section 0, alreadyChecked false.
const EMIT_SORTING_VALUE: f64 = 2_147_483_747.0;

/// (x, y, layer) of the `dist` rows — all values are
/// [`TARGETLESS_DISTANCE`].
const DIST_PROBES: [(f64, f64, i32); 4] = [
    (200000.0, 300000.0, 0),
    (200000.0, 300000.0, 1),
    (200000.0, 300000.0, 3),
    (500000.0, 300000.0, 1),
];

// ---- shape helpers ----

fn pt(x: i32, y: i32) -> Point {
    Point::int(IntPoint::new(x, y))
}

/// The (x, y) of a point (all spike drill locations are IntPoint
/// gravity anchors; `surroundingBox` is their exact degenerate box —
/// the oracle's `(IntPoint) drill.location` cast).
fn point_xy(p: &Point) -> (i32, i32) {
    let ib = p.surrounding_box();
    (ib.ll.x, ib.ll.y)
}

fn box_tile(ll_x: i32, ll_y: i32, ur_x: i32, ur_y: i32) -> TileShape {
    TileShape::RegularTileShape(RegularTileShape::IntBox(IntBox::new(
        IntPoint::new(ll_x, ll_y),
        IntPoint::new(ur_x, ur_y),
    )))
}

fn float_line(ax: i32, ay: i32, bx: i32, by: i32) -> FloatLine {
    FloatLine::new(
        FloatPoint::new(f64::from(ax), f64::from(ay)),
        FloatPoint::new(f64::from(bx), f64::from(by)),
    )
}

/// The A battery's shapeEntry (oracle `entry`).
fn entry_a() -> FloatLine {
    float_line(190000, 295000, 210000, 305000)
}

/// The A-rip run's shapeEntry (oracle `entryA2`).
fn entry_a_rip() -> FloatLine {
    float_line(290000, 145000, 310000, 155000)
}

/// The B runs' shapeEntry (oracle `entryB`).
fn entry_b() -> FloatLine {
    float_line(490000, 445000, 510000, 455000)
}

/// Java `new RouterSettings()` defaults projected to the IR (the
/// oracle constructs the settings-free control; only
/// `viasAllowed:true` reaches a pinned row).
fn default_settings_ir(layer_count: usize) -> RouterSettingsIr {
    RouterSettingsIr {
        trace_costs: Vec::new(),
        via_costs: 1,
        vias_allowed: true,
        bend_costs: vec![0.0; layer_count],
        layer_active: vec![true; layer_count],
        automatic_neckdown: false,
    }
}

// ---- the replay harness ----

/// The replay harness: the parsed fixture board + the class-1
/// autoroute tree holding items AND completed rooms + the room
/// registry ([`NeighbourEngine`] + [`DrillEngine`]).
struct Harness {
    board: epic_board::board::Board,
    /// The manager-resolved tree variant (Generic on this fixture).
    resolved_variant: SearchTreeVariant,
    /// The harness net (1 for the A phase, 2 for B — mirrors the two
    /// engines sharing one room database).
    net: i32,
    tree: SearchTree,
    item_shapes: std::collections::BTreeMap<u64, Vec<Option<TileShape>>>,
    item_layers: std::collections::BTreeMap<(u64, u32), i32>,
    rooms: Vec<ExpansionRoom>,
    keys: Vec<u64>,
    next_key: u64,
    id_counter: i32,
    pending_completed: Vec<u64>,
    obstacle_rooms: std::collections::BTreeMap<(u64, u32), u64>,
}

impl Harness {
    fn build(net: i32) -> Harness {
        let (mut manager, mut board) = crate::test_util::parse(FIXTURE_DSN);
        let tree_index = manager.get_autoroute_tree(&mut board, TREE_CLASS);
        let resolved_variant = manager.trees()[tree_index].variant;
        let mut harness = Harness {
            board,
            resolved_variant,
            net,
            tree: SearchTree::new(resolved_variant, TREE_CLASS),
            item_shapes: std::collections::BTreeMap::new(),
            item_layers: std::collections::BTreeMap::new(),
            rooms: Vec::new(),
            keys: Vec::new(),
            next_key: 0,
            id_counter: 0,
            pending_completed: Vec::new(),
            obstacle_rooms: std::collections::BTreeMap::new(),
        };
        // Freeze the item tree shapes + per-shape layers (the same
        // shapes get_autoroute_tree just inserted into the manager's
        // tree; the per-shape layer is the board's own derivation).
        let ids: Vec<ItemId> = harness.board.iter_descending().map(|e| e.id).collect();
        for id in ids {
            let key = u64::from(id.get());
            let shapes = item_tree_shapes(&mut harness.board, resolved_variant, TREE_CLASS, id);
            harness.tree.insert(key, &shapes);
            for (index, shape) in shapes.iter().enumerate() {
                if shape.is_some() {
                    let layer = harness
                        .board
                        .item_shape_layer(id, index as i32)
                        .unwrap_or_else(|| panic!("item {key} shape {index} carries a layer"));
                    harness.item_layers.insert((key, index as u32), layer);
                }
            }
            harness.item_shapes.insert(key, shapes);
        }
        harness
    }

    /// Java `board.insertVia` projected to the replay: the explicit
    /// item id mirrors the capture's monotone `insertedVia` rows, the
    /// via enters the board AND the harness tree (Java inserts at
    /// `insertVia` through the tree manager), shapes frozen like the
    /// parsed items.
    fn insert_via(
        &mut self,
        id: u32,
        center: (i32, i32),
        padstack_no: i32,
        nets: &[i32],
        clearance_class: i32,
        attach_smd_allowed: bool,
    ) {
        self.board.insert_item(epic_board::board::ItemEntry {
            id: ItemId::new(id),
            data: ItemData::Via {
                center: IntPoint::new(center.0, center.1),
                padstack_no,
                attach_smd_allowed,
            },
            nets: nets.to_vec(),
            clearance_class,
            component_id: 0,
            fixed: FixedState::Unfixed,
            on_the_board: false,
        });
        let key = u64::from(id);
        let shapes = item_tree_shapes(
            &mut self.board,
            self.resolved_variant,
            TREE_CLASS,
            ItemId::new(id),
        );
        self.tree.insert(key, &shapes);
        for (index, shape) in shapes.iter().enumerate() {
            if shape.is_some() {
                let layer = self
                    .board
                    .item_shape_layer(ItemId::new(id), index as i32)
                    .unwrap_or_else(|| panic!("via {id} shape {index} carries a layer"));
                self.item_layers.insert((key, index as u32), layer);
            }
        }
        self.item_shapes.insert(key, shapes);
    }

    /// The 1-based padstack number with this name (the capture
    /// resolves the rule vias by name; numbers are registry order).
    fn padstack_no(&self, name: &str) -> i32 {
        self.board
            .library()
            .padstacks
            .iter()
            .position(|p| p.name == name)
            .map(|index| index as i32 + 1)
            .unwrap_or_else(|| panic!("padstack {name} absent"))
    }

    fn alloc_key(&mut self) -> u64 {
        self.next_key += 1;
        ROOM_KEY_BASE + self.next_key
    }

    fn idx_of(&self, key: u64) -> usize {
        self.keys
            .iter()
            .position(|&k| k == key)
            .unwrap_or_else(|| panic!("room key {key} not registered"))
    }

    fn room(&self, key: u64) -> &ExpansionRoom {
        &self.rooms[self.idx_of(key)]
    }

    fn room_mut(&mut self, key: u64) -> &mut ExpansionRoom {
        let i = self.idx_of(key);
        &mut self.rooms[i]
    }

    fn room_id_of_key(&self, key: u64) -> i32 {
        self.rooms[self.idx_of(key)].id()
    }

    fn room_ids_of(&self, slots: &[Option<u64>]) -> Vec<i32> {
        slots
            .iter()
            .map(|slot| slot.map_or(-1, |key| self.room_id_of_key(key)))
            .collect()
    }

    fn is_room_key(key: u64) -> bool {
        key >= ROOM_KEY_BASE
    }

    fn item_nets(&self, key: u64) -> &[i32] {
        let id = ItemId::new(u32::try_from(key).expect("item key fits u32"));
        self.board.get(id).map(|e| e.nets.as_slice()).unwrap_or(&[])
    }

    fn remove_room(&mut self, key: u64) {
        let i = self.idx_of(key);
        self.rooms.remove(i);
        self.keys.remove(i);
    }

    /// Inserts the SURVIVING completed room of a [`complete`] call and
    /// discards the abandoned restart attempts (they burned their ids
    /// but never reached the tree — the T4 capture's gap semantics).
    fn flush_completed_inserts(&mut self, accepted: Option<u64>) {
        for key in std::mem::take(&mut self.pending_completed) {
            if Some(key) == accepted {
                let shape = self.rooms[self.idx_of(key)].shape().clone();
                self.tree.insert(key, &[Some(shape)]);
            } else {
                self.remove_room(key);
            }
        }
    }
}

impl NeighbourEngine for Harness {
    fn net_number(&self) -> i32 {
        self.net
    }

    fn generate_room_id_no(&mut self) -> i32 {
        self.id_counter += 1;
        self.id_counter
    }

    fn board_bounding_octagon(&self) -> epic_geometry::int_octagon::IntOctagon {
        TileShape::RegularTileShape(RegularTileShape::IntBox(BOARD_BOX))
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
        self.room_mut(room_key).clear_doors();
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
        let Some(leaves) = self.tree.query_candidates(shape, |a, b| a.cmp(&b)) else {
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
        if Self::is_room_key(object_key) {
            self.rooms[self.idx_of(object_key)].id()
        } else {
            i32::try_from(object_key).expect("item keys are u32 ids")
        }
    }

    fn is_trace_obstacle(&self, object_key: u64, net_number: i32) -> bool {
        if Self::is_room_key(object_key) {
            let room = &self.rooms[self.idx_of(object_key)];
            if room.is_complete_free_space() {
                // Java CompleteFreeSpaceExpansionRoom participates in
                // completeShape's restraint/ignore arms.
                return true;
            }
            if let RoomKind::Obstacle { item_key, .. } = room.kind {
                return !self.item_nets(item_key).contains(&net_number);
            }
            return false;
        }
        !self.item_nets(object_key).contains(&net_number)
    }

    fn tree_shape(&self, object_key: u64, shape_index: u32) -> Option<TileShape> {
        if Self::is_room_key(object_key) {
            return Some(self.rooms[self.idx_of(object_key)].shape().clone());
        }
        self.item_shapes
            .get(&object_key)
            .and_then(|shapes| shapes.get(shape_index as usize))
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
            net_number: self.net,
            ignore_object,
            ignore_shape,
        };
        complete_shape(&self.tree, self, &query, &BOARD_BOX)
    }

    fn tree_object_room(&self, object_key: u64) -> Option<u64> {
        Self::is_room_key(object_key).then_some(object_key)
    }

    fn is_item(&self, object_key: u64) -> bool {
        !Self::is_room_key(object_key)
    }

    fn item_is_routable(&self, object_key: u64) -> bool {
        let id = ItemId::new(u32::try_from(object_key).expect("item key fits u32"));
        self.board
            .get(id)
            .map(|e| {
                matches!(
                    e.data,
                    ItemData::Trace { .. } | ItemData::Pin { .. } | ItemData::Via { .. }
                )
            })
            .unwrap_or(false)
    }

    fn item_is_connectable(&self, object_key: u64) -> bool {
        self.item_is_routable(object_key)
    }

    fn item_contains_net(&self, object_key: u64, net_number: i32) -> bool {
        self.item_nets(object_key).contains(&net_number)
    }

    fn item_shares_net(&self, first_key: u64, second_key: u64) -> bool {
        let first = self.item_nets(first_key);
        let second = self.item_nets(second_key);
        first.iter().any(|n| second.contains(n))
    }

    fn item_is_polyline_trace(&self, object_key: u64) -> bool {
        let id = ItemId::new(u32::try_from(object_key).expect("item key fits u32"));
        self.board
            .get(id)
            .map(|e| matches!(e.data, ItemData::Trace { .. }))
            .unwrap_or(false)
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
        self.tree.insert(key, &[Some(shape)]);
        self.obstacle_rooms.insert((object_key, shape_index), key);
        Some(key)
    }

    fn trace_connection_shape(&self, object_key: u64, shape_index: u32) -> Option<TileShape> {
        NeighbourEngine::tree_shape(self, object_key, shape_index)
    }

    fn trace_first_or_last_parallel(
        &self,
        _item_key: u64,
        _index_in_item: u32,
        _door_line: &epic_geometry::line::Line,
    ) -> Option<bool> {
        // The fixture has no PolylineTrace items — the Java check
        // falls through to true (None = fall-through here).
        None
    }

    fn room_shape(&self, room_key: u64) -> TileShape {
        self.rooms[self.idx_of(room_key)].shape().clone()
    }

    fn room_layer(&self, room_key: u64) -> i32 {
        self.rooms[self.idx_of(room_key)].layer()
    }

    fn room_id(&self, room_key: u64) -> i32 {
        self.rooms[self.idx_of(room_key)].id()
    }

    fn room_is_incomplete(&self, room_key: u64) -> bool {
        self.rooms[self.idx_of(room_key)].is_incomplete()
    }

    fn room_is_obstacle(&self, room_key: u64) -> bool {
        self.rooms[self.idx_of(room_key)].is_obstacle()
    }

    fn room_is_complete_free_space(&self, room_key: u64) -> bool {
        self.rooms[self.idx_of(room_key)].is_complete_free_space()
    }

    fn room_contained_shape(&self, room_key: u64) -> Option<TileShape> {
        self.rooms[self.idx_of(room_key)].contained_shape().cloned()
    }

    fn room_obstacle_item_key(&self, room_key: u64) -> Option<u64> {
        match self.rooms[self.idx_of(room_key)].kind {
            RoomKind::Obstacle { item_key, .. } => Some(item_key),
            _ => None,
        }
    }

    fn room_obstacle_index_in_item(&self, room_key: u64) -> Option<u32> {
        match self.rooms[self.idx_of(room_key)].kind {
            RoomKind::Obstacle { index_in_item, .. } => Some(index_in_item),
            _ => None,
        }
    }

    fn room_has_door_to(&self, room_key: u64, other_room_id: i32) -> bool {
        self.rooms[self.idx_of(room_key)].door_exists(other_room_id)
    }

    fn room_doors(&self, room_key: u64) -> Vec<ExpansionDoor> {
        self.rooms[self.idx_of(room_key)].doors().to_vec()
    }

    fn room_key_of_id(&self, id: i32) -> Option<u64> {
        self.rooms
            .iter()
            .zip(&self.keys)
            .find(|(r, _)| r.id() == id)
            .map(|(_, k)| *k)
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
}

impl CompleteShapeObjects for Harness {
    fn is_trace_obstacle(&self, object_key: u64, net_number: i32) -> bool {
        NeighbourEngine::is_trace_obstacle(self, object_key, net_number)
    }

    fn shape_layer(&self, object_key: u64, shape_index: u32) -> i32 {
        if Self::is_room_key(object_key) {
            return self.rooms[self.idx_of(object_key)].layer();
        }
        self.item_layers
            .get(&(object_key, shape_index))
            .copied()
            .expect("live tree shapes carry a layer")
    }

    fn tree_shape(&self, object_key: u64, shape_index: u32) -> Option<TileShape> {
        NeighbourEngine::tree_shape(self, object_key, shape_index)
    }

    fn is_complete_free_space(&self, object_key: u64) -> bool {
        Self::is_room_key(object_key)
            && self.rooms[self.idx_of(object_key)].is_complete_free_space()
    }
}

impl DrillEngine for Harness {
    fn board_bounds(&self) -> IntBox {
        BOARD_BOX
    }

    fn layer_count(&self) -> i32 {
        4
    }

    fn stop_flag(&self) -> Option<&std::sync::atomic::AtomicBool> {
        None
    }

    fn overlapping_items(&self, shape: &TileShape, layer: i32) -> Vec<u64> {
        // Java `board.overlappingItems` — the tree's TreeSet leaf
        // order (object id DESCENDING), items only (rooms are
        // skipped by the callers' instanceof arms; keeping them out
        // here preserves the item-relative order either way).
        let Some(leaves) = self.tree.query_candidates(shape, |a, b| a.cmp(&b)) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for leaf in leaves {
            if Self::is_room_key(leaf.object_key) {
                continue;
            }
            let Some(stored) = self
                .item_shapes
                .get(&leaf.object_key)
                .and_then(|shapes| shapes.get(leaf.shape_index_in_object as usize))
                .cloned()
                .flatten()
            else {
                continue;
            };
            if layer >= 0
                && self
                    .item_layers
                    .get(&(leaf.object_key, leaf.shape_index_in_object))
                    .copied()
                    != Some(layer)
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
        // overrides to `containsNet` (`Trace.java:222`); ConductionArea
        // to `!isObstacle || containsNet` (`ConductionArea.java:403`).
        // Pin and Via do NOT override — never drillable for the
        // cutout test (the attach-SMD relaxation is the separate
        // `pin_drill_allowed` arm).
        let Some(entry) = self.board.get(ItemId::new(
            u32::try_from(item_key).expect("item key fits u32"),
        )) else {
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
        self.board
            .get(ItemId::new(
                u32::try_from(item_key).expect("item key fits u32"),
            ))
            .is_some_and(|e| matches!(e.data, ItemData::Pin { .. }))
    }

    fn pin_drill_allowed(&self, item_key: u64) -> bool {
        // Java `Pin.drillAllowed()` (`Pin.java:348-351`):
        // `firstLayer() == lastLayer()` — a single-layer (SMD) pin.
        let Some(entry) = self.board.get(ItemId::new(
            u32::try_from(item_key).expect("item key fits u32"),
        )) else {
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
        self.board.pin_center(ItemId::new(
            u32::try_from(item_key).expect("item key fits u32"),
        ))
    }

    fn item_is_via(&self, item_key: u64) -> bool {
        self.board
            .get(ItemId::new(
                u32::try_from(item_key).expect("item key fits u32"),
            ))
            .is_some_and(|e| matches!(e.data, ItemData::Via { .. }))
    }

    fn item_padstack_no(&self, item_key: u64) -> Option<i32> {
        self.board
            .get(ItemId::new(
                u32::try_from(item_key).expect("item key fits u32"),
            ))
            .and_then(|e| match &e.data {
                ItemData::Via { padstack_no, .. } => Some(*padstack_no),
                _ => None,
            })
    }

    fn item_clearance_class(&self, item_key: u64) -> i32 {
        self.board
            .get(ItemId::new(
                u32::try_from(item_key).expect("item key fits u32"),
            ))
            .map(|e| e.clearance_class)
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
            .and_then(|p| p.get_shape(usize::try_from(layer).unwrap_or(0)))
            .cloned()
    }

    fn via_rule_vias(&self) -> Vec<ViaRuleVia> {
        let rules = self.board.rules();
        let class_index = rules
            .nets
            .get(self.net)
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

    fn complete_expansion_room(
        &mut self,
        room_shape: Option<&TileShape>,
        contained_shape: &TileShape,
        layer: i32,
    ) -> Vec<u64> {
        // The drill seam always mirrors Java
        // `new IncompleteFreeSpaceExpansionRoom(null, layer, shape)`.
        debug_assert!(
            room_shape.is_none(),
            "the drill completion seam passes the null room shape"
        );
        // Java `completeExpansionRoom` (`AutorouteEngine.java:418-517`):
        // a fresh null-shape room has NO doors, so the
        // (ignoreObject, fromDoorShape) pair is (null, null). The FIRST
        // 2-dim candidate is completed directly
        // (`addCompleteRoom` = calculateDoors + tree insert); every
        // SUBSEQUENT candidate is RE-COMPLETED first
        // (`completeShape(currentIncompleteRoom, ...)` :496-498) —
        // the first room's insertion may have consumed its region
        // (the restraint's contained-point rule drops candidates whose
        // contained point lies beyond every cut line), and only the
        // recalc survivors are completed. Skipping the recalc burns an
        // extra room id per multi-candidate completion (the probe
        // oracle DrillSpikeProbe pins the exact burn counts).
        let variant = self.resolved_variant;
        let cells = self.complete_shape(None, Some(contained_shape), layer, None, None);
        let mut completed = Vec::new();
        let mut cell_keys = Vec::new();
        let mut first_done = false;
        for cell in cells {
            if cell.shape.dimension() != 2 {
                continue;
            }
            // The candidates the recalc walks; the first branch uses a
            // one-element list to share the insert helper.
            let batch: Vec<epic_index::complete_shape::IncompleteRoom> = if !first_done {
                first_done = true;
                vec![cell]
            } else {
                self.complete_shape(
                    Some(&cell.shape),
                    Some(&cell.contained_shape),
                    layer,
                    None,
                    None,
                )
            };
            for tmp in batch {
                let key = self.add_incomplete_expansion_room(
                    tmp.shape.clone(),
                    tmp.layer,
                    tmp.contained_shape.clone(),
                );
                cell_keys.push(key);
                let done_key = complete(self, key, variant);
                if let Some(done_key) = done_key {
                    completed.push(done_key);
                }
                self.flush_completed_inserts(done_key);
            }
        }
        for key in cell_keys {
            self.remove_room(key);
        }
        completed
    }
}

// ---- the SEAM stubs ----

/// One recorded `checkLayer` probe: (layer, clearance class,
/// attachSmdAllowed). The pin tables compare the (layer, attach)
/// columns and the CALL COUNT — the span-filter decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CheckerCall {
    layer: i32,
    clearance_class: i32,
    attach_smd_allowed: bool,
}

/// The checker stub forced by the A mask battery (module doc):
/// attach-allowed probes are DRILLABLE_WITH_ATTACH_SMD, everything
/// else NOT_DRILLABLE.
struct StubChecker {
    calls: Vec<CheckerCall>,
}

impl StubChecker {
    fn new() -> StubChecker {
        StubChecker { calls: Vec::new() }
    }

    fn take_calls(&mut self) -> Vec<CheckerCall> {
        std::mem::take(&mut self.calls)
    }
}

impl ViaLayerChecker for StubChecker {
    fn check_layer(
        &mut self,
        _required_radius: f64,
        clearance_class: i32,
        attach_smd_allowed: bool,
        _room_shape: &TileShape,
        _location: &Point,
        layer: i32,
        _net_number: i32,
    ) -> CheckDrillResult {
        self.calls.push(CheckerCall {
            layer,
            clearance_class,
            attach_smd_allowed,
        });
        if attach_smd_allowed {
            CheckDrillResult::DrillableWithAttachSmd
        } else {
            CheckDrillResult::NotDrillable
        }
    }
}

/// The targetless distance stub — the capture's `dist` rows all read
/// 2147483647.0 (`initConnection(net, null, null)`).
struct TargetlessDistance;

impl DestinationDistance for TargetlessDistance {
    fn calculate(&self, _middle: &FloatPoint, _layer: i32) -> f64 {
        TARGETLESS_DISTANCE
    }
}

/// The expected `checkLayerWithAnyMatchingVia` probe sequence for one
/// free-branch run over the drill span 0..=3: VT (span 0..=3) and VA
/// (span 0..=3, attach) probe on EVERY layer, VH (span 0..=1) only on
/// layers 0..=1 — the rule-via span filter, literal from the
/// `viaRule` capture row.
fn expected_free_calls(via_t: i32, via_h: i32) -> Vec<CheckerCall> {
    let mut calls = Vec::new();
    for layer in 0..=1 {
        calls.push(CheckerCall {
            layer,
            clearance_class: 1,
            attach_smd_allowed: false,
        });
        calls.push(CheckerCall {
            layer,
            clearance_class: 1,
            attach_smd_allowed: true,
        });
        calls.push(CheckerCall {
            layer,
            clearance_class: 1,
            attach_smd_allowed: false,
        });
        let _ = via_t;
        let _ = via_h;
    }
    for layer in 2..=3 {
        calls.push(CheckerCall {
            layer,
            clearance_class: 1,
            attach_smd_allowed: false,
        });
        calls.push(CheckerCall {
            layer,
            clearance_class: 1,
            attach_smd_allowed: true,
        });
    }
    calls
}

// ---- the replay ----

/// One drill dump: (j, i, d, locX, locY, room ids over layers 0..=3).
type DrillDump = (i32, i32, i32, i32, i32, Vec<i32>);

/// The full replay result: everything the pins assert on.
struct Replay {
    h: Harness,
    /// The attach=false enumeration array (all 416 pages walked).
    array: DrillPageArray,
    /// The FRESH attach array (the spike's memo-key quirk: attach is
    /// NOT part of the memo, so the second battery recomputes).
    attach_array: DrillPageArray,
    via_t: i32,
    via_h: i32,
    via_x: i32,
    zero_pages: Vec<(i32, i32)>,
    drills_attach_false: Vec<DrillDump>,
    drills_attach_true: Vec<DrillDump>,
}

/// Builds the harness, asserts the meta/rule literals, and walks the
/// 416-page attach=false enumeration + the fresh-array attach
/// battery — the capture's phase order (room ids 1.. burn in page
/// order; the attach pass reuses the existing rooms).
fn replay_enumeration() -> Replay {
    let mut h = Harness::build(1);

    // meta row.
    assert_eq!(h.board.bounding_box(), Some(BOARD_BOX), "meta: bounds");
    assert_eq!(h.board.layers().layers.len(), 4, "meta: layerCount");

    // items rows: id/kind/netCount verbatim (ids 4,3,2,1 descending).
    let kind_of = |id: u32| {
        let entry = h
            .board
            .get(ItemId::new(id))
            .unwrap_or_else(|| panic!("item {id} exists"));
        (
            match &entry.data {
                ItemData::Pin { .. } => "Pin",
                ItemData::ObstacleArea { .. } => "ObstacleArea",
                ItemData::BoardOutline { .. } => "BoardOutline",
                _ => "other",
            },
            entry.nets.len(),
        )
    };
    let (kind4, nets4) = kind_of(4);
    assert_eq!((kind4, nets4), ("Pin", 1), "items: id 4");
    let (kind3, nets3) = kind_of(3);
    assert_eq!((kind3, nets3), ("Pin", 1), "items: id 3");
    let (kind2, nets2) = kind_of(2);
    assert_eq!((kind2, nets2), ("ObstacleArea", 0), "items: id 2");
    let (kind1, nets1) = kind_of(1);
    assert_eq!((kind1, nets1), ("BoardOutline", 0), "items: id 1");

    // nets row.
    let net_a = crate::test_util::net_no(&h.board, "NET_A");
    let net_b = crate::test_util::net_no(&h.board, "NET_B");
    assert_eq!((net_a, net_b), (1, 2), "nets row");

    // viaRule row: names/spans/classes/attach verbatim; the padstack
    // NUMBERS are registry identities resolved by name.
    let via_t = h.padstack_no("VIA_T");
    let via_h = h.padstack_no("VIA_H");
    let via_x = h.padstack_no("VIA_X");
    let rule = h.via_rule_vias();
    assert_eq!(
        rule,
        vec![
            ViaRuleVia {
                padstack_no: via_t,
                clearance_class: 1,
                attach_smd_allowed: false
            },
            ViaRuleVia {
                padstack_no: via_t,
                clearance_class: 1,
                attach_smd_allowed: true
            },
            ViaRuleVia {
                padstack_no: via_h,
                clearance_class: 1,
                attach_smd_allowed: false
            },
        ],
        "viaRule row: T5_RULE = VT VA VH"
    );
    assert_eq!(
        h.padstack_layer_span(via_t),
        (0, 3),
        "viaRule via0/via1 span"
    );
    assert_eq!(h.padstack_layer_span(via_h), (0, 1), "viaRule via2 span");
    assert_eq!(
        h.padstack_layer_span(via_x),
        (0, 3),
        "VIA_X span (foreign via)"
    );
    assert!(
        rule.iter().all(|via| via.padstack_no != via_x),
        "VIA_X is not in T5_RULE"
    );

    // pageMeta row.
    assert_eq!(
        h.board.default_via_diameter(),
        8000.0,
        "pageMeta: defaultViaDiameter"
    );
    let max_page_width = max_drill_page_width(8000.0);
    assert_eq!(max_page_width, 40000, "pageMeta: maxPageWidth");
    let mut array = DrillPageArray::new(&h, max_page_width);
    assert_eq!(array.column_count, 26, "pageMeta: columnCount");
    assert_eq!(array.row_count, 16, "pageMeta: rowCount");
    assert_eq!(array.page_width, 38539, "pageMeta: pageWidth");
    assert_eq!(array.page_height, 37625, "pageMeta: pageHeight");

    // The attach array is FRESH (the spike's second DrillPageArray).
    let mut attach_array = DrillPageArray::new(&h, max_page_width);

    // pageBounds rows + the attach=false enumeration walk.
    let mut zero_pages = Vec::new();
    let mut drills_attach_false = Vec::new();
    for j in 0..array.row_count {
        for i in 0..array.column_count {
            for &(fj, fi, box4) in &PAGE_BOUNDS {
                if fj == j && fi == i {
                    let shape = &array.pages[j as usize][i as usize].shape;
                    assert_eq!(
                        [shape.ll.x, shape.ll.y, shape.ur.x, shape.ur.y],
                        box4,
                        "pageBounds ({j},{i})"
                    );
                }
            }
            let drills = array.pages[j as usize][i as usize].get_drills(&mut h, net_a, false);
            if drills.is_empty() {
                zero_pages.push((j, i));
            }
            for &(fj, fi) in &FULL_PAGES {
                if fj == j && fi == i {
                    for (d, drill) in drills.iter().enumerate() {
                        let (x, y) = point_xy(&drill.location);
                        drills_attach_false.push((
                            j,
                            i,
                            d as i32,
                            x,
                            y,
                            h.room_ids_of(&drill.room_arr),
                        ));
                    }
                }
            }
        }
    }

    // The attach battery on the FRESH array.
    let mut drills_attach_true = Vec::new();
    for &(fj, fi) in &FULL_PAGES {
        let drills = attach_array.pages[fj as usize][fi as usize].get_drills(&mut h, net_a, true);
        for (d, drill) in drills.iter().enumerate() {
            let (x, y) = point_xy(&drill.location);
            drills_attach_true.push((fj, fi, d as i32, x, y, h.room_ids_of(&drill.room_arr)));
        }
    }

    Replay {
        h,
        array,
        attach_array,
        via_t,
        via_h,
        via_x,
        zero_pages,
        drills_attach_false,
        drills_attach_true,
    }
}

/// A drill over a via location with the PRODUCTION room discovery
/// (oracle `buildRipDrill`): with `rip_item` the layer-0 slot is then
/// overwritten by the obstacle room over the via — the state a ripped
/// via leaves in the maze. Returns (drill, discovered room ids).
fn build_rip_drill(
    h: &mut Harness,
    x: i32,
    y: i32,
    rip_item: Option<(u64, u32)>,
) -> (ExpansionDrill, Vec<i32>) {
    let mut drill = ExpansionDrill::new(
        box_tile(x - 4000, y - 4000, x + 4000, y + 4000),
        pt(x, y),
        0,
        3,
    );
    assert!(
        drill.calculate_expansion_rooms(h),
        "ripDiscovery ({x},{y}): room discovery succeeds"
    );
    let ids = h.room_ids_of(&drill.room_arr);
    if let Some((item_key, shape_index)) = rip_item {
        let room_key = NeighbourEngine::item_expansion_room(h, item_key, shape_index)
            .unwrap_or_else(|| panic!("via item {} carries a tree shape", item_key));
        drill.room_arr[0] = Some(room_key);
    }
    (drill, ids)
}

/// Runs one layer-change expansion and returns the emissions in mask
/// order (Java's TreeSet order for the equal sorting values).
fn run_layer_change(
    h: &Harness,
    ctrl: &AutorouteControl,
    drill: &ExpansionDrill,
    shape_entry: &FloatLine,
    checker: &mut StubChecker,
) -> Vec<DrillMazeListElement> {
    let mut emits = Vec::new();
    expand_to_other_layers(
        h,
        ctrl,
        DRILL_KEY,
        drill,
        0,
        EXPANSION_VALUE,
        shape_entry,
        checker,
        &TargetlessDistance,
        &mut |element| emits.push(element),
    );
    emits
}

/// Asserts the emit rows: (section, nextRoomId, roomRipped) table plus
/// the shared value literals and the door/backtrack key identity. The
/// shapeEntry is carried through unchanged (the capture does not dump
/// it; the Java body never rewrites it).
fn assert_emits(
    emits: &[DrillMazeListElement],
    rows: &[(i32, i32, bool)],
    h: &Harness,
    entry: &FloatLine,
    what: &str,
) {
    assert_eq!(emits.len(), rows.len(), "{what}: emitted count");
    for (k, (element, &(section, next_room_id, room_ripped))) in
        emits.iter().zip(rows.iter()).enumerate()
    {
        assert_eq!(element.door_key, DRILL_KEY, "{what}[{k}]: door key");
        assert_eq!(element.section_no_of_door, section, "{what}[{k}]: section");
        assert_eq!(
            h.room_id_of_key(element.next_room_key),
            next_room_id,
            "{what}[{k}]: nextRoomId"
        );
        assert_eq!(element.room_ripped, room_ripped, "{what}[{k}]: roomRipped");
        assert_eq!(
            element.backtrack_door_key, DRILL_KEY,
            "{what}[{k}]: backtrack door"
        );
        assert_eq!(
            element.section_no_of_backtrack_door, 0,
            "{what}[{k}]: sectionOfBacktrack"
        );
        assert_eq!(
            element.expansion_value, EXPANSION_VALUE,
            "{what}[{k}]: expansionValue"
        );
        assert_eq!(
            element.sorting_value, EMIT_SORTING_VALUE,
            "{what}[{k}]: sortingValue"
        );
        assert!(!element.already_checked, "{what}[{k}]: alreadyChecked");
        assert_eq!(
            &element.shape_entry, entry,
            "{what}[{k}]: shape entry carried"
        );
    }
}

// ---- the pins ----

/// Pin 1 — the grid: pageMeta arithmetic (40000 page width from the
/// 8000.0 default via diameter, 26x16 pages of 38539x37625), the
/// pageBounds rows of the three full pages, and the three
/// overlappingPages probes (whole board -> all 416 pages — the exact
/// 16 x 37625 board height makes this the strict-`<`-bound witness a
/// `<=` mutant panics on; the exact page-corner probe
/// [[1,1],[1,2],[2,1],[2,2]] pinning interior coordinates; and the
/// degenerate zero-height probe -> empty).
#[test]
fn page_grid_and_overlap_probes() {
    let replay = replay_enumeration();

    // Probe 1: the board box overlaps every page (416 = 16 x 26), in
    // scan order.
    let whole = replay
        .array
        .overlapping_pages(&box_tile(-1000, -1000, 1001000, 601000));
    assert_eq!(whole.len(), 416, "probe 1: all pages");
    assert_eq!(whole[0], (0, 0), "probe 1: scan order start");
    assert_eq!(whole[415], (15, 25), "probe 1: scan order end");

    // Probe 2: a box whose right/bottom edges land EXACTLY on the
    // page-2 grid lines ([38539,37625,77078,75250] = [pw,ph,2pw,2ph]).
    // NOTE this probe does NOT discriminate the strict `j < maxJ`
    // bound — its upper bounds (≈2.026) are non-integers where the
    // ceil and the dimension filter coincide anyway; it pins the
    // interior page coordinates. The strict-bound witnesses are
    // probe 1 (board height 602000 = 16 x 37625 EXACTLY, so a `<=`
    // mutant scans row 16 and panics on the missing grid row) and the
    // synthetic `overlapping_pages_strict_upper_bound_at_exact_integer`.
    let corner = replay
        .array
        .overlapping_pages(&box_tile(38539, 37625, 77078, 75250));
    assert_eq!(
        corner,
        vec![(1, 1), (1, 2), (2, 1), (2, 2)],
        "probe 2: exact page corner"
    );

    // Probe 3: a DEGENERATE box ([100000,100000,104000,100000]) —
    // dimension-1, no page overlap anywhere.
    let degenerate = replay
        .array
        .overlapping_pages(&box_tile(100000, 100000, 104000, 100000));
    assert!(degenerate.is_empty(), "probe 3: zero-height probe");
}

/// Pin 2 — the attach=false candidate enumeration: the zero-drill
/// page set equals the 18 capture rows exactly, and the three full
/// pages' drills match the `drill` rows verbatim (locations,
/// first/last layers, room-id quads — keepout page [1,2,13,20] vs pin
/// pages [1,8,19,26]/[32,4,15,22]/[34,4,15,22]).
#[test]
fn candidate_enumeration_zero_set_and_full_pages() {
    let replay = replay_enumeration();
    assert_eq!(
        replay.zero_pages, ZERO_PAGES,
        "the zero-drill page set (capture page rows)"
    );
    let flat: Vec<(i32, i32, i32, i32, i32, Vec<i32>)> = replay
        .drills_attach_false
        .iter()
        .map(|&(j, i, d, x, y, ref rooms)| (j, i, d, x, y, rooms.clone()))
        .collect();
    assert_eq!(
        flat,
        DRILLS_ATTACH_FALSE
            .iter()
            .map(|&(j, i, d, x, y, rooms)| (j, i, d, x, y, rooms.to_vec()))
            .collect::<Vec<_>>(),
        "the attach=false full-page drill rows"
    );
}

/// Pin 3 — the attach-SMD relaxation on the FRESH array (the memo key
/// excludes attach, so the second battery RECOMPUTES): the keepout
/// page keeps its three gravity-anchored drills, page (8,5) collapses
/// to one drill anchored at the KA pin center (200000,330000) with
/// rooms [1,8,19,26] (own-net rooms cover the pin), page (8,20) yields
/// NOTHING (the KB pin center is foreign-net: no room covers it and
/// the completion dies against the pin's own dilated shape), and the
/// pin drill's hash id reproduces the Java `drillId` row.
#[test]
fn attach_relaxation_fresh_array_and_pin_drill_id() {
    let mut replay = replay_enumeration();
    let flat: Vec<(i32, i32, i32, i32, i32, Vec<i32>)> = replay
        .drills_attach_true
        .iter()
        .map(|&(j, i, d, x, y, ref rooms)| (j, i, d, x, y, rooms.clone()))
        .collect();
    assert_eq!(
        flat,
        DRILLS_ATTACH_TRUE
            .iter()
            .map(|&(j, i, d, x, y, rooms)| (j, i, d, x, y, rooms.to_vec()))
            .collect::<Vec<_>>(),
        "the attach=true full-page drill rows"
    );

    let mut h = replay.h;
    let drills = replay.attach_array.pages[8][5].get_drills(&mut h, 1, true);
    let pin_drill = drills
        .iter()
        .find(|drill| point_xy(&drill.location) == (200000, 330000))
        .expect("the pin-center drill exists on page (8,5) attach");
    assert_eq!(pin_drill.get_id(), PIN_DRILL_ID, "drillId row");
}

/// Pin 4 — the NET_A mask battery on the pin drill (capture runs
/// A-full / A-mask3-noattach / A-mask01-noattach / A-mask01-attach)
/// plus the `dist` rows: the default masks emit sections {1,2,3} at
/// nextRoomIds {8,19,26}; the no-attach masks emit NOTHING (the
/// component-side attach flag is set — the stub-inference witness);
/// the attach mask [0,1] emits exactly {1}; every free run probes the
/// rule vias in rule order with the VH span filter (10 calls: 3 on
/// layers 0..=1, 2 on layers 2..=3); the targetless distance rows
/// read 2147483647.0.
#[test]
fn layer_change_mask_battery_net_a() {
    let mut replay = replay_enumeration();
    let mut h = replay.h;
    let net_a = 1;

    // The ctrl row for NET_A.
    let mut ctrl = AutorouteControl::new(&mut h.board, net_a, &default_settings_ir(4));
    assert_eq!(ctrl.net_number, 1, "ctrl: netNumber");
    assert_eq!(ctrl.via_clearance_class, 1, "ctrl: viaClearanceClass");
    assert!(ctrl.attach_smd_allowed, "ctrl: attachSmdAllowed");
    assert!(!ctrl.ripup_allowed, "ctrl: ripupAllowed");
    assert!(ctrl.vias_allowed, "ctrl: viasAllowed");
    assert_eq!(ctrl.via_lower_bound, 0, "ctrl: viaLowerBound");
    assert_eq!(ctrl.via_upper_bound, 4, "ctrl: viaUpperBound");
    assert!(
        ctrl.add_via_costs
            .iter()
            .all(|cost| cost.to_layer.iter().all(|&c| c == 0)),
        "ctrl: addViaCosts all zero"
    );
    assert_eq!(
        ctrl.via_infos,
        vec![
            ViaMask {
                from_layer: 0,
                to_layer: 3,
                attach_smd_allowed: false
            },
            ViaMask {
                from_layer: 0,
                to_layer: 3,
                attach_smd_allowed: true
            },
            ViaMask {
                from_layer: 0,
                to_layer: 1,
                attach_smd_allowed: false
            },
        ],
        "ctrl: viaInfos (T5_RULE masks)"
    );

    let saved_masks = ctrl.via_infos.clone();
    let entry = entry_a();
    let mut checker = StubChecker::new();

    // Locate the pin drill on the attach page (memoized — no
    // recomputation, same drills the enumeration pinned).
    let drills = replay.attach_array.pages[8][5].get_drills(&mut h, net_a, true);
    let pin_drill = drills
        .iter()
        .find(|drill| point_xy(&drill.location) == (200000, 330000))
        .expect("the pin drill exists");

    // A-full.
    let emits = run_layer_change(&h, &ctrl, pin_drill, &entry, &mut checker);
    assert_emits(&emits, &EMITS_A_FULL, &h, &entry, "A-full");
    assert_eq!(
        checker.take_calls(),
        expected_free_calls(replay.via_t, replay.via_h),
        "A-full: checker probe sequence (rule order + VH span filter)"
    );

    // A-mask3-noattach: the [0,3] no-attach mask is dead — the
    // component-side attach flag from the layer-0 WithAttachSmd
    // verdict blocks every to-layer (0 emissions). The attachSmdAllowed
    // toggle mirrors the oracle (the port's DECISION code never reads
    // it — the mask flags are the gate — mirrored for sequence
    // faithfulness).
    ctrl.attach_smd_allowed = false;
    ctrl.via_infos = vec![ViaMask {
        from_layer: 0,
        to_layer: 3,
        attach_smd_allowed: false,
    }];
    let emits = run_layer_change(&h, &ctrl, pin_drill, &entry, &mut checker);
    assert!(emits.is_empty(), "A-mask3-noattach: no emissions");
    assert_eq!(
        checker.take_calls(),
        expected_free_calls(replay.via_t, replay.via_h),
        "A-mask3-noattach: probe sequence unchanged"
    );

    // A-mask01-noattach: same block on the [0,1] mask.
    ctrl.via_infos = vec![ViaMask {
        from_layer: 0,
        to_layer: 1,
        attach_smd_allowed: false,
    }];
    let emits = run_layer_change(&h, &ctrl, pin_drill, &entry, &mut checker);
    assert!(emits.is_empty(), "A-mask01-noattach: no emissions");
    checker.take_calls();

    // A-mask01-attach: exactly the section-1 emission (the contrast
    // witness for the mask truth table).
    ctrl.via_infos = vec![ViaMask {
        from_layer: 0,
        to_layer: 1,
        attach_smd_allowed: true,
    }];
    let emits = run_layer_change(&h, &ctrl, pin_drill, &entry, &mut checker);
    assert_emits(
        &emits,
        &EMITS_A_MASK01_ATTACH,
        &h,
        &entry,
        "A-mask01-attach",
    );
    checker.take_calls();
    ctrl.via_infos = saved_masks;
    ctrl.attach_smd_allowed = true;

    // The dist rows: the targetless heuristic value.
    let distance = TargetlessDistance;
    for (x, y, layer) in DIST_PROBES {
        assert_eq!(
            distance.calculate(&FloatPoint::new(x, y), layer),
            TARGETLESS_DISTANCE,
            "dist ({x},{y},{layer})"
        );
    }
}

/// Pin 5 — the ripped-via battery: A-rip-positive on the NET_A side,
/// then the full NET_B battery (B-free-control, B-rip-positive,
/// B-rip-ripup-off, B-rip-wrong-class, B-rip-foreign-padstack) with
/// the `ripDiscovery` room-id rows, the monotone `insertedVia` ids
/// (5,6,7,8), and the ripDiag gate facts. The ripped branch NEVER
/// probes the checker (zero calls — the free/ripped branch contrast),
/// and the three negative gates each silence the run.
#[test]
fn ripped_via_battery_and_via_insertions() {
    let replay = replay_enumeration();
    let mut h = replay.h;
    let net_a = 1;
    let net_b = 2;
    let mut checker = StubChecker::new();

    // ---- the NET_A ripped run ----
    let mut ctrl_a = AutorouteControl::new(&mut h.board, net_a, &default_settings_ir(4));
    h.insert_via(VIA_ID_A, (300000, 150000), replay.via_t, &[net_a], 1, false);
    let via_entry = h
        .board
        .get(ItemId::new(VIA_ID_A))
        .unwrap_or_else(|| panic!("insertedVia A: item {} exists", VIA_ID_A));
    assert!(
        matches!(via_entry.data, ItemData::Via { .. }),
        "insertedVia A: kind"
    );
    assert_eq!(via_entry.nets, vec![net_a], "insertedVia A: net");
    assert_eq!(via_entry.clearance_class, 1, "insertedVia A: class");

    let (rip_drill_a, rooms_a) =
        build_rip_drill(&mut h, 300000, 150000, Some((u64::from(VIA_ID_A), 0)));
    assert_eq!(rooms_a, RIP_DISCOVERY[0].3.to_vec(), "ripDiscovery A");
    ctrl_a.ripup_allowed = true;
    let entry = entry_a_rip();
    let emits = run_layer_change(&h, &ctrl_a, &rip_drill_a, &entry, &mut checker);
    assert_emits(&emits, &EMITS_A_RIP, &h, &entry, "A-rip-positive");
    assert!(
        checker.take_calls().is_empty(),
        "A-rip-positive: the ripped branch never probes the checker"
    );
    ctrl_a.ripup_allowed = false;

    // ---- the NET_B battery (same room database — the capture's
    // [42,8,19,26] reuse proves the shared tree) ----
    let mut ctrl_b = AutorouteControl::new(&mut h.board, net_b, &default_settings_ir(4));
    assert_eq!(ctrl_b.net_number, 2, "ripDiag: ctrlNet");
    assert_eq!(
        ctrl_b.via_clearance_class, 1,
        "ripDiag: ctrlClass == viaClass"
    );
    assert_eq!(
        ctrl_b.via_rule.as_ref().map(|rule| rule.name.as_str()),
        Some("T5_RULE"),
        "ripDiag: viaRuleName (NOT the empty auto rule)"
    );
    assert!(!ctrl_b.is_fanout, "ripDiag: isFanout");

    h.insert_via(VIA_ID_B, (500000, 450000), replay.via_t, &[net_b], 1, false);

    // Positive control: a NO-rip drill over an EMPTY spot (via B sits
    // at (500000,450000) and would block its own free-branch probes).
    let (free_drill, rooms_free) = build_rip_drill(&mut h, 620000, 450000, None);
    assert_eq!(
        rooms_free,
        RIP_DISCOVERY[1].3.to_vec(),
        "ripDiscovery B-free"
    );
    let (rip_drill, rooms_rip) =
        build_rip_drill(&mut h, 500000, 450000, Some((u64::from(VIA_ID_B), 0)));
    assert_eq!(rooms_rip, RIP_DISCOVERY[2].3.to_vec(), "ripDiscovery B-rip");
    let entry = entry_b();
    ctrl_b.ripup_allowed = true;

    let emits = run_layer_change(&h, &ctrl_b, &free_drill, &entry, &mut checker);
    assert_emits(&emits, &EMITS_B_FREE, &h, &entry, "B-free-control");
    assert_eq!(
        checker.take_calls(),
        expected_free_calls(replay.via_t, replay.via_h),
        "B-free-control: probe sequence (shared class-1 tree)"
    );

    let emits = run_layer_change(&h, &ctrl_b, &rip_drill, &entry, &mut checker);
    assert_emits(&emits, &EMITS_B_RIP, &h, &entry, "B-rip-positive");
    assert!(
        checker.take_calls().is_empty(),
        "B-rip-positive: zero checker probes (padstack span is the via range)"
    );

    // Gate 1: ripupAllowed false -> nothing.
    ctrl_b.ripup_allowed = false;
    let emits = run_layer_change(&h, &ctrl_b, &rip_drill, &entry, &mut checker);
    assert!(emits.is_empty(), "B-rip-ripup-off: no emissions");
    assert!(
        checker.take_calls().is_empty(),
        "B-rip-ripup-off: no probes"
    );
    ctrl_b.ripup_allowed = true;

    // Gate 2: a via whose clearance class (0) differs from
    // ctrl.viaClearanceClass (1) is not a legal own-net via.
    h.insert_via(
        VIA_ID_B2,
        (520000, 450000),
        replay.via_t,
        &[net_b],
        0,
        false,
    );
    let b2_entry = h
        .board
        .get(ItemId::new(VIA_ID_B2))
        .unwrap_or_else(|| panic!("insertedVia B2: item {} exists", VIA_ID_B2));
    assert_eq!(b2_entry.clearance_class, 0, "insertedVia B2: class 0");
    let (rip_drill2, rooms2) =
        build_rip_drill(&mut h, 520000, 450000, Some((u64::from(VIA_ID_B2), 0)));
    assert_eq!(rooms2, RIP_DISCOVERY[3].3.to_vec(), "ripDiscovery B2");
    let emits = run_layer_change(&h, &ctrl_b, &rip_drill2, &entry, &mut checker);
    assert!(emits.is_empty(), "B-rip-wrong-class: no emissions");
    assert!(
        checker.take_calls().is_empty(),
        "B-rip-wrong-class: no probes"
    );

    // Gate 3: a padstack NOT in the via rule (VIA_X) is rejected by
    // the rule-membership arm.
    h.insert_via(
        VIA_ID_B3,
        (540000, 450000),
        replay.via_x,
        &[net_b],
        1,
        false,
    );
    let (rip_drill3, rooms3) =
        build_rip_drill(&mut h, 540000, 450000, Some((u64::from(VIA_ID_B3), 0)));
    assert_eq!(rooms3, RIP_DISCOVERY[4].3.to_vec(), "ripDiscovery B3");
    let emits = run_layer_change(&h, &ctrl_b, &rip_drill3, &entry, &mut checker);
    assert!(emits.is_empty(), "B-rip-foreign-padstack: no emissions");
    assert!(
        checker.take_calls().is_empty(),
        "B-rip-foreign-padstack: no probes"
    );
}

// ---- the completion-stream probe pin ----

/// One probe row of the DrillSpikeProbe capture
/// (`/tmp/drill_probe.rows`, the companion oracle
/// `rust/harness/oracle/DrillSpikeProbe.java`): a fresh-engine
/// single-completion call `completeExpansionRoom(new
/// IncompleteFreeSpaceExpansionRoom(null, layer, pointShape))` — the
/// EXACT drill-seam call — against the items-only shared tree.
/// One completed room of a probe row: `(roomId, [llx, lly, urx, ury])`.
type ProbeRoom = (i32, [i32; 4]);

/// One probe row of the DrillSpikeProbe capture: (what, x, y, layer,
/// burned, incompleteCount, rooms).
type ProbeRow = (&'static str, i32, i32, i32, i32, i32, &'static [ProbeRoom]);

/// (x, y, layer, burned, incompleteCount, [(roomId, box)]) — `burned`
/// is the generateRoomIdNo delta, `incompleteCount` the engine's
/// incomplete-room population after the call (the sorter's
/// incomplete-neighbour seeds persist).
const PROBE_ROWS: [ProbeRow; 12] = [
    (
        "empty",
        10000,
        10000,
        0,
        1,
        2,
        &[(1, [1350, 1350, 398750, 598650])],
    ),
    (
        "empty",
        10000,
        10000,
        1,
        7,
        4,
        &[
            (2, [1350, 1350, 500000, 300000]),
            (4, [500000, 1350, 998650, 549325]),
            (8, [1350, 300000, 723988, 598650]),
        ],
    ),
    (
        "empty",
        10000,
        10000,
        2,
        7,
        6,
        &[
            (9, [1350, 1350, 500000, 300000]),
            (11, [500000, 1350, 998650, 549325]),
            (15, [1350, 300000, 723988, 598650]),
        ],
    ),
    (
        "empty",
        10000,
        10000,
        3,
        7,
        8,
        &[
            (16, [1350, 1350, 500000, 300000]),
            (18, [500000, 1350, 998650, 549325]),
            (22, [1350, 300000, 723988, 598650]),
        ],
    ),
    (
        "below-keepout",
        500000,
        10000,
        0,
        1,
        10,
        &[(23, [398750, 1350, 998650, 198750])],
    ),
    ("below-keepout", 500000, 10000, 1, 0, 10, &[]),
    ("below-keepout", 500000, 10000, 2, 0, 10, &[]),
    ("below-keepout", 500000, 10000, 3, 0, 10, &[]),
    ("covered", 10000, 500000, 0, 0, 10, &[]),
    ("covered", 10000, 500000, 1, 0, 10, &[]),
    ("covered", 10000, 500000, 2, 0, 10, &[]),
    ("covered", 10000, 500000, 3, 0, 10, &[]),
];

/// Pin 6 — the completion stream: the fresh-engine probe rows pin the
/// whole first-candidate/recalc structure of
/// [`Harness::complete_expansion_room`]. The layer-1 row is the
/// load-bearing one: FOUR naive quads enter, but only THREE rooms
/// leave — the fourth candidate's recalc is dropped by the
/// restraint's contained-point rule (its region was consumed by the
/// enlarged third room's octagon), and the burn count 7 (not 4)
/// exposes the sorter's interleaved incomplete-neighbour ids
/// (3, 5, 6, 7). A port that skips the recalc arm completes all four
/// cells directly (burned 8, room id 9 appears) — the mutant this pin
/// kills. The count=0 rows pin the covered-region early-out (0 rooms,
/// 0 burns — no restart, no sorter call).
#[test]
fn completion_probe_first_calls() {
    let mut h = Harness::build(1);
    for (what, x, y, layer, burned, incomplete_count, rooms) in PROBE_ROWS {
        let point_shape = box_tile(x, y, x, y);
        let before = h.id_counter;
        let done = Harness::complete_expansion_room(&mut h, None, &point_shape, layer);
        assert_eq!(
            h.id_counter - before,
            burned,
            "probe ({what}) ({x},{y}) layer {layer}: burned id count"
        );
        let got: Vec<(i32, [i32; 4])> = done
            .iter()
            .map(|&key| {
                let b = h.room(key).shape().bounding_box();
                (h.room(key).id(), [b.ll.x, b.ll.y, b.ur.x, b.ur.y])
            })
            .collect();
        assert_eq!(
            got, rooms,
            "probe ({what}) ({x},{y}) layer {layer}: completed rooms"
        );
        // Java's `incompleteCount` is the engine's CUMULATIVE
        // incomplete-room population (the sorter's neighbour seeds
        // persist); the harness starts empty, so the registry count is
        // directly comparable.
        let incomplete_total = h.rooms.iter().filter(|r| r.is_incomplete()).count();
        assert_eq!(
            i32::try_from(incomplete_total).unwrap_or(-1),
            incomplete_count,
            "probe ({what}) ({x},{y}) layer {layer}: cumulative incomplete rooms"
        );
    }
}
