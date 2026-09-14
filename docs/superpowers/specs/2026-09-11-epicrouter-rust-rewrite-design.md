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
