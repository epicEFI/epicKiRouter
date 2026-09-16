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
| `RoutingBoard.checkTraceSegment` (2 overloads) | `RoutingBoard.java:198-237` | 40 | NEW port — the per-segment legality probe the maze shover and `FoundConnectionInserter` call (T4-T9); the facade overloads delegate to `RoutingBoardOperations.checkTraceSegment` (`:207`/`:231`), port with that consumer |
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
| `BasicBoard.getMinTraceHalfWidth` | `BasicBoard.java:1124-1128` | 5 | already-ported — `BoardRules::get_min_trace_half_width` (M2; the reader's pin-edge fallback consumes it) |
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
| `ShapeSearchTree.completeShape` | `ShapeSearchTree.java:580-700` | 121 | NEW port — enlarge a room to a maximal free shape; port the 45-degree and any-angle arms (the brief's scope), `restrainShape` `:701+` is its private helper |
| `SortedRoomNeighbours` (whole) | `autoroute/expansion/SortedRoomNeighbours.java` | 807 | NEW port — the room-neighbour dispatch the maze search consumes: `complete` `:65`, `calculate` `:95`, `calculateNewIncompleteRooms` `:510`, record `SortedRoomNeighbour` `:670-698` |

Note: `SortedRoomNeighbours` lives in `autoroute/expansion/` but is listed here
because it is the SEARCHTREE-side half of the maze seam — it belongs with the
`completeShape` port, not with the pass driver.

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
