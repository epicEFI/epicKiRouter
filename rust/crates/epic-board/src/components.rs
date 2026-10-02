//! Components, the board-side library mirror, and PIN PLACEMENT
//! RESOLUTION (M2 Task 3).
//!
//! Java anchors: `board/model/structure/Component.java` (ctor
//! normalization `:54-79`, the `rotate` trap `:127-148`),
//! `board/model/structure/Components.java` (the container + its own
//! `UndoableObjects`), `board/model/items/Pin.java`
//! (`relativeLocation` `:64-89`, `getCenter` `:91-120`, `getShape`
//! `:165-242`, `getPadstackLayer` `:248-257`),
//! `board/model/items/DrillItem.java` (`firstLayer` `:162-172`), and
//! `core/library/Padstack.java` (`fromLayer`/`toLayer` `:137-153`).
//!
//! ## Where pin resolution lives (the plan left this open)
//!
//! [`pin_relative_location`] / [`pin_center`] live HERE, not in
//! `items/`: they consume only the components table, the flip-style
//! flag, and the package/padstack tables — all of which this module
//! owns — and the item side contributes nothing but
//! `(component_id, pin_index)`. The `Board::pin_center(item_id)`
//! convenience on the arena is a thin delegate.
//!
//! ## Ground truth (jar spike, pins quote it)
//!
//! `rust/harness/oracle/PinResolutionSpike.java` drove the frozen jar
//! over `KiCad_10_demos/StickHub.dsn` — NO tier-A fixture has a
//! non-90-degree component rotation (all 11 scanned), so the
//! smallest tier-B board that has one is the spike fixture; its
//! capture `/tmp/epic-t3-pins.out` pins the 90-degree exact branch,
//! the non-90 float branch, the back-side mirror-BEFORE branch (12
//! distinct side/rotation keys), and a SYNTHETIC pad-shape correction
//! (no corpus fixture triggers `Pin.java:114` — every fixture pad
//! shape contains its raw center — so the spike constructed an
//! off-center pad through the real board API). Phase 6 (the T3 review
//! fix round, capture `/tmp/epic-t3-pins-final.out`) adds the
//! flip-style re-query (`FLIPSTYLE`: the same C23 pin under
//! rotate-first — mirror-AFTER), a MULTI-LAYER synthetic under
//! rotate-first (`MLPIN`: 2-layer padstack with distinct asymmetric
//! per-layer boxes, pin rotation 90 — pins the back-side
//! padstack-layer remap, the pin-rotation arm, and the center
//! correction through the full chain), and the same geometry FRESH
//! under mirror-first (`SPIKE3_MIRRORFIRST`).
//!
//! ## Math.toRadians bit-parity
//!
//! Java `Math.toRadians(a) = a / 180.0 * PI` (divide first); Rust's
//! `f64::to_radians` multiplies by the precomputed `PI / 180.0`,
//! which can differ by 1 ulp. Every Java `Math.toRadians` call site
//! here goes through [`java_to_radians`] instead.
//!
//! ## Documented divergences (none observable in the M2 gates)
//!
//! - `Pin.getCenter` caches via `setCenter` and re-returns the stored
//!   value (`Pin.java:97-119`); the stored value IS the computed one,
//!   so the port computes fresh each call — the cache is a Java
//!   performance detail with no observable read.
//! - `Pin.getShape` likewise MEMOIZES the whole shape array on the
//!   FIRST call (`precalculatedShapes`, `Pin.java:170-236` — "all
//!   shapes have to be calculated at once, because otherwise
//!   calculation of fromLayer and toLayer may not be correct"); the
//!   port computes fresh each call. The two differ only when the
//!   flip-style flag or the component table changes between calls of
//!   the SAME pin — never through any M2 gate (the flag is parse-set
//!   and no board flow mutates it mid-board). The spike's Phase 6
//!   leans on exactly this: its mirror-first pin is inserted FRESH
//!   because re-querying the rotate-first pin would return Java's
//!   cached shapes (verified: the re-query produced byte-identical
//!   corners — it pins the cache, not the chain).
//! - `Components` keeps its OWN `component_arr` mirror beside the
//!   undo stack ([`crate::undo::ComponentsUndoStack`]): Java's
//!   `componentArr` Vector holds the LIVE objects (the undo list
//!   holds clones created by `saveForUndo`); the port clones at
//!   `save_for_undo` instead and re-syncs the arr from the visible
//!   undo entries after undo/redo
//!   (`restoreComponentArrFromUndoList`, `Components.java:133-146`) —
//!   the same observable state at the same points.
//! - The arr NEVER shrinks and `get(id)` reads the ARR, not the undo
//!   list: a component created above `stack_level` that
//!   `disableRedo` drops from the undo map is still returned by
//!   `get(id)` in Java (nothing ever removes from `componentArr`) —
//!   the port reproduces exactly that.

use epic_dsn::sink::{ComponentIr, FixedStateIr, ImageIr, ImagePinIr, PadstackIr};
use epic_geometry::direction::Direction;
use epic_geometry::float_point::FloatPoint;
use epic_geometry::int_point::IntPoint;
use epic_geometry::int_vector::IntVector;
use epic_geometry::line::Line;
use epic_geometry::point::Point;
use epic_geometry::polyline::Polyline;
use epic_geometry::tile_shape::TileShape;
use epic_geometry::vector::Vector;

use crate::board::board_shape_from_ir;
use crate::items::BoardShape;
use crate::undo::ComponentsUndoStack;

/// Java `Math.toRadians(angdeg)` (`StrictMath`: `angdeg / 180.0 * PI`)
/// — division FIRST; Rust's `to_radians()` differs by up to 1 ulp.
/// `pub(crate)`: the item-geometry modules ([`crate::items`]) run the
/// same anchored conversions (`ObstacleArea.getArea`
/// `ObstacleArea.java:130`, `ComponentOutline.getArea`
/// `ComponentOutline.java:203`).
pub(crate) fn java_to_radians(degrees: f64) -> f64 {
    degrees / 180.0 * std::f64::consts::PI
}

// ---------------------------------------------------------------------------
// The board-side library mirror
// ---------------------------------------------------------------------------

/// Java `core.library.Padstack` — the board-side read surface: the
/// per-layer shape array plus the two placement flags. The layer-span
/// helpers are RE-DERIVED here because epic-dsn's IR helpers are
/// private to that crate; the derivations are the anchored Java
/// algorithms (`Padstack.java:137-153`), and `from_ir` copies the
/// already-converted parse shapes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoardPadstack {
    /// Java `Padstack.name`.
    pub name: String,
    /// Java `Padstack.shapes` — `ConvexShape[layerCount]`, indexed by
    /// 0-based layer; length == the board layer count; `None` is a
    /// Java null slot.
    pub shapes: Vec<Option<BoardShape>>,
    /// Java `Padstack.is_drillable`.
    pub drillable: bool,
    /// Java `Padstack.placed_absolute`.
    pub placed_absolute: bool,
    /// Java `Padstack.holeOnly` (`Padstack.java:41`) — a PUBLIC field
    /// with NO writer anywhere in the Java main tree (verified by grep:
    /// only the declaration exists), so it is `false` for every parsed
    /// board; [`crate::tree_shapes`] reads it for the
    /// `drillHoleClearanceDelta` copper-radius arm
    /// (`ShapeSearchTree.java:1049-1050`).
    pub hole_only: bool,
}

impl BoardPadstack {
    /// Java `Padstack.boardLayerCount()` = `shapes.length`.
    #[must_use]
    pub fn board_layer_count(&self) -> usize {
        self.shapes.len()
    }

    /// Java `Padstack.fromLayer()` (`:137-143`): the FIRST layer with
    /// a shape; `shapes.length` when every slot is null.
    #[must_use]
    pub fn from_layer(&self) -> usize {
        self.shapes
            .iter()
            .position(|shape| shape.is_some())
            .unwrap_or(self.shapes.len())
    }

    /// Java `Padstack.toLayer()` (`:146-152`): the LAST layer with a
    /// shape; -1 when every slot is null.
    #[must_use]
    pub fn to_layer(&self) -> i32 {
        self.shapes
            .iter()
            .rposition(|shape| shape.is_some())
            .map_or(-1, |index| index as i32)
    }

    /// Java `Padstack.getShape(index)` — the RAW per-layer shape,
    /// before any pin transform.
    #[must_use]
    pub fn get_shape(&self, index: usize) -> Option<&BoardShape> {
        self.shapes.get(index).and_then(Option::as_ref)
    }

    /// Java `Padstack.getSmallestRadius()` (`Padstack.java:114-125`): the
    /// minimum of `min(bbox.width, bbox.height) / 2` over the NON-NULL
    /// shapes (all layers), `0.0` when every slot is null
    /// (`Double.MAX_VALUE` sentinel kept unreturned).
    #[must_use]
    pub fn smallest_radius(&self) -> f64 {
        let mut min_radius = f64::MAX;
        for shape in self.shapes.iter().flatten() {
            let bounds = shape.bounding_box();
            let radius = f64::from(bounds.width().min(bounds.height())) / 2.0;
            if radius < min_radius {
                min_radius = radius;
            }
        }
        if min_radius == f64::MAX {
            0.0
        } else {
            min_radius
        }
    }

    /// Java `Padstack.getDrillRadius()` (`Padstack.java:72-112`): the
    /// drill radius in BOARD UNITS, parsed from the padstack NAME when it
    /// carries the `..._<outer>:<drill>_...` KiCad pattern —
    ///
    /// * `drillStr` = the substring after `:` up to the next `_` (or the
    ///   name end), stripped of everything but digits and `.`,
    /// * `outerStr` = the substring between the LAST `_` BEFORE the colon
    ///   and the colon, stripped the same way,
    /// * success (`outerDia > 0` AND `smallestRadius > 0`) returns
    ///   `smallestRadius * drillDia / outerDia` — the ratio is unit-free,
    ///   so the resolution-scaled shapes give the board-unit radius,
    /// * every other path (no colon, unparseable numbers, zero radius)
    ///   falls through to `smallestRadius * 0.45`.
    ///
    /// DIVERGENCE (cache artifact, unobservable): Java memoizes into
    /// `cachedDrillRadius` (`:44`, written at `:87-89`/`:108-110` before
    /// any value-dependent branch); the port recomputes per call — same
    /// value every time, no branch reads the cache.
    ///
    /// Captured pins (TreeShapesSpike against the frozen jar,
    /// `/tmp/epic-t6-treeshapes*.out`): `Via[0-1]_600:300_um` (circle
    /// r=3000) -> 1500.0; `Via[0-3]_450:250_um` (r=2250) -> 1250.0;
    /// `Round[A]Pad_1700_um` (r=8500) -> 3825.0; `Rect[T]Pad_
    /// 2650x1000_um` (box 26500x10000) -> 2250.0; the synthetic
    /// `spike_hole_600:300` (r=3000) -> 1500.0.
    #[must_use]
    pub fn drill_radius(&self) -> f64 {
        if let Some(colon_index) = self.name.find(':') {
            // drillStr: after the colon up to the next '_' (or the name
            // end) — `name.indexOf('_', colonIndex)` with the
            // `underscoreIndex > colonIndex` condition.
            let after_colon = &self.name[colon_index + 1..];
            let drill_part = match after_colon.split_once('_') {
                Some((before, _)) => before,
                None => after_colon,
            };
            if let Some(drill_dia) = parse_name_number(drill_part) {
                // The outer diameter: between the LAST '_' before the
                // colon and the colon (`name.lastIndexOf('_', colonIndex)`
                // — Java searches [0, colonIndex), so an underscore AT
                // the colon position is impossible anyway). `> 0`
                // guards both numbers (Java `:101-104`).
                let outer_dia = self.name[..colon_index]
                    .rfind('_')
                    .and_then(|last_underscore| {
                        parse_name_number(&self.name[last_underscore + 1..colon_index])
                    })
                    .filter(|&dia| dia > 0.0);
                if let Some(outer_dia) = outer_dia {
                    let outer_radius = self.smallest_radius();
                    if outer_radius > 0.0 {
                        return outer_radius * (drill_dia / outer_dia);
                    }
                }
            }
        }
        self.smallest_radius() * 0.45
    }
}

/// The `replaceAll("[^0-9.]", "")` + `Double.parseDouble` pair of
/// `Padstack.getDrillRadius` (`Padstack.java:83-95`): `None` is Java's
/// `NumberFormatException` (an empty or multi-dot string), which the
/// caller turns into the `0.45` fallback.
fn parse_name_number(raw: &str) -> Option<f64> {
    let stripped: String = raw
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    stripped.parse::<f64>().ok()
}

/// Java `Package.Pin` (`core/library/Package.java`: `name`,
/// `padstackId`, `relativeLocation`, `rotationInDegree`).
#[derive(Clone, Debug, PartialEq)]
pub struct BoardPackagePin {
    /// Java `Pin.name`.
    pub name: String,
    /// Java `Pin.padstackId` — the 1-based padstack number.
    pub padstack_no: i32,
    /// Java `Pin.relativeLocation` — image-relative (the IR stores the
    /// reader's rounded IntPoint; the Java `Vector` for a parsed pin is
    /// always integral for the same reason).
    pub rel_location: IntPoint,
    /// Java `Pin.rotationInDegree`.
    pub rotation: f64,
}

/// Java `Package` — the board-side read subset: identity + pins. The
/// outline/keepout arrays stay IR-side (they were already consumed
/// into ITEMS by [`crate::board::Board::from_ses_board`]; no
/// board-side reader re-reads them through the library).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoardPackage {
    /// Java `Package.name` (post-dedup).
    pub name: String,
    /// Java `Package.pins`, file order (`getPin(index)` is 0-based).
    pub pins: Vec<BoardPackagePin>,
}

impl BoardPackage {
    /// Java `Package.getPin(index)` — 0-based, `None` out of range.
    #[must_use]
    pub fn get_pin(&self, index: i32) -> Option<&BoardPackagePin> {
        usize::try_from(index)
            .ok()
            .and_then(|index| self.pins.get(index))
    }
}

/// Java `BoardLibrary`'s padstack + package tables, mirrored as
/// 1-based positional registries (Java `Padstacks`/`Packages` insert
/// in file order; id = position + 1).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoardLibrary {
    /// Java `library.padstacks` — 1-based registry order.
    pub padstacks: Vec<BoardPadstack>,
    /// Java `library.packages` — 1-based registry order.
    pub packages: Vec<BoardPackage>,
}

impl BoardLibrary {
    /// Java `Padstacks.get(padstackId)` — 1-based; `None` on a miss
    /// (Java would return null and the caller warns). Precondition:
    /// `padstack_no >= 1` — id 0 underflows the `index - 1` (Java:
    /// `elementAt(-1)` throws AIOOBE); ids are 1-based by
    /// construction (the registry appends).
    #[must_use]
    pub fn padstack(&self, padstack_no: i32) -> Option<&BoardPadstack> {
        usize::try_from(padstack_no)
            .ok()
            .and_then(|index| self.padstacks.get(index - 1))
    }

    /// `Packages.get(packageNo)` — 1-based. Precondition:
    /// `package_no >= 1` — id 0 underflows the `index - 1` (same
    /// AIOOBE-in-Java shape as [`BoardLibrary::padstack`]).
    #[must_use]
    pub fn package(&self, package_no: i32) -> Option<&BoardPackage> {
        usize::try_from(package_no)
            .ok()
            .and_then(|index| self.packages.get(index - 1))
    }

    /// The IR conversion: shapes cross the crate boundary through
    /// [`board_shape_from_ir`] (the same variant-matched conversion the
    /// item conversion uses), pins map 1:1.
    #[must_use]
    pub fn from_ir(padstacks: &[PadstackIr], packages: &[ImageIr]) -> Self {
        Self {
            padstacks: padstacks
                .iter()
                .map(|padstack| BoardPadstack {
                    name: padstack.name.clone(),
                    shapes: padstack
                        .shapes
                        .iter()
                        .map(|shape| shape.as_ref().map(board_shape_from_ir))
                        .collect(),
                    drillable: padstack.drillable,
                    placed_absolute: padstack.placed_absolute,
                    // The IR carries no hole-only flag — Java's field is
                    // never written by the parse either (module docs of
                    // the field).
                    hole_only: false,
                })
                .collect(),
            packages: packages
                .iter()
                .map(|package| BoardPackage {
                    name: package.name.clone(),
                    pins: package.pins.iter().map(package_pin_from_ir).collect(),
                })
                .collect(),
        }
    }
}

/// The [`BoardPackagePin`] conversion.
fn package_pin_from_ir(pin: &ImagePinIr) -> BoardPackagePin {
    BoardPackagePin {
        name: pin.name.clone(),
        padstack_no: pin.padstack_no,
        rel_location: pin.rel_location,
        rotation: pin.rotation,
    }
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

/// Java `board.model.structure.Component` — the placement record.
/// Package references are the 1-based registry numbers
/// ([`BoardLibrary`]); `getPackage()` resolves
/// `is_front ? package_front : package_back` (`Component.java:238-246`).
#[derive(Clone, Debug, PartialEq)]
pub struct Component {
    /// Java `Component.name` — the placed instance name (e.g. `U1`).
    pub name: String,
    /// Java `libPackageFront` as a 1-based package number.
    pub package_front: i32,
    /// Java `libPackageBack` as a 1-based package number.
    pub package_back: i32,
    /// Java `Component.location` — `None` = unplaced (a null coor still
    /// ADDS the component; only the pin/board insertion is skipped,
    /// `Network.java:969-971`).
    pub location: Option<IntPoint>,
    /// Java `Component.rotationInDegree` — normalized to `[0, 360)` by
    /// the constructor's while loops.
    pub rotation_in_degree: f64,
    /// Java `Component.onFront`.
    pub is_front: bool,
    /// Java `Component.positionFixed`.
    pub position_fixed: bool,
}

impl Component {
    /// Java `Component(...)` (`Component.java:54-79`): the rotation is
    /// normalized by WHILE LOOPS (`>= 360` subtracts, `< 0` adds) —
    /// NOT a `%` operation: `-45.5 -> 314.5`, `720.5 -> 0.5` (a `%`
    /// would keep `-45.5`).
    #[must_use]
    pub fn new(
        name: &str,
        location: Option<IntPoint>,
        rotation_in_degree: f64,
        is_front: bool,
        package_front: i32,
        package_back: i32,
        position_fixed: bool,
    ) -> Self {
        let mut rotation = rotation_in_degree;
        while rotation >= 360.0 {
            rotation -= 360.0;
        }
        while rotation < 0.0 {
            rotation += 360.0;
        }
        Self {
            name: name.to_string(),
            package_front,
            package_back,
            location,
            rotation_in_degree: rotation,
            is_front,
            position_fixed,
        }
    }

    /// Java `Component.placedOnFront()`.
    #[must_use]
    pub fn placed_on_front(&self) -> bool {
        self.is_front
    }

    /// Java `Component.getPackage()` (`:238-246`): the front or back
    /// package number by side.
    #[must_use]
    pub fn package_no(&self) -> i32 {
        if self.is_front {
            self.package_front
        } else {
            self.package_back
        }
    }

    /// Java `Component.translateBy(Vector)` (`:104-108`) — a no-op on
    /// an unplaced component. The port keeps the location integral: a
    /// rational translation result (unreachable from the parse, whose
    /// pin/package locations are rounded IntPoints) is rounded back.
    pub fn translate_by(&mut self, vector: &Vector) {
        if let Some(location) = self.location {
            let translated = Point::Int(location).translate_by(vector);
            self.location = Some(match translated {
                Point::Int(point) => point,
                Point::Rational(_) => translated.to_float().round(),
            });
        }
    }

    /// Java `Component.turn90Degree(factor, pole)` (`:110-125`):
    /// `rotation += factor * 90` (re-normalized) and the location
    /// turns EXACTLY (integer geometry).
    pub fn turn_90_degree(&mut self, factor: i32, pole: &IntPoint) {
        if factor == 0 {
            return;
        }
        self.rotation_in_degree += f64::from(factor) * 90.0;
        while self.rotation_in_degree >= 360.0 {
            self.rotation_in_degree -= 360.0;
        }
        while self.rotation_in_degree < 0.0 {
            self.rotation_in_degree += 360.0;
        }
        if let Some(location) = self.location {
            self.location = Some(
                match Point::Int(location).turn_90_degree(factor, &Point::Int(*pole)) {
                    Point::Int(point) => point,
                    // turn90Degree of an IntPoint around an IntPoint is
                    // always integral; the rational arm is unreachable.
                    Point::Rational(_) => unreachable!("integer turn cannot go rational"),
                },
            );
        }
    }

    /// Java `Component.rotate(angleInDegree, pole, flipStyleRotateFirst)`
    /// (`:127-148`) — **THE T68 TRAP**: on the BACK side with the
    /// rotate-first flip style the ROTATION advances by
    /// `360 - angle` while the LOCATION rotates by the ORIGINAL
    /// `angle` (`Math.toRadians(angleInDegree)`, not the turn angle).
    /// Both sides are pinned in the tests.
    pub fn rotate(&mut self, angle_in_degree: f64, pole: &IntPoint, flip_style_rotate_first: bool) {
        if angle_in_degree == 0.0 {
            return;
        }
        let mut turn_angle = angle_in_degree;
        if flip_style_rotate_first && !self.placed_on_front() {
            // take care of the order of mirroring and rotating on the
            // back side of the board (Component.java:133-135)
            turn_angle = 360.0 - angle_in_degree;
        }
        self.rotation_in_degree += turn_angle;
        while self.rotation_in_degree >= 360.0 {
            self.rotation_in_degree -= 360.0;
        }
        while self.rotation_in_degree < 0.0 {
            self.rotation_in_degree += 360.0;
        }
        if let Some(location) = self.location {
            let pole_float = FloatPoint {
                x: f64::from(pole.x),
                y: f64::from(pole.y),
            };
            self.location = Some(
                Point::Int(location)
                    .to_float()
                    .rotate(java_to_radians(angle_in_degree), &pole_float)
                    .round(),
            );
        }
    }

    /// Java `Component.changeSide(pole)` (`:153-156`): flips the side
    /// and mirrors the location at the vertical line through the pole.
    pub fn change_side(&mut self, pole: &IntPoint) {
        self.is_front = !self.is_front;
        if let Some(location) = self.location {
            self.location = Some(
                match Point::Int(location).mirror_vertical(&Point::Int(*pole)) {
                    Point::Int(point) => point,
                    Point::Rational(_) => unreachable!("integer mirror cannot go rational"),
                },
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Components container
// ---------------------------------------------------------------------------

/// Java `board.model.structure.Components` — the live table plus its
/// OWN undo stack (T63: independent of the board's item stack).
///
/// Component ids are 1-BASED (`componentArr.size() + 1` at add time)
/// and NEVER reused: the arr never shrinks, `undo`/`redo` re-sync
/// slots BY INDEX (`restoreComponentArrFromUndoList`,
/// `Components.java:133-146`), and `get(id)` reads the ARR even for
/// components the undo list no longer knows (module docs).
#[derive(Clone, Debug, Default)]
pub struct Components {
    /// Java `componentArr` — slot `i` holds id `i + 1`, forever.
    component_arr: Vec<Component>,
    /// Java `undoList` — the components-side `UndoableObjects`.
    undo_list: ComponentsUndoStack<Component>,
    /// Java `flipStyleRotateFirst` — back-side components rotate
    /// before mirroring when true, else mirror before rotating.
    flip_style_rotate_first: bool,
}

impl Components {
    /// An empty table (mirror-first flip style — Java's field default;
    /// the parse sets it from `(flip_style ...)`).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Java `Components.add(...)` (`:30-54`): id =
    /// `componentArr.size() + 1` (1-based, monotone), a slot in the
    /// arr, and an insert at the CURRENT undo level.
    pub fn add(&mut self, component: Component) -> u32 {
        let id = self.undo_list.add(component.clone());
        self.component_arr.push(component);
        debug_assert_eq!(id as usize, self.component_arr.len());
        id
    }

    /// Java `Components.get(int componentId)` (`:88-94`) — reads the
    /// ARR (`componentArr.elementAt(id - 1)`); ids run 1..=count.
    /// Precondition: `component_id >= 1` — id 0 underflows the
    /// `id - 1` index (Java: `elementAt(-1)` throws AIOOBE);
    /// unreachable, callers pass add()-issued ids.
    #[must_use]
    pub fn get(&self, component_id: u32) -> Option<&Component> {
        self.component_arr.get(component_id as usize - 1)
    }

    /// Java `Components.get(String name)` (`:75-82`): first
    /// case-SENSITIVE `equals` match over the arr (INCLUDING
    /// components the undo list has dropped — the arr is the truth).
    #[must_use]
    pub fn get_by_name(&self, name: &str) -> Option<(u32, &Component)> {
        self.component_arr
            .iter()
            .enumerate()
            .find(|(_, component)| component.name == name)
            .map(|(index, component)| (index as u32 + 1, component))
    }

    /// Java `Components.count()` = the arr size (never shrinks).
    #[must_use]
    pub fn count(&self) -> u32 {
        self.component_arr.len() as u32
    }

    /// Java `Components.getAll()` — the arr in id order.
    pub fn iter(&self) -> impl Iterator<Item = (u32, &Component)> {
        self.component_arr
            .iter()
            .enumerate()
            .map(|(index, component)| (index as u32 + 1, component))
    }

    /// Java `Components.getFlipStyleRotateFirst()`.
    #[must_use]
    pub fn flip_style_rotate_first(&self) -> bool {
        self.flip_style_rotate_first
    }

    /// Java `Components.setFlipStyleRotateFirst(value)`.
    pub fn set_flip_style_rotate_first(&mut self, value: bool) {
        self.flip_style_rotate_first = value;
    }

    /// Java `Components.generateSnapshot()`.
    pub fn generate_snapshot(&mut self) {
        self.undo_list.generate_snapshot();
    }

    /// The components-side `stackLevel` (`Components.undoList.stackLevel`
    /// — Java reads the private field reflectively in the Task 14
    /// oracle; the port exposes it).
    #[must_use]
    pub fn stack_level(&self) -> usize {
        self.undo_list.stack_level()
    }

    /// Java `Components.undo(observers)` (`:113-119`): the stack undo,
    /// then the arr re-sync from the VISIBLE undo entries.
    pub fn undo(&mut self) -> bool {
        if self.undo_list.undo().is_none() {
            return false;
        }
        self.sync_component_arr_from_undo_list();
        true
    }

    /// Java `Components.redo(observers)` (`:121-128`).
    pub fn redo(&mut self) -> bool {
        if self.undo_list.redo().is_none() {
            return false;
        }
        self.sync_component_arr_from_undo_list();
        true
    }

    /// Java `restoreComponentArrFromUndoList` (`:133-146`): every
    /// visible undo entry lands back in its OWN slot (`id - 1`).
    fn sync_component_arr_from_undo_list(&mut self) {
        for (id, component) in self.undo_list.iter_visible() {
            if let Some(slot) = self.component_arr.get_mut(id as usize - 1) {
                *slot = component.clone();
            }
        }
    }

    /// Java `Components.move(componentId, vector)` (`:148-156`):
    /// `saveForUndo` then mutate the live component. (`move` is a Rust
    /// keyword, hence the raw identifier.)
    pub fn r#move(&mut self, component_id: u32, vector: &Vector) {
        self.undo_list.save_for_undo(component_id);
        if let Some(component) = self.component_arr.get_mut(component_id as usize - 1) {
            component.translate_by(vector);
        }
        self.sync_live_undo_value(component_id);
    }

    /// Java `Components.turn90Degree(componentId, factor, pole)`
    /// (`:158-166`).
    pub fn turn_90_degree(&mut self, component_id: u32, factor: i32, pole: &IntPoint) {
        self.undo_list.save_for_undo(component_id);
        if let Some(component) = self.component_arr.get_mut(component_id as usize - 1) {
            component.turn_90_degree(factor, pole);
        }
        self.sync_live_undo_value(component_id);
    }

    /// Java `Components.rotate(componentId, rotationInDegree, pole)`
    /// (`:172-176`) — passes the table's flip-style flag through to
    /// [`Component::rotate`].
    pub fn rotate(&mut self, component_id: u32, angle_in_degree: f64, pole: &IntPoint) {
        self.undo_list.save_for_undo(component_id);
        let flip_style = self.flip_style_rotate_first;
        if let Some(component) = self.component_arr.get_mut(component_id as usize - 1) {
            component.rotate(angle_in_degree, pole, flip_style);
        }
        self.sync_live_undo_value(component_id);
    }

    /// Java `Components.changeSide(componentId, pole)` (`:182-186`).
    pub fn change_side(&mut self, component_id: u32, pole: &IntPoint) {
        self.undo_list.save_for_undo(component_id);
        if let Some(component) = self.component_arr.get_mut(component_id as usize - 1) {
            component.change_side(pole);
        }
        self.sync_live_undo_value(component_id);
    }

    /// After a save-then-mutate, the arr's mutated value must land in
    /// the undo stack's CURRENT node too: Java's `componentArr` slot and
    /// the undo map entry reference ONE `Component` object, so the
    /// mutation is visible through both; the port clones into the map
    /// node here. (Missing this makes undo/redo a no-op — pinned by
    /// `components_container_ids_lookup_and_undo_resync`.)
    fn sync_live_undo_value(&mut self, component_id: u32) {
        let Some(component) = self.component_arr.get(component_id as usize - 1) else {
            return;
        };
        let mutated = component.clone();
        if let Some(live) = self.undo_list.value_mut(component_id) {
            *live = mutated;
        }
    }

    /// The IR conversion: every IR component in table order (id =
    /// position + 1), with the flip-style flag from the parse
    /// (`metadata.flip_style == "rotate_first"` — the same predicate
    /// the epic-dsn obstacle transform uses, `ses_board.rs:356`).
    #[must_use]
    pub fn from_ir(components: &[ComponentIr], flip_style_rotate_first: bool) -> Self {
        let mut table = Components::new();
        table.flip_style_rotate_first = flip_style_rotate_first;
        for component in components {
            table.add(Component::new(
                &component.name,
                component.location,
                component.rotation,
                component.is_front,
                component.package_front,
                component.package_back,
                // Network.java:974-976: position-fixed placements map to
                // SYSTEM_FIXED, everything else to UNFIXED — the boolean
                // Component field is exactly the fixed side of that.
                matches!(component.fixed, FixedStateIr::SystemFixed),
            ));
        }
        table
    }
}

// ---------------------------------------------------------------------------
// Pin placement resolution (Pin.java)
// ---------------------------------------------------------------------------

/// Resolves the package pin a board pin refers to: component ->
/// package (by side) -> pin (by index).
fn resolve_package_pin<'a>(
    components: &'a Components,
    library: &'a BoardLibrary,
    component_id: u32,
    pin_index: i32,
) -> Option<(&'a Component, &'a BoardPackagePin)> {
    let component = components.get(component_id)?;
    let package = library.package(component.package_no())?;
    let pin = package.get_pin(pin_index)?;
    Some((component, pin))
}

/// Java `Pin.relativeLocation()` (`Pin.java:64-89`) — the pin location
/// relative to its component origin, in three stages:
///
/// 1. mirror-BEFORE on the back side when the flip style is NOT
///    rotate-first (`mirrorAtYAxis`, `:73-75`),
/// 2. the component rotation: 90-multiples take the EXACT
///    `turn90Degree((int) rotation / 90)` branch (`:76-81`), everything
///    else the float branch `to_float().rotate(to_radians(rot),
///    ZERO).round().difference_by(ZERO)` (`:82-87`),
/// 3. mirror-AFTER on the back side when the flip style IS
///    rotate-first (`:88-90`).
///
/// Returns `None` when the component/package/pin chain does not
/// resolve (Java warns and dies on a null; the parse never stores a
/// dangling pin).
#[must_use]
pub fn pin_relative_location(
    components: &Components,
    library: &BoardLibrary,
    component_id: u32,
    pin_index: i32,
) -> Option<Vector> {
    let (component, package_pin) =
        resolve_package_pin(components, library, component_id, pin_index)?;
    let mut rel_location = Vector::Int(IntVector::new(
        package_pin.rel_location.x,
        package_pin.rel_location.y,
    ));
    let component_rotation = component.rotation_in_degree;
    if !component.placed_on_front() && !components.flip_style_rotate_first() {
        rel_location = rel_location.mirror_at_y_axis();
    }
    // Java `componentRotation % 90 == 0` — double modulo, EXACT
    // multiples only (45.0 -> 45.0, not 0).
    if component_rotation % 90.0 == 0.0 {
        let factor = (component_rotation as i32) / 90;
        if factor != 0 {
            rel_location = rel_location.turn_90_degree(factor);
        }
    } else {
        // rotation may be not exact (Pin.java:82-87)
        let location_approx = rel_location
            .to_float()
            .rotate(java_to_radians(component_rotation), &FloatPoint::ZERO);
        rel_location = Point::Int(location_approx.round()).difference_by(&Point::ZERO);
    }
    if !component.placed_on_front() && components.flip_style_rotate_first() {
        rel_location = rel_location.mirror_at_y_axis();
    }
    Some(rel_location)
}

/// Java `DrillItem.firstLayer()` for a Pin (`DrillItem.java:162-172`):
/// the first board layer the drilled item occupies — for a front-side
/// (or absolutely-placed) padstack the padstack's own from-layer, else
/// the mirrored span.
#[must_use]
pub(crate) fn pin_first_layer(component: &Component, padstack: &BoardPadstack) -> i32 {
    if component.placed_on_front() || padstack.placed_absolute {
        padstack.from_layer() as i32
    } else {
        padstack.board_layer_count() as i32 - padstack.to_layer() - 1
    }
}

/// Java `DrillItem.lastLayer()` for a Pin (`DrillItem.java:174-185`)
/// — the MIRROR-SIDE span of [`pin_first_layer`]: for a front-side (or
/// absolutely-placed) padstack the padstack's own to-layer, else
/// `boardLayerCount - fromLayer - 1`. The two derivations swap
/// from/to between first and last (`DrillItem.java:168` vs `:181`).
#[must_use]
pub(crate) fn pin_last_layer(component: &Component, padstack: &BoardPadstack) -> i32 {
    if component.placed_on_front() || padstack.placed_absolute {
        padstack.to_layer()
    } else {
        padstack.board_layer_count() as i32 - padstack.from_layer() as i32 - 1
    }
}

/// Java `Pin.getPadstackLayer(index)` (`Pin.java:248-257`): the
/// PADSTACK layer of the pin's shape index — identity for front-side
/// and absolutely-placed padstacks, mirrored (`layerCount - index -
/// firstLayer - 1`) for back-side ones.
#[must_use]
pub(crate) fn padstack_layer(component: &Component, padstack: &BoardPadstack, index: i32) -> i32 {
    if component.placed_on_front() || padstack.placed_absolute {
        index + pin_first_layer(component, padstack)
    } else {
        padstack.board_layer_count() as i32 - index - pin_first_layer(component, padstack) - 1
    }
}

/// Java `Pin.getShape(index)` (`Pin.java:165-242`) — the FULL pin
/// shape transform chain, in Java's exact order:
///
/// 1. the raw padstack shape at [`padstack_layer`] (null slot ->
///    `None`),
/// 2. the PIN rotation — the same 90-exact / `rotateApprox` split,
/// 3. `mirrorVertical` when the flip style mirrors BEFORE,
/// 4. translate by the (mirrored) package pin location,
/// 5. the COMPONENT rotation (turn90 / rotateApprox — note the SHAPE
///    rotates here, unlike the rounded-POINT path in
///    [`pin_relative_location`]),
/// 6. `mirrorVertical` when the flip style mirrors AFTER,
/// 7. translate by the component location.
#[must_use]
pub(crate) fn pin_shape(
    components: &Components,
    library: &BoardLibrary,
    component_id: u32,
    pin_index: i32,
    shape_index: i32,
) -> Option<BoardShape> {
    let (component, package_pin) =
        resolve_package_pin(components, library, component_id, pin_index)?;
    let location = component.location?;
    let padstack = library.padstack(package_pin.padstack_no)?;
    let layer = padstack_layer(component, padstack, shape_index);
    let current = padstack.get_shape(usize::try_from(layer).ok()?)?.clone();

    let mirror_before = !component.placed_on_front() && !components.flip_style_rotate_first();
    let mirror_after = !component.placed_on_front() && components.flip_style_rotate_first();

    let mut rel_location = Vector::Int(IntVector::new(
        package_pin.rel_location.x,
        package_pin.rel_location.y,
    ));
    if mirror_before {
        rel_location = rel_location.mirror_at_y_axis();
    }

    // (2) the pin rotation (Pin.java:200-213)
    let mut shape = current;
    let pin_rotation = package_pin.rotation;
    if pin_rotation % 90.0 == 0.0 {
        let factor = (pin_rotation as i32) / 90;
        if factor != 0 {
            shape = shape_turn_90_degree(shape, factor);
        }
    } else {
        shape = shape_rotate_approx(shape, java_to_radians(pin_rotation));
    }

    // (3) mirror before (Pin.java:214-216)
    if mirror_before {
        shape = shape_mirror_vertical(shape);
    }

    // (4) translate relative to the component (Pin.java:219)
    let mut shape = shape_translate_by(shape, &rel_location);

    // (5) the component rotation (Pin.java:221-232)
    let component_rotation = component.rotation_in_degree;
    if component_rotation % 90.0 == 0.0 {
        let factor = (component_rotation as i32) / 90;
        if factor != 0 {
            shape = shape_turn_90_degree(shape, factor);
        }
    } else {
        shape = shape_rotate_approx(shape, java_to_radians(component_rotation));
    }

    // (6) mirror after (Pin.java:233-234)
    if mirror_after {
        shape = shape_mirror_vertical(shape);
    }

    // (7) translate to the component location (Pin.java:236-237)
    let component_translation = Point::Int(location).difference_by(&Point::ZERO);
    Some(shape_translate_by(shape, &component_translation))
}

/// Java `Pin.getCenter()` (`Pin.java:91-120`): the raw center
/// `component.location.translateBy(relativeLocation())` (`:98`),
/// CORRECTED to `firstShape.centreOfGravity().round()` (`:115`) when
/// the first non-null padstack shape over the pin's layer span does
/// not contain it (`:106-114`). A pin with no shape at all keeps its
/// raw center (Java logs a warning there).
#[must_use]
pub fn pin_center(
    components: &Components,
    library: &BoardLibrary,
    component_id: u32,
    pin_index: i32,
) -> Option<Point> {
    let (component, package_pin) =
        resolve_package_pin(components, library, component_id, pin_index)?;
    let location = component.location?;
    let rel_location = pin_relative_location(components, library, component_id, pin_index)?;
    let pin_center_point = Point::Int(location).translate_by(&rel_location);

    let padstack = library.padstack(package_pin.padstack_no)?;
    let from_layer = padstack.from_layer() as i32;
    let to_layer = padstack.to_layer();
    for shape_index in 0..to_layer - from_layer + 1 {
        if let Some(shape) = pin_shape(components, library, component_id, pin_index, shape_index) {
            if !shape_contains_inside(&shape, &pin_center_point) {
                // Pin.java:114-116: the correction branch.
                return Some(Point::Int(shape_centre_of_gravity(&shape).round()));
            }
            break;
        }
    }
    Some(pin_center_point)
}

// ---------------------------------------------------------------------------
// Pin-name normalization (upstream 14b28b6ff, #925b)
// ---------------------------------------------------------------------------

/// Java `Pin.getBasePinName` (upstream `14b28b6ff`, #925 — the second
/// half, P4): normalizes a pin name by stripping a composite sub-pad
/// suffix to expose the base logical pad two sub-pads share. Two
/// separator families, tried in order:
///
/// * everything from the FIRST `@` or `#` (`PAD@1` / `PAD@2` →
///   `PAD`; no digit requirement on these suffixes — `@` marks a
///   sub-pad unconditionally);
/// * the LAST `_` or `-` when the suffix after it is ALL DIGITS and
///   the separator is neither the first nor the last character
///   (`pad_1_1` → `pad_1`, `1-1` → `1`; `A_B` / `A-B` keep their
///   non-digit suffixes, `_1` / `PAD_` keep their edge separators).
///
/// The `_` family is tried BEFORE `-` (`pad_1-1` → the `_` suffix
/// `1-1` is not all digits → falls through → the `-` suffix `1` is →
/// `pad_1`). The digit test is ASCII where Java's
/// `Character.isDigit` also admits Unicode decimal digits — DSN pin
/// names are ASCII tokens, and ASCII is the safe subset.
#[must_use]
pub fn base_pin_name(pin_name: &str) -> &str {
    if let Some(at) = pin_name.find('@') {
        return &pin_name[..at];
    }
    if let Some(hash) = pin_name.find('#') {
        return &pin_name[..hash];
    }
    strip_digits_suffixed(pin_name, '_')
        .or_else(|| strip_digits_suffixed(pin_name, '-'))
        .unwrap_or(pin_name)
}

/// The `_`/`-` family of [`base_pin_name`]: strip at the LAST
/// `separator` iff the trailing run after it is non-empty and all
/// digits (Java `isAllDigits` on the `substring(last + 1)`, guarded
/// by `last > 0 && last < length - 1`).
fn strip_digits_suffixed(name: &str, separator: char) -> Option<&str> {
    let last = name.rfind(separator)?;
    if last == 0 || last == name.len() - 1 {
        return None;
    }
    name[last + 1..]
        .bytes()
        .all(|b| b.is_ascii_digit())
        .then_some(&name[..last])
}

/// The package pin NAME a board pin refers to — the resolution half
/// of upstream `Pin.isSameLogicalPad` (`14b28b6ff`): component →
/// package (by side) → pin (by index) → `name`. `None` for unknown
/// components, unresolvable packages, and out-of-bounds pin indices
/// (Java's three null/bounds guards collapse to `Option` here — the
/// resolution chain is `resolve_package_pin`; the pin name itself is
/// a `String`, never null).
#[must_use]
pub fn pin_name<'a>(
    components: &'a Components,
    library: &'a BoardLibrary,
    component_id: u32,
    pin_index: i32,
) -> Option<&'a str> {
    let (_, package_pin) = resolve_package_pin(components, library, component_id, pin_index)?;
    Some(package_pin.name.as_str())
}

// ---------------------------------------------------------------------------
// Pin trace-exit restrictions (M4-T6, the pull-tight pin-connection tail)
// ---------------------------------------------------------------------------

/// Java `Pin.TraceExitRestriction` (`Pin.java:694-705`) — one allowed
/// trace exit direction from a pin pad, plus the minimal trace line
/// length from the pin center into that direction.
#[derive(Clone, Debug)]
pub struct PinTraceExitRestriction {
    /// Java `direction`.
    pub direction: Direction,
    /// Java `minLength` — the pin-center-to-pad-border distance along
    /// `direction` (computed on the TRANSFORMED pin shape, `Pin.java:317-327`).
    pub min_length: f64,
}

/// Java `Padstack.getTraceExitDirections(layer, factor)`
/// (`Padstack.java:167-196`) — the RAW (untransformed) padstack
/// shape's allowed exit directions. Empty for out-of-range layers,
/// null shape slots, and non-IntBox/IntOctagon shapes (a simplex pad
/// yields none); the long sides only join when the pad is at least
/// `factor` times longer than it is wide. The callers rotate the
/// result into the component frame (`Pin.java:306-315`).
fn padstack_trace_exit_directions(
    padstack: &BoardPadstack,
    layer: i32,
    factor: f64,
) -> Vec<Direction> {
    if layer < 0 || layer >= padstack.board_layer_count() as i32 {
        return Vec::new();
    }
    let Some(BoardShape::Tile(tile)) = padstack.get_shape(layer as usize) else {
        return Vec::new();
    };
    // Java: `currentShape instanceof IntBox || currentShape instanceof
    // IntOctagon` — the Simplex arm is excluded.
    let TileShape::RegularTileShape(_) = tile else {
        return Vec::new();
    };
    let bounds = tile.bounding_box();
    let width = f64::from(bounds.ur.x - bounds.ll.x);
    let height = f64::from(bounds.ur.y - bounds.ll.y);
    let all_dirs = width.max(height) < factor * width.min(height);
    let mut result = Vec::new();
    if all_dirs || width >= height {
        result.push(Direction::RIGHT);
        result.push(Direction::LEFT);
    }
    if all_dirs || width <= height {
        result.push(Direction::UP);
        result.push(Direction::DOWN);
    }
    result
}

/// Java `Pin.getTraceExitRestrictions(layer)` (`Pin.java:265-331`) —
/// the allowed trace exit directions of the pin's pad on `layer` with
/// their minimal trace line lengths. The DIRECTIONS derive from the
/// raw padstack shape (`padstack_trace_exit_directions`) rotated into
/// the component frame; the LENGTHS read the transformed pin shape
/// (`Pin.getShape`) along the ray from the pin center. The pin-count
/// factor is Java's literal: 1.5, doubled for packages with <= 3 pins
/// (`:268-277`).
#[allow(clippy::cast_possible_truncation)] // the `(int) rotation / 45` cast is Java's
pub fn pin_trace_exit_restrictions(
    components: &Components,
    library: &BoardLibrary,
    component_id: u32,
    pin_index: i32,
    layer: i32,
) -> Vec<PinTraceExitRestriction> {
    let Some((component, package_pin)) =
        resolve_package_pin(components, library, component_id, pin_index)
    else {
        // Java: `component == null` returns the empty set AFTER the
        // padstack-directions check (`:285-287`); both orders answer
        // empty, the unresolvable-chain face is the same.
        return Vec::new();
    };
    let padstack = match library.padstack(package_pin.padstack_no) {
        Some(padstack) => padstack,
        None => return Vec::new(),
    };
    let first_layer = pin_first_layer(component, padstack);
    let mut pad_xy_factor = 1.5;
    // Java `:273-277`: the package pin-count doubling.
    if let Some(package) = library.package(component.package_no())
        && package.pins.len() <= 3
    {
        pad_xy_factor *= 2.0;
    }
    let padstack_layer = padstack_layer(component, padstack, layer - first_layer);
    let directions = padstack_trace_exit_directions(padstack, padstack_layer, pad_xy_factor);
    if directions.is_empty() {
        return Vec::new();
    }
    // Java `:288-291`: `getShape(layer - firstLayer())` must be a
    // TileShape — the FULL transformed shape (Simplex INCLUDED, unlike
    // the raw-padstack gate above).
    let Some(BoardShape::Tile(pad_tile)) = pin_shape(
        components,
        library,
        component_id,
        pin_index,
        layer - first_layer,
    ) else {
        return Vec::new();
    };
    let Some(center) = pin_center(components, library, component_id, pin_index) else {
        return Vec::new();
    };
    let center_approx = center.to_float();
    // Java `:306-315`: the component+package rotation into the
    // component frame — the 45-exact turn, else the approx angle.
    let rotation = component.rotation_in_degree + package_pin.rotation;
    let mut result = Vec::new();
    for base in directions {
        let direction = if rotation % 45.0 == 0.0 {
            base.turn_45_degree((rotation as i32) / 45)
        } else {
            Direction::get_instance_approx(java_to_radians(rotation) + base.angle_approx())
        };
        // Java `:317-322`: the min length from the pin center into
        // `direction` on the TRANSFORMED pad shape. A missing border
        // line warns and SKIPS the direction (Java's FRLogger.warn is
        // log-only, D12).
        let border_no = pad_tile.intersecting_border_line_no(&center, &direction);
        if border_no < 0 {
            continue;
        }
        let ray = Line::new_with_direction(center.clone(), direction.clone());
        let border_point = ray.intersection_approx(&pad_tile.border_line(border_no));
        result.push(PinTraceExitRestriction {
            min_length: center_approx.distance(&border_point),
            direction,
        });
    }
    result
}

/// Java `Pin.calcNearestExitRestrictionDirection`
/// (`Pin.java:569-632`) — the nearest legal pin exit direction for
/// changing `trace_polyline` (assumed to start at the pin center),
/// or `None` when no restriction matches. `pin_edge_to_turn_dist` is
/// the caller-cached `rules.getPinEdgeToTurnDist()` (the `< 0` gate
/// answers `None`, `:580-583`).
#[allow(clippy::too_many_arguments)] // the Java signature, kept 1:1
#[must_use]
pub fn pin_calc_nearest_exit_restriction_direction(
    components: &Components,
    library: &BoardLibrary,
    component_id: u32,
    pin_index: i32,
    trace_polyline: &Polyline,
    trace_half_width: i32,
    layer: i32,
    pin_edge_to_turn_dist: f64,
) -> Option<Direction> {
    let restrictions =
        pin_trace_exit_restrictions(components, library, component_id, pin_index, layer);
    if restrictions.is_empty() {
        return None;
    }
    let (component, package_pin) =
        resolve_package_pin(components, library, component_id, pin_index)?;
    let padstack = library.padstack(package_pin.padstack_no)?;
    let first_layer = pin_first_layer(component, padstack);
    let Some(BoardShape::Tile(pin_tile)) = pin_shape(
        components,
        library,
        component_id,
        pin_index,
        layer - first_layer,
    ) else {
        return None;
    };
    if pin_edge_to_turn_dist < 0.0 {
        return None;
    }
    let offset_shape = pin_tile.offset(pin_edge_to_turn_dist + f64::from(trace_half_width));
    let entries = offset_shape.entrance_points(trace_polyline);
    let &latest_entry = entries.last()?;
    let trace_entry_location_approx = trace_polyline.lines[latest_entry.0 as usize]
        .intersection_approx(&offset_shape.border_line(latest_entry.1));
    // The nearest-exit loop (`:595-630`): strictly nearer wins; inside
    // the tolerance-1 band the trace-corner comparison decides.
    let mut min_exit_corner_distance = f64::MAX;
    let mut nearest_exit_corner: Option<FloatPoint> = None;
    let mut pin_exit_direction: Option<Direction> = None;
    const TOLERANCE: f64 = 1.0;
    let center = pin_center(components, library, component_id, pin_index)?;
    for restriction in &restrictions {
        // Java reads the border line unconditionally (`:601-606`);
        // a missing border line on the offset shape is Java-unreachable
        // (the un-offset derivation found one). The port answers None
        // for the frame instead of indexing out of bounds.
        let border_no = offset_shape.intersecting_border_line_no(&center, &restriction.direction);
        if border_no < 0 {
            return None;
        }
        let ray = Line::new_with_direction(center.clone(), restriction.direction.clone());
        let exit_corner = ray.intersection_approx(&offset_shape.border_line(border_no));
        let exit_corner_distance = exit_corner.distance_square(&trace_entry_location_approx);
        let mut new_nearest_corner_found = false;
        if exit_corner_distance + TOLERANCE < min_exit_corner_distance {
            new_nearest_corner_found = true;
        } else if exit_corner_distance < min_exit_corner_distance + TOLERANCE {
            // the distances are near equal, compare to the previous
            // corners of tracePolyline (`:611-624`)
            if let Some(old_corner) = nearest_exit_corner {
                for i in 1..trace_polyline.corner_count() as i32 {
                    let current_trace_corner = trace_polyline.corner_approx(i);
                    let current_trace_corner_distance =
                        current_trace_corner.distance_square(&exit_corner);
                    let old_trace_corner_distance =
                        current_trace_corner.distance_square(&old_corner);
                    if current_trace_corner_distance + TOLERANCE < old_trace_corner_distance {
                        new_nearest_corner_found = true;
                        break;
                    } else if current_trace_corner_distance > old_trace_corner_distance + TOLERANCE
                    {
                        break;
                    }
                }
            }
        }
        if new_nearest_corner_found {
            min_exit_corner_distance = exit_corner_distance;
            pin_exit_direction = Some(restriction.direction.clone());
            nearest_exit_corner = Some(exit_corner);
        }
    }
    pin_exit_direction
}

// ---------------------------------------------------------------------------
// BoardShape transform dispatch (the Java (ConvexShape) casts)
// ---------------------------------------------------------------------------

/// `ConvexShape.turn90Degree(factor, Point.ZERO)` over the shape arms.
/// `pub(crate)`: [`crate::items::obstacle`] runs the same dispatch for
/// the T49 absolute-area chain.
pub(crate) fn shape_turn_90_degree(shape: BoardShape, factor: i32) -> BoardShape {
    let zero = IntPoint::new(0, 0);
    match shape {
        BoardShape::Tile(tile) => BoardShape::Tile(tile.turn_90_degree(factor, &zero)),
        BoardShape::PolygonShape(polygon) => {
            BoardShape::PolygonShape(polygon.turn_90_degree(factor, &zero))
        }
        BoardShape::Circle(circle) => BoardShape::Circle(circle.turn_90_degree(factor, &zero)),
    }
}

/// `ConvexShape.rotateApprox(angle, FloatPoint.ZERO)` over the arms.
/// `pub(crate)`: [`crate::items::obstacle`] (T49 chain).
pub(crate) fn shape_rotate_approx(shape: BoardShape, angle: f64) -> BoardShape {
    match shape {
        BoardShape::Tile(tile) => BoardShape::Tile(tile.rotate_approx(angle, &FloatPoint::ZERO)),
        BoardShape::PolygonShape(polygon) => {
            BoardShape::PolygonShape(polygon.rotate_approx(angle, &FloatPoint::ZERO))
        }
        BoardShape::Circle(circle) => {
            BoardShape::Circle(circle.rotate_approx(angle, &FloatPoint::ZERO))
        }
    }
}

/// `ConvexShape.mirrorVertical(Point.ZERO)` over the arms.
/// `pub(crate)`: [`crate::items::obstacle`] (T49 chain).
pub(crate) fn shape_mirror_vertical(shape: BoardShape) -> BoardShape {
    let zero = IntPoint::new(0, 0);
    match shape {
        BoardShape::Tile(tile) => BoardShape::Tile(tile.mirror_vertical(&zero)),
        BoardShape::PolygonShape(polygon) => {
            BoardShape::PolygonShape(polygon.mirror_vertical(&zero))
        }
        BoardShape::Circle(circle) => BoardShape::Circle(circle.mirror_vertical(&zero)),
    }
}

/// `Shape.translateBy(Vector)` over the arms.
/// `pub(crate)`: [`crate::items::obstacle`] (T49 chain) and
/// [`crate::items::drill`] (`Via.getShape`'s center translation,
/// `Via.java:115-132`).
pub(crate) fn shape_translate_by(shape: BoardShape, vector: &Vector) -> BoardShape {
    match shape {
        BoardShape::Tile(tile) => BoardShape::Tile(tile.translate_by(vector)),
        BoardShape::PolygonShape(polygon) => BoardShape::PolygonShape(polygon.translate_by(vector)),
        BoardShape::Circle(circle) => BoardShape::Circle(circle.translate_by(vector)),
    }
}

/// `Shape.containsInside(Point)` over the arms.
fn shape_contains_inside(shape: &BoardShape, point: &Point) -> bool {
    match shape {
        BoardShape::Tile(tile) => tile.contains_inside(point),
        BoardShape::PolygonShape(polygon) => polygon.contains_inside(point),
        BoardShape::Circle(circle) => circle.contains_inside(point),
    }
}

/// `Shape.centreOfGravity()` over the arms. The polygon arm is
/// unreachable from a PARSED padstack: the library reader converts
/// every padstack shape through the convex conversion (Tile/Circle
/// only — an epic-dsn `BoardShape::PolygonShape` never reaches a
/// padstack slot), and Java's `(ConvexShape)` cast at `Pin.java:180`
/// would have died with a `ClassCastException` for a polygon.
fn shape_centre_of_gravity(shape: &BoardShape) -> FloatPoint {
    match shape {
        BoardShape::Tile(tile) => tile.centre_of_gravity(),
        BoardShape::Circle(circle) => circle.centre_of_gravity(),
        BoardShape::PolygonShape(_) => {
            unreachable!("parse never stores a polygon padstack shape")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use epic_dsn::reader::{DsnReadResult, read_board};
    use epic_geometry::int_box::IntBox;
    use epic_geometry::regular_tile_shape::RegularTileShape;
    use epic_geometry::tile_shape::TileShape;

    /// A component placed at (1_000_000, 0).
    fn placed(rotation: f64, is_front: bool) -> Component {
        Component::new(
            "U1",
            Some(IntPoint::new(1_000_000, 0)),
            rotation,
            is_front,
            1,
            1,
            false,
        )
    }

    /// `Component.java:54-79`: the ctor normalizes with WHILE LOOPS —
    /// `720.5 -> 0.5`, `-45.5 -> 314.5` — NOT `%` (which would leave
    /// `-45.5`) and not `rem_euclid` either.
    #[test]
    fn component_ctor_normalizes_rotation_with_while_loops() {
        assert_eq!(placed(720.5, true).rotation_in_degree, 0.5);
        assert_eq!(placed(-45.5, true).rotation_in_degree, 314.5);
        assert_eq!(placed(360.0, true).rotation_in_degree, 0.0);
        assert_eq!(placed(0.0, true).rotation_in_degree, 0.0);
    }

    /// **THE T68 TRAP** — `Component.rotate` (`Component.java:127-148`)
    /// on a BACK-SIDE component with the rotate-first flip style: the
    /// ROTATION advances by the TURN angle (`360 - 30 = 330`) while the
    /// LOCATION rotates by the ORIGINAL angle (`toRadians(30)`). Both
    /// halves are pinned: using `turn_angle` for the location rotates
    /// the point by 330 degrees (`(866025, -499999)`, sign-flipped y);
    /// using `angle` for the rotation leaves 30. The exact location
    /// `(866025, 500000)` comes from the Java formula
    /// `1e6 * cos(pi/6) = 866025.403...`, `1e6 * sin(pi/6) =
    /// 499999.999...` rounded up (`FloatPoint.round`, ties toward
    /// +infinity — the values are NOT on a tie).
    #[test]
    fn rotate_back_side_rotate_first_turn_angle_for_rotation_original_for_location() {
        let pole = IntPoint::new(0, 0);
        // Back side + rotate-first: rotation += 330, location rotates 30.
        let mut back = placed(0.0, false);
        back.rotate(30.0, &pole, true);
        assert_eq!(back.rotation_in_degree, 330.0, "rotation += turn_angle 330");
        let location = back.location.expect("placed");
        assert_eq!(
            location,
            IntPoint::new(866_025, 500_000),
            "location rotates by the ORIGINAL 30"
        );
        // The 330-degree mistake the trap guards against:
        assert_ne!(location, IntPoint::new(866_025, -500_000));

        // Front side: the flip-style branch does not fire; both are 30.
        let mut front = placed(0.0, true);
        front.rotate(30.0, &pole, true);
        assert_eq!(front.rotation_in_degree, 30.0);
        assert_eq!(
            front.location.expect("placed"),
            IntPoint::new(866_025, 500_000)
        );

        // Back side + mirror-first (the default): turn_angle == angle.
        let mut mirror_first = placed(0.0, false);
        mirror_first.rotate(30.0, &pole, false);
        assert_eq!(mirror_first.rotation_in_degree, 30.0);
        assert_eq!(
            mirror_first.location.expect("placed"),
            IntPoint::new(866_025, 500_000)
        );

        // angle == 0 is Java's early return — nothing moves.
        let mut zero = placed(90.0, false);
        zero.rotate(0.0, &pole, true);
        assert_eq!(zero.rotation_in_degree, 90.0);
        assert_eq!(zero.location.expect("placed"), IntPoint::new(1_000_000, 0));
    }

    /// `Component.java:110-125` `turn90Degree` and `:153-156`
    /// `changeSide`: exact integer turn around a NON-origin pole, the
    /// rotation re-normalization, and the side flip + vertical mirror.
    #[test]
    fn turn_90_degree_and_change_side_are_exact_integer_ops() {
        let pole = IntPoint::new(100_000, 100_000);
        let mut component = placed(315.0, true);
        component.turn_90_degree(1, &pole);
        // 315 + 90 = 405 -> 45.
        assert_eq!(component.rotation_in_degree, 45.0);
        // (1e6, 0) turned 90 CCW around (1e5, 1e5): the point
        // (1e6 - 1e5, -1e5) -> (1e5, 9e5) + pole = (2e5, 1e6).
        assert_eq!(
            component.location.expect("placed"),
            IntPoint::new(200_000, 1_000_000)
        );
        // changeSide: front -> back, mirrored at the vertical line
        // through the pole: x' = 2 * pole.x - x.
        component.change_side(&pole);
        assert!(!component.is_front);
        assert_eq!(
            component.location.expect("placed"),
            IntPoint::new(0, 1_000_000)
        );
        // factor 0 is the early return.
        let before = placed(90.0, true);
        let mut untouched = before.clone();
        untouched.turn_90_degree(0, &pole);
        assert_eq!(untouched, before);
    }

    /// A minimal library + components table for the crafted pin tests:
    /// one padstack (a box on layer 0 of a 2-layer board), one package
    /// with a single pin at a chosen relative location.
    fn pin_table(
        flip_style_rotate_first: bool,
        component: Component,
        pin_rel: IntPoint,
        pin_rotation: f64,
    ) -> (Components, BoardLibrary) {
        let mut components = Components::new();
        components.set_flip_style_rotate_first(flip_style_rotate_first);
        let component_id = components.add(component);
        assert_eq!(component_id, 1);
        let library = BoardLibrary {
            padstacks: vec![BoardPadstack {
                name: "PIN".to_string(),
                shapes: vec![
                    Some(BoardShape::Tile(TileShape::RegularTileShape(
                        RegularTileShape::IntBox(IntBox::new(
                            IntPoint::new(-5000, -5000),
                            IntPoint::new(5000, 5000),
                        )),
                    ))),
                    None,
                ],
                drillable: false,
                placed_absolute: false,
                hole_only: false,
            }],
            packages: vec![BoardPackage {
                name: "PKG".to_string(),
                pins: vec![BoardPackagePin {
                    name: "1".to_string(),
                    padstack_no: 1,
                    rel_location: pin_rel,
                    rotation: pin_rotation,
                }],
            }],
        };
        (components, library)
    }

    /// `Pin.java:73-75` vs `:88-90` — the mirror ordering around the
    /// component rotation. Back side, rotation 90, package pin at
    /// (48500, 38100):
    /// - mirror-FIRST (default): (-48500, 38100) -> turn90(1) ->
    ///   (-38100, -48500),
    /// - rotate-first: turn90(1) -> (-38100, 48500) -> mirror ->
    ///   (38100, 48500).
    ///
    /// The mirror-at-y-axis x-negation is the ONLY difference; a port
    /// that skips either mirror fails its own side.
    #[test]
    fn relative_location_mirror_before_vs_after_both_flip_styles() {
        let pin_rel = IntPoint::new(48_500, 38_100);
        let (mirror_first, library) = pin_table(false, placed(90.0, false), pin_rel, 0.0);
        let rel = pin_relative_location(&mirror_first, &library, 1, 0).expect("resolves");
        match rel {
            Vector::Int(v) => assert_eq!((v.x, v.y), (-38_100, -48_500), "mirror BEFORE"),
            other => panic!("expected an integer vector, got {other:?}"),
        }

        let (rotate_first, library) = pin_table(true, placed(90.0, false), pin_rel, 0.0);
        let rel = pin_relative_location(&rotate_first, &library, 1, 0).expect("resolves");
        match rel {
            Vector::Int(v) => assert_eq!((v.x, v.y), (38_100, 48_500), "mirror AFTER"),
            other => panic!("expected an integer vector, got {other:?}"),
        }

        // Front side: NO mirror in either style — the spike's J1 pin 3
        // (`comp=36 ... front rot=90.0 ... relRaw=48500 38100
        // rel=-38100 48500 mirror=none`).
        let (front_default, library) = pin_table(false, placed(90.0, true), pin_rel, 0.0);
        let rel = pin_relative_location(&front_default, &library, 1, 0).expect("resolves");
        match rel {
            Vector::Int(v) => assert_eq!((v.x, v.y), (-38_100, 48_500), "no mirror"),
            other => panic!("expected an integer vector, got {other:?}"),
        }
    }

    /// The 90-degree EXACT branch (`Pin.java:76-81`): `(int) rotation /
    /// 90` with Java's double-modulo gate. Rotation 270 back-side
    /// mirror-first, package pin (-8250, 0): mirror -> (8250, 0) ->
    /// turn90(3) -> (0, -8250) — the spike's C21 pin 0 (`comp=94 ...
    /// rot=270.0 ... relRaw=-8250 0 rel=0 -8250 branch=90`).
    #[test]
    fn relative_location_ninety_degree_branch_uses_int_cast_factor() {
        let (components, library) =
            pin_table(false, placed(270.0, false), IntPoint::new(-8_250, 0), 0.0);
        let rel = pin_relative_location(&components, &library, 1, 0).expect("resolves");
        match rel {
            Vector::Int(v) => assert_eq!((v.x, v.y), (0, -8_250)),
            other => panic!("expected an integer vector, got {other:?}"),
        }
    }

    /// The non-90 FLOAT branch (`Pin.java:82-87`): rotation 45,
    /// package pin (6750, 0) back-side mirror-first: mirror ->
    /// (-6750, 0) -> rotate 45 deg -> (4772.97..., -4772.97...) ->
    /// round (ties toward +infinity; not on a tie) -> (4773, -4773).
    /// The spike's C23 pin 1 (`comp=93 ... rot=45.0 ... relRaw=6750 0
    /// rel=-4773 -4773 branch=float mirror=before`). Note this is the
    /// mirror of the captured C23 pin 0 pair (relRaw -6750 -> rel
    /// 4773,4773): both are pinned.
    #[test]
    fn relative_location_float_branch_rounds_rotated_point() {
        let (components, library) =
            pin_table(false, placed(45.0, false), IntPoint::new(6_750, 0), 0.0);
        let rel = pin_relative_location(&components, &library, 1, 0).expect("resolves");
        match rel {
            Vector::Int(v) => assert_eq!((v.x, v.y), (-4_773, -4_773)),
            other => panic!("expected an integer vector, got {other:?}"),
        }
        let (components, library) =
            pin_table(false, placed(45.0, false), IntPoint::new(-6_750, 0), 0.0);
        let rel = pin_relative_location(&components, &library, 1, 0).expect("resolves");
        match rel {
            Vector::Int(v) => assert_eq!((v.x, v.y), (4_773, 4_773)),
            other => panic!("expected an integer vector, got {other:?}"),
        }
    }

    /// `Pin.java:91-120` `getCenter` — the raw center and its
    /// pad-shape CORRECTION. No corpus fixture triggers the correction
    /// (the spike's `CORRECTIONS 0` over 274 pins; the scanned fixture
    /// set never produced one), so both sides are pinned on the spike's
    /// SYNTHETIC case, constructed here the same way: an OFF-CENTER pad
    /// box `(1000,1000)-(9000,9000)` whose padstack origin (0,0) lies
    /// OUTSIDE it, package pin at (0,0), component at
    /// (1000000, -1000000). The raw center (1000000, -1000000) is not
    /// contained, so the center becomes `centreOfGravity().round()`:
    /// the transformed box `(1001000,-999000)-(1009000,-991000)` has
    /// gravity `(1005000.0, -995000.0)` -> `(1005000, -995000)` — the
    /// spike's `SYNTHETIC ... corrected=true gravity=1005000.0 -995000.0
    /// ... center=1005000 -995000 agree=true`.
    #[test]
    fn pin_center_corrects_to_centre_of_gravity_when_the_pad_misses() {
        let mut components = Components::new();
        let component_id = components.add(Component::new(
            "SPIKE1",
            Some(IntPoint::new(1_000_000, -1_000_000)),
            0.0,
            true,
            1,
            1,
            false,
        ));
        assert_eq!(component_id, 1);
        let library = BoardLibrary {
            padstacks: vec![BoardPadstack {
                name: "spike_offcenter".to_string(),
                shapes: vec![
                    Some(BoardShape::Tile(TileShape::RegularTileShape(
                        RegularTileShape::IntBox(IntBox::new(
                            IntPoint::new(1_000, 1_000),
                            IntPoint::new(9_000, 9_000),
                        )),
                    ))),
                    None,
                ],
                drillable: false,
                placed_absolute: false,
                hole_only: false,
            }],
            packages: vec![BoardPackage {
                name: "spike_pkg".to_string(),
                pins: vec![BoardPackagePin {
                    name: "1".to_string(),
                    padstack_no: 1,
                    rel_location: IntPoint::new(0, 0),
                    rotation: 0.0,
                }],
            }],
        };

        // The raw center (before correction) is the component location.
        let rel = pin_relative_location(&components, &library, 1, 0).expect("resolves");
        match rel {
            Vector::Int(v) => assert_eq!((v.x, v.y), (0, 0), "pin at the origin"),
            other => panic!("expected an integer vector, got {other:?}"),
        }
        // The first shape over the padstack span: layer 0 only.
        let shape = pin_shape(&components, &library, 1, 0, 0).expect("shape");
        let gravity = shape_centre_of_gravity(&shape);
        assert_eq!((gravity.x, gravity.y), (1_005_000.0, -995_000.0));
        // ...and it does NOT contain the raw center -> corrected.
        let raw_center = Point::Int(IntPoint::new(1_000_000, -1_000_000));
        assert!(!shape_contains_inside(&shape, &raw_center));
        match pin_center(&components, &library, 1, 0).expect("resolves") {
            Point::Int(center) => {
                assert_eq!(center, IntPoint::new(1_005_000, -995_000));
            }
            other => panic!("expected an integer center, got {other:?}"),
        }
    }

    /// The NON-corrected side of `getCenter` (the fixture-real case):
    /// a CENTERED pad box contains the raw center, so the center is
    /// just `location + relativeLocation`.
    #[test]
    fn pin_center_keeps_the_raw_center_inside_a_centered_pad() {
        let (components, library) =
            pin_table(false, placed(45.0, false), IntPoint::new(-6_750, 0), 0.0);
        match pin_center(&components, &library, 1, 0).expect("resolves") {
            Point::Int(center) => assert_eq!(
                center,
                IntPoint::new(1_000_000 + 4_773, 4_773),
                "loc + rel (the spike's C23 pin 0 arithmetic)"
            ),
            other => panic!("expected an integer center, got {other:?}"),
        }
    }

    /// `Components.java`: 1-BASED monotone ids, the arr as the get(id)
    /// truth, case-SENSITIVE name lookup, and the undo/redo re-sync
    /// INTO THE SAME SLOT (`restoreComponentArrFromUndoList`) — a
    /// rotate + undo restores the pre-rotate placement (this extends
    /// the T63 stack pin with REAL Component values through the full
    /// container surface).
    #[test]
    fn components_container_ids_lookup_and_undo_resync() {
        let mut components = Components::new();
        let id1 = components.add(placed(0.0, true));
        let id2 = components.add(Component::new(
            "C23",
            Some(IntPoint::new(1_490_787, -892_287)),
            45.0,
            false,
            1,
            2,
            true,
        ));
        assert_eq!((id1, id2), (1, 2), "1-based, insertion order");
        assert_eq!(components.count(), 2);

        // get(id) reads the arr.
        let c23 = components.get(id2).expect("id 2");
        assert_eq!(c23.name, "C23");
        assert!(!c23.placed_on_front(), "back side");
        assert_eq!(c23.rotation_in_degree, 45.0);
        assert!(c23.position_fixed);
        assert_eq!(c23.package_no(), 2, "back side -> package_back");
        assert!(components.get(3).is_none());
        assert_eq!(components.get(1).expect("id 1").package_no(), 1);

        // Name lookup is case-SENSITIVE over the arr.
        assert_eq!(components.get_by_name("c23"), None);
        assert_eq!(components.get_by_name("C23").map(|(id, _)| id), Some(id2));

        // rotate through the container (saveForUndo + mutate), then
        // undo restores the pre-rotate state INTO THE SAME SLOT.
        components.generate_snapshot();
        components.rotate(id1, 30.0, &IntPoint::new(0, 0));
        let rotated = components.get(id1).expect("id 1");
        assert_eq!(rotated.rotation_in_degree, 30.0);
        assert_eq!(
            rotated.location.expect("placed"),
            IntPoint::new(866_025, 500_000)
        );
        assert!(components.undo(), "undo to the snapshot");
        let restored = components.get(id1).expect("same slot");
        assert_eq!(restored.rotation_in_degree, 0.0, "pre-rotate rotation");
        assert_eq!(
            restored.location.expect("placed"),
            IntPoint::new(1_000_000, 0),
            "pre-rotate location"
        );
        assert_eq!(restored.name, "U1", "slot identity preserved");
        // redo replays the rotate.
        assert!(components.redo());
        assert_eq!(components.get(id1).expect("id 1").rotation_in_degree, 30.0);
        // The id counter never rewinds: the next add is 3.
        assert_eq!(components.add(placed(0.0, true)), 3);
        assert_eq!(components.count(), 3);
    }

    /// The flip-style flag round-trip (`Components.java:190-202`) and
    /// the IR conversion ([`Components::from_ir`]) — the flag, the
    /// placement fields, and the fixed mapping (`position_fixed` is
    /// the SYSTEM_FIXED side of `Network.java:974-976`).
    #[test]
    fn components_from_ir_maps_placements_and_flip_style() {
        let components = [ComponentIr {
            name: "U1".to_string(),
            package_front: 1,
            package_back: 2,
            location: Some(IntPoint::new(100, -200)),
            rotation: -45.5,
            is_front: false,
            fixed: FixedStateIr::SystemFixed,
            part_number: None,
            logical_part: None,
        }];
        let table = Components::from_ir(&components, true);
        assert!(table.flip_style_rotate_first());
        let component = table.get(1).expect("1-based id");
        assert_eq!(component.name, "U1");
        assert_eq!(
            component.rotation_in_degree, 314.5,
            "ctor normalization ran"
        );
        assert!(component.position_fixed, "SYSTEM_FIXED -> position_fixed");
        assert_eq!(component.package_no(), 2, "back side");
        // Unfixed stays false.
        let mut unfixed = components.clone();
        unfixed[0].fixed = FixedStateIr::Unfixed;
        let table = Components::from_ir(&unfixed, false);
        assert!(!table.get(1).expect("id 1").position_fixed);
        assert!(!table.flip_style_rotate_first());
    }

    // -----------------------------------------------------------------
    // Fixture pins — the spike capture (/tmp/epic-t3-pins.out)
    // -----------------------------------------------------------------

    /// The StickHub fixture path (tier B: the smallest fixture with
    /// non-90-degree rotations — no tier-A board has one).
    const STICKHUB: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../scripts/benchmark/fixtures/KiCad_10_demos/StickHub.dsn"
    );

    /// Parses StickHub through the epic-dsn reader and converts it.
    fn stickhub_board() -> crate::board::Board {
        let bytes = std::fs::read(STICKHUB).expect("StickHub fixture present");
        let mut ses = epic_dsn::ses_board::SesBoard::new();
        match read_board(bytes.as_slice(), &mut ses) {
            DsnReadResult::Success { .. } => {}
            other => panic!("expected Success, got {other:?}"),
        }
        crate::board::Board::from_ses_board(&ses)
    }

    /// The spike's BOARD line: `BOARD items=925 components=94 layers=2
    /// flipStyleRotateFirst=false` — the conversion must produce the
    /// same table sizes.
    #[test]
    fn stickhub_board_line_matches_the_spike() {
        let board = stickhub_board();
        assert_eq!(board.item_count(), 925);
        assert_eq!(board.components().count(), 94);
        assert_eq!(board.layers().layers.len(), 2);
        assert!(!board.components().flip_style_rotate_first());
    }

    /// The spike's sampled pins, EXACT longs: relativeLocation and
    /// getCenter for the float branch (C23, rot 45 back-side), the
    /// 90-exact branch (C21 rot 270, J1 rot 90 front), across mirror
    /// sides. Every expectation is a quoted capture line.
    #[test]
    fn stickhub_pin_centers_match_the_spike_capture() {
        let board = stickhub_board();
        let components = board.components();
        let library = board.library();

        // (component id, pin index, expected rel, expected center) from
        // the capture lines quoted per case below.
        type PinCase = (u32, i32, (i32, i32), (i32, i32));
        let cases: &[PinCase] = &[
            // "PIN comp=93 compName=C23 pinIdx=0 ... rot=45.0 ...
            //  relRaw=-6750 0 rel=4773 4773 branch=float mirror=before
            //  ... center=1495560 -887514 agree=true"
            (93, 0, (4_773, 4_773), (1_495_560, -887_514)),
            // "PIN comp=93 compName=C23 pinIdx=1 ... relRaw=6750 0
            //  rel=-4773 -4773 ... center=1486014 -897060 agree=true"
            (93, 1, (-4_773, -4_773), (1_486_014, -897_060)),
            // "PIN comp=94 compName=C21 pinIdx=1 side=back rot=270.0
            //  ... relRaw=8250 0 rel=0 8250 ... center=1572500 -1016750"
            (94, 1, (0, 8_250), (1_572_500, -1_016_750)),
            // "PIN comp=94 compName=C21 pinIdx=0 ... relRaw=-8250 0
            //  rel=0 -8250 ... center=1572500 -1033250"
            (94, 0, (0, -8_250), (1_572_500, -1_033_250)),
            // "PIN comp=36 compName=J1 pinIdx=3 side=front rot=90.0
            //  ... relRaw=48500 38100 rel=-38100 48500 branch=90
            //  mirror=none ... center=1461900 -1151500"
            (36, 3, (-38_100, 48_500), (1_461_900, -1_151_500)),
            // "PIN comp=87 compName=C9 pinIdx=0 side=back rot=315.0
            //  ... relRaw=-4500 0 rel=3182 -3182 branch=float
            //  mirror=before ... center=1466185 -947827"
            (87, 0, (3_182, -3_182), (1_466_185, -947_827)),
        ];
        for &(component_id, pin_index, expected_rel, expected_center) in cases {
            let rel = pin_relative_location(components, library, component_id, pin_index)
                .unwrap_or_else(|| panic!("pin (comp {component_id}, idx {pin_index}) resolves"));
            match rel {
                Vector::Int(v) => assert_eq!(
                    (v.x, v.y),
                    expected_rel,
                    "relativeLocation of comp {component_id} pin {pin_index}"
                ),
                other => panic!("expected an integer vector, got {other:?}"),
            }
            match pin_center(components, library, component_id, pin_index)
                .unwrap_or_else(|| panic!("center of comp {component_id} pin {pin_index}"))
            {
                Point::Int(center) => assert_eq!(
                    (center.x, center.y),
                    expected_center,
                    "getCenter of comp {component_id} pin {pin_index}"
                ),
                other => panic!("expected an integer center, got {other:?}"),
            }
        }
    }

    // -----------------------------------------------------------------
    // Phase 6 pins — the flip-style re-query + the multi-layer
    // synthetic (/tmp/epic-t3-pins-final.out, the T3 review fix round)
    // -----------------------------------------------------------------

    /// The SPIKE2/SPIKE3 geometry (`PinResolutionSpike.java` Phase 6):
    /// a 2-layer padstack with DISTINCT asymmetric per-layer boxes
    /// (L0 flat horizontal, L1 tall and offset), a package pin at rel
    /// (20000, 10000) with PIN ROTATION 90, and a BACK-SIDE component
    /// with rotation 90 at (1500000, -900000). The distinct boxes make
    /// the back-side padstack-LAYER REMAP observable (shape index 0
    /// must read the L1 box — `layerCount - index - firstLayer - 1`;
    /// an identity remap reads the flat L0 box and fails), and the
    /// asymmetric boxes keep the 90-degree pin rotation observable
    /// (turn90 of a box centered on the origin is not identity here).
    fn spike2_geometry(flip_style_rotate_first: bool) -> (Components, BoardLibrary) {
        let mut components = Components::new();
        components.set_flip_style_rotate_first(flip_style_rotate_first);
        let component_id = components.add(Component::new(
            "SPIKE2",
            Some(IntPoint::new(1_500_000, -900_000)),
            90.0,
            false,
            1,
            1,
            false,
        ));
        assert_eq!(component_id, 1);
        let library = BoardLibrary {
            padstacks: vec![BoardPadstack {
                name: "spike_ml".to_string(),
                shapes: vec![
                    Some(BoardShape::Tile(TileShape::RegularTileShape(
                        RegularTileShape::IntBox(IntBox::new(
                            IntPoint::new(-2_000, -1_000),
                            IntPoint::new(6_000, 1_000),
                        )),
                    ))),
                    Some(BoardShape::Tile(TileShape::RegularTileShape(
                        RegularTileShape::IntBox(IntBox::new(
                            IntPoint::new(1_000, -5_000),
                            IntPoint::new(11_000, 4_000),
                        )),
                    ))),
                ],
                drillable: false,
                placed_absolute: false,
                hole_only: false,
            }],
            packages: vec![BoardPackage {
                name: "spike_ml_pkg".to_string(),
                pins: vec![BoardPackagePin {
                    name: "1".to_string(),
                    padstack_no: 1,
                    rel_location: IntPoint::new(20_000, 10_000),
                    rotation: 90.0,
                }],
            }],
        };
        (components, library)
    }

    /// Renders a tile shape's four corners exactly like the spike's
    /// `corners()` helper (`x,y;x,y;x,y;x,y`), so assertions quote the
    /// capture lines verbatim.
    fn int_box_corners(shape: &BoardShape) -> String {
        let BoardShape::Tile(TileShape::RegularTileShape(RegularTileShape::IntBox(b))) = shape
        else {
            panic!("expected an IntBox shape, got {shape:?}");
        };
        (0..4)
            .map(|no| {
                let corner = b.corner(no);
                format!("{},{}", corner.x, corner.y)
            })
            .collect::<Vec<_>>()
            .join(";")
    }

    /// The flip-style re-query (`PinResolutionSpike.java` Phase 6a):
    /// the SAME fixture pin — C23's pin 0, relRaw (-6750, 0), back-side
    /// rotation 45 (the FLOAT branch) — under BOTH styles. Captures:
    ///   PIN      comp=93 ... pinIdx=0 ... relRaw=-6750 0 rel=4773 4773
    ///           branch=float mirror=before   (the fixture default)
    ///   FLIPSTYLE comp=93 pinIdx=0 rel=4773 -4773          (rotate-first)
    /// The mirror-AFTER arm flips the y sign of the float branch; a
    /// port that mirrors before rotating under BOTH styles returns
    /// (4773, 4773) here and fails. (Location-free: rel is
    /// component-relative, so the crafted table needs only the
    /// rotation/side/relRaw triple.)
    #[test]
    fn flip_style_requery_flips_the_mirror_order_on_the_float_branch() {
        // mirror-FIRST (the committed fixture capture): mirror, rotate.
        let (mirror_first, library) =
            pin_table(false, placed(45.0, false), IntPoint::new(-6_750, 0), 0.0);
        match pin_relative_location(&mirror_first, &library, 1, 0).expect("resolves") {
            Vector::Int(v) => assert_eq!((v.x, v.y), (4_773, 4_773), "mirror BEFORE"),
            other => panic!("expected an integer vector, got {other:?}"),
        }
        // rotate-FIRST (the FLIPSTYLE capture): rotate, mirror.
        let (rotate_first, library) =
            pin_table(true, placed(45.0, false), IntPoint::new(-6_750, 0), 0.0);
        match pin_relative_location(&rotate_first, &library, 1, 0).expect("resolves") {
            Vector::Int(v) => assert_eq!((v.x, v.y), (4_773, -4_773), "mirror AFTER"),
            other => panic!("expected an integer vector, got {other:?}"),
        }
    }

    /// The multi-layer synthetic under ROTATE-FIRST (Phase 6b) — the
    /// capture line, quoted whole:
    ///   MLPIN comp=96 rel=10000 20000 rawCenter=1510000 -880000
    ///   shape0=1511000,-884000;1521000,-884000;1521000,-875000;1511000,-875000
    ///   shape1=1508000,-881000;1516000,-881000;1516000,-879000;1508000,-879000
    ///   gravity=1516000.0 -879500.0 center=1516000 -879500
    /// This one geometry pins FOUR `Pin.getShape` behaviors at once:
    /// the back-side padstack-layer remap (shape index 0 reads the L1
    /// box — an identity remap returns the flat L0 box), the
    /// PIN-ROTATION arm (the box actually turns), the mirror-AFTER
    /// arm, and the getCenter CORRECTION fired through the full chain
    /// (the raw center (1510000, -880000) lies outside shape0, so the
    /// center becomes `centreOfGravity().round()`).
    #[test]
    fn rotate_first_multi_layer_pin_remaps_layers_and_corrects_center() {
        let (components, library) = spike2_geometry(true);
        match pin_relative_location(&components, &library, 1, 0).expect("resolves") {
            Vector::Int(v) => assert_eq!((v.x, v.y), (10_000, 20_000), "rel"),
            other => panic!("expected an integer vector, got {other:?}"),
        }
        let shape0 = pin_shape(&components, &library, 1, 0, 0).expect("shape 0");
        assert_eq!(
            int_box_corners(&shape0),
            "1511000,-884000;1521000,-884000;1521000,-875000;1511000,-875000",
            "the L1 box through the rotate-first chain (remap + pin rot + mirror-after)"
        );
        let shape1 = pin_shape(&components, &library, 1, 0, 1).expect("shape 1");
        assert_eq!(
            int_box_corners(&shape1),
            "1508000,-881000;1516000,-881000;1516000,-879000;1508000,-879000",
            "the L0 box through the same chain (index 1 remaps to layer 0)"
        );
        // The raw center loc + rel lies OUTSIDE the first shape, so
        // the center is the corrected gravity — not loc + rel.
        let raw = IntPoint::new(1_500_000 + 10_000, -900_000 + 20_000);
        assert_eq!((raw.x, raw.y), (1_510_000, -880_000));
        assert!(
            !shape_contains_inside(&shape0, &Point::Int(raw)),
            "the correction precondition holds"
        );
        match pin_center(&components, &library, 1, 0).expect("resolves") {
            Point::Int(center) => {
                assert_eq!((center.x, center.y), (1_516_000, -879_500), "gravity round");
                assert_ne!((center.x, center.y), (raw.x, raw.y));
            }
            other => panic!("expected an integer center, got {other:?}"),
        }
    }

    /// The same geometry under MIRROR-FIRST (Phase 6c, the FRESH
    /// SPIKE3 pin — re-querying the SPIKE2 pin would return Java's
    /// memoized shapes, `Pin.java:170-236`; module docs). Capture:
    ///   SPIKE3_MIRRORFIRST comp=97 rel=-10000 -20000
    ///   shape0=1479000,-925000;1489000,-925000;1489000,-916000;1479000,-916000
    ///   shape1=1484000,-921000;1492000,-921000;1492000,-919000;1484000,-919000
    ///   center=1484000 -920500
    /// The mirror-BEFORE arm: the rel AND the shape mirror BEFORE the
    /// rotations, with no mirror after. Both styles' shape0 boxes
    /// differ in every coordinate — neither branch can pass the
    /// other's pin.
    #[test]
    fn mirror_first_multi_layer_pin_mirrors_before_the_rotations() {
        let (components, library) = spike2_geometry(false);
        match pin_relative_location(&components, &library, 1, 0).expect("resolves") {
            Vector::Int(v) => assert_eq!((v.x, v.y), (-10_000, -20_000), "rel"),
            other => panic!("expected an integer vector, got {other:?}"),
        }
        let shape0 = pin_shape(&components, &library, 1, 0, 0).expect("shape 0");
        assert_eq!(
            int_box_corners(&shape0),
            "1479000,-925000;1489000,-925000;1489000,-916000;1479000,-916000"
        );
        let shape1 = pin_shape(&components, &library, 1, 0, 1).expect("shape 1");
        assert_eq!(
            int_box_corners(&shape1),
            "1484000,-921000;1492000,-921000;1492000,-919000;1484000,-919000"
        );
        match pin_center(&components, &library, 1, 0).expect("resolves") {
            Point::Int(center) => {
                assert_eq!((center.x, center.y), (1_484_000, -920_500), "gravity round")
            }
            other => panic!("expected an integer center, got {other:?}"),
        }
    }

    /// Upstream `Pin.getBasePinName` (14b28b6ff, the
    /// `testBasePinNameNormalization` analogue): every separator
    /// family, the family ORDER (`@` before `#` before `_` before
    /// `-`), the digit-only suffix rule, and the edge guards. Each
    /// row kills its own mutant family: order mutants die on the
    /// cross-family rows, digit-check mutants on `A_B`/`A-B`, and
    /// guard mutants on `_1`/`PAD_`.
    #[test]
    fn base_pin_name_strips_composite_subpad_suffixes() {
        let cases: &[(&str, &str)] = &[
            // @ family: first occurrence, no digit requirement.
            ("PAD@1", "PAD"),
            ("PAD@10", "PAD"),
            ("PAD@x", "PAD"),
            ("PAD@1@2", "PAD"),
            // # family: after @, before _.
            ("P#2", "P"),
            ("PAD_1#1", "PAD_1"),
            // _ family: LAST separator, all-digits suffix, not at an edge.
            ("pad_1", "pad"),
            ("pad_1_1", "pad_1"),
            ("1_1", "1"),
            ("P_12", "P"),
            ("A_B", "A_B"),
            ("_1", "_1"),
            ("PAD_", "PAD_"),
            // - family: tried after _ fails its digit test.
            ("1-1", "1"),
            ("P-2", "P"),
            ("pad_1-1", "pad_1"),
            ("A-B", "A-B"),
            // No separator at all.
            ("9", "9"),
            ("", ""),
        ];
        for (input, expected) in cases {
            assert_eq!(base_pin_name(input), *expected, "input {input:?}");
        }
    }
}
