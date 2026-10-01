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
use epic_geometry::polyline::Polyline;

/// Java `FixedState` (`board/model/items/FixedState.java`): enum order
/// UNFIXED, SHOVE_FIXED, USER_FIXED, SYSTEM_FIXED (ordinal order is
/// load-bearing — module docs; the T37 plane heuristic compares
/// `ordinal() < USER_FIXED.ordinal()`, `DsnFile.java:100`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
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
    /// Java `Package.Keepout.layer` — 0-based layer number (taken from the
    /// FIRST shape's parser layer BEFORE the area transform, so it is set
    /// even when the transform fails).
    pub layer_no: i32,
    /// Java `Package.Keepout.area` (board coordinates; windows ride in
    /// [`AreaIr`]). `None` (Java null) when `Shape.transformAreaToBoardRel`
    /// returned null (empty list, non-`PolylineShape` border with holes, or
    /// a null boundary transform): the keepout is STILL STORED with a null
    /// area and the parse SURVIVES (`Library.java:382-390`); the null
    /// surfaces board-side at Task 8 insertion. The `Threw` flavor (null
    /// WINDOW entry) is parse-fatal and never reaches the sink.
    pub area: Option<AreaIr>,
}

/// One outline shape of a library image. Java `Package` stores three
/// PARALLEL arrays (`outline: Shape[]`, `outlineWidths: double[]`,
/// `outlineIsClosed: boolean[]`, all read from the `(outline ...)`
/// scope); the struct is one aligned slice of those arrays.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageOutlineIr {
    /// `Package.outline[i]` — board coordinates relative to the image
    /// origin. `None` (Java null array slot) when the DSN shape's
    /// `transformToBoardRel` returned null (the `PolylinePath` stub) —
    /// the `width`/`is_closed` fields are STILL SET in that case
    /// (`Library.java:350-366`: the transform result and the
    /// width/isClosed writes are independent; jar
    /// `/tmp/epic-t7-probe.out` t7-image `OUTLINE 3 null w=10.0
    /// closed=false`).
    pub shape: Option<BoardShape>,
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
    /// Java `Net.containsPlane` (`rules/Net.java:31`): true for nets
    /// created by the plane-scope and missing-power-plane insertions
    /// (`Nets.add(..., containsPlane=true)`, `Structure.java:1073`/
    /// `:553`) and for nets the T37 plane heuristic promotes
    /// (`DsnFile.adjustPlaneAutorouteSettings`, `:87-89`). Network-scope
    /// nets (Task 8) insert with `false`.
    pub contains_plane: bool,
    /// Java `Net.netClass` — the 0-based index into the net-class table
    /// (`Net` ctor takes `rules.getDefaultNetClass()`, `rules/Net.java:22`
    /// — the eagerly materialized class 0, see [`NetClassIr`]). Mutated by
    /// the class membership lists and the wiring-scope `(net ...)` rules
    /// (Task 8/9).
    pub net_class: i32,
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
/// `components.add(name, location, rotation, isFront, frontPackage,
/// backPackage, positionFixed, partNumber)`): id = `components.size() + 1`.
/// Both package numbers are resolved by the READER (`insertComponent`
/// requires `packages.get(key, true)` AND `get(key, false)` non-null,
/// `Network.java:936-943`); the location keeps Java's Option shape — a
/// null coor still ADDS the component (unplaced) and only skips the board
/// insertion half (`:969-971`).
#[derive(Clone, Debug, PartialEq)]
pub struct ComponentIr {
    /// Java `Component.name` (the placed instance name, e.g. `U1`).
    pub name: String,
    /// Java `Component.packageNo(true)` — the front-side package id
    /// (1-based; `Packages.get(key, true)`).
    pub package_front: i32,
    /// Java `Component.packageNo(false)` — the back-side package id.
    pub package_back: i32,
    /// Java `Component.location` — board coordinates (reader transforms
    /// `dsnToBoard(coor).round()` per axis, `Network.java:945-950`);
    /// `None` (Java null Point) for an unplaced component.
    pub location: Option<IntPoint>,
    /// Java `Component.rotation` — the NORMALIZED rotation in degrees
    /// (the `Component` ctor normalizes with while loops: `>= 360`
    /// subtracts, `< 0` adds — `Component.java:54-79`, NOT a `%` op:
    /// -45.5 -> 314.5, 720.5 -> 0.5).
    pub rotation: f64,
    /// Java `Component.isFront`.
    pub is_front: bool,
    /// Java `Component.fixedState` (position -> SYSTEM_FIXED else
    /// UNFIXED, `Network.java:974-976`).
    pub fixed: FixedStateIr,
    /// Java `Component.partNumber` (`(pn ...)` of the place scope).
    pub part_number: Option<String>,
    /// Java `Component.logicalPart` — set later by `insertLogicalParts`
    /// (`Network.java:895`); `None` until then (kept on the IR so the SES
    /// writer / digest see the final state).
    pub logical_part: Option<String>,
}

/// Java `rules.NetClass` parse subset, field-for-field (`rules/NetClass.
/// java`): identity is the 0-based POSITION in the net-class table —
/// class 0 is the default class, eagerly materialized by `create_board`
/// (Java's laziness in `BoardRules.getDefaultNetClass` is
/// observationally equivalent because the structure reader runs
/// `updateBoardRules` before the callback; with eager materialization the
/// empty-table AIOOBE edge of `appendNetClass` is unreachable).
///
/// Structure-era snapshot semantics:
/// [`BoardRulesIr::default_item_clearance_classes`] /
/// [`BoardRulesIr::default_trace_half_widths`] are written by the
/// STRUCTURE rule readers and consumed by `create_board` (which copies
/// them into class 0) and the structure keepout/plane reads
/// (`default_item_clearance_class`). They are NOT kept in sync
/// afterwards: later mutations of class 0's
/// [`NetClassIr::default_item_clearance_classes`] /
/// [`NetClassIr::trace_clearance_class`] /
/// [`NetClassIr::trace_half_widths`] (network/wiring scope) do NOT
/// mirror into [`BoardRulesIr`]. Java mutates one shared object there,
/// but nothing re-reads those values through the rules surface after
/// the structure scope, so the snapshot is observationally exact for
/// the parse (see [`BoardSink::board_rules_mut`]).
#[derive(Clone, Debug, PartialEq)]
pub struct NetClassIr {
    /// Java `NetClass.name`.
    pub name: String,
    /// Java `NetClass.traceClearanceClass` (`rules/NetClass.java:26`):
    /// ctor 0; `createDefaultNetClass` sets 1; `appendNetClass` copies the
    /// default's; `getNewNetClass` copies the default's.
    pub trace_clearance_class: i32,
    /// Java `NetClass.traceHalfWidthArr` (`new int[layerCount]`, ctor
    /// zeros; default class gets 1500 on every layer via the ALL-LAYERS
    /// `setTraceHalfWidth(int)` fill, `BoardRules.java:206-208`;
    /// `appendNetClass`/`getNewNetClass` fill EVERY layer with the
    /// default's `traceHalfWidth(0)`, `BoardRules.java:148-158`/`:236`).
    pub trace_half_widths: Vec<i32>,
    /// Java `NetClass.activeRoutingLayerArr` — ctor-initializes EVERY
    /// layer to `layer.isSignal` (`rules/NetClass.java:34-42`);
    /// `createActiveTraceLayers` flips entries (`Network.java:711-728`).
    /// Indexed by 0-based layer.
    pub active_routing_layers: Vec<bool>,
    /// Java `NetClass.defaultItemClearanceClasses` — a FRESH
    /// `DefaultItemClearanceClasses` per class (`[0,1,1,1,1,1]`; the
    /// Java-true value incl. the never-read NONE slot 0), CLONED from the
    /// default class by `appendNetClass` (`BoardRules.java:234-235`),
    /// FRESH (not cloned) by `getNewNetClass` (`:212-222`).
    pub default_item_clearance_classes: [i32; 6],
    /// Java `NetClass.viaRule` (null in the ctor; set by the via-rule
    /// passes) — the identity id of [`ViaRuleIr`], `None` = Java null.
    pub via_rule: Option<u32>,
    /// Java `NetClass.pullTight` (ctor true).
    pub pull_tight: bool,
    /// Java `NetClass.shoveFixed` (ctor false).
    pub shove_fixed: bool,
    /// Java `NetClass.minTraceLength` (ctor 0.0; `(circuit (length ...))`).
    pub min_trace_length: f64,
    /// Java `NetClass.maxTraceLength` (ctor 0.0).
    pub max_trace_length: f64,
    /// Java `NetClass.items` — ctor empty and it STAYS EMPTY at parse
    /// time: `Net.setClass` only assigns the net's back-pointer
    /// (`rules/Net.java`); nothing appends to the class-side list during
    /// the DSN read (an earlier "setClass appends" note here was wrong —
    /// the membership is derivable from [`NetIr::net_class`]).
    pub nets: Vec<i32>,
}

/// Java `ViaInfo` (`rules/ViaInfo.java` fields: name, padstack,
/// clearanceClassIndex, attachSmdAllowed) — one `(via ...)` rule of a net
/// class (Task 8, `Network.java:255-322`): the padstack is resolved by
/// the case-SENSITIVE via-list probe first, falling back to the RAW-name
/// case-insensitive registry query — NO `.N` strip on either path
/// (`Network.java:275`; the strip happens only on the network TAIL
/// replace, `:1292-1294`).
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

/// Java `ViaRule` (`rules/ViaRule.java`): a named ordered list of via
/// infos. Identity is the `id`, NOT a table position — Java stores
/// object references in `BoardRules.viaRules` and on every net class;
/// `addViaRule` REMOVES the same-named rule and re-adds, which under a
/// position-identity scheme would shift every later index and silently
/// re-point the net classes that held them. The sink assigns ids
/// monotonically via [`BoardSink::append_via_rule`].
#[derive(Clone, Debug, PartialEq)]
pub struct ViaRuleIr {
    /// The identity (sink-assigned, monotonic from 1).
    pub id: u32,
    /// Java `ViaRule.name`.
    pub name: String,
    /// Java `ViaRule.viaInfos` — the 0-based indexes into the via-info
    /// table in rule order (Java holds the ViaInfo objects; that table is
    /// append-only, so indexes are stable).
    pub via_infos: Vec<i32>,
}

/// Java `LogicalPart.PartPin` board-side parse subset (`core/library/
/// LogicalPart.java` fields `pinIndex`, `pinName`, `gateName`,
/// `gateSwapCode`, `gatePinName`, `gatePinSwapCode`): the parsed
/// [`crate::state::PartPin`] with the package pin INDEX resolved (the
/// case-sensitive `Package.getPinIndex` lookup, `Network.java:855`).
#[derive(Clone, Debug, PartialEq)]
pub struct LogicalPartPinIr {
    /// Java `PartPin.pinIndex`.
    pub pin_index: i32,
    /// Java `PartPin.pinName`.
    pub pin_name: String,
    /// Java `PartPin.gateName`.
    pub gate_name: String,
    /// Java `PartPin.gateSwapCode`.
    pub gate_swap_code: i32,
    /// Java `PartPin.gatePinName`.
    pub gate_pin_name: String,
    /// Java `PartPin.gatePinSwapCode`.
    pub gate_pin_swap_code: i32,
}

/// Java `LogicalPart` board-side (`core/library/LogicalPart.java`): the
/// logical part as stored by `LogicalParts.add` — the pin array SORTED by
/// pin index (`Arrays.sort` before construction). The SES writer and the
/// board inventory walk this shape (the parse-state
/// [`crate::state::LogicalPart`] keeps FILE order).
#[derive(Clone, Debug, PartialEq)]
pub struct LogicalPartIr {
    /// Java `LogicalPart.name`.
    pub name: String,
    /// The sorted part pins.
    pub pins: Vec<LogicalPartPinIr>,
}

/// Java `Pin` parse subset (`Network.java:1035` `insertPin(newComponent.id,
/// i, netNumberArray, clearanceClass, fixedState)`): a component pin; the
/// center is DERIVED, not stored (`Pin` ctor takes the padstack and the
/// component location; the placement lands in the sink's item list via
/// [`ItemIr::Pin`] with no digest line).
#[derive(Clone, Debug, PartialEq)]
pub struct PinIr {
    /// Java `Pin.component_no`.
    pub component_id: i32,
    /// Java `Pin.pin_index` (0-based position in the image pin list).
    pub pin_index: i32,
    /// The resolved padstack number (1-based).
    pub padstack_no: i32,
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
    /// The corner sequence (board coordinates, file order) — the ROUNDED
    /// view of [`TraceIr::polyline`]'s corners (`corner_to_int`). The
    /// harness digest twin still reads this field; the SES writer does
    /// NOT since T15/T40 (`write_wire` reads the verbatim
    /// [`TraceIr::polyline`] corners instead). Kept alongside the
    /// polyline so the M1b byte contracts read the exact field they
    /// always have.
    pub corners: Vec<IntPoint>,
    /// The polyline the reader constructed the JAVA way — `Polyline(Polygon)`
    /// for the `polygon` wire branch (`Wiring.java:531`) and
    /// `Polyline(Line[])` (parallel-filtered, T10) for the `polyline_path`
    /// branch (`Wiring.java:562`) — carried VERBATIM because the live
    /// board must store exactly Java's parse-time polyline. T13: this is
    /// NOT re-derivable from `corners` — `Polyline::from_points` would run
    /// the `Polygon` dedup, which the `PolylinePath` branch never does, so
    /// a duplicated trailing corner (dsn-0061/0063: Java's board keeps
    /// `...C C`) would be dropped and every downstream corner view would
    /// diverge from the oracle.
    pub polyline: Polyline,
    /// Java `Trace.netNumbers` (the wire's single net).
    pub nets: Vec<i32>,
    /// Java clearance class index.
    pub clearance_class: i32,
    /// T36: `(type route)` -> USER_FIXED, `(type fix)` -> SYSTEM_FIXED.
    pub fixed: FixedStateIr,
}

impl TraceIr {
    /// Builds the polyline for a hand-built corner list — `Polyline::
    /// from_points` (Java `Polyline(Point[])` over the `Polygon`
    /// normalization). FOR TEST LITERALS AND HAND-BUILT IRs ONLY: the
    /// wire read path (`scope/wiring.rs::insert_trace`) must carry the
    /// reader's Java-constructed polyline instead — this helper applies
    /// the `Polygon` dedup, which the `PolylinePath` read branch never
    /// does (the [`TraceIr::polyline`] field docs).
    pub fn polyline_of_corners(corners: &[IntPoint]) -> Polyline {
        Polyline::from_points(
            &corners
                .iter()
                .map(|&corner| epic_geometry::point::Point::Int(corner))
                .collect::<Vec<_>>(),
        )
    }
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
/// nets (the digest `K` line therefore has no net list). The component
/// fields are the package-keepout context (`insertComponent` passes
/// translation/rotation/side/component id/name); structure keepouts pass
/// the identity defaults.
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
    /// Structure keepouts are SYSTEM_FIXED (spike item 2); package
    /// keepouts take the component's fixed state.
    pub fixed: FixedStateIr,
    /// Java `ObstacleArea.component_no` — 0 for structure keepouts (Java
    /// passes component 0, `Structure.java:862-876`).
    pub component_id: i32,
    /// Java `ObstacleArea.translation` — `Point.ZERO` for structure
    /// keepouts.
    pub translation: IntPoint,
    /// Java `ObstacleArea.rotationInDegree` — 0.0 for structure
    /// keepouts. The RAW placement-scope rotation (`location.rotation`,
    /// `Network.java:955`), NOT the component's normalized copy: Java's
    /// `Component` ctor wraps ITS rotation into [0,360)
    /// (`board/model/structure/Component.java` ctor) but the obstacle
    /// keeps the unwrapped value, and [`crate::ses_board::SesBoard::
    /// obstacle_absolute_area`] must consume THIS raw field — feeding a
    /// [0,360)-normalized rotation instead diverges from
    /// `ObstacleArea.getArea()` on placements whose rotation falls
    /// outside [0,360) (`(int) rotation / 90` truncation and
    /// `rotateApprox(toRadians(rotation))` argument rounding both see
    /// the raw value, `ObstacleArea.java:130-140`). The reader wires it
    /// raw (`scope/network.rs` passes `rotation_in_degree`, the
    /// un-normalized local, unlike the normalized `ComponentIr` copy).
    pub rotation: f64,
    /// Java `ObstacleArea.side_changed` — false except back-side package
    /// keepouts (`!isFront`, `Network.java:1110`).
    pub side_changed: bool,
    /// Java `ObstacleArea.name` — `Some` only for package keepouts whose
    /// image keepout carried a name (the placement info map key).
    pub name: Option<String>,
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
/// `area: None` (Java null — a PolylinePath-stub outline slot or an
/// untransformed shape) mirrors `BasicBoard.insertComponentOutline`'s
/// null guard: NO item, NO id burned (the id is consumed only when the
/// outline actually inserts).
#[derive(Clone, Debug, PartialEq)]
pub struct ComponentOutlineIr {
    /// Java `ComponentOutline.component_no`.
    pub component_id: i32,
    /// 0-based layer number.
    pub layer_no: i32,
    /// The border + window holes; `None` = the BasicBoard null guard
    /// dropped the insertion.
    pub area: Option<AreaIr>,
    /// Java `ComponentOutline.netNumbers` (empty at parse time).
    pub nets: Vec<i32>,
    /// Java clearance class index.
    pub clearance_class: i32,
    /// Java fixed state.
    pub fixed: FixedStateIr,
    /// Java `ComponentOutline.isFront` — false for back-side components
    /// (the shape is mirrored at geometry level, Task 9+).
    pub is_front: bool,
    /// Java `ComponentOutline.translation` — the component location
    /// minus the origin (`Point.ZERO` translated, `Network.java:1158`).
    /// Unplaced components never reach the outline loop (`insertComponent`
    /// bails at `Network.java:969-971` before pins/keepouts/outlines).
    pub translation: IntPoint,
    /// Java `ComponentOutline.rotation` — the RAW placement rotation
    /// (`insertComponentOutline` passes `rotationInDegree` UN-normalized,
    /// `Network.java:1196`; the `Component` object normalizes its own
    /// copy, the outline item keeps the raw value and mirrors it about
    /// ZERO only for back-side components at geometry time, Task 9+).
    pub rotation: f64,
    /// Java `ComponentOutline.isCourtyard` — the largest-area outline of
    /// the package, or every outline when the package has no widths
    /// (`Network.java:1164-1186`).
    pub is_courtyard: bool,
    /// Java `ComponentOutline.isFabrication` — `!courtyard && width <=
    /// 110.0` (`:1176-1180`).
    pub is_fabrication: bool,
    /// Java `ComponentOutline.isClosed` — the `outlineIsClosed[i]` flag,
    /// FALSE beyond the array (`:1187-1190`).
    pub is_closed: bool,
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
    /// Java `outlineClearanceNo` — resolved by the SINK at `create_board`
    /// from `outlineClearanceClassName` (`MinimalBoardManager.createBoard`,
    /// `ReadScopeParameter.java:139-166`; the three-way resolution is
    /// pinned by the Task 5 boundary pins).
    pub clearance_class: i32,
    /// SYSTEM_FIXED (spike).
    pub fixed: FixedStateIr,
}

/// Java `String.equalsIgnoreCase` over the ASCII identifier space (the ONE
/// shared helper: `ClearanceMatrix.getNo`, the board-side registry lookups
/// in `ses_board` (`rules/Nets.java:39,51`, `Padstacks.get`) and the
/// structure-reader `smd_to_turn_gap` check): both sides lowercased and
/// compared. A full UTF-16 per-char port lands only if a fixture ever
/// diverges.
pub(crate) fn eq_ignore_case(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// Java `ClearanceMatrix` parse subset (`rules/ClearanceMatrix.java`):
/// class names in matrix order and the per-layer clearance values.
///
/// Storage is `values[layer][j][i]` — Java stores `row[j].column[i]
/// .layer[layer]`, i.e. the outer index is the layer, the middle index is
/// the SECOND class argument `j` (the row) and the inner index the FIRST
/// class argument `i` (the column); `get_value` mirrors that order
/// (`ClearanceMatrix.java:107`, `:126-160`).
#[derive(Clone, Debug, PartialEq)]
pub struct ClearanceIr {
    /// Java `ClearanceMatrix` class names, matrix column/row order.
    pub names: Vec<String>,
    /// `values[layer][j][i]`: clearance between classes `i` and `j` on
    /// `layer` (board units, always even). Outer = layer, middle = `j`.
    pub values: Vec<Vec<Vec<i32>>>,
}

impl ClearanceIr {
    /// The pre-`create_board` empty instance ([`SesBoard::new`] default).
    pub fn empty() -> Self {
        Self {
            names: Vec::new(),
            values: Vec::new(),
        }
    }

    /// Java `ClearanceMatrix.getDefaultInstance(layerStructure, 0)`
    /// (`Structure.java:1236`): the two built-in classes `"null"` and
    /// `"default"`, zero matrix, one slot per layer.
    pub fn default_instance(layer_count: usize) -> Self {
        Self {
            names: vec!["null".to_string(), "default".to_string()],
            values: vec![vec![vec![0, 0], vec![0, 0]]; layer_count],
        }
    }

    /// Java `getClassCount()`.
    pub fn class_count(&self) -> i32 {
        self.names.len() as i32
    }

    /// Java `getNo(name)` (`ClearanceMatrix.java:60-68`): first
    /// case-INSENSITIVE name match, `-1` on a miss.
    pub fn get_no(&self, name: &str) -> i32 {
        self.names
            .iter()
            .position(|existing| eq_ignore_case(existing, name))
            .map(|index| index as i32)
            .unwrap_or(-1)
    }

    /// Java `getValue(i, j, layer, addSafetyMargin=false)`
    /// (`ClearanceMatrix.java:126-160`): out-of-bounds requests log a
    /// trace and return 0; the safety margin variant is never used at
    /// parse time (margin 0 would make it an identity anyway).
    pub fn get_value(&self, class_i: i32, class_j: i32, layer: i32) -> i32 {
        let Ok(layer_index) = usize::try_from(layer) else {
            return 0;
        };
        let Some(per_layer) = self.values.get(layer_index) else {
            return 0;
        };
        let (Ok(i), Ok(j)) = (usize::try_from(class_i), usize::try_from(class_j)) else {
            return 0;
        };
        let Some(row) = per_layer.get(j) else {
            return 0;
        };
        row.get(i).copied().unwrap_or(0)
    }

    /// Java `setValue(classI, classJ, layer, value)` (`:107-124`): the
    /// value is clamped to `max(value, 0)` and rounded UP to even
    /// (`Integer.MAX_VALUE` rounds DOWN — unreachable at parse time), then
    /// stored. The row/layer max bookkeeping is not parse-observable.
    pub fn set_value(&mut self, class_i: i32, class_j: i32, layer: i32, value: i32) {
        let mut value = value.max(0);
        if value % 2 != 0 {
            if value == i32::MAX {
                value -= 1;
            } else {
                value += 1;
            }
        }
        let Some(per_layer) = self
            .values
            .get_mut(usize::try_from(layer).unwrap_or(usize::MAX))
        else {
            return;
        };
        let (Ok(i), Ok(j)) = (usize::try_from(class_i), usize::try_from(class_j)) else {
            return;
        };
        if let Some(row) = per_layer.get_mut(j)
            && let Some(entry) = row.get_mut(i)
        {
            *entry = value;
        }
    }

    /// Java `setValue(classI, classJ, value)` (`:102-105`): same entry on
    /// every layer.
    pub fn set_value_all_layers(&mut self, class_i: i32, class_j: i32, value: i32) {
        for layer in 0..self.values.len() as i32 {
            self.set_value(class_i, class_j, layer, value);
        }
    }

    /// Java `setDefaultValue(layer, value)` (`:90-94`): sets every entry
    /// with class number >= 1 on the given layer (the `null` row/column 0
    /// stays 0).
    pub fn set_default_value(&mut self, layer: i32, value: i32) {
        for i in 1..self.class_count() {
            for j in 1..self.class_count() {
                self.set_value(i, j, layer, value);
            }
        }
    }

    /// Java `setDefaultValue(value)` (`:84-88`): every layer.
    pub fn set_default_value_all_layers(&mut self, value: i32) {
        for layer in 0..self.values.len() as i32 {
            self.set_default_value(layer, value);
        }
    }

    /// Java `appendClass(name)` (`:283-325`): case-insensitive dedup
    /// (`getNo(name) >= 0` -> `false`, no change); otherwise the class
    /// count grows by one and EVERY new entry on EVERY layer is
    /// initialized from class 1 (`"default"`): `(new, i)` and `(i, new)`
    /// from `getValue(1, i, layer)`, the diagonal `(new, new)` from
    /// `getValue(1, 1, layer)` (jar `/tmp/epic-t5-probe.out`: the appended
    /// `power`/`ground` rows read 5000 — the default value set earlier by
    /// `(clearance 500)`).
    pub fn append_class(&mut self, name: &str) -> bool {
        if self.get_no(name) >= 0 {
            return false;
        }
        let old_class_count = self.names.len();
        let layer_count = self.values.len();
        let new_class_count = old_class_count + 1;
        self.names.push(name.to_string());
        for per_layer in &mut self.values {
            // grow every existing row and add the new row (zeros first,
            // then the class-1 copy below matches the Java two-phase init)
            for row in per_layer.iter_mut() {
                row.resize(new_class_count, 0);
            }
            per_layer.resize(new_class_count, vec![0; new_class_count]);
        }
        let new_index = old_class_count as i32;
        for i in 0..old_class_count {
            for layer in 0..layer_count as i32 {
                let default_value = self.get_value(1, i as i32, layer);
                self.set_value(new_index, i as i32, layer, default_value);
                self.set_value(i as i32, new_index, layer, default_value);
            }
        }
        for layer in 0..layer_count as i32 {
            let default_value = self.get_value(1, 1, layer);
            self.set_value(new_index, new_index, layer, default_value);
        }
        true
    }
}

/// Java `BoardRules` parse subset (`rules/BoardRules.java:52-63` ctor +
/// the lazy `createDefaultNetClass` (`:206-213`) flattened: the parser
/// touches the default net class only through the fields below, so its
/// lazy materialization collapses into eager initialization).
#[derive(Clone, Debug, PartialEq)]
pub struct BoardRulesIr {
    /// Java `getDefaultNetClass().getTraceHalfWidth(layer)` — the default
    /// trace half width per layer; `createDefaultNetClass` fills 1500 on
    /// every layer (`BoardRules.java:206-208`, jar
    /// `/tmp/epic-t5-probe.out`: rule-less layers read DEFAULT_HW 1500).
    pub default_trace_half_widths: Vec<i32>,
    /// Java `minTraceHalfWidth` (`:37`, ctor 100000): monotonically
    /// lowered by the width setters; feeds the pin-edge fallback.
    pub min_trace_half_width: i32,
    /// Java `maxTraceHalfWidth` (`:38`, ctor 100): monotonically raised by
    /// the width setters.
    pub max_trace_half_width: i32,
    /// Java `pinEdgeToTurnDist` (`:46`, ctor 0.0); `updateBoardRules`
    /// always leaves this at `getMinTraceHalfWidth()` when no
    /// `smd_to_turn_gap` rule fired (`Structure.java:667-669` — jar:
    /// 100000.0 for rule-less boards).
    pub pin_edge_to_turn_dist: f64,
    /// Java `traceAngleRestriction` (`:31`, ctor FORTYFIVE_DEGREE);
    /// `createBoard` overwrites it from the parse state
    /// (`Structure.java:1267`).
    pub trace_angle_restriction: AngleRestriction,
    /// Java `getDefaultNetClass().defaultItemClearanceClasses` — indexed
    /// by [`ItemClassIr`] declaration order (NONE, TRACE, VIA, PIN, SMD,
    /// AREA), all 1 (`DefaultItemClearanceClasses` ctor; jar:
    /// OUTLINE_CLASS 1 for rule-less boards, 7 after the `wire` rule
    /// appends `area`).
    pub default_item_clearance_classes: [i32; 6],
    /// Java `clearanceMatrix` (final field).
    pub clearance: ClearanceIr,
}

impl BoardRulesIr {
    /// The `new BoardRules(layerStructure, ClearanceMatrix
    /// .getDefaultInstance(...))` + lazy default-net-class state as the
    /// structure reader sees it (`Structure.java:1236-1237`).
    pub fn new(layer_count: usize) -> Self {
        Self {
            default_trace_half_widths: vec![1500; layer_count],
            min_trace_half_width: 100_000,
            max_trace_half_width: 100,
            pin_edge_to_turn_dist: 0.0,
            trace_angle_restriction: AngleRestriction::FortyfiveDegree,
            default_item_clearance_classes: [1; 6],
            clearance: ClearanceIr::default_instance(layer_count),
        }
    }

    /// Java `setDefaultTraceHalfWidth(layer, value)` (`:122-127`): stores
    /// the width and lowers `minTraceHalfWidth` / raises
    /// `maxTraceHalfWidth`. An out-of-range layer is ALL-OR-NOTHING: the
    /// store AND the min/max bookkeeping are both skipped. (Java's
    /// `NetClass.setTraceHalfWidth` would throw
    /// `ArrayIndexOutOfBoundsException` there — unreachable at parse time
    /// because the only caller pre-filters with `getNo(layer) >= 0`
    /// (`Structure.java:641-660`); the guard is Rust-side hygiene, not an
    /// observable parity surface.)
    pub fn set_default_trace_half_width(&mut self, layer: i32, value: i32) {
        let Ok(layer_index) = usize::try_from(layer) else {
            return;
        };
        let Some(slot) = self.default_trace_half_widths.get_mut(layer_index) else {
            return;
        };
        *slot = value;
        self.min_trace_half_width = self.min_trace_half_width.min(value);
        self.max_trace_half_width = self.max_trace_half_width.max(value);
    }

    /// Java `setDefaultTraceHalfWidths(value)` (`:137-147`): `value <= 0`
    /// logs a warning and is a NO-OP (jar-pinned semantics; the warning is
    /// FRLogger-only, not a parity-warnings surface), otherwise the width
    /// lands on every layer with the min/max updates.
    pub fn set_default_trace_half_widths(&mut self, value: i32) {
        if value <= 0 {
            return;
        }
        for layer in 0..self.default_trace_half_widths.len() as i32 {
            self.set_default_trace_half_width(layer, value);
        }
    }

    /// Java `setPinEdgeToTurnDist(value)` (`:364-367`): plain assignment.
    pub fn set_pin_edge_to_turn_dist(&mut self, value: f64) {
        self.pin_edge_to_turn_dist = value;
    }

    /// Java `getMinTraceHalfWidth()` (`:94-96`).
    pub fn get_min_trace_half_width(&self) -> i32 {
        self.min_trace_half_width
    }
}

/// Parse-time metadata snapshot (the `Communication`/parser-info fields
/// the digest and the SES writer need). Defaults mirror the parse-state
/// defaults (T24: MIL / 100; string quote `"`; snap angle 45).
///
/// Hand-mirrors the 8 shared fields of [`crate::reader::BoardMetadataIr`]
/// (the two structs deliberately mirror two different Java types — this
/// one the parse-state snapshot, that one Java `BoardMetadata`); when
/// adding a field here, mirror it in `BoardMetadataIr` AND add a
/// `diff_field` call in `rust/harness/tests/dsn_metadata_pin.rs`
/// (enforced by that file's self-test).
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
    /// Java `layerCount` at the `DsnReader.java:261-266` precedence (the
    /// parser `layerStructure` length, ELSE the created board's layer
    /// count, else 0). Java's readBoard `Success` carries metadata=null —
    /// this field exists for the D16 pin's read_board side (Task 10) and
    /// is computed by the same shared helper as `read_metadata`'s
    /// `BoardMetadataIr.layer_count`.
    pub layer_count: i32,
    /// Java `scopeParameter.autorouteSettings` — `None` when no
    /// `(autoroute_settings ...)` scope was read; the plane heuristic does
    /// not touch the parse state (Java's `adjustPlaneAutorouteSettings`
    /// mutates the BOARD). D16 read_board-side exposure (Task 10).
    pub autoroute_settings: Option<crate::scope::autoroute_settings::AutorouteSettingsIr>,
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
            layer_count: 0,
            autoroute_settings: None,
        }
    }
}

/// The `createBoard` argument IR (`BoardParserCallback.java:29-36` with
/// `BoardRules`/`Communication` reduced to their parse-derivable parts:
/// the rules arrive fully built in [`BoardRulesIr`] — the reader applies
/// `updateBoardRules` (`Structure.java:1266`) before the callback — and
/// the sink resolves the outline clearance class against it,
/// `ReadScopeParameter.java:139-166`).
#[derive(Clone, Debug, PartialEq)]
pub struct CreateBoardIr {
    /// Java `boundingBox` — the transformed boundary bounds INCLUDING the
    /// offset(1000) (T43, `Structure.java:1207-1208`).
    pub bounding_box: IntBox,
    /// Java `layerStructure`. Carries the PARSER layers verbatim —
    /// including any `(net ...)` names a `(layer ...)` scope declared:
    /// Java rebuilds the BOARD layers as `new Layer(name, isSignal)`
    /// from the parser list (`Structure.java:1180-1185`), dropping the
    /// net names; only `name`/`no`/`is_signal` are board-observable.
    pub layer_structure: LayerStructure,
    /// Java `outlineShapes` — the transformed boundary shapes (board
    /// coordinates; becomes the id-1 [`BoardOutlineIr`] item).
    pub outline_shapes: Vec<BoardShape>,
    /// Java `outlineClearanceClassName` — `None` when no
    /// `(clearance_class ...)` scope appeared in any boundary (the sink
    /// then falls back to the AREA item class, Issue558 context).
    pub outline_clearance_class: Option<String>,
    /// Java `BoardRules` after `updateBoardRules` + the trace-angle
    /// restriction (`Structure.java:1266-1267`).
    pub rules: BoardRulesIr,
    /// The finalized `CoordinateTransform` (T25 loop already applied).
    pub transform: CoordinateTransform,
}

/// The seam the scope readers emit through (module docs for the Java
/// mapping and the id-assignment contract).
pub trait BoardSink {
    /// Java `getRoutingBoard() == null` (`BoardParserCallback.java:23`,
    /// consumed at `Structure.java:1034`): true once `create_board`
    /// succeeded, false before. The structure reader builds the board only
    /// on the first `(structure ...)` scope; a second scope re-parses
    /// layers/rules but does NOT recreate (jar `/tmp/epic-t4c-twostruct.out`).
    fn has_board(&self) -> bool;

    /// Java `createBoard` (`BoardParserCallback.java:29-36`, called from
    /// `Structure.java:1268-1274`): constructs the board from the parsed
    /// structure scope. Implementations insert the id-1 `BoardOutline`
    /// item (`BasicBoard.java:136`) and resolve the outline clearance
    /// class against the incoming rules
    /// (`ReadScopeParameter.java:139-166`).
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

    /// Java `Nets.get(name, subnetNumber)` — the TWO-argument overload
    /// (`rules/Nets.java:42-51`): the FIRST table entry whose name equals
    /// the query case-INSENSITIVELY and whose subnet number equals
    /// exactly; `None` (Java null) on a miss. Consumed by the plane
    /// insertions (`Structure.java:1075`) and the missing-power-plane
    /// pass (`:549`) — the discriminator pin is `/tmp/epic-t6-board.out`
    /// t6-plane.dsn: after inserting board nets `GND` (1) and `gnd` (2),
    /// the `gnd` plane's `get("gnd", 1)` still resolves to net 1, so BOTH
    /// B.Cu conduction areas carry nets=[1].
    fn net_no_subnet(&self, name: &str, subnet_number: i32) -> Option<i32>;

    /// Java `Nets.get(String)` — the ONE-argument overload
    /// (`rules/Nets.java:54-62`): EVERY table entry whose name equals the
    /// query case-INSENSITIVELY, in table (number) order. Consumed by
    /// `Network.insertNetClass`'s net-list pass (`Network.java:472-477`,
    /// which setClasses ALL matching subnets — the T35 case-variant
    /// discriminator: class list `GND` reclasses both `GND` and `gnd`).
    fn net_nos(&self, name: &str) -> Vec<i32>;

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

    /// Java `Package.name` of the stored package with the given 1-based
    /// number (`board.library.packages.get(no)`). The library-scope dedup
    /// loop needs the STORED name (not the queried one) to tell a
    /// case-insensitive exact match from a `::N`-suffix/other-side hit —
    /// `existingPkg.name.equalsIgnoreCase(testName)` inside the
    /// `Library.java:412-449` dedup loop (`:417`). `None` for an
    /// out-of-range number (Java would throw; unreachable through the
    /// reader, which only queries numbers returned by
    /// [`BoardSink::package_no`]).
    fn package_name(&self, package_no: i32) -> Option<&str>;

    /// The stored pins of the package with the given 1-based number
    /// (`board.library.packages.get(no).pins`), in pin order. The
    /// library-scope dedup loop compares them against the freshly parsed
    /// pins (`arePackagePinsIdentical`, `Library.java:226-259`). An
    /// out-of-range number yields an empty slice (Java would NPE; the
    /// reader only queries numbers returned by
    /// [`BoardSink::package_no`], so the empty count then fails the
    /// comparison — the same outcome by a shorter route).
    fn package_pins(&self, package_no: i32) -> &[ImagePinIr];

    /// The stored package with the given 1-based number — the full
    /// [`ImageIr`] (pins, keepouts, outlines). `None` (Java null +
    /// FRLogger warning) out of range. Consumed by `insertComponent`'s
    /// keepout/outline expansion (`Network.java:1039-1204`), which needs
    /// more than the pin slice.
    fn package(&self, package_no: i32) -> Option<&ImageIr>;

    /// Java `components.get(name).getPackage()` (`Network.java:926`, the
    /// `searchLibPackage` tail): the first component whose name equals the
    /// query CASE-SENSITIVELY, mapped through `Component.getPackage`
    /// (`board/model/structure/Component.java:238-245`) = the FRONT package
    /// number when the component is on the front side, the BACK one
    /// otherwise. `None` (Java null -> the caller's log-only warning) on a
    /// component-name miss.
    fn component_package_no(&self, name: &str) -> Option<i32>;

    // ---- rules: net classes / via infos / via rules (Task 8) ------------

    /// Java `board.rules.netClasses` (`rules/NetClasses.java:22-27`, a
    /// `Vector<NetClass>`): class 0 is the default class (eagerly
    /// materialized by `create_board`, see [`NetClassIr`]). `None`-like
    /// emptiness before `create_board` mirrors Java's null-board NPE —
    /// the network reader treats an empty table as fatal.
    fn net_classes(&self) -> &[NetClassIr];

    /// Mutable SLICE view for the reader-side mutations (`setClass`,
    /// `setTraceHalfWidth`, item-class writes, ...): index assignments
    /// only — class APPENDS have their own sink methods
    /// ([`BoardSink::append_net_class`],
    /// [`BoardSink::append_generated_net_class`]), so readers never need
    /// to grow the table through this view.
    fn net_classes_mut(&mut self) -> &mut [NetClassIr];

    /// Mutable rules view. POST-`create_board` the only reader mutations
    /// through this view hit [`BoardRulesIr::clearance`] (the clearance
    /// matrix retargets of the network rules) — in Java that matrix is
    /// ONE shared object between `BoardRules` and every consumer, so
    /// mirroring the writes is exact. The DEFAULT-class mirrors
    /// ([`BoardRulesIr::default_trace_half_widths`],
    /// [`BoardRulesIr::default_item_clearance_classes`]) are a
    /// STRUCTURE-ERA SNAPSHOT consumed by `create_board` and the
    /// structure keepout/plane reads: they are NOT written through when
    /// the network scope mutates class 0 (Java mutates the shared
    /// `DefaultItemClearanceClasses`/width-array objects there, but
    /// nothing re-reads them through the rules surface after the
    /// structure scope, and the digest does not observe the mirror).
    fn board_rules_mut(&mut self) -> &mut BoardRulesIr;

    /// Java `BoardRules.appendNetClass(String)` (`BoardRules.java:225-239`):
    /// case-SENSITIVE `get(name)` hit returns it unchanged; otherwise
    /// append a class that CLONES the default's item classes, copies the
    /// default's `viaRule`, `traceClearanceClass` and `traceHalfWidth(0)`
    /// (ONLY layer 0 — the rest stay 0). Returns the 0-based class index.
    fn append_net_class(&mut self, name: &str) -> i32;

    /// Java `BoardRules.getNewNetClass()` (`:210-223`): append a class
    /// named `class<k>` (k from 1, advancing while the CASE-SENSITIVE
    /// `get` hits, `NetClasses.java:55-64`) with FRESH item classes (NOT
    /// cloned), the default's `traceClearanceClass`, the default via rule
    /// ([`BoardSink::default_via_rule_id`]) and the default's
    /// `traceHalfWidth(0)`. Returns the 0-based class index.
    fn append_generated_net_class(&mut self) -> i32;

    /// Java `board.rules.viaInfos` (`rules/ViaInfos.java`, a `Vector`).
    fn via_infos(&self) -> &[ViaInfoIr];

    /// Java `ViaInfos.get(String)` (`:43-50`): the 0-based index of the
    /// FIRST info whose name equals the query (CASE-SENSITIVE
    /// `String.equals`); `None` (Java null) on a miss.
    fn via_info_no(&self, name: &str) -> Option<i32>;

    /// Java `ViaInfos.add` (`:28-38`): silently DEDUPED — a no-op when an
    /// info with the same name (case-sensitive) already exists; appends
    /// otherwise. The boolean return is discarded at the call sites.
    fn append_via_info(&mut self, via_info: ViaInfoIr);

    /// Java `board.rules.viaRules` (`rules/ViaRules.java`, a `Vector`).
    fn via_rules(&self) -> &[ViaRuleIr];

    /// Java `viaRules.add(new ViaRule(name))` — appends and returns the
    /// identity id ([`ViaRuleIr::id`]). The member list arrives filled:
    /// all three Task 8 call sites (`addViaRule` after member resolution,
    /// `createViaRule`, `BoardRules.createDefaultViaRule`) append their
    /// members WITH the rule — the members are the 0-based via-info
    /// indexes in rule order.
    fn append_via_rule(&mut self, name: String, via_infos: Vec<i32>) -> u32;

    /// Java `ViaRules.get(String)` (`:21-28`): the id of the FIRST rule
    /// whose name equals the query (case-sensitive); `None` (Java null).
    fn via_rule_no(&self, name: &str) -> Option<u32>;

    /// Java `viaRules.remove(existing)` (Vector identity removal): drops
    /// the rule with the given id; a no-op when absent.
    fn remove_via_rule(&mut self, id: u32);

    /// Java `BoardRules.getDefaultViaRule()` (`:143-149`): the FIRST
    /// rule's id, `None` (Java null) when the table is empty.
    fn default_via_rule_id(&self) -> Option<u32>;

    /// Java `Padstacks.get(int)` (`core/library/Padstacks.java:15-22`):
    /// the 1-based registry entry; `None` (Java null + FRLogger warning)
    /// out of range. The read-only padstack data (shapes for the
    /// from/to-layer derivation, `attachAllowed` for the via-info
    /// defaults) drives `insertComponent`'s pin expansion and the
    /// default-via-infos pass.
    fn padstack(&self, padstack_no: i32) -> Option<&PadstackIr>;

    /// Java `board.rules.nets` (the net table in NUMBER order; net no =
    /// position + 1).
    fn nets(&self) -> &[NetIr];

    /// Mutable view for `Net.setClass` and the logical-part writes.
    fn nets_mut(&mut self) -> &mut Vec<NetIr>;

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

    /// The via-padstack list as registry numbers, in LIST order (the
    /// 0-based via indexes of `BoardLibrary.getViaPadstack(int)` ARE the
    /// positions). Read by the network scope: `(via ...)` scope reads
    /// resolve padstacks against it FIRST (`Network.java:272`), and the
    /// default-via-infos pass walks it (`:364-376`).
    fn via_padstacks(&self) -> &[i32];

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

    /// Java `LogicalParts.add(name, pins)` (`core/library/LogicalParts.java`):
    /// appends a board logical part; the pin array is SORTED by pin index
    /// before construction (`Arrays.sort`, `LogicalParts.add`). The
    /// parsed-order [`crate::state::LogicalPart`] entries convert to this
    /// board shape in `Network.insertLogicalParts` after the package pin
    /// indexes resolve.
    fn append_logical_part(&mut self, part: LogicalPartIr);

    /// Java `LogicalParts.get(String)` (`:33-38`): the canonical name of
    /// the FIRST part whose name equals the query case-INSENSITIVELY;
    /// `None` (Java null) on a miss.
    fn logical_part_name(&self, name: &str) -> Option<String>;

    /// Java `components.get(name).setLogicalPart(part)`
    /// (`Network.java:884-895`): the case-SENSITIVE first component name
    /// match gets the logical part name; a miss (component or part)
    /// warns log-only. LATER mappings OVERWRITE — including an overwrite
    /// to `None` (jar logical2: R1 mapped LPA then LPB ends `lp=LPB`).
    fn set_component_logical_part(&mut self, component_name: &str, logical_part: Option<String>);

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

    /// Java `insertTraceWithoutCleaning` (`Wiring.java:534`, `:564`; the
    /// guard body `BasicBoard.java:183-201`): appends the trace item and
    /// returns its board id. NO normalization, NO endpoint cleaning (D11).
    /// Two drop guards with DIFFERENT id semantics, both returning the 0
    /// "not inserted" sentinel: a sub-2-corner polyline returns null
    /// BEFORE the `Item` is constructed — no id burned; a CLOSED (first
    /// == last corner) trace fixed below USER_FIXED returns null AFTER
    /// construction — the `Item` constructor (`Item.java:86-89`) already
    /// consumed an id, so the id stays burned with no stored item.
    /// BoardSink is the M2 seam: an epic-board implementation MUST
    /// reproduce this asymmetry.
    fn insert_trace(&mut self, trace: TraceIr) -> i32;

    /// Java `insertVia` (`Wiring.java:706`): appends the via item and
    /// returns its board id (duplicate-via detection is a READER rule,
    /// `Wiring.java:699-703`).
    fn insert_via(&mut self, via: ViaIr) -> i32;

    /// Java `Wiring.viaExists` (`:238-255`), the duplicate-via probe that
    /// gates [`BoardSink::insert_via`]: true when a STORED via has the same
    /// center, exactly the queried net set (`Item.netsEqual(int[])`,
    /// `Item.java:1234-1244`: equal LENGTH plus every query net contained —
    /// set semantics, order-insensitive) and the same first/last layer span
    /// as the queried padstack (`Via.firstLayer`/`lastLayer` come from the
    /// via's OWN padstack, so the sink derives both spans from its
    /// registry). `from_layer` may be the shape-count (all-null padstack)
    /// and `to_layer` -1 (`Padstack.fromLayer`/`toLayer`); such a padstack
    /// only matches a stored via with the same degenerate span.
    fn via_exists(&self, location: IntPoint, from_layer: i32, to_layer: i32, nets: &[i32]) -> bool;

    // ---- board queries (Task 6 keepout/plane insertion) ------------------

    /// Java `board.getLayerCount()` (`Structure.java:871`): the BOARD layer
    /// count from `create_board`; 0 before the board exists.
    fn layer_count(&self) -> i32;

    /// Java `rules.getDefaultNetClass().defaultItemClearanceClasses.get(
    /// ItemClass)` — the default net class's clearance class for one item
    /// kind (`Structure.java:921-925` keepouts, `:1097-1100` planes).
    /// The value moves when a rule scope appends a class of the same name
    /// (`append_clearance_class` retargets via/pin/smd/area).
    fn default_item_clearance_class(&self, item_class: ItemClassIr) -> i32;

    /// The `insertMissingPowerPlanes` conduction-area scan
    /// (`Structure.java:532-541`): true when ANY conduction area already
    /// sits on the layer — the power-plane fallback then skips it.
    fn has_conduction_area_on_layer(&self, layer_no: i32) -> bool;

    /// Java `board.boundingBox` (`Structure.java:563`): the bounds stored
    /// by `create_board` (T43, including the offset(1000)); `None` before.
    /// The power-plane fallback covers this box with a conduction area.
    fn board_bounding_box(&self) -> Option<IntBox>;

    /// Java `board.layerStructure` (`Structure.java:387`) — the BOARD
    /// layer structure, distinct from the PARSER one on
    /// [`crate::state::ParseState::layer_structure`]. Task 8 needs it for
    /// `board.layerStructure.getNo(...)` in the net-class layer rules
    /// (`Network.java:502`, `:599`) and the all-signal keepout scan
    /// (`Network.java:1131-1132`). `None` only before `create_board`
    /// (Java's field is never null there; the network reader cannot run
    /// without a board, so callers treat `None` as the NPE parity path).
    fn board_layer_structure(&self) -> Option<&LayerStructure>;

    // ---- diagnostics + metadata -----------------------------------------

    /// Snapshot of the parse metadata (unit/resolution/quote/snap angle
    /// plus the host fields); called by the read_board assembly (Task 9).
    fn set_metadata(&mut self, metadata: MetadataIr);

    /// Java `DsnFile.adjustPlaneAutorouteSettings` (`:33-113`), called by
    /// the read_board assembly (`DsnReader.java:134-136`) when NO
    /// `(autoroute_settings ...)` scope was read: promotes conduction
    /// areas of net-carrying interior plane layers that hold no traces to
    /// USER_FIXED and flags their nets `contains_plane`. The boolean is
    /// log-only in Java (`changedSettings`) — the reader discards it.
    fn adjust_plane_autoroute_settings(&mut self);
}
