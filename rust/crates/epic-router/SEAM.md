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
| Distance slot (T8) | injected trait | `drill::DestinationDistance::calculate` + `join` — the T6 pins replay the REAL `DestinationDistance` as a test-side mirror (`TargetDistance`, Java `DestinationDistance.java:45-379`); T8 owns the production module and can absorb the mirror. The targetless arm (`2147483647.0`) stays the T5 stub value. |
| `pin_nearest_trace_exit_corner` | REAL port on the harness seam; production owes it | `MazeExpansionEngine.expandToDrill:56-65` → `Pin.nearestTraceExitCorner` (`Pin.java:636-668`). The harness (`drill/pins.rs`) carries the full port (exit-restriction directions, padstack shape gate, rotation, offset ray/border-line intersection); the production live-board `DrillEngine` impl lands with T7/T8. Provenance oracle `rust/harness/oracle/MazeExitProbe.java`: the T6 pin pad is the parse-DILATED `IntBox [180000,280000,220000,320000]`, `pinEdgeToTurnDist` = 100000.0, the RIGHT exit corner (322750, 300000) dominates the DRILL k=3 cost (exp 29573.69021909981, sort 458472.6902190998). |
| `ExpansionDoor.allocateSections` | ported, call-site boundary documented | Contract `:193-201`: same-count re-allocation is idempotent, different count RESETS the section state. The T6 capture contains NO re-segmentation event, so the drain pins cannot discriminate the reset arm (cerebrum mode 10) — closed by the direct contract pin `allocate_sections_resegmentation_contract` + two body mutants (always-reset / reset-disabled, both killed). |
| Completion seam | production, probe-pinned | `DrillEngine::complete_expansion_room` trait default delegates to `maze::completion::complete_null_shape_room`; burn-count + room-shape rows verified against the T5 probe pins (12 DrillSpikeProbe rows). |
