//! Java `TargetItemExpansionDoor`
//! (`autoroute/expansion/TargetItemExpansionDoor.java`, 75 lines) — an
//! expansion door leading to a start or destination item of the
//! autoroute algorithm (item tree shape ∩ room shape).
//!
//! The Java constructor reads `item.getTreeShape(searchTree,
//! treeEntryNo)` and `room.getShape()` through its references; the
//! Rust constructor takes the two shapes directly (D17: the caller
//! resolves them, the capture rows re-insert the same tree shapes).
//! The room is kept as its `getId()` value (`None` is Java's null
//! room, which forces the `Simplex.EMPTY` shape, `:25-29`).

use epic_geometry::simplex::Simplex;
use epic_geometry::tile_shape::TileShape;

/// An expansion door to a target item (Java
/// `TargetItemExpansionDoor`).
#[derive(Clone, Debug, PartialEq)]
pub struct TargetItemExpansionDoor {
    /// Java `item` — the owning item key (D17).
    pub item_key: u64,
    /// Java `treeEntryNo` — the item shape index the door leads to.
    pub tree_entry_no: u32,
    /// Java `room.getId()`; `None` is Java's null room.
    pub room_id: Option<i32>,
    shape: TileShape,
}

impl TargetItemExpansionDoor {
    /// Java `new TargetItemExpansionDoor(item, treeEntryNo, room,
    /// searchTree)` (`:20-32`) — `item_shape` is Java's
    /// `item.getTreeShape(searchTree, treeEntryNo)`, `room_shape` the
    /// room's shape (`None` is the null-room arm,
    /// `shape = Simplex.EMPTY`).
    pub fn new(
        item_key: u64,
        tree_entry_no: u32,
        room_id: Option<i32>,
        item_shape: &TileShape,
        room_shape: Option<&TileShape>,
    ) -> Self {
        let shape = match room_shape {
            None => TileShape::Simplex(Box::new(Simplex::empty())),
            Some(room_shape) => item_shape.intersection(room_shape),
        };
        TargetItemExpansionDoor {
            item_key,
            tree_entry_no,
            room_id,
            shape,
        }
    }

    /// Java `getShape()` (`:34-37`).
    #[must_use]
    pub fn shape(&self) -> &TileShape {
        &self.shape
    }

    /// Java `getDimension()` (`:39-42`) — target doors are always
    /// 2-dimensional.
    #[must_use]
    pub const fn dimension() -> i32 {
        2
    }

    /// Java `getId()` (`:70-74`) — `31 * item.getId() + room.getId()`
    /// (0 for the null room). The item key's low 4 bytes stand in for
    /// Java's int `Item.getId()` (the same convention as the obstacle
    /// room ids in `super::room`).
    #[must_use]
    pub fn id(&self) -> i32 {
        let item_id = i32::from_le_bytes(
            self.item_key.to_le_bytes()[0..4]
                .try_into()
                .expect("4 bytes"),
        );
        item_id
            .wrapping_mul(31)
            .wrapping_add(self.room_id.unwrap_or(0))
    }
}
