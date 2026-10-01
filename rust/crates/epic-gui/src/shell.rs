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
