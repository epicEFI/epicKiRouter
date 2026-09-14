//! Spatial index: the Freerouting shape search trees, ported as an
//! arena BVH with incremental insert/remove on the rip-up hot path (M2
//! design §4.1; per-clearance-class Minkowski compensation lands with
//! the board integration).
//!
//! Java anchors: `app.freerouting.datastructures.ShapeTree` (base walks,
//! [`shape_tree`]) and `app.freerouting.datastructures.MinAreaTree`
//! (insert/remove/overlaps strategy, [`min_area_tree`]) — the binary
//! min-area BVH behind every `board.searchtree` index. The port is
//! DIRECTION-FREE (M2 plan D17): this crate holds no bounding-directions
//! variant and no board types — leaves carry opaque `u64` object keys,
//! bounds arrive pre-computed as [`epic_geometry::RegularTileShape`]s,
//! and object ordering enters only through caller-supplied comparator
//! closures. The 45°/orthogonal variants and the `Storable` adapter live
//! caller-side in Task 6's `SearchTree`.

pub mod min_area_tree;
pub mod shape_tree;

pub use min_area_tree::MinAreaTree;
pub use shape_tree::{LeafEntry, Node, NodeIdx, NodeKind, ShapeTree, format_bounds, intersects};
