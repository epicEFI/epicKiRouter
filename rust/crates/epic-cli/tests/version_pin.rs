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
    // EQUALITY against the derived form (the fix-round Q2): the test
    // crate sits in the epic-cli package, so it inherits the same
    // workspace version a hardcoded arm in main.rs would drift from —
    // such a mutant goes red here at the next bump (the `contains`
    // literal alone survived it, witnessed at review evidence/61).
    // The 2.0.0 literal stays as the checkpoint assert beneath it.
    assert_eq!(
        stdout.trim(),
        format!("epicrouter {}", env!("CARGO_PKG_VERSION")),
        "--version must print exactly the package version face"
    );
    assert!(
        stdout.contains("epicrouter 2.0.0"),
        "the 2.0.0 checkpoint: stdout must carry `epicrouter 2.0.0`: {stdout}"
    );
}
