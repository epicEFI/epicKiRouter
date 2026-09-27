//! The M6-T7 settings-ON golden face (`epic-harness global-golden`) —
//! capture/verify of the `router.congestion_global=on` route of a
//! crafted fixture (beyond-Java: no Java oracle exists; the parity
//! claim is the default-off byte-identity of the recurring gates, and
//! THIS gate pins the settings-ON faces against drift).
//!
//! The golden holds the DSN digest, the flag vector, and the SES +
//! manifest digests of the settings-ON route (the manifest carries the
//! `global_plan` face — map digest, guides, planned order). Two runs
//! must agree byte-for-byte at capture time (the determinism
//! self-gate), and `verify` re-runs and compares against the committed
//! golden — a drift in the map, the guides, the ordering, the pattern
//! fast path, or the engine's settings-ON route rotates the digest and
//! fails loudly.
//!
//! Exit-printing by construction (the T6-close rider): the command
//! prints its own invocation echo and ends with an `exit=N` line.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::Subcommand;

use crate::baseline::sha256_file;
use crate::oracle::{find_repo_root, wait_with_timeout};
use crate::router_compare::resolve_fresh_epic_cli;

/// The extra argv the settings-ON face runs under (the harness never
/// passes Java-parity flags; these are Rust-only).
pub const CONGESTION_GLOBAL_FLAGS: [&str; 1] = ["--router.congestion_global=on"];

/// The golden schema (v1): digest face only — no wall clocks.
const GOLDEN_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Subcommand)]
pub enum GlobalGoldenCommand {
    /// Run the settings-ON face twice (determinism self-gate), then
    /// write the golden JSON.
    Capture {
        /// The DSN to route (tiers.yaml-style path relative to the
        /// repo root).
        #[arg(long)]
        dsn: PathBuf,
        /// Where the golden JSON is written.
        #[arg(long)]
        out: PathBuf,
        /// Work-root override (default `rust/harness/runs/global-golden`).
        #[arg(long)]
        work_root: Option<PathBuf>,
    },
    /// Run the settings-ON face once and compare against the committed
    /// golden (exit nonzero on any drift).
    Verify {
        /// The DSN to route.
        #[arg(long)]
        dsn: PathBuf,
        /// The committed golden JSON.
        #[arg(long)]
        golden: PathBuf,
        /// Work-root override.
        #[arg(long)]
        work_root: Option<PathBuf>,
    },
}

/// One settings-ON run's digests (None where the run wrote nothing).
#[derive(Debug)]
struct RunDigests {
    ses_sha: Option<String>,
    manifest_sha: Option<String>,
}

impl RunDigests {
    fn both_present(&self) -> bool {
        self.ses_sha.is_some() && self.manifest_sha.is_some()
    }
}

/// The golden record (serde, JSON).
#[derive(serde::Serialize, serde::Deserialize)]
struct GlobalGolden {
    schema_version: u32,
    /// The DSN file name.
    dsn: String,
    /// The input DSN's SHA-256.
    dsn_sha256: String,
    /// The exact router flags the face runs under.
    flags: Vec<String>,
    /// The settings-ON SES SHA-256.
    ses_sha256: String,
    /// The settings-ON manifest SHA-256 (carries the `global_plan`).
    manifest_sha256: String,
}

/// Prints the invocation echo line (the exact argv, single line).
fn echo_invocation(argv: &[String]) {
    println!("# Invocation: {}", argv.join(" "));
}

/// Runs the settings-ON face once into `work_dir`.
fn run_face(cli_bin: &Path, dsn: &Path, work_dir: &Path, timeout: Duration) -> Result<RunDigests> {
    let ses_path = work_dir.join("out.ses");
    let manifest_path = work_dir.join("manifest.json");
    std::fs::create_dir_all(work_dir)
        .with_context(|| format!("creating {}", work_dir.display()))?;
    let _ = std::fs::remove_file(&ses_path);
    let _ = std::fs::remove_file(&manifest_path);

    let mut argv: Vec<String> = [
        cli_bin.to_string_lossy().into_owned(),
        "route".to_string(),
        "-de".to_string(),
        dsn.to_string_lossy().into_owned(),
        "-do".to_string(),
        ses_path.to_string_lossy().into_owned(),
        "--result-json".to_string(),
        manifest_path.to_string_lossy().into_owned(),
    ]
    .to_vec();
    for flag in CONGESTION_GLOBAL_FLAGS {
        argv.push((*flag).to_string());
    }
    echo_invocation(&argv);

    let stdout_file = std::fs::File::create(work_dir.join("stdout.log"))?;
    let stderr_file = std::fs::File::create(work_dir.join("stderr.log"))?;
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        .stdout(std::process::Stdio::from(stdout_file))
        .stderr(std::process::Stdio::from(stderr_file))
        .spawn()
        .with_context(|| format!("spawning {}", cli_bin.display()))?;
    let (exit_code, timed_out) = wait_with_timeout(&mut child, timeout);
    if timed_out {
        bail!("the settings-ON run timed out after {timeout:?}");
    }
    if exit_code != Some(0) {
        bail!("the settings-ON run failed with exit {exit_code:?}");
    }
    let digest = |path: &Path| -> Result<Option<String>> {
        path.is_file().then(|| sha256_file(path)).transpose()
    };
    Ok(RunDigests {
        ses_sha: digest(&ses_path)?,
        manifest_sha: digest(&manifest_path)?,
    })
}

/// Runs the command (the exit-printing wrapper: any failure still
/// ends with an `exit=1` line before the error surfaces).
pub fn run(cmd: GlobalGoldenCommand) -> Result<()> {
    let result = run_inner(cmd);
    if result.is_err() {
        println!("exit=1");
    }
    result
}

/// Runs the command.
fn run_inner(cmd: GlobalGoldenCommand) -> Result<()> {
    let repo_root = find_repo_root()?;
    let (dsn_rel, out_path, golden_flag, work_root, capture) = match cmd {
        GlobalGoldenCommand::Capture {
            dsn,
            out,
            work_root,
        } => (dsn, Some(out), None, work_root, true),
        GlobalGoldenCommand::Verify {
            dsn,
            golden,
            work_root,
        } => (dsn, None, Some(golden), work_root, false),
    };
    let dsn = if dsn_rel.is_absolute() {
        dsn_rel
    } else {
        repo_root.join(&dsn_rel)
    };
    let work = work_root
        .map(|p| {
            if p.is_absolute() {
                p
            } else {
                repo_root.join(p)
            }
        })
        .unwrap_or_else(|| repo_root.join("rust/harness/runs/global-golden"));
    let cli_bin = resolve_fresh_epic_cli(&repo_root)?;
    let dsn_sha = sha256_file(&dsn).with_context(|| format!("hashing {}", dsn.display()))?;

    let exit_code = if capture {
        let run1 = run_face(&cli_bin, &dsn, &work.join("run1"), Duration::from_secs(120))?;
        let run2 = run_face(&cli_bin, &dsn, &work.join("run2"), Duration::from_secs(120))?;
        if !run1.both_present() || !run2.both_present() {
            bail!("a run produced no SES/manifest (run1={run1:?} run2={run2:?})");
        }
        if run1.ses_sha != run2.ses_sha || run1.manifest_sha != run2.manifest_sha {
            bail!(
                "the two settings-ON runs diverge (ses {:?} vs {:?}, manifest {:?} vs {:?})",
                run1.ses_sha,
                run2.ses_sha,
                run1.manifest_sha,
                run2.manifest_sha
            );
        }
        let golden = GlobalGolden {
            schema_version: GOLDEN_SCHEMA_VERSION,
            dsn: dsn
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("fixture.dsn")
                .to_string(),
            dsn_sha256: dsn_sha,
            flags: CONGESTION_GLOBAL_FLAGS
                .iter()
                .map(|f| (*f).to_string())
                .collect(),
            ses_sha256: run1.ses_sha.expect("checked above"),
            manifest_sha256: run1.manifest_sha.expect("checked above"),
        };
        let out = out_path.expect("capture face");
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&out, serde_json::to_string_pretty(&golden)?)?;
        println!(
            "global-golden capture: wrote {} (ses {}, manifest {})",
            out.display(),
            golden.ses_sha256,
            golden.manifest_sha256
        );
        println!("exit=0");
        0
    } else {
        let golden_flag = golden_flag.expect("verify face");
        let golden_path = if golden_flag.is_absolute() {
            golden_flag
        } else {
            repo_root.join(&golden_flag)
        };
        let raw = std::fs::read_to_string(&golden_path)
            .with_context(|| format!("reading {}", golden_path.display()))?;
        let golden: GlobalGolden = serde_json::from_str(&raw)
            .with_context(|| format!("parsing {}", golden_path.display()))?;
        if golden.dsn_sha256 != dsn_sha {
            bail!(
                "the DSN changed since capture (golden {dsn_sha:?} vs current {:?})",
                sha256_file(&dsn).ok()
            );
        }
        let run1 = run_face(
            &cli_bin,
            &dsn,
            &work.join("verify"),
            Duration::from_secs(120),
        )?;
        if !run1.both_present() {
            bail!("the run produced no SES/manifest (run1={run1:?})");
        }
        if run1.ses_sha.as_deref() != Some(golden.ses_sha256.as_str()) {
            bail!(
                "settings-ON SES drifted: golden {:?} vs current {:?}",
                golden.ses_sha256,
                run1.ses_sha
            );
        }
        if run1.manifest_sha.as_deref() != Some(golden.manifest_sha256.as_str()) {
            bail!(
                "settings-ON manifest drifted (the global_plan face included): golden {:?} vs current {:?}",
                golden.manifest_sha256,
                run1.manifest_sha
            );
        }
        println!(
            "global-golden verify: OK (ses {}, manifest {} match the committed golden)",
            run1.ses_sha.clone().unwrap_or_default(),
            run1.manifest_sha.clone().unwrap_or_default()
        );
        println!("exit=0");
        0
    };
    let _ = exit_code;
    Ok(())
}
