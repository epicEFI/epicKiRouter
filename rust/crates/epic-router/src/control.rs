//! Java `autoroute/maze/AutorouteControl.java` — the per-net cost table
//! the maze engine consumes. Ported at M3-T3 as an owned struct built
//! from `(board, net number, settings IR)`; every field of the Java
//! class has a counterpart, and the two mutation seams (`init_net`,
//! `rebuild_via_info`) are 1:1. The PASS DRIVER is NOT ported here —
//! notably the ripup-cost formula application (see [`AutorouteControl::ripup_costs`]).

use epic_board::board::Board;
use epic_board::items::{BoardItemType, shape_max_width};
use epic_board::rules_surf::{NetClass, ViaRule};

/// Java `RouterSettings` — the fields `AutorouteControl` consumes, and
/// ONLY those (the real settings merger/resolution is T13, epic-cli;
/// this IR is construction-site agnostic — T13 builds it, the control
/// only reads it). Java resolves defaults inside the getters
/// (`getViasAllowed` null → `true`, `getViaCosts` null → 1,
/// `getAutomaticNeckdown` null → `false`, `getBendCost` null → 0.0,
/// `getLayerActive` null → `true`); here the RESOLVER owns defaults,
/// so every field is a plain resolved value.
/// Deliberately NO `Default` impl (M3-T3 quality round, minor 1): a
/// second defaults source beside the resolver duplicates the Java
/// settings-merger trap — defaults live in exactly one place (the
/// `settings()` test builder here; in production, the layered
/// `SettingsMerger` sources).
#[derive(Clone, Debug, PartialEq)]
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
    /// Java `getAutomaticNeckdown()` (`:896-898`). Default TRUE —
    /// `DefaultSettings.java:157/:164` seeds `automaticNeckdown =
    /// true` (verified at the graft baseline e7f9bdf1 AND at
    /// aa909a345^); `false` is only the GUI Workspace checkbox's
    /// persisted default. M11-T9i's widened micro-neckdown gate
    /// (`is_fanout || with_neckdown`) is therefore LIVE at headless
    /// defaults.
    pub automatic_neckdown: bool,
    /// Java `getStartRipupCosts()` (`RouterSettings.java:540-542`,
    /// default 1) — the minimum ripup cost. `MazeRipupResolver
    /// .checkRipup:98-99` gates fanout protection on
    /// `ctrl.ripupCosts <= startRipupCosts * 2`, so the protection
    /// window shrinks as the pass cost grows.
    pub start_ripup_costs: i32,
    /// Java `RouterSettings.fanout` (`FanoutSettings`) — the fanout
    /// stage group. Java holds the group nullable and the maze
    /// frontier gate tests `ctrl.settings.fanout != null` before the
    /// field reads (`MazeSearchEngine.java:104`); the resolver always
    /// materializes the group over `DefaultSettings`, so the port's IR
    /// carries it non-optional and the group-null arm is banked
    /// (unreachable from the standard flow, where DefaultSettings
    /// seeds the group). The per-field null faces (escape lengths,
    /// timeout) stay `Option`.
    pub fanout: FanoutSettingsIr,
}

/// Java `settings.FanoutSettings` — the fanout stage's settings group,
/// RESOLVED (the CLI resolver owns the null-to-default faces; see the
/// [`RouterSettingsIr`] doc). Every field mirrors the Java field of the
/// same snake_case name; the Option fields are the ones Java keeps
/// genuinely nullable AT THE CONSUMPTION SITE (the escape lengths, where
/// `MazeSearchEngine`'s frontier gate falls back to raw coordinate
/// values on null — `MazeSearchEngine.java:107-112/:116-121`), and
/// `timeout_string` (null = no stage deadline).
///
/// The [`Default`] impl IS the `DefaultSettings.java:163-177` fanout
/// face — one statement of the resolved defaults, shared by the test
/// construction sites and the CLI resolver's base. Do NOT state these
/// values anywhere else (the settings-merger trap; CLAUDE.md).
#[derive(Clone, Debug, PartialEq)]
pub struct FanoutSettingsIr {
    /// Java `enabled` — resolved (`isFanoutEnabled()`); DefaultSettings true.
    pub enabled: bool,
    /// Java `maxPasses` — resolved; DefaultSettings 20 (and the Java
    /// consumption site carries the same inline default for null).
    pub max_passes: i32,
    /// Java `maxItems` — resolved; DefaultSettings `Integer.MAX_VALUE`.
    /// The stage's cap gate ALSO requires `maxItems > 0`
    /// (`BatchFanout.java:117-122` between passes, `:222-230` within a
    /// pass), so non-positive disables the cap.
    pub max_items: i32,
    /// Java `maxMillisecondsPerPin` — resolved; DefaultSettings 10000.
    /// Wall clock in Java; behind `deterministic_budgets` the same
    /// value (times `passNo + 1`) is spent as `RouteBudget` ticks.
    pub max_milliseconds_per_pin: i64,
    /// Java `ripupAllowed` — resolved; DefaultSettings true. False sends
    /// `-1` ripup costs to the per-pin engine call (the "no ripup" signal).
    pub ripup_allowed: bool,
    /// Java `minEscapeLengthMm` — DefaultSettings 2.5; `None` keeps the
    /// engine's 500.0-coordinate fallback arm live.
    pub min_escape_length_mm: Option<f64>,
    /// Java `maxEscapeLengthMm` — DefaultSettings 4.5; `None` keeps the
    /// engine's 3000.0-coordinate fallback arm live.
    pub max_escape_length_mm: Option<f64>,
    /// Java `startViaDiameterMm` — DefaultSettings 0.250. Settings-surface
    /// parity only: no engine consumer (banked).
    pub start_via_diameter_mm: Option<f64>,
    /// Java `endViaDiameterMm` — DefaultSettings 0.250. Settings-surface
    /// parity only: no engine consumer (banked).
    pub end_via_diameter_mm: Option<f64>,
    /// Java `pinSortingOrder` — the RAW string, kept verbatim because
    /// Java dispatches on it with four known values
    /// (`inner_first`/`outer_first`/`distanceToClosestOnNet`/
    /// `surroundingsDensity`) and treats ANY OTHER value as "compare
    /// equal, fall through to the `pinIndex` tie-break"
    /// (`BatchFanout.java:764-792` dispatch, `:793-795` tie-break).
    /// DefaultSettings "outer_first".
    pub pin_sorting_order: String,
    /// Java `fallbackToBoardVias` — resolved; DefaultSettings true.
    pub fallback_to_board_vias: bool,
    /// Java `timeout` (`SerializedName("timeout")`) — the stage wall
    /// deadline (`TextManager.parseTimespanString`); DefaultSettings
    /// leaves it null. Port: only the wall-budget profile consults the
    /// parsed deadline (the deterministic profile has no wall to time
    /// out — the banked job-timeout seam).
    pub timeout_string: Option<String>,
}

impl Default for FanoutSettingsIr {
    /// `DefaultSettings.java:163-177` — the resolved fanout defaults,
    /// verbatim (the single statement of these values; see the struct
    /// doc). A derived `Default` would seed the zero faces here —
    /// `max_passes: 0` is ZERO PASSES and `pin_sorting_order: ""` is
    /// the pinIndex tie-break — so the impl is manual by design.
    fn default() -> Self {
        Self {
            enabled: true,
            max_passes: 20,
            max_items: i32::MAX,
            max_milliseconds_per_pin: 10_000,
            ripup_allowed: true,
            min_escape_length_mm: Some(2.5),
            max_escape_length_mm: Some(4.5),
            start_via_diameter_mm: Some(0.250),
            end_via_diameter_mm: Some(0.250),
            pin_sorting_order: "outer_first".to_string(),
            fallback_to_board_vias: true,
            timeout_string: None,
        }
    }
}

/// Java `AngleRestriction` (`board.rules.BasicsOption`) — the trace
/// angle mode; read by `MazeSearchEngine.doorIsSmall` (`:773-784`) to
/// pick the door-length measure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AngleRestriction {
    /// Java `AngleRestriction.NONE`.
    None,
    /// Java `AngleRestriction.NINETY_DEGREE`.
    NinetyDegree,
    /// Java `AngleRestriction.FORTYFIVE_DEGREE`.
    FortyfiveDegree,
}

/// Java `AutorouteControl.ExpansionCostFactor` (record,
/// `AutorouteControl.java:286-287`): horizontal and vertical costs for
/// traces on one board layer.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct ExpansionCostFactor {
    pub horizontal: f64,
    pub vertical: f64,
}

/// The mirror conversion into the epic-board [`TraceCostFactor`] the
/// pull-tight seam consumes (Java `AutorouteControl.ExpansionCostFactor`
/// IS the type `optChangedArea` takes; the port's board crate carries
/// its own copy to stay router-crate-free).
impl From<&ExpansionCostFactor> for epic_board::trace_tightener::TraceCostFactor {
    fn from(f: &ExpansionCostFactor) -> Self {
        epic_board::trace_tightener::TraceCostFactor {
            horizontal: f.horizontal,
            vertical: f.vertical,
        }
    }
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

    /// M6-T9 RUST-ONLY (`router.push_shove`, default false — no Java
    /// field): when true, the maze expansion may WAIVE the rip-up
    /// charge of an obstacle room the shove probe has verified
    /// shovable, within the per-search shove budget
    /// (`maze/ripup.rs::PUSH_SHOVE_ROOM_BUDGET`), so the detail
    /// insertion displaces the neighbor trace instead of ripping it.
    /// The OFF path never consults it (byte-identical decisions).
    pub push_shove: bool,

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

    /// M7-T6 RUST-ONLY: the differential-pair COUPLING preference
    /// (`pipeline/pairs.rs`; no Java counterpart — the improvement
    /// face, Default absent). `Some` only while routing a declared
    /// pair's FOLLOWER whose leader already has routed copper: the
    /// maze's cost model then discounts the follower's in-corridor
    /// steps (`search_engine.rs::expand_to_door_section`). The `None`
    /// path never consults the term — the cost face is bit-identical
    /// (the two-regime discipline).
    pub coupling: Option<crate::pipeline::pairs::CouplingPreference>,
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
            push_shove: false,
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
            coupling: None,
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

    /// Java ctor 5-arg overload
    /// `AutorouteControl(RoutingBoard, int, RouterSettings, int viaCosts,
    /// ExpansionCostFactor[] traceCosts)` (`AutorouteControl.java:122-128`):
    /// the shared ctor body with the CALLER's trace-cost table, then
    /// `initNet(netNumber, board, viaCosts)` with the caller's via
    /// cost. The batch driver resolves both once per run
    /// (`BatchDriver::trace_costs`: the settings table when
    /// preferred directions are kept, the per-layer `(preferred,
    /// preferred)` flattening otherwise — BatchAutorouter.java:138-149)
    /// and the connection router passes
    /// `plane ? planeViaCosts : viaCosts` per net
    /// (`AutorouteConnectionRouter.java:37-46`).
    pub fn new_with_costs(
        board: &mut Board,
        net_number: i32,
        settings: &RouterSettingsIr,
        via_costs: i32,
        trace_costs: &[ExpansionCostFactor],
    ) -> Self {
        let mut control = Self::new(board, net_number, settings);
        control.trace_costs = trace_costs.to_vec();
        // Re-run the net init with the caller's via costs (Java would
        // have used them from the start; `init_net` is idempotent in
        // its non-via fields, so the only double-run residue is a
        // repeated warn on a power-plane layer — log-only).
        control.init_net(board, net_number, via_costs);
        control
    }

    /// The BatchAutorouter trace-cost resolution
    /// (`BatchAutorouter.java:138-149`): keep the settings table, or
    /// flatten every layer to its preferred-direction cost (the
    /// remove-preferred-direction mode). Java reads
    /// `settings.getPreferredDirectionTraceCosts(i)` — the raw
    /// PREFERRED array value, which by construction
    /// (applyBoardSpecificOptimizations) never exceeds the
    /// against-direction cost, so `min(horizontal, vertical)` is the
    /// same value for every reachable settings state (an inverted
    /// user table is not a driver-reachable state; banked in SEAM).
    #[must_use]
    pub fn resolve_trace_costs(
        settings: &RouterSettingsIr,
        with_preferred_directions: bool,
    ) -> Vec<ExpansionCostFactor> {
        if with_preferred_directions {
            return settings.trace_costs.clone();
        }
        settings
            .trace_costs
            .iter()
            .map(|factor| {
                let min = factor.horizontal.min(factor.vertical);
                ExpansionCostFactor {
                    horizontal: min,
                    vertical: min,
                }
            })
            .collect()
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
            // ViaInfo.padstack_no is 1-based into the board library
            // (Java reads `padstacks[no - 1]` directly). A non-positive
            // number is a parse-time impossibility — fail LOUDLY in
            // every profile instead of silently wrapping (the previous
            // `unsigned_abs() as usize - 1` mapped padstack_no 0 onto
            // the LAST library slot and panicked on i32::MIN).
            assert!(
                info.padstack_no >= 1,
                "via padstack_no {} out of range (1-based library index)",
                info.padstack_no
            );
            let padstack_no = usize::try_from(info.padstack_no).expect("checked >= 1 above");
            let padstack = board.library().padstacks.get(padstack_no - 1);
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
    (layer GND (type power) (use_net GNDPLANE))\n\
    (boundary (rect pcb 0 0 120000 60000))\n\
    (rule (width 200) (clearance 200))\n\
    (rule (clearance 400 (type THICK-THICK)))\n\
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
    (via V3 PAD_VIA THICK)\n\
    (via_rule R1 V1)\n\
    (via_rule R2 V2)\n\
    (via_rule R3 V3)\n\
    (via_rule R4 V3 V1)\n\
    (via_rule R5 V1 V3)\n\
    (net MINE)\n\
    (net OTHER)\n\
    (net SMDNET (pins CMP1-P1 CMP1-P2))\n\
    (net THNET (pins CMP2-P1 CMP2-P2))\n\
    (net ATTACHNET (pins CMP3-P1 CMP3-P2))\n\
    (net NET_A (pins CMP1-P1))\n\
    (net NET_B (pins CMP1-P1))\n\
    (net THICKNET)\n\
    (net VIANET)\n\
    (net VIANET2)\n\
    (net GNDPLANE)\n\
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
    (class THICKCLS THICKNET\n\
      (clearance_class THICK)\n\
      (via_rule R3)\n\
      (rule (width 200))\n\
    )\n\
    (class VFIRST VIANET\n\
      (via_rule R4)\n\
      (rule (width 200))\n\
    )\n\
    (class VCONTRAST VIANET2\n\
      (via_rule R5)\n\
      (rule (width 200))\n\
    )\n\
    (class PLANECLS GNDPLANE\n\
      (rule (width 1000))\n\
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
            start_ripup_costs: 1,
            fanout: Default::default(),
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
    ///
    /// PARITY DECISION (2026-10-02, upstream #935 / a917044ff): GND
    /// carries `(use_net GNDPLANE)` (plane synthesized at parse) —
    /// the bare planeless `(type power)` row this fixture had is
    /// PROMOTED to signal by the #935 port, which force-enables GND
    /// and orphans the ctor's `!is_signal` branch. A net-named power
    /// layer is the post-#935 upstream-HEAD shape of a non-signal
    /// layer; the force-false arm is pinned on that shape.
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
    /// (GNDPLANE's PLANECLS `[500, 500, 500]` — the plane net holds
    /// number 1, registered at structure-close create_board BEFORE
    /// the network scope numbers MINE/OTHER, the Java order), NOT
    /// MINE's `[300, 0, 0]` and NOT the default class's `[100, …]`;
    /// its clearance class is the literal
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
        // PARITY DECISION (2026-10-02, #935): net 1 is GNDPLANE now —
        // the plane net registers at structure-close create_board,
        // BEFORE the network scope numbers MINE/OTHER (Java order) —
        // so the net-0 fallback reads GNDPLANE's PLANECLS widths
        // [500, 500, 500], not MINE's. PLANECLS keeps the arm
        // mutant-killing: a no-fallback mutant reads the DEFAULT class
        // [100, 100, 100], a net-2 mutant reads MINE [300, 0, 0].
        assert_eq!(zero_control.trace_half_width, vec![500, 500, 500]);
        assert_eq!(
            zero_control.compensated_trace_half_width,
            vec![600, 600, 600]
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
    /// `AutorouteControl.java:235-239`): rule `R1`'s first via `V1`
    /// names the matrix class `default`, which the parse maps to
    /// matrix class 1 — the same value the NULL-net fallback
    /// hardcodes, so the literal doubles as the fallback-shape
    /// witness. (The NON-default class discriminations live in
    /// [`via_clearance_class_resolution_matrix`].)
    #[test]
    fn via_clearance_class_from_first_via() {
        let (_manager, mut board) = parse(CONTROL_DSN);
        let smd = net_no(&board, "SMDNET");
        let control = control_for(&mut board, smd, vec![true, true, true]);
        assert_eq!(control.via_clearance_class, 1);
    }

    /// THE VIA CLASS RESOLUTION MATRIX (`via[0]`,
    /// `AutorouteControl.java:235-239`, M3-T3 quality round I-2a).
    /// Three rules arrange every plausible mutant to disagree with
    /// Java somewhere:
    /// - `R3 = [V3]` reads the THICK class (matrix class 2) — kills a
    ///   hardcode-1 mutant (all pre-existing crafted vias declared
    ///   `default`, so the mutant was coincidence-blind: 9/9 green);
    /// - `R4 = [V3, V1]` reads the FIRST via (2, not `V1`'s 1) —
    ///   kills a last-via mutant;
    /// - `R5 = [V1, V3]` reads positionally too (1, not the
    ///   max/thickest) — kills a class-maximizing mutant.
    #[test]
    fn via_clearance_class_resolution_matrix() {
        let (_manager, mut board) = parse(CONTROL_DSN);
        let thick = net_no(&board, "THICKNET");
        let first = net_no(&board, "VIANET");
        let contrast = net_no(&board, "VIANET2");
        let thick_control = control_for(&mut board, thick, vec![true, true, true]);
        assert_eq!(thick_control.via_clearance_class, 2, "R3 = [V3] THICK");
        let first_control = control_for(&mut board, first, vec![true, true, true]);
        assert_eq!(
            first_control.via_clearance_class, 2,
            "R4 = [V3, V1]: FIRST via, not last"
        );
        let contrast_control = control_for(&mut board, contrast, vec![true, true, true]);
        assert_eq!(
            contrast_control.via_clearance_class, 1,
            "R5 = [V1, V3]: positional, not class-max"
        );
    }

    /// THE TRACE CLASS PIN (`trace_clearance_class_index`,
    /// `AutorouteControl.java:210-216`, M3-T3 quality round I-2b):
    /// THICKCLS's `(clearance_class THICK)` resolves to the
    /// structure-created matrix class 2 — `(rule (clearance 400
    /// (type THICK-THICK)))` appends one class and sets its diagonal
    /// to 400 on every layer (jar-verified creation syntax). Two
    /// discriminating rows:
    /// - `trace_clearance_class_index == 2` — kills a hardcode-1
    ///   mutant (all five pre-existing classes declared `default`, so
    ///   the mutant was coincidence-blind: 9/9 green);
    /// - `compensated_trace_half_width == [300, 300, 300]` (half
    ///   width 100 + THICK compensation `(400+1)/2 = 200`) against
    ///   SMDNET's `[200, 200, 200]` — SAME half width, default-class
    ///   compensation `(200+1)/2 = 100`, so the 100 delta comes only
    ///   from the class index and the mutant fails there too.
    #[test]
    fn trace_clearance_class_index_carries_the_net_class_class() {
        let (_manager, mut board) = parse(CONTROL_DSN);
        let thick = net_no(&board, "THICKNET");
        let smd = net_no(&board, "SMDNET");
        let thick_control = control_for(&mut board, thick, vec![true, true, true]);
        assert_eq!(thick_control.trace_clearance_class_index, 2);
        assert_eq!(thick_control.trace_half_width, vec![100, 100, 100]);
        assert_eq!(
            thick_control.compensated_trace_half_width,
            vec![300, 300, 300]
        );
        let smd_control = control_for(&mut board, smd, vec![true, true, true]);
        assert_eq!(smd_control.trace_clearance_class_index, 1);
        assert_eq!(
            smd_control.compensated_trace_half_width,
            vec![200, 200, 200],
            "same half width, default-class compensation — the 100 delta is the class"
        );
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
