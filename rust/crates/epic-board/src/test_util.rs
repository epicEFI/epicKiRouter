//! Shared TEST-ONLY parse helpers (T10 review round). Every crate
//! test that needs a live [`Board`] parsed the same way the harness
//! does — fixture file or inline DSN text, epic-dsn reader into a
//! [`epic_dsn::ses_board::SesBoard`], then
//! [`Board::from_ses_board`] — gets it here, so the parse recipe
//! exists once instead of per-test-module (it was previously a
//! byte-similar local in `tree_shapes`, `tree_manager`, and
//! `contacts`). Task 11 (`combine`) and Task 15
//! (`snappedEndpoint`) reuse it.
//!
//! Compiled only under `cfg(test)` — no production surface.

use crate::board::Board;
use epic_dsn::reader::{DsnReadResult, read_board};
use epic_dsn::ses_board::SesBoard;

/// Parses a fixture FILE through the epic-dsn reader and converts it
/// (panics with the path on a read or parse failure — a missing
/// fixture is a test-setup bug, not a case to handle).
pub(crate) fn parse_board_from_path(path: &str) -> Board {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    parse_bytes(&bytes, path)
}

/// Parses DSN TEXT (inline crafted boards) through the same path.
pub(crate) fn parse_board_from_text(text: &str) -> Board {
    parse_bytes(text.as_bytes(), "inline DSN text")
}

fn parse_bytes(bytes: &[u8], name: &str) -> Board {
    let mut ses = SesBoard::new();
    match read_board(bytes, &mut ses) {
        DsnReadResult::Success { .. } => {}
        other => panic!("expected Success for {name}, got {other:?}"),
    }
    Board::from_ses_board(&ses)
}
