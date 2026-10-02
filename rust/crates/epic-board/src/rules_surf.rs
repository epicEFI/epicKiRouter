//! The rules READ surface (M2 Task 3): the clearance matrix, the net
//! table, net classes, via infos/rules, and the `BoardRules` container —
//! built one-shot from the epic-dsn IR in
//! [`crate::board::Board::from_ses_board`].
//!
//! Java anchors: `rules/ClearanceMatrix.java`, `rules/Nets.java`,
//! `rules/NetClass.java`, `rules/ViaInfo.java`, `rules/ViaRule.java`,
//! `rules/BoardRules.java`, and
//! `board/model/structure/AngleRestriction.java`.
//!
//! ## The clearance-matrix index order (T54 — parity-critical)
//!
//! Java stores `row[classJ].column[classI].layer[layer]`; `getValue(i, j,
//! layer)` reads exactly that cell (`ClearanceMatrix.java:163`). The port
//! stores `values[layer][j][i]` — the SAME nesting the epic-dsn IR uses
//! (`epic_dsn::sink::ClearanceIr`, whose `get_value` is the verified
//! M1b reference). Swapping `i`/`j` is only observable when the matrix
//! is ASYMMETRIC, and it can be: `setValue` writes the ONE cell
//! `row[classJ].column[classI]` — NO transpose fill
//! (`ClearanceMatrix.java:100-118`, jar-verified against the source);
//! the two `(spacing ...)` directions a DSN declares need not agree.
//!
//! ## The maintained maxima are accumulators, not traversals
//!
//! `maxValue(class, layer)` returns `row[class].maxValue[layer]` — a
//! per-row high-water mark that `setValue` only ever RAISES
//! (`ClearanceMatrix.java:116`), never recomputes. For a matrix built
//! by a sequence of `set_value` calls this port is bit-identical to
//! Java. The IR conversion ([`ClearanceMatrix::from_ir`]) reconstructs
//! the marks from the FINAL values; the two differ only if a cell was
//! overwritten DOWNWARD mid-parse (Java's mark would stay at the old
//! high). The epic-dsn IR holds final values only, so that corner is
//! unrepresentable in the IR — documented divergence, not observable
//! through any M2 gate (the digest reads `getValue`, never the maxima).

use epic_dsn::sink::{BoardRulesIr, ClearanceIr, NetClassIr, NetIr, ViaInfoIr, ViaRuleIr};

/// Java `board.model.structure.AngleRestriction` (`AngleRestriction.java`):
/// "ordinal() and values() rely on the order" — NONE, FORTYFIVE_DEGREE,
/// NINETY_DEGREE (T59: the autoroute tree selection dispatches on the
/// ordinal; keep the declaration order).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AngleRestriction {
    /// Java `NONE` (ordinal 0).
    #[default]
    None,
    /// Java `FORTYFIVE_DEGREE` (ordinal 1) — the parse-state default.
    FortyfiveDegree,
    /// Java `NINETY_DEGREE` (ordinal 2).
    NinetyDegree,
}

impl AngleRestriction {
    /// Java `AngleRestriction.valueOf(i)` = `values()[i]` — the
    /// ORDINAL-LOAD-BEARING accessor (T59). DIVERGENCE: Java throws
    /// `ArrayIndexOutOfBoundsException` for an ordinal outside
    /// `0..=2`; the port maps every out-of-range ordinal to
    /// `NinetyDegree` (the `_` arm). Parse-unreachable — the
    /// epic-dsn IR enum serializes exactly the three Java ordinals,
    /// so `read_board` never feeds another value here.
    #[must_use]
    pub fn value_of(value: i32) -> Self {
        match value {
            0 => Self::None,
            1 => Self::FortyfiveDegree,
            _ => Self::NinetyDegree,
        }
    }

    /// Java `getValue()` = `ordinal()`.
    #[must_use]
    pub fn get_value(self) -> i32 {
        match self {
            Self::None => 0,
            Self::FortyfiveDegree => 1,
            Self::NinetyDegree => 2,
        }
    }

    /// The conversion from the epic-dsn parse-state enum (same
    /// declaration order).
    #[must_use]
    pub fn from_ir(restriction: epic_dsn::state::AngleRestriction) -> Self {
        match restriction {
            epic_dsn::state::AngleRestriction::None => Self::None,
            epic_dsn::state::AngleRestriction::FortyfiveDegree => Self::FortyfiveDegree,
            epic_dsn::state::AngleRestriction::NinetyDegree => Self::NinetyDegree,
        }
    }
}

/// Java `ClearanceMatrix.clearance_safety_margin` (`:17`) — added by
/// `getValue(..., addSafetyMargin = true)` ONLY. Every Task 6 tree path
/// passes `false` (`ShapeSearchTree.clearanceCompensationValue` `:110`,
/// `drillHoleClearanceDelta` `:1071-1073`); the `true` form is the
/// WithClearance query (Task 8). Captured on the Task 6 fixtures
/// (`MARGIN` lines, `/tmp/epic-t6-treeshapes.out` and
/// `/tmp/epic-t6-treeshapes-1bitsy.out`): Issue575 `v1.1.l0` no=2000
/// with=2016, 1Bitsy no=1490 with=1506.
pub const CLEARANCE_SAFETY_MARGIN: i32 = 16;

/// Java `rules.ClearanceMatrix` — the NxN-per-layer spacing table.
///
/// Storage `values[layer][j][i]` (module docs, T54). All values are EVEN
/// by construction: `set_value` rounds odd values up
/// (`ClearanceMatrix.java:104-113`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClearanceMatrix {
    /// Java `row[j].name` — the class names in matrix order.
    pub names: Vec<String>,
    /// Java `row[j].column[i].layer[layer]`.
    values: Vec<Vec<Vec<i32>>>,
    /// Java `Row.maxValue` as `row_max[j][layer]` — the per-row
    /// high-water marks `setValue` raises (`ClearanceMatrix.java:116`).
    row_max: Vec<Vec<i32>>,
    /// Java `maxValueOnLayer[layer]` (`:19`, raised at `:117`).
    max_on_layer: Vec<i32>,
}

impl ClearanceMatrix {
    /// Java `ClearanceMatrix(classCount, layerStructure, names)`
    /// (`:30-38`): `classCount = max(classCount, 1)`; one zeroed row of
    /// `classCount` columns per class; zeroed maxima per layer.
    #[must_use]
    pub fn new(class_count: usize, layer_count: usize, names: Vec<String>) -> Self {
        let class_count = class_count.max(1);
        Self {
            names,
            values: vec![vec![vec![0; class_count]; class_count]; layer_count],
            row_max: vec![vec![0; layer_count]; class_count],
            max_on_layer: vec![0; layer_count],
        }
    }

    /// Java `getClassCount()`.
    #[must_use]
    pub fn class_count(&self) -> usize {
        self.names.len().max(1)
    }

    /// Java `getLayerCount()` = the layer structure length.
    #[must_use]
    pub fn layer_count(&self) -> usize {
        self.values.len()
    }

    /// Java `getNo(name)` (`:58-65`): the FIRST case-insensitive name
    /// match, -1 on a miss.
    #[must_use]
    pub fn get_no(&self, name: &str) -> i32 {
        for (index, existing) in self.names.iter().enumerate() {
            if existing.to_lowercase() == name.to_lowercase() {
                return index as i32;
            }
        }
        -1
    }

    /// Java `getName(index)` (`:68-74`): `None` = Java's null + warn on
    /// an out-of-range index.
    #[must_use]
    pub fn get_name(&self, index: i32) -> Option<&str> {
        usize::try_from(index)
            .ok()
            .and_then(|index| self.names.get(index))
            .map(String::as_str)
    }

    /// Java `getValue(classI, classJ, layer, addSafetyMargin=false)`
    /// (`:131-201`): `values[layer][classJ][classI]` — the EXACT index
    /// order (T54). Out-of-bounds class or layer → 0 (Java logs a trace
    /// and returns 0, `:133-161`).
    #[must_use]
    pub fn get_value(&self, class_i: i32, class_j: i32, layer: i32) -> i32 {
        self.get_value_opt(class_i, class_j, layer, false)
    }

    /// Java `getValue(classI, classJ, layer, addSafetyMargin)` in full
    /// (`:131-201`). The safety margin is added to the IN-BOUNDS read
    /// only: the out-of-bounds branch (`:133-161`) early-returns 0
    /// BEFORE the margin add, so an out-of-bounds query stays 0, never
    /// [`CLEARANCE_SAFETY_MARGIN`] — a port folding the margin into the
    /// `unwrap_or(0)` chain would return 16 there.
    #[must_use]
    pub fn get_value_opt(
        &self,
        class_i: i32,
        class_j: i32,
        layer: i32,
        add_safety_margin: bool,
    ) -> i32 {
        let (Ok(i), Ok(j), Ok(l)) = (
            usize::try_from(class_i),
            usize::try_from(class_j),
            usize::try_from(layer),
        ) else {
            return 0;
        };
        self.values
            .get(l)
            .and_then(|per_layer| per_layer.get(j))
            .and_then(|row| row.get(i))
            .copied()
            .map(|value| {
                if add_safety_margin {
                    value + CLEARANCE_SAFETY_MARGIN
                } else {
                    value
                }
            })
            .unwrap_or(0)
    }

    /// Java `setValue(classI, classJ, layer, value)` (`:100-118`):
    /// writes the ONE cell `row[classJ].column[classI]` — NO transpose
    /// fill of `[j][i]` (module docs) — after clamping the value to
    /// `max(value, 0)` and rounding an ODD value UP to even
    /// (`Integer.MAX_VALUE` rounds DOWN, `:108-110`). Also raises the
    /// per-row and per-layer maxima (monotone `Math.max`).
    ///
    /// DIVERGENCE: a NEGATIVE or out-of-range class/layer index is a
    /// SILENT no-op here (the `let-else`/`get_mut` misses return),
    /// where Java's `values[layer][classJ][classI]` write would throw
    /// AIOOBE. Parse-unreachable — the DSN writer only emits indices
    /// inside the parsed matrix.
    pub fn set_value(&mut self, class_i: i32, class_j: i32, layer: i32, value: i32) {
        let (Ok(i), Ok(j), Ok(l)) = (
            usize::try_from(class_i),
            usize::try_from(class_j),
            usize::try_from(layer),
        ) else {
            return;
        };
        let mut value = value.max(0);
        if value % 2 != 0 {
            if value == i32::MAX {
                value -= 1;
            } else {
                value += 1;
            }
        }
        let Some(per_layer) = self.values.get_mut(l) else {
            return;
        };
        let Some(row) = per_layer.get_mut(j) else {
            return;
        };
        let Some(cell) = row.get_mut(i) else {
            return;
        };
        *cell = value;
        if let Some(row_max) = self.row_max.get_mut(j).and_then(|m| m.get_mut(l)) {
            *row_max = (*row_max).max(value);
        }
        if let Some(layer_max) = self.max_on_layer.get_mut(l) {
            *layer_max = (*layer_max).max(value);
        }
    }

    /// Java `setValue(classI, classJ, value)` (the all-layers overload,
    /// `:93-97`).
    pub fn set_value_all_layers(&mut self, class_i: i32, class_j: i32, value: i32) {
        for layer in 0..self.layer_count() as i32 {
            self.set_value(class_i, class_j, layer, value);
        }
    }

    /// Java `setDefaultValue(layer, value)` (`:84-90`): every class pair
    /// with BOTH numbers >= 1.
    pub fn set_default_value(&mut self, layer: i32, value: i32) {
        for i in 1..self.class_count() as i32 {
            for j in 1..self.class_count() as i32 {
                self.set_value(i, j, layer, value);
            }
        }
    }

    /// Java `setDefaultValue(value)` (`:77-81`): all layers.
    pub fn set_default_value_all_layers(&mut self, value: i32) {
        for layer in 0..self.layer_count() as i32 {
            self.set_default_value(layer, value);
        }
    }

    /// Java `appendClass(String)` (`:281-322`): appends a new class and
    /// initializes every new entry on EVERY layer from class 1 —
    /// `(new, i)` and `(i, new)` from `getValue(1, i, layer)` (class 0
    /// included), the diagonal `(new, new)` from `getValue(1, 1,
    /// layer)`. Returns false (no-op) when the name already exists.
    /// The old rows' accumulated maxima carry over (Java copies
    /// `currentOldRow.maxValue`); the new row starts at 0 and is
    /// raised by the init `setValue`s — the per-layer max only rises,
    /// exactly as in Java (`:299-321` never lowers `maxValue`).
    pub fn append_class(&mut self, name: &str) -> bool {
        if self.get_no(name) >= 0 {
            return false;
        }
        let old_class_count = self.names.len();
        self.names.push(name.to_string());
        let new_class_count = old_class_count + 1;
        let layer_count = self.values.len();
        for per_layer in &mut self.values {
            for row in per_layer.iter_mut() {
                row.resize(new_class_count, 0);
            }
            per_layer.resize(new_class_count, vec![0; new_class_count]);
        }
        self.row_max.resize(new_class_count, vec![0; layer_count]);
        let new_index = old_class_count as i32;
        for i in 0..old_class_count {
            for layer in 0..layer_count as i32 {
                let default_value = self.get_value(1, i as i32, layer);
                self.set_value(new_index, i as i32, layer, default_value);
                self.set_value(i as i32, new_index, layer, default_value);
            }
        }
        for layer in 0..layer_count as i32 {
            let default_value = self.get_value(1, 1, layer);
            self.set_value(new_index, new_index, layer, default_value);
        }
        true
    }

    /// Java `maxValue(int classI, int layer)` (`:207-213`): the per-row
    /// ACCUMULATED max — class and layer are clamped into range (Java
    /// `Math.max`/`Math.min` clamps, not bounds checks). "Row" here
    /// means `row[classI]`, i.e. the cells `values[layer][classI][*]`
    /// ever written — in an ASYMMETRIC matrix the row max and the
    /// column max differ (pinned in the tests).
    ///
    /// DIVERGENCE: on an EMPTY matrix (0 classes or 0 layers) the
    /// clamp range `0..=count-1` inverts and this PANICS (min > max);
    /// Java clamps to `-1` and throws AIOOBE at `elementAt(-1)`.
    /// Both are fatal; unreachable through the parse (every real
    /// board carries >= 2 layers and >= 1 class).
    #[must_use]
    pub fn max_value(&self, class: i32, layer: i32) -> i32 {
        let class = class.clamp(0, self.class_count() as i32 - 1);
        let layer = layer.clamp(0, self.layer_count() as i32 - 1);
        self.row_max
            .get(class as usize)
            .and_then(|per_layer| per_layer.get(layer as usize))
            .copied()
            .unwrap_or(0)
    }

    /// Java `maxValue(int layer)` (`:216-220`): the whole-layer max.
    #[must_use]
    pub fn max_value_on_layer(&self, layer: i32) -> i32 {
        let layer = layer.clamp(0, self.layer_count() as i32 - 1);
        self.max_on_layer.get(layer as usize).copied().unwrap_or(0)
    }

    /// Java `clearanceCompensationValue(clearanceClassIndex, layer)`
    /// (`:272-275`): `(getValue(c, c, layer, false) + 1) / 2` — INTEGER
    /// division. Diagonal values are always EVEN through the `setValue`
    /// path, so this halves them exactly; the +1-before-divide is
    /// load-bearing for an odd value (a raw-constructed matrix: 9 -> 5
    /// where 9/2 would give 4 — pinned in the tests).
    #[must_use]
    pub fn clearance_compensation_value(&self, class: i32, layer: i32) -> i32 {
        (self.get_value(class, class, layer) + 1) / 2
    }

    /// The IR conversion: copies the final values verbatim and
    /// RECONSTRUCTS the maxima from them (module docs — the IR cannot
    /// represent Java's overwrite-downward high-water corner).
    ///
    /// Precondition: `ir.values` is RECTANGULAR — every per-layer row
    /// jagged against `names.len()` (a ragged IR panics at the direct
    /// index). The epic-dsn parse always emits rectangular matrices.
    #[must_use]
    pub fn from_ir(ir: &ClearanceIr) -> Self {
        let class_count = ir.names.len();
        let layer_count = ir.values.len();
        let mut matrix = ClearanceMatrix::new(class_count, layer_count, ir.names.clone());
        for (layer, per_layer) in ir.values.iter().enumerate() {
            for (j, row) in per_layer.iter().enumerate() {
                for (i, &value) in row.iter().enumerate() {
                    // Direct write, NOT set_value: the IR values are the
                    // POST-rounding Java values — re-rounding would be a
                    // no-op for even values but a silent behavior change
                    // for a raw-odd test matrix.
                    matrix.values[layer][j][i] = value;
                    matrix.row_max[j][layer] = matrix.row_max[j][layer].max(value);
                    matrix.max_on_layer[layer] = matrix.max_on_layer[layer].max(value);
                }
            }
        }
        matrix
    }
}

/// Java `BoardRules.default_clearance_class` (`BoardRules.java:64-66`)
/// — always class 1. The tree-side consumers are the
/// `ShapeSearchTree` port: the compensated-tree class choice
/// (`SearchTreeManager.setClearanceCompensationUsed`, value ? 1 : 0)
/// and `drillHoleClearanceDelta`'s class-0-tree fallback
/// (`ShapeSearchTree.java:1058-1061`,
/// `clearanceClass = treeClass > 0 ? treeClass : defaultClearanceClass()`).
pub const DEFAULT_CLEARANCE_CLASS: i32 = 1;

/// Java `Nets.max_legal_net_number` (`Nets.java:16`).
pub const MAX_LEGAL_NET_NUMBER: i32 = 9_999_999;
/// Java `Nets.hidden_net_number` (`Nets.java:19`) — the auxiliary net
/// number for internal use.
pub const HIDDEN_NET_NUMBER: i32 = 10_000_001;

/// Java `Nets.isNormalNetNumber(netNumber)` (`Nets.java:31-34`).
#[must_use]
pub fn is_normal_net_number(net_number: i32) -> bool {
    net_number > 0 && net_number <= MAX_LEGAL_NET_NUMBER
}

/// Java `rules.Net` (the board-side read fields; the full Java class
/// carries back-pointers the read surface does not need).
#[derive(Clone, Debug, PartialEq)]
pub struct Net {
    /// Java `Net.name`.
    pub name: String,
    /// Java `Net.subnetNumber`.
    pub subnet_number: i32,
    /// Java `Net.containsPlane` — the plane-routing mode switch
    /// (stub+via routing for plane nets).
    pub contains_plane: bool,
    /// Java `Net.netClass` — the 0-based net-class index.
    pub net_class: i32,
}

/// Java `rules.Nets` — the net table. The net NUMBER is the 1-based
/// position (`Nets.java:88-96` `nets.size() + 1` at add time; the table
/// is append-only), carried by the Vec order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Nets {
    nets: Vec<Net>,
}

impl Nets {
    /// An empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Java `maxNetNumber()` (`:36-39`) = the table size.
    #[must_use]
    pub fn max_net_number(&self) -> i32 {
        self.nets.len() as i32
    }

    /// M7-T3: the net-table iteration face — `(net_number, net)` pairs
    /// in number order (1-based number = index + 1, Java 同构). The
    /// meander-need report assembly (route.rs `length_needs`) walks it.
    pub fn iter(&self) -> impl Iterator<Item = (i32, &Net)> {
        self.nets
            .iter()
            .enumerate()
            .map(|(index, net)| (index as i32 + 1, net))
    }

    /// Java `get(int netNumber)` (`:64-74`): the 1-based positional
    /// lookup, `None` outside `[1, size]`.
    #[must_use]
    pub fn get(&self, net_number: i32) -> Option<&Net> {
        if net_number < 1 || net_number > self.nets.len() as i32 {
            return None;
        }
        self.nets.get(net_number as usize - 1)
    }

    /// Java `get(String name, int subnetNumber)` (`:39-52`): the first
    /// CASE-INSENSITIVE name match with the given subnet number.
    #[must_use]
    pub fn get_by_name(&self, name: &str, subnet_number: i32) -> Option<&Net> {
        self.nets.iter().find(|net| {
            net.name.to_lowercase() == name.to_lowercase() && net.subnet_number == subnet_number
        })
    }

    /// The IR conversion: net number = position + 1 (table order IS the
    /// numbering).
    #[must_use]
    pub fn from_ir(nets: &[NetIr]) -> Self {
        Self {
            nets: nets
                .iter()
                .map(|net| Net {
                    name: net.name.clone(),
                    subnet_number: net.subnet_number,
                    contains_plane: net.contains_plane,
                    net_class: net.net_class,
                })
                .collect(),
        }
    }

    /// F2 (Rust-only, no Java counterpart — the current-driven width
    /// face): repoint a net's class membership — the plain field write
    /// for `Net.netClass` (a 0-based index into the board's
    /// net-class table; Java writes the same field at parse, the port
    /// writes it pre-route when a synthetic widened class is appended).
    /// A foreign net number is a quiet no-op (the `set_item_*` family
    /// convention).
    pub fn set_net_class(&mut self, net_number: i32, class_index: i32) {
        if net_number < 1 || net_number > self.nets.len() as i32 {
            return;
        }
        if let Some(net) = self.nets.get_mut(net_number as usize - 1) {
            net.net_class = class_index;
        }
    }

    /// F3 (Rust-only, no Java counterpart — the ground-pour ask):
    /// flip a net's `contains_plane` — the plain field write for
    /// `Net.containsPlane` (the parse sets it at plane insertion;
    /// the port writes it pre-route when a pour is synthesized). A
    /// foreign net number is a quiet no-op (the `set_item_*` family
    /// convention, and `set_net_class` above).
    pub fn set_contains_plane(&mut self, net_number: i32, value: bool) {
        if net_number < 1 || net_number > self.nets.len() as i32 {
            return;
        }
        if let Some(net) = self.nets.get_mut(net_number as usize - 1) {
            net.contains_plane = value;
        }
    }
}

/// Java `rules.NetClass` — the READ surface of the per-class routing
/// rules. Field-for-field mirror of the parse IR
/// (`epic_dsn::sink::NetClassIr`, which documents the Java anchors).
#[derive(Clone, Debug, PartialEq)]
pub struct NetClass {
    /// Java `NetClass.name`.
    pub name: String,
    /// Java `traceClearanceClass`.
    pub trace_clearance_class: i32,
    /// Java `traceHalfWidthArr` — one slot per 0-based layer.
    pub trace_half_widths: Vec<i32>,
    /// Java `activeRoutingLayerArr` — one slot per 0-based layer.
    pub active_routing_layers: Vec<bool>,
    /// Java `defaultItemClearanceClasses` — indexed by item-class
    /// declaration order (NONE, TRACE, VIA, PIN, SMD, AREA).
    pub default_item_clearance_classes: [i32; 6],
    /// Java `viaRule` — the identity id of the [`ViaRule`], `None` =
    /// Java null.
    pub via_rule: Option<u32>,
    /// Java `pullTight` (ctor true).
    pub pull_tight: bool,
    /// Java `shoveFixed` (ctor false).
    pub shove_fixed: bool,
    /// Java `minimumTraceLength`.
    pub min_trace_length: f64,
    /// Java `maximumTraceLength`.
    pub max_trace_length: f64,
    /// Java `ignoreCyclesWithAreas` (`NetClass.java:35`, ctor false).
    pub ignore_cycles_with_areas: bool,
}

impl NetClass {
    /// Java `getIgnoreCyclesWithAreas()` (`NetClass.java:143` — the
    /// `private boolean ignoreCyclesWithAreas` field at `:35`, default
    /// `false`). The DSN reader never writes the field, so every
    /// parsed board carries the ctor default (`NetClassIr` has no
    /// column for it) — the cycle walk
    /// (`crate::trace_ops::is_cycle`) consumes exactly this.
    #[must_use]
    pub fn ignore_cycles_with_areas(&self) -> bool {
        self.ignore_cycles_with_areas
    }

    /// Java `setIgnoreCyclesWithAreas(boolean)` (`NetClass.java:148`):
    /// the plain setter. Java's only production caller is the GUI
    /// (`WindowNetClasses.java:708`); the trace-split tests reach it
    /// the same way the SplitSpike did, to flip the flag before the
    /// split (capture `AR2_*`).
    pub fn set_ignore_cycles_with_areas(&mut self, value: bool) {
        self.ignore_cycles_with_areas = value;
    }

    /// Java `getTraceHalfWidth(layer)` (`NetClass.java:91-97`): 0 for an
    /// out-of-range layer — Java bounds-checks the read and returns 0
    /// with a warn log (it does not throw; unreachable from the parse,
    /// which sizes the array to the layer count).
    #[must_use]
    pub fn trace_half_width(&self, layer: i32) -> i32 {
        usize::try_from(layer)
            .ok()
            .and_then(|layer| self.trace_half_widths.get(layer))
            .copied()
            .unwrap_or(0)
    }

    /// Java `isActiveRoutingLayer(layer)` (`NetClass.java:184-189`).
    /// RESTORED at M3-T3 (M2 review cut the accessor; the parse kept the
    /// field): the first consumer is `AutorouteControl::init_net`
    /// (`AutorouteControl.java:226-228`), which force-disables a layer
    /// the net class marks inactive. Out-of-range layers read `false`:
    /// Java bounds-checks the array read (`:185-187`,
    /// `layerNumber < 0 || layerNumber >= length`) and RETURNS FALSE —
    /// it does not throw. The same convention is the verified one in
    /// the epic-dsn reader (`network.rs:1641-1644`); the in-range read
    /// passes the stored flag through.
    ///
    /// (An earlier doc draft claimed "Java's array read would throw;
    /// default `true`" — that inverted the arm. Caught in the M3-T3
    /// quality review; pinned by
    /// `active_routing_layer_out_of_range_reads_false` below.)
    ///
    /// M3-T3 cut-audit note: of the four M2-cut accessors, only this one
    /// and [`BoardRules::via_rule_by_id`] returned. `default_net_class`
    /// and `Component::is_placed` stay cut — the ported consumer paths
    /// do not call them: Java `initNet`'s null-net arm hardcodes the
    /// clearance-class fallback 1 and `viaRules.firstElement()`
    /// (`AutorouteControl.java:208-216`), never `getDefaultNetClass`,
    /// and `isPureSmdNet` (`:189-202`) tests each connectable item's
    /// kind + drill span, not component placement. Revisit both at the
    /// first real consumer, per the M2 carry-forward rule.
    #[must_use]
    pub fn is_active_routing_layer(&self, layer: i32) -> bool {
        usize::try_from(layer)
            .ok()
            .and_then(|layer| self.active_routing_layers.get(layer))
            .copied()
            .unwrap_or(false)
    }

    /// The IR conversion (1:1 — the IR is the verified field-for-field
    /// parse mirror).
    #[must_use]
    pub fn from_ir(class: &NetClassIr) -> Self {
        Self {
            name: class.name.clone(),
            trace_clearance_class: class.trace_clearance_class,
            trace_half_widths: class.trace_half_widths.clone(),
            active_routing_layers: class.active_routing_layers.clone(),
            default_item_clearance_classes: class.default_item_clearance_classes,
            via_rule: class.via_rule,
            pull_tight: class.pull_tight,
            shove_fixed: class.shove_fixed,
            min_trace_length: class.min_trace_length,
            max_trace_length: class.max_trace_length,
            // The ctor default (`NetClass.java:35`): the parse IR has
            // no column for the flag.
            ignore_cycles_with_areas: false,
        }
    }
}

/// Java `rules.ViaInfo` — one `(via ...)` rule of a net class.
#[derive(Clone, Debug, PartialEq)]
pub struct ViaInfo {
    /// Java `ViaInfo.name`.
    pub name: String,
    /// Java `ViaInfo.padstack` — the 1-based padstack number.
    pub padstack_no: i32,
    /// Java `clearanceClassIndex`.
    pub clearance_class: i32,
    /// Java `attachSmdAllowed`.
    pub attach_smd_allowed: bool,
}

impl ViaInfo {
    /// The IR conversion.
    #[must_use]
    pub fn from_ir(info: &ViaInfoIr) -> Self {
        Self {
            name: info.name.clone(),
            padstack_no: info.padstack_no,
            clearance_class: info.clearance_class,
            attach_smd_allowed: info.attach_smd_allowed,
        }
    }
}

/// Java `rules.ViaRule` — a named ordered list of via infos. Identity is
/// the `id`, NOT a table position (Java `addViaRule` removes and re-adds
/// the same-named rule, which would shift positions;
/// `epic_dsn::sink::ViaRuleIr` module docs).
#[derive(Clone, Debug, PartialEq)]
pub struct ViaRule {
    /// The identity (sink-assigned, monotonic from 1).
    pub id: u32,
    /// Java `ViaRule.name`.
    pub name: String,
    /// Java `viaInfos` — the 0-based indexes into the via-info table in
    /// rule order.
    pub via_infos: Vec<i32>,
}

impl ViaRule {
    /// The IR conversion.
    #[must_use]
    pub fn from_ir(rule: &ViaRuleIr) -> Self {
        Self {
            id: rule.id,
            name: rule.name.clone(),
            via_infos: rule.via_infos.clone(),
        }
    }
}

/// Java `rules.BoardRules` — the container the board holds as
/// `board.rules`. Built from [`BoardRulesIr`] plus the net/class/via
/// tables of the parse IR (`SesBoard`).
#[derive(Clone, Debug, PartialEq)]
pub struct BoardRules {
    /// Java `clearanceMatrix` (final field).
    pub clearance: ClearanceMatrix,
    /// Java `nets`.
    pub nets: Nets,
    /// Java `netClasses` — class 0 is the default class.
    pub net_classes: Vec<NetClass>,
    /// Java `viaInfos`.
    pub via_infos: Vec<ViaInfo>,
    /// Java `viaRules`.
    pub via_rules: Vec<ViaRule>,
    /// Java `traceAngleRestriction` — the ORDINAL-LOAD-BEARING enum
    /// (T59).
    pub trace_angle_restriction: AngleRestriction,
    /// Java `getDefaultNetClass().getTraceHalfWidth(layer)` per layer.
    pub default_trace_half_widths: Vec<i32>,
    /// Java `minTraceHalfWidth` (`BoardRules.java:37`).
    pub min_trace_half_width: i32,
    /// Java `maxTraceHalfWidth` (`:40`).
    pub max_trace_half_width: i32,
    /// Java `holeClearance` (`:49`, ctor 0, `setHoleClearance` clamps
    /// to `max(0, value)` at `:105-110`). NEVER set by the DSN parse —
    /// only the `HeadlessBoardManager` settings path writes it
    /// (ShapeSearchTree.java:1039 reads it; T55).
    pub hole_clearance: i32,
    /// Java `pinEdgeToTurnDist` (`:46`).
    pub pin_edge_to_turn_dist: f64,
    /// Java the default class's `defaultItemClearanceClasses`.
    pub default_item_clearance_classes: [i32; 6],
    /// Java `clearanceToleranceUm` (#925a, upstream 14b28b6ff): the
    /// clearance-violation shortfall tolerance in micrometres —
    /// shortfalls ≤ this are floating-point discretization /
    /// imperial-to-metric rounding noise, not electrical violations
    /// (the DRC measure loop drops them, STRICT `>` at the boundary).
    /// Seeded 1.0 at BOTH construction faces (the ctor re-seed in
    /// Java is the transient-deserialization twin); only the
    /// settings override writes it (epic_engine::drc_tolerance).
    pub clearance_tolerance_um: f64,
}

impl BoardRules {
    /// An empty rules surface (no layers, no classes, no nets) — the
    /// `Board::new()` default; a real board always arrives via
    /// [`BoardRules::from_ir`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Java `getNetClass(no)` equivalent — the 0-based class index.
    #[must_use]
    pub fn net_class(&self, index: i32) -> Option<&NetClass> {
        usize::try_from(index)
            .ok()
            .and_then(|i| self.net_classes.get(i))
    }

    /// Java `getTraceHalfWidth(netNumber, layer)` (`BoardRules.java:74-77`):
    /// the NET's class half width — `nets.get(netNumber).getNetClass()`.
    /// RESTORED at M3-T3 (consumed by `AutorouteControl::init_net`,
    /// `AutorouteControl.java:218-222`, including the
    /// `netNumber > 0 ? netNumber : 1` fallback the caller applies —
    /// the fallback is OBSERVABLE and pinned there). Java NPEs when the
    /// net is absent; every caller guards (`Nets::get` returns `None`
    /// outside `[1, size]`), so the Rust read is 0 — the same answer
    /// [`NetClass::trace_half_width`] gives out-of-range.
    #[must_use]
    pub fn trace_half_width(&self, net_number: i32, layer: i32) -> i32 {
        self.nets
            .get(net_number)
            .and_then(|net| self.net_class(net.net_class))
            .map_or(0, |class| class.trace_half_width(layer))
    }

    /// Java has no named lookup — `NetClass.getViaRule()` returns the
    /// rule OBJECT (`NetClass.java:124-130`, a live reference into
    /// `BoardRules.viaRules`). The Rust class stores the rule's
    /// identity id ([`NetClass::via_rule`]), so this is the id → rule
    /// resolution. RESTORED at M3-T3 (M2 review cut; consumed by
    /// `AutorouteControl::init_net`, `AutorouteControl.java:211` and
    /// the `:214` first-element fallback — which the CALLER applies,
    /// via `via_rules.first()`, so this lookup stays `None`-honest).
    #[must_use]
    pub fn via_rule_by_id(&self, id: u32) -> Option<&ViaRule> {
        self.via_rules.iter().find(|rule| rule.id == id)
    }

    /// Java `setHoleClearance(value)` (`:109-111`): `max(0, value)`.
    pub fn set_hole_clearance(&mut self, value: i32) {
        self.hole_clearance = value.max(0);
    }

    /// M7-T2: the per-net resolved length bounds — the net's class
    /// `getMinimumTraceLength()`/`getMaximumTraceLength()` pair
    /// (`NetClass.java` min/max fields, delivered through the
    /// `> 0` gates of `Network.insertNetClasses` `:466-471`, ported at
    /// `epic-dsn/src/scope/network.rs:1009-1015`). The house-idiomatic
    /// read, on the [`Self::trace_half_width`] pattern: `nets.get`
    /// (the 1-based number) then the net's 0-based class index.
    ///
    /// **`0.0` = no constraint** on either slot (T1's delivery-gate
    /// fact: an absent declaration or the `-1`/`0` write-side
    /// sentinels never survive delivery — the field stays 0.0
    /// downstream).
    ///
    /// CLASS INHERITANCE AUDIT (M7-T2): there is none to walk. Java
    /// `Net.getNetClass()` is a direct object reference — one class
    /// per net, no class hierarchy, no per-net overrides of the class
    /// bounds (`rules/Net.java`/`rules/NetClass.java` carry no
    /// length-inheritance face; the default class is just class 0 in
    /// the same flat table). The resolution is exactly net → its
    /// class → that class's own fields. An ABSENT net or an
    /// out-of-range class index resolves `(0.0, 0.0)` — the same
    /// Java-null-shy answer [`Self::trace_half_width`] gives (callers
    /// guard; the read never panics).
    #[must_use]
    pub fn net_class_length_bounds(&self, net_number: i32) -> (f64, f64) {
        self.nets
            .get(net_number)
            .and_then(|net| self.net_class(net.net_class))
            .map_or((0.0, 0.0), |class| {
                (class.min_trace_length, class.max_trace_length)
            })
    }

    /// M7-T2: the constraint-ACTIVATION predicate — the board-level
    /// "tuning-active" check. TRUE when ANY net resolves a non-zero
    /// length bound (`> 0.0` on either slot; 0.0 = no constraint per
    /// T1's delivery gate — negative write-side sentinels never
    /// survive delivery, and the `> 0.0` test keeps even a hand-built
    /// negative from counting, so the predicate is total over the raw
    /// field domain). Input-driven by construction: no declaration ⇒
    /// false ⇒ the tuning regime never fires (the constraint-free
    /// zero-rotation face; pinned in the tests).
    #[must_use]
    pub fn has_length_constraints(&self) -> bool {
        self.nets.nets.iter().any(|net| {
            self.net_class(net.net_class)
                .is_some_and(|class| class.min_trace_length > 0.0 || class.max_trace_length > 0.0)
        })
    }

    /// M7-T3: the `calcLengthViolation` predicate port (the T1-Q3
    /// adoption, NARROW — the pure predicate for the honest-report face,
    /// NOT Java's DRC-report machinery: no markers, no
    /// `lengthViolation` field, no `|new - old| > 0.1` change tracking —
    /// the port is stateless). Java `drc/NetIncompletes.
    /// calcLengthViolation` (`NetIncompletes.java:257-271`), verbatim:
    /// both bounds absent (`<= 0`) ⇒ 0; `max > 0 && length > max` ⇒
    /// the over-max EXCESS (positive, too long); `min > 0 && length <
    /// min && no incompletes` ⇒ the under-min DEFICIT (negative, too
    /// short; a net with incompletes is still being routed — Java's
    /// gate). The ifs SEQUENCE (the second overwrites, never
    /// else-ifs) — the Java-exact malformed-declaration face where
    /// `min > max`. Positive = over-max excess, negative = under-min
    /// deficit, `0.0` = valid. The net's routed length is a parameter
    /// (the caller walks the board via [`Board::net_trace_length`]).
    #[must_use]
    pub fn length_violation(
        &self,
        net_number: i32,
        net_trace_length: f64,
        net_has_incompletes: bool,
    ) -> f64 {
        let (min, max) = self.net_class_length_bounds(net_number);
        if max <= 0.0 && min <= 0.0 {
            return 0.0;
        }
        let mut new_violation = 0.0;
        if max > 0.0 && net_trace_length > max {
            new_violation = net_trace_length - max;
        }
        if min > 0.0 && net_trace_length < min && !net_has_incompletes {
            new_violation = net_trace_length - min;
        }
        new_violation
    }

    /// The IR conversion — [`ClearanceMatrix::from_ir`] for the matrix,
    /// 1:1 field copies for everything else. The net/class/via tables
    /// come from the `SesBoard` fields (they live beside `rules` there).
    #[must_use]
    pub fn from_ir(
        rules: &BoardRulesIr,
        nets: &[NetIr],
        net_classes: &[NetClassIr],
        via_infos: &[ViaInfoIr],
        via_rules: &[ViaRuleIr],
    ) -> Self {
        Self {
            clearance: ClearanceMatrix::from_ir(&rules.clearance),
            nets: Nets::from_ir(nets),
            net_classes: net_classes.iter().map(NetClass::from_ir).collect(),
            via_infos: via_infos.iter().map(ViaInfo::from_ir).collect(),
            via_rules: via_rules.iter().map(ViaRule::from_ir).collect(),
            trace_angle_restriction: AngleRestriction::from_ir(rules.trace_angle_restriction),
            default_trace_half_widths: rules.default_trace_half_widths.clone(),
            min_trace_half_width: rules.min_trace_half_width,
            max_trace_half_width: rules.max_trace_half_width,
            hole_clearance: 0,
            pin_edge_to_turn_dist: rules.pin_edge_to_turn_dist,
            default_item_clearance_classes: rules.default_item_clearance_classes,
            clearance_tolerance_um: 1.0,
        }
    }
}

impl Default for BoardRules {
    fn default() -> Self {
        Self {
            clearance: ClearanceMatrix::default(),
            nets: Nets::new(),
            net_classes: Vec::new(),
            via_infos: Vec::new(),
            via_rules: Vec::new(),
            trace_angle_restriction: AngleRestriction::default(),
            default_trace_half_widths: Vec::new(),
            min_trace_half_width: 0,
            max_trace_half_width: 0,
            hole_clearance: 0,
            pin_edge_to_turn_dist: 0.0,
            default_item_clearance_classes: [0; 6],
            clearance_tolerance_um: 1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 3-class, 2-layer matrix builder (class names "null", "default",
    /// "power").
    fn matrix_3x2() -> ClearanceMatrix {
        ClearanceMatrix::new(
            3,
            2,
            vec![
                "null".to_string(),
                "default".to_string(),
                "power".to_string(),
            ],
        )
    }

    /// `ClearanceMatrix.java:163` + `:100-118` (T54): `getValue(i, j)`
    /// reads `values[layer][j][i]` and `setValue(i, j)` writes the ONE
    /// cell `row[j].column[i]` with NO transpose fill. Pin an
    /// ASYMMETRIC matrix where both directions differ, cross-checked
    /// against the epic-dsn IR (`ClearanceIr::get_value`, the verified
    /// M1b reference) driven through the identical `set_value` sequence:
    /// a port that swapped `i`/`j` in EITHER get or set fails the
    /// direction asserts (and the IR cross-check).
    #[test]
    fn get_value_index_order_is_j_row_i_column_asymmetric() {
        let mut matrix = matrix_3x2();
        // set_value(class_i=1, class_j=2, layer=0, 400): row 2, column 1.
        matrix.set_value(1, 2, 0, 400);
        // set_value(class_i=2, class_j=1, layer=0, 200): row 1, column 2.
        matrix.set_value(2, 1, 0, 200);
        assert_eq!(matrix.get_value(1, 2, 0), 400, "row[2].column[1]");
        assert_eq!(matrix.get_value(2, 1, 0), 200, "row[1].column[2]");
        assert_ne!(
            matrix.get_value(1, 2, 0),
            matrix.get_value(2, 1, 0),
            "the matrix IS asymmetric — a transposed port cannot pass"
        );
        // The untouched symmetric cells stay 0.
        assert_eq!(matrix.get_value(1, 1, 0), 0);
        assert_eq!(matrix.get_value(0, 2, 0), 0);

        // Cross-check the SAME sequence against the verified IR
        // reference (epic-dsn `ClearanceIr`, M1b-pinned get_value).
        let mut ir = ClearanceIr::default_instance(2);
        ir.append_class("power");
        ir.set_value(1, 2, 0, 400);
        ir.set_value(2, 1, 0, 200);
        assert_eq!(matrix.get_value(1, 2, 0), ir.get_value(1, 2, 0));
        assert_eq!(matrix.get_value(2, 1, 0), ir.get_value(2, 1, 0));
        // And the from_ir conversion lands the same cells.
        let converted = ClearanceMatrix::from_ir(&ir);
        assert_eq!(converted.get_value(1, 2, 0), 400);
        assert_eq!(converted.get_value(2, 1, 0), 200);
    }

    /// `ClearanceMatrix.java:104-113`: values are clamped to >= 0 and
    /// ODD values round UP to even (never down) before storage.
    #[test]
    fn set_value_clamps_negative_and_rounds_odd_up() {
        let mut matrix = matrix_3x2();
        matrix.set_value(1, 1, 0, 201);
        assert_eq!(matrix.get_value(1, 1, 0), 202, "odd rounds UP");
        matrix.set_value(1, 2, 0, -50);
        assert_eq!(matrix.get_value(1, 2, 0), 0, "clamped to 0 (already even)");
        matrix.set_value(2, 1, 0, -3);
        assert_eq!(matrix.get_value(2, 1, 0), 0, "-3 -> max(0) -> 0");
        matrix.set_value(1, 1, 1, 200);
        assert_eq!(matrix.get_value(1, 1, 1), 200, "even stays");
    }

    /// `ClearanceMatrix.java:272-275` (T54): the compensation is
    /// `(value + 1) / 2` with INTEGER division. Through `set_value` the
    /// diagonal is always EVEN (8 -> 4, 10 -> 5 — the +1 never crosses a
    /// rounding boundary); the ODD case (9 -> 5, where plain `value / 2`
    /// would give 4) is reachable only through a raw-constructed IR, so
    /// it is pinned on a hand-built `ClearanceIr` converted verbatim.
    #[test]
    fn compensation_value_is_value_plus_one_over_two_odd_and_even() {
        let mut matrix = matrix_3x2();
        matrix.set_value(1, 1, 0, 8);
        matrix.set_value(2, 2, 0, 10);
        assert_eq!(matrix.clearance_compensation_value(1, 0), 4, "(8+1)/2 = 4");
        assert_eq!(matrix.clearance_compensation_value(2, 0), 5, "(10+1)/2 = 5");

        // Raw odd diagonal (parse-path values are always even — the odd
        // form exists to discriminate the formula from value/2). The IR
        // is grown to a 3x3x1 matrix by hand: extend the EXISTING rows
        // first, then append the new row (appending first would leave a
        // ragged matrix).
        let mut ir = ClearanceIr::default_instance(1);
        for per_layer in &mut ir.values {
            for row in per_layer.iter_mut() {
                row.push(0);
            }
            per_layer.push(vec![0; 3]);
        }
        ir.names.push("odd9".to_string());
        ir.values[0][2][2] = 9;
        let raw = ClearanceMatrix::from_ir(&ir);
        assert_eq!(
            raw.get_value(2, 2, 0),
            9,
            "raw odd value preserved verbatim"
        );
        assert_eq!(
            raw.clearance_compensation_value(2, 0),
            5,
            "(9+1)/2 = 5 — value/2 would give 4"
        );
    }

    /// `ClearanceMatrix.java:131-201` + `:17`: the safety margin lands on
    /// the IN-BOUNDS read only. Captured `MARGIN` lines from the Task 6
    /// jar spike (TreeShapesSpike.java against the frozen jar):
    /// Issue575 `/tmp/epic-t6-treeshapes.out` — `v1.1.l0 no=2000
    /// with=2016`; 1Bitsy `/tmp/epic-t6-treeshapes-1bitsy.out` —
    /// `no=1490 with=1506`. The out-of-bounds forms pin the
    /// early-return-BEFORE-margin order: an implementation that folds
    /// the margin into the bounds-check fallback returns 16, not 0.
    #[test]
    fn safety_margin_adds_only_to_in_bounds_reads() {
        let mut matrix = matrix_3x2();
        matrix.set_value(1, 1, 0, 2000); // Issue575 v1.1
        matrix.set_value(1, 1, 1, 1490); // 1Bitsy v1.1
        assert_eq!(matrix.get_value(1, 1, 0), 2000, "no=2000");
        assert_eq!(matrix.get_value_opt(1, 1, 0, true), 2016, "with=2016");
        assert_eq!(matrix.get_value_opt(1, 1, 1, true), 1506, "with=1506");
        // Out of bounds: class, then layer — 0 both with and without the
        // margin (the `:133-161` early return precedes the `:163-165`
        // margin add).
        assert_eq!(matrix.get_value_opt(4, 1, 0, true), 0, "class OOB stays 0");
        assert_eq!(matrix.get_value_opt(1, -1, 0, true), 0, "negative class 0");
        assert_eq!(matrix.get_value_opt(1, 1, 9, true), 0, "layer OOB stays 0");
    }

    /// `ClearanceMatrix.java:207-213` + `:116`: `maxValue(class, layer)`
    /// is the per-ROW high-water accumulator — in the asymmetric matrix
    /// of the T54 pin, row 2's cells are {0, 400, 0} (max 400) while
    /// row 1's are {0, 0, 200} (max 200): a port that maxes the COLUMN,
    /// or the whole layer, for every class fails this. Out-of-range
    /// class/layer CLAMP into range (Java Math.min/max), and the marks
    /// never shrink when a cell is overwritten downward.
    #[test]
    fn max_value_is_the_per_row_accumulator() {
        let mut matrix = matrix_3x2();
        matrix.set_value(1, 2, 0, 400);
        matrix.set_value(2, 1, 0, 200);
        assert_eq!(matrix.max_value(2, 0), 400, "row 2 holds the 400 cell");
        assert_eq!(matrix.max_value(1, 0), 200, "row 1 holds the 200 cell");
        assert_eq!(matrix.max_value(0, 0), 0, "row 0 untouched");
        // clamps: class 99 / -5 both clamp; layer 7 clamps to 1 (the
        // last layer), where row 2 holds 300 — a discriminating clamp
        // pin (returning 0 for out-of-range layers would fail it).
        matrix.set_value(2, 2, 1, 300);
        assert_eq!(matrix.max_value(99, 0), 400);
        assert_eq!(matrix.max_value(-5, 0), 0);
        assert_eq!(matrix.max_value(2, 7), 300, "layer clamps to 1");
        // whole-layer max: 400 on layer 0, 300 on layer 1.
        assert_eq!(matrix.max_value_on_layer(0), 400);
        assert_eq!(matrix.max_value_on_layer(1), 300);
        // The accumulator never SHRINKS on a downward overwrite.
        matrix.set_value(1, 2, 0, 40);
        assert_eq!(matrix.get_value(1, 2, 0), 40, "the cell went down");
        assert_eq!(matrix.max_value(2, 0), 400, "the row mark stays at 400");
    }

    /// `ClearanceMatrix.java:58-65`: first case-INSENSITIVE name match,
    /// -1 on a miss.
    #[test]
    fn get_no_is_case_insensitive_first_match() {
        let matrix = matrix_3x2();
        assert_eq!(matrix.get_no("default"), 1);
        assert_eq!(matrix.get_no("DEFAULT"), 1);
        assert_eq!(matrix.get_no("Null"), 0);
        assert_eq!(matrix.get_no("missing"), -1);
        assert_eq!(matrix.get_name(2), Some("power"));
        assert_eq!(matrix.get_name(3), None);
    }

    /// `Nets.java:16,19,27-29`: the net-number constants and the normal
    /// range (0 and below are not normal; hidden is above the max).
    #[test]
    fn net_number_constants_and_normal_range() {
        assert_eq!(MAX_LEGAL_NET_NUMBER, 9_999_999);
        assert_eq!(HIDDEN_NET_NUMBER, 10_000_001);
        assert!(is_normal_net_number(1));
        assert!(is_normal_net_number(MAX_LEGAL_NET_NUMBER));
        assert!(!is_normal_net_number(0));
        assert!(!is_normal_net_number(-3));
        assert!(!is_normal_net_number(MAX_LEGAL_NET_NUMBER + 1));
        assert!(!is_normal_net_number(HIDDEN_NET_NUMBER));
    }

    /// `Nets.java:39-74`: 1-based positional get + case-insensitive
    /// name/subnet lookup; `maxNetNumber` = table size.
    #[test]
    fn nets_table_lookups() {
        let nets = Nets::from_ir(&[
            NetIr {
                name: "GND".to_string(),
                subnet_number: 1,
                contains_plane: true,
                net_class: 0,
            },
            NetIr {
                name: "VCC".to_string(),
                subnet_number: 1,
                contains_plane: false,
                net_class: 1,
            },
        ]);
        assert_eq!(nets.max_net_number(), 2);
        assert_eq!(nets.get(1).expect("net 1").name, "GND");
        assert_eq!(nets.get(2).expect("net 2").name, "VCC");
        assert!(nets.get(3).is_none());
        assert!(nets.get(0).is_none());
        assert_eq!(
            nets.get_by_name("gnd", 1).expect("case-insensitive").name,
            "GND"
        );
        assert!(nets.get_by_name("GND", 2).is_none(), "subnet mismatch");
        assert!(nets.get_by_name("missing", 1).is_none());
        assert!(nets.get(1).expect("plane flag").contains_plane);
    }

    /// F2: [`Nets::set_net_class`] — the 1-based positional class
    /// repoint (the plain `Net.netClass` write the synthetic widened
    /// class needs): the field flips for a live number, and a foreign
    /// number (0, past the end, negative) is a quiet no-op.
    #[test]
    fn set_net_class_repoints_and_ignores_foreign_numbers() {
        let mut nets = Nets::from_ir(&[
            NetIr {
                name: "GND".to_string(),
                subnet_number: 1,
                contains_plane: false,
                net_class: 0,
            },
            NetIr {
                name: "VCC".to_string(),
                subnet_number: 1,
                contains_plane: false,
                net_class: 0,
            },
        ]);
        nets.set_net_class(2, 5);
        assert_eq!(nets.get(2).expect("net 2").net_class, 5, "repointed");
        assert_eq!(nets.get(1).expect("net 1").net_class, 0, "untouched");
        nets.set_net_class(0, 9);
        nets.set_net_class(3, 9);
        nets.set_net_class(-1, 9);
        assert_eq!(nets.get(1).expect("net 1").net_class, 0, "no-ops");
        assert_eq!(nets.get(2).expect("net 2").net_class, 5, "no-ops");
    }

    /// F3: [`Nets::set_contains_plane`] — the plane-flag write the
    /// synthesized pour needs (the parse's own `add_plane_net` sets
    /// the same field): the flag flips for a live number, and a
    /// foreign number (0, past the end, negative) is a quiet no-op.
    #[test]
    fn set_contains_plane_flips_and_ignores_foreign_numbers() {
        let mut nets = Nets::from_ir(&[
            NetIr {
                name: "GND".to_string(),
                subnet_number: 1,
                contains_plane: false,
                net_class: 0,
            },
            NetIr {
                name: "VCC".to_string(),
                subnet_number: 1,
                contains_plane: false,
                net_class: 0,
            },
        ]);
        nets.set_contains_plane(1, true);
        assert!(
            nets.get(1).expect("net 1").contains_plane,
            "the synthesized-pour flag"
        );
        assert!(!nets.get(2).expect("net 2").contains_plane, "untouched");
        nets.set_contains_plane(0, true);
        nets.set_contains_plane(3, true);
        nets.set_contains_plane(-1, true);
        assert!(!nets.get(2).expect("net 2").contains_plane, "no-ops");
        // And back — the flag is a plain write, not a one-way latch.
        nets.set_contains_plane(1, false);
        assert!(
            !nets.get(1).expect("net 1").contains_plane,
            "flips both ways"
        );
    }

    /// #925a (upstream `14b28b6ff`): the tolerance seeds 1.0 at BOTH
    /// construction faces — `Default` (the `Board::new()` face) and
    /// `from_ir` (every parsed board; the epic-drc parse pin covers
    /// that arm end-to-end) — so the DRC measure loop's default drops
    /// sub-µm rounding noise everywhere, and only a settings override
    /// (`epic_engine::drc_tolerance`) ever writes another value.
    #[test]
    fn clearance_tolerance_um_seeds_one_at_both_construction_faces() {
        assert_eq!(
            BoardRules::default().clearance_tolerance_um,
            1.0,
            "the Default seed"
        );
        assert_eq!(BoardRules::new().clearance_tolerance_um, 1.0);
    }

    /// `AngleRestriction.java`: the ordinal round-trip — `valueOf(i)` /
    /// `getValue()` (T59: the ordinal is load-bearing downstream).
    #[test]
    fn angle_restriction_ordinals_round_trip() {
        assert_eq!(AngleRestriction::value_of(0), AngleRestriction::None);
        assert_eq!(
            AngleRestriction::value_of(1),
            AngleRestriction::FortyfiveDegree
        );
        assert_eq!(
            AngleRestriction::value_of(2),
            AngleRestriction::NinetyDegree
        );
        assert_eq!(AngleRestriction::None.get_value(), 0);
        assert_eq!(AngleRestriction::FortyfiveDegree.get_value(), 1);
        assert_eq!(AngleRestriction::NinetyDegree.get_value(), 2);
        // the IR conversion keeps the order
        assert_eq!(
            AngleRestriction::from_ir(epic_dsn::state::AngleRestriction::FortyfiveDegree),
            AngleRestriction::FortyfiveDegree
        );
        assert_eq!(
            AngleRestriction::from_ir(epic_dsn::state::AngleRestriction::NinetyDegree),
            AngleRestriction::NinetyDegree
        );
    }

    /// `NetClass.java:184-189` (M3-T3 quality round): the out-of-range
    /// arms of `isActiveRoutingLayer` are a bounds-check that RETURNS
    /// FALSE (`:185-187`), not an array throw — an earlier port draft
    /// defaulted them to `true` (the inverted arm). All four arms are
    /// pinned: both in-range entries pass through, and BOTH out-of-range
    /// directions (`layer == layer_count`, negative) read `false`. The
    /// false rows kill the `unwrap_or(true)` flip mutant; the in-range
    /// contrast proves the bounds-check did not swallow real entries.
    #[test]
    fn active_routing_layer_out_of_range_reads_false() {
        let class = NetClass {
            name: "probe".to_string(),
            trace_clearance_class: 1,
            trace_half_widths: vec![100, 200],
            active_routing_layers: vec![true, false],
            default_item_clearance_classes: [0, 1, 1, 1, 1, 1],
            via_rule: None,
            pull_tight: true,
            shove_fixed: false,
            min_trace_length: 0.0,
            max_trace_length: 0.0,
            ignore_cycles_with_areas: false,
        };
        assert!(class.is_active_routing_layer(0), "in-range true entry");
        assert!(!class.is_active_routing_layer(1), "in-range false entry");
        assert!(
            !class.is_active_routing_layer(2),
            "layer_index == layer_count -> false (NetClass.java:185-187)"
        );
        assert!(
            !class.is_active_routing_layer(-1),
            "negative layer_index -> false (same guard)"
        );
    }
    /// Java `appendClass` (`:281-322`) on the LIVE matrix: the new
    /// row/col initializes from class 1 per layer (`(new, i)` and
    /// `(i, new)` from `getValue(1, i)`, class 0 INCLUDED — the
    /// java-verified `/tmp/epic-t5-probe.out` face; the diagonal from
    /// `getValue(1, 1)`); a second append with the same name is a
    /// false-return no-op; the maxima never go DOWN.
    #[test]
    fn append_class_initializes_from_class_1_and_dedups() {
        let mut matrix = matrix_3x2();
        // Asymmetric seed: v(1,0)=200 raw even, v(1,2)=400; diagonal
        // v(1,1)=2000 (setValue rounds odd up — 199 -> 200).
        matrix.set_value(1, 0, 0, 200);
        matrix.set_value(0, 1, 0, 200);
        matrix.set_value(1, 2, 0, 400);
        matrix.set_value(2, 1, 0, 400);
        matrix.set_value(1, 1, 0, 2000);
        matrix.set_value(1, 1, 1, 1200);

        assert!(matrix.append_class("board_edge"), "first append");
        let edge = matrix.get_no("board_edge");
        assert_eq!(edge, 3, "appended at the tail");
        assert_eq!(matrix.class_count(), 4);
        // (new, i) and (i, new) from class 1, ALL layers — class 0
        // included (200), class 2 (400); the diagonal from v(1,1).
        assert_eq!(matrix.get_value(edge, 0, 0), 200);
        assert_eq!(matrix.get_value(0, edge, 0), 200);
        assert_eq!(matrix.get_value(edge, 2, 0), 400);
        assert_eq!(matrix.get_value(2, edge, 0), 400);
        assert_eq!(matrix.get_value(edge, edge, 0), 2000);
        assert_eq!(matrix.get_value(edge, edge, 1), 1200);
        // The append did not disturb the seeded cells.
        assert_eq!(matrix.get_value(1, 2, 0), 400);
        assert_eq!(matrix.get_value(1, 0, 0), 200);
        // The new row's accumulated max rises with the init writes.
        assert_eq!(
            matrix.max_value(edge, 0),
            2000,
            "row max from the diagonal init"
        );
        // A second append with the same name: false, no growth.
        assert!(!matrix.append_class("BOARD_EDGE"), "case-insensitive dedup");
        assert_eq!(matrix.class_count(), 4);
    }

    // ------------------------------------------------------------------
    // M7-T2: the resolved length-constraint query surface
    // ------------------------------------------------------------------

    /// The M7-T2 world: class 0 the unconstrained default, class 1
    /// "constrained" carrying `min 10.0 / max 20.0`, two nets (1 →
    /// class 0, 2 → class 1). Built as `BoardRules::default()` + direct
    /// field pushes (the same-crate test convention; `Nets::from_ir`
    /// for the table).
    fn length_bounds_world() -> BoardRules {
        let mut rules = BoardRules::default();
        rules.net_classes.push(NetClass {
            name: "default".to_string(),
            trace_clearance_class: 1,
            trace_half_widths: vec![1500],
            active_routing_layers: vec![true],
            default_item_clearance_classes: [0, 1, 1, 1, 1, 1],
            via_rule: None,
            pull_tight: true,
            shove_fixed: false,
            min_trace_length: 0.0,
            max_trace_length: 0.0,
            ignore_cycles_with_areas: false,
        });
        rules.net_classes.push(NetClass {
            name: "constrained".to_string(),
            trace_clearance_class: 1,
            trace_half_widths: vec![1500],
            active_routing_layers: vec![true],
            default_item_clearance_classes: [0, 1, 1, 1, 1, 1],
            via_rule: None,
            pull_tight: true,
            shove_fixed: false,
            min_trace_length: 10.0,
            max_trace_length: 20.0,
            ignore_cycles_with_areas: false,
        });
        rules.nets = Nets::from_ir(&[
            NetIr {
                name: "plain".to_string(),
                subnet_number: 1,
                contains_plane: false,
                net_class: 0,
            },
            NetIr {
                name: "matched".to_string(),
                subnet_number: 1,
                contains_plane: false,
                net_class: 1,
            },
        ]);
        rules
    }

    /// Declared + the class-indirection face + the absent faces. The
    /// declared net resolves its OWN class's bounds verbatim; a net in
    /// the unconstrained default class and every out-of-table number
    /// resolve `(0.0, 0.0)` (0.0 = no constraint, T1's delivery-gate
    /// semantics).
    #[test]
    fn net_class_length_bounds_declared_and_absent() {
        let rules = length_bounds_world();
        assert_eq!(
            rules.net_class_length_bounds(2),
            (10.0, 20.0),
            "the declared net resolves its class's bounds verbatim"
        );
        assert_eq!(
            rules.net_class_length_bounds(1),
            (0.0, 0.0),
            "a net in the unconstrained default class resolves no bound"
        );
        assert_eq!(
            rules.net_class_length_bounds(0),
            (0.0, 0.0),
            "out-of-table net number 0: Java-null-shy, no constraint"
        );
        assert_eq!(
            rules.net_class_length_bounds(99),
            (0.0, 0.0),
            "out-of-table net number 99: no constraint"
        );
    }

    /// The DIVERGENCE-discriminating pin for the class indirection: a
    /// port that resolved the DEFAULT class for every net fails — the
    /// declared net's class 1 bounds differ from class 0's in both
    /// slots. (There is no class HIERARCHY to inherit through — the fn
    /// docs — so the pin names the net→class indirection the only
    /// inheritance face.)
    #[test]
    fn net_class_length_bounds_follow_the_nets_own_class() {
        let rules = length_bounds_world();
        let (min, max) = rules.net_class_length_bounds(2);
        assert_ne!(
            (min, max),
            rules.net_class_length_bounds(1),
            "the net's own class answers, not the default class"
        );
        assert_eq!(min, 10.0);
        assert_eq!(max, 20.0);
    }

    /// The predicate: TRUE exactly when a resolvable `> 0.0` bound
    /// exists; the zero-rotation negative face (no declaration ⇒
    /// false) plus the sentinel/negative defensive rows.
    #[test]
    fn has_length_constraints_gates_on_positive_bounds_only() {
        // The world above: net 2's class carries 10.0/20.0.
        let rules = length_bounds_world();
        assert!(rules.has_length_constraints(), "a declared bound activates");
        assert!(
            !BoardRules::default().has_length_constraints(),
            "the empty world (no nets, no classes) is constraint-free"
        );
        // The constraint-free-invariance negative face: every net in
        // the unconstrained default class ⇒ false.
        let mut plain = BoardRules::default();
        plain.net_classes.push(NetClass {
            name: "default".to_string(),
            trace_clearance_class: 1,
            trace_half_widths: vec![1500],
            active_routing_layers: vec![true],
            default_item_clearance_classes: [0, 1, 1, 1, 1, 1],
            via_rule: None,
            pull_tight: true,
            shove_fixed: false,
            min_trace_length: 0.0,
            max_trace_length: 0.0,
            ignore_cycles_with_areas: false,
        });
        plain.nets = Nets::from_ir(&[NetIr {
            name: "plain".to_string(),
            subnet_number: 1,
            contains_plane: false,
            net_class: 0,
        }]);
        assert!(
            !plain.has_length_constraints(),
            "declared class with 0.0/0.0 bounds: the zero-rotation face"
        );

        // The sentinel/defensive rows: the delivered domain is
        // non-negative (the > 0 delivery gates), but a hand-built
        // negative must not count either.
        let mut negative = length_bounds_world();
        negative.net_classes[1].min_trace_length = -5.0;
        negative.net_classes[1].max_trace_length = -1.0;
        assert!(
            !negative.has_length_constraints(),
            "negative bounds (the write-side sentinel domain) never activate"
        );
        assert_eq!(
            negative.net_class_length_bounds(2),
            (-5.0, -1.0),
            "the QUERY mirrors the stored fields verbatim (the delivery \
             gate upstream guarantees they are never negative; the \
             predicate, not the query, carries the > 0 semantics)"
        );

        // One-sided activation: min-only and max-only classes.
        let mut min_only = length_bounds_world();
        min_only.net_classes[1].min_trace_length = 10.0;
        min_only.net_classes[1].max_trace_length = 0.0;
        assert!(min_only.has_length_constraints(), "min-only activates");
        assert_eq!(min_only.net_class_length_bounds(2), (10.0, 0.0));
        let mut max_only = length_bounds_world();
        max_only.net_classes[1].min_trace_length = 0.0;
        max_only.net_classes[1].max_trace_length = 20.0;
        assert!(max_only.has_length_constraints(), "max-only activates");
        assert_eq!(max_only.net_class_length_bounds(2), (0.0, 20.0));
    }

    /// M7-T3: the `calcLengthViolation` predicate port
    /// (`NetIncompletes.java:257-271`) — every branch and both exact
    /// boundaries, values derived from the world (bounds 100.0/20.0
    /// world: min 10.0/max 20.0 scaled by 10 for readability below).
    #[test]
    fn length_violation_predicate_matches_java_calc_length_violation() {
        let mut rules = length_bounds_world();
        // World bounds on net 2's class: min 100 / max 200.
        rules.net_classes[1].min_trace_length = 100.0;
        rules.net_classes[1].max_trace_length = 200.0;

        // Both bounds absent -> 0 regardless of length (the early-out).
        let mut none = length_bounds_world();
        none.net_classes[1].min_trace_length = 0.0;
        none.net_classes[1].max_trace_length = 0.0;
        assert_eq!(none.length_violation(2, 9_999.0, false), 0.0);

        // Over-max fires with the POSITIVE excess (too long).
        assert_eq!(rules.length_violation(2, 350.0, true), 150.0);
        // EXACT max boundary: length == max is VALID (strict >), one
        // unit above fires with exactly 1.0 (DNR-16 both directions).
        assert_eq!(rules.length_violation(2, 200.0, true), 0.0);
        assert_eq!(rules.length_violation(2, 201.0, true), 1.0);

        // Under-min fires with the NEGATIVE deficit ONLY with no
        // incompletes (Java's incompletes.isEmpty() gate).
        assert_eq!(rules.length_violation(2, 60.0, false), -40.0);
        assert_eq!(rules.length_violation(2, 60.0, true), 0.0);
        // EXACT min boundary: length == min is VALID (strict <), one
        // unit below fires with exactly -1.0 (DNR-16 both directions).
        assert_eq!(rules.length_violation(2, 100.0, false), 0.0);
        assert_eq!(rules.length_violation(2, 99.0, false), -1.0);

        // In-range length: no violation either side.
        assert_eq!(rules.length_violation(2, 150.0, false), 0.0);

        // The Java-exact malformed face: BOTH ifs run in sequence and
        // the SECOND (under-min) OVERWRITES (Java `newViolation` is
        // assigned, not else-if'd) — min 200 > max 100 with length 150
        // reads -50, not +50.
        let mut malformed = length_bounds_world();
        malformed.net_classes[1].min_trace_length = 200.0;
        malformed.net_classes[1].max_trace_length = 100.0;
        assert_eq!(malformed.length_violation(2, 150.0, false), -50.0);
    }
}
