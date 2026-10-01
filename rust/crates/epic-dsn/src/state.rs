//! Port of `io.specctra.parser.ReadScopeParameter`: the mutable parse state
//! threaded through the DSN scope readers, plus the small value types it
//! owns (`Unit`, `AngleRestriction`, `NetList`/`Net`/`Net.Id`,
//! `ComponentPlacement`, `PlaneInfo`).
//!
//! Field mapping (Java `ReadScopeParameter` -> [`ParseState`]):
//! `unit` (default MIL, `:87-88`) -> `unit`; `resolution` (default 100,
//! `:89`) -> `resolution`; `warnings` (`:36`) -> `warnings`; `planeList`
//! (`:42`) -> `plane_list`; `placementList` (`:48`) -> `placement_list`;
//! `netlist` (`:28`) -> `netlist`; `constants` (`:50`) -> `constants`;
//! `viaPadstackNames` (`:56`) -> `via_padstack_names`; `viaAtSmdAllowed`
//! (`:58`) -> `via_at_smd_allowed`; `snapAngle` (default
//! FORTYFIVE_DEGREE, `:59`) -> `snap_angle`; `stringQuote` (default `"`,
//! `:67`) -> `string_quote`; `hostCad`/`hostVersion` (`:69-70`) ->
//! `host_cad`/`host_version`; `dsnFileGeneratedByHost` (`:72`) ->
//! `dsn_file_generated_by_host`; `boardOutlineOk` (`:75`) ->
//! `board_outline_ok`; `writeResolution` (`:77`) -> `write_resolution`;
//! `coordinateTransform` (`:80`) -> `coordinate_transform`;
//! `layerStructure` (`:82`) -> `layer_structure`;
//! `logicalPartMappings`/`logicalParts` (`:62-64`) ->
//! `logical_part_mappings`/`logical_parts` (Task 7 part_library reader).
//!
//! Deliberately deferred (the types land with their scope readers):
//! `autorouteSettings` (`:85`, raw IR per D15, Task 9), and the
//! plumbing fields `scanner`/`boardHandling`/`observers`/`idGenerator`
//! (`:26-29`), which belong to the reader/sink split of Tasks 4/9.
//!
//! T24: the unit comes ONLY from a `(resolution <unit> <n>)` scope — the
//! DSN has no recognized UNIT keyword (the pcb-level dispatch loop
//! skip-scopes `(unit ...)` before any reader sees it), so a
//! `(unit mm)`-only file parses with these defaults. Jar session
//! `/tmp/epic-t25-transform.jsh`, output `/tmp/epic-t25-transform.out`
//! (2026-09-12): `U1 unit/res=mil/100` (no resolution scope) and
//! `U2 unit/res=mil/100` (a `(unit mm)` file) — both observable on the
//! created board's `Communication`.
//!
//! T35: [`NetList`] is CASE-SENSITIVE — Java is a `TreeMap` keyed on
//! `Net.Id.compareTo` (`parser/Net.java:96-102`): UTF-16 name compare,
//! then **wrapping** int subtraction of the subnet numbers. Jar session
//! `/tmp/epic-unit-netid.jsh`, output `/tmp/epic-unit-netid.out`:
//! `GND` vs `gnd` = -32 (distinct keys), and
//! `subnet-overflow(MAX vs -1) = -2147483648` (a true difference of +2^31
//! compares NEGATIVE — the overflow is bug-compatible pinned).
//! (The case-INSENSITIVE net lookup is `rules/Nets.java:39,51`, board-side,
//! not parse state.)

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use crate::coordinate_transform::CoordinateTransform;
use crate::layer_structure::LayerStructure;

/// Java `board.model.structure.Unit` (`Unit.java:10-15`): user units with
/// their micrometer factors. (The `parser/Unit.java` scope-keyword class is
/// dead for reading — T24.)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    /// Java `MIL(25.4)`.
    Mil,
    /// Java `INCH(25_400)`.
    Inch,
    /// Java `MM(1000)`.
    Mm,
    /// Java `UM(1)`.
    Um,
}

impl Unit {
    /// Java private `micrometers` field (jar session
    /// `/tmp/epic-unit-netid.jsh`, output `/tmp/epic-unit-netid.out`:
    /// `UNIT mil micrometers=25.4`, `inch=25400.0`, `mm=1000.0`, `um=1.0`).
    pub fn micrometers(self) -> f64 {
        match self {
            Unit::Mil => 25.4,
            Unit::Inch => 25400.0,
            Unit::Mm => 1000.0,
            Unit::Um => 1.0,
        }
    }

    /// Java `Unit.fromString` (`Unit.java:31-39`):
    /// `valueOf(string.toUpperCase())`, null on no match — i.e.
    /// case-insensitive match of the full token against the four enum
    /// names (jar: `fromString(Um) = um`, `fromString(micron) = null`).
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_uppercase().as_str() {
            "MIL" => Some(Unit::Mil),
            "INCH" => Some(Unit::Inch),
            "MM" => Some(Unit::Mm),
            "UM" => Some(Unit::Um),
            _ => None,
        }
    }

    /// Java `toString` (`Unit.java:41-44`): the lowercase enum name (used by
    /// `Resolution.writeScope`).
    pub fn name(self) -> &'static str {
        match self {
            Unit::Mil => "mil",
            Unit::Inch => "inch",
            Unit::Mm => "mm",
            Unit::Um => "um",
        }
    }

    /// Java `Unit.scale` (`Unit.java:23-25`): scales `value` from `from` to
    /// `to` via the micrometer factors (single multiply/divide — keep the
    /// operation order for bit parity).
    pub fn scale(value: f64, from: Self, to: Self) -> f64 {
        value * from.micrometers() / to.micrometers()
    }
}

/// Java `board.model.structure.AngleRestriction` (`:8-13`): ordinal order
/// NONE, FORTYFIVE_DEGREE, NINETY_DEGREE ("ordinal() and values() rely on
/// the order"). The parse-state default is FORTYFIVE_DEGREE
/// (`ReadScopeParameter.java:59`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AngleRestriction {
    /// Java `NONE`.
    None,
    /// Java `FORTYFIVE_DEGREE`.
    FortyfiveDegree,
    /// Java `NINETY_DEGREE`.
    NinetyDegree,
}

/// Java `Communication.SpecctraParserInfo.WriteResolution` (`:136-145`);
/// set by the parser scope reader (Task 9).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriteResolution {
    /// Java `charName`.
    pub char_name: String,
    /// Java `positiveInt`.
    pub positive_int: i32,
}

/// Java `String.compareTo`: UTF-16 code-unit lexicographic compare; the
/// shorter string compares less when it is a prefix (jar:
/// `NETID GN vs GND = -1`). `str::encode_utf16` iterates exactly the UTF-16
/// code units, so iterator order equality reproduces the sign behavior
/// bit-for-bit for all inputs.
fn cmp_java_string(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// Java `parser/Net.Id` (`Net.java:78-103`): net name + subnet number, the
/// CASE-SENSITIVE TreeMap key of the parse netlist (T35).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetId {
    /// Java `name`.
    pub name: String,
    /// Java `subnetNumber`.
    pub subnet_number: i32,
}

impl Ord for NetId {
    /// Java `Net.Id.compareTo` (`Net.java:96-102`): name compare first, then
    /// `this.subnetNumber - other.subnetNumber` — a WRAPPING i32 subtraction
    /// whose sign is the compare result (jar:
    /// `subnet-overflow(MAX vs -1) = -2147483648` -> compares LESS;
    /// `subnet-overflow(MIN vs 1) = 2147483647` -> compares GREATER).
    fn cmp(&self, other: &Self) -> Ordering {
        match cmp_java_string(&self.name, &other.name) {
            // Java: `this.subnetNumber - other.subnetNumber` with WRAPPING
            // i32 subtraction; only the sign of the wrapped difference is
            // observable through the TreeMap
            Ordering::Equal => self.subnet_number.wrapping_sub(other.subnet_number).cmp(&0),
            ord => ord,
        }
    }
}

impl PartialOrd for NetId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Java `parser/Net.Pin` (`Net.java:107-130`): sorted tuple of component
/// name and pin name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetPin {
    /// Java `componentName`.
    pub component_name: String,
    /// Java `pinName`.
    pub pin_name: String,
}

impl Ord for NetPin {
    /// Java `Net.Pin.compareTo` (`Net.java:122-128`): component name, then
    /// pin name — both `String.compareTo` (jar: `NETPIN a-2 vs a-10 = 1`,
    /// i.e. lexicographic, not numeric).
    fn cmp(&self, other: &Self) -> Ordering {
        match cmp_java_string(&self.component_name, &other.component_name) {
            Ordering::Equal => cmp_java_string(&self.pin_name, &other.pin_name),
            ord => ord,
        }
    }
}

impl PartialOrd for NetPin {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Java `parser/Net` (`Net.java:14-31`). The pin set is a TreeSet in Java
/// (`setPins`, `:73-75`); it is filled by the network scope reader (Task 8).
#[derive(Clone, Debug, PartialEq)]
pub struct Net {
    /// Java `id`.
    pub id: NetId,
    /// Java `pinList` (TreeSet of `Net.Pin`).
    pub pins: BTreeSet<NetPin>,
}

impl Net {
    /// Java `Net(Id)` constructor (`:22-25`): an empty pin list.
    pub fn new(id: NetId) -> Self {
        Self {
            id,
            pins: BTreeSet::new(),
        }
    }
}

/// Java `parser/NetList` (`NetList.java:12-68`): nets sorted by
/// [`NetId`] (a Java `TreeMap` -> `BTreeMap`), case-sensitive (T35).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NetList {
    /// Java private `nets` map; keys are the `Net.Id`s.
    pub nets: BTreeMap<NetId, Net>,
}

impl NetList {
    /// Java `addNet` (`NetList.java:23-35`): adds the net and returns it;
    /// returns None (Java null) and adds NOTHING if a net with the id
    /// already exists.
    pub fn add_net(&mut self, id: NetId) -> Option<&mut Net> {
        if self.nets.contains_key(&id) {
            return None;
        }
        let net = Net::new(id.clone());
        self.nets.insert(id.clone(), net);
        self.nets.get_mut(&id)
    }

    /// Java `contains` (`NetList.java:18-20`).
    pub fn contains(&self, id: &NetId) -> bool {
        self.nets.contains_key(id)
    }

    /// Java `getNet` (`NetList.java:40-42`).
    pub fn get(&self, id: &NetId) -> Option<&Net> {
        self.nets.get(id)
    }

    /// Mutable view for `Net.setPins` (`Network.java:1425`): REPLACES the
    /// pin collection even on a duplicate net id (`getNet` never returns
    /// null right after the contains/add dance above).
    pub fn get_mut(&mut self, id: &NetId) -> Option<&mut Net> {
        self.nets.get_mut(id)
    }

    /// Java `getNets(component, pin)` (`NetList.java:44-55`): every net
    /// whose pin set contains `(component_name, pin_name)`, in netlist
    /// (TreeMap key) order.
    ///
    /// Java's `netPins != null` guard (`:50`) only skips nets whose pin
    /// collection was never assigned; here `pins` is always a (possibly
    /// empty) set, which the `contains` check filters identically. Jar
    /// session `/tmp/epic-t3-part-a.jsh`, output `/tmp/epic-t3-part-a.out`
    /// (2026-09-12): on nets A (pins never set), B {(U1,1),(U2,2)},
    /// C {(U1,2)}, a {(U1,1)} — "NETS getNets(U1,1)=B,a," (B before the
    /// lowercase net proves the case-sensitive key order), "getNets(U1,2)=C,"
    /// (same component, other pin), "getNets(U2,1)=" and "getNets(U9,9)="
    /// (empty).
    pub fn get_nets(&self, component_name: &str, pin_name: &str) -> Vec<&Net> {
        let search_pin = NetPin {
            component_name: component_name.to_string(),
            pin_name: pin_name.to_string(),
        };
        self.nets
            .values()
            .filter(|net| net.pins.contains(&search_pin))
            .collect()
    }
}

/// Java `ComponentPlacement.ItemClearanceInfo` (`ComponentPlacement.java:82-91`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemClearanceInfo {
    /// Java `name` (pin / keepout / via-keepout / place-keepout name).
    pub name: String,
    /// Java `clearanceClass`.
    pub clearance_class: String,
}

/// Java `ComponentPlacement.ComponentLocation` (`ComponentPlacement.java:24-80`).
/// The four info maps are Java TreeMaps keyed by name — name-sorted, NOT
/// file order (T45, `Component.java:190-194`; jar pin lands with the
/// placement reader, Task 7).
#[derive(Clone, Debug, PartialEq)]
pub struct ComponentLocation {
    /// Java `name`: the placed component instance name.
    pub name: String,
    /// Java `coor`: the x/y location (DSN coordinates, as parsed doubles).
    /// `None` (Java null) for an UNPLACED component — `(place U9)` with no
    /// coordinates stores a `ComponentLocation` with a null coor and
    /// `isFront = true`, `rotation = 0` (`Component.java:198-225`; jar
    /// `/tmp/epic-t7-probe.out` t7-place: `U9`/`U10` `loc=null placed=false`).
    pub coor: Option<[f64; 2]>,
    /// Java `isFront`: component side (true) or solder side.
    pub is_front: bool,
    /// Java `rotation` in degrees.
    pub rotation: f64,
    /// Java `positionFixed`.
    pub position_fixed: bool,
    /// Java `pin_infos`.
    pub pin_infos: BTreeMap<String, ItemClearanceInfo>,
    /// Java `keepout_infos`.
    pub keepout_infos: BTreeMap<String, ItemClearanceInfo>,
    /// Java `via_keepout_infos`.
    pub via_keepout_infos: BTreeMap<String, ItemClearanceInfo>,
    /// Java `place_keepout_infos`.
    pub place_keepout_infos: BTreeMap<String, ItemClearanceInfo>,
    /// Java `partNumber` (nullable).
    pub part_number: Option<String>,
}

/// Java `parser/ComponentPlacement` (`ComponentPlacement.java:9-21`): one
/// `(placement (component ...))` group.
#[derive(Clone, Debug, PartialEq)]
pub struct ComponentPlacement {
    /// Java `libName`: the library component (image) name.
    pub lib_name: String,
    /// Java `locations`, in file order.
    pub locations: Vec<ComponentLocation>,
}

/// Java `PartLibrary.PartPin` (`PartLibrary.java:316-336`): one
/// `(pin <pin_name> <int> <gate_name> <gate_swap> <gate_pin_name>
/// <gate_pin_swap> ...)` entry of a logical part. Subgate tokens after
/// the fifth field are DRAINED (do-while to the closing bracket) and not
/// stored (`PartLibrary.java:300-305`).
#[derive(Clone, Debug, PartialEq)]
pub struct PartPin {
    /// Java `pinName`.
    pub pin_name: String,
    /// Java `gateName`.
    pub gate_name: String,
    /// Java `gateSwapCode`.
    pub gate_swap_code: i32,
    /// Java `gatePinName`.
    pub gate_pin_name: String,
    /// Java `gatePinSwapCode`.
    pub gate_pin_swap_code: i32,
}

/// Java `PartLibrary.LogicalPart` (`PartLibrary.java:338-350`): one
/// `(logicalPart <name> ...)` scope — the gate/pin-swap structure of a
/// logical part. Pins keep file order (Java `LinkedList`).
#[derive(Clone, Debug, PartialEq)]
pub struct LogicalPart {
    /// Java `name`.
    pub name: String,
    /// Java `partPins`, file order.
    pub part_pins: Vec<PartPin>,
}

/// Java `PartLibrary.LogicalPartMapping` (`PartLibrary.java:302-314`):
/// one `(logical_part_mapping <name> (comp <components>))` scope. The
/// component set is a Java `TreeSet<String>` — sorted, deduplicated;
/// order is LOAD-BEARING at consumption time (`Network.searchLibPackage`
/// takes `components.getFirst()`, the SORTED-FIRST name — jar
/// `/tmp/epic-t7-probe.out` t7-part vs t7-part2 discriminates it).
/// Rust `BTreeSet` orders by UTF-8 bytes where Java `TreeSet` orders by
/// UTF-16 code units; the divergence needs non-BMP vs U+E000-U+FFFF
/// names (same documented caveat as the Task 2 BTreeMap decision).
#[derive(Clone, Debug, PartialEq)]
pub struct LogicalPartMapping {
    /// Java `name`.
    pub name: String,
    /// Java `components` (sorted set).
    pub components: BTreeSet<String>,
}

/// Mirror of `Shape.ReadAreaScopeResult` (`Shape.java:633-646`): the shape
/// list is needed at INSERTION time (Task 6 keepouts/planes read the first
/// shape's layer and transform border + holes together). The first entry is
/// the border; the rest are `(window ...)` holes. Entries are `None` where
/// Java stored a null shape — only a failed WINDOW shape leaves a `None`
/// in a successful result (a failed FIRST shape sets `resultOk = false` and
/// the whole read returns null).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AreaScopeResult {
    /// Java `areaName` (nullable).
    pub area_name: Option<String>,
    /// Java `clearanceClassName` (nullable).
    pub clearance_class: Option<String>,
    /// Java `shapeList` — border first, holes after, file order.
    pub shapes: Vec<Option<crate::shape::Shape>>,
}

/// Java `ReadScopeParameter.PlaneInfo` (`ReadScopeParameter.java:176-186`):
/// a `(plane ...)` scope held on the parse state until the layers are fully
/// known (evaluated after the structure scope, Task 6).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaneInfo {
    /// Java `area` — `None` (Java null) when `read_area_scope` failed:
    /// `Plane.readScope` stores the result UNCHECKED (`Plane.java:72-81`);
    /// the null surfaces as the `Structure.java:1084` NPE at insertion
    /// (parse-fatal parity, jar `/tmp/epic-t6-board.out` t6-plane-badshape).
    pub area: Option<AreaScopeResult>,
    /// Java `netName`.
    pub net_name: String,
}

/// The mutable DSN parse state (module docs carry the Java field mapping).
/// Data only: defaults here, all logic in the scope readers (Tasks 5-9).
#[derive(Clone, Debug, PartialEq)]
pub struct ParseState {
    /// Java `unit` — default MIL (`ReadScopeParameter.java:87-88`; jar U1).
    pub unit: Unit,
    /// Java `resolution` — default 100 (`:89`; jar U1 `unit/res=mil/100`).
    pub resolution: i32,
    /// Java `warnings` (`:36`), collected during parsing (parity surface
    /// D12).
    pub warnings: Vec<String>,
    /// Java `planeList` (`:42`).
    pub plane_list: Vec<PlaneInfo>,
    /// Java `placementList` (`:48`).
    pub placement_list: Vec<ComponentPlacement>,
    /// Java `logicalPartMappings` (`:62`, `LinkedList` — file order),
    /// appended by the part_library scope (Task 7) and consumed by the
    /// network scope (Task 8, `Network.insertLogicalParts`).
    pub logical_part_mappings: Vec<LogicalPartMapping>,
    /// Java `logicalParts` (`:64`, `LinkedList` — file order), appended by
    /// the part_library scope (Task 7) and consumed by the network scope
    /// (Task 8, `Network.insertLogicalParts`).
    pub logical_parts: Vec<LogicalPart>,
    /// Java `netlist` (`:28`) — case-sensitive order (T35).
    pub netlist: NetList,
    /// Java `constants` (`:50`) — `Collection<String[]>` of name/value pairs.
    pub constants: Vec<[String; 2]>,
    /// Java `viaPadstackNames` (`:56`) — null until the structure scope
    /// fills it.
    pub via_padstack_names: Option<Vec<String>>,
    /// Java `viaAtSmdAllowed` (`:58`) — default false.
    pub via_at_smd_allowed: bool,
    /// Java `snapAngle` (`:59`) — default FORTYFIVE_DEGREE.
    pub snap_angle: AngleRestriction,
    /// Java `stringQuote` (`:67`) — default `"`.
    pub string_quote: String,
    /// Java `hostCad` (`:69`).
    pub host_cad: Option<String>,
    /// Java `hostVersion` (`:70`).
    pub host_version: Option<String>,
    /// Java `dsnFileGeneratedByHost` (`:72`) — default true.
    pub dsn_file_generated_by_host: bool,
    /// Java `boardOutlineOk` (`:75`) — default true; set false by the
    /// structure reader when the outline is absent.
    pub board_outline_ok: bool,
    /// Java `writeResolution` (`:77`).
    pub write_resolution: Option<WriteResolution>,
    /// Java `coordinateTransform` (`:80`) — set when the structure scope
    /// creates the board.
    pub coordinate_transform: Option<CoordinateTransform>,
    /// Java `layerStructure` (`:82`) — set when the structure scope creates
    /// the board.
    pub layer_structure: Option<LayerStructure>,
    /// Java `autorouteSettings` (`:74`, default null) — set by the
    /// structure scope's `(autoroute_settings ...)` arm (Task 9); `None`
    /// (Java null) arms the `adjustPlaneAutorouteSettings` fallback in the
    /// read_board assembly (`DsnReader.java:134-136`).
    pub autoroute_settings: Option<crate::scope::autoroute_settings::AutorouteSettingsIr>,
}

impl Default for ParseState {
    /// The Java field initializers (`ReadScopeParameter.java:36-89`).
    fn default() -> Self {
        Self {
            unit: Unit::Mil,
            resolution: 100,
            warnings: Vec::new(),
            plane_list: Vec::new(),
            placement_list: Vec::new(),
            logical_part_mappings: Vec::new(),
            logical_parts: Vec::new(),
            netlist: NetList::default(),
            constants: Vec::new(),
            via_padstack_names: None,
            via_at_smd_allowed: false,
            snap_angle: AngleRestriction::FortyfiveDegree,
            string_quote: "\"".to_string(),
            host_cad: None,
            host_version: None,
            dsn_file_generated_by_host: true,
            board_outline_ok: true,
            write_resolution: None,
            coordinate_transform: None,
            layer_structure: None,
            autoroute_settings: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Jar session `/tmp/epic-unit-netid.jsh`, output
    /// `/tmp/epic-unit-netid.out` (2026-09-12): "UNIT mil micrometers=25.4",
    /// "UNIT inch micrometers=25400.0", "UNIT mm micrometers=1000.0",
    /// "UNIT um micrometers=1.0".
    #[test]
    fn unit_micrometer_constants() {
        assert_eq!(Unit::Mil.micrometers(), 25.4);
        assert_eq!(Unit::Inch.micrometers(), 25400.0);
        assert_eq!(Unit::Mm.micrometers(), 1000.0);
        assert_eq!(Unit::Um.micrometers(), 1.0);
    }

    /// Jar: "UNIT scale(2.5, MM, INCH) = 0.0984251968503937" — the Java
    /// operation order `value * from / to`.
    #[test]
    fn unit_scale_matches_java_operation_order() {
        assert_eq!(Unit::scale(2.5, Unit::Mm, Unit::Inch), 0.0984251968503937);
    }

    /// Jar: "UNIT fromString(mil) = mil", "fromString(MM) = mm",
    /// "fromString(Um) = um", "fromString(INCH) = inch", "fromString(UM) =
    /// um", "fromString(micron) = null" — case-insensitive full-token match,
    /// None otherwise (the None path is what makes `(resolution micron 10)`
    /// a parse error, jar `/tmp/epic-t25-transform.out` U5 `result=
    /// ParseError`).
    #[test]
    fn unit_from_name_is_case_insensitive() {
        assert_eq!(Unit::from_name("mil"), Some(Unit::Mil));
        assert_eq!(Unit::from_name("MM"), Some(Unit::Mm));
        assert_eq!(Unit::from_name("Um"), Some(Unit::Um));
        assert_eq!(Unit::from_name("INCH"), Some(Unit::Inch));
        assert_eq!(Unit::from_name("UM"), Some(Unit::Um));
        assert_eq!(Unit::from_name("micron"), None);
    }

    /// Jar: "UNIT toString(MIL) = mil" — lowercase names (DSN output
    /// spelling).
    #[test]
    fn unit_names_are_lowercase() {
        assert_eq!(Unit::Mil.name(), "mil");
        assert_eq!(Unit::Inch.name(), "inch");
        assert_eq!(Unit::Mm.name(), "mm");
        assert_eq!(Unit::Um.name(), "um");
    }

    /// T35 name compare (jar `/tmp/epic-unit-netid.out`): "NETID GND vs gnd
    /// = -32" (case-SENSITIVE — both keys coexist in the netlist), "NETID GN
    /// vs GND = -1" (prefix compares less).
    #[test]
    fn net_id_name_compare_is_case_sensitive() {
        assert_eq!(cmp_java_string("GND", "gnd"), Ordering::Less);
        assert_eq!(
            NetId {
                name: "GND".to_string(),
                subnet_number: 0
            }
            .cmp(&NetId {
                name: "gnd".to_string(),
                subnet_number: 0
            }),
            Ordering::Less
        );
        assert_eq!(cmp_java_string("GN", "GND"), Ordering::Less);
    }

    /// T35 subnet compare (jar `/tmp/epic-unit-netid.out`): "NETID
    /// same-subnet = 0"; "NETID subnet-overflow(MAX vs -1) = -2147483648" —
    /// `2147483647 - (-1)` WRAPS to i32::MIN, so the larger subnet compares
    /// LESS; "NETID subnet-overflow(MIN vs 1) = 2147483647" — the smaller
    /// subnet compares GREATER. The Rust port must wrap identically.
    #[test]
    fn net_id_subnet_compare_wraps() {
        let id = |subnet: i32| NetId {
            name: "a".to_string(),
            subnet_number: subnet,
        };
        assert_eq!(id(5).cmp(&id(5)), Ordering::Equal);
        assert_eq!(id(i32::MAX).cmp(&id(-1)), Ordering::Less);
        assert_eq!(id(i32::MIN).cmp(&id(1)), Ordering::Greater);
    }

    /// NetPin compare (jar `/tmp/epic-unit-netid.out`): "NETPIN b-2 vs a-9
    /// = 1" (component name first), "NETPIN a-2 vs a-10 = 1" (pin name
    /// lexicographic: "2" > "10").
    #[test]
    fn net_pin_compare_is_lexicographic() {
        let pin = |component: &str, pin_name: &str| NetPin {
            component_name: component.to_string(),
            pin_name: pin_name.to_string(),
        };
        assert_eq!(pin("b", "2").cmp(&pin("a", "9")), Ordering::Greater);
        assert_eq!(pin("a", "2").cmp(&pin("a", "10")), Ordering::Greater);
    }

    /// T35: the parse netlist keeps `GND` and `gnd` as DISTINCT nets
    /// (case-sensitive keys); `add_net` returns None on a duplicate id and
    /// adds nothing (Java `NetList.addNet`, `NetList.java:23-35`); iteration
    /// is sorted by the pinned `NetId` order ("A" < "GND" < "gnd").
    #[test]
    fn netlist_is_case_sensitive_and_sorted() {
        let mut netlist = NetList::default();
        let id = |name: &str| NetId {
            name: name.to_string(),
            subnet_number: 0,
        };
        assert!(netlist.add_net(id("GND")).is_some());
        assert!(netlist.add_net(id("gnd")).is_some());
        assert!(netlist.add_net(id("A")).is_some());
        // duplicate: Java addNet returns null, no second entry
        assert!(netlist.add_net(id("GND")).is_none());
        assert_eq!(netlist.nets.len(), 3);
        assert!(netlist.contains(&id("gnd")));
        assert_eq!(netlist.get(&id("GND")).expect("GND present").id, id("GND"));
        let names: Vec<&str> = netlist
            .nets
            .values()
            .map(|net| net.id.name.as_str())
            .collect();
        assert_eq!(names, vec!["A", "GND", "gnd"]);
    }

    /// `getNets` (jar session `/tmp/epic-t3-part-a.jsh`, output
    /// `/tmp/epic-t3-part-a.out`, 2026-09-12): nets A (pins never set),
    /// B {(U1,1),(U2,2)}, C {(U1,2)}, a {(U1,1)} — "NETS getNets(U1,1)=B,a,"
    /// (B before the lowercase net: the case-sensitive key order), "getNets
    /// (U1,2)=C," (same component, other pin), "getNets(U2,1)=" (same net
    /// name set, wrong pin pair) and "getNets(U9,9)=" (no match; the
    /// pinless net A never trips a null collection).
    #[test]
    fn get_nets_filters_by_pin_in_key_order() {
        let mut netlist = NetList::default();
        let id = |name: &str| NetId {
            name: name.to_string(),
            subnet_number: 0,
        };
        let pin = |component: &str, pin_name: &str| NetPin {
            component_name: component.to_string(),
            pin_name: pin_name.to_string(),
        };
        netlist.add_net(id("A"));
        netlist
            .add_net(id("B"))
            .expect("B fresh")
            .pins
            .extend([pin("U1", "1"), pin("U2", "2")]);
        netlist
            .add_net(id("C"))
            .expect("C fresh")
            .pins
            .insert(pin("U1", "2"));
        netlist
            .add_net(id("a"))
            .expect("a fresh")
            .pins
            .insert(pin("U1", "1"));

        fn names<'a>(nets: &[&'a Net]) -> Vec<&'a str> {
            nets.iter().map(|net| net.id.name.as_str()).collect()
        }
        assert_eq!(names(&netlist.get_nets("U1", "1")), vec!["B", "a"]);
        assert_eq!(names(&netlist.get_nets("U1", "2")), vec!["C"]);
        assert!(netlist.get_nets("U2", "1").is_empty());
        assert!(netlist.get_nets("U9", "9").is_empty());
    }

    /// Defaults (Java initializers `ReadScopeParameter.java:36-89`).
    /// T24 observable: jar `/tmp/epic-t25-transform.out` U1 `unit/res=
    /// mil/100` (no resolution scope) and U2 `unit/res=mil/100` (a
    /// `(unit mm)`-only file — the dead UNIT keyword never touches the
    /// state), both read off the created board's Communication; P1 in the
    /// same session shows the default resolution at work
    /// (`(resolution mil 100)` behaves identically to no scope at all).
    #[test]
    fn parse_state_defaults() {
        let state = ParseState::default();
        assert_eq!(state.unit, Unit::Mil);
        assert_eq!(state.resolution, 100);
        assert!(state.warnings.is_empty());
        assert!(state.plane_list.is_empty());
        assert!(state.placement_list.is_empty());
        assert!(state.netlist.nets.is_empty());
        assert!(state.constants.is_empty());
        assert_eq!(state.via_padstack_names, None);
        assert!(!state.via_at_smd_allowed);
        assert_eq!(state.snap_angle, AngleRestriction::FortyfiveDegree);
        assert_eq!(state.string_quote, "\"");
        assert_eq!(state.host_cad, None);
        assert_eq!(state.host_version, None);
        assert!(state.dsn_file_generated_by_host);
        assert!(state.board_outline_ok);
        assert_eq!(state.write_resolution, None);
        assert_eq!(state.coordinate_transform, None);
        assert_eq!(state.layer_structure, None);
    }

    /// The info maps of a `ComponentLocation` are name-sorted BTreeMaps
    /// (Java TreeMaps, `Component.java:190-194`, T45; the jar-captured
    /// insertion-order pin lands with the placement reader, Task 7).
    #[test]
    fn component_location_infos_are_name_sorted() {
        let location = ComponentLocation {
            name: "U1".to_string(),
            coor: Some([1000.0, -2000.5]),
            is_front: true,
            rotation: 90.0,
            position_fixed: false,
            pin_infos: BTreeMap::from([
                (
                    "zeta".to_string(),
                    ItemClearanceInfo {
                        name: "zeta".to_string(),
                        clearance_class: "default".to_string(),
                    },
                ),
                (
                    "alpha".to_string(),
                    ItemClearanceInfo {
                        name: "alpha".to_string(),
                        clearance_class: "class 1".to_string(),
                    },
                ),
            ]),
            keepout_infos: BTreeMap::new(),
            via_keepout_infos: BTreeMap::new(),
            place_keepout_infos: BTreeMap::new(),
            part_number: Some("PN-7".to_string()),
        };
        let pin_names: Vec<&str> = location.pin_infos.keys().map(String::as_str).collect();
        assert_eq!(pin_names, vec!["alpha", "zeta"]);
    }
}
