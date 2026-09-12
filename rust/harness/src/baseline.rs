//! Distills oracle runs into committed golden baselines and compares runs
//! against them (design §5: counts/score/state are gated; wall time is a
//! tracked trend, never a gate; the SES hash is identity info only —
//! semantic SES comparison arrives with epic-dsn in M1).

use crate::manifest::RoutingResultManifest;
use crate::oracle::OracleRun;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

pub const BASELINE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
    pub captured_at_unix: i64,
    pub notes: Option<String>,
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let bytes =
        std::fs::read(path).with_context(|| format!("reading {} for hashing", path.display()))?;
    let digest = Sha256::digest(&bytes);
    Ok(format!("{digest:x}"))
}

/// Reduces one oracle run to the committed baseline record.
pub fn distill(fixture_rel: &str, run: &OracleRun) -> BaselineRecord {
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
        captured_at_unix: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
        notes: note,
    }
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
            "clearance_violations": { "total_count": 1, "router_introduced_count": 1 }
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
        let b = distill("fixtures/x.dsn", &run);
        assert_eq!(b.schema_version, 1);
        assert_eq!(b.engine, "java");
        assert_eq!(b.final_state, "COMPLETED");
        assert_eq!(b.incomplete_count, Some(2));
        assert_eq!(b.maximum_count, Some(50));
        assert_eq!(b.clearance_violations_total, Some(1));
        assert_eq!(b.normalized_score, Some(990.0));
        assert_eq!(b.autorouter_seconds, Some(10.0));
        assert_eq!(b.passes_completed, Some(20));
        assert_eq!(b.ses_sha256, None); // ses file does not exist in test
    }

    #[test]
    fn records_harness_timeout_honestly() {
        let run = synthetic_run(None, true);
        let b = distill("fixtures/x.dsn", &run);
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
        let b = distill("fixtures/x.dsn", &run);
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
        let mut record = distill("fixtures/x.dsn", &run);
        // Make the round trip non-trivial: populate a field distill left
        // None-ish and clear another, so Some/None survive JSON equally.
        record.notes = Some("captured on CI runner".into());
        record.optimizer_score = None;
        let json = serde_json::to_string(&record).expect("serialize");
        let back: BaselineRecord = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(record, back);
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
}
