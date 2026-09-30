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

pub mod settings;

#[cfg(test)]
mod tests {
    /// The crate scaffolded in M0; the settings core landed in M9-T1.
    #[test]
    fn crate_scaffolds() {
        let name = env!("CARGO_PKG_NAME");
        assert_eq!(name, "epic-engine");
    }
}
