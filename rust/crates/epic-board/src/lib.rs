//! Board model: items, ids, undo, rules, normalize (M2). The live board
//! the parse IR converts into — [`board::Board::from_ses_board`] — and
//! the surface the router mutates (design §4.1).
//!
//! M2 Task 1 scaffold: [`id`] (the id allocator, T61 burn + wrap),
//! [`items`] (kind dispatch, T70 obstacle kinds, per-kind payloads) and
//! [`board`] (the descending-id arena, D25/T60, revision T69, the
//! epic-dsn -> epic-board conversion boundary).

pub mod board;
pub mod id;
pub mod items;
