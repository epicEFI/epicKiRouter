//! The M6-T7 golden pin: the committed MASTER-face settings-ON golden
//! (`rust/harness/baselines/global/t7_ripup.global-golden.json`) must
//! VERIFY against a fresh MASTER-face settings-ON run. Drift in the
//! congestion map, the guides, the planned order, or the engine's
//! settings-ON route rotates the SES/manifest digests and this pin
//! dies. This pin never executes pattern.rs or the pathfinder
//! scheduler (the master face does not enable their flags — the C5-1
//! overclaim correction, M6-T8; the pattern/pathfinder faces carry
//! their own goldens and pins).
//!
//! Bin freshness by construction: `CARGO_BIN_EXE_epic-harness` makes
//! cargo build the exact bin the test runs (the DNR-17 stale-bin
//! precondition is structural).

use std::process::Command;

const HARNESS_BIN: &str = env!("CARGO_BIN_EXE_epic-harness");

#[test]
fn settings_on_golden_verifies_against_a_fresh_run() {
    let mut cmd = Command::new(HARNESS_BIN);
    cmd.args([
        "global-golden",
        "verify",
        "--dsn",
        "rust/harness/fixtures/maze-spike/t7_ripup.dsn",
        "--golden",
        "rust/harness/baselines/global/t7_ripup.global-golden.json",
        // A PRIVATE work root: libtest runs this binary's tests in
        // PARALLEL, and two concurrent verifies would race on the
        // default `runs/global-golden/verify/` scratch dir (one pin
        // reading the other pin's SES — witnessed live in the first
        // T9 battery run, buglog 204).
        "--work-root",
        "rust/harness/runs/global-golden-pin-master",
    ]);
    let output = cmd.output().expect("the harness bin runs");
    assert!(
        output.status.success(),
        "settings-ON golden verify failed (exit {:?}): {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("exit=0"),
        "the verify face must print its exit line: {stdout}"
    );
}

/// The M6-T9 push-and-shove golden pin: the committed
/// PUSH_SHOVE-face settings-ON golden
/// (`rust/harness/baselines/global/t7_ripup.push_shove.global-golden.json`)
/// must VERIFY against a fresh `--router.push_shove=on` run of the
/// SAME t7_ripup fixture. The face was proven firing BEFORE capture
/// (default SES aa05316d… vs ON b481a8a3… — the T8 deviation-2
/// lesson), so this pin judges a real divergence face: any drift in
/// the waiver predicate, the per-search budget arithmetic, or the
/// shove-composed insertion rotates the digests and the pin dies.
/// The golden's recorded flags are asserted against the face's flags
/// BEFORE the digest compare (C5-1 — a golden captured under one
/// face can never silently verify under another).
#[test]
fn push_shove_golden_verifies_against_a_fresh_run() {
    let mut cmd = Command::new(HARNESS_BIN);
    cmd.args([
        "global-golden",
        "verify",
        "--dsn",
        "rust/harness/fixtures/maze-spike/t7_ripup.dsn",
        "--golden",
        "rust/harness/baselines/global/t7_ripup.push_shove.global-golden.json",
        "--face",
        "push_shove",
        // A PRIVATE work root — see the master pin's comment (the two
        // pins run in parallel under libtest; the shared default
        // `verify/` scratch dir made them read each other's SES).
        "--work-root",
        "rust/harness/runs/global-golden-pin-push-shove",
    ]);
    let output = cmd.output().expect("the harness bin runs");
    assert!(
        output.status.success(),
        "push_shove golden verify failed (exit {:?}): {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("exit=0"),
        "the verify face must print its exit line: {stdout}"
    );
}
