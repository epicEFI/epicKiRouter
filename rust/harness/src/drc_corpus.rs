//! DRC parity corpus (M3 Task 2): the two counts every M3 quality gate
//! consumes — incomplete connections and clearance violations — taken
//! at PARSE time, parity-pinned against the frozen Java engine through
//! `rust/harness/oracle/DrcOracle.java`, with committed JSONL goldens
//! and a java-free comparison.
//!
//! ## The protocol (BOTH sides run it identically — the Java oracle
//! `DrcOracle.java` and [`evaluate_rust`])
//!
//! Per fixture:
//!
//! 1. Parse to the live board (`DsnReader.readBoard` /
//!    `epic_dsn::read_board` + `Board::from_ses_board`).
//! 2. `reinsertTreeItems()` — the uniform fill normalization (the
//!    index-corpus discipline: Java's read fills the search tree
//!    ascending, the port's rebuild path is descending; both sides
//!    populate the DEFAULT tree through the shared public reinsert so
//!    the DRC queries below run over identically-filled trees). No
//!    other pre-steps: the DRC corpus is a pure parse-time surface.
//! 3. `DesignRulesChecker(board, null)` + `calculateAllIncompletes()`:
//!    `incomplete_count` = `getIncompleteCount()` (the lazy/eager
//!    distinction is immaterial here — the oracle calls the
//!    calculator first and the port computes eagerly, both pinned to
//!    the same numbers), `max_connections` = the public
//!    `maxConnections` field — the ENDPOINT CODE formula
//!    `Σ per net max(0, endpoints(Pin|ConductionArea) − 1)` over nets
//!    with ≥1 connectable item. The FRLogger trace string "(formula:
//!    total_items - netCount)" at DesignRulesChecker.java:593 is
//!    STALE — the code is endpoint-based; the port implements the
//!    code (DesignRulesChecker.java:570-581).
//! 4. `getAllClearanceViolations()` (DesignRulesChecker.java:56-87):
//!    walk all items, per item `clearanceViolations()`
//!    (Item.java:367-495), dedup A-B vs B-A by Java's
//!    `sorted(id1,id2) + "-" + sorted(id1,id2) + "-" + layer` string
//!    key keeping the FIRST occurrence. Emitted rows are
//!    `{a, b, layer}` with a=min(id1,id2), b=max — the dedup makes
//!    direction irrelevant, the sort by (a, b, layer) canonicalizes.
//! 5. Per-net rows (SCHEMA v2): the RAW per-net item lists rebuilt with
//!    `calculateAllIncompletes`' own itemList loop (Connectable items
//!    appended per net number — multi-net items appear in EACH net's
//!    list), then one fresh PUBLIC `NetIncompletes(netNo, raw, board)`
//!    per net with a non-empty raw list. A row is
//!    `{net_no, items, groups, incomplete_count, ratsnest, edges}`
//!    where `items` is the RAW list size (before NetIncompletes'
//!    internal filter), `groups` is `getConnectedGroupCount()` — the
//!    count of unique connected sets over the FILTERED items — and
//!    `incomplete_count` is `count()`. v2 adds the Delaunay surface
//!    the count depends on: `ratsnest` is `{id, n}` per GROUPED
//!    (filtered) item — the Delaunay input objects with
//!    `getRatsnestCorners().length` corners, sorted by id — and
//!    `edges` is the canonical (min-id, max-id) pair list of ALL
//!    triangulation ResultEdges (degenerate zero-length
//!    coincident-corner edges included, exact duplicates collapsed —
//!    mirroring the `TreeSet<Edge>` the pairs feed — sorted
//!    lexicographically). Rows sorted by net_no ascending; nets
//!    without items are omitted.
//!
//! ## The equivalence claim: FALSIFIED, hence schema v2
//!
//! The M3-T2 brief's structural claim — the airline count is simply
//! `max(0, connected_groups − 1)` — was FALSIFIED by the spike
//! capture on drc-0013 (655_testboard: nets 3/4/17 count 3/2/2 against
//! groups−1 4/3/3). Mechanism (witness in
//! `logs/M3-T2/equivalence-witness.txt`): `Trace.getRatsnestCorners()`
//! emits only UNCONTACTED stub endpoints, so a both-ends-contacted
//! trace contributes ZERO Delaunay corners (`ratsnest[*].n == 0` rows
//! pin it), the triangulation graph is disconnected across group
//! boundaries, and Kruskal finishes with fewer merges than groups−1.
//! The count is therefore EDGE-SET-DEPENDENT, and the goldens pin the
//! edge set (not just its Kruskal yield): the Rust port implements the
//! FULL filter → grouping → Delaunay → sorted-Edge Kruskal semantics
//! and `drc compare` requires record equality on every fixture,
//! ratsnest counts and edge pairs included.
//!
//! ## SCHEMA v2 (supersedes the v1 goldens)
//!
//! v1 rows (`{net_no, items, groups, incomplete_count}`) are
//! superseded by the v2 rows above; the committed
//! `harness/corpus/drc-golden.jsonl` is a v2 capture. There is no
//! header line in the JSONL corpus files (every line is a strict
//! record), so this module doc and the DrcOracle.java header are the
//! schema version record of the corpus.
//!
//! ## Golden discipline
//!
//! Goldens live at `harness/corpus/drc-golden.jsonl` (committed;
//! regenerate ONLY via `drc golden`). `drc compare` re-evaluates every
//! fixture with the Rust port and diffs field-for-field, reporting the
//! first divergence per fixture — it never needs the JVM (the CI gate
//! runs it with `EPIC_SKIP_GRADLE=1`).
//!
//! ## Deviations (documented, parity-neutral)
//!
//! - `ClearanceViolation.expectedClearance/actualClearance` are NOT
//!   computed by the port: `actualClearance` comes from a 16-iteration
//!   binary search (Item.java:497-519) that is LABEL-ONLY for the T2
//!   surface — the violation GATE is the
//!   `enlarged1 ∩ enlarged2 dimension == 2` test, and the emitted
//!   rows carry (a, b, layer) only.
//! - The port computes `incomplete_count` EAGERLY (Java's
//!   `getIncompleteCount` lazily calls `calculateAllIncompletes` on
//!   first use — same numbers, pinned by the corpus).

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use epic_board::board::Board;
use epic_board::tree_manager::SearchTreeManager;
use epic_dsn::reader::{DsnReadResult, read_board};
use epic_dsn::ses_board::SesBoard;
use serde::{Deserialize, Serialize};

// The shared corpus shell (M3 Task 1): the manifest row, JSONL
// loading, alignment, diff rendering live in corpus_common; this
// module keeps the drc golden record and the protocol.
pub use crate::corpus_common::ManifestEntry;
use crate::corpus_common::{ensure_alignment, json_string, load_jsonl, manifest_bytes, truncate};
use crate::tiers::TierFile;

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum DrcCommand {
    /// Regenerate the drc-parity manifest (tier A fixtures in
    /// tiers.yaml order, then the three fixed PCBench pre-routed
    /// boards) and write it (deterministic byte-for-byte).
    Manifest {
        /// Output path for the manifest (relative paths resolve
        /// against the repo root's `rust/` checkout).
        #[arg(long, default_value = "harness/corpus/drc-manifest.jsonl")]
        out: PathBuf,
    },
    /// Evaluate the manifest with the Java DrcOracle (ONE JVM per
    /// run, index-corpus discipline) and write the goldens.
    Golden {
        #[arg(long, default_value = "harness/corpus/drc-manifest.jsonl")]
        manifest: PathBuf,
        #[arg(long, default_value = "harness/corpus/drc-golden.jsonl")]
        out: PathBuf,
    },
    /// Replay every manifest fixture with the Rust port and diff the
    /// record field-for-field against the committed golden
    /// (java-free, CI-able). Prints every differing field for at most
    /// 20 divergent fixtures in full; exits 1 on any.
    Compare {
        #[arg(long, default_value = "harness/corpus/drc-manifest.jsonl")]
        manifest: PathBuf,
        #[arg(long, default_value = "harness/corpus/drc-golden.jsonl")]
        golden: PathBuf,
    },
}

pub fn run(cmd: DrcCommand, jvm_xmx: &str) -> Result<()> {
    let repo_root = crate::oracle::find_repo_root()?;
    match cmd {
        DrcCommand::Manifest { out } => manifest(&repo_root, &out),
        DrcCommand::Golden { manifest, out } => golden(&repo_root, &manifest, &out, jvm_xmx),
        DrcCommand::Compare { manifest, golden } => compare(&repo_root, &manifest, &golden),
    }
}

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

/// The three fixed PCBench PRE-ROUTED boards appended after tier A:
/// parse-time boards carry routed traces, so the clearance-violation
/// walk has real surface (parse-time tier boards have incompletes but
/// no routed geometry to violate). Repo-relative posix paths; the
/// smallest three reference-routed boards of the PCBench set.
pub const PCBENCH_PRE_ROUTED: [&str; 3] = [
    "scripts/benchmark/fixtures/PCBench/AS5043-Encoder_sensor-board/reference-routed.dsn",
    "scripts/benchmark/fixtures/PCBench/655_testboard/reference-routed.dsn",
    "scripts/benchmark/fixtures/PCBench/1Bitsy_1bitsy/reference-routed.dsn",
];

/// The crafted pin-fixture boards of `epic-drc`'s unit pins (same
/// bytes via `include_str!`), appended after the PCBench trio:
/// capture-backed parity for the scenarios the real boards do not
/// cover — the NQ quad's exact Delaunay diagonal choice, CA corner
/// clusters (self-pair edges), the via-corner contrast
/// (count == groups − 1 over zero-corner traces), the tail/early-exit
/// shapes, and the tie-pin exemption plus its contrast. The
/// count < groups − 1 witness is pinned on the real 655_testboard
/// fixture (see the `testboard_655_witness_rows_literal` pin).
pub const CRAFT_FIXTURES: [&str; 3] = [
    "rust/harness/corpus/craft/drc-main.dsn",
    "rust/harness/corpus/craft/drc-tie.dsn",
    "rust/harness/corpus/craft/drc-tie-contrast.dsn",
];

/// Builds the manifest entries (pure function of the repository tree):
/// the tier A fixtures in tiers.yaml order, then the three PCBench
/// pre-routed boards, dedup by path. Ids are `drc-NNNN` in that order.
pub fn build_manifest(repo_root: &Path) -> Result<Vec<ManifestEntry>> {
    let tiers_path = repo_root.join("rust/harness/config/tiers.yaml");
    let tier_file = TierFile::load(&tiers_path)?;
    let mut seen = std::collections::BTreeSet::new();
    let mut rel = Vec::new();
    // Tier A only: the M3 gates consume tier A; the parse-time digest
    // corpus already pins every fixture's parse surface board-wide.
    for tier in tier_file.tiers.iter().filter(|tier| tier.name == "A") {
        for fixture in &tier.fixtures {
            let path = format!("{}/{}", tier_file.fixtures_root.display(), fixture.path);
            if seen.insert(path.clone()) {
                rel.push(path);
            }
        }
    }
    for path in PCBENCH_PRE_ROUTED {
        if seen.insert(path.to_string()) {
            rel.push(path.to_string());
        }
    }
    for path in CRAFT_FIXTURES {
        if seen.insert(path.to_string()) {
            rel.push(path.to_string());
        }
    }
    Ok(rel
        .into_iter()
        .enumerate()
        .map(|(index, path)| ManifestEntry {
            id: format!("drc-{:04}", index + 1),
            path,
        })
        .collect())
}

/// Serializes the manifest exactly as committed: one compact JSON
/// object per line, `\n`-terminated (including the last).
pub fn build_manifest_bytes(repo_root: &Path) -> Result<Vec<u8>> {
    Ok(manifest_bytes(&build_manifest(repo_root)?))
}

/// Loads a committed manifest (strict per line).
fn load_manifest(path: &Path) -> Result<Vec<ManifestEntry>> {
    load_jsonl(path, "manifest")
}

fn manifest(repo_root: &Path, out: &Path) -> Result<()> {
    let out_path = crate::dsn_corpus::resolve_output(repo_root, out);
    let bytes = build_manifest_bytes(repo_root)?;
    let mut file = std::fs::File::create(&out_path)
        .with_context(|| format!("creating {}", out_path.display()))?;
    file.write_all(&bytes)
        .with_context(|| format!("writing {}", out_path.display()))?;
    println!(
        "wrote {} manifest entr{} to {}",
        bytes.iter().filter(|b| **b == b'\n').count(),
        if bytes.iter().filter(|b| **b == b'\n').count() == 1 {
            "y"
        } else {
            "ies"
        },
        out_path.display()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// The golden record
// ---------------------------------------------------------------------------

/// One deduped clearance violation: `(min(id1,id2), max(id1,id2),
/// layer)` — the record shape of `getAllClearanceViolations` after the
/// canonical sort by (a, b, layer).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViolationRecord {
    pub a: i64,
    pub b: i64,
    pub layer: i64,
}

/// One Delaunay INPUT object: a grouped (filtered) item id with its
/// ratsnest corner count. `n == 0` is the falsification mechanism —
/// a both-ends-contacted trace contributes no corner (Trace stubs
/// only).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RatsnestRecord {
    pub id: i64,
    pub n: i64,
}

/// One per-net row (schema v2). `items` is the RAW net-list size
/// (calculateAllIncompletes' list, pre-filter); `groups` is the count
/// of unique connected sets over the FILTERED items;
/// `incomplete_count` is the Delaunay/Kruskal airline count (NOT the
/// naive `max(0, groups - 1)` — see the falsified-claim docs);
/// `ratsnest` is the Delaunay input object list (sorted by id);
/// `edges` is the canonical sorted (min-id, max-id) ResultEdge pair
/// list — the edge set the count is a function of.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PerNetRecord {
    pub net_no: i64,
    pub items: i64,
    pub groups: i64,
    pub incomplete_count: i64,
    pub ratsnest: Vec<RatsnestRecord>,
    pub edges: Vec<[i64; 2]>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoldenRecord {
    pub id: String,
    pub file: String,
    pub result: String,
    pub incomplete_count: Option<i64>,
    pub max_connections: Option<i64>,
    pub clearance_violations_total: Option<i64>,
    pub violations: Option<Vec<ViolationRecord>>,
    pub per_net: Option<Vec<PerNetRecord>>,
}

impl crate::corpus_common::HasId for GoldenRecord {
    fn id(&self) -> &str {
        &self.id
    }
}

pub fn result_only(id: &str, path: &str, result: &str) -> GoldenRecord {
    GoldenRecord {
        id: id.to_string(),
        file: path.to_string(),
        result: result.to_string(),
        incomplete_count: None,
        max_connections: None,
        clearance_violations_total: None,
        violations: None,
        per_net: None,
    }
}

// ---------------------------------------------------------------------------
// The Rust protocol (the mirror of DrcOracle.java)
// ---------------------------------------------------------------------------

/// Runs the full protocol on one fixture with the Rust port.
pub fn evaluate_rust(id: &str, path: &str, bytes: &[u8]) -> GoldenRecord {
    let mut ses = SesBoard::new();
    let read = read_board(bytes, &mut ses);
    let DsnReadResult::Success { warnings: _ } = read else {
        return result_only(id, path, "read-failed");
    };
    let mut board = Board::from_ses_board(&ses);
    let mut manager = SearchTreeManager::new();
    // Protocol step 2: the uniform fill normalization, no other
    // pre-steps (the drc surface is pure parse-time).
    manager.reinsert_tree_items(&mut board);

    // Steps 3 + 5: the incompletes — one row per non-empty net plus
    // the maxConnections endpoint sum; the fixture total is the per-net
    // sum (the oracle exits 4 unless it equals getIncompleteCount()).
    let (max_conn, rows) = epic_drc::incompletes::all_incompletes(&manager, &mut board);
    let incomplete_total: i64 = rows.iter().map(|row| row.incomplete_count as i64).sum();
    let per_net: Vec<PerNetRecord> = rows
        .iter()
        .map(|row| PerNetRecord {
            net_no: i64::from(row.net_no),
            items: row.items as i64,
            groups: row.groups as i64,
            incomplete_count: row.incomplete_count as i64,
            ratsnest: row
                .ratsnest
                .iter()
                .map(|(item_id, n)| RatsnestRecord {
                    id: i64::from(item_id.get()),
                    n: *n as i64,
                })
                .collect(),
            edges: row.edges.clone(),
        })
        .collect();

    // Step 4: the clearance walk with the dedup + canonical sort.
    let (violation_total, violations) =
        epic_drc::clearance::all_clearance_violations(&mut manager, &mut board);

    GoldenRecord {
        id: id.to_string(),
        file: path.to_string(),
        result: "ok".to_string(),
        incomplete_count: Some(incomplete_total),
        max_connections: Some(max_conn),
        clearance_violations_total: Some(violation_total),
        violations: Some(
            violations
                .iter()
                .map(|row| ViolationRecord {
                    a: row.a,
                    b: row.b,
                    layer: row.layer,
                })
                .collect(),
        ),
        per_net: Some(per_net),
    }
}

// ---------------------------------------------------------------------------
// `drc golden` — one JVM per run, javac-compiled because the oracle
// declares a package (index-corpus machinery, renamed for the drc
// oracle).
// ---------------------------------------------------------------------------

fn golden(repo_root: &Path, manifest: &Path, out: &Path, jvm_xmx: &str) -> Result<()> {
    let started = Instant::now();
    let java = crate::oracle::resolve_java()?;
    let javac = java.with_file_name("javac");
    anyhow::ensure!(
        javac.is_file(),
        "javac not found next to {} — the JDK is required for the drc oracle",
        java.display()
    );
    let jar = crate::oracle::jar_path(repo_root);
    anyhow::ensure!(
        jar.is_file(),
        "oracle jar missing at {} — build it once with `./gradlew executableJar`",
        jar.display()
    );
    let oracle_src = repo_root.join("rust/harness/oracle/DrcOracle.java");
    anyhow::ensure!(
        oracle_src.is_file(),
        "oracle evaluator missing at {}",
        oracle_src.display()
    );
    let manifest_path = crate::dsn_corpus::resolve_input(repo_root, manifest);
    let entries = load_manifest(&manifest_path)?;
    anyhow::ensure!(
        !entries.is_empty(),
        "manifest {} is empty",
        manifest_path.display()
    );

    // Compile the package-declared oracle into a temp classes dir.
    // Every bail from here to the post-wait cleanup goes through
    // `cleanup_temps` (index-corpus leak lesson).
    let classes_dir =
        std::env::temp_dir().join(format!("epic-drc-oracle-classes-{}", std::process::id()));
    let jvm_manifest =
        std::env::temp_dir().join(format!("epic-drc-manifest-{}.jsonl", std::process::id()));
    let stderr_path =
        std::env::temp_dir().join(format!("epic-drc-oracle-stderr-{}.log", std::process::id()));
    fn cleanup_temps(classes_dir: &Path, jvm_manifest: &Path, stderr_path: &Path) {
        let _ = std::fs::remove_file(jvm_manifest);
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
        .with_context(|| format!("running javac on {}", oracle_src.display()))?;
    if !compile.status.success() {
        cleanup_temps(&classes_dir, &jvm_manifest, &stderr_path);
        bail!(
            "javac failed:\n{}",
            String::from_utf8_lossy(&compile.stderr)
        );
    }

    {
        let mut file = std::fs::File::create(&jvm_manifest)
            .with_context(|| format!("creating {}", jvm_manifest.display()))?;
        for entry in &entries {
            writeln!(
                file,
                "{}",
                serde_json::to_string(entry).expect("manifest entry serializes")
            )
            .with_context(|| format!("writing {}", jvm_manifest.display()))?;
        }
    }

    let classpath = format!("{}:{}", jar.display(), classes_dir.display());
    let stderr_file = std::fs::File::create(&stderr_path)
        .with_context(|| format!("creating {}", stderr_path.display()))
        .inspect_err(|_| {
            cleanup_temps(&classes_dir, &jvm_manifest, &stderr_path);
        })?;
    let mut child = std::process::Command::new(&java)
        .arg(format!("-Xmx{jvm_xmx}"))
        // Locale-pinned (tr-TR lesson, dsn corpus).
        .arg("-Duser.language=en")
        .arg("-Duser.country=US")
        .arg("-cp")
        .arg(&classpath)
        .arg("app.freerouting.datastructures.DrcOracle")
        .arg(&jvm_manifest)
        .current_dir(repo_root)
        .stdout(std::process::Stdio::piped())
        .stderr(stderr_file)
        .spawn()
        .with_context(|| format!("spawning {} with the drc oracle", java.display()))
        .inspect_err(|_| {
            cleanup_temps(&classes_dir, &jvm_manifest, &stderr_path);
        })?;

    // Byte-wise read of `{"id"` lines (binary-fixture lesson, dsn
    // corpus).
    let stdout = child
        .stdout
        .take()
        .context("oracle stdout not captured")
        .inspect_err(|_| {
            cleanup_temps(&classes_dir, &jvm_manifest, &stderr_path);
        })?;
    let mut fresh_lines = Vec::new();
    {
        let mut reader = std::io::BufReader::new(stdout);
        let mut raw = Vec::new();
        loop {
            raw.clear();
            let read = reader
                .read_until(b'\n', &mut raw)
                .context("reading oracle stdout")
                .inspect_err(|_| {
                    cleanup_temps(&classes_dir, &jvm_manifest, &stderr_path);
                })?;
            if read == 0 {
                break;
            }
            if raw.starts_with(b"{\"id\"") {
                let line = String::from_utf8_lossy(&raw);
                fresh_lines.push(line.trim_end_matches(['\n', '\r']).to_string());
            }
        }
    }
    drop(child.stderr.take());
    let status = child
        .wait()
        .context("waiting for the oracle")
        .inspect_err(|_| {
            cleanup_temps(&classes_dir, &jvm_manifest, &stderr_path);
        })?;
    let _ = std::fs::remove_file(&jvm_manifest);
    let _ = std::fs::remove_dir_all(&classes_dir);
    if !status.success() {
        let stderr = std::fs::read_to_string(&stderr_path).unwrap_or_default();
        let _ = std::fs::remove_file(&stderr_path);
        bail!(
            "drc oracle failed with {status} (captured {}/{} result lines before failure):\n{}",
            fresh_lines.len(),
            entries.len(),
            stderr.trim_end()
        );
    }
    let stderr_bytes = std::fs::metadata(&stderr_path)
        .map(|meta| meta.len())
        .unwrap_or(0);
    if stderr_bytes > 0 {
        println!(
            "note: oracle stderr held {stderr_bytes} byte(s) of FRLogger noise on a successful capture (discarded)"
        );
    }
    let _ = std::fs::remove_file(&stderr_path);

    anyhow::ensure!(
        fresh_lines.len() == entries.len(),
        "oracle produced {} result line(s) for {} manifest entries — truncated capture must not be committed",
        fresh_lines.len(),
        entries.len()
    );
    let mut records = Vec::new();
    for line in &fresh_lines {
        let record: GoldenRecord = serde_json::from_str(line)
            .with_context(|| format!("parsing oracle result line {line}"))?;
        records.push(record);
    }
    for (entry, record) in entries.iter().zip(&records) {
        anyhow::ensure!(
            record.id == entry.id,
            "oracle returned id {} for manifest entry {} — machinery bug",
            record.id,
            entry.id
        );
    }

    let out_path = crate::dsn_corpus::resolve_output(repo_root, out);
    let mut file = std::fs::File::create(&out_path)
        .with_context(|| format!("creating {}", out_path.display()))?;
    for record in &records {
        writeln!(
            file,
            "{}",
            serde_json::to_string(record).expect("golden record serialization cannot fail")
        )
        .with_context(|| format!("writing {}", out_path.display()))?;
    }
    println!(
        "captured {} golden record(s) to {} in {:.1}s",
        records.len(),
        out_path.display(),
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// `drc compare` — java-free, CI-able
// ---------------------------------------------------------------------------

/// Field-for-field diff: (field path, golden value, rust value) for
/// every differing field, in schema order. Nested sections report
/// the first differing INDEX and field (`per_net[3].groups`).
pub fn diff_records(gold: &GoldenRecord, rust: &GoldenRecord) -> Vec<(String, String, String)> {
    fn diff<T: Serialize + PartialEq>(
        out: &mut Vec<(String, String, String)>,
        field: &str,
        gold: &T,
        rust: &T,
    ) {
        if gold != rust {
            out.push((field.to_string(), json_string(gold), json_string(rust)));
        }
    }
    let mut out = Vec::new();
    diff(&mut out, "id", &gold.id, &rust.id);
    diff(&mut out, "file", &gold.file, &rust.file);
    diff(&mut out, "result", &gold.result, &rust.result);
    diff(
        &mut out,
        "incomplete_count",
        &gold.incomplete_count,
        &rust.incomplete_count,
    );
    diff(
        &mut out,
        "max_connections",
        &gold.max_connections,
        &rust.max_connections,
    );
    diff(
        &mut out,
        "clearance_violations_total",
        &gold.clearance_violations_total,
        &rust.clearance_violations_total,
    );
    match (&gold.violations, &rust.violations) {
        (Some(g), Some(r)) => {
            if g.len() != r.len() {
                out.push((
                    "violations.len".to_string(),
                    g.len().to_string(),
                    r.len().to_string(),
                ));
            }
            for (index, (gv, rv)) in g.iter().zip(r).enumerate() {
                let at = format!("violations[{index}]");
                diff(&mut out, &format!("{at}.a"), &gv.a, &rv.a);
                diff(&mut out, &format!("{at}.b"), &gv.b, &rv.b);
                diff(&mut out, &format!("{at}.layer"), &gv.layer, &rv.layer);
            }
        }
        (None, None) => {}
        (g, r) => out.push((
            "violations".to_string(),
            g.is_some().to_string(),
            r.is_some().to_string(),
        )),
    }
    match (&gold.per_net, &rust.per_net) {
        (Some(g), Some(r)) => {
            if g.len() != r.len() {
                out.push((
                    "per_net.len".to_string(),
                    g.len().to_string(),
                    r.len().to_string(),
                ));
            }
            for (index, (gv, rv)) in g.iter().zip(r).enumerate() {
                let at = format!("per_net[{index}]");
                diff(&mut out, &format!("{at}.net_no"), &gv.net_no, &rv.net_no);
                diff(&mut out, &format!("{at}.items"), &gv.items, &rv.items);
                diff(&mut out, &format!("{at}.groups"), &gv.groups, &rv.groups);
                // THE equivalence-claim field: a divergence here with
                // matching groups is the falsified claim's signature.
                diff(
                    &mut out,
                    &format!("{at}.incomplete_count"),
                    &gv.incomplete_count,
                    &rv.incomplete_count,
                );
                match (&gv.ratsnest, &rv.ratsnest) {
                    (g, r) if g.len() != r.len() => out.push((
                        format!("{at}.ratsnest.len"),
                        g.len().to_string(),
                        r.len().to_string(),
                    )),
                    (g, r) => {
                        for (ri, (gr, rr)) in g.iter().zip(r).enumerate() {
                            let rat = format!("{at}.ratsnest[{ri}]");
                            diff(&mut out, &format!("{rat}.id"), &gr.id, &rr.id);
                            diff(&mut out, &format!("{rat}.n"), &gr.n, &rr.n);
                        }
                    }
                }
                match (&gv.edges, &rv.edges) {
                    (g, r) if g.len() != r.len() => out.push((
                        format!("{at}.edges.len"),
                        g.len().to_string(),
                        r.len().to_string(),
                    )),
                    (g, r) => {
                        for (ei, (ge, re)) in g.iter().zip(r).enumerate() {
                            diff(&mut out, &format!("{at}.edges[{ei}]"), ge, re);
                        }
                    }
                }
            }
        }
        (None, None) => {}
        (g, r) => out.push((
            "per_net".to_string(),
            g.is_some().to_string(),
            r.is_some().to_string(),
        )),
    }
    out
}

fn compare(repo_root: &Path, manifest: &Path, golden: &Path) -> Result<()> {
    let started = Instant::now();
    let manifest_path = crate::dsn_corpus::resolve_input(repo_root, manifest);
    let golden_path = crate::dsn_corpus::resolve_input(repo_root, golden);
    let entries = load_manifest(&manifest_path)?;
    let records: Vec<GoldenRecord> = load_jsonl(&golden_path, "golden")?;
    ensure_alignment(&manifest_path, &entries, &golden_path, &records)?;

    let mut mismatches = 0usize;
    let mut shown = 0usize;
    let mut census: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for entry in &entries {
        let bytes = std::fs::read(repo_root.join(&entry.path))
            .with_context(|| format!("reading fixture {}", entry.path))?;
        let rust = evaluate_rust(&entry.id, &entry.path, &bytes);
        let gold = records
            .iter()
            .find(|record| record.id == entry.id)
            .expect("ensure_alignment checked the id sequence");
        let diffs = diff_records(gold, &rust);
        if diffs.is_empty() {
            continue;
        }
        mismatches += 1;
        *census
            .entry(
                diffs[0]
                    .0
                    .split('.')
                    .next()
                    .unwrap_or(&diffs[0].0)
                    .to_string(),
            )
            .or_insert(0) += 1;
        println!(
            "{} ({}) diverges — {} field(s):",
            entry.id,
            entry.path,
            diffs.len()
        );
        if shown < 20 {
            shown += 1;
            for (field, gold_value, rust_value) in &diffs {
                println!(
                    "  {field}: golden={} rust={}",
                    truncate(gold_value),
                    truncate(rust_value)
                );
            }
        }
    }
    if mismatches == 0 {
        println!(
            "drc compare: {} fixture(s) identical in {:.1}s",
            entries.len(),
            started.elapsed().as_secs_f64()
        );
        Ok(())
    } else {
        let census = census
            .iter()
            .map(|(section, count)| format!("{section}x{count}"))
            .collect::<Vec<_>>()
            .join(", ");
        bail!(
            "drc compare: {mismatches}/{} fixture(s) diverge (first-diff census: {census})",
            entries.len()
        );
    }
}

// ---------------------------------------------------------------------------
// Pins (manifest/record mechanics — the parity pins live with the port)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod pins {
    use super::*;

    /// The manifest is a pure function of the tree: two builds are
    /// byte-identical; tier A's fixtures come first in tiers.yaml
    /// order, then the three PCBench pre-routed boards; the id
    /// scheme is `drc-NNNN`.
    #[test]
    fn manifest_build_is_deterministic_tier_a_then_pcbench() {
        let root = crate::oracle::find_repo_root().expect("repo root");
        let first = build_manifest_bytes(&root).expect("manifest builds");
        let second = build_manifest_bytes(&root).expect("manifest builds again");
        assert_eq!(
            first, second,
            "manifest regeneration must be byte-identical"
        );
        let entries = build_manifest(&root).expect("manifest entries");
        assert_eq!(
            entries.len(),
            17,
            "11 tier A + 3 PCBench pre-routed + 3 craft pin fixtures"
        );
        for (index, entry) in entries.iter().enumerate() {
            assert_eq!(entry.id, format!("drc-{:04}", index + 1), "id scheme");
        }
        let pcbench_at = entries
            .iter()
            .position(|entry| entry.path.contains("PCBench"))
            .expect("PCBench fixtures present");
        assert_eq!(
            pcbench_at, 11,
            "the three PCBench boards start the manifest tail"
        );
        assert!(
            entries[pcbench_at..pcbench_at + 3]
                .iter()
                .all(|entry| entry.path.ends_with("reference-routed.dsn")),
            "then exactly the reference-routed boards"
        );
        assert!(
            entries[pcbench_at + 3..]
                .iter()
                .all(|entry| entry.path.contains("corpus/craft/")),
            "the craft pin fixtures close the manifest"
        );
    }

    /// The record round-trips byte-stable: field order is the struct
    /// order both sides agreed on (the Java Json writer mirrors it).
    /// The pinned row is a v2 shape and carries the falsifier
    /// signature: a filtered item with `n == 0` (the
    /// both-ends-contacted trace that disconnects the Delaunay graph).
    #[test]
    fn golden_record_round_trips_byte_stable() {
        let line = r#"{"id":"drc-0001","file":"f.dsn","result":"ok","incomplete_count":3,"max_connections":9,"clearance_violations_total":1,"violations":[{"a":2,"b":7,"layer":0}],"per_net":[{"net_no":1,"items":4,"groups":2,"incomplete_count":1,"ratsnest":[{"id":11,"n":2},{"id":12,"n":0}],"edges":[[11,12]]}]}"#;
        let record: GoldenRecord = serde_json::from_str(line).expect("parses");
        let row = &record.per_net.as_ref().expect("per_net")[0];
        assert_eq!(
            row.ratsnest[1].n, 0,
            "the n=0 witness row must survive the schema"
        );
        assert_eq!(row.edges, vec![[11, 12]]);
        assert_eq!(
            serde_json::to_string(&record).expect("serializes"),
            line,
            "field order must match the Java emitter"
        );
    }

    /// The diff reports the first diverging per_net row as an indexed
    /// path (`per_net[2].incomplete_count`) — the shape a FALSIFIED
    /// equivalence claim surfaces as in CI.
    #[test]
    fn diff_reports_indexed_per_net_paths() {
        let gold = GoldenRecord {
            id: "drc-0001".into(),
            file: "f.dsn".into(),
            result: "ok".into(),
            incomplete_count: Some(3),
            max_connections: Some(9),
            clearance_violations_total: Some(0),
            violations: Some(Vec::new()),
            per_net: Some(vec![
                PerNetRecord {
                    net_no: 1,
                    items: 4,
                    groups: 2,
                    incomplete_count: 1,
                    ratsnest: vec![RatsnestRecord { id: 11, n: 2 }],
                    edges: vec![[11, 11]],
                },
                PerNetRecord {
                    net_no: 2,
                    items: 9,
                    groups: 3,
                    incomplete_count: 2,
                    ratsnest: vec![
                        RatsnestRecord { id: 21, n: 2 },
                        RatsnestRecord { id: 22, n: 0 },
                        RatsnestRecord { id: 23, n: 1 },
                    ],
                    edges: vec![[21, 23]],
                },
            ]),
        };
        let mut rust = gold.clone();
        rust.per_net.as_mut().expect("per_net")[1].incomplete_count = 1;
        let diffs = diff_records(&gold, &rust);
        assert_eq!(diffs.len(), 1, "only the mutated row: {diffs:?}");
        assert_eq!(diffs[0].0, "per_net[1].incomplete_count");
        // The schema-v2 Delaunay surface diffs at indexed paths too:
        // a mutated corner count (the n=0 witness row) and a mutated
        // edge pair each surface alone.
        let mut rust_n = gold.clone();
        rust_n.per_net.as_mut().expect("per_net")[1].ratsnest[1].n = 1;
        let diffs = diff_records(&gold, &rust_n);
        assert_eq!(diffs.len(), 1, "only the mutated corner: {diffs:?}");
        assert_eq!(diffs[0].0, "per_net[1].ratsnest[1].n");
        let mut rust_edge = gold.clone();
        rust_edge.per_net.as_mut().expect("per_net")[1].edges[0] = [21, 22];
        let diffs = diff_records(&gold, &rust_edge);
        assert_eq!(diffs.len(), 1, "only the mutated edge: {diffs:?}");
        assert_eq!(diffs[0].0, "per_net[1].edges[0]");
        // The fixture TOTAL diverging surfaces as its own top-level
        // field (the Σ the M3 gates consume).
        let mut rust_total = gold.clone();
        rust_total.incomplete_count = Some(4);
        let diffs = diff_records(&gold, &rust_total);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].0, "incomplete_count");
    }

    /// THE WITNESS PIN — on the REAL 655_testboard fixture, loaded
    /// through the SAME corpus path (`evaluate_rust`), no crafted
    /// board. Java's airline count is NOT `groups − 1`: Kruskal unions
    /// run over Delaunay edges of the per-item ratsnest CORNERS, and
    /// both-ends-contacted traces contribute ZERO corners, so whole
    /// groups stay graph-less and the count lands BELOW `groups − 1`.
    /// Nets 3/4/17 are the captured witnesses (count 3 < 4, 2 < 3,
    /// 2 < 3); net 7 is the CONFORMING contrast on the same board
    /// (count 1 == groups − 1). Every literal below is the committed
    /// golden row of drc-0013 — mutation-verified: forcing
    /// `count = groups − 1` flips exactly the witness rows, never the
    /// conforming one.
    #[test]
    fn testboard_655_witness_rows_literal() {
        let rn = |id: i64, n: i64| RatsnestRecord { id, n };
        let zeros = |range: std::ops::RangeInclusive<i64>| {
            range
                .map(|id| RatsnestRecord { id, n: 0 })
                .collect::<Vec<_>>()
        };
        let root = crate::oracle::find_repo_root().expect("repo root");
        let rel = PCBENCH_PRE_ROUTED[1];
        let bytes = std::fs::read(root.join(rel)).expect("655 fixture bytes");
        let record = evaluate_rust("witness-655", rel, &bytes);
        assert_eq!(record.result, "ok", "the fixture must evaluate");
        let per_net = record.per_net.expect("per-net rows");
        let row = |no: i64| {
            per_net
                .iter()
                .find(|row| row.net_no == no)
                .unwrap_or_else(|| panic!("net {no} has a row"))
        };

        // Net 3: 34 raw items, 5 groups, count 3 — BELOW groups − 1.
        let r3 = row(3);
        assert_eq!(
            (r3.items, r3.groups, r3.incomplete_count),
            (34, 5, 3),
            "the falsified-claim verdict on the real board"
        );
        let mut expected_r3 = vec![rn(115, 1), rn(231, 1), rn(243, 1)];
        expected_r3.extend(zeros(355..=372));
        expected_r3.extend(zeros(374..=378));
        expected_r3.extend([rn(677, 1), rn(678, 1)]);
        assert_eq!(r3.ratsnest, expected_r3, "23 corner-less items");
        assert_eq!(
            r3.edges,
            vec![
                [115, 231],
                [115, 243],
                [115, 677],
                [115, 678],
                [231, 243],
                [231, 678],
                [243, 677],
                [677, 678]
            ]
        );

        // Net 4: 23 raw items, 4 groups, count 2 < 3.
        let r4 = row(4);
        assert_eq!((r4.items, r4.groups, r4.incomplete_count), (23, 4, 2));
        let mut expected_r4 = vec![rn(116, 1), rn(255, 1), rn(267, 1)];
        expected_r4.extend(zeros(384..=400));
        assert_eq!(r4.ratsnest, expected_r4, "17 corner-less items");
        assert_eq!(r4.edges, vec![[116, 255], [116, 267], [255, 267]]);

        // Net 17: 34 raw items, 4 groups, count 2 < 3.
        let r17 = row(17);
        assert_eq!((r17.items, r17.groups, r17.incomplete_count), (34, 4, 2));
        let mut expected_r17 = vec![rn(83, 1), rn(140, 1)];
        expected_r17.extend(zeros(597..=620));
        expected_r17.push(rn(623, 0));
        expected_r17.extend([rn(730, 1), rn(731, 1)]);
        assert_eq!(r17.ratsnest, expected_r17, "25 corner-less items");
        assert_eq!(
            r17.edges,
            vec![[83, 140], [83, 731], [140, 730], [140, 731], [730, 731]]
        );

        // The witnesses are genuinely BELOW groups − 1 (the naive
        // formula would say 4/3/3).
        for witness in [r3, r4, r17] {
            assert!(
                witness.incomplete_count < witness.groups - 1,
                "net {} is a count<groups−1 witness",
                witness.net_no
            );
        }

        // THE CONTRAST on the same board: net 7 conforms —
        // count 1 == groups − 1 — so the pin discriminates both
        // behaviors against one fixture.
        let r7 = row(7);
        assert_eq!((r7.items, r7.groups, r7.incomplete_count), (3, 2, 1));
        assert_eq!(
            r7.incomplete_count,
            r7.groups - 1,
            "the conforming contrast survives the same mutation"
        );
        assert_eq!(r7.ratsnest, vec![rn(230, 1), rn(278, 1)]);
        assert_eq!(r7.edges, vec![[230, 278]]);
    }
}
