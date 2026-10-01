//! Java `autoroute/path` — the found-route description layer. T7 ports
//! the [`Connection`] walk (the route segment between two forks); T9
//! ports the found-connection locator family (backtrack walk → trace
//! geometry, corner synthesis) and the inserter lands with T11.

pub mod connection;
pub mod inserter;
pub mod locator;
pub(crate) mod locator_45;
pub(crate) mod locator_any;

pub use connection::Connection;
pub use inserter::{FoundConnectionInserter, InserterEventSink, NullSink};
pub use locator::{FoundConnectionLocator, ResultItem};

#[cfg(test)]
mod pins;
