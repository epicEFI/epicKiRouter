//! The pin-connection TAIL of Java `PolylineTrace.pullTight`
//! (`PolylineTrace.java:810-859`) — the T4-deferral, landed in M4-T6.
//!
//! Java's ladder after the tightener returned the input polyline
//! unchanged (`:840-855`):
//!
//! ```text
//! angleRestriction != NINETY_DEGREE && rules.getPinEdgeToTurnDist() > 0:
//!   swapConnectionToPin(true)  -> pullTight(algo); return true
//!   swapConnectionToPin(false) -> pullTight(algo); return true
//!   correctConnectionToPin(true,  angleRestriction) -> pullTight(algo); return true
//!   correctConnectionToPin(false, angleRestriction) -> pullTight(algo); return true
//!   return false
//! ```
//!
//! The gate is LIVE on every 45-degree board: the DSN parser defaults
//! `pinEdgeToTurnDist` to `minTraceHalfWidth` when no
//! `smd_to_turn_gap` rule is present (`io/specctra/parser/Structure.java:667-669`),
//! so the `> 0` half holds everywhere except explicit `-1`-style
//! disables and the insert path's temporary save/restore
//! (`FoundConnectionInserter.insert_trace`, Java `:138-141`/:447).
//! On 90-degree boards the first half fails and the tail never runs.
//!
//! The three faces (all Java `PolylineTrace` methods):
//!
//! * [`check_connection_to_pin`] (`:1013-1076`) — TRUE = the trace end
//!   satisfies its pin's exit restrictions (direction match AND the
//!   preserved-stub length fits). Read-only.
//! * [`correct_connection_to_pin`] (`:1082-1245`) — the connection is
//!   NOT ok: rebuild the trace end along the nearest legal pin exit
//!   direction, walking the offset pad border, and insert a
//!   `SHOVE_FIXED` exit stub from the pin center.
//! * [`swap_connection_to_pin`] (`:1252-1313`) — the end connects to a
//!   single `SHOVE_FIXED` trace at a sharp angle: adopt a better pin
//!   exit direction by flipping the fixed-state ownership and
//!   combining the two traces.
//!
//! Contact-order note: Java's `getStartContacts`/`getEndContacts` are
//! `TreeSet`s over `Item.compareTo = other.id - id` — DESCENDING id
//! order; the port's [`crate::contacts`] results match, so the "first
//! Pin contact" scans iterate identically.

use epic_geometry::direction::Direction;
use epic_geometry::line::Line;
use epic_geometry::polyline::Polyline;
use epic_geometry::side::Side;
use epic_geometry::tile_shape::TileShape;

use crate::board::Board;
use crate::components::{
    PinTraceExitRestriction, pin_calc_nearest_exit_restriction_direction,
    pin_trace_exit_restrictions,
};
use crate::contacts::{end_contacts, start_contacts};
use crate::id::ItemId;
use crate::items::{BoardShape, FixedState, ItemData};
use crate::routing_board_insert::check_polyline_trace;
use crate::rules_surf::AngleRestriction;
use crate::trace_ops::{change_trace_geometry, combine, insert_trace};
use crate::tree_manager::SearchTreeManager;

/// The board ids of a pin contact scan helper result.
fn first_pin_contact(board: &Board, contacts: &[ItemId]) -> Option<ItemId> {
    contacts.iter().copied().find(|id| {
        board
            .get(*id)
            .is_some_and(|entry| matches!(entry.data, ItemData::Pin { .. }))
    })
}

/// The pin's trace exit restrictions on `layer` (Java
/// `contactPin.getTraceExitRestrictions(this.getLayer())`).
fn pin_exit_restrictions(
    board: &Board,
    pin_id: ItemId,
    layer: i32,
) -> Vec<PinTraceExitRestriction> {
    let Some(entry) = board.get(pin_id) else {
        return Vec::new();
    };
    let ItemData::Pin { pin_index, .. } = &entry.data else {
        return Vec::new();
    };
    // Java `entry.componentId` is an int (0 = none); the resolution
    // chain rejects non-positive ids the same way a missing component
    // answers the empty set.
    let component_id = u32::try_from(entry.component_id).unwrap_or(0);
    pin_trace_exit_restrictions(
        board.components(),
        board.library(),
        component_id,
        *pin_index,
        layer,
    )
}

/// The pin's nearest legal exit direction for `trace_polyline` (Java
/// `contactPin.calcNearestExitRestrictionDirection(polyline,
/// halfWidth, layer)`), with the board-cached `pinEdgeToTurnDist`.
fn pin_nearest_exit_direction(
    board: &Board,
    pin_id: ItemId,
    trace_polyline: &Polyline,
    half_width: i32,
    layer: i32,
) -> Option<Direction> {
    let entry = board.get(pin_id)?;
    let ItemData::Pin { pin_index, .. } = &entry.data else {
        return None;
    };
    let component_id = u32::try_from(entry.component_id).unwrap_or(0);
    pin_calc_nearest_exit_restriction_direction(
        board.components(),
        board.library(),
        component_id,
        *pin_index,
        trace_polyline,
        half_width,
        layer,
        board.rules().pin_edge_to_turn_dist,
    )
}

/// Java `PolylineTrace.checkConnectionToPin(atStart)`
/// (`PolylineTrace.java:1013-1076`) — TRUE when the trace end at the
/// checked side satisfies the pin's exit restrictions: a restriction
/// with the end direction exists AND the preserved stub length
/// (`minLength + halfWidth + max(edgeToTurnDist, clearance + 1)`)
/// fits into the current end line.
#[must_use]
pub(crate) fn check_connection_to_pin(
    manager: &SearchTreeManager,
    board: &mut Board,
    trace_id: ItemId,
    at_start: bool,
) -> bool {
    // Java `:1015-1017`: `board == null` — detached trace, connection
    // trivially ok. The production call sites only see on-board traces
    // (the pull-tight gate ladder checked `isOnTheBoard`), so the port
    // reads the live polyline instead.
    let Some(lines) = board.trace_polyline(trace_id).cloned() else {
        return true;
    };
    if lines.corner_count() < 2 {
        return true;
    }
    let contacts = if at_start {
        start_contacts(manager, board, trace_id)
    } else {
        end_contacts(manager, board, trace_id)
    };
    let Some(contact_pin) = first_pin_contact(board, &contacts) else {
        return true;
    };
    let layer = board.trace_layer(trace_id).unwrap_or(0);
    let trace_exit_restrictions = pin_exit_restrictions(board, contact_pin, layer);
    if trace_exit_restrictions.is_empty() {
        return true;
    }
    let (end_corner, prev_end_corner) = if at_start {
        (lines.first_corner(), lines.corner(1))
    } else {
        (
            lines.last_corner(),
            lines.corner(lines.corner_count() as i32 - 2),
        )
    };
    let (Some(end_corner), Some(prev_end_corner)) = (end_corner, prev_end_corner) else {
        // Java `corner(1)`/`corner(count - 2)` answered null — only
        // reachable under the `< 2` corner guard above.
        return true;
    };
    let Some(trace_end_direction) =
        Direction::get_instance_from_points(&end_corner, &prev_end_corner)
    else {
        return true;
    };
    let Some(matching_exit_restriction) = trace_exit_restrictions
        .iter()
        .find(|restriction| restriction.direction == trace_end_direction)
    else {
        return false;
    };
    let edge_to_turn_dist = board.rules().pin_edge_to_turn_dist;
    if edge_to_turn_dist < 0.0 {
        return false;
    }
    let end_line_length = end_corner.to_float().distance(&prev_end_corner.to_float());
    let trace_class = board.get(trace_id).map_or(0, |entry| entry.clearance_class);
    let pin_class = board
        .get(contact_pin)
        .map_or(0, |entry| entry.clearance_class);
    let current_clearance = f64::from(board.clearance_value(trace_class, pin_class, layer));
    let add_width = edge_to_turn_dist.max(current_clearance + 1.0);
    let half_width = board.trace_half_width(trace_id).unwrap_or(0);
    let preserve_length = matching_exit_restriction.min_length + f64::from(half_width) + add_width;
    preserve_length <= end_line_length
}

/// Java `PolylineTrace.correctConnectionToPin(atStart, angleRestriction)`
/// (`PolylineTrace.java:1082-1245`) — the end connection violates the
/// pin's exit restrictions: rebuild the trace end along the nearest
/// legal exit direction (walking the offset pad border the shorter
/// way round), gate the new geometry through
/// `checkPolylineTrace`, change the trace, and insert the
/// `SHOVE_FIXED` exit stub from the pin center.
pub(crate) fn correct_connection_to_pin(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    trace_id: ItemId,
    at_start: bool,
    angle_restriction: AngleRestriction,
) -> bool {
    if check_connection_to_pin(manager, board, trace_id, at_start) {
        return false;
    }
    let mut trace_polyline = match board.trace_polyline(trace_id).cloned() {
        Some(lines) => lines,
        // Java-unreachable (the check above read the live polyline).
        None => return false,
    };
    let contacts = if at_start {
        start_contacts(manager, board, trace_id)
    } else {
        trace_polyline = trace_polyline.reverse();
        end_contacts(manager, board, trace_id)
    };
    let Some(contact_pin) = first_pin_contact(board, &contacts) else {
        return false;
    };
    let layer = board.trace_layer(trace_id).unwrap_or(0);
    let half_width = board.trace_half_width(trace_id).unwrap_or(0);
    let trace_exit_restrictions = pin_exit_restrictions(board, contact_pin, layer);
    if trace_exit_restrictions.is_empty() {
        return false;
    }
    let Some(entry) = board.get(trace_id) else {
        return false;
    };
    let nets = entry.nets.clone();
    let clearance_class = entry.clearance_class;
    // Java `:1111-1114`: `getShape(layer - firstLayer())` must be a
    // TileShape — the FULL transformed pin shape (the drills gate
    // excludes the Simplex arm only for the raw padstack directions).
    let Some(pin_tile) = pin_shape_tile(board, contact_pin, layer) else {
        return false;
    };
    let edge_to_turn_dist = board.rules().pin_edge_to_turn_dist;
    if edge_to_turn_dist < 0.0 {
        return false;
    }
    let pin_class = board
        .get(contact_pin)
        .map_or(0, |entry| entry.clearance_class);
    let current_clearance = f64::from(board.clearance_value(clearance_class, pin_class, layer));
    let add_width = edge_to_turn_dist.max(current_clearance + 1.0);
    let mut offset_pin_shape = pin_tile.offset(f64::from(half_width) + add_width);
    if angle_restriction == AngleRestriction::NinetyDegree || offset_pin_shape.is_int_box() {
        offset_pin_shape = TileShape::RegularTileShape(
            epic_geometry::regular_tile_shape::RegularTileShape::IntBox(
                offset_pin_shape.bounding_box(),
            ),
        );
    } else if angle_restriction == AngleRestriction::FortyfiveDegree {
        // Java `boundingOctagon()` on a box shape answers the box
        // itself (a box IS an octagon); the Option is always Some for
        // the RegularTileShape arms and None only for a Simplex pad —
        // where Java's Simplex.boundingOctagon() still answers a
        // shape, so the port keeps the un-bounded shape instead.
        if let Some(octagon) = offset_pin_shape.bounding_octagon() {
            offset_pin_shape = TileShape::RegularTileShape(
                epic_geometry::regular_tile_shape::RegularTileShape::IntOctagon(octagon),
            );
        }
    }
    let entries = offset_pin_shape.entrance_points(&trace_polyline);
    let Some(&latest_entry_tuple) = entries.last() else {
        return false;
    };
    let Some(entry_line) = trace_polyline
        .lines
        .get(latest_entry_tuple.0 as usize)
        .cloned()
    else {
        return false;
    };
    let trace_entry_location_approx =
        entry_line.intersection_approx(&offset_pin_shape.border_line(latest_entry_tuple.1));
    // The nearest legal pin exit point to `traceEntryLocationApprox`
    // (Java `:1139-1179`).
    let mut min_exit_corner_distance = f64::MAX;
    let mut nearest_pin_exit_ray: Option<Line> = None;
    let mut nearest_border_line_no: i32 = -1;
    let mut pin_exit_direction: Option<Direction> = None;
    let mut nearest_exit_corner: Option<epic_geometry::float_point::FloatPoint> = None;
    const TOLERANCE: f64 = 1.0;
    let Some(pin_center) = board.pin_center(contact_pin) else {
        return false;
    };
    for restriction in &trace_exit_restrictions {
        let current_intersecting_border_line_no =
            offset_pin_shape.intersecting_border_line_no(&pin_center, &restriction.direction);
        // Java reads `borderLine` unconditionally (`:1150-1152`); a
        // missing border line on the offset shape is Java-unreachable
        // (the un-offset derivation in `pin_trace_exit_restrictions`
        // found one for every direction it returns). The port skips
        // the frame instead of indexing out of bounds. (components.rs's
        // nearest-exit face `return None`s its whole frame for the same
        // Java-unreachable condition — different caller contracts, both
        // safe; the asymmetry is deliberate, see quality-review-t6-1
        // NIT-3.)
        if current_intersecting_border_line_no < 0 {
            continue;
        }
        let current_pin_exit_ray =
            Line::new_with_direction(pin_center.clone(), restriction.direction.clone());
        let current_exit_corner = current_pin_exit_ray.intersection_approx(
            &offset_pin_shape.border_line(current_intersecting_border_line_no),
        );
        let current_exit_corner_distance =
            current_exit_corner.distance_square(&trace_entry_location_approx);
        let mut new_nearest_corner_found = false;
        if current_exit_corner_distance + TOLERANCE < min_exit_corner_distance {
            new_nearest_corner_found = true;
        } else if current_exit_corner_distance < min_exit_corner_distance + TOLERANCE {
            // the distances are near equal, compare to the previous
            // corners of tracePolyline (`:1157-1170`)
            if let Some(old_corner) = nearest_exit_corner {
                for i in 1..trace_polyline.corner_count() as i32 {
                    let current_trace_corner = trace_polyline.corner_approx(i);
                    let current_trace_corner_distance =
                        current_trace_corner.distance_square(&current_exit_corner);
                    let old_trace_corner_distance =
                        current_trace_corner.distance_square(&old_corner);
                    if current_trace_corner_distance + TOLERANCE < old_trace_corner_distance {
                        new_nearest_corner_found = true;
                        break;
                    } else if current_trace_corner_distance > old_trace_corner_distance + TOLERANCE
                    {
                        break;
                    }
                }
            }
        }
        if new_nearest_corner_found {
            min_exit_corner_distance = current_exit_corner_distance;
            nearest_pin_exit_ray = Some(current_pin_exit_ray);
            nearest_border_line_no = current_intersecting_border_line_no;
            pin_exit_direction = Some(restriction.direction.clone());
            nearest_exit_corner = Some(current_exit_corner);
        }
    }
    let (Some(nearest_pin_exit_ray), true) = (nearest_pin_exit_ray, nearest_border_line_no >= 0)
    else {
        // Java-unreachable: `traceExitRestrictions` is non-empty, so
        // the loop always seeds a nearest candidate.
        return false;
    };
    // Append the polygon piece around the border of the pin shape
    // (Java `:1181-1206`) — walk the border the shorter way round
    // from the nearest exit border line to the entry border line
    // (counter-clockwise wins ties).
    let corner_count = offset_pin_shape.border_line_count() as i32;
    let clock_wise_side_diff =
        (nearest_border_line_no - latest_entry_tuple.1 + corner_count) % corner_count;
    let counter_clock_wise_side_diff =
        (latest_entry_tuple.1 - nearest_border_line_no + corner_count) % corner_count;
    let mut current_border_line_no = nearest_border_line_no;
    let middle_len = if counter_clock_wise_side_diff <= clock_wise_side_diff {
        counter_clock_wise_side_diff
    } else {
        clock_wise_side_diff
    };
    let mut current_lines: Vec<Line> = Vec::with_capacity(middle_len as usize + 3);
    current_lines.push(nearest_pin_exit_ray.clone());
    let counter_clock_wise = counter_clock_wise_side_diff <= clock_wise_side_diff;
    for _ in 0..=middle_len {
        current_lines.push(offset_pin_shape.border_line(current_border_line_no));
        current_border_line_no = if counter_clock_wise {
            (current_border_line_no + 1) % corner_count
        } else {
            (current_border_line_no - 1 + corner_count) % corner_count
        };
    }
    current_lines.push(entry_line.clone());
    let border_polyline = Polyline::new(current_lines.clone());
    if !check_polyline_trace(
        manager,
        board,
        &border_polyline,
        layer,
        half_width,
        &nets,
        clearance_class,
    ) {
        return false;
    }

    let cut_line_count = trace_polyline.lines.len() - latest_entry_tuple.0 as usize + 1;
    let mut cut_lines: Vec<Line> = Vec::with_capacity(cut_line_count);
    // Java `:1217`: `cutLines[0] = currentLines[currentLines.length - 2]`
    // — the LAST border line of the walk (the walk always appends at
    // least one border line between the ray and the entry line, so
    // `len - 2` is a valid index).
    cut_lines.push(current_lines[current_lines.len() - 2].clone());
    cut_lines.extend(
        trace_polyline.lines[latest_entry_tuple.0 as usize..]
            .iter()
            .cloned(),
    );
    let cut_polyline = Polyline::new(cut_lines);
    let changed_polyline = match (cut_polyline.first_corner(), cut_polyline.last_corner()) {
        (Some(first), Some(last)) if first == last => border_polyline.clone(),
        // Java-NPE face (an EMPTY cut polyline dereferences
        // firstCorner()); unreachable for a pad-crossing entry line —
        // keep the border piece instead of panicking.
        (None, _) | (_, None) => border_polyline.clone(),
        (Some(_), Some(_)) => border_polyline.combine(Some(&cut_polyline)),
    };
    let changed_polyline = if at_start {
        changed_polyline
    } else {
        changed_polyline.reverse()
    };
    // M7-T3 (beyond-Java): the min-length honoring gate (the same
    // predicate the pull-tight acceptance consults — see
    // `trace_tightener::min_length_gate_allows`). The tail rebuild is a
    // shortening-or-not rebuild; a candidate below the constrained
    // net's `min` is rejected WHOLE: the trace keeps its old geometry
    // and the smoothen face reports no-change (the fixpoint's
    // termination face is untouched). Inert when the tuning regime is
    // OFF or the net carries no `min` (the parity regime).
    if !super::min_length_gate_allows(board, trace_id, &changed_polyline) {
        return false;
    }
    change_trace_geometry(manager, board, trace_id, changed_polyline);

    // Create a shoveFixed exit line (Java `:1231-1243`).
    let Some(pin_exit_direction) = pin_exit_direction else {
        return false;
    };
    let exit_lines = vec![
        Line::new_with_direction(pin_center, pin_exit_direction.turn_45_degree(2)),
        nearest_pin_exit_ray,
        offset_pin_shape.border_line(nearest_border_line_no),
    ];
    insert_trace(
        manager,
        board,
        Polyline::new(exit_lines),
        layer,
        half_width,
        &nets,
        clearance_class,
        FixedState::ShoveFixed,
    );
    true
}

/// The pin's transformed shape at `layer` as a [`TileShape`] (Java
/// `contactPin.getShape(layer - contactPin.firstLayer())`, the
/// `instanceof TileShape` gate at `:1111-1114`).
fn pin_shape_tile(board: &Board, pin_id: ItemId, layer: i32) -> Option<TileShape> {
    let entry = board.get(pin_id)?;
    let ItemData::Pin { pin_index, .. } = &entry.data else {
        return None;
    };
    let component_id = u32::try_from(entry.component_id).unwrap_or(0);
    let component = board.components().get(component_id)?;
    let package_pin_no = {
        let package = board.library().package(component.package_no())?;
        package.get_pin(*pin_index)?.padstack_no
    };
    let padstack = board.library().padstack(package_pin_no)?;
    let first_layer = crate::components::pin_first_layer(component, padstack);
    match board.drill_shape(pin_id, layer - first_layer)? {
        BoardShape::Tile(tile) => Some(tile),
        _ => None,
    }
}

/// Java `PolylineTrace.swapConnectionToPin(atStart)`
/// (`PolylineTrace.java:1252-1313`) — the trace end meets a single
/// `SHOVE_FIXED` trace at a sharp angle: look for a better pin exit
/// direction, hand the fixed state to the contact trace, and combine.
pub(crate) fn swap_connection_to_pin(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    trace_id: ItemId,
    at_start: bool,
) -> bool {
    let mut trace_polyline = match board.trace_polyline(trace_id).cloned() {
        Some(lines) => lines,
        None => return false,
    };
    let contacts = if at_start {
        start_contacts(manager, board, trace_id)
    } else {
        trace_polyline = trace_polyline.reverse();
        end_contacts(manager, board, trace_id)
    };
    if contacts.len() != 1 {
        return false;
    }
    let current_contact = contacts[0];
    let Some(contact_entry) = board.get(current_contact) else {
        return false;
    };
    // Java `:1266-1268`: exact `SHOVE_FIXED` equality and a
    // PolylineTrace instance (every Rust trace is a polyline trace).
    if contact_entry.fixed != FixedState::ShoveFixed {
        return false;
    }
    if !matches!(contact_entry.data, ItemData::Trace { .. }) {
        return false;
    }
    let contact_trace = current_contact;
    let contact_polyline = match board.trace_polyline(contact_trace).cloned() {
        Some(lines) => lines,
        None => return false,
    };
    // Java `:1271`: the contact polyline's LAST line — the outer end
    // of the shove-fixed stub. A stub has >= 3 lines (the
    // constructor's floor), so `len - 2` is valid.
    let Some(contact_last_line) = contact_polyline
        .lines
        .len()
        .checked_sub(2)
        .and_then(|index| contact_polyline.lines.get(index))
        .cloned()
    else {
        return false;
    };
    // Java `:1273`: `tracePolyline.lines[1]` — the first REAL line of
    // the trace (index 0 is the unbounded cap). A trace polyline has
    // >= 3 lines, so index 1 is valid.
    let Some(first_line) = trace_polyline.lines.get(1).cloned() else {
        return false;
    };
    let half_width = board.trace_half_width(trace_id).unwrap_or(0);
    // Check for sharp angle (`:1272-1287`).
    let mut check_swap = contact_last_line
        .direction()
        .projection(first_line.direction())
        == Side::Negative;
    if !check_swap {
        let corner_0 = trace_polyline.corner_approx(0);
        let corner_1 = trace_polyline.corner_approx(1);
        let hw = f64::from(half_width);
        if trace_polyline.lines.len() > 3 && corner_0.distance_square(&corner_1) <= hw * hw {
            // check also for sharp angle with the second line
            if let Some(second_line) = trace_polyline.lines.get(2) {
                check_swap = contact_last_line
                    .direction()
                    .projection(second_line.direction())
                    == Side::Negative;
            }
        }
    }
    if !check_swap {
        return false;
    }
    // Java `:1291-1300`: the PIN at the CONTACT trace's START corner.
    let pin_contacts = start_contacts(manager, board, contact_trace);
    let Some(contact_pin) = first_pin_contact(board, &pin_contacts) else {
        return false;
    };
    let combined_polyline = contact_polyline.combine(Some(&trace_polyline));
    let nearest_pin_exit_direction = pin_nearest_exit_direction(
        board,
        contact_pin,
        &combined_polyline,
        half_width,
        board.trace_layer(trace_id).unwrap_or(0),
    );
    let contact_first_direction = contact_polyline
        .lines
        .get(1)
        .map(|line| line.direction().clone());
    match (nearest_pin_exit_direction, contact_first_direction) {
        (Some(nearest), Some(first)) if nearest != first => {}
        // Java `:1306-1309`: null or unchanged direction — no swap.
        _ => return false,
    }
    let this_fixed = board
        .get(trace_id)
        .map_or(FixedState::Unfixed, |entry| entry.fixed);
    board.set_item_fixed(contact_trace, this_fixed);
    combine(manager, board, trace_id);
    true
}

/// The pull-tight tail gate ladder (Java
/// `PolylineTrace.pullTight(TraceTightener)`, `:840-859`): the swap
/// arms, then the correction arms, each recursion re-entering the
/// FULL gate ladder. `false` = the tail could not improve the trace.
pub(crate) fn pin_connection_tail(
    state: &mut crate::trace_tightener::TraceTightener,
    manager: &mut SearchTreeManager,
    board: &mut Board,
    trace_id: ItemId,
) -> bool {
    let angle_restriction = board.rules().trace_angle_restriction;
    let pin_edge_to_turn_dist = board.rules().pin_edge_to_turn_dist;
    if angle_restriction != AngleRestriction::NinetyDegree && pin_edge_to_turn_dist > 0.0 {
        if swap_connection_to_pin(manager, board, trace_id, true) {
            super::polyline_trace_pull_tight(state, manager, board, trace_id);
            return true;
        }
        if swap_connection_to_pin(manager, board, trace_id, false) {
            super::polyline_trace_pull_tight(state, manager, board, trace_id);
            return true;
        }
        // optimize algorithm could not improve the trace, try to
        // remove acid traps (Java `:852`)
        if correct_connection_to_pin(manager, board, trace_id, true, angle_restriction) {
            super::polyline_trace_pull_tight(state, manager, board, trace_id);
            return true;
        }
        if correct_connection_to_pin(manager, board, trace_id, false, angle_restriction) {
            super::polyline_trace_pull_tight(state, manager, board, trace_id);
            return true;
        }
    }
    false
}
