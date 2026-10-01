//! `epic-cli route -de board.dsn -do out.ses` — the headless
//! DSN -> route -> SES + manifest pipeline (M3-T13).
//!
//! - [`route`]: the flow (epic-dsn read -> epic-board build +
//!   import normalize -> T12 batch driver -> SES writer -> result
//!   JSON) and the manifest emitter. Its settings layering (the
//!   merger, the parse surface, the activation predicates) LIVES in
//!   [`epic_engine::settings`] since the M9-T1 move.
//!
//! The former `settings` module moved to `epic-engine` at M9-T1 (the
//! design §4.1 engine charter); `epic-cli` consumes it through the
//! `epic_engine::settings` path.

pub mod route;
