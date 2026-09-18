// MazeSpike.java — the M3-T6 jar-side probe oracle for the maze
// search CORE port (front ordering, occupancy marking, room-door
// expansion, the expandToDoorSection cost model). DrillSpike /
// ExpansionSpike house pattern: ONE JVM per run, deterministic JSONL
// rows on stdout (lines starting with "{" are the results; the jar's
// FRLogger noise is interleaved and dropped).
//
// Build/run (JDK 25, from the repo root):
//   mkdir -p /tmp/epic-maze-classes && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/javac \
//       -cp build/libs/freerouting-current-executable.jar \
//       -d /tmp/epic-maze-classes rust/harness/oracle/MazeSpike.java && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/java \
//       -Duser.language=en -Duser.country=US \
//       -cp build/libs/freerouting-current-executable.jar:/tmp/epic-maze-classes \
//       app.freerouting.autoroute.maze.MazeSpike \
//       rust/harness/fixtures/maze-spike/t6_maze.dsn
//
// The oracle declares the app.freerouting.autoroute.maze package so
// the package-private MazeSearchEngine ctor, the mazeExpansionList /
// destinationDistance / destinationDoor fields, the MazeListElement
// fields, and the ctor + OccupyNextElement plumbing are directly
// reachable. `doorIsSmall` is PRIVATE and is invoked through
// reflection (the DrillSpike Field precedent).
//
// Capture protocol (both sides must reproduce it EXACTLY):
//   drain(search, cap, phase): while the front is non-empty and k <
//   cap, emit `head` (the RAW fields of front.first()), then call
//   occupyNextElement() ONCE, then emit `popMeta` (front size before /
//   after, the boolean return). With no ripup and no delayed
//   occupation the head IS the popped element; under the fixed
//   protocol any divergence cancels out — Java truth is captured
//   under protocol P and the Rust replay runs the same P.
//
// Determinism: rows are emitted from TreeSet iteration order only;
// every double is printed with Double.toString (exact round-trip); no
// HashSet/HashMap iteration reaches a row.
package app.freerouting.autoroute.maze;

import app.freerouting.autoroute.drill.DrillPageArray;
import app.freerouting.autoroute.expansion.CompleteExpansionRoom;
import app.freerouting.autoroute.expansion.ExpandableObject;
import app.freerouting.autoroute.expansion.ExpansionDoor;
import app.freerouting.autoroute.expansion.FreeSpaceExpansionRoom;
import app.freerouting.board.actions.ItemIdGenerator;
import app.freerouting.board.facade.BasicBoard;
import app.freerouting.board.facade.RoutingBoard;
import app.freerouting.board.model.items.Item;
import app.freerouting.board.model.items.Pin;
import app.freerouting.geometry.planar.FloatLine;
import app.freerouting.geometry.planar.FloatPoint;
import app.freerouting.geometry.planar.IntBox;
import app.freerouting.geometry.planar.IntPoint;
import app.freerouting.io.BoardReadResult;
import app.freerouting.io.specctra.DsnReader;
import app.freerouting.settings.RouterSettings;
import com.google.gson.Gson;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import java.io.ByteArrayInputStream;
import java.lang.reflect.Method;
import java.nio.file.Files;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;
import java.util.Set;
import java.util.SortedSet;
import java.util.TreeSet;

public class MazeSpike {

  private static final Gson GSON = new Gson();

  /** The pop cap of the MAIN (no-vias) drain. */
  private static final int MAIN_CAP = 120;
  /** The pop cap of the drill-pages drain. */
  private static final int DRILL_CAP = 40;
  /** bendCosts[0] forced for the run (pins the bend-penalty rows). */
  private static final double BEND_COST = 47.0;

  private static final java.lang.reflect.Field DEST_DOOR_FIELD;
  private static final java.lang.reflect.Field DEST_SECTION_FIELD;
  // T8: the destinationDistance field (package-private FINAL, assigned
  // in the ctor) and the PRIVATE init — both reached reflectively so
  // the spy is installed BEFORE init and the init-time join rows are
  // captured.
  private static final java.lang.reflect.Field DEST_DISTANCE_FIELD;
  private static final Method INIT_METHOD;

  static {
    try {
      DEST_DOOR_FIELD = MazeSearchEngine.class.getDeclaredField("destinationDoor");
      DEST_DOOR_FIELD.setAccessible(true);
      DEST_SECTION_FIELD =
          MazeSearchEngine.class.getDeclaredField("sectionNoOfDestinationDoor");
      DEST_SECTION_FIELD.setAccessible(true);
      DEST_DISTANCE_FIELD = MazeSearchEngine.class.getDeclaredField("destinationDistance");
      DEST_DISTANCE_FIELD.setAccessible(true);
      INIT_METHOD = MazeSearchEngine.class.getDeclaredMethod("init", Set.class, Set.class);
      INIT_METHOD.setAccessible(true);
    } catch (Exception e) {
      throw new ExceptionInInitializerError(e);
    }
  }

  private static void row(JsonObject obj) {
    System.out.println(GSON.toJson(obj));
  }

  private static JsonObject obj(String kind) {
    JsonObject o = new JsonObject();
    o.addProperty("type", kind);
    return o;
  }

  private static String d(double v) {
    return Double.toString(v);
  }

  /**
   * The RAW fields of a MazeListElement — the expandToDoorSection
   * assigns (:908-963) as observable on the front element.
   */
  private static void emitElement(String type, String phase, int k, MazeListElement e) {
    JsonObject o = obj(type);
    o.addProperty("phase", phase);
    o.addProperty("k", k);
    o.addProperty("doorKind", e.door.getClass().getSimpleName());
    o.addProperty("doorId", e.door.getId());
    o.addProperty("section", e.sectionNoOfDoor);
    o.addProperty("expansionValue", d(e.expansionValue));
    o.addProperty("sortingValue", d(e.sortingValue));
    o.addProperty("roomRipped", e.roomRipped);
    o.addProperty("ripupCost", e.ripupCost);
    o.addProperty("alreadyChecked", e.alreadyChecked);
    if (e.backtrackDoor != null) {
      o.addProperty("backKind", e.backtrackDoor.getClass().getSimpleName());
      o.addProperty("backId", e.backtrackDoor.getId());
    } else {
      o.addProperty("backKind", "null");
      o.addProperty("backId", -1);
    }
    o.addProperty("backSection", e.sectionNoOfBacktrackDoor);
    o.addProperty("nextRoomId", e.nextRoom != null ? e.nextRoom.getId() : -1);
    o.addProperty("ax", d(e.shapeEntry.a.x));
    o.addProperty("ay", d(e.shapeEntry.a.y));
    o.addProperty("bx", d(e.shapeEntry.b.x));
    o.addProperty("by", d(e.shapeEntry.b.y));
    row(o);
  }

  // ---- T8: the destination-distance spy + crafted probe worlds ------

  /**
   * Delegating spy over the REAL DestinationDistance: emits one row
   * per engine `join` (layer + box) and one per engine
   * `calculate(FloatPoint, layer)` call (layer + point + result).
   * The super delegation keeps behavior bit-identical, so every
   * pre-existing row (heads, sortingValues, dist probes) is
   * unchanged — the ddJoin/ddCalc rows are pure INSERTIONS.
   */
  private static final class SpyDistance extends DestinationDistance {
    final String phase;

    SpyDistance(AutorouteControl ctrl, String phase) {
      super(ctrl.traceCosts, ctrl.layerActive, ctrl.minNormalViaCost, ctrl.minCheapViaCost);
      this.phase = phase;
    }

    @Override
    public void join(IntBox box, int layer) {
      super.join(box, layer);
      JsonObject o = obj("ddJoin");
      o.addProperty("phase", phase);
      o.addProperty("layer", layer);
      o.addProperty("llx", box.ll.x);
      o.addProperty("lly", box.ll.y);
      o.addProperty("urx", box.ur.x);
      o.addProperty("ury", box.ur.y);
      row(o);
    }

    @Override
    public double calculate(FloatPoint point, int layer) {
      double value = super.calculate(point, layer);
      JsonObject o = obj("ddCalc");
      o.addProperty("phase", phase);
      o.addProperty("x", d(point.x));
      o.addProperty("y", d(point.y));
      o.addProperty("layer", layer);
      o.addProperty("value", d(value));
      row(o);
      return value;
    }
  }

  /**
   * openSearch with the spy installed BEFORE init (the ctor builds
   * its own DestinationDistance and `init` is PRIVATE — the swap uses
   * reflection, then invokes init the same way). Behavior-identical
   * to getInstance + the same searchOpen row.
   */
  private static MazeSearchEngine openSearchSpied(
      RoutingBoard board,
      AutorouteControl ctrl,
      int net,
      Pin start,
      Pin dest,
      String phase)
      throws Exception {
    AutorouteEngine engine = new AutorouteEngine(board, ctrl.viaClearanceClass, true);
    engine.initConnection(net, null, null);
    java.lang.reflect.Field incompleteField =
        AutorouteEngine.class.getDeclaredField("incompleteExpansionRooms");
    incompleteField.setAccessible(true);
    incompleteField.set(engine, new ArrayList<>());
    MazeSearchEngine search = new MazeSearchEngine(engine, ctrl);
    DEST_DISTANCE_FIELD.set(search, new SpyDistance(ctrl, phase));
    boolean ok = (Boolean) INIT_METHOD.invoke(search, Set.of(start), Set.of(dest));
    JsonObject o = obj("searchOpen");
    o.addProperty("phase", phase);
    o.addProperty("ok", ok);
    row(o);
    return ok ? search : null;
  }

  /** One crafted probe world's ctor-derivation row (the raw fields). */
  private static void ddCtorRow(
      String world,
      DestinationDistance dd,
      int layerCount,
      int activeLayerCount) {
    JsonObject o = obj("ddCtor");
    o.addProperty("world", world);
    o.addProperty("layerCount", layerCount);
    o.addProperty("activeLayerCount", activeLayerCount);
    o.addProperty("minComponent", d(dd.minComponentSideTraceCost));
    o.addProperty("maxComponent", d(dd.maxComponentSideTraceCost));
    o.addProperty("minSolder", d(dd.minSolderSideTraceCost));
    o.addProperty("maxSolder", d(dd.maxSolderSideTraceCost));
    o.addProperty("maxInner", d(dd.maxInnerSideTraceCost));
    o.addProperty("minComponentInner", d(dd.minComponentInnerTraceCost));
    o.addProperty("minSolderInner", d(dd.minSolderInnerTraceCost));
    o.addProperty("minComponentSolderInner", d(dd.minComponentSolderInnerTraceCost));
    row(o);
  }

  private static void ddJoinRow(String world, int layer) {
    JsonObject o = obj("ddJoinP");
    o.addProperty("world", world);
    o.addProperty("layer", layer);
    row(o);
  }

  /** One world's post-join bucket coordinates (MQ1: join accumulation).
   * "llx lly urx ury" per bucket, or "EMPTY" when never joined. The
   * bucket fields are PRIVATE in the jar (unlike the cost fields), so
   * both the box and its is-empty flag are read reflectively. */
  private static Object ddPrivate(DestinationDistance dd, String field) throws Exception {
    java.lang.reflect.Field f = DestinationDistance.class.getDeclaredField(field);
    f.setAccessible(true);
    return f.get(dd);
  }

  private static void ddBucketRow(String world, DestinationDistance dd) {
    try {
      IntBox comp = (IntBox) ddPrivate(dd, "componentSideBox");
      IntBox sold = (IntBox) ddPrivate(dd, "solderSideBox");
      IntBox inner = (IntBox) ddPrivate(dd, "innerSideBox");
      JsonObject o = obj("ddBucket");
      o.addProperty("world", world);
      o.addProperty("component", ddBoxStr(comp, (Boolean) ddPrivate(dd,
          "componentSideBoxIsEmpty")));
      o.addProperty("solder", ddBoxStr(sold, (Boolean) ddPrivate(dd, "solderSideBoxIsEmpty")));
      o.addProperty("inner", ddBoxStr(inner, (Boolean) ddPrivate(dd, "innerSideBoxIsEmpty")));
      row(o);
    } catch (Exception e) {
      throw new RuntimeException(e);
    }
  }

  private static String ddBoxStr(IntBox b, boolean empty) {
    if (empty) {
      return "EMPTY";
    }
    return b.ll.x + " " + b.ll.y + " " + b.ur.x + " " + b.ur.y;
  }

  private static void ddCalcRow(
      String world, String kind, int id, int layer, IntBox box, double value) {
    JsonObject o = obj(kind);
    o.addProperty("world", world);
    o.addProperty("id", id);
    o.addProperty("layer", layer);
    o.addProperty("llx", box.ll.x);
    o.addProperty("lly", box.ll.y);
    o.addProperty("urx", box.ur.x);
    o.addProperty("ury", box.ur.y);
    o.addProperty("value", d(value));
    row(o);
  }

  /**
   * One crafted world: ctor row, TGT joins on the given layers, then
   * the box probes (IntBox overload) + point probes (FloatPoint
   * overload) on each probe layer. Probe box P0 == TGT (all deltas
   * 0), P1 partial overlap, P2 right (deltaX only), P3 above
   * (deltaY only), P4 diagonal TIE (dx == dy → the split's ELSE
   * arm), P5 below-left (both deltas, dy > dx).
   */
  private static void ddWorld(
      String name,
      double[][] layerCosts,
      boolean[] active,
      double normalVia,
      double cheapVia,
      int[] joinedLayers,
      int[] probeLayers) {
    ddWorld(name, layerCosts, active, normalVia, cheapVia, joinedLayers, probeLayers,
        new int[0][]);
  }

  /**
   * ddWorld with EXTRA crafted joins: each row of {@code extraJoins}
   * is {@code {layer, llx, lly, urx, ury}}, joined after the TGT
   * joins. The TGT-only worlds give every bucket the SAME box, which
   * makes the post-gate fallthrough arms coincide with the pre-gate
   * arms — the `==2`/`==3` early-return gates are observationally
   * dead there. Differing boxes per bucket (and all-expensive trace
   * costs, so `minXxxInner > 1`) break the ties and let a mutant's
   * fallthrough/early-return SHrink or grow the result.
   */
  private static void ddWorld(
      String name,
      double[][] layerCosts,
      boolean[] active,
      double normalVia,
      double cheapVia,
      int[] joinedLayers,
      int[] probeLayers,
      int[][] extraJoins) {
    AutorouteControl.ExpansionCostFactor[] costs = new AutorouteControl.ExpansionCostFactor[
        layerCosts.length];
    for (int i = 0; i < layerCosts.length; i++) {
      costs[i] = new AutorouteControl.ExpansionCostFactor(layerCosts[i][0], layerCosts[i][1]);
    }
    DestinationDistance dd = new DestinationDistance(costs, active, normalVia, cheapVia);
    int activeCount = 0;
    for (boolean b : active) {
      if (b) {
        activeCount++;
      }
    }
    ddCtorRow(name, dd, active.length, activeCount);
    IntBox tgt = new IntBox(100000, 200000, 200000, 300000);
    for (int layer : joinedLayers) {
      dd.join(tgt, layer);
      ddJoinRow(name, layer);
    }
    for (int[] xj : extraJoins) {
      dd.join(new IntBox(xj[1], xj[2], xj[3], xj[4]), xj[0]);
      ddJoinRow(name, xj[0]);
    }
    // MQ1: the post-join bucket COORDINATES (join accumulation, not
    // last-join-wins). Emitted only for the quality-round worlds — a
    // per-world row here would interleave into the existing T8 rows
    // and shift the 912-line prefix.
    if (name.equals("MJ") || name.equals("W") || name.equals("X") || name.equals("R")) {
      ddBucketRow(name, dd);
    }
    int[][] probes = {
      {100000, 200000, 200000, 300000},
      {150000, 250000, 250000, 350000},
      {300000, 200000, 400000, 300000},
      {100000, 400000, 200000, 500000},
      {300000, 400000, 400000, 500000},
      {50000, 100000, 60000, 110000},
      {150000, 320000, 190000, 420000},
    };
    int id = 0;
    for (int[] pr : probes) {
      IntBox box = new IntBox(pr[0], pr[1], pr[2], pr[3]);
      for (int layer : probeLayers) {
        ddCalcRow(name, "ddCalcP", id, layer, box, dd.calculate(box, layer));
      }
      id++;
    }
    double[][] points = {{150000, 250000}, {350000, 250000}, {350000, 450000}};
    for (int p = 0; p < points.length; p++) {
      IntBox box = new FloatPoint(points[p][0], points[p][1]).boundingBox();
      for (int layer : probeLayers) {
        ddCalcRow(
            name, "ddPointP", p, layer, box, dd.calculate(new FloatPoint(points[p][0],
                points[p][1]), layer));
      }
    }
    // The cheap-distance arm (world A only): CHEAPBOX (P6) is
    // x-overlapping / y-disjoint, so the TWO-LAYER via arm wins the
    // min — the 400→320 substitution is observable (a delta-only
    // probe like P2 would coincide: the one-layer weighted arm has no
    // via cost). The restore row re-runs CHEAPBOX at normal cost.
    if (name.equals("A")) {
      IntBox cheapBox = new IntBox(150000, 320000, 190000, 420000);
      ddCalcRow(name, "ddCheapP", 0, 0, cheapBox, dd.calculateCheapDistance(cheapBox, 0));
      ddCalcRow(name, "ddRestoreP", 0, 0, cheapBox, dd.calculate(cheapBox, 0));
      // The sentinel: a FRESH instance with nothing joined.
      DestinationDistance fresh =
          new DestinationDistance(costs, active, normalVia, cheapVia);
      ddCalcRow("SENTINEL", "ddCalcP", 0, 0, tgt, fresh.calculate(tgt, 0));
    }
  }

  /** One drain step of the fixed capture protocol. */
  private static void drain(MazeSearchEngine search, int cap, String phase) throws Exception {
    SortedSet<MazeListElement> front = search.mazeExpansionList;
    int k = 0;
    while (!front.isEmpty() && k < cap) {
      MazeListElement head = front.first();
      emitElement("head", phase, k, head);
      int before = front.size();
      boolean cont = search.occupyNextElement();
      JsonObject p = obj("popMeta");
      p.addProperty("phase", phase);
      p.addProperty("k", k);
      p.addProperty("before", before);
      p.addProperty("after", front.size());
      p.addProperty("cont", cont);
      row(p);
      if (!cont) {
        // The destination is reached (or the front drained): pin the
        // result once. The destination fields are PRIVATE — read them
        // through reflection (the DrillSpike Field precedent).
        // BREAK: findConnection is `while (occupyNextElement())` —
        // popping past the destination keeps returning false for the
        // remaining target-door elements (each re-marks the
        // destination), which is drain noise, not search behavior.
        ExpandableObject destDoor =
            (ExpandableObject) DEST_DOOR_FIELD.get(search);
        JsonObject r = obj("dest");
        r.addProperty("phase", phase);
        r.addProperty("found", destDoor != null);
        if (destDoor != null) {
          r.addProperty("doorId", destDoor.getId());
          r.addProperty("section", DEST_SECTION_FIELD.getInt(search));
        }
        row(r);
        k++;
        break;
      }
      k++;
    }
    JsonObject s = obj("drainSummary");
    s.addProperty("phase", phase);
    s.addProperty("pops", k);
    s.addProperty("frontEmpty", front.isEmpty());
    row(s);
  }

  /**
   * A door carrying only its id for the comparator tie probes —
   * ExpandableObject is an interface, so the identity is injective.
   */
  private static final class FakeDoor implements ExpandableObject {
    final int id;

    FakeDoor(int id) {
      this.id = id;
    }

    @Override
    public app.freerouting.geometry.planar.TileShape getShape() {
      return null;
    }

    @Override
    public int getDimension() {
      return 2;
    }

    @Override
    public app.freerouting.autoroute.expansion.CompleteExpansionRoom otherRoom(
        app.freerouting.autoroute.expansion.CompleteExpansionRoom room) {
      return null;
    }

    @Override
    public int mazeSearchElementCount() {
      return 1;
    }

    @Override
    public MazeSearchElement getMazeSearchElement(int index) {
      return new MazeSearchElement();
    }

    @Override
    public void reset() {}

    @Override
    public int getId() {
      return this.id;
    }
  }

  private static MazeListElement mk(ExpandableObject door, int section, double sort, double exp) {
    return new MazeListElement(
        door,
        section,
        null,
        0,
        exp,
        sort,
        null,
        new FloatLine(new FloatPoint(0, 0), new FloatPoint(0, 0)),
        false,
        MazeSearchElement.Adjustment.NONE,
        false);
  }

  /** Dump a TreeSet of MazeListElements as the ordered id list. */
  private static void tieOrder(String probe, String desc, Set<MazeListElement> set) {
    JsonObject o = obj("tie");
    o.addProperty("probe", probe);
    o.addProperty("desc", desc);
    o.addProperty("size", set.size());
    JsonArray arr = new JsonArray(set.size());
    for (MazeListElement e : set) {
      JsonObject el = new JsonObject();
      el.addProperty("id", e.door.getId());
      el.addProperty("section", e.sectionNoOfDoor);
      arr.add(el);
    }
    o.add("order", arr);
    row(o);
  }

  /** The pairwise compareTo verdict (0 / 1 / -1). */
  private static void tieCmp(String probe, MazeListElement a, MazeListElement b) {
    int cmp = a.compareTo(b);
    JsonObject o = obj("tieCmp");
    o.addProperty("probe", probe);
    o.addProperty("cmp", cmp);
    row(o);
  }

  public static void main(String[] p_args) throws Exception {
    if (p_args.length < 1) {
      row(obj("usage-error"));
      System.exit(1);
    }
    byte[] bytes;
    try {
      bytes = Files.readAllBytes(Paths.get(p_args[0]));
    } catch (Exception e) {
      row(obj("read-error"));
      System.exit(2);
      return;
    }
    BoardReadResult read;
    try {
      read =
          DsnReader.readBoard(
              new ByteArrayInputStream(bytes),
              null,
              new ItemIdGenerator(),
              Paths.get(p_args[0]).getFileName().toString());
    } catch (Throwable t) {
      row(obj("parse-error"));
      System.exit(3);
      return;
    }
    if (!(read instanceof BoardReadResult.Success success)) {
      row(obj("read-not-success"));
      System.exit(3);
      return;
    }
    BasicBoard basicBoard = success.board();
    RoutingBoard board = (RoutingBoard) basicBoard;
    board.searchTreeManager.reinsertTreeItems();

    // ---- meta + items --------------------------------------------------
    JsonObject meta = obj("meta");
    IntBox bb = board.boundingBox;
    JsonArray bounds = new JsonArray(4);
    bounds.add(bb.ll.x);
    bounds.add(bb.ll.y);
    bounds.add(bb.ur.x);
    bounds.add(bb.ur.y);
    meta.add("bounds", bounds);
    meta.addProperty("layerCount", board.getLayerCount());
    row(meta);
    for (Item item : board.getItems()) {
      JsonObject it = obj("item");
      it.addProperty("id", item.getId());
      it.addProperty("kind", item.getClass().getSimpleName());
      it.addProperty("netCount", item.netCount());
      row(it);
    }

    // ---- ctrl ----------------------------------------------------------
    int netA = board.rules.nets.get("NET_A", 1).netNumber;
    AutorouteControl ctrl = new AutorouteControl(board, netA, new RouterSettings(board));
    ctrl.viasAllowed = false;
    ctrl.bendCosts[0] = BEND_COST;
    JsonObject ctrlRow = obj("ctrl");
    ctrlRow.addProperty("netNumber", ctrl.netNumber);
    ctrlRow.addProperty("layerCount", ctrl.layerCount);
    ctrlRow.addProperty("viasAllowed", ctrl.viasAllowed);
    ctrlRow.addProperty("bendCost0", d(ctrl.bendCosts[0]));
    ctrlRow.addProperty("bendCost1", d(ctrl.bendCosts[ctrl.layerCount - 1]));
    ctrlRow.addProperty("withNeckdown", ctrl.withNeckdown);
    ctrlRow.addProperty("isFanout", ctrl.isFanout);
    ctrlRow.addProperty("ripupAllowed", ctrl.ripupAllowed);
    ctrlRow.addProperty("maxShoveTraceRecursionDepth", ctrl.maxShoveTraceRecursionDepth);
    JsonArray half = new JsonArray(ctrl.layerCount);
    for (int f = 0; f < ctrl.layerCount; f++) {
      half.add(ctrl.compensatedTraceHalfWidth[f]);
    }
    ctrlRow.add("compensatedTraceHalfWidth", half);
    JsonArray costs = new JsonArray(ctrl.layerCount);
    for (int f = 0; f < ctrl.layerCount; f++) {
      JsonObject cf = new JsonObject();
      cf.addProperty("horizontal", d(ctrl.traceCosts[f].horizontal()));
      cf.addProperty("vertical", d(ctrl.traceCosts[f].vertical()));
      costs.add(cf);
    }
    ctrlRow.add("traceCosts", costs);
    ctrlRow.addProperty(
        "angleRestriction", board.rules.getTraceAngleRestriction().toString());
    ctrlRow.addProperty("minNormalViaCost", d(ctrl.minNormalViaCost));
    ctrlRow.addProperty("minCheapViaCost", d(ctrl.minCheapViaCost));
    row(ctrlRow);

    // ---- engines + searches --------------------------------------------
    // start pin = lower id, destination = higher id (deterministic).
    Pin startPin = null;
    Pin destPin = null;
    for (Item item : board.getItems()) {
      if (!(item instanceof Pin pin) || !item.containsNet(netA)) {
        continue;
      }
      if (startPin == null || pin.getId() < startPin.getId()) {
        destPin = startPin;
        startPin = pin;
      } else {
        destPin = pin;
      }
    }
    JsonObject pinsRow = obj("pins");
    pinsRow.addProperty("startId", startPin != null ? startPin.getId() : -1);
    pinsRow.addProperty("destId", destPin != null ? destPin.getId() : -1);
    row(pinsRow);
    if (startPin == null || destPin == null) {
      row(obj("pins-miss"));
      System.exit(4);
      return;
    }
    final int startPinId = startPin.getId();
    final int destPinId = destPin.getId();

    // Per-phase state isolation: each phase re-parses the DSN into a
    // FRESH board — a maze run mutates item autoroute infos and the
    // shared autoroute tree, and a second engine on the same board
    // fails init against that state (observed: init-failed-drill).
    // The parse ids are deterministic, so item ids stay comparable
    // across phases.
    byte[] phaseBytes = bytes;

    // ---- MAIN phase: fresh board, vias off ------------------------------
    RoutingBoard board1 = parse(phaseBytes, p_args[0]);
    AutorouteControl ctrl1 = new AutorouteControl(board1, netA, new RouterSettings(board1));
    ctrl1.viasAllowed = false;
    ctrl1.bendCosts[0] = BEND_COST;
    Pin start1 = pinById(board1, startPinId);
    Pin dest1 = pinById(board1, destPinId);
    MazeSearchEngine search1 = openSearchSpied(board1, ctrl1, netA, start1, dest1, "MAIN");
    if (search1 == null) {
      row(obj("init-failed-main"));
      System.exit(5);
      return;
    }
    SortedSet<MazeListElement> front1 = search1.mazeExpansionList;

    // The init seed + the start room's doors (before any pop).
    JsonObject seedRow = obj("init");
    seedRow.addProperty("phase", "MAIN");
    seedRow.addProperty("frontSize", front1.size());
    row(seedRow);
    MazeListElement seed = front1.first();
    emitElement("head", "INIT", 0, seed);
    // The start room is a CompleteFreeSpaceExpansionRoom: a
    // CompleteExpansionRoom (the ctor field type) AND a
    // FreeSpaceExpansionRoom (the getDoors owner).
    CompleteExpansionRoom startRoomC = seed.nextRoom;
    FreeSpaceExpansionRoom startRoom = (FreeSpaceExpansionRoom) startRoomC;
    int di = 0;
    for (ExpansionDoor door : startRoom.getDoors()) {
      JsonObject dr = obj("door");
      dr.addProperty("phase", "MAIN");
      dr.addProperty("i", di++);
      dr.addProperty("id", door.getId());
      dr.addProperty("dim", door.getDimension());
      // NOT probed: mazeSearchElementCount() NPEs (sectionArr is
      // allocated lazily by the maze engine); keep the dump read-only.
      dr.addProperty("len", d(door.getShape().boundingBox().maxWidth()));
      row(dr);
    }

    // Destination-distance probes (the T8 seam's real heuristic).
    double[][] dprobes = {
      {200000, 300000, 0},
      {200000, 300000, 1},
      {500000, 300000, 0},
      {10000, 10000, 1},
    };
    for (double[] dp : dprobes) {
      JsonObject dd = obj("dist");
      dd.addProperty("phase", "MAIN");
      dd.addProperty("x", dp[0]);
      dd.addProperty("y", dp[1]);
      dd.addProperty("layer", (int) dp[2]);
      dd.addProperty(
          "value",
          d(search1.destinationDistance.calculate(new FloatPoint(dp[0], dp[1]), (int) dp[2])));
      row(dd);
    }

    // doorIsSmall boundary probes: the strict `<` at exactly the door
    // length W (len → true, len+1 → false).
    Method doorIsSmall =
        MazeSearchEngine.class.getDeclaredMethod("doorIsSmall", ExpansionDoor.class, double.class);
    doorIsSmall.setAccessible(true);
    di = 0;
    for (ExpansionDoor door : startRoom.getDoors()) {
      if (di >= 4) {
        break;
      }
      double len = door.getShape().boundingBox().maxWidth();
      double[] widths = {len - 1.0, len, len + 1.0};
      for (double w : widths) {
        JsonObject s2 = obj("doorIsSmall");
        s2.addProperty("phase", "MAIN");
        s2.addProperty("doorId", door.getId());
        s2.addProperty("w", d(w));
        s2.addProperty("small", (boolean) doorIsSmall.invoke(search1, door, w));
        row(s2);
      }
      di++;
    }

    drain(search1, MAIN_CAP, "MAIN");

    // ---- DRILL phase: fresh board, vias on ------------------------------
    RoutingBoard board2 = parse(phaseBytes, p_args[0]);
    AutorouteControl ctrl2 = new AutorouteControl(board2, netA, new RouterSettings(board2));
    ctrl2.viasAllowed = true;
    ctrl2.bendCosts[0] = BEND_COST;
    MazeSearchEngine search2 =
        openSearch(board2, ctrl2, netA, pinById(board2, startPinId), pinById(board2, destPinId),
            "DRILL");
    if (search2 == null) {
      row(obj("init-failed-drill"));
      System.exit(5);
      return;
    }
    // The page-grid frame (AutorouteEngine ctor): the DRILL rows pin
    // page ids that hang on this grid.
    int maxPageWidth = (int) (5 * board2.rules.getDefaultViaDiameter());
    maxPageWidth = Math.max(maxPageWidth, 10000);
    DrillPageArray pageArray = new DrillPageArray(board2, maxPageWidth);
    JsonObject pm = obj("pageMeta");
    pm.addProperty("defaultViaDiameter", d(board2.rules.getDefaultViaDiameter()));
    pm.addProperty("maxPageWidth", maxPageWidth);
    for (String field : new String[] {"columnCount", "rowCount", "pageWidth", "pageHeight"}) {
      java.lang.reflect.Field f = DrillPageArray.class.getDeclaredField(field);
      f.setAccessible(true);
      pm.addProperty(field, f.getInt(pageArray));
    }
    row(pm);
    drain(search2, DRILL_CAP, "DRILL");

    // ---- BEND phases: seeded elements with a backtrack door -------------
    // A fresh board per probe (the seed occupation marks door sections;
    // the straight/diag pair must not interfere). A seed door element
    // with a DIAGONAL shape entry + a live backtrack door provokes the
    // bend penalty (cross product) on the expansions; bendCosts[0]=47.
    for (String bendPhase : new String[] {"BEND_STRAIGHT", "BEND_DIAG"}) {
      RoutingBoard boardB = parse(phaseBytes, p_args[0]);
      AutorouteControl ctrlB = new AutorouteControl(boardB, netA, new RouterSettings(boardB));
      ctrlB.viasAllowed = false;
      ctrlB.bendCosts[0] = BEND_COST;
      MazeSearchEngine searchB =
          openSearch(boardB, ctrlB, netA, pinById(boardB, startPinId), pinById(boardB, destPinId),
              bendPhase);
      if (searchB == null) {
        row(obj("init-failed-" + bendPhase));
        continue;
      }
      SortedSet<MazeListElement> frontB = searchB.mazeExpansionList;
      MazeListElement seedTarget = frontB.first();
      CompleteExpansionRoom roomBC = seedTarget.nextRoom;
      FreeSpaceExpansionRoom roomB = (FreeSpaceExpansionRoom) roomBC;
      List<ExpansionDoor> doors = roomB.getDoors();
      if (doors.isEmpty()) {
        row(obj("no-doors-" + bendPhase));
        continue;
      }
      ExpansionDoor door = doors.get(0);
      // allocateSections fires inside getSectionSegments — the
      // production precondition for reading sectionArr
      // (MazeSearchEngine.expandToDoor:715). The seed occupation pops
      // through the same read.
      int seedLayer = seedTarget.nextRoom.getLayer();
      double seedHalfWidth = ctrlB.compensatedTraceHalfWidth[seedLayer];
      FloatLine[] seedSections = door.getSectionSegments(seedHalfWidth);
      // The door's shape entry: straight = axis-parallel chord through
      // the door center; diag = a 45-degree chord of the same span.
      FloatPoint c = door.getShape().centreOfGravity();
      FloatLine entry;
      if (bendPhase.equals("BEND_DIAG")) {
        entry = new FloatLine(new FloatPoint(c.x - 2000, c.y - 2000), c);
      } else {
        entry = new FloatLine(new FloatPoint(c.x - 2000, c.y), c);
      }
      MazeListElement seedElem =
          new MazeListElement(
              door,
              0,
              seedTarget.door,
              seedTarget.sectionNoOfDoor,
              100.0,
              100.0,
              roomBC,
              entry,
              false,
              MazeSearchElement.Adjustment.NONE,
              false);
      frontB.clear();
      frontB.add(seedElem);
      int before = frontB.size();
      searchB.occupyNextElement();
      JsonObject bs = obj("bendSeed");
      bs.addProperty("phase", bendPhase);
      bs.addProperty("doorId", door.getId());
      bs.addProperty("sectionCount", seedSections.length);
      bs.addProperty("before", before);
      bs.addProperty("after", frontB.size());
      row(bs);
      int k = 0;
      for (MazeListElement e : frontB) {
        emitElement("bendElem", bendPhase, k, e);
        k++;
      }
    }

    // ---- TIE probes: the comparator's 4-level chain + NaN/-0.0 ----------
    TreeSet<MazeListElement> ts = new TreeSet<>();
    // T1 sortingValue decides.
    ts.clear();
    ts.add(mk(new FakeDoor(31), 0, 10.0, 5.0));
    ts.add(mk(new FakeDoor(7), 0, 20.0, 5.0));
    tieOrder("T1", "sort 10 vs 20: order by sortingValue", ts);
    // T2 expansionValue decides.
    ts.clear();
    ts.add(mk(new FakeDoor(31), 0, 10.0, 6.0));
    ts.add(mk(new FakeDoor(7), 0, 10.0, 5.0));
    tieOrder("T2", "sort equal: order by expansionValue", ts);
    // T3 door id decides.
    ts.clear();
    ts.add(mk(new FakeDoor(31), 0, 10.0, 5.0));
    ts.add(mk(new FakeDoor(7), 0, 10.0, 5.0));
    tieOrder("T3", "values equal: order by doorId", ts);
    // T4 sectionNoOfDoor decides (same id, different sections).
    ts.clear();
    ts.add(mk(new FakeDoor(31), 3, 10.0, 5.0));
    ts.add(mk(new FakeDoor(31), 1, 10.0, 5.0));
    tieOrder("T4", "id equal: order by section", ts);
    // T5 full tie: the set DEDUPS.
    ts.clear();
    ts.add(mk(new FakeDoor(31), 2, 10.0, 5.0));
    boolean first = ts.add(mk(new FakeDoor(31), 2, 10.0, 5.0));
    JsonObject t5 = obj("tie");
    t5.addProperty("probe", "T5");
    t5.addProperty("desc", "full tie: second add rejected");
    t5.addProperty("size", ts.size());
    t5.addProperty("secondAddAccepted", first);
    row(t5);
    // T6 NaN falls through the value tie-breaks (pairwise verdicts).
    MazeListElement na = mk(new FakeDoor(3), 0, Double.NaN, 0.0);
    MazeListElement nb = mk(new FakeDoor(9), 0, 0.0, 0.0);
    MazeListElement nc = mk(new FakeDoor(1), 0, 1.0, 0.0);
    tieCmp("T6-na-nb", na, nb);
    tieCmp("T6-nb-na", nb, na);
    tieCmp("T6-nb-nc", nb, nc);
    tieCmp("T6-na-na", na, mk(new FakeDoor(3), 0, Double.NaN, 0.0));
    // T7 -0.0 ties 0.0 (full tie with same id/section → dedup).
    MazeListElement nz = mk(new FakeDoor(31), 2, -0.0, 5.0);
    MazeListElement pz = mk(new FakeDoor(31), 2, 0.0, 5.0);
    tieCmp("T7-nz-pz", nz, pz);
    ts.clear();
    ts.add(nz);
    boolean second = ts.add(pz);
    JsonObject t7 = obj("tie");
    t7.addProperty("probe", "T7");
    t7.addProperty("desc", "-0.0 vs 0.0: tie, second add rejected");
    t7.addProperty("size", ts.size());
    t7.addProperty("secondAddAccepted", second);
    row(t7);

    // ---- T8 crafted destination-distance worlds ------------------------
    // Each world = one ctor + its join/calc/point probe battery; the
    // ddCtor rows expose the raw derived fields (the 0.0-quirk
    // worlds Q/Qs vs the all-active witness A). Via costs 400/320
    // match the T3 control constants.
    double normalVia = 400.0;
    double cheapVia = 320.0;
    double[][] c4 = {{1.0, 2.7}, {1.6, 1.0}, {2.0, 3.0}, {1.0, 1.0}};
    double[][] c3 = {{1.0, 2.7}, {1.6, 1.0}, {2.0, 3.0}};
    double[][] c2 = {{1.0, 2.7}, {1.6, 1.0}};
    double[][] c1 = {{1.0, 2.7}};
    double[][] cE = {{2.0, 3.0}, {1.0, 2.0}};
    double[][] cF = {{1.0, 2.0}, {2.0, 3.0}};
    // A: 4 layers all active — the full-ladder fallthrough world +
    // inner-layer probes + the cheap/restore/sentinel rows.
    ddWorld("A", c4, new boolean[] {true, true, true, true}, normalVia, cheapVia,
        new int[] {0, 1, 3}, new int[] {0, 1, 3});
    // B: 3 layers — the `==3` early returns.
    ddWorld("B", c3, new boolean[] {true, true, true}, normalVia, cheapVia,
        new int[] {0, 1, 2}, new int[] {0, 1, 2});
    // C: 2 layers — the `==2` (component branch) gate.
    ddWorld("C", c2, new boolean[] {true, true}, normalVia, cheapVia,
        new int[] {0, 1}, new int[] {0, 1});
    // D1: 1 layer active — the `<=1` gate with count 1.
    ddWorld("D1", c1, new boolean[] {true}, normalVia, cheapVia,
        new int[] {0}, new int[] {0});
    // D0: 1 layer INACTIVE — the `<=1` gate with count 0 (the `==1`
    // mutant falls through and shrinks the result) + the all-zero
    // cost quirk.
    ddWorld("D0", c1, new boolean[] {false}, normalVia, cheapVia,
        new int[] {0}, new int[] {0});
    // E: 2 layers, SOLDER cheaper — the component-branch pair TRUE arm.
    ddWorld("E", cE, new boolean[] {true, true}, normalVia, cheapVia,
        new int[] {0, 1}, new int[] {0, 1});
    // F: 2 layers, COMPONENT cheaper — the solder-branch pair TRUE arm.
    ddWorld("F", cF, new boolean[] {true, true}, normalVia, cheapVia,
        new int[] {0, 1}, new int[] {0, 1});
    // G: 2 layers, solder INACTIVE — the solder branch's `<=2` gate
    // with count 1 (a `==2` mutant falls through) + the solder-side
    // 0.0 quirk on a 2-layer board.
    ddWorld("G", c2, new boolean[] {true, false}, normalVia, cheapVia,
        new int[] {0}, new int[] {0, 1});
    // Q: 4 layers, layer 0 INACTIVE — THE 0.0-quirk world (component
    // fields stay 0.0; maxInner = min(0, solder) = 0 propagates).
    ddWorld("Q", c4, new boolean[] {false, true, true, true}, normalVia, cheapVia,
        new int[] {0, 1, 3}, new int[] {0, 1, 3});
    // Qs: 4 layers, SOLDER inactive — the mirrored quirk contrast.
    ddWorld("Qs", c4, new boolean[] {true, true, true, false}, normalVia, cheapVia,
        new int[] {0, 1, 3}, new int[] {0, 1, 3});
    // H: 2 layers, SOLDER CHEAP (3.0/4.0 vs 1.0/1.1) — the
    // component-branch `==2` discriminator: at count 2 the gate
    // returns the two-vias arm (134800 on P5), the deletion mutant
    // falls through to the :272 arm (130800, mcsi*solder_min < 3.0*
    // solder_min). TGT-only worlds cannot shrink there.
    ddWorld("H", new double[][] {{3.0, 4.0}, {1.0, 1.1}}, new boolean[] {true, true},
        normalVia, cheapVia, new int[] {0, 1}, new int[] {0, 1});
    // M: 3 layers, ALL costs > 1 (min 1.5) — the `==3` discriminators
    // (both branches): differing boxes per bucket (FAR TGT component,
    // NEAR inner, MID solder) make the post-`==3` arms diverge from
    // the pre-gate arms. P2@L0: `==3` returns the :265 inner-pair arm
    // 12900 (inner box strictly outside BOTH axes: deltas (5000,
    // 5000), inner_min > 800 so :285's unit-weighted arm undercuts);
    // the deletion mutant falls to :285 (10800). P5@L2: `==3`
    // returns the :331 component arm 150800; deletion falls to :343
    // (131200).
    ddWorld("M", new double[][] {{1.5, 2.0}, {1.6, 1.6}, {1.7, 2.0}},
        new boolean[] {true, true, true}, normalVia, cheapVia,
        new int[] {0}, new int[] {0, 2},
        new int[][] {
          {1, 405000, 305000, 410000, 310000},
          {2, 500000, 200000, 600000, 300000},
        });
    // G3: 3 layers, ONLY layer 0 active (the solder-side 0.0-quirk on
    // a count-1 board with an inner bucket) — the solder-branch
    // `<=2` discriminator: at count 1 the gate returns the pair arm
    // 40400 on P5; the `==2` mutant falls through to the :329 inner
    // arm (inner box NEAR the probe → 30400).
    ddWorld("G3", new double[][] {{1.0, 2.7}, {1.6, 1.0}, {2.0, 3.0}},
        new boolean[] {true, false, false}, normalVia, cheapVia,
        new int[] {0}, new int[] {2},
        new int[][] {{1, 85000, 140000, 95000, 150000}});

    // ---- T8 quality-round worlds (MQ1/MQ2/MQ3/MQ5) ----------------
    // R: 3 layers with a CHEAP inner layer — the ctor 8th-field
    // discriminator (MQ5): maxInner = min(6, 1, 6) = 1.0 so
    // minComponentSolderInner = min(minComponentInner, minSolderInner)
    // = min(1, 1) = 1.0, while the wrong formula
    // min(minComponent, minSolder) = min(5, 5) = 5.0 coincides with
    // the right one on EVERY other world (H: min(1.1,1)=min(3,1)=1;
    // M: min(1.5,1.6)=min(1.5,1.7)=1.5). Ctor-only world.
    ddWorld("R", new double[][] {{5.0, 6.0}, {1.0, 1.0}, {5.0, 6.0}},
        new boolean[] {true, true, true}, normalVia, cheapVia,
        new int[0], new int[0]);
    // W: 4 layers, FAR component bucket, TGT solder bucket, inner
    // bucket EMPTY — the inner-branch tail discriminator (MQ2): P0@L1
    // skips the weighted arm (inner empty → result starts at the
    // sentinel) and the :369 solder-pair arm wins STRICTLY at
    // solder_max + solder_min*minSolderInner + via = 0 + 0*c + 400 =
    // 400.0 (every other arm carries component deltas ~7e5, inner
    // deltas ~1e9, or an extra via). The truncating mutant returns
    // :377's 800.0. MQ3 shares the template (see X).
    ddWorld("W", c4, new boolean[] {true, true, true, true}, normalVia, cheapVia,
        new int[] {3}, new int[] {1},
        new int[][] {{0, 900000, 900000, 950000, 950000}});
    // X: 4 layers ALL-EXPENSIVE (5/6, so minSolderInner =
    // minCompSolderInner = 5 > 1), differing buckets (NEAR-mid
    // component, FAR solder, empty inner) — the solder `==3` count-4
    // face discriminator (MQ3): P4@L3 returns the FOUR-layer arm :345
    // (comp_max + comp_min + 3*via = 50000 + 50000 + 1200 = 101200),
    // strictly under :330 (50000 + 5*50000 + 800 = 300800), :335
    // (solder deltas 500000/400000 + 800), :306 (500400), :318
    // (2500800) and the weighted arm (~3.5e6). A `==3`→`<=3` mutant
    // early-returns 300800. (A is count 4 too, but its L3 pin is
    // decided by the weighted arm — the face was unobserved.)
    ddWorld("X", new double[][] {{5.0, 6.0}, {5.0, 6.0}, {5.0, 6.0}, {5.0, 6.0}},
        new boolean[] {true, true, true, true}, normalVia, cheapVia,
        new int[0], new int[] {3},
        new int[][] {
          {0, 450000, 550000, 550000, 650000},
          {3, 900000, 900000, 950000, 950000},
        });
    // MJ: two DIFFERENT boxes joined into ONE bucket — the join
    // accumulation discriminator (MQ1): the component bucket must end
    // as union(TGT, [300000,300000,400000,400000]) =
    // (100000,200000)-(400000,400000) (ddBucket MJ row); the
    // last-join-wins mutant leaves (300000,300000)-(400000,400000).
    // Join-only world (no probes).
    ddWorld("MJ", c2, new boolean[] {true, true}, normalVia, cheapVia,
        new int[] {0}, new int[0],
        new int[][] {{0, 300000, 300000, 400000, 400000}});

    row(obj("done"));
  }

  /** A fresh parse of the fixture bytes (per-phase board isolation). */
  private static RoutingBoard parse(byte[] bytes, String fileName) throws Exception {
    BoardReadResult read =
        DsnReader.readBoard(
            new ByteArrayInputStream(bytes), null, new ItemIdGenerator(), fileName);
    if (!(read instanceof BoardReadResult.Success success)) {
      throw new IllegalStateException("reparse failed");
    }
    BasicBoard basicBoard = success.board();
    RoutingBoard board = (RoutingBoard) basicBoard;
    board.searchTreeManager.reinsertTreeItems();
    return board;
  }

  /** The pin with the given id, or null. */
  private static Pin pinById(RoutingBoard board, int id) {
    for (Item item : board.getItems()) {
      if (item instanceof Pin pin && pin.getId() == id) {
        return pin;
      }
    }
    return null;
  }

  /**
   * initConnection + the incompleteExpansionRooms reset (the
   * production-state mirror: by the time a maze runs, earlier pipeline
   * phases have populated the list — with a fresh null list the
   * completion path's remove NPEs and init fails) + getInstance.
   */
  private static MazeSearchEngine openSearch(
      RoutingBoard board,
      AutorouteControl ctrl,
      int net,
      Pin start,
      Pin dest,
      String phase)
      throws Exception {
    AutorouteEngine engine = new AutorouteEngine(board, ctrl.viaClearanceClass, true);
    engine.initConnection(net, null, null);
    java.lang.reflect.Field incompleteField =
        AutorouteEngine.class.getDeclaredField("incompleteExpansionRooms");
    incompleteField.setAccessible(true);
    incompleteField.set(engine, new ArrayList<>());
    MazeSearchEngine search =
        MazeSearchEngine.getInstance(Set.of(start), Set.of(dest), engine, ctrl);
    JsonObject o = obj("searchOpen");
    o.addProperty("phase", phase);
    o.addProperty("ok", search != null);
    row(o);
    return search;
  }
}
