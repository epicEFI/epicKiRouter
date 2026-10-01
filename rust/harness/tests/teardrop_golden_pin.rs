//! M8-T6: the teardrop ON-face goldens re-derive byte-identically
//! through the FULL in-process product face (`epic_cli::route::run_route`
//! — parse → routing stage → gloss stages → SES/manifest/sidecar
//! write). The committed fixtures (`fixtures/gloss/teardrop_world*.dsn`)
//! are fully prerouted NINETY_DEGREE worlds: net "sig" runs pin A1 →
//! A2 on F.Cu (trace width 200 DSN into pad diameter 2000 DSN — the
//! teardrop anatomy); the blocked world adds net "other", a foreign
//! run hovering over BOTH junctions' taper spans at bottom-edge gap
//! exactly equal to the 2_000-DBU clearance — the REAL counter's
//! blocked face (all four rows honest `"landed": false`). The blocked
//! world additionally carries 4 PRE-EXISTING foreign pad-pad clearance
//! overlaps (A1–B1 ≈ 5 682 and A2–B2 ≈ 3 599 board DBU — the 2_000-DSN
//! circle pads are 20_000 DBU in DIAMETER, the buglog-218
//! diameter-not-radius trap), recorded at capture
//! (logs/M8-T6/evidence/10-capture.log:11: `score 957.50 (0 unrouted
//! and 4 violations)`). They pre-date the teardrop stage and affect NO
//! pin here: no pin asserts this world's cleanliness — the DRC-clean
//! law is pinned by td3 on clean unit worlds — and the blocked face's
//! pins assert byte-identity plus the four honest `"landed": false`
//! rows. The ON face is
//! `--router.gloss.teardrops=on` with the optimizer DISABLED (the
//! buglog-213 capture trap: the crafted 90° geometry is outside the
//! enabled-optimizer's reshape-and-panic scope — the capture face
//! records this; pre-existing, NOT T6's to fix). The teardrop stage is
//! the TERMINAL gloss slot (after via-place — the recorded slot
//! decision in gloss.rs); the bus/flow/via flags stay OFF in these
//! faces so the teardrop pass is the only mover.
//!
//! Pins here: golden byte-identity of SES + sidecar for both worlds,
//! determinism ×2, the `-mt 1`/`-mt 3` threads-invariance witness
//! (the M5-T7 contract — the gloss pass is sequential, so thread count
//! must not leak into output bytes), the DEFAULT face: no
//! `gloss_teardrops` sidecar block (skip-when-empty), input geometry
//! verbatim, and the flag is the only gate (ON differs, default does
//! not gloss).
//!
//! All runs are IN-PROCESS — no spawned binaries, so the buglog-184
//! stale-bin family cannot apply.

use std::fs;
use std::path::{Path, PathBuf};

use epic_cli::route::run_route;
use epic_engine::settings::{ParsedRouteArgs, parse_route_args};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("harness dir has a parent")
        .parent()
        .expect("rust dir has a parent")
        .to_path_buf()
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("epic-m8t6-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("scratch dir created");
    dir.join(name)
}

/// Runs the full product flow over a teardrop fixture. `teardrops_on`
/// — the `router.gloss.teardrops` flag; `max_threads` — the explicit
/// `-mt` face (pre-tokenized argv list, the buglog-209 trap).
fn run_flow(
    dsn: &Path,
    tag: &str,
    teardrops_on: bool,
    max_threads: Option<i32>,
) -> (Vec<u8>, Vec<u8>) {
    let ses = scratch(&format!("{tag}.ses"));
    let manifest = scratch(&format!("{tag}.manifest.json"));
    let sidecar = scratch(&format!("{tag}.sidecar.json"));
    let mut argv = vec![
        "-de".to_string(),
        dsn.to_string_lossy().into_owned(),
        "-do".to_string(),
        ses.to_string_lossy().into_owned(),
        "--result-json".to_string(),
        manifest.to_string_lossy().into_owned(),
        "--dump-aesthetics".to_string(),
        sidecar.to_string_lossy().into_owned(),
        "--optimizer.enabled=off".to_string(),
    ];
    if let Some(threads) = max_threads {
        argv.push("-mt".to_string());
        argv.push(threads.to_string());
    }
    if teardrops_on {
        argv.push("--router.gloss.teardrops=on".to_string());
    }
    // The argv IS the production path (the T3 harness-pin rule: no
    // manual layer override beside it).
    let args: ParsedRouteArgs = parse_route_args(&argv).expect("argv parses");
    let exit = run_route(&args).expect("run_route succeeds");
    assert_eq!(exit, 0, "the fixture completes cleanly");
    (
        fs::read(&ses).expect("ses written"),
        fs::read(&sidecar).expect("sidecar written"),
    )
}

/// Both ON faces re-derive byte-identically; determinism ×2 holds;
/// `-mt 1` and `-mt 3` outputs are byte-identical; the ON-face
/// semantic pins read the sidecar bytes (the blocked world carries the
/// honest `"landed": false` rows, the clean world lands both
/// junctions).
#[test]
fn teardrop_goldens_rederive_byte_identically() {
    for (fixture, blocked) in [("teardrop_world", false), ("teardrop_world_blocked", true)] {
        let dsn = repo_root().join(format!("rust/harness/fixtures/gloss/{fixture}.dsn"));
        let golden_ses =
            fs::read(repo_root().join(format!("rust/harness/fixtures/gloss/golden/{fixture}.ses")))
                .expect("committed SES golden");
        let golden_sidecar = fs::read(repo_root().join(format!(
            "rust/harness/fixtures/gloss/golden/{fixture}.sidecar.json"
        )))
        .expect("committed sidecar golden");

        let (ses1, sidecar1) = run_flow(&dsn, &format!("{fixture}-a"), true, None);
        assert_eq!(
            ses1, golden_ses,
            "{fixture}: SES ON face matches the committed golden"
        );
        assert_eq!(
            sidecar1, golden_sidecar,
            "{fixture}: sidecar ON face matches the committed golden"
        );

        // Determinism ×2 (fresh flow, fresh outputs).
        let (ses2, sidecar2) = run_flow(&dsn, &format!("{fixture}-b"), true, None);
        assert_eq!(ses2, ses1, "{fixture}: determinism ×2 (SES)");
        assert_eq!(sidecar2, sidecar1, "{fixture}: determinism ×2 (sidecar)");

        // Threads-invariance: -mt 1 vs -mt 3, byte-identical.
        let (ses_mt1, sidecar_mt1) = run_flow(&dsn, &format!("{fixture}-mt1"), true, Some(1));
        let (ses_mt3, sidecar_mt3) = run_flow(&dsn, &format!("{fixture}-mt3"), true, Some(3));
        assert_eq!(ses_mt1, ses_mt3, "{fixture}: -mt 1 == -mt 3 (SES)");
        assert_eq!(
            sidecar_mt1, sidecar_mt3,
            "{fixture}: -mt 1 == -mt 3 (sidecar)"
        );

        // The ON-face semantic pins on the sidecar bytes.
        let text = String::from_utf8(sidecar1).expect("sidecar is UTF-8");
        assert!(
            text.contains("\"gloss_teardrops\""),
            "{fixture}: block present"
        );
        assert!(
            text.contains("\"pad_diameter\": 20000"),
            "{fixture}: the pad row"
        );
        assert!(
            text.contains("\"net_name\": \"sig\""),
            "{fixture}: net named"
        );
        if blocked {
            assert!(
                text.contains("\"landed\": false"),
                "blocked world: the honest stop is recorded"
            );
            assert!(
                !text.contains("\"landed\": true"),
                "blocked world: nothing landed"
            );
        } else {
            assert!(
                !text.contains("\"landed\": false"),
                "clean world: both junctions land"
            );
            assert!(text.contains("\"landed\": true"), "clean world: landed");
        }
    }
}

/// The DEFAULT face (flag off) on the teardrop world: the sidecar
/// carries NO `gloss_teardrops` block (skip-when-empty — the
/// sidecar-rotation law), the SES stays at the input geometry, and the
/// default SES differs from the ON face (the flag is the only gate —
/// an ON run legitimately differs, a default run never glosses).
#[test]
fn teardrop_default_face_rotates_nothing() {
    let dsn = repo_root().join("rust/harness/fixtures/gloss/teardrop_world.dsn");
    let (ses_default, sidecar_default) = run_flow(&dsn, "teardrop-default", false, None);
    let text = String::from_utf8(sidecar_default).expect("sidecar is UTF-8");
    assert!(
        !text.contains("gloss_teardrops"),
        "default sidecar carries no gloss_teardrops block"
    );
    // Default SES = the input geometry verbatim (the plain trace at
    // width 200 DSN; the ON face adds the graded taper wires).
    let default_text = String::from_utf8(ses_default.clone()).expect("ses is UTF-8");
    assert!(
        default_text.contains("(path F.Cu 200"),
        "input trace geometry preserved"
    );
    assert!(
        !default_text.contains("(path F.Cu 20000"),
        "no taper width in the default face"
    );
    let (ses_on, _) = run_flow(&dsn, "teardrop-on", true, None);
    assert_ne!(
        ses_default, ses_on,
        "the gloss flag is the ONLY gate: ON differs, default does not"
    );
}
