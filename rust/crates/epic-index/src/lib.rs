//! Spatial index: the Freerouting shape search trees, ported as an
//! arena BVH with incremental insert/remove on the rip-up hot path (M2
//! design §4.1; per-clearance-class Minkowski compensation lands with
//! the board integration).
//!
//! Java anchors: `app.freerouting.datastructures.ShapeTree` (base walks,
//! [`shape_tree`]) and `app.freerouting.datastructures.MinAreaTree`
//! (insert/remove/overlaps strategy, [`min_area_tree`]) — the binary
//! min-area BVH behind every `board.searchtree` index, plus the
//! configured [`search_tree::SearchTree`] wrapper (M2 Task 6). The
//! crate stays GENERIC (M2 plan D17): no board types, no rules —
//! leaves carry opaque `u64` object keys, bounds arrive pre-computed as
//! [`epic_geometry::RegularTileShape`]s, and object ordering enters only
//! through caller-supplied comparator closures. The tree SHAPES and
//! the clearance-compensation arithmetic live caller-side in
//! epic-board (Task 6's `tree_shapes` / `tree_manager`).

pub mod complete_shape;
pub mod min_area_tree;
pub mod search_tree;
pub mod shape_tree;

/// The per-thread scratch-buffer pool (M5 slice B — crate-internal).
mod scratch;

pub use complete_shape::{CompleteShapeObjects, CompleteShapeQuery, IncompleteRoom};
pub use min_area_tree::MinAreaTree;
pub use search_tree::{SearchTree, SearchTreeVariant, TreeEntry};
pub use shape_tree::{LeafEntry, Node, NodeIdx, NodeKind, ShapeTree, format_bounds, intersects};
