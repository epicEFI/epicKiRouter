//! Differential parity + benchmark harness for the EpicRouter rewrite
//! (design §5). M0 scope: drive the Java oracle engine and capture golden
//! baselines.

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::time::Duration;

mod baseline;
mod corpus;
// Canonical DSN parse digest (M1b Task 4); consumed by the `dsn`
// subcommands' goldens/compare since Task 11.
mod dsn_corpus;
mod dsn_digest;
// Index parity corpus (M2 Task 9): tier + stressor fixtures replayed
// through the landed search-tree surface, Java goldens, java-free CI
// compare.
mod index_corpus;
mod manifest;
mod oracle;
// SES emission-parity goldens + canonical compare (M1b Task 13 B3/B4).
mod ses_compare;
mod tiers;

use baseline::BaselineRecord;
use oracle::OracleRun;
use tiers::TierFile;

#[derive(Parser)]
#[command(name = "epic-harness", about = "EpicRouter parity + benchmark harness")]
struct Cli {
    /// Path to the tier file (relative to cwd or absolute).
    #[arg(long, global = true, default_value = "harness/config/tiers.yaml")]
    tiers: PathBuf,

    /// JVM -Xmx for oracle runs.
    #[arg(long, global = true, default_value = "4g")]
    jvm_xmx: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print the tier matrix and verify every fixture resolves on disk
    /// (exits nonzero, listing offenders, if any fixture is missing).
    List,
    /// Run the Java oracle on fixtures and write golden baselines.
    Capture {
        /// Only capture this tier (A/B/C); omit for all.
        #[arg(long)]
        tier: Option<String>,
    },
    /// Re-run the oracle and compare against committed baselines.
    Verify {
        /// Only verify this tier; omit for all.
        #[arg(long)]
        tier: Option<String>,
    },
    /// Geometry differential corpus (M1a): generate cases, capture Java
    /// goldens, compare epic-geometry against them.
    Corpus {
        #[command(subcommand)]
        cmd: corpus::CorpusCommand,
    },
    /// DSN parse-parity digest corpus (M1b Task 11): manifest generation,
    /// Java-oracle golden capture, java-free Rust comparison.
    Dsn {
        #[command(subcommand)]
        cmd: dsn_corpus::DsnCommand,
    },
    /// Index parity corpus (M2 Task 9): tree dumps + the replay query
    /// script on tier + stressor fixtures, Java-oracle goldens, and a
    /// java-free compare (the CI gate).
    Index {
        #[command(subcommand)]
        cmd: index_corpus::IndexCommand,
    },
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    if let Command::Corpus { cmd } = cli.command {
        // The corpus pipeline needs no tier file or eager JVM resolution.
        return corpus::run(cmd);
    }
    if let Command::Dsn { cmd } = cli.command {
        // The dsn pipeline resolves tiers.yaml itself (manifest
        // generation) and needs no eager JVM resolution.
        return dsn_corpus::run(cmd, &cli.jvm_xmx);
    }
    if let Command::Index { cmd } = cli.command {
        // The index pipeline reads tiers.yaml itself (manifest
        // generation) and resolves the JVM lazily — `index compare`
        // never needs one.
        return index_corpus::run(cmd, &cli.jvm_xmx);
    }
    let repo_root = oracle::find_repo_root()?;
    let tiers_path = resolve_tiers_path(&repo_root, &cli.tiers);
    let tier_file = TierFile::load(&tiers_path)?;
    let fixtures_root = repo_root.join(&tier_file.fixtures_root);
    let java = oracle::resolve_java()?;
    let jar = oracle::jar_path(&repo_root);

    match cli.command {
        // Index/Corpus/Dsn returned before the tier/JVM resolution
        // above; this arm only satisfies the match.
        Command::Index { .. } => unreachable!("handled before tier/JVM resolution"),
        Command::List => {
            let missing = list_tiers(&tier_file, &fixtures_root);
            if missing.is_empty() {
                Ok(())
            } else {
                bail!(
                    "{} fixture(s) missing on disk:\n  {}",
                    missing.len(),
                    missing.join("\n  ")
                );
            }
        }
        Command::Capture { tier } => {
            // Validate the selection before paying for a gradle build.
            let selected = select_tiers(&tier_file, tier.as_deref())?;
            oracle::build_jar(&repo_root)?;
            let baselines_root = tiers_path
                .parent()
                .and_then(Path::parent)
                .map(|p| p.join("baselines"))
                .unwrap_or_else(|| repo_root.join("rust/harness/baselines"));
            let runs_root = baselines_root
                .parent()
                .map(|p| p.join("runs"))
                .unwrap_or_else(|| repo_root.join("rust/harness/runs"));
            for tier in selected {
                capture_tier(
                    tier,
                    &fixtures_root,
                    &tier_file.fixtures_root,
                    &java,
                    &jar,
                    &baselines_root,
                    &runs_root,
                    &cli.jvm_xmx,
                )?;
            }
            Ok(())
        }
        Command::Verify { tier } => {
            // Validate the tier name before paying for the gradle build.
            let selected = select_tiers(&tier_file, tier.as_deref())?;
            let harness_root = tiers_path
                .parent()
                .and_then(Path::parent)
                .map(Path::to_path_buf)
                .unwrap_or_else(|| repo_root.join("rust/harness"));
            let baselines_root = harness_root.join("baselines").join("java");
            let run_root = harness_root.join("runs").join("verify");
            let fixtures_rel = &tier_file.fixtures_root;
            // Pre-flight: every selected fixture's baseline must exist, parse
            // strictly, and carry the current schema version — checked for all
            // tiers BEFORE the gradle build, with every offender tallied into
            // a single bail so one bad file doesn't hide the rest.
            let mut offenders: Vec<String> = Vec::new();
            let mut preflight: Vec<(&tiers::Tier, Vec<BaselineRecord>)> = Vec::new();
            for tier in &selected {
                let mut records = Vec::with_capacity(tier.fixtures.len());
                for fixture in &tier.fixtures {
                    let baseline_path = baselines_root
                        .join(&tier.name)
                        .join(format!("{}.baseline.json", fixture.path));
                    match load_baseline_record(&baseline_path) {
                        Ok(record) => records.push(record),
                        Err(reason) => {
                            offenders.push(format!("  {}/{}: {}", tier.name, fixture.path, reason))
                        }
                    }
                }
                preflight.push((*tier, records));
            }
            if !offenders.is_empty() {
                bail!("missing or invalid baselines:\n{}", offenders.join("\n"));
            }
            oracle::build_jar(&repo_root)?;
            let mut total_failures = 0usize;
            for (tier, records) in &preflight {
                for (fixture, expected) in tier.fixtures.iter().zip(records) {
                    let dsn = fixtures_root.join(&fixture.path);
                    let work_dir = run_root.join(&tier.name).join(&fixture.path);
                    println!("[tier {}] verifying {}…", tier.name, fixture.path);
                    let run = oracle::run_oracle(
                        &java,
                        &jar,
                        &dsn,
                        &work_dir,
                        Duration::from_secs(fixture.timeout_seconds),
                        &cli.jvm_xmx,
                    )
                    .with_context(|| {
                        format!(
                            "oracle run failed while verifying {} (tier {})",
                            fixture.path, tier.name
                        )
                    })?;
                    let actual = baseline::distill(
                        &fixtures_rel.join(&fixture.path).to_string_lossy(),
                        &run,
                    );
                    let failures = baseline::compare(expected, &actual);
                    if failures.is_empty() {
                        println!(
                            "  PASS (wall {:.1}s; baseline routing phases {:.1}s)",
                            run.wall_seconds,
                            expected.autorouter_seconds.unwrap_or(0.0)
                                + expected.optimizer_seconds.unwrap_or(0.0)
                        );
                    } else {
                        total_failures += failures.len();
                        for f in &failures {
                            println!("  FAIL {f:?}");
                        }
                    }
                }
            }
            if total_failures == 0 {
                println!("verify: all gates passed");
                Ok(())
            } else {
                bail!("verify: {total_failures} gate failure(s)");
            }
        }
        // Handled before tier loading; unreachable here.
        Command::Corpus { .. } => unreachable!("corpus command handled before tier loading"),
        Command::Dsn { .. } => unreachable!("dsn command handled before tier loading"),
    }
}

/// Prints the tier matrix (one `[ok]`/`[MISSING]` line per fixture) and
/// returns every fixture that does not resolve on disk, as `tier/path`
/// entries, so the caller can bail — the `list` doc promises verification,
/// not just reporting.
fn list_tiers(tier_file: &TierFile, fixtures_root: &Path) -> Vec<String> {
    let mut missing = Vec::new();
    for tier in &tier_file.tiers {
        println!("tier {} ({} fixtures):", tier.name, tier.fixtures.len());
        for f in &tier.fixtures {
            let p = fixtures_root.join(&f.path);
            let present = p.is_file();
            let status = if present { "ok" } else { "MISSING" };
            println!("  [{status}] {} (timeout {}s)", f.path, f.timeout_seconds);
            if !present {
                missing.push(format!("{}/{}", tier.name, f.path));
            }
        }
    }
    missing
}

fn resolve_tiers_path(repo_root: &Path, given: &Path) -> PathBuf {
    if given.is_absolute() {
        given.into()
    } else if given.is_file() {
        std::fs::canonicalize(given).unwrap_or_else(|_| given.into())
    } else {
        // Default value is relative to rust/harness; also support running
        // from rust/ or repo root.
        let candidates = [repo_root.join("rust").join(given), repo_root.join(given)];
        candidates
            .into_iter()
            .find(|p| p.is_file())
            .unwrap_or_else(|| repo_root.join("rust").join(given))
    }
}

fn select_tiers<'a>(file: &'a TierFile, filter: Option<&str>) -> Result<Vec<&'a tiers::Tier>> {
    match filter {
        Some(name) => {
            let t = file
                .tier(name)
                .with_context(|| format!("tier {name} not found in tier file"))?;
            Ok(vec![t])
        }
        None => Ok(file.tiers.iter().collect()),
    }
}

/// Loads and validates one committed baseline for verify's pre-flight pass:
/// exists, parses strictly (unknown fields rejected), schema version current,
/// engine is `java`. The error is a plain `String` (not anyhow) so the caller
/// can tally every offender across tiers into a single bail listing all of
/// them.
fn load_baseline_record(path: &Path) -> std::result::Result<BaselineRecord, String> {
    if !path.is_file() {
        return Err("missing baseline — run capture first".into());
    }
    let raw = std::fs::read_to_string(path).map_err(|e| format!("unreadable: {e}"))?;
    let record: BaselineRecord =
        serde_json::from_str(&raw).map_err(|e| format!("invalid baseline JSON: {e}"))?;
    if record.schema_version != baseline::BASELINE_SCHEMA_VERSION {
        return Err(format!(
            "schema version {} (harness expects {}) — recapture needed",
            record.schema_version,
            baseline::BASELINE_SCHEMA_VERSION
        ));
    }
    // Verify re-runs the Java oracle; gating an M3+ rust record against a
    // java run (or any future engine mismatch) must fail pre-flight, not
    // produce misleading parity verdicts.
    if record.engine != "java" {
        return Err(format!(
            "engine {} — verify re-runs the Java oracle; expected \"java\" records",
            record.engine
        ));
    }
    Ok(record)
}

#[allow(clippy::too_many_arguments)]
fn capture_tier(
    tier: &tiers::Tier,
    fixtures_root: &Path,
    fixtures_root_raw: &Path,
    java: &Path,
    jar: &Path,
    baselines_root: &Path,
    runs_root: &Path,
    jvm_xmx: &str,
) -> Result<()> {
    let mut infra_suspects: Vec<String> = Vec::new();
    let mut degraded: Vec<String> = Vec::new();
    // `captured` = fixtures whose records were written before this one; the
    // mid-loop bail contexts report it so a failure never loses progress.
    for (captured, fixture) in tier.fixtures.iter().enumerate() {
        let dsn = fixtures_root.join(&fixture.path);
        if !dsn.is_file() {
            bail!(
                "fixture missing: {} (captured {}/{} fixtures in tier {} before failure)",
                dsn.display(),
                captured,
                tier.fixtures.len(),
                tier.name
            );
        }
        let work_dir = runs_root.join(&tier.name).join(&fixture.path);
        println!(
            "[tier {}] routing {} (timeout {}s)…",
            tier.name, fixture.path, fixture.timeout_seconds
        );
        let run: OracleRun = oracle::run_oracle(
            java,
            jar,
            &dsn,
            &work_dir,
            Duration::from_secs(fixture.timeout_seconds),
            jvm_xmx,
        )
        .with_context(|| {
            format!(
                "oracle run failed for {} (captured {}/{} fixtures in tier {} before failure)",
                fixture.path,
                captured,
                tier.fixtures.len(),
                tier.name
            )
        })?;
        // The record's `fixture` field is repo-relative (resolvable without
        // tiers.yaml); the raw yaml fixtures_root is joined, not the
        // absolute one used for I/O.
        let record = baseline::distill(
            &fixtures_root_raw.join(&fixture.path).to_string_lossy(),
            &run,
        );
        let out = baselines_root
            .join("java")
            .join(&tier.name)
            .join(format!("{}.baseline.json", fixture.path));
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(&out, serde_json::to_string_pretty(&record)?)
            .with_context(|| format!("writing {}", out.display()))?;
        println!(
            "  -> {} state={} incomplete={:?} violations={:?} score={:?} ({:.1}s)",
            out.display(),
            record.final_state,
            record.incomplete_count,
            record.clearance_violations_total,
            record.normalized_score,
            run.wall_seconds
        );
        // No manifest without a timeout is never a legitimate baseline
        // producer: exit-0-no-manifest is the GUI-interception / flag-drift
        // signature (SES written, clean exit, nothing distilled), and
        // nonzero-no-manifest smells like a broken environment (wrong JDK,
        // bad -Xmx). The record is still written — honesty preserved — but
        // capture must not quietly succeed, or Task 6 could verify against a
        // consistently broken or intercepted environment.
        if run.manifest.is_none() && !run.timed_out {
            infra_suspects.push(fixture.path.clone());
        }
        // A timeout or any note means a degraded run. The baseline is still
        // written (a timeout IS valid data), but a HARNESS_TIMED_OUT record
        // committed unacknowledged makes verify near-gate-free: None==None
        // passes compare.
        if record.final_state == "HARNESS_TIMED_OUT" || record.notes.is_some() {
            degraded.push(fixture.path.clone());
        }
    }
    if !infra_suspects.is_empty() {
        eprintln!(
            "WARNING: {} fixture(s) in tier {} produced no manifest without timing out — infra failure suspected; see runs/<tier>/<fixture>/stderr.log (affected: {})",
            infra_suspects.len(),
            tier.name,
            infra_suspects.join(", ")
        );
    }
    if !degraded.is_empty() {
        eprintln!(
            "WARNING: {} fixture(s) in tier {} timed out or carry notes — records were written, but committing them unacknowledged would make verify near-gate-free (affected: {})",
            degraded.len(),
            tier.name,
            degraded.join(", ")
        );
    }
    if !infra_suspects.is_empty() || !degraded.is_empty() {
        bail!(
            "tier {} capture requires acknowledgement before commit — infra-suspect (no manifest, no timeout): [{}]; timed-out or noted: [{}]",
            tier.name,
            infra_suspects.join(", "),
            degraded.join(", ")
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tiers::{FixtureRef, Tier};

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn list_tiers_returns_missing_fixtures_after_printing_all_rows() {
        let root = temp_dir("epic-harness-list-test");
        let dir = root.join("real");
        std::fs::create_dir_all(&dir).expect("fixture dir");
        std::fs::write(dir.join("good.dsn"), b"").expect("fixture file");
        let tier_file = TierFile {
            fixtures_root: PathBuf::new(), // unused by list_tiers
            tiers: vec![
                Tier {
                    name: "A".into(),
                    fixtures: vec![FixtureRef {
                        path: "real/good.dsn".into(),
                        timeout_seconds: 60,
                    }],
                },
                Tier {
                    name: "B".into(),
                    fixtures: vec![FixtureRef {
                        path: "real/gone.dsn".into(),
                        timeout_seconds: 60,
                    }],
                },
            ],
        };
        assert_eq!(list_tiers(&tier_file, &root), vec!["B/real/gone.dsn"]);
    }

    #[test]
    fn baseline_pre_flight_rejects_missing_non_java_and_accepts_java() {
        let dir = temp_dir("epic-harness-preflight-test");

        let err = load_baseline_record(&dir.join("nope.json"))
            .expect_err("missing baseline must fail pre-flight");
        assert!(err.contains("missing baseline"), "unexpected error: {err}");

        let rust_path = dir.join("rust.baseline.json");
        std::fs::write(
            &rust_path,
            r#"{"schema_version":1,"engine":"rust","fixture":"x.dsn","final_state":"COMPLETED","captured_at_unix":0}"#,
        )
        .expect("write baseline");
        let err = load_baseline_record(&rust_path)
            .expect_err("rust record must not be gated against a java run");
        assert!(
            err.contains("engine rust") && err.contains("Java oracle"),
            "unexpected error: {err}"
        );

        let java_path = dir.join("java.baseline.json");
        std::fs::write(
            &java_path,
            r#"{"schema_version":1,"engine":"java","fixture":"x.dsn","final_state":"COMPLETED","captured_at_unix":0}"#,
        )
        .expect("write baseline");
        let record = load_baseline_record(&java_path).expect("java record passes pre-flight");
        assert_eq!(record.engine, "java");
    }
}
