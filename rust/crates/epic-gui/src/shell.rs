//! The M9-T6 desktop-shell PROTOCOL module — deliberately UNGATED
//! (no eframe imports): every pure face the census pins need lives
//! here so `cargo test --workspace` at DEFAULT features compiles and
//! pins it, while the eframe-touching code sits behind
//! `#[cfg(feature = "desktop")]` (`crate::desktop`). The module split
//! is the census-pinnability law (the T6 charter's design
//! adjudication (b)).
//!
//! Contents:
//!
//! * [`GuiToWorker`] / [`WorkerToGui`] — the two channel protocols
//!   between the GUI thread and the worker thread (the worker owns
//!   the [`epic_engine::session::Session`]; the GUI never does —
//!   the renders-never-mutates law's host face).
//! * [`coalesce_latest_wins`] — the AM5 mpsc drain policy (LATEST-
//!   WINS COALESCING on the GUI consumer for snapshot-carrying
//!   messages): the GUI drains the unbounded channel at frame
//!   cadence; a burst of N snapshot-carrying messages keeps the LAST
//!   one and drops the earlier ones, while every non-snapshot
//!   (scalar) message is processed in order. Pure, documented,
//!   pinned.
//! * [`rational_zoom_step`] — the wheel→zoom mapping as a RATIONAL
//!   step on [`epic_gui::view::ScreenTransform`]'s
//!   `zoom_num`/`zoom_den`: ×2/÷2 on the RATIO with the FACTORS kept
//!   `>= 1`, saturating at [`ZOOM_FACTOR_MAX`]. The zoom-out door is
//!   open by construction (the ratio may go below 1: zoom 1/2, 1/1000
//!   …), and the shell must NOT assume lossless world→screen at
//!   fractional zoom (the AM4 exactness contract — the ops are
//!   already screen-space; the transform's documented truncation is
//!   the honest face).
//! * [`splice_overlays`] — the AM5 overlay-sourcing law: the shell
//!   sources overlay data ONLY from `Session::snapshot_with_overlays`
//!   (the worker's `Attached` messages). Tee snapshots
//!   ([`EngineEvent::Snapshot`]) carry the EMPTY overlay default by
//!   the T5 purity law — the field docs say it three times, this
//!   module says it a fourth — so a tee snapshot that lands after an
//!   `Attached` must NEVER clobber the attached overlays (pinned).
//!
//! Everything here is plain data + pure functions; the gated desktop
//! module is the only interpreter.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use epic_engine::events::EngineEvent;
use epic_engine::session::RouteSummary;
use epic_engine::snapshot::BoardSnapshot;

// ---------------------------------------------------------------------------
// the command marshalling (GUI -> worker)
// ---------------------------------------------------------------------------

/// GUI -> worker commands (the charter's five-face surface). The
/// worker thread owns the [`epic_engine::session::Session`] and is
/// the ONLY mutator.
///
/// `#[allow(clippy::large_enum_variant)]` (the T5 D1 precedent):
/// `StartRoute` carries the whole [`CliLayer`](~400 B) vs a 24-byte
/// `LoadPath` — boxing the layer would churn every construction
/// site for zero wire cost (an in-process channel; the size is
/// ACCEPTED, documented).
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum GuiToWorker {
    /// Load a DSN from disk (the load face; the worker ships
    /// [`WorkerToGui::LoadResult`] then `Attached` + `DepthTotal`).
    LoadPath(PathBuf),
    /// Start one route run with this CLI layer (the route face; the
    /// worker ships the tee's `Engine` events live, then
    /// [`WorkerToGui::RouteDone`] + `Attached` + `DepthTotal`).
    StartRoute(epic_engine::settings::CliLayer),
    /// The command face of cancel: handled by the worker when IDLE
    /// (it sets the session's stop flag; a pre-route raise is the
    /// documented pass-through COMPLETED face). MID-RUN cancel is
    /// the session's own sanctioned face
    /// ([`epic_engine::session::Session::stop_flag`]): the worker
    /// ships the `Arc<AtomicBool>` handle in
    /// [`WorkerToGui::LoadResult`], and the GUI stores through it
    /// directly — the worker thread is inside `Session::route` while
    /// a route runs and cannot dequeue commands.
    Cancel,
    /// Export the SES session file (the export face, gated by the
    /// session's COMPLETED/TIMED_OUT gate — CANCELLED refuses).
    ExportSes(PathBuf),
    /// Request an attach-step `snapshot_with_overlays` for display
    /// (the worker ships `Attached` + `DepthTotal`).
    Snapshot,
}

// ---------------------------------------------------------------------------
// the event marshalling (worker -> GUI)
// ---------------------------------------------------------------------------

/// The worker's post-load report ([`WorkerToGui::LoadResult`]'s Ok
/// payload).
#[derive(Debug, Clone)]
pub struct LoadReport {
    /// The input name the SES design face is derived from (the
    /// worker sets it from the loaded path's file stem).
    pub input_name: String,
    /// The DSN loaded without a usable outline (the warn-and-continue
    /// `OutlineMissing` face — the board is routed with the default
    /// boundary, exactly the CLI's face).
    pub outline_missing: bool,
    /// The parse warnings collected at load.
    pub warnings: Vec<String>,
    /// The post-load board revision.
    pub revision: u64,
    /// The post-load item count.
    pub item_count: usize,
    /// The session's stop-flag handle — the MID-RUN cancel face (see
    /// [`GuiToWorker::Cancel`]).
    pub stop_flag: Arc<AtomicBool>,
}

/// Worker -> GUI messages (one channel; the shape is the charter's
/// minimum plus the documented extras).
#[derive(Debug, Clone)]
pub enum WorkerToGui {
    /// An engine event from the tee (the worker forwards the whole
    /// typed stream; the GUI applies
    /// [`ENGINE_EVENT_STREAM_VERSION`] discipline on
    /// [`EngineEvent::StreamStarted`]).
    Engine(EngineEvent),
    /// An ATTACH-step snapshot ([`WorkerToGui::Engine`]'s
    /// `EngineEvent::Snapshot`s carry the EMPTY overlay default by
    /// the purity law; this message carries the REAL overlays — the
    /// ONLY overlay source the shell renders, the AM5 law).
    Attached(BoardSnapshot),
    /// The live clearance-violation depth total (the T5 fix-round
    /// face, ADOPTED as the shell's DRC panel face — quality Q3's
    /// production consumer). Shipped at attach cadence.
    DepthTotal(i64),
    /// One route run finished (Err when the command found no loaded
    /// session).
    RouteDone(Result<RouteSummary, String>),
    /// The load face finished (Err: the hard [`epic_engine::session
    /// ::LoadError`] faces — parse/Io — rendered to text).
    LoadResult(Result<LoadReport, String>),
    /// The export face finished (Err carries the gate/write text).
    ExportResult(Result<(), String>),
}

// ---------------------------------------------------------------------------
// the AM5 latest-wins drain policy (pure, pinned)
// ---------------------------------------------------------------------------

/// Whether `msg` carries a whole-board snapshot payload (the
/// coalescing class). `Attached` always; `Engine(Snapshot)` for the
/// tee's pass-granular snapshots (EMPTY overlays by the purity law —
/// display-side splicing below).
fn is_snapshot_carrying(msg: &WorkerToGui) -> bool {
    matches!(
        msg,
        WorkerToGui::Attached(_) | WorkerToGui::Engine(EngineEvent::Snapshot { .. })
    )
}

/// THE AM5 DRAIN POLICY (latest-wins coalescing on the GUI consumer
/// for snapshot-carrying messages): from the drained batch, keep the
/// LAST snapshot-carrying message and EVERY non-snapshot message, in
/// original order. The worker never blocks on the GUI; the GUI never
/// falls behind a snapshot storm — the intermediate frames would
/// never be shown, so they are dropped wholesale.
///
/// Mutant-kill note (the pin's charter letter): a drop-ALL mutant
/// loses the scalars; a keep-FIRST mutant loses the newest geometry;
/// a reorder mutant violates the scalar ordering — the pin asserts
/// the exact surviving sequence, so all three die.
#[must_use]
pub fn coalesce_latest_wins(batch: Vec<WorkerToGui>) -> Vec<WorkerToGui> {
    let mut last_snapshot: Option<usize> = None;
    for (index, msg) in batch.iter().enumerate() {
        if is_snapshot_carrying(msg) {
            last_snapshot = Some(index);
        }
    }
    batch
        .into_iter()
        .enumerate()
        .filter(|(index, msg)| !is_snapshot_carrying(msg) || Some(*index) == last_snapshot)
        .map(|(_, msg)| msg)
        .collect()
}

// ---------------------------------------------------------------------------
// the overlay-sourcing law (the splice, pure, pinned)
// ---------------------------------------------------------------------------

/// Merges an incoming display snapshot with the LAST attached
/// ([`WorkerToGui::Attached`]) overlays: the incoming snapshot's
/// geometry stands, the OVERLAY SLOTS come from `overlays_source`
/// when present. The AM5 law operationalized: tee snapshots carry
/// the EMPTY overlay default (the T5 purity law — `board_snapshot`
/// fills the slots empty by construction), so without the splice a
/// tee snapshot landing after an `Attached` would silently blank the
/// ratsnest/DRC/congestion/tuning panels' data. The GUI keeps the
/// last attached overlays separately and passes them here for every
/// snapshot display update.
#[must_use]
pub fn splice_overlays(
    mut incoming: BoardSnapshot,
    overlays_source: Option<&epic_engine::snapshot::OverlayData>,
) -> BoardSnapshot {
    if let Some(source) = overlays_source {
        incoming.overlays = source.clone();
    }
    incoming
}

// ---------------------------------------------------------------------------
// the rational zoom step (pure, pinned)
// ---------------------------------------------------------------------------

/// The zoom-factor saturation bound (DNR-19 derivation): 2^40 px per
/// DBU is 512x past the point where EVERY world point maps outside
/// the i32 screen range (2^31 px per DBU already saturates any
/// display at 1 DBU of offset), so doubling beyond it carries zero
/// information; 2^40 also keeps `num * (world)` products far inside
/// the transform's i128 intermediates, and it bounds the wheel: the
/// bound is reachable only after 40 In-steps from 1/1. The Out
/// direction mirrors it symmetrically (a px-per-DBU ratio below
/// 2^-40 means a full board spans sub-2^-28 px — past any screen).
pub const ZOOM_FACTOR_MAX: i64 = 1 << 40;

/// The wheel direction (In = more px per DBU).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoomDirection {
    /// Zoom in: the ratio doubles.
    In,
    /// Zoom out: the ratio halves.
    Out,
}

/// ONE wheel notch = ONE RATIONAL ZOOM STEP (DNR-19 derivation of
/// the step size: a factor-of-2 octave is the smallest perceptually
/// uniform multiplicative step on the log-zoom scale and matches the
/// Java GUI's 2x zoom family — one notch, one octave). On the ratio
/// `num/den`:
///
/// * [`ZoomDirection::In`] doubles `num` (the ratio doubles);
/// * [`ZoomDirection::Out`] doubles `den` (the ratio halves — the
///   zoom-OUT door: the ratio may drop below 1 freely, e.g. 1/1 ->
///   1/2 -> 1/4; only the FACTORS are clamped `>= 1`, view.rs:161-175).
///
/// Both factors saturate at [`ZOOM_FACTOR_MAX`] instead of
/// overflowing (a saturated step is a documented no-op), and a
/// zero/negative input factor is clamped to 1 first (the constructor
/// total's twin — the fn is total on i64 inputs).
#[must_use]
pub fn rational_zoom_step(zoom_num: i64, zoom_den: i64, direction: ZoomDirection) -> (i64, i64) {
    let num = zoom_num.max(1);
    let den = zoom_den.max(1);
    match direction {
        ZoomDirection::In => (num.saturating_mul(2).min(ZOOM_FACTOR_MAX), den),
        ZoomDirection::Out => (num, den.saturating_mul(2).min(ZOOM_FACTOR_MAX)),
    }
}

// ---------------------------------------------------------------------------
// the G1 navigation faces (notch-accumulated, cursor-anchored zoom)
// ---------------------------------------------------------------------------

/// ONE wheel notch in egui `smooth_scroll_delta` points (the G1
/// derivation): a physical wheel click delivers ~50 points on the
/// observed X11/Wayland faces (the egui convention), so ONE notch =
/// ONE rational ×2 step. The pre-G1 defect consumed the delta
/// per-FRAME as a full step — a flick's smooth-scroll momentum
/// applied dozens of octave doublings per second and "lost the board
/// in 1s". The constant is the tuning knob: bigger = slower zoom.
pub const WHEEL_NOTCH_POINTS: f32 = 50.0;

/// The wheel→step mapping (pure, G1): fold one frame's scroll delta
/// into the accumulator and emit WHOLE notches only — `(steps,
/// residual)` with the residual kept for the next frame (magnitude
/// `< WHEEL_NOTCH_POINTS`; the sign carries direction: positive steps
/// = In). Total on every f32: a non-finite total resets the
/// accumulator `(0, 0.0)` (a poisoned accumulator would eat every
/// future notch — the documented NaN face), and the step count
/// saturates at the i32 faces rather than wrapping.
#[must_use]
pub fn wheel_notch_steps(accumulated: f32, delta: f32) -> (i32, f32) {
    let total = accumulated + delta;
    if !total.is_finite() {
        return (0, 0.0);
    }
    let steps = (total / WHEEL_NOTCH_POINTS).trunc() as i32;
    let residual = total - steps as f32 * WHEEL_NOTCH_POINTS;
    (steps, residual)
}

/// The CURSOR-ANCHORED zoom re-anchoring (pure, G1): given the pan
/// before and the (num, den) zoom pair before/after one rational
/// step, return the pan that keeps the WORLD POINT UNDER THE CURSOR
/// under the cursor. Per axis, mirroring
/// [`crate::view::ScreenTransform`]'s OWN faces exactly (i128
/// intermediates, truncating division — the AM4 exactness contract's
/// sibling):
///
/// * `world = pan + trunc(cursor · old_den / old_num)` (the
///   transform's `screen_to_world` at the cursor);
/// * `pan_new = world − trunc(cursor · new_den / new_num)` (the world
///   offset whose `world_to_screen` lands on the cursor).
///
/// Factors are clamped `>= 1` first (the constructor-total twin), and
/// `pan_new` saturates at the i64 faces (never a wrap). ANCHOR ERROR:
/// at a zoom where the world lattice is coarser than 1 px per DBU
/// (`new_num / new_den > 1`), no integer pan can hold the cursor
/// EXACTLY — the returned pan's error at the cursor is bounded by
/// `< new_num` px (one lattice step on screen, the minimum any
/// integer transform guarantees); at `new_num | cursor · new_den` the
/// anchor round-trips EXACTLY (pinned).
#[must_use]
pub fn zoom_about_point(
    pan: epic_engine::snapshot::PointPrimitive,
    cursor: (i32, i32),
    old: (i64, i64),
    new: (i64, i64),
) -> epic_engine::snapshot::PointPrimitive {
    let old_num = old.0.max(1);
    let old_den = old.1.max(1);
    let new_num = new.0.max(1);
    let new_den = new.1.max(1);
    // The world axis under the cursor at zoom num/den (i128
    // intermediate — pan and the cursor product cannot overflow it).
    let world_axis = |pan_axis: i64, cursor_axis: i32, den: i64, num: i64| -> i128 {
        i128::from(pan_axis) + i128::from(cursor_axis) * i128::from(den) / i128::from(num)
    };
    // The pan that re-anchors (saturating at the i64 faces).
    let pan_axis = |world: i128, cursor_axis: i32, den: i64, num: i64| -> i64 {
        let value = world - i128::from(cursor_axis) * i128::from(den) / i128::from(num);
        value.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
    };
    let world_x = world_axis(pan.x, cursor.0, old_den, old_num);
    let world_y = world_axis(pan.y, cursor.1, old_den, old_num);
    epic_engine::snapshot::PointPrimitive {
        x: pan_axis(world_x, cursor.0, new_den, new_num),
        y: pan_axis(world_y, cursor.1, new_den, new_num),
    }
}

// ---------------------------------------------------------------------------
// the launch modes (the smoke faces' arg face; the bin parses argv
// through this and hands the mode to the gated shell)
// ---------------------------------------------------------------------------

/// The bin's launch mode (the charter's evidence face (e)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchMode {
    /// The interactive shell (the ONLY mode that does not exit by
    /// itself; the standing constraint is that the shell must not run
    /// unattended — the smoke modes are the bounded faces).
    Interactive,
    /// `--smoke <dsn> --frames <N> --out <png>`: load, optionally
    /// start the route (`--route`), run N frames at the fixed
    /// [`SMOKE_FRAME_DT`], screenshot, write the PNG, exit 0.
    Smoke {
        /// The DSN to load.
        dsn: PathBuf,
        /// The frame count (the bounded-run budget).
        frames: u32,
        /// The PNG output path.
        out: PathBuf,
        /// Whether a default-settings route starts after load.
        route: bool,
    },
    /// `--cancel-smoke <dsn>`: load, start the route, cancel at the
    /// first `PassProgress`, assert CANCELLED + no export file
    /// written, log + exit code.
    CancelSmoke {
        /// The DSN to load.
        dsn: PathBuf,
    },
    /// `--version`: print the version line and exit 0 WITHOUT
    /// launching the GUI (no DISPLAY required — the shipped
    /// desktop-only bin must answer it headlessly). The bin handles
    /// this mode before `desktop::run`; the printed line is
    /// [`version_line`], the single source.
    Version,
}

/// The version face (M10-T6): the ONE source of the `--version` line,
/// shared by the bin's print site and the census-visible pin. Equal to
/// `epicrouter <CARGO_PKG_VERSION>` — the same workspace-manifest
/// source the CLI's `--version` face and the result manifest's
/// `app_version` read.
#[must_use]
pub fn version_line() -> String {
    format!("epicrouter {}", env!("CARGO_PKG_VERSION"))
}

/// The first-display FIT ZOOM (the fix-round Q2 hoist — closes the
/// census-pinnability leak: the math was gated inside the desktop
/// `initialize_view_for` with zero pin coverage; the charter §2b law
/// says pure logic the pins need lives UNGATED). Returns
/// `(zoom_num, zoom_den)` = `(1, ceil(longest / 1400))`.
///
/// DNR-19 derivation of the 1400 px: the LONGEST board side spans a
/// ~1400 px initial viewport (the 1280x800 smoke window's usable
/// canvas budget, with margin), so a rational `<= 1` fits the whole
/// board (the zoom-out door; factors stay `>= 1`). Degenerate inputs
/// (zero/negative longest) clamp to den 1 via the `.max(1)`.
#[must_use]
pub fn fit_zoom(longest: i64) -> (i64, i64) {
    // Manual ceil-div — `i64::div_ceil` is still unstable at the
    // pinned 1.93 toolchain.
    let den = ((longest + 1399) / 1400).max(1);
    (1, den)
}

/// The smoke launch's overlay face (the fix-round F1 answer):
/// ALL FOUR toggles ON when the shell launches in a smoke mode —
/// criterion 3's "each [view] visible in the shell" evidence face
/// must show RENDERINGS, not just controls. Honest absence note:
/// the congestion/tuning FAMILIES are `None`-iff-not-engaged by the
/// T5 design (the snapshot carries them only when the route engaged
/// `congestion_global` / the DSN declares length constraints), and
/// the drc markers render only where violations exist — so on bm08
/// at the parse boundary the RATSNEST airlines are the populated
/// family (the T5 golden's face); the flags being ON cannot invent
/// data.
#[must_use]
pub fn smoke_overlay_flags() -> crate::view::OverlayFlags {
    crate::view::OverlayFlags {
        ratsnest: true,
        drc: true,
        congestion: true,
        tuning: true,
    }
}

/// The smoke frames' fixed dt (DNR-19 derivation): 1/60 s — the
/// plan's 60-fps design intent (T7's render-perf face measures
/// against it). A FIXED dt decouples the smoke from wall-clock so
/// the frame count is the only cadence variable.
pub const SMOKE_FRAME_DT: f32 = 1.0 / 60.0;

/// The launch-arg parser (the bin's argv face; total — every failure
/// is a `Err` usage string, never a panic). Accepted shapes:
/// `(no args)` | `--version` | `--smoke <dsn> --frames <N> --out <png>
/// [--route]` | `--cancel-smoke <dsn>`.
pub fn parse_launch_args<I: Iterator<Item = String>>(args: I) -> Result<LaunchMode, String> {
    let args: Vec<String> = args.collect();
    if args.is_empty() {
        return Ok(LaunchMode::Interactive);
    }
    match args.first().map(String::as_str) {
        Some("--version") => {
            if args.len() != 1 {
                return Err("--version takes no arguments".to_string());
            }
            Ok(LaunchMode::Version)
        }
        Some("--smoke") => {
            let mut dsn = None;
            let mut frames = None;
            let mut out = None;
            let mut route = false;
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--frames" => {
                        let Some(value) = args.get(i + 1) else {
                            return Err("--frames requires a number".to_string());
                        };
                        frames = Some(
                            value
                                .parse::<u32>()
                                .map_err(|_| format!("--frames expects a u32, got {value:?}"))?,
                        );
                        i += 2;
                    }
                    "--out" => {
                        let Some(value) = args.get(i + 1) else {
                            return Err("--out requires a path".to_string());
                        };
                        out = Some(PathBuf::from(value));
                        i += 2;
                    }
                    "--route" => {
                        route = true;
                        i += 1;
                    }
                    other => {
                        if dsn.is_none() && !other.starts_with("--") {
                            dsn = Some(PathBuf::from(other));
                            i += 1;
                        } else {
                            return Err(format!("unexpected argument for --smoke: {other:?}"));
                        }
                    }
                }
            }
            match (dsn, frames, out) {
                (Some(dsn), Some(frames), Some(out)) => Ok(LaunchMode::Smoke {
                    dsn,
                    frames,
                    out,
                    route,
                }),
                _ => Err("--smoke requires --frames <N> and --out <png>".to_string()),
            }
        }
        Some("--cancel-smoke") => {
            let [_, dsn] = args.as_slice() else {
                return Err("--cancel-smoke takes exactly one DSN path".to_string());
            };
            Ok(LaunchMode::CancelSmoke {
                dsn: PathBuf::from(dsn),
            })
        }
        Some(other) => Err(format!("unknown launch mode: {other:?}")),
        None => Ok(LaunchMode::Interactive),
    }
}

// The worker ships every variant across the thread boundary (the
// compile-time twin of the events_stream Send pin): a non-Send
// payload must fail HERE, at the protocol type, not at the worker.
const _: fn() = || {
    fn assert_send<T: Send>() {}
    assert_send::<WorkerToGui>();
    assert_send::<GuiToWorker>();
};

#[cfg(test)]
mod tests {
    use super::*;
    use epic_engine::snapshot::OverlayData;

    /// A BoardSnapshot shaped enough to carry overlay payloads in
    /// the coalescing/splice tests (the wire type has no `Default`
    /// — every field is spelled; the tests build the empty board and
    /// fill the slot under test).
    fn bare_snapshot() -> BoardSnapshot {
        BoardSnapshot {
            traces: Vec::new(),
            vias: Vec::new(),
            pads: Vec::new(),
            areas: Vec::new(),
            outline: Vec::new(),
            nets: Vec::new(),
            bounds: epic_engine::snapshot::BoxPrimitive {
                ll_x: 0,
                ll_y: 0,
                ur_x: 0,
                ur_y: 0,
            },
            revision: 0,
            overlays: OverlayData::default(),
        }
    }

    /// THE AM5 DRAIN PIN: a queue with several snapshot-carrying
    /// messages + interleaved scalars keeps ONLY the LAST snapshot
    /// and preserves scalar order (kills the drop-all mutant — the
    /// scalars survive; the keep-first mutant — the LAST snapshot
    /// survives; the reorder mutant — the exact sequence asserts).
    #[test]
    fn coalesce_keeps_last_snapshot_and_scalar_order() {
        let scalar = |text: &str| {
            WorkerToGui::Engine(EngineEvent::Message {
                level: "info".to_string(),
                text: text.to_string(),
            })
        };
        let batch = vec![
            scalar("first"),
            WorkerToGui::Attached(bare_snapshot()),
            scalar("second"),
            WorkerToGui::Engine(EngineEvent::Snapshot {
                revision: 7,
                snapshot: bare_snapshot(),
            }),
            scalar("third"),
            WorkerToGui::Attached(bare_snapshot()),
            scalar("last"),
        ];
        let drained = coalesce_latest_wins(batch);
        assert_eq!(
            drained.len(),
            5,
            "4 scalars + the LAST snapshot: {drained:?}"
        );
        // Scalars in original order...
        assert!(
            matches!(&drained[0], WorkerToGui::Engine(EngineEvent::Message{ text, ..}) if text == "first")
        );
        assert!(
            matches!(&drained[1], WorkerToGui::Engine(EngineEvent::Message{ text, ..}) if text == "second")
        );
        assert!(
            matches!(&drained[2], WorkerToGui::Engine(EngineEvent::Message{ text, ..}) if text == "third")
        );
        // ...and the LAST snapshot-carrying message is the survivor,
        // in ITS original position relative to the trailing scalar.
        assert!(matches!(&drained[3], WorkerToGui::Attached(_)));
        assert!(
            matches!(&drained[4], WorkerToGui::Engine(EngineEvent::Message{ text, ..}) if text == "last")
        );
    }

    /// The drain's edge faces: an empty batch drains empty; a batch
    /// of ONLY snapshot-carrying messages collapses to the last one;
    /// a batch with no snapshots is returned untouched (the scalar
    /// pass-through — the coalescer must never eat progress rows).
    #[test]
    fn coalesce_edge_faces() {
        assert!(coalesce_latest_wins(Vec::new()).is_empty());
        let only_snapshots = vec![
            WorkerToGui::Attached(bare_snapshot()),
            WorkerToGui::Engine(EngineEvent::Snapshot {
                revision: 1,
                snapshot: bare_snapshot(),
            }),
            WorkerToGui::Attached(bare_snapshot()),
        ];
        let drained = coalesce_latest_wins(only_snapshots);
        assert_eq!(drained.len(), 1, "the LAST of three: {drained:?}");
        assert!(matches!(&drained[0], WorkerToGui::Attached(_)));
        let scalar = WorkerToGui::Engine(EngineEvent::TaskState {
            state: "STARTED".to_string(),
            pass: 1,
            hash: "abc".to_string(),
        });
        let no_snapshots = vec![scalar.clone(), scalar];
        assert_eq!(coalesce_latest_wins(no_snapshots).len(), 2);
    }

    /// THE SPLICE PIN (the AM5 overlay-sourcing law, mutation face):
    /// a tee snapshot (EMPTY overlays by the purity law) landing
    /// after an `Attached` must NOT clobber the attached overlays —
    /// the shells' ratsnest/DRC/congestion/tuning data survives;
    /// with NO attached source the incoming overlays stand (the
    /// pre-attach display face). Kills the naive
    /// `display = incoming` clobber mutant.
    #[test]
    fn tee_snapshot_never_clobbers_attached_overlays() {
        use epic_engine::snapshot::AirLinePrimitive;
        let mut attached = bare_snapshot();
        // A NON-EMPTY attached overlay slot (airlines — the data the
        // ratsnest family renders).
        attached.overlays.airlines = vec![AirLinePrimitive {
            from: epic_engine::snapshot::PointPrimitive { x: 1, y: 2 },
            to: epic_engine::snapshot::PointPrimitive { x: 3, y: 4 },
            net: 1,
        }];
        let tee_snapshot = bare_snapshot(); // EMPTY overlays (the purity law)
        // With an attached source: the attached overlays WIN.
        let spliced = splice_overlays(tee_snapshot.clone(), Some(&attached.overlays));
        assert_eq!(
            spliced.overlays.airlines.len(),
            1,
            "attached overlays survive a tee snapshot"
        );
        // Without one (the pre-attach face): the incoming stand
        // (empty stays empty).
        let unspliced = splice_overlays(tee_snapshot, None);
        assert!(unspliced.overlays.airlines.is_empty());
        let _ = OverlayData::default(); // the empty-default type, named for the doc law
    }

    /// THE RATIONAL ZOOM PIN: x2 and /2 at 1/1, 1/2, 3/1 — factors
    /// stay >= 1, the ratio doubles/halves EXACTLY, saturation at
    /// [`ZOOM_FACTOR_MAX`] is a no-op (kills the num/den swap mutant
    /// and the int-rounding mutant).
    #[test]
    fn rational_zoom_step_faces() {
        use ZoomDirection::{In, Out};
        // 1/1: In -> 2/1, Out -> 1/2.
        assert_eq!(rational_zoom_step(1, 1, In), (2, 1));
        assert_eq!(rational_zoom_step(1, 1, Out), (1, 2));
        // 1/2: In -> 2/2 (the ratio is 1 — factors stay >= 1), Out -> 1/4.
        assert_eq!(rational_zoom_step(1, 2, In), (2, 2));
        assert_eq!(rational_zoom_step(1, 2, Out), (1, 4));
        // 3/1: In -> 6/1, Out -> 3/2.
        assert_eq!(rational_zoom_step(3, 1, In), (6, 1));
        assert_eq!(rational_zoom_step(3, 1, Out), (3, 2));
        // The zoom-out door stays OPEN past ratio 1: from 1/4, Out -> 1/8.
        assert_eq!(rational_zoom_step(1, 4, Out), (1, 8));
        // Saturation: at the bound both directions are no-ops (the
        // documented face — never an overflow).
        let (num, den) = (ZOOM_FACTOR_MAX, 1);
        assert_eq!(rational_zoom_step(num, den, In), (ZOOM_FACTOR_MAX, 1));
        let (num, den) = (1, ZOOM_FACTOR_MAX);
        assert_eq!(rational_zoom_step(num, den, Out), (1, ZOOM_FACTOR_MAX));
        // Degenerate inputs clamp TO 1 (the >= 1 invariant — the
        // constructor's total-face twin; -3 clamps to 1, not to
        // itself).
        assert_eq!(rational_zoom_step(0, 0, In), (2, 1));
        assert_eq!(rational_zoom_step(-3, -3, Out), (1, 2));
    }

    /// THE FIT-ZOOM PIN (fix-round Q2, closes the KM5 survivor):
    /// the DNR-16 ceiling face BOTH directions at every 1400
    /// boundary (exactly ON a multiple stays in that band, one DBU
    /// over bumps the band) + the degenerate `.max(1)` face. Kills
    /// the floor-div mutant (1401 would read den 1) and the
    /// missing-clamp mutant (0 would read den 0 — a divide-by-zero
    /// zoom).
    #[test]
    fn fit_zoom_faces() {
        // ON the first multiple vs one DBU over (both directions of
        // the DNR-16 boundary).
        assert_eq!(fit_zoom(1400), (1, 1));
        assert_eq!(fit_zoom(1401), (1, 2));
        // ON the second multiple vs one DBU over.
        assert_eq!(fit_zoom(2800), (1, 2));
        assert_eq!(fit_zoom(2801), (1, 3));
        // One DBU UNDER the first multiple is still band 1.
        assert_eq!(fit_zoom(1399), (1, 1));
        // Degenerate: zero and negative clamp to den 1 (the
        // constructor-total face — never a zero denominator).
        assert_eq!(fit_zoom(0), (1, 1));
        assert_eq!(fit_zoom(-50), (1, 1));
        // A realistic board face (bm08-class ~1.5M DBU).
        assert_eq!(fit_zoom(1_500_000), (1, 1072));
    }

    /// THE NOTCH-ACCUMULATOR PIN (G1): whole notches only, the
    /// residual carried across frames (a partial notch is a NO-STEP —
    /// the pre-G1 defect applied one FULL x2 step per FRAME); the
    /// sign carries direction; a non-finite total resets the
    /// accumulator (a NaN would otherwise poison every future
    /// notch). Kills the per-frame-consumption mutant and the
    /// truncation-lost-residual mutant.
    #[test]
    fn wheel_notch_steps_faces() {
        // Exactly one notch -> one step, zero residual.
        assert_eq!(wheel_notch_steps(0.0, WHEEL_NOTCH_POINTS), (1, 0.0));
        // A big flick emits MULTIPLE steps and keeps the remainder.
        assert_eq!(wheel_notch_steps(0.0, 120.0), (2, 20.0));
        // The residual carries: 30 held + 30 new = one notch.
        assert_eq!(wheel_notch_steps(30.0, 30.0), (1, 10.0));
        // Sub-notch deltas accumulate to NOTHING (the no-step face).
        assert_eq!(wheel_notch_steps(0.0, 25.0), (0, 25.0));
        // Downward scroll: negative steps, negative residual carried.
        assert_eq!(wheel_notch_steps(0.0, -75.0), (-1, -25.0));
        assert_eq!(wheel_notch_steps(-25.0, -30.0), (-1, -5.0));
        // Non-finite total: reset, never a poisoned accumulator.
        assert_eq!(wheel_notch_steps(f32::NAN, 10.0), (0, 0.0));
        assert_eq!(wheel_notch_steps(0.0, f32::INFINITY), (0, 0.0));
        // Pathological magnitude: the step count saturates at the i32
        // faces (a saturated `as` cast) rather than wrapping — no
        // panic, and the residual stays finite.
        let (steps, residual) = wheel_notch_steps(0.0, f32::MAX);
        assert_eq!(steps, i32::MAX);
        assert!(residual.is_finite());
    }

    /// THE ANCHOR PIN, In arm (G1): after ONE In-step with the pan
    /// re-anchored at the cursor, the transform's OWN
    /// `world_to_screen` maps the world-under-cursor BACK to the
    /// cursor — EXACTLY, at cursor coordinates on the screen lattice
    /// (`new_num | cursor · new_den`). This is the face the defect
    /// broke: pre-G1 the pan was untouched, so a centered board flew
    /// off-screen one octave per frame.
    #[test]
    fn zoom_about_point_holds_the_anchor_on_the_in_arm() {
        use crate::view::ScreenTransform;
        use epic_geometry::int_point::IntPoint;
        let pan = epic_engine::snapshot::PointPrimitive {
            x: 10_000,
            y: -5_000,
        };
        let cursor = (100, 50);
        let old = (1, 1);
        let new = rational_zoom_step(old.0, old.1, ZoomDirection::In);
        let pan_new = zoom_about_point(pan, cursor, old, new);
        let after = ScreenTransform::new(pan_new, new.0, new.1);
        // The world point that WAS under the cursor before the step.
        let before = ScreenTransform::new(pan, old.0, old.1);
        let world = before.screen_to_world(IntPoint::new(cursor.0, cursor.1));
        let screen_after = after.world_to_screen(world);
        assert_eq!((screen_after.x, screen_after.y), cursor);
        // And the anchor moved the pan (the re-anchoring is not a
        // no-op): cursor 100 at 1/1 -> 2/1 halves its world offset.
        assert_eq!(pan_new.x, pan.x + 100 - 50);
        assert_eq!(pan_new.y, pan.y + 50 - 25);
    }

    /// THE ANCHOR PIN, Out arm (G1): the same exactness face on a
    /// zoom-OUT step (`1/2 -> 1/4` — the door below ratio 1), where
    /// the world offset under the cursor DOUBLES per axis.
    #[test]
    fn zoom_about_point_holds_the_anchor_on_the_out_arm() {
        use crate::view::ScreenTransform;
        use epic_geometry::int_point::IntPoint;
        let pan = epic_engine::snapshot::PointPrimitive { x: 0, y: 250_000 };
        let cursor = (200, 0);
        let old = (1, 2);
        let new = rational_zoom_step(old.0, old.1, ZoomDirection::Out);
        assert_eq!(new, (1, 4)); // the zoom-out door: ratio 1/4
        let pan_new = zoom_about_point(pan, cursor, old, new);
        let before = ScreenTransform::new(pan, old.0, old.1);
        let after = ScreenTransform::new(pan_new, new.0, new.1);
        let world = before.screen_to_world(IntPoint::new(cursor.0, cursor.1));
        assert_eq!(
            (
                after.world_to_screen(world).x,
                after.world_to_screen(world).y
            ),
            cursor
        );
    }

    /// THE DRIFT-BOUND PIN (G1): at a coarse zoom the screen lattice
    /// cannot represent every cursor px — the re-anchored pan's error
    /// at the cursor is bounded by `< new_num` px (one world lattice
    /// step on screen), the documented minimum for ANY integer
    /// transform. Kills the exactness-overshoot mutant (a wrong
    /// truncation face would drift by the RATIO, not one lattice
    /// step).
    #[test]
    fn zoom_about_point_drift_is_bounded_by_one_lattice_step() {
        use crate::view::ScreenTransform;
        use epic_geometry::int_point::IntPoint;
        let pan = epic_engine::snapshot::PointPrimitive {
            x: 1_000_000,
            y: 1_000_000,
        };
        // 2/1 -> 4/1: at 4 px/DBU, screen x=3 has NO exact world
        // point — the best any pan can do is within one lattice step.
        let cursor = (3, 7);
        let old = (2, 1);
        let new = rational_zoom_step(old.0, old.1, ZoomDirection::In);
        assert_eq!(new, (4, 1));
        let pan_new = zoom_about_point(pan, cursor, old, new);
        let before = ScreenTransform::new(pan, old.0, old.1);
        let after = ScreenTransform::new(pan_new, new.0, new.1);
        let world = before.screen_to_world(IntPoint::new(cursor.0, cursor.1));
        let screen_after = after.world_to_screen(world);
        assert!(
            (screen_after.x - cursor.0).abs() < i32::try_from(new.0).unwrap_or(i32::MAX),
            "x drift {} must be < new_num {}",
            screen_after.x - cursor.0,
            new.0
        );
        assert!(
            (screen_after.y - cursor.1).abs() < i32::try_from(new.0).unwrap_or(i32::MAX),
            "y drift {} must be < new_num {}",
            screen_after.y - cursor.1,
            new.0
        );
    }

    /// THE SATURATION PIN (G1): pans near the i64 faces with a big
    /// cursor re-anchor SATURATE (clamp) instead of wrapping or
    /// panicking — the shell's zoom stays total on every extreme
    /// input (the AM4 narrow-face twin).
    #[test]
    fn zoom_about_point_saturates_at_the_i64_faces() {
        let at_max = epic_engine::snapshot::PointPrimitive {
            x: i64::MAX - 1,
            y: i64::MIN + 1,
        };
        let cursor = (10, -10);
        let old = (1, 1);
        let new = (2, 1);
        let pan_new = zoom_about_point(at_max, cursor, old, new);
        // x: MAX-1 + 10 world (i128-fine) - 5 = MAX+4 -> clamps to MAX.
        assert_eq!(pan_new.x, i64::MAX);
        // y: MIN+1 - 10 + 5 = MIN-4 -> clamps to MIN.
        assert_eq!(pan_new.y, i64::MIN);
        // The degenerate zero/negative factors clamp to 1 (the
        // constructor-total twin — never a zero divisor); both pairs
        // clamping to 1/1 makes the step the IDENTITY, so the
        // re-anchor is a fixpoint at pan.
        let pan = epic_engine::snapshot::PointPrimitive { x: 5, y: 5 };
        let sane = zoom_about_point(pan, (4, 4), (0, 0), (0, 0));
        assert_eq!((sane.x, sane.y), (pan.x, pan.y));
    }

    /// THE SMOKE OVERLAY PIN (fix-round F1): all four toggles ON
    /// (kills a flag-dropped-at-launch mutant — the evidence PNG must
    /// carry the toggled-ON families, not just the checkboxes).
    #[test]
    fn smoke_overlay_flags_are_all_on() {
        let flags = smoke_overlay_flags();
        assert!(flags.ratsnest && flags.drc && flags.congestion && flags.tuning);
    }

    /// The launch parser's faces: empty argv is Interactive; the two
    /// smoke shapes parse; missing required flags are Err (never a
    /// panic); an unknown mode is Err.
    #[test]
    fn parse_launch_args_faces() {
        let arg = |s: &str| s.to_string();
        assert_eq!(
            parse_launch_args(std::iter::empty()).unwrap_or_else(|e| panic!("{e}")),
            LaunchMode::Interactive
        );
        let smoke = parse_launch_args(
            [
                "--smoke",
                "board.dsn",
                "--frames",
                "12",
                "--out",
                "f.png",
                "--route",
            ]
            .iter()
            .map(|s| arg(s)),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            smoke,
            LaunchMode::Smoke {
                dsn: PathBuf::from("board.dsn"),
                frames: 12,
                out: PathBuf::from("f.png"),
                route: true,
            }
        );
        let cancel = parse_launch_args(["--cancel-smoke", "b.dsn"].iter().map(|s| arg(s)))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            cancel,
            LaunchMode::CancelSmoke {
                dsn: PathBuf::from("b.dsn")
            }
        );
        assert!(parse_launch_args(["--smoke"].iter().map(|s| arg(s))).is_err());
        assert!(parse_launch_args(["--smoke", "b.dsn"].iter().map(|s| arg(s))).is_err());
        assert!(parse_launch_args(["--wat"].iter().map(|s| arg(s))).is_err());
        // Q7: the non-numeric --frames value is Err (the parse face).
        assert!(
            parse_launch_args(
                ["--smoke", "b.dsn", "--frames", "soon", "--out", "f.png"]
                    .iter()
                    .map(|s| arg(s))
            )
            .is_err()
        );
        // Q7: --cancel-smoke with an extra arg is Err (the exact-arity
        // face).
        assert!(
            parse_launch_args(["--cancel-smoke", "b.dsn", "extra"].iter().map(|s| arg(s))).is_err()
        );
        // The M10-T6 version arm: `--version` parses to
        // LaunchMode::Version; extra args are Err (the exact-arity
        // face, the same shape as --cancel-smoke's).
        assert_eq!(
            parse_launch_args(["--version"].iter().map(|s| arg(s)))
                .unwrap_or_else(|e| panic!("{e}")),
            LaunchMode::Version
        );
        assert!(parse_launch_args(["--version", "extra"].iter().map(|s| arg(s))).is_err());
    }

    /// The M10-T6 `--version` pin (AM5 lesson 14): the version line is
    /// EQUALITY against the DERIVED form
    /// `format!("epicrouter {}", env!("CARGO_PKG_VERSION"))` — never a
    /// bare `epicrouter 2.0.0` literal, which alone survives the
    /// hardcode mutant (the literal drifts from a hardcoded arm only
    /// at the next bump; the derived form dies there — the T5 witness
    /// at `logs/M10-T5/` review evidence 61). The 2.0.0 checkpoint
    /// assert rides beneath it, the same shape as the CLI's
    /// `version_pin.rs`. Census-pinnability: this test module is
    /// UNGATED (the shell.rs module law), so the pin is IN the
    /// workspace census — it pins the pure `version_line` source the
    /// bin's `--version` arm prints.
    #[test]
    fn version_line_equals_the_package_version_face() {
        assert_eq!(
            version_line(),
            format!("epicrouter {}", env!("CARGO_PKG_VERSION")),
            "the version line must be single-sourced from the workspace manifest"
        );
        assert!(
            version_line().contains("epicrouter 2.0.0"),
            "the 2.0.0 checkpoint: the line must carry `epicrouter 2.0.0`: {}",
            version_line()
        );
    }
}
