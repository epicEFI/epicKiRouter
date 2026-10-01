//! Distills oracle runs into committed golden baselines and compares runs
//! against them (design §5: counts/score/state are gated; wall time is a
//! tracked trend, never a gate; the SES hash is identity info only —
//! semantic SES comparison arrives with epic-dsn in M1).

use crate::manifest::RoutingResultManifest;
use crate::oracle::OracleRun;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

pub const BASELINE_SCHEMA_VERSION: u32 = 1;

// Unlike manifest.rs (which must tolerate Gson's extra fields), the baseline
// format is written AND read by this harness alone — a strict reader makes
// format drift fail loudly instead of silently defaulting.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BaselineRecord {
    pub schema_version: u32,
    /// "java" in M0; "rust" records appear from M3.
    pub engine: String,
    pub app_version: Option<String>,
    pub git_sha: Option<String>,
    /// Repo-relative fixture path (e.g. `scripts/benchmark/fixtures/.../x.dsn`).
    pub fixture: String,
    pub fixture_sha256: Option<String>,
    pub final_state: String,
    pub exit_code: Option<i32>,
    pub incomplete_count: Option<i64>,
    pub maximum_count: Option<i64>,
    pub clearance_violations_total: Option<i64>,
    pub clearance_router_introduced: Option<i64>,
    pub normalized_score: Option<f64>,
    pub optimizer_score: Option<f64>,
    pub ses_sha256: Option<String>,
    pub autorouter_seconds: Option<f64>,
    pub optimizer_seconds: Option<f64>,
    pub passes_completed: Option<i64>,
    /// The capture profile marker (oracle::OracleProfile::marker): None =
    /// full-flow (and every pre-T14 record), Some("router-only") = captured
    /// with fanout + optimizer disabled. In code, not by path alone —
    /// compare() gates it so an unthreaded verify (a full-flow re-run
    /// against router-only records, the trap-3 mismatch) fails loudly.
    /// Option: absent parses as None, so pre-T14 records stay readable.
    pub profile: Option<String>,
    pub captured_at_unix: i64,
    pub notes: Option<String>,
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let bytes =
        std::fs::read(path).with_context(|| format!("reading {} for hashing", path.display()))?;
    let digest = Sha256::digest(&bytes);
    Ok(format!("{digest:x}"))
}

/// The manifest digest is VERSION-BLIND (the M10-T5 sanctioned handoff:
/// the raw-bytes manifest canary `cf607714…` retired at the M10-T5
/// commit A′; its successor is the normalized literal measured there).
/// The two ambient build-metadata string fields — `app_version` and
/// `git_sha` (the unset-by-default `FREEROUTING_GIT_SHA` ambient) — are
/// replaced with a fixed placeholder BEFORE hashing, so a release bump
/// can never rotate a committed manifest digest. The manifest ARTIFACT
/// keeps its true values (`route.rs` is untouched); only this digest
/// face is scoped. The replace is byte-level and surgical: the CLI
/// manifest is single-line canonical JSON, each field appears exactly
/// once. The scan stops at the next `"` byte — exact for today's
/// values (semver app_version; hex or `"unknown"` git_sha), and for a
/// hypothetical `FREEROUTING_GIT_SHA` containing an escaped quote the
/// value splices SHORT: the digest changes (loud, pinned), it cannot
/// crash — an honest residual, not a claim that a quote is impossible.
///
/// Both digest faces route through this helper: the determinism pair
/// face (`router_compare::determinism_digests`) and the global-golden
/// capture/verify face (`global_golden::run_face`).
pub fn normalized_manifest_sha256(path: &Path) -> Result<String> {
    let bytes =
        std::fs::read(path).with_context(|| format!("reading {} for hashing", path.display()))?;
    let digest = Sha256::digest(normalize_manifest_bytes(&bytes)?);
    Ok(format!("{digest:x}"))
}

/// Replaces the `"app_version":"<any>"` / `"git_sha":"<any>"` values
/// with `"-"`. BOTH fields are unconditional in the CLI's renderer
/// (`route.rs` writes them on every manifest), so a manifest missing
/// either is MALFORMED input for this face and bails loudly — the
/// strict-reader philosophy this module states at its top; a silent
/// raw passthrough would quietly re-arm the version-rotation face the
/// handoff closed.
fn normalize_manifest_bytes(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut out = bytes.to_vec();
    for field in ["app_version", "git_sha"] {
        let marker = format!("\"{field}\":\"");
        let Some(start) = find_subslice(&out, marker.as_bytes()) else {
            bail!("malformed manifest: missing canonical \"{field}\" field");
        };
        let value_start = start + marker.len();
        let Some(rel) = out[value_start..].iter().position(|&b| b == b'"') else {
            bail!("malformed manifest: unterminated {field} string value");
        };
        out.splice(value_start..value_start + rel, std::iter::once(b'-'));
    }
    Ok(out)
}

/// The first occurrence of `needle` in `haystack` (plain byte search —
/// a manifest is ~1 KB, a memchr-grade scan is unwarranted).
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (0..=haystack.len() - needle.len()).find(|&i| &haystack[i..i + needle.len()] == needle)
}

/// Reduces one oracle run to the committed baseline record. `profile_marker`
/// is `oracle::OracleProfile::marker()` for the run — None for full-flow
/// (the capture/verify default), Some("router-only") for the T14 profile.
pub fn distill_profiled(
    profile_marker: Option<&str>,
    fixture_rel: &str,
    run: &OracleRun,
) -> BaselineRecord {
    let (manifest, mut note): (&RoutingResultManifest, Option<String>) = match &run.manifest {
        Some(m) => (m, None),
        None => (
            // A missing manifest is itself a recordable outcome (harness
            // timeout, JVM crash). Synthesize the shell of a manifest; the
            // note distinguishes absent from present-but-unparseable
            // (oracle.rs `manifest_error`: schema drift or a truncated
            // mid-kill write must not read as "never produced").
            &UNPARSED,
            Some(match &run.manifest_error {
                Some(err) => format!(
                    "manifest unparseable: {err} (timed_out={}, exit_code={:?})",
                    run.timed_out, run.exit_code
                ),
                None => format!(
                    "no manifest produced (timed_out={}, exit_code={:?})",
                    run.timed_out, run.exit_code
                ),
            }),
        ),
    };
    let stats = manifest.board_statistics.as_ref();
    let ses_sha256 = if run.ses_path.is_file() {
        match sha256_file(&run.ses_path) {
            Ok(hash) => Some(hash),
            // An unreadable ses file must not silently read as "no ses
            // produced"; fold the failure into the note instead.
            Err(err) => {
                let msg = format!("ses hash unavailable: {err:#}");
                note = match note {
                    Some(existing) => Some(format!("{existing}; {msg}")),
                    None => Some(msg),
                };
                None
            }
        }
    } else {
        None
    };
    BaselineRecord {
        schema_version: BASELINE_SCHEMA_VERSION,
        engine: "java".into(),
        app_version: Some(manifest.app_version.clone()),
        git_sha: Some(manifest.git_sha.clone()),
        fixture: fixture_rel.into(),
        fixture_sha256: manifest.fixture.sha256.clone(),
        final_state: if run.timed_out {
            "HARNESS_TIMED_OUT".into()
        } else {
            manifest.final_state.clone()
        },
        exit_code: run.exit_code,
        incomplete_count: stats
            .and_then(|s| s.connections.as_ref())
            .and_then(|c| c.incomplete_count),
        maximum_count: stats
            .and_then(|s| s.connections.as_ref())
            .and_then(|c| c.maximum_count),
        clearance_violations_total: stats
            .and_then(|s| s.clearance_violations.as_ref())
            .and_then(|c| c.total_count),
        clearance_router_introduced: stats
            .and_then(|s| s.clearance_violations.as_ref())
            .and_then(|c| c.router_introduced_count),
        normalized_score: manifest.normalized_score,
        optimizer_score: manifest.optimizer_score,
        ses_sha256,
        autorouter_seconds: manifest.phases.autorouter.duration_seconds,
        optimizer_seconds: manifest.phases.optimizer.duration_seconds,
        passes_completed: manifest.phases.autorouter.passes_completed,
        profile: profile_marker.map(str::to_string),
        captured_at_unix: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
        notes: note,
    }
}

/// A single gate failure. Wall time and SES hash are deliberately absent —
/// time is a trend metric and SES is compared semantically from M1.
#[derive(Debug, PartialEq)]
pub enum GateFailure {
    FinalState {
        expected: String,
        actual: String,
    },
    ExitCode {
        expected: Option<i32>,
        actual: Option<i32>,
    },
    IncompleteCount {
        expected: Option<i64>,
        actual: Option<i64>,
    },
    ClearanceViolations {
        expected: Option<i64>,
        actual: Option<i64>,
    },
    NormalizedScore {
        expected: Option<f64>,
        actual: Option<f64>,
    },
    /// The capture profiles differ (e.g. a full-flow verify run compared
    /// against router-only records — the trap-3 silent mismatch).
    Profile {
        expected: Option<String>,
        actual: Option<String>,
    },
}

/// Compares a fresh run against a committed baseline. Counts/state/exit are
/// exact; normalized_score uses a relative epsilon (same engine, same seed
/// should be bit-identical, but JSON round-trips make a tiny epsilon the
/// honest bound).
pub fn compare(expected: &BaselineRecord, actual: &BaselineRecord) -> Vec<GateFailure> {
    let mut failures = Vec::new();
    if expected.profile != actual.profile {
        failures.push(GateFailure::Profile {
            expected: expected.profile.clone(),
            actual: actual.profile.clone(),
        });
    }
    if expected.final_state != actual.final_state {
        failures.push(GateFailure::FinalState {
            expected: expected.final_state.clone(),
            actual: actual.final_state.clone(),
        });
    }
    if expected.exit_code != actual.exit_code {
        failures.push(GateFailure::ExitCode {
            expected: expected.exit_code,
            actual: actual.exit_code,
        });
    }
    if expected.incomplete_count != actual.incomplete_count {
        failures.push(GateFailure::IncompleteCount {
            expected: expected.incomplete_count,
            actual: actual.incomplete_count,
        });
    }
    if expected.clearance_violations_total != actual.clearance_violations_total {
        failures.push(GateFailure::ClearanceViolations {
            expected: expected.clearance_violations_total,
            actual: actual.clearance_violations_total,
        });
    }
    if let (Some(e), Some(a)) = (expected.normalized_score, actual.normalized_score) {
        // serde_json cannot produce non-finite f64s (it rejects NaN/Infinity literals and errors serializing them), so the epsilon path only ever sees finite values.
        let epsilon = 1e-6 * e.abs().max(1.0);
        let diff = (e - a).abs();
        if diff > epsilon {
            failures.push(GateFailure::NormalizedScore {
                expected: Some(e),
                actual: Some(a),
            });
        }
    } else if expected.normalized_score.is_some() != actual.normalized_score.is_some() {
        // Some-vs-None: report the true absence instead of a fake 0.0.
        failures.push(GateFailure::NormalizedScore {
            expected: expected.normalized_score,
            actual: actual.normalized_score,
        });
    }
    failures
}

static UNPARSED: RoutingResultManifest = RoutingResultManifest {
    schema_version: 0,
    app_version: String::new(),
    git_sha: String::new(),
    fixture: crate::manifest::FixtureInfo {
        filename: None,
        sha256: None,
    },
    phases: crate::manifest::PhaseMetrics {
        fanout: crate::manifest::PhaseDetail {
            duration_seconds: None,
            passes_completed: None,
        },
        autorouter: crate::manifest::PhaseDetail {
            duration_seconds: None,
            passes_completed: None,
        },
        optimizer: crate::manifest::PhaseDetail {
            duration_seconds: None,
            passes_completed: None,
        },
    },
    board_statistics: None,
    normalized_score: None,
    optimizer_score: None,
    final_state: String::new(),
    exit_code: -1,
    output_written: false,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_manifest_json() -> String {
        r#"{
          "schema_version": 1,
          "app_version": "2.5.0",
          "git_sha": "e7f9bdf1",
          "fixture": { "filename": "x.dsn", "sha256": "deadbeef" },
          "phases": {
            "fanout": {},
            "autorouter": { "duration_seconds": 10.0, "passes_completed": 20 },
            "optimizer": { "duration_seconds": 5.0, "passes_completed": 3 }
          },
          "board_statistics": {
            "connections": { "incomplete_count": 2, "maximum_count": 50 },
            "clearance_violations": { "total_count": 2, "router_introduced_count": 1 }
          },
          "normalized_score": 990.0,
          "optimizer_score": 980.0,
          "final_state": "COMPLETED",
          "exit_code": 0,
          "output_written": true
        }"#
        .into()
    }

    fn synthetic_run(manifest: Option<RoutingResultManifest>, timed_out: bool) -> OracleRun {
        let dir = std::env::temp_dir().join("epic-harness-baseline-test");
        std::fs::create_dir_all(&dir).expect("temp dir");
        OracleRun {
            manifest,
            manifest_error: None,
            exit_code: if timed_out { None } else { Some(0) },
            timed_out,
            wall_seconds: 42.0,
            ses_path: dir.join("nonexistent.ses"),
            manifest_path: dir.join("manifest.json"),
        }
    }

    #[test]
    fn distills_counts_scores_and_phases() {
        let m: RoutingResultManifest =
            serde_json::from_str(&sample_manifest_json()).expect("manifest parses");
        let run = synthetic_run(Some(m), false);
        let b = distill_profiled(None, "fixtures/x.dsn", &run);
        assert_eq!(b.schema_version, 1);
        assert_eq!(b.engine, "java");
        assert_eq!(b.final_state, "COMPLETED");
        assert_eq!(b.incomplete_count, Some(2));
        assert_eq!(b.maximum_count, Some(50));
        // Non-degenerate world (total=2, introduced=1): an extraction swap
        // (introduced <- total, mutant Q2) cannot hide behind equal values.
        assert_eq!(b.clearance_violations_total, Some(2));
        assert_eq!(b.clearance_router_introduced, Some(1));
        assert_eq!(b.normalized_score, Some(990.0));
        assert_eq!(b.autorouter_seconds, Some(10.0));
        // Never-asserted before the quality review (mutant Q2b survived the
        // whole suite): the optimizer-phase duration extraction.
        assert_eq!(b.optimizer_seconds, Some(5.0));
        assert_eq!(b.passes_completed, Some(20));
        assert_eq!(b.ses_sha256, None); // ses file does not exist in test
        assert_eq!(b.profile, None); // full-flow default
    }

    /// The profile marker must land on the record BOTH ways — a router-only
    /// capture that lost its marker would be indistinguishable from a
    /// full-flow record in code (and would then verify against full-flow
    /// runs).
    #[test]
    fn distill_stamps_and_round_trips_the_profile_marker() {
        let m: RoutingResultManifest =
            serde_json::from_str(&sample_manifest_json()).expect("manifest parses");

        let router_only = distill_profiled(
            Some("router-only"),
            "fixtures/x.dsn",
            &synthetic_run(Some(m.clone()), false),
        );
        assert_eq!(router_only.profile, Some("router-only".into()));
        // The marker survives the strict JSON round trip.
        let json = serde_json::to_string(&router_only).expect("serialize");
        let back: BaselineRecord = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.profile, Some("router-only".into()));

        // A pre-T14 record (no profile key) still parses — schema-safe.
        let legacy: BaselineRecord = serde_json::from_str(
            r#"{"schema_version":1,"engine":"java","fixture":"x.dsn","final_state":"COMPLETED","captured_at_unix":0}"#,
        )
        .expect("legacy record must parse");
        assert_eq!(legacy.profile, None);
    }

    /// ses_sha256 must digest the bytes AT ses_path — not some other file —
    /// with a contrast witness (different bytes → different digest).
    #[test]
    fn distill_hashes_the_ses_bytes_present_at_ses_path() {
        let dir = std::env::temp_dir().join("epic-harness-ses-hash-test");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let ses = dir.join("out.ses");
        std::fs::write(&ses, b"(session \"router-only\")").expect("write ses");
        let mut run = synthetic_run(None, false);
        run.ses_path = ses.clone();

        let record = distill_profiled(Some("router-only"), "fixtures/x.dsn", &run);
        let expected = sha256_file(&ses).expect("hash ses");
        assert_eq!(record.ses_sha256, Some(expected.clone()));
        // Contrast: other bytes at the same path would hash differently —
        // the pin discriminates wrong-byte mutants.
        std::fs::write(&ses, b"(session \"full-flow and different\")").expect("rewrite ses");
        let other = sha256_file(&ses).expect("hash other ses");
        assert_ne!(expected, other);
        let record2 = distill_profiled(Some("router-only"), "fixtures/x.dsn", &run);
        assert_eq!(record2.ses_sha256, Some(other));
        assert_ne!(record.ses_sha256, record2.ses_sha256);
        let _ = std::fs::remove_file(&ses);
    }

    #[test]
    fn records_harness_timeout_honestly() {
        let run = synthetic_run(None, true);
        let b = distill_profiled(None, "fixtures/x.dsn", &run);
        assert_eq!(b.final_state, "HARNESS_TIMED_OUT");
        assert!(
            b.notes
                .as_deref()
                .expect("note present")
                .contains("timed_out=true")
        );
        assert_eq!(b.incomplete_count, None);
    }

    #[test]
    fn records_manifest_parse_failure_honestly() {
        let mut run = synthetic_run(None, false);
        run.manifest_error = Some("expected value at line 1 column 1".into());
        let b = distill_profiled(None, "fixtures/x.dsn", &run);
        let note = b.notes.as_deref().expect("note present");
        assert!(note.contains("manifest unparseable"));
        assert!(note.contains("expected value"));
        assert!(!note.contains("no manifest produced"));
    }

    #[test]
    fn baseline_record_round_trips_through_json() {
        let m: RoutingResultManifest =
            serde_json::from_str(&sample_manifest_json()).expect("manifest parses");
        let run = synthetic_run(Some(m), false);
        let mut record = distill_profiled(None, "fixtures/x.dsn", &run);
        // Make the round trip non-trivial: populate a field distill left
        // None-ish and clear another, so Some/None survive JSON equally.
        record.notes = Some("captured on CI runner".into());
        record.optimizer_score = None;
        let json = serde_json::to_string(&record).expect("serialize");
        let back: BaselineRecord = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(record, back);
    }

    /// Contrast with manifest.rs's `parses_manifest_ignoring_unknown_fields`:
    /// the harness both writes and reads baselines, so `deny_unknown_fields`
    /// makes format drift fail loudly — one extra top-level field must not
    /// parse.
    #[test]
    fn parses_baseline_rejecting_unknown_fields() {
        let m: RoutingResultManifest =
            serde_json::from_str(&sample_manifest_json()).expect("manifest parses");
        let run = synthetic_run(Some(m), false);
        let record = distill_profiled(None, "fixtures/x.dsn", &run);
        let mut value = serde_json::to_value(&record).expect("serialize");
        value
            .as_object_mut()
            .expect("record serializes to an object")
            .insert("surprise".into(), serde_json::json!(1));
        let raw = serde_json::to_string(&value).expect("reserialize");
        let err = serde_json::from_str::<BaselineRecord>(&raw)
            .expect_err("unknown top-level field must fail the strict reader");
        assert!(
            err.to_string().contains("unknown field"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn sha256_of_known_bytes_is_stable() {
        let dir = std::env::temp_dir().join("epic-harness-baseline-test");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let p = dir.join("known.txt");
        std::fs::write(&p, b"epic").expect("write");
        let first = sha256_file(&p).expect("hash");
        let second = sha256_file(&p).expect("hash again");
        assert_eq!(first, second);
        assert_eq!(first.len(), 64);
        assert!(
            first
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
    }

    /// The M10-T5 version-blind manifest digest (commit A′): the digest
    /// face must be invariant to the two ambient build-metadata fields
    /// (`app_version`, `git_sha`) and sensitive to everything else.
    /// Three assertions, the T2 non-vacuity shape:
    /// (1) manifests differing ONLY in those two fields hash EQUAL;
    /// (2) a manifest differing in real content hashes DIFFERENT;
    /// (3) the normalization actually fired — a version-bearing
    /// manifest's normalized digest differs from its raw sha256 (guards
    /// the marker against rot, e.g. a serde pretty-print inserting
    /// spaces after the colons).
    #[test]
    fn manifest_digest_is_version_blind_and_content_sensitive() {
        let dir = std::env::temp_dir().join("epic-harness-baseline-test");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let body = r#"{"schema_version":1,"app_version":"#;
        let tail = r#","fixture":{"filename":"x.dsn"},"final_state":"COMPLETED","exit_code":0,"output_written":true}"#;
        // (1) version-blind: only app_version/git_sha differ.
        let v1 = dir.join("manifest-v1.json");
        std::fs::write(
            &v1,
            format!("{body}\"0.1.0\",\"git_sha\":\"unknown\"{tail}"),
        )
        .expect("write v1");
        let v2 = dir.join("manifest-v2.json");
        std::fs::write(
            &v2,
            format!("{body}\"9.9.9\",\"git_sha\":\"deadbeef\"{tail}"),
        )
        .expect("write v2");
        let d1 = normalized_manifest_sha256(&v1).expect("hash v1");
        let d2 = normalized_manifest_sha256(&v2).expect("hash v2");
        assert_eq!(
            d1, d2,
            "manifests differing only in app_version/git_sha must hash equal"
        );
        // (2) content-sensitive: a real field moves the digest.
        let v3 = dir.join("manifest-v3.json");
        std::fs::write(
            &v3,
            format!(
                "{body}\"0.1.0\",\"git_sha\":\"unknown\",\"fixture\":{{\"filename\":\"y.dsn\"}},\"final_state\":\"COMPLETED\",\"exit_code\":0,\"output_written\":true}}"
            ),
        )
        .expect("write v3");
        let d3 = normalized_manifest_sha256(&v3).expect("hash v3");
        assert_ne!(
            d1, d3,
            "a manifest differing in real content must hash different"
        );
        // (3) non-vacuous vs the raw face: the replace fired.
        let raw1 = sha256_file(&v1).expect("raw hash v1");
        assert_ne!(
            d1, raw1,
            "the normalized digest of a version-bearing manifest must differ from its raw sha256"
        );
    }

    /// The fix-round Q1 bail: the two canonical fields are unconditional
    /// in the CLI renderer, so marker-ABSENT bytes are malformed input
    /// for this face and must bail loudly — the silent raw passthrough
    /// would quietly re-arm the version-rotation face the handoff
    /// closed. Two absence shapes: the field omitted, and a
    /// pretty-printed face where the space after the colon defeats the
    /// no-space marker.
    #[test]
    fn manifest_digest_bails_on_marker_absent_bytes() {
        let dir = std::env::temp_dir().join("epic-harness-baseline-test");
        std::fs::create_dir_all(&dir).expect("temp dir");
        // (a) the field omitted entirely.
        let no_field = dir.join("manifest-no-version.json");
        std::fs::write(
            &no_field,
            r#"{"schema_version":1,"git_sha":"unknown","final_state":"COMPLETED"}"#,
        )
        .expect("write no-field");
        let err = normalized_manifest_sha256(&no_field)
            .expect_err("an app_version-less manifest must bail");
        assert!(
            err.to_string().contains("missing canonical"),
            "the bail must name the missing canonical field: {err}"
        );
        // (b) pretty-printed: `"app_version": "2.0.0"` (space after the
        // colon) — the canonical marker never matches.
        let pretty = dir.join("manifest-pretty.json");
        std::fs::write(
            &pretty,
            r#"{"schema_version":1,"app_version": "2.0.0","git_sha":"unknown","final_state":"COMPLETED"}"#,
        )
        .expect("write pretty");
        let err = normalized_manifest_sha256(&pretty)
            .expect_err("a pretty-printed app_version must bail (marker absent)");
        assert!(
            err.to_string().contains("missing canonical"),
            "the bail must name the missing canonical field: {err}"
        );
    }
}

#[cfg(test)]
mod compare_tests {
    use super::*;

    fn record(
        state: &str,
        incomplete: Option<i64>,
        violations: Option<i64>,
        score: Option<f64>,
    ) -> BaselineRecord {
        BaselineRecord {
            schema_version: 1,
            engine: "java".into(),
            app_version: Some("2.5.0".into()),
            git_sha: Some("e7f9bdf1".into()),
            fixture: "x.dsn".into(),
            fixture_sha256: None,
            final_state: state.into(),
            exit_code: Some(0),
            incomplete_count: incomplete,
            maximum_count: Some(50),
            clearance_violations_total: violations,
            clearance_router_introduced: violations,
            normalized_score: score,
            optimizer_score: None,
            ses_sha256: None,
            autorouter_seconds: Some(10.0),
            optimizer_seconds: None,
            passes_completed: Some(20),
            profile: None,
            captured_at_unix: 0,
            notes: None,
        }
    }

    #[test]
    fn identical_records_pass_with_zero_failures() {
        let a = record("COMPLETED", Some(0), Some(0), Some(990.0));
        let b = record("COMPLETED", Some(0), Some(0), Some(990.0));
        assert_eq!(compare(&a, &b), Vec::new());
    }

    #[test]
    fn score_within_relative_epsilon_passes() {
        let a = record("COMPLETED", Some(0), Some(0), Some(990.0));
        let b = record("COMPLETED", Some(0), Some(0), Some(990.0 + 1e-9));
        assert_eq!(compare(&a, &b), Vec::new());
    }

    #[test]
    fn count_and_state_regressions_fail() {
        let a = record("COMPLETED", Some(0), Some(0), Some(990.0));
        let b = record("TIMED_OUT", Some(3), Some(1), Some(950.0));
        assert_eq!(compare(&a, &b).len(), 4);
    }

    #[test]
    fn wall_time_and_ses_hash_are_never_gated() {
        let mut a = record("COMPLETED", Some(0), Some(0), Some(990.0));
        a.autorouter_seconds = Some(10.0);
        a.ses_sha256 = Some("aaa".into());
        let mut b = record("COMPLETED", Some(0), Some(0), Some(990.0));
        b.autorouter_seconds = Some(999.0);
        b.ses_sha256 = Some("bbb".into());
        assert_eq!(compare(&a, &b), Vec::new());
    }

    /// Some-vs-None on the score must surface the true Option payloads (no
    /// fake 0.0), and both-absent must pass — nothing to compare.
    #[test]
    fn score_option_mismatch_reports_absence_honestly() {
        let a = record("COMPLETED", Some(0), Some(0), Some(990.0));
        let b = record("COMPLETED", Some(0), Some(0), None);
        let failures = compare(&a, &b);
        assert_eq!(failures.len(), 1);
        assert_eq!(
            failures[0],
            GateFailure::NormalizedScore {
                expected: Some(990.0),
                actual: None
            }
        );
        let both_absent_a = record("COMPLETED", Some(0), Some(0), None);
        let both_absent_b = record("COMPLETED", Some(0), Some(0), None);
        assert_eq!(compare(&both_absent_a, &both_absent_b), Vec::new());
    }

    /// The profile marker is gated: a full-flow run (profile None) compared
    /// against a router-only record must FAIL with exactly the Profile
    /// failure — the trap-3 silent-mismatch detector — while matching
    /// markers (Some==Some and None==None) stay green.
    #[test]
    fn compare_gates_profile_mismatch_and_agreement() {
        let router_only = {
            let mut r = record("COMPLETED", Some(0), Some(0), Some(990.0));
            r.profile = Some("router-only".into());
            r
        };
        let full_flow = record("COMPLETED", Some(0), Some(0), Some(990.0));

        let failures = compare(&router_only, &full_flow);
        assert_eq!(
            failures.len(),
            1,
            "marker mismatch must be the sole failure"
        );
        assert_eq!(
            failures[0],
            GateFailure::Profile {
                expected: Some("router-only".into()),
                actual: None,
            }
        );

        // Symmetric face: expected full-flow, actual router-only also fails
        // (the direction verify actually hits).
        let failures = compare(&full_flow, &router_only);
        assert_eq!(
            failures[0],
            GateFailure::Profile {
                expected: None,
                actual: Some("router-only".into()),
            }
        );

        assert_eq!(compare(&router_only, &router_only), Vec::new());
        assert_eq!(compare(&full_flow, &full_flow), Vec::new());
    }
}
