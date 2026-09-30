//! The M9-T4 headless view state: [`ViewModel`], [`ColorTable`],
//! [`ScreenTransform`], [`OverlayFlags`] — all plain data (`Serialize`
//! where a consumer needs the wire; the render goldens serialize the
//! `RenderList`, not the view). The view is the INPUT side of the
//! projection ([`crate::render::project`]); it holds no board, no
//! engine handle, nothing mutable-by-rendering (the
//! renders-never-mutates law is structural: nothing in this crate can
//! name a board mutation).
//!
//! The Java GUI is the FEATURE reference for what a board GUI shows
//! (`src/main/java/app/freerouting/gui/rendering/` — the color-table
//! family), never a byte oracle: every default constant below cites
//! its Java file:line and carries a one-line why (DNR-19 — adopted
//! constants get their derivation in writing at adoption).

use std::collections::{BTreeMap, BTreeSet};

use epic_engine::snapshot::PointPrimitive;
use epic_geometry::int_point::IntPoint;
use serde::Serialize;

/// The per-layer + per-class color table (u8 RGBA; Java's
/// `ItemColorTableModel` + `OtherColorTableModel` family, audited
/// 2026-09-30 at the frozen tree). Class set = the plan's T4 names
/// PLUS the T5 ratsnest class: trace / via / pad / outline /
/// violation / background / ratsnest. Per-layer
/// entries override the TRACE color on that layer (Java's
/// front/back trace convention, `ItemColorTableModel.java:33` vs
/// `:40`); via/pad keep their class colors on every layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ColorTable {
    /// The canvas backdrop. WHY: Java's default background
    /// (`OtherColorTableModel.java:18`, `new Color(0, 16, 35)`).
    pub background: [u8; 4],
    /// The board outline stroke/fill. WHY: Java's outline default
    /// (`OtherColorTableModel.java:21`, `(100, 150, 255)`).
    pub outline: [u8; 4],
    /// DRC violation markers (data lands in T5; the class ships now).
    /// WHY: Java's violations default (`OtherColorTableModel.java:22`,
    /// `Color.magenta` = 255,0,255).
    pub violation: [u8; 4],
    /// Traces on layers with no [`Self::per_layer`] entry. WHY: Java's
    /// front-layer trace default (`ItemColorTableModel.java:33`,
    /// `(200, 52, 52)`).
    pub trace: [u8; 4],
    /// Via copper + the drill circle. WHY: Java's via default
    /// (`ItemColorTableModel.java:67`, `(227, 183, 46)`).
    pub via: [u8; 4],
    /// Component pads AND areas (the charter's class set has no
    /// separate area slot; conduction copper ≈ pad copper — Java's
    /// distinct conduction default `(0, 150, 0)`,
    /// `ItemColorTableModel.java:34`, is recorded here for a future
    /// class extension, not adopted). WHY: Java's pin default
    /// (`ItemColorTableModel.java:32`, `(227, 183, 46)`).
    pub pad: [u8; 4],
    /// Ratsnest airlines (M9-T5). WHY: Java's incompletes default —
    /// `OtherColorTableModel.java:20`,
    /// `currentRow[ColumnNames.INCOMPLETES.ordinal()] = Color.white`
    /// — and the getter's fallback face repeats white
    /// (`OtherColorTableModel.java:102`,
    /// `getColorSafe(ColumnNames.INCOMPLETES, Color.white)`). Java
    /// draws the airlines with exactly this color
    /// (`NetIncompletesGraphics.java:28`,
    /// `graphicsContext.getIncompleteColor()`).
    pub ratsnest: [u8; 4],
    /// Per-layer TRACE color overrides (BTreeMap — no HashMap anywhere
    /// in the view/projection: the determinism law). Default: Java's
    /// front/back trace pair — layer 0 `(200, 52, 52)`
    /// (`ItemColorTableModel.java:33`), layer 1 `(77, 127, 196)`
    /// (`:40`).
    pub per_layer: BTreeMap<i32, [u8; 4]>,
}

impl ColorTable {
    /// The documented default table (every constant's why is on its
    /// field). Alpha is 255 throughout: Java's `new Color(r, g, b)` is
    /// opaque; the u8 RGBA wire makes that explicit.
    #[must_use]
    pub fn java_default() -> Self {
        Self {
            background: [0, 16, 35, 255],
            outline: [100, 150, 255, 255],
            violation: [255, 0, 255, 255],
            trace: [200, 52, 52, 255],
            via: [227, 183, 46, 255],
            pad: [227, 183, 46, 255],
            // M9-T5's 7th class: Java's INCOMPLETES white (the field
            // doc carries the file:line citations).
            ratsnest: [255, 255, 255, 255],
            per_layer: BTreeMap::from([(0, [200, 52, 52, 255]), (1, [77, 127, 196, 255])]),
        }
    }

    /// The trace color on `layer` (the per-layer override face — the
    /// ONLY class the per-layer table colors).
    #[must_use]
    pub fn trace_color(&self, layer: i32) -> [u8; 4] {
        self.per_layer.get(&layer).copied().unwrap_or(self.trace)
    }
}

impl Default for ColorTable {
    fn default() -> Self {
        Self::java_default()
    }
}

/// The four overlay toggles. The FLAGS gate the overlay op families
/// at the single extension site in `render.rs` (the T4 plumbing; the
/// T5 overlay DATA lives on the snapshot wire, the ops emit only
/// when a flag is ON — a flags-OFF projection is byte-identical to
/// the T4 output, pinned).
/// The default is ALL OFF (the default view shows board geometry
/// only) — the `bool::default` face the derive ships.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct OverlayFlags {
    /// Ratsnest airlines (the incompletes machinery; T5).
    pub ratsnest: bool,
    /// DRC violation markers (T5).
    pub drc: bool,
    /// Congestion heatmap (present iff the global stage ran; T5).
    pub congestion: bool,
    /// Tuning target-band overlays (present iff constraints declared;
    /// T5).
    pub tuning: bool,
}

/// The world (i64 DBU) <-> screen (i32 px) pan + zoom transform.
///
/// `screen = (world - pan) * zoom_num / zoom_den` per axis, zoom
/// carried as a rational (px per DBU; `zoom_num >= 1`, `zoom_den >=
/// 1` — [`Self::new`] clamps). Intermediates run in i128 so no zoom
/// in range overflows; the final narrowings are documented below.
///
/// # Exactness contract (pinned by `render_goldens.rs` round trips)
///
/// * `world_to_screen`'s i128 intermediate is overflow-free; the final
///   narrowing to i32 SATURATES (clamps to i32::MIN/MAX — documented,
///   deterministic, and unreachable at the fixed test transform whose
///   derivation proves the fit). The VALUE is exact iff
///   `zoom_den | (world - pan) * zoom_num` per axis — at a fractional
///   zoom the i128 division truncates (e.g. `w2s(pan + 1500)` at
///   1/1000 is 1, not 1.5).
/// * `screen_to_world` truncates (i128 division truncates toward
///   zero) — the round trip `screen_to_world(world_to_screen(w)) ==
///   w` holds for EVERY world point iff `zoom_num % zoom_den == 0`
///   (the integer px-per-DBU family; the image lattice is then all of
///   Z), and on the world lattice `{pan + k * zoom_den / g}` for
///   fractional zooms. Symmetrically `world_to_screen(
///   screen_to_world(s)) == s` holds for EVERY screen point iff
///   `zoom_den % zoom_num == 0`. Both directions are therefore fully
///   exact ONLY at zoom 1/1 — which is why the fixed test transform
///   (see the goldens) is the 1:1 bijection, not the dispatch
///   letter's "1:1000-ish" example: the round-trip pin demands
///   world-exactness at the i64 faces, and 1:1 is the unique ratio
///   that delivers it on the full domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ScreenTransform {
    /// The world DBU point mapped to the screen origin.
    pub pan: PointPrimitive,
    /// The zoom numerator (px per DBU = `zoom_num / zoom_den`, >= 1).
    pub zoom_num: i64,
    /// The zoom denominator (>= 1).
    pub zoom_den: i64,
}

impl ScreenTransform {
    /// Builds a transform, clamping the zoom factors to the `>= 1`
    /// invariant (a zero/negative denominator would divide by zero;
    /// the clamp keeps the constructor total).
    #[must_use]
    pub fn new(pan: PointPrimitive, zoom_num: i64, zoom_den: i64) -> Self {
        Self {
            pan,
            zoom_num: zoom_num.max(1),
            zoom_den: zoom_den.max(1),
        }
    }

    /// World DBU -> screen px (i128 intermediate, saturating i32
    /// narrowing — see the exactness contract).
    #[must_use]
    pub fn world_to_screen(&self, p: PointPrimitive) -> IntPoint {
        IntPoint::new(
            narrow_i128_to_i32(
                (i128::from(p.x) - i128::from(self.pan.x)) * i128::from(self.zoom_num)
                    / i128::from(self.zoom_den),
            ),
            narrow_i128_to_i32(
                (i128::from(p.y) - i128::from(self.pan.y)) * i128::from(self.zoom_num)
                    / i128::from(self.zoom_den),
            ),
        )
    }

    /// Screen px -> world DBU (i128 intermediate, truncating i64
    /// narrowing — see the exactness contract).
    #[must_use]
    pub fn screen_to_world(&self, s: IntPoint) -> PointPrimitive {
        PointPrimitive {
            x: narrow_i128_to_i64(
                i128::from(s.x) * i128::from(self.zoom_den) / i128::from(self.zoom_num)
                    + i128::from(self.pan.x),
            ),
            y: narrow_i128_to_i64(
                i128::from(s.y) * i128::from(self.zoom_den) / i128::from(self.zoom_num)
                    + i128::from(self.pan.y),
            ),
        }
    }

    /// The zoom as f32 (the [`crate::render::RenderOp::SetWidth`]
    /// derivation input: DBU half-width x scale = stroke px). i64 ->
    /// f32 may round; the scale feeds only the f32 stroke width,
    /// whose rounding face is documented in `render.rs` — no
    /// exactness claim rides on it.
    #[must_use]
    pub fn scale_f32(&self) -> f32 {
        self.zoom_num as f32 / self.zoom_den as f32
    }
}

/// i128 -> i32 narrowing, saturating at the i32 range (documented on
/// [`ScreenTransform::world_to_screen`]).
fn narrow_i128_to_i32(v: i128) -> i32 {
    if v > i128::from(i32::MAX) {
        i32::MAX
    } else if v < i128::from(i32::MIN) {
        i32::MIN
    } else {
        v as i32
    }
}

/// i128 -> i64 narrowing, saturating at the i64 range (the
/// screen_to_world twin of [`narrow_i128_to_i32`]; `pub(crate)` so
/// `render.rs`'s drill-radius face reuses the ONE narrowing
/// implementation — the quality-review Q7 dedup).
pub(crate) fn narrow_i128_to_i64(v: i128) -> i64 {
    if v > i128::from(i64::MAX) {
        i64::MAX
    } else if v < i128::from(i64::MIN) {
        i64::MIN
    } else {
        v as i64
    }
}

/// The view state: what the renderer shows and how
/// ([`crate::render::project`] is a PURE function of
/// `(snapshot, view, viewport)` — same inputs, byte-identical
/// `RenderList`, always). All plain data; no engine handle, no board.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ViewModel {
    /// The visible copper layers (0-based, ascending-ordered set).
    /// An item renders iff its layer face intersects this set (traces
    /// carry one layer; vias render per-shape for their visible
    /// layers); the layer-less board outline renders regardless.
    pub visible_layers: BTreeSet<i32>,
    /// The color table (the documented Java-derived default:
    /// [`ColorTable::java_default`]).
    pub colors: ColorTable,
    /// The pan + zoom transform (world DBU <-> screen px).
    pub transform: ScreenTransform,
    /// The overlay toggles (each ON flag emits exactly its own op
    /// family at the single extension site in `render.rs`; all OFF
    /// is byte-identical to the T4 geometry-only output).
    pub overlays: OverlayFlags,
}

impl ViewModel {
    /// A view at `transform` with NO visible layers (the caller
    /// populates [`Self::visible_layers`] — there is no implicit
    /// "all layers" without knowing the board's layer count).
    #[must_use]
    pub fn new(transform: ScreenTransform) -> Self {
        Self {
            visible_layers: BTreeSet::new(),
            colors: ColorTable::default(),
            transform,
            overlays: OverlayFlags::default(),
        }
    }
}
