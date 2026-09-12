//! Spatial index: arena BVH/quadtree with incremental insert/remove on the
//! rip-up hot path, per-clearance-class Minkowski compensation (design §4.1;
//! content lands in M2).

#[cfg(test)]
mod tests {
    /// The crate is a scaffold in M0; content lands in M2.
    #[test]
    fn crate_scaffolds() {
        let name = env!("CARGO_PKG_NAME");
        assert_eq!(name, "epic-index");
    }
}
