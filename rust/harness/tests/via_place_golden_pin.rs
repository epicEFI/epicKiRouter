//! M8-T5: the via-place ON-face goldens re-derive byte-identically
//! through the FULL in-process product face (`epic_cli::route::run_route`
//! — parse → routing stage → gloss stages → SES/manifest/sidecar
//! write). The committed fixtures (`fixtures/gloss/via_world*.dsn`) are
//! fully prerouted NINETY_DEGREE worlds whose net "sig" runs pin A1 →
//! F.Cu arm → via at (20000,15000) → staircase B.Cu arm → pin A2; the
//! ON face is `--router.gloss.via_place=on` with the optimizer DISABLED
//! (the buglog-213 trap: the crafted 90° geometry is outside the
//! enabled-optimizer's reshape-and-panic scope — the capture face
//! records this; pre-existing, NOT T5's to fix). The via stage is the
//! TERMINAL gloss slot (after flow — the recorded slot decision in
//! gloss.rs); the bus/flow flags stay OFF in these faces so the via
//! pass is the only mover.
//!
//! Pins here: golden byte-identity of SES + sidecar for both worlds,
//! determinism ×2, the `-mt 1`/`-mt 3` threads-invariance witness
//! (the M5-T7 contract — the gloss pass is sequential, so thread count
//! must not leak into output bytes), and the DEFAULT face: no
//! `gloss_via_place` sidecar block (skip-when-empty), input geometry
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
    let dir = std::env::temp_dir().join(format!("epic-m8t5-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("scratch dir created");
    dir.join(name)
}

/// Runs the full product flow over a via fixture. `via_on` — the
/// `router.gloss.via_place` flag; `max_threads` — the explicit `-mt`
/// face (pre-tokenized argv list, the buglog-209 trap).
fn run_flow(dsn: &Path, tag: &str, via_on: bool, max_threads: Option<i32>) -> (Vec<u8>, Vec<u8>) {
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
    if via_on {
        argv.push("--router.gloss.via_place=on".to_string());
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
/// honest `"landed": false` row, the clean world lands the in-line
/// move to (260000, 150000) DBU).
#[test]
fn via_goldens_rederive_byte_identically() {
    for (fixture, blocked) in [("via_world", false), ("via_world_blocked", true)] {
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
            text.contains("\"gloss_via_place\""),
            "{fixture}: block present"
        );
        assert!(text.contains("\"via_id\""), "{fixture}: the via row");
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
                text.contains("\"to_x\": 200000"),
                "blocked world: to == from (the via did not move)"
            );
        } else {
            assert!(
                !text.contains("\"landed\": false"),
                "clean world: the in-line candidate lands"
            );
            assert!(
                text.contains("\"to_x\": 260000"),
                "clean world: landed at the in-line position"
            );
        }
    }
}

/// The DEFAULT face (flag off) on the via world: the sidecar carries
/// NO `gloss_via_place` block (skip-when-empty — the sidecar-rotation
/// law), the SES stays at the input geometry, and the default SES
/// differs from the ON face (the flag is the only gate — an ON run
/// legitimately differs, a default run never glosses).
#[test]
fn via_default_face_rotates_nothing() {
    let dsn = repo_root().join("rust/harness/fixtures/gloss/via_world.dsn");
    let (ses_default, sidecar_default) = run_flow(&dsn, "via-default", false, None);
    let text = String::from_utf8(sidecar_default).expect("sidecar is UTF-8");
    assert!(
        !text.contains("gloss_via_place"),
        "default sidecar carries no gloss_via_place block"
    );
    // Default SES = the input geometry verbatim (the via at board DBU
    // (200000, 150000); the ON face moved it to (260000, 150000)).
    let default_text = String::from_utf8(ses_default.clone()).expect("ses is UTF-8");
    assert!(
        default_text.contains("200000 150000"),
        "input via position preserved"
    );
    let (ses_on, _) = run_flow(&dsn, "via-on", true, None);
    assert_ne!(
        ses_default, ses_on,
        "the gloss flag is the ONLY gate: ON differs, default does not"
    );
}
