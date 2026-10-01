//! The epic-gui headless view core (M9-T4): the pure
//! [`view::ViewModel`] -> [`render::RenderList`] projection with
//! committed goldens (the design §7 contract: the GUI renders, never
//! mutates — this crate holds snapshots and views, never boards; the
//! desktop shell rides the default-off `desktop` feature in M9-T6,
//! so the default workspace build gains no GUI deps).
//!
//! - [`view`]: the view state — [`view::ViewModel`],
//!   [`view::ColorTable`] (the documented Java-derived default), the
//!   pan/zoom [`view::ScreenTransform`] (world DBU i64 <-> screen px
//!   i32, exactness contract on the type), and the T4 plumbing of the
//!   [`view::OverlayFlags`].
//! - [`render`]: the projection —
//!   [`render::project`](render::project) is a PURE function of
//!   `(snapshot, view, viewport)`: same inputs, byte-identical
//!   [`render::RenderList`], always (deterministic op order, BTree
//!   only, cull precheck with the inclusive DNR-16 boundary face;
//!   goldens in `harness/fixtures/gui-render/golden/`).

pub mod render;
pub mod shell;
pub mod view;

// The desktop shell (M9-T6): the thin eframe host — worker thread,
// channels, panels, canvas. DEFAULT-OFF: the default workspace build
// compiles none of this (no wgpu tree); `--features desktop` gates
// the whole module (the census-pinnability law's other half — every
// pure face the pins need lives UNGATED in [`shell`]).
#[cfg(feature = "desktop")]
pub mod desktop;

#[cfg(test)]
mod tests {
    /// The crate is a scaffold in M0; the headless view core landed
    /// in M9-T4 (the pin stays).
    #[test]
    fn crate_scaffolds() {
        let name = env!("CARGO_PKG_NAME");
        assert_eq!(name, "epic-gui");
    }
}
