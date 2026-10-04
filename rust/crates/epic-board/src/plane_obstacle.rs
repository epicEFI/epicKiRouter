//! Upstream #152 (PR #889): the `planeAsObstacle` board face.
//!
//! Java `RoutingBoard.changePlaneAsObstacle(boolean)` (d9694ab82,
//! renamed from `changeConductionIsObstacle`; the POST-review shape
//! of d0d876e30): flip the `isObstacle` flag of every conduction
//! area on a SIGNAL layer to `value` in one pass, then rebuild the
//! search trees if anything moved. The flag is the BEHAVIORAL gate
//! for foreign-net routing — [`crate::board::Board::
//! item_is_trace_obstacle`] answers `is_obstacle && !nets.contains`
//! (Java `ConductionArea.isObstacle(Item)`) — so the flip decides
//! whether foreign traces may cross the pour.
//!
//! Divergences (documented, both verified against the Java tree):
//! - `rules.ignoreConduction` is NOT ported: the post-review Java
//!   still flips it (`targetIgnore = !value`) but NOTHING on the
//!   headless path reads it — only GUI serialization
//!   (`GuiDefaultsFile`/`WindowRouteParameter`) and the method's own
//!   pre-review guard did (git grep over the sunset tree). A Rust
//!   rules-level field would be write-only.
//! - The pre-review guard bug (`if (getIgnoreConduction() != value)
//!   return;` — inverted, a no-op after the first call) is exactly
//!   what d0d876e30 removed; the port is the post-review shape.
//! - Java walks the undo list and mutates through the iterator; the
//!   port walks the arena values — the same item set, and the flag
//!   write needs no undo bookkeeping (Java's `setIsObstacle` does
//!   not go through `UndoableObjects` either).

use crate::board::Board;
use crate::items::ItemData;
use crate::tree_manager::SearchTreeManager;

/// Flip every signal-layer conduction area's obstacle flag to `value`
/// (Java `changePlaneAsObstacle`, post-review d0d876e30 shape).
/// Returns whether anything changed — the caller's debug note; a
/// no-change call (flag already uniform) skips the tree rebuild,
/// exactly Java's `somethingChanged` gate. Non-signal layers and
/// out-of-range layers are left alone (Java `is_signal` conjunct +
/// the index guard; the port's layers read is bounded, the Java
/// AIOOBE is unreachable for board-inserted areas).
pub fn change_plane_as_obstacle(
    board: &mut Board,
    manager: &mut SearchTreeManager,
    value: bool,
) -> bool {
    // Snapshot the signal flags first — the arena walk below holds
    // the mutable borrow.
    let signal_layers: Vec<bool> = board.layers().layers.iter().map(|l| l.is_signal).collect();
    let mut something_changed = false;
    for entry in board.items.values_mut() {
        let ItemData::ConductionArea {
            layer, is_obstacle, ..
        } = &mut entry.data
        else {
            continue;
        };
        let on_signal_layer = usize::try_from(*layer)
            .ok()
            .and_then(|slot| signal_layers.get(slot).copied())
            .unwrap_or(false);
        if on_signal_layer && *is_obstacle != value {
            *is_obstacle = value;
            something_changed = true;
        }
    }
    if something_changed {
        manager.reinsert_tree_items(board);
    }
    something_changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::ItemId;
    use epic_dsn::reader::{DsnReadResult, read_board};
    use epic_dsn::ses_board::SesBoard;

    /// Craft board: F.Cu (signal) GND plane + PWR (power) VDD plane
    /// (the missing-power-plane fallback builds the latter). Both
    /// areas arrive `is_obstacle = false` (every parse-time insert
    /// passes false — Structure.java:1108/:562).
    const CRAFT_DSN: &str = r#"(pcb plane-obstacle-craft.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer PWR (type power) (use_net VDD))
    (boundary (path pcb 0  0 0  10000 0  10000 8000  0 8000  0 0))
    (plane GND
      (polygon F.Cu 0  1000 1000  9000 1000  9000 7000  1000 7000)
    )
  )
  (network
    (net GND)
    (net VDD)
  )
)
"#;

    fn craft_board() -> (Board, SearchTreeManager) {
        let mut ses = SesBoard::new();
        match read_board(CRAFT_DSN.as_bytes(), &mut ses) {
            DsnReadResult::Success { warnings } => {
                assert!(warnings.is_empty(), "WARN_COUNT 0, got {warnings:?}");
            }
            other => panic!("expected Success, got {other:?}"),
        }
        let mut board = Board::from_ses_board(&ses);
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        (board, manager)
    }

    /// (layer, is_obstacle) of every conduction area, layer-sorted.
    fn obstacle_flags(board: &Board) -> Vec<(i32, bool)> {
        let mut out: Vec<(i32, bool)> = board
            .iter_ascending()
            .filter_map(|entry| match &entry.data {
                ItemData::ConductionArea {
                    layer, is_obstacle, ..
                } => Some((*layer, *is_obstacle)),
                _ => None,
            })
            .collect();
        out.sort();
        out
    }

    fn signal_area_id(board: &Board) -> ItemId {
        board
            .iter_ascending()
            .find(|entry| match &entry.data {
                ItemData::ConductionArea { layer, .. } => *layer == 0,
                _ => false,
            })
            .map(|entry| entry.id)
            .expect("F.Cu conduction area exists")
    }

    /// The post-review semantics: the flip is UNCONDITIONAL (no
    /// ignoreConduction guard), reaches only SIGNAL-layer areas, and
    /// a repeat call with the same value reports no change (the
    /// pre-review guard bug made the method a no-op after the first
    /// call — d0d876e30's fix is the `targetIgnore = !value` shape).
    /// The behavioral gate rides along: after the flip the F.Cu pour
    /// blocks a FOREIGN net but never its own.
    #[test]
    fn flips_signal_layer_areas_only_and_reports_change() {
        let (mut board, mut manager) = craft_board();
        assert_eq!(
            obstacle_flags(&board),
            vec![(0, false), (1, false)],
            "both pours start non-obstructing"
        );
        assert!(
            change_plane_as_obstacle(&mut board, &mut manager, true),
            "first call flips the signal-layer area"
        );
        assert_eq!(
            obstacle_flags(&board),
            vec![(0, true), (1, false)],
            "the PWR-layer pour is untouched (non-signal)"
        );
        // The behavioral gate (Java ConductionArea.isObstacle(Item)
        // — Board::item_is_trace_obstacle): foreign blocked, own net
        // never.
        let area = signal_area_id(&board);
        assert!(
            board.item_is_trace_obstacle(area, 2),
            "VDD is foreign to GND's pour"
        );
        assert!(!board.item_is_trace_obstacle(area, 1), "GND's own pour");
        assert!(
            !change_plane_as_obstacle(&mut board, &mut manager, true),
            "uniform flags -> no change, no rebuild"
        );
        assert!(change_plane_as_obstacle(&mut board, &mut manager, false));
        assert_eq!(obstacle_flags(&board), vec![(0, false), (1, false)]);
        assert!(!board.item_is_trace_obstacle(area, 2), "flipped back");
    }
}
