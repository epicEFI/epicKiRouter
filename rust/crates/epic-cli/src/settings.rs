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
use epic_router::pipeline::board_statistics::{
    RouterScoreSettings, RouterScoringVersion, RouterSettingsScoring, RoutingCostSettings,
    default_routing_cost_settings,
};

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
    /// `router.max_threads` / `-mt` — parsed but unused: the T13 engine
    /// is single-threaded by construction (SEAM).
    pub max_threads: Option<i32>,
    /// `router.optimizer.improvement_threshold` / `-oit` — parsed and
    /// normalized (the x100 quirk) but unused: no optimizer stage in T13.
    pub improvement_threshold: Option<f32>,
    /// `router.scoring.version` (dual-applied from `-scoring-version`).
    pub router_scoring_version: Option<RouterScoringVersion>,
    /// `optimizer.scoring.version` — parsed (with the v2 -> V2_LOWER_BOUND
    /// asymmetry) but unused: no optimizer stage in T13.
    pub optimizer_scoring_version: Option<OptimScoringVersion>,
    /// `router.vias_allowed`.
    pub vias_allowed: Option<bool>,
    /// `router.via_costs`.
    pub via_costs: Option<i32>,
    /// `router.plane_via_costs`.
    pub plane_via_costs: Option<i32>,
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
    /// `maxThreads` — parsed but unused (single-threaded engine).
    pub max_threads: Option<i32>,
    /// `viasAllowed`.
    pub vias_allowed: Option<bool>,
    /// `automaticNeckdown` — DefaultSettings TRUE (the bare-ctor default
    /// is false; the T13 flow always merges over DefaultSettings).
    pub automatic_neckdown: Option<bool>,
    /// `tracePullTightAccuracy`.
    pub trace_pull_tight_accuracy: Option<i32>,
    /// `strictDrc`.
    pub strict_drc: Option<bool>,
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
    /// `optimizer.enabled` — merged but unused in T13 (no optimizer stage).
    pub optimizer_enabled: Option<bool>,
    /// `optimizer.improvementThreshold` — merged but unused in T13.
    pub improvement_threshold: Option<f32>,
    /// The `scoring` box (deep-merged; defaults from DefaultSettings).
    pub scoring: RoutingCostSettings,
    /// The `routerScoring` box.
    pub router_scoring: RouterScoreSettings,
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
            vias_allowed: Some(true),
            automatic_neckdown: Some(true),
            trace_pull_tight_accuracy: Some(500),
            strict_drc: Some(false),
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
            improvement_threshold: Some(2.5),
            scoring: default_routing_cost_settings(),
            router_scoring: default_router_score_settings(),
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
    if let Some(v) = cli.improvement_threshold {
        merged.improvement_threshold = Some(v);
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
    /// `tracePullTightAccuracy`.
    pub pull_tight_accuracy: i32,
    /// `--deterministic-budgets` (default ON — anchors §6).
    pub deterministic_budgets: bool,
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
            },
            max_passes: merged.max_passes,
            max_items: merged.max_items,
            fanout_enabled: merged.fanout_enabled.unwrap_or(true),
            strict_drc: merged.strict_drc.unwrap_or(false),
            run_router: merged.autorouter_enabled.unwrap_or(true),
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
            improvement_threshold: Some(7.5),
            router_scoring_version: Some(RouterScoringVersion::V1Legacy),
            optimizer_scoring_version: Some(OptimScoringVersion::V1Legacy),
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
        // optimizer_scoring_version deliberately has NO merge arm
        // (parsed and banked for the optimizer milestone — module docs).

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
}
