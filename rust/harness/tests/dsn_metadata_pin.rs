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
//! Warnings are deliberately NOT compared: the fast path legitimately
//! stops before the wiring scope, where the bulk of parity warnings
//! accumulate, so `read_board` warnings ≠ `read_metadata` warnings on
//! 15/175 fixtures (measured by the spec-review probe, 2026-09-14).
//!
//! Known unit divergence: Java's `Resolution.java:39-40` assigns
//! `scopeParameter.unit` BEFORE the null check, so an unrecognized unit
//! overwrites the previous value with null in Java's metadata path; the
//! Rust port keeps the previous value (`Unit` is non-nullable,
//! unreachable through `read_board` — see `scope/resolution.rs` module
//! docs).
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
        (DsnReadResult::Success { .. }, MetadataReadResult::Success { metadata, .. }) => {
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
        (DsnReadResult::OutlineMissing { .. }, MetadataReadResult::Success { .. }) => {
            // Classification-only compat arm — NO field diff. The sink
            // metadata snapshot is populated only under `read_ok`
            // (reader.rs:181-199), so a full-path OutlineMissing leaves
            // the full side at `MetadataIr::default()` while the fast
            // side returns real parse-state values; diffing fields here
            // would report false "genuine D16 disagreements" on the
            // first outline-less fixture. The classification itself is
            // legal (the arm's doc on `compare` above).
            None
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

    // 23 tier entries + 152 fixtures/*.dsn at Task 10 time = 175, the
    // digest set. The EXPECTED count is derived from the committed
    // manifest (`rust/harness/corpus/dsn-manifest.jsonl`, the same
    // doc-of-record the corpus pins anchor) instead of a hardcoded
    // number, so the pin never churns when fixtures are added — yet is
    // EXACT: a floor (the old `>= 170`) could not catch a silently
    // broken walk or a truncated tiers.yaml shrinking the set fixture
    // by fixture. If the manifest and this walk disagree, either the
    // walk regressed or the manifest needs regenerating (`epic-harness
    // dsn manifest`).
    let manifest = fs::read_to_string(root.join("rust/harness/corpus/dsn-manifest.jsonl"))
        .expect("committed dsn manifest readable");
    let manifest_dsn_count = manifest
        .lines()
        .filter(|line| line.starts_with("{\"id\":\"dsn-"))
        .count();
    assert_eq!(
        fixture_paths.len(),
        manifest_dsn_count,
        "fixture matrix must equal the manifest's dsn- digest set ({} tier entries + {} walked from fixtures/)",
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

/// Self-tests for the pin machinery above (pure functions, no I/O,
/// microseconds): they fabricate result pairs and call the SAME
/// [`compare`]/[`diff_field`] functions the 175-fixture pin uses, so a
/// regressed compare (a copy-paste field swap — `resolution` and
/// `layer_count` are both i32 — or a metadata field added without a
/// `diff_field` call) fails here even though all 174 real
/// Success/Success pairs would stay silently green.
#[cfg(test)]
mod self_test {
    use super::*;
    use epic_dsn::reader::BoardMetadataIr;
    use epic_dsn::scope::autoroute_settings::AutorouteSettingsIr;
    use epic_dsn::state::{AngleRestriction, Unit};

    /// A valid fast-side snapshot; values are irrelevant except where a
    /// test overrides them.
    fn fast_metadata() -> BoardMetadataIr {
        BoardMetadataIr {
            unit: Unit::Mil,
            resolution: 100,
            string_quote: "\"".to_string(),
            snap_angle: AngleRestriction::None,
            host_cad: None,
            host_version: None,
            layer_count: 0,
            autoroute_settings: None,
        }
    }

    /// Fully divergent pair: ALL 8 compared fields differ, with the two
    /// i32 fields (`resolution`/`layer_count`) and the strings taking
    /// values that would expose a copy-paste swap between the i32 pair
    /// (a swapped compare still names every field, but transposes the
    /// values). `compare` must name all 8 fields with the right values —
    /// this is the only guard against a regressed or incomplete
    /// `diff_field` set, which stays silent on 174 matching fixtures.
    #[test]
    fn fully_divergent_pair_names_all_eight_fields() {
        let full = MetadataIr {
            unit: Unit::Mil,
            resolution: 100,
            string_quote: "\"".to_string(),
            snap_angle: AngleRestriction::None,
            flip_style: None,
            host_cad: Some("full host".to_string()),
            host_version: Some("v1".to_string()),
            layer_count: 2,
            autoroute_settings: None,
        };
        let mut fast = fast_metadata();
        fast.unit = Unit::Inch;
        fast.resolution = 1000;
        fast.string_quote = "'".to_string();
        fast.snap_angle = AngleRestriction::NinetyDegree;
        fast.host_cad = Some("fast host".to_string());
        fast.host_version = Some("v2".to_string());
        fast.layer_count = 4;
        fast.autoroute_settings = Some(AutorouteSettingsIr::new(2));

        let problem = compare(
            &DsnReadResult::Success {
                warnings: Vec::new(),
            },
            &full,
            &MetadataReadResult::Success {
                metadata: fast,
                warnings: Vec::new(),
            },
        )
        .expect("fully divergent pair must report problems");

        for name in [
            "unit",
            "resolution",
            "string_quote",
            "snap_angle",
            "host_cad",
            "host_version",
            "layer_count",
            "autoroute_settings",
        ] {
            assert!(
                problem.contains(&format!("{name}: read_board=")),
                "problem report must name '{name}': {problem}"
            );
        }
        // Value checks: a swapped/aliased i32 compare would still name
        // every field but with transposed values.
        assert!(
            problem.contains("resolution: read_board=100 read_metadata=1000"),
            "resolution values wrong or swapped: {problem}"
        );
        assert!(
            problem.contains("layer_count: read_board=2 read_metadata=4"),
            "layer_count values wrong or swapped: {problem}"
        );
        assert!(
            problem.contains("unit: read_board=Mil read_metadata=Inch"),
            "unit values wrong: {problem}"
        );
    }

    /// The classification-mismatch fallback arm: a fast `Success` paired
    /// with any non-paired full classification must surface. Also pins
    /// the `OutlineMissing ∥ fast Success` compat arm: classification
    /// only, `None` (see the arm's doc on `compare`).
    #[test]
    fn classification_mismatch_surfaces() {
        let full_metadata = MetadataIr::default();
        let fast = MetadataReadResult::Success {
            metadata: fast_metadata(),
            warnings: Vec::new(),
        };
        for full_arm in [
            DsnReadResult::ParseError {
                location: "(pcb".to_string(),
                detail: "header".to_string(),
            },
            DsnReadResult::IoError,
        ] {
            let problem = compare(&full_arm, &full_metadata, &fast)
                .expect("non-paired classification must surface");
            assert!(
                problem.contains("classification mismatch"),
                "unexpected problem text: {problem}"
            );
        }
        // The compat arm itself: legal pair, classification only.
        assert_eq!(
            compare(
                &DsnReadResult::OutlineMissing {
                    warnings: Vec::new()
                },
                &full_metadata,
                &fast
            ),
            None
        );
    }

    /// The ParseError arm must compare BOTH location and detail: a
    /// same-location/different-detail pair (and a location mismatch)
    /// surfaces, an identical pair passes.
    #[test]
    fn parse_error_location_and_detail_mismatches_surface() {
        let full_metadata = MetadataIr::default();
        let full_err = |detail: &str| DsnReadResult::ParseError {
            location: "(pcb".to_string(),
            detail: detail.to_string(),
        };
        // Identical pair: the contract real failing fixtures rely on.
        assert_eq!(
            compare(
                &full_err("DSN structure parsing failed"),
                &full_metadata,
                &MetadataReadResult::ParseError {
                    location: "(pcb".to_string(),
                    detail: "DSN structure parsing failed".to_string(),
                },
            ),
            None
        );
        // Detail mismatch at the same location.
        let problem = compare(
            &full_err("full detail"),
            &full_metadata,
            &MetadataReadResult::ParseError {
                location: "(pcb".to_string(),
                detail: "fast detail".to_string(),
            },
        )
        .expect("detail mismatch must surface");
        assert!(problem.contains("ParseError mismatch"), "{problem}");
        // Location mismatch.
        let problem = compare(
            &full_err("detail"),
            &full_metadata,
            &MetadataReadResult::ParseError {
                location: "(resolution".to_string(),
                detail: "detail".to_string(),
            },
        )
        .expect("location mismatch must surface");
        assert!(problem.contains("ParseError mismatch"), "{problem}");
    }
}
