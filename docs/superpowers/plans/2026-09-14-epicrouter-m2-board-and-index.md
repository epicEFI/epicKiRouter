# EpicRouter M2: Board Model + Spatial Index — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Port the live board model (`epic-board`: items, ids, undo, rules, normalize) and the spatial index (`epic-index`: the `MinAreaTree`/`ShapeSearchTree` family), proven by three new differential gates against the frozen Java jar — **index query/tree parity**, **snapshot/undo parity**, and **post-normalize parse parity** (which retires the dsn-0151 ledger entry) — plus the deferred M1b item T40 endpoint snapping.

**Architecture:** Transliteration of `board/` (~12.5k Java LOC: facade+model+trace+state) and `datastructures/{ShapeTree,MinAreaTree,UndoableObjects}.java` + `board/searchtree/` (~4.1k LOC) into two crates: `epic-index` (pure tree — depends on `epic-geometry` only) and `epic-board` (items/rules/undo/shape-inflation/tree management — depends on `epic-geometry` + `epic-index` + `epic-dsn`). The parse-time `SesBoard` IR (M1b) converts one-shot into the real model via `Board::from_ses_board`; normalization and contacts are board-level machinery the harness drives for parity. Router-only algorithms (`completeShape` ×3, `ShapeTraceEntries`, trace-surgery entry reuse) are **deferred to M3** (D18).

**Tech Stack:** Rust 1.93 workspace; oracles stay single-file Java launchers under `rust/harness/oracle/` (batched JSONL, one JVM, D14 conventions), goldens committed under `rust/harness/corpus/`, compares java-free in CI.

**Design doc:** `docs/superpowers/specs/2026-09-11-epicrouter-rust-rewrite-design.md` §4.1 (workspace), §5 (parity gates: "Index parity: same insert/remove sequence → identical query result sets vs Java tree"), §6 (M2 row: "Index query-set parity; snapshot/undo correct").

**Standing constraints (every task):**
- Never modify Java sources under `src/` or `src_v19/`. Never touch other Claude instances or global `~/.claude`. Never push to any remote.
- Commits on `epic/main`, trailer `Co-Authored-By: Claude Code <noreply@anthropic.com>`.
- Oracle launchers live in `rust/harness/oracle/` — never `src/`. An oracle source file MAY declare an `app.freerouting.*` package for package-private access (single-file source launch honor package declarations).
- `cargo clippy --workspace --all-targets -- -D warnings` green (incl. `clippy::unwrap_used`; tests use `.expect()`).
- Run cargo from `rust/`; run git from the repo root.
- Oracle pins are jar-captured, never memory-reconstructed; non-degenerate; branch-executing (cerebrum rule, 2026-09-12).
- The M1b gates stay green throughout: `corpus compare` (5000), `dsn compare` (1,332), `dsn ses-compare` (20).

---

## Ground truth from recon (established 2026-09-14; do not re-derive)

### board/ at a glance

`src/main/java/app/freerouting/board/` = 21,711 LOC total: `facade` 3888, `model` 5824, `trace` 1581, `actions` 1798, `state` 1196, `optimize` 4002, `searchtree` 3422. M2 ports facade+model+trace+state (minus GUI-only bits) and searchtree's tree/query core. Key files: `facade/BasicBoard.java` (1488), `facade/BoardItemRepository.java` (288), `facade/BoardConnectivityQueries.java` (125), `model/items/Item.java` (1343), `Pin.java` (706), `DrillItem.java` (416), `Via.java` (274), `ObstacleArea.java` (356), `ConductionArea.java` (424), `model/structure/BoardOutline.java` (290), `trace/PolylineTrace.java` (1314), `trace/PolylineTraceNormalization.java` (133), `datastructures/UndoableObjects.java` (343), `datastructures/ShapeTree.java` (236), `datastructures/MinAreaTree.java` (180), `board/searchtree/ShapeSearchTree.java` (1160), `SearchTreeManager.java` (285).

### Item hierarchy + id model

- `Item` fields (`Item.java:38-67`): `id` (final), `board` (transient), `smallestClearance`, `componentId`, `netNumbers int[]`, `clearanceClassIndex`, `searchTreesInfo` (transient), `fixedState`, `onTheBoard`, `autorouteInfo` (transient). Kind dispatch: `getBoardItemType()` (`Item.java:117-146`) — PIN/VIA/TRACE/CONDUCTION_AREA/VIA_OBSTACLE_AREA/COMPONENT_OBSTACLE_AREA/OBSTACLE_AREA/COMPONENT_OUTLINE/BOARD_OUTLINE/OTHER.
- `Trace` = `halfWidth` + `layer`; `PolylineTrace` adds ONE primary field `Polyline lines` (`PolylineTrace.java:42`). `DrillItem` = `center` + precalculated min-width/first/last-layer; `Via` adds padstack/attachAllowed/escape fields; `Pin` adds `pinIndex`/`changedTo` with center DERIVED from placement (`Pin.getCenter`). `ObstacleArea` = relative area + translation/rotation/sideChanged (absolute area computed lazily — the M1b T49 transform); `ConductionArea` adds isObstacle/isFilled.
- Ids: allocated in the **Item constructor** (`Item.java:86-90`), not insert — construct-then-discard BURNS an id. `ItemIdGenerator` monotone from 1, `MAX_ID = Integer.MAX_VALUE/2`, wraps to 1 with one warning (`ItemIdGenerator.java:23,37-55`). Deletion never frees ids.
- Ordering: `Item.compareTo = item.id - id` (`Item.java:95-103`) — **descending id**; `UndoableObjects` is a `ConcurrentSkipListMap` under natural order, so EVERY board enumeration (`BoardItemRepository.getItems/Traces/Vias/Pins`, `getOutline` first-wins, 32 `TreeSet<Item>` contact/overlap sites) walks descending id.

### UndoableObjects semantics (the snapshot/undo gate target)

Level-stack of value snapshots (`UndoableObjects.java`; node = `{object, level, undoObject, redoObject}` :328-342):
- `insert` :66-70 → `disableRedo()` then node at current level. `delete` :76-128 → if `node.level < stackLevel` push the NODE onto the current delete list, else push `node.undoObject` (if non-null); remove from map.
- `generateSnapshot` :131-136 → push empty delete list, `++stackLevel`. `undo` :143-176 → for nodes at `level == stackLevel`: if `undoObject != null` set `undoObject.redoObject = node` and put the undoObject's value in (restored); collect cancelled; then re-put ALL nodes from `deletedObjectsStack[stackLevel-1]`; `--stackLevel`; `redoPossible = true`.
- `redo` :183-226 → `++stackLevel`; nodes with `redoObject.level == stackLevel` swap forward; nodes AT the level are "restored"; deleted nodes re-deleted by walking the redoObject CHAIN (`while redoObject != null && redoObject.level <= stackLevel`).
- `popSnapshot` :232-267 → re-links undo/redo chains across the popped level (`level == stackLevel-1` re-link, `level >= stackLevel` decremented), merges top delete list into second-top with the same node/undoObject dichotomy.
- `saveForUndo` :273-290 → clone-on-first-mutation-per-level: `node.level < stackLevel` → clone value into oldNode, splice `oldNode.undoObject = node.undoObject; oldNode.redoObject = node; node.undoObject = oldNode; node.level = stackLevel`.
- `disableRedo` :293-309 → runs on EVERY mutator (insert/delete/generateSnapshot/saveForUndo/popSnapshot): truncates the delete stack above `stackLevel`, REMOVES nodes `level > stackLevel`, nulls `redoObject` on `level == stackLevel` nodes.
- `readObject` :54-63 skips nodes `level > stackLevel` (redo-only objects invisible).
- Map keys: Java uses the object itself (compareTo on id) — clones replace VALUES under the equal key. Rust: key = item id.
- Board wiring: `BasicBoard.undo/redo/generateSnapshot/popSnapshot` `BasicBoard.java:1234-1299`, side effects (tree remove/insert, observer notify, changed nets, `clearAutorouteInfo`) in `applyUndoRedoSideEffects` :1256-1288 / `RoutingBoardUndoFacade.java:74-104`. `Components` keeps a SECOND independent `UndoableObjects`; component ids are 1-based indexes (`Components.java:16,140`). No parser/router code snapshots — parse builds at level 0.

### normalizeAllTraces mechanics (the dsn-0151 retirement)

- Called once at parse end after `(wiring ...)` closes: `Wiring.java:345-353` (try/catch log-only).
- `BasicBoard.normalizeAllTraces()` `BasicBoard.java:799-886`: group on-board `PolylineTrace`s per net (`HashMap` :802), per net loop `while something changed` capped `MAX_NORMALIZE_ITERATIONS = 2000` (:64, :833, cap → break); per trace `trace.normalize(null)` then, if unchanged and not user-fixed, `removeIfCycle` (:850-856); any change re-collects that net's traces (:862-881); `ConcurrentModificationException` caught as retry signal (:808, :868). The per-net variant `normalizeTraces(int)` :710-796 additionally suppresses via `normalizeSuppressedNetNos` — the ALL variant does NOT.
- `PolylineTraceNormalization.normalize` (`:20-132`): recursion cap `MAX_NORMALIZATION_DEPTH = 16`; `split(clipShape)` → per piece `combine()`; a piece reduced to a single corner is REMOVED unless deletion-forbidden (:102-121); recurse deeper after combine (:122-124).
- `PolylineTrace.split` (`PolylineTrace.java:465-691`): splits this + overlapping traces at intersections (search-tree driven :483-486), splits at drill-item centers (:654-660), removes cycle pieces (:630), refuses deletion-forbidden traces (:727-729); physical split = remove-old + insert-two-pieces via `insertTraceWithoutCleaning` (:741-759) — **each piece a fresh id, old id burned**.
- `PolylineTrace.combine` (`:175-456`, iterative, start-then-end retry): requires EXACTLY ONE contact trace with equal layer/net/halfWidth/fixedState, neither deletion-forbidden (:242-247, :382-387); **survivor keeps its id** (`saveForUndo` + geometry replace), the other removed (:327, :451); collinear join collapse via `isEqualOrOpposite` skipLine (:291, :415) and canonically in the `Polyline` constructor — `geometry/planar/Polyline.java:78-102` → `removeConsecutiveParallelLines` + `removeOverlaps`. **Already ported**: `epic_geometry::polyline::Polyline::new` (polyline.rs:118-123) runs both filters — the dsn-0151 merger core exists.
- Other run sites (M3+): `FoundConnectionInserter.java:108`, `TraceTightener.java:452`, `BasicBoard.insertTrace` :210-243.

### searchtree at a glance

NOT a quadtree — an R-tree-like **binary BVH** ("MinAreaTree"). `ShapeTree.java`: `Leaf {object: Storable, shapeIndexInObject, boundingShape: RegularTileShape, parent}`, `InnerNode {boundingShape, firstChild, secondChild}`, `insert(Storable)` inserts ONE LEAF PER TREE SHAPE, `toArray()` in-order leaf enumeration. `MinAreaTree.java`:
- `insert(Leaf)` :50-87: empty → root; else `positionLocate` descent; new `InnerNode(bounds=union(leaf, replaced))` spliced at the replaced leaf's parent; children order **old-leaf-first, new-leaf-second** (:81-82); root swap if replaced was root.
- `positionLocate` :89-116: at each inner node **eagerly union** the inserted shape into that node's bounds (:94-95 — even nodes whose OTHER child is descended), then descend to the child with minimal area increase, tie `firstAreaIncrease <= secondAreaIncrease` (:109) on DOUBLE `RegularTileShape.area()`.
- `removeLeaf` :118-179: sibling promotion under grandparent; ancestor bounds recomputed **only while strictly shrinking** (`newBounds.contains(old)` → break :172) — stale bounds persist (history-dependent tree).
- `overlaps(shape)` :25-48: DFS with explicit `ArrayStack`, children pushed first-then-second; collects `TreeSet<Leaf>` sorted by `Leaf.compareTo` (object compareTo then `shapeIndexInObject`, `ShapeTree.java:217-223`) — bbox prefilter ONLY, no exact-shape check.

`ShapeSearchTree.java` (1160): clearance compensation (T54), per-item-type tree-shape construction (:871-1074: traces :992-1004, drill :871-906, obstacle areas :908-938 with convex division + `divideIntoSections` at 50000, outline :940-990), the query family (§ below), `EntrySortedByClearance` with the GLOBAL static `lastGeneratedEntryId` (:55, :1135-1159). `SearchTreeManager.java` (285): default tree (45° directions, class 0; compensation rebuilds it class 1 :89-108), per-class autoroute trees chosen by `traceAngleRestriction` (:140-172) with bulk insert, `reinsertTreeItems` MUST `clearDerivedData()` on every item between remove and insert (:186-200), `compensatedSearchTrees` LinkedList with default-tree-slot-0 invariant. Item-side: `ItemSearchTreesInfo` (identity-keyed per-tree `Leaf[]` + shape cache), `Item.getTreeShape` lazy compute (`Item.java:213-237`), `clearDerivedData` :1104.

### Query surface (the index-parity gate target)

| Method | Anchor | Result semantics |
|---|---|---|
| `overlaps(RegularTileShape)` | MinAreaTree.java:25-48 | `TreeSet<Leaf>` — bbox prefilter, sorted-dedup (object order then shape index) |
| `overlappingTreeEntries(shape, layer, ignoreNetNos, Collection)` | ShapeSearchTree.java:390-434 | bbox hits → layer filter (`shapeLayer(i) != layer` skip) → ignore-net filter (`isObstacle(net)`) → **exact-shape `intersects`**, SKIPPED iff query AND stored shapes are both octagons (:421-427); appended in **DFS order** (LinkedList) |
| `overlappingObjects(shape, layer[, nets])` | :355-374 | dedup to objects, TreeSet sorted |
| `overlappingTreeEntriesWithClearance` | :443-524 | compensated tree → plain `overlappingTreeEntries` (:514-524); else bbox query offset `int(1.2 * maxValue(class,layer))` (:462) → collect into `TreeSet<EntrySortedByClearance>` (clearance value, ties by global static counter) → enlarge both shapes by clearance/2, test `intersects` |
| `overlappingObjectsWithClearance` / `overlappingItemsWithClearance` | :530-569 | dispatch + dedup (objects / Items only) |
| `BasicBoard.checkShape` | BasicBoard.java:958 | insertion feasibility incl. board-bbox containment |

`completeShape`/`restrainShape` ×3 variants and `ShapeTraceEntries` are **M3** (D18).

### Existing Rust assets (do NOT re-port)

`epic_geometry`: `Polyline::new` canonicalization, `RegularTileShape::area() -> f64` (regular_tile_shape.rs:161), `ShapeBoundingDirections` (shape.rs:150), `TileShape::bounding_shape(directions)` (tile_shape.rs:312), `IntOctagon`/`IntBox` full API, `java_round`, `Polyline::offset_shapes/offset_shape`. `epic_dsn`: `SesBoard` IR + `BoardSink` (sink.rs), T49 `BoardShape::mirror_vertical/turn_90_degree/rotate_approx/translate_by` + `SesBoard::obstacle_absolute_area` (the lazy-transform reference), `ses::writer::write_session`, digest plumbing (`dsn_digest.rs`, `dsn_corpus.rs` incl. the `DIVERGENCE_LEDGER` at :873-882).

### Parity traps T50–T70 (each needs a pinned test, a corpus case, or a golden-observed value)

| # | Trap | Java behavior to reproduce |
|---|---|---|
| T50 | Insert descent | `positionLocate` EAGERLY unions the inserted shape into every VISITED inner node's bounds (MinAreaTree.java:94-95) — including nodes whose sibling child is then chosen; min-area-increase descent on double `area()`; tie `firstAreaIncrease <= secondAreaIncrease` (:109) → first child; new InnerNode children order old-leaf-first/new-leaf-second (:81-82). |
| T51 | Stale bounds after remove | `removeLeaf` recomputes ancestor bounds only while strictly shrinking (`newBounds.contains(old)` → break, MinAreaTree.java:166-178). Tree shape is history-dependent: same final item set, different insert/remove ORDER ⇒ different tree. |
| T52 | overlaps ordering | DFS pushes first-then-second (popped second-then-first) but result is a `TreeSet<Leaf>` sorted by object compareTo (DESCENDING id) then `shapeIndexInObject` (ShapeTree.java:217-223). `overlappingTreeEntries` instead appends in raw DFS order. |
| T53 | Octagon-skip | Exact-shape `intersects` skipped ONLY when query shape AND stored tree shape are both octagons (ShapeSearchTree.java:421-427) — bbox-touching boundaries diverge if the dispatch differs. |
| T54 | Compensation asymmetry | `offsetWidth = halfWidth + max(0, clearanceMatrix.getValue(itemClass, treeClass, layer, false) − clearanceMatrix.clearanceCompensationValue(treeClass, layer))` (ShapeSearchTree.java:104-114); matrix-side compensation = `(value(c,c,layer)+1)/2` (ClearanceMatrix.java:272-275); `getValue` indexes `row[classJ].column[classI].layer[layer]` (ClearanceMatrix.java:131+) — row/column order is parity-critical. |
| T55 | Drill-hole inflation (EpicRouter-specific) | `drillHoleObstacle` circle for copper-less pads + `drillHoleClearanceDelta = ceil(drillRadius + holeClearance + 10 − copperRadius − copperClearance)` on every drilled item (ShapeSearchTree.java:52, 1006-1074); the 45° tree uses offset-vs-enlarge ASYMMETRY (45Degree.java:488-519). |
| T56 | Clearance-sorted query | Uncompensated `WithClearance`: pre-offset `int(1.2 * maxValue(class,layer))` (:462), result `TreeSet<EntrySortedByClearance>` — ties broken by the GLOBAL STATIC `lastGeneratedEntryId` counter (:55, :1135-1159): tie order depends on how many entries were ever created process-wide; enlarge by clearance/2 both sides then `intersects`. |
| T57 | Compensated dispatch | When `isClearanceCompensationUsed()`, `WithClearance` degenerates to plain `overlappingTreeEntries` (:514-549) — clearance baked into stored shapes. |
| T58 | Tree-shape construction | Traces: one offset shape per segment, `offsetWidth` per T54 (:992-1004); obstacle areas: convex division + `divideIntoSections` at 50000 (:908-938); outline: line shapes (:940-990); per-item per-tree cache identity-keyed; `reinsertTreeItems` MUST `clearDerivedData()` between remove and insert or stale shapes are silently reused (SearchTreeManager.java:186-200). |
| T59 | Autoroute tree selection | `getAutorouteTree(class)` picks default/45°/90° by `traceAngleRestriction` (NONE/45/90, ordinal-load-bearing `AngleRestriction.java`), bulk-inserts the whole board (SearchTreeManager.java:140-172); default tree stays slot 0. |
| T60 | Descending-id iteration | `Item.compareTo = item.id - id`; ConcurrentSkipListMap natural order; 32 `TreeSet<Item>` sites (contacts `Trace.java:156,179`, connectivity `BoardConnectivityQueries.java:104`, overlaps `BasicBoard.java:940`). Rust arena enumeration MUST walk descending id everywhere. |
| T61 | Id burn | Allocation in the Item CONSTRUCTOR (`Item.java:86-90`): closed-trace drop (`BasicBoard.java:192-196`), split pieces, degenerate normalize removals all burn; `MAX_ID = Integer.MAX_VALUE/2` wrap-to-1 (ItemIdGenerator.java:23,37-55). New inserts continue the monotone sequence after undo. |
| T62 | UndoableObjects node rules | delete pushes the NODE iff `level < stackLevel` else `undoObject`; undo cross-links `undoObject.redoObject = node`; redo walks redoObject chains; popSnapshot re-links chains + merges delete lists with the same dichotomy; `disableRedo` on EVERY mutator truncates stack, removes `level > stackLevel` nodes, nulls redoObject at level; `readObject` skips `level > stackLevel`. |
| T63 | Components' second stack | `Components` owns its own `UndoableObjects`; component ids are 1-based indexes into `componentArr` (`Components.java:16, 140`). |
| T64 | normalizeAllTraces convergence | Per-net `HashMap` grouping, while-changed loop cap 2000 (break, NO suppression — the per-net variant's `normalizeSuppressedNetNos` :711-724 does not apply), CME-as-retry → in Rust: deterministic re-collection with the same observable order (descending id walk), `normalize(null)` then `removeIfCycle` if unchanged and not user-fixed. |
| T65 | Normalization depth cap | `MAX_NORMALIZATION_DEPTH = 16` recursion cap; single-corner piece removed UNLESS deletion-forbidden (PolylineTraceNormalization.java:16, 102-124). |
| T66 | combine contract | EXACTLY ONE contact trace, equal layer/net/halfWidth/fixedState, neither deletion-forbidden; survivor keeps id (saveForUndo + geometry replace), other removed; start-then-end retry; `isEqualOrOpposite` skipLine join collapse (PolylineTrace.java:175-456). |
| T67 | split contract | Search-tree-driven intersection splits (both traces), drill-center splits, cycle-piece removal, deletion-forbidden refusal; remove-old + two fresh-id pieces via `insertTraceWithoutCleaning` (PolylineTrace.java:465-691, 741-759). |
| T68 | Placement float rounding | `Component.rotate` → `FloatPoint.rotate(Math.toRadians(rot)).round()` (Component.java:146); `Pin.relativeLocation` non-90° branch + `getCenter` pad-shape correction (Pin.java:78+) — reuse `java_round`. |
| T69 | Order-dependent aggregates | `revision` bumped per insert/remove (BoardItemRepository.java:166,198); `maxTraceHalfWidth/minTraceHalfWidth` updated only when `netsNormal()` (BasicBoard.java:107-110, 198-201). |
| T70 | Keepout kinds stay distinct | `BoardItemType` separates OBSTACLE_AREA / VIA_OBSTACLE_AREA / COMPONENT_OBSTACLE_AREA (Item.java:117-146); the M1b digest erases the kind (M1b plan :278) — the epic-board item enum keeps all three. |

### Port map (Java → crates)

| Java | Rust module | Responsibility |
|---|---|---|
| `UndoableObjects`, `ItemIdGenerator` | `epic-board/src/undo.rs`, `id.rs` | level-stack container, id allocation + burn |
| `Item` + item classes | `epic-board/src/items/{mod,trace,drill,pin,obstacle,outline}.rs` | arena items, kind enum (T70), geometry |
| `BasicBoard`, `BoardItemRepository`, `BoardConnectivityQueries` (read side) | `epic-board/src/board.rs` | insert/remove/query facade, revision, enumeration |
| `model/structure/*`, `rules` read surface | `epic-board/src/{components,rules_surf,layers}.rs` | components/placement resolution, clearance matrix + nets + via rules (T54, T68) |
| `PolylineTrace` split/combine/normalize | `epic-board/src/trace_ops.rs` | T64–T67 |
| `ShapeTree`, `MinAreaTree` | `epic-index/src/{shape_tree,min_area_tree}.rs` | the BVH core (T50–T52) |
| `ShapeSearchTree` (query family + compensation config) | `epic-index/src/search_tree.rs`, `epic-board/src/tree_shapes.rs` | queries (T53, T56, T57); per-item shape construction (T54, T55, T58) in epic-board |
| `SearchTreeManager` | `epic-board/src/tree_manager.rs` | tree set, broadcast, autoroute trees (T59) |
| `SesWriter.snappedEndpoint` | `epic-dsn/src/ses/writer.rs` (+ trait) | T40 via contacts provider (D23) |
| **Deferred M3 (D18)** | — | `completeShape`/`restrainShape` ×3, `ShapeTraceEntries`, entry-reuse fast paths, `optimize/`, `actions/`, autoroute room entries, `RoutingBoard` engine hooks |

Dependency rule: `epic-index → epic-geometry` only; `epic-board → {epic-geometry, epic-index, epic-dsn}`; `epic-dsn` gains NO new crate deps (the T40 trait is defined IN epic-dsn, implemented by epic-board — no cycle).

---

## Locked decisions (do not relitigate during execution)

- **D17 — Crate split:** `epic-index` is a pure tree library (item keys are opaque ids; depends on epic-geometry only). `epic-board` owns items, rules, undo, per-item tree-shape inflation, and tree management. This splits Java's `ShapeSearchTree` conflation cleanly and keeps the M1b ArchUnit rule (epic-dsn depends on epic-geometry only) intact.
- **D18 — M2 index scope:** tree core + query family + compensation + per-item shape construction + manager. DEFERRED to M3: `completeShape`/`restrainShape` (all 3 variants), `ShapeTraceEntries`, trace-surgery entry reuse (`changeEntries`, `mergeEntriesInFront/AtEnd`, `reuseEntriesAfterCutout` — M2 uses semantically-equivalent remove+reinsert; entry-reuse is M5 perf), autoroute room entries as tree objects. normalize's split needs only `overlappingTreeEntries` + exact intersections — no deferred piece.
- **D19 — Parity bar:** identical TREE SHAPE (in-order leaf dump: object id, shape index, bounding shape) and identical DFS-ordered `overlappingTreeEntries` lists — not just set equality; M3's expansion machinery depends on traversal order. Sorted-set equality is reported as a diagnostic.
- **D20 — Index corpus:** board-driven. `IndexOracle.java` loads each fixture via `DsnReader.readBoard`, builds default + compensated trees, dumps (a) full in-order tree dump per tree, (b) scripted replay: remove item K → insert scripted trace → query → re-remove → query (ripg-up hot path), (c) query family results at shapes derived deterministically from the fixture's own items (self-shape, ±offset variants, center box). Synthetic tie/stale-bound stress comes from ~10 crafted mini-DSN fixtures (`rust/harness/fixtures/index-stress/`) — symmetric layouts for exact area-increase ties, remove/reinsert sequences for T51. Subcommands `index golden` (jar+JDK) / `index compare` (java-free, CI); goldens at `rust/harness/corpus/index-golden.jsonl`.
- **D21 — Undo corpus:** `UndoOracle.java` replays scripted `{generate_snapshot, insert_trace(coords), remove_item(id), undo, redo, pop_snapshot, normalize_all}` sequences on tier fixtures; after EVERY step dumps a state digest: one canonical line per item in descending id (`<kind> <id> <layer> <geometry…> <nets> <class> <fixed>`) + `next_id` + `stack_level` + `item_count`. Subcommands `undo golden` / `undo compare` (java-free, CI). NO Java-serialization port — `BoardSnapshotManager.serialize/hash` is replaced by this explicit digest.
- **D22 — Post-normalize digest fields:** `DsnParseOracle.java` gains `post_stats` + `post_geometry_sha256` (state after parse-end `normalizeAllTraces`); ALL dsn goldens regenerate ONCE (justification: the M2 normalize port) in Task 13; the dsn-0151 `DIVERGENCE_LEDGER` entry and its pins are pruned in the same task; after Task 13 all 1,332 fixtures must match post-normalize fields too.
- **D23 — T40 via contacts provider:** `epic-dsn` defines `trait SessionContacts` (endpoint → optional snap target); `write_session` keeps the 2-arg parse-time form (M1b goldens stay valid) and gains a 3-arg form taking `&dyn SessionContacts`. epic-board implements it from board contacts. New gate `ses-snap compare` over routed fixtures (PCBench routed boards with wiring contacts): parse → Board → contacts → emit 3-arg → byte-compare vs Java goldens (`SesEmitOracle` variant emitting post-board-session bytes). Full post-routing SES parity remains M3 (router needed); M2 proves the snap rule itself.
- **D24 — write_net enum restructure** (M1b minor 2) rides in Task 15: replace the `_ => unreachable!` dispatch (writer.rs:452) with an `Emittable` enum returned by the filter match.
- **D25 — Enumeration contract:** every epic-board public enumeration walks descending id (T60). Internal arena layout (BTreeMap<Reverse<ItemId>, _> vs sorted Vec) is free choice; order is contract.
- **D26 — Contacts scope:** port Trace start/end contact STORAGE + compute-on-demand (the Java definition jar-spiked in Task 10 before pinning) + the minimal `BoardConnectivityQueries` read surface combine/snap need. Full connected-set machinery is M3.
- **D27 — Undo wiring parity:** `Board::{undo,redo,generate_snapshot,pop_snapshot}` reproduce `applyUndoRedoSideEffects` (tree remove/insert of cancelled/restored items, changed-nets collection, autoroute-info clear stub). Parse-time board use stays snapshot-free (level 0) exactly like Java.

---

## Task 1: epic-board scaffold — ids, arena, kinds, `Board::from_ses_board`

**Files:** Modify `rust/crates/epic-board/{Cargo.toml (add epic-geometry, epic-dsn path deps), src/lib.rs}`; create `src/id.rs`, `src/items/mod.rs`, `src/board.rs`.

- [ ] `id.rs`: `ItemId(u32)` newtype + `ItemIdGenerator` — monotone from 1, `MAX_ID = i32::MAX/2`, wrap-to-1 (T61) with a one-shot warning flag (Java warns once per WRAP EVENT — ItemIdGenerator.java:41-50; the port's one-shot flag is a documented log-only divergence). Pin: sequence 1..3, a forced wrap via `generator.set_next(MAX_ID)` test seam, monotone-continues-after-delete.
- [ ] `items/mod.rs`: `BoardItemType` enum with ALL TEN kinds (T70) + `ItemData` enum (one variant per kind carrying that kind's primary fields per the recon field map; `Trace { half_width, layer, lines: Polyline }`, `Drill { center, kind: Pin|Via, … }`, `Obstacle { relative_area, translation, rotation, side_changed, kind }`, …). No methods yet.
- [ ] `board.rs`: `Board` struct — arena (`BTreeMap<Reverse<ItemId>, ItemEntry>` or equivalent, D25), `id_generator`, `revision: u64` (bumped on insert AND remove, T69), `on_the_board` flags; `insert_item`/`remove_item`/`iter_descending`. Insert does NOT allocate ids (allocation is at construction — expose `Board::alloc_id()` used by builders; document the burn semantics).
- [ ] `from_ses_board(&SesBoard) -> Board`: convert every `ItemIr` preserving ids and the generator position (`last_assigned_item_id`); keepout kinds distinct (T70 — map `ItemIr::Keepout`'s kind if the IR distinguishes, else record a TODO only if the IR genuinely lost it; check `sink.rs` first); outline/conduction/pins/vias/traces all carried.
- [ ] Pins: id preservation on 3 fixtures (assert `board.iter_descending()` ids equal the digest-order ids); wrap pin via test seam; revision increments.
- [ ] Gates + commit `feat(m2): epic-board scaffold — ids, arena, kinds, from_ses_board`.

## Task 2: UndoableObjects container (pure) + Components' second stack

**Files:** Create `rust/crates/epic-board/src/undo.rs`; modify lib.rs.

- [ ] **Jar spike FIRST** (cerebrum rule 1): write `rust/harness/oracle/UndoSpike.java` (package `app.freerouting.datastructures` for access if needed — public API suffices): a `Storable` impl wrapping `(id, value)` with `compareTo` on id DESCENDING (mirror Item) and a `clone` that copies; drive one scripted sequence (insert a,b,c → snapshot → saveForUndo(b)+mutate → delete(a) → insert(d) → snapshot → delete(c) → undo → undo → redo → popSnapshot interleavings) printing after each step: iteration order (startReadObject/readObject), cancelled/restored collections, stack level. Capture output to `/tmp/epic-t2-undo.out`; EVERY pin below quotes it.
- [ ] Transliterate `UndoableObjects` field-for-field (T62): `objects: BTreeMap<Reverse<ItemId>, Node>` (descending read order = Java's map order), `deleted_objects_stack: Vec<Vec<Node>>`, `stack_level`, `redo_possible`; node `{value: T, level, undo_object: idx, redo_object: idx}` with arena indices for the cross-links (Rust can't hold the node graph in a map — use a node slab; the LIVE map maps id → current node id). Methods: insert/delete/generate_snapshot/undo/redo/pop_snapshot/save_for_undo/disable_redo/read order — every mutator calls disable_redo; redo walks redo chains; delete dichotomy `level < stack_level` → node else undo_object.
- [ ] Components' second stack (T63): `Components`-equivalent holds its own container; ids 1-based indexes.
- [ ] Pins from the spike output: every branch (node-vs-undoObject push, undo cross-link, redo chain walk of length ≥2, popSnapshot re-link + merge, disableRedo truncation, readObject skip of redo-only nodes). Mutation-verify at least the delete dichotomy and the redo-chain pins (flip the condition; the pin must fail).
- [ ] Gates + commit `feat(m2): UndoableObjects level-stack port (jar-pinned) + components stack`.

## Task 3: Rules + structure surface + placement resolution

**Files:** Create `src/rules_surf.rs`, `src/components.rs`, `src/layers.rs`; modify board.rs (Board fields).

- [ ] `rules_surf.rs`: `ClearanceMatrix` (class names, `get_value(row_class, col_class, layer)` with the EXACT index order T54, `set_value`, `max_value(class, layer)`, `clearance_compensation_value(class) = (value(c,c,layer)+1)/2`); `Nets` (table, `max_legal_net_number=9999999`, hidden 10000001), net classes, via infos + via rules, `traceAngleRestriction` ordinals NONE/45/90, per-layer default half widths, hole clearance, `pinEdgeToTurnDist`. Built from the epic-dsn clearance IR (`BoardRulesIr`) — conversion in `from_ses_board`.
- [ ] `layers.rs`: `LayerStructure` port (name→index, signal renumbering `signalLayerCount`/`getSignalLayerNo`, Electra fallback already in epic-dsn — reuse, do not duplicate).
- [ ] `components.rs`: `Component { name, package_front/back, location, rotation, is_front, fixed }` + `Components` container (1-based ids, its undo stack from Task 2); `Component::rotate` → `FloatPoint::rotate(toRadians).round()` via `java_round` (T68); flip-style rotate-first flag from board metadata.
- [ ] `Pin` placement resolution: `relative_location` (90°-multiple exact vs non-90° float branch) + `get_center` incl. the pad-shape correction (T68). **Jar spike before pinning** (jshell on one tier-A fixture: dump 5 pin centers incl. one rotated non-90° and one corrected pad).
- [ ] Pins: matrix index-order asymmetry (a fixture with asymmetric class values where row/col swap changes the result — non-degenerate), compensation value rounding `(v+1)/2` on odd values, rotation rounding on the jar-captured centers.
- [ ] Gates + commit `feat(m2): rules surface (clearance matrix asymmetry) + components + pin resolution`.

## Task 4: Item geometry

**Files:** Create/extend `src/items/{trace,drill,pin,obstacle,outline}.rs`.

- [ ] `PolylineTrace`: `lines: Polyline` + derived corners (`PolylineTraceGeometry` equivalents: first/last corner, corner count); `half_width`, `layer`, fixed state, nets.
- [ ] `DrillItem`/`Via`: center, padstack ref, precalculated `min_width`/`first_layer`/`last_layer` (lazy + clear-on-change); `attach_allowed`.
- [ ] `ObstacleArea` absolute area: the T49 transform chain (mirror → rotate exact/approx → translate) using epic-geometry primitives — `epic_dsn`'s `obstacle_absolute_area` is the REFERENCE implementation; port the same sequence against epic-board's types (no epic-dsn call into item internals — keep the crates' types separate).
- [ ] `ConductionArea` (isObstacle/isFilled flags; NO AWT fill cache), `ComponentOutline` (courtyard/fabrication/closed flags), `BoardOutline` (`shapes`, `keepout_area` derivation, `keepout_lines`, `keepout_outside_outline`, HALF_WIDTH = 100).
- [ ] Pins: absolute-area rows copied from the M1b T49 jar captures (they must reproduce identically through the epic-board path), outline keepout area on one tier fixture.
- [ ] Gates + commit `feat(m2): item geometry — traces, drills, obstacles, outline`.

## Task 5: epic-index core — ShapeTree + MinAreaTree

**Files:** Modify `rust/crates/epic-index/{Cargo.toml (epic-geometry dep), src/lib.rs}`; create `src/shape_tree.rs`, `src/min_area_tree.rs`.

- [ ] `shape_tree.rs`: node arena (`Node::{Leaf { object_key: u64, shape_index_in_object: u32, bounds }, Inner { bounds, first, second }}` — indices into a slab; parent pointers as indices); `insert(object_key, shapes: &[RegularTileShape])` = one leaf per shape; `to_array()` in-order; `leaf_count`. `Leaf` sort key mirrors `ShapeTree.Leaf.compareTo` (ShapeTree.java:217-223): the OBJECT's compareTo (Item descending id) then `shapeIndexInObject` ASC — since epic-index is generic (D17), the object ordering is supplied by the caller as a comparator closure over `object_key`.
- [ ] `min_area_tree.rs` — transliterate MinAreaTree.java verbatim (T50/T51/T52):
```rust
fn position_locate(&mut self, mut node: NodeIdx, leaf: NodeIdx) -> NodeIdx {
    while !self.is_leaf(node) {
        // EAGER union into every VISITED inner node (MinAreaTree.java:94-95)
        self.bounds[node] = self.bounds[leaf].union(&self.bounds[node]);
        let (first, second) = self.children(node);
        let u1 = self.bounds[leaf].union(&self.bounds[first]);
        let u2 = self.bounds[leaf].union(&self.bounds[second]);
        let d1 = u1.area() - self.bounds[first].area(); // f64, Java double
        let d2 = u2.area() - self.bounds[second].area();
        node = if d1 <= d2 { first } else { second };   // T50 tie -> first
    }
    node
}
```
  `remove_leaf`: sibling promotion, parent detach, then the strict-shrink ancestor loop (`new_bounds.contains(old)` → break, T51). `overlaps(shape, cmp)`: explicit Vec stack, push first-then-second, collect into the sorted set keyed by `(cmp(object_key), shape_index)`.
- [ ] Pins (synthetic, pure tree): exact-tie descent (two children whose unions have EQUAL area — symmetric boxes — must pick FIRST); eager-union observability (after an insert that descends right, the left sibling's PARENT bounds must still have grown — assert via to_array dump); stale bounds (insert 3, remove 1 → an ancestor whose tight bounds would shrink keeps the larger shape — assert the dump differs from a freshly-built same-set tree); remove-to-empty, remove-root cases; overlaps dedup+order.
- [ ] Gates + commit `feat(m2): epic-index — MinAreaTree/ShapeTree core (tie + stale-bound pins)`.

## Task 6: ShapeSearchTree config + manager + compensation

**Files:** epic-index: create `src/search_tree.rs` (tree wrapper: bounding-direction variant + `compensated_class: u16` + `use_clearance_compensation` flag); epic-board: create `src/tree_manager.rs`, extend tree_shapes stub.

- [ ] `search_tree.rs`: `SearchTree { tree: MinAreaTree, directions: ShapeBoundingDirections, compensated_clearance_class }`; insertion goes through the caller-provided shape list (epic-board computes compensated shapes; D17 keeps the formula board-side).
- [ ] `tree_manager.rs`: default tree (45° directions, class 0; `set_clearance_compensation_used(true)` → rebuild as class 1), `insert/remove` broadcast to ALL trees, `get_autoroute_tree(class)` per T59 (angle-restriction variant + bulk insert), `reinsert_tree_items` with `clear_derived_data` between remove and insert (T58 — pin that skipping it yields stale shapes on a moved item), `reset_compensated_trees`, default-tree slot-0 invariant.
- [ ] Compensation formula (T54) in epic-board (`tree_shapes.rs`): `offset_width = half_width + max(0, matrix.get_value(item_class, tree_class, layer) − matrix.clearance_compensation_value(tree_class))` — pin the row/column order with an ASYMMETRIC matrix (non-degenerate) and negative-clamp (compensation > value → 0).
- [ ] Drill inflation (T55): `drill_hole_clearance_delta` formula + copper-less pad circle; the 45° offset-vs-enlarge asymmetry — pin all three tree variants against a jar dump (`DrillHoleClearanceShapeTest.java` is the Java precedent — port its cases).
- [ ] Gates + commit `feat(m2): search-tree config, manager, clearance compensation + drill inflation`.

## Task 7: calculateTreeShapes family

**Files:** epic-board: extend `src/tree_shapes.rs`.

- [ ] Per item kind (ShapeSearchTree.java:871-1074 + subclass overrides): trace segments (offset shapes at `offset_width`), drill items (incl. delta), obstacle areas (convex division + `divideIntoSections` at 50000), outline line shapes (line decomposition), conduction/component/via obstacle areas. Per-item per-tree cache keyed by tree identity with `clear_derived_data`.
- [ ] **Oracle spike:** jshell dump of `item.getTreeShape(...)` (or the `ItemSearchTreesInfo` arrays via reflection) for the first N items of one tier fixture per tree variant — the pin source.
- [ ] Pins: one dump-comparison per item kind and per tree variant (45° default, 90°, compensated class-1) — coordinates exact (decimal ints); the 50000 sectioning on a crafted keepout larger than the threshold (non-degenerate: assert ≥2 sections).
- [ ] Gates + commit `feat(m2): per-item tree-shape construction (jar-dumped per kind/variant)`.

## Task 8: Query family

**Files:** epic-index: extend `src/search_tree.rs`; epic-board: thin wrappers.

- [ ] `overlapping_tree_entries(shape, layer, ignore_nets)` — transliterate :390-434: bbox hits (T52 DFS order) → layer filter → `is_obstacle(net)` filter → exact-shape `intersects` with the octagon-skip (T53). Result: DFS-ordered Vec.
- [ ] `overlapping_objects` / `overlapping_items` (dedup, descending-id sort), `with_clearance` both branches (T56 uncompensated: `int(1.2*max_value)` pre-offset, TreeSet-by-clearance with the GLOBAL static entry counter — port the counter as tree-owned state seeded identically; enlarge clearance/2 both sides; T57 compensated dispatch → plain entries). `check_shape` (board-bbox containment too).
- [ ] Pins: boundary-touching shapes where the octagon-skip matters (octagon-query vs box-query on the same stored shape — one skips exact test, one doesn't); both dispatch branches on the same board (compensation on/off); static-counter tie behavior (two entries with EQUAL clearance — order follows creation order across two successive queries); ignore-nets and layer-filter skips.
- [ ] Gates + commit `feat(m2): overlap query family — octagon-skip, clearance branches, DFS order`.

## Task 9: Index parity corpus — IndexOracle, stressor fixtures, CI

**Files:** Create `rust/harness/oracle/IndexOracle.java`, `rust/harness/src/index_corpus.rs`, `rust/harness/fixtures/index-stress/*.dsn` (~10); modify `harness/src/main.rs` (subcommands), `.github/workflows/rust-check.yml`.

- [ ] Stressor fixtures (crafted, minimal): symmetric-layout tie (T50 exact area-increase equality), remove/reinsert stale-bounds (T51), both bounding variants, compensation class matrix (asymmetric), drill inflation (copper-less pad), sectioning (>50000 keepout), octagon-vs-box boundary touch (T53), multi-net ignore filter, clearance-equal ties (T56), empty-tree edges. Each ≤40 lines of DSN; verify EACH fires its trap by inspecting the oracle output once (a stressor that doesn't discriminate is a failed fixture — cerebrum anchor-blind rule).
- [ ] `IndexOracle.java` (D14/D20 conventions): per fixture — parse; build default + compensated-45° (+ 90° where angle restriction says) trees; dump in-order tree shape lines (`L <object_id> <shape_idx> <bounds-canonical>`); replay the fixed script (remove first trace item → query → insert scripted trace at fixture-derived coords → query → remove it → query) dumping `overlapping_tree_entries` DFS-ordered `(object_id, shape_idx)` lists + `overlaps` sorted sets; per-query shapes derived deterministically (each of first-N items' own tree shape; translated ±half-width; board center box). One JSONL line per fixture with all sections.
- [ ] `index_corpus.rs`: `index golden` (one JVM; goldens `rust/harness/corpus/index-golden.jsonl`) / `index compare` (java-free): field-for-field — tree dumps must be byte-equal (D19), query lists order-exact, sorted sets exact; clear mismatch reporting (first diverging fixture + section + line).
- [ ] CI: append `index compare` step to rust-check.yml (EPIC_SKIP_GRADLE=1).
- [ ] Run on all tier A+B+C + stressors; triage any divergence (numeric drift vs tie-break) BEFORE touching code (design §5 divergence discipline).
- [ ] Gates + commit `feat(m2): index parity corpus — oracle, stressors, java-free CI gate`.

## Task 10: Contacts seam

**Files:** epic-board: create `src/contacts.rs`, extend board.rs.

- [ ] **Jar spike FIRST**: on one tier fixture with wiring, dump `Trace.getStartContacts()`/`getEndContacts()` (reflection or a small emitting oracle) for the first N traces — capture ids. Determine the exact Java contact definition (which clearance/shape test, computed when) BEFORE porting; record in `contacts.rs` module docs with anchors.
- [ ] Port: start/end contact STORAGE + compute-on-demand + `clear_contacts` hooks on mutation; contact sets iterate descending id (T60); minimal `BoardConnectivityQueries` read surface (what combine + snappedEndpoint consume).
- [ ] Pins: the jar-captured contact lists (ids exact) on the spiked fixture + a no-contact trace; contacts cleared after geometry change.
- [ ] Gates + commit `feat(m2): trace contacts — jar-spiked definition, compute-on-demand`.

## Task 11: PolylineTrace.combine + removeIfCycle

**Files:** epic-board: create `src/trace_ops.rs`.

- [ ] `combine` (T66): iterative, start-then-end retry; exactly-one-contact contract (pin the TWO-contact and ZERO-contact refusals); survivor keeps id (`save_for_undo` + geometry replace — tree reinsert through the manager remove+insert path, D18); skipLine `isEqualOrOpposite` join; collinear collapse asserted through `Polyline::new` (already canonicalizing).
- [ ] `remove_if_cycle` (PolylineTrace.java cycle-piece logic :630 area).
- [ ] Pins: jar spike on crafted pairs (collinear join, L-join, opposite-direction join, mismatched width/layer/fixed refusals) — capture trace ids + corner lists before/after; id preservation of the survivor; burned id of the removed one.
- [ ] Gates + commit `feat(m2): trace combine + cycle removal (exactly-one-contact contract)`.

## Task 12: PolylineTrace.split + PolylineTraceNormalization

**Files:** epic-board: extend `src/trace_ops.rs`.

- [ ] `split` (T67): search-tree-driven intersection collection (`overlapping_tree_entries`), split of BOTH self and the overlapping trace, drill-center splits, deletion-forbidden refusal, recursive re-split; physical split = remove-old + insert-two-fresh-id pieces; tree updates via manager.
- [ ] `PolylineTraceNormalization::normalize` (T65): depth cap 16, clip-shape split → per-piece combine, single-corner piece removal unless deletion-forbidden, recurse after combine.
- [ ] Pins: jar spike — craft a small DSN whose wires cross + a wire touching a pad center; capture the post-split item list (ids + corners) via a board-dump oracle; cap behavior on a pathological chain (depth exhausted, state stable); id burn count exact.
- [ ] Gates + commit `feat(m2): trace split + normalization recursion (depth cap, id churn)`.

## Task 13: normalizeAllTraces + post-normalize goldens + ledger prune

**Files:** epic-board: extend board.rs (`normalize_all_traces`); harness: `oracle/DsnParseOracle.java` (post fields), `src/dsn_corpus.rs` (compare both field sets; PRUNE the dsn-0151 ledger entry + its pins), regenerate goldens.

- [ ] `normalize_all_traces` (T64): per-net grouping, while-changed loop cap 2000 break (NO suppression), per trace `normalize(null)` then `removeIfCycle` if unchanged and not user-fixed, re-collect after change — order-equivalent to Java's CME-retry (the observable order is the descending-id walk; in Rust re-collect deterministically, no panic path).
- [ ] Oracle: add `post_stats` (items/traces/vias after normalize) + `post_geometry_sha256`; regenerate ALL dsn goldens ONCE (`dsn golden --set all`, commit message states D22 justification). The other fields MUST be unchanged (the regen diff shows only the two new fields — verify and note).
- [ ] Compare: `dsn compare` now checks post fields on all 1,332; **prune the `DIVERGENCE_LEDGER` dsn-0151 entry** and its `pins` tests (the ledger's own StaleMatch rule proves retirement); delete/adjust the census comment (dsn_corpus.rs:858-861).
- [ ] Wire into the harness digest path: parse → `SesBoard` → `Board::from_ses_board` → `normalize_all_traces` → digest post fields (keep the pre fields digest from SesBoard as-is).
- [ ] Gates + commit `feat(m2): normalizeAllTraces port — post-normalize parity on all 1,332, ledger retired (D22)`.

## Task 14: Undo parity corpus — UndoOracle + CI

**Files:** Create `rust/harness/oracle/UndoOracle.java`, `rust/harness/src/undo_corpus.rs`; modify main.rs, rust-check.yml.

- [ ] `UndoOracle.java` (D21): tier fixtures; scripted `{generate_snapshot, insert_trace(coords…), remove_item(id), undo, redo, pop_snapshot, normalize_all}` (ids picked from the fixture's own item list, deterministic); after EVERY step dump the state digest (descending-id canonical lines + `next_id` + `stack_level` + `item_count` + changed-nets collections from the undo call). JSONL per fixture.
- [ ] `undo_corpus.rs`: `undo golden` / `undo compare` (java-free, CI step). State digest on the Rust side mirrors line-for-line.
- [ ] Board-level wiring exercised end-to-end (D27 side effects: tree remove/insert of restored/cancelled — observed through subsequent query dumps in the script).
- [ ] Triage divergences before fixing. Pins for the container already landed in Task 2 — this task's artifact is the CORPUS + green compare on all tier fixtures.
- [ ] Gates + commit `feat(m2): snapshot/undo parity corpus — scripted replay, java-free CI gate`.

## Task 15: T40 snappedEndpoint + contacts provider + write_net enum

**Files:** epic-dsn: modify `src/ses/writer.rs` (+ `Emittable` enum, `SessionContacts` trait — D23/D24); epic-board: implement the trait; harness: `oracle/SesSnapOracle.java` variant, `src/ses_compare.rs` (extend), CI.

- [ ] epic-dsn: `pub trait SessionContacts { fn snapped_endpoint(&self, wire_id/endpoint…) -> Option<(i64,i64)>; }` — shape it to what `snappedEndpoint` actually consumes (SesWriter.java:426-453); `write_session_with_contacts(board, design, contacts)`; the 2-arg form delegates with a no-op provider (M1b goldens byte-stable — assert on the 20 fixtures via existing `dsn ses-compare`).
- [ ] Port `snappedEndpoint` (the snap rule itself) in epic-dsn as a pure function over the contact data; epic-board supplies contacts from Task 10.
- [ ] **Jar spike**: pick 3-5 Routed PCBench boards (wiring with contacts); capture Java's parse→board→emit session bytes (the board has contacts computed by whatever Java path computes them at parse — spike determines WHEN contacts exist on a parse-time board; if they never exist at parse time in Java either, use `board.normalizeAllTraces()` or an explicit contact-computing call the oracle makes — record the exact call sequence).
- [ ] `ses-snap golden` / `ses-snap compare` (byte-equal bar on the routed set; the T40 classifier in ses_compare stays for diagnostics).
- [ ] write_net enum restructure (D24): the kind-match returns `Emittable<'a>`; `_ => unreachable!` (writer.rs:452) becomes non-existent by construction.
- [ ] Gates + commit `feat(m2): T40 endpoint snapping via contacts provider + write_net enum (D23/D24)`.

## Task 16: M2 exit — docs, gates, final review

**Files:** `rust/README.md`, `docs/superpowers/specs/…design.md` (§6 M2 row annotation if numbers changed), `docs/architecture.md` (epic-board/epic-index glossary), `.wolf/STATUS.md` (via /handoff).

- [ ] Full gate run: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` + all six harness compares (corpus 5000, dsn 1,332, dsn ses-compare 20, index, undo, ses-snap).
- [ ] README/design/architecture updates; /handoff; commit.
- [ ] Dispatch the final whole-milestone code review (both reviewers' disciplines: spec compliance vs the design's M2 exit criteria, then quality); fix findings; re-run gates.
