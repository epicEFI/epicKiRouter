//! Specctra DSN/SES reader and writer with semantically-normalized
//! round-trip parity against the Java `io.specctra` package (design §5,
//! I/O parity gate).
//!
//! M1b progress: the hand-written tokenizer ([`lexer`]) and the keyword
//! table with scope skipping ([`keyword`]) port the JFlex-generated
//! `SpecctraDsnStreamReader` scan layer and the `Keyword`/`ScopeKeyword`
//! classes. [`coordinate_transform`] (incl. the T25 scale-factor loop),
//! [`layer_structure`] (T34 Electra fallback) and the parse state
//! ([`state`], T24 defaults + T35 netlist) port the shared reader
//! infrastructure. Scope readers (`readScope` dispatch, wiring/rules/network
//! readers) land in later M1b tasks.

pub mod coordinate_transform;
pub mod keyword;
pub mod layer_structure;
pub mod lexer;
pub mod state;

#[cfg(test)]
mod tests {
    /// The crate roots exist and the milestone modules compile.
    #[test]
    fn crate_scaffolds() {
        let name = env!("CARGO_PKG_NAME");
        assert_eq!(name, "epic-dsn");
        let mut scanner = crate::lexer::Scanner::new(b"(pcb)");
        assert_eq!(scanner.next_token(), crate::lexer::Token::Open);
    }
}
