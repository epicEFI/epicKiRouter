//! The T13 settings subset resolver (Java `SettingsMerger` + `CliSettings`
//! + `DsnFileSettings` + `DefaultSettings`).
//!
//! Sources and priorities (SettingsMerger.java:33-49): Default **0** ->
//! DsnFile **20** -> Cli **60**. The merge (`ReflectionUtil.copyFields`,
//! `:215-338`) is a DEEP merge: wrapper/scalar fields copy when the
//! source value is non-null (primitives: non-default), nested objects
//! (`scoring`, `autorouter`, ...) recurse field-by-field, object arrays
//! (`layers`) merge per element. Every source expresses "no opinion" by
//! leaving the field null — the CLAUDE.md nullable-fields invariant; the
//! Rust model below is the same shape: `Option<T>` per field, resolved
//! in priority order.
//!
//! Three bug-compat facts shape the resolution (all verified against the
//! frozen Java):
//!
//! 1. `boardSpecificTraceCostsApplied` is `private transient`
//!    (`RouterSettings.java`) — `copyFields` copies only non-null
//!    wrapper fields the merger knows, and the headless flow's
//!    `applyBoardSpecificOptimizations` re-initializes the per-layer
//!    trace costs whenever the flag is not TRUE — which, in the merged
//!    headless flow, is ALWAYS. DSN `(preferred_direction_trace_costs
//!    ...)` values are parsed, merged, then DISCARDED: the resolver
//!    ports the geometry pass as unconditional and never carries the
//!    DSN trace costs (SEAM documents the bug-compat).
//! 2. The per-layer preferred direction after the geometry pass is the
//!    ASPECT-RATIO alternation (`horizontalWidth < verticalWidth`,
//!    toggled on every signal layer), not the parse-time `i % 2 == 1`
//!    fallback — but a DSN-set `(preferred_direction ...)` SURVIVES,
//!    because the geometry pass only fills the null slots.
//! 3. `getBendCost` (`RouterSettings.java:696-707`) returns a SET
//!    per-layer bend cost UNCLAMPED and clamps only the DEFAULT
//!    fallback into `[0.0, 9.9]` (the setter clamps on write, but the
//!    geometry pass writes `scoring.defaultBendCost` through raw).

use epic_dsn::scope::autoroute_settings::AutorouteSettingsIr;
use epic_router::control::{ExpansionCostFactor, FanoutSettingsIr, RouterSettingsIr};
use epic_router::pipeline::batch::BatchSettings;
use epic_router::pipeline::board_statistics::{
    OptimizerScoreSettings, OptimizerScoringVersion, RouterScoreSettings, RouterScoringVersion,
    RouterSettingsScoring, RoutingCostSettings, default_routing_cost_settings,
};
use epic_router::pipeline::optimizer::OptimizerSettingsIr;

// ---------------------------------------------------------------------------
// the CLI parse face (CliSettings.java port)
// ---------------------------------------------------------------------------

/// The T13-relevant slice of the parsed CLI layer (priority 60). Only
/// paths the headless pipeline (or the manifest) consumes are carried;
/// any other `router.*`/`optimizer.*` path WARNs and continues, mirroring
/// Java's reflection-failure warn (`CliSettings.java:161-163`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CliLayer {
    /// `router.autorouter.enabled` (or deprecated `router.enabled`).
    pub autorouter_enabled: Option<bool>,
    /// `router.autorouter.max_passes` (or deprecated flat
    /// `router.max_passes`) / `-mp`.
    pub max_passes: Option<i32>,
    /// `router.autorouter.max_items` (or deprecated flat
    /// `router.max_items`).
    pub max_items: Option<i32>,
    /// `router.max_threads` / `-mt` — LIVE since M5-T7: the explicit
    /// CLI face engages the deterministic partitioned executor
    /// (`run_partitioned`, byte-identical to the sequential path at
    /// every N). The default is 1 — the golden sequential path — even
    /// though the merged settings surface keeps Java's
    /// `max(1, cores-1)` parity default for validation (the executor
    /// face reads the EXPLICIT CLI value only; see `route.rs`).
    pub max_threads: Option<i32>,
    /// `router.optimizer.improvement_threshold` / `-oit` (also
    /// `optimizer.improvement_threshold` and the
    /// `optimizationImprovementThreshold` spelling) — the OPTIMIZER
    /// pass-stop threshold, parsed and normalized (the x100 quirk).
    /// LIVE since M4-T9: resolved into `OptimizerSettingsIr
    /// ::improvement_threshold` and consumed by the stage's stop gate.
    /// (Pre-T13 name — the only family member without the
    /// `optimizer_` prefix.)
    pub improvement_threshold: Option<f32>,
    /// `router.scoring.version` (dual-applied from `-scoring-version`).
    pub router_scoring_version: Option<RouterScoringVersion>,
    /// `optimizer.scoring.version` / `optimizer-scoring-version` —
    /// the ONLY CLI-settable field of the optimizer score box: the
    /// `CliSettings` routing guard accepts only `router.*`/`optimizer.*`
    /// prefixed paths, so `--optimizer_scoring.<scalar>` is silently
    /// ignored and `--optimizer.scoring.<scalar>` fails the reflection
    /// walk with a WARN — the scalars are settable only through the
    /// GUI/API sources. Merged into the optimizer score box; the V2
    /// formula's consumer is the M4 optimizer stage.
    pub optimizer_scoring_version: Option<OptimScoringVersion>,
    /// `router.vias_allowed`.
    pub vias_allowed: Option<bool>,
    /// `router.via_costs`.
    pub via_costs: Option<i32>,
    /// `router.plane_via_costs`.
    pub plane_via_costs: Option<i32>,
    /// `router.plane_island_clamp` — the M6-T6 RUST-ONLY intelligence
    /// flag (no Java counterpart; the `router.*` guard path accepts it
    /// by prefix). Default OFF: the clamp is dead code at defaults and
    /// its route effect is measured at T8/T9, not here.
    pub plane_island_clamp: Option<bool>,
    /// `router.congestion_global` — the M6-T7 global-planning master
    /// switch (RUST-ONLY, no Java counterpart): the congestion map +
    /// per-net guides + the planned net order. Default OFF (the T6
    /// seam precedent): at defaults the stage never runs and the
    /// recurring gate set stays byte-identical.
    pub congestion_global: Option<bool>,
    /// `router.congestion_global.pattern` — the L/Z pattern-routing
    /// fast path sub-flag (RUST-ONLY). Requires the master ON; default
    /// OFF. Documented per the dispatch charter.
    pub congestion_global_pattern: Option<bool>,
    /// `router.congestion_global.pathfinder` — the M6-T8 PathFinder
    /// negotiated-congestion scheduler sub-flag (RUST-ONLY, no Java
    /// counterpart). Requires the master ON; default OFF. The
    /// negotiated bases replace the linear pass ladder WHEN ON
    /// (`global/history.rs` docs).
    pub congestion_global_pathfinder: Option<bool>,
    /// `router.push_shove` — the M6-T9 push-and-shove insertion
    /// flag (RUST-ONLY, no Java counterpart; a top-level flag on the
    /// family pattern — no master). When ON, a maze obstacle room the
    /// shove probe has verified shovable may WAIVE its rip-up charge
    /// within a bounded per-search budget, so the insertion displaces
    /// the neighbor trace (M3 `trace_shover::insert`) instead of
    /// ripping it. Default OFF; the default path stays byte-identical.
    pub push_shove: Option<bool>,
    /// `router.tuning` — the M7-T2 tuning OVERRIDE/KILL-SWITCH
    /// (RUST-ONLY, no Java counterpart; a top-level tri-state — the
    /// flag is NOT needed for activation). The tuning regime is
    /// INPUT-DRIVEN: it activates when any net resolves a non-zero
    /// net-class length bound (`BoardRules::has_length_constraints`).
    /// The flag only bends that: `on` forces tuning ON even on
    /// constraint-free input, `off` is the kill-switch (tuning stays
    /// OFF even where a declaration activates it). Absent (the
    /// default) = input-driven. Default settings + no declaration ⇒
    /// tuning never fires — the constraint-free zero-rotation face.
    pub tuning: Option<bool>,
    /// `router.tuning.meander` — the M7-T4 MEANDER stage sub-flag
    /// (RUST-ONLY; a second top-level tri-state on the `router.tuning`
    /// family pattern — the sub-flag does NOT require the master).
    /// `None` (the default) = input-driven: the meander stage rides
    /// the resolved `tuning_active` (a declaration activates both).
    /// `on` forces the stage even on constraint-free input (a natural
    /// no-op there — no declaration, no deficit rows). `off` kills the
    /// meander stage ALONE: the min-length honoring gate (T3) stays
    /// armed, deficits stand and are reported honestly. The M7-T5
    /// MATCH face rides the same activation: constrained nets match
    /// to their class target (the declared max, else the group's
    /// longest routed length, tie net-id ASC) within the group
    /// tolerance — the class's own min–max window when both bounds
    /// are declared, else the NAMED ENGINE CONSTANT
    /// `epic_router::pipeline::tuning::MATCH_TOLERANCE_DBU` (40_000
    /// DBU — deliberately not a setting: a tolerance the routing
    /// engine owns, documented here per the T2/T4 named-constant
    /// family pattern).
    pub tuning_meander: Option<bool>,
    /// `router.tuning.pairs` — the M7-T6 DIFFERENTIAL-PAIR declaration
    /// list (RUST-ONLY; the third activation input). Grammar: a
    /// comma-separated list of `NET_A:NET_B` net-name pairs, e.g.
    /// `usb_p:usb_n,clk_p:clk_n` (names case-preserved, trimmed; a
    /// malformed pair fails the whole value with the standard
    /// warn-and-continue face). Absent (the default) = no declared
    /// pairs = the pair face is structurally inert (byte-identical).
    /// Declaring pairs activates: the pass leader-first order, the
    /// follower's maze coupling preference, and the post-routing pair
    /// match stage. The pair face owns its own TIGHTER delta, the
    /// named engine constant
    /// `epic_router::pipeline::pairs::PAIR_DELTA_DBU` (20_000 DBU —
    /// deliberately tighter than the T5 match tolerance
    /// MATCH_TOLERANCE_DBU's 40_000, never inherited from a class
    /// min–max window; AMENDMENT 5 §6), documented here per the T2/T4
    /// named-constant family pattern. A name resolving to several
    /// subnet nets (a fromto split) or to none stays UNRESOLVED and is
    /// recorded in the manifest's advisory `pair_unresolved` rows —
    /// never guessed.
    pub tuning_pairs: Option<Vec<(String, String)>>,
    /// `router.gloss.bus` — the M8-T3 GLOSS BUS tri-state (RUST-ONLY,
    /// no Java counterpart; the gloss family block beside the
    /// `router.tuning` family). `None` (the default) = OFF: the gloss
    /// stage never runs — the two-regime law's byte-invariance face.
    /// `on` = the post-tuning gloss stage runs the parallel-bus group
    /// detector + the hug/spread re-spacing pass
    /// (`epic_router::pipeline::gloss`; the report rides the
    /// `--dump-aesthetics` SIDECAR as the `bus_groups` block — never
    /// the manifest). `off` = the explicit kill face (identical to
    /// absent: the pass has no input-driven activation). The named
    /// engine constants (`BUS_SPAN_THRESHOLD_DBU` 100_000 DBU strict,
    /// `BUS_MAX_LENGTH_GAIN_DBU` 40_000 DBU, the tied coupling window
    /// 50_000 DBU) live in gloss.rs per the T2/T4 named-constant
    /// family pattern.
    pub gloss_bus: Option<bool>,
    /// `router.gloss.flow` — the M8-T4 GLOSS FLOW tri-state
    /// (RUST-ONLY, no Java counterpart; the gloss family block). `None`
    /// (the default) = OFF: the flow stage never runs — the two-regime
    /// law's byte-invariance face. `on` = the gloss stage slot runs the
    /// 45° flow pass AFTER the bus pass (flow after spread — the AM2
    /// composition law): jog/stub elimination + the miter/recorner
    /// bridge (`epic_router::pipeline::gloss`; the report rides the
    /// `--dump-aesthetics` SIDECAR as the `gloss_flow` block — never
    /// the manifest; a gated hold surfaces as the DISTINCT
    /// `gloss_flow_gated` sibling key). `off` = the explicit kill face
    /// (identical to absent: the pass has no input-driven activation).
    /// The named engine constants (`JOG_MAX_SEGMENT_FACTOR` 6.0 strict
    /// against 2×half-width, `MITER_MAX_STUB_DBU` 30_000 DBU strict,
    /// `FLOW_MAX_LENGTH_GAIN_DBU` 40_000 DBU) live in gloss.rs per the
    /// named-constant family pattern.
    pub gloss_flow: Option<bool>,
    /// `router.gloss.via_place` — the M8-T5 GLOSS VIA-PLACE tri-state
    /// (RUST-ONLY, no Java counterpart; the gloss family block). `None`
    /// (the default) = OFF: the via stage never runs — the two-regime
    /// law's byte-invariance face. `on` = the gloss stage slot runs the
    /// return-path-aware via placement pass AFTER the flow pass (the
    /// terminal slot — the recorded slot decision in gloss.rs):
    /// 45°-lattice + alignment-derived candidates ranked by plane
    /// stitch-distance drop and arm straightening, whole-candidate
    /// acceptance (`epic_router::pipeline::gloss`; the report rides the
    /// `--dump-aesthetics` SIDECAR as the `gloss_via_place` block —
    /// never the manifest; a gated hold surfaces as the DISTINCT
    /// `gloss_via_place_gated` sibling key). `off` = the explicit kill
    /// face (identical to absent: the pass has no input-driven
    /// activation). The named engine constants (`VIA_PLACE_RADIUS_DBU`
    /// 60_000 DBU inclusive Chebyshev, `VIA_PLACE_STEP_DBU` 10_000 DBU,
    /// `VIA_PLACE_MAX_LENGTH_GAIN_DBU` 40_000 DBU) live in gloss.rs per
    /// the named-constant family pattern.
    pub gloss_via_place: Option<bool>,
    /// `router.gloss.teardrops` — the M8-T6 GLOSS TEARDROPS tri-state
    /// (RUST-ONLY, no Java counterpart; the gloss family block beside
    /// the `router.gloss.via_place` field). `None` (the default) = OFF:
    /// the gloss stage never runs the teardrop pass — the two-regime
    /// law's byte-invariance face. `on` = the gloss stage slot runs the
    /// graded-width teardrop pass AFTER the via-place pass (the
    /// terminal slot — the recorded slot decision in gloss.rs) at
    /// trace-to-pad/via junctions whose trace width is strictly below
    /// the named ratio threshold of the pad diameter, whole-candidate
    /// acceptance with REAL clearance probes
    /// (`epic_router::pipeline::gloss`; the report rides the
    /// `--dump-aesthetics` SIDECAR as the `gloss_teardrops` block —
    /// never the manifest; a gated hold surfaces as the DISTINCT
    /// `gloss_teardrops_gated` sibling key). `off` = the explicit kill
    /// face (identical to absent: the pass has no input-driven
    /// activation). The named engine constants
    /// (`TEARDROP_MAX_TRACE_RATIO_PCT` 100 — strictly below, the
    /// anatomy edge, `TEARDROP_WIRE_COUNT` 3, `TEARDROP_STEP_DBU`
    /// 4_000 DBU, `TEARDROP_BUDGET_PER_BOARD` 1024) live in gloss.rs
    /// per the named-constant family pattern.
    pub gloss_teardrops: Option<bool>,
    /// `router.start_ripup_costs`.
    pub start_ripup_costs: Option<i32>,
    /// `router.automatic_neckdown`.
    pub automatic_neckdown: Option<bool>,
    /// `router.trace_pull_tight_accuracy`.
    pub trace_pull_tight_accuracy: Option<i32>,
    /// `router.strict_drc`.
    pub strict_drc: Option<bool>,
    /// `router.fanout.enabled`.
    pub fanout_enabled: Option<bool>,
    /// `router.fanout.max_passes`.
    pub fanout_max_passes: Option<i32>,
    /// `router.fanout.max_items`.
    pub fanout_max_items: Option<i32>,
    /// `router.fanout.max_milliseconds_per_pin` (Java `Long`).
    pub fanout_max_milliseconds_per_pin: Option<i64>,
    /// `router.fanout.ripup_allowed`.
    pub fanout_ripup_allowed: Option<bool>,
    /// `router.fanout.min_escape_length_mm` (Java `Double`).
    pub fanout_min_escape_length_mm: Option<f64>,
    /// `router.fanout.max_escape_length_mm` (Java `Double`).
    pub fanout_max_escape_length_mm: Option<f64>,
    /// `router.fanout.start_via_diameter_mm` — settings-surface parity
    /// only (no engine consumer; the `FanoutSettingsIr` bank).
    pub fanout_start_via_diameter_mm: Option<f64>,
    /// `router.fanout.end_via_diameter_mm` — same bank.
    pub fanout_end_via_diameter_mm: Option<f64>,
    /// `router.fanout.pin_sorting_order` — stored VERBATIM (Java
    /// dispatches on the raw string; any unknown value falls through to
    /// the `pinIndex` tie-break, `BatchFanout.java:764-795`).
    pub fanout_pin_sorting_order: Option<String>,
    /// `router.fanout.fallback_to_board_vias`.
    pub fanout_fallback_to_board_vias: Option<bool>,
    /// `router.fanout.timeout` — the stage wall deadline
    /// (`TextManager.parseTimespanString` shape). The
    /// `fanout.timeout_string` spelling is also accepted: Java's
    /// reflection walk matches the SerializedName AND the field name
    /// (`ReflectionUtil.getFieldByNameOrSerializedName`).
    pub fanout_timeout_string: Option<String>,
    /// `optimizer.enabled` — the stage gate (`getRunOptimizer()`, with
    /// the router-only comparability profile passing `=false`).
    pub optimizer_enabled: Option<bool>,
    /// `optimizer.algorithm` — stored verbatim; the stage normalizes
    /// any non-`freerouting-optimizer` value with a warn.
    pub optimizer_algorithm: Option<String>,
    /// `optimizer.max_passes` (`optimizer.maxPasses` also accepted).
    pub optimizer_max_passes: Option<i32>,
    /// `optimizer.max_items` (`optimizer.maxItems`).
    pub optimizer_max_items: Option<i32>,
    /// `optimizer.max_threads` (`optimizer.maxThreads`) — parsed
    /// (surface parity) but unused: the port is sequential.
    pub optimizer_max_threads: Option<i32>,
    /// `optimizer.threads` — the M8-T7 RUST-ONLY tri-state: the
    /// optimizer candidate loop's partition count. `None` (the
    /// default) = the mandated 1-thread parity face, UNCHANGED code
    /// (the SettingsMerger-family invariant: no initializer — the
    /// Default seed is `None`); `Some(n)` with `n >= 2` = the
    /// deterministic partitioned candidate executor (`n` partitions by
    /// `item_id mod n`, reduction in candidate order — the final board
    /// state is thread-count-invariant by construction). `<= 1`
    /// resolves to the sequential face. No Java counterpart (Java's
    /// `optimizer.maxThreads` is the separate surface-parity box
    /// above).
    pub optimizer_threads: Option<i32>,
    /// `optimizer.enable_preflight_guards`
    /// (`optimizer.enablePreflightGuards`).
    pub optimizer_enable_preflight_guards: Option<bool>,
    /// `optimizer.max_consecutive_failures`
    /// (`optimizer.maxConsecutiveFailures`).
    pub optimizer_max_consecutive_failures: Option<i32>,
    /// `optimizer.max_consecutive_failures_pass1`
    /// (`optimizer.maxConsecutiveFailuresPass1`).
    pub optimizer_max_consecutive_failures_pass1: Option<i32>,
    /// `optimizer.additional_ripup_cost_factor_at_start`
    /// (`optimizer.additionalRipupCostFactorAtStart`).
    pub optimizer_additional_ripup_cost_factor_at_start: Option<i32>,
    /// `optimizer.trace_ripup_cost_factor`
    /// (`optimizer.traceRipupCostFactor`).
    pub optimizer_trace_ripup_cost_factor: Option<f32>,
    /// `optimizer.max_autoroute_passes`
    /// (`optimizer.maxAutoroutePasses`).
    pub optimizer_max_autoroute_passes: Option<i32>,
    /// `optimizer.timeout` (`optimizer.timeout_string` /
    /// `optimizer.timeoutString`) — the stage wall deadline string.
    pub optimizer_timeout_string: Option<String>,
    /// The `-oit` deprecation warns, the deprecated-path warns and the
    /// WARN-and-continue rows, in emission order (Java logs them through
    /// FRLogger.warn).
    pub warnings: Vec<String>,
}

/// Java `settings.OptimizerScoringVersion` (`OptimizerScoringVersion.java`:
/// exactly `V1_LEGACY`, `V2_LOWER_BOUND` — note there is NO V2_CONTINUOUS
/// on this box, which is what makes the `v2` alias asymmetry work).
/// Parsed by the CLI face; consumed by the optimizer milestone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptimScoringVersion {
    /// Java `V1_LEGACY`.
    V1Legacy,
    /// Java `V2_LOWER_BOUND`.
    V2LowerBound,
}

/// The narrowed `-de`/`-do` surface plus the router-settings layer.
///
/// Java `GlobalSettings.java:608-665` consumes ALL non-`-` args after
/// `-de` (multi-file, `+` concatenation, type-by-extension); the `route`
/// subcommand NARROWS this to exactly one DSN and one SES — a second
/// file or a `+` concatenation is a hard ERROR (SEAM: the narrowing).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParsedRouteArgs {
    /// `-de <dsn>` — the single design input.
    pub dsn: Option<String>,
    /// `-do <ses>` — the single session output.
    pub ses: Option<String>,
    /// `--result-json <path>` / `--result-json=<path>` — the manifest.
    pub result_json: Option<String>,
    /// `--deterministic-budgets=on|off` (default ON — anchors §6).
    pub deterministic_budgets: Option<bool>,
    /// `--dump-aesthetics <path>` / `--dump-aesthetics=<path>` — the
    /// M8-T1 sidecar door: the four aesthetics metrics land in THIS
    /// file at manifest-write time, never in the manifest (the
    /// rotation trap named at the write site).
    pub dump_aesthetics: Option<String>,
    /// The parsed router-settings layer (warnings included).
    pub layer: CliLayer,
}

impl ParsedRouteArgs {
    /// True when the args carry `-de` AND `-do` — the implicit
    /// router-enable trigger (`CliSettings.java:100-107`).
    #[must_use]
    pub fn batch_mode(&self) -> bool {
        self.dsn.is_some() && self.ses.is_some()
    }
}

/// The `route` argument walk. `CliSettings.parseArguments` sees the FULL
/// argv (including `-de`/`-do`, which it uses only for the implicit
/// router-enable face); the file extraction is the narrowed
/// `GlobalSettings` port.
///
/// # Errors
///
/// A second `-de`/`-do` file, a `+` concatenation, a missing value, or a
/// malformed `--deterministic-budgets` value is a hard error (the
/// narrowed scope); a bad router-setting VALUE is WARN-and-continue.
pub fn parse_route_args(args: &[String]) -> Result<ParsedRouteArgs, String> {
    let mut parsed = ParsedRouteArgs::default();
    let mut has_explicit_router_enabled = false;
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if let Some(rest) = arg.strip_prefix("--") {
            // The `--property=value` form: Java requires the `=`
            // (`CliSettings.java:46`) — a bare `--property` with no `=`
            // is IGNORED (except the CLI-owned space-form flags below).
            if let Some((property, value)) = rest.split_once('=') {
                if "router.enabled" == property || "router.autorouter.enabled" == property {
                    has_explicit_router_enabled = true;
                }
                match property {
                    "result-json" => parsed.result_json = Some(value.to_string()),
                    "dump-aesthetics" => parsed.dump_aesthetics = Some(value.to_string()),
                    "deterministic-budgets" => {
                        parsed.deterministic_budgets =
                            Some(parse_on_off(value).ok_or_else(|| {
                                format!(
                                    "invalid --deterministic-budgets value '{value}' \
                                     (expected on|off)"
                                )
                            })?);
                    }
                    _ if property.starts_with("router.") || property.starts_with("optimizer.") => {
                        apply_router_setting(&mut parsed.layer, property, value);
                    }
                    // An unknown --property=value is outside the router
                    // namespace: Java never applies it (silent).
                    _ => {}
                }
            } else if rest == "result-json" {
                // CLI-owned space form: `--result-json <path>`.
                let (value, consumed) = next_value(args, i);
                i += consumed;
                parsed.result_json = Some(value.to_string());
            } else if rest == "dump-aesthetics" {
                // CLI-owned space form: `--dump-aesthetics <path>` —
                // the M8-T1 sidecar door (never a `router.*` setting:
                // it adds NO settings-family field, the T1 invariant).
                let (value, consumed) = next_value(args, i);
                i += consumed;
                parsed.dump_aesthetics = Some(value.to_string());
            }
        } else if let Some(flag) = arg.strip_prefix('-') {
            // The `-flag value` form: the value is the next arg IFF it
            // does not start with `-`, else "" (`CliSettings.java:75`).
            let (value, consumed) = next_value(args, i);
            i += consumed;
            match flag {
                "de" => {
                    if parsed.dsn.is_some() {
                        return Err("route takes exactly one -de design file (the multi-file/+ \
                             concatenation surface of GlobalSettings.java:608-665 is \
                             narrowed away)"
                            .to_string());
                    }
                    if value.contains('+') {
                        return Err(format!(
                            "route takes exactly one -de design file ('+' concatenation \
                             rejected: '{value}')"
                        ));
                    }
                    if value.is_empty() {
                        return Err("-de requires a design file path".to_string());
                    }
                    parsed.dsn = Some(value);
                }
                "do" => {
                    if parsed.ses.is_some() {
                        return Err("route takes exactly one -do output file (the additional- \
                             output surface of GlobalSettings.java:667+ is narrowed away)"
                            .to_string());
                    }
                    if value.contains('+') {
                        return Err(format!(
                            "route takes exactly one -do output file ('+' concatenation \
                             rejected: '{value}')"
                        ));
                    }
                    if value.is_empty() {
                        return Err("-do requires a session file path".to_string());
                    }
                    parsed.ses = Some(value);
                }
                "result-json" => {
                    parsed.result_json = Some(value);
                }
                "dump-aesthetics" => {
                    parsed.dump_aesthetics = Some(value);
                }
                "oit" => {
                    // The deprecation warn fires EVEN when the flag then
                    // applies (`CliSettings.java:83-87`).
                    parsed.layer.warnings.push(
                        "The '-oit' command-line flag is deprecated; use \
                         '--router.optimizer.improvement_threshold' instead."
                            .to_string(),
                    );
                    apply_router_setting(
                        &mut parsed.layer,
                        "router.optimizer.improvement_threshold",
                        &value,
                    );
                }
                // The scoring-version flag family: `-scoring-version`
                // dual-applies to BOTH score boxes
                // (`CliSettings.java:56-58, 114-118`); the two
                // side-specific flags address one box each.
                "scoring-version" => {
                    apply_router_setting(&mut parsed.layer, "router.scoring.version", &value);
                    apply_router_setting(&mut parsed.layer, "optimizer.scoring.version", &value);
                }
                "router-scoring-version" => {
                    apply_router_setting(&mut parsed.layer, "router.scoring.version", &value);
                }
                "optimizer-scoring-version" => {
                    apply_router_setting(&mut parsed.layer, "optimizer.scoring.version", &value);
                }
                // The short-flag map (`CliSettings.java:166-178`).
                "mp" => {
                    apply_router_setting(&mut parsed.layer, "router.max_passes", &value);
                }
                "mt" => {
                    apply_router_setting(&mut parsed.layer, "router.max_threads", &value);
                }
                _ => {
                    // Java's mapFlagToProperty returns null for unknown
                    // short flags and the loop SILENTLY skips them.
                }
            }
        }
        i += 1;
    }

    // The implicit router-enable (`CliSettings.java:100-107`): the legacy
    // batch invocation (`-de ... -do ...`) routes immediately UNLESS the
    // caller explicitly set the enable path.
    if parsed.batch_mode() && !has_explicit_router_enabled {
        parsed.layer.autorouter_enabled = Some(true);
    }
    Ok(parsed)
}

/// The `-flag value` value rule (`CliSettings.java:75`): the next arg
/// when it exists and does not start with `-`, else "" (nothing
/// consumed).
fn next_value(args: &[String], i: usize) -> (String, usize) {
    match args.get(i + 1) {
        Some(next) if !next.starts_with('-') => (next.clone(), 1),
        _ => (String::new(), 0),
    }
}

/// `CliSettings.mapFlagToProperty` + `applyRouterSetting` + the
/// `LegacyRouterSettingsBridge` canonicalization + the value
/// normalization quirks, folded into one assign (the T13 subset).
fn apply_router_setting(layer: &mut CliLayer, property: &str, value: &str) {
    // `applyRouterSetting` re-checks the dual-apply at entry
    // (`CliSettings.java:114-118`) — the `scoring-version` name keeps
    // dual-applying even when routed through here.
    if "scoring-version" == property {
        apply_router_setting(layer, "router.scoring.version", value);
        apply_router_setting(layer, "optimizer.scoring.version", value);
        return;
    }
    // Field-path canonicalization (`CliSettings.java:119-134`).
    let field_path = if let Some(relative) = property.strip_prefix("router.") {
        if "scoring.version" == relative {
            // `:122-123` — the router score box.
            "routerScoring.version".to_string()
        } else {
            canonical_cli_path(relative)
        }
    } else if "optimizer.scoring.version" == property {
        // `:124-125` — the optimizer score box.
        "optimizerScoring.version".to_string()
    } else {
        property.to_string()
    };
    // The deprecated flat path warn (LegacyRouterSettingsBridge).
    if let Some(relative) = property.strip_prefix("router.")
        && is_deprecated_flat_autorouter_path(relative)
    {
        layer.warnings.push(format!(
            "Deprecated settings path 'router.{relative}'; use 'router.{}' instead. \
             The old path will be removed in a future release.",
            canonical_cli_path(relative)
        ));
    }

    // The scoring-version value normalization (`CliSettings.java:135-156`)
    // — THE ASYMMETRY: the `v2`/`continuous` alias resolves to
    // V2_CONTINUOUS on the router box but V2_LOWER_BOUND on the optimizer
    // box (which has no V2_CONTINUOUS constant at all). Alias table
    // first; anything else is the raw CONSTANT name (case-insensitive);
    // a constant the addressed box does not have warns and continues.
    if field_path.ends_with(".version") {
        let router_side = !field_path.starts_with("optimizer");
        let constant: Option<String> = match value.trim().to_lowercase().as_str() {
            "v1" | "legacy" => Some("V1_LEGACY".to_string()),
            "v2" | "continuous" => Some(
                if router_side {
                    "V2_CONTINUOUS"
                } else {
                    "V2_LOWER_BOUND"
                }
                .to_string(),
            ),
            "lower_bound" | "lower-bound" => Some("V2_LOWER_BOUND".to_string()),
            // Java's default arm passes the value RAW to valueOf
            // (`CliSettings.java:142` `default -> value`) — only the
            // alias table above is case-insensitive; a mixed-case
            // constant (`V2_Continuous`) or padded value fails valueOf
            // and warns with the field unset.
            _ => Some(value.to_string()),
        };
        match constant.as_deref() {
            Some("V1_LEGACY") => {
                if router_side {
                    layer.router_scoring_version = Some(RouterScoringVersion::V1Legacy);
                } else {
                    layer.optimizer_scoring_version = Some(OptimScoringVersion::V1Legacy);
                }
            }
            Some("V2_CONTINUOUS") if router_side => {
                layer.router_scoring_version = Some(RouterScoringVersion::V2Continuous);
            }
            Some("V2_LOWER_BOUND") if !router_side => {
                layer.optimizer_scoring_version = Some(OptimScoringVersion::V2LowerBound);
            }
            other => {
                layer.warnings.push(format!(
                    "Failed to apply CLI router setting: {property}: No enum constant \
                     {other:?} for the addressed scoring box"
                ));
            }
        }
        return;
    }

    // The improvement_threshold x100 quirk (`CliSettings.java:145-156`):
    // a float in the EXCLUSIVE interval (0, 1) is interpreted as a
    // fraction and multiplied by 100 (0.25 -> 25.0); 0.0 and 1.0 pass
    // through raw; a parse failure warns and continues (value unset).
    if "router.optimizer.improvement_threshold" == property
        || field_path.ends_with("improvement_threshold")
        || field_path.ends_with("optimizationImprovementThreshold")
    {
        match value.trim().parse::<f32>() {
            Ok(parsed) if parsed > 0.0 && parsed < 1.0 => {
                layer.improvement_threshold = Some(parsed * 100.0);
            }
            Ok(parsed) => layer.improvement_threshold = Some(parsed),
            Err(_) => layer.warnings.push(format!(
                "Failed to apply CLI router setting: {property}: For input string: \"{value}\""
            )),
        }
        return;
    }

    // The T13 assign face (ReflectionUtil.setFieldValue over the subset).
    match field_path.as_str() {
        "autorouter.enabled" => match parse_on_off(value) {
            Some(v) => layer.autorouter_enabled = Some(v),
            None => warn_bad_value(layer, property, value),
        },
        "autorouter.max_passes" => match value.parse::<i32>() {
            Ok(v) => layer.max_passes = Some(v),
            Err(_) => warn_bad_value(layer, property, value),
        },
        "autorouter.max_items" => match value.parse::<i32>() {
            Ok(v) => layer.max_items = Some(v),
            Err(_) => warn_bad_value(layer, property, value),
        },
        "max_threads" => match value.parse::<i32>() {
            Ok(v) => layer.max_threads = Some(v),
            Err(_) => warn_bad_value(layer, property, value),
        },
        "vias_allowed" => match parse_on_off(value) {
            Some(v) => layer.vias_allowed = Some(v),
            None => warn_bad_value(layer, property, value),
        },
        "via_costs" => match value.parse::<i32>() {
            Ok(v) => layer.via_costs = Some(v),
            Err(_) => warn_bad_value(layer, property, value),
        },
        "plane_via_costs" => match value.parse::<i32>() {
            Ok(v) => layer.plane_via_costs = Some(v),
            Err(_) => warn_bad_value(layer, property, value),
        },
        "plane_island_clamp" => match parse_on_off(value) {
            Some(v) => layer.plane_island_clamp = Some(v),
            None => warn_bad_value(layer, property, value),
        },
        "congestion_global" => match parse_on_off(value) {
            Some(v) => layer.congestion_global = Some(v),
            None => warn_bad_value(layer, property, value),
        },
        "congestion_global.pattern" => match parse_on_off(value) {
            Some(v) => layer.congestion_global_pattern = Some(v),
            None => warn_bad_value(layer, property, value),
        },
        "congestion_global.pathfinder" => match parse_on_off(value) {
            Some(v) => layer.congestion_global_pathfinder = Some(v),
            None => warn_bad_value(layer, property, value),
        },
        "push_shove" => match parse_on_off(value) {
            Some(v) => layer.push_shove = Some(v),
            None => warn_bad_value(layer, property, value),
        },
        "tuning" => match parse_on_off(value) {
            Some(v) => layer.tuning = Some(v),
            None => warn_bad_value(layer, property, value),
        },
        "tuning.meander" => match parse_on_off(value) {
            Some(v) => layer.tuning_meander = Some(v),
            None => warn_bad_value(layer, property, value),
        },
        "tuning.pairs" => match parse_pairs(value) {
            Some(v) => layer.tuning_pairs = Some(v),
            None => warn_bad_value(layer, property, value),
        },
        "gloss.bus" => match parse_on_off(value) {
            Some(v) => layer.gloss_bus = Some(v),
            None => warn_bad_value(layer, property, value),
        },
        "gloss.flow" => match parse_on_off(value) {
            Some(v) => layer.gloss_flow = Some(v),
            None => warn_bad_value(layer, property, value),
        },
        "gloss.via_place" => match parse_on_off(value) {
            Some(v) => layer.gloss_via_place = Some(v),
            None => warn_bad_value(layer, property, value),
        },
        "gloss.teardrops" => match parse_on_off(value) {
            Some(v) => layer.gloss_teardrops = Some(v),
            None => warn_bad_value(layer, property, value),
        },
        "start_ripup_costs" => match value.parse::<i32>() {
            Ok(v) => layer.start_ripup_costs = Some(v),
            Err(_) => warn_bad_value(layer, property, value),
        },
        "automatic_neckdown" => match parse_on_off(value) {
            Some(v) => layer.automatic_neckdown = Some(v),
            None => warn_bad_value(layer, property, value),
        },
        "trace_pull_tight_accuracy" => match value.parse::<i32>() {
            Ok(v) => layer.trace_pull_tight_accuracy = Some(v),
            Err(_) => warn_bad_value(layer, property, value),
        },
        "strict_drc" => match parse_on_off(value) {
            Some(v) => layer.strict_drc = Some(v),
            None => warn_bad_value(layer, property, value),
        },
        "fanout.enabled" => match parse_on_off(value) {
            Some(v) => layer.fanout_enabled = Some(v),
            None => warn_bad_value(layer, property, value),
        },
        // The M4-T7 fanout group (Java `FanoutSettings` SerializedName
        // faces; value conversions mirror `ReflectionUtil.convertValue`:
        // Integer/Long/Double `parse*` on the exact type, booleans via
        // the on/off face). `pin_sorting_order` and `timeout` are
        // strings — applied verbatim, never validated here.
        "fanout.max_passes" => match value.parse::<i32>() {
            Ok(v) => layer.fanout_max_passes = Some(v),
            Err(_) => warn_bad_value(layer, property, value),
        },
        "fanout.max_items" => match value.parse::<i32>() {
            Ok(v) => layer.fanout_max_items = Some(v),
            Err(_) => warn_bad_value(layer, property, value),
        },
        "fanout.max_milliseconds_per_pin" => match value.parse::<i64>() {
            Ok(v) => layer.fanout_max_milliseconds_per_pin = Some(v),
            Err(_) => warn_bad_value(layer, property, value),
        },
        "fanout.ripup_allowed" => match parse_on_off(value) {
            Some(v) => layer.fanout_ripup_allowed = Some(v),
            None => warn_bad_value(layer, property, value),
        },
        "fanout.min_escape_length_mm" => match value.parse::<f64>() {
            Ok(v) => layer.fanout_min_escape_length_mm = Some(v),
            Err(_) => warn_bad_value(layer, property, value),
        },
        "fanout.max_escape_length_mm" => match value.parse::<f64>() {
            Ok(v) => layer.fanout_max_escape_length_mm = Some(v),
            Err(_) => warn_bad_value(layer, property, value),
        },
        "fanout.start_via_diameter_mm" => match value.parse::<f64>() {
            Ok(v) => layer.fanout_start_via_diameter_mm = Some(v),
            Err(_) => warn_bad_value(layer, property, value),
        },
        "fanout.end_via_diameter_mm" => match value.parse::<f64>() {
            Ok(v) => layer.fanout_end_via_diameter_mm = Some(v),
            Err(_) => warn_bad_value(layer, property, value),
        },
        "fanout.pin_sorting_order" => {
            layer.fanout_pin_sorting_order = Some(value.to_string());
        }
        "fanout.fallback_to_board_vias" => match parse_on_off(value) {
            Some(v) => layer.fanout_fallback_to_board_vias = Some(v),
            None => warn_bad_value(layer, property, value),
        },
        "fanout.timeout" | "fanout.timeout_string" => {
            layer.fanout_timeout_string = Some(value.to_string());
        }
        // The M4-T9 optimizer group (Java `OptimizerSettings`
        // SerializedName + field-name spellings, mirroring the fanout
        // face; `improvement_threshold` never reaches this match — the
        // quirk block above consumes every spelling that ends with it
        // and writes `layer.improvement_threshold`).
        "optimizer.enabled" => match parse_on_off(value) {
            Some(v) => layer.optimizer_enabled = Some(v),
            None => warn_bad_value(layer, property, value),
        },
        "optimizer.algorithm" => {
            layer.optimizer_algorithm = Some(value.to_string());
        }
        "optimizer.max_passes" | "optimizer.maxPasses" => match value.parse::<i32>() {
            Ok(v) => layer.optimizer_max_passes = Some(v),
            Err(_) => warn_bad_value(layer, property, value),
        },
        "optimizer.max_items" | "optimizer.maxItems" => match value.parse::<i32>() {
            Ok(v) => layer.optimizer_max_items = Some(v),
            Err(_) => warn_bad_value(layer, property, value),
        },
        "optimizer.max_threads" | "optimizer.maxThreads" => match value.parse::<i32>() {
            Ok(v) => layer.optimizer_max_threads = Some(v),
            Err(_) => warn_bad_value(layer, property, value),
        },
        "optimizer.threads" => match value.parse::<i32>() {
            Ok(v) => layer.optimizer_threads = Some(v),
            Err(_) => warn_bad_value(layer, property, value),
        },
        "optimizer.enable_preflight_guards" | "optimizer.enablePreflightGuards" => {
            match parse_on_off(value) {
                Some(v) => layer.optimizer_enable_preflight_guards = Some(v),
                None => warn_bad_value(layer, property, value),
            }
        }
        "optimizer.max_consecutive_failures" | "optimizer.maxConsecutiveFailures" => {
            match value.parse::<i32>() {
                Ok(v) => layer.optimizer_max_consecutive_failures = Some(v),
                Err(_) => warn_bad_value(layer, property, value),
            }
        }
        "optimizer.max_consecutive_failures_pass1" | "optimizer.maxConsecutiveFailuresPass1" => {
            match value.parse::<i32>() {
                Ok(v) => layer.optimizer_max_consecutive_failures_pass1 = Some(v),
                Err(_) => warn_bad_value(layer, property, value),
            }
        }
        "optimizer.additional_ripup_cost_factor_at_start"
        | "optimizer.additionalRipupCostFactorAtStart" => match value.parse::<i32>() {
            Ok(v) => layer.optimizer_additional_ripup_cost_factor_at_start = Some(v),
            Err(_) => warn_bad_value(layer, property, value),
        },
        "optimizer.trace_ripup_cost_factor" | "optimizer.traceRipupCostFactor" => {
            match value.parse::<f32>() {
                Ok(v) => layer.optimizer_trace_ripup_cost_factor = Some(v),
                Err(_) => warn_bad_value(layer, property, value),
            }
        }
        "optimizer.max_autoroute_passes" | "optimizer.maxAutoroutePasses" => {
            match value.parse::<i32>() {
                Ok(v) => layer.optimizer_max_autoroute_passes = Some(v),
                Err(_) => warn_bad_value(layer, property, value),
            }
        }
        "optimizer.timeout" | "optimizer.timeout_string" | "optimizer.timeoutString" => {
            layer.optimizer_timeout_string = Some(value.to_string());
        }
        _ => {
            // Any other router.*/optimizer.* path: Java tries the
            // reflection walk and fails; the subset WARNs and continues.
            layer.warnings.push(format!(
                "Failed to apply CLI router setting: {property}: unsupported path \
                 in the route subset"
            ));
        }
    }
}

/// `LegacyRouterSettingsBridge.canonicalCliPath` (`:24-39`): the path is
/// lowercased; the six deprecated flat autorouter keys gain the
/// `autorouter.` prefix.
#[must_use]
pub fn canonical_cli_path(router_relative_path: &str) -> String {
    let path = router_relative_path.to_lowercase();
    match path.as_str() {
        "enabled"
        | "max_passes"
        | "algorithm"
        | "max_items"
        | "save_intermediate_stages"
        | "ignore_net_classes" => format!("autorouter.{path}"),
        _ => path,
    }
}

/// `LegacyRouterSettingsBridge.isDeprecatedFlatAutorouterPath` (`:42-45`).
#[must_use]
pub fn is_deprecated_flat_autorouter_path(router_relative_path: &str) -> bool {
    matches!(
        router_relative_path.to_lowercase().as_str(),
        "enabled"
            | "algorithm"
            | "max_passes"
            | "max_items"
            | "save_intermediate_stages"
            | "ignore_net_classes"
    )
}

/// The on/off/true/false/1/0 value face (`DsnFile.readOnOffScope`-
/// adjacent + `ReflectionUtil.convertValue`'s "0"/"1" bool arm).
/// The M7-T6 pair-declaration grammar: comma-separated `NET_A:NET_B`
/// net-name pairs (each name trimmed, non-empty). `None` = malformed
/// (the caller warns and continues).
fn parse_pairs(value: &str) -> Option<Vec<(String, String)>> {
    let mut pairs = Vec::new();
    for entry in value.split(',') {
        let entry = entry.trim();
        let (name_a, name_b) = entry.split_once(':')?;
        let name_a = name_a.trim();
        let name_b = name_b.trim();
        if name_a.is_empty() || name_b.is_empty() {
            return None;
        }
        pairs.push((name_a.to_string(), name_b.to_string()));
    }
    if pairs.is_empty() {
        return None;
    }
    Some(pairs)
}

fn parse_on_off(value: &str) -> Option<bool> {
    match value.trim().to_lowercase().as_str() {
        "on" | "true" | "1" => Some(true),
        "off" | "false" | "0" => Some(false),
        _ => None,
    }
}

fn warn_bad_value(layer: &mut CliLayer, property: &str, value: &str) {
    layer.warnings.push(format!(
        "Failed to apply CLI router setting: {property}: For input string: \"{value}\""
    ));
}

// ---------------------------------------------------------------------------
// the DSN layer (DsnFileSettings.java port)
// ---------------------------------------------------------------------------

/// One layer's slice of the DSN settings source — the NON-NULL fields of
/// the extracted `RouterSettings.layers[i]` entry (set only when the
/// layer_rule sub-scope appeared; the parse NEVER sets bend cost).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DsnLayerSlot {
    /// `setLayerActive` (`(active on|off)`).
    pub routable: Option<bool>,
    /// `setPreferredDirectionIsHorizontal` (`(preferred_direction ...)`).
    pub preferred_direction_is_horizontal: Option<bool>,
    /// The parse never sets bend cost (no DSN scope does) — always None.
    pub bend_cost: Option<f64>,
}

/// The DSN settings source (priority 20): only the fields the
/// `(autoroute_settings ...)` scope ACTUALLY set (anchors trap 7 —
/// DsnFileSettings.java read in full; its only other opinion is the
/// layer-count seed).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DsnLayer {
    /// `(autoroute on|off)` -> `setRunRouter` -> `autorouter.enabled`;
    /// ALWAYS set when the scope exists (the post-loop assignment).
    pub run_router: Option<bool>,
    /// `(postroute on|off)` -> `optimizer.enabled`; ALWAYS set when the
    /// scope exists.
    pub run_optimizer: Option<bool>,
    /// `(vias on|off)`.
    pub vias_allowed: Option<bool>,
    /// `(via_costs n)` (clamped `max(v, 1)` at the parse).
    pub via_costs: Option<i32>,
    /// `(plane_via_costs n)`.
    pub plane_via_costs: Option<i32>,
    /// `(start_ripup_costs n)`.
    pub start_ripup_costs: Option<i32>,
    /// The layer-count seed (`DsnFileSettings.java:47-52`: sized arrays
    /// "well before applyBoardSpecificOptimizations"). 0 = no opinion.
    pub layer_count: usize,
    /// Per-layer slots, sized to the PARSE's layer count when the scope
    /// exists (else empty — the seed resizes to `layer_count`).
    pub layers: Vec<DsnLayerSlot>,
}

impl DsnLayer {
    /// `DsnFileSettings(inputStream, filename)`: the metadata's
    /// autoroute settings (None when no scope was read — Java passes a
    /// null settings object and `copyFields` copies nothing) + the layer
    /// count seed.
    #[must_use]
    pub fn from_metadata(
        autoroute_settings: Option<&AutorouteSettingsIr>,
        layer_count: usize,
    ) -> Self {
        let mut layer = Self {
            layer_count,
            ..Self::default()
        };
        if let Some(settings) = autoroute_settings {
            layer.run_router = Some(settings.run_router);
            layer.run_optimizer = Some(settings.run_optimizer);
            // The set-flags are the null-vs-set surface: only a scope
            // that appeared copies (copyFields' non-null test).
            if settings.vias_allowed_set {
                layer.vias_allowed = Some(settings.vias_allowed);
            }
            if settings.via_costs_set {
                layer.via_costs = Some(settings.via_costs);
            }
            if settings.plane_via_costs_set {
                layer.plane_via_costs = Some(settings.plane_via_costs);
            }
            if settings.start_ripup_costs_set {
                layer.start_ripup_costs = Some(settings.start_ripup_costs);
            }
            layer.layers = settings
                .layer_rules
                .iter()
                .map(|rule| DsnLayerSlot {
                    routable: rule.active_set.then_some(rule.active),
                    preferred_direction_is_horizontal: rule
                        .preferred_direction_is_horizontal_set
                        .then_some(rule.preferred_direction_is_horizontal),
                    bend_cost: None,
                })
                .collect();
            // The parsed per-layer trace costs are dropped HERE (bug-compat
            // fact 1): the geometry pass re-derives every row unconditionally.
        }
        layer
    }
}

// ---------------------------------------------------------------------------
// the merged model + the geometry pass
// ---------------------------------------------------------------------------

/// One layer's merged slot — Java `LayerSettings` (all nullable).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MergedLayerSlot {
    /// Java `routable` (`getLayerActive`).
    pub routable: Option<bool>,
    /// Java `preferredDirectionHorizontal`.
    pub preferred_direction_is_horizontal: Option<bool>,
    /// Java `bendCost`.
    pub bend_cost: Option<f64>,
}

impl From<DsnLayerSlot> for MergedLayerSlot {
    fn from(slot: DsnLayerSlot) -> Self {
        Self {
            routable: slot.routable,
            preferred_direction_is_horizontal: slot.preferred_direction_is_horizontal,
            bend_cost: slot.bend_cost,
        }
    }
}

/// `DefaultSettings.DEFAULT_COPPER_TO_EDGE_CLEARANCE_UM`
/// (`DefaultSettings.java:83`) — the CLI-default copper-to-edge
/// clearance in µm that drives the load-time outline promotion (the
/// host flow's `apply_copper_to_edge_clearance_override`; in the
/// epic-cli crate since this module's M9-T1 move here).
pub const DEFAULT_COPPER_TO_EDGE_CLEARANCE_UM: f64 = 250.0;

/// The merged `RouterSettings` — the T13-consumed subset, all nullable,
/// resolved Default(0) -> Dsn(20) -> Cli(60). Defaults come from
/// `DefaultSettings.getSettings()` (the single authoritative source).
#[derive(Clone, Debug, PartialEq)]
pub struct MergedSettings {
    /// `autorouter.enabled`.
    pub autorouter_enabled: Option<bool>,
    /// `autorouter.maxPasses` (0 = no limit).
    pub max_passes: Option<i32>,
    /// `autorouter.maxItems`.
    pub max_items: Option<i32>,
    /// `maxThreads` — the Java-parity validation surface (the
    /// `max(1, cores-1)` default, the `< 0`/`> cores` warn arms). The
    /// executor's DEFAULT stays 1 — `route.rs` reads the EXPLICIT CLI
    /// face into `BatchSettings.max_threads`, not this post-validate
    /// value (M5-T7).
    pub max_threads: Option<i32>,
    /// `router.plane_island_clamp` — the M6-T6 Rust-only clamp flag.
    /// No Java/DSN source writes it (CLI source only); absent = OFF.
    pub plane_island_clamp: Option<bool>,
    /// `router.congestion_global` — the M6-T7 master switch (absent =
    /// OFF; the T6 seam precedent).
    pub congestion_global: Option<bool>,
    /// `router.congestion_global.pattern` — the pattern sub-flag
    /// (absent = OFF).
    pub congestion_global_pattern: Option<bool>,
    /// `router.congestion_global.pathfinder` — the M6-T8 scheduler
    /// sub-flag (absent = OFF; requires the master).
    pub congestion_global_pathfinder: Option<bool>,
    /// `router.push_shove` — the M6-T9 push-and-shove insertion flag
    /// (absent = OFF; top-level, no master).
    pub push_shove: Option<bool>,
    /// `router.tuning` — the M7-T2 tri-state (absent = input-driven
    /// activation; `on` = force ON; `off` = kill-switch). CLI-only —
    /// no DSN/default source writes it (the Default seed is `None`).
    pub tuning: Option<bool>,
    /// `router.tuning.meander` — the M7-T4 meander sub-flag (absent =
    /// rides the resolved tuning activation; `on` = force the stage;
    /// `off` = kill the stage alone). CLI-only — no DSN/default source
    /// writes it (the Default seed is `None`).
    pub tuning_meander: Option<bool>,
    /// `router.tuning.pairs` — the M7-T6 pair declaration list
    /// (absent = no pairs; CLI-only, the Default seed is `None`).
    pub tuning_pairs: Option<Vec<(String, String)>>,
    /// `router.gloss.bus` — the M8-T3 gloss-bus tri-state (absent =
    /// OFF; CLI-only — no DSN/default source writes it, the Default
    /// seed is `None`).
    pub gloss_bus: Option<bool>,
    /// `router.gloss.flow` — the M8-T4 gloss-flow tri-state (absent =
    /// OFF; CLI-only — no DSN/default source writes it, the Default
    /// seed is `None`).
    pub gloss_flow: Option<bool>,
    /// `router.gloss.via_place` — the M8-T5 gloss via-place tri-state
    /// (absent = OFF; CLI-only — no DSN/default source writes it, the
    /// Default seed is `None`).
    pub gloss_via_place: Option<bool>,
    /// `router.gloss.teardrops` — the M8-T6 gloss teardrops tri-state
    /// (absent = OFF; CLI-only — no DSN/default source writes it, the
    /// Default seed is `None`).
    pub gloss_teardrops: Option<bool>,
    /// `viasAllowed`.
    pub vias_allowed: Option<bool>,
    /// `automaticNeckdown` — DefaultSettings TRUE (the bare-ctor default
    /// is false; the T13 flow always merges over DefaultSettings).
    pub automatic_neckdown: Option<bool>,
    /// `tracePullTightAccuracy`.
    pub trace_pull_tight_accuracy: Option<i32>,
    /// `strictDrc`.
    pub strict_drc: Option<bool>,
    /// `copperToEdgeClearanceUm` (`RouterSettings.java:28`). No DSN or
    /// CLI source writes it in the frozen tree — only `DefaultSettings`
    /// (:155) seeds the default, so `merge` carries it through
    /// untouched and the field is consumed by the host flow's
    /// `apply_copper_to_edge_clearance_override` (epic-cli `route.rs`
    /// since this module's M9-T1 move into epic-engine).
    pub copper_to_edge_clearance_um: Option<f64>,
    /// `fanout.enabled`.
    pub fanout_enabled: Option<bool>,
    /// `fanout.maxPasses` (`fanout.max_passes` on the wire).
    pub fanout_max_passes: Option<i32>,
    /// `fanout.maxItems`.
    pub fanout_max_items: Option<i32>,
    /// `fanout.maxMillisecondsPerPin`.
    pub fanout_max_milliseconds_per_pin: Option<i64>,
    /// `fanout.ripupAllowed`.
    pub fanout_ripup_allowed: Option<bool>,
    /// `fanout.minEscapeLengthMm`.
    pub fanout_min_escape_length_mm: Option<f64>,
    /// `fanout.maxEscapeLengthMm`.
    pub fanout_max_escape_length_mm: Option<f64>,
    /// `fanout.startViaDiameterMm` — settings-surface parity only.
    pub fanout_start_via_diameter_mm: Option<f64>,
    /// `fanout.endViaDiameterMm` — settings-surface parity only.
    pub fanout_end_via_diameter_mm: Option<f64>,
    /// `fanout.pinSortingOrder` — verbatim (Java dispatches the raw
    /// string; unknown values fall to the `pinIndex` tie-break).
    pub fanout_pin_sorting_order: Option<String>,
    /// `fanout.fallbackToBoardVias`.
    pub fanout_fallback_to_board_vias: Option<bool>,
    /// `fanout.timeout` — the stage wall deadline string.
    pub fanout_timeout_string: Option<String>,
    /// `optimizer.enabled` — the stage gate (`getRunOptimizer()`).
    pub optimizer_enabled: Option<bool>,
    /// `optimizer.algorithm`.
    pub optimizer_algorithm: Option<String>,
    /// `optimizer.maxPasses` (None = unlimited).
    pub optimizer_max_passes: Option<i32>,
    /// `optimizer.maxItems`.
    pub optimizer_max_items: Option<i32>,
    /// `optimizer.maxThreads` — parsed but unused (sequential port).
    pub optimizer_max_threads: Option<i32>,
    /// `optimizer.threads` — the M8-T7 RUST-ONLY tri-state (absent =
    /// the 1-thread parity face; CLI-only — no DSN/default source
    /// writes it, the Default seed is `None`).
    pub optimizer_threads: Option<i32>,
    /// `optimizer.improvementThreshold` — the pass-stop percentage.
    pub improvement_threshold: Option<f32>,
    /// `optimizer.enablePreflightGuards` — only `Some(false)` bypasses
    /// the guards (Java `Boolean.FALSE.equals`).
    pub optimizer_enable_preflight_guards: Option<bool>,
    /// `optimizer.maxConsecutiveFailures` (None → the 50 fallback).
    pub optimizer_max_consecutive_failures: Option<i32>,
    /// `optimizer.maxConsecutiveFailuresPass1` (None → the 12 fallback).
    pub optimizer_max_consecutive_failures_pass1: Option<i32>,
    /// `optimizer.additionalRipupCostFactorAtStart`.
    pub optimizer_additional_ripup_cost_factor_at_start: Option<i32>,
    /// `optimizer.traceRipupCostFactor`.
    pub optimizer_trace_ripup_cost_factor: Option<f32>,
    /// `optimizer.maxAutoroutePasses`.
    pub optimizer_max_autoroute_passes: Option<i32>,
    /// `optimizer.timeout`.
    pub optimizer_timeout_string: Option<String>,
    /// The `scoring` box (deep-merged; defaults from DefaultSettings).
    pub scoring: RoutingCostSettings,
    /// The `routerScoring` box.
    pub router_scoring: RouterScoreSettings,
    /// The `optimizerScoring` box (deep-merged; defaults from
    /// DefaultSettings — V2_LOWER_BOUND). The V2 formula's consumer is
    /// the M4 optimizer stage; the resolved box rides
    /// [`ResolvedRouteSettings::scoring`] from T8 on.
    pub optimizer_scoring: OptimizerScoreSettings,
    /// The layer slots (sized to the BOARD at the geometry pass).
    pub layers: Vec<MergedLayerSlot>,
    /// Java `scoring.preferredDirectionTraceCost` — the per-layer
    /// preferred-direction costs, filled by the geometry pass (empty
    /// before it; `getTraceCosts` falls back to 1.0 per slot).
    pub preferred_trace_costs: Vec<f64>,
    /// Java `scoring.undesiredDirectionTraceCost`.
    pub undesired_trace_costs: Vec<f64>,
}

impl Default for MergedSettings {
    /// `DefaultSettings.getSettings()` — the priority-0 base (the layer
    /// arrays are intentionally EMPTY; their SIZE comes from the
    /// board/DSN at the geometry pass). The fanout group seeds are
    /// `DefaultSettings.java:163-177` verbatim (also stated once more
    /// in `FanoutSettingsIr::default` — the resolver consumes THESE,
    /// the IR Default is the test-side armor).
    fn default() -> Self {
        Self {
            autorouter_enabled: Some(true),
            max_passes: Some(0),
            max_items: Some(i32::MAX),
            max_threads: None,
            plane_island_clamp: None,
            congestion_global: None,
            congestion_global_pattern: None,
            congestion_global_pathfinder: None,
            push_shove: None,
            tuning: None,
            tuning_meander: None,
            tuning_pairs: None,
            gloss_bus: None,
            gloss_flow: None,
            gloss_via_place: None,
            gloss_teardrops: None,
            vias_allowed: Some(true),
            automatic_neckdown: Some(true),
            trace_pull_tight_accuracy: Some(500),
            strict_drc: Some(false),
            // `DefaultSettings.java:83` + `:155` verbatim — the
            // always-live CLI default that drives the load-time
            // copper-to-edge override on every manager DSN load.
            copper_to_edge_clearance_um: Some(DEFAULT_COPPER_TO_EDGE_CLEARANCE_UM),
            fanout_enabled: Some(true),
            fanout_max_passes: Some(20),
            fanout_max_items: Some(i32::MAX),
            fanout_max_milliseconds_per_pin: Some(10_000),
            fanout_ripup_allowed: Some(true),
            fanout_min_escape_length_mm: Some(2.5),
            fanout_max_escape_length_mm: Some(4.5),
            fanout_start_via_diameter_mm: Some(0.250),
            fanout_end_via_diameter_mm: Some(0.250),
            fanout_pin_sorting_order: Some("outer_first".to_string()),
            fanout_fallback_to_board_vias: Some(true),
            fanout_timeout_string: None,
            optimizer_enabled: Some(true),
            optimizer_algorithm: Some("freerouting-optimizer".to_string()),
            // `DefaultSettings.java:180-191` verbatim (the optimizer
            // family's defaults; maxThreads = max(1, cores - 1) is
            // computed at seed time like Java's).
            optimizer_max_passes: Some(100),
            optimizer_max_items: Some(i32::MAX),
            optimizer_max_threads: Some({
                let cores = std::thread::available_parallelism()
                    .map_or(1usize, std::num::NonZeroUsize::get);
                i32::try_from(cores.saturating_sub(1).max(1)).unwrap_or(i32::MAX)
            }),
            // The M8-T7 tri-state: no default source — absent = the
            // 1-thread parity face (the SettingsMerger-family invariant;
            // defaults live only on the resolved faces).
            optimizer_threads: None,
            improvement_threshold: Some(2.5),
            optimizer_enable_preflight_guards: Some(true),
            optimizer_max_consecutive_failures: Some(50),
            optimizer_max_consecutive_failures_pass1: Some(12),
            optimizer_additional_ripup_cost_factor_at_start: Some(10),
            optimizer_trace_ripup_cost_factor: Some(0.6),
            optimizer_max_autoroute_passes: Some(6),
            optimizer_timeout_string: None,
            scoring: default_routing_cost_settings(),
            router_scoring: default_router_score_settings(),
            optimizer_scoring: default_optimizer_score_settings(),
            layers: Vec::new(),
            preferred_trace_costs: Vec::new(),
            undesired_trace_costs: Vec::new(),
        }
    }
}

/// `DefaultSettings`' routerScoring box (`sources/DefaultSettings.java:
/// 89-116, 203-211`): V2_CONTINUOUS with the f32-divided weights.
#[must_use]
pub fn default_router_score_settings() -> RouterScoreSettings {
    RouterScoreSettings {
        version: RouterScoringVersion::V2Continuous,
        unrouted_free_fraction: Some(0.5),
        unrouted_first_half_weight: Some(1000.0 / 3.0),
        unrouted_second_half_weight: Some(2000.0 / 3.0),
        clearance_violation_count_weight: Some(25.0),
        clearance_violation_depth_weight: Some(300.0),
        clearance_violation_depth_scale: Some(1000.0),
    }
}

/// `DefaultSettings`' optimizerScoring box (`sources/DefaultSettings.java:
/// 92-121, 215-220` verbatim): V2_LOWER_BOUND with the 1000/2000/500
/// excess weights and the 1.0/1.0 floors.
#[must_use]
pub fn default_optimizer_score_settings() -> OptimizerScoreSettings {
    OptimizerScoreSettings {
        version: OptimizerScoringVersion::V2LowerBound,
        excess_wire_length_weight: Some(1000.0),
        excess_via_weight: Some(2000.0),
        excess_bend_weight: Some(500.0),
        length_floor: Some(1.0),
        difficulty_scale_floor: Some(1.0),
    }
}

/// The merge: Default(0) -> Dsn(20) -> Cli(60), deep-merge semantics
/// (`copyFields`): a source field copies ONLY where it is non-null (the
/// Option's Some). Order matters — later sources overwrite earlier ones.
#[must_use]
pub fn merge(defaults: &MergedSettings, dsn: &DsnLayer, cli: &CliLayer) -> MergedSettings {
    let mut merged = defaults.clone();

    // --- the DSN layer (priority 20) ---
    if let Some(v) = dsn.run_router {
        merged.autorouter_enabled = Some(v);
    }
    if let Some(v) = dsn.run_optimizer {
        merged.optimizer_enabled = Some(v);
    }
    if let Some(v) = dsn.vias_allowed {
        merged.vias_allowed = Some(v);
    }
    if let Some(v) = dsn.via_costs {
        merged.scoring.via_costs = Some(v);
    }
    if let Some(v) = dsn.plane_via_costs {
        merged.scoring.plane_via_costs = Some(v);
    }
    if let Some(v) = dsn.start_ripup_costs {
        merged.scoring.start_ripup_costs = Some(v);
    }
    // The layer-count seed (`DsnFileSettings.java:47-52`): sized slots
    // BEFORE the geometry pass; the scope's per-layer opinions ride in
    // `dsn.layers` when the scope existed.
    if !dsn.layers.is_empty() {
        merged.layers = dsn.layers.clone().into_iter().map(Into::into).collect();
    } else if dsn.layer_count > 0 && merged.layers.is_empty() {
        merged.layers = vec![MergedLayerSlot::default(); dsn.layer_count];
    }

    // --- the CLI layer (priority 60) ---
    if let Some(v) = cli.autorouter_enabled {
        merged.autorouter_enabled = Some(v);
    }
    if let Some(v) = cli.max_passes {
        merged.max_passes = Some(v);
    }
    if let Some(v) = cli.max_items {
        merged.max_items = Some(v);
    }
    if let Some(v) = cli.max_threads {
        merged.max_threads = Some(v);
    }
    if let Some(v) = cli.plane_island_clamp {
        merged.plane_island_clamp = Some(v);
    }
    if let Some(v) = cli.congestion_global {
        merged.congestion_global = Some(v);
    }
    if let Some(v) = cli.congestion_global_pattern {
        merged.congestion_global_pattern = Some(v);
    }
    if let Some(v) = cli.congestion_global_pathfinder {
        merged.congestion_global_pathfinder = Some(v);
    }
    if let Some(v) = cli.push_shove {
        merged.push_shove = Some(v);
    }
    if let Some(v) = cli.tuning {
        merged.tuning = Some(v);
    }
    if let Some(v) = cli.tuning_meander {
        merged.tuning_meander = Some(v);
    }
    if let Some(v) = cli.tuning_pairs.clone() {
        merged.tuning_pairs = Some(v);
    }
    if let Some(v) = cli.gloss_bus {
        merged.gloss_bus = Some(v);
    }
    if let Some(v) = cli.gloss_flow {
        merged.gloss_flow = Some(v);
    }
    if let Some(v) = cli.gloss_via_place {
        merged.gloss_via_place = Some(v);
    }
    if let Some(v) = cli.gloss_teardrops {
        merged.gloss_teardrops = Some(v);
    }
    if let Some(v) = cli.vias_allowed {
        merged.vias_allowed = Some(v);
    }
    if let Some(v) = cli.automatic_neckdown {
        merged.automatic_neckdown = Some(v);
    }
    if let Some(v) = cli.trace_pull_tight_accuracy {
        merged.trace_pull_tight_accuracy = Some(v);
    }
    if let Some(v) = cli.strict_drc {
        merged.strict_drc = Some(v);
    }
    if let Some(v) = cli.fanout_enabled {
        merged.fanout_enabled = Some(v);
    }
    if let Some(v) = cli.fanout_max_passes {
        merged.fanout_max_passes = Some(v);
    }
    if let Some(v) = cli.fanout_max_items {
        merged.fanout_max_items = Some(v);
    }
    if let Some(v) = cli.fanout_max_milliseconds_per_pin {
        merged.fanout_max_milliseconds_per_pin = Some(v);
    }
    if let Some(v) = cli.fanout_ripup_allowed {
        merged.fanout_ripup_allowed = Some(v);
    }
    if let Some(v) = cli.fanout_min_escape_length_mm {
        merged.fanout_min_escape_length_mm = Some(v);
    }
    if let Some(v) = cli.fanout_max_escape_length_mm {
        merged.fanout_max_escape_length_mm = Some(v);
    }
    if let Some(v) = cli.fanout_start_via_diameter_mm {
        merged.fanout_start_via_diameter_mm = Some(v);
    }
    if let Some(v) = cli.fanout_end_via_diameter_mm {
        merged.fanout_end_via_diameter_mm = Some(v);
    }
    if let Some(v) = cli.fanout_pin_sorting_order.clone() {
        merged.fanout_pin_sorting_order = Some(v);
    }
    if let Some(v) = cli.fanout_fallback_to_board_vias {
        merged.fanout_fallback_to_board_vias = Some(v);
    }
    if let Some(v) = cli.fanout_timeout_string.clone() {
        merged.fanout_timeout_string = Some(v);
    }
    // --- the M4-T9 optimizer family (CLI priority 60) ---
    if let Some(v) = cli.optimizer_enabled {
        merged.optimizer_enabled = Some(v);
    }
    if let Some(v) = cli.optimizer_algorithm.clone() {
        merged.optimizer_algorithm = Some(v);
    }
    if let Some(v) = cli.optimizer_max_passes {
        merged.optimizer_max_passes = Some(v);
    }
    if let Some(v) = cli.optimizer_max_items {
        merged.optimizer_max_items = Some(v);
    }
    if let Some(v) = cli.optimizer_max_threads {
        merged.optimizer_max_threads = Some(v);
    }
    if let Some(v) = cli.optimizer_threads {
        merged.optimizer_threads = Some(v);
    }
    if let Some(v) = cli.improvement_threshold {
        merged.improvement_threshold = Some(v);
    }
    if let Some(v) = cli.optimizer_enable_preflight_guards {
        merged.optimizer_enable_preflight_guards = Some(v);
    }
    if let Some(v) = cli.optimizer_max_consecutive_failures {
        merged.optimizer_max_consecutive_failures = Some(v);
    }
    if let Some(v) = cli.optimizer_max_consecutive_failures_pass1 {
        merged.optimizer_max_consecutive_failures_pass1 = Some(v);
    }
    if let Some(v) = cli.optimizer_additional_ripup_cost_factor_at_start {
        merged.optimizer_additional_ripup_cost_factor_at_start = Some(v);
    }
    if let Some(v) = cli.optimizer_trace_ripup_cost_factor {
        merged.optimizer_trace_ripup_cost_factor = Some(v);
    }
    if let Some(v) = cli.optimizer_max_autoroute_passes {
        merged.optimizer_max_autoroute_passes = Some(v);
    }
    if let Some(v) = cli.optimizer_timeout_string.clone() {
        merged.optimizer_timeout_string = Some(v);
    }
    if let Some(v) = cli.via_costs {
        merged.scoring.via_costs = Some(v);
    }
    if let Some(v) = cli.plane_via_costs {
        merged.scoring.plane_via_costs = Some(v);
    }
    if let Some(v) = cli.start_ripup_costs {
        merged.scoring.start_ripup_costs = Some(v);
    }
    if let Some(v) = cli.router_scoring_version {
        merged.router_scoring.version = v;
    }
    if let Some(v) = cli.optimizer_scoring_version {
        merged.optimizer_scoring.version = match v {
            OptimScoringVersion::V1Legacy => OptimizerScoringVersion::V1Legacy,
            OptimScoringVersion::V2LowerBound => OptimizerScoringVersion::V2LowerBound,
        };
    }
    merged
}

/// Java `RouterSettings.validate()` (`RouterSettings.java:938-977`) —
/// the merger's TRAILING step (`SettingsMerger.java:189`
/// `mergedSettings.validate()`), which the flow runs immediately after
/// [`merge`] and BEFORE
/// [`apply_board_specific_optimizations`]. Normalizes the merged
/// values in place and returns the warn rows in emission order:
///
/// * `maxPasses` `< 0`, or `> 9999` when not the `i32::MAX` sentinel,
///   warns and resets to 0 (= unlimited).
/// * `maxThreads`: `None` takes the default `max(1, cores - 1)`
///   (`defaultMaxThreads`, `:127-129`); `< 0` warns and takes the
///   default; `> cores` warns and caps (0 survives — Java only
///   rejects the two faces).
/// * `tracePullTightAccuracy` `< 1` warns and resets to 500 (Java
///   unboxes directly; the merged flow always carries the
///   DefaultSettings 500, so the `None` arm is deserialization armor).
#[must_use]
pub fn validate(merged: &mut MergedSettings) -> Vec<String> {
    let mut warnings = Vec::new();

    if let Some(max_passes) = merged.max_passes
        && (max_passes < 0 || (max_passes > 9999 && max_passes != i32::MAX))
    {
        warnings.push(format!(
            "Invalid maxPasses value: {max_passes}, using default 0 (no limit)"
        ));
        merged.max_passes = Some(0);
    }

    let available = {
        let cores =
            std::thread::available_parallelism().map_or(1usize, std::num::NonZeroUsize::get);
        i32::try_from(cores).unwrap_or(i32::MAX)
    };
    let default_max_threads = (available - 1).max(1);
    // DEVIATION (M5-T7, F5): Java's normalizeMaxThreads maps `0 →
    // cores` (`RouterSettings.java:139-141`, the `==0` arm); the Rust
    // headless flow DELIBERATELY keeps `0` unnormalized — it means no
    // explicit executor face, and the executor (which reads the
    // EXPLICIT CLI value only, `route.rs`) then stays SEQUENTIAL, so
    // the golden `--threads 1` default can never be accidentally
    // parallel.
    match merged.max_threads {
        None => merged.max_threads = Some(default_max_threads),
        Some(value) if value < 0 => {
            warnings.push(format!(
                "Invalid maxThreads value: {value}, using {default_max_threads}"
            ));
            merged.max_threads = Some(default_max_threads);
        }
        Some(value) if value > available => {
            warnings.push(format!(
                "Invalid maxThreads value: {value}, capping at {available}"
            ));
            merged.max_threads = Some(available);
        }
        Some(_) => {}
    }

    let accuracy = merged.trace_pull_tight_accuracy.unwrap_or(0);
    if accuracy < 1 {
        warnings.push(format!(
            "Invalid tracePullTightAccuracy value: {accuracy}, using default 500"
        ));
        merged.trace_pull_tight_accuracy = Some(500);
    }

    warnings
}

/// `RouterSettings.applyBoardSpecificOptimizations`
/// (`RouterSettings.java:267-360`), ported over the merged model. In the
/// headless flow this runs UNCONDITIONALLY on the merged settings
/// (bug-compat fact 1: the applied flag is private transient and never
/// survives the merge), so the port carries no flag and always
/// initializes the per-layer trace costs.
///
/// `board.layers()` supplies the layer count + the signal flags; the
/// bounding box supplies the aspect ratio (Java reads
/// `board.boundingBox.width()/height()` directly — a built board always
/// has one).
pub fn apply_board_specific_optimizations(
    settings: &mut MergedSettings,
    board: &epic_board::board::Board,
) {
    let layer_structure = board.layers();
    let layer_count = layer_structure.layers.len();
    let bounds = board
        .bounding_box()
        .expect("a built board always has a bounding box (Java would NPE)");
    let horizontal_width = f64::from(bounds.width());
    let vertical_width = f64::from(bounds.height());

    // Additional costs against the preferred direction, one digit behind
    // the decimal point (`:117-121`; Java Math.round(double) =
    // floor(x + 0.5)).
    let horizontal_add = 0.1 * java_math_round(10.0 * horizontal_width / vertical_width);
    let vertical_add = 0.1 * java_math_round(10.0 * vertical_width / horizontal_width);

    // The layer slots: resize to the board preserving existing entries
    // (`:124-142` — the oldLayers[i] != null carry-over is the Vec resize
    // semantics here since slots are never constructed null).
    settings
        .layers
        .resize_with(layer_count, MergedLayerSlot::default);

    // Null-guarded cost defaults (`:154-161`; the DefaultSettings source
    // always carries them, the guards are deserialization armor).
    let default_preferred = settings
        .scoring
        .default_preferred_direction_trace_cost
        .unwrap_or(1.0);
    let default_undesired = settings
        .scoring
        .default_undesired_direction_trace_cost
        .unwrap_or(1.0);
    let default_bend = settings.scoring.default_bend_cost;

    // `:163`: the seed direction comes from the board's aspect ratio.
    let mut current_preferred_horizontal = horizontal_width < vertical_width;

    // `:145-152`: the cost arrays are (re)allocated at board size.
    let mut preferred_costs = vec![0.0; layer_count];
    let mut undesired_costs = vec![0.0; layer_count];
    for (index, slot) in settings.layers.iter_mut().enumerate() {
        let is_signal = layer_structure.layers[index].is_signal;
        if is_signal {
            // Toggle FIRST, then use (`:167-169`).
            current_preferred_horizontal = !current_preferred_horizontal;
        }
        if !is_signal {
            // Non-signal layers are force-disabled (`:170-171`).
            slot.routable = Some(false);
        } else if slot.routable.is_none() {
            slot.routable = Some(true);
        }
        if slot.bend_cost.is_none() {
            // `:175-178`: written through UNCLAMPED (bug-compat fact 3 —
            // only the getter's default fallback clamps).
            slot.bend_cost = Some(default_bend.unwrap_or(0.0));
        }
        if slot.preferred_direction_is_horizontal.is_none() {
            slot.preferred_direction_is_horizontal = Some(current_preferred_horizontal);
        }
        // `:183-191`: ALWAYS initialize in the headless flow.
        preferred_costs[index] = default_preferred;
        undesired_costs[index] = default_undesired
            + if current_preferred_horizontal {
                horizontal_add
            } else {
                vertical_add
            };
    }
    // Outer-layer surcharge on >2-signal-layer boards (`:193-204`).
    let signal_layer_count = layer_structure.signal_layer_count();
    if signal_layer_count > 2 {
        let outer_add = 0.2 * f64::from(signal_layer_count);
        let last = layer_count - 1;
        preferred_costs[0] += outer_add;
        preferred_costs[last] += outer_add;
        undesired_costs[0] += outer_add;
        undesired_costs[last] += outer_add;
    }
    settings.preferred_trace_costs = preferred_costs;
    settings.undesired_trace_costs = undesired_costs;
}

/// Java `Math.round(double)`: `floor(x + 0.5)`.
fn java_math_round(value: f64) -> f64 {
    (value + 0.5).floor()
}

// ---------------------------------------------------------------------------
// the resolution product
// ---------------------------------------------------------------------------

/// The fully resolved route settings — the pieces the T13 flow hands to
/// `BatchSettings` / the manifest. Constructed AFTER the geometry pass
/// (`apply_board_specific_optimizations`) has run on the merged model.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedRouteSettings {
    /// The control IR (the resolved per-layer table materialized).
    pub router_settings: RouterSettingsIr,
    /// The score face.
    pub scoring: RouterSettingsScoring,
    /// `autorouter.maxPasses` (0 = unlimited — the driver's gate reads
    /// `Some(0)` as unlimited).
    pub max_passes: Option<i32>,
    /// `autorouter.maxItems`.
    pub max_items: Option<i32>,
    /// `isFanoutEnabled()`.
    pub fanout_enabled: bool,
    /// `isStrictDrc()`.
    pub strict_drc: bool,
    /// `getRunRouter()`.
    pub run_router: bool,
    /// `getRunOptimizer()` — `optimizer != null && enabled != null ?
    /// enabled : false` (`RouterSettings.java:564-566`).
    pub run_optimizer: bool,
    /// The resolved optimizer-settings group (the stage's reads).
    pub optimizer: OptimizerSettingsIr,
    /// `tracePullTightAccuracy`.
    pub pull_tight_accuracy: i32,
    /// `--deterministic-budgets` (default ON — anchors §6).
    pub deterministic_budgets: bool,
    /// The M6-T6 Rust-only clamp flag (`router.plane_island_clamp`,
    /// default OFF — dead code at defaults).
    pub plane_island_clamp: bool,
    /// The M6-T7 global-planning master switch
    /// (`router.congestion_global`, default OFF).
    pub congestion_global: bool,
    /// The M6-T7 pattern-routing sub-flag
    /// (`router.congestion_global.pattern`, default OFF; requires the
    /// master).
    pub congestion_global_pattern: bool,
    /// The M6-T8 PathFinder scheduler sub-flag
    /// (`router.congestion_global.pathfinder`, default OFF; requires
    /// the master).
    pub congestion_global_pathfinder: bool,
    /// The M6-T9 push-and-shove insertion flag
    /// (`router.push_shove`, default OFF; top-level, no master).
    pub push_shove: bool,
    /// The M7-T2 tuning tri-state, resolved VERBATIM (absent =
    /// input-driven activation — the effective face is computed at the
    /// board seam in `route.rs` where the board exists; `on` = force
    /// ON; `off` = kill-switch). NOT a plain bool: the input-driven
    /// default must survive resolution for the board-seam predicate to
    /// answer it.
    pub tuning: Option<bool>,
    /// The M7-T4 meander sub-flag (`router.tuning.meander`),
    /// resolved VERBATIM (absent = rides the resolved tuning
    /// activation at the board seam in route.rs; `on` = force the
    /// stage; `off` = kill the stage alone). NOT a plain bool: the
    /// input-driven default must survive resolution for the board-seam
    /// predicate to answer it.
    pub tuning_meander: Option<bool>,
    /// The M7-T6 pair declaration list, resolved VERBATIM (absent =
    /// no declared pairs — the pair face's activation input is this
    /// list alone). The name→number resolution happens at the board
    /// seam in route.rs where the netlist exists.
    pub tuning_pairs: Option<Vec<(String, String)>>,
    /// The M8-T3 gloss-bus flag (`router.gloss.bus`), resolved to its
    /// effective face: ON only when explicitly `on` (the pass has no
    /// input-driven activation — absent and `off` are both OFF, the
    /// two-regime law's default face).
    pub gloss_bus: bool,
    /// The M8-T4 gloss-flow flag (`router.gloss.flow`), resolved to
    /// its effective face: ON only when explicitly `on` (same face as
    /// the gloss-bus flag).
    pub gloss_flow: bool,
    /// The M8-T5 gloss via-place flag (`router.gloss.via_place`),
    /// resolved to its effective face: ON only when explicitly `on`
    /// (same face as the gloss-bus flag).
    pub gloss_via_place: bool,
    /// The M8-T6 gloss teardrops flag (`router.gloss.teardrops`),
    /// resolved to its effective face: ON only when explicitly `on`
    /// (same face as the gloss-bus flag).
    pub gloss_teardrops: bool,
}

impl ResolvedRouteSettings {
    /// Materializes the product from a MERGED model that has already
    /// been through `apply_board_specific_optimizations`. The Java
    /// getters this mirrors: `getTraceCosts()` (`:881-894`),
    /// `getBendCost(layer)` (`:696-707`), `getViaCosts` (null -> 1),
    /// `getViasAllowed` (null -> true), `getLayerActive` (null -> true),
    /// `getAutomaticNeckdown` (null -> false), `getStartRipupCosts`
    /// (null -> 1).
    #[must_use]
    pub fn resolve(merged: &MergedSettings, deterministic_budgets: Option<bool>) -> Self {
        // `getTraceCosts()`: horizontal = the preferred cost when the
        // layer's preferred direction is horizontal, else the against
        // cost; vertical = the other. (Post-geometry-pass the direction
        // slot is always filled; the `i % 2 == 1` fallback is the Java
        // parse-time default for un-geometry-passed models.)
        let trace_costs: Vec<ExpansionCostFactor> = merged
            .layers
            .iter()
            .enumerate()
            .map(|(index, slot)| {
                let horizontal = slot
                    .preferred_direction_is_horizontal
                    .unwrap_or(index % 2 == 1);
                let preferred = merged
                    .preferred_trace_costs
                    .get(index)
                    .copied()
                    .unwrap_or(1.0);
                let undesired = merged
                    .undesired_trace_costs
                    .get(index)
                    .copied()
                    .unwrap_or(1.0);
                if horizontal {
                    ExpansionCostFactor {
                        horizontal: preferred,
                        vertical: undesired,
                    }
                } else {
                    ExpansionCostFactor {
                        horizontal: undesired,
                        vertical: preferred,
                    }
                }
            })
            .collect();
        // `getBendCost(layer)`: a SET slot value returns UNCLAMPED; the
        // default fallback clamps into [0.0, 9.9]; a null default is 0.0.
        let bend_costs: Vec<f64> = merged
            .layers
            .iter()
            .map(|slot| {
                slot.bend_cost
                    .unwrap_or_else(|| settings_default_bend_cost(merged))
            })
            .collect();
        Self {
            router_settings: RouterSettingsIr {
                trace_costs,
                via_costs: merged.scoring.via_costs.unwrap_or(1),
                vias_allowed: merged.vias_allowed.unwrap_or(true),
                bend_costs,
                layer_active: merged
                    .layers
                    .iter()
                    .map(|slot| slot.routable.unwrap_or(true))
                    .collect(),
                automatic_neckdown: merged.automatic_neckdown.unwrap_or(false),
                start_ripup_costs: merged.scoring.start_ripup_costs.unwrap_or(1),
                // The fanout group resolves over the merged model; the
                // `unwrap_or` fallbacks are the DefaultSettings seeds
                // (`FanoutSettingsIr::default` states them again as the
                // test-side armor). The Java validate() step touches NO
                // fanout field (`RouterSettings.java:938-977`), so the
                // merged values flow through unvalidated, exactly like
                // Java's.
                fanout: FanoutSettingsIr {
                    enabled: merged.fanout_enabled.unwrap_or(true),
                    max_passes: merged.fanout_max_passes.unwrap_or(20),
                    max_items: merged.fanout_max_items.unwrap_or(i32::MAX),
                    max_milliseconds_per_pin: merged
                        .fanout_max_milliseconds_per_pin
                        .unwrap_or(10_000),
                    ripup_allowed: merged.fanout_ripup_allowed.unwrap_or(true),
                    min_escape_length_mm: merged.fanout_min_escape_length_mm,
                    max_escape_length_mm: merged.fanout_max_escape_length_mm,
                    start_via_diameter_mm: merged.fanout_start_via_diameter_mm,
                    end_via_diameter_mm: merged.fanout_end_via_diameter_mm,
                    pin_sorting_order: merged
                        .fanout_pin_sorting_order
                        .clone()
                        .unwrap_or_else(|| "outer_first".to_string()),
                    fallback_to_board_vias: merged.fanout_fallback_to_board_vias.unwrap_or(true),
                    timeout_string: merged.fanout_timeout_string.clone(),
                },
            },
            scoring: RouterSettingsScoring {
                scoring: Some(merged.scoring.clone()),
                router_scoring: Some(merged.router_scoring.clone()),
                optimizer_scoring: Some(merged.optimizer_scoring.clone()),
            },
            max_passes: merged.max_passes,
            max_items: merged.max_items,
            plane_island_clamp: merged.plane_island_clamp.unwrap_or(false),
            congestion_global: merged.congestion_global.unwrap_or(false),
            congestion_global_pattern: merged.congestion_global_pattern.unwrap_or(false),
            congestion_global_pathfinder: merged.congestion_global_pathfinder.unwrap_or(false),
            push_shove: merged.push_shove.unwrap_or(false),
            tuning: merged.tuning,
            tuning_meander: merged.tuning_meander,
            tuning_pairs: merged.tuning_pairs.clone(),
            gloss_bus: merged.gloss_bus.unwrap_or(false),
            gloss_flow: merged.gloss_flow.unwrap_or(false),
            gloss_via_place: merged.gloss_via_place.unwrap_or(false),
            gloss_teardrops: merged.gloss_teardrops.unwrap_or(false),
            fanout_enabled: merged.fanout_enabled.unwrap_or(true),
            strict_drc: merged.strict_drc.unwrap_or(false),
            run_router: merged.autorouter_enabled.unwrap_or(true),
            run_optimizer: merged.optimizer_enabled.unwrap_or(false),
            optimizer: OptimizerSettingsIr {
                algorithm: merged
                    .optimizer_algorithm
                    .clone()
                    .unwrap_or_else(|| "freerouting-optimizer".to_string()),
                max_passes: merged.optimizer_max_passes,
                max_items: merged.optimizer_max_items,
                improvement_threshold: merged.improvement_threshold,
                enable_preflight_guards: merged.optimizer_enable_preflight_guards,
                max_consecutive_failures: merged.optimizer_max_consecutive_failures,
                max_consecutive_failures_pass1: merged.optimizer_max_consecutive_failures_pass1,
                additional_ripup_cost_factor_at_start: merged
                    .optimizer_additional_ripup_cost_factor_at_start
                    .unwrap_or(10),
                trace_ripup_cost_factor: merged.optimizer_trace_ripup_cost_factor.unwrap_or(0.6),
                max_autoroute_passes: merged.optimizer_max_autoroute_passes.unwrap_or(6),
                timeout_string: merged.optimizer_timeout_string.clone(),
            },
            pull_tight_accuracy: merged.trace_pull_tight_accuracy.unwrap_or(500),
            deterministic_budgets: deterministic_budgets.unwrap_or(true),
        }
    }
}

/// The `getBendCost` DEFAULT fallback: `scoring.defaultBendCost` clamped
/// into `[MIN_BEND_COST, MAX_BEND_COST]` = `[0.0, 9.9]`
/// (`RouterSettings.java:17-18, 701-704`); null -> 0.0.
fn settings_default_bend_cost(merged: &MergedSettings) -> f64 {
    match merged.scoring.default_bend_cost {
        Some(v) => v.clamp(0.0, 9.9),
        None => 0.0,
    }
}

// ---------------------------------------------------------------------------
// the BatchSettings activation predicates (moved from epic-cli route.rs,
// M9-T1 — settings resolution, ResolvedRouteSettings -> BatchSettings)
// ---------------------------------------------------------------------------

/// The `BatchSettings` assembly from the resolved product — the flow's
/// step 4, separated so the wiring is directly pinnable. The
/// load-bearing line is the job-ctor derivation
/// `removeUnconnectedVias = !isFanoutEnabled()`: fanout-off ALSO flips
/// the batch tail sweep, so a silently broken coupling changes routing
/// outcomes, not just via counts.
///
/// `pub` for the harness's T15 first-divergence localizer: the in-process
/// detail pass must build its batch settings through THIS function, not a
/// copy, so the fanout coupling has exactly one source (the harness's
/// subprocess run is the real flow either way; this only keeps the
/// diagnostic re-run honest).
pub fn build_batch_settings(resolved: &ResolvedRouteSettings) -> BatchSettings {
    let mut batch = BatchSettings::new(resolved.router_settings.clone(), resolved.scoring.clone());
    batch.max_passes = resolved.max_passes;
    batch.max_items = resolved.max_items;
    batch.fanout_enabled = resolved.fanout_enabled;
    // The job ctor derives removeUnconnectedVias = !isFanoutEnabled().
    batch.remove_unconnected_vias = !resolved.fanout_enabled;
    batch.pull_tight_accuracy = resolved.pull_tight_accuracy;
    batch.strict_drc = resolved.strict_drc;
    batch.run_router = resolved.run_router;
    batch.deterministic_budgets = resolved.deterministic_budgets;
    // M6-T6: the Rust-only connectivity clamp (default OFF — dead code
    // at defaults; its route effect is measured at T8/T9, not here).
    batch.plane_island_clamp = resolved.plane_island_clamp;
    // M6-T7: the global-planning family (default OFF — dead code at
    // defaults; the settings-ON faces are beyond-Java).
    batch.congestion_global = resolved.congestion_global;
    batch.congestion_global_pattern = resolved.congestion_global_pattern;
    // M6-T8: the negotiated scheduler sub-flag (default OFF — dead
    // code at defaults; the settings-ON faces are beyond-Java).
    batch.congestion_global_pathfinder = resolved.congestion_global_pathfinder;
    // M6-T9: the push-and-shove insertion flag (default OFF — dead
    // code at defaults; the settings-ON faces are beyond-Java).
    batch.push_shove = resolved.push_shove;
    // The via-cost pair: the merged scoring box (DefaultSettings seeds
    // 50/5; the CLI/DSN layers can override).
    let scoring_box = resolved.scoring.scoring.clone().unwrap_or_default();
    batch.via_costs = scoring_box.via_costs.unwrap_or(50);
    batch.plane_via_costs = scoring_box.plane_via_costs.unwrap_or(5);
    // M8-T3: the gloss-bus activation (the resolved flag is the only
    // gate — no board seam needed, the pass is post-route). Default
    // OFF: the stage never runs, byte-identical.
    batch.bus_active = resolved.gloss_bus;
    // M8-T4: the gloss-flow activation (same seam as the bus flag).
    batch.flow_active = resolved.gloss_flow;
    // M8-T5: the gloss via-place activation (same seam as the flow
    // flag).
    batch.via_place_active = resolved.gloss_via_place;
    // M8-T6: the gloss teardrops activation (same seam as the via-place
    // flag).
    batch.teardrops_active = resolved.gloss_teardrops;
    batch
}

/// The M7-T2 tuning ACTIVATION: the effective face of the
/// `router.tuning` tri-state resolved against the BOARD (the
/// input-driven regime). The tuning regime exists as data (the
/// net-class length bounds, `epic_board::rules_surf`) plus this
/// predicate; the M7 honoring/meander/match/pair faces (T3+) read the
/// resolved [`BatchSettings::tuning_active`], never the tri-state.
///
/// - `Some(false)` — the kill-switch: OFF even where a declaration
///   would activate.
/// - `Some(true)` — the override: ON even on constraint-free input
///   (the flag family's explicit face).
/// - `None` (the default) — input-driven: ON iff the board resolves
///   ANY non-zero length bound (`BoardRules::has_length_constraints`).
///
/// Default settings + no declaration ⇒ false ⇒ zero behavior change
/// (the constraint-free zero-rotation negative face; pinned).
///
/// Separated from [`build_batch_settings`] so the predicate is
/// directly pinnable (the M6-T6 separated-wiring convention) — the
/// board does not exist at the plain-settings assembly site.
pub fn apply_tuning_activation(
    batch: &mut BatchSettings,
    resolved: &ResolvedRouteSettings,
    rules: &epic_board::rules_surf::BoardRules,
) {
    batch.tuning_active = match resolved.tuning {
        Some(false) => false,
        Some(true) => true,
        None => rules.has_length_constraints(),
    };
}

/// The M7-T4 meander sub-flag ACTIVATION (the separated-wiring
/// convention: the predicate directly pinnable, not folded into
/// `build_batch_settings`). `None` rides the resolved tuning
/// activation (input-driven — a declaration activates both); `on`
/// forces the stage (a natural no-op on a constraint-free board — no
/// declaration, no deficit rows); `off` kills the meander stage ALONE
/// (the T3 honoring gate stays armed off the resolved
/// `tuning_active`, which this function does NOT touch).
pub fn apply_meander_activation(batch: &mut BatchSettings, resolved: &ResolvedRouteSettings) {
    batch.meander_active = match resolved.tuning_meander {
        Some(false) => false,
        Some(true) => true,
        None => batch.tuning_active,
    };
}

/// The M7-T6 pair ACTIVATION (the separated-wiring convention — the
/// predicate directly pinnable, next to the tuning/meander faces): the
/// declaration list resolves at the board seam (names → numbers; the
/// conservative resolver: a name resolving to several subnet nets or
/// to none leaves the pair UNRESOLVED — recorded in the advisory
/// rows, never guessed). The resolved specs ride
/// [`BatchSettings::pairs`] (the pass leader-first order, the maze
/// coupling preference, and the post-routing match stage read it);
/// Default empty = zero activation, byte-identical.
///
/// The unresolved rows return as RAW `(name_a, name_b)` tuples — the
/// host crate maps them onto its manifest's advisory row type (the
/// M9-T1 move: the manifest types stay in epic-cli, this module's
/// engine home cannot depend back on the host).
pub fn apply_pairs_activation(
    batch: &mut BatchSettings,
    resolved: &ResolvedRouteSettings,
    board: &epic_board::board::Board,
) -> (
    Vec<epic_router::pipeline::pairs::PairSpec>,
    Vec<(String, String)>,
) {
    let Some(declarations) = resolved.tuning_pairs.clone() else {
        return (Vec::new(), Vec::new());
    };
    let (specs, unresolved) = epic_router::pipeline::pairs::resolve_pairs(board, &declarations);
    batch.pairs = specs.clone();
    (specs, unresolved)
}

// ---------------------------------------------------------------------------
// the session layer (the GUI tri-state slot — M9-T1; the CLI never
// constructs it: zero call sites, the layer is T2's session face)
// ---------------------------------------------------------------------------

/// The GUI/session settings layer (design §4.1's "defaults -> JSON ->
/// DSN -> env -> CLI -> GUI" ladder; the M9 `Session` carries one).
/// Every field is a TRI-STATE `Option` mirroring its [`CliLayer`] CLI
/// twin — `None` = no opinion (the merger convention: a field copies
/// only when non-null/non-default) — and [`merge_session`] applies it
/// ABOVE the CLI layer, so a `Some` session field beats a `Some` CLI
/// field on the same slot.
///
/// The field set is the ROUTING-relevant subset of [`CliLayer`]: the
/// router.* tri-state family (tuning, meander, pairs, the gloss
/// family, plane_island_clamp, congestion_global + its sub-flags,
/// push_shove), the pass/item/thread bounds, and the two stage gates.
/// Parse-only faces (the `-oit` deprecation warnings, the argument
/// surface) have NO session twin — the layer is constructed
/// programmatically, never parsed.
///
/// **The M9-T2 field-set re-audit (T1 spec-review F6, re-decided
/// consciously).** ADDED this task — the route-time faces a
/// GUI-style session host can meaningfully override, mirroring the
/// CLI merge arms one-for-one: `vias_allowed`, `via_costs`,
/// `plane_via_costs`, `start_ripup_costs` (the scoring-box trio +
/// the via gate — the same three slots the DSN layer speaks), plus
/// `automatic_neckdown`, `trace_pull_tight_accuracy`, `strict_drc`,
/// `improvement_threshold`, `fanout_enabled`,
/// `optimizer_max_passes`, `optimizer_max_items`. STILL EXCLUDED,
/// each with its reason:
///
/// * `copper_to_edge_clearance_um` — LOAD-time only: the session
///   override cannot reach the parse-time outline promotion (no
///   post-load re-derivation exists; `DefaultSettings` seeds the
///   only value the frozen tree observes).
/// * the scoring-VERSION fields (`router_scoring_version`,
///   `optimizer_scoring_version`) — formula identity, pinned by the
///   committed score goldens; not a route-time knob.
/// * the optimizer fine-tuning family (`optimizer_algorithm`,
///   `optimizer_enable_preflight_guards`,
///   `optimizer_max_consecutive_failures{,_pass1}`,
///   `optimizer_additional_ripup_cost_factor_at_start`,
///   `optimizer_trace_ripup_cost_factor`,
///   `optimizer_max_autoroute_passes`, `optimizer_timeout_string`) —
///   engine-internal parity-constant knobs; no Java-dialog
///   counterpart in the ported surface.
/// * `optimizer_max_threads` — parsed-but-unused surface parity.
/// * the fanout FINE knobs (`fanout_max_passes`,
///   `fanout_max_items`, `fanout_max_milliseconds_per_pin`,
///   `fanout_ripup_allowed`, the escape/via-diameter/pin-sorting/
///   fallback/timeout row) — stage-internal tuning; only the stage
///   GATE (`fanout_enabled`, added above) is a route-time face.
///
/// The CLI does NOT use this layer anywhere (zero call sites; the
/// byte-invariance proof lap pins the CLI path unchanged).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SessionLayer {
    /// CLI twin: [`CliLayer::autorouter_enabled`] —
    /// `router.autorouter.enabled`, the routing-stage gate.
    pub autorouter_enabled: Option<bool>,
    /// CLI twin: [`CliLayer::optimizer_enabled`] — `optimizer.enabled`,
    /// the optimizer-stage gate.
    pub optimizer_enabled: Option<bool>,
    /// CLI twin: [`CliLayer::max_passes`] —
    /// `router.autorouter.max_passes` (0 = unlimited).
    pub max_passes: Option<i32>,
    /// CLI twin: [`CliLayer::max_items`] — `router.autorouter.max_items`.
    pub max_items: Option<i32>,
    /// CLI twin: [`CliLayer::max_threads`] — `router.max_threads` (the
    /// deterministic partitioned executor's explicit face).
    pub max_threads: Option<i32>,
    /// CLI twin: [`CliLayer::optimizer_threads`] — `optimizer.threads`
    /// (the M8-T7 partitioned candidate executor).
    pub optimizer_threads: Option<i32>,
    /// CLI twin: [`CliLayer::plane_island_clamp`] — the M6-T6 Rust-only
    /// connectivity clamp.
    pub plane_island_clamp: Option<bool>,
    /// CLI twin: [`CliLayer::congestion_global`] — the M6-T7
    /// global-planning master switch.
    pub congestion_global: Option<bool>,
    /// CLI twin: [`CliLayer::congestion_global_pattern`] — the L/Z
    /// pattern-routing sub-flag (requires the master).
    pub congestion_global_pattern: Option<bool>,
    /// CLI twin: [`CliLayer::congestion_global_pathfinder`] — the M6-T8
    /// PathFinder scheduler sub-flag (requires the master).
    pub congestion_global_pathfinder: Option<bool>,
    /// CLI twin: [`CliLayer::push_shove`] — the M6-T9 push-and-shove
    /// insertion flag.
    pub push_shove: Option<bool>,
    /// CLI twin: [`CliLayer::tuning`] — the M7-T2 tuning tri-state
    /// (None = input-driven, on = force, off = kill).
    pub tuning: Option<bool>,
    /// CLI twin: [`CliLayer::tuning_meander`] — the M7-T4 meander
    /// sub-flag.
    pub tuning_meander: Option<bool>,
    /// CLI twin: [`CliLayer::tuning_pairs`] — the M7-T6 pair
    /// declaration list (`NET_A:NET_B, ...`).
    pub tuning_pairs: Option<Vec<(String, String)>>,
    /// CLI twin: [`CliLayer::gloss_bus`] — the M8-T3 gloss-bus flag.
    pub gloss_bus: Option<bool>,
    /// CLI twin: [`CliLayer::gloss_flow`] — the M8-T4 gloss-flow flag.
    pub gloss_flow: Option<bool>,
    /// CLI twin: [`CliLayer::gloss_via_place`] — the M8-T5 gloss
    /// via-place flag.
    pub gloss_via_place: Option<bool>,
    /// CLI twin: [`CliLayer::gloss_teardrops`] — the M8-T6 gloss
    /// teardrops flag.
    pub gloss_teardrops: Option<bool>,
    /// CLI twin: [`CliLayer::vias_allowed`] — `router.vias_allowed`
    /// (the M9-T2 re-audit additions start here: the route-time
    /// faces added above the CLI layer).
    pub vias_allowed: Option<bool>,
    /// CLI twin: [`CliLayer::via_costs`] — `router.via_costs` (the
    /// merged scoring box's slot, like the CLI merge arm).
    pub via_costs: Option<i32>,
    /// CLI twin: [`CliLayer::plane_via_costs`] —
    /// `router.plane_via_costs` (the scoring box).
    pub plane_via_costs: Option<i32>,
    /// CLI twin: [`CliLayer::start_ripup_costs`] —
    /// `router.start_ripup_costs` (the scoring box).
    pub start_ripup_costs: Option<i32>,
    /// CLI twin: [`CliLayer::automatic_neckdown`].
    pub automatic_neckdown: Option<bool>,
    /// CLI twin: [`CliLayer::trace_pull_tight_accuracy`].
    pub trace_pull_tight_accuracy: Option<i32>,
    /// CLI twin: [`CliLayer::strict_drc`].
    pub strict_drc: Option<bool>,
    /// CLI twin: [`CliLayer::improvement_threshold`] — the optimizer
    /// pass-stop threshold (NO session twin for the `-oit`
    /// deprecation warning: the layer is never parsed).
    pub improvement_threshold: Option<f32>,
    /// CLI twin: [`CliLayer::fanout_enabled`] — the fanout stage gate.
    pub fanout_enabled: Option<bool>,
    /// CLI twin: [`CliLayer::optimizer_max_passes`].
    pub optimizer_max_passes: Option<i32>,
    /// CLI twin: [`CliLayer::optimizer_max_items`].
    pub optimizer_max_items: Option<i32>,
}

/// The session-layer merge — the GUI slot applied ABOVE the CLI layer
/// (priority > 60; the design's ladder's last settings source before
/// the run). Field-copies ONLY when the session field is `Some`, the
/// exact [`merge`] convention mirrored arm-for-arm off the CLI block:
/// `None` never overwrites (not even with a default). Called AFTER
/// [`merge`] in the host sequence (`merge(dsn, cli)` then
/// `merge_session`), mutating the merged model in place.
pub fn merge_session(merged: &mut MergedSettings, session: &SessionLayer) {
    if let Some(v) = session.autorouter_enabled {
        merged.autorouter_enabled = Some(v);
    }
    if let Some(v) = session.optimizer_enabled {
        merged.optimizer_enabled = Some(v);
    }
    if let Some(v) = session.max_passes {
        merged.max_passes = Some(v);
    }
    if let Some(v) = session.max_items {
        merged.max_items = Some(v);
    }
    if let Some(v) = session.max_threads {
        merged.max_threads = Some(v);
    }
    if let Some(v) = session.optimizer_threads {
        merged.optimizer_threads = Some(v);
    }
    if let Some(v) = session.plane_island_clamp {
        merged.plane_island_clamp = Some(v);
    }
    if let Some(v) = session.congestion_global {
        merged.congestion_global = Some(v);
    }
    if let Some(v) = session.congestion_global_pattern {
        merged.congestion_global_pattern = Some(v);
    }
    if let Some(v) = session.congestion_global_pathfinder {
        merged.congestion_global_pathfinder = Some(v);
    }
    if let Some(v) = session.push_shove {
        merged.push_shove = Some(v);
    }
    if let Some(v) = session.tuning {
        merged.tuning = Some(v);
    }
    if let Some(v) = session.tuning_meander {
        merged.tuning_meander = Some(v);
    }
    if let Some(v) = session.tuning_pairs.clone() {
        merged.tuning_pairs = Some(v);
    }
    if let Some(v) = session.gloss_bus {
        merged.gloss_bus = Some(v);
    }
    if let Some(v) = session.gloss_flow {
        merged.gloss_flow = Some(v);
    }
    if let Some(v) = session.gloss_via_place {
        merged.gloss_via_place = Some(v);
    }
    if let Some(v) = session.gloss_teardrops {
        merged.gloss_teardrops = Some(v);
    }
    // --- the M9-T2 re-audit additions (route-time faces) ---
    if let Some(v) = session.vias_allowed {
        merged.vias_allowed = Some(v);
    }
    if let Some(v) = session.via_costs {
        merged.scoring.via_costs = Some(v);
    }
    if let Some(v) = session.plane_via_costs {
        merged.scoring.plane_via_costs = Some(v);
    }
    if let Some(v) = session.start_ripup_costs {
        merged.scoring.start_ripup_costs = Some(v);
    }
    if let Some(v) = session.automatic_neckdown {
        merged.automatic_neckdown = Some(v);
    }
    if let Some(v) = session.trace_pull_tight_accuracy {
        merged.trace_pull_tight_accuracy = Some(v);
    }
    if let Some(v) = session.strict_drc {
        merged.strict_drc = Some(v);
    }
    if let Some(v) = session.improvement_threshold {
        merged.improvement_threshold = Some(v);
    }
    if let Some(v) = session.fanout_enabled {
        merged.fanout_enabled = Some(v);
    }
    if let Some(v) = session.optimizer_max_passes {
        merged.optimizer_max_passes = Some(v);
    }
    if let Some(v) = session.optimizer_max_items {
        merged.optimizer_max_items = Some(v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use epic_board::board::Board;
    use epic_dsn::coordinate_transform::CoordinateTransform;
    use epic_dsn::layer_structure::{Layer, LayerStructure};
    use epic_dsn::scope::autoroute_settings::{AutorouteSettingsIr, LayerRuleIr};
    use epic_dsn::ses_board::SesBoard;
    use epic_dsn::sink::{BoardRulesIr, BoardSink, CreateBoardIr};
    use epic_geometry::int_box::IntBox;
    use epic_geometry::int_point::IntPoint;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    /// A LayerRuleIr with the trace-cost fields at their defaults (the
    /// tests only drive the active/preferred-direction faces).
    #[allow(clippy::too_many_arguments)]
    fn layer_rule(active: bool, active_set: bool, horiz: bool, horiz_set: bool) -> LayerRuleIr {
        LayerRuleIr {
            active,
            active_set,
            preferred_direction_is_horizontal: horiz,
            preferred_direction_is_horizontal_set: horiz_set,
            preferred_direction_trace_costs: 1.0,
            preferred_direction_trace_costs_set: false,
            against_preferred_direction_trace_costs: 1.0,
            against_preferred_direction_trace_costs_set: false,
        }
    }

    /// A 4-signal-layer board, 100000 x 50000 (aspect 2:1) — the
    /// geometry-pass world below. NO routing items needed: the pass
    /// reads only the layer structure and the bounding box.
    fn four_layer_board() -> Board {
        let mut ses = SesBoard::new();
        ses.create_board(CreateBoardIr {
            bounding_box: IntBox::new(IntPoint::new(0, 0), IntPoint::new(100_000, 50_000)),
            layer_structure: LayerStructure::new(vec![
                Layer::new("S1", 0, true),
                Layer::new("S2", 1, true),
                Layer::new("S3", 2, true),
                Layer::new("S4", 3, true),
            ]),
            outline_shapes: Vec::new(),
            outline_clearance_class: Some("default".to_string()),
            rules: BoardRulesIr::new(4),
            transform: CoordinateTransform::new(10.0, 0.0, 0.0),
        });
        Board::from_ses_board(&ses)
    }

    // -------------------------------------------------------------------
    // the CLI parse face
    // -------------------------------------------------------------------

    /// Precedence worlds (the merge is copy-if-set, Cli(60) > Dsn(20) >
    /// Default(0)): a CLI set field beats the DSN; a CLI-unset field
    /// lets the DSN survive; where neither speaks, the default rides.
    #[test]
    fn merge_precedence_cli_over_dsn_over_default() {
        let mut dsn = DsnLayer::from_metadata(None, 0);
        dsn.via_costs = Some(7);
        dsn.run_router = Some(false);
        let cli = CliLayer {
            via_costs: Some(9),
            strict_drc: Some(true),
            ..CliLayer::default()
        };
        let merged = merge(&MergedSettings::default(), &dsn, &cli);

        // CLI over DSN.
        assert_eq!(merged.scoring.via_costs, Some(9));
        // DSN survives the CLI silence.
        assert_eq!(merged.autorouter_enabled, Some(false));
        // Where neither DSN nor CLI speaks, the DefaultSettings seed
        // value rides unchanged (copy-if-set NEVER nulls).
        assert_eq!(merged.scoring.start_ripup_costs, Some(100));
        // Default when nobody speaks; CLI speaks over default.
        assert_eq!(merged.vias_allowed, Some(true));
        assert_eq!(merged.strict_drc, Some(true));
    }

    /// The argument forms and the short-flag map: `--router.X=v`,
    /// deprecated flat paths (warned), `-mp`/`-mt` shorts, unknown
    /// short flags silently skipped, unknown router paths
    /// WARN-and-continue.
    #[test]
    fn cli_argument_forms_short_flags_and_warns() {
        let parsed = parse_route_args(&args(&[
            "-de",
            "a.dsn",
            "-do",
            "b.ses",
            "--router.autorouter.max_passes=17",
            "-mp",
            "3",
            "--router.max_items=42",
            "-mt",
            "4",
            "--router.strict_drc=on",
            "--router.fanout.enabled=off",
            "--bogus-short",
            "--router.no.such.path=1",
        ]))
        .expect("parses");
        // Later values overwrite earlier ones (Java: every apply is a
        // field write).
        assert_eq!(parsed.layer.max_passes, Some(3));
        assert_eq!(parsed.layer.max_items, Some(42));
        assert_eq!(parsed.layer.max_threads, Some(4));
        assert_eq!(parsed.layer.strict_drc, Some(true));
        assert_eq!(parsed.layer.fanout_enabled, Some(false));
        // The implicit router-enable fired (batch mode, no explicit).
        assert_eq!(parsed.layer.autorouter_enabled, Some(true));
        // WARN-and-continue for the unknown path; deprecation warn for
        // the flat max_items path; no warning for the unknown short flag.
        let warned = parsed.layer.warnings.join(" | ");
        assert!(
            warned.contains("router.no.such.path"),
            "unknown path must warn: {warned}"
        );
        assert!(
            warned.contains(
                "Deprecated settings path 'router.max_items'; use \
                             'router.autorouter.max_items' instead."
            ) && warned.contains("will be removed in a future release"),
            "flat path deprecation must warn: {warned}"
        );
        assert!(!warned.contains("bogus"), "unknown short flag is silent");
    }

    /// The scoring-version normalization table with THE ASYMMETRY:
    /// `-scoring-version v2` resolves V2_CONTINUOUS on the router box
    /// but V2_LOWER_BOUND on the optimizer box (which has no
    /// V2_CONTINUOUS constant at all); `v1`/`legacy` land V1_LEGACY on
    /// both; `lower_bound` is valid ONLY on the optimizer box; an
    /// invalid constant for the addressed box warns and leaves the
    /// field unset.
    #[test]
    fn scoring_version_dual_apply_asymmetry() {
        let parsed = parse_route_args(&args(&[
            "-de",
            "a.dsn",
            "-do",
            "b.ses",
            "-scoring-version",
            "v2",
        ]))
        .expect("parses");
        assert_eq!(
            parsed.layer.router_scoring_version,
            Some(RouterScoringVersion::V2Continuous)
        );
        assert_eq!(
            parsed.layer.optimizer_scoring_version,
            Some(OptimScoringVersion::V2LowerBound)
        );
        assert!(parsed.layer.warnings.is_empty());

        // legacy alias + raw constant names, side-specific flags.
        let parsed = parse_route_args(&args(&[
            "-de",
            "a.dsn",
            "-do",
            "b.ses",
            "-router-scoring-version",
            "legacy",
            "-optimizer-scoring-version",
            "V1_LEGACY",
        ]))
        .expect("parses");
        assert_eq!(
            parsed.layer.router_scoring_version,
            Some(RouterScoringVersion::V1Legacy)
        );
        assert_eq!(
            parsed.layer.optimizer_scoring_version,
            Some(OptimScoringVersion::V1Legacy)
        );

        // `lower_bound` on the ROUTER box: no such constant -> warn,
        // router field stays unset (the optimizer field would take it).
        let parsed = parse_route_args(&args(&[
            "-de",
            "a.dsn",
            "-do",
            "b.ses",
            "-router-scoring-version",
            "lower_bound",
        ]))
        .expect("parses");
        assert_eq!(parsed.layer.router_scoring_version, None);
        assert!(
            parsed
                .layer
                .warnings
                .iter()
                .any(|w| w.contains("V2_LOWER_BOUND")),
            "invalid-for-box constant warns: {:?}",
            parsed.layer.warnings
        );

        // `--optimizer.scoring.version=v2` long form: V2_LOWER_BOUND.
        let parsed = parse_route_args(&args(&[
            "-de",
            "a.dsn",
            "-do",
            "b.ses",
            "--optimizer.scoring.version=v2",
        ]))
        .expect("parses");
        assert_eq!(
            parsed.layer.optimizer_scoring_version,
            Some(OptimScoringVersion::V2LowerBound)
        );
    }

    /// The improvement_threshold x100 quirk: a value in the EXCLUSIVE
    /// (0, 1) interval is a fraction and x100s (0.25 -> 25.0); the
    /// endpoints 0.0 and 1.0 pass through RAW; values > 1 pass raw; a
    /// parse failure warns and leaves the field unset. `-oit` fires the
    /// deprecation warn EVEN when it applies.
    #[test]
    fn improvement_threshold_x100_and_oit_deprecation() {
        let parsed = parse_route_args(&args(&[
            "-de",
            "a.dsn",
            "-do",
            "b.ses",
            "--router.optimizer.improvement_threshold=0.25",
            "-oit",
            "0.5",
            "--router.optimizer.improvement_threshold=0.0",
            "--router.optimizer.improvement_threshold=1.0",
            "--router.optimizer.improvement_threshold=2.5",
        ]))
        .expect("parses");
        // The LAST successful apply wins: 2.5 (raw — outside (0,1)).
        assert_eq!(parsed.layer.improvement_threshold, Some(2.5));
        // 0.0 and 1.0 raw; 0.25 -> 25.0 was observed on the way.
        let world = parse_route_args(&args(&[
            "-de",
            "a.dsn",
            "-do",
            "b.ses",
            "--router.optimizer.improvement_threshold=0.25",
        ]))
        .expect("parses");
        assert_eq!(world.layer.improvement_threshold, Some(25.0));
        // The ENDPOINTS are NOT fractions: 0.0 and 1.0 pass raw
        // (the interval is exclusive on both ends).
        let world = parse_route_args(&args(&[
            "-de",
            "a.dsn",
            "-do",
            "b.ses",
            "--router.optimizer.improvement_threshold=0.0",
        ]))
        .expect("parses");
        assert_eq!(world.layer.improvement_threshold, Some(0.0));
        let world = parse_route_args(&args(&[
            "-de",
            "a.dsn",
            "-do",
            "b.ses",
            "--router.optimizer.improvement_threshold=1.0",
        ]))
        .expect("parses");
        assert_eq!(world.layer.improvement_threshold, Some(1.0));
        let world = parse_route_args(&args(&["-de", "a.dsn", "-do", "b.ses", "-oit", "0.5"]))
            .expect("parses");
        assert_eq!(world.layer.improvement_threshold, Some(50.0));
        assert_eq!(
            world.layer.warnings.first().map(String::as_str),
            Some(
                "The '-oit' command-line flag is deprecated; use \
                 '--router.optimizer.improvement_threshold' instead."
            ),
            "the deprecation warn fires even when the flag applies"
        );
        // Garbage: warn + unset.
        let world = parse_route_args(&args(&[
            "-de",
            "a.dsn",
            "-do",
            "b.ses",
            "--router.optimizer.improvement_threshold=abc",
        ]))
        .expect("parses");
        assert_eq!(world.layer.improvement_threshold, None);
        assert!(
            world
                .layer
                .warnings
                .iter()
                .any(|w| w.contains("For input string: \"abc\""))
        );
    }

    /// The implicit router-enable (`CliSettings.java:100-107`):
    /// `-de`+`-do` with no explicit enable -> Some(true); an explicit
    /// `--router.autorouter.enabled=off` wins over the implicit enable;
    /// without both files there is no implicit enable.
    #[test]
    fn implicit_router_enable_faces() {
        let parsed = parse_route_args(&args(&["-de", "a.dsn", "-do", "b.ses"])).expect("parses");
        assert_eq!(parsed.layer.autorouter_enabled, Some(true));
        assert!(parsed.batch_mode());

        let parsed = parse_route_args(&args(&[
            "-de",
            "a.dsn",
            "-do",
            "b.ses",
            "--router.enabled=off",
        ]))
        .expect("parses");
        assert_eq!(
            parsed.layer.autorouter_enabled,
            Some(false),
            "the explicit enable beats the implicit one"
        );
        // The canonical long form counts as explicit too.
        let parsed = parse_route_args(&args(&[
            "-de",
            "a.dsn",
            "-do",
            "b.ses",
            "--router.autorouter.enabled=false",
        ]))
        .expect("parses");
        assert_eq!(parsed.layer.autorouter_enabled, Some(false));

        // One file short: no implicit enable, no explicit.
        let parsed = parse_route_args(&args(&["-de", "a.dsn"])).expect("parses");
        assert_eq!(parsed.layer.autorouter_enabled, None);
    }

    /// The narrowed `-de`/`-do`: a second file is a hard error, a `+`
    /// concatenation is a hard error, a missing value is a hard error.
    #[test]
    fn narrowed_de_do_rejections() {
        assert!(
            parse_route_args(&args(&["-de", "a.dsn", "-de", "b.dsn", "-do", "b.ses",])).is_err(),
            "second -de rejected"
        );
        assert!(
            parse_route_args(&args(&["-de", "a.dsn+b.dsn", "-do", "b.ses"])).is_err(),
            "'+' concatenation rejected"
        );
        assert!(
            parse_route_args(&args(&["-de", "-do", "b.ses"])).is_err(),
            "-de followed by a flag consumes no value"
        );
        assert!(
            parse_route_args(&args(&["-de", "a.dsn", "-do", "x.ses", "-do", "y.ses"])).is_err(),
            "second -do rejected"
        );
        assert!(parse_route_args(&args(&["-de", "a.dsn", "-do", ""])).is_err());
    }

    /// `canonical_cli_path` (`LegacyRouterSettingsBridge:24-39`): the
    /// six deprecated flat autorouter keys gain the prefix, everything
    /// else lowercases through.
    #[test]
    fn canonical_cli_path_table() {
        assert_eq!(canonical_cli_path("MAX_PASSES"), "autorouter.max_passes");
        assert_eq!(canonical_cli_path("Enabled"), "autorouter.enabled");
        assert_eq!(canonical_cli_path("algorithm"), "autorouter.algorithm");
        assert_eq!(canonical_cli_path("max_items"), "autorouter.max_items");
        assert_eq!(
            canonical_cli_path("save_intermediate_stages"),
            "autorouter.save_intermediate_stages"
        );
        assert_eq!(
            canonical_cli_path("ignore_net_classes"),
            "autorouter.ignore_net_classes"
        );
        assert_eq!(canonical_cli_path("max_threads"), "max_threads");
        assert_eq!(canonical_cli_path("Via_Costs"), "via_costs");
        assert!(is_deprecated_flat_autorouter_path("MAX_PASSES"));
        assert!(!is_deprecated_flat_autorouter_path("max_threads"));
    }

    // -------------------------------------------------------------------
    // the DSN layer + merge
    // -------------------------------------------------------------------

    /// `DsnLayer::from_metadata` reads ONLY the explicit-set flags: a
    /// `(via_costs ...)` that never appeared leaves the field unset
    /// (copyFields' non-null test), run_router/run_optimizer are always
    /// set when the scope exists, and the parsed per-layer trace costs
    /// are DROPPED (bug-compat: the headless geometry pass always
    /// re-derives them).
    #[test]
    fn dsn_layer_reads_only_set_flags_and_drops_trace_costs() {
        let mut ir = AutorouteSettingsIr::new(1);
        ir.run_router = false;
        ir.run_optimizer = true;
        ir.vias_allowed = true;
        ir.vias_allowed_set = true;
        ir.via_costs = 3;
        ir.via_costs_set = false; // parsed value present, scope absent
        ir.layer_rules = vec![layer_rule(false, true, true, false)];

        let dsn = DsnLayer::from_metadata(Some(&ir), 4);
        assert_eq!(dsn.run_router, Some(false), "always set by the post-loop");
        assert_eq!(dsn.run_optimizer, Some(true));
        assert_eq!(dsn.vias_allowed, Some(true), "set flag present");
        assert_eq!(dsn.via_costs, None, "unset scope must NOT copy");
        assert_eq!(dsn.plane_via_costs, None);
        assert_eq!(dsn.start_ripup_costs, None);
        assert_eq!(dsn.layer_count, 4);
        assert_eq!(
            dsn.layers.len(),
            1,
            "per-layer slots mirror the parsed rules"
        );
        assert_eq!(dsn.layers[0].routable, Some(false), "active_set true");
        assert_eq!(
            dsn.layers[0].preferred_direction_is_horizontal, None,
            "preferred_direction scope absent"
        );
        assert_eq!(dsn.layers[0].bend_cost, None, "no DSN scope sets bend");

        // No scope at all: only the layer-count seed survives.
        let dsn = DsnLayer::from_metadata(None, 2);
        assert_eq!(dsn.run_router, None);
        assert_eq!(dsn.layers.len(), 0);
        assert_eq!(dsn.layer_count, 2);
    }

    /// The layer-count seed sizing (`DsnFileSettings.java:47-52`): with
    /// no per-layer rules the merge still sizes the slots so the
    /// geometry pass fills them in place.
    #[test]
    fn merge_sizes_layers_from_seed() {
        let dsn = DsnLayer::from_metadata(None, 3);
        let merged = merge(&MergedSettings::default(), &dsn, &CliLayer::default());
        assert_eq!(merged.layers.len(), 3);
        assert!(
            merged.layers.iter().all(|slot| slot.routable.is_none()),
            "seed slots are empty opinions"
        );
    }

    // -------------------------------------------------------------------
    // the geometry pass (applyBoardSpecificOptimizations)
    // -------------------------------------------------------------------

    /// The aspect-ratio math on the 2:1 four-signal-layer board:
    /// hAdd = 0.1*round(10*100000/50000) = 2.0, vAdd = 0.5; the seed
    /// direction is `hw < vw` = false and each signal layer TOGGLES
    /// FIRST (L0 true, L1 false, L2 true, L3 false); undesired = 1.0 +
    /// (horiz ? 2.0 : 0.5); the 4-signal outer surcharge 0.2*4 = 0.8
    /// rides on slots [0] and [3] of BOTH arrays; then `resolve`
    /// assembles getTraceCosts (horizontal = preferred when the slot is
    /// horizontal). Bend costs fill from the (unclamped-writing)
    /// default 0.0.
    #[test]
    fn geometry_pass_alternation_and_costs_on_four_layer_board() {
        let board = four_layer_board();
        let mut merged = MergedSettings::default();
        merged.scoring.default_bend_cost = None; // DefaultSettings' 0.0 face
        apply_board_specific_optimizations(&mut merged, &board);

        let directions: Vec<bool> = merged
            .layers
            .iter()
            .map(|slot| slot.preferred_direction_is_horizontal.expect("filled"))
            .collect();
        assert_eq!(directions, vec![true, false, true, false]);
        assert!(
            merged.layers.iter().all(|slot| slot.routable == Some(true)),
            "all-signal board: every slot becomes routable"
        );

        // resolved per-layer table (getTraceCosts).
        let resolved = ResolvedRouteSettings::resolve(&merged, None);
        let trace_costs: Vec<(f64, f64)> = resolved
            .router_settings
            .trace_costs
            .iter()
            .map(|factor| (factor.horizontal, factor.vertical))
            .collect();
        //        pref undesired   L0: (h,v)=(1.0, 1.0+2.0)= (1.0, 3.0)
        //   + outer 0.8 on L0/L3 BOTH arrays:
        assert_eq!(trace_costs[0], (1.8, 3.8), "L0 horiz + outer surcharge");
        assert_eq!(trace_costs[1], (1.5, 1.0), "L1 vertical-pref, no surcharge");
        assert_eq!(trace_costs[2], (1.0, 3.0), "L2 horiz, no surcharge");
        assert_eq!(trace_costs[3], (2.3, 1.8), "L3 vertical-pref + surcharge");
        assert_eq!(
            resolved.router_settings.bend_costs,
            vec![0.0; 4],
            "null default falls to 0.0 per slot"
        );
        assert_eq!(
            resolved.router_settings.layer_active,
            vec![true, true, true, true]
        );
    }

    /// The DSN-set per-layer opinions SURVIVE the geometry pass (it
    /// fills only null slots): an `(active off)` slot stays inactive
    /// even on a signal layer, and a set preferred direction is kept
    /// (breaking the alternation for that layer only).
    #[test]
    fn dsn_layer_opinions_survive_geometry_pass() {
        let mut ir = AutorouteSettingsIr::new(4);
        ir.layer_rules = vec![
            layer_rule(true, true, false, true),  // L0 forced vertical
            layer_rule(false, true, true, false), // L1 disabled by the DSN
            layer_rule(true, false, true, false),
            layer_rule(true, false, true, false),
        ];
        let dsn = DsnLayer::from_metadata(Some(&ir), 4);
        let mut merged = merge(&MergedSettings::default(), &dsn, &CliLayer::default());
        apply_board_specific_optimizations(&mut merged, &four_layer_board());

        assert_eq!(
            merged.layers[0].preferred_direction_is_horizontal,
            Some(false),
            "the DSN-set direction survives"
        );
        assert_eq!(merged.layers[1].routable, Some(false), "active off sticks");
        assert_eq!(merged.layers[2].routable, Some(true), "null routable fills");
        // The alternation ignores the DSN-forced slot (Java toggles the
        // seed REGARDLESS of whether the slot was preset — :163/:167).
        assert_eq!(
            merged.layers[1].preferred_direction_is_horizontal,
            Some(false),
            "toggle-first continues under preset slots"
        );
        assert_eq!(
            merged.layers[2].preferred_direction_is_horizontal,
            Some(true)
        );
    }

    // -------------------------------------------------------------------
    // the resolution product (getter faces)
    // -------------------------------------------------------------------

    /// The getter null-faces and THE BEND CLAMP ASYMMETRY
    /// (`RouterSettings.java:696-707`): a SET slot value returns RAW
    /// (12.0 stays 12.0 even beyond MAX 9.9); only the DEFAULT fallback
    /// clamps into [0.0, 9.9]; via_costs null -> 1 (getViaCosts), not
    /// the DefaultSettings 50; layer_active null -> true;
    /// automatic_neckdown null -> false; start_ripup_costs null -> 1.
    #[test]
    fn resolve_getter_faces_and_bend_clamp_asymmetry() {
        let mut merged = MergedSettings::default();
        merged.scoring.via_costs = None;
        merged.scoring.start_ripup_costs = None;
        merged.scoring.default_bend_cost = Some(12.0); // beyond MAX 9.9
        merged.layers = vec![
            MergedLayerSlot {
                routable: None,
                preferred_direction_is_horizontal: Some(true),
                bend_cost: Some(12.0), // SET slot: RAW
            },
            MergedLayerSlot {
                routable: Some(false),
                preferred_direction_is_horizontal: Some(false),
                bend_cost: None, // default fallback: CLAMPED
            },
        ];
        merged.preferred_trace_costs = vec![1.0, 1.0];
        merged.undesired_trace_costs = vec![1.0, 1.0];

        let resolved = ResolvedRouteSettings::resolve(&merged, None);
        assert_eq!(resolved.router_settings.via_costs, 1, "null -> 1");
        assert_eq!(resolved.router_settings.start_ripup_costs, 1);
        assert_eq!(resolved.router_settings.bend_costs, vec![12.0, 9.9]);
        assert_eq!(
            resolved.router_settings.layer_active,
            vec![true, false],
            "null routable -> true, DSN-off -> false"
        );
        assert_eq!(resolved.max_passes, Some(0), "default unlimited");
        assert_eq!(resolved.max_items, Some(i32::MAX));
        assert!(resolved.fanout_enabled);
        assert!(resolved.run_router);
        assert!(!resolved.strict_drc);
        assert_eq!(resolved.pull_tight_accuracy, 500);
        assert!(resolved.deterministic_budgets, "default ON");
        // --deterministic-budgets=off flows through.
        let resolved = ResolvedRouteSettings::resolve(&merged, Some(false));
        assert!(!resolved.deterministic_budgets);
    }

    /// The DefaultSettings seed (the priority-0 base) carries the
    /// documented scalars — a wrong seed silently rewrites every
    /// unset world.
    #[test]
    fn default_settings_seed_values() {
        let merged = MergedSettings::default();
        assert_eq!(merged.max_passes, Some(0));
        assert_eq!(merged.max_items, Some(i32::MAX));
        assert_eq!(merged.vias_allowed, Some(true));
        assert_eq!(merged.automatic_neckdown, Some(true));
        assert_eq!(merged.trace_pull_tight_accuracy, Some(500));
        assert_eq!(merged.strict_drc, Some(false));
        assert_eq!(merged.fanout_enabled, Some(true));
        assert_eq!(merged.optimizer_enabled, Some(true));
        assert_eq!(merged.improvement_threshold, Some(2.5));
        let scoring = default_routing_cost_settings();
        assert_eq!(scoring.via_costs, Some(50));
        assert_eq!(scoring.plane_via_costs, Some(5));
        assert_eq!(scoring.start_ripup_costs, Some(100));
        assert_eq!(scoring.default_bend_cost, Some(0.0));
        let router_scoring = default_router_score_settings();
        assert_eq!(router_scoring.version, RouterScoringVersion::V2Continuous);
        assert_eq!(router_scoring.unrouted_free_fraction, Some(0.5));
    }

    // -------------------------------------------------------------------
    // the fix-round pins (spec review F1/F2, PG1/PG2)
    // -------------------------------------------------------------------

    /// F1: the merger's trailing validate() (`RouterSettings.java:
    /// 938-977`). A negative maxPasses (or one over 9999 that is not
    /// the MAX sentinel) warns and resets to 0 = UNLIMITED — the
    /// driver's router-enabled gate then routes, which the flow-level
    /// witness in route.rs covers end-to-end. maxThreads normalizes
    /// (default / negative / over-cap; 0 survives); accuracy < 1
    /// resets to 500.
    #[test]
    fn validate_resets_max_passes_threads_and_accuracy() {
        // Negative maxPasses: warn + reset to 0.
        let mut merged = MergedSettings {
            max_passes: Some(-5),
            ..MergedSettings::default()
        };
        let warnings = validate(&mut merged);
        assert_eq!(merged.max_passes, Some(0), "negative resets to unlimited");
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("Invalid maxPasses value: -5") && w.contains("no limit")),
            "warn verbatim: {warnings:?}"
        );

        // Over 9999 (not the MAX sentinel) resets too.
        let mut merged = MergedSettings {
            max_passes: Some(10_000),
            ..MergedSettings::default()
        };
        let warnings = validate(&mut merged);
        assert_eq!(merged.max_passes, Some(0));
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("maxPasses value: 10000"))
        );

        // 9999 and the i32::MAX sentinel survive untouched, no warn.
        let mut merged = MergedSettings {
            max_passes: Some(9999),
            ..MergedSettings::default()
        };
        assert!(validate(&mut merged).is_empty());
        assert_eq!(merged.max_passes, Some(9999));
        let mut merged = MergedSettings {
            max_passes: Some(i32::MAX),
            ..MergedSettings::default()
        };
        assert!(validate(&mut merged).is_empty());

        // maxThreads: default fill for None; negative -> default
        // max(1, cores - 1); over-cap -> capped at cores; 0 survives.
        let cores = std::thread::available_parallelism().map_or(1usize, |n| n.get());
        let available = i32::try_from(cores).unwrap_or(i32::MAX);
        let default_max_threads = (available - 1).max(1);
        let mut merged = MergedSettings {
            max_threads: Some(-2),
            ..MergedSettings::default()
        };
        let warnings = validate(&mut merged);
        assert_eq!(merged.max_threads, Some(default_max_threads));
        // The FULL Java text carries the numeric default
        // (RouterSettings.java:958-961) — a prefix-only assert accepted
        // a "using default" mutant.
        assert!(
            warnings
                .iter()
                .any(|w| *w == format!("Invalid maxThreads value: -2, using {default_max_threads}")),
            "warn verbatim: {warnings:?}"
        );
        let mut merged = MergedSettings {
            max_threads: Some(available.saturating_add(1)),
            ..MergedSettings::default()
        };
        let warnings = validate(&mut merged);
        assert_eq!(merged.max_threads, Some(available));
        assert!(
            warnings
                .iter()
                .any(|w| w.contains(&format!(", capping at {available}")))
        );
        let mut merged = MergedSettings {
            max_threads: Some(0),
            ..MergedSettings::default()
        };
        assert!(validate(&mut merged).is_empty());
        assert_eq!(merged.max_threads, Some(0));

        // tracePullTightAccuracy < 1: warn + reset 500.
        let mut merged = MergedSettings {
            trace_pull_tight_accuracy: Some(0),
            ..MergedSettings::default()
        };
        let warnings = validate(&mut merged);
        assert_eq!(merged.trace_pull_tight_accuracy, Some(500));
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("Invalid tracePullTightAccuracy value: 0, using default 500"))
        );
    }

    /// F2: the scoring-version DEFAULT arm passes the value RAW to
    /// valueOf (`CliSettings.java:142`) — a mixed-case constant
    /// (`V2_Continuous`) and non-alias lowercase junk warn with the
    /// field UNSET; only the alias table is case-insensitive; an
    /// exactly-typed constant name applies through the raw arm.
    #[test]
    fn scoring_version_default_arm_is_raw() {
        let parsed = parse_route_args(&args(&[
            "-de",
            "a.dsn",
            "-do",
            "b.ses",
            "-scoring-version",
            "V2_Continuous",
        ]))
        .expect("parses");
        assert_eq!(parsed.layer.router_scoring_version, None);
        assert_eq!(parsed.layer.optimizer_scoring_version, None);
        assert!(
            parsed
                .layer
                .warnings
                .iter()
                .any(|w| w.contains("V2_Continuous")),
            "mixed case warns: {:?}",
            parsed.layer.warnings
        );

        // Lowercase non-alias junk: raw to valueOf -> warn + unset
        // (the OLD Rust uppercased it into a silent hit).
        let parsed = parse_route_args(&args(&[
            "-de",
            "a.dsn",
            "-do",
            "b.ses",
            "-scoring-version",
            "v2_continuous",
        ]))
        .expect("parses");
        assert_eq!(parsed.layer.router_scoring_version, None);
        assert!(
            parsed
                .layer
                .warnings
                .iter()
                .any(|w| w.contains("v2_continuous"))
        );

        // An exactly-typed constant name applies through the raw arm.
        let parsed = parse_route_args(&args(&[
            "-de",
            "a.dsn",
            "-do",
            "b.ses",
            "-router-scoring-version",
            "V2_CONTINUOUS",
        ]))
        .expect("parses");
        assert_eq!(
            parsed.layer.router_scoring_version,
            Some(RouterScoringVersion::V2Continuous)
        );
        assert!(parsed.layer.warnings.is_empty());
    }

    /// PG1: non-signal layers are FORCE-DISABLED by the geometry pass
    /// (`RouterSettings.java:341-343`) — a power slot resolves
    /// routable Some(false) although no DSN opinion spoke, while the
    /// signal slots fill Some(true); the direction alternation toggles
    /// per SIGNAL layer only (the power slot inherits the current
    /// direction without toggling), and a 2-signal board takes no
    /// outer surcharge.
    #[test]
    fn geometry_pass_force_disables_non_signal_layers() {
        let mut ses = SesBoard::new();
        ses.create_board(CreateBoardIr {
            bounding_box: IntBox::new(IntPoint::new(0, 0), IntPoint::new(100_000, 50_000)),
            layer_structure: LayerStructure::new(vec![
                Layer::new("S1", 0, true),
                Layer::new("PWR", 1, false),
                Layer::new("S2", 2, true),
            ]),
            outline_shapes: Vec::new(),
            outline_clearance_class: Some("default".to_string()),
            rules: BoardRulesIr::new(3),
            transform: CoordinateTransform::new(10.0, 0.0, 0.0),
        });
        let board = Board::from_ses_board(&ses);
        let mut merged = MergedSettings::default();
        apply_board_specific_optimizations(&mut merged, &board);

        assert_eq!(merged.layers[0].routable, Some(true));
        assert_eq!(
            merged.layers[1].routable,
            Some(false),
            "the power slot is force-disabled"
        );
        assert_eq!(merged.layers[2].routable, Some(true));

        // Direction: seed hw<vw = false; L0 signal toggles -> true;
        // PWR inherits true WITHOUT toggling; L2 signal toggles ->
        // false.
        let directions: Vec<bool> = merged
            .layers
            .iter()
            .map(|slot| slot.preferred_direction_is_horizontal.expect("filled"))
            .collect();
        assert_eq!(directions, vec![true, true, false]);

        // 2 signal layers -> no outer surcharge; undesired costs ride
        // the direction face (L0/L1 horizontal +2.0, L2 vertical
        // +0.5) — L1's cost row is DERIVED like any other row even
        // though the slot is disabled.
        let resolved = ResolvedRouteSettings::resolve(&merged, None);
        let costs: Vec<(f64, f64)> = resolved
            .router_settings
            .trace_costs
            .iter()
            .map(|factor| (factor.horizontal, factor.vertical))
            .collect();
        assert_eq!(costs[0], (1.0, 3.0));
        assert_eq!(costs[1], (1.0, 3.0));
        assert_eq!(costs[2], (1.5, 1.0));
    }

    /// PG2: an unknown SHORT flag is SILENTLY skipped (`CliSettings.
    /// java:90-96` + `mapFlagToProperty`'s null face) — no error, no
    /// warning, and its operand is consumed by the `-flag value` rule
    /// (it never leaks into a later position or a setting).
    #[test]
    fn unknown_short_flag_is_silently_skipped() {
        let parsed = parse_route_args(&args(&["-zz", "3", "-de", "a.dsn", "-do", "b.ses"]))
            .expect("an unknown short flag must not error");
        assert!(
            parsed.layer.warnings.is_empty(),
            "silent skip: {:?}",
            parsed.layer.warnings
        );
        assert!(parsed.batch_mode());
        assert_eq!(
            parsed.layer.max_passes, None,
            "the -zz operand was consumed, not applied"
        );
    }

    // -------------------------------------------------------------------
    // the quality-round pins (review Q5, Q10c)
    // -------------------------------------------------------------------

    /// Q5 (quality review): the merge is EXHAUSTIVE — every CliLayer
    /// and DsnLayer field lands in the merged model. A deleted copy arm
    /// (the `dsn.run_optimizer` mutant passed the whole suite before
    /// this pin) is a loud failure here, which is the exhaustiveness
    /// the hand-rolled merge lacks before M4 grows it.
    #[test]
    fn merge_copies_every_field_from_both_layers() {
        // The DSN face: every DsnLayer field distinct from the default.
        let dsn = DsnLayer {
            run_router: Some(false),
            run_optimizer: Some(false),
            vias_allowed: Some(false),
            via_costs: Some(7),
            plane_via_costs: Some(8),
            start_ripup_costs: Some(9),
            layer_count: 1,
            layers: vec![DsnLayerSlot {
                routable: Some(false),
                preferred_direction_is_horizontal: Some(true),
                bend_cost: None,
            }],
        };
        let merged = merge(&MergedSettings::default(), &dsn, &CliLayer::default());
        assert_eq!(merged.autorouter_enabled, Some(false), "dsn.run_router");
        assert_eq!(merged.optimizer_enabled, Some(false), "dsn.run_optimizer");
        assert_eq!(merged.vias_allowed, Some(false), "dsn.vias_allowed");
        assert_eq!(merged.scoring.via_costs, Some(7), "dsn.via_costs");
        assert_eq!(
            merged.scoring.plane_via_costs,
            Some(8),
            "dsn.plane_via_costs"
        );
        assert_eq!(
            merged.scoring.start_ripup_costs,
            Some(9),
            "dsn.start_ripup_costs"
        );
        assert_eq!(merged.layers.len(), 1, "dsn.layers");
        assert_eq!(merged.layers[0].routable, Some(false));
        assert_eq!(
            merged.layers[0].preferred_direction_is_horizontal,
            Some(true)
        );

        // The CLI face: every CliLayer merge-consumed field distinct.
        let cli = CliLayer {
            autorouter_enabled: Some(false),
            max_passes: Some(3),
            max_items: Some(44),
            max_threads: Some(2),
            plane_island_clamp: Some(true),
            congestion_global: Some(true),
            congestion_global_pattern: Some(true),
            congestion_global_pathfinder: Some(true),
            push_shove: Some(true),
            tuning: Some(true),
            tuning_meander: Some(true),
            tuning_pairs: Some(vec![
                ("PA".to_string(), "PB".to_string()),
                ("PC".to_string(), "PD".to_string()),
            ]),
            gloss_bus: Some(true),
            gloss_flow: Some(true),
            gloss_via_place: Some(true),
            gloss_teardrops: Some(true),
            improvement_threshold: Some(7.5),
            router_scoring_version: Some(RouterScoringVersion::V1Legacy),
            optimizer_scoring_version: Some(OptimScoringVersion::V1Legacy),
            optimizer_enabled: Some(false),
            optimizer_algorithm: Some("t9-algo".to_string()),
            optimizer_max_passes: Some(21),
            optimizer_max_items: Some(22),
            optimizer_max_threads: Some(23),
            optimizer_threads: Some(2),
            optimizer_enable_preflight_guards: Some(false),
            optimizer_max_consecutive_failures: Some(24),
            optimizer_max_consecutive_failures_pass1: Some(25),
            optimizer_additional_ripup_cost_factor_at_start: Some(26),
            optimizer_trace_ripup_cost_factor: Some(0.7),
            optimizer_max_autoroute_passes: Some(27),
            optimizer_timeout_string: Some("3:00".to_string()),
            vias_allowed: Some(true),
            via_costs: Some(11),
            plane_via_costs: Some(12),
            start_ripup_costs: Some(13),
            automatic_neckdown: Some(false),
            trace_pull_tight_accuracy: Some(123),
            strict_drc: Some(true),
            fanout_enabled: Some(false),
            fanout_max_passes: Some(2),
            fanout_max_items: Some(77),
            fanout_max_milliseconds_per_pin: Some(1500),
            fanout_ripup_allowed: Some(false),
            fanout_min_escape_length_mm: Some(1.5),
            fanout_max_escape_length_mm: Some(6.5),
            fanout_start_via_diameter_mm: Some(0.3),
            fanout_end_via_diameter_mm: Some(0.4),
            fanout_pin_sorting_order: Some("inner_first".to_string()),
            fanout_fallback_to_board_vias: Some(false),
            fanout_timeout_string: Some("90s".to_string()),
            warnings: Vec::new(),
        };
        let merged = merge(&MergedSettings::default(), &DsnLayer::default(), &cli);
        assert_eq!(
            merged.autorouter_enabled,
            Some(false),
            "cli.autorouter_enabled"
        );
        assert_eq!(merged.max_passes, Some(3), "cli.max_passes");
        assert_eq!(merged.max_items, Some(44), "cli.max_items");
        assert_eq!(merged.max_threads, Some(2), "cli.max_threads");
        assert_eq!(
            merged.optimizer_threads,
            Some(2),
            "cli.optimizer_threads (M8-T7)"
        );
        assert_eq!(
            merged.improvement_threshold,
            Some(7.5),
            "cli.improvement_threshold"
        );
        assert_eq!(
            merged.router_scoring.version,
            RouterScoringVersion::V1Legacy,
            "cli.router_scoring_version"
        );
        assert_eq!(merged.vias_allowed, Some(true), "cli.vias_allowed");
        assert_eq!(
            merged.automatic_neckdown,
            Some(false),
            "cli.automatic_neckdown"
        );
        assert_eq!(
            merged.trace_pull_tight_accuracy,
            Some(123),
            "cli.trace_pull_tight_accuracy"
        );
        assert_eq!(merged.strict_drc, Some(true), "cli.strict_drc");
        assert_eq!(merged.fanout_enabled, Some(false), "cli.fanout_enabled");
        assert_eq!(merged.fanout_max_passes, Some(2), "cli.fanout_max_passes");
        assert_eq!(merged.fanout_max_items, Some(77), "cli.fanout_max_items");
        assert_eq!(
            merged.fanout_max_milliseconds_per_pin,
            Some(1500),
            "cli.fanout_max_milliseconds_per_pin"
        );
        assert_eq!(
            merged.fanout_ripup_allowed,
            Some(false),
            "cli.fanout_ripup_allowed"
        );
        assert_eq!(
            merged.fanout_min_escape_length_mm,
            Some(1.5),
            "cli.fanout_min_escape_length_mm"
        );
        assert_eq!(
            merged.fanout_max_escape_length_mm,
            Some(6.5),
            "cli.fanout_max_escape_length_mm"
        );
        assert_eq!(
            merged.fanout_start_via_diameter_mm,
            Some(0.3),
            "cli.fanout_start_via_diameter_mm"
        );
        assert_eq!(
            merged.fanout_end_via_diameter_mm,
            Some(0.4),
            "cli.fanout_end_via_diameter_mm"
        );
        assert_eq!(
            merged.fanout_pin_sorting_order,
            Some("inner_first".to_string()),
            "cli.fanout_pin_sorting_order"
        );
        assert_eq!(
            merged.fanout_fallback_to_board_vias,
            Some(false),
            "cli.fanout_fallback_to_board_vias"
        );
        assert_eq!(
            merged.fanout_timeout_string,
            Some("90s".to_string()),
            "cli.fanout_timeout_string"
        );
        assert_eq!(merged.scoring.via_costs, Some(11), "cli.via_costs");
        assert_eq!(
            merged.scoring.plane_via_costs,
            Some(12),
            "cli.plane_via_costs"
        );
        assert_eq!(
            merged.scoring.start_ripup_costs,
            Some(13),
            "cli.start_ripup_costs"
        );
        // optimizer_scoring_version now MERGES into the optimizer score
        // box (M4-T8; the uptake test is
        // `optimizer_score_group_uptake_parse_merge_resolve` below).

        // Silence from BOTH layers reproduces the DefaultSettings seed
        // exactly (copy-if-set never nulls and never invents).
        let merged = merge(
            &MergedSettings::default(),
            &DsnLayer::default(),
            &CliLayer::default(),
        );
        assert_eq!(merged, MergedSettings::default());
    }

    /// Q10(c) (quality review): the `=` form of the manifest flag
    /// parses too — the flow witnesses exercise only the space form.
    #[test]
    fn result_json_equals_form_parses() {
        let parsed = parse_route_args(&args(&[
            "-de",
            "a.dsn",
            "-do",
            "b.ses",
            "--result-json=m.json",
        ]))
        .expect("parses");
        assert_eq!(parsed.result_json.as_deref(), Some("m.json"));
    }

    /// The M8-T3 gloss-bus tri-state faces (parse -> merge -> resolve):
    /// `on` resolves ON; absent and `off` both resolve OFF (the pass
    /// has no input-driven activation — the two-regime default face);
    /// a malformed value WARNs and stays OFF.
    #[test]
    fn gloss_bus_tri_state_parse_merge_resolve() {
        // `on` = ON.
        let parsed = parse_route_args(&args(&[
            "-de",
            "a.dsn",
            "-do",
            "b.ses",
            "--router.gloss.bus=on",
        ]))
        .expect("parses");
        assert!(parsed.layer.warnings.is_empty());
        let merged = merge(&MergedSettings::default(), &dsn_quiet(), &parsed.layer);
        assert_eq!(merged.gloss_bus, Some(true));
        let resolved = ResolvedRouteSettings::resolve(&merged, None);
        assert!(resolved.gloss_bus, "on resolves ON");
        // Absent = OFF.
        let parsed = parse_route_args(&args(&["-de", "a.dsn", "-do", "b.ses"])).expect("parses");
        let merged = merge(&MergedSettings::default(), &dsn_quiet(), &parsed.layer);
        assert_eq!(merged.gloss_bus, None);
        let resolved = ResolvedRouteSettings::resolve(&merged, None);
        assert!(!resolved.gloss_bus, "absent resolves OFF");
        // `off` = OFF (explicit kill face).
        let parsed = parse_route_args(&args(&[
            "-de",
            "a.dsn",
            "-do",
            "b.ses",
            "--router.gloss.bus=off",
        ]))
        .expect("parses");
        let merged = merge(&MergedSettings::default(), &dsn_quiet(), &parsed.layer);
        let resolved = ResolvedRouteSettings::resolve(&merged, None);
        assert!(!resolved.gloss_bus, "off resolves OFF");
        // Malformed = WARN and OFF.
        let parsed = parse_route_args(&args(&[
            "-de",
            "a.dsn",
            "-do",
            "b.ses",
            "--router.gloss.bus=maybe",
        ]))
        .expect("parses");
        assert_eq!(parsed.layer.warnings.len(), 1, "the warn-and-continue face");
        let merged = merge(&MergedSettings::default(), &dsn_quiet(), &parsed.layer);
        let resolved = ResolvedRouteSettings::resolve(&merged, None);
        assert!(!resolved.gloss_bus, "malformed stays OFF");
    }

    /// The M8-T4 gloss-flow tri-state faces (parse -> merge ->
    /// resolve): `on` resolves ON; absent and `off` both resolve OFF
    /// (no input-driven activation — the two-regime default face); a
    /// malformed value WARNs and stays OFF. The gloss-bus sibling's
    /// exact shape.
    #[test]
    fn gloss_flow_tri_state_parse_merge_resolve() {
        // `on` = ON.
        let parsed = parse_route_args(&args(&[
            "-de",
            "a.dsn",
            "-do",
            "b.ses",
            "--router.gloss.flow=on",
        ]))
        .expect("parses");
        assert!(parsed.layer.warnings.is_empty());
        let merged = merge(&MergedSettings::default(), &dsn_quiet(), &parsed.layer);
        assert_eq!(merged.gloss_flow, Some(true));
        let resolved = ResolvedRouteSettings::resolve(&merged, None);
        assert!(resolved.gloss_flow, "on resolves ON");
        // Absent = OFF.
        let parsed = parse_route_args(&args(&["-de", "a.dsn", "-do", "b.ses"])).expect("parses");
        let merged = merge(&MergedSettings::default(), &dsn_quiet(), &parsed.layer);
        assert_eq!(merged.gloss_flow, None);
        let resolved = ResolvedRouteSettings::resolve(&merged, None);
        assert!(!resolved.gloss_flow, "absent resolves OFF");
        // `off` = OFF (explicit kill face).
        let parsed = parse_route_args(&args(&[
            "-de",
            "a.dsn",
            "-do",
            "b.ses",
            "--router.gloss.flow=off",
        ]))
        .expect("parses");
        let merged = merge(&MergedSettings::default(), &dsn_quiet(), &parsed.layer);
        let resolved = ResolvedRouteSettings::resolve(&merged, None);
        assert!(!resolved.gloss_flow, "off resolves OFF");
        // Malformed = WARN and OFF.
        let parsed = parse_route_args(&args(&[
            "-de",
            "a.dsn",
            "-do",
            "b.ses",
            "--router.gloss.flow=maybe",
        ]))
        .expect("parses");
        assert_eq!(parsed.layer.warnings.len(), 1, "the warn-and-continue face");
        let merged = merge(&MergedSettings::default(), &dsn_quiet(), &parsed.layer);
        let resolved = ResolvedRouteSettings::resolve(&merged, None);
        assert!(!resolved.gloss_flow, "malformed stays OFF");
    }

    /// The M8-T6 gloss-teardrops tri-state faces (parse -> merge ->
    /// resolve): `on` resolves ON; absent and `off` both resolve OFF
    /// (no input-driven activation — the two-regime default face); a
    /// malformed value WARNs and stays OFF. The gloss-bus sibling's
    /// exact shape.
    #[test]
    fn gloss_teardrops_tri_state_parse_merge_resolve() {
        // `on` = ON.
        let parsed = parse_route_args(&args(&[
            "-de",
            "a.dsn",
            "-do",
            "b.ses",
            "--router.gloss.teardrops=on",
        ]))
        .expect("parses");
        assert!(parsed.layer.warnings.is_empty());
        let merged = merge(&MergedSettings::default(), &dsn_quiet(), &parsed.layer);
        assert_eq!(merged.gloss_teardrops, Some(true));
        let resolved = ResolvedRouteSettings::resolve(&merged, None);
        assert!(resolved.gloss_teardrops, "on resolves ON");
        // Absent = OFF.
        let parsed = parse_route_args(&args(&["-de", "a.dsn", "-do", "b.ses"])).expect("parses");
        let merged = merge(&MergedSettings::default(), &dsn_quiet(), &parsed.layer);
        assert_eq!(merged.gloss_teardrops, None);
        let resolved = ResolvedRouteSettings::resolve(&merged, None);
        assert!(!resolved.gloss_teardrops, "absent resolves OFF");
        // `off` = OFF (explicit kill face).
        let parsed = parse_route_args(&args(&[
            "-de",
            "a.dsn",
            "-do",
            "b.ses",
            "--router.gloss.teardrops=off",
        ]))
        .expect("parses");
        let merged = merge(&MergedSettings::default(), &dsn_quiet(), &parsed.layer);
        let resolved = ResolvedRouteSettings::resolve(&merged, None);
        assert!(!resolved.gloss_teardrops, "off resolves OFF");
        // Malformed = WARN and OFF.
        let parsed = parse_route_args(&args(&[
            "-de",
            "a.dsn",
            "-do",
            "b.ses",
            "--router.gloss.teardrops=maybe",
        ]))
        .expect("parses");
        assert_eq!(parsed.layer.warnings.len(), 1, "the warn-and-continue face");
        let merged = merge(&MergedSettings::default(), &dsn_quiet(), &parsed.layer);
        let resolved = ResolvedRouteSettings::resolve(&merged, None);
        assert!(!resolved.gloss_teardrops, "malformed stays OFF");
    }

    /// A neutral DSN layer (no metadata, one layer) for the merge
    /// helpers.
    fn dsn_quiet() -> DsnLayer {
        DsnLayer::from_metadata(None, 0)
    }

    /// The M4-T7 fanout group's UPTAKE face: every CLI flag rides
    /// parse -> merge -> resolve into `RouterSettingsIr.fanout` (the
    /// uptake surface is the RESOLVED IR, never the WARN line — buglog
    /// 168). Worlds: (1) every flag set (including the
    /// `fanout.timeout_string` spelling Java's reflection walk also
    /// accepts); (2) silence reproduces the `DefaultSettings.java:
    /// 163-177` seeds; (3) a malformed value WARNs and leaves the
    /// default in place; (4) an unknown fanout path WARNs as
    /// unsupported.
    #[test]
    fn fanout_group_uptake_parse_merge_resolve() {
        // World 1: every flag set.
        let parsed = parse_route_args(&args(&[
            "-de",
            "a.dsn",
            "-do",
            "b.ses",
            "--router.fanout.enabled=off",
            "--router.fanout.max_passes=3",
            "--router.fanout.max_items=55",
            "--router.fanout.max_milliseconds_per_pin=2500",
            "--router.fanout.ripup_allowed=off",
            "--router.fanout.min_escape_length_mm=1.25",
            "--router.fanout.max_escape_length_mm=7.75",
            "--router.fanout.start_via_diameter_mm=0.35",
            "--router.fanout.end_via_diameter_mm=0.45",
            "--router.fanout.pin_sorting_order=distanceToClosestOnNet",
            "--router.fanout.fallback_to_board_vias=off",
            "--router.fanout.timeout=2:30",
        ]))
        .expect("parses");
        assert!(
            parsed.layer.warnings.is_empty(),
            "no warns on the well-formed group: {:?}",
            parsed.layer.warnings
        );
        let mut merged = merge(
            &MergedSettings::default(),
            &DsnLayer::default(),
            &parsed.layer,
        );
        assert!(crate::settings::validate(&mut merged).is_empty());
        let resolved = ResolvedRouteSettings::resolve(&merged, None);
        let fanout = &resolved.router_settings.fanout;
        assert!(!fanout.enabled);
        assert_eq!(fanout.max_passes, 3);
        assert_eq!(fanout.max_items, 55);
        assert_eq!(fanout.max_milliseconds_per_pin, 2500);
        assert!(!fanout.ripup_allowed);
        assert_eq!(fanout.min_escape_length_mm, Some(1.25));
        assert_eq!(fanout.max_escape_length_mm, Some(7.75));
        assert_eq!(fanout.start_via_diameter_mm, Some(0.35));
        assert_eq!(fanout.end_via_diameter_mm, Some(0.45));
        assert_eq!(fanout.pin_sorting_order, "distanceToClosestOnNet");
        assert!(!fanout.fallback_to_board_vias);
        assert_eq!(fanout.timeout_string, Some("2:30".to_string()));

        // The field-name spelling `fanout.timeout_string` is ALSO a Java
        // key (`ReflectionUtil.getFieldByNameOrSerializedName` matches
        // the SerializedName AND the field name); both spellings land in
        // the same slot.
        let parsed_alias = parse_route_args(&args(&[
            "-de",
            "a.dsn",
            "-do",
            "b.ses",
            "--router.fanout.timeout_string=5m",
        ]))
        .expect("alias parses");
        assert!(parsed_alias.layer.fanout_timeout_string == Some("5m".to_string()));

        // World 2: silence = the DefaultSettings seeds.
        let mut merged = merge(
            &MergedSettings::default(),
            &DsnLayer::default(),
            &CliLayer::default(),
        );
        assert!(crate::settings::validate(&mut merged).is_empty());
        let resolved = ResolvedRouteSettings::resolve(&merged, None);
        assert_eq!(
            resolved.router_settings.fanout,
            FanoutSettingsIr::default(),
            "the resolved silence face IS the DefaultSettings fanout group"
        );

        // World 3: a malformed value WARNs and the default survives
        // (Java: NumberFormatException -> the warn row, field untouched).
        let parsed = parse_route_args(&args(&[
            "-de",
            "a.dsn",
            "-do",
            "b.ses",
            "--router.fanout.max_passes=many",
        ]))
        .expect("parses");
        assert_eq!(parsed.layer.warnings.len(), 1, "the bad-value warn");
        // No `validate` in this world — the failed apply is observed at
        // resolve time directly (the default already survived).
        let merged = merge(
            &MergedSettings::default(),
            &DsnLayer::default(),
            &parsed.layer,
        );
        let resolved = ResolvedRouteSettings::resolve(&merged, None);
        assert_eq!(
            resolved.router_settings.fanout.max_passes, 20,
            "the DefaultSettings seed survives the failed apply"
        );

        // World 4: an unknown fanout path WARNs as unsupported (Java:
        // NoSuchFieldException -> the warn row).
        let parsed = parse_route_args(&args(&[
            "-de",
            "a.dsn",
            "-do",
            "b.ses",
            "--router.fanout.no_such_field=1",
        ]))
        .expect("parses");
        assert!(
            parsed.layer.warnings.len() == 1 && parsed.layer.warnings[0].contains("no_such_field")
        );
    }
    /// The M4-T8 optimizer-score group's UPTAKE face (the uptake
    /// surface is the RESOLVED box, never the WARN line — buglog 168).
    /// Java-exact CLI surface: ONLY the version is settable (`--optimizer.scoring.version`
    /// special case, the `-optimizer-scoring-version` flag, and the
    /// `-scoring-version` dual apply); the scalars keep the
    /// `DefaultSettings.java:92-121, 215-220` seeds headlessly.
    /// Worlds: (1) version=v2 → V2_LOWER_BOUND merged + resolved;
    /// (2) the dual-apply asymmetry lands on BOTH boxes;
    /// (3) `--optimizer_scoring.<scalar>` is SILENTLY ignored (the
    /// CliSettings routing guard) and (4) `--optimizer.scoring.<scalar>`
    /// hits the reflection-miss WARN — neither touches the box;
    /// (5) silence reproduces the DefaultSettings seeds verbatim
    /// (jar-anchored: the bm08 manifest `settings_snapshot.
    /// optimizer_scoring` carries exactly this box).
    #[test]
    fn optimizer_score_group_uptake_parse_merge_resolve() {
        // (1) The version flag.
        let parsed = parse_route_args(&args(&[
            "-de",
            "a.dsn",
            "-do",
            "b.ses",
            "--optimizer.scoring.version=v2",
        ]))
        .expect("parses");
        assert_eq!(
            parsed.layer.optimizer_scoring_version,
            Some(OptimScoringVersion::V2LowerBound),
            "the v2 alias resolves to V2_LOWER_BOUND on the optimizer box"
        );
        let merged = merge(
            &MergedSettings::default(),
            &DsnLayer::default(),
            &parsed.layer,
        );
        assert_eq!(
            merged.optimizer_scoring.version,
            OptimizerScoringVersion::V2LowerBound
        );
        let resolved = ResolvedRouteSettings::resolve(&merged, None);
        assert_eq!(
            resolved
                .scoring
                .optimizer_scoring
                .as_ref()
                .expect("box present")
                .version,
            OptimizerScoringVersion::V2LowerBound,
            "the resolved face carries the box (the T9 stage's entry point)"
        );

        // (2) The dual-apply asymmetry lands on BOTH boxes.
        let parsed = parse_route_args(&args(&[
            "-de",
            "a.dsn",
            "-do",
            "b.ses",
            "-scoring-version",
            "v2",
        ]))
        .expect("parses");
        assert_eq!(
            parsed.layer.router_scoring_version,
            Some(RouterScoringVersion::V2Continuous),
            "the router box takes V2_CONTINUOUS"
        );
        assert_eq!(
            parsed.layer.optimizer_scoring_version,
            Some(OptimScoringVersion::V2LowerBound),
            "the optimizer box takes V2_LOWER_BOUND"
        );
        let merged = merge(
            &MergedSettings::default(),
            &DsnLayer::default(),
            &parsed.layer,
        );
        assert_eq!(
            merged.router_scoring.version,
            RouterScoringVersion::V2Continuous
        );
        assert_eq!(
            merged.optimizer_scoring.version,
            OptimizerScoringVersion::V2LowerBound
        );

        // (3) `--optimizer_scoring.<scalar>`: SILENTLY ignored (outside
        // the router./optimizer. routing guard) — no warn, no field.
        let parsed = parse_route_args(&args(&[
            "-de",
            "a.dsn",
            "-do",
            "b.ses",
            "--optimizer_scoring.excess_via_weight=2400",
        ]))
        .expect("parses");
        assert!(
            parsed.layer.warnings.is_empty(),
            "silent-ignore parity: {:?}",
            parsed.layer.warnings
        );
        // (4) `--optimizer.scoring.<scalar>`: the reflection-miss WARN,
        // field untouched.
        let parsed = parse_route_args(&args(&[
            "-de",
            "a.dsn",
            "-do",
            "b.ses",
            "--optimizer.scoring.excess_via_weight=2400",
        ]))
        .expect("parses");
        assert!(
            parsed.layer.warnings.len() == 1
                && parsed.layer.warnings[0].contains("excess_via_weight"),
            "the reflection-miss warn row: {:?}",
            parsed.layer.warnings
        );

        // (5) Silence reproduces the DefaultSettings seeds verbatim —
        // the same box the jar's manifest `settings_snapshot.
        // optimizer_scoring` renders (evidence jar-bounds/bm08-manifest.json).
        let merged = merge(
            &MergedSettings::default(),
            &DsnLayer::default(),
            &CliLayer::default(),
        );
        assert_eq!(
            merged.optimizer_scoring,
            default_optimizer_score_settings(),
            "V2_LOWER_BOUND / 1000 / 2000 / 500 / 1.0 / 1.0"
        );
    }

    // -----------------------------------------------------------------------
    // M4-T9: the optimizer settings family
    // -----------------------------------------------------------------------

    /// The `--optimizer.*` CLI face: snake_case SerializedName and
    /// camelCase field-name spellings both apply (`ReflectionUtil`
    /// matches either); the improvement-threshold x100 quirk covers the
    /// optimizer spellings too; a bad value WARNs and leaves the field
    /// unset; silence keeps the DefaultSettings seeds
    /// (`DefaultSettings.java:177-191`).
    #[test]
    fn t9_optimizer_cli_family_parses_and_merges() {
        // (1) snake_case spellings.
        let parsed = parse_route_args(&args(&[
            "--optimizer.enabled=off",
            "--optimizer.max_passes=7",
            "--optimizer.max_items=11",
            "--optimizer.max_threads=2",
            "--optimizer.enable_preflight_guards=off",
            "--optimizer.max_consecutive_failures=21",
            "--optimizer.max_consecutive_failures_pass1=5",
            "--optimizer.additional_ripup_cost_factor_at_start=8",
            "--optimizer.trace_ripup_cost_factor=0.8",
            "--optimizer.max_autoroute_passes=3",
            "--optimizer.timeout=1:30",
            "--optimizer.algorithm=hyper",
        ]))
        .expect("parse");
        let layer = &parsed.layer;
        assert_eq!(layer.optimizer_enabled, Some(false));
        assert_eq!(layer.optimizer_max_passes, Some(7));
        assert_eq!(layer.optimizer_max_items, Some(11));
        assert_eq!(layer.optimizer_max_threads, Some(2));
        assert_eq!(layer.optimizer_enable_preflight_guards, Some(false));
        assert_eq!(layer.optimizer_max_consecutive_failures, Some(21));
        assert_eq!(layer.optimizer_max_consecutive_failures_pass1, Some(5));
        assert_eq!(
            layer.optimizer_additional_ripup_cost_factor_at_start,
            Some(8)
        );
        assert_eq!(layer.optimizer_trace_ripup_cost_factor, Some(0.8));
        assert_eq!(layer.optimizer_max_autoroute_passes, Some(3));
        assert_eq!(layer.optimizer_timeout_string.as_deref(), Some("1:30"));
        assert_eq!(layer.optimizer_algorithm.as_deref(), Some("hyper"));
        assert!(layer.warnings.is_empty(), "{:?}", layer.warnings);

        // (2) camelCase field-name spellings apply to the SAME slots.
        let parsed = parse_route_args(&args(&[
            "--optimizer.maxPasses=9",
            "--optimizer.maxAutoroutePasses=4",
            "--optimizer.enablePreflightGuards=on",
            "--optimizer.traceRipupCostFactor=0.5",
            "--optimizer.timeoutString=2:00",
        ]))
        .expect("parse");
        let layer = &parsed.layer;
        assert_eq!(layer.optimizer_max_passes, Some(9));
        assert_eq!(layer.optimizer_max_autoroute_passes, Some(4));
        assert_eq!(layer.optimizer_enable_preflight_guards, Some(true));
        assert_eq!(layer.optimizer_trace_ripup_cost_factor, Some(0.5));
        assert_eq!(layer.optimizer_timeout_string.as_deref(), Some("2:00"));

        // (3) The threshold quirk: optimizer spellings get the fraction
        // x100 treatment, including the camelCase field name.
        let parsed = parse_route_args(&args(&[
            "--optimizer.improvement_threshold=0.025",
            "--optimizer.optimizationImprovementThreshold=0.05",
        ]))
        .expect("parse");
        // The SECOND flag overwrites the first (both write the same slot).
        assert_eq!(parsed.layer.improvement_threshold, Some(5.0));
        assert!(parsed.layer.warnings.is_empty());

        // (3b) MIN-3: the NON-FINITE sanitation inputs are reachable
        // through the CLI face too — Rust parses "NaN"/"inf" exactly
        // like Java's Float.parseFloat("NaN"/"Infinity"), and the quirk
        // block's (0, 1) interval excludes both, so the raw value rides
        // to the stage's reset arm (pinned in optimizer.rs).
        let parsed = parse_route_args(&args(&[
            "--optimizer.improvement_threshold=NaN",
            "--optimizer.improvement_threshold=inf",
        ]))
        .expect("parse");
        let threshold = parsed
            .layer
            .improvement_threshold
            .expect("the non-finite value rides to the layer");
        assert!(
            threshold.is_infinite() && threshold.is_sign_positive(),
            "the +inf face: {threshold}",
        );

        // (4) A bad value warns and leaves the slot unset.
        let parsed = parse_route_args(&args(&["--optimizer.max_passes=nope"])).expect("parse");
        assert_eq!(parsed.layer.optimizer_max_passes, None);
        assert!(
            parsed
                .layer
                .warnings
                .iter()
                .any(|w| w.contains("optimizer.max_passes") && w.contains("nope")),
            "the warn row: {:?}",
            parsed.layer.warnings
        );

        // (5) Merge over defaults: only the CLI opinions move.
        let mut merged = merge(
            &MergedSettings::default(),
            &DsnLayer::default(),
            &parsed.layer,
        );
        // Silence on every other optimizer slot keeps the seeds:
        assert_eq!(merged.optimizer_enabled, Some(true));
        assert_eq!(merged.optimizer_max_passes, Some(100));
        assert_eq!(merged.optimizer_max_items, Some(i32::MAX));
        assert_eq!(merged.optimizer_enable_preflight_guards, Some(true));
        assert_eq!(merged.optimizer_max_consecutive_failures, Some(50));
        assert_eq!(merged.optimizer_max_consecutive_failures_pass1, Some(12));
        assert_eq!(
            merged.optimizer_additional_ripup_cost_factor_at_start,
            Some(10)
        );
        assert_eq!(merged.optimizer_trace_ripup_cost_factor, Some(0.6));
        assert_eq!(merged.optimizer_max_autoroute_passes, Some(6));
        assert_eq!(merged.improvement_threshold, Some(2.5));
        assert_eq!(
            merged.optimizer_algorithm.as_deref(),
            Some("freerouting-optimizer")
        );

        // And the DSN `(postroute off)` face turns the stage gate off.
        let dsn = DsnLayer {
            run_optimizer: Some(false),
            ..DsnLayer::default()
        };
        merged = merge(&MergedSettings::default(), &dsn, &CliLayer::default());
        assert_eq!(merged.optimizer_enabled, Some(false));

        // (6) The resolve product: getRunOptimizer()'s null face is
        // FALSE (RouterSettings.java:564-566), the group fields ride
        // verbatim, and the two factors resolve to scalars.
        merged = merge(
            &MergedSettings::default(),
            &DsnLayer::default(),
            &CliLayer::default(),
        );
        let resolved = ResolvedRouteSettings::resolve(&merged, None);
        assert!(resolved.run_optimizer, "DefaultSettings seeds enabled=true");
        assert_eq!(resolved.optimizer.algorithm, "freerouting-optimizer");
        assert_eq!(resolved.optimizer.max_passes, Some(100));
        assert_eq!(resolved.optimizer.max_items, Some(i32::MAX));
        assert_eq!(resolved.optimizer.improvement_threshold, Some(2.5));
        assert_eq!(resolved.optimizer.enable_preflight_guards, Some(true));
        assert_eq!(resolved.optimizer.max_consecutive_failures, Some(50));
        assert_eq!(resolved.optimizer.max_consecutive_failures_pass1, Some(12));
        assert_eq!(resolved.optimizer.additional_ripup_cost_factor_at_start, 10);
        assert_eq!(resolved.optimizer.trace_ripup_cost_factor, 0.6);
        assert_eq!(resolved.optimizer.max_autoroute_passes, 6);
        assert_eq!(resolved.optimizer.timeout_string, None);

        let disabled = MergedSettings {
            optimizer_enabled: None,
            ..merged.clone()
        };
        assert!(
            !ResolvedRouteSettings::resolve(&disabled, None).run_optimizer,
            "enabled == null reads as FALSE (not the default-true face)",
        );
    }

    /// The M6-T6 Rust-only clamp flag: default OFF, `on|off` uptake
    /// through the `router.*` guard path, and the resolved product
    /// carries it into BatchSettings' seam.
    #[test]
    fn plane_island_clamp_default_off_and_uptake() {
        // World 1: silence = OFF at every layer.
        let parsed = parse_route_args(&args(&["-de", "a.dsn", "-do", "b.ses"])).expect("parses");
        let merged = merge(
            &MergedSettings::default(),
            &DsnLayer::default(),
            &parsed.layer,
        );
        let resolved = ResolvedRouteSettings::resolve(&merged, None);
        assert!(!resolved.plane_island_clamp, "default OFF");

        // World 2: explicit uptake both spellings of the boolean.
        let on = parse_route_args(&args(&[
            "-de",
            "a.dsn",
            "-do",
            "b.ses",
            "--router.plane_island_clamp=on",
        ]))
        .expect("parses");
        assert!(on.layer.plane_island_clamp == Some(true));
        assert!(
            on.layer.warnings.is_empty(),
            "no warns on the Rust-only flag: {:?}",
            on.layer.warnings
        );
        let merged_on = merge(&MergedSettings::default(), &DsnLayer::default(), &on.layer);
        assert_eq!(merged_on.plane_island_clamp, Some(true));
        assert!(ResolvedRouteSettings::resolve(&merged_on, None).plane_island_clamp);

        let off = parse_route_args(&args(&[
            "-de",
            "a.dsn",
            "-do",
            "b.ses",
            "--router.plane_island_clamp=off",
        ]))
        .expect("parses");
        assert!(
            !ResolvedRouteSettings::resolve(
                &merge(&MergedSettings::default(), &DsnLayer::default(), &off.layer),
                None
            )
            .plane_island_clamp
        );

        // World 3: a malformed value WARNs and leaves OFF in place.
        let bad = parse_route_args(&args(&[
            "-de",
            "a.dsn",
            "-do",
            "b.ses",
            "--router.plane_island_clamp=yes-please",
        ]))
        .expect("parses");
        assert!(!bad.layer.warnings.is_empty(), "the malformed value warns");
        assert!(
            !ResolvedRouteSettings::resolve(
                &merge(&MergedSettings::default(), &DsnLayer::default(), &bad.layer),
                None
            )
            .plane_island_clamp
        );
    }

    /// The M6-T7 seam: default OFF through the whole chain, uptake via
    /// the explicit CLI face, the sub-flag independent of the master.
    #[test]
    fn congestion_global_default_off_and_uptake() {
        fn args(flags: &[&str]) -> Vec<String> {
            let mut argv = vec![
                "-de".to_string(),
                "a.dsn".to_string(),
                "-do".to_string(),
                "b.ses".to_string(),
            ];
            argv.extend(flags.iter().map(|flag| (*flag).to_string()));
            argv
        }

        // World 1: defaults — OFF everywhere.
        let defaults = parse_route_args(&args(&[])).expect("parses");
        assert!(defaults.layer.congestion_global.is_none(), "no initializer");
        let merged = merge(
            &MergedSettings::default(),
            &DsnLayer::default(),
            &defaults.layer,
        );
        assert!(merged.congestion_global.is_none());
        let resolved = ResolvedRouteSettings::resolve(&merged, None);
        assert!(!resolved.congestion_global, "default OFF");
        assert!(!resolved.congestion_global_pattern, "default OFF");

        // World 2: master uptake.
        let on = parse_route_args(&args(&["--router.congestion_global=on"])).expect("parses");
        assert!(on.layer.congestion_global == Some(true));
        assert!(
            on.layer.warnings.is_empty(),
            "no warns on the Rust-only flag: {:?}",
            on.layer.warnings
        );
        let merged_on = merge(&MergedSettings::default(), &DsnLayer::default(), &on.layer);
        assert_eq!(merged_on.congestion_global, Some(true));
        assert!(ResolvedRouteSettings::resolve(&merged_on, None).congestion_global);

        // World 3: the sub-flag parses; without the master the RESOLVED
        // pair is (true pattern, false master) — the sub-flag is inert
        // in the engine without the master (batch.rs ctor + uptake).
        let pattern_only =
            parse_route_args(&args(&["--router.congestion_global.pattern=on"])).expect("parses");
        assert!(pattern_only.layer.congestion_global_pattern == Some(true));
        let merged_pattern = merge(
            &MergedSettings::default(),
            &DsnLayer::default(),
            &pattern_only.layer,
        );
        let resolved_pattern = ResolvedRouteSettings::resolve(&merged_pattern, None);
        assert!(resolved_pattern.congestion_global_pattern);
        assert!(!resolved_pattern.congestion_global, "master stays OFF");

        // World 4 (M6-T8): the pathfinder sub-flag — same chain, same
        // inert-without-the-master face.
        let defaults = parse_route_args(&args(&[])).expect("parses");
        assert!(
            defaults.layer.congestion_global_pathfinder.is_none(),
            "no initializer"
        );
        let merged_defaults = merge(
            &MergedSettings::default(),
            &DsnLayer::default(),
            &defaults.layer,
        );
        assert!(merged_defaults.congestion_global_pathfinder.is_none());
        let resolved_defaults = ResolvedRouteSettings::resolve(&merged_defaults, None);
        assert!(
            !resolved_defaults.congestion_global_pathfinder,
            "default OFF"
        );

        let pf_on =
            parse_route_args(&args(&["--router.congestion_global.pathfinder=on"])).expect("parses");
        assert!(pf_on.layer.congestion_global_pathfinder == Some(true));
        assert!(
            pf_on.layer.warnings.is_empty(),
            "no warns on the Rust-only flag: {:?}",
            pf_on.layer.warnings
        );
        let merged_pf = merge(
            &MergedSettings::default(),
            &DsnLayer::default(),
            &pf_on.layer,
        );
        assert_eq!(merged_pf.congestion_global_pathfinder, Some(true));
        let resolved_pf = ResolvedRouteSettings::resolve(&merged_pf, None);
        assert!(resolved_pf.congestion_global_pathfinder);
        assert!(!resolved_pf.congestion_global, "master stays OFF");
    }

    // -------------------------------------------------------------------
    // the BatchSettings activation predicates (moved from epic-cli
    // route.rs with their code, M9-T1 — the unit pins travel with the
    // unit)
    // -------------------------------------------------------------------

    /// The clamp flag flows into BatchSettings through
    /// `build_batch_settings` (default OFF; explicit ON survives).
    #[test]
    fn build_batch_settings_carries_plane_island_clamp() {
        let merged = MergedSettings::default();
        let default_batch = build_batch_settings(&ResolvedRouteSettings::resolve(&merged, None));
        assert!(!default_batch.plane_island_clamp, "default OFF");

        let merged_on = MergedSettings {
            plane_island_clamp: Some(true),
            ..MergedSettings::default()
        };
        let batch_on = build_batch_settings(&ResolvedRouteSettings::resolve(&merged_on, None));
        assert!(batch_on.plane_island_clamp);
    }

    /// The M6-T7 seam: build_batch_settings carries the master + the
    /// pattern sub-flag, both default OFF.
    #[test]
    fn build_batch_settings_carries_congestion_global() {
        let merged = MergedSettings::default();
        let default_batch = build_batch_settings(&ResolvedRouteSettings::resolve(&merged, None));
        assert!(!default_batch.congestion_global, "default OFF");
        assert!(!default_batch.congestion_global_pattern, "default OFF");

        let merged_on = MergedSettings {
            congestion_global: Some(true),
            congestion_global_pattern: Some(true),
            ..MergedSettings::default()
        };
        let batch_on = build_batch_settings(&ResolvedRouteSettings::resolve(&merged_on, None));
        assert!(batch_on.congestion_global);
        assert!(batch_on.congestion_global_pattern);
    }

    /// The M8-T3 seam: build_batch_settings carries the gloss-bus flag
    /// (default OFF — the two-regime law; explicit ON survives).
    #[test]
    fn build_batch_settings_carries_gloss_bus() {
        let merged = MergedSettings::default();
        let default_batch = build_batch_settings(&ResolvedRouteSettings::resolve(&merged, None));
        assert!(!default_batch.bus_active, "default OFF");

        let merged_on = MergedSettings {
            gloss_bus: Some(true),
            ..MergedSettings::default()
        };
        let batch_on = build_batch_settings(&ResolvedRouteSettings::resolve(&merged_on, None));
        assert!(batch_on.bus_active, "explicit ON survives");
    }

    /// The M8-T4 seam: build_batch_settings carries the gloss-flow
    /// flag (default OFF — the two-regime law; explicit ON survives).
    #[test]
    fn build_batch_settings_carries_gloss_flow() {
        let merged = MergedSettings::default();
        let default_batch = build_batch_settings(&ResolvedRouteSettings::resolve(&merged, None));
        assert!(!default_batch.flow_active, "default OFF");

        let merged_on = MergedSettings {
            gloss_flow: Some(true),
            ..MergedSettings::default()
        };
        let batch_on = build_batch_settings(&ResolvedRouteSettings::resolve(&merged_on, None));
        assert!(batch_on.flow_active, "explicit ON survives");
    }

    /// The fanout-to-tail-sweep coupling pin (route.rs's
    /// `batch_settings_wiring_fanout_to_remove_unconnected_vias`,
    /// moved with its code, M9-T1): the job ctor's
    /// `removeUnconnectedVias = !isFanoutEnabled()` — fanout-off ALSO
    /// flips the batch tail sweep, so the coupling is
    /// double-load-bearing for T14. The pin asserts the REAL batch face
    /// through the same builder the flow uses (the `= false` mutant
    /// passed the whole suite before this pin existed).
    #[test]
    fn batch_settings_wiring_fanout_to_remove_unconnected_vias() {
        let default_batch = build_batch_settings(&ResolvedRouteSettings::resolve(
            &MergedSettings::default(),
            None,
        ));
        assert!(
            default_batch.fanout_enabled,
            "the DefaultSettings seed fans out"
        );
        assert!(
            !default_batch.remove_unconnected_vias,
            "fanout on -> the tail sweep is NOT run"
        );

        let merged = MergedSettings {
            fanout_enabled: Some(false),
            ..MergedSettings::default()
        };
        let batch = build_batch_settings(&ResolvedRouteSettings::resolve(&merged, None));
        assert!(!batch.fanout_enabled);
        assert!(
            batch.remove_unconnected_vias,
            "fanout off -> the tail sweep removes unconnected vias"
        );
    }

    /// The M7-T2 pin world: a `BoardRules` with one unconstrained
    /// default class + one constrained class, two nets (1 plain, 2 in
    /// the constrained class). Assembled through the pub IR face
    /// (`BoardRules::from_ir`) — the same construction the parse uses.
    fn tuning_world(min: f64, max: f64) -> epic_board::rules_surf::BoardRules {
        use epic_dsn::sink::{BoardRulesIr, NetClassIr, NetIr};
        let class = NetClassIr {
            name: "constrained".to_string(),
            trace_clearance_class: 1,
            trace_half_widths: vec![1500],
            active_routing_layers: vec![true],
            default_item_clearance_classes: [0, 1, 1, 1, 1, 1],
            via_rule: None,
            pull_tight: true,
            shove_fixed: false,
            min_trace_length: min,
            max_trace_length: max,
            nets: Vec::new(),
        };
        let default = NetClassIr {
            name: "default".to_string(),
            ..class.clone()
        };
        let rules_ir = BoardRulesIr {
            clearance: epic_dsn::sink::ClearanceIr::default_instance(1),
            trace_angle_restriction: epic_dsn::state::AngleRestriction::FortyfiveDegree,
            default_trace_half_widths: vec![1500],
            min_trace_half_width: 1500,
            max_trace_half_width: 1500,
            pin_edge_to_turn_dist: 0.0,
            default_item_clearance_classes: [0, 1, 1, 1, 1, 1],
        };
        epic_board::rules_surf::BoardRules::from_ir(
            &rules_ir,
            &[
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
            ],
            &[default, class],
            &[],
            &[],
        )
    }

    /// Helper: a `ResolvedRouteSettings` with the default tri-state.
    fn resolved_none() -> ResolvedRouteSettings {
        ResolvedRouteSettings::resolve(&MergedSettings::default(), None)
    }

    /// The constraint-free ZERO-ROTATION negative face: default
    /// settings + no declaration ⇒ the predicate is false ⇒ tuning
    /// never fires.
    #[test]
    fn tuning_activation_default_and_unconstrained_is_off() {
        let mut batch = build_batch_settings(&resolved_none());
        let rules = tuning_world(0.0, 0.0);
        apply_tuning_activation(&mut batch, &resolved_none(), &rules);
        assert!(
            !batch.tuning_active,
            "no declaration + default tri-state (None): input-driven OFF"
        );
    }

    /// The tri-state faces: the declaration alone (tri-state None)
    /// activates; the kill-switch silences a declared board; the
    /// override fires on constraint-free input. The M2 mutant (the
    /// kill-switch arm answering true) dies here; the M1 mutant
    /// (`>= 0.0` in the board predicate) dies in the epic-board pins.
    #[test]
    fn tuning_activation_tri_state_faces() {
        let rules = tuning_world(10.0, 20.0);

        // Input-driven ON.
        let mut batch = build_batch_settings(&resolved_none());
        apply_tuning_activation(&mut batch, &resolved_none(), &rules);
        assert!(batch.tuning_active, "a declaration activates (None)");

        // Kill-switch: Some(false) silences a declared board.
        let merged_kill = MergedSettings {
            tuning: Some(false),
            ..MergedSettings::default()
        };
        let resolved_kill = ResolvedRouteSettings::resolve(&merged_kill, None);
        let mut batch = build_batch_settings(&resolved_none());
        apply_tuning_activation(&mut batch, &resolved_kill, &rules);
        assert!(
            !batch.tuning_active,
            "the kill-switch silences a declared board"
        );

        // Override: Some(true) fires on constraint-free input.
        let rules_free = tuning_world(0.0, 0.0);
        let merged_on = MergedSettings {
            tuning: Some(true),
            ..MergedSettings::default()
        };
        let resolved_on = ResolvedRouteSettings::resolve(&merged_on, None);
        let mut batch = build_batch_settings(&resolved_none());
        apply_tuning_activation(&mut batch, &resolved_on, &rules_free);
        assert!(
            batch.tuning_active,
            "the override fires on constraint-free input"
        );
    }

    /// The meander tri-state resolution (the separated predicate):
    /// None rides the resolved tuning activation; `on` forces the
    /// stage on a tuning-inert board; `off` kills the stage alone
    /// while `tuning_active` (the gate's flag) stays true.
    #[test]
    fn meander_activation_tri_state_faces() {
        let rules_free = tuning_world(0.0, 0.0);
        let rules_declared = tuning_world(10.0, 0.0);
        // None + free board: rides the (false) activation.
        let mut batch = build_batch_settings(&resolved_none());
        apply_tuning_activation(&mut batch, &resolved_none(), &rules_free);
        apply_meander_activation(&mut batch, &resolved_none());
        assert!(!batch.meander_active, "None rides input-driven OFF");
        // None + declared board: rides the (true) activation.
        let mut batch = build_batch_settings(&resolved_none());
        apply_tuning_activation(&mut batch, &resolved_none(), &rules_declared);
        apply_meander_activation(&mut batch, &resolved_none());
        assert!(batch.meander_active, "None rides input-driven ON");
        // `on` forces on a constraint-free board.
        let merged_on = MergedSettings {
            tuning_meander: Some(true),
            ..MergedSettings::default()
        };
        let resolved_on = ResolvedRouteSettings::resolve(&merged_on, None);
        let mut batch = build_batch_settings(&resolved_none());
        apply_tuning_activation(&mut batch, &resolved_on, &rules_free);
        apply_meander_activation(&mut batch, &resolved_on);
        assert!(batch.meander_active, "on forces");
        // `off` kills the stage alone.
        let merged_off = MergedSettings {
            tuning_meander: Some(false),
            ..MergedSettings::default()
        };
        let resolved_off = ResolvedRouteSettings::resolve(&merged_off, None);
        let mut batch = build_batch_settings(&resolved_none());
        apply_tuning_activation(&mut batch, &resolved_off, &rules_declared);
        apply_meander_activation(&mut batch, &resolved_off);
        assert!(!batch.meander_active, "off kills the stage alone");
        assert!(batch.tuning_active, "the honoring gate's flag stays armed");
    }

    // -------------------------------------------------------------------
    // the session layer (M9-T1 pins)
    // -------------------------------------------------------------------

    /// PIN SL-1: an EMPTY SessionLayer is a NO-OP — the merged model
    /// after `merge_session` is EXACTLY the model before it: full
    /// structural equality (PartialEq over every field, the
    /// serialization-equivalent face — `MergedSettings` carries no
    /// serde `Serialize` derive and the relocation adds none) PLUS
    /// byte-equality of the Debug serialization of both models. (The
    /// charter's "serialize both, compare" face, honored through the
    /// derive `MergedSettings` already carries; noted in the M9-T1
    /// report.)
    #[test]
    fn session_layer_empty_is_a_no_op() {
        let cli = CliLayer {
            max_passes: Some(5),
            tuning: Some(false),
            gloss_bus: Some(true),
            ..CliLayer::default()
        };
        let mut merged = merge(&MergedSettings::default(), &DsnLayer::default(), &cli);
        let before = merged.clone();

        merge_session(&mut merged, &SessionLayer::default());

        assert_eq!(merged, before, "an empty session layer changes nothing");
        assert_eq!(
            format!("{merged:?}"),
            format!("{before:?}"),
            "byte-equal Debug serialization pre/post merge_session"
        );
    }

    /// PIN SL-2: every `Some` session field WINS over the CLI-layer
    /// value on the same slot (the session layer merges ABOVE the CLI
    /// layer), across the whole field set — gates, bounds, threads,
    /// every router.* tri-state family member, AND the M9-T2
    /// re-audit additions (the scoring trio, strict_drc,
    /// improvement_threshold, fanout_enabled, the optimizer bounds).
    #[test]
    fn session_layer_some_fields_win_over_cli() {
        let cli = CliLayer {
            autorouter_enabled: Some(false),
            optimizer_enabled: Some(false),
            max_passes: Some(3),
            max_items: Some(100),
            max_threads: Some(1),
            optimizer_threads: Some(2),
            plane_island_clamp: Some(false),
            congestion_global: Some(false),
            congestion_global_pattern: Some(false),
            vias_allowed: Some(false),
            via_costs: Some(11),
            plane_via_costs: Some(12),
            start_ripup_costs: Some(13),
            automatic_neckdown: Some(false),
            trace_pull_tight_accuracy: Some(111),
            strict_drc: Some(false),
            improvement_threshold: Some(1.5),
            fanout_enabled: Some(false),
            optimizer_max_passes: Some(10),
            optimizer_max_items: Some(2000),
            congestion_global_pathfinder: Some(false),
            push_shove: Some(false),
            tuning: Some(false),
            tuning_meander: Some(false),
            tuning_pairs: Some(vec![("A".to_string(), "B".to_string())]),
            gloss_bus: Some(false),
            gloss_flow: Some(false),
            gloss_via_place: Some(false),
            gloss_teardrops: Some(false),
            ..CliLayer::default()
        };
        let mut merged = merge(&MergedSettings::default(), &DsnLayer::default(), &cli);
        let session = SessionLayer {
            autorouter_enabled: Some(true),
            optimizer_enabled: Some(true),
            max_passes: Some(7),
            max_items: Some(200),
            max_threads: Some(4),
            optimizer_threads: Some(8),
            plane_island_clamp: Some(true),
            congestion_global: Some(true),
            congestion_global_pattern: Some(true),
            congestion_global_pathfinder: Some(true),
            push_shove: Some(true),
            tuning: Some(true),
            tuning_meander: Some(true),
            tuning_pairs: Some(vec![
                ("X".to_string(), "Y".to_string()),
                ("P".to_string(), "Q".to_string()),
            ]),
            gloss_bus: Some(true),
            gloss_flow: Some(true),
            gloss_via_place: Some(true),
            gloss_teardrops: Some(true),
            vias_allowed: Some(true),
            via_costs: Some(21),
            plane_via_costs: Some(22),
            start_ripup_costs: Some(23),
            automatic_neckdown: Some(true),
            trace_pull_tight_accuracy: Some(222),
            strict_drc: Some(true),
            improvement_threshold: Some(9.5),
            fanout_enabled: Some(true),
            optimizer_max_passes: Some(20),
            optimizer_max_items: Some(3000),
        };

        merge_session(&mut merged, &session);

        assert_eq!(merged.autorouter_enabled, Some(true));
        assert_eq!(merged.optimizer_enabled, Some(true));
        assert_eq!(merged.max_passes, Some(7));
        assert_eq!(merged.max_items, Some(200));
        assert_eq!(merged.max_threads, Some(4));
        assert_eq!(merged.optimizer_threads, Some(8));
        assert_eq!(merged.plane_island_clamp, Some(true));
        assert_eq!(merged.congestion_global, Some(true));
        assert_eq!(merged.congestion_global_pattern, Some(true));
        assert_eq!(merged.congestion_global_pathfinder, Some(true));
        assert_eq!(merged.push_shove, Some(true));
        assert_eq!(merged.tuning, Some(true));
        assert_eq!(merged.tuning_meander, Some(true));
        assert_eq!(
            merged.tuning_pairs,
            Some(vec![
                ("X".to_string(), "Y".to_string()),
                ("P".to_string(), "Q".to_string())
            ])
        );
        assert_eq!(merged.gloss_bus, Some(true));
        assert_eq!(merged.gloss_flow, Some(true));
        assert_eq!(merged.gloss_via_place, Some(true));
        assert_eq!(merged.gloss_teardrops, Some(true));
        // The M9-T2 re-audit additions: session wins on every new
        // slot, including the scoring-box trio (the same slots the
        // CLI merge arm targets).
        assert_eq!(merged.vias_allowed, Some(true));
        assert_eq!(merged.scoring.via_costs, Some(21));
        assert_eq!(merged.scoring.plane_via_costs, Some(22));
        assert_eq!(merged.scoring.start_ripup_costs, Some(23));
        assert_eq!(merged.automatic_neckdown, Some(true));
        assert_eq!(merged.trace_pull_tight_accuracy, Some(222));
        assert_eq!(merged.strict_drc, Some(true));
        assert_eq!(merged.improvement_threshold, Some(9.5));
        assert_eq!(merged.fanout_enabled, Some(true));
        assert_eq!(merged.optimizer_max_passes, Some(20));
        assert_eq!(merged.optimizer_max_items, Some(3000));
    }

    /// PIN SL-3: `None` session fields NEVER overwrite — every CLI
    /// value survives a session layer that has no opinion on the slot
    /// (copy-if-`Some`, never nulls, never invents).
    #[test]
    fn session_layer_none_fields_never_overwrite_cli_values() {
        let cli = CliLayer {
            autorouter_enabled: Some(false),
            optimizer_enabled: Some(false),
            max_passes: Some(3),
            max_items: Some(100),
            max_threads: Some(1),
            optimizer_threads: Some(2),
            plane_island_clamp: Some(true),
            congestion_global: Some(true),
            congestion_global_pattern: Some(true),
            congestion_global_pathfinder: Some(true),
            push_shove: Some(true),
            tuning: Some(true),
            tuning_meander: Some(true),
            tuning_pairs: Some(vec![("A".to_string(), "B".to_string())]),
            gloss_bus: Some(true),
            gloss_flow: Some(true),
            gloss_via_place: Some(true),
            gloss_teardrops: Some(true),
            ..CliLayer::default()
        };
        let mut merged = merge(&MergedSettings::default(), &DsnLayer::default(), &cli);
        let before = merged.clone();

        merge_session(&mut merged, &SessionLayer::default());

        assert_eq!(merged, before, "silence from the session layer is a no-op");

        // One opinionated field moves ONLY that field.
        let mut merged = merge(&MergedSettings::default(), &DsnLayer::default(), &cli);
        let session = SessionLayer {
            tuning: Some(false),
            ..SessionLayer::default()
        };
        merge_session(&mut merged, &session);
        assert_eq!(merged.tuning, Some(false), "only the voiced slot moves");
        assert_eq!(
            merged.max_passes,
            Some(3),
            "unvoiced slots keep the CLI values"
        );
        assert_eq!(
            merged.gloss_bus,
            Some(true),
            "unvoiced slots keep the CLI values"
        );
    }
}
