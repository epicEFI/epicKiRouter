//! Drill geometry — `DrillItem`'s precalculated spans (M2 Task 4).
//!
//! Java anchor: `board/model/items/DrillItem.java` — the common
//! superclass of `Pin` and `Via` ("Common superclass for Pins and
//! Vias", `:24`). Its three lazily-precalculated fields
//! (`precalculatedMinWidth` `:34`, `precalculatedFirstLayer` `:40`,
//! `precalculatedLastLayer` `:46`) are MEMOIZED per item with the
//! sentinel `-1`; the port keeps the trio in
//! [`DrillPrecalc`] with `Option`s and memoizes it Board-side
//! ([`crate::board::Board`]'s `drill_precalc` table — the arena's
//! `ItemEntry` carries only persistent payload, see its docs).
//!
//! ## The clear-on-change contract (D19-observable)
//!
//! `DrillItem.clearDerivedData()` (`DrillItem.java:390-395`) clears
//! ONLY `precalculatedFirstLayer`/`precalculatedLastLayer` —
//! `precalculatedMinWidth` is NEVER reset in Java, not even by the
//! geometry mutators (`translateBy` `:62-68`, `turn90Degree`
//! `:70-76`, `rotateApprox` `:78-85`, `changePlacementSide` `:87-93`
//! all call `clearDerivedData`). A moved drill therefore keeps
//! reporting its STALE min width until something else recomputes it —
//! except the width is center-independent for symmetric padstacks, so
//! the staleness is unobservable in practice. The port reproduces the
//! exact asymmetry in [`crate::board::Board::clear_derived_data`] and
//! pins it; "fixing" it would diverge.
//!
//! ## Evidence (jar spike `rust/harness/oracle/ItemGeometrySpike.java`,
//! captures `/tmp/epic-t4-items-bm08.out` + `/tmp/epic-t4-items-ch.out`)
//!
//! Fixture pins quote the `DRILL` capture lines verbatim; the
//! synthetic rows (partial-span via, back-side pin, full-span
//! asymmetric via, non-signal skip) were inserted through the REAL
//! Java board API (`insertVia`/`insertPin`) in the spike and are
//! reproduced here through hand-built tables with identical inputs —
//! the same free functions [`crate::board::Board`] delegates to.

use epic_geometry::int_point::IntPoint;

use crate::components::{BoardPadstack, shape_translate_by};
use crate::items::BoardShape;
use crate::layers::LayerStructure;
use epic_geometry::vector::Vector;

/// The memoizable `DrillItem` state (module docs): `None` is Java's
/// `-1` sentinel. Owned by the Board side table, never by the item
/// entry.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DrillPrecalc {
    /// Java `precalculatedFirstLayer`.
    pub first_layer: Option<i32>,
    /// Java `precalculatedLastLayer`.
    pub last_layer: Option<i32>,
    /// Java `precalculatedMinWidth`.
    pub min_width: Option<f64>,
}

/// Java `DrillItem.firstLayer()` for a VIA (`DrillItem.java:162-172`)
/// — [`pin_first_layer`] with `isPlacedOnFront() == true` baked in:
/// `Via` does NOT override `isPlacedOnFront` (`DrillItem.java:363-366`
/// returns true; only `Pin.java:482` overrides), so a via always takes
/// the padstack's own span (the `placed_absolute` arm stays for
/// absolute padstacks, which skip the mirror either way).
#[must_use]
pub fn via_first_layer(padstack: &BoardPadstack) -> i32 {
    // Via.isPlacedOnFront() == true -> the first arm of DrillItem
    // :165 (`isPlacedOnFront() || padstack.placedAbsolute`).
    padstack.from_layer() as i32
}

/// Java `DrillItem.lastLayer()` for a VIA (`DrillItem.java:174-185`)
/// — the same `isPlacedOnFront() == true` reduction.
#[must_use]
pub fn via_last_layer(padstack: &BoardPadstack) -> i32 {
    padstack.to_layer()
}

/// Java `Via.getShape(index)` (`Via.java:115-132`): the raw padstack
/// shape at `index + firstLayer()`, translated by the via CENTER.
/// (Java memoizes the array in `precalculatedShapes` — a cache
/// artifact with no observable read for a fixed padstack; the port
/// computes per call. A null padstack layer slot yields `None`,
/// Java's null entry.)
#[must_use]
pub fn via_shape(padstack: &BoardPadstack, center: IntPoint, index: i32) -> Option<BoardShape> {
    let padstack_layer = index + via_first_layer(padstack);
    let raw = padstack.get_shape(usize::try_from(padstack_layer).ok()?)?;
    let translate = Vector::Int(epic_geometry::int_vector::IntVector::new(
        center.x, center.y,
    ));
    Some(shape_translate_by(raw.clone(), &translate))
}

/// Java `DrillItem.minWidth()` (`DrillItem.java:368-388`) — the pure
/// loop half (the memoization is Board-side):
///
/// - the accumulator starts at `Integer.MAX_VALUE` AS A DOUBLE
///   (`:371` — `2147483647.0`, reproduced via [`i32::MAX`]; a drill
///   whose every layer is non-signal or shapeless keeps exactly this
///   value, which is how a no-shape pin reports `2.1e9`),
/// - layers OUTSIDE the item's span are not visited, and NON-SIGNAL
///   layers are SKIPPED (`:374-377` — the fixture pin on
///   `complex_hierarchy`'s power layer 0 exists to pin this skip),
/// - per layer the shape is fetched BY LAYER (`getShapeOnLayer =
///   getShape(layer - firstLayer)`, `:263-271`) and the running
///   minimum shrinks to the bounding box width AND height
///   (`:380-383`).
///
/// `shape_on_layer` receives the BOARD layer number and returns the
/// kind-specific shape (`via_shape` for a via, [`pin_shape`] with
/// `index = layer - first_layer` for a pin).
#[must_use]
pub fn drill_min_width(
    layers: &LayerStructure,
    first_layer: i32,
    last_layer: i32,
    mut shape_on_layer: impl FnMut(i32) -> Option<BoardShape>,
) -> f64 {
    let mut min_width = f64::from(i32::MAX);
    for current_layer in first_layer..=last_layer {
        let Some(layer) = usize::try_from(current_layer)
            .ok()
            .and_then(|index| layers.layers.get(index))
        else {
            continue;
        };
        if !layer.is_signal {
            // DrillItem.java:374-377: the non-signal skip.
            continue;
        }
        if let Some(shape) = shape_on_layer(current_layer) {
            let bounds = shape.bounding_box();
            min_width = min_width.min(f64::from(bounds.width()));
            min_width = min_width.min(f64::from(bounds.height()));
        }
    }
    min_width
}

/// Java `DrillItem.tileShapeCount()` (`DrillItem.java:202-208`) — the
/// PADSTACK span (`toLayer - fromLayer + 1`), NOT the item's
/// first/last layer: for a back-side pin these differ, and Java reads
/// the padstack directly.
#[must_use]
pub fn drill_tile_shape_count(padstack: &BoardPadstack) -> i32 {
    padstack.to_layer() - padstack.from_layer() as i32 + 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{
        BoardLibrary, BoardPackage, BoardPackagePin, Component, Components, pin_first_layer,
        pin_last_layer, pin_shape,
    };
    use epic_geometry::int_box::IntBox;
    use epic_geometry::int_vector::IntVector;
    use epic_geometry::regular_tile_shape::RegularTileShape;
    use epic_geometry::tile_shape::TileShape;

    /// An `IntBox` tile shape helper (the synthetic padstack shapes).
    fn box_shape(x0: i32, y0: i32, x1: i32, y1: i32) -> BoardShape {
        BoardShape::Tile(TileShape::RegularTileShape(RegularTileShape::IntBox(
            IntBox::new(IntPoint::new(x0, y0), IntPoint::new(x1, y1)),
        )))
    }

    /// Renders a tile shape's corners like the spike's `corners()`
    /// (`x,y;...`, exact ints), so assertions quote capture lines.
    fn corners(shape: &BoardShape) -> String {
        let BoardShape::Tile(tile) = shape else {
            panic!("expected a tile shape, got {shape:?}");
        };
        (0..tile.border_line_count())
            .map(|no| match tile.corner(no as i32) {
                epic_geometry::point::Point::Int(p) => format!("{},{}", p.x, p.y),
                other => panic!("expected an integer corner, got {other:?}"),
            })
            .collect::<Vec<_>>()
            .join(";")
    }

    /// The spike's `spike_partial` padstack (`ItemGeometrySpike.java`
    /// `syntheticDrills`): an ASYMMETRIC box on layer 1 only, on a
    /// 2-layer board — the partial-span via and the back-side pin
    /// below are its two placements.
    fn spike_partial_tables() -> (Components, BoardLibrary, LayerStructure) {
        let mut components = Components::new();
        components.add(Component::new(
            "SPIKE_BACK",
            Some(IntPoint::new(1_200_000, -800_000)),
            0.0,
            false,
            1,
            1,
            false,
        ));
        let library = BoardLibrary {
            padstacks: vec![BoardPadstack {
                name: "spike_partial".to_string(),
                shapes: vec![None, Some(box_shape(-1000, -1000, 5000, 1000))],
                drillable: true,
                placed_absolute: false,
                hole_only: false,
            }],
            packages: vec![BoardPackage {
                name: "spike_partial_pkg".to_string(),
                pins: vec![BoardPackagePin {
                    name: "1".to_string(),
                    padstack_no: 1,
                    rel_location: IntPoint::new(20_000, 10_000),
                    rotation: 0.0,
                }],
            }],
        };
        let layers = LayerStructure::new(vec![
            crate::layers::Layer::new("Top", true),
            crate::layers::Layer::new("Bottom", true),
        ]);
        (components, library, layers)
    }

    /// The partial-span SYNTHETIC VIA — capture lines
    /// (`/tmp/epic-t4-items-bm08.out`):
    /// ```text
    /// DRILL kind=synthVia id=42 center=700000 -300000 padstack=spike_partial
    ///   psFrom=1 psTo=1 psLayerCount=2 placedFront=true firstLayer=1
    ///   lastLayer=1 attach=false minWidth=2000.0
    /// SYNTH_VIA_SHAPE i=0 699000,-301000;705000,-301000;705000,-299000;699000,-299000
    /// ```
    /// The padstack's only shape sits on layer 1, so the via span is
    /// first=last=1 (a full-span port returns 0..1 and fails); the
    /// min width 2000.0 is the layer-1 box HEIGHT (5000-(-1000)=6000
    /// wide, 1000-(-1000)=2000 tall — an implementation that forgets
    /// the height half of `DrillItem.java:380-383` returns 6000).
    #[test]
    fn synthetic_partial_span_via_first_last_layer_min_width_and_shape() {
        let (_, library, layers) = spike_partial_tables();
        let padstack = library.padstack(1).expect("spike_partial");
        assert_eq!(via_first_layer(padstack), 1, "psFrom=1, front-placed");
        assert_eq!(via_last_layer(padstack), 1, "psTo=1");
        assert_eq!(drill_tile_shape_count(padstack), 1, "1 - 1 + 1");

        let center = IntPoint::new(700_000, -300_000);
        let width = drill_min_width(&layers, 1, 1, |layer| {
            via_shape(padstack, center, layer - 1)
        });
        assert_eq!(width, 2000.0, "minWidth=2000.0 (box height)");

        let shape = via_shape(padstack, center, 0).expect("shape 0");
        assert_eq!(
            corners(&shape),
            "699000,-301000;705000,-301000;705000,-299000;699000,-299000",
            "SYNTH_VIA_SHAPE i=0"
        );
        // The shape is the RAW layer-1 box translated by the center
        // (Via.java:123-130) — anchor-blind form: untranslated corners
        // (-1000,-1000...) or a wrong layer slot both miss.
        assert_ne!(
            corners(&shape),
            "-1000,-1000;5000,-1000;5000,1000;-1000,1000"
        );
    }

    /// The partial-span BACK-SIDE PIN — capture lines:
    /// ```text
    /// DRILL kind=synthBackPin id=43 center=1180000 -790000 padstack=spike_partial
    ///   psFrom=1 psTo=1 psLayerCount=2 placedFront=false firstLayer=0
    ///   lastLayer=0 attach=false minWidth=2000.0
    /// SYNTH_BACKPIN_SHAPE i=0 1175000,-791000;1181000,-791000;1181000,-789000;1175000,-789000
    /// ```
    /// The MIRRORED span (`DrillItem.java:168`/`:181`): layer count 2
    /// minus to/from — first=last=0, NOT the padstack's 1..1. The
    /// shape runs the full `Pin.getShape` chain (mirror-BEFORE at
    /// rel (20000,10000) -> (-20000,10000), + component location).
    #[test]
    fn synthetic_back_side_pin_mirrored_span_and_shape() {
        let (components, library, layers) = spike_partial_tables();
        let component = components.get(1).expect("SPIKE_BACK");
        assert!(!component.placed_on_front(), "back side");
        let package_pin = library
            .package(component.package_no())
            .expect("package")
            .get_pin(0)
            .expect("pin 0");
        let padstack = library.padstack(package_pin.padstack_no).expect("padstack");
        assert_eq!(pin_first_layer(component, padstack), 0, "2 - 1 - 1");
        assert_eq!(pin_last_layer(component, padstack), 0, "2 - 1 - 1");

        let width = drill_min_width(&layers, 0, 0, |layer| {
            // shape index = layer - first_layer, and first_layer == 0.
            pin_shape(&components, &library, 1, 0, layer)
        });
        assert_eq!(width, 2000.0, "minWidth=2000.0 through the pin chain");

        let shape = pin_shape(&components, &library, 1, 0, 0).expect("shape 0");
        assert_eq!(
            corners(&shape),
            "1175000,-791000;1181000,-791000;1181000,-789000;1175000,-789000",
            "SYNTH_BACKPIN_SHAPE i=0"
        );
    }

    /// The full-span via with DISTINCT per-layer shapes — capture:
    /// ```text
    /// DRILL kind=synthFullVia id=44 center=650000 -250000 padstack=spike_full
    ///   psFrom=0 psTo=1 psLayerCount=2 placedFront=true firstLayer=0
    ///   lastLayer=1 attach=false minWidth=2000.0
    /// SYNTH_FULL_VIA_SHAPE i=0 645000,-255000;655000,-255000;655000,-245000;645000,-245000
    /// SYNTH_FULL_VIA_SHAPE i=1 648000,-251000;652000,-251000;652000,-249000;648000,-249000
    /// ```
    /// minWidth=2000.0 is the MINIMUM over both layers (layer 0's box
    /// is 10000x10000, layer 1's is 4000x2000 — a port that reads
    /// only the first layer returns 10000.0).
    #[test]
    fn synthetic_full_span_via_min_width_spans_both_layers() {
        let (_, library, layers) = spike_partial_tables();
        let library = BoardLibrary {
            padstacks: vec![BoardPadstack {
                name: "spike_full".to_string(),
                shapes: vec![
                    Some(box_shape(-5000, -5000, 5000, 5000)),
                    Some(box_shape(-2000, -1000, 2000, 1000)),
                ],
                drillable: true,
                placed_absolute: false,
                hole_only: false,
            }],
            ..library
        };
        let padstack = library.padstack(1).expect("spike_full");
        assert_eq!(via_first_layer(padstack), 0);
        assert_eq!(via_last_layer(padstack), 1);
        assert_eq!(drill_tile_shape_count(padstack), 2);

        let center = IntPoint::new(650_000, -250_000);
        assert_eq!(
            corners(&via_shape(padstack, center, 0).expect("shape 0")),
            "645000,-255000;655000,-255000;655000,-245000;645000,-245000",
            "SYNTH_FULL_VIA_SHAPE i=0"
        );
        assert_eq!(
            corners(&via_shape(padstack, center, 1).expect("shape 1")),
            "648000,-251000;652000,-251000;652000,-249000;648000,-249000",
            "SYNTH_FULL_VIA_SHAPE i=1"
        );
        let width = drill_min_width(&layers, 0, 1, |layer| via_shape(padstack, center, layer));
        assert_eq!(width, 2000.0, "the minimum over BOTH layers");
    }

    /// The NON-SIGNAL SKIP (`DrillItem.java:374-377`) — capture
    /// (`/tmp/epic-t4-items-ch.out`):
    /// ```text
    /// DRILL kind=synthPowerVia id=1133 center=150000 -60000 padstack=spike_power
    ///   psFrom=0 psTo=1 psLayerCount=2 placedFront=true firstLayer=0
    ///   lastLayer=1 attach=false minWidth=4000.0
    /// ```
    /// `complex_hierarchy`'s layer 0 (`top_copper`) is `(type power)`
    /// — NOT signal — and the spike put the SMALLER box (1000x1000)
    /// there, the larger (4000x4000) on the signal layer 1: minWidth
    /// 4000.0 PROVES the skip; a port without it returns 1000.0.
    #[test]
    fn min_width_skips_non_signal_layers() {
        let layers = LayerStructure::new(vec![
            crate::layers::Layer::new("top_copper", false),
            crate::layers::Layer::new("bottom_copper", true),
        ]);
        let library = BoardLibrary {
            padstacks: vec![BoardPadstack {
                name: "spike_power".to_string(),
                shapes: vec![
                    Some(box_shape(-500, -500, 500, 500)),
                    Some(box_shape(-2000, -2000, 2000, 2000)),
                ],
                drillable: true,
                placed_absolute: false,
                hole_only: false,
            }],
            ..BoardLibrary::default()
        };
        let padstack = library.padstack(1).expect("spike_power");
        let center = IntPoint::new(150_000, -60_000);
        let width = drill_min_width(&layers, 0, 1, |layer| via_shape(padstack, center, layer));
        assert_eq!(width, 4000.0, "minWidth=4000.0 — the power layer skipped");
        assert_ne!(width, 1000.0, "the no-skip answer");
    }

    /// The no-shape arm (`DrillItem.java:371`): every layer shapeless
    /// or non-signal keeps the `Integer.MAX_VALUE` accumulator —
    /// exact 2147483647.0, not f64::MAX and not 0.
    #[test]
    fn min_width_keeps_integer_max_when_nothing_contributes() {
        let layers = LayerStructure::new(vec![crate::layers::Layer::new("L0", true)]);
        let library = BoardLibrary {
            padstacks: vec![BoardPadstack {
                name: "empty".to_string(),
                shapes: vec![None],
                drillable: true,
                placed_absolute: false,
                hole_only: false,
            }],
            ..BoardLibrary::default()
        };
        let padstack = library.padstack(1).expect("empty");
        let width = drill_min_width(&layers, 0, 0, |layer| {
            via_shape(padstack, IntPoint::ZERO, layer)
        });
        assert_eq!(width, f64::from(i32::MAX), "Integer.MAX_VALUE as double");
    }

    /// `Via.getShape` translates by the CENTER (`Via.java:123-130`) —
    /// and only that (no rotation/mirror arms exist for a via). The
    /// vector construction is the port's
    /// `difference_by(Point.ZERO)`: `IntVector(center.x, center.y)`.
    #[test]
    fn via_shape_translate_vector_is_the_center() {
        let (_, library, _) = spike_partial_tables();
        let padstack = library.padstack(1).expect("spike_partial");
        let center = IntPoint::new(700_000, -300_000);
        let translate = Vector::Int(IntVector::new(center.x, center.y));
        let direct = shape_translate_by(box_shape(-1000, -1000, 5000, 1000), &translate);
        assert_eq!(via_shape(padstack, center, 0), Some(direct));
        assert_eq!(via_shape(padstack, center, -2), None, "below the span");
    }
}
