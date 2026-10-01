//! Index parity corpus (M2 Task 9): the LANDED search-tree/index
//! surface of Tasks 5–8 (tree builds, tree structure, the overlap
//! query family, the replay script) driven against the frozen Java
//! oracle on the tier fixtures plus the crafted stressors, with
//! committed JSONL goldens and a java-free comparison.
//!
//! ## The protocol (BOTH sides run it identically — the Java oracle
//! `rust/harness/oracle/IndexOracle.java` and [`evaluate_rust`])
//!
//! Per fixture:
//!
//! 1. Parse to the live board (`DsnReader.readBoard` /
//!    `epic_dsn::read_board` + `Board::from_ses_board`).
//! 2. Scripted pre-steps by fixture name (currently only
//!    `drill-inflate.dsn` → `setHoleClearance(2500)`; recorded in
//!    `facts.pre`).
//! 3. `reinsertTreeItems()` — the uniform fill normalization. Java's
//!    read fills trees item-by-item ASCENDING; the port's rebuild
//!    paths are DESCENDING. Instead of mirroring read order, BOTH
//!    sides rebuild through the public `reinsertTreeItems()`
//!    (remove-all + insert-all descending) so the fill order is the
//!    identical code path on both sides — and the pre-step above
//!    becomes effective (reinsert recomputes every tree shape under
//!    the changed rules). This step deliberately REDEFINES the
//!    brief's "ctor state": the cc0 dump below is the tree as
//!    rebuilt by the shared reinsert path, not the raw post-parse
//!    fill — fill order is not part of the parity surface gated
//!    here (structure and queries are).
//! 4. Tree build sequence, dumping each tree's canonical structure:
//!    ctor default (cc0) → `setClearanceCompensationUsed(true)` (cc1)
//!    → `getAutorouteTree(2)` (cc2, variant follows the board's angle
//!    restriction). The dump is `MinAreaTree::dump_lines` — pre-order,
//!    `L obj=<item id> idx=<n> <bounds>` / `I <bounds>` — byte-equal
//!    between the sides (D19). Trees are identified by the canonical
//!    `key` (`ShapeSearchTree_FortyfiveDegree_cc1`-style), NEVER by
//!    the JVM identity counter. The delivered format deviates from
//!    the brief's literal in-order `L <key> <id> <idx> <bounds>`
//!    form: it is pre-order and ALSO emits `I <bounds>` inner-node
//!    lines — strictly more pinned structure (the split topology),
//!    byte-equal between the sides.
//! 5. Replay script (fixed, fixture-derived): `T` = the first trace
//!    by descending id (fallback: the first netted item — e.g. the
//!    via on `drill-inflate`), `N` = the first 3 items by descending
//!    id, `S` = a scripted 2-point trace at the bbox center (width =
//!    `hw`, empty net list, class 1 clamped to the class count — see
//!    the insert site, unfixed).
//!    Phases: `rmT` (manager-remove T), `insS` (insert S, dump its
//!    shapes per tree), `rmS` (manager-remove S, then
//!    `validate_entries` on the first 5 items that still HAVE
//!    entries).
//!    Id-burn hazard (dsn-0151): Java's read ends with
//!    `board.normalizeAllTraces()` (`Wiring.java:347`), which can
//!    SPLIT a trace at an interior same-net contact and burn an item
//!    id; the Rust reader defers normalization, so the sides' id
//!    generators can disagree on a fixture containing that construct.
//!    `facts.s_id` (the scripted trace's allocated id) is the drift
//!    detector: a future fixture hitting the hazard diverges as a
//!    self-explaining `facts.s_id` row instead of a cryptic
//!    `queries[i].e` mismatch that reads as an index bug. Shaping
//!    rule for new stressors: keep scripted wires clear of interior
//!    same-net contacts (no trace-pin crossings).
//! 6. Query set per tree (cc1 + cc2): the anchor's first/second
//!    non-null tree shapes (`own0`/`own1`), `own0` translated by
//!    (+hw, 0) / (0, −hw) (`xlate+hw`/`xlate-hw`), and the
//!    board-center box of side = the median trace full width
//!    (`ctrbox`), each against 6 ignore-net lists
//!    (`[]`, `[net_a]`, `[net_b]`, `[net_a, net_b]`, `[0]`, `[9999]`)
//!    through `overlapping_tree_entries` — order-exact
//!    `[(item_id, shape_idx)]`. Shape-count note: the set is
//!    anchor-derived, so it caps at 5 shapes (`own0`/`own1`/two
//!    translates/`ctrbox`); `own1` appears only when the anchor has
//!    ≥2 non-null shapes on that tree, and when none of the first-3
//!    descending items carries a shape the set degrades to `ctrbox`
//!    alone. At capture time 25 of 33 fixtures therefore sit BELOW
//!    the ≥5-shapes-per-tree target (structural, not a parity gap —
//!    both sides derive the identical set).
//! 7. `with-clearance`: the same shapes on the cc1 default tree at
//!    clearance classes {0, 1, 2} through the 5-arg CORE form, at
//!    `t_layer` (a REAL layer — the clearance windows read the
//!    matrix's per-layer row maxima, and a negative layer zeroes
//!    them, degenerating the with-clearance path to exact tests).
//! 8. `objects/items` family: the ctrbox's `overlapping_objects`
//!    ids on each queried tree (layer −1; ids are the entry query's
//!    objects DEDUPED and TreeSet-descending — the dedup/reorder is
//!    itself pinned) plus one `overlapping_items_with_clearance`
//!    row on the cc1 tree at S's scripted class 1 and `t_layer` —
//!    the DISPATCHING (T57) form; cc1 carries compensation so it
//!    takes the plain branch, complementing the core-form wc rows.
//! 9. `check_shape`: the center box + a deliberately-outside-bbox
//!    box at net lists {[], [net_a]} (class 1, `t_layer` — same
//!    convention).
//!
//! ## Golden discipline
//!
//! Goldens live at `harness/corpus/index-golden.jsonl` (committed;
//! regenerate ONLY via `index golden`). `index compare` re-evaluates
//! every fixture with the Rust port and diffs field-for-field,
//! reporting the first divergence per fixture — it never needs the
//! JVM (the CI gate runs it with `EPIC_SKIP_GRADLE=1`).

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// The shared corpus shell (M3 Task 1): the manifest row, JSONL
// loading, alignment, diff rendering, sha-hex, and the tier-then-
// stressor walk live in corpus_common; this module keeps the index
// golden record and the replay protocol.
pub use crate::corpus_common::ManifestEntry;
use crate::corpus_common::{
    ensure_alignment, hex, json_string, load_jsonl, manifest_bytes, tier_then_stressor_paths,
    truncate,
};

use epic_board::board::{Board, ItemEntry};
use epic_board::id::ItemId;
use epic_board::items::{Area, BoardShape, FixedState, ItemData};
use epic_board::tree_manager::SearchTreeManager;
use epic_dsn::reader::{DsnReadResult, read_board};
use epic_dsn::ses_board::SesBoard;
use epic_geometry::int_box::IntBox;
use epic_geometry::int_point::IntPoint;
use epic_geometry::point::Point;
use epic_geometry::polyline::Polyline;
use epic_geometry::tile_shape::TileShape;
use epic_geometry::vector::Vector;
use epic_index::TreeEntry;

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum IndexCommand {
    /// Regenerate the index-parity manifest (tier fixtures in
    /// tiers.yaml order, then the index-stress stressors
    /// lexicographic) and write it (deterministic byte-for-byte).
    Manifest {
        /// Output path for the manifest (relative paths resolve
        /// against the repo root's `rust/` checkout).
        #[arg(long, default_value = "harness/corpus/index-manifest.jsonl")]
        out: PathBuf,
    },
    /// Evaluate the manifest with the Java IndexOracle (ONE JVM per
    /// run, D14) and write the goldens.
    Golden {
        #[arg(long, default_value = "harness/corpus/index-manifest.jsonl")]
        manifest: PathBuf,
        #[arg(long, default_value = "harness/corpus/index-golden.jsonl")]
        out: PathBuf,
    },
    /// Replay every manifest fixture with the Rust port and diff the
    /// record field-for-field against the committed golden
    /// (java-free, CI-able). Prints every differing field for at most
    /// 20 divergent fixtures in full; exits 1 on any.
    Compare {
        #[arg(long, default_value = "harness/corpus/index-manifest.jsonl")]
        manifest: PathBuf,
        #[arg(long, default_value = "harness/corpus/index-golden.jsonl")]
        golden: PathBuf,
    },
}

pub fn run(cmd: IndexCommand, jvm_xmx: &str) -> Result<()> {
    let repo_root = crate::oracle::find_repo_root()?;
    match cmd {
        IndexCommand::Manifest { out } => manifest(&repo_root, &out),
        IndexCommand::Golden { manifest, out } => golden(&repo_root, &manifest, &out, jvm_xmx),
        IndexCommand::Compare { manifest, golden } => compare(&repo_root, &manifest, &golden),
    }
}

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

// The manifest row is the shared `ManifestEntry` (re-exported above);
// repo-relative posix paths, same convention as the dsn corpus. The
// `idx-NNNN` id scheme is this corpus's own.

/// Builds the manifest entries (pure function of the repository tree):
/// the tier A+B+C fixtures in tiers.yaml order, then every
/// `rust/harness/fixtures/index-stress/*.dsn` lexicographic, dedup by
/// path. Ids are `idx-NNNN` in that order.
pub fn build_manifest(repo_root: &Path) -> Result<Vec<ManifestEntry>> {
    Ok(tier_then_stressor_paths(repo_root)?
        .into_iter()
        .enumerate()
        .map(|(index, path)| ManifestEntry {
            id: format!("idx-{:04}", index + 1),
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

/// The per-fixture facts (drift detectors + the protocol's own
/// constants — `hw`, `net_a`, `s_layer` derivation — recorded so a
/// golden can be read without re-running the protocol).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FactsRecord {
    pub items: i64,
    pub layers: i64,
    /// Class names in matrix order (the IR's literal `"null"` for the
    /// unnamed class 0 — DsnParseOracle convention).
    pub classes: Vec<String>,
    pub bbox: Option<Vec<i64>>,
    /// `T` = the first trace by descending id (fallback: the first
    /// netted item — `t_kind` says which fired).
    pub t_id: Option<i64>,
    pub t_kind: Option<String>,
    pub t_layer: Option<i64>,
    pub t_hw: Option<i64>,
    pub t_net: Option<i64>,
    /// The first 3 items by descending id.
    pub n_ids: Vec<i64>,
    /// The replay constants: `hw` (T's half width, else 250) drives
    /// the translate offsets and S's width; `median_w` (the median
    /// trace HALF width) drives the center box's half side.
    pub hw: i64,
    pub median_w: i64,
    pub net_a: i64,
    pub net_b: i64,
    pub snap: String,
    /// The scripted pre-steps applied before the build (e.g.
    /// `"setHoleClearance=2500"`).
    pub pre: Vec<String>,
    /// The scripted trace S's ALLOCATED id — the id-generator drift
    /// detector for the dsn-0151 id-burn hazard (Java's
    /// normalizeAllTraces can burn ids at parse; the Rust reader
    /// defers): a disagreeing fixture diverges HERE, self-explaining,
    /// instead of as a cryptic `queries[i].e` mismatch.
    pub s_id: Option<i64>,
}

/// One tree dump: the canonical structure lines, digested (sha256 of
/// the `\n`-joined lines) + the first 40 lines verbatim + the FULL
/// lines when the tree is small (`n <= 200`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreeRecord {
    /// `cc0` (ctor default), `cc1` (post compensation rebuild) or
    /// `cc2` (the autoroute tree).
    pub phase: String,
    /// The canonical tree key (`ShapeSearchTree_FortyfiveDegree_cc1`).
    pub key: String,
    /// The TREE's `isClearanceCompensationUsed()` (cc0 false, cc1/cc2
    /// true by construction — recorded to pin the property itself).
    pub flag: bool,
    pub n: i64,
    pub sha256: String,
    pub head: Vec<String>,
    pub lines: Option<Vec<String>>,
}

/// The scripted item S's tree shapes, one record per queried tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SShapesRecord {
    pub key: String,
    /// `None` = a null slot (Java `getTreeShape` null — a shape index
    /// with no shape on this tree).
    pub shapes: Vec<Option<String>>,
}

/// One plain-query row, in one of the replay phases.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueryRecord {
    pub phase: String,
    pub key: String,
    pub tag: String,
    pub ig: Vec<i64>,
    /// The `(item_id, shape_idx)` pairs in RETURNED order (the
    /// `TreeSet<Leaf>` candidate order).
    pub e: Vec<(i64, i64)>,
}

/// One with-clearance core row (cc1 default tree only).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WcRecord {
    pub key: String,
    pub tag: String,
    pub ig: Vec<i64>,
    pub cls: i64,
    pub e: Vec<(i64, i64)>,
}

/// One objects/items-family row (`ob`: the ctrbox's
/// `overlapping_objects` ids per queried tree; `ic`: the single
/// `overlapping_items_with_clearance` row on cc1). The ids are the
/// entry query's objects DEDUPED and in `TreeSet` order (descending
/// item id) — deliberately NOT the raw entry order, so the
/// dedup+reorder step is itself pinned.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObRecord {
    pub key: String,
    pub e: Vec<i64>,
}

/// One `check_shape` row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CsRecord {
    pub tag: String,
    pub nets: Vec<i64>,
    pub ok: bool,
}

/// One `validate_entries` row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValRecord {
    pub id: i64,
    pub ok: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoldenRecord {
    pub id: String,
    pub file: String,
    pub result: String,
    pub facts: Option<FactsRecord>,
    pub trees: Option<Vec<TreeRecord>>,
    pub sshapes: Option<Vec<SShapesRecord>>,
    pub queries: Option<Vec<QueryRecord>>,
    pub wc: Option<Vec<WcRecord>>,
    pub ob: Option<Vec<ObRecord>>,
    pub ic: Option<Vec<ObRecord>>,
    pub cs: Option<Vec<CsRecord>>,
    pub val: Option<Vec<ValRecord>>,
}

impl crate::corpus_common::HasId for GoldenRecord {
    fn id(&self) -> &str {
        &self.id
    }
}

fn result_only(id: &str, path: &str, result: &str) -> GoldenRecord {
    GoldenRecord {
        id: id.to_string(),
        file: path.to_string(),
        result: result.to_string(),
        facts: None,
        trees: None,
        sshapes: None,
        queries: None,
        wc: None,
        ob: None,
        ic: None,
        cs: None,
        val: None,
    }
}

// ---------------------------------------------------------------------------
// The Rust protocol (the mirror of IndexOracle.java)
// ---------------------------------------------------------------------------

/// The scripted pre-steps by fixture FILE NAME (both sides — the
/// Java oracle switches on the same name).
pub fn pre_steps_for(file_name: &str) -> Vec<String> {
    match file_name {
        // drill-inflate: hole clearance is not DSN-expressible; the
        // T55 drill-hole inflation only fires once the rules carry a
        // hole clearance (applied BEFORE the reinsert normalization
        // so every tree shape is computed under it).
        "drill-inflate.dsn" => vec!["setHoleClearance=2500".to_string()],
        _ => Vec::new(),
    }
}

/// The center of an IntBox by Java integer division
/// (`ll + (ur - ll) / 2`) — the ctrbox corner convention.
fn box_center(b: &IntBox) -> (i32, i32) {
    (
        b.ll.x + (b.ur.x - b.ll.x) / 2,
        b.ll.y + (b.ur.y - b.ll.y) / 2,
    )
}

/// `fmt` of the spikes (Java `IndexOracle.fmt`): exact ints via field
/// access, the simplex rendered as `tile[oct[...]]` through its
/// bounding octagon.
pub fn fmt_shape(shape: &TileShape) -> Option<String> {
    match shape {
        TileShape::RegularTileShape(reg) => Some(epic_index::format_bounds(reg)),
        TileShape::Simplex(_) => shape.bounding_octagon().map(|oct| {
            format!(
                "tile[{}]",
                epic_index::format_bounds(
                    &epic_geometry::regular_tile_shape::RegularTileShape::IntOctagon(oct)
                )
            )
        }),
    }
}

/// The tree-digest triple of one tree: (line count, sha256, head 40).
pub fn dump_digest(lines: &[String]) -> (i64, String, Vec<String>) {
    let mut hasher = Sha256::new();
    for line in lines {
        hasher.update(line.as_bytes());
        hasher.update(b"\n");
    }
    let sha = hex(&hasher.finalize());
    (
        lines.len() as i64,
        sha,
        lines.iter().take(40).cloned().collect(),
    )
}

/// One (tag, shape) of the per-tree query set.
struct QShape {
    tag: &'static str,
    shape: TileShape,
}

/// The replay constants derived from the parsed board.
struct ReplayPlan {
    t_id: Option<ItemId>,
    t_layer: i32,
    hw: i32,
    median_w: i32,
    net_a: i32,
    net_b: i32,
    s_center: Option<(i32, i32)>,
    ctrbox: Option<TileShape>,
    n_ids: Vec<ItemId>,
}

fn plan_replay(board: &Board, classes_bbox: Option<&IntBox>) -> ReplayPlan {
    // T = the first trace by descending id, else the first netted
    // item (drill-inflate's via; tier boards without traces).
    let mut t: Option<(ItemId, i32, i32, i32)> = None; // (id, layer, hw, net)
    let mut fallback: Option<(ItemId, i32)> = None; // (id, net)
    let mut widths: Vec<i32> = Vec::new();
    for entry in board.iter_descending() {
        if let (
            None,
            ItemData::Trace {
                layer, half_width, ..
            },
        ) = (&t, &entry.data)
        {
            let net = entry.nets.iter().copied().find(|net| *net != 0);
            t = Some((entry.id, *layer, *half_width, net.unwrap_or(0)));
        }
        if let ItemData::Trace { half_width, .. } = &entry.data {
            widths.push(*half_width);
        }
        if fallback.is_none() && !entry.nets.is_empty() {
            let net = entry.nets.iter().copied().find(|net| *net != 0);
            fallback = Some((entry.id, net.unwrap_or(0)));
        }
    }
    let (t_id, t_hw, t_net) = match t {
        Some((id, _, hw, net)) => (Some(id), Some(hw), if net == 0 { None } else { Some(net) }),
        None => match fallback {
            Some((id, net)) => (Some(id), None, if net == 0 { None } else { Some(net) }),
            None => (None, None, None),
        },
    };
    let t_layer = t_id
        .and_then(|id| board.get(id))
        .and_then(|entry| match &entry.data {
            ItemData::Trace { layer, .. } => Some(*layer),
            _ => None,
        })
        .unwrap_or(0);

    let hw = t_hw.unwrap_or(250);
    widths.sort_unstable();
    let median_w = widths.get(widths.len() / 2).copied().unwrap_or(0);
    let net_a = t_net.unwrap_or(1);
    let mut net_b = net_a;
    'outer: for entry in board.iter_descending() {
        for net in &entry.nets {
            if *net != 0 && *net != net_a {
                net_b = *net;
                break 'outer;
            }
        }
    }

    let (s_center, ctrbox) = match classes_bbox {
        Some(bbox) => {
            let (cx, cy) = box_center(bbox);
            let ctr = TileShape::RegularTileShape(
                epic_geometry::regular_tile_shape::RegularTileShape::IntBox(IntBox::new(
                    IntPoint::new(cx - median_w, cy - median_w),
                    IntPoint::new(cx + median_w, cy + median_w),
                )),
            );
            (Some((cx, cy)), Some(ctr))
        }
        None => (None, None),
    };

    ReplayPlan {
        t_id,
        t_layer,
        hw,
        median_w,
        net_a,
        net_b,
        s_center,
        ctrbox,
        n_ids: board.iter_descending().take(3).map(|e| e.id).collect(),
    }
}

/// The per-tree query shapes: `own0`/`own1` from the first N item
/// with shapes on THIS tree, their translated variants, and the
/// shared ctrbox.
fn query_shapes(
    board: &mut Board,
    manager: &SearchTreeManager,
    tree_index: usize,
    plan: &ReplayPlan,
) -> Vec<QShape> {
    let (variant, cclass, object_id) = {
        let tree = &manager.trees()[tree_index];
        (
            tree.variant,
            tree.compensated_clearance_class,
            tree.object_id(),
        )
    };
    let mut out = Vec::new();
    for id in &plan.n_ids {
        let shapes = board.tree_shape_precalc(*id, object_id, variant, cclass);
        let present: Vec<usize> = shapes
            .iter()
            .enumerate()
            .filter_map(|(index, shape)| shape.as_ref().map(|_| index))
            .collect();
        if present.is_empty() {
            continue;
        }
        let own0 = shapes[present[0]]
            .clone()
            .expect("present index has a shape");
        out.push(QShape {
            tag: "own0",
            shape: own0.clone(),
        });
        if let Some(second) = present.get(1) {
            out.push(QShape {
                tag: "own1",
                shape: shapes[*second].clone().expect("present index has a shape"),
            });
        }
        out.push(QShape {
            tag: "xlate+hw",
            shape: own0.translate_by(&Vector::get_instance(plan.hw, 0)),
        });
        out.push(QShape {
            tag: "xlate-hw",
            shape: own0.translate_by(&Vector::get_instance(0, -plan.hw)),
        });
        break;
    }
    if let Some(ctr) = &plan.ctrbox {
        out.push(QShape {
            tag: "ctrbox",
            shape: ctr.clone(),
        });
    }
    out
}

fn entries_json(entries: &[TreeEntry]) -> Vec<(i64, i64)> {
    entries
        .iter()
        .map(|entry| {
            (
                i64::try_from(entry.object_key).unwrap_or(i64::MAX),
                i64::from(entry.shape_index_in_object),
            )
        })
        .collect()
}

/// Runs the full protocol on one fixture with the Rust port.
pub fn evaluate_rust(id: &str, path: &str, bytes: &[u8]) -> GoldenRecord {
    let mut ses = SesBoard::new();
    let read = read_board(bytes, &mut ses);
    let DsnReadResult::Success { warnings: _ } = read else {
        return result_only(id, path, "read-failed");
    };
    let mut board = Board::from_ses_board(&ses);
    let mut manager = SearchTreeManager::new();

    // Pre-steps + the uniform fill normalization (see module docs).
    for step in pre_steps_for(
        Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(""),
    ) {
        if step == "setHoleClearance=2500" {
            board.rules_mut().set_hole_clearance(2500);
        }
    }
    manager.reinsert_tree_items(&mut board);

    // Facts (before any mutation — T/N derivation is stateless).
    let class_names = board.rules().clearance.names.clone();
    let bbox = board.bounding_box();
    let plan = plan_replay(&board, bbox.as_ref());
    let mut facts = FactsRecord {
        items: board.item_count() as i64,
        layers: board.rules().clearance.layer_count() as i64,
        classes: class_names,
        bbox: bbox
            .as_ref()
            .map(|b| vec![b.ll.x as i64, b.ll.y as i64, b.ur.x as i64, b.ur.y as i64]),
        t_id: plan.t_id.map(|id| i64::from(id.get())),
        t_kind: plan.t_id.map(|_| {
            if plan
                .t_id
                .and_then(|id| board.get(id))
                .is_some_and(|entry| matches!(entry.data, ItemData::Trace { .. }))
            {
                "trace".to_string()
            } else {
                "item".to_string()
            }
        }),
        t_layer: if plan.t_id.is_some_and(|id| {
            board
                .get(id)
                .is_some_and(|e| matches!(e.data, ItemData::Trace { .. }))
        }) {
            Some(plan.t_layer as i64)
        } else {
            None
        },
        t_hw: plan
            .t_id
            .and_then(|id| board.get(id))
            .and_then(|entry| match &entry.data {
                ItemData::Trace { half_width, .. } => Some(*half_width as i64),
                _ => None,
            }),
        t_net: plan
            .t_id
            .and_then(|id| board.get(id))
            .and_then(|entry| entry.nets.iter().copied().find(|net| *net != 0))
            .map(i64::from),
        n_ids: plan.n_ids.iter().map(|id| i64::from(id.get())).collect(),
        hw: plan.hw as i64,
        median_w: plan.median_w as i64,
        net_a: i64::from(plan.net_a),
        net_b: i64::from(plan.net_b),
        snap: match board.rules().trace_angle_restriction {
            epic_board::rules_surf::AngleRestriction::None => "none",
            epic_board::rules_surf::AngleRestriction::FortyfiveDegree => "45",
            epic_board::rules_surf::AngleRestriction::NinetyDegree => "90",
        }
        .to_string(),
        pre: pre_steps_for(
            Path::new(path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(""),
        ),
        s_id: None,
    };

    // Tree build sequence: cc0 dump, compensation rebuild, cc1 dump,
    // autoroute, cc2 dump. Each tree's dump_lines() is computed ONCE
    // (quality round NIT: the digest and the stored lines shared a
    // second identical walk).
    let mut trees = Vec::new();
    let lines0 = manager.trees()[0].min_area_tree().dump_lines();
    let (n0, sha0, head0) = dump_digest(&lines0);
    trees.push(TreeRecord {
        phase: "cc0".to_string(),
        key: manager.trees()[0].key(),
        flag: manager.trees()[0].is_clearance_compensation_used(),
        n: n0,
        sha256: sha0,
        head: head0,
        lines: None,
    });
    if n0 <= 200 {
        trees[0].lines = Some(lines0);
    }
    manager.set_clearance_compensation_used(&mut board, true);
    let lines1 = manager.trees()[0].min_area_tree().dump_lines();
    let (n1, sha1, head1) = dump_digest(&lines1);
    trees.push(TreeRecord {
        phase: "cc1".to_string(),
        key: manager.trees()[0].key(),
        flag: manager.trees()[0].is_clearance_compensation_used(),
        n: n1,
        sha256: sha1,
        head: head1,
        lines: None,
    });
    if n1 <= 200 {
        trees[1].lines = Some(lines1);
    }
    let ar = manager.get_autoroute_tree(&mut board, 2);
    let lines2 = manager.trees()[ar].min_area_tree().dump_lines();
    let (n2, sha2, head2) = dump_digest(&lines2);
    trees.push(TreeRecord {
        phase: "cc2".to_string(),
        key: manager.trees()[ar].key(),
        flag: manager.trees()[ar].is_clearance_compensation_used(),
        n: n2,
        sha256: sha2,
        head: head2,
        lines: None,
    });
    if n2 <= 200 {
        trees[2].lines = Some(lines2);
    }

    // The query set per tree, computed once post-build.
    let query_trees = [
        (0usize, manager.trees()[0].key()),
        (ar, manager.trees()[ar].key()),
    ];
    let per_tree: Vec<(usize, String, Vec<QShape>)> = query_trees
        .iter()
        .map(|(index, key)| {
            (
                *index,
                key.clone(),
                query_shapes(&mut board, &manager, *index, &plan),
            )
        })
        .collect();
    let igs: Vec<Vec<i32>> = vec![
        vec![],
        vec![plan.net_a],
        vec![plan.net_b],
        vec![plan.net_a, plan.net_b],
        vec![0],
        vec![9999],
    ];

    let mut queries = Vec::new();
    let emit = |manager: &mut SearchTreeManager,
                board: &mut Board,
                phase: &str,
                tree_index: usize,
                key: &str,
                shapes: &[QShape],
                queries: &mut Vec<QueryRecord>| {
        for qshape in shapes {
            for ig in &igs {
                let entries =
                    manager.overlapping_tree_entries(board, tree_index, &qshape.shape, -1, ig);
                queries.push(QueryRecord {
                    phase: phase.to_string(),
                    key: key.to_string(),
                    tag: qshape.tag.to_string(),
                    ig: ig.iter().map(|net| i64::from(*net)).collect(),
                    e: entries_json(&entries),
                });
            }
        }
    };

    // Phase rmT.
    if let Some(t_id) = plan.t_id {
        manager.remove(&mut board, t_id);
        for (index, key, shapes) in &per_tree {
            emit(
                &mut manager,
                &mut board,
                "rmT",
                *index,
                key,
                shapes,
                &mut queries,
            );
        }
    }

    // Phase insS: the scripted trace at the bbox center.
    let mut sshapes = Vec::new();
    if let Some((cx, cy)) = plan.s_center {
        let s_id = board.alloc_id();
        // The id-burn drift detector (the module-doc hazard note): a
        // future fixture where the sides' id generators disagree shows
        // as a facts.s_id divergence, not a cryptic queries[i].e one.
        facts.s_id = Some(i64::from(s_id.get()));
        // Java's repository insert clamps an out-of-range clearance
        // class to 0 with a log-only warn (BoardItemRepository.java:
        // 146-153, reached through insertTraceWithoutCleaning); the
        // port's Board::insert_item is the raw arena insert with NO
        // clamp, so the harness applies the same clamp itself — on a
        // 1-class fixture the scripted class 1 lands at 0 on BOTH
        // sides instead of diverging loudly-and-misleadingly. Every
        // current fixture carries >= 2 classes (min observed 2), so
        // the clamp never fires on the committed corpus.
        let class_count = board.rules().clearance.names.len() as i32;
        let s_class = if class_count <= 1 { 0 } else { 1 };
        board.insert_item(ItemEntry {
            id: s_id,
            data: ItemData::Trace {
                layer: plan.t_layer,
                half_width: plan.hw,
                lines: Polyline::from_points(&[
                    Point::Int(IntPoint::new(cx - plan.hw, cy)),
                    Point::Int(IntPoint::new(cx + plan.hw, cy)),
                ]),
            },
            nets: Vec::new(),
            clearance_class: s_class,
            component_id: 0,
            fixed: FixedState::Unfixed,
            on_the_board: false,
        });
        manager.insert(&mut board, s_id);
        for (index, key, _) in &per_tree {
            let (variant, cclass, object_id) = {
                let tree = &manager.trees()[*index];
                (
                    tree.variant,
                    tree.compensated_clearance_class,
                    tree.object_id(),
                )
            };
            let shapes = board.tree_shape_precalc(s_id, object_id, variant, cclass);
            sshapes.push(SShapesRecord {
                key: key.clone(),
                shapes: shapes
                    .iter()
                    .map(|shape| shape.as_ref().and_then(fmt_shape))
                    .collect(),
            });
        }
        for (index, key, shapes) in &per_tree {
            emit(
                &mut manager,
                &mut board,
                "insS",
                *index,
                key,
                shapes,
                &mut queries,
            );
        }

        // Phase rmS.
        manager.remove(&mut board, s_id);
        for (index, key, shapes) in &per_tree {
            emit(
                &mut manager,
                &mut board,
                "rmS",
                *index,
                key,
                shapes,
                &mut queries,
            );
        }

        // with-clearance core: the cc1 default tree at classes {0,1,2}.
        // Layer s_layer, NOT -1: the clearance windows read the matrix's
        // per-layer row maxima, and a negative layer zeroes them (the
        // T56 trap would degenerate to exact tests).
        let mut wc = Vec::new();
        let (_, key, shapes) = &per_tree[0];
        for qshape in shapes {
            for cls in [0, 1, 2] {
                for ig in [&Vec::new(), &vec![plan.net_a]] {
                    let entries = manager.overlapping_tree_entries_with_clearance_core(
                        &mut board,
                        0,
                        &qshape.shape,
                        plan.t_layer,
                        ig,
                        cls,
                    );
                    wc.push(WcRecord {
                        key: key.clone(),
                        tag: qshape.tag.to_string(),
                        ig: ig.iter().map(|net| i64::from(*net)).collect(),
                        cls: i64::from(cls),
                        e: entries_json(&entries),
                    });
                }
            }
        }

        // objects/items family (the T8 surface): the ctrbox's OBJECTS
        // on each queried tree (layer −1 like the plain rows — the
        // ids are deduped + TreeSet-descending, not entry order) and
        // one DISPATCHING with-clearance ITEMS row on cc1 at S's
        // scripted class (cc1 carries compensation, so T57 routes
        // this to the plain branch — the wc rows above pin the 5-arg
        // core, this pins the dispatch).
        let mut ob = Vec::new();
        let mut ic = Vec::new();
        if let Some(ctr) = plan.ctrbox.as_ref() {
            for (index, key, _) in &per_tree {
                let ids = manager.overlapping_objects(&mut board, *index, ctr, -1, &[]);
                ob.push(ObRecord {
                    key: key.clone(),
                    e: ids.iter().map(|id| i64::from(id.get())).collect(),
                });
            }
            let ids =
                manager.overlapping_items_with_clearance(&mut board, 0, ctr, plan.t_layer, &[], 1);
            ic.push(ObRecord {
                key: per_tree[0].1.clone(),
                e: ids.iter().map(|id| i64::from(id.get())).collect(),
            });
        }

        // check_shape: the center box + a deliberately-outside box.
        let mut cs = Vec::new();
        if let Some(bbox) = board.bounding_box() {
            let outside = TileShape::RegularTileShape(
                epic_geometry::regular_tile_shape::RegularTileShape::IntBox(IntBox::new(
                    IntPoint::new(bbox.ur.x + 10_000, bbox.ur.y + 10_000),
                    IntPoint::new(bbox.ur.x + 11_000, bbox.ur.y + 11_000),
                )),
            );
            for (tag, shape) in [("ctrbox", plan.ctrbox.clone()), ("outside", Some(outside))] {
                let Some(shape) = shape else { continue };
                let area = Area {
                    border: BoardShape::Tile(shape),
                    holes: Vec::new(),
                };
                for nets in [&Vec::new(), &vec![plan.net_a]] {
                    // Same s_layer convention as wc: a real layer keeps
                    // the clearance windows alive in checkShape's core.
                    let ok = manager.check_shape(&mut board, &area, plan.t_layer, nets, 1);
                    cs.push(CsRecord {
                        tag: tag.to_string(),
                        nets: nets.iter().map(|net| i64::from(*net)).collect(),
                        ok,
                    });
                }
            }
        }

        // validate_entries on the first 5 items that still have
        // entries in the default tree (the absent-array case is the
        // documented Java-NPE divergence — excluded by design).
        let mut val = Vec::new();
        let default_object_id = manager.trees()[0].object_id();
        for entry in board.iter_descending() {
            if val.len() >= 5 {
                break;
            }
            if manager.tree_entries(entry.id, default_object_id).is_some() {
                val.push(ValRecord {
                    id: i64::from(entry.id.get()),
                    ok: manager.validate_entries(entry.id),
                });
            }
        }

        GoldenRecord {
            id: id.to_string(),
            file: path.to_string(),
            result: "ok".to_string(),
            facts: Some(facts),
            trees: Some(trees),
            sshapes: Some(sshapes),
            queries: Some(queries),
            wc: Some(wc),
            ob: Some(ob),
            ic: Some(ic),
            cs: Some(cs),
            val: Some(val),
        }
    } else {
        // No bbox → no scripted trace; the record carries the tree
        // facts only (no tier fixture hits this — every parsed board
        // has a bounding box).
        GoldenRecord {
            id: id.to_string(),
            file: path.to_string(),
            result: "no-bbox".to_string(),
            facts: Some(facts),
            trees: Some(trees),
            sshapes: None,
            queries: Some(queries),
            wc: None,
            ob: None,
            ic: None,
            cs: None,
            val: None,
        }
    }
}

// ---------------------------------------------------------------------------
// `index golden` — one JVM per run (D14), javac-compiled because the
// oracle declares a package (single-file source launcher rejects the
// package/path mismatch — MinAreaTreeSpike precedent).
// ---------------------------------------------------------------------------

fn golden(repo_root: &Path, manifest: &Path, out: &Path, jvm_xmx: &str) -> Result<()> {
    let started = Instant::now();
    let java = crate::oracle::resolve_java()?;
    let javac = java.with_file_name("javac");
    anyhow::ensure!(
        javac.is_file(),
        "javac not found next to {} — the JDK is required for the index oracle",
        java.display()
    );
    let jar = crate::oracle::jar_path(repo_root);
    anyhow::ensure!(
        jar.is_file(),
        "oracle jar missing at {} — build it once with `./gradlew executableJar`",
        jar.display()
    );
    let oracle_src = repo_root.join("rust/harness/oracle/IndexOracle.java");
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
    // `cleanup_temps` (quality round NIT: the javac-failure and
    // spawn-failure paths leaked the PID-namespaced temp dirs).
    let classes_dir =
        std::env::temp_dir().join(format!("epic-index-oracle-classes-{}", std::process::id()));
    let jvm_manifest =
        std::env::temp_dir().join(format!("epic-index-manifest-{}.jsonl", std::process::id()));
    let stderr_path = std::env::temp_dir().join(format!(
        "epic-index-oracle-stderr-{}.log",
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
        .arg("app.freerouting.datastructures.IndexOracle")
        .arg(&jvm_manifest)
        .current_dir(repo_root)
        .stdout(std::process::Stdio::piped())
        .stderr(stderr_file)
        .spawn()
        .with_context(|| format!("spawning {} with the index oracle", java.display()))
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
            "index oracle failed with {status} (captured {}/{} result lines before failure):\n{}",
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
// `index compare` — java-free, CI-able
// ---------------------------------------------------------------------------

/// Field-for-field diff: (field path, golden value, rust value) for
/// every differing field, in schema order. Nested sections report
/// the first differing INDEX and field (`queries[42].e`).
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
    // Per-field facts paths (quality round MINOR): the blob diff put
    // every drift detector (hw, median_w, net_a, net_b, snap, pre)
    // past the 160-char truncate — two visually identical strings.
    match (&gold.facts, &rust.facts) {
        (Some(g), Some(r)) => {
            diff(&mut out, "facts.items", &g.items, &r.items);
            diff(&mut out, "facts.layers", &g.layers, &r.layers);
            diff(&mut out, "facts.classes", &g.classes, &r.classes);
            diff(&mut out, "facts.bbox", &g.bbox, &r.bbox);
            diff(&mut out, "facts.t_id", &g.t_id, &r.t_id);
            diff(&mut out, "facts.t_kind", &g.t_kind, &r.t_kind);
            diff(&mut out, "facts.t_layer", &g.t_layer, &r.t_layer);
            diff(&mut out, "facts.t_hw", &g.t_hw, &r.t_hw);
            diff(&mut out, "facts.t_net", &g.t_net, &r.t_net);
            diff(&mut out, "facts.n_ids", &g.n_ids, &r.n_ids);
            diff(&mut out, "facts.hw", &g.hw, &r.hw);
            diff(&mut out, "facts.median_w", &g.median_w, &r.median_w);
            diff(&mut out, "facts.net_a", &g.net_a, &r.net_a);
            diff(&mut out, "facts.net_b", &g.net_b, &r.net_b);
            diff(&mut out, "facts.snap", &g.snap, &r.snap);
            diff(&mut out, "facts.pre", &g.pre, &r.pre);
            diff(&mut out, "facts.s_id", &g.s_id, &r.s_id);
        }
        (None, None) => {}
        (g, r) => out.push((
            "facts".to_string(),
            g.is_some().to_string(),
            r.is_some().to_string(),
        )),
    }

    match (&gold.trees, &rust.trees) {
        (Some(g), Some(r)) => {
            if g.len() != r.len() {
                out.push((
                    "trees.len".to_string(),
                    g.len().to_string(),
                    r.len().to_string(),
                ));
            }
            for (index, (gt, rt)) in g.iter().zip(r).enumerate() {
                let at = format!("trees[{index}]");
                diff(&mut out, &format!("{at}.phase"), &gt.phase, &rt.phase);
                diff(&mut out, &format!("{at}.key"), &gt.key, &rt.key);
                diff(&mut out, &format!("{at}.flag"), &gt.flag, &rt.flag);
                diff(&mut out, &format!("{at}.n"), &gt.n, &rt.n);
                diff(&mut out, &format!("{at}.sha256"), &gt.sha256, &rt.sha256);
                diff(&mut out, &format!("{at}.head"), &gt.head, &rt.head);
                // The Option is diffed DIRECTLY (quality round
                // IMPORTANT): the old some/some guard made a nulled or
                // truncated golden `lines` a silent skip — a proven
                // false PASS on committed-golden integrity, and
                // compare is the golden's only CI guard.
                diff(&mut out, &format!("{at}.lines"), &gt.lines, &rt.lines);
            }
        }
        (None, None) => {}
        (g, r) => out.push((
            "trees".to_string(),
            g.is_some().to_string(),
            r.is_some().to_string(),
        )),
    }
    match (&gold.sshapes, &rust.sshapes) {
        (Some(g), Some(r)) => {
            if g.len() != r.len() {
                out.push((
                    "sshapes.len".to_string(),
                    g.len().to_string(),
                    r.len().to_string(),
                ));
            }
            for (index, (gs, rs)) in g.iter().zip(r).enumerate() {
                diff(&mut out, &format!("sshapes[{index}].key"), &gs.key, &rs.key);
                diff(
                    &mut out,
                    &format!("sshapes[{index}].shapes"),
                    &gs.shapes,
                    &rs.shapes,
                );
            }
        }
        (None, None) => {}
        (g, r) => out.push((
            "sshapes".to_string(),
            g.is_some().to_string(),
            r.is_some().to_string(),
        )),
    }
    match (&gold.queries, &rust.queries) {
        (Some(g), Some(r)) => {
            if g.len() != r.len() {
                out.push((
                    "queries.len".to_string(),
                    g.len().to_string(),
                    r.len().to_string(),
                ));
            }
            for (index, (gq, rq)) in g.iter().zip(r).enumerate() {
                let at = format!("queries[{index}]");
                diff(&mut out, &format!("{at}.phase"), &gq.phase, &rq.phase);
                diff(&mut out, &format!("{at}.key"), &gq.key, &rq.key);
                diff(&mut out, &format!("{at}.tag"), &gq.tag, &rq.tag);
                diff(&mut out, &format!("{at}.ig"), &gq.ig, &rq.ig);
                diff(&mut out, &format!("{at}.e"), &gq.e, &rq.e);
            }
        }
        (None, None) => {}
        (g, r) => out.push((
            "queries".to_string(),
            g.is_some().to_string(),
            r.is_some().to_string(),
        )),
    }
    match (&gold.wc, &rust.wc) {
        (Some(g), Some(r)) => {
            if g.len() != r.len() {
                out.push((
                    "wc.len".to_string(),
                    g.len().to_string(),
                    r.len().to_string(),
                ));
            }
            for (index, (gw, rw)) in g.iter().zip(r).enumerate() {
                let at = format!("wc[{index}]");
                diff(&mut out, &format!("{at}.key"), &gw.key, &rw.key);
                diff(&mut out, &format!("{at}.tag"), &gw.tag, &rw.tag);
                diff(&mut out, &format!("{at}.ig"), &gw.ig, &rw.ig);
                diff(&mut out, &format!("{at}.cls"), &gw.cls, &rw.cls);
                diff(&mut out, &format!("{at}.e"), &gw.e, &rw.e);
            }
        }
        (None, None) => {}
        (g, r) => out.push((
            "wc".to_string(),
            g.is_some().to_string(),
            r.is_some().to_string(),
        )),
    }
    for (field, g, r) in [("ob", &gold.ob, &rust.ob), ("ic", &gold.ic, &rust.ic)] {
        match (g, r) {
            (Some(g), Some(r)) => {
                if g.len() != r.len() {
                    out.push((
                        format!("{field}.len"),
                        g.len().to_string(),
                        r.len().to_string(),
                    ));
                }
                for (index, (go, ro)) in g.iter().zip(r).enumerate() {
                    diff(&mut out, &format!("{field}[{index}].key"), &go.key, &ro.key);
                    diff(&mut out, &format!("{field}[{index}].e"), &go.e, &ro.e);
                }
            }
            (None, None) => {}
            (g, r) => out.push((
                field.to_string(),
                g.is_some().to_string(),
                r.is_some().to_string(),
            )),
        }
    }
    match (&gold.cs, &rust.cs) {
        (Some(g), Some(r)) => {
            if g.len() != r.len() {
                out.push((
                    "cs.len".to_string(),
                    g.len().to_string(),
                    r.len().to_string(),
                ));
            }
            for (index, (gc, rc)) in g.iter().zip(r).enumerate() {
                let at = format!("cs[{index}]");
                diff(&mut out, &format!("{at}.tag"), &gc.tag, &rc.tag);
                diff(&mut out, &format!("{at}.nets"), &gc.nets, &rc.nets);
                diff(&mut out, &format!("{at}.ok"), &gc.ok, &rc.ok);
            }
        }
        (None, None) => {}
        (g, r) => out.push((
            "cs".to_string(),
            g.is_some().to_string(),
            r.is_some().to_string(),
        )),
    }
    match (&gold.val, &rust.val) {
        (Some(g), Some(r)) => {
            if g.len() != r.len() {
                out.push((
                    "val.len".to_string(),
                    g.len().to_string(),
                    r.len().to_string(),
                ));
            }
            for (index, (gv, rv)) in g.iter().zip(r).enumerate() {
                let at = format!("val[{index}]");
                diff(&mut out, &format!("{at}.id"), &gv.id, &rv.id);
                diff(&mut out, &format!("{at}.ok"), &gv.ok, &rv.ok);
            }
        }
        (None, None) => {}
        (g, r) => out.push((
            "val".to_string(),
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
            "index compare: {} fixture(s) identical in {:.1}s",
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
            "index compare: {mismatches}/{} fixture(s) diverge (first-diff census: {census})",
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

    /// Every stressor must parse through the RUST reader without
    /// warnings AND convert to a live board (process rule 1 of the
    /// task brief — a stressor the Rust reader rejects is a broken
    /// fixture, not a reader bug).
    #[test]
    fn stressor_fixtures_parse_warning_free_through_the_rust_reader() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/index-stress");
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .expect("index-stress fixture dir exists")
            .map(|entry| {
                entry
                    .expect("dir entry readable")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .filter(|name| name.ends_with(".dsn"))
            .collect();
        names.sort();
        assert!(
            names.len() >= 10,
            "expected at least 10 stressors, found {}",
            names.len()
        );
        for name in &names {
            let bytes = std::fs::read(dir.join(name)).expect("stressor readable");
            let mut ses = SesBoard::new();
            match read_board(&bytes, &mut ses) {
                DsnReadResult::Success { warnings } => {
                    assert!(
                        warnings.is_empty(),
                        "stressor {name} parsed with warnings: {warnings:?}"
                    );
                    let _board = Board::from_ses_board(&ses);
                }
                other => panic!("stressor {name} failed to parse: {other:?}"),
            }
        }
    }

    /// The manifest is a pure function of the tree: two builds are
    /// byte-identical, the tier fixtures come first in tiers.yaml
    /// order, and the stressors follow lexicographically.
    #[test]
    fn manifest_build_is_deterministic_and_orders_stressors_last() {
        let root = crate::oracle::find_repo_root().expect("repo root");
        let first = build_manifest_bytes(&root).expect("manifest builds");
        let second = build_manifest_bytes(&root).expect("manifest builds again");
        assert_eq!(
            first, second,
            "manifest regeneration must be byte-identical"
        );
        let text = String::from_utf8(first).expect("manifest is utf-8");
        let paths: Vec<String> = text
            .lines()
            .filter_map(|line| serde_json::from_str::<ManifestEntry>(line).ok())
            .map(|entry| {
                assert!(entry.id.starts_with("idx-"), "id scheme: {}", entry.id);
                entry.path
            })
            .collect();
        assert!(paths.len() >= 33, "23 tier fixtures + 10 stressors minimum");
        // Stressors are exactly the 10, sorted, and LAST.
        let stress_count = paths.iter().filter(|p| p.contains("index-stress")).count();
        assert_eq!(stress_count, 10, "exactly the 10 stressors");
        let tail: Vec<&String> = paths.iter().skip(paths.len() - stress_count).collect();
        assert!(
            tail.iter().all(|p| p.contains("index-stress")),
            "stressors are the tail: {tail:?}"
        );
        let mut sorted_tail: Vec<&String> = tail.clone();
        sorted_tail.sort();
        assert_eq!(sorted_tail, tail, "stressors are the sorted tail");
        // Tier fixtures keep tiers.yaml order (first entry is tier A's
        // first fixture — spot-check it is NOT a stressor path).
        assert!(!paths[0].contains("index-stress"));
    }

    /// The digest triple of a known dump: line count, sha256 of the
    /// `\n`-joined lines, head capped at 40.
    #[test]
    fn dump_digest_pins_count_sha_and_head_cap() {
        let lines: Vec<String> = (0..55).map(|i| format!("line-{i}")).collect();
        let (n, sha, head) = dump_digest(&lines);
        assert_eq!(n, 55);
        assert_eq!(head.len(), 40);
        assert_eq!(head[0], "line-0");
        assert_eq!(head[39], "line-39");
        let mut hasher = Sha256::new();
        for line in &lines {
            hasher.update(line.as_bytes());
            hasher.update(b"\n");
        }
        assert_eq!(sha, hex(&hasher.finalize()));
        let (n2, _, head2) = dump_digest(&lines[..3]);
        assert_eq!(n2, 3);
        assert_eq!(head2.len(), 3);
    }

    /// ctrbox math: Java integer division on the bbox center, the
    /// box grown by the median half width on every side.
    #[test]
    fn box_center_uses_java_integer_division() {
        let b = IntBox::new(IntPoint::new(0, 1), IntPoint::new(100_001, 60_003));
        assert_eq!(box_center(&b), (50_000, 30_002));
        let odd = IntBox::new(IntPoint::new(-3, -4), IntPoint::new(4, 5));
        // (-3 + 7/2, -4 + 9/2) — truncation toward zero like Java.
        assert_eq!(box_center(&odd), (0, 0));
        // The discriminating anchor (quality round MINOR-4: the two
        // above agree with the naive (ll+ur)/2): sum negative, odd
        // span — naive (-1001+1000)/2 = -1/2 = 0 (Java truncates
        // toward zero), the ported ll + (ur-ll)/2 = -1001 + 2001/2
        // = -1.
        let neg = IntBox::new(IntPoint::new(-1001, -1001), IntPoint::new(1000, 1000));
        assert_eq!(box_center(&neg), (-1, -1));
    }

    /// Pre-steps are keyed by fixture FILE name only.
    #[test]
    fn pre_steps_are_keyed_by_fixture_name() {
        assert_eq!(
            pre_steps_for("drill-inflate.dsn"),
            vec!["setHoleClearance=2500"]
        );
        assert!(pre_steps_for("sym-tie.dsn").is_empty());
        assert!(pre_steps_for("sub/dir/drill-inflate.dsn").is_empty());
    }

    /// The diff reports the first diverging section with an indexed
    /// field path (`queries[2].e`), the shape the CI gate prints.
    #[test]
    fn diff_reports_indexed_query_paths() {
        let gold = GoldenRecord {
            id: "idx-0001".into(),
            file: "f.dsn".into(),
            result: "ok".into(),
            facts: None,
            trees: None,
            sshapes: None,
            queries: Some(vec![
                QueryRecord {
                    phase: "rmT".into(),
                    key: "k".into(),
                    tag: "own0".into(),
                    ig: vec![1],
                    e: vec![(4, 0), (3, 0)],
                },
                QueryRecord {
                    phase: "rmT".into(),
                    key: "k".into(),
                    tag: "own1".into(),
                    ig: vec![],
                    e: vec![],
                },
            ]),
            wc: None,
            ob: None,
            ic: None,
            cs: None,
            val: None,
        };
        let mut rust = gold.clone();
        if let Some(queries) = &mut rust.queries {
            queries[1].e = vec![(9, 9)];
        }
        let diffs = diff_records(&gold, &rust);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].0, "queries[1].e");
        assert!(diffs[0].1.contains("[]"));
        assert!(diffs[0].2.contains("[[9,9]]"));
    }

    /// Golden lines round-trip: parse → serialize is byte-stable
    /// (field order is the struct order both sides agreed on).
    #[test]
    fn golden_record_round_trips_byte_stable() {
        let line = r#"{"id":"idx-0001","file":"f.dsn","result":"ok","facts":null,"trees":null,"sshapes":null,"queries":null,"wc":null,"ob":null,"ic":null,"cs":null,"val":null}"#;
        let record: GoldenRecord = serde_json::from_str(line).expect("parses");
        assert_eq!(
            serde_json::to_string(&record).expect("serializes"),
            line,
            "field order must match the Java emitter"
        );
    }

    // -----------------------------------------------------------------
    // Committed-artifact guard pins (quality round)
    // -----------------------------------------------------------------

    /// Loads the committed golden records (strict per line; a line
    /// that fails to parse is SKIPPED so a single malformed record
    /// reports as the missing fixture below, not a test panic here).
    fn committed_golden() -> Vec<GoldenRecord> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus/index-golden.jsonl");
        std::fs::read_to_string(&path)
            .expect("committed golden readable")
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }

    fn golden_for(name: &str) -> GoldenRecord {
        committed_golden()
            .into_iter()
            .find(|record| record.file.ends_with(name))
            .unwrap_or_else(|| panic!("no committed golden for {name}"))
    }

    fn tree_ns(record: &GoldenRecord) -> Vec<i64> {
        record
            .trees
            .as_ref()
            .expect("trees")
            .iter()
            .map(|tree| tree.n)
            .collect()
    }

    fn query_row(
        record: &GoldenRecord,
        phase: &str,
        tag: &str,
        key_suffix: &str,
        ig: &[i64],
    ) -> QueryRecord {
        record
            .queries
            .as_ref()
            .and_then(|queries| {
                queries.iter().find(|q| {
                    q.phase == phase && q.tag == tag && q.key.ends_with(key_suffix) && q.ig == ig
                })
            })
            .cloned()
            .unwrap_or_else(|| panic!("no {phase}/{tag}/{key_suffix}/ig={ig:?} row"))
    }

    /// IMPORTANT (quality round): a committed golden whose `lines`
    /// field is nulled must be REPORTED — compare is the golden's only
    /// CI guard, and the old some/some guard made the null a silent
    /// skip (the reviewer proved a false PASS by nulling idx-0005's
    /// 103-line array).
    #[test]
    fn a_nulled_lines_field_is_reported_as_a_divergence() {
        let mut gold = committed_golden()
            .into_iter()
            .find(|record| {
                record
                    .trees
                    .as_ref()
                    .is_some_and(|trees| trees.iter().any(|tree| tree.lines.is_some()))
            })
            .expect("a golden record with committed lines");
        let rust = gold.clone();
        let index = gold
            .trees
            .as_ref()
            .expect("trees")
            .iter()
            .position(|tree| tree.lines.is_some())
            .expect("a lines-bearing tree");
        gold.trees.as_mut().expect("trees")[index].lines = None;
        let diffs = diff_records(&gold, &rust);
        assert!(
            diffs
                .iter()
                .any(|(field, _, _)| field == &format!("trees[{index}].lines")),
            "the nulled lines must surface as trees[{index}].lines, got {diffs:?}"
        );
    }

    /// MINOR-2 (quality round): the COMMITTED manifest is fresh — a
    /// fixture added to tiers.yaml or the stressor dir without
    /// `index manifest` + `index golden` would otherwise be silently
    /// ungated by CI (dsn-corpus convention, `dsn_corpus.rs`).
    #[test]
    fn committed_manifest_regenerates_byte_identical() {
        let root = crate::oracle::find_repo_root().expect("repo root");
        let committed = std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus/index-manifest.jsonl"),
        )
        .expect("committed index manifest");
        let regenerated = build_manifest_bytes(&root).expect("manifest regeneration");
        assert_eq!(
            regenerated, committed,
            "index manifest regeneration drifted from the committed file — run \
             `epic-harness index manifest` then `index golden`"
        );
    }

    /// MINOR-7 (quality round): the crafted-trap TREE facts, asserted
    /// against the committed golden — a legitimate golden regen that
    /// accidentally loses a trap fails HERE instead of silently
    /// keeping CI green. Values verified anchor-blind in the Task 9
    /// fix round and re-extracted from the committed goldens.
    #[test]
    fn committed_golden_holds_the_tree_fact_traps() {
        // drill-inflate: the scripted hole clearance + the
        // hole-inflated shape count.
        let record = golden_for("drill-inflate.dsn");
        assert_eq!(
            record.facts.as_ref().expect("facts").pre,
            vec!["setHoleClearance=2500"]
        );
        assert_eq!(tree_ns(&record), vec![39, 39, 39]);

        // sectioning: only the compensated cc1 tree splits the grid.
        let record = golden_for("sectioning.dsn");
        assert_eq!(tree_ns(&record), vec![21, 25, 21]);

        // asym-classes: 4 classes and per-class compensation changes
        // the dump.
        let record = golden_for("asym-classes.dsn");
        assert_eq!(
            record.facts.as_ref().expect("facts").classes,
            vec!["null", "default", "CB", "CC"]
        );
        let trees = record.trees.expect("trees");
        assert_ne!(
            trees[0].sha256, trees[1].sha256,
            "cc0 vs cc1 must differ under asymmetric classes"
        );

        // deg90: the autoroute tree switches to the 90-degree variant.
        let record = golden_for("deg90.dsn");
        assert_eq!(
            record.trees.expect("trees")[2].key,
            "ShapeSearchTree90Degree_Orthogonal_cc2"
        );

        // sym-tie: BOTH mirrored equal-area keepout leaves (obj=2 and
        // obj=3) in every dump, in the pinned tie order (the trace
        // first, keepout 3, the outline windows, keepout 2 last).
        let record = golden_for("sym-tie.dsn");
        for tree in record.trees.expect("trees") {
            let lines = tree.lines.expect("sym-tie is small enough for full lines");
            let leaf = |obj: i64| {
                lines
                    .iter()
                    .any(|line| line.trim_start().starts_with(&format!("L obj={obj} idx=")))
            };
            assert!(leaf(2), "{} misses the obj=2 keepout", tree.phase);
            assert!(leaf(3), "{} misses the obj=3 keepout", tree.phase);
            let order: Vec<i64> = lines
                .iter()
                .filter(|line| line.trim_start().starts_with("L obj="))
                .map(|line| {
                    line.trim_start()["L obj=".len()..]
                        .split_whitespace()
                        .next()
                        .and_then(|id| id.parse().ok())
                        .unwrap_or(-1)
                })
                .collect();
            assert_eq!(&order[..2], &[4, 3], "{} tie order", tree.phase);
        }
    }

    /// MINOR-7 (quality round): the crafted-trap QUERY rows, against
    /// the committed golden (same rationale as the tree-fact traps).
    #[test]
    fn committed_golden_holds_the_query_row_traps() {
        // stale-bounds: the ctrbox window opens only while S is live
        // (0/12 -> 12/12 -> 0/12 across the phases).
        let record = golden_for("stale-bounds.dsn");
        for (phase, expect) in [("rmT", 0), ("insS", 12), ("rmS", 0)] {
            let rows: Vec<&QueryRecord> = record
                .queries
                .as_ref()
                .expect("queries")
                .iter()
                .filter(|q| q.phase == phase && q.tag == "ctrbox")
                .collect();
            assert_eq!(rows.len(), 12, "{phase} ctrbox rows (2 trees x 6 igs)");
            assert_eq!(
                rows.iter().filter(|q| !q.e.is_empty()).count(),
                expect,
                "{phase} non-empty ctrbox rows"
            );
        }

        // oct-touch: the exact 45-degree boundary contact — own0
        // INCLUDED, xlate-hw EXCLUDED, xlate+hw INCLUDED (cc2, rmT,
        // no ignore nets).
        let record = golden_for("oct-touch.dsn");
        assert_eq!(
            query_row(&record, "rmT", "own0", "_cc2", &[]).e,
            vec![(2, 0)]
        );
        assert_eq!(
            query_row(&record, "rmT", "xlate-hw", "_cc2", &[]).e,
            Vec::<(i64, i64)>::new()
        );
        assert_eq!(
            query_row(&record, "rmT", "xlate+hw", "_cc2", &[]).e,
            vec![(2, 0)]
        );

        // multi-net: the ANY-net ignore semantics, both directions —
        // the 2-net pin survives exactly when NO ignored net is
        // carried (net 0 never ignores).
        let record = golden_for("multi-net.dsn");
        for (ig, expected) in [
            (vec![], vec![(2, 0)]),
            (vec![0], vec![(2, 0)]),
            (vec![9999], vec![(2, 0)]),
            (vec![2], Vec::<(i64, i64)>::new()),
            (vec![1], Vec::<(i64, i64)>::new()),
            (vec![2, 1], Vec::<(i64, i64)>::new()),
        ] {
            assert_eq!(
                query_row(&record, "rmT", "ctrbox", "_cc1", &ig).e,
                expected,
                "ig {ig:?}"
            );
        }

        // clear-tie: the equal-clearance keepouts tie in descending
        // order at class 1; classes 0 and 2 are the empty contrast.
        let record = golden_for("clear-tie.dsn");
        for row in record.wc.expect("wc") {
            if row.tag != "ctrbox" {
                continue;
            }
            let expected = if row.cls == 1 {
                vec![(3, 0), (2, 0)]
            } else {
                Vec::new()
            };
            assert_eq!(row.e, expected, "cls {} ig {:?}", row.cls, row.ig);
        }

        // empty-tree: the ONLY non-empty rows are insS x ctrbox (the
        // scripted trace found through the emptied tree).
        let record = golden_for("empty-tree.dsn");
        let queries = record.queries.expect("queries");
        let non_empty = queries.iter().filter(|q| !q.e.is_empty()).count();
        assert_eq!(non_empty, 12, "exactly the 12 insS x ctrbox rows");
        for q in &queries {
            assert_eq!(
                !q.e.is_empty(),
                q.phase == "insS" && q.tag == "ctrbox",
                "phase {} tag {} key {}",
                q.phase,
                q.tag,
                q.key
            );
        }
    }
}
