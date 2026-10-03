//! The batch-autoroute pipeline (M3-T12): the multi-pass driver over
//! the T11 engine — `autoroute/pipeline/BatchAutorouter.java`'s item
//! selection, `AutorouteConnectionRouter`'s per-connection route with
//! the necked retry, `AutoroutePassRunner`'s single-thread pass walk,
//! `AutorouteBatchLoop`'s pass loop (stagnation + restore), and the
//! `BoardStatistics`/score face the loop reads its progress from.

pub mod batch;
pub mod board_hash;
pub mod board_history;
pub mod board_statistics;
pub mod board_statistics_bounds;
pub mod connection_router;
pub mod event_sink;
pub mod fanout;
pub mod full;
pub mod gloss;
pub mod last_mile;
pub mod optimizer;
pub mod pairs;
pub mod pass_runner;
pub mod tuning;
