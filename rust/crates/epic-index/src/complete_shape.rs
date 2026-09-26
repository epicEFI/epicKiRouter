//! Java `ShapeSearchTree.completeShape` + `restrainShape` +
//! `divideLargeRoom` — the maximal-free-shape completion behind the
//! autorouter's expansion rooms (M3-T4).
//!
//! Ported arms (Java virtual dispatch is by tree subclass):
//! * base arm — `ShapeSearchTree.completeShape`
//!   (`ShapeSearchTree.java:580-693`, 114 lines), with
//!   `restrainShape` (`:701-811`) and `divideLargeRoom`
//!   (`:1095-1118`).
//! * 90-degree arm — `ShapeSearchTree90Degree.completeShape`
//!   (`ShapeSearchTree90Degree.java:39-195`, ported in M3-T9 for the
//!   A90 locator pin), with its box-typed `restrainShape`
//!   (`:196-320`). No `divideLargeRoom` — the 90-degree override does
//!   NOT divide.
//! * 45-degree arm — `ShapeSearchTree45Degree.completeShape`
//!   (`ShapeSearchTree45Degree.java:96-281`, 186 lines), with its
//!   `restrainShape` (`:305-418`), `divideLargeRoom` override
//!   (`:288-298`), and the integer helpers `signedLineDistance`
//!   (`:67-86`), `obstacleSegmentTouchesInside` (`:38-65`),
//!   `calcOutsideRestrainedShape` / `calcInsideRestrainedShape`
//!   (`:424-486`).
//!
//! D17 genericity (the crate sees no board types): the object
//! semantics the Java walk reads off each leaf — trace-obstacle for a
//! net, shape layer, the stored tree shape, the
//! `CompleteFreeSpaceExpansionRoom` type test — come from the
//! [`CompleteShapeObjects`] trait the caller implements. The result
//! rooms are the [`IncompleteRoom`] triple (shape, layer, contained
//! shape); ids/doors/handles are epic-router concerns.
//!
//! Determinism notes (all capture-proven, M3-T4 spike
//! `logs/M3-T4/spike_run1.jsonl`):
//! * The BASE arm sorts its leaves by Java `Leaf.compareTo`
//!   (`datastructures/ShapeTree.java:217-223`): object order, then
//!   shape index. The comparator is a total order on distinct
//!   entries — two leaves tie only if they share object AND index,
//!   which is the same leaf — so sort stability is moot and the
//!   result equals the `TreeSet` order
//!   [`SearchTree::query_candidates`] already returns.
//! * The 45-degree arm does NOT sort: obstacles are processed at DFS
//!   pop time (`ArrayStack.push(firstChild); push(secondChild)` →
//!   second child pops first). The raw arena walk below reproduces
//!   that stack order with a `Vec`. MUTATION FINDING (M3-T4): neither
//!   the 45-degree push order (first<->second flipped) nor the base
//!   arm's leaf sort (comparator reversed) is OBSERVABLE through the
//!   S/T pins — the fixture's obstacles are pairwise disjoint, and
//!   half-plane restraining over disjoint obstacles is confluent (the
//!   final cells of the cut arrangement do not depend on cut order).
//!   The order-faithful port is kept anyway: Java's orders are the
//!   documented behaviour, and order CAN matter once obstacles
//!   interact (overlapping keepouts, compensated-shape overlaps).
//! * Both arms reassign a `boundingShape`/`newBoundingShape` variable
//!   inside the obstacle loop (union-accumulated over the result
//!   rooms). The BASE arm's leaf DFS has finished by then — dead
//!   state there; the 45-degree DFS READS the accumulated union
//!   afterwards to narrow the search hull (Java `:157` read after the
//!   `:264` write). The port omits the narrowing — RESULT-EQUIVALENT,
//!   not dead: rooms only SHRINK as obstacles are processed, so an
//!   object disjoint from the current rooms' union can never
//!   intersect a later (smaller) room — the restraint the narrowing
//!   would skip is a no-op.
//! * `FRLogger` diagnostics (warn/debug/trace, incl. the
//!   `COMPLETE_SHAPE_BLOCKED` trace at
//!   `ShapeSearchTree45Degree.java:248-262`) are non-functional and
//!   omitted; the guard ARMS they annotate are ported exactly.
//!
//! Allocation strategy (M5 slice B — storage only, ZERO semantic
//! change): same inserts, same walk/iteration order, same removals,
//! same query results. The per-query/per-obstacle `Vec` churn the T1
//! profile ranked #1 (the `complete_shape.rs:810` family, 39.2% of
//! bm06's allocations) is eliminated through three storage-only
//! mechanisms, each value-identical to the owned form it replaced:
//! * the room list is a DOUBLE BUFFER swapped per processed obstacle
//!   (Java's `result`/`newResult` pair) instead of a fresh `Vec` per
//!   obstacle — `clear` + `swap` preserve contents and order exactly;
//! * `restrainShape`/`divideLargeRoom` gained appending/in-place
//!   cores (`*_into` / `*_in_place`); the owned Java-parity surfaces
//!   remain and delegate to them (same pushes in the same order, the
//!   recursion appends into the caller's buffer);
//! * the room buffers, DFS stacks and sort keys come from a
//!   per-thread pool ([`crate::scratch`], panic-free take/put) whose
//!   capacity persists across queries; buffers are cleared before
//!   every fill and the returned room list leaves the pool (one
//!   `Vec` per query at the floor — the caller owns it).

use crate::scratch::{RoomBuffers, with_node_stack, with_room_buffers};
use crate::search_tree::{SearchTree, SearchTreeVariant};
use crate::shape_tree::{NodeKind, intersects as bounds_intersect};
use epic_geometry::int_box::IntBox;
use epic_geometry::int_octagon::IntOctagon;
use epic_geometry::int_point::IntPoint;
use epic_geometry::line::Line;
use epic_geometry::line_segment::LineSegment;
use epic_geometry::regular_tile_shape::RegularTileShape;
use epic_geometry::shape::ShapeBoundingDirections;
use epic_geometry::side::Side;
use epic_geometry::simplex::Simplex;
use epic_geometry::tile_shape::TileShape;

/// The room triple `IncompleteFreeSpaceExpansionRoom` carries through
/// `completeShape` (Java `autoroute/expansion/IncompleteFreeSpaceExpansionRoom.java`):
/// the (not yet maximal) room shape, the layer, and the shape that must
/// stay contained in every result room.
#[derive(Clone, Debug, PartialEq)]
pub struct IncompleteRoom {
    /// Java `getShape()`.
    pub shape: TileShape,
    /// Java `getLayer()`.
    pub layer: i32,
    /// Java `getContainedShape()` — never null past the entry guards.
    pub contained_shape: TileShape,
}

/// The `completeShape` inputs beyond the tree: the room being completed
/// and the ignore pair (Java parameters `room`, `netNumber`,
/// `ignoreObject`, `ignoreShape`).
#[derive(Clone, Debug)]
pub struct CompleteShapeQuery<'a> {
    /// Java `room.getShape()` — `None` is Java's null (the base arm
    /// then completes against the whole board).
    pub room_shape: Option<&'a TileShape>,
    /// Java `room.getContainedShape()` — `None` is Java's null, the
    /// empty-result guard.
    pub contained: Option<&'a TileShape>,
    /// Java `room.getLayer()`.
    pub layer: i32,
    /// Java `netNumber`.
    pub net_number: i32,
    /// Java `ignoreObject` — reference identity there, object key here.
    pub ignore_object: Option<u64>,
    /// Java `ignoreShape`.
    pub ignore_shape: Option<&'a TileShape>,
}

/// The per-leaf object semantics `completeShape` reads off the tree
/// (D17: the callers own the objects). Java reads the same off
/// `SearchTreeObject` (`board/searchtree/SearchTreeObject.java`).
pub trait CompleteShapeObjects {
    /// Java `SearchTreeObject.isTraceObstacle(netNumber)`.
    fn is_trace_obstacle(&self, object_key: u64, net_number: i32) -> bool;
    /// Java `SearchTreeObject.shapeLayer(shapeIndex)`.
    fn shape_layer(&self, object_key: u64, shape_index: u32) -> i32;
    /// Java `SearchTreeObject.getTreeShape(tree, shapeIndex)` — the
    /// shape the obstacle walk restrains with (compensated shapes when
    /// the tree holds them). `None` is unreachable for live entries
    /// (Java would NPE).
    fn tree_shape(&self, object_key: u64, shape_index: u32) -> Option<TileShape>;
    /// Java `currentObject instanceof CompleteFreeSpaceExpansionRoom`.
    fn is_complete_free_space(&self, object_key: u64) -> bool;
}

/// The virtual dispatch (`complete_shape_generic` for the plain tree,
/// `complete_shape_fortyfive_degree` for the 45-degree tree,
/// `complete_shape_ninety_degree` for the 90-degree tree). Java
/// resolves the same choice by tree subclass; the Rust caller resolves
/// by [`SearchTree::variant`].
#[must_use]
pub fn complete_shape<OBJ: CompleteShapeObjects>(
    tree: &SearchTree,
    objects: &OBJ,
    query: &CompleteShapeQuery,
    board_bbox: &IntBox,
) -> Vec<IncompleteRoom> {
    match tree.variant {
        SearchTreeVariant::FortyfiveDegree => {
            complete_shape_fortyfive_degree(tree, objects, query, board_bbox)
        }
        SearchTreeVariant::NinetyDegree => {
            complete_shape_ninety_degree(tree, objects, query, board_bbox)
        }
        SearchTreeVariant::Generic => complete_shape_generic(tree, objects, query, board_bbox),
    }
}

/// Java `ShapeSearchTree.completeShape`
/// (`ShapeSearchTree.java:580-693`) — the plain-tree arm.
#[must_use]
pub fn complete_shape_generic<OBJ: CompleteShapeObjects>(
    tree: &SearchTree,
    objects: &OBJ,
    query: &CompleteShapeQuery,
    board_bbox: &IntBox,
) -> Vec<IncompleteRoom> {
    let Some(contained) = query.contained else {
        // Java :585-588 — FRLogger.warn, empty result (the S2 capture
        // row).
        return Vec::new();
    };
    if tree.min_area_tree().root().is_none() {
        // Java :589-591 — empty tree, empty result.
        return Vec::new();
    }

    // Java :592-595: startShape = board bbox, intersected with the room
    // shape when the room carries one.
    let mut start_shape = TileShape::RegularTileShape(RegularTileShape::IntBox(*board_bbox));
    if let Some(room_shape) = query.room_shape {
        start_shape = start_shape.intersection(room_shape);
    }

    // Java :596. Unbounded hulls cannot arise here (start_shape is
    // clipped by the board box), so the Java NPE arm is unreachable.
    let directions: ShapeBoundingDirections = tree.variant.bounding_directions();
    if start_shape.bounding_shape(&directions).is_none() {
        return Vec::new();
    }

    // The walk runs inside the per-thread scratch pool (slice B): the
    // room double buffer is swapped per obstacle and the final list is
    // moved out; the pool's capacity persists across queries. The
    // buffers are cleared before every fill, so the reuse is invisible
    // (the in-file reuse pin + the compares witness it).
    with_room_buffers(|bufs| {
        let RoomBuffers { current, next } = bufs;
        current.clear();
        if start_shape.dimension() == 2 {
            // Java :598-603 — the seed room only for 2-dimensional starts.
            current.push(IncompleteRoom {
                shape: start_shape.clone(),
                layer: query.layer,
                contained_shape: contained.clone(),
            });
        }

        // Java :608-629 — collect the leaves whose bounds intersect the
        // hull, sorted by (object, shapeIndex). `query_candidates` returns
        // exactly that order (the comparator is a total order on distinct
        // entries, so Java's List.sort stability is moot).
        let Some(leaves) = tree.query_candidates(&start_shape, |a, b| a.cmp(&b)) else {
            return Vec::new();
        };

        for leaf in leaves {
            let object_key = leaf.object_key;
            if !objects.is_trace_obstacle(object_key, query.net_number)
                || objects.shape_layer(object_key, leaf.shape_index_in_object) != query.layer
                || query.ignore_object == Some(object_key)
            {
                continue;
            }
            let Some(object_shape) = objects.tree_shape(object_key, leaf.shape_index_in_object)
            else {
                // Java NPE arm (getTreeShape null) — unreachable for live
                // entries.
                continue;
            };

            // Java :639-688 — restrain every current room against the
            // obstacle. The `somethingChanged` flag decides whether the
            // room itself survives: restrained rooms are replaced by their
            // pieces, ignored/non-intersecting rooms survive unchanged.
            // `next` is the Java `newResult`: cleared per obstacle,
            // swapped into `current` after it (capacity retained).
            next.clear();
            for room in current.iter() {
                let mut something_changed = false;
                let intersection = room.shape.intersection(&object_shape);
                if intersection.dimension() == 2 {
                    // Java :647-650 — the completed-room ignore arm.
                    let ignore_expansion_room = objects.is_complete_free_space(object_key)
                        && query
                            .ignore_shape
                            .is_some_and(|is| is.contains_tile(&intersection));
                    if !ignore_expansion_room {
                        something_changed = true;
                        restrain_shape_generic_into(room, &object_shape, next);
                    }
                }
                if !something_changed {
                    // Java :681-686 — the room survives unchanged.
                    next.push(room.clone());
                }
            }
            std::mem::swap(current, next);
        }

        // Java :692.
        divide_large_room_in_place(current, board_bbox);
        std::mem::take(current)
    })
}

/// Java `ShapeSearchTree.restrainShape`
/// (`ShapeSearchTree.java:701-811`) — cut `room` against
/// `obstacle_shape` in Simplex arithmetic (the Java comment: converting
/// to Simplex first because octagon border lines of length 0 may
/// otherwise be mishandled). Allocation-free core in
/// [`restrain_shape_generic_into`]; this owned form is the Java-parity
/// surface. No in-tree callers (the arms call the `*_into` core);
/// kept as the owned Java-parity surface.
#[must_use]
pub fn restrain_shape_generic(
    room: &IncompleteRoom,
    obstacle_shape: &TileShape,
) -> Vec<IncompleteRoom> {
    let mut result = Vec::new();
    restrain_shape_generic_into(room, obstacle_shape, &mut result);
    result
}

/// The appending core of [`restrain_shape_generic`] (slice B): pushes
/// the same rooms in the same order the owned form returns, without
/// the per-call `Vec` (the recursion appends into the caller's
/// buffer).
fn restrain_shape_generic_into(
    room: &IncompleteRoom,
    obstacle_shape: &TileShape,
    out: &mut Vec<IncompleteRoom>,
) {
    let obstacle_simplex = obstacle_shape.to_simplex();
    // Java :716-722 — the contained shape is converted to Simplex too.
    let shape_to_be_contained: TileShape =
        TileShape::Simplex(Box::new(room.contained_shape.to_simplex()));
    if shape_to_be_contained.is_empty() {
        // Java :724-727 (null was already excluded by the entry guards;
        // IncompleteRoom never carries a null contained shape).
        return;
    }
    let layer = room.layer;

    // Java :729-747 — the border line whose segment intersects the room
    // interior and that is furthest from the contained shape (double
    // compare); cutLine stores the line's OPPOSITE (the kept half
    // plane).
    let mut cut_line: Option<Line> = None;
    let mut cut_line_distance = -1.0f64;
    let border_count = obstacle_simplex.border_line_count() as i32;
    for i in 0..border_count {
        let segment = simplex_border_segment(&obstacle_simplex, i);
        if room.shape.is_intersected_interior_by(&segment) {
            let current_line = obstacle_simplex.border_line(i);
            let current_min_distance = shape_to_be_contained.distance_to_the_left(&current_line);
            if current_min_distance > cut_line_distance {
                cut_line_distance = current_min_distance;
                cut_line = Some(current_line.opposite());
            }
        }
    }

    if let Some(cut_line) = cut_line {
        // Java :749-756 — keep the half plane right of the cut line.
        let half_plane = TileShape::from_line(&cut_line);
        let result_piece = room.shape.intersection(&half_plane);
        if result_piece.dimension() >= 2 {
            out.push(IncompleteRoom {
                shape: result_piece,
                layer,
                contained_shape: shape_to_be_contained,
            });
        }
        return;
    }

    // Java :757-808 — no cut line keeps the whole contained shape: the
    // collinear fallback.
    if shape_to_be_contained.dimension() < 1 {
        // Java :761-764 — a completed room already surrounds it.
        return;
    }
    let mut cut_line: Option<Line> = None;
    for i in 0..border_count {
        let segment = simplex_border_segment(&obstacle_simplex, i);
        if room.shape.is_intersected_interior_by(&segment) {
            let current_line = obstacle_simplex.border_line(i);
            if shape_to_be_contained.side_of_line(&current_line) == Side::Collinear {
                cut_line = Some(current_line.opposite());
                break;
            }
        }
    }
    let Some(cut_line) = cut_line else {
        // Java :778-782 — no cut line found.
        return;
    };
    let cut_half_plane = TileShape::from_line(&cut_line);
    let new_shape_to_be_contained = shape_to_be_contained.intersection(&cut_half_plane);
    let result_piece = room.shape.intersection(&cut_half_plane);
    if result_piece.dimension() >= 2 {
        out.push(IncompleteRoom {
            shape: result_piece,
            layer,
            contained_shape: new_shape_to_be_contained,
        });
    }
    let opposite_half_plane = TileShape::from_line(&cut_line.opposite());
    let rest_piece = room.shape.intersection(&opposite_half_plane);
    if rest_piece.dimension() >= 2 {
        let rest_shape_to_be_contained = shape_to_be_contained.intersection(&opposite_half_plane);
        let rest_room = IncompleteRoom {
            shape: rest_piece,
            layer,
            contained_shape: rest_shape_to_be_contained,
        };
        restrain_shape_generic_into(&rest_room, obstacle_shape, out);
    }
}

/// Java `ShapeSearchTree.divideLargeRoom`
/// (`ShapeSearchTree.java:1095-1118`) — split a single large room into
/// sections so every layer keeps more than one room for the maze's via
/// handling. Owned Java-parity surface over the in-place core. No
/// in-tree callers (the arms call the `*_in_place` core); kept as the
/// owned Java-parity surface.
#[must_use]
pub fn divide_large_room_generic(
    mut room_list: Vec<IncompleteRoom>,
    board_bbox: &IntBox,
) -> Vec<IncompleteRoom> {
    divide_large_room_in_place(&mut room_list, board_bbox);
    room_list
}

/// The in-place core of `divideLargeRoom` (slice B): same verdicts,
/// same section order, no fresh `Vec` on the identity paths.
fn divide_large_room_in_place(rooms: &mut Vec<IncompleteRoom>, board_bbox: &IntBox) {
    if rooms.len() != 1 {
        return;
    }
    let room_bbox = rooms[0].shape.bounding_box();
    let board_height = board_bbox.ur.y - board_bbox.ll.y;
    let board_width = board_bbox.ur.x - board_bbox.ll.x;
    if 2 * (room_bbox.ur.y - room_bbox.ll.y) <= board_height
        || 2 * (room_bbox.ur.x - room_bbox.ll.x) <= board_width
    {
        return;
    }
    let room = rooms.pop().expect("len checked == 1");
    let max_section_width = 0.5 * f64::from(board_height.max(board_width));
    for section in room.shape.divide_into_sections(max_section_width) {
        let contained = section.intersection(&room.contained_shape);
        rooms.push(IncompleteRoom {
            shape: section,
            layer: room.layer,
            contained_shape: contained,
        });
    }
}

/// Java `ShapeSearchTree90Degree.completeShape`
/// (`ShapeSearchTree90Degree.java:39-195`) — the 90-degree arm: pure
/// IntBox arithmetic over a RAW tree walk. Unlike the base arm (sorted
/// leaf query) the obstacles are processed AT POP TIME in ArrayStack
/// order (push firstChild, push secondChild → second pops first), and
/// the walk prunes against a DYNAMIC `boundingShape` — the union of
/// the surviving room boxes, reassigned after every processed
/// obstacle (Java `:64` write, read at the `:88` prune of later
/// nodes). The port reproduces the stack order and the dynamic hull
/// exactly; the prune READS state the walk writes mid-flight, so
/// neither is omissible.
#[must_use]
pub fn complete_shape_ninety_degree<OBJ: CompleteShapeObjects>(
    tree: &SearchTree,
    objects: &OBJ,
    query: &CompleteShapeQuery,
    board_bbox: &IntBox,
) -> Vec<IncompleteRoom> {
    // Java :41-46 — the contained shape must be an IntBox (warn +
    // empty otherwise).
    let Some(shape_to_be_contained) = (match query.contained {
        Some(TileShape::RegularTileShape(RegularTileShape::IntBox(shape))) => Some(*shape),
        Some(_) | None => None,
    }) else {
        return Vec::new();
    };
    let Some(root) = tree.min_area_tree().root() else {
        // Java :47-50 — empty tree, empty result.
        return Vec::new();
    };

    // Java :51-62 — startShape = board bbox, intersected with the room
    // shape, which must be an IntBox (warn + empty otherwise).
    let mut start_shape = *board_bbox;
    if let Some(room_shape) = query.room_shape {
        let TileShape::RegularTileShape(RegularTileShape::IntBox(room_box)) = room_shape else {
            return Vec::new();
        };
        start_shape = room_box.intersection(board_bbox);
    }
    // Java :64 — the dynamic pruning hull.
    let mut bounding_shape = start_shape;

    // The walk runs inside the per-thread scratch pool (slice B): the
    // room double buffer is swapped per obstacle, the DFS stack is
    // pooled, and the final list is moved out. Buffers are cleared
    // before every fill — the reuse is invisible (the in-file reuse
    // pin + the compares witness it).
    with_room_buffers(|bufs| {
        with_node_stack(|stack| {
            let RoomBuffers { current, next } = bufs;
            let arena = tree.min_area_tree();

            // Java :71-73 — the seed room is added UNCONDITIONALLY (no
            // dimension gate, unlike the base arm's `dimension() == 2` check).
            current.clear();
            current.push(IncompleteRoom {
                shape: box_tile(&start_shape),
                layer: query.layer,
                contained_shape: box_tile(&shape_to_be_contained),
            });

            stack.clear();
            stack.push(root);
            while let Some(idx) = stack.pop() {
                let (node_bounds, leaf_payload) = match &arena.node(idx).kind {
                    NodeKind::Leaf {
                        object_key,
                        shape_index_in_object,
                        bounds,
                    } => (bounds.clone(), Some((*object_key, *shape_index_in_object))),
                    NodeKind::Inner { bounds, .. } => (bounds.clone(), None),
                };
                // Java :88 — prune whole subtrees against the dynamic hull.
                if !bounds_intersect(&node_bounds, &RegularTileShape::IntBox(bounding_shape)) {
                    continue;
                }
                let Some((object_key, shape_index)) = leaf_payload else {
                    // Java :183-185 — push firstChild then secondChild; the LIFO
                    // pop visits second first.
                    if let NodeKind::Inner { first, second, .. } = &arena.node(idx).kind {
                        stack.push(*first);
                        stack.push(*second);
                    }
                    continue;
                };

                // Java :93-96 — the inline filters.
                if !objects.is_trace_obstacle(object_key, query.net_number)
                    || objects.shape_layer(object_key, shape_index) != query.layer
                    || query.ignore_object == Some(object_key)
                {
                    continue;
                }
                // Java :101 — the obstacle shape is its tree shape's bounding
                // BOX (the 90-degree tree stores rectilinear dilations).
                let Some(object_tile) = objects.tree_shape(object_key, shape_index) else {
                    // Java NPE arm — unreachable for live entries.
                    continue;
                };
                let object_shape = object_tile.bounding_box();

                // Java :103-172 — restrain every current room against the
                // obstacle. Java re-unions the WHOLE `newResult` into
                // `newBoundingShape` after every restraining room and the kept
                // room in the non-overlap arm; union (min/max box) is
                // associative, commutative and idempotent, so accumulating each
                // surviving piece exactly once yields the identical final hull.
                // `next` is the Java `newResult`: cleared per obstacle,
                // swapped into `current` after it (capacity retained).
                next.clear();
                let mut new_bounding_shape = IntBox::EMPTY;
                for room in current.iter() {
                    let TileShape::RegularTileShape(RegularTileShape::IntBox(current_shape)) =
                        room.shape
                    else {
                        unreachable!("90-degree completion rooms always carry IntBox shapes");
                    };
                    if !current_shape.overlaps(&object_shape) {
                        // Java :160-165 — KEEP_NON_OVERLAP.
                        next.push(room.clone());
                    } else {
                        // Java :111-127 — the completed-room ignore arm: drop the
                        // room when its overlap with the completed room lies
                        // inside ignoreShape (note the ABSENT
                        // `intersection.dimension() == 2` gate the base arm has —
                        // `IntBox.overlaps` is the only over here).
                        let mut skip = false;
                        if objects.is_complete_free_space(object_key)
                            && let Some(ignore_shape) = query.ignore_shape
                        {
                            let intersection = current_shape.intersection(&object_shape);
                            if ignore_shape.contains_tile(&box_tile(&intersection)) {
                                skip = true;
                            }
                        }
                        if !skip {
                            // Java :134-137 — RESTRAIN (the room does not survive
                            // alongside its pieces).
                            restrain_shape_ninety_degree_into(room, &object_shape, next);
                        }
                    }
                    let piece = next
                        .last()
                        .expect("each arm (except the ignore skip) just pushed")
                        .shape
                        .bounding_box();
                    new_bounding_shape = new_bounding_shape.union(&piece);
                }
                // Java :174-175 — the COMPLETE_SHAPE_BLOCKED trace fires when an
                // obstacle removed every room; diagnostic only, omitted.
                std::mem::swap(current, next);
                bounding_shape = new_bounding_shape;
            }
            // Java :186 — returned DIRECTLY: no divideLargeRoom in the
            // 90-degree override.
            std::mem::take(current)
        })
    })
}

/// Java `ShapeSearchTree90Degree.restrainShape`
/// (`:196-320`) — cut the box-typed room against the obstacle box.
/// The four directional arms take the FURTHEST cut line (integer
/// compare starting at 0 — the base arm starts at -1.0) and yield ONE
/// restrained box; the fallback splits the room at the obstacle edge
/// the contained shape touches and RECURSES on the far piece.
/// Allocation-free core in [`restrain_shape_ninety_degree_into`]; this
/// owned form is the Java-parity surface. No in-tree callers (the arm
/// calls the `*_into` core); kept as the owned Java-parity surface.
#[must_use]
pub fn restrain_shape_ninety_degree(
    room: &IncompleteRoom,
    obstacle_shape: &IntBox,
) -> Vec<IncompleteRoom> {
    let mut result = Vec::new();
    restrain_shape_ninety_degree_into(room, obstacle_shape, &mut result);
    result
}

/// The appending core of [`restrain_shape_ninety_degree`] (slice B):
/// pushes the same rooms in the same order the owned form returns,
/// without the per-call `Vec`.
#[allow(unused_assignments)] // Java :302-310 keeps the running max across all four arms; the LAST arm's store is dead in Java too.
fn restrain_shape_ninety_degree_into(
    room: &IncompleteRoom,
    obstacle_shape: &IntBox,
    out: &mut Vec<IncompleteRoom>,
) {
    // Java :199-204 — the empty-contained guard (null excluded
    // upstream).
    if room.contained_shape.is_empty() {
        return;
    }
    // Java :206-207 — box casts via bounding boxes.
    let room_shape = room.shape.bounding_box();
    let shape_to_be_contained = room.contained_shape.bounding_box();
    let layer = room.layer;

    // Java :209-268 — the four directional cut arms. Each fires when
    // that obstacle edge's segment strictly intersects the room
    // interior; the furthest cut line wins.
    let mut cut_line_distance = 0i32;
    let mut restrained_shape: Option<IntBox> = None;

    // The right edge segment of the obstacle.
    if room_shape.ll.x < obstacle_shape.ur.x
        && room_shape.ur.x > obstacle_shape.ur.x
        && room_shape.ur.y > obstacle_shape.ll.y
        && room_shape.ll.y < obstacle_shape.ur.y
    {
        let current_distance = shape_to_be_contained.ll.x - obstacle_shape.ur.x;
        if current_distance > cut_line_distance {
            cut_line_distance = current_distance;
            restrained_shape = Some(IntBox::new(
                IntPoint::new(obstacle_shape.ur.x, room_shape.ll.y),
                IntPoint::new(room_shape.ur.x, room_shape.ur.y),
            ));
        }
    }
    // The left edge segment.
    if room_shape.ll.x < obstacle_shape.ll.x
        && room_shape.ur.x > obstacle_shape.ll.x
        && room_shape.ur.y > obstacle_shape.ll.y
        && room_shape.ll.y < obstacle_shape.ur.y
    {
        let current_distance = obstacle_shape.ll.x - shape_to_be_contained.ur.x;
        if current_distance > cut_line_distance {
            cut_line_distance = current_distance;
            restrained_shape = Some(IntBox::new(
                IntPoint::new(room_shape.ll.x, room_shape.ll.y),
                IntPoint::new(obstacle_shape.ll.x, room_shape.ur.y),
            ));
        }
    }
    // The lower edge segment.
    if room_shape.ll.y < obstacle_shape.ll.y
        && room_shape.ur.y > obstacle_shape.ll.y
        && room_shape.ur.x > obstacle_shape.ll.x
        && room_shape.ll.x < obstacle_shape.ur.x
    {
        let current_distance = obstacle_shape.ll.y - shape_to_be_contained.ur.y;
        if current_distance > cut_line_distance {
            cut_line_distance = current_distance;
            restrained_shape = Some(IntBox::new(
                IntPoint::new(room_shape.ll.x, room_shape.ll.y),
                IntPoint::new(room_shape.ur.x, obstacle_shape.ll.y),
            ));
        }
    }
    // The upper edge segment.
    if room_shape.ll.y < obstacle_shape.ur.y
        && room_shape.ur.y > obstacle_shape.ur.y
        && room_shape.ur.x > obstacle_shape.ll.x
        && room_shape.ll.x < obstacle_shape.ur.x
    {
        let current_distance = shape_to_be_contained.ll.y - obstacle_shape.ur.y;
        if current_distance > cut_line_distance {
            cut_line_distance = current_distance;
            restrained_shape = Some(IntBox::new(
                IntPoint::new(room_shape.ll.x, obstacle_shape.ur.y),
                IntPoint::new(room_shape.ur.x, room_shape.ur.y),
            ));
        }
    }
    if let Some(restrained) = restrained_shape {
        out.push(IncompleteRoom {
            shape: box_tile(&restrained),
            layer,
            contained_shape: box_tile(&shape_to_be_contained),
        });
        return;
    }

    // Java :272-281 — the contained shape intersects the obstacle: the
    // room (and the contained shape) split in two at the touched
    // obstacle edge.
    let is = shape_to_be_contained.intersection(obstacle_shape);
    if box_tile(&is).is_empty() {
        // Java :277-280 — FRLogger.warn, empty result.
        return;
    }
    let mut new_shape1: Option<IntBox> = None;
    let mut new_shape2: Option<IntBox> = None;
    if is.ll.x > room_shape.ll.x && is.ll.x == obstacle_shape.ll.x && is.ll.x < room_shape.ur.x {
        new_shape1 = Some(IntBox::new(
            IntPoint::new(room_shape.ll.x, room_shape.ll.y),
            IntPoint::new(is.ll.x, room_shape.ur.y),
        ));
        new_shape2 = Some(IntBox::new(
            IntPoint::new(is.ll.x, room_shape.ll.y),
            IntPoint::new(room_shape.ur.x, room_shape.ur.y),
        ));
    } else if is.ur.x > room_shape.ll.x
        && is.ur.x == obstacle_shape.ur.x
        && is.ur.x < room_shape.ur.x
    {
        new_shape2 = Some(IntBox::new(
            IntPoint::new(room_shape.ll.x, room_shape.ll.y),
            IntPoint::new(is.ur.x, room_shape.ur.y),
        ));
        new_shape1 = Some(IntBox::new(
            IntPoint::new(is.ur.x, room_shape.ll.y),
            IntPoint::new(room_shape.ur.x, room_shape.ur.y),
        ));
    } else if is.ll.y > room_shape.ll.y
        && is.ll.y == obstacle_shape.ll.y
        && is.ll.y < room_shape.ur.y
    {
        new_shape1 = Some(IntBox::new(
            IntPoint::new(room_shape.ll.x, room_shape.ll.y),
            IntPoint::new(room_shape.ur.x, is.ll.y),
        ));
        new_shape2 = Some(IntBox::new(
            IntPoint::new(room_shape.ll.x, is.ll.y),
            IntPoint::new(room_shape.ur.x, room_shape.ur.y),
        ));
    } else if is.ur.y > room_shape.ll.y
        && is.ur.y == obstacle_shape.ur.y
        && is.ur.y < room_shape.ur.y
    {
        new_shape2 = Some(IntBox::new(
            IntPoint::new(room_shape.ll.x, room_shape.ll.y),
            IntPoint::new(room_shape.ur.x, is.ur.y),
        ));
        new_shape1 = Some(IntBox::new(
            IntPoint::new(room_shape.ll.x, is.ur.y),
            IntPoint::new(room_shape.ur.x, room_shape.ur.y),
        ));
    }
    if let (Some(new_shape1), Some(new_shape2)) = (new_shape1, new_shape2) {
        // Java :300-318 — keep the near piece only when its share of the
        // contained shape is non-degenerate; the far piece recurses (its
        // contained share may be EMPTY — Java passes it through).
        let new_shape_to_be_contained = shape_to_be_contained.intersection(&new_shape1);
        if box_tile(&new_shape_to_be_contained).dimension() > 0 {
            out.push(IncompleteRoom {
                shape: box_tile(&new_shape1),
                layer,
                contained_shape: box_tile(&new_shape_to_be_contained),
            });
            let new_incomplete_room = IncompleteRoom {
                shape: box_tile(&new_shape2),
                layer,
                contained_shape: box_tile(&shape_to_be_contained.intersection(&new_shape2)),
            };
            restrain_shape_ninety_degree_into(&new_incomplete_room, obstacle_shape, out);
        }
    }
}

/// Java `ShapeSearchTree45Degree.completeShape`
/// (`ShapeSearchTree45Degree.java:96-281`) — the 45-degree arm.
#[must_use]
pub fn complete_shape_fortyfive_degree<OBJ: CompleteShapeObjects>(
    tree: &SearchTree,
    objects: &OBJ,
    query: &CompleteShapeQuery,
    board_bbox: &IntBox,
) -> Vec<IncompleteRoom> {
    let Some(contained_raw) = query.contained else {
        // Java :102-107 — FRLogger.warn, empty result.
        return Vec::new();
    };
    // Java :108-115 — a non-octagon contained shape is approximated by
    // its bounding octagon (debug only; the S3/T3 capture rows show the
    // approximation).
    let Some(shape_to_be_contained) = contained_raw.bounding_octagon() else {
        // Java :117-126 — degenerate contained shape, empty result.
        return Vec::new();
    };
    let Some(root) = tree.min_area_tree().root() else {
        // Java :128-130 — empty tree (the S4 capture row).
        return Vec::new();
    };

    // Java :132-140 — startShape = board bbox octagon, intersected with
    // the ROOM shape's octagon. The room shape MUST be an IntOctagon
    // (warn + empty otherwise — the S5 capture row pins this guard;
    // an octagon's boundingOctagon() is itself).
    let mut start_shape = TileShape::RegularTileShape(RegularTileShape::IntBox(*board_bbox))
        .bounding_octagon()
        .expect("an IntBox always has a bounding octagon");
    if let Some(room_shape) = query.room_shape {
        let Some(room_oct) = shape_octagon(room_shape) else {
            return Vec::new();
        };
        start_shape = room_oct.intersection(&start_shape);
    }

    // The walk runs inside the per-thread scratch pool (slice B): the
    // room double buffer is swapped per obstacle, the DFS stack is
    // pooled, the restrain appends in place, and the final list is
    // moved out. Buffers are cleared before every fill — the reuse is
    // invisible (the in-file reuse pin + the compares witness it).
    with_room_buffers(|bufs| {
        with_node_stack(|stack| {
            let RoomBuffers { current, next } = bufs;
            let arena = tree.min_area_tree();

            // Java :146-147 — the seed room is added UNCONDITIONALLY (even a
            // degenerate start), unlike the base arm's dimension check.
            current.clear();
            current.push(IncompleteRoom {
                shape: oct_tile(&start_shape),
                layer: query.layer,
                contained_shape: oct_tile(&shape_to_be_contained),
            });

            // Java :148-274 — the DFS walk. Leaves are processed AT POP TIME in
            // ArrayStack order (push firstChild, push secondChild → second pops
            // first); the pooled Vec reproduces that LIFO order exactly.
            stack.clear();
            stack.push(root);
            while let Some(idx) = stack.pop() {
                let (node_bounds, leaf_payload) = match &arena.node(idx).kind {
                    NodeKind::Leaf {
                        object_key,
                        shape_index_in_object,
                        bounds,
                    } => (bounds.clone(), Some((*object_key, *shape_index_in_object))),
                    NodeKind::Inner { bounds, .. } => (bounds.clone(), None),
                };
                if !bounds_intersect(&node_bounds, &RegularTileShape::IntOctagon(start_shape)) {
                    continue;
                }
                let Some((object_key, shape_index)) = leaf_payload else {
                    // Java :270-271 — push firstChild then secondChild; the
                    // LIFO pop visits second first.
                    if let NodeKind::Inner { first, second, .. } = &arena.node(idx).kind {
                        stack.push(*first);
                        stack.push(*second);
                    }
                    continue;
                };

                if !objects.is_trace_obstacle(object_key, query.net_number)
                    || objects.shape_layer(object_key, shape_index) != query.layer
                    || query.ignore_object == Some(object_key)
                {
                    continue;
                }
                let Some(object_tile) = objects.tree_shape(object_key, shape_index) else {
                    // Java NPE arm — unreachable for live entries.
                    continue;
                };
                // Java :180-181 — the obstacle enters octagon arithmetic.
                let Some(object_shape) = object_tile.bounding_octagon() else {
                    continue;
                };

                // Java :186-264 — restrain every current room.
                // `next` is the Java `newResult`: cleared per obstacle,
                // swapped into `current` after it (capacity retained).
                next.clear();
                for room in current.iter() {
                    let current_shape = shape_octagon(&room.shape)
                        .expect("45-degree completeShape rooms always carry IntOctagon shapes");
                    if !current_shape.overlaps(&object_shape) {
                        // Java :233-246 — KEEP_NON_OVERLAP.
                        next.push(room.clone());
                        continue;
                    }
                    // Java :193-215 — the completed-room ignore arm: rooms whose
                    // intersection with the obstacle lies inside ignoreShape are
                    // kept ONLY if the room itself is not swallowed too.
                    if objects.is_complete_free_space(object_key)
                        && let Some(ignore_shape) = query.ignore_shape
                    {
                        let intersection = current_shape.intersection(&object_shape);
                        if ignore_shape.contains_tile(&oct_tile(&intersection)) {
                            if !ignore_shape.contains_tile(&room.shape) {
                                next.push(room.clone());
                            }
                            continue;
                        }
                    }
                    // Java :226-232 — RESTRAIN (the room does not survive
                    // alongside its pieces).
                    restrain_shape_fortyfive_degree_into(room, &object_shape, next);
                }
                // Java :248-262 — the COMPLETE_SHAPE_BLOCKED trace fires when an
                // obstacle removed every room; diagnostic only, omitted.
                std::mem::swap(current, next);
            }

            // Java :276-280 — divide, then drop rooms whose shape is contained
            // in their own contained shape (endless-loop guard). Both in
            // place; the result leaves the pool.
            divide_large_room_fortyfive_in_place(current, board_bbox);
            current.retain(|room| !room.contained_shape.contains_tile(&room.shape));
            std::mem::take(current)
        })
    })
}

/// Java `ShapeSearchTree45Degree.divideLargeRoom`
/// (`:288-298`) — the base split, then every result shape (and
/// contained shape) converted to its bounding octagon. The `None` arms
/// of the conversions are unreachable (Java would store null and NPE
/// later): the original shape is kept there. Owned Java-parity surface
/// over the in-place core. No in-tree callers (the 45-degree arm calls
/// the `*_in_place` core); kept as the owned Java-parity surface.
#[must_use]
pub fn divide_large_room_fortyfive_degree(
    mut room_list: Vec<IncompleteRoom>,
    board_bbox: &IntBox,
) -> Vec<IncompleteRoom> {
    divide_large_room_fortyfive_in_place(&mut room_list, board_bbox);
    room_list
}

/// The in-place core of the 45-degree `divideLargeRoom` (slice B):
/// same transforms in the same order, no fresh `Vec`.
fn divide_large_room_fortyfive_in_place(rooms: &mut Vec<IncompleteRoom>, board_bbox: &IntBox) {
    divide_large_room_in_place(rooms, board_bbox);
    for room in rooms.iter_mut() {
        if let Some(oct) = room.shape.bounding_octagon() {
            room.shape = oct_tile(&oct);
        }
        if let Some(oct) = room.contained_shape.bounding_octagon() {
            room.contained_shape = oct_tile(&oct);
        }
    }
}

/// Java `ShapeSearchTree45Degree.restrainShape`
/// (`:305-418`) — pure IntOctagon integer arithmetic. Allocation-free
/// core in [`restrain_shape_fortyfive_degree_into`]; this owned form
/// is the Java-parity surface. No in-tree callers (the arm calls the
/// `*_into` core); kept as the owned Java-parity surface.
#[must_use]
pub fn restrain_shape_fortyfive_degree(
    room: &IncompleteRoom,
    obstacle_shape: &IntOctagon,
) -> Vec<IncompleteRoom> {
    let mut result = Vec::new();
    restrain_shape_fortyfive_degree_into(room, obstacle_shape, &mut result);
    result
}

/// The appending core of [`restrain_shape_fortyfive_degree`] (slice
/// B): pushes the same rooms in the same order the owned form returns,
/// without the per-call `Vec`.
fn restrain_shape_fortyfive_degree_into(
    room: &IncompleteRoom,
    obstacle_shape: &IntOctagon,
    out: &mut Vec<IncompleteRoom>,
) {
    // Java :317-334 — the contained shape must convert to an octagon.
    // IntOctagon converts via its own bounding octagon (identity);
    // Simplex via approximation; a BOX is the "incompatible shape type"
    // arm (unreachable through the 45-degree engine, which seeds
    // octagon contained shapes). The empty check (:318-321) precedes
    // the conversion arms.
    if room.contained_shape.is_empty() {
        return;
    }
    let shape_to_be_contained: Option<IntOctagon> = match &room.contained_shape {
        TileShape::RegularTileShape(RegularTileShape::IntOctagon(oct)) => Some(*oct),
        TileShape::Simplex(_) => room.contained_shape.bounding_octagon(),
        TileShape::RegularTileShape(RegularTileShape::IntBox(_)) => None,
    };
    let Some(shape_to_be_contained) = shape_to_be_contained else {
        // Java :327-330 — unbounded Simplex approximation.
        return;
    };

    // Java :336-348 — the room shape likewise.
    let room_shape: Option<IntOctagon> = match &room.shape {
        TileShape::RegularTileShape(RegularTileShape::IntOctagon(oct)) => Some(*oct),
        TileShape::Simplex(_) => room.shape.bounding_octagon(),
        TileShape::RegularTileShape(RegularTileShape::IntBox(_)) => None,
    };
    let Some(room_shape) = room_shape else {
        // Java :341-344 — unbounded Simplex approximation.
        return;
    };

    // Java :350-362 — the restraining border line: furthest from the
    // contained shape (double compare, 0.5 diagonal factors) among the
    // lines whose segment touches the room interior. The touch check is
    // only evaluated when the distance improves (Java's nested if).
    let mut cut_line_distance = -1.0f64;
    let mut restraining_line_no = -1i32;
    for obstacle_line_no in 0..8 {
        let current_distance =
            signed_line_distance(obstacle_shape, obstacle_line_no, &shape_to_be_contained);
        if current_distance > cut_line_distance
            && obstacle_segment_touches_inside(obstacle_shape, obstacle_line_no, &room_shape)
        {
            cut_line_distance = current_distance;
            restraining_line_no = obstacle_line_no;
        }
    }
    if cut_line_distance >= 0.0 {
        // Java :363-370 — single outside room, NO dimension check.
        let restrained_shape =
            calc_outside_restrained_shape(obstacle_shape, restraining_line_no, &room_shape);
        out.push(IncompleteRoom {
            shape: oct_tile(&restrained_shape),
            layer: room.layer,
            contained_shape: oct_tile(&shape_to_be_contained),
        });
        return;
    }

    // Java :372-394 — the collinear fallback.
    if octagon_dimension(&shape_to_be_contained) < 1 {
        // Java :375-378 — a completed room already surrounds it.
        return;
    }
    let mut restraining_line_no: i32 = -1;
    for obstacle_line_no in 0..8 {
        if obstacle_segment_touches_inside(obstacle_shape, obstacle_line_no, &room_shape) {
            let current_line = obstacle_shape.border_line(obstacle_line_no);
            if octagon_side_of(&shape_to_be_contained, &current_line) == Side::Collinear {
                restraining_line_no = obstacle_line_no;
                break;
            }
        }
    }
    if restraining_line_no < 0 {
        // Java :391-395 — no cut line found.
        return;
    }

    // Java :396-405 — the outside piece (2-dim only, and only kept when
    // the contained shape survives with positive dimension).
    let restrained_shape =
        calc_outside_restrained_shape(obstacle_shape, restraining_line_no, &room_shape);
    if octagon_dimension(&restrained_shape) == 2 {
        let new_shape_to_be_contained = shape_to_be_contained.intersection(&restrained_shape);
        if octagon_dimension(&new_shape_to_be_contained) > 0 {
            out.push(IncompleteRoom {
                shape: oct_tile(&restrained_shape),
                layer: room.layer,
                contained_shape: oct_tile(&new_shape_to_be_contained),
            });
        }
    }

    // Java :407-416 — the inside rest piece recurses.
    let rest_piece = calc_inside_restrained_shape(obstacle_shape, restraining_line_no, &room_shape);
    if octagon_dimension(&rest_piece) >= 2 {
        let rest_shape_to_be_contained = shape_to_be_contained.intersection(&rest_piece);
        if octagon_dimension(&rest_shape_to_be_contained) >= 0 {
            let rest_room = IncompleteRoom {
                shape: oct_tile(&rest_piece),
                layer: room.layer,
                contained_shape: oct_tile(&rest_shape_to_be_contained),
            };
            restrain_shape_fortyfive_degree_into(&rest_room, obstacle_shape, out);
        }
    }
}

/// Java `ShapeSearchTree45Degree.signedLineDistance`
/// (`:67-86`). The diagonal cases carry a 0.5 factor "to prefer
/// orthogonal lines slightly to diagonal restraining lines".
fn signed_line_distance(obstacle: &IntOctagon, line_no: i32, contained: &IntOctagon) -> f64 {
    match line_no {
        0 => f64::from(obstacle.bottom_y - contained.top_y),
        2 => f64::from(contained.left_x - obstacle.right_x),
        4 => f64::from(contained.bottom_y - obstacle.top_y),
        6 => f64::from(obstacle.left_x - contained.right_x),
        1 => 0.5 * f64::from(contained.upper_left_diagonal_x - obstacle.lower_right_diagonal_x),
        3 => 0.5 * f64::from(contained.lower_left_diagonal_x - obstacle.upper_right_diagonal_x),
        5 => 0.5 * f64::from(obstacle.upper_left_diagonal_x - contained.lower_right_diagonal_x),
        7 => 0.5 * f64::from(obstacle.lower_left_diagonal_x - contained.upper_right_diagonal_x),
        _ => 0.0, // Java warns; unreachable in the 0..8 walks.
    }
}

/// Java `ShapeSearchTree45Degree.obstacleSegmentTouchesInside`
/// (`:38-65`): the obstacle's border-line segment with index `line_no`
/// lies inside `room` iff the segment's start corner is on-the-left of
/// the NEXT 5 border lines and its end corner on-the-left of the
/// PREVIOUS 3.
fn obstacle_segment_touches_inside(obstacle: &IntOctagon, line_no: i32, room: &IntOctagon) -> bool {
    let start_corner = obstacle.corner(line_no);
    let mut current_line_no = line_no;
    for _ in 0..5 {
        if room.side_of_border_line(start_corner.x, start_corner.y, current_line_no)
            != Side::Positive
        {
            return false;
        }
        current_line_no = (current_line_no + 1) % 8;
    }
    let end_corner = obstacle.corner((line_no + 1) % 8);
    let mut current_line_no = (line_no + 5) % 8;
    for _ in 0..3 {
        if room.side_of_border_line(end_corner.x, end_corner.y, current_line_no) != Side::Positive {
            return false;
        }
        current_line_no = (current_line_no + 1) % 8;
    }
    true
}

/// Java `ShapeSearchTree45Degree.calcOutsideRestrainedShape`
/// (`:424-452`): the room clipped by the OUTSIDE half plane of border
/// line `line_no`.
fn calc_outside_restrained_shape(
    obstacle: &IntOctagon,
    line_no: i32,
    room: &IntOctagon,
) -> IntOctagon {
    let mut lx = room.left_x;
    let mut ly = room.bottom_y;
    let mut rx = room.right_x;
    let mut uy = room.top_y;
    let mut ulx = room.upper_left_diagonal_x;
    let mut lrx = room.lower_right_diagonal_x;
    let mut llx = room.lower_left_diagonal_x;
    let mut urx = room.upper_right_diagonal_x;
    match line_no {
        0 => uy = obstacle.bottom_y,
        2 => lx = obstacle.right_x,
        4 => ly = obstacle.top_y,
        6 => rx = obstacle.left_x,
        1 => ulx = obstacle.lower_right_diagonal_x,
        3 => llx = obstacle.upper_right_diagonal_x,
        5 => lrx = obstacle.upper_left_diagonal_x,
        7 => urx = obstacle.lower_left_diagonal_x,
        _ => {} // Java warns; unreachable in the 0..8 walks.
    }
    IntOctagon::new(lx, ly, rx, uy, ulx, lrx, llx, urx).normalize()
}

/// Java `ShapeSearchTree45Degree.calcInsideRestrainedShape`
/// (`:458-486`): the room clipped by the INSIDE half plane.
fn calc_inside_restrained_shape(
    obstacle: &IntOctagon,
    line_no: i32,
    room: &IntOctagon,
) -> IntOctagon {
    let mut lx = room.left_x;
    let mut ly = room.bottom_y;
    let mut rx = room.right_x;
    let mut uy = room.top_y;
    let mut ulx = room.upper_left_diagonal_x;
    let mut lrx = room.lower_right_diagonal_x;
    let mut llx = room.lower_left_diagonal_x;
    let mut urx = room.upper_right_diagonal_x;
    match line_no {
        0 => ly = obstacle.bottom_y,
        2 => rx = obstacle.right_x,
        4 => uy = obstacle.top_y,
        6 => lx = obstacle.left_x,
        1 => lrx = obstacle.lower_right_diagonal_x,
        3 => urx = obstacle.upper_right_diagonal_x,
        5 => ulx = obstacle.upper_left_diagonal_x,
        7 => llx = obstacle.lower_left_diagonal_x,
        _ => {} // Java warns; unreachable in the 0..8 walks.
    }
    IntOctagon::new(lx, ly, rx, uy, ulx, lrx, llx, urx).normalize()
}

/// The no-th border segment of a Simplex — Java
/// `LineSegment(PolylineShape, int)` (`LineSegment.java:45-63`):
/// start = previous border line (wrapping), middle = the line, end =
/// next border line (wrapping).
fn simplex_border_segment(simplex: &Simplex, no: i32) -> LineSegment {
    let count = simplex.border_line_count() as i32;
    let start = simplex.border_line(if no == 0 { count - 1 } else { no - 1 });
    let middle = simplex.border_line(no);
    let end = simplex.border_line(if no == count - 1 { 0 } else { no + 1 });
    LineSegment::new(start, middle, end)
}

/// The octagon wrapped as a `TileShape`.
fn oct_tile(octagon: &IntOctagon) -> TileShape {
    TileShape::RegularTileShape(RegularTileShape::IntOctagon(*octagon))
}

/// The box wrapped as a `TileShape`.
fn box_tile(shape: &IntBox) -> TileShape {
    TileShape::RegularTileShape(RegularTileShape::IntBox(*shape))
}

/// The octagon arm of a `TileShape`, `None` for box/simplex.
fn shape_octagon(shape: &TileShape) -> Option<&IntOctagon> {
    match shape {
        TileShape::RegularTileShape(RegularTileShape::IntOctagon(octagon)) => Some(octagon),
        _ => None,
    }
}

/// `IntOctagon` side-of-line through the `TileShape` dispatch.
fn octagon_side_of(octagon: &IntOctagon, line: &Line) -> Side {
    oct_tile(octagon).side_of_line(line)
}

/// `IntOctagon` dimension through the `TileShape` dispatch.
fn octagon_dimension(octagon: &IntOctagon) -> i32 {
    oct_tile(octagon).dimension()
}

#[cfg(test)]
mod tests {
    //! Pins from the M3-T4 spike capture
    //! (`logs/M3-T4/spike_run1.jsonl`, double-run byte-identical). The
    //! tree_shape rows (plain + fortyfive) are re-inserted verbatim so
    //! the obstacle walks see exactly the Java trees' entries
    //! (compensation included for the 45-degree tree). Expected values
    //! are capture-row literals ONLY (pin-failure mode 8).

    use super::*;
    use epic_geometry::int_point::IntPoint;
    use epic_geometry::point::Point;

    fn pt(x: i32, y: i32) -> Point {
        Point::int(IntPoint::new(x, y))
    }

    // ---- the board (meta row: bounds [-1000,-1000,1001000,601000]) ----
    fn board_bbox() -> IntBox {
        IntBox::new(IntPoint::new(-1000, -1000), IntPoint::new(1001000, 601000))
    }
    fn board_box_tile() -> TileShape {
        TileShape::RegularTileShape(RegularTileShape::IntBox(board_bbox()))
    }
    fn board_oct_tile() -> TileShape {
        oct_tile(
            &board_box_tile()
                .bounding_octagon()
                .expect("IntBox bounding octagon"),
        )
    }
    fn box_tile(ll_x: i32, ll_y: i32, ur_x: i32, ur_y: i32) -> TileShape {
        TileShape::RegularTileShape(RegularTileShape::IntBox(IntBox::new(
            IntPoint::new(ll_x, ll_y),
            IntPoint::new(ur_x, ur_y),
        )))
    }
    fn assert_bbox(shape: &TileShape, ll_x: i32, ll_y: i32, ur_x: i32, ur_y: i32) {
        let bbox = shape.bounding_box();
        assert_eq!(
            (bbox.ll.x, bbox.ll.y, bbox.ur.x, bbox.ur.y),
            (ll_x, ll_y, ur_x, ur_y),
            "bounding box mismatch"
        );
    }
    fn assert_oct(shape: &TileShape, coords: [i32; 8]) {
        let oct = shape_octagon(shape).expect("expected IntOctagon shape");
        assert_eq!(
            [
                oct.left_x,
                oct.bottom_y,
                oct.right_x,
                oct.top_y,
                oct.upper_left_diagonal_x,
                oct.lower_right_diagonal_x,
                oct.lower_left_diagonal_x,
                oct.upper_right_diagonal_x
            ],
            coords,
            "octagon coordinate mismatch"
        );
    }

    // The corridor contained shape (S1/T1 head row:
    // contained_bounds [450000,250000,550000,350000]).
    fn corridor() -> TileShape {
        box_tile(450000, 250000, 550000, 350000)
    }
    // The triangle contained shape (S3/T3; oracle t0=(490000,360000)
    // t1=(510000,360000) t2=(500000,380000)).
    fn triangle() -> TileShape {
        TileShape::Simplex(Box::new(Simplex::get_instance(&[
            Line::new(pt(490000, 360000), pt(510000, 360000)),
            Line::new(pt(510000, 360000), pt(500000, 380000)),
            Line::new(pt(500000, 380000), pt(490000, 360000)),
        ])))
    }

    // ---- capture rows: `"row":"tree_shape"` (plain tree) ----
    const PLAIN_OCT_OUTLINE: [[i32; 8]; 4] = [
        [-100, -100, 1000100, 100, -141, 1000141, -141, 1000141],
        [
            999900, -100, 1000100, 600100, 399859, 1000141, 999859, 1600141,
        ],
        [
            -100, 599900, 1000100, 600100, -600141, 400141, 599859, 1600141,
        ],
        [-100, -100, 100, 600100, -600141, 141, -141, 600141],
    ];
    fn plain_grid(x0: i32, y0: i32) -> [[i32; 4]; 16] {
        let mut grid = [[0; 4]; 16];
        let mut i = 0;
        for row in 0..4 {
            for col in 0..4 {
                grid[i] = [
                    x0 + 50000 * col,
                    y0 + 50000 * row,
                    x0 + 50000 * (col + 1),
                    y0 + 50000 * (row + 1),
                ];
                i += 1;
            }
        }
        grid
    }
    fn plain_shapes(object_key: u64) -> Vec<Option<TileShape>> {
        let box_shape = |b: &[i32; 4]| Some(box_tile(b[0], b[1], b[2], b[3]));
        let oct_shape = |o: &[i32; 8]| {
            Some(oct_tile(&IntOctagon::new(
                o[0], o[1], o[2], o[3], o[4], o[5], o[6], o[7],
            )))
        };
        match object_key {
            // Outline: 4 edges duplicated (capture indices 0-3 == 4-7).
            1 => PLAIN_OCT_OUTLINE
                .iter()
                .chain(PLAIN_OCT_OUTLINE.iter())
                .map(oct_shape)
                .collect(),
            // Keepout A tiles: grid x=200000,y=200000 (capture rows
            // item 2, 16 boxes).
            2 => plain_grid(200000, 200000).iter().map(box_shape).collect(),
            // Keepout B tiles: grid x=600000 (capture item 3).
            3 => plain_grid(600000, 200000).iter().map(box_shape).collect(),
            // Keepout C tiles: 2 rows x 3 cols (capture item 4).
            4 => [
                [440000, 480000, 480000, 510000],
                [480000, 480000, 520000, 510000],
                [520000, 480000, 560000, 510000],
                [440000, 510000, 480000, 540000],
                [480000, 510000, 520000, 540000],
                [520000, 510000, 560000, 540000],
            ]
            .iter()
            .map(box_shape)
            .collect(),
            5 => vec![oct_tile(&IntOctagon::new(
                80000, 500000, 120000, 540000, -460000, -380000, 580000, 660000,
            ))]
            .into_iter()
            .map(Some)
            .collect(),
            6 => vec![oct_tile(&IntOctagon::new(
                880000, 60000, 920000, 100000, 780000, 860000, 940000, 1020000,
            ))]
            .into_iter()
            .map(Some)
            .collect(),
            _ => panic!("unknown object key {object_key}"),
        }
    }

    // ---- capture rows: `"row":"tree_shape"` (fortyfive tree) ----
    const OCT_OUTLINE_45: [[i32; 8]; 4] = [
        [-1350, -1350, 1001350, 1350, -1909, 1001909, -1909, 1001909],
        [
            998650, -1350, 1001350, 601350, 398091, 1001909, 998091, 1601909,
        ],
        [
            -1350, 598650, 1001350, 601350, -601909, 401909, 598091, 1601909,
        ],
        [-1350, -1350, 1350, 601350, -601909, 1909, -1909, 601909],
    ];
    const KEEPOUT_A_45: [[i32; 8]; 16] = [
        [
            198750, 198750, 239250, 239250, -40500, 40500, 398232, 478500,
        ],
        [239250, 198750, 279750, 239250, 0, 81000, 438000, 519000],
        [
            279750, 198750, 320250, 239250, 40500, 121500, 478500, 559500,
        ],
        [
            320250, 198750, 360750, 239250, 81000, 162000, 519000, 600000,
        ],
        [
            360750, 198750, 401250, 239250, 121500, 201768, 559500, 640500,
        ],
        [198750, 239250, 239250, 279750, -81000, 0, 438000, 519000],
        [
            239250, 239250, 279750, 279750, -40500, 40500, 478500, 559500,
        ],
        [279750, 239250, 320250, 279750, 0, 81000, 519000, 600000],
        [
            320250, 239250, 360750, 279750, 40500, 121500, 559500, 640500,
        ],
        [
            360750, 239250, 401250, 279750, 81000, 162000, 600000, 681000,
        ],
        [
            198750, 279750, 239250, 320250, -121500, -40500, 478500, 559500,
        ],
        [239250, 279750, 279750, 320250, -81000, 0, 519000, 600000],
        [
            279750, 279750, 320250, 320250, -40500, 40500, 559500, 640500,
        ],
        [320250, 279750, 360750, 320250, 0, 81000, 600000, 681000],
        [
            360750, 279750, 401250, 320250, 40500, 121500, 640500, 721500,
        ],
        [
            198750, 320250, 239250, 360750, -162000, -81000, 519000, 600000,
        ],
    ];
    const KEEPOUT_B_45: [[i32; 8]; 16] = [
        [
            598750, 198750, 639250, 239250, 359500, 440500, 798232, 878500,
        ],
        [
            639250, 198750, 679750, 239250, 400000, 481000, 838000, 919000,
        ],
        [
            679750, 198750, 720250, 239250, 440500, 521500, 878500, 959500,
        ],
        [
            720250, 198750, 760750, 239250, 481000, 562000, 919000, 1000000,
        ],
        [
            760750, 198750, 801250, 239250, 521500, 601768, 959500, 1040500,
        ],
        [
            598750, 239250, 639250, 279750, 319000, 400000, 838000, 919000,
        ],
        [
            639250, 239250, 679750, 279750, 359500, 440500, 878500, 959500,
        ],
        [
            679750, 239250, 720250, 279750, 400000, 481000, 919000, 1000000,
        ],
        [
            720250, 239250, 760750, 279750, 440500, 521500, 959500, 1040500,
        ],
        [
            760750, 239250, 801250, 279750, 481000, 562000, 1000000, 1081000,
        ],
        [
            598750, 279750, 639250, 320250, 278500, 359500, 878500, 959500,
        ],
        [
            639250, 279750, 679750, 320250, 319000, 400000, 919000, 1000000,
        ],
        [
            679750, 279750, 720250, 320250, 359500, 440500, 959500, 1040500,
        ],
        [
            720250, 279750, 760750, 320250, 400000, 481000, 1000000, 1081000,
        ],
        [
            760750, 279750, 801250, 320250, 440500, 521500, 1040500, 1121500,
        ],
        [
            598750, 320250, 639250, 360750, 238000, 319000, 919000, 1000000,
        ],
    ];
    const KEEPOUT_C_45: [[i32; 8]; 6] = [
        [438750, 478750, 479584, 510000, -71250, 834, 918232, 989584],
        [
            479584, 478750, 520418, 510000, -30416, 41668, 958334, 1030418,
        ],
        [
            520418, 478750, 561250, 510000, 10418, 81768, 999168, 1071250,
        ],
        [
            438750, 510000, 479584, 541250, -101768, -30416, 948750, 1020834,
        ],
        [
            479584, 510000, 520418, 541250, -61666, 10418, 989584, 1061668,
        ],
        [
            520418, 510000, 561250, 541250, -20832, 51250, 1030418, 1101768,
        ],
    ];
    fn oct_shapes(coords: &[[i32; 8]]) -> Vec<Option<TileShape>> {
        coords
            .iter()
            .map(|o| {
                Some(oct_tile(&IntOctagon::new(
                    o[0], o[1], o[2], o[3], o[4], o[5], o[6], o[7],
                )))
            })
            .collect()
    }
    fn fortyfive_shapes(object_key: u64) -> Vec<Option<TileShape>> {
        match object_key {
            1 => oct_shapes(&[OCT_OUTLINE_45, OCT_OUTLINE_45].concat()),
            2 => oct_shapes(&KEEPOUT_A_45),
            3 => oct_shapes(&KEEPOUT_B_45),
            4 => oct_shapes(&KEEPOUT_C_45),
            5 => oct_shapes(&[[
                78750, 498750, 121250, 541250, -462500, -377500, 577500, 662500,
            ]]),
            6 => oct_shapes(&[[
                871250, 51250, 928750, 108750, 762500, 877500, 922500, 1037500,
            ]]),
            _ => panic!("unknown object key {object_key}"),
        }
    }

    /// The per-object semantics behind the capture: outline/keepouts are
    /// trace obstacles for every net; each net's OWN pad is not
    /// (Java `Item.isTraceObstacle`); all shapes live on layer 0.
    struct CaptureObjects {
        fortyfive: bool,
    }
    impl CompleteShapeObjects for CaptureObjects {
        fn is_trace_obstacle(&self, object_key: u64, net_number: i32) -> bool {
            match object_key {
                5 => net_number != 1, // NET_A's pad (KA)
                6 => net_number != 2, // NET_B's pad (KB)
                _ => true,
            }
        }
        fn shape_layer(&self, _object_key: u64, _shape_index: u32) -> i32 {
            0
        }
        fn tree_shape(&self, object_key: u64, shape_index: u32) -> Option<TileShape> {
            let shapes = if self.fortyfive {
                fortyfive_shapes(object_key)
            } else {
                plain_shapes(object_key)
            };
            shapes.get(shape_index as usize).cloned().flatten()
        }
        fn is_complete_free_space(&self, _object_key: u64) -> bool {
            false
        }
    }

    fn build_tree(fortyfive: bool) -> SearchTree {
        let mut tree = SearchTree::new(
            if fortyfive {
                SearchTreeVariant::FortyfiveDegree
            } else {
                SearchTreeVariant::Generic
            },
            if fortyfive { 1 } else { 0 },
        );
        for key in 1..=6u64 {
            let shapes = if fortyfive {
                fortyfive_shapes(key)
            } else {
                plain_shapes(key)
            };
            tree.insert(key, &shapes);
        }
        tree
    }

    fn query<'a>(
        room_shape: Option<&'a TileShape>,
        contained: Option<&'a TileShape>,
    ) -> CompleteShapeQuery<'a> {
        CompleteShapeQuery {
            room_shape,
            contained,
            layer: 0,
            net_number: 1,
            ignore_object: None,
            ignore_shape: None,
        }
    }

    /// S1_corridor / S3_simplex — the base arm's maximal free Simplex
    /// around the contained shape (capture rows `complete_shape_room`,
    /// probes S1_corridor + S3_simplex).
    #[test]
    fn base_arm_maximal_free_simplex() {
        let tree = build_tree(false);
        let objects = CaptureObjects { fortyfive: false };

        // S1: seed room = board box; result = the maximal free simplex
        // [400000,100,600000,480000] around the corridor.
        let room = board_box_tile();
        let contained = corridor();
        let q = query(Some(&room), Some(&contained));
        let result = complete_shape_generic(&tree, &objects, &q, &board_bbox());
        assert_eq!(result.len(), 1, "S1_corridor: exactly one room");
        assert!(
            matches!(result[0].shape, TileShape::Simplex(_)),
            "S1_corridor: shape_class Simplex"
        );
        assert_bbox(&result[0].shape, 400000, 100, 600000, 480000);
        assert!(
            matches!(result[0].contained_shape, TileShape::Simplex(_)),
            "S1_corridor: contained_class Simplex (restrain converts)"
        );
        assert_bbox(&result[0].contained_shape, 450000, 250000, 550000, 350000);
        assert_eq!(result[0].layer, 0);

        // S3: simplex contained shape → bounding approximation upstream,
        // result [400000,250000,600000,480000].
        let room = board_box_tile();
        let contained = triangle();
        let q = query(Some(&room), Some(&contained));
        let result = complete_shape_generic(&tree, &objects, &q, &board_bbox());
        assert_eq!(result.len(), 1, "S3_simplex: exactly one room");
        assert_bbox(&result[0].shape, 400000, 250000, 600000, 480000);
        assert_bbox(&result[0].contained_shape, 490000, 360000, 510000, 380000);
    }

    /// S2_null_contained (result_count 0) and S4_empty_tree
    /// (result_count 0) — the base null-contained guard and the 45
    /// empty-tree guard.
    #[test]
    fn guard_arms_return_empty() {
        let tree = build_tree(false);
        let objects = CaptureObjects { fortyfive: false };
        // S2: null contained → empty (Java :585-588).
        let room = board_box_tile();
        let q = query(Some(&room), None);
        assert!(complete_shape_generic(&tree, &objects, &q, &board_bbox()).is_empty());

        // S4: empty 45-degree tree (no inserts) → empty (Java :128-130).
        let empty_tree = SearchTree::new(SearchTreeVariant::FortyfiveDegree, 1);
        let objects45 = CaptureObjects { fortyfive: true };
        let room = board_oct_tile();
        let contained = corridor();
        let q = query(Some(&room), Some(&contained));
        let bbox = board_bbox();
        assert!(complete_shape_fortyfive_degree(&empty_tree, &objects45, &q, &bbox).is_empty());
    }

    /// S5_room_shape_not_octagon (result_count 0) — the 45-degree arm's
    /// room-shape guard (Java :133-140): an IntBox room shape is
    /// rejected with an empty result.
    #[test]
    fn fortyfive_rejects_box_room_shape() {
        let tree = build_tree(true);
        let objects = CaptureObjects { fortyfive: true };
        let room = board_box_tile();
        let contained = corridor();
        let q = query(Some(&room), Some(&contained));
        let bbox = board_bbox();
        let result = complete_shape_fortyfive_degree(&tree, &objects, &q, &bbox);
        assert!(
            result.is_empty(),
            "S5_room_shape_not_octagon: result_count 0"
        );
    }

    /// T1_corridor / T3_simplex — the 45-degree arm's maximal free
    /// IntOctagon rooms (capture rows `complete_shape_room`, probes
    /// T1_corridor + T3_simplex; net NET_B = 2, compensated class-1
    /// shapes).
    #[test]
    fn fortyfive_arm_maximal_free_octagon() {
        let tree = build_tree(true);
        let objects = CaptureObjects { fortyfive: true };

        let room = board_oct_tile();
        let contained = corridor();
        let mut q = query(Some(&room), Some(&contained));
        q.net_number = 2;
        let result = complete_shape_fortyfive_degree(&tree, &objects, &q, &board_bbox());
        assert_eq!(result.len(), 1, "T1_corridor: exactly one room");
        assert!(
            matches!(
                result[0].shape,
                TileShape::RegularTileShape(RegularTileShape::IntOctagon(_))
            ),
            "T1_corridor: shape_class IntOctagon"
        );
        assert_oct(
            &result[0].shape,
            [
                401250, 1350, 598750, 478750, -77500, 597400, 402600, 1077500,
            ],
        );
        assert_oct(
            &result[0].contained_shape,
            [
                450000, 250000, 550000, 350000, 100000, 300000, 700000, 900000,
            ],
        );
        assert_eq!(result[0].layer, 0);

        let room = board_oct_tile();
        let contained = triangle();
        let mut q = query(Some(&room), Some(&contained));
        q.net_number = 2;
        let result = complete_shape_fortyfive_degree(&tree, &objects, &q, &board_bbox());
        assert_eq!(result.len(), 1, "T3_simplex: exactly one room");
        assert_oct(
            &result[0].shape,
            [
                401250, 239250, 598750, 478750, -77500, 359500, 640500, 1077500,
            ],
        );
        assert_oct(
            &result[0].contained_shape,
            [
                490000, 360000, 510000, 380000, 120000, 150000, 850000, 880000,
            ],
        );
    }

    /// The dispatch contrast (pin-failure mode 7): the same contained
    /// shape through both trees produces DIFFERENT arms' results —
    /// plain Simplex vs 45-degree IntOctagon — so an arm-flip in the
    /// dispatcher cannot pass silently. (Each arm gets its legal room
    /// shape: the base arm accepts an IntBox, the 45-degree arm
    /// REJECTS one — that is the S5 guard, not a dispatch artifact.)
    #[test]
    fn dispatch_contrast_plain_vs_fortyfive() {
        let plain = build_tree(false);
        let fortyfive = build_tree(true);
        let plain_room = board_box_tile();
        let fortyfive_room = board_oct_tile();
        let contained = corridor();
        let q = query(Some(&plain_room), Some(&contained));
        let plain_result = complete_shape(
            &plain,
            &CaptureObjects { fortyfive: false },
            &q,
            &board_bbox(),
        );
        let q = query(Some(&fortyfive_room), Some(&contained));
        let fortyfive_result = complete_shape(
            &fortyfive,
            &CaptureObjects { fortyfive: true },
            &q,
            &board_bbox(),
        );
        assert_eq!(plain_result.len(), 1, "plain_result: {plain_result:?}");
        assert!(matches!(plain_result[0].shape, TileShape::Simplex(_)));
        assert_eq!(
            fortyfive_result.len(),
            1,
            "fortyfive_result: {fortyfive_result:?}"
        );
        assert!(matches!(
            fortyfive_result[0].shape,
            TileShape::RegularTileShape(RegularTileShape::IntOctagon(_))
        ));
    }

    /// The scratch pool's cross-query reuse must be invisible (M5
    /// slice B): a SECOND query through the pooled room buffers — after
    /// a first query that left several rooms in them — returns exactly
    /// the fresh-state rooms, and equals a FRESH TREE control run after
    /// it. A stale-buffer mutant (a missing `clear` or `swap`) shows as
    /// duplicated or stale rooms in `second`/`third`. (The buffer
    /// contents never carry state between uses — the pool retains only
    /// capacity.)
    #[test]
    fn scratch_reuse_across_queries_is_invisible() {
        let tree = build_tree(true);
        let objects = CaptureObjects { fortyfive: true };
        let room = board_oct_tile();

        // Query 1 (the T1_corridor world): leaves the walk's pooled
        // buffers populated.
        let contained = corridor();
        let mut q1 = query(Some(&room), Some(&contained));
        q1.net_number = 2;
        let first = complete_shape_fortyfive_degree(&tree, &objects, &q1, &board_bbox());
        assert_eq!(first.len(), 1, "T1_corridor face");
        assert_oct(
            &first[0].shape,
            [
                401250, 1350, 598750, 478750, -77500, 597400, 402600, 1077500,
            ],
        );

        // Query 2 (the T3_simplex world) through the SAME pooled
        // buffers.
        let contained = triangle();
        let mut q2 = query(Some(&room), Some(&contained));
        q2.net_number = 2;
        let second = complete_shape_fortyfive_degree(&tree, &objects, &q2, &board_bbox());
        assert_eq!(second.len(), 1, "T3_simplex face after a prior query");
        assert_oct(
            &second[0].shape,
            [
                401250, 239250, 598750, 478750, -77500, 359500, 640500, 1077500,
            ],
        );

        // The fresh-tree control: the same query on a NEWLY built tree
        // (the pooled buffers are hot here) must equal query 2 exactly.
        let fresh = build_tree(true);
        let third = complete_shape_fortyfive_degree(&fresh, &objects, &q2, &board_bbox());
        assert_eq!(third, second, "pooled reuse equals the fresh state");
    }
}
