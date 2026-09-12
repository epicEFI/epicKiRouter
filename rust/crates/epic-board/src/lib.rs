//! Board model: items, layers, nets, clearance classes, pours, keepouts.
//! Arena-allocated struct-of-arrays layout with snapshot/undo (design §4.1;
//! content lands in M2).

#[cfg(test)]
mod tests {
    /// The crate is a scaffold in M0; content lands in M2.
    #[test]
    fn crate_scaffolds() {
        let name = env!("CARGO_PKG_NAME");
        assert_eq!(name, "epic-board");
    }
}
