// TraceShoverProbe.java — the M3-T10b jar-side probe oracle for the
// MUTUAL-RECURSION shove drivers (TraceShover.check/insert ->
// DrillItemMover.shoveVias/tryShoveViaPoints -> ForcedPadRouter ->
// TraceShover). ShapeTraceEntriesProbe house pattern: ONE JVM per run,
// deterministic JSONL rows on stdout (lines starting with "{" are the
// spike results; the jar's own noise is interleaved and dropped by the
// consumer).
//
// Build/run (JDK 25, from the repo root):
//   mkdir -p /tmp/epic-t10b-classes && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/javac \
//       -cp build/libs/freerouting-current-executable.jar \
//       -d /tmp/epic-t10b-classes rust/harness/oracle/TraceShoverProbe.java && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/java \
//       -cp build/libs/freerouting-current-executable.jar:/tmp/epic-t10b-classes \
//       app.freerouting.board.optimize.TraceShoverProbe \
//       rust/harness/fixtures/locator-spike/t9_locator45.dsn
//
// The probe declares the app.freerouting.board.optimize package so the
// TraceShover surface is directly reachable (its ctor + check/insert
// are public; the package declaration is the T10b brief contract).
//
// Synthetic shove world (deterministic, inserted through the real
// board API on a fresh parse per run): one FOREIGN-net horizontal
// trace crossing the board-bbox center C, and one FOREIGN-net
// through-all via at (cx, cy+600) — off every trace line so
// insertVia's splitTraces cannot eat it (the T10a lesson). The own net
// N001 wants the vertical corridor through C: the main shape is the
// IntBox [cx-400, cx+400] x [cy-800, cy+800], which overlaps BOTH the
// foreign trace and the foreign via.
//
// Runs, in order (check runs are non-mutating; insert runs mutate):
//   check_main  — native [shove_check_obstacles] row (pieces > 0)
//   check_zero  — via-only box: native [shove_check_obstacles_zero]
//   insert_zero — via-only box: DrillItemMover.shoveVias MOVES the
//                 via, then native [shove_insert_obstacles_zero] row
//   insert_main — native [shove_insert_obstacles] row + real trace
//                 cutout
//
// The [shove_check_obstacles]/[shove_insert_obstacles] rows are
// ORACLE-NATIVE (TraceShover.java:268-303 and :466-502): the drivers
// emit them through FRLogger.trace whenever netNumbers is non-empty
// and the obstacle collection is non-empty, with the `_zero` suffix
// when substituteTraceCount() == 0. A log4j tap (the RipupSpike
// precedent) re-emits every message with one of those two prefixes as
// a row, in emission order — the recursion emits one row per
// TraceShover.check/insert invocation, so a capture shows the whole
// recursion tree.
//
// Determinism: inventory rows walk board.getItems() sorted by id; the
// native obstacle rows iterate the search-tree pre-order collection
// (deterministic for a deterministically built tree); all doubles are
// Double.toString; no HashSet/HashMap iteration reaches a row.
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
import org.apache.logging.log4j.Level;
import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.core.LogEvent;
import org.apache.logging.log4j.core.LoggerContext;
import org.apache.logging.log4j.core.appender.AbstractAppender;
import org.apache.logging.log4j.core.config.Property;

public class TraceShoverProbe {

  private static final Gson GSON = new Gson();

  /** The two ORACLE-NATIVE row prefixes (TraceShover.java:271/:470). */
  private static final String CHECK_PREFIX = "[shove_check_obstacles";
  private static final String INSERT_PREFIX = "[shove_insert_obstacles";

  /**
   * The native rows embed `traceShape.boundingBox()` whose default
   * toString carries a JVM-varying identity hash
   * (`...IntBox@548a24a`). Normalize every `Class@hash` token to
   * `Class#ordinal` by FIRST APPEARANCE — the emission order is the
   * deterministic recursion order, so the ordinals are stable across
   * JVMs while still distinguishing which rows share one shape
   * object.
   */
  private static final java.util.Map<String, Integer> IDENTITY_ORDINALS =
      new java.util.LinkedHashMap<>();
  private static final java.util.regex.Pattern IDENTITY_HASH =
      java.util.regex.Pattern.compile("[A-Za-z0-9.$]+@[0-9a-f]+");

  private static String normalizeIdentityHashes(String msg) {
    java.util.regex.Matcher m = IDENTITY_HASH.matcher(msg);
    StringBuilder out = new StringBuilder();
    while (m.find()) {
      String token = m.group();
      Integer ordinal =
          IDENTITY_ORDINALS.computeIfAbsent(token, k -> IDENTITY_ORDINALS.size() + 1);
      String classPart = token.substring(0, token.indexOf('@'));
      m.appendReplacement(
          out, java.util.regex.Matcher.quoteReplacement(classPart + "#" + ordinal));
    }
    m.appendTail(out);
    return out.toString();
  }

  /** Emitted synchronously; stdout is the single ordered sink. */
  private static synchronized void row(JsonObject o) {
    System.out.println(GSON.toJson(o));
  }

  private static JsonObject obj(String type) {
    JsonObject o = new JsonObject();
    o.addProperty("type", type);
    return o;
  }

  /**
   * The log4j tap: every native [shove_check_obstacles*] /
   * [shove_insert_obstacles*] message becomes a row, in emission
   * order. Wiring is the RipupSpike precedent: the LOGGER's level is
   * not enough (dispatch filters on the LoggerConfig), so a dedicated
   * ALL-level LoggerConfig is registered for the Freerouting logger
   * and the tap is bound on both the config and the core logger.
   */
  private static void attachShoveTap() {
    LoggerContext ctx = (LoggerContext) LogManager.getContext(false);
    AbstractAppender tap =
        new AbstractAppender("shove-spike-tap", null, null, true, Property.EMPTY_ARRAY) {
          @Override
          public void append(LogEvent event) {
            String m = event.getMessage().getFormattedMessage();
            if (m != null
                && (m.startsWith(CHECK_PREFIX) || m.startsWith(INSERT_PREFIX))) {
              JsonObject o = obj("trace_row");
              o.addProperty("msg", normalizeIdentityHashes(m));
              row(o);
            }
          }
        };
    tap.start();
    org.apache.logging.log4j.core.config.Configuration config = ctx.getConfiguration();
    org.apache.logging.log4j.core.config.LoggerConfig loggerConfig =
        new org.apache.logging.log4j.core.config.LoggerConfig(
            "app.freerouting.Freerouting", Level.ALL, false);
    loggerConfig.addAppender(tap, Level.ALL, null);
    config.addLogger("app.freerouting.Freerouting", loggerConfig);
    ctx.updateLoggers();
    org.apache.logging.log4j.core.Logger freeroutingLogger =
        ctx.getLogger("app.freerouting.Freerouting");
    freeroutingLogger.setLevel(Level.ALL);
    freeroutingLogger.addAppender(tap);
    config.getRootLogger().addAppender(tap, Level.ALL, null);
    ctx.updateLoggers();
    org.apache.logging.log4j.core.Logger core =
        (org.apache.logging.log4j.core.Logger)
            LogManager.getLogger(app.freerouting.Freerouting.class);
    core.addAppender(tap);
    // Self-test: the tap must see this probe row; it is filtered OUT
    // of the row stream (wrong prefix), so the direct marker below
    // proves the tap is alive only indirectly — the native rows in the
    // stream are the real proof.
    app.freerouting.logger.FRLogger.trace("[shove_tap] alive");
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

  /** The synthetic shove world: ids of the inserted foreign items. */
  private static final class World {
    final int foreignTrace;
    final int foreignVia;
    final int ownNet;
    final int foreignNet;
    final int cx;
    final int cy;

    World(int foreignTrace, int foreignVia, int ownNet, int foreignNet, int cx, int cy) {
      this.foreignTrace = foreignTrace;
      this.foreignVia = foreignVia;
      this.ownNet = ownNet;
      this.foreignNet = foreignNet;
      this.cx = cx;
      this.cy = cy;
    }
  }

  private static World buildWorld(RoutingBoard board) {
    IntBox box = board.boundingBox;
    int cx = (box.ll.x + box.ur.x) / 2;
    int cy = (box.ll.y + box.ur.y) / 2;
    int ownNet = board.rules.nets.get("N001", 1).netNumber;
    int foreignNet = board.rules.nets.get("N002", 1).netNumber;
    int layerCount = board.layerStructure.layers.length;
    // a dedicated through-all via padstack covering every layer
    Padstack viaPadstack =
        board.library.padstacks.add(
            new IntBox(new IntPoint(-250, -250), new IntPoint(250, 250)), 0, layerCount - 1);
    PolylineTrace foreignTrace =
        board.insertTraceWithoutCleaning(
            new Polyline(new IntPoint(cx - 2000, cy), new IntPoint(cx + 2000, cy)),
            0,
            100,
            new int[] {foreignNet},
            0,
            FixedState.UNFIXED);
    // via center OFF every trace line so splitTraces cannot eat it
    Via foreignVia =
        board.insertVia(
            viaPadstack,
            new IntPoint(cx, cy + 600),
            new int[] {foreignNet},
            0,
            FixedState.UNFIXED,
            false);
    return new World(
        foreignTrace.getId(), foreignVia.getId(), ownNet, foreignNet, cx, cy);
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

  private static Via findVia(RoutingBoard board, int id) {
    for (Item item : board.getItems()) {
      if (item instanceof Via via && via.getId() == id) {
        return via;
      }
    }
    throw new IllegalStateException("via " + id + " not found");
  }

  private static void verdictRow(
      String run, boolean verdict, TileShape shape, ShapeEntrySide fromSide) {
    JsonObject o = obj("verdict");
    o.addProperty("run", run);
    o.addProperty("result", verdict);
    o.addProperty("shape", shapeNameOf(shape));
    o.addProperty("fromSideNo", fromSide.no);
    row(o);
  }

  private static void shovingStateRow(String phase, RoutingBoard board) {
    JsonObject o = obj("shoving-state");
    o.addProperty("phase", phase);
    o.addProperty("failingLayer", board.getShoveFailingLayer());
    Item obstacle = board.getShoveFailingObstacle();
    o.addProperty("failingObstacleId", obstacle == null ? -1 : obstacle.getId());
    row(o);
  }

  private static void runAll(byte[] bytes, String fileName) throws Exception {
    RoutingBoard board = parse(bytes, fileName);
    World world = buildWorld(board);

    JsonObject worldRow = obj("world");
    worldRow.addProperty("foreignTrace", world.foreignTrace);
    worldRow.addProperty("foreignVia", world.foreignVia);
    worldRow.addProperty("ownNet", world.ownNet);
    worldRow.addProperty("foreignNet", world.foreignNet);
    worldRow.addProperty("cx", world.cx);
    worldRow.addProperty("cy", world.cy);
    worldRow.addProperty("layers", board.layerStructure.layers.length);
    row(worldRow);
    dumpInventory("before", board);

    // main corridor shape: overlaps the foreign trace AND the via
    TileShape mainShape =
        new IntBox(
            new IntPoint(world.cx - 400, world.cy - 800),
            new IntPoint(world.cx + 400, world.cy + 800));
    // zero-piece shape: overlaps ONLY the via (clear of the trace)
    TileShape zeroShape =
        new IntBox(
            new IntPoint(world.cx - 400, world.cy + 300),
            new IntPoint(world.cx + 400, world.cy + 900));

    TraceShover shover = new TraceShover(board);
    int[] ownNets = {world.ownNet};
    int depth = 10;
    int viaDepth = 10;
    int springDepth = 2;

    // ---- non-mutating check runs ---------------------------------
    ShapeEntrySide mainSide =
        new ShapeEntrySide(new IntPoint(world.cx, world.cy - 800), mainShape);
    boolean checkMain =
        shover.check(
            mainShape, mainSide, null, 0, ownNets, 0, depth, viaDepth, springDepth, null);
    verdictRow("check_main", checkMain, mainShape, mainSide);
    shovingStateRow("after-check-main", board);

    ShapeEntrySide zeroSide =
        new ShapeEntrySide(new IntPoint(world.cx, world.cy + 300), zeroShape);
    boolean checkZero =
        shover.check(
            zeroShape, zeroSide, null, 0, ownNets, 0, depth, viaDepth, springDepth, null);
    verdictRow("check_zero", checkZero, zeroShape, zeroSide);
    shovingStateRow("after-check-zero", board);

    // ---- mutating insert runs ------------------------------------
    // insert_zero physically SHOVES the via out of the zero box
    // (DrillItemMover.shoveVias), then hits tracePieceCount == 0.
    boolean insertZero =
        shover.insert(
            zeroShape, zeroSide, 0, ownNets, 0, new LinkedList<>(), depth, viaDepth, springDepth);
    verdictRow("insert_zero", insertZero, zeroShape, zeroSide);
    Via viaAfter = findVia(board, world.foreignVia);
    FloatPoint viaCenter = viaAfter.getCenter().toFloat();
    JsonObject viaRow = obj("via-after-shove");
    viaRow.addProperty("run", "insert_zero");
    viaRow.addProperty("x", Double.toString(viaCenter.x));
    viaRow.addProperty("y", Double.toString(viaCenter.y));
    row(viaRow);
    shovingStateRow("after-insert-zero", board);

    // insert_main cuts the foreign trace apart around the corridor
    boolean insertMain =
        shover.insert(
            mainShape, mainSide, 0, ownNets, 0, new LinkedList<>(), depth, viaDepth, springDepth);
    verdictRow("insert_main", insertMain, mainShape, mainSide);
    shovingStateRow("after-insert-main", board);

    dumpInventory("after", board);
  }

  public static void main(String[] args) throws Exception {
    byte[] bytes = Files.readAllBytes(Paths.get(args[0]));
    String fileName = args.length > 1 ? args[1] : "t9_locator45.dsn";
    attachShoveTap();
    runAll(bytes, fileName);
  }
}
