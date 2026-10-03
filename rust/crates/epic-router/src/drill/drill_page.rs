//! Java `autoroute/drill/DrillPage.java` (194 lines) — one grid cell's
//! candidate via enumeration: the page minus the obstacle cutouts,
//! split into convex drill pieces, each anchored by an
//! [`ExpansionDrill`].

use epic_geometry::int_box::IntBox;
use epic_geometry::point::Point;
use epic_geometry::regular_tile_shape::RegularTileShape;
use epic_geometry::tile_shape::TileShape;

use super::expansion_drill::ExpansionDrill;
use super::maze_search_element::MazeSearchElement;
use super::{DrillEngine, java_ordered_entries};

/// Java `DrillPage` — the memoized drill set of one page.
pub struct DrillPage {
    /// Java `shape` (public final `IntBox`).
    pub shape: IntBox,
    /// Java `mazeSearchElements` — one per BOARD layer.
    maze_search_elements: Vec<MazeSearchElement>,
    /// Java `drills` — `None` = Java null (not yet calculated).
    drills: Option<Vec<ExpansionDrill>>,
    /// Java `netNumber` — the memoized net (-1 initial).
    net_number: i32,
}

/// Java `calcPinCenterInDrill` (`:49-60`): the center of a
/// drill-allowed pin contained inside the drill shape on the layer.
/// Java iterates `board.overlappingItems` — a `TreeSet<Item>` in
/// DESCENDING item id — with NO break, so the LAST match wins, i.e.
/// the LOWEST-id pin in the shape ([`DrillEngine::overlapping_items`]
/// preserves that order; a first-match port silently changes
/// attach-SMD drill targets when two pins share one convex piece).
fn calc_pin_center_in_drill(
    ctx: &impl DrillEngine,
    drill_shape: &TileShape,
    layer: i32,
) -> Option<Point> {
    let mut result = None;
    for item_key in ctx.overlapping_items(drill_shape, layer) {
        if !ctx.item_is_pin(item_key) {
            continue;
        }
        if !ctx.pin_drill_allowed(item_key) {
            continue;
        }
        let Some(center) = ctx.pin_center(item_key) else {
            continue;
        };
        if drill_shape.contains_inside(&center) {
            result = Some(center);
        }
    }
    result
}

impl DrillPage {
    /// Java ctor (`:31-47`): the maze-search-element array is one per
    /// board layer.
    pub fn new(shape: IntBox, layer_count: i32) -> Self {
        let layer_count = usize::try_from(layer_count.max(0)).unwrap_or(0);
        Self {
            shape,
            maze_search_elements: (0..layer_count)
                .map(|_| MazeSearchElement::default())
                .collect(),
            drills: None,
            net_number: -1,
        }
    }

    /// Java `getShape()` — the page as the tile-shape family member
    /// the tree queries take.
    #[must_use]
    pub fn shape_tile(&self) -> TileShape {
        TileShape::RegularTileShape(RegularTileShape::IntBox(self.shape))
    }

    /// Java `getDimension()` — a page is 2-dimensional.
    #[must_use]
    pub fn get_dimension(&self) -> i32 {
        2
    }

    /// Java `getId()` (`:190-193`) — `31 * shape.getId() + netNumber`
    /// over the MEMOIZED net number (a cached page reports the net it
    /// was computed for). Wrapping like Java int overflow.
    #[must_use]
    pub fn get_id(&self) -> i32 {
        31i32
            .wrapping_mul(self.shape.get_id())
            .wrapping_add(self.net_number)
    }

    /// Java `mazeSearchElementCount()`.
    #[must_use]
    pub fn maze_search_element_count(&self) -> usize {
        self.maze_search_elements.len()
    }

    /// The memoized drills (Java `page.drills` — `None` is the Java
    /// null before the first `get_drills` call).
    #[must_use]
    pub fn drills(&self) -> Option<&[ExpansionDrill]> {
        self.drills.as_deref()
    }

    /// The mutable memoized drills (the maze engine's post-memoization
    /// section-state access).
    pub fn drills_mut(&mut self) -> Option<&mut Vec<ExpansionDrill>> {
        self.drills.as_mut()
    }

    /// Java `getMazeSearchElement(int)`.
    #[must_use]
    pub fn maze_search_element(&self, index: usize) -> &MazeSearchElement {
        &self.maze_search_elements[index]
    }

    /// Java `getMazeSearchElement(int)` mutable.
    pub fn maze_search_element_mut(&mut self, index: usize) -> &mut MazeSearchElement {
        &mut self.maze_search_elements[index]
    }

    /// Java `invalidate()` (`:170-172`): drills are recomputed on the
    /// next [`Self::get_drills`].
    pub fn invalidate(&mut self) {
        self.drills = None;
    }

    /// Java `reset()` (`:153-164`): resets each MEMOIZED drill's
    /// maze-search elements and the page's own elements. The drills
    /// memo SURVIVES — Java `invalidate()` (`:170-172`) is the
    /// separate operation that nulls it, so a reset does NOT force a
    /// re-enumeration at the next [`Self::get_drills`].
    pub fn reset(&mut self) {
        if let Some(drills) = &mut self.drills {
            for drill in drills {
                drill.reset();
            }
        }
        for element in &mut self.maze_search_elements {
            element.reset();
        }
    }

    /// Java `getDrills(netNumber, attachSmd)` (`:63-131`) — the
    /// candidate enumeration. Memoized on `(drills == null ||
    /// netNumber changed)`; NOTE `attachSmd` is NOT part of the key —
    /// a first call with `attachSmd = false` caches drills that a
    /// later `attachSmd = true` call reuses (Java-faithful quirk,
    /// pinned by test).
    pub fn get_drills(
        &mut self,
        ctx: &mut impl DrillEngine,
        net_number: i32,
        attach_smd: bool,
    ) -> &[ExpansionDrill] {
        if self.drills.is_none() || net_number != self.net_number {
            self.net_number = net_number;
            let page_shape = self.shape_tile();
            // The cutouts, in the Java tree-set iteration order (the
            // LinkedList walk over `overlappingTreeEntries` — object id
            // descending, shape index ascending, dedup).
            let mut cutouts: Vec<TileShape> = Vec::new();
            let mut prev_obstacle_shape =
                TileShape::RegularTileShape(RegularTileShape::IntBox(IntBox::EMPTY));
            for entry in java_ordered_entries(ctx, &page_shape, -1) {
                // Java: `!(currObject instanceof Item)` -> skip. Rooms
                // are skipped BEFORE the prev-shape update.
                if !ctx.is_item(entry.object_key) {
                    continue;
                }
                // Drillable items (own-net traces and planes) are not
                // obstacles for the drills.
                if ctx.item_is_drillable(entry.object_key, net_number) {
                    continue;
                }
                // With the attach-SMD relaxation, drill-allowed (SMD)
                // pins of the ROUTING NET stop being cutout obstacles
                // (M11-T9d, upstream #931: `attachSmd &&
                // pin.drillAllowed() && pin.containsNet(netNumber)` —
                // pre-#931 the skip applied to EVERY drill-allowed
                // pin, so drills could be proposed straight through a
                // foreign net's SMD pad).
                if ctx.item_is_pin(entry.object_key)
                    && attach_smd
                    && ctx.pin_drill_allowed(entry.object_key)
                    && ctx.item_contains_net(entry.object_key, net_number)
                {
                    continue;
                }
                let Some(current_obstacle_shape) =
                    ctx.tree_shape(entry.object_key, entry.shape_index_in_object)
                else {
                    // Java would dereference null here; live tree
                    // entries always carry their shape.
                    continue;
                };
                // The dedup compares the PREVIOUS ENTRY's shape, not a
                // running union — a via whose shape equals the previous
                // entry's is skipped even if an earlier different shape
                // also contained it (the same-shape-on-all-layers via
                // dedup; ported literally, the [Y1, Y2, Y1] entry run
                // cuts THREE holes).
                if !prev_obstacle_shape.contains_tile(&current_obstacle_shape) {
                    let cutout = current_obstacle_shape.intersection(&page_shape);
                    if cutout.dimension() == 2 {
                        cutouts.push(cutout);
                    }
                }
                prev_obstacle_shape = current_obstacle_shape;
            }
            // Java `new PolylineArea(this.shape, holes)` keeps the
            // holes' TILE identity — `TileShape` IS a `PolylineShape`
            // subclass — so `PolylineArea.splitToConvex` dispatches
            // `dividePiece.cutout(holePiece)` on the hole's dynamic
            // type: an octagon cutout follows
            // `IntOctagon.cutoutFrom(IntBox)`, whose piece ORDER
            // differs from any polygon conversion (the DrillSpike
            // capture caught exactly this on the two-drill pages).
            // The loop below is `PolylineArea.splitToConvex`
            // (PolylineArea.java:168-204) verbatim over TileShapes,
            // with the Rust PolylineArea's polygon flattening
            // bypassed.
            let mut current_piece_list: Vec<TileShape> = page_shape.split_to_convex();
            for hole in &cutouts {
                if hole.dimension() < 2 {
                    // Java: FRLogger.warn("dimension 2 for hole expected")
                    continue;
                }
                // `TileShape::split_to_convex` is `[self]` (tile
                // shapes are convex).
                for hole_piece in hole.split_to_convex() {
                    let mut new_piece_list: Vec<TileShape> = Vec::new();
                    for divide_piece in &current_piece_list {
                        if let Some(flag) = ctx.stop_flag()
                            && flag.load(std::sync::atomic::Ordering::Relaxed)
                        {
                            // Upstream 8fb76a61b: `if (drillShapes ==
                            // null) return this.drills;` — splitToConvex
                            // signals stop with null (checked per divide
                            // piece, PolylineArea.java:188-190) and the
                            // guard returns the LinkedList assigned EMPTY
                            // at the top of the recompute branch.
                            // `this.netNumber` was set up top too, so the
                            // memo key stays consistent: same-net re-asks
                            // answer the empty memo, a net change
                            // re-derives. Pre-fix Java NPE'd at
                            // `drillShapes.length`; the port had mirrored
                            // that as the panic this replaces.
                            self.drills = Some(Vec::new());
                            return &[];
                        }
                        // Java `dividePiece.cutout(holePiece)`: the
                        // DIVIDE piece's dynamic type selects the leaf
                        // body — `IntBox.cutout` (`IntBox.java:688-695`)
                        // simplifies each result, `IntOctagon.cutout`
                        // (`IntOctagon.java:1059-1061`) and
                        // `Simplex.cutout` (`Simplex.java:695-697`) do
                        // NOT. [`TileShape::cutout`] owns that dispatch,
                        // so a box-like octagon round result STAYS an
                        // `IntOctagon` when the divide piece is an
                        // octagon, and the next round's `cutoutFrom`
                        // dispatches the octagon arm (an unconditional
                        // simplify here re-typed it `IntBox` and changed
                        // the decomposition the next hole round and the
                        // T6 drill order inherit).
                        for piece in divide_piece.cutout(&hole_piece) {
                            if piece.dimension() == 2 {
                                new_piece_list.push(piece);
                            }
                        }
                    }
                    current_piece_list = new_piece_list;
                }
            }
            let drill_shapes = current_piece_list;
            let drill_first_layer = 0;
            let drill_last_layer = ctx.layer_count() - 1;
            let mut drills = Vec::new();
            for current_drill_shape in drill_shapes {
                let mut location = None;
                if attach_smd {
                    location =
                        calc_pin_center_in_drill(ctx, &current_drill_shape, drill_first_layer);
                    if location.is_none() {
                        location =
                            calc_pin_center_in_drill(ctx, &current_drill_shape, drill_last_layer);
                    }
                }
                let location = location
                    .unwrap_or_else(|| Point::Int(current_drill_shape.centre_of_gravity().round()));
                let mut drill = ExpansionDrill::new(
                    current_drill_shape,
                    location,
                    drill_first_layer,
                    drill_last_layer,
                );
                let ok = drill.calculate_expansion_rooms(ctx);
                if ok {
                    drills.push(drill);
                }
            }
            self.drills = Some(drills);
        }
        self.drills
            .as_deref()
            .expect("drills just computed or previously memoized")
    }
}
