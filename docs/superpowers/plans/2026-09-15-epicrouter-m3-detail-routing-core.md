# EpicRouter M3 — Detail Routing Core + epic-cli Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Port the Java detail-routing core (single-net maze routing: expansion graph → maze search → backtrack → forced insertion) plus `epic-drc` and `epic-cli`, with differential quality gates proving "single-net quality ≥ Java; Tier A per-fixture completion ≥ Java and violations ≤ Java" (design §6 M3 row).

**Architecture:** New crates `epic-router` (maze/expansion/drill/path + pass driver), `epic-drc` (incompletes + clearance violations), `epic-cli` (headless `route`). They build on the M2 surface (epic-board queries/contacts/insertion-undo, epic-index trees). Byte-parity is impossible by design (Java itself is wall-clock nondeterministic via TimeLimits); parity is proven by (a) directional quality gates vs router-only Java baselines (fanout/optimizer disabled), (b) a determinism self-gate, (c) a maze-level event-stream micro-corpus on small fixtures, (d) zero regressions on all six M2 compares.

**Tech Stack:** Rust workspace in `rust/` (run cargo from there); Java oracle via `rust/harness` (JDK 25 at `~/.jdks/jdk-25.0.4.1+1`); branch `epic/main`; NEVER push; NEVER modify Java under `src/`/`src_v19/` (reading fine; `rust/harness/oracle/*.java` is harness-side/editable).

**Recon basis:** controller-commissioned recon of `src/main/java/app/freerouting/autoroute/` (2026-09-15) — all anchors below are recon-verified but implementers RE-VERIFY (jar wins). Key recon findings baked in: GUI coupling in `autoroute/` is zero (events = no-op sink); `autoroutePassMultiThread`/BatchAutorouterThread is dead code (do not port); fanout/optimizer/orthogonal-neighbour/plane-tuning are out of scope (M4/M6); the TRUE cost center is board-side forced insertion + pull-tight (`board/optimize` TraceShover, `RoutingBoard.checkForcedTracePolyline/insertForcedTracePolyline/optChangedArea`) and `ShapeSearchTree.completeShape`.

---

## Milestone exit criteria (design §6 M3)

1. **Tier A directional gates**: per fixture, `incomplete_count ≤ JavaRouterOnly`, `clearance_violations_total ≤ JavaRouterOnly`, router score ≥ Java − ε (relative ε per harness baseline.rs:159 idiom); `router_introduced` violations recorded (gate: 0 preferred) — vs a RE-CAPTURED router-only oracle baseline (fanout+optimizer disabled), not the M0 full-pipeline baselines.
2. **Determinism self-gate**: same DSN → byte-identical SES across runs.
3. **Event-stream micro-corpus**: maze-level digests (destination door/section, assign-time expansion/sorting values, ripped-item id set, per-trace corner counts + first/last corners) pinned vs Java on small deterministic fixtures.
4. **Zero regressions**: all six M2 compares stay green (corpus 5000/5000, dsn 1,332 ledger-empty, ses-compare 20/20, index 33/33, undo 33/33, ses-snap 5/5); `cargo test --workspace` additions only.

**Carry-forwards from the M2 milestone review (baked into tasks):** corpus-shell extraction (T1, FIRST); the four cut pub items return with their first consumer, pinned (T3/T6 note); the undo-digest topology note applies only if the undo replay grows a second tree (not planned in M3 — do not touch the undo corpus).

---

## Task 1: `corpus_common.rs` — extract the corpus shell (M2 carry-forward)

**Files:** Create `rust/harness/src/corpus_common.rs`; modify `dsn_corpus.rs`, `index_corpus.rs`, `undo_corpus.rs` (their shell sections only).

- [ ] Extract the triplicated shell: `ManifestEntry` + `build_manifest`/`build_manifest_bytes` + strict `load_manifest` + `manifest()` + `ensure_alignment` + `json_string` + `truncate` (pick ONE truncate cap — document the choice; the drift was 120 vs 160) + the four sha256-hex helpers (dsn_digest.rs:462, ses_compare.rs:349, index_corpus.rs:499, undo_corpus.rs:550) into one module with generic parameters for corpus-specific rows.
- [ ] The deep machinery (canon text, `dsn_corpus::resolve_input/resolve_output`) stays where it is.
- [ ] All existing corpora compile onto the shared shell; NO golden/manifest format changes (byte-identical manifest output; if the truncate-cap unification changes any diff output, that is a golden-visible change — verify all six compares + the alignment pins stay green, and state the cap choice in the commit).
- [ ] Pins: an inventory pin per corpus asserting the manifest files still load exactly (they exist already — keep green); add one pin for `corpus_common::ensure_alignment` catching a count mismatch AND an id-sequence mismatch (both sides load-bearing).
- [ ] Gates + ONE commit `refactor(m3): corpus shell extraction — corpus_common.rs (M2 carry-forward)` with trailer `Co-Authored-By: Claude Code <noreply@anthropic.com>`.

## Task 2: epic-drc skeleton — incompletes + clearance violations

**Files:** Create `rust/crates/epic-drc/` (lib.rs, incompletes.rs, clearance.rs); wire into workspace; Cargo: epic-drc → {epic-board, epic-index, epic-geometry}.

- [ ] `calculateAllIncompletes` (drc/DesignRulesChecker.java:545-626): `maxConnections = Σ per net max(0, endpoints(Pin|ConductionArea) − 1)` (:570-581); per-net `NetIncompletes` airline counting; `getIncompleteCount` (:666-708). **Re-scoped 2026-09-15 after T2's spike falsified the controller's `count == groups−1` shortcut**: the Kruskal union runs over Delaunay edges of per-item `getRatsnestCorners()` POINTS (uncontacted endpoints only — Trace.java:338-368), so on routed boards fully-contacted traces contribute zero points and `count < groups−1` (witness: 655_testboard net 3, 5 groups → count 3; 74 vs 77 fixture totals). The task therefore ports the FULL algorithm: filter + connected-set grouping + per-kind ratsnest corners + `PlanarDelaunayTriangulation` (datastructures/, 978 l) + sorted-Edge Kruskal — and pins the Delaunay EDGE SET (degenerate cocircular inputs can flip diagonals), not just counts.
- [ ] `getAllClearanceViolations` (:56-87): walk item clearance violations, dedup A-B/B-A by sorted-id+layer key; output is a count + per-pair list. Depends on per-item clearance violation enumeration — T2 ports it on top of the M2 index query family (epic-board query surface; if a needed primitive is missing, add it minimally with anchors).
- [ ] Oracle: extend an existing oracle (or a small `DrcOracle.java`) — per tier fixture, PARSE-time board → both counts (parse-time boards have incompletes but zero router-introduced violations — good non-vacuous coverage for incompletes; violations path gets coverage in T15 when routed boards exist; ALSO capture on 2-3 ROUTED PCBench reference boards where violations may be nonzero — check; if reference-routed boards have zero violations by construction, pin the dedup logic with a crafted DSN instead).
- [ ] Pins: maxConnections formula on crafted nets (0/1/N endpoints; conduction-area endpoints), dedup asymmetry (A-B vs B-A insertion order), boundary cases.
- [ ] Gates + ONE commit `feat(m3): epic-drc — incompletes + clearance violation counting (parity-pinned)`.

## Task 3: AutorouteControl + the board-router seam audit

**Files:** Create `rust/crates/epic-router/` scaffold (lib.rs, control.rs); modify `epic-board/src/contacts.rs` if `get_connected_set`/`get_unconnected_set` equivalents are missing (TreeSet-by-id semantics, Item.java:650-658, 720-734).

- [ ] `AutorouteControl` (maze/AutorouteControl.java:117-231): layer masks, per-layer `traceHalfWidth`/`compensatedTraceHalfWidth`, via rule/masks + pure-SMD relaxation (`rebuildViaInfo` :234-284), `minNormalViaCost`/`minCheapViaCost`, `ripupCosts = startRipupCosts × ripupPassNo` (:46). Cost tables come from rules — consume epic-board's rules surface; where a rules accessor was CUT in the M2 review (`NetClass::is_active_routing_layer`, `BoardRules::default_net_class`, `BoardRules::via_rule_by_id`), RESTORE exactly the ones consumed, PINNED at restoration (M2 carry-forward).
- [ ] `getUnconnectedSet(net)`/`getConnectedSet(net)` equivalents in epic-board contacts (TreeSet ordered by item id DESCENDING — Item.java:95-103; plane-nets swap start/dest + `CONNECTED_TO_PLANE` short-circuit).
- [ ] **Seam audit deliverable** (doc section in epic-router lib.rs or `rust/crates/epic-router/SEAM.md`): the exact `RoutingBoard`/`board.actions`/`board.optimize`/`searchtree.completeShape` surface the router touches, with line counts, mapping each to an M2 Rust seam or a NEW port (this list sizes T4-T10; correct the plan's anchors where reality differs).
- [ ] Pins: control table construction on a real fixture's rules (jar-witnessed literals); SMD relaxation branch; ripup cost multiplication.
- [ ] Gates + ONE commit `feat(m3): epic-router scaffold — AutorouteControl + connected/unconnected sets + seam audit`.

## Task 4: epic-index `completeShape` + expansion-graph primitives

**Files:** Modify `rust/crates/epic-index/` (complete_shape.rs new); create `rust/crates/epic-router/src/expansion/` (room.rs, door.rs, neighbours.rs, target_door.rs).

- [ ] `ShapeSearchTree.completeShape` port on epic-index (board/searchtree/ — read the Java; the 45° tree's completion semantics differ from 90°; port the 45° + any-angle paths, skip orthogonal).
- [ ] Expansion rooms: `IncompleteFreeSpaceExpansionRoom` → completion via the T3-audit path (`AutorouteEngine.completeExpansionRoom`, AutorouteEngine.java:418-522); `ObstacleExpansionRoom`.
- [ ] Doors: `ExpansionDoor` common-edge + section array sized by trace width (`getSectionSegments(halfWidth)`); `TargetItemExpansionDoor` (item ∩ room shape).
- [ ] `SortedRoomNeighbours` dispatch (SortedRoomNeighbours.java:65-91) + `Sorted45DegreeRoomNeighbours` (982 l) incl. the explicit `(objectId, shapeIndex)` sort (:104-112, the v1.9-parity order) + the any-angle variant. SKIP `SortedOrthogonalRoomNeighbours` unless a Tier A fixture proves 90° (verify Tier A angle restrictions in this task; report if orthogonal is needed — scope change).
- [ ] Spike-first: a crafted two-obstacle room-neighbour case captured from the jar; pin door sections + neighbour order as literals.
- [ ] Gates + ONE commit `feat(m3): expansion graph — completeShape, rooms, doors, 45°/any-angle neighbours`.

## Task 5: drill subsystem

**Files:** Create `rust/crates/epic-router/src/drill/` (page_array.rs, expansion_drill.rs).

- [ ] `DrillPageArray` (page size `max(5×default via dia, 10000)`, invalidation on tree change), `ExpansionDrill`, candidate drill enumeration, `expandToOtherLayers` via-mask checks (MazeExpansionEngine.java:237-375).
- [ ] attach-SMD semantics per padstack (`attach_smd_allowed` exists in ViaIr from M1b — reconcile with the Java via-mask logic).
- [ ] Spike: drill-page layout + candidate set on a via-bearing fixture, jar-captured; pins on page bounds + candidate order.
- [ ] Gates + ONE commit `feat(m3): drill pages + via candidate enumeration`.

## Task 6: MazeSearchEngine core

**Files:** Create `rust/crates/epic-router/src/maze/` (search_engine.rs, list_element.rs, expansion_engine.rs).

- [ ] `init` (MazeSearchEngine.java:969-1103): destination-shape join into `DestinationDistance` is T8 — structure the seam so T8 slots in; start-room creation/completion; front seeding with `TargetItemExpansionDoor`s.
- [ ] Front = totally-ordered structure replicating the TreeSet order EXACTLY: `sortingValue → expansionValue → doorId → sectionNo` (MazeListElement.java:80-113). Rust: `BinaryHeap` cannot express stable tie handling — use a `BTreeSet` over a key tuple (f64 bits are totally ordered via `total_cmp`; document the float-order caveat vs Java's double compareTo — Java TreeSet uses compareTo which is NOT total on NaN/-0.0; determine reachable values and pin).
- [ ] `findConnection`/`occupyNextElement` (:300-384): occupancy marking, DrillPage dispatch, door-section reach → done.
- [ ] `expandToRoomDoors` (:390-626) WITHOUT ripup/shove (T7 seams stubbed): neckdown, thick/thin room classification, door-section width filter (≥ trace width), overlapping drill pages.
- [ ] Cost accumulation `expandToDoorSection` (:791-966): `expansionValue` = weighted distance + addCosts + bend penalty (:855-883); `sortingValue = expansionValue + destinationDistance(...)`.
- [ ] Spike-first: a 2-pin single-layer fixture; capture the first N front pops from the jar (the RAW_SECTION stream, MazeSearchEngine.java:907-963, is the upstream capture tool — reuse its format); pin expansion/sorting values as literals.
- [ ] Gates + ONE commit `feat(m3): maze search core — front ordering, occupancy, room expansion, cost model`.

## Task 7: ripup resolver + shove probe + Java Random

**Files:** Create `rust/crates/epic-router/src/maze/ripup.rs`, `shove_probe.rs`; possibly `rust/crates/epic-geometry/src/java_random.rs` (REUSE if a Java-Random twin already exists — recon says fixed-seed RNGs are ported; verify bit-exact LCG before writing a new one).

- [ ] `MazeRipupResolver.checkRipup` (MazeRipupResolver.java:72-197): ripup costing, fanout-protection factor, the SEEDED detour randomization (`setSeed(ctrl.ripupCosts)` MazeSearchEngine.java:63,79-80; consumed :158-163 only when `ripupPassNo≥4 && ripupPassNo%3 != 0`) — the RNG SEQUENCE must match Java's `Random` bit-for-bit if that branch is reachable in the corpus; if not reachable, port + pin the seam and document unreachability.
- [ ] `MazeTraceShover.checkShoveTraceLine` READ-ONLY probe (:1130-1201 calls into board/optimize TraceShover — port only the check path, no mutation).
- [ ] Pins: ripup-cost arithmetic, the randomization gate conditions, shove-probe accept/reject on crafted overlaps.
- [ ] Gates + ONE commit `feat(m3): ripup resolver + read-only shove probe`.

## Task 8: DestinationDistance heuristic

**Files:** Create `rust/crates/epic-router/src/maze/destination_distance.rs`.

- [ ] Verbatim double port of `DestinationDistance` (DestinationDistance.java:45-99): destination-shape join, `calculate(...)` lower-bound. Property-style pins vs jar-computed values on the T6 spike fixtures (literal capture rows).
- [ ] Gates + ONE commit `feat(m3): destination-distance heuristic (jar-pinned)`.

## Task 9: FoundConnectionLocator — backtrack + corner synthesis

**Files:** Create `rust/crates/epic-router/src/path/` (locator.rs, locator_45.rs, locator_any.rs).

- [ ] `backtrack` (FoundConnectionLocator.java:225-327): walk `backtrackDoor` chain destination→start, collect ripped items.
- [ ] Per-layer trace split `calculateNextTrace` (:410-493) + angle-restricted corner insertion (`calculateAdditionalCorner` :390-404); subclasses `FoundConnectionLocator45Degree` (357 l) + `FoundConnectionLocatorAnyAngle` (455 l).
- [ ] Corner rounding must reproduce Java's exact IntPoint sequence (rounding order is observable in the corpus — pin first/last corners + counts).
- [ ] Spike: the T6 fixtures' backtracks captured end-to-end from the jar; pin corner lists as literals.
- [ ] Gates + ONE commit `feat(m3): found-connection locator — backtrack + 45°/any-angle corner synthesis`.

## Task 10: forced insertion + shove (the cost center)

**Files:** Modify `rust/crates/epic-board/` (forced_insert.rs new; actions surface); create `rust/crates/epic-board/src/optimize/` (trace_shover.rs — check where board-optimizer code belongs per the M2 module layout; keep D17-style crate discipline).

- [ ] `RoutingBoard.checkForcedTracePolyline` + `insertForcedTracePolyline` (RoutingBoard.java:456+): forced-line insertion with shoving via `TraceShover` — port the shove path the recon sizes as deepest; the T3 seam audit's line counts decide whether this task splits (controller decision at dispatch; if `board/optimize` TraceShover exceeds ~2.5k lines, split T10 into T10a check+insert, T10b TraceShover).
- [ ] `ForcedViaInserter` + `board.insertVia` path; `connectToTrace` trace-to-trace ends; `removeTraceTails` per changed net.
- [ ] **Minimal pull-tight**: the `optChangedArea` subset needed for pass-end tail removal + post-route normalize (AutorouteConnectionRouter.java:107-113, BatchAutorouter.java:487-503) — NOT the full M4 optimizer; scope-guard the module doc.
- [ ] Spike-first: crafted shove cases (straight, corner, via-adjacent) captured from the jar (insertion result + post-normalize geometry); pins on resulting board digests.
- [ ] Gates + ONE commit `feat(m3): forced insertion + trace shove + minimal pull-tight`.

## Task 11: FoundConnectionInserter + AutorouteEngine assembly

**Files:** Create `rust/crates/epic-router/src/engine.rs` (AutorouteEngine), `src/path/inserter.rs`.

- [ ] `AutorouteEngine.autorouteConnection` (AutorouteEngine.java:130-280): maze → locate → rip (`board.removeItems` + tails) → insert (T10) per FoundConnectionInserter.java:40-111 incl. neckdown/micro-neckdown fallbacks (:201-217) and `board.normalizeTraces(net)` (:108 — the per-net `normalizeTraces(int)` sibling, BasicBoard.java:710-798, NOT `normalizeAllTraces`; port with its oscillation suppression).
- [ ] `board.initAutoroute` equivalent (RoutingBoard.java:882-897): autoroute tree selection via `getAutorouteTree` + DrillPageArray init.
- [ ] End-to-end: a 2-net fixture routes Java-identically at the DIGEST level (T15's stream makes this precise; here pin the resulting board canon).
- [ ] Gates + ONE commit `feat(m3): autoroute engine — connection routing end-to-end`.

## Task 12: pass driver + BoardHistory + deterministic budgets

**Files:** Create `rust/crates/epic-router/src/pipeline/` (batch.rs, pass_runner.rs, connection_router.rs, board_history.rs, failure_log.rs); events = no-op sink module.

- [ ] `BatchAutorouter.getAutorouteItems` (BatchAutorouter.java:345-409) insertion-order scan + unroutable/plane filters; `AutorouteConnectionRouter.route` (:30-166) incl. `retryConnectionNecked` (:168-247) + `enforceStrictDrc` (:249-253, active `isStrictDrc || ripupPassNo≥3`, rips on introduced violations).
- [ ] `AutorouteBatchLoop.run` pass loop (AutorouteBatchLoop.java:40-651) MINUS fanout (M4): stagnation/restore via `BoardHistory` (MAX_HISTORY_SIZE=30, restore by router score), constants STOP_AT_PASS_MINIMUM=8 / STOP_AT_PASS_MODULO=4 / STAGNATION_PASS_LIMIT=10 / STAGNATION_SCORE_THRESHOLD=0.5F (BatchAutorouter.java:46-60) — pin each.
- [ ] **Deterministic budgets**: replace wall-clock `TimeLimit 100000·2^(pass-1)` ms (AutorouteConnectionRouter.java:72-74) and `optChangedArea` TIME_LIMIT=1000 with deterministic item/expansion counters behind a `DeterministicBudgets` flag; document the deviation (Java wall-clock is the ONLY run-to-run variance source — recon §2); `isStopRequested` (AutorouteEngine.java:294-304) consumes the budget.
- [ ] Router score for BoardHistory: epic-drc T2 counts + trace/via/bend costs (BoardStatistics.java:678-686 legacy formula — port the legacy default only).
- [ ] Gates + ONE commit `feat(m3): batch driver — pass loop, stagnation/restore, deterministic budgets`.

## Task 13: epic-cli `route` command

**Files:** Create `rust/crates/epic-cli/` (main.rs, route.rs, settings.rs); Cargo: epic-cli → {dsn, board, index, drc, router}.

- [ ] Settings subset resolver: trace/via costs, layer masks, max passes/items, ripup start, angle restriction (from DSN rules via epic-board) — mirror the SettingsMerger PRECEDENCE semantics for the CLI-source subset only (sources/Java: settings/sources/CliSettings.java:59+); `-de/-do`, `--router.*` overrides, result-JSON flag emitting the manifest schema the harness parses (baseline.rs).
- [ ] Single-threaded; deterministic budgets default-ON in the corpus profile, wall-clock available behind a flag.
- [ ] Smoke: route a small DSN headless → SES out + manifest; assert manifest fields.
- [ ] Gates + ONE commit `feat(m3): epic-cli route — headless DSN→route→SES+manifest`.

## Task 14: router-only oracle baselines re-capture

**Files:** Modify `rust/harness/` (baseline capture path + `rust/harness/baselines/` new router-only set); oracle runs the JAVA jar with fanout+optimizer DISABLED.

- [ ] Verify the exact CliSettings disable flags (recon uncertainty (c)); PROVE both stages off via the manifest (zero optimizer passes, zero fanout activity) before capturing.
- [ ] Capture per Tier A fixture: incomplete_count, clearance_violations_total + router_introduced, normalized_score, optimizer_score (expected ~neutral), passes_completed, ses_sha256, phase seconds. Commit as `rust/harness/baselines/router-only/` (regeneration discipline: harness subcommand + justification).
- [ ] Gates + ONE commit `feat(m3): router-only Java baselines — Tier A re-capture (fanout/optimizer off)`.

## Task 15: directional compare gates + determinism self-gate

**Files:** Modify `rust/harness/src/` (router_compare.rs new; corpus_common shell from T1); CI step in `rust-check.yml`.

- [ ] `router compare`: per Tier A fixture — `incomplete ≤ Java`, `violations_total ≤ Java`, `score ≥ Java − ε` (relative ε); report (not gate) `router_introduced` (preferred 0) + `passes_completed`; trace/via/bend count deltas reported with tolerance bands.
- [ ] `router determinism`: same DSN → byte-identical SES + manifest across 2 runs (java-free).
- [ ] First-divergence localizer: on gate failure, print the fixture's first differing component (incomplete by net, violation pairs, score decomposition).
- [ ] Gates + ONE commit `feat(m3): router compare gates — directional quality + determinism`.

## Task 16: single-net event-stream micro-corpus

**Files:** Create `rust/harness/oracle/RouteEventOracle.java` (or extend — the upstream `RAW_SECTION`/`compare_trace_*` FRLogger streams, AutoroutePassRunner.java:250-257 + MazeSearchEngine.java:907-963, are the capture format); `rust/harness/src/route_events.rs`; small deterministic fixture set (2-5 nets; the bm01 2-item slice is the model).

- [ ] Java capture: per connection — destination door id/section, assign/skip events with expansion/sorting values, ripped-item id set, per-trace corner counts + first/last corners. Deterministic-budget profile on the Java side too (the wall-clock TimeLimits are the variance source — if the Java jar cannot be made deterministic on the chosen fixtures, pick SMALLER fixtures where passes complete far under budget; prove run-to-run stability by double-capture + diff).
- [ ] Rust mirror: emit the same stream from epic-router; compare field-by-field; first-divergence localizer.
- [ ] Pins: literal stream rows for the branch-critical events (first ripup, first neckdown, a tie-break flip).
- [ ] Gates + ONE commit `feat(m3): route event-stream corpus — maze-level differential parity`.

## Task 17: Tier A quality close + milestone exit

**Files:** `rust/README.md`, design §6 M3 footnote, `docs/architecture.md` (epic-router/epic-drc/epic-cli entries), `.wolf/STATUS.md` via /handoff (controller-side).

- [ ] Full gate battery: fmt, clippy `-D warnings`, `cargo test --workspace`, all six M2 compares + router compare + determinism + event-stream corpus.
- [ ] Close Tier A gaps found by the gates (every fix re-runs full gates; no ungated "tuning").
- [ ] Docs updates (numbers verbatim from the gate capture); ONE commit `docs(m3): M3 exit — README/design/architecture, full gate evidence`.
- [ ] Controller dispatches the whole-milestone review (spec vs design M3 exit criteria, then quality); fix findings; re-run gates; /handoff; close.

---

## Standing per-task process (applies to every task)

1. Dispatch: fresh implementer per task; controller stages a detailed brief at `logs/M3-T<n>/dispatch-prompt.md` (recon anchors above are the starting point; controller re-verifies load-bearing anchors before staging). Spike-first wherever a jar capture is listed; capture to logs/, run twice + diff.
2. Reviews: sonnet-tier spec review → fix round → opus-tier quality review → fix round → close. Implementers are resumable via SendMessage; never parallel implementers; never skip re-reviews.
3. Gates each task: `cargo fmt --all` + `--check`; `cargo clippy --workspace --all-targets -- -D warnings` (incl. `clippy::unwrap_used`); `cargo test --workspace` (baseline = prior close report; additions only); the six M2 compares stay green.
4. Discipline: ONE commit per task with trailer; explicit paths (never `git add -A`); `.wolf/` only in chore(wolf) commits; log bugs in `.wolf/buglog.json` (working tree); triage divergences BEFORE fixing; BLOCKED with witness if unresolvable; fish-shell `cd` every Bash call; JDK 25 for oracles.
5. Pin rules: cerebrum modes 1-9 (literal capture rows; mutation-verified; contrast witnesses; no memoized-accessor re-query; winner-selection inputs).

## Self-review notes (controller, 2026-09-15)

- Spec coverage: design §6 M3 row → T11 (engine), T12 (driver), T13 (cli); exit criteria 1→T14+T15, 2→T15, 3→T16, 4→standing gates. Carry-forwards → T1 (shell), T3 (cut items restored pinned), undo-topology note (not triggered — no undo corpus change planned).
- Placeholder scan: T10's split condition is an explicit controller decision point, not a placeholder; anchors marked "re-verify" are recon-verified but jar-wins.
- Type consistency: crate names epic-router/epic-drc/epic-cli consistent; `DeterministicBudgets` named in T12/T13; `corpus_common` named in T1/T15.
- Known risks (tracked at dispatch): T4 orthogonal-neighbor scope surprise; T6 float tie-break totality; T7 Java-Random reachability; T10 size (split rule defined); T14 flag verification.
