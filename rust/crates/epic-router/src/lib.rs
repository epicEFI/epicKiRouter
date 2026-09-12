//! The staged router pipeline: fanout/escape, global routing (PathFinder
//! negotiated congestion), detail routing (octagon A* + push-and-shove),
//! plane routing, tuning, gloss (design §4.2; content lands from M3 on).

#[cfg(test)]
mod tests {
    /// The crate is a scaffold in M0; content lands from M3 on.
    #[test]
    fn crate_scaffolds() {
        let name = env!("CARGO_PKG_NAME");
        assert_eq!(name, "epic-router");
    }
}
