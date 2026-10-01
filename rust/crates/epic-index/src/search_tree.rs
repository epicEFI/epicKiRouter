//! The configured shape search tree: a [`MinAreaTree`] plus the
//! per-tree configuration Java carries on `board.searchtree.
//! ShapeSearchTree` (M2 Task 6).
//!
//! Java anchors: `ShapeSearchTree.java` — the package-private
//! constructor `(directions, board, compensatedClearanceClassNo)`
//! `:71-77`, `getKey` `:80-87`, `isClearanceCompensationUsed`
//! `:95-97`, `clearanceCompensationValue` `:104-114` (the FORMULA
//! itself lives caller-side in epic-board — D17) — and
//! `datastructures/ShapeTree.java:45-60` for the per-shape insert.
//! The three concrete variants are `ShapeSearchTree` (the base /
//! generic tree, constructed with `FortyfiveDegreeBoundingDirections`
//! by both the manager and `getAutorouteTree`'s no-restriction arm),
//! `ShapeSearchTree45Degree` and `ShapeSearchTree90Degree`.
//!
//! D17 boundary: this crate sees NO board types and NO rules. The tree
//! keeps only (a) the bounding-directions variant — an epic-geometry
//! concept — (b) the compensated clearance class NUMBER, and (c) a
//! process-unique object id standing in for Java's object identity
//! (see [`SEARCH_TREE_ID_COUNTER`]). Shapes arrive CALLER-PROVIDED as
//! `Option<TileShape>` lists (Java `getTreeShape` results, nulls
//! included); the directions hull is applied HERE, exactly where Java
//! applies it inside `ShapeTree.insert`.

use std::cmp::Ordering as CmpOrdering;
use std::sync::atomic::{AtomicU64, Ordering};

use epic_geometry::regular_tile_shape::RegularTileShape;
use epic_geometry::shape::ShapeBoundingDirections;
use epic_geometry::tile_shape::TileShape;

use crate::MinAreaTree;
use crate::shape_tree::{LeafEntry, NodeIdx, NodeKind};

/// The source of Java's tree OBJECT IDENTITY. `ItemSearchTreesInfo`
/// keys an item's per-tree entries and shapes by `tree == tree`
/// reference equality (ItemSearchTreesInfo.java — every lookup walks
/// the list comparing the tree REFERENCE), so after
/// `setClearanceCompensationUsed` replaces a class-0 tree with a
/// same-class-slot class-1 tree (or `resetCompensatedTrees` drops and
/// a later `getAutorouteTree` re-creates the same key), Java sees a
/// DIFFERENT tree and recomputes; a key-STRING-keyed port would
/// falsely hit the stale entries. Every [`SearchTree`] takes the next
/// value; ids are never reused.
static SEARCH_TREE_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

/// The concrete tree variant (which Java subclass the tree mirrors).
///
/// * [`SearchTreeVariant::Generic`] — the base `ShapeSearchTree`. Its
///   bounding directions are still 45-degree (every construction site
///   passes `FortyfiveDegreeBoundingDirections.INSTANCE`), but the
///   DRILL shape dispatch reads the board's angle restriction at CALL
///   time (caller-side, `ShapeSearchTree.calculateTreeShapes(DrillItem)`
///   `:885-893`).
/// * [`SearchTreeVariant::FortyfiveDegree`] — `ShapeSearchTree45Degree`
///   (`ShapeSearchTree45Degree.java:30-31`).
/// * [`SearchTreeVariant::NinetyDegree`] — `ShapeSearchTree90Degree`
///   (`ShapeSearchTree90Degree.java:27-28`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SearchTreeVariant {
    /// Java base `ShapeSearchTree` (45-degree directions).
    Generic,
    /// Java `ShapeSearchTree45Degree`.
    FortyfiveDegree,
    /// Java `ShapeSearchTree90Degree`.
    NinetyDegree,
}

impl SearchTreeVariant {
    /// The bounding directions every Java construction site of this
    /// variant passes: the 45-degree subclass and the generic tree
    /// both use `FortyfiveDegreeBoundingDirections.INSTANCE`; only the
    /// 90-degree subclass uses `OrthogonalBoundingDirections.INSTANCE`.
    #[must_use]
    pub fn bounding_directions(self) -> ShapeBoundingDirections {
        match self {
            SearchTreeVariant::Generic | SearchTreeVariant::FortyfiveDegree => {
                ShapeBoundingDirections::FortyfiveDegree
            }
            SearchTreeVariant::NinetyDegree => ShapeBoundingDirections::Orthogonal,
        }
    }

    /// The `class.getSimpleName()` half of Java `getKey`
    /// (`ShapeSearchTree.java:80-87`).
    #[must_use]
    pub fn class_simple_name(self) -> &'static str {
        match self {
            SearchTreeVariant::Generic => "ShapeSearchTree",
            SearchTreeVariant::FortyfiveDegree => "ShapeSearchTree45Degree",
            SearchTreeVariant::NinetyDegree => "ShapeSearchTree90Degree",
        }
    }

    /// The `directions.getSimpleName()` half of Java `getKey` with the
    /// `"BoundingDirections"` suffix stripped
    /// (`ShapeSearchTree.java:83-85`).
    #[must_use]
    pub fn directions_simple_name(self) -> &'static str {
        match self.bounding_directions() {
            ShapeBoundingDirections::FortyfiveDegree => "FortyfiveDegree",
            ShapeBoundingDirections::Orthogonal => "Orthogonal",
        }
    }
}

/// One query hit — Java `ShapeTree.TreeEntry` (`ShapeTree.java:158-168`)
/// minus the object reference: the caller's opaque object key plus the
/// shape index. The Java entry carries `(object, shapeIndexInObject)`;
/// the index crate never sees objects, so `object_key` is the handle
/// the CALLER resolves (epic-board: `ItemId`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TreeEntry {
    /// Java `TreeEntry.object` (as the caller's object key).
    pub object_key: u64,
    /// Java `TreeEntry.shapeIndexInObject`.
    pub shape_index_in_object: u32,
}

/// A configured search tree (Java `ShapeSearchTree` and subclasses).
// Deliberately NOT `Clone`: a copy would alias `object_id`, and tree
// entries are identity-keyed (Java reference equality) — a cloned tree
// handed to a manager would corrupt the entry bookkeeping (T6 quality
// review NIT-3).
#[derive(Debug)]
pub struct SearchTree {
    /// The min-area BVH (T5).
    tree: MinAreaTree,
    /// The Java subclass this tree mirrors.
    pub variant: SearchTreeVariant,
    /// Java `compensatedClearanceClassNo` (`ShapeSearchTree.java:61`)
    /// — a plain class NUMBER here; the rules semantics (the
    /// compensation formula, `ShapeSearchTree.java:104-114`) live in
    /// epic-board.
    pub compensated_clearance_class: i32,
    /// Process-unique identity (Java object reference).
    object_id: u64,
    /// Java `EntrySortedByClearance.lastGeneratedEntryId`
    /// (`ShapeSearchTree.java:55`): the STATIC tie counter behind the
    /// with-clearance query's sort order. Java's is a static field on
    /// the inner class (one counter for ALL trees of the JVM); the M2
    /// decision is TREE-OWNED persistent state — the observable is the
    /// WITHIN-QUERY tie (equal clearances come out in construction
    /// order, i.e. descending item id on a board), which a per-tree
    /// counter reproduces exactly. The `Integer.MAX_VALUE` wrap-to-0
    /// quirk (`:1144-1148`, the wrap-reset assignment inside the
    /// constructor) is ported but deliberately NOT pinned: the
    /// wrap needs 2^31 prior queries.
    last_generated_entry_id: i32,
}

impl SearchTree {
    /// Java `ShapeSearchTree(directions, board, compensatedClassNo)`
    /// (`:71-77`) — the empty tree. `variant` fixes the directions
    /// (every Java construction site pairs them as
    /// [`SearchTreeVariant::bounding_directions`] documents).
    #[must_use]
    pub fn new(variant: SearchTreeVariant, compensated_clearance_class: i32) -> Self {
        Self {
            tree: MinAreaTree::new(),
            variant,
            compensated_clearance_class,
            object_id: SEARCH_TREE_ID_COUNTER.fetch_add(1, Ordering::Relaxed),
            last_generated_entry_id: 0,
        }
    }

    /// This tree's unique identity (Java reference equality; see
    /// [`SEARCH_TREE_ID_COUNTER`]). NEVER compare trees by key or
    /// (variant, class) — Java's entry lookups are identity-based and
    /// a re-created tree must not inherit its predecessor's entries.
    #[must_use]
    pub fn object_id(&self) -> u64 {
        self.object_id
    }

    /// Java `getKey()` (`ShapeSearchTree.java:80-87`):
    /// `"<class>_<directions>_cc<classNo>"`. Captured forms (the Task 6
    /// jar spike): `ShapeSearchTree_FortyfiveDegree_cc0`,
    /// `ShapeSearchTree45Degree_FortyfiveDegree_cc0`,
    /// `ShapeSearchTree90Degree_Orthogonal_cc0`,
    /// `ShapeSearchTree45Degree_FortyfiveDegree_cc1`,
    /// `ShapeSearchTree_FortyfiveDegree_cc1`.
    #[must_use]
    pub fn key(&self) -> String {
        format!(
            "{}_{}_cc{}",
            self.variant.class_simple_name(),
            self.variant.directions_simple_name(),
            self.compensated_clearance_class
        )
    }

    /// Java `isClearanceCompensationUsed()` (`:95-97`):
    /// `compensatedClearanceClassNo > 0`. This mirrors the DERIVED
    /// property only — the manager's sticky `clearanceCompensationUsed`
    /// FLAG (which drives tree rebuilding) is manager state, not tree
    /// state.
    #[must_use]
    pub fn is_clearance_compensation_used(&self) -> bool {
        self.compensated_clearance_class > 0
    }

    /// Java `ShapeTree.insert(Storable)` + the per-shape
    /// `insert(object, index)` (`ShapeTree.java:31-60`), with the
    /// caller providing the tree shapes (Java `getTreeShape(tree, i)`
    /// — the per-item shape cache is epic-board's, Task 7):
    ///
    /// * an EMPTY shape list is a no-op that stores NO entries (Java
    ///   `shapeCount <= 0` returns before `setSearchTreeEntries`),
    /// * a `None` slot stays `None` in the result (Java's null shape
    ///   returns a null Leaf — the entry array keeps the slot so
    ///   remove can skip it),
    /// * a `Some` shape is hulled into this tree's bounding directions
    ///   HERE (Java `boundingShape(boundingDirections)` inside
    ///   `ShapeTree.insert`; a null hull — an unbounded simplex — also
    ///   yields `None`, Java warns and returns null).
    ///
    /// Returns the per-shape leaves, index-aligned with the input.
    pub fn insert(
        &mut self,
        object_key: u64,
        shapes: &[Option<TileShape>],
    ) -> Vec<Option<NodeIdx>> {
        let mut leaves = Vec::with_capacity(shapes.len());
        for (index, shape) in shapes.iter().enumerate() {
            leaves.push(self.insert_one(object_key, index as u32, shape.as_ref()));
        }
        leaves
    }

    /// Java `ShapeSearchTree.insert(item, index)` — ONE shape at an
    /// EXPLICIT entry index (the merge fast paths insert the link
    /// entries at their positions in the NEW entry list,
    /// ShapeSearchTree.java:235/:309/:161). The shape is hulled into
    /// this tree's directions exactly like the list form; an
    /// unbounded shape stores no leaf (Java's null Leaf) and returns
    /// [`None`].
    pub fn insert_one(
        &mut self,
        object_key: u64,
        shape_index_in_object: u32,
        shape: Option<&TileShape>,
    ) -> Option<NodeIdx> {
        let directions = self.variant.bounding_directions();
        let bounds = shape.and_then(|shape| shape.bounding_shape(&directions))?;
        let leaf = self
            .tree
            .base
            .push_leaf(object_key, shape_index_in_object, bounds);
        self.tree.insert_leaf(leaf);
        Some(leaf)
    }

    /// Java's in-place owner re-attribution on SURVIVING leaves
    /// (`leaf.object = toTrace; leaf.shapeIndexInObject = i` —
    /// ShapeSearchTree.java:214-215, :221, :294-295, :150,
    /// :325-327): no structural op, no bounds change — the leaf keeps
    /// its exact tree position while the (owner, index) pair moves.
    /// Queries report the NEW owner afterwards, and a later
    /// [`SearchTree::remove`] of the new owner's entry list drops the
    /// same slab slot Java would drop.
    pub fn relabel_leaf(&mut self, node: NodeIdx, object_key: u64, shape_index_in_object: u32) {
        self.tree
            .base
            .relabel_leaf(node, object_key, shape_index_in_object);
    }

    /// Removes every leaf of `entries` (Java `ShapeTree.remove`,
    /// `ShapeTree.java:97-104`); `None` slots are skipped (a null
    /// entry holds no leaf).
    pub fn remove(&mut self, entries: &[Option<NodeIdx>]) {
        for leaf in entries.iter().flatten() {
            self.tree.remove_leaf(*leaf);
        }
    }

    /// Java `ShapeTree.size()` — live leaves in the BVH.
    #[must_use]
    pub fn leaf_count(&self) -> usize {
        self.tree.leaf_count()
    }

    /// The underlying BVH. Not used by any production query (they go
    /// through the query surfaces above); exposed as read access for
    /// the M2 T9 epic-harness white-box dumps that need to walk leaves
    /// directly.
    #[must_use]
    pub fn min_area_tree(&self) -> &MinAreaTree {
        &self.tree
    }

    // -----------------------------------------------------------------
    // The overlap query family (M2 Task 8; Java
    // ShapeSearchTree.java:390-570). The CANDIDATE halves live here;
    // the object/layer/net filters and the stored-shape fetch live
    // caller-side in epic-board (D17 — this crate sees no items).
    // -----------------------------------------------------------------

    /// The candidate half of Java `overlappingTreeEntries`
    /// (`ShapeSearchTree.java:399-405`): the query hulled into this
    /// tree's bounding directions, then the leaves whose stored BOUNDS
    /// intersect that hull — in the `TreeSet<Leaf>` order (the
    /// caller-supplied object comparator, then shape index ASC, dedup;
    /// [`MinAreaTree::overlaps`]).
    ///
    /// `None` = the query is not bounded in this tree's directions
    /// (Java warns "shape not bounded" and returns an empty result).
    ///
    /// NOTE (brief correction): the entries do NOT come out in raw
    /// DFS order — Java collects the candidates into a `TreeSet<Leaf>`
    /// BEFORE the per-leaf loop (`:404`), so the result order IS the
    /// sorted order. Capture: `A_LEAVES box
    /// [(815#0),(815#1),(787#0),(786#0),...]` — descending item id.
    pub fn query_candidates<CMP>(&self, shape: &TileShape, cmp: CMP) -> Option<Vec<LeafEntry>>
    where
        CMP: Fn(u64, u64) -> CmpOrdering,
    {
        let directions = self.variant.bounding_directions();
        let bounds = shape.bounding_shape(&directions)?;
        Some(self.tree.overlaps(&bounds, cmp))
    }

    /// The exact-test half of Java `overlappingTreeEntries`
    /// (`ShapeSearchTree.java:421-433`) — the **T53 octagon-skip**:
    ///
    /// * if the QUERY shape is an `IntOctagon` AND the stored shape is
    ///   an `IntOctagon`, the candidate pass already proved overlap
    ///   (the 45-degree hull of an octagon IS the octagon), so the
    ///   intersection test is SKIPPED and the entry counts as a hit —
    ///   even when the two octagons are actually DISJOINT corner
    ///   regions within overlapping bounding hulls (capture section D:
    ///   query `oct[800 800 1000 1000 ...]` against the stored
    ///   `oct[0 0 1000 1000 ... 0 1500]` — `intersects` is false, the
    ///   query still returns the entry),
    /// * otherwise the stored shape's exact `intersects` against the
    ///   query (Java's receiver order: `currentShape.intersects(shape)`).
    ///
    /// The skip is unobservable through every PRODUCTION tree
    /// configuration (a 45-degree tree's octagon bounds make
    /// candidate ⟺ exact for octagon queries; a 90-degree tree never
    /// stores octagons) — it fires only where a NON-45-degree tree
    /// holds a genuine octagon leaf, which the epic-index test below
    /// constructs directly.
    #[must_use]
    pub fn entry_intersects(query: &TileShape, stored: &TileShape) -> bool {
        if matches!(
            (query, stored),
            (
                TileShape::RegularTileShape(RegularTileShape::IntOctagon(_)),
                TileShape::RegularTileShape(RegularTileShape::IntOctagon(_))
            )
        ) {
            return true;
        }
        stored.intersects(query)
    }

    /// The candidate half of Java
    /// `overlappingTreeEntriesWithClearance` (`:456-467`): the query
    /// hull (falling back to `fallback_bounds` — Java
    /// `board.getBoundingBox()` — when the query is unbounded; Java
    /// warns first), offset by `max_clearance`, then the sorted
    /// candidates. `max_clearance` arrives pre-truncated (Java
    /// `(int)(1.2 * maxValue)` — the epic-board wrapper owns the
    /// arithmetic).
    ///
    /// An unbounded query with no fallback yields no candidates.
    pub fn clearance_candidates<CMP>(
        &self,
        shape: &TileShape,
        max_clearance: i32,
        fallback_bounds: Option<&RegularTileShape>,
        cmp: CMP,
    ) -> Vec<LeafEntry>
    where
        CMP: Fn(u64, u64) -> CmpOrdering,
    {
        let directions = self.variant.bounding_directions();
        let bounds = shape
            .bounding_shape(&directions)
            .or(fallback_bounds.cloned());
        let Some(bounds) = bounds else {
            return Vec::new();
        };
        let offset_bounds = bounds.offset(f64::from(max_clearance));
        self.tree.overlaps(&offset_bounds, cmp)
    }

    /// The sift half of Java `overlappingTreeEntriesWithClearance`
    /// (`:469-507`): the candidates (each with its clearance value
    /// and its STORED shape) sorted by
    /// `EntrySortedByClearance.compareTo` — clearance ASC, tie
    /// `entryId` ASC (`:1152-1159`) — then the half-clearance
    /// stepping walk:
    ///
    /// ```text
    /// currentHalfClearance = 0, currentOffsetShape = the RAW query
    /// for each entry (clearance order):
    ///     if clearance/2 != currentHalfClearance:          // Java int /2
    ///         currentHalfClearance = clearance/2
    ///         currentOffsetShape = query.enlarge(half)
    ///     accept iff currentOffsetShape.intersects(stored.enlarge(half))
    /// ```
    ///
    /// The `enlarge`s take the half clearance as DOUBLE (Java's int
    /// argument promotes through `enlarge(double)`), and the query
    /// starts UN-enlarged — an entry whose half-clearance is 0 is
    /// tested raw-vs-raw.
    ///
    /// The tie counter is this tree's persistent
    /// [`SearchTree::last_generated_entry_id`] — ids are allocated in
    /// the candidates' arrival order, so equal-clearance entries keep
    /// their candidate (descending-id) order. Mutation-verified: an
    /// ascending-id tie sort fails the capture pin below.
    pub fn clearance_test(
        &mut self,
        query: &TileShape,
        candidates: &[(LeafEntry, i32, TileShape)],
    ) -> Vec<TreeEntry> {
        // The TreeSet sort keys: (clearance, entryId). entryIds are
        // allocated HERE, in candidate arrival order — the only order
        // Java can construct them in. The keys Vec is the pooled
        // per-thread scratch (slice B): cleared per call, capacity
        // retained.
        crate::scratch::with_clearance_sort_keys(|sorted| {
            sorted.clear();
            for (index, (_, clearance, _)) in candidates.iter().enumerate() {
                self.last_generated_entry_id = if self.last_generated_entry_id == i32::MAX {
                    0
                } else {
                    self.last_generated_entry_id + 1
                };
                sorted.push((*clearance, self.last_generated_entry_id, index));
            }
            // (clearance, entryId) is a total order — ids are unique — so
            // an unstable sort cannot reorder ties arbitrarily.
            sorted.sort_unstable();

            let mut result = Vec::new();
            let mut current_half_clearance = 0;
            // Java :490-491: the initial offset shape is the RAW query —
            // NOT enlarge(0).
            let mut current_offset_shape = query.clone();
            for (clearance, _entry_id, index) in sorted.iter().copied() {
                let (leaf, _leaf_clearance, stored) = &candidates[index];
                // Rust `/` and Java int `/` both truncate toward zero —
                // identical results for EVERY sign combination, so the
                // port is exact by construction (the clearances here are
                // non-negative regardless). Do not "fix" this into a
                // flooring divide; that would diverge on negatives.
                let tmp_half_clearance = clearance / 2;
                if tmp_half_clearance != current_half_clearance {
                    current_half_clearance = tmp_half_clearance;
                    current_offset_shape = query.enlarge(f64::from(tmp_half_clearance));
                }
                let stored_offset = stored.enlarge(f64::from(current_half_clearance));
                if current_offset_shape.intersects(&stored_offset) {
                    result.push(TreeEntry {
                        object_key: leaf.object_key,
                        shape_index_in_object: leaf.shape_index_in_object,
                    });
                }
            }
            result
        })
    }

    /// Java `ShapeSearchTree.validateEntries(Item)`
    /// (`:1120-1130`): every stored leaf's `shapeIndexInObject` equals
    /// its slot index in the entry array.
    ///
    /// DIVERGENCE: Java dereferences the array unguarded — an item
    /// with NO entry array NPEs (capture `E_ABSENT id=384 →
    /// NullPointerException`) and a NULL SLOT (a copper-less padstack
    /// layer) NPEs at `currentLeaf.shapeIndexInObject`. The port
    /// treats a `None` slot as SKIPPED (null leaves carry no shape
    /// index to check); the absent-array case is the caller-side
    /// `Option` (epic-board holds the entry arrays as
    /// `Option<Vec<Option<NodeIdx>>>`).
    pub fn validate_entries(&self, entries: &[Option<NodeIdx>]) -> bool {
        entries.iter().enumerate().all(|(index, slot)| {
            slot.is_none_or(|node| {
                match &self.tree.base.node(node).kind {
                    NodeKind::Leaf {
                        shape_index_in_object,
                        ..
                    } => *shape_index_in_object as usize == index,
                    // An inner node in an entry slot is corrupt data,
                    // not an index mismatch — Java's field read would
                    // silently read garbage; treat as inconsistent.
                    NodeKind::Inner { .. } => false,
                }
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use epic_geometry::int_box::IntBox;
    use epic_geometry::regular_tile_shape::RegularTileShape;

    fn box_shape(llx: i32, lly: i32, urx: i32, ury: i32) -> TileShape {
        TileShape::RegularTileShape(RegularTileShape::IntBox(IntBox::from_corners(
            llx, lly, urx, ury,
        )))
    }

    /// Java `getKey` (`:80-87`) — the five captured key forms from the
    /// Task 6 jar spike (`TreeShapesSpike.java`, the `tree=` field of
    /// every SHAPES line). A port that derives the directions name
    /// from the VARIANT NAME (45Degree->FortyfiveDegree) instead of
    /// the directions the variant constructs with, or forgets the
    /// `"BoundingDirections"` strip, diverges here.
    #[test]
    fn key_mirrors_get_key_forms() {
        assert_eq!(
            SearchTree::new(SearchTreeVariant::Generic, 0).key(),
            "ShapeSearchTree_FortyfiveDegree_cc0"
        );
        assert_eq!(
            SearchTree::new(SearchTreeVariant::FortyfiveDegree, 0).key(),
            "ShapeSearchTree45Degree_FortyfiveDegree_cc0"
        );
        assert_eq!(
            SearchTree::new(SearchTreeVariant::NinetyDegree, 0).key(),
            "ShapeSearchTree90Degree_Orthogonal_cc0"
        );
        assert_eq!(
            SearchTree::new(SearchTreeVariant::FortyfiveDegree, 1).key(),
            "ShapeSearchTree45Degree_FortyfiveDegree_cc1"
        );
        assert_eq!(
            SearchTree::new(SearchTreeVariant::Generic, 1).key(),
            "ShapeSearchTree_FortyfiveDegree_cc1"
        );
    }

    /// Java `isClearanceCompensationUsed()` (`:95-97`): derived from
    /// the class number alone.
    #[test]
    fn compensation_flag_follows_class_number() {
        assert!(!SearchTree::new(SearchTreeVariant::Generic, 0).is_clearance_compensation_used());
        assert!(
            SearchTree::new(SearchTreeVariant::FortyfiveDegree, 1).is_clearance_compensation_used()
        );
        assert!(SearchTree::new(SearchTreeVariant::Generic, 2).is_clearance_compensation_used());
    }

    /// Object identity is UNIQUE per tree and never equal for two
    /// same-configuration trees — the property Java's
    /// `ItemSearchTreesInfo` entry lookups depend on (`tree == tree`)
    /// and a key-string-keyed port would lose.
    #[test]
    fn object_ids_are_unique_even_for_identical_configuration() {
        let a = SearchTree::new(SearchTreeVariant::Generic, 0);
        let b = SearchTree::new(SearchTreeVariant::Generic, 0);
        assert_eq!(a.key(), b.key());
        assert_ne!(a.object_id(), b.object_id(), "same key, distinct trees");
    }

    /// The insert contract (`ShapeTree.java:31-60`): an empty shape
    /// list stores NO entries; `None` slots keep their index as `None`;
    /// `Some` shapes are hulled into the tree's directions (the
    /// 45-degree tree stores the octagon HULL of a box — captured
    /// convention `box[0 0 10 10]` -> `oct[0 0 10 10 -10 10 0 20]`,
    /// T5) while the 90-degree tree keeps the box; remove skips the
    /// null slots and empties the tree.
    #[test]
    fn insert_preserves_null_slots_and_hulls_per_directions() {
        let mut fortyfive = SearchTree::new(SearchTreeVariant::Generic, 0);
        let shapes = vec![Some(box_shape(0, 0, 10, 10)), None];
        let entries = fortyfive.insert(7, &shapes);
        assert_eq!(entries.len(), 2, "index-aligned entry slots");
        assert!(entries[0].is_some(), "the real shape got a leaf");
        assert_eq!(entries[1], None, "the null shape keeps a null slot");
        assert_eq!(fortyfive.leaf_count(), 1);
        // The stored hull is the octagon, verified through the dump.
        let dump = fortyfive.min_area_tree().dump_lines();
        assert_eq!(
            dump,
            vec!["L obj=7 idx=0 oct[0 0 10 10 -10 10 0 20]"],
            "45-degree directions store the octagon hull"
        );

        let mut ninety = SearchTree::new(SearchTreeVariant::NinetyDegree, 0);
        ninety.insert(7, &shapes);
        assert_eq!(
            ninety.min_area_tree().dump_lines(),
            vec!["L obj=7 idx=0 box[0 0 10 10]"],
            "orthogonal directions keep the box"
        );

        // An empty shape list: no entries at all (Java shapeCount<=0).
        assert!(fortyfive.insert(8, &[]).is_empty());
        assert_eq!(fortyfive.leaf_count(), 1);

        // Remove skips the null slot and empties the tree.
        fortyfive.remove(&entries);
        assert_eq!(fortyfive.leaf_count(), 0);
    }

    // -----------------------------------------------------------------
    // Task 8 — the overlap query family
    // (/tmp/epic-t8-query.out, the QuerySpike capture)
    // -----------------------------------------------------------------

    /// The capture's octagon field order: `oct[lx ly rx uy ulx lrx llx
    /// urx]`.
    fn oct_shape(o: epic_geometry::int_octagon::IntOctagon) -> TileShape {
        TileShape::RegularTileShape(RegularTileShape::IntOctagon(o))
    }

    /// **T53 — the octagon-skip** (`ShapeSearchTree.java:421-427`),
    /// capture section D: an OCTAGON query against an OCTAGON leaf in
    /// a NON-45-degree tree returns the entry even when the two
    /// octagons are DISJOINT (`D_Q_INTERSECTS_O false`,
    /// `D_ENTRIES oct-query → [(7#0)]`), while the same-region BOX
    /// query against the same leaf returns NOTHING
    /// (`D_ENTRIES box-query → []`). The skip condition is the
    /// mutation target: an exact-test-always port fails the first
    /// assertion, an always-skip port fails the second.
    #[test]
    fn octagon_skip_returns_disjoint_candidates_but_not_boxes() {
        // The spike's stored octagon O = oct[0 0 1000 1000 -500 500 0 1500]
        // in a REAL ShapeSearchTree90Degree (orthogonal directions, so
        // the leaf bounds are O's bounding box, not O itself).
        let stored = oct_shape(epic_geometry::int_octagon::IntOctagon::new(
            0, 0, 1000, 1000, -500, 500, 0, 1500,
        ));
        let mut ninety = SearchTree::new(SearchTreeVariant::NinetyDegree, 0);
        ninety.insert(7, &[Some(stored.clone())]);

        // Q: the bounding octagon of box[800 800 1000 1000] — its bbox
        // shares the corner region with O's leaf bounds, but its region
        // (x+y >= 1600) misses O's diagonal face (x+y <= 1500).
        let query_box = box_shape(800, 800, 1000, 1000);
        let query_oct = query_box.bounding_octagon().expect("bounded box");
        let query_oct = TileShape::RegularTileShape(RegularTileShape::IntOctagon(query_oct));

        // The witness: the exact test is FALSE...
        assert!(
            !query_oct.intersects(&stored),
            "D_Q_INTERSECTS_O false — the shapes are disjoint"
        );
        // ...the octagon query still yields the leaf...
        let candidates = ninety
            .query_candidates(&query_oct, |a, b| b.cmp(&a))
            .expect("bounded query");
        assert_eq!(
            candidates.len(),
            1,
            "the corner region overlaps the leaf bounds"
        );
        assert_eq!(candidates[0].object_key, 7);
        assert!(
            SearchTree::entry_intersects(&query_oct, &stored),
            "the skip fires"
        );
        // ...and the BOX query over the same region does not (no skip,
        // the exact test runs and rejects).
        let candidates = ninety
            .query_candidates(&query_box, |a, b| b.cmp(&a))
            .expect("bounded query");
        assert_eq!(candidates.len(), 1, "the box is a candidate too");
        assert!(
            !SearchTree::entry_intersects(&query_box, &stored),
            "D_ENTRIES box-query → [] — no octagon query, no skip"
        );

        // The genuinely-overlapping control (D_CTRL): both query forms
        // hit.
        let control = box_shape(0, 0, 200, 200);
        assert!(SearchTree::entry_intersects(&control, &stored));
        let control_oct = TileShape::RegularTileShape(RegularTileShape::IntOctagon(
            control.bounding_octagon().expect("bounded box"),
        ));
        assert!(SearchTree::entry_intersects(&control_oct, &stored));

        // The production-tree control (D_ENTRIES_45 → []): in a
        // 45-degree tree the leaf bounds ARE the octagon, so the
        // disjoint octagon query never even becomes a candidate — the
        // skip is unreachable there.
        let mut fortyfive = SearchTree::new(SearchTreeVariant::FortyfiveDegree, 0);
        fortyfive.insert(8, &[Some(stored.clone())]);
        let candidates = fortyfive
            .query_candidates(&query_oct, |a, b| b.cmp(&a))
            .expect("bounded query");
        assert!(
            candidates.is_empty(),
            "D_ENTRIES_45 oct-query → [] — the 45-degree hull is the octagon itself"
        );
    }

    /// **The tie order of the with-clearance sift**
    /// (`EntrySortedByClearance.compareTo`, `:1152-1159`; capture
    /// `C_TIE layer=0 nets=[] → [(3#0),(2#0),(1#0)]` and
    /// `C_TIE_AGAIN → [(3#0),(2#0),(1#0)]`): equal clearances come out
    /// in CANDIDATE ARRIVAL order (entryIds are allocated in that
    /// order and compared ASCENDING) — on a board that is descending
    /// item id. A port sorting ties by item id ASCENDING fails the
    /// first assertion; a port whose counter resets per query fails
    /// the AGAIN form only if it also reorders — the arrival-order
    /// pin covers both.
    #[test]
    fn clearance_tie_keeps_candidate_arrival_order() {
        let query = box_shape(0, 0, 100, 100);
        let shape_of = |key: u64| box_shape((key as i32) * 10, 0, (key as i32) * 10 + 5, 5);
        let row = |key: u64| {
            (
                LeafEntry {
                    node: NodeIdx::from_index(0),
                    object_key: key,
                    shape_index_in_object: 0,
                    bounds: RegularTileShape::IntBox(IntBox::from_corners(0, 0, 1, 1)),
                },
                5,
                shape_of(key),
            )
        };
        // Descending arrival (the board order): stays descending.
        let mut tree = SearchTree::new(SearchTreeVariant::Generic, 0);
        let hits = tree.clearance_test(&query, &[row(3), row(2), row(1)]);
        assert_eq!(
            hits.iter().map(|h| h.object_key).collect::<Vec<_>>(),
            vec![3, 2, 1],
            "C_TIE → [(3#0),(2#0),(1#0)] — arrival order under ties"
        );
        // Ascending arrival: stays ascending (ids follow arrival).
        let hits = tree.clearance_test(&query, &[row(1), row(2), row(3)]);
        assert_eq!(
            hits.iter().map(|h| h.object_key).collect::<Vec<_>>(),
            vec![1, 2, 3],
            "the counter allocates in ARRIVAL order, not key order"
        );
        // Unequal clearances sort by clearance ASC regardless of
        // arrival: (2, c=9) then (1, c=4) comes out [1, 2].
        let mut one = row(1);
        one.1 = 4;
        let mut two = row(2);
        two.1 = 9;
        let hits = tree.clearance_test(&query, &[two, one]);
        assert_eq!(
            hits.iter().map(|h| h.object_key).collect::<Vec<_>>(),
            vec![1, 2],
            "clearance ASC dominates the tie id"
        );
    }

    /// **The half-clearance stepping** (`:470-507`): each entry is
    /// tested with the query enlarged by THAT entry's `clearance / 2`
    /// (Java int division — clearance 1 means half 0, the RAW query),
    /// and the stored shape enlarged by the SAME half — the effective
    /// reach is the SUM of the halves. A port testing raw-vs-raw (no
    /// enlarge) rejects the first row; a port enlarging by the FULL
    /// clearance accepts the second; a port enlarging by 1 on a
    /// clearance-1 entry accepts the third.
    #[test]
    fn clearance_test_enlarges_both_sides_by_the_half_clearance() {
        let query = box_shape(0, 0, 100, 100);
        let leaf_of = |key: u64| LeafEntry {
            node: NodeIdx::from_index(0),
            object_key: key,
            shape_index_in_object: 0,
            bounds: RegularTileShape::IntBox(IntBox::from_corners(0, 0, 1, 1)),
        };
        // Gap 1 along x, vertically centered (out of the octagon
        // corner-cut regions). Raw: disjoint.
        let stored = box_shape(101, 40, 106, 60);
        assert!(!query.intersects(&stored), "the raw shapes are disjoint");

        // clearance 30 (half 15): reach 15+15 = 30 > gap 1 — accept.
        let mut tree = SearchTree::new(SearchTreeVariant::Generic, 0);
        let hits = tree.clearance_test(&query, &[(leaf_of(1), 30, stored.clone())]);
        assert_eq!(hits.len(), 1, "half 15 reaches across the gap");

        // clearance 10 (half 5): reach 10 — still > 1, accept (the
        // sum-of-halves semantics); the REJECT discriminator needs a
        // gap beyond the reach, below.
        let hits = tree.clearance_test(&query, &[(leaf_of(2), 10, stored.clone())]);
        assert_eq!(hits.len(), 1, "both sides enlarge — 5+5 > 1");

        // Gap 11 vs reach 10: rejected.
        let far = box_shape(111, 40, 116, 60);
        let hits = tree.clearance_test(&query, &[(leaf_of(3), 10, far)]);
        assert!(hits.is_empty(), "half 5+5 does not reach a gap of 11");

        // clearance 1 (half 0 — Java int division): the RAW query
        // against the raw stored shape (Java :490-491 keeps the
        // un-enlarged start); gap 1 stays a gap.
        let hits = tree.clearance_test(&query, &[(leaf_of(4), 1, stored.clone())]);
        assert!(hits.is_empty(), "clearance 1 → half 0 → raw vs raw");

        // clearance 2 (half 1): reach 2 > gap 1 — the touching face
        // accepts (closed shapes).
        let hits = tree.clearance_test(&query, &[(leaf_of(5), 2, stored)]);
        assert_eq!(hits.len(), 1, "half 1+1 reaches across the gap of 1");
    }

    /// `validateEntries` (`:1120-1130`): aligned entries validate,
    /// a slot swap does not, null slots are skipped (the documented
    /// divergence — Java NPEs on them), and an absent list is
    /// vacuously true.
    #[test]
    fn validate_entries_checks_slot_alignment_and_skips_nulls() {
        let mut tree = SearchTree::new(SearchTreeVariant::Generic, 0);
        let entries = tree.insert(
            9,
            &[
                Some(box_shape(0, 0, 5, 5)),
                None,
                Some(box_shape(7, 7, 8, 8)),
            ],
        );
        assert_eq!(entries.len(), 3);
        assert!(tree.validate_entries(&entries), "aligned by construction");
        // Swap slots 0 and 2 — the capture's E_POISONED form.
        let mut poisoned = entries.clone();
        poisoned.swap(0, 2);
        assert!(
            !tree.validate_entries(&poisoned),
            "E_POISONED — a swapped slot has the wrong shapeIndexInObject"
        );
        assert!(
            tree.validate_entries(&[]),
            "an empty list is vacuously true"
        );
    }
}
