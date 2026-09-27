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
