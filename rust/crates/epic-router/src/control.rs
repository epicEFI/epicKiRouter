//! Java `autoroute/maze/AutorouteControl.java` — the per-net cost table
//! the maze engine consumes. Ported at M3-T3 as an owned struct built
//! from `(board, net number, settings IR)`; every field of the Java
//! class has a counterpart, and the two mutation seams (`init_net`,
//! `rebuild_via_info`) are 1:1. The PASS DRIVER is NOT ported here —
//! notably the ripup-cost formula application (see [`AutorouteControl::ripup_costs`]).

use epic_board::board::Board;
use epic_board::items::{BoardItemType, BoardShape};
use epic_board::rules_surf::{NetClass, ViaRule};

/// Java `RouterSettings` — the fields `AutorouteControl` consumes, and
/// ONLY those (the real settings merger/resolution is T13, epic-cli;
/// this IR is construction-site agnostic — T13 builds it, the control
/// only reads it). Java resolves defaults inside the getters
/// (`getViasAllowed` null → `true`, `getViaCosts` null → 1,
/// `getAutomaticNeckdown` null → `false`, `getBendCost` null → 0.0,
/// `getLayerActive` null → `true`); here the RESOLVER owns defaults,
/// so every field is a plain resolved value.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct RouterSettingsIr {
    /// Java `getTraceCosts()` (`RouterSettings.java:881-894`) — the
    /// per-layer `(horizontal, vertical)` trace-cost factors. Java
    /// returns a zero-length array when unconfigured; the control then
    /// indexes it per layer from the maze engine (T6), not here.
    pub trace_costs: Vec<ExpansionCostFactor>,
    /// Java `getViaCosts()` (`:604-606`, default 1).
    pub via_costs: i32,
    /// Java `getViasAllowed()` (`:599-601`, default `true`).
    pub vias_allowed: bool,
    /// Java `getBendCost(layer)` (`:696-710`) per layer (default 0.0).
    pub bend_costs: Vec<f64>,
    /// Java `getLayerActive(layer)` (`:657-673`) per layer
    /// (default `true`). Out-of-range slots are read as `false` (Java
    /// warns and returns false).
    pub layer_active: Vec<bool>,
    /// Java `getAutomaticNeckdown()` (`:896-898`, default `false`).
    pub automatic_neckdown: bool,
}

/// Java `AutorouteControl.ExpansionCostFactor` (record,
/// `AutorouteControl.java:286-287`): horizontal and vertical costs for
/// traces on one board layer.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct ExpansionCostFactor {
    pub horizontal: f64,
    pub vertical: f64,
}

/// Java `AutorouteControl.ViaCost` (`:289-297`): the additional costs
/// to `min_normal_via_cost` for inserting a via between two layers.
/// Java zeroes `toLayer` at construction; the ENGINE (T4+) fills the
/// table — T3 carries the zeroed structure.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ViaCost {
    pub to_layer: Vec<i32>,
}

/// Java `AutorouteControl.ViaMask` (`:299-310`): one possible via
/// range — the padstack's layer span plus the via's SMD-attach flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViaMask {
    pub from_layer: i32,
    pub to_layer: i32,
    pub attach_smd_allowed: bool,
}

/// Structure for controlling the autoroute algorithm (Java
/// `AutorouteControl.java:18-311`). Construction runs the private
/// ctor body and then `initNet` — the Java 3-arg public ctor
/// (`:117-120`); the 5-arg overload has no consumer outside the maze
/// engine and lands with T6 if it needs it.
#[derive(Clone, Debug, PartialEq)]
pub struct AutorouteControl {
    /// Java holds the LIVE settings reference; T3 owns a clone (the
    /// driver re-reads `via_costs` per pass — T12).
    pub settings: RouterSettingsIr,

    /// Java `traceCosts` — `settings.getTraceCosts()`, stored verbatim.
    pub trace_costs: Vec<ExpansionCostFactor>,

    /// Java `bendCosts` — `settings.getBendCost(i)` per layer.
    pub bend_costs: Vec<f64>,

    /// Java `withNeckdown` — `settings.getAutomaticNeckdown()`.
    pub with_neckdown: bool,

    /// Java `layerActive` — per layer, settings value with the two
    /// force-false overrides (non-signal layer in the ctor;
    /// net-class inactive layer in `init_net`).
    pub layer_active: Vec<bool>,

    /// Java `layerCount` — `board.getLayerCount()`.
    pub layer_count: usize,

    /// Java `traceHalfWidth` — the trace half widths per layer.
    pub trace_half_width: Vec<i32>,

    /// Java `compensatedTraceHalfWidth` — `traceHalfWidth` plus the
    /// clearance-compensation value of [`Self::trace_clearance_class_index`].
    pub compensated_trace_half_width: Vec<i32>,

    /// Java `viaRadii` — persists ACROSS `rebuild_via_info` calls
    /// (Java never zeroes it there; the per-via radii accumulate by
    /// max). Mirrors that exactly.
    pub via_radii: Vec<f64>,

    /// Java `addViaCosts` — one zeroed [`ViaCost`] per layer.
    pub add_via_costs: Vec<ViaCost>,

    /// Java `traceClearanceClassIndex` — the trace clearance class.
    pub trace_clearance_class_index: i32,

    /// Java `viasAllowed` — `settings.getViasAllowed()`.
    pub vias_allowed: bool,

    /// Java `attachSmdAllowed` — true if any via of the rule may drill
    /// to an SMD pad (or the pure-SMD relaxation forced it).
    pub attach_smd_allowed: bool,

    /// Java `minNormalViaCost` — `viaCosts * max(max(maxViaRadius, 1), ×0.1 pure SMD)`.
    pub min_normal_via_cost: f64,

    /// Java `ripupAllowed` — ctor false; the driver (T12) enables it.
    pub ripup_allowed: bool,

    /// Java `ripupCosts` — ctor default 1000 (`AutorouteControl.java:185`).
    /// The PASS DRIVER overwrites it each pass:
    /// `autorouteControl.ripupCosts = router.getStartRipupCosts() * ripupPassNo;`
    /// (`AutorouteConnectionRouter.java:46` and `:202`) — T3 ports the
    /// FIELD and documents the formula; the APPLICATION is T12's.
    pub ripup_costs: i32,

    /// Java `ripupPassNo` — ctor 1 (`:186`); the driver bumps it.
    pub ripup_pass_no: i32,

    /// Java `isFanout` — ctor false; the fanout pass (T12) flips it.
    pub is_fanout: bool,

    /// Java `fanoutStartPinName` — targeted fanout diagnostics (T12).
    pub fanout_start_pin_name: Option<String>,

    /// Java `fanoutStartPinCenter` — targeted fanout diagnostics (T12).
    pub fanout_start_pin_center: Option<epic_geometry::point::Point>,

    /// Java `fanoutStartPinLayer` — ctor -1.
    pub fanout_start_pin_layer: i32,

    /// Java `removeUnconnectedVias` — ctor true (`:167`); "normally
    /// true, if the autorouter contains no fanout pass".
    pub remove_unconnected_vias: bool,

    /// Java `viaRule` — the net class's via rule (or the null-net
    /// first-rule fallback). Java's field is null until `initNet`;
    /// `Option` mirrors that. `None` after construction is impossible
    /// via [`AutorouteControl::new`] (init always runs), but
    /// `rebuild_via_info` stays `None`-tolerant (Java would NPE — an
    /// unreachable path treated as zero vias).
    pub via_rule: Option<ViaRule>,

    /// Java `netNumber` — the currently routed net.
    pub net_number: i32,

    /// Java `viaClearanceClass` — the first via's clearance class, else 1.
    pub via_clearance_class: i32,

    /// Java `viaInfos` — one [`ViaMask`] per via of the rule.
    pub via_infos: Vec<ViaMask>,

    /// Java `viaLowerBound` — ctor 0.
    pub via_lower_bound: i32,

    /// Java `viaUpperBound` — ctor `layerCount`.
    pub via_upper_bound: i32,

    /// Java `maxViaRadius` — the sweep max of [`Self::via_radii`].
    pub max_via_radius: f64,

    /// Java `tidyRegionWidth` — ctor `Integer.MAX_VALUE` (`:169`).
    pub tidy_region_width: i32,

    /// Java `pullTightAccuracy` — ctor 500 (`:170`).
    pub pull_tight_accuracy: i32,

    /// Java `maxShoveTraceRecursionDepth` — ctor 20 (`:171`).
    pub max_shove_trace_recursion_depth: i32,

    /// Java `maxShoveViaRecursionDepth` — ctor 5 (`:172`).
    pub max_shove_via_recursion_depth: i32,

    /// Java `maxSpringOverRecursionDepth` — ctor 5 (`:173`).
    pub max_spring_over_recursion_depth: i32,

    /// Java `minCheapViaCost` — `0.8 * min_normal_via_cost`.
    pub min_cheap_via_cost: f64,
}

impl AutorouteControl {
    /// Java `AutorouteControl(RoutingBoard, int, RouterSettings)`
    /// (`:117-120`): the private ctor body, then
    /// `initNet(netNumber, board, settings.getViaCosts())`.
    pub fn new(board: &mut Board, net_number: i32, settings: &RouterSettingsIr) -> Self {
        let layer_count = board.layers().layers.len();
        let mut control = Self {
            settings: settings.clone(),
            trace_costs: settings.trace_costs.clone(),
            bend_costs: vec![0.0; layer_count],
            with_neckdown: settings.automatic_neckdown,
            layer_active: vec![false; layer_count],
            layer_count,
            trace_half_width: vec![0; layer_count],
            compensated_trace_half_width: vec![0; layer_count],
            via_radii: vec![0.0; layer_count],
            add_via_costs: Vec::with_capacity(layer_count),
            trace_clearance_class_index: 0,
            vias_allowed: settings.vias_allowed,
            attach_smd_allowed: false,
            min_normal_via_cost: 0.0,
            ripup_allowed: false,
            ripup_costs: 1000,
            ripup_pass_no: 1,
            is_fanout: false,
            fanout_start_pin_name: None,
            fanout_start_pin_center: None,
            fanout_start_pin_layer: -1,
            remove_unconnected_vias: true,
            via_rule: None,
            net_number: 0,
            via_clearance_class: 0,
            via_infos: Vec::new(),
            via_lower_bound: 0,
            via_upper_bound: layer_count as i32,
            max_via_radius: 0.0,
            tidy_region_width: i32::MAX,
            pull_tight_accuracy: 500,
            max_shove_trace_recursion_depth: 20,
            max_shove_via_recursion_depth: 5,
            max_spring_over_recursion_depth: 5,
            min_cheap_via_cost: 0.0,
        };
        // Java ctor loop 1 (`:145-147`): bend costs per layer. The
        // settings array is expected layer-sized; a short IR degrades
        // to 0.0 (Java's getBendCost warns + returns 0.0 out of range).
        for (i, slot) in control.bend_costs.iter_mut().enumerate() {
            *slot = settings.bend_costs.get(i).copied().unwrap_or(0.0);
        }
        // Java ctor loop 2 (`:149-162`): one ViaCost per layer + the
        // layer-active init with the NON-SIGNAL FORCE-FALSE branch
        // (`:152-161`): a dedicated power plane cannot be routed — the
        // warn (FRLogger.warn, `:153-157`) has no Rust logger yet; the
        // behavioral content is the forced false. Contrast arm: the
        // same settings value on a signal layer passes through.
        for i in 0..layer_count {
            control.add_via_costs.push(ViaCost {
                to_layer: vec![0; layer_count],
            });
            let active_setting = settings.layer_active.get(i).copied().unwrap_or(false);
            let layer = &board.layers().layers[i];
            if !layer.is_signal && active_setting {
                control.layer_active[i] = false;
            } else {
                control.layer_active[i] = active_setting;
            }
        }
        // Java re-zeroes addViaCosts' toLayer in a nested loop
        // (`:174-178`) — a no-op on fresh arrays; the vec![0; …] above
        // is that constructor's zeroing.
        control.init_net(board, net_number, settings.via_costs);
        control
    }

    /// Java `initNet(netNumber, board, viaCosts)` (`:204-231`).
    fn init_net(&mut self, board: &mut Board, net_number: i32, via_costs: i32) {
        self.net_number = net_number;
        let rules = board.rules();
        let current_net = rules.nets.get(net_number);
        let current_net_class: Option<&NetClass> = current_net
            .map(|net| net.net_class)
            .and_then(|class_idx| rules.net_class(class_idx));
        // Java (`:208-216`): net found → the class's trace clearance
        // class + via rule; NULL-net fallback → class index 1 and the
        // FIRST via rule (`board.rules.viaRules.firstElement()`). The
        // fallback is OBSERVABLE and pinned (net 0 picks net 1's half
        // widths and the first rule, not the default class's).
        match current_net_class {
            Some(net_class) => {
                self.trace_clearance_class_index = net_class.trace_clearance_class;
                // NetClass.via_rule is the rule identity id; the M2-cut
                // `BoardRules::via_rule_by_id` resolves it. A class
                // with a `None` rule id (Java `getViaRule()` returning
                // null — unreachable from the parse, which gives every
                // class the default rule) falls back to the first rule
                // like the null-net arm; Java would NPE one line later
                // in rebuildViaInfo, so no parsed board distinguishes.
                self.via_rule = net_class
                    .via_rule
                    .and_then(|id| rules.via_rule_by_id(id))
                    .cloned()
                    .or_else(|| rules.via_rules.first().cloned());
            }
            None => {
                self.trace_clearance_class_index = 1;
                self.via_rule = rules.via_rules.first().cloned();
            }
        }
        for i in 0..self.layer_count as i32 {
            // Java (`:218-222`): `getTraceHalfWidth(netNumber > 0 ?
            // netNumber : 1, i)` — the fallback to net 1 is observable.
            let width = rules.trace_half_width(if net_number > 0 { net_number } else { 1 }, i);
            self.trace_half_width[i as usize] = width;
            self.compensated_trace_half_width[i as usize] = width
                + rules
                    .clearance
                    .clearance_compensation_value(self.trace_clearance_class_index, i);
            // Java (`:226-228`): the M2-cut
            // `NetClass::is_active_routing_layer` consumer — a net
            // class that marks a layer inactive force-disables it.
            if current_net_class.is_some_and(|net_class| !net_class.is_active_routing_layer(i)) {
                self.layer_active[i as usize] = false;
            }
        }
        self.rebuild_via_info(board, via_costs, net_number);
    }

    /// Java `isPureSmdNet(board, netNumber)` (`:189-202`): every
    /// connectable item of the net is a single-layer Pin. Empty nets
    /// are NOT pure.
    fn is_pure_smd_net(board: &mut Board, net_number: i32) -> bool {
        let net_items = board.get_connectable_items(net_number);
        if net_items.is_empty() {
            return false;
        }
        for id in net_items {
            let is_pin = board
                .get(id)
                .is_some_and(|entry| entry.board_item_type() == BoardItemType::Pin);
            if !is_pin {
                return false;
            }
            let first = board.drill_first_layer(id);
            if first.is_none() || first != board.drill_last_layer(id) {
                return false;
            }
        }
        true
    }

    /// Java `rebuildViaInfo(board, viaCosts, netNumber)` (`:234-284`).
    /// Also the re-entry point for the driver's ripup passes (T12) —
    /// the via radii accumulate by max across calls, exactly like
    /// Java's (no zeroing here).
    pub fn rebuild_via_info(&mut self, board: &mut Board, via_costs: i32, net_number: i32) {
        // Java (`:235-239`): the clearance class of via[0], else 1.
        let rule_vias: Vec<i32> = self
            .via_rule
            .as_ref()
            .map_or_else(Vec::new, |rule| rule.via_infos.clone());
        let rules = board.rules();
        self.via_clearance_class = rule_vias
            .first()
            .and_then(|idx| rules.via_infos.get(*idx as usize))
            .map_or(1, |info| info.clearance_class);
        // Java (`:241-261`): rebuild the masks; attach is ANY-of; the
        // padstack shapes raise the per-layer radii (0.5 × maxWidth,
        // 0 for a null shape slot).
        self.via_infos = Vec::with_capacity(rule_vias.len());
        self.attach_smd_allowed = false;
        for via_idx in &rule_vias {
            let Some(info) = rules.via_infos.get(*via_idx as usize) else {
                // Java indexes directly (an unresolved member is a
                // parse-time impossibility — addViaRule resolved every
                // member); skipping keeps the Rust port total.
                continue;
            };
            if info.attach_smd_allowed {
                self.attach_smd_allowed = true;
            }
            // ViaInfo.padstack_no is 1-based into the board library.
            let padstack = board
                .library()
                .padstacks
                .get(info.padstack_no.unsigned_abs() as usize - 1);
            let (from_layer, to_layer) = padstack.map_or((0, -1), |padstack| {
                (
                    i32::try_from(padstack.from_layer()).unwrap_or(0),
                    padstack.to_layer(),
                )
            });
            if to_layer >= 0 {
                for j in from_layer..=to_layer {
                    let Some(radius_slot) = self.via_radii.get_mut(j as usize) else {
                        // Java would AIOOBE (unreachable: padstack
                        // spans are board-layer sized).
                        continue;
                    };
                    let current_radius = padstack
                        .and_then(|padstack| padstack.get_shape(j as usize))
                        .map_or(0.0, |shape| 0.5 * shape_max_width(shape));
                    *radius_slot = (*radius_slot).max(current_radius);
                }
            }
            self.via_infos.push(ViaMask {
                from_layer,
                to_layer,
                attach_smd_allowed: info.attach_smd_allowed,
            });
        }
        // The rules borrow ended with the loop above (NLL); the
        // pure-SMD test below needs `&mut`.
        // Java (`:263-269`): THE PURE-SMD RELAXATION. THREE conditions
        // (`!attachSmdAllowed && layerCount > 1 && pureSmdNet`), ONE
        // effect here (attach forced true). It relaxes the same-net
        // escape gate only; cross-net DRC remains governed by the
        // padstack/via-info attach flag. The SECOND effect — the cost
        // ×0.1 below — is guarded by `pureSmdNet` ALONE (`:277-281`),
        // so a rule that already allows attach still gets the cheap
        // escape (pinned).
        let pure_smd_net = Self::is_pure_smd_net(board, net_number);
        if !self.attach_smd_allowed && self.layer_count > 1 && pure_smd_net {
            self.attach_smd_allowed = true;
        }
        // Java (`:271-283`): the radii floor at the trace half width,
        // the max sweep, the cost factor and the ×0.1 PURE-SMD arm.
        for j in 0..self.layer_count {
            self.via_radii[j] = self.via_radii[j].max(f64::from(self.trace_half_width[j]));
            self.max_via_radius = self.max_via_radius.max(self.via_radii[j]);
        }
        let mut via_cost_factor = self.max_via_radius.max(1.0);
        if pure_smd_net {
            via_cost_factor *= 0.1;
        }
        self.min_normal_via_cost = f64::from(via_costs) * via_cost_factor;
        self.min_cheap_via_cost = 0.8 * self.min_normal_via_cost;
    }
}

/// Java `ConvexShape.maxWidth()` through [`BoardShape`] — the twin of
/// epic-dsn's parse-side `board_shape_max_width` (private there; the
/// board crate owns the shape type). `TileShape`/`Circle` implement
/// maxWidth natively; `PolygonShape` is NOT a Java `ConvexShape`
/// (padstack shapes never carry it — the parse converts), so the
/// bounding-box fallback is defensive only.
fn shape_max_width(shape: &BoardShape) -> f64 {
    match shape {
        BoardShape::Tile(tile) => tile.max_width(),
        BoardShape::PolygonShape(polygon) => polygon.bounding_box().max_width(),
        BoardShape::Circle(circle) => circle.max_width(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{net_no, parse};
    use epic_board::id::ItemId;

    /// The crafted control board — 3 layers (F.Cu signal, B.Cu signal,
    /// GND POWER — the force-false witness), five net classes on one
    /// shared via table:
    ///
    /// * class `W300` on net MINE — `(use_layer F.Cu)` → B.Cu/GND
    ///   inactive, half widths `[300, 0, 0]` (class rule `(width 600)`).
    /// * class `W700` on net OTHER — no use_layer → `[700, 700, 700]`,
    ///   all layers active. THE layer-force CONTRAST arm.
    /// * class `SMDCLS` on net SMDNET — both CMP1 pins are single-layer
    ///   PAD_SMD → the PURE-SMD arm; rule via `R1` (no attach) → the
    ///   attach FORCE fires and the cost gets the ×0.1.
    /// * class `THCLS` on net THNET — CMP2 pins are 2-layer PAD_TH →
    ///   the NOT-pure arm (a pin whose span is two layers).
    /// * class `ATTACHCLS` on net ATTACHNET — pure SMD AND rule via
    ///   `R2` (attach) → the ISOLATION arm: attach was already allowed,
    ///   yet the ×0.1 STILL fires (its guard is `pureSmdNet` alone).
    /// * net MINE also carries one F.Cu trace (a non-Pin item on a
    ///   routed net — the second not-pure flavor).
    /// * nets NET_A / NET_B re-declare CMP1-P1 → the multi-net pin for
    ///   the epic-board union branch tests (not consumed here).
    ///
    /// The via table: `(via V1 PAD_VIA default)` (no attach) and
    /// `(via V2 PAD_VIA default attach)`; rules `R1 = [V1]`,
    /// `R2 = [V2]`. PAD_VIA is an 800-DIAMETER circle on F.Cu + B.Cu (the DSN circle
    /// token is the diameter) → per-via radius `0.5 × maxWidth = 400.0`,
    /// no GND shape.
    const CONTROL_DSN: &str = "\
(pcb epic-router-control.dsn\n\
  (parser\n\
    (string_quote \")\n\
    (space_in_quoted_tokens on)\n\
  )\n\
  (resolution um 1)\n\
  (unit um)\n\
  (structure\n\
    (layer F.Cu (type signal))\n\
    (layer B.Cu (type signal))\n\
    (layer GND (type power))\n\
    (boundary (rect pcb 0 0 120000 60000))\n\
    (rule (width 200) (clearance 200))\n\
  )\n\
  (placement\n\
    (component CMP1\n\
      (place CMP1 20000 40000 front 0)\n\
    )\n\
    (component CMP2\n\
      (place CMP2 60000 40000 front 0)\n\
    )\n\
    (component CMP3\n\
      (place CMP3 90000 40000 front 0)\n\
    )\n\
  )\n\
  (library\n\
    (padstack PAD_SMD\n\
      (shape (circle F.Cu 600 0 0))\n\
    )\n\
    (padstack PAD_TH\n\
      (shape (circle F.Cu 500 0 0))\n\
      (shape (circle B.Cu 500 0 0))\n\
    )\n\
    (padstack PAD_VIA\n\
      (shape (circle F.Cu 800 0 0))\n\
      (shape (circle B.Cu 800 0 0))\n\
    )\n\
    (image CMP1\n\
      (pin PAD_SMD P1 0 0)\n\
      (pin PAD_SMD P2 20000 0)\n\
    )\n\
    (image CMP2\n\
      (pin PAD_TH P1 0 0)\n\
      (pin PAD_TH P2 20000 0)\n\
    )\n\
    (image CMP3\n\
      (pin PAD_SMD P1 0 0)\n\
      (pin PAD_SMD P2 20000 0)\n\
    )\n\
  )\n\
  (network\n\
    (via V1 PAD_VIA default)\n\
    (via V2 PAD_VIA default attach)\n\
    (via_rule R1 V1)\n\
    (via_rule R2 V2)\n\
    (net MINE)\n\
    (net OTHER)\n\
    (net SMDNET (pins CMP1-P1 CMP1-P2))\n\
    (net THNET (pins CMP2-P1 CMP2-P2))\n\
    (net ATTACHNET (pins CMP3-P1 CMP3-P2))\n\
    (net NET_A (pins CMP1-P1))\n\
    (net NET_B (pins CMP1-P1))\n\
    (class W300 MINE\n\
      (clearance_class default)\n\
      (via_rule R1)\n\
      (rule (width 600))\n\
      (circuit (use_layer F.Cu))\n\
    )\n\
    (class W700 OTHER\n\
      (clearance_class default)\n\
      (via_rule R1)\n\
      (rule (width 1400))\n\
    )\n\
    (class SMDCLS SMDNET\n\
      (clearance_class default)\n\
      (via_rule R1)\n\
      (rule (width 200))\n\
    )\n\
    (class THCLS THNET\n\
      (clearance_class default)\n\
      (via_rule R1)\n\
      (rule (width 200))\n\
    )\n\
    (class ATTACHCLS ATTACHNET\n\
      (clearance_class default)\n\
      (via_rule R2)\n\
      (rule (width 200))\n\
    )\n\
  )\n\
  (wiring\n\
    (wire (path F.Cu 200  20000 40000  30000 40000) (net MINE))\n\
  )\n\
)\n";

    /// All-true layer activity — the ctor force-false arm needs the
    /// settings to SAY true on the power layer.
    fn settings(layer_active: Vec<bool>) -> RouterSettingsIr {
        RouterSettingsIr {
            trace_costs: vec![
                ExpansionCostFactor {
                    horizontal: 2.0,
                    vertical: 3.0,
                };
                3
            ],
            via_costs: 1,
            vias_allowed: true,
            bend_costs: vec![1.0, 2.0, 3.0],
            layer_active,
            automatic_neckdown: true,
        }
    }

    fn control_for(board: &mut Board, net: i32, layer_active: Vec<bool>) -> AutorouteControl {
        AutorouteControl::new(board, net, &settings(layer_active))
    }

    /// Ctor constants (`AutorouteControl.java:163-186`) and the
    /// settings copy-in — the literal rows of the scaffold.
    #[test]
    fn ctor_constants_and_settings_copy() {
        let (_manager, mut board) = parse(CONTROL_DSN);
        let mine = net_no(&board, "MINE");
        let control = control_for(&mut board, mine, vec![true, true, true]);
        assert_eq!(control.layer_count, 3);
        assert_eq!(control.bend_costs, vec![1.0, 2.0, 3.0]);
        assert!(control.with_neckdown);
        assert!(control.vias_allowed);
        assert_eq!(
            control.trace_costs,
            vec![
                ExpansionCostFactor {
                    horizontal: 2.0,
                    vertical: 3.0
                };
                3
            ]
        );
        assert!(!control.ripup_allowed);
        assert_eq!(control.ripup_costs, 1000);
        assert_eq!(control.ripup_pass_no, 1);
        assert!(!control.is_fanout);
        assert_eq!(control.fanout_start_pin_name, None);
        assert_eq!(control.fanout_start_pin_center, None);
        assert_eq!(control.fanout_start_pin_layer, -1);
        assert!(control.remove_unconnected_vias);
        assert_eq!(control.tidy_region_width, i32::MAX);
        assert_eq!(control.pull_tight_accuracy, 500);
        assert_eq!(control.max_shove_trace_recursion_depth, 20);
        assert_eq!(control.max_shove_via_recursion_depth, 5);
        assert_eq!(control.max_spring_over_recursion_depth, 5);
        assert_eq!(control.via_lower_bound, 0);
        assert_eq!(control.via_upper_bound, 3);
        assert_eq!(control.add_via_costs.len(), 3);
        for via_cost in &control.add_via_costs {
            assert_eq!(via_cost.to_layer, vec![0, 0, 0]);
        }
    }

    /// THE LAYER-ACTIVE FORCES, both sources, on one board with the
    /// SAME all-true settings:
    ///
    /// * MINE (class W300, `(use_layer F.Cu)`): `[true, false, false]`
    ///   — F.Cu passes through; B.Cu is force-disabled by the NET
    ///   CLASS (`is_active_routing_layer`, the M2-cut accessor); GND
    ///   is force-disabled by the CTOR (non-signal layer — the
    ///   `AutorouteControl.java:152-161` branch).
    /// * OTHER (class W700, no use_layer): `[true, true, false]` —
    ///   B.Cu STAYS active under identical settings, isolating the
    ///   class arm to W300; GND is false on BOTH — and settings say
    ///   TRUE there, so a mutant dropping the `!is_signal` check
    ///   yields `true` on GND and fails this pin (the settings value
    ///   passing through is the contrast half of the witness).
    #[test]
    fn layer_active_force_false_both_sources() {
        let (_manager, mut board) = parse(CONTROL_DSN);
        let mine = net_no(&board, "MINE");
        let other = net_no(&board, "OTHER");
        let mine_control = control_for(&mut board, mine, vec![true, true, true]);
        let other_control = control_for(&mut board, other, vec![true, true, true]);
        assert_eq!(mine_control.layer_active, vec![true, false, false]);
        assert_eq!(other_control.layer_active, vec![true, true, false]);

        // Settings false stays false on every layer (the pass-through
        // arm for the ctor branch — no force needed, no warn in Java).
        let quiet = control_for(&mut board, other, vec![false, true, true]);
        assert_eq!(quiet.layer_active, vec![false, true, false]);
    }

    /// Half widths: the class rule widths (`dsnToBoard(w) / 2` at the
    /// craft's um-1 resolution → identity) and the `use_layer` zeroing.
    /// Compensation is NON-ZERO on this craft: the `(clearance 200)`
    /// rule fills the matrix diagonal, and
    /// `clearanceCompensationValue = (diag + 1) / 2 = 100` per layer —
    /// so `compensatedTraceHalfWidth` is the width row plus 100
    /// (`AutorouteControl.java:223-225`).
    /// THE NET-0 FALLBACK: `getTraceHalfWidth(netNumber > 0 ?
    /// netNumber : 1, i)` — control(net 0) reads NET 1's widths
    /// (MINE's `[300, 0, 0]`), NOT OTHER's `[700, …]` and NOT the
    /// default class's `[100, …]`; its clearance class is the literal
    /// fallback 1 and its via rule is the FIRST rule (R1) — the
    /// `viaRules.firstElement()` arm.
    #[test]
    fn half_widths_and_net0_fallback() {
        let (_manager, mut board) = parse(CONTROL_DSN);
        let mine = net_no(&board, "MINE");
        let other = net_no(&board, "OTHER");

        let mine_control = control_for(&mut board, mine, vec![true, true, true]);
        assert_eq!(mine_control.trace_half_width, vec![300, 0, 0]);
        assert_eq!(
            mine_control.compensated_trace_half_width,
            vec![400, 100, 100]
        );

        let other_control = control_for(&mut board, other, vec![true, true, true]);
        assert_eq!(other_control.trace_half_width, vec![700, 700, 700]);
        assert_eq!(
            other_control.compensated_trace_half_width,
            vec![800, 800, 800]
        );

        let zero_control = control_for(&mut board, 0, vec![true, true, true]);
        assert_eq!(zero_control.trace_half_width, vec![300, 0, 0]);
        assert_eq!(
            zero_control.compensated_trace_half_width,
            vec![400, 100, 100]
        );
        assert_eq!(zero_control.trace_clearance_class_index, 1);
        assert_eq!(
            zero_control.via_rule.as_ref().expect("fallback rule").name,
            "R1"
        );
    }

    /// THE PURE-SMD RELAXATION, force arm (SMDNET): the rule's only
    /// via `V1` carries no attach, so `attachSmdAllowed` starts false;
    /// every net item is a single-layer pin → the three conditions
    /// (`!attach && layerCount > 1 && pureSmdNet`) hold and attach is
    /// FORCED true (`AutorouteControl.java:263-269`), while the cost
    /// gets the ×0.1 (`:277-281`): factor `max(800, 1) × 0.1 = 80`.
    /// The via radii: PAD_VIA's F.Cu/B.Cu circles give 800.0 on layers
    /// 0-1; the GND slot has no shape and floors at the trace half
    /// width 100 (`:271-274`). The masks: one `ViaMask{0, 1, false}`.
    #[test]
    fn pure_smd_net_forces_attach_and_halves_cost() {
        let (_manager, mut board) = parse(CONTROL_DSN);
        let smd = net_no(&board, "SMDNET");
        let control = control_for(&mut board, smd, vec![true, true, true]);
        assert!(control.attach_smd_allowed, "the forced arm");
        assert_eq!(control.via_radii, vec![400.0, 400.0, 100.0]);
        assert_eq!(control.max_via_radius, 400.0);
        assert_eq!(control.min_normal_via_cost, 40.0);
        assert_eq!(control.min_cheap_via_cost, 32.0);
        assert_eq!(
            control.via_infos,
            vec![ViaMask {
                from_layer: 0,
                to_layer: 1,
                attach_smd_allowed: false
            }]
        );
    }

    /// THE ISOLATION ARM (ATTACHNET): the rule's via `V2` already
    /// allows attach, so the force branch is a no-op here — yet the
    /// cost STILL gets the ×0.1, because its guard is `pureSmdNet`
    /// ALONE (`:277-281`). A mutant tying the ×0.1 to the force
    /// condition (`!attach_smd_allowed && pure_smd_net`) suppresses
    /// the ×0.1 and yields `min_normal_via_cost` 400.0 here (factor
    /// `max(maxViaRadius, 1) = 400`, full) instead of 40.0 — and the
    /// same 400.0 in [`pure_smd_net_forces_attach_and_halves_cost`]
    /// (the forcing runs BEFORE the cost block, flipping attach true)
    /// — these rows are what kill it.
    #[test]
    fn attach_already_allowed_still_gets_cheap_escape() {
        let (_manager, mut board) = parse(CONTROL_DSN);
        let attach = net_no(&board, "ATTACHNET");
        let control = control_for(&mut board, attach, vec![true, true, true]);
        assert!(control.attach_smd_allowed);
        assert_eq!(
            control.via_infos,
            vec![ViaMask {
                from_layer: 0,
                to_layer: 1,
                attach_smd_allowed: true
            }]
        );
        assert_eq!(control.min_normal_via_cost, 40.0);
        assert_eq!(control.min_cheap_via_cost, 32.0);
    }

    /// THE NOT-PURE ARMS: MINE carries a TRACE (a non-Pin item) and
    /// THNET's pins span TWO layers — both fail `isPureSmdNet`, so no
    /// force fires (rule `R1` has no attach → stays FALSE — the
    /// contrast half of the force witness) and the cost keeps the full
    /// factor 800 (no ×0.1). W300's `use_layer` zeroing makes MINE's
    /// GND radius floor 0 here.
    #[test]
    fn non_pure_nets_keep_attach_off_and_full_cost() {
        let (_manager, mut board) = parse(CONTROL_DSN);
        let mine = net_no(&board, "MINE");
        let th = net_no(&board, "THNET");
        let mine_control = control_for(&mut board, mine, vec![true, true, true]);
        let th_control = control_for(&mut board, th, vec![true, true, true]);
        assert!(!mine_control.attach_smd_allowed);
        assert_eq!(mine_control.min_normal_via_cost, 400.0);
        assert_eq!(mine_control.min_cheap_via_cost, 320.0);
        assert_eq!(mine_control.via_radii, vec![400.0, 400.0, 0.0]);
        assert!(!th_control.attach_smd_allowed);
        assert_eq!(th_control.min_normal_via_cost, 400.0);
    }

    /// The driver re-entry seam (T12): `rebuildViaInfo` recomputes the
    /// costs from the (persistent, max-accumulating) radii — a second
    /// call with `viaCosts = 2` doubles both cost rows on the SAME
    /// control. Java's driver owns the `ripupCosts` FORMULA
    /// (`getStartRipupCosts() * ripupPassNo`,
    /// `AutorouteConnectionRouter.java:46`); T3 ports the field only.
    #[test]
    fn rebuild_via_info_reentry_recomputes_costs() {
        let (_manager, mut board) = parse(CONTROL_DSN);
        let smd = net_no(&board, "SMDNET");
        let mut control = control_for(&mut board, smd, vec![true, true, true]);
        assert_eq!(control.min_normal_via_cost, 40.0);
        control.rebuild_via_info(&mut board, 2, smd);
        assert_eq!(
            control.via_radii,
            vec![400.0, 400.0, 100.0],
            "no re-zeroing"
        );
        assert_eq!(control.min_normal_via_cost, 80.0);
        assert_eq!(control.min_cheap_via_cost, 64.0);
    }

    /// The clearance-class resolution of the via table (`via[0]`,
    /// `AutorouteControl.java:235-239`): both crafted vias name the
    /// matrix class `default`, which the parse maps to matrix
    /// class 1 — the same value the NULL-net fallback hardcodes, so
    /// the literal doubles as the fallback-shape witness.
    #[test]
    fn via_clearance_class_from_first_via() {
        let (_manager, mut board) = parse(CONTROL_DSN);
        let smd = net_no(&board, "SMDNET");
        let control = control_for(&mut board, smd, vec![true, true, true]);
        assert_eq!(control.via_clearance_class, 1);
    }

    /// Mode-8 lookup guard: the multi-net pin CMP1-P1 carries exactly
    /// nets [SMDNET, NET_A, NET_B] — the epic-board union-branch tests
    /// (contacts.rs) rely on the same craft shape.
    #[test]
    fn multi_net_pin_carries_three_nets() {
        let (_manager, board) = parse(CONTROL_DSN);
        let smd = net_no(&board, "SMDNET");
        let net_a = net_no(&board, "NET_A");
        let net_b = net_no(&board, "NET_B");
        let pin: Vec<ItemId> = board
            .iter_descending()
            .filter(|entry| {
                entry.board_item_type() == epic_board::items::BoardItemType::Pin
                    && entry.nets.contains(&net_a)
            })
            .map(|entry| entry.id)
            .collect();
        assert_eq!(pin.len(), 1);
        let mut nets = board.get(pin[0]).expect("pin").nets.clone();
        nets.sort();
        assert_eq!(nets, vec![smd, net_a, net_b]);
    }
}
