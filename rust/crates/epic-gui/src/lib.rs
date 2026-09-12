//! egui/wgpu frontend: GPU board canvas, per-stage routing progress,
//! ratsnest, DRC markers, congestion heatmap (design §7; content lands in M9).

#[cfg(test)]
mod tests {
    /// The crate is a scaffold in M0; content lands in M9.
    #[test]
    fn crate_scaffolds() {
        let name = env!("CARGO_PKG_NAME");
        assert_eq!(name, "epic-gui");
    }
}
