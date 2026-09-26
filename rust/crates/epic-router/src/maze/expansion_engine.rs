//! Java `autoroute/maze/MazeExpansionEngine.java:31-236` — the drill
//! dispatch half of the maze expansion (`expandToDrill`,
//! `expandToDrillPage`, `expandToDrillsOfPage`). The layer-change half
//! (`expandToOtherLayers` + `checkLayerWithAnyMatchingVia`,
//! `:237-414`) is the T5 port in [`crate::drill::expand_other_layers`],
//! wired into the engine by [`crate::maze::search_engine`].

use epic_geometry::float_line::FloatLine;
use epic_geometry::tile_shape::TileShape;

use crate::control::AutorouteControl;
use crate::drill::{DestinationDistance, DrillEngine, DrillPageArray, ExpansionDrill};
use crate::expansion::NeighbourEngine;
use crate::maze::list_element::{ExpandableObject, Front, MazeListElement};

/// The maze front handle the expansion engine writes through — the
/// engine owns the front; this trait keeps the expansion module free
/// of the search-engine struct (avoids the borrow knot).
pub trait FrontSink {
    /// Java `search.mazeExpansionList.add` — the TreeSet add override
    /// INCLUDING the fanout frontier filter (`MazeSearchEngine
    /// .java:87-124`). The two optionals are the per-element facts
    /// Java reads live off the room/drill objects at add time:
    /// `next_room_layer` is `element.nextRoom.getLayer()` (`None` =
    /// Java `nextRoom == null`), `drill_location` the
    /// `ExpansionDrill.location` when the door is a drill. See
    /// [`Front::gated_add`].
    fn add(
        &mut self,
        element: MazeListElement,
        next_room_layer: Option<i32>,
        drill_location: Option<&epic_geometry::float_point::FloatPoint>,
    ) -> bool;
}

impl FrontSink for Front {
    fn add(
        &mut self,
        element: MazeListElement,
        next_room_layer: Option<i32>,
        drill_location: Option<&epic_geometry::float_point::FloatPoint>,
    ) -> bool {
        Front::gated_add(self, element, next_room_layer, drill_location)
    }
}

/// Java `MazeExpansionEngine` — the drill dispatch. All state comes in
/// per call; the struct exists only to group the three methods (Java
/// holds a `search` back-reference instead).
pub struct MazeExpansionEngine;

impl MazeExpansionEngine {
    /// Java `expandToDrill` (`:31-112`).
    #[allow(clippy::too_many_arguments)]
    pub fn expand_to_drill(
        ctx: &mut impl DrillEngine,
        ctrl: &AutorouteControl,
        front: &mut impl FrontSink,
        destination_distance: &mut impl DestinationDistance,
        pages: &DrillPageArray,
        drill: &ExpansionDrill,
        drill_door: &ExpandableObject,
        from_element: &MazeListElement,
        add_costs: i32,
    ) {
        let Some(next_room_key) = from_element.next_room_key else {
            // Java reads fromElement.nextRoom.getLayer() unconditionally
            // — a drill expansion from a null next-room is unreachable
            // (Java would NPE).
            panic!("expandToDrill requires a non-null nextRoom");
        };
        let layer = ctx.room_layer(next_room_key);
        let trace_half_width = ctrl.compensated_trace_half_width[layer as usize];
        let room_shape_is_thin =
            ctx.room_shape(next_room_key).min_width() < 2.0 * f64::from(trace_half_width);

        // The thin-room rejection (`:37-51`): a thin room is enterable
        // only through the backtrack door's shape.
        let backtrack_intersects = match &from_element.backtrack_door {
            None => false,
            Some(backtrack) => drill
                .get_shape()
                .intersects(&Self::door_shape(ctx, pages, backtrack)),
        };
        if room_shape_is_thin && !backtrack_intersects {
            return;
        }

        let via_radius = ctrl.via_radii[layer as usize];
        let shrinked_drill_shape = drill.get_shape().shrink(via_radius);
        let mut compare_corner = from_element
            .shape_entry
            .a
            .middle_point(&from_element.shape_entry.b);
        // The pin-exit-corner special case (`:56-65`): expanding from a
        // drill page whose backtrack door is a target door of a PIN.
        if let (ExpandableObject::DrillPage { .. }, Some(ExpandableObject::TargetDoor(door))) =
            (&from_element.door, from_element.backtrack_door.as_ref())
            && let Some(nearest_exit_corner) = ctx.pin_nearest_trace_exit_corner(
                door.item_key,
                &drill.location.to_float(),
                trace_half_width,
                layer,
            )
        {
            compare_corner = nearest_exit_corner;
        }
        let nearest_point = shrinked_drill_shape.nearest_point_approx(&compare_corner);
        let shape_entry = FloatLine::new(nearest_point, nearest_point);
        let section_index = layer - drill.first_layer;
        let mut expansion_value = from_element.expansion_value
            + f64::from(add_costs)
            + nearest_point.weighted_distance(
                &compare_corner,
                ctrl.trace_costs[layer as usize].horizontal,
                ctrl.trace_costs[layer as usize].vertical,
            );
        // The backtrack inheritance (`:76-85`): through a drill PAGE the
        // backtrack chain is kept; through any other door a normal via
        // cost is paid and the door becomes the backtrack.
        let (new_backtrack_door, new_section_no_of_backtrack_door) =
            if matches!(from_element.door, ExpandableObject::DrillPage { .. }) {
                (
                    from_element.backtrack_door.clone(),
                    from_element.section_no_of_backtrack_door,
                )
            } else {
                expansion_value += ctrl.min_normal_via_cost;
                (
                    Some(from_element.door.clone()),
                    from_element.section_no_of_door,
                )
            };
        let sorting_value = expansion_value + destination_distance.calculate(&nearest_point, layer);
        let new_element = MazeListElement::new(
            drill_door.clone(),
            section_index,
            new_backtrack_door,
            new_section_no_of_backtrack_door,
            expansion_value,
            sorting_value,
            None,
            shape_entry,
            from_element.room_ripped,
            crate::drill::Adjustment::None,
            false,
        );
        // The element's next room is Java-null (a drill element) and
        // the door IS a drill — the start-layer arm cannot fire, the
        // drill arm reads this drill's location (`:109-119`).
        front.add(new_element, None, Some(&drill.location.to_float()));
    }

    /// Java `expandToDrillPage` (`:115-143`).
    #[allow(clippy::too_many_arguments)]
    pub fn expand_to_drill_page(
        ctx: &mut impl DrillEngine,
        ctrl: &AutorouteControl,
        front: &mut impl FrontSink,
        destination_distance: &mut impl DestinationDistance,
        pages: &DrillPageArray,
        row: i32,
        column: i32,
        from_element: &MazeListElement,
    ) {
        let next_room_key = from_element
            .next_room_key
            .expect("expandToDrillPage requires a non-null nextRoom");
        let layer = ctx.room_layer(next_room_key);
        let from_element_shape_entry_middle = from_element
            .shape_entry
            .a
            .middle_point(&from_element.shape_entry.b);
        // Java `drillPage.shape.nearestPoint(FloatPoint)` — the
        // IntBox overload returning a FloatPoint.
        let nearest_point = pages
            .page(row, column)
            .shape
            .nearest_point(&from_element_shape_entry_middle);
        let expansion_value = from_element.expansion_value + ctrl.min_normal_via_cost;
        let sorting_value = expansion_value
            + nearest_point.weighted_distance(
                &from_element_shape_entry_middle,
                ctrl.trace_costs[layer as usize].horizontal,
                ctrl.trace_costs[layer as usize].vertical,
            )
            + destination_distance.calculate(&nearest_point, layer);
        let page_id = pages.page(row, column).get_id();
        let new_element = MazeListElement::new(
            ExpandableObject::DrillPage {
                row,
                column,
                id: page_id,
            },
            layer,
            Some(from_element.door.clone()),
            from_element.section_no_of_door,
            expansion_value,
            sorting_value,
            from_element.next_room_key,
            from_element.shape_entry,
            from_element.room_ripped,
            crate::drill::Adjustment::None,
            false,
        );
        // The element's next room is the from element's room (the page
        // inherits it, layer already resolved above) and the door is a
        // DrillPage — NOT an `ExpansionDrill`, so the drill arm cannot
        // fire; only the start-layer arm is live here.
        front.add(new_element, Some(layer), None);
    }

    /// Java `expandToDrillsOfPage` (`:145-235`) — the occupancy checks
    /// at `:224-234` included.
    pub fn expand_to_drills_of_page(
        ctx: &mut impl DrillEngine,
        ctrl: &AutorouteControl,
        front: &mut impl FrontSink,
        destination_distance: &mut impl DestinationDistance,
        pages: &mut DrillPageArray,
        from_element: &MazeListElement,
    ) {
        let ExpandableObject::DrillPage { row, column, .. } = from_element.door else {
            panic!("expandToDrillsOfPage requires a DrillPage door");
        };
        let from_room_layer = from_element.section_no_of_door;
        // Java iterates the LIVE `page.drills` array (`getDrills`
        // memoized once before the loop, `DrillPage.java:63-131` +
        // `:145-235`). The port previously snapshotted it with a full
        // deep clone (`to_vec()` — the M5-T1 ranked drill site,
        // 4.3%/6.9% of all allocations: every `ExpansionDrill` carries
        // two heap `Vec`s, so each page expansion cloned 1 + 2N
        // buffers). The clone was unnecessary BY THE BORROW CHECKER:
        // `pages` and `ctx` are distinct borrows, the loop body reads
        // the memo only through `&DrillPageArray`, and nothing on the
        // `expand_to_drill` path mutates it — so a scoped memoization
        // plus an index walk over the live array is read-for-read
        // identical.
        let drill_count = {
            let page = pages.page_mut(row, column);
            page.get_drills(ctx, ctrl.net_number, ctrl.attach_smd_allowed)
                .len()
        };
        for d in 0..drill_count {
            let current_drill = pages.page_drill(row, column, d);
            let section_index = from_room_layer - current_drill.first_layer;
            if section_index < 0 || section_index >= current_drill.room_arr.len() as i32 {
                continue;
            }
            // Java `roomArr[sectionIndex] != fromElement.nextRoom` — a
            // null slot skips (null != room), a room key mismatch too.
            if current_drill.room_arr[section_index as usize] != from_element.next_room_key {
                continue;
            }
            if current_drill
                .maze_search_element(section_index as usize)
                .is_occupied
            {
                continue;
            }
            Self::expand_to_drill(
                ctx,
                ctrl,
                front,
                destination_distance,
                pages,
                current_drill,
                &ExpandableObject::Drill {
                    row,
                    column,
                    d,
                    id: current_drill.get_id(),
                },
                from_element,
                0,
            );
        }
    }

    /// Java `door.getShape()` for an [`ExpandableObject`] — the door
    /// line between the live room shapes for room doors, the stored
    /// shape for target doors, the page box / drill shape otherwise.
    #[must_use]
    pub fn door_shape(
        ctx: &impl NeighbourEngine,
        pages: &DrillPageArray,
        door: &ExpandableObject,
    ) -> TileShape {
        match door {
            ExpandableObject::RoomDoor(room_door) => {
                let first = ctx.room_shape(ctx.room_key_of_id(room_door.first_room_id).unwrap_or_else(
                    || {
                        panic!(
                            "a live room door's first room is registered (door id {}, first room id {})",
                            room_door.id(),
                            room_door.first_room_id
                        )
                    },
                ));
                let second = ctx.room_shape(ctx.room_key_of_id(room_door.second_room_id).unwrap_or_else(
                    || {
                        panic!(
                            "a live room door's second room is registered (door id {}, second room id {})",
                            room_door.id(),
                            room_door.second_room_id
                        )
                    },
                ));
                crate::expansion::ExpansionDoor::shape_between(&first, &second)
            }
            ExpandableObject::TargetDoor(target_door) => target_door.shape().clone(),
            ExpandableObject::DrillPage { row, column, .. } => pages.page_shape(*row, *column),
            ExpandableObject::Drill { row, column, d, .. } => {
                pages.page_drill_shape(*row, *column, *d)
            }
            ExpandableObject::StandaloneDrill { shape, .. } => shape.clone(),
        }
    }
}
