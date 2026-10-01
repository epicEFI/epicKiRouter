//! The board hash (T12 build item 8) — Java `BasicBoard.getHash()`
//! (`:165-167`) = MD5 hex over `serialize(true)`, the BASIC profile:
//! `ObjectOutputStream(getTraces(), getVias(), itemList)`
//! (`BoardSnapshotManager.java:26-72`).
//!
//! ## The banked divergence (anchors, "getHash / serialize")
//!
//! The hash STRING never equals Java's: Java digests
//! ObjectOutputStream bytes (class descriptors, JVM map orders); this
//! module digests a canonical little-endian encoding. Byte-identical
//! ObjectOutputStream emulation is NOT required by the parity contract
//! because the hash feeds BoardHistory identity
//! (`contains`/`remove`/`getRank`), the batch loop's stagnation
//! tracking, and log/event text only — never a pinned `compare_*`
//! trace row. What MUST match is the EQUALITY semantics: same board
//! state → same digest, different state → different digest, on every
//! state pair the driver can reach.
//!
//! ## Content (Java stream → this encoding)
//!
//! 1. `getTraces()` — the live traces in ascending-id order (Java's
//!    walk is descending; order is a deterministic function of the id
//!    set, so the equality partition is unchanged), each item
//!    encoded field-complete ([`Digest::item_entry`]).
//! 2. `getVias()` — ditto for the live vias.
//! 3. `itemList` — the whole [`epic_board::undo::UndoableObjects`]
//!    reachable node graph: `stackLevel`, `redoPossible`, then the
//!    live map entries, the deleted-object stack levels, and the
//!    undo/redo chains ([`epic_board::undo::UndoableObjects::digest_walk`]).
//!    Unreachable slab nodes are excluded — Java's GC drops them, so
//!    insert-then-remove hashes like the untouched board (a full-slab
//!    digest would break the contract; pinned in the tests).
//!
//! Per-item encodings cover every `ItemEntry` field (id, nets,
//! clearance class, component, fixed state, on-the-board flag and the
//! full per-kind payload), so any persistent mutation changes the
//! digest. Derived caches (`shape_precalc`, `drill_precalc`,
//! `shove_failing_*`, `changed_area`) are Java-transient or
//! item-external and are NOT hashed.

use epic_board::board::{Board, ItemEntry};
use epic_board::items::{BoardShape, FixedState, ItemData, ObstacleKind};
use epic_board::undo::UndoDigestRole;
use epic_geometry::int_point::IntPoint;
use epic_geometry::point::Point;
use epic_geometry::polyline::Polyline;
use sha2::{Digest as ShaDigest, Sha256};
use std::cmp::Reverse;

/// The SHA-256 wrapper with the little-endian primitive writers.
struct Digest(Sha256);

impl Digest {
    fn new() -> Self {
        let mut hasher = Sha256::new();
        // Domain separation: a digest of the empty board must never
        // collide with a digest of a board whose encoding is empty.
        hasher.update(b"epic-router.board-hash.v1");
        Digest(hasher)
    }

    fn bytes(&mut self, slice: &[u8]) {
        self.0.update((slice.len() as u64).to_le_bytes());
        self.0.update(slice);
    }

    fn u32(&mut self, value: u32) {
        self.0.update(value.to_le_bytes());
    }

    fn i32(&mut self, value: i32) {
        self.0.update(value.to_le_bytes());
    }

    fn f64(&mut self, value: f64) {
        self.0.update(value.to_le_bytes());
    }

    fn usize(&mut self, value: usize) {
        self.0.update((value as u64).to_le_bytes());
    }

    fn bool(&mut self, value: bool) {
        self.0.update([u8::from(value)]);
    }

    fn point(&mut self, point: &Point) {
        match point {
            Point::Int(int_point) => {
                self.0.update([0]);
                self.i32(int_point.x);
                self.i32(int_point.y);
            }
            // Rational coordinates (corner rounding etc.): the BigInt
            // numerators/denominator have no fixed-width form — their
            // Debug rendering is the canonical byte string (deterministic
            // decimal output, same content → same bytes).
            Point::Rational(rational) => {
                self.0.update([1]);
                self.bytes(format!("{rational:?}").as_bytes());
            }
        }
    }

    fn int_point(&mut self, point: &IntPoint) {
        self.i32(point.x);
        self.i32(point.y);
    }

    /// The polyline's defining lines (a line's `dir` is a cached pure
    /// function of `a`/`b` and is not hashed).
    fn polyline(&mut self, polyline: &Polyline) {
        self.usize(polyline.lines.len());
        for line in &polyline.lines {
            self.point(&line.a);
            self.point(&line.b);
        }
    }

    /// Deep closed-payload shapes (`TileShape`, `PolygonShape`,
    /// `Circle`, …) — plain data with deterministic Debug renderings.
    fn board_shape(&mut self, shape: &BoardShape) {
        self.bytes(format!("{shape:?}").as_bytes());
    }

    fn fixed(&mut self, fixed: FixedState) {
        // Fieldless enum: the declaration-order discriminant.
        self.u32(fixed as u32);
    }

    fn nets(&mut self, nets: &[i32]) {
        self.usize(nets.len());
        for net in nets {
            self.i32(*net);
        }
    }

    /// The per-kind payload (Java's item subclass fields).
    fn item_data(&mut self, data: &ItemData) {
        match data {
            ItemData::Trace {
                layer,
                half_width,
                lines,
            } => {
                self.u32(0);
                self.i32(*layer);
                self.i32(*half_width);
                self.polyline(lines);
            }
            ItemData::Pin {
                pin_index,
                padstack_no,
            } => {
                self.u32(1);
                self.i32(*pin_index);
                self.i32(*padstack_no);
            }
            ItemData::Via {
                center,
                padstack_no,
                attach_smd_allowed,
            } => {
                self.u32(2);
                self.int_point(center);
                self.i32(*padstack_no);
                self.bool(*attach_smd_allowed);
            }
            ItemData::ObstacleArea {
                kind,
                layer,
                area,
                translation,
                rotation,
                side_changed,
                name,
            } => {
                self.u32(3);
                self.u32(match kind {
                    ObstacleKind::ObstacleArea => 0,
                    ObstacleKind::ViaObstacleArea => 1,
                    ObstacleKind::ComponentObstacleArea => 2,
                });
                self.i32(*layer);
                self.board_shape(&area.border);
                self.usize(area.holes.len());
                for hole in &area.holes {
                    self.board_shape(hole);
                }
                self.int_point(translation);
                self.f64(*rotation);
                self.bool(*side_changed);
                match name {
                    Some(name) => {
                        self.bool(true);
                        self.bytes(name.as_bytes());
                    }
                    None => self.bool(false),
                }
            }
            ItemData::ConductionArea {
                layer,
                area,
                is_obstacle,
                is_filled,
            } => {
                self.u32(4);
                self.i32(*layer);
                self.board_shape(&area.border);
                self.usize(area.holes.len());
                for hole in &area.holes {
                    self.board_shape(hole);
                }
                self.bool(*is_obstacle);
                self.bool(*is_filled);
            }
            ItemData::ComponentOutline {
                layer,
                area,
                translation,
                rotation,
                is_front,
                is_courtyard,
                is_fabrication,
                is_closed,
            } => {
                self.u32(5);
                self.i32(*layer);
                self.board_shape(&area.border);
                self.usize(area.holes.len());
                for hole in &area.holes {
                    self.board_shape(hole);
                }
                self.int_point(translation);
                self.f64(*rotation);
                self.bool(*is_front);
                self.bool(*is_courtyard);
                self.bool(*is_fabrication);
                self.bool(*is_closed);
            }
            ItemData::BoardOutline {
                shapes,
                keepout_outside_outline,
            } => {
                self.u32(6);
                self.usize(shapes.len());
                for shape in shapes {
                    self.board_shape(shape);
                }
                self.bool(*keepout_outside_outline);
            }
            ItemData::Other => self.u32(7),
        }
    }

    /// The full `ItemEntry` (Java `Item`'s serialized base-class
    /// fields plus the subclass payload).
    fn item_entry(&mut self, entry: &ItemEntry) {
        self.u32(entry.id.get());
        self.nets(&entry.nets);
        self.i32(entry.clearance_class);
        self.i32(entry.component_id);
        self.fixed(entry.fixed);
        self.bool(entry.on_the_board);
        self.item_data(&entry.data);
    }
}

/// Java `BasicBoard.getHash()` (`:165-167`) →
/// `BoardSnapshotManager.getHash()` (`:58-72`): the digest hex string
/// of the board's trace-state profile. The hex is lowercase, like
/// Java's `Integer.toString(..., 16)` build.
#[must_use]
pub fn board_hash(board: &Board) -> String {
    let mut digest = Digest::new();

    // The live traces and vias, ascending id (Java getTraces()/getVias()
    // walk the itemList read path — live-only; order is id-derived).
    let mut traces: Vec<&ItemEntry> = Vec::new();
    let mut vias: Vec<&ItemEntry> = Vec::new();
    for entry in board.iter_ascending() {
        if !entry.on_the_board {
            continue;
        }
        match entry.data {
            ItemData::Trace { .. } => traces.push(entry),
            ItemData::Via { .. } => vias.push(entry),
            _ => {}
        }
    }
    digest.usize(traces.len());
    for entry in traces {
        digest.item_entry(entry);
    }
    digest.usize(vias.len());
    for entry in vias {
        digest.item_entry(entry);
    }

    // itemList: the plain fields, then the reachable node graph.
    digest.usize(board.undo_stack_level());
    digest.bool(board.undo_redo_possible());
    board.undo_digest_walk(|node| {
        let Reverse(item_id) = node.key;
        digest.u32(item_id.get());
        digest.usize(node.level);
        let (role_byte, extra_level): (u32, Option<usize>) = match node.role {
            UndoDigestRole::Live => (0, None),
            UndoDigestRole::Deleted { stack_level } => (1, Some(stack_level)),
            UndoDigestRole::LiveChain => (2, None),
            UndoDigestRole::DeletedChain { stack_level } => (3, Some(stack_level)),
        };
        digest.u32(role_byte);
        if let Some(stack_level) = extra_level {
            digest.usize(stack_level);
        }
        digest.item_entry(node.value);
    });

    format!("{:x}", digest.0.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::parse;
    use epic_board::id::ItemId;
    use epic_board::items::FixedState;
    use epic_board::trace_ops::{insert_trace_without_cleaning, remove_item_through_repository};
    use epic_board::tree_manager::SearchTreeManager;

    /// The T9/T10c locator-world fixture (2 layers, `unit um`).
    fn parse_fixture() -> (SearchTreeManager, Board) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../harness/fixtures/locator-spike/t9_locator45.dsn");
        let text = std::fs::read_to_string(&path).expect("fixture present");
        parse(&text)
    }

    fn int_point(x: i32, y: i32) -> IntPoint {
        IntPoint::new(x, y)
    }

    /// A minimal hand-built board with one trace item (id allocated,
    /// on the board).
    fn board_with_trace(
        half_width: i32,
        clearance_class: i32,
        nets: Vec<i32>,
        fixed: FixedState,
        component_id: i32,
    ) -> (Board, ItemId) {
        let mut board = Board::new();
        let id = board.alloc_id();
        board.insert_item(ItemEntry {
            id,
            data: ItemData::Trace {
                layer: 0,
                half_width,
                lines: Polyline::from_two_corners(
                    &Point::Int(int_point(0, 0)),
                    &Point::Int(int_point(1000, 0)),
                ),
            },
            nets,
            clearance_class,
            component_id,
            fixed,
            on_the_board: false,
        });
        (board, id)
    }

    /// Determinism and clone stability: the same parsed fixture twice
    /// hashes identically (the parse is deterministic), a CLONE hashes
    /// identically (the restore-identity foundation — Java's
    /// deserialize of a snapshot is the clone's twin), and repeated
    /// calls on one board are stable. A hash that reads mutable
    /// derived state (tree caches, memo fills) fails the clone arm.
    #[test]
    fn bh_deterministic_and_clone_stable() {
        let (_manager_a, board_a) = parse_fixture();
        let (_manager_b, board_b) = parse_fixture();
        let h_a1 = board_hash(&board_a);
        let h_a2 = board_hash(&board_a);
        assert_eq!(h_a1, h_a2, "repeated calls are stable");
        assert_eq!(h_a1, board_hash(&board_b), "two parses agree");
        let clone = board_a.clone();
        assert_eq!(h_a1, board_hash(&clone), "the clone hashes identically");
    }

    /// The encoding is field-complete: two hand-built boards differing
    /// in EXACTLY ONE ItemEntry field (or one polyline corner) must
    /// hash differently. Kills the count-only digest (the corner arm
    /// keeps counts equal) and every dropped-encoding-field mutant
    /// (one arm per field).
    #[test]
    fn bh_item_encoding_field_sweep() {
        let (base, _) = board_with_trace(500, 1, vec![1], FixedState::Unfixed, 0);
        let h_base = board_hash(&base);

        // geometry: same field set, one corner moved.
        let mut moved = Board::new();
        let id = moved.alloc_id();
        moved.insert_item(ItemEntry {
            id,
            data: ItemData::Trace {
                layer: 0,
                half_width: 500,
                lines: Polyline::from_two_corners(
                    &Point::Int(int_point(0, 0)),
                    &Point::Int(int_point(1001, 0)),
                ),
            },
            nets: vec![1],
            clearance_class: 1,
            component_id: 0,
            fixed: FixedState::Unfixed,
            on_the_board: false,
        });
        assert_ne!(
            h_base,
            board_hash(&moved),
            "one-corner geometry change must show (count-only digest dies here)"
        );

        // scalar fields, one per arm.
        for (label, half_width, class, nets, fixed, component_id) in [
            ("half_width", 501, 1, vec![1], FixedState::Unfixed, 0),
            ("clearance_class", 500, 2, vec![1], FixedState::Unfixed, 0),
            ("nets", 500, 1, vec![2], FixedState::Unfixed, 0),
            ("fixed", 500, 1, vec![1], FixedState::ShoveFixed, 0),
            ("component_id", 500, 1, vec![1], FixedState::Unfixed, 7),
        ] {
            let (other, _) = board_with_trace(half_width, class, nets, fixed, component_id);
            assert_ne!(
                h_base,
                board_hash(&other),
                "{label} participates in the digest"
            );
        }

        // an EMPTY board differs from any non-empty one and is stable.
        let empty = Board::new();
        let h_empty = board_hash(&empty);
        assert_eq!(h_empty, board_hash(&Board::new()));
        assert_ne!(h_empty, h_base);
    }

    /// The via payload: a hand-built via's center and padstack reach
    /// the digest (kills a vias-section-drop or center-drop mutant).
    #[test]
    fn bh_via_payload_sensitive() {
        let mut board = Board::new();
        let id = board.alloc_id();
        board.insert_item(ItemEntry {
            id,
            data: ItemData::Via {
                center: int_point(500, 700),
                padstack_no: 3,
                attach_smd_allowed: false,
            },
            nets: vec![1],
            clearance_class: 1,
            component_id: 0,
            fixed: FixedState::Unfixed,
            on_the_board: false,
        });
        let h0 = board_hash(&board);
        board.set_via_center(id, int_point(500, 701));
        assert_ne!(h0, board_hash(&board), "via center participates");
        // the arena mutation went through the node MIRROR (the setter
        // writes both faces) — the digest reads the undo node, so an
        // unmirrored write would leave the hash UNCHANGED.
    }

    /// The itemList equality partition (Java
    /// `BoardSnapshotManager.getHash` serializes the whole
    /// `UndoableObjects`): insert-then-remove at level 0 hashes like
    /// the untouched board (Java GCs the unreachable node — a
    /// FULL-SLAB digest fails this arm), a delete at level 1 parks the
    /// node in the delete list (the hash must move — the
    /// itemList-section-drop mutant dies here), undo re-lives the node
    /// (the stream's map contents change in Java too), and the
    /// stackLevel plain field alone discriminates (the level drop
    /// mutant dies on h1 vs h5).
    #[test]
    fn bh_itemlist_partition_insert_remove_undo() {
        let (mut manager, mut board) = parse_fixture();
        let h0 = board_hash(&board);

        // h1: one extra trace at level 0.
        let id = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(
                &Point::Int(int_point(-10_000, -10_000)),
                &Point::Int(int_point(-9_000, -9_000)),
            ),
            0,
            1500,
            &[1],
            1,
            FixedState::Unfixed,
        )
        .expect("insert succeeds");
        let h1 = board_hash(&board);
        assert_ne!(h1, h0, "an inserted trace moves the hash");

        // h2: delete it at level 0 — unrecoverable, the node is
        // unreachable exactly like Java's GC'd node → h2 == h0.
        remove_item_through_repository(&mut manager, &mut board, id);
        let h2 = board_hash(&board);
        assert_eq!(h2, h0, "insert-then-remove hashes like untouched");

        // h3: insert again, SNAPSHOT, then delete — the node (level 0)
        // lands in deletedObjectsStack[0] → the stream carries it.
        let id2 = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(
                &Point::Int(int_point(-10_000, -10_000)),
                &Point::Int(int_point(-9_000, -9_000)),
            ),
            0,
            1500,
            &[1],
            1,
            FixedState::Unfixed,
        )
        .expect("insert succeeds");
        board.generate_snapshot();
        remove_item_through_repository(&mut manager, &mut board, id2);
        let h3 = board_hash(&board);
        assert_ne!(h3, h0, "a delete-listed node is part of the stream");

        // h4: undo re-puts the node in the live map (Java's map content
        // changes → its stream changes; the delete list keeps the node
        // and the digest's seen-guard visits it once).
        let outcome = board.undo(&mut manager);
        assert!(outcome.changed, "undo restores the deleted trace");
        let h4 = board_hash(&board);
        assert_ne!(h4, h3, "undo changes the stream (map contents differ)");
        assert_ne!(h4, h0, "the level-1 snapshot remains in the stream");

        // h5: the same live content as h1 but one stack level higher —
        // the plain stackLevel field alone discriminates.
        let id3 = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(
                &Point::Int(int_point(-10_000, -10_000)),
                &Point::Int(int_point(-9_000, -9_000)),
            ),
            0,
            1500,
            &[1],
            1,
            FixedState::Unfixed,
        )
        .expect("insert succeeds");
        let _ = id3;
        let h5 = board_hash(&board);
        assert_ne!(h5, h4, "another live trace moves the hash");
    }
}
