//! Router quality scoreboard (M3 Task 15): DIRECTIONAL compare gates for
//! the Rust router against the committed T14 router-only Java records,
//! plus the Rust-vs-Rust determinism self-gate and the first-divergence
//! localizer. Java-free at run time — the Java side is the COMMITTED
//! records (`baselines/router-only/A/`); no oracle is ever spawned here
//! (record verification is T14's `verify` path, not this one).
//!
//! ## The gates (per tier fixture, anchors §2)
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
//! ## The profile face (M4-T11)
//!
//! `router compare` runs under ONE of two profiles:
//!
//! - `router-only` (THE DEFAULT — the M2/M3 regression face, byte-for-byte
//!   the pre-T11 invocation): the Rust side carries BOTH comparability
//!   flags (`--router.fanout.enabled=false --optimizer.enabled=false`),
//!   the gates read the committed T14 records (`baselines/router-only/A`),
//!   and `router determinism`'s canary faces stay the router-only digests.
//!   The default exists so the profile switch cannot rotate any default
//!   invocation's behavior — CI's bare `router compare` (GATE mode
//!   since the M8-T8 exit flip, commit `ba5e587bb`) stays this face.
//! - `full` (the M4 battery face): the Rust side drops BOTH flags and
//!   runs the assembled fanout → router → optimizer pipeline
//!   (`pipeline/full.rs`, T9/T10); the gates read the M0 full-flow jar
//!   records (`baselines/java/A`, profile marker ABSENT — captured
//!   default-settings with both stages on). Same directional gates + ε
//!   idiom. The M0 records are reused verbatim (fixture hashes verified
//!   against the current DSNs at T11); committed artifacts are never
//!   modified, so no new capture happened.
//!
//! The two profiles write scratch under separate run roots
//! (`runs/router-compare` vs `runs/router-compare-full`) so the faces
//! never clobber each other's evidence. The profile names are the
//! dispatch's; the capture/verify surfaces spell the same full profile
//! `full-flow` (`OracleProfile::from_name`) — the mapping is
//! [`CompareProfile::oracle_profile`], and any other name fails loudly
//! (never a silent fallback).
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
//!
//! REBUILD CAVEAT (witnessed M4-T4, closed STRUCTURALLY after the
//! quality review): the gates spawn the BUILT `epic-cli` binary (the
//! [`resolve_epic_cli`] resolution), and `cargo run/build -p
//! epic-harness` does NOT rebuild that bin — epic-cli is linked only
//! as a LIBRARY dependency here, so a stale `epic-cli` executable
//! silently routes with OLD engine code and the gate stays green on
//! stale bytes. TWO staleness axes were witnessed live on 2026-09-22:
//! the LIBRARY-DEP axis (a T3-era release bin masked a full A/B round)
//! and the PROFILE-SIBLING axis (a fresh release build PLUS the stale
//! pre-T4 `target/debug/epic-cli`, adopted as the debug harness's
//! sibling, reproduced the DORMANT digest with exit 0 —
//! quality-review MAJOR-1). Both are closed structurally now: the
//! gates resolve through [`resolve_fresh_epic_cli`], which BAILS when
//! the resolved bin is older than the newest workspace-crate source,
//! naming the bin and the rebuild command for BOTH profiles; and
//! [`resolve_epic_cli`] prefers the release profile and never adopts a
//! debug sibling. The e2e smokes spawn built artifacts outside these
//! gates and keep the manual rule: `cargo build -p epic-cli` (and
//! `… --release`) before judging any digest.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result, bail};
use clap::Subcommand;

use crate::baseline::{
    BASELINE_SCHEMA_VERSION, BaselineRecord, normalized_manifest_sha256, sha256_file,
};
use crate::manifest::RoutingResultManifest;
use crate::oracle::{OracleProfile, find_repo_root, parse_manifest_file, wait_with_timeout};
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

/// THE M4-T9 companion flag: the router-only Java captures ran the jar
/// with `--optimizer.enabled=false` (`OracleProfile::RouterOnly::flags`),
/// and the T9 stage is gated on the same box — the committed records'
/// scores are the OPTIMIZER-OFF face, so the compare argv must disable
/// the stage too (a bare default run now enables it). `false` never
/// parses into a WARN since M4-T9 (the route subset carries the field).
pub const OPTIMIZER_OFF_FLAG: &str = "--optimizer.enabled=false";

/// THE compare profile (M4-T11, module docs): which pipeline face the
/// battery's Rust side runs and which committed records the gates read.
/// `RouterOnly` is the DEFAULT — the pre-T11 face byte-for-byte, so the
/// profile switch cannot rotate any default invocation; `Full` is the M4
/// battery face (no comparability flags, the assembled pipeline, the M0
/// full-flow jar records in `baselines/java`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareProfile {
    /// Fanout + optimizer disabled on the Rust side (BOTH comparability
    /// flags); gates against `baselines/router-only` (the T14 records).
    RouterOnly,
    /// The assembled pipeline, NO comparability flags; gates against
    /// `baselines/java` (the M0 full-flow jar records, profile marker
    /// absent).
    Full,
}

impl CompareProfile {
    /// Parses the `--profile` CLI value; omitted means `router-only`.
    /// ANY other name is a LOUD failure (never a silent fallback to the
    /// default — a typo'd profile must not silently re-face the battery).
    /// Note: the capture/verify surfaces spell the same full profile
    /// `full-flow`; the compare face's names are `router-only|full` and
    /// `full-flow` is deliberately NOT accepted here (a compare against
    /// the full-flow records is spelled `--profile full`).
    pub fn from_name(name: Option<&str>) -> Result<Self> {
        match name {
            None | Some("router-only") => Ok(Self::RouterOnly),
            Some("full") => Ok(Self::Full),
            Some(other) => {
                bail!(
                    "unknown compare profile {other:?} (expected \"router-only\" or \"full\"; \
                     the capture/verify surface spells the full profile \"full-flow\")"
                )
            }
        }
    }

    /// The CLI spelling.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::RouterOnly => "router-only",
            Self::Full => "full",
        }
    }

    /// The oracle profile this compare face mirrors — the single source
    /// for the baselines dir (`dir_name`: `router-only` vs `java`) and the
    /// record marker (`marker`: `Some("router-only")` vs `None`). The
    /// compare gains NO second copy of that routing.
    #[must_use]
    pub fn oracle_profile(self) -> OracleProfile {
        match self {
            Self::RouterOnly => OracleProfile::RouterOnly,
            Self::Full => OracleProfile::FullFlow,
        }
    }

    /// THE argv flag suffix for this profile's subprocess runs (the one
    /// flag source [`route_argv`] appends): router-only carries BOTH
    /// comparability flags, full carries NONE (the assembled pipeline's
    /// default-settings face).
    #[must_use]
    pub fn argv_flags(self) -> &'static [&'static str] {
        match self {
            Self::RouterOnly => &[COMPARABILITY_FLAG, OPTIMIZER_OFF_FLAG],
            Self::Full => &[],
        }
    }

    /// The detail-pass argv flags — DELIBERATELY not [`Self::argv_flags`]:
    /// the router-only detail face is the historical T15 face
    /// (`COMPARABILITY_FLAG` ONLY — no optimizer flag; the subprocess
    /// gained the optimizer flag at T9, the detail pass predates it and
    /// the buglog-176 fingerprint pin is anchored to THIS face), so the
    /// router-only arm reproduces it exactly. The full arm drops both
    /// flags: the detail pass must mirror ITS profile's subprocess face
    /// (no flags). Do not "fix" the asymmetry — rotating the router-only
    /// detail face rotates the pinned fingerprint for nothing.
    #[must_use]
    pub fn detail_argv_flags(self) -> &'static [&'static str] {
        match self {
            Self::RouterOnly => &[COMPARABILITY_FLAG],
            Self::Full => &[],
        }
    }

    /// The scratch run root for this profile's battery (separate so the
    /// two faces never clobber each other's evidence — `run_cli` deletes
    /// stale outputs, but the runs/ trees are the task evidence logs).
    #[must_use]
    pub fn runs_dir_name(self) -> &'static str {
        match self {
            Self::RouterOnly => "router-compare",
            Self::Full => "router-compare-full",
        }
    }

    /// Loads one committed record for THIS profile's record set — the
    /// strict pre-flight keyed on the ORACLE profile's own marker and
    /// capture-surface name (no literals restated one layer below their
    /// source: `Some("router-only")`/`None` and the error's profile face
    /// come from [`OracleProfile::marker`]/[`OracleProfile::capture_name`]
    /// through [`Self::oracle_profile`]).
    pub fn load_record(&self, path: &Path) -> std::result::Result<BaselineRecord, String> {
        let oracle = self.oracle_profile();
        load_java_record_with_marker(path, oracle.marker(), oracle.capture_name())
    }
}

/// THE one argv builder for every `router compare` subprocess run (the
/// run faces cannot drift between sites — there is no second argv). The
/// profile's flag suffix rides EXACTLY once, as the argv suffix:
/// router-only appends BOTH comparability flags, full appends NONE.
#[must_use]
pub fn route_argv(
    cli_bin: &Path,
    dsn: &Path,
    ses_path: &Path,
    manifest_path: &Path,
    profile: CompareProfile,
) -> Vec<String> {
    let mut argv = vec![
        cli_bin.to_string_lossy().into_owned(),
        "route".to_string(),
        "-de".to_string(),
        dsn.to_string_lossy().into_owned(),
        "-do".to_string(),
        ses_path.to_string_lossy().into_owned(),
        "--result-json".to_string(),
        manifest_path.to_string_lossy().into_owned(),
    ];
    argv.extend(profile.argv_flags().iter().map(|flag| (*flag).to_string()));
    argv
}

/// [`route_argv`] plus the T7 threads face: `-mt <n>` appended when
/// `max_threads` is `Some` (the gate's one argv extension — the flag
/// value rides the same single-source argv builder so it cannot drift
/// between the run faces).
pub fn route_argv_threads(
    cli_bin: &Path,
    dsn: &Path,
    ses_path: &Path,
    manifest_path: &Path,
    profile: CompareProfile,
    max_threads: Option<usize>,
    extra_args: &[String],
) -> Vec<String> {
    let mut argv = route_argv(cli_bin, dsn, ses_path, manifest_path, profile);
    if let Some(n) = max_threads {
        argv.push("-mt".to_string());
        argv.push(n.to_string());
    }
    // The M6-T7 extension: extra router flags appended VERBATIM (the
    // settings-ON faces; empty = the historical faces, byte-identical).
    argv.extend(extra_args.iter().cloned());
    argv
}

/// The profile-aware banner phrase, DERIVED from an actual built argv —
/// if a flag ever leaves (router-only) or enters (full) [`route_argv`],
/// the log says so instead of lying (the OW-M2 "hardcoded banner" face).
/// The router-only arm derives the both-flags claim (or the MISSING face);
/// the full arm derives the no-flags claim (or the PRESENT face). Both
/// mismatch faces are loud — the banner can never lie about which face
/// the battery is running.
#[must_use]
pub fn profile_banner_phrase(profile: CompareProfile, argv: &[String]) -> String {
    match profile {
        CompareProfile::RouterOnly => {
            if argv.iter().any(|arg| arg == COMPARABILITY_FLAG)
                && argv.iter().any(|arg| arg == OPTIMIZER_OFF_FLAG)
            {
                format!(
                    "{COMPARABILITY_FLAG} {OPTIMIZER_OFF_FLAG} vs committed router-only records"
                )
            } else {
                "COMPARABILITY FLAG MISSING FROM THE ARGV vs committed router-only records".into()
            }
        }
        CompareProfile::Full => {
            if !argv.iter().any(|arg| arg == COMPARABILITY_FLAG)
                && !argv.iter().any(|arg| arg == OPTIMIZER_OFF_FLAG)
            {
                "no comparability flags (the assembled fanout → router → optimizer pipeline) vs committed full-flow records".into()
            } else {
                "COMPARABILITY FLAG PRESENT IN A FULL-PROFILE ARGV (the full face runs the default-settings pipeline; a disable flag here is the flag-gating bug)".into()
            }
        }
    }
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum RouterCommand {
    /// Per tier fixture: run `epic-cli route` under ONE compare profile —
    /// `--profile router-only` (the default; BOTH comparability flags, the
    /// committed T14 records) or `--profile full` (NO flags, the assembled
    /// fanout → router → optimizer pipeline, the M0 full-flow jar records
    /// in `baselines/java`). Every fixture in the walked tier gets an
    /// explicit verdict.
    Compare {
        /// Only run fixtures whose tiers.yaml path contains this substring
        /// (the rest still appear, marked filtered-out).
        #[arg(long)]
        fixture: Option<String>,
        /// The compare profile face (module docs): "router-only" (default)
        /// or "full". Any other value fails loudly (never a silent
        /// fallback).
        #[arg(long)]
        profile: Option<String>,
        /// CI report mode: print every verdict, always exit 0 (the T17
        /// flip is dropping this flag — see SEAM).
        #[arg(long)]
        report_only: bool,
        /// Force the in-process detail pass on EVERY fixture (doubles the
        /// routing wall time; adds trace/via/bend counts + the
        /// detail-vs-manifest consistency face per fixture).
        #[arg(long)]
        detail: bool,
        /// Which tier battery to walk (A/B/C; default A — the
        /// pre-M6-T3 face, byte-identical argv). B and C run the FULL
        /// profile only: the full-flow jar records exist for all three
        /// tiers, the router-only records for A alone (a B/C router-only
        /// battery is a loud bail, never a silent RED walk).
        #[arg(long, default_value = "A")]
        tier: String,
        /// Work-root override (default `harness/runs/router-compare`).
        #[arg(long)]
        work_root: Option<PathBuf>,
        /// Extra `--router.*` flags appended VERBATIM to every walked
        /// fixture's run argv (the M6-T10 carry-forward, design :413 —
        /// the clean settings-ON battery face: the compare verdicts
        /// fall out against the COMMITTED records, so an ON walk over
        /// a population whose records were captured at defaults will
        /// red wherever the flag matters — that IS the instrument).
        #[arg(long)]
        extra_router_arg: Vec<String>,
    },
    /// Java-free determinism self-gate: one DSN, two epic-cli runs to
    /// SEPARATE output paths, byte-identical SES + manifest expected.
    Determinism {
        /// The fixture (tiers.yaml path suffix); defaults to the smallest
        /// Tier A board (`DAC2020_boards/DAC2020_bm08.dsn`).
        #[arg(long)]
        fixture: Option<String>,
        /// Extra `--router.*` flags appended verbatim to BOTH run
        /// argvs (the M6-T7 settings-ON faces; empty = the historical
        /// face, byte-identical argv).
        #[arg(long)]
        extra_router_arg: Vec<String>,
        /// Work-root override (default `harness/runs/router-determinism`).
        #[arg(long)]
        work_root: Option<PathBuf>,
    },
    /// Java-free THREADS-INVARIANCE gate (M5-T7, the design :78
    /// "`--threads N` output is byte-identical to `--threads 1`"
    /// contract): for each gate fixture (default bm08 + bm06) run
    /// `epic-cli route` THREE times — `--threads 1`, the odd N (3) and
    /// the even N (4) — and require byte-identical SES + manifest
    /// across all three. Exit nonzero on any divergence.
    ThreadsInvariance {
        /// Only run this fixture (tiers.yaml path suffix); default runs
        /// both gate fixtures (bm08 + bm06).
        #[arg(long)]
        fixture: Option<String>,
        /// The even thread count (default 4).
        #[arg(long, default_value_t = 4)]
        even_threads: usize,
        /// The odd thread count (default 3).
        #[arg(long, default_value_t = 3)]
        odd_threads: usize,
        /// Extra `--router.*` flags appended verbatim to ALL THREE
        /// run argvs (the M6-T7 settings-ON witness face; empty = the
        /// historical face).
        #[arg(long)]
        extra_router_arg: Vec<String>,
        /// Work-root override (default `harness/runs/router-threads`).
        #[arg(long)]
        work_root: Option<PathBuf>,
    },
}

pub fn run(cmd: RouterCommand) -> Result<()> {
    let repo_root = find_repo_root()?;
    match cmd {
        RouterCommand::Compare {
            fixture,
            profile,
            tier,
            report_only,
            detail,
            work_root,
            extra_router_arg,
        } => compare_cmd(
            &repo_root,
            fixture.as_deref(),
            CompareProfile::from_name(profile.as_deref())?,
            &tier,
            report_only,
            detail,
            work_root,
            &extra_router_arg,
        ),
        RouterCommand::Determinism {
            fixture,
            extra_router_arg,
            work_root,
        } => determinism_cmd(&repo_root, fixture.as_deref(), &extra_router_arg, work_root),
        RouterCommand::ThreadsInvariance {
            fixture,
            even_threads,
            odd_threads,
            extra_router_arg,
            work_root,
        } => threads_invariance_cmd(
            &repo_root,
            fixture.as_deref(),
            even_threads,
            odd_threads,
            &extra_router_arg,
            work_root,
        ),
    }
}

// ---------------------------------------------------------------------------
// The Tier A fixture enum (tiers.yaml authoritative; trap 2: no silent skip)
// ---------------------------------------------------------------------------

/// The baselines directory for ONE tier under the profile's record set
/// (single-sourced through [`CompareProfile::oracle_profile`]: the
/// router-only records live in `baselines/router-only/{tier}`, the M0
/// full-flow jar records in `baselines/java/{tier}` — router-only
/// committed for A alone, full-flow for A/B/C).
#[must_use]
pub fn profile_tier_baselines_dir(
    repo_root: &Path,
    profile: CompareProfile,
    tier: &str,
) -> PathBuf {
    repo_root
        .join("rust/harness/baselines")
        .join(profile.oracle_profile().dir_name())
        .join(tier)
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
    /// The committed record for the fixture under the walk's profile
    /// (the `profile.oracle_profile().dir_name()` record set: the T14
    /// router-only records vs the M0 full-flow `java` set).
    pub baseline_path: PathBuf,
}

/// The Tier A fixture walk IN tiers.yaml ORDER, each with its committed
/// record path FOR THE PROFILE's record set. The enum completeness is
/// pinned for BOTH profiles: dropping a fixture here is the finding-class
/// trap 2 names.
pub fn tier_a_fixtures(repo_root: &Path, profile: CompareProfile) -> Result<Vec<RouterFixture>> {
    tier_fixtures(repo_root, profile, "A")
}

/// The fixture walk for ONE tier (the M6-T3 tier filter): tiers.yaml
/// order, each with its committed record path under the profile's
/// `{tier}` record set. `tier_a_fixtures` is the `"A"` spelling.
pub fn tier_fixtures(
    repo_root: &Path,
    profile: CompareProfile,
    tier_name: &str,
) -> Result<Vec<RouterFixture>> {
    let tiers_path = repo_root.join("rust/harness/config/tiers.yaml");
    let tier_file = TierFile::load(&tiers_path)?;
    let tier = tier_file
        .tier(tier_name)
        .with_context(|| format!("tier {tier_name} missing from {}", tiers_path.display()))?;
    let fixtures_root = tier_file.fixtures_root.display().to_string();
    let baselines = profile_tier_baselines_dir(repo_root, profile, tier_name);
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

/// The strict record pre-flight shared by BOTH profile loaders: exists,
/// parses (`deny_unknown_fields`), schema current, engine `java`, and the
/// record's profile marker equals the profile's expected marker exactly.
/// The error is a plain String so callers can tally every offender into
/// explicit verdicts (never a silent skip).
fn load_java_record_with_marker(
    path: &Path,
    expected_marker: Option<&str>,
    profile_desc: &str,
) -> std::result::Result<BaselineRecord, String> {
    if !path.is_file() {
        return Err(format!(
            "missing baseline record — the {profile_desc} capture never wrote it"
        ));
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
    if record.profile.as_deref() != expected_marker {
        return Err(format!(
            "profile {:?} — the compare gates against {profile_desc} records",
            record.profile
        ));
    }
    Ok(record)
}

// ---------------------------------------------------------------------------
// The epic-cli subprocess
// ---------------------------------------------------------------------------

/// Locates the `epic-cli` binary: `EPIC_CLI` env, the harness binary's
/// RELEASE-profile sibling (cargo puts bin targets of one profile in
/// the same `target/<profile>/` dir), or the repo's
/// `target/{release,debug}`. Never spawns anything.
///
/// Two buglog-184 hardenings (quality-review MAJOR-1, witnessed live
/// 2026-09-22): a DEBUG sibling is deliberately NOT adopted — `cargo
/// run -p epic-harness` never rebuilds it, and the debug harness's
/// stale pre-T4 sibling reproduced the dormant digest under a green
/// gate — and the repo fallback prefers RELEASE, the documented
/// official profile for every spawn-backed gate (debug stays the CI
/// face, where the debug bin is built fresh one step before the gate).
/// Spawn-backed gates resolve through [`resolve_fresh_epic_cli`],
/// which adds the structural staleness guard.
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
        && parent.file_name().is_some_and(|name| name == "release")
    {
        let sibling = parent.join("epic-cli");
        if sibling.is_file() {
            return Ok(sibling);
        }
    }
    for profile in ["release", "debug"] {
        let candidate = repo_root.join("rust/target").join(profile).join("epic-cli");
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    bail!(
        "epic-cli binary not found — build it once with `cargo build -p epic-cli` (or set EPIC_CLI)"
    )
}

/// Newest `(mtime, path)` over the workspace-crate sources the spawned
/// `epic-cli` BIN is built from — `rust/crates/*/src/**/*.rs`, each
/// crate's `Cargo.toml`, and `rust/Cargo.lock` (consumed by
/// [`assert_epic_cli_fresh`]). `None` when `rust/crates` is absent (a
/// non-workspace layout skips the guard rather than guessing); stray
/// unreadable entries are skipped — the guard's job is the ordinary
/// staleness axis, not to brick exotic checkouts.
fn newest_workspace_source(repo_root: &Path) -> Option<(SystemTime, PathBuf)> {
    let crates_dir = repo_root.join("rust/crates");
    if !crates_dir.is_dir() {
        return None;
    }
    let mut newest: Option<(SystemTime, PathBuf)> = None;
    let mut consider = |path: &Path| {
        let mtime = std::fs::metadata(path)
            .and_then(|meta| meta.modified())
            .ok();
        if let Some(mtime) = mtime
            && newest.as_ref().is_none_or(|(best, _)| mtime > *best)
        {
            newest = Some((mtime, path.to_path_buf()));
        }
    };
    // Workspace-level lock: a dependency bump alone can change the
    // built bin.
    consider(&repo_root.join("rust/Cargo.lock"));
    for crate_entry in std::fs::read_dir(&crates_dir).ok()?.flatten() {
        let crate_dir = crate_entry.path();
        if !crate_dir.is_dir() {
            continue;
        }
        consider(&crate_dir.join("Cargo.toml"));
        let mut stack = vec![crate_dir.join("src")];
        while let Some(dir) = stack.pop() {
            let entries = match std::fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    consider(&path);
                }
            }
        }
    }
    newest
}

/// Buglog-184's structural guard (quality-review MAJOR-1): the
/// spawn-backed gates BAIL when the resolved bin is OLDER than the
/// newest workspace-crate source — it would route with OLD engine code
/// and the gate could stay GREEN on stale bytes, which a prose caveat
/// alone failed to prevent on the profile-sibling axis. Bail-over-
/// auto-build BY DESIGN: an automatic `cargo build` nested under
/// `cargo run` contends with cargo's own file lock, so the message
/// names both rebuild profiles instead. False positives are safe:
/// `git checkout` refreshes source mtimes to now, so a bail's worst
/// case is a no-op rebuild.
pub fn assert_epic_cli_fresh(repo_root: &Path, cli_bin: &Path) -> Result<()> {
    let bin_mtime = std::fs::metadata(cli_bin)
        .and_then(|meta| meta.modified())
        .map_err(|e| anyhow::anyhow!("stating resolved epic-cli bin {}: {e}", cli_bin.display()))?;
    let Some((newest_mtime, newest_path)) = newest_workspace_source(repo_root) else {
        return Ok(()); // not a workspace checkout — nothing to compare against
    };
    if bin_mtime >= newest_mtime {
        return Ok(());
    }
    let epoch_secs = |t: SystemTime| {
        t.duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    };
    bail!(
        "STALE epic-cli bin: {}\n  \
         bin mtime {}s (epoch) is OLDER than the newest crate source {} at {}s (epoch).\n  \
         The gate would spawn OLD engine code and can stay GREEN on stale bytes (buglog-184; \
         the profile-sibling face was witnessed live 2026-09-22 — a fresh release build plus \
         a stale debug sibling reproduced the dormant digest with exit 0).\n  \
         The staleness scan is WORKSPACE-WIDE: an edit in ANY crate stale-flags this bin\n  \
         even when epic-cli's own sources are untouched, and a plain `cargo build -p\n  \
         epic-cli` may NOT clear the flag — an up-to-date target can skip the relink,\n  \
         leaving the bin's mtime old (M10 AM6 lesson 19).\n  \
         FORCE the relink (the witnessed TRUE relink is the clean below,\n  \
         buglog 220), then rebuild BOTH profiles before judging any digest:\n    \
         cargo clean -p epic-cli\n    \
         cargo build -p epic-cli\n    \
         cargo build -p epic-cli --release\n  \
         (from rust/; or point EPIC_CLI at a freshly built bin)",
        cli_bin.display(),
        epoch_secs(bin_mtime),
        newest_path.display(),
        epoch_secs(newest_mtime),
    )
}

/// The spawn-backed gates' ONE bin entry ([`determinism_cmd`] and
/// [`compare_cmd`]): [`resolve_epic_cli`] plus the structural
/// freshness guard [`assert_epic_cli_fresh`].
pub fn resolve_fresh_epic_cli(repo_root: &Path) -> Result<PathBuf> {
    let cli_bin = resolve_epic_cli(repo_root)?;
    assert_epic_cli_fresh(repo_root, &cli_bin)?;
    Ok(cli_bin)
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

/// Runs `epic-cli route` under the profile's flag face into `work_dir`
/// (`out.ses`, `manifest.json`, `stdout.log`, `stderr.log`) with a harness
/// timeout. Stale outputs are deleted BEFORE the spawn so a run that dies
/// early never reads the previous run's files as its own. (The T7 threads
/// face: `-mt <n>` when `max_threads` is `Some`; the M7-T4 extras face
/// appends `extra_args` VERBATIM — the ONE argv source for every
/// spawned run.)
pub fn run_cli_threads(
    cli_bin: &Path,
    dsn: &Path,
    work_dir: &Path,
    timeout: Duration,
    profile: CompareProfile,
    max_threads: Option<usize>,
    extra_args: &[String],
) -> Result<CliRun> {
    let ses_path = work_dir.join(RUN_SES_FILE);
    let manifest_path = work_dir.join(RUN_MANIFEST_FILE);
    std::fs::create_dir_all(work_dir)
        .with_context(|| format!("creating work dir {}", work_dir.display()))?;
    let _ = std::fs::remove_file(&ses_path);
    let _ = std::fs::remove_file(&manifest_path);

    // THE single argv source ([`route_argv`]/[`route_argv_threads`]):
    // the comparability flag cannot drift between the run faces.
    let argv = route_argv_threads(
        cli_bin,
        dsn,
        &ses_path,
        &manifest_path,
        profile,
        max_threads,
        extra_args,
    );
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

/// Runs the in-process detail pass behind the T17b wall watchdog: the
/// body executes on a worker thread and the caller waits with a
/// [`Duration`] deadline. Three verdict classes, each DISTINGUISHABLE
/// in the returned `Err`/`Ok` shape:
///
/// * the pass finishes inside the deadline → the body's OWN result
///   untouched (`Ok(RustDetail)` compared by the directional gates, or
///   `Err("detail pass panicked: …")`) — the watchdog is silent and the
///   deterministic outcome is byte-for-byte what an unwatched run
///   produces (the engine has no thread-local state; the worker is a
///   plain relocation);
/// * the deadline overruns → `Err("detail pass deadline exceeded after
///   …s …")` — the wall bound, distinct from a panic and from a
///   quality failure;
/// * the worker cannot start / exits without reporting → a distinct
///   infra `Err`.
///
/// The default panic hook is silenced for the wait (spec-review NOTE:
/// the hook would print the caught panic to stderr before the harness
/// renders its own NOTE) and RESTORED ON EVERY PATH OUT — the T17b
/// deadline path included (the RAII `panic_hook::Silenced` guard's
/// Drop; the hook swap is process-global, so TEST callers must hold
/// `panic_hook::lock()` around this call — the three watchdog pins
/// do, and corpus::compare takes the same lock for its own window;
/// quality-review T17b M-Q2). On a deadline the worker LEAKS by design (Rust has no safe
/// thread kill): it owns its board, writes only its process-unique
/// scratch ses, cannot reach the receiver again, and dies at process
/// exit; if it panics after the restore the restored hook fires —
/// stderr noise, never corruption. The engine's tick-budget currency
/// is untouched (controller constraint, buglog 175): only the WALL
/// bound is non-deterministic.
fn run_detail_pass(
    dsn: &Path,
    deadline: Duration,
    profile: CompareProfile,
) -> Result<RustDetail, String> {
    // RAII silence (quality-review T17b M-Q2): the drop restores the
    // previous hook on EVERY path out — the T17b deadline path
    // included (the sentinel's asserted invariant). NO lock HERE: the
    // sentinel pin holds `panic_hook::lock()` across this call and
    // std's Mutex is not reentrant, so the LOCK is the callers'
    // contract — the three watchdog pins hold it; the bin's
    // compare_cmd is the only participant in its own process.
    let _silence = crate::panic_hook::Silenced::new();
    let body = move |dsn: &Path| {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            route_detail_inner(dsn, profile).map_err(|e| e.to_string())
        }))
        .unwrap_or_else(|payload| {
            Err(format!(
                "detail pass panicked: {}",
                payload
                    .downcast_ref::<&str>()
                    .map(|s| (*s).to_string())
                    .or_else(|| payload.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "unknown panic payload".into())
            ))
        })
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let worker_dsn = dsn.to_path_buf();
    let worker = std::thread::Builder::new()
        .name("epic-detail-pass".to_string())
        .spawn(move || {
            let _ = tx.send(body(&worker_dsn));
        });
    let outcome = match worker {
        Ok(_) => match rx.recv_timeout(deadline) {
            Ok(result) => Some(result),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => None,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Some(Err(
                "detail pass worker exited without reporting".to_string(),
            )),
        },
        Err(spawn_err) => Some(Err(format!(
            "detail pass worker could not start: {spawn_err}"
        ))),
    };
    // The restore runs on EVERY path — the deadline path included
    // (T17b: this is the sentinel's asserted invariant) — via the
    // `_silence` guard's Drop, which fires before this function's
    // return value reaches the caller.
    outcome.unwrap_or_else(|| {
        Err(format!(
            "detail pass deadline exceeded after {:.0}s (wall watchdog; the engine outcome was not observed — a bound, not a quality verdict or a panic)",
            deadline.as_secs_f32()
        ))
    })
}

/// The detail pass body (panic boundary lives in [`run_detail_pass`]).
fn route_detail_inner(dsn: &Path, profile: CompareProfile) -> Result<RustDetail, anyhow::Error> {
    use epic_board::board::Board;
    use epic_board::tree_manager::SearchTreeManager;
    use epic_cli::route::CliDriverSink;
    use epic_dsn::reader::{DsnReadResult, read_board};
    use epic_dsn::ses_board::SesBoard;
    use epic_engine::settings::{
        DsnLayer, MergedSettings, ResolvedRouteSettings, apply_board_specific_optimizations,
        build_batch_settings, merge, parse_route_args, validate,
    };
    use epic_router::pipeline::batch::{BatchDriver, StopFace};
    use epic_router::pipeline::board_statistics::BoardStatistics;

    // The profile's detail argv face ([`CompareProfile::detail_argv_flags`]):
    // router-only = the historical T15 face (fanout flag ONLY — the pinned
    // buglog-176 fingerprint face), full = NO flags (the assembled
    // pipeline). The fanout coupling flows through the SAME
    // build_batch_settings the subprocess uses. The scratch path exists
    // only so parse_route_args sees a complete argv face — the in-process
    // pass NEVER writes the session file.
    let scratch_ses = std::env::temp_dir().join(format!(
        "epic-t15-detail-{}-{}.ses",
        std::process::id(),
        dsn.file_name().and_then(|n| n.to_str()).unwrap_or("board")
    ));
    let mut argv = vec![
        "-de".to_string(),
        dsn.to_string_lossy().into_owned(),
        "-do".to_string(),
        scratch_ses.to_string_lossy().into_owned(),
    ];
    // The profile's detail flags — for RouterOnly the SAME const the
    // subprocess argv builder uses (single-sourced), for Full nothing.
    argv.extend(
        profile
            .detail_argv_flags()
            .iter()
            .map(|flag| (*flag).to_string()),
    );
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

    // Settings: the same resolution chain as run_route step 1b —
    // hoisted above the board build so the override can consume the
    // merged settings at its parse-time call point.
    let dsn_layer = DsnLayer::from_metadata(
        ses.metadata.autoroute_settings.as_ref(),
        usize::try_from(ses.metadata.layer_count).unwrap_or(0),
    );
    let mut merged = merge(&MergedSettings::default(), &dsn_layer, &args.layer);
    for warning in validate(&mut merged) {
        eprintln!("Warning: {warning}");
    }

    let mut board = Board::from_ses_board(&ses);
    let mut manager = SearchTreeManager::new();
    manager.reinsert_tree_items(&mut board);
    // The manager's parse-time copper-to-edge override (Java
    // `createBoard` :346, BEFORE `Wiring.java:347` normalizeAllTraces)
    // — the detail pass MUST mirror the subprocess face it localizes
    // (the buglog-189 fix; the seed below reads post-override state,
    // exactly like Java's deferred DRC at `:788-793`).
    epic_engine::session::apply_copper_to_edge_clearance_override(
        &merged,
        &mut manager,
        &mut board,
    );
    epic_board::normalize_all::normalize_all_traces(&mut manager, &mut board);

    // The load-time violation seed (the flow's step 2b — post-override,
    // Java's deferred-seed state).
    let (pre_total, _) =
        epic_drc::clearance::all_clearance_violation_depths(&mut manager, &mut board);
    board.pre_existing_clearance_violations_count = i32::try_from(pre_total).unwrap_or(i32::MAX);

    apply_board_specific_optimizations(&mut merged, &board);
    let resolved = ResolvedRouteSettings::resolve(&merged, args.deterministic_budgets);
    let batch = build_batch_settings(&resolved);

    // The batch driver (the same face as the subprocess).
    let mut sink = CliDriverSink;
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

/// The canonical artifact names inside one run's work dir — single-sourced
/// between [`run_cli`] (which WRITES them) and [`pair_from_runs`] (which
/// READS them), so the runner and the pair builder cannot drift.
pub const RUN_SES_FILE: &str = "out.ses";
pub const RUN_MANIFEST_FILE: &str = "manifest.json";

/// Builds the two runs' artifact pair from their SEPARATE work dirs
/// (spec-rereview MINOR-A hardening, NEW-B survivor): run2's paths derive
/// from `run2_dir` ONLY. The construction lives in one named function so
/// the pin can hold it — a mutant that builds run2 from run1's dir
/// aliases the pair and dies on `pair_from_runs_derives_run2_from_run2_dir_only`.
#[must_use]
pub fn pair_from_runs(run1_dir: &Path, run2_dir: &Path) -> DeterminismPair {
    DeterminismPair {
        run1: DeterminismArtifacts {
            ses: run1_dir.join(RUN_SES_FILE),
            manifest: run1_dir.join(RUN_MANIFEST_FILE),
        },
        run2: DeterminismArtifacts {
            ses: run2_dir.join(RUN_SES_FILE),
            manifest: run2_dir.join(RUN_MANIFEST_FILE),
        },
    }
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
/// The manifest digests are the VERSION-BLIND face
/// ([`normalized_manifest_sha256`]: the M10-T5 handoff — a release bump
/// can never rotate a determinism digest; the ses digests stay raw).
pub fn determinism_digests(pair: &DeterminismPair) -> Result<DeterminismDigests> {
    let digest = |path: &Path| -> Result<Option<String>> {
        path.is_file().then(|| sha256_file(path)).transpose()
    };
    let manifest_digest = |path: &Path| -> Result<Option<String>> {
        path.is_file()
            .then(|| normalized_manifest_sha256(path))
            .transpose()
    };
    Ok((
        digest(&pair.run1.ses)?,
        digest(&pair.run2.ses)?,
        manifest_digest(&pair.run1.manifest)?,
        manifest_digest(&pair.run2.manifest)?,
    ))
}

fn determinism_cmd(
    repo_root: &Path,
    fixture: Option<&str>,
    extra_router_args: &[String],
    work_root: Option<PathBuf>,
) -> Result<()> {
    let started = Instant::now();
    let cli_bin = resolve_fresh_epic_cli(repo_root)?;
    // THE DETERMINISM GATE STAYS ROUTER-ONLY (the dispatch tripwire: the
    // router-only determinism faces — canary ses `253e7b14…` — must not
    // rotate; the full profile's determinism witness is the bm08 ×2
    // full-flow digest reproduction, not this gate).
    let fixtures = tier_a_fixtures(repo_root, CompareProfile::RouterOnly)?;
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
    let run1 = run_cli_threads(
        &cli_bin,
        &dsn,
        &run1_dir,
        Duration::from_secs(entry.timeout_seconds),
        CompareProfile::RouterOnly,
        None,
        extra_router_args,
    )?;
    let run2 = run_cli_threads(
        &cli_bin,
        &dsn,
        &run2_dir,
        Duration::from_secs(entry.timeout_seconds),
        CompareProfile::RouterOnly,
        None,
        extra_router_args,
    )?;
    // The digest sources are a STRUCTURAL run1/run2 pair (review OW-M3 +
    // MINOR-A hardening): the construction is the pinned `pair_from_runs`
    // helper, so neither the path naming nor the run2-from-run1 aliasing
    // can slip through unobserved.
    let pair = pair_from_runs(&run1_dir, &run2_dir);
    // Call-site consistency (spec-rereview-2 MINOR-C / NEW-D survivor):
    // the helper's ARGUMENTS are the one face no pin can reach (pins see
    // the helper's output, not this call site), so the call site asserts
    // its own wiring against the RUNNER's truth. Under the NEW-D mutant
    // (`pair_from_runs(&run1_dir, &run1_dir)`) run2 is never read and the
    // HARD gate prints a false OK — here the aliasing dies loudly INSTEAD.
    assert_eq!(
        pair.run1.ses, run1.ses_path,
        "determinism call-site slip: run1's SES digest source must be run1's own output file"
    );
    assert_eq!(
        pair.run2.ses, run2.ses_path,
        "determinism call-site slip: run2's SES digest source must be run2's own output file (NEW-D guard)"
    );
    assert_eq!(
        pair.run1.manifest,
        run1_dir.join(RUN_MANIFEST_FILE),
        "determinism call-site slip: run1's manifest digest source must be run1's own dir"
    );
    assert_eq!(
        pair.run2.manifest,
        run2_dir.join(RUN_MANIFEST_FILE),
        "determinism call-site slip: run2's manifest digest source must be run2's own dir"
    );
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
// Threads-invariance gate (M5-T7)
// ---------------------------------------------------------------------------

/// The DEFAULT three thread faces one fixture runs under (the golden,
/// the odd N, the even N) — the command's `--odd-threads`/
/// `--even-threads` overrides replace slots 1/2, and the verdict names
/// the ACTUAL faces it is handed (quality round Q1: the diagnostics
/// must not lie about which face diverged).
pub const THREADS_GATE_FACES: [usize; 3] = [1, 3, 4];

/// The threads-invariance verdict over the three runs' digests: OK
/// line, or an Err naming the first divergent (or missing) face.
/// Pure — planted-divergence pinnable.
pub fn threads_invariance_verdict(
    digests: [(Option<String>, Option<String>); 3],
    faces: [usize; 3],
) -> std::result::Result<String, String> {
    for (index, (ses, manifest)) in digests.iter().enumerate() {
        if ses.is_none() || manifest.is_none() {
            return Err(format!(
                "the -mt {} run produced no session or manifest (ses={ses:?} manifest={manifest:?})",
                faces[index]
            ));
        }
    }
    let ses_1 = digests[0].0.clone().expect("checked above");
    let manifest_1 = digests[0].1.clone().expect("checked above");
    for index in 1..3 {
        if digests[index].0 != digests[0].0 {
            return Err(format!(
                "SES bytes diverge between -mt {} and -mt {} — threads invariance broken",
                faces[0], faces[index]
            ));
        }
        if digests[index].1 != digests[0].1 {
            return Err(format!(
                "manifest bytes diverge between -mt {} and -mt {} — threads invariance broken",
                faces[0], faces[index]
            ));
        }
    }
    Ok(format!(
        "threads invariance: OK (ses {ses_1}, manifest {manifest_1} byte-identical across \
         -mt {}/{}/{})",
        faces[0], faces[1], faces[2]
    ))
}

/// The default gate fixtures (tiers.yaml path suffixes): the smallest
/// Tier A board + the parallelism-sensitive middle board.
pub const THREADS_GATE_FIXTURES: [&str; 2] = [
    "DAC2020_boards/DAC2020_bm08.dsn",
    "DAC2020_boards/DAC2020_bm06.dsn",
];

fn threads_invariance_cmd(
    repo_root: &Path,
    fixture: Option<&str>,
    even_threads: usize,
    odd_threads: usize,
    extra_router_args: &[String],
    work_root: Option<PathBuf>,
) -> Result<()> {
    let started = Instant::now();
    let cli_bin = resolve_fresh_epic_cli(repo_root)?;
    let fixtures = tier_a_fixtures(repo_root, CompareProfile::RouterOnly)?;
    let wanted: Vec<&str> = fixture
        .map(|f| vec![f])
        .unwrap_or_else(|| THREADS_GATE_FIXTURES.to_vec());
    let work = work_root
        .map(|p| {
            if p.is_absolute() {
                p
            } else {
                repo_root.join(p)
            }
        })
        .unwrap_or_else(|| repo_root.join("rust/harness/runs/router-threads"));
    let mut ran = 0usize;
    for wanted_path in &wanted {
        let entry = fixtures
            .iter()
            .find(|f| f.tier_path == *wanted_path)
            .with_context(|| {
                format!("fixture {wanted_path:?} is not a Tier A fixture (tiers.yaml paths only)")
            })?;
        let dsn = repo_root.join(&entry.repo_rel_dsn);
        // Slot 0 (the golden) is always 1; the flags replace the odd
        // and even slots of the DEFAULT face set.
        let mut faces = THREADS_GATE_FACES;
        faces[1] = odd_threads;
        faces[2] = even_threads;
        println!(
            "router threads-invariance: {} (faces -mt 1/-mt {odd_threads}/-mt {even_threads}, fanout-off, timeout {}s)",
            entry.tier_path, entry.timeout_seconds
        );
        let mut digests: [(Option<String>, Option<String>); 3] =
            [(None, None), (None, None), (None, None)];
        for (index, threads) in faces.iter().enumerate() {
            let run_dir = work.join(format!(
                "{}-mt{}",
                entry
                    .tier_path
                    .rsplit('/')
                    .next()
                    .unwrap_or("fixture")
                    .trim_end_matches(".dsn"),
                threads
            ));
            let run = run_cli_threads(
                &cli_bin,
                &dsn,
                &run_dir,
                Duration::from_secs(entry.timeout_seconds),
                CompareProfile::RouterOnly,
                Some(*threads),
                extra_router_args,
            )?;
            let ses_digest = std::fs::File::open(&run.ses_path)
                .ok()
                .and_then(|_| sha256_file(&run.ses_path).ok());
            let manifest_path = run_dir.join(RUN_MANIFEST_FILE);
            // The version-blind face (the fix-round Q3 — the one
            // manifest-digest site the A' handoff missed): a release
            // bump can never rotate a threads-invariance digest either.
            // Intra-build the verdict was already version-invariant
            // (the same binary across -mt 1/3/4); this closes the last
            // raw manifest-hash site.
            let manifest_digest = std::fs::File::open(&manifest_path)
                .ok()
                .and_then(|_| normalized_manifest_sha256(&manifest_path).ok());
            println!(
                "  -mt {threads}: exit {:?} wall {:.1}s ses {:?}",
                run.exit_code,
                run.wall_seconds,
                ses_digest.as_deref().map(|d| &d[..12])
            );
            digests[index] = (ses_digest, manifest_digest);
            ran += 1;
        }
        match threads_invariance_verdict(digests, faces) {
            Ok(line) => println!("  {line}"),
            Err(reason) => {
                bail!(
                    "router threads-invariance FAILED for {}: {reason}",
                    entry.tier_path
                )
            }
        }
    }
    if ran == 0 {
        bail!("router threads-invariance ran no fixtures");
    }
    println!(
        "router threads-invariance: {} run(s) in {:.1}s",
        ran,
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// `router compare`
// ---------------------------------------------------------------------------

/// The no-extras RECORD SWEEP, shared (the M6 banked harness NIT —
/// formerly two hand-rolled walk loops in the tier pins): walks
/// `baselines_dir` RECURSIVELY and returns the relative paths of every
/// committed `.baseline.json` record that maps to NO entry of
/// `walked` — a stale or misnamed record the battery silently ignores
/// is the finding class this kills (quality MINOR-3).
#[cfg(test)]
fn record_extras(baselines_dir: &Path, walked: &std::collections::BTreeSet<String>) -> Vec<String> {
    let mut extras: Vec<String> = Vec::new();
    let mut stack = vec![baselines_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("record dir") {
            let entry = entry.expect("dir entry");
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "json") {
                let name = entry.file_name().into_string().expect("utf8");
                if !name.ends_with(".baseline.json") {
                    continue;
                }
                let rel = path
                    .strip_prefix(baselines_dir)
                    .expect("under the record dir")
                    .to_string_lossy()
                    .into_owned();
                if !walked.contains(&rel) {
                    extras.push(rel);
                }
            }
        }
    }
    extras
}

#[allow(clippy::too_many_arguments)] // the Java-comparable battery face, kept flat
fn compare_cmd(
    repo_root: &Path,
    fixture_filter: Option<&str>,
    profile: CompareProfile,
    tier: &str,
    report_only: bool,
    force_detail: bool,
    work_root: Option<PathBuf>,
    extra_router_args: &[String],
) -> Result<()> {
    let started = Instant::now();
    // Tier validation comes FIRST (the walk errors on an unknown tier
    // name): a typo'd --tier gets the "tier {t} missing from
    // tiers.yaml" diagnostic under BOTH profiles, before the guard
    // below and before any bin resolution (MINOR-4, quality round).
    let fixtures = tier_fixtures(repo_root, profile, tier)?;
    // The profile guard fires AFTER tier validation and BEFORE bin
    // resolution: a B/C battery without the full-flow records is a
    // loud bail, never a silent 9-RED "missing baseline record" walk
    // (the M6-T3 chore). Valid tiers only — a typo'd tier never
    // reaches this arm (the walk above bailed first).
    if tier != "A" && profile == CompareProfile::RouterOnly {
        bail!(tier_profile_guard_bail(tier));
    }
    let cli_bin = resolve_fresh_epic_cli(repo_root)?;
    let work = work_root
        .map(|p| {
            if p.is_absolute() {
                p
            } else {
                repo_root.join(p)
            }
        })
        .unwrap_or_else(|| {
            repo_root
                .join("rust/harness/runs")
                .join(profile.runs_dir_name())
        });
    // The banner's face claim is DERIVED from the actual argv builder
    // (not hardcoded): if a comparability flag ever leaves (router-only)
    // or enters (full) [`route_argv`], the log says so instead of lying
    // (OW-M2).
    let banner_comparability = profile_banner_phrase(
        profile,
        &route_argv(
            &cli_bin,
            Path::new("<fixture.dsn>"),
            Path::new("<out.ses>"),
            Path::new("<manifest.json>"),
            profile,
        ),
    );
    println!(
        "{}",
        battery_banner(
            profile.name(),
            fixtures.len(),
            tier,
            SCORE_RELATIVE_EPSILON * 100.0,
            &banner_comparability,
            if report_only {
                "report (exit 0)"
            } else {
                "gate"
            },
        )
    );
    let mut red = 0usize;
    let mut green = 0usize;
    let mut filtered = 0usize;
    // The Σ-decomposition accumulator (the M6 banked harness NIT):
    // the summary's elapsed is WHOLE-LOOP wall (detail re-runs, record
    // loads, printing included) — the Σ of the SUBPROCESS RUN walls is
    // the reconcilable constituent face (DNR-18: every adopted
    // aggregate is reconciled against its constituents; the printed
    // pair makes the gap a first-class row instead of an undisclosed
    // difference).
    let mut subprocess_run_wall_sum = 0f64;
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
        // a RED verdict, never a skip. The loader is profile-keyed
        // ([`CompareProfile::load_record`]: router-only records carry
        // `Some("router-only")`, full-flow records NO marker — the M0
        // capture face — single-sourced through the oracle profile).
        let java = match profile.load_record(&entry.baseline_path) {
            Ok(record) => record,
            Err(reason) => {
                red += 1;
                println!("  [RED] {}  baseline: {reason}", entry.tier_path);
                continue;
            }
        };
        let dsn = repo_root.join(&entry.repo_rel_dsn);
        let work_dir = work.join(&entry.tier_path);
        let run = run_cli_threads(
            &cli_bin,
            &dsn,
            &work_dir,
            Duration::from_secs(entry.timeout_seconds),
            profile,
            None,
            extra_router_args,
        )
        .with_context(|| format!("spawning the run for {}", entry.tier_path))?;
        subprocess_run_wall_sum += run.wall_seconds;
        let outcome = RustOutcome::from_run(&run);
        let failures = compare_directional(&java, &outcome);
        let mut report = extract_report_only(&java, &run);

        // The detail pass: forced by --detail, otherwise only on a red
        // gate (the localizer). Panic-caught + deadline-bounded inside
        // — the subprocess tier timeout doubles as the detail wall (the
        // same bound both faces of the fixture's run get). A KILLED run
        // (harness timeout) is EXCLUDED unless --detail forces it: the
        // in-process re-run on a wall-truncated board can only re-burn
        // the same wall up to its own deadline — it localizes nothing
        // for a slowness timeout — so without the exclusion one fixture
        // burns TWO tier walls and its verdict row waits out the second
        // in silence (the T17b controller round: a 3400s external bound
        // died inside bm01's post-kill detail pass, zero rows printed).
        let mut detail: Option<RustDetail> = None;
        if detail_pass_should_run(force_detail, &failures, run.timed_out) {
            match run_detail_pass(&dsn, detail_deadline(entry), profile) {
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
        } else if run.timed_out {
            // The killed-skip arm (the only reachable else-if case:
            // killed ⇒ integrity failure ⇒ failures non-empty, so an
            // unforced killed run lands here exactly when the guard
            // excluded it). Explicit, so the row's missing detail
            // fields never read as an accident.
            println!(
                "  NOTE {}: detail pass skipped — the routing run was killed at the tier wall (a re-run would only re-burn it); the verdict stands on the integrity gate; pass --detail to force it.",
                entry.tier_path
            );
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
        "router compare: {green} green, {red} red, {filtered} filtered-out in {:.1}s \
         (Σ subprocess run walls {:.1}s, mode: {})",
        started.elapsed().as_secs_f64(),
        subprocess_run_wall_sum,
        if report_only { "report" } else { "gate" }
    );
    // A filter that matched NOTHING must not false-green: gate mode's
    // `red == 0` is vacuously true when zero fixtures ran, in BOTH modes
    // (quality-review MINOR-2, witnessed live with a typo'd filter).
    if let Some(filter) = fixture_filter
        && green + red == 0
    {
        bail!("{}", empty_filter_bail(filter, tier));
    }
    // The exit ladder, extracted pure for the pin (MINOR-3).
    battery_exit(report_only, red)
}

/// The battery banner's single line, ONE template for every tier (the
/// profile name, fixture count, and tier letter interpolate; the
/// default A face is byte-identical to the pre-M6-T3 literal). Pure so
/// the pin can hold the exact rendered strings (quality-round MINOR-2,
/// the `battery_exit` extraction discipline).
fn battery_banner(
    profile_name: &str,
    fixture_count: usize,
    tier: &str,
    epsilon_pct: f64,
    comparability_phrase: &str,
    mode: &str,
) -> String {
    format!(
        "router compare[{profile_name}]: {fixture_count} Tier {tier} fixture(s); gates: \
         incomplete<=J, violations<=J, score>=J-{epsilon_pct}%*J (one relative policy); \
         runs: epic-cli {comparability_phrase} ; mode: {mode}"
    )
}

/// The empty-`--fixture`-filter bail (the empty-battery guard), pure
/// for the pin. The default A rendering is byte-identical to the
/// pre-M6-T3 literal the `#[ignore]` e2e asserts.
fn empty_filter_bail(filter: &str, tier: &str) -> String {
    format!("--fixture {filter:?} matched no Tier {tier} fixture")
}

/// The router-only+non-A-tier guard bail, pure for the pin (the
/// message is sanctioned verbatim — the M6-T3 chore kept it, the
/// quality round only pins it).
fn tier_profile_guard_bail(tier: &str) -> String {
    format!("--tier {tier} requires --profile full (router-only records exist for Tier A only)")
}

/// The battery's exit ladder — THE T17 flip face (CI drops
/// `--report-only` and this starts biting). Pure so the pin can hold
/// both directions: report mode ALWAYS exits 0 (the CI face); gate mode
/// exits 0 only when every fixture passed. (Quality-review MINOR-3: the
/// inline ladder was guttable with all 13 pins green.)
pub fn battery_exit(report_only: bool, red: usize) -> Result<()> {
    if report_only || red == 0 {
        Ok(())
    } else {
        bail!("router compare: {red} fixture(s) RED")
    }
}

/// The battery loop's detail-pass decision (the T17b controller
/// round): `--detail` (force) ALWAYS runs it — the flag is an explicit
/// opt-in that accepts the wall — otherwise the localizer runs only
/// for a red verdict on a run the harness did NOT kill. A KILLED run
/// is excluded because the in-process re-run on a wall-truncated
/// board cannot converge: it re-burns the same wall up to its own
/// deadline and localizes nothing for a slowness timeout, so honoring
/// it would let ONE fixture burn two tier walls before its verdict row
/// prints. Pure so the pin can hold the full flag × killed matrix.
fn detail_pass_should_run(force_detail: bool, failures: &[GateFailure], timed_out: bool) -> bool {
    force_detail || (!failures.is_empty() && !timed_out)
}

/// The battery loop's detail-pass WALL: the fixture's own tier timeout
/// (the same bound both faces of the fixture's run get — the subprocess
/// tier timeout doubles as the detail wall). Pure fn extracted at the
/// loop call site (quality-review T17b M-Q4) so the tier-scaling is
/// unit-observed — a global constant here would mis-scale both bm08's
/// 120s and bm01's 1800s, and nothing else in the suite would notice.
#[must_use]
fn detail_deadline(entry: &RouterFixture) -> Duration {
    Duration::from_secs(entry.timeout_seconds)
}

// ---------------------------------------------------------------------------
// Pins
// ---------------------------------------------------------------------------

#[cfg(test)]
mod pins {
    use super::*;

    /// The watchdog tests' generous wall for the worker-reported paths
    /// (worlds that finish far inside it — the early-error body is
    /// immediate). Only the deadline worlds use short walls.
    const WATCHDOG_TEST_WALL: Duration = Duration::from_secs(60);

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
        // The .abs() face (quality-review MINOR-5 / Q2 survivor): a
        // NEGATIVE Java score must not shrink ε — dropping .abs() makes
        // ε negative there, silently stricter than the policy.
        assert_eq!(epsilon_for(-500.0), 10.0);

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
        let dir = record_world("t15-preflight");

        let err = CompareProfile::RouterOnly
            .load_record(&dir.join("nope.json"))
            .expect_err("missing record must be an explicit error");
        assert!(err.contains("missing baseline record"), "{err}");

        let write = |name: &str, body: &str| write_record(&dir, name, body);
        let rust_record = write(
            "rust.json",
            r#"{"schema_version":1,"engine":"rust","fixture":"x.dsn","final_state":"COMPLETED","captured_at_unix":0}"#,
        );
        let err = CompareProfile::RouterOnly
            .load_record(&rust_record)
            .expect_err("rust record rejected");
        assert!(err.contains("engine rust"), "{err}");

        let no_profile = write(
            "noprofile.json",
            r#"{"schema_version":1,"engine":"java","fixture":"x.dsn","final_state":"COMPLETED","captured_at_unix":0}"#,
        );
        let err = CompareProfile::RouterOnly
            .load_record(&no_profile)
            .expect_err("profile-less record rejected");
        assert!(err.contains("profile"), "{err}");

        let full_flow = write(
            "fullflow.json",
            r#"{"schema_version":1,"engine":"java","fixture":"x.dsn","final_state":"COMPLETED","captured_at_unix":0,"profile":"full-flow"}"#,
        );
        let err = CompareProfile::RouterOnly
            .load_record(&full_flow)
            .expect_err("full-flow record rejected");
        assert!(err.contains("router-only"), "{err}");

        let good = write(
            "good.json",
            r#"{"schema_version":1,"engine":"java","fixture":"x.dsn","final_state":"COMPLETED","captured_at_unix":0,"profile":"router-only","incomplete_count":2,"normalized_score":986.32}"#,
        );
        let record = CompareProfile::RouterOnly
            .load_record(&good)
            .expect("good record loads");
        assert_eq!(record.incomplete_count, Some(2));
        assert_eq!(record.normalized_score, Some(986.32));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// THE full-flow record pre-flight (M4-T11): the full profile's loader
    /// accepts EXACTLY the M0 capture face (profile marker ABSENT) and
    /// rejects every mismatched record LOUDLY — a router-only-stamped
    /// record (the wrong-profile trap), a marker typo, a missing file, a
    /// rust-engine record. Contrast faces against the router-only loader:
    /// the SAME record bodies get opposite verdicts from the two loaders
    /// (the crossing cell — a loader that accepted both markers could
    /// silently mix faces).
    #[test]
    fn full_flow_record_pre_flight_accepts_only_the_markerless_m0_face() {
        let dir = record_world("t15-fullflow");

        let write = |name: &str, body: &str| write_record(&dir, name, body);
        // THE M0 capture face: no `profile` key at all. The full loader
        // accepts it and the numbers ride.
        let m0_face = write(
            "m0.json",
            r#"{"schema_version":1,"engine":"java","fixture":"x.dsn","final_state":"COMPLETED","captured_at_unix":0,"incomplete_count":0,"maximum_count":25,"normalized_score":1000.0,"optimizer_score":823.79,"optimizer_seconds":4.31}"#,
        );
        let record = CompareProfile::Full
            .load_record(&m0_face)
            .expect("the M0 face is the full record");
        assert_eq!(record.profile, None);
        assert_eq!(record.optimizer_score, Some(823.79));

        // Missing file: loud.
        let err = CompareProfile::Full
            .load_record(&dir.join("nope.json"))
            .expect_err("missing record must be an explicit error");
        assert!(
            err.contains("missing baseline record") && err.contains("full-flow"),
            "the error must name the profile face: {err}"
        );

        // A router-only-STAMPED record is NOT a full-flow baseline — the
        // wrong-profile trap must not pass silently (contrast face: the
        // router-only loader ACCEPTS this same body).
        let stamped = write(
            "stamped.json",
            r#"{"schema_version":1,"engine":"java","fixture":"x.dsn","final_state":"COMPLETED","captured_at_unix":0,"profile":"router-only"}"#,
        );
        let err = CompareProfile::Full
            .load_record(&stamped)
            .expect_err("stamped record rejected");
        assert!(
            err.contains("profile") && err.contains("full-flow"),
            "{err}"
        );
        assert!(
            CompareProfile::RouterOnly.load_record(&stamped).is_ok(),
            "contrast: the same body loads under the router-only loader"
        );

        // A marker TYPO (`full-flow` spelled into the compare face's
        // value space) is rejected — only ABSENT is the M0 face.
        let typo = write(
            "typo.json",
            r#"{"schema_version":1,"engine":"java","fixture":"x.dsn","final_state":"COMPLETED","captured_at_unix":0,"profile":"full-flow"}"#,
        );
        let err = CompareProfile::Full
            .load_record(&typo)
            .expect_err("typo'd marker rejected");
        assert!(err.contains("profile"), "{err}");

        // Wrong engine: loud, profile-independent — a rust-engine record
        // (markerless, the full face's marker) must be rejected by the
        // full loader naming the engine, exactly as the router-only
        // loader rejects it (the shared core's engine check is
        // profile-INdependent; a mutant that made it
        // router-only-conditional dies HERE).
        let rust_body = write(
            "rust_full.json",
            r#"{"schema_version":1,"engine":"rust","fixture":"x.dsn","final_state":"COMPLETED","captured_at_unix":0}"#,
        );
        let err = CompareProfile::Full
            .load_record(&rust_body)
            .expect_err("rust record rejected");
        assert!(err.contains("engine rust"), "{err}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// THE profile-switch pin (M4-T11): the compare profile parses the
    /// dispatch's two names exactly — omitted and `router-only` mean the
    /// M2/M3 regression face, `full` means the M4 battery face — and ANY
    /// other value is a LOUD failure naming the accepted values (never a
    /// silent fallback to the default: a typo'd profile must not silently
    /// re-face the battery). The oracle/capture spelling `full-flow` is
    /// deliberately NOT a compare name (the error names the two values so
    /// the mismatch is self-explaining).
    #[test]
    fn compare_profile_parsing_is_exact_and_loud_on_unknown_names() {
        assert_eq!(
            CompareProfile::from_name(None).expect("omitted"),
            CompareProfile::RouterOnly,
            "the omitted profile is the router-only default (the pre-T11 face)"
        );
        assert_eq!(
            CompareProfile::from_name(Some("router-only")).expect("router-only"),
            CompareProfile::RouterOnly
        );
        assert_eq!(
            CompareProfile::from_name(Some("full")).expect("full"),
            CompareProfile::Full
        );
        // Every wrong spelling is LOUD, naming the accepted values.
        for wrong in ["full-flow", "routeronly", "FULL", "", "Full", "flow"] {
            let err = CompareProfile::from_name(Some(wrong))
                .expect_err("a wrong profile name must fail loudly");
            let msg = format!("{err:#}");
            assert!(
                msg.contains("unknown compare profile"),
                "unexpected error for {wrong:?}: {msg}"
            );
            assert!(
                msg.contains("router-only") && msg.contains("full"),
                "the error must name BOTH accepted values for {wrong:?}: {msg}"
            );
            // NIT-2: the `full-flow` case is the one the clause exists
            // for — the user learned that spelling from the capture/
            // verify surface, and the error must map it in place.
            if wrong == "full-flow" {
                assert!(
                    msg.contains("capture/verify surface spells the full profile"),
                    "the error must map the capture-surface spelling: {msg}"
                );
            }
        }
    }

    /// THE fixture-enum completeness canary (real repo tree, java-free,
    /// cheap): Tier A in tiers.yaml order, every record present, and the
    /// committed record set has NO extras — for BOTH profiles. A dropped
    /// fixture (the walk or the capture) is the finding-class trap 2
    /// names — this pin dies on that mutant. The full arm keys the M0
    /// `baselines/java` record set (the reuse decision's structural
    /// witness: the battery's jar side exists for every Tier A fixture).
    #[test]
    fn tier_a_fixture_enum_is_complete_against_tiers_and_records() {
        let root = crate::oracle::find_repo_root().expect("repo root");
        for (profile, _tier_dirs) in [
            (
                CompareProfile::RouterOnly,
                vec!["DAC2020_boards", "KiCad_10_demos"],
            ),
            (
                CompareProfile::Full,
                vec!["DAC2020_boards", "KiCad_10_demos"],
            ),
        ] {
            let fixtures = tier_a_fixtures(&root, profile).expect("tier A walk");
            assert_eq!(fixtures.len(), 11, "the committed Tier A battery");
            assert_eq!(
                fixtures[0].tier_path, "DAC2020_boards/DAC2020_bm01.dsn",
                "tiers.yaml order ({profile:?})"
            );
            assert_eq!(
                fixtures.last().expect("non-empty").tier_path,
                "KiCad_10_demos/sonde xilinx.dsn",
                "the space-bearing path survives the walk verbatim ({profile:?})"
            );
            for fixture in &fixtures {
                assert!(
                    fixture.baseline_path.is_file(),
                    "record missing for {} under {profile:?}: {}",
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
            // No extras (the shared sweep): every committed record
            // maps back to a walked fixture.
            let walked: std::collections::BTreeSet<String> = fixtures
                .iter()
                .map(|f| format!("{}.baseline.json", f.tier_path))
                .collect();
            let extras = record_extras(&profile_tier_baselines_dir(&root, profile, "A"), &walked);
            assert!(
                extras.is_empty(),
                "records without a Tier A fixture ({profile:?}): {extras:?}"
            );
        }
    }

    /// THE tier-filter walk pin (M6-T3 chore, extended at the quality
    /// round): `tier_fixtures` walks tiers B and C in tiers.yaml order
    /// against the REAL repo tree, every full-flow record present, the
    /// no-extras record sweep per tier (the trap-2 complement — every
    /// committed record under `baselines/java/{B,C}` maps back to a
    /// walked fixture), the router-only+B battery is a LOUD bail (the
    /// compare_cmd profile guard), and a typo'd tier gets the
    /// tiers.yaml diagnostic. The default A WALK face is held by
    /// `tier_a_fixture_enum_is_complete_against_tiers_and_records`;
    /// the banner/bail STRINGS are held by
    /// `battery_banner_and_bail_literals_are_pinned`.
    #[test]
    fn tier_fixture_walk_covers_b_and_c_against_full_records() {
        let root = crate::oracle::find_repo_root().expect("repo root");
        let expected: &[(&str, usize, &str, &str)] = &[
            (
                "B",
                9,
                "DAC2020_boards/DAC2020_bm05.dsn",
                "PCBench/1Bitsy_1bitsy/unrouted.dsn",
            ),
            (
                "C",
                3,
                "DAC2020_boards/DAC2020_bm04.dsn",
                "PCBench/front-end-modules_LimeSDR_Sony/unrouted.dsn",
            ),
        ];
        for (tier_name, count, first, last) in expected {
            let fixtures =
                tier_fixtures(&root, CompareProfile::Full, tier_name).expect("tier walk");
            assert_eq!(
                fixtures.len(),
                *count,
                "the committed tier {tier_name} battery"
            );
            assert_eq!(&fixtures[0].tier_path, first, "tiers.yaml order");
            assert_eq!(
                &fixtures.last().expect("non-empty").tier_path,
                last,
                "tiers.yaml order"
            );
            for fixture in &fixtures {
                assert!(
                    fixture.baseline_path.is_file(),
                    "full-flow record missing for tier {tier_name}: {}",
                    fixture.baseline_path.display()
                );
                assert!(
                    root.join(&fixture.repo_rel_dsn).is_file(),
                    "fixture missing on disk: {}",
                    fixture.repo_rel_dsn
                );
                assert!(fixture.timeout_seconds > 0);
            }
            // The no-extras sweep (the shared helper): every committed
            // full-flow record under `baselines/java/{tier}` maps back
            // to a walked fixture.
            let walked: std::collections::BTreeSet<String> = fixtures
                .iter()
                .map(|f| format!("{}.baseline.json", f.tier_path))
                .collect();
            let extras = record_extras(
                &profile_tier_baselines_dir(&root, CompareProfile::Full, tier_name),
                &walked,
            );
            assert!(
                extras.is_empty(),
                "records without a tier {tier_name} fixture: {extras:?}"
            );
        }
        // The B/C router-only face is the loud bail — AFTER tier
        // validation (the guard can only see valid tiers) and BEFORE
        // bin resolution (the pin never touches a bin).
        let err = compare_cmd(
            &root,
            None,
            CompareProfile::RouterOnly,
            "B",
            true,
            false,
            None,
            &[],
        )
        .expect_err("router-only + tier B must bail");
        assert!(
            err.to_string().contains("requires --profile full"),
            "the bail must name the profile requirement: {err}"
        );
        // A typo'd tier bails at the walk with the tiers.yaml
        // diagnostic, under BOTH profiles (quality MINOR-4). The
        // router-only arm DISCRIMINATES the walk-before-guard order —
        // the pre-round order (guard first) returned the guard's
        // "requires --profile full" here, failing this assert; the
        // walk-before-BIN-resolution order is witnessed by code read
        // (the walk is compare_cmd's first fallible statement).
        for (profile_face, profile_desc) in [
            (CompareProfile::Full, "full"),
            (CompareProfile::RouterOnly, "router-only"),
        ] {
            let err = compare_cmd(&root, None, profile_face, "D", true, false, None, &[])
                .expect_err("a typo'd tier must bail");
            assert!(
                err.to_string().contains("tier D missing from"),
                "the typo'd tier must get the tiers.yaml diagnostic under {profile_desc}: {err}"
            );
        }
    }

    /// THE banner/bail literal pin (quality-round MINOR-2): the three
    /// pure helpers render EXACTLY the pre-M6-T3 default-A literals
    /// plus the B/C interpolation. Kills any template or guard-message
    /// drift: a changed banner template, bail wording, epsilon face,
    /// or interpolation breaks the assert_eq!s below.
    #[test]
    fn battery_banner_and_bail_literals_are_pinned() {
        let phrase = "<comparability>";
        // The default A banner face, byte-identical to the pre-M6-T3
        // literal (the same line the e2e smokes substring-match).
        assert_eq!(
            battery_banner(
                "router-only",
                11,
                "A",
                SCORE_RELATIVE_EPSILON * 100.0,
                phrase,
                "report (exit 0)",
            ),
            format!(
                "router compare[router-only]: 11 Tier A fixture(s); gates: incomplete<=J, \
                 violations<=J, score>=J-2%*J (one relative policy); runs: epic-cli {phrase} ; \
                 mode: report (exit 0)"
            ),
        );
        // The B/C interpolation (the M6-T3 B/C batteries' faces).
        assert_eq!(
            battery_banner(
                "full",
                9,
                "B",
                SCORE_RELATIVE_EPSILON * 100.0,
                phrase,
                "gate",
            ),
            format!(
                "router compare[full]: 9 Tier B fixture(s); gates: incomplete<=J, violations<=J, \
                 score>=J-2%*J (one relative policy); runs: epic-cli {phrase} ; mode: gate"
            ),
        );
        assert_eq!(
            battery_banner(
                "full",
                3,
                "C",
                SCORE_RELATIVE_EPSILON * 100.0,
                phrase,
                "gate",
            ),
            format!(
                "router compare[full]: 3 Tier C fixture(s); gates: incomplete<=J, violations<=J, \
                 score>=J-2%*J (one relative policy); runs: epic-cli {phrase} ; mode: gate"
            ),
        );
        // The guard bail, sanctioned verbatim.
        assert_eq!(
            tier_profile_guard_bail("B"),
            "--tier B requires --profile full (router-only records exist for Tier A only)"
        );
        assert_eq!(
            tier_profile_guard_bail("C"),
            "--tier C requires --profile full (router-only records exist for Tier A only)"
        );
        // The empty-filter bail: the default A rendering byte-identical
        // to the literal the #[ignore] e2e asserts, plus the C face.
        assert_eq!(
            empty_filter_bail("zzz", "A"),
            "--fixture \"zzz\" matched no Tier A fixture"
        );
        assert_eq!(
            empty_filter_bail("foo", "C"),
            "--fixture \"foo\" matched no Tier C fixture"
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

        // M5-T7: the threads-invariance gate's verdict — all-equal
        // passes; a planted divergence on the ODD-N face and on the
        // EVEN-N face each fail naming the face. Kills the
        // run3-blind mutant (a verdict comparing only -mt 1 vs -mt 3).
        let ok = threads_invariance_verdict(
            [
                (Some("aaaa".into()), Some("m1".into())),
                (Some("aaaa".into()), Some("m1".into())),
                (Some("aaaa".into()), Some("m1".into())),
            ],
            THREADS_GATE_FACES,
        )
        .expect("all-equal digests are OK");
        assert!(ok.contains("threads invariance: OK"), "{ok}");

        let err = threads_invariance_verdict(
            [
                (Some("aaaa".into()), Some("m1".into())),
                (Some("bbbb".into()), Some("m1".into())),
                (Some("aaaa".into()), Some("m1".into())),
            ],
            THREADS_GATE_FACES,
        )
        .expect_err("a planted ODD-N SES divergence must fail");
        assert!(err.contains("-mt 3"), "{err}");

        let err = threads_invariance_verdict(
            [
                (Some("aaaa".into()), Some("m1".into())),
                (Some("aaaa".into()), Some("m1".into())),
                (Some("aaaa".into()), Some("m2".into())),
            ],
            THREADS_GATE_FACES,
        )
        .expect_err("a planted EVEN-N manifest divergence must fail");
        assert!(err.contains("-mt 4"), "{err}");

        let err = threads_invariance_verdict(
            [
                (Some("aaaa".into()), Some("m1".into())),
                (Some("aaaa".into()), Some("m1".into())),
                (None, Some("m1".into())),
            ],
            THREADS_GATE_FACES,
        )
        .expect_err("a missing EVEN-N run must fail naming the face");
        assert!(err.contains("-mt 4"), "{err}");

        let err = determinism_verdict(
            Some("aaaa".into()),
            Some("aaaa".into()),
            None,
            Some("m1".into()),
        )
        .expect_err("a missing manifest must fail independently of the ses face");
        assert!(err.contains("no manifest"), "{err}");
    }

    /// M5-T7 quality Q1: the verdict names the ACTUAL faces it is
    /// handed — an override run (`--odd-threads 5 --even-threads 6`)
    /// gets a banner naming 1/5/6 and divergence messages naming the
    /// override face, never the defaults (the gate's diagnostics must
    /// not lie — the `route_argv` banner standard).
    #[test]
    fn threads_gate_verdict_names_override_faces() {
        let overrides = [1usize, 5, 6];
        let ok = threads_invariance_verdict(
            [
                (Some("aaaa".into()), Some("m1".into())),
                (Some("aaaa".into()), Some("m1".into())),
                (Some("aaaa".into()), Some("m1".into())),
            ],
            overrides,
        )
        .expect("all-equal digests are OK");
        assert!(
            ok.contains("-mt 1/5/6"),
            "banner must name the overrides: {ok}"
        );

        let err = threads_invariance_verdict(
            [
                (Some("aaaa".into()), Some("m1".into())),
                (Some("bbbb".into()), Some("m1".into())),
                (Some("aaaa".into()), Some("m1".into())),
            ],
            overrides,
        )
        .expect_err("a divergence at the override odd face names it");
        assert!(err.contains("-mt 5"), "{err}");
        assert!(
            !err.contains("-mt 3"),
            "the default odd face must not appear: {err}"
        );

        let err = threads_invariance_verdict(
            [
                (Some("aaaa".into()), Some("m1".into())),
                (Some("aaaa".into()), Some("m1".into())),
                (Some("aaaa".into()), Some("m2".into())),
            ],
            overrides,
        )
        .expect_err("a divergence at the override even face names it");
        assert!(err.contains("-mt 6"), "{err}");
        assert!(
            !err.contains("-mt 4"),
            "the default even face must not appear: {err}"
        );
    }

    /// M5-T7: the threads gate's argv wiring — `-mt <n>` rides the
    /// SAME single-source argv builder, appended AFTER the profile's
    /// comparability flags; the plain face carries no `-mt`. Kills the
    /// drop-the-flag mutant.
    #[test]
    fn threads_gate_argv_appends_mt_flag() {
        let cli = Path::new("/tmp/epic-cli");
        let dsn = Path::new("/tmp/board.dsn");
        let ses = Path::new("/tmp/out.ses");
        let manifest = Path::new("/tmp/manifest.json");
        let plain = route_argv(cli, dsn, ses, manifest, CompareProfile::RouterOnly);
        assert!(
            !plain.iter().any(|arg| arg == "-mt"),
            "the plain face must not carry -mt"
        );
        let threaded = route_argv_threads(
            cli,
            dsn,
            ses,
            manifest,
            CompareProfile::RouterOnly,
            Some(3),
            &[],
        );
        let mt_position = threaded
            .iter()
            .position(|arg| arg == "-mt")
            .expect("-mt must be present");
        assert_eq!(
            threaded.get(mt_position + 1).map(String::as_str),
            Some("3"),
            "-mt takes the thread count as its value"
        );
        assert!(
            threaded.starts_with(&plain[..]),
            "-mt is appended after the profile's comparability flags"
        );
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

    /// THE comparability argv pin (spec-review MINOR-1 / OW-M2 survivor),
    /// M4-T11 profile face: `route_argv` — the ONE builder `run_cli`
    /// spawns from — carries the PROFILE's flag suffix, and the banner
    /// phrase derived from that argv claims the profile's face. The 2×2
    /// crossing cell is pinned: RouterOnly ⇒ BOTH flags EXACTLY once as
    /// the suffix (fanout second-to-last, optimizer last — the M4-T9
    /// face); Full ⇒ NEITHER flag on the same base contract (the same
    /// first eight args, so the full face is the bare default-settings
    /// pipeline). Banner mismatch faces BOTH directions: a router-only
    /// argv with a dropped flag says MISSING; a full argv with a planted
    /// disable flag says PRESENT (the flag-gating bug face). Mutation
    /// faces: dropping a flag from the router-only builder fails the
    /// position/last asserts AND flips the banner to MISSING; the
    /// full-arm flag-injection mutant fails the no-flag asserts AND
    /// flips the banner to PRESENT.
    #[test]
    fn run_cli_argv_carries_the_comparability_flag() {
        let base = |profile| {
            route_argv(
                Path::new("/bin/epic-cli"),
                Path::new("/f/x.dsn"),
                Path::new("/w/out.ses"),
                Path::new("/w/manifest.json"),
                profile,
            )
        };
        // ARM 1 — RouterOnly: BOTH flags, exactly once, as the suffix.
        let argv = base(CompareProfile::RouterOnly);
        let flag_positions: Vec<usize> = argv
            .iter()
            .enumerate()
            .filter_map(|(i, arg)| (*arg == COMPARABILITY_FLAG).then_some(i))
            .collect();
        assert_eq!(
            flag_positions,
            vec![argv.len() - 2],
            "the fanout flag rides once, second-to-last (the optimizer flag is the suffix): \
             {argv:?}"
        );
        assert_eq!(
            argv.last().map(String::as_str),
            Some(OPTIMIZER_OFF_FLAG),
            "the M4-T9 optimizer-off flag is the argv suffix: {argv:?}"
        );
        // The banner face is DERIVED: a correct argv yields the claim.
        let phrase = profile_banner_phrase(CompareProfile::RouterOnly, &argv);
        assert!(
            phrase.contains(COMPARABILITY_FLAG) && phrase.contains(OPTIMIZER_OFF_FLAG),
            "the banner must claim both comparability flags from the actual argv: {phrase}"
        );
        // Degraded face: an argv WITHOUT the fanout flag (the OW-M2
        // mutant) must make the banner say MISSING — the log can never
        // lie about the comparability face. The optimizer-flag drop is
        // caught by the SAME conjunction (either missing -> MISSING) —
        // pinned by its OWN planted arm below (NIT-1: the conjunction's
        // second column must not ride on the comment).
        let mutant_argv: Vec<String> = argv
            .iter()
            .filter(|arg| *arg != COMPARABILITY_FLAG)
            .cloned()
            .collect();
        let phrase = profile_banner_phrase(CompareProfile::RouterOnly, &mutant_argv);
        assert!(
            phrase.contains("COMPARABILITY FLAG MISSING"),
            "a flag-less argv must stop the banner claiming fanout-off: {phrase}"
        );
        // The second column: dropping ONLY the optimizer-off flag (the
        // M4-T9 flag) from an otherwise-correct router-only argv must
        // produce the SAME MISSING face — a mutant that keyed the banner
        // on the fanout flag alone survives nothing.
        let optimizer_only_drop: Vec<String> = argv
            .iter()
            .filter(|arg| *arg != OPTIMIZER_OFF_FLAG)
            .cloned()
            .collect();
        assert!(
            optimizer_only_drop
                .iter()
                .any(|arg| arg == COMPARABILITY_FLAG),
            "premise: the fanout flag is still present in this planted argv"
        );
        let phrase = profile_banner_phrase(CompareProfile::RouterOnly, &optimizer_only_drop);
        assert!(
            phrase.contains("COMPARABILITY FLAG MISSING"),
            "an optimizer-flag-only drop must ALSO say MISSING: {phrase}"
        );

        // ARM 2 — Full: NEITHER flag, and the SAME base contract as the
        // router-only argv minus the flags (the full face is the bare
        // default-settings pipeline; a flags-from-full-fanout mutant that
        // changed the base would fail the prefix equality).
        let full_argv = base(CompareProfile::Full);
        assert!(
            !full_argv.iter().any(|arg| arg == COMPARABILITY_FLAG),
            "the full profile must NOT carry the fanout-off flag: {full_argv:?}"
        );
        assert!(
            !full_argv.iter().any(|arg| arg == OPTIMIZER_OFF_FLAG),
            "the full profile must NOT carry the optimizer-off flag: {full_argv:?}"
        );
        let base_len = argv.len() - 2; // the two flag suffix args
        assert_eq!(
            &full_argv[..base_len],
            &argv[..base_len],
            "full and router-only share the base contract; ONLY the flag suffix differs: \
             {argv:?} vs {full_argv:?}"
        );
        assert_eq!(full_argv.len(), base_len, "no flags on the full argv");
        // The banner claims the full face — and a planted disable flag in
        // a full argv is the flag-gating bug face (the PRESENT arm).
        let phrase = profile_banner_phrase(CompareProfile::Full, &full_argv);
        assert!(
            phrase.contains("no comparability flags"),
            "the banner must claim the full face from the actual argv: {phrase}"
        );
        let planted = base(CompareProfile::Full);
        let planted = {
            let mut v = planted;
            v.push(OPTIMIZER_OFF_FLAG.to_string());
            v
        };
        let phrase = profile_banner_phrase(CompareProfile::Full, &planted);
        assert!(
            phrase.contains("COMPARABILITY FLAG PRESENT IN A FULL-PROFILE ARGV"),
            "a disable flag in a full argv must stop the banner claiming the full face: {phrase}"
        );

        // The detail-pass argv faces ([`CompareProfile::detail_argv_flags`]):
        // RouterOnly = the historical T15 face (fanout flag ONLY — the
        // pinned buglog-176 fingerprint face), Full = NO flags.
        assert_eq!(
            CompareProfile::RouterOnly.detail_argv_flags(),
            &[COMPARABILITY_FLAG],
            "the router-only detail face is fanout-flag-only"
        );
        assert_eq!(
            CompareProfile::Full.detail_argv_flags(),
            &[] as &[&str],
            "the full detail face carries no flags"
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
        let dir = record_world("t15-digest-pair");
        std::fs::create_dir_all(dir.join("run1")).expect("run1 dir");
        std::fs::create_dir_all(dir.join("run2")).expect("run2 dir");
        // Planted-different bytes: run1 and run2 wrote DIFFERENT output.
        std::fs::write(dir.join("run1/out.ses"), b"session one").expect("ses1");
        std::fs::write(
            dir.join("run1/manifest.json"),
            b"{\"app_version\":\"0\",\"git_sha\":\"x\",\"m\":1}",
        )
        .expect("man1");
        std::fs::write(dir.join("run2/out.ses"), b"session two").expect("ses2");
        std::fs::write(
            dir.join("run2/manifest.json"),
            b"{\"app_version\":\"0\",\"git_sha\":\"x\",\"m\":2}",
        )
        .expect("man2");
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
        std::fs::write(
            dir.join("run2/manifest.json"),
            b"{\"app_version\":\"0\",\"git_sha\":\"x\",\"m\":1}",
        )
        .expect("man2 equal");
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

    /// THE pair-construction pin (spec-rereview MINOR-A / NEW-B survivor):
    /// `pair_from_runs` derives run2's artifacts from `run2_dir` ONLY —
    /// witnessed structurally (the exact paths) AND behaviorally (planted-
    /// different bytes in the two dirs MUST digest differently). Mutation-
    /// verified: the reviewer's NEW-B (run2's joins taken from run1_dir)
    /// fails BOTH the path asserts and the digest face.
    #[test]
    fn pair_from_runs_derives_run2_from_run2_dir_only() {
        let dir = record_world("t15-pair-runs");
        std::fs::create_dir_all(dir.join("run1")).expect("run1 dir");
        std::fs::create_dir_all(dir.join("run2")).expect("run2 dir");
        // Different bytes per run: a pair that aliases run2 onto run1
        // cannot produce different digests.
        std::fs::write(dir.join("run1").join(RUN_SES_FILE), b"session one").expect("ses1");
        std::fs::write(
            dir.join("run1").join(RUN_MANIFEST_FILE),
            b"{\"app_version\":\"0\",\"git_sha\":\"x\",\"m\":1}",
        )
        .expect("man1");
        std::fs::write(dir.join("run2").join(RUN_SES_FILE), b"session two").expect("ses2");
        std::fs::write(
            dir.join("run2").join(RUN_MANIFEST_FILE),
            b"{\"app_version\":\"0\",\"git_sha\":\"x\",\"m\":2}",
        )
        .expect("man2");

        let pair = pair_from_runs(&dir.join("run1"), &dir.join("run2"));
        // Structural faces: each run's artifacts live in ITS OWN dir,
        // under the same canonical names the runner writes.
        assert_eq!(
            pair.run1.ses,
            dir.join("run1").join(RUN_SES_FILE),
            "{pair:?}"
        );
        assert_eq!(
            pair.run1.manifest,
            dir.join("run1").join(RUN_MANIFEST_FILE),
            "{pair:?}"
        );
        assert_eq!(
            pair.run2.ses,
            dir.join("run2").join(RUN_SES_FILE),
            "run2's SES must derive from run2_dir: {pair:?}"
        );
        assert_eq!(
            pair.run2.manifest,
            dir.join("run2").join(RUN_MANIFEST_FILE),
            "run2's manifest must derive from run2_dir: {pair:?}"
        );
        assert_ne!(pair.run1.ses, pair.run2.ses);
        assert_ne!(pair.run1.manifest, pair.run2.manifest);
        // Behavioral face: planted-different bytes digest differently —
        // the NEW-B aliasing fails right here too.
        let (ses_1, ses_2, man_1, man_2) = determinism_digests(&pair).expect("digests");
        assert_ne!(
            ses_1, ses_2,
            "run2 built from run2_dir must read run2's bytes"
        );
        assert_ne!(
            man_1, man_2,
            "run2 built from run2_dir must read run2's manifest"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// THE hook-restore pin (spec-rereview MINOR-B / NEW-C survivor): the
    /// previous panic hook is restored on EVERY path out of
    /// [`run_detail_pass`] — the worker-reported path (an early error
    /// sent through the watchdog channel) AND the T17b deadline path (a
    /// real Tier A fixture whose in-process route overruns the wall —
    /// bm01's detail pass alone is >50 min in the debug build). Sentinel
    /// mechanics: install a recording hook as the "previous" hook, run
    /// the pass, then panic deliberately — the DELIBERATE panic must
    /// reach the SENTINEL (the restored hook), not a lingering no-op.
    /// Mutation-verified: deleting the `set_hook(previous_hook)` restore
    /// (the reviewer's NEW-C) makes the sentinel stay silent and this
    /// pin fail — on BOTH worlds, so the deadline-path restore is
    /// covered by the same kill.
    ///
    /// FULL-SUITE GATE (T17b; buglog 175): the watchdog bounds this
    /// sentinel (world 2 fires a 10s deadline — bm01's route is orders
    /// of magnitude past it), so the bare suite runs again:
    /// `cargo test --workspace` — the `--skip detail_pass_restores`
    /// protocol AND the workflow step are RETIRED. The leaked world-2
    /// worker keeps routing until the test binary exits (documented in
    /// [`run_detail_pass`]) — it owns its board and cannot affect any
    /// later test's state.
    #[test]
    fn detail_pass_restores_the_previous_panic_hook_on_every_path() {
        let _hook_guard = crate::panic_hook::lock();
        let sentinel_fires: std::sync::Arc<std::sync::Mutex<Vec<String>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));

        // Self-cleaning: whatever hook the test harness installed before
        // this pin goes back when the pin ends.
        let before = std::panic::take_hook();
        let fires = std::sync::Arc::clone(&sentinel_fires);
        std::panic::set_hook(Box::new(move |info| {
            fires.lock().expect("sentinel mutex").push(
                info.payload()
                    .downcast_ref::<String>()
                    .cloned()
                    .unwrap_or_else(|| {
                        info.payload()
                            .downcast_ref::<&str>()
                            .map(|s| (*s).to_string())
                            .unwrap_or_default()
                    }),
            );
        }));

        // World 1 — the worker-reported early-error path: a DSN that
        // cannot be read makes route_detail_inner bail inside the worker;
        // the result travels the channel and the restore must still run
        // (a future early return between the hook swap and the restore is
        // exactly the leak this world catches).
        let err = run_detail_pass(
            Path::new("/nonexistent/epic-t15-world.dsn"),
            WATCHDOG_TEST_WALL,
            CompareProfile::RouterOnly,
        )
        .expect_err("unreadable DSN must be an Err");
        assert!(err.contains("reading"), "the read-context error: {err}");
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            panic!("T15-SENTINEL-after-early-error");
        }));
        assert!(
            sentinel_fires
                .lock()
                .expect("sentinel mutex")
                .iter()
                .any(|m| m.contains("T15-SENTINEL-after-early-error")),
            "the previous hook must be restored on the early-error path"
        );

        // World 2 — the DEADLINE path: bm01's in-process detail route
        // overruns the 10s watchdog wall (pass 1 alone measured 1074s in
        // the debug build, buglog 175) — the verdict must be the
        // deadline class AND the hook must come back before it is
        // returned.
        let root = crate::oracle::find_repo_root().expect("repo root");
        let bm01 = tier_a_fixtures(&root, CompareProfile::RouterOnly)
            .expect("tier A walk")
            .into_iter()
            .find(|f| f.tier_path == "DAC2020_boards/DAC2020_bm01.dsn")
            .expect("bm01 is a Tier A fixture");
        let verdict = run_detail_pass(
            &root.join(&bm01.repo_rel_dsn),
            Duration::from_secs(10),
            CompareProfile::RouterOnly,
        )
        .expect_err("bm01's detail route cannot finish inside 10s (pass 1 alone >1000s)");
        assert!(
            verdict.contains("deadline exceeded"),
            "the deadline verdict class: {verdict}"
        );
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            panic!("T15-SENTINEL-after-deadline-path");
        }));
        assert!(
            sentinel_fires
                .lock()
                .expect("sentinel mutex")
                .iter()
                .any(|m| m.contains("T15-SENTINEL-after-deadline-path")),
            "the previous hook must be restored on the deadline path too"
        );

        // Restore the harness's own hook (run_detail_pass put the sentinel
        // back; take it and reinstall the original).
        let _ = std::panic::take_hook();
        std::panic::set_hook(before);
    }

    /// T17b deadline-FIRES pin: an in-process route that overruns its
    /// wall returns the DEADLINE verdict class — distinguishable from
    /// the panic class (`detail pass panicked: …`) and from any quality
    /// face (an `Ok` detail). bm01's detail route is orders of
    /// magnitude past a 2s wall (pass 1 alone measured 1074s in the
    /// debug build, buglog 175), so the deadline fires mid-route, not
    /// before the worker starts. Mutants: relabeling the deadline Err
    /// with the `panicked:` phrasing dies at the second assert; a
    /// watchdog that waits forever instead of
    /// [`std::sync::mpsc::RecvTimeoutError::Timeout`] turns this pin
    /// into a hang the suite cannot miss.
    #[test]
    fn detail_pass_deadline_fires_distinguishably() {
        let _hook_guard = crate::panic_hook::lock();
        let root = crate::oracle::find_repo_root().expect("repo root");
        let bm01 = tier_a_fixtures(&root, CompareProfile::RouterOnly)
            .expect("tier A walk")
            .into_iter()
            .find(|f| f.tier_path == "DAC2020_boards/DAC2020_bm01.dsn")
            .expect("bm01 is a Tier A fixture");
        let verdict = run_detail_pass(
            &root.join(&bm01.repo_rel_dsn),
            Duration::from_secs(2),
            CompareProfile::RouterOnly,
        )
        .expect_err("bm01's detail route cannot finish inside 2s");
        assert!(
            verdict.contains("deadline exceeded"),
            "the deadline verdict class: {verdict}"
        );
        assert!(
            !verdict.contains("panicked"),
            "deadline-red is NOT panic-red (buglog 175's distinguishable-verdict requirement): {verdict}"
        );
    }

    /// Quality-review T17b M-Q4 — the loop's deadline WIRING pin
    /// (closes the banked T17B-Q1 mutant): the three watchdog pins
    /// call `run_detail_pass` with their own walls, so a mutant that
    /// replaced the LOOP's call-site value with a constant (Q1 used
    /// 1s) left every gate green. [`detail_deadline`] is the extracted
    /// pure wiring: it must equal the fixture's tiers.yaml timeout AND
    /// scale across fixtures (bm08's 120s wall vs bm01's 1800s).
    #[test]
    fn detail_deadline_equals_the_tier_wall_and_scales_across_fixtures() {
        let root = crate::oracle::find_repo_root().expect("repo root");
        let fixtures = tier_a_fixtures(&root, CompareProfile::RouterOnly).expect("tier A walk");
        let find = |tier_path: &str| {
            fixtures
                .iter()
                .find(|f| f.tier_path == tier_path)
                .unwrap_or_else(|| panic!("{tier_path} is a Tier A fixture"))
        };
        let bm08 = find("DAC2020_boards/DAC2020_bm08.dsn");
        let bm01 = find("DAC2020_boards/DAC2020_bm01.dsn");
        // The literal walls are tiers.yaml's at pin time — if the tier
        // changes, this pin forces the change to be acknowledged here.
        assert_eq!(bm08.timeout_seconds, 120, "bm08's tier wall");
        assert_eq!(bm01.timeout_seconds, 1800, "bm01's tier wall");
        // The wiring: the deadline IS the tier wall, per fixture.
        assert_eq!(detail_deadline(bm08), Duration::from_secs(120));
        assert_eq!(detail_deadline(bm01), Duration::from_secs(1800));
        // The scaling face: one fn, two walls an order of magnitude
        // apart (the constant-deadline mutant dies here and at the
        // equalities above).
        assert_ne!(bm08.timeout_seconds, bm01.timeout_seconds);
        assert_ne!(detail_deadline(bm08), detail_deadline(bm01));
    }

    /// T17b deadline-SILENT pin: a completing fixture under a generous
    /// wall returns the body's OWN outcome — the watchdog machinery
    /// (worker thread + channel round-trip) does not perturb the
    /// deterministic result. ecc83-pp_v2 is the buglog-176 fingerprint
    /// fixture (the T17b-measured POST-fix battery row: incomplete 0 —
    /// the isTraceObstacle parity fix let the maze route net 6 — 16
    /// violation pairs at the time, wall ~0.2s release; the PRE-fix
    /// row was incomplete 1 / ~6.4s) — the unwatched subprocess run
    /// and the watched in-process run must agree on all of them (the
    /// subprocess row equaled the committed JAVA record exactly then:
    /// 991.32 / 0 unrouted / 16 violations). P4 rotation (upstream
    /// 14b28b6ff, #925b): the same-component Pin-Pin exemptions drop
    /// exactly this board's two same-component pin-pin pairs from the
    /// walk — the fingerprint violations count is 14 now (the same two
    /// rows the frozen drc-0009 golden pins; the tierA gate still
    /// holds by its ≤-law, fewer violations only help). M4-T4
    /// geometry rotation: with
    /// the 45° tightener live the detail pass straightens its own
    /// output — 15 traces / 0 vias / 66 bends became 14 traces / 0
    /// vias / 19 bends. M4-T6 rotation: with the engine's
    /// `DrillEngine::shove_trace_check` wired to the production
    /// `epic_board::trace_shover::check_max_length` (bug-187 — the
    /// 0.0 stub had short-circuited every maze shove to ripup-only),
    /// the detail pass's shove decisions changed again: 14/0/19 became
    /// 25/0/22, re-measured deterministic across runs. Score and
    /// violations UNCHANGED through both rotations: the Java record
    /// 991.32 / 0 / 16 still matches (the counts are the buglog-176
    /// DETERMINISM FINGERPRINT, not a jar anchor). Mutants: a
    /// machinery that fabricates or swallows the worker result dies
    /// here; a wall that fires early turns this `Ok` into the deadline
    /// `Err` and dies here; a regression of the buglog-176 fix (the CA
    /// flag leaving the virtual dispatch again) resurrects incomplete
    /// 1 and dies here (witnessed RED at the fix, 2026-09-21).
    #[test]
    fn detail_pass_deadline_silent_preserves_the_deterministic_outcome() {
        let _hook_guard = crate::panic_hook::lock();
        let root = crate::oracle::find_repo_root().expect("repo root");
        let fixture = tier_a_fixtures(&root, CompareProfile::RouterOnly)
            .expect("tier A walk")
            .into_iter()
            .find(|f| f.tier_path == "KiCad_10_demos/ecc83-pp_v2.dsn")
            .expect("ecc83-pp_v2 is a Tier A fixture");
        let detail = run_detail_pass(
            &root.join(&fixture.repo_rel_dsn),
            Duration::from_secs(600),
            CompareProfile::RouterOnly,
        )
        .expect("the generous wall must stay silent on a ~6s route");
        assert_eq!(
            detail.incomplete_total, 0,
            "buglog-176 fingerprint: net 6 ROUTES (the isTraceObstacle parity fix)"
        );
        assert_eq!(
            detail.violations.len(),
            14,
            "buglog-176 fingerprint: 14 violation pairs (P4, upstream 14b28b6ff \
             #925b — the two same-component pin-pin pairs this board carries \
             are exempt now, the same two rows the frozen drc-0009 golden \
             holds; was 16 pre-P4)"
        );
        assert_eq!(
            (detail.trace_count, detail.via_count, detail.bend_count),
            (25, 0, 22),
            "buglog-176 geometry counts post M4-T6: 25 traces / 0 vias / \
             22 bends (the wired production shover; was 14/0/19 under the \
             T4 tightener alone, 15/0/66 pre-T4)"
        );
    }

    /// THE exit-ladder pin (quality-review MINOR-3 / Q3 survivor): report
    /// mode exits 0 REGARDLESS of red; gate mode exits 0 only at red == 0
    /// and names the count otherwise. This is THE T17 flip face — a
    /// gutted ladder silently kept a red battery green. Mutation-verified:
    /// gutting [`battery_exit`] to `Ok(())` fails the gate+red direction.
    #[test]
    fn battery_exit_pins_the_gate_report_ladder() {
        assert!(battery_exit(true, 0).is_ok());
        assert!(
            battery_exit(true, 7).is_ok(),
            "report mode ALWAYS exits 0 — that is the CI face"
        );
        assert!(battery_exit(false, 0).is_ok(), "a clean gate exits 0");
        let err = battery_exit(false, 10).expect_err("a red gate must exit nonzero");
        assert!(err.to_string().contains("10 fixture(s) RED"), "{err}");
    }

    /// THE killed-run detail-skip pin (T17b controller round): the
    /// battery must never burn a SECOND tier wall inside one fixture.
    /// A harness-KILLED run skips the detail pass (the in-process
    /// re-run on a wall-truncated board would only re-burn the same
    /// wall — it localizes nothing for a slowness timeout), a
    /// COMPLETED red keeps the localizer (the panic-class reproducer),
    /// and `--detail` (force) overrides even the skip. The matrix
    /// crosses flag × killed at every row and column
    /// (pin-failure-mode 13a); the killed × unforced cell is the
    /// discriminator the T17a-era `force || !failures.is_empty()`
    /// behavior fails. Mutation-verified: M1 (dropping `&& !timed_out`)
    /// dies on the killed-red arm, M2 (inverting to `&& timed_out`)
    /// dies on the completed-red arm, M3 (dropping the force term) dies
    /// on both force arms.
    #[test]
    fn detail_pass_policy_skips_killed_runs_and_keeps_the_localizer() {
        let integrity = vec![GateFailure::RunIntegrity {
            reason: "harness timeout — the run was killed".into(),
        }];
        // force column (both rows).
        assert!(
            detail_pass_should_run(true, &integrity, true),
            "--detail explicitly accepts the wall even on a killed run"
        );
        assert!(
            detail_pass_should_run(true, &[], false),
            "--detail on a green run"
        );
        // unforced column (both rows).
        assert!(
            detail_pass_should_run(false, &integrity, false),
            "a COMPLETED red keeps the detail-pass localizer"
        );
        assert!(
            !detail_pass_should_run(false, &integrity, true),
            "a KILLED red skips — no second tier wall inside one fixture"
        );
        assert!(
            !detail_pass_should_run(false, &[], false),
            "a green run stays silent (the historic face)"
        );
        // killed ⇒ integrity failure is structural (compare_directional
        // pushes it unconditionally on timed_out), so this cell is
        // unreachable in the loop; its conservative face stays silent.
        assert!(
            !detail_pass_should_run(false, &[], true),
            "killed-with-no-failures (structural dead arm) stays silent"
        );
    }

    /// THE truncation-boundary pin (quality-review NOTE-A / Q6 survivor):
    /// a world with MORE violation pairs than FIRST_N renders EXACTLY
    /// FIRST_N rows and says so in the header. The single-pair world of
    /// the shape pin could not see the `FIRST_N 10 -> 9` mutant
    /// (`min(9, 1) == min(10, 1)`); at 12 pairs the mutant renders 9 rows
    /// and this pin dies. The re-review R3 extension also holds the WALK
    /// ORDER (first = pair 0, last = pair 9): the `.take` →
    /// `.rev().take` mutant rendered the wrong 10 rows under the "first
    /// 10 in walk order" header with the count and header asserts green.
    #[test]
    fn localizer_truncates_violation_pairs_at_exactly_first_n() {
        let java = java_record(2, 0, 986.32);
        let outcome = rust_outcome(5, 12, 900.0);
        let failures = compare_directional(&java, &outcome);
        let detail = RustDetail {
            incomplete_total: 5,
            incomplete_nets: Vec::new(),
            violations: (0..12)
                .map(|i| ViolationPair {
                    a: i,
                    b: 100 + i,
                    layer: 1,
                    expected_clearance: 200.0,
                    actual_clearance: 100.0,
                })
                .collect(),
            terms: ScoreTerms {
                maximum: 1000.0,
                unrouted: 50.0,
                violation: 10.0,
                bend: 0.0,
                via: 0.0,
                trace_length: 0.0,
                normalized: 940.0,
            },
            trace_count: 1,
            via_count: 0,
            bend_count: 0,
        };
        let lines = localization_lines("x.dsn", &java, &outcome, &failures, Some(&detail), &[]);
        let text = lines.join("\n");
        let rendered_rows = lines
            .iter()
            .filter(|line| line.contains(" <-> item "))
            .count();
        assert_eq!(
            rendered_rows, 10,
            "exactly FIRST_N violation pairs may render: {text}"
        );
        assert!(
            text.contains("violation pairs (12 total, first 10 in walk order):"),
            "the header must state the total AND the truncation: {text}"
        );
        // Walk order (re-review R3): the FIRST rendered row must be the
        // walk-first pair (a=0) and the LAST the truncation boundary
        // (a=9) — a `.rev().take(FIRST_N)` renders the wrong 10 rows
        // under the same count+header.
        let rows: Vec<&str> = lines
            .iter()
            .filter(|line| line.contains(" <-> item "))
            .map(String::as_str)
            .collect();
        assert!(
            rows.first()
                .is_some_and(|row| row.contains("item 0 <-> item 100")),
            "the first rendered row must be the walk-first pair (a=0): {text}"
        );
        assert!(
            rows.last()
                .is_some_and(|row| row.contains("item 9 <-> item 109")),
            "the last rendered row must be pair 9 (the truncation boundary): {text}"
        );
    }

    // ------------------------------------------------------------------
    // The bin-freshness guard (buglog-184's structural close,
    // quality-review MAJOR-1). These pins build their own tiny
    // workspace-shaped world in a temp dir — they never touch the real
    // repo tree. Creation order sets mtimes (ns resolution), so the
    // stale/fresh shapes need no mtime-mutating dependency: a bin
    // written BEFORE the sources is the stale world; the bin written
    // AFTER them is the fresh one.
    // ------------------------------------------------------------------

    /// A temp `rust/` workspace-shaped root with `crates/k/src` in place.
    fn fresh_guard_world(tag: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let root = std::env::temp_dir().join(format!(
            "epic_fresh_guard_{tag}_{}_{}",
            std::process::id(),
            nanos
        ));
        let src = root.join("rust/crates/k/src");
        std::fs::create_dir_all(&src).expect("temp workspace src");
        root
    }

    /// THE kill face (the guard must be mutant-observable, not another
    /// prose caveat): a bin OLDER than the newest crate source BAILS,
    /// naming the resolved path and the rebuild command for BOTH
    /// profiles.
    #[test]
    fn freshness_guard_bails_on_stale_bin_naming_both_rebuilds() {
        let root = fresh_guard_world("stale");
        let bin = root.join("rust/target/release/epic-cli");
        std::fs::create_dir_all(bin.parent().expect("bin dir")).expect("bin dir");
        // Bin FIRST (older), sources AFTER (newer) — the stale shape.
        // (Creation order sets mtimes: the bin must PREDATE the newest
        // source; writing it last would make it the fresh side.) The
        // sleep is load-bearing: file timestamps come from the kernel's
        // COARSE clock (one tick per jiffy), so back-to-back writes tie
        // at the same mtime and the guard's `>=` would read the world
        // as fresh.
        std::fs::write(&bin, "stale bin bytes").expect("bin");
        std::thread::sleep(std::time::Duration::from_millis(50));
        std::fs::write(root.join("rust/crates/k/src/lib.rs"), "fn m() {}\n").expect("src");
        std::fs::write(root.join("rust/crates/k/Cargo.toml"), "[package]\n").expect("toml");
        let err = assert_epic_cli_fresh(&root, &bin).expect_err("stale bin must bail");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("STALE epic-cli bin"),
            "headline missing: {msg}"
        );
        assert!(
            msg.contains(bin.to_string_lossy().as_ref()),
            "resolved path missing: {msg}"
        );
        assert!(
            msg.contains("cargo build -p epic-cli\n"),
            "debug cmd missing: {msg}"
        );
        assert!(
            msg.contains("cargo build -p epic-cli --release"),
            "release cmd missing: {msg}"
        );
        assert!(
            msg.contains("cargo clean -p epic-cli\n"),
            "clean cmd missing: {msg}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The fresh face: bin newest → the guard is a no-op (it must never
    /// block the ordinary up-to-date run).
    #[test]
    fn freshness_guard_passes_when_bin_is_newest() {
        let root = fresh_guard_world("fresh");
        let bin = root.join("rust/target/debug/epic-cli");
        std::fs::create_dir_all(bin.parent().expect("bin dir")).expect("bin dir");
        // Lock and sources first, bin LAST — the fresh shape.
        std::fs::write(root.join("rust/Cargo.lock"), "# lock\n").expect("lock");
        std::fs::write(root.join("rust/crates/k/src/lib.rs"), "fn m() {}\n").expect("src");
        std::fs::write(&bin, "fresh bin bytes").expect("bin");
        assert_epic_cli_fresh(&root, &bin).expect("fresh bin must pass");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Resolution hardening (MAJOR-1 complementary): the repo fallback
    /// prefers RELEASE, with debug retained as the CI face. The
    /// never-adopt-a-debug-sibling rule is not directly assertable from
    /// a test process (`current_exe` is fixed, and lives under
    /// `target/debug/deps/` anyway) — it is proven live by the
    /// before/after determinism run in the MAJOR-1 fix round: the stale
    /// debug sibling present AND a fresh release bin resolving to the
    /// LIVE digest. Skipped when the shell exports EPIC_CLI — the
    /// override legitimately wins over everything asserted here, and
    /// tests must not mutate env.
    #[test]
    fn resolution_prefers_release_and_keeps_debug_ci_face() {
        if std::env::var("EPIC_CLI").is_ok() {
            return;
        }
        let root = fresh_guard_world("resolve");
        let release = root.join("rust/target/release/epic-cli");
        let debug = root.join("rust/target/debug/epic-cli");
        std::fs::create_dir_all(release.parent().expect("dir")).expect("dir");
        std::fs::create_dir_all(debug.parent().expect("dir")).expect("dir");
        std::fs::write(&release, "release").expect("release bin");
        std::fs::write(&debug, "debug").expect("debug bin");
        // Both present → RELEASE wins (debug-first before MAJOR-1).
        assert_eq!(
            resolve_epic_cli(&root).expect("resolve"),
            release,
            "the repo fallback must prefer the release profile"
        );
        // Debug-only → debug still resolves (the CI face).
        std::fs::remove_file(&release).expect("remove release bin");
        assert_eq!(
            resolve_epic_cli(&root).expect("resolve"),
            debug,
            "debug remains the CI fallback face"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The BUILT `epic-harness` binary, located the way the e2e smokes
    /// (and `resolve_epic_cli`) do: the unit-test exe lives in
    /// `target/<profile>/deps/`, so the bin target is one level up; fall
    /// back to the repo-relative debug path.
    ///
    /// STALE-BIN PRECONDITION (spec-review-t11-1 MINOR-3, live-witnessed
    /// false green): these smokes spawn the BUILT ARTIFACT, not the
    /// freshly-compiled test exe — `cargo test -p epic-harness`
    /// rebuilds the test binary but does NOT relink
    /// `target/debug/epic-harness` (witnessed: bin mtime 14:39 vs test
    /// binary 15:08), so after ANY harness-source edit the pin silently
    /// judges the STALE bin and can stay green on old bytes — the
    /// buglog-184 stale-bin-green family, one layer up. ALWAYS run
    /// `cargo build -p epic-harness` before judging these pins (or any
    /// mutant against them); CI orders its steps the same way (build →
    /// run). Where a pin routes a real router face, the spawned
    /// `epic-cli` needs its own freshness (`cargo build -p epic-cli`
    /// and/or `--release` — the gates' structural guard does not apply
    /// to these smokes).
    fn e2e_built_harness_bin(root: &std::path::Path) -> std::path::PathBuf {
        std::env::current_exe()
            .expect("test exe")
            .parent()
            .and_then(|parent| parent.parent())
            .map(|profile_dir| profile_dir.join("epic-harness"))
            .filter(|candidate| candidate.is_file())
            .unwrap_or_else(|| root.join("rust/target/debug/epic-harness"))
    }

    /// A throwaway world dir for the pin tests and smoke work-roots
    /// (NIT-6 dedup): the `epic-<tag>-<pid>` temp-dir boilerplate in ONE
    /// place. Each caller appends its own files (record worlds) or hands
    /// the path to `--work-root` (the smokes — NIT-5 hermeticity: a pin
    /// run must never rewrite the `runs/` battery evidence) and removes
    /// the dir when done.
    fn record_world(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("epic-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    /// Writes one record-body file into a pin's record world (NIT-6
    /// dedup: the closure was redefined per record pin). Returns the
    /// file's path.
    fn write_record(dir: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, body).expect("write record");
        path
    }

    /// THE five e2e smokes' ONE spawn face (NIT-6 dedup): the BUILT
    /// `epic-harness` binary, the CI-skip env baked, per-smoke env
    /// overrides (the stub smoke's `EPIC_CLI`), and the centralized
    /// spawn-panic message. Judgment precondition: [`e2e_built_harness_bin`]'s
    /// STALE-BIN PRECONDITION — build before judging.
    fn run_built_harness(
        root: &std::path::Path,
        args: &[&str],
        env: &[(&str, &std::ffi::OsStr)],
    ) -> std::process::Output {
        let bin = e2e_built_harness_bin(root);
        Command::new(&bin)
            .args(args)
            .env("EPIC_SKIP_GRADLE", "1")
            .envs(env.iter().copied())
            .output()
            .unwrap_or_else(|e| panic!("spawning {} (build it first): {e}", bin.display()))
    }

    /// THE dispatch smoke (quality-review MINOR-4 / Q5 survivor): the
    /// `run()` dispatcher's Determinism arm is clap wiring no pin can
    /// reach — the Q5 mutant swapped the arm to `compare_cmd` and the
    /// HARD gate silently became a report step (exit 0, ~0.6s, all pins
    /// green). This e2e runs the BUILT `epic-harness` binary's
    /// `router determinism` as a real subprocess and asserts the output
    /// is the DETERMINISM gate's own OK line. Java-free; ~1s; requires
    /// a FRESHLY LINKED `epic-harness` bin (`cargo build -p
    /// epic-harness` — see [`e2e_built_harness_bin`]'s stale-bin
    /// precondition) plus `cargo build -p epic-cli` (the determinism
    /// run spawns it). Run explicitly: `cargo test -p epic-harness
    /// --bin epic-harness e2e_router_determinism -- --ignored` (also
    /// after any dispatch refactor).
    #[test]
    #[ignore]
    fn e2e_router_determinism_subcommand_is_the_determinism_gate() {
        let root = crate::oracle::find_repo_root().expect("repo root");
        // NIT-5 hermeticity: the two runs land under a THROWAWAY work
        // root (run1/ + run2/ stay distinct paths — the trap-5 property
        // under test is untouched), never under runs/router-determinism.
        let work = record_world("t11-smoke-det");
        let work_str = work.to_string_lossy().into_owned();
        let output = run_built_harness(
            &root,
            &[
                "router",
                "determinism",
                "--fixture",
                DEFAULT_DETERMINISM_FIXTURE,
                "--work-root",
                &work_str,
            ],
            &[],
        );
        let _ = std::fs::remove_dir_all(&work);
        assert!(
            output.status.success(),
            "the determinism gate must exit 0: {} stderr: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("determinism: OK"),
            "the Determinism arm must run the DETERMINISM gate, not some other cmd: {stdout}"
        );
    }

    /// THE gate-exit smoke (quality re-review MINOR-R1): the
    /// `battery_exit(report_only, red)` CALL SITE is the extraction's
    /// remaining unguarded face — the R1 mutant bypassed the call
    /// (`let _ = (report_only, red); Ok(())` at the `compare_cmd` tail,
    /// fn left correct) and kept every unit pin green while gate mode
    /// exited 0 with a red fixture; post-T17-flip that makes a red
    /// battery exit 0 with CI green.
    ///
    /// RE-FACED M4-T11 (the original premise is STALE, witnessed live
    /// 2026-09-24): it drove `--fixture bm01` expecting the M3-era
    /// panic-class fast red — but the T17a integrity close ended bm01's
    /// panics and the T7 fanout stage turned the router-only row GREEN
    /// (the bare default-profile run exited 0), so the pin could no
    /// longer construct its red. The re-face keeps the R1 target — the
    /// battery_exit CALL SITE — with a CONSTRUCTED red that is cheap
    /// (~1s), profile-independent, and touches no committed artifact:
    /// `EPIC_CLI` points at a freshly written no-op stub (mtime = now,
    /// so the bin-freshness guard passes), the run produces NO manifest,
    /// the integrity gate goes red, and GATE MODE must exit NONZERO
    /// naming the red tally. The R1 bypass mutant survives nothing: with
    /// the call removed the stub world exits 0 and this pin dies. (The
    /// REAL reds of the current battery — bm01/bm06/bm11 full — are
    /// witnessed in the T11 battery log; a real red costs 80s-1800s of
    /// wall, wrong shape for a smoke.) Requires a FRESHLY LINKED
    /// `epic-harness` bin (`cargo build -p epic-harness` — see
    /// [`e2e_built_harness_bin`]'s stale-bin precondition; the stub
    /// replaces the routed bin, so no epic-cli build matters here).
    /// Run explicitly: `cargo test -p epic-harness --bin epic-harness
    /// e2e_router_compare_gate_mode -- --ignored` (also after any
    /// compare_cmd tail refactor).
    #[test]
    #[ignore]
    fn e2e_router_compare_gate_mode_exits_nonzero_on_a_red_battery() {
        let root = crate::oracle::find_repo_root().expect("repo root");
        // The no-op stub: exits 0 instantly, writes nothing — the
        // integrity-red face ("no manifest produced"). Written FRESH so
        // its mtime passes the structural bin-freshness guard.
        let stub_dir =
            std::env::temp_dir().join(format!("epic-t15-cli-stub-{}", std::process::id()));
        std::fs::create_dir_all(&stub_dir).expect("stub dir");
        let stub = stub_dir.join("epic-cli");
        std::fs::write(&stub, "#!/bin/sh\nexit 0\n").expect("stub body");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
                .expect("chmod stub");
        }
        let work = record_world("t11-smoke-stub");
        let work_str = work.to_string_lossy().into_owned();
        let output = run_built_harness(
            &root,
            &[
                "router",
                "compare",
                "--fixture",
                "bm08",
                "--work-root",
                &work_str,
            ],
            &[("EPIC_CLI", stub.as_os_str())],
        );
        let _ = std::fs::remove_dir_all(&stub_dir);
        let _ = std::fs::remove_dir_all(&work);
        assert!(
            !output.status.success(),
            "a red battery in gate mode must exit NONZERO (the battery_exit call site must be \
             reachable): status {}, stderr: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("[RED] DAC2020_boards/DAC2020_bm08.dsn"),
            "the red fixture's [RED] verdict line must render: {stdout}"
        );
        assert!(
            stdout.contains("integrity(no manifest produced)"),
            "the stub world's red face is the integrity gate (the numeric gates are not \
             judged): {stdout}"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("1 fixture(s) RED"),
            "the bail must tally the red: {stderr}"
        );
    }

    /// THE empty-battery smoke (quality re-review MINOR-R2): the bail's
    /// tally condition is cmd-level glue no pin can reach — the R2
    /// mutant (`green + red == 0` → `== 2`) survived every pin and
    /// silently restored the round-1 hazard (`--fixture zzz` exits 0 in
    /// both modes). This e2e runs the BUILT binary's
    /// `router compare --fixture zzz` in gate mode — the filter matches
    /// nothing, so the bail fires before any fixture runs (~0s) — and
    /// asserts the exit is NONZERO with the exact bail message on
    /// stderr. Java-free; needs a FRESHLY LINKED `epic-harness` bin
    /// (`cargo build -p epic-harness` — see
    /// [`e2e_built_harness_bin`]'s stale-bin precondition; no epic-cli:
    /// nothing spawns). Run explicitly: `cargo test -p epic-harness
    /// --bin epic-harness e2e_router_compare_empty_filter --
    /// --ignored`.
    #[test]
    #[ignore]
    fn e2e_router_compare_empty_filter_bails_instantly() {
        let root = crate::oracle::find_repo_root().expect("repo root");
        // NIT-5 hermeticity: uniform temp work-root (the bail fires before
        // any fixture runs, so nothing is ever written here anyway).
        let work = record_world("t11-smoke-empty");
        let work_str = work.to_string_lossy().into_owned();
        let output = run_built_harness(
            &root,
            &[
                "router",
                "compare",
                "--fixture",
                "zzz",
                "--work-root",
                &work_str,
            ],
            &[],
        );
        let _ = std::fs::remove_dir_all(&work);
        assert!(
            !output.status.success(),
            "a --fixture filter matching ZERO fixtures must exit NONZERO (the empty-battery \
             bail must fire): status {}",
            output.status
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("--fixture \"zzz\" matched no Tier A fixture"),
            "the bail must name the unmatched filter: stderr: {stderr}"
        );
    }

    /// THE full-profile battery smoke (M4-T11): the BUILT harness binary's
    /// `router compare --profile full` on ONE CHEAP fixture (bm08 — the
    /// smallest board AND the dispatch's F1/F1′ anchor) in GATE mode must
    /// exit 0 with the [PASS] verdict row, the `router compare[full]`
    /// banner, and the derived no-flags comparability claim. This is the
    /// profile wiring's end-to-end witness: the Rust side ran the
    /// assembled pipeline (no flags) against the M0 full-flow record and
    /// cleared all three directional gates. It does NOT duplicate the
    /// battery's own run — the battery is 11 fixtures; this smoke is the
    /// one-fixture gate-mode exit-code face (the report-mode battery log
    /// is the task evidence). Requires a FRESHLY LINKED `epic-harness`
    /// bin (`cargo build -p epic-harness` — see
    /// [`e2e_built_harness_bin`]'s stale-bin precondition); the spawned
    /// `epic-cli` resolves release-first (a fresh release bin routes
    /// bm08's full face in ~7s). Run explicitly: `cargo test -p
    /// epic-harness --bin epic-harness e2e_router_compare_full_profile
    /// -- --ignored`.
    #[test]
    #[ignore]
    fn e2e_router_compare_full_profile_passes_bm08_in_gate_mode() {
        let root = crate::oracle::find_repo_root().expect("repo root");
        // NIT-5 hermeticity: the run lands under a THROWAWAY work root —
        // the battery's runs/router-compare-full evidence is never touched
        // by a pin run.
        let work = record_world("t11-smoke-full");
        let work_str = work.to_string_lossy().into_owned();
        let output = run_built_harness(
            &root,
            &[
                "router",
                "compare",
                "--profile",
                "full",
                "--fixture",
                "bm08",
                "--work-root",
                &work_str,
            ],
            &[],
        );
        let _ = std::fs::remove_dir_all(&work);
        assert!(
            output.status.success(),
            "the full-profile bm08 row must PASS in gate mode: {} stderr: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("router compare[full]"),
            "the banner must name the full profile: {stdout}"
        );
        assert!(
            stdout.contains("no comparability flags"),
            "the derived comparability claim must be the full face: {stdout}"
        );
        assert!(
            stdout.contains("[PASS] DAC2020_boards/DAC2020_bm08.dsn"),
            "bm08's full-profile verdict row must be [PASS]: {stdout}"
        );
        assert!(
            !stdout.contains("router-only records"),
            "a full-profile run must never claim the router-only face: {stdout}"
        );
    }

    /// THE profile-switch loud-failure smoke (M4-T11): the capture/verify
    /// spelling `full-flow` is NOT a compare profile name — requesting it
    /// must exit NONZERO with the error naming BOTH accepted values,
    /// before any fixture runs (~0s). Kills the silent-fallback mutant
    /// (an unknown profile quietly meaning the default).
    #[test]
    #[ignore]
    fn e2e_router_compare_wrong_profile_fails_loudly() {
        let root = crate::oracle::find_repo_root().expect("repo root");
        // NIT-5 hermeticity: uniform temp work-root (the profile parse
        // fails before any fixture runs, so nothing is ever written here).
        let work = record_world("t11-smoke-wrong");
        let work_str = work.to_string_lossy().into_owned();
        let output = run_built_harness(
            &root,
            &[
                "router",
                "compare",
                "--profile",
                "full-flow",
                "--fixture",
                "zzz",
                "--work-root",
                &work_str,
            ],
            &[],
        );
        let _ = std::fs::remove_dir_all(&work);
        assert!(
            !output.status.success(),
            "a wrong profile name must exit NONZERO: status {}",
            output.status
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("unknown compare profile"),
            "the error must name the profile face: {stderr}"
        );
        assert!(
            stderr.contains("router-only") && stderr.contains("\"full\""),
            "the error must name BOTH accepted values: {stderr}"
        );
        // NIT-2: the clause that maps the capture-surface spelling rides
        // the REAL binary's error output (the user's actual failure face).
        assert!(
            stderr.contains("capture/verify surface spells the full profile"),
            "the error must map the capture-surface spelling: {stderr}"
        );
    }
}
