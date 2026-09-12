# Autorouter State of the Art (September 2026)

**Research companion to:** `docs/superpowers/specs/2026-09-11-epicrouter-rust-rewrite-design.md`

Local grounding: this fork (post-2026-refactor Freerouting) confirms the engine layout: `autoroute/maze/MazeExpansionEngine.java`, `autoroute/expansion/*` (rooms/doors), `board/searchtree/ShapeSearchTree.java` (tile-based hierarchical index, per-angle trees, per-clearance-class compensation), `geometry.planar.IntOctagon`, `board.optimize` (pull-tight/shove/via-optimizer).

## 1. Algorithm families

| Algorithm | One-line description | Source |
|---|---|---|
| Lee maze router (1961) | BFS wavefront over a grid; complete but O(cells) memory/time | [DeepPCB history](https://deeppcb.ai/the-60-year-routing-problem-nobody-solved/), [TinyComputers](https://tinycomputers.io/posts/the-mathematics-of-pcb-trace-routing.html) |
| Mikami-Tabuchi (1968) / Hightower (1969) line search | Probe escape lines instead of flooding cells; fast, memory-light; Hightower incomplete | [Hightower DAC'69](https://www.semanticscholar.org/paper/a-solution-to-line-routing-problems-on-the-plane-Hightower/653807aa3b96a5a5156e2d0c95882321ab2426bab) |
| Heyns line expansion (1980) | Expands lines into regions to fix escape-line explosion | [ACM](https://dl.acm.org/doi/pdf/10.1145/800139.804534) |
| A* (1968) | Best-first with admissible heuristic; Freerouting's actual core | [TinyComputers](https://tinycomputers.io/posts/the-mathematics-of-pcb-trace-routing.html) |
| Jump Point Search (2011) / JPS4 (2025) | Symmetry-breaking pruning for uniform-cost grid A*; order-of-magnitude node reductions | [JPS](https://harabor.net/data/papers/harabor-grastien-socs12.pdf), [JPS4](https://arxiv.org/html/2501.14816v1) |
| 4-geometry maze routing (2005) | Maze routing over 4/8-geometry (octilinear) grids — academic analog of IntOctagon | [ACM](https://dl.acm.org/doi/10.1145/1044111.1044118) |
| Gridless area routing (Cooper & Chyan 1989 → Specctra) | Shape-based routing on continuous geometry; industry-standard lineage | [Specctra](https://en.wikipedia.org/wiki/Specctra) |
| Multi-layer gridless with wire planning (Cong et al., ISPD 2000) | Plan wires globally, realize gridless detail | [ACM](https://dl.acm.org/doi/10.1145/332357.332367) |
| Rubber-band/topological routing (SURF 1991; TopoR) | Route as rubber-band sketch, snap taut; free-angle topological routers | [DAC'91](https://dl.acm.org/doi/10.1145/127601.127622), [TopoR](https://t.eremex.com/) |
| Expansion-room/free-space routing (Freerouting; Specctra lineage) | Lazily partition free space into convex rooms connected by doors; A* over room graph | local source; [TinyComputers](https://tinycomputers.io/posts/the-mathematics-of-pcb-trace-routing.html) |
| Global + detail decomposition (IC-style) | Coarse tile-path planning for all nets, then detail within guides | [TritonRoute](https://ieeexplore.ieee.org/document/8587728/), [Dr. CU](https://baloneymath.github.io/files/ICCAD18_dr.pdf), [ISPD'19](https://dl.acm.org/doi/10.1145/3299902.3311067) |
| Two-stage PCB routing, polygon partition + MCTS (He et al., DATE 2023) | Pad-focused polygon partition; MCTS global routing, A* detail; **beat Freerouting** on success rate/wirelength | [PDF](https://past.date-conference.com/proceedings-archive/2023/DATA/603.pdf), [Iowa State dissertation](https://dr.lib.iastate.edu/server/api/core/bitstreams/baa06fe6-541d-4f4a-888d-94f3083cd518/content) |
| Rip-up and reroute (Dees & Karger 1982) | Remove and re-route blocking connections with escalating costs | cited in [PCBWorld](https://arxiv.org/html/2607.05915v2) |
| PathFinder negotiated congestion (McMurchie & Ebeling 1995) | Node cost = (base + history) × present-sharing; history accumulates so contested resources become expensive until convergence | [original](http://www.sttitt.ece.ufl.edu/courses/eel4720_5721/reading/pathfinder.pdf) (use gstitt mirror if stale), [explainer](https://stackoverflow.com/questions/17494396/can-anyone-explain-pathfinder-algorithm-used-in-fpga-routing) |
| Partial rip-up PathFinder (Zha & Li) | Rip only congested sub-paths | [Semantic Scholar](https://www.semanticscholar.org/paper/Revisiting-PathFinder-Routing-Algorithm-Zha-Li/04fd9d7b7514dda3ccdfd3433059ff7e81988b2f) |
| Strategic rip-up (GLSVLSI 2025) | Rip *groups* of connections | [ACM](https://dl.acm.org/doi/full/10.1145/3716368.3735162) |
| Collision-aware parallel RRR (NCTU-GR 2.0, TCAD 2013) | Net-level multithreaded routing with race detection + bounded-length maze routing | [paper](https://www.semanticscholar.org/paper/NCTU-GR-2.0%253A-Multithreaded-Collision-Aware-Global-Liu-Kao/20c61c0941b57e186fa942aa53cb969912b69f86) |
| Push-and-shove interactive routing (KiCad PNS) | Displace blocking traces/vias perpendicular with cascade limits | [KiCad discussion](https://gitlab.com/kicad/code/kicad/-/issues/5448) |
| Pattern routing (L/Z/U) | Fixed 1–2 bend routes for easy nets before maze search; Specctra "pattern" pass; GPU hybrid won ISPD'24 | [Specctra guide](https://www.scribd.com/document/403446730/Cadence-Spec-c-Tra-Auto-Router), [ISPD'24](https://liangrj2014.github.io/ISPD24_contest/) |
| Escape routing (B-Escape 2010; Ozdal 2008; Lin 2025) | Route all pins of a dense component to its boundary simultaneously (LP/matching formulations) | [survey](https://dl.acm.org/doi/pdf/10.1145/3394885.3431568) |
| Unified constraint-driven full-board PCB router (Lin et al., ASP-DAC 2021) | One algorithm for topology/length/clearance incl. diff pairs | [free PDF](https://dl.acm.org/doi/pdf/10.1145/3394885.3431568) |
| CERT curved escape routing (2026) | Multi-layer BGA escape with curved (arc) traces | [ACM](https://dl.acm.org/doi/full/10.1145/3787109.3815238) |

## 2. What commercial routers do that open source doesn't

- **Cadence Specctra/Allegro** — gridless shape-based; documented pass suite `fanout → clean → pattern → route (25/45/90) → spread → optimize → miter → recorner → protect`. [guide](https://www.scribd.com/document/403446730/Cadence-Spec-c-Tra-Auto-Router), [release notes](https://community.cadence.com/cfs-file/__key/telligent-evolution-components-attachments/00-27-01-00-00-00-51-96/specctraWN.pdf)
- **Altium Situs** — topological (free-space flow corridors → grid-based routing in corridor); largely single-threaded, little improved since ~2008. [docs](https://www.altium.com/documentation/altium-designer/pcb/routing/situs-topological-autorouter), [critique](https://www.ninedotconnects.com/back9-autorouter-in-ad)
- **Siemens Xpedition AutoActive** — hug routing (Multiple Hug Trace Plus), sketch routing, gloss/centering, automatic unused-pad removal, automated DDR tune. [blog](https://blogs.sw.siemens.com/electronic-systems-design/2023/07/19/pcb-routing/), [routing aids](https://resources.sw.siemens.com/en-US/product-demo-interactive-pcb-routing-aids/)
- **Pulsonix** — multipass cost-based conflict reduction + explicit **bus routing** (nets following similar paths routed as a group). [Advanced Router](https://pulsonix.com/advancedautorouter.asp), [bus behavior](https://pulsonix.com/docs/14.0/Advanced-Router/idh_autorouter_results)
- **TopoR (Eremex)** — commercial free-angle topological router; near-100% completion, shorter traces, but "looks strange" vs 45° norms. [site](https://t.eremex.com/), [review](https://wp.josh.com/2017/10/23/adventures-in-autorouting/)
- Multithreading/GPU: almost nothing disclosed commercially; the real work is academic (below).

## 3. Academic / ML (2023–2026)

- **PCBWorld (LG AI Research, 2026)** — wraps KiCad C++ engine as Gym env; benchmarks Freerouting, OrthoRoute, RL (PPO/GRPO/A2C/Sable), LLM agents on 20k synthetic + 679 real boards. **Freerouting remains the strongest classical baseline and wins on large real boards**; engine-grounded PPO matches it only on small boards; grid-action RL collapses with resolution; LLMs score 0.00 on large boards. [arXiv](https://arxiv.org/html/2607.05915v2)
- **DeepPCB (InstaDeep)** — deep-RL cloud router; EEVblog's independent test was mixed. [site](https://deeppcb.ai/), [EEVblog test](https://www.eevblog.com/forum/blog/eevblog-1535-deeppcb-ai-autorouting-tested!/)
- **Quilter AI** — RL search + classical physics solvers; placement+routing solved jointly; "Physics Rule Checks" during generation; BGA fanout and length matching still in development (admitted). [blog](https://www.quilter.ai/blog/pcb-autorouter-was-the-right-idea)
- **OrthoRoute** — GPU Manhattan lattice + PathFinder negotiated congestion; strong on huge regular backplanes, poor on general boards. [GitHub](https://github.com/bbenchoff/OrthoRoute)
- **FanoutNet (AAAI 2023)** — deep-RL fanout/escape. [AAAI](https://ojs.aaai.org/index.php/AAAI/article/view/26030)
- **Congestion prediction** — RouteNet (ICCAD 2018); ML congestion forecasting pre-loads PathFinder history costs (FCCM 2022). [RouteNet](https://dl.acm.org/doi/10.1145/3240765.3240843), [forecasting](https://ieeexplore.ieee.org/document/9900091/)
- **ISPD contest winners** — TritonRoute (intra-layer parallel detail routing), Dr. CU (multithreaded), InstantGR (DAC 2024 open GPU global router), GTA (ICCAD 2025), Hippo/PKU (ISPD'25 GPU hybrid pattern routing). [InstantGR](https://github.com/cuhk-eda/InstantGR), [ISPD'25](https://dl.acm.org/doi/pdf/10.1145/3698364.3715706)
- **KiCadRoutingTools (KRT, 2025–26)** — open-source Python + **Rust A* core (~10× faster)**; octilinear; crossing-aware net ordering (MPS), progressive N+1 blocker rip-up, BGA/QFN fanout with under-pad escape, **diff pairs via pose-based A* with Dubins heuristic**, trombone meanders, Voronoi plane partitioning; no push-and-shove, no global stage. Directly relevant prior art for the Rust port. [GitHub](https://github.com/drandyhaas/KiCadRoutingTools)
- Benchmarks: **PCBench** (164+ real boards; upstream repo integrates 1,182), PCBWorld-Bench, PCB-Bench.

## 4. Quality/aesthetic techniques

| Technique | State of the art | Source |
|---|---|---|
| Bus/hug routing | Xpedition hug; Pulsonix bus detection; Specctra `spread` pass | links above |
| Diff pairs | ASP-DAC'21 unified constraint router; KRT pose-based A* + Dubins | [ASP-DAC'21](https://dl.acm.org/doi/pdf/10.1145/3394885.3431568) |
| Length matching | Classic algorithm (1998); KiCad PNS meander tuner; Altium switchback vs serpentine; **Freerouting has none — issue #716** | [1998](https://www.researchgate.net/publication/3225949_A_Length-Matching_Routing_Algorithm_for_High-Performance_Printed_Circuit_Boards), [#716](https://github.com/freerouting/freerouting/issues/716) |
| Obstacle-aware length matching (2024) | MSDTW: collapse pair to median trace, DTW, re-expand | [arXiv](https://arxiv.org/html/2407.19195v1) |
| Miter/recorner/gloss | Specctra `miter`/`recorner`; Xpedition gloss + centering + unused-pad removal | links above |
| Teardrops | KiCad 8+ native generator | [KiCad](https://gitlab.com/kicad/code/kicad/-/issues/21246) |
| BGA fanout | Dog-bone, via-in-pad, HDI stacking; GA escape (2026); CERT; FanoutNet; KRT under-pad | [survey](https://dl.acm.org/doi/pdf/10.1145/3394885.3431568) |

## 5. Performance techniques

- Spatial indexing: R-trees, interval trees; **OpenDRC** (DAC 2023) and **HeteroDRC** (ICCAD 2023) — open-source incremental CPU/GPU DRC engines with hierarchical spatial structures; PDRC (DAC 2024) hierarchical interval lists for non-Manhattan DRC. [OpenDRC](https://www.cse.cuhk.edu.hk/~byu/papers/C172-DAC2023-OpenDRC.pdf), [HeteroDRC](http://www.cse.cuhk.edu.hk/~byu/papers/C188-ICCAD2023-HeteroDRC.pdf)
- JPS symmetry breaking; bounded-length maze routing (BLMR in NCTU-GR 2.0) caps A* cost with little quality loss; Minkowski-sum clearance inflation (Freerouting already does this).
- Parallelism that works: net-level collision-aware RRR (NCTU-GR 2.0, 225 citations), intra-layer parallel detail (TritonRoute), GR-guided multithreaded detail (Dr. CU), parallel negotiation (SPRoute), GPU global routing (InstantGR). Racing on shared resources is the hazard; collision detection/rollback is the remedy.
- Memory layout is the real language win: KRT's Rust A* core is 10× over Python; naive per-node malloc can *lose* to Java's GC — arenas/pools/flat arrays are the requirement, not "Rust" per se.

## 6. What Freerouting (baseline) already uses

A* maze search over shape-based expansion rooms with doors (lazy convex free-space rooms + clearance halos; 90/45/free modes); IntOctagon 45° geometry; rip-up with linearly pass-scaled costs + coarse snapshot-restore backtracking (NOT PathFinder history costs — key gap); net order = DSN file order (distance sort removed in v2.3 for convergence); shove (cascade 20 traces/5 vias) + pull-tight + via optimizer (multithreaded candidate evaluation only); Minkowski compensation in per-clearance-class trees; MinAreaTree tile index; pours as static obstacles with stub+via plane mode (known clearance bugs).

**Not present:** global/detail decomposition, pattern routing, negotiated congestion, bus/hug routing, diff pairs, length matching, teardrops, net-ordering intelligence, parallel maze search, GPU, ML, dynamic pour re-flood.

## 7. Top 10 improvements (ranked, all gated by the parity harness)

1. **PathFinder negotiated congestion** as the rip-up scheduler — the most validated convergence mechanism in the field; run as alternative scheduler mode with the current scheduler as fixture-gated fallback.
2. **Global routing stage + pattern routing** (DATE 2023 formulation beat Freerouting; makes net ordering a planned decision).
3. **Bus/hug/group routing** — the biggest "looks human" win (group detection by ratsnest topological similarity → shared corridor → parallel insertion, uniform spacing).
4. **Diff-pair + length-matching engine** (pose-based A* + Dubins heuristic; MSDTW; post-route meander insertion). Nothing else moves high-speed boards from unusable to usable this much.
5. **BGA/dense-fanout upgrade** (ordered escape planning, dog-bone, via-in-pad, under-pad escape).
6. **Gloss pass suite** (`spread → centering → miter → teardrops`) — cheap post-completion aesthetics, each toggleable per fixture for zero-regression enforcement.
7. **Parallelize the maze search** (collision-aware net-level RRR) + arena/SoA memory layout.
8. **Intelligent net ordering + rip-up prioritization** (crossing-aware MPS; optional ML congestion forecasting off by default).
9. **Via-count + layer-assignment optimization** in search cost and post-pass (via chains, unused pads, via-minimizing Steiner topologies).
10. **Incremental DRC in the routing loop + dynamic copper pour modeling** (re-flood after insertion, thermal relief) — removes the structural ceiling; highest effort.

**Cross-cutting:** classical algorithms are the path; RL/LLM not worth importing today (PCBWorld 2026).
