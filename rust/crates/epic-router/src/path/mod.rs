//! Java `autoroute/path` — the found-route description layer. T7 ports
//! the [`Connection`] walk (the route segment between two forks);
//! the locators land with T9 and the inserter with T11.

pub mod connection;

pub use connection::Connection;
