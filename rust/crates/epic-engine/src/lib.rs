//! Jobs, layered settings (defaults -> JSON -> DSN -> env -> CLI -> GUI with
//! Java SettingsMerger precedence semantics), progress/cancel, seed control
//! (design §4.1; content lands in M3).

#[cfg(test)]
mod tests {
    /// The crate is a scaffold in M0; content lands in M3.
    #[test]
    fn crate_scaffolds() {
        let name = env!("CARGO_PKG_NAME");
        assert_eq!(name, "epic-engine");
    }
}
