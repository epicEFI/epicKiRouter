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
(the gate stays pinned in-tree, green and hashed).)

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
**VERDICT: <per milestone adjudication>**

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
this tree; the M4 note's `:125` cites below are T13-era numbering —
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
remains the first task landing one.

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
