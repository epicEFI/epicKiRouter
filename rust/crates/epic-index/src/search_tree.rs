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

use std::sync::atomic::{AtomicU64, Ordering};

use epic_geometry::shape::ShapeBoundingDirections;
use epic_geometry::tile_shape::TileShape;

use crate::MinAreaTree;
use crate::shape_tree::NodeIdx;

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

/// A configured search tree (Java `ShapeSearchTree` and subclasses).
#[derive(Clone, Debug)]
pub struct SearchTree {
    /// The min-area BVH (T5).
    tree: MinAreaTree,
    /// The Java subclass this tree mirrors.
    pub variant: SearchTreeVariant,
    /// Java `compensatedClearanceClassNo` (`ShapeSearchTree.java:70`)
    /// — a plain class NUMBER here; the rules semantics (the
    /// compensation formula, `ShapeSearchTree.java:104-114`) live in
    /// epic-board.
    pub compensated_clearance_class: i32,
    /// Process-unique identity (Java object reference).
    object_id: u64,
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
        if shapes.is_empty() {
            return Vec::new();
        }
        let directions = self.variant.bounding_directions();
        let mut leaves = Vec::with_capacity(shapes.len());
        for (index, shape) in shapes.iter().enumerate() {
            let bounds = shape
                .as_ref()
                .and_then(|shape| shape.bounding_shape(&directions));
            leaves.push(bounds.map(|bounds| {
                let leaf = self.tree.base.push_leaf(object_key, index as u32, bounds);
                self.tree.insert_leaf(leaf);
                leaf
            }));
        }
        leaves
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

    /// The underlying BVH (read access for query surfaces, Task 8).
    #[must_use]
    pub fn min_area_tree(&self) -> &MinAreaTree {
        &self.tree
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
        // A clone shares the id (it is the same logical tree snapshot).
        assert_eq!(a.clone().object_id(), a.object_id());
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
}
