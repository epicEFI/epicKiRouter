//! Port of `io.specctra.parser.AutorouteSettings` (`AutorouteSettings.java`):
//! the `(autoroute_settings ...)` scope reader — D15 raw IR.
//!
//! D15: the parse result is a FIELD-FOR-FIELD IR ([`AutorouteSettingsIr`]),
//! NOT the Java `RouterSettings` object — the settings merger that consumes
//! it is M3 work. The IR materializes the OBSERVABLE getter values of the
//! Java object the jar builds (`RouterSettings` + `LayerSettings`, both
//! nullable-field holders with defaulted getters):
//!
//! - `new AutorouteSettingsIr(layer_count)` mirrors
//!   `new RouterSettings()` + `setLayerCount(n)` (jar
//!   `/tmp/epic-t9-autosettings.out` DEFAULTS block: runRouter=true,
//!   runOptimizer=FALSE (constructor default; readScope overwrites both
//!   after the loop), viasAllowed=true, costs 1, per-layer active=true,
//!   horizontal=`i % 2 == 1`, trace costs 1.0).
//! - the three cost setters clamp `max(value, 1)` (jar: plane_via_costs 0
//!   -> 1, start_ripup_costs -5 -> 1) and the per-layer trace-cost setters
//!   clamp `max(value, 0.1)` (jar `/tmp/epic-t9-ar3.out`: 0.05 -> 0.1,
//!   0 -> 0.1).
//!
//! `(fanout ...)` is READ AND DISCARDED (`AutorouteSettings.java:44-45`
//! calls `readOnOffScope` and ignores the result — `fanoutEnabled` keeps
//! its constructor default false; jar CASE autosettings `fanoutEnabled=
//! false` despite `(fanout off)`).
//!
//! A failed `layer_rule` (unknown layer name — jar
//! `/tmp/epic-t9-ar2.out` CASE arbadlayer; bad `preferred_direction`
//! keyword — CASE arbaddir; also a non-String layer name or a missing
//! bracket) makes the WHOLE read return None, which leaves
//! `state.autoroute_settings` unset and later arms the
//! `adjustPlaneAutorouteSettings` fallback (Task 6 port, already in
//! `ses_board.rs`). EOF/IO likewise.

use crate::keyword::{Keyword, skip_scope};
use crate::layer_structure::LayerStructure;
use crate::lexer::{LexicalState, Scanner, Token};
use crate::scope::structure::{read_float_scope, read_integer_scope, read_on_off_scope};

/// One layer's rule slice: the observable `RouterSettings` layer getters
/// (`getLayerActive`, `getPreferredDirectionIsHorizontal`,
/// `getPreferredDirectionTraceCosts`, `getAgainstPreferredDirectionTraceCosts`).
///
/// The `*_set` flags are the T13 settings-merger face (M3-T13): the Java
/// parse (`AutorouteSettings.readLayerRule`) calls the setters ONLY for
/// the layer_rule sub-scopes that appear, so the extracted
/// `RouterSettings` layers carry NULL elsewhere — and
/// `ReflectionUtil.copyFields` copies a field only when the source value
/// is non-null. The IR materializes the getter defaults (D15); the flags
/// restore the set-vs-inherited distinction the layered resolver needs.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerRuleIr {
    /// `getLayerActive` — default true (`RouterSettings.java:664-668`).
    pub active: bool,
    /// `(active on|off)` appeared — `setLayerActive` was called.
    pub active_set: bool,
    /// `getPreferredDirectionIsHorizontal` — default `i % 2 == 1`
    /// (`:747-750`).
    pub preferred_direction_is_horizontal: bool,
    /// `(preferred_direction ...)` appeared.
    pub preferred_direction_is_horizontal_set: bool,
    /// `getPreferredDirectionTraceCosts` — default 1.0, clamped
    /// `max(v, 0.1)` on set (`:778`, `:793-797`).
    pub preferred_direction_trace_costs: f64,
    /// `(preferred_direction_trace_costs ...)` appeared.
    pub preferred_direction_trace_costs_set: bool,
    /// `getAgainstPreferredDirectionTraceCosts` — default 1.0, clamped
    /// `max(v, 0.1)` on set.
    pub against_preferred_direction_trace_costs: f64,
    /// `(against_preferred_direction_trace_costs ...)` appeared.
    pub against_preferred_direction_trace_costs_set: bool,
}

impl LayerRuleIr {
    /// `new LayerSettings()` + the getter defaults — the jar DEFAULTS block.
    fn new(layer_index: i32) -> Self {
        Self {
            active: true,
            active_set: false,
            preferred_direction_is_horizontal: layer_index % 2 == 1,
            preferred_direction_is_horizontal_set: false,
            preferred_direction_trace_costs: 1.0,
            preferred_direction_trace_costs_set: false,
            against_preferred_direction_trace_costs: 1.0,
            against_preferred_direction_trace_costs_set: false,
        }
    }
}

/// The whole `(autoroute_settings ...)` read result (D15).
///
/// T13 set-flags (see [`LayerRuleIr`]): `readScope` calls
/// `setRunRouter`/`setRunOptimizer` post-loop UNCONDITIONALLY (even an
/// empty scope sets both), so those two carry no flag; the four scalars
/// are set only when their sub-scope appeared.
#[derive(Clone, Debug, PartialEq)]
pub struct AutorouteSettingsIr {
    /// `getRunRouter` — the post-loop `withAutoroute` (`:68`); ALWAYS set.
    pub run_router: bool,
    /// `getRunOptimizer` — the post-loop `withPostroute` (`:69`); ALWAYS set.
    pub run_optimizer: bool,
    /// `getViasAllowed` — default true.
    pub vias_allowed: bool,
    /// `(vias on|off)` appeared.
    pub vias_allowed_set: bool,
    /// `getViaCosts` — default 1, clamped `max(v, 1)` (`:613`).
    pub via_costs: i32,
    /// `(via_costs ...)` appeared.
    pub via_costs_set: bool,
    /// `getPlaneViaCosts` — default 1, clamped `max(v, 1)` (`:626`).
    pub plane_via_costs: i32,
    /// `(plane_via_costs ...)` appeared.
    pub plane_via_costs_set: bool,
    /// `getStartRipupCosts` — default 1, clamped `max(v, 1)` (`:549`).
    pub start_ripup_costs: i32,
    /// `(start_ripup_costs ...)` appeared.
    pub start_ripup_costs_set: bool,
    /// One entry per layer at read time (`setLayerCount(layerCount)`, `:20`).
    pub layer_rules: Vec<LayerRuleIr>,
}

impl AutorouteSettingsIr {
    /// `new RouterSettings()` + `setLayerCount(n)` — the constructor and
    /// per-layer getter defaults (`RouterSettings.java:456-464`,
    /// `LayerSettings()`).
    pub fn new(layer_count: usize) -> Self {
        Self {
            run_router: true,
            // the CONSTRUCTOR default; readScope overwrites it post-loop
            run_optimizer: false,
            vias_allowed: true,
            vias_allowed_set: false,
            via_costs: 1,
            via_costs_set: false,
            plane_via_costs: 1,
            plane_via_costs_set: false,
            start_ripup_costs: 1,
            start_ripup_costs_set: false,
            layer_rules: (0..layer_count as i32).map(LayerRuleIr::new).collect(),
        }
    }
}

/// Java `AutorouteSettings.readScope` (`:18-71`). `layer_structure` is the
/// parser's layer table (the structure reader creates it just before this
/// call — `Structure.java:1006-1011`, bug-compat: when the layer structure
/// already exists the scope is NOT read at all). `None` (Java null) on
/// EOF/IO or a failed layer_rule.
pub fn read_scope(
    scanner: &mut Scanner,
    layer_structure: &LayerStructure,
) -> Option<AutorouteSettingsIr> {
    let mut result = AutorouteSettingsIr::new(layer_structure.layers.len());
    let mut with_autoroute = true;
    let mut with_postroute = true;
    let mut prev_token: Option<Token> = None;
    loop {
        let next_token = scanner.next_token();
        // Java: IOException -> error log + null; null -> warn (log-only) + null.
        if matches!(next_token, Token::Eof | Token::Error(_)) {
            return None;
        }
        if next_token == Token::Close {
            break;
        }
        if prev_token == Some(Token::Open) {
            match next_token {
                Token::Keyword(Keyword::Fanout) => {
                    // Java `:44-45`: read AND DISCARD.
                    let _ = read_on_off_scope(scanner);
                }
                Token::Keyword(Keyword::Autoroute) => {
                    with_autoroute = read_on_off_scope(scanner);
                }
                Token::Keyword(Keyword::Postroute) => {
                    with_postroute = read_on_off_scope(scanner);
                }
                Token::Keyword(Keyword::Vias) => {
                    result.vias_allowed = read_on_off_scope(scanner);
                    result.vias_allowed_set = true;
                }
                Token::Keyword(Keyword::ViaCosts) => {
                    let value = read_integer_scope(scanner);
                    result.via_costs = value.max(1);
                    result.via_costs_set = true;
                }
                Token::Keyword(Keyword::PlaneViaCosts) => {
                    let value = read_integer_scope(scanner);
                    result.plane_via_costs = value.max(1);
                    result.plane_via_costs_set = true;
                }
                Token::Keyword(Keyword::StartRipupCosts) => {
                    let value = read_integer_scope(scanner);
                    result.start_ripup_costs = value.max(1);
                    result.start_ripup_costs_set = true;
                }
                Token::Keyword(Keyword::LayerRule) => {
                    read_layer_rule(scanner, layer_structure, &mut result)?;
                }
                _ => {
                    // Java `:63-65`: unknown arm — generic skip; the
                    // skipScope return value is discarded.
                    let _ = skip_scope(scanner);
                }
            }
        }
        prev_token = Some(next_token);
    }
    result.run_router = with_autoroute;
    result.run_optimizer = with_postroute;
    Some(result)
}

/// Java `AutorouteSettings.readLayerRule` (`:73-157`). Returns false
/// (Java null) on a bad layer name, a bad `preferred_direction` keyword,
/// a missing bracket, or EOF — the caller then abandons the WHOLE settings
/// read.
fn read_layer_rule(
    scanner: &mut Scanner,
    layer_structure: &LayerStructure,
    settings: &mut AutorouteSettingsIr,
) -> Option<()> {
    // Java `:75`: the layer NAME is NAME-forced (a layer called e.g. `via`
    // still reads as a String).
    scanner.set_lexical_state(LexicalState::Name);
    let layer_name = match scanner.next_token() {
        Token::Str(name) => name.to_string(),
        // Java: warn "AutorouteSettings.read_layer_rule: String expected at
        // '<id>'" (log-only).
        _ => return None,
    };
    let layer_index = layer_structure.get_no(&layer_name);
    if layer_index < 0 {
        // Java: warn "AutorouteSettings.read_layer_rule: layer not found at
        // '<id>'" (log-only; jar arbadlayer).
        return None;
    }
    let layer_index = layer_index as usize;
    let mut prev_token: Option<Token> = None;
    loop {
        let next_token = scanner.next_token();
        if matches!(next_token, Token::Eof | Token::Error(_)) {
            return None;
        }
        if next_token == Token::Close {
            break;
        }
        if prev_token == Some(Token::Open) {
            match next_token {
                Token::Keyword(Keyword::Active) => {
                    let rule = &mut settings.layer_rules[layer_index];
                    rule.active = read_on_off_scope(scanner);
                    rule.active_set = true;
                }
                Token::Keyword(Keyword::PreferredDirection) => {
                    // Java `:120-141`: VERTICAL/HORIZONTAL cascade; an
                    // unexpected keyword fails the WHOLE read (jar
                    // arbaddir). The closing bracket is consumed HERE — a
                    // missing one also fails the read.
                    let mut pref_dir_is_horizontal = true;
                    match scanner.next_token() {
                        Token::Keyword(Keyword::Vertical) => pref_dir_is_horizontal = false,
                        Token::Keyword(Keyword::Horizontal) => {}
                        // Java: warn "AutorouteSettings.read_layer_rule:
                        // unexpected key word at '<id>'" (log-only).
                        _ => return None,
                    }
                    let rule = &mut settings.layer_rules[layer_index];
                    rule.preferred_direction_is_horizontal = pref_dir_is_horizontal;
                    rule.preferred_direction_is_horizontal_set = true;
                    if scanner.next_token() != Token::Close {
                        // Java: warn "... closing bracket expected at
                        // '<id>'" (log-only).
                        return None;
                    }
                }
                Token::Keyword(Keyword::PreferredDirectionTraceCosts) => {
                    let value = read_float_scope(scanner);
                    let rule = &mut settings.layer_rules[layer_index];
                    rule.preferred_direction_trace_costs = value.max(0.1);
                    rule.preferred_direction_trace_costs_set = true;
                }
                Token::Keyword(Keyword::AgainstPreferredDirectionTraceCosts) => {
                    let value = read_float_scope(scanner);
                    let rule = &mut settings.layer_rules[layer_index];
                    rule.against_preferred_direction_trace_costs = value.max(0.1);
                    rule.against_preferred_direction_trace_costs_set = true;
                }
                _ => {
                    let _ = skip_scope(scanner);
                }
            }
        }
        prev_token = Some(next_token);
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer_structure::Layer;

    /// The 4-layer table of the jar fixtures (F.Cu, In1.Cu, In2.Cu, B.Cu).
    fn four_layers() -> LayerStructure {
        LayerStructure::new(vec![
            Layer::new("F.Cu", 0, true),
            Layer::new("In1.Cu", 1, true),
            Layer::new("In2.Cu", 2, true),
            Layer::new("B.Cu", 3, true),
        ])
    }

    /// Drives `read_scope` positioned after the `autoroute_settings`
    /// keyword, mirroring the jar probe sessions
    /// (`Keyword.PCB_SCOPE.readScope(p)` reaching the structure arm).
    fn run_settings(body: &str) -> Option<AutorouteSettingsIr> {
        let input = format!("(autoroute_settings {body})");
        let mut scanner = Scanner::new(input.as_bytes());
        assert_eq!(scanner.next_token(), Token::Open);
        assert_eq!(
            scanner.next_token(),
            Token::Keyword(Keyword::AutorouteSettings)
        );
        read_scope(&mut scanner, &four_layers())
    }

    /// Jar `/tmp/epic-t9-autosettings.out` DEFAULTS block:
    /// `new RouterSettings()` + `setLayerCount(4)` — runRouter=true,
    /// runOptimizer=false, viasAllowed=true, all costs 1, per-layer
    /// active=true, horizontal=i%2==1 (L0 false, L1 true, L2 false, L3
    /// true), trace costs 1.0.
    #[test]
    fn constructor_defaults_match_jar() {
        let settings = AutorouteSettingsIr::new(4);
        assert!(settings.run_router);
        assert!(!settings.run_optimizer);
        assert!(settings.vias_allowed);
        assert_eq!(settings.via_costs, 1);
        assert_eq!(settings.plane_via_costs, 1);
        assert_eq!(settings.start_ripup_costs, 1);
        let expected_horizontal = [false, true, false, true];
        for (index, rule) in settings.layer_rules.iter().enumerate() {
            assert!(rule.active, "L{index} active default");
            assert_eq!(
                rule.preferred_direction_is_horizontal, expected_horizontal[index],
                "L{index} horizontal default"
            );
            assert_eq!(rule.preferred_direction_trace_costs, 1.0);
            assert_eq!(rule.against_preferred_direction_trace_costs, 1.0);
        }
    }

    /// Jar `/tmp/epic-t9-autosettings.out` CASE autosettings
    /// (`/tmp/epic-t9-autosettings.dsn`): the full battery. `(fanout off)`
    /// is read and DISCARDED; `(postroute off)` lands in run_optimizer;
    /// `(vias off)`; `(via_costs 7)`; the clamps are VISIBLE
    /// (`plane_via_costs 0` -> 1, `start_ripup_costs -5` -> 1);
    /// F.Cu horizontal=true; In1.Cu active=false + vertical + 2.5/0.5;
    /// untouched L2/L3 keep the i%2==1 defaults.
    #[test]
    fn full_battery_jar_probe() {
        let settings = run_settings(
            "(fanout off)\
             (autoroute on)\
             (postroute off)\
             (vias off)\
             (via_costs 7)\
             (plane_via_costs 0)\
             (start_ripup_costs -5)\
             (layer_rule F.Cu\
               (active on)\
               (preferred_direction horizontal)\
             )\
             (layer_rule In1.Cu\
               (active off)\
               (preferred_direction vertical)\
               (preferred_direction_trace_costs 2.5)\
               (against_preferred_direction_trace_costs 0.5)\
             )",
        )
        .expect("settings read");
        assert!(settings.run_router);
        assert!(!settings.run_optimizer);
        assert!(!settings.vias_allowed);
        assert_eq!(settings.via_costs, 7);
        assert_eq!(settings.plane_via_costs, 1);
        assert_eq!(settings.start_ripup_costs, 1);
        assert!(settings.layer_rules[0].active);
        assert!(settings.layer_rules[0].preferred_direction_is_horizontal);
        assert!(!settings.layer_rules[1].active);
        assert!(!settings.layer_rules[1].preferred_direction_is_horizontal);
        assert_eq!(settings.layer_rules[1].preferred_direction_trace_costs, 2.5);
        assert_eq!(
            settings.layer_rules[1].against_preferred_direction_trace_costs,
            0.5
        );
        // untouched layers keep the constructor defaults
        assert!(settings.layer_rules[2].active);
        assert!(!settings.layer_rules[2].preferred_direction_is_horizontal);
        assert!(settings.layer_rules[3].preferred_direction_is_horizontal);
        assert_eq!(settings.layer_rules[3].preferred_direction_trace_costs, 1.0);
    }

    /// Jar `/tmp/epic-t9-ar2.out` CASE arlate (`/tmp/epic-t9-arlate.dsn`):
    /// the layer NAME resolves through `LayerStructure.getNo`
    /// (B.Cu -> index 3), later scopes in the same body still apply
    /// (`(via_costs 3)` after the layer_rule), and untouched layers keep
    /// the defaults.
    #[test]
    fn layer_name_resolves_to_index() {
        let settings = run_settings(
            "(layer_rule B.Cu (active off) (preferred_direction vertical))\
             (via_costs 3)",
        )
        .expect("settings read");
        assert_eq!(settings.via_costs, 3);
        assert!(!settings.layer_rules[3].active);
        assert!(!settings.layer_rules[3].preferred_direction_is_horizontal);
        assert!(settings.layer_rules[0].active);
        assert!(!settings.layer_rules[0].preferred_direction_is_horizontal);
    }

    /// Jar `/tmp/epic-t9-ar3.out` (`/tmp/epic-t9-arclamp.dsn`): the
    /// per-layer trace-cost setters clamp `max(v, 0.1)` — 0.05 and 0 both
    /// store 0.1 (observable through the getters).
    #[test]
    fn trace_cost_clamps_at_one_tenth() {
        let settings = run_settings(
            "(layer_rule F.Cu\
               (preferred_direction_trace_costs 0.05)\
               (against_preferred_direction_trace_costs 0)\
             )",
        )
        .expect("settings read");
        assert_eq!(settings.layer_rules[0].preferred_direction_trace_costs, 0.1);
        assert_eq!(
            settings.layer_rules[0].against_preferred_direction_trace_costs,
            0.1
        );
    }

    /// Jar `/tmp/epic-t9-ar2.out` CASE arbadlayer
    /// (`/tmp/epic-t9-arbadlayer.dsn`): an unknown layer name fails the
    /// WHOLE read (None -> the parse state keeps autoroute_settings unset
    /// and the adjustPlaneAutorouteSettings fallback applies later).
    #[test]
    fn unknown_layer_name_fails_whole_read() {
        let settings = run_settings("(layer_rule Nope.Cu (active off))");
        assert!(settings.is_none());
    }

    /// Jar `/tmp/epic-t9-ar2.out` CASE arbaddir
    /// (`/tmp/epic-t9-arbaddir.dsn`): an unexpected keyword inside
    /// `(preferred_direction ...)` fails the WHOLE read.
    #[test]
    fn bad_direction_keyword_fails_whole_read() {
        let settings = run_settings("(layer_rule B.Cu (preferred_direction diagonal))");
        assert!(settings.is_none());
    }

    /// Java `:21-23` + `:68-69`: an EMPTY scope leaves withAutoroute/
    /// withPostroute at their local defaults — both true AFTER the loop
    /// (overriding the runOptimizer constructor default). D15: the IR
    /// stores the POST-LOOP values.
    #[test]
    fn empty_scope_defaults_both_flags_true() {
        let settings = run_settings("").expect("settings read");
        assert!(settings.run_router);
        assert!(settings.run_optimizer);
    }

    /// T13 (M3-T13): the explicit-set flags mirror Java's null-vs-set
    /// surface in the extracted `RouterSettings` — the post-loop
    /// `setRunRouter`/`setRunOptimizer` fire ALWAYS; every scalar and
    /// per-layer field is set ONLY when its sub-scope appeared. The
    /// materialized values keep their getter defaults either way.
    #[test]
    fn explicit_set_flags_track_the_scopes() {
        let settings = run_settings(
            "(autoroute on)\
             (postroute off)\
             (via_costs 3)\
             (layer_rule In1.Cu (preferred_direction vertical))",
        )
        .expect("settings read");
        // ALWAYS set (post-loop assignment, empty scope included).
        assert!(settings.run_router);
        assert!(!settings.run_optimizer);
        // Sub-scope-gated: via_costs yes, the rest no.
        assert!(!settings.vias_allowed_set);
        assert!(settings.via_costs_set);
        assert_eq!(settings.via_costs, 3);
        assert!(!settings.plane_via_costs_set);
        assert!(!settings.start_ripup_costs_set);
        assert_eq!(settings.plane_via_costs, 1);
        // In1.Cu = layer 1: only the direction sub-scope appeared.
        let in1 = &settings.layer_rules[1];
        assert!(!in1.active_set);
        assert!(in1.active);
        assert!(in1.preferred_direction_is_horizontal_set);
        assert!(!in1.preferred_direction_is_horizontal);
        assert!(!in1.preferred_direction_trace_costs_set);
        assert!(!in1.against_preferred_direction_trace_costs_set);
        // Untouched layer: nothing set.
        let untouched = &settings.layer_rules[0];
        assert!(!untouched.active_set);
        assert!(!untouched.preferred_direction_is_horizontal_set);
        assert!(!untouched.preferred_direction_trace_costs_set);
        assert!(!untouched.against_preferred_direction_trace_costs_set);
    }
}
