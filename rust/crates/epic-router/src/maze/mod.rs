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
//! * ripup (`checkRipup` / `checkLeavingRippedItem`,
//!   [`ripup`]) and the READ-ONLY shove probe
//!   (`shoveTraceRoom` / `MazeTraceShover.checkShoveTraceLine`,
//!   [`shove_probe`]) are T7 ports; the shove's board-mutation body
//!   (`TraceShover.check` recursion + `checkTraceSegment`) remains a
//!   T10 seam ([`crate::drill::DrillEngine::shove_trace_check`] /
//!   [`crate::drill::DrillEngine::check_trace_segment`]);
//! * the lower-bound heuristic itself ([`destination_distance`]) is
//!   the T8 production implementation of the
//!   [`crate::drill::DestinationDistance`] trait; `ViaLayerChecker`
//!   (T10) remains an injected-trait seam.

pub mod completion;
pub mod destination_distance;
pub mod expansion_engine;
pub mod list_element;
pub mod locator_access;
pub mod ripup;
pub mod search_engine;
pub mod shove_probe;

#[cfg(test)]
pub(crate) mod pins;
