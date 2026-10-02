//! The readiness-fix M1/M2 bin-level pins: the built `epic-cli` debug
//! bin spawned via `CARGO_BIN_EXE_epic-cli` (the established
//! lesson-20 pattern — compile-time env!, never runtime lookup; bin
//! freshness by construction, the DNR-17 structural face). The help
//! text's settings table is the M1 const in `main.rs`; these pins
//! assert a REPRESENTATIVE set of names appears (the charter's
//! fallback: the table is a static const, not mechanically derived
//! from the parse match — the representative-set pin is what kills a
//! table-drift mutant).

use std::process::Command;

const CLI_BIN: &str = env!("CARGO_BIN_EXE_epic-cli");

/// `--help`: stdout, exit 0, and a representative set of the parseable
/// settings + the short flags appear.
#[test]
fn help_flag_exits_zero_and_lists_the_surface() {
    let output = Command::new(CLI_BIN)
        .arg("--help")
        .output()
        .expect("the epic-cli bin runs");
    assert_eq!(
        output.status.code(),
        Some(0),
        "--help must exit 0 (a discovery surface, not the usage-error class)"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for needle in [
        "usage: epic-cli route -de <board.dsn> -do <out.ses>",
        "router.enabled=<on|off>",
        "router.via_costs=<integer>",
        "router.strict_drc=<on|off>",
        "router.fanout.enabled=<on|off>",
        "router.optimizer.enabled=<on|off>",
        "router.optimizer.improvement_threshold=<float>",
        "router.tuning.pairs=",
        "router.assign.pins=<REF[,REF...]>",
        "router.current.nets=<NET:AMPS[,NET:AMPS...]>",
        "router.current.copper_oz=<float>",
        "router.gloss.bus=<on|off>",
        "optimizer.* mirrors",
        "-mp <passes>",
        "-mt <threads>",
        "-oit <threshold>",
        "--result-json <manifest.json>",
        "--deterministic-budgets=on|off",
    ] {
        assert!(
            stdout.contains(needle),
            "help must list `{needle}`:\n{stdout}"
        );
    }
    // `--help` must NOT print to stderr (discovery goes to stdout).
    assert!(
        String::from_utf8_lossy(&output.stderr).is_empty(),
        "--help writes nothing to stderr"
    );
}

/// `help` (the bare word): the same stdout face, exit 0.
#[test]
fn help_word_exits_zero_with_the_same_face() {
    let output = Command::new(CLI_BIN)
        .arg("help")
        .output()
        .expect("the epic-cli bin runs");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("usage: epic-cli route -de <board.dsn> -do <out.ses>"));
    assert!(stdout.contains("router.scoring.version="));
}

/// No arguments: the SAME help text on STDERR, exit 2 — the usage
/// class is unchanged; the run is an error, just an informed one.
#[test]
fn no_args_prints_help_to_stderr_and_exits_two() {
    let output = Command::new(CLI_BIN)
        .output()
        .expect("the epic-cli bin runs");
    assert_eq!(
        output.status.code(),
        Some(2),
        "no-args keeps the usage-error exit class"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("usage: epic-cli route -de <board.dsn> -do <out.ses>"));
    assert!(stderr.contains("router.via_costs=<integer>"));
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
}

/// Readiness-fix M2, bin level: `-do` under a path that cannot be a
/// directory (a regular file is) fails as the USAGE class (exit 2)
/// with a message naming the output path — before any routing work.
/// The design file need not exist for the pre-flight to fire first;
/// this pin also proves the ordering (a missing -de would otherwise
/// surface as exit 1 `cannot read design`, never 2 with THIS message).
#[test]
fn unwritable_output_parent_is_the_usage_class_exit_two() {
    let dir = std::env::temp_dir().join(format!("epic-fix-m2-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let blocker = dir.join("blocker");
    std::fs::write(&blocker, b"not a directory").expect("blocker file");
    let output = Command::new(CLI_BIN)
        .args([
            "route",
            "-de",
            dir.join("absent.dsn").to_string_lossy().as_ref(),
            "-do",
            blocker.join("out.ses").to_string_lossy().as_ref(),
        ])
        .output()
        .expect("the epic-cli bin runs");
    assert_eq!(
        output.status.code(),
        Some(2),
        "the output pre-flight failure is the usage class (exit 2), before the input read"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("output directory not usable"),
        "the pre-flight message must name the -do parent: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
