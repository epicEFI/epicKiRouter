//! M2 Task 1 pins: `Board::from_ses_board` on real tier-A fixtures —
//! item ids preserved EXACTLY (no renumbering; parse-time burned gaps
//! kept, T61), every enumeration DESCENDING (D25/T60), kinds preserved
//! per IR item including the three distinct obstacle kinds (T70), and
//! the id generator continuing one past the parse.
//!
//! Fixture set (tier A, chosen for kind coverage): DAC2020_bm08
//! (dense pins/vias, no planes), ecc83-pp (a power plane ->
//! ConductionArea), pic_programmer (4 structure keepouts ->
//! ObstacleArea + the largest pin/place count). Traces and the burned
//! gap are pinned in-crate (epic-board `board.rs` tests) because none
//! of these fixtures ship routed wires.

use std::path::{Path, PathBuf};

use epic_board::board::Board;
use epic_board::items::BoardItemType;
use epic_dsn::reader::{DsnReadResult, read_board};
use epic_dsn::ses_board::{ItemIr, SesBoard};
use epic_dsn::sink::KeepoutKindIr;

/// CARGO_MANIFEST_DIR = <repo>/rust/harness; the repo root is two up.
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .expect("repo root is two levels above the harness crate")
}

/// The tier-A fixtures (paths under the tiers.yaml fixtures_root).
const FIXTURES: [&str; 3] = [
    "DAC2020_boards/DAC2020_bm08.dsn",
    "KiCad_10_demos/ecc83-pp.dsn",
    "KiCad_10_demos/pic_programmer.dsn",
];

/// Parses one fixture through the M1b reader (clean Success: a warning
/// here would shift ids and poison the pins).
fn parse_fixture(rel: &str) -> SesBoard {
    let path = repo_root().join("scripts/benchmark/fixtures").join(rel);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    let mut ses = SesBoard::new();
    match read_board(&bytes, &mut ses) {
        DsnReadResult::Success { warnings } => {
            assert!(
                warnings.is_empty(),
                "{rel}: expected WARN_COUNT 0, got {warnings:?}"
            );
        }
        other => panic!("{rel}: expected Success, got {other:?}"),
    }
    ses
}

/// The expected epic-board kind of an IR item — the T70 mapping stated
/// INDEPENDENTLY of the conversion under test (a conversion that
/// collapses the obstacle kinds or swaps any kind fails here).
fn expected_kind(item: &ItemIr) -> BoardItemType {
    match item {
        ItemIr::Trace { .. } => BoardItemType::Trace,
        ItemIr::Via { .. } => BoardItemType::Via,
        ItemIr::Pin { .. } => BoardItemType::Pin,
        ItemIr::Keepout { keepout, .. } => match keepout.kind {
            KeepoutKindIr::Keepout => BoardItemType::ObstacleArea,
            KeepoutKindIr::ViaKeepout => BoardItemType::ViaObstacleArea,
            KeepoutKindIr::PlaceKeepout => BoardItemType::ComponentObstacleArea,
        },
        ItemIr::ConductionArea { .. } => BoardItemType::ConductionArea,
        ItemIr::ComponentOutline { .. } => BoardItemType::ComponentOutline,
        ItemIr::BoardOutline { .. } => BoardItemType::BoardOutline,
    }
}

fn convert(rel: &str) -> (SesBoard, Board) {
    let ses = parse_fixture(rel);
    let board = Board::from_ses_board(&ses);
    (ses, board)
}

/// Id preservation + count + descending enumeration + kind preservation
/// per item, on all three fixtures. A renumbering conversion (dense
/// 1..n), an ascending arena, or a kind swap fails this pin.
#[test]
fn from_ses_board_preserves_ids_kinds_and_descending_order() {
    for rel in FIXTURES {
        let (ses, board) = convert(rel);
        let ir_ids: Vec<u32> = ses
            .items
            .iter()
            .map(|item| u32::try_from(item.id()).expect("IR ids are positive"))
            .collect();

        // The IR ids are strictly ascending and unique (parse order);
        // the board must hold exactly that set.
        let mut sorted = ir_ids.clone();
        sorted.sort_unstable();
        assert_eq!(ir_ids, sorted, "{rel}: IR ids not strictly ascending");
        assert_eq!(board.item_count(), ir_ids.len(), "{rel}: item count");

        // Descending enumeration equals the IR set reversed EXACTLY —
        // byte-for-byte ids, no renumbering (an ascending arena or a
        // dense renumber fails).
        let descending: Vec<u32> = board.iter_descending().map(|e| e.id.get()).collect();
        let mut expected = ir_ids.clone();
        expected.reverse();
        assert_eq!(descending, expected, "{rel}: descending ids vs IR ids");

        // Every IR item is present under its own id with its own kind.
        for (item, id) in ses.items.iter().zip(ir_ids.iter()) {
            let entry = board
                .get(epic_board::id::ItemId::new(*id))
                .unwrap_or_else(|| panic!("{rel}: id {id} missing"));
            assert_eq!(
                entry.board_item_type(),
                expected_kind(item),
                "{rel}: kind of id {id}"
            );
            assert!(entry.on_the_board, "{rel}: id {id} inserted");
        }
    }
}

/// The generator continues EXACTLY at the IR's position on every
/// fixture: the first post-conversion allocation is
/// `last_assigned_item_id() + 1` (Java `maxGeneratedId()` + 1), and the
/// conversion consumed no ids of its own.
#[test]
fn from_ses_board_continues_the_generator_at_the_ir_position() {
    for rel in FIXTURES {
        let (ses, mut board) = convert(rel);
        assert!(
            ses.last_assigned_item_id() > 0,
            "{rel}: fixture assigned ids"
        );
        assert_eq!(
            board.alloc_id().get(),
            u32::try_from(ses.last_assigned_item_id()).expect("positive") + 1,
            "{rel}: next id continues the parse sequence"
        );
    }
}

/// Kind-coverage guard for the fixture set itself (anchor discipline):
/// across the three fixtures the conversion must have exercised pins,
/// vias, conduction areas (planes), obstacle areas (keepouts), and the
/// board outline — a fixture-swap that silently drops coverage fails.
#[test]
fn the_fixture_set_exercises_the_kind_map_end_to_end() {
    let mut pins = 0usize;
    let mut vias = 0usize;
    let mut conduction = 0usize;
    let mut obstacle = 0usize;
    let mut outlines = 0usize;
    for rel in FIXTURES {
        let (ses, board) = convert(rel);
        for item in &ses.items {
            match expected_kind(item) {
                BoardItemType::Pin => pins += 1,
                BoardItemType::Via => vias += 1,
                BoardItemType::ConductionArea => conduction += 1,
                BoardItemType::ObstacleArea => obstacle += 1,
                BoardItemType::BoardOutline => outlines += 1,
                _ => {}
            }
        }
        // Per-fixture invariant: exactly one board outline (the id-1
        // anchor item of every parse).
        assert_eq!(
            board
                .iter_descending()
                .filter(|e| e.board_item_type() == BoardItemType::BoardOutline)
                .count(),
            1,
            "{rel}: exactly one board outline"
        );
    }
    assert!(pins > 100, "pin coverage: {pins}");
    // Unrouted fixtures carry NO via items — vias are router-created;
    // the Via conversion arm is pinned by the epic-board in-crate DSN
    // test (a real parse-time via). This assertion keeps the census
    // honest: if a fixture ever ships vias, update the expectation.
    assert_eq!(vias, 0, "unrouted fixture via items: {vias}");
    assert!(conduction > 0, "conduction (plane) coverage: {conduction}");
    assert!(obstacle > 0, "obstacle (keepout) coverage: {obstacle}");
    assert_eq!(outlines, 3, "one outline per fixture");
}
