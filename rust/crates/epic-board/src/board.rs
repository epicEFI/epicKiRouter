//! The live board: the item arena, the id generator, and the
//! parse-IR conversion boundary.
//!
//! Java anchors: `board/facade/BoardItemRepository.java` (insert/remove
//! bookkeeping — the revision bumps at `:166` and `:198`, T69) and
//! `board/model/items/Item.java` (the item-common fields, the
//! constructor id allocation `:86-90`, the `compareTo` order `:95-103`).
//!
//! ## Enumeration contract (D25, T60)
//!
//! `Item.compareTo = item.id - id` and Java's item list is a
//! `ConcurrentSkipListMap` under natural order, so EVERY board
//! enumeration walks DESCENDING id. The arena is keyed
//! `Reverse<ItemId>`, which makes that the default iteration order;
//! every future epic-board enumeration MUST preserve it.
//!
//! ## Id and revision semantics (T61, T69)
//!
//! Insert does NOT allocate: ids are allocated at item CONSTRUCTION
//! (`Item.java:86-90`), so construct-then-discard burns an id and a
//! deleted item's id is never reused — builders call
//! [`Board::alloc_id`] first and put the id into the entry they insert.
//! `revision` bumps on EVERY successful insert AND remove
//! (`BoardItemRepository.java:166`, `:198`); the deletion-forbidden
//! early-out (`:189-191`) and the trace min/max width bookkeeping are
//! later M2 tasks (they need the rules surface).

use std::cmp::Reverse;
use std::collections::BTreeMap;

use epic_dsn::ses_board::{ItemIr, SesBoard};
use epic_dsn::shape::BoardShape as IrBoardShape;
use epic_dsn::sink::{AreaIr, FixedStateIr, KeepoutKindIr};

use crate::id::{ItemId, ItemIdGenerator};
use crate::items::{Area, BoardItemType, BoardShape, FixedState, ItemData, ObstacleKind};

/// One arena entry: the Java `Item` base-class fields plus the
/// per-kind payload. `on_the_board` mirrors `Item.onTheBoard`
/// (`Item.java:63-64`) — false while constructed-but-not-inserted
/// (Java's burned-id state), true from [`Board::insert_item`] on.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemEntry {
    /// The item id (Java `Item.getId()`), allocated at construction.
    pub id: ItemId,
    /// The per-kind payload (the Java subclass fields).
    pub data: ItemData,
    /// Java `Item.netNumbers`.
    pub nets: Vec<i32>,
    /// Java `Item.clearanceClassIndex`.
    pub clearance_class: i32,
    /// Java `Item.componentId` (0 = belongs to no component).
    pub component_id: i32,
    /// Java `Item.fixedState`.
    pub fixed: FixedState,
    /// Java `Item.onTheBoard` — set true by [`Board::insert_item`].
    pub on_the_board: bool,
}

impl ItemEntry {
    /// Java `Item.getBoardItemType()` (`Item.java:117-146`).
    #[must_use]
    pub fn board_item_type(&self) -> BoardItemType {
        self.data.board_item_type()
    }
}

/// The live board model (Java `BasicBoard`'s item-list face).
#[derive(Clone, Debug, Default)]
pub struct Board {
    /// The items keyed by DESCENDING id (module docs, D25/T60).
    items: BTreeMap<Reverse<ItemId>, ItemEntry>,
    /// The id generator (Java `board.communication.idGenerator`).
    id_generator: ItemIdGenerator,
    /// Java `BasicBoard.revision` — bumped per insert and remove (T69).
    revision: u64,
}

impl Board {
    /// An empty board.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocates the next item id — the item-CONSTRUCTION counterpart
    /// of Java's `board.communication.idGenerator.newId()` call inside
    /// `Item`'s constructor (`Item.java:86-90`).
    ///
    /// # Burn semantics (T61)
    ///
    /// Allocation is NOT part of insertion: an id handed out here and
    /// never inserted (construct-then-discard — Java's closed-trace
    /// drop `BasicBoard.java:192-196`, split pieces, degenerate
    /// normalize removals) stays consumed, and deletion never frees
    /// ids. The generator is strictly monotone up to the wrap at
    /// [`crate::id::MAX_ID`].
    pub fn alloc_id(&mut self) -> ItemId {
        self.id_generator.new_id()
    }

    /// Inserts a constructed entry. The id must have come from
    /// [`Board::alloc_id`] (or a parse IR) and must be free — Java's
    /// map would silently replace a duplicate key, which this port
    /// treats as the programming error it is. Bumps the revision (T69)
    /// and sets the entry's `on_the_board` flag (`Item.java:63-64`).
    /// Does NOT touch the generator: insertion consumes no id.
    pub fn insert_item(&mut self, mut entry: ItemEntry) {
        assert!(
            !self.items.contains_key(&Reverse(entry.id)),
            "insert_item: item id {} already on the board",
            entry.id.get()
        );
        entry.on_the_board = true;
        self.items.insert(Reverse(entry.id), entry);
        self.revision += 1;
    }

    /// Removes the entry with the given id, returning it (Java holds
    /// the `Item` object; the id is the port's handle). Bumps the
    /// revision ONLY when an entry was actually removed
    /// (`BoardItemRepository.java:198` sits behind the successful-removal
    /// path). A no-op on a foreign id.
    pub fn remove_item(&mut self, id: ItemId) -> Option<ItemEntry> {
        let removed = self.items.remove(&Reverse(id));
        if removed.is_some() {
            self.revision += 1;
        }
        removed
    }

    /// The item with the given id, if present.
    #[must_use]
    pub fn get(&self, id: ItemId) -> Option<&ItemEntry> {
        self.items.get(&Reverse(id))
    }

    /// EVERY public enumeration walks DESCENDING id (D25/T60 — Java's
    /// `ConcurrentSkipListMap` under `Item.compareTo`).
    pub fn iter_descending(&self) -> impl Iterator<Item = &ItemEntry> {
        self.items.values()
    }

    /// The number of live items.
    #[must_use]
    pub fn item_count(&self) -> usize {
        self.items.len()
    }

    /// The board revision — bumped on every insert and remove (T69).
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Converts the parse-time IR into the live board (M2 plan:
    /// `SesBoard` is the one-shot parse model; this is the ONLY
    /// epic-dsn -> epic-board item path).
    ///
    /// - Every IR item is converted with its id EXACTLY preserved (no
    ///   renumbering; the IR's gaps are Java's burned ids, T61).
    /// - The id generator continues at the IR's position
    ///   (`SesBoard::last_assigned_item_id` mirrors
    ///   `ItemIdGenerator.maxGeneratedId()`), so the next
    ///   [`Board::alloc_id`] returns one past the parse.
    /// - The obstacle keepout kinds stay DISTINCT (T70): the IR's
    ///   `KeepoutKindIr` maps to the three Java `ObstacleArea`
    ///   subclasses exactly as the readers chose them
    ///   (`Structure.java:926-933`, `Network.java:1080-1149`).
    /// - Trace polylines are rebuilt from the IR's (rounded) corner
    ///   list with `Polyline::from_points` — the inverse of Java's
    ///   `new Polyline(polygon)` (`Wiring.java:531`); for the
    ///   `PolylinePath` path (`Wiring.java:562`, `new Polyline(lines)`)
    ///   the interior corners are identical and the end lines take the
    ///   canonical perpendicular directions. Corner rounding itself is
    ///   the DOCUMENTED M1b divergence (`scope/wiring.rs` module docs:
    ///   Java keeps exact rationals; the port rounds at parse), not
    ///   re-derived here.
    #[must_use]
    pub fn from_ses_board(ses: &SesBoard) -> Self {
        let mut board = Board::new();
        board
            .id_generator
            .set_next(u32::try_from(ses.last_assigned_item_id()).expect("IR ids are non-negative"));
        for item in &ses.items {
            if let Some(entry) = convert_item(item) {
                board.insert_item(entry);
            }
        }
        board
    }
}

/// Converts one IR item. `None` skips the item (defensively) — today
/// only the never-stored null-area component outline can hit it.
fn convert_item(item: &ItemIr) -> Option<ItemEntry> {
    let id = ItemId::new(u32::try_from(item.id()).expect("IR ids are positive"));
    let entry = match item {
        ItemIr::Trace { trace, .. } => ItemEntry {
            id,
            data: ItemData::Trace {
                layer: trace.layer_no,
                half_width: trace.half_width,
                lines: polyline_from_corners(&trace.corners),
            },
            nets: trace.nets.clone(),
            clearance_class: trace.clearance_class,
            component_id: 0,
            fixed: fixed_from_ir(trace.fixed),
            on_the_board: false,
        },
        ItemIr::Via { via, .. } => ItemEntry {
            id,
            data: ItemData::Via {
                center: via.location,
                padstack_no: via.padstack_no,
                attach_smd_allowed: via.attach_smd_allowed,
            },
            nets: via.nets.clone(),
            clearance_class: via.clearance_class,
            component_id: 0,
            fixed: fixed_from_ir(via.fixed),
            on_the_board: false,
        },
        ItemIr::Pin { pin, .. } => ItemEntry {
            id,
            data: ItemData::Pin {
                pin_index: pin.pin_index,
                padstack_no: pin.padstack_no,
            },
            nets: pin.nets.clone(),
            clearance_class: pin.clearance_class,
            component_id: pin.component_id,
            fixed: fixed_from_ir(pin.fixed),
            on_the_board: false,
        },
        ItemIr::Keepout { keepout, .. } => ItemEntry {
            id,
            data: ItemData::ObstacleArea {
                kind: obstacle_kind_from_ir(keepout.kind),
                layer: keepout.layer_no,
                area: area_from_ir(&keepout.area),
                translation: keepout.translation,
                rotation: keepout.rotation,
                side_changed: keepout.side_changed,
                name: keepout.name.clone(),
            },
            // Parse-time keepouts never carry nets (the KeepoutIr docs).
            nets: Vec::new(),
            clearance_class: keepout.clearance_class,
            component_id: keepout.component_id,
            fixed: fixed_from_ir(keepout.fixed),
            on_the_board: false,
        },
        ItemIr::ConductionArea { area, .. } => ItemEntry {
            id,
            data: ItemData::ConductionArea {
                layer: area.layer_no,
                area: area_from_ir(&area.area),
            },
            nets: area.nets.clone(),
            clearance_class: area.clearance_class,
            component_id: 0,
            fixed: fixed_from_ir(area.fixed),
            on_the_board: false,
        },
        ItemIr::ComponentOutline { outline, .. } => {
            // The IR's sink never stores a None-area component outline
            // (`insert_component_outline` mirrors the
            // `BasicBoard.insertComponentOutline` null guard: no item,
            // no id). The skip keeps the conversion total if that ever
            // changes.
            let area = outline.area.as_ref()?;
            ItemEntry {
                id,
                data: ItemData::ComponentOutline {
                    layer: outline.layer_no,
                    area: area_from_ir(area),
                    translation: outline.translation,
                    rotation: outline.rotation,
                    is_front: outline.is_front,
                    is_courtyard: outline.is_courtyard,
                    is_fabrication: outline.is_fabrication,
                    is_closed: outline.is_closed,
                },
                nets: outline.nets.clone(),
                clearance_class: outline.clearance_class,
                component_id: outline.component_id,
                fixed: fixed_from_ir(outline.fixed),
                on_the_board: false,
            }
        }
        ItemIr::BoardOutline { outline, .. } => ItemEntry {
            id,
            data: ItemData::BoardOutline {
                shapes: outline.shapes.iter().map(board_shape_from_ir).collect(),
            },
            // Java constructs the outline with `new int[0]`
            // (`BoardOutline.java:47`).
            nets: Vec::new(),
            clearance_class: outline.clearance_class,
            component_id: 0,
            fixed: fixed_from_ir(outline.fixed),
            on_the_board: false,
        },
    };
    Some(entry)
}

/// Java `FixedState` ordinal mirror from the IR (both enums share the
/// declaration order; the IR docs pin the order as load-bearing).
fn fixed_from_ir(fixed: FixedStateIr) -> FixedState {
    match fixed {
        FixedStateIr::Unfixed => FixedState::Unfixed,
        FixedStateIr::ShoveFixed => FixedState::ShoveFixed,
        FixedStateIr::UserFixed => FixedState::UserFixed,
        FixedStateIr::SystemFixed => FixedState::SystemFixed,
    }
}

/// The readers' keepout-kind -> `ObstacleArea`-subclass choice
/// (`Structure.java:926-933`: `via_keepout` -> `ViaObstacleArea`,
/// `place_keepout` -> `ComponentObstacleArea`, else `ObstacleArea`;
/// `Network.java:1080-1149` keeps the same k=0/1/2 order for package
/// keepouts).
fn obstacle_kind_from_ir(kind: KeepoutKindIr) -> ObstacleKind {
    match kind {
        KeepoutKindIr::Keepout => ObstacleKind::ObstacleArea,
        KeepoutKindIr::ViaKeepout => ObstacleKind::ViaObstacleArea,
        KeepoutKindIr::PlaceKeepout => ObstacleKind::ComponentObstacleArea,
    }
}

/// The epic-board mirror of the IR's `BoardShape` — variants matched
/// 1:1 (an epic-dsn variant addition fails this match at compile time,
/// which is the conversion boundary doing its job).
fn board_shape_from_ir(shape: &IrBoardShape) -> BoardShape {
    match shape {
        IrBoardShape::Tile(tile) => BoardShape::Tile(tile.clone()),
        IrBoardShape::PolygonShape(polygon) => BoardShape::PolygonShape(polygon.clone()),
        IrBoardShape::Circle(circle) => BoardShape::Circle(*circle),
    }
}

/// The IR's `border + holes` area, converted shape-by-shape.
fn area_from_ir(area: &AreaIr) -> Area {
    Area {
        border: board_shape_from_ir(&area.border),
        holes: area.holes.iter().map(board_shape_from_ir).collect(),
    }
}

/// Rebuilds a trace polyline from the IR's corner list — see
/// [`Board::from_ses_board`] for the constructor contract.
fn polyline_from_corners(
    corners: &[epic_geometry::int_point::IntPoint],
) -> epic_geometry::polyline::Polyline {
    let points: Vec<epic_geometry::point::Point> = corners
        .iter()
        .map(|&corner| epic_geometry::point::Point::Int(corner))
        .collect();
    epic_geometry::polyline::Polyline::from_points(&points)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::MAX_ID;
    use epic_dsn::reader::{DsnReadResult, read_board};

    /// A hand-built entry for arena tests.
    fn pin_entry(id: ItemId) -> ItemEntry {
        ItemEntry {
            id,
            data: ItemData::Pin {
                pin_index: 0,
                padstack_no: 1,
            },
            nets: vec![1],
            clearance_class: 1,
            component_id: 1,
            fixed: FixedState::Unfixed,
            on_the_board: false,
        }
    }

    /// `BoardItemRepository.java:166`/`:198`: the revision bumps on
    /// every insert AND every successful remove; a remove of a foreign
    /// id is a no-op and must NOT bump (T69).
    #[test]
    fn revision_bumps_on_insert_and_remove_but_not_on_a_missed_remove() {
        let mut board = Board::new();
        assert_eq!(board.revision(), 0);
        let one = board.alloc_id();
        board.insert_item(pin_entry(one));
        assert_eq!(board.revision(), 1, "insert bumps");
        let two = board.alloc_id();
        board.insert_item(pin_entry(two));
        assert_eq!(board.revision(), 2, "second insert bumps");
        assert!(board.remove_item(one).is_some());
        assert_eq!(board.revision(), 3, "remove bumps");
        assert!(board.remove_item(one).is_none());
        assert_eq!(board.revision(), 3, "a missed remove does not bump");
    }

    /// D25/T60: the arena enumerates DESCENDING id regardless of
    /// insertion order; removal does not disturb the order of the rest.
    /// A plain-ascending arena fails this pin.
    #[test]
    fn iteration_walks_descending_id() {
        let mut board = Board::new();
        let one = board.alloc_id();
        let two = board.alloc_id();
        let three = board.alloc_id();
        // Insert in a shuffled order — the output order is the map's
        // contract, not the insertion order.
        board.insert_item(pin_entry(two));
        board.insert_item(pin_entry(three));
        board.insert_item(pin_entry(one));
        let ids: Vec<u32> = board
            .iter_descending()
            .map(|entry| entry.id.get())
            .collect();
        assert_eq!(ids, vec![three.get(), two.get(), one.get()]);
        board.remove_item(two);
        let ids: Vec<u32> = board
            .iter_descending()
            .map(|entry| entry.id.get())
            .collect();
        assert_eq!(ids, vec![three.get(), one.get()]);
    }

    /// T61: allocation is at CONSTRUCTION, not insert — an allocated
    /// but never-inserted id stays burned, and the inserted entry keeps
    /// exactly the id it was built with.
    #[test]
    fn alloc_burns_and_insert_preserves_the_id() {
        let mut board = Board::new();
        let burned = board.alloc_id(); // constructed, never inserted
        let kept = board.alloc_id();
        board.insert_item(pin_entry(kept));
        let ids: Vec<u32> = board
            .iter_descending()
            .map(|entry| entry.id.get())
            .collect();
        assert_eq!(ids, vec![kept.get()], "the burned id is absent");
        assert_eq!(kept.get(), burned.get() + 1);
        let next = board.alloc_id();
        assert_eq!(next.get(), kept.get() + 1, "the burn is permanent");
    }

    /// T61: deletion never frees ids — the generator continues past
    /// every deleted id.
    #[test]
    fn ids_continue_monotonically_after_delete() {
        let mut board = Board::new();
        let one = board.alloc_id();
        board.insert_item(pin_entry(one));
        let two = board.alloc_id();
        board.insert_item(pin_entry(two));
        board.remove_item(one);
        let three = board.alloc_id();
        assert_eq!(three.get(), two.get() + 1, "no id reuse after delete");
        board.insert_item(pin_entry(three));
        let ids: Vec<u32> = board
            .iter_descending()
            .map(|entry| entry.id.get())
            .collect();
        assert_eq!(ids, vec![three.get(), two.get()]);
    }

    /// `Item.java:63-64`: the on-the-board flag is false at
    /// construction and true once inserted.
    #[test]
    fn on_the_board_is_set_by_insert() {
        let mut board = Board::new();
        let id = board.alloc_id();
        assert!(!pin_entry(id).on_the_board, "constructed, not inserted");
        board.insert_item(pin_entry(id));
        assert!(board.get(id).expect("inserted").on_the_board);
    }

    /// The generator survives a forced wrap THROUGH the board seam:
    /// `alloc_id` must expose the wrap-to-1 path (T61), not skip it.
    #[test]
    fn board_alloc_id_wraps_at_max_id() {
        let mut board = Board::new();
        board.id_generator.set_next(MAX_ID);
        assert_eq!(board.alloc_id().get(), 1, "wrap to 1");
        assert_eq!(board.alloc_id().get(), 2);
    }

    /// The from_ses_board DSN: outline (id 1) -> structure keepout
    /// (id 2) -> via (id 3) -> a CLOSED UNFIXED wire whose id 4 is
    /// BURNED by the `BasicBoard.java:192-196` drop guard -> an open
    /// wire (id 5). The IR therefore holds ids {1, 2, 3, 5} with a
    /// burned gap at 4 — a renumbering conversion (dense 1..4) fails
    /// the pin.
    const KEEPOUT_BURN_DSN: &str = r#"(pcb keepout-burn.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary
      (path pcb 0  0 0  10000 0  10000 10000  0 10000  0 0)
    )
    (keepout
      (path F.Cu 0  2000 2000  4000 2000  4000 4000  2000 4000  2000 2000)
    )
  )
  (library
    (padstack ViaPad_V
      (shape (circle F.Cu 600))
      (attach off)
    )
  )
  (network
    (net PERFECT)
  )
  (wiring
    (via ViaPad_V 5000 5000 (net PERFECT))
    (wire (path F.Cu 125  1000 1000  3000 1000  3000 3000  1000 1000) (net PERFECT))
    (wire (path F.Cu 125  5000 1000  5000 2000) (net PERFECT))
  )
)
"#;

    /// Parses [`KEEPOUT_BURN_DSN`].
    fn parse_keepout_burn() -> epic_dsn::ses_board::SesBoard {
        let mut ses = epic_dsn::ses_board::SesBoard::new();
        match read_board(KEEPOUT_BURN_DSN.as_bytes(), &mut ses) {
            DsnReadResult::Success { warnings } => {
                assert!(warnings.is_empty(), "WARN_COUNT 0, got {warnings:?}");
            }
            other => panic!("expected Success, got {other:?}"),
        }
        ses
    }

    /// `from_ses_board` preserves the IR ids EXACTLY (gap included),
    /// enumerates them DESCENDING, keeps the kinds (T70), and leaves
    /// the generator one past the parse (T61/T69).
    #[test]
    fn from_ses_board_preserves_ids_kinds_and_generator_position() {
        let ses = parse_keepout_burn();
        let ir_ids: Vec<u32> = ses.items.iter().map(|item| item.id() as u32).collect();
        assert_eq!(
            ir_ids,
            vec![1, 2, 3, 5],
            "outline, keepout, via, open trace"
        );
        assert_eq!(ses.last_assigned_item_id(), 5, "id 4 burned");

        let mut board = Board::from_ses_board(&ses);
        assert_eq!(board.item_count(), ses.items.len());
        let descending: Vec<u32> = board.iter_descending().map(|e| e.id.get()).collect();
        let mut expected = ir_ids.clone();
        expected.reverse();
        assert_eq!(descending, expected, "exact ids, descending order");
        // Every converted entry is ON the board (insert_item set the flag).
        for id in &ir_ids {
            let entry = board.get(ItemId::new(*id)).expect("id preserved");
            assert!(entry.on_the_board, "entry {} was inserted", id);
        }

        // Kinds: the keepout maps to OBSTACLE_AREA (plain kind), the
        // outline to BOARD_OUTLINE.
        assert_eq!(
            board
                .get(ItemId::new(1))
                .expect("outline")
                .board_item_type(),
            BoardItemType::BoardOutline
        );
        let keepout = board.get(ItemId::new(2)).expect("keepout");
        assert_eq!(keepout.board_item_type(), BoardItemType::ObstacleArea);
        assert!(
            matches!(
                keepout.data,
                ItemData::ObstacleArea {
                    kind: ObstacleKind::ObstacleArea,
                    ..
                }
            ),
            "T70: the plain keepout keeps its distinct kind"
        );
        assert_eq!(keepout.fixed, FixedState::SystemFixed, "structure keepout");
        assert!(keepout.nets.is_empty(), "parse keepouts carry no nets");

        // The via carries the IR's fields untouched.
        let ItemIr::Via { via: ir_via, .. } = &ses.items[2] else {
            panic!("IR item 3 is the via");
        };
        let via_entry = board.get(ItemId::new(3)).expect("via");
        assert_eq!(via_entry.board_item_type(), BoardItemType::Via);
        assert_eq!(via_entry.nets, ir_via.nets, "via nets");
        assert_eq!(via_entry.component_id, 0, "vias belong to no component");
        match &via_entry.data {
            ItemData::Via {
                center,
                padstack_no,
                attach_smd_allowed,
            } => {
                assert_eq!(*center, ir_via.location);
                assert_eq!(*padstack_no, ir_via.padstack_no);
                assert_eq!(*attach_smd_allowed, ir_via.attach_smd_allowed);
            }
            other => panic!("expected a via payload, got {other:?}"),
        }

        let trace = board.get(ItemId::new(5)).expect("trace");
        assert_eq!(trace.board_item_type(), BoardItemType::Trace);
        assert_eq!(trace.nets, vec![1]);
        if let ItemData::Trace { lines, .. } = &trace.data {
            assert_eq!(lines.corner_count(), 2, "two IR corners -> two corners");
        } else {
            panic!("expected a trace payload");
        }

        // The generator continues one past the parse.
        assert_eq!(board.alloc_id().get(), 6, "next id continues the parse");
    }
}
