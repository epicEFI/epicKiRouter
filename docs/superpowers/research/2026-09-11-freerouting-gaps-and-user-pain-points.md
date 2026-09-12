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
