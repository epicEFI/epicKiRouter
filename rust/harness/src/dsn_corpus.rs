//! DSN parse-parity digest corpus (M1b Task 11): deterministic manifest
//! generation, one-JVM golden capture over the Java oracle, and a
//! java-free field-for-field comparison against the Rust port (D10/D14).
//!
//! The digest record contract is `dsn_digest.rs` (DOC-OF-RECORD) and the
//! plan's `:105-119` schema; the Java side is
//! `rust/harness/oracle/DsnParseOracle.java`, which must byte-match the
//! canonical geometry text and the D12 warning normalization.
//!
//! ## The divergence ledger (Task 12 — RETIRED by M2 Task 13, D22)
//!
//! [`DIVERGENCE_LEDGER`] held the fixtures whose Java goldens encode Java
//! board machinery deliberately NOT ported yet (documented divergence
//! accounting — NEVER a comparator weakening): an entry reclassified a
//! mismatch only when every differing field was in its `allowed_fields`,
//! any other divergence failed, and a ledgered fixture that MATCHED
//! failed as a stale entry to prune. Its one entry — dsn-0151, the
//! M1b-shaped `normalizeAllTraces` gap — died with the T13 port (the
//! Rust digest is now post-normalize like the oracle; the StaleMatch
//! rule fired and the entry was pruned; see the const docs). The
//! classify machinery remains generic, pinned over synthetic entries.
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

use epic_board::board::Board;
use epic_board::items::ItemData;
use epic_board::tree_manager::SearchTreeManager;
use epic_dsn::reader::{DsnReadResult, read_board};
use epic_dsn::ses_board::SesBoard;
use epic_dsn::state::{AngleRestriction, Unit};

// The shared corpus shell (M3 Task 1): the manifest row type and the
// JSONL/alignment/diff mechanics live in corpus_common; this module
// keeps the dsn golden record and its two-set manifest policy.
pub use crate::corpus_common::ManifestEntry;
use crate::corpus_common::{ensure_alignment, json_string, load_jsonl, manifest_bytes, truncate};

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
    /// Capture the Java oracle's parse→emit SES goldens for tier A+B
    /// (Task 13 B3): ONE JVM parses each tier fixture and emits its
    /// session via the jar's SesWriter; the bytes land in
    /// `harness/corpus/ses/<flattened>.ses.golden` (committed artifacts;
    /// regenerate only via this subcommand).
    SesGolden {
        /// Tier file holding the fixtures_root + tier A/B fixture lists.
        #[arg(long, default_value = "harness/config/tiers.yaml")]
        tiers: PathBuf,
        /// Golden output directory (relative paths resolve under rust/).
        #[arg(long, default_value = "harness/corpus/ses")]
        out: PathBuf,
    },
    /// Compare the Rust port's session emission against the committed
    /// SES goldens (Task 13 B4): parse each tier A+B fixture with
    /// epic-dsn, emit via ses::writer, canonical-compare
    /// (whitespace-insensitive sexpr trees, numbers exact) + byte-diff
    /// classification. Java-free, CI-able; exits nonzero on any
    /// T40-snap or genuine divergence.
    SesCompare {
        /// Tier file holding the fixtures_root + tier A/B fixture lists.
        #[arg(long, default_value = "harness/config/tiers.yaml")]
        tiers: PathBuf,
        /// The committed golden directory (relative paths resolve under
        /// rust/).
        #[arg(long, default_value = "harness/corpus/ses")]
        golden: PathBuf,
    },
    /// Capture the ses-snap corpus (M2 Task 15): run SesSnapOracle
    /// (ONE JVM, capture mode) over the COMMITTED routed-fixture
    /// manifest, cross-check the captured bytes/counters against the
    /// stats sidecar (drift = the jar changed; deliberate update
    /// required), and rewrite the corpus goldens + `stats.json`.
    /// Manual step — CI runs the java-free `ses-snap-compare` only.
    SesSnapGolden,
    /// Compare the CONTACTS-WIRED Rust writer against the committed
    /// ses-snap goldens (M2 Task 15): parse each ROUTED fixture,
    /// replay the T13 pipeline (board + tree fill + normalize, with a
    /// normalize-stability assert), collect SessionDrillContacts,
    /// reproduce the Java counters (provider gate), and byte-compare
    /// `write_session_with_contacts` (byte-equal bar). Java-free,
    /// CI-able; bails on any diff, vacuous fixture, or snap canary
    /// firing.
    SesSnapCompare,
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
        DsnCommand::SesGolden { tiers, out } => {
            crate::ses_compare::golden(&repo_root, &tiers, &out, jvm_xmx)
        }
        DsnCommand::SesCompare { tiers, golden } => {
            crate::ses_compare::compare(&repo_root, &tiers, &golden)
        }
        DsnCommand::SesSnapGolden => crate::ses_compare::snap_golden(&repo_root, jvm_xmx),
        DsnCommand::SesSnapCompare => crate::ses_compare::snap_compare(&repo_root),
    }
}

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

// The manifest row is the shared `ManifestEntry` (re-exported above);
// the `dsn-`/`soak-` id schemes and the two-set fixture policy below
// are this corpus's own. `path` is repo-relative — see module docs.

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
    Ok(manifest_bytes(&build_manifest(repo_root)?))
}

/// Loads a committed manifest (strict per line).
fn load_manifest(path: &Path) -> Result<Vec<ManifestEntry>> {
    load_jsonl(path, "manifest")
}

pub(crate) fn resolve_input(repo_root: &Path, given: &Path) -> PathBuf {
    // exists(), not is_file(): inputs may be directories (the SES golden
    // corpus dir for `dsn ses-compare`).
    if given.is_absolute() || given.exists() {
        given.to_path_buf()
    } else {
        // Default values are written relative to the rust/ checkout; also
        // accept repo-root-relative spellings.
        let candidate = repo_root.join("rust").join(given);
        if candidate.exists() {
            candidate
        } else {
            repo_root.join(given)
        }
    }
}

pub(crate) fn resolve_output(repo_root: &Path, given: &Path) -> PathBuf {
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
    /// T13 post-normalize fields: the SECOND `normalizeAllTraces` call's
    /// re-digest. By construction a fixpoint — `post_stats == stats` and
    /// `post_geometry_sha256 == geometry_sha256` for every fixture — the
    /// goldens' ENCODED idempotence claim. Appended AFTER snap_angle on
    /// both sides (Gson insertion order / Rust struct order).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_stats: Option<StatsRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_geometry_sha256: Option<String>,
}

impl crate::corpus_common::HasId for GoldenRecord {
    fn id(&self) -> &str {
        &self.id
    }
}

/// The `stats` builder over the POST-normalize Board — the mirror of
/// `DsnParseOracle.buildStats` (T13). Java counts `board.getItems()`
/// (LIVE items only — the on-the-board filter; dsn-0151 reads items=2,
/// not the parsed item total), and `nets` is
/// `rules.nets.maxNetNumber()` (Java `DsnParseOracle.java:216`) — NOT a
/// net-table length (the T13 trap note: sparse net numbers diverge).
fn board_stats(board: &Board) -> StatsRecord {
    let mut items = 0i64;
    let mut pads = 0i64;
    let mut traces = 0i64;
    let mut vias = 0i64;
    for entry in board.iter_descending() {
        if !entry.on_the_board {
            continue; // Java getItems() is live-only
        }
        items += 1;
        match entry.data {
            ItemData::Pin { .. } => pads += 1,
            ItemData::Trace { .. } => traces += 1,
            ItemData::Via { .. } => vias += 1,
            _ => {}
        }
    }
    StatsRecord {
        layers: i64::try_from(board.layers().layers.len()).expect("layer count fits i64"),
        items,
        components: i64::from(board.components().count()),
        pads,
        nets: i64::from(board.rules().nets.max_net_number()),
        traces,
        vias,
    }
}

/// Builds the digest record for a parsed board (mirror of
/// `DsnParseOracle.successRecord`; every field is defined identically on
/// both sides — `stats` counts, layer-0 clearance matrix, net/layer
/// tables, D12 warnings, unit/resolution/snap angle).
///
/// T13: `stats` + `geometry_sha256` are sourced from the POST-normalize
/// Board — the Java oracle digests `success.board()` AFTER the read's
/// in-scope `normalizeAllTraces()` (Wiring.java:343-353), so the Rust
/// side replays exactly that: `Board::from_ses_board` → the READ-path
/// tree fill (`insert_items_creation_order`, ASCENDING — the T11
/// two-path record; the DESCENDING rebuild fill is a different skeleton)
/// → `normalize_all_traces` → digest. `post_stats` +
/// `post_geometry_sha256` then run the SECOND call (the fixpoint the
/// goldens encode). Every OTHER field stays on the SesBoard IR —
/// unchanged code paths, unchanged values.
fn success_record(id: &str, path: &str, ses: &SesBoard, warnings: &[String]) -> GoldenRecord {
    let clearance_ir = &ses.rules.clearance;
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
    let string_quote = ses.metadata.string_quote.clone();
    let mut board = Board::from_ses_board(ses);
    let mut manager = SearchTreeManager::new();
    manager.insert_items_creation_order(&mut board);
    // The in-read call (Wiring.java:343-353) — Java's parse already ran
    // it before successRecord saw the board.
    epic_board::normalize_all::normalize_all_traces(&mut manager, &mut board);
    let stats = board_stats(&board);
    let geometry = crate::dsn_digest::geometry_sha256_board(&board, &string_quote);
    // The SECOND call — the explicit fixpoint re-digest the post_ fields
    // make (the oracle's `board.normalizeAllTraces()` in successRecord).
    epic_board::normalize_all::normalize_all_traces(&mut manager, &mut board);
    let post_stats = board_stats(&board);
    let post_geometry = crate::dsn_digest::geometry_sha256_board(&board, &string_quote);
    GoldenRecord {
        id: id.to_string(),
        file: path.to_string(),
        result: "Success".to_string(),
        stats: Some(stats),
        geometry_sha256: Some(geometry),
        clearance: Some(clearance),
        // KNOWN ASYMMETRY (T13 quality round): the net_table renders
        // `ses.nets` in IR order, while `board_stats` above uses
        // `max_net_number()`. On a fixture with SPARSE net numbers Java
        // emits "" gap rows for the missing numbers here and Rust
        // compacts them — the digest compare FAILS LOUDLY (never a
        // silent pass), and no corpus fixture is sparse, so the 1,332
        // corpus gate is the guard. If one ever appears, replicate
        // Java's gap-row padding here.
        net_table: Some(ses.nets.iter().map(|net| net.name.clone()).collect()),
        layer_table: Some(
            ses.layers
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
            match ses.metadata.unit {
                Unit::Mil => "MIL",
                Unit::Inch => "INCH",
                Unit::Mm => "MM",
                Unit::Um => "UM",
            }
            .to_string(),
        ),
        resolution: Some(i64::from(ses.metadata.resolution)),
        snap_angle: Some(
            match ses.rules.trace_angle_restriction {
                AngleRestriction::None => "none",
                AngleRestriction::FortyfiveDegree => "45",
                AngleRestriction::NinetyDegree => "90",
            }
            .to_string(),
        ),
        post_stats: Some(post_stats),
        post_geometry_sha256: Some(post_geometry),
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
        post_stats: None,
        post_geometry_sha256: None,
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
    load_jsonl(path, "golden")
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
    diff(&mut out, "post_stats", &gold.post_stats, &rust.post_stats);
    diff(
        &mut out,
        "post_geometry_sha256",
        &gold.post_geometry_sha256,
        &rust.post_geometry_sha256,
    );
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
        // Locale-pinned: locale-sensitive toUpperCase/equalsIgnoreCase paths in the engine must not capture different goldens on a non-en host (tr-TR).
        .arg("-Duser.language=en")
        .arg("-Duser.country=US")
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
    // Success: keep the discard semantics, but surface non-empty FRLogger
    // noise instead of silently deleting it.
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

/// One DOCUMENTED divergence between the goldens and the Rust port: a
/// fixture whose Java golden encodes Java board machinery deliberately NOT
/// ported in M1b. This is divergence ACCOUNTING, not comparator weakening —
/// the fixture is still fully parsed and field-for-field diffed, and an
/// entry only reclassifies a mismatch when EVERY differing field is in
/// `allowed_fields`; anything else fails the compare. A ledgered fixture
/// that unexpectedly MATCHES also fails (stale entry to prune).
///
/// RETIRED (M2 Task 13, decision D22): the ledger's one entry — dsn-0151
/// (`fixtures/Issue723-CombineStackOverflow.dsn`, "Java ends the wiring
/// read with board.normalizeAllTraces() (Wiring.java:345-353), combining
/// the file's 4,000 collinear wire segments into one trace with a fresh
/// id (stats items=2, traces=1); deliberately NOT ported in M1b — board
/// machinery scheduled for M2 (plan :38, decision D11)", trap
/// "T39-adjacent divergence class (plan D11)", allowed fields
/// stats + geometry_sha256) — died with the T13 port: the Rust digest
/// now sources stats/geometry from the post-normalize Board exactly like
/// the Java oracle, dsn-0151 matches on EVERY field, the StaleMatch rule
/// fired on the pre-prune compare, and the entry was pruned. The list is
/// expected to stay empty; the classify machinery below remains generic
/// (pinned over synthetic entries) for any future documented divergence.
struct LedgerEntry {
    /// The manifest fixture id.
    id: &'static str,
    /// Why the divergence exists (doc-of-record; printed by `dsn compare`).
    reason: &'static str,
    /// The trap-table class of the divergence.
    trap: &'static str,
    /// The ONLY golden-vs-rust fields this reason may explain.
    allowed_fields: &'static [&'static str],
}

const DIVERGENCE_LEDGER: &[LedgerEntry] = &[];

fn ledger_entry<'a>(ledger: &'a [LedgerEntry], id: &str) -> Option<&'a LedgerEntry> {
    ledger.iter().find(|entry| entry.id == id)
}

/// The verdict for one fixture's diff against the ledger (pure — pinned in
/// [`pins`], every branch executed there).
enum LedgerVerdict<'a> {
    /// The fixture matches AND is ledgered — a stale entry (compare fails).
    StaleMatch,
    /// The fixture matches and is not ledgered — the normal green path.
    Match,
    /// Ledgered and every differing field is in `allowed_fields`.
    Covered,
    /// Ledgered but some differing field is NOT in `allowed_fields`.
    Outside(&'a LedgerEntry, Vec<&'static str>),
    /// No ledger entry — a plain mismatch.
    Unledgered,
}

fn ledger_classify<'a>(
    ledger: &'a [LedgerEntry],
    id: &str,
    diffs: &[(&'static str, String, String)],
) -> LedgerVerdict<'a> {
    match (diffs.is_empty(), ledger_entry(ledger, id)) {
        (true, Some(_)) => LedgerVerdict::StaleMatch,
        (true, None) => LedgerVerdict::Match,
        (false, None) => LedgerVerdict::Unledgered,
        (false, Some(entry)) => {
            let uncovered: Vec<&'static str> = diffs
                .iter()
                .map(|(field, _, _)| *field)
                .filter(|field| !entry.allowed_fields.contains(field))
                .collect();
            if uncovered.is_empty() {
                LedgerVerdict::Covered
            } else {
                LedgerVerdict::Outside(entry, uncovered)
            }
        }
    }
}

fn compare(repo_root: &Path, manifest: &Path, golden: &Path, set: &str) -> Result<()> {
    let started = Instant::now();
    let sel = SetSel::parse(set)?;
    let manifest_path = resolve_input(repo_root, manifest);
    let golden_path = resolve_input(repo_root, golden);
    let entries = load_manifest(&manifest_path)?;
    let records = load_golden(&golden_path)?;
    ensure_alignment(&manifest_path, &entries, &golden_path, &records)?;

    let mut per_set: BTreeMap<&'static str, (usize, usize)> = BTreeMap::new(); // set -> (checked, matched)
    let mut census: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut ledgered_ids: Vec<&str> = Vec::new();
    let mut stale_ledger: Vec<&str> = Vec::new();
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
        match ledger_classify(DIVERGENCE_LEDGER, &entry.id, &diffs) {
            LedgerVerdict::Match => tally.1 += 1,
            LedgerVerdict::StaleMatch => {
                // A ledgered fixture that now matches means the
                // divergence class was fixed (or the golden changed): the
                // entry is stale and MUST be pruned — a ledger that
                // silently passes on a match is how drift hides. NOT
                // counted as a match (the run fails on it below); the
                // per-set line shows it as a mismatch so the printout
                // never reads clean on a failing run.
                stale_ledger.push(entry.id.as_str());
            }
            LedgerVerdict::Covered => {
                // Documented divergence: fully parsed and diffed, loudly
                // printed, not a failure.
                tally.1 += 1;
                ledgered_ids.push(entry.id.as_str());
                let ledger =
                    ledger_entry(DIVERGENCE_LEDGER, &entry.id).expect("Covered implies ledgered");
                println!(
                    "{} LEDGERED DIVERGENCE ({} differing field(s): {}):",
                    entry.id,
                    diffs.len(),
                    diffs
                        .iter()
                        .map(|(field, _, _)| *field)
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                println!("  trap: {}", ledger.trap);
                println!("  reason: {}", ledger.reason);
            }
            LedgerVerdict::Outside(ledger, uncovered) => {
                *census.entry(diffs[0].0).or_insert(0) += 1;
                println!(
                    "{} diverges OUTSIDE its ledger allowance ({uncovered:?}) — not covered by: {}",
                    entry.id, ledger.reason
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
            LedgerVerdict::Unledgered => {
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
    }

    // Mirror of golden()'s empty-selection ensure: id-scheme drift must
    // fail loudly, not print `0 fixture(s), 0 mismatch(es)` and pass the
    // CI gate while comparing nothing.
    let total_checked: usize = per_set.values().map(|(checked, _)| checked).sum();
    anyhow::ensure!(
        total_checked > 0,
        "set {set} selects no manifest entries from {} — nothing was compared",
        manifest_path.display()
    );

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
        "compare --set {set}: {} fixture(s), {} mismatch(es), {} ledgered divergence(s) in {:.1}s",
        per_set.values().map(|(checked, _)| checked).sum::<usize>(),
        total_mismatches,
        ledgered_ids.len(),
        started.elapsed().as_secs_f64()
    );
    if !stale_ledger.is_empty() {
        bail!(
            "stale divergence ledger: {stale_ledger:?} now matches the golden — prune the entries \
             (the documented divergence is fixed or the golden moved)"
        );
    }
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

    /// The ledger's committed content (doc-of-record): EMPTY — retired by
    /// M2 Task 13 (D22). The dsn-0151 normalizeAllTraces entry died with
    /// the port: the Rust digest now sources stats/geometry from the
    /// post-normalize Board exactly like the Java oracle, the fixture
    /// matches on EVERY field, and the StaleMatch rule fired on the
    /// pre-prune compare (the retirement demonstration). A re-landed
    /// entry without a real divergence fails the compare as stale — the
    /// prune direction is one-way.
    #[test]
    fn divergence_ledger_is_retired() {
        assert_eq!(
            DIVERGENCE_LEDGER.len(),
            0,
            "the M1b divergence ledger was retired by T13 (dsn-0151 matches on every field); \
             a new entry requires a documented divergence and a fresh census"
        );
    }

    /// A synthetic ledger entry exercising the same shape the retired
    /// dsn-0151 entry had (id + stats/geometry allowance), so the
    /// classify machinery stays pinned without pinning any real
    /// divergence.
    fn synthetic_ledger() -> Vec<LedgerEntry> {
        vec![LedgerEntry {
            id: "syn-0151",
            reason: "synthetic entry for the verdict-branch pins",
            trap: "none",
            allowed_fields: &["stats", "geometry_sha256"],
        }]
    }

    fn diff(field: &'static str) -> (&'static str, String, String) {
        (field, "g".to_string(), "r".to_string())
    }

    /// EVERY ledger verdict branch, driven with synthetic diffs against
    /// the SYNTHETIC ledger (branch-executing per the cerebrum pin
    /// rules): green match, stale match (compare must fail), covered
    /// divergence, divergence outside the allowance, and the unledgered
    /// mismatch. Plus the empty-ledger degenerate: with the ledger
    /// retired, EVERYTHING classifies Match/Unledgered.
    #[test]
    fn ledger_verdict_branches_are_exhaustive() {
        let ledger = synthetic_ledger();
        // Green: no diff, no entry.
        assert!(matches!(
            ledger_classify(&ledger, "dsn-0002", &[]),
            LedgerVerdict::Match
        ));
        // Stale: the ledgered fixture MATCHES — compare must fail on it
        // (the exact verdict dsn-0151 produced on the pre-prune compare).
        assert!(matches!(
            ledger_classify(&ledger, "syn-0151", &[]),
            LedgerVerdict::StaleMatch
        ));
        // Covered: the exact retired-entry divergence shape (stats +
        // geometry).
        assert!(matches!(
            ledger_classify(
                &ledger,
                "syn-0151",
                &[diff("stats"), diff("geometry_sha256")]
            ),
            LedgerVerdict::Covered
        ));
        // Outside: the ledger never whitewashes a result flip or any
        // third field.
        let LedgerVerdict::Outside(entry, uncovered) =
            ledger_classify(&ledger, "syn-0151", &[diff("result")])
        else {
            panic!("a result flip must be Outside the allowance");
        };
        assert_eq!(entry.id, "syn-0151");
        assert_eq!(uncovered, vec!["result"]);
        let LedgerVerdict::Outside(_, uncovered) = ledger_classify(
            &ledger,
            "syn-0151",
            &[diff("stats"), diff("geometry_sha256"), diff("warnings_n")],
        ) else {
            panic!("a partially-covered diff must be Outside the allowance");
        };
        assert_eq!(uncovered, vec!["warnings_n"]);
        // Unledgered: any other fixture diverging is a plain mismatch.
        assert!(matches!(
            ledger_classify(&ledger, "dsn-0042", &[diff("stats")]),
            LedgerVerdict::Unledgered
        ));
        // The retired (empty) ledger: matches stay green, mismatches are
        // plain — no entry can ever classify again.
        assert!(matches!(
            ledger_classify(DIVERGENCE_LEDGER, "syn-0151", &[]),
            LedgerVerdict::Match
        ));
        assert!(matches!(
            ledger_classify(DIVERGENCE_LEDGER, "syn-0151", &[diff("stats")]),
            LedgerVerdict::Unledgered
        ));
    }
}

/// Task 11 quality-round pins: comparator/record-builder discrimination
/// (cerebrum pin rules 3+4) + golden-load integrity. All synthetic
/// in-memory records — no parsing, no JVM. A weakened `diff_records` or a
/// vacuous alignment guard must fail these pins naming the gap.
#[cfg(test)]
mod comparator_pins {
    use super::*;
    use crate::corpus_common::parse_jsonl;

    /// A fully populated Success-shaped record. The discrimination pin
    /// mutates exactly ONE field per case, so a comparator that skips a
    /// field fails the pin naming the gap.
    fn sample_record() -> GoldenRecord {
        GoldenRecord {
            id: "dsn-0001".to_string(),
            file: "fixtures/example.dsn".to_string(),
            result: "Success".to_string(),
            stats: Some(StatsRecord {
                layers: 4,
                items: 120,
                components: 10,
                pads: 80,
                nets: 12,
                traces: 30,
                vias: 2,
            }),
            geometry_sha256: Some("ab".repeat(32)),
            clearance: Some(ClearanceRecord {
                classes: vec!["default".to_string(), "power".to_string()],
                values: vec![vec![0, 100], vec![100, 0]],
            }),
            net_table: Some(vec!["gnd".to_string(), "vcc".to_string()]),
            layer_table: Some(vec![
                LayerRecord {
                    name: "top".to_string(),
                    signal: true,
                },
                LayerRecord {
                    name: "drill".to_string(),
                    signal: false,
                },
            ]),
            warnings_n: Some(vec!["wire # skipped".to_string()]),
            unit: Some("UM".to_string()),
            resolution: Some(10),
            snap_angle: Some("45".to_string()),
            post_stats: Some(StatsRecord {
                layers: 4,
                items: 120,
                components: 10,
                pads: 80,
                nets: 12,
                traces: 30,
                vias: 2,
            }),
            post_geometry_sha256: Some("ab".repeat(32)),
        }
    }

    /// Mutating exactly one field must yield EXACTLY ONE diff entry, and
    /// it must name that field — a diff that omits or misnames it is a
    /// vacuous comparator (the anchor-blind gap from the Task 10 review).
    fn assert_field_discriminated(field: &'static str, mutate: impl FnOnce(&mut GoldenRecord)) {
        let gold = sample_record();
        let mut rust = sample_record();
        mutate(&mut rust);
        let diffs = diff_records(&gold, &rust);
        assert_eq!(
            diffs.len(),
            1,
            "mutating only {field} must produce exactly one diff entry, got {diffs:?}"
        );
        assert_eq!(
            diffs[0].0, field,
            "the diff entry must name {field}, got {diffs:?}"
        );
    }

    /// Every comparable field of the golden record can fail the diff on
    /// its own mutation (stats via three different sub-counts; clearance
    /// via classes and values independently).
    #[test]
    fn each_comparable_field_is_discriminated() {
        assert_field_discriminated("id", |r| r.id = "dsn-0002".to_string());
        assert_field_discriminated("file", |r| r.file = "fixtures/other.dsn".to_string());
        assert_field_discriminated("result", |r| r.result = "OutlineMissing".to_string());
        assert_field_discriminated("stats", |r| {
            r.stats.as_mut().expect("stats present").layers += 1;
        });
        assert_field_discriminated("stats", |r| {
            r.stats.as_mut().expect("stats present").nets += 1;
        });
        assert_field_discriminated("stats", |r| {
            r.stats.as_mut().expect("stats present").vias += 1;
        });
        assert_field_discriminated("geometry_sha256", |r| {
            r.geometry_sha256 = Some("cd".repeat(32));
        });
        assert_field_discriminated("clearance", |r| {
            r.clearance.as_mut().expect("clearance present").classes[1] = "signal".to_string();
        });
        assert_field_discriminated("clearance", |r| {
            r.clearance.as_mut().expect("clearance present").values[0][1] += 5;
        });
        assert_field_discriminated("net_table", |r| {
            r.net_table.as_mut().expect("net_table present")[0] = "gnd2".to_string();
        });
        assert_field_discriminated("layer_table", |r| {
            r.layer_table.as_mut().expect("layer_table present")[0].name = "bottom".to_string();
        });
        assert_field_discriminated("warnings_n", |r| {
            r.warnings_n
                .as_mut()
                .expect("warnings_n present")
                .push("extra".to_string());
        });
        assert_field_discriminated("unit", |r| r.unit = Some("MIL".to_string()));
        assert_field_discriminated("resolution", |r| r.resolution = Some(100));
        assert_field_discriminated("snap_angle", |r| r.snap_angle = Some("90".to_string()));
        assert_field_discriminated("post_stats", |r| {
            r.post_stats.as_mut().expect("post_stats present").items += 1;
        });
        assert_field_discriminated("post_geometry_sha256", |r| {
            r.post_geometry_sha256 = Some("ef".repeat(32));
        });
    }

    /// A pair of identical records yields an empty diff (no false
    /// positives).
    #[test]
    fn identical_records_diff_empty() {
        assert!(diff_records(&sample_record(), &sample_record()).is_empty());
    }

    fn sample_entries(count: usize) -> Vec<ManifestEntry> {
        (1..=count)
            .map(|no| ManifestEntry {
                id: format!("dsn-{no:04}"),
                path: format!("fixtures/f{no}.dsn"),
            })
            .collect()
    }

    fn golden_lines(count: usize) -> Vec<String> {
        (1..=count)
            .map(|no| {
                let mut record = sample_record();
                record.id = format!("dsn-{no:04}");
                serde_json::to_string(&record).expect("golden record serializes")
            })
            .collect()
    }

    /// Golden-load integrity: the compare-path load+zip rejects a
    /// truncated golden (missing last line) and a reordered golden (two
    /// lines swapped) — non-empty failure, never silent acceptance.
    #[test]
    fn golden_load_rejects_truncated_and_reordered() {
        let entries = sample_entries(3);
        let lines = golden_lines(3);
        let join = |ls: &[String]| ls.join("\n");

        // Control: an aligned pair is accepted.
        let good: Vec<GoldenRecord> =
            parse_jsonl(&join(&lines), "control").expect("control golden parses");
        ensure_alignment(Path::new("m.jsonl"), &entries, Path::new("g.jsonl"), &good)
            .expect("aligned pair accepted");

        // Truncated: the last golden line is missing.
        let mut truncated_lines = lines.clone();
        truncated_lines.pop();
        let truncated: Vec<GoldenRecord> =
            parse_jsonl(&join(&truncated_lines), "truncated").expect("truncated golden parses");
        assert!(
            ensure_alignment(
                Path::new("m.jsonl"),
                &entries,
                Path::new("g.jsonl"),
                &truncated
            )
            .is_err(),
            "a truncated golden must not align silently"
        );

        // Reordered: the first two lines are swapped.
        let mut swapped = lines.clone();
        swapped.swap(0, 1);
        let reordered: Vec<GoldenRecord> =
            parse_jsonl(&join(&swapped), "reordered").expect("reordered golden parses");
        assert!(
            ensure_alignment(
                Path::new("m.jsonl"),
                &entries,
                Path::new("g.jsonl"),
                &reordered
            )
            .is_err(),
            "a reordered golden must not align silently"
        );
    }
}
