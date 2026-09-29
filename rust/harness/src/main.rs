//! Differential parity + benchmark harness for the EpicRouter rewrite
//! (design §5). M0 scope: drive the Java oracle engine and capture golden
//! baselines.

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::time::Duration;

mod baseline;
// The M5-T1 allocation-cost instrument (feature `alloc-profile` ONLY —
// default builds cfg it out entirely and keep the std allocator, so the
// production face is untouched; never wired into CI).
#[cfg(feature = "alloc-profile")]
mod alloc_profile;
#[cfg(feature = "alloc-profile")]
mod alloc_route;
// THE workflow tripwire pin (M3-T17c; closes banked mutant S6): the CI
// test step's 30-minute wall and the router-compare step's
// `--report-only` are pinned — the exit flip or a wall move must be a
// conscious same-commit act with the pin. Pin-only module (no command
// consumes it), so it exists for the test target alone.
#[cfg(test)]
mod ci_tripwire;
mod corpus;
// The shared corpus shell (M3 Task 1, M2 carry-forward): manifest row +
// byte format, strict JSONL, positional alignment, diff rendering, the
// one sha256_hex — one copy for the dsn/index/undo/routing corpora.
mod corpus_common;
// Canonical DSN parse digest (M1b Task 4); consumed by the `dsn`
// subcommands' goldens/compare since Task 11.
mod dsn_corpus;
mod dsn_digest;
// DRC parity corpus (M3 Task 2): parse-time incomplete connections +
// clearance violations on tier A + PCBench pre-routed boards, Java
// goldens, java-free CI compare.
mod drc_corpus;
// Index parity corpus (M2 Task 9): tier + stressor fixtures replayed
// through the landed search-tree surface, Java goldens, java-free CI
// compare.
mod global_golden;
mod index_corpus;
mod islands_face;
mod manifest;
mod oracle;
// The ONE home for the process-global panic-hook swap (quality-review
// T17b M-Q2): shared lock + RAII silence guard used by BOTH hook
// participants in the test binary (run_detail_pass's watchdog pins and
// corpus::compare) so their take/silence/restore windows cannot race.
mod panic_hook;
// Router quality scoreboard (M3 Task 15): directional Rust-vs-Java-record
// compare gates (java-free), the Rust-vs-Rust determinism self-gate, and
// the first-divergence localizer.
mod aesthetics_face;
mod router_compare;
// Route event-stream corpus (M3 Task 16): the maze-level trace stream
// (RAW_SECTION assign/skip + compare_trace_ripped/route_item) captured
// from the Java runBatchLoop by the RouteEventProbe, mirrored by the Rust
// batch driver, compared kind-ordinally field-by-field (java-free CI
// compare; the golden carries the settings witness the Rust world builds
// from).
mod route_events;
// SES emission-parity goldens + canonical compare (M1b Task 13 B3/B4).
mod ses_compare;
mod tiers;
// Undo/snapshot parity corpus (M2 Task 14): the scripted replay of
// snapshot/insert/remove/undo/redo/pop through the board facade, Java
// goldens, java-free CI compare.
mod undo_corpus;

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
        /// The oracle profile: "full-flow" (default) or "router-only"
        /// (fanout + optimizer disabled via JAVA jar flags only). The
        /// flags NEVER go to epic-cli (the Rust CLI drops
        /// `--optimizer.enabled=false` with a spurious warning).
        #[arg(long)]
        profile: Option<String>,
    },
    /// Re-run the oracle and compare against committed baselines.
    Verify {
        /// Only verify this tier; omit for all.
        #[arg(long)]
        tier: Option<String>,
        /// Must match the profile the baselines were captured with — the
        /// profile threads through to the re-run (trap 3: an unthreaded
        /// verify captures a full-flow run and the profile gate in
        /// compare fails the mismatch loudly).
        #[arg(long)]
        profile: Option<String>,
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
    /// Undo/snapshot parity corpus (M2 Task 14): the scripted
    /// snapshot/insert/remove/undo/redo/pop replay on tier + stressor
    /// fixtures, Java-oracle goldens, and a java-free compare (the CI
    /// gate).
    Undo {
        #[command(subcommand)]
        cmd: undo_corpus::UndoCommand,
    },
    /// DRC parity corpus (M3 Task 2): parse-time incompletes +
    /// clearance violations on tier A + PCBench pre-routed boards,
    /// Java-oracle goldens, and a java-free compare (the CI gate).
    Drc {
        #[command(subcommand)]
        cmd: drc_corpus::DrcCommand,
    },
    /// Router quality scoreboard (M3 Task 15): per-Tier-A directional
    /// compare gates against the committed router-only Java records
    /// (java-free — the records ARE the Java side), the Rust-vs-Rust
    /// determinism self-gate, and the first-divergence localizer.
    /// Resolves tiers.yaml + epic-cli itself; never touches the JVM.
    Router {
        #[command(subcommand)]
        cmd: router_compare::RouterCommand,
    },
    /// The M6-T6 pour-island detector face (advisory, java-free): parse
    /// the DSN and print the per-pour region/island rows. Never a gate;
    /// the fixture-evidence instrument for the detector population.
    Islands {
        /// The DSN file to analyze.
        #[arg(long)]
        dsn: PathBuf,
    },
    /// The M8-T1 aesthetics reference face (java-free): parse the DSN
    /// through the drc_corpus parse path and print the four-metric
    /// JSON (the golden bytes). Never a gate; the golden producer and
    /// the fixture-evidence instrument for the aesthetics population.
    Aesthetics {
        /// The DSN file to measure (a reference-routed board).
        #[arg(long)]
        dsn: PathBuf,
    },
    /// The M6-T7 settings-ON golden face (java-free): capture/verify
    /// the `router.congestion_global=on` route of a crafted fixture
    /// (determinism self-gate + digest comparison against the
    /// committed golden).
    GlobalGolden {
        #[command(subcommand)]
        cmd: global_golden::GlobalGoldenCommand,
    },
    /// Route event-stream corpus (M3 Task 16): the Java probe capture
    /// (double-run byte-identical) into the committed golden, and the
    /// java-free kind-ordinal field-level compare of the Rust engine's
    /// stream against it.
    Events {
        #[command(subcommand)]
        cmd: route_events::EventsCommand,
    },
    /// M5-T1 allocation-cost instrument (feature `alloc-profile`): route
    /// ONE fixture in-process under the counting allocator. Never a
    /// default build; never wired into CI.
    #[cfg(feature = "alloc-profile")]
    AllocRoute {
        /// The fixture DSN to route (absolute path).
        #[arg(long)]
        dsn: PathBuf,
        /// Where the allocator stats file is flushed (1 s watcher + final).
        #[arg(long)]
        stats: PathBuf,
        /// Capture a backtrace sample every Nth allocation (0 = off).
        #[arg(long, default_value_t = 0)]
        sample: u64,
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
    // The allocation instrument needs no tier file or JVM resolution.
    #[cfg(feature = "alloc-profile")]
    if let Command::AllocRoute { dsn, stats, sample } = cli.command {
        return alloc_route::run(&dsn, &stats, sample);
    }
    if let Command::Corpus { cmd } = cli.command {
        // The corpus pipeline needs no tier file or eager JVM resolution.
        return corpus::run(cmd);
    }
    if let Command::Islands { dsn } = cli.command {
        return islands_face::run(&dsn);
    }
    if let Command::Aesthetics { dsn } = cli.command {
        // The M8-T1 reference door: java-free, parse-only, no tier
        // file or JVM resolution needed.
        aesthetics_face::run(&dsn)?;
        return Ok(());
    }
    if let Command::GlobalGolden { cmd } = cli.command {
        // The M6-T7 settings-ON golden face: java-free, resolves
        // epic-cli itself (the router-compare shape).
        return global_golden::run(cmd);
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
    if let Command::Undo { cmd } = cli.command {
        // Same shape as the index pipeline: tiers.yaml read at
        // manifest time, JVM resolved lazily — `undo compare` never
        // needs one.
        return undo_corpus::run(cmd, &cli.jvm_xmx);
    }
    if let Command::Drc { cmd } = cli.command {
        // Same shape as the index pipeline: tiers.yaml read at
        // manifest time, JVM resolved lazily — `drc compare` never
        // needs one.
        return drc_corpus::run(cmd, &cli.jvm_xmx);
    }
    if let Command::Router { cmd } = cli.command {
        // Same shape: tiers.yaml + epic-cli resolved inside; the compare
        // is java-free (the committed records are the Java side) so no
        // JVM resolution ever happens on this path.
        return router_compare::run(cmd);
    }
    if let Command::Events { cmd } = cli.command {
        // The events pipeline: `golden` spawns the probe JVM itself
        // (javac + jar, resolved inside); `compare` is java-free. No
        // tier-file resolution either way.
        return route_events::run(cmd, &cli.jvm_xmx);
    }
    let repo_root = oracle::find_repo_root()?;
    let tiers_path = resolve_tiers_path(&repo_root, &cli.tiers);
    let tier_file = TierFile::load(&tiers_path)?;
    let fixtures_root = repo_root.join(&tier_file.fixtures_root);
    let java = oracle::resolve_java()?;
    let jar = oracle::jar_path(&repo_root);

    match cli.command {
        // Index/Undo/Drc/Router/Events/Corpus/Dsn returned before the
        // tier/JVM resolution above; this arm only satisfies the match.
        Command::Index { .. } => unreachable!("handled before tier/JVM resolution"),
        Command::Undo { .. } => unreachable!("handled before tier/JVM resolution"),
        Command::Drc { .. } => unreachable!("handled before tier/JVM resolution"),
        Command::Router { .. } => unreachable!("handled before tier/JVM resolution"),
        Command::GlobalGolden { .. } => unreachable!("handled before tier/JVM resolution"),
        Command::Events { .. } => unreachable!("handled before tier/JVM resolution"),
        #[cfg(feature = "alloc-profile")]
        Command::AllocRoute { .. } => unreachable!("handled before tier/JVM resolution"),
        Command::Islands { .. } | Command::Aesthetics { .. } => {
            unreachable!("handled before tier/JVM resolution")
        }

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
        Command::Capture { tier, profile } => {
            let profile = oracle::OracleProfile::from_name(profile.as_deref())?;
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
            // Profile-routed output: full-flow keeps the classic
            // baselines/java/ + runs/<tier>/ layout; router-only writes
            // baselines/router-only/ with scratch under runs/router-only/
            // so the profiles never clobber each other.
            let profile_baselines_root = baselines_root.join(profile.dir_name());
            let profile_runs_root = match profile {
                oracle::OracleProfile::FullFlow => runs_root,
                oracle::OracleProfile::RouterOnly => runs_root.join(profile.dir_name()),
            };
            for tier in selected {
                capture_tier(
                    tier,
                    &fixtures_root,
                    &tier_file.fixtures_root,
                    &java,
                    &jar,
                    &profile_baselines_root,
                    &profile_runs_root,
                    &cli.jvm_xmx,
                    profile,
                )?;
            }
            Ok(())
        }
        Command::Verify { tier, profile } => {
            // The profile must be threaded into BOTH the record lookup and
            // the re-run's argv, or verify would capture a full-flow run and
            // compare it against router-only records (trap 3).
            let profile = oracle::OracleProfile::from_name(profile.as_deref())?;
            // Validate the tier name before paying for the gradle build.
            let selected = select_tiers(&tier_file, tier.as_deref())?;
            let harness_root = tiers_path
                .parent()
                .and_then(Path::parent)
                .map(Path::to_path_buf)
                .unwrap_or_else(|| repo_root.join("rust/harness"));
            let baselines_root = harness_root.join("baselines").join(profile.dir_name());
            let run_root = harness_root
                .join("runs")
                .join("verify")
                .join(profile.dir_name());
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
                        profile,
                    )
                    .with_context(|| {
                        format!(
                            "oracle run failed while verifying {} (tier {})",
                            fixture.path, tier.name
                        )
                    })?;
                    // Defense symmetry with capture (quality-review MINOR-3):
                    // the verify re-run is proved stages-off under the
                    // router-only profile BEFORE its record is distilled, so
                    // a disable flag dropped at this call site fails here —
                    // even on fixtures where the stages don't bind (bm08/
                    // ecc83 verify green either way on the numbers alone).
                    if let Some(manifest) = run.manifest.as_ref() {
                        profile.assert_stages_off(manifest).with_context(|| {
                            format!("verify run for {} (tier {})", fixture.path, tier.name)
                        })?;
                    }
                    let actual = baseline::distill_profiled(
                        profile.marker(),
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
    profile: oracle::OracleProfile,
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
        let timeout = Duration::from_secs(fixture.timeout_seconds);
        let work_base = runs_root.join(&tier.name).join(&fixture.path);
        let work_dir = |run_no: usize| {
            if profile.double_run() {
                work_base.join(format!("run{run_no}"))
            } else {
                work_base.clone()
            }
        };
        println!(
            "[tier {}] routing {} (timeout {}s, profile {})…",
            tier.name,
            fixture.path,
            fixture.timeout_seconds,
            profile.marker().unwrap_or("full-flow")
        );
        let run: OracleRun = oracle::run_oracle(
            java,
            jar,
            &dsn,
            &work_dir(1),
            timeout,
            jvm_xmx,
            profile,
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

        // PROVE-stages-off pre-gate, baked into the capture command: under
        // the router-only profile, a typo'd or dropped disable flag would
        // silently capture a full-flow baseline into the router-only
        // directory — the gate makes that fail fast, per fixture, on BOTH
        // double-runs. Scoped BY PROFILE (quality-review MAJOR-1): a
        // full-flow manifest legitimately carries the stage faces, so the
        // gate must never fire for FullFlow (the ungated call site broke
        // every default capture).
        if let Some(manifest) = run.manifest.as_ref() {
            profile.assert_stages_off(manifest).with_context(|| {
                format!(
                    "fixture {} captured under profile {:?}",
                    fixture.path,
                    profile.marker().unwrap_or("full-flow")
                )
            })?;
        }

        // Double-run + diff discipline (the T12 MINOR-2 snapshot-race
        // lesson): stable fields must agree EXACTLY across the two runs or
        // the capture is racy and must not pass silently. Racy-but-tracked
        // faces (the SES digest) are returned as a note and stamped onto the
        // committed record — recorded, never normalized away (trap 4).
        let race_note = if profile.double_run() {
            let run2: OracleRun = oracle::run_oracle(
                java,
                jar,
                &dsn,
                &work_dir(2),
                timeout,
                jvm_xmx,
                profile,
            )
            .with_context(|| {
                format!(
                    "oracle double-run failed for {} (captured {}/{} fixtures in tier {} before failure)",
                    fixture.path,
                    captured,
                    tier.fixtures.len(),
                    tier.name
                )
            })?;
            if let Some(manifest) = run2.manifest.as_ref() {
                profile.assert_stages_off(manifest).with_context(|| {
                    format!(
                        "fixture {} double-run under profile router-only",
                        fixture.path
                    )
                })?;
            }
            diff_double_runs(&run, &run2, &fixture.path)?
        } else {
            None
        };

        // The record's `fixture` field is repo-relative (resolvable without
        // tiers.yaml); the raw yaml fixtures_root is joined, not the
        // absolute one used for I/O. The kept run is the FIRST one; the
        // second run is the determinism witness.
        let mut record = baseline::distill_profiled(
            profile.marker(),
            &fixtures_root_raw.join(&fixture.path).to_string_lossy(),
            &run,
        );
        if let Some(note) = race_note {
            record.notes = Some(match record.notes.take() {
                Some(existing) => format!("{existing}; {note}"),
                None => note,
            });
        }
        let out = baselines_root
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

/// The double-run diff (router-only capture discipline): the parity-relevant
/// fields must agree EXACTLY across two oracle runs of the same fixture —
/// any drift is a racy capture and bails rather than passing silently. Time
/// fields (seconds) legitimately vary and are not diffed. Returns a note for
/// the committed record when the SES digest diverges (a determinism face
/// T15 must know about) — recorded, never normalized away (trap 4).
fn diff_double_runs(run1: &OracleRun, run2: &OracleRun, fixture: &str) -> Result<Option<String>> {
    let record1 = baseline::distill_profiled(None, "", run1);
    let record2 = baseline::distill_profiled(None, "", run2);
    let mut drifted: Vec<String> = Vec::new();
    for (name, a, b) in [
        (
            "final_state",
            record1.final_state.clone(),
            record2.final_state.clone(),
        ),
        (
            "exit_code",
            format!("{:?}", record1.exit_code),
            format!("{:?}", record2.exit_code),
        ),
        (
            "incomplete_count",
            format!("{:?}", record1.incomplete_count),
            format!("{:?}", record2.incomplete_count),
        ),
        (
            "maximum_count",
            format!("{:?}", record1.maximum_count),
            format!("{:?}", record2.maximum_count),
        ),
        (
            "clearance_violations_total",
            format!("{:?}", record1.clearance_violations_total),
            format!("{:?}", record2.clearance_violations_total),
        ),
        (
            "clearance_router_introduced",
            format!("{:?}", record1.clearance_router_introduced),
            format!("{:?}", record2.clearance_router_introduced),
        ),
        (
            "normalized_score",
            format!("{:?}", record1.normalized_score),
            format!("{:?}", record2.normalized_score),
        ),
        (
            "passes_completed",
            format!("{:?}", record1.passes_completed),
            format!("{:?}", record2.passes_completed),
        ),
    ] {
        if a != b {
            drifted.push(format!("{name}: {a} vs {b}"));
        }
    }
    if !drifted.is_empty() {
        bail!(
            "fixture {} double-run drift in parity fields — capture is racy, NOT committed as clean: {}",
            fixture,
            drifted.join("; ")
        );
    }
    if record1.ses_sha256 != record2.ses_sha256 {
        let msg = format!(
            "ses_sha256 differs across the double-run (racy SES emission; record keeps run 1): {:?} vs {:?}",
            record1.ses_sha256, record2.ses_sha256
        );
        eprintln!("NOTE ({fixture}): {msg}");
        return Ok(Some(msg));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::RoutingResultManifest;
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

    /// The trap-3 witness, end-to-end: the committed router-only bm08
    /// baseline must verify green against a Java re-run WITH the same
    /// disable flags, through the same distill+compare path the Verify
    /// subcommand uses — and the SAME run distilled without the marker
    /// (an unthreaded verify's record) must fail with exactly the Profile
    /// gate. Requires the jar + JDK 25; run explicitly with
    /// `cargo test -- --ignored` (the capture battery is the fuller gate).
    #[test]
    #[ignore]
    fn router_only_bm08_baseline_verifies_green_and_profile_gate_bites() {
        let root = oracle::find_repo_root().expect("repo root");
        let java = oracle::resolve_java().expect("resolving java");
        let jar = oracle::jar_path(&root);
        let fixture_repo_rel = "scripts/benchmark/fixtures/DAC2020_boards/DAC2020_bm08.dsn";
        let dsn = root.join(fixture_repo_rel);
        let baseline_path = root
            .join("rust/harness/baselines")
            .join(oracle::OracleProfile::RouterOnly.dir_name())
            .join("A")
            .join("DAC2020_boards/DAC2020_bm08.dsn.baseline.json");
        let raw = std::fs::read_to_string(&baseline_path)
            .expect("committed router-only baseline (run the capture first)");
        let expected: BaselineRecord = serde_json::from_str(&raw).expect("baseline parses");
        assert_eq!(expected.profile.as_deref(), Some("router-only"));

        let work = std::env::temp_dir().join("epic-harness-router-only-verify-test");
        let run = oracle::run_oracle(
            &java,
            &jar,
            &dsn,
            &work,
            Duration::from_secs(300),
            "4g",
            oracle::OracleProfile::RouterOnly,
        )
        .expect("oracle run");
        let manifest = run.manifest.as_ref().expect("manifest written");
        oracle::OracleProfile::RouterOnly
            .assert_stages_off(manifest)
            .expect("stages proven off");

        let actual = baseline::distill_profiled(
            oracle::OracleProfile::RouterOnly.marker(),
            fixture_repo_rel,
            &run,
        );
        assert_eq!(
            baseline::compare(&expected, &actual),
            Vec::new(),
            "router-only re-run must verify green against the committed record"
        );

        // Contrast face: the identical run distilled WITHOUT the marker is
        // exactly what an unthreaded verify would compare — the profile
        // gate must be its sole failure.
        let unthreaded = baseline::distill_profiled(None, fixture_repo_rel, &run);
        let failures = baseline::compare(&expected, &unthreaded);
        assert_eq!(
            failures,
            vec![baseline::GateFailure::Profile {
                expected: Some("router-only".into()),
                actual: None,
            }]
        );

        // Prove-off-side face (quality-review MINOR-3): a disable flag
        // dropped AT the verify call site shows up as stages-on faces under
        // the router-only profile — the profile-scoped gate must reject it
        // before compare, while the SAME manifest is legitimate under
        // FullFlow (a full-flow verify runs the stages).
        let mut stages_on = manifest.clone();
        stages_on.phases.optimizer.duration_seconds = Some(4.37);
        let err = oracle::OracleProfile::RouterOnly
            .assert_stages_off(&stages_on)
            .expect_err("stages-on manifest must fail the router-only verify gate");
        assert!(
            err.to_string().contains("PROVE-stages-off"),
            "unexpected error: {err}"
        );
        assert!(
            oracle::OracleProfile::FullFlow
                .assert_stages_off(&stages_on)
                .is_ok(),
            "a full-flow verify must not be gated by the prove-off gate"
        );
    }

    /// MAJOR-1 regression witness (quality review Probe A): the DEFAULT
    /// full-flow capture path must work end to end — `capture_tier` scopes
    /// the prove-off gate to RouterOnly, so a stages-on manifest is
    /// legitimate here and the sanctioned regeneration path for
    /// `baselines/java/` stays alive. Also the first pin reaching
    /// `capture_tier` itself. Requires the jar + JDK 25; run explicitly
    /// with `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn full_flow_capture_tier_writes_a_correct_record_and_stays_green() {
        let root = oracle::find_repo_root().expect("repo root");
        let java = oracle::resolve_java().expect("resolving java");
        let jar = oracle::jar_path(&root);
        let scratch = std::env::temp_dir().join("epic-harness-full-flow-capture-test");
        let baselines_root = scratch.join("baselines/java");
        let runs_root = scratch.join("runs");
        let tier = Tier {
            name: "A".into(),
            fixtures: vec![FixtureRef {
                path: "DAC2020_boards/DAC2020_bm08.dsn".into(),
                timeout_seconds: 120,
            }],
        };
        capture_tier(
            &tier,
            &root.join("scripts/benchmark/fixtures"),
            Path::new("scripts/benchmark/fixtures"),
            &java,
            &jar,
            &baselines_root,
            &runs_root,
            "4g",
            oracle::OracleProfile::FullFlow,
        )
        .expect("full-flow capture must not bail on stages-on manifests (MAJOR-1)");

        let record_path = baselines_root
            .join("A")
            .join("DAC2020_boards/DAC2020_bm08.dsn.baseline.json");
        let raw = std::fs::read_to_string(&record_path).expect("full-flow record written");
        let record: BaselineRecord = serde_json::from_str(&raw).expect("parses");
        assert_eq!(record.profile, None, "full-flow record carries no marker");
        // The repo-relative path T15 keys baselines on (distill gets
        // fixtures_root_raw joined with the tier fixture path).
        assert_eq!(
            record.fixture,
            "scripts/benchmark/fixtures/DAC2020_boards/DAC2020_bm08.dsn"
        );
        assert_eq!(record.engine, "java");
        assert_eq!(record.final_state, "COMPLETED");
        assert_eq!(record.exit_code, Some(0));
        assert!(
            record.optimizer_score.is_some() && record.optimizer_seconds.is_some(),
            "a full-flow record must carry POPULATED optimizer faces"
        );
        assert!(record.ses_sha256.is_some());
        // notes=None proves the capture ran clean (no race note, no
        // degraded/infra-suspect acknowledgement demanded).
        assert_eq!(record.notes, None);
        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// Fully-populated parity manifest for the diff_double_runs pins: every
    /// one of the eight compared record fields is Some/non-default, so a
    /// world that mutates one field cannot hide behind a None==None face.
    const DIFF_MANIFEST: &str = r#"{
  "schema_version": 1,
  "app_version": "t",
  "git_sha": "abc123",
  "fixture": { "filename": "drift.dsn" },
  "phases": { "autorouter": { "duration_seconds": 1.0, "passes_completed": 18 } },
  "board_statistics": {
    "connections": { "incomplete_count": 2, "maximum_count": 50 },
    "clearance_violations": { "total_count": 1, "router_introduced_count": 0 }
  },
  "normalized_score": 986.32,
  "final_state": "COMPLETED",
  "exit_code": 0,
  "output_written": true
}"#;

    /// Synthetic OracleRun for the diff pins. `ses` points at a real file
    /// only in the ses-divergence pin; elsewhere it is deliberately absent
    /// (distill hashes nothing -> ses_sha256 None on both sides, so the
    /// eight parity fields are the only compared faces).
    fn diff_test_run(m: RoutingResultManifest, exit_code: i32, ses: &Path) -> OracleRun {
        OracleRun {
            manifest: Some(m),
            manifest_error: None,
            exit_code: Some(exit_code),
            timed_out: false,
            wall_seconds: 1.0,
            ses_path: ses.to_path_buf(),
            manifest_path: PathBuf::from("/nonexistent/epic-harness/manifest.json"),
        }
    }

    fn expect_drift_named(run1: &OracleRun, run2: &OracleRun, field: &str) {
        let err = diff_double_runs(run1, run2, "fixtures/drift.dsn")
            .expect_err("a one-field drift must bail the capture as racy");
        let msg = err.to_string();
        assert!(msg.contains("double-run drift"), "unexpected error: {msg}");
        assert!(
            msg.contains(field),
            "error must name the drifted field {field}: {msg}"
        );
        assert!(
            msg.contains("fixtures/drift.dsn"),
            "error must name the fixture: {msg}"
        );
    }

    /// The trap-4 discipline is only as strong as its detector: EVERY one of
    /// the eight parity fields must independently bail the double-run diff
    /// naming the drifted field, and a fully agreeing pair must pass clean.
    /// (The review mutant R2 — diff neutered to always-report-equal —
    /// survived the rest of the suite; this pin is its dedicated killer.)
    #[test]
    fn diff_double_runs_bails_naming_each_drifted_parity_field_and_passes_on_agreement() {
        let base: RoutingResultManifest =
            serde_json::from_str(DIFF_MANIFEST).expect("manifest parses");
        let no_ses = Path::new("/nonexistent/epic-harness/out.ses");
        let run1 = diff_test_run(base.clone(), 0, no_ses);

        // Agreement face: identical parity fields (both ses absent ->
        // None == None) must produce no drift and no note.
        let run2 = diff_test_run(base.clone(), 0, no_ses);
        let note =
            diff_double_runs(&run1, &run2, "fixtures/drift.dsn").expect("agreement must not bail");
        assert_eq!(note, None);

        // One-field drift worlds, one per parity field. exit_code is read
        // from the OracleRun (distill takes run.exit_code); the rest come
        // from the manifest.
        let mut m = base.clone();
        m.final_state = "TIMED_OUT".into();
        expect_drift_named(&run1, &diff_test_run(m, 0, no_ses), "final_state");

        expect_drift_named(&run1, &diff_test_run(base.clone(), 1, no_ses), "exit_code");

        let mut m = base.clone();
        if let Some(stats) = m.board_statistics.as_mut()
            && let Some(c) = stats.connections.as_mut()
        {
            c.incomplete_count = Some(3);
        }
        expect_drift_named(&run1, &diff_test_run(m, 0, no_ses), "incomplete_count");

        let mut m = base.clone();
        if let Some(stats) = m.board_statistics.as_mut()
            && let Some(c) = stats.connections.as_mut()
        {
            c.maximum_count = Some(51);
        }
        expect_drift_named(&run1, &diff_test_run(m, 0, no_ses), "maximum_count");

        let mut m = base.clone();
        if let Some(stats) = m.board_statistics.as_mut()
            && let Some(v) = stats.clearance_violations.as_mut()
        {
            v.total_count = Some(2);
        }
        expect_drift_named(
            &run1,
            &diff_test_run(m, 0, no_ses),
            "clearance_violations_total",
        );

        let mut m = base.clone();
        if let Some(stats) = m.board_statistics.as_mut()
            && let Some(v) = stats.clearance_violations.as_mut()
        {
            v.router_introduced_count = Some(1);
        }
        expect_drift_named(
            &run1,
            &diff_test_run(m, 0, no_ses),
            "clearance_router_introduced",
        );

        let mut m = base.clone();
        m.normalized_score = Some(986.31);
        expect_drift_named(&run1, &diff_test_run(m, 0, no_ses), "normalized_score");

        let mut m = base.clone();
        m.phases.autorouter.passes_completed = Some(17);
        expect_drift_named(&run1, &diff_test_run(m, 0, no_ses), "passes_completed");
    }

    /// The ses divergence face: equal parity fields but different ses bytes
    /// must NOT bail (the SES digest is identity info, not a parity gate) —
    /// it comes back as a note for the committed record (recorded, never
    /// normalized away), while equal ses bytes produce no note at all.
    #[test]
    fn diff_double_runs_notes_ses_divergence_and_stays_silent_on_equal_ses() {
        let base: RoutingResultManifest =
            serde_json::from_str(DIFF_MANIFEST).expect("manifest parses");
        let dir =
            std::env::temp_dir().join(format!("epic-harness-diff-ses-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let ses_a = dir.join("a.ses");
        let ses_b = dir.join("b.ses");
        std::fs::write(&ses_a, b"(session \"one\")").expect("write ses a");
        std::fs::write(&ses_b, b"(session \"two\")").expect("write ses b");

        // Equal ses bytes: no note.
        let run1 = diff_test_run(base.clone(), 0, &ses_a);
        let run2 = diff_test_run(base.clone(), 0, &ses_a);
        let note =
            diff_double_runs(&run1, &run2, "fixtures/x.dsn").expect("equal ses must not bail");
        assert_eq!(note, None);

        // Divergent ses bytes: a note naming the face, not a bail.
        let run2 = diff_test_run(base.clone(), 0, &ses_b);
        let note = diff_double_runs(&run1, &run2, "fixtures/x.dsn")
            .expect("ses divergence must be recorded, not bailed");
        let note = note.expect("a divergence must yield a note");
        assert!(note.contains("ses_sha256"), "unexpected note: {note}");
        assert!(
            note.contains("racy SES"),
            "note should mark the emission racy: {note}"
        );

        let _ = std::fs::remove_file(&ses_a);
        let _ = std::fs::remove_file(&ses_b);
    }
}
