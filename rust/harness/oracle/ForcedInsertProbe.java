// ForcedInsertProbe.java — the M3-T10c jar-side probe oracle for the
// FORCED-INSERTION board surface (RoutingBoard.insertForcedTracePolyline
// / connectToTrace / removeTraceTails + the changed-area session).
// TraceShoverProbe house pattern: ONE JVM per run, deterministic JSONL
// rows on stdout.
//
// Build/run (JDK 25, from the repo root):
//   mkdir -p /tmp/epic-t10c-classes && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/javac \
//       -cp build/libs/freerouting-current-executable.jar \
//       -d /tmp/epic-t10c-classes rust/harness/oracle/ForcedInsertProbe.java && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/java \
//       -cp build/libs/freerouting-current-executable.jar:/tmp/epic-t10c-classes \
//       app.freerouting.board.facade.ForcedInsertProbe \
//       rust/harness/fixtures/locator-spike/t9_locator45.dsn
//
// The probe declares app.freerouting.board.facade (RoutingBoard's
// package) so package-private seams stay reachable.
//
// ORACLE-NATIVE rows: insertForcedTracePolyline already emits the
// `compare_trace_insert_forced_sub` / `compare_trace_shove_shape` rows
// (one-arg FRLogger.trace) and — when FRLogger.granularTraceEnabled is
// set and the net passes DebugControl's (empty) filter — the five-arg
// `compare_trace_insert_forced_fail` / `compare_trace_insert_forced_obstacle`
// rows, formatted "[<method>] [<operation>] <message>: <impactedItems>".
// A log4j tap re-emits every message containing one of the four tokens,
// in emission order. The probe sets FRLogger.granularTraceEnabled = true
// (public static field) so the five-arg rows fire.
//
// Worlds (fresh parse each, net N094 = net number 94, the native
// fail-row gate):
//   insert_tidy   — full success ladder with tidyWidth 400 (Java's
//                   pullTight RUNS; post-pull-tight state is the
//                   documented T10c divergence — rows captured, Rust
//                   pins skip after_pull_tight + post-pull geometry)
//   insert_notidy — SAME seeds/ladder with tidyWidth 0 (pullTight
//                   skipped) — fully comparable end to end; carries
//                   the connectToTrace ladder (c1 contains-shortcut,
//                   c3 check-blocked, c2 insert + tail cleanup LAST,
//                   because c2's tail removal deletes the target)
//   insert_fail   — corridor through an unfixed via with
//                   maxViaRecursionDepth 0: shove-loop check fails,
//                   sampling retry re-check fails -> the native
//                   `compare_trace_insert_forced_fail` +
//                   `compare_trace_insert_forced_obstacle` rows; the
//                   world stays UNDAMAGED (inventory pinned unchanged)
//   tails_none / tails_via / tails_fanout_via — fresh copies of a
//                   stub world (floating net-94 stub, net-95 via +
//                   stub pair, fanout via on the SMD pin D094), one
//                   removeTraceTails(-1, option) call each
//
// Changed-area rows dump board.changedArea per layer (the 8 IntOctagon
// fields by Java name) + surroundingBox after each phase.
//
// Determinism: inventory rows sort by id; doubles are Double.toString;
// identity-hash tokens normalized to Class#ordinal by first appearance;
// no HashSet iteration reaches a row.
package app.freerouting.board.facade;

import app.freerouting.board.actions.DrillItemMover;
import app.freerouting.board.actions.ItemIdGenerator;
import app.freerouting.board.model.items.Item;
import app.freerouting.board.model.items.Pin;
import app.freerouting.board.model.items.Trace;
import app.freerouting.board.model.items.Via;
import app.freerouting.board.model.structure.FixedState;
import app.freerouting.board.state.ChangedArea;
import app.freerouting.board.trace.PolylineTrace;
import app.freerouting.core.library.Padstack;
import app.freerouting.geometry.planar.FloatPoint;
import app.freerouting.geometry.planar.IntBox;
import app.freerouting.geometry.planar.IntPoint;
import app.freerouting.geometry.planar.IntVector;
import app.freerouting.geometry.planar.LineSegment;
import app.freerouting.geometry.planar.IntOctagon;
import app.freerouting.geometry.planar.Point;
import app.freerouting.geometry.planar.Polyline;
import app.freerouting.io.BoardReadResult;
import app.freerouting.io.specctra.DsnReader;
import app.freerouting.logger.FRLogger;
import com.google.gson.Gson;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import java.io.ByteArrayInputStream;
import java.nio.file.Files;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import org.apache.logging.log4j.Level;
import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.core.LogEvent;
import org.apache.logging.log4j.core.LoggerContext;
import org.apache.logging.log4j.core.appender.AbstractAppender;
import org.apache.logging.log4j.core.config.Property;

public class ForcedInsertProbe {

  private static final Gson GSON = new Gson();

  /** The ORACLE-NATIVE row tokens (RoutingBoard.java :497/:605/:529/:715). */
  private static final String[] NATIVE_TOKENS = {
    "compare_trace_insert_forced_sub",
    "compare_trace_shove_shape",
    "compare_trace_insert_forced_fail",
    "compare_trace_insert_forced_obstacle"
  };

  private static boolean isNativeRow(String msg) {
    for (String token : NATIVE_TOKENS) {
      if (msg.contains(token)) {
        return true;
      }
    }
    return false;
  }

  /**
   * Identity-hash normalization (TraceShoverProbe pattern): every
   * `Class@hash` token becomes `Class#ordinal` by FIRST APPEARANCE.
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

  private static synchronized void row(JsonObject o) {
    System.out.println(GSON.toJson(o));
  }

  private static JsonObject obj(String type) {
    JsonObject o = new JsonObject();
    o.addProperty("type", type);
    return o;
  }

  /** The log4j tap for the native rows (RipupSpike / TraceShoverProbe wiring). */
  private static void attachNativeTap() {
    LoggerContext ctx = (LoggerContext) LogManager.getContext(false);
    AbstractAppender tap =
        new AbstractAppender("forced-insert-tap", null, null, true, Property.EMPTY_ARRAY) {
          @Override
          public void append(LogEvent event) {
            String m = event.getMessage().getFormattedMessage();
            if (m != null && isNativeRow(m)) {
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
  }

  /** A fresh parse of the fixture bytes (TraceShoverProbe pattern). */
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

  /**
   * The net with NUMBER 94 (the native fail-row gate is
   * `netNumbers[0] == 94`, a number comparison). The NAME-to-number
   * mapping is off by one on this fixture (N094 has number 95), so the
   * probe selects by number and asserts the lookup exists.
   */
  private static int net94(RoutingBoard board) {
    if (board.rules.nets.get(94) == null) {
      throw new IllegalStateException("no net number 94");
    }
    return 94;
  }

  private static Padstack addViaPadstack(RoutingBoard board, String name) {
    int layerCount = board.layerStructure.layers.length;
    return board.library.padstacks.add(
        new IntBox(new IntPoint(-250, -250), new IntPoint(250, 250)), 0, layerCount - 1);
  }

  private static void dumpInventory(String phase, RoutingBoard board) {
    JsonArray items = new JsonArray();
    List<Item> sorted = new ArrayList<>(board.getItems());
    sorted.sort(Comparator.comparingInt(Item::getId));
    for (Item item : sorted) {
      JsonObject it = new JsonObject();
      it.addProperty("id", item.getId());
      it.addProperty("kind", item.getClass().getSimpleName());
      it.addProperty("fixed", item.getFixedState().toString());
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

  /** The changed-area session state (null when no session is active). */
  private static void dumpChangedArea(String phase, RoutingBoard board) {
    JsonObject out = obj("changed_area");
    out.addProperty("phase", phase);
    ChangedArea area = board.changedArea;
    if (area == null) {
      out.addProperty("session", false);
      row(out);
      return;
    }
    out.addProperty("session", true);
    JsonArray layers = new JsonArray();
    for (int l = 0; l < board.layerStructure.layers.length; l++) {
      IntOctagon oct = area.getArea(l);
      JsonObject lo = new JsonObject();
      lo.addProperty("layer", l);
      lo.addProperty("leftX", oct.leftX);
      lo.addProperty("bottomY", oct.bottomY);
      lo.addProperty("rightX", oct.rightX);
      lo.addProperty("topY", oct.topY);
      lo.addProperty("lowerLeftDiagonalX", oct.lowerLeftDiagonalX);
      lo.addProperty("lowerRightDiagonalX", oct.lowerRightDiagonalX);
      lo.addProperty("upperLeftDiagonalX", oct.upperLeftDiagonalX);
      lo.addProperty("upperRightDiagonalX", oct.upperRightDiagonalX);
      layers.add(lo);
    }
    out.add("layers", layers);
    IntBox box = area.surroundingBox();
    JsonObject bb = new JsonObject();
    bb.addProperty("x1", box.ll.x);
    bb.addProperty("y1", box.ll.y);
    bb.addProperty("x2", box.ur.x);
    bb.addProperty("y2", box.ur.y);
    out.add("surroundingBox", bb);
    row(out);
  }

  private static PolylineTrace findNet94TraceAt(RoutingBoard board, IntPoint at) {
    for (Item item : board.getItems()) {
      if (item instanceof PolylineTrace trace
          && trace.netNumbers.length == 1
          && trace.netNumbers[0] == net94(board)
          && trace.getLayer() == 0
          && trace.polyline().contains(at)) {
        return trace;
      }
    }
    throw new IllegalStateException("no net-94 trace at " + at);
  }

  /**
   * The insert world (W1/W2): net-94 trace T_own ending at the center,
   * foreign vertical trace X crossing at cx+1000, foreign via V at
   * (cx+600, cy); the forced polyline runs S=(cx,cy) -> E=(cx+2000,cy)
   * with withCheck=true. `tidyWidth` selects the W1/W2 face.
   */
  private static void runInsertWorld(byte[] bytes, String fileName, int tidyWidth)
      throws Exception {
    String label = tidyWidth > 0 ? "insert_tidy" : "insert_notidy";
    RoutingBoard board = parse(bytes, fileName);
    int net = net94(board);
    IntBox bbox = board.boundingBox;
    int cx = (bbox.ll.x + bbox.ur.x) / 2;
    int cy = (bbox.ll.y + bbox.ur.y) / 2;
    int foreignNet = 2;
    int thirdNet = 3;
    Padstack viaPadstack = addViaPadstack(board, label + "_via");

    PolylineTrace ownTrace =
        board.insertTraceWithoutCleaning(
            new Polyline(new IntPoint(cx - 2000, cy), new IntPoint(cx, cy)),
            0,
            100,
            new int[] {net},
            0,
            FixedState.UNFIXED);
    PolylineTrace crossTrace =
        board.insertTraceWithoutCleaning(
            new Polyline(new IntPoint(cx + 1000, cy - 2000), new IntPoint(cx + 1000, cy + 2000)),
            0,
            100,
            new int[] {foreignNet},
            0,
            FixedState.UNFIXED);
    Via corridorVia =
        board.insertVia(
            viaPadstack, new IntPoint(cx + 600, cy), new int[] {thirdNet}, 0,
            FixedState.UNFIXED, false);

    JsonObject worldRow = obj("world");
    worldRow.addProperty("name", label);
    worldRow.addProperty("net", net);
    worldRow.addProperty("foreignNet", foreignNet);
    worldRow.addProperty("thirdNet", thirdNet);
    worldRow.addProperty("ownTrace", ownTrace.getId());
    worldRow.addProperty("crossTrace", crossTrace.getId());
    worldRow.addProperty("corridorVia", corridorVia.getId());
    worldRow.addProperty("cx", cx);
    worldRow.addProperty("cy", cy);
    worldRow.addProperty("halfWidth", 100);
    worldRow.addProperty("layer", 0);
    worldRow.addProperty("maxRecursionDepth", 10);
    worldRow.addProperty("maxViaRecursionDepth", 10);
    worldRow.addProperty("maxSpringOverRecursionDepth", 2);
    worldRow.addProperty("tidyWidth", tidyWidth);
    worldRow.addProperty("pullTightAccuracy", 100);
    worldRow.addProperty("withCheck", true);
    row(worldRow);
    dumpInventory("before", board);
    dumpChangedArea("before", board);

    Point result =
        board.insertForcedTracePolyline(
            new Polyline(new IntPoint(cx, cy), new IntPoint(cx + 2000, cy)),
            100,
            0,
            new int[] {net},
            0,
            10,
            10,
            2,
            tidyWidth,
            100,
            true,
            null);
    JsonObject resultRow = obj("insert_result");
    resultRow.addProperty("world", label);
    resultRow.addProperty("result", result == null ? "null" : result.toString());
    row(resultRow);
    dumpChangedArea("after_insert", board);
    dumpInventory("after_insert", board);

    if (tidyWidth == 0) {
      // The connectToTrace ladder runs on the comparable (untightened)
      // board. c2 (the insert face) runs LAST: its tail removal
      // deletes the target trace.
      PolylineTrace target = findNet94TraceAt(board, new IntPoint(cx + 500, cy));
      // c1: the contains short-circuit.
      boolean c1 = board.connectToTrace(new IntPoint(cx + 500, cy), target, 100, 0);
      JsonObject c1Row = obj("connect");
      c1Row.addProperty("run", "c1_contains");
      c1Row.addProperty("fromX", cx + 500);
      c1Row.addProperty("fromY", cy);
      c1Row.addProperty("result", c1);
      row(c1Row);
      dumpInventory("after_c1", board);
      dumpChangedArea("after_c1", board);
      // c3: the check-blocked face (the corridor crosses the shoved
      // corridor via).
      boolean c3 = board.connectToTrace(new IntPoint(cx + 500, cy + 1200), target, 100, 0);
      JsonObject c3Row = obj("connect");
      c3Row.addProperty("run", "c3_blocked");
      c3Row.addProperty("fromX", cx + 500);
      c3Row.addProperty("fromY", cy + 1200);
      c3Row.addProperty("result", c3);
      row(c3Row);
      dumpInventory("after_c3", board);
      dumpChangedArea("after_c3", board);
      // c2: the insert face, LAST (tail removal deletes the target).
      boolean c2 = board.connectToTrace(new IntPoint(cx + 200, cy + 1200), target, 100, 0);
      JsonObject c2Row = obj("connect");
      c2Row.addProperty("run", "c2_insert");
      c2Row.addProperty("fromX", cx + 200);
      c2Row.addProperty("fromY", cy + 1200);
      c2Row.addProperty("result", c2);
      row(c2Row);
      dumpInventory("after_c2", board);
      dumpChangedArea("after_c2", board);
    }
  }

  /**
   * The fail world (W3): a straight corridor through an UNFIXED via
   * with maxViaRecursionDepth 0. The shove-loop check fails on the
   * via, the sampling retry re-fails -> the native fail + obstacle
   * rows; the world stays UNDAMAGED.
   */
  private static void runFailWorld(byte[] bytes, String fileName) throws Exception {
    RoutingBoard board = parse(bytes, fileName);
    int net = net94(board);
    IntBox bbox = board.boundingBox;
    int cx = (bbox.ll.x + bbox.ur.x) / 2;
    int cy = (bbox.ll.y + bbox.ur.y) / 2;
    int thirdNet = 3;
    Padstack viaPadstack = addViaPadstack(board, "fail_via");

    int sw = board.getMinTraceHalfWidth();
    int sampleWidth = 2 * sw;
    int length = 40 * sw;
    if (length <= sampleWidth) {
      length = 4 * sampleWidth; // degenerate-min guard; never hit on this fixture
    }
    int viaX = cx + sw + 300;
    Via blockingVia =
        board.insertVia(
            viaPadstack, new IntPoint(viaX, cy), new int[] {thirdNet}, 0,
            FixedState.UNFIXED, false);

    JsonObject worldRow = obj("world");
    worldRow.addProperty("name", "insert_fail");
    worldRow.addProperty("net", net);
    worldRow.addProperty("thirdNet", thirdNet);
    worldRow.addProperty("blockingVia", blockingVia.getId());
    worldRow.addProperty("cx", cx);
    worldRow.addProperty("cy", cy);
    worldRow.addProperty("minTraceHalfWidth", sw);
    worldRow.addProperty("sampleWidth", sampleWidth);
    worldRow.addProperty("length", length);
    worldRow.addProperty("viaX", viaX);
    worldRow.addProperty("halfWidth", 100);
    worldRow.addProperty("layer", 0);
    worldRow.addProperty("maxRecursionDepth", 10);
    worldRow.addProperty("maxViaRecursionDepth", 0);
    worldRow.addProperty("maxSpringOverRecursionDepth", 2);
    worldRow.addProperty("tidyWidth", 0);
    worldRow.addProperty("withCheck", true);
    row(worldRow);
    dumpInventory("before", board);
    dumpChangedArea("before", board);

    Point result =
        board.insertForcedTracePolyline(
            new Polyline(new IntPoint(cx, cy), new IntPoint(cx + length, cy)),
            100,
            0,
            new int[] {net},
            0,
            10,
            0,
            2,
            0,
            100,
            true,
            null);
    JsonObject resultRow = obj("insert_result");
    resultRow.addProperty("world", "insert_fail");
    resultRow.addProperty("result", result == null ? "null" : result.toString());
    row(resultRow);
    Item failing = board.getShoveFailingObstacle();
    JsonObject failRow = obj("failing_obstacle");
    failRow.addProperty(
        "obstacle",
        failing == null
            ? "null"
            : failing.getClass().getSimpleName() + "#" + failing.getId());
    row(failRow);
    dumpChangedArea("after_fail", board);
    dumpInventory("after_fail", board);
  }

  /**
   * The tails world (W4): floating net-94 stub A, net-95 via Vn + stub
   * C at the center-east, and a fanout via Vf on the SMD pin of N094.
   * One removeTraceTails(-1, option) call per fresh copy.
   */
  private static void runTailsWorld(byte[] bytes, String fileName, String optionName)
      throws Exception {
    RoutingBoard board = parse(bytes, fileName);
    int net = net94(board);
    int fifthNet = 96;
    IntBox bbox = board.boundingBox;
    int cx = (bbox.ll.x + bbox.ur.x) / 2;
    int cy = (bbox.ll.y + bbox.ur.y) / 2;
    Padstack viaPadstack = addViaPadstack(board, "tails_via");

    // the SMD pin of N094 (image CD -> single-layer)
    Pin pin94 = null;
    for (Item item : board.getItems()) {
      if (item instanceof Pin pin
          && pin.netNumbers.length == 1
          && pin.netNumbers[0] == net) {
        pin94 = pin;
        break;
      }
    }
    if (pin94 == null) {
      throw new IllegalStateException("no N094 pin found");
    }
    Point pinCenter = pin94.getCenter();
    if (!(pinCenter instanceof IntPoint pinCenterInt)) {
      throw new IllegalStateException("pin center not integral");
    }

    PolylineTrace stubA =
        board.insertTraceWithoutCleaning(
            new Polyline(new IntPoint(cx - 4000, cy), new IntPoint(cx - 2000, cy)),
            0,
            100,
            new int[] {net},
            0,
            FixedState.UNFIXED);
    Via nonFanoutVia =
        board.insertVia(
            viaPadstack, new IntPoint(cx + 3000, cy), new int[] {fifthNet}, 0,
            FixedState.UNFIXED, false);
    PolylineTrace stubC =
        board.insertTraceWithoutCleaning(
            new Polyline(new IntPoint(cx + 3000, cy), new IntPoint(cx + 4000, cy)),
            0,
            100,
            new int[] {fifthNet},
            0,
            FixedState.UNFIXED);
    Via fanoutVia =
        board.insertVia(
            viaPadstack,
            new IntPoint(pinCenterInt.x, pinCenterInt.y),
            new int[] {net},
            0,
            FixedState.UNFIXED,
            false);

    JsonObject worldRow = obj("world");
    worldRow.addProperty("name", "tails_" + optionName);
    worldRow.addProperty("net", net);
    worldRow.addProperty("fifthNet", fifthNet);
    worldRow.addProperty("stubA", stubA.getId());
    worldRow.addProperty("nonFanoutVia", nonFanoutVia.getId());
    worldRow.addProperty("stubC", stubC.getId());
    worldRow.addProperty("fanoutVia", fanoutVia.getId());
    worldRow.addProperty("pin94", pin94.getId());
    worldRow.addProperty("pin94Center", pinCenterInt.x + ":" + pinCenterInt.y);
    worldRow.addProperty(
        "pin94Layers", pin94.firstLayer() + ":" + pin94.lastLayer());
    worldRow.addProperty("option", optionName);
    row(worldRow);
    dumpInventory("before", board);

    Item.StopConnectionOption option =
        switch (optionName) {
          case "via" -> Item.StopConnectionOption.VIA;
          case "fanout_via" -> Item.StopConnectionOption.FANOUT_VIA;
          default -> Item.StopConnectionOption.NONE;
        };
    boolean result = board.removeTraceTails(-1, option);
    JsonObject resultRow = obj("remove_tails");
    resultRow.addProperty("option", optionName);
    resultRow.addProperty("result", result);
    row(resultRow);
    dumpInventory("after", board);
  }

  /**
   * THROWAWAY debug world: replays the W2 world, then performs the c2
   * connectToTrace steps MANUALLY with an inventory dump between them,
   * to ground the in-call id dance (which items consume 113/114 and
   * what survives as 115).
   */
  private static void runConnectDebugWorld(byte[] bytes, String fileName) throws Exception {
    RoutingBoard board = parse(bytes, fileName);
    int net = net94(board);
    IntBox bbox = board.boundingBox;
    int cx = (bbox.ll.x + bbox.ur.x) / 2;
    int cy = (bbox.ll.y + bbox.ur.y) / 2;
    Padstack viaPadstack = addViaPadstack(board, "c2dbg_via");
    PolylineTrace ownTrace =
        board.insertTraceWithoutCleaning(
            new Polyline(new IntPoint(cx - 2000, cy), new IntPoint(cx, cy)),
            0, 100, new int[] {net}, 0, FixedState.UNFIXED);
    PolylineTrace crossTrace =
        board.insertTraceWithoutCleaning(
            new Polyline(new IntPoint(cx + 1000, cy - 2000), new IntPoint(cx + 1000, cy + 2000)),
            0, 100, new int[] {2}, 0, FixedState.UNFIXED);
    board.insertVia(viaPadstack, new IntPoint(cx + 600, cy), new int[] {3}, 0,
        FixedState.UNFIXED, false);
    board.insertForcedTracePolyline(
        new Polyline(new IntPoint(cx, cy), new IntPoint(cx + 2000, cy)),
        100, 0, new int[] {net}, 0, 10, 10, 2, 0, 100, true, null);
    PolylineTrace target = findNet94TraceAt(board, new IntPoint(cx + 500, cy));
    dumpInventory("dbg_start", board);
    IntPoint from = new IntPoint(cx + 200, cy + 1200);
    LineSegment proj = target.polyline().projectionLine(from);
    Polyline conn = proj.toPolyline();
    JsonObject step = obj("c2dbg");
    step.addProperty("check", board.checkPolylineTrace(conn, 0, 100, target.netNumbers, 0));
    row(step);
    dumpInventory("dbg_after_check", board);
    board.insertTrace(conn, 0, 100, target.netNumbers, 0, FixedState.UNFIXED);
    dumpInventory("dbg_after_insertTrace", board);
    Point fc = target.firstCorner();
    Point lc = target.lastCorner();
    Trace tail1 = board.getTraceTail(fc, 0, target.netNumbers);
    JsonObject t1 = obj("c2dbg_tail1");
    t1.addProperty(
        "tail", tail1 == null ? "null" : tail1.getClass().getSimpleName() + "#" + tail1.getId());
    if (tail1 != null && !tail1.isUserFixed()) {
      board.removeItem(tail1);
    }
    row(t1);
    dumpInventory("dbg_after_tail1", board);
    Trace tail2 = board.getTraceTail(lc, 0, target.netNumbers);
    JsonObject t2 = obj("c2dbg_tail2");
    t2.addProperty(
        "tail", tail2 == null ? "null" : tail2.getClass().getSimpleName() + "#" + tail2.getId());
    if (tail2 != null && !tail2.isUserFixed()) {
      board.removeItem(tail2);
    }
    row(t2);
    dumpInventory("dbg_after_tail2", board);
  }

  /**
   * FIX-ROUND world (spec-review MINOR-1): the sampling-retry SHORTEN
   * arm, observed on SUCCESS. A straight 400000 corridor through an
   * UNFIXED via at cx+60000 with maxViaRecursionDepth 0: the shove-loop
   * check fails at shape 0 (lastShapeNo=0 < traceShapes.length=1), the
   * lastSegmentLength (400000) lies strictly between sampleWidth
   * (20000) and 100*sampleWidth (2000000), so the shorten arm fires and
   * the SHORTENED corridor (500000 -> ~520000, clear of the via) passes
   * the re-check and INSERTS — the returned newCorner is the sampled
   * IntPoint, not the toCorner (unlike W3, where the re-check failed and
   * discarded it).
   */
  private static void runRetryWorld(byte[] bytes, String fileName) throws Exception {
    RoutingBoard board = parse(bytes, fileName);
    int net = net94(board);
    IntBox bbox = board.boundingBox;
    int cx = (bbox.ll.x + bbox.ur.x) / 2;
    int cy = (bbox.ll.y + bbox.ur.y) / 2;
    int thirdNet = 3;
    Padstack viaPadstack = addViaPadstack(board, "retry_via");

    int sw = board.getMinTraceHalfWidth();
    int sampleWidth = 2 * sw;
    int length = 40 * sw;
    int viaX = cx + 6 * sw; // 560000: inside the corridor, far beyond
    // the sampleWidth strip the shorten arm keeps
    Via blockingVia =
        board.insertVia(
            viaPadstack, new IntPoint(viaX, cy), new int[] {thirdNet}, 0,
            FixedState.UNFIXED, false);

    JsonObject worldRow = obj("world");
    worldRow.addProperty("name", "insert_retry");
    worldRow.addProperty("net", net);
    worldRow.addProperty("thirdNet", thirdNet);
    worldRow.addProperty("blockingVia", blockingVia.getId());
    worldRow.addProperty("cx", cx);
    worldRow.addProperty("cy", cy);
    worldRow.addProperty("minTraceHalfWidth", sw);
    worldRow.addProperty("sampleWidth", sampleWidth);
    worldRow.addProperty("length", length);
    worldRow.addProperty("viaX", viaX);
    worldRow.addProperty("halfWidth", 100);
    worldRow.addProperty("layer", 0);
    worldRow.addProperty("maxRecursionDepth", 10);
    worldRow.addProperty("maxViaRecursionDepth", 0);
    worldRow.addProperty("maxSpringOverRecursionDepth", 2);
    worldRow.addProperty("tidyWidth", 0);
    worldRow.addProperty("withCheck", true);
    row(worldRow);
    dumpInventory("before", board);
    dumpChangedArea("before", board);

    Point result =
        board.insertForcedTracePolyline(
            new Polyline(new IntPoint(cx, cy), new IntPoint(cx + length, cy)),
            100,
            0,
            new int[] {net},
            0,
            10,
            0,
            2,
            0,
            100,
            true,
            null);
    JsonObject resultRow = obj("insert_result");
    resultRow.addProperty("world", "insert_retry");
    resultRow.addProperty("result", result == null ? "null" : result.toString());
    row(resultRow);
    Item failing = board.getShoveFailingObstacle();
    JsonObject failRow = obj("failing_obstacle");
    failRow.addProperty(
        "obstacle",
        failing == null
            ? "null"
            : failing.getClass().getSimpleName() + "#" + failing.getId());
    row(failRow);
    dumpChangedArea("after_retry", board);
    dumpInventory("after_retry", board);
  }

  /**
   * FIX-ROUND world (spec-review MINOR-2): the forced_pad / drill-join
   * RUN faces under a LIVE marking session. A single-layer via at the
   * board center is moved +4000 in x by DrillItemMover.insert (Java
   * :110-167) with the session started: forcedPad must shove the
   * foreign N002 vertical trace at cx+4000 aside (its substitute-piece
   * corners join, ForcedPadRouter :434-436, then normalize with the
   * live clip :440-450), and the four corners of the UNTRANSLATED
   * +-250 box join per layer (:159-162). The two join webs sit ~4000
   * apart, so the pinned after_move octagon discriminates either join
   * dropped INDEPENDENTLY.
   */
  private static void runDrillMoveWorld(byte[] bytes, String fileName) throws Exception {
    RoutingBoard board = parse(bytes, fileName);
    IntBox bbox = board.boundingBox;
    int cx = (bbox.ll.x + bbox.ur.x) / 2;
    int cy = (bbox.ll.y + bbox.ur.y) / 2;
    int foreignNet = 2;
    int thirdNet = 3;
    Padstack movePadstack =
        board.library.padstacks.add(
            new IntBox(new IntPoint(-250, -250), new IntPoint(250, 250)), 0, 0);
    Via movedVia =
        board.insertVia(
            movePadstack, new IntPoint(cx, cy), new int[] {thirdNet}, 0,
            FixedState.UNFIXED, false);
    PolylineTrace crossTrace =
        board.insertTraceWithoutCleaning(
            new Polyline(new IntPoint(cx + 4000, cy - 3000), new IntPoint(cx + 4000, cy + 3000)),
            0,
            100,
            new int[] {foreignNet},
            0,
            FixedState.UNFIXED);
    board.startMarkingChangedArea();

    JsonObject worldRow = obj("world");
    worldRow.addProperty("name", "drill_move");
    worldRow.addProperty("foreignNet", foreignNet);
    worldRow.addProperty("thirdNet", thirdNet);
    worldRow.addProperty("movedVia", movedVia.getId());
    worldRow.addProperty("crossTrace", crossTrace.getId());
    worldRow.addProperty("cx", cx);
    worldRow.addProperty("cy", cy);
    worldRow.addProperty("dx", 4000);
    worldRow.addProperty("halfWidth", 100);
    worldRow.addProperty("layer", 0);
    worldRow.addProperty("maxRecursionDepth", 10);
    worldRow.addProperty("maxViaRecursionDepth", 10);
    worldRow.addProperty("withSession", true);
    row(worldRow);
    dumpInventory("before_move", board);
    dumpChangedArea("before_move", board);

    boolean ok = DrillItemMover.insert(movedVia, new IntVector(4000, 0), 10, 10, null, board);
    JsonObject resultRow = obj("move_result");
    resultRow.addProperty("world", "drill_move");
    resultRow.addProperty("result", ok);
    row(resultRow);
    dumpChangedArea("after_move", board);
    dumpInventory("after_move", board);
  }

  /**
   * QUALITY-REVIEW world (M-1): the normalize-true branch —
   * splitTracesAtKeepPoint + the post-split re-pick (Java :809-833) —
   * observed on a REAL keep-point split. Seeds, both net 94:
   * a vertical trace crossing the corridor MIDDLE at cx+2000 (forces
   * normalize true: split_clip found-splits the crossing and own-splits
   * the inserted trace, pieces != 1), and a COLLINEAR continuation
   * from the corridor end cx+4000 to cx+8000 (the :756 combine merges
   * it, so the keep point lands in the merged trace's INTERIOR and the
   * keep-point split is a real 2-piece split, not an end no-op). The
   * re-pick then runs over the split pieces. tidyWidth stays 0 so Java
   * performs no pull-tight on the re-picked trace (the M4 seam arm) and
   * the final geometry is seam-independent.
   */
  private static void runKeepPointWorld(byte[] bytes, String fileName) throws Exception {
    RoutingBoard board = parse(bytes, fileName);
    int net = net94(board);
    IntBox bbox = board.boundingBox;
    int cx = (bbox.ll.x + bbox.ur.x) / 2;
    int cy = (bbox.ll.y + bbox.ur.y) / 2;
    PolylineTrace crossingTrace =
        board.insertTraceWithoutCleaning(
            new Polyline(new IntPoint(cx + 2000, cy - 3000), new IntPoint(cx + 2000, cy + 3000)),
            0, 100, new int[] {net}, 0, FixedState.UNFIXED);
    PolylineTrace continuationTrace =
        board.insertTraceWithoutCleaning(
            new Polyline(new IntPoint(cx + 4000, cy), new IntPoint(cx + 8000, cy)),
            0, 100, new int[] {net}, 0, FixedState.UNFIXED);

    JsonObject worldRow = obj("world");
    worldRow.addProperty("name", "keep_point");
    worldRow.addProperty("net", net);
    worldRow.addProperty("crossingTrace", crossingTrace.getId());
    worldRow.addProperty("continuationTrace", continuationTrace.getId());
    worldRow.addProperty("cx", cx);
    worldRow.addProperty("cy", cy);
    worldRow.addProperty("halfWidth", 100);
    worldRow.addProperty("layer", 0);
    worldRow.addProperty("maxRecursionDepth", 10);
    worldRow.addProperty("maxViaRecursionDepth", 0);
    worldRow.addProperty("maxSpringOverRecursionDepth", 2);
    worldRow.addProperty("tidyWidth", 0);
    worldRow.addProperty("withCheck", true);
    row(worldRow);
    dumpInventory("before", board);
    dumpChangedArea("before", board);

    Point result =
        board.insertForcedTracePolyline(
            new Polyline(new IntPoint(cx, cy), new IntPoint(cx + 4000, cy)),
            100,
            0,
            new int[] {net},
            0,
            10,
            0,
            2,
            0,
            100,
            true,
            null);
    JsonObject resultRow = obj("insert_result");
    resultRow.addProperty("world", "keep_point");
    resultRow.addProperty("result", result == null ? "null" : result.toString());
    row(resultRow);
    dumpChangedArea("after", board);
    dumpInventory("after", board);
  }

  public static void main(String[] args) throws Exception {
    byte[] bytes = Files.readAllBytes(Paths.get(args[0]));
    String fileName = args.length > 1 ? args[1] : "t9_locator45.dsn";
    FRLogger.granularTraceEnabled = true;
    // The five-arg trace path filters through DebugControl ->
    // Freerouting.globalSettings; a bare probe JVM has none.
    if (app.freerouting.Freerouting.globalSettings == null) {
      app.freerouting.Freerouting.globalSettings =
          new app.freerouting.settings.GlobalSettings();
    }
    attachNativeTap();
    runInsertWorld(bytes, fileName, 400);
    runInsertWorld(bytes, fileName, 0);
    runFailWorld(bytes, fileName);
    runTailsWorld(bytes, fileName, "none");
    runTailsWorld(bytes, fileName, "via");
    runTailsWorld(bytes, fileName, "fanout_via");
    if (args.length > 2 && args[2].equals("c2debug")) {
      runConnectDebugWorld(bytes, fileName);
    } else if (args.length > 2 && args[2].equals("fixrun")) {
      runRetryWorld(bytes, fileName);
      runDrillMoveWorld(bytes, fileName);
    } else if (args.length > 2 && args[2].equals("m1run")) {
      runKeepPointWorld(bytes, fileName);
    }
  }
}
