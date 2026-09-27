//! The global-planning stage (M6-T7) — the congestion map, per-net
//! guides, the planned net order, and L/Z pattern routing (design
//! §4.2 stage 2), entirely behind `router.congestion_global`
//! (default OFF — the T6 `-mt` seam precedent: the entire recurring
//! gate set stays byte-identical at defaults).
//!
//! Beyond-Java: no Java oracle exists for any of this (the T6
//! interpretation-5 precedent) — the parity claim is the default-off
//! byte-identity of the recurring gates alone; every settings-ON face
//! is beyond-Java.

pub mod history;
pub mod map;
pub mod pattern;
pub mod plan;

#[cfg(test)]
mod tests;
