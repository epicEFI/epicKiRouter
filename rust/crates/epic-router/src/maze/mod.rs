//! Java `autoroute/maze` — the maze search CORE (T6): the totally
//! ordered expansion front ([`list_element`]), the drill dispatch
//! ([`expansion_engine`]), the search engine proper
//! ([`search_engine`]) and the production completion seam composing
//! the T4 primitives ([`completion`]).
//!
//! Boundary map (see `SEAM.md`):
//! * the front is a [`BTreeSet`] replicating Java's
//!   `TreeSet<MazeListElement>` `<`/`>`-semantics — NaN falls through
//!   tie-breaks, -0.0 ties +0.0, comparator-equal adds are dropped
//!   (`list_element` module doc);
//! * ripup (`checkRipup` / `checkLeavingRippedItem`) and shove
//!   (`shoveTraceRoom` / `MazeTraceShover`) are T7 seams stubbed at
//!   their Java-observable no-ripup verdicts;
//! * `DestinationDistance` (T8), `ViaLayerChecker` (T10),
//!   `checkForcedTracePolyline` (T7) and the ripped-via drill
//!   construction (T7) are [`crate::drill::DrillEngine`] /
//!   injected-trait seams.

pub mod completion;
pub mod expansion_engine;
pub mod list_element;
pub mod search_engine;

#[cfg(test)]
mod pins;
