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
        "router.pour.nets=<NET[,NET...]>",
        "router.pour.layer=<layer name>",
        "router.gloss.bus=<on|off>",
        "optimizer.* mirrors",
        "-mp <passes>",
        "-mt <threads>",
        "-oit <threshold>",
        "--result-json <manifest.json>",
        "--deterministic-budgets=on|off",
        "--interview=on|off|show",
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

/// F4, bin level: `--interview=show` prints the board-derived
/// questions (each with the `--router.` fragment that answers it)
/// and routes unchanged — and WITHOUT the flag the stderr carries
/// no interview lines at all (the OFF default: scripted pipelines
/// and every golden face stay byte-identical). The F3 ask still
/// fires in both runs (its own default-on face, unchanged).
#[test]
fn interview_show_lists_questions_and_off_default_prints_none() {
    // The lean interview craft: GND (ground, unpoured, 3 pins),
    // USB_DP/USB_DN (the 2-char pair family), 3V3 (a power rail),
    // DATA0/DATA1 (ordinary nets that must raise nothing).
    const INTERVIEW_DSN: &str = r#"(pcb interview-cli.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal)(property(index 0)))
    (layer B.Cu (type signal)(property(index 1)))
    (boundary (path pcb 0  0 0  128000 0  128000 128000  0 128000  0 0))
    (rule (clearance 250))
  )
  (placement
    (component "CONN" (place "CONN1" 20000 64000 Front 0.000000))
    (component "TGT" (place "T9" 36000 4000 Front 0.000000))
    (component "TGT" (place "T8" 36000 8000 Front 0.000000))
    (component "TGT" (place "T7" 36000 12000 Front 0.000000))
    (component "TGT" (place "T6" 36000 16000 Front 0.000000))
    (component "TGT" (place "T5" 36000 20000 Front 0.000000))
    (component "TGT" (place "T4" 100000 32000 Front 0.000000))
    (component "TGT" (place "T3" 100000 40000 Front 0.000000))
    (component "TGT" (place "T2" 100000 48000 Front 0.000000))
    (component "TGT" (place "T1" 100000 56000 Front 0.000000))
  )
  (library
    (image "CONN"
      (pin "PAD" "CA1" 0 0)
      (pin "PAD" "CA2" 0 -8000)
      (pin "PAD" "CA3" 0 -16000)
      (pin "PAD" "CA4" 0 -24000)
    )
    (image "TGT"
      (pin "PAD" "TA" 0 0)
    )
    (padstack "PAD"
      (shape (circle F.Cu 2000))
      (attach off)
    )
  )
  (network
    (net "GND" (pins "CONN1"-"CA1" "T1"-"TA" "T2"-"TA"))
    (net "USB_DP" (pins "CONN1"-"CA2" "T3"-"TA"))
    (net "USB_DN" (pins "CONN1"-"CA3" "T4"-"TA"))
    (net "3V3" (pins "CONN1"-"CA4" "T5"-"TA"))
    (net "DATA0" (pins "T6"-"TA" "T7"-"TA"))
    (net "DATA1" (pins "T8"-"TA" "T9"-"TA"))
    (class kicad_default "GND" "USB_DP" "USB_DN" "3V3" "DATA0" "DATA1"
      (rule (clearance 250)(width 200))
    )
  )
)"#;
    let dir = std::env::temp_dir().join(format!("epic-f4-cli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let dsn = dir.join("interview.dsn");
    std::fs::write(&dsn, INTERVIEW_DSN).expect("write dsn");

    // SHOW: the questions + their answering fragments, exit 0.
    let show = Command::new(CLI_BIN)
        .args([
            "route",
            "-de",
            dsn.to_string_lossy().as_ref(),
            "-do",
            dir.join("show.ses").to_string_lossy().as_ref(),
            "--interview=show",
        ])
        .output()
        .expect("the epic-cli bin runs");
    assert_eq!(show.status.code(), Some(0), "show routes normally");
    let stderr = String::from_utf8_lossy(&show.stderr);
    assert!(
        stderr.contains("interview: net GND (3 pins) has no copper pour"),
        "the pour question prints:\n{stderr}"
    );
    assert!(
        stderr.contains("--router.tuning.pairs=USB_DP:USB_DN"),
        "the pair question carries its answer fragment:\n{stderr}"
    );
    assert!(
        stderr.contains("--router.current.nets=3V3:<amps>"),
        "the current question carries its answer fragment:\n{stderr}"
    );
    assert!(
        !stderr.contains("DATA0"),
        "ordinary nets raise no question:\n{stderr}"
    );

    // The OFF control: the same run without the flag prints NO
    // interview lines (byte-identical default), and the F3 ask is
    // unchanged.
    let off = Command::new(CLI_BIN)
        .args([
            "route",
            "-de",
            dsn.to_string_lossy().as_ref(),
            "-do",
            dir.join("off.ses").to_string_lossy().as_ref(),
        ])
        .output()
        .expect("the epic-cli bin runs");
    assert_eq!(off.status.code(), Some(0));
    let off_stderr = String::from_utf8_lossy(&off.stderr);
    assert!(
        !off_stderr.contains("interview:"),
        "no flag, no interview output:\n{off_stderr}"
    );
    assert!(
        off_stderr.contains("note: net GND (3 pins) has no copper pour"),
        "the F3 ask still fires by default:\n{off_stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
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
