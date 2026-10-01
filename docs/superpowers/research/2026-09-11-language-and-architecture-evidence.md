# Language & Architecture Evidence for the Rust Rewrite

**Research companion to:** `docs/superpowers/specs/2026-09-11-epicrouter-rust-rewrite-design.md`

## Workload profile (from the actual code)

1. **Graph search with a boxed red-black-tree priority queue.** `src/main/java/app/freerouting/autoroute/maze/MazeSearchEngine.java` (1,258 lines) uses `SortedSet<MazeListElement>` backed by a `TreeSet` override; every expansion allocates a fresh immutable `MazeListElement` boxed into tree nodes. A flat d-ary heap over struct-of-arrays wins: no per-insert allocation, contiguous memory, O(1) amortized peek. Also: `LinkedList` collections; `FloatPoint` (double) heuristics mixed with exact integer geometry.
2. **Pointer-chasing spatial index.** `src/main/java/app/freerouting/datastructures/ShapeTree.java` — binary tree of heap-allocated `Leaf`/`InnerNode` with parent pointers, `instanceof` dispatch, fresh `Leaf` + bounding shape per insert. `board/searchtree/` + `autoroute/maze/` ≈ 7,300 lines of constant re-insert/remove during ripup-reroute. The repo's own docs describe cumulative JVM allocations in tens of GB per job — direct evidence of allocation churn.
3. **Exact rational geometry.** `geometry/planar/Point.java` exposes `Point.getInstance(BigInteger x, BigInteger y, BigInteger z)` — homogeneous rational coordinates; 35 files in `geometry/planar/`. A native rewrite must decide where `i128` suffices and where rational fallback is required.
4. **Determinism is a product requirement** (`Random(ripupCosts)` seeding) and the router is single-threaded in the maze search ([#289](https://github.com/freerouting/freerouting/issues/289), ~11% of 8-core CPU).

## Why not "keep Java"

- Allocation-heavy workloads are GC's worst case: GC'd languages need up to 5× heap to match malloc/free throughput ([Hertz et al., PLDI 2005](https://people.cs.umass.edu/~emory/pubs/gcvma.pdf)); >10% of CPU in GC at high allocation rates is common ([Datadog](https://www.datadoghq.com/blog/java-performance-tuning/)).
- Pauses aren't the issue; throughput and cache locality are ([Morling on ZGC](https://www.morling.dev/blog/lower-java-tail-latencies-with-zgc/), [Mill benchmarks](https://mill-build.org/blog/6-garbage-collector-perf.html), [Netflix GenZGC](https://netflixtechblog.com/bending-pause-times-to-your-will-with-generational-zgc-256629c9386b)). Every `MazeListElement`/`Leaf`/`InnerNode`/`BigInteger` is a pointer indirection; arenas/SoA eliminate them per pass.
- JVM escape hatches not ready: Vector API still incubating ([JEP 508](https://openjdk.org/jeps/508)); Valhalla value objects only reach preview in JDK 28 ([project page](https://openjdk.org/projects/valhalla/)); virtual threads don't help CPU-bound flood fill.
- Realistic Java-side tuning gains: 1.5–3×. The 10–100× goal requires changing the data representation.

## Rust vs C++

- Broadly a wash in raw speed ([arXiv 2410.19146](https://arxiv.org/abs/2410.19146) with community critique); choose on engineering grounds.
- Rust wins here: memory safety under aggressive data-oriented refactoring; `cargo-fuzz`/`proptest` for the parity harness; `unsafe` confined to SIMD kernels; trivial C ABI (`cbindgen`); clean cross-platform packaging. C++ wins: EDA ecosystem readability (KiCad PNS router, OpenROAD), hiring pool.
- Verdict: **Rust**, C++ acceptable fallback.

## Comparable EDA evidence

- KiCad push-and-shove router: C++, CERN-origin; KiCad *removed* its old autorouter and points users at Freerouting (quality, not language, decided that). [KiCad docs](https://docs.kicad.org/8.0/en/pcbnew/pcbnew.html#autorouter)
- OpenROAD: C++ core + Tcl — the canonical "native compute core, thin shell" architecture. [project](https://theopenroadproject.org/)
- LibrEDA: Rust EDA framework (early-stage; prior art for geometry crates). [site](https://libreda.org/)
- pcb-rnd: C lineage; small C example autorouter. tscircuit: TS, parallelism + simple algorithms.
- KiCadRoutingTools: Python + Rust A* core, ~10× faster — direct precedent for this exact port.
- User-documented scaling limits: 56-hour optimizer runs; single-threaded (#289).

## Expected speedup (honest)

Data-structure replacement (TreeSet→flat heap, pointer tree→arena BVH, BigInteger→i128): **3–10×** on hot loops. Deterministic net-level parallelism: **2–6×** more. SIMD kernels (octagon/box intersection batches): **2–4×** on those kernels. Compounded 10–100× on large boards is plausible *for the core*, each layer measured separately. Largest published multipliers (e.g., [Discord Go→Rust](https://discord.com/blog/how-discord-supercharged-network-codec-performance-with-rust-aka-2-7x-speedup-22x-better-p99)) involved algorithmic restructuring, not transliteration.

## Architecture decision

**Primary: Java Swing GUI/API/MCP/job scheduler + DSN/SES parsing stay in Java initially; routing core (geometry, indexes, maze engine, optimizer) becomes a Rust library behind a C ABI.** In-process via Panama FFM (~50ns/call — fine at pass granularity, catastrophic at per-shape granularity; [JEP 454](https://openjdk.org/jeps/454)). Out-of-process sidecar CLI as first milestone and permanent fallback (GPL-clean by construction; matches the existing KiCad-launches-Freerouting shape). Full GUI rewrite deferred to last (egui; [Rust GUI survey 2025](https://www.boringcactus.com/2025/01/11/rust-gui-2025.html) supports retained-mode-for-complex-UIs; wgpu canvas for the board). *(User decision 2026-09-11: full rewrite including GUI in Rust — the staged boundary above still defines the porting order; the Java side simply gets sunset at M10 instead of kept.)*

Boundary design: `epic_route(board_ir, settings, progress_cb, cancel_flag) -> ses/board` at job/pass granularity; board model owned by one side (start: Java-side model, per-pass IR; later: Rust-owned with mirrored snapshots); deterministic merge order for parallel sections; prebuilt cdylibs per platform; Java fallback engine remains the CI oracle.

## Zero-regression migration machinery

The repo already has: 123+ DSN fixtures, fixture-test assertion DSL, slow-tagged suites, scoring model, "WIP vs v2.3.0" policy. Extend into:
1. **Golden corpus** (fixtures × settings matrix; frozen Java outputs: SES, per-pass counts, scores, violations, time). Semantic (structured, normalized) SES comparison — never byte equality.
2. **Differential runner** per commit: (Java, Rust) divergence reports; gates on score/violations/completion equal-or-better; time as trend.
3. **Two-engine toggling** (`engine=java|native|both`) — same job, identical seeds.
4. **Trace-level parity**: reproduce `FRLogger.trace` structured events (`RAW_SECTION` door/section/cost) behind a debug flag; first diverging event localizes porting bugs. Gate trace-string construction behind cheap checks (it's a measurable Java cost today).
5. **Porting order** (each behind the engine flag, each gate-green): Step 0 pure-Java representation wins (`LinkedList` removal, trace-string hoisting) → geometry kernel → spatial index (query-set equality on same mutation sequences) → maze engine (d-ary heap over arena slab, SoA MazeListElement) → optimizer + deterministic parallelism → FFM integration.
6. **Fuzzing/property tests:** cargo-fuzz (or Jazzer for Java parser) with fixture-seeded corpus — parser must never panic, round-trips canonically, whitespace-mutations are semantic no-ops; proptest/jqwik for intersection idempotence/commutativity, octagon bounding, expansion-value monotonicity, i128 overflow boundaries (~2^62), double-vs-rational epsilon consistency; cross-engine metamorphic tests (same mutation → same DRC verdict).

## Known pitfalls (catalog for implementation)

1. **BigInteger→i128 overflow** — checked arithmetic, `BigRational` fallback, overflow-hunting property tests ([Shewchuk robust predicates](https://www.cs.cmu.edu/~quake/robust.html)).
2. **Tie-break/ordering changes alter outcomes** — define tie-break explicitly (lexicographic on sortingValue, doorId, sectionNo); gate on scores, not textual equality.
3. **FP drift at thresholds** — forbid `mul_add`; keep operation order identical; rational/fixed-point predicates where Java compares doubles near equality; port first, optimize later.
4. **Determinism vs parallelism** — fixed partitioning by net ID; deterministic reduction; CI asserts parallel ≡ sequential.
5. **GC idioms don't transliterate** — no per-op immutable-object churn as naive `Box`; iterator early-exit → index loops; exception control flow (`safeMazeSectionCount`) → `Option`; **arenas must support incremental reinsertion** (trees mutate mid-search), e.g. slab + epoch reclamation.
6. **Reference-identity `==` / `instanceof`** → enum discriminants + ID equality; grep Java for reference `==` per port; trace diffs catch survivors.
7. **FFI granularity** — pass/job-level calls only; never panic across `extern "C"` (`catch_unwind` → error codes); document buffer ownership in the ABI before coding.
8. **Licensing** — translation is a modification → Rust port stays GPLv3 ([GPL FAQ](https://www.gnu.org/licenses/gpl-faq.en.html#TranslateCode)); in-process FFI forms a combined work (fine, both GPLv3); algorithms (ideas) are not copyrighted expression — but we're staying GPLv3 anyway; preserve per-file attribution.
9. **Human factors** — engine-flag strategy keeps Java live the whole migration; replicate ArchUnit boundary discipline as Rust workspace/lint rules from day one or the layering erodes.
