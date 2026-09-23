// TraceTightenerProbe.java — the M4-T3 jar-side probe oracle for the
// pull-tight layer (RoutingBoard.optChangedArea -> TraceTightener /
// TraceTightener90). ForcedInsertProbe house pattern: ONE JVM per run,
// deterministic JSONL rows on stdout.
//
// Build/run (JDK 25, from the repo root):
//   mkdir -p /tmp/epic-t3-classes && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/javac \
//       -cp build/libs/freerouting-current-executable.jar \
//       -d /tmp/epic-t3-classes rust/harness/oracle/TraceTightenerProbe.java && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/java \
//       -cp build/libs/freerouting-current-executable.jar:/tmp/epic-t3-classes \
//       app.freerouting.board.facade.TraceTightenerProbe \
//       rust/harness/fixtures/trace-tightener/tightener90.dsn
//
// The probe declares app.freerouting.board.facade (RoutingBoard's
// package) so package-private seams stay reachable.
//
// Worlds (fresh parse each, all on the 90-degree fixture
// tightener90.dsn; the call is the production-consumption shape
// optChangedArea(new int[0], null, 100, null, null, 0) — empty nets,
// null clip = unbounded, accuracy 100 = the >= 100 clamp face, null
// costs (the fixture has no vias, the via arm is dead), null stopper,
// timeLimit 0 = no budget so the jar witness is deterministic):
//
//   stair      — the 6-corner orthogonal staircase (net N1): joins its
//                six corners; the reposition + skip-segments ladder
//                collapses it; jar-witnessed before/after geometry
//                (fixpoint-termination + reposition witness)
//   acid       — the straight net-N3 trace FROM its own-net pin PA
//                (component CA at its west end) crossed by the foreign
//                net-N4 trace: joins T4's two corners. Java's
//                avoidAcidTraps is `if (true) return polyline;` (dead
//                body) so T4 must stay EXACTLY 2-corner unchanged —
//                the identity witness whose mutant (activated body)
//                wraps the trace around the crossing
//   region_in  — a single join point on the N5 C-cup's bottom-arm
//                centerline (y=180000), 6000 DBU west of its west end
//                segment edge: the changed-area octagon enlarged by
//                1.5 * (clearance + 2 * halfwidth)
//                = 1.5 * (2500 + 2000) = 6750 DBU reaches it (the
//                measured reach boundary is x* = 992250 = the box west
//                edge 999000 - 6750, see the regionscan evidence), so
//                the cup is processed and collapses to the west
//                vertical bar (1000000,200000)-(1000000,180000) — the
//                region-enlargement arithmetic witness (the mutant
//                dropping the 2 * halfwidth term reaches only
//                994000 + 3750 = 997750 < 999000 and misses by 1250)
//   region_out — the same cup from 14000 DBU away: 985000 + 6750
//                = 991750 < 999000 — the cup must stay UNTOUCHED (the
//                control arm)
//   block      — two nested C-cups: blocker A (net N7, lower id —
//                wired first) sits with its middle vertical at x=48000
//                exactly where blocked B (net N6, higher id — wired
//                second) wants to collapse. Descending-id processing
//                runs B FIRST: bisection creeps B toward A's east edge
//                (min-translate granularity 100), then A fully
//                collapses, and the SECOND sweep — possible only if
//                the fixpoint re-marks and re-runs — collapses B to
//                x=48000. Jar-witnessed before/after for BOTH traces
//                (the early-exit-after-one-sweep mutant leaves B
//                stranded near x~48450)
//
// Joins: every world starts its own changed-area session and joins the
// plan points on layer 0 — the exact surface the production
// Route/BatchAutorouter callers drive through the pull-tight seam.
//
// Determinism: inventory rows sort by id; doubles are Double.toString;
// no wall-clock, no hash iteration reaches a row.
package app.freerouting.board.facade;

import app.freerouting.autoroute.maze.AutorouteControl.ExpansionCostFactor;
import app.freerouting.board.actions.ItemIdGenerator;
import app.freerouting.board.model.items.Item;
import app.freerouting.board.model.items.Via;
import app.freerouting.board.optimize.ViaOptimizer;
import app.freerouting.board.trace.PolylineTrace;
import app.freerouting.geometry.planar.FloatPoint;
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
import java.util.List;

public class TraceTightenerProbe {

  private static final Gson GSON = new Gson();

  private static synchronized void row(JsonObject o) {
    System.out.println(GSON.toJson(o));
  }

  private static JsonObject obj(String type) {
    JsonObject o = new JsonObject();
    o.addProperty("type", type);
    return o;
  }

  /** A fresh parse of the fixture bytes (ForcedInsertProbe pattern). */
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

  private static void dumpInventory(String phase, RoutingBoard board) {
    JsonArray items = new JsonArray();
    List<Item> sorted = new ArrayList<>(board.getItems());
    sorted.sort(Comparator.comparingInt(Item::getId));
    for (Item item : sorted) {
      JsonObject it = new JsonObject();
      it.addProperty("id", item.getId());
      it.addProperty("kind", item.getClass().getSimpleName());
      it.addProperty("nets", java.util.Arrays.toString(item.netNumbers));
      if (item instanceof PolylineTrace trace) {
        // T6 extension: the trace fixed state — the swap arm's
        // fixed-state transfer (contactTrace.setFixedState before the
        // combine) is only visible with it on the rows.
        it.addProperty("fixed", trace.getFixedState().toString());
        JsonArray corners = new JsonArray();
        for (int i = 0; i < trace.polyline().lines.length - 1; i++) {
          FloatPoint c = trace.polyline().cornerApprox(i);
          JsonArray pt = new JsonArray();
          pt.add(Double.toString(c.x));
          pt.add(Double.toString(c.y));
          corners.add(pt);
        }
        it.add("corners", corners);
      }
      // T5 extension: vias carry their center + shove-fixed state so
      // relocation witnesses are readable straight off the rows
      // (Point is abstract; toFloat + Double.toString is the
      // corner-dump convention).
      if (item instanceof Via via) {
        FloatPoint c = via.getCenter().toFloat();
        it.addProperty("center", Double.toString(c.x) + ":" + Double.toString(c.y));
        it.addProperty("shoveFixed", via.isShoveFixed());
      }
      items.add(it);
    }
    JsonObject out = obj("inventory");
    out.addProperty("phase", phase);
    out.add("items", items);
    row(out);
  }

  /** The changed-area session state (diagnostic; null when inactive). */
  private static void dumpChangedArea(String phase, RoutingBoard board) {
    JsonObject out = obj("changed_area");
    out.addProperty("phase", phase);
    app.freerouting.board.state.ChangedArea area = board.changedArea;
    if (area == null) {
      out.addProperty("session", false);
      row(out);
      return;
    }
    out.addProperty("session", true);
    JsonArray layers = new JsonArray();
    for (int l = 0; l < board.layerStructure.layers.length; l++) {
      app.freerouting.geometry.planar.IntOctagon oct = area.getArea(l);
      JsonObject lo = new JsonObject();
      lo.addProperty("layer", l);
      lo.addProperty("leftX", oct.leftX);
      lo.addProperty("bottomY", oct.bottomY);
      lo.addProperty("rightX", oct.rightX);
      lo.addProperty("topY", oct.topY);
      layers.add(lo);
    }
    out.add("layers", layers);
    row(out);
  }

  /** One world: fresh parse, session + join plan, the opt call, dumps. */
  private static void runWorld(
      byte[] bytes, String fileName, String name, int[][] joinPlan) throws Exception {
    RoutingBoard board = parse(bytes, fileName);
    // The parse maps DSN units to DBU through the resolution (um 10
    // here -> x10); the join plans below are written in DBU already.
    int dbuScale = board.communication.resolution;

    JsonObject worldRow = obj("world");
    worldRow.addProperty("name", name);
    worldRow.addProperty("dbuScale", dbuScale);
    JsonArray joins = new JsonArray();
    for (int[] p : joinPlan) {
      JsonArray pt = new JsonArray();
      pt.add(p[0]);
      pt.add(p[1]);
      joins.add(pt);
    }
    worldRow.add("joinPlan", joins);
    worldRow.addProperty("accuracy", 100);
    // The offset-arithmetic feeders (TraceTightener.optChangedArea
    // :138-141): clearanceMatrix.maxValue(0) + 2 * maxTraceHalfWidth.
    worldRow.addProperty("maxTraceHalfWidth", board.rules.getMaxTraceHalfWidth());
    worldRow.addProperty("minTraceHalfWidth", board.rules.getMinTraceHalfWidth());
    worldRow.addProperty("maxClearanceL0", board.rules.clearanceMatrix.maxValue(0));
    row(worldRow);

    board.startMarkingChangedArea();
    for (int[] p : joinPlan) {
      board.joinChangedArea(new FloatPoint(p[0], p[1]), 0);
    }
    dumpChangedArea("after_join", board);
    dumpInventory("before", board);

    // The production-consumption shape (AutorouteConnectionRouter :107-113
    // with the wall-clock limit zeroed for a deterministic witness).
    board.optChangedArea(new int[0], null, 100, null, null, 0);
    row(obj("opt_done"));

    dumpInventory("after", board);
  }

  /**
   * The region-reach SCAN: fresh parse per x, one join at (x, 180000) —
   * ON the bottom arm's centerline (the 90-degree tree holds
   * per-segment boxes WITHOUT end caps, so the reach test is
   * region-vs-segment-box, not region-vs-bounding-area; the earlier
   * y=190000 scan sat between the arms and never reached) — the same
   * opt call; records whether the N5 cup moved. The boundary x* where
   * movement starts measures the EFFECTIVE offset: x* + offset >=
   * 1000000 (the bottom arm's west segment edge), freezing the offset
   * formula against the jar instead of assuming it.
   */
  private static void runRegionScan(byte[] bytes, String fileName) throws Exception {
    for (int x = 984000; x <= 997000; x += 250) {
      RoutingBoard board = parse(bytes, fileName);
      String before = traceCorners(board, 6);
      board.startMarkingChangedArea();
      board.joinChangedArea(new FloatPoint(x, 180000), 0);
      board.optChangedArea(new int[0], null, 100, null, null, 0);
      String after = traceCorners(board, 6);
      JsonObject r = obj("region_scan");
      r.addProperty("x", x);
      r.addProperty("maxTraceHalfWidth", board.rules.getMaxTraceHalfWidth());
      r.addProperty("maxClearanceL0", board.rules.clearanceMatrix.maxValue(0));
      r.addProperty("moved", !before.equals(after));
      if (!before.equals(after)) {
        r.addProperty("after", after);
      }
      row(r);
    }
  }

  /** The corner chain of trace p_id as one string ("x:y;x:y;..."). */
  private static String traceCorners(RoutingBoard board, int id) {
    String s = traceCornersOrNull(board, id);
    if (s == null) {
      throw new IllegalStateException("no trace " + id);
    }
    return s;
  }

  /**
   * Null when the id is gone: the 45-degree pin-connection tail
   * (swapConnectionToPin/correctConnectionToPin) may REMOVE and
   * REINSERT the trace under a new id (or split it), so a corner-world
   * row must tolerate a vanished focal id instead of killing the run.
   */
  private static String traceCornersOrNull(RoutingBoard board, int id) {
    for (Item item : board.getItems()) {
      if (item instanceof PolylineTrace trace && item.getId() == id) {
        StringBuilder b = new StringBuilder();
        for (int i = 0; i < trace.polyline().lines.length - 1; i++) {
          FloatPoint c = trace.polyline().cornerApprox(i);
          if (b.length() > 0) {
            b.append(';');
          }
          b.append(Double.toString(c.x)).append(':').append(Double.toString(c.y));
        }
        return b.toString();
      }
    }
    return null;
  }

  /**
   * The CORNER-WORLDS mode (`args[1] == "cornerworlds"`): one world PER
   * PolylineTrace of the fixture (fresh parse each; ascending trace id),
   * joining that trace's own corners on layer 0 — the same
   * production-shape opt call. Used to jar-witness crafted end-geometry
   * candidates (e.g. the last-corner-skip arm census on
   * last-corner-skip.dsn). One "corner_world" row per trace with the before
   * and after chains.
   *
   * <p>T4 extension: each world also dumps the FULL inventory
   * (every item, every trace's corner chain at Double.toString
   * precision) before and after — the 45-degree witnesses need the
   * whole board, not just the focal trace: the smoothen-at-trace arms
   * reshape the CONTACT trace, the junction worlds move the partner,
   * and the pin-connection tail may insert a shove-fixed exit stub
   * under a NEW trace id. Traces with no PolylineTrace corners (pins,
   * vias, keepouts) still appear with kind+nets so id churn is visible.
   */
  private static void runCornerWorlds(byte[] bytes, String dsnPath) throws Exception {
    // The row's `fixture` field is the DSN BASENAME (the caller's
    // second argument is the mode string "cornerworlds", not a name).
    String fileName = Paths.get(dsnPath).getFileName().toString();
    RoutingBoard once = parse(bytes, fileName);
    List<Integer> ids = new ArrayList<>();
    for (Item item : once.getItems()) {
      if (item instanceof PolylineTrace) {
        ids.add(item.getId());
      }
    }
    ids.sort(Comparator.comparingInt(Integer::intValue));
    for (int id : ids) {
      RoutingBoard board = parse(bytes, fileName);
      String before = traceCornersOrNull(board, id);
      board.startMarkingChangedArea();
      for (Item item : board.getItems()) {
        if (item instanceof PolylineTrace trace && item.getId() == id) {
          for (int i = 0; i < trace.polyline().lines.length - 1; i++) {
            FloatPoint c = trace.polyline().cornerApprox(i);
            board.joinChangedArea(c, 0);
          }
        }
      }
      dumpInventory("before", board);
      board.optChangedArea(new int[0], null, 100, null, null, 0);
      String after = traceCornersOrNull(board, id);
      dumpInventory("after", board);
      JsonObject r = obj("corner_world");
      r.addProperty("fixture", fileName);
      r.addProperty("trace", id);
      r.addProperty("before", before);
      r.addProperty("after", after);
      row(r);
    }
  }

  /**
   * The TAIL-WORLDS mode (`args[1] == "tailworlds"`, M4-T6): the
   * pin-connection tail witness (swapConnectionToPin /
   * correctConnectionToPin inside `PolylineTrace.pullTight`, gated
   * `angleRestriction != NINETY_DEGREE && pinEdgeToTurnDist > 0`).
   * One world PER PolylineTrace of the fixture (fresh parse each,
   * ascending id, the cornerworlds join shape: the focal trace's own
   * corners joined on layer 0, then the production-shape opt call).
   * Emits a `tail_world` row per world (focal id + corner chains
   * before/after, null when the focal id vanished — the swap arm
   * combines and deletes traces) on top of the full inventory dumps
   * (which since the T6 extension carry the trace fixed states, so
   * the swap arm's setFixedState + combine shows as two traces
   * becoming one unfixed one). Mode "tailgate0" (the boolean):
   * `board.rules.setPinEdgeToTurnDist(0)` BEFORE the opt call — the
   * `pinEdgeToTurnDist > 0` gate's negative arm; the tail must never
   * fire and the inventory must stay put (the default-distance run of
   * the same fixture is the positive control).
   */
  private static void runTailWorlds(byte[] bytes, String dsnPath, boolean gate0) throws Exception {
    String fileName = Paths.get(dsnPath).getFileName().toString();
    RoutingBoard once = parse(bytes, fileName);
    List<Integer> ids = new ArrayList<>();
    for (Item item : once.getItems()) {
      if (item instanceof PolylineTrace) {
        ids.add(item.getId());
      }
    }
    ids.sort(Comparator.comparingInt(Integer::intValue));
    for (int id : ids) {
      RoutingBoard board = parse(bytes, fileName);
      if (gate0) {
        board.rules.setPinEdgeToTurnDist(0);
      }
      String before = traceCornersOrNull(board, id);
      board.startMarkingChangedArea();
      for (Item item : board.getItems()) {
        if (item instanceof PolylineTrace trace && item.getId() == id) {
          for (int i = 0; i < trace.polyline().lines.length - 1; i++) {
            FloatPoint c = trace.polyline().cornerApprox(i);
            board.joinChangedArea(c, 0);
          }
        }
      }
      dumpInventory("before", board);
      board.optChangedArea(new int[0], null, 100, null, null, 0);
      String after = traceCornersOrNull(board, id);
      dumpInventory("after", board);
      JsonObject r = obj("tail_world");
      r.addProperty("fixture", fileName);
      r.addProperty("gate0", gate0);
      r.addProperty("trace", id);
      r.addProperty("before", before);
      r.addProperty("after", after);
      row(r);
    }
  }

  /**
   * The VIA-WORLDS mode (`args[1] == "viaworlds"`, M4-T5): the crafted
   * via worlds on via_optimizer45.dsn, each a fresh parse with ONE join
   * at the world's via center (layer 0) and the production-shape opt
   * call WITH non-null per-layer traceCosts — the feed that arms the
   * ViaOptimizer hook (null costs keeps the via arm dead, which is why
   * the T3/T4 captures never saw via moves). Two cost tables: 0 =
   * uniform (1,1) on both layers; 1 = F.Cu (1,3) / B.Cu (3,1), the
   * layer-cost discriminator (the same N6 geometry moves under 1 and
   * must stay put under 0 — the 2x2 reject-arm cell).
   *
   * <p>Worlds: collinear (N1 overlap walk), gate3 (N2, 3 contacts ->
   * untouched), fanout (N3 walk along its own trace), projection (N4:
   * the direct move is mover-blocked by the N9 B.Cu wire, the
   * projection face drops the via onto the prev line), costs /
   * costs_uniform (N6, the weighted-distance pair), acute (N8, the
   * acute-angle arm under uniform costs), shove_fixed (N5: the same
   * 2-trace westward geometry as N1 WITH (type shoveFixed) — the join
   * is AT its center so the opt pass actually processes it; the
   * isShoveFixed gate must hold it while the unfixed N1 twin moves),
   * costs_null (N1 again with traceCosts == null — the consumption
   * gate: the whole via arm is dead without costs, so N1 must stay).
   */
  private static void runViaWorlds(byte[] bytes, String dsnPath) throws Exception {
    String fileName = Paths.get(dsnPath).getFileName().toString();
    ExpansionCostFactor[][] costTables = {
      {new ExpansionCostFactor(1, 1), new ExpansionCostFactor(1, 1)},
      {new ExpansionCostFactor(1, 3), new ExpansionCostFactor(3, 1)},
    };
    int[][][] worlds = {
      // name-id, costs-id (-1 = null costs), join plan (each join sits
      // ON the world's via center so the changed area reaches it)
      {{1}, {0}, {600000, 380000}}, // collinear (N1)
      {{2}, {0}, {200000, 450000}}, // gate3 (N2)
      {{3}, {0}, {900000, 150000}}, // fanout (N3)
      {{4}, {0}, {520000, 330000}}, // projection (N4)
      {{6}, {1}, {720000, 450000}}, // costs (N6)
      {{7}, {0}, {720000, 450000}}, // costs_uniform (N6, reject cell)
      {{8}, {0}, {200000, 150000}}, // acute (N8)
      {{5}, {0}, {450000, 600000}}, // shove_fixed (N5, gate world)
      {{1}, {-1}, {600000, 380000}}, // costs_null (N1, consumption gate)
      // T6 ledger world (b): the SAME N1 collinear geometry under the
      // NON-UNIFORM cost table — where the collinear arm's first move
      // and the acute arm's wd comparison may diverge (the T5-M1
      // banked-healer settlement world).
      {{1}, {1}, {600000, 380000}}, // collinear_costs (N1, costs 1)
    };
    String[] names = {
      "collinear", "gate3", "fanout", "projection", "costs", "costs_uniform", "acute",
      "shove_fixed", "costs_null", "collinear_costs",
    };
    for (int w = 0; w < worlds.length; w++) {
      RoutingBoard board = parse(bytes, fileName);
      int costsId = worlds[w][1][0];
      int[][] joinPlan = worlds[w].length > 2 ? new int[][] {worlds[w][2]} : new int[0][];

      JsonObject worldRow = obj("via_world");
      worldRow.addProperty("name", names[w]);
      worldRow.addProperty("costs", costsId);
      worldRow.add("beforeCenters", viaCenters(board));
      row(worldRow);

      board.startMarkingChangedArea();
      for (int[] p : joinPlan) {
        board.joinChangedArea(new FloatPoint(p[0], p[1]), 0);
      }
      dumpInventory("before", board);
      // costs-id -1 = null traceCosts — the via arm is dead (the T3/T4
      // consumption-gate face).
      ExpansionCostFactor[] costs = costsId < 0 ? null : costTables[costsId];
      board.optChangedArea(new int[0], null, 100, costs, null, 0);
      row(obj("opt_done"));
      dumpInventory("after", board);

      JsonObject doneRow = obj("via_world_done");
      doneRow.addProperty("name", names[w]);
      doneRow.addProperty("costs", costsId);
      doneRow.add("afterCenters", viaCenters(board));
      row(doneRow);
    }
  }

  /**
   * The VIA-DEPTH-DIAG mode (`args[1] == "viadepth"`, M4-T5 quality-review
   * fix round, MINOR-1): the jar-side witness behind the t5_depth_gate
   * pin's one-call intermediate. Drives the PUBLIC production entry
   * ViaOptimizer.optViaLocation(board, via, costs, 100, depth) DIRECTLY on
   * fresh parses of via_optimizer45.dsn — the same call the changed-area
   * loop makes for vias (TraceTightener.optChangedArea passes
   * this.minTranslateDist and the hardcoded max depth 10; accuracy 100 at
   * the seam yields minTranslateDist 100 via the >= 100 clamp) — but with
   * the depth under test as the ONLY variable, so each row is a
   * single-call result at a controlled depth. Two cost tables: the
   * uniform (1,1) reject cell ("costs_uniform" — same geometry, the via
   * must stay put) and the non-uniform F.Cu (1,3) / B.Cu (3,1)
   * discriminator ("costs" — the t5_depth_gate world). Depths per table:
   * 0 = the gate must refuse (no move); 1 = exactly the ONE-CALL arm
   * (the depth-1 intermediate the pins cite, here witnessed on-disk);
   * 10 = the production depth (a SINGLE production-depth call — the
   * seam's fixpoint loop may issue several, so this row witnesses the
   * single-call result, not the run4/run5 afterCenters final). Via
   * id 16 (net N6, net number 5) at (720000,450000). Deterministic
   * JSONL; run twice,
   * cmp-identical (via45_costs_diag.txt / via45_costs_diag_b.txt).
   */
  private static void runViaDepthDiag(byte[] bytes) throws Exception {
    ExpansionCostFactor[][] tables = {
      {new ExpansionCostFactor(1, 1), new ExpansionCostFactor(1, 1)},
      {new ExpansionCostFactor(1, 3), new ExpansionCostFactor(3, 1)},
    };
    String[] tableNames = {"costs_uniform", "costs"};
    int[] depths = {0, 1, 10};
    for (int t = 0; t < tables.length; t++) {
      for (int depth : depths) {
        RoutingBoard board = parse(bytes, "via_optimizer45.dsn");
        Via via = viaById(board, 16);
        String before = centerString(via);
        boolean changed = ViaOptimizer.optViaLocation(board, via, tables[t], 100, depth);
        JsonObject r = obj("via_depth");
        r.addProperty("world", tableNames[t]);
        r.addProperty("depth", depth);
        r.addProperty("before", before);
        r.addProperty("changed", changed);
        r.addProperty("after", centerString(via));
        row(r);
      }
    }
    row(obj("via_depth_done"));
  }

  /** The via with the given id (the T5 fixture's ids are stable). */
  private static Via viaById(RoutingBoard board, int id) {
    for (Item item : board.getItems()) {
      if (item instanceof Via via && item.getId() == id) {
        return via;
      }
    }
    throw new IllegalStateException("no via with id " + id);
  }

  /** Via center in the inventory-row convention "x:y" (Double.toString). */
  private static String centerString(Via via) {
    FloatPoint c = via.getCenter().toFloat();
    return Double.toString(c.x) + ":" + Double.toString(c.y);
  }

  /** All via centers as id-sorted [[x, y], ...] (determinism). */
  private static JsonArray viaCenters(RoutingBoard board) {
    List<Via> vias = new ArrayList<>();
    for (Item item : board.getItems()) {
      if (item instanceof Via via) {
        vias.add(via);
      }
    }
    vias.sort(Comparator.comparingInt(Item::getId));
    JsonArray arr = new JsonArray();
    for (Via via : vias) {
      FloatPoint c = via.getCenter().toFloat();
      JsonArray pt = new JsonArray();
      pt.add(Double.toString(c.x));
      pt.add(Double.toString(c.y));
      arr.add(pt);
    }
    return arr;
  }

  public static void main(String[] args) throws Exception {
    byte[] bytes = Files.readAllBytes(Paths.get(args[0]));
    String fileName = args.length > 1 ? args[1] : "tightener90.dsn";
    if (args.length > 1 && args[1].equals("regionscan")) {
      runRegionScan(bytes, fileName);
      return;
    }
    if (args.length > 1 && args[1].equals("cornerworlds")) {
      runCornerWorlds(bytes, args[0]);
      return;
    }
    if (args.length > 1 && args[1].equals("viaworlds")) {
      runViaWorlds(bytes, args[0]);
      return;
    }
    if (args.length > 1 && args[1].equals("viadepth")) {
      runViaDepthDiag(bytes);
      return;
    }
    if (args.length > 1 && args[1].equals("tailworlds")) {
      runTailWorlds(bytes, args[0], false);
      return;
    }
    if (args.length > 1 && args[1].equals("tailgate0")) {
      runTailWorlds(bytes, args[0], true);
      return;
    }

    runWorld(
        bytes,
        fileName,
        "stair",
        new int[][] {
          {200000, 400000},
          {360000, 400000},
          {360000, 340000},
          {240000, 340000},
          {240000, 280000},
          {160000, 280000},
        });
    runWorld(
        bytes,
        fileName,
        "acid",
        new int[][] {
          {200000, 120000},
          {360000, 120000},
        });
    runWorld(
        bytes,
        fileName,
        "region_in",
        new int[][] {
          {994000, 180000},
        });
    runWorld(
        bytes,
        fileName,
        "region_out",
        new int[][] {
          {985000, 180000},
        });
    runWorld(
        bytes,
        fileName,
        "block",
        new int[][] {
          {450000, 390000},
          {480000, 390000},
          {480000, 410000},
          {450000, 410000},
          {480000, 360000},
          {540000, 360000},
          {540000, 420000},
          {480000, 420000},
        });
  }
}
