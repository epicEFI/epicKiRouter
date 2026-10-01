//! M9-T2: the WORKFLOW PARITY PIN — the task's acceptance proof.
//!
//! `Session::load_dsn(bytes, SessionLayer::default()) →
//! route(&CliLayer::default(), sink) → export_ses(tmp)` produces a
//! file SHA256-IDENTICAL to `epic-cli route` (default argv:
//! `-de <fixture> -do <tmp2>`) on the same fixture — byte-level
//! compare, both files on disk.
//!
//! WHY THE SUBPROCESS FACE (the charter's refinement of the plan's
//! in-process placement): the spawned binary is the true product
//! face — `run_route` in-process would prove the session against the
//! LIBRARY, not against the binary a user runs. The pin therefore
//! lives in the harness (where the spawn machinery and the freshness
//! law's documentation live) and enforces the DNR-17/buglog-184
//! freshness law itself: it resolves the `epic-cli` binary (EPIC_CLI
//! env override, the harness-exe's release sibling, then the repo
//! target dirs), refuses any bin OLDER than the newest workspace
//! source (test-binary freshness is NOT bin freshness — cerebrum DNR
//! 17), and fails LOUDLY with the rebuild instruction when none is
//! fresh. Both profiles are built by the gate lap before this pin is
//! judged.
//!
//! Fixture: DAC2020_bm08 (a Tier A fixture with a committed face;
//! the charter's named choice — probed 2026-09-30: a ~7.5 s default
//! route, COMPLETED in 1 pass, so the pin is cheap AND the export
//! gate is open).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use epic_engine::session::{LoadError, Session};
use epic_engine::settings::{CliLayer, SessionLayer};
use epic_router::pipeline::event_sink::DriverSink;
use sha2::Digest;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("harness dir has a parent")
        .parent()
        .expect("rust dir has a parent")
        .to_path_buf()
}

/// The newest mtime over the workspace-crate sources the spawned
/// `epic-cli` bin links — `rust/crates/**/*.rs`, each crate's
/// `Cargo.toml`, and `rust/Cargo.lock` (the router_compare.rs
/// `newest_workspace_source` walk, inlined: that helper is
/// bin-internal and integration tests cannot import it).
fn newest_workspace_source(repo_root: &Path) -> Option<SystemTime> {
    let crates_dir = repo_root.join("rust/crates");
    let mut newest: Option<SystemTime> = None;
    let mut stack = vec![crates_dir.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let is_source = path.extension().is_some_and(|ext| ext == "rs")
                || path.file_name().is_some_and(|name| name == "Cargo.toml");
            if is_source
                && let Ok(meta) = fs::metadata(&path)
                && let Ok(mtime) = meta.modified()
                && newest.is_none_or(|current| mtime > current)
            {
                newest = Some(mtime);
            }
        }
    }
    if let Ok(meta) = fs::metadata(repo_root.join("rust/Cargo.lock"))
        && let Ok(mtime) = meta.modified()
        && newest.is_none_or(|current| mtime > current)
    {
        newest = Some(mtime);
    }
    newest
}

/// Resolve a FRESH `epic-cli` binary: `EPIC_CLI` env, the harness
/// test binary's release sibling, `target/debug/epic-cli`,
/// `target/release/epic-cli` — in that order, adopting the FIRST
/// bin whose mtime is not older than the newest workspace source.
/// In the `cargo test` context the just-built DEBUG bin is the
/// freshest truth (cargo builds bin targets before running tests),
/// so the order deliberately prefers it over a possibly stale
/// release face — a documented inversion of router_compare's
/// prefer-release spawn gates.
fn fresh_epic_cli(repo_root: &Path) -> PathBuf {
    let newest = newest_workspace_source(repo_root);
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(path) = std::env::var("EPIC_CLI") {
        candidates.push(PathBuf::from(path));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        candidates.push(parent.join("epic-cli"));
    }
    for profile in ["debug", "release"] {
        candidates.push(repo_root.join("rust/target").join(profile).join("epic-cli"));
    }
    for candidate in candidates {
        if !candidate.is_file() {
            continue;
        }
        let fresh = fs::metadata(&candidate)
            .ok()
            .and_then(|meta| meta.modified().ok())
            .is_some_and(|bin_mtime| newest.is_none_or(|src_mtime| bin_mtime >= src_mtime));
        if fresh {
            return candidate;
        }
    }
    panic!(
        "no fresh epic-cli binary found — the workflow parity pin judges the SPAWNED product \
         face, and a stale bin judges pre-edit code (buglog-184 family). Run `cargo build -p \
         epic-cli` and `cargo build -p epic-cli --release` from rust/ first, or point EPIC_CLI \
         at a freshly built bin."
    );
}

/// A tiny recorder sink (the session rows are not under pin here; the
/// CLI's stderr rows are captured instead).
#[derive(Default)]
struct SilentSink;

impl DriverSink for SilentSink {}

/// THE pin: session export bytes == spawned-CLI output bytes on the
/// same fixture at default settings.
#[test]
fn session_export_is_byte_identical_to_the_cli() {
    let root = repo_root();
    let dsn = root.join("scripts/benchmark/fixtures/DAC2020_boards/DAC2020_bm08.dsn");
    let dsn = fs::canonicalize(dsn).expect("the bm08 fixture exists");
    let bytes = fs::read(&dsn).expect("the bm08 fixture reads");

    // --- the session path (in-process) ---
    let mut session = match Session::load_dsn(&bytes, SessionLayer::default()) {
        Ok(session) => session,
        Err(LoadError::OutlineMissing(session)) => *session,
        Err(LoadError::Parse(detail)) => panic!("bm08 must parse: {detail}"),
        Err(LoadError::Io(detail)) => panic!("bm08 must not hit the Io arm: {detail}"),
    };
    // The SES design face comes from the INPUT name (the CLI reads it
    // off the -de path; the session host sets it — see
    // Session::set_input_name).
    let input_name = dsn
        .file_name()
        .and_then(|name| name.to_str())
        .expect("the fixture path has a file name")
        .to_string();
    session.set_input_name(input_name);
    let mut sink = SilentSink;
    let summary = session
        .route(&CliLayer::default(), &mut sink)
        .expect("route succeeds");
    assert_eq!(
        summary.final_state, "COMPLETED",
        "bm08 must complete at defaults for the parity pin (passes {}, incomplete {})",
        summary.passes, summary.incomplete_count
    );
    let scratch = std::env::temp_dir().join(format!("session_parity_pin_{}", std::process::id()));
    fs::create_dir_all(&scratch).expect("the scratch dir creates");
    let session_out = scratch.join("session.ses");
    session.export_ses(&session_out).expect("the export writes");

    // --- the CLI path (the spawned product face) ---
    let cli_bin = fresh_epic_cli(&root);
    let cli_out = scratch.join("cli.ses");
    let output = Command::new(&cli_bin)
        .arg("route")
        .arg("-de")
        .arg(&dsn)
        .arg("-do")
        .arg(&cli_out)
        .output()
        .expect("the epic-cli subprocess spawns");
    assert!(
        output.status.success(),
        "epic-cli route must exit 0 (stderr: {})",
        String::from_utf8_lossy(&output.stderr)
    );

    // --- the byte-level compare (both files on disk) ---
    let session_bytes = fs::read(&session_out).expect("the session export reads back");
    let cli_bytes = fs::read(&cli_out).expect("the CLI output reads back");
    let mut session_hasher = sha2::Sha256::new();
    session_hasher.update(&session_bytes);
    let mut cli_hasher = sha2::Sha256::new();
    cli_hasher.update(&cli_bytes);
    let session_digest = format!("{:x}", session_hasher.finalize());
    let cli_digest = format!("{:x}", cli_hasher.finalize());
    assert_eq!(
        session_bytes, cli_bytes,
        "the session export must be byte-identical to the CLI output \
         (session sha256 {session_digest}, cli sha256 {cli_digest})"
    );
    assert_ne!(session_bytes.len(), 0, "neither file is empty");
    let _ = fs::remove_dir_all(&scratch);
}
