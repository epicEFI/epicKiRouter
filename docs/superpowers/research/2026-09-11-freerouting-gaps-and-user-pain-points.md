# Freerouting: Gaps, Complaints, and Feature Demand (September 2026)

**Research companion to:** `docs/superpowers/specs/2026-09-11-epicrouter-rust-rewrite-design.md`

## (a) Top 15 pain points (frequency × severity)

1. **No copper-pour/power-plane awareness** — #1 most-upvoted open feature request; every GND/VCC net must be excluded and routed by hand; workarounds documented as broken. [Issue #152](https://github.com/freerouting/freerouting/issues/152), [KiCad forum](https://forum.kicad.info/t/ground-planes-fill-zones-and-freerouting/11042)
2. **v2.x quality regression (2024–25)** — completion got dramatically worse vs v1.9 on some boards (300-connection board: 0 → 28 unrouted); one CLI case ran ~1000 passes consuming ~399,066 CPU-seconds. Root cause: board-specific parameter optimizations not applied to CLI jobs. [Discussion #508](https://github.com/freerouting/freerouting/discussions/508), [Issue #461](https://github.com/freerouting/freerouting/issues/461), [Issue #483](https://github.com/freerouting/freerouting/issues/483)
3. **DRC false negatives** — internal checker misses clearance violations; multithreaded optimization documented to generate violations. [Issue #103](https://github.com/freerouting/freerouting/issues/103), [#191](https://github.com/freerouting/freerouting/issues/191), [#558](https://github.com/freerouting/freerouting/issues/558), [#240](https://github.com/freerouting/freerouting/issues/240), [HN](https://news.ycombinator.com/item?id=43499992)
4. **Slowness / effectively single-threaded** — 1000+ net boards take hours-days; 11% CPU on modern 8-core; 56-hour optimizer runs. [Issue #289](https://github.com/freerouting/freerouting/issues/289), [Reddit](https://www.reddit.com/r/KiCad/comments/1brk8zc/freerouter_running_for_56_hours_and_still/), [Issue #390](https://github.com/freerouting/freerouting/issues/390)
5. **Optimizer OOM** — `OutOfMemoryError: Java heap space`; users hand-tune `-Xmx`. [Issue #420](https://github.com/freerouting/freerouting/issues/420)
6. **Ugly, non-human trace geometry** — the most repeated community complaint. [KiCad forum](https://forum.kicad.info/t/why-do-people-choose-to-not-use-the-auto-router/25849), [EEVblog](https://www.eevblog.com/forum/kicad/open-source-high-quality-autorouting-is-it-possible/), [Quilter review](https://www.quilter.ai/blog/the-2026-automated-pcb-layout-software-review-ai-vs-autorouters-vs-manual-design)
7. **No differential pairs or length matching** — requested 2021, closed 2023 unimplemented. [Issue #133](https://github.com/freerouting/freerouting/issues/133), [#716](https://github.com/freerouting/freerouting/issues/716)
8. **KiCad plugin fragility** — breaks across KiCad versions. [#360](https://github.com/freerouting/freerouting/issues/360), [#537](https://github.com/freerouting/freerouting/issues/537), [#71](https://github.com/freerouting/freerouting/issues/71)
9. **Net classes ignored / no routing priority** — layer assignments ignored; CLI has only an *ignore* flag. [#507](https://github.com/freerouting/freerouting/issues/507), [Discussion #327](https://github.com/freerouting/freerouting/discussions/327)
10. **Layer-direction preferences violated** — routes on inactive layers when via cost ≤ 45; can't constrain to one layer. [#230](https://github.com/freerouting/freerouting/issues/230), [#689](https://github.com/freerouting/freerouting/issues/689)
11. **Keepout handling broken** — crashes on KiCad keepouts; wrong rotation on rotated components; toggles not in CLI. [#185](https://github.com/freerouting/freerouting/issues/185), [KiCad GitLab #8753](https://gitlab.com/kicad/code/kicad/-/work_items/8753), [#620](https://github.com/freerouting/freerouting/issues/620)
12. **Crash patterns** — StackOverflow recursion in trace combine (#759, fixed Aug 2026), AIOOBE, macOS packaging. [#15](https://github.com/freerouting/freerouting/issues/15), [#39](https://github.com/freerouting/freerouting/issues/39)
13. **Fanout infinite loops / dense-package failures** — [#483](https://github.com/freerouting/freerouting/issues/483), [KiCadRoutingTools #614](https://github.com/drandyhaas/KiCadRoutingTools/issues/614); BGA breakout is exactly where users want automation.
14. **DSN round-trip data loss** — net-class clearances written incorrectly by KiCad's exporter; constraint set doesn't carry over; SES re-import loses track widths. [KiCad GitLab #14713](https://gitlab.com/kicad/code/kicad/-/issues/14713), [forum 60202](https://forum.kicad.info/t/solved-kicad-9-01-freerouting-2-01-drc-not-transferred/60202), [forum](https://forum.kicad.info/t/track-width-lost-with-freerouter/25509)
15. **Optimizer can't fix greedy first pass** — early traces lock in; commercial routers reportedly also struggle. [HN](https://news.ycombinator.com/item?id=43499992), [EEVblog](https://www.eevblog.com/forum/kicad/open-source-high-quality-autorouting-is-it-possible/)

## (b) Missing features (by demand)

1. Copper pour / plane-aware routing ([#152](https://github.com/freerouting/freerouting/issues/152))
2. Differential pairs + length matching ([#133](https://github.com/freerouting/freerouting/issues/133); paywalled at the only GPU competitor — [pcbautorouter.top pricing](https://www.pcbautorouter.top/pricing))
3. Net-class priority/routing order + enforcement ([#507](https://github.com/freerouting/freerouting/issues/507))
4. Robust keepouts incl. per-net rules ([#620](https://github.com/freerouting/freerouting/issues/620))
5. Push-and-shove during autoroute ([manual](https://freerouting.org/freerouting/manual/routing-options))
6. Real glossing passes (Cadence Gloss by comparison)
7. Layer-direction *enforcement* ([#230](https://github.com/freerouting/freerouting/issues/230))
8. BGA escape/fanout that terminates
9. Return-path / reference-plane awareness (nobody in FOSS has it; Quilter ships "Physics Rule Checks")
10. Impedance/stackup awareness
11. Placement assistance / placement-routing co-optimization (users build routers-as-oracles around it — [KiCadRoutingTools](https://github.com/drandyhaas/KiCadRoutingTools))
12. Back-drilling, blind/buried vias (industry-wide gap)

## (c) What AI-router startups do differently

- **Quilter** — RL-trained search + classical physics solvers; placement+routing solved jointly (every promising placement fully routed before scoring); thousands of candidates in hours; Physics Rule Checks *during* generation (return path, diff-pair coupling, impedance); explicit not-imitating-humans stance. Limits: 100–1,000 components, <20% pin density, through-hole vias only; fanout/length-matching still in development. [technology](https://www.quilter.ai/product/technology), [blog](https://www.quilter.ai/blog/pcb-autorouter-was-the-right-idea)
- **DeepPCB** — RL cloud router; independent test verdict mixed ("It sucks. Humans for the win." — EEVblog #1535). Cautionary for pure-RL claims.
- **Flux.ai** — LLM copilot + AI auto-layout marketing; struggles on simple circuits per user reports.
- **tscircuit** — TS router; the best public autorouter engineering postmortem: A* everywhere, spatial-hash over trees, avoid recursion/Monte Carlo, 13-stage pipeline with per-stage failure visualization, massive pre-computed caching. [13 things](https://blog.autorouting.com/p/13-things-i-would-have-told-myself-before-building-an-autorouter)
- **pcbautorouter.top** — GPU-parallel classical on same DSN pipeline; monetizes diff pairs + length matching (proof the gaps are worth paying for). OrthoRoute — FPGA pathfinder on GPU for backplanes; pre-alpha.
- **JITX** — programmable/generative design; LLMs best at generating design *code*, not geometry.

## (d) The 5 things that make autorouted boards look machine-made

1. **No placement awareness** — traces wander around bad component orientation; humans iterate placement↔routing together.
2. **Spaghetti geometry** — jogs, stubs, micro-detours, no bus/parallel discipline; humans route buses as coordinated parallel groups.
3. **Scattered vias without return-path logic** — each via breaks the reference plane; invisible to DRC, catastrophic for EMC.
4. **Completion-rate as the objective** — "a 100% connected board isn't necessarily a working board"; no concept of power short/thick, decoupling tight, planes unbroken.
5. **No cleanup of first-pass greed + no gloss** — "spends ages trying to fix those early placed traces"; no shove+gloss means no smooth 45° flow.

**Strategic summary:** fix correctness foundations first (DRC), then planes (#152) and high-speed features (#133/#716) — the two most-demanded and both paywalled at the only GPU competitor; performance is architectural (single-threaded, greedy-first-fix-late); the deepest differentiation is objective-function design (intent/physics scoring vs completion rate). Credibility comes from reproducible benchmarks (the PCBench corpus upstream already integrates).

---

## Cross-check addendum (2026-09-13): external Copilot analysis

User ran GitHub Copilot's repo analysis (top feature requests by engagement + code-level perf concerns) against ours. Reconciliation:

**Already covered (no action):**
- #152 copper pour / #133+#716 diff pairs & length tuning / #558 edge clearance — pain points #1/#7 above; M6/M7 milestones; #558 pinned in M1b Task 5.
- Performance items (TreeSet maze PQ, ShapeTree pointer-chasing, precalculatedTreeShapes churn, memory retention #684/PR#887) — §13 of the design doc names these classes with evidence and folds them into the arena/SoA + parallelism targets (3–10× + 2–6×). GC pauses moot in Rust.

**One overstated claim:** Copilot's "`getTreeEntries` O(n²) per routing pass" — the `LinkedList<SearchTreeInfo>` iterates search *trees* (a per-board small constant, one per clearance-class group), not items. Constant-factor fix, not a complexity-class one.

**Genuinely new items (banked):**
- **#718 net-ties / overlapping pads DRC** (6 comments) — DRC false-positive class; candidate scope add for M6 (epic-drc), alongside plane-aware checks.
- **#383 star-ground routing** (8 comments) — routing-topology constraint (dedicated net topology enforcement); M3+ router backlog, after the core maze engine parity lands.
- **#726 ETA / progress display** (6 comments) — M9 GUI (egui) backlog; trivial, good UX win.

None change the M0–M10 ordering; all three are post-parity improvements, exactly where the rewrite's headroom lives.

---

## DipTrace comparison addendum (2026-09-13)

User supplied an external DipTrace-vs-Freerouting feature/algorithm analysis (DipTrace = commercial shape-based standalone EDA; the best-regarded router among hobby/pro tools for years). Fact-checked against our port work and recon:

**Already in our research/design — with better sourcing than the analysis:**
- PathFinder negotiated congestion = our #1 recommended scheduler upgrade (state-of-the-art doc: McMurchie & Ebeling 1995 formulation `(base + history) × present-sharing`; baseline uses "linear pass-scaled costs + snapshot restore, prone to oscillation — NOT PathFinder history costs — key gap"). Design M-stage: global routing w/ PathFinder, current scheduler as fixture-gated fallback.
- Congestion-map global stage: design has it; a DATE 2023 global-routing paper *beat Freerouting* with exactly this.
- Fanout/escape improvements: design already itemizes congestion-map-aware pin ordering, BGA channel-aware escapes, dog-bone/via-in-pad/under-pad, ordered-escape planning.
- Coupled diff-pair + length matching: pose-based A* w/ Dubins heuristic + MSDTW (KRT Rust prior art); M7.
- Teardrops: in the gloss suite (`spread → centering → miter → teardrops`); KiCad 8 also does them natively.
- Multithreading: NCTU-GR 2.0 collision-aware net-level RRR is the cited technique; §13 has the data-layout side.
- Plane islanding: M6 exit criteria include island detection.
- The analysis's closing advice ("PathFinder + dedicated escape pass first, diff pairs later") independently confirms our M4→M6→M7 ordering.

**Factual errors in the analysis (do NOT absorb into backlog):**
- "Freerouting has no automated fanout" — FALSE: `autoroute.pipeline` runs fanout → batch → optimizer. The real gap is fanout *quality*, already itemized above.
- "Lacks class-to-class clearance matrices" — FALSE: `ClearanceMatrix` per-pair per-layer values are core DSN (`class_class`), ported in M1b Tasks 5+8 (T33); the parity digest literally carries the full matrix.
- "Cannot enforce via sizes by net class" — FALSE: `ViaRule`/`ViaInfo` per net class (`use_via`) ported in Task 8. Fair critique: *selection* intelligence when multiple vias allowed.
- "Treats pours as static keepouts" — HALF: `ConductionArea` + `contains_plane` stub+via mode is more than static; no dynamic re-flood is by DSN-workflow design (host re-pours on SES import). M6 adds in-router awareness.

**Genuinely new items (banked):**
1. **Parallel-run/crosstalk cost penalty** — path cost grows with parallel proximity+length on same/adjacent layers. Slots: M6 global-stage objective term.
2. **Max stub length DRC** — flags via/branch stubs that reflect signals. Slots: M6 epic-drc checks (cheap, high value).
3. **Phase tuning bumps** — intra-pair phase correction at corners. Slots: M7 diff-pair engine sub-feature.
4. **Layer direction bias strengthening** — penalize against-grain routing, allow short breakouts. Slots: M3/M4 cost weights (fixture-gated for zero regression).
5. **Controlled impedance width/spacing from stackup** — BLOCKED on input side: DSN carries no dielectric/Er data (KiCad doesn't export it); needs a settings/rules extension. Adjacent to the parked post-2.0 "impedance-driven stackup solving" line; pull earlier only if the input gap is solved.
6. **Thermal relief generation in-router** — workflow caveat: KiCad regenerates reliefs when re-pouring on SES import, so in-router spokes would double up unless flag-gated default-off. M6 consideration.
7. **Arc/curved + freeform routing** — fundamental extension beyond the 45° kernel port (epic-geometry is deliberately 45°-exact for parity). Post-M10 additive layer; DSN path syntax supports arcs so I/O would not block it.

---

## Tier-one router passes addendum (2026-09-13)

User supplied an analysis of commercial pipeline passes (Situs/Specctra/TopoR/Allegro). Cross-checked:

**Already covered:** hug/spread gloss (Specctra `spread → miter → recorner` ordering + Xpedition hug already in the design's gloss suite, M8); bus/bundle group routing (the "biggest looks-human win", M6 global + M8); TopoR itself (free-angle topological, "looks strange" — research doc); blind/buried vias parked post-2.0.

**Genuinely new (banked):**
1. **Memory/DDR daisy-chain pre-pass** — detect shared X/Y coordinate pins on adjacent components (DDR banks), strict U-pattern fanout+connect BEFORE the global router clutters the space. Adjacent to Xpedition "automated DDR tune" (already cited). Slots: M6 pipeline, after BGA escape, before global PathFinder.
2. **Clean pad entry pass** — force SMD pad exits parallel to the pad's long axis for a short run before turning (acid traps, tombstoning). Slots: M8 gloss suite (manufacturability member).
3. **Hug as a MID-ROUTING move** — new nuance vs our end-gloss framing: run hug mid-pipeline when completion stalls to consolidate channels for failing nets; spread stays end-of-pipeline. Slots: M6/M8 pipeline placement.
4. **Multi-candidate parallel branching (portfolio search)** — 4-8 parallel global-router instances with perturbed cost weights, score all, import best. Constraints: determinism is non-negotiable (fixed per-thread perturbations, deterministic winner selection — the parity harness depends on reproducibility); composes with NCTU-GR collision-aware threading but multiplies the parallel budget. Attribution to "TopoR/Quilter method" is loose (TopoR is topological; Quilter is RL) — treat as algorithm-portfolio search. Slots: M6+ once parallel global stage is stable.
5. **Dynamic pin swapping** — swap logically-equivalent destinations to uncross the ratsnest (resistor networks, FPGA banks); requires a back-annotation swap-list output for KiCad. Verified: Freerouting's 101-keyword DSN table has NO swap keyword — the feature is absent from the entire pipeline, not just the router. Needs input-side equivalence data (DSN `pin_swap` rules exist in Specctra; KiCad export support unknown). Slots: late M7+ / post-2.0, input-gated like impedance.
6. **Via-type-per-layer-pair legality maps + stacked/staggered microvia DRC** — HDI constraints; refinements of the parked blind/buried-via line (rule model can land in M6/M7 constraint work before the vias themselves do).

**Corrections:** bundle routing gives *similar* lengths, not target-matched — M7 tuning still required ("guarantees length matching out of the gate" overstated); "multi-threaded C++" — engine is Rust by design decision (§Language evidence); proprietary Situs/Allegro pass details are directional, not verifiable.
