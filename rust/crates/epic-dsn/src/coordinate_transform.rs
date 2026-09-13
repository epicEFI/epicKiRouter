//! Port of `app.freerouting.io.CoordinateTransform`: scaling between board
//! coordinates and external (Specctra DSN / KiCad) coordinates, both
//! directions.
//!
//! Java stores `scaleFactor`, `baseX`, `baseY` as doubles and every method is
//! a single IEEE-754 multiply/divide/add chain — Rust `f64` reproduces the
//! bits exactly with the same operation order. Note the asymmetry:
//! `dsnToBoard` subtracts the base before scaling, `boardToDsn` adds it after
//! scaling (pinned by `dsn_to_board_tuple`/`board_to_dsn_point` tests — the
//! jar session captures `boardToDsn(16) + (-2.25) = -0.6499999999999999`,
//! i.e. division first, then base add).
//!
//! The two shape-producing overloads (`boardToDsn(board.Shape, Layer)` /
//! `boardToDsnRel(...)`, `CoordinateTransform.java:90-107,138-155`) return
//! parser `Shape` objects and land with `shape.rs` (M1b Task 3); everything
//! else in the file is ported here.
//!
//! [`calc_scale_factor`] is the scale-factor prelude of
//! `Structure.createBoard` (`Structure.java:1189-1203`): it derives the
//! transform's scale from the DSN `(resolution ...)` value and shrinks it by
//! powers of ten when the bounding box risks integer overflow (T25). The
//! division `scaleFactor /= 10` is **integer** division — it can reach 0
//! (jar session `/tmp/epic-t25-transform.jsh`, output
//! `/tmp/epic-t25-transform.out`: resolution 15 with 1e7 coordinates yields
//! `P3 sf=0.0` after two passes, `15/10=1` then `1/10=0`).

use epic_geometry::float_point::FloatPoint;
use epic_geometry::int_box::IntBox;
use epic_geometry::limits::CRIT_INT;
use epic_geometry::line::Line;
use epic_geometry::vector::Vector;

/// Java `CoordinateTransform` (`io/CoordinateTransform.java:20-31`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoordinateTransform {
    /// Java `scaleFactor`.
    pub scale_factor: f64,
    /// Java `baseX`.
    pub base_x: f64,
    /// Java `baseY`.
    pub base_y: f64,
}

impl CoordinateTransform {
    /// Java constructor (`CoordinateTransform.java:27-31`).
    pub fn new(scale_factor: f64, base_x: f64, base_y: f64) -> Self {
        Self {
            scale_factor,
            base_x,
            base_y,
        }
    }

    /// Java `boardToDsn(double)` (`:34-36`): scales a value from the board to
    /// the external coordinate system.
    pub fn board_to_dsn_value(&self, value: f64) -> f64 {
        value / self.scale_factor
    }

    /// Java `dsnToBoard(double)` (`:158-160`): scales a value from the
    /// external to the board coordinate system.
    pub fn dsn_to_board_value(&self, value: f64) -> f64 {
        value * self.scale_factor
    }

    /// Java `boardToDsn(FloatPoint)` (`:39-44`): point transform, base added
    /// *after* scaling.
    pub fn board_to_dsn_point(&self, point: FloatPoint) -> [f64; 2] {
        [
            self.board_to_dsn_value(point.x) + self.base_x,
            self.board_to_dsn_value(point.y) + self.base_y,
        ]
    }

    /// Java `boardToDsn(FloatPoint[])` (`:47-54`): interleaved x/y flat
    /// array, base added after scaling.
    pub fn board_to_dsn_points(&self, points: &[FloatPoint]) -> Vec<f64> {
        let mut result = Vec::with_capacity(2 * points.len());
        for point in points {
            result.push(self.board_to_dsn_value(point.x) + self.base_x);
            result.push(self.board_to_dsn_value(point.y) + self.base_y);
        }
        result
    }

    /// Java `boardToDsn(Line[])` (`:57-68`): 4 entries per line (a.x, a.y,
    /// b.x, b.y), base added after scaling.
    pub fn board_to_dsn_lines(&self, lines: &[Line]) -> Vec<f64> {
        let mut result = Vec::with_capacity(4 * lines.len());
        for line in lines {
            let a = line.a.to_float();
            let b = line.b.to_float();
            result.push(self.board_to_dsn_value(a.x) + self.base_x);
            result.push(self.board_to_dsn_value(a.y) + self.base_y);
            result.push(self.board_to_dsn_value(b.x) + self.base_x);
            result.push(self.board_to_dsn_value(b.y) + self.base_y);
        }
        result
    }

    /// Java `boardToDsn(Vector)` (`:71-77`): vector transform — base is NOT
    /// added (vectors are relative).
    pub fn board_to_dsn_vector(&self, vector: &Vector) -> [f64; 2] {
        let value = vector.to_float();
        [
            self.board_to_dsn_value(value.x),
            self.board_to_dsn_value(value.y),
        ]
    }

    /// Java `boardToDsn(IntBox)` (`:80-87`): ll.x, ll.y, ur.x, ur.y, base
    /// added after scaling.
    pub fn board_to_dsn_box(&self, bounding_box: &IntBox) -> [f64; 4] {
        [
            f64::from(bounding_box.ll.x) / self.scale_factor + self.base_x,
            f64::from(bounding_box.ll.y) / self.scale_factor + self.base_y,
            f64::from(bounding_box.ur.x) / self.scale_factor + self.base_x,
            f64::from(bounding_box.ur.y) / self.scale_factor + self.base_y,
        ]
    }

    /// Java `boardToDsnRel(FloatPoint)` (`:110-115`): relative (vector)
    /// coordinates — no base offset.
    pub fn board_to_dsn_rel_point(&self, point: FloatPoint) -> [f64; 2] {
        [
            self.board_to_dsn_value(point.x),
            self.board_to_dsn_value(point.y),
        ]
    }

    /// Java `boardToDsnRel(FloatPoint[])` (`:118-125`): interleaved x/y flat
    /// array, no base offset.
    pub fn board_to_dsn_rel_points(&self, points: &[FloatPoint]) -> Vec<f64> {
        let mut result = Vec::with_capacity(2 * points.len());
        for point in points {
            result.push(self.board_to_dsn_value(point.x));
            result.push(self.board_to_dsn_value(point.y));
        }
        result
    }

    /// Java `boardToDsnRel(IntBox)` (`:128-135`): no base offset.
    pub fn board_to_dsn_rel_box(&self, bounding_box: &IntBox) -> [f64; 4] {
        [
            f64::from(bounding_box.ll.x) / self.scale_factor,
            f64::from(bounding_box.ll.y) / self.scale_factor,
            f64::from(bounding_box.ur.x) / self.scale_factor,
            f64::from(bounding_box.ur.y) / self.scale_factor,
        ]
    }

    /// Java `dsnToBoard(double[])` (`:163-167`): external tuple to board
    /// point, base subtracted *before* scaling.
    pub fn dsn_to_board_tuple(&self, tuple: [f64; 2]) -> FloatPoint {
        FloatPoint::new(
            self.dsn_to_board_value(tuple[0] - self.base_x),
            self.dsn_to_board_value(tuple[1] - self.base_y),
        )
    }

    /// Java `dsnToBoardRel(double[])` (`:170-174`): relative coordinates, no
    /// base offset.
    pub fn dsn_to_board_rel_tuple(&self, tuple: [f64; 2]) -> FloatPoint {
        FloatPoint::new(
            self.dsn_to_board_value(tuple[0]),
            self.dsn_to_board_value(tuple[1]),
        )
    }
}

/// Scale-factor computation of `Structure.createBoard`
/// (`Structure.java:1189-1203`), transliterated exactly.
///
/// `scale_factor = max(resolution, 1)`; then, while `5 * max_coor` reaches
/// `Limits.CRIT_INT`, `scale_factor /= 10` (**i32 truncating division** — can
/// reach 0) and `max_coor /= 10` (double division). The caller computes
/// `max_coor` over the boundary bounding box as
/// `max(|coor[i] * resolution|)` (see [`max_abs_scaled_coordinate`]) and
/// handles the `max_coor == 0` outline-missing abort (`:1195-1198`) itself.
pub fn calc_scale_factor(resolution: i32, max_coor: f64) -> i32 {
    let mut scale_factor = resolution.max(1);
    let mut max_coor = max_coor;
    // Integer division on the scale factor — it can reach 0 (T25).
    while 5.0 * max_coor >= f64::from(CRIT_INT) {
        scale_factor /= 10;
        max_coor /= 10.0;
    }
    scale_factor
}

/// The `maxCoor` prelude of `Structure.createBoard`
/// (`Structure.java:1191-1194`): `maxCoor = max_i |coor[i] * resolution|`
/// over the four bounding-box coordinates (double multiply — no overflow).
pub fn max_abs_scaled_coordinate(coor: &[f64; 4], resolution: i32) -> f64 {
    let mut max_coor: f64 = 0.0;
    for &value in coor.iter() {
        max_coor = max_coor.max((value * f64::from(resolution)).abs());
    }
    max_coor
}

#[cfg(test)]
mod tests {
    use super::*;
    use epic_geometry::int_point::IntPoint;
    use epic_geometry::int_vector::IntVector;
    use epic_geometry::point::Point;

    /// Jar session `/tmp/epic-t25-transform.jsh` (repo root, JDK 25 jar),
    /// output `/tmp/epic-t25-transform.out`, transform (10, 3.5, -2.25).
    fn ct() -> CoordinateTransform {
        CoordinateTransform::new(10.0, 3.5, -2.25)
    }

    /// "CT boardToDsn(7) = 0.7", "CT boardToDsn(-0.05) = -0.005",
    /// "CT dsnToBoard(7) = 70.0".
    #[test]
    fn scalar_scaling_both_directions() {
        assert_eq!(ct().board_to_dsn_value(7.0), 0.7);
        assert_eq!(ct().board_to_dsn_value(-0.05), -0.005);
        assert_eq!(ct().dsn_to_board_value(7.0), 70.0);
    }

    /// "CT boardToDsn(FloatPoint(7,-3)) = [4.2, -2.55]" — base added after
    /// scaling; "CT boardToDsnRel(FloatPoint(7,-3)) = [0.7, -0.3]" — no base.
    #[test]
    fn point_transform_adds_base_after_scaling() {
        assert_eq!(
            ct().board_to_dsn_point(FloatPoint::new(7.0, -3.0)),
            [4.2, -2.55]
        );
        assert_eq!(
            ct().board_to_dsn_rel_point(FloatPoint::new(7.0, -3.0)),
            [0.7, -0.3]
        );
    }

    /// "CT boardToDsn(FloatPoint[2]) = [3.6, -2.05, 3.2, -1.85]" for points
    /// (1,2) and (-3,4): interleaved x/y flat layout.
    #[test]
    fn points_transform_is_interleaved() {
        let points = [FloatPoint::new(1.0, 2.0), FloatPoint::new(-3.0, 4.0)];
        assert_eq!(
            ct().board_to_dsn_points(&points),
            vec![3.6, -2.05, 3.2, -1.85]
        );
    }

    /// Jar session `/tmp/epic-ct-extra.jsh`, output `/tmp/epic-ct-extra.out`:
    /// "CT boardToDsn(Line[(10,-4)->(30,16)]) = [4.5, -2.65, 6.5,
    /// -0.6499999999999999]" — the fourth value proves the Java order
    /// `boardToDsn(b.y) + baseY` (divide, THEN add base): computing
    /// `(b.y + baseY*scale)/scale` would give a different double.
    #[test]
    fn line_transform_divides_then_adds_base() {
        let lines = [Line::new(
            Point::int(IntPoint::new(10, -4)),
            Point::int(IntPoint::new(30, 16)),
        )];
        assert_eq!(
            ct().board_to_dsn_lines(&lines),
            vec![4.5, -2.65, 6.5, -0.6499999999999999]
        );
    }

    /// "CT boardToDsn(IntVector(20,40)) = [2.0, 4.0]" — vectors get NO base
    /// offset.
    #[test]
    fn vector_transform_skips_base() {
        assert_eq!(
            ct().board_to_dsn_vector(&Vector::Int(IntVector::new(20, 40))),
            [2.0, 4.0]
        );
    }

    /// "CT boardToDsn(IntBox(-100,-50..100,50)) = [-6.5, -7.25, 13.5, 2.75]";
    /// "CT boardToDsnRel(...) = [-10.0, -5.0, 10.0, 5.0]".
    #[test]
    fn box_transform_orders_ll_ll_ur_ur() {
        let bounding_box = IntBox::new(IntPoint::new(-100, -50), IntPoint::new(100, 50));
        assert_eq!(
            ct().board_to_dsn_box(&bounding_box),
            [-6.5, -7.25, 13.5, 2.75]
        );
        assert_eq!(
            ct().board_to_dsn_rel_box(&bounding_box),
            [-10.0, -5.0, 10.0, 5.0]
        );
    }

    /// Base subtracted BEFORE scaling ((4.2-3.5)*10, (1.95+2.25)*10).
    /// CAUTION: `FloatPoint.toString()` rounds to 4 fraction digits
    /// (`FloatPoint.java:469-473`) — the first capture printed "(7 , 42)"
    /// while the EXACT doubles (jar session `/tmp/epic-ct-precise.jsh`,
    /// output `/tmp/epic-ct-precise.out`, printed via `Double.toString`) are
    /// "dsnToBoard([4.2,1.95]).x exact = 7.000000000000002", ".y = 42.0",
    /// "dsnToBoardRel([4.2,-1.5]).x = 42.0", ".y = -15.0". Never pin values
    /// off `FloatPoint.toString`.
    #[test]
    fn tuple_transform_subtracts_base_before_scaling() {
        let point = ct().dsn_to_board_tuple([4.2, 1.95]);
        assert_eq!((point.x, point.y), (7.000000000000002, 42.0));
        let rel = ct().dsn_to_board_rel_tuple([4.2, -1.5]);
        assert_eq!((rel.x, rel.y), (42.0, -15.0));
    }

    // ---- T25: scale-factor loop (Structure.java:1189-1203) -------------
    // Jar sessions `/tmp/epic-t25-transform.jsh` + `/tmp/epic-t25-bounds.jsh`
    // (minimal DSNs driven through DsnReader.readBoard; the transform's
    // private `scaleFactor` read back by reflection).

    /// P1: resolution 100, boundary corners +/-5000 -> maxCoor = 5e5,
    /// 5*5e5 < CRIT_INT -> loop NOT entered, sf stays 100 ("P1 sf=100.0";
    /// end-to-end: "P1 bounds=-501000,-501000 .. 501000,501000", i.e. board
    /// corners 5000*100 +/- the 1000 offset of Structure.java:1208).
    #[test]
    fn scale_factor_p1_no_loop() {
        assert_eq!(
            max_abs_scaled_coordinate(&[-5000.0, -5000.0, 5000.0, 5000.0], 100),
            500000.0
        );
        assert_eq!(calc_scale_factor(100, 500000.0), 100);
    }

    /// P2: resolution 10, corners +/-1e6 -> maxCoor = 1e7, 5e7 >= CRIT_INT ->
    /// ONE loop pass (10/10 = 1), then 5*1e6 < CRIT_INT stops ("P2 sf=1.0";
    /// end-to-end "P2 bounds=..1001000,1001000" = dsn*1 + 1000).
    #[test]
    fn scale_factor_p2_loop_once() {
        assert_eq!(
            max_abs_scaled_coordinate(&[-1000000.0, -1000000.0, 1000000.0, 1000000.0], 10),
            10000000.0
        );
        assert_eq!(calc_scale_factor(10, 10000000.0), 1);
    }

    /// P3: resolution 15, corners +/-1e7 -> maxCoor = 1.5e8: pass 1
    /// (7.5e8 >= CRIT_INT) 15/10 = 1, pass 2 (7.5e7 >= CRIT_INT) **1/10 = 0
    /// by integer division**, then 7.5e6 < CRIT_INT stops. Jar: "P3 sf=0.0"
    /// — the transform scale reaches ZERO (all dsnToBoard coordinates
    /// collapse to 0). Bug-compatible: the Rust port must produce 0 too.
    #[test]
    fn scale_factor_p3_integer_division_reaches_zero() {
        assert_eq!(
            max_abs_scaled_coordinate(&[-10000000.0, -10000000.0, 10000000.0, 10000000.0], 15),
            150000000.0
        );
        assert_eq!(calc_scale_factor(15, 150000000.0), 0);
    }

    /// P4: resolution 1, corners +/-4e6 -> maxCoor = 4e6, 2e7 < CRIT_INT ->
    /// no loop; `max(resolution, 1)` keeps sf at 1 ("P4 sf=1.0").
    #[test]
    fn scale_factor_p4_max_guard_and_no_loop() {
        assert_eq!(
            max_abs_scaled_coordinate(&[-4000000.0, -4000000.0, 4000000.0, 4000000.0], 1),
            4000000.0
        );
        assert_eq!(calc_scale_factor(1, 4000000.0), 1);
    }

    /// Resolution 0 (or negative) clamps to scale factor 1
    /// (`Math.max(scopeParameter.resolution, 1)`, `Structure.java:1189`).
    /// Source-derived guard, jar-corroborated by P4 (sf=1 with resolution 1).
    #[test]
    fn scale_factor_clamps_resolution_to_one() {
        assert_eq!(calc_scale_factor(0, 0.0), 1);
        assert_eq!(calc_scale_factor(-5, 0.0), 1);
    }
}
