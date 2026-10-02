//! The F2 current-driven trace-width core: the IPC-2221B closed-form
//! conductor-sizing model, self-contained and pure. Tyler's ask
//! (2026-10-02): "there needs to be some way to say yo this trace
//! needs to handle like 10a" — a declared current per net must become
//! a MINIMUM trace width before routing starts.
//!
//! Model: IPC-2221B section 6.2 (the classic design-chart regression,
//! the same formula KiCad's trace-width calculator carries):
//!
//! ```text
//! A [mils^2] = ( I / (k * dT^0.44) )^(1/0.725)
//!   k = 0.048 external layers, 0.024 internal layers
//! W [mils]   = A / T,   T = copper thickness [mils] (1.37 mils/oz)
//! ```
//!
//! IPC-2152 (2009) supersedes the chart with derating curves but is
//! nomograph/table-based; IPC-2221 remains the standard CLOSED form
//! and errs conservative on internal layers (k halves — the formula
//! charges buried traces 2^(1/0.725) ~= 2.61x the cross-section).
//!
//! Module law (the pin_assign.rs idiom): the pure faces live here
//! ungated; the board-mutating integration (synthetic net-class
//! synthesis) composes these in the engine/session faces.
//!
//! Rounding law: a MINIMUM width must never round DOWN. The full
//! width in board units is CEILED, and the half width is the ceil of
//! HALF THE CEILED full width (an odd full width rounds its half UP,
//! e.g. full 5 -> half 3), so the routed trace is always >= the
//! computed minimum.

/// IPC-2221B external-layer coefficient `k`.
pub const EXTERNAL_K: f64 = 0.048;
/// IPC-2221B internal-layer coefficient `k` (half the external value).
pub const INTERNAL_K: f64 = 0.024;
/// IPC-2221B temperature-rise exponent (`dT^b`, `b = 0.44`).
pub const TEMP_EXP: f64 = 0.44;
/// IPC-2221B current exponent (`I^(1/c)`, `c = 0.725`).
pub const AREA_EXP: f64 = 0.725;
/// Copper foil thickness per ounce (mils; 1 oz/ft^2 ~= 35 um ~= 1.37 mils).
pub const MILS_PER_OZ: f64 = 1.37;

/// One net's current requirement plus the two board-level sizing
/// inputs (all strictly positive — [`CurrentWidthSpec::validate`] is
/// the parse-side gate).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CurrentWidthSpec {
    /// The current the trace must carry (A).
    pub amps: f64,
    /// Copper weight (oz); 1.0 is the common default.
    pub copper_oz: f64,
    /// Allowed temperature rise above ambient (deg C); 10 is the
    /// common conservative default.
    pub temp_rise_c: f64,
}

impl CurrentWidthSpec {
    /// The parse/apply-side validation: every field strictly positive
    /// (zero or negative amps/copper/rise are outside the IPC-2221
    /// chart domain and would poison the powers). The error strings
    /// name the field for the settings warning face.
    pub fn validate(&self) -> Result<(), String> {
        if self.amps <= 0.0 {
            return Err(format!("current must be > 0 A, got {}", self.amps));
        }
        if self.copper_oz <= 0.0 {
            return Err(format!(
                "copper weight must be > 0 oz, got {}",
                self.copper_oz
            ));
        }
        if self.temp_rise_c <= 0.0 {
            return Err(format!(
                "temperature rise must be > 0 deg C, got {}",
                self.temp_rise_c
            ));
        }
        Ok(())
    }
}

/// IPC-2221B minimum cross-sectional area (mils^2) for `amps` at
/// `temp_rise_c`, external (`k = 0.048`) or internal (`k = 0.024`).
/// Inputs are the caller's validated positives (the powers are total
/// there; this module deliberately has no NaN/inf handling).
#[must_use]
pub fn ipc2221_area_mils2(amps: f64, temp_rise_c: f64, external: bool) -> f64 {
    let k = if external { EXTERNAL_K } else { INTERNAL_K };
    (amps / (k * temp_rise_c.powf(TEMP_EXP))).powf(1.0 / AREA_EXP)
}

/// IPC-2221B minimum trace WIDTH (mils) for the whole spec — the area
/// over the copper thickness (`copper_oz` foil mils).
#[must_use]
pub fn ipc2221_width_mils(spec: &CurrentWidthSpec, external: bool) -> f64 {
    ipc2221_area_mils2(spec.amps, spec.temp_rise_c, external) / (spec.copper_oz * MILS_PER_OZ)
}

/// A minimum width in mils as a FULL width in board units — CEILED,
/// never rounded down (`units_per_mil` is the board's
/// `resolution_mil()`: board units per mil, e.g. 254 for
/// `(resolution um 10)`).
#[must_use]
pub fn full_width_dbu(width_mils: f64, units_per_mil: f64) -> i32 {
    (width_mils * units_per_mil).ceil() as i32
}

/// The HALF width the rules tables carry ([`epic_board::rules_surf::
/// NetClass::trace_half_widths`]) — ceil of half the CEILED full
/// width, so an odd full width rounds its half UP (full 5 -> half 3)
/// and the routed trace stays >= the minimum.
#[must_use]
pub fn half_width_dbu(width_mils: f64, units_per_mil: f64) -> i32 {
    let full = full_width_dbu(width_mils, units_per_mil);
    (f64::from(full) / 2.0).ceil() as i32
}

// ===========================================================================
// The integration half (F2): the board-mutating apply face.
// ===========================================================================

use epic_board::board::Board;

/// One requested current-driven widening: net NAME + amps (the copper
/// weight and temperature rise are board-level inputs shared by every
/// request — the settings layer owns their defaults, 1 oz / 10 deg C).
#[derive(Debug, Clone, PartialEq)]
pub struct CurrentNetRequest {
    /// The net NAME as declared in the netlist (resolved
    /// CASE-INSENSITIVELY, first match — parsed nets may carry any
    /// subnet number).
    pub net: String,
    /// The current the net's traces must carry (A, strictly positive).
    pub amps: f64,
}

/// One net's performed widening, for the report (the CLI manifest rows
/// and the session telemetry render these verbatim).
#[derive(Debug, Clone, PartialEq)]
pub struct CurrentWidthRow {
    /// The widened net's 1-based number.
    pub net_number: i32,
    /// The widened net's name.
    pub net_name: String,
    /// The requested current (A).
    pub amps: f64,
    /// The class the net LEFT (name).
    pub old_class: String,
    /// The synthetic class the net now rides (name).
    pub new_class: String,
    /// The WIDENED slots only — `(layer, old half width, new half
    /// width)` in board units; a layer whose minimum did not rise is
    /// omitted.
    pub layers: Vec<(i32, i32, i32)>,
}

/// The apply face's answer: the performed widenings, the refs that
/// could not be honored (unknown net, invalid current — never fatal,
/// never guessed), and the applied-but-noted warnings.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CurrentWidthReport {
    /// The performed widenings, in request order.
    pub rows: Vec<CurrentWidthRow>,
    /// Unhonored requests with reasons.
    pub unresolved: Vec<String>,
    /// Applied-but-noted faces (e.g. the computed width exceeds the
    /// board's declared max trace half width — the current requirement
    /// still wins, the parse-time max is advisory).
    pub warnings: Vec<String>,
}

/// F2: for each requested net, move the net onto a SYNTHETIC net class
/// cloned from its current one with per-layer trace half widths
/// widened to the IPC-2221 minimum for the declared current — Tyler's
/// ask: "yo this trace needs to handle like 10a".
///
/// Mechanics (why a synthetic class): every width consumer resolves
/// through the net's class (`BoardRules::trace_half_width` ->
/// `NetClass::trace_half_widths`), so APPENDING a cloned-and-widened
/// class and repointing only the named net's `net_class` index widens
/// that one net with ZERO read-path changes — the class table is
/// position-keyed (append-safe) and sibling nets sharing the old class
/// are untouched. The synthetic name (`{old}~{amps}A`) keeps the SES
/// output self-describing.
///
/// Semantics:
///
/// * `copper_oz` / `temp_rise_c` are validated WITH each request's
///   amps ([`CurrentWidthSpec::validate`]); a rejection lands in
///   `unresolved` naming the net.
/// * The layer split: the FIRST and LAST board layers are external
///   (`k = 0.048`), everything between is internal (`k = 0.024` — the
///   standard stackup heuristic; a 2-layer board has both external).
/// * ONLY-WIDEN floor: per layer the new half width is
///   `max(old, computed)` — a net already wide enough for the declared
///   current keeps its widths and gets NO row. A slot missing from the
///   old class (out-of-range read = 0) materializes at the computed
///   minimum.
/// * The minimum never rounds down ([`half_width_dbu`]'s law).
/// * The caller runs this BEFORE any routing pass (the route head /
///   CLI pre-route stage, after pin assignment); routing then uses the
///   widened widths for the maze, fanout, and optimizer alike.
pub fn apply_current_widths(
    board: &mut Board,
    requests: &[CurrentNetRequest],
    copper_oz: f64,
    temp_rise_c: f64,
) -> CurrentWidthReport {
    let mut report = CurrentWidthReport::default();
    let units_per_mil = board.communication().resolution_mil();
    let layer_count = board.layers().layers.len() as i32;
    let last_layer = layer_count - 1;
    for request in requests {
        let spec = CurrentWidthSpec {
            amps: request.amps,
            copper_oz,
            temp_rise_c,
        };
        if let Err(err) = spec.validate() {
            report.unresolved.push(format!("{}: {err}", request.net));
            continue;
        }
        let Some((net_number, net)) = board
            .rules()
            .nets
            .iter()
            .find(|(_, net)| net.name.to_lowercase() == request.net.to_lowercase())
        else {
            report
                .unresolved
                .push(format!("{}: no such net", request.net));
            continue;
        };
        let net_name = net.name.clone(); // the CANONICAL name (the
        // request may carry any case; reports and warnings name the
        // board's own spelling).
        let old_class_index = net.net_class;
        let Some(old_class_name) = board
            .rules()
            .net_classes
            .get(usize::try_from(old_class_index).ok().unwrap_or(usize::MAX))
            .map(|class| class.name.clone())
        else {
            report.unresolved.push(format!(
                "{}: net class {old_class_index} missing",
                request.net
            ));
            continue;
        };
        let mut new_class = board.rules().net_classes
            [usize::try_from(old_class_index).expect("guarded above")]
        .clone();
        new_class.name = format!("{old_class_name}~{}A", request.amps);
        let new_class_name = new_class.name.clone();
        let mut widened: Vec<(i32, i32, i32)> = Vec::new();
        for layer in 0..layer_count {
            let internal = layer != 0 && layer != last_layer;
            let minimum = half_width_dbu(ipc2221_width_mils(&spec, !internal), units_per_mil);
            let slot = usize::try_from(layer).ok().unwrap_or(usize::MAX);
            let old = new_class.trace_half_widths.get(slot).copied().unwrap_or(0);
            let new_width = old.max(minimum);
            if slot < new_class.trace_half_widths.len() {
                new_class.trace_half_widths[slot] = new_width;
            } else {
                // A missing slot materializes (the out-of-range read
                // is 0, so the floor is the computed minimum).
                new_class.trace_half_widths.resize(slot + 1, 0);
                new_class.trace_half_widths[slot] = new_width;
            }
            if new_width > old {
                widened.push((layer, old, new_width));
            }
        }
        if widened.is_empty() {
            continue; // already wide enough — no row, no class churn
        }
        let max_half = board.rules().max_trace_half_width;
        if max_half > 0
            && widened
                .iter()
                .any(|&(_, _, new_width)| new_width > max_half)
        {
            report.warnings.push(format!(
                "{}: width {} exceeds the board's max trace half width {max_half} (applied — the current requirement wins)",
                net_name,
                widened.iter().map(|&(_, _, w)| w).max().expect("widened is non-empty")
            ));
        }
        let index = board.rules().net_classes.len() as i32;
        board.rules_mut().net_classes.push(new_class);
        board.rules_mut().nets.set_net_class(net_number, index);
        report.rows.push(CurrentWidthRow {
            net_number,
            net_name,
            amps: request.amps,
            old_class: old_class_name,
            new_class: new_class_name,
            layers: widened,
        });
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    /// THE CHART ANCHOR, external 1A / 1oz / 10 deg C: the IPC-2221B
    /// design chart reads ~12 mil width (the "1 amp needs about 0.3 mm"
    /// rule of thumb). The formula: area 16.3 mils^2, width 11.9 mils.
    /// A band, not equality — the chart itself is a regression.
    #[test]
    fn ipc2221_anchor_one_amp_external_ten_deg() {
        let spec = CurrentWidthSpec {
            amps: 1.0,
            copper_oz: 1.0,
            temp_rise_c: 10.0,
        };
        let area = ipc2221_area_mils2(1.0, 10.0, true);
        assert!((area - 16.3).abs() < 0.4, "area {area}");
        let width = ipc2221_width_mils(&spec, true);
        assert!((width - 11.9).abs() < 0.4, "width {width}");
    }

    /// THE CHART ANCHOR, high current: 10A / 1oz / 30 deg C external
    /// reads ~146 mils (3.7 mm) on the chart — the formula's 200.0
    /// mils^2 over 1.37 mils. Plus the INTERNAL face: k halves, so the
    /// area scales by exactly 2^(1/0.725) = 2.6124 — the conservatism
    /// IPC charges buried layers, pinned as a ratio (kills a mutant
    /// that swaps or averages the two k constants).
    #[test]
    fn ipc2221_anchor_ten_amps_and_internal_ratio() {
        let spec = CurrentWidthSpec {
            amps: 10.0,
            copper_oz: 1.0,
            temp_rise_c: 30.0,
        };
        let external = ipc2221_width_mils(&spec, true);
        assert!((external - 146.0).abs() < 1.5, "external {external}");
        let internal = ipc2221_width_mils(&spec, false);
        let ratio = internal / external;
        assert!(
            (ratio - 2.0_f64.powf(1.0 / AREA_EXP)).abs() < 0.01,
            "internal/external {ratio}"
        );
    }

    /// The monotonicity faces: more current -> wider, more temp-rise
    /// allowance -> narrower, and copper scales the width INVERSELY
    /// LINEAR (doubling the foil halves the width exactly — the area
    /// is copper-independent).
    #[test]
    fn ipc2221_monotone_and_inverse_linear_in_copper() {
        let base = CurrentWidthSpec {
            amps: 3.0,
            copper_oz: 1.0,
            temp_rise_c: 10.0,
        };
        let hotter = CurrentWidthSpec {
            temp_rise_c: 30.0,
            ..base
        };
        let fatter = CurrentWidthSpec {
            copper_oz: 2.0,
            ..base
        };
        let w = ipc2221_width_mils(&base, true);
        assert!(
            ipc2221_width_mils(&hotter, true) < w,
            "a larger rise allowance narrows the trace"
        );
        assert!(
            ipc2221_width_mils(&fatter, true) < w,
            "thicker copper narrows the trace"
        );
        assert!(
            (ipc2221_width_mils(&fatter, true) - w / 2.0).abs() < 1e-9,
            "copper is inverse-linear"
        );
        // Strictly monotone in current across a sweep.
        let mut prev = 0.0;
        for amps in [0.5_f64, 1.0, 2.0, 4.0, 8.0, 16.0] {
            let spec = CurrentWidthSpec { amps, ..base };
            let width = ipc2221_width_mils(&spec, true);
            assert!(width > prev, "{amps} A must beat {prev} mils");
            prev = width;
        }
    }

    /// THE ROUNDING LAW: a minimum never rounds down. The full width
    /// ceils the product (0.999 mil at 1 unit/mil -> 1 dbu, and 7.001
    /// -> 8); the half width ceils half the CEILED full (full 5 ->
    /// half 3; full 4 -> half 2). Both faces must sit >= the exact
    /// rational value.
    #[test]
    fn dbu_rounding_never_undersizes() {
        assert_eq!(full_width_dbu(0.999, 1.0), 1);
        assert_eq!(full_width_dbu(7.001, 1.0), 8);
        assert_eq!(full_width_dbu(7.0, 1.0), 7, "an exact integer stays put");
        assert_eq!(half_width_dbu(5.0, 1.0), 3, "odd full rounds the half UP");
        assert_eq!(half_width_dbu(4.0, 1.0), 2);
        assert_eq!(half_width_dbu(9.999, 1.0), 5, "full 10 -> half 5");
        // Never below the exact rational value.
        let width_mils = 3.7;
        let units = 25.4_f64;
        assert!(
            f64::from(half_width_dbu(width_mils, units)) >= width_mils * units / 2.0 - f64::EPSILON
        );
    }

    /// The validation gate: every non-positive field is rejected with
    /// a message naming it (the settings warning face renders these
    /// verbatim); a sane spec passes.
    #[test]
    fn spec_validate_rejects_non_positive_fields() {
        let good = CurrentWidthSpec {
            amps: 10.0,
            copper_oz: 1.0,
            temp_rise_c: 10.0,
        };
        assert!(good.validate().is_ok());
        let zero_amps = CurrentWidthSpec { amps: 0.0, ..good };
        assert!(
            zero_amps
                .validate()
                .expect_err("zero amps must be rejected")
                .contains("current")
        );
        let zero_oz = CurrentWidthSpec {
            copper_oz: 0.0,
            ..good
        };
        assert!(
            zero_oz
                .validate()
                .expect_err("zero copper must be rejected")
                .contains("copper")
        );
        let zero_rise = CurrentWidthSpec {
            temp_rise_c: -5.0,
            ..good
        };
        assert!(
            zero_rise
                .validate()
                .expect_err("negative rise must be rejected")
                .contains("rise")
        );
    }
}

/// The F2 integration tests: the apply face on a CRAFT DSN (the F1
/// grammar) extended to a 4-LAYER stackup so the internal/external
/// split is live (F.Cu/In1.Cu/In2.Cu/B.Cu: layers 1-2 internal).
#[cfg(test)]
mod apply_tests {
    use super::*;
    use epic_dsn::reader::{DsnReadResult, read_board};
    use epic_dsn::ses_board::SesBoard;

    const FOUR_LAYER_DSN: &str = r#"(pcb width-craft.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal)(property(index 0)))
    (layer In1.Cu (type signal)(property(index 1)))
    (layer In2.Cu (type signal)(property(index 2)))
    (layer B.Cu (type signal)(property(index 3)))
    (boundary (path pcb 0  0 0  128000 0  128000 128000  0 128000  0 0))
    (rule (clearance 250))
  )
  (placement
    (component "CONN" (place "CONN1" 20000 64000 Front 0.000000))
    (component "TGT" (place "T4" 100000 64000 Front 0.000000))
    (component "TGT" (place "T3" 100000 56000 Front 0.000000))
    (component "TGT" (place "T2" 100000 48000 Front 0.000000))
    (component "TGT" (place "T1" 100000 40000 Front 0.000000))
  )
  (library
    (image "CONN"
      (pin "PAD" "CA1" 0 0)
      (pin "PAD" "CA2" 0 -8000)
      (pin "PAD" "CA3" 0 -16000)
      (pin "PAD" "CA4" 0 -24000)
    )
    (image "TGT"
      (pin "PAD" "TA" 0 0)
    )
    (padstack "PAD"
      (shape (circle F.Cu 2000))
      (attach off)
    )
  )
  (network
    (net "N1" (pins "CONN1"-"CA1" "T1"-"TA"))
    (net "N2" (pins "CONN1"-"CA2" "T2"-"TA"))
    (net "N3" (pins "CONN1"-"CA3" "T3"-"TA"))
    (net "N4" (pins "CONN1"-"CA4" "T4"-"TA"))
    (class kicad_default "N1" "N2" "N3" "N4"
      (rule (clearance 250)(width 2000))
    )
  )
)
"#;

    /// Parses the craft DSN into a live board (no tree manager needed
    /// — the width face mutates RULES only, never items).
    fn four_layer_board() -> Board {
        let mut ses = SesBoard::new();
        match read_board(FOUR_LAYER_DSN.as_bytes(), &mut ses) {
            DsnReadResult::Success { warnings } => {
                assert!(warnings.is_empty(), "WARN_COUNT 0, got {warnings:?}");
            }
            other => panic!("expected Success, got {other:?}"),
        }
        Board::from_ses_board(&ses)
    }

    fn net_no(board: &Board, name: &str) -> i32 {
        board
            .rules()
            .nets
            .iter()
            .find(|(_, net)| net.name == name)
            .unwrap_or_else(|| panic!("net {name} exists"))
            .0
    }

    /// THE F2 PIN: 10 A on N1 synthesizes a widened class — the net
    /// repoints to it, the widths equal the calculator's outputs per
    /// layer with the internal split live (In1 charges the 2.61x
    /// internal factor), the ORIGINAL class is untouched (the clone
    /// law — sibling nets keep 2000), and the board-max warning fires.
    #[test]
    fn ten_amps_widens_external_and_internal_layers() {
        let mut board = four_layer_board();
        let class_count = board.rules().net_classes.len() as i32;
        let n1 = net_no(&board, "N1");
        let n2 = net_no(&board, "N2");
        let original_widths = board.rules().net_classes[usize::try_from(
            board.rules().nets.get(n1).expect("N1").net_class,
        )
        .expect("class index")]
        .trace_half_widths
        .clone();
        let spec = CurrentWidthSpec {
            amps: 10.0,
            copper_oz: 1.0,
            temp_rise_c: 10.0,
        };
        let units = board.communication().resolution_mil();

        let report = apply_current_widths(
            &mut board,
            &[CurrentNetRequest {
                net: "n1".to_string(), // case-insensitive resolution
                amps: 10.0,
            }],
            1.0,
            10.0,
        );

        assert!(report.unresolved.is_empty(), "{:?}", report.unresolved);
        assert_eq!(report.rows.len(), 1, "one widening");
        let row = &report.rows[0];
        assert_eq!(row.net_number, n1);
        assert_eq!(row.net_name, "N1");
        // The DSN's `kicad_default` class is the Java-parity ALIAS for
        // board class 0, named "default" (network.rs:985,
        // Network.java:694) — so the net rides class 0 and the
        // synthetic class is "default~10A" (the REAL KiCad case: every
        // KiCad DSN export names its class kicad_default).
        assert_eq!(row.old_class, "default");
        assert_eq!(row.new_class, "default~10A");
        // Every layer widened (2000 << any 10A minimum).
        assert_eq!(row.layers.len(), 4, "{:?}", row.layers);
        // THE INTEGRATION CONTRACT: each slot equals the calculator's
        // output for that layer's exposure (external 0/3, internal
        // 1/2) at this board's resolution.
        let ext = half_width_dbu(ipc2221_width_mils(&spec, true), units);
        let int = half_width_dbu(ipc2221_width_mils(&spec, false), units);
        assert_eq!(row.layers[0].2, ext, "layer 0 external");
        assert_eq!(row.layers[1].2, int, "layer 1 internal");
        assert_eq!(row.layers[2].2, int, "layer 2 internal");
        assert_eq!(row.layers[3].2, ext, "layer 3 external");
        // The internal factor, live on the board (~2.61x external).
        let ratio = int as f64 / ext as f64;
        assert!((ratio - 2.61).abs() < 0.03, "ratio {ratio}");
        // Band anchor against unit disasters: 10A/1oz/10C external is
        // ~285 mils -> half ~36202 DBU at resolution um 10.
        assert!((35_000..38_000).contains(&ext), "external half width {ext}");
        // The net repoints to the SYNTHESIZED class; siblings and the
        // original class are untouched (the clone law).
        assert_eq!(
            board.rules().nets.get(n1).expect("N1").net_class,
            class_count,
            "N1 rides the appended class"
        );
        assert_eq!(
            board.rules().net_classes[class_count as usize].name,
            "default~10A"
        );
        assert_eq!(
            board.rules().net_classes[0].trace_half_widths,
            original_widths,
            "the ORIGINAL class is unmutated"
        );
        let n2_class = board.rules().nets.get(n2).expect("N2").net_class;
        assert_eq!(n2_class, 0, "the sibling net keeps the original class");
        // The board-max warning fires (36202 > the declared 2000).
        let max_half = board.rules().max_trace_half_width;
        if max_half > 0 && ext > max_half {
            assert_eq!(report.warnings.len(), 1, "{:?}", report.warnings);
            assert!(report.warnings[0].contains("N1"));
            assert!(report.warnings[0].contains("max trace half width"));
        } else {
            assert!(report.warnings.is_empty());
        }
    }

    /// The ONLY-WIDEN floor: a current whose minimum sits UNDER the
    /// class's existing width produces NO row and NO class churn.
    #[test]
    fn small_current_never_narrows() {
        let mut board = four_layer_board();
        let class_count = board.rules().net_classes.len();
        let n2 = net_no(&board, "N2");
        let n2_class_before = board.rules().nets.get(n2).expect("N2").net_class;

        let report = apply_current_widths(
            &mut board,
            &[CurrentNetRequest {
                net: "N2".to_string(),
                amps: 0.01,
            }],
            1.0,
            10.0,
        );

        assert!(report.rows.is_empty(), "{:?}", report.rows);
        assert!(report.unresolved.is_empty());
        assert!(report.warnings.is_empty());
        assert_eq!(board.rules().net_classes.len(), class_count);
        assert_eq!(
            board.rules().nets.get(n2).expect("N2").net_class,
            n2_class_before
        );
    }

    /// Unknown nets and invalid currents are REPORTED, never fatal:
    /// both land in `unresolved` with the net named, nothing mutates.
    #[test]
    fn unknown_net_and_invalid_amps_reported_not_fatal() {
        let mut board = four_layer_board();
        let class_count = board.rules().net_classes.len();
        let requests = [
            CurrentNetRequest {
                net: "NOPE".to_string(),
                amps: 5.0,
            },
            CurrentNetRequest {
                net: "N1".to_string(),
                amps: 0.0,
            },
        ];

        let report = apply_current_widths(&mut board, &requests, 1.0, 10.0);

        assert_eq!(report.unresolved.len(), 2, "{:?}", report.unresolved);
        assert!(report.unresolved[0].contains("NOPE"));
        assert!(report.unresolved[1].contains("N1"));
        assert!(report.rows.is_empty());
        assert_eq!(board.rules().net_classes.len(), class_count);
    }
}
