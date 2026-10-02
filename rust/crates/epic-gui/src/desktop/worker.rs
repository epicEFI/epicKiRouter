//! The M9-T6 desktop worker thread (desktop-gated; its PROTOCOL types
//! live UNGATED in [`crate::shell`]): a `std::thread` owning the
//! [`Session`]. GUI→worker [`GuiToWorker`] commands, worker→GUI
//! [`WorkerToGui`] messages. The worker NEVER blocks on the GUI (the
//! tee's event channel is unbounded and the GUI drains at frame
//! cadence with the AM5 latest-wins coalescing), and the GUI never
//! holds the Session (the renders-never-mutates law's host face).
//!
//! The route face runs `Session::route` with a
//! [`TeeDriverSink`] over a small no-op inner sink: the TEE already
//! converts sink rows to [`EngineEvent`]s (the T3 stream law), so the
//! inner sink has nothing to do — it exists to satisfy the `&mut dyn
//! DriverSink` argument, imported through the ONE additive
//! epic-engine re-export so epic-gui's LIB stays epic-router-free.
//!
//! Mid-run CANCEL: the worker thread is INSIDE `Session::route` while
//! a route runs and cannot dequeue commands, so mid-run cancel uses
//! the session's own sanctioned face
//! ([`Session::stop_flag`], whose doc names "the GUI button" as the
//! consumer): the worker ships the `Arc<AtomicBool>` handle in
//! [`LoadReport::stop_flag`], and the GUI stores through it
//! directly. The [`GuiToWorker::Cancel`] command remains the IDLE
//! face (handled between commands; a pre-route raise is the
//! documented pass-through COMPLETED face).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{Receiver, Sender};

use epic_engine::DriverSink;
use epic_engine::events::{EngineEvent, TeeDriverSink};
use epic_engine::session::{LoadError, RouteError, RouteSummary, Session};
use epic_engine::settings::SessionLayer;
use epic_engine::snapshot::BoardSnapshot;

use crate::shell::{GuiToWorker, LoadReport, WorkerToGui};

/// The route's inner sink: the TEE converts every row to an
/// [`EngineEvent`], so this sink is a documented no-op (every trait
/// method defaults — the all-default trait, `event_sink.rs:15-43`).
struct NoopSink;

impl DriverSink for NoopSink {}

/// Spawns the worker thread (one per shell process; it exits when
/// the command channel closes — the GUI dropped).
pub(crate) fn spawn(cmd_rx: Receiver<GuiToWorker>, tx: Sender<WorkerToGui>) -> Result<(), String> {
    std::thread::Builder::new()
        .name("epic-gui-worker".to_string())
        .spawn(move || {
            let mut worker = Worker {
                session: None,
                cmd_rx,
                tx,
                stop_flag: None,
            };
            while let Ok(command) = worker.cmd_rx.recv() {
                worker.handle(command);
            }
        })
        .map(|_join| ())
        .map_err(|error| format!("cannot spawn the worker thread: {error}"))
}

struct Worker {
    session: Option<Session>,
    cmd_rx: Receiver<GuiToWorker>,
    tx: Sender<WorkerToGui>,
    /// The session's stop-flag handle, captured at load (the mid-run
    /// cancel face — the module docs).
    stop_flag: Option<Arc<AtomicBool>>,
}

impl Worker {
    fn handle(&mut self, command: GuiToWorker) {
        match command {
            GuiToWorker::LoadPath(path) => self.load(path),
            GuiToWorker::StartRoute(cli) => self.route(cli),
            GuiToWorker::Cancel => {
                // The IDLE cancel face: raise the flag; a pre-route
                // raise is the pipeline's documented pass-through
                // COMPLETED face (session.rs request_cancel docs).
                if let Some(flag) = &self.stop_flag {
                    flag.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
            GuiToWorker::ExportSes(path) => self.export(path),
            GuiToWorker::Snapshot => self.attach_snapshot(),
            GuiToWorker::InterviewRequest => self.interview_questions(),
        }
    }

    /// The F4 interview face: the session's board-derived questions
    /// (empty when no session is loaded — the GUI renders the empty
    /// face as a status line, never an empty dialog). PURE on the
    /// session (the getter is the load-time model; nothing mutates).
    fn interview_questions(&self) {
        let questions = self
            .session
            .as_ref()
            .map_or_else(Vec::new, Session::interview_questions);
        let _ = self.tx.send(WorkerToGui::InterviewQuestions(questions));
    }

    /// The load face: read the file, `Session::load_dsn` (all four
    /// arms — `OutlineMissing` is the warn-and-continue face, the
    /// board routes on with the default boundary, exactly the CLI's
    /// face), set the input name from the path, then ship
    /// `LoadResult` + the first `Attached` + `DepthTotal`.
    fn load(&mut self, path: PathBuf) {
        let outcome = std::fs::read(&path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))
            .and_then(
                |bytes| match Session::load_dsn(&bytes, SessionLayer::default()) {
                    Ok(session) => Ok((session, false)),
                    Err(LoadError::OutlineMissing(session)) => Ok((*session, true)),
                    Err(LoadError::Parse(detail)) => Err(format!("parse error: {detail}")),
                    Err(LoadError::Io(detail)) => Err(format!("io error: {detail}")),
                },
            );
        let (mut session, outline_missing) = match outcome {
            Ok(loaded) => loaded,
            Err(text) => {
                self.stop_flag = None;
                let _ = self.tx.send(WorkerToGui::LoadResult(Err(text)));
                return;
            }
        };
        // The input name: the file stem (the CLI derives the same
        // face from the -de path; the session gets a path here).
        let input_name = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "board".to_string());
        session.set_input_name(input_name.clone());
        let report = {
            let session = &session;
            LoadReport {
                input_name,
                outline_missing,
                warnings: session.warnings().to_vec(),
                revision: session.board_revision(),
                item_count: session.board().item_count(),
                stop_flag: session.stop_flag(),
            }
        };
        self.session = Some(session);
        self.stop_flag = Some(Arc::clone(&report.stop_flag));
        let _ = self.tx.send(WorkerToGui::LoadResult(Ok(report)));
        self.attach_snapshot();
    }

    /// The route face: the tee over [`NoopSink`], the engine events
    /// forwarded LIVE to the GUI during the run (a scoped forwarder
    /// thread drains the tee's channel while the worker thread is
    /// inside `Session::route`), then `RouteDone` + `Attached` +
    /// `DepthTotal` (the attach cadence). Mid-run CANCEL rides the
    /// stop-flag handle (the module docs) — the command queue drains
    /// after the route returns.
    fn route(&mut self, cli: epic_engine::settings::CliLayer) {
        let Some(session) = self.session.as_mut() else {
            let _ = self.tx.send(WorkerToGui::RouteDone(Err(
                "cannot route: no board loaded".to_string()
            )));
            return;
        };
        let (engine_tx, engine_rx) = std::sync::mpsc::channel::<EngineEvent>();
        let gui_tx = self.tx.clone();
        let result: Result<RouteSummary, RouteError> = std::thread::scope(|scope| {
            let forwarder = scope.spawn(move || {
                for event in engine_rx {
                    if gui_tx.send(WorkerToGui::Engine(event)).is_err() {
                        break; // the GUI dropped — best-effort stream.
                    }
                }
            });
            // Inner block so the tee (and its engine_tx clone) drops
            // BEFORE the scope joins the forwarder — the forwarder's
            // `for` ends on the channel disconnect, and joining it
            // here keeps the scope's implicit join from deadlocking
            // on a still-open channel.
            let result = {
                let mut noop = NoopSink;
                let mut tee = TeeDriverSink::new(&mut noop, engine_tx);
                session.route(&cli, &mut tee)
            };
            let _ = forwarder.join();
            result
        });
        match result {
            Ok(summary) => {
                let _ = self.tx.send(WorkerToGui::RouteDone(Ok(summary)));
                self.attach_snapshot();
            }
            Err(error) => {
                // The Q6 face (M10-T2): re-route stays DISABLED — the
                // already-routed session refuses with the clean error;
                // the board state (and the GUI's last attach) stands.
                let _ = self.tx.send(WorkerToGui::RouteDone(Err(error.to_string())));
            }
        }
    }

    /// The attach step: `snapshot_with_overlays` (the ONE overlay
    /// source — the AM5 law) + the live depth total (the Q3 adoption)
    /// at attach cadence.
    fn attach_snapshot(&mut self) {
        let Some(session) = self.session.as_mut() else {
            // Q3: the silent no-op class leaves a trace (mirroring
            // `route`'s None-arm idiom) — a None session here means a
            // Snapshot command raced a failed load.
            let _ = self.tx.send(WorkerToGui::Engine(EngineEvent::Message {
                level: "warn".to_string(),
                text: "cannot attach: no board loaded".to_string(),
            }));
            return;
        };
        let snapshot: BoardSnapshot = session.snapshot_with_overlays();
        let depth_total = session.clearance_violation_depth_total();
        let _ = self.tx.send(WorkerToGui::Attached(snapshot));
        let _ = self.tx.send(WorkerToGui::DepthTotal(depth_total));
    }

    /// The export face (the session's COMPLETED/TIMED_OUT gate —
    /// CANCELLED refuses, the text carries).
    fn export(&mut self, path: PathBuf) {
        let result = match &mut self.session {
            Some(session) => session.export_ses(&path),
            None => Err("cannot export: no board loaded".to_string()),
        };
        let _ = self.tx.send(WorkerToGui::ExportResult(result));
    }
}
