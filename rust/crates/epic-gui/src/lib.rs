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
pub mod view;

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
