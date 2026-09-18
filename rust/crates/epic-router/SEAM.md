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
| `ViaOptimizer` | `ViaOptimizer.java` | 733 | skip — optimizer-stage pass (post-detail-routing gloss), not the T4-T10 seam |

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

Ported: `FoundConnectionLocator.java` (660 lines) → `path/locator.rs`; `FoundConnectionLocator45Degree.java` (356 lines) → `path/locator_45.rs`; `FoundConnectionLocatorAnyAngle.java` (~450 lines) → `path/locator_any.rs`. The Java abstract-base + two-subclass family is modeled as the [`Synthesis`] enum (FortyFive serves BOTH the ninety- and fortyfive-degree restrictions, per Java `getInstance :196-200`; the restriction itself flows into `calculateAdditionalCorner`) with free functions over a `LocatorState` struct carrying Java's protected fields.

| Seam | Status | Contract / rationale |
|---|---|---|
| Key→door resolution (D17) | CLOSED via `maze::locator_access::LocatorAccess` | Java reads LIVE room/door objects off the maze elements (`backtrackDoor.otherRoom(...)`, `drill.roomArr[...]`, door shapes); the port resolves opaque door keys through the engine's backtrack REGISTRY (every door of a finished chain is registered when its section state is written, which precedes every backtrack read — invariant pinned) + room reads through `NeighbourEngine`. `expandable_object_shape` re-types room-door shapes via `ExpansionDoor.shape_between(first, second)`. |
| `emitDiagnostics` (`:502-516`) | NOT ported | No Rust diagnostic sink (same disposition as the drill `emitDiagnostics`, drill/mod.rs deviations). |
| `rippedItemList` ordering deviation | documented | Java `SortedSet<Item>` with DESCENDING id order (`Item.compareTo = other.id - id`); port carries `BTreeMap<i32, u64>` ASCENDING — the T10/T11 consumer MUST iterate `.rev()`. |
| `ripupCosts` nullability | narrowed | Java's map parameter is nullable; every engine call site passes non-null → port takes `&mut HashMap`. |
| Tree-variant-per-restriction contract | DOCUMENTED (bug-137) | Java `SearchTreeManager.getAutorouteTree` (SearchTreeManager.java:140-158) selects the tree variant from the CURRENT board restriction AT ENGINE-CREATION time. Never set restrictions after building the search structures; the harness `build_with_restriction` applies the restriction BEFORE tree creation. The restraints in room completion ARE the tree shapes. |
| 90-degree room completion | PORTED in epic-index | `complete_shape_ninety_degree` + `restrain_shape_ninety_degree` (ShapeSearchTree90Degree.java:39-320) landed for the A90 pin; dispatcher arm by `SearchTree::variant`. Unconditional seed room, dynamic `boundingShape` prune (union accumulation is result-identical to Java's O(n²) re-scan by min/max idempotence), box-typed 4-directional restrain with per-arm assignment order, no divideLargeRoom. |
| A90 pin | DE-SCOPED `#[ignore]` | The A90 path then needs `SortedOrthogonalRoomNeighbours` (728 lines, orthogonal neighbour sorter) — an expansion-engine (T4-scope) component, NOT locator surface. Capture banked `logs/M3-T9/captures/t9_locator45_capture.rows`. |
| contains(FloatPoint) virtual dispatch | FIXED in epic-geometry (bug-135) | Java `IntOctagon` OVERRIDES the ONE-ARG `contains(FloatPoint)` (IntOctagon.java:332-344) with border-INCLUDED `<=` arithmetic; IntBox/Simplex fall through to the strict two-arg loop. `TileShape::contains_float` now dispatches per variant; `contains_float_tolerance` (the two-arg) stays strict — no octagon override exists for it. P14/P14B pins capture both (jshell: oct one-arg (0,5)=true, box one-arg (0,5)=false, oct two-arg (0,5)=false). GLOBAL parity fix — every contains_float caller shares the dispatch. |
| Via worlds (A45V/BANYV/BBT/CBT pins) | DE-SCOPED `#[ignore]` | Harness stub `LegalChecker` always answers Drillable; Java runs the STATIC `ForcedViaInserter.checkLayer` (ForcedViaInserter.java:30) whose real port pulls `ForcedPadRouter` (T10 shove machinery). A capture-forced lookup-table checker is feasible (the board is static during a phase) — banked as the follow-up. CBT capture shows Java detouring to layer 1 (pop 422, 24 ripped-queued) where the stub goes direct (pop 154). |
| CBTR/CBTR2/CBTR10 pins | DE-SCOPED `#[ignore]` | Ripup-world search-order divergence (Java pop 11 vs Rust 72): points at ripup-aware collision-cost/destroy handling in the SEARCH, not the locator. Seeds 2/10 are byte-identical to seed 1000 in Java, so they add no independent discrimination until CBTR correlates. Captures banked `logs/M3-T9/captures/t9_ripup_capture.rows`. |
| BANY pin | DE-SCOPED `#[ignore]` | Pop 29 (Rust) vs 36 (Java) divergence INVARIANT under tree variant (byte-identical failure under FORTYFIVE- and GENERIC-variant trees) — not tree-shape related; root cause unreached in budget. Capture banked `logs/M3-T9/captures/t9_locator_any_capture.rows`. |
| Mutation verification | 4 killed / 5 survived-documented / 3 blocked | KILLED: factory dispatch (3 pins), round-dedup (3 pins), empty-shrink fallback gate (A45+ABT), fortyfive `abs_dx <= abs_dy` boundary (unit pin). SURVIVED (traced to fixture degeneracy/coincidence, code matches Java): fanout adjust gate (AFAN adjust returns None), obstacle-vs-freespace shrink offset (no obstacle rooms on green chains), adjust horizontal-first flag (degenerate 2-unit vertical move rounds identically), nearest-border `>=` boundary (no exact ties), FromDoor/ToDoor comparator swap (verdicts coincide on green chains). BLOCKED by de-scoped pins: ripped-step harvest + no-rip arm (CBTR/CBT), any-angle index_of left/right (BANY/BBT). |
| Comparator equivalence finding | documented in pin | Inside `fortyfiveDegreeCorner`, the inner `to.y >= from.y` / `to.x > from.x` strictness is NOT separately observable (ties unreachable or ±0-coincident); the ONLY observable boundary is `abs_dx <= abs_dy`. Pinned + doc'd in `fortyfive_degree_corner_comparator_pins`. |
| T10/T11 required input | `connection_items` hook | The locator returns `FoundConnectionLocator { connection_items: Vec<ResultItem{corners, layer}>, start_item, start_layer, target_item, target_layer }` — the T10 (ForcedPadRouter/inserter) + T11 (connection assembly) consumer surface. ResultItems carry ROUNDED IntPoint corners, Java `:450-459`. |
