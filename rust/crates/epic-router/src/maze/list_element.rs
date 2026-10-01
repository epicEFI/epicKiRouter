//! Java `autoroute/maze/MazeListElement.java` (115 lines) — one
//! element of the maze expansion front, plus the front itself.
//!
//! ## The front data structure (the load-bearing decision)
//!
//! Java: `TreeSet<MazeListElement>` ordered by
//! [`MazeListElement::compareTo`] (`:80-113`) —
//! `sortingValue → expansionValue → door.getId() → sectionNoOfDoor → 0`,
//! ALL compared with `<`/`>` (never `Double.compare`). Port: a
//! [`BTreeSet`] over [`MazeListElement`] with an [`Ord`] that
//! replicates the `<`/`>` semantics through
//! `f64::partial_cmp` + fall-through:
//!
//! * **NaN** (Java): both `<` and `>` are false → the value
//!   tie-break FALLS THROUGH to the next key (the id/section ties) —
//!   NaN behaves as "equal on values". Rust: `partial_cmp` returns
//!   `None` → exactly the same fall-through. `total_cmp` would place
//!   NaN ABOVE every finite value — a silent front reorder — so it is
//!   deliberately NOT used.
//! * **-0.0 vs 0.0** (Java): `-0.0 < 0.0` and `-0.0 > 0.0` are both
//!   false → the values tie and the next key decides (full tie ⇒ the
//!   element is a DUPLICATE the set drops). `total_cmp` orders
//!   `-0.0 < +0.0`; `partial_cmp` ties them — Java again.
//! * **Unreachability**: every producer of `expansion_value` /
//!   `sorting_value` is finite by construction — weighted distances
//!   (squared/abs coordinate deltas times positive cost-table
//!   entries), integer add-costs, the bend penalty (≥ 0), and
//!   `DestinationDistance::calculate` (the T5 capture showed the
//!   targetless arm emits `2147483647.0`, FINITE; the T8 heuristic is
//!   a lower bound over joined boxes). The constructor therefore
//!   `debug_assert!`s finiteness: with all-finite inputs the
//!   comparator is a strict lexicographic total order (Java's NaN
//!   fall-through is intransitive in general — e.g. a(NaN, id 3) <
//!   b(0.0, id 9) < c(1.0, id 1) but a > c — so an unguarded
//!   `BTreeSet` over it would order by insertion history). The
//!   [`compare`] helper stays NaN-faithful for direct pairwise pins
//!   (Java's `compareTo` is only ever a pairwise contract).
//! * **Dedup**: a Java `TreeSet.add` whose compareTo yields 0 is a
//!   NO-OP (the second element is dropped). `BTreeSet::insert`
//!   returning `false` is the same contract — the front is a SET, not
//!   a multiset, and insertion order never matters.

use std::collections::BTreeSet;

use epic_geometry::float_line::FloatLine;
use epic_geometry::float_point::FloatPoint;
use epic_geometry::tile_shape::TileShape;

use crate::control::FanoutSettingsIr;
use crate::drill::Adjustment;
use crate::expansion::{ExpansionDoor, TargetItemExpansionDoor};

/// Java `ExpandableObject` — the front's door polymorphism. The four
/// Java implementors relevant to the maze search (`ExpansionDoor`,
/// `TargetItemExpansionDoor`, `DrillPage`, `ExpansionDrill`) collapse
/// into one enum; identity inside a variant is the Java `getId()`
/// formula plus the engine-side coordinates for the grid-anchored
/// kinds.
#[derive(Clone, Debug, PartialEq)]
pub enum ExpandableObject {
    /// Java `ExpansionDoor` — a regular room door. NOTE Java's
    /// `instanceof` hierarchy: `TargetItemExpansionDoor` IMPLEMENTS
    /// `ExpandableObject` directly and does NOT match
    /// `instanceof ExpansionDoor` (`TargetItemExpansionDoor.java:11`).
    RoomDoor(ExpansionDoor),
    /// Java `TargetItemExpansionDoor`.
    TargetDoor(TargetItemExpansionDoor),
    /// Java `DrillPage` — identified by its (row, column) grid cell in
    /// the [`DrillPageArray`]. `id` is Java's LIVE
    /// `DrillPage.getId()` (`31 * shape.getId() + netNumber`)
    /// SNAPSHOTTED at element construction (the engine resolves it
    /// through the page array) — see the deviation note below.
    DrillPage { row: i32, column: i32, id: i32 },
    /// Java `ExpansionDrill` — the `d`-th drill of the page cell.
    /// `id` is the Java `ExpansionDrill.getId()` formula snapshot.
    Drill {
        row: i32,
        column: i32,
        d: usize,
        id: i32,
    },
    /// Java `ExpansionDrill` NOT living in the drill-page grid — the
    /// drill constructed over a ripped via (`MazeSearchEngine.expandToRoomDoors`
    /// → `Via.getAutorouteDrillInfo`). Java carries the drill OBJECT in
    /// the element; the Rust page-grid variant cannot represent it (its
    /// state resolves through `(row, column, d)`), so standalone drills
    /// are tagged separately and their state lives in
    /// [`crate::maze::search_engine::MazeSearchEngine`]'s drill map.
    /// `shape` is the drill's stored shape (`DrillItem.getShape`), kept
    /// here because `door_shape` is engine-free.
    StandaloneDrill { id: i32, shape: TileShape },
}

/// DEVIATION (front tie-break 2): Java's `TreeSet` comparator reads
/// `door.getId()` LIVE on every comparison, and `DrillPage.getId()`
/// changes when `getDrills` first memoizes the page's net
/// (`netNumber` goes -1 → net; bug-128). A `BTreeSet` needs a stable
/// key, so the Rust element freezes the id AS OF THE ADD. The
/// ordering-invariance proof covers PAGE-VS-PAGE pairs only (the net
/// term cancels there). A page-vs-nonpage exact-f64 tie CAN resolve
/// differently — but that window is Java's comparator-invalidation
/// territory (`getId` mutates on first `getDrills` memoization while
/// the element sits in the `TreeSet`: ordering unspecified there), so
/// freeze-at-add cannot diverge from DEFINED Java semantics. Pin
/// suites must not compare pre-memo page elements against post-memo
/// insertions.
impl ExpandableObject {
    /// Java `ExpandableObject.getDimension()` — 2 for target doors,
    /// pages and drills; the door's dimension for room doors. (Java
    /// `TargetItemExpansionDoor` implements `ExpandableObject`
    /// directly and is NOT an `ExpansionDoor` — the enum variants are
    /// the exact `instanceof` partition.)
    #[must_use]
    pub fn dimension(&self) -> i32 {
        match self {
            ExpandableObject::RoomDoor(door) => door.dimension,
            ExpandableObject::TargetDoor(_)
            | ExpandableObject::DrillPage { .. }
            | ExpandableObject::Drill { .. }
            | ExpandableObject::StandaloneDrill { .. } => 2,
        }
    }

    /// Java `door instanceof ExpansionDrill` — page-grid drills and
    /// standalone (ripped-via) drills are one Java class; the
    /// instanceof checks of `expandToRoomDoors` (fanout `:361-368`,
    /// other-layers, the drill-page trigger `:478-489`) all read true
    /// for both.
    #[must_use]
    pub fn is_drill(&self) -> bool {
        matches!(
            self,
            ExpandableObject::Drill { .. } | ExpandableObject::StandaloneDrill { .. }
        )
    }
}

impl ExpandableObject {
    /// Java `getId()` per kind (`MazeListElement.compareTo:95-96`
    /// reads it for the second value tie-break).
    #[must_use]
    pub fn id(&self) -> i32 {
        match self {
            ExpandableObject::RoomDoor(door) => door.id(),
            ExpandableObject::TargetDoor(door) => door.id(),
            ExpandableObject::DrillPage { id, .. } => *id,
            ExpandableObject::Drill { id, .. } => *id,
            ExpandableObject::StandaloneDrill { id, .. } => *id,
        }
    }

    /// The opaque `u64` key stored in [`crate::drill::MazeSearchElement`]
    /// `backtrack_door` (the D17 opaque-key decision). Tagged encoding:
    /// kind in the top bits, identity below — deterministic, no
    /// allocation, stable across a search.
    #[must_use]
    pub fn key(&self) -> u64 {
        const ROOM_DOOR: u64 = 1 << 62;
        const TARGET_DOOR: u64 = 2 << 62;
        const PAGE: u64 = 3 << 62;
        const DRILL: u64 = 4 << 62;
        const STANDALONE_DRILL: u64 = 5 << 62;
        match self {
            ExpandableObject::RoomDoor(door) => {
                // Java's maze-search state is PER DOOR OBJECT
                // (`sectionArr` lives on the door); the instance tag
                // (door.rs module doc) is that identity — fold it so
                // equal-VALUED doors of id-colliding incomplete-room
                // twins keep separate state slots (the t7 phantom
                // class). Bits 32-33 carry the dimension, bits 34-61
                // the tag; the tag counter is PROCESS-GLOBAL and never
                // resets, so the operative wrap margin is cumulative
                // door constructions per process (~2.7e8 before the
                // low-28 bits wrap), not a per-search population.
                ROOM_DOOR
                    | (door.id() as u32 as u64)
                    | ((door.dimension.clamp(0, 3) as u64) << 32)
                    | ((door.tag & 0x0FFF_FFFF) << 34)
            }
            // Java's maze-search state is PER OBJECT — each
            // `TargetItemExpansionDoor` owns its own `mazeSearchInfo`
            // field (`TargetItemExpansionDoor.java:19/:54`) — so two
            // tree-entry doors of one item (SAME `getId()`, distinct
            // objects) must NOT share a state slot. `getId()`
            // (`:70-74`) ignores `treeEntryNo` (it is also Java
            // `MazeListElement.compareTo`'s tie-break), so the STATE
            // key folds the tree entry into the free bits 32-61.
            // (T6 root cause of the t7 divergence: the missing fold
            // aliased item-14 entries 0/1 in one search — the 14/1
            // seed's post-expansion occupation poisoned the 14/0
            // seed's slot, the pop loop discarded the seed as
            // occupied, and the ten-row expansion of Java's golden
            // rows 74-83 never ran.)
            ExpandableObject::TargetDoor(door) => {
                TARGET_DOOR
                    | (door.id() as u32 as u64)
                    | ((u64::from(door.tree_entry_no) & 0x3FFF_FFFF) << 32)
            }
            ExpandableObject::DrillPage { row, column, .. } => {
                PAGE | (*row as u32 as u64) | ((*column as u32 as u64) << 32)
            }
            ExpandableObject::Drill { row, column, d, .. } => {
                DRILL
                    | (*row as u32 as u64)
                    | ((*column as u32 as u64) << 21)
                    | ((*d as u32 as u64) << 42)
            }
            // Drill ids are the Java `ExpansionDrill.getId()` HASH
            // (`31 * (31 * location.getId() + firstLayer) + lastLayer`,
            // one id per (location, layer span) WITHIN one search) —
            // NOT globally unique across searches, and the
            // `standalone_drills` map is fresh per engine, so keying
            // the state slot by the id inside one engine instance is
            // exactly Java's object-identity scope.
            //
            // COLLISION WINDOW: the hash is not injective — two
            // DISTINCT (location, layer-span) triples of the SAME
            // search can fold to one i32. The consequence is silent
            // state aliasing (the second drill replaces the first's
            // `standalone_drills` entry and shares its maze-search
            // state) where Java's reference identity keeps the objects
            // apart. The window is scoped to the handful of drills a
            // single search constructs; no fixture has produced a
            // within-search collision.
            ExpandableObject::StandaloneDrill { id, .. } => STANDALONE_DRILL | (*id as u32 as u64),
        }
    }
}

/// Java `MazeListElement` — the front element (ctor `:54-77`, the
/// non-ctor mutable `ripupCost` field carried as a plain field; it is
/// set by the ripup paths T7 ports).
#[derive(Clone, Debug, PartialEq)]
pub struct MazeListElement {
    /// Java `door` — the door or drill this element expands through.
    pub door: ExpandableObject,
    /// Java `sectionNoOfDoor` — the door section (the drill LAYER for
    /// drills, the board layer for pages).
    pub section_no_of_door: i32,
    /// Java `backtrackDoor` — the door this element was expanded from
    /// (`None` for the init seeds, Java null).
    pub backtrack_door: Option<ExpandableObject>,
    /// Java `sectionNoOfBacktrackDoor`.
    pub section_no_of_backtrack_door: i32,
    /// Java `expansionValue` — the weighted distance to the start.
    pub expansion_value: f64,
    /// Java `sortingValue` — expansionValue + destination distance;
    /// the front's primary key.
    pub sorting_value: f64,
    /// Java `nextRoom` — the room expanded from this element
    /// (`None` = Java null: drill elements and target-door elements).
    pub next_room_key: Option<u64>,
    /// Java `shapeEntry` — the entry segment (both endpoints usually
    /// equal).
    pub shape_entry: FloatLine,
    /// Java `roomRipped`.
    pub room_ripped: bool,
    /// Java `adjustment`.
    pub adjustment: Adjustment,
    /// Java `alreadyChecked` — the ctor's 11th field.
    pub already_checked: bool,
    /// Java `ripupCost` — NON-CTOR field, default 0; only set when
    /// this element directly paid ripup cost (T7 propagates it).
    pub ripup_cost: i32,
}

impl MazeListElement {
    /// Java ctor (`:54-77`) — argument order mirrored. Debug-asserts
    /// the finiteness of both values (see the module doc: every
    /// producer is finite; the assert keeps the [`Ord`] a total order).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        door: ExpandableObject,
        section_no_of_door: i32,
        backtrack_door: Option<ExpandableObject>,
        section_no_of_backtrack_door: i32,
        expansion_value: f64,
        sorting_value: f64,
        next_room_key: Option<u64>,
        shape_entry: FloatLine,
        room_ripped: bool,
        adjustment: Adjustment,
        already_checked: bool,
    ) -> Self {
        debug_assert!(
            expansion_value.is_finite(),
            "expansionValue must be finite (Java's producers cannot emit NaN/Inf): {expansion_value}"
        );
        debug_assert!(
            sorting_value.is_finite(),
            "sortingValue must be finite (Java's producers cannot emit NaN/Inf): {sorting_value}"
        );
        MazeListElement {
            door,
            section_no_of_door,
            backtrack_door,
            section_no_of_backtrack_door,
            expansion_value,
            sorting_value,
            next_room_key,
            shape_entry,
            room_ripped,
            adjustment,
            already_checked,
            ripup_cost: 0,
        }
    }
}

/// Java `MazeListElement.compareTo` (`:80-113`) — the front order.
/// Pairwise-faithful replication of the `<`/`>` semantics:
/// `partial_cmp`'s `None` (NaN) and `Some(Equal)` (±0.0) both FALL
/// THROUGH to the next tie-break, exactly like both-`<`-and-`>`-false
/// in Java. Only sound as a set order when all values are finite —
/// which [`MazeListElement::new`] debug-asserts.
#[must_use]
pub fn compare(a: &MazeListElement, b: &MazeListElement) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let value_tie = |x: f64, y: f64| match x.partial_cmp(&y) {
        Some(Ordering::Less) => Ordering::Less,
        Some(Ordering::Greater) => Ordering::Greater,
        // Equal (including -0.0 vs 0.0) and NaN (None) fall through.
        _ => Ordering::Equal,
    };
    // Tie-break 0: sortingValue.
    match value_tie(a.sorting_value, b.sorting_value) {
        Ordering::Equal => {}
        other => return other,
    }
    // Tie-break 1: expansionValue.
    match value_tie(a.expansion_value, b.expansion_value) {
        Ordering::Equal => {}
        other => return other,
    }
    // Tie-break 2: door id.
    match a.door.id().cmp(&b.door.id()) {
        Ordering::Equal => {}
        other => return other,
    }
    // Tie-break 3: sectionNoOfDoor; full tie → Equal (the set dedups).
    a.section_no_of_door.cmp(&b.section_no_of_door)
}

impl PartialEq for FrontOrder {
    fn eq(&self, other: &Self) -> bool {
        compare(&self.0, &other.0) == std::cmp::Ordering::Equal
    }
}

/// [`Ord`] adapter so the element can live in a [`BTreeSet`].
/// Delegates to [`compare`] (see its doc for the NaN caveat).
#[derive(Clone, Debug)]
pub struct FrontOrder(pub MazeListElement);

impl Eq for FrontOrder {}

impl Ord for FrontOrder {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        compare(&self.0, &other.0)
    }
}

impl PartialOrd for FrontOrder {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Java `SortedSet<MazeListElement> mazeExpansionList`
/// (`MazeSearchEngine.java:52`, constructed `:84`) — a `TreeSet`
/// whose `add` override is fanout-aware and inert while
/// `ctrl.isFanout` is false (`:88`: the whole filter body is behind
/// `if (ctrl.isFanout && ...)`). The Rust front is a plain
/// [`BTreeSet`] over [`FrontOrder`] plus the optional
/// [`FanoutFrontGate`] — the port of that add override, installed by
/// the production composition (`RoutingBoardEngine::autoroute_connection`)
/// exactly when Java would close over a live gate, and `None`
/// (inert) in every detail-route search.
#[derive(Default)]
pub struct Front {
    set: BTreeSet<FrontOrder>,
    fanout_gate: Option<FanoutFrontGate>,
}

impl Front {
    /// Java `mazeExpansionList.add` — returns `false` when an EQUAL
    /// element is already present (the TreeSet dedup: the second add
    /// is a no-op).
    pub fn add(&mut self, element: MazeListElement) -> bool {
        self.set.insert(FrontOrder(element))
    }

    /// Installs the fanout frontier gate (Java's `TreeSet.add`
    /// override, `MazeSearchEngine.java:87-124`). Once installed, every
    /// `gated_add` filters; `add` stays the raw `super.add` for
    /// probe/pin worlds that construct a gateless front by hand.
    pub fn set_fanout_gate(&mut self, gate: FanoutFrontGate) {
        self.fanout_gate = Some(gate);
    }

    /// Java `mazeExpansionList.add` WITH the fanout filter — the
    /// engine's single entry point for front insertions. When no gate
    /// is installed this is exactly [`Self::add`].
    ///
    /// The two arguments are the per-element facts Java reads LIVE
    /// from the element's room/drill objects at add time:
    ///
    /// * `next_room_layer` — `element.nextRoom.getLayer()`, resolved
    ///   by the caller through the room registry; callers MUST pass
    ///   the layer whenever `element.next_room_key` is `Some` (Java's
    ///   room object always carries its layer). `None` = Java
    ///   `nextRoom == null` (target-door and drill elements).
    /// * `drill_location` — the `ExpansionDrill.location` when the
    ///   door IS a drill (page-grid or standalone; Java
    ///   `door instanceof ExpansionDrill`). `None` for every other
    ///   door kind.
    pub fn gated_add(
        &mut self,
        element: MazeListElement,
        next_room_layer: Option<i32>,
        drill_location: Option<&FloatPoint>,
    ) -> bool {
        if let Some(gate) = &self.fanout_gate
            && gate.rejects(&element, next_room_layer, drill_location)
        {
            return false;
        }
        self.set.insert(FrontOrder(element))
    }

    /// Java `isEmpty()`.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.set.is_empty()
    }

    /// Java `size()`.
    #[must_use]
    pub fn len(&self) -> usize {
        self.set.len()
    }

    /// The iterator-first pop (`occupyNextElement:327-329`): the
    /// SMALLEST element, removed.
    pub fn pop_first(&mut self) -> Option<MazeListElement> {
        self.set.pop_first().map(|order| order.0)
    }

    /// Java `clear()` (the spike phase runner reuses it).
    pub fn clear(&mut self) {
        self.set.clear();
    }

    /// Sorted iteration (spike capture: the front dump between pops).
    pub fn iter(&self) -> impl Iterator<Item = &MazeListElement> {
        self.set.iter().map(|order| &order.0)
    }
}

/// The fanout frontier gate — Java `MazeSearchEngine`'s `TreeSet.add`
/// override body (`MazeSearchEngine.java:87-124`), hoisted into a pure
/// data + one method. Java closes over `ctrl` and the board on every
/// add; the port snapshots the four constants the body reads (they are
/// invariant during one search: the pin anchor and layer are
/// `AutorouteControl` fields the fanout caller sets once, the escape
/// lengths resolve through the same `?:` ternaries, and
/// `communication.getResolution(UM)` never mutates mid-search) and
/// resolves the two per-element facts live at [`Front::gated_add`]
/// time, like Java reads them off the room/drill objects.
///
/// Gating is EXACTLY Java's: the whole body sits behind
/// `ctrl.isFanout && ctrl.fanoutStartPinCenter != null` — the port
/// installs the gate only when both hold, so no detail-route search
/// ever sees a filter (the T7 regression tripwire).
pub struct FanoutFrontGate {
    /// Java `ctrl.fanoutStartPinCenter.toFloat()`.
    pin_center: FloatPoint,
    /// Java `ctrl.fanoutStartPinLayer`.
    pin_layer: i32,
    /// Java `maxLen`: `maxEscapeLengthMm * 1000.0` when set, else the
    /// raw 3000.0-coordinate fallback (`:95-98`).
    max_escape_length: f64,
    /// Java `minLen`: `minEscapeLengthMm * 1000.0` when set, else the
    /// raw 500.0-coordinate fallback (`:110-113`).
    min_escape_length: f64,
    /// Java `board.communication.getResolution(Unit.UM)`.
    resolution: f64,
}

impl FanoutFrontGate {
    /// Java gate constant capture — the `?:` ternaries of
    /// `MazeSearchEngine.java:95-98`/`:110-113` resolve here once.
    /// `settings`
    /// is `ctrl.settings.fanout` (the group-null arm is banked: the
    /// resolver always materializes the group, see
    /// [`crate::control::RouterSettingsIr::fanout`]).
    pub fn new(
        settings: &FanoutSettingsIr,
        pin_center: &epic_geometry::point::Point,
        pin_layer: i32,
        resolution: f64,
    ) -> Self {
        FanoutFrontGate {
            pin_center: pin_center.to_float(),
            pin_layer,
            max_escape_length: settings
                .max_escape_length_mm
                .map_or(3000.0, |mm| mm * 1000.0),
            min_escape_length: settings
                .min_escape_length_mm
                .map_or(500.0, |mm| mm * 1000.0),
            resolution,
        }
    }

    /// The override body (`:88-123`): reject when the element's entry
    /// segment middle escapes beyond `maxEscapeLength` of the fanout
    /// pin ON THE PIN'S LAYER, or when a drill sits closer than
    /// `minEscapeLength`. Both arms are independent (Java's two `if`s
    /// fall through; rejection is `return false` — the element never
    /// enters the set).
    fn rejects(
        &self,
        element: &MazeListElement,
        next_room_layer: Option<i32>,
        drill_location: Option<&FloatPoint>,
    ) -> bool {
        // Java `:91-93`: onStartLayer = nextRoom != null &&
        // nextRoom.getLayer() == ctrl.fanoutStartPinLayer.
        let on_start_layer =
            element.next_room_key.is_some() && next_room_layer == Some(self.pin_layer);
        if on_start_layer {
            let entry_point = element.shape_entry.a.middle_point(&element.shape_entry.b);
            let dist = entry_point.distance(&self.pin_center);
            if dist > self.max_escape_length * self.resolution {
                return true;
            }
        }
        // Java `:109-119`: `element.door instanceof ExpansionDrill`
        // (page-grid and standalone drills are one Java class).
        if element.door.is_drill() {
            // The caller resolves the location for every drill door
            // (the callers' `drill_location` is `Some` exactly when
            // `door.is_drill()`); the `None` arm is a contract breach,
            // treated as no-rejection to keep the gate total.
            if let Some(location) = drill_location {
                let drill_dist = location.distance(&self.pin_center);
                if drill_dist < self.min_escape_length * self.resolution {
                    return true;
                }
            }
        }
        false
    }
}

// ---------------------------------------------------------------------------
// pins
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use epic_geometry::point::Point;
    use std::cell::Cell;

    /// A unique sort value per probe element — without one the front
    /// comparator would treat the probes as EQUAL and `gated_add`
    /// would report the SET DEDUP, not the gate verdict.
    fn next_seq() -> f64 {
        thread_local! {
            static SEQ: Cell<u64> = const { Cell::new(0) };
        }
        SEQ.with(|seq| {
            seq.set(seq.get() + 1);
            seq.get() as f64
        })
    }

    /// A gate over the pin anchor (0, 0) / layer 0 with the escape
    /// lengths set EXACTLY at the mm values (Java
    /// `maxEscapeLengthMm * 1000.0`); `None` arms exercise the raw
    /// 3000.0 / 500.0 fallbacks.
    fn gate(max: Option<f64>, min: Option<f64>, resolution: f64) -> FanoutFrontGate {
        let settings = FanoutSettingsIr {
            max_escape_length_mm: max,
            min_escape_length_mm: min,
            ..FanoutSettingsIr::default()
        };
        FanoutFrontGate::new(&settings, &Point::ZERO, 0, resolution)
    }

    /// A room-door element whose entry-segment MIDDLE sits at
    /// (`middle_x`, 0) — the max arm's measured point.
    fn door_elem(middle_x: f64) -> MazeListElement {
        MazeListElement {
            door: ExpandableObject::RoomDoor(ExpansionDoor::new(1, 2, 0)),
            section_no_of_door: 0,
            backtrack_door: None,
            section_no_of_backtrack_door: 0,
            expansion_value: next_seq(),
            sorting_value: next_seq(),
            next_room_key: Some(9),
            shape_entry: FloatLine::new(
                FloatPoint::new(middle_x, 0.0),
                FloatPoint::new(middle_x, 0.0),
            ),
            room_ripped: false,
            adjustment: Adjustment::None,
            already_checked: false,
            ripup_cost: 0,
        }
    }

    /// A drill element (next_room_key None — the max arm is silent
    /// for it) whose location sits at (`loc_x`, 0) — the min arm's
    /// measured point. Returns the element AND the location (the
    /// caller resolves it live, like the engine callers).
    fn drill_elem(loc_x: f64) -> (MazeListElement, FloatPoint) {
        let location = FloatPoint::new(loc_x, 0.0);
        let element = MazeListElement {
            door: ExpandableObject::Drill {
                row: 0,
                column: 0,
                d: 0,
                id: 1,
            },
            section_no_of_door: 0,
            backtrack_door: None,
            section_no_of_backtrack_door: 0,
            expansion_value: next_seq(),
            sorting_value: next_seq(),
            next_room_key: None,
            shape_entry: FloatLine::new(location, location),
            room_ripped: false,
            adjustment: Adjustment::None,
            already_checked: false,
            ripup_cost: 0,
        };
        (element, location)
    }

    /// Drives the gate through the GENERIC production insertion entry —
    /// the `FrontSink::add` impl (`maze/expansion_engine.rs`) is a
    /// one-line delegation to [`Front::gated_add`], and every
    /// production front insertion rides that trait path. A revert of
    /// the delegation to the raw `Front::add` would silently disable
    /// the gate everywhere (cerebrum 13c production-impl blind spot);
    /// routing the boundary probes through this generic helper keeps
    /// the delegation itself under pin.
    fn admits(
        front: &mut impl crate::maze::expansion_engine::FrontSink,
        element: MazeListElement,
        layer: Option<i32>,
        drill: Option<&FloatPoint>,
    ) -> bool {
        front.add(element, layer, drill)
    }

    /// NIT-Q2 (M7-T2): the DIRECT front-pop-order pin the T9 review
    /// banked. The docs (three sites: `Front::pop_first` here,
    /// `search_engine.rs`'s budget field, the pins.rs conservation
    /// doc) say pops run in ASCENDING `compare` order (sorting_value
    /// primary, expansion_value tie-break, door id, section) INDEPENDENT
    /// of insertion order; the original pin set corroborated this only
    /// transitively. This pin drives the REAL `Front` (BTreeSet over
    /// `FrontOrder`) with a scrambled insertion and reads back the
    /// exact pop sequence, including the tie-break pair and the
    /// duplicate-dedup row.
    #[test]
    fn front_pop_order_is_ascending_compare_independent_of_insertion_order() {
        // Door ids 10..13, all distinct sorting/expansion values chosen
        // so INSERTION order is the REVERSE of compare order for the
        // first three; the last two tie on sorting_value and separate
        // on expansion_value.
        let elem = |door_id: i32, sort: f64, exp: f64| MazeListElement {
            door: ExpandableObject::DrillPage {
                row: 0,
                column: 0,
                id: door_id,
            },
            section_no_of_door: 0,
            backtrack_door: None,
            section_no_of_backtrack_door: 0,
            expansion_value: exp,
            sorting_value: sort,
            next_room_key: None,
            shape_entry: FloatLine::new(FloatPoint::ZERO, FloatPoint::ZERO),
            room_ripped: false,
            adjustment: Adjustment::None,
            already_checked: false,
            ripup_cost: 0,
        };
        let mut front = Front::default();
        // Insertion order: (40, s=3) (30, s=2) (20, s=1) — reverse
        // order — then the tie pair (12, s=0, exp=9) (11, s=0, exp=5)
        // inserted BIGGER-tie first, then an exact duplicate of the
        // first insert (the set dedups it).
        for (id, sort, exp) in [(40, 3.0, 0.0), (30, 2.0, 0.0), (20, 1.0, 0.0)] {
            assert!(front.gated_add(elem(id, sort, exp), None, None));
        }
        assert!(front.gated_add(elem(12, 0.0, 9.0), None, None));
        assert!(front.gated_add(elem(11, 0.0, 5.0), None, None));
        assert!(
            !front.gated_add(elem(40, 3.0, 0.0), None, None),
            "the exact duplicate is a set-dedup false"
        );
        // The pops: ascending sorting_value; the tie pair comes out
        // expansion_value-ascending (11 before 12) despite being
        // inserted the other way.
        let mut popped = Vec::new();
        while let Some(element) = front.pop_first() {
            popped.push(element.door.id());
        }
        assert_eq!(
            popped,
            vec![11, 12, 20, 30, 40],
            "pop order is ascending compare (sort, then exp tie-break), \
             independent of insertion order"
        );
    }

    /// The max arm at its EXACT boundary (cerebrum 16): Java rejects
    /// on the STRICT `dist > maxLen * resolution` — the boundary
    /// distance itself is KEPT, one unit beyond is REJECTED, one
    /// unit inside is KEPT. maxLen = 4.5mm * 1000 = 4500.
    #[test]
    fn gate_max_arm_exact_boundary() {
        let mut front = Front::default();
        front.set_fanout_gate(gate(Some(4.5), Some(2.5), 1.0));
        assert!(
            admits(&mut front, door_elem(4500.0), Some(0), None),
            "exact 4500 kept"
        );
        assert!(
            !admits(&mut front, door_elem(4501.0), Some(0), None),
            "4501 rejected"
        );
        assert!(
            admits(&mut front, door_elem(4499.0), Some(0), None),
            "4499 kept"
        );
        assert_eq!(front.len(), 2, "exactly the two kept rows entered");
    }

    /// The min arm at its EXACT boundary: Java rejects a drill on
    /// the STRICT `drillDist < minLen * resolution` — the boundary
    /// is KEPT, one unit closer REJECTED, one unit farther KEPT.
    /// minLen = 2.5mm * 1000 = 2500. The far-drill row (8000) also
    /// proves the MAX arm is silent for drills (next_room_key None).
    #[test]
    fn gate_min_arm_exact_boundary() {
        let mut front = Front::default();
        front.set_fanout_gate(gate(Some(4.5), Some(2.5), 1.0));
        let (elem, loc) = drill_elem(2500.0);
        assert!(
            admits(&mut front, elem, None, Some(&loc)),
            "exact 2500 kept"
        );
        let (elem, loc) = drill_elem(2499.0);
        assert!(!admits(&mut front, elem, None, Some(&loc)), "2499 rejected");
        let (elem, loc) = drill_elem(2501.0);
        assert!(admits(&mut front, elem, None, Some(&loc)), "2501 kept");
        let (elem, loc) = drill_elem(8000.0);
        assert!(
            admits(&mut front, elem, None, Some(&loc)),
            "far drill kept — the max arm does not fire for next_room None"
        );
    }

    /// The `None` settings arms: maxLen falls to the RAW 3000.0
    /// coordinate fallback and minLen to 500.0 (Java `:108-121`
    /// ternaries), scaled by the resolution like any other arm.
    #[test]
    fn gate_none_settings_fallbacks() {
        let mut front = Front::default();
        front.set_fanout_gate(gate(None, None, 1.0));
        assert!(
            admits(&mut front, door_elem(3000.0), Some(0), None),
            "exact 3000 kept"
        );
        assert!(
            !admits(&mut front, door_elem(3001.0), Some(0), None),
            "3001 rejected"
        );
        let (elem, loc) = drill_elem(500.0);
        assert!(admits(&mut front, elem, None, Some(&loc)), "exact 500 kept");
        let (elem, loc) = drill_elem(499.0);
        assert!(!admits(&mut front, elem, None, Some(&loc)), "499 rejected");
    }

    /// The resolution factor: with resolution 2.0 the thresholds are
    /// maxLen * 2 = 9000 and the fallback minLen * 2 = 1000 — a
    /// resolution-swallowing mutant (thresholds pre-divided) dies on
    /// the 9000/999 rows.
    #[test]
    fn gate_resolution_scaling() {
        let mut front = Front::default();
        front.set_fanout_gate(gate(Some(4.5), None, 2.0));
        assert!(
            admits(&mut front, door_elem(9000.0), Some(0), None),
            "exact 9000 kept"
        );
        assert!(
            !admits(&mut front, door_elem(9001.0), Some(0), None),
            "9001 rejected"
        );
        let (elem, loc) = drill_elem(1000.0);
        assert!(
            admits(&mut front, elem, None, Some(&loc)),
            "exact 1000 kept"
        );
        let (elem, loc) = drill_elem(999.0);
        assert!(!admits(&mut front, elem, None, Some(&loc)), "999 rejected");
    }

    /// The two max-arm GUARDS: the arm fires only when the next
    /// room EXISTS and sits ON the pin's layer — a next room on
    /// another layer (Java `nextRoom.getLayer() != fanoutStartPinLayer`)
    /// or a null next room never rejects, however far the middle.
    #[test]
    fn gate_max_arm_layer_and_room_guards() {
        let mut front = Front::default();
        front.set_fanout_gate(gate(Some(4.5), Some(2.5), 1.0));
        assert!(
            admits(&mut front, door_elem(4501.0), Some(1), None),
            "next room on layer 1 ≠ pin layer 0: kept"
        );
        assert!(
            admits(&mut front, door_elem(4501.0), None, None),
            "null next room: kept"
        );
    }

    /// The tripwire face: a gateless front (every detail-route
    /// search) accepts anything through `gated_add` — the filter is
    /// installed ONLY for fanout searches.
    #[test]
    fn gateless_front_is_inert() {
        let mut front = Front::default();
        assert!(admits(&mut front, door_elem(999_999.0), Some(0), None));
        let (elem, loc) = drill_elem(0.0);
        assert!(admits(&mut front, elem, None, Some(&loc)));
        assert_eq!(front.len(), 2);
    }
}
