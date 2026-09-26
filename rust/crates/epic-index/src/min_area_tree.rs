//! The `MinAreaTree`: the binary min-area BVH behind every Freerouting
//! shape search tree.
//!
//! Java anchor: `src/main/java/app/freerouting/datastructures/
//! MinAreaTree.java` (the whole class) extending `ShapeTree.java` — both
//! live in `datastructures`, NOT `board.searchtree`. The port is a
//! line-for-line transliteration onto the [`crate::shape_tree`] arena
//! (object references become [`NodeIdx`] slab indices).
//!
//! Traps covered (M2 plan T50–T52):
//!
//! * **T50 — insert descent.** `position_locate` EAGERLY unions the
//!   inserted leaf into every VISITED inner node's bounds
//!   (MinAreaTree.java:94-95) — including nodes whose sibling child is
//!   then chosen — BEFORE descending. The child choice is
//!   minimal-area-increase on f64 areas (Java `double`), and an exact tie
//!   descends FIRST (`firstAreaIncrease <= secondAreaIncrease`,
//!   MinAreaTree.java:109). The spliced inner node's children are
//!   old-leaf-FIRST, new-leaf-SECOND (MinAreaTree.java:81-82).
//! * **T51 — history dependence.** `remove_leaf` promotes the sibling
//!   under the grandparent, then recomputes ancestor bounds only while
//!   STRICTLY shrinking (`newBounds.contains(old)` → break,
//!   MinAreaTree.java:166-178). The tree SHAPE is therefore
//!   history-dependent: the same final leaf set built through a different
//!   insert/remove order yields a different tree (pinned below).
//! * **T52 — overlaps ordering.** The DFS pushes first-then-second
//!   (popped second-then-first) but collects into a TreeSet sorted by
//!   `Leaf.compareTo` (ShapeTree.java:217-223): the OBJECT's order
//!   (caller-supplied comparator; `Item` is DESCENDING id,
//!   Item.java:94-103), then `shapeIndexInObject` ASC; compareTo-equal
//!   duplicates collapse. `to_array` (an ARRAY) keeps duplicates.
//!
//! Bounds-tightness note (jar-spiked, `/tmp/epic-t5-tree.out`): under the
//! public insert/remove API the stored inner bounds NEVER diverge from
//! the tight union of the children — the eager union lands the leaf
//! below every visited node, and the remove loop's `contains` break
//! fires exactly when the recomputation equals the stored bounds (new
//! ⊆ old always holds). The `contains` break is therefore an
//! optimization, not an observable-staleness source; T51's observable is
//! the STRUCTURE. Pins re-derive the tightness invariant on every dump.

use crate::shape_tree::{LeafEntry, NodeIdx, NodeKind, ShapeTree, format_bounds, intersects};
use epic_geometry::regular_tile_shape::RegularTileShape;
use std::cmp::Ordering;

/// The min-area binary BVH (Java `MinAreaTree extends ShapeTree`).
///
/// Generic over stored objects (D17): leaves carry an opaque `u64` object
/// key, and every object-ordering-sensitive surface takes the ordering as
/// a comparator closure ([`MinAreaTree::overlaps`]). Bounds arrive
/// pre-computed — the bounding-directions variant lives caller-side.
#[derive(Clone, Debug, Default)]
pub struct MinAreaTree {
    /// Java's inherited `ShapeTree` base state.
    pub(crate) base: ShapeTree,
}

impl MinAreaTree {
    /// An empty tree (Java `MinAreaTree(directions)`; directions are
    /// caller-side here).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Java `ShapeTree.size()` — the number of live leaves.
    #[must_use]
    pub fn leaf_count(&self) -> usize {
        self.base.leaf_count()
    }

    /// The root node, if the tree is non-empty.
    #[must_use]
    pub fn root(&self) -> Option<NodeIdx> {
        self.base.root()
    }

    /// The node at `idx` (slab slot; possibly detached garbage after a
    /// removal).
    #[must_use]
    pub fn node(&self, idx: NodeIdx) -> &crate::shape_tree::Node {
        self.base.node(idx)
    }

    /// Inserts all shapes of one object: ONE LEAF PER SHAPE, in list
    /// order (Java `ShapeTree.insert(Storable)`, ShapeTree.java:31-42).
    /// Returns the leaf indices (Java hands them to the object via
    /// `setSearchTreeEntries`); callers pass them back to
    /// [`MinAreaTree::remove`]. An empty shape list is a no-op returning
    /// no leaves (Java `shapeCount <= 0`).
    pub fn insert(&mut self, object_key: u64, shapes: &[RegularTileShape]) -> Vec<NodeIdx> {
        if shapes.is_empty() {
            return Vec::new();
        }
        let mut leaves = Vec::with_capacity(shapes.len());
        for (index, bounds) in shapes.iter().enumerate() {
            let leaf = self
                .base
                .push_leaf(object_key, index as u32, bounds.clone());
            self.insert_leaf(leaf);
            leaves.push(leaf);
        }
        leaves
    }

    /// Removes every leaf of `entries` (Java `ShapeTree.remove`,
    /// ShapeTree.java:97-104).
    pub fn remove(&mut self, entries: &[NodeIdx]) {
        for leaf in entries {
            self.remove_leaf(*leaf);
        }
    }

    /// Java `MinAreaTree.insert(Leaf)` (MinAreaTree.java:50-87),
    /// transliterated.
    pub(crate) fn insert_leaf(&mut self, leaf: NodeIdx) {
        self.base.leaf_count += 1;

        // Tree is empty - just insert the new leaf.
        let Some(root) = self.base.root else {
            self.base.root = Some(leaf);
            return;
        };

        // Non-empty tree - do a location for leaf replacement.
        let leaf_to_replace = self.position_locate(root, leaf);

        // Construct a new node - whenever a leaf is added so is a new
        // node. Children: leafToReplace FIRST, leaf SECOND
        // (MinAreaTree.java:81-82).
        let new_bounds = self
            .base
            .bounds(leaf)
            .union(self.base.bounds(leaf_to_replace));
        let current_parent = self.base.parent(leaf_to_replace);
        let new_node = self
            .base
            .push_inner(new_bounds, current_parent, leaf_to_replace, leaf);

        if let Some(parent) = current_parent {
            // Replace the pointer from the parent to the leaf with the
            // new node (first-child check, else second — Java :70-74).
            let (first_slot, second_slot) = self.base.children_mut(parent);
            if *first_slot == leaf_to_replace {
                *first_slot = new_node;
            } else {
                *second_slot = new_node;
            }
        }
        // Parent pointers of the old leaf and the new leaf point at
        // the new node — both set by these two calls (push_inner's
        // `parent` parameter sets the NEW INNER node's own parent,
        // not the children's; Java :77-78).
        self.base.set_parent(leaf_to_replace, Some(new_node));
        self.base.set_parent(leaf, Some(new_node));

        if self.base.root == Some(leaf_to_replace) {
            self.base.root = Some(new_node);
        }
    }

    /// Java `MinAreaTree.positionLocate` (MinAreaTree.java:89-116),
    /// transliterated. Returns the leaf the descent lands on.
    fn position_locate(&mut self, current: NodeIdx, leaf: NodeIdx) -> NodeIdx {
        let mut node = current;
        while let Some((first, second)) = self.base.inner_children(node) {
            // EAGER union into every VISITED inner node
            // (MinAreaTree.java:94-95) — BEFORE the child choice.
            let leaf_bounds = self.base.bounds(leaf).clone();
            let grown = leaf_bounds.union(self.base.bounds(node));
            *self.base.bounds_mut(node) = grown;

            // Choose the child whose area increase after the union with
            // the inserted shape is minimal (f64 areas, Java double).
            let first_bounds = self.base.bounds(first).clone();
            let second_bounds = self.base.bounds(second).clone();
            let d1 = leaf_bounds.union(&first_bounds).area() - first_bounds.area();
            let d2 = leaf_bounds.union(&second_bounds).area() - second_bounds.area();

            // T50: a tie descends FIRST (`<=`).
            node = if d1 <= d2 { first } else { second };
        }
        node
    }

    /// Removes a leaf (Java `MinAreaTree.removeLeaf`,
    /// MinAreaTree.java:118-179), transliterated.
    ///
    /// Hazard inherited from Java: removing a leaf that is not in the
    /// tree (double remove) corrupts the structure exactly like the Java
    /// original (a detached leaf has no parent, so the removal clears
    /// the root). Callers must remove live leaves only. The COUNT
    /// diverges in that corrupt state: Java's `leafCount` silently
    /// goes negative; the port's `usize` count underflows (debug
    /// panic, release wrap — and the wrapped count then aborts the
    /// next `to_array` at the capacity computation) where Java NPEs
    /// inside the walk.
    pub fn remove_leaf(&mut self, leaf: NodeIdx) {
        // Java nulls the leaf's fields here (:126-128, GC hygiene); the
        // slab slot stays populated but unreachable — except the parent
        // link, cleared to mirror `leaf.parent = null`.
        let parent = self.base.parent(leaf);
        self.base.set_parent(leaf, None);
        self.base.leaf_count -= 1;
        let Some(parent) = parent else {
            // Tree gets empty (Java :130-134).
            self.base.root = None;
            return;
        };

        // Find the other leaf of the parent (:135-144).
        let Some((first, second)) = self.base.inner_children(parent) else {
            unreachable!("remove_leaf: parent is not an inner node (corrupt tree)");
        };
        let other = if second == leaf {
            first
        } else if first == leaf {
            second
        } else {
            // Java warns ("parent inconsistent") and leaves otherLeaf
            // null; the next statement would then NPE (:141-147). The
            // parent-child invariant makes this unreachable.
            unreachable!("remove_leaf: parent does not own the leaf (Java warns then NPEs)");
        };

        // Link the other leaf to the grandparent and remove the parent
        // node (:145-159). Java also nulls the detached inner node's
        // fields (:160-163); the slab slot becomes unreachable instead.
        let grand_parent = self.base.parent(parent);
        self.base.set_parent(other, grand_parent);
        match grand_parent {
            None => {
                // Only one leaf left in the tree.
                self.base.root = Some(other);
            }
            Some(gp) => {
                let (gfirst_slot, gsecond_slot) = self.base.children_mut(gp);
                if *gsecond_slot == parent {
                    *gsecond_slot = other;
                } else if *gfirst_slot == parent {
                    *gfirst_slot = other;
                } else {
                    // Java warns ("grandParent inconsistent") and leaves
                    // the tree corrupt (:156-158); unreachable by the
                    // invariant.
                    unreachable!("remove_leaf: grandparent does not own the parent");
                }
            }
        }

        // Recalculate the bounding shapes of the ancestors as long as
        // they STRICTLY shrink (:165-178) — `newBounds.contains(old)`
        // breaks the walk.
        let mut node_to_recalculate = grand_parent;
        while let Some(n) = node_to_recalculate {
            let Some((first, second)) = self.base.inner_children(n) else {
                unreachable!("ancestor recalculation through a leaf (corrupt tree)");
            };
            // Java computes secondChild.boundingShape.union(firstChild…).
            let new_bounds = self.base.bounds(second).union(self.base.bounds(first));
            if new_bounds.contains_regular(self.base.bounds(n)) {
                // The new bounds are not smaller — no further
                // recalculation necessary.
                break;
            }
            *self.base.bounds_mut(n) = new_bounds;
            node_to_recalculate = self.base.parent(n);
        }
    }

    /// The objects in this tree whose leaf bounds intersect `shape`
    /// (Java `MinAreaTree.overlaps`, MinAreaTree.java:25-48):
    /// bbox prefilter ONLY — no exact-shape test. The result mirrors
    /// Java's `TreeSet<Leaf>` iteration: sorted by `cmp(object_key)`
    /// (the OBJECT's compareTo — `Item` is DESCENDING id) then
    /// `shape_index_in_object` ASC, with compareTo-equal duplicates
    /// collapsed (an object inserted twice is reported once per
    /// shape). The RETAINED entry is the first leaf in DFS
    /// discovery order — matching Java's TreeSet, which keeps the
    /// first-added key on an equal re-add.
    ///
    /// The DFS itself pushes first-then-second (popped
    /// second-then-first); the sort makes the traversal order
    /// unobservable HERE, but `overlapping_tree_entries` (Task 8) must
    /// keep the same stack order.
    pub fn overlaps<CMP>(&self, shape: &RegularTileShape, cmp: CMP) -> Vec<LeafEntry>
    where
        CMP: Fn(u64, u64) -> Ordering,
    {
        let mut found: Vec<LeafEntry> = Vec::new();
        let Some(root) = self.base.root else {
            return found;
        };
        // Java ArrayStack (LIFO): push firstChild, then secondChild —
        // second pops first (MinAreaTree.java:42-43). The stack is the
        // pooled per-thread DFS stack (slice B): cleared before the
        // walk, capacity retained across queries.
        crate::scratch::with_node_stack(|node_stack| {
            node_stack.clear();
            node_stack.push(root);
            while let Some(current) = node_stack.pop() {
                if intersects(self.base.bounds(current), shape) {
                    match &self.base.node(current).kind {
                        NodeKind::Leaf {
                            object_key,
                            shape_index_in_object,
                            bounds,
                        } => {
                            found.push(LeafEntry {
                                node: current,
                                object_key: *object_key,
                                shape_index_in_object: *shape_index_in_object,
                                bounds: bounds.clone(),
                            });
                        }
                        NodeKind::Inner { first, second, .. } => {
                            node_stack.push(*first);
                            node_stack.push(*second);
                        }
                    }
                }
            }
        });
        // TreeSet<Leaf> semantics (Leaf.compareTo, ShapeTree.java:217-223):
        // object order (caller-supplied) then shape index ASC, dedup on
        // compareTo == 0.
        found.sort_by(|a, b| {
            cmp(a.object_key, b.object_key)
                .then(a.shape_index_in_object.cmp(&b.shape_index_in_object))
        });
        found.dedup_by(|a, b| {
            cmp(a.object_key, b.object_key) == Ordering::Equal
                && a.shape_index_in_object == b.shape_index_in_object
        });
        found
    }

    /// The tree's leaves, in-order (Java `ShapeTree.toArray` — see
    /// [`ShapeTree::to_array`]); ALL live leaves, no dedup.
    #[must_use]
    pub fn to_array(&self) -> Vec<LeafEntry> {
        self.base.to_array()
    }

    /// Canonical pre-order structural dump — `I <bounds>` /
    /// `L obj=<key> idx=<i> <bounds>`, the root at depth 0, four spaces
    /// per level below, children in first/second order. Matches the
    /// Task 5 jar capture (`/tmp/epic-t5-tree.out`, produced by
    /// `rust/harness/oracle/MinAreaTreeSpike.java` against the frozen
    /// jar) modulo that capture's uniform extra indent (the spike dumps
    /// the root at depth 1, nested under its `dump` header) and its
    /// `tight=` probe suffix.
    #[must_use]
    pub fn dump_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(root) = self.base.root {
            self.dump_node(root, 0, &mut lines);
        }
        lines
    }

    fn dump_node(&self, idx: NodeIdx, depth: usize, lines: &mut Vec<String>) {
        let indent = "    ".repeat(depth);
        match &self.base.node(idx).kind {
            NodeKind::Leaf {
                object_key,
                shape_index_in_object,
                bounds,
            } => lines.push(format!(
                "{indent}L obj={object_key} idx={shape_index_in_object} {}",
                format_bounds(bounds)
            )),
            NodeKind::Inner {
                bounds,
                first,
                second,
            } => {
                lines.push(format!("{indent}I {}", format_bounds(bounds)));
                self.dump_node(*first, depth + 1, lines);
                self.dump_node(*second, depth + 1, lines);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use epic_geometry::int_box::IntBox;
    use epic_geometry::int_octagon::IntOctagon;

    /// Item order for the overlaps comparator: DESCENDING id
    /// (Item.java:94-103 `item.id - id`).
    fn descending(a: u64, b: u64) -> Ordering {
        b.cmp(&a)
    }

    fn box_shape(llx: i32, lly: i32, urx: i32, ury: i32) -> RegularTileShape {
        RegularTileShape::IntBox(IntBox::from_corners(llx, lly, urx, ury))
    }

    /// The 45-degree hull of a box — what Java's insert stores in a
    /// FortyfiveDegreeBoundingDirections tree (captured convention:
    /// box[0 0 10 10] -> oct[0 0 10 10 -10 10 0 20]).
    fn hull(llx: i32, lly: i32, urx: i32, ury: i32) -> RegularTileShape {
        RegularTileShape::IntOctagon(IntBox::from_corners(llx, lly, urx, ury).to_int_octagon())
    }

    fn leaf_order(tree: &MinAreaTree) -> Vec<(u64, u32)> {
        tree.to_array()
            .into_iter()
            .map(|e| (e.object_key, e.shape_index_in_object))
            .collect()
    }

    fn overlap_keys(tree: &MinAreaTree, shape: &RegularTileShape) -> Vec<(u64, u32)> {
        tree.overlaps(shape, descending)
            .into_iter()
            .map(|e| (e.object_key, e.shape_index_in_object))
            .collect()
    }

    /// Every inner node's stored bounds must equal the tight union of its
    /// children (the jar capture never prints STALE; see the module docs
    /// for why this is an invariant of the public API).
    fn assert_bounds_tight(tree: &MinAreaTree) {
        fn walk(tree: &MinAreaTree, idx: NodeIdx) {
            if let Some((first, second)) = tree.base.inner_children(idx) {
                let tight = tree.base.bounds(second).union(tree.base.bounds(first));
                assert_eq!(
                    &tight,
                    tree.base.bounds(idx),
                    "inner node {} bounds not tight",
                    idx
                );
                walk(tree, first);
                walk(tree, second);
            }
        }
        if let Some(root) = tree.base.root {
            walk(tree, root);
        }
    }

    // ---- T50: exact-tie descent picks FIRST (MinAreaTree.java:109) --------

    /// Jar capture SCEN tie_orth (dumps after "1,2" and after "3"):
    /// leaves 1=[0 10]^2 and 2=[100 110]x[0 10]; inserting 3=[50 60]x
    /// [0 10] gives EQUAL area increases d1 = d2 = 600 - 100 = 500, so
    /// the descent takes FIRST — leaf 3 pairs with leaf 1, and the
    /// in-order dump becomes (1,3,2). A `<` tie-break would pair 3 with
    /// 2 and produce (1,2,3) — the wrong-vs-right discriminator.
    #[test]
    fn t50_tie_descends_first() {
        let mut tree = MinAreaTree::new();
        tree.insert(1, &[box_shape(0, 0, 10, 10)]);
        tree.insert(2, &[box_shape(100, 0, 110, 10)]);
        assert_eq!(leaf_order(&tree), vec![(1, 0), (2, 0)]);
        tree.insert(3, &[box_shape(50, 0, 60, 10)]);

        assert_eq!(leaf_order(&tree), vec![(1, 0), (3, 0), (2, 0)]);
        assert_eq!(tree.leaf_count(), 3);
        assert_eq!(
            tree.dump_lines(),
            vec![
                "I box[0 0 110 10]",
                "    I box[0 0 60 10]",
                "        L obj=1 idx=0 box[0 0 10 10]",
                "        L obj=3 idx=0 box[50 0 60 10]",
                "    L obj=2 idx=0 box[100 0 110 10]",
            ]
        );
        // Capture: overlaps tie-center q=box[45 0 65 10] -> [(3,0)].
        assert_eq!(overlap_keys(&tree, &box_shape(45, 0, 65, 10)), vec![(3, 0)]);
        assert_bounds_tight(&tree);
    }

    // ---- T50: the eager union grows every VISITED inner node --------------

    /// Jar capture SCEN eager_right_orth: inserting 3=box[25 -5 40 5]
    /// descends RIGHT (d2 = 200 < d1 = 500), yet the ROOT — the left
    /// sibling's parent — has grown from box[0 0 30 10] to
    /// box[0 -5 40 10] by the EAGER union (MinAreaTree.java:94-95).
    /// Skipping the eager union leaves the root at box[0 0 30 10] —
    /// wrong differs from right in the dump.
    #[test]
    fn t50_eager_union_grows_sibling_parent() {
        let mut tree = MinAreaTree::new();
        tree.insert(1, &[box_shape(0, 0, 10, 10)]);
        tree.insert(2, &[box_shape(20, 0, 30, 10)]);
        assert_eq!(
            tree.dump_lines(),
            vec![
                "I box[0 0 30 10]",
                "    L obj=1 idx=0 box[0 0 10 10]",
                "    L obj=2 idx=0 box[20 0 30 10]",
            ]
        );
        tree.insert(3, &[box_shape(25, -5, 40, 5)]);

        assert_eq!(
            tree.dump_lines(),
            vec![
                "I box[0 -5 40 10]",
                "    L obj=1 idx=0 box[0 0 10 10]",
                "    I box[20 -5 40 10]",
                "        L obj=2 idx=0 box[20 0 30 10]",
                "        L obj=3 idx=0 box[25 -5 40 5]",
            ]
        );
        assert_eq!(leaf_order(&tree), vec![(1, 0), (2, 0), (3, 0)]);
        assert_bounds_tight(&tree);
    }

    /// Jar capture SCEN eager_deep_orth: a two-level descent (5 past 1
    /// at the root, then 3 past 2 at N1) must grow BOTH visited inner
    /// nodes. After inserts 1..4 the root is box[0 0 35 110] with N1 =
    /// box[0 0 10 110]; inserting 5=box[2 108 4 120] grows the root to
    /// box[0 0 35 120] AND N1 to box[0 0 10 120] — even though the
    /// eventual splice parent of the replaced leaf 3 is N1 itself, the
    /// ROOT's growth can only come from the eager union.
    #[test]
    fn t50_eager_union_every_visited_node() {
        let mut tree = MinAreaTree::new();
        tree.insert(1, &[box_shape(0, 0, 10, 10)]);
        tree.insert(2, &[box_shape(20, 0, 30, 10)]);
        tree.insert(3, &[box_shape(0, 100, 10, 110)]);
        tree.insert(4, &[box_shape(25, 95, 35, 105)]);
        assert_eq!(
            tree.dump_lines(),
            vec![
                "I box[0 0 35 110]",
                "    I box[0 0 10 110]",
                "        L obj=1 idx=0 box[0 0 10 10]",
                "        L obj=3 idx=0 box[0 100 10 110]",
                "    I box[20 0 35 105]",
                "        L obj=2 idx=0 box[20 0 30 10]",
                "        L obj=4 idx=0 box[25 95 35 105]",
            ]
        );
        tree.insert(5, &[box_shape(2, 108, 4, 120)]);

        assert_eq!(
            tree.dump_lines(),
            vec![
                "I box[0 0 35 120]",
                "    I box[0 0 10 120]",
                "        L obj=1 idx=0 box[0 0 10 10]",
                "        I box[0 100 10 120]",
                "            L obj=3 idx=0 box[0 100 10 110]",
                "            L obj=5 idx=0 box[2 108 4 120]",
                "    I box[20 0 35 105]",
                "        L obj=2 idx=0 box[20 0 30 10]",
                "        L obj=4 idx=0 box[25 95 35 105]",
            ]
        );
        assert_eq!(
            leaf_order(&tree),
            vec![(1, 0), (3, 0), (5, 0), (2, 0), (4, 0)]
        );
        assert_bounds_tight(&tree);
    }

    // ---- T51: history dependence + the strict-shrink remove loop ----------

    /// Jar capture SCEN history_orth / fresh_orth: the SAME final leaf
    /// set {B,C,D,E} built through "insert A,B,C,D; remove A; insert E"
    /// differs from the fresh insertion order B,C,D,E — the root's
    /// children swap ((D,(C,E)) with B second vs B first with
    /// ((C,E),D) second). Tree shape is history-dependent (T51); the
    /// dumps pin BOTH trees exactly, including the mid-sequence
    /// sibling-promotion state (remove A promotes D and C under N1 with
    /// the root unchanged — the shrink loop breaks on equal bounds).
    #[test]
    fn t51_history_differs_from_fresh() {
        let mut tree = MinAreaTree::new();
        let a = tree.insert(1, &[box_shape(0, 0, 10, 10)]);
        tree.insert(2, &[box_shape(100, 0, 110, 10)]);
        tree.insert(3, &[box_shape(45, 0, 55, 10)]);
        tree.insert(4, &[box_shape(0, 100, 10, 110)]);
        let history_abcd = vec![
            "I box[0 0 110 110]",
            "    I box[0 0 55 110]",
            "        I box[0 0 10 110]",
            "            L obj=1 idx=0 box[0 0 10 10]",
            "            L obj=4 idx=0 box[0 100 10 110]",
            "        L obj=3 idx=0 box[45 0 55 10]",
            "    L obj=2 idx=0 box[100 0 110 10]",
        ];
        assert_eq!(tree.dump_lines(), history_abcd);
        assert_bounds_tight(&tree);

        tree.remove_leaf(a[0]);
        assert_eq!(
            tree.dump_lines(),
            vec![
                "I box[0 0 110 110]",
                "    I box[0 0 55 110]",
                "        L obj=4 idx=0 box[0 100 10 110]",
                "        L obj=3 idx=0 box[45 0 55 10]",
                "    L obj=2 idx=0 box[100 0 110 10]",
            ]
        );
        assert_eq!(leaf_order(&tree), vec![(4, 0), (3, 0), (2, 0)]);

        tree.insert(5, &[box_shape(0, 0, 10, 10)]);
        let history = tree.dump_lines();
        assert_eq!(
            history,
            vec![
                "I box[0 0 110 110]",
                "    I box[0 0 55 110]",
                "        L obj=4 idx=0 box[0 100 10 110]",
                "        I box[0 0 55 10]",
                "            L obj=3 idx=0 box[45 0 55 10]",
                "            L obj=5 idx=0 box[0 0 10 10]",
                "    L obj=2 idx=0 box[100 0 110 10]",
            ]
        );

        let mut fresh = MinAreaTree::new();
        fresh.insert(2, &[box_shape(100, 0, 110, 10)]);
        fresh.insert(3, &[box_shape(45, 0, 55, 10)]);
        fresh.insert(4, &[box_shape(0, 100, 10, 110)]);
        fresh.insert(5, &[box_shape(0, 0, 10, 10)]);
        let fresh_dump = vec![
            "I box[0 0 110 110]",
            "    L obj=2 idx=0 box[100 0 110 10]",
            "    I box[0 0 55 110]",
            "        I box[0 0 55 10]",
            "            L obj=3 idx=0 box[45 0 55 10]",
            "            L obj=5 idx=0 box[0 0 10 10]",
            "        L obj=4 idx=0 box[0 100 10 110]",
        ];
        assert_eq!(fresh.dump_lines(), fresh_dump);

        // T51 discriminator: same leaf set, different tree.
        assert_ne!(history, fresh_dump);
        let mut history_keys = leaf_order(&tree);
        let mut fresh_keys = leaf_order(&fresh);
        history_keys.sort();
        fresh_keys.sort();
        assert_eq!(history_keys, fresh_keys, "same final leaf set");
        assert_bounds_tight(&tree);
        assert_bounds_tight(&fresh);
    }

    /// Jar capture SCEN shrink_loop_orth: removing a leaf whose extent
    /// is NOT covered by its sibling or uncle makes the strict-shrink
    /// loop ASSIGN — removing 5 shrinks N1 box[0 0 10 120] ->
    /// box[0 0 10 110] and the root box[0 0 35 120] -> box[0 0 35 110]
    /// (two levels); removing 4 then promotes 2 directly under the root
    /// and shrinks it again to box[0 0 30 110].
    #[test]
    fn t51_strict_shrink_loop_assigns() {
        let mut tree = MinAreaTree::new();
        let l1 = tree.insert(1, &[box_shape(0, 0, 10, 10)]);
        tree.insert(2, &[box_shape(20, 0, 30, 10)]);
        tree.insert(3, &[box_shape(0, 100, 10, 110)]);
        let l4 = tree.insert(4, &[box_shape(25, 95, 35, 105)]);
        let l5 = tree.insert(5, &[box_shape(2, 108, 4, 120)]);
        assert_eq!(
            leaf_order(&tree),
            vec![(1, 0), (3, 0), (5, 0), (2, 0), (4, 0)]
        );
        assert_eq!(l1.len(), 1);
        assert_eq!(l4.len(), 1);
        assert_eq!(l5.len(), 1);

        tree.remove_leaf(l5[0]);
        assert_eq!(
            tree.dump_lines(),
            vec![
                "I box[0 0 35 110]",
                "    I box[0 0 10 110]",
                "        L obj=1 idx=0 box[0 0 10 10]",
                "        L obj=3 idx=0 box[0 100 10 110]",
                "    I box[20 0 35 105]",
                "        L obj=2 idx=0 box[20 0 30 10]",
                "        L obj=4 idx=0 box[25 95 35 105]",
            ]
        );
        assert_eq!(tree.leaf_count(), 4);
        assert_bounds_tight(&tree);

        tree.remove_leaf(l4[0]);
        assert_eq!(
            tree.dump_lines(),
            vec![
                "I box[0 0 30 110]",
                "    I box[0 0 10 110]",
                "        L obj=1 idx=0 box[0 0 10 10]",
                "        L obj=3 idx=0 box[0 100 10 110]",
                "    L obj=2 idx=0 box[20 0 30 10]",
            ]
        );
        assert_eq!(tree.leaf_count(), 3);
        assert_bounds_tight(&tree);
    }

    // ---- remove edge cases -------------------------------------------------

    /// Jar capture SCEN remove_edges_orth: removing one of two leaves
    /// promotes the sibling to root; removing it empties the tree
    /// (overlaps on the empty tree returns nothing); a single-leaf tree
    /// removed through the root-leaf branch empties it too.
    #[test]
    fn remove_edges() {
        let mut tree = MinAreaTree::new();
        let one = tree.insert(1, &[box_shape(0, 0, 10, 10)]);
        let two = tree.insert(2, &[box_shape(20, 0, 30, 10)]);
        // Capture: overlaps two-leaf q=box[0 0 30 10] -> [(2,0),(1,0)] —
        // DESCENDING id, the TreeSet order (T52).
        assert_eq!(
            overlap_keys(&tree, &box_shape(0, 0, 30, 10)),
            vec![(2, 0), (1, 0)]
        );

        tree.remove_leaf(one[0]);
        assert_eq!(tree.leaf_count(), 1);
        assert_eq!(tree.dump_lines(), vec!["L obj=2 idx=0 box[20 0 30 10]"]);
        assert_eq!(tree.root().map(|r| tree.base.is_leaf(r)), Some(true));

        tree.remove_leaf(two[0]);
        assert_eq!(tree.leaf_count(), 0);
        assert!(tree.to_array().is_empty());
        assert_eq!(tree.root(), None);
        assert_eq!(
            overlap_keys(&tree, &box_shape(0, 0, 100, 100)),
            Vec::<(u64, u32)>::new()
        );

        // Single leaf as root: remove hits the parent==null branch.
        let solo = tree.insert(9, &[box_shape(50, 50, 60, 60)]);
        assert_eq!(
            overlap_keys(&tree, &box_shape(55, 55, 65, 65)),
            vec![(9, 0)]
        );
        tree.remove_leaf(solo[0]);
        assert_eq!(tree.leaf_count(), 0);
        assert_eq!(tree.root(), None);
    }

    /// `ShapeTree.remove` removes every leaf of an object; an empty
    /// entries slice is a no-op (Java `remove(null)` guard +
    /// ShapeTree.java:97-104).
    #[test]
    fn remove_object_entries() {
        let mut tree = MinAreaTree::new();
        let entries = tree.insert(7, &[box_shape(0, 0, 10, 10), box_shape(20, 0, 30, 10)]);
        tree.insert(3, &[box_shape(15, 0, 18, 10)]);
        tree.remove(&[]);
        assert_eq!(tree.leaf_count(), 3);
        tree.remove(&entries);
        assert_eq!(tree.leaf_count(), 1);
        assert_eq!(leaf_order(&tree), vec![(3, 0)]);
    }

    // ---- T52: overlaps dedup + order ---------------------------------------

    /// Jar capture SCEN overlaps_dedup_orth: object 7 (TWO shapes) plus
    /// objects 3 and 9. `overlaps` reports DESCENDING id with shape
    /// index ASC — [(7,0),(7,1),(3,0)] — border TOUCHING counts (the
    /// bbox prefilter is the inclusive `intersects`), and re-inserting
    /// the same object (six live leaves, `to_array` keeps every one)
    /// still yields each (7,i) ONCE: the TreeSet dedups on
    /// `Leaf.compareTo == 0` (ShapeTree.java:217-223).
    #[test]
    fn t52_overlaps_dedup_and_order() {
        let mut tree = MinAreaTree::new();
        let seven_shapes = [box_shape(0, 0, 10, 10), box_shape(20, 0, 30, 10)];
        tree.insert(7, &seven_shapes);
        tree.insert(3, &[box_shape(15, 0, 18, 10)]);
        tree.insert(9, &[box_shape(500, 0, 510, 10)]);
        assert_eq!(tree.leaf_count(), 4);
        assert_eq!(
            tree.dump_lines(),
            vec![
                "I box[0 0 510 10]",
                "    L obj=7 idx=0 box[0 0 10 10]",
                "    I box[15 0 510 10]",
                "        I box[20 0 510 10]",
                "            L obj=7 idx=1 box[20 0 30 10]",
                "            L obj=9 idx=0 box[500 0 510 10]",
                "        L obj=3 idx=0 box[15 0 18 10]",
            ]
        );

        // q=box[5 0 40 10] touches both 7-shapes and 3; 9 is far away.
        let wide = box_shape(5, 0, 40, 10);
        assert_eq!(overlap_keys(&tree, &wide), vec![(7, 0), (7, 1), (3, 0)]);
        // q=box[10 0 20 10] touches 7/0's right border and 7/1's left
        // border (inclusive) and covers 3 — same result.
        assert_eq!(
            overlap_keys(&tree, &box_shape(10, 0, 20, 10)),
            vec![(7, 0), (7, 1), (3, 0)]
        );

        // Re-insert the SAME object: to_array holds SIX leaves (an array,
        // no dedup), overlaps still reports each (7,i) once.
        tree.insert(7, &seven_shapes);
        assert_eq!(tree.leaf_count(), 6);
        assert_eq!(
            leaf_order(&tree),
            vec![(7, 0), (7, 0), (7, 1), (7, 1), (9, 0), (3, 0)]
        );
        assert_eq!(overlap_keys(&tree, &wide), vec![(7, 0), (7, 1), (3, 0)]);
        assert_bounds_tight(&tree);
    }

    /// The comparator is the ONLY object-ordering source (D17): the same
    /// tree under an ASCENDING comparator reports [(3,0),(7,0),(7,1)].
    #[test]
    fn overlaps_order_follows_comparator() {
        let mut tree = MinAreaTree::new();
        tree.insert(7, &[box_shape(0, 0, 10, 10), box_shape(20, 0, 30, 10)]);
        tree.insert(3, &[box_shape(15, 0, 18, 10)]);
        let wide = box_shape(5, 0, 40, 10);
        let ascending: Vec<(u64, u32)> = tree
            .overlaps(&wide, |a: u64, b: u64| a.cmp(&b))
            .into_iter()
            .map(|e| (e.object_key, e.shape_index_in_object))
            .collect();
        assert_eq!(ascending, vec![(3, 0), (7, 0), (7, 1)]);
    }

    // ---- 45-degree hulls + mixed octagon/box dispatch ----------------------

    /// Jar capture SCEN fortyfive_mixed: under
    /// FortyfiveDegreeBoundingDirections Java stores each box tree-shape
    /// as its octagon HULL (box[0 0 10 10] -> oct[0 0 10 10 -10 10 0
    /// 20]); the tie arithmetic carries over unchanged (hull areas equal
    /// box areas), the true-octagon leaf 4 unions through the (Oct,Oct)
    /// arms, and the mixed overlaps queries exercise the (Box,Oct) /
    /// (Oct,Box) intersects dispatch — including the captured
    /// box-vs-oct-leaf MISS (q=box[230 30 240 40] -> []).
    #[test]
    fn fortyfive_hulls_and_mixed_overlaps() {
        let mut tree = MinAreaTree::new();
        tree.insert(1, &[hull(0, 0, 10, 10)]);
        tree.insert(2, &[hull(100, 0, 110, 10)]);
        tree.insert(3, &[hull(50, 0, 60, 10)]);
        assert_eq!(leaf_order(&tree), vec![(1, 0), (3, 0), (2, 0)]);
        assert_eq!(
            tree.dump_lines(),
            vec![
                "I oct[0 0 110 10 -10 110 0 120]",
                "    I oct[0 0 60 10 -10 60 0 70]",
                "        L obj=1 idx=0 oct[0 0 10 10 -10 10 0 20]",
                "        L obj=3 idx=0 oct[50 0 60 10 40 60 50 70]",
                "    L obj=2 idx=0 oct[100 0 110 10 90 110 100 120]",
            ]
        );

        // A TRUE octagon (corners cut by 20): the exact constructor
        // values the capture printed for leaf 4.
        tree.insert(
            4,
            &[RegularTileShape::IntOctagon(IntOctagon::new(
                200, 0, 260, 60, /* ul */ 220, /* lr */ 300, /* ll */ 160,
                /* ur */ 240,
            ))],
        );
        assert_eq!(leaf_order(&tree), vec![(1, 0), (3, 0), (2, 0), (4, 0)]);
        assert_eq!(
            tree.dump_lines(),
            vec![
                "I oct[0 0 260 60 -10 300 0 240]",
                "    I oct[0 0 60 10 -10 60 0 70]",
                "        L obj=1 idx=0 oct[0 0 10 10 -10 10 0 20]",
                "        L obj=3 idx=0 oct[50 0 60 10 40 60 50 70]",
                "    I oct[100 0 260 60 90 300 100 240]",
                "        L obj=2 idx=0 oct[100 0 110 10 90 110 100 120]",
                "        L obj=4 idx=0 oct[200 0 260 60 220 300 160 240]",
            ]
        );
        assert_bounds_tight(&tree);

        // Captured mixed-dispatch results (Oct leaf vs Box query and
        // Box leaves vs Oct query).
        assert_eq!(
            overlap_keys(&tree, &box_shape(230, 30, 240, 40)),
            Vec::<(u64, u32)>::new()
        );
        let oct_query = RegularTileShape::IntOctagon(IntOctagon::new(5, -5, 15, 15, 5, 25, -5, 15));
        assert_eq!(overlap_keys(&tree, &oct_query), vec![(1, 0)]);
    }
}
