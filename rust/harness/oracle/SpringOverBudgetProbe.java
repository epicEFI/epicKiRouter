// SpringOverBudgetProbe.java — M3-T10b quality-review fix round: the
// jar-side oracle for the SPRING-OVER BUDGET SCOPE of the insert piece
// loop (TraceShover.java:513-541). Java reads maxSpringOverRecursion-
// Depth into a mutable method-parameter local; a successful spring-over
// decrements it (:539) for the REST of the invocation — the NEXT piece
// in the loop sees the decremented value both at its own gate (:521)
// and in the recursive insert calls (:556). At 0 the later piece's
// spring-over is SKIPPED and its STRAIGHT segments are probed instead.
// The port re-armed the budget per piece (trace_shover.rs, the Q1
// mutant face); this probe captures Java's per-invocation verdicts.
//
// Build/run (JDK 25, from the repo root):
//   mkdir -p /tmp/epic-t10b-fix-classes && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/javac \
//       -cp build/libs/freerouting-current-executable.jar \
//       -d /tmp/epic-t10b-fix-classes rust/harness/oracle/SpringOverBudgetProbe.java && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/java \
//       -cp build/libs/freerouting-current-executable.jar:/tmp/epic-t10b-fix-classes \
//       app.freerouting.board.optimize.SpringOverBudgetProbe \
//       rust/harness/fixtures/locator-spike/t9_locator45.dsn
//
// World (all coordinates absolute; the t9_locator45 bbox center is
// (500000, 300000), so the rig sits ~190000 west of the fixture's own
// items — ObstacleAreas 2-4 live at x[480000,520000], the pins are far,
// and a bbox dump of every fixture item confirms the rig band
// x[308300,311500] x y[298500,301400] is otherwise empty):
//
//   S      = the BOUNDING OCTAGON of IntBox [309000, 299000, 311000,
//            301000] (probe shape; 45-degree mode). OCTAGON, not
//            IntBox, DELIBERATELY: the insert recursion probes each
//            piece segment with shape_and_entry_side_core, and an
//            IntBox probe shape sets is_orthogonal_mode — every
//            segment probe becomes the segment's BOUNDING BOX. The
//            spring-over wrap of a via hugs the via +- 67 OCTAGON,
//            whose 45-degree corner-cut segments have bounding boxes
//            that overlap the via's box corner (~23 units) even though
//            the true octagon probes are 17 clear — an IntBox probe
//            shape false-fails the wrapped corner segments. The
//            autorouter's own probe shapes in 45-degree mode are
//            octagons (shape_and_entry_side_core tile shapes), so the
//            octagon face is the production-realistic one.
//   X      = N002 trace hw 50, 45-degree diagonal (310500, 298600) ->
//            (311400, 299500) (UNFIXED; runs NE, crosses S's BOTTOM
//            then EAST edge)
//   Y      = N003 trace hw 50, 45-degree diagonal (309500, 301300) ->
//            (308400, 300200) (UNFIXED; runs NW, crosses S's TOP then
//            WEST edge)
//   VX     = N001 via pad 100, USER_FIXED, at (311150, 298980)
//   VY     = N001 via pad 100, USER_FIXED, at (308850, 301100)
//
// TWO-PIECE design notes (the hard-won part — the first three capture
// rounds proved the failure modes):
//   - cutoutTraces (:511) detours every severed substitute piece around
//     the probe shape's offset boundary S +- 67 (67 = hw 50 + 1 +
//     cl 16, two-step enlarge) — with the octagon S this is the offset
//     OCTAGON, whose lower-right / upper-left corners carry 45-degree
//     cuts — COUNTER CLOCK WISE from the piece's first boundary
//     crossing to its last. The detour ARC is the CCW border walk
//     between the two crossing sides: X (bottom -> east) holds the
//     SHORT bottom-right arc (captured corners [310833,298933,
//     311028,298933] [311067,298972] [311067,299167]), Y (top -> west)
//     the SHORT top-left arc ([309267,301067] [308972,301067]
//     [308933,301028] [308933,300733]).
//   - The arcs stay SEPARATE pieces only because X and Y are on
//     DIFFERENT nets: ShapeTraceEntries.resort (:519-552) prunes all
//     MIDDLE entry points of a same-net run to its first and last, so
//     two same-net crossings collapse into ONE combined circuit
//     (pieceCount 1 — capture round 3 proved it with opposite-orientation
//     horizontals, rounds 1-2 with same-orientation verticals at 4-unit
//     and 119-unit spacings). Different nets skip the pruning, and the
//     ring order of the four entries over the octagon's border lines is
//     X-in(0 bottom), X-out(2 east), Y-in(4 top), Y-out(6 west) —
//     arc-closed-before-arc-opens, so calculateStackLevels keeps BOTH
//     at stack level 1 (maxStackLevel 1: the drivers' stackDepth gate
//     stays green) and substituteTraceCount() == 2. fromSide sits on
//     the bottom side west of X-in, so the resort break fires at the
//     list head and X pops FIRST.
//   - VX.box = [311050, 298880, 311250, 299080] sits in X's arc corner
//     corridor; VY.box = [308750, 301000, 308950, 301200] in Y's. BOTH
//     pieces therefore need their spring-over (wrap the via) for the
//     recursive segment probes to pass.
//   - The TOP-LEVEL store never sees the vias: the octagon S enlarged
//     by the query-reach acceptance (cl/2 = 8) stays inside the box
//     x[308992, 311008] x y[298992, 301008] — VX (x from 311050) and
//     VY (x to 308950) are both x-disjoint — insert's piece loop runs
//     with both circuits and no obstacle (pieces-store row: obstacle
//     ids [106, 105], descending).
//   - The spring-over wrap hugs the via +- 67 OCTAGON: the wrapped legs
//     move to the offset boundary and stay 17 clear of the via box in
//     true-octagon probe space — the wrapped corner-cut diagonals'
//     probe shapes (segment tree shapes, exact octagons per
//     IntOctagon.toSimplex) clear the via, while their BOUNDING BOXES
//     overlap the via's box corner (~23 units): the IntBox face
//     false-fails exactly here (see the S paragraph).
//
// Runs — one FRESH parse + world per run (insert mutates the board):
//   insert_b0  spring budget 0: both gates skip; X pops first and its
//              straight-arc probe hits VX      -> FALSE (failing VX)
//   insert_b1  spring budget 1: X (first piece) wraps VX (1 -> 0); Y's
//              gate then SKIPS (per-invocation scope), Y's straight-
//              arc probe hits VY              -> FALSE (failing VY;
//              THE pin verdict — the per-piece re-arm bug would hand Y
//              a fresh budget 1, wrap VY too -> TRUE)
//   insert_b2  spring budget 2: both pieces wrap with a budget left
//                                           -> TRUE (rig validity)
//   pieces-store replicates insert's store + cutout and dumps BOTH
//              real pieces (corners + compensated half width + a
//              reflective springOver at depth 1: both must report
//              "changed" — each piece wraps its own via at depth 1).
//
// Determinism: verdict + failing-obstacle + id-sorted inventory rows
// only; doubles via Double.toString; no identity-hash surfaces (no
// log4j tap — the verdicts are the pin literals).
package app.freerouting.board.optimize;

import app.freerouting.board.actions.ItemIdGenerator;
import app.freerouting.board.facade.BasicBoard;
import app.freerouting.board.facade.RoutingBoard;
import app.freerouting.board.model.items.Item;
import app.freerouting.board.model.items.Via;
import app.freerouting.board.model.structure.FixedState;
import app.freerouting.board.model.structure.ShapeEntrySide;
import app.freerouting.board.trace.PolylineTrace;
import app.freerouting.core.library.Padstack;
import app.freerouting.geometry.planar.FloatPoint;
import app.freerouting.geometry.planar.IntBox;
import app.freerouting.geometry.planar.IntPoint;
import app.freerouting.geometry.planar.Polyline;
import app.freerouting.geometry.planar.TileShape;
import app.freerouting.io.BoardReadResult;
import app.freerouting.io.specctra.DsnReader;
import com.google.gson.Gson;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import java.io.ByteArrayInputStream;
import java.nio.file.Files;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.LinkedList;
import java.util.List;

public class SpringOverBudgetProbe {

  private static final Gson GSON = new Gson();

  // absolute rig coordinates (see the header derivation)
  private static final int S_LL_X = 309000;
  private static final int S_LL_Y = 299000;
  private static final int S_UR_X = 311000;
  private static final int S_UR_Y = 301000;
  private static final int X_X0 = 310500;
  private static final int X_Y0 = 298600;
  private static final int X_X1 = 311400;
  private static final int X_Y1 = 299500;
  private static final int Y_X0 = 309500;
  private static final int Y_Y0 = 301300;
  private static final int Y_X1 = 308400;
  private static final int Y_Y1 = 300200;
  private static final int TRACE_HW = 50;
  private static final int VX_X = 311150;
  private static final int VX_Y = 298980;
  private static final int VY_X = 308850;
  private static final int VY_Y = 301100;
  private static final int VIA_PAD = 100;
  // the fromSide point: the interior of the probe shape's BOTTOM side
  // (side 0), west of X's bottom crossing
  private static final int FROM_X = 310000;
  private static final int FROM_Y = 299000;

  /** The probe shape: the bounding octagon of the rig box (45-degree mode). */
  private static TileShape probeShape() {
    return new IntBox(new IntPoint(S_LL_X, S_LL_Y), new IntPoint(S_UR_X, S_UR_Y))
        .boundingOctagon();
  }

  private static ShapeEntrySide probeFromSide(TileShape shape) {
    return new ShapeEntrySide(new IntPoint(FROM_X, FROM_Y), shape);
  }

  private static synchronized void row(JsonObject o) {
    System.out.println(GSON.toJson(o));
  }

  private static JsonObject obj(String type) {
    JsonObject o = new JsonObject();
    o.addProperty("type", type);
    return o;
  }

  /** A fresh parse of the fixture bytes. */
  private static RoutingBoard parse(byte[] bytes, String fileName) throws Exception {
    BoardReadResult read =
        DsnReader.readBoard(
            new ByteArrayInputStream(bytes), null, new ItemIdGenerator(), fileName);
    if (!(read instanceof BoardReadResult.Success success)) {
      throw new IllegalStateException("parse failed");
    }
    BasicBoard basicBoard = success.board();
    RoutingBoard board = (RoutingBoard) basicBoard;
    board.searchTreeManager.reinsertTreeItems();
    return board;
  }

  private static final class World {
    final int traceX;
    final int traceY;
    final int vx;
    final int vy;

    World(int traceX, int traceY, int vx, int vy) {
      this.traceX = traceX;
      this.traceY = traceY;
      this.vx = vx;
      this.vy = vy;
    }
  }

  private static World buildWorld(RoutingBoard board) {
    int ownNet = board.rules.nets.get("N001", 1).netNumber;
    int netX = board.rules.nets.get("N002", 1).netNumber;
    int netY = board.rules.nets.get("N003", 1).netNumber;
    int layerCount = board.layerStructure.layers.length;
    // X: NE diagonal crossing S's bottom then east edge (short
    // bottom-right CCW detour arc)
    PolylineTrace traceX =
        board.insertTraceWithoutCleaning(
            new Polyline(new IntPoint(X_X0, X_Y0), new IntPoint(X_X1, X_Y1)),
            0,
            TRACE_HW,
            new int[] {netX},
            0,
            FixedState.UNFIXED);
    // Y: NW diagonal crossing S's top then west edge (short top-left
    // CCW detour arc); REVERSED direction puts its entries ring-adjacent
    PolylineTrace traceY =
        board.insertTraceWithoutCleaning(
            new Polyline(new IntPoint(Y_X0, Y_Y0), new IntPoint(Y_X1, Y_Y1)),
            0,
            TRACE_HW,
            new int[] {netY},
            0,
            FixedState.UNFIXED);
    Padstack viaPadstack =
        board.library.padstacks.add(
            new IntBox(
                new IntPoint(-VIA_PAD, -VIA_PAD), new IntPoint(VIA_PAD, VIA_PAD)),
            0,
            layerCount - 1);
    Via vx =
        board.insertVia(
            viaPadstack,
            new IntPoint(VX_X, VX_Y),
            new int[] {ownNet},
            0,
            FixedState.USER_FIXED,
            false);
    Via vy =
        board.insertVia(
            viaPadstack,
            new IntPoint(VY_X, VY_Y),
            new int[] {ownNet},
            0,
            FixedState.USER_FIXED,
            false);
    return new World(traceX.getId(), traceY.getId(), vx.getId(), vy.getId());
  }

  private static String shapeNameOf(TileShape shape) {
    IntBox bb = shape.boundingBox();
    return bb.ll.x + ":" + bb.ll.y + ":" + bb.ur.x + ":" + bb.ur.y;
  }

  /** Item inventory: traces (corners) + vias (centers), id-sorted. */
  private static void dumpInventory(String phase, RoutingBoard board) {
    JsonArray items = new JsonArray();
    List<Item> sorted = new ArrayList<>(board.getItems());
    sorted.sort(Comparator.comparingInt(Item::getId));
    for (Item item : sorted) {
      JsonObject it = new JsonObject();
      it.addProperty("id", item.getId());
      it.addProperty("kind", item.getClass().getSimpleName());
      it.addProperty("onBoard", item.isOnTheBoard());
      it.addProperty("nets", java.util.Arrays.toString(item.netNumbers));
      if (item instanceof PolylineTrace trace) {
        JsonArray corners = new JsonArray();
        for (int i = 0; i < trace.polyline().lines.length - 1; i++) {
          FloatPoint c = trace.polyline().cornerApprox(i);
          JsonArray pt = new JsonArray();
          pt.add(Double.toString(c.x));
          pt.add(Double.toString(c.y));
          corners.add(pt);
        }
        it.add("corners", corners);
      } else if (item instanceof Via via) {
        FloatPoint c = via.getCenter().toFloat();
        JsonArray pt = new JsonArray();
        pt.add(Double.toString(c.x));
        pt.add(Double.toString(c.y));
        it.add("center", pt);
      }
      items.add(it);
    }
    JsonObject out = obj("inventory");
    out.addProperty("phase", phase);
    out.add("items", items);
    row(out);
  }

  private static void shovingStateRow(String phase, RoutingBoard board) {
    JsonObject o = obj("shoving-state");
    o.addProperty("phase", phase);
    o.addProperty("failingLayer", board.getShoveFailingLayer());
    Item obstacle = board.getShoveFailingObstacle();
    o.addProperty("failingObstacleId", obstacle == null ? -1 : obstacle.getId());
    row(o);
  }

  /** One insert run on a fresh board+world; emits verdict + state. */
  private static void runBudget(byte[] bytes, String fileName, String run, int springDepth)
      throws Exception {
    RoutingBoard board = parse(bytes, fileName);
    World world = buildWorld(board);

    JsonObject worldRow = obj("world");
    worldRow.addProperty("run", run);
    worldRow.addProperty("springDepth", springDepth);
    worldRow.addProperty("traceX", world.traceX);
    worldRow.addProperty("traceY", world.traceY);
    worldRow.addProperty("vx", world.vx);
    worldRow.addProperty("vy", world.vy);
    row(worldRow);
    dumpInventory(run + ":before", board);

    TileShape shape = probeShape();
    ShapeEntrySide fromSide = probeFromSide(shape);
    int ownNet = board.rules.nets.get("N001", 1).netNumber;
    TraceShover shover = new TraceShover(board);
    boolean verdict =
        shover.insert(
            shape,
            fromSide,
            0,
            new int[] {ownNet},
            0,
            new LinkedList<>(),
            10,
            10,
            springDepth);
    JsonObject verdictRow = obj("verdict");
    verdictRow.addProperty("run", run);
    verdictRow.addProperty("result", verdict);
    verdictRow.addProperty("shape", shapeNameOf(shape));
    verdictRow.addProperty("shapeKind", shape.getClass().getSimpleName());
    verdictRow.addProperty("fromSideNo", fromSide.no);
    verdictRow.addProperty("springDepth", springDepth);
    row(verdictRow);
    shovingStateRow(run + ":after", board);
    dumpInventory(run + ":after", board);
  }

  /**
   * Replicates insert's store + cutout EXACTLY (same query, same
   * storeItems flags) and dumps the REAL substitute pieces — polyline
   * corners + compensated half width — then reflects springOver on
   * EACH piece at depth 1: both must wrap their via ("changed").
   */
  private static void runPieces(byte[] bytes, String fileName) throws Exception {
    RoutingBoard board = parse(bytes, fileName);
    buildWorld(board);
    int ownNet = board.rules.nets.get("N001", 1).netNumber;
    TileShape shape = probeShape();
    ShapeEntrySide fromSide = probeFromSide(shape);
    app.freerouting.board.searchtree.ShapeSearchTree searchTree =
        board.searchTreeManager.getDefaultTree();
    java.util.Collection<Item> obstacles =
        searchTree.overlappingItemsWithClearance(shape, 0, new int[0], 0);
    app.freerouting.board.searchtree.ShapeTraceEntries entries =
        new app.freerouting.board.searchtree.ShapeTraceEntries(
            shape, 0, new int[] {ownNet}, 0, fromSide, board);
    JsonObject storeRow = obj("pieces-store");
    storeRow.addProperty("obstacleIds", java.util.Arrays.toString(
        obstacles.stream().mapToInt(Item::getId).toArray()));
    storeRow.addProperty(
        "shovable", entries.storeItems(obstacles, false, true));
    storeRow.addProperty("pieceCount", entries.substituteTraceCount());
    storeRow.addProperty("stackDepth", entries.stackDepth());
    row(storeRow);
    entries.cutoutTraces(obstacles);
    java.lang.reflect.Method springOver =
        TraceShover.class.getDeclaredMethod(
            "springOver",
            Polyline.class,
            int.class,
            int.class,
            int[].class,
            int.class,
            boolean.class,
            int.class,
            java.util.Set.class);
    springOver.setAccessible(true);
    TraceShover shover = new TraceShover(board);
    PolylineTrace piece;
    int index = 0;
    while ((piece = entries.nextSubstituteTracePiece()) != null) {
      JsonObject p = obj("piece");
      p.addProperty("index", index);
      p.addProperty("id", piece.getId());
      p.addProperty("nets", java.util.Arrays.toString(piece.netNumbers));
      p.addProperty(
          "compensatedHalfWidth", piece.getCompensatedHalfWidth(searchTree));
      JsonArray corners = new JsonArray();
      for (int i = 0; i < piece.polyline().lines.length - 1; i++) {
        FloatPoint c = piece.polyline().cornerApprox(i);
        JsonArray pt = new JsonArray();
        pt.add(Double.toString(c.x));
        pt.add(Double.toString(c.y));
        corners.add(pt);
      }
      p.add("corners", corners);
      Object springResult =
          springOver.invoke(
              shover,
              piece.polyline(),
              piece.getCompensatedHalfWidth(searchTree),
              0,
              piece.netNumbers,
              piece.clearanceClassIndex(),
              false,
              1,
              null);
      p.addProperty("springAtDepth1", springResult == null ? "null" : "changed");
      if (springResult == null) {
        Item obstacle = board.getShoveFailingObstacle();
        p.addProperty("failingObstacleId", obstacle == null ? -1 : obstacle.getId());
      } else {
        JsonArray wrapped = new JsonArray();
        Polyline wrappedPolyline = (Polyline) springResult;
        for (int i = 0; i < wrappedPolyline.lines.length - 1; i++) {
          FloatPoint c = wrappedPolyline.cornerApprox(i);
          JsonArray pt = new JsonArray();
          pt.add(Double.toString(c.x));
          pt.add(Double.toString(c.y));
          wrapped.add(pt);
        }
        p.add("wrappedCorners", wrapped);
      }
      row(p);
      index++;
    }
  }

  public static void main(String[] args) throws Exception {
    byte[] bytes = Files.readAllBytes(Paths.get(args[0]));
    String fileName = args.length > 1 ? args[1] : "t9_locator45.dsn";
    runBudget(bytes, fileName, "insert_b0", 0);
    runBudget(bytes, fileName, "insert_b1", 1);
    runBudget(bytes, fileName, "insert_b2", 2);
    runPieces(bytes, fileName);
  }
}
