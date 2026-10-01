//! Java `autoroute/maze/DestinationDistance.java` (392 lines) — the
//! admissible lower-bound distance from a front element to the joined
//! destination set: the heuristic that turns the maze search into an
//! A*-shaped search (`sortingValue = expansionValue + calculate(..)`,
//! Java `MazeSearchEngine.java:884`).
//!
//! Construction and wiring:
//! * the engine constructs it in its ctor
//!   (`MazeSearchEngine.java:126-133`) — the Rust production
//!   construction site is [`DestinationDistance::from_ctrl`];
//! * `init` joins every destination tree-shape bounding box at its
//!   layer (`MazeSearchEngine.java:983`) plus the board bounding box
//!   on both outer layers for fanout (`:991-992`);
//! * [`drill::DestinationDistance::calculate`] is called per door
//!   section (`:884`) and per start-room seed (`:1065`).
//!
//! The trait seam lives in [`crate::drill`] (the T5 home); this module
//! owns the production implementation. The IntBox overload
//! ([`DestinationDistance::calculate_box`]) and
//! [`DestinationDistance::calculate_cheap_distance`] are inherent
//! methods — the engine only ever calls the FloatPoint path.

use epic_geometry::float_point::FloatPoint;
use epic_geometry::int_box::IntBox;

use crate::control::{AutorouteControl, ExpansionCostFactor};
use crate::drill::DestinationDistance as DestinationDistanceTrait;

/// Java `Integer.MAX_VALUE` widened to double (`:124`, and the initial
/// `result` at `:217`): the "nothing joined yet" sentinel — the same
/// 2147483647.0 family the T5 targetless arm pinned.
const INT_MAX_AS_DOUBLE: f64 = 2147483647.0;

/// Java `DestinationDistance` — the destination-bucket lower bound.
/// The derived cost fields are private by Java's letter (most are
/// package-visible there, but no external consumer reads them); the
/// in-module tests inspect them directly.
pub struct DestinationDistance {
    trace_costs: Vec<ExpansionCostFactor>,
    layer_active: Vec<bool>,
    layer_count: usize,
    active_layer_count: usize,
    min_normal_via_cost: f64,
    min_cheap_via_cost: f64,
    min_component_side_trace_cost: f64,
    max_component_side_trace_cost: f64,
    min_solder_side_trace_cost: f64,
    max_solder_side_trace_cost: f64,
    /// minimum of the maximal trace costs on each inner layer
    max_inner_side_trace_cost: f64,
    /// minimum of minComponentSideTraceCost and maxInnerSideTraceCost
    min_component_inner_trace_cost: f64,
    /// minimum of minSolderSideTraceCost and maxInnerSideTraceCost
    min_solder_inner_trace_cost: f64,
    /// minimum of minComponentInnerTraceCost and minSolderInnerTraceCost
    min_component_solder_inner_trace_cost: f64,
    component_side_box: IntBox,
    solder_side_box: IntBox,
    inner_side_box: IntBox,
    box_is_empty: bool,
    component_side_box_is_empty: bool,
    solder_side_box_is_empty: bool,
    inner_side_box_is_empty: bool,
}

impl DestinationDistance {
    /// Java ctor (`:45-99`). `trace_costs` and `layer_active` are of
    /// dimension `layer_active.len()`.
    ///
    /// # The 0.0-quirk (LOAD-BEARING, do not "fix")
    ///
    /// `min/max_component_side_trace_cost` are assigned ONLY
    /// `if (layerActive[0])` (`:63-71`) and the solder pair ONLY
    /// `if (layerActive[layerCount-1])` (`:73-83`). An inactive outer
    /// layer leaves its fields at the Java numeric-field default 0.0,
    /// which then propagates into `maxInnerSideTraceCost =
    /// min(maxComponent, maxSolder)` (`:86` — the 0.0 min!) and onward
    /// into the three inner mins (`:95-98`). A port that skips
    /// inactive layers diverges on every partial-active board.
    pub fn new(
        trace_costs: &[ExpansionCostFactor],
        layer_active: &[bool],
        min_normal_via_cost: f64,
        min_cheap_via_cost: f64,
    ) -> Self {
        let layer_count = layer_active.len();
        let active_layer_count = layer_active.iter().filter(|&&active| active).count();
        let mut this = DestinationDistance {
            trace_costs: trace_costs.to_vec(),
            layer_active: layer_active.to_vec(),
            layer_count,
            active_layer_count,
            min_normal_via_cost,
            min_cheap_via_cost,
            min_component_side_trace_cost: 0.0,
            max_component_side_trace_cost: 0.0,
            min_solder_side_trace_cost: 0.0,
            max_solder_side_trace_cost: 0.0,
            max_inner_side_trace_cost: 0.0,
            min_component_inner_trace_cost: 0.0,
            min_solder_inner_trace_cost: 0.0,
            min_component_solder_inner_trace_cost: 0.0,
            component_side_box: IntBox::EMPTY,
            solder_side_box: IntBox::EMPTY,
            inner_side_box: IntBox::EMPTY,
            box_is_empty: true,
            component_side_box_is_empty: true,
            solder_side_box_is_empty: true,
            inner_side_box_is_empty: true,
        };
        // `:63-71` — the guarded component-side min/max (the quirk
        // arm; an inactive layer 0 leaves both at 0.0).
        if this.layer_active[0] {
            if this.trace_costs[0].horizontal < this.trace_costs[0].vertical {
                this.min_component_side_trace_cost = this.trace_costs[0].horizontal;
                this.max_component_side_trace_cost = this.trace_costs[0].vertical;
            } else {
                this.min_component_side_trace_cost = this.trace_costs[0].vertical;
                this.max_component_side_trace_cost = this.trace_costs[0].horizontal;
            }
        }
        // `:73-83` — the guarded solder-side min/max (same quirk).
        let last = this.layer_count - 1;
        if this.layer_active[last] {
            if this.trace_costs[last].horizontal < this.trace_costs[last].vertical {
                this.min_solder_side_trace_cost = this.trace_costs[last].horizontal;
                this.max_solder_side_trace_cost = this.trace_costs[last].vertical;
            } else {
                this.min_solder_side_trace_cost = this.trace_costs[last].vertical;
                this.max_solder_side_trace_cost = this.trace_costs[last].horizontal;
            }
        }
        // `:85-94`. The :85 comment CLAIMS inner layers cost 1 in the
        // preferred direction; the CODE takes max(h, v) per ACTIVE
        // inner layer and min-accumulates (`:91-93`). Documented
        // behavior is the code's.
        this.max_inner_side_trace_cost = this
            .max_component_side_trace_cost
            .min(this.max_solder_side_trace_cost);
        for ind2 in 1..last {
            if !this.layer_active[ind2] {
                continue;
            }
            let current_max_cost = this.trace_costs[ind2]
                .horizontal
                .max(this.trace_costs[ind2].vertical);
            this.max_inner_side_trace_cost = this.max_inner_side_trace_cost.min(current_max_cost);
        }
        // `:95-98` — with an inactive outer layer these mins inherit
        // its 0.0 (the quirk's downstream half).
        this.min_component_inner_trace_cost = this
            .min_component_side_trace_cost
            .min(this.max_inner_side_trace_cost);
        this.min_solder_inner_trace_cost = this
            .min_solder_side_trace_cost
            .min(this.max_inner_side_trace_cost);
        this.min_component_solder_inner_trace_cost = this
            .min_component_inner_trace_cost
            .min(this.min_solder_inner_trace_cost);
        this
    }

    /// The engine construction site (`MazeSearchEngine.java:126-133`):
    /// `new DestinationDistance(ctrl.traceCosts, ctrl.layerActive,
    /// ctrl.minNormalViaCost, ctrl.minCheapViaCost)`.
    pub fn from_ctrl(ctrl: &AutorouteControl) -> Self {
        Self::new(
            &ctrl.trace_costs,
            &ctrl.layer_active,
            ctrl.min_normal_via_cost,
            ctrl.min_cheap_via_cost,
        )
    }

    /// The joined component-side bucket. Java's fields are
    /// package-visible (the same-package MazeSpike oracle reads them
    /// directly); this is the crate-level mirror of that visibility.
    /// Test-only until a production consumer (T9/T11 probes) needs it.
    #[cfg(test)]
    pub(crate) fn component_side_box(&self) -> &IntBox {
        &self.component_side_box
    }

    /// The joined solder-side bucket (see [`Self::component_side_box`]).
    #[cfg(test)]
    pub(crate) fn solder_side_box(&self) -> &IntBox {
        &self.solder_side_box
    }

    /// The joined inner-side bucket (see [`Self::component_side_box`]).
    #[cfg(test)]
    pub(crate) fn inner_side_box(&self) -> &IntBox {
        &self.inner_side_box
    }

    /// Java `join(IntBox, int)` (`:102-114`) — bucket the box by
    /// layer: 0 → component side, `layerCount-1` → solder side, else
    /// inner side; `IntBox::union` per bucket (EMPTY identity), each
    /// bucket its own is-empty flag, and ANY join clears the global
    /// `boxIsEmpty` (the sentinel gate).
    pub fn join(&mut self, shape: &IntBox, layer: i32) {
        if layer == 0 {
            self.component_side_box = self.component_side_box.union(shape);
            self.component_side_box_is_empty = false;
        } else if layer == self.layer_count as i32 - 1 {
            self.solder_side_box = self.solder_side_box.union(shape);
            self.solder_side_box_is_empty = false;
        } else {
            self.inner_side_box = self.inner_side_box.union(shape);
            self.inner_side_box_is_empty = false;
        }
        self.box_is_empty = false;
    }

    /// Java `calculate(IntBox, int)` (`:122-379`) — the layered
    /// lower-bound minimum. The port keeps Java's structure verbatim
    /// (all inputs finite; no NaN arms).
    pub fn calculate_box(&self, shape: &IntBox, layer: i32) -> f64 {
        if self.box_is_empty {
            return INT_MAX_AS_DOUBLE;
        }
        // `:127-182` — the outside-distance deltas per bucket: STRICT
        // comparisons on both arms, else 0 (an overlap contributes a
        // zero delta even when the other axis is far).
        let delta = |side_box: &IntBox| -> (f64, f64) {
            let dx = if shape.ll.x > side_box.ur.x {
                f64::from(shape.ll.x - side_box.ur.x)
            } else if shape.ur.x < side_box.ll.x {
                f64::from(side_box.ll.x - shape.ur.x)
            } else {
                0.0
            };
            let dy = if shape.ll.y > side_box.ur.y {
                f64::from(shape.ll.y - side_box.ur.y)
            } else if shape.ur.y < side_box.ll.y {
                f64::from(side_box.ll.y - shape.ur.y)
            } else {
                0.0
            };
            (dx, dy)
        };
        let (component_dx, component_dy) = delta(&self.component_side_box);
        let (solder_dx, solder_dy) = delta(&self.solder_side_box);
        let (inner_dx, inner_dy) = delta(&self.inner_side_box);
        // `:187-215` — the max/min pairing; ties (`dx == dy`) fall to
        // the ELSE arm (the swap), which is exact for equal values.
        let split = |dx: f64, dy: f64| -> (f64, f64) { if dx > dy { (dx, dy) } else { (dy, dx) } };
        let (component_max, component_min) = split(component_dx, component_dy);
        let (solder_max, solder_min) = split(solder_dx, solder_dy);
        let (inner_max, inner_min) = split(inner_dx, inner_dy);

        // `:217` — the sentinel is ALSO the initial result: the
        // "no bucket found + ladder exhausted" fallthrough keeps it.
        let mut result = INT_MAX_AS_DOUBLE;
        let min_normal = self.min_normal_via_cost;
        if layer == 0 {
            // `:219-294` — the component-side branch.
            if !self.component_side_box_is_empty {
                result = shape.weighted_distance(
                    &self.component_side_box,
                    self.trace_costs[0].horizontal,
                    self.trace_costs[0].vertical,
                );
            }
            // `:228` — NOTE the `<=`: activeLayerCount 1 AND 0 both
            // return here (the `==1` mutant is killed by an
            // activeLayerCount-0 world, where the fallthrough arms
            // shrink the result).
            if self.active_layer_count <= 1 {
                return result;
            }
            // `:234-245` — the cost-ordered two-layer pair. The
            // CHEAPER side's cost rides the MAX delta. The solder-side
            // branch (`:306-316`) flips this comparison — never
            // copy-paste between the two.
            let tmp = if self.min_solder_side_trace_cost < self.min_component_side_trace_cost {
                self.min_solder_side_trace_cost * solder_max
                    + self.min_component_side_trace_cost * solder_min
                    + min_normal
            } else {
                self.min_component_side_trace_cost * solder_max
                    + self.min_solder_side_trace_cost * solder_min
                    + min_normal
            };
            result = result.min(tmp);
            // `:252-257` — two-layer with two vias.
            let tmp = component_max
                + component_min * self.min_component_inner_trace_cost
                + 2.0 * min_normal;
            result = result.min(tmp);
            // `:259` — `==` here (contrast the `<=` above).
            if self.active_layer_count == 2 {
                return result;
            }
            // `:265-268` — component side + an inner side.
            let tmp = inner_max + inner_min * self.min_component_inner_trace_cost + min_normal;
            result = result.min(tmp);
            // `:272-276` — three-layer. Java writes
            // `+ +minComponentSolderInnerTraceCost` (a double unary,
            // `:274`) — it compiles to a single addition; kept as one.
            let tmp = solder_max
                + self.min_component_solder_inner_trace_cost * solder_min
                + 2.0 * min_normal;
            result = result.min(tmp);
            let tmp = component_max + component_min + 2.0 * min_normal;
            result = result.min(tmp);
            // `:281` — `==` again.
            if self.active_layer_count == 3 {
                return result;
            }
            // `:285` + `:291-293` — inner two-layer, then the
            // four-layer arm.
            let tmp = inner_max + inner_min + 2.0 * min_normal;
            result = result.min(tmp);
            let tmp = solder_max + solder_min + 3.0 * min_normal;
            return result.min(tmp);
        }
        let last = self.layer_count as i32 - 1;
        if layer == last {
            // `:295-347` — the solder-side branch. NOT a textual
            // mirror of the layer-0 branch: each arm is ported from
            // its own lines.
            if !self.solder_side_box_is_empty {
                result = shape.weighted_distance(
                    &self.solder_side_box,
                    self.trace_costs[last as usize].horizontal,
                    self.trace_costs[last as usize].vertical,
                );
            }
            // `:306-316` — the OPPOSITE pair comparison (component
            // cheaper takes the TRUE arm here; the layer-0 branch
            // asked solder cheaper).
            let tmp = if self.min_component_side_trace_cost < self.min_solder_side_trace_cost {
                self.min_component_side_trace_cost * component_max
                    + self.min_solder_side_trace_cost * component_min
                    + min_normal
            } else {
                self.min_solder_side_trace_cost * component_max
                    + self.min_component_side_trace_cost * component_min
                    + min_normal
            };
            result = result.min(tmp);
            let tmp = solder_max + solder_min * self.min_solder_inner_trace_cost + 2.0 * min_normal;
            result = result.min(tmp);
            // `:321` — `<=` (mixed comparators within this branch:
            // `<=2` then `==3`).
            if self.active_layer_count <= 2 {
                return result;
            }
            let tmp = inner_min * self.min_solder_inner_trace_cost + inner_max + min_normal;
            result = result.min(tmp);
            let tmp = component_max
                + self.min_component_solder_inner_trace_cost * component_min
                + 2.0 * min_normal;
            result = result.min(tmp);
            let tmp = solder_max + solder_min + 2.0 * min_normal;
            result = result.min(tmp);
            // `:337` — `==`.
            if self.active_layer_count == 3 {
                return result;
            }
            let tmp = inner_max + inner_min + 2.0 * min_normal;
            result = result.min(tmp);
            let tmp = component_max + component_min + 3.0 * min_normal;
            return result.min(tmp);
        }
        // `:349-378` — an inner layer. NO activeLayerCount early
        // returns in this branch.
        if !self.inner_side_box_is_empty {
            result = shape.weighted_distance(
                &self.inner_side_box,
                self.trace_costs[layer as usize].horizontal,
                self.trace_costs[layer as usize].vertical,
            );
        }
        let tmp = inner_max + inner_min + min_normal;
        result = result.min(tmp);
        let tmp = component_max + component_min * self.min_component_inner_trace_cost + min_normal;
        result = result.min(tmp);
        let tmp = solder_max + solder_min * self.min_solder_inner_trace_cost + min_normal;
        result = result.min(tmp);
        let tmp = component_max + component_min + 2.0 * min_normal;
        result = result.min(tmp);
        let tmp = solder_max + solder_min + 2.0 * min_normal;
        result.min(tmp)
    }

    /// Java `calculateCheapDistance(IntBox, int)` (`:382-390`) —
    /// saves `minNormalViaCost`, substitutes `minCheapViaCost`,
    /// calculates, RESTORES. Zero call sites in the entire Java tree
    /// (grep-verified; see SEAM.md) — ported as public surface with
    /// the save/restore semantics pinned; `&mut self` is the honest
    /// shape for the temporary mutation.
    pub fn calculate_cheap_distance(&mut self, shape: &IntBox, layer: i32) -> f64 {
        let min_normal_via_cost_save = self.min_normal_via_cost;
        self.min_normal_via_cost = self.min_cheap_via_cost;
        let result = self.calculate_box(shape, layer);
        self.min_normal_via_cost = min_normal_via_cost_save;
        result
    }
}

impl DestinationDistanceTrait for DestinationDistance {
    /// Java `calculate(FloatPoint, int)` (`:117-119`) — a point is a
    /// DEGENERATE bounding box (the DrillItem.java:359 dispatch
    /// class of degeneracy).
    fn calculate(&self, middle: &FloatPoint, layer: i32) -> f64 {
        self.calculate_box(&middle.bounding_box(), layer)
    }

    fn join(&mut self, shape: &IntBox, layer: i32) {
        DestinationDistance::join(self, shape, layer);
    }
}

#[cfg(test)]
mod tests {
    //! Jar-capture pins — the literals are `Double.toString` rows of
    //! `rust/harness/oracle/MazeSpike.java`'s T8 crafted-world battery
    //! (`logs/M3-T8/t8q_capture_{1,2}.rows`, 946 rows = 730 `dd` + 182
    //! legacy + 34 quality-round rows, run TWICE byte identical; the
    //! 911-line content prefix is byte-identical to the open-round
    //! `t8_capture_5.rows`, and the 182 pre-existing T6 rows are
    //! unchanged by the spy). Probe boxes P0-P6 are shared by
    //! every world: P0 == TGT
    //! (all deltas 0), P1 overlap, P2 right (deltaX only), P3 above
    //! (deltaY only), P4 diagonal TIE (dx == dy → the split's ELSE
    //! arm), P5 below-left (both deltas), P6 x-overlap/y-disjoint
    //! (the two-layer via arm wins — the cheap-distance probe).

    use super::*;
    use epic_geometry::int_point::IntPoint;

    /// Via costs 400/320 — the capture's `normalVia`/`cheapVia`.
    const NORMAL_VIA: f64 = 400.0;
    const CHEAP_VIA: f64 = 320.0;

    fn dist(costs: &[(f64, f64)], active: &[bool]) -> DestinationDistance {
        let trace_costs: Vec<ExpansionCostFactor> = costs
            .iter()
            .map(|&(h, v)| ExpansionCostFactor {
                horizontal: h,
                vertical: v,
            })
            .collect();
        DestinationDistance::new(&trace_costs, active, NORMAL_VIA, CHEAP_VIA)
    }

    fn box_at(llx: i32, lly: i32, urx: i32, ury: i32) -> IntBox {
        IntBox::new(IntPoint::new(llx, lly), IntPoint::new(urx, ury))
    }

    fn tgt() -> IntBox {
        box_at(100000, 200000, 200000, 300000)
    }

    /// The A-world cost table (4 layers, all active).
    const COSTS_A: &[(f64, f64)] = &[(1.0, 2.7), (1.6, 1.0), (2.0, 3.0), (1.0, 1.0)];

    /// Asserts one `ddCtor` capture row — ALL EIGHT fields QUOTED
    /// (MQ5: the 8th was re-derived as `expected[6].min(expected[5])`,
    /// a formula that coincides with the production port's on every
    /// pre-R world, so the ctor mutant survived the matrix).
    #[track_caller]
    fn assert_ctor(dd: &DestinationDistance, expected: [f64; 8]) {
        assert_eq!(dd.min_component_side_trace_cost, expected[0]);
        assert_eq!(dd.max_component_side_trace_cost, expected[1]);
        assert_eq!(dd.min_solder_side_trace_cost, expected[2]);
        assert_eq!(dd.max_solder_side_trace_cost, expected[3]);
        assert_eq!(dd.max_inner_side_trace_cost, expected[4]);
        assert_eq!(dd.min_component_inner_trace_cost, expected[5]);
        assert_eq!(dd.min_solder_inner_trace_cost, expected[6]);
        assert_eq!(dd.min_component_solder_inner_trace_cost, expected[7]);
    }

    /// All ten `ddCtor` rows of the capture. The QUIRK worlds are the
    /// load-bearing half: Q (layer 0 inactive) leaves the component
    /// fields at 0.0, and `maxInner = min(0.0, 1.0) = 0.0` propagates
    /// into ALL three inner mins (`:86`, `:95-98`); D0 and G show the
    /// same on 1- and 2-layer boards. The all-active witness A proves
    /// a "clean" skip-inactive port would also flip these rows.
    #[test]
    fn ctor_derivation_matrix() {
        // A: 4L all active — ddCtor A (8th field QUOTED throughout:
        // MQ5 — the derived form coincided with the wrong formula).
        assert_ctor(
            &dist(COSTS_A, &[true, true, true, true]),
            [1.0, 2.7, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0],
        );
        // B: 3L — ddCtor B (maxInner = min(2.7, 3.0, 1.6) = 1.6).
        assert_ctor(
            &dist(&[(1.0, 2.7), (1.6, 1.0), (2.0, 3.0)], &[true, true, true]),
            [1.0, 2.7, 2.0, 3.0, 1.6, 1.0, 1.6, 1.0],
        );
        // C: 2L — ddCtor C.
        assert_ctor(
            &dist(&[(1.0, 2.7), (1.6, 1.0)], &[true, true]),
            [1.0, 2.7, 1.0, 1.6, 1.6, 1.0, 1.0, 1.0],
        );
        // D1: 1L active — layer 0 is BOTH sides (layerCount-1 == 0),
        // both guards fire on the same (1.0, 2.7).
        assert_ctor(
            &dist(&[(1.0, 2.7)], &[true]),
            [1.0, 2.7, 1.0, 2.7, 2.7, 1.0, 1.0, 1.0],
        );
        // D0: 1L INACTIVE — every field stays at the Java numeric
        // default 0.0 (ddCtor D0).
        assert_ctor(&dist(&[(1.0, 2.7)], &[false]), [0.0; 8]);
        // E: 2L solder-cheaper — ddCtor E.
        assert_ctor(
            &dist(&[(2.0, 3.0), (1.0, 2.0)], &[true, true]),
            [2.0, 3.0, 1.0, 2.0, 2.0, 2.0, 1.0, 1.0],
        );
        // F: 2L component-cheaper — ddCtor F.
        assert_ctor(
            &dist(&[(1.0, 2.0), (2.0, 3.0)], &[true, true]),
            [1.0, 2.0, 2.0, 3.0, 2.0, 1.0, 2.0, 1.0],
        );
        // G: 2L solder INACTIVE — solder fields 0.0, maxInner
        // min(2.7, 0.0) = 0.0 (ddCtor G).
        assert_ctor(
            &dist(&[(1.0, 2.7), (1.6, 1.0)], &[true, false]),
            [1.0, 2.7, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        );
        // Q: 4L, layer 0 INACTIVE — THE quirk row (ddCtor Q):
        // minComponent/maxComponent 0.0 → maxInner 0.0 →
        // minSolderInner = min(1.0, 0.0) = 0.0.
        assert_ctor(
            &dist(COSTS_A, &[false, true, true, true]),
            [0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0],
        );
        // Qs: 4L, solder INACTIVE — the mirrored quirk (ddCtor Qs).
        assert_ctor(
            &dist(COSTS_A, &[true, true, true, false]),
            [1.0, 2.7, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        );
        // R: 3L with a CHEAP inner layer [(5,6),(1,1),(5,6)] — the
        // ctor 8th-field DISCRIMINATOR (ddCtor R): maxInner =
        // min(6, 1, 6) = 1.0, so minComponentSolderInner =
        // min(minComponentInner, minSolderInner) = min(1, 1) = 1.0;
        // the wrong formula min(minComponent, minSolder) = min(5, 5)
        // = 5.0 coincides with the true one on every other world
        // (H: min(1.1,1) = min(3,1) = 1; M: min(1.5,1.6) = min(1.5,
        // 1.7) = 1.5).
        assert_ctor(
            &dist(&[(5.0, 6.0), (1.0, 1.0), (5.0, 6.0)], &[true, true, true]),
            [5.0, 6.0, 5.0, 6.0, 1.0, 1.0, 1.0, 1.0],
        );
        // activeLayerCount: capture rows A=4, Q=3, D0=0.
        assert_eq!(
            dist(COSTS_A, &[true, true, true, true]).active_layer_count,
            4
        );
        assert_eq!(
            dist(COSTS_A, &[false, true, true, true]).active_layer_count,
            3
        );
        assert_eq!(dist(&[(1.0, 2.7)], &[false]).active_layer_count, 0);
    }

    /// The gate ladder's early returns, per branch and comparator.
    /// The `<=1` arm is pinned on BOTH sides of the boundary: count 1
    /// (D1) and count 0 (D0) — the `==1` mutant falls through on D0
    /// and shrinks the result to 400.0 (the all-zero quirk costs
    /// make the via arm win), so the D0 rows kill it.
    #[test]
    fn gate_ladder_early_returns() {
        // ddCalcP D1 id3 L0 → 270000.0 (count 1; the pure weighted
        // value 2.7 * 100000 — no via arm may touch it).
        let mut d1 = dist(&[(1.0, 2.7)], &[true]);
        d1.join(&tgt(), 0);
        assert_eq!(
            d1.calculate_box(&box_at(100000, 400000, 200000, 500000), 0),
            270000.0
        );
        // ddCalcP D0 id3 L0 → 270000.0 (count 0 returns here too).
        let mut d0 = dist(&[(1.0, 2.7)], &[false]);
        d0.join(&tgt(), 0);
        assert_eq!(
            d0.calculate_box(&box_at(100000, 400000, 200000, 500000), 0),
            270000.0
        );
        // ddCalcP C id2 L0 → 100000.0 / id2 L1 → 100400.0 (the `==2`
        // returns, component and solder branches). World C joins BOTH
        // layers {0,1} — a solder-branch probe with an EMPTY component
        // bucket would let the two-vias arm win (100800.0) instead of
        // the pair arm.
        let mut c0 = dist(&[(1.0, 2.7), (1.6, 1.0)], &[true, true]);
        c0.join(&tgt(), 0);
        let mut c1 = dist(&[(1.0, 2.7), (1.6, 1.0)], &[true, true]);
        c1.join(&tgt(), 0);
        c1.join(&tgt(), 1);
        assert_eq!(
            c0.calculate_box(&box_at(300000, 200000, 400000, 300000), 0),
            100000.0
        );
        assert_eq!(
            c1.calculate_box(&box_at(300000, 200000, 400000, 300000), 1),
            100400.0
        );
        // ddCalcP B id2 L0 → 100000.0 / id2 L2 → 100400.0 (the `==3`
        // returns).
        let mut b0 = dist(&[(1.0, 2.7), (1.6, 1.0), (2.0, 3.0)], &[true, true, true]);
        b0.join(&tgt(), 0);
        b0.join(&tgt(), 1);
        b0.join(&tgt(), 2);
        assert_eq!(
            b0.calculate_box(&box_at(300000, 200000, 400000, 300000), 0),
            100000.0
        );
        assert_eq!(
            b0.calculate_box(&box_at(300000, 200000, 400000, 300000), 2),
            100400.0
        );
        // ddCalcP B id4 L0 → 200400.0: the count-3 return includes the
        // :265-268 component+inner arm (200400) BELOW the pre-gate
        // two-vias minimum (200800) — a `>=3`-style mutant on the
        // `==2` gate's neighbor returns the prefix min instead.
        assert_eq!(
            b0.calculate_box(&box_at(300000, 400000, 400000, 500000), 0),
            200400.0
        );
        // ddCalcP B id5 L2 → 130800.0: the solder branch's post-`:321`
        // component arm (:331-334, 130800) beats the pre-gate pair arm
        // (170400) — an early-return mutant at `:321` keeps 170400.
        assert_eq!(
            b0.calculate_box(&box_at(50000, 100000, 60000, 110000), 2),
            130800.0
        );
        // The `==3` fallthrough contrast: the SAME probe and joins,
        // 3L world B (ddCalcP B id4 L2 → 200800.0, count 3 returns)
        // vs 4L world A (ddCalcP A id4 L3 → 141421.35623730952,
        // count 4 falls through and drops BELOW the 3L prefix min
        // through the four-layer arms) — a `>=3`/`!=3` mutant on
        // either gate flips one of these rows.
        let mut a = dist(COSTS_A, &[true, true, true, true]);
        a.join(&tgt(), 0);
        a.join(&tgt(), 1);
        a.join(&tgt(), 3);
        assert_eq!(
            a.calculate_box(&box_at(300000, 400000, 400000, 500000), 3),
            141421.35623730952
        );
        assert_eq!(
            b0.calculate_box(&box_at(300000, 400000, 400000, 500000), 2),
            200800.0
        );
        // The solder branch's `<=2` gate with count 1 (world G, layer
        // 1 INACTIVE — ddCalcP G id5 L1 → 40400.0; a `==2` mutant
        // falls through into the inner/three-layer arms).
        let mut g = dist(&[(1.0, 2.7), (1.6, 1.0)], &[true, false]);
        g.join(&tgt(), 0);
        assert_eq!(
            g.calculate_box(&box_at(50000, 100000, 60000, 110000), 1),
            40400.0
        );
        // And the count-2 face of the same gate, world F (both
        // active): ddCalcP F id5 L1 → 170400.0 — the mutant
        // discrimination lives in the mutation run below.
        let mut f = dist(&[(1.0, 2.0), (2.0, 3.0)], &[true, true]);
        f.join(&tgt(), 0);
        f.join(&tgt(), 1);
        assert_eq!(
            f.calculate_box(&box_at(50000, 100000, 60000, 110000), 1),
            170400.0
        );
    }

    /// The one-layer weightedDistance arm (world D1: the `<=1` gate
    /// returns before ANY via arm, so these rows are the pure
    /// `IntBox::weighted_distance` values against the joined
    /// component box).
    #[test]
    fn one_layer_weighted_arm() {
        let mut d1 = dist(&[(1.0, 2.7)], &[true]);
        d1.join(&tgt(), 0);
        // ddCalcP D1 id4 L0 → 287923.6009777594 (the diagonal probe).
        assert_eq!(
            d1.calculate_box(&box_at(300000, 400000, 400000, 500000), 0),
            287923.6009777594
        );
        // ddCalcP D1 id5 L0 → 246270.1768383659 (below-left probe).
        assert_eq!(
            d1.calculate_box(&box_at(50000, 100000, 60000, 110000), 0),
            246270.1768383659
        );
        // ddCalcP D1 id6 L0 → 54000.0 (x-overlap → the vertical term
        // 2.7 * 20000).
        assert_eq!(
            d1.calculate_box(&box_at(150000, 320000, 190000, 420000), 0),
            54000.0
        );
    }

    /// The fallthrough-shrink discriminators: the four gate mutants
    /// that survive every TGT-only world (where all buckets share one
    /// box, so a fallthrough/early-return arm can never beat the
    /// pre-gate minimum) are killed by the H / M / G3 capture worlds —
    /// expensive-solder costs or DIFFERING boxes per bucket. Mutation
    /// results: M05 (comp `==2`→`<=1`), M06 (comp `==3`→`<=2`),
    /// M07 (solder `<=2`→`==2`), M09 (solder `==3`→`<=2`) all died on
    /// this test after surviving the rest of the battery.
    #[test]
    fn gate_fallthrough_shrink_pins() {
        // World H (2L, solder 1.0/1.1 << component 3.0/4.0, TGT in
        // both buckets): ddCalcP H id5 L0 → 134800.0 — the `==2` gate
        // returns the two-vias arm (90000 + 40000*1.1 + 800); the
        // deletion mutant falls through to :272/:278 (90000 + 40000*
        // 1.0 + 800 = 130800).
        let mut h = dist(&[(3.0, 4.0), (1.0, 1.1)], &[true, true]);
        h.join(&tgt(), 0);
        h.join(&tgt(), 1);
        assert_eq!(
            h.calculate_box(&box_at(50000, 100000, 60000, 110000), 0),
            134800.0
        );
        // World M (3L, all costs ≥ 1.5; FAR TGT component, NEAR inner
        // [405000,305000,410000,310000], MID solder [500000,200000,
        // 600000,300000]):
        // ddCalcP M id2 L0 → 12900.0 — the `==3` gate returns the
        // :265 inner-pair arm (inner_max 5000 + inner_min 5000*1.5 +
        // 400 = 12900); the deletion mutant falls to :285 (inner_max
        // 5000 + inner_min 5000 + 800 = 10800).
        let mut m = dist(&[(1.5, 2.0), (1.6, 1.6), (1.7, 2.0)], &[true, true, true]);
        m.join(&tgt(), 0);
        m.join(&box_at(405000, 305000, 410000, 310000), 1);
        m.join(&box_at(500000, 200000, 600000, 300000), 2);
        assert_eq!(
            m.calculate_box(&box_at(300000, 200000, 400000, 300000), 0),
            12900.0
        );
        // ddCalcP M id5 L2 → 150800.0 — the solder branch's `==3` gate
        // returns the :331 component arm (90000 + 1.5*40000 + 800);
        // the deletion mutant falls to :343 (90000 + 40000 + 1200 =
        // 131200).
        assert_eq!(
            m.calculate_box(&box_at(50000, 100000, 60000, 110000), 2),
            150800.0
        );
        // World G3 (3L, ONLY layer 0 active — the count-1 quirk; TGT
        // component, NEAR inner [85000,140000,95000,150000], solder
        // bucket EMPTY): ddCalcP G3 id5 L2 → 40400.0 — the `<=2` gate
        // returns the quirk pair arm (0*90000 + 1.0*40000 + 400); the
        // `==2` mutant falls through to :329 (inner_min*0 + 30000 +
        // 400 = 30400).
        let mut g3 = dist(&[(1.0, 2.7), (1.6, 1.0), (2.0, 3.0)], &[true, false, false]);
        g3.join(&tgt(), 0);
        g3.join(&box_at(85000, 140000, 95000, 150000), 1);
        assert_eq!(
            g3.calculate_box(&box_at(50000, 100000, 60000, 110000), 2),
            40400.0
        );
    }

    /// The cost-ordered two-layer pair arms, BOTH ways on BOTH
    /// branches (`:235-245` vs `:306-316` — opposite comparisons).
    #[test]
    fn cost_ordered_pair_arms_both_ways() {
        // World E (solder 1.0 < component 2.0): layer-0 branch TRUE
        // arm — ddCalcP E id2 L0 → 100400.0
        // (1.0*100000 + 2.0*0 + 400; a swapped mutant gives 200400).
        let mut e = dist(&[(2.0, 3.0), (1.0, 2.0)], &[true, true]);
        e.join(&tgt(), 0);
        e.join(&tgt(), 1);
        assert_eq!(
            e.calculate_box(&box_at(300000, 200000, 400000, 300000), 0),
            100400.0
        );
        // World C (equal mins → the ELSE arm on both branches):
        // ddCalcP C id2 L0 → 100000.0 (weighted wins over the pair).
        // World F (component 1.0 < solder 2.0): solder-branch TRUE
        // arm — ddCalcP F id2 L1 → 100400.0
        // (1.0*100000 + 2.0*0 + 400; swapped mutant gives 200400).
        let mut f = dist(&[(1.0, 2.0), (2.0, 3.0)], &[true, true]);
        f.join(&tgt(), 0);
        f.join(&tgt(), 1);
        assert_eq!(
            f.calculate_box(&box_at(300000, 200000, 400000, 300000), 1),
            100400.0
        );
        // World E solder branch ELSE arm — ddCalcP E id2 L1 →
        // 100000.0 (the one-layer weighted arm wins over 100400).
        assert_eq!(
            e.calculate_box(&box_at(300000, 200000, 400000, 300000), 1),
            100000.0
        );
    }

    /// MQ1 — join ACCUMULATION: two DIFFERENT boxes joined into ONE
    /// bucket must union. No open-round world did this, so a
    /// last-join-wins mutant was observationally dead. Pin = the
    /// ddBucket MJ row's accumulated COORDINATES (component
    /// "100000 200000 400000 400000" = union(TGT,
    /// [300000,300000,400000,400000]); solder/inner "EMPTY") read
    /// through the `#[cfg(test)]` bucket accessors — their first live
    /// consumer.
    #[test]
    fn join_accumulation_bucket_coordinates() {
        let mut mj = dist(&[(1.0, 2.7), (1.6, 1.0)], &[true, true]);
        mj.join(&tgt(), 0);
        mj.join(&box_at(300000, 300000, 400000, 400000), 0);
        assert_eq!(
            mj.component_side_box(),
            &box_at(100000, 200000, 400000, 400000)
        );
        assert_eq!(mj.solder_side_box(), &IntBox::EMPTY);
        assert_eq!(mj.inner_side_box(), &IntBox::EMPTY);
    }

    /// MQ2 — the inner-branch tail arms never STRICTLY won in any
    /// open-round pin (both inner-layer pins came from the weighted
    /// arm), so a truncating mutant survived. World W (4L, FAR
    /// component bucket, TGT solder bucket, EMPTY inner): P0@L1 skips
    /// the weighted arm (inner empty → result starts at the sentinel)
    /// and the :369 solder-pair arm wins strictly at solder_max +
    /// solder_min*minSolderInner + via = 0 + 0*1.0 + 400 = 400.0
    /// (ddCalcP W id0 L1); the truncating mutant leaves :377's
    /// 0 + 0 + 800 = 800.0. Contrast id2/id3 = 100400.0 — the value
    /// the reviewer's truncation lands on for those probes.
    #[test]
    fn inner_tail_solder_pair_arm_wins() {
        let mut w = dist(COSTS_A, &[true, true, true, true]);
        w.join(&tgt(), 3);
        w.join(&box_at(900000, 900000, 950000, 950000), 0);
        assert_eq!(w.calculate_box(&tgt(), 1), 400.0);
    }

    /// MQ3 — the solder `==3` gate's count-4 face was unobserved: A's
    /// L3 pin is decided by the weighted arm, so an early-return-
    /// everything mutant survived. World X (4L ALL-expensive 5/6 —
    /// minSolderInner = minCompSolderInner = 5 — NEAR-mid component,
    /// FAR solder, empty inner): P4@L3 returns the FOUR-layer arm :345
    /// = comp_max + comp_min + 3*via = 50000 + 50000 + 1200 = 101200.0
    /// (ddCalcP X id4 L3), strictly under :330 (50000 + 5*50000 + 800
    /// = 300800), :335 (500000 + 400000 + 800 = 900800), :306
    /// (5*50000 + 5*50000 + 400 = 500400), :318 (500000 + 400000*5 +
    /// 800 = 2500800) and the weighted arm (~3.5e6). The `==3`→`<=3`
    /// mutant early-returns the pre-gate min 300800.0.
    #[test]
    fn solder_count4_face_four_layer_arm() {
        let mut x = dist(
            &[(5.0, 6.0), (5.0, 6.0), (5.0, 6.0), (5.0, 6.0)],
            &[true, true, true, true],
        );
        x.join(&box_at(450000, 550000, 550000, 650000), 0);
        x.join(&box_at(900000, 900000, 950000, 950000), 3);
        assert_eq!(
            x.calculate_box(&box_at(300000, 400000, 400000, 500000), 3),
            101200.0
        );
    }

    /// The `boxIsEmpty` sentinel (`:123-124` — Java
    /// `Integer.MAX_VALUE`, printed by the capture as
    /// "2.147483647E9") and the per-bucket is-empty arms.
    #[test]
    fn sentinel_and_bucket_empties() {
        // The SENTINEL row: a fresh instance, nothing joined.
        assert_eq!(
            dist(COSTS_A, &[true, true, true, true]).calculate_box(&tgt(), 0),
            2147483647.0
        );
        // World G, layer 1, probe P0 (ddCalcP G id0 L1 → 400.0): the
        // SOLDER bucket is empty (never joined — layer 1 is the
        // solder layer and only layer 0 was joined), so the weighted
        // arm is skipped and the quirk-zero pair arm (0*x + 0*y + 400)
        // is the whole answer.
        let mut g = dist(&[(1.0, 2.7), (1.6, 1.0)], &[true, false]);
        g.join(&tgt(), 0);
        assert_eq!(g.calculate_box(&tgt(), 1), 400.0);
        // World Q, layer 1 probe P6 (ddCalcP Q id6 L1 → 20000.0): the
        // INNER bucket is joined (layer 1 of a 4L board) and its
        // weighted arm runs with the layer's own costs (1.0 vertical).
        let mut q = dist(COSTS_A, &[false, true, true, true]);
        q.join(&tgt(), 0);
        q.join(&tgt(), 1);
        q.join(&tgt(), 3);
        assert_eq!(
            q.calculate_box(&box_at(150000, 320000, 190000, 420000), 1),
            20000.0
        );
    }

    /// The outside-distance delta zero-arms (strict comparisons, else
    /// 0) and the max/min split tie (world A, layer 0).
    #[test]
    fn delta_zero_arms_and_split_tie() {
        let mut a = dist(COSTS_A, &[true, true, true, true]);
        a.join(&tgt(), 0);
        a.join(&tgt(), 1);
        a.join(&tgt(), 3);
        // ddCalcP A id0 L0 → 0.0 (P0 == TGT: every delta is 0).
        assert_eq!(a.calculate_box(&tgt(), 0), 0.0);
        // ddCalcP A id2 L0 → 100000.0 (deltaX only).
        assert_eq!(
            a.calculate_box(&box_at(300000, 200000, 400000, 300000), 0),
            100000.0
        );
        // ddCalcP A id3 L0 → 100400.0 (deltaY only: the vertical
        // weighted 270000 LOSES to the two-layer via arm
        // 1.0*100000 + 0 + 400 — the zero deltaX rides the pair).
        assert_eq!(
            a.calculate_box(&box_at(100000, 400000, 200000, 500000), 0),
            100400.0
        );
        // ddCalcP A id4 L0 → 200400.0 (dx == dy == 100000: the split's
        // ELSE arm — max/min swap — decides which cost multiplies).
        assert_eq!(
            a.calculate_box(&box_at(300000, 400000, 400000, 500000), 0),
            200400.0
        );
        // ddCalcP A id5 L1 → 110435.50153822819 (both deltas, inner
        // layer, the diagonal weighted legs).
        assert_eq!(
            a.calculate_box(&box_at(50000, 100000, 60000, 110000), 1),
            110435.50153822819
        );
    }

    /// `calculateCheapDistance` save/substitute/restore (`:382-390`).
    /// CHEAPBOX (P6) is the probe where a VIA arm wins the minimum —
    /// the plain-weighted probes would coincide (cerebrum pin
    /// failure mode 4).
    #[test]
    fn calculate_cheap_distance_save_restore() {
        let mut a = dist(COSTS_A, &[true, true, true, true]);
        a.join(&tgt(), 0);
        a.join(&tgt(), 1);
        a.join(&tgt(), 3);
        let cheap_box = box_at(150000, 320000, 190000, 420000);
        // ddCalcP A id6 L0 → 20400.0 (normal: 1.0*20000 + 400).
        assert_eq!(a.calculate_box(&cheap_box, 0), 20400.0);
        // ddCheapP → 20320.0 (cheap: 1.0*20000 + 320).
        assert_eq!(a.calculate_cheap_distance(&cheap_box, 0), 20320.0);
        // ddRestoreP → 20400.0 (the restore is observable: a leaked
        // cheap cost would repeat 20320.0).
        assert_eq!(a.calculate_box(&cheap_box, 0), 20400.0);
        assert_eq!(a.min_normal_via_cost, NORMAL_VIA);
    }

    /// The FloatPoint overload (the trait path the ENGINE calls) is
    /// the degenerate-box dispatch — ddPointP rows.
    #[test]
    fn point_overload_degenerate_box() {
        let mut a = dist(COSTS_A, &[true, true, true, true]);
        a.join(&tgt(), 0);
        a.join(&tgt(), 1);
        a.join(&tgt(), 3);
        // ddPointP A id1: (350000, 250000) → L0 150000.0, L1 150400.0,
        // L3 150000.0.
        let p1 = FloatPoint::new(350000.0, 250000.0);
        assert_eq!(DestinationDistanceTrait::calculate(&a, &p1, 0), 150000.0);
        assert_eq!(DestinationDistanceTrait::calculate(&a, &p1, 1), 150400.0);
        assert_eq!(DestinationDistanceTrait::calculate(&a, &p1, 3), 150000.0);
        // ddPointP A id2: (350000, 450000) → L3 212132.03435596425.
        let p2 = FloatPoint::new(350000.0, 450000.0);
        assert_eq!(
            DestinationDistanceTrait::calculate(&a, &p2, 3),
            212132.03435596425
        );
    }
}
