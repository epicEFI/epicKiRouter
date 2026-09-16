//! Port of `app.freerouting.datastructures.PlanarDelaunayTriangulation`
//! (the Delaunay triangulation behind the ratsnest/incomplete-connection
//! counts; de Berg et al. 9.3 incremental insertion with a DAG search
//! structure).
//!
//! ## Home decision (M3-T2 re-scope)
//!
//! This lives in epic-geometry, not epic-drc: the algorithm is pure planar
//! geometry over [`Point`]s plus generic graph bookkeeping — its only
//! dependencies (`Side`, `Limits::CRIT_INT`,
//! [`FloatPoint::inside_circle`], [`JavaRandom`]) are all epic-geometry
//! residents already. The board/item semantics stay in epic-drc, which
//! feeds object-indexed corner lists and consumes the edge list.
//!
//! ## Determinism contract (the M3-T2 audit)
//!
//! The Java class is deterministic by construction, but only bit-exactly
//! reproducible through every one of these doors, and this port walks each:
//!
//! 1. **RNG**: a `static final Random(99)` re-seeded per construction, then
//!    `Collections.shuffle(cornerList, rng)` decides the insertion order
//!    ([`JavaRandom::shuffle`], bit-exact).
//! 2. **In-circle test**: `Edge.isLegal` calls
//!    `FloatPoint.insideCircle` on f64 coordinates — order-sensitive float
//!    arithmetic, reused verbatim from the M1a parity port.
//! 3. **Edge ids**: creation order (`newEdgeId`) orders the result
//!    TreeSet; here the arena index IS the id, pushed in creation order.
//! 4. **Cocircular degeneracy**: with four points on one circle the
//!    resulting edge set genuinely depends on the insertion order, so the
//!    pins below capture Java's edge set for FIXED input orders —
//!    including the degenerate square where a different order flips the
//!    diagonal.
//!
//! The caller owns the input order (Java's `NetIncompletes` passes its
//! `calculateNetItems` array; its group order is JVM-identity-hash
//! dependent, but the resulting Delaunay EDGE SET is order-invariant
//! except under (4), and the drc corpus pins the canonical edge set).
//!
//! ## Deviations
//!
//! - Java's debug-only `validate()` machinery is not ported.
//! - Java warn-and-continue sites (`FRLogger.warn`) that cannot trigger
//!   for the bounded inputs Freerouting feeds the class (corners strictly
//!   inside the ±`CRIT_INT` bounding triangle) become defensive
//!   `expect`s carrying the Java message — Java would NPE or produce
//!   garbage one step later, so both sides fail loudly.

use std::collections::BTreeSet;

use crate::int_point::IntPoint;
use crate::java_random::JavaRandom;
use crate::limits::CRIT_INT;
use crate::point::Point;
use crate::side::Side;

/// Java `PlanarDelaunayTriangulation.seed`: the fixed shuffle seed.
const SEED: i64 = 99;

type CornerId = usize;
/// The arena index doubles as Java's monotonically increasing edge id.
type EdgeId = usize;
type TriId = usize;

/// Java `ResultEdge`: one output segment between two input objects
/// (indices into the caller's object list).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultEdge {
    /// The object at the start point (Java `startObject`).
    pub start_object: usize,
    /// The start point of the line segment (Java `startPoint`).
    pub start_point: Point,
    /// The object at the end point (Java `endObject`).
    pub end_object: usize,
    /// The end point of the line segment (Java `endPoint`).
    pub end_point: Point,
}

/// Java `Corner`: a point together with the input object it belongs to
/// (the bounding corners carry no object). No `PartialEq` — Java Corner
/// equality is object identity, here corner-id equality.
#[derive(Debug, Clone)]
struct Corner {
    object: Option<usize>,
    coor: Point,
}

#[derive(Debug, Clone)]
struct EdgeData {
    start: CornerId,
    end: CornerId,
    /// The triangle on the left side of this edge (Java `leftTriangle`).
    left: Option<TriId>,
    /// The triangle on the right side of this edge (Java `rightTriangle`).
    right: Option<TriId>,
}

/// Java `Triangle`: three edge lines in counter-clockwise border order
/// plus the DAG search-graph bookkeeping.
#[derive(Debug, Clone)]
struct TriData {
    edge_lines: [EdgeId; 3],
    /// Triangles from an edge flip have two parents; traversal follows
    /// `first_parent` only, so nodes are visited once.
    first_parent: Option<TriId>,
    children: Vec<TriId>,
    /// Frozen at graph-insert time: was this triangle the LEFT triangle
    /// of its i-th edge line? Inner nodes need this because an edge's
    /// `left`/`right` always point at the current LEAF on each side.
    on_left: Option<[bool; 3]>,
}

/// The ported triangulation over caller-supplied corner lists, one per
/// input object (Java's `Storable.getTriangulationCorners()` arrays).
#[derive(Debug)]
pub struct PlanarDelaunayTriangulation {
    corners: Vec<Corner>,
    edges: Vec<EdgeData>,
    triangles: Vec<TriData>,
    /// Zero-length edges between coincident corners of DIFFERENT objects
    /// (Java `degenerateEdges`), in creation order.
    degenerate_edges: Vec<EdgeId>,
    /// Java `searchGraph.anchor`.
    anchor: Option<TriId>,
}

impl PlanarDelaunayTriangulation {
    /// Java constructor: collect the corners, shuffle them with the
    /// fixed seed, insert them into the bounding triangle.
    pub fn new(objects: &[Vec<Point>]) -> PlanarDelaunayTriangulation {
        let mut corner_list: Vec<Corner> = Vec::new();
        for (object, corners) in objects.iter().enumerate() {
            for coor in corners {
                corner_list.push(Corner {
                    object: Some(object),
                    coor: coor.clone(),
                });
            }
        }

        // create a random permutation of the corners.
        // use a fixed seed to get reproducible result
        JavaRandom::new(SEED).shuffle(&mut corner_list);

        let mut triangulation = PlanarDelaunayTriangulation {
            corners: Vec::new(),
            edges: Vec::new(),
            triangles: Vec::new(),
            degenerate_edges: Vec::new(),
            anchor: None,
        };

        // create a big triangle containing all corners in the list to
        // start with.
        let bounding_coor = CRIT_INT;
        let bounding_corners = [
            triangulation.new_corner(None, Point::Int(IntPoint::new(bounding_coor, 0))),
            triangulation.new_corner(None, Point::Int(IntPoint::new(0, bounding_coor))),
            triangulation.new_corner(
                None,
                Point::Int(IntPoint::new(-bounding_coor, -bounding_coor)),
            ),
        ];

        let edge_lines = [
            triangulation.new_edge(bounding_corners[0], bounding_corners[1]),
            triangulation.new_edge(bounding_corners[1], bounding_corners[2]),
            triangulation.new_edge(bounding_corners[2], bounding_corners[0]),
        ];

        let start_triangle = triangulation.new_triangle(edge_lines, None);

        // Set the left triangle of the edge lines to startTriangle.
        // The right triangles remains null.
        for &edge in &edge_lines {
            triangulation.edges[edge].left = Some(start_triangle);
        }

        // Initialize the search graph.
        triangulation.graph_insert(start_triangle, None);

        // Insert the corners in the corner list into the search graph
        // (they were appended to the arena in their shuffled order; the
        // insertion loop walks them by arena index).
        let first_new_corner = triangulation.corners.len();
        triangulation.corners.extend(corner_list);
        for corner_id in first_new_corner..triangulation.corners.len() {
            let triangle_to_split = triangulation
                .position_locate(corner_id)
                .expect("TriangleGraph.position_locate: containing triangle not found");
            triangulation.split(triangle_to_split, corner_id);
        }
        triangulation
    }

    fn new_corner(&mut self, object: Option<usize>, coor: Point) -> CornerId {
        self.corners.push(Corner { object, coor });
        self.corners.len() - 1
    }

    fn new_edge(&mut self, start: CornerId, end: CornerId) -> EdgeId {
        self.edges.push(EdgeData {
            start,
            end,
            left: None,
            right: None,
        });
        self.edges.len() - 1
    }

    fn new_triangle(&mut self, edge_lines: [EdgeId; 3], first_parent: Option<TriId>) -> TriId {
        self.triangles.push(TriData {
            edge_lines,
            first_parent,
            children: Vec::new(),
            on_left: None,
        });
        self.triangles.len() - 1
    }

    /// Java `TriangleGraph.insert` (plus
    /// `initializeIsOnTheLeftOfEdgeLineArray`): freeze the edge-side
    /// relationship at leaf time and hang the triangle under its parent.
    fn graph_insert(&mut self, triangle: TriId, parent: Option<TriId>) {
        let on_left = [
            self.edges[self.triangles[triangle].edge_lines[0]].left == Some(triangle),
            self.edges[self.triangles[triangle].edge_lines[1]].left == Some(triangle),
            self.edges[self.triangles[triangle].edge_lines[2]].left == Some(triangle),
        ];
        self.triangles[triangle].on_left = Some(on_left);
        match parent {
            None => self.anchor = Some(triangle),
            Some(parent) => self.triangles[parent].children.push(triangle),
        }
    }

    /// Java `TriangleGraph.positionLocate`: search for the leaf triangle
    /// containing the corner (not unique if the corner lies on an edge —
    /// the first found in child order wins, exactly as in Java).
    fn position_locate(&self, corner: CornerId) -> Option<TriId> {
        let anchor = self.anchor?;
        if self.triangles[anchor].children.is_empty() {
            return Some(anchor);
        }
        for child_index in 0..self.triangles[anchor].children.len() {
            let child = self.triangles[anchor].children[child_index];
            if let Some(found) = self.position_locate_reku(corner, child) {
                return Some(found);
            }
        }
        None
    }

    /// Recursive part of [`Self::position_locate`].
    fn position_locate_reku(&self, corner: CornerId, triangle: TriId) -> Option<TriId> {
        if !self.triangle_contains(triangle, corner) {
            return None;
        }
        if self.triangles[triangle].children.is_empty() {
            return Some(triangle);
        }
        for child_index in 0..self.triangles[triangle].children.len() {
            let child = self.triangles[triangle].children[child_index];
            if let Some(found) = self.position_locate_reku(corner, child) {
                return Some(found);
            }
        }
        None
    }

    /// Java `Triangle.contains`: inside or on the border, via the frozen
    /// `on_left` array.
    fn triangle_contains(&self, triangle: TriId, corner: CornerId) -> bool {
        let on_left = self.triangles[triangle]
            .on_left
            .expect("Triangle.contains: array isOnTheLeftOfEdgeLine not initialized");
        let coor = &self.corners[corner].coor;
        for (i, &edge_id) in self.triangles[triangle].edge_lines.iter().enumerate() {
            let edge = &self.edges[edge_id];
            let side = coor.side_of(&self.corners[edge.start].coor, &self.corners[edge.end].coor);
            if on_left[i] {
                if side == Side::Negative {
                    return false;
                }
            } else if side == Side::Positive {
                return false;
            }
        }
        true
    }

    /// Java `Triangle.getCorner`: the corner with the given index, via the
    /// edge's CURRENT side assignment (leaf triangles only).
    fn get_corner(&self, triangle: TriId, no: usize) -> CornerId {
        let edge = &self.edges[self.triangles[triangle].edge_lines[no]];
        if edge.left == Some(triangle) {
            edge.start
        } else if edge.right == Some(triangle) {
            edge.end
        } else {
            panic!("Triangle.get_corner: inconsistent edge lines")
        }
    }

    /// Java `Triangle.oppositeCorner`: the corner of this triangle opposite
    /// to the given edge line.
    fn opposite_corner(&self, triangle: TriId, edge_line: EdgeId) -> CornerId {
        let edge_lines = &self.triangles[triangle].edge_lines;
        let edge_line_no = edge_lines
            .iter()
            .position(|&e| e == edge_line)
            .expect("Triangle.opposite_corner: edgeLine not found");
        let next_edge = &self.edges[edge_lines[(edge_line_no + 1) % 3]];
        if next_edge.left == Some(triangle) {
            next_edge.end
        } else {
            next_edge.start
        }
    }

    /// Java `Edge.isLegal`: legal iff no opposite corner lies inside the
    /// circle through the edge's endpoints and the other opposite corner.
    /// The f64 arithmetic order is load-bearing (degenerate flips).
    fn edge_is_legal(&self, edge: EdgeId) -> bool {
        let (left, right) = match (self.edges[edge].left, self.edges[edge].right) {
            (Some(left), Some(right)) => (left, right),
            _ => return true,
        };
        let left_opposite = self.opposite_corner(left, edge);
        let right_opposite = self.opposite_corner(right, edge);
        let inside_circle = self.corners[right_opposite].coor.to_float().inside_circle(
            &self.corners[self.edges[edge].start].coor.to_float(),
            &self.corners[left_opposite].coor.to_float(),
            &self.corners[self.edges[edge].end].coor.to_float(),
        );
        !inside_circle
    }

    /// Java `Edge.flip`: flip the edge to the segment between the opposite
    /// corners of the adjacent triangles, returning the new edge id.
    fn flip_edge(&mut self, edge: EdgeId) -> EdgeId {
        let left_triangle = self.edges[edge]
            .left
            .expect("Edge.flip: edge line inconsistent");
        let right_triangle = self.edges[edge]
            .right
            .expect("Edge.flip: edge line inconsistent");
        let first_parent = self.edges[edge].left;

        // Create the flipped edge, so that the start corner of this edge
        // is on the left and the end corner of this edge on the right.
        let new_start = self.opposite_corner(right_triangle, edge);
        let new_end = self.opposite_corner(left_triangle, edge);
        let flipped_edge = self.new_edge(new_start, new_end);

        // Calculate the index of this edge line in the left and right
        // adjacent triangles.
        let left_index = self.triangles[left_triangle]
            .edge_lines
            .iter()
            .position(|&e| e == edge)
            .expect("Edge.flip: edge line inconsistent");
        let right_index = self.triangles[right_triangle]
            .edge_lines
            .iter()
            .position(|&e| e == edge)
            .expect("Edge.flip: edge line inconsistent");

        let left_prev_edge = self.triangles[left_triangle].edge_lines[(left_index + 2) % 3];
        let left_next_edge = self.triangles[left_triangle].edge_lines[(left_index + 1) % 3];
        let right_prev_edge = self.triangles[right_triangle].edge_lines[(right_index + 2) % 3];
        let right_next_edge = self.triangles[right_triangle].edge_lines[(right_index + 1) % 3];

        // Create the left triangle of the flipped edge.
        let new_left_triangle = self.new_triangle(
            [flipped_edge, left_prev_edge, right_next_edge],
            first_parent,
        );
        self.edges[flipped_edge].left = Some(new_left_triangle);
        if self.edges[left_prev_edge].left == Some(left_triangle) {
            self.edges[left_prev_edge].left = Some(new_left_triangle);
        } else {
            self.edges[left_prev_edge].right = Some(new_left_triangle);
        }
        if self.edges[right_next_edge].left == Some(right_triangle) {
            self.edges[right_next_edge].left = Some(new_left_triangle);
        } else {
            self.edges[right_next_edge].right = Some(new_left_triangle);
        }

        // Create the right triangle of the flipped edge.
        let new_right_triangle = self.new_triangle(
            [flipped_edge, right_prev_edge, left_next_edge],
            first_parent,
        );
        self.edges[flipped_edge].right = Some(new_right_triangle);
        if self.edges[right_prev_edge].left == Some(right_triangle) {
            self.edges[right_prev_edge].left = Some(new_right_triangle);
        } else {
            self.edges[right_prev_edge].right = Some(new_right_triangle);
        }
        if self.edges[left_next_edge].left == Some(left_triangle) {
            self.edges[left_next_edge].left = Some(new_right_triangle);
        } else {
            self.edges[left_next_edge].right = Some(new_right_triangle);
        }

        flipped_edge
    }

    /// Java `split`: split a triangle at the given corner — inner point →
    /// three triangles, edge point → two plus two in the neighbour,
    /// coincident corner of a DIFFERENT object → a degenerate edge.
    /// Returns false exactly where Java's version returns false.
    fn split(&mut self, triangle: TriId, corner: CornerId) -> bool {
        // check, if corner is in the interior of this triangle or
        // if corner is contained in an edge line.
        let mut containing_edge: Option<EdgeId> = None;
        for i in 0..3 {
            let current_edge = self.triangles[triangle].edge_lines[i];
            let current_side = if self.edges[current_edge].left == Some(triangle) {
                self.corners[corner].coor.side_of(
                    &self.corners[self.edges[current_edge].start].coor,
                    &self.corners[self.edges[current_edge].end].coor,
                )
            } else {
                self.corners[corner].coor.side_of(
                    &self.corners[self.edges[current_edge].end].coor,
                    &self.corners[self.edges[current_edge].start].coor,
                )
            };
            if current_side == Side::Negative {
                // Java: FRLogger.warn("...corner is outside") and return
                // false (reachable only for unbounded inputs).
                return false;
            } else if current_side == Side::Collinear {
                if let Some(previous_edge) = containing_edge {
                    // corner is equal to a corner of this triangle
                    let common_corner = match self.common_corner(current_edge, previous_edge) {
                        Some(common_corner) => common_corner,
                        None => {
                            // Java: FRLogger.warn("...common corner expected").
                            return false;
                        }
                    };
                    if self.corners[corner].object == self.corners[common_corner].object {
                        return false;
                    }
                    let degenerate = self.new_edge(corner, common_corner);
                    self.degenerate_edges.push(degenerate);
                    return true;
                }
                containing_edge = Some(current_edge);
            }
        }

        match containing_edge {
            None => {
                // split triangle into 3 new triangles by adding edges from
                // the corners of triangle to corner.
                let new_triangles = match self.split_at_inner_point(triangle, corner) {
                    Some(new_triangles) => new_triangles,
                    None => return false,
                };
                for &current_triangle in &new_triangles {
                    self.graph_insert(current_triangle, Some(triangle));
                }
                let original_edges = self.triangles[triangle].edge_lines;
                for edge in original_edges {
                    self.legalize_edge(corner, edge);
                }
            }
            Some(containing_edge) => {
                // split this triangle and the neighbour triangle into 4
                // new triangles by adding edges from the corners of the
                // triangles to corner.
                let neighbour_to_split = self.other_neighbour(containing_edge, triangle);
                let new_triangles =
                    match self.split_at_border_point(triangle, corner, neighbour_to_split) {
                        Some(new_triangles) => new_triangles,
                        None => return false,
                    };
                // There are exact four new triangles with the first 2
                // dividing triangle and the last 2 dividing
                // neighbourToSplit.
                self.graph_insert(new_triangles[0], Some(triangle));
                self.graph_insert(new_triangles[1], Some(triangle));
                self.graph_insert(new_triangles[2], Some(neighbour_to_split));
                self.graph_insert(new_triangles[3], Some(neighbour_to_split));

                let this_edges = self.triangles[triangle].edge_lines;
                for edge in this_edges {
                    if edge != containing_edge {
                        self.legalize_edge(corner, edge);
                    }
                }
                let neighbour_edges = self.triangles[neighbour_to_split].edge_lines;
                for edge in neighbour_edges {
                    if edge != containing_edge {
                        self.legalize_edge(corner, edge);
                    }
                }
            }
        }
        true
    }

    /// Java `Edge.commonCorner`: the shared corner of two edges of the
    /// same triangle, by corner identity.
    fn common_corner(&self, edge: EdgeId, other: EdgeId) -> Option<CornerId> {
        let (edge_start, edge_end) = (self.edges[edge].start, self.edges[edge].end);
        let (other_start, other_end) = (self.edges[other].start, self.edges[other].end);
        if other_start == edge_start || other_end == edge_start {
            Some(edge_start)
        } else if other_start == edge_end || other_end == edge_end {
            Some(edge_end)
        } else {
            None
        }
    }

    /// Java `Edge.otherNeighbour`: the neighbour triangle on the other
    /// side of the edge.
    fn other_neighbour(&self, edge: EdgeId, triangle: TriId) -> TriId {
        if self.edges[edge].left == Some(triangle) {
            self.edges[edge]
                .right
                .expect("Edge.other_neighbour: inconsistent neighbour triangle")
        } else if self.edges[edge].right == Some(triangle) {
            self.edges[edge]
                .left
                .expect("Edge.other_neighbour: inconsistent neighbour triangle")
        } else {
            panic!("Edge.other_neighbour: inconsistent neighbour triangle")
        }
    }

    /// Java `legalizeEdge`: flip the edge if illegal, then recurse on the
    /// remaining edge lines of the changed triangle.
    fn legalize_edge(&mut self, corner: CornerId, edge: EdgeId) -> bool {
        if self.edge_is_legal(edge) {
            return false;
        }
        let triangle_to_change = {
            let left = self.edges[edge]
                .left
                .expect("edge lines inconsistent (legalize)");
            let right = self.edges[edge]
                .right
                .expect("edge lines inconsistent (legalize)");
            if self.opposite_corner(left, edge) == corner {
                right
            } else if self.opposite_corner(right, edge) == corner {
                left
            } else {
                // Java: FRLogger.warn("...edge lines inconsistent").
                panic!("PlanarDelaunayTriangulation.legalize_edge: edge lines inconsistent")
            }
        };
        let flipped_edge = self.flip_edge(edge);

        // Update the search graph (the flipped triangles hang under BOTH
        // former adjacent triangles, in Java's insertion order).
        let flipped_left = self.edges[flipped_edge].left;
        let flipped_right = self.edges[flipped_edge].right;
        let old_left = self.edges[edge].left;
        let old_right = self.edges[edge].right;
        for (child, parent) in [
            (flipped_left, old_left),
            (flipped_right, old_left),
            (flipped_left, old_right),
            (flipped_right, old_right),
        ] {
            if let (Some(child), Some(parent)) = (child, parent) {
                self.graph_insert(child, Some(parent));
            }
        }

        // Call this function recursively for the other edge lines of
        // triangleToChange.
        let triangle_edges = self.triangles[triangle_to_change].edge_lines;
        for candidate in triangle_edges {
            if candidate != edge {
                self.legalize_edge(corner, candidate);
            }
        }
        true
    }

    /// Java `Triangle.splitAtInnerPoint`.
    fn split_at_inner_point(&mut self, triangle: TriId, corner: CornerId) -> Option<[TriId; 3]> {
        // Java creates three helper edges here that are immediately
        // discarded (they only consume edge ids); kept for exact
        // id-sequence parity.
        for i in 0..3 {
            let triangle_corner = self.get_corner(triangle, i);
            let _dead_edge = self.new_edge(triangle_corner, corner);
        }

        let original_edges = self.triangles[triangle].edge_lines;
        let corner_0 = self.get_corner(triangle, 0);
        let corner_1 = self.get_corner(triangle, 1);
        let corner_2 = self.get_corner(triangle, 2);

        // construct the 3 new triangles.
        let first_splitting = self.new_edge(corner_1, corner);
        let second_splitting = self.new_edge(corner, corner_0);
        let new_triangle_0 = self.new_triangle(
            [original_edges[0], first_splitting, second_splitting],
            Some(triangle),
        );

        let third_splitting = self.new_edge(corner_2, corner);
        // Java reuses newTriangles[0].edgeLines[1] (= first_splitting).
        let new_triangle_1 = self.new_triangle(
            [original_edges[1], third_splitting, first_splitting],
            Some(triangle),
        );

        // Java: [edgeLines[2], newTriangles[0].edgeLines[2],
        //        newTriangles[1].edgeLines[1]]
        let new_triangle_2 = self.new_triangle(
            [original_edges[2], second_splitting, third_splitting],
            Some(triangle),
        );
        let new_triangles = [new_triangle_0, new_triangle_1, new_triangle_2];

        // Set the new neighbour triangles of the edge lines.
        for &current_triangle in &new_triangles {
            let current_edge = self.triangles[current_triangle].edge_lines[0];
            if self.edges[current_edge].left == Some(triangle) {
                self.edges[current_edge].left = Some(current_triangle);
            } else {
                self.edges[current_edge].right = Some(current_triangle);
            }
            // The other neighbour triangle remains valid.
        }

        let current_edge = self.triangles[new_triangle_0].edge_lines[1];
        self.edges[current_edge].left = Some(new_triangle_0);
        self.edges[current_edge].right = Some(new_triangle_1);

        let current_edge = self.triangles[new_triangle_1].edge_lines[1];
        self.edges[current_edge].left = Some(new_triangle_1);
        self.edges[current_edge].right = Some(new_triangle_2);

        let current_edge = self.triangles[new_triangle_2].edge_lines[1];
        self.edges[current_edge].left = Some(new_triangle_0);
        self.edges[current_edge].right = Some(new_triangle_2);
        Some(new_triangles)
    }

    /// Java `Triangle.splitAtBorderPoint`.
    fn split_at_border_point(
        &mut self,
        triangle: TriId,
        corner: CornerId,
        neighbour_to_split: TriId,
    ) -> Option<[TriId; 4]> {
        // look for the triangle edge of this and the neighbour triangle
        // containing corner; the LAST collinear edge wins (Java's loop has
        // no break).
        let mut this_touching_edge_no: Option<usize> = None;
        let mut neighbour_touching_edge_no: Option<usize> = None;
        for i in 0..3 {
            let current_edge = self.triangles[triangle].edge_lines[i];
            if self.corners[corner].coor.side_of(
                &self.corners[self.edges[current_edge].start].coor,
                &self.corners[self.edges[current_edge].end].coor,
            ) == Side::Collinear
            {
                this_touching_edge_no = Some(i);
            }
            let current_edge = self.triangles[neighbour_to_split].edge_lines[i];
            if self.corners[corner].coor.side_of(
                &self.corners[self.edges[current_edge].start].coor,
                &self.corners[self.edges[current_edge].end].coor,
            ) == Side::Collinear
            {
                neighbour_touching_edge_no = Some(i);
            }
        }
        let (this_touching_edge_no, neighbour_touching_edge_no) =
            match (this_touching_edge_no, neighbour_touching_edge_no) {
                (Some(this_no), Some(neighbour_no)) => (this_no, neighbour_no),
                _ => {
                    // Java: FRLogger.warn("...touching edge not found").
                    return None;
                }
            };
        let touching_edge = self.triangles[triangle].edge_lines[this_touching_edge_no];
        let other_touching_edge =
            self.triangles[neighbour_to_split].edge_lines[neighbour_touching_edge_no];
        if touching_edge != other_touching_edge {
            // Java: FRLogger.warn("...edges inconsistent").
            return None;
        }

        // Construct the new edge lines that the 2 split triangles of this
        // triangle will be on the left side of the new common touching
        // edges.
        let (first_common_new_edge, second_common_new_edge) =
            if self.edges[touching_edge].left == Some(triangle) {
                (
                    self.new_edge(self.edges[touching_edge].start, corner),
                    self.new_edge(corner, self.edges[touching_edge].end),
                )
            } else {
                (
                    self.new_edge(self.edges[touching_edge].end, corner),
                    self.new_edge(corner, self.edges[touching_edge].start),
                )
            };

        // Construct the first split triangle of this triangle.
        let prev_edge = self.triangles[triangle].edge_lines[(this_touching_edge_no + 2) % 3];
        // construct the splitting edge line of this triangle, so that the
        // first split triangle lies on the left side, and the second split
        // triangle on the right side.
        let this_splitting_edge = if self.edges[prev_edge].left == Some(triangle) {
            self.new_edge(corner, self.edges[prev_edge].start)
        } else {
            self.new_edge(corner, self.edges[prev_edge].end)
        };
        let new_triangle_0 = self.new_triangle(
            [prev_edge, first_common_new_edge, this_splitting_edge],
            Some(triangle),
        );
        if self.edges[prev_edge].left == Some(triangle) {
            self.edges[prev_edge].left = Some(new_triangle_0);
        } else {
            self.edges[prev_edge].right = Some(new_triangle_0);
        }
        self.edges[first_common_new_edge].left = Some(new_triangle_0);
        self.edges[this_splitting_edge].left = Some(new_triangle_0);

        // Construct the second split triangle of this triangle.
        let next_edge = self.triangles[triangle].edge_lines[(this_touching_edge_no + 1) % 3];
        let new_triangle_1 = self.new_triangle(
            [this_splitting_edge, second_common_new_edge, next_edge],
            Some(triangle),
        );
        self.edges[this_splitting_edge].right = Some(new_triangle_1);
        self.edges[second_common_new_edge].left = Some(new_triangle_1);
        if self.edges[next_edge].left == Some(triangle) {
            self.edges[next_edge].left = Some(new_triangle_1);
        } else {
            self.edges[next_edge].right = Some(new_triangle_1);
        }

        // construct the first split triangle of neighbourToSplit
        let neighbour_next_edge =
            self.triangles[neighbour_to_split].edge_lines[(neighbour_touching_edge_no + 1) % 3];
        // construct the splitting edge line of neighbourToSplit, so that
        // the first split triangle lies on the left side, and the second
        // split triangle on the right side.
        let neighbour_splitting_edge =
            if self.edges[neighbour_next_edge].left == Some(neighbour_to_split) {
                self.new_edge(self.edges[neighbour_next_edge].end, corner)
            } else {
                self.new_edge(self.edges[neighbour_next_edge].start, corner)
            };
        let new_triangle_2 = self.new_triangle(
            [
                neighbour_splitting_edge,
                first_common_new_edge,
                neighbour_next_edge,
            ],
            Some(neighbour_to_split),
        );
        self.edges[neighbour_splitting_edge].left = Some(new_triangle_2);
        self.edges[first_common_new_edge].right = Some(new_triangle_2);
        if self.edges[neighbour_next_edge].left == Some(neighbour_to_split) {
            self.edges[neighbour_next_edge].left = Some(new_triangle_2);
        } else {
            self.edges[neighbour_next_edge].right = Some(new_triangle_2);
        }

        // construct the second split triangle of neighbourToSplit
        let prev_edge =
            self.triangles[neighbour_to_split].edge_lines[(neighbour_touching_edge_no + 2) % 3];
        let new_triangle_3 = self.new_triangle(
            [prev_edge, second_common_new_edge, neighbour_splitting_edge],
            Some(neighbour_to_split),
        );
        if self.edges[prev_edge].left == Some(neighbour_to_split) {
            self.edges[prev_edge].left = Some(new_triangle_3);
        } else {
            self.edges[prev_edge].right = Some(new_triangle_3);
        }
        self.edges[second_common_new_edge].right = Some(new_triangle_3);
        self.edges[neighbour_splitting_edge].right = Some(new_triangle_3);

        Some([
            new_triangle_0,
            new_triangle_1,
            new_triangle_2,
            new_triangle_3,
        ])
    }

    /// Java `getEdgeLines`: the degenerate edges (creation order), then
    /// the leaf edge set sorted by edge id, restricted to edges whose
    /// endpoints both belong to input objects.
    pub fn edge_lines(&self) -> Vec<ResultEdge> {
        let mut result = Vec::new();
        for &edge in &self.degenerate_edges {
            result.push(self.result_edge(edge));
        }
        if let Some(anchor) = self.anchor {
            let mut result_edges = BTreeSet::new();
            self.get_leaf_edges(anchor, &mut result_edges);
            for edge in result_edges {
                result.push(self.result_edge(edge));
            }
        }
        result
    }

    fn result_edge(&self, edge: EdgeId) -> ResultEdge {
        ResultEdge {
            start_object: self.corners[self.edges[edge].start]
                .object
                .expect("result edges skip bounding corners"),
            start_point: self.corners[self.edges[edge].start].coor.clone(),
            end_object: self.corners[self.edges[edge].end]
                .object
                .expect("result edges skip bounding corners"),
            end_point: self.corners[self.edges[edge].end].coor.clone(),
        }
    }

    /// Java `Triangle.getLeafEdges`: the edges of all leaves below this
    /// node, deduplicated by edge id.
    fn get_leaf_edges(&self, triangle: TriId, result_edges: &mut BTreeSet<EdgeId>) {
        if self.triangles[triangle].children.is_empty() {
            for &edge in &self.triangles[triangle].edge_lines {
                if self.corners[self.edges[edge].start].object.is_some()
                    && self.corners[self.edges[edge].end].object.is_some()
                {
                    // Skip edges containing a bounding corner.
                    result_edges.insert(edge);
                }
            }
        } else {
            for child_index in 0..self.triangles[triangle].children.len() {
                let child = self.triangles[triangle].children[child_index];
                if self.triangles[child].first_parent == Some(triangle) {
                    // to prevent traversing nodes more than once
                    self.get_leaf_edges(child, result_edges);
                }
            }
        }
    }
}

/// Convenience wrapper matching the NetIncompletes call shape: triangulate
/// the given per-object corner lists and return the edge lines.
pub fn triangulate(objects: &[Vec<Point>]) -> Vec<ResultEdge> {
    PlanarDelaunayTriangulation::new(objects).edge_lines()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::float_point::FloatPoint;
    use crate::side::Side;

    type Pt = (i32, i32);

    fn pts(specs: &[&[Pt]]) -> Vec<Vec<Point>> {
        specs
            .iter()
            .map(|corners| {
                corners
                    .iter()
                    .map(|(x, y)| Point::Int(IntPoint::new(*x, *y)))
                    .collect()
            })
            .collect()
    }

    /// The ResultEdge list as comparable tuples IN getEdgeLines() order
    /// (degenerate edges first, then by edge id) — the literal shape the
    /// probe prints.
    fn edge_tuples(objects: &[Vec<Point>]) -> Vec<(usize, Pt, usize, Pt)> {
        triangulate(objects)
            .into_iter()
            .map(|edge| {
                let start = match edge.start_point {
                    Point::Int(p) => (p.x, p.y),
                    _ => panic!("probe corners are integral"),
                };
                let end = match edge.end_point {
                    Point::Int(p) => (p.x, p.y),
                    _ => panic!("probe corners are integral"),
                };
                (edge.start_object, start, edge.end_object, end)
            })
            .collect()
    }

    /// JavaRandom(99) shuffle pinned against `Collections.shuffle` with
    /// `new Random(99)` (JDK 25, DelaunayProbe SHUFFLE lines): the
    /// permutation is order-significant, so a swapped loop direction or
    /// off-by-one bound diverges.
    #[test]
    fn shuffle_matches_jdk_25_permutations() {
        let mut four: Vec<usize> = (0..4).collect();
        JavaRandom::new(99).shuffle(&mut four);
        assert_eq!(four, vec![1, 0, 3, 2]);

        let mut ten: Vec<usize> = (0..10).collect();
        JavaRandom::new(99).shuffle(&mut ten);
        assert_eq!(ten, vec![8, 1, 4, 3, 9, 6, 0, 2, 5, 7]);
    }

    /// Captured with DelaunayProbe against the frozen jar (JDK 25). The
    /// four COCIRCULAR square cases are the determinism heart of this
    /// port: the in-circle test rides exactly on the circle, so the edge
    /// set genuinely depends on the shuffled insertion order — order A
    /// drops boundary edge (2,3) outright, B and C produce different
    /// shapes. An order-insensitive mutant (skipped shuffle, wrong flip
    /// condition) cannot satisfy all three.
    #[test]
    fn cocircular_square_edge_set_matches_java_per_input_order() {
        // CASE square_order_a: 4 edges, diagonal (1,3), boundary (2,3) gone.
        let a = edge_tuples(&pts(&[&[(0, 0)], &[(10, 0)], &[(10, 10)], &[(0, 10)]]));
        assert_eq!(
            a,
            vec![
                (1, (10, 0), 0, (0, 0)),
                (0, (0, 0), 3, (0, 10)),
                (1, (10, 0), 3, (0, 10)),
                (1, (10, 0), 2, (10, 10)),
            ],
            "square order A"
        );

        // CASE square_order_b: full 5-edge triangulation, opposite
        // diagonal (0,1).
        let b = edge_tuples(&pts(&[&[(10, 10)], &[(0, 0)], &[(0, 10)], &[(10, 0)]]));
        assert_eq!(
            b,
            vec![
                (1, (0, 0), 0, (10, 10)),
                (1, (0, 0), 3, (10, 0)),
                (3, (10, 0), 0, (10, 10)),
                (2, (0, 10), 1, (0, 0)),
                (2, (0, 10), 0, (10, 10)),
            ],
            "square order B"
        );

        // CASE square_order_c: yet another 4-edge shape, diagonal (1,0),
        // boundary (0,3) gone.
        let c = edge_tuples(&pts(&[&[(0, 10)], &[(10, 0)], &[(0, 0)], &[(10, 10)]]));
        assert_eq!(
            c,
            vec![
                (1, (10, 0), 0, (0, 10)),
                (1, (10, 0), 3, (10, 10)),
                (1, (10, 0), 2, (0, 0)),
                (0, (0, 10), 2, (0, 0)),
            ],
            "square order C"
        );
    }

    /// CASE generic_4: a non-degenerate point set — the edge set is the
    /// unique Delaunay graph (sanity that ordinary insertion works).
    #[test]
    fn generic_four_matches_java() {
        let edges = edge_tuples(&pts(&[&[(0, 0)], &[(100, 0)], &[(30, 80)], &[(0, 10)]]));
        assert_eq!(
            edges,
            vec![
                (1, (100, 0), 0, (0, 0)),
                (0, (0, 0), 3, (0, 10)),
                (1, (100, 0), 3, (0, 10)),
                (2, (30, 80), 1, (100, 0)),
                (3, (0, 10), 2, (30, 80)),
            ]
        );
    }

    /// CASE coincident: two objects sharing coordinates produce the
    /// ZERO-LENGTH degenerate edge, emitted FIRST (Java's getEdgeLines
    /// leads with degenerateEdges) — the ratsnest airline between
    /// stacked items.
    #[test]
    fn coincident_corners_produce_leading_degenerate_edge() {
        let edges = edge_tuples(&pts(&[
            &[(5, 5)],
            &[(5, 5)],
            &[(20, 0)],
            &[(0, 0), (40, 0)],
        ]));
        assert_eq!(
            edges.first(),
            Some(&(1, (5, 5), 0, (5, 5))),
            "the degenerate edge leads"
        );
        assert_eq!(
            edges,
            vec![
                (1, (5, 5), 0, (5, 5)),
                (0, (5, 5), 3, (40, 0)),
                (3, (0, 0), 0, (5, 5)),
                (3, (40, 0), 2, (20, 0)),
                (2, (20, 0), 3, (0, 0)),
                (0, (5, 5), 2, (20, 0)),
            ]
        );
    }

    /// CASE collinear_plus_interior: the border-split path (third corner
    /// on the segment between the first two); the frozen algorithm drops
    /// boundary edge (0,3) here — pinned as-is.
    #[test]
    fn collinear_plus_interior_matches_java() {
        let edges = edge_tuples(&pts(&[&[(0, 0)], &[(10, 0)], &[(20, 0)], &[(10, 10)]]));
        assert_eq!(
            edges,
            vec![
                (1, (10, 0), 0, (0, 0)),
                (1, (10, 0), 3, (10, 10)),
                (1, (10, 0), 2, (20, 0)),
                (2, (20, 0), 3, (10, 10)),
            ]
        );
    }

    /// CASE stubby_objects: multi-corner objects (trace stubs) can be
    /// joined corner-to-corner — including BOTH corners of the SAME
    /// object (the (0,0) edge pair) — and a zero-corner object
    /// contributes nothing (the falsified-claim mechanism's geometric
    /// side).
    #[test]
    fn stubby_objects_same_object_edge_matches_java() {
        let edges = edge_tuples(&pts(&[
            &[(0, 0), (40, 0)],
            &[(10, 30)],
            &[(30, 25)],
            &[],
            &[(20, 5)],
        ]));
        assert_eq!(
            edges,
            vec![
                (4, (20, 5), 2, (30, 25)),
                (4, (20, 5), 0, (0, 0)),
                (4, (20, 5), 0, (40, 0)),
                (0, (40, 0), 2, (30, 25)),
                (0, (40, 0), 0, (0, 0)),
                (2, (30, 25), 1, (10, 30)),
                (1, (10, 30), 0, (0, 0)),
                (4, (20, 5), 1, (10, 30)),
            ]
        );
    }

    /// CASE generic_6: deeper flip cascades over six points.
    #[test]
    fn generic_six_matches_java() {
        let edges = edge_tuples(&pts(&[
            &[(0, 0)],
            &[(50, 5)],
            &[(90, 30)],
            &[(70, 80)],
            &[(20, 70)],
            &[(45, 40)],
        ]));
        assert_eq!(
            edges,
            vec![
                (4, (20, 70), 0, (0, 0)),
                (4, (20, 70), 5, (45, 40)),
                (5, (45, 40), 2, (90, 30)),
                (0, (0, 0), 5, (45, 40)),
                (3, (70, 80), 2, (90, 30)),
                (4, (20, 70), 3, (70, 80)),
                (3, (70, 80), 5, (45, 40)),
                (1, (50, 5), 0, (0, 0)),
                (2, (90, 30), 1, (50, 5)),
                (5, (45, 40), 1, (50, 5)),
            ]
        );
    }

    /// Empty input and no-corner input triangulate to no edges (the
    /// bounding corners are objectless and skipped).
    #[test]
    fn empty_inputs_yield_no_edges() {
        assert!(triangulate(&[]).is_empty());
        assert!(triangulate(&[Vec::new()]).is_empty());
    }

    /// The in-circle contract the legality gate rides on
    /// (FloatPoint.java:461-466): inside the circumcircle through p1..p3
    /// minus the `- 1` radius-square tolerance. ALSO pins the frozen
    /// NaN door: `circleCenter` divides by edge slopes, so a triangle
    /// with a VERTICAL edge yields a NaN center and `inside_circle` is
    /// false for every point — this is why the axis-aligned cocircular
    /// square captures above never flip their diagonals. A mutant that
    /// "fixed" the NaN (or dropped the tolerance) breaks the square
    /// pins while passing ordinary cases.
    #[test]
    fn incircle_tolerance_and_nan_quirk() {
        // Circumcircle of (0,0),(8,2),(4,10): center (28/9, 41/9),
        // radius-square 2465/81 ≈ 30.43. Centroid (4,4) is strictly
        // inside (dist² ≈ 1.1 < 29.43); corner (4,10) sits exactly on
        // the circle (30.43 > 29.43 → outside — the tolerance door).
        let a = FloatPoint::new(0.0, 0.0);
        let b = FloatPoint::new(8.0, 2.0);
        let c = FloatPoint::new(4.0, 10.0);
        assert!(FloatPoint::new(4.0, 4.0).inside_circle(&a, &b, &c));
        assert!(!c.inside_circle(&a, &b, &c));
        // The vertical-edge NaN door: false even for a point clearly
        // inside the geometric circumcircle (center (5,5), radius² 50).
        let va = FloatPoint::new(0.0, 0.0);
        let vb = FloatPoint::new(10.0, 0.0);
        let vc = FloatPoint::new(10.0, 10.0);
        assert!(!FloatPoint::new(1.0, 9.0).inside_circle(&va, &vb, &vc));
        let _ = Side::Positive;
    }
}
