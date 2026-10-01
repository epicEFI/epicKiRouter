//! The M9-T6 desktop shell (desktop-gated): the THIN eframe host —
//! worker thread, channels, canvas, panels. No geometry logic lives
//! here (the pure faces are `epic_gui::{view, render, shell}`, the
//! census pins exercise those); the shell composes them and adds
//! ONLY presentation + plumbing.
//!
//! The AM5 decisions, operationalized:
//!
//! * the GUI drain loop is the latest-wins coalescing
//!   ([`crate::shell::coalesce_latest_wins`], pinned) — the
//!   worker never blocks the GUI;
//! * overlays come ONLY from `snapshot_with_overlays` (the worker's
//!   `Attached` messages); tee snapshots carry the EMPTY overlay
//!   default by the purity law and are spliced accordingly
//!   ([`crate::shell::splice_overlays`], pinned);
//! * `MarkerPhase` is DATA (both phases render identically — the T5
//!   documented limitation);
//! * the DRC panel shows the live depth total (the Q3 adoption);
//! * the interactive shell never runs unattended: the only
//!   self-exiting modes are the bounded smoke faces
//!   ([`crate::shell::LaunchMode`]).
//!
//! The screenshot face (PROBE RESULT, the charter's evidence face
//! (e)): eframe 0.35 ships exactly ONE pixel-readback face — the
//! `__screenshot` feature's glow readback (`eframe-0.35.0/src/native
//! /glow_integration.rs:1641`, `save_screenshot_and_exit`, reading
//! `egui_glow::Painter::read_screen_rgba`; its `Frame::gl` docs
//! bless the glow readback face). The wgpu backend's hook asserts
//! "not yet implemented" (`wgpu_integration.rs:117-120`). The smoke
//! face therefore rides a DEV-BOX-ONLY cargo feature `smoke` =
//! `["eframe/glow", "eframe/__screenshot"]` (never in CI,
//! default-off), the run selects `Renderer::Glow`, and the PNG is
//! written by EFRAME'S OWN HOOK (its `image` dep) — the raw
//! `Frame::gl()`/`glow::Context::read_pixels` faces were attempted
//! FIRST and FALSIFIED (both post-swap planes read all-black on this
//! driver; the attempts are recorded in `screenshot.rs`'s module
//! docs). The chartered `png` dep stays resolved behind the feature
//! letter but is currently unused (the fix-round F4 truth; the T6
//! report's deviation 3).

pub(crate) mod canvas;
pub(crate) mod panels;
pub(crate) mod worker;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;

use crate::shell::{GuiToWorker, LaunchMode, WorkerToGui, coalesce_latest_wins, splice_overlays};
use eframe::egui;
use epic_engine::events::{ENGINE_EVENT_STREAM_VERSION, EngineEvent};
use epic_engine::session::RouteSummary;
use epic_engine::snapshot::{BoardSnapshot, OverlayData};

/// The recent engine-message log cap (the panel's memory bound —
/// the unbounded tee stream would otherwise grow forever; 200 rows
/// is a screenful of scrollback, a documented display face).
const MESSAGE_LOG_CAP: usize = 200;

/// Runs the shell in `mode`; returns the process exit code. The
/// smoke faces self-exit; Interactive runs until the window closes.
pub fn run(mode: LaunchMode) -> Result<i32, String> {
    #[cfg(not(feature = "smoke"))]
    if matches!(mode, LaunchMode::Smoke { .. }) {
        return Err("the --smoke PNG face needs the dev-box `smoke` feature \
                    (eframe's glow readback; wgpu has none) — rebuild with \
                    --features desktop,smoke"
            .to_string());
    }
    let exit = Arc::new(Mutex::new(None::<i32>));
    // Mutated only under the smoke feature (the glow renderer
    // selection) — the allow keeps the plain-desktop face clean.
    #[cfg_attr(not(feature = "smoke"), allow(unused_mut))]
    let mut options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]),
        ..Default::default()
    };
    #[cfg(feature = "smoke")]
    if let LaunchMode::Smoke { out, .. } = &mode {
        // The ONLY renderer with a pixel-readback face (the module
        // docs' probe result) AND the ONLY correct PNG face: eframe's
        // own `__screenshot` hook (pre-swap read_screen_rgba; the raw
        // glow planes read black on this driver — the falsified
        // attempts are recorded in screenshot.rs's module docs).
        options.renderer = eframe::Renderer::Glow;
        screenshot::arm_hook(out)?;
    }
    let exit_for_app = Arc::clone(&exit);
    eframe::run_native(
        "EpicRouter",
        options,
        Box::new(move |cc| {
            let app = ShellApp::new(mode, exit_for_app, cc.egui_ctx.clone())?;
            Ok(Box::new(app))
        }),
    )
    .map_err(|error| format!("eframe: {error}"))?;
    match exit.lock() {
        Ok(mut code) => Ok(code.take().unwrap_or(0)),
        Err(_) => Err("the exit-code mutex was poisoned".to_string()),
    }
}

/// The shell application state (one per process).
struct ShellApp {
    /// THE view state — pan/zoom/visibility/overlays mutate ONLY
    /// this (the renders-never-mutates law's GUI face).
    view: crate::view::ViewModel,
    /// The egui Context handle (the Q1 liveness face: `send` pings
    /// the repaint clock so an idle reactive-mode UI drains the
    /// worker's reply).
    egui_ctx: egui::Context,
    /// Worker -> GUI messages.
    rx: std::sync::mpsc::Receiver<WorkerToGui>,
    /// GUI -> worker commands.
    cmd_tx: Sender<GuiToWorker>,
    /// The displayed snapshot (any source; overlays spliced per the
    /// AM5 law).
    display: Option<BoardSnapshot>,
    /// The LAST attached overlays (the ONLY overlay source).
    attached_overlays: Option<OverlayData>,
    /// The live depth total (the Q3 adoption).
    depth_total: Option<i64>,
    /// Per-phase last counters row (the progress panel).
    progress: BTreeMap<String, (i32, String)>,
    /// The last TaskState row.
    task_state: Option<(String, i32, String)>,
    /// The capped engine-message log.
    message_log: Vec<(String, String)>,
    /// The load face.
    load_report: Option<crate::shell::LoadReport>,
    load_error: Option<String>,
    /// Whether a route is mid-run.
    route_running: bool,
    /// The finished route's summary face.
    route_done: Option<Result<RouteSummary, String>>,
    /// The finished export's face.
    export_status: Option<Result<(), String>>,
    /// The stages seen (StageEntered order — a Vec keeps first-sight
    /// order for the panel).
    stages: Vec<String>,
    /// A stream-version mismatch (the worker's protocol discipline).
    protocol_error: Option<String>,
    /// The last frame's op count (the status line).
    op_count: usize,
    /// The smoke face (None in Interactive).
    smoke: Option<Smoke>,
    /// The shared exit code (set before the viewport closes).
    exit: Arc<Mutex<Option<i32>>>,
}

/// The bounded smoke state machine (the evidence face; never a
/// gate). Plain data — the per-frame step CLONES it, drives the
/// logic against a local copy, and writes the copy back (the
/// worker-command sends need `&mut self`, which a held
/// `self.smoke` borrow would block).
#[derive(Clone)]
struct Smoke {
    cancel: bool,
    dsn: PathBuf,
    frames_total: u32,
    frames_done: u32,
    route: bool,
    load_sent: bool,
    route_sent: bool,
    cancel_sent: bool,
    pass_seen: bool,
    export_probe: PathBuf,
    export_probe_sent: bool,
    finished: bool,
    failures: Vec<String>,
}

impl Smoke {
    fn new(mode: &LaunchMode) -> Self {
        match mode {
            LaunchMode::Smoke {
                dsn, frames, route, ..
            } => Self {
                cancel: false,
                dsn: dsn.clone(),
                frames_total: *frames,
                frames_done: 0,
                route: *route,
                load_sent: false,
                route_sent: false,
                cancel_sent: false,
                pass_seen: false,
                export_probe: PathBuf::from("smoke-export-should-not-exist.ses"),
                export_probe_sent: false,
                finished: false,
                failures: Vec::new(),
            },
            LaunchMode::CancelSmoke { dsn } => Self {
                cancel: true,
                dsn: dsn.clone(),
                frames_total: 0,
                frames_done: 0,
                route: true,
                load_sent: false,
                route_sent: false,
                cancel_sent: false,
                pass_seen: false,
                export_probe: PathBuf::from("smoke-export-should-not-exist.ses"),
                export_probe_sent: false,
                finished: false,
                failures: Vec::new(),
            },
            LaunchMode::Interactive => unreachable("interactive has no smoke state"),
            // The bin answers `--version` BEFORE `run` (the headless
            // version face) — this arm exists for match totality only.
            LaunchMode::Version => unreachable("version has no smoke state"),
        }
    }
}

/// `unreachable!` spelled through a helper so the panic text stays
/// single-sourced (no unwrap/expect anywhere; this is the one
/// intentionally-total face: a smoke state cannot exist for
/// Interactive by construction).
fn unreachable(what: &str) -> ! {
    panic!("unreachable: {what}")
}

impl ShellApp {
    fn new(
        mode: LaunchMode,
        exit: Arc<Mutex<Option<i32>>>,
        egui_ctx: egui::Context,
    ) -> Result<Self, String> {
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<GuiToWorker>();
        let (event_tx, event_rx) = std::sync::mpsc::channel::<WorkerToGui>();
        worker::spawn(cmd_rx, event_tx)?;
        // The view: no layers visible until the first attach; pan
        // anchors at the world origin until the first snapshot sets
        // it (the first-display face below).
        let mut view = crate::view::ViewModel::new(crate::view::ScreenTransform::new(
            epic_engine::snapshot::PointPrimitive { x: 0, y: 0 },
            1,
            1,
        ));
        let smoke = match &mode {
            LaunchMode::Interactive => None,
            other => Some(Smoke::new(other)),
        };
        // F1: the smoke launch renders the overlay FAMILIES (all four
        // toggles ON — the pure face + its honest-absence note in
        // shell.rs), not just the toggle controls.
        if smoke.is_some() {
            view.overlays = crate::shell::smoke_overlay_flags();
        }
        Ok(Self {
            view,
            egui_ctx,
            rx: event_rx,
            cmd_tx,
            display: None,
            attached_overlays: None,
            depth_total: None,
            progress: BTreeMap::new(),
            task_state: None,
            message_log: Vec::new(),
            load_report: None,
            load_error: None,
            route_running: false,
            route_done: None,
            export_status: None,
            stages: Vec::new(),
            protocol_error: None,
            op_count: 0,
            smoke,
            exit,
        })
    }

    fn send(&mut self, command: GuiToWorker) {
        if self.cmd_tx.send(command).is_err() {
            self.protocol_error = Some("the worker thread is gone (channel closed)".to_string());
        } else {
            // Q1 liveness: a worker reply (LoadResult/Attached/…) may
            // arrive while no route is running and no smoke is active
            // — the 16ms cadence only fires while
            // `route_running || smoke_is_active()`, so an idle
            // reactive-mode UI would never drain the reply. One
            // 100ms ping per issued command closes that gap
            // (imperceptible; one ping per click, not a poll loop).
            self.egui_ctx
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    /// The frame drain: try_recv to Empty, then the AM5
    /// latest-wins coalescing, then in-order processing.
    fn drain(&mut self) {
        let mut batch = Vec::new();
        while let Ok(message) = self.rx.try_recv() {
            batch.push(message);
        }
        for message in coalesce_latest_wins(batch) {
            self.process(message);
        }
    }

    fn process(&mut self, message: WorkerToGui) {
        match message {
            WorkerToGui::Engine(event) => self.process_engine(event),
            WorkerToGui::Attached(snapshot) => {
                self.attached_overlays = Some(snapshot.overlays.clone());
                let revision = snapshot.revision;
                let bounds = snapshot.bounds;
                let snapshot = splice_overlays(snapshot, None);
                let first_display = self.display.is_none();
                self.display = Some(snapshot);
                if first_display {
                    self.initialize_view_for(revision, bounds);
                }
            }
            WorkerToGui::DepthTotal(depth) => self.depth_total = Some(depth),
            WorkerToGui::RouteDone(result) => {
                self.route_running = false;
                self.route_done = Some(result);
            }
            WorkerToGui::LoadResult(result) => match result {
                Ok(report) => {
                    self.load_error = None;
                    self.load_report = Some(report);
                }
                Err(text) => {
                    self.load_report = None;
                    self.load_error = Some(text);
                }
            },
            WorkerToGui::ExportResult(result) => self.export_status = Some(result),
        }
    }

    /// The first-display face: anchor the board's lower-left at the
    /// canvas origin (pan = bounds.ll), fit the whole board into the
    /// initial viewport, and make every snapshot layer visible (the
    /// caller populates — there is no implicit all-layers view).
    fn initialize_view_for(&mut self, _revision: u64, bounds: epic_engine::snapshot::BoxPrimitive) {
        self.view.transform.pan = epic_engine::snapshot::PointPrimitive {
            x: bounds.ll_x,
            y: bounds.ll_y,
        };
        // The fit zoom: the pure face hoisted to `shell::fit_zoom`
        // (the Q2 census-pinnability hoist — the DNR-19 derivation
        // lives on the hoisted fn, pinned in shell.rs).
        let longest = (bounds.ur_x - bounds.ll_x)
            .max(bounds.ur_y - bounds.ll_y)
            .max(1);
        (self.view.transform.zoom_num, self.view.transform.zoom_den) =
            crate::shell::fit_zoom(longest);
        if let Some(display) = &self.display {
            for layer in panels::present_layers(display) {
                self.view.visible_layers.insert(layer);
            }
        }
    }

    fn process_engine(&mut self, event: EngineEvent) {
        match event {
            EngineEvent::StreamStarted { version } => {
                if version != ENGINE_EVENT_STREAM_VERSION {
                    self.protocol_error = Some(format!(
                        "engine event stream version {version} != the compiled \
                         {ENGINE_EVENT_STREAM_VERSION}"
                    ));
                }
            }
            EngineEvent::BoardLoaded { .. } => {
                // The worker's LoadResult covers the load face.
            }
            EngineEvent::StageEntered { stage } => {
                if !self.stages.contains(&stage) {
                    self.stages.push(stage);
                }
            }
            EngineEvent::PassProgress {
                pass,
                phase,
                counters_summary,
            } => {
                self.progress.insert(phase, (pass, counters_summary));
                if let Some(smoke) = &mut self.smoke {
                    smoke.pass_seen = true;
                }
            }
            EngineEvent::TaskState { state, pass, hash } => {
                self.task_state = Some((state, pass, hash));
            }
            EngineEvent::Snapshot { snapshot, .. } => {
                // The AM5 law: tee snapshots carry the EMPTY overlay
                // default — the attached overlays (if any) ride on
                // top (the splice, pinned). Disjoint field borrows.
                let attached = self.attached_overlays.as_ref();
                self.display = Some(splice_overlays(snapshot, attached));
            }
            EngineEvent::RoutingFinished { .. } => {
                // The worker's RouteDone covers the finish face.
            }
            EngineEvent::Message { level, text } => {
                self.message_log.push((level, text));
                if self.message_log.len() > MESSAGE_LOG_CAP {
                    let excess = self.message_log.len() - MESSAGE_LOG_CAP;
                    self.message_log.drain(0..excess);
                }
            }
        }
    }
}

/// The smoke machine's per-step verdict.
enum SmokeAction {
    /// Keep stepping (next frame).
    Wait,
    /// The state machine concluded (ok = the failures list is
    /// empty).
    Finish,
    /// The frames budget is spent: take the screenshot NOW (the
    /// caller holds the `&eframe::Frame`).
    Shoot,
}

impl ShellApp {
    /// The smoke state machine's per-frame step (the evidence face,
    /// never a gate).
    fn smoke_step(&mut self, ctx: &egui::Context) {
        let finished = self
            .smoke
            .as_ref()
            .map(|smoke| smoke.finished)
            .unwrap_or(true);
        if finished {
            return;
        }
        let mut action = SmokeAction::Wait;
        {
            // The clone-drive-writeback dance (see the struct docs):
            // the step logic mutates a LOCAL copy, the self-touching
            // faces (send / route / load state) stay free.
            let mut smoke = self
                .smoke
                .clone()
                .unwrap_or_else(|| unreachable("smoke vanished"));
            if !smoke.load_sent {
                smoke.load_sent = true;
                self.send(GuiToWorker::LoadPath(smoke.dsn.clone()));
            } else if self.load_report.is_none() {
                if let Some(text) = self.load_error.clone() {
                    smoke.failures.push(format!("load failed: {text}"));
                    action = SmokeAction::Finish;
                }
            } else if smoke.cancel {
                action = self.cancel_smoke_step(&mut smoke);
            } else {
                action = self.frames_smoke_step(&mut smoke);
            }
            self.smoke = Some(smoke);
        }
        match action {
            SmokeAction::Wait => {}
            SmokeAction::Shoot => {
                // The PNG face is eframe's own hook (armed at run());
                // it fires at egui pass 2 and process-exits BEFORE
                // this arm — reaching Shoot at all means the hook did
                // NOT fire (a non-glow face, or pass counting moved):
                // that is a smoke FAILURE with the honest text.
                if let Some(smoke) = self.smoke.as_mut() {
                    smoke.failures.push(
                        "the eframe __screenshot hook did not fire (the PNG face is its pass-2 exit, not this path)"
                            .to_string(),
                    );
                }
                self.finish_smoke(ctx);
            }
            SmokeAction::Finish => self.finish_smoke(ctx),
        }
    }

    /// The cancel smoke: start the route, cancel at the first
    /// PassProgress, assert CANCELLED + no export written.
    fn cancel_smoke_step(&mut self, smoke: &mut Smoke) -> SmokeAction {
        if !smoke.route_sent {
            smoke.route_sent = true;
            self.send(GuiToWorker::StartRoute(
                epic_engine::settings::CliLayer::default(),
            ));
            self.route_running = true;
            return SmokeAction::Wait;
        }
        if smoke.pass_seen && !smoke.cancel_sent {
            smoke.cancel_sent = true;
            eprintln!("epic-gui cancel-smoke: first PassProgress seen — raising the stop flag");
            if let Some(report) = &self.load_report {
                report.stop_flag.store(true, Ordering::Relaxed);
            }
            return SmokeAction::Wait;
        }
        let Some(result) = &self.route_done else {
            return SmokeAction::Wait;
        };
        let summary = match result {
            Err(text) => {
                smoke.failures.push(format!("route failed: {text}"));
                return SmokeAction::Finish;
            }
            Ok(summary) => summary,
        };
        if summary.final_state != "CANCELLED" {
            smoke.failures.push(format!(
                "final_state is {} after a mid-run cancel (wanted CANCELLED)",
                summary.final_state
            ));
        }
        // The export-gate probe: a CANCELLED session must refuse the
        // export AND write nothing.
        if !smoke.export_probe_sent {
            smoke.export_probe_sent = true;
            self.send(GuiToWorker::ExportSes(smoke.export_probe.clone()));
            return SmokeAction::Wait;
        }
        let Some(export) = &self.export_status else {
            return SmokeAction::Wait;
        };
        if export.is_ok() {
            smoke
                .failures
                .push("the export gate ACCEPTED a CANCELLED session".to_string());
        }
        if smoke.export_probe.exists() {
            smoke.failures.push(format!(
                "an export file was written despite CANCELLED: {}",
                smoke.export_probe.display()
            ));
        }
        eprintln!(
            "epic-gui cancel-smoke: final_state={} depth_total={:?} probe={} exists={} \
             export_refused={}",
            summary.final_state,
            self.depth_total,
            smoke.export_probe.display(),
            smoke.export_probe.exists(),
            export.is_err(),
        );
        SmokeAction::Finish
    }

    /// The frames smoke: run N frames at the fixed dt, then the glow
    /// readback -> PNG (the Shoot verdict).
    fn frames_smoke_step(&mut self, smoke: &mut Smoke) -> SmokeAction {
        if smoke.route && !smoke.route_sent {
            // HOLD PASS 1 OPEN until the first attach (bounded): the
            // hook screenshots at pass 2, so the load + the route
            // start must land inside pass 1 for the shot to show the
            // workflow (screenshot.rs module docs).
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while self.display.is_none() && std::time::Instant::now() < deadline {
                self.drain();
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            if self.display.is_none() {
                smoke
                    .failures
                    .push("the first attach never landed (30 s hold)".to_string());
                return SmokeAction::Finish;
            }
            smoke.route_sent = true;
            self.send(GuiToWorker::StartRoute(
                epic_engine::settings::CliLayer::default(),
            ));
            self.route_running = true;
            return SmokeAction::Wait;
        }
        smoke.frames_done += 1;
        if smoke.frames_done < smoke.frames_total {
            return SmokeAction::Wait;
        }
        SmokeAction::Shoot
    }

    /// Concludes the smoke: the failures list IS the verdict; log
    /// lines + the shared exit code + close the viewport.
    fn finish_smoke(&mut self, ctx: &egui::Context) {
        let (failures, ok) = match self.smoke.as_ref() {
            Some(smoke) => (smoke.failures.clone(), smoke.failures.is_empty()),
            None => (Vec::new(), false),
        };
        eprintln!("epic-gui smoke: {}", if ok { "OK" } else { "FAILED" });
        for failure in &failures {
            eprintln!("epic-gui smoke FAIL: {failure}");
        }
        if let Ok(mut code) = self.exit.lock() {
            *code = Some(if ok { 0 } else { 1 });
        }
        if let Some(smoke) = self.smoke.as_mut() {
            smoke.finished = true;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

impl eframe::App for ShellApp {
    // eframe 0.35's App face: `ui` hands the shell a bare Ui (the
    // old `update(ctx, frame)` is gone); the shell pulls the Context
    // off it and lays out its own panels.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.drain();
        self.smoke_step(&ctx);
        // Panels chain through the passed Ui (the 0.35 Panel face —
        // `show` takes `&mut Ui`, not the Context).

        // The status bar.
        egui::Panel::top("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                let input = self
                    .load_report
                    .as_ref()
                    .map(|report| report.input_name.clone())
                    .unwrap_or_else(|| "(no board)".to_string());
                ui.label(format!("input: {input}"));
                if self.route_running {
                    ui.label("ROUTING");
                }
                if let Some(done) = &self.route_done {
                    match done {
                        Ok(summary) => ui.label(format!(
                            "route: {} incomplete {} violations {}",
                            summary.final_state, summary.incomplete_count, summary.violations_total
                        )),
                        Err(text) => ui.label(format!("route error: {text}")),
                    };
                }
                ui.label(format!("ops: {}", self.op_count));
                if let Some(text) = &self.protocol_error {
                    ui.colored_label(egui::Color32::RED, text);
                }
                if let Some(text) = &self.load_error {
                    ui.colored_label(egui::Color32::RED, format!("load: {text}"));
                }
            });
        });

        // The control panel.
        egui::Panel::right("controls").show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                if ui.button("Load DSN...").clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("DSN", &["dsn"])
                        .pick_file()
                {
                    self.send(GuiToWorker::LoadPath(path));
                }
                let routed = self.route_done.is_some() || self.route_running;
                ui.add_enabled_ui(self.load_report.is_some() && !routed, |ui| {
                    if ui.button("Route (defaults)").clicked() {
                        self.route_running = true;
                        self.route_done = None;
                        self.send(GuiToWorker::StartRoute(
                            epic_engine::settings::CliLayer::default(),
                        ));
                    }
                });
                ui.add_enabled_ui(self.route_running, |ui| {
                    if ui.button("Cancel").clicked() {
                        // The MID-RUN face: the stop-flag handle (the
                        // worker is inside Session::route and cannot
                        // dequeue commands — worker.rs module docs).
                        if let Some(report) = &self.load_report {
                            report.stop_flag.store(true, Ordering::Relaxed);
                        }
                        self.send(GuiToWorker::Cancel);
                    }
                });
                ui.add_enabled_ui(self.route_done.is_some(), |ui| {
                    if ui.button("Export SES...").clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .add_filter("SES", &["ses"])
                            .set_file_name("routed.ses")
                            .save_file()
                    {
                        self.send(GuiToWorker::ExportSes(path));
                    }
                });
                if let Some(status) = &self.export_status {
                    match status {
                        Ok(()) => ui.label("export: written"),
                        Err(text) => {
                            ui.colored_label(egui::Color32::RED, format!("export: {text}"))
                        }
                    };
                }
                ui.separator();
                if let Some(display) = &self.display {
                    panels::layers(ui, display, &mut self.view);
                    ui.separator();
                }
                panels::overlays(ui, &mut self.view);
                ui.separator();
                panels::progress(ui, &self.progress, &self.task_state, self.route_running);
                if !self.stages.is_empty() {
                    ui.label(format!("stages: {}", self.stages.join(" -> ")));
                }
                ui.separator();
                panels::drc(ui, self.depth_total);
                ui.separator();
                let (name, missing, warnings): (Option<String>, bool, Vec<String>) =
                    match &self.load_report {
                        Some(report) => (
                            Some(report.input_name.clone()),
                            report.outline_missing,
                            report.warnings.clone(),
                        ),
                        None => (None, false, Vec::new()),
                    };
                panels::settings(ui, name.as_deref(), missing, &warnings);
                ui.separator();
                panels::message_log(ui, &self.message_log);
            });
        });

        // The canvas.
        let canvas_response = egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                let (rect, response) =
                    ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
                if let Some(display) = &self.display {
                    self.op_count = canvas::paint(&ui.painter().clone(), rect, display, &self.view);
                } else {
                    ui.painter()
                        .rect_filled(rect, 0.0, color_of_background(&self.view));
                }
                response
            })
            .inner;

        // Pan: pointer drag mutating ONLY transform.pan (the world
        // point under the cursor follows the cursor).
        if canvas_response.dragged() {
            let delta = canvas_response.drag_delta();
            let num = self.view.transform.zoom_num;
            let den = self.view.transform.zoom_den;
            self.view.transform.pan.x -= (delta.x as f64 * den as f64 / num as f64) as i64;
            self.view.transform.pan.y -= (delta.y as f64 * den as f64 / num as f64) as i64;
        }
        // Zoom: wheel -> the rational step (x2 // /2 on the ratio;
        // the factors stay >= 1 — the zoom-out door stays open).
        if canvas_response.hovered() {
            let scroll = ctx.input(|input| input.smooth_scroll_delta.y);
            if scroll != 0.0 {
                let direction = if scroll > 0.0 {
                    crate::shell::ZoomDirection::In
                } else {
                    crate::shell::ZoomDirection::Out
                };
                let transform = self.view.transform;
                let (zoom_num, zoom_den) = crate::shell::rational_zoom_step(
                    transform.zoom_num,
                    transform.zoom_den,
                    direction,
                );
                self.view.transform.zoom_num = zoom_num;
                self.view.transform.zoom_den = zoom_den;
            }
        }

        // Repaint while the engine is active (the drain cadence).
        if self.route_running || self.smoke_is_active() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }
}

impl ShellApp {
    fn smoke_is_active(&self) -> bool {
        self.smoke
            .as_ref()
            .map(|smoke| !smoke.finished)
            .unwrap_or(false)
    }
}

/// The empty-canvas backdrop (the color table's background).
fn color_of_background(view: &crate::view::ViewModel) -> egui::Color32 {
    let b = view.colors.background;
    egui::Color32::from_rgba_unmultiplied(b[0], b[1], b[2], b[3])
}

// The glow readback face (dev-box evidence; the module-docs probe
// result names it the only screenshot face eframe 0.35 ships).
#[cfg(feature = "smoke")]
pub(crate) mod screenshot;
