//! The pattern router (M6-T7) — L/Z 1-2-bend routes inside guides for
//! easy nets (design §4.2 stage 2: "pattern routing (L/Z 1-2 bend
//! routes for easy nets) inside guides" — the cheap-net fast path:
//! nets that pattern-route never hit the maze search).
//!
//! ## The contract
//!
//! Eligibility (deterministic, conservative): no plane on the net, the
//! attempt connects exactly two single-layer pins that share one
//! signal layer, and that layer is routing-active. Candidates in FIXED
//! order — L1 `[a, (ax,by), b]`, L2 `[a, (bx,ay), b]`, then Z1/Z2 at
//! the mid-coordinate (mid = (a.y + b.y) / 2, integer division toward
//! zero; a Z degenerating to a tried L is skipped) — first corridor
//! clears on the current board's congestion map (the coarse filter)
//! is inserted through the SAME forced-insertion face the maze arm
//! uses (`path::inserter::get_instance`, same-layer = no via);
//! insertion failure returns None and the caller falls back to the
//! normal maze arm (a partial same-net trace left by a failed insert
//! is ordinary mid-route state the pass structure already handles).
//!
//! Beyond-Java: no oracle exists; the parity claim is the default-off
//! byte-identity of the recurring gate set only.

use epic_board::board::Board;
use epic_board::id::ItemId;
use epic_board::items::ItemData;
use epic_board::trace_tightener::TraceTightenerSeam;
use epic_board::tree_manager::SearchTreeManager;
use epic_geometry::int_point::IntPoint;
use epic_geometry::point::Point;

use crate::control::AutorouteControl;
use crate::path::inserter::get_instance as insert_found_connection;
use crate::path::locator::FoundConnectionLocator;
use crate::path::locator::ResultItem;
use crate::pipeline::batch::BatchSettings;
use crate::pipeline::event_sink::DriverSink;

/// The pattern-route attempt (module docs for the full contract).
/// `Some` = the connection routed; `None` = not attempted or the
/// insertion failed (the caller falls back to the maze arm).
#[allow(clippy::too_many_arguments)] // the route() surface, mirrored
pub fn try_pattern_route(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    settings: &BatchSettings,
    ctrl: &AutorouteControl,
    start_items: &[u64],
    dest_items: &[u64],
    sink: &mut dyn DriverSink,
) -> Option<crate::engine::AutorouteAttemptResult> {
    if !settings.congestion_global || !settings.congestion_global_pattern {
        return None;
    }
    // Easy-net shape: exactly one start and one destination item.
    if start_items.len() != 1 || dest_items.len() != 1 {
        return None;
    }
    let start_id = ItemId::new(u32::try_from(start_items[0]).ok()?);
    let dest_id = ItemId::new(u32::try_from(dest_items[0]).ok()?);

    // Both endpoints single-layer pins on one shared signal layer.
    let start_layer = pin_single_layer(board, start_id);
    let dest_layer = pin_single_layer(board, dest_id);
    let (Some(start_layer), Some(dest_layer)) = (start_layer, dest_layer) else {
        return None;
    };
    if start_layer != dest_layer {
        return None;
    }
    // The plane gate: a plane net routes stub+via, never pattern.
    if board
        .rules()
        .nets
        .get(ctrl.net_number)
        .is_some_and(|net| net.contains_plane)
    {
        return None;
    }
    // The layer must be routing-active in the settings mask.
    if !settings
        .router_settings
        .layer_active
        .get(start_layer as usize)
        .copied()
        .unwrap_or(false)
    {
        return None;
    }
    // The centers (Int corners only — parsed pins are Int).
    let Some(Point::Int(a)) = board.drill_center(start_id) else {
        return None;
    };
    let Some(Point::Int(b)) = board.drill_center(dest_id) else {
        return None;
    };
    if a == b {
        return None;
    }

    // The congestion-map corridor filter (a fresh map — the current
    // board state, already-routed occupancy included). The map is
    // signal-ORDINAL-indexed (every map-build site converts via
    // `signal_ordinal`); the pin layer is a PHYSICAL index and MUST be
    // converted too — on a board with a non-signal layer below, the raw
    // index skips the map's ordinal rows entirely (R-M3 bank, fixed at
    // M6-T8 with its mixed-layer crafted-world pin).
    let map = super::map::CongestionMap::build(board);
    let signal_layer = super::map::CongestionMap::signal_ordinal(board, start_layer)?;
    let corners = candidate_corners(&map, signal_layer, ctrl.net_number, a, b)?;

    // The locator (single item, same layer — the inserter's
    // same-layer via no-op covers it, `path/inserter.rs`).
    let locator = FoundConnectionLocator {
        connection_items: vec![ResultItem {
            corners: corners.clone(),
            layer: start_layer,
        }],
        start_item: Some(start_items[0]),
        start_layer,
        target_item: Some(dest_items[0]),
        target_layer: start_layer,
    };
    let mut seam = TraceTightenerSeam;
    let mut bridge = crate::pipeline::connection_router::SinkBridge(sink);
    let inserted =
        insert_found_connection(Some(&locator), manager, board, &mut seam, ctrl, &mut bridge);
    inserted?;

    // The observability row (log-only): the face the pin bank and the
    // settings-ON runs use to witness that the fast path — not the
    // maze fallback — routed the connection.
    sink.debug(&format!(
        "global_pattern_route net={} layer={} from={} to={}",
        ctrl.net_number, start_layer, a, b,
    ));
    Some(crate::engine::AutorouteAttemptResult::new(
        crate::engine::AutorouteAttemptState::Routed,
    ))
}

/// The L/Z candidate corners for one pin pair (module docs): first
/// corridor-clear candidate wins; None = no candidate clears.
pub(crate) fn candidate_corners(
    map: &super::map::CongestionMap,
    signal_layer: usize,
    net: i32,
    a: IntPoint,
    b: IntPoint,
) -> Option<Vec<IntPoint>> {
    let opt = |corners: Vec<IntPoint>| -> Option<Vec<IntPoint>> {
        if corridor_of(map, signal_layer, net, &corners) {
            Some(corners)
        } else {
            None
        }
    };
    if a.y == b.y || a.x == b.x {
        // The straight face: the degenerate L IS the straight line.
        return opt(vec![a, b]);
    }
    // L1: bend at (a.x, b.y).
    if let Some(c) = opt(vec![a, IntPoint { x: a.x, y: b.y }, b]) {
        return Some(c);
    }
    // L2: bend at (b.x, a.y).
    if let Some(c) = opt(vec![a, IntPoint { x: b.x, y: a.y }, b]) {
        return Some(c);
    }
    // The Z mid-row (toward-zero integer division).
    let mid = (a.y + b.y) / 2;
    if mid != a.y && mid != b.y {
        // Z1: vertical-first jog at the mid row.
        if let Some(c) = opt(vec![
            a,
            IntPoint { x: a.x, y: mid },
            IntPoint { x: b.x, y: mid },
            b,
        ]) {
            return Some(c);
        }
    }
    // The Z mid-column.
    let mid_x = (a.x + b.x) / 2;
    if mid_x != a.x && mid_x != b.x {
        // Z2: horizontal-first jog at the mid column.
        if let Some(c) = opt(vec![
            a,
            IntPoint { x: mid_x, y: a.y },
            IntPoint { x: mid_x, y: b.y },
            b,
        ]) {
            return Some(c);
        }
    }
    None
}

/// The corridor check over an L/Z corner run.
fn corridor_of(
    map: &super::map::CongestionMap,
    signal_layer: usize,
    net: i32,
    corners: &[IntPoint],
) -> bool {
    for pair in corners.windows(2) {
        if !map.corridor_clear(pair[0], pair[1], signal_layer, Some(net)) {
            return false;
        }
    }
    true
}

/// The single signal layer of a single-layer PIN (None for through-
/// hole spans, non-pins, or absent items).
fn pin_single_layer(board: &mut Board, id: ItemId) -> Option<i32> {
    let entry = board.get(id)?;
    match &entry.data {
        ItemData::Pin { .. } => {}
        _ => return None,
    }
    let first = board.drill_first_layer(id)?;
    let last = board.drill_last_layer(id)?;
    if first == last { Some(first) } else { None }
}
