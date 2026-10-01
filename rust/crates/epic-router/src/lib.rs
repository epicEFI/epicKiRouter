//! The staged router pipeline: fanout/escape, global routing (PathFinder
//! negotiated congestion), detail routing (octagon A* + push-and-shove),
//! plane routing, tuning, gloss (design §4.2).
//!
//! M3-T3 content: [`control`] — the `AutorouteControl` per-net cost
//! table port (`autoroute/maze/AutorouteControl.java`). The T4-T10
//! porting contract (the exact Java board/searchtree/optimize surface
//! with line counts and dispositions) is `SEAM.md` beside this file.

pub mod control;
pub mod drill;
pub mod engine;
pub mod expansion;
pub mod global;
pub mod maze;
pub mod path;
pub mod pipeline;

#[cfg(test)]
pub(crate) mod test_util;
