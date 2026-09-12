//! Port of Java `app.freerouting.geometry.planar.FortyfiveDegreeDirection`.

use crate::int_direction::IntDirection;

/// Enum for the eight 45-degree direction starting from right in
/// counterclocksense to down45 (Java doc verbatim).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FortyfiveDegreeDirection {
    /// East.
    RIGHT,
    /// Northeast.
    RIGHT45,
    /// North.
    UP,
    /// Northwest.
    UP45,
    /// West.
    LEFT,
    /// Southwest.
    LEFT45,
    /// South.
    DOWN,
    /// Southeast.
    DOWN45,
}

impl FortyfiveDegreeDirection {
    /// All eight constants in Java `values()` declaration order (the
    /// iteration order of `IntOctagon.nearestBorderProjections`).
    pub const VALUES: [FortyfiveDegreeDirection; 8] = [
        Self::RIGHT,
        Self::RIGHT45,
        Self::UP,
        Self::UP45,
        Self::LEFT,
        Self::LEFT45,
        Self::DOWN,
        Self::DOWN45,
    ];

    /// Returns the matching direction constant (Java `getDirection()`).
    pub fn get_direction(self) -> IntDirection {
        match self {
            Self::RIGHT => IntDirection::RIGHT,
            Self::RIGHT45 => IntDirection::RIGHT45,
            Self::UP => IntDirection::UP,
            Self::UP45 => IntDirection::UP45,
            Self::LEFT => IntDirection::LEFT,
            Self::LEFT45 => IntDirection::LEFT45,
            Self::DOWN => IntDirection::DOWN,
            Self::DOWN45 => IntDirection::DOWN45,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Java `getDirection()` maps every enum constant onto the matching
    /// IntDirection constant.
    #[test]
    fn get_direction_maps_all_eight() {
        assert_eq!(
            FortyfiveDegreeDirection::RIGHT.get_direction(),
            IntDirection::RIGHT
        );
        assert_eq!(
            FortyfiveDegreeDirection::RIGHT45.get_direction(),
            IntDirection::RIGHT45
        );
        assert_eq!(
            FortyfiveDegreeDirection::UP.get_direction(),
            IntDirection::UP
        );
        assert_eq!(
            FortyfiveDegreeDirection::UP45.get_direction(),
            IntDirection::UP45
        );
        assert_eq!(
            FortyfiveDegreeDirection::LEFT.get_direction(),
            IntDirection::LEFT
        );
        assert_eq!(
            FortyfiveDegreeDirection::LEFT45.get_direction(),
            IntDirection::LEFT45
        );
        assert_eq!(
            FortyfiveDegreeDirection::DOWN.get_direction(),
            IntDirection::DOWN
        );
        assert_eq!(
            FortyfiveDegreeDirection::DOWN45.get_direction(),
            IntDirection::DOWN45
        );
    }
}
