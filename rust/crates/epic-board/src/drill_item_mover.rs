//! Shoving vias (and, in Java, pins) aside: the drill-item half of the
//! mutual-recursion shove cycle (Java `board/actions/DrillItemMover.java`,
//! 325 lines, ported in full). The cycle is
//! `TraceShover.check/insert → DrillItemMover.shoveVias/tryShoveViaPoints
//! → ForcedPadRouter.checkForcedPad/forcedPad → TraceShover.check/insert`;
//! every re-entry decrements its recursion budget (the D1 contract).
//!
//! ## Via-only move surface
//!
//! Java's `DrillItem` covers pins and vias, but every T10b-relevant
//! caller passes a VIA: the shove via lists collect only `Via` items
//! (`ShapeTraceEntries.storeItems`). A pin moves with its component
//! placement (`Component.moveComponent` — the T10c board surface), so
//! [`drill_move_by`] and the layer loops here are documented and
//! pinned on the via face; the pin face would need the placement
//! transform first.

use std::cmp::Reverse;

use epic_geometry::int_octagon::IntOctagon;
use epic_geometry::int_point::IntPoint;
use epic_geometry::point::Point;
use epic_geometry::regular_tile_shape::RegularTileShape;
use epic_geometry::tile_shape::TileShape;
use epic_geometry::vector::Vector;

use crate::board::Board;
use crate::contacts::item_normal_contacts;
use crate::forced_pad_router::{CheckDrillResult, check_forced_pad, forced_pad};
use crate::id::ItemId;
use crate::items::{FixedState, ItemData};
use crate::rules_surf::AngleRestriction;
use crate::shape_entry_side::ShapeEntrySide;
use crate::shape_trace_entries::ShapeTraceEntries;
use crate::time_limit::TimeLimit;
use crate::trace_ops::{contains_net, insert_trace, is_shove_fixed, split_clip};
use crate::tree_manager::SearchTreeManager;

/// Java static `check` (`:33-103`): can the drill item be translated by
/// `vector` by shoving obstacle traces and vias aside, without
/// clearance violations? The board is NOT changed.
///
/// Java MUTATES the caller's `ignoreItems` list (the drill item is
/// added to it) — the port mirrors that with `&mut Vec<ItemId>`; the
/// TraceShover callers hand in a fresh list per candidate.
#[allow(clippy::too_many_arguments)] // the Java read set, kept flat
pub fn check(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    drill_id: ItemId,
    vector: &Vector,
    max_recursion_depth: i32,
    max_via_recursion_depth: i32,
    ignore_items: &mut Vec<ItemId>,
    time_limit: Option<&TimeLimit>,
) -> bool {
    if let Some(limit) = time_limit
        && limit.limit_exceeded()
    {
        return false;
    }
    if is_shove_fixed(board, drill_id) {
        return false;
    }

    // Check, that drillitem is only connected to traces (or plane
    // areas — Java allows `Trace || ConductionArea`).
    let contact_list = item_normal_contacts(manager, board, drill_id);
    for current_contact in contact_list {
        let allowed = matches!(
            board.get(current_contact).map(|e| &e.data),
            Some(ItemData::Trace { .. }) | Some(ItemData::ConductionArea { .. })
        );
        if !allowed {
            return false;
        }
    }
    // Java: `effectiveIgnoreItems.add(drillItem)` — the caller's list
    // is MUTATED (the callers pass throwaway lists).
    ignore_items.push(drill_id);

    let attach_allowed = attach_smd_allowed(board, drill_id);
    let tree = manager.default_tree();
    let (tree_oid, tree_variant, tree_class) = (
        tree.object_id(),
        tree.variant,
        tree.compensated_clearance_class,
    );
    let first_layer = board
        .item_first_layer(drill_id)
        .expect("Java NPE: drill item without layers");
    let last_layer = board
        .item_last_layer(drill_id)
        .expect("Java NPE: drill item without layers");
    let drill_nets = item_nets(board, drill_id);
    let drill_class = board
        .item_clearance_class(drill_id)
        .expect("live drill item class");
    let center = board
        .drill_center(drill_id)
        .expect("Java NPE: drill item without a center");
    let is_ninety_degree = board.rules().trace_angle_restriction == AngleRestriction::NinetyDegree;

    let mut current_layer = first_layer;
    while current_layer <= last_layer {
        let current_ind = current_layer - first_layer;
        let Some(current_shape) =
            board.tree_shape_precalc(drill_id, tree_oid, tree_variant, tree_class)
                [current_ind.max(0) as usize]
                .clone()
        else {
            current_layer += 1;
            continue;
        };
        let new_shape = current_shape.translate_by(vector);
        let current_tile_shape = if is_ninety_degree {
            TileShape::RegularTileShape(RegularTileShape::IntBox(new_shape.bounding_box()))
        } else {
            TileShape::RegularTileShape(RegularTileShape::IntOctagon(
                new_shape
                    .bounding_octagon()
                    .expect("bounding octagon of a non-empty tree shape"),
            ))
        };
        let from_side = ShapeEntrySide::from_point(center.clone(), &current_tile_shape);
        if check_forced_pad(
            manager,
            board,
            &current_tile_shape,
            from_side,
            current_layer,
            &drill_nets,
            drill_class,
            attach_allowed,
            ignore_items,
            max_recursion_depth,
            max_via_recursion_depth,
            true,
            time_limit,
        ) == CheckDrillResult::NotDrillable
        {
            return false;
        }
        current_layer += 1;
    }
    true
}

/// Java static `insert` (`:110-167`): translates the drill item by
/// `vector`, shoving obstacles aside — the board IS changed; call
/// [`check`] first.
///
/// The `tidy_region` parameter is a JAVA DEAD-STORE: the body rebinds
/// the local (`tidyRegion = tidyRegion.union(...)`) and never reads it
/// afterwards. The port keeps the parameter for call-signature parity
/// and ignores it.
#[allow(clippy::too_many_arguments)] // the Java read set, kept flat
pub fn insert(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    drill_id: ItemId,
    vector: &Vector,
    max_recursion_depth: i32,
    max_via_recursion_depth: i32,
    _tidy_region: Option<&IntOctagon>, // Java dead-store, see doc
) -> bool {
    if is_shove_fixed(board, drill_id) {
        return false;
    }

    let attach_allowed = attach_smd_allowed(board, drill_id);
    let ignore_items = vec![drill_id];
    let tree = manager.default_tree();
    let (tree_oid, tree_variant, tree_class) = (
        tree.object_id(),
        tree.variant,
        tree.compensated_clearance_class,
    );
    let first_layer = board
        .item_first_layer(drill_id)
        .expect("Java NPE: drill item without layers");
    let last_layer = board
        .item_last_layer(drill_id)
        .expect("Java NPE: drill item without layers");
    let drill_nets = item_nets(board, drill_id);
    let drill_class = board
        .item_clearance_class(drill_id)
        .expect("live drill item class");
    let center = board
        .drill_center(drill_id)
        .expect("Java NPE: drill item without a center");
    let is_ninety_degree = board.rules().trace_angle_restriction == AngleRestriction::NinetyDegree;

    let mut current_layer = first_layer;
    while current_layer <= last_layer {
        let current_ind = current_layer - first_layer;
        let Some(current_shape) =
            board.tree_shape_precalc(drill_id, tree_oid, tree_variant, tree_class)
                [current_ind.max(0) as usize]
                .clone()
        else {
            current_layer += 1;
            continue;
        };
        let new_shape = current_shape.translate_by(vector);
        let current_tile_shape = if is_ninety_degree {
            TileShape::RegularTileShape(RegularTileShape::IntBox(new_shape.bounding_box()))
        } else {
            TileShape::RegularTileShape(RegularTileShape::IntOctagon(
                new_shape
                    .bounding_octagon()
                    .expect("bounding octagon of a non-empty tree shape"),
            ))
        };
        // Java dead-store: `tidyRegion = tidyRegion.union(
        // currentTileShape.boundingOctagon())` — never read again.
        let from_side = ShapeEntrySide::from_point(center.clone(), &current_tile_shape);
        if !forced_pad(
            manager,
            board,
            &current_tile_shape,
            from_side,
            current_layer,
            &drill_nets,
            drill_class,
            attach_allowed,
            &ignore_items,
            max_recursion_depth,
            max_via_recursion_depth,
        ) {
            return false;
        }
        // Java `:159-162`: mark the four corners of the UNTRANSLATED
        // tree shape's bounding box in the changed area (note the
        // asymmetry: forcedPad ran on the translated `currentTileShape`,
        // the join reads `currentShape`). `RoutingBoardOperations
        // .joinChangedArea` guards on the null session internally.
        if board.changed_area.is_some() {
            let current_bounding_box = current_shape.bounding_box();
            for j in 0..4 {
                let corner = current_bounding_box.corner(j).to_float();
                if let Some(area) = board.changed_area.as_mut() {
                    area.join_point(&corner, current_layer);
                }
            }
        }
        current_layer += 1;
    }
    drill_move_by(manager, board, drill_id, vector);
    true
}

/// Java static `shoveVias` (`:173-249`): shoves the vias collected in
/// the shape's via list out of the way. Returns false only when the
/// database is DAMAGED and an undo is necessary — the `true`-on-failure
/// quirks are kept verbatim:
///
/// * a failed `storeItems` returns TRUE (`:186-188`),
/// * an exhausted via recursion budget returns TRUE mid-loop
///   (`:207-209`).
#[allow(clippy::too_many_arguments)] // the Java read set, kept flat
pub fn shove_vias(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    obstacle_shape: &TileShape,
    from_side: Option<&ShapeEntrySide>,
    layer: i32,
    net_numbers: &[i32],
    clearance_class_index: i32,
    ignore_items: &[ItemId],
    max_recursion_depth: i32,
    max_via_recursion_depth: i32,
    copper_sharing_allowed: bool,
) -> bool {
    let mut shape_entries = ShapeTraceEntries::new(
        obstacle_shape.clone(),
        layer,
        net_numbers.to_vec(),
        clearance_class_index,
        from_side.cloned(),
    );
    let obstacles = manager.overlapping_items_with_clearance(
        board,
        0,
        obstacle_shape,
        layer,
        &[],
        clearance_class_index,
    );

    if !shape_entries.store_items(manager, board, &obstacles, false, copper_sharing_allowed) {
        // Java QUIRK: store failure means NOT shovable, but the
        // database is NOT damaged — the answer is still true.
        return true;
    }
    // Java `shoveViaList.removeAll(ignoreItems)` if non-null —
    // order-preserving retain.
    if !ignore_items.is_empty() {
        shape_entries
            .shove_via_list
            .retain(|id| !ignore_items.contains(id));
    }
    if shape_entries.shove_via_list.is_empty() {
        return true;
    }
    let shape_radius = 0.5 * obstacle_shape.bounding_box().min_width();
    let via_list = shape_entries.shove_via_list.clone();
    for via_id in via_list {
        let via_nets = item_nets(board, via_id);
        if crate::contacts::shares_net_no(&via_nets, net_numbers) {
            continue;
        }
        if max_via_recursion_depth <= 0 {
            // Java QUIRK: budget exhausted mid-loop answers true.
            return true;
        }
        let try_via_centers = try_shove_via_points(
            manager,
            board,
            obstacle_shape,
            layer,
            via_id,
            clearance_class_index,
            true,
        );
        let mut new_via_center: Option<IntPoint> = None;
        let first_layer = board
            .item_first_layer(via_id)
            .expect("Java NPE: via without layers");
        // Java `getShapeOnLayer(layer)` — the RAW (absolute-layer)
        // shape, i.e. the relative index `layer - firstLayer` here.
        let via_raw_shape = board
            .drill_shape(via_id, layer - first_layer)
            .expect("Java NPE: null via shape on layer");
        let via_box = crate::trace_shover::shape_bounding_box(&via_raw_shape);
        let max_dist = 0.5 * via_box.max_width() + shape_radius;
        let max_dist_square = max_dist * max_dist;
        let current_via_center = drill_int_center(board, via_id);
        let check_via_center = current_via_center.to_float();
        let mut rel_coor: Option<Vector> = None;
        for (i, try_center) in try_via_centers.iter().enumerate() {
            if i == 0 || check_via_center.distance_square(&try_center.to_float()) <= max_dist_square
            {
                let mut local_ignore_items = ignore_items.to_vec();
                rel_coor =
                    Some(Point::Int(*try_center).difference_by(&Point::Int(current_via_center)));
                // No time limit here because the item database is
                // already changed.
                if check(
                    manager,
                    board,
                    via_id,
                    rel_coor.as_ref().expect("set one line above"),
                    max_recursion_depth,
                    max_via_recursion_depth - 1,
                    &mut local_ignore_items,
                    None,
                ) {
                    new_via_center = Some(*try_center);
                    break;
                }
            }
        }
        let Some(_) = new_via_center else {
            continue;
        };
        if !insert(
            manager,
            board,
            via_id,
            rel_coor.as_ref().expect("Java NPE: relCoor set"),
            max_recursion_depth,
            max_via_recursion_depth - 1,
            None,
        ) {
            return false;
        }
    }
    true
}

/// Java static `tryShoveViaPoints` (`:256-325`): possible new locations
/// for a via to shove outside `obstacle_shape`; more than one when
/// `extended_check` (the TraceShover.check ladder passes `true`, the
/// static max-length probe `false`). Empty when the via has no tree
/// shape on the layer.
///
/// The `+2` on the shove distance is Java's empirical tolerance for
/// diagonal shoving — kept verbatim (`:291`).
#[allow(clippy::too_many_arguments)] // the Java read set, kept flat
pub fn try_shove_via_points(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    obstacle_shape: &TileShape,
    layer: i32,
    via_id: ItemId,
    clearance_class_index: i32,
    extended_check: bool,
) -> Vec<IntPoint> {
    let tree = manager.default_tree();
    let (tree_oid, tree_variant, tree_class) = (
        tree.object_id(),
        tree.variant,
        tree.compensated_clearance_class,
    );
    let compensated = tree.is_clearance_compensation_used();
    let first_layer = board
        .item_first_layer(via_id)
        .expect("Java NPE: via without layers");
    let Some(current_via_shape) =
        board.tree_shape_precalc(via_id, tree_oid, tree_variant, tree_class)
            [(layer - first_layer).max(0) as usize]
            .clone()
    else {
        return Vec::new();
    };
    let is_int_octagon = obstacle_shape.is_int_octagon();
    let via_class = board.item_clearance_class(via_id).expect("live via class");
    let clearance_value = f64::from(board.clearance_value(clearance_class_index, via_class, layer));
    let is_ninety_degree = board.rules().trace_angle_restriction == AngleRestriction::NinetyDegree;
    let mut shove_distance;
    if is_ninety_degree || is_int_octagon {
        shove_distance = 0.5 * current_via_shape.bounding_box().max_width();
        if !compensated {
            shove_distance += clearance_value;
        }
    } else {
        // a different algorithm is used for calculating the new via
        // centers
        shove_distance = 0.0;
        if !compensated {
            // enlarge obstacleShape and currentViaShape by half of the
            // clearance value to synchronize with the check algorithm
            // in ShapeSearchTree.overlapping_tree_entries_with_clearance
            shove_distance += 0.5 * clearance_value;
        }
    }

    // The additional constant 2 is an empirical value for the tolerance
    // in case of diagonal shoving.
    shove_distance += 2.0;

    let current_via_center = drill_int_center(board, via_id);
    let mut try_count = 1;
    if is_ninety_degree {
        let current_offset_box = obstacle_shape.bounding_box().offset(shove_distance);
        if extended_check {
            try_count = 2;
        }
        current_offset_box.nearest_border_projections(&current_via_center, try_count)
    } else if is_int_octagon {
        let current_offset_octagon = obstacle_shape
            .bounding_octagon()
            .expect("bounding octagon of a non-empty shape")
            .enlarge(shove_distance);
        if extended_check {
            try_count = 4;
        }
        current_offset_octagon.nearest_border_projections(&current_via_center, try_count)
    } else {
        let current_offset_shape = obstacle_shape.enlarge(shove_distance);
        let via_shape = if !compensated {
            current_via_shape.enlarge(0.5 * clearance_value)
        } else {
            current_via_shape
        };
        if extended_check {
            try_count = 4;
        }
        let shove_deltas =
            current_offset_shape.nearest_relative_outside_locations(&via_shape, try_count);
        shove_deltas
            .iter()
            .map(|delta| {
                let current_delta = Point::Int(delta.round()).difference_by(&Point::ZERO);
                match Point::Int(current_via_center).translate_by(&current_delta) {
                    Point::Int(p) => p,
                    // Java's cast `(IntPoint) currentViaCenter
                    // .translateBy(currentDelta)` — an IntPoint center
                    // moved by an IntVector difference stays integral.
                    Point::Rational(_) => {
                        panic!("Java ClassCastException: non-integer via center")
                    }
                }
            })
            .collect()
    }
}

/// Java `DrillItem.moveBy` (`DrillItem.java:104-152`) on the via face —
/// the full `Item.moveBy` choreography plus the reconnect traces:
///
/// 1. remember the (layer, half width, class) of every connected trace
///    — Java's `TreeSet<TraceInfo>` dedups BY LAYER ONLY
///    (`TraceInfo.compareTo = other.layer - this.layer`, the
///    first-inserted width/class wins) and iterates DESCENDING layer,
/// 2. `saveForUndo`, tree remove, `translateBy` (+ `clearDerivedData`),
///    tree re-insert (the M2 undo pattern),
/// 3. insert one UNFIXED trace from the old center to the new center
///    per remembered trace info, with the ninety/fortyfive-degree
///    corner inserted between the endpoints when the angle restriction
///    demands it.
pub(crate) fn drill_move_by(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    drill_id: ItemId,
    vector: &Vector,
) {
    let old_center = board
        .drill_center(drill_id)
        .expect("Java NPE: drill item without a center");
    // remember the contact situation of this drillitem to traces on
    // each layer — (layer, halfWidth, class), first-wins per layer,
    // iterated descending by layer.
    let mut contact_trace_info: Vec<(i32, i32, i32)> = Vec::new();
    for current_contact in item_normal_contacts(manager, board, drill_id) {
        let is_trace = matches!(
            board.get(current_contact).map(|e| &e.data),
            Some(ItemData::Trace { .. })
        );
        if is_trace {
            let layer = board
                .trace_layer(current_contact)
                .expect("contact trace layer");
            let half_width = board
                .trace_half_width(current_contact)
                .expect("contact trace half width");
            let class = board
                .item_clearance_class(current_contact)
                .expect("contact trace class");
            // TreeSet.add: compareTo == 0 (same layer) REJECTS the new
            // element — the first-inserted info survives.
            if !contact_trace_info.iter().any(|(l, _, _)| *l == layer) {
                contact_trace_info.push((layer, half_width, class));
            }
        }
    }
    contact_trace_info.sort_by(|a, b| b.0.cmp(&a.0)); // descending layer

    // Item.moveBy: saveForUndo; searchTree remove; translateBy;
    // searchTree insert. DrillItem.translateBy also clears the derived
    // data (the shape-precalc and drill-span memos in the port).
    board.item_undo.save_for_undo(&Reverse(drill_id));
    manager.remove(board, drill_id);
    let new_center = old_center.translate_by(vector);
    let new_center = match new_center {
        Point::Int(p) => p,
        // A Via center is stored as an IntPoint datum (Java stores the
        // Point; every forced-insertion caller is integer).
        Point::Rational(_) => panic!("Java ClassCastException: non-integer via center"),
    };
    board.set_via_center(drill_id, new_center);
    board.clear_derived_data(drill_id);
    manager.insert(board, drill_id);

    // Insert a Trace from the old center to the new center, on all
    // layers, where this DrillItem was connected to a Trace.
    let (old_int, new_int) = match (&old_center, &Point::Int(new_center)) {
        (Point::Int(o), Point::Int(n)) => (*o, *n),
        _ => panic!("Java ClassCastException: non-integer drill centers"),
    };
    let add_corner = match board.rules().trace_angle_restriction {
        AngleRestriction::NinetyDegree => old_int.ninety_degree_corner(&new_int, true),
        AngleRestriction::FortyfiveDegree => old_int.fortyfive_degree_corner(&new_int, true),
        AngleRestriction::None => None,
    };
    let mut connect_points = vec![old_center.clone()];
    if let Some(corner) = add_corner {
        connect_points.push(Point::Int(corner));
    }
    connect_points.push(Point::Int(new_center));
    let drill_nets = item_nets(board, drill_id);
    for (layer, half_width, class) in contact_trace_info {
        insert_trace(
            manager,
            board,
            epic_geometry::polyline::Polyline::from_points(&connect_points),
            layer,
            half_width,
            &drill_nets,
            class,
            FixedState::Unfixed,
        );
    }
}

/// Java `BasicBoard.insertVia` (`BasicBoard.java:269-296`): inserts a
/// via and splits the own-net traces underneath it — on every layer
/// STRICTLY BELOW `toLayer` (`for (int i = fromLayer; i < toLayer;
/// i++)`, `:280` — the top padstack layer never splits). Returns the
/// new via's id.
#[allow(clippy::too_many_arguments)] // the Java read set, kept flat
pub fn insert_via(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    padstack_no: i32,
    center: IntPoint,
    net_numbers: &[i32],
    clearance_class_index: i32,
    fixed_state: FixedState,
    attach_allowed: bool,
) -> ItemId {
    let (from_layer, to_layer) = {
        let padstack = board
            .library()
            .padstack(padstack_no)
            .expect("Java NPE: unknown padstack");
        (padstack.from_layer() as i32, padstack.to_layer())
    };
    let via_id = board.alloc_id();
    board.insert_item(crate::board::ItemEntry {
        id: via_id,
        data: ItemData::Via {
            center,
            padstack_no,
            attach_smd_allowed: attach_allowed,
        },
        nets: net_numbers.to_vec(),
        clearance_class: clearance_class_index,
        component_id: 0,
        fixed: fixed_state,
        on_the_board: false,
    });
    manager.insert(board, via_id);
    let mut layer = from_layer;
    while layer < to_layer {
        for current_net_number in net_numbers {
            split_traces(
                manager,
                board,
                &Point::Int(center),
                layer,
                *current_net_number,
            );
        }
        layer += 1;
    }
    via_id
}

/// Java `BasicBoard.splitTraces` (`BasicBoard.java:892-910`): splits
/// every trace of `net_number` whose polygon contains `location` on
/// `layer`; returns true when at least one trace was split into a
/// number of pieces OTHER than one (a real split — Java's
/// `splitPieces.size() != 1` guard; a no-op split returns a single
/// piece).
pub(crate) fn split_traces(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    location: &Point,
    layer: i32,
    net_number: i32,
) -> bool {
    // Java pickItems(location, layer, TRACES filter): a TreeSet over
    // the no-clearance overlap query — ASCENDING id order.
    let point_shape =
        TileShape::RegularTileShape(RegularTileShape::IntBox(location.surrounding_box()));
    let mut picked: Vec<ItemId> = manager.overlapping_objects(board, 0, &point_shape, layer, &[]);
    picked.retain(|id| {
        matches!(
            board.get(*id).map(|e| &e.data),
            Some(ItemData::Trace { .. })
        )
    });
    picked.sort_by_key(|id| id.get());
    let location_shape =
        TileShape::RegularTileShape(RegularTileShape::IntBox(location.surrounding_box()))
            .bounding_octagon()
            .expect("bounding octagon of a point box");
    let mut trace_split = false;
    for trace_id in picked {
        let nets = item_nets(board, trace_id);
        if contains_net(&nets, net_number) {
            let split_pieces = split_clip(manager, board, trace_id, Some(&location_shape));
            if split_pieces.len() != 1 {
                trace_split = true;
            }
        }
    }
    trace_split
}

/// Java `Via.attachAllowed` on the via face; false for anything else
/// (Java reads the field through an `instanceof Via` pattern).
fn attach_smd_allowed(board: &Board, drill_id: ItemId) -> bool {
    matches!(
        board.get(drill_id).map(|e| &e.data),
        Some(ItemData::Via {
            attach_smd_allowed: true,
            ..
        })
    )
}

/// Java `(IntPoint) drillItem.getCenter()` — the cast is total for
/// vias (the datum is an IntPoint); Java would throw on a FloatPoint
/// center, which no production caller produces.
fn drill_int_center(board: &Board, drill_id: ItemId) -> IntPoint {
    match board
        .drill_center(drill_id)
        .expect("Java NPE: drill item without a center")
    {
        Point::Int(p) => p,
        Point::Rational(_) => panic!("Java ClassCastException: non-integer via center"),
    }
}

/// The nets of an item, empty for a missing id (Java would NPE;
/// unreachable through the seam).
fn item_nets(board: &Board, id: ItemId) -> Vec<i32> {
    board
        .get(id)
        .map(|entry| entry.nets.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    //! Pins for [`check`] — the pure via-move feasibility probe of the
    //! shove cycle. World: the TraceShoverProbe replay
    //! (`trace_shover::shover_world`; captures
    //! `logs/M3-T10b/captures/trace_shover_rows.jsonl`).

    use super::*;
    use crate::trace_shover::shover_world::build_shover_world;

    /// The guard ORDER of `check` (`DrillItemMover.java:76-118`): a
    /// SHOVE_FIXED drill answers false at `:79` — BEFORE the
    /// `ignoreItems.add(this)` caller-list mutation at `:118` — so the
    /// caller's list stays empty. Kill target: moving the push above
    /// the guard would leave `[fixed_via]` behind.
    #[test]
    fn t_drill_shove_fixed_guard_precedes_the_ignore_push() {
        let (mut manager, mut board, w) = build_shover_world();
        let fixed_via = insert_via(
            &mut manager,
            &mut board,
            w.padstack_no,
            IntPoint::new(505000, 307000),
            &[3],
            0,
            FixedState::ShoveFixed,
            false,
        );
        let zero = Point::int(IntPoint::new(505000, 307000))
            .difference_by(&Point::int(IntPoint::new(505000, 307000)));
        let mut ignore_items = Vec::new();
        assert!(!check(
            &mut manager,
            &mut board,
            fixed_via,
            &zero,
            10,
            10,
            &mut ignore_items,
            None,
        ));
        assert!(ignore_items.is_empty(), "the guard fires before the push");
    }

    /// The pure-check + caller-mutation contract: the UNFIXED foreign
    /// via 106 at (500000,300600) can move north by (0,1000) — check
    /// answers true, pushes the moved via into the CALLER's list
    /// (Java `:118`), and leaves the board untouched.
    #[test]
    fn t_drill_check_is_pure_and_reports_the_moved_via() {
        let (mut manager, mut board, w) = build_shover_world();
        let delta = Point::int(IntPoint::new(500000, 301600))
            .difference_by(&Point::int(IntPoint::new(500000, 300600)));
        let mut ignore_items = Vec::new();
        assert!(check(
            &mut manager,
            &mut board,
            w.via,
            &delta,
            10,
            10,
            &mut ignore_items,
            None,
        ));
        assert_eq!(ignore_items, vec![w.via], "the caller list got the via");
        match board.drill_center(w.via).expect("via alive") {
            Point::Int(ip) => assert_eq!((ip.x, ip.y), (500000, 300600), "pure: nothing moved"),
            Point::Rational(_) => panic!("integer center expected"),
        }
    }

    // ------------------------------------------------------------------
    // M7-T3 — buglog 205 disposition witness (the crafted SPLIT WORLD)
    // ------------------------------------------------------------------

    /// Inline DSN for the split world: one horizontal N1 trace through
    /// (600000, 300000) (hw 1000 board DBU), one N2 net for the foreign
    /// via. Class clearance 200 DSN → 2000 board DBU.
    fn split_world_dsn() -> String {
        r#"(PCB split205.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal) (property (index 0)))
    (layer B.Cu (type signal) (property (index 1)))
    (boundary (rect pcb 0 0 120000 60000))
    (snap_angle ninety_degree)
    (rule (width 200) (clearance 200))
  )
  (library
    (padstack "VIA_PAD"
      (shape (circle F.Cu 300 0 0))
      (shape (circle B.Cu 300 0 0))
      (attach off)
    )
  )
  (network
    (via VT VIA_PAD kicad_default)
    (net "N1")
    (net "N2")
    (class kicad_default "N1" "N2" (rule (clearance 200)))
  )
  (wiring
    (wire (path F.Cu 200 40000 30000 80000 30000) (net N1))
  )
)"#
        .to_string()
    }

    fn build_split_world() -> (SearchTreeManager, Board) {
        let mut board = crate::test_util::parse_board_from_text(&split_world_dsn());
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        crate::normalize_all::normalize_all_traces(&mut manager, &mut board);
        (manager, board)
    }

    /// The N1 traces on layer 0 as (first, last) corner pairs (straight
    /// pieces only — the world guarantees straight pieces).
    fn n1_trace_spans(board: &Board) -> Vec<((i64, i64), (i64, i64))> {
        let mut spans = Vec::new();
        for entry in board.iter_descending() {
            let ItemData::Trace {
                layer: 0, lines, ..
            } = &entry.data
            else {
                continue;
            };
            if !entry.nets.contains(&1) {
                continue;
            }
            let first = lines.corner_approx(0);
            let last = lines.corner_approx(lines.corner_count() as i32 - 1);
            spans.push((
                (first.x as i64, first.y as i64),
                (last.x as i64, last.y as i64),
            ));
        }
        spans.sort_unstable();
        spans
    }

    /// Buglog 205 (M7-T3): the crafted SPLIT WORLD — the post-split DRC
    /// state of the tightener via-arm's re-split face, checkable by
    /// hand. World: N1 trace (400000,300000)-(800000,300000), hw 1000;
    /// FOREIGN N2 via (pad radius 3000) already at (605000,300000) —
    /// its copper overlaps the trace corridor (center distance 5000 <
    /// 3000+1000+2000 = 6000 required), a pre-existing violation. Then
    /// an OWN-NET via lands on the trace path via `insert_via` — the
    /// BasicBoard.insertVia face — whose internal `split_traces` cuts
    /// the trace at (600000,300000) and re-lands the pieces through
    /// `insert_trace_without_cleaning`, exactly the face T2's ITWC
    /// backtrace pinned (bm10 origin). Contract pinned (JAVA PARITY,
    /// `PolylineTrace.split(int,Line)` → `removeItem` + 2x
    /// `insertTraceWithoutCleaning`, no clearance check anywhere in the
    /// chain; the port mirrors it — `split_at_line`/`insert_trace_
    /// without_cleaning`): the split pieces LAND even though their
    /// geometry runs through the foreign via's clearance zone (a
    /// checked landing would have rejected them). Both engines land
    /// split pieces unchecked — bm10's ri=1 is parity behavior, not a
    /// port divergence (outcome ii, PARITY-CONFIRMED-NOTED).
    #[test]
    fn m7t3_split_world_re_lands_pieces_without_a_clearance_check() {
        let (mut manager, mut board) = build_split_world();
        assert_eq!(
            n1_trace_spans(&board),
            vec![((400000, 300000), (800000, 300000))]
        );
        // The foreign violating via first (insertion is unchecked —
        // Java-parity insertVia).
        let via_padstack_no = board
            .library()
            .padstacks
            .iter()
            .position(|p| p.name == "VIA_PAD")
            .map(|i| i as i32 + 1)
            .expect("VIA_PAD padstack");
        insert_via(
            &mut manager,
            &mut board,
            via_padstack_no,
            IntPoint::new(605000, 300000),
            &[2],
            0,
            FixedState::Unfixed,
            false,
        );
        // Own-net via lands ON the trace path → insert_via's internal
        // split_traces cuts the N1 trace at the via location.
        insert_via(
            &mut manager,
            &mut board,
            via_padstack_no,
            IntPoint::new(600000, 300000),
            &[1],
            0,
            FixedState::Unfixed,
            false,
        );
        // The trace is split into TWO pieces landing at the split point
        // — the pieces tile the ORIGINAL path (no clearance check fired
        // on the re-landing): (400000..600000) + (600000..800000).
        assert_eq!(
            n1_trace_spans(&board),
            vec![
                ((400000, 300000), (600000, 300000)),
                ((600000, 300000), (800000, 300000)),
            ],
            "split pieces re-land unchecked through the foreign via's clearance zone",
        );
    }
}
