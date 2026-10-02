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
