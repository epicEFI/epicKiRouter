//! Router quality scoreboard (M3 Task 15): DIRECTIONAL compare gates for
//! the Rust router against the committed T14 router-only Java records,
//! plus the Rust-vs-Rust determinism self-gate and the first-divergence
//! localizer. Java-free at run time — the Java side is the COMMITTED
//! records (`baselines/router-only/A/`); no oracle is ever spawned here
//! (record verification is T14's `verify` path, not this one).
//!
//! ## The gates (per Tier A fixture, anchors §2)
//!
//! - RUN INTEGRITY (precondition): the epic-cli subprocess must finish
//!   inside the tiers.yaml timeout, produce a parseable manifest, end
//!   `final_state == "COMPLETED"` with `exit_code == 0`, and leave the
//!   session file on disk. ANY integrity miss is a red gate for the
//!   fixture ("missing or TERMINATED Rust run = red gate") and the
//!   numeric gates are NOT evaluated — the comparison does not count.
//! - `incomplete_count(Rust) <= incomplete_count(Java)` — HARD.
//! - `clearance_violations_total(Rust) <= clearance_violations_total(Java)`
//!   — HARD.
//! - `normalized_score(Rust) >= normalized_score(Java) − ε` with the ONE
//!   relative policy [`SCORE_RELATIVE_EPSILON`] (see below). The equal
//!   boundary passes: `== Java` satisfies both `<=` gates and
//!   `>= Java − ε`.
//! - REPORT-ONLY, never gating: router_introduced (preferred 0),
//!   passes_completed, and (when the detail pass ran) the Rust
//!   trace/via/bend counts. The committed T14 records carry NO Java-side
//!   trace/via/bend counts (the distill never extracted them), so those
//!   deltas are Rust-context-only, not Rust-vs-Java.
//!
//! ## The ε policy (one policy, stated once)
//!
//! `ε(fixture) = 0.02 × |normalized_score(Java)|` — the RELATIVE 2% the
//! plan's anchor named as the example. Justification from the observed
//! deltas (T15 battery, 2026-09-20): the only fixture whose Rust run
//! completes today (bm08) reproduces the Java score EXACTLY (delta
//! 0.00), so the pure round-trip noise floor is at or below 1e-6
//! relative; 2% is four orders of magnitude above it while staying
//! SMALLER than one unrouted connection on the tightest board in the
//! tier (bm08: 1000/25 = 40 points = 4%), so ε can never mask a
//! completion regression — and it never needs to: the incomplete and
//! violation gates already gate every count regression exactly. The ε
//! arbitrates only score drift WITH equal-or-better counts (trace
//! length / via / bend aesthetics), where ±2% is the legitimate
//! cross-engine geometry tolerance. One policy for every fixture, no
//! per-fixture tuning; T17 may tighten it with real delta data.
//!
//! ## Comparability (the load-bearing flag)
//!
//! Every Rust run is `epic-cli route … --router.fanout.enabled=false`:
//! the T14 records were captured with fanout + optimizer disabled, the
//! optimizer does not exist in the Rust engine, and fanout-off ALSO
//! flips `remove_unconnected_vias = true` (the batch tail sweep) — the
//! exact Java face the records witness. A full-flow Rust run against
//! these records compares apples to oranges.
//!
//! ## The localizer
//!
//! On any red gate the harness prints the FIRST differing component:
//! the Rust run's per-net incomplete rows (which nets remain unrouted),
//! the first violation pairs in walk order, the legacy 0-1000 score
//! decomposition (unrouted penalty, violation penalty, bend penalty,
//! via costs, trace-length costs), and the run-integrity evidence
//! (exit code + stderr tail — today's panic signature class). The
//! per-net/violation/decomposition faces come from an IN-PROCESS detail
//! pass (same DSN, same argv face, same settings resolution through
//! `epic_cli`), which reproduces the subprocess run because the engine
//! is deterministic (the `router determinism` gate pins subprocess
//! byte-identity independently). The detail pass is panic-caught: a
//! fixture whose route panics (today: 10 of 11 — the T17 gap
//! inventory) localizes from the subprocess evidence instead.
//!
//! ## Determinism self-gate (java-free)
//!
//! One DSN, TWO epic-cli runs to SEPARATE output paths, byte-identical
//! SES files AND byte-identical manifests expected (the manifest omits
//! the non-determinism family by construction; deterministic budgets
//! default ON). Any divergence is a real determinism bug — buglog it.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::Subcommand;

use crate::baseline::{BASELINE_SCHEMA_VERSION, BaselineRecord, sha256_file};
use crate::manifest::RoutingResultManifest;
use crate::oracle::{find_repo_root, parse_manifest_file, wait_with_timeout};
use crate::tiers::TierFile;

/// The ONE relative score-epsilon policy (module docs): ε = 2% of the
/// Java score. Never tuned per fixture; T17 may tighten with data.
pub const SCORE_RELATIVE_EPSILON: f64 = 0.02;

/// The default determinism fixture: the smallest Tier A board AND the
/// only fixture whose Rust route completes today (a panicking fixture
/// would produce no output to compare).
pub const DEFAULT_DETERMINISM_FIXTURE: &str = "DAC2020_boards/DAC2020_bm08.dsn";

/// THE comparability flag (module docs): fanout-off flips the batch tail
/// sweep via [`epic_cli::route::build_batch_settings`] — the exact
/// router-only face the committed T14 records witness. It lives here as a
/// const so the argv builder is the single source and the banner phrase is
/// DERIVED from the built argv (they cannot drift: a dropped flag stops
/// the log claiming fanout-off). Spec-review OW-M2 survivor hardening.
pub const COMPARABILITY_FLAG: &str = "--router.fanout.enabled=false";

/// THE one argv builder for every `router compare` subprocess run (the
/// run faces cannot drift between sites — there is no second argv). The
/// comparability flag rides EXACTLY once, as the argv suffix.
#[must_use]
pub fn route_argv(
    cli_bin: &Path,
    dsn: &Path,
    ses_path: &Path,
    manifest_path: &Path,
) -> Vec<String> {
    vec![
        cli_bin.to_string_lossy().into_owned(),
        "route".to_string(),
        "-de".to_string(),
        dsn.to_string_lossy().into_owned(),
        "-do".to_string(),
        ses_path.to_string_lossy().into_owned(),
        "--result-json".to_string(),
        manifest_path.to_string_lossy().into_owned(),
        COMPARABILITY_FLAG.to_string(),
    ]
}

/// The banner's comparability phrase, DERIVED from an actual built argv —
/// if the flag ever leaves [`route_argv`], the log stops claiming
/// fanout-off instead of lying (the OW-M2 "hardcoded banner" face).
#[must_use]
pub fn comparability_banner_phrase(argv: &[String]) -> String {
    if argv.iter().any(|arg| arg == COMPARABILITY_FLAG) {
        format!("{COMPARABILITY_FLAG} vs committed router-only records")
    } else {
        "COMPARABILITY FLAG MISSING FROM THE ARGV vs committed router-only records".into()
    }
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum RouterCommand {
    /// Per Tier A fixture: run `epic-cli route --router.fanout.enabled=false`,
    /// gate the manifest against the committed router-only Java record
    /// (java-free). Every Tier A fixture gets an explicit verdict.
    Compare {
        /// Only run fixtures whose tiers.yaml path contains this substring
        /// (the rest still appear, marked filtered-out).
        #[arg(long)]
        fixture: Option<String>,
        /// CI report mode: print every verdict, always exit 0 (the T17
        /// flip is dropping this flag — see SEAM).
        #[arg(long)]
        report_only: bool,
        /// Force the in-process detail pass on EVERY fixture (doubles the
        /// routing wall time; adds trace/via/bend counts + the
        /// detail-vs-manifest consistency face per fixture).
        #[arg(long)]
        detail: bool,
        /// Work-root override (default `harness/runs/router-compare`).
        #[arg(long)]
        work_root: Option<PathBuf>,
    },
    /// Java-free determinism self-gate: one DSN, two epic-cli runs to
    /// SEPARATE output paths, byte-identical SES + manifest expected.
    Determinism {
        /// The fixture (tiers.yaml path suffix); defaults to the smallest
        /// Tier A board (`DAC2020_boards/DAC2020_bm08.dsn`).
        #[arg(long)]
        fixture: Option<String>,
        /// Work-root override (default `harness/runs/router-determinism`).
        #[arg(long)]
        work_root: Option<PathBuf>,
    },
}

pub fn run(cmd: RouterCommand) -> Result<()> {
    let repo_root = find_repo_root()?;
    match cmd {
        RouterCommand::Compare {
            fixture,
            report_only,
            detail,
            work_root,
        } => compare_cmd(
            &repo_root,
            fixture.as_deref(),
            report_only,
            detail,
            work_root,
        ),
        RouterCommand::Determinism { fixture, work_root } => {
            determinism_cmd(&repo_root, fixture.as_deref(), work_root)
        }
    }
}

// ---------------------------------------------------------------------------
// The Tier A fixture enum (tiers.yaml authoritative; trap 2: no silent skip)
// ---------------------------------------------------------------------------

/// The baselines directory of the router-only records.
pub fn router_only_baselines_dir(repo_root: &Path) -> PathBuf {
    repo_root
        .join("rust/harness/baselines")
        .join("router-only")
        .join("A")
}

/// One Tier A compare row: the fixture, its timeout, its committed record.
#[derive(Debug, Clone)]
pub struct RouterFixture {
    /// Path relative to `fixtures_root` (tiers.yaml spelling, may contain
    /// spaces — `KiCad_10_demos/sonde xilinx.dsn`).
    pub tier_path: String,
    /// Repo-relative DSN path.
    pub repo_rel_dsn: String,
    pub timeout_seconds: u64,
    /// The committed router-only record for the fixture.
    pub baseline_path: PathBuf,
}

/// The Tier A fixture walk IN tiers.yaml ORDER, each with its committed
/// record path. The enum completeness is pinned: dropping a fixture here
/// is the finding-class trap 2 names.
pub fn tier_a_fixtures(repo_root: &Path) -> Result<Vec<RouterFixture>> {
    let tiers_path = repo_root.join("rust/harness/config/tiers.yaml");
    let tier_file = TierFile::load(&tiers_path)?;
    let tier = tier_file
        .tier("A")
        .with_context(|| format!("tier A missing from {}", tiers_path.display()))?;
    let fixtures_root = tier_file.fixtures_root.display().to_string();
    let baselines = router_only_baselines_dir(repo_root);
    Ok(tier
        .fixtures
        .iter()
        .map(|fixture| RouterFixture {
            tier_path: fixture.path.clone(),
            repo_rel_dsn: format!("{fixtures_root}/{}", fixture.path),
            timeout_seconds: fixture.timeout_seconds,
            baseline_path: baselines.join(format!("{}.baseline.json", fixture.path)),
        })
        .collect())
}

/// Loads one committed router-only record strictly: exists, parses
/// (`deny_unknown_fields`), schema current, engine `java`, profile
/// `router-only`. The error is a plain String so callers can tally every
/// offender into explicit verdicts (never a silent skip).
pub fn load_java_router_only_record(path: &Path) -> std::result::Result<BaselineRecord, String> {
    if !path.is_file() {
        return Err("missing baseline record — the T14 capture never wrote it".into());
    }
    let raw = std::fs::read_to_string(path).map_err(|e| format!("unreadable: {e}"))?;
    let record: BaselineRecord =
        serde_json::from_str(&raw).map_err(|e| format!("invalid baseline JSON: {e}"))?;
    if record.schema_version != BASELINE_SCHEMA_VERSION {
        return Err(format!(
            "schema version {} (harness expects {})",
            record.schema_version, BASELINE_SCHEMA_VERSION
        ));
    }
    if record.engine != "java" {
        return Err(format!(
            "engine {} — the records are the JAVA side",
            record.engine
        ));
    }
    if record.profile.as_deref() != Some("router-only") {
        return Err(format!(
            "profile {:?} — the compare gates against router-only records",
            record.profile
        ));
    }
    Ok(record)
}

// ---------------------------------------------------------------------------
// The epic-cli subprocess
// ---------------------------------------------------------------------------

/// Locates the `epic-cli` binary: `EPIC_CLI` env, the harness binary's
/// sibling (cargo puts both in `target/<profile>/`), or the repo's
/// `target/{debug,release}`. Never spawns anything.
pub fn resolve_epic_cli(repo_root: &Path) -> Result<PathBuf> {
    if let Ok(path) = std::env::var("EPIC_CLI") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
        bail!("EPIC_CLI={} is not a file", path.display());
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        let sibling = parent.join("epic-cli");
        if sibling.is_file() {
            return Ok(sibling);
        }
    }
    for profile in ["debug", "release"] {
        let candidate = repo_root.join("rust/target").join(profile).join("epic-cli");
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    bail!(
        "epic-cli binary not found — build it once with `cargo build -p epic-cli` (or set EPIC_CLI)"
    )
}

/// One epic-cli subprocess run's on-disk outcome.
pub struct CliRun {
    pub timed_out: bool,
    pub exit_code: Option<i32>,
    pub manifest: Option<RoutingResultManifest>,
    pub manifest_error: Option<String>,
    /// The session file path (may not exist — a failed run writes none).
    pub ses_path: PathBuf,
    pub wall_seconds: f64,
}

/// Runs `epic-cli route` fanout-off into `work_dir` (`out.ses`,
/// `manifest.json`, `stdout.log`, `stderr.log`) with a harness timeout.
/// Stale outputs are deleted BEFORE the spawn so a run that dies early
/// never reads the previous run's files as its own.
pub fn run_cli(cli_bin: &Path, dsn: &Path, work_dir: &Path, timeout: Duration) -> Result<CliRun> {
    let ses_path = work_dir.join("out.ses");
    let manifest_path = work_dir.join("manifest.json");
    std::fs::create_dir_all(work_dir)
        .with_context(|| format!("creating work dir {}", work_dir.display()))?;
    let _ = std::fs::remove_file(&ses_path);
    let _ = std::fs::remove_file(&manifest_path);

    // THE single argv source ([`route_argv`]): the comparability flag
    // cannot drift between the run faces.
    let argv = route_argv(cli_bin, dsn, &ses_path, &manifest_path);
    let stdout_file = std::fs::File::create(work_dir.join("stdout.log"))
        .with_context(|| format!("creating {}", work_dir.join("stdout.log").display()))?;
    let stderr_file = std::fs::File::create(work_dir.join("stderr.log"))
        .with_context(|| format!("creating {}", work_dir.join("stderr.log").display()))?;

    let started = Instant::now();
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::from(stderr_file))
        .spawn()
        .with_context(|| format!("spawning {}", cli_bin.display()))?;
    let (exit_code, timed_out) = wait_with_timeout(&mut child, timeout);
    let wall_seconds = started.elapsed().as_secs_f64();
    let (manifest, manifest_error) = parse_manifest_file(&manifest_path);
    Ok(CliRun {
        timed_out,
        exit_code,
        manifest,
        manifest_error,
        ses_path,
        wall_seconds,
    })
}

/// The last non-empty stderr lines (the panic/stop-cause evidence the
/// localizer prints on a red integrity gate).
pub fn stderr_tail(work_dir: &Path, max_lines: usize) -> Vec<String> {
    let raw = std::fs::read_to_string(work_dir.join("stderr.log")).unwrap_or_default();
    let lines: Vec<String> = raw
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_string)
        .collect();
    let start = lines.len().saturating_sub(max_lines);
    lines[start..].to_vec()
}

// ---------------------------------------------------------------------------
// The directional gates
// ---------------------------------------------------------------------------

/// The gate-relevant projection of a [`CliRun`] (what [`compare_directional`]
/// reads). Synthetic-world pinnable: no filesystem, no subprocess.
#[derive(Debug, Clone)]
pub struct RustOutcome {
    pub timed_out: bool,
    pub exit_code: Option<i32>,
    pub final_state: Option<String>,
    pub output_written: bool,
    pub incomplete_count: Option<i64>,
    pub clearance_violations_total: Option<i64>,
    pub normalized_score: Option<f64>,
    /// Whether the session file exists on disk (harness-side check).
    pub ses_present: bool,
}

impl RustOutcome {
    /// Projects a subprocess run. A missing manifest maps to all-None
    /// faces (the integrity gate fires first — numerics are not judged).
    pub fn from_run(run: &CliRun) -> Self {
        let manifest = run.manifest.as_ref();
        let stats = manifest.and_then(|m| m.board_statistics.as_ref());
        RustOutcome {
            timed_out: run.timed_out,
            exit_code: run.exit_code,
            final_state: manifest.map(|m| m.final_state.clone()),
            output_written: manifest.is_some_and(|m| m.output_written),
            incomplete_count: stats
                .and_then(|s| s.connections.as_ref())
                .and_then(|c| c.incomplete_count),
            clearance_violations_total: stats
                .and_then(|s| s.clearance_violations.as_ref())
                .and_then(|c| c.total_count),
            normalized_score: manifest.and_then(|m| m.normalized_score),
            ses_present: run.ses_path.is_file(),
        }
    }
}

/// One directional gate failure. The report-only fields are structurally
/// absent — they CANNOT gate because they never enter this enum.
#[derive(Debug, Clone, PartialEq)]
pub enum GateFailure {
    /// The run does not count (timeout / no manifest / bad state / bad
    /// exit / missing session): the numeric gates are skipped.
    RunIntegrity { reason: String },
    IncompleteNotBetter {
        java: Option<i64>,
        rust: Option<i64>,
    },
    ViolationsNotBetter {
        java: Option<i64>,
        rust: Option<i64>,
    },
    ScoreNotWithinEpsilon {
        java: Option<f64>,
        rust: Option<f64>,
        epsilon: f64,
    },
}

/// `ε(fixture) = SCORE_RELATIVE_EPSILON × |java score|`, floored at 1.0
/// so a tiny Java score cannot shrink ε below the JSON round-trip scale.
#[must_use]
pub fn epsilon_for(java_score: f64) -> f64 {
    SCORE_RELATIVE_EPSILON * java_score.abs().max(1.0)
}

/// The directional compare (module docs): integrity FIRST — any integrity
/// failure is the sole verdict and the numeric gates are not judged (the
/// comparison does not count). Otherwise the three directional gates,
/// each independent, all evaluated. Directional EQUALITY passes: `==`
/// Java satisfies `<=` and `>= Java − ε`.
pub fn compare_directional(java: &BaselineRecord, rust: &RustOutcome) -> Vec<GateFailure> {
    // Run integrity (the precondition).
    let mut integrity: Vec<GateFailure> = Vec::new();
    if rust.timed_out {
        integrity.push(GateFailure::RunIntegrity {
            reason: "harness timeout — the run was killed".into(),
        });
    }
    if rust.final_state.is_none() {
        integrity.push(GateFailure::RunIntegrity {
            reason: "no manifest produced".into(),
        });
    }
    if let Some(state) = &rust.final_state
        && state != "COMPLETED"
    {
        integrity.push(GateFailure::RunIntegrity {
            reason: format!("final_state {state} (COMPLETED required)"),
        });
    }
    if rust.exit_code != Some(0) {
        integrity.push(GateFailure::RunIntegrity {
            reason: format!("exit code {:?} (0 required)", rust.exit_code),
        });
    }
    if rust.final_state.as_deref() == Some("COMPLETED") && !rust.ses_present {
        integrity.push(GateFailure::RunIntegrity {
            reason: "manifest says COMPLETED but no session file exists".into(),
        });
    }
    if !integrity.is_empty() {
        return integrity;
    }

    let mut failures = Vec::new();
    // incomplete <= Java (both Some: exact compare; any None fails loudly
    // naming the absence — no fake 0.0).
    match (java.incomplete_count, rust.incomplete_count) {
        (Some(j), Some(r)) if r <= j => {}
        (j, r) => failures.push(GateFailure::IncompleteNotBetter { java: j, rust: r }),
    }
    match (
        java.clearance_violations_total,
        rust.clearance_violations_total,
    ) {
        (Some(j), Some(r)) if r <= j => {}
        (j, r) => failures.push(GateFailure::ViolationsNotBetter { java: j, rust: r }),
    }
    match (java.normalized_score, rust.normalized_score) {
        (Some(j), Some(r)) => {
            let epsilon = epsilon_for(j);
            if r < j - epsilon {
                failures.push(GateFailure::ScoreNotWithinEpsilon {
                    java: Some(j),
                    rust: Some(r),
                    epsilon,
                });
            }
        }
        (j, r) => failures.push(GateFailure::ScoreNotWithinEpsilon {
            java: j,
            rust: r,
            epsilon: f64::NAN,
        }),
    }
    failures
}

// ---------------------------------------------------------------------------
// Report-only fields (never gate; they only render)
// ---------------------------------------------------------------------------

/// The report-only faces. Structurally separate from [`GateFailure`]:
/// mutating any field here cannot change a verdict, only the line.
#[derive(Debug, Clone, PartialEq)]
pub struct ReportOnly {
    pub java_router_introduced: Option<i64>,
    pub rust_router_introduced: Option<i64>,
    pub java_passes: Option<i64>,
    pub rust_passes: Option<i64>,
    /// Detail-pass-only Rust geometry counts (None unless the detail pass
    /// ran); the committed Java records carry no such counts.
    pub rust_trace_count: Option<i64>,
    pub rust_via_count: Option<i64>,
    pub rust_bend_count: Option<i64>,
}

/// Extracts the manifest-backed report-only faces.
pub fn extract_report_only(java: &BaselineRecord, run: &CliRun) -> ReportOnly {
    let stats = run
        .manifest
        .as_ref()
        .and_then(|m| m.board_statistics.as_ref());
    ReportOnly {
        java_router_introduced: java.clearance_router_introduced,
        rust_router_introduced: stats
            .and_then(|s| s.clearance_violations.as_ref())
            .and_then(|c| c.router_introduced_count),
        java_passes: java.passes_completed,
        rust_passes: run
            .manifest
            .as_ref()
            .and_then(|m| m.phases.autorouter.passes_completed),
        rust_trace_count: None,
        rust_via_count: None,
        rust_bend_count: None,
    }
}

/// The per-fixture verdict line: explicit PASS/RED, every gate's
/// Rust-vs-Java numbers, the wall time, and the report-only fields.
#[must_use]
pub fn verdict_line(
    fixture: &str,
    failures: &[GateFailure],
    java: &BaselineRecord,
    rust: &RustOutcome,
    report: &ReportOnly,
    wall_seconds: f64,
) -> String {
    let status = if failures.is_empty() { "PASS" } else { "RED" };
    let eps = java.normalized_score.map(epsilon_for).unwrap_or(f64::NAN);
    let mut line = format!(
        "[{status}] {fixture}  incomplete {}<={:?}  violations {}<={:?}  score {}>={:?} (eps {eps:.2})  wall {wall_seconds:.1}s",
        display_opt_i64(rust.incomplete_count),
        java.incomplete_count,
        display_opt_i64(rust.clearance_violations_total),
        java.clearance_violations_total,
        display_opt_f64(rust.normalized_score),
        java.normalized_score,
    );
    if !failures.is_empty() {
        let names: Vec<String> = failures.iter().map(failure_name).collect();
        line.push_str(&format!("  FAIL[{}]", names.join(", ")));
    }
    line.push_str(&format!(
        "  | report-only: introduced {}/<{}> passes {}/<{}> traces {} vias {} bends {}",
        display_opt_i64(report.rust_router_introduced),
        display_opt_i64(report.java_router_introduced),
        display_opt_i64(report.rust_passes),
        display_opt_i64(report.java_passes),
        display_opt_i64(report.rust_trace_count),
        display_opt_i64(report.rust_via_count),
        display_opt_i64(report.rust_bend_count),
    ));
    line
}

fn display_opt_i64(value: Option<i64>) -> String {
    value.map_or("?".into(), |v| v.to_string())
}

fn display_opt_f64(value: Option<f64>) -> String {
    value.map_or("?".into(), |v| format!("{v:.2}"))
}

fn failure_name(failure: &GateFailure) -> String {
    match failure {
        GateFailure::RunIntegrity { reason } => format!("integrity({reason})"),
        GateFailure::IncompleteNotBetter { .. } => "incomplete>java".into(),
        GateFailure::ViolationsNotBetter { .. } => "violations>java".into(),
        GateFailure::ScoreNotWithinEpsilon { .. } => "score<java-eps".into(),
    }
}

// ---------------------------------------------------------------------------
// The first-divergence localizer
// ---------------------------------------------------------------------------

/// The legacy 0-1000 score decomposition (Java `BoardStatistics
/// calculateScore` faces): each term is the score POINTS the component
/// costs (normalized contribution = raw/maximum × 1000).
#[derive(Debug, Clone, PartialEq)]
pub struct ScoreTerms {
    pub maximum: f64,
    pub unrouted: f64,
    pub violation: f64,
    pub bend: f64,
    pub via: f64,
    pub trace_length: f64,
    /// The recomputed normalized score from the terms (engine face:
    /// `max(0, maximum − penalties − costs) / maximum × 1000`).
    pub normalized: f64,
}

/// Decomposes the legacy router score from the counting aggregate and the
/// scoring box (`None` scoring terms read 0 — the engine's NPE-parity
/// expects are unreachable through the counting ctor; the localizer must
/// not panic where the engine would have had numbers).
pub fn score_decomposition(
    stats: &epic_router::pipeline::board_statistics::BoardStatistics,
    scoring: &epic_router::pipeline::board_statistics::RoutingCostSettings,
) -> ScoreTerms {
    let maximum_count = f64::from(stats.connections.maximum_count.unwrap_or(0));
    let incomplete = f64::from(stats.connections.incomplete_count.unwrap_or(0));
    let violations = f64::from(stats.clearance_violations.total_count.unwrap_or(0));
    let bends = f64::from(stats.bends.total_count);
    let vias = f64::from(stats.vias.total_count);
    let trace_length = f64::from(
        stats
            .traces
            .total_length_mm
            .unwrap_or(stats.traces.total_length),
    );
    let net_penalty = f64::from(scoring.unrouted_net_penalty.unwrap_or(0.0));
    // maximum = maximumCount × unroutedNetPenalty (the legacy face); every
    // term below is normalized to the 0-1000 scale the score lives on.
    let maximum = maximum_count * net_penalty;
    let term = |raw: f64| {
        if maximum <= 0.0 {
            0.0
        } else {
            raw / maximum * 1000.0
        }
    };
    let unrouted = term(incomplete * net_penalty);
    let violation =
        term(violations * f64::from(scoring.clearance_violation_penalty.unwrap_or(0.0)));
    let bend = term(bends * f64::from(scoring.bend_penalty.unwrap_or(0.0)));
    let via = term(vias * f64::from(scoring.via_costs.unwrap_or(0)));
    let trace = term(
        trace_length
            * scoring
                .default_preferred_direction_trace_cost
                .unwrap_or(0.0),
    );
    // normalized = max(0, 1000 − Σterms); the maximum<=0 guard mirrors the
    // engine's divide-by-zero face (score 0, not a panic).
    let sum = unrouted + violation + bend + via + trace;
    let normalized = if maximum > 0.0 {
        (1000.0 - sum).max(0.0)
    } else {
        0.0
    };
    ScoreTerms {
        maximum,
        unrouted,
        violation,
        bend,
        via,
        trace_length: trace,
        normalized,
    }
}

/// One per-net incomplete row of the localizer (the nets the Rust run
/// left unrouted).
#[derive(Debug, Clone, PartialEq)]
pub struct NetRow {
    pub net_no: i32,
    pub groups: usize,
    pub incomplete_count: usize,
}

/// One violating item pair of the localizer (walk order, with depths).
#[derive(Debug, Clone, PartialEq)]
pub struct ViolationPair {
    pub a: i64,
    pub b: i64,
    pub layer: i64,
    pub expected_clearance: f64,
    pub actual_clearance: f64,
}

/// The in-process detail pass's outcome — the localizer's data source.
#[derive(Debug, Clone, PartialEq)]
pub struct RustDetail {
    pub incomplete_total: i64,
    /// Rows for nets with remaining incompletes only (ascending net).
    pub incomplete_nets: Vec<NetRow>,
    /// ALL walk-ordered deduped violation pairs (the localizer prints the
    /// first N).
    pub violations: Vec<ViolationPair>,
    pub terms: ScoreTerms,
    pub trace_count: i64,
    pub via_count: i64,
    pub bend_count: i64,
}

/// Renders the localizer output (the shape the pin locks): the FIRST
/// differing component named, then per-net incompletes, violation pairs,
/// the score decomposition, and the geometry counts. `detail` is None
/// when the detail pass was unavailable (a panicking engine) — the shape
/// degrades to the subprocess evidence + the aggregate numbers, never to
/// silence.
#[must_use]
pub fn localization_lines(
    fixture: &str,
    java: &BaselineRecord,
    rust: &RustOutcome,
    failures: &[GateFailure],
    detail: Option<&RustDetail>,
    stderr: &[String],
) -> Vec<String> {
    let mut lines = Vec::new();
    let first = failures.first().map(failure_name).unwrap_or_default();
    lines.push(format!("localize[{fixture}]: FIRST DIVERGENCE — {first}"));
    // Integrity evidence first: exit code + stderr tail (the panic class).
    if failures
        .iter()
        .any(|f| matches!(f, GateFailure::RunIntegrity { .. }))
    {
        lines.push(format!(
            "  run: timed_out={} exit_code={:?} final_state={:?} output_written={} ses={}",
            rust.timed_out,
            rust.exit_code,
            rust.final_state,
            rust.output_written,
            if rust.ses_present {
                "present"
            } else {
                "absent"
            },
        ));
        for line in stderr.iter().take(4) {
            lines.push(format!("  stderr: {line}"));
        }
    }
    // Aggregate faces.
    lines.push(format!(
        "  incomplete: rust {} vs java {:?}; violations: rust {} vs java {:?}",
        display_opt_i64(rust.incomplete_count),
        java.incomplete_count,
        display_opt_i64(rust.clearance_violations_total),
        java.clearance_violations_total,
    ));
    // Per-net incompletes (the nets the Rust run left unrouted).
    if let Some(detail) = detail {
        if detail.incomplete_nets.is_empty() {
            lines.push("  per-net incomplete: none (all nets routed or net-less)".into());
        } else {
            lines.push("  per-net incomplete (rust run):".into());
            for row in &detail.incomplete_nets {
                lines.push(format!(
                    "    net {}: incomplete {} (groups {})",
                    row.net_no, row.incomplete_count, row.groups
                ));
            }
        }
        // Violation pairs (first N, walk order).
        const FIRST_N: usize = 10;
        if detail.violations.is_empty() {
            lines.push("  violation pairs: none".into());
        } else {
            lines.push(format!(
                "  violation pairs ({} total, first {} in walk order):",
                detail.violations.len(),
                FIRST_N.min(detail.violations.len())
            ));
            for pair in detail.violations.iter().take(FIRST_N) {
                lines.push(format!(
                    "    item {} <-> item {} on layer {} (actual {:.3} < expected {:.3})",
                    pair.a, pair.b, pair.layer, pair.actual_clearance, pair.expected_clearance,
                ));
            }
        }
        // Score decomposition.
        let terms = &detail.terms;
        lines.push(format!(
            "  score decomposition (legacy 0-1000; maximum {:.2}):",
            terms.maximum
        ));
        lines.push(format!("    unrouted penalty      -{:.2}", terms.unrouted));
        lines.push(format!("    violation penalty     -{:.2}", terms.violation));
        lines.push(format!("    bend penalty          -{:.2}", terms.bend));
        lines.push(format!("    via costs             -{:.2}", terms.via));
        lines.push(format!(
            "    trace-length costs    -{:.2}",
            terms.trace_length
        ));
        lines.push(format!(
            "    => score {:.2} vs java {} (epsilon {:.2})",
            terms.normalized,
            display_opt_f64(java.normalized_score),
            java.normalized_score.map_or(f64::NAN, epsilon_for),
        ));
        lines.push(format!(
            "  traces {} vias {} bends {} (rust detail pass; the committed java records carry no geometry counts)",
            detail.trace_count, detail.via_count, detail.bend_count,
        ));
    } else {
        lines.push(
            "  detail pass unavailable (route did not complete) — per-net/violation/decomposition faces need a completing run".into(),
        );
    }
    lines
}

/// Runs the in-process detail pass: the SAME flow as the subprocess
/// (`epic_cli::route::run_route`'s steps, fanout-off argv face), stopped
/// after the post-route statistics. Panic-caught: an engine panic becomes
/// an Err stop-cause (today's Tier A reality on 10 of 11 fixtures). The
/// default panic hook is silenced for the duration (spec-review NOTE: the
/// hook would print the caught panic to stderr before the harness renders
/// its own NOTE) and restored on EVERY path out — the hook swap is
/// process-global, which is safe here because the harness is
/// single-threaded on this path and nothing else can panic concurrently.
fn run_detail_pass(dsn: &Path) -> Result<RustDetail, String> {
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        route_detail_inner(dsn).map_err(|e| e.to_string())
    }));
    std::panic::set_hook(previous_hook);
    result.unwrap_or_else(|payload| {
        Err(format!(
            "detail pass panicked: {}",
            payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic payload".into())
        ))
    })
}

/// The detail pass body (panic boundary lives in [`run_detail_pass`]).
fn route_detail_inner(dsn: &Path) -> Result<RustDetail, anyhow::Error> {
    use epic_board::board::Board;
    use epic_board::tree_manager::SearchTreeManager;
    use epic_cli::route::{CliDriverSink, build_batch_settings};
    use epic_cli::settings::{
        DsnLayer, MergedSettings, ResolvedRouteSettings, apply_board_specific_optimizations, merge,
        parse_route_args, validate,
    };
    use epic_dsn::reader::{DsnReadResult, read_board};
    use epic_dsn::ses_board::SesBoard;
    use epic_router::pipeline::batch::{BatchDriver, StopFace};
    use epic_router::pipeline::board_statistics::BoardStatistics;

    // The identical argv face the compare's subprocess gets (the fanout
    // coupling flows through the SAME build_batch_settings the flow uses).
    let scratch_ses = std::env::temp_dir().join(format!(
        "epic-t15-detail-{}-{}.ses",
        std::process::id(),
        dsn.file_name().and_then(|n| n.to_str()).unwrap_or("board")
    ));
    let argv = [
        "-de".to_string(),
        dsn.to_string_lossy().into_owned(),
        "-do".to_string(),
        scratch_ses.to_string_lossy().into_owned(),
        // The SAME const the subprocess argv builder uses — every run face
        // single-sources the comparability flag.
        COMPARABILITY_FLAG.to_string(),
    ];
    let args = parse_route_args(&argv).map_err(anyhow::Error::msg)?;

    let bytes = std::fs::read(dsn).with_context(|| format!("reading {}", dsn.display()))?;
    let mut ses = SesBoard::new();
    match read_board(&bytes, &mut ses) {
        DsnReadResult::Success { .. } | DsnReadResult::OutlineMissing { .. } => {}
        DsnReadResult::ParseError { location, detail } => {
            bail!("parse error at {location}: {detail}");
        }
        DsnReadResult::IoError => bail!("I/O error reading {}", dsn.display()),
    }

    let mut board = Board::from_ses_board(&ses);
    let mut manager = SearchTreeManager::new();
    manager.reinsert_tree_items(&mut board);
    epic_board::normalize_all::normalize_all_traces(&mut manager, &mut board);

    // The load-time violation seed (the flow's step 2b).
    let (pre_total, _) =
        epic_drc::clearance::all_clearance_violation_depths(&mut manager, &mut board);
    board.pre_existing_clearance_violations_count = i32::try_from(pre_total).unwrap_or(i32::MAX);

    // Settings: the same resolution chain as run_route step 3.
    let dsn_layer = DsnLayer::from_metadata(
        ses.metadata.autoroute_settings.as_ref(),
        usize::try_from(ses.metadata.layer_count).unwrap_or(0),
    );
    let mut merged = merge(&MergedSettings::default(), &dsn_layer, &args.layer);
    for warning in validate(&mut merged) {
        eprintln!("Warning: {warning}");
    }
    apply_board_specific_optimizations(&mut merged, &board);
    let resolved = ResolvedRouteSettings::resolve(&merged, args.deterministic_budgets);
    let batch = build_batch_settings(&resolved);

    // The batch driver (the same face as the subprocess).
    let mut sink = CliDriverSink::default();
    let mut driver = BatchDriver::new(&mut manager, &mut board, batch, StopFace::default());
    let run_result = driver.run(&mut sink);
    drop(driver);
    run_result.map_err(|e| anyhow::anyhow!("batch driver error: {e:?}"))?;

    // Post-route faces: the counting aggregate (score + geometry counts)
    // and the localizer walks (per-net incompletes + violation pairs).
    let stats = BoardStatistics::new(&mut manager, &mut board);
    let (_, net_rows) = epic_drc::incompletes::all_incompletes(&manager, &mut board);
    let (_violation_total, violation_rows) =
        epic_drc::clearance::all_clearance_violation_depths(&mut manager, &mut board);
    let _ = pre_total;
    let scoring = resolved
        .scoring
        .scoring
        .clone()
        .unwrap_or_else(epic_router::pipeline::board_statistics::default_routing_cost_settings);
    let terms = score_decomposition(&stats, &scoring);
    let incomplete_nets: Vec<NetRow> = net_rows
        .iter()
        .filter(|row| row.incomplete_count > 0)
        .map(|row| NetRow {
            net_no: row.net_no,
            groups: row.groups,
            incomplete_count: row.incomplete_count,
        })
        .collect();
    let incomplete_total: i64 = net_rows.iter().map(|row| row.incomplete_count as i64).sum();
    let violations: Vec<ViolationPair> = violation_rows
        .iter()
        .map(|row| ViolationPair {
            a: row.a,
            b: row.b,
            layer: row.layer,
            expected_clearance: row.expected_clearance,
            actual_clearance: row.actual_clearance,
        })
        .collect();
    let _ = scratch_ses; // no session write happens in-process
    Ok(RustDetail {
        incomplete_total,
        incomplete_nets,
        violations,
        terms,
        trace_count: i64::from(stats.traces.total_count),
        via_count: i64::from(stats.vias.total_count),
        bend_count: i64::from(stats.bends.total_count),
    })
}

// ---------------------------------------------------------------------------
// Determinism (Rust-vs-Rust byte identity)
// ---------------------------------------------------------------------------

/// The determinism verdict on the two runs' digests: OK line, or an Err
/// naming the first differing (or missing) artifact. Planted-divergence
/// pinnable.
pub fn determinism_verdict(
    ses_sha_1: Option<String>,
    ses_sha_2: Option<String>,
    manifest_sha_1: Option<String>,
    manifest_sha_2: Option<String>,
) -> std::result::Result<String, String> {
    if ses_sha_1.is_none() || ses_sha_2.is_none() {
        return Err(format!(
            "a run produced no session file (ses1={:?} ses2={:?})",
            ses_sha_1, ses_sha_2
        ));
    }
    if manifest_sha_1.is_none() || manifest_sha_2.is_none() {
        return Err(format!(
            "a run produced no manifest (man1={:?} man2={:?})",
            manifest_sha_1, manifest_sha_2
        ));
    }
    if ses_sha_1 != ses_sha_2 {
        return Err(format!(
            "SES bytes diverge across the two runs: {ses_sha_1:?} vs {ses_sha_2:?} — a real determinism bug"
        ));
    }
    if manifest_sha_1 != manifest_sha_2 {
        return Err(format!(
            "manifest bytes diverge across the two runs: {manifest_sha_1:?} vs {manifest_sha_2:?} — a real determinism bug"
        ));
    }
    Ok(format!(
        "determinism: OK (ses {ses_sha_1:?}, manifest {manifest_sha_1:?} byte-identical across two runs)"
    ))
}

/// One run's on-disk determinism artifacts (review OW-M3 hardening).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeterminismArtifacts {
    pub ses: PathBuf,
    pub manifest: PathBuf,
}

/// The two runs' artifacts as a STRUCTURAL pair: the digest step takes
/// both, so "which run's files feed which digest" is typed, not implied
/// by loose locals — a run1/run2 path slip cannot silently hash one run
/// twice (CI's only HARD gate rides on this).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeterminismPair {
    pub run1: DeterminismArtifacts,
    pub run2: DeterminismArtifacts,
}

/// The four run digests in verdict order: `(ses1, ses2, manifest1,
/// manifest2)` — one per [`DeterminismPair`] artifact (`None` when that
/// run wrote nothing).
pub type DeterminismDigests = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// Hashes the pair: `(ses1, ses2, manifest1, manifest2)`, each digest
/// computed from its OWN run's path (`None` when that run wrote nothing).
pub fn determinism_digests(pair: &DeterminismPair) -> Result<DeterminismDigests> {
    let digest = |path: &Path| -> Result<Option<String>> {
        path.is_file().then(|| sha256_file(path)).transpose()
    };
    Ok((
        digest(&pair.run1.ses)?,
        digest(&pair.run2.ses)?,
        digest(&pair.run1.manifest)?,
        digest(&pair.run2.manifest)?,
    ))
}

fn determinism_cmd(
    repo_root: &Path,
    fixture: Option<&str>,
    work_root: Option<PathBuf>,
) -> Result<()> {
    let started = Instant::now();
    let cli_bin = resolve_epic_cli(repo_root)?;
    let fixtures = tier_a_fixtures(repo_root)?;
    let wanted = fixture.unwrap_or(DEFAULT_DETERMINISM_FIXTURE);
    let entry = fixtures
        .iter()
        .find(|f| f.tier_path == wanted)
        .with_context(|| {
            format!("fixture {wanted:?} is not a Tier A fixture (tiers.yaml paths only)")
        })?;
    let dsn = repo_root.join(&entry.repo_rel_dsn);
    let work = work_root
        .map(|p| {
            if p.is_absolute() {
                p
            } else {
                repo_root.join(p)
            }
        })
        .unwrap_or_else(|| repo_root.join("rust/harness/runs/router-determinism"));
    println!(
        "router determinism: {} (two runs, fanout-off, timeout {}s)",
        entry.tier_path, entry.timeout_seconds
    );
    // Trap 5: SEPARATE output paths — run1/ and run2/ never share a file.
    let run1_dir = work.join("run1");
    let run2_dir = work.join("run2");
    let run1 = run_cli(
        &cli_bin,
        &dsn,
        &run1_dir,
        Duration::from_secs(entry.timeout_seconds),
    )?;
    let run2 = run_cli(
        &cli_bin,
        &dsn,
        &run2_dir,
        Duration::from_secs(entry.timeout_seconds),
    )?;
    // The digest sources are a STRUCTURAL run1/run2 pair (review OW-M3
    // hardening): a path slip cannot silently feed one run's files to
    // both sides of the verdict.
    let pair = DeterminismPair {
        run1: DeterminismArtifacts {
            ses: run1.ses_path.clone(),
            manifest: run1_dir.join("manifest.json"),
        },
        run2: DeterminismArtifacts {
            ses: run2.ses_path.clone(),
            manifest: run2_dir.join("manifest.json"),
        },
    };
    let (ses_1, ses_2, manifest_1, manifest_2) = determinism_digests(&pair)?;
    match determinism_verdict(ses_1, ses_2, manifest_1, manifest_2) {
        Ok(line) => {
            println!(
                "  {line}  (walls {:.1}s / {:.1}s, total {:.1}s)",
                run1.wall_seconds,
                run2.wall_seconds,
                started.elapsed().as_secs_f64()
            );
            Ok(())
        }
        Err(reason) => bail!(
            "router determinism FAILED for {}: {reason}",
            entry.tier_path
        ),
    }
}

// ---------------------------------------------------------------------------
// `router compare`
// ---------------------------------------------------------------------------

fn compare_cmd(
    repo_root: &Path,
    fixture_filter: Option<&str>,
    report_only: bool,
    force_detail: bool,
    work_root: Option<PathBuf>,
) -> Result<()> {
    let started = Instant::now();
    let cli_bin = resolve_epic_cli(repo_root)?;
    let fixtures = tier_a_fixtures(repo_root)?;
    let work = work_root
        .map(|p| {
            if p.is_absolute() {
                p
            } else {
                repo_root.join(p)
            }
        })
        .unwrap_or_else(|| repo_root.join("rust/harness/runs/router-compare"));
    // The banner's fanout-off claim is DERIVED from the actual argv
    // builder (not hardcoded): if the comparability flag ever leaves
    // [`route_argv`], the log says so instead of lying (OW-M2).
    let banner_comparability = comparability_banner_phrase(&route_argv(
        &cli_bin,
        Path::new("<fixture.dsn>"),
        Path::new("<out.ses>"),
        Path::new("<manifest.json>"),
    ));
    println!(
        "router compare: {} Tier A fixture(s); gates: incomplete<=J, violations<=J, score>=J-{}%*J (one relative policy); runs: epic-cli {} ; mode: {}",
        fixtures.len(),
        (SCORE_RELATIVE_EPSILON * 100.0),
        banner_comparability,
        if report_only {
            "report (exit 0)"
        } else {
            "gate"
        }
    );
    let mut red = 0usize;
    let mut green = 0usize;
    let mut filtered = 0usize;
    for entry in &fixtures {
        if let Some(filter) = fixture_filter
            && !entry.tier_path.contains(filter)
        {
            filtered += 1;
            println!(
                "  [SKIP] {}  (filtered-out by --fixture {filter:?})",
                entry.tier_path
            );
            continue;
        }
        // Every fixture gets an explicit verdict (trap 2): a bad record is
        // a RED verdict, never a skip.
        let java = match load_java_router_only_record(&entry.baseline_path) {
            Ok(record) => record,
            Err(reason) => {
                red += 1;
                println!("  [RED] {}  baseline: {reason}", entry.tier_path);
                continue;
            }
        };
        let dsn = repo_root.join(&entry.repo_rel_dsn);
        let work_dir = work.join(&entry.tier_path);
        let run = run_cli(
            &cli_bin,
            &dsn,
            &work_dir,
            Duration::from_secs(entry.timeout_seconds),
        )
        .with_context(|| format!("spawning the run for {}", entry.tier_path))?;
        let outcome = RustOutcome::from_run(&run);
        let failures = compare_directional(&java, &outcome);
        let mut report = extract_report_only(&java, &run);

        // The detail pass: forced by --detail, otherwise only on a red
        // gate (the localizer). Panic-caught inside.
        let mut detail: Option<RustDetail> = None;
        if force_detail || !failures.is_empty() {
            match run_detail_pass(&dsn) {
                Ok(pass) => {
                    report.rust_trace_count = Some(pass.trace_count);
                    report.rust_via_count = Some(pass.via_count);
                    report.rust_bend_count = Some(pass.bend_count);
                    // Consistency face (diagnostic, not a gate): the detail
                    // pass must reproduce the subprocess manifest's
                    // aggregates — the engine is deterministic.
                    if outcome.final_state.is_some()
                        && pass.incomplete_total != outcome.incomplete_count.unwrap_or(-1)
                    {
                        println!(
                            "  NOTE {}: detail-pass incomplete total {} != subprocess manifest {} — investigate (determinism face)",
                            entry.tier_path,
                            pass.incomplete_total,
                            outcome.incomplete_count.unwrap_or(-1)
                        );
                    }
                    detail = Some(pass);
                }
                Err(reason) => {
                    println!(
                        "  NOTE {}: detail pass unavailable: {reason}",
                        entry.tier_path
                    );
                }
            }
        }

        let line = verdict_line(
            &entry.tier_path,
            &failures,
            &java,
            &outcome,
            &report,
            run.wall_seconds,
        );
        if failures.is_empty() {
            green += 1;
            println!("  {line}");
        } else {
            red += 1;
            println!("  {line}");
            // A present-but-unparseable manifest (schema drift, a write
            // truncated by our own timeout kill) must not read as "never
            // produced" — surface the parse error with the red verdict.
            if let Some(err) = &run.manifest_error {
                println!(
                    "  NOTE {}: the manifest was present but unparseable: {err}",
                    entry.tier_path
                );
            }
            let stderr = stderr_tail(&work_dir, 4);
            for localized in localization_lines(
                &entry.tier_path,
                &java,
                &outcome,
                &failures,
                detail.as_ref(),
                &stderr,
            ) {
                println!("  {localized}");
            }
        }
    }
    println!(
        "router compare: {green} green, {red} red, {filtered} filtered-out in {:.1}s (mode: {})",
        started.elapsed().as_secs_f64(),
        if report_only { "report" } else { "gate" }
    );
    // Report mode ALWAYS exits 0 (the CI face); gate mode exits 0 only
    // when every fixture passed.
    if report_only || red == 0 {
        Ok(())
    } else {
        bail!("router compare: {red} fixture(s) RED");
    }
}

// ---------------------------------------------------------------------------
// Pins
// ---------------------------------------------------------------------------

#[cfg(test)]
mod pins {
    use super::*;

    /// A fully-populated synthetic Java record (the router-only face:
    /// profile Some, optimizer null).
    fn java_record(incomplete: i64, violations: i64, score: f64) -> BaselineRecord {
        BaselineRecord {
            schema_version: 1,
            engine: "java".into(),
            app_version: Some("2.4.2-SNAPSHOT".into()),
            git_sha: Some("unknown".into()),
            fixture: "scripts/benchmark/fixtures/x.dsn".into(),
            fixture_sha256: None,
            final_state: "COMPLETED".into(),
            exit_code: Some(0),
            incomplete_count: Some(incomplete),
            maximum_count: Some(100),
            clearance_violations_total: Some(violations),
            clearance_router_introduced: Some(0),
            normalized_score: Some(score),
            optimizer_score: None,
            ses_sha256: Some("aaaa".into()),
            autorouter_seconds: Some(1.0),
            optimizer_seconds: None,
            passes_completed: Some(18),
            profile: Some("router-only".into()),
            captured_at_unix: 0,
            notes: None,
        }
    }

    /// A healthy completing Rust outcome.
    fn rust_outcome(incomplete: i64, violations: i64, score: f64) -> RustOutcome {
        RustOutcome {
            timed_out: false,
            exit_code: Some(0),
            final_state: Some("COMPLETED".into()),
            output_written: true,
            incomplete_count: Some(incomplete),
            clearance_violations_total: Some(violations),
            normalized_score: Some(score),
            ses_present: true,
        }
    }

    /// THE directional semantics, both directions AND the equal boundary
    /// (anchors: "`== Java` passes <= and >=−ε"): better/equal Rust
    /// passes, worse Rust fails naming the gate, and the strict-`<`
    /// mutant (equality dropped) fails the equal world.
    #[test]
    fn directional_gates_both_directions_and_equal_boundary() {
        let java = java_record(2, 3, 986.32);

        // Equal: the boundary face — passes every gate.
        assert_eq!(
            compare_directional(&java, &rust_outcome(2, 3, 986.32)),
            Vec::new()
        );
        // Strictly better on every gate: passes.
        assert_eq!(
            compare_directional(&java, &rust_outcome(0, 0, 999.0)),
            Vec::new()
        );
        // Equal counts, better score: passes.
        assert_eq!(
            compare_directional(&java, &rust_outcome(2, 3, 990.0)),
            Vec::new()
        );

        // Worse in each direction, alone: fails naming exactly that gate.
        let f = compare_directional(&java, &rust_outcome(3, 3, 986.32));
        assert_eq!(
            f,
            vec![GateFailure::IncompleteNotBetter {
                java: Some(2),
                rust: Some(3)
            }]
        );
        let f = compare_directional(&java, &rust_outcome(2, 4, 986.32));
        assert_eq!(
            f,
            vec![GateFailure::ViolationsNotBetter {
                java: Some(3),
                rust: Some(4)
            }]
        );
        // Both count directions flipped at once: two failures.
        assert_eq!(
            compare_directional(&java, &rust_outcome(5, 9, 986.32)).len(),
            2
        );

        // Option-absence faces fail loudly naming the true payloads (the
        // epsilon is NaN there, which can never be assert_eq'd — f64 NaN
        // != NaN — so the payload match is structural).
        let mut absent = rust_outcome(2, 3, 986.32);
        absent.normalized_score = None;
        let failures = compare_directional(&java, &absent);
        assert_eq!(failures.len(), 1);
        assert!(
            matches!(
                failures[0],
                GateFailure::ScoreNotWithinEpsilon {
                    java: Some(986.32),
                    rust: None,
                    ..
                }
            ),
            "{failures:?}"
        );
    }

    /// THE ε policy application: RELATIVE (2% of the Java score), with
    /// the equal boundary and both edges pinned. The world's Java score
    /// is 500.0 so a relative ε (1.0) and a same-numbered ABSOLUTE ε
    /// mutant (2.0) DIVERGE at delta 1.5 — the mutant that survives a
    /// 1000-scale world dies here.
    #[test]
    fn score_gate_epsilon_is_relative_not_absolute() {
        assert_eq!(epsilon_for(500.0), 10.0);
        assert_eq!(epsilon_for(1000.0), 20.0);
        assert_eq!(epsilon_for(0.5), SCORE_RELATIVE_EPSILON, "the 1.0 floor");

        let java = java_record(0, 0, 500.0);
        // Equal passes; strictly better passes.
        assert_eq!(
            compare_directional(&java, &rust_outcome(0, 0, 500.0)),
            Vec::new()
        );
        assert_eq!(
            compare_directional(&java, &rust_outcome(0, 0, 501.0)),
            Vec::new()
        );
        // Inside ε (delta 1.5 < relative ε 10.0... and > absolute-mutant 2.0):
        // the RELATIVE policy passes 498.5 (500 − 10 = 490 <= 498.5).
        assert_eq!(
            compare_directional(&java, &rust_outcome(0, 0, 498.5)),
            Vec::new()
        );
        // Below ε: 480 < 500 − 10 → fails.
        let f = compare_directional(&java, &rust_outcome(0, 0, 480.0));
        assert_eq!(
            f,
            vec![GateFailure::ScoreNotWithinEpsilon {
                java: Some(500.0),
                rust: Some(480.0),
                epsilon: 10.0
            }]
        );

        // The 1000-scale boundary face: rust == java − ε passes EXACTLY
        // (>= is inclusive), rust just below fails.
        let big = java_record(0, 0, 1000.0);
        let eps = epsilon_for(1000.0);
        assert_eq!(
            compare_directional(&big, &rust_outcome(0, 0, 1000.0 - eps)),
            Vec::new()
        );
        let f = compare_directional(&big, &rust_outcome(0, 0, 1000.0 - eps - 0.01));
        assert_eq!(f.len(), 1);
        assert!(matches!(f[0], GateFailure::ScoreNotWithinEpsilon { .. }));
    }

    /// A TERMINATED/failed run with BETTER numbers is still red — the
    /// integrity precondition short-circuits and the numeric gates are
    /// not judged ("the comparison does not count").
    #[test]
    fn integrity_failure_short_circuits_even_with_better_numbers() {
        let java = java_record(5, 5, 800.0);
        let mut terminated = rust_outcome(0, 0, 1000.0);
        terminated.final_state = Some("TERMINATED".into());
        terminated.exit_code = Some(1);
        let failures = compare_directional(&java, &terminated);
        assert!(
            failures
                .iter()
                .all(|f| matches!(f, GateFailure::RunIntegrity { .. })),
            "only integrity failures: {failures:?}"
        );
        assert_eq!(failures.len(), 2, "state + exit, not the numeric gates");

        // Timeout face alone.
        let mut timed_out = rust_outcome(0, 0, 1000.0);
        timed_out.timed_out = true;
        let failures = compare_directional(&java, &timed_out);
        assert_eq!(failures.len(), 1);
        assert!(matches!(failures[0], GateFailure::RunIntegrity { .. }));

        // Completed-but-no-session face.
        let mut no_ses = rust_outcome(0, 0, 986.32);
        no_ses.ses_present = false;
        let failures = compare_directional(&java_record(2, 3, 986.32), &no_ses);
        assert_eq!(failures.len(), 1);
        assert!(matches!(failures[0], GateFailure::RunIntegrity { .. }));

        // No manifest at all (the panic face — today's Tier A reality):
        // the no-manifest face and the bad-exit face, and NOT the numeric
        // gates.
        let mut no_manifest = rust_outcome(0, 0, 0.0);
        no_manifest.final_state = None;
        no_manifest.exit_code = Some(101);
        no_manifest.ses_present = false;
        let failures = compare_directional(&java, &no_manifest);
        assert_eq!(failures.len(), 2, "no-manifest + exit only: {failures:?}");
        assert!(matches!(failures[0], GateFailure::RunIntegrity { .. }));
    }

    /// THE report-only pin: mutating router_introduced, passes_completed,
    /// or the geometry counts changes the VERDICT LINE but never the
    /// GATES (they are structurally outside `GateFailure`). Kills the
    /// report-field-into-gate mutant.
    #[test]
    fn report_only_fields_never_gate_but_do_render() {
        let java = java_record(2, 3, 986.32);
        let outcome = rust_outcome(2, 3, 986.32);
        assert_eq!(compare_directional(&java, &outcome), Vec::new());

        let quiet_report = ReportOnly {
            java_router_introduced: Some(0),
            rust_router_introduced: Some(0),
            java_passes: Some(18),
            rust_passes: Some(17),
            rust_trace_count: Some(10),
            rust_via_count: Some(2),
            rust_bend_count: Some(4),
        };
        let mut loud_report = quiet_report.clone();
        // Mutate EVERY report-only face...
        loud_report.rust_router_introduced = Some(50);
        loud_report.rust_passes = Some(1);
        loud_report.rust_trace_count = Some(9999);
        loud_report.rust_via_count = Some(9999);
        loud_report.rust_bend_count = Some(9999);

        // ...the gates STAY green (the report fields never enter them).
        assert_eq!(
            compare_directional(&java, &outcome),
            Vec::new(),
            "report-only faces must never gate"
        );

        // ...and the verdict line DOES change with them (they render) —
        // while the PASS/RED STATUS derives from the gates alone: even the
        // fully-mutated report keeps the clean run "[PASS]".
        let quiet = verdict_line("f.dsn", &[], &java, &outcome, &quiet_report, 1.0);
        let loud = verdict_line("f.dsn", &[], &java, &outcome, &loud_report, 1.0);
        assert!(quiet.starts_with("[PASS] f.dsn"), "{quiet}");
        assert!(
            loud.starts_with("[PASS] f.dsn"),
            "report-only mutations must never flip the status: {loud}"
        );
        assert!(quiet.contains("introduced 0/<0> passes 17/<18> traces 10 vias 2 bends 4"));
        assert!(loud.contains("introduced 50/<0>"));
        assert_ne!(quiet, loud, "report-only faces must still RENDER");

        // Contrast: a REAL gate failure flips the status on the same data.
        let red = verdict_line(
            "f.dsn",
            &[GateFailure::IncompleteNotBetter {
                java: Some(2),
                rust: Some(3),
            }],
            &java,
            &outcome,
            &quiet_report,
            1.0,
        );
        assert!(red.starts_with("[RED] f.dsn"), "{red}");
    }

    /// The record pre-flight: missing file, wrong engine, wrong/absent
    /// profile, wrong schema — each an explicit named error (the trap-2
    /// anti-skip); a good router-only record loads.
    #[test]
    fn record_pre_flight_rejects_missing_and_mismatched_records() {
        let dir = std::env::temp_dir().join(format!("epic-t15-preflight-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");

        let err = load_java_router_only_record(&dir.join("nope.json"))
            .expect_err("missing record must be an explicit error");
        assert!(err.contains("missing baseline record"), "{err}");

        let write = |name: &str, body: String| {
            let path = dir.join(name);
            std::fs::write(&path, body).expect("write record");
            path
        };
        let rust_record = write(
            "rust.json",
            r#"{"schema_version":1,"engine":"rust","fixture":"x.dsn","final_state":"COMPLETED","captured_at_unix":0}"#.into(),
        );
        let err = load_java_router_only_record(&rust_record).expect_err("rust record rejected");
        assert!(err.contains("engine rust"), "{err}");

        let no_profile = write(
            "noprofile.json",
            r#"{"schema_version":1,"engine":"java","fixture":"x.dsn","final_state":"COMPLETED","captured_at_unix":0}"#.into(),
        );
        let err =
            load_java_router_only_record(&no_profile).expect_err("profile-less record rejected");
        assert!(err.contains("profile"), "{err}");

        let full_flow = write(
            "fullflow.json",
            r#"{"schema_version":1,"engine":"java","fixture":"x.dsn","final_state":"COMPLETED","captured_at_unix":0,"profile":"full-flow"}"#.into(),
        );
        let err = load_java_router_only_record(&full_flow).expect_err("full-flow record rejected");
        assert!(err.contains("router-only"), "{err}");

        let good = write(
            "good.json",
            r#"{"schema_version":1,"engine":"java","fixture":"x.dsn","final_state":"COMPLETED","captured_at_unix":0,"profile":"router-only","incomplete_count":2,"normalized_score":986.32}"#.into(),
        );
        let record = load_java_router_only_record(&good).expect("good record loads");
        assert_eq!(record.incomplete_count, Some(2));
        assert_eq!(record.normalized_score, Some(986.32));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// THE fixture-enum completeness canary (real repo tree, java-free,
    /// cheap): Tier A in tiers.yaml order, every record present, and the
    /// committed record set has NO extras. A dropped fixture (the walk or
    /// the capture) is the finding-class trap 2 names — this pin dies on
    /// that mutant.
    #[test]
    fn tier_a_fixture_enum_is_complete_against_tiers_and_records() {
        let root = crate::oracle::find_repo_root().expect("repo root");
        let fixtures = tier_a_fixtures(&root).expect("tier A walk");
        assert_eq!(fixtures.len(), 11, "the committed Tier A battery");
        let expected_first = "DAC2020_boards/DAC2020_bm01.dsn";
        assert_eq!(fixtures[0].tier_path, expected_first, "tiers.yaml order");
        assert_eq!(
            fixtures.last().expect("non-empty").tier_path,
            "KiCad_10_demos/sonde xilinx.dsn",
            "the space-bearing path survives the walk verbatim"
        );
        for fixture in &fixtures {
            assert!(
                fixture.baseline_path.is_file(),
                "record missing for {}: {}",
                fixture.tier_path,
                fixture.baseline_path.display()
            );
            assert!(
                root.join(&fixture.repo_rel_dsn).is_file(),
                "fixture missing on disk: {}",
                fixture.repo_rel_dsn
            );
            assert!(
                fixture.timeout_seconds > 0,
                "the tiers.yaml timeout must ride"
            );
        }
        // No extras: every committed record maps back to a walked fixture.
        let walked: std::collections::BTreeSet<String> = fixtures
            .iter()
            .map(|f| format!("{}.baseline.json", f.tier_path))
            .collect();
        let mut extras = Vec::new();
        let baselines = router_only_baselines_dir(&root);
        for tier_dir in ["DAC2020_boards", "KiCad_10_demos"] {
            let dir = baselines.join(tier_dir);
            let entries = std::fs::read_dir(&dir).expect("record dir");
            for entry in entries {
                let name = entry
                    .expect("dir entry")
                    .file_name()
                    .into_string()
                    .expect("utf8");
                if name.ends_with(".baseline.json")
                    && !walked.contains(&format!("{tier_dir}/{name}"))
                {
                    extras.push(format!("{tier_dir}/{name}"));
                }
            }
        }
        assert!(
            extras.is_empty(),
            "records without a Tier A fixture: {extras:?}"
        );
    }

    /// THE localizer output shape (synthetic failing pair, no engine):
    /// the FIRST differing gate is named; per-net rows, violation pairs,
    /// the score decomposition, and the geometry counts all render; and
    /// the detail-less face degrades to subprocess evidence, never to
    /// silence. Kills the aggregates-only localizer mutant.
    #[test]
    fn localizer_prints_per_net_pairs_and_decomposition() {
        let java = java_record(2, 0, 986.32);
        let outcome = rust_outcome(5, 1, 900.0);
        let failures = compare_directional(&java, &outcome);
        assert_eq!(failures.len(), 3);
        let detail = RustDetail {
            incomplete_total: 5,
            incomplete_nets: vec![
                NetRow {
                    net_no: 3,
                    groups: 5,
                    incomplete_count: 2,
                },
                NetRow {
                    net_no: 7,
                    groups: 3,
                    incomplete_count: 1,
                },
            ],
            violations: vec![ViolationPair {
                a: 12,
                b: 34,
                layer: 1,
                expected_clearance: 200.0,
                actual_clearance: 100.0,
            }],
            terms: ScoreTerms {
                maximum: 1000.0,
                unrouted: 30.0,
                violation: 100.0,
                bend: 10.0,
                via: 200.0,
                trace_length: 100.0,
                normalized: 560.0,
            },
            trace_count: 40,
            via_count: 4,
            bend_count: 10,
        };
        let stderr = vec![
            "Info: Auto-routing pass #1 ...".to_string(),
            "thread 'main' panicked: Line only implemented for IntPoints till now".to_string(),
        ];

        let lines = localization_lines("x.dsn", &java, &outcome, &failures, Some(&detail), &stderr);
        let text = lines.join("\n");
        // First divergence named (incomplete is the first failing gate).
        assert!(
            lines[0].starts_with("localize[x.dsn]: FIRST DIVERGENCE — "),
            "{text}"
        );
        assert!(
            lines[0].contains("incomplete"),
            "the first differing gate: {text}"
        );
        // Per-net rows (the shape the plan names).
        assert!(text.contains("per-net incomplete (rust run):"));
        assert!(text.contains("net 3: incomplete 2 (groups 5)"));
        assert!(text.contains("net 7: incomplete 1 (groups 3)"));
        // Violation PAIRS.
        assert!(text.contains("item 12 <-> item 34 on layer 1"));
        assert!(text.contains("(actual 100.000 < expected 200.000)"));
        // Score DECOMPOSITION (every legacy term).
        assert!(text.contains("score decomposition (legacy 0-1000; maximum 1000.00)"));
        assert!(text.contains("unrouted penalty      -30.00"));
        assert!(text.contains("violation penalty     -100.00"));
        assert!(text.contains("bend penalty          -10.00"));
        assert!(text.contains("via costs             -200.00"));
        assert!(text.contains("trace-length costs    -100.00"));
        assert!(text.contains("=> score 560.00 vs java 986.32 (epsilon 19.73)"));
        // Geometry counts + the java-side absence note.
        assert!(text.contains("traces 40 vias 4 bends 10"));
        assert!(text.contains("no geometry counts"));

        // The degraded face (detail pass unavailable — the panic class):
        // subprocess evidence renders, never silence.
        let degraded = localization_lines("x.dsn", &java, &outcome, &failures, None, &stderr);
        let text = degraded.join("\n");
        assert!(text.contains("detail pass unavailable"), "{text}");
        assert!(text.contains("incomplete: rust 5 vs java Some(2)"));
    }

    /// THE determinism byte-check: identical digests pass with the OK
    /// line; a planted SES divergence and a planted manifest divergence
    /// each fail NAMING the artifact; missing output fails naming the
    /// absence. Kills the always-OK mutant.
    #[test]
    fn determinism_check_catches_a_planted_divergence() {
        let ok = determinism_verdict(
            Some("aaaa".into()),
            Some("aaaa".into()),
            Some("m1".into()),
            Some("m1".into()),
        )
        .expect("identical digests are OK");
        assert!(ok.contains("determinism: OK"), "{ok}");

        let err = determinism_verdict(
            Some("aaaa".into()),
            Some("bbbb".into()),
            Some("m1".into()),
            Some("m1".into()),
        )
        .expect_err("a planted SES divergence must fail");
        assert!(err.contains("SES"), "{err}");

        let err = determinism_verdict(
            Some("aaaa".into()),
            Some("aaaa".into()),
            Some("m1".into()),
            Some("m2".into()),
        )
        .expect_err("a planted manifest divergence must fail");
        assert!(err.contains("manifest"), "{err}");

        let err = determinism_verdict(None, None, Some("m1".into()), Some("m1".into()))
            .expect_err("missing output must fail (a panicking run writes no session)");
        assert!(err.contains("no session file"), "{err}");

        let err = determinism_verdict(
            Some("aaaa".into()),
            Some("aaaa".into()),
            None,
            Some("m1".into()),
        )
        .expect_err("a missing manifest must fail independently of the ses face");
        assert!(err.contains("no manifest"), "{err}");
    }

    /// The score decomposition terms sum to the engine face: with an
    /// all-Some world the recomputed normalized score equals BOTH the
    /// hand-computed 1000 − Σterms AND the engine's own
    /// `get_legacy_normalized_score` on the same stats.
    #[test]
    fn score_decomposition_terms_sum_to_the_engine_score() {
        use epic_router::pipeline::board_statistics::{
            BendsCounts, BoardStatistics, ConnectionsCounts, RoutingCostSettings, TracesCounts,
            ViasCounts,
        };
        let mut stats = BoardStatistics::new_empty();
        stats.connections = ConnectionsCounts {
            maximum_count: Some(100),
            incomplete_count: Some(3),
        };
        stats.traces = TracesCounts {
            total_count: 8,
            total_length: 500.0,
            total_length_mm: Some(500.0),
            average_length: 62.5,
        };
        stats.bends = BendsCounts {
            total_count: 10,
            ninety_degree_count: 2,
            forty_five_degree_count: 8,
            other_angle_count: 0,
        };
        stats.vias = ViasCounts {
            total_count: 4,
            through_hole_count: 4,
            blind_count: 0,
            buried_count: 0,
        };
        stats.clearance_violations.total_count = Some(2);
        let scoring = RoutingCostSettings {
            default_preferred_direction_trace_cost: Some(0.2),
            default_undesired_direction_trace_cost: Some(0.2),
            via_costs: Some(50),
            plane_via_costs: Some(5),
            start_ripup_costs: Some(1000),
            default_bend_cost: Some(1.0),
            unrouted_net_penalty: Some(10.0),
            clearance_violation_penalty: Some(100.0),
            bend_penalty: Some(1.0),
        };
        let terms = score_decomposition(&stats, &scoring);
        // maximum = 100 × 10 = 1000; the five normalized contributions:
        let approx = |a: f64, b: f64| (a - b).abs() < 1e-3;
        assert!(approx(terms.maximum, 1000.0), "{terms:?}");
        assert!(approx(terms.unrouted, 30.0), "3/100 × 1000: {terms:?}");
        assert!(
            approx(terms.violation, 200.0),
            "2×100/1000 × 1000: {terms:?}"
        );
        assert!(approx(terms.bend, 10.0), "{terms:?}");
        assert!(approx(terms.via, 200.0), "4×50/1000 × 1000: {terms:?}");
        assert!(
            approx(terms.trace_length, 100.0),
            "500×0.2/1000 × 1000: {terms:?}"
        );
        // 1000 − (30+200+10+200+100) = 460, matching the engine's own
        // legacy score on the same stats (the mutant that decomposes with
        // a wrong term cannot reproduce both faces).
        assert!(approx(terms.normalized, 460.0), "{terms:?}");
        let engine = f64::from(stats.get_legacy_normalized_score(&scoring));
        assert!(
            (terms.normalized - engine).abs() < 1e-3,
            "decomposition must reproduce the engine score: {terms:?} vs {engine}"
        );

        // The floor face: a world costing more than the maximum clamps at 0.
        stats.connections.incomplete_count = Some(100);
        let terms = score_decomposition(&stats, &scoring);
        assert!(approx(terms.normalized, 0.0), "floored: {terms:?}");
        let engine = f64::from(stats.get_legacy_normalized_score(&scoring));
        assert!(approx(engine, 0.0), "engine floors too: {engine}");
    }

    /// THE comparability argv pin (spec-review MINOR-1 / OW-M2 survivor):
    /// `route_argv` — the ONE builder `run_cli` spawns from — carries the
    /// fanout-off flag EXACTLY once, as the argv suffix, and the banner
    /// phrase derived from that argv claims fanout-off. Mutation-verified:
    /// dropping the flag from the builder makes BOTH asserts fail (and the
    /// banner face flips to "MISSING" instead of a hardcoded lie).
    #[test]
    fn run_cli_argv_carries_the_comparability_flag() {
        let argv = route_argv(
            Path::new("/bin/epic-cli"),
            Path::new("/f/x.dsn"),
            Path::new("/w/out.ses"),
            Path::new("/w/manifest.json"),
        );
        let flag_positions: Vec<usize> = argv
            .iter()
            .enumerate()
            .filter_map(|(i, arg)| (*arg == COMPARABILITY_FLAG).then_some(i))
            .collect();
        assert_eq!(
            flag_positions,
            vec![argv.len() - 1],
            "the comparability flag must ride EXACTLY once, as the argv suffix: {argv:?}"
        );
        // The banner face is DERIVED: a correct argv yields the claim.
        let phrase = comparability_banner_phrase(&argv);
        assert!(
            phrase.contains(COMPARABILITY_FLAG),
            "the banner must claim fanout-off from the actual argv: {phrase}"
        );
        // Degraded face: an argv WITHOUT the flag (the OW-M2 mutant) must
        // make the banner say MISSING — the log can never lie about the
        // comparability face.
        let mutant_argv: Vec<String> = argv
            .iter()
            .filter(|arg| *arg != COMPARABILITY_FLAG)
            .cloned()
            .collect();
        let phrase = comparability_banner_phrase(&mutant_argv);
        assert!(
            phrase.contains("COMPARABILITY FLAG MISSING"),
            "a flag-less argv must stop the banner claiming fanout-off: {phrase}"
        );
    }

    /// THE determinism digest-source pin (spec-review MINOR-2 / OW-M3
    /// survivor): the digests are computed from the pair's DISTINCT
    /// run1/run2 paths — planted-different bytes MUST produce different
    /// digests (and a failing verdict), planted-equal bytes MUST produce
    /// equal digests (and the OK verdict). Mutation-verified: feeding
    /// run1's files to BOTH sides makes the differing-bytes world produce
    /// equal digests and this pin fail.
    #[test]
    fn determinism_digests_read_distinct_run_paths() {
        let dir = std::env::temp_dir().join(format!("epic-t15-digest-pair-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("run1")).expect("run1 dir");
        std::fs::create_dir_all(dir.join("run2")).expect("run2 dir");
        // Planted-different bytes: run1 and run2 wrote DIFFERENT output.
        std::fs::write(dir.join("run1/out.ses"), b"session one").expect("ses1");
        std::fs::write(dir.join("run1/manifest.json"), b"{\"m\":1}").expect("man1");
        std::fs::write(dir.join("run2/out.ses"), b"session two").expect("ses2");
        std::fs::write(dir.join("run2/manifest.json"), b"{\"m\":2}").expect("man2");
        let pair = DeterminismPair {
            run1: DeterminismArtifacts {
                ses: dir.join("run1/out.ses"),
                manifest: dir.join("run1/manifest.json"),
            },
            run2: DeterminismArtifacts {
                ses: dir.join("run2/out.ses"),
                manifest: dir.join("run2/manifest.json"),
            },
        };
        // Structural distinctness first: the pair cannot alias.
        assert_ne!(pair.run1.ses, pair.run2.ses);
        assert_ne!(pair.run1.manifest, pair.run2.manifest);

        let (ses_1, ses_2, man_1, man_2) = determinism_digests(&pair).expect("digests");
        // Distinct bytes MUST hash differently — the OW-M3 mutant (run1's
        // files hashed for both sides) fails right here.
        assert_ne!(
            ses_1, ses_2,
            "run1/run2 SES bytes differ, so their digests must differ"
        );
        assert_ne!(
            man_1, man_2,
            "run1/run2 manifest bytes differ, so their digests must differ"
        );
        // And the verdict must catch it, naming the SES face.
        let err = determinism_verdict(ses_1.clone(), ses_2.clone(), man_1.clone(), man_2.clone())
            .expect_err("planted divergence must fail");
        assert!(err.contains("SES"), "{err}");

        // Planted-equal bytes: identical output in both runs is the OK face.
        std::fs::write(dir.join("run2/out.ses"), b"session one").expect("ses2 equal");
        std::fs::write(dir.join("run2/manifest.json"), b"{\"m\":1}").expect("man2 equal");
        let (ses_1, ses_2, man_1, man_2) = determinism_digests(&pair).expect("digests equal");
        assert_eq!(ses_1, ses_2, "equal bytes must hash equal");
        assert_eq!(man_1, man_2);
        determinism_verdict(ses_1, ses_2, man_1, man_2)
            .expect("equal digests from distinct paths are OK");

        // The missing-file face still flows through the pair: an absent
        // run2 SES digests to None and the verdict names the absence.
        std::fs::remove_file(dir.join("run2/out.ses")).expect("remove ses2");
        let (ses_1, ses_2, man_1, man_2) = determinism_digests(&pair).expect("digests partial");
        assert!(ses_1.is_some() && ses_2.is_none(), "{ses_1:?} {ses_2:?}");
        let err =
            determinism_verdict(ses_1, ses_2, man_1, man_2).expect_err("missing run2 ses fails");
        assert!(err.contains("no session file"), "{err}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
