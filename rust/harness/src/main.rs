//! Differential parity + benchmark harness for the EpicRouter rewrite
//! (design §5). M0 scope: drive the Java oracle engine and capture golden
//! baselines.

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::time::Duration;

mod baseline;
mod manifest;
mod oracle;
mod tiers;

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
    /// Print the tier matrix and verify every fixture resolves on disk.
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
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let repo_root = oracle::find_repo_root()?;
    let tiers_path = resolve_tiers_path(&repo_root, &cli.tiers);
    let tier_file = TierFile::load(&tiers_path)?;
    let fixtures_root = repo_root.join(&tier_file.fixtures_root);
    let java = oracle::resolve_java()?;
    let jar = oracle::jar_path(&repo_root);

    match cli.command {
        Command::List => {
            for tier in &tier_file.tiers {
                println!("tier {} ({} fixtures):", tier.name, tier.fixtures.len());
                for f in &tier.fixtures {
                    let p = fixtures_root.join(&f.path);
                    let status = if p.is_file() { "ok" } else { "MISSING" };
                    println!("  [{status}] {} (timeout {}s)", f.path, f.timeout_seconds);
                }
            }
            Ok(())
        }
        Command::Capture { tier } => {
            oracle::build_jar(&repo_root)?;
            let baselines_root = tiers_path
                .parent()
                .and_then(Path::parent)
                .map(|p| p.join("baselines"))
                .unwrap_or_else(|| PathBuf::from("baselines"));
            let selected = select_tiers(&tier_file, tier.as_deref())?;
            for tier in selected {
                capture_tier(
                    tier,
                    &fixtures_root,
                    &java,
                    &jar,
                    &baselines_root,
                    &cli.jvm_xmx,
                )?;
            }
            Ok(())
        }
        Command::Verify { .. } => {
            bail!("verify is implemented in Task 6");
        }
    }
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

#[allow(clippy::too_many_arguments)]
fn capture_tier(
    tier: &tiers::Tier,
    fixtures_root: &Path,
    java: &Path,
    jar: &Path,
    baselines_root: &Path,
    jvm_xmx: &str,
) -> Result<()> {
    let run_root = baselines_root
        .parent()
        .map(|p| p.join("runs"))
        .unwrap_or_else(|| PathBuf::from("runs"));
    for fixture in &tier.fixtures {
        let dsn = fixtures_root.join(&fixture.path);
        if !dsn.is_file() {
            bail!("fixture missing: {}", dsn.display());
        }
        let work_dir = run_root.join(&tier.name).join(&fixture.path);
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
        )?;
        let record = baseline::distill(&fixture.path, &run);
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
    }
    Ok(())
}
