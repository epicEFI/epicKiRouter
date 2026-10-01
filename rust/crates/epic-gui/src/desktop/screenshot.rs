//! The smoke's PNG face (compiled ONLY under the dev-box `smoke`
//! feature = `["eframe/glow", "eframe/__screenshot"]`).
//!
//! THE PROBE RESULT (the charter's evidence face (e), recorded):
//! eframe 0.35 ships exactly ONE pixel-readback face — the
//! `__screenshot` feature's `EFRAME_SCREENSHOT_TO` hook
//! (`eframe-0.35.0/src/native/glow_integration.rs:1641`,
//! `save_screenshot_and_exit`: `egui_glow::Painter::
//! read_screen_rgba` PRE-SWAP, then the PNG write, then exit 0). The
//! wgpu backend's hook asserts "not yet implemented"
//! (`wgpu_integration.rs:117-120`).
//!
//! The RAW glow faces were attempted FIRST and are recorded here as
//! falsified: `glow::Context::read_pixels` at `App::ui` time reads
//! the BACK plane pre-paint (all black) or the FRONT plane post-swap
//! (also all black — this driver does not preserve post-swap planes;
//! both attempts witnessed live on the dev box). The hook is the
//! ONLY correct face: it runs inside eframe's paint step, before the
//! swap. It fires after egui pass 2 — so the smoke HOLDS PASS 1 OPEN
//! until the board is loaded and the route started (the
//! `hold_for_first_display` face), making the pass-2 shot show the
//! loaded board + the routing status.

use std::path::Path;

/// Arms eframe's `__screenshot` hook for the frames smoke (the ONLY
/// proven-correct PNG face — the module docs). Returns the pass-2
/// timing note.
///
/// # Safety (the `set_var` face)
///
/// Edition-2024 `set_var` is `unsafe`: at this point the process is
/// SINGLE-THREADED (before `eframe::run_native` constructs the app,
/// hence before the worker thread spawns), so no other thread can
/// concurrently read the environment.
#[cfg(feature = "smoke")]
pub(crate) fn arm_hook(out: &Path) -> Result<(), String> {
    #[allow(unsafe_code)]
    unsafe {
        std::env::set_var("EFRAME_SCREENSHOT_TO", out);
    }
    Ok(())
}
