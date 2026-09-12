//! Incremental exact design-rule checking; the single violation authority
//! (design §4.1; content lands in M2/M6).

#[cfg(test)]
mod tests {
    /// The crate is a scaffold in M0; content lands in M2/M6.
    #[test]
    fn crate_scaffolds() {
        let name = env!("CARGO_PKG_NAME");
        assert_eq!(name, "epic-drc");
    }
}
