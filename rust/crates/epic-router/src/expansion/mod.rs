//! Java `autoroute/expansion` — the expansion-graph primitives the
//! maze engine (T6) expands through: rooms, doors, target doors and
//! the sorted room-neighbour machinery.

pub mod door;
pub mod neighbours;
pub mod neighbours_forty_five;
#[cfg(test)]
pub(crate) mod pins;
pub mod room;
pub mod target_door;

pub use door::{ExpansionDoor, TRACE_WIDTH_TOLERANCE};
pub use neighbours::{
    CalculationMode, NeighbourEngine, TreeEntry, complete, select_calculation_mode,
};
pub use room::{ExpansionRoom, RoomKind};
pub use target_door::TargetItemExpansionDoor;
