// ShapeTraceEntriesProbe.java — the M3-T10a jar-side probe oracle for
// the shove substrate port (ShapeTraceEntries + ShapeEntrySide +
// ShapeAndEntrySide). LocatorSpike house pattern: ONE JVM per run,
// deterministic JSONL rows on stdout (lines starting with "{" are the
// spike results; the jar's FRLogger noise is interleaved and dropped).
//
// Build/run (JDK 25, from the repo root):
//   mkdir -p /tmp/epic-t10a-classes && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/javac \
//       -cp build/libs/freerouting-current-executable.jar \
//       -d /tmp/epic-t10a-classes rust/harness/oracle/ShapeTraceEntriesProbe.java && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/java \
//       -cp build/libs/freerouting-current-executable.jar:/tmp/epic-t10a-classes \
//       app.freerouting.board.searchtree.ShapeTraceEntriesProbe \
//       rust/harness/fixtures/locator-spike/t9_locator45.dsn
//
// The probe declares the app.freerouting.board.searchtree package so
// the package-private searchtree surfaces are directly reachable;
// ShapeTraceEntries.listAnchor and the private static EntryPoint class
// are reached REFLECTIVELY (the chain order is the observable the
// port's Vec<EntryPoint> models).
//
// Synthetic world (deterministic, inserted through the real board API
// on a fresh parse per run): four 45-degree-legal traces through a
// common center C (the board bbox center), two nets (N001 own / N002
// foreign), plus one through-all via of the foreign net. Runs:
// three shapes x from-side variants through storeItems (chain dump,
// piece counts, shove-via list, substitute pieces until null) and
// three cutoutTrace runs (nothing-cut / fast two-piece / slow
// multi-piece) with per-trace tree-entry leaf dumps after.
//
// Determinism: the storeItems item list is a TreeSet ordered by item
// id; every double is printed with Double.toString; all walks are
// over sorted ids or the (deterministic) entry chain itself.
package app.freerouting.board.searchtree;

import app.freerouting.board.actions.ItemIdGenerator;
import app.freerouting.board.facade.BasicBoard;
import app.freerouting.board.facade.RoutingBoard;
import app.freerouting.board.model.items.Item;
import app.freerouting.board.model.items.Via;
import app.freerouting.board.model.structure.FixedState;
import app.freerouting.board.model.structure.ShapeAndEntrySide;
import app.freerouting.board.model.structure.ShapeEntrySide;
import app.freerouting.board.trace.PolylineTrace;
import app.freerouting.core.library.Padstack;
import app.freerouting.datastructures.ShapeTree;
import app.freerouting.geometry.planar.FloatPoint;
import app.freerouting.geometry.planar.IntBox;
import app.freerouting.geometry.planar.IntPoint;
import app.freerouting.geometry.planar.Line;
import app.freerouting.geometry.planar.Polyline;
import app.freerouting.geometry.planar.TileShape;
import app.freerouting.io.BoardReadResult;
import app.freerouting.io.specctra.DsnReader;
import com.google.gson.Gson;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import java.io.ByteArrayInputStream;
import java.lang.reflect.Field;
import java.nio.file.Files;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import java.util.TreeSet;

public class ShapeTraceEntriesProbe {

  private static final Gson GSON = new Gson();

  private static final Field LIST_ANCHOR_FIELD;
  private static final Field EP_NEXT_FIELD;
  private static final Field EP_TRACE_FIELD;
  private static final Field EP_TRACE_LINE_NO_FIELD;
  private static final Field EP_ENTRY_APPROX_FIELD;
  private static final Field EP_EDGE_INDEX_FIELD;
  private static final Field EP_STACK_LEVEL_FIELD;

  static {
    try {
      LIST_ANCHOR_FIELD = ShapeTraceEntries.class.getDeclaredField("listAnchor");
      LIST_ANCHOR_FIELD.setAccessible(true);
      Class<?> epClass = Class.forName("app.freerouting.board.searchtree.ShapeTraceEntries$EntryPoint");
      EP_NEXT_FIELD = epClass.getDeclaredField("next");
      EP_NEXT_FIELD.setAccessible(true);
      EP_TRACE_FIELD = epClass.getDeclaredField("trace");
      EP_TRACE_FIELD.setAccessible(true);
      EP_TRACE_LINE_NO_FIELD = epClass.getDeclaredField("traceLineNo");
      EP_TRACE_LINE_NO_FIELD.setAccessible(true);
      EP_ENTRY_APPROX_FIELD = epClass.getDeclaredField("entryApprox");
      EP_ENTRY_APPROX_FIELD.setAccessible(true);
      EP_EDGE_INDEX_FIELD = epClass.getDeclaredField("edgeIndex");
      EP_EDGE_INDEX_FIELD.setAccessible(true);
      EP_STACK_LEVEL_FIELD = epClass.getDeclaredField("stackLevel");
      EP_STACK_LEVEL_FIELD.setAccessible(true);
    } catch (Exception e) {
      throw new ExceptionInInitializerError(e);
    }
  }

  private static void row(JsonObject obj) {
    System.out.println(GSON.toJson(obj));
  }

  private static void addPoint(JsonArray arr, FloatPoint p) {
    JsonArray pt = new JsonArray();
    pt.add(Double.toString(p.x));
    pt.add(Double.toString(p.y));
    arr.add(pt);
  }

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

  /** The synthetic shove world: ids of the inserted traces/via. */
  private static final class World {
    final int traceA1; // horizontal, own net
    final int traceB1; // vertical, foreign net
    final int traceB2; // diagonal, foreign net
    final int traceA2; // anti-diagonal, own net
    final int viaB; // foreign-net through via
    final int ownNet;
    final int foreignNet;

    World(
        int traceA1, int traceB1, int traceB2, int traceA2, int viaB, int ownNet,
        int foreignNet) {
      this.traceA1 = traceA1;
      this.traceB1 = traceB1;
      this.traceB2 = traceB2;
      this.traceA2 = traceA2;
      this.viaB = viaB;
      this.ownNet = ownNet;
      this.foreignNet = foreignNet;
    }
  }

  private static World buildWorld(RoutingBoard board) {
    IntBox box = board.boundingBox;
    int cx = (box.ll.x + box.ur.x) / 2;
    int cy = (box.ll.y + box.ur.y) / 2;
    int ownNet = board.rules.nets.get("N001", 1).netNumber;
    int foreignNet = board.rules.nets.get("N002", 1).netNumber;
    int layerCount = board.layerStructure.layers.length;
    // a dedicated through-all via padstack covering every layer, with
    // the internally generated (deterministic) name
    Padstack viaPadstack =
        board.library.padstacks.add(
            new IntBox(new IntPoint(-250, -250), new IntPoint(250, 250)), 0, layerCount - 1);
    PolylineTrace t1 =
        board.insertTraceWithoutCleaning(
            new Polyline(new IntPoint(cx - 2000, cy), new IntPoint(cx + 2000, cy)),
            0,
            100,
            new int[] {ownNet},
            0,
            FixedState.UNFIXED);
    PolylineTrace t2 =
        board.insertTraceWithoutCleaning(
            new Polyline(new IntPoint(cx, cy - 2000), new IntPoint(cx, cy + 2000)),
            0,
            100,
            new int[] {foreignNet},
            0,
            FixedState.UNFIXED);
    PolylineTrace t3 =
        board.insertTraceWithoutCleaning(
            new Polyline(new IntPoint(cx - 1500, cy - 1500), new IntPoint(cx + 1500, cy + 1500)),
            0,
            100,
            new int[] {foreignNet},
            0,
            FixedState.UNFIXED);
    PolylineTrace t4 =
        board.insertTraceWithoutCleaning(
            new Polyline(new IntPoint(cx - 1000, cy + 1000), new IntPoint(cx + 1000, cy - 1000)),
            0,
            100,
            new int[] {ownNet},
            0,
            FixedState.UNFIXED);
    Via via =
        board.insertVia(
            viaPadstack,
            // off every trace line: a via center ON a same-net trace
            // makes insertVia's splitTraces eat the trace (that bit us
            // at (cx+500, cy+500), exactly on the diagonal). The pad
            // box corner still touches the diagonal — overlap, not
            // split — which is exactly the shove material we want.
            new IntPoint(cx + 500, cy + 1000),
            new int[] {foreignNet},
            0,
            FixedState.UNFIXED,
            false);
    return new World(
        t1.getId(),
        t2.getId(),
        t3.getId(),
        t4.getId(),
        via.getId(),
        ownNet,
        foreignNet);
  }

  private static String shapeNameOf(TileShape shape) {
    IntBox bb = shape.boundingBox();
    return bb.ll.x + ":" + bb.ll.y + ":" + bb.ur.x + ":" + bb.ur.y;
  }

  /** Dumps the entry chain (listAnchor .. end) via reflection. */
  private static void dumpChain(String run, ShapeTraceEntries entries) throws Exception {
    Object anchor = LIST_ANCHOR_FIELD.get(entries);
    JsonObject chain = new JsonObject();
    chain.addProperty("type", "chain");
    chain.addProperty("run", run);
    JsonArray list = new JsonArray();
    int order = 0;
    while (anchor != null) {
      JsonObject ep = new JsonObject();
      ep.addProperty("order", order);
      PolylineTrace trace = (PolylineTrace) EP_TRACE_FIELD.get(anchor);
      ep.addProperty("traceId", trace.getId());
      ep.addProperty("lineNo", (Integer) EP_TRACE_LINE_NO_FIELD.get(anchor));
      FloatPoint approx = (FloatPoint) EP_ENTRY_APPROX_FIELD.get(anchor);
      JsonArray approxArr = new JsonArray();
      approxArr.add(Double.toString(approx.x));
      approxArr.add(Double.toString(approx.y));
      ep.add("approx", approxArr);
      ep.addProperty("edge", (Integer) EP_EDGE_INDEX_FIELD.get(anchor));
      ep.addProperty("level", (Integer) EP_STACK_LEVEL_FIELD.get(anchor));
      list.add(ep);
      anchor = EP_NEXT_FIELD.get(anchor);
      order++;
    }
    chain.add("entries", list);
    row(chain);
  }

  private static void addCorners(JsonArray corners, Polyline poly) {
    for (int i = 0; i < poly.lines.length - 1; i++) {
      addPoint(corners, poly.cornerApprox(i));
    }
  }

  /** Runs storeItems and dumps the full observable surface. */
  private static void runStore(
      String run,
      RoutingBoard board,
      World world,
      TileShape shape,
      ShapeEntrySide fromSide,
      boolean isPadCheck,
      boolean copperSharingAllowed,
      int layer)
      throws Exception {
    TreeSet<Item> items = new TreeSet<>(Comparator.comparingInt(Item::getId));
    items.addAll(board.overlappingItems(shape.boundingBox(), layer));

    ShapeTraceEntries entries =
        new ShapeTraceEntries(shape, layer, new int[] {world.ownNet}, 0, fromSide, board);
    boolean result = entries.storeItems(items, isPadCheck, copperSharingAllowed);

    JsonObject meta = new JsonObject();
    meta.addProperty("type", "run");
    meta.addProperty("run", run);
    meta.addProperty("shape", shapeNameOf(shape));
    meta.addProperty("layer", layer);
    meta.addProperty("ownNet", world.ownNet);
    meta.addProperty("padCheck", isPadCheck);
    meta.addProperty("share", copperSharingAllowed);
    meta.addProperty("result", result);
    JsonArray itemIds = new JsonArray();
    for (Item item : items) {
      itemIds.add(item.getId());
    }
    meta.add("items", itemIds);
    row(meta);

    // chain dump BEFORE the substitute loop consumes it
    dumpChain(run, entries);

    JsonObject counts = new JsonObject();
    counts.addProperty("type", "counts");
    counts.addProperty("run", run);
    counts.addProperty("pieceCount", entries.substituteTraceCount());
    counts.addProperty("maxStackLevel", entries.stackDepth());
    counts.addProperty("tailsInShape", entries.traceTailsInShape());
    Item obstacle = entries.getFoundObstacle();
    counts.addProperty("foundObstacle", obstacle == null ? -1 : obstacle.getId());
    JsonArray viaIds = new JsonArray();
    for (Via via : entries.shoveViaList) {
      viaIds.add(via.getId());
    }
    counts.add("shoveViaList", viaIds);
    row(counts);

    int pieceIndex = 0;
    for (; ; ) {
      PolylineTrace piece = entries.nextSubstituteTracePiece();
      if (piece == null) {
        break;
      }
      JsonObject pieceRow = new JsonObject();
      pieceRow.addProperty("type", "piece");
      pieceRow.addProperty("run", run);
      pieceRow.addProperty("i", pieceIndex);
      pieceRow.addProperty("layer", piece.getLayer());
      pieceRow.addProperty("halfWidth", piece.getHalfWidth());
      pieceRow.addProperty("class", piece.clearanceClassIndex());
      JsonArray nets = new JsonArray();
      for (int netNo : piece.netNumbers) {
        nets.add(netNo);
      }
      pieceRow.add("nets", nets);
      JsonArray corners = new JsonArray();
      addCorners(corners, piece.polyline());
      pieceRow.add("corners", corners);
      row(pieceRow);
      pieceIndex++;
    }
    JsonObject endRow = new JsonObject();
    endRow.addProperty("type", "pieces-end");
    endRow.addProperty("run", run);
    endRow.addProperty("pieces", pieceIndex);
    row(endRow);
  }

  private static void dumpTraceLeaves(String run, String phase, RoutingBoard board) {
    JsonArray traces = new JsonArray();
    List<Item> sorted = new ArrayList<>(board.getItems());
    sorted.sort(Comparator.comparingInt(Item::getId));
    ShapeSearchTree defaultTree = board.searchTreeManager.getDefaultTree();
    for (Item item : sorted) {
      if (item instanceof PolylineTrace trace) {
        JsonObject tr = new JsonObject();
        tr.addProperty("id", trace.getId());
        tr.addProperty("onBoard", trace.isOnTheBoard());
        JsonArray corners = new JsonArray();
        addCorners(corners, trace.polyline());
        tr.add("corners", corners);
        if (trace.isOnTheBoard()) {
          ShapeTree.Leaf[] leaves = trace.getSearchTreeEntries(defaultTree);
          JsonArray leafArr = new JsonArray();
          if (leaves != null) {
            for (ShapeTree.Leaf leaf : leaves) {
              JsonObject leafRow = new JsonObject();
              leafRow.addProperty(
                  "objId", ((app.freerouting.board.model.items.Item) leaf.object).getId());
              leafRow.addProperty("shapeIdx", leaf.shapeIndexInObject);
              leafArr.add(leafRow);
            }
          }
          tr.add("leaves", leafArr);
        }
        traces.add(tr);
      }
    }
    JsonObject out = new JsonObject();
    out.addProperty("type", "cutout-traces");
    out.addProperty("run", run);
    out.addProperty("phase", phase);
    out.add("traces", traces);
    row(out);
  }

  private static PolylineTrace findTrace(RoutingBoard board, int id) {
    for (Item item : board.getItems()) {
      if (item instanceof PolylineTrace trace && trace.getId() == id) {
        return trace;
      }
    }
    throw new IllegalStateException("trace " + id + " not found");
  }

  private static void dumpFromSide(String run, ShapeEntrySide side) {
    JsonObject fs = new JsonObject();
    fs.addProperty("type", "from-side");
    fs.addProperty("run", run);
    fs.addProperty("no", side.no);
    if (side.borderIntersection != null) {
      JsonArray bi = new JsonArray();
      bi.add(Double.toString(side.borderIntersection.x));
      bi.add(Double.toString(side.borderIntersection.y));
      fs.add("borderIntersection", bi);
    }
    row(fs);
  }

  /** Dumps a ShapeAndEntrySide: bounding box, border lines, from-side. */
  private static void dumpShapeAndEntrySide(String run, ShapeAndEntrySide saes) {
    JsonObject out = new JsonObject();
    out.addProperty("type", "saes");
    out.addProperty("run", run);
    IntBox bb = saes.shape.boundingBox();
    out.addProperty(
        "bb", bb.ll.x + ":" + bb.ll.y + ":" + bb.ur.x + ":" + bb.ur.y);
    JsonArray borders = new JsonArray();
    int count = saes.shape.borderLineCount();
    for (int i = 0; i < count; i++) {
      Line line = saes.shape.borderLine(i);
      JsonArray ln = new JsonArray();
      ln.add(Double.toString(line.a.toFloat().x));
      ln.add(Double.toString(line.a.toFloat().y));
      ln.add(Double.toString(line.b.toFloat().x));
      ln.add(Double.toString(line.b.toFloat().y));
      borders.add(ln);
    }
    out.add("borders", borders);
    ShapeEntrySide side = saes.fromSide;
    if (side == null) {
      out.add("fromSide", null);
    } else {
      JsonArray fsArr = new JsonArray();
      fsArr.add(side.no);
      if (side.borderIntersection != null) {
        fsArr.add(Double.toString(side.borderIntersection.x));
        fsArr.add(Double.toString(side.borderIntersection.y));
      } else {
        fsArr.add((String) null);
      }
      out.add("fromSide", fsArr);
    }
    row(out);
  }

  private static void runCutout(
      String run, byte[] bytes, String fileName, IntBox cutShapeBox, int classIndex)
      throws Exception {
    RoutingBoard board = parse(bytes, fileName);
    World world = buildWorld(board);
    dumpTraceLeaves(run, "before", board);
    PolylineTrace target = findTrace(board, world.traceB2);
    ShapeTraceEntries.cutoutTrace(target, cutShapeBox, classIndex);
    dumpTraceLeaves(run, "after", board);
  }

  public static void main(String[] args) throws Exception {
    byte[] bytes = Files.readAllBytes(Paths.get(args[0]));
    String fileName = Paths.get(args[0]).getFileName().toString();

    // storeItems runs: three shapes x from-side variants, one board
    {
      RoutingBoard board = parse(bytes, fileName);
      World world = buildWorld(board);
      IntBox box = board.boundingBox;
      int cx = (box.ll.x + box.ur.x) / 2;
      int cy = (box.ll.y + box.ur.y) / 2;

      // R1: shape covering the crossing, from side not calculated
      TileShape s1 =
          new IntBox(new IntPoint(cx - 1500, cy - 1500), new IntPoint(cx + 1500, cy + 1500));
      runStore("s1-notcalc", board, world, s1, ShapeEntrySide.NOT_CALCULATED, false, false, 0);

      // R2: offset shape (left half), from side not calculated
      TileShape s2 =
          new IntBox(new IntPoint(cx - 3500, cy - 1500), new IntPoint(cx - 500, cy + 1500));
      runStore("s2-offset", board, world, s2, ShapeEntrySide.NOT_CALCULATED, false, false, 0);

      // R3: small tight shape, from side via the nearest-border ctor
      TileShape s3 =
          new IntBox(new IntPoint(cx - 600, cy - 600), new IntPoint(cx + 600, cy + 600));
      ShapeEntrySide fromPointSide = new ShapeEntrySide(new IntPoint(cx, cy), s3);
      dumpFromSide("s3-frompoint", fromPointSide);
      runStore("s3-frompoint", board, world, s3, fromPointSide, false, false, 0);

      // R4: pad-check run with an entry-no from side on the diagonal
      // trace polyline (line no 1)
      PolylineTrace t3 = findTrace(board, world.traceB2);
      ShapeEntrySide entryNoSide = new ShapeEntrySide(t3.polyline(), 1, s1);
      dumpFromSide("s1-padcheck", entryNoSide);
      runStore("s1-padcheck", board, world, s1, entryNoSide, true, false, 0);

      // dog-ear captures: ShapeAndEntrySide ctor. Inserted AFTER the
      // store runs so the store-run item lists (and ids) stay as
      // captured; the cutout runs below use fresh parses, so t5's id
      // (110) does not collide with their split pieces.
      PolylineTrace t5 =
          board.insertTraceWithoutCleaning(
              new Polyline(
                  new IntPoint[] {
                    new IntPoint(cx - 2600, cy + 800),
                    new IntPoint(cx - 2600, cy - 800),
                    new IntPoint(cx - 1600, cy - 800),
                    new IntPoint(cx - 1600, cy + 800)
                  }),
              0,
              100,
              new int[] {world.ownNet},
              0,
              FixedState.UNFIXED);
      dumpShapeAndEntrySide("de-107-both", new ShapeAndEntrySide(t3, 0, false, false));
      dumpShapeAndEntrySide("de-107-orth", new ShapeAndEntrySide(t3, 0, true, false));
      // middle segment of a 4-segment trace: NO cutline fires; the
      // inShoveCheck gate keeps fromSide null (vs the fallback ctor A)
      dumpShapeAndEntrySide("de-5-mid-check", new ShapeAndEntrySide(t5, 1, false, true));
      dumpShapeAndEntrySide("de-5-mid", new ShapeAndEntrySide(t5, 1, false, false));
      dumpShapeAndEntrySide("de-5-start", new ShapeAndEntrySide(t5, 0, false, false));
    }

    // cutout runs on fresh parses: nothing-cut / fast two-piece / slow path
    {
      RoutingBoard board = parse(bytes, fileName);
      IntBox box = board.boundingBox;
      int cx = (box.ll.x + box.ur.x) / 2;
      int cy = (box.ll.y + box.ur.y) / 2;
      // (a) far from the diagonal trace: nothing cut off
      runCutout(
          "cut-none",
          bytes,
          fileName,
          new IntBox(new IntPoint(cx + 4000, cy + 4000), new IntPoint(cx + 4500, cy + 4500)),
          0);
      // (b) middle of the diagonal: fast two-piece path
      runCutout(
          "cut-mid",
          bytes,
          fileName,
          new IntBox(new IntPoint(cx - 400, cy - 400), new IntPoint(cx + 400, cy + 400)),
          0);
      // (c) end third of the diagonal: slow remove-and-reinsert path
      runCutout(
          "cut-end",
          bytes,
          fileName,
          new IntBox(new IntPoint(cx + 700, cy + 700), new IntPoint(cx + 2200, cy + 2200)),
          0);
    }
  }
}
