//! Specctra DSN/SES reader and writer with semantically-normalized
//! round-trip parity against the Java `io.specctra` package (design §5,
//! I/O parity gate).

#[cfg(test)]
mod tests {
    /// The crate is a scaffold in M0; content lands in M1.
    #[test]
    fn crate_scaffolds() {
        let name = env!("CARGO_PKG_NAME");
        assert_eq!(name, "epic-dsn");
    }
}
