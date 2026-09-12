//! Port of Java `app.freerouting.geometry.planar.IntDirection`.
//!
//! Java `IntDirection` is package-private with a package-private
//! constructor; it surfaces publicly through the nine static constants on
//! `Direction` and the factories' return values. The Rust mirror is a
//! plain public struct so the [`crate::direction::Direction`] enum can
//! hold it by value (and so the nine constants exist with their Java
//! names on both types).
//!
//! Bit-parity notes (decision D1):
//! - `turn45Degree` computes `n = factor % 8` with Java's remainder
//!   (sign of the dividend), so every negative factor lands in the
//!   `default` arm and yields the NULL direction — reproduced with `%`,
//!   NOT `rem_euclid` (jshell pins `E`-`G`).
//! - The turn-table arithmetic and `opposite` wrap like Java `int`.
//! - The angle-order `compareTo` worker compares the determinant as a
//!   `double` (`Signum.asInt`), not in 64-bit integer arithmetic.

use std::cmp::Ordering;

use crate::direction::Direction;
use crate::int_vector::IntVector;
use crate::side::Side;
use crate::vector::Vector;

/// Implements the abstract class Direction as an equivalence class of
/// IntVectors.
#[derive(Debug, Clone, Copy)]
pub struct IntDirection {
    /// The x coordinate of this direction.
    pub x: i32,
    /// The y coordinate of this direction.
    pub y: i32,
}

impl IntDirection {
    /// Java `Direction.NULL`.
    pub const NULL: IntDirection = IntDirection::new(0, 0);
    /// The direction to the east.
    pub const RIGHT: IntDirection = IntDirection::new(1, 0);
    /// The direction to the northeast.
    pub const RIGHT45: IntDirection = IntDirection::new(1, 1);
    /// The direction to the north.
    pub const UP: IntDirection = IntDirection::new(0, 1);
    /// The direction to the northwest.
    pub const UP45: IntDirection = IntDirection::new(-1, 1);
    /// The direction to the west.
    pub const LEFT: IntDirection = IntDirection::new(-1, 0);
    /// The direction to the southwest.
    pub const LEFT45: IntDirection = IntDirection::new(-1, -1);
    /// The direction to the south.
    pub const DOWN: IntDirection = IntDirection::new(0, -1);
    /// The direction to the southeast.
    pub const DOWN45: IntDirection = IntDirection::new(1, -1);

    /// Java package-private constructor `IntDirection(int, int)`.
    pub const fn new(x: i32, y: i32) -> IntDirection {
        IntDirection { x, y }
    }

    /// Java package-private constructor `IntDirection(IntVector)`.
    pub const fn from_int_vector(vector: &IntVector) -> IntDirection {
        IntDirection::new(vector.x, vector.y)
    }

    /// Java `dir == Direction.NULL` — **reference** equality against the
    /// shared constant, true exactly for the zero vector: the only
    /// producers of zero directions are the collinear branch of
    /// `Point.perpendicularDirection` (which hands back the shared
    /// `Direction.NULL` itself) and `turn45Degree` of an already-zero
    /// direction; a non-zero direction never turns to zero. Deliberately
    /// NOT [`PartialEq`], which mirrors Java's inherited `equals()`
    /// (collinearity plus positive projection) — under that contract two
    /// distinct zero directions are unequal.
    pub fn is_null(&self) -> bool {
        self.x == 0 && self.y == 0
    }

    /// Returns true, if the direction is horizontal or vertical.
    pub fn is_orthogonal(&self) -> bool {
        self.x == 0 || self.y == 0
    }

    /// Returns true, if the direction is diagonal (`Math.abs` wraps at
    /// `i32::MIN`, hence `wrapping_abs`).
    pub fn is_diagonal(&self) -> bool {
        self.x.wrapping_abs() == self.y.wrapping_abs()
    }

    /// Returns true, if the direction is orthogonal or diagonal.
    pub fn is_multiple_of_45_degree(&self) -> bool {
        self.is_orthogonal() || self.is_diagonal()
    }

    /// Returns any Vector pointing into this direction.
    pub fn get_vector(&self) -> Vector {
        Vector::Int(IntVector::new(self.x, self.y))
    }

    /// Turns the direction by factor times 45 degree. Java computes
    /// `n = factor % 8` with the sign of the dividend: negative factors
    /// (and everything reducing below 0 after the remainder) fall into the
    /// `default` arm and yield the NULL direction — an intentional quirk
    /// (plan trap T4). The table arithmetic wraps like Java `int`, and
    /// results are NOT normalized (turning `DOWN45` by 45 degrees gives
    /// the raw `(2, 0)`, jshell pin `D`).
    pub fn turn_45_degree(&self, factor: i32) -> Direction {
        let n = factor % 8;
        let (x, y) = (self.x, self.y);
        let direction = match n {
            0 => IntDirection::new(x, y), // 0 degrees
            1 => IntDirection::new(x.wrapping_sub(y), x.wrapping_add(y)), // 45 degrees
            2 => IntDirection::new(y.wrapping_neg(), x), // 90 degrees
            3 => IntDirection::new(x.wrapping_neg().wrapping_sub(y), x.wrapping_sub(y)), // 135 degrees
            4 => IntDirection::new(x.wrapping_neg(), y.wrapping_neg()), // 180 degrees
            5 => IntDirection::new(y.wrapping_sub(x), x.wrapping_neg().wrapping_sub(y)), // 225 degrees
            6 => IntDirection::new(y, x.wrapping_neg()), // 270 degrees
            7 => IntDirection::new(x.wrapping_add(y), y.wrapping_sub(x)), // 315 degrees
            _ => IntDirection::new(0, 0),                // Java default: the NULL direction
        };
        Direction::Int(direction)
    }

    /// Returns the opposite direction of this direction (wraps like Java's
    /// unary minus at `i32::MIN`).
    pub fn opposite(&self) -> IntDirection {
        IntDirection::new(self.x.wrapping_neg(), self.y.wrapping_neg())
    }

    /// Java package-private `determinant(IntDirection)`: the determinant
    /// of this direction and other in double arithmetic.
    pub fn determinant(&self, other: &IntDirection) -> f64 {
        self.x as f64 * other.y as f64 - self.y as f64 * other.x as f64
    }
}

/// Java `equals`: `IntDirection` overrides no `equals` of its own — it
/// inherits the final `Direction.equals` (collinearity plus POSITIVE
/// projection), so this impl mirrors that logic instead of a componentwise
/// comparison. Distinct NULL directions are therefore unequal, and (2, 0)
/// equals (1, 0).
impl PartialEq for IntDirection {
    fn eq(&self, other: &Self) -> bool {
        let this_vector = self.get_vector();
        let other_vector = other.get_vector();
        if this_vector.side_of(&other_vector) != Side::Collinear {
            return false;
        }
        // check, that dir and other_dir do not point into opposite directions
        this_vector.projection(&other_vector) == Side::Positive
    }
}

impl Eq for IntDirection {}

/// Java package-private `IntDirection.compareTo(IntDirection)`: the
/// angle-order worker. The full ascending order of the nine constants is
/// the declaration order RIGHT, RIGHT45, UP, UP45, LEFT, LEFT45, DOWN,
/// DOWN45 (jshell pin `K`); the NULL direction orders like the negative x
/// axis.
pub(crate) fn compare_int_int(a: &IntDirection, b: &IntDirection) -> Ordering {
    let (x, y) = (a.x, a.y);
    if y > 0 {
        if b.y < 0 {
            return Ordering::Less;
        }
        if b.y == 0 {
            return if b.x > 0 {
                Ordering::Greater
            } else {
                Ordering::Less
            };
        }
    } else if y < 0 {
        if b.y >= 0 {
            return Ordering::Greater;
        }
    } else {
        // y == 0
        if x > 0 {
            if b.y != 0 || b.x < 0 {
                return Ordering::Less;
            }
            return Ordering::Equal;
        }
        // x <= 0 (the Java comment says x < 0; x == 0 flows in here too)
        if b.y > 0 || (b.y == 0 && b.x > 0) {
            return Ordering::Greater;
        }
        if b.y < 0 {
            return Ordering::Less;
        }
        return Ordering::Equal;
    }

    // now this direction and other are located in the same
    // open horizontal half plane

    // Java: (double) other.x * y - (double) other.y * x — note the swapped
    // operand order relative to `IntDirection.determinant`.
    let determinant = b.x as f64 * y as f64 - b.y as f64 * x as f64;
    match Side::as_int(determinant) {
        1 => Ordering::Greater,
        -1 => Ordering::Less,
        _ => Ordering::Equal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// jshell pins `G0`-`G7`/`H`/`I`: the exhaustive 8-factor turn table
    /// for RIGHT, plus unnormalized results on diagonals — turning DOWN45
    /// by 45 degrees yields the raw `(2, 0)`, `UP45` by factor 5 the raw
    /// `(2, 0)` (arm 5), and `DOWN45` by factor 6 the raw `(-1, -1)`
    /// (arm 6).
    #[test]
    fn turn_45_degree_is_unnormalized() {
        let xy = |d: &Direction| match d {
            Direction::Int(dir) => (dir.x, dir.y),
            Direction::BigInt(_) => panic!("expected an int direction"),
        };
        let expected = [
            (1, 0),   // 0 degrees
            (1, 1),   // 45 degrees
            (0, 1),   // 90 degrees
            (-1, 1),  // 135 degrees
            (-1, 0),  // 180 degrees
            (-1, -1), // 225 degrees
            (0, -1),  // 270 degrees
            (1, -1),  // 315 degrees
        ];
        for (factor, want) in expected.iter().enumerate() {
            assert_eq!(
                xy(&IntDirection::RIGHT.turn_45_degree(factor as i32)),
                *want
            );
        }
        assert_eq!(xy(&IntDirection::new(1, -1).turn_45_degree(1)), (2, 0));
        assert_eq!(xy(&IntDirection::UP45.turn_45_degree(5)), (2, 0));
        assert_eq!(xy(&IntDirection::DOWN45.turn_45_degree(6)), (-1, -1));
        // 180 degrees from UP equals the opposite direction (pin `h`).
        assert_eq!(
            IntDirection::UP.turn_45_degree(4),
            Direction::Int(IntDirection::UP.opposite())
        );
    }

    /// jshell pins `E`/`F`/`G`/`H`: Java `%` keeps the dividend sign, so
    /// negative factors land in the default arm (NULL), while -8 and 16
    /// hit case 0 and return the direction unchanged.
    #[test]
    fn negative_turn_factors_yield_null() {
        // Distinct NULL directions are never `equals` (zero projection is
        // not POSITIVE), so the assertions compare coordinates.
        let xy = |d: &Direction| match d {
            Direction::Int(dir) => (dir.x, dir.y),
            Direction::BigInt(_) => panic!("expected an int direction"),
        };
        assert_eq!(xy(&IntDirection::RIGHT.turn_45_degree(-1)), (0, 0));
        assert_eq!(xy(&IntDirection::RIGHT.turn_45_degree(-7)), (0, 0));
        assert_eq!(xy(&IntDirection::RIGHT.turn_45_degree(-9)), (0, 0));
        assert_eq!(xy(&IntDirection::RIGHT.turn_45_degree(-8)), (1, 0));
        assert_eq!(xy(&IntDirection::RIGHT.turn_45_degree(16)), (1, 0));
    }

    /// The angle-order worker: half-plane branches and the double
    /// determinant with the swapped operand order (Java line 76).
    #[test]
    fn worker_orders_by_angle() {
        assert_eq!(
            compare_int_int(&IntDirection::UP45, &IntDirection::RIGHT),
            Ordering::Greater
        );
        assert_eq!(
            compare_int_int(&IntDirection::UP, &IntDirection::UP45),
            Ordering::Less
        );
        assert_eq!(
            compare_int_int(&IntDirection::UP, &IntDirection::LEFT45),
            Ordering::Less
        );
        // The NULL direction behaves like the negative x axis in the
        // worker (this is what makes toString(NULL) report "LEFT").
        assert_eq!(
            compare_int_int(&IntDirection::NULL, &IntDirection::RIGHT),
            Ordering::Greater
        );
        assert_eq!(
            compare_int_int(&IntDirection::NULL, &IntDirection::LEFT),
            Ordering::Equal
        );
    }

    /// Predicates wrap `Math.abs` at `i32::MIN`.
    #[test]
    fn predicates() {
        assert!(IntDirection::new(i32::MIN, i32::MIN).is_diagonal());
        assert!(IntDirection::new(0, 5).is_orthogonal());
        assert!(!IntDirection::new(3, 2).is_multiple_of_45_degree());
        assert!(IntDirection::new(3, 3).is_multiple_of_45_degree());
    }

    /// `opposite` wraps like Java's unary minus; `determinant` computes in
    /// double.
    #[test]
    fn opposite_and_determinant() {
        assert_eq!(
            IntDirection::new(i32::MIN, 5).opposite(),
            IntDirection::new(i32::MIN, -5)
        );
        assert_eq!(IntDirection::RIGHT.determinant(&IntDirection::UP), 1.0);
        assert_eq!(IntDirection::UP.determinant(&IntDirection::RIGHT), -1.0);
    }

    /// `getVector` builds a plain IntVector from the raw coordinates.
    #[test]
    fn get_vector_is_a_plain_int_vector() {
        let v = IntDirection::new(3, -4).get_vector();
        match v {
            Vector::Int(iv) => {
                assert_eq!(iv.x, 3);
                assert_eq!(iv.y, -4);
            }
            Vector::Rational(_) => panic!("expected an int vector"),
        }
    }
}
