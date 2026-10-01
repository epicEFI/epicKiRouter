// consumed by M0 Task 5
#![allow(dead_code)]

//! Drives the Java engine as the parity oracle: builds the executable jar,
//! runs headless CLI jobs per fixture, and returns their manifests.
//! The Java tree was never modified and is now deleted (design §4.1;
//! sunset M10-T4).
//!
//! **SUNSET (M10-T4, 2026-09-30): the Java tree is deleted — the capture
//! modes are UNAVAILABLE post-sunset.** `build_jar`/`run_oracle` (gradle +
//! JDK 25) and every harness subcommand of the `golden`/capture family
//! (`dsn golden`, `ses golden`, `ses-snap golden`, `index golden`,
//! `undo golden`, `drc golden`, `events golden`, geometry `golden`,
//! `oracle build`/`oracle run`) fail cleanly when `./gradlew` or `src/`
//! is absent. The last executable jar can be rebuilt only from the
//! PRE-SUNSET commit (`9c967a937^`, i.e. `62a68a8e5`) — `git checkout
//! 62a68a8e5` + `./gradlew executableJar` under JDK 25. The committed
//! goldens (`rust/harness/baselines/**`, the corpus dirs, events-golden,
//! `config/tiers.yaml`) ARE the parity record from here on; the COMPARE
//! faces are java-free and unaffected (they read the committed goldens
//! and re-run the Rust engine).

use crate::manifest::RoutingResultManifest;
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Which stages the Java oracle runs (M3-T14) and where its baselines live.
/// The disable flags go to the JAVA jar argv for the capture/verify faces;
/// the RUST compare runs mirror them since M4-T9 (the route subset parses
/// `--optimizer.enabled` and [`route_argv`] carries
/// `--optimizer.enabled=false` next to the fanout flag — the committed
/// router-only records are the both-stages-OFF face on either side).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OracleProfile {
    /// Full pipeline (fanout + autorouter + optimizer): the classic
    /// `baselines/java/` captures.
    FullFlow,
    /// Fanout and optimizer disabled; only the maze autorouter runs.
    /// Behavioral coupling: with fanout off the Java job derives
    /// `removeUnconnectedVias = true` (BatchAutorouter.java:111-117), which
    /// skips the final tail sweep (AutorouteBatchLoop.java:594-600) — these
    /// captures are NOT comparable to the full-flow ones.
    RouterOnly,
}

impl OracleProfile {
    /// Parses the `--profile` CLI value; omitted means full-flow.
    pub fn from_name(name: Option<&str>) -> Result<Self> {
        match name {
            None | Some("full-flow") => Ok(Self::FullFlow),
            Some("router-only") => Ok(Self::RouterOnly),
            Some(other) => {
                bail!("unknown profile {other:?} (expected \"full-flow\" or \"router-only\")")
            }
        }
    }

    /// Extra flags appended to the oracle argv, after the base contract.
    /// Both defaults are ON in DefaultSettings (fanout.enabled :165,
    /// optimizer.enabled :178), so the router-only profile must pass BOTH
    /// disables explicitly; the property paths reach RouterSettings through
    /// the CliSettings `router.`/`optimizer.` prefix gates (the bridge keeps
    /// `fanout.enabled` verbatim).
    pub fn extra_args(&self) -> &'static [&'static str] {
        match self {
            Self::FullFlow => &[],
            Self::RouterOnly => &["--optimizer.enabled=false", "--router.fanout.enabled=false"],
        }
    }

    /// The profile's name on the capture/verify `--profile` surface — the
    /// exact spelling [`OracleProfile::from_name`] accepts. Single source
    /// for error text that names a record set's own surface (the
    /// router-compare loader errors route through here, so a marker
    /// mismatch names the capture face the records came from).
    pub fn capture_name(self) -> &'static str {
        match self {
            Self::FullFlow => "full-flow",
            Self::RouterOnly => "router-only",
        }
    }

    /// Baselines directory under `rust/harness/baselines/`.
    pub fn dir_name(&self) -> &'static str {
        match self {
            Self::FullFlow => "java",
            Self::RouterOnly => "router-only",
        }
    }

    /// The record marker that makes a router-only record unmistakable from
    /// a full-flow one in code, not by path alone (compare() gates it).
    pub fn marker(&self) -> Option<&'static str> {
        match self {
            Self::FullFlow => None,
            Self::RouterOnly => Some("router-only"),
        }
    }

    /// The T14 capture discipline: router-only captures double-run + diff
    /// every fixture (the T12 MINOR-2 snapshot-race lesson); the committed
    /// full-flow captures predate this profile machinery.
    pub fn double_run(&self) -> bool {
        matches!(self, Self::RouterOnly)
    }
}

/// The PROVE-stages-off gate (M3-T14): a router-only manifest must show BOTH
/// disabled stages silent. The faces are read off the Java, not guessed:
/// - optimizer off (RoutingPipeline.java:36 — the optimizer stage is never
///   constructed): its PhaseDetail is never touched, so duration_seconds and
///   passes_completed are absent, and RoutingResultManifest.java:200-203
///   emits top-level optimizer_score only iff the optimizer phase carried a
///   before/after snapshot — absent here.
/// - fanout off (AutorouteBatchLoop.java:96-102 — the phase block at
///   :232-251 runs only when isFanoutEnabled()): duration_seconds and
///   passes_completed absent.
///   Any populated face means a disable flag was dropped or typo'd: fail
///   fast instead of committing a full-flow run into the router-only
///   directory.
pub fn assert_router_only_manifest(manifest: &RoutingResultManifest) -> Result<()> {
    let mut faces: Vec<String> = Vec::new();
    if let Some(d) = manifest.phases.optimizer.duration_seconds {
        faces.push(format!("phases.optimizer.duration_seconds={d}"));
    }
    if let Some(p) = manifest.phases.optimizer.passes_completed {
        faces.push(format!("phases.optimizer.passes_completed={p}"));
    }
    if let Some(s) = manifest.optimizer_score {
        faces.push(format!("optimizer_score={s}"));
    }
    if let Some(d) = manifest.phases.fanout.duration_seconds {
        faces.push(format!("phases.fanout.duration_seconds={d}"));
    }
    if let Some(p) = manifest.phases.fanout.passes_completed {
        faces.push(format!("phases.fanout.passes_completed={p}"));
    }
    if faces.is_empty() {
        Ok(())
    } else {
        bail!(
            "PROVE-stages-off gate failed — the router-only flags did not take \
             (disabled-stage activity in the manifest): {}",
            faces.join("; ")
        )
    }
}

impl OracleProfile {
    /// The PROVE-stages-off gate, SCOPED BY PROFILE: only `RouterOnly` has
    /// disabled stages to prove, so only it gates. A full-flow manifest
    /// necessarily carries optimizer/fanout faces, which is why an
    /// unconditional call site bailed every default `capture` after a
    /// successful run (quality-review MAJOR-1, live-witnessed) — folding the
    /// profile decision into the gate itself makes that mis-scoping
    /// unrepresentable at any call site, capture or verify.
    pub fn assert_stages_off(self, manifest: &RoutingResultManifest) -> Result<()> {
        match self {
            Self::FullFlow => Ok(()),
            Self::RouterOnly => assert_router_only_manifest(manifest),
        }
    }
}

/// Where results of one oracle invocation land.
#[derive(Debug)]
pub struct OracleRun {
    pub manifest: Option<RoutingResultManifest>,
    /// Some(serde error) when a manifest file existed but failed to parse.
    pub manifest_error: Option<String>,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub wall_seconds: f64,
    pub ses_path: PathBuf,
    pub manifest_path: PathBuf,
}

/// Locates the repository root by walking up from the current directory
/// until a directory containing `.git` is found. (Pre-M10-T4 this also
/// required `build.gradle` — the sunset deleted the gradle build, so the
/// marker would never match and EVERY harness subcommand, java-free
/// compares included, would fail. M10-T4 repair, tree `ed07e2cfc`.)
pub fn find_repo_root() -> Result<PathBuf> {
    let mut dir = std::env::current_dir().context("getting current directory")?;
    loop {
        if dir.join(".git").exists() {
            // The Q-5 marker hardening (M10-T5 C′): a bare `.git` is not
            // enough — a foreign repo (or an unrelated project's tree)
            // would send every path join below haywire. An EpicRouter
            // tree is the one that carries the harness itself.
            if !dir.join("rust/harness").is_dir() {
                bail!(
                    "found .git at {} but not an EpicRouter tree (no rust/harness under it)",
                    dir.display()
                );
            }
            return Ok(dir);
        }
        if !dir.pop() {
            bail!("could not locate repo root (no .git upward from cwd)");
        }
    }
}

/// The jar requires Java 25; the system default may be older, so prefer an
/// explicit override, then `~/.jdks/jdk-25*`, then PATH `java`.
pub fn resolve_java() -> Result<PathBuf> {
    let explicit = std::env::var("EPIC_ORACLE_JAVA").unwrap_or_default();
    if !explicit.is_empty() {
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
        let java = candidates.iter().find_map(|jdk| {
            let java = jdk.join("bin").join("java");
            java.is_file().then_some(java)
        });
        if let Some(java) = java {
            return Ok(java);
        }
    }
    Ok(PathBuf::from("java"))
}

/// Path of the oracle executable jar the gradle `executableJar` task produces.
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
/// `extra_args` (the profile's flags) is appended verbatim AFTER the base
/// contract — one suffix site shared by capture AND verify, so the two paths
/// cannot drift.
pub fn oracle_argv(
    java: &Path,
    jar: &Path,
    jvm_xmx: &str,
    dsn: &Path,
    ses_out: &Path,
    manifest_out: &Path,
    extra_args: &[&str],
) -> Vec<String> {
    let mut argv = vec![
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
    ];
    argv.extend(extra_args.iter().map(|s| (*s).to_string()));
    argv
}

/// Spawns one oracle run under `profile`. `work_dir` receives `out.ses`,
/// `manifest.json`, `stdout.log`, `stderr.log`. Output goes to files (not
/// pipes) so a chatty JVM can never deadlock on a full pipe buffer. Taking
/// the profile here (not raw flags) forces every caller — capture AND
/// verify — through the same flag threading; there is no second argv site
/// to forget the suffix on.
pub fn run_oracle(
    java: &Path,
    jar: &Path,
    dsn: &Path,
    work_dir: &Path,
    timeout: Duration,
    jvm_xmx: &str,
    profile: OracleProfile,
) -> Result<OracleRun> {
    let ses_path = work_dir.join("out.ses");
    let manifest_path = work_dir.join("manifest.json");
    let stdout_path = work_dir.join("stdout.log");
    let stderr_path = work_dir.join("stderr.log");
    std::fs::create_dir_all(work_dir)
        .with_context(|| format!("creating work dir {}", work_dir.display()))?;
    // A reused work_dir may hold a previous run's outputs; a run that dies
    // before writing must never have them read back as its own.
    let _ = std::fs::remove_file(&manifest_path);
    let _ = std::fs::remove_file(&ses_path);

    let argv = oracle_argv(
        java,
        jar,
        jvm_xmx,
        dsn,
        &ses_path,
        &manifest_path,
        profile.extra_args(),
    );
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

    let (manifest, manifest_error) = parse_manifest_file(&manifest_path);

    Ok(OracleRun {
        manifest,
        manifest_error,
        exit_code,
        timed_out,
        wall_seconds,
        ses_path,
        manifest_path,
    })
}

/// Reads and parses a run's manifest. A missing file means the engine never
/// wrote one (both `None`). A present-but-unparseable file — Gson-vs-mirror
/// schema drift, or a write truncated by our own timeout kill — must not
/// masquerade as "no manifest produced", so the error is surfaced separately.
/// `pub(crate)` for the T15 router compare: the Rust epic-cli manifest
/// parses through the SAME tolerant mirror as the Java manifest (the
/// mirror deliberately has no `deny_unknown_fields`).
pub(crate) fn parse_manifest_file(path: &Path) -> (Option<RoutingResultManifest>, Option<String>) {
    if !path.is_file() {
        return (None, None);
    }
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading manifest {}", path.display()));
    match raw {
        Ok(raw) => match serde_json::from_str(&raw) {
            Ok(m) => (Some(m), None),
            Err(e) => (None, Some(e.to_string())),
        },
        Err(e) => (None, Some(format!("{e:#}"))),
    }
}

/// `pub(crate)` for the T15 router compare: one timeout-wait loop shared
/// by the oracle and the epic-cli subprocess runs (200ms poll, kill at the
/// deadline, `(None, true)` on timeout).
pub(crate) fn wait_with_timeout(
    child: &mut std::process::Child,
    timeout: Duration,
) -> (Option<i32>, bool) {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return (status.code(), false),
            Ok(None) => {}
            Err(_) => {
                // Near-impossible on Unix; downstream gates fail loudly on
                // exit_code None, and a child left unreaped is negligible
                // for a short-lived CLI process.
                return (None, false);
            }
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

    /// Minimal manifest JSON every manifest-related test can rely on.
    const VALID_MANIFEST: &str = r#"{
  "schema_version": 1,
  "app_version": "test",
  "git_sha": "abc123",
  "fixture": {},
  "phases": {},
  "final_state": "COMPLETED",
  "exit_code": 0,
  "output_written": true
}"#;

    #[test]
    fn parse_manifest_file_valid_and_absent() {
        // (a) valid file → (Some, None)
        let good = std::env::temp_dir().join(format!(
            "epic-harness-parse-good-{}.json",
            std::process::id()
        ));
        std::fs::write(&good, VALID_MANIFEST).expect("writing manifest fixture");
        let (m, err) = parse_manifest_file(&good);
        let _ = std::fs::remove_file(&good);
        assert!(err.is_none());
        let m = m.expect("valid manifest must parse");
        assert_eq!(m.final_state, "COMPLETED");

        // (c) nonexistent path → (None, None): the engine never wrote one
        let (m, err) = parse_manifest_file(Path::new("/nonexistent/epic-harness/manifest.json"));
        assert!(m.is_none());
        assert!(err.is_none());
    }

    #[test]
    fn parse_manifest_file_garbage_reports_error() {
        // (b) present but unparseable → (None, Some(serde error))
        let bad = std::env::temp_dir().join(format!(
            "epic-harness-parse-bad-{}.json",
            std::process::id()
        ));
        std::fs::write(&bad, "not json").expect("writing garbage fixture");
        let (m, err) = parse_manifest_file(&bad);
        let _ = std::fs::remove_file(&bad);
        assert!(m.is_none());
        let err = err.expect("garbage must yield a parse error");
        // serde_json always appends its source position — a stable marker
        // that the error came from the JSON parser itself.
        assert!(err.contains("at line 1 column"), "unexpected error: {err}");
    }

    /// Regression: a reused work_dir holding a previous run's outputs must
    /// not have them read back when the new run dies before writing anything.
    /// `/bin/true` stands in for the JVM: exits 0 immediately, writes nothing.
    #[test]
    fn stale_outputs_removed_and_reported_honestly() {
        let work = std::env::temp_dir().join("epic-harness-stale-outputs-test");
        std::fs::create_dir_all(&work).expect("creating work dir");
        let stale_manifest = work.join("manifest.json");
        let stale_ses = work.join("out.ses");
        std::fs::write(&stale_manifest, VALID_MANIFEST).expect("seeding stale manifest");
        std::fs::write(&stale_ses, b"stale ses").expect("seeding stale ses");

        let run = run_oracle(
            Path::new("/bin/true"),
            Path::new("/repo/no-such-jar.jar"),
            Path::new("/repo/no-such-design.dsn"),
            &work,
            Duration::from_secs(10),
            "4g",
            OracleProfile::FullFlow,
        )
        .expect("oracle run");

        assert_eq!(run.exit_code, Some(0));
        assert!(!run.timed_out);
        assert!(
            run.manifest.is_none(),
            "stale manifest must be removed, not parsed"
        );
        assert!(run.manifest_error.is_none());
        assert!(!run.manifest_path.is_file());
        // The stale-SES face is the worse hazard: a reused work dir that
        // keeps the previous run's SES lets distill_profiled hash STALE
        // bytes into the record (and the double-run diff compare
        // fresh-vs-stale digests). The file must be gone with the manifest.
        assert!(
            !run.ses_path.is_file(),
            "stale ses must be removed, not left for the next distill to hash"
        );
        let _ = std::fs::remove_file(&stale_manifest);
        let _ = std::fs::remove_file(&stale_ses);
    }

    #[test]
    fn argv_order_matches_java_cli_contract() {
        let argv = oracle_argv(
            Path::new("/jdks/jdk-25/bin/java"),
            Path::new("/repo/build/libs/freerouting-current-executable.jar"),
            "4g",
            Path::new("/repo/fixtures/sonde xilinx.dsn"),
            Path::new("/tmp/w/out.ses"),
            Path::new("/tmp/w/manifest.json"),
            OracleProfile::FullFlow.extra_args(),
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

    /// The router-only profile appends EXACTLY the two disable flags, in
    /// order, as an argv suffix — never interleaved with the base contract,
    /// never fed anywhere but the Java jar (epic-cli would drop
    /// `--optimizer.enabled=false` with a warning).
    #[test]
    fn router_only_argv_appends_disable_flags_as_suffix() {
        let argv = oracle_argv(
            Path::new("/jdks/jdk-25/bin/java"),
            Path::new("/repo/build/libs/freerouting-current-executable.jar"),
            "4g",
            Path::new("/repo/fixtures/x.dsn"),
            Path::new("/tmp/w/out.ses"),
            Path::new("/tmp/w/manifest.json"),
            OracleProfile::RouterOnly.extra_args(),
        );
        assert_eq!(argv.len(), 14);
        // The base contract is untouched (contrast face: the profile only
        // ever ADDS a suffix).
        assert_eq!(&argv[..12][4], "-de");
        assert_eq!(argv[8], "--router.result_json=/tmp/w/manifest.json");
        assert_eq!(argv[11], "--api_server.enabled=false");
        // The suffix is exactly the two disables, in order.
        assert_eq!(argv[12], "--optimizer.enabled=false");
        assert_eq!(argv[13], "--router.fanout.enabled=false");
    }

    /// The PROVE-stages-off gate: each disabled-stage face must be rejected
    /// INDEPENDENTLY (a gate keying on only one field would pass a run where
    /// just that other stage stayed on), and the all-clear router-only face
    /// must pass.
    #[test]
    fn prove_off_gate_rejects_each_disabled_stage_face_independently() {
        let base = r#"{
          "schema_version": 1,
          "app_version": "t",
          "git_sha": "abc123",
          "fixture": {},
          "phases": { "fanout": {}, "autorouter": {}, "optimizer": {} },
          "final_state": "COMPLETED",
          "exit_code": 0,
          "output_written": true
        }"#;
        let all_clear: RoutingResultManifest = serde_json::from_str(base).expect("parses");
        assert!(assert_router_only_manifest(&all_clear).is_ok());

        for (field, json) in [
            (
                "optimizer duration",
                r#""phases": { "fanout": {}, "optimizer": { "duration_seconds": 0.0 } }"#,
            ),
            (
                "optimizer passes",
                r#""phases": { "fanout": {}, "optimizer": { "passes_completed": 0 } }"#,
            ),
            (
                "optimizer score",
                r#""phases": { "fanout": {}, "optimizer": {} }, "optimizer_score": 980.0"#,
            ),
            (
                "fanout duration",
                r#""phases": { "fanout": { "duration_seconds": 0.5 }, "optimizer": {} }"#,
            ),
            (
                "fanout passes",
                r#""phases": { "fanout": { "passes_completed": 1 }, "optimizer": {} }"#,
            ),
        ] {
            let raw = base.replacen(
                r#""phases": { "fanout": {}, "autorouter": {}, "optimizer": {} }"#,
                json,
                1,
            );
            let m: RoutingResultManifest = serde_json::from_str(&raw).expect("world must parse");
            let err = assert_router_only_manifest(&m)
                .expect_err(&format!("{field} face must fail the gate"));
            let msg = err.to_string();
            assert!(
                msg.contains("PROVE-stages-off"),
                "unexpected error for {field}: {msg}"
            );
            assert!(
                msg.contains(field.split_once(' ').expect("two words").0),
                "error for {field} must name the stage: {msg}"
            );
        }
    }

    /// A full-flow-shaped manifest (BOTH stages on) trips every face —
    /// the gate cannot be satisfied by fixing only one stage.
    #[test]
    fn prove_off_gate_rejects_full_flow_manifest_listing_both_stages() {
        let raw = r#"{
          "schema_version": 1,
          "app_version": "t",
          "git_sha": "abc123",
          "fixture": {},
          "phases": {
            "fanout": { "duration_seconds": 0.5, "passes_completed": 1 },
            "autorouter": { "duration_seconds": 10.0, "passes_completed": 20 },
            "optimizer": { "duration_seconds": 5.0, "passes_completed": 3 }
          },
          "optimizer_score": 980.0,
          "final_state": "COMPLETED",
          "exit_code": 0,
          "output_written": true
        }"#;
        let m: RoutingResultManifest = serde_json::from_str(raw).expect("parses");
        let err = assert_router_only_manifest(&m).expect_err("full-flow manifest must fail");
        let msg = err.to_string();
        assert!(msg.contains("optimizer") && msg.contains("fanout"), "{msg}");
        assert!(msg.contains("optimizer_score"), "{msg}");
    }

    /// The profile contract: exact flag set, dir routing, record marker,
    /// double-run discipline, and name parsing (both faces + the unknown
    /// face).
    #[test]
    fn router_only_profile_contract_flags_dirs_marker_and_names() {
        assert_eq!(OracleProfile::FullFlow.extra_args(), &[] as &[&str]);
        assert_eq!(
            OracleProfile::RouterOnly.extra_args(),
            &["--optimizer.enabled=false", "--router.fanout.enabled=false"]
        );
        assert_eq!(OracleProfile::FullFlow.dir_name(), "java");
        assert_eq!(OracleProfile::RouterOnly.dir_name(), "router-only");
        assert_eq!(OracleProfile::FullFlow.marker(), None);
        assert_eq!(OracleProfile::RouterOnly.marker(), Some("router-only"));
        assert!(!OracleProfile::FullFlow.double_run());
        assert!(OracleProfile::RouterOnly.double_run());
        assert!(matches!(
            OracleProfile::from_name(None),
            Ok(OracleProfile::FullFlow)
        ));
        assert!(matches!(
            OracleProfile::from_name(Some("full-flow")),
            Ok(OracleProfile::FullFlow)
        ));
        assert!(matches!(
            OracleProfile::from_name(Some("router-only")),
            Ok(OracleProfile::RouterOnly)
        ));
        let err = OracleProfile::from_name(Some("routeronly"))
            .expect_err("a typo'd profile name must be rejected, not silently full-flow");
        assert!(
            err.to_string().contains("unknown profile"),
            "unexpected error: {err}"
        );
    }

    /// The gate is SCOPED BY PROFILE (quality-review MAJOR-1): a full-flow
    /// capture must sail through on a stages-on manifest (the ungated call
    /// site bailed every default capture after a successful run), while the
    /// SAME manifest under RouterOnly is the flag-drop signature. Both
    /// directions of the scoping decision are pinned; the mutants (FullFlow
    /// gated too / RouterOnly no-op) die here.
    #[test]
    fn stages_off_gate_is_profile_scoped() {
        let stages_on: RoutingResultManifest = serde_json::from_str(
            r#"{
              "schema_version": 1,
              "app_version": "t",
              "git_sha": "abc123",
              "fixture": {},
              "phases": {
                "fanout": { "duration_seconds": 0.53 },
                "autorouter": { "duration_seconds": 3.0, "passes_completed": 2 },
                "optimizer": { "duration_seconds": 4.37, "passes_completed": 2 }
              },
              "optimizer_score": 823.79,
              "final_state": "COMPLETED",
              "exit_code": 0,
              "output_written": true
            }"#,
        )
        .expect("parses");
        // FullFlow face: stages-on is the LEGITIMATE full-flow shape — no
        // gate, ever, regardless of what the manifest carries.
        assert!(
            OracleProfile::FullFlow
                .assert_stages_off(&stages_on)
                .is_ok(),
            "full-flow capture must never be gated by the prove-off gate"
        );
        // RouterOnly face: the same manifest means a disable flag was
        // dropped — the gate must reject and name the faces.
        let err = OracleProfile::RouterOnly
            .assert_stages_off(&stages_on)
            .expect_err("stages-on manifest must fail the router-only gate");
        let msg = err.to_string();
        assert!(msg.contains("PROVE-stages-off"), "unexpected error: {msg}");
        assert!(msg.contains("optimizer") && msg.contains("fanout"), "{msg}");

        // All-clear router-only shape passes its own profile's gate.
        let all_clear: RoutingResultManifest = serde_json::from_str(
            r#"{
              "schema_version": 1,
              "app_version": "t",
              "git_sha": "abc123",
              "fixture": {},
              "phases": { "fanout": {}, "autorouter": {}, "optimizer": {} },
              "final_state": "COMPLETED",
              "exit_code": 0,
              "output_written": true
            }"#,
        )
        .expect("parses");
        assert!(
            OracleProfile::RouterOnly
                .assert_stages_off(&all_clear)
                .is_ok()
        );
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
        let run = run_oracle(
            &java,
            &jar,
            &dsn,
            &work,
            Duration::from_secs(300),
            "4g",
            OracleProfile::FullFlow,
        )
        .expect("oracle run");
        let m = run.manifest.as_ref().expect("manifest must be written");
        assert_eq!(m.final_state, "COMPLETED");
        assert_eq!(m.exit_code, 0);
        assert!(run.ses_path.is_file());
        assert!(run.ses_path.metadata().expect("ses stat").len() > 0);
    }
}
