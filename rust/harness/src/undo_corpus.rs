//! Undo/snapshot parity corpus (M2 Task 14): the snapshot/undo/redo
//! FACADE ([`epic_board::undo_facade`]) driven against the frozen Java
//! oracle `rust/harness/oracle/UndoOracle.java` on the tier fixtures
//! plus the index-stress stressors, with committed JSONL goldens and a
//! java-free comparison (the T9 corpus shape, mirrored).
//!
//! ## The protocol (BOTH sides run it identically — the Java oracle
//! and [`evaluate_rust`])
//!
//! Per fixture:
//!
//! 1. Parse to the live board (`DsnReader.readBoard` /
//!    `epic_dsn::read_board` + `Board::from_ses_board` +
//!    `insert_items_creation_order` + the in-read
//!    `normalizeAllTraces` tail, `Wiring.java:343-353`). NO
//!    `reinsertTreeItems`: the undo oracle's tree witness pins the raw
//!    CREATION-ORDER parse fill, which both sides reproduce (the T9
//!    corpus rebuilds through reinsert instead — a different, deliberate
//!    shaping; the two gates are independent).
//! 2. Script derivation (a pure function of the parse, ids by rule —
//!    never iteration luck): `t1` = the FIRST `PolylineTrace` of the
//!    DESCENDING-id walk (the `Item.compareTo` order — the canonical
//!    text's order; Java sorts `getItems()` explicitly because its
//!    collection order is not trusted), `X` = the NEXT trace in that
//!    walk that is not deletion-forbidden. An all-protect board gives
//!    `X = null` and the script degenerates gracefully.
//! 3. Facts: the item count, `t1`'s layer/half-width/class/nets and
//!    first two corners, `X`'s id, and `s_id` — the id-generator
//!    watermark after the scripted insert, the dsn-0151 id-burn drift
//!    detector (a transient split/combine piece burns generator ids;
//!    und-t02's insert burns 3).
//! 4. Steps. Step 0 is the post-parse BASELINE (a divergence there is
//!    a READ bug, self-explaining). Then the op list —
//!    `snap`, `insert_trace`, `snap`, [`remove_item`], `normalize_all`,
//!    `query`, `undo`, `query`, `undo`, `redo`, `query`,
//!    `pop_snapshot`, `undo`, `redo`, `query`, `undo`, `query`,
//!    `pop_snapshot`, `undo` — the bracketed ops plus `query` only
//!    when the fixture carries a trace (`insert_trace`/`remove_item`
//!    additionally need `t1`/`X`):
//!    * `insert_trace`: a 2-corner polyline from `t1`'s first two
//!      corners, `t1`'s layer/width/nets/class, UNFIXED — through the
//!      CLEANING `BasicBoard.insertTrace` wrapper (insert + in-insert
//!      `normalize`; on und-t02 the inserted duplicate is consumed by
//!      its own normalize's combine, exercising remove-list restores).
//!    * `remove_item`: `X` through the repository path
//!      (`BasicBoard.removeItem` — forbidden-skip, tree remove, list
//!      delete).
//!    * `query` is the read-only op: the digest's `query` row IS the
//!      capture.
//!    * `undo`/`redo` capture the Java boolean return, the sorted
//!      changed nets, and the ORDERED cancelled/restored id lists
//!      (Java appends them: map order — descending id — for the swap
//!      phase, then delete-list order; an item may sit in BOTH lists).
//!      `ret=true` with EMPTY delta lists is normal (und-t03 step n=8).
//!    * `pop_snapshot` is the components asymmetry carrier (the item
//!      level drops, the components level does not — und-t01 n=11).
//! 5. Per-step digest: the item/components `stackLevel`s, `next_id`
//!    (`maxGeneratedId` — the id-burn channel), the live item count,
//!    the canonical-geometry sha (descending-id canon, D27's default
//!    tree dumped as `id:idx` pair SET — sorted, sha256 of the
//!    `"\n"`-join, full list only when ≤ 400 pairs), the focused
//!    canon lines (`t1`/`X`), and the corner query
//!    (`t1`'s corner(0) ± half-width, its layer, through
//!    `overlappingObjects` — the 2-arg form, `ignore_nets = []`).
//!    Every undo/redo is followed by a digest, and every trace-bearing
//!    run has a `query` after its undos — the D27 observability rule.
//!
//! ## Golden discipline
//!
//! Goldens live at `harness/corpus/undo-golden.jsonl` (committed;
//! regenerate ONLY via `undo golden`). `undo compare` re-evaluates
//! every fixture with the Rust port and diffs field-for-field,
//! reporting the first divergence per fixture — it never needs the
//! JVM (the CI gate runs it with `EPIC_SKIP_GRADLE=1`). The oracle
//! internally replays every fixture TWICE (real facade vs exposed
//! container calls + a verbatim `applyUndoRedoSideEffects` copy) and
//! demotes a disagreement to `result: "ABDiverge"`; a committed golden
//! that is not `"ok"` fails the compare (and the in-file pins).
//!
//! ## Known coverage boundary (mutation-tested, honest-negative)
//!
//! The gate KILLS the components-order-quirk mutant (dropping the
//! facade's `components.undo()` call diverges 33/33 at `comp_level`)
//! but does NOT gate the multi-element delta-list ORDER: across all
//! 231 undo/redo rows the derived script never accumulates more than
//! one item per list (the tier fixtures are dominated by protected
//! traces — only 2 of 33 even carry a removable victim), so a mutant
//! that re-SORTS the lists survives here. That order contract is
//! gated one layer down: the T2 container pins replay the jar
//! capture's multi-element rows verbatim
//! (`crates/epic-board/src/undo.rs`, S12/S13), and the facade-level
//! pins in [`epic_board::undo_facade`] drive multi-element synthetic
//! histories through `UndoOutcome` (delete-list restore order, the
//! both-lists case). The digest's `tree_sha` is a sorted `(id:idx)`
//! pair set over the DEFAULT tree only — tree TOPOLOGY and non-default
//! trees are not covered by the undo digest (the undo script touches
//! only the default tree; a non-default-tree mutant passes undo 33/33
//! while index compare fails — consistent today, blind if M3 adds a
//! second tree to the replay).

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use serde::{Deserialize, Serialize};

// The shared corpus shell (M3 Task 1): the manifest row, JSONL
// loading, alignment, diff rendering, sha-hex, and the tier-then-
// stressor walk live in corpus_common; this module keeps the undo
// golden record and the scripted replay protocol.
pub use crate::corpus_common::ManifestEntry;
use crate::corpus_common::{
    ensure_alignment, json_string, load_jsonl, manifest_bytes, sha256_hex,
    tier_then_stressor_paths, truncate,
};

use epic_board::board::Board;
use epic_board::id::ItemId;
use epic_board::items::{FixedState, ItemData};
use epic_board::normalize_all::normalize_all_traces;
use epic_board::trace_ops;
use epic_board::tree_manager::SearchTreeManager;
use epic_dsn::reader::{DsnReadResult, read_board};
use epic_geometry::int_box::IntBox;
use epic_geometry::int_point::IntPoint;
use epic_geometry::point::Point;
use epic_geometry::polyline::Polyline;
use epic_geometry::regular_tile_shape::RegularTileShape;
use epic_geometry::tile_shape::TileShape;

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum UndoCommand {
    /// Regenerate the undo-parity manifest (tier fixtures in
    /// tiers.yaml order, then the index-stress stressors
    /// lexicographic, dedup by path) and write it (deterministic
    /// byte-for-byte).
    Manifest {
        /// Output path for the manifest (relative paths resolve
        /// against the repo root).
        #[arg(long, default_value = "harness/corpus/undo-manifest.jsonl")]
        out: PathBuf,
    },
    /// Evaluate the manifest with the Java UndoOracle (ONE JVM per
    /// run) and write the goldens.
    Golden {
        #[arg(long, default_value = "harness/corpus/undo-manifest.jsonl")]
        manifest: PathBuf,
        #[arg(long, default_value = "harness/corpus/undo-golden.jsonl")]
        out: PathBuf,
    },
    /// Replay every manifest fixture with the Rust port and diff the
    /// record field-for-field against the committed golden
    /// (java-free, CI-able). Prints every differing field for at most
    /// 20 divergent fixtures in full; exits 1 on any.
    Compare {
        #[arg(long, default_value = "harness/corpus/undo-manifest.jsonl")]
        manifest: PathBuf,
        #[arg(long, default_value = "harness/corpus/undo-golden.jsonl")]
        golden: PathBuf,
    },
}

pub fn run(cmd: UndoCommand, jvm_xmx: &str) -> Result<()> {
    let repo_root = crate::oracle::find_repo_root()?;
    match cmd {
        UndoCommand::Manifest { out } => manifest(&repo_root, &out),
        UndoCommand::Golden { manifest, out } => golden(&repo_root, &manifest, &out, jvm_xmx),
        UndoCommand::Compare { manifest, golden } => compare(&repo_root, &manifest, &golden),
    }
}

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

// The manifest row is the shared `ManifestEntry` (re-exported above);
// repo-relative posix paths, same convention as the dsn/index corpora.
// The `und-NNNN` id scheme is this corpus's own.

/// Builds the manifest entries (pure function of the repository tree):
/// the tier A+B+C fixtures in tiers.yaml order, then every
/// `rust/harness/fixtures/index-stress/*.dsn` lexicographic, dedup by
/// path. Ids are `und-NNNN` in that order. The fixture SET is the
/// index corpus's (the same boards exercise the undo surface) — the
/// shared walk lives in corpus_common; only the id prefix differs.
pub fn build_manifest(repo_root: &Path) -> Result<Vec<ManifestEntry>> {
    Ok(tier_then_stressor_paths(repo_root)?
        .into_iter()
        .enumerate()
        .map(|(index, path)| ManifestEntry {
            id: format!("und-{:04}", index + 1),
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
// The golden record (mirrors UndoOracle.java's JSON, deny_unknown_fields)
// ---------------------------------------------------------------------------

/// The per-fixture facts (drift detectors + the script's own
/// constants, recorded so a golden can be read without re-running the
/// protocol).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UndoFacts {
    /// The live item count after the read (pre-mutation).
    pub items: i64,
    /// The first trace of the descending walk (`None` on a traceless
    /// board — the script degenerates to level probes).
    pub t_id: Option<i64>,
    pub t_layer: Option<i64>,
    pub t_hw: Option<i64>,
    pub t_cls: Option<i64>,
    /// `t1`'s first two corners, `"x y"` strings.
    pub t_c0: Option<String>,
    pub t_c1: Option<String>,
    /// `t1`'s nets in RAW stored order (never sorted — the nets are
    /// replayed verbatim into the insert).
    pub t_nets: Vec<i64>,
    /// The first non-deletion-forbidden trace after `t1`.
    pub x_id: Option<i64>,
    /// The id-generator watermark after the scripted insert (`None`
    /// when the fixture carries no trace) — the dsn-0151 id-burn
    /// drift detector.
    pub s_id: Option<i64>,
}

/// The per-step state digest (Java `Replay.digestBody`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DigestRecord {
    /// The item-list `stackLevel`.
    pub item_level: i64,
    /// The components-list `stackLevel` (the pop asymmetry witness).
    pub comp_level: i64,
    /// `maxGeneratedId` — the id-burn channel.
    pub next_id: i64,
    /// The live item count.
    pub item_count: i64,
    /// sha256 of the canonical geometry text (descending-id canon).
    pub geo_sha: String,
    /// Its line count (0 on an empty text).
    pub geo_lines: i64,
    /// The FULL canon lines of the focused ids (`t1`/`X`), canon order.
    pub geo_focused: Vec<String>,
    /// sha256 of the sorted `id:idx` default-tree pair set.
    pub tree_sha: String,
    /// The pair count.
    pub tree_n: i64,
    /// The pair set itself — full when ≤ 400 pairs, else `null` (the
    /// sha carries the content either way). Accepted cliff: above the
    /// threshold a first-divergence report degrades to sha-vs-sha
    /// (no field-level witness); emitting a first-N sample would
    /// change the digest schema on BOTH sides (oracle mirror + JVM
    /// golden recapture) for a diagnostic-only gain.
    pub tree_pairs: Option<Vec<String>>,
    /// The corner query's object ids (descending), `None` on a
    /// traceless fixture.
    pub query: Option<Vec<i64>>,
}

/// One replay step. `ret` is `None` on the baseline and on ops with
/// no boolean return (insert/query/snap). `ids_cancelled`/`ids_restored`
/// are the Java append ORDERS (never sorted — that order IS the
/// contract); `nets` is the changed-net set, sorted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepRecord {
    pub n: i64,
    pub op: String,
    pub ret: Option<bool>,
    pub ids_cancelled: Vec<i64>,
    pub ids_restored: Vec<i64>,
    pub nets: Vec<i64>,
    pub digest: DigestRecord,
}

/// One fixture's record — the oracle's JSON schema verbatim.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UndoRecord {
    pub id: String,
    pub file: String,
    /// `"ok"`, `"read-failed"`, `"EvalError"` (the Java Throwable
    /// demotion — e.g. a rational-corner fixture the script cannot
    /// cast), or `"ABDiverge"` (the oracle's internal facade-vs-
    /// container disagreement; a committed golden with this result
    /// fails the compare).
    pub result: String,
    pub facts: Option<UndoFacts>,
    pub steps: Option<Vec<StepRecord>>,
}

impl crate::corpus_common::HasId for UndoRecord {
    fn id(&self) -> &str {
        &self.id
    }
}

// ---------------------------------------------------------------------------
// The Rust replay (the protocol's Rust side)
// ---------------------------------------------------------------------------

/// The script's fixture-derived constants, captured once (both sides
/// derive them from the parse — never from iteration luck).
struct Script {
    t_id: Option<ItemId>,
    x_id: Option<ItemId>,
    /// `t1`'s first two corners, its layer/width/class/nets — the
    /// insert op's verbatim inputs.
    insert: Option<InsertPlan>,
}

struct InsertPlan {
    c0: (i32, i32),
    c1: (i32, i32),
    layer: i32,
    half_width: i32,
    clearance_class: i32,
    nets: Vec<i32>,
}

/// Derives the script: `t1` = the FIRST trace of the descending walk,
/// `X` = the next trace that is not deletion-forbidden. Fails only on
/// a non-integer corner — the exact failure class the Java oracle's
/// `(IntPoint)` cast demotes to an EvalError record.
fn derive_script(board: &Board) -> std::result::Result<Script, ()> {
    let mut t_id: Option<ItemId> = None;
    let mut x_id: Option<ItemId> = None;
    for entry in board.iter_descending() {
        if !matches!(entry.data, ItemData::Trace { .. }) {
            continue;
        }
        if t_id.is_none() {
            t_id = Some(entry.id);
            continue;
        }
        if x_id.is_none() && !trace_ops::is_deletion_forbidden(board, entry.id) {
            x_id = Some(entry.id);
            break;
        }
    }
    let insert = match t_id {
        None => None,
        Some(id) => {
            let entry = board.get(id).expect("t1 was just enumerated");
            let ItemData::Trace {
                layer,
                half_width,
                lines,
                ..
            } = &entry.data
            else {
                unreachable!("t1 is a trace by construction")
            };
            let corner = |index: i32| match lines.corner(index) {
                Some(Point::Int(p)) => Ok((p.x, p.y)),
                Some(other) => {
                    let _ = other;
                    Err(())
                }
                None => Err(()),
            };
            Some(InsertPlan {
                c0: corner(0)?,
                c1: corner(1)?,
                layer: *layer,
                half_width: *half_width,
                clearance_class: entry.clearance_class,
                nets: entry.nets.clone(),
            })
        }
    };
    Ok(Script { t_id, x_id, insert })
}

/// The digest's query constants: `t1`'s corner(0) ± half-width box and
/// its layer (fixed for the whole run — derived pre-mutation).
fn query_box(plan: &InsertPlan) -> (IntBox, i32) {
    let (cx, cy) = plan.c0;
    let hw = plan.half_width;
    (
        IntBox::new(
            IntPoint::new(cx - hw, cy - hw),
            IntPoint::new(cx + hw, cy + hw),
        ),
        plan.layer,
    )
}

/// The per-step digest. `focused` = `t1`/`X`; `qbox` = `None` on a
/// traceless fixture. Field order and normalization mirror the oracle
/// byte-for-byte.
fn digest(
    board: &mut Board,
    manager: &SearchTreeManager,
    string_quote: &str,
    focused: &[i64],
    qbox: Option<&(IntBox, i32)>,
) -> DigestRecord {
    let geo = crate::dsn_digest::canonical_geometry_text_board(board, string_quote);
    // Java: geo.isEmpty() ? 0 : split("\n", -1).length - 1 — the text
    // is "\n"-terminated per line, so this is the line count.
    let geo_lines = if geo.is_empty() {
        0
    } else {
        geo.split('\n').count() - 1
    };
    let geo_focused: Vec<String> = geo
        .split('\n')
        .filter(|line| {
            line.find(' ').is_some_and(|sp| {
                sp > 0
                    && line[sp + 1..]
                        .split(' ')
                        .next()
                        .is_some_and(|token| focused.iter().any(|id| token == id.to_string()))
            })
        })
        .map(str::to_string)
        .collect();

    // The D27 default-tree witness: the sorted `id:idx` pair SET
    // (Java TreeSet<String>), sha256 of the "\n"-join — the exact
    // twin of the oracle's `ShapeTree.toArray()` leaf walk.
    let mut pairs: BTreeSet<String> = BTreeSet::new();
    for leaf in manager.default_tree().min_area_tree().to_array() {
        pairs.insert(format!(
            "{}:{}",
            leaf.object_key, leaf.shape_index_in_object
        ));
    }
    let joined = pairs
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("\n");
    let tree_sha = sha256_hex(joined.as_bytes());

    let query = qbox.map(|(box_, layer)| {
        let shape = TileShape::RegularTileShape(RegularTileShape::IntBox(*box_));
        manager
            .overlapping_objects(
                board,
                SearchTreeManager::DEFAULT_TREE_INDEX,
                &shape,
                *layer,
                &[],
            )
            .into_iter()
            .map(|id| i64::from(id.get()))
            .collect::<Vec<_>>()
    });

    DigestRecord {
        item_level: i64::try_from(board.item_stack_level()).expect("levels are small"),
        comp_level: i64::try_from(board.components_stack_level()).expect("levels are small"),
        next_id: i64::from(board.max_generated_id()),
        item_count: i64::try_from(board.item_count()).expect("counts are small"),
        geo_sha: sha256_hex(geo.as_bytes()),
        geo_lines: i64::try_from(geo_lines).expect("counts are small"),
        geo_focused,
        tree_sha,
        tree_n: i64::try_from(pairs.len()).expect("counts are small"),
        tree_pairs: if pairs.len() <= 400 {
            Some(pairs.into_iter().collect())
        } else {
            None
        },
        query,
    }
}

/// Evaluates one fixture with the Rust port — the protocol's Rust
/// side, mirror of the oracle's `evaluateFixture`.
pub fn evaluate_rust(id: &str, path: &str, bytes: &[u8]) -> UndoRecord {
    let mut ses = epic_dsn::ses_board::SesBoard::new();
    if !matches!(read_board(bytes, &mut ses), DsnReadResult::Success { .. }) {
        return UndoRecord {
            id: id.to_string(),
            file: path.to_string(),
            result: "read-failed".to_string(),
            facts: None,
            steps: None,
        };
    }
    let string_quote = ses.metadata.string_quote.clone();
    let mut board = Board::from_ses_board(&ses);
    let mut manager = SearchTreeManager::new();
    manager.insert_items_creation_order(&mut board);
    // The in-read call (Wiring.java:343-353) — the oracle's parse has
    // already run it before the baseline digest.
    normalize_all_traces(&mut manager, &mut board);

    let script = match derive_script(&board) {
        Ok(script) => script,
        Err(()) => {
            // The `(IntPoint)` cast class — Java demotes the Throwable
            // to a full-null record; the port mirrors the demotion so
            // both sides agree on an undrivable fixture.
            return UndoRecord {
                id: id.to_string(),
                file: path.to_string(),
                result: "EvalError".to_string(),
                facts: None,
                steps: None,
            };
        }
    };

    let facts = UndoFacts {
        items: i64::try_from(board.item_count()).expect("counts are small"),
        t_id: script.t_id.map(|id| i64::from(id.get())),
        t_layer: script.insert.as_ref().map(|plan| i64::from(plan.layer)),
        t_hw: script
            .insert
            .as_ref()
            .map(|plan| i64::from(plan.half_width)),
        t_cls: script
            .insert
            .as_ref()
            .map(|plan| i64::from(plan.clearance_class)),
        t_c0: script.insert.as_ref().map(|plan| {
            let (x, y) = plan.c0;
            format!("{x} {y}")
        }),
        t_c1: script.insert.as_ref().map(|plan| {
            let (x, y) = plan.c1;
            format!("{x} {y}")
        }),
        t_nets: script
            .insert
            .as_ref()
            .map(|plan| {
                plan.nets
                    .iter()
                    .map(|net| i64::from(*net))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
        x_id: script.x_id.map(|id| i64::from(id.get())),
        // s_id is spliced in after the insert op runs (Java: the LAST
        // fact, emitted post-replay).
        s_id: None,
    };
    let mut facts = facts;

    // The focused canon-line ids and the fixed query constants.
    let mut focused: Vec<i64> = Vec::new();
    if let Some(id) = script.t_id {
        focused.push(i64::from(id.get()));
        if let Some(x) = script.x_id {
            focused.push(i64::from(x.get()));
        }
    }
    let qbox = script.insert.as_ref().map(query_box);

    let mut steps: Vec<StepRecord> = Vec::new();
    let mut emit = |n: i64,
                    op: &str,
                    ret: Option<bool>,
                    cancelled: Vec<i64>,
                    restored: Vec<i64>,
                    nets: Vec<i64>,
                    board: &mut Board,
                    manager: &SearchTreeManager| {
        steps.push(StepRecord {
            n,
            op: op.to_string(),
            ret,
            ids_cancelled: cancelled,
            ids_restored: restored,
            nets,
            digest: digest(board, manager, &string_quote, &focused, qbox.as_ref()),
        });
    };

    // Step 0: the post-parse baseline (a divergence here is a READ
    // bug, not an undo bug — it self-explains).
    emit(
        0,
        "baseline",
        None,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        &mut board,
        &manager,
    );

    // The op list (module docs) — the trace-conditional steps only
    // when the fixture carries a trace.
    let mut ops: Vec<Op> = vec![Op::Snap];
    if script.insert.is_some() {
        ops.push(Op::InsertTrace);
        ops.push(Op::Snap);
        if script.x_id.is_some() {
            ops.push(Op::RemoveItem);
        }
        ops.push(Op::NormalizeAll);
        ops.push(Op::Query);
    }
    ops.push(Op::Undo);
    if script.insert.is_some() {
        ops.push(Op::Query);
    }
    ops.push(Op::Undo);
    ops.push(Op::Redo);
    if script.insert.is_some() {
        ops.push(Op::Query);
    }
    ops.push(Op::PopSnapshot);
    ops.push(Op::Undo);
    ops.push(Op::Redo);
    if script.insert.is_some() {
        ops.push(Op::Query);
    }
    ops.push(Op::Undo);
    if script.insert.is_some() {
        ops.push(Op::Query);
    }
    ops.push(Op::PopSnapshot);
    ops.push(Op::Undo);

    enum Op {
        Snap,
        InsertTrace,
        RemoveItem,
        NormalizeAll,
        Query,
        PopSnapshot,
        Undo,
        Redo,
    }

    let mut s_id: Option<i64> = None;
    for (index, op) in ops.iter().enumerate() {
        let n = i64::try_from(index + 1).expect("counts are small");
        let mut ret = None;
        let mut cancelled: Vec<i64> = Vec::new();
        let mut restored: Vec<i64> = Vec::new();
        let mut nets: Vec<i64> = Vec::new();
        match op {
            Op::Snap => board.generate_snapshot(),
            Op::InsertTrace => {
                let plan = script.insert.as_ref().expect("insert op implies a plan");
                let (x0, y0) = plan.c0;
                let (x1, y1) = plan.c1;
                trace_ops::insert_trace(
                    &mut manager,
                    &mut board,
                    Polyline::from_points(&[
                        Point::Int(IntPoint::new(x0, y0)),
                        Point::Int(IntPoint::new(x1, y1)),
                    ]),
                    plan.layer,
                    plan.half_width,
                    &plan.nets,
                    plan.clearance_class,
                    FixedState::Unfixed,
                );
                s_id = Some(i64::from(board.max_generated_id()));
            }
            Op::RemoveItem => {
                let victim = script.x_id.expect("remove op implies a victim");
                trace_ops::remove_item_through_repository(&mut manager, &mut board, victim);
            }
            Op::NormalizeAll => ret = Some(normalize_all_traces(&mut manager, &mut board)),
            Op::Query => {}
            Op::PopSnapshot => ret = Some(board.pop_snapshot()),
            Op::Undo | Op::Redo => {
                let outcome = match op {
                    Op::Redo => board.redo(&mut manager),
                    _ => board.undo(&mut manager),
                };
                ret = Some(outcome.changed);
                cancelled = outcome
                    .cancelled
                    .iter()
                    .map(|id| i64::from(id.get()))
                    .collect();
                restored = outcome
                    .restored
                    .iter()
                    .map(|id| i64::from(id.get()))
                    .collect();
                nets = outcome
                    .changed_nets
                    .iter()
                    .map(|net| i64::from(*net))
                    .collect();
            }
        }
        emit(
            n,
            match op {
                Op::Snap => "snap",
                Op::InsertTrace => "insert_trace",
                Op::RemoveItem => "remove_item",
                Op::NormalizeAll => "normalize_all",
                Op::Query => "query",
                Op::PopSnapshot => "pop_snapshot",
                Op::Undo => "undo",
                Op::Redo => "redo",
            },
            ret,
            cancelled,
            restored,
            nets,
            &mut board,
            &manager,
        );
    }

    facts.s_id = s_id;
    UndoRecord {
        id: id.to_string(),
        file: path.to_string(),
        result: "ok".to_string(),
        facts: Some(facts),
        steps: Some(steps),
    }
}

// ---------------------------------------------------------------------------
// `undo golden` — one JVM per run, javac-compiled (the oracle imports
// the default-package DsnParseOracle; single-file source launcher
// rejects the package/path mismatch — index-corpus precedent).
// ---------------------------------------------------------------------------

fn golden(repo_root: &Path, manifest: &Path, out: &Path, jvm_xmx: &str) -> Result<()> {
    let started = Instant::now();
    let java = crate::oracle::resolve_java()?;
    let javac = java.with_file_name("javac");
    anyhow::ensure!(
        javac.is_file(),
        "javac not found next to {} — the JDK is required for the undo oracle",
        java.display()
    );
    let jar = crate::oracle::jar_path(repo_root);
    anyhow::ensure!(
        jar.is_file(),
        "oracle jar missing at {} — build it once with `./gradlew executableJar`",
        jar.display()
    );
    let oracle_src = repo_root.join("rust/harness/oracle/UndoOracle.java");
    let dsn_oracle_src = repo_root.join("rust/harness/oracle/DsnParseOracle.java");
    anyhow::ensure!(
        oracle_src.is_file(),
        "oracle evaluator missing at {}",
        oracle_src.display()
    );
    anyhow::ensure!(
        dsn_oracle_src.is_file(),
        "canonical-text oracle missing at {}",
        dsn_oracle_src.display()
    );
    let manifest_path = crate::dsn_corpus::resolve_input(repo_root, manifest);
    let entries = load_manifest(&manifest_path)?;
    anyhow::ensure!(
        !entries.is_empty(),
        "manifest {} is empty",
        manifest_path.display()
    );

    // Compile BOTH oracle sources together into a temp classes dir
    // (UndoOracle reads DsnParseOracle's canonical text in the same
    // default package). Every bail from here to the post-wait cleanup
    // goes through `cleanup_temps` (the T9 leak lesson).
    let classes_dir =
        std::env::temp_dir().join(format!("epic-undo-oracle-classes-{}", std::process::id()));
    let jvm_manifest =
        std::env::temp_dir().join(format!("epic-undo-manifest-{}.jsonl", std::process::id()));
    let stderr_path = std::env::temp_dir().join(format!(
        "epic-undo-oracle-stderr-{}.log",
        std::process::id()
    ));
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
        .arg(&dsn_oracle_src)
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
        // The oracle lives in the DEFAULT package (direct access to
        // DsnParseOracle + the public BasicBoard fields).
        .arg("UndoOracle")
        .arg(&jvm_manifest)
        .current_dir(repo_root)
        .stdout(std::process::Stdio::piped())
        .stderr(stderr_file)
        .spawn()
        .with_context(|| format!("spawning {} with the undo oracle", java.display()))
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
            "undo oracle failed with {status} (captured {}/{} result lines before failure):\n{}",
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
            "note: oracle stderr held {stderr_bytes} byte(s) of FRLogger noise / ABDiverge notes on a successful capture (discarded)"
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
        let record: UndoRecord = serde_json::from_str(line)
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
// `undo compare` — java-free, CI-able
// ---------------------------------------------------------------------------

/// Field-for-field diff: (field path, golden value, rust value) for
/// every differing field, in schema order. Nested sections report the
/// first differing INDEX and field (`steps[7].digest.geo_sha`).
pub fn diff_records(gold: &UndoRecord, rust: &UndoRecord) -> Vec<(String, String, String)> {
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
    match (&gold.facts, &rust.facts) {
        (Some(g), Some(r)) => {
            diff(&mut out, "facts.items", &g.items, &r.items);
            diff(&mut out, "facts.t_id", &g.t_id, &r.t_id);
            diff(&mut out, "facts.t_layer", &g.t_layer, &r.t_layer);
            diff(&mut out, "facts.t_hw", &g.t_hw, &r.t_hw);
            diff(&mut out, "facts.t_cls", &g.t_cls, &r.t_cls);
            diff(&mut out, "facts.t_c0", &g.t_c0, &r.t_c0);
            diff(&mut out, "facts.t_c1", &g.t_c1, &r.t_c1);
            diff(&mut out, "facts.t_nets", &g.t_nets, &r.t_nets);
            diff(&mut out, "facts.x_id", &g.x_id, &r.x_id);
            diff(&mut out, "facts.s_id", &g.s_id, &r.s_id);
        }
        (None, None) => {}
        (g, r) => out.push((
            "facts".to_string(),
            g.is_some().to_string(),
            r.is_some().to_string(),
        )),
    }

    match (&gold.steps, &rust.steps) {
        (Some(g), Some(r)) => {
            if g.len() != r.len() {
                out.push((
                    "steps.len".to_string(),
                    g.len().to_string(),
                    r.len().to_string(),
                ));
            }
            for (index, (gs, rs)) in g.iter().zip(r).enumerate() {
                let at = format!("steps[{index}]");
                diff(&mut out, &format!("{at}.n"), &gs.n, &rs.n);
                diff(&mut out, &format!("{at}.op"), &gs.op, &rs.op);
                diff(&mut out, &format!("{at}.ret"), &gs.ret, &rs.ret);
                diff(
                    &mut out,
                    &format!("{at}.ids_cancelled"),
                    &gs.ids_cancelled,
                    &rs.ids_cancelled,
                );
                diff(
                    &mut out,
                    &format!("{at}.ids_restored"),
                    &gs.ids_restored,
                    &rs.ids_restored,
                );
                diff(&mut out, &format!("{at}.nets"), &gs.nets, &rs.nets);
                let gd = &gs.digest;
                let rd = &rs.digest;
                let at = format!("{at}.digest");
                diff(
                    &mut out,
                    &format!("{at}.item_level"),
                    &gd.item_level,
                    &rd.item_level,
                );
                diff(
                    &mut out,
                    &format!("{at}.comp_level"),
                    &gd.comp_level,
                    &rd.comp_level,
                );
                diff(&mut out, &format!("{at}.next_id"), &gd.next_id, &rd.next_id);
                diff(
                    &mut out,
                    &format!("{at}.item_count"),
                    &gd.item_count,
                    &rd.item_count,
                );
                diff(&mut out, &format!("{at}.geo_sha"), &gd.geo_sha, &rd.geo_sha);
                diff(
                    &mut out,
                    &format!("{at}.geo_lines"),
                    &gd.geo_lines,
                    &rd.geo_lines,
                );
                diff(
                    &mut out,
                    &format!("{at}.geo_focused"),
                    &gd.geo_focused,
                    &rd.geo_focused,
                );
                diff(
                    &mut out,
                    &format!("{at}.tree_sha"),
                    &gd.tree_sha,
                    &rd.tree_sha,
                );
                diff(&mut out, &format!("{at}.tree_n"), &gd.tree_n, &rd.tree_n);
                // The Options are diffed DIRECTLY (T9 quality round
                // lesson): a some/none guard would make a nulled or
                // truncated golden `tree_pairs` a silent skip.
                diff(
                    &mut out,
                    &format!("{at}.tree_pairs"),
                    &gd.tree_pairs,
                    &rd.tree_pairs,
                );
                diff(&mut out, &format!("{at}.query"), &gd.query, &rd.query);
            }
        }
        (None, None) => {}
        (g, r) => out.push((
            "steps".to_string(),
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
    let records: Vec<UndoRecord> = load_jsonl(&golden_path, "golden")?;
    ensure_alignment(&manifest_path, &entries, &golden_path, &records)?;

    let mut mismatches = 0usize;
    let mut shown = 0usize;
    let mut census: BTreeMap<String, usize> = BTreeMap::new();
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
            "undo compare: {} fixture(s) identical in {:.1}s",
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
            "undo compare: {mismatches}/{} fixture(s) diverge (first-diff census: {census})",
            entries.len()
        );
    }
}

// ---------------------------------------------------------------------------
// Pins
// ---------------------------------------------------------------------------

#[cfg(test)]
mod pins {
    use super::*;

    /// The committed manifest is byte-stable against a rebuild (the
    /// corpus regen must be a no-op).
    #[test]
    fn committed_manifest_matches_rebuild() {
        let repo_root = crate::oracle::find_repo_root().expect("repo root");
        let committed = std::fs::read(repo_root.join("rust/harness/corpus/undo-manifest.jsonl"))
            .expect("committed undo manifest");
        assert_eq!(
            committed,
            build_manifest_bytes(&repo_root).expect("rebuild"),
            "undo-manifest.jsonl is stale — regenerate with `undo manifest`"
        );
    }

    /// Golden integrity: every committed record is `ok` (an ABDiverge
    /// or EvalError golden means the capture is not a parity baseline
    /// — it must fail HERE, at pin time, not silently gate), carries
    /// facts + steps, and opens with the baseline step.
    #[test]
    fn every_golden_record_is_ok_with_a_baseline_step() {
        let repo_root = crate::oracle::find_repo_root().expect("repo root");
        let raw = std::fs::read_to_string(repo_root.join("rust/harness/corpus/undo-golden.jsonl"))
            .expect("committed undo golden");
        let mut count = 0usize;
        for line in raw.lines().filter(|line| !line.trim().is_empty()) {
            count += 1;
            let record: UndoRecord = serde_json::from_str(line).expect("golden parses");
            assert_eq!(
                record.result, "ok",
                "{} result {}",
                record.id, record.result
            );
            let (facts, steps) = match (&record.facts, &record.steps) {
                (Some(facts), Some(steps)) => (facts, steps),
                _ => panic!("{} has null facts/steps", record.id),
            };
            assert_eq!(
                facts.s_id.is_some(),
                facts.t_id.is_some(),
                "{}: s_id present exactly when a trace was inserted",
                record.id
            );
            assert_eq!(steps[0].n, 0, "{} opens with the baseline", record.id);
            assert_eq!(steps[0].op, "baseline");
            // The D27 observability rule: every trace-bearing run has
            // a query step after its first undo, and undo/redo rows
            // carry the ordered delta lists.
            if facts.t_id.is_some() {
                let first_undo = steps
                    .iter()
                    .find(|step| step.op == "undo")
                    .expect("trace-bearing run has undos");
                assert!(
                    steps
                        .iter()
                        .any(|step| step.op == "query" && step.n > first_undo.n),
                    "{}: no query after the first undo (D27)",
                    record.id
                );
            }
            for step in steps.iter().skip(1) {
                if step.op == "undo" || step.op == "redo" {
                    assert!(
                        step.ret.is_some(),
                        "{} step {}: undo/redo rows carry a ret",
                        record.id,
                        step.n
                    );
                } else {
                    assert!(
                        step.ret.is_none()
                            || step.op == "normalize_all"
                            || step.op == "pop_snapshot",
                        "{} step {}: plain rows carry ret=null (got {:?})",
                        record.id,
                        step.n,
                        step.ret
                    );
                }
            }
        }
        assert!(count >= 30, "expected the full corpus, got {count}");
    }

    /// The replay is deterministic: evaluating one real fixture twice
    /// yields byte-identical records (the no-JVM half of the
    /// run-twice discipline). The fixture is the manifest's und-0028
    /// (`rust/harness/fixtures/index-stress/empty-tree.dsn`): small,
    /// and TRACE-BEARING, so the inserted-trace facts (`s_id`,
    /// focused canon lines, the corner query) are exercised too.
    #[test]
    fn evaluate_rust_is_deterministic() {
        let repo_root = crate::oracle::find_repo_root().expect("repo root");
        let path = "rust/harness/fixtures/index-stress/empty-tree.dsn";
        let bytes = std::fs::read(repo_root.join(path)).expect("fixture present");
        let first = evaluate_rust("und-det", path, &bytes);
        let second = evaluate_rust("und-det", path, &bytes);
        assert_eq!(
            serde_json::to_string(&first).expect("serializes"),
            serde_json::to_string(&second).expect("serializes")
        );
        assert_eq!(first.result, "ok");
        let facts = first.facts.expect("ok record carries facts");
        assert!(
            facts.t_id.is_some() && facts.s_id.is_some(),
            "the fixture is trace-bearing (insert facts live)"
        );
    }

    /// The digest's pair threshold: `tree_pairs` is full exactly when
    /// `tree_n <= 400` (the sha carries the content either way). TWO
    /// real manifest fixtures keep BOTH sides load-bearing: und-0028
    /// (`index-stress/empty-tree.dsn`, every step ≤ 400 — an
    /// always-None mutant dies here) and und-0001
    /// (`DAC2020_bm01.dsn`, traceless, every step > 400 — an
    /// always-Some mutant dies there).
    #[test]
    fn tree_pairs_threshold_contract() {
        let repo_root = crate::oracle::find_repo_root().expect("repo root");
        for (path, expect_some) in [
            ("rust/harness/fixtures/index-stress/empty-tree.dsn", true),
            (
                "scripts/benchmark/fixtures/DAC2020_boards/DAC2020_bm01.dsn",
                false,
            ),
        ] {
            let bytes = std::fs::read(repo_root.join(path)).expect("fixture present");
            let record = evaluate_rust("und-thresh", path, &bytes);
            let steps = record.steps.expect("ok record");
            for step in &steps {
                assert_eq!(
                    step.digest.tree_pairs.is_some(),
                    step.digest.tree_n <= 400,
                    "{path} step {}: tree_n {} vs pairs presence",
                    step.n,
                    step.digest.tree_n
                );
                if let Some(pairs) = &step.digest.tree_pairs {
                    assert_eq!(pairs.len() as i64, step.digest.tree_n);
                }
            }
            let all_some = steps.iter().all(|step| step.digest.tree_pairs.is_some());
            assert_eq!(all_some, expect_some, "{path}: the exercised side");
        }
    }
}
