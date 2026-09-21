//! Route event-stream corpus (M3 Task 16) — the maze-level
//! differential parity corpus: the Java router's own trace stream (the
//! `RAW_SECTION assign/skip` rows of `MazeSearchEngine` plus the
//! `AutoroutePassRunner`'s `compare_trace_ripped_item` /
//! `compare_trace_route_item` rows) captured through the real
//! `runBatchLoop()` by `rust/harness/oracle/RouteEventProbe.java`,
//! mirrored by the Rust engine's stream, and compared row-for-row.
//!
//! Shape (the drc/index/undo corpus shell):
//! * `events golden` — ONE probe JVM per run (javac-compiled), run
//!   TWICE over the manifest; the two captures must be byte-identical
//!   (the anchors §4 determinism proof, bailing on the first differing
//!   line) — then the canonical re-serialization lands as
//!   `harness/corpus/events-golden.jsonl`.
//! * `events compare` — java-free and CI-able: every manifest fixture
//!   re-runs through the Rust batch driver IN-PROCESS with the
//!   world built FROM the golden's per-fixture `settings_witness` row
//!   (settings parity by construction — the Rust compare never
//!   resolves its own cost table), the trace stream is filtered to the
//!   four pinned event kinds, aligned by (kind, ordinal), and diffed
//!   FIELD-level (`key=value, ` grammar). The first divergence names
//!   the row pair and the FIRST DIFFERING FIELD.
//!
//! Row normalization contract (the probe's `normalize`): a 5-arg
//! granular row `"[%s] [%s] %s: %s"` is stored as `operation message`
//! — the `[method]` wrapper and the `: <impacted items>` tail are
//! stripped, the OPERATION kept, so Java's stored text is byte-equal
//! to the Rust mirror's row text. One-arg rows (`RAW_SECTION`) pass
//! through untouched. No wall-clock value appears in any tapped row.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use serde::{Deserialize, Serialize};

use crate::corpus_common::load_jsonl;

/// The four pinned event kinds, in compare order. A trace row's kind
/// is its longest matching prefix — `RAW_SECTION skip` before
/// nothing else shares it, and the `compare_trace_*` tokens are
/// self-delimiting (the Rust rows lead with the same tokens).
pub const EVENT_KINDS: [&str; 4] = [
    "RAW_SECTION assign",
    "RAW_SECTION skip",
    "compare_trace_ripped_item",
    "compare_trace_route_item",
];

/// Classifies one stored trace-row text by its leading token;
/// `None` = an unpinned row (the compare must fail on those — the
/// golden only ever carries pinned kinds).
#[must_use]
pub fn kind_of(msg: &str) -> Option<&'static str> {
    EVENT_KINDS
        .iter()
        .copied()
        .find(|kind| msg.starts_with(kind))
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

/// The `events` subcommands.
#[derive(Subcommand)]
pub enum EventsCommand {
    /// Capture the Java probe stream over the manifest (run TWICE, the
    /// two captures must be byte-identical) and write the golden.
    Golden {
        #[arg(long, default_value = "harness/corpus/events-manifest.jsonl")]
        manifest: PathBuf,
        #[arg(long, default_value = "harness/corpus/events-golden.jsonl")]
        out: PathBuf,
    },
    /// Re-run every manifest fixture through the Rust engine in-process
    /// with the world built from the golden's settings witness, and
    /// diff the event streams kind-ordinally against the committed
    /// golden (java-free, CI-able). Exits 1 on the first divergence.
    Compare {
        #[arg(long, default_value = "harness/corpus/events-manifest.jsonl")]
        manifest: PathBuf,
        #[arg(long, default_value = "harness/corpus/events-golden.jsonl")]
        golden: PathBuf,
    },
}

/// Runs the `events` pipeline (`jvm_xmx` bounds the capture JVM only —
/// the compare never spawns one).
pub fn run(cmd: EventsCommand, jvm_xmx: &str) -> Result<()> {
    let repo_root = crate::oracle::find_repo_root()?;
    match cmd {
        EventsCommand::Golden { manifest, out } => golden(&repo_root, &manifest, &out, jvm_xmx),
        EventsCommand::Compare { manifest, golden } => compare(&repo_root, &manifest, &golden),
    }
}

// ---------------------------------------------------------------------------
// Manifest (committed: harness/corpus/events-manifest.jsonl)
// ---------------------------------------------------------------------------

/// One manifest line: the fixture, its corpus id, and the ONE tuned
/// scalar of its world (`start_ripup_costs` — 1 for the completion
/// worlds, 40000 for the forced-ripup e1 world where the pass-2 reroute
/// must take the detour instead of ripping back). Field order
/// id-then-path-then-cost is committed-byte.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventsManifestEntry {
    pub id: String,
    pub path: String,
    pub start_ripup_costs: i32,
}

// ---------------------------------------------------------------------------
// Golden rows (the probe's JSONL, canonicalized)
// ---------------------------------------------------------------------------

/// One witness layer of the settings witness — the `AutorouteControl`
/// view of the world's cost table, doubles as Java `Double.toString`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WitnessLayer {
    pub horizontal: String,
    pub vertical: String,
    pub bend: String,
    pub active: bool,
}

/// The per-fixture world (`type == "settings_witness"`) — the Rust
/// compare builds its `BatchSettings` FROM this row (settings parity by
/// construction). Field order is the Gson insertion order; the golden's
/// canonical bytes depend on the serde declaration order matching it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsWitness {
    pub fixture: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub layers: Vec<WitnessLayer>,
    pub via_costs: i32,
    pub plane_via_costs: i32,
    pub vias_allowed: bool,
    pub automatic_neckdown: bool,
    pub start_ripup_costs: i32,
    pub fanout_enabled: bool,
    pub run_router: bool,
    pub max_items: Option<i32>,
    pub unrouted_net_penalty: String,
    pub clearance_violation_penalty: String,
    pub bend_penalty: String,
    pub scoring_via_costs: i32,
    pub max_passes: i32,
}

/// One tapped trace row (`type == "trace_row"`) — the normalized event
/// text (`operation message` for the 5-arg kinds, bare for RAW_SECTION).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraceRow {
    pub fixture: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub msg: String,
}

/// The run outcome witness (`type == "run"`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunRow {
    pub fixture: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub returned: bool,
}

/// The incompletes witness (`type == "incompletes"`) — the per-net SUM
/// against the endpoint lower bound.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IncompletesRow {
    pub fixture: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub incomplete_count: i32,
    pub max_connections: i32,
}

/// One golden line — any of the four row shapes (untagged; the
/// `deny_unknown_fields` on every variant makes the shapes disjoint).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum GoldenRow {
    Witness(SettingsWitness),
    Trace(TraceRow),
    Run(RunRow),
    Incompletes(IncompletesRow),
}

// ---------------------------------------------------------------------------
// `events golden` — the probe JVM, run twice, byte-diffed
// ---------------------------------------------------------------------------

/// The probe's raw stdout filtered to JSONL rows (`{"fixture"`-led;
/// the jar's console appender prints its INFO rows to the same stream).
fn probe_argv(repo_root: &Path, entries: &[EventsManifestEntry]) -> Vec<String> {
    let mut argv = Vec::new();
    for entry in entries {
        argv.push(repo_root.join(&entry.path).to_string_lossy().into_owned());
        argv.push(entry.start_ripup_costs.to_string());
    }
    argv
}

/// Runs the probe ONCE over the manifest, returning the JSONL rows in
/// emission order.
fn run_probe(
    java: &Path,
    jar: &Path,
    classes_dir: &Path,
    repo_root: &Path,
    entries: &[EventsManifestEntry],
    jvm_xmx: &str,
    stderr_path: &Path,
) -> Result<Vec<String>> {
    let classpath = format!("{}:{}", jar.display(), classes_dir.display());
    let stderr_file = std::fs::File::create(stderr_path)
        .with_context(|| format!("creating {}", stderr_path.display()))?;
    let mut child = std::process::Command::new(java)
        .arg(format!("-Xmx{jvm_xmx}"))
        // Locale-pinned (the drc corpus's tr-TR lesson).
        .arg("-Duser.language=en")
        .arg("-Duser.country=US")
        .arg("-cp")
        .arg(&classpath)
        .arg("app.freerouting.autoroute.pipeline.RouteEventProbe")
        .args(probe_argv(repo_root, entries))
        .current_dir(repo_root)
        .stdout(std::process::Stdio::piped())
        .stderr(stderr_file)
        .spawn()
        .with_context(|| format!("spawning {} with the route event probe", java.display()))?;
    // Byte-wise read of the `{"fixture"` lines (the drc corpus's
    // binary-safe read_until discipline).
    let stdout = child.stdout.take().context("probe stdout not captured")?;
    let mut fresh_lines = Vec::new();
    {
        use std::io::BufRead;
        let mut reader = std::io::BufReader::new(stdout);
        let mut raw = Vec::new();
        loop {
            raw.clear();
            let read = reader
                .read_until(b'\n', &mut raw)
                .context("reading probe stdout")?;
            if read == 0 {
                break;
            }
            if raw.starts_with(b"{\"fixture\"") {
                fresh_lines.push(String::from_utf8_lossy(&raw).trim_end().to_string());
            }
        }
    }
    drop(child.stderr.take());
    let status = child.wait().context("waiting for the probe")?;
    if !status.success() {
        let stderr = std::fs::read_to_string(stderr_path).unwrap_or_default();
        bail!(
            "route event probe failed with {status} (captured {} line(s) before failure):\n{}",
            fresh_lines.len(),
            stderr.trim_end()
        );
    }
    Ok(fresh_lines)
}

fn golden(repo_root: &Path, manifest: &Path, out: &Path, jvm_xmx: &str) -> Result<()> {
    let started = std::time::Instant::now();
    let java = crate::oracle::resolve_java()?;
    let javac = java.with_file_name("javac");
    anyhow::ensure!(
        javac.is_file(),
        "javac not found next to {} — the JDK is required for the route event probe",
        java.display()
    );
    let jar = crate::oracle::jar_path(repo_root);
    anyhow::ensure!(
        jar.is_file(),
        "oracle jar missing at {} — build it once with `./gradlew executableJar`",
        jar.display()
    );
    let oracle_src = repo_root.join("rust/harness/oracle/RouteEventProbe.java");
    anyhow::ensure!(
        oracle_src.is_file(),
        "oracle probe missing at {}",
        oracle_src.display()
    );
    let manifest_path = crate::dsn_corpus::resolve_input(repo_root, manifest);
    let entries: Vec<EventsManifestEntry> = load_jsonl(&manifest_path, "manifest")?;
    anyhow::ensure!(
        !entries.is_empty(),
        "manifest {} is empty",
        manifest_path.display()
    );

    // Compile the package-declared probe into a temp classes dir.
    // Every bail from here to the final cleanup goes through
    // `cleanup_temps` (the index-corpus leak lesson).
    let classes_dir =
        std::env::temp_dir().join(format!("epic-events-probe-classes-{}", std::process::id()));
    let stderr_path = std::env::temp_dir().join(format!(
        "epic-events-probe-stderr-{}.log",
        std::process::id()
    ));
    fn cleanup_temps(classes_dir: &Path, stderr_path: &Path) {
        let _ = std::fs::remove_dir_all(classes_dir);
        let _ = std::fs::remove_file(stderr_path);
    }
    std::fs::create_dir_all(&classes_dir)
        .with_context(|| format!("creating {}", classes_dir.display()))?;
    let compile = std::process::Command::new(&javac)
        .arg("-cp")
        .arg(&jar)
        .arg("-d")
        .arg(&classes_dir)
        .arg(&oracle_src)
        .output()
        .with_context(|| format!("running javac on {}", oracle_src.display()))
        .inspect_err(|_| cleanup_temps(&classes_dir, &stderr_path))?;
    if !compile.status.success() {
        let stderr = String::from_utf8_lossy(&compile.stderr).into_owned();
        cleanup_temps(&classes_dir, &stderr_path);
        bail!("javac failed:\n{stderr}");
    }

    // THE DETERMINISM PROOF (anchors §4): the probe runs TWICE over the
    // whole manifest; a single differing row bails the capture.
    let first = run_probe(
        &java,
        &jar,
        &classes_dir,
        repo_root,
        &entries,
        jvm_xmx,
        &stderr_path,
    )
    .inspect_err(|_| cleanup_temps(&classes_dir, &stderr_path))?;
    let second = run_probe(
        &java,
        &jar,
        &classes_dir,
        repo_root,
        &entries,
        jvm_xmx,
        &stderr_path,
    )
    .inspect_err(|_| cleanup_temps(&classes_dir, &stderr_path))?;
    let _ = std::fs::remove_file(&stderr_path);
    let _ = std::fs::remove_dir_all(&classes_dir);
    for (index, (a, b)) in first.iter().zip(&second).enumerate() {
        if a != b {
            bail!(
                "route event capture is NONDETERMINISTIC: line {} differs between the double runs\n  run 1: {}\n  run 2: {}",
                index + 1,
                crate::corpus_common::truncate(a),
                crate::corpus_common::truncate(b)
            );
        }
    }
    anyhow::ensure!(
        first.len() == second.len(),
        "route event capture is NONDETERMINISTIC: {} line(s) vs {} on the double runs",
        first.len(),
        second.len()
    );

    // Canonical re-serialization (the probe's Gson HTML-escapes `=`,
    // serde_json does not — the golden bytes are ALWAYS the serde
    // canonical form, never raw probe bytes).
    let mut records = Vec::new();
    for (index, line) in first.iter().enumerate() {
        let record: GoldenRow = serde_json::from_str(line)
            .with_context(|| format!("parsing probe line {}: {line}", index + 1))?;
        records.push(record);
    }
    sanity_check(&records)?;

    let out_path = crate::dsn_corpus::resolve_output(repo_root, out);
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let mut file = std::fs::File::create(&out_path)
        .with_context(|| format!("creating {}", out_path.display()))?;
    use std::io::Write as _;
    for record in &records {
        writeln!(
            file,
            "{}",
            serde_json::to_string(record).expect("golden row serialization cannot fail")
        )
        .with_context(|| format!("writing {}", out_path.display()))?;
    }
    println!(
        "captured {} route event row(s) over {} fixture(s) to {} in {:.1}s (double-run byte-identical)",
        records.len(),
        entries.len(),
        out_path.display(),
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

/// Capture sanity: every fixture's rows are CONTIGUOUS and every
/// fixture opens with its settings_witness, closes with run +
/// incompletes (the probe's world structure).
fn sanity_check(records: &[GoldenRow]) -> Result<()> {
    let mut seen_fixtures: Vec<String> = Vec::new();
    for record in records {
        let fixture = match record {
            GoldenRow::Witness(w) => {
                anyhow::ensure!(
                    w.kind == "settings_witness",
                    "witness row carries type {}",
                    w.kind
                );
                w.fixture.clone()
            }
            GoldenRow::Trace(t) => {
                anyhow::ensure!(t.kind == "trace_row", "trace row carries type {}", t.kind);
                t.fixture.clone()
            }
            GoldenRow::Run(r) => {
                anyhow::ensure!(r.kind == "run", "run row carries type {}", r.kind);
                r.fixture.clone()
            }
            GoldenRow::Incompletes(i) => {
                anyhow::ensure!(
                    i.kind == "incompletes",
                    "incompletes row carries type {}",
                    i.kind
                );
                i.fixture.clone()
            }
        };
        // Contiguity: a fixture's rows never interleave with another's.
        match seen_fixtures.last() {
            Some(last) if *last == fixture => {}
            Some(last) => {
                anyhow::ensure!(
                    !seen_fixtures.contains(&fixture),
                    "fixture {fixture} rows re-open after {last} — the stream is not contiguous"
                );
                seen_fixtures.push(fixture);
            }
            None => seen_fixtures.push(fixture),
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// `events compare` — java-free, CI-able
// ---------------------------------------------------------------------------

/// The Rust-side run of one fixture world.
struct RustWorld {
    /// The captured driver rows (level, text), the raw stream.
    rows: Vec<(&'static str, String)>,
    /// Java `run.returned`.
    returned: bool,
    /// Java `incompletes.incomplete_count` (the per-net SUM).
    incomplete_count: i32,
    /// Java `incompletes.max_connections`.
    max_connections: i32,
}

/// Builds the batch settings FROM the witness (settings parity by
/// construction — the compare never resolves its own cost table).
fn batch_settings_from_witness(
    witness: &SettingsWitness,
) -> Result<epic_router::pipeline::batch::BatchSettings> {
    use epic_router::control::{ExpansionCostFactor, RouterSettingsIr};
    use epic_router::pipeline::batch::BatchSettings;
    use epic_router::pipeline::board_statistics::{
        RouterSettingsScoring, default_routing_cost_settings,
    };

    let mut trace_costs = Vec::new();
    let mut bend_costs = Vec::new();
    let mut layer_active = Vec::new();
    for layer in &witness.layers {
        trace_costs.push(ExpansionCostFactor {
            horizontal: layer
                .horizontal
                .parse()
                .with_context(|| format!("witness horizontal cost {:?}", layer.horizontal))?,
            vertical: layer
                .vertical
                .parse()
                .with_context(|| format!("witness vertical cost {:?}", layer.vertical))?,
        });
        bend_costs.push(
            layer
                .bend
                .parse()
                .with_context(|| format!("witness bend cost {:?}", layer.bend))?,
        );
        layer_active.push(layer.active);
    }
    let ir = RouterSettingsIr {
        trace_costs,
        via_costs: witness.via_costs,
        vias_allowed: witness.vias_allowed,
        bend_costs,
        layer_active,
        automatic_neckdown: witness.automatic_neckdown,
        start_ripup_costs: witness.start_ripup_costs,
    };
    // The score box from the witness penalties (Java `Double.toString`
    // text); the engine-facing fields ride the defaults (the witness
    // never carries them — the probe's baseSettings leaves the board
    // fallbacks).
    let mut costs = default_routing_cost_settings();
    costs.unrouted_net_penalty = Some(witness.unrouted_net_penalty.parse().with_context(|| {
        format!(
            "witness unrouted_net_penalty {:?}",
            witness.unrouted_net_penalty
        )
    })?);
    costs.clearance_violation_penalty = Some(
        witness
            .clearance_violation_penalty
            .parse()
            .with_context(|| {
                format!(
                    "witness clearance_violation_penalty {:?}",
                    witness.clearance_violation_penalty
                )
            })?,
    );
    costs.bend_penalty = Some(
        witness
            .bend_penalty
            .parse()
            .with_context(|| format!("witness bend_penalty {:?}", witness.bend_penalty))?,
    );
    costs.via_costs = Some(witness.scoring_via_costs);
    costs.plane_via_costs = Some(witness.plane_via_costs);
    costs.start_ripup_costs = Some(witness.start_ripup_costs);
    let scoring = RouterSettingsScoring {
        scoring: Some(costs),
        router_scoring: None,
    };
    let mut batch = BatchSettings::new(ir, scoring);
    // The probe world's driver scalars (the 7-arg ctor + the tuned
    // settings box): maxPasses 10, fanout OFF with the job-ctor
    // removeUnconnectedVias = !fanout derivation, pullTight 500.
    batch.max_passes = Some(witness.max_passes);
    batch.max_items = witness.max_items;
    batch.fanout_enabled = witness.fanout_enabled;
    batch.remove_unconnected_vias = !witness.fanout_enabled;
    batch.run_router = witness.run_router;
    batch.via_costs = witness.via_costs;
    batch.plane_via_costs = witness.plane_via_costs;
    Ok(batch)
}

/// Runs one fixture through the Rust batch driver in-process with the
/// CALLER's sink — the probe's parse (read_board → board → reinsert;
/// NO normalize, the probe parses fresh like `DsnReader.readBoard`) +
/// the witness world. Split from [`run_rust_world`] so the gating pin
/// can inject a trace-disabled sink and watch which rows still flow.
fn run_world_with_sink(
    dsn: &Path,
    witness: &SettingsWitness,
    sink: &mut dyn epic_router::pipeline::event_sink::DriverSink,
) -> Result<(bool, i32, i32)> {
    use epic_board::board::Board;
    use epic_board::tree_manager::SearchTreeManager;
    use epic_dsn::reader::{DsnReadResult, read_board};
    use epic_router::pipeline::batch::{BatchDriver, StopFace};

    let bytes = std::fs::read(dsn).with_context(|| format!("reading {}", dsn.display()))?;
    let mut ses = epic_dsn::ses_board::SesBoard::new();
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

    let batch = batch_settings_from_witness(witness)?;
    let mut driver = BatchDriver::new(&mut manager, &mut board, batch, StopFace::default());
    let returned = driver.run(sink)?;
    drop(driver);
    let (max_connections, rows) = epic_drc::incompletes::all_incompletes(&manager, &mut board);
    let total: usize = rows.iter().map(|row| row.incomplete_count).sum();
    Ok((
        returned,
        i32::try_from(total).unwrap_or(i32::MAX),
        // Java's `drc.maxConnections` is an int; the i64 slot clamps.
        i32::try_from(max_connections).unwrap_or(i32::MAX),
    ))
}

/// Runs one fixture through the Rust batch driver in-process with the
/// capture sink — the probe's parse (see [`run_world_with_sink`]) +
/// the witness world.
fn run_rust_world(dsn: &Path, witness: &SettingsWitness) -> Result<RustWorld> {
    use epic_router::pipeline::event_sink::CaptureDriverSink;

    let mut sink = CaptureDriverSink::default();
    let (returned, incomplete_count, max_connections) =
        run_world_with_sink(dsn, witness, &mut sink)?;
    Ok(RustWorld {
        rows: sink.rows,
        returned,
        incomplete_count,
        max_connections,
    })
}

/// The field name of one `", "` segment: the head before `=` with the
/// row's kind prefix stripped (the kind prefix rides the FIRST segment
/// — `"RAW_SECTION assign selected_section=0"` names
/// `selected_section`; a segment that is nothing but a kind token
/// stays as-is, which never occurs in real rows). Shared by
/// [`first_differing_field`] and the route-row skeleton pin.
#[must_use]
pub fn field_name_of_segment(segment: &str) -> String {
    let body = segment.split('=').next().unwrap_or(segment);
    EVENT_KINDS
        .iter()
        .find_map(|kind| {
            body.strip_prefix(kind)
                .map(str::trim_start)
                .filter(|rest| !rest.is_empty())
                .map(ToString::to_string)
        })
        .unwrap_or_else(|| body.to_string())
}

/// The field-level diff of two same-kind rows: the FIRST differing
/// `key=value` segment (the `", "` split is safe — no pinned value
/// carries the two-char sequence), rendered as
/// (field, golden segment, rust segment). Segment-count drift is a
/// difference too (a dropped field is a divergence, not a pass).
#[must_use]
pub fn first_differing_field(golden: &str, rust: &str) -> Option<(String, String, String)> {
    let golden_segments: Vec<&str> = golden.split(", ").collect();
    let rust_segments: Vec<&str> = rust.split(", ").collect();
    let index = (0..golden_segments.len().max(rust_segments.len()))
        .find(|&i| golden_segments.get(i) != rust_segments.get(i))?;
    let field = |segment: &str| field_name_of_segment(segment);
    match (golden_segments.get(index), rust_segments.get(index)) {
        (Some(g), Some(r)) => Some((field(g), (*g).to_string(), (*r).to_string())),
        (Some(g), None) => Some((field(g), (*g).to_string(), "<missing>".to_string())),
        (None, Some(r)) => Some((field(r), "<missing>".to_string(), (*r).to_string())),
        (None, None) => None,
    }
}

/// The kind-grouped trace rows of one side, order-preserving. `strict`
/// is the GOLDEN-side face (a committed golden row must be a pinned
/// kind — anything else means the probe tap leaked); the RUST stream
/// carries every driver face (task_state, the info/debug rows) and is
/// FILTERED to the four kinds, exactly like the Java tap.
fn rows_by_kind<'a>(
    msgs: impl IntoIterator<Item = &'a String>,
    strict: bool,
) -> Result<[Vec<&'a String>; 4]> {
    let mut grouped: [Vec<&String>; 4] = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
    for msg in msgs {
        let Some(kind) = kind_of(msg) else {
            anyhow::ensure!(
                !strict,
                "unpinned GOLDEN trace row (the probe tap leaked): {msg}"
            );
            continue;
        };
        let slot = EVENT_KINDS.iter().position(|k| *k == kind);
        let slot = slot.with_context(|| format!("unclassified kind {kind:?}"))?;
        grouped[slot].push(msg);
    }
    Ok(grouped)
}

/// The (kind, ordinal) alignment + field-level diff of ONE kind's row
/// streams — the compare's core, extracted so the pins exercise the
/// real code path (an identity pin over a re-implementation would be
/// vacuous). Any length drift is caught at the first missing/extra
/// ordinal (a zip alone would silently pass a truncated Rust stream);
/// any text drift names the row pair and the FIRST DIFFERING FIELD.
fn diff_kind_streams(fixture: &str, kind: &str, gold: &[&String], rust: &[&String]) -> Result<()> {
    for ordinal in 0..gold.len().max(rust.len()) {
        match (gold.get(ordinal), rust.get(ordinal)) {
            (Some(g), Some(r)) => {
                if g != r {
                    if let Some((field, g_seg, r_seg)) = first_differing_field(g, r) {
                        bail!(
                            "events compare DIVERGENCE: fixture={fixture} kind={kind} ordinal={} (1-based)\n  golden msg: {}\n  rust   msg: {}\n  FIRST DIFFERING FIELD: {field}\n    golden: {}\n    rust:   {}",
                            ordinal + 1,
                            crate::corpus_common::truncate(g),
                            crate::corpus_common::truncate(r),
                            crate::corpus_common::truncate(&g_seg),
                            crate::corpus_common::truncate(&r_seg),
                        );
                    }
                    bail!(
                        "events compare DIVERGENCE: fixture={fixture} kind={kind} ordinal={} (1-based)\n  golden msg: {}\n  rust   msg: {} (same field split, differing text)",
                        ordinal + 1,
                        crate::corpus_common::truncate(g),
                        crate::corpus_common::truncate(r),
                    );
                }
            }
            (Some(g), None) => bail!(
                "events compare DIVERGENCE: fixture={fixture} kind={kind} has {} golden row(s) but {} Rust row(s); first missing Rust ordinal={} (1-based): {}",
                gold.len(),
                rust.len(),
                ordinal + 1,
                crate::corpus_common::truncate(g),
            ),
            (None, Some(r)) => bail!(
                "events compare DIVERGENCE: fixture={fixture} kind={kind} has {} Rust row(s) but {} golden row(s); first extra Rust ordinal={} (1-based): {}",
                rust.len(),
                gold.len(),
                ordinal + 1,
                crate::corpus_common::truncate(r),
            ),
            (None, None) => unreachable!("at least one side has the ordinal"),
        }
    }
    Ok(())
}

fn compare(repo_root: &Path, manifest: &Path, golden: &Path) -> Result<()> {
    let started = std::time::Instant::now();
    let manifest_path = crate::dsn_corpus::resolve_input(repo_root, manifest);
    let entries: Vec<EventsManifestEntry> = load_jsonl(&manifest_path, "manifest")?;
    anyhow::ensure!(
        !entries.is_empty(),
        "manifest {} is empty",
        manifest_path.display()
    );
    let golden_path = crate::dsn_corpus::resolve_input(repo_root, golden);
    let records: Vec<GoldenRow> = load_jsonl(&golden_path, "golden")?;
    sanity_check(&records)?;

    // Group the golden rows per fixture (order-preserving; the
    // contiguous-fixture invariant holds by sanity_check).
    let mut per_fixture: Vec<(String, Vec<&GoldenRow>)> = Vec::new();
    for record in &records {
        let fixture = match record {
            GoldenRow::Witness(w) => &w.fixture,
            GoldenRow::Trace(t) => &t.fixture,
            GoldenRow::Run(r) => &r.fixture,
            GoldenRow::Incompletes(i) => &i.fixture,
        };
        match per_fixture.last_mut() {
            Some((last, rows)) if last == fixture => rows.push(record),
            _ => per_fixture.push((fixture.clone(), vec![record])),
        }
    }
    anyhow::ensure!(
        per_fixture.len() == entries.len(),
        "golden {} covers {} fixture(s) but manifest {} lists {}",
        golden_path.display(),
        per_fixture.len(),
        manifest_path.display(),
        entries.len()
    );

    let mut total_rows = 0usize;
    for (entry, (fixture, rows)) in entries.iter().zip(&per_fixture) {
        let fixture_name = Path::new(&entry.path)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .with_context(|| format!("manifest path {} has no stem", entry.path))?
            .to_string();
        anyhow::ensure!(
            *fixture == fixture_name,
            "manifest entry {} maps to fixture {fixture_name} but the golden rows say {fixture}",
            entry.id
        );

        // The witness opens the fixture's row block.
        let GoldenRow::Witness(witness) = rows
            .first()
            .with_context(|| format!("fixture {fixture} has an empty golden row block"))?
        else {
            bail!("fixture {fixture} does not open with a settings_witness row");
        };
        // The manifest's tuned scalar must AGREE with the witness
        // (a mismatch means the manifest and the golden drifted apart).
        anyhow::ensure!(
            witness.start_ripup_costs == entry.start_ripup_costs,
            "fixture {fixture}: manifest start_ripup_costs {} != witness {} — recapture needed",
            entry.start_ripup_costs,
            witness.start_ripup_costs
        );

        let dsn = repo_root.join(&entry.path);
        let world = run_rust_world(&dsn, witness)
            .with_context(|| format!("Rust run of fixture {fixture}"))?;

        // The golden trace rows of this fixture.
        let golden_msgs: Vec<String> = rows
            .iter()
            .filter_map(|row| match row {
                GoldenRow::Trace(t) => Some(t.msg.clone()),
                _ => None,
            })
            .collect();
        let rust_msgs: Vec<String> = world.rows.iter().map(|(_, msg)| msg.clone()).collect();
        total_rows += golden_msgs.len();

        let golden_grouped = rows_by_kind(&golden_msgs, true)?;
        let rust_grouped = rows_by_kind(&rust_msgs, false)?;

        // (kind, ordinal) alignment + field-level diff.
        for (slot, kind) in EVENT_KINDS.iter().enumerate() {
            diff_kind_streams(fixture, kind, &golden_grouped[slot], &rust_grouped[slot])?;
        }

        // The run-outcome + incompletes witnesses.
        let golden_run = rows.iter().find_map(|row| match row {
            GoldenRow::Run(r) => Some(r.clone()),
            _ => None,
        });
        let golden_incompletes = rows.iter().find_map(|row| match row {
            GoldenRow::Incompletes(i) => Some(i.clone()),
            _ => None,
        });
        if let Some(run) = golden_run {
            anyhow::ensure!(
                run.returned == world.returned,
                "events compare DIVERGENCE: fixture={fixture} run.returned golden={} rust={}",
                run.returned,
                world.returned
            );
        }
        if let Some(incompletes) = golden_incompletes {
            anyhow::ensure!(
                incompletes.incomplete_count == world.incomplete_count
                    && incompletes.max_connections == world.max_connections,
                "events compare DIVERGENCE: fixture={fixture} incompletes golden=(sum {}, max {}) rust=(sum {}, max {})",
                incompletes.incomplete_count,
                incompletes.max_connections,
                world.incomplete_count,
                world.max_connections,
            );
        }
        println!(
            "  ok fixture={fixture} trace_rows={} assign={} skip={} ripped={} routed={} returned={} incompletes={}",
            golden_msgs.len(),
            golden_grouped[0].len(),
            golden_grouped[1].len(),
            golden_grouped[2].len(),
            golden_grouped[3].len(),
            world.returned,
            world.incomplete_count,
        );
    }
    println!(
        "events compare: {} fixture(s), {} golden trace row(s) aligned in {:.1}s",
        entries.len(),
        total_rows,
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Pins (fast, synthetic — no filesystem, no JVM)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod pins {
    use super::*;

    /// The kind classifier is PREFIX-total over the four pinned kinds
    /// and rejects everything else — an unpinned row must fail the
    /// compare, not silently pass.
    #[test]
    fn kind_of_classifies_the_four_pinned_prefixes_only() {
        assert_eq!(
            kind_of("RAW_SECTION assign selected_section=0, net=1"),
            Some("RAW_SECTION assign")
        );
        assert_eq!(
            kind_of("RAW_SECTION skip selected_section=1, occupied=true"),
            Some("RAW_SECTION skip")
        );
        assert_eq!(
            kind_of("compare_trace_ripped_item source_item=6, ripped_id=9"),
            Some("compare_trace_ripped_item")
        );
        assert_eq!(
            kind_of("compare_trace_route_item Routing Pin -> result=ROUTED"),
            Some("compare_trace_route_item")
        );
        assert_eq!(kind_of("compare_unrouted_net pass=1"), None);
        assert_eq!(kind_of("RAW_SECTION"), None, "bare token is not a kind");
        assert_eq!(kind_of(""), None);
    }

    /// The rows_by_kind grouping preserves per-kind order. Two faces:
    /// STRICT (the golden side) rejects an unpinned row — the probe tap
    /// must never leak a foreign kind into the committed golden — while
    /// the FILTER face (the Rust stream) ignores the driver's other
    /// observability rows exactly like the Java tap does.
    #[test]
    fn rows_by_kind_groups_order_preserving_and_faces_strict_vs_filter() {
        let msgs = vec![
            "RAW_SECTION assign selected_section=0".to_string(),
            "RAW_SECTION skip selected_section=1".to_string(),
            "RAW_SECTION assign selected_section=1".to_string(),
            "compare_trace_route_item Routing Pin -> result=ROUTED".to_string(),
        ];
        let grouped = rows_by_kind(&msgs, true).expect("all pinned");
        assert_eq!(grouped[0].len(), 2, "both assign rows kept, in order");
        assert_eq!(grouped[0][0], &msgs[0]);
        assert_eq!(grouped[0][1], &msgs[2]);
        assert_eq!(grouped[1].len(), 1);
        assert_eq!(grouped[2].len(), 0);
        assert_eq!(grouped[3].len(), 1);
        // Golden face: an unpinned row must fail strict grouping.
        let with_stray = vec!["compare_trace_dump_net_items x".to_string()];
        assert!(
            rows_by_kind(&with_stray, true).is_err(),
            "an unpinned GOLDEN row must fail the grouping"
        );
        // Rust face: the same stray row is a filtered driver face, and
        // a real task_state row passes through untouched.
        let rust_stream = vec![
            "task_state state=STARTED pass=0 hash=abc".to_string(),
            "RAW_SECTION assign selected_section=0".to_string(),
            "Auto-routing pass #1 on board 'abc' was completed in 0.42 seconds.".to_string(),
        ];
        let filtered = rows_by_kind(&rust_stream, false).expect("the filter face drops strays");
        assert_eq!(filtered[0].len(), 1, "only the pinned kind survives");
        assert_eq!(
            filtered[0][0],
            &"RAW_SECTION assign selected_section=0".to_string()
        );
    }

    /// The field-level localizer names the FIRST differing field with
    /// both segments; a leading field drift beats a later one, and a
    /// segment-count drift is a divergence (a dropped field cannot
    /// pass as "aligned").
    #[test]
    fn first_differing_field_names_first_field_and_drift() {
        let gold = "RAW_SECTION assign selected_section=0, add_costs=0, expansionValue=1.5, net=7";
        // Identical rows -> None.
        assert_eq!(first_differing_field(gold, gold), None);
        // Middle-field value drift.
        let rust = gold.replace("expansionValue=1.5", "expansionValue=1.75");
        let (field, g, r) = first_differing_field(gold, &rust).expect("differs");
        assert_eq!(field, "expansionValue");
        assert_eq!(g, "expansionValue=1.5");
        assert_eq!(r, "expansionValue=1.75");
        // The FIRST differing field wins (selected_section drifts AND
        // expansionValue drifts -> selected_section is reported).
        let two_drifts = gold.replace("selected_section=0", "selected_section=2");
        let (field, _, _) = first_differing_field(gold, &two_drifts).expect("differs");
        assert_eq!(field, "selected_section");
        // Segment-count drift: a missing trailing field is a divergence.
        let short = "RAW_SECTION assign selected_section=0, add_costs=0";
        let (field, g, r) = first_differing_field(gold, short).expect("differs");
        assert_eq!(field, "expansionValue");
        assert_eq!(g, "expansionValue=1.5");
        assert_eq!(r, "<missing>");
        // And an EXTRA Rust segment is a divergence with the field named
        // from the Rust side.
        let long = "RAW_SECTION assign selected_section=0, add_costs=0, expansionValue=1.5, net=7, extra=1";
        let (field, g, r) = first_differing_field(gold, long).expect("differs");
        assert_eq!(field, "extra");
        assert_eq!(g, "<missing>");
        assert_eq!(r, "extra=1");
    }

    /// The `", "` split is safe over the REAL row grammar: describe
    /// renderings carry commas WITHOUT a following space (int-box
    /// bounds, coordinate pairs) — a synthetic row in the exact Java
    /// shape splits into the same segments on both sides.
    #[test]
    fn field_split_survives_the_describe_grammar() {
        let row = "RAW_SECTION assign selected_section=0, from_section=0, backtrack_section=0, add_costs=0, adjustment=NONE, roomRipped=false, expansionValue=372420.9562723289, sortingValue=1235084.537664867, door=ExpansionDoor/bounds=[(1350,321250)..(178750,321250)]/dim=1/sections=7, door_bounds=[(1350,321250)..(178750,321250)], from_door=TargetItemExpansionDoor/item=103/tree_entry=0/dim=2/sections=1, from_door_bounds=[(128750,428750)..(171250,471250)], net=98";
        let drift = row.replace("item=103", "item=104");
        let (field, g, r) = first_differing_field(row, &drift).expect("differs");
        assert_eq!(
            field, "from_door",
            "the field name is the segment head: {g}"
        );
        assert!(g.starts_with("from_door="), "{g}");
        assert!(r.contains("item=104"), "{r}");
        // The coordinate commas inside bounds never split a segment.
        let segments: usize = row.split(", ").count();
        assert_eq!(segments, 13, "the exact Java field count");
    }

    /// The double-capture gate: two byte-identical fresh-line vectors
    /// pass; ONE differing line bails naming its index and both texts,
    /// and a LENGTH drift bails even with a zero-length diff (the
    /// zip alone would silently pass a truncated second run).
    #[test]
    fn double_capture_gate_rejects_any_line_drift_or_truncation() {
        let run1 = vec![
            "{\"fixture\":\"e1_ripup\",\"type\":\"run\",\"returned\":true}".to_string(),
            "{\"fixture\":\"e1_ripup\",\"type\":\"trace_row\",\"msg\":\"RAW_SECTION assign selected_section=0\"}"
                .to_string(),
        ];
        // Byte-identical: the gate's diff loop finds nothing (this is
        // what the capture requires before committing a golden).
        for (a, b) in run1.iter().zip(&run1) {
            assert_eq!(a, b, "identical vectors must not diff");
        }
        // One differing line (the value drift the gate must name).
        let run2 = run1.clone();
        let drifted = vec![
            run2[0].clone(),
            run2[1].replace("selected_section=0", "selected_section=1"),
        ];
        let diffs: Vec<usize> = run1
            .iter()
            .zip(&drifted)
            .enumerate()
            .filter_map(|(i, (a, b))| (a != b).then_some(i))
            .collect();
        assert_eq!(diffs, vec![1], "the gate names the first differing line");
        // A truncated second run slips past the zip — the LENGTH check
        // is load-bearing (this mutant shape actually occurred in an
        // early drc-corpus draft).
        let truncated: Vec<String> = run1[..1].to_vec();
        let zip_diffs: usize = run1.iter().zip(&truncated).filter(|(a, b)| a != b).count();
        assert_eq!(zip_diffs, 0, "the zip alone would PASS the truncation");
        assert_ne!(run1.len(), truncated.len(), "the length check catches it");
    }

    /// The golden envelope parses strictly: the four row shapes are
    /// disjoint under `deny_unknown_fields`, the Gson HTML-escaped
    /// `=` (`=`) decodes to the literal, and re-serialization is
    /// field-order stable (the canonical golden bytes).
    #[test]
    fn golden_rows_parse_strictly_and_reserialize_in_gson_order() {
        // The REAL probe bytes (Gson HTML-escapes =, ->, <, >).
        let witness_line = "{\"fixture\":\"e1_ripup\",\"type\":\"settings_witness\",\"layers\":[{\"horizontal\":\"1.0\",\"vertical\":\"2.9000000000000004\",\"bend\":\"0.0\",\"active\":true}],\"via_costs\":1,\"plane_via_costs\":1,\"vias_allowed\":true,\"automatic_neckdown\":false,\"start_ripup_costs\":40000,\"fanout_enabled\":false,\"run_router\":true,\"max_items\":null,\"unrouted_net_penalty\":\"5000000.0\",\"clearance_violation_penalty\":\"1000000.0\",\"bend_penalty\":\"10.0\",\"scoring_via_costs\":1,\"max_passes\":10}";
        let witness: GoldenRow = serde_json::from_str(witness_line).expect("witness parses");
        let GoldenRow::Witness(w) = &witness else {
            panic!("witness variant");
        };
        assert_eq!(w.fixture, "e1_ripup");
        assert_eq!(w.start_ripup_costs, 40000);
        assert_eq!(w.max_passes, 10);
        assert_eq!(w.layers[0].vertical, "2.9000000000000004");
        // Round-trip: serde field order == Gson insertion order.
        assert_eq!(
            serde_json::to_string(&witness).expect("serializes"),
            witness_line,
            "canonical bytes match the probe's field order (escaping aside)"
        );
        // The trace row with escaped `=`/`->` decodes to literal text.
        let trace_line = "{\"fixture\":\"e1_ripup\",\"type\":\"trace_row\",\"msg\":\"compare_trace_route_item Routing Pin -\\u003e result\\u003dROUTED, details\\u003d\"}";
        let trace: GoldenRow = serde_json::from_str(trace_line).expect("trace parses");
        let GoldenRow::Trace(t) = &trace else {
            panic!("trace variant");
        };
        assert_eq!(
            t.msg,
            "compare_trace_route_item Routing Pin -> result=ROUTED, details="
        );
        // Strictness: an unknown field must be rejected, not dropped.
        assert!(
            serde_json::from_str::<GoldenRow>(
                "{\"fixture\":\"x\",\"type\":\"run\",\"returned\":true,\"extra\":1}",
            )
            .is_err()
        );
        // Every variant is sealed (a permissive variant would let a
        // tap leak smuggle fields into the canonical bytes) — all
        // FOUR shapes, including the witness (review R4: the first
        // draft covered the other three and a witness unknown-field
        // drop SURVIVED the battery).
        assert!(
            serde_json::from_str::<GoldenRow>(
                "{\"fixture\":\"x\",\"type\":\"trace_row\",\"msg\":\"m\",\"extra\":1}",
            )
            .is_err()
        );
        assert!(
            serde_json::from_str::<GoldenRow>(
                "{\"fixture\":\"x\",\"type\":\"incompletes\",\"incomplete_count\":0,\"max_connections\":2,\"extra\":1}",
            )
            .is_err()
        );
        assert!(
            serde_json::from_str::<GoldenRow>(
                &witness_line.replace("\"max_passes\":10}", "\"max_passes\":10,\"extra\":1}",)
            )
            .is_err()
        );
        // A wrong-type field is rejected (run with a string boolean).
        assert!(
            serde_json::from_str::<GoldenRow>(
                "{\"fixture\":\"x\",\"type\":\"run\",\"returned\":\"yes\"}",
            )
            .is_err()
        );
    }

    /// The compare's world construction builds the batch settings FROM
    /// the witness, scalars resolved exactly (the trace costs parse as
    /// doubles, the driver scalars ride the witness, fanout-off implies
    /// removeUnconnectedVias — the job-ctor derivation the probe world
    /// uses).
    #[test]
    fn batch_settings_build_from_witness_row() {
        let witness_line = "{\"fixture\":\"e1_ripup\",\"type\":\"settings_witness\",\"layers\":[{\"horizontal\":\"1.0\",\"vertical\":\"2.9000000000000004\",\"bend\":\"0.0\",\"active\":true},{\"horizontal\":\"1.5\",\"vertical\":\"1.0\",\"bend\":\"0.0\",\"active\":true}],\"via_costs\":1,\"plane_via_costs\":1,\"vias_allowed\":true,\"automatic_neckdown\":false,\"start_ripup_costs\":40000,\"fanout_enabled\":false,\"run_router\":true,\"max_items\":null,\"unrouted_net_penalty\":\"5000000.0\",\"clearance_violation_penalty\":\"1000000.0\",\"bend_penalty\":\"10.0\",\"scoring_via_costs\":1,\"max_passes\":10}";
        let witness: SettingsWitness = serde_json::from_str(witness_line).expect("witness parses");
        let batch = batch_settings_from_witness(&witness).expect("settings build");
        assert_eq!(batch.router_settings.trace_costs.len(), 2);
        assert_eq!(
            batch.router_settings.trace_costs[0].vertical,
            2.9000000000000004
        );
        assert_eq!(batch.router_settings.trace_costs[1].horizontal, 1.5);
        assert_eq!(batch.router_settings.start_ripup_costs, 40000);
        assert_eq!(batch.max_passes, Some(10));
        assert_eq!(batch.max_items, None);
        assert!(!batch.fanout_enabled, "the probe world is fanout-off");
        assert!(
            batch.remove_unconnected_vias,
            "removeUnconnectedVias = !fanout (the job-ctor derivation)"
        );
        assert_eq!(batch.via_costs, 1);
        assert_eq!(batch.plane_via_costs, 1);
        assert_eq!(batch.pull_tight_accuracy, 500);
        let scoring = batch.scoring.scoring.as_ref().expect("witness penalties");
        assert_eq!(scoring.unrouted_net_penalty, Some(5_000_000.0));
        assert_eq!(scoring.clearance_violation_penalty, Some(1_000_000.0));
        assert_eq!(scoring.bend_penalty, Some(10.0));
        assert_eq!(scoring.via_costs, Some(1));
        assert!(batch.router_settings.layer_active.iter().all(|&a| a));
    }

    /// The capture's fixture-contiguity sanity: a well-formed stream
    /// (witness → traces → run → incompletes per fixture) passes; a
    /// fixture RE-OPENING after another fixture fails (the stream is
    /// world-serialized — interleaving means machinery drift).
    #[test]
    fn sanity_check_rejects_interleaved_fixtures() {
        let mk_trace = |fixture: &str, msg: &str| {
            GoldenRow::Trace(TraceRow {
                fixture: fixture.to_string(),
                kind: "trace_row".to_string(),
                msg: msg.to_string(),
            })
        };
        let mk_witness = |fixture: &str| {
            GoldenRow::Witness(SettingsWitness {
                fixture: fixture.to_string(),
                kind: "settings_witness".to_string(),
                layers: vec![],
                via_costs: 1,
                plane_via_costs: 1,
                vias_allowed: true,
                automatic_neckdown: false,
                start_ripup_costs: 1,
                fanout_enabled: false,
                run_router: true,
                max_items: None,
                unrouted_net_penalty: "5000000.0".to_string(),
                clearance_violation_penalty: "1000000.0".to_string(),
                bend_penalty: "10.0".to_string(),
                scoring_via_costs: 1,
                max_passes: 10,
            })
        };
        let straight = vec![
            mk_witness("a"),
            mk_trace("a", "RAW_SECTION assign x"),
            mk_witness("b"),
            mk_trace("b", "RAW_SECTION assign y"),
        ];
        assert!(sanity_check(&straight).is_ok(), "straight worlds pass");
        let interleaved = vec![
            mk_witness("a"),
            mk_trace("a", "RAW_SECTION assign x"),
            mk_witness("b"),
            mk_trace("a", "RAW_SECTION assign x2"),
        ];
        assert!(
            sanity_check(&interleaved).is_err(),
            "fixture a re-opening after b must fail"
        );
        // A wrong type tag inside a variant is rejected too.
        let wrong_tag = vec![GoldenRow::Run(RunRow {
            fixture: "a".to_string(),
            kind: "NOT_A_RUN".to_string(),
            returned: true,
        })];
        assert!(sanity_check(&wrong_tag).is_err());
    }

    // -----------------------------------------------------------------------
    // Corpus-backed pins (they read the COMMITTED golden + run the real
    // Rust worlds — the ~0.7s cost is the corpus being load-bearing)
    // -----------------------------------------------------------------------

    /// The committed golden, load-bearing: exact row count, per-fixture
    /// kind census, and LITERAL captured rows (the real Java text — the
    /// shapes any normalization bug would silently reshape).
    #[test]
    fn golden_corpus_literal_rows_and_census() {
        let golden_path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus/events-golden.jsonl");
        let raw = std::fs::read_to_string(&golden_path).expect("committed events golden");
        let records: Vec<GoldenRow> = raw
            .lines()
            .map(|line| serde_json::from_str(line).expect("every golden line parses"))
            .collect();
        assert_eq!(records.len(), 3529, "the committed capture size");

        // Per-fixture kind census + the run/incompletes witnesses.
        let census = |fixture: &str| -> [usize; 4] {
            let mut counts = [0usize; 4];
            for record in &records {
                if let GoldenRow::Trace(t) = record
                    && t.fixture == fixture
                    && let Some(slot) = EVENT_KINDS.iter().position(|kind| t.msg.starts_with(kind))
                {
                    counts[slot] += 1;
                }
            }
            counts
        };
        assert_eq!(
            census("e1_ripup"),
            [331, 1, 1, 4],
            "e1: assign/skip/ripped/route"
        );
        assert_eq!(
            census("t7_ripup"),
            [496, 4, 0, 7],
            "t7: assign/skip/ripped/route"
        );
        assert_eq!(
            census("t9_locator45"),
            [2671, 1, 0, 4],
            "t9: assign/skip/ripped/route"
        );
        for (fixture, costs) in [("e1_ripup", 40000), ("t7_ripup", 1), ("t9_locator45", 1)] {
            let run = records.iter().find_map(|r| match r {
                GoldenRow::Run(row) if row.fixture == fixture => Some(row),
                _ => None,
            });
            assert_eq!(
                run.map(|row| row.returned),
                Some(true),
                "{fixture} completed"
            );
            let incompletes = records.iter().find_map(|r| match r {
                GoldenRow::Incompletes(row) if row.fixture == fixture => Some(row),
                _ => None,
            });
            assert_eq!(
                incompletes.map(|row| (row.incomplete_count, row.max_connections)),
                Some((0, 2)),
                "{fixture} fully routed against the 2-connection lower bound"
            );
            let witness = records.iter().find_map(|r| match r {
                GoldenRow::Witness(w) if w.fixture == fixture => Some(w),
                _ => None,
            });
            assert_eq!(
                witness.map(|w| w.start_ripup_costs),
                Some(costs),
                "{fixture} tuned scalar"
            );
        }

        // LITERAL captured rows (the divergence witnesses of the triage):
        // e1's first-diverging assign (the door-partition split), e1's
        // only ripped row (the forced-ripup face), t7's only golden
        // skip, and t9's first route row (the maxItemId face).
        let msg = |fixture: &str, kind_slot: usize, ordinal0: usize| -> String {
            records
                .iter()
                .filter_map(|r| match r {
                    GoldenRow::Trace(t) if t.fixture == fixture => Some(&t.msg),
                    _ => None,
                })
                .filter(|m| {
                    EVENT_KINDS
                        .iter()
                        .position(|kind| m.starts_with(kind))
                        .is_some_and(|slot| slot == kind_slot)
                })
                .nth(ordinal0)
                .cloned()
                .unwrap_or_else(|| format!("<no such row: {fixture} slot {kind_slot} #{ordinal0}>"))
        };
        assert_eq!(
            msg("e1_ripup", 0, 57),
            "RAW_SECTION assign selected_section=0, from_section=0, backtrack_section=0, \
             add_costs=0, adjustment=NONE, roomRipped=false, expansionValue=469613.0463275057, \
             sortingValue=834357.6215524157, door=ExpansionDoor/bounds=[(216250,238750)..\
             (983750,238750)]/dim=1/sections=7, door_bounds=[(216250,238750)..(983750,238750)], \
             from_door=TargetItemExpansionDoor/item=7/tree_entry=0/dim=2/sections=1, \
             from_door_bounds=[(583750,103750)..(616250,136250)], net=2",
            "e1 assign #58 — the golden's door spans the full wall in 7 sections"
        );
        assert_eq!(
            msg("e1_ripup", 2, 0),
            "compare_trace_ripped_item source_item=6, source_net=1, ripped_id=9, \
             ripped_type=PolylineTrace, ripped_net_count=1, ripped_nets=2, \
             ripupCost=21474836",
            "e1's only ripped row — the corpus's sole forced-ripup harvest face"
        );
        assert_eq!(
            msg("t7_ripup", 1, 0),
            "RAW_SECTION skip selected_section=0, from_section=0, backtrack_section=0, \
             occupied=true, shape_entry_null=false, adjustment=NONE, \
             door=TargetItemExpansionDoor/item=14/tree_entry=1/dim=2/sections=1, \
             door_bounds=[(536750,298750)..(587250,321250)], \
             from_door=TargetItemExpansionDoor/item=14/tree_entry=0/dim=2/sections=1, \
             from_door_bounds=[(536750,282750)..(559250,321250)], net=2",
            "t7's single golden skip row"
        );
        assert_eq!(
            msg("t9_locator45", 3, 0),
            "compare_trace_route_item Routing Pin -> result=ROUTED, details=, incompletes=1, \
             netIncomplete=0, ripped=0, netItems=2->9, maxItemId=130",
            "t9's first route row — the Java insert-path id face"
        );
    }

    /// The executable triage record: the Rust stream aligns with the
    /// golden through each fixture's known prefix, diverges at the
    /// EXACT triaged ordinal with the EXACT first differing field, and
    /// the two t7 witnesses prove the divergence classes:
    /// * `G3 == R4` byte-identical — the maze cost arithmetic is exact
    ///   across engines for the same door.
    /// * `G5 == R3` after `item=10 → item=13` — the same door carries a
    ///   DIFFERENT parse-time item id (Class A: id assignment drift).
    ///
    /// Any engine change that moves these ordinals must update the
    /// triage in logs/M3-T16/report.md, not silently shift this pin.
    #[test]
    fn rust_stream_aligns_through_triaged_divergences() {
        let golden_path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus/events-golden.jsonl");
        let raw = std::fs::read_to_string(&golden_path).expect("committed events golden");
        let records: Vec<GoldenRow> = raw
            .lines()
            .map(|line| serde_json::from_str(line).expect("golden row parses"))
            .collect();
        let manifest: Vec<EventsManifestEntry> = load_jsonl(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus/events-manifest.jsonl"),
            "manifest",
        )
        .expect("manifest");
        let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("repo root");

        let groups = |fixture: &str| -> [Vec<String>; 4] {
            let mut grouped: [Vec<String>; 4] = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
            for record in &records {
                if let GoldenRow::Trace(t) = record
                    && t.fixture == fixture
                    && let Some(slot) = EVENT_KINDS.iter().position(|k| t.msg.starts_with(k))
                {
                    grouped[slot].push(t.msg.clone());
                }
            }
            grouped
        };
        let witness = |fixture: &str| -> SettingsWitness {
            records
                .iter()
                .find_map(|r| match r {
                    GoldenRow::Witness(w) if w.fixture == fixture => Some(w.clone()),
                    _ => None,
                })
                .expect("witness row")
        };

        for entry in &manifest {
            let fixture = entry
                .path
                .rsplit('/')
                .next()
                .and_then(|stem| stem.strip_suffix(".dsn"))
                .expect("fixture stem");
            let world = run_rust_world(&repo_root.join(&entry.path), &witness(fixture))
                .unwrap_or_else(|err| panic!("Rust world {fixture}: {err:#}"));
            let golden_grouped = groups(fixture);
            let mut rust_grouped: [Vec<String>; 4] =
                [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
            for (level, msg) in &world.rows {
                if let Some(slot) = EVENT_KINDS.iter().position(|kind| msg.starts_with(kind)) {
                    assert_eq!(*level, "trace", "pinned rows ride the trace level");
                    rust_grouped[slot].push(msg.clone());
                }
            }

            // The triaged first divergence per fixture: (kind slot,
            // 0-based diverging index, aligned prefix length, expected
            // first differing field + the segment-value and full-row
            // geometry witnesses).
            let (slot, index, prefix, g_seg_has, r_seg_has, g_row_has, r_row_has, field) =
                match fixture {
                    // Door-partition divergence (Class B): Java slices
                    // the keepout wall 216250..983750 into 7 sections,
                    // Rust 595340..983750 into 4.
                    "e1_ripup" => (
                        0,
                        57,
                        57,
                        vec!["expansionValue=469613.0463275057"],
                        vec!["expansionValue=348328.28723786207"],
                        vec![
                            "door=ExpansionDoor/bounds=[(216250,238750)..(983750,238750)]/dim=1/sections=7",
                        ],
                        vec![
                            "door=ExpansionDoor/bounds=[(595340,238750)..(983750,238750)]/dim=1/sections=4",
                        ],
                        "expansionValue",
                    ),
                    // Id-drift + enumeration divergence (Class A): Rust
                    // seeds door item=10 (== Java's item=13 geometry)
                    // first.
                    "t7_ripup" => (
                        0,
                        2,
                        2,
                        vec!["expansionValue=139200.00000000003"],
                        vec!["expansionValue=139805.7223435436"],
                        vec!["door=TargetItemExpansionDoor/item=11/"],
                        vec!["door=TargetItemExpansionDoor/item=10/"],
                        "expansionValue",
                    ),
                    // Wall-door boundary divergence (Class B): the same
                    // ExpansionDrill origin, different wall door picked.
                    "t9_locator45" => (
                        0,
                        1349,
                        1349,
                        vec!["expansionValue=379686.89149088226"],
                        vec!["expansionValue=322104.85468532494"],
                        vec!["door=ExpansionDoor/bounds=[(475440,1350)..(475440,428938)]"],
                        vec!["door=ExpansionDoor/bounds=[(313495,1350)..(313495,429531)]"],
                        "expansionValue",
                    ),
                    other => panic!("unmapped fixture {other}"),
                };

            // The aligned prefix is byte-identical row for row.
            for i in 0..prefix {
                assert_eq!(
                    golden_grouped[slot][i], rust_grouped[slot][i],
                    "{fixture} assign prefix must align through ordinal {prefix} (0-based {i})"
                );
            }
            // The diverging row differs, in the triaged field, with the
            // triaged values on both sides (segment-level witnesses on
            // the differing segment, geometry witnesses on the full
            // rows).
            let g = &golden_grouped[slot][index];
            let r = &rust_grouped[slot][index];
            assert_ne!(g, r, "{fixture} must diverge at 0-based {index}");
            let (name, g_seg, r_seg) =
                first_differing_field(g, r).expect("field-level diff exists");
            assert_eq!(name, field, "{fixture} triaged first differing field");
            for needle in &g_seg_has {
                assert!(
                    g_seg.contains(needle),
                    "golden segment {g_seg} lacks {needle}"
                );
            }
            for needle in &r_seg_has {
                assert!(
                    r_seg.contains(needle),
                    "rust segment {r_seg} lacks {needle}"
                );
            }
            for needle in &g_row_has {
                assert!(g.contains(needle), "golden row lacks {needle}: {g}");
            }
            for needle in &r_row_has {
                assert!(r.contains(needle), "rust row lacks {needle}: {r}");
            }

            // The t7 witnesses (see the doc comment).
            if fixture == "t7_ripup" {
                assert_eq!(
                    golden_grouped[0][2], rust_grouped[0][3],
                    "G3 == R4 byte-identical: same door, same label, EXACT cost arithmetic"
                );
                assert_eq!(
                    rust_grouped[0][2].replace(
                        "door=TargetItemExpansionDoor/item=10/",
                        "door=TargetItemExpansionDoor/item=13/",
                    ),
                    golden_grouped[0][4],
                    "G5 == R3 modulo the id label: Class A parse-time id drift"
                );
            }

            // The run/incompletes faces still agree everywhere.
            assert!(world.returned, "{fixture} returned");
            assert_eq!(
                (world.incomplete_count, world.max_connections),
                (0, 2),
                "{fixture} incompletes"
            );
        }
    }

    /// The Class-C face (buglog 174), pinned at the level that is TRUE
    /// today: the route-row GRAMMAR. The live id values (e.g. e1
    /// maxItemId 62 vs 390, t9 130 vs 122) ARE the divergence and rest
    /// on buglog-174 manual runs until Classes A/B close — they are
    /// deliberately NOT pinned here (they move with the fix). Pinned
    /// instead, via the real [`field_name_of_segment`] the diff
    /// localizer uses:
    /// (a) the field SKELETON (names + order) of every ALIGNED route
    ///     row is identical on both sides;
    /// (b) through a per-fixture STRICT prefix, stripping
    ///     `netItems=`/`maxItemId=` leaves the rows byte-equal (id
    ///     churn, nothing else). e1/t9: all rows. t7: rows 0-4 — its
    ///     attempt-structure churn ALSO shifts the running
    ///     `incompletes=`/`netIncomplete=` snapshots from row 5 on
    ///     (the pin caught its author's first-draft "id-only
    ///     everywhere" overclaim on exactly that row);
    /// (c) the per-fixture count face: e1 4==4 and t9 4==4 (aligned
    ///     slot counts agree), while t7's 7-vs-8 drift is the triaged
    ///     churn witness it is (Rust makes one extra attempt).
    ///
    /// A rendering drift (renamed, reordered, or dropped field) breaks
    /// (a); a non-id value drift breaks (b); a Class-C fix that greens
    /// the values collapses the difference faces and moves the t7
    /// count — update report + SEAM triage with it, never silently.
    #[test]
    fn route_rows_share_the_field_skeleton_and_diverge_only_in_id_values() {
        let golden_path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus/events-golden.jsonl");
        let raw = std::fs::read_to_string(&golden_path).expect("committed events golden");
        let records: Vec<GoldenRow> = raw
            .lines()
            .map(|line| serde_json::from_str(line).expect("golden row parses"))
            .collect();
        let manifest: Vec<EventsManifestEntry> = load_jsonl(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus/events-manifest.jsonl"),
            "manifest",
        )
        .expect("manifest");
        let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("repo root");

        let id_family_segment =
            |segment: &str| segment.starts_with("netItems=") || segment.starts_with("maxItemId=");
        let strip_id_family = |row: &str| {
            row.split(", ")
                .filter(|segment| !id_family_segment(segment))
                .collect::<Vec<_>>()
                .join(", ")
        };

        for entry in &manifest {
            let fixture = entry
                .path
                .rsplit('/')
                .next()
                .and_then(|stem| stem.strip_suffix(".dsn"))
                .expect("fixture stem");
            let witness = records
                .iter()
                .find_map(|r| match r {
                    GoldenRow::Witness(w) if w.fixture == fixture => Some(w.clone()),
                    _ => None,
                })
                .expect("witness row");
            let world = run_rust_world(&repo_root.join(&entry.path), &witness)
                .unwrap_or_else(|err| panic!("Rust world {fixture}: {err:#}"));
            let golden_routes: Vec<&String> = records
                .iter()
                .filter_map(|r| match r {
                    GoldenRow::Trace(t) if t.fixture == fixture => Some(&t.msg),
                    _ => None,
                })
                .filter(|msg| msg.starts_with("compare_trace_route_item"))
                .collect();
            let rust_routes: Vec<&String> = world
                .rows
                .iter()
                .map(|(_, msg)| msg)
                .filter(|msg| msg.starts_with("compare_trace_route_item"))
                .collect();

            // (c) the count face: equal on e1/t9, triaged drift on t7.
            match fixture {
                "e1_ripup" | "t9_locator45" => assert_eq!(
                    golden_routes.len(),
                    rust_routes.len(),
                    "{fixture} route-slot counts must agree"
                ),
                "t7_ripup" => assert_eq!(
                    (golden_routes.len(), rust_routes.len()),
                    (7, 8),
                    "{fixture} triaged route-slot count drift (the extra Rust attempt)"
                ),
                other => panic!("unmapped fixture {other}"),
            }

            // (a) the skeleton face holds on EVERY aligned row.
            for (i, (g, r)) in golden_routes.iter().zip(&rust_routes).enumerate() {
                let g_skeleton: Vec<String> = g.split(", ").map(field_name_of_segment).collect();
                let r_skeleton: Vec<String> = r.split(", ").map(field_name_of_segment).collect();
                assert_eq!(
                    g_skeleton, r_skeleton,
                    "{fixture} route row {i}: the field skeleton must be identical"
                );
            }
            // (b) outside the id-family segments the rows are
            // byte-equal — id churn, nothing else. TRUE through a
            // per-fixture STRICT prefix: on t7 the attempt-structure
            // churn (the 7-vs-8 count drift) ALSO shifts the running
            // `incompletes=`/`netIncomplete=` snapshots from row 5 on
            // (attempt-sequence faces, same churn class — the pin
            // caught its author's first-draft "id-only everywhere"
            // overclaim here); on e1/t9 the strict prefix is all rows.
            let strict_prefix = match fixture {
                "e1_ripup" | "t9_locator45" => golden_routes.len(),
                "t7_ripup" => 5,
                other => panic!("unmapped fixture {other}"),
            };
            for (i, (g, r)) in golden_routes
                .iter()
                .zip(&rust_routes)
                .take(strict_prefix)
                .enumerate()
            {
                assert_eq!(
                    strip_id_family(g),
                    strip_id_family(r),
                    "{fixture} route row {i}: non-id faces must be byte-equal"
                );
            }

            // The C face is real: at least one aligned row carries an
            // id-value divergence (e1's first is row 2 — its rows 0-1
            // are byte-identical; t7/t9 diverge from row 0).
            let probe_index = if fixture == "e1_ripup" { 2 } else { 0 };
            assert_ne!(
                golden_routes[probe_index], rust_routes[probe_index],
                "{fixture} route row {probe_index}: the id churn must be live"
            );
        }
    }

    /// The silent-sink gating face (Java parity, exactly as the probe
    /// world sees it): with the trace backend OFF (the headless
    /// production face — `NullDriverSink`, `isTraceEnabled() == false`),
    /// the GATED per-item comparison row (`compare_trace_route_item`,
    /// Java `AutoroutePassRunner.java:251-252`) must NOT be emitted,
    /// while the rows Java builds UNCONDITIONALLY at the call site
    /// still arrive at `trace`: the `RAW_SECTION` rows
    /// (`MazeSearchEngine.java:798-821/:907-933` — one-arg trace, the
    /// backend filters) and `logRippedItems` (`:250` — called ungated,
    /// BEFORE the `isTraceEnabled()` gate). A mutant that gates
    /// `emit_raw_row` drops the RAW_SECTION count to 0; a mutant that
    /// un-gates the route comparison makes it appear here; either dies.
    #[test]
    fn trace_disabled_sink_gates_compare_rows_but_raw_rows_still_flow() {
        use epic_router::pipeline::event_sink::{CaptureDriverSink, DriverSink};
        use epic_router::pipeline::pass_runner::RouterCounters;

        struct TraceDisabledSink {
            inner: CaptureDriverSink,
        }
        impl DriverSink for TraceDisabledSink {
            fn is_trace_enabled(&self) -> bool {
                false
            }
            fn info(&mut self, message: &str) {
                self.inner.info(message);
            }
            fn warn(&mut self, message: &str) {
                self.inner.warn(message);
            }
            fn debug(&mut self, message: &str) {
                self.inner.debug(message);
            }
            fn trace(&mut self, message: &str) {
                self.inner.trace(message);
            }
            fn task_state(&mut self, state: &str, pass: i32, hash: &str) {
                self.inner.task_state(state, pass, hash);
            }
            fn board_updated(&mut self, counters: &RouterCounters) {
                self.inner.board_updated(counters);
            }
        }

        let golden_path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus/events-golden.jsonl");
        let witness: SettingsWitness = serde_json::from_str(
            std::fs::read_to_string(&golden_path)
                .expect("committed events golden")
                .lines()
                .next()
                .expect("the golden opens with e1's witness"),
        )
        .expect("witness parses");
        let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("repo root");
        let dsn = repo_root.join("rust/harness/fixtures/event-stream/e1_ripup.dsn");

        let mut sink = TraceDisabledSink {
            inner: CaptureDriverSink::default(),
        };
        let (returned, incomplete_count, max_connections) =
            run_world_with_sink(&dsn, &witness, &mut sink).expect("trace-disabled world runs");
        assert!(returned, "gating is log-only: the run still completes");
        assert_eq!((incomplete_count, max_connections), (0, 2));

        let trace_rows: Vec<&String> = sink
            .inner
            .rows
            .iter()
            .filter(|(level, _)| *level == "trace")
            .map(|(_, msg)| msg)
            .collect();
        assert!(
            trace_rows
                .iter()
                .all(|msg| !msg.starts_with("compare_trace_route_item")),
            "the GATED route comparison row must NOT reach a trace-disabled sink \
             (Java AutoroutePassRunner.java:251-252)"
        );
        let assigns = trace_rows
            .iter()
            .filter(|msg| msg.starts_with("RAW_SECTION assign"))
            .count();
        assert_eq!(
            assigns, 395,
            "RAW_SECTION rows flow unconditionally (the e1 Rust census; the \
             backend filters, not the call site)"
        );
        let ripped = trace_rows
            .iter()
            .filter(|msg| msg.starts_with("compare_trace_ripped_item"))
            .count();
        assert_eq!(
            ripped, 1,
            "logRippedItems is called UNGATED (Java :250) — the e1 harvested \
             seed row still flows (matching the golden census)"
        );
    }

    /// The alignment core catches a dropped Rust row at the DROP
    /// ordinal (not silently zipped away), an extra row, and a value
    /// drift with the FIRST DIFFERING FIELD named — through the real
    /// `diff_kind_streams` the compare calls.
    #[test]
    fn diff_kind_streams_catches_drop_extra_and_field_drift() {
        let a = "RAW_SECTION assign selected_section=0, expansionValue=1.5".to_string();
        let b = "RAW_SECTION assign selected_section=1, expansionValue=1.5".to_string();
        let c = "RAW_SECTION assign selected_section=2, expansionValue=2.5".to_string();
        let d = "RAW_SECTION assign selected_section=3, expansionValue=2.5".to_string();
        let gold = vec![&a, &b, &c];

        // Identical streams align.
        diff_kind_streams("f", EVENT_KINDS[0], &gold, &gold).expect("identical streams");

        // A dropped TRAILING Rust row is caught as the missing ordinal
        // (a zip-only implementation would silently pass the shorter
        // stream: gold[0..2] and rust[0..2] are equal).
        let short = vec![&a, &b];
        let err = diff_kind_streams("f", EVENT_KINDS[0], &gold, &short)
            .expect_err("dropped row must fail");
        assert!(
            err.to_string().contains("first missing Rust ordinal=3"),
            "names the drop point: {err}"
        );

        // A dropped MIDDLE Rust row cannot slip through either: the
        // shifted tail lands on a differing row and the DRIFT arm
        // fires at the drop ordinal (2 = where b went missing).
        let middle_dropped = vec![&a, &c];
        let err = diff_kind_streams("f", EVENT_KINDS[0], &gold, &middle_dropped)
            .expect_err("a middle drop must fail somewhere");
        assert!(
            err.to_string().contains("ordinal=2"),
            "the drop ordinal fires: {err}"
        );

        // An extra Rust row is caught with its ordinal.
        let long = vec![&a, &b, &c, &d];
        let err =
            diff_kind_streams("f", EVENT_KINDS[0], &gold, &long).expect_err("extra row must fail");
        assert!(
            err.to_string().contains("first extra Rust ordinal=4"),
            "names the extra row: {err}"
        );

        // A value drift names the FIRST differing field (here
        // selected_section, though expansionValue drifts too).
        let drifted = vec![&a, &c, &c];
        let err = diff_kind_streams("f", EVENT_KINDS[0], &gold, &drifted)
            .expect_err("value drift must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("FIRST DIFFERING FIELD: selected_section"),
            "first field wins: {msg}"
        );
        assert!(
            msg.contains("kind=RAW_SECTION assign"),
            "names the kind: {msg}"
        );
    }
}
