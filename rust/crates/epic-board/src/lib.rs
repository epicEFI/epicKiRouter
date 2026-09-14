//! Board model: items, ids, undo, rules, layers, components (M2). The
//! live board the parse IR converts into —
//! [`board::Board::from_ses_board`] — and the surface the router
//! mutates (design §4.1).
//!
//! M2 Task 1 scaffold: [`id`] (the id allocator, T61 burn + wrap),
//! [`items`] (kind dispatch, T70 obstacle kinds, per-kind payloads) and
//! [`board`] (the descending-id arena, D25/T60, revision T69, the
//! epic-dsn -> epic-board conversion boundary). Task 2 adds
//! [`undo`] — the `UndoableObjects` level-stack port (T62) jar-pinned
//! by `rust/harness/oracle/UndoSpike.java`, plus the components-side
//! second stack (T63). Task 3 adds the READ surfaces:
//! [`rules_surf`] (the clearance matrix with its ASYMMETRIC index
//! order, T54, the net table, classes, vias), [`layers`] (the
//! board-side `LayerStructure` — exact `getNo`, no parser fallback),
//! and [`components`] (placements with the T68 `rotate` trap, the
//! library mirror, and pin placement resolution jar-pinned by
//! `rust/harness/oracle/PinResolutionSpike.java`).

pub mod board;
pub mod components;
pub mod id;
pub mod items;
pub mod layers;
pub mod rules_surf;
pub mod undo;
