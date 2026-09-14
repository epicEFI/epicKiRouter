//! D16 shared-semantics pin (M1b Task 10): the `read_metadata` fast path
//! (`DsnReader.readMetadata`, `DsnReader.java:182-280`) must agree with
//! `read_board` on the metadata fields over the FULL fixture matrix —
//! the tier boards of `rust/harness/config/tiers.yaml` (paths relative to
//! `scripts/benchmark/fixtures`) plus every `*.dsn` under the repo-root
//! `fixtures/` directory (23 + 152 = 175 files at Task 10 time).
//!
//! Java-side agreement (that Java's readMetadata and readBoard produce
//! the same fields on these files) is deferred to Task 11's golden
//! capture — this pin is Rust-side only.
//!
//! Lives in the harness crate (which already depends on epic-dsn) so the
//! reader crate stays filesystem-free.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use epic_dsn::reader::{DsnReadResult, MetadataReadResult, read_board, read_metadata};
use epic_dsn::ses_board::SesBoard;
use epic_dsn::sink::MetadataIr;

#[derive(serde::Deserialize)]
struct TiersFile {
    fixtures_root: String,
    tiers: Vec<Tier>,
}

#[derive(serde::Deserialize)]
struct Tier {
    fixtures: Vec<FixtureEntry>,
}

#[derive(serde::Deserialize)]
struct FixtureEntry {
    path: String,
}

/// `rust/harness` -> `rust` -> repo root (compile-time stable; the
/// harness binary resolves it the same way at runtime).
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("harness dir has a parent")
        .parent()
        .expect("rust dir has a parent")
        .to_path_buf()
}

fn collect_dsn_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .expect("fixtures dir readable")
        .map(|entry| entry.expect("dir entry readable").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect_dsn_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "dsn") {
            out.push(path);
        }
    }
}

/// One field comparison; appends a diagnostic on divergence.
fn diff_field<T: PartialEq + std::fmt::Debug>(
    name: &str,
    full: &T,
    fast: &T,
    problems: &mut Vec<String>,
) {
    if full != fast {
        problems.push(format!(
            "{name}: read_board={full:?} read_metadata={fast:?}"
        ));
    }
}

/// Compares one fixture's two read results.
///
/// Classification mapping: fast-path `Success` must correspond to
/// full-path `Success` OR `OutlineMissing` — readMetadata has NO
/// OutlineMissing arm (it ignores every scope read's return and builds
/// `Success` unconditionally, `DsnReader.java:278-279`). A full-path
/// structure-level `ParseError` ("DSN structure parsing failed") is NOT
/// reproducible on the fast path by construction — such a fixture is a
/// genuine D16 disagreement and must surface.
fn compare(
    full_result: &DsnReadResult,
    full_metadata: &MetadataIr,
    fast_result: &MetadataReadResult,
) -> Option<String> {
    match (full_result, fast_result) {
        (
            DsnReadResult::Success { .. } | DsnReadResult::OutlineMissing { .. },
            MetadataReadResult::Success { metadata, .. },
        ) => {
            let mut problems = Vec::new();
            diff_field("unit", &full_metadata.unit, &metadata.unit, &mut problems);
            diff_field(
                "resolution",
                &full_metadata.resolution,
                &metadata.resolution,
                &mut problems,
            );
            diff_field(
                "string_quote",
                &full_metadata.string_quote,
                &metadata.string_quote,
                &mut problems,
            );
            diff_field(
                "snap_angle",
                &full_metadata.snap_angle,
                &metadata.snap_angle,
                &mut problems,
            );
            diff_field(
                "host_cad",
                &full_metadata.host_cad,
                &metadata.host_cad,
                &mut problems,
            );
            diff_field(
                "host_version",
                &full_metadata.host_version,
                &metadata.host_version,
                &mut problems,
            );
            diff_field(
                "layer_count",
                &full_metadata.layer_count,
                &metadata.layer_count,
                &mut problems,
            );
            diff_field(
                "autoroute_settings",
                &full_metadata.autoroute_settings,
                &metadata.autoroute_settings,
                &mut problems,
            );
            if problems.is_empty() {
                None
            } else {
                Some(problems.join("; "))
            }
        }
        (
            DsnReadResult::ParseError { location, detail },
            MetadataReadResult::ParseError {
                location: fast_location,
                detail: fast_detail,
            },
        ) => {
            if location == fast_location && detail == fast_detail {
                None
            } else {
                Some(format!(
                    "ParseError mismatch: read_board={location}/{detail} read_metadata={fast_location}/{fast_detail}"
                ))
            }
        }
        (DsnReadResult::IoError, MetadataReadResult::IoError) => None,
        (full_arm, fast_arm) => Some(format!(
            "classification mismatch: read_board={full_arm:?} read_metadata={fast_arm:?}"
        )),
    }
}

/// The pin: over every fixture, `read_metadata` must collect exactly the
/// fields `read_board` collects. Measured at 7.2 s debug-mode over the
/// full 175-fixture matrix (Task 10) — fast enough to stay enabled in
/// the default `cargo test --workspace` run (the brief's ignore
/// threshold was 60 s).
#[test]
fn d16_read_metadata_agrees_with_read_board_on_every_fixture() {
    let root = repo_root();
    let tiers: TiersFile = serde_norway::from_str(
        &fs::read_to_string(root.join("rust/harness/config/tiers.yaml"))
            .expect("tiers.yaml readable"),
    )
    .expect("tiers.yaml parses");

    let mut fixture_paths: BTreeSet<PathBuf> = BTreeSet::new();
    let mut tier_count = 0usize;
    for tier in &tiers.tiers {
        for fixture in &tier.fixtures {
            tier_count += 1;
            fixture_paths.insert(root.join(&tiers.fixtures_root).join(&fixture.path));
        }
    }
    let mut walked = Vec::new();
    collect_dsn_files(&root.join("fixtures"), &mut walked);
    let walked_count = walked.len();
    for path in walked {
        fixture_paths.insert(path);
    }

    // 23 tier entries + 152 fixtures/*.dsn at Task 10 time. A floor (not
    // an exact count) keeps the pin from churning when fixtures are
    // added, while still catching a silently broken walk or a truncated
    // tiers.yaml pinning an (almost) empty set.
    assert!(
        fixture_paths.len() >= 170,
        "fixture matrix suspiciously small: {} total ({} tier entries + {} walked from fixtures/)",
        fixture_paths.len(),
        tier_count,
        walked_count
    );

    let mut mismatches: Vec<String> = Vec::new();
    for path in &fixture_paths {
        let bytes = fs::read(path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        let mut full_board = SesBoard::new();
        let full_result = read_board(&bytes, &mut full_board);
        let mut fast_board = SesBoard::new();
        let fast_result = read_metadata(&bytes, &mut fast_board);
        if let Some(problem) = compare(&full_result, &full_board.metadata, &fast_result) {
            mismatches.push(format!("{}: {problem}", path.display()));
        }
    }
    assert!(
        mismatches.is_empty(),
        "D16 pin: {} fixture(s) disagree between read_metadata and read_board (first 10):\n{}",
        mismatches.len(),
        mismatches
            .iter()
            .take(10)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
