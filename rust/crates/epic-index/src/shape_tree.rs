//! The `ShapeTree` node arena: leaves, inner nodes and the base-state
//! walks every tree variant builds on.
//!
//! Java anchor: `src/main/java/app/freerouting/datastructures/ShapeTree.java`
//! (the whole class — NOT `board.searchtree`; the fork keeps the base tree
//! in `datastructures`). Rust has no inheritance, so the split is:
//! this module owns the node types, the slab, the root/leaf-count state
//! and the direction-free base walks ([`ShapeTree::to_array`],
//! `ShapeTree.size()`); [`crate::min_area_tree::MinAreaTree`] owns the
//! insert/remove/overlaps strategy (Java's subclass) and composes this
//! struct as `MinAreaTree { base: ShapeTree }`.
//!
//! Design decisions (M2 plan Task 5, D17):
//!
//! * **Arena, not pointers.** Java's `Leaf`/`InnerNode` objects and their
//!   `parent` links become slab indices ([`NodeIdx`]). Removal does not
//!   free slots — Java nulls the detached node's fields for GC
//!   (MinAreaTree.java:126-128, 160-163); here the slot stays populated
//!   but unreachable from the root. Re-inserting allocates a FRESH slot,
//!   so a stale [`NodeIdx`] can never alias a live leaf.
//! * **Generic over objects (D17).** Java's `Leaf.compareTo`
//!   (ShapeTree.java:217-223) delegates to the OBJECT's `compareTo` —
//!   `Item` compares DESCENDING id (Item.java:94-103) — then breaks ties
//!   by `shapeIndexInObject` ASC. epic-index stores an opaque
//!   [`u64`] object key instead; every ordering-sensitive surface
//!   ([`MinAreaTree::overlaps`]) takes the object ordering as a caller
//!   comparator closure. No epic-board type appears here.
//! * **Bounds arrive pre-computed.** Java `ShapeTree.insert(Storable,
//!   index)` computes `objectShape.boundingShape(boundingDirections)`
//!   (ShapeTree.java:51); the tree itself is direction-agnostic. Here the
//!   caller supplies already-bounded [`RegularTileShape`]s — the 45°
//!   bounding directions live caller-side (Task 6's `SearchTree`), which
//!   keeps this crate pure.
//!
//! Divergences (all unobservable through the public surface):
//!
//! * Java's `insert(Storable, index)` can return `null` leaves (null tree
//!   shape or null bounds, ShapeTree.java:46-59) and `remove` skips them;
//!   Rust shapes are non-null, so [`NodeIdx`] has no absent state.
//! * `ShapeTree.statistics` (ShapeTree.java:112-136) is log-only output —
//!   not ported (M2 scope).

use epic_geometry::regular_tile_shape::RegularTileShape;
use std::fmt;

/// Index of a node in the tree's slab (Java's `Leaf`/`InnerNode` object
/// references become these).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct NodeIdx(u32);

impl NodeIdx {
    /// The raw slab position.
    #[must_use]
    pub fn index(self) -> usize {
        self.0 as usize
    }

    /// A node index from its raw slab position — TEST-ONLY
    /// construction (production leaves come from
    /// [`ShapeTree::push_leaf`], which owns the numbering). The
    /// with-clearance sift ignores the node handle, so its unit tests
    /// synthesize rows.
    #[cfg(test)]
    pub(crate) fn from_index(index: usize) -> NodeIdx {
        NodeIdx(index as u32)
    }
}

/// One leaf or inner node (Java `ShapeTree.TreeNode` + subclasses).
///
/// `Leaf` mirrors `ShapeTree.Leaf` (ShapeTree.java:199-214): the stored
/// object (an opaque key here, D17), the shape's index within that
/// object's shape list, and the pre-bounded tile shape. `Inner` mirrors
/// `ShapeTree.InnerNode` (ShapeTree.java:182-193): the fork bounds and
/// the two children in Java's first/second order (the order is
/// OBSERVABLE — `toArray` walks it, M2 parity decision D19).
#[derive(Clone, Debug)]
pub enum NodeKind {
    /// A leaf: one stored shape of one object.
    Leaf {
        /// The caller's opaque object key (Java: the `Storable` object).
        object_key: u64,
        /// Index of the shape within the object's tree-shape list.
        shape_index_in_object: u32,
        /// The (pre-bounded) shape bounds stored in the tree.
        bounds: RegularTileShape,
    },
    /// An inner node: the fork.
    Inner {
        /// Union bounds of the subtree.
        bounds: RegularTileShape,
        /// Java `firstChild`.
        first: NodeIdx,
        /// Java `secondChild`.
        second: NodeIdx,
    },
}

/// A node: its kind plus the parent link common to both
/// (Java `TreeNode.parent`, ShapeTree.java:173-177).
#[derive(Clone, Debug)]
pub struct Node {
    /// Java `TreeNode.parent` (`None` for the root or a detached node).
    pub parent: Option<NodeIdx>,
    /// Leaf or inner payload.
    pub kind: NodeKind,
}

/// One leaf as enumerated by [`ShapeTree::to_array`] / overlaps results —
/// the D19 parity surface (object id, shape index, bounding shape) plus
/// the slab position so callers can remove the leaf later.
#[derive(Clone, Debug, PartialEq)]
pub struct LeafEntry {
    /// The leaf's slab position (Java: the `Leaf` reference itself).
    pub node: NodeIdx,
    /// The caller's opaque object key.
    pub object_key: u64,
    /// Index of the shape within the object's tree-shape list.
    pub shape_index_in_object: u32,
    /// The bounds stored in the tree.
    pub bounds: RegularTileShape,
}

/// Java `ShapeTree` base state: the node slab, `root` and `leafCount`
/// (ShapeTree.java:15-29). The insert/remove strategy lives in
/// [`crate::min_area_tree::MinAreaTree`].
#[derive(Clone, Debug, Default)]
pub struct ShapeTree {
    pub(crate) nodes: Vec<Node>,
    pub(crate) root: Option<NodeIdx>,
    /// Java `leafCount` — the number of LIVE leaves (`ShapeTree.size()`).
    pub(crate) leaf_count: usize,
}

impl ShapeTree {
    /// An empty tree (Java `ShapeTree` constructor).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Java `ShapeTree.size()` (ShapeTree.java:107-109).
    #[must_use]
    pub fn leaf_count(&self) -> usize {
        self.leaf_count
    }

    /// The root node, if the tree is non-empty.
    #[must_use]
    pub fn root(&self) -> Option<NodeIdx> {
        self.root
    }

    /// The node at `idx` (slab slot; possibly detached garbage).
    #[must_use]
    pub fn node(&self, idx: NodeIdx) -> &Node {
        &self.nodes[idx.index()]
    }

    /// Allocates a leaf slot (Java `new Leaf(object, index, parent,
    /// boundingShape)`, ShapeTree.java:57).
    pub(crate) fn push_leaf(
        &mut self,
        object_key: u64,
        shape_index_in_object: u32,
        bounds: RegularTileShape,
    ) -> NodeIdx {
        // `NodeIdx` is u32: the slab index truncates past 2^32 nodes
        // (~256 GB of nodes; Java OOMs long before).
        debug_assert!(
            self.nodes.len() <= u32::MAX as usize,
            "slab index truncation"
        );
        let idx = NodeIdx(self.nodes.len() as u32);
        self.nodes.push(Node {
            parent: None,
            kind: NodeKind::Leaf {
                object_key,
                shape_index_in_object,
                bounds,
            },
        });
        idx
    }

    /// Allocates an inner-node slot (Java `new InnerNode(boundingShape,
    /// parent)` + the child assignments, ShapeTree.java:66, 81-82).
    pub(crate) fn push_inner(
        &mut self,
        bounds: RegularTileShape,
        parent: Option<NodeIdx>,
        first: NodeIdx,
        second: NodeIdx,
    ) -> NodeIdx {
        debug_assert!(
            self.nodes.len() <= u32::MAX as usize,
            "slab index truncation"
        );
        let idx = NodeIdx(self.nodes.len() as u32);
        self.nodes.push(Node {
            parent,
            kind: NodeKind::Inner {
                bounds,
                first,
                second,
            },
        });
        idx
    }

    /// True if the slot holds a leaf.
    #[must_use]
    pub fn is_leaf(&self, idx: NodeIdx) -> bool {
        matches!(self.nodes[idx.index()].kind, NodeKind::Leaf { .. })
    }

    /// `(first, second)` if the slot is an inner node, `None` for a leaf
    /// (Java `instanceof InnerNode` guards).
    #[must_use]
    pub fn inner_children(&self, idx: NodeIdx) -> Option<(NodeIdx, NodeIdx)> {
        match &self.nodes[idx.index()].kind {
            NodeKind::Inner { first, second, .. } => Some((*first, *second)),
            NodeKind::Leaf { .. } => None,
        }
    }

    /// The node's stored bounds (leaf bounds or inner-node union).
    #[must_use]
    pub fn bounds(&self, idx: NodeIdx) -> &RegularTileShape {
        match &self.nodes[idx.index()].kind {
            NodeKind::Leaf { bounds, .. } | NodeKind::Inner { bounds, .. } => bounds,
        }
    }

    /// Mutable access to a node's stored bounds (the eager-union and
    /// strict-shrink writes).
    pub(crate) fn bounds_mut(&mut self, idx: NodeIdx) -> &mut RegularTileShape {
        match &mut self.nodes[idx.index()].kind {
            NodeKind::Leaf { bounds, .. } | NodeKind::Inner { bounds, .. } => bounds,
        }
    }

    /// The parent link (Java `TreeNode.parent`).
    #[must_use]
    pub fn parent(&self, idx: NodeIdx) -> Option<NodeIdx> {
        self.nodes[idx.index()].parent
    }

    /// Writes the parent link.
    pub(crate) fn set_parent(&mut self, idx: NodeIdx, parent: Option<NodeIdx>) {
        self.nodes[idx.index()].parent = parent;
    }

    /// Re-labels a leaf's owner IN PLACE (Java's direct field writes
    /// `leaf.object = ...; leaf.shapeIndexInObject = ...` —
    /// ShapeSearchTree.java:214-215/:221/:294-295/:150 and
    /// :325-327): the stored bounds are untouched, so the BVH
    /// skeleton, every inner bound and the leaf's position are all
    /// unchanged — this is deliberately NOT a structural op, which is
    /// exactly the M2 merge fast paths' point (surviving entries keep
    /// their tree position; only their owner is re-attributed).
    pub(crate) fn relabel_leaf(
        &mut self,
        idx: NodeIdx,
        object_key: u64,
        shape_index_in_object: u32,
    ) {
        match &mut self.nodes[idx.index()].kind {
            NodeKind::Leaf {
                object_key: stored_key,
                shape_index_in_object: stored_index,
                ..
            } => {
                *stored_key = object_key;
                *stored_index = shape_index_in_object;
            }
            NodeKind::Inner { .. } => {
                unreachable!("relabel_leaf on an inner node (Java writes Leaf fields only)");
            }
        }
    }

    /// Mutable `(first, second)` child slots for the splice sites (Java
    /// writes `currentParent.firstChild/secondChild` and
    /// `grandParent.firstChild/secondChild` directly).
    pub(crate) fn children_mut(&mut self, idx: NodeIdx) -> (&mut NodeIdx, &mut NodeIdx) {
        match &mut self.nodes[idx.index()].kind {
            NodeKind::Inner { first, second, .. } => (first, second),
            NodeKind::Leaf { .. } => {
                unreachable!("children_mut on a leaf (Java assigns InnerNode fields only)");
            }
        }
    }

    /// Inserts the tree's leaves into an array, in-order
    /// (Java `ShapeTree.toArray()`, ShapeTree.java:67-94 — down to the
    /// leftmost leaf, up while we came from `secondChild`, then into the
    /// parent's `secondChild`). The result carries ALL live leaves — like
    /// Java's array it is NOT deduplicated (a double-inserted object
    /// yields two leaves per shape).
    #[must_use]
    pub fn to_array(&self) -> Vec<LeafEntry> {
        let mut result = Vec::with_capacity(self.leaf_count);
        let Some(mut current) = self.root else {
            return result;
        };
        loop {
            // Go down from current to the left-most leaf.
            while let Some((first, _)) = self.inner_children(current) {
                current = first;
            }
            match &self.nodes[current.index()].kind {
                NodeKind::Leaf {
                    object_key,
                    shape_index_in_object,
                    bounds,
                } => result.push(LeafEntry {
                    node: current,
                    object_key: *object_key,
                    shape_index_in_object: *shape_index_in_object,
                    bounds: bounds.clone(),
                }),
                NodeKind::Inner { .. } => {
                    unreachable!("the descent above stops at leaves only");
                }
            }
            // Up until parent.secondChild != current (we came from
            // firstChild), i.e. climb while current IS the parent's second.
            let mut parent = self.nodes[current.index()].parent;
            while let Some(p) = parent {
                let second = match self.inner_children(p) {
                    Some((_, second)) => second,
                    None => unreachable!("to_array walk through a leaf parent"),
                };
                if second != current {
                    break;
                }
                current = p;
                parent = self.nodes[current.index()].parent;
            }
            let Some(p) = parent else {
                break;
            };
            current = match self.inner_children(p) {
                Some((_, second)) => second,
                None => unreachable!("to_array walk through a leaf parent"),
            };
        }
        result
    }
}

/// Java `RegularTileShape.intersects(Shape)` as used by
/// `MinAreaTree.overlaps` (MinAreaTree.java:38): double dispatch — the
/// receiver's `intersects(Shape)` forwards to `other.intersects(this)`,
/// so every mixed pair lands on the NON-receiver's concrete overload
/// (IntBox.java:329, IntOctagon.java:628). Arm by arm:
///
/// * box, box → `Box.intersects(IntBox)` (inclusive border touch);
/// * box, oct → `Octagon.intersects(IntBox)` = octagon test vs the box's
///   octagon hull;
/// * oct, box → `Box.intersects(IntOctagon)` = the octagon vs the BOX's
///   hull (argument converts, not the receiver);
/// * oct, oct → `Octagon.intersects(IntOctagon)` (normalized test).
#[must_use]
pub fn intersects(a: &RegularTileShape, b: &RegularTileShape) -> bool {
    match (a, b) {
        (RegularTileShape::IntBox(x), RegularTileShape::IntBox(y)) => y.intersects(x),
        (RegularTileShape::IntBox(x), RegularTileShape::IntOctagon(y)) => y.intersects_box(x),
        (RegularTileShape::IntOctagon(x), RegularTileShape::IntBox(y)) => y.intersects_octagon(x),
        (RegularTileShape::IntOctagon(x), RegularTileShape::IntOctagon(y)) => y.intersects(x),
    }
}

/// Canonical bounds formatting for dumps and pins — the exact format of
/// the Task 5 jar capture (`/tmp/epic-t5-tree.out`, produced by
/// `rust/harness/oracle/MinAreaTreeSpike.java`):
/// `box[llx lly urx ury]` / `oct[lx by rx ty ulx lrx llx urx]`.
#[must_use]
pub fn format_bounds(shape: &RegularTileShape) -> String {
    match shape {
        RegularTileShape::IntBox(b) => {
            format!("box[{} {} {} {}]", b.ll.x, b.ll.y, b.ur.x, b.ur.y)
        }
        RegularTileShape::IntOctagon(o) => {
            format!(
                "oct[{} {} {} {} {} {} {} {}]",
                o.left_x,
                o.bottom_y,
                o.right_x,
                o.top_y,
                o.upper_left_diagonal_x,
                o.lower_right_diagonal_x,
                o.lower_left_diagonal_x,
                o.upper_right_diagonal_x
            )
        }
    }
}

impl fmt::Display for NodeIdx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use epic_geometry::int_box::IntBox;

    fn box_shape(llx: i32, lly: i32, urx: i32, ury: i32) -> RegularTileShape {
        RegularTileShape::IntBox(IntBox::from_corners(llx, lly, urx, ury))
    }

    /// `toArray` walk on the empty tree (Java returns the empty array,
    /// ShapeTree.java:68-70).
    #[test]
    fn to_array_empty() {
        let tree = ShapeTree::new();
        assert!(tree.to_array().is_empty());
        assert_eq!(tree.leaf_count(), 0);
        assert_eq!(tree.root(), None);
    }

    /// `toArray` in-order walk shape on a three-leaf chain built by hand:
    /// root(L1, N(L2, L3)) enumerates L1, L2, L3 (leftmost descent, then
    /// the parent's second subtree), mirroring the loop in
    /// ShapeTree.java:72-92.
    #[test]
    fn to_array_walks_in_order() {
        let mut tree = ShapeTree::new();
        let l1 = tree.push_leaf(1, 0, box_shape(0, 0, 10, 10));
        let l2 = tree.push_leaf(2, 0, box_shape(20, 0, 30, 10));
        let l3 = tree.push_leaf(3, 0, box_shape(40, 0, 50, 10));
        let n = tree.push_inner(box_shape(20, 0, 50, 10), None, l2, l3);
        tree.set_parent(l2, Some(n));
        tree.set_parent(l3, Some(n));
        let root = tree.push_inner(box_shape(0, 0, 50, 10), None, l1, n);
        tree.set_parent(l1, Some(root));
        tree.set_parent(n, Some(root));
        tree.root = Some(root);
        tree.leaf_count = 3;

        let keys: Vec<(u64, u32)> = tree
            .to_array()
            .into_iter()
            .map(|e| (e.object_key, e.shape_index_in_object))
            .collect();
        assert_eq!(keys, vec![(1, 0), (2, 0), (3, 0)]);
        // The array carries ALL leaves with their bounds (D19 surface).
        assert_eq!(tree.to_array()[2].bounds, box_shape(40, 0, 50, 10));
    }

    /// `format_bounds` matches the jar capture's two formats exactly.
    #[test]
    fn format_bounds_matches_capture() {
        assert_eq!(format_bounds(&box_shape(0, -5, 40, 10)), "box[0 -5 40 10]");
        // The 45-degree hull of box[0,10]^2 as captured in the
        // fortyfive_mixed scenario (oct[0 0 10 10 -10 10 0 20]).
        let oct = IntBox::from_corners(0, 0, 10, 10).to_int_octagon();
        assert_eq!(
            format_bounds(&RegularTileShape::IntOctagon(oct)),
            "oct[0 0 10 10 -10 10 0 20]"
        );
    }

    /// The `intersects` double dispatch: box/box border touch is
    /// inclusive (IntBox.java:334-347), a one-unit gap misses; the mixed
    /// arms route through octagon hulls whose geometry equals the source
    /// boxes, so the expected results reduce to box algebra (the
    /// true-octagon mixed cases are pinned in `min_area_tree` from the
    /// fortyfive_mixed capture).
    #[test]
    fn intersects_dispatch_arms() {
        let b0 = box_shape(0, 0, 10, 10);
        let b_touch = box_shape(10, 0, 20, 10);
        let b_gap = box_shape(11, 0, 20, 10);
        let b_far = box_shape(100, 0, 110, 10);
        assert!(intersects(&b0, &b_touch));
        assert!(intersects(&b_touch, &b0));
        assert!(!intersects(&b0, &b_gap));
        assert!(!intersects(&b0, &b_far));

        // Hulls of the same boxes: (Box, Oct) and (Oct, Box) arms. The
        // hull of a box is geometrically the box itself, so the expected
        // results match the box/box row above.
        let h0 = RegularTileShape::IntOctagon(IntBox::from_corners(0, 0, 10, 10).to_int_octagon());
        let h_touch =
            RegularTileShape::IntOctagon(IntBox::from_corners(10, 0, 20, 10).to_int_octagon());
        let h_far =
            RegularTileShape::IntOctagon(IntBox::from_corners(100, 0, 110, 10).to_int_octagon());
        assert!(intersects(&b0, &h_touch)); // (Box, Oct)
        assert!(intersects(&h_touch, &b0)); // (Oct, Box)
        assert!(intersects(&h0, &h_touch)); // (Oct, Oct)
        assert!(!intersects(&b0, &h_far));
        assert!(!intersects(&h_far, &b0));
        assert!(!intersects(&h0, &h_far));
    }
}
