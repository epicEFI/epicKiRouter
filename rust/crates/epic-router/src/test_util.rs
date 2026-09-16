//! Parse + lookup helpers for the epic-router pin suites (compiled for
//! `cargo test` only) — the epic-drc pattern: crafted DSN text through
//! the SAME reader + board build the corpus harness parity-verifies.
//!
//! Lookup discipline (cerebrum pin mode 8): NO absolute item ids —
//! items are found by net membership or kind; net numbers are pinned
//! only by the craft's DECLARATION order (1-based netlist position).

use epic_board::board::Board;
use epic_board::tree_manager::SearchTreeManager;
use epic_dsn::reader::{DsnReadResult, read_board};
use epic_dsn::ses_board::SesBoard;

/// Parse a crafted DSN; returns the tree manager (the harness fill)
/// over the board.
pub(crate) fn parse(text: &str) -> (SearchTreeManager, Board) {
    let mut ses = SesBoard::new();
    let parsed = read_board(text.as_bytes(), &mut ses);
    let DsnReadResult::Success { warnings: _ } = &parsed else {
        panic!("crafted DSN must parse: {parsed:?}");
    };
    let mut board = Board::from_ses_board(&ses);
    let mut manager = SearchTreeManager::new();
    manager.reinsert_tree_items(&mut board);
    (manager, board)
}

/// The 1-based number of the net with this name (declaration-order
/// position; panics when absent — a missing net is a craft bug).
pub(crate) fn net_no(board: &Board, name: &str) -> i32 {
    (1..=board.rules().nets.max_net_number())
        .find(|&n| {
            board
                .rules()
                .nets
                .get(n)
                .is_some_and(|net| net.name == name)
        })
        .unwrap_or_else(|| panic!("net {name} absent"))
}
