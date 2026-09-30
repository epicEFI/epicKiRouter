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
