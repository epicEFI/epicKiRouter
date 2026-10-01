# SEAM — the Java surface beyond `autoroute/` (M3-T3 audit)

The T4+ porting contract: the Java artifacts OUTSIDE `app.freerouting.autoroute.*`
that the detail-routing pipeline calls into, AS ENUMERATED BY THIS AUDIT — a
working inventory, not a completeness proof; rows are appended whenever a
consumer surfaces one missed here (the M3-T3 quality round added the
trace-check/shove band below). Each row carries file:line anchors, line counts,
and a disposition (`already-ported` | `NEW port` | `stub` | `skip — reason`).
Java sources
are the frozen oracle tree (`origin/master` grafted at `e7f9bdf1`); counts are
`wc -l` at audit time and are load-bearing (they decide the T10 split, below).
Method ranges are member-declaration boundaries.

Already-ported Rust anchors: `epic-board::trace_ops` (insert/split/combine/
normalize/remove_if_cycle/get_trace_tail — M2 T11-T13), `epic-board::contacts`
(contact + connected/unconnected sets — M2 T10, M3-T3), `epic-board::normalize_all`
(M2 T13), ChangedArea bookkeeping in `trace_ops` (M2).

## board/facade

`RoutingBoard.java` (1,442 lines) and `BasicBoard.java` (1,488 lines) are the
two seams the routing stages live on. `RoutingBoard` composes
`RoutingBoardOperations` (121 lines, thin delegator — shaped in M2),
`RoutingBoardSearchFacade` (218), `RoutingBoardUndoFacade` (105);
`BoardItemRepository`/`BoardSnapshotManager` are M2 done;
`BoardConnectivityQueries.java:23-37` (`getConnectableItems`) is already
ported as `contacts::connectable_items` + `Board::get_connectable_items`
(M3-T3, pinned).

| Java surface | Anchor | Lines | Disposition |
|---|---|---|---|
| `RoutingBoard.optChangedArea` (2 overloads) | `RoutingBoard.java:151-197` | 47 | already-ported (ChangedArea marking/joining, M2); the overload bodies delegate to `RoutingBoardOperations.optChangedArea` — that DRIVER is the optimizer pass, not detail routing |
| `RoutingBoard.checkTraceSegment` (2 overloads) | `RoutingBoard.java:198-237` | 40 | NEW port — the per-segment legality probe the maze shover and `FoundConnectionInserter` call (T4-T9); the facade overloads delegate to `RoutingBoardSearchFacade.checkTraceSegment` (the search facade, NOT RoutingBoardOperations), port with that consumer |
| `RoutingBoard.pickNearestRoutingItem` | `RoutingBoard.java:303-311` | 9 | skip — GUI/interactive pick helper, no headless routing consumer |
| `RoutingBoard.forcedVia` | `RoutingBoard.java:312-360` | 49 | NEW port — manual via-drop entry; pairs with `ForcedViaInserter` |
| `RoutingBoard.insertForcedTraceSegment` | `RoutingBoard.java:361-407` | 47 | NEW port — manual single-segment insert (check + insert + pull-tight) |
| `RoutingBoard.checkForcedTracePolyline` | `RoutingBoard.java:408-455` | 48 | NEW port — manual/forced-route legality check (shove-fail probe) |
| `RoutingBoard.insertForcedTracePolyline` | `RoutingBoard.java:456-881` | 426 | NEW port — the forced-insert body: per-segment check shapes, shove integration, pull-tight on success. Largest single method in the facade |
| `RoutingBoard.initAutoroute` | `RoutingBoard.java:882-899` | 18 | NEW port — lands WITH the `AutorouteEngine` task (thin: engine ctor + `initConnection`) |
| `RoutingBoard.connectToTrace` | `RoutingBoard.java:1116-1175` | 60 | NEW port — attach a route end to an existing same-net trace/pin |
| `RoutingBoard.containsTraceTails` | `RoutingBoard.java:1176-1192` | 17 | NEW port — the predicate behind the tail sweep; pairs with the row below |
| `RoutingBoard.removeTraceTails` | `RoutingBoard.java:1193-1240` | 48 | NEW port — the per-net tail sweep; tail DETECTION is already in `trace_ops::get_trace_tail` (`Trace.java`/`Via.java` overrides) |
| `RoutingBoard` autoroute-data + shove-failing band | `RoutingBoard.java:1241-1400` | ~160 | NEW port (T10) — `clearAllItemTemporaryAutorouteData` `:1241`, `changeConductionIsObstacle` `:1253`, `reduceNetsOfRouteItems` `:1284`, the shove-failing obstacle/layer channel `:1359-1383` (the MazeTraceShover failure report), autoroute-database flags `:1384-1400` |
| `BasicBoard.insertVia` | `BasicBoard.java:269-310` | 42 | NEW port — via insertion + tree insert + undo bookkeeping |
| `BasicBoard.insertEscapeVia` | `BasicBoard.java:311-336` | 26 | NEW port — insertVia's obstacle-arm variant |
| `BasicBoard.normalizeTraces(net)` | `BasicBoard.java:710-798` | 89 | NEW port — per-net normalize; thin over the M2 T13 normalize core (`normalize_all.rs`) |
| `BasicBoard.checkTraceShape` | `BasicBoard.java:989-1053` | 65 | NEW port — the check-shape core beneath the forced-insert and shove families |
| `BasicBoard.checkPolylineTrace` | `BasicBoard.java:1054-1077` | 24 | NEW port — polyline wrapper over `checkTraceShape` |
| `BasicBoard.clearanceValue` | `BasicBoard.java:1111-1118` | 8 | NEW port — the matrix read the check families call |
| `BasicBoard.getMinTraceHalfWidth` | `BasicBoard.java:1124-1128` | 5 | already-ported — `Board::min_trace_half_width` (`epic-board/src/board.rs:383`, M2; the reader's pin-edge fallback consumes it) |
| `BasicBoard` graphics-update-box family (`reset`/`get`/`joinGraphicsUpdateBox`) | `BasicBoard.java:1143-1160` | 18 | stub — GUI repaint bookkeeping; the headless port is a no-op |
| `BasicBoard` insert/normalize core | `:169-200`, `:405-421`, split/combine/normalize family | — | already-ported (M2 T11-T13) |

Skip-with-reason in this file: `autoroute`/`fanout` drivers `:900-1115`
(engine-side, T12 stage orchestration), undo/statistics/deep-copy delegation
`:1401-1441` (facade plumbing already shaped in M2).

## board/actions (manual-route helpers consumed by the forced path)

| Java surface | Anchor | Lines | Disposition |
|---|---|---|---|
| `ForcedPadRouter` (whole) | `ForcedPadRouter.java` | 500 | NEW port — ctor `:38`, `calcCheckShapeForFromSide` `:42`, `inFrontOfPad` `:57-220`, `checkForcedPad` `:221-500` |
| `ForcedViaInserter` (whole, static) | `ForcedViaInserter.java` | 462 | NEW port — `checkLayer` `:30-130`, `check` `:131-248`, `insert` `:249-362`, `holeCheckShape` `:363`, `calculateFromSide` `:377` |

Both feed `insertForcedTracePolyline`/`forcedVia` (`RoutingBoard.java:312-360`)
— port them as one unit with the forced path, not with the maze stage.

## board/optimize

| Java surface | Anchor | Lines | Disposition |
|---|---|---|---|
| `TraceShover` (whole) | `TraceShover.java` | **875** | NEW port (T10). static `check` `:57-230`, instance `check` `:231-416`, `insert` `:417-610`, `springOver` `:611-826`, `springOverObstacles` `:827-875` |
| `TraceTightener` (dispatcher) | `TraceTightener.java` | 547 | NEW port (T10, minimal pull-tight) |
| `TraceTightener45` | `TraceTightener45.java` | 674 | NEW port (T10) |
| `TraceTightener90` | `TraceTightener90.java` | 169 | NEW port (T10) |
| `TraceTightenerAnyAngle` | `TraceTightenerAnyAngle.java` | 1004 | NEW port — deferred behind the 45/90 pair; only any-angle designs reach it |
| `ViaOptimizer` | `ViaOptimizer.java` | 733 | ~~skip — optimizer-stage pass (post-detail-routing gloss), not the T4-T10 seam~~ **SUPERSEDED M4-T5** — landed as `epic-board::trace_tightener::via_optimizer` behind the `opt_changed_area` via arm (`ItemKindTag::Via`, gated on trace costs, recursion budget 10; the T17a-2 POINTS-overload forward note honored at all four `checkTraceSegment` sites; plane face = loud M6 stub). Row kept for the decision history; current truth lives in the `via_optimizer.rs` module docs |

## searchtree + expansion

| Java surface | Anchor | Lines | Disposition |
|---|---|---|---|
| `ShapeSearchTree.completeShape` (base/any-angle arm) | `ShapeSearchTree.java:580-693` | 114 | already-ported (M3-T4) — `epic_index::complete_shape::complete_shape_generic`; `restrainShape` `:701-811` and `divideLargeRoom` `:1095-1118` are its private helpers, ported in the same module |
| `ShapeSearchTree45Degree.completeShape` (45-degree override) | `ShapeSearchTree45Degree.java:96-281` | 186 | already-ported (M3-T4) — `complete_shape_fortyfive_degree`; its `restrainShape` `:305-418` + integer helpers `:38-86`, `:424-486` included. (The brief's open question answered: the 45-degree override DOES have a restrainShape analogue.) |
| `ShapeSearchTree90Degree.completeShape` (90-degree override) | `ShapeSearchTree90Degree.java:39+` | — | skip (M3-T4) — the 90-degree tree has its OWN completeShape/restrainShape overrides (`:39`/`:198`), NOT the base arm; Tier A is all-45-degree so the dispatcher debug-asserts it unreachable. Port only if a 90-degree fixture ever appears |
| `SortedRoomNeighbours` (whole) | `autoroute/expansion/SortedRoomNeighbours.java` | 807 | already-ported (M3-T4) — `epic_router::expansion::neighbours`: dispatch `complete` `:65-72` + `selectCalculationMode` `:80-88`, `calculate` `:95-136`, `calculateNeighbours` `:187-329` (the (objectId, shapeIndex) v1.9-parity pre-sort `:204-213`), `insertDoorOk` `:332-389`, `tryRemoveEdge` `:415-507`, `calculateNewIncompleteRooms` `:510-659`, inner class `SortedRoomNeighbour` `:665-806` (compareTo `:720-762`, firstCorner/lastCorner `:765-805`) |
| `Sorted45DegreeRoomNeighbours` (whole) | `autoroute/expansion/Sorted45DegreeRoomNeighbours.java` | 982 | already-ported (M3-T4) — `neighbours` 45-degree arm: `calculate` `:45-79`, `calculateNeighbours` `:85-172` (same pre-sort), `removeNotTouchingBorderLines` `:174-243`, `tryRemoveEdgeLine` `:316-431`, `calculateEdgeIncompleteRoomsOfObstacleExpansionRoom` `:255-310`, `calculateNewIncompleteRoomsForObstacleExpansionRoom` `:475-609`, `calculateNewIncompleteRooms` `:611-797`, inner class `:803-981` (first/lastTouchingSide detection `:825-916`, counterclockwise compareTo `:923-980` with REVERSED corner-sign on sides 4-7) |

Note: the two `SortedRoomNeighbours` classes live in `autoroute/expansion/`
but are listed here because they are the SEARCHTREE-side half of the maze
seam — they belong with the `completeShape` port, not with the pass driver.
The engine seam (`AutorouteEngine.addIncompleteExpansionRoom`,
`generateRoomIdNo`, `removeAllDoors`, `TRACE_WIDTH_TOLERANCE = 2`) is a
Rust trait/callback boundary (`expansion::NeighbourEngine`), NOT a ported
engine class; the maze state (MazeSearchElement sections, MazeSearchEngine)
lands with T6.

## Dependency shape of the NEW-port cluster

* The maze stage (T4-T9) consumes: `completeShape` (through the expansion-room
  creation path), `SortedRoomNeighbours`, `initAutoroute`, `insertVia`/
  `insertEscapeVia`, `connectToTrace`, `containsTraceTails` + `removeTraceTails`,
  `normalizeTraces(net)`, and — through the shove/insert path —
  `checkTraceSegment` plus the `:1241-1400` shove-failing band. Of these only
  the trace_ops/contacts/query core is already-ported; the REST of the list is
  NEW port surface (the earlier draft's "everything routes through the
  already-ported surface" was falsified in the M3-T3 quality review — the
  trace-check/shove rows above had been missed).
* The manual-route cluster (`forcedVia`, `insertForcedTraceSegment`,
  `checkForcedTracePolyline`, `insertForcedTracePolyline`,
  `ForcedPadRouter`, `ForcedViaInserter`) forms one unit totalling
  500 + 462 + 49 + 47 + 48 + 426 = 1,532 lines; port it after the shove
  engine exists (it calls TraceShover), before/with interactive routing.
* `TraceTightenerAnyAngle` (1,004) is only reachable from any-angle designs;
  the 45/90 pair plus the dispatcher (1,390) covers the default designs.

## T10 split decision

**TraceShover.java = 875 lines (verbatim `wc -l`).** The split threshold was
">2,500 lines ⇒ split T10a/T10b". 875 is far below it — and the corrected
count still is: adding the RoutingBoard `:1241-1400` band the shove engine
consumes (~160) gives ~1,035 lines of port-bearing shove logic. **No split —
T10 stays one task** (TraceShover + the minimal pull-tight trio
`TraceTightener`/`45`/`90`, 2,390 lines combined, with
`TraceTightenerAnyAngle` deferred).

## T5 drill seams (appended by M3-T5)

The drill port (`autoroute/drill/` + `MazeExpansionEngine.expandToOtherLayers`)
exposes three Rust traits whose production backing is OUT of scope for T5.
The oracle-backed pins (`epic-router/src/drill/pins.rs`) run all three as
stubs whose behavior is FORCED by the capture rows (see the pins module doc).

| Rust seam trait | Java backing | Anchor | Lands with |
|---|---|---|---|
| `drill::ViaLayerChecker` (`check_layer`) | `ForcedViaInserter.checkLayer` — the deep per-layer via legality probe (radius/clearance/keepout/attach), static | `ForcedViaInserter.java:30-130`, called from `MazeExpansionEngine.checkLayerWithAnyMatchingVia` at `MazeExpansionEngine.java:391` | T10 (ForcedViaInserter, NEW port row above) |
| `drill::DestinationDistance` (`calculate`) | `DestinationDistance` — CONCRETE class in the current tree (NOT a v1.9-style interface): lower-bound heuristic over the joined destination boxes (`join`/`calculate(IntBox, int)`); an empty join set returns `Integer.MAX_VALUE` — exactly the capture's targetless 2147483647.0 | `autoroute/maze/DestinationDistance.java` (391 lines, whole class); constructed `MazeSearchEngine.java:126-127`, boxes joined `:983-991`, call sites `MazeExpansionEngine.java:87/128/358` | T8 (destination machinery) |
| `drill::expand_to_other_layers` emit callback (`DrillMazeListElement`) | the producer side inserts into `MazeSearchEngine.mazeExpansionList` (the `TreeSet<MazeListElement>` with its comparator — the consumer owns ordering/dedup) | field `MazeSearchEngine.java:52`, constructed `:84`; the SINGLE emit site is `MazeExpansionEngine.java:373`, SHARED by the ripped and free-space branches (`roomRipped` is only an element flag, `:370`); the consumer-side `add` map is `MazeSearchEngine.java:547/964/1079` | T6 (MazeSearchEngine core) |

Note: `MazeExpansionEngine.checkLayerWithAnyMatchingVia` itself
(`MazeExpansionEngine.java:377-414`) IS T5 scope — ported as
`drill::check_layer_with_any_matching_via` with the rule-via span filter and
short-circuit pinned; only the per-layer `checkLayer` body behind it is the
T10 seam.

Also T5 (not a stub seam, an engine callback): the drill completion path
`ExpansionDrill.calculateExpansionRooms` → `AutorouteEngine.completeExpansionRoom`
(`AutorouteEngine.java:418-522`) rides the existing
`expansion::NeighbourEngine`/`complete` glue (M3-T4) — with the
first-candidate/recalc split pinned by the companion probe oracle
`rust/harness/oracle/DrillSpikeProbe.java` (`/tmp/drill_probe.rows`).

## T6 maze-core status (appended by M3-T6)

The maze CORE landed: `epic-router/src/maze/` — `list_element.rs` (the front:
`TreeSet`-over-comparator semantics on a `BTreeSet`, see the deviation window
below), `search_engine.rs` (`findConnection`/`occupyNextElement`/`init`/
`expandToRoomDoors`/`expandToDoor(Section)`, Java
`MazeSearchEngine.java:300-1103`), `expansion_engine.rs` (the drill dispatch
`MazeExpansionEngine.java:31-236`), `completion.rs` (the production
`complete_null_shape_room` the `DrillEngine` trait default delegates to),
`pins.rs` (the jar-capture replay). The T5 emit-callback row above is CLOSED:
the producer inserts through `maze::Front` (the consumer owns ordering/dedup).

| Seam / semantics point | Status | Detail |
|---|---|---|
| Front dedup + ordering | ported, pinned | Java `TreeSet<MazeListElement>` comparator `sortingValue → expansionValue → door.getId() → sectionNoOfDoor`, ALL with `<`/`>` (`MazeListElement.java:80-113`): NaN falls through the value keys, -0.0 ties +0.0, full tie = the `add` is a no-op (dedup). Rust: `BTreeSet<FrontOrder>` + `compare` helper with `partial_cmp` + fall-through (`total_cmp` deliberately NOT used); T1-T7 pins + 6 comparator mutants. |
| **Live-id deviation window** | documented deviation, ordering-proven (page-vs-page) | Java's comparator reads `door.getId()` LIVE on every comparison; `DrillPage.getId()` = `31*shape.getId() + netNumber` (`DrillPage.java:190-193`) embeds the page's MUTABLE drill-net (-1 until its first `getDrills`, the net after; bug-128). The Rust front freezes the id AS OF THE ADD (a `BTreeSet` key must be stable). The ordering-invariance proof covers PAGE-VS-PAGE pairs only — the net term cancels there, so live-vs-frozen can only disagree when ΔshapeId = 0, i.e. the SAME page, where both reads are equal and the comparator falls through to the section anyway. A page-vs-NONPAGE exact-f64 tie CAN resolve differently; that window is Java's comparator-invalidation territory (`getId` mutates on first `getDrills` memoization while the element sits in the `TreeSet` — ordering unspecified there), so freeze-at-add cannot diverge from DEFINED Java semantics. The T6 capture exercised exactly this mutation (DRILL head k=10: frozen -765662255 vs live -765662253, Δ = +2·net after k=9 drilled the page); the pin layer resolves the LIVE id for comparison (`assert_head`'s `live_door_id`). |
| Ripup stubs (T7) | stub, live decision sites | `MazeRipupResolver.checkLeavingRippedItem` / `checkRipup` (`MazeSearchEngine.java:506`/`:519`) — stubbed "cannot leave through a small door" / "free, costs 0, rippable"; identical to Java while no ripped items exist (`ripupAllowed` false in the T6 fixtures). The `shoveTraceRoom` stub (`:1130-1201`) answers "not shoved": with `ripupCosts == 0` Java's flow continues identically past `if (!shoved)`. |
| Distance slot (T8) | CLOSED by M3-T8 | `maze::destination_distance::DestinationDistance` is the production impl of the `drill::DestinationDistance` trait; the `TargetDistance` mirror is DELETED (see the T8 section below). The targetless arm (`2147483647.0`) remains — now as the production class's own `boxIsEmpty` sentinel (`DestinationDistance.java:123-124`), not a stub value. |
| `pin_nearest_trace_exit_corner` | REAL port on the harness seam; production owes it | `MazeExpansionEngine.expandToDrill:56-65` → `Pin.nearestTraceExitCorner` (`Pin.java:636-668`). The harness (`drill/pins.rs`) carries the full port (exit-restriction directions, padstack shape gate, rotation, offset ray/border-line intersection); the production live-board `DrillEngine` impl lands with T7/T8. Provenance oracle `rust/harness/oracle/MazeExitProbe.java`: the T6 pin pad is the parse-DILATED `IntBox [180000,280000,220000,320000]`, `pinEdgeToTurnDist` = 100000.0, the RIGHT exit corner (322750, 300000) dominates the DRILL k=3 cost (exp 29573.69021909981, sort 458472.6902190998). |
| `ExpansionDoor.allocateSections` | ported, call-site boundary documented | Contract `:193-201`: same-count re-allocation is idempotent, different count RESETS the section state. The T6 capture contains NO re-segmentation event, so the drain pins cannot discriminate the reset arm (cerebrum mode 10) — closed by the direct contract pin `allocate_sections_resegmentation_contract` + two body mutants (always-reset / reset-disabled, both killed). |
| Completion seam | production, probe-pinned | `DrillEngine::complete_expansion_room` trait default delegates to `maze::completion::complete_null_shape_room`; burn-count + room-shape rows verified against the T5 probe pins (12 DrillSpikeProbe rows). |

## T7 ripup resolver + read-only shove probe (appended by M3-T7)

Landed: `maze/ripup.rs` (`MazeRipupResolver` — `calcFanoutViaRipupCostFactor`,
`checkRipup`/`checkRipupTraced`, `checkLeavingRippedItem`,
`enterThroughSmallDoor`), `maze/shove_probe.rs` (the READ-ONLY
`MazeTraceShover.checkShoveTraceLine` probe + `DoorSection` +
`shoveTraceRoom`'s expand loop, `MazeSearchEngine.java:1130-1201`),
`path/connection.rs` (`Connection.get` + the fork walk for the detour
economics). The T6 ripup/shove stub sites in `search_engine.rs` now call the
real ports.

| Seam / contract | Status | Detail |
|---|---|---|
| RNG ownership | ported, pinned | Java's ENGINE owns the `Random` (`MazeSearchEngine.java:63`, seeded with `ctrl.ripupCosts` at `:79-81`; the resolver holds nothing and draws through `search.randomGenerator.nextDouble()`, `MazeRipupResolver.java:160` — there is no `random()` accessor); the port mirrors that ownership — one `JavaRandom` owned by the ENGINE, seeded ONCE per construction with `ctrl.ripup_costs`, the resolver drawing through it — lifetime witness: P4a `2221734702.386413` ≠ P4b `2818803825.9464397` (the same pass re-run on a fresh engine draws differently; the generator is stateful ACROSS `checkRipup` calls, so capture-order matters). Randomize gate `passNo >= 4 && passNo % 3 != 0`: passes 1/3/6 false, 4/7 true (capture rows; pinned in the prefix battery). NO new RNG sites — the T6 seed row stays the only seed. |
| `checkTraceSegment` / `TraceShover.check` | stub, T10 remainder | `DrillEngine::check_trace_segment` answers `2147483647.0` (nothing fits) and `DrillEngine::shove_trace_check` answers `0.0` (nothing shovable) at the `shove_probe.rs` seam reads. The probe's gate/branch STRUCTURE is arm-for-arm Java (`:32-314`); with Seam-0 defaults the tail is trivially TRUE with an empty door list — Java's "board with nothing to shove into". |
| Shove-verdict pins (de-scoped, pop 9+) | documented deviation | INTENT: pin the drain's pop/ripped stream through pop 71 (the full capture). WHY THE FIXTURE CANNOT: at pop 9 Java re-parks the roomRipped door-458753 element 3× because its ENGINE-INTERNAL `shoveTraceRoom`→`checkShoveTraceLine` verdict for (door 458753, obstacle room 14337 = item 14 piece 1) is FALSE — a verdict computed from semantic board state behind the dim-2 branch's three FALSE arms (otherRoom-obstacle / endPointsMatching / corner distance), NONE of which the capture probes directly. With T10's seams stubbed the port cannot reproduce FALSE there (arm-for-arm faithful; disagreement is in the board state, not the ported arms). Downstream the streams diverge: Java capture room 232 vs port 231 (Δ1 id burn), Rust stream drains at pop 45 vs capture 72; the capture's harvest order is the literal 14,13,12,15,11,16,18 and the port's post-divergence stream interleaves it differently (hence the obstacle-keyed contract pairing below). EVIDENCE THE CAPTURE GIVES INSTEAD: the pre-divergence prefix (pops 0-8) is pinned row-for-row (pops, ripped scans, harvest 0's full battery; harvests 1-6 contract-pinned (P1 economics + room-derived state), gate triples, leaving/small-door streams, shove-probe calls, stale + hw-gate probes); post-divergence rows are pinned by CONTRACT (harvest set by obstacle lookup + obstacle-derived state + unrandomized-P1 economics — RNG-detour literals are not stream-independent post-divergence because leaked passNo=7 makes internal `checkRipup` calls draw). T8+ REQUIRED INPUT: the REAL `checkTraceSegment` + `TraceShover.check` (T10 remainder) — with them the dim-2 verdict reproduces and the tail pins can be re-literalized. Two banked survivors join that re-literalization: M3 (the fanout-cost factor's START-before-END contact probe order) needs a fixture with a protecting contact at BOTH ends of one fanout via, so the two probe orders disagree; M6 (the `shapeEntryCheckDistance` 5-vs-4 section-count boundary) needs a door segmentation row crossing that boundary. |
| `calcFanoutViaRipupCostFactor` SHOVE_FIXED arm | ported, arm UNPINNED on this fixture | The port implements Java's full trace-contact arm (`item_is_shove_fixed && corner_count == 2`). t7_ripup.dsn contains NO shove-fixed trace, so that arm is UNOBSERVABLE in the capture — a mutant weakening the corner-count strictness survives every gate on this fixture (cerebrum pin-failure-mode 9: the fixture's candidate behaviors coincide). T8+ REQUIRED INPUT: a SHOVE_FIXED fanout-arm witness needs a fixture with a `(type protect)`-style 2-corner shove-fixed trace. |
| Door arm `item_is_routable` | ported, pinned (bug-132) | Java `Item.isRoutable` is base-FALSE (Item.java:908-910); only Trace/Via override. The walk's door arm (`Sorted45DegreeRoomNeighbours.java:149-169`) therefore builds NO obstacle door against a pin. The T4 rig had conflated this with `isConnectable` — fixed (see buglog-132); both harness rigs now share the trace/via-only semantics. |
| Import-time `normalizeAllTraces` | ported, pinned (bug-131) | Java runs it at board import (Wiring.java:347); `Harness::build_with` now does too — T7 pops 0-8 pins required it (wires 10+13 merge, id 10 deleted). Live-only walk filters (`get_autoroute_tree`, harness freeze loops) mirror `UndoableObjects.delete`'s unlink; `insert_all_board_items` stays deliberately UNFILTERED (parsed items are born flag-false; see the NOTE doc + bug-131). |

## T8 destination-distance heuristic (appended by M3-T8)

Landed: `maze/destination_distance.rs` — the production
`DestinationDistance` (Java `autoroute/maze/DestinationDistance.java`,
392 lines): the ctor's derived-cost matrix, `join`'s three-way bucket
dispatch (component / solder / inner, `IntBox::union` with the EMPTY
identity), the `calculate(IntBox, int)` branch ladder, the FloatPoint
trait path (degenerate-box dispatch), and `calculateCheapDistance`. The
engine wires it as the production `D` of `MazeSearchEngine<D, C>`;
`DestinationDistance::from_ctrl` is the construction-site mirror of
`MazeSearchEngine.java:126-133`.

| Seam / contract | Status | Detail |
|---|---|---|
| The 0.0-quirk | ported VERBATIM, mutation-verified | `min/maxComponentSideTraceCost` assigned ONLY `if (layerActive[0])` (:63-71), the solder pair ONLY `if (layerActive[layerCount-1])` (:73-83); an inactive outer layer keeps the Java numeric default 0.0, which propagates through `maxInnerSideTraceCost = min(maxComponent, maxSolder)` (:86) into all three inner mins (:95-98). The bound is therefore INADMISSIBLE on partial-active boards by construction — Java truth, not a bug. Pinned by the 10-world ddCtor matrix (Q/Qs/G/D0 quirk rows vs the all-active witnesses); mutant M01 ("clean" skip-inactive port) killed. |
| Gate ladder | ported per-branch, mutation-verified | layer-0 branch `<=1` (:228) / `==2` (:259) / `==3` (:281); solder branch `<=2` (:321) / `==3` (:337); inner branch has NO early returns. Comparators are per-line truth: the `<=1` arm is pinned on BOTH faces (count 1 = D1, count 0 = D0 — the `==1` mutant's fallthrough collapses D0 to the via arm). |
| `calculateCheapDistance` | ported, ZERO consumers | save/substitute/restore of `minNormalViaCost` (:382-390). grep-verified: NO call site in the entire Java tree — ported as public surface with the save/restore semantics pinned (`calculate_cheap_distance_save_restore`: normal 20400.0 → cheap 20320.0 → restore 20400.0 + the field back at 400.0). `&mut self` is the honest shape. T9/T11 may consume it (the Java-side caller may appear in the batch autorouter); until then it is intentionally dead production surface. |
| Mirror absorption | NO pin-setup changes | the T6/T7 `TargetDistance` mirror ALREADY implemented real bucketing + the full ladder (it was a class-for-class port, not a stub), so absorbing it required zero pin setup edits: `make_run`/`t7_make_run` tuple slots and the two `::new(&ctrl)` sites swapped to `DestinationDistance::from_ctrl(&ctrl)`, the `drain` signature updated — all 8 T6/T7 pin tests pass UNCHANGED against production (their literals are the downstream differential proof). |
| Spike provenance | `MazeSpike.java` T8 battery | `SpyDistance` (delegating subclass) reflectively swapped into the engine field BEFORE `init` (the ctor owns the field; `init` is private — both reached via `setAccessible`, same-package + JDK 25): one `ddJoin`/`ddCalc` row per ENGINE call over t6_maze.dsn, PLUS a 17-world crafted battery (A-Qs from T8-open, H/M/G3 mutant killers, R/W/X/MJ quality-round killers). Capture `logs/M3-T8/t8q_capture_1.rows` = `t8q_capture_2.rows` (946 rows = 730 `dd` + 182 legacy + 34 quality-round, run twice BYTE-IDENTICAL; the 911-line content prefix is byte-identical to the open-round `t8_capture_5.rows` [912 rows incl. the `done` row]); the 182 legacy rows are byte-equal to a pre-spy t6_maze.dsn run of the T6-era spike (git `cfa41f06^:rust/harness/oracle/MazeSpike.java` — NOT t7_ripup.dsn, whose capture is a different fixture). |
| Mutation battery | 16/16 killed | Open round M01-M12 (quirk-clean ctor, `<=1`→`==1`, the two OPPOSITE pair-arm comparisons, comp `==2`/`==3` deletions, solder `<=2`→`==2`/`>=2`/`==3` deletion, join misdispatch, sentinel, cheap-no-restore); quality round MQ1 last-join-wins, MQ2 inner-tail truncation, MQ3 solder `==3`→`>=3`, MQ5 wrong 8th-field ctor formula. The four open-round gate-deletion survivors were killed by ADDING capture worlds H/M/G3 — single-box-per-bucket worlds cannot shrink a fallthrough arm below the pre-gate min (cerebrum mode 9); M needs all-expensive costs (minCompInner > 1) plus a strictly-outside NEAR inner box (inner_min > 800) so `:285` undercuts `:265`. Mutant-design traps hit in the quality round: `==3`→`<=3` is a NO-OP (the `<=2` gate above returns first — the surviving direction is `>=3`), and a half-wrong 8th-field formula `min(minCompInner, minSolderSide)` still coincides on R (min(1, 5) = 1); only the full `min(minComponent, minSolder)` diverges. |
| T8 quality round (MQ1-MQ5) | 4 new pins + ctor-literal discipline | MQ1 join accumulation — no open world joined two DIFFERENT boxes into one bucket; `ddBucket MJ` pins the accumulated component coordinates ("100000 200000 400000 400000") via the (now live) `#[cfg(test)]` accessors. MQ2 inner tail — both open inner-layer pins came from the weighted arm; world W (FAR component / TGT solder / EMPTY inner, 4L) makes the :369 solder-pair arm win strictly (P0@L1 → 400.0; truncation → :377's 800.0). MQ3 count-4 face — A's L3 pin is weighted-arm-decided; world X (all-expensive 5/6, differing buckets) makes the :345 FOUR-layer arm win strictly (P4@L3 → 101200.0; `>=3` early-return → 300800.0). MQ5 — `assert_ctor` re-derived the 8th field (`expected[6].min(expected[5])`), which coincides with the wrong `min(minComponent, minSolder)` on every open world (H: min(1.1,1) = min(3,1); M: min(1.5,1.6) = min(1.5,1.7)); it now QUOTES all 8 `ddCtor` fields and world R (cheap inner layer) discriminates (1.0 vs 5.0). |
| T9/T11 hooks | open | T9 (locator) consumes `calculate` for `MazeExpansionEngine` cost shaping (`MazeExpansionEngine.java:87/128/358` call sites); T11 (inserter assembly) owns the batch-autorouter loop that may call `calculateCheapDistance` (still zero consumers). The fanout-time board-bounds joins (`:991-992`) are ALREADY PORTED at `search_engine.rs:1100-1106` (the `is_fanout` gate, outer + inner bounds joins) — T11 inherits them, don't re-derive; they stay unobserved in the T8 capture (`isFanout` false on the fixture). The bucket accessors (`component_side_box` etc.) are `#[cfg(test)] pub(crate)`, mirroring Java's private fields; first live consumer is the MQ1 pin. |

## T9 found-connection locator (appended by M3-T9)

Ported: `FoundConnectionLocator.java` (570 lines) → `path/locator.rs`; `FoundConnectionLocator45Degree.java` (357 lines) → `path/locator_45.rs`; `FoundConnectionLocatorAnyAngle.java` (455 lines) → `path/locator_any.rs`. The Java abstract-base + two-subclass family is modeled as the [`Synthesis`] enum (FortyFive serves BOTH the ninety- and fortyfive-degree restrictions, per Java `getInstance :196-200`; the restriction itself flows into `calculateAdditionalCorner`) with free functions over a `LocatorState` struct carrying Java's protected fields.

| Seam | Status | Contract / rationale |
|---|---|---|
| Key→door resolution (D17) | CLOSED via `maze::locator_access::LocatorAccess` | Java reads LIVE room/door objects off the maze elements (`backtrackDoor.otherRoom(...)`, `drill.roomArr[...]`, door shapes); the port resolves opaque door keys through the engine's backtrack REGISTRY (every door of a finished chain is registered when its section state is written, which precedes every backtrack read — invariant pinned) + room reads through `NeighbourEngine`. `expandable_object_shape` re-types room-door shapes via `ExpansionDoor.shape_between(first, second)`. |
| `emitDiagnostics` (`:502-516`) | NOT ported | No Rust diagnostic sink (same disposition as the drill `emitDiagnostics`, drill/mod.rs deviations). |
| `rippedItemList` ordering deviation | documented | Java `SortedSet<Item>` with DESCENDING id order (`Item.compareTo = other.id - id`); port carries `BTreeMap<i32, u64>` ASCENDING — the T10/T11 consumer MUST iterate `.rev()`. |
| `ripupCosts` nullability | narrowed | Java's map parameter is nullable; every engine call site passes non-null → port takes `&mut HashMap`. |
| Tree-variant-per-restriction contract | DOCUMENTED (bug-137) | Java `SearchTreeManager.getAutorouteTree` (SearchTreeManager.java:140-158) selects the tree variant from the CURRENT board restriction AT ENGINE-CREATION time. Never set restrictions after building the search structures; the harness `build_with_restriction` applies the restriction BEFORE tree creation. The restraints in room completion ARE the tree shapes. |
| 90-degree room completion | PORTED in epic-index, LIVE-BUT-UNTESTED | `complete_shape_ninety_degree` + `restrain_shape_ninety_degree` (+316 lines, `epic-index/src/complete_shape.rs`; ShapeSearchTree90Degree.java:39-320) landed for the A90 pin — with the A90 pin de-scoped below, NO test drives them yet; they go live with A90/SortedOrthogonalRoomNeighbours (expansion-engine scope) and must be mutation-verified then; dispatcher arm by `SearchTree::variant`. Unconditional seed room, dynamic `boundingShape` prune (union accumulation is result-identical to Java's O(n²) re-scan by min/max idempotence), box-typed 4-directional restrain with per-arm assignment order, no divideLargeRoom. |
| A90 pin | DE-SCOPED `#[ignore]` | The A90 path then needs `SortedOrthogonalRoomNeighbours` (728 lines, orthogonal neighbour sorter) — an expansion-engine (T4-scope) component, NOT locator surface. Capture banked `logs/M3-T9/captures/t9_locator45_capture.rows`. |
| contains(FloatPoint) virtual dispatch | FIXED in epic-geometry (bug-135) | Java `IntOctagon` OVERRIDES the ONE-ARG `contains(FloatPoint)` (IntOctagon.java:332-344) with border-INCLUDED `<=` arithmetic; IntBox/Simplex fall through to the strict two-arg loop. `TileShape::contains_float` now dispatches per variant; `contains_float_tolerance` (the two-arg) stays strict — no octagon override exists for it. P14/P14B pins capture both (jshell: oct one-arg (0,5)=true, box one-arg (0,5)=false, oct two-arg (0,5)=false). GLOBAL parity fix — every contains_float caller shares the dispatch. |
| Via worlds (A45V/BANYV/BBT/CBT pins) | DE-SCOPED `#[ignore]` | Harness stub `LegalChecker` always answers Drillable; Java runs the STATIC `ForcedViaInserter.checkLayer` (ForcedViaInserter.java:30) whose real port pulls `ForcedPadRouter` (T10 shove machinery). A capture-forced lookup-table checker is feasible (the board is static during a phase) — banked as the follow-up. CBT capture shows Java detouring to layer 1 (pop 422, 24 ripped-queued) where the stub goes direct (pop 154). |
| CBTR/CBTR2/CBTR10 pins | DE-SCOPED `#[ignore]` | Ripup-world search-order divergence (Java pop 11 vs Rust 72): points at ripup-aware collision-cost/destroy handling in the SEARCH, not the locator. Seeds 2/10 are byte-identical to seed 1000 in Java, so they add no independent discrimination until CBTR correlates. Captures banked `logs/M3-T9/captures/t9_ripup_capture.rows`. |
| BANY pin | DE-SCOPED `#[ignore]` | Pop 36 (Rust) vs 29 (Java) SEARCH divergence INVARIANT under tree variant (byte-identical failure under FORTYFIVE- and GENERIC-variant trees) — not tree-shape related; root cause unreached in budget. The locator side is EXPECTED convergent — a probe HYPOTHESIS, not a banked test (a one-off fix-round probe drove the Rust locator over the Java search's room chain and it emitted the exact Java capture trace; see the BANY pin doc — re-verify first if the search de-scope is revisited). De-scope is search-only, and the passed-door arms do not fire on this chain. Capture banked `logs/M3-T9/captures/t9_locator_any_capture.rows`. |
| Mutation verification | 15 killed / 4 survived-documented / 4 blocked (round 2 + quality round) | KILLED: factory dispatch incl. the NinetyDegree arm (spec-review A1 — now pinned directly on `synthesis_for`), round-dedup (3 pins), empty-shrink fallback gate (A45+ABT), fortyfive `abs_dx <= abs_dy` boundary (unit pin), MAJOR-1 passed-door revert (both arms empty → `any_angle_passed_door_alias_continues_trace` sees 1 corner vs 2), FromDoor/ToDoor face swap (spec-review C1 class — the kill is LEVEL-DEPENDENT: a core-BODY swap dies on the ToDoor dim!=1 OFF-TIE literals, a wrapper-level swap dies on the semantic pin's off-tie wrapper probes `pins.rs:1136-1149`; the faces agree at every tie, so tie literals alone cannot see a swap), ToDoor `!core` revert + per-arm comparator flips (bug-139 round 2 — the four JAVA-tie literals kill `!core` and boundary widening `<`→`<=`/`>`→`>=`, WITNESSED failing against the pre-fix `!core` code before the verbatim rewrite; the off-tie literals kill strict `<`↔`>` flips), right-diagonal core comparator (arm battery). SURVIVED (traced to fixture degeneracy/coincidence, code matches Java): fanout adjust gate (AFAN adjust returns None), obstacle-vs-freespace shrink offset (no obstacle rooms on green chains), adjust horizontal-first flag (degenerate 2-unit vertical move rounds identically), nearest-border `>=` boundary (no exact ties). BLOCKED by de-scoped pins: ripped-step harvest + no-rip arm (CBTR/CBT), any-angle index_of left/right (BANY/BBT), the destination-drill harvest arm (`FoundConnectionLocator.java:256-265` → locator.rs:438 region — the flat-cost mutant survives because NO banked capture, Java's included, reaches a drill destination with `room_ripped=true`; the required input is a T10 fanout+ripup world), and the passed-door arm 2 (`:178-186`, Java's own "should not happen" sweep arm) is only exercised by the JOINT revert-mutant — the synthetic chain fires arm 1 first; no pin isolates arm 2. |
| backtrack section clamp (`currentSectionNo to big`) | ported verbatim, disposition DOCUMENTED | Java `FoundConnectionLocator.java:278-281` warns "currentSectionNo to big" and clamps `currentSectionNo` to elementCount - 1. The port keeps the CLAMP (the behavioral payload — the element read below uses the clamped index) and documents the warn as NOT ported (diagnostic-only, same disposition as `emitDiagnostics`) at the clamp site in `locator.rs::backtrack`. |
| Passed-door alias semantics (MAJOR-1 fix) | ported via `NextCorner`, SEMANTIC PIN | Java's passed-door arms (`FoundConnectionLocatorAnyAngle.java:98-103`, `:178-186`) advance `currentToDoorIndex` and return the `currentFromPoint` REFERENCE as a non-empty singleton: the base reference filter (`FoundConnectionLocator.java:432`) drops it, but the non-empty result keeps the trace loop RUNNING from the advanced index. An empty return there is Java's trace-END signal (`:428-429`) and truncates the trace. The port models the reference/alias distinction as `NextCorner::{Fresh, FromPointAlias}` (a value-equal fresh corner would desync `previousFromPoint`; the enum IS Java's reference semantics). Pinned by `any_angle_passed_door_alias_continues_trace` — a SEMANTIC contract pin (expectations derived from the Java ARMS, not capture rows, per pin-failure-mode (8)): no agreeing any-angle search world exists (BANY/BBT de-scoped), so the world is the real F45 search and the backtrack chain is synthetic (from = door midpoint + 50·left normal; both scalarProduct terms are exactly 2500 by construction). Revert-mutant dies (1 corner vs 2). |
| `calcHorizontalFirstFromDoor`/`ToDoor` mirror | two verbatim faces, JAVA-verdict pins | The pair is COMPLEMENT OFF-TIE, AGREEMENT ON TIE: the five comparator arms AGREE at their ties (`height >= width` vs `height <= width` — both TRUE at height == width; each diagonal strict `<`/`>` vs its mirror — both FALSE at |dx| == |dy|); only the fixed about-vertical/horizontal constants negate. The round-1 `!core` structuralization flipped all five tie verdicts (bug-139), so the port now carries TWO separate verbatim verdict fns — `horizontal_first_core` (FromDoor, `:48-101`) and `horizontal_first_to_door_core` (ToDoor, `:304-356`, classification block deliberately duplicated) — and `calc_horizontal_first_to_door` delegates WITHOUT negating. Pins: the FromDoor arm battery (`horizontal_first_core_arm_pins`, incl. its `>=` tie) and `horizontal_first_to_door_java_tie_pins` — every ToDoor arm against JAVA'S OWN verdict with the Java line quoted: four TIE literals (square box height==width; |dx|==|dy| on both diagonals under both signum relations — reachable in default 45° mode on integer coordinates), off-tie literals per diagonal arm, the dim!=1 off-tie pair, and the about-vertical/horizontal constants. The semantic-pin world asserts wrapper DELEGATION only; `assert_ne!(from_face, to_face)` is dropped (the faces AGREE at ties). |
| Any-angle gap-tie (`<=` keeps LEFT) | ported verbatim, UNPINNED — de-scope doc | `FoundConnectionLocatorAnyAngle.java:144`: when the visible gap is too small, `leftCornerDistance <= rightCornerDistance` takes the LEFT turn — the `<=` tie-break has no witness in any green world (the 45-degree dispatch has no equivalent tie; BANY/BBT de-scoped). A `<=` → `<` flip is invisible to every current gate. Goes live with any-angle search parity. |
| Any-angle nearest point: EXACT vs approx | ported verbatim, UNPINNED on the any-angle side — de-scope doc | The any-angle target arm uses the EXACT `nearestPoint` (Java `:70-78`, `:252-263` feed `nearestPoint(IntPoint)`), while the 45-degree dispatch uses `nearestPointApprox` — pinned only on the 45-degree side (capture literals A45/ABT/AFAN). The exact-face nearest-point arithmetic has no green any-angle world to pin it (BANY de-scope, search-side); goes live with any-angle search parity. The passed-door semantic pin DOES drive the exact `nearest_point` (its second corner) — the arm, not the tie/boundary variants. |
| Comparator equivalence finding | documented in pin | Inside `fortyfiveDegreeCorner`, the inner `to.y >= from.y` / `to.x > from.x` strictness is NOT separately observable (ties unreachable or ±0-coincident); the ONLY observable boundary is `abs_dx <= abs_dy`. Pinned + doc'd in `fortyfive_degree_corner_comparator_pins`. |
| T10/T11 required input | `connection_items` hook | The locator returns `FoundConnectionLocator { connection_items: Vec<ResultItem{corners, layer}>, start_item, start_layer, target_item, target_layer }` — the T10 (ForcedPadRouter/inserter) + T11 (connection assembly) consumer surface. ResultItems carry ROUNDED IntPoint corners, Java `:450-459`. |

## T10a shove substrate (appended by M3-T10a)

Ported: `ShapeTraceEntries.java` (806 lines) → `epic-board/src/shape_trace_entries.rs` (in full: ctor, storeItems, nextSubstituteTracePiece, cutoutTraces/cutoutTrace/fastCutoutTrace, storeTrace, searchFromSide, resort, calculateStackLevels, popPiece, insertEntryPoint, rotateEntryListAroundAnchor, netNosEqual); `ShapeEntrySide.java` (161 lines) → `shape_entry_side.rs` (4 Java ctors → const `NOT_CALCULATED` + `new_precomputed` + `from_entry_no` + `from_point` + `from_line_segment`); `ShapeAndEntrySide.java` (120 lines) → `shape_and_entry_side.rs` (free fn `shape_and_entry_side` + the two cutline helpers). Six parity gaps closed: `board.clearance_value`, `trace_compensated_half_width`, `contains_on_border_line_no`/`nearest_border_point`, `TileShape::is_contained_in_int_box`, `SearchTreeManager::reuse_entries_after_cutout`, `Polyline::combine`. Oracle: `rust/harness/oracle/ShapeTraceEntriesProbe.java` (jar-side spike, two byte-identical runs) + jshell captures; rows in `logs/M3-T10a/captures/` (`shape_trace_entries_rows.jsonl` + `_run2.jsonl`, `shape_entry_side_caps.txt`).

| Seam | Status | Contract / rationale |
|---|---|---|
| EntryPoint chain modeling | VEC-ORDER-IS-THE-CHAIN | Java's intrusive `EntryPoint` singly-linked list behind private `listAnchor` is a `Vec<EntryPoint>` where Vec position IS the `next` order — the probe's reflection walk over `next` pointers is the order witness, and every Java list op translates 1:1 (sorted `Vec::insert` / rotate prefix++edge_count / sliding-window dedup+trims / `drain` splices). Caller-order contract: `storeItems` inserts `item_list` in ITERATION order, so the caller must supply the M2-parity `overlappingItemsWithClearance` order — a differently ordered list reorders the chain and, through the pops, the inserted substitute pieces (T10b's drivers supply it). Java hands out UNINSERTED id-0 `PolylineTrace` objects from `nextSubstituteTracePiece`; the port returns a `SubstituteTracePiece` value (id assigned at insert time — Java's id-0 ctor defers the same way). `entry_chain()` is a `pub(crate)` test window onto the private list. |
| insertEntryPoint sorted-walk position | FIXED (bug-142), mutation-witnessed | Java's splice inserts BEFORE `currentNext` — the position where the walk stopped. The first port materialized the position only in the same-edge tie arm; the `edgeIndex >` break fell through to append-at-end. Live-caught by the s1-notcalc/s1-padcheck/s3-frompoint capture pins (chain came out insertion-ordered, searchFromSide picked the wrong own-net head, resort never rotated). One line: `insert_pos = pos` before the break. |
| foundObstacle semantics | QUIRK PRESERVED + PINNED | `storeTrace` sets `foundObstacle = trace` ON SUCCESS too (`:440-441`), so a Some value alone is NOT a failure verdict (the store pins assert `found_obstacle == Some(108)` on `store_items == true` runs). `get_found_obstacle()` doc carries the warning. |
| `:380-381` eq_op quirk | PRESERVED VERBATIM; tautological AS WRITTEN — its fix-mutant is UN-EXERCISABLE in the pinned worlds | Java compares `contactItem.clearanceClassIndex() != contactTrace.clearanceClassIndex()` — BOTH names denote the SAME item, so the clause is tautologically FALSE and the shove-fixed/half-width/class gate degenerates to its first two arms. Port keeps the clause under `#[allow(clippy::eq_op)]` with the analysis comment. Mutant status (corrected per spec review): the natural fix-mutant (`contact_class != trace_class` against the STORED trace's class) EXISTS and IS behavior-changing in worlds where a contact trace's clearance class differs from the stored trace's — it is merely UN-EXERCISABLE in the pinned worlds (every item there is clearance class 0, so the mutant survives them), the same status as the viaTraceDiff arms; banked for T10b world coverage. |
| reuseEntriesAfterCutout fidelity | PORTED (documented mechanism deviation, observably equivalent — see the `reuse_entries_after_cutout` doc in `tree_manager.rs`), LIVE-PINNED | `fastCutoutTrace` inserts both pieces with `on_the_board: false`, calls `manager.reuse_entries_after_cutout(board, old, start, end)`, THEN removes the old trace. Java TRANSFERS leaves (re-labels `leaf.object` / `shapeIndexInObject` on the start/end slots, fresh-inserts only the LAST start-piece and FIRST end-piece leaves that straddle the cutlines, and leaves the middle cutline leaves for the later `removeItem`); the port instead DROPS `from_trace`'s entries per tree and re-inserts BOTH pieces through the ordinary `insert_into_tree` — no slot transfer, no descending-id re-insert, no clear_derived_data. Sound because the two leaf sets are VALUE-identical (a trace tree shape `i` involves only lines `i`/`i+1`, and the transferred indices never straddle a cutline, so each transferred shape equals the fresh computation from the unchanged piece polyline) and leaf IDENTITY is unobservable outside the module (query results come back in sorted descending-object-key candidate order, not BVH skeleton order). The cut-mid pin asserts pieces 110/111 each carry exactly ONE tree entry at shapeIdx 0, matching the capture — count and shape index are the observable surface, leaf identity is not. The `board.additionalUpdateAfterChange` hook (RoutingBoard override invalidates the autoroute database) is a documented no-op — engine seam landing with T11. |
| inShoveCheck gate | PORTED, MUTATION-KILLED | `ShapeAndEntrySide` ctor suppresses the nearest-border from-side fallback in check-shove mode ("may produce an undesired stackLevel > 1"); capture pair de-5-mid-check (fromSide null) vs de-5-mid (fromSide [4, ...]) pins the gate, M15 flip dies. The `tmpShape != currentShape` identity comparisons in the cut arms are VACUOUSLY TRUE (fresh simplex every time) — documented, emptiness is the live gate. |
| mutation round (epic-board pins) | 15 killed / 3 survived-documented / 3 un-exercisable | KILLED: storeItems via-gate (shoveViaList pins), tails flag, resort rotation (+edge_count), tail trim (s1), head trim (s2), stack-level raise arm, popPiece own-net recursion, popPiece top-first, nextSubstitute border wraparound, ctor-A fallback tie rule (killed by the de-5-mid 8-border dog-ear world — the strictly-closer rule has a real tie), ctor-C direction swap (octagon +2/-2), inShoveCheck gate, end cutline, cutoutTrace nothing-cut identity arm, cutout fast-path gate. SURVIVED (traced, code matches Java — pin-world blind spots): resort triple-dedup (no captured world has three consecutive same-net chain entries — needs a third foreign trace between two crossings of one of them), calculateStackLevels stack-property fail arm (no world violates the close-at-open-level property), nextSubstitute empty-piece recursion (no world produces an empty piece; s2's degenerate piece is 2 identical corners but valid). UN-EXERCISABLE in the captured worlds: viaTraceDiff <0/==0 arms (the probe via never contacts a trace end corner — needs a padstack with smallest_radius exactly == trace compensated half width) and the `:380-381` eq_op fix-mutant (all pinned items are clearance class 0 — see the eq_op row). |
| coverage gaps (banked for T10b) | documented | (a) the three survived mutants above — each needs one extra capture world from the existing probe (small world edits); (b) viaTraceDiff arms; (c) storeItems ladder arms for ConductionArea/ComponentObstacleArea items (no such item in the world — the ladder is exercised only via the via/trace/pin arms); (d) copper_sharing_allowed=true runs (both capture runs pass false); (e) resort `fromPointDist >=` reset arm (needs a world whose from-side projection sits past the side end); (f) head-trim netsEqual-vs-netNosEqual distinctness (needs a net-0-bearing trace at the chain head); (g) end-corner `contact_count == 1` projection-entry arm (no captured row has trace_line_no 0 or lines.len()-1 — the 0-vs-last swap mutant survives all pins). |
| Java-over-brief notes | none open | The brief's anchors all checked out this round; the two API mismatches hit during probe-writing (PolylineTrace package, Item.getId() → int) were resolved by reading the Java (documented in the probe header). |

## T10b shove/pad/via drivers (appended by M3-T10b)

The four MUTUALLY-RECURSIVE driver classes landed as one commit:
`TraceShover` → `epic-board/src/trace_shover.rs` (static/instance check+insert,
springOver, springOverObstacles), `DrillItemMover` → `drill_item_mover.rs`
(check/insert/shoveVias/tryShoveViaPoints + `BasicBoard.insertVia`/`splitTraces`
as the D2 via surface), `ForcedPadRouter` → `forced_pad_router.rs`
(checkForcedPad/forcedPad/calculateFromSide/checkTraceShape), and
`ForcedViaInserter` → `forced_via_inserter.rs` (checkLayer/check/insert/
holeCheckShape). Oracle: `rust/harness/oracle/TraceShoverProbe.java` (full
insert-replay rows), `rust/harness/oracle/ShapeTraceDebtProbe.java` (the
T10a banked-debt worlds + the T9 via-world checkLayer ladder), and the two
spec-review fix-round probes `rust/harness/oracle/SpringOverNestProbe.java`
(the nest-pair springOver world + the reflective depth-1 killer row) and
`rust/harness/oracle/ForcedPadFrontProbe.java` (the inFrontOfPad
(1,−1)-slope table via reflection on the private static + the
checkForcedPad budget sweep); captures
`logs/M3-T10b/captures/` (`trace_shover_rows.jsonl` + `_run2.jsonl`,
`shape_debt_rows.jsonl` + `_run2.jsonl`, `spring_nest_rows_run1.jsonl` +
`_run2.jsonl`, `pad_front_rows_run1.jsonl` + `_run2.jsonl`, each pair
byte-identical). 17 driver pins (t1-t8, 2 drill, t9-t10, v1-v5, t11-t12,
2 fix-round side-6 pins).

| Seam / semantics point | Status | Detail |
|---|---|---|
| Recursion/depth contract | PORTED; MUTATION-KILLED except two banked decrements | The cycle `TraceShover.check/insert → DrillItemMover.shoveVias/tryShoveViaPoints → ForcedPadRouter.checkForcedPad/forcedPad → TraceShover.check/insert` decrements its two budgets at every re-entry: `maxRecursionDepth` gates on `<= 0` (M5's twin at `forced_pad_router.rs:319`, killed by t10), `maxViaRecursionDepth` gates `shoveVias` mid-loop (`<= 0` answers TRUE — damaged-db quirk `:207-209`). The via MOVE recursions' `- 1` per level is Java-verbatim but UNPINNED — quality-round Q3 (via-budget decrement `−1` → `−0` at the `shoveVias`→`check` re-entry) SURVIVED the suite; discrimination needs a nested-via world (a via move consuming exactly one nested level, driven at via budget 1 — foreclosed in every pinned world: t1's via shove is single-level, t6(b) returns at the `<= 0` gate). Banked alongside the static-check stackDepth survivor below. The SPRING-OVER budget is PER-INVOCATION across the substitute-piece loop (binding hoisted above the loop in both drivers, Java `:384`/`:539` decrement — quality-round MAJOR-1, the per-piece re-arm form diverged and is now pinned: t13's octagon two-piece world at budgets 0/1/2, jar capture `spring_budget_rows_run{1,2}.jsonl`). `ShapeTraceEntries.stackDepth() > 1` hard-fails the instance-CHECK face (`trace_shover.rs:379`, Java `TraceShover.check :379-382`) and the checkForcedPad ladder (`forced_pad_router.rs:378`, Java `:297-299`); the INSERT faces carry NO stackDepth gate (Java `insert` has none — correctly absent here). The instance-check-site gate is mutant-killed (t2) [label corrected per spec review: earlier text misnamed it the insert-site], the STATIC-check-site gate (`trace_shover.rs:150`) is a BANKED SURVIVOR: in t2's world the gate early-out and the deeper shove-fail answer the SAME (false, obstacle 115) — the verdict+obstacle pair coincides (cerebrum mode 9), discrimination needs a depth-2 world where the recursive shove would SUCCEED (verdict false vs true). |
| Deadline model | PORTED (parity gap closure) | `TimeLimit` → `epic-board/src/time_limit.rs`: Java `TimeLimit.checkLimitExceeded` accumulates elapsed-vs-limit; the drivers thread `Option<&TimeLimit>` through check faces ONLY (insert faces run without a limit — Java's item database is already changed when the insert starts, aborting there would corrupt it; DrillItemMover.insert passes null verbatim). |
| shoveFailing* surface | PORTED for T11 | `check`/`insert` set `board.set_shove_failing_layer(i)` on every early-out (ForcedViaInserter `:180/:205/:234/:307`); `TraceShover.check` additionally reports `setShoveFailingObstacle` (the instance check only — the STATIC check does NOT, `:150` note). `checkLayer` NEVER reports either — only the delegated `checkForcedPad` runs can (module doc). This is the failure-report channel the maze drill phase (T11) consumes. |
| The changedArea-null NPE-skip normalize seam | PORTED AS THE NULL FACE; T10c completes it | `TraceShover.insert`'s tail wraps the substitute-piece `normalize(board.changedArea.getArea(layer))` in try/catch (`:545-550`). `RoutingBoard.changedArea` is `public transient` and NULL until `startMarkingChangedArea()` (autoroute pipeline / pull-tight callers only). The T10b probe calls insert directly on a fresh parse → `getArea` NPEs INSIDE the try → the ENTIRE normalization is skipped (demoted to FRLogger.error). The port therefore does NOT normalize (documented seam comment in the insert tail): an unrestricted `normalize(.., None)` would MERGE the substitute piece's perpendicular end contacts via `PolylineTrace.combine` (which has NO collinearity requirement — start-to-start abutments merge through the reverseOrder branch) and diverge from the capture (t1's halves 108/109 + U 110 survive SEPARATE). Buglog 144. T10c replaces the skip with `normalize(.., Some(changed_area.get_area(layer)))` under active marking. Join-changed-area calls in DrillItemMover.insert (`:153-156`) are the same seam (documented no-op). |
| Query-reach model (the t11 lesson) | FACT, DRIFT-GUARDED | `overlappingItemsWithClearance` observable reach = candidate prune ∩ acceptance. (1) PRUNE: the query bound is offset by `(int)(1.2 * ClearanceMatrix.maxValue(queryClass, layer))` (`max_clearance_offset`, tree_manager.rs) and R-tree-overlapped against RAW stored leaves. `maxValue(classI, layer)` reads `row[classI].maxValue` — a ZERO-INIT accumulator updated only in `setValue` (which writes `row[classJ]`); the parse targets rows/columns ≥ 1 only (`getNo` ≥ 1, "wire" default 1, `setDefaultValue` loops from 1), so `maxValue(0,0) = 0` (Java parity, buglog 145) while `maxValue(1,0)` = the fixture default (t9_locator45: DSN 250 parse-scaled ×10 → 2500, offset 3000). (2) ACCEPTANCE: a candidate passes only if query and stored, EACH enlarged by `cl/2` where `cl = getValue(query, item, layer, +clearance_safety_margin=16)`, intersect — the unwritten `(x,0)` cells are 0, so ±8 per side. The ±8 acceptance DOMINATES the observable reach (it filters far class-1 candidates and subsumes the class-0 raw reach); t11's geometry C = 306098 sits mid-window (center+96, center+100]. Stored tree shapes: the default tree is NOT clearance-compensated → stored = geometry ± plain half width (`item_class <= 0` guard: class-0 items never compensate). |
| Non-compensation + descending order facts | PINNED | The M3 worlds run the default (non-compensated) tree: cutout/offset shapes take the two-step symmetric enlarge (halfWidth, then clearance+1), and class-0 stored shapes carry NO clearance pad. The raw overlap query returns items DESCENDING by id (`BTreeSet<Reverse<ItemId>>`), so `storeItems` iterates high-to-low and the `:440` quirk's "last-stored trace" = the LOWEST id (t2 pins 115). The t2/t9 pins pass the driver the production query order. |
| insert_zero id burn | PINNED | `TraceShover.insert`'s substitute pieces burn ids at POP time (Java's `PolylineTrace` ctor assigns the real id at construction, `Item.java:86-90`); t1's world burns exactly one id (107) with no lasting item — final ids 108/109/110 match the capture. Degenerate pieces skip construction (no id), matching the probe's tracePieceCount-0 recursive probe rows. |
| checkLayer maze facts | PORTED, pinned v1-v5 | The maze drill call site `MazeExpansionEngine.checkLayerWithAnyMatchingVia` (`:377-414`) passes `maxViaRecursionDepth` as LITERAL 0 (`:396`) — every via-shove recursion inside a maze checkLayer is budget-dead on arrival (drill pages may only shove TRACES). Required radius = `max(viaRadius, ctrl.traceHalfWidth[layer])` per via-rule entry; DRILLABLE short-circuits the via loop, DRILLABLE_WITH_ATTACH_SMD accumulates. t1-t5 pin the checkLayer ladder (zero-radius short-circuit, SMD attach downgrade, SHOVE_FIXED start-trace block, room-outside, NINETY_DEGREE tile branch) against the ShapeTraceDebtProbe captures. |
| t5 min_x lesson | recorded | Equal-length CW/CCW wrap circuits are discriminated by MIN X of the corner set (the left wrap wins the `<=` tie after reversal); the pin asserts the full corner list, not just counts. M3 (`<=`→`<` at `spring_over_obstacles`) dies at t5. |
| T10a debt paydown | 8/8 CLOSED | (a) resort triple-dedup survivor → debt_world w1 (two same-net foreign traces fully crossing; dedup removes the two MIDDLES, both ends of the LOWER trace survive — the remove-prev mutant flips the survivors). (b) stack-property fail arm → w2 (interleaved X crossing cannot close level 2; result false with pieces still counted and foundObstacle = the :440 quirk trace). (c) empty-piece recursion → CLOSED AS HONEST BLOCKER: no replay world produces an EMPTY piece (the s2 degenerate piece has 2 identical corners and is valid); the recursion arm stays pin-free — documented, not faked. (d) viaTraceDiff arms → w6 (half width 200 > via radius 100 → diff < 0 → storeTrace fails on the VIA) + w7 (end corner EXACTLY on the offset boundary: contains holds, containsInside does not → no projection entry) + w5 (diff == 0 WITH inside → the contact-segment projection entries, lineNo 0 vs lines.len()-1). (e) fromPointDist reset arm → w3a (projection 3200 ≥ side length 1400 → the reset arm swaps to side (0, null), the chain ROTATES, 110-bottom carries edgeIndex+edgeCount) vs w3b (mid-side projection 100 < 1400 → NO reset, the scan pre-assigns levels, the raise prunes). (f) netsEqual-vs-netNosEqual → CLOSED BY PROOF + w4/w4b: the head-trim site uses ITEM netsEqual (`containsNet`-routed), the tail/dedup sites use the local order-independent netNosEqual; the provable-equivalence domain is ALL DISTINCT-ELEMENT net-0-free arrays — not just the singletons in the pinned worlds [domain extended per spec review NIT]: Item.netsEqual (Item.java:1234-1244) and netNosEqual (ShapeTraceEntries.java:153-167) both reduce to set equality under equal length + distinctness, so a discriminator needs duplicate nets or a net-0 argument — not production-reachable; w4/w4b pin the trim GEOMETRY (two-step head+tail trims, interior survival); documented as proven-equivalent rather than capture-discriminated. (g) end-corner projection arm → w5 (the `contact_count == 1` arm inserts entries with trace_line_no 0 and lines.len()-1: rows 120@0 and 118@2). |
| Mutation round (drivers) | 6 killed + M7 / 1 survived-banked | M1 `tryShoveViaPoints` +2 tolerance → +0: killed (t7 dedicated + t1 replay). M2 `inFrontOfPad` case-0 self-add `line_b.x + line_b.x` → cross-add: killed (t9). M3 spring tie `<=` → `<`: killed (t5, geometry flip). M4 instance-check-site `stackDepth() > 1` → `> 2`: killed (t2). M5 forced-pad via-budget `<= 0` → `< 0`: killed (t10). M6 piece-loop `dir: None => true` → `false`: killed (t8). M7 `holeCheckShape` +10 → +0: killed (t11 — the hole query empties, the class-1 face passes, the `!check` assertion dies at the QUERY). SURVIVOR: the static-check `stackDepth() > 1` site (banked above, coincidence-blind). |
| Java-over-brief notes | none blocking | The brief's ±8-only reach arithmetic for t11 was WRONG in one half: the candidate prune for nonzero rows (offset 3000 on the fixture) admits far candidates, but the ±8 acceptance filters them — the window algebra moved C from 306110 to 306098 (resolved by reading `ClearanceMatrix.java` + `Structure.java:752-793`, buglog 145). |
| springOverObstacles attempt order (spec-review survivor (c)) | BANKED BLIND SPOT | The CW-first/CCW-first attempt-order mutant SURVIVED all driver pins: every pinned world is order-blind — t5's wrap world is mirror-symmetric (equal-length circuits, decided by the `<=` tie, itself pinned by M3), and the fix-round nest world (t12) sends both attempts around the SAME single obstacle cluster, so either order yields the same verdict. Discriminating-world design (unbuilt): an ASYMMETRIC corridor where an obstacle is reachable from only ONE traversal direction — a second SHOVE_FIXED obstacle wall blocking one of the two wrap corridors so the CCW attempt wraps while the CW attempt on the reversed input FAILS (or yields a structurally different circuit), driven through `spring_over_obstacles` with the returned circuit's full corner list pinned; any attempt-order/tie restructure then flips either the verdict or the circuit. Banked for a future capture round. |
| ForcedPadRouter → TraceShover.check depth−1 edge (spec-review survivor (e)) | HONEST BANK | `check_forced_pad`'s piece loop re-enters `trace_shover::check` with `max_recursion_depth - 1` (`forced_pad_router.rs:420`, Java `:329`); the decrement is UNPINNED — no world consumes the budget exactly at this edge. Jar budget sweep (`ForcedPadFrontProbe` part 2, `pad_front_rows_run1.jsonl`): a pad box over ONE inserted foreign trace answers NOT_DRILLABLE only at budget 0 (the `:286` PRE-LOOP gate — before any piece-loop recursion) and DRILLABLE for budgets 1..10, because a single-trace chain consumes ZERO recursion (the nested check succeeds at depth `B-1` with nothing left to probe). Chains deep enough to make the `:420` decrement observable are foreclosed by the `stackDepth() > 1` gate (`:378`) — the t2 precedent. Discriminating-world design (unbuilt): a forced-pad world whose re-entry chain needs EXACTLY one nested check level, driven at budget B=2, where Java answers FAIL at nested-depth 0 while the `−1`→`0` mutant succeeds at nested-depth 1. Banked: the decrement is Java-verbatim (`:329` read directly); only the PIN is missing. |
| Capture-commit deviation (spec-review MINOR-4) | DOCUMENTED, KEEP | The original T10b commit `149502d6` was the FIRST task to commit `logs/` captures (`git ls-files logs/` = exactly the four T10b files: trace_shover_rows ± `_run2`, shape_debt_rows ± `_run2`); prior tasks (T9/T10a) committed none, and the brief's header says keep `logs/` out of git — the implementer's "per the brief's explicit path list" justification does not hold. Disposition KEEP (spec review): the pins are SELF-CONTAINED literals (no test-time reads of `logs/`), so the files are provenance, not a test dependency. The fix-round capture pairs (`spring_nest_rows_run1/_run2.jsonl`, `pad_front_rows_run1/_run2.jsonl`) stay UNCOMMITTED per the standing fix-round rule (nothing new into `logs/` in git); do NOT `git rm --cached` the four committed files. |
| Mutation round (spec-review fix round) | F1 + F2 KILLED | F1 springOver containment predicates → Java direction (the spec review's correction mutant at `trace_shover.rs:851`): killed by t12's DEPTH-1 arm — at the public depth-20 driver both forms converge on the same pin circuit (the wrap recursion HEALS a wrong inner-first pick; first t12 draft asserting only the final geometry was blind — pin failure mode 12, logged), so the pin drives `spring_over` at recursion depth 1 where the fixed form wraps the pin directly and the swapped form burns the budget on the inner USER_FIXED via and fails when the pin surfaces in the depth-0 recursion. F2 six `in_front_of_pad` sum-of-mins/maxes sites → Java min/max-of-SUMS: killed by the two side-6 pins (`t9_in_front_of_pad_side6_min_of_sums` on the (1,−1)-slope main-2nd-disjunct face, `t9_in_front_of_pad_side6_ws_max_of_sums` on the withSides conj-2 max face — jar `pad_front_rows` rows `main6_hit`/`ws6_trap`), each pin killing exactly its own reverted face. |

## T10c forced-insertion board surface + pull-tight seam (appended by M3-T10c)

The `RoutingBoard` insert surface landed as
`epic-board/src/routing_board_insert.rs`: `checkForcedTracePolyline`
(`:408-448`), the 15-step `insertForcedTracePolyline` (`:456-876`, the
return-null damage contract mapped to `None` at both Java arms plus the
`:756` unguarded-deref NPE — module docs, "The return-null contract"),
`connectToTrace` (`:1116-1170`), `removeTraceTails` (`:1193-1238`, the
`(−1, option)` form + the `Item.isFanoutVia` protection walk), the
changed-area session trio (`RoutingBoardOperations :26-50`), the
`optChangedArea` skeleton (`:52-79`) behind the [`PullTightSeam`] trait
with the documented no-op `NoPullTight` (the M4 `TraceTightener` is NOT
ported; `TraceTightener.getInstance` is projected as the clamped
`PullTightAlgo` value and `splitTracesAtKeepPoint` `:474-491` IS ported
as a pick+split driver), plus `BasicBoard.checkPolylineTrace`
(`:1054-1075`) and the pick helper. `ChangedArea` →
`epic-board/src/changed_area.rs`; the `board.changed_area` field and the
four T10b stub seams are now WIRED: DrillItemMover.insert old-box corners
(`:159-162`), TraceShover.insert piece corners + the DIRECT-DEREF
normalize (skip-when-no-session, run-with-clip when marking — the
T10b-era null face was the bug-144 bank), ForcedPadRouter piece corners +
the NULL-CLIP unconditional normalize (`:440-450`), PolylineTrace
combine start/end corner joins (`:328-330`/`:452-454`), and
`insertTrace`'s clip extraction (`:223-229`). Oracle:
`rust/harness/oracle/ForcedInsertProbe.java` — 58 JSONL rows over six
worlds on net NUMBER 94 (the NAME N094 has number 95 on this fixture —
the native fail-row gate compares numbers; probe selects by number),
double-run byte-identical, captures
`logs/M3-T10c/captures/forced_insert_rows_run{1,2}.jsonl` (committed
per the dispatch's explicit path list, T10b-original precedent). 12 pins
(t10c_*): the insert success ladder (ids, moved via, wrapped substitute
piece, merged trace, L0+L1 octagons field-by-field), the tidy-width-400
no-op-seam equality face, the clamp pin, the connect ladder (contains /
check-blocked / insert+destructive tail walk), the c2 id-stream
watermark pin, the fail world (undamaged inventory + active-empty
session at the ±CRIT_INT sentinels), the bare-check faces, the tails
3-way option ladder + the net-0 all-nets face, and the
`opt_changed_area` teardown skeleton.

| Seam / semantics point | Status | Detail |
|---|---|---|
| The checkPolylineTrace id burn | PINNED (bug 150) | Java's check constructs a tmp PolylineTrace it never inserts — and `Item.java:87` assigns the id in the CONSTRUCTOR — so every check call burns one generator id. The capture's `after_c2 new=[115]` is unreachable without it: c3's failed check burns 113, c2's check burns 114, the connection = 115. The port burns explicitly (`board.alloc_id()` at the top of `check_polyline_trace`); the stream watermark after c2 is 117 (the connection's normalize splits target 112 into transient pieces 116/117 — `split(int,Line)` removes the original and re-inserts both pieces — then the tail walk removes them), pinned by `t10c_c2_id_stream_transient_split_pieces` against a throwaway oracle `c2debug` world (manual c2 steps with inventories between). Java allocates identity at construction, so "read-only" checks advance the global id stream — the waste is part of the behavior. |
| checkTraceShape obstacle-net polarity | PORTED-VERBATIM; PORT BUG FIXED (bug 149) | The per-net loop is Java's double negation: `isTraceObstacle(net) = !containsNet(net)`, so `if (!isTraceObstacle) isObstacle = false` clears the flag on OWN-net entries. The first port had the predicate inverted (own-net traces flagged, foreign passing) and SURVIVED T10b because `calc_from_side` passes an empty net array — the loop body never ran. The c2 insert face (a connection into the own-net target) was the first discriminating input. Fixed + de-instrumented; killed by the connect-ladder pin. |
| pull-tight seam | NO-OP (M4), DIVERGENCES LEDGERED | `PullTightSeam::opt_changed_area` / `pull_tight_trace` are no-ops behind the trait; `opt_changed_area`'s skeleton (null-session early-out, `clipShape != EMPTY` gate, update-box drop, teardown) is verbatim and pinned (teardown/early-out faces). Named divergences while the seam is a no-op (module docs): post-route corner coordinates (Java's pullTight moves corners inside the tidy region), tightener split id deltas (`after_pull_tight` rows have no Rust counterparts), and the `tidyWidth == Integer.MAX_VALUE` trap (the pull-tight CALL fires even when only the region construction is skipped — the port keeps the call-site gate verbatim). The EMPTY-gate inversion mutant is unobservable through a no-op seam — M4 pins the seam arm. |
| Mutation round (board surface) | 11 killed this segment + 3 pre-compaction / 1 banked | This segment (each edit→run→expect-fail→revert-by-editing-back): M4 check-burn drop (ladder + stream watermark 117→115), M5 tails VIA-skip drop (via-spares pin), M6 fanout-protect drop (fanout pin), M7 connect contains→false (c1 pin), M8 connect Unfixed→UserFixed (fixed-state pin), M9 connection changed-area join drop (after_c2 octagon growth), M11 `get_instance` clamp drop (dedicated clamp pin), M12 tails net gate `<= 0`→`< 0` (the net-0 face — a structural pin, the capture worlds only call −1), M13 `opt_changed_area` teardown drop (skeleton pin), M14 split_at_line refuse-all (stream watermark — the transient-split discriminator the live inventory cannot see). Pre-compaction: M1 shorten skip-arm target, M2 projection `<`→`<=` tie, M3 skip_lines guard. BANKED SURVIVOR: M10 the pre-insert corner-marking loop drop (`:747-750`) — in the captured world its octagon contribution is fully SUBSUMED (the from-corner (500000,300000) is also joined by the combine-at-start site `PolylineTrace.java:329` — a T10c-wired join, not the `:749` loop itself, which sits INSIDE the dropped loop; the to-corner (502000,300000) is inside the substitute piece's 502217 extreme), so the L0/L1 pins cannot see it; the site is wired and 3 lines verbatim. Discriminating world (unbuilt): a bent polyline whose middle corner exits all other join contributions. |
| Return-null damage faces | HONEST BANK | Both Java `return null` arms (`:616-618` main-loop insert fail, `:742-745` sampling-retry insert fail) require the shove INSERT to fail AFTER the check ladder passed and the board was already mutated — no pinned world reaches them (the W3 fail world fails at CHECK: board untouched, `Some(from_corner)`). The port maps both to `None` and the `:756` NPE onto `None` too (module docs). Banked: a discriminating world needs an insert/check race inside the shover (foreclosed by the check-first ladder at every reachable budget). |
| Capture-commit note | COMMITTED PER DISPATCH | `logs/M3-T10c/captures/forced_insert_rows_run{1,2}.jsonl` (58 rows each, `cmp`-identical, md5 1ee2e66b5be2e92f6e3a1103008b6aa4) are committed with this task — the dispatch's explicit path list overrides the `/logs/` ignore (same as the four T10b-original files). The throwaway `c2debug` world stays probe-side only (NOT part of the committed capture rows; the run output was consumed interactively to ground bug 150 + the watermark pin). |
| Java-over-brief deviations | 2, both documented | (1) `projectionLine` lives on `Polyline` (`Polyline.java:871-910`), not `LineSegment` as the brief named it — the Java placement wins (`epic-geometry/src/polyline.rs`), with the strictly-closer candidate walk (first-minimal wins ties) and the exact `sideOf` containment filter pinned. (2) `insertTraceWithoutCleaning` returns `None` where Java returns null on a degenerate polyline — the port's `?`-propagation surfaces it as `None` from `insert_forced_trace_polyline` (the same damage contract). The board `updateBox` graphics join (`:77`) is computed-and-dropped (no GUI observer surface headless). |
| Spec-review-1 fix round | 3 pins added + 2 banks sharpened | Review findings 1–3 were observability gaps; all three now pinned (no production-code changes). (MINOR-1) `insert_retry` probe world: the sampling-retry SHORTEN arm observed on SUCCESS — straight 400000 corridor, blocking via at 560000, via budget 0; the shape-0 check fails on the via, the last segment (400000) sits strictly between sampleWidth (20000) and 100×sampleWidth, the shorten arm clips to (520000,300000) and the re-check PASSES, so the SAMPLED corner is the return value + the inserted trace 106 (the W3 fail world fires the same arm but discards the corner when its re-check fails). Pin `t10c_retry_shorten_arm_samples_the_corner` replays the capture (`forced_insert_rows_fix_run{1,2}.jsonl`, cmp-identical, md5 b5ee688bc8eb198634ff16c9de272c46); kills `2*`→`1*` (sampled corner → 510000), the shorten-arm drop (re-fail → fromCorner, no trace), and a 10× cap (too-many-cycles arm at 400000 > 200000). (MINOR-2) `drill_move` probe world: `DrillItemMover.insert` (Java `:110-167`) under a LIVE session — via 105 (single-layer padstack) moved +4000 through the N002 trace; forcedPad shoves it (re-formed as 109), the piece corners join (`:434-436`) and the untranslated box corners join (`:159-162`); the two webs sit ~4000 apart so the pinned octagon (box leftX 499750 vs piece rightX 504367) discriminates either join dropped INDEPENDENTLY (kills review mutant R11 + the drill-join drop). Clip-normalize sub-face (`:440-450`) is EXERCISED (runs with the live clip) but the clip-vs-unbounded distinction is UNOBSERVABLE in this world — every piece corner lies inside the just-joined web, so normalize(clip) == normalize(None) here (mutant probe applied, survived, reverted): discriminating world (unbuilt) needs a normalize-active corner outside the join web (an absorbed neighbor's far corner that unbounded pull-tight would move and the clip pins). (MINOR-3) `t10c_nested_start_keeps_the_live_session` pins Java `RoutingBoardOperations :27-29` (second start keeps the marker; kills the R8 overwrite mutant directly). NIT-1 cite (M10 bank → `PolylineTrace.java:329`) and NIT-2 normalize-face comment reworded per the review. |
| Quality-review-1 M-1: split-at-keep-point + re-pick | BRANCH PINNED; pick-order sub-face BANKED | New `keep_point` probe world (gate `m1run`, capture `forced_insert_rows_m1_run{1,2}.jsonl`, 72 rows each, cmp-identical, md5 14c572497cb1d840e04cb8a88a8ccc48) + pin `t10c_keep_point_split_splits_and_re_picks`. World: bare fixture + net-94 seeds — a vertical crossing at (502000,300000) (forces `normalize result=true`: split_clip found-splits it 105→108/109 AND own-splits the merged trace 107→110/111, so `result = pieces != 1`) and a COLLINEAR continuation (504000,300000)→(508000,300000) — the `:756` combine merges it (`combined=true`), putting the keep point in the merged trace's INTERIOR, which makes `splitTracesAtKeepPoint` a REAL split (111→112+113, capture row `split_at_keep idBefore=111 idAfter=113 delta=2`) instead of an end-corner no-op; the re-pick then runs (`pickedAtEndCorner=2`). Pin kills: normalize-result forced-false (branch deleted → 111 whole) and the split-call drop (re-pick runs over the unsplit 111). PICK-ORDER SUB-FACE (Q5) honestly BANKED, not killed: with one interior trace at the keep point the pick order is forced, and the two-candidate design is FORECLOSED — normalize's found-splits pre-split every same-net crossing ON the inserted polyline, and an end-abutting second candidate is combine-eaten at `:756` (the M-2 masking mechanism); the order only becomes observable via M4's pull-tight seam (the re-picked value's only consumer, dead at tidyWidth 0). |
| Quality-review-1 M-2: picked-trace combine-candidate gate | BANKED SURVIVOR (Q3) | Java `:508-517` gate (`pickedItems.size()==1` + netsEqual/halfWidth/class) unpinnable in every committed world: its only effect is the `start_shape_no` shape-set shift (picked 4−3=1 vs unpicked 3−3=0), and the final merge happens anyway through the trailing `newTrace.combine()` (`:756`) — combine-at-end MASKS the gate. Discriminating world (unbuilt): one where the picked trace's polyline changes the offset-shape set (`startShapeNo`) so a shape that the UNPICKED path never checks becomes the checked shape AND its dog-ear-cut offset flips a shove verdict (check ok → insert, vs check fail → lastShapeNo/return fromCorner); needs a from-corner abutment within a shove-distance of a foreign obstacle. Bank stands until T11's FoundConnectionInserter worlds exercise picked-corner geometry under obstacles. |
| Quality-review-1 M-3: shove-loop entry-side `+ i` | BANKED SURVIVOR (Q1) | Java `:567-571` `cornerCount - traceShapes.length - 1 + i` ShapeEntrySide index — every committed world's shove verdicts are entry-side-blind (straight/empty from-corners: both entry sides give the same first-shove target). Discriminating world (unbuilt): a from-corner abutting a FOREIGN trace at a dog-ear where entering along shape 0's side shoves the foreign trace one way and shape i's side the other — the first check verdict (and thus `lastShapeNo`/insert-vs-retry) flips with the increment dropped. Same family as the T10b springOver entry-side pins; T11's routed-fixture worlds are the natural habitat. |
| Quality-review-1 M-4: connect tail-walk witness | PINNED (one-assertion fix) | `t10c_connect_ladder_contains_blocked_insert` now asserts the capture `after_c2` inventory directly: `ids.len() == 107` (104 fixture + [107,111,115]) AND `!contains(&116) && !contains(&117)` — the transient normalize-split pieces of target 112 that the end-corner tail walk removes. Both review mutants die: protection inverted (`!user_fixed`→`user_fixed`, inventory 109≠107) and removal disabled entirely (same 109≠107) — before the fix both survived because `!contains(&112)` was already true from the split itself. The SEAM ladder row's "insert+destructive tail walk" claim is now true. |

## T11 autoroute engine assembly (appended by M3-T11)

The connection-routing engine landed as `epic-router/src/engine.rs`
(3189 lines incl. pins): the room registry (live `rooms`/`keys` +
graveyard keyed at `ROOM_KEY_BASE`), the drill-page array, the
`NeighbourEngine`/`DrillEngine`/`CompleteShapeObjects` production impls
(T5-harness bodies lifted onto the manager's autoroute tree + the board
warm cache), `autoroute_connection` (Java `:131-282`), and the
`init_autoroute`/`finish_autoroute` lifecycle faces (Java
`RoutingBoard :882-905`). ASSEMBLY CALL GRAPH (one route attempt):
`init_autoroute` → `init_connection` (net-switch room purge +
`additional_update_after_change` per net item) →
`autoroute_connection`: maze (`MazeSearchEngine::new` +
`find_connection_between`) → raw maze-result row (nets {33,66,67} gate)
→ `locator::get_instance` (T9) → cleanup (`clear()` when the database
is not maintained, else `reset_all_doors()` — pages only) →
post-locator layers-disabled gate ×2 → ripup removals
(`get_connection_items` + descending-id `remove_item_through_repository`
+ `remove_trace_tails` per changed net) → `inserter::get_instance`
(T10: per-result-item `insert_via` + `insert_trace`; insert_trace walks
segments through `insert_forced_trace_polyline` — id-delta row,
ADVANCE/VIOLATION_CORRECTED/FAIL ladder, neckdown + fanout micro arms —
then the stub-cleanup walk and the per-net
`normalize_traces_of_net` tail) → `Routed` / the three FAILED literals.
Observers are a NO-OP SEAM: Java brackets the ripup removals and the
insertion with `startNotifyObservers`/`endNotifyObservers` when no
observer is active; the Rust board has no observer system, so the
bracket is absent (log-only face, no state difference in a headless
run). PIN COUNT (spec-review F2/F3 corrected): 23 NEW at the assembly
commit `d9b216905` (19 epic-router + 4 epic-board search-file) — the
commit message's "24 t11 pins (5 epic-board + 19 epic-router)"
double-counts the pre-existing T10b forced_via t11 test (landed
`149502d63`, before baseline `8b9a70d5e`); arithmetic: baseline
**1136** + 23 = **1159** (the final report's "baseline 1135 + 24" was
wrong on both operands; root: the dispatch's own line-136 "1135"
typo). The fix round adds 6 more (5 epic-router neckdown/span-gate +
1 epic-board suppression-read — rows below), so the T11 family totals
29 t11-named pins + the suppression pin, workspace 1159 → 1165. The
quality-review round adds 2 more engine pins (ripup walk + reuse
faces — rows below) → 31 t11-named + suppression, workspace
1165 → 1167, and lands the T11 family's FIRST production change: the
`init_autoroute` slot parameter was UNCALLABLE (E0499 for every
possible caller — the slot engine holds the very `&mut` borrows the
call re-lends), reshaped to the borrow-free rebuild arm + the
`is_reusable_for` condition method (row below).
Capture `logs/M3-T11/captures/autoroute_engine_rows_run{1,2}.jsonl`
(205 rows each, cmp-IDENTICAL, md5 31286da6f8e8914008a79658d14f0c10),
oracle `rust/harness/oracle/AutorouteEngineProbe.java` (11 worlds over
the t9_locator45 fixture).

| Seam / semantics point | Status | Detail |
|---|---|---|
| Tightener-stable fixture evidence (the mandated method) | EVIDENCED + TRAP DOCUMENTED | The mandated before/after pull-tight row check on the corridor world is FIELD-EQUAL (rows carry only pickedAtEndCorner + newCorner) while the geometry PROVABLY diverges — row-equality is a WEAK SIGNAL here (trap finding; bug 151). The positive attribution: the jar tightener run on the Rust NoPullTight staircases (probe `pulltight0/1` worlds) collapses the layer-1 staircase EXACTLY to the jar canon 121 `(480738,309375)(662957,127156)(662957,29188)` and the layer-0 7-corner staircase to a DIFFERENT 45° local optimum `(620000,300000)(490113,300000)(480738,309375)` (canon 116 is `(620000,300000)(610625,309375)(480738,309375)`) — pull-tight is PATH-DEPENDENT (per-segment insert history), no normalize/combine step can produce those reshapes (`Polyline.removeConsecutiveParallelLines` only drops consecutive parallels). Per the dispatch's adjust-the-fixture arm, the STRAIGHT-CORRIDOR world (vertical anchor (663500,26000)-(663500,28000) north of net 94's pin) forces a straight optimal path where pull-tight is a NO-OP: `t11_straight_corridor_canon` pins the FULL board canon byte-exact vs the jar (`route_straight94:done`: ROUTED, one ADVANCE segment, normalize absorbs piece+anchor into trace 109 `(663500,28000)(663500,20000)`, before/after pull-tight rows equal, removed_stubs=0). The corridor world's canon is pinned to the NoPullTight-verdict SUBSET (structure + ids + row-verbatim geometry + jar-exact pin leg), with the tightener-attributed geometry delta ledgered below. |
| Event-row surface | PINNED | `compare_trace_maze_result_raw` (nets {33,66,67} gate — `t11_maze_result_row_gate` pins the net-33 row verbatim `net=33, section=0, destination_type=TargetItemExpansionDoor` vs the probe capture AND the closed gate on net 94 via the end-to-end pin; kills the gate-off and type-name-swap mutants), `compare_trace_connection_item_raw` + `compare_trace_insert_segment_raw`/twin + `compare_trace_insert_segment_ids` (straight-canon pin asserts the four rows jar-verbatim, incl. the delta=5 id burn), `compare_trace_stub_found`/`cleanup` (row text pinned; removal face banked below), `FANOUT_DIAG` (T10a surface). The jar emits all rows through FRLogger; the probe tap re-emits in order — run1/run2 byte-identical. |
| Init-failure message collapse | BANKED (jar witness) | Java distinguishes `MazeSearchEngine.getInstance` null ("…because the maze search algorithm could not be created.") from `findConnection` null ("…because no connection was found between their nets."); the Rust maze folds both into `None` and `autoroute_connection` always emits the no-connection literal. JAR WITNESS: probe world `route_debug49` FAILED with the init-failure literal on this very fixture ("Failed to route connection between pin of component #48 and polylinetrace, because the maze search algorithm could not be created.") — the literal EXISTS in the jar's reachable surface; the collapse costs only the details string of a degenerate init (state + downstream handling identical). |
| Layers-disabled gate + maze starvation | BANKED-BY-INSPECTION (dead arm) + PINNED starvation | The post-locator gate (Java `:221-222`) is dead-defensive in BOTH engines: the maze reads `ctrl.layerActive` (`MazeSearchEngine :396-397/:478/:493`, `DestinationDistance :57-63`) and starves first, and the batch router pre-checks (`AutorouteConnectionRouter :189`). `t11_layer_active_faces` pins the starvation (FAILED no-connection literal, board unchanged) and the unused-layer face (layer 1 inactive still routes — the gate consults only the FOUND connection's layers). The gate's own FAILED literal is pinned by inspection only (unreachable through any engine). |
| startInfo persistence face | PINNED | Java's `initConnection` net-switch walk covers only the NEW net's items, so a foreign net's start infos legitimately survive; `t11_get_rooms_with_target_items_descending_and_net_switch` pins the pin's start info across the 94→2 switch (plus the net-dependent room purge — kills the purge-drop mutant). |
| Descending room-id order (`get_rooms_with_target_items`) | BANKED (order-degenerate world) | The sort mirrors Java's TreeSet (`other.id - this.id`). The fixture world yields exactly ONE target room (debug: `target_rooms=[7]` of 182 keys), so the sort-drop mutant SURVIVED (order trivially forced). Discriminating world (unbuilt): two complete free-space rooms with target doors to the anchor in non-descending creation order — requires shaping the completion around the anchor to emit two rooms. |
| Insert-failure arm (`inserted.is_none()` → FAILED "could not be inserted") | BANKED SURVIVOR | No engine-level world reaches it: the locator only returns connections whose pieces the T10c insert surface inserts (the insert/check ladder was pinned there); forcing an insert failure needs a check/insert race inside the shover (foreclosed at every reachable budget — the T10c return-null bank's same argument, one level up). Mutant (arm → fall through to Routed) survived all 24 pins. Discriminating world (unbuilt): a ctrl/board combination where `get_instance` returns None after the locator succeeded (e.g. an insert_via failure at a layer change — needs a keepout-ringed via site on the START item). |
| Stub-removal face | BANKED (fixture-unreachable) | The stub-cleanup walk (Java `:405-431`) removes NOTHING in any world — including the jar's own (all captures: `removed_stubs=0`, zero `stub_found` rows). Dropping the removal (`remove_item_through_repository` + counter) SURVIVED all pins. Discriminating world (unbuilt): a previously-routed leg ENDING at a corner the new route passes THROUGH as an intermediate corner — `get_trace_tail` finds the dangling tail there (the jar's own worlds never leave one on this fixture). The row surface IS pinned (`t11_straight_corridor_canon` cleanup-row text). |
| Insert-tail normalize (`:423`) | BANKED (fixpoint-subsumed) | Dropping the connection-tail `normalize_traces_of_net` SURVIVED: the per-segment normalize inside `insert_forced_trace_polyline` (T10c) has already reached the fixpoint by the tail's turn, and the stub walk removed nothing (row above), so the extra pass is provably a no-op — `t11_board_is_normalize_fixpoint_after_route` pins exactly that exit-state fixpoint (and KILLS the per-segment normalize drop C1, which fails 3 canon pins). Java keeps both callsites; the port too. |
| Keep-point split endpoint no-op | BANKED (endpoint-degenerate, jar-confirmed) | Dropping `split_traces_at_keep_point` from the forced-insert tail SURVIVED all t11 worlds: the keep point (= the piece's end corner) lands at a COMBINED-trace endpoint in every one of them, where the split no-ops — the JAR confirms the same no-op in its own splitcombine world (one merged trace, `pickedAtEndCorner` follows). The interior-keep-point face is already T10c-pinned (`t10c_keep_point_split_splits_and_re_picks`, the M-1 world puts the keep point INSIDE the merged trace). |
| Tightener-attributed geometry delta | LEDGERED (bug 151) | The corridor canon pins the NoPullTight-verdict subset; the jar's tightener reshapes traces 115/121 (evidence above). M4's TraceTightener port closes the delta; until then `PullTightSeam::pull_tight_trace` stays the documented no-op (T10c bank) and the straight world is the Java-identical surface. |
| checkTraceSegment port | PINNED TO JAR LITERALS | `epic-board/src/routing_board_search.rs` (561 lines): the point-pair + segment faces, the shortened-length arithmetic (offset/projection/clearance-or-compensation addend/`-1` margin/early zero), the `onlyNotShovableObstacles` arm. `t11_check_trace_segment_jar_literals` replays the probe `check_segment` world's five literals EXACTLY (`2147483647.0`, `2483.0`/MAX_VALUE flag-split, `7483.0`/`7483.0`). |
| Mutation round (engine assembly) | 22 killed / 6 banked (28 applied) | KILLED: A1 cleanup-arm inversion (4 pins), A3 net-purge drop, A4 start_infos.clear drop, A5 time-limit arm drop, A6 finish-without-clear, A7 describe join ", "→"+" (the multi-element face added this task — the original single-element pin was blind), A9 maze-row gate off, A10 type-name swap, A11 ROOM_KEY_BASE shift, A12 ctor net -1→0, B3 ADVANCE window reset (3 pins), B4 45↔90 angle swap (end-to-end canon), B5 insert_via drop (end-to-end canon), B6 id-delta swap (straight canon), C1 per-segment normalize drop (3 canon pins), D1 equal-corners return drop, D2 ok_length sentinel 1e9 (3 pins), D3 `-1` margin drop (literals), D4 not-shovable flag neuter (2 pins), D6 nearest-point basis swap (literals), E4 insert_trace verdict inversion (9 pins), E5 maze start/dest swap (3 pins incl. both canons). BANKED: A2 (order-degenerate, above), A8 (insert-failure arm, above), B1 (fixpoint-subsumed tail normalize, above), B2 (stub removal fixture-unreachable, above), C2 (endpoint no-op, above), D5 early-zero-return drop — EQUIVALENT mutant: the `.max(0.0)` clamp guarantees the same return value and the loop is read-only, so the early exit is a pure optimization in Java too. |
| Suppression-set faces (spec-review F1) | READ PINNED (injection) / WRITE BANKED | The review's R1 (short-circuit removal) and R2 (add-on-cap drop) survived while `inserter.rs` claimed the add-on-cap face pinned — the false claim is deleted (replaced with the truthful pointer). READ face (Java `:714-725` short-circuit before any walk) now pinned by direct injection: epic-board `normalize_all::tests::suppressed_net_short_circuits_before_any_walk` latches net 1 (the set is `pub(crate)`), asserts result=false + the unfolded chain + trace-untouched literals, with an UNLATCHED positive control folding the same geometry (kills R1; not observability-vacuous — cerebrum mode 10 honored). WRITE face (the `:748` add-on-cap latch) stays BANKED: no crafted geometry reaches the 2000-iteration cap — every split/combine/degenerate candidate converges (the T12 depth-cap argument), and reviewer R3's call-site `>` comparison (`:261`) is the same unreachable face (helper-pinned only — OBS-1, folded here per the review). Discriminating world (unbuilt): a collinear chain engineered to oscillate split↔combine past 2000 iterations — no construction found; revisit only if a corpus compare ever shows a normalize oscillation. |
| Neckdown + micro ladder (spec-review F4) | PINNED (fix round, 4 worlds / 5 tests) + 4 REMAINING BANKS | `with_neckdown` was false in EVERY pre-fix world (R5/R8 survived undisclosed). Fix round (all mutants applied by edit, reverted by edit-back, kills verified): `t11_neckdown_start_pin_narrows_blocked_target_to_pin_halfwidth` — the start-pin arm end-to-end: full-width forced insert FAILS all-or-nothing at an endpoint-shadowed target (okPoint = the from corner), the arm fires, the final narrowed insert (the fixture pin's `pin_trace_neckdown_halfwidth` = 499 for the 1000-wide pad) lands WHOLE back to the pin; kills the verdict comparison (`==`→`!=`: all 3 neckdown worlds flip) and the OUT-OF-SPAN `pin_is_on_layer` answer (inversion killed). `t11_neckdown_distance_gate_open_side_narrows_to_stop_point` + `t11_neckdown_distance_gate_boundary_rejects` — the `>=` distance-gate STRADDLE: fixture gate value 2·(0.5·1000+2516) = 6032; a 3-corner item whose second window fails on an endpoint shadow hands the arm ok=M; M at 6031 narrows to the stop point (verdict true, hw-499 + hw-1500 traces), M at EXACTLY 6032 refuses (FAIL row `fromCornerNo=1`, no 499 trace, stub cleanup removes the stranded full-width leg — the board ends net-94-empty); the `>=`→`>` mutant opens the boundary world (narrowed lands at 4000 ≥ 3115) — killed by the verdict/row/trace asserts. `t11_fanout_micro_neckdown_candidate_order` — the micro candidate ORDER over a 3642 gap: candidates [1125, 900, 750] (pin widths absent: `with_neckdown` false keeps start_pin/end_pin None, fractions only), 1125 passes by 1 unit; kills BOTH the reorder (3/4↔3/5 → 900 wins) and the fraction literal (3/4→4/5 → 1200 fails, 900 wins) via the `candidate_half_width=1125` row literal + the hw-1125 trace. REMAINING BANKS (each verified SURVIVING post-fix, disclosed): **R5** horizontalFirst `>=` tie (`:852`) — needs a MIDDLE ok_point (a multi-segment window whose earlier segments land); windows wider than 2 corners arise only through VIOLATION_CORRECTED re-windowing, whose first segment deterministically re-fails (zero progress) all the way to the FAIL — the legs path is construction-unreachable through `insert_trace`'s ADVANCE chain; bank stands. **R8** halfwidth floor `.max(1.0)` (`pin.rs:107`) — fires only for minWidth < 4 (zero-area pad); no fixture padstack is that thin; design (unbuilt): a crafted 2-unit circle padstack. The neckdown hw-gate `>=` (`:815`, refuse when neck_down_halfwidth >= trace_half_width) — both literals false for this fixture (499 < 1500); needs a pin with minWidth ≥ 2·(base+1) = 3002 (bm08's 15240 oval qualifies; different fixture, unbuilt). The IN-SPAN `&&` of `pin_is_on_layer` — `&&`→`||` is true whenever `&&` is on a 2-layer fixture with full-span pins (every route layer satisfies one arm): world-impossible, verified surviving. QUALITY-ROUND ADDITIONS (OBS-Q6, both applied + surviving this round): **the `try_neck_down` sentinel face** (`inserter.rs:837`, quality Q5: `ok_length >= f64::from(i32::MAX)` → `>`) — the "already-clear window → return try's FROM corner" face needs a SHOVABLE obstacle with the forced insert failing at ZERO shove budget; at any reachable world the mutant CONVERGES (the ladder's far-point insert fails at the same shadowed endpoint and returns the same try-from corner), so this is bank-with-caveat: the discriminator would be an ok_length in [2^31, 2^31+1) — constructible only by an obstacle window wider than the board diagonal; **the single-point arm's `first_corner` bookkeeping + `cornerCount=1` row** (`inserter.rs:440-449`, quality Q11: arm drops the `first_corner` assignment) — needs a 1-corner ResultItem in the located connection (a degenerate trace whose corner iteration yields one corner — the T9 FromPointAlias surface); no engine world produces one (every located trace has ≥2 corners); discriminating world (unbuilt): hand-assemble a `FoundConnectionLocator` with a 1-corner item and drive `insert_trace` directly, asserting `first_corner == last_corner == that corner` plus the `cornerCount=1` row. |
| insert_via span-gate rejection (spec-review F5) | PINNED (fix round) | A via rule whose only entry references PADD (single-layer F.Cu padstack, library no. 6) cannot cover the 0→1 span: `t11_insert_via_span_gate_rejects_missing_span` asserts verdict=false, ZERO id burn, the `via mask not found for net #94 covering layers 0 to 1` debug, and the gated `via_mask_not_found` FANOUT_DIAG with the full field set (`pin=U27-P1, fromLayer=0, toLayer=1, location=(700000,30000), …`). The gate-inversion mutant admits PADD and answers TRUE (insert succeeds) <!-- codespell:ignore (PADD is the fixture padstack name) --> — verified killed by the verdict assert. OBS-2 (R9, the layers-disabled detail literal) stays as reviewed: defensive arm, documented unreachable through composition, the starve face pinned instead — no action. |
| Ripup-removal block (quality F-Q1) | PINNED (presence + tail walk) / ORDER + ACCUMULATION BANKED | `t11_ripup_removal_walk_removes_listed_connection_and_net_tails` seeds a caller-owned `ripped_item_list` (the locator only INSERTS into it) with a foreign net-2 trace far off the canon corridor and routes the canon world: the LISTED trace vanishes (block-drop mutant KILLED — applied, listed survives), the UNLISTED lone net-2 tail is swept by `remove_trace_tails(2, None)` off the listed item's changed-net set alone (tail-sweep-drop KILLED — applied, unlisted survives while listed is gone; the sweep's `nets.len()==1`+routable candidate filter is what keeps the pre-existing net-2 fixture item id 6 alive). BANKED faces, all applied + surviving this round: **accumulation-drop** (`get_connection_items` → `ripped_connections` skipped) is SWEEP-SUBSUMED in every single-net trace world — any open chain dies from its 1-contact ends through the sweep's own per-stub connection walk, so the mutant is board-final-state-equivalent there; discriminator (unbuilt): a connection containing a MULTI-NET item (excluded from the sweep's `nets.len()==1` candidates, still walked by `get_connection_items`) — needs a via joining net 2 to a second net; **both `.rev()` order walks** (accumulation over descending keys, removal over the descending-id set; quality Q1 applied + survived) are board-final-state-INVARIANT — set removal is order-independent and the Rust board has no observer system, so the only discriminating observable is the UNDO-REPOSITORY record order (T14 machinery; `remove_item_through_repository` appends per call), i.e. a future undo-replay world, or T12's pass driver which exercises the block naturally. WORLD-DESIGN HAZARD (fixture-side lesson): the listed item's connection must never be a CLOSED LOOP — `getConnectionItems` has no cycle termination in EITHER engine (Java `Item.java:746-817` spins identically; `remove_if_cycle` is the only cycle-safe walk) and the pin hangs. |
| Locator-null FAILED literal (quality F-Q2; wording precision R-1) | BANKED (unreachable in the port; in Java reachable only via the swallowed `getInstance` exception — no production change) | Java's third arm (`AutorouteEngine.java:215-219`: locator null after a found search → plain `"Failed to route connection between X."`, no because-clause) is reachable in Java ONLY through the abnormal swallowed-exception path: `:178` null-initializes `autorouteResult` and `:180-191` catches any `FoundConnectionLocator.getInstance` exception (logged, absorbed), leaving the null the arm fires on — `getInstance` itself (`:192-194`) answers null ONLY for a null maze search, which the earlier `:206-213` arm already handled. The ported locator mirrors that: `locator::get_instance` returns None only through the `maze_search_result?` guard (`locator.rs:188`); on a Some search every path returns Some (bail arms carry null-ish FIELDS, not None), and the port has no exception swallow to reproduce — the arm is unreachable outright. The port folds both nulls into the single arm at `engine.rs:884`, which serves the reachable search-null face and therefore keeps the no-connection literal (emitting the plain literal there would DIVERGE the live face). Unobservable collapse; ledgered in the module-doc substitution bullet ("The init-failure FAILED row collapses") alongside the init-failure jar-witness row above. |
| `init_autoroute` reuse faces (quality F-Q3) | PINNED + SIGNATURE FIXED (the T11 family's first production change) | The reviewed fix ("a two-call pin init → route → re-init the SAME slot") is IMPOSSIBLE as the function stood: `slot: Option<AutorouteEngine<'a>>` + `&'a mut manager/board` in one signature is E0499 for EVERY possible caller (the slot engine holds the very borrows the call re-lends; verified by compile probe — T12's pass loop would have hit the same wall). Production reshaped along Java's real ownership structure (Java mutates the `this.autorouteEngine` FIELD): `init_autoroute` dropped the slot param and is now the borrow-free REBUILD arm (ctor + `init_connection`), and the verbatim `:888-897` reuse condition moved onto the held engine as `is_reusable_for(class, retain)`. `t11_init_autoroute_reuse_faces` pins: condition TRUE on the routed held engine (maintain + same compensated class) and honoring it carries the registry (`id_counter` and rooms survive a same-net `init_connection`); condition FALSE for retain=false and for a different class; the rebuild arm yields a fresh registry (`id_counter == 0`, empty keys) with the MANAGER carrying tree continuity (same class → same tree index; different class → a different tree). Both directions kill the retain-flip mutant (`retain_database &&` → `!retain_database &&` — quality Q2 applied, face 1 catches it: "maintain + same compensated class reuses the engine"). |
| Mode-2 row fields (quality OBS-Q5) | delta PINNED (trio) / section BANKED | `t11_autoroute_connection_routes_end_to_end` now asserts the three id-burn rows VERBATIM — `i=1 delta=5`, `i=2 delta=1`, `i=3 delta=0` with the chaining watermark 105→110→111→111 — so the constant-delta mutant (quality Q3, applied: killed at the i=2 row) and the before/after swap cannot satisfy the trio. The `section=` field stays at its coincident 0 in every world (the only gate-net row in the capture is net 33 → `TargetItemExpansionDoor`, section 0); a nonzero section needs a DRILL/fanout destination (the locator's fanout arm reads `section_no_of_door` for `target_layer`) — banked as the discriminating-world design for T12's pass worlds, which route to vias/fanouts naturally. |
| T12 readiness pointers (quality F-Q4) | DONE (doc-only) | (a) `is_stop_requested`'s doc now announces that T12's deterministic budgets (maxPasses/maxItems faces replacing the wall-clock `TimeLimit` — `logs/M3-T12/anchors.md`, the `:211-220` maxItems budget and the `:72-74` maxMilliseconds face) land on that same decision, limit-first order as the contract they extend. (b) The inserter id-burn site (`inserter.rs`, above `max_item_id_before`) now names T12's BoardHistory id-watermark work, the bug-150 `alloc_id` burn in `check_polyline_trace` as the other watermark site, and the `delta=` row as the capture-visible surface — the bug-150 ledger no longer lives only in `.wolf/buglog.json`. |
| Capture-commit note | COMMITTED PER DISPATCH | `logs/M3-T11/captures/autoroute_engine_rows_run{1,2}.jsonl` (205 rows each, cmp-identical, md5 31286da6f8e8914008a79658d14f0c10) committed with this task — the dispatch's explicit captures path overrides the `/logs/` ignore (T10b/T10c precedent). |

## T12 batch driver assembly (appended by M3-T12)

The T11 engine is now driven multi-pass: `epic-router/src/pipeline/`
(8 files, 6087 lines incl. pins) — `batch.rs` (1298: `BatchAutorouter`
+ `AutorouteBatchLoop` + `AutorouteUnroutedReport`: the item queue,
`BatchSettings`, `StopFace`, the pass loop with restore gate /
stagnation pair / task-state events, `format_score`, the unrouted
report), `pass_runner.rs` (1039: `AutoroutePassRunner.runSingleThread`
+ `RouterCounters` + the compare rows), `connection_router.rs` (921:
`AutorouteConnectionRouter.route` + necked retry + strict-DRC
enforcement), `board_statistics.rs` (1372: `core/scoring/
BoardStatistics` + the `DesignRulesChecker` score faces), `board_hash.rs`
(566: the modeled `BasicBoard.getHash` digest), `board_history.rs`
(775: `BoardHistory` + `restore_from_snapshot`), `event_sink.rs` (102:
the log/event sink), `mod.rs`. Support surfaces: epic-board
`failure_log.rs` (157: `RoutingFailureLog`, ON the board so restores
roll it back), `board.rs` (marking session, `reset_transient_after_restore`,
`undo_digest_walk`, undo level faces), `undo.rs` digest walk,
`clearance.rs` `item_clearance_violations`, `time_limit.rs`,
`trace_ops` `remove_trace_tails` publish; router `engine.rs`
(`RippedItemSeed`, `RouteBudget`, `take_budget`), `control.rs`
(`resolve_trace_costs`, pub `ExpansionCostFactor`), `maze/search_engine.rs`,
`drill/mod.rs`. PIN ARITHMETIC: 41 new tests (board_hash 4 +
board_history 9 + failure_log 1 + board.rs 1 + batch 9 + pass_runner 7
+ board_statistics 7 + connection_router 3), workspace 1167 → 1208.
Capture `logs/M3-T12/captures/batch-driver-probe.jsonl` (840 rows;
839/840 BYTE-STABLE + 1 known-racy family — see the capture-commit
row; committed md5 a9db46eae82119fd57cfd7dbbe494cbc),
oracle `rust/harness/oracle/BatchDriverProbe.java` — ONE JVM, 8
`runBatchLoop` worlds + `settings_witness` + `queue` over
t9_locator45.dsn, with the NamedAlgorithm task-state and
BoardUpdatedEvent counters streams rendered in the Rust row shapes.
Every Rust row literal validated EXACT against the jar: the queue rows
(order, ids 101–104, nets 98/33, "connected: 1/2"), the stage-start
row, the pass-completed row + `formatScore` suffix, the stagnation
pair + "Incompletes: 2 -> 2." recovery + the unrouted-report appendix,
the max-items row, the pass header + per-net incomplete rows, the
engine failure details, and the no-layers warn → CANCELLED(pass=0) →
IllegalArgumentException ORDER. Mutation battery: 17 valid mutants
applied, 17/17 KILLED (table below).

| Seam / semantics point | Status | Detail |
|---|---|---|
| Mutation battery (driver assembly) | 17 killed / 0 banked (17 applied) | M1 `log_ripped_items` `.rev()` drop; M2 ripped-cost default −1→0; M3 `render_counters` fanout field dropped; M4b items-remaining decrement moved after the failure row (M4a's inline-format form was syntactically invalid — replaced); M5 failed/routed tally swap; M6 stagnation `>=`→`>`; M7 global-best update dropped; M8 `fanout_recovery_applied` never set; M9 maxItems `stop.request()` dropped; M10 final CANCELLED event dropped; M11 maxPasses gate `>`→`>=`; M12 enforce-removal loop dropped; M13 strict gate `>=3`→`>=4`; M14 empty-queue returns `true`; M15 plane-skip `continue` dropped; M16 incompletes sum→`max_connections`; M17 mid-loop restore gate `>`→`>=` (killed via the stop-bypass arm in the max-items pin — equal 0.0 scores must NOT restore); M18 final best-restore gate `>`→`>=` (killed by the starved-local no-restore assert). |
| `getHash` string divergence | BANKED (modeled digest) + EQUALITY PINNED | Java digests ObjectOutputStream bytes (class descriptors, JVM map orders); the port digests a canonical little-endian encoding — the STRING never equals Java's. Not required by the parity contract: the hash feeds BoardHistory identity, stagnation text, and event rows only (the probe normalizes 32-hex → `<boardhash>`). What must match is the EQUALITY partition: `bh_deterministic_and_clone_stable` + `bh_item_encoding_field_sweep` + `bh_via_payload_sensitive` + `bh_itemlist_partition_insert_remove_undo` pin same-state→same / any-persistent-mutation→different, including the unreachable-slab exclusion (insert-then-remove hashes like the untouched board — Java's GC drops them too). |
| Restore model (bytes → clone) | PINNED (9 board_history pins) | Java's entry stores `serialize(false)` bytes and deserializes on restore; the port stores a `Board` clone (the value-semantics twin) + `Board::reset_transient_after_restore` for what deserialize does BEYOND copying (`normalizeSuppressedNetNos` cleared, changed-area/shove-failing transients re-inited — `board.rs` pin). The `Communication.idGenerator` NON-transient face — a restore rolls `max_generated_id()` back to snapshot-time so post-restore allocations re-burn discarded ids — rides the clone for free and is pinned. Gates pinned: `getMaxScore` floors at 0, add-at-cap strict-better-only eviction, `restoreBoard(<=0)` = unlimited, the in-place stable score-descending sort that changes `getRank` semantics, `contains` dedup by hash, `restoreBestBoard` = `restore_board(0)`. |
| Mid-loop restore gate reachability | PINNED (stop-bypass arm) / size arm BANKED | In the starved worlds the gate's `history.size() >= 8` arm never admits a restore: scores are constant so the strict `>` skips even when the modulo arm opens, and stagnation fires at 18 before any history entry could out-score. The stop-bypass arm (max-items world: the gate IS evaluated on a stop) reaches it with EQUAL scores — `t12_driver_max_items_stops_mid_pass` asserts no "Restoring an earlier board" row there, which is what kills M17. A deterministic world with a genuinely regressing pass (strict-better restore firing) needs an attempted route that RIPS UP earlier work and lands worse — non-constructible on this fixture (every starved attempt fails with zero expansion work; the completion world finishes in 2 passes); the face is Java-verbatim-ported and mutation-covered at the equality boundary. COVERAGE PRECISION (review X12/X13): "mutation-covered at the equality boundary" refers to the restore GATES (M17/M18), NOT the restore BODIES — the `Some(restored)` arm (`batch.rs`: the rank check + `restore_from_snapshot` call + score recompute + restore row) and the final best-restore body are UNREACHABLE-ON-FIXTURE in BOTH engines (zero "Restoring an earlier board" rows in all 840 capture rows — strict `>` can never fire on the identically-0.0 scores), so the bodies are covered only by the production-function unit pins (`bhe_restore_from_snapshot_model` through `restore_from_snapshot`), not end-to-end. This is the faithful mirror of a Java-side coverage gap — the Java capture cannot witness these rows either. |
| M4 fanout pre-pass stage | SEAM (documented no-op) — LIVE IN JAVA, DOSSIER CAPTURED | The gate is kept; the stage body (SMD pin collection, `BatchFanout`, recovery bookkeeping) lands in M4. JAR WITNESS: fanout is ON by default and the stage is LIVE in `AutorouteBatchLoop:135` — on the probe's completion_fanout_on world it placed 4 fanout vias (4/100 SMD pins escaped) across 2 fanout passes and introduced 32 clearance violations; that capture world is M4's dossier. REVIEW CORRECTION (spec MINOR-1): the earlier "NO Java world exists with gate on, stage no-op" claim was OVERSTATED — Java's `AutorouteBatchLoop.java:100-103` IS that observability world (fanout enabled + `getSmdPins().isEmpty()` → `logInfo("Fanout stage is enabled but skipped because the board has no SMD pins.")`), with the unconditional `logDebug("Checking fanout pre-pass. …")` check row at `:86-90` firing before any router-enabled gate. Both rows are now PORTED (`batch.rs`, same gate arithmetic; the `smd_pin_count` read mirrors `BoardItemRepository.getSmdPins` — on-board pins with `firstLayer()==lastLayer()`) and pinned by `t12_driver_fanout_pre_pass_rows` (world 1: all pins removed + router disabled via maxPasses=−1 → `smdPins=0` + both rows fire, proving the rows precede the gate; world 2 positive control: bare fixture → `smdPins=100`, the dossier's "4/100" denominator, + no skip row; mutants FM1 skip-gate `&&`→`||`, FM2 debug-row drop, FM3 info-row drop — all three KILLED). What remains structurally unachievable is exact-counter parity for the STAGE BODY on the completion world; `completion_fanout_off` is the row-parity mirror (it carries the job-ctor derivation `removeUnconnectedVias = !isFanoutEnabled()` = TRUE, whereas the Rust world's separate `remove_unconnected_vias` field stays false — a counters-tail divergence documented, not row-visible). Java leaves `fanoutExtraViasCount` NULL in autoroute-phase counter events; the Rust row renders 0 (port-side rendering of the absent field). The recovery GATE face (one-time, `>= 3` stagnant passes, `removeTails(None)`) is live and pinned both ways (M8; "Incompletes: 2 -> 2." exact). |
| Incompletes semantics (sum vs maxConnections) | PINNED + TRAP DOCUMENTED | `calculate_incomplete_count` is Java `getIncompleteCount()` = the SUM of per-net `NetIncompletes.count()`; `all_incompletes()`' FIRST tuple slot is `maxConnections` (the endpoint-sum LOWER BOUND) — never the total. The two coincide only while nothing is routed: the probe's incompletes rows witness 0 vs 2 on the completed world (masked-coincidence trap; buglog). `t12_driver_finishes_with_skip_tally` pins the final `incomplete=0` counter row (sum hits 0 while maxConnections stays 2). |
| Task-state + counters events → rows | PINNED | Java's `NamedAlgorithm` listeners and `BoardUpdatedEvent` carry OBJECTS; the port renders one `task_state` row (state/pass/hash) and one `board_updated` row per fire. Sequences pinned per world: STARTED(0)+RUNNING×N+FINISHED (completion, N=2, no pass 3), CANCELLED(4) via the maxPasses gate, CANCELLED(1) mid-pass via maxItems, CANCELLED(18) via stagnation, CANCELLED(0) BEFORE the no-layers error. Counter rows pinned verbatim incl. the pre-pass vs final-fill tally and the pass-2-empty-queue no-fire (2 rows only). |
| Stop face (shared flag + local bit) | PINNED | Java's `StoppableThread.stopRequested` is shared thread state; the port splits a `local` bit (driver-side stops survive flagless) + optional `Arc<AtomicBool>` handed to the engines as their `stoppableThread`. The pass runner's maxItems face raises it (`stop.request()` — M9), the gate re-reads it every loop head. |
| Deterministic budgets (TimeLimit family) | SEAM: units deviation (documented) | Java builds a wall-clock `TimeLimit` per connection route (`100000·2^(pass−1)` ms capped at `Integer.MAX_VALUE`, `AutorouteConnectionRouter:72-74`) and answers `elapsed > limit` in `isStopRequested`. The corpus profile spends the SAME limit VALUE as call ticks at the SAME sites with the SAME strict `>` (`RouteBudget::Deterministic`); the wall face stays available (`RouteBudget::Wall`). Same family: the pass-duration wall clock in the pass-completed row stays real (normalized `<t>` in captures, asserted never in pins), and the 250 ms board-update throttle + progress snapshot are dropped with the GUI event while the interval COUNTER survives (`update_progress`, pinned at 7). Also this family (quality MINOR-4, BANKED): the pass's trace pair is MODELED, not verbatim — Java's `traceEntry` is SILENT (a `perfData` timestamp, `FRLogger.java:159-172`) and `traceExit` emits ONE differently-shaped row ("Method '…' was performed in X.", `:194-224`), while the port emits two entry-formatted rows gated on `is_trace_enabled` (Java gates on the global TRACE level). Log-only, unpinned; the load-bearing face (the wall clock feeding the pass-completed row) is preserved — revisit when T16 consumes the trace channel. |
| Job seam | BANKED (host business) | `job.state == TIMED_OUT → requestStop`, the CPU-suffix arm of the pass-completed row (`job.resourceUsage`), `saveIntermediateStages` snapshot events, the session-summary fields, and `job.logInfo/logDebug`'s "[shortName] " prefix all need the host job object; the port's stop face is the only stop source and all stopped exits read CANCELLED (the TIMED_OUT variant is unreachable without a job). The `!isOptimizerAutorouter` gate on the pass-completed row is constant true for this driver (the optimizer variant of the loop lands with the M-optimizer milestone). |
| `IllegalArgumentException` → `Result` | PINNED | Java's no-active-layer abort throws after emitting CANCELLED(0); the port returns `BatchLoopError::NoActiveSignalLayers` with the message verbatim ("Cannot start autorouter: all layers are disabled.") and the SAME emit order (warn row → event → error). The blanket `catch (Exception)` faces down the stack (pass runner `:330-334`, connection router `:162-165`) do not exist here — a panic propagates loudly. |
| `alreadyRoutedBoardHashes` | BANKED (dead in Java) | The consult block is commented out (`AutorouteBatchLoop:297-306`, "Same-hash stop disabled because ripup budgets and random seeds change per-pass"); only the `clear()` calls survive. The port keeps the doc and drops the field (a live unused field would trip `dead_code`). |
| `hasIgnoredNets` ≡ false | BANKED (constant conjunct) | Java writes the flag only on GUI edit paths — on every parsed board the queue gate's `!hasIgnoredNets()` conjunct is constant true; no port field. |
| removeTails stop option | PINNED (by construction) + flip documented | `runSingleThread:296-302` picks `StopConnectionOption.None` when `removeUnconnectedVias` else `FanoutVia`; the port mirrors the branch verbatim (`pass_runner.rs`). On the T12 worlds the sweep removes only free-floating tails (the anchor-sweep pin), so the OPTION flip is not counter-visible here — the option plumbing is exercised through the shared `remove_tails` free function the driver's recovery face also calls. |
| Empty-scoring NPE parity arms | PINNED (witnessed in the jar) | Java's `getMaximumScore` (`BoardStatistics:655`) unboxes `scoring.unroutedNetPenalty` and `calculateScore` (`:648`) unboxes `scoring.viaCosts` — an EMPTY scoring box (`new RouterSettings()`) NPEs at run time. The port's `expect("Java NPE parity: …")` arms mirror exactly those unboxes; the probe reproduced all three NPE faces live (unroutedNetPenalty, tracePullTightAccuracy via the fanout stage, viaCosts) before backfilling. `legacyScoringOrDefault` documented in `board_statistics.rs`. |
| Settings witness (the hardcoded cost table) | EVIDENCED | The probe's `settings_witness` row validates `new RouterSettings(board)` on the fixture — per-layer trace costs 1.0/2.7 and 1.6/1.0 (aspect-ratio-derived), bend 0.0, via 1, plane via 1, vias allowed, neckdown false, startRipup 1 — exactly the `settings_ir()` table the Rust worlds hardcode. The SCORE penalty scalars stay NULL in the bare-board ctor (Java NPE parity above); Rust worlds score with `RouterSettingsScoring::default()` and via-cost 50 vs the probe's witnessed fallback 1 — score VALUES are not pinned anywhere, only row texts (documented in the probe source). |
| Necked retry + strict-DRC enforcement | strict-DRC PINNED (M12/M13) / necked-retry BODY UNPINNED (gate-closed face only) | `applyStrictDrcAfterRoute`'s gate (`isStrictDrc() || pass >= 3`, enforcement-only vs snapshot-only split) and `enforceStrictDrc`'s id-watermark rip are pinned end-to-end including the snapshot rollback (M12/M13 killed there). The `retryConnectionNecked` (`:168-247`) BODY is UNPINNED — no neck-width world exists (`neck_width_um` is 0.0 in every world, so the retry gate never arms; quality-review Q-G deleting the retry's budget-carry `take_budget()` passed 147/0), and the body includes the deliberate SAME-BUDGET carry Java guarantees by passing the one `TimeLimit` instance to both attempts. DISCRIMINATING WORLD (unbuilt): `neck_width_um > 0` on a world whose connection FAILS full-width then routes necked — the budget-carry face rides that world. The only live face today is the gate-closed default. |
| Plane-world queue/route interplay | PINNED | `t12_plane_world_connected_to_plane_and_queue_contrast`: net 94 as a pour net — a pour-connected pin answers CONNECTED_TO_PLANE with ZERO engine work, and the queue gate skips pour-connected candidates (4 vs 5 items with the flag flipped off; the handled-marked pin never seeds; the CA row "connected: 2/3" verbatim). M15 kills the queue-skip drop. |
| Queue walk (getAutorouteItems) | PINNED (via driver + pass pins) | Descending-seed-id walk, one queue entry per qualifying net, the `handled` single-net marking, the population gate (`connected >= population` skip), the plane skip, and the debug rows ("Queuing item for routing: Pin on net 'NET_98' (connected: 1/2)") — all validated against the jar's queue world AND the probe's native tap rows (order + ids + nets exact: 104,103,102,101 / 98,98,33,33). |
| Failure logbook | PINNED (1 pin) + placement documented | `RoutingFailureLog` lives ON the board (Java: a non-transient Serializable field — snapshots carry it and restores roll it back); keyed by item id with first-net captured at creation; `FAILURE_THRESHOLD` 50 carried (the pass runner reads `failure_count`, gating the per-item failure row at `>= 3` — pinned in `pr_pass_walk_rows_and_tally`). |
| Probe-wiring lesson (log4j) | BUGLOG | FRLogger logs to the NAMED "app.freerouting.Freerouting" logger whose jar config has additivity=false — a root-only tap attachment receives NOTHING; the full wiring (named LoggerConfig + appender, root appender, core logger addAppender) is required, and the jar's console appender pollutes stdout (pipe through `grep '^{"type"'`). Buglog entry logged. |
| Capture-commit note | COMMITTED PER DISPATCH | `logs/M3-T12/captures/batch-driver-probe.jsonl` (840 rows, md5 a9db46eae82119fd57cfd7dbbe494cbc) committed with this task — the dispatch's explicit captures path overrides the `/logs/` ignore (T10b/T10c/T11 precedent). BYTE-STABILITY (spec MINOR-2, recorded not normalized): the capture is 839/840 byte-stable across fresh JDK-25 runs — exactly ONE row family is racy in JAVA itself, the per-attempt `board_updated` row at the starved_local pass-15/16 boundary (variants pass15/q3/f1 ↔ pass16/q3/f1 ↔ duplicated pass15/q0/f4 — a Java-internal snapshot race at the pass boundary; content conserved, total items 4 in every variant). The implementer's double run was byte-identical; the reviewer's fresh runs diverged from the committed file AND each other only in this family. No Rust pin targets the racy family (the per-attempt board_updated reduction is already banked above), and a probe-side normalization cannot be proven to affect only that family — so the committed baseline stays untouched per the "if in doubt, record only" rule. |

## T13 epic-cli route (appended by M3-T13; fix round appended after spec review)

New crate `rust/crates/epic-cli/` (`main.rs` 47: arg dispatch + exit codes;
`route.rs` 1327: the flow + manifest + projection + flow witnesses;
`settings.rs` 2082: the resolver + CLI parse face + validate + pins; deps
dsn/board/drc/router/geometry + serde + serde_json + sha2). Flow: read DSN
bytes (kept for the fixture hash) → `SesBoard` parse → `Board::from_ses_board`
+ `SearchTreeManager` + `normalize_all_traces` (bug-131) → the load-time
violation seed (F3, below) → resolver: `merge` → `validate` →
`apply_board_specific_optimizations` → `BatchSettings::new` + pub-field
overrides → `BatchDriver::run` with a stderr-echoing `CliDriverSink` (last
`RouterCounters` kept) → `BoardStatistics` → projection → SES write →
manifest → exit code.

**Mutant ledger (round 1, M1-M12 all killed; round 2 M13-M19 all killed).**
Round-1 sites: M1 = `merge()` CLI block — a CLI overwrite deleted (DSN/default
value survives), killed by `merge_precedence_cli_over_dsn_over_default`; M2 =
`DsnLayer::from_metadata` — a `*_set` gate inverted (copies the parsed value
though the scope never appeared), killed by
`dsn_layer_reads_only_set_flags_and_drops_trace_costs`; M3 = scoring-version
asymmetry (v2 → V2_LOWER_BOUND on the router box too), killed by
`scoring_version_dual_apply_asymmetry`; M4 = ×100 bounds made inclusive
(0.0/1.0 also ×100), killed by `improvement_threshold_x100_and_oit_deprecation`;
M5 = implicit router-enable dropped, killed by `implicit_router_enable_faces`;
M6 = geometry pass toggle moved AFTER the direction fill (off-by-one
alternation), killed by `geometry_pass_alternation_and_costs_on_four_layer_board`;
M7 = outer-surcharge block (>2 signal layers) deleted, killed by the same pin;
M8/M9/M10/M11/M12 per the rows below. Round-2 sites (fix round): M13 =
`run_route` validate() call deleted, killed by
`negative_max_passes_routes_after_validate_reset` (in-suite end-to-end); M14 =
default scoring arm back to `trim().to_uppercase()`, killed by
`scoring_version_default_arm_is_raw`; M15 = the load-time violation seed block
deleted, killed by `pre_existing_violations_seeded_from_load` on the
violation-bearing craft board (mutant run reported introduced 1 == total 1);
M16 = extension strip skipped (`file_name` passed raw), killed by the smoke's
exact `(session "t9_locator45"` header assert; M17 = non-signal force-disable
arm deleted, killed by `geometry_pass_force_disables_non_signal_layers`
(closes review gap PG1/RM5); M18 = unknown short flag turned into a hard
error, killed by `unknown_short_flag_is_silently_skipped` (closes PG2/RM8);
M19 = validate maxPasses bound `< 0` → `< 1`, killed by
`validate_resets_max_passes_threads_and_accuracy`. The PG3 pin
(`router_introduced_subtracts_pre_existing_seed`) additionally caught a REAL
pre-existing bug in the (3,5) world before it shipped: `i32::saturating_sub`
floors at i32::MIN, not 0 — the port now uses Java's exact `max(0, ...)` on
i64 (buglog 167). Round-3 sites (quality review, all killed): Q1-a = the
SES-write state gate bypassed (`matches!` forced true), killed by
`terminated_run_writes_no_ses_and_reports_output_unwritten`; Q1-b =
`render_manifest` hardcoding `output_written: true`, killed by the same pin;
Q2 = the manifest-write `Err` propagation restored, killed by
`manifest_write_failure_keeps_ses_exit_code`; Q3-a = the pre-existing `-do`
delete disabled, killed by world 1 of `stale_output_deleted_after_input_load`;
Q3-b = the delete moved AHEAD of the input load, killed by the same pin's
world-2 contrast (Java aborts BEFORE the delete on a load failure —
`Freerouting.java:114-118` vs `:122-127` — so the stale file survives there);
Q4 = `remove_unconnected_vias = false`, killed by
`batch_settings_wiring_fanout_to_remove_unconnected_vias` (QM10's survivor);
Q5 = the `dsn.run_optimizer` merge arm deleted, killed by
`merge_copies_every_field_from_both_layers` (QM2's survivor); Q6 = `rfind` →
`find`, killed by `filename_without_extension_cuts_at_last_dot` (QM5's
survivor); Q7 = the `UserFixed`/`SystemFixed` ladder arms swapped, killed by
BOTH `fixed_to_ir_ladder_is_permutation_sensitive` and
`projection_preserves_fixed_state` (QM6's survivor); Q8 = the maxThreads warn
number dropped, killed by `validate_resets_max_passes_threads_and_accuracy`
(QM7's survivor); Q10c = the `--result-json=` form arm disabled, killed by
`result_json_equals_form_parses`. QM1 (the main.rs usage-exit-2 arm) stays
binary-witness-only — the dispatch contract (usage=2 vs failure=1) has no
in-suite pin; banked by review disposition.

| Seam | Status | Notes |
|---|---|---|
| SettingsMerger layer model | PINNED | Priorities Default 0 → DsnFile 20 → Cli 60; `ReflectionUtil.copyFields` (:215-338) is a DEEP copy-if-set merge → every source field is `Option<T>` in the port (`CliLayer`/`DsnLayer`/`MergedSettings`), `merge()` applies in priority order and NEVER nulls (a silent source leaves the earlier value). Precedence world pinned; defaults come only from the `DefaultSettings.getSettings()` seed (`MergedSettings::default`, scalars pinned: maxPasses 0, maxItems i32::MAX, pullTight 500, neckdown true, fanout true, improvement 2.5, via 50/plane 5/ripup 100, routerScoring V2_CONTINUOUS + 0.5/1000/3-weight set). Mutants M1, M2 killed. |
| validate() — the merger's trailing step (`SettingsMerger.java:189` + `RouterSettings.java:938-977`) | PINNED (fix round F1) | `validate(&mut MergedSettings)` runs after `merge`, before the geometry pass, and returns the warn rows: maxPasses `< 0` (or `> 9999` and not the i32::MAX sentinel) → WARN "Invalid maxPasses value: N, using default 0 (no limit)" + reset 0 (= unlimited); maxThreads None → `max(1, cores-1)` (`defaultMaxThreads` :127-129), `< 0` → warn + default, `> cores` → warn + cap (0 survives); tracePullTightAccuracy `< 1` → warn + reset 500. Without it a negative max_passes reached the T12 router-enabled gate and silently routed NOTHING at exit 0 (review-verified); the flow witness routes a `-5` run end-to-end (in-suite + binary: warn verbatim, pass ran, path rows written). Mutant M19 killed. |
| DSN settings layer reads ONLY set flags | PINNED | `DsnFileSettings`' opinions are exactly the `(autoroute_settings ...)` fields the parse actually SET (the `*_set` flags landed in epic-dsn for T13): run_router/run_optimizer ALWAYS set (post-loop), vias/via_costs/plane_via_costs/start_ripup_costs only on scope presence, per-layer active/preferred_direction per scope, bend cost NEVER. The layer-count seed (`:47-52`) sizes the slots before the geometry pass. The parsed per-layer `(preferred_direction_trace_costs ...)` values are parsed then DROPPED here — bug-compat fact below. |
| Trace-cost discard (boardSpecificTraceCostsApplied) | BUG-COMPAT (documented deviation from a naive reading) | The flag is `private transient` and never survives the merger, so the headless flow's `applyBoardSpecificOptimizations` ALWAYS re-derives the per-layer cost tables — DSN trace-cost scopes are dead writes in Java. The port mirrors the OUTCOME: `apply_board_specific_optimizations` is unconditional and no port field ever carries DSN trace costs. (If a future Java fix stops re-deriving, this seam is the one to revisit.) |
| Geometry pass port (`RouterSettings.java:267-360`) | PINNED | hw/vw = bounding-box w/h as f64; adds `0.1*Math.round(10*hw/vw)` / `0.1*Math.round(10*vw/hw)` (round = floor(x+0.5)); slots resize to the board; `currentPrefHoriz` seeds `hw < vw` and each signal layer TOGGLES FIRST then uses; `!signal` forces routable=false (PG1 pin: mixed S/PWR/S board → power slot Some(false), signal slots Some(true), no toggle on power, no surcharge at 2 signals); signal+null → true; bend null → `scoring.defaultBendCost ?: 0.0` written through UNCLAMPED; prefHoriz null → current (DSN-set survives — the pass fills only null slots); undesired = default + (horiz ? hAdd : vAdd); >2 signal layers → outerAdd `0.2*signalCount` on [0] and [last] of BOTH arrays. Pinned on a 4-signal 2:1 board (directions [T,F,T,F], rows (1.8,3.8)/(1.5,1.0)/(1.0,3.0)/(2.3,1.8)) + a DSN-opinion-survival world. Mutants M6, M7, M17 killed. |
| getBendCost clamp asymmetry (`:696-707`) | PINNED | A SET slot value returns RAW (12.0 stays 12.0); only the default fallback clamps into [0.0, 9.9] (MIN/MAX constants; null default → 0.0). Mutant M8 (clamp-everything) killed by the asymmetry pin. |
| getTraceCosts (`:881-894`) | PINNED | horizontal = preferred iff the layer's preferred direction is horizontal, vertical = the other; post-geometry-pass the direction slot is always filled (the `i % 2 == 1` fallback is the un-geometry-passed parse default). |
| CLI parse surface (`CliSettings.java`) | PINNED | `--x=v` requires the `=` (bare `--x` ignored except CLI-owned `--result-json <path>` space form); `-f v` consumes the next non-`-` arg else ""; short map `-mp`→max_passes, `-mt`→max_threads, `-scoring-version` (dual-apply), `-router-scoring-version`, `-optimizer-scoring-version`, `-oit` (warns "The '-oit' command-line flag is deprecated; use '--router.optimizer.improvement_threshold' instead." EVEN when it applies); unknown short flags SILENTLY skipped (`:90-96` + mapFlagToProperty null face — PG2 pin: `-zz 3` parses Ok, no warn, operand consumed, never applied); unknown `router.*` paths WARN-and-continue; deprecated flat keys (enabled/max_passes/algorithm/max_items/save_intermediate_stages/ignore_net_classes) canonicalize with the `LegacyRouterSettingsBridge` warn text. Mutant M18 killed. |
| Scoring-version normalization + THE ASYMMETRY (`:135-156`) | PINNED (default arm corrected, fix round F2) | Alias table {v1|legacy→V1_LEGACY; v2|continuous→router? V2_CONTINUOUS : V2_LOWER_BOUND; lower_bound|lower-bound→V2_LOWER_BOUND} is case-INSENSITIVE; the DEFAULT arm passes the value RAW to valueOf (`default -> value`, CliSettings.java:142) — mixed-case (`V2_Continuous`) and non-alias junk (`v2_continuous`) warn with the field unset; an exactly-typed constant applies; V1_LEGACY valid on both boxes; V2_CONTINUOUS ONLY on the router box; V2_LOWER_BOUND ONLY on the optimizer box (OptimizerScoringVersion has NO V2_CONTINUOUS — that absence is what makes the v2 alias asymmetric); an invalid constant for the addressed box warns and leaves the field unset. (Round-1 implementation uppercased the default arm — over-normalized; corrected + pinned.) Mutants M3, M14 killed. |
| improvement_threshold ×100 quirk (`:145-156`) | PINNED | f32 parse; value in the EXCLUSIVE (0,1) → ×100 (0.25→25.0); endpoints 0.0/1.0 raw; >1 raw; parse failure → warn + unset. Mutant M4 (inclusive bounds) killed. Parsed but UNUSED in T13 (no optimizer stage) — banked for the optimizer milestone. |
| Implicit router enable (`:100-107`) | PINNED | `-de`+`-do` with no explicit enable path → autorouter_enabled=Some(true); explicit `--router.enabled=` / `--router.autorouter.enabled=` wins (has_explicit tracks both forms); one file short → no trigger. Mutant M5 killed. |
| Narrowed -de/-do | DEVIATION (deliberate, documented) | `GlobalSettings.java:608-665` consumes ALL non-flag args after -de (multi-file, `+` concatenation, type-by-extension) and additional -do outputs; the `route` subcommand takes exactly ONE DSN and ONE SES — a second file, a `+` concatenation, an empty value, or a flag-as-value is a hard usage error (exit 2). The SES design name is the job name WITHOUT extension (next row). |
| SES job name = filename WITHOUT extension (`RoutingJob.java:519` → `SesWriter.java:52-88`) | PINNED (fix round F4) | The port mirrors `BoardFileDetails.getFilenameWithoutExtension` (:205-210: cut at the LAST dot, else unchanged) and hands the stripped name to `write_session` — so the writer's `.replace(".dsn", ".ses")` is a no-op and the header reads `(session "t9_locator45"` / `(base_design "t9_locator45")`, no `.dsn` anywhere. Round-1 passed the raw basename, so every CLI SES header carried `.dsn` (invisible to the ses-compare gates, which feed the writer the correct name — caught by review). The manifest's `fixture.filename` keeps the RAW input name (it identifies the input file). Mutant M16 killed (smoke asserts the exact stripped header). |
| Final-state mapping | DEVIATION vs naive driver reading (Java-scheduler-faithful) | `Ok(true)` → COMPLETED; `Ok(false)` → CANCELLED only for the EXTERNAL `StopReason::UserStop` — every INTERNAL stop (MaxPasses/MaxItems/stagnation/restore exhaustion) is COMPLETED, because the Java job scheduler treats the batch loop's own stop as normal end-of-work (T12's stop face notes all stopped exits read CANCELLED at the DRIVER level; the CLI re-maps at the scheduler level). `Err(BatchLoopError)` → TERMINATED. TIMED_OUT unreachable (no wall-clock job behind `--deterministic-budgets=on`). Pinned as a table; mutant M12 killed. |
| Exit code (`MainResult` face) | PINNED | 0 iff COMPLETED (or the unreachable TIMED_OUT) AND output_written (file exists, size > 0); else 1; usage/argument errors exit 2 before any routing. The output side is the REAL write outcome (quality Q1): the SES write is state-gated like Java's `writeCliOutputIfAvailable` (`Freerouting.java:265-267` — only COMPLETED/TIMED_OUT produce a session; a TERMINATED run leaves NO file) and the manifest carries the measured flag, not a constant — the pre-fix binary witness leaked a 4290-byte unrouted SES plus `output_written: true` on the no-signal-layer board, exactly where Java leaves no output and emits `false` (`RoutingResultManifest.java:191`); the write-failure `len() > 0` face is near-unreachable and unpinned (QM3 disposition). The gate's CANCELLED EXCLUSION is live-correct but unguarded and unreachable today — `StopFace::default()` carries no external stop, so no CLI run ends CANCELLED, and the renderer-level pins build telemetry directly — the reviewer's mutant adding CANCELLED to the write-allowed set survives 37/37 (re-review RQ1, banked): re-examine before M4 wires a stop source into `StopFace`. Mutants M10, Q1-a, Q1-b killed. The main.rs usage-exit-2 arm remains binary-witness-only (QM1, banked). |
| Load-time violation seed (`HeadlessBoardManager.java:788-793`) | PINNED (fix round F3) | After board build + tree insert + normalize, BEFORE the driver, the flow runs `epic_drc::clearance::all_clearance_violation_depths` (the `getAllClearanceViolations` face — the depth walk, quality Q10b: the same count without materializing `ViolationRow`s) and stores the count in `board.pre_existing_clearance_violations_count`. Java defers this to a background post-load thread (its BoardStatistics treats 0 as "not yet measured" — a Java-side race); the CLI walks synchronously, which is strictly MORE correct and matches the intent. The manifest face (`BoardStatistics.java:390-393` `max(0, total − preExisting)`) is the pure `clearance_face` (i64 math — the i32-saturating_sub floor bug the PG3 pin caught is fixed). Wiring witness: the violation-bearing craft board `harness/corpus/craft/drc-main.dsn` (golden drc-0015 = 3 load-time rows) routes with `router_introduced < total`; binary witness concurs (total 1, introduced 0). Mutant M15 killed. |
| Manifest (RoutingResultManifest v1) | PINNED + deliberate omission | Field names mirror `harness/src/manifest.rs` (snake_case, no deny_unknown_fields). HONEST DIVERGENCE (round-1 rationale was wrong — review D1): the emitted bytes OMIT generated_at, all duration_seconds, cpu/memory rows, resource_usage, cpu_score, and also settings_snapshot/bounds — but Java EMITS those keys with real payloads (settingsSnapshot is ctor-initialized `RoutingJob.java:120` + post-merge-reassigned `Freerouting.java:152`, never null; bounds non-null whenever the board exists; Gson null-elision is irrelevant). T13 chooses not to emit them: the brief's mandated emit list excludes them and the mirror reader ignores them. `optimizer_score`'s absence likewise reflects the missing T13 optimizer stage (brief-permitted, Java runs it by default) — NOT null-elision. Iff-gates pinned: normalized_score present iff a scoring face exists (always in T13), optimizer_score ALWAYS absent, board_statistics present iff stats walked, passes_completed backfilled from the LAST counters' pass_count when > 0. The backfill source is deliberately PHASE-BLIND today (last counters regardless of `counters.phase`) because T13 emits only AUTOROUTE counters — M4 must filter by phase when the fanout stage starts emitting counters, or the backfill mis-attributes (review QM8 note, banked). Mutant M9 killed. |
| fixture.sha256 | PINNED | SHA-256 of the INPUT DSN bytes (not the output session); `git_sha` from `FREEROUTING_GIT_SHA` env else "unknown"; app_version = the crate version. Mutant M11 killed. |
| Board → SES projection | BY DESIGN (not the parse face) | Routed items project through `SesBoard::push_routed_item` (LIVE id preserved, no parse drop guards — those belong to `insert_trace`), walking `iter_ascending()` filtered `on_the_board`; Trace → TraceIr{layer, half_width, corners (rational corners rounded per-axis java_round), polyline clone, nets, clearance class, fixed-state 1:1}; Via likewise. Pins/keepouts/outlines are already in the parse `ses.items`. The fixed-state ladder AND the id/state round trip (ses → Board → `project_routed_items` → ses) are pinned (`fixed_to_ir_ladder_is_permutation_sensitive` + `projection_preserves_fixed_state`; review QM6's arm swap died on both — quality Q7); the emitted-BYTES face (a real fixed input through the binary) stays banked for T15's byte compare. |
| remove_unconnected_vias derivation | PINNED (T12 semantics, T13 wiring + pin) | The job ctor's `removeUnconnectedVias = !isFanoutEnabled()` derivation lives in `build_batch_settings` (the flow's step 4, separated so the wiring is directly pinnable) and `batch_settings_wiring_fanout_to_remove_unconnected_vias` asserts the REAL batch face both ways (fanout on → tail sweep off; fanout off → tail sweep on) — the review's QM10 mutant (`= false`) had passed the whole suite before this pin existed; the remaining BatchSettings overrides come straight off `ResolvedRouteSettings` (max_passes Some(0) ≡ unlimited via the driver gate, via/plane costs off the merged scoring box). Mutant Q4 killed. |
| max_threads | BANKED (parsed, validated, unused by the engine) | `-mt`/`router.max_threads` parses (i32, bad value warns), merges, and now passes through `validate()`'s normalization (None → default, <0 → warn+default, >cores → warn+cap) so the merged value matches Java's face; the T13 engine stays single-threaded by construction. |
| SesBoard metadata layer_count | NOTE | The layer-count seed reads `ses.metadata.layer_count` — the PARSE layer count, matching Java's `DsnFileSettings` ctor argument; the geometry pass then resizes to the BOARD layer count (which can only match or the DSN is malformed). |
| Smoke | EVIDENCED | `smoke_route_locator_fixture` (t9_locator45.dsn, `--router.autorouter.max_passes=1`): exit 0, COMPLETED, manifest asserted (fixture sha = input bytes, passes ≥ 1, score face present), SES written non-empty with routed `(path ...)` rows, header asserted extension-less (F4). No SES reader exists in epic-dsn (the DSN-only M1b reader) — the round-trip assert is on the emitted text; an SES reader, when it lands, must be pointed at the CLI output. Binary witnesses (fix round): `-5` max_passes run → WARN verbatim on stderr + pass ran + path rows; drc-main.dsn run → introduced 0 < total 1. |

## T14 router-only oracle baselines (appended by M3-T14)

Harness-side task (no epic-router engine code): re-capture the Tier A oracle
baselines through the JAVA jar with fanout + optimizer disabled, committed
under `rust/harness/baselines/router-only/A/` (11 fixtures, tiers.yaml
authoritative). These are the ground truth T15's directional compare gates
(`incomplete ≤ Java`, `violations ≤ Java`, `score ≥ Java − ε`) gate the Rust
router against.

| Seam | Status | Notes |
|---|---|---|
| Oracle profile model (`oracle::OracleProfile`) | PINNED | `FullFlow` (default; extra args `[]`, baselines dir `java/`, record marker `None`) vs `RouterOnly` (extra args EXACTLY `["--optimizer.enabled=false", "--router.fanout.enabled=false"]`, dir `router-only/`, marker `Some("router-only")`, double-run capture discipline ON). `run_oracle` takes the PROFILE, not raw flags — one argv suffix site (`oracle_argv(…, extra_args)`) shared by capture AND verify, so the trap-3 unthreading has no second call site to mutate (M6 banked by construction). Flags go to the JAVA jar argv ONLY — never epic-cli (`--optimizer.enabled=false` warns "unsupported path in the route subset" and is dropped there; the Rust engine has no optimizer stage; T15 expresses fanout-off on the Rust side with `--router.fanout.enabled=false`). Pin: `router_only_profile_contract_flags_dirs_marker_and_names` (both faces + unknown-name bail). Mutants M1 (suffix dropped → 2 pins die), M3 (compare gate removed → pin dies) killed. |
| Flag parse faces (Java, re-verified in-tree) | EVIDENCED | Both defaults ON (`DefaultSettings.java:165` fanout.enabled, `:178` optimizer.enabled) so BOTH disables are passed explicitly. `--optimizer.enabled=false` / `--router.fanout.enabled=false` reach `RouterSettings` via the CliSettings `router.`/`optimizer.` prefix gates (`CliSettings.java:59+`); `LegacyRouterSettingsBridge.canonicalCliPath` keeps `fanout.enabled` verbatim; `ReflectionUtil` parses the Boolean. COSMETIC JAVA QUIRK (costs an investigation, documented): stdout carries `WARN Unknown settings property: optimizer.enabled` — that is the GlobalSettings LAYER (`GlobalSettings.applyCommandLineArguments` → `setValue` `:596` → NoSuchFieldException warn `:523` — GlobalSettings has `routerSettings`, no `optimizer` field), NOT a dropped flag; the merger's CliSettings layer applies it, and the manifest phases prove the effect. A warning-driven "flag didn't take" reading would be wrong — read the manifest. |
| PROVE-stages-off gate (`OracleProfile::assert_stages_off`, wrapping `assert_router_only_manifest`) | PINNED + jar-witnessed | SCOPED BY PROFILE (the T14 quality-review MAJOR-1 fix): only `RouterOnly` gates — a full-flow manifest legitimately carries the stage faces, so the gate no-ops for `FullFlow` at EVERY call site (capture run 1 + double-run 2, and the Verify arm since the MINOR-3 fix); folding the profile decision into the gate API makes the ungated-call-site class (which broke every default `capture` before the fix) unrepresentable. Under RouterOnly: bail on ANY populated face of `phases.optimizer.duration_seconds` / `passes_completed`, top-level `optimizer_score`, `phases.fanout.duration_seconds` / `passes_completed`. Faces read off the Java, not guessed: optimizer off ⇒ `RoutingPipeline.java:36` never constructs the stage ⇒ its PhaseDetail is untouched (`{}`) AND `RoutingResultManifest.java:200-203` emits optimizer_score iff the optimizer phase carried before/after ⇒ absent. Fanout off ⇒ `AutorouteBatchLoop.java:96-102` leaves `fanoutBeforeStats` null ⇒ the phase block at `:232-251` never runs ⇒ `{}`. Faces are BEHAVIORAL, not structural, and adjudicated sound: the optimizer face is structural (the stage is never constructed), while the fanout face is ENABLED-driven, not activity-driven — `fanoutBeforeStats` is set whenever `isFanoutEnabled()` (`AutorouteBatchLoop.java:94-98`), BEFORE the SMD-empty skip at `:101-104`, so it populates even on SMD-less boards; if the Java face shapes ever drift, gating the effective settings from the manifest's `settings_snapshot` (`RoutingResultManifest.java:44-45`, deliberately ignored by the mirror) is the structural hardening path. Probe run (bm08, both flags, JDK 25) witnessed exactly these faces before capture; the record's `optimizer_score = null`, `optimizer_seconds = null` on all 11 fixtures are the committed witnesses. Pins: `prove_off_gate_rejects_each_disabled_stage_face_independently` (5 worlds + all-clear), full-flow-shaped-manifest world, and `stages_off_gate_is_profile_scoped` (both scoping directions — FullFlow-gated-too and RouterOnly-no-op'd mutants die there); mutant M5 (gate weakened to optimizer_score only) dies on the duration/passes worlds. |
| Record field `profile: Option<String>` | PINNED | The profile marker lives ON the record (not by path alone): `None` = full-flow (and every pre-T14 record — `Option` keeps them parseable), `Some("router-only")` = router-only. `compare()` gates it (`GateFailure::Profile`) so an unthreaded verify (full-flow re-run vs router-only records) fails loudly — the e2e pin's contrast face distills the SAME run without the marker and asserts exactly that single failure. `ses_sha256` was already on the record (M0); T14 adds nothing else. Mutant M2 (marker dropped) + M4 (ses hash reading the wrong bytes) die on `distill_stamps_and_round_trips_the_profile_marker` / `distill_hashes_the_ses_bytes_present_at_ses_path`. |
| removeUnconnectedVias coupling (the T15 note) | WITNESSED in the captures | With fanout disabled, the Java job derives `removeUnconnectedVias = true` (`BatchAutorouter.java:111-117`), which SKIPS the final tail sweep (`AutorouteBatchLoop.java:594-600`, the `wasRouterRun && !(removeUnconnectedVias || continueAutorouting || stopRequested)` gate). Consequence visible in the numbers: router-only ≠ full-flow and the two are NOT comparable (no fanout pre-pass, no optimizer, no final tail sweep). E.g. bm01 router-only incomplete=2/score=986.32 vs full-flow 0/1000.0; bm11 14/883.33 vs 3/975.0; bm02 1/960.78 vs 0/1000.0; bm07, bm08, bm09, ecc83-pp, ecc83-pp_v2 identical or near-identical (boards where the changed faces don't bind). T15's Rust compare must run the same setting (`--router.fanout.enabled=false` ⇒ remove_unconnected_vias=true ⇒ tail sweep off — the T12/T13 wiring pin `batch_settings_wiring_fanout_to_remove_unconnected_vias` already pins the derivation). |
| Double-run + diff capture discipline | EVIDENCED (zero races) | Router-only capture runs EVERY fixture twice (trap 4 / T12 MINOR-2 lesson), diffs the parity fields EXACTLY (final_state, exit_code, incomplete/maximum, violations total+introduced, normalized_score, passes_completed) and bails on any drift; a ses_sha256 divergence would be stamped into the record's notes (recorded, never normalized) and flip the fixture into capture's requires-acknowledgement set. RESULT: all 11 fixtures diffed EXACT including ses_sha256 — no racy field surfaced, no notes; capture exited 0 clean. Tier timeouts kept (trap 5). |
| Verify-path test | EVIDENCED | `router_only_bm08_baseline_verifies_green_and_profile_gate_bites` (`#[ignore]`, jar+JDK 25; run in-suite manually this task): committed router-only bm08 record verifies green against a live re-run with the same flags through the same distill+compare path as the `Verify` subcommand, and the contrast face proves the trap-3 gate bites (marker-less distill of the SAME run ⇒ exactly one `GateFailure::Profile`); the prove-off-side face (MINOR-3 fix) shows a stages-on manifest failing the router-only verify gate while passing FullFlow. The `Verify` arm itself now runs the profile-scoped gate before distill (defense symmetry with capture). Companion e2e `full_flow_capture_tier_writes_a_correct_record_and_stays_green` pins the MAJOR-1 regression (default full-flow `capture_tier` exits 0 and writes a correct marker-less record with populated optimizer faces) — the first pin reaching `capture_tier`. |

## T15 router compare gates (appended by M3-T15)

Harness-side task (no epic-router engine code): `rust/harness/src/router_compare.rs`
adds `epic-harness router compare` / `router determinism` (wired in
`harness/src/main.rs`). Java-free at run time — the Java side is the COMMITTED
T14 records (`baselines/router-only/A/`); no oracle is ever spawned here.

| Seam | Status | Notes |
|---|---|---|
| Directional gates (`compare_directional`) | PINNED (15 pins + 3 `#[ignore]` e2e smokes; M1–M9 + reviewer OW-M2/OW-M3 + re-review NEW-B/NEW-C + quality Q3/Q5 + quality re-review R1/R2 all killed) | Per Tier A fixture (tiers.yaml walk, completeness canary-pinned): RUN INTEGRITY is the precondition (timeout / no manifest / non-COMPLETED / exit≠0 / COMPLETED-but-no-SES ⇒ sole verdict, numerics NOT judged — "the comparison does not count"), then `incomplete ≤ Java` HARD, `clearance_violations_total ≤ Java` HARD (both Option-absence = loud failure, no fake 0.0), `score ≥ Java − ε` (equal boundary passes). Report-only fields (router_introduced, passes_completed, trace/via/bend counts) are STRUCTURALLY outside `GateFailure` — they cannot gate, only render (pinned both ways: mutating them keeps the verdict `[PASS]` while the line changes; the status-prefix mutant dies). Pins: `directional_gates_both_directions_and_equal_boundary` (both directions + `== Java`), `score_gate_epsilon_is_relative_not_absolute` (500-scale world so a same-numbered absolute-ε mutant diverges at delta 1.5), `integrity_failure_short_circuits_even_with_better_numbers` (TERMINATED with 0/0/1000 still red), `report_only_fields_never_gate_but_do_render`, `record_pre_flight_rejects_missing_and_mismatched_records` (missing/wrong-engine/wrong-profile/full-flow), `tier_a_fixture_enum_is_complete_against_tiers_and_records` (11 fixtures, tiers order, space-bearing path verbatim, no extra records — the trap-2 dropped-fixture canary), `localizer_prints_per_net_pairs_and_decomposition`, `determinism_check_catches_a_planted_divergence`, `score_decomposition_terms_sum_to_the_engine_score` (decomposition cross-checked against `BoardStatistics::get_legacy_normalized_score` on the same stats), plus the spec-review hardening pins `run_cli_argv_carries_the_comparability_flag` (OW-M2), `determinism_digests_read_distinct_run_paths` (OW-M3), `pair_from_runs_derives_run2_from_run2_dir_only` (re-review NEW-B/MINOR-A), and `detail_pass_restores_the_previous_panic_hook_on_every_path` (re-review NEW-C/MINOR-B); the quality-review round adds `battery_exit_pins_the_gate_report_ladder` (the extracted pure exit ladder — MINOR-3/Q3), `localizer_truncates_violation_pairs_at_exactly_first_n` (a 12-pair world rendering exactly 10 rows — NOTE-A/Q6), and `e2e_router_determinism_subcommand_is_the_determinism_gate` (`#[ignore]` e2e smoke on the BUILT binary — MINOR-4/Q5), plus the ε `.abs()` assert line inside the existing ε pin (MINOR-5/Q2) and the empty-battery bail (MINOR-2, live-witnessed both modes); the quality RE-review round adds two more `#[ignore]` e2e smokes over the same built binary — `e2e_router_compare_gate_mode_exits_nonzero_on_a_red_battery` (bm01 gate mode: exit NONZERO + the `[RED]` verdict line — holds the `battery_exit` CALL-SITE route, R1) and `e2e_router_compare_empty_filter_bails_instantly` (`--fixture zzz`: exit NONZERO + the exact bail message on stderr, ~0s — holds the bail's tally condition, R2) — and extends the truncation pin with the walk-order face (first row = pair 0, last = pair 9, R3). Mutants M1/M2 (gates strict-`<`), M3 (ε absolute), M4 (status keys on a report field), M5 (determinism always-OK), M6 (fixture dropped from the walk), M7 (missing record → fake zeroed record), M8 (localizer gutted to aggregates), M9 (score comparison inverted) ALL KILLED; the reviewer's surviving mutants OW-M2 (comparability flag dropped from the run argv), OW-M3 (run1's files hashed for both determinism digests), NEW-B (determinism pair built with run2 from run1's dir), and NEW-C (panic-hook restore deleted) were each RE-APPLIED post-hardening and now die on the four hardening pins respectively (see the Comparability flag, `router determinism`, and localizer rows). |
| ε policy (`SCORE_RELATIVE_EPSILON = 0.02`, `epsilon_for`) | PINNED | ONE relative policy, stated once in the module docs: `ε = 0.02 × max(\|java_score\|, 1.0)` (the 1.0 floor keeps JSON-round-trip scale on tiny scores). Rationale from the observed deltas: bm08 (the only completing fixture) reproduces the Java score EXACTLY, so the noise floor is ≤1e-6 relative and 2% is four orders of magnitude above it while staying SMALLER than one unrouted connection on bm08 (1000/25 = 40 pts = 4%) — ε can never mask a completion regression because the count gates gate those exactly; ε only arbitrates score drift with equal-or-better counts (trace/via/bend aesthetics). One policy, no per-fixture tuning; T17 may tighten with real delta data. Quality-review MINOR-5 (Q2 class closed): the `.abs()` face is pinned by one line inside the same ε pin (`assert_eq!(epsilon_for(-500.0), 10.0)`) — dropping `.abs()` makes ε negative on a negative Java score (silently STRICTER, fails safe); the domain is unreachable through real records (the engine clamps normalized_score ≥ 0 on both sides) but the invariant is now mutation-verified rather than latent. |
| Comparability flag | PINNED (T13/T15 wiring; OW-M2 survivor hardened) | Every Rust run is `epic-cli route … --router.fanout.enabled=false`: T14 records were captured fanout+optimizer-off, the Rust engine has no optimizer, and fanout-off flips `remove_unconnected_vias` (the batch tail sweep) — the coupling is single-sourced in `epic_cli::route::build_batch_settings` (T13) and the T15 detail pass calls the SAME function, so the two run faces cannot drift. Spec-review OW-M2 hardening: the flag lives as the `COMPARABILITY_FLAG` const; `route_argv` is THE one argv builder (`run_cli` spawns from it, the detail pass reuses the const) and the banner's fanout-off claim is DERIVED from the built argv (`comparability_banner_phrase`) — drop the flag and the banner prints "COMPARABILITY FLAG MISSING" instead of lying. Pin `run_cli_argv_carries_the_comparability_flag`: the flag rides EXACTLY once as the argv suffix + the derived-banner both faces. Mutation witness: re-applying the reviewer's OW-M2 (flag dropped from the builder) failed the pin ("the comparability flag must ride EXACTLY once, as the argv suffix"); reverted by edit-back. |
| First-divergence localizer (`localization_lines`) | PINNED | On a red gate: the FIRST failing gate is named, then the run-integrity evidence (exit code + stderr tail — today's panic class), aggregate rust-vs-java numbers, per-net incomplete rows, first-10 walk-ordered violation pairs (with expected/actual clearances), the legacy 0-1000 decomposition (unrouted/violation/bend/via/trace-length, each normalized raw/maximum×1000, `normalized = max(0, 1000 − Σ)` with the maximum≤0 floor mirroring the engine's divide-by-zero face), and the Rust geometry counts (labeled java-side-absent — the committed records carry none). Sourced from an IN-PROCESS detail pass replicating the subprocess flow (same argv face → same settings chain → `BatchDriver`), panic-caught (`catch_unwind`): a panicking fixture degrades to subprocess evidence, never silence. Spec-review NOTE hardening: the caught panic no longer leaks through the DEFAULT panic hook first — `run_detail_pass` silences the hook (`take_hook`/`set_hook(no-op)`) and restores the previous hook on EVERY path out; the process-global hook swap is safe there because the harness is single-threaded on this path (nothing else can panic concurrently). Witnessed live on bm01 gate mode: zero raw `thread 'main'` hook lines in the output; the panic reason survives only inside the harness's own formatted NOTE. Re-review MINOR-B hardening (NEW-C survivor): the restore property is PINNED by the sentinel pin `detail_pass_restores_the_previous_panic_hook_on_every_path` — a recording hook installed as "previous", then a deliberate panic after BOTH exit paths (the early-error world: an unreadable DSN; the caught-panic world: bm01's in-process engine panic today, degrading to the success path once T17 fixes it) must reach the SENTINEL, not a lingering no-op; the pin is self-cleaning (it reinstalls the hook it found). Mutation witness: NEW-C re-applied (restore statement deleted) failed the pin at the early-error world; reverted by edit-back. A present-but-unparseable manifest is surfaced as its own NOTE, not "never produced". Quality-review NOTE-A hardening (Q6 class closed): the first-10 truncation is now pinned at a >FIRST_N world — `localizer_truncates_violation_pairs_at_exactly_first_n` builds 12 synthetic violation pairs and asserts EXACTLY 10 rows render plus the `violation pairs (12 total, first 10 in walk order)` header; the `FIRST_N` 10→9 mutant (quality Q6) renders 9 and dies. Re-review R3: the pin ALSO holds the WALK ORDER — the first rendered row must be pair 0 and the last pair 9 — closing the `.take` → `.rev().take` mutant (renders the wrong 10 rows under the same count+header; re-applied, the walk-order assert kills it). |
| `router determinism` | GREEN (live; OW-M3 survivor hardened) | One DSN (default bm08 — smallest Tier A board AND the only completing one), TWO epic-cli runs to SEPARATE output paths (trap 5), sha256-compared SES + manifest. Live run: `determinism: OK (ses 96ba73…, manifest 16714d…) byte-identical across two runs`, exit 0, 0.8s total. Pinned for all four faces (OK / SES diverge / manifest diverge / missing artifacts). Spec-review OW-M3 hardening: the digest sources are a typed `DeterminismPair {run1, run2}` (each `DeterminismArtifacts {ses, manifest}`) feeding `determinism_digests` — "which run's files feed which digest" is structural, not loose cmd-level locals, so a run1/run2 path slip cannot silently hash one run twice (this glue backs CI's only HARD gate, so it is PINNED, not banked). Pin `determinism_digests_read_distinct_run_paths`: planted-different run1/run2 bytes MUST yield different digests + a failing SES-naming verdict, planted-equal bytes the OK face, and the missing-run2-SES face flows through the pair. Mutation witness: re-applying the reviewer's OW-M3 (run1's SES+manifest hashed for both sides) failed the pin ("run1/run2 SES bytes differ, so their digests must differ"); reverted by edit-back. Re-review MINOR-A hardening (NEW-B survivor): the pair is now built by the pinned `pair_from_runs(run1_dir, run2_dir)` helper (not cmd-level literal joins), with the artifact names single-sourced as `RUN_SES_FILE`/`RUN_MANIFEST_FILE` shared by `run_cli` (writer) and the helper (reader). Pin `pair_from_runs_derives_run2_from_run2_dir_only`: the exact path faces AND planted-different-bytes digests. Mutation witness: NEW-B re-applied INSIDE the helper (run2's joins from run1_dir) failed BOTH faces ("run2's SES must derive from run2_dir"); reverted by edit-back. Re-review-2 MINOR-C hardening (NEW-D survivor): the CALL SITE's argument choice is one level above any reachable pin, so `determinism_cmd` carries an inline consistency assert tying the built pair to the runner's truth (both `ses_path`s + both dirs' manifests) — NEW-D (`pair_from_runs(&run1_dir, &run1_dir)`) previously survived every pin AND printed a live false OK; now the command exits 101 at the named NEW-D guard instead. |
| CI (`rust-check.yml`) | LANDED GREEN, report mode | Steps: `cargo build -q -p epic-cli` → `router determinism` (HARD gate) → `router compare --report-only` (exit 0, every fixture's verdict in the CI log), all `EPIC_SKIP_GRADLE=1`. THE T17 EXIT-CONDITION FLIP is documented in the workflow comment: drop `--report-only` once every Tier A fixture passes all three gates. Measured dry-run (debug build, local, java-free): the whole chain exits 0 in 217s — the compare step is ~216s because the 10 red fixtures each auto-run the panic-caught in-process detail pass for the localizer (roughly double the subprocess time; the wall drops once T17 fixes the panics). Quality-review MINOR-1: all three steps now carry `timeout-minutes:` — build 5 (cold `cargo build -p epic-cli` with headroom; warm-cache seconds), determinism 30 (= 7.5× the worst case, two runs × the 120s tier timeout — a hang trips the step instead of the 360-min GitHub job default), compare 30 (≈ 8× the measured ~220s wall; completing fixtures route FASTER than their tier timeouts, so the bound stays valid post-T17). T17 watchdog note (workflow comment + here): `run_detail_pass`/`route_detail_inner` has NO deadline of its own — it is fast today only because 10 fixtures panic out of it; once T17's fixes convert those panics into real in-process routes, a hang would trip the 30-min step bound, and T17 must add a real watchdog (wall-clock deadline in the driver loop or a thread + channel timeout). |
| HONEST INVENTORY (the reason compare lands in report mode) | OPEN — T17 | 10 of 11 Tier A fixtures are RED on RUN INTEGRITY today: two engine panic classes (8× `Line only implemented for IntPoints till now` — rational endpoint in the routing path, buglog 169; 2× `a via carries its drill info` — shove_probe, buglog 170). Only bm08 completes and it passes ALL gates exactly: incomplete 0≤0, violations 0≤0, score 1000.00≥1000.0 (delta 0.00; Java passes 2 vs Rust 1 — report-only). Gate mode bites: red fixture in gate mode exits 1 (witnessed on bm01); report mode exits 0. |
| Banks | DOCUMENTED | (a) cmd-LEVEL wiring mutants in BOTH cmds (e.g. swapping the work-dir join, breaking the summary tally, the determinism run spawn/timeout/verdict-print wiring) are out of pin reach — the pins cover the `compare_directional`/`verdict_line`/`localization_lines`/`determinism_verdict`/`pair_from_runs`/`determinism_digests`/`tier_a_fixtures`/`CompareProfile::load_record` API surface (renamed at T11 quality: the two loader wrappers became the enum method), which is where the gate semantics live; the cmd-level wiring is exercised live by the CI step + this task's runs. Quality-review round narrowed it further (MINOR-2/3/4): the compare EXIT LADDER is OUT of this bank — extracted pure as `battery_exit(report_only, red)` and pinned both directions by `battery_exit_pins_the_gate_report_ladder` (report mode always Ok; gate mode Ok only at red==0, else a bail naming the count — the T17 flip face; witness: the Q3 gutting re-applied failed the pin at "a red gate must exit nonzero", reverted by edit-back); the EMPTY-BATTERY face is bail-guarded in BOTH modes (`error: --fixture {filter:?} matched no Tier A fixture`; witnessed live: `--fixture "zzz"` exits 1 in gate AND report mode); and the `run()` clap DISPATCHER is OUT — e2e-smoked by `e2e_router_determinism_subcommand_is_the_determinism_gate` (`#[ignore]`, T14 convention): it locates the BUILT epic-harness binary (next to the test exe, else `rust/target/debug/`), runs `router determinism --fixture <bm08>` under `EPIC_SKIP_GRADLE=1`, asserts exit 0 + stdout contains `determinism: OK` — ~0.8s java-free, needs `cargo build -p epic-cli` first (doc comment says so); witness: the Q5 dispatch swap re-applied failed the pin (payload showed the compare banner where the HARD gate belongs), reverted by edit-back. Quality RE-review narrowed the RESIDUAL (R1/R2): the extracted ladder's CALL-SITE ROUTE (bypassing the `battery_exit(report_only, red)` tail keeps the fn pinned while gate mode exits 0 on a red battery — R1) and the bail's TALLY CONDITION (`== 0` → `== 2` silently restores the round-1 hazard — R2) are OUT of the banked class too, both held by gate-mode e2e smokes over the BUILT binary (`e2e_router_compare_gate_mode_exits_nonzero_on_a_red_battery`: bm01 — a panic-class fixture, subprocess dies in ~1s — asserts exit NONZERO + the `[RED] DAC2020_boards/DAC2020_bm01.dsn` verdict line; `e2e_router_compare_empty_filter_bails_instantly`: `--fixture zzz` asserts exit NONZERO + the exact bail message on stderr, ~0s); witnesses: R1 (call bypassed, fn left correct) and R2 (tally weakened) were each re-applied with the bin REBUILT and each failed its e2e — and the witness itself surfaced a trap now documented on the shared `e2e_built_harness_bin` helper: `cargo test --bin` does NOT rebuild the spawned bin, so a STALE binary false-PASSES an e2e (`cargo build -p epic-harness` first; CI's step order already does this). Also banked verbatim now (NOTE-F face): the four inline call-site asserts in `determinism_cmd` are wholesale-DELETABLE by a refactor — no #[test] reaches them; only the NEW-D guard witness proves they bind. Spec-review MINOR-2 + re-review MINOR-A narrowed this bank to its honest residual: BOTH `determinism_cmd` faces that carry gate semantics are now PINNED — the digest computation (`determinism_digests_read_distinct_run_paths` on the typed `DeterminismPair`) AND the pair construction (`pair_from_runs_derives_run2_from_run2_dir_only`, with the artifact names single-sourced between the runner and the builder via `RUN_SES_FILE`/`RUN_MANIFEST_FILE`) — so the round-1 wording "moved OUT of the banked class" is now TRUE face-by-face, and the wiring residual itself shrank again at re-review 2 (MINOR-C / NEW-D): the helper's CALL-SITE ARGUMENTS are held by an inline consistency assert in `determinism_cmd` (not a #[test] — pins see the helper's output, never the call site) tying the pair to the RUNNER's truth (`pair.run1/2.ses` vs the `CliRun`s' `ses_path`, manifests vs each dir's `RUN_MANIFEST_FILE`); witness: the NEW-D mutant (`pair_from_runs(&run1_dir, &run1_dir)`) previously survived 13/13 pins AND printed a live FALSE OK (exit 0, run2 never read) — re-applied post-assert the command dies RED (exit 101, "determinism call-site slip: run2's SES digest source must be run2's own output file (NEW-D guard)"); reverted by edit-back. What remains banked for `determinism_cmd` is therefore ONLY its spawn/timeout/print wiring, the same residual class as `compare_cmd`'s banked glue. (Round-1 drift note: that wording originally overclaimed — the pair-construction literal was in neither named bucket and the NEW-B mutant survived it; the re-review caught it and this pin closes it.) (b) The detail-vs-manifest consistency NOTE (detail pass must reproduce the subprocess aggregates) is diagnostic, not a gate — the determinism gate already pins subprocess byte-identity, and 10/11 fixtures cannot complete a detail pass today. (c) Per-net/violation localizer faces are synthetic-world pinned (no completing failing fixture exists yet to witness them live — T17's fixing work will exercise them on real data). |

## T16 route event-stream corpus (appended by M3-T16)

Maze-level differential parity corpus: the Java router's own trace stream (the
`RAW_SECTION assign/skip` rows of `MazeSearchEngine.java:798-821/:907-933` plus
the `AutoroutePassRunner` `compare_trace_ripped_item`/`compare_trace_route_item`
rows) captured through the real `runBatchLoop()` by
`rust/harness/oracle/RouteEventProbe.java` (javac-compiled against the oracle
jar, ONE JVM per capture, `events golden` runs it TWICE and bails unless the
captures are byte-identical — anchors §4), mirrored by the Rust engine riding
the `DriverSink`, and compared row-for-row by the java-free
`epic-harness events compare` (`rust/harness/src/route_events.rs`). Manifest:
3 fixtures the Rust engine completes today (`e1_ripup` — the forced-ripup world
at `start_ripup_costs=40000` (2 nets), `t7_ripup` (3 nets), `t9_locator45`),
committed as
`harness/corpus/events-manifest.jsonl` + `events-golden.jsonl` (3529 rows,
double-run byte-identical). Drift policy: SHRINK the fixture, never filter the
stream (a diverging board was replaced, the stream never trimmed).
NET-SHAPE NOTE (spec-review minor 3): t9_locator45 is a pre-existing M2
fixture and is a 97-net DSN (96 single-pin stub nets `D0xx-PD` with zero
connections + `NET_98` carrying the real pins) — it exceeds the 2-5-net
model-fixture shape and never had that shape; it is in the set for its
WORKLOAD (a 2-connection world, deterministic on both engines, completing
far under budget), which is what the anchors require.

| Seam | Status | Notes |
|---|---|---|
| Row normalization contract | PINNED | A 5-arg granular row `"[%s] [%s] %s: %s"` is stored as `operation message` (the `[method]` wrapper and the `: <impacted items>` tail stripped, the OPERATION kept) so Java's stored text is byte-equal to the Rust mirror's row text; one-arg rows (`RAW_SECTION`) pass through untouched. No wall-clock value appears in any tapped row (the pass-duration row is a different kind and is filtered out by the tap's four-kind whitelist). |
| Gating semantics (Java-verified, NOT assumed) | PINNED | `emit_raw_row` (search_engine.rs) is UNCONDITIONAL — Java builds the RAW_SECTION row strings AT THE CALL SITE (`MazeSearchEngine.java:798-821/:907-933`) and the log4j backend filters; `logRippedItems` is called UNGATED (`AutoroutePassRunner.java:250`), only `logTraceRouteComparison` sits behind `isTraceEnabled()` (`:251-252`). The Rust mirror matches exactly. This cost a pin fix: the first draft of the gating pin asserted BOTH compare kinds gated off and FAILED because `compare_trace_ripped_item` legitimately flows — the Java source won over the pin author's assumption (grep of AutoroutePassRunner.java :250-252 is the witness). Pin: `trace_disabled_sink_gates_compare_rows_but_raw_rows_still_flow` (TraceDisabledSink over e1: zero route_item rows, 395 assigns, 1 ripped, run still completes — gating is log-only). |
| The engine mirror (`emit_raw_row` + `describe_expandable`, search_engine.rs:217-300) | PINNED + engine-attached | Java's exact field order + rendering ported; `engine.rs` attaches the one-arg trace backend (`maze.trace_sink(&mut *sink)` — Java `FRLogger.trace(String)` is a separate backend face from the granular 5-arg trace) and the maze-result raw row now rides `emit_raw_row` for identical backend/position semantics. Mutant M8 (emit_raw_row → no-op) drops the e1 assigns 395→0 and dies on the gating pin + compare; M9 (removing the `is_trace_enabled` gate on the route comparison) leaks a route row into the disabled sink and dies on the same pin. |
| `events compare` alignment core (`diff_kind_streams` + `first_differing_field`) | PINNED (9 mutants killed: M1–M9) | Strict golden load (`deny_unknown_fields` on all four row shapes, untagged `GoldenRow` enum), per-fixture (kind, ordinal) alignment — length drift caught at the first missing/extra ordinal (a zip alone is truncation-blind; M7 proven) — and field-level diff over the `", "` grammar (`first_differing_field` names the FIRST differing segment with kind-prefix stripping; segment-count drift is a divergence). 13 pins total (spec-review round). Core: `kind_of…` (M1), `rows_by_kind…` (strict golden face vs filter rust face), `first_differing_field…` (M2), `field_split_survives_the_describe_grammar` (real-row 13-field shape, no false split), `double_capture_gate…` (zip + length), `golden_rows_parse_strictly…` (Gson HTML-escape decode + field-order round-trip + strictness on ALL FOUR variant faces — M3 exposed a real pin gap (Run face only), the strengthened faces re-killed it; spec-review R4 then caught the MISSING WITNESS face (`deny_unknown_fields` off `SettingsWitness` survived 12/12) and the added witness extra-field reject kills the R4 re-apply), `sanity_check_rejects_interleaved_fixtures` (M6), `diff_kind_streams_catches_drop_extra_and_field_drift` (M4/M5/M7). Corpus-backed (three): `golden_corpus_literal_rows_and_census` (3529 rows + census + real captured texts: e1 assign#58, e1's only RIPPED row — the forced-ripup harvest face, added in the fix round (self-check mutant: the literal's ripupCost drifts one count → pin dies), t7 skip#1, t9 route#1), `rust_stream_aligns_through_triaged_divergences` (the triage record, below), and `route_rows_share_the_field_skeleton_and_diverge_only_in_id_values` (the Class-C grammar face, below). Quality round (+4, 17 total): `check_manifest_entry_rejects_name_drift_and_missing_witness` (the extracted manifest↔golden glue core: name / witness-opening / tuned-scalar faces) + `compare_glue_tripwires_fire_before_the_divergence_face` — the corpus-backed CALL-SITE binding (quality MIN-1: the reviewer's Q1 cross-check-gutted and Q6 sanity-call-removed mutants survived the 13-pin battery because every pin tested only the helpers; both re-applied and die at the glue pin — Q1 at the recapture assert, the drifted manifest otherwise sailing into the ordinary Class-B divergence; Q6 at the contiguity (re-opened golden) and block-shape (closers stripped) faces instead of the count check / silent outcome-skip; quality re-review RR-A: a 4th DISTINCT well-formed block (e1's lines, fixture renamed) trips the block-count face — RR-1 (count check gutted) re-applied died there, the zip otherwise silently ignoring the extra block into the Class-B divergence), `sanity_check_enforces_the_probe_world_shape` (MIN-2: each block opens with the witness and closes with exactly one run + one incompletes, trace rows before the closers — compare()'s outcome faces are `if let Some`, so a missing closer would silently lose the outcome witness), and `golden_trace_rows_carry_the_per_kind_segment_census` (O-2: the `", "` grammar is segment-count invariant per kind across the WHOLE committed corpus — assign 13 ×3498, skip 11 ×6, ripped 7 ×1, route 7 ×15, zero exceptions over all 3520 trace rows). |
| `events golden` capture discipline | EVIDENCED | One probe JVM per invocation (javac into a pid-keyed temp classes dir, cleanup on every bail), deterministic `-Duser.language=en -Duser.country=US`, binary-safe `read_until` of `{"fixture"`-led lines (the drc corpus tr-TR/leak lessons), double-run byte-diff bail, canonical serde re-serialization (Gson HTML-escapes `=`/`->`; the golden bytes are ALWAYS the serde canonical form, never raw probe bytes). Quality round (MIN-3): `IDENTITY_ORDINALS.clear()` at the top of each `runDriverWorld` — the identity-hash map is PER-WORLD state and, uncleared, would stamp a future hash-carrying row's ordinal by first appearance ACROSS fixtures (silently re-stamped by a manifest reorder); dead-in-practice today (zero `@` in the golden), proven byte-neutral by a /tmp re-capture identical to the committed golden (sha `cc50216c…`) — untestable until a hash row exists, deliberately UNPINNED. O-1: `isTappedRow` taps by SUBSTRING (`contains`) where the Rust classifier is `starts_with` — documented on the method; any over-capture is loud at compare (the strict golden face, Q4-witnessed). The committed golden's census: e1 assign/skip/ripped/route 331/1/1/4, t7 496/4/0/7, t9 2671/1/0/4; all three worlds `returned=true`, incompletes (0, 2), witness costs 40000/1/1 — pinned literally by `golden_corpus_literal_rows_and_census` (with the e1 assign#58, t7 skip#1, t9 route#1 real texts). |
| DIVERGENCE TRIAGE (the compare is honestly RED; every divergence classified, none tuned away) | ALL CLASSES CLOSED (A: M4-T1; B: M4-T4→T5→T6; C: M4-T4→T6) — the compare exits 0 (M4-T6) | Class A — parse-time item-id assignment drift (buglog 172) — CLOSED by M4-T1 with the root cause CORRECTED: parse id ASSIGNMENT order was already byte-identical (both sides burn ids 1..20 in file order, GEN_MAX 20); the real mechanism was Java's in-read `board.normalizeAllTraces()` (`Wiring.java:347`) merging t7's collinear wire pair (id 10 deleted, id 13 extended to (600000,350000)(558000,350000)) — Java's post-parse board carries 19 items, no id 10, while the events world's D11-deferred parse kept 20. Fix: the events world's shared prelude `parse_world_board` (harness/src/route_events.rs) now runs `normalize_all_traces` after reinsert — the consumer pattern every other read path already had; `from_ses_board` itself needed NO change. Witnesses: `parse_world_board_matches_the_jar_post_parse_id_sequence` pins the literal IdOrderProbe jar captures (t7 19 rows incl. merged id-13 corners; e1 10 rows — the normalize-inert contrast arm); mutants T1-M1 (normalize dropped → pin dies 20v19) and T1-M2 (`combine_at_start` direction arm flipped → pin dies 18v19) both killed; former ordinal-3 row byte-identical (executable witness golden[2]==rust[2]). Class B — expansion-room door-slicing divergence (buglog 173): e1 diverges at assign 58 (Java slices the keepout wall 216250..983750 into 7 sections, Rust 595340..983750 into 4); t9 diverges at 1350 (same ExpansionDrill origin, different wall door); t7's post-M4-T1 divergence at assign 23 is the same family (a target-door CHOICE: Java expands door item=7 at 47000.0, Rust door item=6). Class C — insert-path id churn (buglog 174): route_item rows' `netItems`/`maxItemId` faces (t9 maxItemId 130 vs 122) PLUS a triaged route-slot COUNT drift on t7 (golden 7 vs Rust 8 — one extra Rust attempt, whose tail rows also shift the running `incompletes=`/`netIncomplete=` snapshots); the live VALUES stay unpinned (they move with the fix) — the executable face is the route-row GRAMMAR pin `route_rows_share_the_field_skeleton_and_diverge_only_in_id_values`: identical field skeletons on every aligned row (via the real `field_name_of_segment` the localizer uses), rows byte-equal outside the id-family segments through a per-fixture strict prefix (e1/t9 all rows; t7 rows 0-4), count face (e1 4==4, t9 4==4, t7 7v8), and at least one live id divergence per fixture. Mutants C1 (`netItems=` renamed in the pass_runner rendering → skeleton face dies) and C2 (a non-id `netIncomplete` value shift → stripped-equality face dies on e1's byte-identical rows 0-1) both killed. The strict prefix is itself EXECUTABLE (spec re-review R-A — reviewer mutant S1, prefix 5→4, survived an unpinned literal): the pin derives the FIRST aligned row whose non-id faces drift and asserts the prefix equals it — a too-long prefix already self-detects in the `.take()` loop, the derivation kills the shortening direction (S1 re-applied, died at the boundary assert). The triage pin runs the REAL compare path per fixture and pins the exact (slot, ordinal, prefix, field, segment+geometry witnesses) for each class — any engine change that moves these ordinals must update the report triage, not silently shift the pin. Aligned prefixes before first divergence: e1 57, t7 22, t9 1349 byte-identical rows. M4-T6 CLOSE (2026-09-23): the compare is GREEN — exit 0, 3520 golden trace rows aligned (e1 337 = 331 assign + 1 skip + 1 ripped + 4 routed; t7 507 = 496+4+0+7; t9 2676 = 2671+1+0+4), all worlds returned with incompletes (0, 2) — evidence `logs/M4-T6/evidence/cmp_events_t6.log`. Root cause of the remaining t7 0-based-73 and t9 route-row-0 residuals (bug-187; closes buglog 173/174's last faces): the engine's `DrillEngine::shove_trace_check` was a T10-era 0.0 stub — every maze shove short-circuited to "impossible"/ripup-only, churning the rip/reinsert op sequence and everything downstream (tree op order → id churn → the netItems/maxItemId faces); it now delegates to the production `epic_board::trace_shover::check_max_length`. The pins re-triaged GOLDEN-ANCHORED: the triage tuples are GONE (all three streams byte-equal end-to-end), the former divergence ordinals remain as equality witnesses (t7 assign#73 carries the golden `TargetItemExpansionDoor/item=6` at `62570.28048522717`; t9 route#0 carries the golden `netItems=2->9`/`maxItemId=130`; t7 route#3 the golden `2->12`/`107`), and the Rust-measured residual literals (821846.30, 2->7, 126, 93) are purged. `rust_stream_aligns_through_triaged_divergences` + `route_rows_share_the_field_skeleton_and_diverge_only_in_id_values` held the former reds BY DESIGN; both now hold the closed state (18/18 route_events pins green, `logs/M4-T6/evidence/t6tailpins_35green.log` for the tightener bank). |
| CI | RECOMMENDED AT THE M4 CI-FLIP MOMENT — nothing flipped in T6 (deliberate) | T6 UPDATE (2026-09-23): Classes A/B/C are ALL CLOSED — `events compare` now exits 0 (3520 golden trace rows aligned, byte-identical streams; see the DIVERGENCE TRIAGE row above). The command is java-free and CI-able today; adding it to rust-check.yml should ride the M4 CI-flip commit (the M3 adjudication holds the tier flip for M4's one-commit moment — the events step, ~0.4s over 3 in-process worlds, belongs in that same workflow edit rather than shipping alone). Gate evidence meanwhile: `logs/M4-T6/evidence/cmp_events_t6.log`. |
| Banks | DOCUMENTED | (a) The three triage classes are FINDINGS, not fixes — fixing them in T16 would have meant touching id assignment (M1b-owned) or expansion-room door slicing (engine work out of T16's corpus scope); the contract says a real divergence is banked, classified, and pinned, then fixed upstream with the pin as the regression witness. (b) cmd-level `events` spawn/print wiring (javac argv, temp-dir names) is out of pin reach, same residual class as the other corpora; the capture discipline itself is exercised live by the golden regeneration run recorded in the report. |

## T17a Tier A integrity close (appended by M3-T17a)

Two engine panic classes closed at the DELEGATION faces (buglog 169/170) —
8+2 Tier A fixtures stopped exiting 101. Neither fix touches epic-geometry's
cast faces: Java's own geometry THROWS ClassCastException on the same inputs,
so the port's panics are Java-intent; what was unfaithful was WHO fed the
rational points into them. Java wins every conflict below.

| Seam | Status | Notes |
|---|---|---|
| `DrillEngine::check_trace_segment` (engine.rs) — segment delegation | FIXED + PINNED (P1 red→green, P1b literals) | Root cause 169 (refines the T15 guess "rational line inputs"): Java `MazeTraceShover.java:178` calls `board.checkTraceSegment(shoveLineSegment, …)` — the LineSegment OVERLOAD (`RoutingBoard.java:223-233`) delegates the segment straight through, and the facade materializes the polyline from the segment's OWN int lines (`lineSegment.toPolyline()`) — the corner points are NEVER computed. The old port DECOMPOSED the segment to `start_point()`/`end_point()` (= `middle ∩ end`, an exact RationalPoint whenever the closing lines are not 45°-perpendicular through int corners) and re-ran the POINTS overload → `Polyline::from_two_corners` with a rational endpoint → panic at `int_pair` (line.rs:71) via `intersection_approx`. The fix delegates to the segment face (`routing_board_search::check_trace_segment`) exactly as Java does; the decomposition is gone. Pin `t17_segment_face_survives_a_rational_corner` (engine.rs): a corridor segment whose end corner is Rat(9_799_970_000, 6_000_000_000, 20_000) = (489998.5, 300000) answers 2147483647.0 through the engine face — RED before the fix (panicked at line.rs:71 via the old decomposition), GREEN after. Pin `t17_segment_face_answers_the_jar_literals` keeps the t11 wall-105/106 world answering 2483.0/7483.0 through the SAME engine face (no int-world drift). Mutation witness M1 (decomposition restored): P1 dies with the line.rs:71 panic while P1b stays green (the int-corner world never discriminates — the documented equivalence: the two faces agree exactly when closing lines are perpendicular through int corners, which the shove construction usually but NOT always produces). |
| `Line::intersection_approx` int-cast face (epic-geometry) | UNCHANGED + PINNED (P3) | Java `Line.intersectionApprox` `(IntPoint)`-casts and ClassCastException's on rational input (Line.java:311-336) — the panic at line.rs:71 IS the faithful face and bug 169 was never "make Line accept rational". Pin `t17_intersection_approx_still_rejects_rational_lines` (`#[should_panic]`, line.rs tests) freezes the rejection: mutation witness M4 (a float fallback accepting rational lines) dies on it. No silent int-truncation, no blanket float conversion — anywhere. |
| `LineSegment::end_point` exact rational corner (epic-geometry) | PINNED (P2) | The construction face P1's segment rides on: `end_point()` is `middle.intersection_point(end)` — the EXACT BigInteger triple, never rounded or truncated. Pin `t17_end_point_is_the_exact_rational_corner` (line_segment.rs tests) asserts the full Rat(9_799_970_000, 6_000_000_000, 20_000) triple AND the float face (489998.5, 300000.0). Mutation witness M3 (int-truncated end point) dies on the exact triple. Cerebrum 11/12 discipline: the pin sits at the CONSTRUCTION face, not at a recursion-healed route outcome, and is mutation-verified. |
| `DrillEngine::via_center` + `end_points_matching` via arm | FIXED + PINNED (P4 red→green) | Root cause 170: Java `endPointsMatching` (MazeTraceShover.java:320-342) reads `item.getCenter()` for the `instanceof DrillItem` arm — the STORED center (total, always present for a live via) — and NEVER touches `getAutorouteDrillInfo` (Via.java:203-216), the transient the maze EXPANSION computes lazily. The old port `expect`ed the drill info here (shove_probe.rs:552 "a via carries its drill info") — an expect on data Java never reads at this site; bm06/bm07 died. New trait face `via_center` (drill/mod.rs) = `DrillItem.getCenter()` (`Board::drill_center`, epic-board), implemented by the production engine, the pins Harness and the scripted SynthWorld; `end_points_matching`'s via arm compares the stored center to the trace's first/last corner (`points_equal`). Pin `t17_end_points_matching_computes_the_via_center_not_drill_info` (maze/pins.rs, a real DSN craft: via PAD_C600 at (40000,40000) sharing a corner with trace W1): asserts the premise (production `via_drill_info` is None — the lazy face never runs here), the center match TRUE for W1, FALSE for a corner-distant trace, FALSE for a foreign net. RED before the fix (panicked at :552), GREEN after; mutation witness M2 (the old expect restored) dies. The T7 maze-drill-expansion face that DOES compute drill info (search_engine.rs skip-on-None) is untouched — different Java site. |
| Battery wall / CI bound | FINDING — deferred scope | Post-fix the Tier A battery runs REAL routes; the CI `router compare --report-only` step (timeout-minutes 30) does NOT survive: bm01's subprocess alone burns its full 1800s tier timeout (TERMINATED), its pass #1 alone measured 1074s (score 466.67, 78/195 unrouted) vs Java's 106s TOTAL 20-pass run, and every red fixture additionally burns an UNBOUNDED in-process detail pass (the T15 SEAM already named this: the watchdog is the fix, not the step bound). The deterministic-tick budgets (T12 row) map Java's wall-clock `100000·2^(pass−1)` limit VALUES to ticks — on driver worlds that is fast, on bm01-class boards a budget-exhausting connection takes pass-count-scaled wall in debug. The `detail_pass_restores_the_previous_panic_hook_on_every_path` sentinel (router_compare.rs) degrades as designed ("if T17 ever fixes it, this world degrades to the success path") in CORRECTNESS but not in TIME — its world-2 bm01 route is now an unbounded full route inside `cargo test`. Both the watchdog and the CI flip are explicitly deferred T17 scope; measured here, not fixed here. |
| Tier A inventory (post-fix) | MEASURED | Per-fixture verdicts with walls in the T17a report (logs/M3-T17), agreeing with buglog 175/177 and the commit message — 5 GREEN: bm02/bm07/bm08/bm09/ecc83-pp complete and pass all three directional gates (several at score 1000 ≥ Java); ecc83-pp_v2 completes but is QUALITY-red (incomplete 1 > Java 0, score 927.83 < 991.32 − ε — buglog 176, T17b scope); 5 INTEGRITY-red: bm01/bm06/bm11/pic_programmer/sonde are TERMINATED at their tier timeouts (1800/600/600/300/300s) with no manifest, so the directional gates were never judged for them — bm06's DETAIL pass completed the full board in-process and bm11/pic_programmer's detail passes ran healthy full passes past the old panic sites (route-health witnesses, NOT gate passes), and sonde's detail pass hit the new buglog-177 panic class. (Quality-review T17a-2 Q-3: this row previously claimed the four terminated fixtures "pass all three directional gates" — corrected; the adjacent Battery-wall row above always carried the truth.) The determinism digests (96ba7300…/16714d5e…) and the events-compare triage faces are UNCHANGED (int-world parity intact). |
| Spec-review fix round (T17a-1, SPEC_COMPLIANT_WITH_MINORS) | CLOSED — pins only, no engine changes | The reviewer's surviving mutants marked two OBSERVABILITY gaps, both closed by pins: SR-3 (the delegation's flag passthrough unobservable — P1b's arms were unfixed×false and fixed×true, never crossing flag × shovability) dies at the new S3 arm of P1b: the S1 span with `onlyNotShovableObstacles=true` → 2147483647.0, verified against Java BOTH at source (RoutingBoard.java:196; facade `:63` okLength = Integer.MAX_VALUE init; `:67-71` skip gate `isRoutable() && !isShoveFixed()`, Item.java:881 + Trace.java:206) and at runtime (the oracle row `S1_unfixed_true` = 2.147483647E9/max_value=true, already captured in T11 at logs/M3-T11/captures/autoroute_engine_rows_run1.jsonl — also proving no OTHER fixed obstacle hides in the span; Java agreed with the doc, so no deviation to mirror). SR-4 (the PRODUCTION `AutorouteEngine::via_center` unit-unobserved — P4 drives the pins-Harness impl) dies at the new pin `t17_production_engine_via_center_answers_the_stored_center` (engine.rs): the P4 craft replayed through parse + build_engine + the production DrillEngine impl — premise (drill info absent), exact stored center (40000,40000), `end_points_matching` TRUE/FALSE/foreign-net. Own mutants (delegation flag INVERSION; production center x+1 value shift) also die at the same pins. Pin census 5→6 (S3 is an arm inside P1b, the production pin is the new #[test]); suite 1292/0/15 under the skip protocol; digests + events face exact-unchanged. |
| ViaOptimizer forward note (quality-review T17a-2 §6c) | BANKED | Java `ViaOptimizer` has four `checkTraceSegment` call sites (`ViaOptimizer.java:246/:317/:402/:416`) and ALL use the POINTS overload (`Point, Point`), not the segment overload — when the optimizer is ported they must route through `epic_board::routing_board_search::check_trace_segment_points` (the existing points-face call at `path/inserter.rs:831`, Java `FoundConnectionInserter.java:566`, is the model), NEVER through the segment face that bug 169's fix delegates to. |

## T17b Battery truth (appended by M3-T17b)

Bug 177's panic class closed at the forced-insertion face + the watchdog
family + the true Tier A inventory. The tick ladder stays the
deterministic currency (controller constraint); only wall bounding is
non-deterministic. Java wins every conflict below.

| Seam | Status | Notes |
|---|---|---|
| Picked-trace tombstone at the forced-insertion combine sites (bug 177) | FIXED (contract-shape) | Java `insertForcedTracePolyline` picks a same-net trace OBJECT at `:489-517` and reads `pickedTrace.polyline()` — the object's CURRENT `lines` field — at both combine sites (`:536-542`, retry `:682`). The reference is memory-alive across the whole shove dance: after the dance REMOVES the picked trace, the read answers REMOVAL-TIME geometry (removals unregister; they never rewrite fields). The port's bare id re-read panicked (`trace_polyline(picked).expect` — buglog 177's sonde class). Fix (`picked_trace_lines`, routing_board_insert.rs): hold `(ItemId, Polyline)` — the id plus the PICK-TIME snapshot; re-read the id while it resolves (in-place geometry writes keep the id, so Java's current-field read is observable), answer the snapshot once the id is dead (equal to Java's removal-time geometry in every death the engine produces — all removal paths unregister WITHOUT changing geometry first; the dodge-pad replacement that killed the picked trace in the pin's world inserts NEW substitute items and never rewrites the dying trace). Residual corner (documented, unhandled): an in-place geometry write FOLLOWED by removal between pick and re-read would make Java answer the changed geometry where the port answers pick-time — unreachable today (in-place writes are combine receivers, which stay live). VACUITY DISCLOSURE (mutation analysis): a skip-when-dead shape is behaviorally indistinguishable in the current engine — the site-2 recombined value propagates only through `shape_index = lines-3` (anchored to the route's LAST segment regardless of the picked prepend), the entry-side corner VALUE at that index (identical), and the `< 3` gate (dead: the bare shortened polyline always has ≥ 3 lines) — the port still computes Java's value (contract shape) rather than skipping, so a future observable consumer inherits the faithful answer. Pin: `t17b_picked_trace_tombstone_recombines_after_midloop_death` (epic-board) — the world kills the picked trace mid-loop via the dodge-pad replacement; RED witness = the pre-fix panic at the site-2 expect; mutant 1 (expect reverted) DIES at the pin (panic replay), mutant 2 (skip-when-dead) SURVIVES per the disclosed inertness. Digests/compares exact-unchanged (the fix only converts a panic path into Java's value). |
| `Item.isTraceObstacle(int)` VIRTUAL dispatch — maze faces + insert gate (bug 176) | FIXED + PINNED (2 epic-board pins + the harness fingerprint pin; 3 mutants) | Java's face is a VIRTUAL dispatch with exactly three overrides the port had collapsed to net-membership: base `!containsNet(net)` (`Item.java:170-172`), `ConductionArea` = `isObstacle && !containsNet` (`:398-400`), `ComponentObstacleArea` (`:71-73`) / `ViaObstacleArea` (`:100-102`) = unconditionally `false` (place/via keepouts never block traces). Every parse-time DSN plane/pour is inserted `isObstacle=false` (`Structure.java:1113`, `Wiring.java:485`), so a FOREIGN net routes THROUGH a plane — the Rust maze faces blocked the bottom corridor on ecc83-pp_v2 (net 6 stuck at incomplete 1, score 927.83 vs Java 991.32, T17a buglog 176). Fix: ONE shared face `Board::item_is_trace_obstacle` (epic-board) honoring the full dispatch, consumed by all three production trait bodies (`EngineShapeView::is_trace_obstacle` engine.rs; the Harness `NeighbourEngine` impls in expansion/pins.rs + drill/pins.rs — the `RoomKind::Obstacle` arm models Java querying the room's CONTAINED item, `SortedRoomNeighbours.java:217-223`) and the insert gate `check_trace_shape` (forced_pad_router.rs, Java `BasicBoard.checkTraceShape:1017-1022` gates each entry on the virtual face). `checkTraceSegment` is UNTOUCHED — Java's segment face has no flag gate (`RoutingBoardSearchFacade.java:46-109`), the Rust port was already Java-equal. `overlapping_objects_ignore_nets` (engine.rs) keeps the BASE `isObstacle(int)` face for item keys (`ShapeSearchTree.java:412-413` overrides nothing; pre-fix the two faces coincided — documented at the site); rooms keep the trait face (`CompleteFreeSpaceExpansionRoom.isObstacle` unconditionally true, `:77-79`). Pins: `item_is_trace_obstacle_pins_the_java_virtual_dispatch` (board.rs — the flag×net×kind matrix incl. the net-0 and unknown-id verdicts), `t17b_check_trace_shape_routes_through_a_non_obstacle_plane` (forced_pad_router.rs — a parsed `(plane GND …)` world asserting all FOUR flag×net cells at the insert gate; NOTE the internal-unit probe: `(resolution um 10)` scales DSN coordinates ×10, a DSN-unit probe lands in empty space and the world goes vacuous), and the end-to-end fingerprint pin in the harness (`detail_pass_deadline_silent_preserves_the_deterministic_outcome`: incomplete 0 / 16 violations / 15-0-66 geometry, the subprocess row equal to the Java record 991.32 exactly). Mutants: M1 CA-flag dropped → all three pins die (the fingerprint pin at `left: 1 right: 0`, net 6 unroutes again); M2 keepout arm → base face → the Board pin's keepout arms die; M3 the insert gate reverted to the base face → the insert-gate pin dies at the crossing cell. Battery: ecc83-pp_v2 GREEN at Java's exact score; sonde xilinx GREEN BETTER (1000.00 vs Java 980.1); determinism digests unchanged (bm08 has no planes). |
| Detail-pass WALL watchdog + bare-suite protocol (bug 175) | LANDED + PINNED (sentinel rewrite + 2 new pins; suite bare in CI) | `run_detail_pass(dsn, deadline)` (harness router_compare.rs): the body relocates to a worker thread; the caller waits `mpsc::recv_timeout`. THREE distinguishable verdict classes — the body's own `Ok`/panic `Err` (the watchdog is silent; the engine has no thread-local state, the worker is a plain relocation, the deterministic outcome byte-for-byte unchanged), the deadline `Err("detail pass deadline exceeded after …s …")` (a WALL bound, distinct from a panic and from a quality face), and infra `Err`s (worker cannot start / exits without reporting). The panic hook is restored on EVERY path out — the deadline path included; `WATCHDOG_HOOK_LOCK` serializes the hook-observing tests (the hook swap is PROCESS-GLOBAL: concurrent `run_detail_pass` calls raced take/silence/restore and one test restored another's silence hook, observed live as the sentinel failing with its message swallowed). On a deadline the worker LEAKS by design (no safe thread kill in Rust): it owns its board, writes only its process-unique scratch ses, cannot reach the receiver again, dies at process exit. TIER-TIMEOUT POLICY (bug 175): the tiers.yaml timeouts stay EXACTLY as committed and the deterministic tick ladder stays the currency — only the WALL bound is non-deterministic, and it never feeds a verdict the tick face owns. The compare call site reuses `entry.timeout_seconds` as the detail wall (one bound for both faces of a fixture's run). CI: the test step runs the suite BARE (`cargo test --workspace`) — the `--skip detail_pass_restores` protocol AND its workflow line are RETIRED (the sentinel's world-2 route now fires a 10s deadline instead of routing bm01 unbounded, >50 min observed); the compare step keeps `--report-only` (the exit-0 posture flip is T17c, NOT this task). Debug→release stability: all three pins green in BOTH profiles (bm01's detail route exceeds the 10s/2s walls by orders of magnitude in either build — debug pass 1 measured 1074s, buglog 175; the generous 600s wall sits far above ecc83-pp_v2's ~0.2s release route), and the deadline-SILENT fingerprint (incomplete 0 / 16 violations / 15-0-66) is identical across profiles — the watchdog does not perturb the deterministic outcome. That pin doubles as the buglog-176 end-to-end witness: it asserts COUNTS only — the localizer's `score_decomposition` reads `resolved.scoring.scoring`, a DIFFERENT settings slice than the engine's internal scoring face (it printed 847.61 where the manifest says 991.32), so no pin may assert the decomposition score. |
| Killed-run detail-skip — the one-fixture-two-walls gap (bug 175 ext, T17b controller round) | FIXED + PINNED (`detail_pass_policy_skips_killed_runs_and_keeps_the_localizer`; 3 mutants) | Forensics of the truncated 3400s battery: the routing kill DID fire (bm01's captured stdout flushed at exactly T0+1800s — `run_cli` kills the subprocess at the tier value; the harness T0 reconciles with the observed process elapsed and the file mtime to the second), but the red-gate branch then ran `run_detail_pass` with the SAME 1800s deadline and the verdict row prints only AFTER it — bm01's footprint was therefore 2× tier (3600s), and the miscalibrated EXTERNAL bound (sized against Σ routing walls only, forgetting the red-fixture detail wall the B commit itself introduced) died inside the post-kill detail pass with ZERO verdict rows and ZERO run manifests flushed: total silence, indistinguishable from a hang. Fix: `detail_pass_should_run(force, failures, timed_out) = force || (!failures.is_empty() && !timed_out)` — a harness-KILLED run skips the detail pass (the in-process re-run of a wall-truncated board can only re-burn the same wall up to its own deadline; it localizes nothing for a slowness timeout), a COMPLETED red KEEPS the localizer (the panic-class reproducer — that is its job), and `--detail` force-overrides even the skip (an explicit opt-in accepts the wall). Killed ⇒ integrity failure is STRUCTURAL (`compare_directional` pushes `RunIntegrity` unconditionally on `timed_out`), so the unforced-killed cell is exactly the skip arm; the loop prints an explicit `detail pass skipped` NOTE so the row's missing detail fields never read as an accident. Battery arithmetic: worst-case per-fixture footprint drops 2× tier → 1× tier (killed fixtures); size the external bound above Σ tier + Σ detail-of-completed-reds (the re-run used 10800s ≥ the 10320s hard worst case). WALL/STABILITY NOTE (controller-requested measurement): release bm01 pass #1 = 103.18s vs debug ≈1074s with IDENTICAL deterministic outcome (score 466.67 / 78 unrouted / 0 violations) — a ~10.4× wall factor with the byte-stable result untouched by profile; now recorded in the rust-check.yml wall warning (whose stale "until the watchdog lands" clause was refreshed the same commit — the CI compare step STILL trips its 30-min bound in the debug profile, by design, until T17c). Call-site wiring is one atom (`detail_pass_should_run(force_detail, &failures, run.timed_out)`): the pinned face is the pure decision (full flag×killed matrix, every row and column), the loop atom is inspection-verified. CENSUS (spec review T17b-1 M-1): the bare suite at the landing commit measured **1300 passed / 0 failed / 15 ignored** (~42s wall ≪ the 30-min step bound) — the 15 `#[ignore]` tags are PRE-EXISTING and untouched (9 epic-router capture replays in `path/pins.rs` + 6 harness slow/e2e gates); the earlier "1300/0/0" report was a misread of the ignored count, there is no 15→0 delta (the report half corrected the same round). |
| Drill-harness `overlapping_objects_ignore_nets` ITEM face (spec review T17b-1 M-2) | FIXED + PINNED (`t17b_ignore_nets_walk_filters_items_on_the_base_obstacle_face`; 1 mutant) | The T17b-D virtual-dispatch fix silently flipped the drill HARNESS's `overlapping_objects_ignore_nets` (drill/pins.rs, inside `impl DrillEngine for Harness`) to filter ALL keys through `NeighbourEngine::is_trace_obstacle` — diverging from Java (`ShapeSearchTree.overlappingObjects` `:412-413` filters on the BASE `Item.isObstacle(int)`, `Item.java:162-164`, overridden by NO item subclass) and from the production engine.rs face exactly in the plane/keepout cell. Test-harness-only (the production ripup walk routes through engine.rs:2122-2131, which was already correct), and no capture exercised the divergence (all gates green) — but a live harness/Java inconsistency. Fix: the same `is_room_key` room/item branch as the production engine — rooms keep the trait face (Java tree rooms are `CompleteFreeSpaceExpansionRoom`, whose `isObstacle(int)` is unconditionally true, `:77-79`), ITEM keys use the BASE `Board::item_is_obstacle`. Pin (drill/pins.rs, `PLANE_WALK_DSN`): a parse-time `(plane GND …)` (a NON-obstacle ConductionArea, `Structure.java:1113`) plus a netless keepout on In1.Cu, probed in INTERNAL units through the harness walk — the full flag×net discriminator, every row and column: plane×foreign STAYS (THE CROSSING CELL: base `!contains`=true vs virtual `false && …`=false), plane×own drops, keepout×foreign stays, keepout×own stays; a no-ignore-nets sanity arm proves the plane reaches the walk at all (non-vacuity), and the pin's premise asserts the two faces genuinely disagree on the item. Rooms are not probed: the only rooms ever in the tree are complete-free-space rooms — `isObstacle(int)` unconditionally true on EVERY face (`:77-84`), an agreement cell by Java's own definition. Mutant (the M-2 divergence re-applied: trait face for ALL keys) DIES at the crossing-cell assert. Suite census 1300→**1301/0/15**. |
| Quality-review minors M-Q1..M-Q4 (review t17b-1) | LANDED + PINNED (2 new pins; 3 mutants all killed) | **M-Q1 (structural hoist):** the two-face ignore-nets branch and the key-dispatch ladder are now SINGLE-SOURCED in `expansion/neighbours.rs` — `ROOM_KEY_BASE` (was THREE consts: engine.rs, drill/pins.rs, expansion/pins.rs, held together by the t11 equality pin), `board_item_is_trace_obstacle(board, key, net)` (was THREE byte-identical copies: the engine free fn + two inherent harness twins), and the generic `ignore_nets_key_is_obstacle<E: NeighbourEngine>(engine, board, key, net)` (rooms → trait face, items → BASE `Item.isObstacle`, `ShapeSearchTree.java:412-413`) — pointed at by BOTH walk call sites (production `engine.rs` ripup walk AND the drill harness). The t11 const pin is REWRITTEN to the boundary-value face (the equality face is enforced by construction now; comparing re-exports of one const is tautological). **The production walk is pinned** (pin mode 13c): new `t17b_production_ignore_nets_walk_filters_items_on_the_base_obstacle_face` (engine.rs tests) re-drives the SAME `PLANE_WALK_DSN` world through `build_engine` — the real `settings_ir` cost table (hardcoded 2-layer, so the world was reduced to F.Cu+In1.Cu; the probe layer In1.Cu keeps index 1, the world's discriminator is untouched) — and the real manager tree, with the full flag×net discriminator incl. the plane×foreign crossing cell and a no-ignore-nets sanity arm. Mutant: trait-face-for-all-keys AT THE SHARED HELPER kills BOTH the harness pin and the production pin (2/2 at the crossing assert). **M-Q2 (hook race):** new `harness::panic_hook` module — `lock()` (caller-held, poison-tolerant static Mutex) + `Silenced` RAII take/silence/restore that deliberately does NOT self-lock (the sentinel pin holds the lock across its `run_detail_pass` call; std Mutex is non-reentrant — the contract is documented in the module and at `run_detail_pass`). `corpus::compare` (the unlocked second participant in the same test binary; its private `PanicHookGuard` deleted) now takes lock+Silenced; `run_detail_pass`'s manual take/restore pair is replaced by the RAII guard; `WATCHDOG_HOOK_LOCK` deleted, the three watchdog pins take `crate::panic_hook::lock()`. Mutant (Q5-analog: guards removed from the two deadline pins) kills the sentinel 3/3 under the default parallel runner; restored, the watchdog subset ran 5/5 green ×12.4s and the corpus golden pin 5/5 green ×0.15s. **M-Q3:** the killed-run skip NOTE names the force-override (`; pass --detail to force it.`). **M-Q4:** `detail_deadline(&RouterFixture)` extracted pure at the loop call site and pinned (`detail_deadline_equals_the_tier_wall_and_scales_across_fixtures`: bm08 120s / bm01 1800s literal walls + tier equality + cross-fixture scaling) — the banked T17B-Q1 mutant (constant 1s) now DIES. Census **1303/0/15** (1301 + the 2 new pins); determinism digests byte-identical (`96ba7300…`/`16714d5e…`) — the M-Q1 refactor is digest-neutral, the proof the hoist changed no behavior. |

## T17c Battery truth (appended by M3-T17c)

**A — workflow tripwire:** commit `cc275b5a5`, 2 pins in `harness/src/ci_tripwire.rs` (rust-check.yml keeps `timeout-minutes: 30` on the test step AND `--report-only` on the compare step; both mutants kill). The CI exit flip remains DEFERRED by design — when it lands it must update these pins in the same commit. CENSUS PROTOCOL (spec-review survivor T17C-S4): the bare-suite census (1305, pins green BY NAME) is the only guard on test-module wiring — deleting a `mod` declaration in main.rs unwires a module silently (1305→1303, nothing red); any census drift is a failure.

**B — bm06 root cause: CLASSIFIED TUNING, no code change (buglog 181).** Attempt-stream diff (production-world captures both sides, rows aligned on the invariant fields result/incompletes/netIncomplete/ripped): P1 attempts 1–41 align exactly; the first divergence is attempt 42 (net 37) where JAVA rips (inc stays 58) and RUST finds a LEGAL RIP-FREE path (inc 57, violations 8 on both sides) — Rust's search space strictly CONTAINS Java's, the reverse of the missing-path signature, so this is not a parity bug by the task's own definition. Everything downstream is victim-choice drift on the bistable U9-adjacent pair (nets 12/13): Java's net-13 re-route rips net-12's VIA 841, Rust's rips TRACE 903; Java's net-12 re-route rips net-1 335, Rust's rips net-5 1199 — identical decomposition keys throughout, different geometry. From P2 on, nets 12/13 swap the incompleteness every pass (each re-routes by ripping the other), score-neutral from P6 (61↔62 ripupCosts war), and the P18 stagnation stop lands on whichever net holds the conflict at the stop pass (Rust: net 12). Reachability is PROVEN — each net completes on the alternating passes — so no capability is missing: "same space, worse order" = TUNING, an M4 problem (levers: maze victim-choice tie-break alignment + rip-free-path exploration order). Final faces: Rust 10 incompletes vs Java 9 (completion RED — the only failing gate), score 862.79 ≥ 876.39−2% GREEN, violations 8 ≤ 8 GREEN. Diagnostic-only sibling found on the way (buglog 180): Rust's stagnation report prints ALL collapsed Delaunay candidate pairs (`row.edges`, incompletes.rs) while Java's `AutorouteUnroutedReport` prints Kruskal-accepted airlines only — count AND content diverge, LOG-OLD, cosmetic.

**C — the wall reds speak truth; TIER-TIMEOUT POLICY CONFIRMED (buglog 175 updated).** bm01 ladder arithmetic: every pass burns its FULL tick-ladder cap (100000·2^(pass−1)) — clean battery walls P1–P4 = 94.24/234.79/268.74/780.92 s at ms/tick 0.94/1.17/0.67/0.98 (score 466.67→623.93→699.15→774.36, unrouted 78→55→44→33, violations 0 throughout); contended one-off P5–P6 = 1227.90 s (822.22/26) / 991.60 s (849.57/22) at 0.77/0.31 ms/tick. The ms/tick is congestion-dependent (0.31–1.28), NOT constant: wall = ladder ticks × a growing board-state cost. The ladder doubles every pass, so Java's 20-pass end (106.37 s total, 986.32/2) maps to a Rust natural end of ≈ 102.3M ticks ≈ 8.8–36 h at the observed rates — even the single best observed ms/tick (0.31) gives ≈ 31,700 s, far above every bound in play; and the stagnation stop (10 consecutive passes with <0.5 gain) is unreachable because bm01 was still gaining +27/pass at P6. VERDICT: bm01 is WALL-LIMITED, not search-limited. Policy (finalizing the T17b stance): tiers.yaml stays EXACTLY as committed — the CI compare step's 30-min workflow wall cannot absorb a bump — the battery row for bm01 is an honest red BY WALL, the one-off protocol (release profile, generous EXTERNAL timeout, never in-process unbounded, never kill anything not yours) is the truth mechanism for wall-limited fixtures, and the lift is M4 speed work (per-pass search cost), never tier edits. (OUTCOME CORRECTION, final harvest: the 21600s one-off bound fired mid-P14 after THIRTEEN completed passes — not the P8–P9 stop estimated from the P6-era data; see D for the full trajectory. The stagnation-unreachability argument STRENGTHENED with the final data: the engine was still gaining +34/pass at P12.) bm11 TRUTH (natural stagnation end, 18 passes = the SAME pass count Java takes, total wall 1088.5 s vs Java 21.19 s): COMPLETED, score 875.00, incomplete 15/160, violations 0 — vs Java 883.33 / 14 / 0. Gates at truth: score GREEN (≥ 883.33−2% = 865.66), violations GREEN (0 ≤ 0), incomplete RED by 1 — wall is NOT the cause (the battery's 600 s kill masks an already-plateaued board; the terminal plateau 875.00/15 holds from P4 through P18); the single missing connection is attempt-order/victim-choice divergence, the same M4 tuning family as bm06's, not a missing path (the plateau is score-stable, not crash-limited).

**D — TRUE final inventory (final tree = fa0a8eaad + this amendment).** The OFFICIAL battery is the RELEASE profile (`cargo run --release -p epic-harness -- router compare`; the T17b baseline shape "8 green / 3 red, bm06 quality" is release — run the battery with `--release`): **8 PASS / 3 RED, exit 1** — bm02 0≤1/0/1000.00 (3.0s, 5 passes), bm07 0≤2/0/1000.00 (14.6s, 7), bm08 0/0/1000.00 (0.2s, 1), bm09 1≤1/0/988.51 == Java exactly (20.6s, 18), ecc83-pp 0/0/1000.00 (0.2s, 1), ecc83-pp_v2 0≤0/16≤16/991.32 == Java (0.2s, 1), pic_programmer 1≤1/1≤1/989.19 == Java (4.8s, 18), sonde xilinx 0≤1/0/1000.00 (0.6s, 2). The three REDs, each triaged: **bm06 = QUALITY red only** (incomplete 10 vs Java 9; violations 8≤8 GREEN; score 862.79 ≥ 876.39−2% GREEN; wall 88.0s, 18 passes, traces 114 / vias 10 / bends 464 — the TUNING case of buglog 181: bistable nets 12/13 victim-choice drift, reachability proven), **bm11 = battery wall red + completion red by 1 AT TRUTH** (one-off natural end: COMPLETED, 875.00, 15/160, violations 0, 18 passes — the SAME pass count Java takes — total pass wall 1088.5s vs Java 21.19s; score GREEN ≥ 865.66, violations GREEN; the missing connection is attempt-order divergence, the M4 tuning family — wall is NOT its cause, the 875.00/15 plateau holds P4–P18), **bm01 = WALL-LIMITED** (one-off: 13 passes completed under the 21600s external bound, then killed mid-P14 — timeout exit=124, no manifest by design; trajectory 466.67/78 → 774.36/33 (P4) → 849.57/22 (P6, P7 flat) → 890.60/16 (P8) → 904.27/14 (P9) → 917.95/12 (P10) → 931.62/10 (P11) → **965.81/5 (P12, best)** → 958.97/6 (P13, first regression — tolerated, next restore point P16 unreachable); vs Java's terminal 986.32/2 at 20 passes the engine was 20.51 points / 3 connections short WITH THE SCORE STILL CLIMBING (+34 at P12) when the bound fired). GROWTH-LAW REFINEMENT (final data): the ladder binds only on runaway searches — P1–P10 exhausted their caps at congestion-dependent ms/tick (1.28 falling to 0.077 as the board emptied), but P11 finished well under its 102.4M cap (2365s, 0.023 ms/tick) and P13 faster still (1481s): as the incomplete set shrinks, later passes run FASTER than earlier ones despite the doubling ladder. NATURAL-END BOUND (corrected in the spec-review round — the earlier "~5–9 h" was a remaining-time magnitude presented as a total, and its floor sat below the 5.66 h already measured): 20,384 s = 5.66 h is MEASURED through P13; the 7 remaining passes extrapolate at the observed uncontended per-pass wall range 1,481–3,955 s (P13 min … P10 max) → remaining ≈ 2.9–7.7 h, so a natural 20-pass end totals ≈ 8.5–13.4 h — above every tier and CI wall in play, below the earlier 36 h worst-case extrapolation. DEBUG-PROFILE NOTE: a debug-profile battery on the same tree (run first, before the profile was identified) integrity-kills ALL THREE wall fixtures — bm06's debug pass cost is ~8× release (P1 13.79s vs 1.76s), pushing it over its 600s tier — recorded as an integrity-only observation, NOT a verdict source; the verdict rows above are release. All ten non-bm01 rows carry deterministic outcomes (score ties with Java on bm09/pic_programmer/ecc83-pp_v2 to the second decimal); zero "?" rows remain. |

**MILESTONE-CRITERION MAP (design §6 M3 → the D-table faces; T17d input):** criterion 2 ("Tier A per-fixture completion ≥ Java and violations ≤ Java") reads the table's incomplete and violations faces and is FACE-INSENSITIVE this milestone — every red is red at tier AND at truth (bm01: 33 at the P4 tier-kill, 5 best, 6 terminal — all > Java 2; bm11: 15 at both > 14; bm06: 10 at both > 9) with violations ≤ Java everywhere (0 introduced), so no adjudication outcome flips on the choice; criterion 1 ("single-net quality ≥ Java") has NO D-table row — its evidence face is the single-net parity pins and capture gates of the M3 single-net tasks (SEAM/gate history), which the adjudication must cite explicitly; and the harness score gate (R ≥ J−2%) is NOT a §6 criterion — bm01 fails it under both faces while its §6 violation is completion, and bm11 PASSES it while failing completion by 1, so the two vocabularies must not be substituted for each other. |

## T2 expansion-room door-slicing checkpoint (appended by M4-T2)

Task 2 set out to port a hypothesized Java-exact door-partitioning rule
difference (the Class-B signature of buglog 173) and to re-measure the
buglog-181 levers after it. The premise did not survive dual-engine
measurement; the partitioning port is already Java-exact. What follows is
the disconfirmation dossier, the parity pins it left behind, and the
unchanged checkpoint. The Java wins every conflict.

| Seam | Status | Notes |
|---|---|---|
| ExpansionDoor section partitioning (`getSectionSegments`, `ExpansionDoor.java:105-143`) | VERIFIED JAVA-EXACT (pins T2-P1..P4; mutants T2-M1/M2/M3) | The arithmetic Java-exactly as ported: offset = offsetParam + `TRACE_WIDTH_TOLERANCE` (2.0, `AutorouteEngine.java:41`); dim=1 → `diagonalCornerSegment().shrinkSegment(offset)`; dim=2 both-complete-free-space → `calcDoorLineSegment` (corners on BOTH room borders) + the small-door gate `distanceSquare < 4*offset²` → empty (`:128-131`); otherwise the gravity-point zero-length line; count = `(int)(L / (10*offset)) + 1`; `divideSegmentIntoSections`. Ulp-critical op order is faithful: Java `shrinkSegment` computes `newB = a + d·(L−effOff)/L` (NOT `b − off·û`), and the divide snaps the LAST endpoint to the exact `b` (`FloatLine.java:210-272`). Pins in `expansion/door.rs`: T2-P1 the e1 full-wall door (7 sections, literal coordinates (227502..972498, y=238750) = golden ords 58-64), T2-P2 the e1 dim=2 free-space trapezoid (4 sections on the restraint line (539482,91250)→(216250,233750), Java-double literals to the last ulp = golden ord 65+), T2-P3 the small dim=2 door NOT expanded, T2-P4 the gravity-point arm. Mutants: T2-M1 (count formula `+1` dropped → both count pins die), T2-M2 (naive `b − off·û` shrink → ONLY T2-P2 dies, and only via its ulp literals — counts survive; proof the line assertions add observability beyond counts), T2-M3 (tolerance `+2.0` dropped → line literals die with counts unchanged). RAW PROVENANCE (quality-review-t2-1 MINOR-1): the Java-leg instruments are COMMITTED — `rust/harness/oracle/RoomSnapProbe.java` (the T2-P1 world + T2-P2 room-label witness) and `rust/harness/oracle/RoomDiagProbe.java`; the rescued raw dumps (e1-java-snap.jsonl, e1-rust-diag4.txt, bm06/bm11 checkpoint manifests + stderr) live in `logs/M4-T2/evidence/` (git-ignored, README names each file's witness row). |
| The Class-B DIVERGENCE FACES (buglog 173) | DISCONFIRMED AS DOOR-SLICING — reclassified DOWNSTREAM-OF-GEOMETRY (T6 scope) | **SUPERSEDED by the T4 row below — e1 closed end-to-end, t7 now 1-based 74** (this row kept for the classification history; do not plan from its ordinals). Dual-engine measurement of e1's decisive board state: Java item 20 (the net-1 route) is the STRAIGHT bar `(1000000,250000) (200000,250000)` with ONE tree shape; Rust item 20 is a 6-corner dogleg `(1000000,250000) (600000,250000) (600000,260682) (588750,271932) (221932,271932) (200000,250000)` with FIVE tree shapes, bulging over the ripped prewire column. Every witnessed face is a consequence of that upstream found-path/insert/pull-tight geometry divergence, not of partitioning: the extra Rust shapes create ObstacleExpansionRoom 20481 + extra doors + the extra incomplete room above the wall, which changes WHICH doors/walls exist at each assign ordinal (e1 58: Java's 7-section full wall vs Rust's 4-section sub-range are doors of DIFFERENT boards — the SAME count formula `(int)(L/112520)+1` on different spans: Java wall [216250,983750] = 767500 → 7, dogleg-mitered Rust door span [595340,983750] = 388410 → 4 (595340 = 600000 − 11250·(√2−1), the 45° miter meeting the wall line); t9 1350 and t7 23 likewise; the seed faces show door SETS differing, 5 vs 4 — no order-of-equal-sets verdict exists to pin). The events compare stays honestly RED at the SAME ordinals (e1 58 — first differing field now localized to `expansionValue` 469613.0463275057 vs 348328.28723786207, the maze priority computed over the divergent board; t7 23; t9 1349-prefix triage pins all green). DEFERRED: an enumeration-ORDER pin would be an identity pin today (cerebrum mode 11) — it needs an `events id-order` capture pipeline; see BACKLOG. |
| Causal-ancestor checkpoint (buglog 181) | RUN — BOTH UNMOVED (honest null result: no engine change landed, so no movement was possible; the lever stays the T12 tie-break family) | bm06 single-fixture RELEASE (fanout-off argv, external timeout): COMPLETED, exit 0, 10/98 incompletes, 8 violations (0 router-introduced), 862.79, 18 passes — vs Java truth 9/98, 8, 876.39, 18 and vs M3-exit Rust 10/98, 8, 862.79, 18: identical. bm11 single-fixture RELEASE: COMPLETED, exit 0, 15/160 incompletes, 0 violations, 875.0, 18 passes (plateau held P4–P18, ~70 s/pass) — vs Java truth 14/160, 0, 883.33, 18 and vs M3-exit Rust truth 15/160, 0, 875.00, 18: identical. Both one-connection gaps unchanged. |
| Ripple (all compares `EPIC_SKIP_GRADLE=1`, real exits) | BYTE-UNROTATED | corpus 5000/5000 exit 0; dsn compare 1332 fixtures 0 mismatch (digest 175/175, soak 1157/1157) exit 0; index 33/33 exit 0; undo 33/33 exit 0; drc 17/17 exit 0; events exit 1 at the unchanged e1-58 face (above); `router determinism` exit 0, digests UNROTATED: ses `96ba7300c26dc153…`, manifest `16714d5ee3dff007…` (== M3-exit/T1 citations; no SEAM/README rotation). Battery CHEAP release: bm08 exit 0 (0/0/1000.00), ecc83-pp exit 0 (0/0/1000.00), ecc83-pp_v2 exit 0 (0/16/991.32) — value-identical to the T17c D-table rows. |

**BACKLOG (banked residuals riding the future `events id-order` capture pipeline):** (1) the T1 FORMAT tripwire pins the probe's DECLARED constant, not its emitted bytes — a pipeline that captures the probe's stdout would let the pin assert the emitted header too; (2) the deferred T2 seed/door ENUMERATION-ORDER pin — once id-order capture exists, the per-assign door-id sequence becomes pinnable Java-exactly (the current faces cannot support it: the witnessed door SETS differ, so any order assertion there would be vacuous or an identity pin). Both wait for the same harness capability; neither blocks T6.

**BACKLOG (next touch of `expansion/door.rs`; spec-review MINOR-3, mode-13 observation):** two `getSectionSegments` arms remain unpinned — the EMPTY door-shape arm (Java `:109-112`, `(0, empty)` before any dimension dispatch) and the both-complete-NULL arm (Java `:124-127`, fewer than two distinct common corners → null → `(0, empty)`; "a complete room inside the other"). Neither is exercised by the witnessed faces (and T2-P3's small-door `(0, empty)` runs through neither code path), so a mutant mis-deriving the empty-check arity survives today. Both need only crafted shapes, no new capture capability — **CLOSED by M5-T7**: both arms are pinned (mutation-verified) at the next edit of door.rs — see the M5-T7 dossier's door.rs row below.

## T3 TraceTightener base + 90° variant (appended by M4-T3)

Java's changed-area pull-tight layer — `board/optimize/TraceTightener.java`
(547 l) + `TraceTightener90.java` (169 l) — is ported into
`epic-board/src/trace_tightener/` and wired behind the WIDENED
`PullTightSeam` as the production `TraceTightenerSeam`. All named
divergences (dead acid-trap body, wall→tick budget, reference-vs-value
equality proxies, `changeEntries` collapse, inert smoothen arm on 90°,
T5 ViaOptimizer stub, kept `contactPins` leak) are documented in the
`trace_tightener/mod.rs` module docs; the jar-leg instruments are
committed (`rust/harness/oracle/TraceTightenerProbe.java`, five worlds +
`regionscan` + `cornerworlds` modes) and the raw captures rescued to
`logs/M4-T3/evidence/` (README names each witness row).

| Seam | Status | Notes |
|---|---|---|
| The port (`TraceTightener` base + 90° variant) | COMPLETE (10 pins; mutants T3-M1..M6) | Base: the `while somethingChanged` fixpoint over the LIVE marking session (region `setEmpty` BEFORE processing, enlarge `1.5*(maxValue(layer) + 2*maxTraceHalfWidth)` — int-wrapping inside the parens, double multiply outside; default-tree descending-id item loop; the two break disciplines: keep-point split breaks the item loop, successful smoothen breaks it "because items may be removed"); `repositionLine` binary search (clip double-corner gate, same-side gate, `first_time` biggest-change break, changed-area joins on acceptance); `skipSegmentsOfLength0` (exact-corner retention at both ends, `c_min_corner_dist_square` 0.9 in the middle, the constructor-collapse veto on the 45°-line perf shortcut); `smoothenEndCornersAtTrace1` with the remove/reinsert/double-remove/split/normalize ladder and the KEPT `contactPins` leak on the keep-point early return. 90° variant: `pullTight` fixpoint (skip-second-corner → skip-corners → reposition), `trySkipSecondCorner` checking BOTH offset shapes (Java loop bound `i < 2` — the pin-caught port bug: my first draft wrote `0..3`, killed by the stair pin's offset-shape panic; buglog 183), `trySkipCorners` with the `second_last_corner_skipped` tail and the kept-line indexing walk. The smoothen-corner overrides answer null on 90° (Java `:161-168`), so the whole smoothen arm is inert here — the real bodies land with T4's 45° variant. |
| `PullTightSeam` widening + production swap | COMPLETE (3 consumption points → 5 call sites; `NoPullTight` retained for tests/oracle) | The seam gained `trace_costs` + `stoppable` + `time_limit_millis` + `deterministic_budgets` (Java's wall-clock `TimeLimit` re-faced as the deterministic tick budget — one tick per `is_stop_requested` consult, strict `>`, the `RouteBudget` pattern; `timeLimit > 0` construction gate kept). Production call sites swapped to `TraceTightenerSeam` (spec-review-verified numbers): `connection_router.rs` :209/:236 (inside `route` — the engine pass and its `opt_changed_area`) and :394/:411 (inside `retry_connection_necked` — the same pair; the dispatch said "two sites", there are THREE points = FIVE call sites) + `batch.rs` :807 (the pass-tail opt). `NoPullTight` survives only in `#[cfg(test)]` mods (`routing_board_insert.rs`, `engine.rs`, `inserter.rs`; zero uses in `rust/harness/`). All tier fixtures are 45° (census below), so `active_for` gates every production call to the T10c no-op face — the swap is dispatch-active but behaviorally dormant until T4; the determinism digests prove it. |
| Pins (`trace_tightener/pins.rs`, 10 fns; suite 1311→1319→1320→**1321/0/15**) | JAR-LITERAL (probe capture `logs/M4-T3/evidence/tightener90_capture_run3.jsonl`, double-run byte-identical; reviewer re-derived every literal byte-identically) | Fixture `fixtures/trace-tightener/tightener90.dsn` (crafted: 90°, rule width 200/clr 250, offset 6750 DBU measured via regionscan boundary x* = 992250 = box edge 999000 − 6750, NOT assumed). stair = the 6-corner→3-corner collapse + fixpoint termination + region selectivity; acid = the `avoidAcidTraps` `if (true) return polyline;` dead-body identity (crossing trace would be wrapped by an activated body); region in/out = the 2×2 offset-arithmetic discriminator (M1's boundary shift flips the in-cell); **offset-boundary exact pin (spec-review MINOR-2 fix, `t3_region_offset_boundary_exact_pin`)** = the reach boundary asserted AT 992250: 992249 out-arm byte-exact, 992250 and 992251 collapse to the west bar (witness: regionscan bracket + closed form); block = the cross-trace two-sweep fixpoint (descending-id runs blocked-B first, stranded until sweep 2 re-marks it); corner-worlds = jar finals for the `last-corner-skip.dsn` census fixture (5 nets, the `i == len` arm fires on 3 of 5); **45°-inertness pin (quality-review MINOR-1 fix, `t3_seam_inert_on_fortyfive_degree_boards`)** = the seam's 45° face asserted BYTE-EXACT end to end: arm (a) the locator `t9_locator45.dsn` (no routed traces — structurally inert, pins the gate face; survives T3-M6 by construction, disclosed) + arm (b) the crafted 45° staircase `tightener45.dsn` — the killer: mutant T3-M6 collapses the staircase and dies at the assert (gate-open ALONE stays green by design). Budget/stopper pins (partial face, flag face) have NO jar counterpart — Java's budget is wall clock — disclosed as Rust-face pins. |
| Self-review mutants | M1 KILLED, M2 SURVIVED→BANKED INERT (reach+heal evidence, ~90 obs; reviewer T3-S5 reproduced), M3 KILLED (after cap-count fix), M4 KILLED (after re-aim), M5 KILLED BOTH WAYS (spec-review fix round), M6 KILLED (quality-review fix round) | M1 (drop `2*maxTraceHalfWidth`, offset 6750→3750): region pin dies at the in-cell. M2 (kill `second_last_corner_skipped`): survived every pin — instrumentation proved the arm FIRES (24 firings in a pinned face sweep) but heals INSIDE sweep 1 at every budget face; coverage pin added so a future heal-break surfaces. T4-revisit condition (folded into the pins.rs corner-worlds caveat): TWO banked healers — the M2 flag arm AND quality-review T3-Q4's removal of `reposition_line`'s `first_time` biggest-change break (Java `TraceTightener.repositionLine :313-315`), which survives 9/9 while firing 8x (the fixpoint converges to the same finals; witnessing the break needs a world where first-accepted ≠ fixpoint, i.e. a clearance-borderline bisection). If T4 changes `reposition_lines`/Polyline construction, re-probe BOTH. M3 (activate the acid-trap dead body): first stub inert because I guarded on `lines.len() == 1` — open polylines carry TWO end caps, a 1-segment trace has 3 lines; corrected `== 3` stub killed by four pins. M4 (one-sweep early exit): first attempt broke `pull_tight_90`'s INTERNAL while (Java's own per-trace loop) and survived; the two-sweep story lives in the OUTER `opt_changed_area` fixpoint — the re-aimed mutant dies at the block pin. M5 (±1-DBU offset drift, closing reviewer T3-S4's survived ±1): offset `+1.0` (6751) dies at the boundary pin's 992249 out-arm, offset `−1.0` (6749) dies at its 992250 boundary arm (`mut_T3-M5_offset{6751,6749}_killed.log`). M6 (composite gate-open + 45°→90° mis-dispatch, closing quality-review T3-Q1's dormancy gap): `active_for` wired open AND `pull_tight_polyline`'s `_ =>` arm dispatched to `pull_tight_90` — the T4 mis-integration shape; gate-open ALONE is inert by design (the fallback is identity; matches T3-Q1), so this composite is the honest killer — dies at the inertness pin's `tightener45.dsn` staircase arm, exit 101, geometry reshaped (`mut_T3-M6_gateopen_90dispatch_killed.log`). All edits by edit + edit-back `cmp` byte-exact; logs in `logs/M4-T3/evidence/`. |
| Angle census (the T3 activation gate) | **0 ninety / 23 fortyfive / 0 any-angle** over the 23 `tiers.yaml` fixtures | Ground truth sharpened from the earlier grep: NO tier fixture carries a `snap_angle` keyword at all — all 23 parse as `FORTYFIVE_DEGREE` through the JAVA PARSER DEFAULT (`io/specctra/parser/ReadScopeParameter.java:59`, overridden only when the keyword is present, `RulesReader.java:149-151`); paths resolved at `scripts/benchmark/fixtures/` (incl. the space-bearing `sonde xilinx.dsn`). The T3 seam is therefore a NO-OP on every battery fixture — the gate faces below are expected to be byte-stable, and are. |
| Ripple (all compares `EPIC_SKIP_GRADLE=1`, real exits) | BYTE-UNROTATED (dispatch expected rotation; census explains the absence) | corpus 5000/5000 exit 0; dsn compare `--set all` 1332 fixtures 0 mismatch (digest 175/175, soak 1157/1157) exit 0; ses-compare 20/20 byte-equal exit 0; ses-snap 5/5 exit 0; index 33/33, undo 33/33, drc 17/17 all exit 0; events exit 1 at the UNCHANGED T6 face (e1 ordinal 58, `expansionValue` 469613.0463275057 vs 348328.28723786207 — the M4-T2 dossier's downstream-of-geometry divergence, not a T3 regression); `router determinism` (release) exit 0, digests UNROTATED: ses `96ba7300c26dc153…`, manifest `16714d5ee3dff007…` (== M3-exit/T2 citations). Battery CHEAP release: bm08 0/0/1000.00, ecc83-pp 0/0/1000.00, ecc83-pp_v2 16≤16/0/991.32, all exit 0 — value-identical to the T17c D-table rows. Gates: fmt exit 0; clippy `--workspace --all-targets -D warnings` exit 0; `cargo test --workspace` 1319 passed / 0 failed / 15 ignored. |

## T4 TraceTightener45 — the 45° variant goes LIVE (appended by M4-T4)

Java `board/optimize/TraceTightener45.java` (674 l) is ported to
`epic-board/src/trace_tightener/tightener45.rs` (877 l post-fmt) and the
T3 activation gate is OPENED: `active_for` (`mod.rs:1176`) returns true
for everything but `AngleRestriction::None`, so every production pull-tight
call site wired in T3 — `batch.rs:807` (pass-tail opt) and
`connection_router.rs:209/:236` (inside `route`) / `:394/:411` (inside
`retry_connection_necked`) — now runs the REAL 45° fixpoint. The any-angle
variant stays DEFERRED behind the pinned loud-failure stub (census: 0
any-angle fixtures; the parse default is FORTYFIVE_DEGREE,
`ReadScopeParameter.java:59`).

| Seam | Status | Notes |
|---|---|---|
| The port (`tightener45.rs`) | COMPLETE | `pull_tight_45` (`:58`, Java `:35-45`): acid-trap identity → the reduce-corner → smoothen-corner → reposition fixpoint (value-equality proxy for Java's reference `!=`, documented); `reduce_corners` (`:86`, Java `:52-221`) — the 4-slot corner window, both clip-gated translation arms with their `checkTraceShape` clearances; the REAL smoothen bodies the 90° variant stubs: `smoothen_corners` (`:315`), `smoothenSharpCorner` (`:366`, the check-free `(sqrt2−1)*halfWidth` shave), `smoothenNonIntegerCorner` (`:410`), `smoothenCorner` (`:473` — greedy bisection; tie `prevDist <= nextDist` picks PREV at `:495`, the `translateDist == maxTranslateDist` biggest-change break at `:547`), and the two trace-contact overrides `smoothenStartCornerAtTrace` (`:591`) / `smoothenEndCornerAtTrace` (`:736`, turn-45 choices SWAPPED at `:822-826`). Tie faces measured + banked as healers (mutants below). |
| Activation + probes | LIVE | The T3 dormancy pin `t3_seam_inert_on_fortyfive_degree_boards` is CONVERTED to liveness: `t4_staircase_liveness_jar_final` (`pins.rs:190`) asserts the crafted 45° staircase's jar-witnessed COLLAPSED final through the live production dispatch. Probe extended for 45° worlds (`rust/harness/oracle/TraceTightenerProbe.java`, committed); three NEW crafted fixtures (`tightener45_diag.dsn`, `tightener45_smooth.dsn`, `tightener45_tail.dsn`) with double-run jar captures (run3/run3b byte-identical pairs, `logs/M4-T4/evidence/` README names every witness). |
| Pins (`pins.rs`, census **1321 → 1325/0/15**) | JAR-LITERAL + boundary | 4 NEW: `t4_diag_corner_worlds_jar` (`:941`), `t4_smooth_corner_worlds_jar` (`:963`), `t4_tail_deferral_counter_witness` (`:1009` — the `PolylineTrace.pullTight` PIN-CONNECTION TAIL deferral counter-witness: the un-ported `swapConnectionToPin`/`correctConnectionToPin` tail (Java `:842-855`, disclosed at `mod.rs:78-85`; T6 owns the port) is isolated as the ONLY face the tail world moves), `t4_skip_length0_min_corner_dist_boundary` (`:1049` — the exact 0.9 boundary, ±1 mutant-killed both ways). UPDATED: `t3_dispatch_active_for_census` (`:384`) now pins the LIVE dispatch contract (45°→`pull_tight_45`, 90°→`pull_tight_90`, `None`→no-op loud stub, `t9_locator_any.dsn`→false — the ANY-ANGLE deferral face is pinned HERE, not on the tail pin). |
| Mutants + re-probes (logs in `logs/M4-T4/evidence/`) | M1/M2 HEAL (banked), M3/M4/M5 KILLED; T3-M2/T3-Q4 STILL HEAL | T4-M1 (`reposition_line` strict nearer-corner pick flipped): HEALS — fires 22× across the probe sweep / 11× across the 45° pin suite, converges identically (`mut_T4-M1_tie_reach_*`). T4-M2 (`smoothen_corner` tie `<=`→`<`): HEALS — fires once on the symmetric chamfer world, same final. T4-M3 (dx/dy swap of the sharp-corner shave anchor): KILLED (termination face). T4-M4 (composite 45°→90° mis-dispatch): KILLED — the required mis-dispatch death at the liveness pin. T4-M5a/b (`c_min_corner_dist` 0.9→1.5 / 0.4): KILLED both directions. RE-PROBES (dispatch obligation): T3-M2 (`second_last_corner_skipped` off) and T3-Q4 (`first_time` biggest-change break removed) re-run under the 45° suite — both STILL HEAL (14/14 pins green under each mutant; `reprobe_T3-M2_*` — filename is a misnomer, the log shows survival — and `reprobe_T3-Q4_*`); re-banked. **CONSOLIDATION (quality-review MINOR-2): T6 owns the ledger close** — it already ports the pin-connection tail (`swapConnectionToPin`/`correctConnectionToPin`) and the two remaining Class-B faces, and the tail port touches reposition/normalize machinery, the natural moment to re-probe all four healers once more and either upgrade or permanently close them (T12 is TOO LATE — the optimizer re-bases geometry deliberately). The ONE crafted world that can settle all four entries at once: a clearance-borderline bisection whose FIRST accepted translate differs from the fixpoint final (the shared revisit condition recorded at the pins.rs corner-worlds caveat) — witnessing that break upgrades-or-kills T3-M2, T3-Q4, T4-M1, T4-M2 simultaneously. **T5 ADDITION (quality review): the settlement is TWO worlds, not one** — T5-M1 (the collinear-arm healer) is geometrically unrelated to that bisection; its revisit condition is a NON-UNIFORM-COSTS collinear world (under uniform costs the acute arm's else branch re-derives the identical call, so the healer stays invisible). T6 therefore owns TWO crafted worlds: the bisection world (settles T3-M2/T3-Q4/T4-M1/T4-M2) PLUS the costs-collinear world (settles T5-M1). |
| Events re-triage (#61 milestone on e1) | e1 CLOSED END-TO-END; t7 moved to 1-based 74; t9 holds at 1-based 1350 | With the tightener live, `events compare` (`events_compare_post_t4_full.log`): **e1_ripup is BYTE-IDENTICAL to the committed Java golden across the whole pinned stream** — 337 trace rows (331 assign + 1 skip + 1 ripped + 4 routed), returned, incompletes 0. The former first divergence (1-based 58, Class B door-partition: Java sliced keepout wall 216250..983750 into 7 sections, Rust 595340..983750 into 4) is CLOSED — pass-1 geometry is now Java-exact, so every downstream door slicing/choice matches. The T7-vs-t7 route-count drift (7 Rust vs 8 golden attempts) is ALSO closed (7,7). Remaining reds, both Class B expansionValue-CHOICE family (T6 scope): t7 first divergence MOVED 1-based 23 → 74 (golden expands target door `item=6` at 62570.28048522717, Rust at 821846.2953107631; the tightener pushed it past the former 23); its id-churn face MOVED to route row 3 (netItems 2→11 vs 2→12, maxItemId 93 vs 107 — rows 0-2 byte-equal). t9 holds at 1-based 1350, Rust-side value MOVED with the tightened geometry (322104.85468532494 → 321502.98082895903; wall door x=475440 golden vs 315497 rust, both 16 sections). Pins re-triaged with measured literals in `route_events.rs` (pin 2 `:2028` triage via the `EventTriage` alias `:2095`; pin 1 `:2244` count face now all-fixtures equal, probe index t7→3; pin 3 `:2403` rotation 395→331). |
| Determinism rotation — REAL, was masked (bug-184) | ses ROTATED `96ba7300c26dc153…` → `8bd773fca5767948…`; manifest `16714d5ee3dff007…` UNCHANGED | The first post-T4 `router determinism` run reported the OLD digests; an A/B (seam forced dormant via edit + edit-back) reproduced them, nearly documenting a false "bm08 is tightener-inert". Root cause: the gate spawns `target/release/epic-cli` and `-p epic-harness` rebuilds it only as a LIBRARY dep — the stale T3-era bin (45°-dormant) silently routed (proven: touch `crates/epic-cli/src/main.rs` + `cargo build -p epic-harness` leaves the bin mtime unchanged). Fresh-built live bin rotates the SES (dormant 6072 B vs live 4855 B — straightened output, A/B ses pairs in evidence); the MANIFEST digest is UNCHANGED (counts/score invariant — the buglog-176 pattern). Rebuild caveat added to the `router_compare.rs` module docs and `rust/README.md`. (Quality-review MAJOR-1 then closed the remedy STRUCTURALLY: the reviewer followed the prose caveat exactly and still reproduced a green gate on the stale pre-T4 `target/debug/epic-cli` via the profile-sibling axis — so the gates now resolve through `resolve_fresh_epic_cli`, which bails naming the bin and BOTH rebuild commands whenever the resolved bin is older than the newest crate source, and `resolve_epic_cli` prefers release and never adopts a debug sibling; before/after in the report's quality-review fix round.) |
| Ripple (all compares `EPIC_SKIP_GRADLE=1`, real exits) | PARSE FACES BYTE-STABLE; ROUTED FACES CONVERGE | corpus 5000/5000 exit 0; dsn compare `--set all` 1332 fixtures 0 mismatch (digest 175/175, soak 1157/1157) exit 0; ses-compare 20/20 byte-equal exit 0; ses-snap 5/5 exit 0; index 33/33, undo 33/33, drc 17/17 all exit 0; events exit 1 triaged above (T6 scope). Battery CHEAP release, gate mode, external `timeout 1500`: bm08 0/0/1000.00, ecc83-pp 0/0/1000.00, ecc83-pp_v2 0/16/991.32 — all exit 0, value-IDENTICAL to the T17c rows (the tightener rotates geometry, not scores). buglog-176 pin rotated: ecc83-pp_v2 detail pass now (14 traces, 0 vias, 19 bends), was (15, 0, 66) (`router_compare.rs:2448/:2469`); score 991.32 / 0 unrouted / 16 violations unchanged vs the Java record. Gates: fmt exit 0; clippy `--workspace --all-targets -D warnings` exit 0 (the 8-field triage tuple tripped `type_complexity` ONLY under `--all-targets` — mode-14 face witnessed again — factored into the `EventTriage` alias, no allow); `cargo test --workspace` **1325 passed / 0 failed / 15 ignored**. |

## T5 ViaOptimizer — the via arm goes LIVE (appended by M4-T5, spec-review fix round)

Java `board/optimize/ViaOptimizer.java` (733 l) is ported to
`epic-board/src/trace_tightener/via_optimizer.rs` (1128 l) and the T3/T4
seam's third consumption arm is OPEN: `opt_changed_area`'s
`Some(ItemKindTag::Via) if trace_costs.is_some()` arm (`mod.rs`) drives
`opt_via_location` with the production recursion budget 10
(`TraceTightener.java:165` literal). Cost wiring was ALREADY complete at
every seam site (`connection_router.rs`, `batch.rs` — the T3 `PullTightSeam`
widening above), so the arm is LIVE across the pipeline; the T5 determinism
digests are UNCHANGED (`8bd773fca…`/`16714d5e…`) because bm08 is a live-arm
NO-MOVE, corroborated by t9's events healing through the same seam.

| Seam | Status | Notes |
|---|---|---|
| The fresh-algo pull-tight face (`with_fresh_algo_face`, `mod.rs`) | FAITHFUL (spec-review MINOR-2 closed in the fix round) | Java `PolylineTrace.pullTight(boolean, int, Stoppable)` (`:869-886`) builds a FRESH `getInstance` tightener for the via arm's pull-tights (`ViaOptimizer.java:144/:148/:290`, all passing `stoppableThread = null`); the port swaps FIVE state fields for the closure duration — only-net list, clip, budget (`None`), accuracy (the `getInstance` clamp `max(accuracy, 100)`, `TraceTightener.java:112`), and (fix-round addition) the STOP FLAG cleared to `None`, so a via-arm pull-tight never stops mid-tighten, Java-exact. The first draft retained the fixpoint's flag (latent, T5-S3-class); the swap is behavior-neutral on deterministic gates (census 1339/0/15 unchanged, digests unchanged). The RETAINED-fields checklist (keep_point sixth-field candidate, flip condition, Q2 observability note) lives in the face's doc comment (quality-review MINOR-2). |
| Banked survivors (T5; full mutant tables in the review logs) | 3+3 SURVIVORS banked with named revisit conditions | T5-M1 collinear healer (arm dropped: SURVIVED — the acute arm's else branch re-derives the identical call; revisit = T6's costs-collinear world, see the T4 row above). T5-M4/M4b redundant shove-fixed guard (single-site drop SURVIVED, both-site KILLED — permanent banking; the behavior is pinned end-to-end). T5-S1b gate3 outcome-not-predicate (contact gate `!= 2`→`< 2` SURVIVED — that world keeps N2 put even un-gated; revisit = a 3-contact via whose first two descending-id contacts are clean movable traces; recorded in the `t5_gate3_three_contacts_stay` pin doc). Quality-review round: T5-Q1 the wd↔acute arm ORDER is Java-faithful but inert on every current world (revisit = a non-uniform-cost world with scalarProduct > 0 where the wd target differs from the acute target); T5-Q2 the fresh-face stop swap is UNOBSERVABLE until a stoppable-carrying caller exists (noted at the face doc); T5-Q3 the tolerance truncation-vs-rounding face is mode-16 unpinned (revisit = odd-DBU via min width + a within-tolerance-but-nonzero via/endpoint offset). Kills on record: T5-M2 (depth gate), T5-M3 (recursion decrement), T5-M5 (consumption gate), T5-S2 (pin literals), T5-S4 (contact-scan ordering), T5-Q4 (projection literal). |
| epic-dsn comment faces (bug-186) | OPEN — carry-forward, NOT T5 scope | The jar's flex lexer DROPS `#` EOL comments (and `/* */`) via a single ignore action (`SpecctraFileDescription.flex:22` `Comment = {TraditionalComment} | {EndOfLineComment}`, `:25`, wired `:219`); `#` is also a mid-token identifier char (`:34` `SpecCharASCII`) — token-start longest-match picks the whole-line comment. epic-dsn has NO comment handling, so a commented DSN lexes garbage tokens and the wiring scope silently truncates (bug-186: items 14-23 absent, parse "succeeds"). T5 worked around it fixture-side (via_optimizer45.dsn comment-free; jar witness byte-identical before/after). LANDING: M1b-reopen — port BOTH faces through the one ignore action, jar pins on a comment-heavy fixture, 1332-corpus neutrality, truncation-loudness companion (silent truncation must ERROR). Until it lands, crafted fixtures stay comment-free. |

## T6 pin-connection tail + the events close (appended by M4-T6)

The T4-deferred `PolylineTrace.pullTight` pin-connection tail is LANDED
as `trace_tightener/pin_tail.rs` (`check_connection_to_pin`,
`correct_connection_to_pin`, `swap_connection_to_pin`, dispatched by
`pin_connection_tail` from `polyline_trace_pull_tight`), gated exactly
as Java gates it (`angleRestriction != NINETY_DEGREE &&
pinEdgeToTurnDist > 0` — live on every 45° board via the parser default
`pinEdgeToTurnDist = minTraceHalfWidth`, `Structure.java:667-668`).
With the tail live, the LAST events residuals fell to a different root
cause (bug-187): the engine's `DrillEngine::shove_trace_check` was a
T10-era 0.0 stub — Java's `MazeTraceShover.checkShoveTraceLine` calls
the real static `TraceShover.check`, the stub answered "impossible",
every maze shove short-circuited to ripup-only, and the resulting
rip/reinsert op-sequence churn produced the t7 0-based-73
expansionValue choice and the netItems/maxItemId id faces (tree op
order → id churn; the netItems segment ordering rides the id-hash
bucket chain). The engine now delegates to the production
`epic_board::trace_shover::check_max_length`.

**THE EVENTS COMPARE IS GREEN**: exit 0, 3520 golden trace rows aligned
(e1 337 = 331+1+1+4, t7 507 = 496+4+0+7, t9 2676 = 2671+1+0+4), all
worlds returned, incompletes (0, 2) — buglog 172/173/174 CLOSED, the
#61 milestone ledger EMPTY. Both RED-era harness pins re-triaged
GOLDEN-ANCHORED (triage tuples deleted; former divergence ordinals kept
as equality witnesses; Rust-measured residual literals purged).

| Seam | Status | Notes |
|---|---|---|
| The pin-connection tail (`pin_tail.rs`) | LIVE + JAR-LITERAL pins (8 new) | `t6_tail_correct_arm_jar_literal`, `t6_tail_swap_world_outcome_jar_literal` (+ `_notshove_outcome_control`), `t6_tail_swap_fire_jar_literal` (+ `_not_shove_control`), `t6_tail_swap_diag_parse_consolidation_jar_literal` — every expected geometry is a jar capture (`tail45*_worlds.jsonl`, `tail90_worlds.jsonl` in `logs/M4-T6/evidence/`); the not-shove controls pin the `isShoveFixed` gate arm both ways. Gate negatives: `t6_tail_gate_zero_edge_dist_negative` (pinEdgeToTurnDist 0 → inert) and `t6_tail_gate_ninety_degree_negative` (90° board → inert). Fixture bank: `tightener45_tail*.dsn` ×6 + `tightener90_tail.dsn`. |
| Engine shove probe (`DrillEngine::shove_trace_check`) | WIRED to the production shover (bug-187) | The T10-era 0.0 stub → `epic_board::trace_shover::check_max_length` (the ported `TraceShover.check` max-length arithmetic; Java's early-return asymmetry — most gates answer TRUE/try-next-section, a stale `cornerNo` answers FALSE/hard-reject — lives in the PRE-EXISTING T7-era boolean wrapper `check_shove_trace_line` (`maze/shove_probe.rs`), which maps this f64 to the search's verdict, NOT in `check_max_length` itself; spec-review MINOR-2 fix). Mutants: T6-M1 (swap-equality relaxed), T6-M2 (45° gate inverted), T6-M3 (dist gate `>`→`>=`), T6-M4 (stub 0.0 re-applied → the FULL 35-pin + events faces die) — all killed, logs in `logs/M4-T6/evidence/mut_T6-M*.log`. |
| Tightener ledger (T4 consolidation + T5-M1) | FOUR PERMANENT CLOSES (T3-M2, T3-Q4, T4-M1, T4-M2) + T5-M1 CLOSED INERT-BY-WORLD (world-conditional — spec-review MINOR-3 fix) | T5-M1: the costs-collinear ledger world runs the collinear arm with NON-UNIFORM costs — dropped-arm mutant SURVIVES on the jar outcome (identical final; the arm is redundant there) → closed as inert-by-world (`mut_T5-M1_collinear_dropped_t6_ledger.log`, `via45_viaworlds_t6_ledger.jsonl`, pin `t6_collinear_costs_ledger_world_jar_literal`). T3-M2/T4-M1/T4-M2: healers re-run on the 33-pin tail-era suite — STILL HEAL, closed on structural arguments recorded at the pins (T3-M2: the skipped-corner arm re-derives into the same accept; T4-M1: tie-reach converges identically; T4-M2: single symmetric-chamfer world, tie-insensitive). T3-Q4: PERMANENT close by Int-lattice proof + two jar-anchored worlds — for any f64-exact nearest corner the `first_time` break is inert (the overshoot guard crawls back in exact 0.5 steps to the exactly-on-corner translation and re-accepts the identical line), and mid-search dyadic acceptances are transient under the `while somethingChanged` fixpoint (the dyadic world's jar terminal is all-integer: 245459, not the predicted dyadic 249414.0625); bisect world (`tightener45_bisect.dsn` → skip-arm preemption, `t6_bisect45_skip_preempt_jar_literal`) + dyadic world (`tightener45_dyadic.dsn` → the search body, `t6_dyadic45_integer_terminal_jar_literal`); residual = float-epsilon straddle at diagonal corners, the T4-M1 close's uncraftable JVM-luck class. Mutant log: `mut_T3-Q4_first_time_break_t6ledger_35green.log` (heals, 35/35). |
| Events pins (`route_events.rs`) | RE-TRIAGED GOLDEN-ANCHORED (18/18 green) | `rust_stream_aligns_through_triaged_divergences`: per-fixture whole-stream `assert_eq!` (all four kinds, all three fixtures) + former-ordinal witnesses (t7 assign#73 carries golden `TargetItemExpansionDoor/item=6` at `62570.28048522717`; t9 route#0 carries golden `netItems=2->9`/`maxItemId=130`); the `EventTriage` tuple machinery is DELETED. `route_rows_share_the_field_skeleton_and_diverge_only_in_id_values`: per-fixture route-row `assert_eq!` + golden-literal witnesses (t7 row 3: `2->12`/`107`; t9 row 0: `2->9`/`130`); skeleton/strip/count/boundary faces retained (now trivially full-length). Rust-measured literals (821846.30, 2->7, 126, 93) purged per the dispatch convergence rule. |
| Ripple | ses ROTATED, manifest/scores UNCHANGED | ses `8bd773fca5767948…` → `253e7b1401775172…` (the landed tail face moves pad-exit geometry on bm08), manifest `16714d5ee3dff007…` UNCHANGED; router-only compare 11 green (fanout-off profile vs committed router-only records; ecc83-pp_v2 0/16/991.32 exact; reviewer-reproduced 11/0 in 336.1s; the full-flow battery CHEAP was NOT run in T6 — T12's face; spec-review MINOR-1 disclosure); the buglog-176 determinism fingerprint ROTATED with the shover wiring — 14/0/19 → 25/0/22 traces/vias/bends, score face unchanged, re-measured deterministic, re-pinned at `router_compare.rs` (`detail_pass_deadline_silent…`); seven compares green (`cmp_*_t6.log`, incl. `cmp_ses_t6.log` 20/20 + `cmp_ses_snap_t6.log` 5/5 landed in the T6 fix round); `cargo test --workspace` 1349 passed / 0 failed / 15 ignored. |

Undisclosed-mechanism disclosure (added by the T6 fix round, spec review
MINOR-5): the events close rode one more engine change than the original
T6 prose named — ExpansionDoor REFERENCE IDENTITY
(`rust/crates/epic-router/src/expansion/door.rs`, module doc :14,
`DOOR_TAG_COUNTER: AtomicU64` :60): every `ExpansionDoor` construction
draws a process-unique `tag: u64` included in its `PartialEq` (clones of
one construction carry it — they are the copies of one Java object;
distinct constructions differ), and the seven consumer files
(`expansion/neighbours.rs`, `expansion/neighbours_forty_five.rs`,
`maze/list_element.rs`, `maze/completion.rs`, `maze/search_engine.rs`,
`maze/ripup.rs`, `expansion/pins.rs`) lean on that identity for door
removal/endpoint resolution. The motivation is Java parity, not
convenience: Java doors are heap objects and `List.remove`/`equals` fall
back to reference identity, while the value-only Rust equality broke
under room-id collisions of the incomplete-room scheme
`31 * shape.getId() + layer` — two live same-shape same-layer rooms
share an id, their doors share the whole value, and a value-based
removal could pick the wrong twin (the observed phantom completion).
Tags never enter any output row, ordering, or hash-key ordering — they
feed only equality/membership tests and the maze state-slot key fold,
so run-to-run determinism is unaffected.

## T7 fanout stage + the bm06 pass-2 adjudication (appended by M4-T7 fix round, spec review round 1)

The fanout stage's full dossier (banks, ordering model, front-gate
mechanics, deadline/throttler seams) lives in the module doc
(`pipeline/fanout.rs`) and `logs/M4-T7/report-t7.md`; this section
carries what T12's close-out will look for.

**The bm06 pass-2+ retry attribution is EARNED-deep — recorded, not
chased (exit condition = the M4 lever).** Fanout full-flow vs jar
(`logs/M4-T7/evidence/jar_evidence_bm06_t7.log`, re-derived post-fix in
`jar_evidence_bm06_t7fix.log`): stage-start 115 of 126 / 9 already
connected / 2 netless EXACT; pass #1 101/5/0/+28 (ripup 100) EXACT;
stage-completed 124 total, escaped 113/124 (91.1%) EXACT; final
notRouted=2 both sides; pass-2 entry candidates EQUAL (9 = 115−101−5 =
6+3 jar = 7+2 rust) — both sides re-walk the SAME ctor-sorted static
rows with no re-enumeration, and the ripup/budget ramps appear verbatim
(200/300). The split (rust 7/2/+3 @200 then 0/2/+0 @300 in 3 passes vs
jar 6/3/+1, 2/3/+1, 1/2/+0, 0/2/+0 in 5; cumulative retry-pass vias +31
vs +30) is therefore decided by maze tie-breaks over boards that
already differ GEOMETRICALLY at pass-2 entry — pass-1 counter-exactness
is not geometry-exactness (the cross-language board hashes are
incomparable) — i.e. the bug-181 victim-choice family observed one
stage earlier. NOT a T7 pass-loop gap. Full-flow end faces: rust 5/98
incompletes, 8 violations (0 router-introduced), 930.81 vs jar 2/98, 8,
971.63. bm06 full-flow was never at Rust parity pre-T7 (no fanout stage
existed), so this is a new-stage delta, not a regression.

**Spec-review round-1 fix faces (this round):**

| Item | Disposition |
|---|---|
| REQ-2 — empty-unconnected arm state | `fanout_pin` now answers `NoUnconnectedNets` carrying Java's reused "already connected" MESSAGE literal (`RoutingBoard.java:999-1001`) — the consumer renders `pin_no_unconnected_nets` and bumps NO counter, exactly Java's `BatchFanout` switch. INFO-level faces are UNCHANGED on both evidence boards (fresh rust rows byte-identical to pre-fix, per-stage board hashes included — no board-state change; both arms return before any engine call). The correction is trace-row-level only: the pre-fix rust pass_end trace rows inflated `alreadyConnected=` relative to Java; post-fix they align. Pinned by `fanout_board_starved_one_pass_rows` (1 already-connected + 3 no-unconnected-nets, was 4 + 0); the re-flip mutant dies on exactly that pin. Evidence: `jar_evidence_bm08_t7fix.log` (all-EXACT, unchanged), `jar_evidence_bm06_t7fix.log` (pass-1/stage/end EXACT, pass-2+ divergence UNMOVED). |
| REQ-1 — strict-DRC revert call site pinned | `fanout_pass`'s `enforce_strict_drc` revert (Java `BatchFanout.java:287-304`) is now driven through the PRODUCTION stage loop by `fanout_board_reverts_violating_escape`: a pincer world yielding all three outcome arms in one pass — CMP1-P1 maze-fail (pad-crossed start room, no revert row), CMP1-P2 clean escape SURVIVES (the crossing cell; a lone first-drill via), CMP3-P1 routed-then-reverted (`fanout_via_reverted` row with the 3-new-item count, FAILED answer, notRouted attribution, watermark rip with on-board counts unrotated). Revert-block deletion mutant: KILLED. |
| MINOR-1 — production oscillation constant | `STAGNATION_PASS_LIMIT` (Java `BatchFanout.java:105`) is now cited and wired into `stagnation_limit_production_constant_is_java_face`; reviewer mutant T7-S1 (3→2) dies (previously survived — the boundary pin fed its own literal). |

Dossier references: buglog-181 (`fix` field, T7 FANOUT CHECKPOINT),
`logs/M4-T7/report-t7.md` §5.1, evidence logs above.
## T8 optimizer score V2_LOWER_BOUND + lower bounds (appended by M4-T8 spec-review fix round)

The optimizer-score dossier lives in the module docs
(`pipeline/board_statistics.rs` for the score/`ensure_difficulty` port,
`pipeline/board_statistics_bounds.rs` for the MST/via-cover bounds
calculator and the two-faces pin contract) and
`logs/M4-T8/report-t8.md`. What T9+ must carry forward:

**FORWARD CARRIAGE — T9 ANCHORING RULE.** Jar-vs-Rust ROUTED GEOMETRY
drifts 0.04% on the fanout-ON bm08 full-flow profile: jar 94.96 mm of
trace vs Rust ~95.00 mm at IDENTICAL counts (1 via, 21 bends, D 80) and
identical parse faces (bounds raw-bits equal) — the composed V2
optimizer score therefore reads 823.79 (jar) vs 823.37 (Rust), ~0.4 pp
of the router-compare 2 pp relative budget. Pre-T8 (the ses bytes
differ: jar `03e3260a…` vs Rust `ec22a0f2…`, T7's recorded
self-determinism face); diagnosed and adjudicated in the T8 review
(SPEC_COMPLIANT_WITH_MINORS, F1 closed in the implementer's favor — the
jar's optimizer restore is byte-exact, so the delta is purely
routing-side). **T9's composed-score pins vs the jar must use
component-exact + composed-relative anchoring, NEVER composed-exact**;
bounds-side pins stay exact (bounds are bit-exact and
routing-independent). Evidence:
`logs/M4-T8/evidence/jar-bounds/` (bm08-manifest.json = the 823.79
full-flow face; bm08-noopt-manifest.json = the component faces, no
optimizer_score; the two jar ses files byte-identical at `03e3260a…`),
`logs/M4-T8/report-t8.md` §F1.

**Walk-order fact (T8 recon, now doc-of-record).** Java's item walks
(`UndoableObjects.startReadObject`) yield items in DESCENDING-id order
(`ConcurrentSkipListMap` keyed by `Item.compareTo = other.id -
this.id`). Order-sensitive consumers (Prim in the bounds calculator)
must walk `iter_descending` — pinned by
`t8_prim_walk_order_bend_count_discriminated` (a 3-terminal tie world
where ascending yields a 1-bend MST and descending the jar's 2-bend MST
at equal 140-unit length; the T8-S1 ascending mutant dies there). The
counting walks are order-independent counts and stay ascending.

## T9 BatchOptimizer — rip-and-reroute optimizer stage (appended by M4-T9)

The dossier lives in the module docs (`pipeline/optimizer.rs`); report in
`logs/M4-T9/report-t9.md`, evidence in `logs/M4-T9/evidence/`. Baseline
`90590891c`, census 1388 → **1410**/0/15. What T10+ must carry forward:

**The one-winner mapping (threaded → sequential-deterministic).** Java
races candidates on a `maxThreads` pool, then applies ONE board:
`winningCandidate == null || res.result.improvedOver(winning)`
(`BatchOptimizer.java:750`) — the compareTo-minimum outcome with the
earliest-submitted candidate winning ties, then `this.board =
winningCandidate.board` (`:788`). The port evaluates candidates in
`ReadSortedRouteItems` order on per-candidate clones and applies the same
rule; in a one-thread world that IS Java's own sequential semantics (the
deterministic analog of the race, not a byte-port of it). Pinned by
`t9_item_route_result_compare_and_item_id_tie` (rule + tie faces) and the
E2E faces `t9_e2e_first_candidate_applied` (max_items=1 isolates the
first candidate: adopted-board hash `f3fcdc3c…` in the completed row,
then the pass gate rejects and the incumbent is restored) plus
`t9_e2e_reject_restore_threshold_stop` (final hash == baseline
`981c19c5…` after both passes adopt-then-restore).

**The improved-ladder quirk (doc-of-record, do NOT "fix").**
`ItemRouteResult` compares `traceLengthBefore` = the WHOLE BOARD's
`totalWeightedLength` (`:633`) against `traceLengthAfter` = the plain
`traces.totalLength` — on an all-unfixed board the weighted face
dominates, so ANY candidate whose reroute keeps incompletes and via
counts reads "improved" even at identical geometry. The discipline is
the PASS-level gate (strict optimizer-score improvement) + the loop-level
restore, not the candidate ladder. The quirk is why the e2e world's
passes adopt-then-restore every pass (both pinned verbatim, incl.
`951.88 -> 942.92 (REGRESSED, -0.9414%)` for the even-pass reroute).

**ReadSortedRouteItems dedups duplicate keys.** `next()` returns the
single minimum key strictly past the cursor and then EXCLUDES that key:
two items at the same (x, y, layer) collapse to the first encountered in
the DESCENDING-id walk (higher id), the other never enumerates — pinned
by `t9_read_sorted_route_items_order` (which also pins the lower-layer
trace stealing a via's key, the shove-fixed/user-fixed/via-touching skip
arms, and strict key ascent). Mutant T9-M2 (order inversion) kills 6 pins.

**The optimizer counters' phase is a port rendering.** Java's optimizer
`RouterCounters` leaves `phase` null; the port sets `phase = "optimizer"`
(Rust field is `String`, not nullable). The CLI manifest backfill filter
(`phase == "autoroute"`) must keep excluding it — pinned by
`t9_optimizer_counters_do_not_backfill_autorouter_passes`.

**Stage gate coupling (M4-T9 hardening of the harness; MIN-2 correction).**
`RoutingPipeline.runOptimizationStage` gates on `getRunOptimizer()`
(default TRUE from DefaultSettings.java:178) AND
`!thread.isStopRequested()`. That full stop in Java is raised only by the
max-items path (`AutoroutePassRunner.java:218`) and EXTERNAL stops; the
router-side stops (max-passes/stagnation/restore-exhaustion) raise
AUTO_ROUTER_ONLY (`AutorouteBatchLoop.java:311/349/356/512/543`), which
does NOT satisfy the gate — Java lets the stage RUN on router-side stops
and preflight guard 1 (incompletes > 0) does the skipping. The port's
single shared flag collapses the two faces: same net outcome (no
optimization on partially-routed boards), with one corner divergence (a
fully-routed-but-stopped board runs no-op reroute passes in Java and
skips the stage in the port).
The committed router-only Java records are the both-stages-OFF face, so
`route_argv` now carries `--optimizer.enabled=false` next to the fanout
flag (the banner phrase derives both; `run_cli_argv_carries_the_
comparability_flag` pins the pair). The DISABLED face reproduces the
pre-T9 flow byte-exactly: bm08 full flow with the flag returns T7's
recorded ses `ec22a0f2…` + manifest `16714d5e…` exactly
(`determinism_fullflow_t9.log`). T9's new recorded full-flow face
(optimizer ON): ses `ea68d513…`, manifest `a2178650…` (run ×2
byte-equal; bm08's stage ran and improved 823.37 → 823.50, +0.0153% <
2.5% threshold → stop).

**`normalizeAlgorithm` moved.** Java warns in `BatchOptimizer.create`;
the port warns as the stage's first row (construction has no sink) —
`freerouting-optimizer` is `BatchOptimizerStage::ALGORITHM_ID`.

**The ripup-cost round-vs-trunc domain (MIN-4, fix round).**
`trace_ripup_cost_factor`'s `Math.round` is MUTANT-DISCRIMINABLE only in the
increased-face-DROPPED arm: the base is `start_ripup_costs` — an INTEGER through
every settings path (CLI/DSN parse i32, DefaultSettings 100) — and while the
increased face holds the base is 10·start (even), so f32(0.6)·(10·start) lands
within ~1.2e-5·start of 6·start (f32(0.6)·500 = 300.0000119…) — a fraction
≪ 0.5, hence round == trunc == 6·start; for start ≡ 1 or 3 (mod 5) the dropped arm's real
fraction is 0.6/0.8, f32(0.6)'s ~2.4e-8·base error keeps it above 0.5, and round
≠ trunc (start=51 → 31 vs trunc 30; start=3 → 2 vs 1). Pinned in
`t9_candidate_ripup_costs_ladder`; mutant T9-S3 (round→trunc) now dies there
(fix-round evidence `mut_T9-S3_round_trunc.log`). The T8-S4 equivalent-mutant
precedent applies to the INCREASED arm: on it the mutant is equivalent, and that
is a domain fact, not a pin gap.

**Banked (unreachable headless, documented in optimizer.rs):** the
thread pool (`maxThreads` parsed, unused), `resultMap` + the PRIORITIZED
arm (transient GUI-only strategy; headless always SEQUENTIAL),
`currentPosition`/`ProgressThrottler` GUI faces, per-stage CPU/alloc/heap
metrics (rendered as the fanout `0.00/0.00/0.0` literals), the dead
`if (stillUnroutedItems && … && updatedRoutingBoard == null) {}` empty
block, the stage wall deadline (wall-profile-only — armed only behind
`--deterministic-budgets=off`, pinned both faces).

**World-limited pin gap (documented, not silent):** guard 4's
through-hole-pin arm (`onlySmdPinsAndTraces=false` via a multi-layer pin
in the connected set) is unobservable on t9_locator45 — all 100 pins are
single-layer SMD on layer 0. Pinned arms: crossesLayers=false (routed
world), netCount==0, twin-via-in-set, empty-set-false, and the
all-user-fixed vacuous-TRUE quirk (`:231-233`).

**T10 owes:** the manifest optimizer faces (`optimizer_score` iff-gate,
`phases.optimizer` snapshots), `job.stage` IDLE transitions, fanout-only
mode (`maxPasses=0` temporary override), stage-order pins. The stage
itself, its settings family, and the CLI gate are T9-done.

## T10 full-pipeline assembly — RoutingPipeline parity (appended by M4-T10)

`pipeline/full.rs` is the port of `RoutingPipeline.java`; the CLI flow
(`epic-cli/src/route.rs` step 4) now calls `full::run` — ONE pipeline shape,
the T9 inline wiring moved, not forked. Report: `logs/M4-T10/report-t10.md`,
evidence in `logs/M4-T10/evidence/`. Baseline `b32ad60fa`, census 1410 →
**1418**/0/15.

**The fanout-only mode's real semantics (doc-of-record — the "mode" is not
what its name suggests).** Java's branch (`RoutingPipeline.java:104-113`)
fires when `routerEnabled == false` — i.e. `runRouter == false` OR
`maxPasses < 0` — and temporarily sets `maxPasses = 0`, which per
`RouterSettings.java:939` means NO LIMIT (the batch loop's pass-count gate
reads `maxPasses > 0`, `AutorouteBatchLoop.java:308-311`). The override is
load-bearing exactly on the `maxPasses < 0` face: the loop recomputes
`isRouterEnabled` from the live settings (`:249-253`) and seeds
`continueAutorouting = isRouterEnabled` (`:274`) — with `runRouter == false`
the loop never enters a pass (true fanout-only, the `--router.enabled=off`
face); with `runRouter == true, maxPasses < 0` the override flips the
recomputation TRUE and the router RUNS with no pass-count limit. Ported
verbatim; both arms pinned (`t10_fanout_only_mode_restores_setting_and_skips_
the_router`, `t10_fanout_only_override_load_bearing_on_negative_max_passes`;
the `-1` face is settings-reachable only — CLI `validate()` normalizes
negatives to 0 first, the T13 F1 pin).

**finishAutoroute — recon recorded, no port face.** `RoutingBoard.java:
899-904` + `AutorouteEngine.java:307-316`: clears the retained
expansion-room database and `clearAllItemTemporaryAutorouteData()` —
memory-cache cleanup only; nothing after the pipeline reads the engine, and
the port holds no retained engine between stages.

**Stage lifecycle events — recon recorded, no new emission.** Java's
per-stage STARTED/RUNNING/FINISHED task-state events are emitted by the
stages themselves (`BatchAutorouter`, `BatchOptimizer`) — already mirrored
by the T12 driver and the T9 optimizer through the sink. `job.stage`
(ROUTING/OPTIMIZATION/IDLE, `:99/:126/:89`) is a public FIELD write — no
event, and the headless scheduler registers only a board-updated listener +
an afterRouting StageListener (telemetry-only, banked). The port records the
transitions in `PipelineOutcome::stage_transitions` so the stage-order pin
has a witness; events compare stayed 3520 rows aligned.

**The T13 phase-blind bank — RESOLVED (and a new sink fix).** The T13 bank
("M4 must filter the `passes_completed` backfill by phase") was resolved at
T7 (the `phase == "autoroute"` filter); T10 extends the attribution: the
OPTIMIZER row's pass count comes from the stage OUTCOME
(`OptimizerOutcome.passes_completed` — Java `BatchOptimizer.java:523`,
explicit bypass `0` at `:309-313`), never from counters. The T13-bank concern
then surfaced for real: the CLI sink kept ONE last-counters slot, so once the
optimizer emitted counters LAST, the autorouter backfill filter discarded the
legitimate autoroute counters and `phases.autorouter.passes_completed` went
absent (caught by the flow pin before landing). `CliDriverSink` now keeps
last counters PER PHASE (`last_counters_by_phase`). Java's own manifest
backfill (`RoutingResultManifest.java:206-208`, `job.getCurrentPass()`) stays
phase-BLIND by contrast — it would attribute an optimizer pass to
`phases.autorouter` in a router-disabled world; the port deliberately filters
(a divergence-from-Java recorded here; fanout never sets `currentPass`, so
the fanout leak Java cannot have is also pinned dead).

**Manifest per-phase fill (the T10 recorded rotation; fix-round corrected).**
Snapshot rows are Java's deterministic subset: `board_statistics` (connections,
clearance_violations with the load-seed subtraction, `vias.total_count`,
`traces.total_length_mm`), the score rows, the phase's own `score` flavor and
`score_source` (`current` / fanout's `not_applicable`). TWO content corrections
from the spec review: (F1) the FANOUT rows carry NO score rows at all —
Java's fanout fill (`AutorouteBatchLoop.java:234-239`) builds a bare
`PhaseSnapshot` with only boardStatistics + scoreSource; (F2) every manifest
float renders through `GsonProvider`'s TwoDecimal adapters
(`GsonProvider.java:39-47/57-65`) — `%.2f` via BigDecimal, HALF_UP, trailing
zeros — NOT `Float.toString` (the first-cut rationale was wrong); the port
computes HALF_UP arithmetically (`java_two_decimal`; f32-eighth ties make
`format!`'s half-to-EVEN wrong) and writes the literal text via `JavaDecimal`
(serde_json `raw_value`) — `925.00`, `0.00`, `73.87`, text- and value-exact
vs the jar; `-0.0` renders `0.00`. The T13-era top-level `normalized_score`
(raw widening) now rides the same adapter face. Captures sit at
Java's exact moments: fanout boundary opens with the fanout gate BEFORE the
SMD-skip arm (`:93-98`) and closes after the stage bodies (`:231-247`) —
captured in the driver (`StageBoundaries`); autorouter before/after inside
the router-enabled gate and after the tail sweep; optimizer at stage
entry/exit (a guard bypass leaves before == after, matching Java's bypass
fill, and emits `passes_completed: 0` + the top-level `optimizer_score` —
the jar's `bm08-noroute-manifest.json` face, pinned by
`t10_fanout_only_flow_face_matches_the_jar_noroute_face`). Top-level
`optimizer_score` iff-gate = `RoutingResultManifest.java:200-202`.

**Digest rotation record (all sanctioned, determinism ×2 byte-equal
everywhere).** Ses faces ALL unrotated across both rounds — full-flow
`ea68d5133f555700…`, disabled `ec22a0f2…`, canaries `253e7b14…`. Manifest
rotations: round 1 (the per-phase fill landing): full-flow
`a2178650f8ba6290…` → `c08e791ad44d2e80…`; disabled `16714d5ee3dff007…` →
`c179abcd827bdef7…` (fanout ran → its rows land; optimizer stays `{}`); canary
`16714d5ee3dff007…` → `183087754c1cec82…` (autorouter ran → its rows land).
Round 2 (the fix round's F1+F2 content corrections): full-flow →
`d0b5f0dc33420e0184f6587a620d0480c4b43eb910f3087c2f58b50305295b36`; disabled
→ `d2a7102c971cd99a166c78d6cdc807df687feb7cb3e45cafb57b90410aac9e00`; canary
→ `8c4a97738375e67d7c937c6518b7bc27e29236f77b7062da3d46447b465cc1db`. No
disabled/skipped stage ever emits rows (the run-based part held).

**Fix round (spec-review-t10-1, SPEC_COMPLIANT_WITH_MINORS).** F1: fanout
rows carry NO score rows (oracle: the T10 probe
`logs/M4-T10/evidence/jar-fanout-ran-probe.log` — jar fanout-ran world on
bm08, `fanout.before` keys = board_statistics + score_source only). F2: the
%.2f adapter face, value + text (above). F3: the optimizer gate's INPUT face
is pinned (`t10_optimizer_gate_reads_the_post_routing_stop_face` — the
reviewer's M7 pre-run-face mutant now dies). F4: `:276`→`:274`,
`:255`→`:261`, `:494-519`→`:603-626` cite corrections — the initial sweep
fixed the capture-SITE comments but MISSED the [`StageBoundaries`] type doc
(batch.rs), which carried the same two stale cites; (caught by the quality
round, completed in the quality fix round). F5: the
`_reroute_passes` reservation comment corrected (mis-scoped in T9; the face
is fed from `OptimizerOutcome.passes_completed`), and the fanout-only flow
pin's doc reclassified — only its optimizer-bypass rows are oracle-anchored;
the fanout-row-POPULATED face is a port-side self-face (the Java CLI couples
router-off to fanout-off; the port does not — settings-layer divergence
bank). Census 1418 → 1419/0/15.

**Quality round (quality-review-t10-1, APPROVED_WITH_NITS).** MINOR-1: the
F4 cite sweep completed — the [`StageBoundaries`] TYPE doc carried the same
two stale cites the capture-site comments had already shed (`:255`→`:261`,
`:494-519`→`:603-626`, spans re-verified against the frozen tree: gate
`:603`, after-capture `:604`, fill `:603-626`); the SEAM F4 line above is
reconciled to say the sweep half-landed, not silently. MINOR-2: the
non-finite render boundary is TOTAL — `JavaDecimal::of` returns `None` on
`!is_finite`, which feeds the existing skip-serialize face (exactly Java's
`out.nullValue()` omitted-field face, `GsonProvider.java:39`/`:59`);
pinned by `t10_non_finite_render_boundary_is_java_null_value_face`
(NAN/±INF → None; the `-0.0` → positive `0.00` face; the f32-eighth
HALF_UP tie `2.125` → `2.13` where Rust's half-to-EVEN would print
`2.12`). MINOR-3: the optimizer `after`-capture cite `:508-510` →
`:519-526` (the actual `finalStats` walk is `:519`). NIT-1:
`GsonProvider.java:44` → `:42`/`:62`. NIT-2: the two T10 flow pins now
build argv through `route_argv` + a `temp_out` helper (the T9 wiring pin's
inline block left as-is — pre-existing committed pin). NIT-3: the six
`phase_snapshot` calls collapse through a local closure. NIT-4: the T8
pin's doc refreshed (its world is deliberately optimizer-OUT via direct
BatchDriver drive; the jar anchor still holds). ZERO bytes moved: all
three SES faces unrotated AND all three manifests byte-equal to
549f69eb1's digests (`d0b5f0dc…`/`d2a7102c…`/`8c4a9773…`). Census 1419 →
1420/0/15.

**Banked:** the optimizer stage's stop face is the T9 fresh face (Java shares
`job.thread`; the CLI flow has no external stop source yet — when one is
wired, hand the stages the shared flag). The headless scheduler's
afterRouting StageListener (session-summary telemetry) and the whole-run
duration backfill (`:210-216`) are non-determinism-family omissions.

## T11 full-pipeline baselines + battery face flip (appended by M4-T11)

`router compare` runs under ONE of two profiles
(`CompareProfile`, `harness/src/router_compare.rs`): `router-only` (THE
DEFAULT — the pre-T11 face byte-for-byte, so CI's bare
`router compare --report-only` stays the M2/M3 regression face until the
T13 exit flip) and `full` (the M4 battery face: the Rust side drops BOTH
comparability flags and runs the assembled fanout → router → optimizer
pipeline; the gates read the M0 full-flow jar records). Any other
`--profile` value is a LOUD failure naming both accepted names — the
capture/verify spelling `full-flow` is deliberately NOT a compare name.
The two profiles write scratch under separate run roots
(`runs/router-compare` vs `runs/router-compare-full`). The banner now
names the face (`router compare[router-only]` / `[full]`) with the
comparability phrase DERIVED from the built argv in both arms (a flag
dropped from the router-only argv says MISSING; a disable flag in a full
argv says PRESENT — the flag-gating bug face). The router-only detail
pass keeps its EXACT historical argv face (fanout flag ONLY — the
pinned buglog-176 fingerprint face); the full detail pass mirrors its
profile (no flags). `router determinism` stays router-only by design
(the canary faces must not rotate).

**The M0 baselines are REUSED verbatim (carry-forward 2 — the verify-FIRST
question).** The M0 `baselines/java/` captures were the DEFAULT-profile
(`full-flow`: no disable flags, fanout+optimizer on) jar runs at the
frozen tree `e7f9bdf1` with tiers.yaml timeouts. The verifiable reuse
facts: all 11 Tier A records carry the `optimizer_seconds` and
`optimizer_score` FIELDS; exactly four carry NONZERO `optimizer_seconds`
(bm01 912.7s, bm08 4.31s, bm02 9.02s, sonde 6.15s) and the rest 0.0 —
consistent with the manifest's iff-gate emitting `optimizer_score` only
when the optimizer phase carries before/after snapshots
(`RoutingResultManifest.java:200-203`, the harness's own PROVE-gate doc,
`oracle.rs`); bm01's 912.7s-with-score-0.0 combination is left
UNCHARACTERIZED (an unverified engine-behavior story does not belong in
the reuse record — the gates never read these fields). The profile
marker is ABSENT (= the full marker) and every `fixture_sha256` was
re-verified against the current DSNs at T11 (11/11 match). NO new
capture happened; no committed artifact was modified; the M3 router-only
one-off captures were never re-run. Reuse, not recapture.

**The M4 full-face battery (release, gate mode, external 7200s bound;
2182.9s actual; exit 1 with 3 reds — the honest inventory, nothing
tuned).** The M3 exit snapshot beside which these rows land: the router-only
face read 8/3 (bm06 tuning, bm11 completion-by-1-at-truth, bm01
wall-limited — T17c D-table). The FULL face reads **8 PASS / 3 RED**:

| fixture | Rust full (inc/viol/score, passes, wall) | Java full (M0) | verdict + triage |
|---|---|---|---|
| bm01 | 1/0/993.16, 20 passes, 663.0s | 0/0/1000.0 (17 passes, optimizer 912.7s) | **RED — completion by 1** (score GREEN ≥ 980, violations GREEN). The fanout stage transformed the face vs M3 (wall-limited 33-unrouted kill → a stagnation plateau): passes 12-20 flat at 993.16/1 (ten no-gain passes → the stop fired), residual = one connection of the U53→U8 serial family (CTS/DSR/DCD/RI chain, the stop report). Not the wall (663s ≪ 1800s tier). Triage: the bug-181 victim-choice/attempt-order family at the router's plateau + the T10-banked stop face (Java's optimizer stage RAN on its 0-incomplete board; the port's preflight guard skips it at incompletes>0 — the T10 mirror). T12's lever. |
| bm02 | 0/0/1000.00, 1 pass, 4.0s | 0/0/1000.0 (2 passes) | PASS — exact equal on all three gates; the autorouter needed ONE pass (fanout pre-routing did the heavy lifting; jar needed 2). |
| bm06 | 5/8/930.81, 18 passes, 80.4s | 2/8/971.63 (18 passes) | **RED — incompletes 5 > 2 AND score 930.81 < 952.20 (J−ε)**; violations GREEN 8 ≤ 8. The EXACT T7 probe face (5/8/930.81, `logs/M4-T7/evidence/jar_evidence_bm06_t7fix.log`) — unchanged by T8-T10, so the red is the PRE-EXISTING bug-181 victim-choice family one stage earlier (fanout pass-2+ retry divergence: rust 7/2/+3@200 then 0/2/+0@300 in 3 passes vs jar 6/3/+1… in 5), not a new-stage delta. T12's lever per the T7 checkpoint. |
| bm07 | 0/0/1000.00, 1 pass, 54.0s | 2/0/968.99 (18 passes) | PASS **BETTER than Java** — 0 incompletes vs 2, score 1000.00 vs 968.99. |
| bm08 | 0/0/1000.00, 1 pass, 0.6s | 0/0/1000.0 | PASS — exact equal. |
| bm09 | 1/0/988.51, 18 passes, 20.0s | 1/0/988.51 | PASS — exact equal to the second decimal. |
| bm11 | wall-killed at 600.2s (integrity red: no manifest) | 3/0/975.0 | **RED — integrity by wall**, and the plateau face AT the kill was **975.00 / 3 / 0 — JAVA'S EXACT TERMINAL FACE** (passes 9-10, 62-64s each). Not a quality red: the engine converged to parity and the stagnation stop simply had not fired yet (needs ten flat passes; 9-10 was the first flat pair). Same wall family as M3's bm11 bank, but at parity. T12 lever: stop-face timing; a natural end = Java parity. |
| ecc83-pp | 0/0/1000.00, 1 pass, 0.2s | 0/0/1000.0 | PASS — exact equal. |
| ecc83-pp_v2 | 0/16/991.32, 1 pass, 0.2s | 0/16/991.32 | PASS — exact equal (the buglog-176 fingerprint face holds on the full pipeline too). |
| pic_programmer | 0/1/999.77, 1 pass, 1.8s | 0/1/999.77 | PASS — exact equal. |
| sonde xilinx | 0/0/1000.00, 1 pass, 4.0s | 0/0/1000.0 | PASS — exact equal. |

**The F1/F1′ re-anchor (T8/T9 owed, closed).** The full-flow bm08 faces
reproduce BYTE-EXACTLY through the new full profile — three independent
runs (the battery's own + two dedicated `--work-root` witness runs): ses
`ea68d5133f5557004bb2778b441777dde742f87a058ba481293d8fc5df08b110`,
manifest `d0b5f0dc33420e0184f6587a620d0480c4b43eb910f3087c2f58b50305295b36`
(`bm08_full_digests_t11.log`). The router-only faces are unrotated: the
determinism canary reproduced ses `253e7b1401775172…` + manifest
`8c4a9773…` (×2 byte-equal), and the disabled-control ses `ec22a0f2…`
was not re-run (the disabled face was T9's control; the T11 gates pin the
canary + the seven compares + events, all green — and was subsequently
reproduced byte-exact by the spec reviewer: ses `ec22a0f2…` + manifest
`d2a7102c…`). All seven compares + events green at the pinned faces
(corpus 5000/5000, dsn `--set all` 1332/0, ses 20, ses-snap 5, index 33,
undo 33, drc 17/17, events 3520 — evidence under
`logs/M4-T11/evidence/` — the seven-compare family in `compare_*_t11*.log`, with determinism/e2e/census/witness logs alongside).

**Stale-pin re-face (honest, witnessed).** The pre-existing e2e
`e2e_router_compare_gate_mode_exits_nonzero_on_a_red_battery` expected
bm01 as the M3-era fast panic-class red — but the T17a integrity close
ended bm01's panics and the T7 fanout stage turned the row green (the
bare default-profile run exited 0, live-witnessed at T11; the pin died
in the T11 e2e set). Re-faced to a CONSTRUCTED red: `EPIC_CLI` → a
freshly-written no-op stub (mtime passes the freshness guard), no
manifest → integrity red → gate mode exits nonzero naming the tally.
~0.7s, profile-independent, no committed artifact touched; the R1
bypass mutant (battery_exit call removed) still dies. The real reds of
the current battery cost 80s-1800s of wall — wrong shape for a smoke;
the battery log is their witness.

**Pins (crossing cells: profile × flags × record-marker, every row and
column).** `compare_profile_parsing_is_exact_and_loud_on_unknown_names`
(omitted/router-only/full parse; every other spelling LOUD, error names
both values); `run_cli_argv_carries_the_comparability_flag` EXTENDED to
the 2×2 (router-only argv = base + both flags exactly once as suffix;
full argv = the SAME base with NO flags; banner mismatch faces BOTH
directions — MISSING and PRESENT; detail-argv faces pinned); 
`full_flow_record_pre_flight_accepts_only_the_markerless_m0_face` (the
M0 face accepted, a router-only-stamped record rejected with the
router-only loader ACCEPTING the same body as the contrast arm, marker
typo rejected, missing file loud);
`tier_a_fixture_enum_is_complete_against_tiers_and_records` EXTENDED to
both profiles (11 records present under BOTH record sets, no extras);
two e2e smokes (`e2e_router_compare_full_profile_passes_bm08_in_gate_
mode` — banner face + derived no-flags claim + [PASS] row through the
BUILT binary; `e2e_router_compare_wrong_profile_fails_loudly` — the
`full-flow` spelling exits nonzero naming both values) + the re-faced
red-battery e2e above. Every new pin is world-built (temp-dir records),
except the two enum pins which read the real repo tree.

**Gates.** fmt 0; clippy `--workspace --all-targets -- -D warnings` 0
(incl. unwrap_used); bare `cargo test --workspace` exit 0 — census
**1422/0/17** (1420/0/15 → +2 unit pins, +2 `#[ignore]` e2e smokes);
seven compares + events exit 0 at the pinned faces; `router determinism`
exit 0 with the canary ses `253e7b14…`/manifest `8c4a9773…` byte-equal
×2; full-profile bm08 ×2 (+battery = ×3) reproducing `ea68d513…`/
`d0b5f0dc…`; the full battery release/gate-mode exit 1 with the 8/3
inventory above (external 7200s bound, actual 2182.9s). Docs: the rust
README command block gained the `--profile full` face line.

**Banked for T12+:** (1) bm06/bm01 reds = the bug-181 family with the
T7 checkpoint's lever list (victim-choice tie-breaks, rip-free
exploration order); (2) bm11's stop-face timing — the engine reached
Java's face and the stagnation stop lagged; (3) the M0 `baselines/java`
B/C-tier records are likewise reusable for any future full-face battery
extension (fixture hashes NOT re-verified for B/C — only A was needed);
(4) the banner-text face (`router compare[router-only]`) is an
intentional stdout change in the default invocation's TEXT, not its
behavior (flags/records/exits identical; no pin keys on the banner
prefix).

## T12 bm06 fork re-derivation + the completion-face residual (appended by M4-T12)

Task 12's alignment work did NOT land a code change; the honest outcome
is a NEW divergence family classified upstream of the buglog-181
tie-break family and a residual adjudicated not-alignable this task.
The Java wins every conflict below. Full dossier: buglog 189
(`.wolf/buglog.json` — id note: the stale-bin family IS landed as string
`bug-184` and the string block runs `bug-184`…`bug-188` (numeric 185
would have collided with `bug-185`, the M4-T5 DSN `;`-comment lexer
bug), so the T12 entry takes numeric **189**); raw instruments under
`logs/M4-T12/scratch/` (git-ignored).

**The first full-flow fork is NOT the tie-break family.** The
attempt-stream diff re-derived on the full-flow face (fanout-only
capture, both engines traced with `--debug.enable_detailed_logging=true`
vs a scratch capture sink through the production `BatchDriver`
construction): assign-stream rows 1-6 align byte-for-byte (the west
door's six section rows) of the FIRST fanout search (pin U9-1, net 21);
the fork is stream row 7 — the EAST door, the second door-group: the
start room's east door differs because the START ROOM's completed
north edge is **y=-911636 (jar CLI) vs y=-911136 (Rust)** — a 500-DBU
completion-geometry divergence at the 45°
`ShapeSearchTree45Degree.completeShape`/`restrainShape` face
(`ROOM_EDGE_REMOVE applied` newBounds row vs the port's identical-input
trace). Downstream of that row the streams cannot be aligned at all, so
the M3-era fork worlds (attempt 42; via-841-vs-trace-903; net-1-335-vs-
net-5-1199) are UNREACHABLE on this face — the tie-break alignment had
no landable pin world this task.

**Why it is a residual and not a port fix (the disconfirmation
dossier):** (1) the two engines' compensated class-1 search trees are
LEAF-IDENTICAL AS A MULTISET (20 rows in the query box; the dumps' ROW
ORDER differs and the Java side carries a `total_leaves` line, so the
files are not byte-identical; all 8 octagon fields equal) — and the
Java side was dumped from the direct-read construction whose board hash
is `4207fa7f…` (reproduced at HEAD `88fac5944`:
`evidence/instrument_directread_hash.out` — hash-neutral to the tree
reinsert), a THIRD state distinct from the cliflow replication
(`202d1017…`) and the CLI (`b84a773a…`): leaf-identity binds Rust vs
direct-read only, and tree identity against the CLI's live process
remains unproven — exactly what M5 lever (a) must observe; (2) Java's
own `completeShape`, called DIRECTLY from jshell with the reconstructed
inputs — the enlarged room octagon (all 16 diagonal-field variants),
the pin-center point contained octagon, net 21, layer 0, ignore null —
on the manager's own class-1 tree, in BOTH descending and ascending
insert orders, returns **-911136 = the Rust face**, every time; (3) a
jshell replication of the CLI flow (HeadlessBoardManager +
loadFromSpecctraDsn + RouterSettings + RoutingPipeline.run) also
produces -911136, and a DIFFERENT board hash (`202d1017…` vs the CLI's
stable `b84a773a…`) — the CLI's loaded board state differs in a way the
replication does not reach; (4) the CLI subprocess reproduces -911636
deterministically (×2, 1 h 52 min apart); (5) Java's
`signedLineDistance`/`obstacleSegmentTouchesInside` tables (reflection
per border line) MATCH the port — both sides landed:
`evidence/instrument_distance_table_java.out` /
`instrument_distance_table_rust.out` (values equal line-for-line; Rust
prints integral f64 without the `.0`); (6) excluded causes, each verified
from source: `DsnFile.adjustPlaneAutorouteSettings` (bm06 is 2-layer,
early return), the hole-clearance override (DSN 0 == default 0.0 µm →
no reinsert), the copper-to-edge override (flag unset),
`setClearanceCompensationUsed` (GUI-only), tree insert order, the merged
trace costs (settings-only), the adjacent-`.rules` auto-load (none
exists), DSN `autoroute_settings` (absent). The one instrument that
would close it — a recording `ShapeSearchTree45Degree` subclass
injected into the LIVE CLI process to read the engine's actual
completeShape inputs — did not land inside the task budget (jshell/
session-API friction; the attempted subclass form is preserved at
`evidence/instrument_T12RecordingTree45.java.txt`), and the only
remaining route (a jar rebuilt with traces at the restrain sites) is
FORBIDDEN by the oracle freeze.

**The mechanism and the reachable oracle (spec-review MINOR-3, the
reviewer's derivation from the T12 artifacts):** the reconstruction's
completed north face −911136 EQUALS the compensated BoardOutline band's
inner edge (outline leaf bbox ly=−911136, both leaf dumps; centerline
−910036 = `scratch/java_fanoutonly/java-trace.log:31`, the BoardOutline
ITEM_ACTIVITY INSERT row's bounds y2; calibration values:
`evidence/rust_values_dump.out` — outline class 1 × tree class 1
comp=1000, val=2000 = the DSN's 200 µm default at resolution 10), no
other obstacle sits near the
north band in the query box, and the CLI's face is exactly −911636 = one
500-DBU clearance quantum further south — so the CLI's north-band
restraint source (the outline band, or a CLI-only item on it) is 500 DBU
more restrictive than the port's (candidate faces: matrix val 3000 vs
2000 on the outline×class-1 pair, or a thicker raw outline insertion).
`BasicBoard.getHash()` (`BoardSnapshotManager.java:58-72`) printed by
the CLI at fanout start makes the state hunt a CLOSED reproduction
oracle — a jshell loop replicating the CLI load path and comparing
`getHash()` to `b84a773a…` needs no live-process injection. (Labeled
speculative, from the review: a port room extending 500 DBU closer to
the board edge is the DRC-risk direction and rhymes with the known KiCad
copper-to-edge quirk.)

**bm01 wall discharge (the plan's step 4, recorded):** the full
pipeline is NOT wall-limited on bm01 — 20 passes, Σ pass walls 663.0 s
at T11 / 673.6 s at the T12 re-run, stagnation-stopped at
993.16/1/0 (passes 12-20 flat); the M3-era 8.5-13.4 h router-only
natural-end bound is COLLAPSED by the fanout stage. buglog 175 stays
OPEN with the M4 measurement appended; bm01's residual = the one
U53→U8 connection = the same tuning family as bm06 (upstream of it:
buglog 189).

**bm11 stop-face verification (no change):** the port's stagnation stop
fires on Java's exact predicate at the same pass boundary
(`batch.rs:88/:94` — `STAGNATION_PASS_LIMIT` 10, `STAGNATION_SCORE_THRESHOLD`
0.5, the pass-local + global trackers, the one-time fanout recovery;
Java: the constants at `BatchAutorouter.java:46-60`, the stop block at
`AutorouteBatchLoop.java:450-543`) — read-verified this task. The
600 s-tier red is the per-pass wall (62-64 s vs Java's ~21 s TOTAL),
i.e. search cost — the M5-chartered lift; the T10-banked optimizer stop
face is NOT involved and stays banked.

**Battery + gates (the end-state verification, tree byte-identical to
T11's `a0251cc45` — zero feat commits):** census **1422/0/17**
(additions zero); fmt 0; clippy `--workspace --all-targets -D warnings`
0; corpus 5000/5000, dsn `--set all` 1332/0, ses (dsn ses-compare) 20,
ses-snap 5, index 33, undo 33, drc 17, events **3520 aligned exit 0**
(all `EPIC_SKIP_GRADLE=1`, real exits); `router determinism` ×2 byte-
equal with the canary digests UNROTATED (ses `253e7b14…`, manifest
`8c4a9773…`); the full battery re-run (release, gate mode, external
7200 s) re-confirmed the T11 face — 8 PASS / 3 RED, no row moved (the
truth table and triage lines are T11's, re-anchored in the T12 report).

## M4 milestone-criterion map (appended by M4-T13; successor to the T17c map above — adjudication input)

The T17c map above stays exactly as landed (HISTORICAL). Criteria read
the design §6 M4 row's own faces: the pipeline runs (criterion 1), all
fixtures ≥ Java on completion/violations/score (criterion 2, the
zero-regression moment), and the standing §5 guarantee (criterion 3).
The battery of record is the FULL-face battery (T11 face, re-confirmed
at T12; `logs/M4-T12/evidence/battery_full_t12.log`).

| Criterion | Verdict | Evidence + citation |
|---|---|---|
| 1 — the full pipeline runs (fanout + detail router + optimizer assembled) | **MET** | `pipeline/full.rs` mirrors `RoutingPipeline.run()` (T10 dossier); `--profile full` gates the assembled pipeline vs the M0 full-flow records with all 11 `fixture_sha256` re-verified — reuse, not recapture (T11 dossier, "The M0 baselines are REUSED verbatim"); the CLI drives the whole pipeline. The F1/F1′ re-anchor closed at T11 (full-flow bm08 byte-exact ×3). |
| 2 — all fixtures ≥ Java on completion/violations/score | **NOT MET** | 8 PASS / 3 RED, exit 1, 2061.2 s (`battery_full_t12.log:711/:1035/:1064/:1077-1079`): bm01 1/0/993.16 (673.6 s, stagnation-stopped — NOT wall-limited; completion by 1; the 175/181 victim-choice family, upstream 189); bm06 5/8/930.81 vs 2/8/971.63 (incompletes AND score; the 500-DBU room-completion fork at the first fanout search — 189's face, upstream of every maze fork); bm11 wall-integrity 600.2 s, no manifest, the plateau AT the kill = Java's exact terminal face 975.00/3/0 (per-pass search cost — the M5 lift; 175 + the T12 stop-face verification). Violations ≤ Java on ALL 11 fixtures (0 router-introduced anywhere); the full truth table with triage lines is T11's, re-anchored in the T12 dossier above. |
| 3 — the standing §5 zero-regression guarantee (all compares green, no M2/M3 regression) | **MET** | T12 end-state gates (spec-review A5 re-ran all personally, exits recorded): corpus 5000/5000; dsn `--set all` 1,332/0; ses 20/20; ses-snap 5/5; index 33/33; undo 33/33; drc 17/17; events 3,520 golden rows aligned exit 0 (the buglog 172-174 classes stay closed); `router determinism` ×2 byte-equal, canaries UNROTATED (ses `253e7b14…`, manifest `8c4a9773…`); census 1422/0/17. |

**The flip-hold record (M4-T13).** The CI flip precondition is unmet on
BOTH faces: (a) the battery reads 8/3, not 11/11; (b) the suite wall
(2061.2 s release ≈ 34 min) exceeds the compare step's 30-minute
budget (`.github/workflows/rust-check.yml:125`, the compare step's
`timeout-minutes: 30` — line per the post-comment-refresh committed tree).
`--report-only` stays with both `ci_tripwire.rs` pins intact — the
held posture continues from the M3 adjudication, now with M4 evidence
and named M5 exit conditions (full battery 11/11 AND wall under
budget). FACE SEPARATION (the map's non-substitution rule, T17c
precedent): the CI step's own router-only profile has read 11 green /
0 red in 337.7 s since M4-T6 (`logs/M4-T6/evidence/cmp_router_t6.log`,
live-witnessed exit 0 at T11) — a green router-only face is NOT the
flip criterion; the flip is judged on the milestone full-face battery,
and the two vocabularies must not be substituted for each other.

**Events-compare-in-CI decision (M4-T13).** The events step stays OUT
of CI for M4 and rides the SAME flip-commit family when the M5 exit
conditions are met, with its own `ci_tripwire`-style pin. Rationale:
T6's recommendation (SEAM T16 "CI" row) is explicitly "RECOMMENDED AT
THE M4 CI-FLIP MOMENT … adding it to rust-check.yml should ride the M4
CI-flip commit … rather than shipping alone" — it does not argue
independence from the flip; and with the compare gate itself still
`--report-only`, adding events now would widen a gate that is on hold.
The step is CI-READY (java-free, ~0.4 s over three in-process worlds,
exit 0 since M4-T6) — the flip commit adds it in the same edit.

**Pointer rows into the task dossiers:** criterion-1 evidence → T10
(full-pipeline assembly) + T11 (profiles, M0-reuse record, the battery
face-flip table); criterion-2 reds → T12's dossier (the 189 fork
re-derivation + the bm01 discharge + the bm11 stop-face verification)
and buglog 175/181/189; criterion-3 → the T12 gates table + the T11
"Ripple" faces; the fanout/optimizer stage parity → T7/T8/T9 dossiers;
the events close → T4→T5→T6 (buglog 172-174, bug-187); the M3 map and
its snapshot rows above stay HISTORICAL, never overwritten.

## T3 (M5) copper-to-edge override — the 189 fix, the canary rotation, the 181 re-charter (appended by M5-T3)

**The fix (buglog 189 CLOSED).** The Rust load path lacked Java's CLI copper-to-edge
clearance override entirely (`HeadlessBoardManager.applyCopperToEdgeClearanceOverride`,
`:470-556`, called from the manager load flow at `:346`/`:750` — NOT from
`DsnReader.readBoard`, which is why every direct-read oracle probe and the events
corpus (`RouteEventProbe.java:198-205`) legitimately bypass it). Ported as
`epic_cli::route::apply_copper_to_edge_clearance_override` (pub; called from
`run_route` right after the default-tree fill and BEFORE `normalize_all_traces`,
the violation seed, and `apply_board_specific_optimizations` — the Java EFFECTIVE
ordering: `:346` fires inside `createBoard` (the parser reaches it at
`Structure.java:1268`) before the wiring scope's `Wiring.java:347` normalize,
before `:749`, and before the deferred seed's background DRC (`:788-793`) reads
the board; the second Java call site (`:750`) is a skip-gate no-op / idempotent,
so the one parse-time call carries the whole state. FIX ROUND 1 (commit 74728b887's
successor) moved it there from the textual `:749->:750` position after the T3
spec review's Finding 1 — the seed had read pre-override state — and the
settings resolution hoisted to step 1b to feed the override its merged settings;
`apply_board_specific_optimizations` consumes no override-mutated state (it
reads bounds/layers, writes settings only), so its order is state-equivalent
either way and mirrors Java's `:749`-after-`:346`, called from BOTH
in-process flow replicators (the harness detail pass `router_compare.rs`, the T1
alloc instrument `alloc_route.rs`) so the subprocess and in-process faces cannot
split, and from NO direct-read path). Supporting surfaces: the live
`ClearanceMatrix::append_class` (Java `:281-322` port — new row/col init from
class 1 per layer, class 0 included, diagonal `v(1,1)`, maxima never lowered),
`MergedSettings.copper_to_edge_clearance_um = Some(250.0)`
(`DefaultSettings.java:83/:155`; no DSN/CLI source writes it in the frozen tree).

**Pins (cerebrum 16, mutation-verified).** The measured band edges pinned as
LITERALS at a bm06-faithful crafted world (bm06's OWN boundary path — scale-10
parse face; `(rule (clearance 200))` -> v(1,1) = 2000; fallback AREA outline
class): the class-1 outline leaf's north face is -911136 before the override (the
T12 direct-read face) and -911636 after (the Java CLI face), AND the REAL
completion (`get_autoroute_tree(1)` + epic-index `complete_shape` with T2's exact
fork-row input) returns uy = -911136 pre / -911636 post. Mutants: gate-invert M1
(killed by the post-fix pin and the skip-gate contrast pin — the explicit-class
default-value face stays untouched, the non-default-value face still fires),
units+1 M2 (2501 stores 2502 -> leaf -911638, killed), units-3 M3 (2497 stores
2498 -> -911634, killed), reclass-skip M4 (killed); units-1 is ABSORBED by the
even lattice (2499 stores 2500 — Java's own rounding), pinned as the lattice face.

**Canary rotation (sanctioned; the M4-T1 precedent).** The determinism canaries
ROTATED: ses `253e7b1401775172...` / manifest `8c4a97738375e67d...` (the M4-close
values, cited in the README status block) -> ses
`dc271114627632d6a959891559d2ee837c180142c06fb36fde20f441f0f91deb` / manifest
`cf6077141d2f2910b709764b8d3a61ace9e8be2defb85b92b4c956b68b438639`, x2
byte-identical. Why sanctioned: the canary rides bm08, whose outline ALSO sits at
the fallback AREA class — the override promotes it like bm06 — so bm08's route
moves toward Java's live CLI state. This is the FIRST sanctioned digest rotation
in the rewrite; the events golden is NOT part of it (3520 rows aligned, exit 0 —
the STOP condition never fired). The router-only battery rows that moved
(bm01 1->0/1000.00/7 passes; bm02 -> Java-exact 1/960.78/18; bm07 -> 1/984.50/18;
bm06 unchanged at Java's exact 9/8/876.39) are the same movement seen through the
gate faces: every moved row landed ON or past Java's face, 11/0 green.

**The 181 re-charter.** The fix moved bm06's full-flow face 5/8/930.81 ->
3/8/958.02 (Java 2/8/971.63) without collapsing the cascade: the fanout stage's
totals align exactly (101/5/0/+28 pass 1; escaped 113/124 both), but the PASS-2+
ATTRIBUTION fork survives (rust 7/2/+3 @ripup 200 vs jar 6/3/+1 @200,
`logs/M4-T7/evidence/jar_evidence_bm06_t7fix.log`) — the NEW first fork. No
tie-break tuning; owner M6 (T8 re-faces the battery with the updated red row).

## T4 (M5) slice B — the epic-index hot-path allocation elimination (appended by M5-T4)

**The slice (plan Task 5 content, T1's binding ranking #1; commit `perf(m5): slice B …`).**
The T1 profile ranked `epic_index::complete_shape` #1 (family 39.2%/29.2% of all
allocations on bm06/bm11; single site `complete_shape.rs:810` = the 45-degree
KEEP_NON_OVERLAP `room.clone()` push, 36.0%/26.2% — cite re-verified at dispatch HEAD
`e81cd82e7`). ZERO semantic change: same inserts, same walk/iteration order, same
removals, same query results — only the storage/allocation strategy moved. Three
storage-only mechanisms, each value-identical to the owned form it replaced:

1. **Per-thread scratch pool** (`epic-index/src/scratch.rs`, crate-private): the
   complete-shape room double buffer (Java's `result`/`newResult` pair), the DFS node
   stacks (the Java `ArrayStack` of the 45/90-degree walks and `MinAreaTree::overlaps`)
   and the with-clearance sift's sort keys are TAKE/PUT-pooled per thread; capacity
   persists across queries, every consumer clears before filling, the returned room
   list leaves the pool (one `Vec` per query at the floor). Thread-local (not
   per-tree) because the walk APIs take `&SearchTree`; per-THREAD (not global) so the
   M5-T7 deterministic parallelism inherits safety for free (no cross-thread state).
2. **Double-buffer swap instead of fresh Vecs**: `complete_shape` (all three arms)
   reuses Java's `result`/`newResult` pair via `clear` + `swap` per processed obstacle
   instead of allocating a new `Vec` per obstacle; `restrainShape` (all three arms) and
   `divideLargeRoom` gained appending/in-place cores (`*_into` / `*_in_place`) so the
   per-restrain-call `Vec` and the recursion's extends land in the caller's buffer.
   The owned Java-parity surfaces (`restrain_shape_*`, `divide_large_room_*`) remain
   and delegate to the cores.
3. **Query-path elimination**: `SearchTree::clearance_test`'s sort-keys `Vec` is pooled
   (the sift runs per maze element via `expand_to_door_section`); `MinAreaTree::overlaps`
   walks the pooled DFS stack.

Epoch reclamation (design :261's third leg) was EVALUATED AND DEFERRED: the slab
(`shape_tree.rs:16`) keeps removed slots unreachable-but-populated and re-insertion
allocates fresh — with the rip-up churn measured at only 1.6% of allocations
(`min_area_tree`), a free-list/epoch scheme buys nothing T1 measured and risks the
Java-slot-match pin (`search_tree.rs:262`); recorded honestly as not-taken.
SoA field split likewise not-taken: the hot walk reads the WHOLE node kind
(bounds + payload together), so an SoA split adds indirection without removing a
measured chase.

**Pins (+1, census 1430→1431).** `scratch_reuse_across_queries_is_invisible`
(complete_shape.rs tests): a second 45-degree query through the SAME pooled buffers
reproduces the fresh-state T3_simplex octagon exactly and equals a fresh-tree control.
Mutation-verified: dropping the 45-degree arm's `next.clear()` makes the pin unable to
complete — without the per-obstacle clear, surviving stale rooms double every obstacle
(2^obstacles blowup; the mutant dies at any time/memory cap). Kill by non-completion,
witnessed honestly (buglog 194 records the excursion: an unverified bulk mutation sed
briefly took out BOTH `next.clear()` sites (:546 90-degree, :889 45-degree) and the
resulting 14.5 GB test blowup OOM-killed two coordinator sessions; every cargo/harness
command now runs under `systemd-run --user --scope -p MemoryMax=6G`).

**Ripple (all at the slice commit; EPIC_SKIP_GRADLE=1, real exits, memory-capped).**
corpus 5000/5000 exit 0; dsn `--set all` 1332/0 exit 0; ses 20/20 byte-equal; ses-snap
5/5; index 33/33; undo 33/33; drc 17/17; events golden 3520 rows aligned exit 0;
`router determinism` ×2 byte-identical, canaries UNROTATED (ses `dc271114…` / manifest
`cf607714…`, the T3-rotated values); router-only Tier A battery **11 green / 0 red,
every row at T3's exact terminal face** (bm01 0/0/1000.00/7, bm02 1/0/960.78/18,
bm06 9/8/876.39, bm11 14/0/883.33/18) — no row moved. Gates: fmt 0; clippy
`--workspace --all-targets -D warnings` 0; census 1431/0 (+1 = the new pin).

**Wall + alloc delta** (T1 instrument faces, in-process driver window, same machine,
before/after both release + CARGO_PROFILE_RELEASE_DEBUG=2):

| fixture | wall before → after | allocs before → after | bytes before → after | FACE |
|---|---|---|---|---|
| bm08 | 0.103 s → 0.089 s | 569,694 → 433,646 (−23.9%) | 66.7 MB → 53.2 MB (−20.2%) | 0/0/64/1 identical |
| bm06 | 46.15 s → 46.45 s (+0.6%) | 72,950,462 → 41,868,462 (**−42.6%**) | 14.13 GB → 5.52 GB (**−60.9%**) | 3/8/223/21 identical |
| bm11 | 501.96 s → 524.78 s (+4.5%) | 239,982,054 → 155,594,549 (**−35.2%**) | 50.00 GB → 26.20 GB (**−47.6%**) | 4/0/326/55 identical, 18+3 passes both |

Honest reading (T1's own caveat): allocation COUNT and BYTES drop hard on the
allocation-ranked fixtures, but the WALL on the wall-dominated fixtures is unchanged
within this shared box's noise (bm06 +0.6%; bm11 +4.5% decomposed as +18.2 s on pass
#17 PLUS +4.8 s on pass #16, board hashes identical passes #16–#18 both runs — same
work, pass-duration swing: bm11 passes run 0.9–88 s at load ~6). The slice's win is the
churn/bytes floor (peak-live unchanged — the retained buffers are KB-scale); whether it
moves the Σ wall is T8's judgment.

---

## T5 (M5) slice A (re-pointed #2) — maze/drill element churn + the formatting face (appended by M5-T5)

**The slice (T1's binding ranking #2: the formatting face + maze/drill element
churn; plan-Task-4 lineage kept in the commit message).** T1 ranked
`java_double_to_string` #2 (7.8%/8.5% of ALL allocations on bm06/bm11 —
write_scope.rs:148 at dispatch HEAD; the appending core `java_double_to_string_into` lands at :203) with 100% of its sampled hot-path traces
riding `MazeSearchEngine` row construction (`expand_to_door_section`'s
`expansionValue`/`sortingValue` fields, search_engine.rs:1257 at dispatch), the
`epic_router::maze` bucket #3 (4.5%/7.8%) and `expand_to_drills_of_page`'s
drill snapshot #4 (4.3%/6.9%). Two mechanisms, ZERO semantic change — same
inserts, same construction/iteration/pop order, same removals, same row TEXT
byte-for-byte, same stream positions:

1. **The formatting face — allocation elimination, CALLER-SIDE, formatter
   untouched on the output byte-path.** `write_scope.rs` gains
   `java_double_to_string_into(value, &mut String)`: the identical algorithm
   (same `{:e}` shortest-round-trip digits, same layout rules) with the
   `{magnitude:e}` render and the digit extraction moved onto STACK buffers
   (shortest-round-trip ≤ 17 digits; the 17-digit bound is asserted), pushing
   straight into the caller's `String` — zero allocations. The owning
   `java_double_to_string` delegates to it, so the SES/DSN output byte-path is
   UNCHANGED (ses 20/20 + ses-snap 5/5 byte-equal are the direct judges). The
   maze row builders consume the core: both RAW_SECTION sites (skip + assign)
   build into a per-engine reusable row buffer (`row_buf`, take → clear →
   build → emit → restore) and the describes render via appending cores
   (`describe_expandable_into`/`_bounds_into`, `int_box_to_string_into`,
   `point_to_string_into`); the drill arm of `describe_expandable_into` reads
   the live drill BY REFERENCE (the owned form deep-cloned the drill's two
   `Vec`s just to print three fields). Java parity on the construction itself
   is DELIBERATELY preserved: Java builds the raw rows unconditionally at the
   call site and the backend filters — the committed events pin
   `trace_disabled_sink_gates_compare_rows_but_raw_rows_still_flow` (route_events.rs)
   holds a RECORDING trace-disabled sink at 331 assigns, so the alternative
   "skip construction when the sink is silent" design was REJECTED (it would
   change a pinned observable); only the allocation strategy moved.
2. **The drill churn — the deep clone eliminated, live reads proven identical
   by the borrow checker.** `expand_to_drills_of_page` previously snapshotted
   the memoized drill array with `to_vec()` (1 + 2N heap buffers per page
   expansion — every `ExpansionDrill` carries two `Vec`s); now the memoization
   runs in a scoped `&mut` borrow (get_drills called exactly once, same side
   effects) and the loop reads the drill fields through `&DrillPageArray`
   (`page_drill(row, column, d)`), coexisting with the shared reborrow the
   callee takes. `pages`/`ctx` are distinct borrows, the loop body never
   mutates pages, and nothing on the `expand_to_drill` path touches the memo —
   so live index reads are read-for-read identical to the snapshot reads.
   Same loop order (`0..drill_count`), same filters, same `Drill{id}` tags.

**Why a per-engine FIELD, not the epic-index scratch pool** (SEAM "T4 slice
B" item 1, the per-thread take/put pool): the engine OWNS `row_buf` and is
its only consumer — one long-lived buffer, an existing `&mut self` row path,
no `&self`-immutable API to serve, no cross-fn nesting — so the field needs
neither pool machinery nor an epic-index dependency.

**Not-taken (recorded):** a Front-arena/SoA conversion — the charter's
"maze element construction" bucket decomposes at the T1 traces into the row
strings (the slice took these), the drill snapshot (taken), and
`completion::complete_candidates`-path Vec churn (797/2733 bm11 samples — NOT
named by the charter; routed to the coordinator as a T6-adjacent candidate);
the Front `BTreeSet` node churn itself measured <0.5% (the PQ family is T6's
by charter) — an arena there chases unmeasured noise. A `is_trace_enabled()`
gate on row construction was REJECTED (see 1).

**Pins (+2, census 1431→1433).** `raw_row_buffer_reuse_is_invisible`
(maze/pins.rs): three consecutive emissions on ONE engine (assign → skip →
the same assign) must reproduce byte-identical texts for identical inputs,
carry their template prefixes, terminate exactly at their `net=` field (no
stale-residue face), and leave the buffer retained (capacity witness). 
Mutation-verified: (i) drill-loop mutant `0.. → 1..` — KILLED by 8 existing
pins (drill_phase_protocol_pins among them); (ii) `row.clear()` removed at
both sites — KILLED on the text face; (iii) `row_buf` restore removed —
text-invisible (identical rows either way), KILLED by the capacity witness;
(iv) `java_double_to_string_into` clobbering `out` — KILLED by
`java_double_to_string_into_appends_without_clobbering` (write_scope.rs; also
covers the append-only contract: special values, plain range with padding and
leading-zero faces, scientific, appending into a non-empty buffer).

**Ripple (all at the slice commit; EPIC_SKIP_GRADLE=1, real exits,
memory-capped):** corpus 5000/5000; dsn `--set all` 1332/0; ses 20/20
byte-equal; ses-snap 5/5; index 33/33; undo 33/33; drc 17/17; events golden
3520 rows aligned; `router determinism` ×2 byte-identical, canaries UNROTATED
(ses `dc271114…` / manifest `cf607714…`, the T3-rotated values); router-only
Tier A battery 11/11 green, every row at T3's exact terminal face (bm01
0/0/1000.00/7, bm02 1/0/960.78/18, bm06 9/8/876.39, bm11 14/0/883.33/18).
Gates: fmt 0; clippy `--workspace --all-targets -D warnings` 0; census
1433/0/17 (+2 = the two new pins).

**Wall + alloc delta** (T1 instrument faces, in-process driver window, same
machine; before captured at dispatch HEAD `6a4ff650c`, after at the slice
tree; both release + CARGO_PROFILE_RELEASE_DEBUG=2):

| fixture | wall before → after | allocs before → after | bytes before → after | FACE |
|---|---|---|---|---|
| bm08 | 0.103 s → 0.120 s | 433,646 → 361,406 (**−16.7%**) | 53.2 MB → 47.3 MB (−11.1%) | 0/0/64/1 identical |
| bm06 | 49.329 s → 63.330 s (2nd after-sample **42.213 s**) | 41,868,462 → 29,873,741 (**−28.6%**) | 5.521 GB → 4.607 GB (**−16.6%**) | 3/8/223/21 identical |
| bm11 | 546.224 s → 445.087 s (**−18.5%**) | 155,594,549 → 97,051,551 (**−37.6%**) | 26.199 GB → 21.079 GB (**−19.5%**) | 4/0/326/55 identical |

Honest reading: the alloc/bytes drop is real and stable (identical counts across
re-runs — deterministic work), and the FACE lines reproduce exactly. The WALL
readings sit inside this box's load noise (load average 8–9 during the runs;
bm06's two after-samples differ by 21 s with BYTE-IDENTICAL allocation counts
— same work, pure pass-duration swing, the same phenomenon T4 decomposed on
bm11): bm11's −18.5% should be read as "possibly helped, unproven on a shared
box", bm06's +28%/−14% spread as noise. The slice's proven win is the churn
floor (T1 sites #2/#3/#4 cut 28–38%); the Σ-wall judgment stays T8's.

---

## T6 (M5) slice C — geometry/rational churn + the maps/PQ family + the completion-candidates churn (appended by M5-T6)

**The slice (T1's binding ranking #3):** the T5-surfaced
`completion::complete_candidates` churn + the compute-side maps/PQ family
(plan-Task-4 content, <0.5% allocations, wall-judged) + the geometry
temporaries. The T1 allocation ranking was RE-MEASURED at the dispatch HEAD
`b5bbca54b` (post-T4/T5 the allocation profile changed shape): bm06's 299
samples / bm11's 971 samples now rank the Simplex/BigInt intersection
family, `tree_shape_precalc` cache-clone, `min_area_tree::overlaps`, the
completion door scan, `divide_segment_into_sections`, and — via an op-count
instrument (patch → measure → revert) — **411.7M LINEAR REGISTRY SCANS on
bm11 (77.3M on bm06)** in `registry_resolve`'s live-registry arm
(`keys.iter().position()` per resolve; Java resolves by object reference at
O(1)). That linear scan was the family's real compute-side cost — not
B-tree rebalancing.

**Mechanisms (all storage/allocation strategy; ZERO semantic change — same
inserts, same construction/iteration/pop order, same removals, same query
results, comparators untouched):**

1. **The registry key→index side map** (`engine.rs`): `key_index:
   HashMap<u64, usize>` answers `registry_resolve`/`room_mut` with the
   same index the linear scan found (keys unique — monotone
   `alloc_key`); maintained at every push, rebuilt on the
   order-preserving removal (`drop_room_reference`), cleared in
   `clear()`. The rebuild REUSES the table's capacity (clear + insert
   loop): the first draft's fresh-`collect()` per removal allocated a
   full new table at every graveyard move and DOUBLED bm06's byte churn
   (measured 4.61 → 8.95 GB, alloc_bm06_after2_v1_rebuildcollect.* in
   `logs/M5-T6/evidence/` — the excursion is retained as the honesty
   record; the pin battery stayed green through it, which is exactly
   why the alloc delta protocol exists). The rebuild itself is
   allocation-free at steady state. The `EngineShapeView` snapshot
   carries the map by reference, so the completion-query resolves take
   the same path. SCOPE DISCLOSURE (spec round NIT-6): the side map
   serves `registry_resolve`/`room_mut` only — the REMOVAL paths keep
   their linear scans (`keys.iter().position`/`contains` at the
   removal/clear sites); they are bounded by the measured removal op
   counts (145k graveyard moves/run on bm06, 352k on bm11) and were
   left untouched. Graveyard/obstacle_rooms/room_tree_entries/
   ripped_item_list stay BTreeMap: the op-count instrument measured
   graveyard 145k ops (max 5,609) / 352k (max 14,579 bm11),
   obstacle_rooms 121k/263k (max 416/841), room_tree_entries 31k/91k,
   ripped_item_list 285/388 ops (max 2/5 — negligible); B-tree
   rebalancing at those sizes is noise. (Instrument outputs retained at
   the dispatch HEAD: `opcount_bm06_pre_slice.log` / `opcount_bm11_pre_slice.log`
   in `logs/M5-T6/evidence/` — spec-round MINOR-1 closure.)
2. **`complete_candidates`** (`maze/completion.rs`): the per-first-candidate
   one-element `vec![cell]` is gone (the first 2-dim candidate registers
   directly), and the registration body (`register_candidate`) MOVES the
   candidate's two shapes instead of cloning them (the candidate value has
   no other reader — Java clones its own copies; the constructed room is
   field-for-field identical). Same candidate order, same
   first/recalc split, same key burn order (12 existing completion pins
   kill the recalc-skip mutant).
3. **`divide_segment_into_sections_into` + `get_section_segments_into`**
   (`epic-geometry/float_line.rs`, `expansion/door.rs`): appending cores
   (clear-before-fill) with owned delegates kept; the maze engine's
   `expand_to_door` takes the per-engine `section_buf` (the T5 `row_buf`
   field pattern), restores on the normal exit, and drops it on the early
   `return false` paths (capacity loss only). `allocate_sections`
   re-segmentation reuses capacity (clear + resize; all-default values
   identical — capacity-only). An observability note (spec round NIT-4):
   a capacity witness IS possible here (the T5 `row_buf.capacity()`
   precedent) — it was not added because the VALUES are already covered
   by the existing maze-section pins and the capacity face would pin
   only the reuse implementation, not any Java-parity behavior.
4. **`big_integer_int_value`** (`big_int_aux.rs`): the low-word extraction
   walks `iter_u32_digits()` (zero-alloc) instead of materializing
   `to_u32_digits()` — identical truncation (both answer the
   least-significant nonzero word; both yield nothing for zero).
5. **NOT-TAKEN (each with its measured derivation, bm06/bm11 shares):**
   - **Front PQ (BTreeSet) arena/SoA conversion** — the op-count
     instrument measured 574k adds + 474k pops (bm06) and 2.72M adds +
     2.36M pops (bm11, max depth 37,099): ~1-2% of the event loop's op
     volume at B-tree-log cost each, i.e. an unproven ≤5% wall on a
     structure whose dedup + NaN-fall-through comparator semantics are
     load-bearing (list_element.rs module docs); the comparator is
     INVARIANT and the storage conversion risks order drift for a
     compute win the profile does not name.
   - **Simplex intersection family** (`remove_redundant_lines`/
     `intersection_simplex`/`to_simplex`/`offset`; 12/299 bm06, 36/971
     bm11 samples): the loops mutate the line vector while holding
     prev/current/next line values (borrow-checker rejects
     reference-ification), the nothing-removed arm returns a clone, and
     the float-tolerance control flow (`side_of_intersection`'s float
     fast path feeding the exact check) must not be restructured —
     algorithm-inherent owned transforms.
   - **`tree_shape_precalc` cache-hit clone** (board.rs; 9/299 bm06,
     12/971 bm11): owned-return API serving six call sites that mutate
     the board between reads — a borrow-return would ripple; an
     epic-board scratch pool is out of slice scope.
   - **`min_area_tree::overlaps` `found` Vec** (12/971 bm11): owned
     return (result leaves any pool); single growth chain per query.
   - **`insert_entry_point` Polyline/nets clones** (24/971 bm11): the
     entry OWNS the data afterward — an Rc refactor is out of slice
     scope.
   - **SIMD kernels**: out of scope by charter; no kernel ≥15% named by
     the profile (none expected) — decision recorded.

**Ripple + wall/alloc delta + pins/census (the T4-dossier shape, appended
by the M5-T6 quality round — this paragraph is T8's durable reference; the
slice is the milestone's FIRST COMPUTE-SIDE WALL WIN):**

| fixture | wall before → after | allocs before → after | bytes before → after | FACE |
|---|---|---|---|---|
| bm08 | 0.095 s → 0.088 s | 361,406 → 347,391 (−3.9%) | 47.32 → 47.16 MB | 0/0/64/1 identical |
| bm06 | 40.444 s → 25.202 s (**−37.7%**) | 29,873,741 → 27,584,115 (−7.7%) | 4.607 → 4.575 GB | 3/8/223/21 identical |
| bm06 (2nd sample) | 25.377 s | 27,584,115 (byte-identical) | 4,574,763,300 (byte-identical) | identical |
| bm11 | 420.912 s → 187.578 s (**−55.4%**) | 97,051,551 → 88,140,377 (−9.2%) | 21.079 → 20.923 GB | 4/0/326/55 identical |

Honest reading: the wall wins are the mechanism's (411.7M/77.3M O(n)
registry scans → O(1) hash lookups), stable across the two bm06 samples
whose allocation counts AND bytes are byte-identical (determinism of
work); bytes ~flat (−0.7%) because the linear scans never allocated —
this slice's win is compute-side, the alloc-count drop (−7.7%/−9.2%) is
the completion/section/bigint takes. The T1 "allocation share ≠ wall
share" caveat runs the OTHER way here: a <0.5%-of-allocations family
turned out to carry the wall. The Σ-wall judgment stays T8's, which
should carry slice C as wall-proven on the two heavier fixtures.

**Ripple (what the slice ripples forward):** T8's Σ wall table (the
bm11/bm06 walls above are the per-fixture faces); T7's deterministic
parallelism (the side map is per-engine state, unshared — thread-safe by
construction); the op-count evidence files (`opcount_bm06_pre_slice.log`
/ `opcount_bm11_pre_slice.log`, byte-reproducible at the dispatch HEAD)
are the durable witness for this dossier's every op count.

**Ripple faces (all at the slice commits; EPIC_SKIP_GRADLE=1, real exits,
memory-capped, re-run on the final tree):** corpus 5000/5000; dsn
`--set all` 1332/0; ses 20/20 byte-equal; ses-snap 5/5; index 33/33;
undo 33/33; drc 17/17; events golden 3520 rows aligned; `router
determinism` ×2 byte-identical, canaries UNROTATED (ses `dc271114…` /
manifest `cf607714…`); router-only Tier A battery 11/11 green, every row
at the exact terminal face (bm01 0/0/1000.00/7, bm02 1/0/960.78/18, bm06
9/8/876.39, bm11 14/0/883.33/18). Census **1433 → 1435** (+2 = the two
new pins), both mutation-verified with BOTH kill faces witnessed
independently: `t6_registry_side_map_tracks_removal_rebuild` (the
+1-index mutant dies on k2's wrong-shape assert; the
rebuild-only-first-key mutant dies on k3's resolution failure with k2
passing) and `divide_segment_into_sections_reuse_is_invisible` (the
missing-clear mutant dies on the stale-tail length face). Gates: fmt 0;
clippy `--workspace --all-targets -D warnings` 0; census 1435/0/17.

## M5-T7 — deterministic per-net partitioned executor + `--threads` + the door.rs BACKLOG pins (appended by M5-T7)

**The charter's conflict analysis, and the executor it forced (the honest
core of this slice):** with ripup enabled EVERY batch-pass search may rip
any item on the board (`ctrl.ripup_allowed = true`, net-agnostic), so
every (item, net) work unit is a CONFLICTED point — a concurrent search
could not observe the sequential interleaving the byte contract demands.
The charter's own escape arm ("serialize exactly the conflicted points
and parallelize only the independent searches") therefore resolves to:
serialize ALL units in the golden walk order and parallelize nothing that
could move a byte. What the executor delivers at N>1 is the partitioned
STRUCTURE (fixed net-id partitions, per-partition worker residency so
the thread-local scratch pools and caches warm per partition) with the
wall delta honestly ≈ neutral (below); search-level parallelism needs
the future global-routing plans (design §4 stage 2) to make independence
provable a priori.

| Seam | Status | Notes |
|---|---|---|
| Fixed net-id partitioning (`pass_runner::net_partition`) | LIVE | `((net-1) * threads) / max_net` onto contiguous ranges — a pure function of `(net, threads, max_net)`, monotone, total (out-of-domain arms pinned). The partition decides WHICH worker executes a unit, never WHEN; the deterministic reduction is the golden walk order itself (queue order × net order — the strictly finer order over "sorted by net id, then tie order": the golden order IS net-id-grouped per item and it is the only order that reproduces the bytes). |
| The partitioned executor (`pass_runner::run_partitioned`) | LIVE | Scoped-thread fleet of `threads` workers; the board/manager/sink environment (`UnitEnv`) is handed off per unit through channels; the coordinator dispatches ONE unit at a time and blocks on the reply — exclusive access by construction. The `Send` assertion is the workspace's single deliberate `unsafe` opt-in outside the SIMD charter (`#[allow(unsafe_code)]`, soundness contract on `SendUnitEnv`: one-in-flight dispatch + no thread-affine state — the only production thread-locals are the epic-index scratch pools, owned `Cell<Option<Vec<_>>>` buffers with take/put inside one call; the per-engine `row_buf`/`key_index` are created within one attempt). |
| The shared unit body (`run_attempt` / `max_items_gate` / `begin_pass` / `finish_pass`) | PURE MOTION | The sequential `run_single_thread` is byte-for-byte the pre-T7 walk over the extracted helpers; the executor runs the SAME body, so the golden bytes cannot drift between faces. `run_pass` is the BATCH stage's pass entry (dispatch on `settings.max_threads`; 0/1 → sequential); the optimizer's reroute passes call `run_single_thread` directly (sequential only at this tree — scope comment at the call site, spec round F3). |
| `--threads` CLI face | LIVE, default 1 | `router.max_threads`/`-mt` was parsed-but-unused; now `route.rs` reads the EXPLICIT CLI face into `BatchSettings.max_threads` (default 1 — the golden path — even though the merged surface keeps Java's `max(1, cores-1)` validation default; the executor engages only on an explicit `-mt`). `BatchSettings::new` seeds 1. The optimizer stage's inline reroute passes still call `run_single_thread` directly (the scope comment now exists at that call site — spec round F3(ii)). **Spec round F1:** the executor's reply channels are PER-WORKER, so a panicking worker fail-fasts the pass with the unit identity (the `#[should_panic]` pin `t7_f1_panicking_worker_fails_fast_with_unit_identity`; the natural no-propagation mutant hangs and is disclosed unpinnable-without-timeout). |
| DOOR_TAG_COUNTER threads strategy (`expansion/door.rs`) | RECONCILED-BY-SERIALIZATION | The plan's "reconciled to the sequential counter order" arm: the executor's one-in-flight discipline makes every tag draw happen in the golden sequential order regardless of thread count — strategy documented at the counter site; the "hoist to the reduction" arm is explicitly not needed while the board is fully serialized. |
| The M4-T2 BACKLOG pins (door.rs) | LANDED (the obligation closed) | `empty_door_shape_answers_empty_before_dimension_dispatch` (Java `ExpansionDoor.java:109-112` — the EMPTY arm fires before any dimension dispatch; world: dim=1 door on a disjoint-intersection empty shape) and `dim2_complete_rooms_without_common_border_corners_answer_null_arm` (Java `:124-127` — fewer than two distinct common-border corners → null → `(0, empty)`; world: room 1 strictly contains the door shape, room 2 IS it, shape deliberately LARGE so it cannot be confused with T2-P3's small-door arm). Both mutation-verified. |
| Threads-invariance gate (harness `router threads-invariance`) | LIVE, exit-0 truth | New java-free gate: bm08 + bm06 (default fixtures), three faces `-mt 1/-mt 3/-mt 4`, byte-identical SES + manifest required across all three. `route_argv_threads`/`run_cli_threads` extend the single-source argv builder. bm06 doubles as the budget-exercising world (18 passes up the deterministic ladder). |

**Threads-invariance + ripple faces (all at the slice tree, release profile,
EPIC_SKIP_GRADLE=1, memory-capped, real exits):** threads gate GREEN on
both fixtures — bm08 ses `dc271114…` / manifest `cf607714…` (== the
dispatch canaries — the golden path unrotated THROUGH the pure-motion
refactor) byte-identical across -mt 1/3/4; bm06 ses `77caf2099a2f…` /
manifest `90f08ad2…` byte-identical across -mt 1/3/4 (18-pass budget
ladder exercised). BONUS face: bm11 SES `d979bfea…` byte-identical
across the same three faces. The seven compares: corpus 5000/5000; dsn
`--set all` 1332/0 (digest 175/175, soak 1157/1157); ses 20/20
byte-equal; ses-snap 5/5; index 33/33; undo 33/33; drc 17/17; events
golden 3520 rows aligned; `router determinism` ×2 canaries UNROTATED;
router-only Tier A battery **11/11 green, every row at the exact
terminal face** (bm01 0/0/1000.00/7 passes wall 135.6s; bm02 1/0/960.78/
18; bm06 9/8/876.39/18; bm11 14/0/883.33/18 — no row moved).

**Wall delta on bm11 (the parallelism-sensitive fixture; release
single-fixture re-runs, fanout-off argv, honest):** -mt 1 = 32.99 s,
-mt 3 = 32.83 s, -mt 4 = 32.86 s — delta ≈ 0 (−0.5%, within noise). The
executor buys structure, not speed, today: at N>1 the hand-off cost
(µs-scale rendezvous per unit) is offset by per-partition warm caches,
netting neutral. NOT a regression either — the charter's explicit
allowance (the executor is the deliverable; T8 judges). Search-level
wall wins need provable search independence, which needs the design §4
global-routing plans.

**Pins + census (1435 → 1443 across feat + spec-fix, +7 +1):** door.rs `empty_door_shape_answers_
empty_before_dimension_dispatch` (mutant: empty-check disabled → the
dim=1 dispatch panics on the empty shape — KILLED) and
`dim2_complete_rooms_without_common_border_corners_answer_null_arm`
(mutant: NULL-arm `return 0` → `return 1` — KILLED); pass_runner
`t7_net_partition_exact_boundaries` (mutant: boundary `max` → `max+1`
— KILLED), `t7_executor_rows_and_hash_identical_across_threads` (mutant:
executor-side unit increment dropped → attempt-count divergence —
KILLED), `t7_executor_engages_distinct_partition_workers` (witness
threshold ≥4 threads — the FIRST threshold ≥2 SURVIVED the
always-partition-0 mutant because the coordinator's prelude/tail rows
count as a second thread; strengthened and the mutant RE-KILLED — the
honest correction record), `t7_deterministic_budget_ladder_thread_
invariant` (ladder faces incl. the exact saturation boundary pass 16 =
i32::MAX; a pure function + per-attempt budget instance — no shared
thread state to move a limit); harness `threads_gate_argv_appends_mt_
flag` (mutant: `-mt` append dropped — KILLED) + the threads-invariance
verdict faces inside `determinism_check_catches_a_planted_divergence`
(mutant: verdict comparing only -mt 1 vs -mt 3 → the planted EVEN-N
faces die — KILLED). Spec round F1 adds the partitioned fail-fast pin
`t7_f1_panicking_worker_fails_fast_with_unit_identity` (its mutation
record in the spec-fix paragraph below). Gates: fmt 0; clippy
`--workspace --all-targets -D warnings` 0; census **1442/0/17 at the
feat commit `923f16ae5`, 1443/0/17 at the spec-fix `c6d148a7a` (its
F1 pin) and at the docs addendum `3092f2effb` (carried, its own
record), 1444/0/17 at the quality round `977e07a47` (its +1 Q1
verdict-face pin)** — each figure attested in its own commit message
— commit-anchored, never "at HEAD" (the T7-quality-r2 N2 closure: an
"at HEAD" census figure goes stale on the very next pin-adding
commit).

## M5 hold posture (appended by M5-T9; the flip record at the SEAM surface)

The M5 close adjudicated the CI flip **NO-GO on both preconditions**
at T8 (`logs/M5-T8/report-t8.md` §(e); measurement-only — no engine
change, tree untouched). This section is the append-only posture
record; everything above it stands as landed.

| Row | Record | Evidence |
|---|---|---|
| Flip decision | **NO-GO, both preconditions unmet**: battery 8/11 ≠ 11/11 AND Σ wall 2286.1 s harness-elapsed (≈ 38.1 min) > the compare step's 30-min budget (`rust-check.yml` `timeout-minutes: 30`). `--report-only` and BOTH `ci_tripwire.rs` pins stay INTACT (the NO-GO rule) — the second consecutive honest hold, with the failure composition changed from M4 | `logs/M5-T8/report-t8.md` §(a)/(b)/(e); `logs/M5-T8/evidence/battery_full_t8.log` |
| Battery face (composition changed) | bm01 routing now COMPLETES 0/0/1000.00 at P8 in ~276 s (fanout 4.43 s + routing P1–P9 271.26 s = 275.69 s) — the maze-search wall is GONE (M4: kill-at-plateau 668.8 s; bm11 600.2 → 197.7 s; bm06 80.0 → 25.0 s); the bm01 red is the T1b-parity-corrected optimizer stage, killed at the 1800 s tier on a non-improvable 0.00-score incumbent (not a search regression); bm06 3>2 and bm11 4>3 incompletes are the buglog-181 completion residuals, owner M6 | §(a) table; `bm01_stderr_t8.log`; T3/T6 dossiers above |
| Criterion 1 (Σ wall ≤ 0.5× Σ-Java) | **MISS** — Σ 2286.1 s harness-elapsed = 2.02× Σ-Java 1130.39 s vs the ≤ 565.19 s target; like-for-like per-fixture excl-bm01: 256.7 s = 2.42× (both bases labeled, per finding MINOR-1; verdict invariant under either basis) | evidence: `logs/M5-T8/report-t8.md` §(b) |
| Criterion 2 (trend toward 10× on large boards) | **NOT MET** — speedup peaks small (bm08 4.51×) and collapses large (bm11 0.24×, bm06 0.69×, bm09 0.57×, bm01 ≤0.57× kill-truncated); the M4→M5 compression is the progress sentence: bm11 ≥12.5×→4.13×, bm06 4.63×→1.45×, bm07 7.74×→0.58× slowness | evidence: `logs/M5-T8/report-t8.md` §(c) |
| Criterion 3 (threads-invariance) | **MET** — byte-identical across `-mt 1/3/4` at the T7 hashes (bm08 ses `dc271114…` / manifest `cf607714…`; bm06 ses `77caf2099a2f…` / manifest `90f08ad2…`) | evidence: `logs/M5-T8/report-t8.md` §(d); the T7 dossier above |
| Threads-gate CI wiring | **RE-DEFERRED to the M6 flip-adjacent moment** (coordinator decision riding T8's closeout): the plan's NO-GO branch is ONE docs-only commit, so no CI step additions ride T9; the gate stays pinned in-tree (green, hashed, argv + verdict + override-naming pins) | T7 dossier (pins + hashes); the T8 closeout decisions of record |
| Suite census | 1444/0/17 at `977e07a47` — commit-anchored in the T7 dossier above, never "at HEAD"; both `ci_tripwire` pins ok inside the bare suite | T8 gates r2 (`census_t8_r2.log`) |
| M6 flip conditions | the wall work — the bm01 optimizer-entry question (should the optimizer be entered at all on a 1000.00/0/0 incumbent; re-reading Java's preflight guard is the first M6 step) + the 1800 s tier bound + the bm06/bm11 completion levers (buglog 181) — until the full battery reads 11/11 AND its wall fits the CI budget; the flip commit then also brings the events step with its own pin | T8 concerns (1)–(3); the M4 criterion map's flip-hold record above |

## M5 milestone-criterion map (appended by M5-T10; successor to the M4 map above — adjudication input)

The M4 map above stays exactly as landed (HISTORICAL), as does the M5
hold-posture table (T9). Criteria read the design §6 M5 row and the
plan's "Milestone exit criteria" faces. Battery of record:
`logs/M5-T8/evidence/battery_full_t8.log` (measured at `977e07a47`;
the posture chain is T9's `e6302bb3e` + `8950deba9`).

| Criterion | Verdict | Evidence + citation |
|---|---|---|
| 1 — Tier A Σ wall ≤ 0.5× Σ-Java | **NOT MET (both bases labeled)** | Σ 2286.1 s harness-elapsed = 2.02× Σ-Java 1130.39 s vs the ≤ 565.19 s target (4.04× target); like-for-like per-fixture excl-bm01: 256.7 s = 2.42× — the verdict is invariant under either basis (MINOR-1 discipline; basis labels on every Σ). Evidence: `logs/M5-T8/report-t8.md` §(a)/(b); `logs/M5-T8/evidence/battery_full_t8.log`. |
| 2 — large-board trend toward 10× | **NOT MET** — speedup (Java/Rust) peaks small (bm08 4.51×, bm02 2.31×, bm07 1.73×) and collapses large (bm06 0.69×, bm09 0.57×, bm11 0.24×, bm01 ≤0.57× kill-truncated); the M4→M5 compression is the progress sentence, not the verdict: bm11 ≥12.5×→4.13×, bm06 4.63×→1.45×, bm07 7.74×→0.58× slowness | Evidence: `logs/M5-T8/report-t8.md` §(c). |
| 3 — `--threads N` ≡ `--threads 1` byte-identical, pinned in-tree | **MET** | `router threads-invariance` re-faced at T8 with fresh bins: exit 0, 6 runs 127.7 s, ses `77caf2099a2f…` / manifest `90f08ad2b94d…` byte-identical across `-mt 1/3/4` — the T7 pinned hashes EXACT (T7 dossier above; landed `923f16ae5`); the gate's CI wiring RE-DEFERRED to the M6 flip-adjacent moment (green, hashed, in-tree only). Evidence: `logs/M5-T8/report-t8.md` §(d). |

**Erratum (T10-era, cite-numbering only):** the M4-era prose cites the
compare step as `rust-check.yml:125` — the design doc's M4-parenthetical
passage (the M4 exit note's flip-hold paragraph, including the T9
parenthetical chain) and the M4 map's flip-hold record above. Those
anchors are T13-era numbering, one yml comment-refresh behind: after
T9's comment refresh the compare run line is `:138` and its
`timeout-minutes: 30` is `:139` (verified at the T10 tree). The M4
texts stay append-frozen; read their `:125` as `:139`. The design
doc's M5 parenthetical (T9) already carries the same fact for its own
context.

**Pointer rows into the task dossiers:** criterion-1/2 before/after
faces → T1 (`logs/M5-T1/report-t1.md` §(b)) + T8 (§(a)–(c)); the bm01
optimizer-entry/tier-bound question → T8 concern (1), buglog 196,
owner M6; the bm06/bm11 completion reds → buglog 181 (owner M6) + the
T3/T6 dossiers above; 189's close → the T3 dossier (fix commit
`74728b887`); threads-invariance → the T7 dossier + the T8 §(d)
re-face; the flip-hold → the M5 hold-posture table above (T9) and the
design §6 M5 exit note.

## M6 hold posture (appended by M6-T4; the flip record at the SEAM surface)

The M6-T3 close adjudicated the CI flip **NO-GO (honest hold) on both
preconditions** (measurement-only — no engine change; this T4 commit
is the docs-only record). Append-only posture record; everything
above it stands as landed. Full record: `logs/M6-T3/report-t3.md`;
the yml comment is the primary durable record.

| Row | Record | Evidence |
|---|---|---|
| Flip decision | **NO-GO (honest hold), both preconditions failing — the third consecutive hold.** Precondition 1 FAILS on the harness verdict face: **10 green / 1 red** (bm01's 1800.2 s tier kill on the INTEGRITY gate — harness timeout, no manifest, exit None; NOT a completion failure — pre-kill router face 0-unrouted/1000.00, optimizer P1 571.93 s REJECTED `OPTIMIZER_SCORE_NOT_IMPROVED`, incumbent restored) while the completion-only reading PASSES (**11/11**; incompletes ≤ Java and violations ≤ Java on all ten completers). Precondition 2 FAILS outright: Σ Tier A = **2089.5 s** > the compare step's `timeout-minutes: 30` (= 1800 s), over by 289.5 s = 16.1%. Either failing ⇒ NO-GO; both fail. `--report-only` and BOTH `ci_tripwire.rs` pins stay INTACT (the NO-GO rule) | `logs/M6-T3/report-t3.md` headline; `logs/M6-T3/evidence/battery-A-full.log`; the rust-check.yml compare-step comment |
| Flip decision | **GO — THE FLIP LANDED at M8-T8 (2026-09-29, tree `9fe3a0751`), the fifth moment, adjudicated on the arithmetic (`logs/M8-T8/report-t8.md` §5):** the exact post-flip CI face (router-only profile, mode: gate) reads **11 green / 0 red, Σ 286.5 s** (DNR-18-reconciled, exit 0; CI debug-history datum 337.7 s) — 5.3–6.3× headroom under the 1800 s step budget; strip context: the full-flow Σ with the T7 lever (1340.78 + 285.7 = 1626.5 s) also fits (lever opt-in, NOT load-bearing for the CI step — the router-only profile never engages the optimizer). The one-commit protocol executed in full (`--report-only` dropped; compare-step pin re-pinned to the exact GATE argv; events-compare step added with its own pin asserts + 10-min wall; posture prose refreshed). The four prior holds (M4 close, M5-T8, M6-T3, M7) stand as history | `logs/M8-T8/evidence/80-gate-laps.log`; `.github/workflows/rust-check.yml`; `rust/harness/src/ci_tripwire.rs` |
| Battery faces | The first tier-aware full batteries (`--tier A|B|C`, default A byte-identical): Tier A 10 green / 1 red (Σ 2089.5 s), Tier B 4 green / 5 red (Σ 3197.9 s), Tier C 1 green / 2 red (Σ 5102.6 s) — the honest BEFORE-faces T10's strictly-> adjudication judges; completing pour fixtures' post-route totals equal Java's committed counts exactly, 0 router-introduced violations on every completing fixture | `logs/M6-T3/report-t3.md` tables; `logs/M6-T3/evidence/battery-{A,B,C}-full.log` |
| Reopen bound + lever | The other ten Tier A walls sum 289.3 s (4.8 min) — the flip reopens exactly when bm01's total completes **< 1510.7 s**. Named lever, RECORDED NOT ATTEMPTED: deterministic candidate parallelism (buglog 197; Java `DefaultSettings.java:182` `optimizer.maxThreads = cores−1` vs Rust's mandated 1-thread face; the M5-T7 partition pattern) | `logs/M6-T3/report-t3.md` reopen arithmetic (coordinator re-summed); buglog 197 |
| Bound-rise adjudication | **NOT taken** — the wall drops, the bound does not rise (the same discipline as the tiers.yaml timeouts); the prior comment's "or this bound to rise" option is retired as policy | the rust-check.yml compare-step comment (M6-T4 refresh) |
| Threads-gate CI wiring | **RE-DEFERRED WITH CAUSE** (the M5-T8 decision (2) deferral continues): the wiring lands WITH the flip commit; the cause of the continued deferral is that the flip preconditions failed at M6-T3. The gate stays pinned in-tree (green, hashed, argv + verdict + override-naming pins) | the rust-check.yml comment (THREADS-GATE paragraph); the T7 dossier above |
| Suite census | **1449/0/17 at the T3 quality round `358e56e7f`** (carried to this T4 record — the T4 docs commit adds no tests, so the census is unchanged; commit-anchored, never "at HEAD"); both `ci_tripwire` pins ok inside the bare suite | `logs/M6-T3/evidence/gate-cargo-test-t3fix.log` (1449 attested at `358e56e7f`); `logs/M6-T4/evidence/gate-cargo-test.log` |
| Residuals | B/C completion reds = the T8/T9 tuning population (bm05 29v18, interf_u 1v0, 1Bitsy 2v1, bm04 3v2); kill faces bm01 (A) + bm10/StickHub/LimeSDR (B/C); the **fanout-full-flow gate gap** — the events fixtures are capture replays that never exercise the changed fanout arm and the router-only/ses/det gates run fanout-disabled ⇒ zero byte-invariance coverage for the buglog-181 fix until a fanout-full-flow golden exists (guards: the pins + sanctioned faces + batteries) — a standing exposure until then; the Σ-decomposition battery print and the banked quality-r2 NITs (shared no-extras sweep helper, bail! idiom consistency, multi-concern walk pin) wait on the first harness touch; T5's pour worklist: 43 parse-time plane-induced rows on multichannel_mixer-unrouted / CM5_MINIMA_3 (the Java post-route real-walk instrument is T5's first item) | `logs/M6-T3/spec-review-t3-1.md` PLAN-LEVEL findings 3/4; `quality-review-t3-2.md` Q2; the README CI-posture comment (the ledger's home) |

## M6 milestone-criterion map (appended by M6-T11; successor to the M5 map above — adjudication input)

The M5 map above stays exactly as landed (HISTORICAL), as do the M5
hold-posture (T9) and M6 hold-posture (T4) tables. Criteria read the
design §6 M6 row and the M6 plan's "Milestone exit criteria" faces.
Batteries of record: `logs/M6-T3/evidence/battery-{A,B,C}-full.log`
(the BEFORE-faces, measured at `62c417a38`) and
`logs/M6-T10/evidence/battery-{B,C}-on.log` +
`battery-A-default.log` (the AFTER-faces, measured at `7b1b44c36`);
the engine tree is frozen at census 1497/0/17 across the close-out.
The verdict slot in the design doc §6 M6 exit note was filled by the
milestone adjudication (2026-09-28, tree `a6d6ac38c`):
**M6_EXIT_WITH_DEVIATIONS** — this map records the measured faces the
verdict was adjudicated from.

| Criterion | Verdict (the measured face; the design doc §6 note carries the adjudicated verdict) | Evidence + citation |
|---|---|---|
| 1 — Tier B/C completion strictly > Java (12 B/C fixtures) | **NOT MET on the measured faces** — 2/12 strictly after the sanctioned tuning (1-Wire-Wing 0<1 via the empty flag subset; bm04 0<2 via push_shove + max_passes=6), 4/12 exact parity (complex after tuning, mm, mm-u, StickHub after tuning), 6/12 red; ALL-ON measured NET-HARMFUL vs default (B 7-vs-5 red, C 3-vs-2; 1-Wire win→loss, complex parity→+3, CM5 complete→kill); best-known regime **default + push_shove-only** (the two strictly wins + StickHub kill→completion @321.5 s at exact Java parity 4=4/1=1/976.85=976.85). Both faces recorded | `logs/M6-T10/report-t10.md` criterion-1 table + tuning-iteration records; BEFORE-face `logs/M6-T3/report-t3.md` |
| 2 — Issue093-class violations = 0 (pour population, REAL counter) | **MET** — router-introduced = 0 on every completing pour fixture on BOTH faces (T3 census 10 fixtures; T10 both arms; counter = `router_introduced_count` ← `all_clearance_violation_depths`, the `getAllClearanceViolations` class — never the outline-only `BoardStatistics` face); post-route totals equal parse-time input counts (16/1/285/29) = Java's committed counts; two honest kill-absences (StickHub both faces, CM5 ON) + StickHub's tuning-face completion 4/1/ri 0; bm10's ri=1 is NOT a criterion-2 breach (no pour) — deviations row, buglog 205 | `logs/M6-T10/report-t10.md` criterion-2 section; T5 audit `fa8654edb` (zero plane-face divergences, both engines 0 introduced) |
| 3 — island detection works | **MET** — T6 detector landed (`epic-board/src/islands.rs`, 4-connected regions, MIN_BRIDGE_WIDTH boundary mutation-verified both directions, advisory `pour_islands` face, pour-free manifests byte-unchanged); T8 region-level clamp (`pour_region_seeded_by`, islands.rs:825, behind `router.plane_island_clamp`); T10 zero-divergence re-confirmation at HEAD (all 10 pour-fixture counts match T6 exactly) | `logs/M6-T10/report-t10.md` criterion-3 section; the T6/T8 dossiers above |
| 4 — inherited residuals (196 / 181 / flip / threads-gate) | **DISCHARGED AS RECORDED** — 196 CLOSED `e0ccffb74` (premise falsified, AMENDMENT 1: Java enters its optimizer on bm01 too); 181 CLOSED `c63fc9610` + `ea5bdbe4f` (BRANCH A: corner-count off-by-one, a violated Java rule; bm06 1/8/985.24 beats Java 2/8/971.63, bm11 at Java's exact 3/0/975.00); 197 PARTIAL `f842c3ea2` (semantics PARITY; wall target honestly missed; successor lever unchartered); flip **NO-GO held** `caa3115fa` (third consecutive hold; both preconditions fail); threads-gate wiring re-deferred WITH CAUSE | `logs/M6-T1/report-t1.md` (the full T1 dossier; the plan AMENDMENT 1 record + buglog 196 carry the durable face); `logs/M6-T2/report-t2.md`; `logs/M6-T3/report-t3.md`; the M6 hold-posture table above |
| 5 — zero regressions at default | **MET** — T10 gate lap 13/13 at the unchanged engine tree: seven compares green (corpus 5000/5000; dsn 1,332/0; ses 20; ses-snap 5; index 33; undo 33; drc 17), events 3,520 exit 0, determinism canaries IMMOVABLE/UNROTATED (ses `dc271114…` / manifest `cf607714…`) + settings-ON byte-stable at its own digests, census 1497/0/17 (additions only across the milestone), Tier A at its held face (10 green / 1 red; completion-only reading 11/11). The close-out re-ran the CHEAP set only; T10's lap at the same tree is the full evidence of record | `logs/M6-T10/report-t10.md` gate-lap table; the close-out's own gate logs `logs/M6-T11/evidence/` |

**Wall claims — the fixture-wall face (the two-face convention, named
once):** the battery-total Σ covers the per-fixture subprocess walls
PLUS the in-process detail-localizer re-routes on red non-killed
fixtures (`router_compare.rs`: subprocess wall `started :801` /
`wall_seconds :809`, loop total `started :1900` / `elapsed :2088`,
red-non-killed guard `:2159-2160`); cross-milestone
wall claims quote the fixture-wall face. Fixture-wall Σs: T3-B default
**2632.5 s = 3.10×** Σ-Java 850.4 s; B-ON **3041.0 s = 3.58×**; T3-C
default **3387.5 s = 3.36×** Σ-Java 1009.3 s; C-ON **5400.3 s =
5.35×**; Tier A **2075.7 s = 1.84×** Σ-Java 1130.39 s (A and C-ON
identical under both faces — A ran no detail passes, C-ON killed all
three). The 0.5× Σ goal stays far on every tier under either face;
ALL-ON worsens both B and C.

**Reopen bound — EXPLICIT form (bound = tier cap 1800.0 s − the
ten-wall strip):** canonical bound 1800.0 − 289.3 = **1510.7 s**
(T3-era strip); at T10 the measured bm01 wall was 1800.2 s (kill), the
this-run strip 275.5 s, this-run bound 1524.5 s — 1800.2 ≫ 1510.7, the
flip does NOT reopen, the bound does not rise.

**Pointer rows into the task dossiers:** criterion-1 before/after +
tuning → T3 (`logs/M6-T3/report-t3.md`) + T10
(`logs/M6-T10/report-t10.md`); the bm01 optimizer wall → buglog 197 +
T1b (the plan AMENDMENT 2 record; commit `f842c3ea2`); the 181 close →
T2 (`logs/M6-T2/report-t2.md`, commits `c63fc9610`/`ea5bdbe4f`); the
plane audit → T5 (`fa8654edb`); island detection → T6
(`d6a9d835f`) + the T8 clamp (`7a7d6e529`); the intelligence family →
T7 (`5d4199884`) + T8 (`7a7d6e529`) + T9 (`b670bbc28`), all
default-OFF with the byte-invariance contract (default settings
byte-identical outputs; settings-ON faces are NEW goldens); the flip
hold → the M6 hold-posture table above + the rust-check.yml
compare-step comment; bm10's ri=1 → buglog 205 (NEW, owner M7).

## M7 milestone-criterion map (appended by M7-T8; successor to the M6 map above — adjudication input)

The M6 map above stays exactly as landed (HISTORICAL), as do the M5
and M4 maps and all hold-posture tables — ZERO deletions above this
point. Criteria read the design §6 M7 row and the M7 plan's
"Milestone exit criteria" faces. Battery of record:
`logs/M7-T7/evidence/` (the tuning-population table, the Tier A
battery, and the 14/14 gate lap — all measured at `6165ac7aa`; the
engine tree is frozen at census 1547/0/17 across the close-out). The
verdict slot in the design doc §6 M7 exit note stays OPEN — this map
records the measured faces the adjudication reads.

| Criterion | Verdict (the measured face) | Owning surfaces + evidence |
|---|---|---|
| 1 — DSN-declared constraints honored (min / max / match-to-target) | **MET** — the read/deliver chain verified Java-exact (T1: the recon's dead-reader premise falsified; the three `ses_board.rs` zero-sites are ctor-parity); min honored by the tightener gate (whole-candidate rejection, landing-at-min allowed, inert on flag-off/no-min); meander insertion fills deficits with an honest stop; match-to-target with per-class goals (both-declared goal = min exactly; min-only target = the longest routed member); max-length = the honest report-only face (never truncated — `meander_blocked`'s −40 000 `length_report` row); demonstrated on mutation-verified crafted pins + the six committed tuning fixtures | DSN delivery: `epic-dsn/src/scope/network.rs` (`read_circuit_scope` / `read_length_scope` / `insert_net_class`); the settings seam `router.tuning.*` (+ `.meander` / `.pairs`) in `epic-cli/src/settings.rs`; the honoring gate `crates/epic-board/src/trace_tightener/mod.rs:1170`; the meander + match stage `crates/epic-router/src/pipeline/tuning.rs`; the pair face `crates/epic-router/src/pipeline/pairs.rs`; the T7 evidence: `logs/M7-T7/report-t7.md` §1 + the T1–T6 pinned dossiers |
| 2 — tuning DRC-clean on the fixture set | **MET** — ri = 0 on all six tuning fixtures at every `router_introduced_count` block (the REAL counter: `crates/epic-router/src/pipeline/board_statistics.rs:596-598` ← `all_clearance_violation_depths`, `crates/epic-drc/src/clearance.rs:557` — never the outline-only face); min_stair's total 1 is its own input pre-existing violation; both pair ON goldens re-verified green; bm10's ri=1 dispositioned parity-confirmed-noted (buglog 205 CLOSED — Java's own unchecked split re-landing; no fix without anti-parity) | the REAL-counter manifest face `crates/epic-router/src/pipeline/board_statistics.rs`; `logs/M7-T7/report-t7.md` §1 |
| 3 — zero regressions, default + constraint-free | **MET** — Tier A 11/11 held row-for-row (10 green / 1 red completion-only; Σ 2133.1 vs 2133.0 re-sums = print rounding; zero detail runs); the constraint-free-invariance rule pinned on the mixed world (`p3_undeclared_net_geometry_identical_on_vs_off`, `pipeline/pairs.rs:815`); canaries `dc271114…`/`cf607714…` UNROTATED all milestone; gate lap 14/14 | `logs/M7-T7/report-t7.md` §2 + §4 |
| 4 — inherited dispositions (205 / 210 / 201 / the B/C re-faces / 197) | **DISCHARGED AS RECORDED** — 205 CLOSED (parity-confirmed-noted, the T3 crafted split world + chain-complete source read); 210 CLOSED (outcome (i) Rust divergence; the Java-exact drain fix `a7b00639b` + the coordinator-sanctioned two-golden re-capture — the goldens pinned the defect's own bytes); 201 CLOSED (T4's harness touch: `--out`/`--golden` join the repo root); the B/C re-faces reduced to bm10 (ri=1 reproduced flag-independently, 1343.8 = 443.8 + 900.0 reconciled) + 1Bitsy (completion-only red 2>1, kill face NOT reproduced, 520.9 = 220.9 + 300.0); 197 stays OPEN (the successor lever — deterministic candidate parallelism — recorded NOT attempted, owner M8+) | `logs/M7-T7/report-t7.md` §3; the buglog sweep (M7-T8, this task) |
| 5 — gates green; census additions only; committed artifacts untouched | **MET** — census EXACTLY 1547/0/17; the M7 additions ledger, enumerated: the crafted-world pin sets (T1 four `network.rs` boundary worlds; T3 split world + honoring pins, mutation log 6/6; T4 W1–W8 + F1–F3; T5 M1–M7, mutation log 4/4; T6 P1–P6), the SIX new tuning fixtures (`rust/harness/fixtures/tuning/`), and the TWO new pair goldens — plus the two SANCTIONED t7_ripup re-captures (buglog 210, the milestone's only committed-baseline modifications, justified per the capture rule); seven compares + events 3,520 + canaries unrotated + the det-ON digest pair byte-identical | `logs/M7-T7/report-t7.md` §4 + §1; the T4/T5 sanctioned-capture dossiers |

**Pointer rows into the task dossiers:** the length-constraint
parity/verification → T1 (`6f6334e51`; AMENDMENT 1's audit ledger); the
constraint model + the 205 localization → T2 (`0626ff8a2`; AMENDMENT
2); the split-world disposition + the honoring gate → T3
(`b7b174859` + `349a192df`; AMENDMENT 3); the meander engine + the
harness banks (`--extra-router-arg`, buglog 201, the fanout-full-flow
golden, the Σ-decomposition print) → T4 (`514a4d5e0` + `c9fb800e4` +
`d87da94b7` + `c1478a31d`; AMENDMENT 4); the match face + the buglog-210
drain fix → T5 (`d6c7bd859` + `758356366` + `a7b00639b` +
`595049285`; AMENDMENT 5); the pair face → T6 (`36f4570c2` +
`bdfd39216` + `7e772fa2c`; AMENDMENT 6); the measurement moment → T7
(`logs/M7-T7/report-t7.md`, no commits; AMENDMENT 7); the Finding-1
label correction + the mixer fact (125 `(wire`) ride the design doc §6
M7 exit note; the close-out (this map, the exit note, the sweep) → T8.

## M8 milestone-criterion map (appended by M8-T9; successor to the M7 map above — adjudication input)

The M7 map above and every earlier map and hold-posture table stay
exactly as landed (HISTORICAL) — ZERO deletions above this point.
The map reads the design §6 M8 row (§6's table + the M8 exit note's
verdict slot) and the M8 plan's exit criteria
(`docs/superpowers/plans/2026-09-28-epicrouter-m8-gloss.md` :24-30);
evidence: `logs/M8-T2/report-t2.md` (the BEFORE table),
`logs/M8-T8/report-t8.md` §1–§5 (the AFTER battery, the ablations,
the re-faces, the gate lap, the flip record), the per-task dossiers
in AMENDMENTS 1–8. The verdict slot in the design doc §6 M8 exit note
stays OPEN — this map presents, the adjudicator decides.

| # | Criterion | Verdict on the record | Locations / evidence |
|---|---|---|---|
| 1 — aesthetics metrics improve monotonically vs PCBench ground truth | **PRESENTED AS MET-ON-ONE-METRIC / FINDINGS-ON-TWO — the verdict formulation is the adjudication's question.** Judgment table (T8 §1, 24 cells coordinator-re-derived exact): mean_length_excess 0.1807 → 0.4743 REGRESSION-FINDING; via_density 1.5891 → 1.3905 letter-PASS with the monotone-up intermediate VIOLATED (down); bend_to_length_ratio 17.8165 → 15.3972 letter-PASS, ARTIFACT-DOMINATED; parallelism_ratio 0.93875 → 0.9325 REGRESSION-FINDING. | the measurer `rust/crates/epic-board/src/aesthetics.rs` (`aesthetics_metrics` :238; the metric struct :91–:100; the window constant :83); the sidecar door `--dump-aesthetics` in `rust/crates/epic-cli/src/route.rs` (the rotation-trap comment); the java-free reference door (harness `aesthetics --dsn`); the committed sample `rust/harness/fixtures/aesthetics/sample.yaml` + 24 reference goldens; `logs/M8-T2/report-t2.md`; `logs/M8-T8/report-t8.md` §1 (the judgment + ablation tables) |
| 2 — no completion/DRC regression | **MET clean** — no-regression grid 20/20 (incompletes byte-equal ON vs default; ri = 0 everywhere, the REAL counter); Tier A held face 10 green / 1 red EXACTLY (the bm01 chronic cap-scrape; Σ 2096.1 s DNR-18-reconciled); eight compares green by name; events 3,520; determinism ×2 default (canaries `dc271114…`/`cf607714…` UNROTATED) + ×2 ALL-ON (ses `3ea73c82…` byte-stable); census 1629/0/17 EXACT ×3; the LMS6002-Pmod kill row is the pinned PERMANENT row (exit=124 recurs at ALL-ON — a kill, not a regression; n = 20 everywhere) | `logs/M8-T8/report-t8.md` §2 |
| 3 — M7-exit deviations dispositioned | **MET** — bm01/197: the charter decision made and measured (T7 wall 1340.78 s < the 1514.3 this-run / 1510.7 canonical bounds — the reopen OPENED; the lever `optimizer.threads` landed opt-in); the T8 refresh honest row-for-row: bm05 29>18 WORSE, bm10 completion 3>1 → tier-kill @900 s (FACE CHANGED, worse), interf_u 1>0 improved-but-red, mm/mm-u floors held byte-exact, StickHub/LimeSDR kills held, 1Bitsy 2>1 and bm04 3>2 held, CM5's kill did NOT recur (PASS 6≤6), gloss-ON row-for-row NO-OP on every B/C fixture both tiers; pour measurement gaps carried; all still-standing reds re-owned M9+ | `logs/M8-T7/report-t7.md`; `logs/M8-T8/report-t8.md` §3 |
| 4 — M8-opening chores | **DISCHARGED** — the README M7 status flip landed early (AM2); the `pairs.rs` :48–:50 module-doc swap fix landed at T3; the chore(wolf) bank lands as the coordinator's separate chore(wolf) commit immediately after the close-out commit (never folded) | the M8 gloss plan :22 (the no-fold rule); AM8's T9 banks + the T9 charter (the immediately-after timing); AM2/AM3 |
| 5 — gates green; census additions only; committed artifacts untouched | **MET** — census 1547 → 1629/0/17 additions-only across M8 (1562 → 1581/1584 → 1598/1599 → 1613 → 1622/1625 → 1629); committed artifacts untouched EXCEPT the sanctioned NEW captures — the aesthetics sample + 24 reference goldens + `select.py` (T1) and the ON-face goldens (T3 bus worlds, T4 flow worlds, T5 via worlds, T6 teardrop worlds); nothing pushed | AM1–AM7 census lines; the per-task reports |

**The gloss stage family + flags** (all default-OFF; the two-regime
byte-invariance contract: a DEFAULT run never reaches the family, ON
runs are the improvement regime, golden-captured at first landing and
pinned per pass): `router.gloss.bus` (T3 — the axis-aligned bus group
detector + the hug/spread pass), `router.gloss.flow` (T4 — the
POST-tightener jog/stub elimination + miter/recorner pass),
`router.gloss.via_place` (T5 — return-path-aware via relocation; via
COUNT never increases), `router.gloss.teardrops` (T6 — graded-width
wires at trace/pad junctions; the investigation chose the graded-wire
representation after BOTH polygon consumers proved to drop the shape).
The stage slots live in `pipeline/full.rs` (:440–:492), the flags in
`epic-cli/src/settings.rs`, and the sidecar blocks (`gloss_report`,
`gloss_flow`, `gloss_via_place`, `gloss_teardrops` + their `*_gated`
siblings) ride `--dump-aesthetics` — never the manifest.

**The T8 measurement verdicts table** is §1 of
`logs/M8-T8/report-t8.md` (the criterion-1 row above summarizes it;
the full per-metric/per-pass deltas live there — not duplicated
here).

**Pointer rows into the task dossiers:** the measurer + committed
sample → T1 (`25183bbfb`; AMENDMENT 1); the bus face → T3
(`eaa4791d6` + `ccf33107c`; AMENDMENT 3); the flow face → T4
(`4c622b4d5` + `48e4d84b6` + `c627758cc`; AMENDMENT 4); via_place →
T5 (`3d03600f5` + `e139bdbd3` + `0c83e2c91`; AMENDMENT 5); teardrops
→ T6 (`c15d5c33a` + `32d5cea69` + `a42f76b26`; AMENDMENT 6); the 197
lever → T7 (`e92b57cc9` + `b92b4affa` + `ccd85aa9d`; AMENDMENT 7);
the flip + the measurement record → T8 (`ba5e587bb` + `186c9eafd` +
`040d5dade`; AMENDMENT 8); the close-out → T9 (`23a8dba79` + the
fix-round commit that lands this row).

**The flip posture row** already landed in the M6 hold posture table
above (appended by M8-T8) — referenced, not duplicated.

**M9 (appended 2026-09-30 by the coordinator at close-out — the AM7
charter input "epic-router untouched across M9" was falsified at T8
verification; the append was correctly deferred by M9-T8 per its
zero-rust-files law):** exactly ONE commit touched this crate in M9 —
`3f20acafd` (M9-T3): the additive `DriverSink::board_snapshot(&mut
self, &Board)` default-no-op hook (event_sink.rs:68) + the
`DRIVER_SINK_METHOD_COUNT = 8` forwarding-exhaustion guard
(event_sink.rs:22), plus SIX mirror calls firing immediately after the
`board_updated` faces (batch.rs:125 via the fanout progress listener;
pass_runner.rs:564/:627 begin/finish pass; optimizer.rs:1158/:1518/
:1536). All additive and default-path-inert — CLI sinks never override
the hook, so no snapshot rows ever exist in captured/replayed streams
— and byte-invariance re-proven at M9-T7 (the 17/17 session-vs-CLI
workflow battery + the standing compares). Records: `logs/M9-T3/
report-t3.md` (AMENDMENT 3), `logs/M9-T8/report-t8.md` §4, and the
bracketed T8 erratum in the design §6 M9 exit note.

---

**M10-T1 (appended 2026-09-30, inside the task's commit — Opening rule
4):** the stop-flag WIRING — buglog 224's closure, the M9-adjudicated
carry. `run_optimization_stage` gains `parent_stop: &StopFace` (after
`full_stop`) and builds its stage face OVER the parent's shared flag —
`StopFace::from_flag(parent_stop.flag().cloned())` (full.rs, replacing
the T9-era flagless `StopFace::default()`; the `run()` call site passes
`&stop`) — so an EXTERNAL raise mid-optimization is visible to the
optimizer's pass-loop polls (optimizer.rs:1238-1242/:1458/:1738/:1802),
Java-faithful: `BatchOptimizer.java:384` reads the shared
`isStopRequested()` face via `job.thread`; the stage GATE
(`RoutingPipeline.java:122`) is unchanged. **Byte-invariance is
STRUCTURAL on the CLI path** (the CLI production face is flagless;
`from_flag(None)` is field-identical to the old `StopFace::default()`)
**and measured**: the full M9-T7 battery faces re-run at this commit —
Tier A + crafted + Tier B/C, byte-identity on every comparable row (the
two watchdog faces, bm01/bm10, are CLI_KILLED rows with no in-slice CLI
bytes — their session-side walls SHORTENED, the wiring's intended
effect: the 1800s/900s watchdog raises now terminate the optimization
stage), the events compare (3 fixtures / 3520 golden rows) + the bm08
canaries (ses `dc2711146276…`, manifest `cf6077141d2f…`) byte-identical.
**No new outcome face**: the probe found
`OptimizerOutcome { passes_completed, is_timed_out }` propagates no stop
state and Java propagates none either (`RoutingPipeline.java:121-134`
is void) — the pins witness the stop through the stage's own
`interrupted:` summary row (Java `BatchOptimizer.java:530`) + the pass
count, and the HOST re-reads the shared flag. **Pins** (DNR-16 both
directions witnessed in `logs/M10-T1/evidence/40-44-*`: the revert
mutant kills the raise pins — router + session — and NOT the control;
the over-wiring mutant kills the control and NOT the raise pins;
restores green):
`m10_t1_optimizer_stage_shares_the_parent_flag` +
`m10_t1_flagged_parent_without_a_raise_runs_clean` (full.rs tests) +
`mid_optimization_cancel_is_observed_by_the_optimizer_stage`
(epic-engine `session_workflow.rs`; the honest final state is
COMPLETED — a mid-optimization cancel lands after routing returned
`Ok(true)`, so the session mapping's `Ok(true) => COMPLETED` arm
governs; the state-name mapping is unchanged and Java-faithful —
`RoutingPipeline.run()` propagates nothing). Census 1682 → 1685/0/19
(declared before the run). Erratum to the plan's recon:
`StopFace::flag()` PRE-EXISTED (M3's `507eacc76`) — the recon's "has NO
public flag accessor" was stale; only the accessor's doc gained the
M10-T1 wiring-face note (batch.rs).
[Erratum 2026-09-30: the poll-site list above is off-by-four — the true
set is optimizer.rs:907/:1234/:1458/:1738/:1802 (:907 is the loop-head
poll the raise pins' kill rides on), verified by the T1 quality review
(logs/M10-T1/quality-review-t1-1.md, Q1); corrected in the live comment
at the T1 quality fix commit.]

**M10-T2 (appended 2026-09-30, inside the task's commit — Opening rule 4):
the Q7 `final_state_for` hoist + the Q6 re-route product decision.** The
duplicate-resolution row from M9-T2's deviations register is CLOSED: the
final-state mapping now exists ONCE, as `pub fn final_state_for(run_ok: bool,
stop_reason: Option<StopReason>) -> &'static str` in
`epic-router/src/pipeline/batch.rs`, directly beside `StopReason` (body
byte-verbatim from the two deleted copies — epic-cli `route.rs:447` public +
epic-engine `session.rs:878` private, whose doc recorded the
epic-engine-cannot-depend-on-epic-cli constraint, now RESOLVED by the hoist);
both hosts import it from `epic_router::pipeline::batch` (no new pin — a pure
import-move; the existing arm pins in both hosts exercise the hoisted body).
The Q6 probe (pre-change, `logs/M10-T2/evidence/10-q6-probe.log`) measured the
second `Session::route()` face as a SILENT `Ok` re-run (e1_ripup: two
COMPLETED runs, rev 651 twice, identical 1222-byte SES — no refusal, no
panic); the pre-made product decision landed as
`RouteError::AlreadyRouted { final_state }` — the guard fires BEFORE any
merge/pipeline work, re-route stays DISABLED, a re-route requires a fresh
`Session::load_dsn`. Pin `reroute_is_disabled_with_a_clean_error`
(epic-engine `session_workflow.rs`; DNR-16 both directions). Census
1685 + 1 = 1686/0/19 declared before the run. Citation hygiene: the task
re-pointed the session docs' pre-existing stale `route.rs` line cites (they
predate the M8-T4 sidecar re-layout and were already false at HEAD) to their
true locations at the committed tree, including the stale epic-board
`pins.rs` cite `:486-490` → `:905-919`.

**M10-T5 commit A′ (appended 2026-10-01, inside the task's commit — Opening
rule 4): comment-only canary-citation refresh; zero engine change.** The
coordinator's Option-B adjudication (`logs/M10-T5/report-t5.md`) made the
HARNESS manifest-digest face version-blind
(`baseline::normalized_manifest_sha256`: `app_version`/`git_sha` are
normalized out before hashing — a release bump can never rotate a committed
manifest digest; the manifest ARTIFACT keeps its true values). The raw-bytes
manifest canary `cf607714…` retired at A′; its successor — the normalized
literal measured there — is `c6644a910255b865a8c5fa7e903b8e98a5228d0129c29d741180749ce8121f4d`;
the ses canary `dc271114…` stayed immovable throughout. This commit's
epic-router touches are COMMENT-ONLY: the ten live
"the canary cf607714 pins manifest bytes" citations in `pipeline/gloss.rs`
(4) and `pipeline/full.rs` (1) — plus their epic-cli siblings in
`route.rs` (5) — now cite the version-blind canary face. No behavior, no
signature, no pin change in either engine crate.

**M10-T6 (appended 2026-10-01, inside the task's commit — Opening rule 4):
the codespell/rustfmt marker hygiene in the T11 span-gate test
(`path/inserter.rs`).** TEST-ONLY, zero engine change: the test's same-line
`codespell:ignore` marker previously rode the multi-line `.is_some_and`
chain, so a `#[rustfmt::skip]` was load-bearing (rustfmt reflows trailing
comments off chain lines, detaching the marker from the flagged word — and
a standalone marker line does NOT engage, probed, `logs/M10-T6/evidence/05`).
The test now binds the padstack name to a `let` and carries the marker on a
single-line let statement — rustfmt-stable and codespell-green with the skip
attribute DROPPED. Semantics unchanged (the same first-match 1..=32
padstack-name scan; the verdict + id-burn + row asserts untouched). Probe
rounds A–D logged (`logs/M10-T6/evidence/05-n3-probe.log`; red rounds B/C
preserved).
