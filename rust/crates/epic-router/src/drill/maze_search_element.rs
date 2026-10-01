//! Java `autoroute/maze/MazeSearchElement.java` (40 lines) — the
//! per-section maze search state both drill types embed
//! (`DrillPage.mazeSearchElements`, one per board layer;
//! `ExpansionDrill.mazeSearchElements`, one per drill layer).
//! Minimal twin: T6 owns the front's USE of these fields.

/// Java `MazeSearchElement.Adjustment` — declaration order mirrored
/// (`NONE, RIGHT, LEFT`); referenced as `Adjustment.NONE` by
/// `expandToOtherLayers:371` when seeding the emitted maze element.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Adjustment {
    /// Java `NONE`.
    #[default]
    None,
    /// Java `RIGHT`.
    Right,
    /// Java `LEFT`.
    Left,
}

/// Java `MazeSearchElement` — all defaults from the Java field
/// initializers (`isOccupied = false`, `backtrackDoor = null`,
/// `sectionNoOfBacktrackDoor = 0`, `roomRipped = false`,
/// `adjustment = Adjustment.NONE`, `ripupCost = 0`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MazeSearchElement {
    /// Java `isOccupied` — a maze section already claimed this layer.
    pub is_occupied: bool,
    /// Java `backtrackDoor` — an `ExpandableObject` reference; an
    /// opaque registry key here (the D17 decision).
    pub backtrack_door: Option<u64>,
    /// Java `sectionNoOfBacktrackDoor`.
    pub section_no_of_backtrack_door: i32,
    /// Java `roomRipped`.
    pub room_ripped: bool,
    /// Java `adjustment`.
    pub adjustment: Adjustment,
    /// Java `ripupCost`.
    pub ripup_cost: i32,
}

impl MazeSearchElement {
    /// Java `reset()` (`:26-36`): every field back to its initializer.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}
