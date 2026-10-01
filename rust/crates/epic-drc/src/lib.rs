//! The two counts every M3 quality gate consumes — incomplete
//! connections (the ratsnest airline count) and clearance violations
//! — ported bit-faithfully from the frozen Java classes
//! `app.freerouting.drc.NetIncompletes` and
//! `app.freerouting.drc.DesignRulesChecker` (parity oracle at commit
//! `e7f9bdf1`), pinned against the committed drc corpus
//! (`harness/corpus/drc-golden.jsonl`, `epic-harness drc compare`).
//!
//! Module map:
//!
//! * [`incompletes`] — the ratsnest machinery: the raw per-net item
//!   lists, the `maxConnections` endpoint formula, and the FULL
//!   filter → connected-set grouping → Delaunay triangulation →
//!   sorted-Edge Kruskal pipeline behind the airline count. The count
//!   is EDGE-SET-DEPENDENT (the M3-T2 spike FALSIFIED the naive
//!   `groups − 1` claim); see the module docs.
//! * [`clearance`] — the violation walk: the per-kind `isObstacle`
//!   matrix, the enlarged-intersection gate, the tie-pin and
//!   outline-contains exemptions, and the A-B/B-A dedup.

pub mod clearance;
pub mod incompletes;

#[cfg(test)]
pub(crate) mod test_util;

#[cfg(test)]
mod tests {
    /// The crate name matches the package metadata.
    #[test]
    fn crate_scaffolds() {
        let name = env!("CARGO_PKG_NAME");
        assert_eq!(name, "epic-drc");
    }
}
