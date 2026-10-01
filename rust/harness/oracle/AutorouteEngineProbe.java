// AutorouteEngineProbe.java — the M3-T11 jar-side probe oracle for the
// CONNECTION-ROUTING ENGINE assembly (AutorouteEngine.autorouteConnection
// + RoutingBoard.initAutoroute/finishAutoroute + the checkTraceSegment
// search facade). ForcedInsertProbe house pattern: ONE JVM per run,
// deterministic JSONL rows on stdout.
//
// Build/run (JDK 25, from the repo root):
//   mkdir -p /tmp/epic-t11-classes && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/javac \
//       -cp build/libs/freerouting-current-executable.jar \
//       -d /tmp/epic-t11-classes rust/harness/oracle/AutorouteEngineProbe.java && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/java \
//       -cp build/libs/freerouting-current-executable.jar:/tmp/epic-t11-classes \
//       app.freerouting.autoroute.maze.AutorouteEngineProbe \
//       rust/harness/fixtures/locator-spike/t9_locator45.dsn
//
// The probe declares app.freerouting.autoroute.maze (AutorouteEngine's
// package) so package-private seams stay reachable.
//
// ORACLE-NATIVE rows: the jar itself emits the whole T11 event-row
// surface through FRLogger — compare_trace_maze_result_raw (the
// {33,66,67} gate), compare_trace_connection_item_raw,
// compare_trace_insert_segment_raw + the structured five-arg twin,
// compare_trace_insert_segment_ids, compare_trace_stub_found/cleanup,
// compare_trace_insert_forced_sub (including the
// step=before_pull_tight / step=after_pull_tight pair per inserted
// segment — the TIGHTENER-STABILITY evidence), and FANOUT_DIAG. The
// log4j tap re-emits every message carrying one of the tokens, in
// emission order. FRLogger.granularTraceEnabled = true + a bare
// GlobalSettings make the five-arg rows fire.
//
// Worlds (fresh parse each; the routing net is selected by NUMBER —
// net number 94 is the native fail-row gate net of this fixture):
//   net_pin_info                  — for numbers 33/66/67/94: the pin
//                                   carrying the number (name/id/
//                                   center) — resolves the fixture's
//                                   name-to-number mapping on record.
//   route_routed                  — pin -> seeded corridor anchor
//                                   (identical to the Rust end-to-end
//                                   world), initAutoroute retain=true,
//                                   autorouteConnection; attempt row +
//                                   full inventory canon after.
//   route_fail_no_connection      — pin -> pin (degenerate), TWO
//                                   attempts on the same engine: the
//                                   FAILED row + the cleanup-is-
//                                   repeatable invariant.
//   route_fail_layers_disabled    — ctrl.layerActive[0] = false: the
//                                   actual jar verdict (starvation vs
//                                   the late layers-disabled gate).
//   route_maze_row                — the same corridor world for the
//                                   first routable gate net of
//                                   {33, 66, 67}: fires the
//                                   compare_trace_maze_result_raw row.
//   route_straight94              — the STRAIGHT-RUN canon world: pin
//                                   -> vertical stub north of the pin;
//                                   pull-tight-stable by design, so
//                                   the full board canon (ids +
//                                   geometry) is Java-identical.
//   route_debug49                 — the full route on net 49 (the
//                                   debugNet49 bisect instrument; this
//                                   world's maze init FAILS on the
//                                   fixture — the jar's init-failure
//                                   details literal, the banked
//                                   message-collapse witness).
//   pulltight0/1                  — the pull-tight attribution worlds:
//                                   the Rust NoPullTight staircases
//                                   seeded as net-49 traces and run
//                                   through the insert tail's exact
//                                   tightener construction.
//   splitcombine                  — the P1-over-anchor split/combine
//                                   bisect world (net 49 debug rows).
//   ctrl_costs                    — the cost table the jar maze runs
//                                   with (RouterSettings(board)).
//   check_segment                 — direct checkTraceSegment calls:
//                                   clear segment (MAX_VALUE), crossing
//                                   an UNFIXED foreign wall
//                                   (flag=false clips, flag=true
//                                   ignores), crossing a USER_FIXED
//                                   wall (both clip).
//
// Determinism: inventory rows sort by id; doubles are Double.toString;
// identity-hash tokens normalized to Class#ordinal by first appearance;
// single-element item sets keep describeConnection order-safe; no
// HashSet iteration reaches a row.
package app.freerouting.autoroute.maze;

import app.freerouting.autoroute.AutorouteAttemptResult;
import app.freerouting.board.actions.ItemIdGenerator;

import app.freerouting.board.facade.RoutingBoard;
import app.freerouting.board.model.items.Item;
import app.freerouting.board.model.items.Pin;
import app.freerouting.board.model.structure.FixedState;
import app.freerouting.board.optimize.TraceTightener;
import app.freerouting.board.trace.PolylineTrace;
import app.freerouting.geometry.planar.IntPoint;
import app.freerouting.geometry.planar.Polyline;
import app.freerouting.io.BoardReadResult;
import app.freerouting.io.specctra.DsnReader;
import app.freerouting.logger.FRLogger;
import app.freerouting.settings.RouterSettings;
import com.google.gson.Gson;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import java.io.ByteArrayInputStream;
import java.nio.file.Files;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeSet;
import org.apache.logging.log4j.Level;
import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.core.LogEvent;
import org.apache.logging.log4j.core.LoggerContext;
import org.apache.logging.log4j.core.appender.AbstractAppender;
import org.apache.logging.log4j.core.config.Property;

public class AutorouteEngineProbe {

  private static final Gson GSON = new Gson();

  /**
   * The T11 row tokens (AutorouteEngine / FoundConnectionInserter /
   * RoutingBoard emissions; "compare_trace_insert_segment" covers both
   * the raw row and the structured "[...]" twin, as
   * "compare_trace_stub_cleanup" covers its twin).
   */
  private static final String[] TAP_TOKENS = {
    "compare_trace_maze_result_raw",
    "compare_trace_connection_item_raw",
    "compare_trace_insert_segment",
    "compare_trace_stub_found",
    "compare_trace_stub_cleanup",
    "compare_trace_insert_forced_sub",
    "compare_trace_shove_shape",
    "compare_trace_insert_forced_fail",
    "compare_trace_insert_forced_obstacle",
    "compare_trace_normalize_net49",
    "compare_trace_combine_at_start_net49",
    "FANOUT_DIAG"
  };

  private static boolean isTappedRow(String msg) {
    for (String token : TAP_TOKENS) {
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

  /** The log4j tap for the native rows (ForcedInsertProbe wiring). */
  private static void attachNativeTap() {
    LoggerContext ctx = (LoggerContext) LogManager.getContext(false);
    AbstractAppender tap =
        new AbstractAppender("autoroute-engine-tap", null, null, true, Property.EMPTY_ARRAY) {
          @Override
          public void append(LogEvent event) {
            String m = event.getMessage().getFormattedMessage();
            if (m != null && isTappedRow(m)) {
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
    RoutingBoard board = (RoutingBoard) success.board();
    board.searchTreeManager.reinsertTreeItems();
    return board;
  }

  /** The first pin carrying the net number, or null. */
  private static Pin findPin(RoutingBoard board, int net) {
    for (Item item : board.getItems()) {
      if (item instanceof Pin pin && pin.containsNet(net)) {
        return pin;
      }
    }
    return null;
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
      if (item instanceof app.freerouting.board.model.items.DrillItem drill) {
        // Via canon: the stored center (the layer-change junction).
        it.addProperty("center", drill.getCenter().toString());
      }
      if (item instanceof PolylineTrace trace) {
        it.addProperty("layer", trace.getLayer());
        JsonArray corners = new JsonArray();
        for (int i = 0; i < trace.polyline().lines.length - 1; i++) {
          var c = trace.polyline().cornerApprox(i);
          JsonArray pt = new JsonArray();
          pt.add(Double.toString(c.x));
          pt.add(Double.toString(c.y));
          corners.add(pt);
        }
        it.add("corners", corners);
      }
      items.add(it);
    }
    JsonObject out = obj("inventory");
    out.addProperty("phase", phase);
    out.add("items", items);
    row(out);
  }

  /**
   * Seeds the corridor anchor ((600000,300000)-(620000,300000), layer
   * 0, half width 1500, class 0 — the Rust end-to-end world's anchor).
   */
  private static PolylineTrace seedAnchor(RoutingBoard board, int net) {
    return board.insertTraceWithoutCleaning(
        new Polyline(new IntPoint(600000, 300000), new IntPoint(620000, 300000)),
        0,
        1500,
        new int[] {net},
        0,
        FixedState.UNFIXED);
  }

  /** The net-pin info rows: which pin carries each gate/net number. */
  private static void runNetPinInfos(RoutingBoard board) {
    int[] nets = {33, 66, 67, 94};
    for (int net : nets) {
      Pin pin = findPin(board, net);
      JsonObject out = obj("net_pin_info");
      out.addProperty("net", net);
      if (pin == null) {
        out.addProperty("present", false);
      } else {
        out.addProperty("present", true);
        out.addProperty("pin_id", pin.getId());
        out.addProperty("center", pin.getCenter().toString());
        out.addProperty("component_id", pin.getComponentId());
      }
      row(out);
    }
  }

  /**
   * The ctrl cost table the jar's maze actually runs with: the
   * 3-arg AutorouteControl over `new RouterSettings(board)` —
   * per-layer horizontal/vertical trace costs (the geometry-derived
   * preferred/undesired pair), bend costs, layer activity, via costs.
   */
  private static void runCtrlCosts(RoutingBoard board, int net) throws Exception {
    AutorouteControl ctrl = new AutorouteControl(board, net, new RouterSettings(board));
    JsonObject out = obj("ctrl_costs");
    out.addProperty("net", net);
    JsonArray layers = new JsonArray();
    for (int i = 0; i < ctrl.layerCount; i++) {
      JsonObject layer = new JsonObject();
      layer.addProperty("horizontal", Double.toString(ctrl.traceCosts[i].horizontal()));
      layer.addProperty("vertical", Double.toString(ctrl.traceCosts[i].vertical()));
      layer.addProperty("bend", Double.toString(ctrl.bendCosts[i]));
      layer.addProperty("active", ctrl.layerActive[i]);
      layers.add(layer);
    }
    out.add("layers", layers);
    out.addProperty("via_costs", ctrl.settings.getViaCosts());
    out.addProperty("vias_allowed", ctrl.viasAllowed);
    out.addProperty("automatic_neckdown", ctrl.withNeckdown);
    row(out);
  }

  /**
   * The split/combine bisect world: the T11 layer-0 leg's FIRST
   * per-segment insert (P1 (610000,300000)→(521250,300000)
   * overlapping the seeded anchor) replayed standalone on net 49 so
   * the jar's own debugNet49 normalize/combine rows fire. The exact
   * per-segment args of FoundConnectionInserter (:176-190).
   */
  private static void runSplitCombineWorld(byte[] bytes, String fileName) throws Exception {
    RoutingBoard board = parse(bytes, fileName);
    board.insertTraceWithoutCleaning(
        new Polyline(new IntPoint(600000, 300000), new IntPoint(620000, 300000)),
        0,
        1500,
        new int[] {49},
        0,
        FixedState.UNFIXED);
    dumpInventory("splitcombine:seeded", board);
    app.freerouting.geometry.planar.Point ok =
        board.insertForcedTracePolyline(
            new Polyline(new IntPoint(610000, 300000), new IntPoint(521250, 300000)),
            1500,
            0,
            new int[] {49},
            0,
            20,
            5,
            5,
            Integer.MAX_VALUE,
            500,
            true,
            null);
    JsonObject out = obj("splitcombine_result");
    out.addProperty("okPoint", ok == null ? "null" : ok.toString());
    row(out);
    dumpInventory("splitcombine:done", board);
  }

  /**
   * The pull-tight attribution world: seed the Rust engine's raw
   * (NoPullTight) staircase geometries for net 94's layer-0 and
   * layer-1 legs as net-49 traces, then run the EXACT tightener
   * construction of the insert tail (RoutingBoard
   * insertForcedTracePolyline: TraceTightener.getInstance with an
   * empty net array (maxRecursionDepth 20 > 0), null clip shape
   * (tidyWidth = MAX_VALUE), minTranslateDist 500, keep point = the
   * trace's last corner) and dump the inventory. Verdict answers: does
   * the jar's own tightener collapse the raw staircases into the
   * route_routed:done canon geometries?
   */
  private static void runPullTightWorld(byte[] bytes, String fileName) throws Exception {
    int[][] staircases = {
      // the Rust layer-1 leg (trace 121): east run + 45-degree diagonal + vertical
      {480738, 309375, 502752, 309375, 662957, 149170, 662957, 29188},
      // the Rust layer-0 leg (trace 115): west run + jitter + diagonal
      {620000, 300000, 521250, 300000, 521248, 300002, 516679, 300002,
       516664, 300017, 490096, 300017, 480738, 309375},
    };
    int[] layers = {1, 0};
    for (int world = 0; world < staircases.length; world++) {
      RoutingBoard board = parse(bytes, fileName);
      int layer = layers[world];
      int[] xy = staircases[world];
      IntPoint[] corners = new IntPoint[xy.length / 2];
      for (int i = 0; i < corners.length; i++) {
        corners[i] = new IntPoint(xy[2 * i], xy[2 * i + 1]);
      }
      PolylineTrace trace =
          board.insertTraceWithoutCleaning(
              new Polyline(corners), layer, 1500, new int[] {49}, 0, FixedState.UNFIXED);
      dumpInventory("pulltight" + world + ":seeded", board);
      TraceTightener algo =
          TraceTightener.getInstance(
              board, new int[0], null, 500, null, -1, corners[corners.length - 1], layer);
      trace.pullTight(algo);
      dumpInventory("pulltight" + world + ":done", board);
    }
  }

  /**
   * The connection-routing world: fresh parse, seed anchor, init,
   * autorouteConnection (twice when repeat is set), attempt rows +
   * inventory canon. The anchor is the y=300000 corridor stub unless
   * {@code straightAnchor} is set, in which case it is the vertical
   * stub immediately north of net 94's pin — the STRAIGHT-RUN world
   * (dispatch: "adjust the fixture (straight runs, generous
   * clearances) until stable"): a forced-vertical optimal path with
   * no keepout interaction, where pull-tight must be a no-op and the
   * full board canon (ids + geometry) becomes Java-identical.
   */
  private static void runRouteWorld(
      byte[] bytes,
      String fileName,
      String world,
      int net,
      boolean degenerate,
      boolean disableLayer0,
      boolean repeat,
      boolean straightAnchor)
      throws Exception {
    RoutingBoard board = parse(bytes, fileName);
    Pin pin = findPin(board, net);
    if (pin == null) {
      throw new IllegalStateException(world + ": no pin for net " + net);
    }
    PolylineTrace anchor =
        degenerate
            ? null
            : straightAnchor
                ? board.insertTraceWithoutCleaning(
                    new Polyline(new IntPoint(663500, 26000), new IntPoint(663500, 28000)),
                    0,
                    1500,
                    new int[] {net},
                    0,
                    FixedState.UNFIXED)
                : seedAnchor(board, net);
    dumpInventory(world + ":seeded", board);

    AutorouteControl ctrl = new AutorouteControl(board, net, new RouterSettings(board));
    ctrl.removeUnconnectedVias = false;
    if (disableLayer0) {
      ctrl.layerActive[0] = false;
    }
    AutorouteEngine engine =
        board.initAutoroute(net, ctrl.traceClearanceClassIndex, null, null, true);

    Set<Item> startSet = new HashSet<>(List.of(pin));
    Set<Item> destSet = degenerate ? startSet : new HashSet<>(List.of((Item) anchor));
    for (int attempt = 1; attempt <= (repeat ? 2 : 1); attempt++) {
      AutorouteAttemptResult result =
          engine.autorouteConnection(
              startSet, destSet, ctrl, new TreeSet<>(), new HashMap<>());
      JsonObject out = obj("attempt");
      out.addProperty("world", world);
      out.addProperty("attempt", attempt);
      out.addProperty("state", result.state.toString());
      out.addProperty("details", result.details);
      row(out);
    }
    board.finishAutoroute();
    dumpInventory(world + ":done", board);
  }

  /**
   * The checkTraceSegment worlds: a clear segment, a crossing of an
   * UNFIXED foreign wall (flag discriminates), a crossing of a
   * USER_FIXED foreign wall (both flags clip).
   */
  private static void runCheckSegmentWorld(byte[] bytes, String fileName) throws Exception {
    RoutingBoard board = parse(bytes, fileName);
    // The unfixed wall at x=505000, the fixed wall at x=545000 (both
    // crossing y=300000 where the fixture's F.Cu keepout gap is clear).
    board.insertTraceWithoutCleaning(
        new Polyline(new IntPoint(505000, 100000), new IntPoint(505000, 500000)),
        0,
        1000,
        new int[] {2},
        0,
        FixedState.UNFIXED);
    board.insertTraceWithoutCleaning(
        new Polyline(new IntPoint(545000, 100000), new IntPoint(545000, 500000)),
        0,
        1000,
        new int[] {2},
        0,
        FixedState.USER_FIXED);
    int[] nets = {94};
    String[][] probes = {
      {"S0_clear_false", "480000", "300000", "490000", "300000", "false"},
      {"S1_unfixed_false", "500000", "300000", "520000", "300000", "false"},
      {"S1_unfixed_true", "500000", "300000", "520000", "300000", "true"},
      {"S2_fixed_false", "535000", "300000", "555000", "300000", "false"},
      {"S2_fixed_true", "535000", "300000", "555000", "300000", "true"},
    };
    for (String[] probe : probes) {
      double value =
          board.checkTraceSegment(
              new IntPoint(Integer.parseInt(probe[1]), Integer.parseInt(probe[2])),
              new IntPoint(Integer.parseInt(probe[3]), Integer.parseInt(probe[4])),
              0,
              nets,
              1500,
              0,
              Boolean.parseBoolean(probe[5]));
      JsonObject out = obj("check_segment");
      out.addProperty("probe", probe[0]);
      out.addProperty("value", Double.toString(value));
      out.addProperty("max_value", value >= (double) Integer.MAX_VALUE);
      row(out);
    }
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

    // World: net-pin info (number -> pin mapping, on record).
    RoutingBoard infoBoard = parse(bytes, fileName);
    runNetPinInfos(infoBoard);
    // World: the ctrl cost table (mirror this in the Rust worlds).
    runCtrlCosts(parse(bytes, fileName), 94);
    // World: the P1-over-anchor split/combine bisect (net 49 debug).
    runSplitCombineWorld(bytes, fileName);

    // World: the routed end-to-end canon.
    runRouteWorld(bytes, fileName, "route_routed", 94, false, false, false, false);
    // World: the degenerate FAILED (twice — cleanup repeatability).
    runRouteWorld(bytes, fileName, "route_fail_no_connection", 94, true, false, true, false);
    // World: layer 0 disabled — starvation vs the late gate.
    runRouteWorld(bytes, fileName, "route_fail_layers_disabled", 94, false, true, false, false);
    // World: the maze-result row for the first routable gate net.
    for (int gateNet : new int[] {33, 66, 67}) {
      if (findPin(parse(bytes, fileName), gateNet) != null) {
        runRouteWorld(bytes, fileName, "route_maze_row_net" + gateNet, gateNet, false, false, false, false);
        break;
      }
    }
    // World: the full route on net 49 — the debugNet49 rows fire for
    // EVERY normalize/combine of the route (the T11 bisect instrument).
    runRouteWorld(bytes, fileName, "route_debug49", 49, false, false, false, false);
    // World: the STRAIGHT-RUN canon (tightener-stable by design — the
    // dispatch's mandated fixture-adjustment arm).
    runRouteWorld(bytes, fileName, "route_straight94", 94, false, false, false, true);
    // World: pull-tight attribution — does the jar tightener collapse
    // the Rust NoPullTight staircases into the route canon geometries?
    runPullTightWorld(bytes, fileName);
    // World: the checkTraceSegment literals.
    runCheckSegmentWorld(bytes, fileName);
  }
}
