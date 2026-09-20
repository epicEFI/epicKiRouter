// consumed by M0 Tasks 4-6
#![allow(dead_code)]

//! Serde mirror of the Java `RoutingResultManifest` (schema v1) — the subset
//! the parity gates read. Field names MUST match the `@SerializedName`
//! values in `src/main/java/app/freerouting/core/results/RoutingResultManifest.java`,
//! with the nested board-statistics names in `core/scoring/BoardStatistics*.java`.
//! Gson emits additional fields (bounds, resource_usage, settings_snapshot…)
//! that we deliberately ignore: do NOT add `deny_unknown_fields`.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct RoutingResultManifest {
    pub schema_version: u32,
    pub app_version: String,
    pub git_sha: String,
    pub fixture: FixtureInfo,
    pub phases: PhaseMetrics,
    pub board_statistics: Option<BoardStatistics>,
    pub normalized_score: Option<f64>,
    pub optimizer_score: Option<f64>,
    pub final_state: String,
    pub exit_code: i32,
    pub output_written: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FixtureInfo {
    pub filename: Option<String>,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PhaseMetrics {
    #[serde(default)]
    pub fanout: PhaseDetail,
    #[serde(default)]
    pub autorouter: PhaseDetail,
    #[serde(default)]
    pub optimizer: PhaseDetail,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PhaseDetail {
    pub duration_seconds: Option<f64>,
    pub passes_completed: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BoardStatistics {
    pub connections: Option<Connections>,
    pub clearance_violations: Option<ClearanceViolations>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Connections {
    pub incomplete_count: Option<i64>,
    pub maximum_count: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClearanceViolations {
    pub total_count: Option<i64>,
    pub router_introduced_count: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
  "schema_version": 1,
  "generated_at": "2026-09-11T12:00:00Z",
  "app_version": "2.5.0",
  "git_sha": "e7f9bdf1",
  "fixture": { "filename": "DAC2020_bm08.dsn", "sha256": "abc123" },
  "settings_snapshot": { "ignored": true },
  "phases": {
    "fanout": { "duration_seconds": 0.5, "passes_completed": 1 },
    "autorouter": { "duration_seconds": 12.25, "passes_completed": 20 },
    "optimizer": { "duration_seconds": 30.5, "passes_completed": 5 }
  },
  "board_statistics": {
    "connections": { "incomplete_count": 0, "maximum_count": 42 },
    "clearance_violations": { "total_count": 0, "router_introduced_count": 0 },
    "vias": { "total_count": 10 }
  },
  "bounds": { "ignored": true },
  "normalized_score": 998.5,
  "optimizer_score": 991.25,
  "final_state": "COMPLETED",
  "exit_code": 0,
  "output_written": true,
  "cpu_score": null
}"#;

    #[test]
    fn parses_manifest_ignoring_unknown_fields() {
        let m: RoutingResultManifest = serde_json::from_str(SAMPLE).expect("manifest must parse");
        assert_eq!(m.schema_version, 1);
        assert_eq!(m.git_sha, "e7f9bdf1");
        assert_eq!(m.fixture.sha256.as_deref(), Some("abc123"));
        assert_eq!(m.phases.autorouter.duration_seconds, Some(12.25));
        assert_eq!(m.phases.autorouter.passes_completed, Some(20));
        let stats = m.board_statistics.expect("stats present");
        assert_eq!(
            stats.connections.expect("connections").incomplete_count,
            Some(0)
        );
        assert_eq!(
            stats.clearance_violations.expect("violations").total_count,
            Some(0)
        );
        assert_eq!(m.normalized_score, Some(998.5));
        assert_eq!(m.final_state, "COMPLETED");
        assert_eq!(m.exit_code, 0);
        assert!(m.output_written);
    }

    /// A failed run: `fromJob` leaves `board_statistics` null (Gson omits
    /// nulls) and phase details may be absent — everything still parses.
    #[test]
    fn parses_minimal_manifest_with_absent_optionals() {
        let minimal: &str = r#"{
  "schema_version": 1,
  "app_version": "2.5.0",
  "git_sha": "e7f9bdf1",
  "fixture": { "filename": "DAC2020_bm08.dsn" },
  "phases": {},
  "final_state": "COMPLETED",
  "exit_code": 0,
  "output_written": true
}"#;
        let m: RoutingResultManifest =
            serde_json::from_str(minimal).expect("minimal manifest must parse");
        assert!(m.phases.fanout.duration_seconds.is_none());
        assert!(m.phases.fanout.passes_completed.is_none());
        assert!(m.phases.autorouter.duration_seconds.is_none());
        assert!(m.phases.optimizer.passes_completed.is_none());
        assert!(m.board_statistics.is_none());
        assert!(m.normalized_score.is_none());
        assert!(m.optimizer_score.is_none());
    }
}
