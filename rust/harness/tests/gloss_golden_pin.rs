//! M8-T3: the gloss ON-face goldens re-derive byte-identically through
//! the FULL in-process product face (`epic_cli::route::run_route` —
//! parse → routing stage → gloss stage → SES/manifest/sidecar write).
//! The committed fixtures (`fixtures/gloss/*.dsn`) are fully prerouted
//! 3-net bus worlds; the ON face is `--router.gloss.bus=on` with the
//! optimizer DISABLED (the crafted 90° geometry is outside the T4
//! 45° scope — an enabled optimizer both panics there and would
//! reshape the crafted ladder; the capture face records this).
//!
//! Pins here: golden byte-identity of SES + sidecar for both worlds,
//! determinism ×2, and the `-mt 1`/`-mt 3` threads-invariance witness
//! (the M5-T7 contract — the gloss pass is sequential, so thread count
//! must not leak into output bytes).
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
    let dir = std::env::temp_dir().join(format!("epic-m8t3-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("scratch dir created");
    dir.join(name)
}

/// Runs the full product flow over a gloss fixture. `gloss_on` — the
/// `router.gloss.bus` flag; `max_threads` — the explicit `-mt` face.
fn run_flow(dsn: &Path, tag: &str, gloss_on: bool, max_threads: Option<i32>) -> (Vec<u8>, Vec<u8>) {
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
    if gloss_on {
        argv.push("--router.gloss.bus=on".to_string());
    }
    // The argv IS the production path (quality-r1 NIT: no manual
    // layer override beside it — the parsed flag rides parse ->
    // merge -> resolve -> build_batch_settings unaided).
    let args: ParsedRouteArgs = parse_route_args(&argv).expect("argv parses");
    let exit = run_route(&args).expect("run_route succeeds");
    assert_eq!(exit, 0, "the fixture completes cleanly");
    (
        fs::read(&ses).expect("ses written"),
        fs::read(&sidecar).expect("sidecar written"),
    )
}

/// Both ON faces re-derive byte-identically; determinism ×2 holds;
/// `-mt 1` and `-mt 3` outputs are byte-identical.
#[test]
fn gloss_goldens_rederive_byte_identically() {
    for (fixture, blocked) in [("bus_world", false), ("bus_world_blocked", true)] {
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

        // The ON-face semantic pins on the sidecar bytes: the blocked
        // world must carry the honest rejected row, the clean world
        // must land both movers.
        let text = String::from_utf8(sidecar1).expect("sidecar is UTF-8");
        assert!(text.contains("\"bus_groups\""), "{fixture}: block present");
        if blocked {
            assert!(
                text.contains("\"landed\": null"),
                "blocked world: net 3's honest stop is recorded"
            );
        } else {
            assert!(
                !text.contains("\"landed\": null"),
                "clean world: every move lands"
            );
        }
    }
}

/// The DEFAULT face (flag off) on the bus world: the sidecar carries
/// NO `bus_groups` block (skip-when-empty — the sidecar-rotation law),
/// the SES stays at the input geometry, and the default SES differs
/// from the ON face (the flag is the only gate — an ON run legitimately
/// differs, a default run never glosses).
#[test]
fn gloss_default_face_rotates_nothing() {
    let dsn = repo_root().join("rust/harness/fixtures/gloss/bus_world.dsn");
    let (ses_default, sidecar_default) = run_flow(&dsn, "bus-default", false, None);
    let text = String::from_utf8(sidecar_default).expect("sidecar is UTF-8");
    assert!(
        !text.contains("bus_groups"),
        "default sidecar carries no bus_groups block"
    );
    // Default SES = the input geometry verbatim (net 3 span at
    // 185_000, board DBU); the ON face moved it to 182_500.
    let default_text = String::from_utf8(ses_default.clone()).expect("ses is UTF-8");
    assert!(default_text.contains("185000"), "input span preserved");
    let (ses_on, _) = run_flow(&dsn, "bus-on", true, None);
    assert_ne!(
        ses_default, ses_on,
        "the gloss flag is the ONLY gate: ON differs, default does not"
    );
}
