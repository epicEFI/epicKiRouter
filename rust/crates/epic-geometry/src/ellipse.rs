//! Port of Java `app.freerouting.geometry.planar.Ellipse` — describes an
//! ellipse in the plane with float coordinates. Java does NOT implement
//! `ConvexShape` for it ("because coordinates are float"), and the class
//! has no methods beyond the constructor; the port mirrors that with a
//! plain data struct (no `Shape` impl, no `Display` — Java has no
//! `toString`).

use crate::float_point::FloatPoint;

/// Java `Ellipse`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ellipse {
    /// The center (Java public final field).
    pub center: FloatPoint,
    /// Rotation of the ellipse in radian normed to 0 <= rotation < pi
    /// (Java public final field).
    pub rotation: f64,
    /// The bigger of the two radii (Java public final field).
    pub bigger_radius: f64,
    /// The smaller of the two radii (Java public final field).
    pub smaller_radius: f64,
}

impl Ellipse {
    /// Creates a new instance of Ellipse (Java
    /// `Ellipse(FloatPoint, double, double, double)`): radii are ordered
    /// (swapping adds pi/2 to the rotation) and the rotation is normed
    /// into [0, pi) by repeated addition/subtraction of pi.
    pub fn new(center: FloatPoint, rotation: f64, radius1: f64, radius2: f64) -> Ellipse {
        let bigger_radius;
        let smaller_radius;
        let mut current_rotation;
        if radius1 >= radius2 {
            bigger_radius = radius1;
            smaller_radius = radius2;
            current_rotation = rotation;
        } else {
            bigger_radius = radius2;
            smaller_radius = radius1;
            current_rotation = rotation + 0.5 * std::f64::consts::PI;
        }
        while current_rotation >= std::f64::consts::PI {
            current_rotation -= std::f64::consts::PI;
        }
        while current_rotation < 0.0 {
            current_rotation += std::f64::consts::PI;
        }
        Ellipse {
            center,
            rotation: current_rotation,
            bigger_radius,
            smaller_radius,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Radii are ordered and the rotation normalization keeps [0, pi).
    #[test]
    fn ctor_ordering_and_normalization() {
        let e = Ellipse::new(FloatPoint::new(1.0, 2.0), 0.5, 3.0, 5.0);
        assert_eq!(e.bigger_radius, 5.0);
        assert_eq!(e.smaller_radius, 3.0);
        // radius1 < radius2 adds pi/2: 0.5 + pi/2 ~ 2.07 < pi
        assert!((e.rotation - (0.5 + std::f64::consts::FRAC_PI_2)).abs() < 1e-12);

        // negative rotation is lifted into [0, pi)
        let e2 = Ellipse::new(FloatPoint::ZERO, -0.25, 5.0, 3.0);
        assert!((e2.rotation - (-0.25 + std::f64::consts::PI)).abs() < 1e-12);

        // rotation >= pi wraps down
        let e3 = Ellipse::new(FloatPoint::ZERO, std::f64::consts::PI + 0.125, 5.0, 3.0);
        assert!((e3.rotation - 0.125).abs() < 1e-12);
    }
}
