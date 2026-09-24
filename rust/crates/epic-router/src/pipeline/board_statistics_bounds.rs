//! Java `core/scoring/BoardStatisticsBoundsCalculator.java` — the
//! board-only lower bounds the V2 optimizer score consumes: a
//! per-net Prim MST over terminal positions (manhattan metric), the
//! axis-aligned bend count of that MST, and the greedy via-count
//! lower bound from the net class's via-rule spans.
//!
//! ## Determinism face (why exact jar pins are legal here)
//!
//! The walk reads ONLY parse-time state — terminal items are pins and
//! conduction areas (`Net.getTerminalItems` filters
//! `containsNet && !isRoutable()`; traces/vias answer `isRoutable()`
//! true unless user-fixed, and `toTerminal` answers null for them
//! anyway — so the terminal set is pins + conduction areas), their
//! positions, and the net classes' via rules. Routing moves none of
//! it; Java additionally memoizes the result per board
//! (`WeakHashMap` cache, `BoardStatisticsBoundsCalculator.java:25-37`),
//! so the manifest's `bounds` block is the LOAD-TIME capture even
//! though `new BoardStatistics(job.board)` runs post-routing.
//! Evidence: the jar's `--router.result_json` `bounds` block is
//! identical with and without routing on bm08
//! (`logs/M4-T8/evidence/jar-bounds/`: `bm08-manifest.json` vs
//! `bm08-noroute-manifest.json`, both `85.27/0/15`).
//! The Rust port recomputes on every call — observationally identical
//! (the function is pure over parse state) — and `BoardStatistics`
//! snapshots it into its `bounds` field at construction.
//!
//! ## Order sensitivity (why the walk order matters at all)
//!
//! Prim's seeding and the strict-`<` tie-breaks make the MST length
//! and bend count DEPEND on the terminal order: Java's
//! `getTerminalItems` walks `UndoableObjects.startReadObject`, whose
//! `ConcurrentSkipListMap` is keyed by `Item.compareTo` =
//! `other.id - this.id` — the DESCENDING-id order. The port walks
//! [`Board::iter_descending`] (live items only) to reproduce it; ties
//! break toward the FIRST minimum in the selection scan and toward the
//! OLD parent in the relaxation (both strict `<`).
//!
//! ## The two rendering faces (why the jar pins pin raw bits)
//!
//! `GsonProvider.GSON` registers a `TwoDecimalFloatAdapter` — every
//! `Float` field of the result JSON renders through `String.format
//! (%.2f)` (`GsonProvider.java:26-27, 36-43`), so the manifest's
//! `bounds.min_trace_length_mm` is a TWO-DECIMAL VIEW, not the stored
//! f32 (`85.26519775...` renders as `85.27`). Pins therefore capture
//! the RAW f32 bits from the harness-side probe (`BoundsOracle.java`,
//! the TraceTightenerProbe precedent) and assert the rendered view
//! separately against the `--router.result_json` literals.

use epic_board::board::Board;
use epic_board::id::ItemId;
use epic_board::items::{Area, BoardShape, ItemData};
use epic_dsn::state::Unit;
use epic_geometry::float_point::FloatPoint;

/// Java `core/scoring/BoardStatisticsBounds` (the `BoardStatistics`
/// field annotated `@SerializedName("bounds")`) — the serialized keys
/// are `min_trace_length_mm` / `min_via_count` / `min_bend_count`.
///
/// `Default` (all-`None`) is the GSON-empty face (Java's field
/// initializer `new BoardStatisticsBounds()`); [`Self::empty`] spells
/// it at the call sites so they read honestly.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoardStatisticsBounds {
    /// Java `minTraceLengthMm` (`Float`) — the summed MST manhattan
    /// length in millimetres; the f64 accumulator is cast to f32 ONCE,
    /// at the end (`(float) minTraceLength`), not per net.
    pub min_trace_length_mm: Option<f32>,
    /// Java `minViaCount` (`Integer`).
    pub min_via_count: Option<i32>,
    /// Java `minBendCount` (`Integer`).
    pub min_bend_count: Option<i32>,
}

impl BoardStatisticsBounds {
    /// The GSON-empty face (Java's field initializer
    /// `new BoardStatisticsBounds()` — every field null); equals
    /// `Self::default()`.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            min_trace_length_mm: None,
            min_via_count: None,
            min_bend_count: None,
        }
    }
}

/// Java `BoardStatisticsBoundsCalculator.Terminal` — one terminal's
/// position (board units, f64) and its signal-layer set.
#[derive(Clone, Debug, PartialEq)]
struct Terminal {
    x: f64,
    y: f64,
    /// Java `HashSet<Integer>` — the partition arithmetic below
    /// consumes only membership/size, never iteration order; the
    /// ordered set keeps the port's own iteration deterministic.
    signal_layers: std::collections::BTreeSet<i32>,
}

/// Java `BoardStatisticsBoundsCalculator.MstResult`.
#[derive(Clone, Debug, PartialEq)]
struct MstResult {
    length: f64,
    bend_count: i32,
}

/// Java `BoardStatisticsBoundsCalculator.calculate(BasicBoard)` — the
/// uncached body (`calculateUncached`); see the module docs for the
/// cache note.
#[must_use]
pub fn calculate(board: &Board) -> BoardStatisticsBounds {
    let mut min_trace_length = 0.0f64;
    let mut min_via_count = 0i32;
    let mut min_bend_count = 0i32;
    // Java: `Unit.scale(1.0, unit, MM) / (resolution > 0 ? resolution : 1)`
    // — the same f64 factor the counting ctor uses for
    // `traces.totalLengthMm`.
    let communication = board.communication();
    let resolution = i64::from(communication.resolution);
    let resolution = if resolution > 0 { resolution } else { 1 };
    let board_unit_to_mm_factor =
        Unit::scale(1.0, communication.unit, Unit::Mm) / resolution as f64;

    let max_net_number = board.rules().nets.max_net_number();
    for net_number in 1..=max_net_number {
        // Java `continue`s when `nets.get(n)` answers null — kept 1:1.
        let Some(net) = board.rules().nets.get(net_number) else {
            continue;
        };
        let terminals = get_terminals(board, net_number);
        if terminals.len() < 2 {
            continue;
        }
        let mst = calculate_mst(&terminals);
        min_trace_length += mst.length * board_unit_to_mm_factor;
        min_bend_count += mst.bend_count;
        min_via_count += calculate_minimum_via_count(board, net.net_class, &terminals);
    }

    BoardStatisticsBounds {
        min_trace_length_mm: Some(min_trace_length as f32),
        min_via_count: Some(min_via_count),
        min_bend_count: Some(min_bend_count),
    }
}

/// Java `getTerminals` + `toTerminal` fused over the LIVE walk in the
/// skip-list order: pins and conduction areas of the net whose
/// signal-layer set is non-empty. (`toTerminal` answers null for every
/// other item kind — including user-fixed traces/vias, which
/// `getTerminalItems` collects but `toTerminal` drops — and
/// `getTerminals` skips terminals with an empty layer set.)
///
/// ORDER: `UndoableObjects.objects` is a `ConcurrentSkipListMap` keyed
/// by the item with `Item.compareTo` = `other.id - this.id`, so the
/// `startReadObject` walk yields items in DESCENDING-id order —
/// [`Board::iter_descending`]. (The counting ctor's counts are
/// order-independent, which is why the ascending walk there is
/// observably identical; Prim's algorithm is not, so this port must
/// walk the comparator order.)
fn get_terminals(board: &Board, net_number: i32) -> Vec<Terminal> {
    let mut result = Vec::new();
    for entry in board.iter_descending() {
        if !entry.on_the_board {
            // Java's `UndoableObjects.delete` removed the node from the
            // map; the tombstone must not surface (the counting ctor's
            // discipline).
            continue;
        }
        if !entry.nets.contains(&net_number) {
            continue;
        }
        let Some(terminal) = to_terminal(board, entry.id, &entry.data) else {
            continue;
        };
        if terminal.signal_layers.is_empty() {
            continue;
        }
        result.push(terminal);
    }
    result
}

/// Java `toTerminal`: a pin contributes its (possibly shape-corrected)
/// center and the signal layers of its padstack span; a conduction area
/// contributes its border's centre of gravity and its own layer;
/// anything else is null.
fn to_terminal(board: &Board, id: ItemId, data: &ItemData) -> Option<Terminal> {
    match data {
        ItemData::Pin { .. } => {
            // Java `pin.getCenter().toFloat()` — the shape-corrected
            // center (the port recomputes the same pure value;
            // `epic_board::components::pin_center` documents the chain).
            let center = board.pin_center(id)?;
            let float = center.to_float();
            Some(Terminal {
                x: float.x,
                y: float.y,
                signal_layers: pin_signal_layers(board, id),
            })
        }
        ItemData::ConductionArea { layer, .. } => {
            let area: Area = board.conduction_area(id)?;
            // Java `area.getArea().getBorder().centreOfGravity()`: for a
            // `PolylineArea` the border is the boundary `PolylineShape`
            // (corner average); a parse-time conduction area over a bare
            // shape carries that shape's own face — the Circle arm is
            // `center.toFloat()`, the tile family's is the corner
            // average of its border lines' endpoints.
            let gravity = match area.border {
                BoardShape::Tile(ref tile) => tile.centre_of_gravity(),
                BoardShape::PolygonShape(ref polygon) => {
                    // Java inherits `PolylineShape.centreOfGravity`:
                    // sum `cornerApprox(i)` over `borderLineCount()`
                    // corners in order, divide each axis by the count.
                    let count = polygon.border_line_count();
                    let mut x = 0.0f64;
                    let mut y = 0.0f64;
                    for i in 0..count {
                        let current = polygon.corner(i as i32).to_float();
                        x += current.x;
                        y += current.y;
                    }
                    FloatPoint::new(x / count as f64, y / count as f64)
                }
                BoardShape::Circle(ref circle) => circle.centre_of_gravity(),
            };
            let mut signal_layers = std::collections::BTreeSet::new();
            add_signal_layer(board, *layer, &mut signal_layers);
            Some(Terminal {
                x: gravity.x,
                y: gravity.y,
                signal_layers,
            })
        }
        _ => None,
    }
}

/// Java `DrillItem.firstLayer()/lastLayer()` probe + `Pin.getShape`:
/// for every layer in the (mirrored) padstack span, a non-null shape
/// contributes the layer IF it is a signal layer. The span endpoints
/// come from probing [`Board::pin_is_on_layer`] (the port of the
/// mirrored span), which is exactly the contiguous span the Java loop
/// iterates.
fn pin_signal_layers(board: &Board, id: ItemId) -> std::collections::BTreeSet<i32> {
    let mut result = std::collections::BTreeSet::new();
    let layer_count = board.layers().layers.len() as i32;
    let Some(first) = (0..layer_count).find(|&layer| board.pin_is_on_layer(id, layer)) else {
        return result;
    };
    let Some(last) = (0..layer_count)
        .rev()
        .find(|&layer| board.pin_is_on_layer(id, layer))
    else {
        return result;
    };
    for layer in first..=last {
        // Java `pin.getShape(layer - pin.firstLayer()) != null`.
        if board.pin_shape(id, layer - first).is_some() {
            add_signal_layer(board, layer, &mut result);
        }
    }
    result
}

/// Java `addSignalLayer`: in range AND a signal layer, else skip.
fn add_signal_layer(
    board: &Board,
    layer: i32,
    signal_layers: &mut std::collections::BTreeSet<i32>,
) {
    let layers = board.layers();
    if layer >= 0 && layer < layers.layers.len() as i32 && layers.layers[layer as usize].is_signal {
        signal_layers.insert(layer);
    }
}

/// Java `calculateMst` — Prim's algorithm, seeded at terminal 0, with
/// Java's strict-`<` comparisons (first minimum wins the selection
/// scan; ties keep the old parent in the relaxation). f64 end to end;
/// the bend test is Java's DOUBLE EQUALITY on both axes.
fn calculate_mst(terminals: &[Terminal]) -> MstResult {
    let count = terminals.len();
    let mut used = vec![false; count];
    let mut distances = vec![f64::INFINITY; count];
    let mut parents = vec![-1i32; count];
    distances[0] = 0.0;

    let mut length = 0.0f64;
    let mut bend_count = 0i32;
    for _edge in 0..count {
        let mut current: i32 = -1;
        for index in 0..count {
            if !used[index] && (current < 0 || distances[index] < distances[current as usize]) {
                current = index as i32;
            }
        }
        if current < 0 {
            break;
        }
        let current = current as usize;
        used[current] = true;
        if parents[current] >= 0 {
            let from = &terminals[parents[current] as usize];
            let to = &terminals[current];
            length += distances[current];
            if from.x != to.x && from.y != to.y {
                bend_count += 1;
            }
        }
        for index in 0..count {
            if !used[index] {
                let distance = manhattan_distance(&terminals[current], &terminals[index]);
                if distance < distances[index] {
                    distances[index] = distance;
                    parents[index] = current as i32;
                }
            }
        }
    }
    MstResult { length, bend_count }
}

/// Java `manhattanDistance`.
fn manhattan_distance(first: &Terminal, second: &Terminal) -> f64 {
    (first.x - second.x).abs() + (first.y - second.y).abs()
}

/// Java `calculateMinimumViaCount` — the greedy set cover of the
/// terminals' signal-layer groups by the net class's via-rule spans (a
/// span covers a group when the group's REPRESENTATIVE layer sits in
/// `[min(from, to), max(from, to)]`; first-best wins ties, strict `>`).
fn calculate_minimum_via_count(board: &Board, net_class: i32, terminals: &[Terminal]) -> i32 {
    let mut groups = LayerGroups::default();
    for terminal in terminals {
        groups.add_layers(&terminal.signal_layers);
    }
    if groups.count() <= 1 {
        return 0;
    }

    // Java: `net.getNetClass() != null ? ...getViaRule() : null`, then
    // `if (viaRule == null) return 0;` — every null face falls back to
    // zero vias.
    let Some(class) = board.rules().net_classes.get(net_class as usize) else {
        return 0;
    };
    let Some(rule_id) = class.via_rule else {
        return 0;
    };
    let Some(via_rule) = board.rules().via_rule_by_id(rule_id) else {
        return 0;
    };

    let mut spans = Vec::new();
    for &info_index in &via_rule.via_infos {
        let Some(info) = board.rules().via_infos.get(info_index as usize) else {
            continue;
        };
        let Some(padstack) = board.library().padstack(info.padstack_no) else {
            continue;
        };
        let first = padstack.from_layer() as i32;
        let last = padstack.to_layer();
        spans.push((first.min(last), first.max(last)));
    }

    let mut remaining_groups = groups.roots();
    let mut via_count = 0i32;
    while !remaining_groups.is_empty() {
        let mut best_span: Option<(i32, i32)> = None;
        let mut best_covered_groups = std::collections::BTreeSet::new();
        for &span in &spans {
            let covered = groups
                .groups_covered_by(span)
                .intersection(&remaining_groups)
                .copied()
                .collect::<std::collections::BTreeSet<i32>>();
            if covered.len() > best_covered_groups.len() {
                best_span = Some(span);
                best_covered_groups = covered;
            }
        }
        if best_span.is_none() || best_covered_groups.is_empty() {
            break;
        }
        for &group in &best_covered_groups {
            remaining_groups.remove(&group);
        }
        via_count += 1;
    }
    via_count
}

/// Java `LayerGroups` — the union-find over the terminals' signal
/// layers. The partition (hence `count`/`roots`/`groups_covered_by`)
/// is iteration-order independent, so the port's ordered map is honest;
/// `find` climbs the chain WITHOUT path compression (Java rewrites the
/// map — same root, and the port's `&self` stays pure).
#[derive(Default)]
struct LayerGroups {
    parent: std::collections::BTreeMap<i32, i32>,
}

impl LayerGroups {
    fn add_layers(&mut self, layers: &std::collections::BTreeSet<i32>) {
        let mut first_layer: Option<i32> = None;
        for &layer in layers {
            self.parent.entry(layer).or_insert(layer);
            match first_layer {
                None => first_layer = Some(layer),
                Some(first) => self.union(first, layer),
            }
        }
    }

    fn count(&self) -> usize {
        self.roots().len()
    }

    fn roots(&self) -> std::collections::BTreeSet<i32> {
        self.parent
            .keys()
            .copied()
            .map(|layer| self.find(layer))
            .collect()
    }

    fn groups_covered_by(&self, span: (i32, i32)) -> std::collections::BTreeSet<i32> {
        self.parent
            .keys()
            .copied()
            .filter(|&layer| layer >= span.0 && layer <= span.1)
            .map(|layer| self.find(layer))
            .collect()
    }

    fn find(&self, layer: i32) -> i32 {
        let current_parent = self.parent[&layer];
        if current_parent != layer {
            return self.find(current_parent);
        }
        current_parent
    }

    fn union(&mut self, first: i32, second: i32) {
        let first_root = self.find(first);
        let second_root = self.find(second);
        if first_root != second_root {
            self.parent.insert(second_root, first_root);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::board_statistics::BoardStatistics;
    use crate::test_util::parse;

    // ------------------------------------------------------------------
    // crafted worlds — the arithmetic faces the jar boards cannot pin
    // ------------------------------------------------------------------

    /// One diagonal net (2 pins, both axes differ): MST length is the
    /// manhattan distance and the bend count 1. `unit um`, resolution
    /// 1 → 1 unit = 0.001 mm.
    #[test]
    fn t8_mst_two_pin_diagonal_world() {
        let dsn = r#"(pcb t8mst.dsn
  (resolution um 1)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary (rect pcb 0 0 130000 70000))
    (rule (width 200) (clearance 200))
  )
  (placement
    (component CMP_A (place CMP_A 20000 30000 front 0))
    (component CMP_B (place CMP_B 35000 48000 front 0))
  )
  (library
    (padstack PAD_SMD (shape (circle F.Cu 600 0 0)))
    (padstack PAD_VIA
      (shape (circle F.Cu 800 0 0))
      (shape (circle B.Cu 800 0 0))
    )
    (image CMP_A (pin PAD_SMD P1 0 0))
    (image CMP_B (pin PAD_SMD P1 0 0))
  )
  (network
    (via V1 PAD_VIA default)
    (via_rule R1 V1)
    (net NET1 (pins CMP_A-P1 CMP_B-P1))
  )
)
"#;
        let (_manager, board) = parse(dsn);
        let bounds = calculate(&board);
        // manhattan (15000 + 18000) units * 0.001 mm/unit = 33.0 mm.
        assert_eq!(bounds.min_trace_length_mm, Some(33.0));
        assert_eq!(bounds.min_bend_count, Some(1), "both axes differ");
        assert_eq!(bounds.min_via_count, Some(0), "single-group net");
    }

    /// The collinear control: same x → NO bend (Java's double equality
    /// `from.y != to.y` is false), length is the single-axis delta.
    #[test]
    fn t8_mst_collinear_no_bend_control() {
        let dsn = r#"(pcb t8col.dsn
  (resolution um 1)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary (rect pcb 0 0 130000 70000))
    (rule (width 200) (clearance 200))
  )
  (placement
    (component CMP_A (place CMP_A 20000 30000 front 0))
    (component CMP_B (place CMP_B 35000 30000 front 0))
  )
  (library
    (padstack PAD_SMD (shape (circle F.Cu 600 0 0)))
    (padstack PAD_VIA
      (shape (circle F.Cu 800 0 0))
      (shape (circle B.Cu 800 0 0))
    )
    (image CMP_A (pin PAD_SMD P1 0 0))
    (image CMP_B (pin PAD_SMD P1 0 0))
  )
  (network
    (via V1 PAD_VIA default)
    (via_rule R1 V1)
    (net NET1 (pins CMP_A-P1 CMP_B-P1))
  )
)
"#;
        let (_manager, board) = parse(dsn);
        let bounds = calculate(&board);
        assert_eq!(bounds.min_trace_length_mm, Some(15.0));
        assert_eq!(bounds.min_bend_count, Some(0), "same y → no bend");
    }

    /// A cross-layer net: the terminals sit on DIFFERENT signal layers
    /// (an SMD front pin + a back padstack-only pin is not craftable in
    /// plain DSN text, so the world uses a front SMD pin + a
    /// through-hole pin on a 2-layer board — the TH pin contributes
    /// BOTH layers, the SMD pin layer 0; the union is one group →
    /// still zero vias. The VIA arm needs DISJOINT groups, crafted in
    /// [`Self::t8_via_cover_two_disjoint_groups`]).
    #[test]
    fn t8_via_single_group_control() {
        let dsn = r#"(pcb t8via1.dsn
  (resolution um 1)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary (rect pcb 0 0 130000 70000))
    (rule (width 200) (clearance 200))
  )
  (placement
    (component CMP_A (place CMP_A 20000 30000 front 0))
    (component CMP_TH (place CMP_TH 35000 48000 front 0))
  )
  (library
    (padstack PAD_SMD (shape (circle F.Cu 600 0 0)))
    (padstack PAD_TH
      (shape (circle F.Cu 500 0 0))
      (shape (circle B.Cu 500 0 0))
    )
    (padstack PAD_VIA
      (shape (circle F.Cu 800 0 0))
      (shape (circle B.Cu 800 0 0))
    )
    (image CMP_A (pin PAD_SMD P1 0 0))
    (image CMP_TH (pin PAD_TH P1 0 0))
  )
  (network
    (via V1 PAD_VIA default)
    (via_rule R1 V1)
    (net NET1 (pins CMP_A-P1 CMP_TH-P1))
  )
)
"#;
        let (_manager, board) = parse(dsn);
        let bounds = calculate(&board);
        // The TH pin's layer set {0,1} ∪ the SMD pin's {0} = one group.
        assert_eq!(bounds.min_via_count, Some(0));
    }

    /// The disjoint-groups world: two SMD pins on OPPOSITE sides. A
    /// back-side component mirrors its padstack span (placed on the
    /// back → the pin's signal layer is 1), so the net's groups are
    /// {0} and {1} and the via rule's full-span via covers both →
    /// exactly one via. Mutation armor: the ±1 via-count mutants and
    /// the dropped-cover-break mutant all move this value.
    #[test]
    fn t8_via_cover_two_disjoint_groups() {
        let dsn = r#"(pcb t8via2.dsn
  (resolution um 1)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary (rect pcb 0 0 130000 70000))
    (rule (width 200) (clearance 200))
  )
  (placement
    (component CMP_A (place CMP_A 20000 30000 front 0))
    (component CMP_B (place CMP_B 35000 48000 back 0))
  )
  (library
    (padstack PAD_SMD (shape (circle F.Cu 600 0 0)))
    (padstack PAD_VIA
      (shape (circle F.Cu 800 0 0))
      (shape (circle B.Cu 800 0 0))
    )
    (image CMP_A (pin PAD_SMD P1 0 0))
    (image CMP_B (pin PAD_SMD P1 0 0))
  )
  (network
    (via V1 PAD_VIA default)
    (via_rule R1 V1)
    (net NET1 (pins CMP_A-P1 CMP_B-P1))
  )
)
"#;
        let (_manager, board) = parse(dsn);
        let bounds = calculate(&board);
        assert_eq!(
            bounds.min_via_count,
            Some(1),
            "disjoint groups need exactly one spanning via"
        );
    }

    /// THE ORDER-DISCRIMINATING WORLD (spec-review MINOR-1 fix; kills
    /// the T8-S1 ascending-walk mutant). Three terminals where two
    /// equal-weight MSTs of the SAME point set carry different bend
    /// counts: t0=(20000,30000), t1=(20000,30100), t2=(20020,30080).
    /// The seed tie (t1 and t2 both at manhattan 100 from t0) is broken
    /// toward the FIRST index, so:
    ///   * walk [t0,t1,t2] (ASCENDING ids — the T8-S1 mutant): tree
    ///     {t0-t1, t1-t2}, bends 1 (only t1-t2 is L-shaped);
    ///   * walk [t2,t1,t0] (DESCENDING — Java's skip-list order): tree
    ///     {t2-t1, t2-t0}, bends 2 (both edges L-shaped).
    ///
    /// Both trees span 140 units (Prim correctness — the LENGTH is
    /// order-invariant; the BEND COUNT is the order-variant face).
    /// JAR-ANCHORED via BoundsOracle on this exact DSN (evidence
    /// `logs/M4-T8/evidence/t8_order_world_bounds_oracle.log`): walk
    /// order [t2,t1,t0], mst 140.0 (bits 4639129828656676864), bends 2,
    /// min_trace_length_mm f32 bits 1041194025 (0.14000000059604645).
    #[test]
    fn t8_prim_walk_order_bend_count_discriminated() {
        let dsn = r#"(pcb t8_order_world.dsn
  (resolution um 1)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary (rect pcb 0 0 130000 70000))
    (rule (width 200) (clearance 200))
  )
  (placement
    (component CMP_T0 (place CMP_T0 20000 30000 front 0))
    (component CMP_T1 (place CMP_T1 20000 30100 front 0))
    (component CMP_T2 (place CMP_T2 20020 30080 front 0))
  )
  (library
    (padstack PAD_SMD (shape (circle F.Cu 600 0 0)))
    (padstack PAD_VIA
      (shape (circle F.Cu 800 0 0))
      (shape (circle B.Cu 800 0 0))
    )
    (image CMP_T0 (pin PAD_SMD P1 0 0))
    (image CMP_T1 (pin PAD_SMD P1 0 0))
    (image CMP_T2 (pin PAD_SMD P1 0 0))
  )
  (network
    (via V1 PAD_VIA default)
    (via_rule R1 V1)
    (net NET1 (pins CMP_T0-P1 CMP_T1-P1 CMP_T2-P1))
  )
)
"#;
        let (_manager, board) = parse(dsn);
        let bounds = calculate(&board);
        // The jar's own walk (descending) answers bends 2 — the port
        // must reproduce it; the ascending mutant answers 1.
        assert_eq!(
            bounds.min_bend_count,
            Some(2),
            "the descending-id Prim walk must take the t2-t1 + t2-t0 tree"
        );
        assert_eq!(
            bounds.min_trace_length_mm.map(f32::to_bits),
            Some(1041194025),
            "0.14000000059604645 mm raw — length is order-INVARIANT (140 units)"
        );
        assert_eq!(bounds.min_via_count, Some(0));
    }

    // ------------------------------------------------------------------
    // jar-exact tier-fixture pins (the parse/ratsnest faces)
    //
    // TWO faces, both captured 2026-09-23 (evidence:
    // logs/M4-T8/evidence/jar-bounds/):
    //   * RAW f32 bits from the harness-side probe `BoundsOracle.java`
    //     (oracle JVM, jar e7f9bdf1) — the honest capture (cerebrum
    //     mode 5: never pin through a formatting toString);
    //   * the RENDERED view from the jar `--router.result_json`
    //     manifests — GsonProvider's TwoDecimalFloatAdapter renders
    //     every Float through `%.2f`, so e.g. bm08's raw
    //     85.26519775... renders as "85.27". The render pins apply the
    //     documented serialization contract to the port's own f32 and
    //     compare the STRING (Rust's `{:.2}` and Java's `%.2f` agree at
    //     these values — none sits on a half-unit boundary).
    // ------------------------------------------------------------------

    const BM08: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../scripts/benchmark/fixtures/DAC2020_boards/DAC2020_bm08.dsn"
    );
    const ECC83: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../scripts/benchmark/fixtures/KiCad_10_demos/ecc83-pp.dsn"
    );
    const DECEL: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../scripts/benchmark/fixtures/PCBench/decelerator4030_decelerator4030/reference-routed.dsn"
    );

    /// bm08: pins-only board, 9 multi-terminal nets. Bounds are
    /// routing-independent (jar manifest identical routed vs unrouted:
    /// `bm08-manifest.json` vs `bm08-noroute-manifest.json`).
    #[test]
    fn t8_bounds_bm08_jar_exact() {
        let bytes = std::fs::read(BM08).expect("bm08 fixture present");
        let mut ses = epic_dsn::ses_board::SesBoard::new();
        let parsed = epic_dsn::reader::read_board(bytes.as_slice(), &mut ses);
        assert!(matches!(
            parsed,
            epic_dsn::reader::DsnReadResult::Success { .. }
        ));
        let board = epic_board::board::Board::from_ses_board(&ses);
        let bounds = calculate(&board);
        // Raw f32 bits from BoundsOracle: min_trace_length_mm=1118472136.
        assert_eq!(
            bounds.min_trace_length_mm.map(f32::to_bits),
            Some(1118472136),
            "85.26519775... mm raw"
        );
        assert_eq!(bounds.min_via_count, Some(0));
        assert_eq!(bounds.min_bend_count, Some(15));
        // The rendered view the jar manifest carries.
        assert_eq!(
            format!(
                "{:.2}",
                bounds.min_trace_length_mm.expect("length bound set")
            ),
            "85.27",
            "TwoDecimalFloatAdapter view of the raw f32"
        );
    }

    /// ecc83-pp: the second tier fixture. Probe bits 1133874815
    /// (299.11325... mm); manifest renders "299.11".
    #[test]
    fn t8_bounds_ecc83_jar_exact() {
        let bytes = std::fs::read(ECC83).expect("ecc83 fixture present");
        let mut ses = epic_dsn::ses_board::SesBoard::new();
        let parsed = epic_dsn::reader::read_board(bytes.as_slice(), &mut ses);
        assert!(matches!(
            parsed,
            epic_dsn::reader::DsnReadResult::Success { .. }
        ));
        let board = epic_board::board::Board::from_ses_board(&ses);
        let bounds = calculate(&board);
        assert_eq!(
            bounds.min_trace_length_mm.map(f32::to_bits),
            Some(1133874815)
        );
        assert_eq!(bounds.min_via_count, Some(0));
        assert_eq!(bounds.min_bend_count, Some(19));
        assert_eq!(
            format!(
                "{:.2}",
                bounds.min_trace_length_mm.expect("length bound set")
            ),
            "299.11"
        );
    }

    /// The decelerator reference-routed board (no-router jar run): the
    /// multi-group world — 206 nets, exactly ONE net needs a spanning
    /// via (`min_via_count` 1, caught by the greedy cover arm) — and a
    /// large f32 length. Probe bits 1187429928 (25439.078125 mm);
    /// manifest renders "25439.08".
    #[test]
    fn t8_bounds_decel_jar_exact() {
        let bytes = std::fs::read(DECEL).expect("decel fixture present");
        let mut ses = epic_dsn::ses_board::SesBoard::new();
        let parsed = epic_dsn::reader::read_board(bytes.as_slice(), &mut ses);
        assert!(matches!(
            parsed,
            epic_dsn::reader::DsnReadResult::Success { .. }
                | epic_dsn::reader::DsnReadResult::OutlineMissing { .. }
        ));
        let board = epic_board::board::Board::from_ses_board(&ses);
        let bounds = calculate(&board);
        assert_eq!(
            bounds.min_trace_length_mm.map(f32::to_bits),
            Some(1187429928)
        );
        assert_eq!(bounds.min_via_count, Some(1));
        assert_eq!(bounds.min_bend_count, Some(883));
        assert_eq!(
            format!(
                "{:.2}",
                bounds.min_trace_length_mm.expect("length bound set")
            ),
            "25439.08"
        );
    }

    /// The snapshot wiring: `BoardStatistics::with_options` carries the
    /// calculator's result (bm08) and the empty face is all-`None`
    /// (Java's field initializer).
    #[test]
    fn t8_bounds_snapshot_wiring() {
        let bytes = std::fs::read(BM08).expect("bm08 fixture present");
        let mut ses = epic_dsn::ses_board::SesBoard::new();
        let parsed = epic_dsn::reader::read_board(bytes.as_slice(), &mut ses);
        assert!(matches!(
            parsed,
            epic_dsn::reader::DsnReadResult::Success { .. }
        ));
        let mut board = epic_board::board::Board::from_ses_board(&ses);
        let mut manager = epic_board::tree_manager::SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        let stats = BoardStatistics::new(&mut manager, &mut board);
        assert_eq!(
            stats.bounds.min_trace_length_mm.map(f32::to_bits),
            Some(1118472136)
        );
        assert_eq!(stats.bounds.min_via_count, Some(0));
        assert_eq!(stats.bounds.min_bend_count, Some(15));
        assert_eq!(
            BoardStatisticsBounds::empty(),
            BoardStatisticsBounds {
                min_trace_length_mm: None,
                min_via_count: None,
                min_bend_count: None,
            }
        );
    }
}
