//! The M10-T5 `--version` pin: the built `epic-cli` bin, spawned with
//! `--version`, prints `epicrouter 2.0.0` on stdout and exits 0. The
//! expected string is the charter LITERAL (a future bump must update
//! this pin — that is the pin working).
//!
//! Bin freshness by construction: `CARGO_BIN_EXE_epic-cli` makes cargo
//! build the exact bin the test runs (the DNR-17 stale-bin
//! precondition is structural — the same face as
//! `harness/tests/global_golden_pin.rs`).

use std::process::Command;

const CLI_BIN: &str = env!("CARGO_BIN_EXE_epic-cli");

#[test]
fn version_flag_prints_epicrouter_2_0_0_and_exits_zero() {
    let output = Command::new(CLI_BIN)
        .arg("--version")
        .output()
        .expect("the epic-cli bin runs");
    assert!(
        output.status.success(),
        "--version must exit 0 (got {:?})",
        output.status.code()
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("epicrouter 2.0.0"),
        "stdout must carry `epicrouter 2.0.0`: {stdout}"
    );
}
