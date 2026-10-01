//! The M10-T6 fix-round Q1 pin: the built `epic-gui` bin, spawned with
//! `--version`, exits 0 and prints EXACTLY the single-sourced
//! [`epic_gui::shell::version_line`] face — the SAME source the ungated
//! census pin guards (`shell.rs` tests), so a stale hardcode at the bin's
//! print site dies here (the QD1 mutant-a survivor, review evidence 41).
//! Mirrors `epic-cli/tests/version_pin.rs`.
//!
//! `#![cfg(feature = "desktop")]`: the shipped bin is desktop-only, so the
//! file compiles EMPTY at default features — OUT of the default workspace
//! census (declared 1690 + 0 = 1690/0/19) — and its own green log is the
//! pin's gate face:
//! `cargo test -p epic-gui --test version_pin --features desktop`
//! (capped, `--jobs 4`).
//!
//! Bin freshness by construction: `CARGO_BIN_EXE_epic-gui` makes cargo
//! build the exact bin the test runs (the DNR-17 stale-bin precondition is
//! structural — the epic-cli pin's same face). The var is read via
//! compile-time `env!` (the epic-cli pin's proven shape — the runtime
//! `std::env::var` face is NOT provided by cargo, witnessed by the kept
//! red round 55); the read sits INSIDE this cfg'd file, so default-feature
//! builds never evaluate it.

#![cfg(feature = "desktop")]

use std::process::Command;

const GUI_BIN: &str = env!("CARGO_BIN_EXE_epic-gui");

#[test]
fn version_flag_prints_the_single_sourced_line_and_exits_zero() {
    let output = Command::new(GUI_BIN)
        .arg("--version")
        .output()
        .expect("the epic-gui bin runs");
    assert!(
        output.status.success(),
        "--version must exit 0 (got {:?})",
        output.status.code()
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    // EQUALITY against the single source (the Q1 fix): the bin's printed
    // line must equal the SAME `version_line()` the ungated pin guards —
    // a hardcoded print arm drifts from it at the next bump and dies here.
    assert_eq!(
        stdout.trim(),
        epic_gui::shell::version_line(),
        "--version must print exactly the single-sourced version face"
    );
    // The 2.0.0 checkpoint beneath (the epic-cli pin's shape).
    assert!(
        stdout.contains("epicrouter 2.0.0"),
        "the 2.0.0 checkpoint: stdout must carry `epicrouter 2.0.0`: {stdout}"
    );
}

#[test]
fn version_flag_rejects_extra_arguments() {
    let output = Command::new(GUI_BIN)
        .arg("--version")
        .arg("extra")
        .output()
        .expect("the epic-gui bin runs");
    assert!(
        !output.status.success(),
        "--version with an extra argument must exit non-zero (the arity face)"
    );
}
