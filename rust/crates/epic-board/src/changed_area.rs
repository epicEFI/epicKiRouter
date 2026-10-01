//! The changed-area marker (Java `board/state/ChangedArea.java`, 120
//! lines, ported in full).
//!
//! Java's `RoutingBoard.changedArea` is a TRANSIENT session object: it
//! is `null` outside a marking session and created lazily by
//! `startMarkingChangedArea` (`RoutingBoardOperations.java:29-34`) —
//! nested starts keep the existing session (`if (board.changedArea ==
//! null)`), and `optChangedArea` (`:52-79`) nulls it at the end. The
//! Rust port models the session as `Option<ChangedArea>` on
//! [`crate::board::Board`] (`changed_area`), so every Java `changedArea
//! != null` guard is a `match`/`map` on the option.
//!
//! ## The NPE-flip contract (T10c's heaviest parity item)
//!
//! Inside the shove drivers the two read faces of the session differ:
//!
//! * [`TraceShover.insert`'s tail](crate::trace_shover::insert) calls
//!   `currentSubstituteTrace.normalize(board.changedArea.getArea(layer))`
//!   inside a swallowing try/catch (Java :545-550): with the session
//!   null, the `getArea` dereference NPEs and the WHOLE normalization is
//!   skipped (bug-144) — skip-when-no-session, run-when-marking.
//! * `ForcedPadRouter` (:440-450) and `BasicBoard.insertTrace`
//!   (:223-229) read the area into a LOCAL first (`optArea = null` when
//!   the session is null) and call `normalize(optArea)`
//!   unconditionally — the null clip means unbounded normalization.
//!
//! Ports must preserve this asymmetry; normalizing unconditionally at a
//! direct-deref site merges perpendicular end contacts outside sessions
//! (the bug-144 lesson, see `.wolf/buglog.json`).

use epic_geometry::float_point::FloatPoint;
use epic_geometry::int_box::IntBox;
use epic_geometry::int_octagon::IntOctagon;
use epic_geometry::int_point::IntPoint;
use epic_geometry::tile_shape::TileShape;

/// The single-layer mutable octagon (Java private class
/// `ChangedArea.MutableOctagon`): axis bounds plus the two diagonal
/// bands (`x - y` for `ulx`/`lrx`, `x + y` for `llx`/`urx`).
#[derive(Clone, Copy, Debug)]
struct MutableOctagon {
    lx: f64,
    ly: f64,
    rx: f64,
    uy: f64,
    ulx: f64,
    lrx: f64,
    llx: f64,
    urx: f64,
}

impl MutableOctagon {
    /// Java `setEmpty()`: every min-bound to `Integer.MAX_VALUE`, every
    /// max-bound to `Integer.MIN_VALUE` (widened to f64 exactly).
    fn set_empty(&mut self) {
        self.lx = f64::from(i32::MAX);
        self.ly = f64::from(i32::MAX);
        self.rx = f64::from(i32::MIN);
        self.uy = f64::from(i32::MIN);
        self.ulx = f64::from(i32::MAX);
        self.lrx = f64::from(i32::MIN);
        self.llx = f64::from(i32::MAX);
        self.urx = f64::from(i32::MIN);
    }

    /// Java `toInt()`: the smallest `IntOctagon` containing this
    /// octagon — `EMPTY` when inverted on any band, else the
    /// floor/ceil-rounded bounds in the Java ctor order
    /// (lx, ly, rx, uy, ulx, lrx, llx, urx).
    fn to_int(self) -> IntOctagon {
        if self.rx < self.lx || self.uy < self.ly || self.lrx < self.ulx || self.urx < self.llx {
            return IntOctagon::EMPTY;
        }
        IntOctagon::new(
            self.lx.floor() as i32,
            self.ly.floor() as i32,
            self.rx.ceil() as i32,
            self.uy.ceil() as i32,
            self.ulx.floor() as i32,
            self.lrx.ceil() as i32,
            self.llx.floor() as i32,
            self.urx.ceil() as i32,
        )
    }
}

/// Java `board/state/ChangedArea.java`: the per-layer octagon union
/// grown by `join` and read back through [`ChangedArea::get_area`]
/// (the normalize clips) and [`ChangedArea::surrounding_box`] (the
/// graphics update box in `optChangedArea`).
#[derive(Clone, Debug)]
pub struct ChangedArea {
    layer_count: usize,
    arr: Vec<MutableOctagon>,
}

impl ChangedArea {
    /// Java ctor: every layer starts EMPTY.
    pub fn new(layer_count: usize) -> ChangedArea {
        let mut arr = Vec::with_capacity(layer_count);
        for _ in 0..layer_count {
            let mut oct = MutableOctagon {
                lx: 0.0,
                ly: 0.0,
                rx: 0.0,
                uy: 0.0,
                ulx: 0.0,
                lrx: 0.0,
                llx: 0.0,
                urx: 0.0,
            };
            oct.set_empty();
            arr.push(oct);
        }
        ChangedArea { layer_count, arr }
    }

    /// Java `join(FloatPoint point, int layer)` (`:25-39`): enlarge the
    /// layer octagon so that it contains the point — min/max on the
    /// axis bounds and on both diagonal bands.
    pub fn join_point(&mut self, point: &FloatPoint, layer: i32) {
        let current = &mut self.arr[layer as usize];
        current.lx = current.lx.min(point.x);
        current.ly = current.ly.min(point.y);
        current.rx = current.rx.max(point.x);
        current.uy = current.uy.max(point.y);

        let mut tmp = point.x - point.y;
        current.ulx = current.ulx.min(tmp);
        current.lrx = current.lrx.max(tmp);

        tmp = point.x + point.y;
        current.llx = current.llx.min(tmp);
        current.urx = current.urx.max(tmp);
    }

    /// Java `join(TileShape shape, int layer)` (`:42-50`): join every
    /// border corner approximation. A null shape is a no-op (the
    /// caller models Java null as `None`).
    pub fn join_shape(&mut self, shape: Option<&TileShape>, layer: i32) {
        let Some(shape) = shape else {
            return;
        };
        let corner_count = shape.border_line_count();
        for i in 0..corner_count {
            let corner = shape
                .corner_approx(i as i32)
                .expect("border corner of a legal shape");
            self.join_point(&corner, layer);
        }
    }

    /// Java `getArea(int layer)` (`:53-56`): the layer marking octagon
    /// as an `IntOctagon`.
    pub fn get_area(&self, layer: i32) -> IntOctagon {
        self.arr[layer as usize].to_int()
    }

    /// Java `surroundingBox()` (`:58-74`): the floor/ceil box around
    /// every layer's axis bounds; `IntBox.EMPTY` when inverted.
    pub fn surrounding_box(&self) -> IntBox {
        let mut llx = i32::MAX;
        let mut lly = i32::MAX;
        let mut urx = i32::MIN;
        let mut ury = i32::MIN;
        for current in &self.arr {
            llx = llx.min(current.lx.floor() as i32);
            lly = lly.min(current.ly.floor() as i32);
            urx = urx.max(current.rx.ceil() as i32);
            ury = ury.max(current.uy.ceil() as i32);
        }
        if llx > urx || lly > ury {
            return IntBox::EMPTY;
        }
        IntBox::new(IntPoint::new(llx, lly), IntPoint::new(urx, ury))
    }

    /// Java `setEmpty(int layer)` (`:77-79`).
    pub fn set_empty(&mut self, layer: i32) {
        self.arr[layer as usize].set_empty();
    }

    /// Java `layerCount` field (package-private read for the marking
    /// functions).
    pub fn layer_count(&self) -> usize {
        self.layer_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use epic_geometry::int_point::IntPoint;
    use epic_geometry::regular_tile_shape::RegularTileShape;

    fn fp(x: f64, y: f64) -> FloatPoint {
        FloatPoint::new(x, y)
    }

    /// A fresh marker is EMPTY on every layer; `getArea` returns
    /// `IntOctagon.EMPTY` and `surroundingBox` the empty box.
    #[test]
    fn fresh_marker_is_empty() {
        let marker = ChangedArea::new(4);
        for layer in 0..4 {
            assert_eq!(marker.get_area(layer), IntOctagon::EMPTY);
        }
        assert_eq!(marker.surrounding_box(), IntBox::EMPTY);
        assert_eq!(marker.layer_count(), 4);
    }

    /// `joinPoint` grows the axis bounds AND both diagonal bands; the
    /// toInt rounding is floor on the min bounds (incl. ulx, llx) and
    /// ceil on the max bounds (incl. lrx, urx).
    #[test]
    fn join_point_grows_all_bands() {
        let mut marker = ChangedArea::new(2);
        marker.join_point(&fp(10.0, 20.0), 0);
        marker.join_point(&fp(-3.4, 5.6), 0);
        let area = marker.get_area(0);
        // lx = floor(-3.4) = -4, ly = floor(5.6) = 5,
        // rx = ceil(10) = 10, uy = ceil(20) = 20,
        // ulx = min(MAX, 10-20, -3.4-5.6) = -10 (floor stays -10),
        // lrx = max(MIN, -10, -9.0) = -9 (ceil stays -9),
        // llx = floor(-3.4 + 5.6) = floor(2.2) = 2,
        // urx = ceil(10 + 20) = 30.
        assert_eq!(
            area,
            IntOctagon::new(-4, 5, 10, 20, -10, -9, 2, 30),
            "floor/ceil rounding per band"
        );
        // The second layer is untouched.
        assert_eq!(marker.get_area(1), IntOctagon::EMPTY);
    }

    /// The inversion check of `toInt`: a single joined point gives
    /// degenerate-but-valid bounds; setEmpty restores EMPTY.
    #[test]
    fn set_empty_restores_the_empty_octagon() {
        let mut marker = ChangedArea::new(1);
        marker.join_point(&fp(7.0, 9.0), 0);
        assert_ne!(marker.get_area(0), IntOctagon::EMPTY);
        marker.set_empty(0);
        assert_eq!(marker.get_area(0), IntOctagon::EMPTY);
        assert_eq!(marker.surrounding_box(), IntBox::EMPTY);
    }

    /// `joinShape` joins every border corner: an IntBox shape's
    /// octagon hull is its bounding octagon.
    #[test]
    fn join_shape_covers_the_border() {
        let mut marker = ChangedArea::new(1);
        let shape = TileShape::RegularTileShape(RegularTileShape::IntBox(IntBox::new(
            IntPoint::new(0, 0),
            IntPoint::new(10, 10),
        )));
        marker.join_shape(Some(&shape), 0);
        assert_eq!(
            marker.get_area(0),
            shape.bounding_octagon().expect("box has an octagon hull")
        );
        // The Java null-shape arm: a no-op.
        marker.join_shape(None, 0);
        assert_eq!(
            marker.get_area(0),
            shape.bounding_octagon().expect("box has an octagon hull")
        );
    }

    /// `surroundingBox` unions the per-layer axis bounds with floor on
    /// the low side and ceil on the high side.
    #[test]
    fn surrounding_box_unions_layers() {
        let mut marker = ChangedArea::new(2);
        marker.join_point(&fp(1.2, 3.7), 0);
        marker.join_point(&fp(50.0, 60.0), 1);
        marker.join_point(&fp(-8.0, -9.0), 1);
        let b = marker.surrounding_box();
        assert_eq!(
            (b.ll.x, b.ll.y, b.ur.x, b.ur.y),
            (-8, -9, 50, 60),
            "floor of the mins, ceil of the maxes across layers"
        );
    }
}
