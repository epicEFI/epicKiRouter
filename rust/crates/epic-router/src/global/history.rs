//! The PathFinder negotiated-congestion scheduler (M6-T8) — per-resource
//! history costs persisting across passes (design §4.2 stage 2, design
//! :24/:70). The most validated convergence mechanism in the field,
//! absent from Freerouting, whose linear pass-scaled costs + snapshot
//! restore are oscillation-prone: WHEN the setting is on, the negotiated
//! bases are consumed at the batch pass loop's scheduler seam as a
//! FLOOR over Java's `start_ripup_costs * pass` ladder (`max(negotiated,
//! linear)`, M11-T4 fix round 2026-10-03 — the original replace
//! composition capped every hot net at `2 * start` forever, which
//! removed the ladder's per-pass price escalation and sustained the
//! #931-cluster-F near-tie ripup limit cycle on gv-iu, buglog 256), and
//! the batch loop skips Java's snapshot-restore arm (the history IS the
//! convergence memory — a restore would rewind the routes while the
//! history remembers them). `router.congestion_global.pathfinder`,
//! default OFF — the parity contract is the default-off byte-identity
//! of the recurring gates.
//!
//! ## The model (beyond-Java: no oracle exists)
//!
//! A RESOURCE is one congestion-map cell on one signal layer (the M6-T7
//! grid, [`CongestionMap`]). Every resource carries a history value
//! `h(layer, ix, iy) >= 0`. At the END of each pass the history updates
//! from the pass-end board's congestion map (decay-then-increment):
//!
//! ```text
//! h' = min(h * HISTORY_DECAY_NUM / HISTORY_DECAY_DEN   (per-pass decay)
//!          + HISTORY_INCREMENT * overflow(cell),       (present pressure)
//!          HISTORY_MAX)                                (per-resource cap)
//! ```
//!
//! A resource abandoned by every net forgets (values below the decay
//! fraction fall to zero — 1*3/4 = 0 — sub-threshold congestion is
//! forgotten; intended). A re-congested resource re-arms from zero.
//!
//! At the BEGIN of each pass the scheduler derives the pass's NEGOTIATED
//! rip-up bases, per net, from the pass-START board's map:
//!
//! ```text
//! pressure(net) = SUM over the net's guide-region cells, all signal
//!                 layers, of (present overflow + HISTORY_MIX * history)
//! base(net)   = min(start + start * pressure / PRESSURE_SCALE,
//!                     start * BASE_CAP_FACTOR)
//! ```
//!
//! The RELATIVE cap (`2 * start`) keeps the maze's fanout-protection
//! threshold (`ripup_costs <= start * 2`, `maze/ripup.rs`) armed at
//! every pass: the M6-T8 tuning round found the un-capped negotiated
//! bases ripping fanout vias from pass 1 and churning interf_u's wall
//! by an order of magnitude — the cap is the evidence-backed fix.
//!
//! The present + history MIX is the PathFinder cost face: a net whose
//! region carries overflow or accumulated history pays more to rip
//! through contested resources, so nets negotiate around each other
//! instead of every net paying the same unbounded `start * pass`
//! (Freerouting's linear ladder).
//!
//! ## Determinism contract
//!
//! Pure functions of (board state at the pass boundary, the persisted
//! history): the map build is the M6-T7 pure face; history entries live
//! in a BTreeMap walked in ascending key order; integer arithmetic with
//! saturating clamps only; no HashMap, no wall clock. The ONE f64 face
//! is the guides-consumption bias ([`apply_preferred_layer_bias`],
//! [`GUIDE_LAYER_RANK_STEP`]): a per-layer scaling of the ctrl cost
//! vector in a FIXED layer order (never across a reordered iteration),
//! documented in place (the T8 quality Q1 contradiction fix — the
//! blanket "no float" claim was wrong while the bias arithmetic is
//! f64; the carve-out is named rather than moved because the bias
//! order-independence is structural, not incidental). The
//! per-pass [`PathfinderPass`] is immutable for the pass's duration and
//! shared with the partitioned executor through an `Arc` (the executor
//! serializes every unit, so the read side never races).

use std::collections::BTreeMap;

use epic_board::board::Board;

use crate::global::map::CongestionMap;
use crate::global::plan::GlobalPlan;

/// The per-pass history decay numerator (`h * 3 / 4` per pass).
pub(crate) const HISTORY_DECAY_NUM: i64 = 3;
/// The per-pass history decay denominator.
pub(crate) const HISTORY_DECAY_DEN: i64 = 4;
/// The history increment per overflow unit per pass.
pub(crate) const HISTORY_INCREMENT: i64 = 1;
/// The per-resource history cap.
pub(crate) const HISTORY_MAX: i64 = 1 << 20;
/// The history weight in the present + history mix (1:1 with the
/// present overflow).
pub(crate) const HISTORY_MIX: i64 = 1;
/// The pressure units per `+start_ripup_costs` (100%) of negotiated
/// base: `base = start + start * pressure / PRESSURE_SCALE`, so the
/// pressure range 0..=PRESSURE_SCALE resolves the full [start,
/// 2*start] band the relative cap allows.
pub(crate) const PRESSURE_SCALE: i64 = 100;
/// The negotiated-base RELATIVE cap, in units of `start_ripup_costs`:
/// the negotiated COMPONENT never exceeds `2 * start`. TUNING (M6-T8,
/// evidence-backed): the maze's fanout-protection arm (`maze/ripup.rs`
/// `preserve_fanout_protection = ripup_costs <= start * 2`) must stay
/// armed for the negotiated band — the un-capped bases ripped fanout
/// vias from pass 1 and churned interf_u's wall by an order of
/// magnitude. M11-T4 fix round (2026-10-03): the seam now floors the
/// negotiated component over Java's linear ladder, so the CONSUMED
/// base exceeds `2 * start` from pass 3 on wherever the ladder does —
/// disarming fanout protection exactly as Java's own late passes
/// (the ladder is the dampener; the cap governs only the negotiated
/// component). `RIPUP_CAP` remains the absolute i32-face ceiling.
pub(crate) const BASE_CAP_FACTOR: i64 = 2;
/// The absolute negotiated-base ceiling (the maze ripup-arithmetic
/// saturation face — `maze/ripup.rs` clamps at the same value).
pub(crate) const RIPUP_CAP: i64 = i32::MAX as i64 / 100;
/// The guides-consumption face (the T7 residual: preferred-layers was
/// computed but unconsumed): the maze's per-layer trace costs get a
/// `1 + rank * GUIDE_LAYER_RANK_STEP` multiplier, where `rank` is the
/// layer's position in the net's guide preference order (0 = most
/// preferred / least congested). A pure f64 scaling of the ctrl cost
/// vector, fixed order — the containment face is a cost bias, not a
/// geometric clamp (the report's relaxation note).
pub(crate) const GUIDE_LAYER_RANK_STEP: f64 = 0.5;

/// One history entry key: (signal ordinal, ix, iy) in the grid
/// coordinates of the geometry captured at the first update.
type HistoryKey = (usize, usize, usize);

/// The per-resource history costs, persisted across passes.
#[derive(Debug, Default, Clone)]
pub struct HistoryCosts {
    /// The grid geometry the keys refer to (captured at the first
    /// update; a later map with a DIFFERENT geometry resets the
    /// history — a new coordinate system, the old values are not
    /// comparable).
    geometry: Option<(i64, (i64, i64))>,
    /// The history values, ascending-key walked (determinism docs).
    /// Crate-visible for the boundary pins (`global/tests.rs` seeds
    /// entries directly — the closed-form decay ladder).
    pub(crate) entries: BTreeMap<HistoryKey, i64>,
}

impl HistoryCosts {
    /// The end-of-pass update: decay every entry, then add
    /// `HISTORY_INCREMENT * overflow` for every resource the pass-end
    /// map carries overflow on. A map whose geometry differs from the
    /// stored one resets the history first (module docs).
    pub fn update(&mut self, map: &CongestionMap) {
        let geometry = (map.cell_size(), map.grid_origin());
        if self.geometry.is_some() && self.geometry != Some(geometry) {
            self.entries.clear();
        }
        self.geometry = Some(geometry);
        // Decay (ascending key order — the BTreeMap walk); entries
        // that decay to zero are PRUNED (forgotten — the pin face
        // below holds `entry(...) == None` for them).
        for value in self.entries.values_mut() {
            *value = (*value * HISTORY_DECAY_NUM / HISTORY_DECAY_DEN).min(HISTORY_MAX);
        }
        self.entries.retain(|_, value| *value > 0);
        // The increments (layer-major, row-major — the map's canonical
        // cell order).
        for layer in 0..map.total_overflow().len() {
            let (nx, ny) = map.grid_dims();
            for iy in 0..ny {
                for ix in 0..nx {
                    let overflow = map.overflow(ix, iy, layer, None);
                    if overflow <= 0 {
                        continue;
                    }
                    let entry = self.entries.entry((layer, ix, iy)).or_insert(0);
                    *entry = (*entry + HISTORY_INCREMENT * overflow).min(HISTORY_MAX);
                }
            }
        }
    }

    /// The history value at a resource (0 = none).
    fn history_at(&self, layer: usize, ix: usize, iy: usize) -> i64 {
        self.entries.get(&(layer, ix, iy)).copied().unwrap_or(0)
    }

    /// The region's history sum (the negotiated-pressure input).
    fn history_sum_region(
        &self,
        layers: usize,
        ix0: usize,
        ix1: usize,
        iy0: usize,
        iy1: usize,
    ) -> i64 {
        let mut sum = 0i64;
        for layer in 0..layers {
            for iy in iy0..=iy1 {
                for ix in ix0..=ix1 {
                    sum += self.history_at(layer, ix, iy);
                }
            }
        }
        sum
    }

    /// The exact entry value (the pin face; None = no entry).
    #[must_use]
    pub fn entry(&self, layer: usize, ix: usize, iy: usize) -> Option<i64> {
        self.entries.get(&(layer, ix, iy)).copied()
    }

    /// The entry count (the observability/pin face).
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no entry is carried.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// The per-pass negotiated state: the negotiated rip-up bases per net,
/// immutable for the pass's duration.
#[derive(Debug, Clone, Default)]
pub struct PathfinderPass {
    /// net number -> negotiated rip-up base (absent nets fall back to
    /// the linear face at the seam).
    pub bases: BTreeMap<i32, i32>,
    /// net number -> PHYSICAL layer indices, most preferred (least
    /// congested) first — consumed by the detail route's per-layer
    /// cost bias ([`apply_preferred_layer_bias`]).
    pub preferred_layers: BTreeMap<i32, Vec<i32>>,
}

/// The pass-boundary scheduler: owned by the batch driver for the
/// batch's lifetime. `begin_pass` derives the pass's negotiated bases
/// from the pass-START board + the persisted history; `end_pass` folds
/// the pass-END congestion into the history.
#[derive(Debug, Default)]
pub struct PathFinder {
    history: HistoryCosts,
}

impl PathFinder {
    /// The fresh scheduler (empty history — pass 1 runs on present
    /// congestion only).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The pass-opening face: builds the plan over the pass-START
    /// board, sums each net's guide-region pressure (present + history
    /// mix), and derives the negotiated bases.
    #[must_use]
    pub fn begin_pass(&mut self, board: &mut Board, start_ripup_costs: i32) -> PathfinderPass {
        let plan = GlobalPlan::build(board);
        let map = plan.map();
        let layers = map.total_overflow().len();
        let mut bases = BTreeMap::new();
        let mut preferred_layers = BTreeMap::new();
        for guide in plan.guides() {
            let (ix0, ix1, iy0, iy1) = guide.region_cells_debug();
            let mut present = 0i64;
            for layer in 0..layers {
                for iy in iy0..=iy1 {
                    for ix in ix0..=ix1 {
                        present += map.overflow(ix, iy, layer, None);
                    }
                }
            }
            let history = self.history.history_sum_region(layers, ix0, ix1, iy0, iy1);
            let pressure = present + HISTORY_MIX * history;
            let base = negotiated_base(pressure, i64::from(start_ripup_costs));
            bases.insert(guide.net_no(), base);
            // The guide's preferred layers (signal ordinals, least
            // congested first) mapped to PHYSICAL indices.
            let physical: Vec<i32> = guide
                .layers()
                .iter()
                .map(|&ordinal| board.layers().get_layer_no(ordinal as i32))
                .collect();
            preferred_layers.insert(guide.net_no(), physical);
        }
        PathfinderPass {
            bases,
            preferred_layers,
        }
    }

    /// The pass-closing face: folds the pass-END congestion into the
    /// history (decay + increments, module docs).
    pub fn end_pass(&mut self, board: &mut Board) {
        let map = CongestionMap::build(board);
        self.history.update(&map);
    }

    /// The history (the pin/observability face).
    #[must_use]
    pub fn history(&self) -> &HistoryCosts {
        &self.history
    }
}

/// The negotiated-base formula (isolated for the DNR-16 boundary
/// pins): `min(start + start * pressure / PRESSURE_SCALE,
/// min(start * BASE_CAP_FACTOR, RIPUP_CAP))` — integer arithmetic,
/// saturating at the i32 face. The relative cap keeps the maze's
/// fanout-protection threshold (`ripup_costs <= start * 2`) armed
/// across the NEGOTIATED band (the M6-T8 tuning finding; module
/// docs); the consuming seam floors this over Java's linear ladder
/// (M11-T4 fix round), so the consumed base may exceed the cap
/// wherever the ladder does.
pub(crate) fn negotiated_base(pressure: i64, start_ripup_costs: i64) -> i32 {
    let relative_cap = start_ripup_costs
        .saturating_mul(BASE_CAP_FACTOR)
        .min(RIPUP_CAP);
    let scaled_add = start_ripup_costs
        .checked_mul(pressure)
        .map_or(i64::MAX, |v| v / PRESSURE_SCALE);
    let base = start_ripup_costs
        .saturating_add(scaled_add)
        .min(relative_cap);
    i32::try_from(base).unwrap_or(i32::MAX)
}

/// The guides-consumption bias (M6-T8): scales `trace_costs` (the
/// ctrl's per-PHYSICAL-layer cost rows) by
/// `1 + rank * GUIDE_LAYER_RANK_STEP`, where `rank` is the layer's
/// position in `preferred` (the net's guide order). Layers absent from
/// `preferred` (non-signal rows) are left untouched. Deterministic:
/// fixed iteration order, pure f64 arithmetic.
pub fn apply_preferred_layer_bias(
    trace_costs: &mut [crate::control::ExpansionCostFactor],
    preferred: &[i32],
) {
    let mut rank_of: BTreeMap<i32, usize> = BTreeMap::new();
    for (rank, layer) in preferred.iter().enumerate() {
        rank_of.entry(*layer).or_insert(rank);
    }
    for (layer_index, factor) in trace_costs.iter_mut().enumerate() {
        let layer = layer_index as i32;
        if let Some(&rank) = rank_of.get(&layer) {
            let multiplier = 1.0 + rank as f64 * GUIDE_LAYER_RANK_STEP;
            factor.horizontal *= multiplier;
            factor.vertical *= multiplier;
        }
    }
}
