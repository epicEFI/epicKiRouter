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
//! - Packages (library images): an insertion-ordered table mirroring Java
//!   `Packages` (`Packages.java:14`); `insert_package` ALWAYS appends
//!   (`Packages.java:69-94`, id = `size() + 1`). The duplicate-image
//!   dedup is READER-side (`Library.java:409-449`): retry the base name
//!   as `NAME::1`, `NAME::2`, ... and insert under the first name whose
//!   `Packages.get` hit does not already carry that exact stored name
//!   (`:417`) — the `::N`-strip inside `get` is what terminates the loop
//!   — skipping entirely when the existing package's pins are identical
//!   (`arePackagePinsIdentical`, `Library.java:226-259`: pin count, name
//!   `String.equals`, padstack id, location/rotation within 0.001). Jar
//!   `/tmp/epic-t4c-images.out`: identical-pin dup deduped
//!   (PACKAGE_COUNT stays 1), different-pin dup inserted as `PAD::1`,
//!   case-variant same-pin `pad` deduped into `PAD`.
//! - Via-padstack list: a SECOND index over the padstack registry (the
//!   routing-eligible subset); `SesWriter`'s library section walks THE
//!   LIST, not the registry. `set_via_padstacks` REPLACES it wholesale
//!   (`BoardLibrary.java:81-84`; network tail `Network.java:1273-1313`,
//!   `.N`-cleaned names, warn + drop misses, fires whenever any net
//!   class was seen — a class without `(circuit (use_via ...))` WIPES
//!   the list, jar `/tmp/epic-t4c-images.out` VIAPADSTACK_COUNT 0);
//!   `append_via_padstack` is the via-info path (`Network.java:272-284`
//!   via `addViaPadstack`, `BoardLibrary.java:90-101`), deduped by
//!   case-SENSITIVE name over the list. `via_padstack_no` is the
//!   case-sensitive `getViaPadstack(String)` (`:53-62`).
//! - `create_board` runs ONCE per board: Java re-invoking the structure
//!   reader's `createBoard` is guarded by
//!   `if (getRoutingBoard() == null)` (`Structure.java:1034-1036`), so a
//!   second `(structure ...)` scope re-parses its layers/rules/keepouts
//!   but does NOT rebuild the board (jar `/tmp/epic-t4c-twostruct.out`:
//!   two structure scopes yield ONE outline, items dense 1..5, the
//!   second boundary ignored). The sink mirrors the guard by
//!   early-returning once `transform` is set.

use crate::coordinate_transform::CoordinateTransform;
use crate::layer_structure::LayerStructure;
use crate::shape::BoardShape;
use crate::sink::{
    AreaIr, BoardOutlineIr, BoardRulesIr, BoardSink, ComponentIr, ComponentOutlineIr,
    ConductionAreaIr, CreateBoardIr, FixedStateIr, ImageIr, ImagePinIr, ItemClassIr, KeepoutIr,
    LogicalPartIr, MetadataIr, NetClassIr, NetIr, PadstackIr, PinIr, PlacementIr, TraceIr,
    ViaInfoIr, ViaIr, ViaRuleIr, eq_ignore_case,
};
use crate::state::AngleRestriction;
use epic_geometry::float_point::FloatPoint;
use epic_geometry::int_box::IntBox;
use epic_geometry::int_point::IntPoint;
use epic_geometry::int_vector::IntVector;
use epic_geometry::vector::Vector;

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

/// The parse-derivable board model (module docs).
#[derive(Clone, Debug, PartialEq)]
pub struct SesBoard {
    /// Parse metadata snapshot (unit/resolution/quote/snap angle/hosts).
    pub metadata: MetadataIr,
    /// The finalized coordinate transform (`None` before create_board).
    pub transform: Option<CoordinateTransform>,
    /// The layer structure (`None` before create_board).
    pub layers: Option<LayerStructure>,
    /// Java `Board.boundingBox` — the createBoard bounds INCLUDING the
    /// offset(1000) (T43, `Structure.java:1207-1208`); `None` before
    /// create_board.
    pub bounding_box: Option<IntBox>,
    /// The net table in NUMBER order (net no = position + 1).
    pub nets: Vec<NetIr>,
    /// The placement table in insertion (file) order.
    pub placements: Vec<PlacementIr>,
    /// The padstack registry in insertion order (raw names; T32 strip is
    /// query-side only).
    pub padstacks: Vec<PadstackIr>,
    /// The library image/package table in insertion (file) order; the
    /// package id is the 1-based position (`Packages.java:14`,
    /// `:69-94`). Task 8 pin expansion and Task 13 placement grouping
    /// walk this in LIBRARY order.
    pub packages: Vec<ImageIr>,
    /// The via-padstack list: the routing-eligible subset of
    /// [`SesBoard::padstacks`], as 1-based registry numbers in list
    /// order (0-based via indexes in Java, `BoardLibrary.java:45-50`).
    /// REPLACED wholesale by `set_via_padstacks`, appended by
    /// `append_via_padstack` — see the trait docs for the two
    /// construction paths.
    pub via_padstacks: Vec<i32>,
    /// The component table (id = position + 1).
    pub components: Vec<ComponentIr>,
    /// Net classes (Task 8 fills field-for-field); class 0 is the default
    /// class, eagerly materialized by `create_board`.
    pub net_classes: Vec<NetClassIr>,
    /// Via infos (net-class via rules, Task 8).
    pub via_infos: Vec<ViaInfoIr>,
    /// Java `BoardRules.viaRules` — identity is [`ViaRuleIr::id`], not a
    /// position (module docs on the struct: `addViaRule` removes and
    /// re-adds, which would shift positions).
    pub via_rules: Vec<ViaRuleIr>,
    /// The id the NEXT appended via rule gets (monotonic from 1).
    next_via_rule_id: u32,
    /// Java `BoardLibrary.logicalParts` (`core/library/LogicalParts.java`):
    /// board logical parts in insertion order; pins sorted by pin index
    /// (`LogicalParts.add`). Filled by the network scope
    /// (`insertLogicalParts`, Task 8).
    pub logical_parts: Vec<LogicalPartIr>,
    /// The board rules (default trace widths, clearance matrix, item
    /// classes); built by the reader and handed over at `create_board`.
    pub rules: BoardRulesIr,
    /// The items in ASCENDING id (parse) order; descending-id consumers
    /// walk `.iter().rev()` (T39).
    pub items: Vec<ItemIr>,
    /// The id the NEXT inserted item gets (starts at 1; mirrors
    /// `ItemIdGenerator.lastGeneratedId + 1`).
    next_item_id: i32,
}

impl Default for SesBoard {
    fn default() -> Self {
        Self::new()
    }
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

/// Java `replaceAll("::\\d+$", "")` (the `Packages.get` retry strip,
/// `Packages.java:40` — anchored at the END, unlike the T32 `.N` strip):
/// removes the final `::<digits>` run if the name ends with one —
/// `PAD::1` -> `PAD`, `PAD::1::2` -> `PAD::1`, `PAD::` and `PAD` stay.
/// At most one candidate suffix can match (a matching suffix spans all
/// digits to the end, so an earlier `::` can never match), hence a
/// single backward scan is Java-exact.
pub fn strip_package_dedup_suffix(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut end = chars.len();
    for i in (0..chars.len().saturating_sub(1)).rev() {
        // `::<digits>` with at least one digit, reaching end-of-name.
        if chars[i] == ':' && chars[i + 1] == ':' && chars.len() > i + 2 {
            if chars[i + 2..].iter().all(|c| c.is_ascii_digit()) {
                end = i;
            }
            // Only the rightmost `::` can match (see doc); stop scanning.
            break;
        }
    }
    chars[..end].iter().collect()
}

impl SesBoard {
    /// An empty board (no `create_board` yet); metadata carries the
    /// parse-state defaults.
    pub fn new() -> Self {
        Self {
            metadata: MetadataIr::default(),
            transform: None,
            layers: None,
            bounding_box: None,
            nets: Vec::new(),
            placements: Vec::new(),
            padstacks: Vec::new(),
            packages: Vec::new(),
            via_padstacks: Vec::new(),
            components: Vec::new(),
            net_classes: Vec::new(),
            via_infos: Vec::new(),
            via_rules: Vec::new(),
            next_via_rule_id: 1,
            logical_parts: Vec::new(),
            rules: BoardRulesIr::new(0),
            items: Vec::new(),
            next_item_id: 1,
        }
    }

    /// The last item id handed out (`ItemIdGenerator.maxGeneratedId()` in
    /// the spike — 8 for the fixture of `/tmp/epic-t4-ids.out`).
    pub fn last_assigned_item_id(&self) -> i32 {
        self.next_item_id - 1
    }

    /// T13 (M3-T13): appends a ROUTER-created item projection after the
    /// parse items. The id is the LIVE BOARD item's id (`Board::
    /// from_ses_board` seeds the generator past the parse ids, so every
    /// routed trace/via carries a higher id — the ascending order of
    /// `items` is preserved when the caller walks the board ascending).
    /// Deliberately NOT the [`BoardSink::insert_trace`] face: that one
    /// assigns a fresh id and applies the parse drop guards, neither of
    /// which belongs to a projection of already-inserted board items.
    pub fn push_routed_item(&mut self, item: ItemIr) {
        let id = item.id();
        if id >= self.next_item_id {
            self.next_item_id = id.saturating_add(1);
        }
        self.items.push(item);
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

    /// T49 — `ObstacleArea.getArea()` (`ObstacleArea.java:119-144`): the
    /// absolute (placement-transformed) form of a stored obstacle area.
    /// Java applies the component placement LAZILY — the item keeps the
    /// image-relative geometry and the flag is read at first consumption —
    /// so the port stores the raw area in [`SesBoard::items`] and applies
    /// this transform in the digest/emission consumers. Transform order:
    /// mirror (back-side, flip style not rotate-first), then the rotation
    /// (exact 90° multiples via `turn90Degree`, everything else via
    /// `rotateApprox`), then the mirror for a rotate-first flip style, then
    /// the translation. The flip-style flag mirrors
    /// `board.components.getFlipStyleRotateFirst()`, which the parse sets
    /// from the structure `(flip_style ...)` scope (`Structure.java:1041`)
    /// and `(place_control (flip_style ...))` (`PlaceControl.java:69`) —
    /// both funneled into [`MetadataIr::flip_style`] = `rotate_first`.
    pub fn obstacle_absolute_area(
        &self,
        area: &AreaIr,
        translation: IntPoint,
        rotation: f64,
        side_changed: bool,
    ) -> AreaIr {
        let flip_style_rotate_first = self.metadata.flip_style.as_deref() == Some("rotate_first");
        let transform_shape = |shape: &BoardShape| -> BoardShape {
            let mut turned = shape.clone();
            if side_changed && !flip_style_rotate_first {
                turned = turned.mirror_vertical(&IntPoint::new(0, 0));
            }
            if rotation != 0.0 {
                if rotation % 90.0 == 0.0 {
                    turned = turned.turn_90_degree((rotation as i32) / 90, &IntPoint::new(0, 0));
                } else {
                    turned =
                        turned.rotate_approx(rotation.to_radians(), &FloatPoint::new(0.0, 0.0));
                }
            }
            if side_changed && flip_style_rotate_first {
                turned = turned.mirror_vertical(&IntPoint::new(0, 0));
            }
            turned.translate_by(&Vector::Int(IntVector::new(translation.x, translation.y)))
        };
        AreaIr {
            border: transform_shape(&area.border),
            holes: area.holes.iter().map(transform_shape).collect(),
        }
    }

    fn alloc_item_id(&mut self) -> i32 {
        let id = self.next_item_id;
        self.next_item_id += 1;
        id
    }

    /// The `NetClass`-ctor initial `activeRoutingLayerArr`: per-layer
    /// `isSignal` (`rules/NetClass.java:34-42`). The `None`-layers
    /// fallback is unreachable through the readers (net classes are only
    /// appended after `create_board`).
    fn fresh_active_routing_layers(&self, layer_count: usize) -> Vec<bool> {
        match &self.layers {
            Some(layers) => layers.layers.iter().map(|layer| layer.is_signal).collect(),
            None => vec![true; layer_count],
        }
    }

    /// Java `DsnFile.adjustPlaneAutorouteSettings` (`DsnFile.java:32-114`),
    /// called from `DsnReader.readBoard` when the DSN has no
    /// `(autoroute_settings ...)` scope (the Task 9 assembly owns that
    /// gate). Promotes interior-layer conduction areas covering >= half
    /// the board to power planes: their nets gain `contains_plane` and
    /// areas below USER_FIXED are promoted to USER_FIXED. Returns whether
    /// anything changed (Java `!nothingChanged`).
    ///
    /// FRLogger INFO per changed layer is log-only — no Rust counterpart.
    /// Divergence (documented): Java sums `PolylineArea.splitToConvex`
    /// pieces (holes CUT OUT per piece); this port subtracts the hole-piece
    /// areas from the border-piece areas, which is exact for the
    /// non-overlapping, border-contained polygon windows every exporter
    /// produces (circle windows never reach a conduction area —
    /// `transform_area_to_board` rejects them). A split failure
    /// (`PolygonShape::split_to_convex` -> `None`, Java null pieces -> NPE
    /// at `DsnFile.java:88`) returns `false` here.
    pub fn adjust_plane_autoroute_settings(&mut self) -> bool {
        let Some(layer_structure) = &self.layers else {
            return false; // Java `routingBoard == null` (`:33-35`)
        };
        let layer_count = layer_structure.layers.len();
        if layer_count <= 2 {
            // `:38-40`: a plane needs interior layers to exist at all
            return false;
        }
        if layer_structure.layers.iter().any(|layer| !layer.is_signal) {
            // `:41-45`: any non-signal layer disables the heuristic
            return false;
        }
        let mut layer_contains_wires = vec![false; layer_count];
        let mut conduction_indices: Vec<usize> = Vec::new();
        for (index, item) in self.items.iter().enumerate() {
            match item {
                ItemIr::Trace { trace, .. } => {
                    let layer = trace.layer_no;
                    // Java `layerContainsWiresArr[currentLayer] = true` —
                    // an out-of-range trace layer would AIOOBE (unreachable:
                    // traces only insert on board layers); guarded skip.
                    if let Ok(slot) = usize::try_from(layer)
                        && slot < layer_count
                    {
                        layer_contains_wires[slot] = true;
                    }
                }
                ItemIr::ConductionArea { .. } => conduction_indices.push(index),
                _ => {}
            }
        }
        let mut nothing_changed = true;

        // board area (`:64-73`): the sum over the outline's convex pieces;
        // a null piece split (PolygonShape that cannot split) is SKIPPED
        // (`:68-72` — the one split-failure Java tolerates).
        let mut board_area = 0.0f64;
        for item in &self.items {
            if let ItemIr::BoardOutline { outline, .. } = item {
                for shape in &outline.shapes {
                    if let Some(pieces) = shape.split_to_convex() {
                        for piece in pieces {
                            board_area += piece.area();
                        }
                    }
                }
            }
        }
        for index in conduction_indices {
            let ItemIr::ConductionArea { area, .. } = &self.items[index] else {
                unreachable!("collected indices point at conduction areas");
            };
            let layer_index = area.layer_no;
            // Java `layerContainsWiresArr[layerIndex]` would AIOOBE out of
            // range (unreachable: conduction areas insert on board layers);
            // guarded skip.
            let Ok(layer_slot) = usize::try_from(layer_index) else {
                continue;
            };
            if layer_slot >= layer_count || layer_contains_wires[layer_slot] {
                // `:76-78`: wires on the plane layer veto the promotion
                continue;
            }
            let layer_is_signal =
                self.layers.as_ref().expect("checked above").layers[layer_slot].is_signal;
            if !layer_is_signal || layer_slot == 0 || layer_slot == layer_count - 1 {
                // `:81-85`: only INTERIOR signal layers promote
                continue;
            }
            let Some(current_area) = convex_area(&area.area) else {
                // Java null convex pieces -> NPE at `:88` (parse dies);
                // the port degrades to "no change".
                return false;
            };
            if current_area < 0.5 * board_area {
                // `:91-93`: the >= half-board-area gate
                continue;
            }
            for net_no in &area.nets {
                // Java `nets.get(netNumber)` NPEs on an unknown number
                // (unreachable: plane nets resolve before insertion);
                // guarded skip.
                if let Some(net) = usize::try_from(*net_no)
                    .ok()
                    .and_then(|slot| self.nets.get_mut(slot.checked_sub(1)?))
                {
                    net.contains_plane = true;
                    nothing_changed = false;
                }
            }
            let ItemIr::ConductionArea { area, .. } = &mut self.items[index] else {
                unreachable!("collected indices point at conduction areas");
            };
            if area.fixed < FixedStateIr::UserFixed {
                // `:100-102`: UNFIXED/SHOVE_FIXED -> USER_FIXED (the
                // `ordinal() <` comparison is the FixedStateIr order)
                area.fixed = FixedStateIr::UserFixed;
            }
            // `changedLayerArr` feeds the FRLogger INFO loop (`:104-112`)
            // — log-only, not tracked here.
        }
        !nothing_changed
    }
}

/// The convex-piece area of an [`AreaIr`] for the plane heuristic: the
/// border's `splitToConvex` piece areas, minus the holes' piece areas when
/// holes exist (module docs on
/// [`SesBoard::adjust_plane_autoroute_settings`] for the divergence).
/// `None` mirrors a failed Java `splitToConvex`.
fn convex_area(area: &crate::sink::AreaIr) -> Option<f64> {
    let mut total = 0.0f64;
    for piece in area.border.split_to_convex()? {
        total += piece.area();
    }
    for hole in &area.holes {
        for piece in hole.split_to_convex()? {
            total -= piece.area();
        }
    }
    Some(total)
}

impl BoardSink for SesBoard {
    fn has_board(&self) -> bool {
        // `getRoutingBoard() == null` mirror: `transform` is only ever set
        // by `create_board`, so it is the board-exists flag (module docs).
        self.transform.is_some()
    }

    fn create_board(&mut self, board: CreateBoardIr) {
        // Java's structure reader only builds the board when none exists
        // (`if (getRoutingBoard() == null)`, `Structure.java:1034-1036`):
        // a second `(structure ...)` scope re-parses but does NOT
        // recreate — jar `/tmp/epic-t4c-twostruct.out` (two structure
        // scopes: one outline, items dense 1..5, second boundary
        // ignored).
        if self.transform.is_some() {
            return;
        }
        self.transform = Some(board.transform);
        self.layers = Some(board.layer_structure);
        self.bounding_box = Some(board.bounding_box);
        self.rules = board.rules;
        // The default net class, eagerly materialized (Java's
        // `BoardRules.getDefaultNetClass` creates it lazily on first
        // touch, `BoardRules.java:136-143`; every consumer — structure
        // keepouts through the rules mirror, network/wiring scope readers
        // — observes the same end state because nothing between
        // createBoard and the first touch can differ). Field-by-field:
        // `createDefaultNetClass` (`:202-209`) sets hw 1500 all layers +
        // traceClearanceClass 1, but the structure width rules already
        // WROTE the same class's hw array (setDefaultTraceHalfWidth =
        // getDefaultNetClass().setTraceHalfWidth + min/max, `:122-127`),
        // so the final hw is the rules mirror verbatim; the item classes
        // are the same array the rule retargets wrote
        // (rules.default_item_clearance_classes). activeRoutingLayerArr
        // starts per-layer isSignal (NetClass ctor, `rules/NetClass.java:
        // 34-42`), viaRule stays null (createDefaultNetClass sets none),
        // pullTight true, shoveFixed false, min/max 0.
        let layer_structure = self.layers.as_ref().expect("just set above");
        self.net_classes.push(NetClassIr {
            name: "default".to_string(),
            trace_clearance_class: 1,
            trace_half_widths: self.rules.default_trace_half_widths.clone(),
            active_routing_layers: layer_structure
                .layers
                .iter()
                .map(|layer| layer.is_signal)
                .collect(),
            default_item_clearance_classes: self.rules.default_item_clearance_classes,
            via_rule: None,
            pull_tight: true,
            shove_fixed: false,
            min_trace_length: 0.0,
            max_trace_length: 0.0,
            nets: Vec::new(),
        });
        // MinimalBoardManager.createBoard (ReadScopeParameter.java:139-166):
        // `outlineClearanceNo = 0`; with a class NAME present the number is
        // `max(0, clearanceMatrix.getNo(name))` — a MISS resolves to 0, the
        // `null` class (jar /tmp/epic-t5-probe.out OUTLINE_CLASS 0 for a
        // nonexistent name); without a name it is the AREA item class of
        // the default net class (jar: 1 rule-less, 7 after the `wire` rule
        // appended `area`).
        let outline_class = match board.outline_clearance_class.as_deref() {
            Some(name) => self.rules.clearance.get_no(name).max(0),
            None => {
                self.rules.default_item_clearance_classes[crate::sink::ItemClassIr::Area as usize]
            }
        };
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
        // Java CleararanceMatrix.append_class: case-insensitive dedup, the
        // new row/column copies class 1 ("default") per layer — the
        // full port lives on the IR (sink::ClearanceIr::append_class).
        self.rules.clearance.append_class(name);
    }

    fn clearance_class_no(&self, name: &str) -> Option<i32> {
        let no = self.rules.clearance.get_no(name);
        if no < 0 { None } else { Some(no) }
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

    fn insert_package(&mut self, image: ImageIr) -> i32 {
        self.packages.push(image);
        self.packages.len() as i32
    }

    fn package_no(&self, name: &str, is_front: bool) -> Option<i32> {
        // Packages.get(name, isFront) (Packages.java:27-52), verbatim:
        // pass 1 = first case-insensitive same-side match; wrong-side
        // matches are remembered (LAST one wins, `otherSidePackage`);
        // pass 2 = the same against the name with its trailing ::N
        // suffix stripped (:40); final fallback = the remembered
        // other-side package (:51).
        let mut other_side: Option<usize> = None;
        for (index, package) in self.packages.iter().enumerate() {
            if eq_ignore_case(&package.name, name) {
                if package.is_front == is_front {
                    return Some(index as i32 + 1);
                }
                other_side = Some(index);
            }
        }
        let base = strip_package_dedup_suffix(name);
        if !eq_ignore_case(&base, name) {
            for (index, package) in self.packages.iter().enumerate() {
                if eq_ignore_case(&package.name, &base) {
                    if package.is_front == is_front {
                        return Some(index as i32 + 1);
                    }
                    other_side = Some(index);
                }
            }
        }
        other_side.map(|index| index as i32 + 1)
    }

    fn package_name(&self, package_no: i32) -> Option<&str> {
        self.packages
            .get(package_no.checked_sub(1)? as usize)
            .map(|package| package.name.as_str())
    }

    fn package_pins(&self, package_no: i32) -> &[ImagePinIr] {
        if package_no < 1 {
            return &[];
        }
        match self.packages.get((package_no - 1) as usize) {
            Some(package) => package.pins.as_slice(),
            None => &[],
        }
    }

    fn package(&self, package_no: i32) -> Option<&ImageIr> {
        let index = usize::try_from(package_no.checked_sub(1)?).ok()?;
        self.packages.get(index)
    }

    fn component_package_no(&self, name: &str) -> Option<i32> {
        let component = self
            .components
            .iter()
            .find(|component| component.name == name)?;
        Some(if component.is_front {
            component.package_front
        } else {
            component.package_back
        })
    }

    fn net_classes(&self) -> &[NetClassIr] {
        &self.net_classes
    }

    fn net_classes_mut(&mut self) -> &mut [NetClassIr] {
        &mut self.net_classes
    }

    fn board_rules_mut(&mut self) -> &mut BoardRulesIr {
        &mut self.rules
    }

    fn append_net_class(&mut self, name: &str) -> i32 {
        // BoardRules.appendNetClass(String) (BoardRules.java:225-239):
        // case-SENSITIVE get hit returns the stored class; otherwise
        // append one that CLONES the default's item classes and copies
        // viaRule + traceClearanceClass, filling EVERY hw layer with the
        // default's hw[0] (the all-layers setTraceHalfWidth(int) fill).
        if let Some(index) = self.net_classes.iter().position(|c| c.name == name) {
            return index as i32;
        }
        let default = &self.net_classes[0];
        let hw0 = default.trace_half_widths.first().copied().unwrap_or(0);
        let layer_count = default.trace_half_widths.len();
        let new_class = NetClassIr {
            name: name.to_string(),
            trace_clearance_class: default.trace_clearance_class,
            trace_half_widths: vec![hw0; layer_count],
            active_routing_layers: self.fresh_active_routing_layers(layer_count),
            default_item_clearance_classes: default.default_item_clearance_classes,
            via_rule: default.via_rule,
            pull_tight: true,
            shove_fixed: false,
            min_trace_length: 0.0,
            max_trace_length: 0.0,
            nets: Vec::new(),
        };
        self.net_classes.push(new_class);
        (self.net_classes.len() - 1) as i32
    }

    fn append_generated_net_class(&mut self) -> i32 {
        // BoardRules.getNewNetClass() (BoardRules.java:145-151) over
        // NetClasses.append(layerStructure, clearanceMatrix)
        // (NetClasses.java:55-64): the generated name loop `do { ++index;
        // "class" + index } while get != null` (case-SENSITIVE), FRESH
        // item classes (NOT cloned), default traceClearanceClass, default
        // via rule, and EVERY hw layer = default hw[0].
        let mut index: u32 = 0;
        let name = loop {
            index += 1;
            let candidate = format!("class{}", index);
            if !self.net_classes.iter().any(|c| c.name == candidate) {
                break candidate;
            }
        };
        let default = &self.net_classes[0];
        let hw0 = default.trace_half_widths.first().copied().unwrap_or(0);
        let layer_count = default.trace_half_widths.len();
        let new_class = NetClassIr {
            name,
            trace_clearance_class: default.trace_clearance_class,
            trace_half_widths: vec![hw0; layer_count],
            active_routing_layers: self.fresh_active_routing_layers(layer_count),
            // Fresh DefaultItemClearanceClasses() = new int[6] + setAll(1)
            // with setAll starting at i=1 -> [0,1,1,1,1,1] (the never-read
            // NONE slot stays 0).
            default_item_clearance_classes: [0, 1, 1, 1, 1, 1],
            via_rule: self.default_via_rule_id(),
            pull_tight: true,
            shove_fixed: false,
            min_trace_length: 0.0,
            max_trace_length: 0.0,
            nets: Vec::new(),
        };
        self.net_classes.push(new_class);
        (self.net_classes.len() - 1) as i32
    }

    fn via_infos(&self) -> &[ViaInfoIr] {
        &self.via_infos
    }

    fn via_info_no(&self, name: &str) -> Option<i32> {
        // ViaInfos.get(String) (ViaInfos.java:43-50): first CASE-SENSITIVE
        // name match, 0-based index.
        self.via_infos
            .iter()
            .position(|info| info.name == name)
            .map(|index| index as i32)
    }

    fn append_via_info(&mut self, via_info: ViaInfoIr) {
        // ViaInfos.add (ViaInfos.java:28-38): silently deduped by
        // case-sensitive name (the boolean return is discarded at the
        // call sites).
        if self.via_info_no(&via_info.name).is_some() {
            return;
        }
        self.via_infos.push(via_info);
    }

    fn via_rules(&self) -> &[ViaRuleIr] {
        &self.via_rules
    }

    fn append_via_rule(&mut self, name: String, via_infos: Vec<i32>) -> u32 {
        // viaRules.add(new ViaRule(name)): append under a fresh identity
        // (ViaRuleIr::id is monotonic; positions are NOT identity). The
        // members arrive resolved (Task 8 readers pass them filled).
        let id = self.next_via_rule_id;
        self.next_via_rule_id += 1;
        self.via_rules.push(ViaRuleIr {
            id,
            name,
            via_infos,
        });
        id
    }

    fn via_rule_no(&self, name: &str) -> Option<u32> {
        // ViaRules.get(String): first CASE-SENSITIVE name match.
        self.via_rules
            .iter()
            .find(|rule| rule.name == name)
            .map(|rule| rule.id)
    }

    fn remove_via_rule(&mut self, id: u32) {
        // viaRules.remove(existing): Vector identity removal.
        self.via_rules.retain(|rule| rule.id != id);
    }

    fn default_via_rule_id(&self) -> Option<u32> {
        // BoardRules.getDefaultViaRule() (BoardRules.java:243-249): first
        // rule or null.
        self.via_rules.first().map(|rule| rule.id)
    }

    fn padstack(&self, padstack_no: i32) -> Option<&PadstackIr> {
        // Padstacks.get(int) (core/library/Padstacks.java:15-22): 1-based;
        // out of range warns + null (the FRLogger warn is log-only here).
        let index = usize::try_from(padstack_no.checked_sub(1)?).ok()?;
        self.padstacks.get(index)
    }

    fn nets(&self) -> &[NetIr] {
        &self.nets
    }

    fn nets_mut(&mut self) -> &mut Vec<NetIr> {
        &mut self.nets
    }

    fn set_via_padstacks(&mut self, padstack_nos: Vec<i32>) {
        self.via_padstacks = padstack_nos;
    }

    fn via_padstack_no(&self, name: &str) -> Option<i32> {
        // BoardLibrary.getViaPadstack(String) (BoardLibrary.java:53-62):
        // first list entry whose REGISTRY padstack name equals the query
        // with String.equals — CASE-SENSITIVE, unlike every registry
        // query. Returns the matched entry's registry number; the 0-based
        // list index (Java getViaPadstack(int), :45-50) is the position.
        for padstack_no in &self.via_padstacks {
            if self.padstack_name(*padstack_no) == Some(name) {
                return Some(*padstack_no);
            }
        }
        None
    }

    fn append_via_padstack(&mut self, padstack_no: i32) {
        // BoardLibrary.addViaPadstack (BoardLibrary.java:90-101, driven
        // by the via-info reader Network.java:283): no-op when the list
        // already holds an entry whose name equals the candidate's
        // (case-SENSITIVE String.equals, :91); append otherwise. An
        // unresolvable number cannot occur for a correct reader (Java
        // holds the Padstack object) and is ignored here.
        let Some(candidate_name) = self.padstack_name(padstack_no) else {
            return;
        };
        if self.via_padstack_no(candidate_name).is_some() {
            return;
        }
        self.via_padstacks.push(padstack_no);
    }

    fn via_padstacks(&self) -> &[i32] {
        &self.via_padstacks
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

    fn append_logical_part(&mut self, mut part: LogicalPartIr) {
        // LogicalParts.add sorts the pin array by pin index BEFORE
        // construction (Arrays.sort — stable merge sort).
        part.pins.sort_by_key(|pin| pin.pin_index);
        self.logical_parts.push(part);
    }

    fn logical_part_name(&self, name: &str) -> Option<String> {
        // LogicalParts.get(String): first CASE-INSENSITIVE name match.
        self.logical_parts
            .iter()
            .find(|part| eq_ignore_case(&part.name, name))
            .map(|part| part.name.clone())
    }

    fn set_component_logical_part(&mut self, component_name: &str, logical_part: Option<String>) {
        // Components.get(String): first CASE-SENSITIVE name match
        // (Components.java); a miss is the caller's log-only warning.
        if let Some(component) = self
            .components
            .iter_mut()
            .find(|component| component.name == component_name)
        {
            component.logical_part = logical_part;
        }
    }

    fn insert_pin(&mut self, pin: PinIr) -> i32 {
        let id = self.alloc_item_id();
        self.items.push(ItemIr::Pin { id, pin });
        id
    }

    fn insert_component_outline(&mut self, outline: ComponentOutlineIr) -> i32 {
        // BasicBoard.insertComponentOutline null guard: a null area (the
        // PolylinePath-stub outline slot) inserts NOTHING and burns NO id
        // (the ItemIdGenerator is only consulted on the insert path);
        // 0 is the "not inserted" sentinel — real ids start at 1.
        let Some(area) = outline.area else {
            return 0;
        };
        let id = self.alloc_item_id();
        self.items.push(ItemIr::ComponentOutline {
            id,
            outline: ComponentOutlineIr {
                area: Some(area),
                ..outline
            },
        });
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
        // BasicBoard.insertTraceWithoutCleaning (`BasicBoard.java:183-201`)
        // has TWO drop guards with DIFFERENT id semantics: (1) a
        // sub-2-corner polyline returns null BEFORE the PolylineTrace is
        // constructed — no id burned; (2) first == last with a fixed state
        // below USER_FIXED returns null AFTER construction — the `Item`
        // constructor already allocated the id (`Item.java:86-89`), so the
        // id is burned with no stored item (jar /tmp/t9rev-two.out CASE
        // closed: the closed UNFIXED wire consumes id 2 and is not stored;
        // the open wire lands at id 3, GEN_MAX 3).
        if trace.corners.len() < 2 {
            return 0;
        }
        let id = self.alloc_item_id();
        let last_corner = trace.corners[trace.corners.len() - 1];
        if trace.corners[0] == last_corner && trace.fixed < FixedStateIr::UserFixed {
            return 0;
        }
        self.items.push(ItemIr::Trace { id, trace });
        id
    }

    fn insert_via(&mut self, via: ViaIr) -> i32 {
        let id = self.alloc_item_id();
        self.items.push(ItemIr::Via { id, via });
        id
    }

    fn via_exists(&self, location: IntPoint, from_layer: i32, to_layer: i32, nets: &[i32]) -> bool {
        // Java `Wiring.viaExists` (`:238-255`): pick the vias at the
        // location, then compare net sets (`Item.netsEqual(int[])`,
        // `Item.java:1234-1244` — equal LENGTH plus every QUERY net
        // contained, set semantics), the exact center, and the first/last
        // layer span, which Java derives from the via's OWN padstack — so
        // the sink derives both spans from the stored via's registry
        // padstack ([`crate::scope::network::padstack_from_layer`] /
        // [`crate::scope::network::padstack_to_layer`]).
        for item in &self.items {
            let ItemIr::Via { via, .. } = item else {
                continue;
            };
            if via.location != location || !nets_equal(&via.nets, nets) {
                continue;
            }
            let Some(padstack) = self.padstack(via.padstack_no) else {
                continue;
            };
            let first = crate::scope::network::padstack_from_layer(&padstack.shapes) as i32;
            let last = crate::scope::network::padstack_to_layer(&padstack.shapes);
            if first == from_layer && last == to_layer {
                return true;
            }
        }
        false
    }

    fn net_no_subnet(&self, name: &str, subnet_number: i32) -> Option<i32> {
        // Nets.get(name, subnetNumber) (rules/Nets.java:42-51): the FIRST
        // table entry whose name matches case-INSENSITIVELY and whose
        // subnet number equals exactly. Discriminator pin
        // /tmp/epic-t6-board.out t6-plane.dsn: after appending board nets
        // GND(1) and gnd(2), get("gnd", 1) still resolves to net 1, so
        // both B.Cu conduction areas carry nets=[1].
        self.nets
            .iter()
            .position(|net| eq_ignore_case(&net.name, name) && net.subnet_number == subnet_number)
            .map(|index| index as i32 + 1)
    }

    fn net_nos(&self, name: &str) -> Vec<i32> {
        // Nets.get(String) (rules/Nets.java:54-62): EVERY case-insensitive
        // name match, in table order — insertNetClass's net-list pass
        // setClasses all of them (T35: `GND` reclasses `gnd` too).
        self.nets
            .iter()
            .enumerate()
            .filter(|(_, net)| eq_ignore_case(&net.name, name))
            .map(|(index, _)| index as i32 + 1)
            .collect()
    }

    fn layer_count(&self) -> i32 {
        // board.getLayerCount() — 0 before create_board.
        self.layers
            .as_ref()
            .map_or(0, |layers| layers.layers.len() as i32)
    }

    fn default_item_clearance_class(&self, item_class: ItemClassIr) -> i32 {
        // rules.getDefaultNetClass().defaultItemClearanceClasses.get(kind)
        // — moves when a rule scope appends a class of the same name
        // (append_clearance_class retargets via/pin/smd/area).
        self.rules.default_item_clearance_classes[item_class as usize]
    }

    fn has_conduction_area_on_layer(&self, layer_no: i32) -> bool {
        // the insertMissingPowerPlanes scan (Structure.java:532-539)
        self.items.iter().any(
            |item| matches!(item, ItemIr::ConductionArea { area, .. } if area.layer_no == layer_no),
        )
    }

    fn board_bounding_box(&self) -> Option<IntBox> {
        self.bounding_box
    }

    fn board_layer_structure(&self) -> Option<&LayerStructure> {
        self.layers.as_ref()
    }

    fn set_metadata(&mut self, mut metadata: MetadataIr) {
        // The snapshot cannot carry the board-held flip_style: Java sets
        // `board.components.setFlipStyleRotateFirst(true)` from the
        // STRUCTURE scope (Task 6, `Structure.java:1041-1043`), before the
        // read_board assembly snapshots the parser fields
        // (`DsnReader.java:138-147`). MERGE instead of replace: keep the
        // existing flip_style when the incoming snapshot carries None.
        let flip_style = metadata
            .flip_style
            .take()
            .or(self.metadata.flip_style.take());
        self.metadata = metadata;
        self.metadata.flip_style = flip_style;
    }

    fn adjust_plane_autoroute_settings(&mut self) {
        // Java `DsnFile.adjustPlaneAutorouteSettings` (`:33-113`); the
        // logic lives on the inherent method (Task 6, tested there) — the
        // trait seam only reaches it through `dyn BoardSink`. The return
        // value feeds an FRLogger INFO in Java only. (The `SesBoard::`
        // path form resolves to the INHERENT method; plain method-call
        // syntax would too, but the qualified form states it.)
        let _changed = SesBoard::adjust_plane_autoroute_settings(self);
    }
}

/// Java `Item.netsEqual(int[])` (`Item.java:1234-1244`): equal LENGTH plus
/// every QUERY net contained in the item's net list (set semantics,
/// order-insensitive).
fn nets_equal(stored: &[i32], query: &[i32]) -> bool {
    stored.len() == query.len() && query.iter().all(|net| stored.contains(net))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shape::BoardShape;
    use crate::sink::{AreaIr, ClearanceIr, ImagePinIr, KeepoutKindIr};
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
            outline_clearance_class: Some("default".to_string()),
            rules: BoardRulesIr::new(2),
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
                contains_plane: false,
                net_class: 0,
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
            component_id: 0,
            translation: IntPoint::new(0, 0),
            rotation: 0.0,
            side_changed: false,
            name: None,
        });

        // network scope: component + pin insertions in parse order
        let u1 = board.insert_component(ComponentIr {
            name: "U1".to_string(),
            package_front: 1,
            package_back: 1,
            location: Some(IntPoint::new(20_000, 50_000)),
            rotation: 0.0,
            is_front: true,
            fixed: FixedStateIr::Unfixed,
            part_number: None,
            logical_part: None,
        });
        let u2 = board.insert_component(ComponentIr {
            name: "U2".to_string(),
            package_front: 1,
            package_back: 1,
            location: Some(IntPoint::new(80_000, 50_000)),
            rotation: 0.0,
            is_front: true,
            fixed: FixedStateIr::Unfixed,
            part_number: None,
            logical_part: None,
        });
        board.insert_pin(PinIr {
            component_id: u1,
            pin_index: 0,
            padstack_no: 2,
            nets: vec![1],
            clearance_class: 1,
            fixed: FixedStateIr::Unfixed,
        });
        board.insert_pin(PinIr {
            component_id: u2,
            pin_index: 0,
            padstack_no: 2,
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
            polyline: TraceIr::polyline_of_corners(&[
                IntPoint::new(20_000, 50_000),
                IntPoint::new(80_000, 50_000),
            ]),
            nets: vec![1],
            clearance_class: 1,
            fixed: FixedStateIr::UserFixed,
        });
        board.insert_trace(TraceIr {
            layer_no: 0,
            half_width: 625,
            corners: vec![IntPoint::new(20_000, 51_000), IntPoint::new(80_000, 51_000)],
            polyline: TraceIr::polyline_of_corners(&[
                IntPoint::new(20_000, 51_000),
                IntPoint::new(80_000, 51_000),
            ]),
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
            board.rules.clearance,
            ClearanceIr::default_instance(2),
            "default instance is [null, default] with a zero matrix per layer"
        );
        assert_eq!(
            board.rules.default_trace_half_widths,
            vec![1500, 1500],
            "lazy default net class materialized with widths 1500"
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
                contains_plane: false,
                net_class: 0,
            }),
            1
        );
        assert_eq!(
            board.append_net(NetIr {
                name: "gnd".to_string(),
                subnet_number: 1,
                contains_plane: false,
                net_class: 0,
            }),
            2
        );
        assert_eq!(
            board.append_net(NetIr {
                name: "VDD".to_string(),
                subnet_number: 2,
                contains_plane: false,
                net_class: 0,
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

    /// Clearance class append: unknown names append (the new row/column
    /// copies class 1, `ClearanceMatrix.appendClass`), known names
    /// (case-insensitive, like `get_no`) are no-ops.
    #[test]
    fn clearance_class_append_dedups() {
        let mut board = spike_board();
        board.append_clearance_class("power");
        assert_eq!(board.clearance_class_no("POWER"), Some(2));
        assert_eq!(
            board.rules.clearance.names,
            vec!["null", "default", "power"]
        );
        assert_eq!(board.rules.clearance.values.len(), 2, "per layer");
        assert!(
            board
                .rules
                .clearance
                .values
                .iter()
                .all(|per_layer| per_layer.len() == 3)
        );
        // class-1 copy: the appended entries read the "default" values (0)
        assert_eq!(board.rules.clearance.get_value(1, 2, 0), 0);
        assert_eq!(board.rules.clearance.get_value(2, 2, 0), 0);
        board.append_clearance_class("default");
        assert_eq!(
            board.rules.clearance.names.len(),
            3,
            "dedup: no second default"
        );
        // Case variant: the dedup match rule is the same case-insensitive
        // rule as get_no (`ClearanceMatrix.getNo`, ClearanceMatrix.java:
        // 58-65, via append_class) — "DEFAULT" is a no-op too.
        board.append_clearance_class("DEFAULT");
        assert_eq!(
            board.clearance_class_no("DEFAULT"),
            Some(1),
            "case variant resolves to the existing class"
        );
        assert_eq!(
            board.rules.clearance.names.len(),
            3,
            "case-variant append is a no-op"
        );
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
                coor: Some([2000.0, 5000.0]),
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
                package_front: 1,
                package_back: 1,
                location: Some(IntPoint::new(20_000, 50_000)),
                rotation: 0.0,
                is_front: true,
                fixed: FixedStateIr::Unfixed,
                part_number: None,
                logical_part: None,
            }),
            1
        );
        assert_eq!(
            board.insert_component(ComponentIr {
                name: "U2".to_string(),
                package_front: 1,
                package_back: 1,
                location: Some(IntPoint::new(20_000, 50_000)),
                rotation: 0.0,
                is_front: true,
                fixed: FixedStateIr::Unfixed,
                part_number: None,
                logical_part: None,
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

    /// The `Packages.get` retry strip (Java `replaceAll("::\\d+$", "")`,
    /// `Packages.java:40` — anchored at the end, unlike the T32 strip).
    #[test]
    fn strip_package_dedup_suffix_cases() {
        assert_eq!(strip_package_dedup_suffix("PAD::1"), "PAD");
        assert_eq!(strip_package_dedup_suffix("PAD::1::2"), "PAD::1");
        assert_eq!(strip_package_dedup_suffix("PAD::"), "PAD::");
        assert_eq!(strip_package_dedup_suffix("PAD"), "PAD");
        assert_eq!(strip_package_dedup_suffix("A::1:B"), "A::1:B");
        assert_eq!(strip_package_dedup_suffix("PAD:::5"), "PAD:");
    }

    /// Java `arePackagePinsIdentical` (`Library.java:226-259`): pin
    /// count, then per pin name `String.equals` (case-SENSITIVE),
    /// padstack id, exact relative location and rotation within 0.001.
    fn package_pins_identical(existing: &ImageIr, pins: &[ImagePinIr]) -> bool {
        if existing.pins.len() != pins.len() {
            return false;
        }
        existing.pins.iter().zip(pins).all(|(a, b)| {
            a.name == b.name
                && a.padstack_no == b.padstack_no
                && a.rel_location == b.rel_location
                && (a.rotation - b.rotation).abs() <= 0.001
        })
    }

    /// The library-scope image reader flow (`Library.java:409-449`):
    /// retry the base name as `NAME::1`, `NAME::2`, ... and insert under
    /// the first name whose `Packages.get` hit does not already carry
    /// that exact stored name (`:417` — the `::N`-strip inside `get` is
    /// what terminates the loop), skipping entirely on identical pins.
    fn insert_image_scope(
        board: &mut SesBoard,
        scope_name: &str,
        is_front: bool,
        pins: Vec<ImagePinIr>,
    ) -> i32 {
        let base = strip_package_dedup_suffix(scope_name);
        let mut suffix = 0;
        loop {
            let test_name = if suffix == 0 {
                base.clone()
            } else {
                format!("{base}::{suffix}")
            };
            let (insert_here, identical_hit) = match board.package_no(&test_name, is_front) {
                None => (true, false),
                Some(existing_no) => {
                    let existing = &board.packages[(existing_no - 1) as usize];
                    if !eq_ignore_case(&existing.name, &test_name) {
                        (true, false)
                    } else {
                        (false, package_pins_identical(existing, &pins))
                    }
                }
            };
            if insert_here {
                break board.insert_package(ImageIr {
                    name: test_name,
                    pins,
                    outline: Vec::new(),
                    keepouts: Vec::new(),
                    via_keepouts: Vec::new(),
                    place_keepouts: Vec::new(),
                    is_front,
                });
            }
            if identical_hit {
                // Java returns the existing package (dedup skip).
                return board
                    .package_no(&test_name, is_front)
                    .expect("hit vanished between the two queries");
            }
            suffix += 1;
        }
    }

    /// Package table reader flow, pinned to the jar probe
    /// `/tmp/epic-t4c-images.jsh`/`.out` (fixture
    /// `/tmp/epic-t4c-images.dsn`): five `(image ...)` scopes — PAD with
    /// 1 pin, PAD duplicated identically, PAD with 2 pins, case-variant
    /// `pad` with the base's pins, and a back-side `(side back)` PAD with
    /// 2 shifted pins. Jar: `PACKAGE_COUNT 3` with rows `PAD`(front),
    /// `PAD::1`(front), `PAD::1`(BACK) — the identical dup deduped, the
    /// 2-pin dup inserted as `PAD::1`, the case variant deduped into the
    /// base, and the back-side dup landing under `PAD::1` TOO because
    /// `Packages.get`'s other-side fallback returns the front base whose
    /// stored name differs from the queried `PAD::1` (`:417` insert
    /// fires). The `package_no` pins then discriminate every `get` arm:
    /// exact same-side, exact other-side, `::N`-strip retry, other-side
    /// fallback and total miss.
    #[test]
    fn package_table_reader_flow_jar_probe() {
        let pin = |name: &str, x: i32, y: i32| ImagePinIr {
            name: name.to_string(),
            padstack_no: 2,
            rel_location: IntPoint::new(x, y),
            rotation: 0.0,
        };
        let mut board = SesBoard::new();

        // image PAD, 1 pin at (0,0) — appended as id 1.
        assert_eq!(
            insert_image_scope(&mut board, "PAD", true, vec![pin("1", 0, 0)]),
            1
        );
        // duplicate identical — deduped, still 1 package (jar PACKAGE 1).
        assert_eq!(
            insert_image_scope(&mut board, "PAD", true, vec![pin("1", 0, 0)]),
            1
        );
        // PAD with a second pin — different pins, `PAD::1` appended (id 2).
        assert_eq!(
            insert_image_scope(
                &mut board,
                "PAD",
                true,
                vec![pin("1", 0, 0), pin("2", 1000, 0)]
            ),
            2
        );
        // case-variant `pad` with the base pins — deduped into id 1.
        assert_eq!(
            insert_image_scope(&mut board, "pad", true, vec![pin("1", 0, 0)]),
            1
        );
        // back-side PAD, 2 shifted pins — jar PACKAGE 3: `PAD::1` BACK
        // (the other-side fallback poisons the name search: at suffix 1,
        // get("PAD::1", back) returns the front base PAD whose name
        // differs from the queried "PAD::1", so the insert fires under
        // the suffixed name).
        assert_eq!(
            insert_image_scope(
                &mut board,
                "PAD",
                false,
                vec![pin("1", 0, 500), pin("2", 1000, 500)]
            ),
            3
        );

        let names: Vec<(&str, bool, usize)> = board
            .packages
            .iter()
            .map(|image| (image.name.as_str(), image.is_front, image.pins.len()))
            .collect();
        assert_eq!(
            names,
            vec![("PAD", true, 1), ("PAD::1", true, 2), ("PAD::1", false, 2)],
            "jar PACKAGE lines 1-3"
        );

        // Packages.get arms (`Packages.java:27-52`):
        assert_eq!(board.package_no("PAD", true), Some(1), "exact same side");
        assert_eq!(
            board.package_no("pad", true),
            Some(1),
            "case-insensitive name"
        );
        assert_eq!(
            board.package_no("PAD::1", true),
            Some(2),
            "exact hit on the suffixed name, front"
        );
        assert_eq!(
            board.package_no("PAD::1", false),
            Some(3),
            "exact hit on the suffixed name, back"
        );
        assert_eq!(
            board.package_no("PAD::2", true),
            Some(1),
            "::N-strip retry falls into the base (the loop terminator)"
        );
        assert_eq!(
            board.package_no("PAD", false),
            Some(1),
            "other-side fallback (wrong-side match remembered in pass 1)"
        );
        assert_eq!(board.package_no("NOPE", true), None, "total miss");
    }

    /// A second `create_board` is a NO-OP: Java's structure reader only
    /// builds the board when none exists (`if (getRoutingBoard() ==
    /// null)`, `Structure.java:1034-1036`); jar probe
    /// `/tmp/epic-t4c-twostruct.jsh`/`.out` (two `(structure ...)` scopes,
    /// second with a different boundary): ONE outline, items dense 1..5,
    /// `GEN_MAX 5` — the second boundary never became a second board.
    /// Accumulating stale state on re-invocation would duplicate the
    /// outline and reset the clearance table.
    #[test]
    fn create_board_twice_is_noop_jar_probe() {
        let mut board = SesBoard::new();
        let first = CreateBoardIr {
            bounding_box: IntBox::new(IntPoint::new(0, 0), IntPoint::new(100_000, 100_000)),
            layer_structure: spike_layers(),
            outline_shapes: Vec::new(),
            outline_clearance_class: Some("default".to_string()),
            rules: BoardRulesIr::new(2),
            transform: crate::coordinate_transform::CoordinateTransform::new(10.0, 0.0, 0.0),
        };
        board.create_board(first.clone());
        board.create_board(CreateBoardIr {
            bounding_box: IntBox::new(IntPoint::new(0, 0), IntPoint::new(200_000, 100_000)),
            layer_structure: spike_layers(),
            outline_shapes: Vec::new(),
            outline_clearance_class: Some("default".to_string()),
            rules: BoardRulesIr::new(2),
            transform: crate::coordinate_transform::CoordinateTransform::new(10.0, 0.0, 0.0),
        });
        assert_eq!(
            board.transform,
            Some(first.transform),
            "the first transform survived"
        );
        assert_eq!(
            board.items.len(),
            1,
            "one BoardOutline, not one per create_board"
        );
        assert_eq!(board.last_assigned_item_id(), 1);
        assert_eq!(board.rules.clearance, ClearanceIr::default_instance(2));
    }

    /// The via-padstack list is a REPLACED-then-appended second index
    /// over the registry; lookups over it are case-SENSITIVE
    /// (`BoardLibrary.getViaPadstack(String)`, `:53-62`) while the
    /// registry query is not — the discriminating pair. Replace
    /// semantics pin the network tail (`Network.java:1313`), the empty
    /// replace pins the jar wipe (`/tmp/epic-t4c-images.out`:
    /// VIAPADSTACK_COUNT 0 — class-scope vias written as bare `(via ...)`
    /// never reach `useVia`, so the tail replaced with an empty list).
    #[test]
    fn via_padstack_list_replace_append_and_case_sensitive_lookup() {
        let mut board = SesBoard::new();
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

        board.set_via_padstacks(vec![2, 1]);
        assert_eq!(board.via_padstacks, vec![2, 1], "order is significant");
        assert_eq!(
            board.via_padstack_no("CirclePad_F_800_um"),
            Some(2),
            "exact-name hit at list position 0"
        );
        assert_eq!(
            board.via_padstack_no("circlepad_f_800_um"),
            None,
            "case-SENSITIVE: viaPadstack_no misses where the registry hits"
        );
        assert_eq!(
            board.padstack_no("circlepad_f_800_um"),
            Some(2),
            "registry query is case-insensitive"
        );
        assert_eq!(board.via_padstack_no("ViaPad_F"), Some(1));
        assert_eq!(board.via_padstack_no("Nope"), None);

        // addViaPadstack (BoardLibrary.java:90-101): no-op on an existing
        // case-sensitive name, append otherwise.
        board.append_via_padstack(2);
        assert_eq!(board.via_padstacks, vec![2, 1], "name dedup: no re-append");
        board.append_via_padstack(1);
        assert_eq!(board.via_padstacks, vec![2, 1], "ViaPad_F already listed");

        // The tail replace clobbers via-info appends and can WIPE the
        // list entirely (a net class without `(circuit (use_via ...))`
        // makes viaPadstackNames non-null empty — jar probe outcome).
        board.set_via_padstacks(Vec::new());
        assert!(board.via_padstacks.is_empty(), "empty replace = wipe");
        assert_eq!(board.via_padstack_no("ViaPad_F"), None);
    }

    /// The T51 probe fixture (exact bytes of /tmp/t9rev-closed.dsn): a
    /// closed 3-corner UNFIXED wire, then an open wire, both on net
    /// PERFECT over the um/10 two-layer board.
    const T9REV_CLOSED_DSN: &str = r#"(pcb t9rev-closed.dsn
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
  )
  (network
    (net PERFECT)
  )
  (wiring
    (wire (path F.Cu 125  1000 1000  3000 1000  3000 3000  1000 1000) (net PERFECT))
    (wire (path F.Cu 125  5000 1000  5000 2000) (net PERFECT))
  )
)
"#;

    /// Jar /tmp/t9rev-two.out CASE closed: the closed sub-USER_FIXED wire
    /// is dropped by `BasicBoard.java:183-201`'s second guard — but the
    /// `Item` constructor (`Item.java:86-89`) burned its id first — so
    /// only the open trace is stored, at id 3, and GEN_MAX is 3 (id 2
    /// consumed by the dropped closed wire). WARN_COUNT 0.
    #[test]
    fn t51_closed_trace_drop_burns_an_id() {
        let mut board = SesBoard::new();
        let result = crate::reader::read_board(T9REV_CLOSED_DSN.as_bytes(), &mut board);
        match result {
            crate::reader::DsnReadResult::Success { warnings } => {
                assert!(warnings.is_empty(), "WARN_COUNT 0, got {warnings:?}");
            }
            other => panic!("expected Success, got {other:?}"),
        }
        assert_eq!(board.items.len(), 2, "outline + open trace only");
        let traces: Vec<(i32, [i32; 2], FixedStateIr)> = board
            .items
            .iter()
            .filter_map(|item| match item {
                ItemIr::Trace { id, trace } => {
                    Some((*id, [trace.corners[0].x, trace.corners[0].y], trace.fixed))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            traces,
            vec![(3, [50000, 10000], FixedStateIr::Unfixed)],
            "TRACE id=3 first=(50000,10000) fixed=UNFIXED"
        );
        assert_eq!(
            board.last_assigned_item_id(),
            3,
            "GEN_MAX 3 — the closed wire burned id 2"
        );
    }

    /// T49 pins — [`SesBoard::obstacle_absolute_area`] against the Java
    /// oracle. Every row's input AND expected output were captured from
    /// `build/libs/freerouting-current-executable.jar` by the one-time
    /// jshell probe `/tmp/epic-t12-probe.jsh` (reflection off
    /// `ObstacleArea`: `translation`/`rotationInDegree`/`sideChanged`,
    /// expected = the item's `getArea()` output; doubles via
    /// `Double.toString`). Each row names its corpus fixture. The rows
    /// cover every `getArea()` branch (`ObstacleArea.java:119-144`):
    /// translation-only, exact 90-degree turns x1/x-1/x2/x3 under both
    /// flip styles, `rotateApprox` at an odd angle, the default-order
    /// mirror (mirror -> rotate -> translate), and the rotate-first
    /// ordering (rotate -> mirror -> translate).
    ///
    /// The rows `back_rot0_mirror_moves_x` and
    /// `flip_first_rotate_then_mirror` are REGENERABLE from the
    /// supplemental probe `/tmp/epic-t49b.jsh` (output
    /// `/tmp/epic-t49b.out`, lines `T49B MIRROR-ABS id=682` and
    /// `T49B FLIPFIRST-CIRC abs`); its early-`return` sweep bug is fixed
    /// so part 2 (the rotate-first composition) runs.
    #[test]
    fn t49_obstacle_absolute_area_matches_jar_captures() {
        use epic_geometry::circle::Circle as BoardCircle;

        struct Case {
            name: &'static str,
            provenance: &'static str,
            flip_style: Option<&'static str>,
            border: BoardShape,
            translation: (i32, i32),
            rotation: f64,
            side_changed: bool,
            expected_border: BoardShape,
        }
        let circle =
            |x: i32, y: i32, r: i32| BoardShape::Circle(BoardCircle::new(IntPoint::new(x, y), r));
        let rect = |x0: i32, y0: i32, x1: i32, y1: i32| {
            BoardShape::PolygonShape(PolygonShape::new(&[
                Point::Int(IntPoint::new(x0, y0)),
                Point::Int(IntPoint::new(x1, y0)),
                Point::Int(IntPoint::new(x1, y1)),
                Point::Int(IntPoint::new(x0, y1)),
            ]))
        };
        let cases = vec![
            // T49 BRANCH flipFirst=false sideChanged=false rot0 (count=2572).
            // A no-translate bug pins x at 39500 (vs 1370841).
            Case {
                name: "translation_only",
                provenance: "dsn-0002 DAC2020_bm02.dsn keepout id=362",
                flip_style: None,
                border: circle(39500, 0, 10000),
                translation: (1331341, -1036066),
                rotation: 0.0,
                side_changed: false,
                expected_border: circle(1370841, -1036066, 10000),
            },
            // T49 BRANCH flipFirst=false sideChanged=false rot90x1 (count=57).
            // turn90Degree(1) is CCW: (109000,37000) -> (-37000,109000).
            Case {
                name: "front_rot90_ccw",
                provenance: "dsn-0012 DAC2020_bm05.dsn keepout id=162",
                flip_style: None,
                border: circle(109000, 37000, 11000),
                translation: (1384311, -1233536),
                rotation: 90.0,
                side_changed: false,
                expected_border: circle(1347311, -1124536, 11000),
            },
            // T49 BRANCH flipFirst=false sideChanged=false rot90x-1 (count=66).
            // turn90Degree(-1) is CW: (50800,-88900) -> (-88900,-50800).
            Case {
                name: "front_rot_minus90_cw",
                provenance: "dsn-0019 1-Wire_Wing unrouted.dsn keepout id=170",
                flip_style: None,
                border: circle(50800, -88900, 18750),
                translation: (1193160, -1089020),
                rotation: -90.0,
                side_changed: false,
                expected_border: circle(1104260, -1139820, 18750),
            },
            // T49 BRANCH flipFirst=false sideChanged=false rot90x2 (count=536).
            // turn90Degree(2) negates both: (35800,0) -> (-35800,0).
            Case {
                name: "front_rot180",
                provenance: "dsn-0022 CM5_MINIMA_3.dsn keepout id=655",
                flip_style: None,
                border: circle(35800, 0, 6000),
                translation: (861500, -622500),
                rotation: 180.0,
                side_changed: false,
                expected_border: circle(825700, -622500, 6000),
            },
            // T49 BRANCH flipFirst=true sideChanged=false rot90x3 (count=12):
            // rotate_first metadata alone must not mirror a front keepout.
            // turn90Degree(3): (0,-220000) -> (-220000,0).
            Case {
                name: "front_rot270_under_rotate_first",
                provenance: "dsn-0051 Issue143-rpi_splitter.dsn keepout id=33",
                flip_style: Some("rotate_first"),
                border: circle(0, -220000, 146600),
                translation: (1016000, 3556000),
                rotation: 270.0,
                side_changed: false,
                expected_border: circle(796000, 3556000, 146600),
            },
            // T49 BRANCH flipFirst=false sideChanged=true rot0 — the pure
            // mirror branch with a NON-DEGENERATE center (x=28800 != 0; the
            // CM5_MINIMA first example at x=0 cancels the mirror and is
            // anchor-blind). Mirror: (28800,12900) -> (-28800,12900).
            // Regenerable: /tmp/epic-t49b.out `T49B MIRROR-ABS id=682`.
            Case {
                name: "back_rot0_mirror_moves_x",
                provenance: "dsn-0083 Issue297-myboard.dsn keepout id=682",
                flip_style: None,
                border: circle(28800, 12900, 5750),
                translation: (1365223, -1614442),
                rotation: 0.0,
                side_changed: true,
                expected_border: circle(1336423, -1601542, 5750),
            },
            // T49 BRANCH flipFirst=false sideChanged=true rot90x1 — the
            // mirror-PROVING pin: mirror-then-turn90 gives (5600,-15000);
            // a mirror-skipping bug gives (5600,15000) and abs y -713800
            // instead of -743800.
            Case {
                name: "back_rot90_mirror_proving",
                provenance: "dsn-0022 CM5_MINIMA_3.dsn keepout id=1267",
                flip_style: None,
                border: circle(15000, -5600, 6500),
                translation: (1301800, -728800),
                rotation: 90.0,
                side_changed: true,
                expected_border: circle(1307400, -743800, 6500),
            },
            // T49 BRANCH flipFirst=false sideChanged=true rot90x2 — mirror,
            // then turn90Degree(2): (-38100,-25400) -> (38100,-25400) ->
            // (-38100,25400).
            Case {
                name: "back_rot180_mirror_then_turn",
                provenance: "dsn-0088 corney_island_wireless.dsn keepout id=25",
                flip_style: None,
                border: circle(-38100, -25400, 17500),
                translation: (1950000, -786250),
                rotation: 180.0,
                side_changed: true,
                expected_border: circle(1911900, -760850, 17500),
            },
            // T49 BRANCH flipFirst=false sideChanged=true rot90x3 — the
            // polygon branch: mirror + turn90Degree(3) + translate. The
            // expected corner SEQUENCE is the jar's `getArea()` dump verbatim
            // (Java re-normalizes the polygon's start corner under the
            // transform; hand-permuting rel corners gives the same corner
            // set in a DIFFERENT order — pinned as captured, not recomputed).
            Case {
                name: "back_rot270_polygon",
                provenance: "dsn-0155 Issue732-RoyalBlue54L-Feather.dsn keepout id=961",
                flip_style: None,
                border: rect(-12700, -6350, 12700, 6350),
                translation: (1286100, -1049725),
                rotation: 270.0,
                side_changed: true,
                expected_border: BoardShape::PolygonShape(PolygonShape::new(&[
                    Point::Int(IntPoint::new(1279750, -1062425)),
                    Point::Int(IntPoint::new(1292450, -1062425)),
                    Point::Int(IntPoint::new(1292450, -1037025)),
                    Point::Int(IntPoint::new(1279750, -1037025)),
                ])),
            },
            // T49 BRANCH flipFirst=false sideChanged=false rotOdd:330.0 — the
            // rotateApprox branch (cos330 * -55000 = -47631.397 -> -47631).
            // Case below shares this exact input with side_changed=true +
            // rotate_first, so the two rows together discriminate BOTH the
            // mirror and the ordering (wrong order -> y -1369250 here).
            Case {
                name: "front_rot330_approx",
                provenance: "dsn-0033 Issue054-tairakb.dsn keepout id=1479",
                flip_style: None,
                border: circle(-55000, 0, 9000),
                translation: (1742920, -1341750),
                rotation: 330.0,
                side_changed: false,
                expected_border: circle(1695289, -1314250, 9000),
            },
            // rotate-first ordering discriminator (supplemental probe
            // /tmp/epic-t49b.jsh: no corpus fixture combines flip_style
            // rotate_first with a side-changed keepout, so the probe applied
            // the flipRotateFirst=true order — rotateApprox, mirrorVertical,
            // translateBy — with Java's own Area methods on the jar-parsed
            // relative area of the same keepout). rotateApprox rounds FIRST
            // (-47631,27500), then the mirror flips to (47631,27500):
            // abs (1790551,-1314250). The default order on the same input is
            // the row above (1695289,-1314250) — the ordering swap moves
            // BOTH coordinates.
            Case {
                name: "flip_first_rotate_then_mirror",
                provenance: "dsn-0033 Issue054-tairakb.dsn keepout id=1479 shape, rotate_first order",
                flip_style: Some("rotate_first"),
                border: circle(-55000, 0, 9000),
                translation: (1742920, -1341750),
                rotation: 330.0,
                side_changed: true,
                expected_border: circle(1790551, -1314250, 9000),
            },
        ];
        for case in &cases {
            let mut board = SesBoard::new();
            if let Some(flip_style) = case.flip_style {
                board.set_flip_style(flip_style.to_string());
            }
            let area = AreaIr::simple(case.border.clone());
            let absolute = board.obstacle_absolute_area(
                &area,
                IntPoint::new(case.translation.0, case.translation.1),
                case.rotation,
                case.side_changed,
            );
            assert_eq!(
                absolute,
                AreaIr::simple(case.expected_border.clone()),
                "T49 {} ({}): wrong placement transform",
                case.name,
                case.provenance
            );
        }
    }

    /// T49 rotateApprox ROUNDING pin (supplemental probe
    /// `/tmp/epic-t49c.jsh` -> `/tmp/epic-t49c.out`, lines
    /// `T49C ROUNDDISC`): 10 degrees on a circle at (55000, 0). The
    /// Y coordinate is the round-vs-truncate discriminator —
    /// sin(10 deg) * 55000 = 9550.6498..., so round-half-up gives
    /// 9551 while truncate/floor give 9550. (X does NOT discriminate:
    /// cos(10 deg) * 55000 = 54164.4264 rounds down to 54164 either
    /// way; the row 10 rot330 pin had the same blindness with
    /// -47631.397, which is why this one exists.) Synthetic
    /// composition of Java's own `Circle.rotateApprox(toRadians(10),
    /// FloatPoint 0)` + `translateBy` — the exact ops
    /// `obstacle_absolute_area` performs at rotation=10, side_changed
    /// = false.
    #[test]
    fn t49_rotate_approx_rounds_half_up_not_truncates() {
        use epic_geometry::circle::Circle as BoardCircle;
        let board = SesBoard::new();
        let area = AreaIr::simple(BoardShape::Circle(BoardCircle::new(
            IntPoint::new(55000, 0),
            9000,
        )));
        let absolute =
            board.obstacle_absolute_area(&area, IntPoint::new(1742920, -1341750), 10.0, false);
        assert_eq!(
            absolute,
            AreaIr::simple(BoardShape::Circle(BoardCircle::new(
                IntPoint::new(1797084, -1332199),
                9000
            ))),
            "T49 rotateApprox rounding: y=-1341750+9551 (round) not -1341750+9550 (truncate)"
        );
    }

    /// T49 HOLES arm pin (supplemental probe `/tmp/epic-t49c.jsh` ->
    /// `/tmp/epic-t49c.out`, lines `T49C HOLES`): a holed
    /// [`AreaIr`] transformed by `obstacle_absolute_area` must apply
    /// the SAME default-order transform (mirrorVertical ->
    /// turn90Degree(1) -> translateBy) to every hole as to the
    /// border. No corpus keepout carries `(window)` holes (16 files
    /// do overall, none as image keepouts), so this is a SYNTHETIC
    /// composition: a border rect(-20000,-10000,20000,10000) with a
    /// hole rect(-5000,-2500,5000,2500), pushed through Java's own
    /// `PolylineArea.mirrorVertical/turn90Degree/translateBy`. The
    /// expected corner SEQUENCES are the jar dump verbatim (Java
    /// re-normalizes start corner under the transform — same rule as
    /// the `back_rot270_polygon` row: pinned as captured, not
    /// recomputed). A border-only implementation leaves the hole at
    /// its relative position — off by the full mirror+turn here.
    #[test]
    fn t49_placement_transform_reaches_holes() {
        let rect = |x0: i32, y0: i32, x1: i32, y1: i32| {
            BoardShape::PolygonShape(PolygonShape::new(&[
                Point::Int(IntPoint::new(x0, y0)),
                Point::Int(IntPoint::new(x1, y0)),
                Point::Int(IntPoint::new(x1, y1)),
                Point::Int(IntPoint::new(x0, y1)),
            ]))
        };
        let board = SesBoard::new();
        let area = AreaIr {
            border: rect(-20000, -10000, 20000, 10000),
            holes: vec![rect(-5000, -2500, 5000, 2500)],
        };
        let absolute =
            board.obstacle_absolute_area(&area, IntPoint::new(1301800, -728800), 90.0, true);
        let expected_border = rect(1291800, -748800, 1311800, -708800);
        // jar corner order: (1291800,-748800) (1311800,-748800)
        // (1311800,-708800) (1291800,-708800) — identical to the rect()
        // construction here (start corner survives the normalization),
        // verified against the dump.
        assert_eq!(
            absolute,
            AreaIr {
                border: expected_border,
                holes: vec![BoardShape::PolygonShape(PolygonShape::new(&[
                    Point::Int(IntPoint::new(1299300, -733800)),
                    Point::Int(IntPoint::new(1304300, -733800)),
                    Point::Int(IntPoint::new(1304300, -723800)),
                    Point::Int(IntPoint::new(1299300, -723800)),
                ]))],
            },
            "T49 holes arm: hole must mirror+turn+translate with the border"
        );
    }
}
