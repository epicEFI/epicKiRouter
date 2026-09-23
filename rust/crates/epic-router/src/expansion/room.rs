//! Java `autoroute/expansion` room types: the `ExpansionRoom` interface
//! (`ExpansionRoom.java`), the `FreeSpaceExpansionRoom` base
//! (`FreeSpaceExpansionRoom.java`), `IncompleteFreeSpaceExpansionRoom`
//! (`IncompleteFreeSpaceExpansionRoom.java`, 42 lines),
//! `CompleteFreeSpaceExpansionRoom`
//! (`CompleteFreeSpaceExpansionRoom.java`, 210 lines) and
//! `ObstacleExpansionRoom` (`ObstacleExpansionRoom.java`, 159 lines).
//!
//! Java keeps the room kinds apart by inheritance and cross-links them
//! through door lists holding OBJECT REFERENCES. Rust value semantics
//! break reference identity, so doors store the endpoint rooms' `getId()`
//! values instead (`room_id` handles; the `getId` formulas below are the
//! exact Java bodies). The three kinds collapse into one struct with a
//! [`RoomKind`] discriminator — the maze engine (T6) will own the room
//! registry this implies.
//!
//! Id semantics (all capture-proven, M3-T4 spike `logs/M3-T4/
//! spike_run1.jsonl`):
//! * incomplete rooms: `31 * shape.getId() + layer`
//!   (`IncompleteFreeSpaceExpansionRoom.java:38-41`). Java int overflow
//!   WRAPS — the arithmetic here is `wrapping_*` on purpose. Capture
//!   examples: `-333950178` (E1), `2058449700` (F1). Collisions are
//!   possible in Java and remain possible here (documented divergence
//!   from door identity, see `door_exists`).
//! * completed free-space rooms: the engine-sequential
//!   `generateRoomIdNo()` value (`CompleteFreeSpaceExpansionRoom.java:100-102`;
//!   engine `AutorouteEngine.java:672-674`). Capture: 1, 2, 4 (E) and
//!   3, 6 (F) — the gaps pin the tryRemoveEdge RESTART allocations.
//! * obstacle rooms: `(item.getId() << 10) | indexInItem`
//!   (`ObstacleExpansionRoom.java:49-51`).

use epic_geometry::tile_shape::TileShape;

use super::door::ExpansionDoor;
use super::target_door::TargetItemExpansionDoor;

/// Which Java subclass this room mirrors.
#[derive(Clone, Debug, PartialEq)]
pub enum RoomKind {
    /// `IncompleteFreeSpaceExpansionRoom` — the shape is not yet
    /// maximal; carries the shape the completed room must contain.
    Incomplete {
        /// Java `getContainedShape()` / `setContainedShape`.
        contained_shape: TileShape,
    },
    /// `CompleteFreeSpaceExpansionRoom` — engine-sequential id.
    CompleteFreeSpace {
        /// Java `getId()` (`:100-102`).
        id: i32,
    },
    /// `ObstacleExpansionRoom` — one tile shape of a board item.
    Obstacle {
        /// The owning item (Java `Item.getId()` key; D17: an opaque key
        /// here, the engine resolves it).
        item_key: u64,
        /// Java `getIndexInItem()`.
        index_in_item: u32,
    },
}

/// A room of the maze expansion graph (`FreeSpaceExpansionRoom` +
/// `ObstacleExpansionRoom` merged).
#[derive(Clone, Debug)]
pub struct ExpansionRoom {
    /// The kind discriminator.
    pub kind: RoomKind,
    shape: TileShape,
    layer: i32,
    /// Java `FreeSpaceExpansionRoom.doors` / `ObstacleExpansionRoom.doors`
    /// — append order is the sorter's insertion order (capture-pinned
    /// `door_index` rows).
    doors: Vec<ExpansionDoor>,
    /// Java `CompleteFreeSpaceExpansionRoom.targetDoors`; the other
    /// kinds keep it empty (`IncompleteFreeSpaceExpansionRoom.getTargetDoors`
    /// returns a fresh empty list, `ObstacleExpansionRoom` likewise).
    target_doors: Vec<TargetItemExpansionDoor>,
    /// Java `roomIsNetDependent`
    /// (`CompleteFreeSpaceExpansionRoom.java:31,87-97`).
    net_dependent: bool,
}

impl ExpansionRoom {
    /// Java `new IncompleteFreeSpaceExpansionRoom(shape, layer,
    /// containedShape)`.
    pub fn new_incomplete(shape: TileShape, layer: i32, contained_shape: TileShape) -> Self {
        ExpansionRoom {
            kind: RoomKind::Incomplete { contained_shape },
            shape,
            layer,
            doors: Vec::new(),
            target_doors: Vec::new(),
            net_dependent: false,
        }
    }

    /// Java `new CompleteFreeSpaceExpansionRoom(shape, layer, id)`.
    pub fn new_complete_free_space(shape: TileShape, layer: i32, id: i32) -> Self {
        ExpansionRoom {
            kind: RoomKind::CompleteFreeSpace { id },
            shape,
            layer,
            doors: Vec::new(),
            target_doors: Vec::new(),
            net_dependent: false,
        }
    }

    /// Java `new ObstacleExpansionRoom(item, indexInItem, shapeTree)` —
    /// the shape arrives caller-provided (Java reads
    /// `item.getTreeShape(shapeTree, indexInItem)`; D17).
    pub fn new_obstacle(item_key: u64, index_in_item: u32, shape: TileShape, layer: i32) -> Self {
        ExpansionRoom {
            kind: RoomKind::Obstacle {
                item_key,
                index_in_item,
            },
            shape,
            layer,
            doors: Vec::new(),
            target_doors: Vec::new(),
            net_dependent: false,
        }
    }

    /// Java `getId()` per kind (see the module doc for the formulas and
    /// the wrapping-arithmetic requirement).
    #[must_use]
    pub fn id(&self) -> i32 {
        match &self.kind {
            // IncompleteFreeSpaceExpansionRoom.java:38-41.
            RoomKind::Incomplete { .. } => 31i32
                .wrapping_mul(self.shape.get_id())
                .wrapping_add(self.layer),
            RoomKind::CompleteFreeSpace { id } => *id,
            // ObstacleExpansionRoom.java:49-51.
            RoomKind::Obstacle {
                item_key,
                index_in_item,
            } => {
                (i32::from_le_bytes(item_key.to_le_bytes()[0..4].try_into().expect("4 bytes"))
                    << 10)
                    | *index_in_item as i32
            }
        }
    }

    /// Java `getShape()` / `setShape` (the setter is used by
    /// tryRemoveEdge / tryRemoveEdgeLine on the incomplete from-room).
    #[must_use]
    pub fn shape(&self) -> &TileShape {
        &self.shape
    }

    /// Java `FreeSpaceExpansionRoom.setShape` (`:70-72`).
    pub fn set_shape(&mut self, shape: TileShape) {
        self.shape = shape;
    }

    /// Java `getLayer()`; for obstacle rooms Java reads
    /// `item.shapeLayer(indexInItem)` — the engine resolves that to the
    /// same value the room was constructed with.
    #[must_use]
    pub fn layer(&self) -> i32 {
        self.layer
    }

    /// Java `IncompleteFreeSpaceExpansionRoom.getContainedShape()`;
    /// `None` for the other kinds.
    #[must_use]
    pub fn contained_shape(&self) -> Option<&TileShape> {
        match &self.kind {
            RoomKind::Incomplete { contained_shape } => Some(contained_shape),
            _ => None,
        }
    }

    /// Java `setContainedShape` (incomplete rooms only; a no-op
    /// otherwise — Java would not call it there).
    pub fn set_contained_shape(&mut self, shape: TileShape) {
        if let RoomKind::Incomplete { contained_shape } = &mut self.kind {
            *contained_shape = shape;
        }
    }

    /// True for [`RoomKind::CompleteFreeSpace`] (Java `instanceof
    /// CompleteFreeSpaceExpansionRoom`).
    #[must_use]
    pub fn is_complete_free_space(&self) -> bool {
        matches!(self.kind, RoomKind::CompleteFreeSpace { .. })
    }

    /// True for [`RoomKind::Obstacle`] (Java `instanceof
    /// ObstacleExpansionRoom`).
    #[must_use]
    pub fn is_obstacle(&self) -> bool {
        matches!(self.kind, RoomKind::Obstacle { .. })
    }

    /// True for [`RoomKind::Incomplete`] (Java `instanceof
    /// IncompleteFreeSpaceExpansionRoom`).
    #[must_use]
    pub fn is_incomplete(&self) -> bool {
        matches!(self.kind, RoomKind::Incomplete { .. })
    }

    /// Java `getDoors()` — append order preserved.
    #[must_use]
    pub fn doors(&self) -> &[ExpansionDoor] {
        &self.doors
    }

    /// Java `addDoor`.
    pub fn add_door(&mut self, door: ExpansionDoor) {
        self.doors.push(door);
    }

    /// Java `removeDoor` (`ExpansionRoom.removeDoor`) — the FIRST
    /// equal door instance leaves the list. Java `List.remove` is
    /// reference identity and the port matches it directly:
    /// `ExpansionDoor`'s `PartialEq` includes the instance tag
    /// (`expansion/door.rs`, module doc "REFERENCE IDENTITY"), so no
    /// doors-between-one-pair uniqueness assumption is load-bearing
    /// here.
    pub fn remove_door(&mut self, door: &ExpansionDoor) {
        if let Some(index) = self.doors.iter().position(|d| d == door) {
            self.doors.remove(index);
        }
    }

    /// Java `clearDoors` — a fresh list replaces the old one.
    pub fn clear_doors(&mut self) {
        self.doors = Vec::new();
        // CompleteFreeSpaceExpansionRoom.clearDoors (:197-201) also
        // resets the target doors.
        self.target_doors = Vec::new();
    }

    /// Java `doorExists(other)` (`FreeSpaceExpansionRoom.java:81-91`)
    /// — reference identity there, room-id comparison here. The
    /// incomplete-room id formula can COLLIDE (two rooms of equal shape
    /// id and layer); Java's identity check would distinguish them.
    ///
    /// REVISIT STATUS: FIRED. The earlier premise "no capture flow
    /// creates two same-id rooms inside one sorter run" is DISPROVEN —
    /// the T6 events capture holds id 98239 twice live (see
    /// `NeighbourCtx::room_key_of_id` in `expansion/pins.rs`). What the
    /// collision broke first was door-VALUE equality (wrong-twin
    /// removal/membership/endpoint resolution — the t7 phantom class),
    /// and THAT is resolved: the door instance tag restores Java's
    /// object identity at every equality/membership site
    /// (`expansion/door.rs`, module doc "REFERENCE IDENTITY"). What
    /// remains here is the separate id-proxy face: a same-id twin can
    /// only make this answer TRUE where Java's identity check answers
    /// FALSE — every door incident to THIS room carries this room's own
    /// id as an endpoint, so a query whose target shares that id trips
    /// the proxy and `room_has_door_to` skips a door Java would create
    /// (its two production callers: `insert_door_ok_pair` /
    /// `create_overlap_door` in `expansion/neighbours.rs`). The T6
    /// closes pin the shipped flows against that path (events compare
    /// exit 0, 3520 rows byte-identical). A new caller must not trust
    /// this proxy — use the door-holding form (`room_key_of_door`)
    /// instead.
    #[must_use]
    pub fn door_exists(&self, other_id: i32) -> bool {
        self.doors
            .iter()
            .any(|door| door.first_room_id == other_id || door.second_room_id == other_id)
    }

    /// Java `getTargetDoors()` — empty for incomplete and obstacle
    /// rooms.
    #[must_use]
    pub fn target_doors(&self) -> &[TargetItemExpansionDoor] {
        &self.target_doors
    }

    /// Java `CompleteFreeSpaceExpansionRoom.addTargetDoor` (`:110-113`).
    pub fn add_target_door(&mut self, door: TargetItemExpansionDoor) {
        self.target_doors.push(door);
    }

    /// Java `setNetDependent` / `isNetDependent`
    /// (`CompleteFreeSpaceExpansionRoom.java:87-97`).
    pub fn set_net_dependent(&mut self, value: bool) {
        self.net_dependent = value;
    }

    /// Java `isNetDependent()`.
    #[must_use]
    pub fn is_net_dependent(&self) -> bool {
        self.net_dependent
    }
}
