// ForcedPadFrontProbe.java — the M3-T10b FIX-ROUND capture probe for
// the inFrontOfPad min/max-of-sums terms (spec review MAJOR-2) and the
// ForcedPadRouter -> TraceShover.check depth-1 edge (review MINOR-2).
// ONE JVM per run, deterministic JSONL rows on stdout; the consumer
// drops the jar noise (lines not starting with "{").
//
// Build/run (JDK 25, from the repo root):
//   mkdir -p /tmp/epic-t10b-fix-classes && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/javac \
//       -cp build/libs/freerouting-current-executable.jar \
//       -d /tmp/epic-t10b-fix-classes rust/harness/oracle/ForcedPadFrontProbe.java && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/java \
//       -cp build/libs/freerouting-current-executable.jar:/tmp/epic-t10b-fix-classes \
//       app.freerouting.board.actions.ForcedPadFrontProbe \
//       rust/harness/fixtures/locator-spike/t9_locator45.dsn
//
// Part 1 — the inFrontOfPad table (reflection: the method is private
// static). Pad = bounding octagon of IntBox(0,0,100,100): top/bottom
// 100/0, left/right 0/100, upperRightDiagonalX = 200 (x+y), lowerLeft
// = 0, lowerRight = 0 (x-y), upperLeft = -100. diagWidth = w*sqrt(2).
// Every line below is a (1,-1)-slope segment (a 45-degree Line), the
// direction where min(a+c, b+d) STRICTLY exceeds
// min(a,b)+min(c,d) and max(a+c, b+d) STRICTLY undershoots
// max(a,b)+max(c,d):
//   main6_hit      a=(0,210)   b=(210,0)   side 6 w=4  wS=false
//                  min-of-sums 210 >= 200+4*sqrt2 = 205.657 -> TRUE;
//                  sum-of-mins 0+0 = 0 -> FALSE (the swapped form).
//   main6_miss     a=(0,180)   b=(180,0)   side 6 w=4  wS=false
//                  min-of-sums 180 < 205.657 -> FALSE (both forms).
//   ws6_trap       a=(0,220)   b=(100,120) side 6 w=20 wS=true
//                  main misses; wS conj 2 partner min(y)=120 >=
//                  top+w=120 holds; max-of-sums = max(220,220) = 220
//                  < 200+20*sqrt2 = 228.284 -> FALSE; sum-of-maxes =
//                  100+220 = 320 >= 228.284 -> TRUE (the swapped form).
//   ws6_positive   a=(0,240)   b=(100,140) side 6 w=20 wS=true
//                  main 2nd disjunct min-of-sums 240 >= 228.284 ->
//                  TRUE (the wS arm is not even reached).
//   main6_hit_ws   main6_hit's line with wS=true -> TRUE (other path,
//                  same verdict).
//
// Part 2 — the checkForcedPad budget sweep (MINOR-2): a pad-shaped
// IntBox [cx-400,cx+400]x[cy-300,cy+300] around the board center that
// overlaps ONLY the probe world's foreign trace (the foreign via at
// cy+600 with its +/-250 padstack stays outside), swept over
// maxRecursionDepth 0..10 with checkOnlyFront=false; one row per
// budget with the verdict and the reported failing obstacle.
//
// Determinism: fixed row order; booleans and ints only; the sweep
// runs on one fresh parse in ascending budget order.
package app.freerouting.board.actions;

import app.freerouting.board.actions.ItemIdGenerator;
import app.freerouting.board.facade.BasicBoard;
import app.freerouting.board.facade.RoutingBoard;
import app.freerouting.board.model.items.Item;
import app.freerouting.board.model.structure.FixedState;
import app.freerouting.board.model.structure.ShapeEntrySide;
import app.freerouting.board.trace.PolylineTrace;
import app.freerouting.geometry.planar.IntBox;
import app.freerouting.geometry.planar.IntPoint;
import app.freerouting.geometry.planar.Line;
import app.freerouting.geometry.planar.Polyline;
import app.freerouting.geometry.planar.TileShape;
import app.freerouting.io.BoardReadResult;
import app.freerouting.io.specctra.DsnReader;
import com.google.gson.Gson;
import com.google.gson.JsonObject;
import java.io.ByteArrayInputStream;
import java.lang.reflect.Method;
import java.nio.file.Files;
import java.nio.file.Paths;
import java.util.LinkedList;

public class ForcedPadFrontProbe {

  private static final Gson GSON = new Gson();

  private static synchronized void row(JsonObject o) {
    System.out.println(GSON.toJson(o));
  }

  private static JsonObject obj(String type) {
    JsonObject o = new JsonObject();
    o.addProperty("type", type);
    return o;
  }

  private static boolean inFrontOfPad(
      Method method, Line line, TileShape pad, int side, int width, boolean withSides)
      throws Exception {
    return (Boolean) method.invoke(null, line, pad, side, width, withSides);
  }

  public static void main(String[] args) throws Exception {
    byte[] bytes = Files.readAllBytes(Paths.get(args[0]));
    BoardReadResult read =
        DsnReader.readBoard(
            new ByteArrayInputStream(bytes),
            null,
            new ItemIdGenerator(),
            args.length > 1 ? args[1] : "t9_locator45.dsn");
    if (!(read instanceof BoardReadResult.Success success)) {
      throw new IllegalStateException("parse failed");
    }
    RoutingBoard board = (RoutingBoard) success.board();
    board.searchTreeManager.reinsertTreeItems();

    Method method =
        ForcedPadRouter.class.getDeclaredMethod(
            "inFrontOfPad", Line.class, TileShape.class, int.class, int.class, boolean.class);
    method.setAccessible(true);

    TileShape pad4 =
        new IntBox(new IntPoint(0, 0), new IntPoint(100, 100)).boundingOctagon();

    JsonObject padRow = obj("pad");
    app.freerouting.geometry.planar.IntOctagon oct =
        (app.freerouting.geometry.planar.IntOctagon) pad4;
    padRow.addProperty("leftX", oct.leftX);
    padRow.addProperty("rightX", oct.rightX);
    padRow.addProperty("bottomY", oct.bottomY);
    padRow.addProperty("topY", oct.topY);
    padRow.addProperty("upperLeftDiagonalX", oct.upperLeftDiagonalX);
    padRow.addProperty("lowerLeftDiagonalX", oct.lowerLeftDiagonalX);
    padRow.addProperty("lowerRightDiagonalX", oct.lowerRightDiagonalX);
    padRow.addProperty("upperRightDiagonalX", oct.upperRightDiagonalX);
    row(padRow);

    // the table; widths are per-row so one pad serves both regimes
    Object[][] table = {
      {"main6_hit", new int[] {0, 210}, new int[] {210, 0}, 6, 4, false},
      {"main6_miss", new int[] {0, 180}, new int[] {180, 0}, 6, 4, false},
      {"ws6_trap", new int[] {0, 220}, new int[] {100, 120}, 6, 20, true},
      {"ws6_positive", new int[] {0, 240}, new int[] {100, 140}, 6, 20, true},
      {"main6_hit_ws", new int[] {0, 210}, new int[] {210, 0}, 6, 4, true},
    };
    for (Object[] entry : table) {
      String label = (String) entry[0];
      int[] a = (int[]) entry[1];
      int[] b = (int[]) entry[2];
      int side = (Integer) entry[3];
      int width = (Integer) entry[4];
      boolean withSides = (Boolean) entry[5];
      Line line = new Line(new IntPoint(a[0], a[1]), new IntPoint(b[0], b[1]));
      boolean result = inFrontOfPad(method, line, pad4, side, width, withSides);
      JsonObject o = obj("front");
      o.addProperty("label", label);
      o.addProperty("ax", a[0]);
      o.addProperty("ay", a[1]);
      o.addProperty("bx", b[0]);
      o.addProperty("by", b[1]);
      o.addProperty("side", side);
      o.addProperty("width", width);
      o.addProperty("withSides", withSides);
      o.addProperty("result", result);
      row(o);
    }

    // ---- Part 2: the checkForcedPad budget sweep -------------------
    IntBox box = board.boundingBox;
    int cx = (box.ll.x + box.ur.x) / 2;
    int cy = (box.ll.y + box.ur.y) / 2;
    int ownNet = board.rules.nets.get("N001", 1).netNumber;
    int foreignNet = board.rules.nets.get("N002", 1).netNumber;
    // an UNFIXED foreign trace across the pad box: the shove target
    PolylineTrace sweepTrace =
        board.insertTraceWithoutCleaning(
            new Polyline(new IntPoint(cx - 1000, cy), new IntPoint(cx + 1000, cy)),
            0,
            150,
            new int[] {foreignNet},
            0,
            FixedState.UNFIXED);
    TileShape traceBox =
        new IntBox(new IntPoint(cx - 400, cy - 300), new IntPoint(cx + 400, cy + 300));
    ShapeEntrySide side = new ShapeEntrySide(new IntPoint(cx, cy - 300), traceBox);
    int[] ownNets = {ownNet};
    JsonObject sweepWorld = obj("sweep-world");
    sweepWorld.addProperty("traceId", sweepTrace.getId());
    sweepWorld.addProperty("cx", cx);
    sweepWorld.addProperty("cy", cy);
    row(sweepWorld);
    for (int budget = 0; budget <= 10; budget++) {
      ForcedPadRouter.CheckDrillResult result =
          new ForcedPadRouter(board)
              .checkForcedPad(
                  traceBox,
                  side,
                  0,
                  ownNets,
                  0,
                  false,
                  new LinkedList<>(),
                  budget,
                  10,
                  false,
                  null);
      JsonObject o = obj("sweep");
      o.addProperty("budget", budget);
      o.addProperty("result", result.name());
      Item failing = board.getShoveFailingObstacle();
      o.addProperty("failingObstacleId", failing == null ? -1 : failing.getId());
      row(o);
    }
  }
}
