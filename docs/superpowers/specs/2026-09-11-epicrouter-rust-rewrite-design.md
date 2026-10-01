# EpicRouter: Rust Rewrite Design

**Date:** 2026-09-11
**Status:** Approved design (pending implementation plan)
**Baseline:** upstream `freerouting/freerouting` master @ `e7f9bdf1` (post-2026-refactor; benchmarks tag the parity baseline as v2.3.0+)
**License:** GPLv3 (a translation/rewrite of GPLv3 code is a derivative work and stays GPLv3; per-file attribution preserved)

## 1. Mission

Rebuild the Freerouting PCB autorouter from the ground up in Rust so that routing is dramatically faster, more accurate, more capable, and produces boards that look like a human routed them — **with zero regressions** against the Java baseline, enforced by machine-checked gates rather than intent.

Four first-class feature areas (user-selected, all are milestone drivers):
1. **Speed + DRC foundations** — deterministic parallelism, arena/SoA data layout, exact incremental DRC as the single violation authority.
2. **Plane/pour awareness** — pours as first-class connection targets; no pour fragmentation; floating-island detection.
3. **High-speed constraints** — differential pairs, length matching, serpentine/trombone tuning, net-class priorities.
4. **Aesthetics/gloss** — bus hugging/parallel discipline, smooth 45° flow, minimal well-placed vias, teardrops.

## 2. Research summary (evidence base)

Three research streams (full citations in `docs/superpowers/research/` companions; key facts here):

**Freerouting gaps (community/issue evidence):** copper-pour/plane awareness is the #1 most-upvoted open feature (upstream issue #152, open 4 years, currently introduces clearance violations — Issues 093/152); diff pairs + length matching were requested and closed unimplemented (#133, #716); multithreaded optimization is documented to generate clearance violations (#103); internal DRC misses violations the maintainer acknowledges (#508); routing is effectively single-threaded (#289, ~11% CPU on 8 cores) with 56-hour optimizer runs reported; net-class priorities ignored (#507); layer-direction preferences violated (#230, #689); keepouts broken (#185); completion ~51% and DRC-clean ~40% on the 1,157-board PCBench corpus for v2.5-RC2 (and v2.5 is *slower* than v1.9 — 4,329s vs 2,575s total on the corpus).

**State of the art:** the highest-leverage classical techniques are (1) PathFinder-style negotiated congestion (persistent per-resource history costs) as the rip-up scheduler — the most validated convergence mechanism in the field, absent from Freerouting (it uses linear pass-scaled costs + snapshot restore, prone to oscillation); (2) a global-routing stage (coarse congestion-aware planning) before detail routing — a DATE 2023 paper doing exactly this (pad-focused polygon partition + planning + A* detail) *beat Freerouting* on success rate/wirelength; (3) bus/hug/group routing (Pulsonix detects nets following similar paths and routes them as a group; Xpedition hug routing; Specctra `spread` pass) — the biggest single "looks human" win; (4) diff-pair/tuning engines (pose-based A* with Dubins heuristic; MSDTW median-trace length matching, 2024); (5) gloss pass suite (Specctra's documented `spread → miter → recorner` ordering; Xpedition centering/unused-pad removal; KiCad 8 teardrops); (6) collision-aware parallel rip-up-reroute (NCTU-GR 2.0); (7) incremental spatial-index-driven DRC (OpenDRC/HeteroDRC architecture). The 2026 PCBWorld benchmark (20k synthetic + 679 real boards) shows classical engines still beat RL/LLM routers on real boards — the design stays classical; ML is out of scope beyond an optional future congestion-forecast experiment.

**Language/architecture evidence:** the workload (boxed red-black-tree priority queue with per-insert allocation in `MazeSearchEngine`, pointer-chasing `ShapeTree` spatial index, exact `BigInteger` rational geometry, documented multi-GB cumulative allocation churn per job) is the worst case for GC and the best case for arena/SoA native code: realistic 3–10× from data-layout alone, 2–6× more from deterministic net-level parallelism, 2–4× on SIMD-able kernels. Rust chosen over C++ for memory-safety-under-refactor, `cargo-fuzz`/`proptest` tooling (the parity harness), trivial C ABI, and cross-platform packaging. Known porting traps and mitigations are cataloged in §9.

## 3. Decisions (user-approved)

| Decision | Choice |
|---|---|
| Rewrite scope | **Full rewrite including GUI** (Java tree stays untouched as the parity oracle until M10) |
| Language | **Rust everywhere** (core + CLI + GUI via egui/wgpu) |
| Strategy | **C: port the correctness kernel (geometry + DSN/SES I/O), reinvent the router** (modern staged architecture above proven-exact foundations) |
| Feature scope | All four areas above are milestone drivers |
| API/MCP parity | Deferred past 2.0 |
| ML components | Out of scope |

## 4. Architecture

### 4.1 Workspace layout

```
rust/
├── crates/
│   ├── epic-geometry/   exact 45° kernel: IntPoint/IntBox/IntOctagon/TileShape/Polyline;
│   │                    i128 fast path + BigRational fallback; checked arithmetic everywhere
│   ├── epic-dsn/        Specctra DSN/SES reader+writer; semantically-normalized round-trip parity
│   ├── epic-board/      board model: items, layers, nets, clearance classes, pours, keepouts;
│   │                    arena-allocated, SoA, snapshot/undo
│   ├── epic-index/      spatial index: arena BVH/quadtree with incremental insert/remove
│   │                    (the rip-up hot path), per-clearance-class compensation
│   ├── epic-drc/        incremental exact DRC; single violation authority (no incomplete shortcuts)
│   ├── epic-router/     staged router (below); deterministic parallelism primitives
│   ├── epic-engine/     jobs, layered settings (defaults→JSON→DSN→env→CLI→GUI, same precedence
│   │                    semantics as Java SettingsMerger), progress/cancel, seed control
│   ├── epic-cli/        `epicrouter -de board.dsn -do out.ses` drop-in CLI
│   └── epic-gui/        egui frontend, GPU board canvas
└── harness/             differential parity + benchmark harness (Rust binary orchestration;
                        replaces the Windows-only PowerShell scripts)
```

The existing Java tree is not modified during the rewrite (it is the oracle); it is deleted at M10.

### 4.2 The staged router pipeline

All stages read/write one board model, run deterministically in parallel per-net/per-region, and each has its own scoring. Stage order:

1. **Fanout/escape** — congestion-map-aware pin ordering (fixes the documented adjacent-QFP mutual-blocking failure); BGA channel-aware escapes; dog-bone/via-in-pad/under-pad strategies; ordered-escape planning for dense components.
2. **Global routing** — coarse congestion estimation; **PathFinder negotiated congestion** (per-resource history costs persist across iterations); pattern routing (L/Z 1–2 bend routes for easy nets) inside guides; produces per-net plans (regions/layers/expected topology) that detail routing honors. Net ordering becomes a planned decision, not DSN file order.
3. **Detail routing** — exact octagon maze search (A* with admissible heuristic over expansion rooms — the ported kernel discipline) honoring global plans; **push-and-shove** of neighbors (not only rip-up); via cost shaped by reference-plane continuity.
4. **Plane routing** — pours as connection targets (one via to plane, cheap plane-via cost); pour-fragmentation avoidance; floating-island detection (upstream cannot do this at all).
5. **Tuning** — differential pairs (coupled topology, pose-based A* with Dubins heuristic); length matching against constraint targets; serpentine/trombone/accordion meander insertion with clearance-aware accommodation (MSDTW median-trace technique where applicable).
6. **Gloss** — bus/hug/parallel-group routing (topological-similarity group detection → shared corridor → uniform spacing); Specctra-style `spread → miter → recorner` passes; 45° flow smoothing; jog/stub elimination; via *placement* optimization (return-path aware); teardrops.

**Scoring:** replaces completion-rate-only objectives. Stage-level + final board score over: completion, exact DRC count, length excess, via count *and* via quality, bus parallelism ratio, pour integrity, tuning compliance. 0–1000 scale for benchmark comparability with upstream.

**Determinism contract:** fixed seeds, fixed net ordering by ID, deterministic reduction order in parallel sections; `--threads N` output is byte-identical to `--threads 1` (CI-enforced). This preserves the v1.9/v2.x reproducibility semantics the Java engine has (`Random(ripupCosts)` seeding).

**Clearance model:** Minkowski-sum clearance inflation (obstacle ⊕ clearance halo → overlap tests) as today, built on the exact geometry kernel; per-clearance-class compensated indexes as today.

## 5. Parity & benchmark harness (the zero-regression guarantee)

Exists before any router code. Corpus:
- **Tier A/B/C fixtures** (existing benchmark tiers; Tier A = 2-layer canary that must stay 100% clean).
- **PCBench corpus** (1,182 professionally-routed KiCad boards; unrouted variants are inputs; professional routes are ground truth for aesthetics metrics: mean length excess, via density, parallelism ratio, bend-to-length ratio).
- **Golden baselines** from the Java engine (built from this tree at the frozen baseline commit `e7f9bdf1`) per fixture × settings matrix: SES, per-pass incomplete counts, scores, DRC counts, wall time (versioned; regenerable via one harness command).

Gates (CI-enforced per commit touching `rust/`):

| Gate | Rule |
|---|---|
| Geometry parity | Property tests + cross-check vs Java on random shape corpora; exact match (or documented ULP epsilon for double heuristics) |
| I/O parity | DSN→internal→SES round trip vs Java on every fixture; semantically identical (normalized structured comparison) |
| Index parity | Same insert/remove sequence → identical query result sets vs Java tree |
| Router gates | Every fixture: completion ≥ Java, violations ≤ Java, score ≥ Java; time is a tracked trend, not a correctness gate |
| Determinism | `--threads 1` ≡ `--threads N`, byte-identical SES |
| No-slow-regression | From M5: Tier A wall time ≤ 0.5× Java, ratcheting down per milestone |

Divergence triage: structured routing event streams (`RAW_SECTION` door/section/cost traces) replicated in Rust behind a debug flag; first diverging net localizes the bug; classify numeric-drift vs tie-break-ordering before fixing (upstream's own discipline, inherited).

## 6. Milestones

| # | Milestone | Exit criteria |
|---|---|---|
| M0 | Fork setup: Rust workspace scaffold, harness skeleton, golden baselines from Java | Baseline manifests for Tier A/B/C; `cargo test` + `./gradlew test` green |
| M1 | `epic-geometry` + `epic-dsn` | Geometry parity green; DSN parses all fixtures (the planning-time "162" was a stale census — the enumerated corpus is 175 digest + 1,157 soak, all green; see the M1b plan D10) ; SES round-trip parity on Tier A |
| M2 | `epic-board` + `epic-index` | Index query-set parity; snapshot/undo correct |
| M3 | Detail routing core + `epic-cli` | Single-net quality ≥ Java; Tier A per-fixture completion ≥ Java and violations ≤ Java |
| M4 | Full pipeline (fanout + detail + optimizer) | **Zero-regression moment**: all fixtures ≥ Java on completion/violations/score |
| M5 | Performance: arena/SoA + deterministic parallelism | Tier A ≤ 0.5× Java wall time; large-board trend toward 10×; threads-invariance byte-identical |
| M6 | Plane-aware routing + congestion-aware global stage + push-and-shove | Tier B/C completion **strictly >** Java; Issue093-class violations = 0; island detection works |
| M7 | Tuning: diff pairs, length matching, serpentine | DSN-declared constraints honored; tuning DRC-clean on fixture set |
| M8 | Gloss: bus hugging, 45° flow, via placement, teardrops | Aesthetics metrics improve monotonically vs PCBench ground truth; no completion/DRC regression |
| M9 | `epic-gui` (egui) | Load any fixture, route, export SES — full workflow without Java |
| M10 | 2.0: sunset Java oracle, release | Docs, Linux packaging first, migration guide |

M2 exit note (T16): exit criteria met — index query-set parity 33/33 and
snapshot/undo replay 33/33 on the tier + stressor corpora, 927 workspace
tests green, all M1 gates still green (geometry 5,000/5,000; DSN 1,332
all-match with an empty divergence ledger since D22; SES 20/20
byte-equal). Two deviations on record: (a) the ses-snap corpus pins the
endpoint-snap rule with snapFired=0 — its output-changing arm is
structurally dead upstream (class-strict `Point.equals` puts every drill
contact at distance exactly 0.0, so the ≤0.5 at-center arm always wins;
pinned as data); (b) the undo digest's >400-pair cliff (the golden's
tree-pair list goes null above 400 pairs; the sha still carries the
content) is mirrored as-is — a diagnosability trade-off accepted at T14.

M3 exit note (T17, adjudicated 2026-09-22 at tree `a4c072430`):
**M3_EXIT_WITH_DEVIATIONS**, adjudicated on this row's own faces.
Criterion 1 (single-net quality ≥ Java) met with deviation: the
maze/ripup/shove/locator/forced-insertion decisions are
jar-capture-pinned and the T16 event-stream corpus shows Rust ≥ Java on
every measured outcome row (on t7 the Rust engine routes a net the Java
golden reports unconnected), but the stream's divergence classes
(parse-time item ids, expansion-door slicing, insert-path id churn;
buglog 172–174) are open as #61 — scheduled early M4 — so the criterion
is not met clean. Criterion 2 (Tier A per-fixture completion ≥ Java,
violations ≤ Java) not met: battery 8/11, with violations ≤ Java and
zero router-introduced violations on all 11 fixtures, and three
completion reds — each root-caused non-parity:

| Red | Root cause | M4 lever | Buglog |
| --- | --- | --- | --- |
| bm06 completion 10 vs Java 9 | TUNING: attempt-42 fork (Java rips, Rust finds a legal rip-free path), then victim-choice swaps on bistable nets | victim-choice tie-break alignment, sequenced after #61's door-slicing class (Class B's slicing may be the causal ancestor of the victim-choice drift — closing it first is cheaper than tuning on unaligned order) | 181 |
| bm11 completion 15 vs 14 at truth | attempt-order/victim-choice drift; plateau score-stable, same 18 passes as Java — the wall is not the cause | same tuning family as bm06 | 175 |
| bm01 completion 5 best / 6 terminal vs 2 | WALL-LIMITED: every pass exhausts the tick ladder; the best measured state is 3 connections short of Java's 2 with the score still climbing at the bound — reaching 2 is extrapolation, not measurement | per-pass search cost (M4 detail-stage work; M5 arena/parallelism as backstop) | 175 |

The standing §5 zero-regression guarantee held: all six M2 compares
green (geometry 5000/5000; dsn 1,332; ses 20; index 33; undo 33;
ses-snap 5), 1,305-test census, determinism digests stable. Two
plan-level supersets tracked by the M3 plan are NOT §6 criteria (SEAM
criterion map) and are recorded here only to keep the vocabularies
apart: the score gate (R ≥ J − 2%), which bm01 fails, and
events-corpus equality — the corpus is honest-red by charter, a triage
instrument (§5), not a gate. The CI router gate stays HELD in
`--report-only` (a tripwire-pinned deviation from §5's CI-enforced
letter): the flip is one same-commit act once the battery reads 11/11
under a wall-resolved profile — drop `--report-only`, update both
`ci_tripwire.rs` pins, and refresh the prose describing the posture
(the rust/README status block and gates comment, and this paragraph);
that moment is M4's zero-regression moment. Faces and ledger:
`rust/crates/epic-router/SEAM.md` (MILESTONE-CRITERION MAP). (M4
UPDATE, T13 close: the HOLD continues — the flip was adjudicated unmet
on both preconditions at the M4 close: the milestone full-face battery
reads 8 PASS / 3 RED (buglog 175/181/189;
`logs/M4-T12/evidence/battery_full_t12.log`) and its suite wall
(2061.2 s release ≈ 34 min) exceeds the compare step's 30-minute
budget (the compare step's `timeout-minutes: 30`, now at
`rust-check.yml:125` after the T13 comment refresh). The events-compare step also stays out
of CI until the same flip commit brings it with its own pin. M5 flip
conditions: full battery 11/11 AND wall under budget. The M4 exit note
below carries the full record.) (M5 UPDATE, T9: the HOLD continues —
adjudicated NO-GO on both preconditions at the M5-T8 close
(`logs/M5-T8/report-t8.md`, §(e)): the milestone full-face battery
reads 8 PASS / 3 RED (`logs/M5-T8/evidence/battery_full_t8.log`; Σ
wall 2286.1 s harness-elapsed ≈ 38.1 min > the compare step's 30-min
budget, `rust-check.yml` `timeout-minutes: 30` — at `:139` in this
tree, post-T9-comment-refresh; the M4 note's `:125` anchors below are
T13-era numbering) — the hold's
composition changed, not its verdict: the maze-search wall collapsed
(bm01 routing now completes 0/0/1000.00 in ~276 s; bm11 600.2→197.7 s;
bm06 80.0→25.0 s), but the T1b-parity-corrected optimizer stage is now
the binding face on bm01 (killed at the 1800 s tier on a
non-improvable 0.00-score incumbent). The named deviations rows,
compactly (the M5 exit note will expand them): criterion 1 **MISS** —
Σ 2286.1 s harness-elapsed = 2.02× Σ-Java 1130.39 s vs the ≤0.5×
target (565.19 s); like-for-like per-fixture excl-bm01 256.7 s =
2.42× (both bases labeled, per finding MINOR-1); criterion 2 **NOT
MET** — the speedup collapses on the large boards (bm11 0.24×, bm06
0.69×; bm01 ≤0.57× kill-truncated) while the M4→M5 compression is
real progress: bm11 ≥12.5×→4.13×, bm06 4.63×→1.45×, bm07 7.74×→0.58×
slowness; criterion 3 **MET** — threads-invariance byte-identical at
the T7 hashes (bm08 `dc271114…`/`cf607714…`, bm06
`77caf2099a2f…`/`90f08ad2…` across `-mt 1/3/4`); the three reds —
bm01: optimizer-stage wall kill at the 1800 s tier with routing
COMPLETE (the optimizer-entry question — should the optimizer be
entered at all on a 1000.00/0/0 incumbent, Java's preflight guard —
plus the tier-bound question) → M6; bm06 3>2 and bm11 4>3
incompletes: the buglog-181 completion residuals → M6. The
threads-gate CI wiring is RE-DEFERRED to the M6 flip-adjacent moment
(the gate stays pinned in-tree, green and hashed.) (M6 UPDATE, T4:
the HOLD continues — adjudicated NO-GO (honest hold) on both
preconditions at the M6-T3 close (`logs/M6-T3/report-t3.md`): the
Tier A battery fails the harness verdict face — **10 green / 1 red**
(bm01's 1800.2 s tier kill on the INTEGRITY gate: harness timeout, no
manifest, exit None; NOT a completion failure — pre-kill router face
0-unrouted/1000.00, optimizer P1 571.93 s REJECTED
`OPTIMIZER_SCORE_NOT_IMPROVED`, incumbent restored) — while the
completion-only reading passes it (**11/11**; both faces are named
because the flip precondition is judged on both, closing the plan's
"expect 11/11" self-contradiction); and Σ Tier A = **2089.5 s** >
the compare step's 30-min budget (`timeout-minutes: 30`), over by
289.5 s = 16.1%. Either failing ⇒ NO-GO; both fail. Reopen bound:
the other ten walls sum 289.3 s — the flip reopens exactly when bm01
completes < **1510.7 s**; the named lever is deterministic candidate
parallelism (buglog 197; `DefaultSettings.java:182`
`optimizer.maxThreads = cores−1` vs Rust's mandated 1-thread face),
recorded NOT attempted. The bound-rise option (the prior comment's
"or this bound to rise") is adjudicated NOT taken — the wall drops,
the bound does not rise, the same discipline as the tiers.yaml
timeouts. The threads-gate CI wiring is re-deferred WITH CAUSE — it
lands with the flip commit; the cause is that the flip preconditions
failed at M6-T3. The full hold record lives in the rust-check.yml
compare-step comment (refreshed by this task — the M5 note's `:139`
timeout anchors are pre-refresh numbering) and the SEAM M6 hold
posture table. Post-refresh line anchor (verify at the committed
tree): the compare step's run line is `rust-check.yml:169`, its
`timeout-minutes: 30` at `:170` — the M5 note's `:139` anchors are
pre-refresh numbering.) (M8 UPDATE, T8 — THE FLIP
LANDED: adjudicated GO on the arithmetic at tree `9fe3a0751`
(`logs/M8-T8/report-t8.md` §5, evidence `logs/M8-T8/evidence/
80-gate-laps.log`): the EXACT post-flip CI face (router-only profile,
mode: gate) read **11 green / 0 red, Σ 286.5 s** (DNR-18-reconciled,
exit 0; the CI debug-history datum 337.7 s, M4-T6) — 5.3–6.3× headroom
under the compare step's 1800 s budget; strip context: the M6-T3
reopen frame's full-flow Σ with the T7 lever (bm01 ot=4 1340.78 s +
the 285.7 s ten-fixture strip = 1626.5 s) also fits — the T7 reopen
verdict stays arithmetically open, with the honest caveat that the
lever is opt-in and NOT load-bearing for the CI step (the router-only
profile never engages the optimizer). The full-profile held face held
exactly (10 green + the bm01 chronic cap-scrape at mode: report; the
same red in mode: gate — the deterministic budgets do NOT
preflight-skip bm01's optimizer). The one-commit protocol executed:
`--report-only` dropped, both `ci_tripwire.rs` pin faces updated (the
compare-step pin now pins the exact GATE argv, the events step joins
with its own pin asserts in the same test fn — census unchanged, no
new test count), the events-compare step added with a 10-minute wall,
and the posture prose refreshed (this paragraph, the rust/README
status block + gates comment, the SEAM posture row,
docs/architecture.md). The four prior holds (M4 close, M5-T8, M6-T3,
M7) stand as history — the fifth moment was the flip. ANCHOR NOTE:
the pre-existing line anchors in this paragraph-chain — the compare
step's run line `rust-check.yml:169`/`timeout-minutes: 30` at `:170`
(:229-231 above) and the M5 note's `:139` (:373) — are PRE-FLIP
numbering; post-flip the compare step sits at `:133`/`:134` and the
events step at `:140`.)

M4 exit note (T13, adjudicated 2026-09-24 at tree `6a0dbd00b`):
**M4_EXIT_WITH_DEVIATIONS**, adjudicated on this row's own faces.
Criterion 1 (the full pipeline runs — fanout + detail router +
optimizer assembled end-to-end) **MET**: `pipeline/full.rs` mirrors
`RoutingPipeline.run()` (M4-T10), the router-compare `--profile full`
face gates the assembled pipeline against the M0 full-flow jar records
(M4-T11 — reuse, not recapture: all 11 `fixture_sha256` re-verified),
and `epic-cli` drives the whole pipeline.

Criterion 2 (all fixtures ≥ Java on completion/violations/score — the
zero-regression moment) **NOT MET**: the full-face battery reads
8 PASS / 3 RED (exit 1, 2061.2 s;
`logs/M4-T12/evidence/battery_full_t12.log` — T11's face reproduced
exactly at T12):

| Red | Face (inc/viol/score, wall) | Root cause | Owner | Buglog |
| --- | --- | --- | --- | --- |
| bm01 | 1/0/993.16, 20 passes, 673.6 s — completion by 1; score/violations green; NOT wall-limited (stagnation-stopped, passes 12–20 flat) | the victim-choice/attempt-order family at the router's plateau, upstaged upstream by the 189 completion-face divergence (lever (c) re-derives 181's fork evidence after 189 closes) | M5 | 175/181 → 189 |
| bm06 | 5/8/930.81 vs jar 2/8/971.63 — incompletes 5 > 2 AND score < J−ε; violations green 8 ≤ 8 | a 500-DBU expansion-room completion divergence at the FIRST fanout search (start-room north face −911636 jar vs −911136 Rust; compensated outline-band mechanism candidate, `getHash()` reproduction oracle) — upstream of every maze fork | M5, levers (a)/(b)/(c) | 189 (181's family downstream) |
| bm11 | wall-integrity red: harness-killed at 600.2 s, no manifest; the plateau AT the kill is 975.00/3/0 — Java's exact terminal face | per-pass search cost (62–64 s/pass vs Java's ~21 s TOTAL); the stagnation stop is Java-exact (`BatchAutorouter.java:46-60`, `AutorouteBatchLoop.java:450-543`) and simply had not fired yet | M5 (arena/SoA + deterministic parallelism) | 175 |

Criterion 3 (the standing §5 zero-regression guarantee — every compare
green, no regression on the M2/M3 faces) **MET** at the T12 close:
corpus 5000/5000; dsn `--set all` 1,332/0; ses 20/20; ses-snap 5/5;
index 33/33; undo 33/33; drc 17/17; events 3,520 golden rows aligned,
exit 0; `router determinism` ×2 byte-equal with the canaries UNROTATED
(ses `253e7b14…`, manifest `8c4a9773…`); census 1422/0/17.

The CI flip was adjudicated HELD at the M4 close — both preconditions
unmet: (a) the battery reads 8/3, not 11/11; (b) the suite wall
(2061.2 s release ≈ 34 min) exceeds the compare step's 30-minute CI
budget (`rust-check.yml:125`, post-comment-refresh numbering).
`--report-only` stays; both
`ci_tripwire.rs` pins are intact. Face separation (do not substitute
vocabularies): the CI step's own router-only profile has read 11/0 in
337.7 s since M4-T6 (`logs/M4-T6/evidence/cmp_router_t6.log`) — that
is NOT the flip criterion; the flip is judged on the milestone's
full-face battery, the §6 zero-regression face. M5 exit conditions for
the flip: the full battery reads 11/11 AND its wall fits the CI
budget; the flip commit also brings the events-compare step (java-free,
green since M4-T6) with its own pin.

Carry-forwards (every line traces to a dossier/entry): the buglog-189
levers (a) live-process recording instrument, (b) the `getHash()`
reproduction oracle + CLI-state diff, (c) re-derive the 181 fork
evidence after the completion face aligns — M5; bm11/bm01 per-pass
search cost — M5 arena/SoA + deterministic parallelism; bug-186 — port
BOTH DSN comment faces (EOL `#` + TraditionalComment) into epic-dsn
with the corpus-neutrality proof and a truncation-loudness companion —
M1b-reopen; the T10-banked optimizer stop faces stay banked (not
involved in bm11's red — verified T12); SEAM BACKLOG: the FORMAT
tripwire + the enumeration-order pin (both wait on the `events
id-order` capture pipeline); the keep_point sixth-field fresh-face
candidate (the RETAINED-fields checklist,
`epic-board/src/trace_tightener/mod.rs` `with_fresh_algo_face` doc);
the design §6 dead-hypothesis annotations — the M3 note's "Class-B
slicing may be the causal ancestor" cell is superseded (T2
disconfirmed the door-slicing premise; T12 located the ancestor one
stage earlier, the 189 completion face) and THIS note supersedes it,
the M3 table stays as landed; M5 checklist — ExpansionDoor tag
construction order under parallelism (`expansion/door.rs`
`DOOR_TAG_COUNTER` single-threaded-determinism note) and T9's F3 exit
condition (the guard-4 through-hole-pin arm closes with a crafted
through-hole-pin fixture); the T2 BACKLOG pair on the same file (the
unpinned `getSectionSegments` EMPTY door-shape and both-complete-NULL
arms, SEAM :728) pins at that door.rs edit.

M5 exit note (T10, 2026-09-26; close-out docs commit as sole child of
`8950deba9` — the measurement record predates this note: T8 measured
at `977e07a47`, the posture record is T9's `e6302bb3e` plus its
quality round):
**VERDICT: M5_EXIT_WITH_DEVIATIONS**, adjudicated 2026-09-26 at tree `bff565249` (fresh-eyes adjudicator): criteria 1 + 2 NOT MET on the T8 record (Σ 2286.1 s harness-elapsed = 2.02× Σ-Java 1130.39 s vs ≤ 565.19 s; speedup peaks small and collapses large), criterion 3 MET (the T7 hashes exact); all three reds owned by M6 via buglog 196 (bm01 optimizer-entry/tier-bound) and 181 (bm06/bm11 completion); the zero-regression guarantee held; the flip stays HELD on both preconditions (8/11; 2286.1 s ≈ 38.1 min > 30 min).

Criterion 1 (Tier A Σ wall ≤ 0.5× Java — Σ `autorouter_seconds +
optimizer_seconds` over the 11 Tier A fixtures; the §5 :98 ratchet's
first notch) **NOT MET**: Rust Σ 2286.1 s harness-elapsed = 2.02×
Σ-Java 1130.39 s (independently re-derived from the committed
`baselines/java/A/**` JSONs) vs the ≤ 565.19 s target — 4.04× the
target. Decompositions, both bases labeled (the MINOR-1 discipline;
the verdict is invariant under either basis): excl-bm01 on the
harness-elapsed basis 486.0 s = 4.59× vs Σ-Java-minus-bm01
105.93 s; like-for-like per-fixture basis 256.7 s = 2.42×. bm01
autoroute-only ~276 s vs Java 111.76 s = 2.47× (diagnostic column).
Evidence: `logs/M5-T8/report-t8.md` §(a)/(b);
`logs/M5-T8/evidence/battery_full_t8.log`.

Criterion 2 (large-board trend toward 10× — the criterion is the
TREND, stated with numbers, not a threshold) **NOT MET**: speedup
(Java/Rust) peaks on SMALL boards (bm08 4.51×, bm02 2.31×, bm07
1.73×) and collapses on the large ones (bm06 0.69×, bm09 0.57×,
bm11 0.24×, bm01 ≤0.57× kill-truncated). The M4→M5 compression is
the progress sentence, distinct from the verdict: large-board
slowness compressed 3–4× (bm11 ≥12.5×→4.13×, bm06 4.63×→1.45×,
bm07 7.74×→0.58×). Evidence: `logs/M5-T8/report-t8.md` §(c).

Criterion 3 (`--threads N` output byte-identical to `--threads 1`,
pinned in-tree) **MET**: `router threads-invariance` re-faced at T8
(fresh bins): exit 0, 6 runs 127.7 s, ses `77caf2099a2f…` /
manifest `90f08ad2b94d…` byte-identical across `-mt 1/3/4` — T7's
pinned hashes EXACT. The gate's CI wiring is RE-DEFERRED to the M6
flip-adjacent moment (the plan's NO-GO branch is one docs-only
commit); the gate stays pinned in-tree, green and hashed.
Evidence: `logs/M5-T8/report-t8.md` §(d); the T7 dossier.

Inherited completion parity (the flip's precondition family): buglog
189 CLOSED at M5-T3 (fix commit `74728b887`, the copper-to-edge
override; the sanctioned canary rotation `dc271114…`/`cf607714…` is
recorded there) — but the full battery reads 8/11, not 11/11: bm06
3>2 and bm11 4>3 incompletes are the buglog-181 completion residuals
(owner M6), and bm01's red is the optimizer stage, not a completion
face. The standing zero-regression guarantee held (criterion-6
family): all seven compares green (corpus 5000/5000; dsn 1,332/0;
ses 20; ses-snap 5; index 33; undo 33; drc 17), events 3,520 exit 0,
determinism ×2 canaries UNROTATED at the T3 values, census
1444/0/17.

The CI flip: **NO-GO on both preconditions** at the T8 close — (a)
8/11 ≠ 11/11; (b) Σ wall 2286.1 s ≈ 38.1 min > the compare step's
30-min budget (`timeout-minutes: 30`, at `rust-check.yml:139` in
this tree; the M4 note's `:125` cites above are T13-era numbering —
T9's comment refresh moved the compare run line to :138/:139).
`--report-only` and both `ci_tripwire.rs` pins stay INTACT: the
second consecutive honest hold, with the composition changed — M4's
wall was the maze search (mid-routing kills); M5's is the bm01
optimizer stage under the 1800 s tier plus the two completion
residuals.

Deviations (the three reds, each with face / root cause / owner /
buglog):

| Red | Face (T8) | Root cause | Owner | Buglog |
| --- | --- | --- | --- | --- |
| bm01 | wall-killed 1800.1 s inside optimizer P2; routing COMPLETED 0/0/1000.00 at P8 in 275.69 s (fanout 4.43 s + routing P1–P9 271.26 s); optimizer P1 866.57 s REJECTED (`OPTIMIZER_SCORE_NOT_IMPROVED`, score already 0.00) | the T1b-parity-corrected optimizer ENTERS on a completed, non-improvable 0.00-score incumbent (Java's preflight-guard answer needs re-reading — the first M6 step) and burns the 1800 s tier bound sized for the pre-slice router; the maze-search wall it replaced is GONE | M6 | 196 (NEW: optimizer-entry + tier bound); 175 face updated |
| bm06 | 3/8/958.02, 18 passes — incomplete 3 > Java 2; score/violations green | the buglog-181 bistable victim-choice/attempt-order family (net 12), post-189 alignment | M6 | 181 |
| bm11 | 4/0/966.67, 18 passes — incomplete 4 > Java 3; no longer wall-killed (600.2 → 197.7 s) | same 181 family, completion-only | M6 | 181 |

Carry-forwards (every line traces to a dossier/buglog entry): M6
flip conditions — the bm01 optimizer-entry question + the 1800 s
tier bound (buglog 196; T8 concern (1) verbatim is its seed), the
buglog-181 completion levers (bm06 net-12 / bm11; closing the fork
needs a T12/T17c-style attempt-stream re-derivation inside the
fanout stage), and the threads-gate CI wiring (re-deferred — T8
closeout decision (2)); the T3 spec-review-r2 banked N3 — the
hole-override deferral note's conflation — rides the future
hole-override port task, which must cite Java's actual board-state
mutation at `HeadlessBoardManager.java:350-468` (`logs/M5-T3/report-t3.md`
:110); the M4-T9 F3 through-hole-pin fixture stays open under
return-with-first-consumer — T8 §(f) swept the whole M5 test diff
and found zero drill-page-expansion worlds, so first consumer
remains the first task landing one. Two residues named by M5's dossiers, recorded here for durability: the T7 quality-r2 banked N3 — the duplicated inert `unwrap_or(-1)` fallback at `pass_runner.rs:623`/`:864` — folds into M6's first pass_runner touch; the buglog sweep (196 NEW, 175/181 tags) lands via the chore(wolf) commit of this adjudication.

M6 exit note (T11, 2026-09-28; close-out docs commit as sole child of
`337e78b51` — the measurement record predates this note: the flip
adjudication is T3's (measured at `62c417a38`), the strictly>/
criterion faces are T10's (measured at `7b1b44c36`); the engine tree
came out of the close-out identical, census 1497/0/17):
**VERDICT: M6_EXIT_WITH_DEVIATIONS**, adjudicated 2026-09-28 at tree `a6d6ac38c` (fresh-eyes adjudicator): criterion 1 (Tier B/C completion strictly > Java) NOT MET on the measured faces — 2/12 strictly after the sanctioned tuning, 4/12 exact parity, 6/12 red, ALL-ON measured net-harmful, best-known regime default + push_shove-only — with every red owned by M7 via the deviations table; criteria 2 (Issue093-class violations = 0 on the real counter, both faces), 3 (island detection), and 5 (zero regressions at default, the 13/13 gate lap) MET; criterion 4 DISCHARGED AS RECORDED (196 + 181 closed, 197 partial with its successor lever unchartered, the flip NO-GO held — third consecutive — on the intact 1510.7 s reopen bound, threads-gate re-deferred with cause); the intelligence family landed complete and default-off with the byte-invariance contract held.

M6's arc, honestly stated: the milestone chartered three routing-intelligence pillars (plane-aware routing, congestion-aware global planning, push-and-shove) plus the M5 residuals (buglog 196, buglog 181, the CI flip). Two plan premises were falsified by the oracle and the record says so (AMENDMENT 1/2): (1) Java does NOT bypass its optimizer on bm01 — its own baseline carries `optimizer_seconds: 912.70` with `optimizer_score: 0.0`, so the preflight-guard port (T1, `e0ccffb74`) became a verification that Rust's guard chain was already Java-exact (M4-T10) plus the exact-edge pins; (2) the derived "17 optimizer passes / 14–16× per-pass gap" divided by the ROUTER's pass count — Java's optimizer runs ~2 passes on bm01, the same order as Rust's serial pass, and the dominant residual is Java's default candidate parallelism (`DefaultSettings.java:182`, `maxThreads = cores−1` vs Rust's mandated 1-thread face; buglog 197). T1b's wall slice (`UndoableObjects::compact`, `f842c3ea2`, −16.6% allocs / −18.3% bytes on the optimizer window) honestly missed its bm01 target; T2's fanout re-derivation OVERTURNED the M5-era "bistable tuning" classification — the fork was a derivable Java rule the port violated (corner-count off-by-one; 181 CLOSED `c63fc9610`), moving bm06 to 1/8/985.24 (beats Java's 2/8/971.63) and bm11 to Java's exact 3/0/975.00. T3 landed the first Tier B/C batteries and adjudicated the flip NO-GO (honest hold, third consecutive); T4 recorded the hold (`caa3115fa`); T5's plane audit found every Java plane face already ported Java-exactly (zero divergences; router-introduced violations 0 on both engines); T6 landed floating-island detection (advisory, beyond-Java); T7–T9 landed the default-off intelligence family (`router.congestion_global` + `.pattern` + `.pathfinder`, `router.plane_island_clamp`, `router.push_shove`); T10 measured the AFTER-faces and found the ALL-ON regime net-harmful (below).

Criterion verdicts (evidence: `logs/M6-T3/report-t3.md` the BEFORE-faces and flip arithmetic; `logs/M6-T10/report-t10.md` the AFTER-faces, tuning records, and gate lap):

- **Criterion 1 (Tier B/C completion strictly > Java) NOT MET on the measured faces.** The T10 criterion-1 table, verbatim: 2/12 strictly-better after the sanctioned tuning (1-Wire-Wing 0<1 via the empty flag subset; bm04 0<2 via push_shove + `max_passes=6`), 4/12 exact parity (complex_hierarchy after tuning, multichannel_mixer, multichannel_mixer-unrouted, StickHub after tuning), 6/12 red. The ALL-ON face is measured NET-HARMFUL vs default (B 7-vs-5 red, C 3-vs-2; 1-Wire win→loss, complex parity→+3, CM5 complete→kill). Best-known regime: **default + push_shove-only** — the two strictly wins plus StickHub's standing 900 s kill face flipped to completion @321.5 s at exact Java parity (4=4, 1=1, score 976.85=976.85). Both faces recorded; the ALL-ON verdict and the best-known regime are both measurements, not policy.
- **Criterion 2 (Issue093-class violations = 0) MET.** Real counter only — the manifest's `router_introduced_count` derives from `all_clearance_violation_depths` (the `getAllClearanceViolations` full-board walk; the outline-only `BoardStatistics` count is never the gate). router-introduced = 0 on EVERY completing pour fixture on BOTH faces (T3 census 10 fixtures; T10 both arms); post-route totals equal the parse-time input-board counts exactly (16/1/285/29), which equal Java's committed counts. Two honest kill-absences (StickHub both faces; CM5's ON face) recorded as kills — StickHub completed on its tuning face at 4/1/ri 0. bm10's single router-introduced violation (ri=1) is NOT a criterion-2 breach (bm10 carries no pour) — its own deviations row below.
- **Criterion 3 (island detection works) MET.** T6 landed the detector (`epic-board/src/islands.rs`: 4-connected metal regions over a 1-unit lattice, MIN_BRIDGE_WIDTH boundary mutation-verified both directions, advisory `pour_islands` manifest face, pour-free manifests byte-unchanged); T8 landed the region-level clamp wiring (`pour_region_seeded_by` behind `router.plane_island_clamp`); T10 re-confirmed the advisory face at HEAD with ZERO divergence (all 10 pour-fixture counts match T6 exactly).
- **Criterion 4 (inherited residuals) DISCHARGED AS RECORDED.** 196 CLOSED `e0ccffb74` (the port became a verification; premise falsified per AMENDMENT 1); 181 CLOSED `c63fc9610` (+ review round `ea5bdbe4f`; BRANCH A — a violated Java rule, not tuning; verified against `logs/M6-T2/report-t2.md`); 197 PARTIAL `f842c3ea2` (the per-pass semantics verdict is PARITY; the wall target honestly missed; the successor lever — deterministic candidate parallelism — is recorded NOT attempted and unchartered). The flip: **NO-GO held** at T3, record landed `caa3115fa` — both preconditions fail (verdict face 10 green / 1 red; Σ Tier A 2089.5 s > the compare step's 30-min budget); the threads-gate CI wiring re-deferred WITH CAUSE (it lands with the flip commit; the cause is the failed preconditions).
- **Criterion 5 (zero regressions at default) MET.** T10's full gate lap (13/13, at the unchanged engine tree): all seven compares green (corpus 5000/5000; dsn 1,332/0; ses 20; ses-snap 5; index 33; undo 33; drc 17), events 3,520 golden rows exit 0, determinism canaries IMMOVABLE/UNROTATED (ses `dc271114…` / manifest `cf607714…`; settings-ON determinism byte-stable at its own digests), census 1497/0/17 (additions only across the milestone), Tier A default battery at its held face (10 green / 1 red — the standing bm01 kill; completion-only reading 11/11). The close-out re-ran the cheap set only (fmt / clippy `--all-targets` / bare census); T10's lap at the same tree is the full evidence of record.

Wall claims — the FIXTURE-WALL face (the convention, named once: the battery-total Σ covers the per-fixture subprocess walls PLUS the in-process detail-localizer re-routes the harness runs on red non-killed fixtures — `router_compare.rs`; the per-fixture `wall` fields time only the subprocess — so cross-milestone wall claims quote the fixture-wall face; battery-total figures are detail-inclusive): T3-B default **2632.5 s = 3.10×** Σ-Java 850.4 s; B-ON **3041.0 s = 3.58×**; T3-C default **3387.5 s = 3.36×** Σ-Java 1009.3 s; C-ON **5400.3 s = 5.35×**; Tier A and C-ON are identical under both faces (A: no detail runs — only red was the bm01 kill; C-ON: all three killed). Tier A default **2075.7 s = 1.84×** Σ-Java 1130.39 s (the T10 run; the T3 run measured Σ 2089.5 s — the figure criterion 4 quotes — the Δ box-load noise on the unchanged engine, T10 report :87/:95). The 0.5× Σ goal stays far on every tier under either face, and ALL-ON worsens both B and C. The flip's reopen bound, EXPLICIT form (bound = tier cap 1800.0 s − the ten-wall strip): canonical bound 1800.0 − 289.3 = **1510.7 s** (the T3-era strip); at T10 the measured bm01 wall was **1800.2 s** (kill again), the this-run strip **275.5 s**, this-run bound **1524.5 s** — 1800.2 ≫ 1510.7, the flip does NOT reopen, the bound does not rise.

Deviations (every red still standing, face / root cause / owner / buglog):

| Red | Face (T10 unless noted) | Root cause | Owner | Buglog |
| --- | --- | --- | --- | --- |
| bm01 (Tier A) | optimizer wall: routing COMPLETES 0/0/1000.00 (pass 15), optimizer P1 532.0 s REJECTED, P2 to the 1800.2 s tier kill | per-pass optimizer cost, same order as Java's serial pass; the named lever is deterministic candidate parallelism (Java `maxThreads = cores−1`), RECORDED NOT ATTEMPTED, unchartered | M7+ (needs its own charter) | 197 |
| B/C completion reds (6/12) | bm05 27>18; bm10 3>1; interf_u 3>0; 1Bitsy, CM5, LimeSDR tier kills; mm/mm-u exact-parity floors | engine-capability/wall faces at today's maturity; the congestion/pathfinder flags measured net-harmful, push_shove-only the one lever with wins; no lever in the T8/T9 inheritance moved the floors (mm 160, mm-u 128 byte-stable across faces) | M7 | — (the T3/T10 tables are the record; 181 closed) |
| bm10 router-INTRODUCED violation | ri=1 on the ON face AND the push_shove-only face — flag-INDEPENDENT; first measured instance of the route path introducing a violation on this population (bm10 is not a pour fixture, so criterion 2 is untouched) | cause unlocalized (route-insertion face under shove); never fires at default (bm10 dies in the optimizer there) | M7 | 205 (NEW) |
| 1Bitsy default-face drift flag | T10 tuning kill @300 s vs T3's 235.3 s completion on a census-pinned tree (load caveat; an honest kill either way) | unmeasured; re-measure only if cheap — the fixture is red under every flag face regardless | M7 (measurement) | — |
| StickHub / CM5 pour-face absences | criterion-2 counts honestly absent where the runs were killed (StickHub both faces; CM5 ON); StickHub's tuning-face completion carries 4/1/ri 0 | measurement gaps, NOT divergences; no new work chartered | — | — |

Carry-forwards (every line traces to a dossier/buglog entry): the harness finding that `router compare` accepts no settings-flag face (`--extra-router-arg` exists only on determinism/threads-invariance; T10 rode the documented `EPIC_CLI` pass-through wrapper) — adding `--extra-router-arg` to compare is the clean harness face if an ON battery is ever wanted again, owner M7+ harness (T10 deviation 1); buglog 197's successor lever (the optimizer candidate-loop executor, the M5-T7 deterministic-partition pattern — the flip's reopen bound rides it); the unlanded T9 NIT banks (NIT-Q1 derivable `push_shove_waivers` counter; NIT-Q2 front-pop-order pin/doc; NIT-Q3 "1 waives" unreachability note; NIT-Q4 Q6 pin dust) and T8's optional Q2/Q3/Q8 — DECLINED as T11 riders (a docs-only task with no natural engine touch) and BANKED for the first engine touch; the fanout-full-flow golden gap (zero byte-invariance coverage for the 181 fix until one exists — SEAM M6 hold posture) and the banked harness NITs (Σ-decomposition battery print, shared no-extras sweep helper, bail! idiom, multi-concern walk pin) wait on the first harness touch; bug-186's lexer-parity scope stays M1b-reopen; StickHub post-route pour measurement and CM5's ON-face pour count are recorded measurement gaps, no new work.

M7 exit note (T8, 2026-09-28; close-out docs commit as sole child of
`0741bd728` — the measurement record predates this note: the tuning
measurement is T7's, measured at tree `6165ac7aa` (`logs/M7-T7/report-t7.md`
+ 33 evidence files; two post-review rounds, coordinator-verified at source
each round); the engine tree came out of the close-out identical, census
1547/0/17):
**VERDICT: M7_EXIT_WITH_DEVIATIONS**, adjudicated 2026-09-28 at tree
`8dcd776f3` (fresh-eyes adjudicator): all five criteria MET / DISCHARGED
AS RECORDED — criterion 1 (constraints honored: delivery Java-exact, the
never-shorten gate at exactly the two tightener acceptance faces, meander
to min, match-to-target, the honest max report) MET on the crafted worlds
+ the six committed fixtures; criterion 2 (tuning DRC-clean) MET with
ri = 0 across the tuning population under the REAL counter, bm10's ri=1
honestly owned parity-confirmed-noted (205 CLOSED `b7b174859`/
`349a192df`) within the criterion's letter; criterion 3 (zero
regressions) MET — Tier A 11/11 held row-for-row, census exactly
1547/0/17, canaries unrotated, constraint-free invariance structural and
pinned; criterion 4 DISCHARGED AS RECORDED (205/210/201/211/212 CLOSED
with pins, 197 OPEN owner-M8+ with its lever recorded not attempted, the
B/C re-faces reduced to bm10 + 1Bitsy by the zero-declaration census);
criterion 5 MET — the T7 14/14 gate lap at the unchanged tree plus the
close-out cheap set, the additions ledger matching the span exactly, the
two sanctioned t7_ripup re-captures the only baseline modifications,
nothing pushed — with every still-standing red (bm01/197, the B/C
capability floors, bm10 completion, 1Bitsy) owned by M8 via the
deviations table.

M7's arc, honestly stated: the milestone chartered the design's tuning
pillar (§4 :73 — length-constraint honoring, clearance-aware meander
insertion, differential pairs) plus the M6-owned residuals (bm10's ri=1,
the B/C completion reds, the first-touch harness/engine banks). Two plan
premises were falsified and the record says so (AMENDMENTS 1 and 7): (1)
T1's audit falsified the dead-reader premise — the length-constraint
read/deliver chain was ALREADY Java-exact at the pre-T1 tree (the three
`ses_board.rs` hard-zero sites are `rules/NetClass.java:36-37`
ctor-parity sites, not divergences; the Rust DSN write face does not
exist at all), so the parity port became a verification plus four
boundary pin worlds (`6f6334e51`); (2) AMENDMENT 6 §3's erratum was
itself label-swapped — the committed goldens are right, the prose wrong
twice over (the Finding-1 correction below). T2 landed the constraint
model + 205's localization (`0626ff8a2`); T3 the split-world
parity-confirmed-noted disposition + the min-length honoring gate
(`b7b174859` + `349a192df`); T4 the clearance-aware meander engine +
the harness banks (`514a4d5e0` + `c9fb800e4` + `d87da94b7` +
`c1478a31d`); T5 the match-to-target face + the buglog-210 drain fix
with its sanctioned two-golden re-capture (`d6c7bd859` + `758356366` +
`a7b00639b` + `595049285`); T6 the differential-pair face
(`36f4570c2` + `bdfd39216` + `7e772fa2c`); T7 the measurement moment
(no commits — a battery/DRC-clean/disposition task at the unchanged
tree).

Criterion verdicts (evidence: `logs/M7-T7/report-t7.md` — §1 the
tuning-population table, §2 the Tier A face, §3 the M6 dispositions +
the deviations refresh, §4 the 14/14 gate lap; per-task dossiers at the
pinned commits above):

- **Criterion 1 (DSN-declared constraints honored) MET.** The full
  chain: parse/deliver verified Java-exact (T1); the resolved-constraint
  query surface `BoardRules::net_class_length_bounds`
  (`crates/epic-board/src/rules_surf.rs:783`, 0.0 = no constraint,
  flat class table — the class-inheritance audit found no hierarchy) +
  `has_length_constraints` (`:802`); the NEVER-shorten honoring gate
  `min_length_gate_allows` (`crates/epic-board/src/trace_tightener/mod.rs:1170`),
  consulted at exactly the two tightener change-acceptance faces
  (pull-tight acceptance `:1143`; pin-tail rebuild
  `crates/epic-board/src/trace_tightener/pin_tail.rs:433`) —
  whole-candidate rejection, landing-at-min allowed, inert when no min
  is declared; meander insertion to reach min (T4, `pipeline/tuning.rs`:
  fixed amplitude ladder `:150`, per-site dent cap `:156`, clearance
  probe fails-safe, honest stop); match-to-target with per-class goals
  (T5: both-declared goal = min exactly; min-only target = the longest
  routed member, tolerance `MATCH_TOLERANCE_DBU` `tuning.rs:174`);
  max-length = the honest report-only face (`length_violation` =
  Java `calcLengthViolation` verbatim, `rules_surf.rs:825`; over-max is
  never truncated — meander_blocked's −40 000 `length_report` row is
  the exercised face). Demonstrated on crafted worlds with
  mutation-verified pins (T1's four boundary worlds + 9-run mutation
  record; T3's split world; T4's W1–W8; T5's M1–M7; T6's P1–P6) and
  the six committed tuning fixtures (T7 §1: every deficit filled or
  honestly reported).
- **Criterion 2 (tuning DRC-clean on the fixture set) MET.** ri = 0 on
  all six tuning fixtures at every `router_introduced_count` block (the
  REAL counter, `crates/epic-router/src/pipeline/board_statistics.rs:596-598`
  ← `all_clearance_violation_depths`,
  `crates/epic-drc/src/clearance.rs:557` — never the outline-only
  face); min_stair's total of 1 is the fixture's own input pre-existing
  violation (AMENDMENT-3-known, untouched); every landed meander is
  DRC-clean; the pair fixtures' default-flag faces are INERT (AMENDMENT
  6 §4) with both committed ON goldens re-verified green; the P5
  equality boundary green in-process. The ONE router-introduced
  violation in the entire measurement remains bm10's pre-dispositioned
  parity face (below).
- **Criterion 3 (zero regressions, default + constraint-free) MET.**
  Tier A 11/11 held row-for-row (10 green / 1 red completion-only; Σ
  2133.1 summary vs 2133.0 two independent re-sums — print rounding;
  zero detail runs, so both Σ faces coincide); every PASS row
  `introduced 0`; the constraint-free-invariance rule held all
  milestone (the committed corpus declares zero `(length`; the
  mixed-world pin `p3_undeclared_net_geometry_identical_on_vs_off`,
  `crates/epic-router/src/pipeline/pairs.rs:815`; canaries
  `dc271114…`/`cf607714…` UNROTATED throughout); the gate lap 14/14.
- **Criterion 4 (inherited dispositions) DISCHARGED AS RECORDED.**
  205 CLOSED — parity-confirmed-noted (T3's crafted split world +
  chain-complete source read: Java's own tightener re-lands split
  pieces unchecked; no fix without anti-parity). 210 CLOSED — outcome
  (i), a real Rust divergence: the SES input-wiring duplication (the
  parse board retained `(wiring` items next to the live projection);
  the Java-exact drain fix landed (`a7b00639b`) with the
  coordinator-sanctioned two-golden re-capture (the goldens pinned the
  defect's own bytes; canaries proven unaffected). 201 CLOSED — the
  `--out` path-joining asymmetry fixed at T4's harness touch (the
  `--out`/`--golden` faces now join the repo root like
  `--dsn`/`--work-root`). The M6 NIT banks: NIT-Q2/Q3 landed at T2
  (first engine touch); NIT-Q1 dropped with cause; NIT-Q4 + T8's
  optional banks declined by name; T4's harness banks landed
  (`--extra-router-arg` on compare, the fanout-full-flow golden, the
  Σ-decomposition print, `record_extras`, the `bail!` idiom) or
  declined with cause (the `git_sha` embed — anti-instrument). 197
  stays OPEN with its successor lever (deterministic candidate
  parallelism) recorded NOT attempted — a wall lever, explicitly out of
  the M7 scope, owner M8+ (the buglog entry carries the sweep note).
  The B/C reds re-faced: the constraint census found zero `(length`
  declarations among all twelve fixtures, so the tuning faces could not
  matter — the re-faces reduce to bm10 (ri=1 REPRODUCED
  flag-independently, 3/1/979.87, 443.8 s; the run total 1343.8 = 443.8
  + 900.0 detail-burn, reconciled exactly) and 1Bitsy (re-measured
  RED completion-only, 2>1, `introduced 0`, 220.9 s, NOT killed — the
  kill face NOT reproduced; 520.9 = 220.9 + 300.0, reconciled); the
  held M6 faces stand as the record for the rest.
- **Criterion 5 (gates green; census additions only; committed
  artifacts untouched) MET.** Census EXACTLY 1547/0/17 at the
  unchanged engine tree; the M7 ADDITIONS LEDGER, enumerated: the
  crafted-world pin sets (T1's four `network.rs` boundary worlds; T3's
  split world + honoring pins + the 6/6 mutation log; T4's W1–W8 +
  F1–F3; T5's M1–M7 group pins + the 4/4 mutation log; T6's P1–P6 pair
  pins), the SIX new committed tuning fixtures
  (`rust/harness/fixtures/tuning/`: `min_stair_tuning`,
  `meander_room_tuning`, `meander_blocked_tuning`,
  `meander_multi_tuning`, `pair_coupled_tuning`, `pair_split_tuning`),
  and the TWO new pair goldens (`pair_coupled.global-golden.json`,
  `pair_split.global-golden.json`) — PLUS the two SANCTIONED
  t7_ripup golden re-captures (buglog 210; the milestone's ONLY
  committed-baseline modifications, justified per the capture rule);
  seven compares green (corpus 5000/5000; dsn 1,332/0; ses 20;
  ses-snap 5; index 33; undo 33; drc 17), events 3,520, canaries
  unrotated, det-ON digest pair byte-identical.

Deviations (the T7 §3 refresh, verbatim — the M8 adjudication input;
every still-standing red):

| Red | Face (T7, tree `6165ac7aa`, unless noted) | Root cause | Owner | Buglog |
|---|---|---|---|---|
| bm01 (Tier A) | tier-wall kill 1800.1 s, no manifest; unchanged from M6 | per-pass optimizer cost on a completed 0.00-score incumbent; named lever = deterministic candidate parallelism, recorded NOT attempted | M8+ (own charter) | 197 |
| bm10 ri=1 | reproduced at T7 at the push_shove-only face (3/1/979.87, introduced 1; M6's ON face not re-run — flag-independence already established) | Java's own tightener design: `split_traces` re-lands pieces unchecked (parity-confirmed-noted) | DISPOSED (parity-confirmed-noted; no fix without anti-parity) | 205 (CLOSED) |
| B/C completion reds — bm05 27>18, interf_u 3>0, mm/mm-u parity floors, CM5 + LimeSDR tier kills | NOT re-faced at T7 (zero `(length` declarations — tuning could not matter; engine unchanged; the M6-T3/T10 faces remain the record) | engine-capability/wall faces at today's maturity | M8 adjudication | — (the T3/T10 tables are the record) |
| bm10 completion red | 3>1 re-confirmed at T7 | same engine-capability family | M8 adjudication | — |
| 1Bitsy | re-faced at T7: RED completion-only 2>1 @220.9 s (kill face NOT reproduced; red either way) | completion/bistability family; box-load sensitive pass tail | M8 adjudication | — |
| StickHub / CM5 pour-face absences | unchanged (measurement gaps, not divergences; no pour battery ran at T7) | — | — | — |

Carry-forwards (every line traces to a dossier/buglog entry): buglog
197's successor lever (the optimizer candidate-loop executor, the
M5-T7 deterministic-partition pattern — the flip's reopen bound rides
it) recorded NOT attempted, unchartered, owner M8+; the banked e2e
out-of-window decoupled witness (AMENDMENT 6 §5e — not taken at T7; the
in-process P2 owns the face; an e2e witness is an enhancement);
the tightener-gate-rejection fixture face (a pre-routed slack +
min-above-folded-length world) declined by name twice (T4/T5), stays
banked; the MSDTW median-trace face declined-on-measurement (T5 — all
four tuning fixtures route tuned nets on separate corridors; reopen
only for a shared-corridor charter, the natural candidate being a pair
corridor extension) and the Dubins A* declined on measurement +
realizability (T6 — the 45°/90° polyline engine cannot land G1 arc
geometry); both declines live in the module docs with their re-charter
triggers; the harness usage note that `--extra-router-arg` requires the
`=`-form (the space-separated form is eaten by clap, exit=2 — T7
Finding 3); bug-186's lexer-parity scope stays M1b-reopen; the T6/T5
pair-delta design faces (goal=min, `PAIR_DELTA_DBU` never the class
window) stand as landed, documented in the module docs.

The Finding-1 correction (AMENDMENT 7 §4 — the erratum-of-the-erratum):
AMENDMENT 6 §3's erratum is itself label-swapped. The committed
**pair_SPLIT** golden's ON manifest carries `coupled_length` **381 482.0**
(delta 10 270.24); **pair_COUPLIED** carries **0.0** (delta 15 147.19) —
the advisory honestly reporting that its members share no
within-window parallel span under `COUPLING_WINDOW_DBU`
(`crates/epic-router/src/pipeline/pairs.rs:93`) while still matching by
meander. Both `matched:true`, anchor net 2; the goldens pin the correct
`dsn_sha256`s — committed artifacts right, prose wrong. The corrected
reproduction (T7 report FIX ROUND 1, with `--work-root`; run from
`rust/`):

```
cargo run -q -p epic-harness -- global-golden verify \
  --dsn rust/harness/fixtures/tuning/pair_<X>_tuning.dsn \
  --golden rust/harness/baselines/global/pair_<X>.global-golden.json \
  --face pair_<X> \
  --work-root /home/tyler/Downloads/EpicRouter/rust/harness/runs/t7-pair_<X>-verify
```

for X ∈ {coupled, split}, then read
`harness/runs/t7-pair_<X>-verify/verify/manifest.json`.

The mixer fact (battery reasoning, carried from AMENDMENTS 5 §3 / 6 §5d
and re-counted exact at T7): Tier B's `multichannel_mixer-unrouted.dsn`
carries GND pre-routes — 125 `(wire` scopes at the canonical fixture —
and its post-route session bytes changed under the M7-T5 drain fix
(buglog 210). Safety rests on the positive attribution: only the two
sanctioned goldens moved and no committed artifact pins the mixer's
session bytes. No Tier B/C battery ran at T7, so no mixer face was
re-measured; the fact rides this note for every future battery
reading.

The engine tree is frozen at census 1547/0/17 across the close-out; the
verdict slot above stays OPEN for the milestone adjudication (the
M4-T17d/M5/M6 pattern; the deviations table above is its input).

ERRATUM (adjudication): the closing line above predates the adjudication —
the slot is now FILLED (`M7_EXIT_WITH_DEVIATIONS`, above). Criterion 5's
"ses-snap 5" is a per-task gate face (T3/T4/T5/T6, all green — the
artifact-verified list), not re-run at T7's 14/14 lap; the lap-covered
compares are the other six plus events (listed separately).

ERRATUM (adjudication, extends the Finding-1 correction): the `pairs.rs`
module doc's Dubins note (:48-50) carries the same swapped `coupled_length`
attribution Finding 1 corrected in the T6 report and AMENDMENT 6 §3 —
381 482.0 / delta 10 270 belong to the SPLIT fixture; the source comment
fixes at the first `pairs.rs` touch (M8). The measurement verdicts are
unaffected.

M8 exit note (T9, 2026-09-29; close-out docs commit `23a8dba79` as
sole child of `720c72d52` (the AM8 banking commit, itself
`040d5dade`'s sole child) — the measurement record predates this note:
the AFTER
battery, the per-pass ablations, the inherited re-faces, and the CI
flip are T8's, measured at tree `9fe3a0751`
(`logs/M8-T8/report-t8.md` + evidence; spec r1 fix `186c9eafd`,
quality r1 fix `040d5dade`); the engine tree came out of the close-out
identical, census 1629/0/17):
**VERDICT: M8_EXIT_WITH_DEVIATIONS**, adjudicated 2026-09-29 at tree `277ee7970` (fresh-eyes adjudicator): criterion 1 (aesthetics metrics improve monotonically vs PCBench ground truth) **NOT MET on the committed four-metric instrument** — under the governing AM2 predicates (the milestone's reviewed legislative record; the strict GT-gap letter would have scored this record no better — only bend improves under it), mean_length_excess 0.1807→0.4743 and parallelism_ratio 0.93875→0.9325 are direction-gate regression findings, via_density 1.5891→1.3905 is a third finding (AM2's band constant 2.4752 is hereby corrected to its declared gap-of-means base 1.5752 — |AFTER−GT| 1.7738 > 1.5752: letter FAILURE, not letter PASS; the monotone-up violation stands under either constant), and bend 17.8165→15.3972 is the sole pass, artifact-dominated (−2.3794 of −2.4193 is the teardrop denominator effect); all four metric moves own to the teardrop pass's 976 landed tapers on 15 boards (teardrops-alone ≈ ALL-ON) while the designed primary bend lever (flow) is an EXACT no-op on all 20 sample boards; the teardrop tension (consumer-honest, DRC-clean geometry the four-metric instrument scores as regression) is dispositioned DEFAULT-OFF-WITH-TENSION-DOCUMENTED — no retroactive instrument change (the re-baseline the criterion forbids); the instrument-vs-consumer decision (teardrop-aware term, explicit exclusion, or the letter kept) is an M9 carry-forward that must precede any metric-facing pass; criterion 2 (no completion/DRC regression) MET clean (the 20/20 grid with ri = 0 everywhere on the REAL counter, Tier A held face 10+1 exact Σ 2096.1, eight compares, events 3520, canaries UNROTATED + ON-face byte-stable ×2, census 1629/0/17, the LMS6002 kill pinned permanent); criterion 3 MET (the 197 charter decision landed and measured — 1340.78 s < 1514.3/1510.7, the reopen OPENED; the T8 refresh honest: bm05 worse 29>18, bm10 face-changed-worse, interf_u improved 1>0, CM5's kill green, gloss-ON a row-for-row NO-OP lever — discharged); criterion 4 DISCHARGED (the README flips, the pairs.rs :48-50 erratum fix at T3, the chore(wolf) bank at `277ee7970`); criterion 5 MET (census additions only 1547→1629, committed artifacts untouched except the sanctioned NEW captures, nothing pushed); the CI flip SUSTAINED at `ba5e587bb` (11/0 at the exact gate face, Σ 286.5 s, 5.3–6.3× headroom) — with every still-standing red (the B/C capability floors, bm10, 1Bitsy, bm04, bm01's adoption face + P2 skew, the flow engagement question, the teardrop instrument question, the F3 armed trigger, the threads-gate CI wiring, the pour measurement gaps) owned by M9 via the deviations table.

M8's arc, honestly stated (four beats, from AM8's T9 banks): (a)
**instrument-before-improvement held** — T1 built the four-metric
measurer, the committed 21-board PCBench sample, and the 24 reference
goldens before any pass existed, T2 measured the BEFORE face before
any pass ran, and the falsification reads kept the plan's task order
(the T5-first re-order REJECTED on measurement, AM2; the
`ground_truth.json` demotion to context rows, AM1); (b) **the gloss
family's real-corpus outcome is the milestone's honest center** — the
designed primary bend lever (the flow pass) never engaged at all
(EXACT no-op on all 20 sample boards, zero movers on default-routed
geometry), bus and via-place moved noise-level deltas, and the one
pass that moved real geometry — teardrops, 976 landed tapers on 15
boards, geometry the consumer wants — is scored by the four-metric
instrument as REGRESSION, because its taper copper adds routed length
into every length-bearing metric; (c) **the 197 charter decision
landed deterministically and opened the flip on measurement** — the
deterministic candidate parallelism (T7, `e92b57cc9`) pulled bm01
under both bounds (1340.78 s vs 1514.3/1510.7), four holds preceded
the fifth moment, and the flip LANDED at `ba5e587bb` (11 green / 0 red
at the exact post-flip CI face, Σ 286.5 s); (d) **the re-faces: no
gloss-ON rescue, two rows worse, one improved, one kill green** — the
push_shove precedent does NOT repeat (gloss-ON is row-for-row NO-OP on
every B/C fixture, both tiers), bm05 and bm10 moved worse, interf_u
improved, CM5's kill did not recur.

Criterion verdicts (evidence: `logs/M8-T8/report-t8.md` — §1 the
judgment + attribution tables, §2 the no-regression face, §3 the
re-faces, §5 the flip record; `logs/M8-T2/report-t2.md` the BEFORE
table; AMENDMENTS 1–8 in the gloss plan):

- **Criterion 1 (aesthetics metrics improve monotonically vs PCBench
  ground truth) — PRESENTED HONESTLY AS MET-ON-ONE-METRIC /
  FINDINGS-ON-TWO; the verdict formulation is THE adjudication
  question.** The judgment table (T8 §1; all 24 cells
  coordinator-re-derived exact from the raw sidecars), n = 20 (the
  LMS6002 kill row pinned, denominators stable); the verdict cells'
  thresholds are the AM2 predicates — the direction gates are
  BEFORE ± ε with ε = 0.0005 (gloss plan :146-149), not GT-derived
  bounds:

  | metric | BEFORE (T2) | AFTER (ALL-ON) | GT mean | verdict |
  |---|---|---|---|---|
  | mean_length_excess | 0.1807 | 0.4743 | 0.2093 | **REGRESSION-FINDING** (> 0.1812) |
  | via_density | 1.5891 | 1.3905 | 3.1643 | letter PASS; **monotone-up VIOLATED (down)** |
  | bend_to_length_ratio | 17.8165 | 15.3972 | 15.1677 | PASS — **artifact-dominated** |
  | parallelism_ratio | 0.93875 | 0.9325 | 0.9054 | **REGRESSION-FINDING** (< 0.9383) |

  Attribution (the ablation table, T8 §1): BOTH regression findings +
  the via-density violation + the bend "win" own to the TEARDROPS
  pass's taper copper — 976 landed tapers on 15 boards ADD routed
  length, which enters the length-excess numerator and the
  via-density/bend-ratio denominators (teardrops-alone ≈ ALL-ON:
  len 0.4745, bend 15.4370). The genuine levers are tiny:
  via-place −0.048 bends; bus near-neutral; **flow-alone = EXACT
  no-op on all 20 boards** (zero movers on default-routed sample
  geometry). The measurement-contract tension, owned to the record:
  the AM6 position ("teardrops are unscored by the four metrics") is
  FALSIFIED in its metric-inert reading — they are unscored as CREDIT,
  but they still move all four metrics through the length faces; the
  instrument-vs-consumer question (geometry the consumer wants that
  the four-metric instrument scores as regression) is the
  adjudicator's input, not an engine bug. **No finding is
  re-baselined.** Whether this record reads MET-with-findings or
  NOT-MET is left open by design.
- **Criterion 2 (no completion/DRC regression) MET clean.** The
  no-regression grid 20/20 (incompletes byte-equal ON vs default;
  ri = 0 everywhere, the REAL counter); the Tier A default battery at
  its held face 10 green / 1 red EXACTLY (the bm01 chronic
  cap-scrape; Σ 2096.1 s DNR-18-reconciled); eight compares green by
  name; events 3520; determinism ×2 default (canaries
  `dc271114…`/`cf607714…` UNROTATED) + ×2 ALL-ON (ses `3ea73c82…`
  byte-stable); census 1629/0/17 EXACT ×3. The LMS6002-Pmod kill row
  is UNCHANGED (exit=124 recurs at ALL-ON) — the pinned PERMANENT
  row: a kill, not a regression.
- **Criterion 3 (the M7-exit deviations rows dispositioned) MET.**
  bm01/197: the charter decision made and measured (T7: wall
  1340.78 s < the 1514.3 this-run / 1510.7 canonical bounds — the
  reopen OPENED; the lever `optimizer.threads` landed opt-in, the
  flip judged at T8). The T8 refresh (§3), every row honest:
  bm05 **worse** (29 > 18, was 27 > 18); bm10's **FACE CHANGED
  worse** (completion 3 > 1 → tier-kill @900 s); interf_u
  **improved** (1 > 0, was 3 > 0, still red); the mm/mm-u parity
  floors held byte-exact (160 = 160; 128 = 128, cv 285 = 285);
  StickHub/LimeSDR kills held; 1Bitsy 2 > 1 and bm04 3 > 2 held
  exactly; **CM5's kill did NOT recur — PASS 6 ≤ 6 (green)**; and
  gloss-ON as a lever candidate is **row-for-row NO-OP on every B/C
  fixture, both tiers** (the push_shove precedent does not repeat).
  The StickHub/CM5 pour measurement gaps carry. F3's reopen trigger:
  DECLINED with evidence (diagonal-parallel groups EXIST — 7 groups
  on 3 boards — but no board carries bend/length headroom; the
  trigger stays armed for M9+ corpora). All still-standing reds
  re-owned M9+ (table below).
- **Criterion 4 (the M8-opening chores landed) DISCHARGED.** The
  README M7 status flip landed early (AM2, riding the T2 amendment);
  the `pairs.rs` :48-:50 module-doc swap fix landed at T3 (the
  adjudication erratum corrected at source; the committed module doc
  now attributes 381 482.0 / delta 10 270.24 to pair_SPLIT); **the
  chore(wolf) bank — the M8 buglog dispositions, including 197's
  flip-judgment tag now resolving at `ba5e587bb` — lands as the
  coordinator's separate chore(wolf) commit IMMEDIATELY AFTER this
  close-out commit** (the no-fold rule is gloss plan :22; the
  immediately-after timing is AM8's T9 banks + the T9 charter's own
  phrasing; it is not part of this note's commit).
- **Criterion 5 (gates green; census additions only; committed
  artifacts untouched; nothing pushed) MET.** Census 1547 →
  1629/0/17 across M8, additions only (the per-task ledger: 1562 at
  T1 → 1581/1584 at T3 → 1598/1599 at T4 → 1613 at T5 → 1622/1625 at
  T6 → 1629 at T7); committed artifacts untouched EXCEPT the
  sanctioned NEW captures — the aesthetics sample + 24 reference
  goldens + `select.py` (T1) and the ON-face goldens (T3 bus worlds,
  T4 flow worlds, T5 via worlds, T6 teardrop worlds), each justified
  in its landing commit; nothing pushed.

Deviations (the T8 §3 refresh, the M7 deviations-table shape — the
adjudication input; every still-standing red):

| Red | Face (T8, tree `9fe3a0751`) | Root cause | Owner | Buglog |
| --- | --- | --- | --- | --- |
| bm01 (Tier A) | full-flow tier-kill 1800 s in BOTH modes (gate mode does NOT preflight-skip its optimizer); with the lever ot=4: 1340.78 s (T7) — opt-in, NOT load-bearing for the router-only CI face | optimizer wall; the lever LANDED (deterministic candidate parallelism, `optimizer.threads`); the adoption face is open | M9+ (adoption face) | 197 (closed at implementation) |
| bm05 | RED 29 > 18 (was 27 > 18) | engine-capability (congestion) | M9+ | — |
| bm10 | tier-kill @900 s (was completion 3 > 1) — FACE CHANGED, worse | engine-capability + load-sensitive tail | M9+ | — |
| interf_u | RED 1 > 0 (was 3 > 0) — the row improves, still red | engine-capability | M9+ | — |
| mm / mm-u | parity floors HELD (160 = 160; 128 = 128, cv 285 = 285) | — | — | — |
| StickHub / LimeSDR | tier-kills held @900 s | engine-capability/wall | M9+ | — |
| 1Bitsy | RED 2 > 1 held exactly | completion/bistability | M9+ | — |
| bm04 | 3 > 2 held exactly | completion | M9+ | — |
| CM5 | kill did NOT recur — PASS 6 ≤ 6 | was kill; now green | — | — |
| gloss-ON lever | row-for-row NO-OP on every B/C fixture, both tiers | the push_shove precedent does NOT repeat | adjudication | — |
| StickHub / CM5 pour faces | not re-measured (no pour battery) | measurement gaps | — | — |

Carry-forwards (every line traces to a record): the via_density
full-closure routing-stage lever (M9+, OUTSIDE the gloss family — AM2
named it; the pass family's count-never-increases guard means the
gloss passes structurally cannot close the ≈half-of-GT via deficit);
the flow-pass engagement question (zero movers on default-routed
sample geometry — constant recalibration, a pre-flow normalizer
investigation, or an honest decline of the pass family; the designed
primary bend lever never engaged the corpus it was built for); the
teardrop metric-tension disposition (instrument change vs
default-off-with-tension-documented — the adjudication's input; the
pass itself is DRC-clean and consumer-honest, the four-metric
instrument scores its copper as regression); the P2 partition-skew
tuning follow-up (206.7 vs 710.8 s at N=4 under the fixed mod key,
AM7 — a performance face, not correctness); the F3 diagonal trigger
(ARMED, declined-with-evidence at T8 — 7 groups on 3 boards exist but
no headroom; reopen for M9+ corpora); the threads-gate CI wiring
(re-owned to the next workflow-touching act; the in-tree-only posture
disclosed in rust-check.yml and README); the O(N²·S²) gloss detector
prefilter — DECLINED on measurement (+8 s on the whole battery, T8
§8); the StickHub/CM5 pour measurement gaps (carried, no new work).

The engine tree is frozen at census 1629/0/17 across the close-out;
the verdict slot above stays OPEN for the milestone adjudication (the
M4-T17d/M5/M6/M7 pattern; the deviations table and the criterion-1
verdict formulation are its inputs).

ERRATUM (adjudication): the closing line above predates the
adjudication — the slot is now FILLED (`M8_EXIT_WITH_DEVIATIONS`,
above). Two further adjudication errata:

1. **C1 label replaced**: the criterion-1 presentation's
"MET-ON-ONE-METRIC / FINDINGS-ON-TWO" label (the T9 charter's own
phrasing) is REPLACED by the adjudicated formulation — C1 NOT MET on
the committed instrument; one metric passes (bend, artifact-
dominated), three are findings (mean_length_excess,
parallelism_ratio, via_density).
2. **The via_density band erratum (F-ADJ-1)**: AMENDMENT 2's
via_density band constant 2.4752 has no derivation from the committed
BEFORE record; the declared base (gap-of-means) gives 1.5752 (the
bend band 2.6488 matches its gap-of-means exactly, establishing the
intended derivation law). The T8 judgment table's via cell and
`build-table-t8.py`'s constant carry the same erratum — via_density
reads REGRESSION-FINDING, not "letter PASS". No measurement changes;
the denominators, sidecars, and all other 23 cells stand verified
exact.

LAW-GOVERNANCE NOTE (adjudication): AM2's amended criterion-1
predicates govern M8's exit (the strict original letter would have
scored this record better — 2 of 4 improving — evidencing
non-self-serving adoption); the corrected AM2 law stands for M9 until
formally amended.

M9 exit note (T8, 2026-09-30; close-out docs commit `9bc40ae90` as
sole child of `e5591969c` (the T7-cycle wolf-bank commit, itself
`8a024eed8`'s sole child — the AMENDMENT 7 record; `3b9e0d8fb` is the
T6-cycle wolf bank two steps down) — the measurement record predates this note:
the workflow battery, the render-perf face, the inherited re-faces,
and the gate lap are T7's, measured at tree `3b9e0d8fb`
(`logs/M9-T7/report-t7.md` + evidence); the engine tree came out of
the close-out identical, census 1682/0/19):
**VERDICT: M9_EXIT_WITH_DEVIATIONS**, adjudicated 2026-09-30 at tree `66f563cf5` (fresh-eyes adjudicator): criterion 1 (the full workflow without Java, on the session path) **MET WITH FINDINGS** — the T7 battery 17/17 byte-identical between `Session` load→route(default)→export and `epic-cli route` (11 Tier A + 3 Tier B incl. the PCBench 1Bitsy + 3 crafted; bm01/bm10 completed by natural-wall CLI re-runs and byte-proven twice, `5c34ef4c…`/`9063a828…`), all four `LoadError` arms pinned at T2, and the interactive demonstration SOUND on the ride-forward (the rust/ CODE tree byte-identical since `94e17f873`; the overlays-ON bm08 shell PNG and the routing-stage cancel smoke `final_state=CANCELLED` independently re-viewed) — with the FINDING that the demonstrated cancel is ROUTING-stage-only: the optimizer stage runs on a flagless `StopFace::default()` (`full.rs:341-342`, the documented T9 wiring artifact), so the GUI's mid-OPTIMIZATION cancel is structurally ineffective (pre-existing, NOT an M9 regression; buglog 224; carried M10 as a wiring change or a documented bound); criterion 2 (renders never mutate) MET structurally (the grep gate green at every task and re-run clean by the adjudicator; no mutation surface exists in epic-gui); criterion 3 (the §7 view set with headless goldens) MET (10 committed NEW goldens — the T4 render trio, the T5 six overlay extensions, the T3 event-kind golden — plus the unit pins, each family visible in the shell, the conditionals pinned present-iff); criterion 4 (zero regressions) MET (seven compares green; events 3,520; canaries `dc271114…`/`cf607714…` IMMOVABLE ×2; Tier A CI gate face 11/0, Σ 313.4 s; census additions only 1629→1682, measured 1682/0/19 thrice; the settings move byte-neutral at T1; committed artifacts untouched except the sanctioned NEW goldens; nothing pushed); criterion 5 (the M8 deviations table dispositioned) MET (the threads-gate CI wiring LANDED at T6 and verified in `rust-check.yml`; every other row re-faced at T7 row-for-row against the raw logs and carried with named M10+ ownership); with thirteen chartered deviations riding the 14-row carry table (the threads-gate row closed) and every still-standing red owned by M10.

M9's arc, honestly stated (five beats, from AMENDMENTS 1–7 and the T7
measurement record): (a) **the settings-move neutrality held exactly**
— T1's relocation of the settings layering from epic-cli to
epic-engine (`4a0d7b3f5`) was pure relocation by construction and by
proof (the CLI byte-invariance lap green at T1; census-invariant at
1632 = 1629 + 3 SessionLayer pins, the eight moved tests
count-invariant), the one engine-side change of its kind M9 was
chartered (the plan's law is two-clause — this MOVE, plus NEW additive
read-only faces: the T3 `DriverSink::board_snapshot` hook in epic-router
and the T5 `airline_segments` face in epic-drc, both default-path-inert,
named in the erratum below);
(b) **the session-parity proof architecture is the milestone's
structural center** — the session IS the CLI path (the same
`pipeline::full::run`, the same moved resolution predicates, the same
merged layers), so the GUI cannot diverge from the CLI because it
cannot reach the engine any other way; the acceptance proof is
byte-identity, proven 17/17 at T7 with the two kill-face rows (bm01,
bm10) completed by natural-wall CLI re-runs and byte-proven twice
(independently by the coordinator and the spec reviewer —
`logs/M9-T7/evidence/coord-bm01-bm10-byte-proof.log`);
(c) **the shell's thinness is a law that held** — the module split
(ungated `shell.rs` pure faces + pins vs `desktop/*` behind the
default-off feature) survived the whole milestone, and the ONE
census-pinnability leak (~15 lines of fit-zoom math gated inside the
shell) was found by the T6 quality review and hoisted + pinned at fix2
(`94e17f873`); (d) **the falsified-premise record is the cycle's
discipline story** — AM2's three charter bugs (the
`deterministic_budgets` default, the pre-route cancel face, the bm08
multi-pass premise) were all caught by the implementer with probes;
and at T7 the coordinator's own stop-flag mechanism verification
(pass granularity) was WRONG and the quality review caught it from the
tree — the optimizer stage runs on a FLAGLESS `StopFace::default()`
(`full.rs:341-342`, the documented T9 wiring artifact at `:49-52`), so
the GUI's mid-OPTIMIZATION cancel is structurally ineffective
(pre-existing and documented, NOT an M9 regression — M9 touched no
engine stop wiring); buglog 224's amendment is the model erratum;
(e) **the render-perf measurement closed a chartered question by
measuring it** — `project` at 0.004 ms median (bm08) / 0.160 ms
(interf_u) vs the 16.6 ms/frame budget (0.02% / 1.0%), per-pass-bound
snapshot 1.2 / 58.6 ms — the wgpu-callback follow-up NOT chartered
(the measurement-overrides-plan identity); and the two chronic walls
read as wall-overrun COMPLETIONS with honest PROCESS walls — bm01
3589 s and bm10 1372 s vs the 1800/900 s tier budgets (QF2-corrected
digit provenance: engine-echoed "completed in N s" lines are STAGE
walls; process walls come from the log's START/END windows).

Criterion verdicts (evidence: `logs/M9-T7/report-t7.md` — §1 what ran,
§2 the battery table + aggregates, §3 the render-perf verdicts, §4 the
re-faces + carry table, §5 the gate lap; `logs/M9-T6/report-t6.md` the
smoke evidence; AMENDMENTS 1–7 in the GUI plan):

- **Criterion 1 (the full workflow without Java, on the session path)
  MET.** The T7 battery: 17/17 fixtures byte-identical between
  `Session` load→route(default)→export and `epic-cli route` on the same
  fixture×settings — all 11 Tier A (bm01 LAST per its discipline),
  3 Tier B (bm10, 1Bitsy/unrouted [PCBench], interf_u), 3 crafted;
  15 in-slice TRUE + bm01/bm10 completed by natural-wall CLI re-runs
  and byte-proven twice (T7 §2 + the coordinator byte-proof); all four
  `LoadError` arms pinned at T2 (parse/IO hard-fail; `OutlineMissing`
  warn-and-continue). The interactive demonstration rides the T6
  evidence forward on the rust/ CODE-tree-byte-identical-since-`94e17f873`
  argument (re-verified at T7 and T8; the only post-T6 `rust/` changes
  are the docs `README.md` and `SEAM.md`): the overlays-ON bm08 PNG
  (`logs/M9-T6/evidence/23-bm08-smoke-overlays.png`, CONFIRMED by
  independent eyes) and the mid-route cancel smoke
  (`22-cancel-smoke.log`: `final_state=CANCELLED`, no export).
  Known bound, honestly recorded: mid-OPTIMIZATION cancel is
  structurally ineffective (beat d; M10 carry — buglog 224).
- **Criterion 2 (renders-never-mutates, structural) MET.** The Opening
  rules grep law (no `&mut Board`/`&mut SearchTreeManager`, no board
  mutation calls in epic-gui, tests included) green at every task that
  touched the crate (T4/T5/T6 records; the T6 face extended to
  `src/` including `desktop/` plus the bin) — no mutation surface
  exists in epic-gui; board access is `&Board` read faces only.
- **Criterion 3 (the §7 view set with headless goldens) MET.** The
  headless core: the T4 trio of render goldens (bm08, a PCBench
  reference, a crafted world) + the T5 six overlay-golden extensions +
  unit pins (layer-visibility exclusion, cull ±1 both directions,
  overlay-flags-off, pan/zoom round-trip exactness, narrowing
  saturation, per-layer color override, dash geometry); each view
  family visible in the shell (the T6 PNG — ratsnest CONFIRMED by
  independent eyes; the other three families honestly absent on that
  board's clean COMPLETED face); tuning overlays present iff
  constraints declared (the crafted tuning fixture), congestion present
  iff the global stage ran (None at defaults, pinned).
- **Criterion 4 (zero regressions) MET.** Seven compares green (corpus
  5000/5000; DSN 1,332; ses 20/20; ses-snap 5/5; index 33; undo 33;
  drc 17); events 3,520 aligned; determinism canaries IMMOVABLE in-log
  ×2 (ses `dc271114…`, manifest `cf607714…`); Tier A CI gate face 11/0
  (Σ 313.4 s, DNR-18-reconciled); census additions only 1629 → 1682
  (the ledger chain below); the settings move proven byte-neutral at
  T1; committed artifacts untouched EXCEPT the sanctioned NEW goldens
  (the T3 event-kind golden, the T4 render trio, the T5 six overlay
  extensions — each via its capture door, justified in its landing
  commit); nothing pushed.
- **Criterion 5 (the M8 deviations table dispositioned) MET.** The
  threads-gate CI wiring LANDED at T6 (`e7508f974` + `94e17f873`: both
  steps argv-exact and tripwire-pinned in `ci_tripwire.rs`; the local
  face green — 6 runs, 127.7 s, byte-identical across -mt 1/3/4); every
  other row re-faced at T7 row-for-row (Tier B 4 green / 5 red, Tier C
  1/2 — no drift vs the M8 record, none retried) and carried with
  named M10+ ownership (the table below); the corrected AM2 law stands
  untouched (M9 has no aesthetics criterion).

Deviations/carry table (the T7 §4 refresh, statuses verbatim — the
adjudication input; 13 M8 rows re-faced + the NEW stop-flag row):

| carry row | status at T7 | owner | what would close it |
|---|---|---|---|
| teardrop instrument-vs-consumer decision | UNCHANGED — M9 has no aesthetics criterion; the corrected AM2 law stands untouched | M10+ (aesthetics milestone) | an instrument decision (teardrop-aware term / explicit exclusion / letter kept) before any metric-facing pass |
| flow engagement question | UNCHANGED — the flow pass remains an exact no-op in the M8 record; no M9 face moved it | M10+ | recalibration / pre-flow normalizer / a documented decline |
| via_density routing-stage lever | UNCHANGED — no M9 routing-stage work | M10+ | a chartered lever with the via_density gap as target |
| F3 diagonal trigger (ARMED, corpora-gated) | CARRIED — no NEW geometry-corpus face appeared in M9 (the gui-render goldens are render faces, not geometry corpus; the events corpus is unchanged since M4) | M10+ | the next new-corpus milestone act must re-check the trigger |
| StickHub/CM5 pour measurement gaps | CARRIED — no pour battery in M9 (CM5's cv 29=29 re-confirmed at T7) | M10+ first pour battery | measured pour faces |
| bm01 adoption face + P2 partition skew | NEW DATA, row OPEN: defaults full-profile completes at natural wall (3465 s / 3139.9 s CLI) — a wall-overrun completion, not a completion failure; the ot=4 lever remains opt-in and NOT load-bearing for the router-only CI face (11/0 green at T7); P2 skew untouched | M9+/M10+ | the adoption decision (or documented decline) + the wall re-measured under it; P2 skew dispositioned |
| bm05 29>18 | RE-FACED EXACT (engine capability, congestion family) | M10+ | completion red cleared or an owned lever |
| bm10 tier-kill | FACE SHARPENED: the 900 s kill is a budget-kill on a board that COMPLETES at 1446 s session / 1220.5 s CLI [stage wall; process 1372 s — QF2 erratum 2026-09-30] with byte-identical SES both sides | M10+ | budget policy decision (tier budget vs natural wall) or a wall lever |
| stop-flag / mid-optimization cancel gap (buglog 224) | NEW AT T7, mechanism QF1-corrected: the optimizer stage runs on a FLAGLESS `StopFace::default()` (full.rs:341-342, the documented T9 wiring artifact) — external stops raised during optimization are invisible; the GUI's mid-optimization cancel is structurally ineffective (pre-existing, NOT an M9 regression) | M10+ | wire the parent face/flag into `run_optimization_stage` (the session/GUI face; CLI flow intentionally flagless per full.rs:49-52) or document the bound |
| interf_u 1>0 | RE-FACED EXACT (the M8 improvement face holds; must not silently freeze at 1) | M10+ | 0 incompletes or re-owned |
| multichannel_mixer / mm-unrouted parity floors | HOLDING EXACT (160=160; 128=128 / cv 285=285) | M10+ | parity floors re-recorded each milestone |
| StickHub / LimeSDR kills | RE-FACED EXACT (both kill @900 s) | M10+ | kills cleared or wall-attributed (bm10's sharpened face is the precedent) |
| 1Bitsy 2>1 | RE-FACED EXACT | M10+ | completion parity (1≤1) |
| bm04 3>2 | RE-FACED EXACT | M10+ | completion parity (2≤2) |

[T8 erratum on AM7 charter input (f): the SEAM verification was run and
it is NOT clean — `git log --oneline a6cf56d0b..HEAD --
rust/crates/epic-router/` names ONE CODE commit, `3f20acafd` (M9-T3; at
the post-append tree the same log also names the docs-only SEAM append
`03f911836`, recorded below): the
chartered AM3 additive `DriverSink::board_snapshot` hook +
`DRIVER_SINK_METHOD_COUNT` forwarding-exhaustion guard + the six
mirror calls (event_sink.rs, batch.rs, fanout.rs, optimizer.rs,
pass_runner.rs — all additive, default-path-inert, byte-invariance
re-proven at T7). The premise "epic-router untouched across M9" was
therefore falsified at verification. The SEAM.md append LANDED
at `03f911836` (the coordinator's follow-up commit, +17/−0 append-only;
T8's binding zero-rust-files law — the commit-proof gate `git diff
--stat HEAD~1..HEAD -- rust/` must print nothing — forbade it in the
task's own commit).
The same verification census over ALL engine crates: epic-board and
epic-geometry untouched; **epic-drc touched by exactly ONE commit —
`7a61ef5f5` (M9-T5)**: the additive `airline_segments` face +
`AirLineSegment` in `incompletes.rs` (+136/−4, the parity row path
byte-identical per AM5) — disclosed in AM5 but in no close-out face
until this erratum (quality-review QF1). Errata applied pre-adjudication
2026-09-30: header parentage corrected (spec-review F1 — the true chain
`3b9e0d8fb` → `8a024eed8` → `e5591969c` → `9bc40ae90`), the
amend-garbled header fragment cleaned, beat (a) rewritten to the
plan's two-clause law, and the byte-proof citation given its full path
— that erratum commit's body is the record.]

The census ledger: 1629 → 1632 (T1, +3 SessionLayer pins) → 1642 (T2)
→ 1649/1650 (T3 +7/+1) → 1659/1660/1663 (T4 +9/+1/+3) → 1672/1673/1673
(T5 +9/+1/+0) → 1680/1681/1682 (T6 +7/+1/+1) → 1682 (T7 +0, measured
thrice — implementer, coordinator, spec reviewer). The engine tree is
frozen at census 1682/0/19 across the close-out (T8: docs-only, zero
rust files); the verdict slot above is FILLED — adjudicated 2026-09-30
at tree `66f563cf5` (this fill commit's parent; the M4-T17d/M5/M6/M7/M8
pattern; the deviations/carry table and the criterion verdicts above
were its inputs; the adjudicator's E-ADJ-1/E-ADJ-2 precision errata are
applied in this commit — its body is the record).

M10 exit note (T7, 2026-10-01, at tree `36bcc7cd2` — the prior commit;
the close-out commit is its child — the deviation-3 lesson: the note
names the prior commit, the T8 fill names its own parent).
**VERDICT: M10_EXIT_WITH_DEVIATIONS** — adjudicated 2026-10-01 at tree
`ef6f5a736` (this fill commit's parent; the M4-T17d/M5/M6/M7/M8/M9
pattern), by the fresh-opus terminal adjudicator (task record
`logs/M10-T8/report-t8.md`; AM8 in the plan is the committed carrier).
All six concrete exit criteria **MET** on the adjudication's own
re-derivations at the M10-final tree:

- **Criterion 1 (stop-flag wiring) — MET**: the wiring hunks live at
  `full.rs:341` (`parent_stop: &StopFace`) and `:375`
  (`StopFace::from_flag(parent_stop.flag().cloned())`), the accessor at
  `batch.rs:245`; the named wiring/cancel pins re-run green by the
  adjudicator (`m10_t1_optimizer_stage_shares_the_parent_flag`,
  `m10_t1_flagged_parent_without_a_raise_runs_clean`,
  `mid_optimization_cancel_is_observed_by_the_optimizer_stage`); DNR-16
  both-directions kills at T2's fix round; CLI byte-invariance structural
  (the flagless production face) and measured (17/17 session shas,
  T1/T3); buglog 224 `fixed`.
- **Criterion 2 (Q7 hoist + Q6 decision) — MET**: `final_state_for` at
  `batch.rs:312`, imported and consumed by both hosts (`session.rs`,
  `route.rs`); the Q6 guard `RouteError::AlreadyRouted` fires before any
  merge/pipeline work, pinned `reroute_is_disabled_with_a_clean_error`;
  the decision text is what the migration guide carries.
- **Criterion 3 (deviations dispositioned) — MET**: the 13-row table
  (`:1162-1174`) byte-verbatim vs T3 (block cmp IDENTICAL — proven by
  the T7 spec reviewer, the T7 coordinator post-fix, and the
  adjudicator); row-for-row agreement with AM3's condensed dispositions;
  floors re-recorded HOLDING EXACT (160=160, 128=128, cv 285=285).
- **Criterion 4 (Java sunset) — MET**: 47 `.java` keepers / 49 oracle
  files / zero gradle surface, re-derived; the live-reference grep
  re-derived at exactly 33 governed lines (16 `AGENTS.md` + 16
  `docs/architecture.md` + 1 `scripts/pcbench/README.md`);
  `rust-check.yml` untouched.
- **Criterion 5 (2.0 released) — MET**: `rust/Cargo.toml`
  `version = "2.0.0"`; the `--version` face live-probed from the
  committed tree — `epicrouter 2.0.0`, exit 0; the release tarball is
  not on disk at adjudication time, so its sha256 was not re-checkable —
  the packaging faces (`package-linux.sh`, the reworked
  `create-release.yml`) are present and AM5's banked artifact sha stands
  as the record.
- **Criterion 6 (zero regressions) — MET**: census **1690/0/19 exact
  over 41 suites** (declared-before, adjudicator-re-summed, coordinator
  re-summed), additions-only 1682→1690; canaries EXACT ×2 at the final
  tree (ses `dc271114…91deb` IMMOVABLE; manifest `c6644a91…1f4d`, the
  AM5 sanctioned successor); goldens integrity = EXACTLY the sanctioned
  T5-A′ 7-golden `manifest_sha256` re-capture, nothing else; never-push
  verified on all three faces (only `upstream` in `git remote`; 13
  remote refs ALL under `upstream/`, `upstream/master` at the pinned
  `e7f9bdf1`; `epic/main@{upstream}` unresolvable — no tracking ref, no
  push target).

The verdict is WITH deviations, not clean: twelve of the thirteen carry
rows ride post-2.0 with named owners + exit conditions (the carry table
above, cells verbatim, is the authoritative deviations list; row 13/Q6
is the sole in-milestone closure; row 10 is a standing re-record
obligation; rows 6/8 are landed DECISIONS whose residuals — P2 skew, the
bm10 wall lever — ride). The adjudicator re-faces them against zero
production-code drift since T3's measurement (post-T3 engine deltas are
comment-only or test-module-only; canaries and goldens exact at the
final tree), so the T3 battery still governs.

**Adjudication errata:**
- **E-ADJ-1 (precision-only):** the count-errata chain above quotes the
  codespell ignore-list as **43 entries** — true at T4's measured tree
  (`cfa8a1bef`) — but the M10-final tree carries **39**: T5 commit C′
  (`8d5e3a4c9`, the AM4 Q-2 intake) pruned exactly the four risky tokens
  `ot`/`padd`/`ser`/`abd`. Criterion 4's keeper digits (47/49, 33 lines)
  are unaffected and re-derive exactly.
- **E-ADJ-2 (record-only):** `git for-each-ref refs/remotes` at the
  final tree names 13 refs, ALL under the graft remote `upstream/` — not
  master alone. The never-push substance is intact: no origin exists,
  `epic/main` has no tracking ref, and nothing was ever pushed anywhere
  the repo can see.

The loop's mandate ("until m10 and everything is complete") is
**DISCHARGED** by this fill; no M11 opens.

Criterion verdicts — each **OPEN** here; the terminal adjudication
judges THESE (the plan's concrete criteria) against the cited evidence
chains:

1. **The stop-flag wiring landed and proven — OPEN.** Landed at T1
   (`01b6b2f93`): `run_optimization_stage` gains `parent_stop:
   &StopFace` and builds its stage face OVER the parent's shared flag
   (buglog 224 → `status: fixed`); the fix round `21f24b982` corrected
   the poll-site citations and documented the outbound direction
   (AMENDMENT 1). The deterministic mid-optimization cancel pin exists
   in both hosts (epic-router wiring pin; epic-engine end-to-end pin,
   DNR-16 both directions at T2's fix round — two mutants dying at two
   different assertion sites, AM2). CLI byte-invariance holds
   STRUCTURALLY (the CLI production stop face is flagless by
   construction — a `None` flag can never be raised) and was re-proven
   MEASURED at T1 and T3: all 17 battery session shas byte-identical
   across both laps, 15 comparable rows `byte_equal TRUE`, bm01/bm10
   watchdog-honored rows whose session shas match their T1 records (a
   literal bm10 session-vs-CLI compare cannot read TRUE post-T1 by
   construction — see the carry-table preamble). The seven compares +
   canaries green at every lap (T1/T3/T4/T5/T6).
2. **The Q7 hoist landed and the Q6 decision recorded — OPEN.** T2
   (`a0b49a726`): one `final_state_for` beside `StopReason` in
   `epic-router` (body byte-verbatim from both deleted copies), both
   hosts consume; the epic-engine-cannot-depend-on-epic-cli constraint
   resolved. The Q6 product decision: the probe measured a SILENT
   `Ok` re-route; the landed face is `RouteError::AlreadyRouted {
   final_state }`, the guard firing before any merge/pipeline work
   (fix round `21285ae8e` adds the second-route CANCELLED pin killing
   both review survivors, AM2). The AM2 decision text is what the
   migration guide carries.
3. **The deviations table dispositioned at the M10 moment — OPEN.** T3
   (zero commits, tree `e47ce50f7`; AMENDMENT 3 banked at
   `f9e9ef6e9`): every one of the 13 carried rows re-faced against
   fresh battery/walk runs and dispositioned — CLOSED (row 13, Q6) or
   re-owned post-2.0 with a named owner + exit condition; the parity
   floors re-recorded HOLDING EXACT (multichannel_mixer 160 = 160;
   mm-unrouted 128 = 128, cv 285 = 285). The table below is the
   verbatim record.
4. **The Java oracle sunsetted — OPEN.** T4's five-commit chain
   (`9c967a937` the enumeration — 3,359 files, −319,384 lines —
   `ed07e2cfc` CI/scripts/docs, `8da5a20fe` the find_repo_root repair,
   `cfa8a1bef` + `97e587e0c` the pre-commit greens; never rewritten;
   AMENDMENT 4 at `02f55313f`). `rust-check.yml` green on the
   Java-free tree, UNTOUCHED itself (ci_tripwire pins hold). The
   count-errata chain, quoted: java keepers = **47 `.java` files**
   (erratum 1), reconciled by erratum 4 as **49 total files** in
   `rust/harness/oracle/` (47 `.java` + `t2agent/README.md` +
   `manifest.txt`); the live-reference grep = **33 lines**, all
   banner/addendum/struck-through governed, zero LIVE refs (erratum 3
   supersedes erratum 2's "21"); codespell ignore-list = **43
   entries**. The parity record's preservation stated in the sunset
   commit body: git history + the grafted `origin/master` at
   `e7f9bdf1` + the committed goldens.
5. **2.0 released — OPEN.** T5's four-commit chain (`af1b1471b` the
   version-blind digest + the sanctioned 7-golden re-capture +
   the canary handoff; `b7d90011f` Cargo 2.0.0 + `--version` +
   `package-linux.sh` + the reworked `create-release.yml` + the
   artifact smoke; `8d5e3a4c9` the AM4 intakes; `50a43bce0` fix round
   1; AMENDMENT 5 at `a3b9bc91f`): workspace version 2.0.0, the
   `--version` face, the CI-built Linux x86_64 tarball
   `epicrouter-2.0.0-x86_64-unknown-linux-gnu.tar.gz`, sha256
   `0d055d42fe00ab1f79e93d46beadd7b8b041fd2a4cac48e31983f43217ebe844`,
   smoke-tested from the unpacked artifact. T6 (`bb976d8a7` + fix
   `39c64b732`; AMENDMENT 6 at `f61b5af06`): the migration guide
   (`docs/migration-guide.md`, ships IN the tarball), the README/
   architecture faces. The C2 KiCad decision recorded: 2.0 ships
   CLI + GUI, no jar/plugin face — the DSN/SES interchange IS the
   integration.
6. **Zero regressions to the last — OPEN.** Census additions only:
   1682 → 1690 (the ledger chain below). Canaries: ses
   `dc271114627632d6a959891559d2ee837c180142c06fb36fde20f441f0f91deb`
   IMMOVABLE at every lap; the manifest canary moved ONCE, by
   SANCTIONED rotation (T5 A′, the Option-B adjudication — the raw
   `cf607714…` retired, the version-blind successor `c6644a91…`
   reproduced exactly at 2.0.0 and at every later tree). Nothing
   pushed. The committed artifacts untouched except the sanctioned
   delta faces (the T5 7-golden `manifest_sha256` re-capture; the
   T4-era NEW-golden captures all preceded M10). The goldens-
   integrity probe over ALL of M10's span (`74f17bd9e..36bcc7cd2`)
   reads EMPTY except the sanctioned A′ face (see the close-out
   verification faces below).

Deviations/carry table — T3's dispositionment, the cells VERBATIM (the
verbatim-cell law; full digests where the record carries them; the
post-2.0 ownership in the disposition cells, intact). Preamble, per
the banked intakes: the battery behind these rows is **15 comparable
rows `byte_equal TRUE` + bm01/bm10 watchdog-honored CLI_KILLED rows
whose session shas match their T1 records** — NOT a "17/17" headline
(a literal bm10 session-vs-CLI compare cannot read TRUE post-T1 by
construction: the flagless CLI at natural wall produces the
pre-wiring board `9063a828…`; a natural-wall re-run has witness value
only as a policy-divergence record). The load-bearing session-sha
literals, carried FULL because git-ignored `logs/` is not a
comparability carrier post-sunset: bm01
`5c34ef4c4c51f4144d1aeb092e51c63bc5a707841a32f4a5cdc4c9a8270ffc9a`,
bm10
`66803c464baf628c2a93878a6b4c18ff04a3a0c69951e6f9fdd3cc3229d40955`.
**The committed goldens (`rust/harness/baselines|corpus`, the
events-golden, `config/tiers.yaml`) are the post-sunset comparability
carrier** — every T3 face is golden- or Rust-internal, no live-Java
dependency, so the T4 sunset made nothing incomparable.
The `(ev N)` cites in the cells are T3's evidence-log cites into
git-ignored `logs/M10-T3/` task records; the committed carrier is
AMENDMENT 3. In the rows,
**ot=4 is the opt-in optimizer-threads=4 wall lever** (design `:800`,
the deterministic candidate-parallelism face). The two budget-stop
rows carry erratum 2's scoping: bm01's wall-clock stop is
**convergence-backed** (its 1800 s board is byte-identical to its
3465 s natural-wall board — structurally byte-stable); bm10's is
**load-positioned** (its 900 s board differs from its 1446 s natural
board — byte-reproducible at the budget face EMPIRICALLY, observed at
T1 and T3 under comparable load, no structural guarantee). The M9
14-row intake loses two rows before this table — bm04's threads-gate
row CLOSED at M9-T6 (`e7508f974`+`94e17f873`); the stop-flag WIRING
row CLOSED at T1 (`01b6b2f93` + fix `21f24b982`, buglog 224 fixed).

| # | row | fresh face | disposition |
|---|---|---|---|
| 1 | **teardrop instrument-vs-consumer (DECISION)** | M10 landed zero geometry (T1 wiring, T2 API guard); all 17 battery shas = T1 → zero aesthetics movement; no M10 aesthetics criterion | **DECISION: the instrument letter is KEPT; the tension stands documented.** The four-metric instrument is an M8-vintage measurement instrument, not a 2.0 product face; changing it now is the re-baseline the M8 criterion forbids and M10 has no criterion to serve. Any instrument change (teardrop-aware term / explicit exclusion) is a POST-2.0 aesthetics-milestone act, chartered BEFORE any metric-facing pass. Owner: post-2.0 aesthetics milestone; exit condition: that instrument decision before any metric-facing pass. (ev 30/31/32; design `:680`) |
| 2 | **flow engagement (DECISION)** | flow pass remains an exact no-op on every fresh face — walk rows `introduced 0/<0> passes 18/<18>` throughout | **DECISION: documented decline for 2.0.** No aesthetics criterion exists to engage flow for; the pass ships available-but-unengaged; the migration guide states the default pipeline does not engage it. Owner: post-2.0 aesthetics milestone; exit condition: recalibration or pre-flow normalizer making flow engage without regressions, or the decline made permanent in product docs. (ev 22/23) |
| 3 | **via_density lever (DECISION)** | no M10 routing-stage work; via faces byte-stable; corrected band **1.5752** (F-ADJ-1) is the record — DNR-19 re-derivation: 2.4752 matches no recorded face, bend's 2.6488 matches its own gap proving the gap-of-means law; gap stands 1.7738 > 1.5752 | **DECISION: carried post-2.0; M10 does not open the lever** (engine work under an aesthetics criterion M10 lacks). Owner: post-2.0 aesthetics/routing milestone; exit condition: a chartered lever targeting the via_density gap, measured under the corrected 1.5752 band. (design `:680`, `:848`) |
| 4 | F3 trigger re-check | **probe run: ZERO commits touched `rust/harness/corpus|fixtures|baselines` in 379ee6c47..e47ce50f7** (empty log + empty diff; the M10 span is plan + T1 wiring/pins + T2 hoist/guard + wolf/docs only) | **CARRIED (armed, corpora-gated)** — no new geometry-corpus face in M10; the next corpus-touching milestone act must re-check. Owner: M10+ first corpus act. (ev 40) |
| 5 | StickHub/CM5 pour battery | **RUN** (the M6-T6 advisory `islands` face, seconds/board): StickHub pours=5 islands=0; CM5 pours=18 islands=1 (+5V layer=3 island, bbox (783043,−811000)−(1329000,−239000)); both rc=0; CM5's completion face re-confirmed green in the walk (6=6, cv 29=29 @754.2 s) | **SHARPENED, not closed — carried post-2.0.** First measured pour faces = INPUT-pour population + island detection (advisory); the ROUTING-side pour face (`contains_plane`, Issue 093/152 plane-routing clearance risk) remains UNMEASURED — no argv measures it. Owner: post-2.0 first routing-side pour battery; exit condition: measured post-route pour faces. (ev 41; ev 23) |
| 6 | **bm01 adoption + P2 skew (DECISION)** | battery session side COMPLETED at 1800.493 s, **watchdog HONORED (the T1 wiring), bytes IDENTICAL** (`5c34ef4c…`) to the M9-T7 natural-wall completion (3464.982 s) — the lost 1664 s of post-watchdog passes were provably no-ops; router-only gate face 11/0 | **DECISION: DECLINE ot=4 adoption (stays opt-in, non-load-bearing); ADOPT the watchdog-honored budget face as bm01's 2.0 posture.** Post-T1 the session path stops AT budget on the identical final board, so the natural-wall overrun face no longer exists on the product path; the CLI path stays flagless by design (its in-slice face is the honest kill). The wall motivation for ot=4 evaporates on the session path; the router-only CI face never engaged it. **P2 skew: untouched, carried post-2.0** (owner: post-2.0 partition act; exit: skew measured and dispositioned). (ev 30, ev 17) |
| 7 | bm05 29>18 | RE-FACED EXACT: RED 29>18 @289.3 s (M9-T7 256.1; M8 282.3); localizer NOTE bounded 1800 s | CARRIED post-2.0 (engine capability, congestion family). Exit: red cleared or an owned lever. (ev 22) |
| 8 | **bm10 budget-policy (DECISION)** | battery session side COMPLETED at 900.914 s, watchdog honored, sha `66803c46…` = T1's (reproduced twice now); CLI killed in-slice @900.176; walk kill @900.1 unchanged | **DECISION: the tier budget IS the binding policy — keep it; timeouts never raised.** Post-T1 the session path honors the 900 s budget; the walk red is a budget-kill record face, not an engine defect; M9-T7's natural-wall completion (1446 s) proved the engine completes unbounded, so "budget vs natural wall" is answered IN FAVOR OF THE BUDGET for 2.0 (bounded, deterministic, byte-reproducible at the budget face). A wall lever (beating rather than honoring the budget) is owned post-2.0. Exit: bm10 under 900 s, or this decision quoted as bm10's product face. (ev 32, ev 22) |
| 9 | interf_u 1>0 | RE-FACED EXACT: RED 1>0 @71.8 s; battery row incomplete 1, sha = T1 | CARRIED post-2.0 (must not silently freeze at 1). Exit: 0 incompletes or re-owned. (ev 22, 32) |
| 10 | multichannel parity floors | **RE-RECORDED:** multichannel_mixer **160=160** @108.0 s; mm-unrouted **128=128, cv 285=285** @167.3 s — HOLDING EXACT, no drift | Floors re-recorded (this table is the record). Re-record each milestone. (ev 22) |
| 11 | StickHub/LimeSDR kills | RE-FACED EXACT: both kill @900.1 s (B and C walks); StickHub's pour face now additionally measured (row 5) | CARRIED post-2.0. Exit: kills cleared or wall-attributed (row 8's budget-face decision is the attribution precedent). (ev 22, 23) |
| 12 | 1Bitsy 2>1 | RE-FACED EXACT: RED 2>1 @271.1 s; battery row incomplete 2, sha = T1 | CARRIED post-2.0. Exit: completion parity (1≤1). (ev 22, 32) |
| 13 | Q6 product decision | decided + landed at T2: `RouteError::AlreadyRouted { final_state }`, guard fires before any merge/pipeline work | **CLOSED**: code `a0b49a726`, fix `21285ae8e`, AMENDMENT 2 banked (incl. the T6 migration-guide intake). Live corroboration this cycle: the pre-T2 M9-T7 runner no longer compiles against the current `Session::route` signature — the Result face is now the API surface (ev 00; session.rs `:332-343`) |

M10's arc, honestly stated (the milestone's real shape, not a
sanitized one; every beat from AMENDMENTS 1–6 and the task reports):

(a) **The milestone's one engine-code change was preceded by a
charter-time falsified premise.** The plan's recon said `StopFace`
"has NO public flag accessor" — stale (it landed at M3 `507eacc76`).
The T1 implementer probed, refused the duplicate, and filed the
erratum; the law since: charter-time symbol-absence claims get a
`git log -S` probe.

(b) **The honest cancel face.** The T1 wiring makes external raises
visible mid-optimization, but the session's final state on a
mid-optimization cancel is **COMPLETED**, not CANCELLED (the cancel
lands after routing returned `Ok(true)`; Java propagates no stop
state) — the state-name question is a PRODUCT decision deferred to
the exit note and migration guide (T6 carried the two-case face).

(c) **The T5 BLOCKED → Option-B adjudication.** The 2.0.0 version
bump necessarily rotates the raw-bytes manifest canary; the T5
implementer went BLOCKED at probe stage, the coordinator widened the
exposure (2 census pins + 7 committed goldens + the latent
`FREEROUTING_GIT_SHA`), and Option B was chosen over re-rotation:
normalize `app_version`/`git_sha` out of the sha256 at BOTH digest
call sites — the manifest artifact keeps its true version,
`route.rs` behavior untouched, and the digest is bump-neutral at
every future bump by construction. The ONE sanctioned canary
rotation of the milestone; the raw `cf607714…` retired, the
version-blind successor `c6644a91…` reproduced exactly at 2.0.0 and
every later tree.

(d) **The reboot-interrupted census, kept as a witness.** T5's census
run was killed mid-run by a machine reboot (AM5 lesson 16: a warm
SendMessage-resume across a reboot preserves the evidence chain —
DNR-17's nothing-survives-a-reboot applies to bins, not
transcripts); the killed run is kept as a witness, re-run declared
and green after the resume.

(e) **The count-errata discipline.** T4's java-keeper count went
16 → 21 → 33 on one grep (errata chain, AM4 lesson 9: a count
correction is a count — re-derive the full enumeration before
publishing an erratum), and the 47-vs-49 reconciliation (erratum 4)
closed it: 47 `.java` + `t2agent/README.md` + `manifest.txt` = 49
files, no fabrication.

(f) **The false-green pre-commit lesson, twice witnessed.** T5's
"pre-commit 13/13 green" was 11 MEASURED (`stages: [manual]` hooks
are silently omitted from `--all-files`; AM5 erratum 2 / lesson 11's
recurrence — the configured count is not evidence); T6's round-1
pre-commit ran BEFORE the new guide was staged and was therefore a
false green over an untracked file (AM6 lesson 18: `pre-commit
--all-files` judges `git ls-files` — unjudged ≠ passed; run it AFTER
staging). This close-out runs its pre-commit AFTER staging (lesson
18) and reports the MEASURED hook count (lesson 11).

(g) **The stale-bin lesson hardened twice.** T5's fix round committed
15 unformatted `router_compare.rs` lines (its fmt gate ran BEFORE the
ripple edit; T6 repaired formatting-only, red round kept — erratum
3, lesson 17: gates judge the COMMIT); and the DNR-17 stale-bin
guard's own bail message named a remedy that cannot always clear the
flag (a workspace-wide newest-source scan + no-op builds don't
refresh mtime) — reworded THIS commit to the forced-relink remedy
(lesson 19; the reword rides this close-out).

(h) **The contained T4 incident.** The mixed-line-ending hook
CRLF-normalized 23 committed golden files in the WORKING TREE during
pre-commit rounds — never staged (explicit-path staging held),
restored byte-exact, hardened structurally at `97e587e0c` (the
top-level exclude).

The census ledger: 1682 (M9) → 1685 (T1, +3) → 1686 (T2, +1) → 1686
(T3, +0 — the measurement task adds no pins) → 1686 (T4, +0) → 1687
(T5 A′, +1 `manifest_digest_is_version_blind_and_content_sensitive`)
→ 1688 (T5 B′, +1 `version_flag_prints_epicrouter_2_0_0_and_exits_zero`)
→ 1688 (T5 C′, +0) → 1689 (T5 fix round, +1
`manifest_digest_bails_on_marker_absent_bytes`) → 1690 (T6, +1
`version_line_equals_the_package_version_face`) → the close-out
declares 1690 + 0 = 1690/0/19 expected (the two riding intakes add
no pins: a bail-message reword + a doc footnote), measured at this
close-out. The verdict slot is FILLED above — `M10_EXIT_WITH_DEVIATIONS`,
the terminal adjudication (T8); the loop's mandate discharged.

Sequencing rationale: speed (M5) lands before intelligence (M6–M8) so new stages are built on the settled parallel/arena architecture. The Java tree remains runnable throughout. Upstream `master` stays fetchable for DSN parser behavioral updates.

## 7. GUI

egui + wgpu: GPU canvas (100k+ items at 60fps — the Java CPU renderer is a real large-board complaint). Board snapshots arrive via a versioned engine event stream; the GUI renders, never mutates (mutations go through the same engine command API the router uses, so manual edits get push-and-shove for free). Views: board editor (layers/colors/pan/zoom), live per-stage routing progress, ratsnest, DRC markers, congestion heatmap (free once the global stage exists), tuning overlays. Settings UI mirrors the layered settings model.

## 8. Out of scope (2.0)

REST API/MCP parity (thin wrapping, post-2.0); blind/buried vias, back-drilling, impedance-driven stackup solving; placement co-optimization (engine designed so a placement oracle can drive it later); ML/RL components (evidence: classical wins on real boards today; optional congestion-forecast experiment noted for the future).

## 9. Risk register

| Risk | Mitigation |
|---|---|
| i128 overflow / exact-geometry port bugs | Checked arithmetic + BigRational fallback; property tests probing 2^62 boundaries; no `mul_add`; operation order identical to Java |
| Double-precision drift flipping threshold decisions | Port expressions verbatim first; rational/fixed-point predicates where Java compares doubles near equality; optimize only behind the differential harness |
| Tie-break divergence mistaken for regression | Gates compare scores/violations/completion, never textual SES; explicit tie-break policy (lexicographic on sortingValue, doorId, sectionNo) |
| Determinism broken by parallelism | Fixed net-ID partitioning; deterministic reduction; CI parallel≡sequential assertion |
| Search trees mutate during search (incremental reinsertion) | Index designed for incremental insert/remove (slab allocation, epoch reclamation); not per-pass arenas |
| Reference-identity `==` / `instanceof` port bugs | Enum discriminants + ID equality in Rust; grep Java for reference `==` during each port; trace-stream diffs catch survivors |
| FFI/boundary granularity (future in-process use) | Boundary at pass/job granularity only (~50ns/call is fine per pass, catastrophic per shape) |
| Scope/motivation drift | Every milestone ends in a usable artifact (M3 CLI routes; M5 2× faster; …) |
| GPLv3 obligations | Translation is a derivative work → Rust stays GPLv3; per-file attribution preserved; already a GPLv3 fork |

## 10. Companion research documents

The three full research reports (autorouter state of the art; Freerouting gaps with issue links; language/architecture evidence with sources) are preserved verbatim alongside this spec in `docs/superpowers/research/` for implementation-time reference.
