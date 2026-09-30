//! The M9-T6 side panels (desktop-gated): layer visibility, the four
//! overlay toggles, live progress, the read-only settings display,
//! and the DRC depth-total panel (the Q3 adoption). Each panel binds
//! a slice of the shell state; NOTHING here touches the engine (the
//! renders-never-mutates law) — settings display is READ-ONLY this
//! milestone (a settings-dialog INPUT face is future work).

use std::collections::{BTreeMap, BTreeSet};

use crate::view::ViewModel;
use eframe::egui;
use epic_engine::snapshot::BoardSnapshot;

/// The layers present in a snapshot, ascending.
pub(crate) fn present_layers(snapshot: &BoardSnapshot) -> BTreeSet<i32> {
    let mut present: BTreeSet<i32> = BTreeSet::new();
    for trace in &snapshot.traces {
        present.insert(trace.layer);
    }
    for pad in &snapshot.pads {
        present.insert(pad.layer);
    }
    for via in &snapshot.vias {
        for layer in &via.layers {
            present.insert(*layer);
        }
    }
    for area in &snapshot.areas {
        present.insert(area.layer);
    }
    present
}

/// The layer-visibility panel: one checkbox per layer seen in the
/// snapshot (BTreeSet bind — the view model's own ordering).
pub(crate) fn layers(ui: &mut egui::Ui, snapshot: &BoardSnapshot, view: &mut ViewModel) {
    ui.heading("Layers");
    for layer in present_layers(snapshot) {
        let mut visible = view.visible_layers.contains(&layer);
        if ui
            .checkbox(&mut visible, format!("layer {layer}"))
            .changed()
        {
            if visible {
                view.visible_layers.insert(layer);
            } else {
                view.visible_layers.remove(&layer);
            }
        }
    }
}

/// The four overlay toggles (the OverlayFlags bind).
pub(crate) fn overlays(ui: &mut egui::Ui, view: &mut ViewModel) {
    ui.heading("Overlays");
    let flags = &mut view.overlays;
    ui.checkbox(&mut flags.ratsnest, "ratsnest");
    ui.checkbox(&mut flags.drc, "drc markers");
    ui.checkbox(&mut flags.congestion, "congestion heatmap");
    ui.checkbox(&mut flags.tuning, "tuning bands");
}

/// The live progress panel: per-phase last `PassProgress` counters
/// row + the last `TaskState` row (phase-keyed like the CLI's
/// `last_counters_by_phase`).
pub(crate) fn progress(
    ui: &mut egui::Ui,
    rows: &BTreeMap<String, (i32, String)>,
    task_state: &Option<(String, i32, String)>,
    route_running: bool,
) {
    ui.heading("Progress");
    if route_running {
        ui.label("routing...");
    }
    egui::ScrollArea::vertical()
        .max_height(120.0)
        .show(ui, |ui| {
            for (phase, (pass, summary)) in rows {
                ui.label(format!("{phase} pass {pass}: {summary}"));
            }
            if let Some((state, pass, hash)) = task_state {
                ui.label(format!("task: {state} pass {pass} hash {hash}"));
            }
            if rows.is_empty() && task_state.is_none() {
                ui.label("(no engine activity yet)");
            }
        });
}

/// The DRC panel: the live clearance-violation DEPTH total (the T5
/// fix-round face, adopted as this panel's live face — quality Q3's
/// production consumer; shipped at attach cadence).
pub(crate) fn drc(ui: &mut egui::Ui, depth_total: Option<i64>) {
    ui.heading("DRC");
    match depth_total {
        Some(depth) => ui.label(format!("clearance violation depth total: {depth}")),
        None => ui.label("clearance violation depth total: (not attached yet)"),
    };
}

/// The read-only settings display (tri-state provenance):
/// `SessionLayer`/`CliLayer` ride their DEFAULTS this milestone (a
/// settings-dialog INPUT face is future work), so every field shows
/// its DEFAULT face; the DSN layer is applied at load INSIDE the
/// session (session-internal — no read face exists, and engine
/// changes beyond the one `DriverSink` re-export are out of the T6
/// charter), so its face is named, not shown per-field.
pub(crate) fn settings(
    ui: &mut egui::Ui,
    input_name: Option<&str>,
    outline_missing: bool,
    warnings: &[String],
) {
    ui.heading("Settings (read-only)");
    ui.label(format!("input: {}", input_name.unwrap_or("(none loaded)")));
    ui.label("CLI layer: unset -> defaults");
    ui.label("session layer: defaults");
    if outline_missing {
        ui.label("DSN outline: MISSING (default boundary, the warn-and-continue face)");
    } else {
        ui.label("DSN layer: applied at load (session-internal; per-field display is future work)");
    }
    if !warnings.is_empty() {
        ui.label(format!("load warnings: {}", warnings.len()));
        for warning in warnings {
            ui.label(format!("  {warning}"));
        }
    }
}

/// The recent engine message log (info/warn rows from the tee).
pub(crate) fn message_log(ui: &mut egui::Ui, log: &[(String, String)]) {
    ui.heading("Engine log");
    egui::ScrollArea::vertical()
        .max_height(120.0)
        .show(ui, |ui| {
            for (level, text) in log {
                ui.label(format!("[{level}] {text}"));
            }
            if log.is_empty() {
                ui.label("(no engine messages)");
            }
        });
}
