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
//! golden — a drift in the map, the guides, the ordering, or the
//! engine's settings-ON route rotates the digest and fails loudly.
//! FACE COVERAGE (C5-1, corrected at M6-T8): a face only exercises the
//! code its FLAGS gate — the master-only face never executes
//! pattern.rs or the pathfinder scheduler; drift in THOSE does not
//! rotate a master-face golden. The `--face` selector (M6-T8) grows the
//! population: `pattern` and `pathfinder` faces run their sub-flags,
//! and `verify` asserts the golden's `flags` against the flags the face
//! actually runs BEFORE comparing digests (a golden captured under one
//! face can never silently verify under another).
//!
//! Exit-printing by construction (the T6-close rider): the command
//! prints its own invocation echo and ends with an `exit=N` line.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::Subcommand;

use crate::baseline::{normalized_manifest_sha256, sha256_file};
use crate::oracle::{find_repo_root, wait_with_timeout};
use crate::router_compare::resolve_fresh_epic_cli;

/// The named settings-ON faces (M6-T8): the flag vector each golden
/// face runs. `master` keeps the T7 vector byte-for-byte (the
/// committed `t7_ripup.global-golden.json` verifies unchanged). The
/// empty string in the master row is dropped by [`face_flags`].
pub const GOLDEN_FACES: [(&str, [&str; 3]); 7] = [
    // M7-T6: the DIFFERENTIAL-PAIR faces — the declared pair routes
    // coupled (leader-first pass order + the follower's maze coupling
    // preference + the post-routing match stage) and the manifest
    // carries the advisory `pair_report` rows. The declaration is the
    // fixture's own net pair (`pa:pb` — the face is fixture-bound, the
    // table's fixed flag-vector shape). The goldens pin the settings-ON
    // faces byte-for-byte; the default (no `router.tuning.pairs`) face
    // of the same fixtures stays covered by the recurring compares'
    // byte-invariance (the pair face is inert at default).
    ("pair_coupled", ["--router.tuning.pairs=pa:pb", "", ""]),
    ("pair_split", ["--router.tuning.pairs=pa:pb", "", ""]),
    // M7-T4: the FANOUT-FULL-FLOW face (the M6 carry-forward — the
    // fanout-full-flow GATE GAP): NO flags — the default pipeline with
    // the fanout stage LIVE. This is the first committed byte golden
    // that exercises the changed fanout arm (the events fixtures are
    // capture replays; the router-only/ses/det gates run
    // fanout-disabled — the buglog-181 fix carried zero byte-invariance
    // coverage until this face exists). Capture it ONLY on a fixture
    // whose fanout stage actually fires (SMD pins needing fanout > 0 —
    // DAC2020_bm08: 30 of 36).
    ("fanout_full_flow", ["", "", ""]),
    ("master", ["--router.congestion_global=on", "", ""]),
    (
        "pattern",
        [
            "--router.congestion_global=on",
            "--router.congestion_global.pattern=on",
            // The fanout stage escapes every SMD pin with a via, so the
            // two-pin pattern eligibility can never fire in the full-flow
            // face (the M-PAT mutant round proved byte-identity there).
            // The face disables fanout — the fast path's reachable regime,
            // and the deviation is named in the task report.
            "--router.fanout.enabled=false",
        ],
    ),
    (
        "pathfinder",
        [
            "--router.congestion_global=on",
            "--router.congestion_global.pathfinder=on",
            "",
        ],
    ),
    (
        // M6-T9: the push-and-shove insertion face — a TOP-LEVEL flag
        // (no master; the family pattern's own flag). The waiver only
        // rotates bytes where the M3 shove probe answers shoved=true on
        // an obstacle room the maze would otherwise rip — proven firing
        // on the t7_ripup world BEFORE capture (the T8 deviation-2
        // lesson: ON/OFF SES digests differ there).
        "push_shove",
        ["--router.push_shove=on", "", ""],
    ),
];

/// The flag vector of a named face (None = unknown face name).
#[must_use]
pub fn face_flags(face: &str) -> Option<Vec<String>> {
    GOLDEN_FACES
        .iter()
        .find(|(name, _)| *name == face)
        .map(|(_, flags)| {
            flags
                .iter()
                .filter(|f| !f.is_empty())
                .map(|f| (*f).to_string())
                .collect()
        })
}

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
        /// The named settings-ON face (`master` | `pattern` |
        /// `pathfinder`; default `master` — the T7 capture face).
        #[arg(long, default_value = "master")]
        face: String,
        /// Work-root override (default `rust/harness/runs/global-golden`).
        #[arg(long)]
        work_root: Option<PathBuf>,
        /// Per-run timeout in seconds (default 120; raise for fixtures
        /// whose settings-ON face routes longer).
        #[arg(long, default_value_t = 120)]
        timeout_secs: u64,
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
        /// The named settings-ON face the golden was captured under
        /// (default `master`); asserted against the golden's `flags`
        /// BEFORE the digest compare (C5-1).
        #[arg(long, default_value = "master")]
        face: String,
        /// Work-root override.
        #[arg(long)]
        work_root: Option<PathBuf>,
        /// Per-run timeout in seconds (default 120).
        #[arg(long, default_value_t = 120)]
        timeout_secs: u64,
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
fn run_face(
    cli_bin: &Path,
    dsn: &Path,
    work_dir: &Path,
    timeout: Duration,
    flags: &[String],
) -> Result<RunDigests> {
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
    for flag in flags {
        argv.push(flag.clone());
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
    // The manifest digest is the VERSION-BLIND face (the M10-T5
    // handoff): a release bump can never rotate a committed
    // global-golden digest; the artifact bytes still carry the true
    // version. The SES digest stays raw (byte-identity gate).
    let manifest_digest = |path: &Path| -> Result<Option<String>> {
        path.is_file()
            .then(|| normalized_manifest_sha256(path))
            .transpose()
    };
    Ok(RunDigests {
        ses_sha: digest(&ses_path)?,
        manifest_sha: manifest_digest(&manifest_path)?,
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
    let (dsn_rel, out_path, golden_flag, face, work_root, timeout_secs, capture) = match cmd {
        GlobalGoldenCommand::Capture {
            dsn,
            out,
            face,
            work_root,
            timeout_secs,
        } => (dsn, Some(out), None, face, work_root, timeout_secs, true),
        GlobalGoldenCommand::Verify {
            dsn,
            golden,
            face,
            work_root,
            timeout_secs,
        } => (
            dsn,
            None,
            Some(golden),
            face,
            work_root,
            timeout_secs,
            false,
        ),
    };
    let flags = face_flags(&face).ok_or_else(|| {
        anyhow::anyhow!("unknown face '{face}' (master | pattern | pathfinder | push_shove)")
    })?;
    let dsn = if dsn_rel.is_absolute() {
        dsn_rel
    } else {
        repo_root.join(&dsn_rel)
    };
    // Buglog 201's path-joining asymmetry, CLOSED (the first-harness-
    // touch bank): `--out` (capture) and `--golden` (verify) now join
    // the repo root exactly like `--dsn` and `--work-root` — a
    // repo-root-relative path from any CWD. (Previously `--out` was
    // used AS-GIVEN: a `rust/`-CWD capture with `--out
    // rust/harness/...` wrote `rust/rust/harness/...`.)
    let out_path = out_path.map(|p| {
        if p.is_absolute() {
            p
        } else {
            repo_root.join(p)
        }
    });
    let golden_flag = golden_flag.map(|p| {
        if p.is_absolute() {
            p
        } else {
            repo_root.join(p)
        }
    });
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
        let run1 = run_face(
            &cli_bin,
            &dsn,
            &work.join("run1"),
            Duration::from_secs(timeout_secs),
            &flags,
        )?;
        let run2 = run_face(
            &cli_bin,
            &dsn,
            &work.join("run2"),
            Duration::from_secs(timeout_secs),
            &flags,
        )?;
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
            flags: flags.clone(),
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
        // C5-1: the golden's recorded flags must equal the flags the
        // face actually runs — a golden captured under one face can
        // never silently verify under another.
        if golden.flags != flags {
            bail!(
                "the golden's flags {:?} do not match the '{face}' face's flags {:?}",
                golden.flags,
                flags
            );
        }
        let run1 = run_face(
            &cli_bin,
            &dsn,
            &work.join("verify"),
            Duration::from_secs(timeout_secs),
            &flags,
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
