//! DSN parse-parity digest corpus (M1b Task 11): deterministic manifest
//! generation, one-JVM golden capture over the Java oracle, and a
//! java-free field-for-field comparison against the Rust port (D10/D14).
//!
//! The digest record contract is `dsn_digest.rs` (DOC-OF-RECORD) and the
//! plan's `:105-119` schema; the Java side is
//! `rust/harness/oracle/DsnParseOracle.java`, which must byte-match the
//! canonical geometry text and the D12 warning normalization.
//!
//! ## Manifest path determinism (the plan's "abs_path", interpreted)
//!
//! The plan (`:260`) words the manifest entries as `{id, abs_path}`.
//! A machine-absolute path cannot be committed without breaking the
//! byte-identical regeneration pin on every other checkout (the M0/M1
//! pins require `epic-harness dsn manifest` to reproduce the committed
//! file exactly). The committed manifest therefore stores REPO-RELATIVE
//! posix paths (`fixtures/example.dsn`,
//! `scripts/benchmark/fixtures/.../unrouted.dsn`); `dsn golden` runs the
//! oracle JVM with the repository root as its working directory, so the
//! relative paths resolve exactly as an absolute path would.

use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use serde::{Deserialize, Serialize};

use epic_dsn::reader::{DsnReadResult, read_board};
use epic_dsn::ses_board::{ItemIr, SesBoard};
use epic_dsn::state::{AngleRestriction, Unit};

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum DsnCommand {
    /// Regenerate the parse-parity manifest from tiers.yaml + directory
    /// walks and write it (deterministic: two runs are byte-identical).
    Manifest {
        /// Output path for the manifest (relative paths resolve against
        /// the repo root's `rust/` checkout).
        #[arg(long, default_value = "harness/corpus/dsn-manifest.jsonl")]
        out: PathBuf,
    },
    /// Evaluate the manifest with the Java oracle (ONE JVM per run, D14)
    /// and write the goldens. Fresh results replace the selected set's
    /// lines; the other set's lines are carried over from the existing
    /// golden (missing entries are an error — capture `--set all` first).
    Golden {
        #[arg(long, default_value = "harness/corpus/dsn-manifest.jsonl")]
        manifest: PathBuf,
        #[arg(long, default_value = "harness/corpus/dsn-golden.jsonl")]
        out: PathBuf,
        /// Which manifest set to capture: digest, soak, or all.
        #[arg(long, default_value = "all")]
        set: String,
    },
    /// Parse every manifest fixture with epic-dsn and diff the digest
    /// record field-for-field against the committed golden (java-free,
    /// CI-able). Prints at most 20 mismatches; exits 1 on any.
    Compare {
        #[arg(long, default_value = "harness/corpus/dsn-manifest.jsonl")]
        manifest: PathBuf,
        #[arg(long, default_value = "harness/corpus/dsn-golden.jsonl")]
        golden: PathBuf,
        /// Which manifest set to compare: digest, soak, or all.
        #[arg(long, default_value = "all")]
        set: String,
    },
}

pub fn run(cmd: DsnCommand, jvm_xmx: &str) -> Result<()> {
    let repo_root = crate::oracle::find_repo_root()?;
    match cmd {
        DsnCommand::Manifest { out } => manifest(&repo_root, &out),
        DsnCommand::Golden { manifest, out, set } => {
            golden(&repo_root, &manifest, &out, &set, jvm_xmx)
        }
        DsnCommand::Compare {
            manifest,
            golden,
            set,
        } => compare(&repo_root, &manifest, &golden, &set),
    }
}

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

/// One manifest line: `{"id":"dsn-0001","path":"fixtures/example.dsn"}`.
/// See the module docs for why `path` is repo-relative.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestEntry {
    pub id: String,
    pub path: String,
}

/// The selected manifest set (`--set digest|soak|all`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetSel {
    Digest,
    Soak,
    All,
}

impl SetSel {
    fn parse(spec: &str) -> Result<Self> {
        match spec {
            "digest" => Ok(Self::Digest),
            "soak" => Ok(Self::Soak),
            "all" => Ok(Self::All),
            other => bail!("unknown set {other:?} (expected digest, soak, or all)"),
        }
    }

    /// The id prefix convention: digest fixtures are `dsn-NNNN`, soak
    /// fixtures are `soak-NNNN` (documented id scheme, module docs of the
    /// manifest builder).
    fn selects(self, id: &str) -> bool {
        match self {
            Self::Digest => id.starts_with("dsn-"),
            Self::Soak => id.starts_with("soak-"),
            Self::All => true,
        }
    }
}

/// Deterministic recursive walk: collects the repo-relative posix paths of
/// (non-directory) files whose names satisfy `keep`, under `dir`, with
/// `rel_prefix` as the repo-relative prefix. The RESULT is sorted by the
/// caller, so the OS directory order never leaks into the manifest.
fn walk_files(
    dir: &Path,
    rel_prefix: &str,
    keep: &mut dyn FnMut(&str) -> bool,
    out: &mut Vec<String>,
) -> Result<()> {
    let entries =
        std::fs::read_dir(dir).with_context(|| format!("reading directory {}", dir.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| format!("reading directory {}", dir.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel = format!("{rel_prefix}/{name}");
        let path = dir.join(&name);
        if path.is_dir() {
            walk_files(&path, &rel, keep, out)?;
        } else if keep(&name) {
            out.push(rel);
        }
    }
    Ok(())
}

/// Builds the manifest entries (pure: a function of the repository tree).
///
/// - Digest set (`dsn-NNNN`): the 23 tier fixtures in tiers.yaml order
///   (paths resolved against the yaml's `fixtures_root`), then every
///   `fixtures/**/*.dsn` lexicographic; dedup by path across the two
///   sources.
/// - Soak set (`soak-NNNN`): every file named `unrouted.dsn` under
///   `scripts/benchmark/fixtures` (recursive), lexicographic by path.
pub fn build_manifest(repo_root: &Path) -> Result<Vec<ManifestEntry>> {
    let mut seen = std::collections::HashSet::new();
    let mut digest_rel = Vec::new();

    let tiers_path = repo_root.join("rust/harness/config/tiers.yaml");
    let tier_file = crate::tiers::TierFile::load(&tiers_path)?;
    for tier in &tier_file.tiers {
        for fixture in &tier.fixtures {
            let rel = format!("{}/{}", tier_file.fixtures_root.display(), fixture.path);
            if seen.insert(rel.clone()) {
                digest_rel.push(rel);
            }
        }
    }

    let mut root_dsn = Vec::new();
    walk_files(
        &repo_root.join("fixtures"),
        "fixtures",
        &mut |name| name.ends_with(".dsn"),
        &mut root_dsn,
    )?;
    root_dsn.sort();
    for rel in root_dsn {
        if seen.insert(rel.clone()) {
            digest_rel.push(rel);
        }
    }

    let mut entries = Vec::new();
    for (index, rel) in digest_rel.iter().enumerate() {
        entries.push(ManifestEntry {
            id: format!("dsn-{:04}", index + 1),
            path: rel.clone(),
        });
    }

    let mut soak_rel = Vec::new();
    walk_files(
        &repo_root.join("scripts/benchmark/fixtures"),
        "scripts/benchmark/fixtures",
        &mut |name| name == "unrouted.dsn",
        &mut soak_rel,
    )?;
    soak_rel.sort();
    for (index, rel) in soak_rel.iter().enumerate() {
        entries.push(ManifestEntry {
            id: format!("soak-{:04}", index + 1),
            path: rel.clone(),
        });
    }
    Ok(entries)
}

/// Serializes the manifest exactly as committed: one compact JSON object
/// per line, `\n`-terminated (including the last).
pub fn build_manifest_bytes(repo_root: &Path) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    for entry in build_manifest(repo_root)? {
        buf.extend_from_slice(
            serde_json::to_string(&entry)
                .expect("manifest entry serialization cannot fail")
                .as_bytes(),
        );
        buf.push(b'\n');
    }
    Ok(buf)
}

/// Loads a committed manifest (strict per line).
fn load_manifest(path: &Path) -> Result<Vec<ManifestEntry>> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading manifest {}", path.display()))?;
    let mut entries = Vec::new();
    for (lineno, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let entry: ManifestEntry = serde_json::from_str(line)
            .with_context(|| format!("{} line {}", path.display(), lineno + 1))?;
        entries.push(entry);
    }
    Ok(entries)
}

fn resolve_input(repo_root: &Path, given: &Path) -> PathBuf {
    if given.is_absolute() || given.is_file() {
        given.to_path_buf()
    } else {
        // Default values are written relative to the rust/ checkout; also
        // accept repo-root-relative spellings.
        let candidate = repo_root.join("rust").join(given);
        if candidate.is_file() {
            candidate
        } else {
            repo_root.join(given)
        }
    }
}

fn resolve_output(repo_root: &Path, given: &Path) -> PathBuf {
    if given.is_absolute() {
        given.to_path_buf()
    } else {
        repo_root.join("rust").join(given)
    }
}

fn manifest(repo_root: &Path, out: &Path) -> Result<()> {
    let started = Instant::now();
    let bytes = build_manifest_bytes(repo_root)?;
    let out_path = resolve_output(repo_root, out);
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(&out_path, &bytes).with_context(|| format!("writing {}", out_path.display()))?;
    let lines = bytes.iter().filter(|&b| *b == b'\n').count();
    let digest = bytes
        .split(|&b| b == b'\n')
        .filter(|line| !line.is_empty())
        .filter(|line| line.starts_with(b"{\"id\":\"dsn-"))
        .count();
    println!(
        "wrote {} manifest line(s) ({} digest, {} soak) to {} in {:.1}s",
        lines,
        digest,
        lines - digest,
        out_path.display(),
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Golden records (strict serde; harness-side only — epic-dsn stays pure)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatsRecord {
    pub layers: i64,
    pub items: i64,
    pub components: i64,
    pub pads: i64,
    pub nets: i64,
    pub traces: i64,
    pub vias: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClearanceRecord {
    pub classes: Vec<String>,
    /// `values[i][j]` = the layer-0 clearance between classes i and j with
    /// no safety margin (Java `getValue(i, j, 0, false)`;
    /// `ClearanceIr.values[0][j][i]` Rust-side).
    pub values: Vec<Vec<i64>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerRecord {
    pub name: String,
    pub signal: bool,
}

/// One golden/compare line (plan `:105-119`). Non-Success results carry
/// only id/file/result — the Option fields are omitted in JSON, matching
/// the Java oracle's emission.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoldenRecord {
    pub id: String,
    pub file: String,
    pub result: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats: Option<StatsRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geometry_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clearance: Option<ClearanceRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub net_table: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer_table: Option<Vec<LayerRecord>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warnings_n: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snap_angle: Option<String>,
}

/// Builds the digest record for a parsed board (mirror of
/// `DsnParseOracle.successRecord`; every field is defined identically on
/// both sides — `stats` counts, layer-0 clearance matrix, net/layer
/// tables, D12 warnings, unit/resolution/snap angle).
fn success_record(id: &str, path: &str, board: &SesBoard, warnings: &[String]) -> GoldenRecord {
    let layers = board
        .layers
        .as_ref()
        .map(|ls| i64::try_from(ls.layers.len()).expect("layer count fits i64"))
        .unwrap_or(0);
    let count = |wanted: &dyn Fn(&ItemIr) -> bool| -> i64 {
        i64::try_from(board.items.iter().filter(|item| wanted(item)).count())
            .expect("item count fits i64")
    };
    let clearance_ir = &board.rules.clearance;
    let class_count = clearance_ir.names.len();
    let clearance = ClearanceRecord {
        classes: clearance_ir.names.clone(),
        values: (0..class_count)
            .map(|i| {
                (0..class_count)
                    .map(|j| {
                        // Java reads getValue(i, j, layer 0, no safety
                        // margin) == ir.values[0][j][i].
                        clearance_ir
                            .values
                            .first()
                            .and_then(|per_layer| per_layer.get(j))
                            .and_then(|row| row.get(i))
                            .copied()
                            .unwrap_or(0)
                            .into()
                    })
                    .collect()
            })
            .collect(),
    };
    GoldenRecord {
        id: id.to_string(),
        file: path.to_string(),
        result: "Success".to_string(),
        stats: Some(StatsRecord {
            layers,
            items: i64::try_from(board.items.len()).expect("item count fits i64"),
            components: i64::try_from(board.components.len()).expect("component count fits i64"),
            pads: count(&|item| matches!(item, ItemIr::Pin { .. })),
            nets: i64::try_from(board.nets.len()).expect("net count fits i64"),
            traces: count(&|item| matches!(item, ItemIr::Trace { .. })),
            vias: count(&|item| matches!(item, ItemIr::Via { .. })),
        }),
        geometry_sha256: Some(crate::dsn_digest::geometry_sha256(board)),
        clearance: Some(clearance),
        net_table: Some(board.nets.iter().map(|net| net.name.clone()).collect()),
        layer_table: Some(
            board
                .layers
                .as_ref()
                .map(|ls| {
                    ls.layers
                        .iter()
                        .map(|layer| LayerRecord {
                            name: layer.name.clone(),
                            signal: layer.is_signal,
                        })
                        .collect()
                })
                .unwrap_or_default(),
        ),
        warnings_n: Some(
            warnings
                .iter()
                .map(|warning| crate::dsn_digest::normalize_warning_digits(warning))
                .collect(),
        ),
        unit: Some(
            match board.metadata.unit {
                Unit::Mil => "MIL",
                Unit::Inch => "INCH",
                Unit::Mm => "MM",
                Unit::Um => "UM",
            }
            .to_string(),
        ),
        resolution: Some(i64::from(board.metadata.resolution)),
        snap_angle: Some(
            match board.rules.trace_angle_restriction {
                AngleRestriction::None => "none",
                AngleRestriction::FortyfiveDegree => "45",
                AngleRestriction::NinetyDegree => "90",
            }
            .to_string(),
        ),
    }
}

fn result_only(id: &str, path: &str, result: &str) -> GoldenRecord {
    GoldenRecord {
        id: id.to_string(),
        file: path.to_string(),
        result: result.to_string(),
        stats: None,
        geometry_sha256: None,
        clearance: None,
        net_table: None,
        layer_table: None,
        warnings_n: None,
        unit: None,
        resolution: None,
        snap_angle: None,
    }
}

/// Parses one fixture with the Rust port and digests it (mirror of
/// `DsnParseOracle.evaluateCase`, including the throwable->ParseError
/// equivalence, which for the in-memory Rust reader is just the ParseError
/// variant).
fn evaluate_rust(id: &str, path: &str, bytes: &[u8]) -> GoldenRecord {
    let mut board = SesBoard::new();
    match read_board(bytes, &mut board) {
        DsnReadResult::Success { warnings } => success_record(id, path, &board, &warnings),
        DsnReadResult::OutlineMissing { .. } => result_only(id, path, "OutlineMissing"),
        DsnReadResult::ParseError { .. } => result_only(id, path, "ParseError"),
        DsnReadResult::IoError => result_only(id, path, "IoError"),
    }
}

/// Loads a committed golden file (strict per line).
fn load_golden(path: &Path) -> Result<Vec<GoldenRecord>> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading golden {}", path.display()))?;
    let mut records = Vec::new();
    for (lineno, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let record: GoldenRecord = serde_json::from_str(line)
            .with_context(|| format!("{} line {}", path.display(), lineno + 1))?;
        records.push(record);
    }
    Ok(records)
}

fn json_string<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "<unserializable>".to_string())
}

fn truncate(text: &str) -> String {
    if text.len() <= 120 {
        text.to_string()
    } else {
        let mut cut = 120;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        format!("{}...", &text[..cut])
    }
}

/// Field-for-field diff: (field name, golden value, rust value) for every
/// differing field, in schema order.
fn diff_records(gold: &GoldenRecord, rust: &GoldenRecord) -> Vec<(&'static str, String, String)> {
    fn diff<T: Serialize + PartialEq>(
        out: &mut Vec<(&'static str, String, String)>,
        field: &'static str,
        gold: &T,
        rust: &T,
    ) {
        if gold != rust {
            out.push((field, json_string(gold), json_string(rust)));
        }
    }
    let mut out = Vec::new();
    diff(&mut out, "id", &gold.id, &rust.id);
    diff(&mut out, "file", &gold.file, &rust.file);
    diff(&mut out, "result", &gold.result, &rust.result);
    diff(&mut out, "stats", &gold.stats, &rust.stats);
    diff(
        &mut out,
        "geometry_sha256",
        &gold.geometry_sha256,
        &rust.geometry_sha256,
    );
    diff(&mut out, "clearance", &gold.clearance, &rust.clearance);
    diff(&mut out, "net_table", &gold.net_table, &rust.net_table);
    diff(
        &mut out,
        "layer_table",
        &gold.layer_table,
        &rust.layer_table,
    );
    diff(&mut out, "warnings_n", &gold.warnings_n, &rust.warnings_n);
    diff(&mut out, "unit", &gold.unit, &rust.unit);
    diff(&mut out, "resolution", &gold.resolution, &rust.resolution);
    diff(&mut out, "snap_angle", &gold.snap_angle, &rust.snap_angle);
    out
}

// ---------------------------------------------------------------------------
// `dsn golden` — one JVM per run (D14)
// ---------------------------------------------------------------------------

fn golden(repo_root: &Path, manifest: &Path, out: &Path, set: &str, jvm_xmx: &str) -> Result<()> {
    let started = Instant::now();
    let sel = SetSel::parse(set)?;
    let java = crate::oracle::resolve_java()?;
    let jar = crate::oracle::jar_path(repo_root);
    anyhow::ensure!(
        jar.is_file(),
        "oracle jar missing at {} — build it once with `./gradlew executableJar`",
        jar.display()
    );
    let oracle_src = repo_root.join("rust/harness/oracle/DsnParseOracle.java");
    anyhow::ensure!(
        oracle_src.is_file(),
        "oracle evaluator missing at {}",
        oracle_src.display()
    );
    let manifest_path = resolve_input(repo_root, manifest);
    let entries = load_manifest(&manifest_path)?;
    let selected: Vec<&ManifestEntry> = entries
        .iter()
        .filter(|entry| sel.selects(&entry.id))
        .collect();
    anyhow::ensure!(
        !selected.is_empty(),
        "set {set} selects no manifest entries from {}",
        manifest_path.display()
    );

    // The JVM manifest: same {"id","path"} shape. The oracle resolves the
    // repo-relative paths against ITS working directory, which we pin to
    // the repo root below.
    let jvm_manifest =
        std::env::temp_dir().join(format!("epic-dsn-manifest-{}.jsonl", std::process::id()));
    {
        let mut file = std::fs::File::create(&jvm_manifest)
            .with_context(|| format!("creating {}", jvm_manifest.display()))?;
        for entry in &selected {
            writeln!(
                file,
                "{}",
                serde_json::to_string(entry).expect("manifest entry serializes")
            )
            .with_context(|| format!("writing {}", jvm_manifest.display()))?;
        }
    }
    let stderr_path =
        std::env::temp_dir().join(format!("epic-dsn-oracle-stderr-{}.log", std::process::id()));

    let mut child = std::process::Command::new(&java)
        .arg(format!("-Xmx{jvm_xmx}"))
        .arg("-cp")
        .arg(&jar)
        .arg(&oracle_src)
        .arg(&jvm_manifest)
        .current_dir(repo_root)
        .stdout(std::process::Stdio::piped())
        .stderr(
            std::fs::File::create(&stderr_path)
                .with_context(|| format!("creating {}", stderr_path.display()))?,
        )
        .spawn()
        .with_context(|| format!("spawning {} with the DSN parse oracle", java.display()))?;

    // Only result lines start with `{"id"` — the jar's FRLogger warnings
    // write straight to stdout in between and are dropped here. Read
    // BYTE-wise: a binary fixture (Issue006) makes FRLogger echo raw
    // control bytes, which would abort UTF-8 `lines()` mid-run.
    let stdout = child.stdout.take().context("oracle stdout not captured")?;
    let mut fresh_lines = Vec::new();
    {
        let mut reader = std::io::BufReader::new(stdout);
        let mut raw = Vec::new();
        loop {
            raw.clear();
            let read = reader
                .read_until(b'\n', &mut raw)
                .context("reading oracle stdout")?;
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
    let status = child.wait().context("waiting for the oracle")?;
    let _ = std::fs::remove_file(&jvm_manifest);
    if !status.success() {
        let stderr = std::fs::read_to_string(&stderr_path).unwrap_or_default();
        let _ = std::fs::remove_file(&stderr_path);
        bail!(
            "oracle failed with {status} (captured {}/{} result lines before failure):\n{}",
            fresh_lines.len(),
            selected.len(),
            stderr.trim_end()
        );
    }
    let _ = std::fs::remove_file(&stderr_path);

    anyhow::ensure!(
        fresh_lines.len() == selected.len(),
        "oracle produced {} result line(s) for {} selected case(s) — truncated capture must not be committed",
        fresh_lines.len(),
        selected.len()
    );
    let mut fresh = Vec::new();
    for line in &fresh_lines {
        let record: GoldenRecord = serde_json::from_str(line)
            .with_context(|| format!("parsing oracle result line {line}"))?;
        fresh.push(record);
    }
    for (entry, record) in selected.iter().zip(&fresh) {
        anyhow::ensure!(
            record.id == entry.id,
            "oracle returned id {} for manifest entry {} — machinery bug",
            record.id,
            entry.id
        );
    }

    // Merge: the selected set's lines come from this run; every other
    // manifest line is carried over from the existing golden so any
    // subset recapture preserves the full committed file.
    let out_path = resolve_output(repo_root, out);
    let existing: HashMap<String, GoldenRecord> = if out_path.is_file() {
        load_golden(&out_path)?
            .into_iter()
            .map(|record| (record.id.clone(), record))
            .collect()
    } else {
        HashMap::new()
    };
    let fresh_by_id: HashMap<String, GoldenRecord> = fresh
        .into_iter()
        .map(|record| (record.id.clone(), record))
        .collect();
    let mut merged = Vec::with_capacity(entries.len());
    for entry in &entries {
        if sel.selects(&entry.id) {
            merged.push(
                fresh_by_id
                    .get(&entry.id)
                    .cloned()
                    .expect("fresh ids verified against the manifest above"),
            );
        } else {
            let carried = existing.get(&entry.id).cloned();
            anyhow::ensure!(
                carried.is_some(),
                "existing golden {} has no entry for unselected id {} — capture --set all first",
                out_path.display(),
                entry.id
            );
            merged.push(carried.expect("checked is_some above"));
        }
    }
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let mut file = std::fs::File::create(&out_path)
        .with_context(|| format!("creating {}", out_path.display()))?;
    for record in &merged {
        writeln!(
            file,
            "{}",
            serde_json::to_string(record).expect("golden record serializes")
        )
        .with_context(|| format!("writing {}", out_path.display()))?;
    }
    println!(
        "captured {} fixture(s) (set {set}) into {} golden line(s) at {} in {:.1}s (java: {})",
        selected.len(),
        merged.len(),
        out_path.display(),
        started.elapsed().as_secs_f64(),
        java.display()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// `dsn compare` — java-free (D10's CI gate)
// ---------------------------------------------------------------------------

fn compare(repo_root: &Path, manifest: &Path, golden: &Path, set: &str) -> Result<()> {
    let started = Instant::now();
    let sel = SetSel::parse(set)?;
    let manifest_path = resolve_input(repo_root, manifest);
    let golden_path = resolve_input(repo_root, golden);
    let entries = load_manifest(&manifest_path)?;
    let records = load_golden(&golden_path)?;
    // Counts checked (M1a lesson): a truncated golden must fail loudly,
    // and the id sequences must agree position by position.
    anyhow::ensure!(
        records.len() == entries.len(),
        "golden {} has {} line(s) but manifest {} has {} — recapture needed",
        golden_path.display(),
        records.len(),
        manifest_path.display(),
        entries.len()
    );
    for (index, (entry, record)) in entries.iter().zip(&records).enumerate() {
        anyhow::ensure!(
            record.id == entry.id,
            "golden line {} carries id {} but manifest line {} is {} — recapture needed",
            index + 1,
            record.id,
            index + 1,
            entry.id
        );
    }

    let mut per_set: BTreeMap<&'static str, (usize, usize)> = BTreeMap::new(); // set -> (checked, matched)
    let mut census: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut shown = 0usize;
    for (entry, gold) in entries.iter().zip(&records) {
        if !sel.selects(&entry.id) {
            continue;
        }
        let set_name: &'static str = if entry.id.starts_with("soak-") {
            "soak"
        } else {
            "digest"
        };
        let abs = repo_root.join(&entry.path);
        if !abs.is_file() {
            bail!(
                "manifest fixture missing on disk: {} ({})",
                entry.id,
                entry.path
            );
        }
        let bytes = std::fs::read(&abs)
            .with_context(|| format!("reading fixture {} ({})", entry.path, entry.id))?;
        let actual = evaluate_rust(&entry.id, &entry.path, &bytes);
        let diffs = diff_records(gold, &actual);
        let tally = per_set.entry(set_name).or_insert((0, 0));
        tally.0 += 1;
        if diffs.is_empty() {
            tally.1 += 1;
        } else {
            *census.entry(diffs[0].0).or_insert(0) += 1;
            if shown < 20 {
                shown += 1;
                println!("{} ({} differing field(s)):", entry.id, diffs.len());
                for (field, gold_value, rust_value) in &diffs {
                    println!(
                        "  {field}: golden={} rust={}",
                        truncate(gold_value),
                        truncate(rust_value)
                    );
                }
            }
        }
    }

    let total_mismatches: usize = census.values().sum();
    for (set_name, (checked, matched)) in &per_set {
        println!(
            "{set_name} set: {checked} fixture(s), {matched} match, {} mismatch",
            checked - matched
        );
    }
    if !census.is_empty() {
        println!("first-differing field census:");
        for (field, count) in &census {
            println!("  {field}: {count}");
        }
    }
    println!(
        "compare --set {set}: {} fixture(s), {} mismatch(es) in {:.1}s",
        per_set.values().map(|(checked, _)| checked).sum::<usize>(),
        total_mismatches,
        started.elapsed().as_secs_f64()
    );
    if total_mismatches > 0 {
        bail!("dsn compare: {total_mismatches} mismatch(es) — parse parity not green");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Pins (fast: no parsing, no JVM — M1b Task 11 regeneration pins). They
// live in this module (not tests/) because the harness is a binary crate:
// integration tests cannot import its code.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod pins {
    use super::*;

    fn repo_root() -> PathBuf {
        // CARGO_MANIFEST_DIR = <repo>/rust/harness.
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .expect("repo root is two levels above the harness crate")
    }

    fn corpus_path(name: &str) -> PathBuf {
        repo_root().join("rust/harness/corpus").join(name)
    }

    /// (a) Regenerating the manifest into a buffer is byte-identical to
    /// the committed `dsn-manifest.jsonl`.
    #[test]
    fn manifest_regenerates_byte_identical() {
        let committed =
            std::fs::read(corpus_path("dsn-manifest.jsonl")).expect("committed manifest");
        let regenerated = build_manifest_bytes(&repo_root()).expect("manifest regeneration");
        assert_eq!(
            regenerated, committed,
            "dsn manifest regeneration drifted from the committed file"
        );
    }

    /// (b) The committed golden line count equals the manifest line count
    /// AND the pinned 1,332 (175 digest + 1,157 soak).
    #[test]
    fn golden_line_count_matches_manifest_and_pin() {
        let manifest =
            std::fs::read(corpus_path("dsn-manifest.jsonl")).expect("committed manifest");
        let golden = std::fs::read(corpus_path("dsn-golden.jsonl")).expect("committed golden");
        let manifest_lines = manifest.iter().filter(|&b| *b == b'\n').count();
        let golden_lines = golden.iter().filter(|&b| *b == b'\n').count();
        assert_eq!(manifest_lines, golden_lines, "golden/manifest count drift");
        assert_eq!(golden_lines, 1332, "pinned corpus size");
    }

    /// (c) The golden's id sequence exactly equals the manifest's.
    #[test]
    fn golden_ids_exactly_equal_manifest_ids() {
        let manifest = load_manifest(&corpus_path("dsn-manifest.jsonl")).expect("manifest");
        let golden = load_golden(&corpus_path("dsn-golden.jsonl")).expect("golden");
        let manifest_ids: Vec<&str> = manifest.iter().map(|entry| entry.id.as_str()).collect();
        let golden_ids: Vec<&str> = golden.iter().map(|record| record.id.as_str()).collect();
        assert_eq!(manifest_ids, golden_ids);
    }

    /// Id scheme + set selection round-trip on synthetic ids.
    #[test]
    fn set_selection_by_id_prefix() {
        assert!(SetSel::parse("digest").expect("valid").selects("dsn-0001"));
        assert!(!SetSel::parse("digest").expect("valid").selects("soak-0001"));
        assert!(SetSel::parse("soak").expect("valid").selects("soak-1157"));
        assert!(!SetSel::parse("soak").expect("valid").selects("dsn-0175"));
        assert!(SetSel::parse("all").expect("valid").selects("dsn-0001"));
        assert!(SetSel::parse("all").expect("valid").selects("soak-0001"));
        assert!(SetSel::parse("bogus").is_err());
    }
}
