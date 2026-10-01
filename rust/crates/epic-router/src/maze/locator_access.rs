//! The T9 locator seam (Java: the `FoundConnectionLocator` reaches the
//! maze-search state through the door/drill object graph).
//!
//! Java's `FoundConnectionLocator.backtrack` starts at
//! `mazeSearchResult.destinationDoor` and walks
//! `MazeSearchElement.backtrackDoor` REFERENCES backwards; the door and
//! drill objects carry their section arrays (`ExpansionDoor.sectionArr`,
//! `ExpansionDrill.mazeSearchElements`), the drill carries
//! `location`/`firstLayer`/`roomArr`, and the room shapes are read LIVE
//! through the door endpoint references. The Rust port separates the
//! same reads into this trait:
//!
//! * [`LocatorAccess::Ctx`] + [`LocatorAccess::engine`] — the
//!   room/item read seam ([`NeighbourEngine`], the trait T4/T6 pin
//!   harnesses already implement). Java reaches the rooms through
//!   object references; here the locator reads them through the
//!   engine's room registry, keyed as everywhere else. An associated
//!   type instead of a supertrait: a `LocatorAccess: NeighbourEngine`
//!   bound would force a 45-method delegation impl on
//!   [`MazeSearchEngine`] (whose `ctx` already IS the engine) for
//!   reads the locator never needs to own.
//! * [`LocatorAccess::resolve_backtrack_door`] — the key → object
//!   resolution Java gets for free from reference identity (the
//!   engine's backtrack registry, filled per popped element).
//! * [`LocatorAccess::maze_search_element`] /
//!   [`LocatorAccess::maze_search_element_count`] — Java
//!   `getMazeSearchElement(int)` / `mazeSearchElementCount()` on the
//!   expandable object, dispatched per door kind inside the engine.
//! * [`LocatorAccess::drill_info`] — the `ExpansionDrill` reads
//!   (`location`, `firstLayer`, `roomArr`); `None` for non-drills
//!   (Java's `instanceof ExpansionDrill` guards).
//! * [`LocatorAccess::expandable_object_shape`] — Java
//!   `ExpandableObject.getShape()`: the LIVE endpoint-room
//!   intersection for room doors, the construction-time shape for
//!   target doors (`TargetItemExpansionDoor.java:34-37` returns the
//!   stored field), the page/drill shape for the drill kinds.

use epic_geometry::point::Point;
use epic_geometry::tile_shape::TileShape;

use crate::drill::MazeSearchElement;
use crate::expansion::NeighbourEngine;

use super::list_element::ExpandableObject;
use super::search_engine::MazeSearchEngine;

/// The `ExpansionDrill` reads the locator needs (Java
/// `drill.location` / `drill.firstLayer` / `drill.roomArr`).
/// `room_arr` entries are the per-layer completed room KEYS (`None` =
/// a Java null slot). The drill SHAPE is deliberately NOT carried in
/// this bundle — the locator reads it through
/// [`Self::expandable_object_shape`] (= Java's polymorphic
/// `ExpandableObject.getShape()`, e.g. AnyAngle.java:46/:58,
/// 45Degree.java:50/:306), so a bundle field would be redundant;
/// re-add here with the first real consumer (and a pin), per the
/// cut-items discipline.
#[derive(Clone, Debug)]
pub struct DrillInfo {
    /// Java `location` — the drill anchor.
    pub location: Point,
    /// Java `firstLayer`.
    pub first_layer: i32,
    /// Java `roomArr` — indexed by `layer - firstLayer`.
    pub room_arr: Vec<Option<u64>>,
}

/// The maze-state read seam of the T9 locator (module doc).
pub trait LocatorAccess {
    /// The room/item read seam behind the locator's room reads.
    type Ctx: NeighbourEngine;

    /// The engine the room reads go through.
    fn engine(&self) -> &Self::Ctx;

    /// Java: reading the `MazeSearchElement.backtrackDoor` REFERENCE.
    /// `None` for an unregistered key — the engine's registry covers
    /// every door of a finished backtrack chain, so `None` there is an
    /// invariant violation (callers panic, matching the Java
    /// impossible-null).
    fn resolve_backtrack_door(&self, key: u64) -> Option<ExpandableObject>;

    /// Java `ExpandableObject.getMazeSearchElement(section)` — the
    /// element VALUE (Java returns a reference into the door's state;
    /// the state is frozen once the search finished).
    fn maze_search_element(&self, door: &ExpandableObject, section: i32) -> MazeSearchElement;

    /// Java `ExpandableObject.mazeSearchElementCount()`.
    fn maze_search_element_count(&self, door: &ExpandableObject) -> i32;

    /// The `ExpansionDrill` reads; `None` when `door` is not a drill
    /// (the Java `instanceof ExpansionDrill` tests).
    fn drill_info(&self, door: &ExpandableObject) -> Option<DrillInfo>;

    /// Java `ExpandableObject.getShape()` (module doc for the
    /// per-kind resolution).
    fn expandable_object_shape(&self, door: &ExpandableObject) -> TileShape;
}

impl<
    'a,
    E: crate::drill::DrillEngine,
    D: crate::drill::DestinationDistance,
    V: crate::drill::ViaLayerChecker,
> LocatorAccess for MazeSearchEngine<'a, E, D, V>
{
    type Ctx = E;

    fn engine(&self) -> &E {
        self.ctx
    }

    fn resolve_backtrack_door(&self, key: u64) -> Option<ExpandableObject> {
        self.backtrack_registry.get(&key).cloned()
    }

    fn maze_search_element(&self, door: &ExpandableObject, section: i32) -> MazeSearchElement {
        self.maze_element(door, section).clone()
    }

    fn maze_search_element_count(&self, door: &ExpandableObject) -> i32 {
        self.maze_search_element_count(door)
    }

    fn drill_info(&self, door: &ExpandableObject) -> Option<DrillInfo> {
        if !door.is_drill() {
            return None;
        }
        let drill = self.live_drill(door);
        Some(DrillInfo {
            location: drill.location.clone(),
            first_layer: drill.first_layer,
            room_arr: drill.room_arr.clone(),
        })
    }

    fn expandable_object_shape(&self, door: &ExpandableObject) -> TileShape {
        match door {
            // Java `ExpansionDoor.getShape()` = LIVE intersection of
            // the current endpoint-room shapes.
            ExpandableObject::RoomDoor(d) => {
                let first_key = self
                    .ctx
                    .room_key_of_id(d.first_room_id)
                    .expect("door endpoint room resolved by id");
                let second_key = self
                    .ctx
                    .room_key_of_id(d.second_room_id)
                    .expect("door endpoint room resolved by id");
                crate::expansion::ExpansionDoor::shape_between(
                    &self.ctx.room_shape(first_key),
                    &self.ctx.room_shape(second_key),
                )
            }
            // Java `TargetItemExpansionDoor.getShape()` returns the
            // construction-time field (`TargetItemExpansionDoor.java:34-37`).
            ExpandableObject::TargetDoor(d) => d.shape().clone(),
            ExpandableObject::DrillPage { row, column, .. } => self.pages.page_shape(*row, *column),
            ExpandableObject::Drill { .. } | ExpandableObject::StandaloneDrill { .. } => {
                self.live_drill(door).get_shape().clone()
            }
        }
    }
}
