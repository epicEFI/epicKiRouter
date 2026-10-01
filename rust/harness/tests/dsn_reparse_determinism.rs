//! M1b Task 12 invariant: re-parse determinism over the full committed
//! digest corpus. Every `dsn-NNNN` fixture of
//! `rust/harness/corpus/dsn-manifest.jsonl` is parsed TWICE into fresh
//! [`SesBoard`]s; both the [`DsnReadResult`] (variant + payload) and the
//! resulting board must be equal. The parser has no excuse for run-to-run
//! drift: no hashmap iteration order may leak into item ids, net tables,
//! warning order, or geometry, or the digest goldens would be unreproducible
//! byte-for-byte.
//!
//! Lives in the harness crate (which already depends on epic-dsn) so the
//! reader crate stays filesystem-free; mirrors the corpus compare, which
//! walks the same manifest but against Java goldens.

use std::fs;
use std::path::PathBuf;

use epic_dsn::reader::{DsnReadResult, read_board};
use epic_dsn::ses_board::SesBoard;

/// `rust/harness` -> `rust` -> repo root (compile-time stable; the
/// harness binary resolves it the same way at runtime).
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("harness dir has a parent")
        .parent()
        .expect("rust dir has a parent")
        .to_path_buf()
}

#[derive(serde::Deserialize)]
struct ManifestEntry {
    id: String,
    path: String,
}

/// Every digest fixture re-parses bit-identically.
#[test]
fn every_digest_fixture_reparses_identically() {
    let manifest = fs::read_to_string(repo_root().join("rust/harness/corpus/dsn-manifest.jsonl"))
        .expect("committed manifest is readable");
    let entries: Vec<ManifestEntry> = manifest
        .lines()
        .filter(|line| line.starts_with("{\"id\":\"dsn-"))
        .map(|line| serde_json::from_str(line).expect("manifest line parses (committed format)"))
        .collect();
    assert_eq!(
        entries.len(),
        175,
        "pinned digest corpus size — update alongside the manifest/golden pins"
    );
    for entry in &entries {
        let bytes = fs::read(repo_root().join(&entry.path))
            .unwrap_or_else(|error| panic!("reading {} ({}): {error}", entry.path, entry.id));
        let parse = || {
            let mut board = SesBoard::new();
            let result = read_board(&bytes, &mut board);
            (result, board)
        };
        let (first_result, first_board) = parse();
        let (second_result, second_board) = parse();
        assert_eq!(
            first_result, second_result,
            "{}: read result drifted across re-parse",
            entry.id
        );
        assert_eq!(
            first_board, second_board,
            "{}: board state drifted across re-parse",
            entry.id
        );
    }
}

/// The result variants stay structurally stable (exhaustive match — a new
/// variant must be consciously added here, not silently ignored).
#[test]
fn dsn_read_result_variants_are_exhaustive() {
    let mut board = SesBoard::new();
    match read_board(b"(pcb)", &mut board) {
        DsnReadResult::Success { .. }
        | DsnReadResult::OutlineMissing { .. }
        | DsnReadResult::ParseError { .. }
        | DsnReadResult::IoError => {}
    }
}
