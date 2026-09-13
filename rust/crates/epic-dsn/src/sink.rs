//! The `BoardSink` seam (plan decision D9): the trait the DSN scope
//! readers (M1b Tasks 5-9) emit parse results through, mirroring the Java
//! parser's callback plus the `BasicBoard` insertion methods those readers
//! drive.
//!
//! ## Relationship to Java `BoardParserCallback`
//!
//! Java's seam (`parser/BoardParserCallback.java:18`, 3 methods) is a
//! LIFECYCLE interface: `getRoutingBoard` (`:23`, accessor for the board
//! created by an earlier callback), `createBoard` (`:29-36`, the structure
//! reader hands over bounds/layers/outline/rules and the implementation
//! constructs the board) and `getCurrentRoutingJob` (`:41`, `null` in pure
//! DSN-reader mode). The production implementation is the package-private
//! `MinimalBoardManager` nested in `ReadScopeParameter`; the insertion
//! calls the scope readers make afterwards go through the created
//! `BasicBoard` directly.
//!
//! The Rust split merges the two roles into one object-safe trait:
//!
//! - `getRoutingBoard` has NO counterpart — the parser holds
//!   `&mut dyn BoardSink` and never reads a board back mid-parse; the
//!   queries below ([`BoardSink::resolve_padstack_query`],
//!   [`BoardSink::padstack_no`], [`BoardSink::net_no`],
//!   [`BoardSink::clearance_class_no`]) cover
//!   exactly what the Java readers legitimately look up mid-parse
//!   (`Padstacks.get`, `Nets.get`, `ClearanceMatrix.get_no`).
//! - `createBoard` (`BoardParserCallback.java:29-36`) maps to
//!   [`BoardSink::create_board`] with router types (`BoardRules`,
//!   `Communication`) replaced by their parse-derivable IR (the transform;
//!   the clearance matrix starts from the default instance on the sink
//!   side, `Structure.java:1236`).
//! - `getCurrentRoutingJob` (`:41`) has NO counterpart — always `null` in
//!   DSN-reader mode (the javadoc says so); headless parse has no job.
//!
//! ## Contract rules (D9)
//!
//! - Method-per-insertion, IR-typed parameters, NO board intelligence:
//!   no normalization, no contact sets, no router types. The readers own
//!   all decisions (drop rules, warning strings, `.N` aliasing *calls*);
//!   the sink stores what it is given and assigns item ids.
//! - Item ids are assigned BY THE SINK on insertion, mirroring Java where
//!   `Item` fetches `board.communication.idGenerator.newId()` inside the
//!   board's insert path (`board/model/items/Item.java:87`);
//!   `ItemIdGenerator` is a counter starting at 0 whose `newId` is
//!   `++lastGeneratedId` — ids are sequential from 1. Jar-verified end to
//!   end on a crafted fixture (session `/tmp/epic-t4-ids.jsh`, output
//!   `/tmp/epic-t4-ids.out`): ids 1..8 dense, id 1 = the `BoardOutline`
//!   the `BasicBoard` constructor inserts (`BasicBoard.java:136`), id 2 =
//!   the structure keepout, ids 3-4 = pins, 5-6 = traces, 7 = via,
//!   8 = conduction area, `GEN_MAX 8` = item count. The id argument in
//!   the IR structs is therefore write-only for callers: pass `0` and
//!   read the id returned by the insert method.
//! - [`FixedStateIr`] mirrors the Java `FixedState` enum and its ORDINAL
//!   ORDER (UNFIXED, SHOVE_FIXED, USER_FIXED, SYSTEM_FIXED) — order is
//!   load-bearing for SES writing (`SesWriter` maps by state, Task 13).
//!   T36 (jar-confirmed in Task 1): `(type route)` -> USER_FIXED,
//!   `fix` -> SYSTEM_FIXED (spike items 5/6 in `/tmp/epic-t4-ids.out`).

use crate::coordinate_transform::CoordinateTransform;
use crate::layer_structure::LayerStructure;
use crate::shape::BoardShape;
use crate::state::{AngleRestriction, ComponentLocation};
use epic_geometry::int_box::IntBox;
use epic_geometry::int_point::IntPoint;

/// Java `FixedState` (`board/model/items/FixedState.java`): enum order
/// UNFIXED, SHOVE_FIXED, USER_FIXED, SYSTEM_FIXED (ordinal order is
/// load-bearing — module docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FixedStateIr {
    /// Java `UNFIXED`.
    Unfixed,
    /// Java `SHOVE_FIXED` (set by the router, never by the parser).
    ShoveFixed,
    /// Java `USER_FIXED` — `(type route)` wires and the T37 plane
    /// heuristic (T36).
    UserFixed,
    /// Java `SYSTEM_FIXED` — `(type fix)` wires, keepouts, outlines (T36).
    SystemFixed,
}

/// Java `ItemClass` (`rules/netClasses/DefaultItemClearanceClasses.java`
/// key enum): NONE, TRACE, VIA, PIN, SMD, AREA. The DEFAULT clearance
/// class of every item class is index 1 (`"default"`) — the SMD/PIN
/// default split happens reader-side (`Network.java:1027-1033`), which is
/// why pins carry an explicit class in [`PinIr`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemClassIr {
    /// Java `NONE`.
    None,
    /// Java `TRACE`.
    Trace,
    /// Java `VIA`.
    Via,
    /// Java `PIN`.
    Pin,
    /// Java `SMD`.
    Smd,
    /// Java `AREA`.
    Area,
}

impl ItemClassIr {
    /// Java `DefaultItemClearanceClasses.getNo(ItemClass)` — every class
    /// defaults to clearance class 1 (`"default"`).
    pub fn default_clearance_class(self) -> i32 {
        1
    }
}

/// The `border + holes` area IR (Task 3 review pre-decision, recorded in
/// the plan Task 6 bullet): `BoardShape` alone cannot carry `(window)`
/// holes, and Java's parse-time result is either a bare
/// `geometry.planar` shape or a `PolylineArea(border, holes)`
/// (`Shape.transformAreaToBoard`; the first shape is the border, the rest
/// are holes). A keepout/plane/conduction area without windows has
/// `holes: []`.
#[derive(Clone, Debug, PartialEq)]
pub struct AreaIr {
    /// The border shape (board coordinates).
    pub border: BoardShape,
    /// The `(window ...)` holes (board coordinates), in file order.
    pub holes: Vec<BoardShape>,
}

impl AreaIr {
    /// A hole-free area.
    pub fn simple(border: BoardShape) -> Self {
        Self {
            border,
            holes: Vec::new(),
        }
    }
}

/// Java `Padstack` parse subset (`Library.java:222`
/// `boardPadstacks.add(padstackName, padstackShapes, isDrilllable,
/// placedAbsolute)`): a library padstack in BOARD coordinates (the reader
/// transforms the shape scopes before calling the sink).
#[derive(Clone, Debug, PartialEq)]
pub struct PadstackIr {
    /// Java `Padstack.name`. The library-scope reader strips the `.N`
    /// alias at READ time (`Library.java:113`,
    /// `replaceAll("\\.\\d+", "")`), so DSN padstacks arrive here
    /// pre-stripped and the sink stores them verbatim; query sites strip
    /// again idempotently ([`BoardSink::resolve_padstack_query`]).
    pub name: String,
    /// Java `Padstack.shapes` — the per-layer `ConvexShape[layerCount]`
    /// array (`Library.java:96-118` sizes it, `Padstacks.add` stores it),
    /// indexed by 0-based layer number with length == layer count; `None`
    /// is a Java null slot.
    ///
    /// Why per-layer slots and not `(layer set, shape)` pairs: this IS
    /// Java's storage, `SesWriter.writePadstack` walks it per index
    /// (`SesWriter.java:271-307`, emitting `getShape(i)` per layer), and
    /// a DSN padstack may mix a layer-less `(shape ...)` (Java FILLS all
    /// slots) with layered `(shape (layers ...) ...)` scopes (ASSIGNS
    /// only the listed slots, later scopes overwriting earlier fills —
    /// `Library.java:179-221`). Pairs could not represent per-slot
    /// overwrite order. The fill-vs-assign expansion itself stays
    /// READER-side (Task 7).
    pub shapes: Vec<Option<BoardShape>>,
    /// Java `Padstack.is_drillable`.
    pub drillable: bool,
    /// Java `Padstack.placed_absolute`.
    pub placed_absolute: bool,
}

/// Java `Package.Pin` parse subset (`core/library/Package.java` fields
/// `name`, `padstackId`, `relativeLocation`, `rotationInDegree`): one pin
/// of a library image/package.
#[derive(Clone, Debug, PartialEq)]
pub struct ImagePinIr {
    /// Java `Package.Pin.name` — the pin number/name as written in the
    /// `(pin <padstack> <name> <x> <y> [...])` scope.
    pub name: String,
    /// Java `Package.Pin.padstackId` — the resolved 1-based padstack
    /// number (reader resolves through [`BoardSink::resolve_padstack_query`],
    /// `Library.java:321-323`).
    pub padstack_no: i32,
    /// Java `Package.Pin.relativeLocation` — board coordinates relative
    /// to the image origin (`Package.java` stores the `Vector` as read;
    /// front/back mirroring happens at pin-expansion time, Task 8,
    /// `Network.java:979-1034`).
    pub rel_location: IntPoint,
    /// Java `Package.Pin.rotationInDegree`.
    pub rotation: f64,
}

/// Java `Package.Keepout` parse subset (`core/library/Package.java`
/// nested `Keepout`: fields `name`, `area`, `layer`): one keepout of a
/// library image, in image-relative coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageKeepoutIr {
    /// Java `Package.Keepout.name`.
    pub name: String,
    /// Java `Package.Keepout.layer` — 0-based layer number.
    pub layer_no: i32,
    /// Java `Package.Keepout.area` (board coordinates; windows ride in
    /// [`AreaIr`]).
    pub area: AreaIr,
}

/// One outline shape of a library image. Java `Package` stores three
/// PARALLEL arrays (`outline: Shape[]`, `outlineWidths: double[]`,
/// `outlineIsClosed: boolean[]`, all read from the `(outline ...)`
/// scope); the struct is one aligned slice of those arrays.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageOutlineIr {
    /// `Package.outline[i]` — board coordinates relative to the image
    /// origin.
    pub shape: BoardShape,
    /// `Package.outlineWidths[i]` — the `(outline ... (width w))` value.
    pub width: f64,
    /// `Package.outlineIsClosed[i]`.
    pub is_closed: bool,
}

/// Java `Package` parse subset (`core/library/Package.java` fields): one
/// library image (DSN `(image ...)` scope). Insertion-ordered table =
/// Java `Packages` (`Packages.java:14` `Vector<Package>`); ids are
/// 1-based positions. Two consumers depend on this table existing in
/// LIBRARY order: Task 8 pin expansion (`Network.java:979-1034` resolves
/// `placement` lib names through `Packages.get`) and Task 13 placement
/// grouping (`SesWriter` groups placements per package). The dedup that
/// rewrites duplicate image names to `<name>::<k>` happens in the
/// READER before this lands (`Library.java:409-443`; jar
/// `/tmp/epic-t4c-images.out`: identical-pin dup deduped, different-pin
/// dup inserted as `PAD::1`, case-variant same-pin dup deduped).
#[derive(Clone, Debug, PartialEq)]
pub struct ImageIr {
    /// Java `Package.name` (post-dedup: possibly `NAME::k`).
    pub name: String,
    /// Java `Package.pins` (`Pin[]`), file order.
    pub pins: Vec<ImagePinIr>,
    /// Java `Package.outline` + `outlineWidths` + `outlineIsClosed`,
    /// zipped, file order.
    pub outline: Vec<ImageOutlineIr>,
    /// Java `Package.keepouts`.
    pub keepouts: Vec<ImageKeepoutIr>,
    /// Java `Package.viaKeepouts`.
    pub via_keepouts: Vec<ImageKeepoutIr>,
    /// Java `Package.placeKeepoutArr`.
    pub place_keepouts: Vec<ImageKeepoutIr>,
    /// Java `Package.isFront` (an `(image ... (front)` / back flip).
    pub is_front: bool,
}

/// Java `rules.Net` parse subset: the board net table entry. The net
/// NUMBER is the 1-based position in insertion order (`Nets.add`:
/// `nets.size() + 1`, `rules/Nets.java:88-96` — ALWAYS appends, no
/// lookup), carried implicitly by [`SesBoard`](crate::ses_board::SesBoard)'s
/// table order.
#[derive(Clone, Debug, PartialEq)]
pub struct NetIr {
    /// Java `Net.name` — as written in the DSN (case preserved; the
    /// case-SENSITIVE is-new guard is the parser `NetList` TreeMap keyed
    /// by `Net.Id.compareTo` = `String.compareTo`
    /// (`NetList.java:13-33`, `parser/Net.java:95-98`, guard at
    /// `Network.java:1410`), so case variants are DISTINCT nets; the
    /// board-side lookup by name is case-insensitive
    /// (`rules/Nets.java:42-62`)).
    pub name: String,
    /// Java `Net.subnetNumber` (`rules/Net.java:29`): the optional
    /// positional integer directly after the net name in the `(net ...)`
    /// scope (`Network.java:1336-1345`, default 1) — a bare integer, NOT
    /// a `(subnet_number N)` scope. A net split into several subnets
    /// inserts one board net per subnet with the SAME name and
    /// incrementing subnet numbers (`Network.java:1461`).
    pub subnet_number: i32,
}

/// Java `ComponentPlacement` group + one placed instance, stored in
/// insertion (file) order; ids are the 1-based positions. Reuses the
/// Task 2 placement IR ([`ComponentLocation`]) unchanged — the placement
/// scope reader (Task 7) parses into this shape.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacementIr {
    /// Java `ComponentPlacement.libName`: the library image name.
    pub lib_name: String,
    /// Java `ComponentLocation` for the placed instance.
    pub location: ComponentLocation,
}

/// Java `Component` parse subset (`Network.java:957-966`
/// `components.add(name, placement)`): id = `components.size() + 1`.
#[derive(Clone, Debug, PartialEq)]
pub struct ComponentIr {
    /// Java `Component.name` (the placed instance name, e.g. `U1`).
    pub name: String,
    /// 0-based index into the placements table (Java stores the
    /// `ComponentLocation` reference).
    pub placement: usize,
}

/// Java `rules.NetClass` parse subset (`Network.java:436-543`
/// `insertNetClass`); extended field-for-field by the Task 8 reader.
#[derive(Clone, Debug, PartialEq)]
pub struct NetClassIr {
    /// Java `NetClass.name`.
    pub name: String,
    /// Java `NetClass.clearance_class` — the clearance-class INDEX.
    pub clearance_class: i32,
    /// Java `NetClass.items` — the contained net numbers.
    pub nets: Vec<i32>,
}

/// Java `ViaInfo` (`rules/ViaInfo.java` fields: name, padstack,
/// clearanceClassIndex, attachSmdAllowed) — one `(via ...)` rule of a net
/// class (Task 8, `Network.java:1286-1318` resolves the padstack through
/// the T32 query strip before this lands).
#[derive(Clone, Debug, PartialEq)]
pub struct ViaInfoIr {
    /// Java `ViaInfo.name`.
    pub name: String,
    /// Java `ViaInfo.padstack` — the resolved 1-based padstack number.
    pub padstack_no: i32,
    /// Java `ViaInfo.clearance_class_index`.
    pub clearance_class: i32,
    /// Java `ViaInfo.attach_smd_allowed`.
    pub attach_smd_allowed: bool,
}

/// Java `Pin` parse subset (`Network.java:1035` `insertPin(newComponent.id,
/// i, netNumberArray, clearanceClass, fixedState)`): a component pin,
/// placed at the transformed package-pin location.
#[derive(Clone, Debug, PartialEq)]
pub struct PinIr {
    /// Java `Pin.component_no`.
    pub component_id: i32,
    /// Java `Pin.pin_index` (0-based position in the image pin list).
    pub pin_index: i32,
    /// The resolved padstack number (1-based).
    pub padstack_no: i32,
    /// Java `Pin.getCenter()` — board coordinates (spike:
    /// `place U1 2000 5000` at scale 10 -> `(20000, 50000)`).
    pub center: IntPoint,
    /// Java `Pin.netNumbers`.
    pub nets: Vec<i32>,
    /// Java clearance class index — SMD (same-layer padstack) vs PIN
    /// default is a READER decision (`Network.java:1027-1033`).
    pub clearance_class: i32,
    /// Java `Pin.fixedState` (network-scope pins are UNFIXED).
    pub fixed: FixedStateIr,
}

/// Java `PolylineTrace` parse subset (`Wiring.java` `insertTraceWithoutCleaning`,
/// `:534`/`:564`): corners are the INSERTED polyline corners in file
/// order, already rounded to board coordinates; NO normalization (the
/// Java parse-time `normalizeAllTraces` call is deliberately not ported,
/// D11).
#[derive(Clone, Debug, PartialEq)]
pub struct TraceIr {
    /// Java `Trace.get_layer()` — 0-based layer number.
    pub layer_no: i32,
    /// Java `Trace.get_half_width()` — `(int) Math.round(dsnToBoard(width
    /// / 2))` is a READER computation (`Wiring.java:454`).
    pub half_width: i32,
    /// The corner sequence (board coordinates, file order).
    pub corners: Vec<IntPoint>,
    /// Java `Trace.netNumbers` (the wire's single net).
    pub nets: Vec<i32>,
    /// Java clearance class index.
    pub clearance_class: i32,
    /// T36: `(type route)` -> USER_FIXED, `(type fix)` -> SYSTEM_FIXED.
    pub fixed: FixedStateIr,
}

/// Java `Via` parse subset (`Wiring.java:706` `insertVia(padstack, loc,
/// nets, class, fixed, attachAllowed)`).
#[derive(Clone, Debug, PartialEq)]
pub struct ViaIr {
    /// The resolved padstack number (1-based; T32 `.N` query strip
    /// happened reader-side, `Wiring.java:659-671`).
    pub padstack_no: i32,
    /// Java via location — `(int) Math.round(dsnToBoard(coor))` per axis
    /// is a READER computation (`Wiring.java:698`).
    pub location: IntPoint,
    /// Java `Via.netNumbers`.
    pub nets: Vec<i32>,
    /// Java clearance class index.
    pub clearance_class: i32,
    /// T36 fixed state.
    pub fixed: FixedStateIr,
    /// Java `attach_smd_allowed`.
    pub attach_smd_allowed: bool,
}

/// Java `ObstacleArea` keepout kinds: the three structure scopes
/// (`keepout`, `place_keepout`, `via_keepout`, Task 6) and the three
/// package-info kinds (Task 8, `Network.java:1039-1124`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeepoutKindIr {
    /// Java `ItemClass.AREA` keepout (`(keepout ...)` / `keepout_infos`).
    Keepout,
    /// Java place keepout (`(place_keepout ...)` / `place_keepout_infos`).
    PlaceKeepout,
    /// Java via keepout (`(via_keepout ...)` / `via_keepout_infos`).
    ViaKeepout,
}

/// Java `ObstacleArea` keepout parse subset (structure keepouts
/// `Structure.java:862-876` + outline holes `:1279-1283` + package
/// keepouts `Network.java:1039-1124`). Parse-time keepouts never carry
/// nets (the digest `K` line therefore has no net list).
#[derive(Clone, Debug, PartialEq)]
pub struct KeepoutIr {
    /// Which of the three keepout item classes.
    pub kind: KeepoutKindIr,
    /// 0-based layer number.
    pub layer_no: i32,
    /// The border + window holes.
    pub area: AreaIr,
    /// Java clearance class index.
    pub clearance_class: i32,
    /// Structure keepouts are SYSTEM_FIXED (spike item 2).
    pub fixed: FixedStateIr,
}

/// Java `ConductionArea` parse subset: plane scopes (`Structure.java:
/// 1068-1122`, Task 6) and rectangle wires (`Wiring.java:483-486`,
/// Task 9). Planes can cover several nets (`A` line net list); a rectangle
/// wire has exactly one.
#[derive(Clone, Debug, PartialEq)]
pub struct ConductionAreaIr {
    /// 0-based layer number.
    pub layer_no: i32,
    /// The border + window holes.
    pub area: AreaIr,
    /// Java `ConductionArea.netNumbers` (spike item 8: `[1]`).
    pub nets: Vec<i32>,
    /// Java clearance class index.
    pub clearance_class: i32,
    /// UNFIXED for parse-time rectangle wires (spike item 8); USER_FIXED
    /// when the T37 plane heuristic fires (Task 6).
    pub fixed: FixedStateIr,
}

/// Java `ComponentOutline` parse subset (`Network.java:1192`, inserted
/// when a package outline has more than one shape): burns an item id but
/// emits NO digest line (the digest schema covers T/V/K/A only).
#[derive(Clone, Debug, PartialEq)]
pub struct ComponentOutlineIr {
    /// Java `ComponentOutline.component_no`.
    pub component_id: i32,
    /// 0-based layer number.
    pub layer_no: i32,
    /// The border + window holes.
    pub area: AreaIr,
    /// Java `ComponentOutline.netNumbers` (empty at parse time).
    pub nets: Vec<i32>,
    /// Java clearance class index.
    pub clearance_class: i32,
    /// Java fixed state.
    pub fixed: FixedStateIr,
}

/// Java `BoardOutline` parse subset: created by the `BasicBoard`
/// constructor from the createBoard outline shapes (`BasicBoard.java:136`)
/// — the id-1 item of every parse (spike item 8 of
/// `/tmp/epic-t4-ids.out`, `id=1 type=BoardOutline`). Burns an id, emits
/// no digest line.
#[derive(Clone, Debug, PartialEq)]
pub struct BoardOutlineIr {
    /// The outline shapes (board coordinates; boundary holes are NOT part
    /// of this — they come back as separate keepout insertions,
    /// `Structure.java:1279-1283`).
    pub shapes: Vec<BoardShape>,
    /// Java `outlineClearanceClassName` from `createBoard`, resolved to
    /// its class index by the reader (Task 5).
    pub clearance_class: i32,
    /// SYSTEM_FIXED (spike).
    pub fixed: FixedStateIr,
}

/// Parse-time metadata snapshot (the `Communication`/parser-info fields
/// the digest and the SES writer need). Defaults mirror the parse-state
/// defaults (T24: MIL / 100; string quote `"`; snap angle 45).
#[derive(Clone, Debug, PartialEq)]
pub struct MetadataIr {
    /// Java `Unit` from `(resolution <unit> <n>)` (T24).
    pub unit: crate::state::Unit,
    /// Java resolution value.
    pub resolution: i32,
    /// Java `stringQuote` (default `"`).
    pub string_quote: String,
    /// Java `snapAngle`.
    pub snap_angle: AngleRestriction,
    /// Structure `(flip_style ...)` raw value (Task 6; `None` until set).
    pub flip_style: Option<String>,
    /// `(parser (host_cad ...))` (Task 9).
    pub host_cad: Option<String>,
    /// `(parser (host_version ...))` (Task 9).
    pub host_version: Option<String>,
}

impl Default for MetadataIr {
    /// The parse-state defaults (`ReadScopeParameter.java:36-89`).
    fn default() -> Self {
        Self {
            unit: crate::state::Unit::Mil,
            resolution: 100,
            string_quote: "\"".to_string(),
            snap_angle: AngleRestriction::FortyfiveDegree,
            flip_style: None,
            host_cad: None,
            host_version: None,
        }
    }
}

/// The `createBoard` argument IR (`BoardParserCallback.java:29-36` with
/// `BoardRules`/`Communication` reduced to their parse-derivable parts:
/// the rules IR arrives through the class/padstack setters as Tasks 5-8
/// land; the clearance matrix default instance is created sink-side,
/// `Structure.java:1236`).
#[derive(Clone, Debug, PartialEq)]
pub struct CreateBoardIr {
    /// Java `boundingBox` — the transformed boundary bounds INCLUDING the
    /// offset(1000) (T43, `Structure.java:1207-1208`, Task 5).
    pub bounding_box: IntBox,
    /// Java `layerStructure`.
    pub layer_structure: LayerStructure,
    /// Java `outlineShapes` — the transformed boundary shapes (board
    /// coordinates; becomes the id-1 [`BoardOutlineIr`] item).
    pub outline_shapes: Vec<BoardShape>,
    /// Java `outlineClearanceClassName` (Issue558 context: the name
    /// resolves against the clearance table; the resolution is Task 5).
    pub outline_clearance_class: String,
    /// The finalized `CoordinateTransform` (T25 loop already applied).
    pub transform: CoordinateTransform,
}

/// The seam the scope readers emit through (module docs for the Java
/// mapping and the id-assignment contract).
pub trait BoardSink {
    /// Java `createBoard` (`BoardParserCallback.java:29-36`, called from
    /// `Structure.java:1268-1274`): constructs the board from the parsed
    /// structure scope. Implementations insert the id-1 `BoardOutline`
    /// item (`BasicBoard.java:136`) and initialize the clearance table
    /// with the default instance `["null", "default"]`
    /// (`Structure.java:1236`).
    fn create_board(&mut self, board: CreateBoardIr);

    // ---- tables: nets, clearance classes, padstacks ---------------------

    /// Java `Nets.add(name, subnetNumber, containsPlane)` at the
    /// network/plane call sites (`Network.java:1414`,
    /// `Structure.java:1073`): ALWAYS appends — no lookup, no merge
    /// (`rules/Nets.java:88-96`, `newNetNo = nets.size() + 1`) — and
    /// returns the net number. The is-new guard lives in the READER: the
    /// parser `NetList` is a case-SENSITIVE TreeMap keyed by
    /// `Net.Id(name, subnetNumber)` (`NetList.java:13-33`; `compareTo` =
    /// `String.compareTo`, `parser/Net.java:95-98`), so case variants are
    /// distinct nets (jar `/tmp/epic-t4b-review.out`: `GND` + `gnd` give
    /// nets 1 AND 2) and an exact duplicate scope is skipped by the
    /// reader guard (`Network.java:1410`). Net number shifts from a merge
    /// here would diverge every downstream net reference.
    fn append_net(&mut self, net: NetIr) -> i32;

    /// Java `Nets.get(name, subnetNumber)` (`rules/Nets.java:42-51`):
    /// case-insensitive NAME match; the single-name overload
    /// (`:54-62`) returns ALL subnets — multi-subnet resolution follows
    /// `getSubnets` (`Wiring.java:223-236`) and lands with the Task 9
    /// wiring reader. This first-match query is `None` (Java null) on a
    /// miss.
    fn net_no(&self, name: &str) -> Option<i32>;

    /// Java `ClearanceMatrix.append_class`: appends a class unless a class
    /// with the name is already present (the dedup match rule is pinned by
    /// the Task 5 clearance pins; the two initial classes are `"null"` and
    /// `"default"`, `Structure.java:1236`).
    fn append_clearance_class(&mut self, name: &str);

    /// Java `ClearanceMatrix.get_no(name)`: case-insensitive lookup,
    /// `None` (Java -1) on a miss.
    fn clearance_class_no(&self, name: &str) -> Option<i32>;

    /// Java `Padstacks.add` (`Library.java:222`): ALWAYS appends (the
    /// dedup guard is at the CALL SITE, `Library.java:158-161`, on the
    /// name ALREADY stripped at `:113`) and returns the 1-based padstack
    /// number (`size() + 1`).
    fn append_padstack(&mut self, padstack: PadstackIr) -> i32;

    /// Java `Padstacks.get(name)` — RAW name, NO `.N` strip
    /// (`core/library/Padstacks.java:25-32`, case-insensitive first
    /// match). The one parse-side consumer is the `(via ...)` via-info
    /// rule fallback (`Network.java:275`; its first attempt,
    /// `getViaPadstack`, is a case-SENSITIVE via-subset lookup,
    /// `BoardLibrary.java:53-62`). For every other name query the strip
    /// variant applies. `None` (Java null) on a miss.
    fn padstack_no(&self, name: &str) -> Option<i32>;

    /// Java `Padstacks.get(name)` with the T32 query strip applied:
    /// the query name has every `.<digits>` run removed (Java
    /// `replaceAll("\\.\\d+", "")` — anywhere in the name, not only
    /// suffixes), then the registry is scanned in insertion order for the
    /// first CASE-INSENSITIVE match of the stripped name. Parse-side
    /// consumers: via placement (`Wiring.java:659-660`), net-class via
    /// instantiation (`Network.java:1292-1294`), package image pins
    /// (`Library.java:321-323`) and the library-scope dedup itself
    /// (`Library.java:158` on the `:113`-stripped name — jar
    /// `/tmp/epic-t4b-review.out`: a `(padstack viapad_f.1 ...)` scope is
    /// NOT added beside `ViaPad_F`; `PADSTACK_COUNT 2`). `None` (Java
    /// null) on a miss — the CALLER owns the missing-padstack warning +
    /// drop (`Wiring.java:661-671`).
    fn resolve_padstack_query(&self, name: &str) -> Option<i32>;

    /// Java `Packages.add` (`core/library/Packages.java:69-94`): ALWAYS
    /// appends to the insertion-ordered package table (no dedup IN the
    /// table — the get-then-add dedup is the library-scope reader's job,
    /// `Library.java:409-443`) and returns the 1-based package id
    /// (`packages.size() + 1`).
    fn insert_package(&mut self, image: ImageIr) -> i32;

    /// Java `Packages.get(name, isFront)` (`Packages.java:27-52`),
    /// verbatim: (1) scan in insertion order for the first package whose
    /// name equals the query CASE-INSENSITIVELY and whose side matches —
    /// wrong-side matches are remembered (`otherSidePackage`, last one
    /// wins); (2) unless the name is already suffix-free, retry both
    /// rules against the name with its trailing `::N` suffix stripped
    /// (`replaceAll("::\\d+$", "")`, `:40` — anchored at the END, unlike
    /// the T32 `.N` strip); (3) return the remembered other-side package
    /// (`:51`), else `None` (Java null). The `::N`-strip inner lookup is
    /// what makes the reader dedup insert `PAD::1` beside `PAD`: the
    /// inner get returns the base package, whose stored name is not equal
    /// to the queried `PAD::1` (jar `/tmp/epic-t4c-images.out`,
    /// PACKAGE_COUNT 2).
    fn package_no(&self, name: &str, is_front: bool) -> Option<i32>;

    // ---- rules ----------------------------------------------------------

    /// Java `insertNetClass` (`Network.java:436-543`, Task 8).
    fn insert_net_class(&mut self, net_class: NetClassIr);

    /// Java via-info insertion (net-class via rules, Task 8).
    fn append_via_info(&mut self, via_info: ViaInfoIr);

    /// Java `BoardLibrary.setViaPadstacks` (`core/library/BoardLibrary.java:81-84`):
    /// REPLACES the whole via-padstack list (the routing-eligible subset
    /// of the padstack registry, `BoardLibrary.java:24`). This list is a
    /// SEPARATE index from the padstack registry: `SesWriter`'s library
    /// section iterates THE LIST (`library_out` via padstacks), not the
    /// registry. Two construction paths feed it (both reader-side):
    ///
    /// 1. Network tail REPLACE (`Network.java:1273-1313`, after the
    ///    network scope closes): names come from the structure `(via
    ///    <names>)` scope (`Structure.java:979-980`) plus every net
    ///    class's `(circuit (use_via ...))` scopes (`NetClass.java:52-53`
    ///    via `Circuit.java:107`, gathered at `Network.java:1274-1280`).
    ///    Each name is `.N`-cleaned (`replaceAll("\\.\\d+", "")`,
    ///    `:1292-1294`) and resolved case-insensitively against the
    ///    registry; misses are warned and DROPPED (the array shrinks,
    ///    `:1307-1312`), then the survivors REPLACE the list (`:1313`).
    ///    The block runs whenever at least one net class OR a structure
    ///    via list was seen (`viaPadstackNames != null`, `:1286`) — a
    ///    class with no use_via therefore WIPES the list (jar
    ///    `/tmp/epic-t4c-images.out`: class vias written as bare `(via
    ///    ...)` are skipped, `useVia` stays empty, VIAPADSTACK_COUNT 0).
    /// 2. Via-info appends DURING the network scope (`Network.java:272-284`,
    ///    each `(via <name> <padstack> <class> [(attach)])` scope,
    ///    `:1240-1242`): case-sensitive list probe first
    ///    ([`BoardSink::via_padstack_no`]), then RAW case-insensitive
    ///    registry query with NO `.N` clean (`:275`); a hit is appended
    ///    through `addViaPadstack` ([`BoardSink::append_via_padstack`]),
    ///    a miss is warned and the via rule dropped (`:276-282`). Parse
    ///    order note: these appends run BEFORE path 1's tail replace, so
    ///    the tail replace clobbers them whenever it fires — Task 7's
    ///    reader mirrors that order.
    ///
    /// Entries are registry padstack numbers (1-based); order is
    /// significant (0-based via indexes, `BoardLibrary.java:45-50`).
    fn set_via_padstacks(&mut self, padstack_nos: Vec<i32>);

    /// Java `BoardLibrary.getViaPadstack(String)` (`:53-62`): scan the
    /// via-padstack list in order for the first entry whose REGISTRY
    /// padstack name equals the query — case-SENSITIVE `String.equals`,
    /// unlike every registry query. `None` (Java null) on a miss.
    fn via_padstack_no(&self, name: &str) -> Option<i32>;

    /// Java `BoardLibrary.addViaPadstack` (`:90-101`, driven by the
    /// via-info reader, `Network.java:283`): no-op when the list already
    /// holds an entry whose registry name equals the candidate's name
    /// (case-SENSITIVE `String.equals`, `:91`); otherwise APPENDS the
    /// padstack number. Dedup by name, not by number: two registry names
    /// never collide (registry dedup), so this matches Java 1:1.
    fn append_via_padstack(&mut self, padstack_no: i32);

    /// Structure `(flip_style ...)` raw value (Task 6).
    fn set_flip_style(&mut self, flip_style: String);

    /// Java `snapAngle` from the structure/parser scopes (Task 6/9).
    fn set_snap_angle(&mut self, snap_angle: AngleRestriction);

    // ---- placement + network -------------------------------------------

    /// Placement-scope insertion (Task 7): appends to the placement table
    /// in file order; ids are the 1-based positions.
    fn append_placement(&mut self, placement: PlacementIr);

    /// Java `components.add` (`Network.java:957-966`, driven by
    /// `insertComponents` in parse order `:832-837`): appends and returns
    /// the 1-based component id.
    fn insert_component(&mut self, component: ComponentIr) -> i32;

    /// Java `insertPin` (`Network.java:1035`): appends the pin item and
    /// returns its board id.
    fn insert_pin(&mut self, pin: PinIr) -> i32;

    /// Java component-outline insertion (`Network.java:1192`, when a
    /// package `outline` has more than one shape): appends the item and
    /// returns its board id (the id is consumed even though the digest
    /// emits no line for component outlines).
    fn insert_component_outline(&mut self, outline: ComponentOutlineIr) -> i32;

    // ---- obstacles (structure + wiring) ---------------------------------

    /// Java keepout insertion (structure keepouts + outline holes
    /// `Structure.java:862-876`/`:1279-1283`, package keepouts
    /// `Network.java:1039-1124`): appends the item and returns its board
    /// id.
    fn insert_keepout(&mut self, keepout: KeepoutIr) -> i32;

    /// Java conduction-area insertion (plane scopes `Structure.java:
    /// 1068-1122`, rectangle wires `Wiring.java:483-486`): appends the
    /// item and returns its board id.
    fn insert_conduction_area(&mut self, area: ConductionAreaIr) -> i32;

    /// Java `insertTraceWithoutCleaning` (`Wiring.java:534`, `:564`):
    /// appends the trace item and returns its board id. NO normalization,
    /// NO endpoint cleaning (D11).
    fn insert_trace(&mut self, trace: TraceIr) -> i32;

    /// Java `insertVia` (`Wiring.java:706`): appends the via item and
    /// returns its board id (duplicate-via detection is a READER rule,
    /// `Wiring.java:699-703`).
    fn insert_via(&mut self, via: ViaIr) -> i32;

    // ---- diagnostics + metadata -----------------------------------------

    /// Appends a parity warning (surface D12: digit-normalized in the
    /// digest; exact strings live on the sink).
    fn push_warning(&mut self, warning: String);

    /// Snapshot of the parse metadata (unit/resolution/quote/snap angle
    /// plus the host fields); called by the read_board assembly (Task 9).
    fn set_metadata(&mut self, metadata: MetadataIr);
}
