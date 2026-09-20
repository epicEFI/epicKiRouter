//! `epic-cli route -de board.dsn -do out.ses` — the headless
//! DSN -> route -> SES + manifest pipeline (M3-T13).
//!
//! - [`settings`]: the settings subset resolver, Default(0) ->
//!   DsnFile(20) -> Cli(60), plus the CLI parse surface
//!   (`CliSettings.java` port).
//! - [`route`]: the flow (epic-dsn read -> epic-board build +
//!   import normalize -> T12 batch driver -> SES writer -> result
//!   JSON) and the manifest emitter.

pub mod route;
pub mod settings;
