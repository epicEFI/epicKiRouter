// consumed by M0 Task 5
#![allow(dead_code)]

//! Drives the Java engine as the parity oracle: builds the executable jar,
//! runs headless CLI jobs per fixture, and returns their manifests.
//! The Java tree is never modified (design §4.1).

use crate::manifest::RoutingResultManifest;
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Where results of one oracle invocation land.
#[derive(Debug)]
pub struct OracleRun {
    pub manifest: Option<RoutingResultManifest>,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub wall_seconds: f64,
    pub ses_path: PathBuf,
    pub manifest_path: PathBuf,
}

pub fn find_repo_root() -> Result<PathBuf> {
    let mut dir = std::env::current_dir().context("getting current directory")?;
    loop {
        if dir.join(".git").exists() && dir.join("build.gradle").exists() {
            return Ok(dir);
        }
        if !dir.pop() {
            bail!("could not locate repo root (no .git + build.gradle upward from cwd)");
        }
    }
}

/// The jar requires Java 25; the system default may be older, so prefer an
/// explicit override, then `~/.jdks/jdk-25*`, then PATH `java`.
pub fn resolve_java() -> Result<PathBuf> {
    if let Ok(explicit) = std::env::var("EPIC_ORACLE_JAVA") {
        return Ok(PathBuf::from(explicit));
    }
    if let Some(home) = std::env::var_os("HOME") {
        let jdk_root = Path::new(&home).join(".jdks");
        let mut candidates: Vec<PathBuf> = match std::fs::read_dir(&jdk_root) {
            Ok(entries) => entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with("jdk-25"))
                })
                .collect(),
            Err(_) => Vec::new(),
        };
        candidates.sort();
        if let Some(jdk) = candidates.first() {
            let java = jdk.join("bin").join("java");
            if java.is_file() {
                return Ok(java);
            }
        }
    }
    Ok(PathBuf::from("java"))
}

pub fn jar_path(repo_root: &Path) -> PathBuf {
    repo_root.join("build/libs/freerouting-current-executable.jar")
}

/// Builds the oracle jar via Gradle unless `EPIC_SKIP_GRADLE=1`.
pub fn build_jar(repo_root: &Path) -> Result<()> {
    if std::env::var("EPIC_SKIP_GRADLE").as_deref() == Ok("1") {
        return Ok(());
    }
    let status = Command::new("./gradlew")
        .arg("-q")
        .arg("executableJar")
        .current_dir(repo_root)
        .status()
        .context("running ./gradlew executableJar")?;
    if !status.success() {
        bail!("gradle executableJar failed with {status}");
    }
    Ok(())
}

/// Pure argv builder (unit-tested; no I/O). Paths may contain spaces —
/// callers pass this directly to `Command::args`, never through a shell.
pub fn oracle_argv(
    java: &Path,
    jar: &Path,
    jvm_xmx: &str,
    dsn: &Path,
    ses_out: &Path,
    manifest_out: &Path,
) -> Vec<String> {
    vec![
        java.to_string_lossy().into_owned(),
        format!("-Xmx{jvm_xmx}"),
        "-jar".into(),
        jar.to_string_lossy().into_owned(),
        "-de".into(),
        dsn.to_string_lossy().into_owned(),
        "-do".into(),
        ses_out.to_string_lossy().into_owned(),
        format!("--router.result_json={}", manifest_out.to_string_lossy()),
        "--usage_and_diagnostic_data.disable_analytics=true".into(),
        // Only the headless CLI path (`initializeCli`) honors
        // `--router.result_json`; with a display present, the GUI auto-start
        // batch flow would intercept `-de/-do` and never write the manifest.
        // Mirror scripts/benchmark/lib/BenchmarkRunner.ps1.
        "--gui.enabled=false".into(),
        "--api_server.enabled=false".into(),
    ]
}

/// Spawns one oracle run. `work_dir` receives `out.ses`, `manifest.json`,
/// `stdout.log`, `stderr.log`. Output goes to files (not pipes) so a chatty
/// JVM can never deadlock on a full pipe buffer.
pub fn run_oracle(
    java: &Path,
    jar: &Path,
    dsn: &Path,
    work_dir: &Path,
    timeout: Duration,
    jvm_xmx: &str,
) -> Result<OracleRun> {
    std::fs::create_dir_all(work_dir)
        .with_context(|| format!("creating work dir {}", work_dir.display()))?;
    let ses_path = work_dir.join("out.ses");
    let manifest_path = work_dir.join("manifest.json");
    let stdout_path = work_dir.join("stdout.log");
    let stderr_path = work_dir.join("stderr.log");

    let argv = oracle_argv(java, jar, jvm_xmx, dsn, &ses_path, &manifest_path);
    let (java_bin, rest) = argv.split_first().expect("argv is never empty");
    let stdout_file = std::fs::File::create(&stdout_path)
        .with_context(|| format!("creating {}", stdout_path.display()))?;
    let stderr_file = std::fs::File::create(&stderr_path)
        .with_context(|| format!("creating {}", stderr_path.display()))?;

    let started = Instant::now();
    let mut child = Command::new(java_bin)
        .args(rest)
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::from(stderr_file))
        .spawn()
        .with_context(|| format!("spawning oracle {}", java_bin))?;

    let (exit_code, timed_out) = wait_with_timeout(&mut child, timeout);
    let wall_seconds = started.elapsed().as_secs_f64();

    let manifest = if manifest_path.is_file() {
        serde_json::from_str(&std::fs::read_to_string(&manifest_path)?).ok()
    } else {
        None
    };

    Ok(OracleRun {
        manifest,
        exit_code,
        timed_out,
        wall_seconds,
        ses_path,
        manifest_path,
    })
}

fn wait_with_timeout(child: &mut std::process::Child, timeout: Duration) -> (Option<i32>, bool) {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return (status.code(), false),
            Ok(None) => {}
            Err(_) => return (None, false),
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return (None, true);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argv_order_matches_java_cli_contract() {
        let argv = oracle_argv(
            Path::new("/jdks/jdk-25/bin/java"),
            Path::new("/repo/build/libs/freerouting-current-executable.jar"),
            "4g",
            Path::new("/repo/fixtures/sonde xilinx.dsn"),
            Path::new("/tmp/w/out.ses"),
            Path::new("/tmp/w/manifest.json"),
        );
        assert_eq!(argv[0], "/jdks/jdk-25/bin/java");
        assert_eq!(argv[1], "-Xmx4g");
        assert_eq!(argv[2], "-jar");
        assert!(argv[3].ends_with("freerouting-current-executable.jar"));
        assert_eq!(argv[4], "-de");
        assert_eq!(argv[5], "/repo/fixtures/sonde xilinx.dsn");
        assert_eq!(argv[6], "-do");
        assert_eq!(argv[7], "/tmp/w/out.ses");
        assert_eq!(argv[8], "--router.result_json=/tmp/w/manifest.json");
        assert_eq!(
            argv[9],
            "--usage_and_diagnostic_data.disable_analytics=true"
        );
        assert_eq!(argv[10], "--gui.enabled=false");
        assert_eq!(argv[11], "--api_server.enabled=false");
        assert_eq!(argv.len(), 12);
    }

    /// End-to-end smoke on the tiniest fixture. Requires the jar + JDK 25;
    /// run explicitly with `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn oracle_routes_bm08_end_to_end() {
        let root = find_repo_root().expect("repo root");
        build_jar(&root).expect("building jar");
        let java = resolve_java().expect("resolving java");
        let jar = jar_path(&root);
        let dsn = root.join("scripts/benchmark/fixtures/DAC2020_boards/DAC2020_bm08.dsn");
        let work = std::env::temp_dir().join("epic-harness-bm08-smoke");
        let run = run_oracle(&java, &jar, &dsn, &work, Duration::from_secs(300), "4g")
            .expect("oracle run");
        let m = run.manifest.as_ref().expect("manifest must be written");
        assert_eq!(m.final_state, "COMPLETED");
        assert_eq!(m.exit_code, 0);
        assert!(run.ses_path.is_file());
        assert!(run.ses_path.metadata().expect("ses stat").len() > 0);
    }
}
