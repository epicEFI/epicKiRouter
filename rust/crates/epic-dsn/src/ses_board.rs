//! The parse-derivable mini board model (plan decision D9):
//! [`SesBoard`] is the concrete [`BoardSink`] the M1b reader assembles —
//! layers, net table, placements, padstack registry, sequential-id items,
//! clearance IR, warnings, metadata, transform. NO normalization, NO
//! contact sets, NO router types; epic-board (M2) will implement the same
//! trait against the real model.
//!
//! ## Item ids (jar-verified)
//!
//! Ids are sequential from 1 in insertion order; the `BasicBoard`
//! constructor's `BoardOutline` takes id 1 at `create_board`
//! (`BasicBoard.java:136`), structure keepouts / outline holes /
//! network pins / wiring traces-vias-conduction areas follow in scope
//! order, and package component outlines (`Network.java:1192`) burn ids
//! too. [`SesBoard::items`] stores ASCENDING id order (parse order);
//! every descending-id consumer (digest T39, SES network_out) walks it in
//! reverse. End-to-end jar session `/tmp/epic-t4-ids.jsh`, output
//! `/tmp/epic-t4-ids.out`: a fixture with 1 keepout, 2 pins, 2 traces
//! (route + fix), 1 via and 1 rectangle wire yields ids 1..8 with
//! `GEN_MAX 8` — mirrored by
//! `tests::board_sink_id_assignment_matches_jar_spike`.
//!
//! ## Registry semantics (bug-compatible)
//!
//! - Padstacks: `add` ALWAYS appends (`Library.java:222`), the number is
//!   `size() + 1` (1-based); the dedup guard is at the reader call site
//!   (`Library.java:158-161`) on the name ALREADY stripped at `:113`, so
//!   DSN library padstacks arrive pre-stripped (jar
//!   `/tmp/epic-t4b-review.out`: a `(padstack viapad_f.1 ...)` scope adds
//!   NOTHING beside `ViaPad_F` — `PADSTACK_COUNT 2`). Name queries strip
//!   then match case-insensitively, first match
//!   ([`SesBoard::resolve_padstack_query`]; `Wiring.java:659-660`,
//!   `Network.java:1292-1294`, `Library.java:321-323`); the via-info rule
//!   fallback queries the RAW name ([`BoardSink::padstack_no`],
//!   `Network.java:275`).
//! - Nets: TWO layers. The parser `NetList` is a case-SENSITIVE TreeMap
//!   keyed by `Net.Id(name, subnetNumber)` (`NetList.java:13-33`;
//!   `compareTo` = `String.compareTo`, `parser/Net.java:95-98`) whose
//!   contains-guard (`Network.java:1410`) decides is-new. The board table
//!   (`rules/Nets.java:88-96`) ALWAYS appends — `GND` and `gnd` are two
//!   nets and an explicit positional subnet number is carried (jar
//!   `/tmp/epic-t4b-review.out`: nets 1, 2 and VDD/2 as net 3); an exact
//!   duplicate scope adds nothing because the READER skipped it. The
//!   parse netlist (T35) is the reader-side
//!   [`ParseState`](crate::state::ParseState)::netlist, not this table.
//! - Clearance classes: the `create_board` default instance is
//!   `["null", "default"]` with a zero matrix (`ClearanceMatrix
//!   .getDefaultInstance(layerStructure, 0)`, `Structure.java:1236`);
//!   `get_no` is case-insensitive with a -1 (here `None`) miss. The
//!   exact `append_class` dedup rule is pinned by the Task 5 clearance
//!   pins; the skeleton here appends only unknown names (case-insensitive
//!   check) and grows the matrix with zeros.
//! - Components: id = `size() + 1` (`Network.java:957-966`).

use crate::coordinate_transform::CoordinateTransform;
use crate::layer_structure::LayerStructure;
use crate::sink::{
    BoardOutlineIr, BoardSink, ComponentIr, ComponentOutlineIr, ConductionAreaIr, CreateBoardIr,
    FixedStateIr, KeepoutIr, MetadataIr, NetClassIr, NetIr, PadstackIr, PinIr, PlacementIr,
    TraceIr, ViaInfoIr, ViaIr,
};
use crate::state::AngleRestriction;

/// A parse-time board item with its Java `Item.getId()` board id
/// (`board/model/items/Item.java:87` — the sink assigns the id on
/// insertion; see the [`sink`] module docs for the jar evidence).
#[derive(Clone, Debug, PartialEq)]
pub enum ItemIr {
    /// A wiring trace (`Wiring.java:534`/`:564`).
    Trace {
        /// The board id.
        id: i32,
        /// The trace payload.
        trace: TraceIr,
    },
    /// A via (`Wiring.java:706`).
    Via {
        /// The board id.
        id: i32,
        /// The via payload.
        via: ViaIr,
    },
    /// A component pin (`Network.java:1035`) — occupies an id, emits no
    /// digest line.
    Pin {
        /// The board id.
        id: i32,
        /// The pin payload.
        pin: PinIr,
    },
    /// A keepout obstacle (structure, outline hole or package keepout).
    Keepout {
        /// The board id.
        id: i32,
        /// The keepout payload.
        keepout: KeepoutIr,
    },
    /// A conduction area (plane or rectangle wire).
    ConductionArea {
        /// The board id.
        id: i32,
        /// The conduction-area payload.
        area: ConductionAreaIr,
    },
    /// A package component outline (`Network.java:1192`) — burns an id,
    /// emits no digest line.
    ComponentOutline {
        /// The board id.
        id: i32,
        /// The outline payload.
        outline: ComponentOutlineIr,
    },
    /// The board outline created at `create_board`
    /// (`BasicBoard.java:136`) — id 1 of every parse, emits no digest
    /// line.
    BoardOutline {
        /// The board id.
        id: i32,
        /// The outline payload.
        outline: BoardOutlineIr,
    },
}

impl ItemIr {
    /// The board id.
    pub fn id(&self) -> i32 {
        match self {
            ItemIr::Trace { id, .. }
            | ItemIr::Via { id, .. }
            | ItemIr::Pin { id, .. }
            | ItemIr::Keepout { id, .. }
            | ItemIr::ConductionArea { id, .. }
            | ItemIr::ComponentOutline { id, .. }
            | ItemIr::BoardOutline { id, .. } => *id,
        }
    }
}

/// Java `ClearanceMatrix` parse skeleton: class names in matrix order and
/// the `values[i][j]` clearance between classes `i` and `j` (board units).
#[derive(Clone, Debug, PartialEq)]
pub struct ClearanceIr {
    /// Java `ClearanceMatrix` class names, matrix column/row order.
    pub names: Vec<String>,
    /// The clearance values; `values.len() == names.len()` and every row
    /// matches.
    pub values: Vec<Vec<i32>>,
}

impl ClearanceIr {
    /// Java `ClearanceMatrix.getDefaultInstance(layerStructure, 0)`
    /// (`Structure.java:1236`): the two built-in classes `"null"` and
    /// `"default"`, zero matrix.
    pub fn default_instance() -> Self {
        Self {
            names: vec!["null".to_string(), "default".to_string()],
            values: vec![vec![0, 0], vec![0, 0]],
        }
    }
}

/// The parse-derivable board model (module docs).
#[derive(Clone, Debug, PartialEq)]
pub struct SesBoard {
    /// Parse metadata snapshot (unit/resolution/quote/snap angle/hosts).
    pub metadata: MetadataIr,
    /// The finalized coordinate transform (`None` before create_board).
    pub transform: Option<CoordinateTransform>,
    /// The layer structure (`None` before create_board).
    pub layers: Option<LayerStructure>,
    /// The net table in NUMBER order (net no = position + 1).
    pub nets: Vec<NetIr>,
    /// The placement table in insertion (file) order.
    pub placements: Vec<PlacementIr>,
    /// The padstack registry in insertion order (raw names; T32 strip is
    /// query-side only).
    pub padstacks: Vec<PadstackIr>,
    /// The component table (id = position + 1).
    pub components: Vec<ComponentIr>,
    /// Net classes (Task 8 fills field-for-field).
    pub net_classes: Vec<NetClassIr>,
    /// Via infos (net-class via rules, Task 8).
    pub via_infos: Vec<ViaInfoIr>,
    /// The clearance class table + matrix.
    pub clearance: ClearanceIr,
    /// The items in ASCENDING id (parse) order; descending-id consumers
    /// walk `.iter().rev()` (T39).
    pub items: Vec<ItemIr>,
    /// Exact parity warnings (D12: the digest digit-normalizes).
    pub warnings: Vec<String>,
    /// The id the NEXT inserted item gets (starts at 1; mirrors
    /// `ItemIdGenerator.lastGeneratedId + 1`).
    next_item_id: i32,
}

impl Default for SesBoard {
    fn default() -> Self {
        Self::new()
    }
}

/// Java `String.equalsIgnoreCase` for the board-side lookups
/// (`rules/Nets.java:39,51`, `Padstacks.get`, `ClearanceMatrix.get_no`):
/// simple lowercase folding on both sides agrees with Java's per-char
/// loop for the corpus name space (ASCII identifiers; a full UTF-16
/// per-char port lands only if a fixture ever diverges).
fn eq_ignore_case(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// Java `replaceAll("\\.\\d+", "")` (T32): removes EVERY dot followed by
/// a maximal ASCII-digit run, anywhere in the name — `ViaPad_F.1` ->
/// `ViaPad_F`, `x.1.2` -> `x`, `a.1b` -> `ab`, a trailing `.` with no
/// digits stays (`\.\d+` needs at least one digit).
pub fn strip_padstack_alias(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::with_capacity(name.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '.' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit() {
            // consume '.' plus the greedy digit run
            i += 1;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

impl SesBoard {
    /// An empty board (no `create_board` yet); metadata carries the
    /// parse-state defaults.
    pub fn new() -> Self {
        Self {
            metadata: MetadataIr::default(),
            transform: None,
            layers: None,
            nets: Vec::new(),
            placements: Vec::new(),
            padstacks: Vec::new(),
            components: Vec::new(),
            net_classes: Vec::new(),
            via_infos: Vec::new(),
            clearance: ClearanceIr {
                names: Vec::new(),
                values: Vec::new(),
            },
            items: Vec::new(),
            warnings: Vec::new(),
            next_item_id: 1,
        }
    }

    /// The last item id handed out (`ItemIdGenerator.maxGeneratedId()` in
    /// the spike — 8 for the fixture of `/tmp/epic-t4-ids.out`).
    pub fn last_assigned_item_id(&self) -> i32 {
        self.next_item_id - 1
    }

    /// The 1-based padstack name, `None` (Java null) out of range.
    pub fn padstack_name(&self, padstack_no: i32) -> Option<&str> {
        let index = usize::try_from(padstack_no.checked_sub(1)?).ok()?;
        self.padstacks.get(index).map(|p| p.name.as_str())
    }

    /// The 1-based net name, `None` (Java null) out of range.
    pub fn net_name(&self, net_no: i32) -> Option<&str> {
        let index = usize::try_from(net_no.checked_sub(1)?).ok()?;
        self.nets.get(index).map(|net| net.name.as_str())
    }

    fn alloc_item_id(&mut self) -> i32 {
        let id = self.next_item_id;
        self.next_item_id += 1;
        id
    }
}

impl BoardSink for SesBoard {
    fn create_board(&mut self, board: CreateBoardIr) {
        self.transform = Some(board.transform);
        self.layers = Some(board.layer_structure);
        self.clearance = ClearanceIr::default_instance();
        // The outline class name resolves against the fresh default table;
        // the miss behavior (unresolvable name) is pinned by the Task 5
        // boundary pins (Issue558 context) — skeleton falls back to the
        // "default" index 1.
        let outline_class = self
            .clearance_class_no(&board.outline_clearance_class)
            .unwrap_or(1);
        let id = self.alloc_item_id();
        self.items.push(ItemIr::BoardOutline {
            id,
            outline: BoardOutlineIr {
                shapes: board.outline_shapes,
                clearance_class: outline_class,
                fixed: FixedStateIr::SystemFixed,
            },
        });
    }

    fn append_net(&mut self, net: NetIr) -> i32 {
        // Nets.add (rules/Nets.java:88-96): ALWAYS appends, number =
        // nets.size() + 1 — no lookup, no merge. The case-sensitive
        // is-new guard is the reader's (Network.java:1410); merging here
        // would shift every later net number (review finding, Task 4).
        self.nets.push(net);
        self.nets.len() as i32
    }

    fn net_no(&self, name: &str) -> Option<i32> {
        self.nets
            .iter()
            .position(|net| eq_ignore_case(&net.name, name))
            .map(|index| index as i32 + 1)
    }

    fn append_clearance_class(&mut self, name: &str) {
        if self.clearance_class_no(name).is_some() {
            return;
        }
        for row in &mut self.clearance.values {
            row.push(0);
        }
        self.clearance
            .values
            .push(vec![0; self.clearance.names.len() + 1]);
        self.clearance.names.push(name.to_string());
    }

    fn clearance_class_no(&self, name: &str) -> Option<i32> {
        self.clearance
            .names
            .iter()
            .position(|existing| eq_ignore_case(existing, name))
            .map(|index| index as i32)
    }

    fn append_padstack(&mut self, padstack: PadstackIr) -> i32 {
        self.padstacks.push(padstack);
        self.padstacks.len() as i32
    }

    fn padstack_no(&self, name: &str) -> Option<i32> {
        // Padstacks.get(String): RAW name (NO `.N` strip), case-sensitive
        // NO — the Java lookup is a case-INSENSITIVE first match
        // (core/library/Padstacks.java:25-32); the via-info rule fallback
        // consumer is Network.java:275.
        self.padstacks
            .iter()
            .position(|padstack| eq_ignore_case(&padstack.name, name))
            .map(|index| index as i32 + 1)
    }

    fn resolve_padstack_query(&self, name: &str) -> Option<i32> {
        let stripped = strip_padstack_alias(name);
        self.padstacks
            .iter()
            .position(|padstack| eq_ignore_case(&padstack.name, &stripped))
            .map(|index| index as i32 + 1)
    }

    fn insert_net_class(&mut self, net_class: NetClassIr) {
        self.net_classes.push(net_class);
    }

    fn append_via_info(&mut self, via_info: ViaInfoIr) {
        self.via_infos.push(via_info);
    }

    fn set_flip_style(&mut self, flip_style: String) {
        self.metadata.flip_style = Some(flip_style);
    }

    fn set_snap_angle(&mut self, snap_angle: AngleRestriction) {
        self.metadata.snap_angle = snap_angle;
    }

    fn append_placement(&mut self, placement: PlacementIr) {
        self.placements.push(placement);
    }

    fn insert_component(&mut self, component: ComponentIr) -> i32 {
        self.components.push(component);
        self.components.len() as i32
    }

    fn insert_pin(&mut self, pin: PinIr) -> i32 {
        let id = self.alloc_item_id();
        self.items.push(ItemIr::Pin { id, pin });
        id
    }

    fn insert_component_outline(&mut self, outline: ComponentOutlineIr) -> i32 {
        let id = self.alloc_item_id();
        self.items.push(ItemIr::ComponentOutline { id, outline });
        id
    }

    fn insert_keepout(&mut self, keepout: KeepoutIr) -> i32 {
        let id = self.alloc_item_id();
        self.items.push(ItemIr::Keepout { id, keepout });
        id
    }

    fn insert_conduction_area(&mut self, area: ConductionAreaIr) -> i32 {
        let id = self.alloc_item_id();
        self.items.push(ItemIr::ConductionArea { id, area });
        id
    }

    fn insert_trace(&mut self, trace: TraceIr) -> i32 {
        let id = self.alloc_item_id();
        self.items.push(ItemIr::Trace { id, trace });
        id
    }

    fn insert_via(&mut self, via: ViaIr) -> i32 {
        let id = self.alloc_item_id();
        self.items.push(ItemIr::Via { id, via });
        id
    }

    fn push_warning(&mut self, warning: String) {
        self.warnings.push(warning);
    }

    fn set_metadata(&mut self, metadata: MetadataIr) {
        self.metadata = metadata;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shape::BoardShape;
    use crate::sink::{AreaIr, KeepoutKindIr};
    use epic_geometry::int_box::IntBox;
    use epic_geometry::int_point::IntPoint;
    use epic_geometry::point::Point;
    use epic_geometry::polygon_shape::PolygonShape;

    /// The layer structure of the spike fixture
    /// (`/tmp/epic-t4-ids.dsn`: F.Cu, B.Cu signal layers).
    fn spike_layers() -> LayerStructure {
        LayerStructure::new(vec![
            crate::layer_structure::Layer::new("F.Cu", 0, true),
            crate::layer_structure::Layer::new("B.Cu", 1, true),
        ])
    }

    /// Replays `/tmp/epic-t4-ids.dsn` (scale 10, base 0) through the
    /// sink in Java scope order: create_board -> structure keepout ->
    /// network pins -> wiring traces/via/rectangle wire. The expected
    /// ids are the jar output `/tmp/epic-t4-ids.out`.
    fn spike_board() -> SesBoard {
        let mut board = SesBoard::new();
        board.create_board(CreateBoardIr {
            bounding_box: IntBox::new(IntPoint::new(0, 0), IntPoint::new(100_000, 100_000)),
            layer_structure: spike_layers(),
            outline_shapes: Vec::new(),
            outline_clearance_class: "default".to_string(),
            transform: crate::coordinate_transform::CoordinateTransform::new(10.0, 0.0, 0.0),
        });
        board.append_padstack(PadstackIr {
            name: "ViaPad_F".to_string(),
            shapes: Vec::new(),
            drillable: true,
            placed_absolute: false,
        });
        board.append_padstack(PadstackIr {
            name: "CirclePad_F_800_um".to_string(),
            shapes: Vec::new(),
            drillable: false,
            placed_absolute: false,
        });
        assert_eq!(
            board.append_net(NetIr {
                name: "PERFECT".to_string(),
                subnet_number: 1,
            }),
            1
        );

        // structure keepout: polygon F.Cu 1000..3000 -> x10
        let corners: Vec<Point> = [
            (10_000, 10_000),
            (30_000, 10_000),
            (30_000, 30_000),
            (10_000, 30_000),
        ]
        .iter()
        .map(|&(x, y)| Point::Int(IntPoint::new(x, y)))
        .collect();
        board.insert_keepout(KeepoutIr {
            kind: KeepoutKindIr::Keepout,
            layer_no: 0,
            area: AreaIr::simple(BoardShape::PolygonShape(PolygonShape::new(&corners))),
            clearance_class: 1,
            fixed: FixedStateIr::SystemFixed,
        });

        // network scope: component + pin insertions in parse order
        let u1 = board.insert_component(ComponentIr {
            name: "U1".to_string(),
            placement: 0,
        });
        let u2 = board.insert_component(ComponentIr {
            name: "U2".to_string(),
            placement: 1,
        });
        board.insert_pin(PinIr {
            component_id: u1,
            pin_index: 0,
            padstack_no: 2,
            center: IntPoint::new(20_000, 50_000),
            nets: vec![1],
            clearance_class: 1,
            fixed: FixedStateIr::Unfixed,
        });
        board.insert_pin(PinIr {
            component_id: u2,
            pin_index: 0,
            padstack_no: 2,
            center: IntPoint::new(80_000, 50_000),
            nets: vec![1],
            clearance_class: 1,
            fixed: FixedStateIr::Unfixed,
        });

        // wiring scope: (type route) trace, (type fix) trace, via,
        // rectangle wire -> conduction area
        board.insert_trace(TraceIr {
            layer_no: 0,
            half_width: 625,
            corners: vec![IntPoint::new(20_000, 50_000), IntPoint::new(80_000, 50_000)],
            nets: vec![1],
            clearance_class: 1,
            fixed: FixedStateIr::UserFixed,
        });
        board.insert_trace(TraceIr {
            layer_no: 0,
            half_width: 625,
            corners: vec![IntPoint::new(20_000, 51_000), IntPoint::new(80_000, 51_000)],
            nets: vec![1],
            clearance_class: 1,
            fixed: FixedStateIr::SystemFixed,
        });
        board.insert_via(ViaIr {
            padstack_no: 1,
            location: IntPoint::new(50_000, 50_000),
            nets: vec![1],
            clearance_class: 1,
            fixed: FixedStateIr::Unfixed,
            attach_smd_allowed: false,
        });
        board.insert_conduction_area(ConductionAreaIr {
            layer_no: 0,
            area: AreaIr::simple(crate::shape::BoardShape::Tile(
                epic_geometry::tile_shape::TileShape::RegularTileShape(
                    epic_geometry::regular_tile_shape::RegularTileShape::IntBox(IntBox::new(
                        IntPoint::new(40_000, 40_000),
                        IntPoint::new(60_000, 60_000),
                    )),
                ),
            )),
            nets: vec![1],
            clearance_class: 1,
            fixed: FixedStateIr::Unfixed,
        });
        board
    }

    /// Jar output `/tmp/epic-t4-ids.out` (session
    /// `/tmp/epic-t4-ids.jsh`): ids 1..8 dense and in scope order —
    /// 1 BoardOutline, 2 ObstacleArea (keepout, SYSTEM_FIXED), 3-4 Pins,
    /// 5 trace USER_FIXED (`(type route)`), 6 trace SYSTEM_FIXED
    /// (`(type fix)`), 7 Via, 8 ConductionArea UNFIXED; GEN_MAX 8;
    /// COMPONENTS 2. The descending enumeration of the jar
    /// (`board.getItems()`) is the T39 order the digest walks.
    #[test]
    fn board_sink_id_assignment_matches_jar_spike() {
        let board = spike_board();
        let ids: Vec<i32> = board.items.iter().map(ItemIr::id).collect();
        assert_eq!(ids, vec![1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(board.last_assigned_item_id(), 8);
        assert_eq!(board.components.len(), 2);
        match &board.items[0] {
            ItemIr::BoardOutline { id, .. } => assert_eq!(*id, 1),
            other => panic!("expected BoardOutline first, got {other:?}"),
        }
        match &board.items[1] {
            ItemIr::Keepout { id, keepout } => {
                assert_eq!(*id, 2);
                assert_eq!(keepout.fixed, FixedStateIr::SystemFixed);
            }
            other => panic!("expected Keepout second, got {other:?}"),
        }
        match &board.items[4] {
            ItemIr::Trace { id, trace } => {
                assert_eq!(*id, 5);
                assert_eq!(trace.fixed, FixedStateIr::UserFixed);
            }
            other => panic!("expected route trace fifth, got {other:?}"),
        }
        match &board.items[5] {
            ItemIr::Trace { id, trace } => {
                assert_eq!(*id, 6);
                assert_eq!(trace.fixed, FixedStateIr::SystemFixed);
            }
            other => panic!("expected fix trace sixth, got {other:?}"),
        }
        assert!(matches!(board.items[6], ItemIr::Via { id: 7, .. }));
        match &board.items[7] {
            ItemIr::ConductionArea { id, area } => {
                assert_eq!(*id, 8);
                assert_eq!(area.fixed, FixedStateIr::Unfixed);
            }
            other => panic!("expected ConductionArea last, got {other:?}"),
        }
        // descending walk = the jar enumeration order (T39)
        let descending: Vec<i32> = board.items.iter().rev().map(ItemIr::id).collect();
        assert_eq!(descending, vec![8, 7, 6, 5, 4, 3, 2, 1]);
    }

    /// `create_board` initializes the default clearance instance
    /// (`Structure.java:1236`) and the outline falls in the `"default"`
    /// class (index 1).
    #[test]
    fn create_board_default_clearance_and_outline_class() {
        let board = spike_board();
        assert_eq!(
            board.clearance,
            ClearanceIr::default_instance(),
            "default instance is [null, default] with a zero matrix"
        );
        match &board.items[0] {
            ItemIr::BoardOutline { outline, .. } => {
                assert_eq!(outline.clearance_class, 1);
                assert_eq!(outline.fixed, FixedStateIr::SystemFixed);
            }
            other => panic!("expected BoardOutline, got {other:?}"),
        }
        assert!(board.transform.is_some());
        assert!(board.layers.is_some());
    }

    /// The two-layer net rule, pinned to the jar probe
    /// `/tmp/epic-t4b-review.jsh`/`.out` (fixture
    /// `/tmp/epic-t4b-review.dsn`): the SINK always appends
    /// (`rules/Nets.java:88-96`) — case variants `GND`/`gnd` become nets
    /// 1 AND 2 (the case-sensitive contains-guard lives in the parser
    /// NetList, `Network.java:1410`), the explicit positional subnet
    /// integer is carried (`(net VDD 2 ...)` -> subnet 2, net 3), and
    /// `net_no` looks names up case-insensitively (Java `Nets.get`,
    /// `rules/Nets.java:42-51`). A merge-at-append port would produce 2
    /// nets here instead of 3 and shift every downstream net number.
    #[test]
    fn append_net_always_appends_subnet_jar_probe() {
        let mut board = SesBoard::new();
        assert_eq!(
            board.append_net(NetIr {
                name: "GND".to_string(),
                subnet_number: 1,
            }),
            1
        );
        assert_eq!(
            board.append_net(NetIr {
                name: "gnd".to_string(),
                subnet_number: 1,
            }),
            2
        );
        assert_eq!(
            board.append_net(NetIr {
                name: "VDD".to_string(),
                subnet_number: 2,
            }),
            3
        );
        assert_eq!(board.nets.len(), 3);
        assert_eq!(board.net_name(1), Some("GND"));
        assert_eq!(board.net_name(2), Some("gnd"));
        assert_eq!(board.net_name(3), Some("VDD"));
        assert_eq!(
            board
                .nets
                .iter()
                .map(|net| net.subnet_number)
                .collect::<Vec<_>>(),
            vec![1, 1, 2],
            "jar NET lines: subnet=1, subnet=1, subnet=2"
        );
        assert_eq!(board.net_no("vdd"), Some(3));
        assert_eq!(board.net_no("missing"), None);
    }

    /// Padstack registry (T32): `add` always appends (1-based numbers);
    /// the reader's library-scope dedup queries the scope name stripped
    /// of every `.digits` run (`Library.java:113` + `:158-161`) THEN
    /// matches case-insensitively over the registry — jar
    /// `/tmp/epic-t4b-review.out`: scopes `ViaPad_F`, `viapad_f.1`,
    /// `ViaPad_F`, `CirclePad_F_800_um` produce `PADSTACK_COUNT 2` and
    /// the `(via ViaPad_F.1 ...)` resolves to padstack 1 (first-match).
    /// The RAW query ([`BoardSink::padstack_no`], via-info fallback
    /// `Network.java:275`) does NOT strip: `viapad_f.1` misses exactly
    /// where the strip query hits — the discriminating pair. Strip
    /// semantics of Java `replaceAll("\\.\\d+", "")`: `x.1.2` -> `x`,
    /// `a.1b` -> `ab`, trailing bare `.` survives.
    #[test]
    fn padstack_registry_and_t32_query_strip() {
        assert_eq!(strip_padstack_alias("ViaPad_F.1"), "ViaPad_F");
        assert_eq!(strip_padstack_alias("x.1.2"), "x");
        assert_eq!(strip_padstack_alias("a.1b"), "ab");
        assert_eq!(strip_padstack_alias("trailing."), "trailing.");
        assert_eq!(strip_padstack_alias("no_alias"), "no_alias");

        // The reader's library-scope flow (Library.java:113 strip, :158
        // guard, :222 append): strip the scope name, skip on query hit,
        // append otherwise.
        fn append_scope(board: &mut SesBoard, scope_name: &str) {
            let stripped = strip_padstack_alias(scope_name);
            if board.resolve_padstack_query(&stripped).is_none() {
                board.append_padstack(PadstackIr {
                    name: stripped,
                    shapes: Vec::new(),
                    drillable: true,
                    placed_absolute: false,
                });
            }
        }
        let mut board = SesBoard::new();
        append_scope(&mut board, "ViaPad_F");
        append_scope(&mut board, "viapad_f.1");
        append_scope(&mut board, "ViaPad_F");
        append_scope(&mut board, "CirclePad_F_800_um");
        assert_eq!(board.padstacks.len(), 2, "jar: PADSTACK_COUNT 2");
        assert_eq!(board.padstack_name(1), Some("ViaPad_F"));
        assert_eq!(board.padstack_name(2), Some("CirclePad_F_800_um"));
        assert_eq!(board.padstack_name(3), None);

        // Wiring-side queries strip the alias first (`Wiring.java:659`):
        // `ViaPad_F.1` -> `viapad_f` -> the FIRST case-insensitive match.
        assert_eq!(board.resolve_padstack_query("ViaPad_F.1"), Some(1));
        assert_eq!(board.resolve_padstack_query("viapad_f"), Some(1));
        assert_eq!(
            board.resolve_padstack_query("CIRCLEPAD_F_800_UM.12"),
            Some(2)
        );
        assert_eq!(board.resolve_padstack_query("NoSuch.3"), None);
        // The raw via-info fallback (Network.java:275) does NOT strip.
        assert_eq!(board.padstack_no("ViaPad_F"), Some(1));
        assert_eq!(board.padstack_no("viapad_f.1"), None);
        assert_eq!(board.padstack_no("circlepad_f_800_um"), Some(2));
    }

    /// Clearance class append: unknown names append (matrix grows with
    /// zeros), known names (case-insensitive, like `get_no`) are no-ops.
    #[test]
    fn clearance_class_append_dedups() {
        let mut board = spike_board();
        board.append_clearance_class("power");
        assert_eq!(board.clearance_class_no("POWER"), Some(2));
        assert_eq!(board.clearance.names, vec!["null", "default", "power"]);
        assert_eq!(board.clearance.values.len(), 3);
        assert!(board.clearance.values.iter().all(|row| row.len() == 3));
        board.append_clearance_class("default");
        assert_eq!(board.clearance.names.len(), 3, "dedup: no second default");
    }

    /// A fresh board has no items and no transform; the component/placement
    /// tables number 1-based.
    #[test]
    fn fresh_board_and_component_numbering() {
        let mut board = SesBoard::new();
        assert!(board.items.is_empty());
        assert_eq!(board.last_assigned_item_id(), 0);
        assert_eq!(board.transform, None);
        assert_eq!(board.metadata.unit, crate::state::Unit::Mil);
        assert_eq!(board.metadata.resolution, 100);
        board.append_placement(PlacementIr {
            lib_name: "PAD".to_string(),
            location: crate::state::ComponentLocation {
                name: "U1".to_string(),
                coor: [2000.0, 5000.0],
                is_front: true,
                rotation: 0.0,
                position_fixed: false,
                pin_infos: Default::default(),
                keepout_infos: Default::default(),
                via_keepout_infos: Default::default(),
                place_keepout_infos: Default::default(),
                part_number: None,
            },
        });
        assert_eq!(
            board.insert_component(ComponentIr {
                name: "U1".to_string(),
                placement: 0,
            }),
            1
        );
        assert_eq!(
            board.insert_component(ComponentIr {
                name: "U2".to_string(),
                placement: 0,
            }),
            2
        );
    }

    /// The shape IR round-trips through the keepout area untouched
    /// (polygon corners stay in file order for ccw input — the
    /// `PolygonShape` constructor normalizes only cw input away).
    #[test]
    fn keepout_area_keeps_ccw_corner_order() {
        let board = spike_board();
        match &board.items[1] {
            ItemIr::Keepout { keepout, .. } => match &keepout.area.border {
                BoardShape::PolygonShape(polygon) => {
                    let corners: Vec<(i32, i32)> = (0..polygon.border_line_count() as i32)
                        .map(|i| match polygon.corner(i) {
                            Point::Int(p) => (p.x, p.y),
                            other => panic!("expected int corner, got {other:?}"),
                        })
                        .collect();
                    assert_eq!(
                        corners,
                        vec![
                            (10_000, 10_000),
                            (30_000, 10_000),
                            (30_000, 30_000),
                            (10_000, 30_000)
                        ]
                    );
                }
                other => panic!("expected PolygonShape, got {other:?}"),
            },
            other => panic!("expected Keepout, got {other:?}"),
        }
    }
}
