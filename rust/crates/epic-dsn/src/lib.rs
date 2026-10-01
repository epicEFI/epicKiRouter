//! Specctra DSN/SES reader and writer with semantically-normalized
//! round-trip parity against the Java `io.specctra` package (design §5,
//! I/O parity gate).
//!
//! **M1b COMPLETE**: all 1,332 corpus fixtures parse to byte-identical
//! digests vs the Java oracle (175 digest + 1,157 soak; one documented
//! ledgered divergence on dsn-0151 — the `normalizeAllTraces` board
//! machinery deferred to M2 per D11), and the tier A+B sessions emit
//! BYTE-EQUAL through [`ses::writer`]. The gates run java-free in CI
//! (`epic-harness dsn compare` / `dsn ses-compare`).
//!
//! M1b progress: the hand-written tokenizer ([`lexer`]) and the keyword
//! table with scope skipping ([`keyword`]) port the JFlex-generated
//! `SpecctraDsnStreamReader` scan layer and the `Keyword`/`ScopeKeyword`
//! classes. [`coordinate_transform`] (incl. the T25 scale-factor loop),
//! [`layer_structure`] (T34 Electra fallback) and the parse state
//! ([`state`], T24 defaults + T35 netlist) port the shared reader
//! infrastructure. [`shape`] ports the `parser/Shape` hierarchy: the shape
//! IR, the five scope readers, the board transforms and the T44
//! bug-compatible bounding boxes. [`sink`] introduces the `BoardSink`
//! seam (D9) and [`ses_board`] the parse-derivable mini board model the
//! scope readers of Tasks 5-9 emit through; [`write_scope`] holds the
//! T38 identifier rule + the emission core and [`ses::writer`] the full
//! Task 13 `SesWriter` port (session emission from a parse-time board).

pub mod coordinate_transform;
pub mod keyword;
pub mod layer_structure;
pub mod lexer;
pub mod reader;
pub mod scope;
pub mod ses;
pub mod ses_board;
pub mod shape;
pub mod sink;
pub mod state;
pub mod write_scope;

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
