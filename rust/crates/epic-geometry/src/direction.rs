//! Port of Java `app.freerouting.geometry.planar.Direction`.
//!
//! Java models directions as an abstract class with the package-private
//! implementations `IntDirection` and `BigIntDirection` and resolves the
//! `compareTo` double dispatch through package-private overloads. This
//! port seals the hierarchy as a 2-variant enum (decision D1) and
//! reproduces the dispatch exactly:
//!
//! - the public `compareTo(Direction)` is the NEGATED package-private
//!   dispatch (`-otherDirection.compareTo(this)`),
//! - `equals` is the final `Direction.equals`: a `getClass()` check (an
//!   `IntDirection` is never equal to a `BigIntDirection`), collinearity,
//!   and a POSITIVE projection — so opposite directions and distinct NULL
//!   directions are unequal,
//! - `toString` walks the UN-negated worker chain in declaration order,
//!   which makes the NULL direction report "LEFT" (its worker identifies
//!   it with the negative x axis before the NULL entry is reached) —
//!   jshell-pinned quirk, not a port bug (pin `A`).
//!
//! The nine Java static constants exist both here (as `Direction` values)
//! and on [`crate::int_direction::IntDirection`] (their Java static
//! type), with the Java names.
//!
//! Deferred: none. (Task 4-6 types are not needed by any Direction
//! method: `middleApprox`/`getInstanceApprox` build plain `IntVector`s,
//! exactly as in Java.)

use std::cmp::Ordering;
use std::fmt;

use crate::big_int_direction::{BigIntDirection, compare_big_big};
use crate::int_direction::{IntDirection, compare_int_int};
use crate::int_vector::IntVector;
use crate::point::Point;
use crate::rounding::java_round;
use crate::side::Side;
use crate::vector::Vector;

/// Abstract class defining functionality of directions in the plane. A
/// Direction is an equivalence class of vectors.
#[derive(Debug, Clone)]
pub enum Direction {
    /// Integer coordinates (Java `IntDirection`).
    Int(IntDirection),
    /// Infinite-precision coordinates (Java `BigIntDirection`).
    BigInt(Box<BigIntDirection>),
}

impl Direction {
    /// Java `Direction.NULL`.
    pub const NULL: Direction = Direction::Int(IntDirection::NULL);
    /// The direction to the east.
    pub const RIGHT: Direction = Direction::Int(IntDirection::RIGHT);
    /// The direction to the northeast.
    pub const RIGHT45: Direction = Direction::Int(IntDirection::RIGHT45);
    /// The direction to the north.
    pub const UP: Direction = Direction::Int(IntDirection::UP);
    /// The direction to the northwest.
    pub const UP45: Direction = Direction::Int(IntDirection::UP45);
    /// The direction to the west.
    pub const LEFT: Direction = Direction::Int(IntDirection::LEFT);
    /// The direction to the southwest.
    pub const LEFT45: Direction = Direction::Int(IntDirection::LEFT45);
    /// The direction to the south.
    pub const DOWN: Direction = Direction::Int(IntDirection::DOWN);
    /// The direction to the southeast.
    pub const DOWN45: Direction = Direction::Int(IntDirection::DOWN45);

    /// Creates a Direction from the input Vector (Java
    /// `getInstance(Vector)`).
    pub fn get_instance(vector: &Vector) -> Direction {
        vector.to_normalized_direction()
    }

    /// Calculates the direction from `from` to `to`. If `from` and `to`
    /// are equal, Java returns null — this port returns
    /// [`Option::None`] (Java `getInstance(Point, Point)`; renamed
    /// because Rust has no overloading).
    pub fn get_instance_from_points(from: &Point, to: &Point) -> Option<Direction> {
        if from == to {
            return None;
        }
        Some(Self::get_instance(&to.difference_by(from)))
    }

    /// Creates a Direction whose angle with the x-axis is nearly equal to
    /// angle. Java rounds the scaled cosine/sine into a plain IntVector
    /// and normalizes it through `getInstance` (gcd reduction; a
    /// RationalVector promotion cannot occur because the coordinates are
    /// bounded by the scale factor).
    pub fn get_instance_approx(angle: f64) -> Direction {
        const SCALE_FACTOR: f64 = 10000.0;
        let x = java_round(angle.cos() * SCALE_FACTOR) as i32;
        let y = java_round(angle.sin() * SCALE_FACTOR) as i32;
        Self::get_instance(&Vector::Int(IntVector::new(x, y)))
    }

    /// Returns any Vector pointing into this direction.
    pub fn get_vector(&self) -> Vector {
        match self {
            Direction::Int(d) => d.get_vector(),
            Direction::BigInt(d) => d.get_vector(),
        }
    }

    /// Returns true, if the direction is horizontal or vertical.
    pub fn is_orthogonal(&self) -> bool {
        match self {
            Direction::Int(d) => d.is_orthogonal(),
            Direction::BigInt(d) => d.is_orthogonal(),
        }
    }

    /// Returns true, if the direction is diagonal.
    pub fn is_diagonal(&self) -> bool {
        match self {
            Direction::Int(d) => d.is_diagonal(),
            Direction::BigInt(d) => d.is_diagonal(),
        }
    }

    /// Returns true, if the direction is orthogonal or diagonal.
    pub fn is_multiple_of_45_degree(&self) -> bool {
        self.is_orthogonal() || self.is_diagonal()
    }

    /// Turns the direction by factor times 45 degree. A BigIntDirection is
    /// returned unchanged (Java warns "not yet implemented" and returns
    /// this).
    pub fn turn_45_degree(&self, factor: i32) -> Direction {
        match self {
            Direction::Int(d) => d.turn_45_degree(factor),
            Direction::BigInt(_) => self.clone(),
        }
    }

    /// Returns the opposite direction of this direction.
    pub fn opposite(&self) -> Direction {
        match self {
            Direction::Int(d) => Direction::Int(d.opposite()),
            Direction::BigInt(d) => Direction::BigInt(Box::new(d.opposite())),
        }
    }

    /// Let L be the line from the Zero Vector to other.getVector(). The
    /// function returns Side::Positive, if this.getVector() is on the left
    /// of L, Side::Negative, if this.getVector() is on the right of L and
    /// Side::Collinear, if this.getVector() is collinear with L (Java
    /// `sideOf`).
    pub fn side_of(&self, other: &Direction) -> Side {
        self.get_vector().side_of(&other.get_vector())
    }

    /// The function returns Side::Positive, if the scalar product of a
    /// vector representing this direction and a vector representing other
    /// is > 0, Side::Negative if it is < 0, and Side::Collinear if it is
    /// equal 0 (Java `Signum projection`, ported as [`Side`]).
    pub fn projection(&self, other: &Direction) -> Side {
        self.get_vector().projection(&other.get_vector())
    }

    /// Calculates an approximation of the direction in the middle of this
    /// direction and other. Java normalizes the rounded mean vector
    /// through `getInstance`.
    pub fn middle_approx(&self, other: &Direction) -> Direction {
        let v1 = self.get_vector().to_float();
        let v2 = other.get_vector().to_float();
        let length1 = v1.size();
        let length2 = v2.size();
        let x = v1.x / length1 + v2.x / length2;
        let y = v1.y / length1 + v2.y / length2;
        const SCALE_FACTOR: f64 = 1000.0;
        let vm = IntVector::new(
            java_round(x * SCALE_FACTOR) as i32,
            java_round(y * SCALE_FACTOR) as i32,
        );
        Self::get_instance(&Vector::Int(vm))
    }

    /// Returns 1, if the angle between 1 and this direction is bigger the
    /// angle between 2 and this direction, 0, if 1 is equal to 2, and -1
    /// otherwise (Java `compareFrom`).
    pub fn compare_from(&self, p1: &Direction, p2: &Direction) -> i32 {
        let result = if p1.compare_to(self) != Ordering::Less {
            if p2.compare_to(self) != Ordering::Less {
                p1.compare_to(p2)
            } else {
                Ordering::Less
            }
        } else if p2.compare_to(self) != Ordering::Less {
            Ordering::Greater
        } else {
            p1.compare_to(p2)
        };
        match result {
            Ordering::Greater => 1,
            Ordering::Equal => 0,
            Ordering::Less => -1,
        }
    }

    /// Returns an approximation of the signed angle corresponding to this
    /// direction.
    pub fn angle_approx(&self) -> f64 {
        self.get_vector().angle_approx_x_axis()
    }

    /// Java public `compareTo(Direction)`: the negated package-private
    /// double dispatch (`-otherDirection.compareTo(this)`). Returns
    /// whether this direction has a strictly bigger angle with the
    /// positive x-axis than other.
    pub fn compare_to(&self, other: &Direction) -> Ordering {
        other.worker_compare_to(self).reverse()
    }

    /// The package-private `compareTo(IntDirection)` /
    /// `compareTo(BigIntDirection)` dispatch of `this`: the un-negated
    /// worker with `other` in argument position.
    fn worker_compare_to(&self, other: &Direction) -> Ordering {
        match (self, other) {
            (Direction::Int(a), Direction::Int(b)) => compare_int_int(a, b),
            // Java: IntDirection.compareTo(BigIntDirection) =
            // -other.compareTo(this): widen the int side, negate.
            (Direction::Int(a), Direction::BigInt(b)) => {
                compare_big_big(&BigIntDirection::from_int_direction(a), b).reverse()
            }
            // Java: BigIntDirection.compareTo(IntDirection): widen the int
            // side, no negation.
            (Direction::BigInt(a), Direction::Int(b)) => {
                compare_big_big(a, &BigIntDirection::from_int_direction(b))
            }
            (Direction::BigInt(a), Direction::BigInt(b)) => compare_big_big(a, b),
        }
    }
}

/// Java `Direction.equals` (final): a `getClass()` check first — an
/// IntDirection is never equal to a BigIntDirection — then collinearity of
/// the two vectors plus a POSITIVE projection ("check, that dir and
/// other_dir do not point into opposite directions"). Java's `other ==
/// this` reference shortcut has no Rust counterpart; it is unobservable
/// except for the NULL direction, whose DISTINCT instances are unequal in
/// Java as well (the zero vector projects to Signum.ZERO).
impl PartialEq for Direction {
    fn eq(&self, other: &Direction) -> bool {
        if std::mem::discriminant(self) != std::mem::discriminant(other) {
            return false;
        }
        if self.side_of(other) != Side::Collinear {
            return false;
        }
        // check, that dir and other_dir do not point into opposite directions
        self.projection(other) == Side::Positive
    }
}

impl Eq for Direction {}

/// Java `Comparable<Direction>`: the angle order. Deliberately NOT
/// consistent with [`PartialEq`] — Java has the same inconsistency
/// (compareTo identifies the NULL direction with the negative x axis,
/// equals never identifies them); reproduced, not repaired.
impl PartialOrd for Direction {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(std::cmp::Ord::cmp(self, other))
    }
}

impl Ord for Direction {
    fn cmp(&self, other: &Self) -> Ordering {
        self.compare_to(other)
    }
}

/// Java `toString`: the chain runs through the UN-negated package-private
/// worker (virtual dispatch resolves `this.compareTo(RIGHT)` to the
/// worker, not the public negating wrapper), in declaration order, first
/// match wins. Hence NULL reports "LEFT" and non-45-degree directions
/// report "UNKNOWN" (jshell pins `A`, `g`, `Q`).
impl fmt::Display for Direction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = if self.worker_compare_to(&Self::RIGHT) == Ordering::Equal {
            "RIGHT"
        } else if self.worker_compare_to(&Self::RIGHT45) == Ordering::Equal {
            "UP-RIGHT"
        } else if self.worker_compare_to(&Self::UP) == Ordering::Equal {
            "UP"
        } else if self.worker_compare_to(&Self::UP45) == Ordering::Equal {
            "UP-LEFT"
        } else if self.worker_compare_to(&Self::LEFT) == Ordering::Equal {
            "LEFT"
        } else if self.worker_compare_to(&Self::LEFT45) == Ordering::Equal {
            "DOWN-LEFT"
        } else if self.worker_compare_to(&Self::DOWN) == Ordering::Equal {
            "DOWN"
        } else if self.worker_compare_to(&Self::DOWN45) == Ordering::Equal {
            "DOWN-RIGHT"
        } else if self.worker_compare_to(&Self::NULL) == Ordering::Equal {
            "NULL"
        } else {
            "UNKNOWN"
        };
        f.write_str(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rational_vector::RationalVector;
    use num_bigint::BigInt;

    fn b(v: i64) -> BigInt {
        BigInt::from(v)
    }

    fn rat(x: i64, y: i64, z: i64) -> Vector {
        Vector::Rational(Box::new(RationalVector::new(b(x), b(y), b(z))))
    }

    fn int_pair(d: &Direction) -> (i32, i32) {
        match d.get_vector() {
            Vector::Int(v) => (v.x, v.y),
            Vector::Rational(_) => panic!("expected an int direction"),
        }
    }

    /// jshell pins `B`-`I` (delegated to the IntDirection worker tests)
    /// plus the enum-level dispatch including the BigInt no-op.
    #[test]
    fn turn_45_degree_dispatches() {
        assert_eq!(
            Direction::DOWN45.turn_45_degree(1),
            Direction::Int(IntDirection::new(2, 0))
        );
        assert_eq!(int_pair(&Direction::RIGHT.turn_45_degree(-1)), (0, 0));
        // BigIntDirection: warn-and-return-this.
        let big = Direction::BigInt(Box::new(BigIntDirection::new(b(3), b(2))));
        assert_eq!(big.turn_45_degree(1), big.clone());
        assert_eq!(big.turn_45_degree(-1), big.clone());
    }

    /// jshell pin `K`: sorting the 8 compass directions by compareTo keeps
    /// the declaration (counterclockwise) order; pins `L`/`M`/`f`.
    #[test]
    fn compare_to_orders_the_compass_like_the_declaration() {
        let compass = [
            Direction::RIGHT,
            Direction::RIGHT45,
            Direction::UP,
            Direction::UP45,
            Direction::LEFT,
            Direction::LEFT45,
            Direction::DOWN,
            Direction::DOWN45,
        ];
        for pair in compass.windows(2) {
            assert_eq!(pair[0].compare_to(&pair[1]), Ordering::Less);
        }
        let mut sorted = compass.clone();
        sorted.sort();
        assert_eq!(sorted, compass);
        assert_eq!(
            Direction::RIGHT.compare_to(&Direction::UP45),
            Ordering::Less
        );
        assert_eq!(
            Direction::DOWN.compare_to(&Direction::DOWN45),
            Ordering::Less
        );
        // compareTo treats the NULL direction like the negative x axis.
        let null_dir = Direction::get_instance(&Vector::ZERO);
        assert_eq!(Direction::RIGHT.compare_to(&null_dir), Ordering::Less);
        // Ord agrees with compare_to.
        assert!(Direction::LEFT > Direction::UP);
        assert!(Direction::DOWN45 > Direction::DOWN);
    }

    /// jshell pins `A`/`g`/`Q`: toString walks the worker chain in
    /// declaration order; NULL therefore reports "LEFT".
    #[test]
    fn to_string_uses_the_worker_chain() {
        assert_eq!(Direction::NULL.to_string(), "LEFT");
        assert_eq!(Direction::RIGHT.to_string(), "RIGHT");
        assert_eq!(Direction::RIGHT45.to_string(), "UP-RIGHT");
        assert_eq!(Direction::UP.to_string(), "UP");
        assert_eq!(Direction::UP45.to_string(), "UP-LEFT");
        assert_eq!(Direction::LEFT.to_string(), "LEFT");
        assert_eq!(Direction::LEFT45.to_string(), "DOWN-LEFT");
        assert_eq!(Direction::DOWN.to_string(), "DOWN");
        assert_eq!(Direction::DOWN45.to_string(), "DOWN-RIGHT");
        let unknown = Direction::get_instance(&Vector::Int(IntVector::new(4, 2)));
        assert_eq!(unknown.to_string(), "UNKNOWN");
    }

    /// jshell pins `c`/`d`/`W` (plan trap T9): equals is collinearity plus
    /// positive projection; opposite directions differ; an IntDirection is
    /// never equal to a BigIntDirection (getClass check); distinct NULL
    /// directions are unequal (zero projection).
    #[test]
    fn equals_semantics() {
        let two_zero = Direction::get_instance(&Vector::Int(IntVector::new(2, 0)));
        assert_eq!(Direction::RIGHT, two_zero);
        assert_eq!(two_zero, Direction::RIGHT);
        assert_ne!(Direction::RIGHT, Direction::LEFT);
        assert_ne!(Direction::RIGHT, Direction::UP);
        let big_right = Direction::BigInt(Box::new(BigIntDirection::from_int_direction(
            &IntDirection::RIGHT,
        )));
        assert_ne!(Direction::RIGHT, big_right);
        assert_ne!(big_right, Direction::RIGHT);
        // W: the zero-vector direction is not equal to RIGHT.
        let null1 = Direction::get_instance(&Vector::ZERO);
        assert_ne!(null1, Direction::RIGHT);
        let null2 = Direction::get_instance(&Vector::Int(IntVector::new(0, 0)));
        assert_ne!(null1, null2);
    }

    /// jshell pins `T`/`U`/`V`/`R`/`S`: getInstance over vectors and
    /// points, with the gcd and promotion quirks.
    #[test]
    fn get_instance_pins() {
        assert_eq!(
            int_pair(&Direction::get_instance(&Vector::Int(IntVector::new(4, 2)))),
            (2, 1)
        );
        // T: rational (2, 4, 8) gcd-reduces to (1, 2).
        assert_eq!(int_pair(&Direction::get_instance(&rat(2, 4, 8))), (1, 2));
        // U: the huge rational (2^40, 2^40, 1) gcd-reduces back to an
        // IntDirection (1, 1).
        let huge_x = BigInt::from(1u32) << 40u32;
        let huge = Vector::Rational(Box::new(RationalVector::new(huge_x.clone(), huge_x, b(1))));
        assert_eq!(int_pair(&Direction::get_instance(&huge)), (1, 1));
        // V: the direction from (1, 1) to (4, 3) is the vector (3, 2);
        // equal points yield null.
        let from = Point::get_instance(1, 1);
        let to = Point::get_instance(4, 3);
        let d = Direction::get_instance_from_points(&from, &to)
            .expect("different points give a direction");
        assert_eq!(int_pair(&d), (3, 2));
        assert!(Direction::get_instance_from_points(&from, &from).is_none());
        // R/S: the binaryGcd Math.abs(i32::MIN) wraparound leaves the
        // vector unnormalized.
        assert_eq!(
            int_pair(&Direction::get_instance(&Vector::Int(IntVector::new(
                i32::MIN,
                0
            )))),
            (i32::MIN, 0)
        );
        assert_eq!(
            int_pair(&Direction::get_instance(&Vector::Int(IntVector::new(
                i32::MIN,
                i32::MIN
            )))),
            (i32::MIN, i32::MIN)
        );
    }

    /// jshell pins `O`/`P`: the approximation factories (both build an
    /// IntDirection from a plain IntVector — no CRIT_INT promotion).
    #[test]
    fn approx_factories() {
        assert_eq!(int_pair(&Direction::get_instance_approx(0.0)), (1, 0));
        assert_eq!(int_pair(&Direction::get_instance_approx(0.5)), (4388, 2397));
        assert_eq!(
            int_pair(&Direction::get_instance_approx(std::f64::consts::PI)),
            (-1, 0)
        );
        assert_eq!(
            int_pair(&Direction::get_instance_approx(std::f64::consts::FRAC_PI_2)),
            (0, 1)
        );
        assert_eq!(
            int_pair(&Direction::RIGHT.middle_approx(&Direction::UP)),
            (1, 1)
        );
    }

    /// jshell pin `N` and the side/projection/angle/predicate helpers.
    #[test]
    fn compare_from_side_projection_and_predicates() {
        assert_eq!(
            Direction::UP.compare_from(&Direction::RIGHT, &Direction::LEFT45),
            1
        );
        assert_eq!(
            Direction::UP.compare_from(&Direction::RIGHT, &Direction::RIGHT),
            0
        );
        assert_eq!(
            Direction::UP.compare_from(&Direction::UP, &Direction::LEFT45),
            -1
        );
        assert_eq!(Direction::UP.side_of(&Direction::RIGHT), Side::Positive);
        assert_eq!(Direction::UP.projection(&Direction::UP45), Side::Positive);
        assert_eq!(Direction::UP.projection(&Direction::DOWN), Side::Negative);
        assert_eq!(Direction::UP.angle_approx(), std::f64::consts::FRAC_PI_2);
        assert!(!Direction::DOWN.is_diagonal());
        assert!(Direction::DOWN.is_orthogonal());
        assert!(Direction::DOWN.is_multiple_of_45_degree());
        assert!(Direction::UP45.is_diagonal());
        assert!(Direction::UP45.is_multiple_of_45_degree());
        // opposite round trip: RIGHT.opposite() == LEFT, and turning by 4
        // equals the opposite (pin `h` at enum level).
        assert_eq!(Direction::RIGHT.opposite(), Direction::LEFT);
        assert_eq!(Direction::UP.turn_45_degree(4), Direction::UP.opposite());
    }

    /// Freezes the documented Ord/PartialEq divergence, confined to the
    /// NULL direction (jshell pins `L`/`M`): compareTo identifies NULL
    /// with the negative x axis (Equal), while equals does not (the zero
    /// projection is not POSITIVE). Reproduced Java behavior — do not
    /// "fix" one side to match the other.
    #[test]
    fn null_direction_ord_and_partial_eq_diverge() {
        assert_eq!(
            std::cmp::Ord::cmp(&Direction::NULL, &Direction::LEFT),
            Ordering::Equal
        );
        assert_eq!(
            Direction::NULL.compare_to(&Direction::LEFT),
            Ordering::Equal
        );
        assert_ne!(Direction::NULL, Direction::LEFT);
        assert_ne!(Direction::LEFT, Direction::NULL);
    }
}
