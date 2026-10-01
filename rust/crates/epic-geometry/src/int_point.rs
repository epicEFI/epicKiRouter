//! Port of Java `app.freerouting.geometry.planar.IntPoint`.
//!
//! The Java double-dispatch overloads (`translateBy`, `differenceBy`,
//! `compareX`, `compareY`, `sideOf`, `perpendicularProjection`) live on the
//! [`crate::point::Point`] enum. Java declares
//! `public static final IntPoint ZERO = new IntPoint(0, 0)` on `Point`; it
//! is [`IntPoint::ZERO`] here.
//!
//! Constructor note (soft guard): the Java constructor logs
//! `"IntPoint: x is out of range"` via `FRLogger.debug` when
//! `|x| > Limits.CRIT_INT` and then **continues** — construction never
//! fails. Faithfully porting that log-and-continue means the Rust
//! constructor performs no check at all (a `debug_assert!` would panic in
//! debug builds on exactly the parity path that requires out-of-range
//! values: `Point::get_instance` constructs the IntPoint first and
//! wraps it in a RationalPoint afterwards). The Java bodies never read the
//! log, so dropping the message preserves observable behavior.
//!
//! Deferred: `sideOf(Line)` and `perpendicularProjection(Line)` (Task 6,
//! Line). `surroundingBox` / `isContainedIn` landed with Task 4 (IntBox);
//! `surroundingOctagon` with Task 5 (IntOctagon).

use std::fmt;
use std::hash::{Hash, Hasher};

use crate::float_point::FloatPoint;
use crate::int_box::IntBox;
use crate::int_octagon::IntOctagon;
use crate::int_vector::IntVector;

/// Implementation of the abstract class Point as a tuple of integers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntPoint {
    /// The x coordinate of this point.
    pub x: i32,
    /// The y coordinate of this point.
    pub y: i32,
}

impl IntPoint {
    /// Java `Point.ZERO`.
    pub const ZERO: IntPoint = IntPoint { x: 0, y: 0 };

    /// Creates an IntPoint from two integer coordinates. See the module
    /// docs for the out-of-range soft-guard parity note.
    pub const fn new(x: i32, y: i32) -> IntPoint {
        IntPoint { x, y }
    }

    /// Returns the smallest IntBox containing only this point (Java
    /// `surroundingBox`, landed with Task 4).
    pub fn surrounding_box(&self) -> IntBox {
        IntBox::new(*self, *self)
    }

    /// Returns the smallest IntOctagon containing only this point (Java
    /// `surroundingOctagon`, landed with Task 5): the four diagonal
    /// intercepts run through the point itself (int sums wrap like Java).
    pub fn surrounding_octagon(&self) -> IntOctagon {
        let tmp1 = self.x.wrapping_sub(self.y);
        let tmp2 = self.x.wrapping_add(self.y);
        IntOctagon::new(self.x, self.y, self.x, self.y, tmp1, tmp1, tmp2, tmp2)
    }

    /// Returns true if this point lies inside `int_box` or on its border
    /// (Java `isContainedIn(IntBox)`, landed with Task 4).
    pub fn is_contained_in(&self, int_box: &IntBox) -> bool {
        self.x >= int_box.ll.x
            && self.y >= int_box.ll.y
            && self.x <= int_box.ur.x
            && self.y <= int_box.ur.y
    }

    /// Returns true — IntPoints are never infinite.
    pub fn is_infinite(&self) -> bool {
        false
    }

    /// Converts this point to a FloatPoint.
    pub fn to_float(&self) -> FloatPoint {
        FloatPoint::new(self.x as f64, self.y as f64)
    }

    /// Returns a unique ID for deterministic tie-breaking. Java
    /// `getId() == hashCode() == 31 * x + y` (int wraparound).
    pub fn get_id(&self) -> i32 {
        31i32.wrapping_mul(self.x).wrapping_add(self.y)
    }

    /// Java package-private `differenceBy(IntPoint)`: the componentwise
    /// difference with int wraparound.
    pub(crate) fn difference_by(&self, other: &IntPoint) -> IntVector {
        IntVector::new(self.x.wrapping_sub(other.x), self.y.wrapping_sub(other.y))
    }

    /// Returns the determinant of the vectors (x, y) and (other.x,
    /// other.y), in 64-bit arithmetic (IntPoint.java:131-133).
    pub fn determinant(&self, other: &IntPoint) -> i64 {
        (self.x as i64) * (other.y as i64) - (self.y as i64) * (other.x as i64)
    }

    /// Returns the signed area of the parallelogram spanned by the vectors
    /// `2 - 1` and `this - 1` as a double (Java widens the i64 determinant
    /// of `IntVector.determinant`).
    pub fn signed_area(&self, p1: &IntPoint, p2: &IntPoint) -> f64 {
        let d21 = p2.difference_by(p1);
        let d01 = self.difference_by(p1);
        d21.determinant(&d01) as f64
    }

    /// Calculates the square of the distance between this point and
    /// to_point. The coordinate differences are computed in int arithmetic
    /// (with wraparound) before widening to double, exactly as in Java.
    pub fn distance_square(&self, to_point: &IntPoint) -> f64 {
        let dx = to_point.x.wrapping_sub(self.x) as f64;
        let dy = to_point.y.wrapping_sub(self.y) as f64;
        dx * dx + dy * dy
    }

    /// Calculates the distance between this point and to_point.
    pub fn distance(&self, to_point: &IntPoint) -> f64 {
        self.distance_square(to_point).sqrt()
    }

    /// Calculates the nearest point to this point on the horizontal or
    /// vertical line through other (snaps this point onto an orthogonal
    /// line through other).
    pub fn orthogonal_projection(&self, other: &IntPoint) -> IntPoint {
        let horizontal_distance = self.x.wrapping_sub(other.x).wrapping_abs();
        let vertical_distance = self.y.wrapping_sub(other.y).wrapping_abs();
        if horizontal_distance <= vertical_distance {
            // projection onto the vertical line through other
            IntPoint::new(other.x, self.y)
        } else {
            // projection onto the horizontal line through other
            IntPoint::new(self.x, other.y)
        }
    }

    /// Calculates the nearest point to this point on an orthogonal or
    /// diagonal line through other (snaps this point onto a 45-degree line
    /// through other). The minimum over the four candidate distances uses
    /// a strict `<` update, so the first index wins ties, and the diagonal
    /// values are computed and truncated exactly as in Java
    /// (IntPoint.java:219-252).
    pub fn fortyfive_degree_projection(&self, other: &IntPoint) -> IntPoint {
        let dx = self.x.wrapping_sub(other.x);
        let dy = self.y.wrapping_sub(other.y);
        // distArr[0]/[1]: |dx|, |dy| computed in int (wrapping at MIN),
        // then widened to double.
        let dist0 = dx.wrapping_abs() as f64;
        let dist1 = dy.wrapping_abs() as f64;
        let diagonal1 = (dy as f64 - dx as f64) / 2.0;
        let diagonal2 = (dy as f64 + dx as f64) / 2.0;
        let dist2 = diagonal1.abs();
        let dist3 = diagonal2.abs();

        let mut min_dist = dist0;
        if dist1 < min_dist {
            min_dist = dist1;
        }
        if dist2 < min_dist {
            min_dist = dist2;
        }
        if dist3 < min_dist {
            min_dist = dist3;
        }

        if min_dist == dist0 {
            // projection onto the vertical line through other
            IntPoint::new(other.x, self.y)
        } else if min_dist == dist1 {
            // projection onto the horizontal line through other
            IntPoint::new(self.x, other.y)
        } else if min_dist == dist2 {
            // projection onto the right diagonal line through other
            // (Java `(int)` cast of a double saturates; Rust `as` matches)
            let diagonal_value = diagonal2 as i32;
            IntPoint::new(
                other.x.wrapping_add(diagonal_value),
                other.y.wrapping_add(diagonal_value),
            )
        } else {
            // projection onto the left diagonal line through other
            let diagonal_value = diagonal1 as i32;
            IntPoint::new(
                other.x.wrapping_sub(diagonal_value),
                other.y.wrapping_add(diagonal_value),
            )
        }
    }

    /// Calculates a corner point p so that the lines through this point
    /// and p and from p to to_point are multiples of 45 degrees, and that
    /// the angle at p will be 45 degrees. If left_turn, to_point will be
    /// on the left of the line from this point to p, else on the right.
    /// Returns None if the line from this point to to_point is already a
    /// multiple of 45 degrees. All arithmetic wraps like Java's int ops.
    pub fn fortyfive_degree_corner(
        &self,
        to_point: &IntPoint,
        left_turn: bool,
    ) -> Option<IntPoint> {
        let dx = to_point.x.wrapping_sub(self.x);
        let dy = to_point.y.wrapping_sub(self.y);

        // handle the 8 sections between the 45 degree lines
        if dy > 0 && dy < dx {
            Some(if left_turn {
                IntPoint::new(to_point.x.wrapping_sub(dy), self.y)
            } else {
                IntPoint::new(self.x.wrapping_add(dy), to_point.y)
            })
        } else if dx > 0 && dy > dx {
            Some(if left_turn {
                IntPoint::new(to_point.x, self.y.wrapping_add(dx))
            } else {
                IntPoint::new(self.x, to_point.y.wrapping_sub(dx))
            })
        } else if dx < 0 && dy > dx.wrapping_neg() {
            Some(if left_turn {
                IntPoint::new(self.x, to_point.y.wrapping_add(dx))
            } else {
                IntPoint::new(to_point.x, self.y.wrapping_sub(dx))
            })
        } else if dy > 0 && dy < dx.wrapping_neg() {
            Some(if left_turn {
                IntPoint::new(self.x.wrapping_sub(dy), to_point.y)
            } else {
                IntPoint::new(to_point.x.wrapping_add(dy), self.y)
            })
        } else if dy < 0 && dy > dx {
            Some(if left_turn {
                IntPoint::new(to_point.x.wrapping_sub(dy), self.y)
            } else {
                IntPoint::new(self.x.wrapping_add(dy), to_point.y)
            })
        } else if dx < 0 && dy < dx {
            Some(if left_turn {
                IntPoint::new(to_point.x, self.y.wrapping_add(dx))
            } else {
                IntPoint::new(self.x, to_point.y.wrapping_sub(dx))
            })
        } else if dx > 0 && dy < dx.wrapping_neg() {
            Some(if left_turn {
                IntPoint::new(self.x, to_point.y.wrapping_add(dx))
            } else {
                IntPoint::new(to_point.x, self.y.wrapping_sub(dx))
            })
        } else if dy < 0 && dy > dx.wrapping_neg() {
            Some(if left_turn {
                IntPoint::new(self.x.wrapping_sub(dy), to_point.y)
            } else {
                IntPoint::new(to_point.x.wrapping_add(dy), self.y)
            })
        } else {
            // the line from this point to to_point is already a multiple
            // of 45 degree
            None
        }
    }

    /// Calculates a corner point p so that the lines through this point
    /// and p and from p to to_point are horizontal or vertical, and that
    /// the angle at p will be 90 degrees. Returns None if the line from
    /// this point to to_point is already orthogonal.
    pub fn ninety_degree_corner(&self, to_point: &IntPoint, left_turn: bool) -> Option<IntPoint> {
        let dx = to_point.x.wrapping_sub(self.x);
        let dy = to_point.y.wrapping_sub(self.y);

        // handle the 4 quadrants
        if (dx > 0 && dy > 0) || (dx < 0 && dy < 0) {
            Some(if left_turn {
                IntPoint::new(to_point.x, self.y)
            } else {
                IntPoint::new(self.x, to_point.y)
            })
        } else if (dx < 0 && dy > 0) || (dx > 0 && dy < 0) {
            Some(if left_turn {
                IntPoint::new(self.x, to_point.y)
            } else {
                IntPoint::new(to_point.x, self.y)
            })
        } else {
            // the line from this point to to_point is already orthogonal
            None
        }
    }
}

/// Java `hashCode() == getId() == 31 * x + y` with int wraparound.
impl Hash for IntPoint {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_i32(self.get_id());
    }
}

/// Java `toString()`: `"(" + x + "," + y + ")"`.
impl fmt::Display for IntPoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({},{})", self.x, self.y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Port of `PointEqualsHashCodeTest.intPointEqualsAndHashCodeContract`,
    /// with the hashCode pins from jshell (`P4`–`P6`).
    #[test]
    fn equals_and_hash_code_contract() {
        let p1 = IntPoint::new(100, 200);
        let p2 = IntPoint::new(100, 200);
        assert_eq!(p1, p2);
        assert_eq!(p1.get_id(), p2.get_id());

        assert_ne!(p1, IntPoint::new(100, 201));
        assert_ne!(p1, IntPoint::new(101, 200));

        // Java hashCode == getId == 31 * x + y (int wraparound).
        assert_eq!(p1.get_id(), 3300);
        assert_eq!(IntPoint::new(-5, 7).get_id(), -148);
    }

    /// jshell pins `P1`/`P2`: the determinant is computed in i64, so the
    /// product 2^25 * 2^25 = 2^50 stays exact where an i32 port would wrap
    /// (2^25 * 64 already exceeds i32::MAX).
    #[test]
    fn determinant_is_64_bit_at_the_crit_int_boundary() {
        let a = IntPoint::new(33_554_432, 0);
        let b = IntPoint::new(0, 33_554_432);
        assert_eq!(a.determinant(&b), 1_125_899_906_842_624);
        let c = IntPoint::new(0, 64);
        assert_eq!(a.determinant(&c), 2_147_483_648);
    }

    /// jshell pins `P70`/`P74`: `distanceSquare((3,4)) == 25.0`.
    #[test]
    fn distance_square_and_distance() {
        let zero = IntPoint::ZERO;
        assert_eq!(zero.distance_square(&IntPoint::new(3, 4)), 25.0);
        assert_eq!(IntPoint::new(3, 4).distance(&zero), 5.0);
    }

    /// jshell pin `P59`: signedArea == 1.0.
    #[test]
    fn signed_area_matches_java() {
        let p = IntPoint::new(0, 1);
        assert_eq!(p.signed_area(&IntPoint::ZERO, &IntPoint::new(1, 0)), 1.0);
    }

    /// jshell pin `P21`: (3,1) projected onto (0,0) is (3,0) — the
    /// horizontal distance wins, so the projection is onto the horizontal
    /// line through other.
    #[test]
    fn orthogonal_projection_snaps_to_the_nearer_axis() {
        assert_eq!(
            IntPoint::new(3, 1).orthogonal_projection(&IntPoint::ZERO),
            IntPoint::new(3, 0)
        );
        // Ties go to the vertical projection (<=).
        assert_eq!(
            IntPoint::new(2, 2).orthogonal_projection(&IntPoint::ZERO),
            IntPoint::new(0, 2)
        );
    }

    /// jshell pins `P22`/`P23`: (3,1) snaps to (3,0); (2,1) snaps onto the
    /// right diagonal (1,1).
    #[test]
    fn fortyfive_degree_projection_pins() {
        assert_eq!(
            IntPoint::new(3, 1).fortyfive_degree_projection(&IntPoint::ZERO),
            IntPoint::new(3, 0)
        );
        assert_eq!(
            IntPoint::new(2, 1).fortyfive_degree_projection(&IntPoint::ZERO),
            IntPoint::new(1, 1)
        );
    }

    /// jshell pins `P24`/`P25`/`P27`.
    #[test]
    fn fortyfive_degree_corner_pins() {
        assert_eq!(
            IntPoint::new(5, 5).fortyfive_degree_corner(&IntPoint::new(9, 8), true),
            Some(IntPoint::new(6, 5))
        );
        assert_eq!(
            IntPoint::new(5, 5).fortyfive_degree_corner(&IntPoint::new(9, 8), false),
            Some(IntPoint::new(8, 8))
        );
        // Already a 45-degree line: Java returns null.
        assert_eq!(
            IntPoint::ZERO.fortyfive_degree_corner(&IntPoint::new(1, 1), true),
            None
        );
    }

    /// jshell pin `P26` and the already-orthogonal case.
    #[test]
    fn ninety_degree_corner_pins() {
        assert_eq!(
            IntPoint::new(5, 5).ninety_degree_corner(&IntPoint::new(9, 8), true),
            Some(IntPoint::new(9, 5))
        );
        assert_eq!(
            IntPoint::new(5, 5).ninety_degree_corner(&IntPoint::new(9, 8), false),
            Some(IntPoint::new(5, 8))
        );
        assert_eq!(
            IntPoint::ZERO.ninety_degree_corner(&IntPoint::new(0, 5), true),
            None
        );
    }

    /// jshell pins `C1`-`C8` (both turns each): the 8 direction sections
    /// of `fortyfiveDegreeCorner`, taken from ZERO so dx/dy are the
    /// toPoint coordinates themselves.
    #[test]
    fn fortyfive_degree_corner_all_eight_sections() {
        let o = IntPoint::ZERO;
        let cases = [
            ((8, 3), ((5, 0), (3, 3))),      // 0-45 degrees
            ((3, 8), ((3, 3), (0, 5))),      // 45-90 degrees
            ((-3, 8), ((0, 5), (-3, 3))),    // 90-135 degrees
            ((-8, 3), ((-3, 3), (-5, 0))),   // 135-180 degrees
            ((-8, -3), ((-5, 0), (-3, -3))), // 180-225 degrees
            ((-3, -8), ((-3, -3), (0, -5))), // 225-270 degrees
            ((3, -8), ((0, -5), (3, -3))),   // 270-315 degrees
            ((8, -3), ((3, -3), (5, 0))),    // 315-360 degrees
        ];
        for ((tx, ty), (left, right)) in cases {
            assert_eq!(
                o.fortyfive_degree_corner(&IntPoint::new(tx, ty), true),
                Some(IntPoint::new(left.0, left.1))
            );
            assert_eq!(
                o.fortyfive_degree_corner(&IntPoint::new(tx, ty), false),
                Some(IntPoint::new(right.0, right.1))
            );
        }
    }

    /// jshell pins `F*`: `fortyfiveDegreeProjection` branch battery. The
    /// dist3 branch consumes `diagonal1` — including negative diagonals,
    /// where `(int)` truncates TOWARD zero ((int) -2.5 = -2; a floor-based
    /// port would give -3). The dist2 branch consumes `diagonal2` and its
    /// result lands on the other diagonal (the Java comments name the
    /// diagonals, not the landing lines; the formulas are the parity).
    #[test]
    fn fortyfive_degree_projection_branch_battery() {
        let o = IntPoint::ZERO;
        let cases = [
            ((3, -2), (2, -2)),   // dist3: (int) -2.5 = -2
            ((-2, 1), (-1, 1)),   // dist3: (int) 1.5 = 1
            ((-4, 2), (-3, 3)),   // dist3: diagonal1 = 3 exact
            ((4, -2), (3, -3)),   // dist3: diagonal1 = -3 exact
            ((2, -3), (2, -2)),   // dist3: (int) -2.5 = -2
            ((5, -2), (3, -3)),   // dist3: (int) -3.5 = -3
            ((5, 3), (4, 4)),     // dist2: diagonal2 = 4 exact
            ((3, 5), (4, 4)),     // dist2: diagonal2 = 4 exact
            ((-3, -2), (-2, -2)), // dist2: (int) -2.5 = -2
        ];
        for ((x, y), (ex, ey)) in cases {
            assert_eq!(
                IntPoint::new(x, y).fortyfive_degree_projection(&o),
                IntPoint::new(ex, ey)
            );
        }
    }

    /// jshell pin `N2`: `ninetyDegreeCorner` second quadrant
    /// (dx < 0, dy > 0), both turns.
    #[test]
    fn ninety_degree_corner_second_quadrant() {
        let o = IntPoint::ZERO;
        assert_eq!(
            o.ninety_degree_corner(&IntPoint::new(-3, 5), true),
            Some(IntPoint::new(0, 5))
        );
        assert_eq!(
            o.ninety_degree_corner(&IntPoint::new(-3, 5), false),
            Some(IntPoint::new(-3, 0))
        );
    }

    /// jshell pins `D1`/`D2`: distanceSquare widens AFTER the int
    /// subtraction wraps (`double dx = toPoint.x - this.x` is int
    /// arithmetic in Java): 0 - MIN wraps to MIN, MIN - 1 wraps to MAX,
    /// and the squares are those of the wrapped values.
    #[test]
    fn distance_square_wraps_before_widening() {
        let d1 = IntPoint::ZERO.distance_square(&IntPoint::new(i32::MIN, i32::MIN));
        // 0x1.0p63 = 9.223372036854776e18; the unwrapped 2^64 would be
        // 1.8446744073709552e19.
        assert_eq!(d1, 9.223_372_036_854_776e18);
        let d2 = IntPoint::new(1, 0).distance_square(&IntPoint::new(i32::MIN, 0));
        // 0x1.fffffff8p61 = (double) 2147483647^2 rounded.
        assert_eq!(d2, 4.611_686_014_132_420_6e18);
    }

    /// jshell pin `P75`.
    #[test]
    fn to_float_widens_and_display_matches_tostring() {
        let f = IntPoint::new(2, 3).to_float();
        assert_eq!((f.x, f.y), (2.0, 3.0));
        assert_eq!(IntPoint::new(3, 0).to_string(), "(3,0)");
    }

    #[test]
    fn is_infinite_is_always_false() {
        assert!(!IntPoint::new(-7, 9).is_infinite());
    }

    /// jshell pin: IntPoint(3,-4).surroundingOctagon() has the diagonals
    /// through the point (tmp1 = 3 - (-4) = 7, tmp2 = 3 + (-4) = -1).
    #[test]
    fn surrounding_octagon_pin() {
        let o = IntPoint::new(3, -4).surrounding_octagon();
        assert_eq!((o.left_x, o.bottom_y, o.right_x, o.top_y), (3, -4, 3, -4));
        assert_eq!(o.upper_left_diagonal_x, 7);
        assert_eq!(o.lower_right_diagonal_x, 7);
        assert_eq!(o.lower_left_diagonal_x, -1);
        assert_eq!(o.upper_right_diagonal_x, -1);
    }
}
