//! Board model: items, ids, undo, rules, layers, components (M2). The
//! live board the parse IR converts into —
//! [`board::Board::from_ses_board`] — and the surface the router
//! mutates (design §4.1).
//!
//! M2 Task 1 scaffold: [`id`] (the id allocator, T61 burn + wrap),
//! [`items`] (kind dispatch, T70 obstacle kinds, per-kind payloads) and
//! [`board`] (the descending-id arena, D25/T60, revision T69, the
//! epic-dsn -> epic-board conversion boundary). Task 2 adds
//! [`undo`] — the `UndoableObjects` level-stack port (T62) jar-pinned
//! by `rust/harness/oracle/UndoSpike.java`, plus the components-side
//! second stack (T63). Task 3 adds the READ surfaces:
//! [`rules_surf`] (the clearance matrix with its ASYMMETRIC index
//! order, T54, the net table, classes, vias), [`layers`] (the
//! board-side `LayerStructure` — exact `getNo`, no parser fallback),
//! and [`components`] (placements with the T68 `rotate` trap, the
//! library mirror, and pin placement resolution jar-pinned by
//! `rust/harness/oracle/PinResolutionSpike.java`). Task 6 adds the
//! search-tree surface: [`tree_shapes`] (the T54 compensation and
//! T55 drill-hole inflation feeding a tree's per-item shapes,
//! jar-pinned by `rust/harness/oracle/TreeShapesSpike.java`) and
//! [`tree_manager`] (the tree set and the insert/remove/rebuild
//! broadcast, T58/T59). Task 10 adds [`contacts`] — the trace
//! CONTACTS seam (`Trace.getNormalContacts` and its start/end/union
//! wrappers), compute-on-demand over the query surface, jar-pinned
//! by `rust/harness/oracle/ContactsSpike.java`. Task 11 adds
//! [`trace_ops`] — the trace COMBINE seam (`PolylineTrace.combine`
//! and the join halves) plus the cycle-removal closure
//! (`removeIfCycle`/`getTraceTail`/`isCycle`/`getConnectionItems`),
//! jar-pinned by `rust/harness/oracle/CombineSpike.java`. Task 12
//! completes the module with the trace SPLIT family
//! (`split(IntOctagon)`/`split(Point)`/`split(int, Line)` +
//! `splitInsideDrillPadProhibited`) and `normalize` (depth cap 16),
//! jar-pinned by `rust/harness/oracle/SplitSpike.java`.

pub mod aesthetics;
pub mod board;
pub mod changed_area;
pub mod components;
pub mod contacts;
pub mod drill_item_mover;
pub mod failure_log;
pub mod forced_pad_router;
pub mod forced_via_inserter;
pub mod id;
pub mod islands;
pub mod items;
pub mod layers;
pub mod normalize_all;
pub mod routing_board_insert;
pub mod routing_board_search;
pub mod routing_ledger;
pub mod rules_surf;
pub mod session_contacts;
pub mod shape_and_entry_side;
pub mod shape_entry_side;
pub mod shape_trace_entries;
pub mod time_limit;
pub mod trace_ops;
pub mod trace_shover;
pub mod trace_tightener;
pub mod tree_manager;
pub mod tree_shapes;
pub mod undo;
pub mod undo_facade;

#[cfg(test)]
pub(crate) mod test_util;
