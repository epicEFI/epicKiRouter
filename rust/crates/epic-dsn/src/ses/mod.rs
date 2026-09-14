//! The SES writer half of `io.specctra` (M1b Task 13): [`writer`] ports
//! `SesWriter.java` — session emission from a parse-time [`SesBoard`].
//!
//! The DSN reader lives in the sibling flat modules; `ses/` exists as a
//! directory because the plan (`:93`, `:282`) names `ses/writer.rs`
//! explicitly and the writer is a self-contained producer (nothing in
//! the reader depends on it).

pub mod writer;
