//! The headless application core: jobs, the layered settings
//! (defaults -> JSON -> DSN -> env -> CLI -> GUI with Java
//! SettingsMerger precedence semantics), progress/cancel, seed control
//! (design §4.1; the settings layering MOVED here from epic-cli at
//! M9-T1, the Session/Event core lands in M9-T2/T3).
//!
//! - [`settings`]: the settings layering (Java `SettingsMerger` +
//!   `CliSettings` + `DsnFileSettings` + `DefaultSettings` port): the
//!   `CliLayer`/`DsnLayer`/`MergedSettings` model, `merge` +
//!   `validate` + `apply_board_specific_optimizations` +
//!   `ResolvedRouteSettings::resolve`, the CLI parse surface
//!   (`parse_route_args`), the `ResolvedRouteSettings ->
//!   `BatchSettings` activation predicates (`build_batch_settings`,
//!   the tuning/meander/pairs activations), and the M9-T1
//!   `SessionLayer` slot (`merge_session`, the GUI tri-state layer
//!   merged above the CLI layer — the CLI never constructs it).
//! - [`session`]: the M9-T2 headless workflow (`Session::load_dsn` ->
//!   `route` -> `request_cancel` -> `export_ses` -> `statistics`),
//!   driving the EXACT `run_route` sequence with the moved faces; the
//!   acceptance proof is the harness `session_parity_pin.rs`
//!   byte-identity. Also carries the relocated
//!   `apply_copper_to_edge_clearance_override` (the load-time outline
//!   promotion both host flows share).
//! - [`export`]: the board -> SES projection + the design-name face,
//!   relocated from epic-cli route.rs so BOTH paths call the same
//!   projection/writer code.
//! - [`events`] (M9-T3): the versioned
//!   [`EngineEvent`](events::EngineEvent) stream + the
//!   [`TeeDriverSink`](events::TeeDriverSink) (the host composes the
//!   tee around its own inner sink; the parity stream is untouched).
//! - [`snapshot`] (M9-T3): the pass-granular
//!   [`BoardSnapshot`](snapshot::BoardSnapshot) render projection
//!   (`&Board` read faces only — the renders-never-mutates law).

pub mod current_width;
pub mod drc_tolerance;
pub mod events;
pub mod export;
pub mod interview;
pub mod pin_assign;
pub mod plane_nets;
pub mod pour;
pub mod session;
pub mod settings;
pub mod snapshot;

/// The M9-T6 host re-export: the desktop shell's worker needs the
/// [`DriverSink`] trait to type its inner no-op sink, and epic-gui's
/// LIB must stay epic-router-free (the dev-dependency law — the leaf
/// crate's consumers never grow the router dep). One additive line:
/// `epic-router` is already an epic-engine dependency, so this moves
/// NO code and changes NO behavior.
pub use epic_router::pipeline::event_sink::DriverSink;

#[cfg(test)]
mod tests {
    /// The crate scaffolded in M0; the settings core landed in M9-T1.
    #[test]
    fn crate_scaffolds() {
        let name = env!("CARGO_PKG_NAME");
        assert_eq!(name, "epic-engine");
    }
}
