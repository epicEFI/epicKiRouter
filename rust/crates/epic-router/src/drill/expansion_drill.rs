//! Java `autoroute/drill/ExpansionDrill.java` (140 lines) — the
//! layer-change expansion object anchored at one drill location: it
//! owns the per-layer completed rooms a via would land in.

use epic_geometry::int_box::IntBox;
use epic_geometry::point::Point;
use epic_geometry::regular_tile_shape::RegularTileShape;
use epic_geometry::tile_shape::TileShape;

use super::maze_search_element::MazeSearchElement;
use super::{DrillEngine, java_ordered_entries};

/// Java `ExpansionDrill` — the drill's own maze state plus the room
/// array over `[firstLayer, lastLayer]`.
#[derive(Clone, Debug)]
pub struct ExpansionDrill {
    /// Java `location` (public final) — the drill anchor.
    pub location: Point,
    /// Java `firstLayer` (public final).
    pub first_layer: i32,
    /// Java `lastLayer` (public final).
    pub last_layer: i32,
    /// Java `roomArr` — the per-layer completed room KEYS
    /// (`roomArr[layer - firstLayer]`); `None` = a Java null slot
    /// (only transiently — [`Self::calculate_expansion_rooms`] fills
    /// or rejects the whole drill).
    pub room_arr: Vec<Option<u64>>,
    /// Java `mazeSearchElements` — one per DRILL layer
    /// (`lastLayer - firstLayer + 1`).
    maze_search_elements: Vec<MazeSearchElement>,
    /// Java `shape` (public final).
    pub shape: TileShape,
}

impl ExpansionDrill {
    /// Java ctor (`:32-53`).
    pub fn new(shape: TileShape, location: Point, first_layer: i32, last_layer: i32) -> Self {
        let layer_count = usize::try_from((last_layer - first_layer + 1).max(0)).unwrap_or(0);
        Self {
            location,
            first_layer,
            last_layer,
            room_arr: vec![None; layer_count],
            maze_search_elements: (0..layer_count)
                .map(|_| MazeSearchElement::default())
                .collect(),
            shape,
        }
    }

    /// Java `getShape()`.
    #[must_use]
    pub fn get_shape(&self) -> &TileShape {
        &self.shape
    }

    /// Java `getId()` (`:127-130`) — `31 * (31 * location.getId() +
    /// firstLayer) + lastLayer`; wrapping like Java int overflow. The
    /// M1a `Point::get_id` mirrors Java `Point.getId()` (the grid
    /// 2D-encode), so the port reproduces the Java hash exactly.
    #[must_use]
    pub fn get_id(&self) -> i32 {
        31i32
            .wrapping_mul(
                31i32
                    .wrapping_mul(self.location.get_id())
                    .wrapping_add(self.first_layer),
            )
            .wrapping_add(self.last_layer)
    }

    /// Java `getDimension()` — a drill is 2-dimensional.
    #[must_use]
    pub fn get_dimension(&self) -> i32 {
        2
    }

    /// Java `mazeSearchElementCount()`.
    #[must_use]
    pub fn maze_search_element_count(&self) -> usize {
        self.maze_search_elements.len()
    }

    /// Java `getMazeSearchElement(int)` — the caller indexes with the
    /// i32 offset `layer - firstLayer` (the T6 expansion code's
    /// discipline), converted at the boundary.
    #[must_use]
    pub fn maze_search_element(&self, index: usize) -> &MazeSearchElement {
        &self.maze_search_elements[index]
    }

    /// Java `getMazeSearchElement(int)` mutable.
    pub fn maze_search_element_mut(&mut self, index: usize) -> &mut MazeSearchElement {
        &mut self.maze_search_elements[index]
    }

    /// Java `reset()` (`:120-123`).
    pub fn reset(&mut self) {
        for element in &mut self.maze_search_elements {
            element.reset();
        }
    }

    /// Java `calculateExpansionRooms(autorouteEngine)` (`:55-92`):
    /// fills `roomArr` with the completed room on every layer spanned
    /// by the drill, or returns `false` (whole drill REJECTED) when
    /// any layer's completion does not produce exactly one room. The
    /// walk consumes `overlappingObjects(pointBox, -1)` — Java's
    /// TreeSet (object id descending) — removing each consumed entry;
    /// on a layer without a matching existing room, a fresh null-shape
    /// room is completed through the T4 seam (Java wraps completion in
    /// `catch (Exception)` -> empty; the port lets the completion
    /// machinery panic, the M1a/M3 discipline).
    pub fn calculate_expansion_rooms(&mut self, ctx: &mut impl DrillEngine) -> bool {
        // Java `TileShape.getInstance(location)` — the point-degenerate
        // IntBox (TileShape.java:61 delegates to IntBox.getInstance).
        let Point::Int(location) = self.location else {
            panic!("ExpansionDrill location is an IntPoint (Java IntBox.getInstance requires it)");
        };
        let search_shape =
            TileShape::RegularTileShape(RegularTileShape::IntBox(IntBox::new(location, location)));
        let overlaps = java_ordered_entries(ctx, &search_shape, -1);
        // Java consumes the collection with it.remove(); entries are
        // flagged consumed here.
        let mut consumed = vec![false; overlaps.len()];
        for i in self.first_layer..=self.last_layer {
            let mut found_room: Option<u64> = None;
            for (index, entry) in overlaps.iter().enumerate() {
                if consumed[index] {
                    continue;
                }
                match ctx.tree_object_room(entry.object_key) {
                    // Not a room: removed from the collection either way.
                    None => consumed[index] = true,
                    Some(room_key) => {
                        if ctx.room_layer(room_key) == i {
                            found_room = Some(room_key);
                            consumed[index] = true;
                            break;
                        }
                    }
                }
            }
            if found_room.is_none() {
                // Java `IncompleteFreeSpaceExpansionRoom(null, i,
                // searchShape)` — the null room shape mirrors as None.
                let new_rooms = ctx.complete_expansion_room(None, &search_shape, i);
                if new_rooms.len() != 1 {
                    // An obstacle sits in the compensated tree at this
                    // location; the rooms of EARLIER layers of this
                    // drill stay in the engine (Java-faithful).
                    return false;
                }
                found_room = Some(new_rooms[0]);
            }
            self.room_arr[(i - self.first_layer) as usize] = found_room;
        }
        true
    }
}
