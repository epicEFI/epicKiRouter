// RipupSpike.java — the M3-T7 jar-side probe oracle for the ripup
// resolver (MazeRipupResolver), the READ-ONLY shove probe
// (MazeTraceShover.checkShoveTraceLine) and the java.util.Random
// stream the engine seeds with ctrl.ripupCosts. MazeSpike house
// pattern: ONE JVM per run, deterministic JSONL rows on stdout (lines
// starting with "{" are results; the jar's own noise is interleaved
// and dropped by the consumer).
//
// Build/run (JDK 25, from the repo root):
//   mkdir -p /tmp/epic-maze-classes && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/javac \
//       -cp build/libs/freerouting-current-executable.jar \
//       -d /tmp/epic-maze-classes rust/harness/oracle/RipupSpike.java && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/java \
//       -Duser.language=en -Duser.country=US \
//       -cp build/libs/freerouting-current-executable.jar:/tmp/epic-maze-classes \
//       app.freerouting.autoroute.maze.RipupSpike \
//       rust/harness/fixtures/maze-spike/t7_ripup.dsn
//
// The oracle declares the app.freerouting.autoroute.maze package so
// the package-private MazeSearchEngine ctor + fields, MazeRipupResolver
// (ctor + checkRipup + checkLeavingRippedItem), the MazeListElement
// fields and the MazeTraceShover.DoorSection fields are directly
// reachable. `enterThroughSmallDoor` is PRIVATE and is invoked through
// reflection (the doorIsSmall Field precedent).
//
// The FRLogger CHECK_RIPUP payload (MazeRipupResolver.java:173-195) IS
// the ripup capture format: a log4j appender on the freerouting logger
// turns every CHECK_RIPUP message into a row, in emission order, so
// BOTH the engine's own ripup calls (unmarked) and the spike's DIRECT
// resolver calls (each preceded by a `call` marker row) land in one
// deterministic stream. All doubles travel as Double.toString (exact
// round-trip); rows are emitted from TreeSet/front iteration only; no
// HashSet/HashMap iteration reaches a row.
package app.freerouting.autoroute.maze;

import app.freerouting.autoroute.expansion.ExpansionDoor;
import app.freerouting.autoroute.expansion.ObstacleExpansionRoom;
import app.freerouting.board.actions.ItemIdGenerator;
import app.freerouting.board.facade.BasicBoard;
import app.freerouting.board.facade.RoutingBoard;
import app.freerouting.board.model.items.Item;
import app.freerouting.board.model.items.Pin;
import app.freerouting.board.model.items.Trace;
import app.freerouting.board.trace.PolylineTrace;
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
import java.nio.file.Files;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.LinkedList;
import java.util.List;
import java.util.Set;
import org.apache.logging.log4j.Level;
import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.core.LogEvent;
import org.apache.logging.log4j.core.LoggerContext;
import org.apache.logging.log4j.core.appender.AbstractAppender;
import org.apache.logging.log4j.core.config.Property;

public class RipupSpike {

  private static final Gson GSON = new Gson();
  private static final int CAP = 600;
  private static final int[] PASSES = {1, 3, 4, 4, 6, 7};
  private static final String[] SEED_PHASES = {"SEED2", "SEED10", "PROBE"};

  /** Emitted synchronously; stdout is the single ordered sink. */
  private static synchronized void row(JsonObject o) {
    System.out.println(GSON.toJson(o));
  }

  private static JsonObject obj(String type) {
    JsonObject o = new JsonObject();
    o.addProperty("type", type);
    return o;
  }

  private static String d(double v) {
    return Double.toString(v);
  }

  /**
   * Turns a FRLogger CHECK_RIPUP message (MazeRipupResolver.java:173-195)
   * into a row. Message shape: `CHECK_RIPUP net=N, k=v, ...` with all
   * values `, `-separated `k=v` pairs; lists are `[a, b]` (Java
   * Arrays.toString, always space after comma).
   */
  private static void emitCheckRipup(String msg) {
    JsonObject o = obj("check_ripup");
    String body = msg.substring("CHECK_RIPUP ".length());
    String[] pairs = body.split(", ");
    for (String pair : pairs) {
      int eq = pair.indexOf('=');
      if (eq < 0) {
        continue;
      }
      String key = pair.substring(0, eq);
      String val = pair.substring(eq + 1);
      switch (key) {
        case "net", "obstacle_id", "ripupCosts", "itemCount", "result" ->
            o.addProperty(key, Integer.parseInt(val));
        case "halfWidth", "traceLength", "minTraceLength", "detour" ->
            o.addProperty(key, Double.parseDouble(val));
        case "obstacle_nets", "connectionItems" -> o.addProperty(key, val);
        default -> o.addProperty(key, val);
      }
    }
    row(o);
  }

  private static final java.util.concurrent.atomic.AtomicInteger TAP_COUNT =
      new java.util.concurrent.atomic.AtomicInteger();

  /** The log4j tap: every CHECK_RIPUP trace message becomes a row. */
  private static void attachRipupTap() {
    LoggerContext ctx = (LoggerContext) LogManager.getContext(false);
    AbstractAppender tap =
        new AbstractAppender("ripup-spike-tap", null, null, true, Property.EMPTY_ARRAY) {
          @Override
          public void append(LogEvent event) {
            TAP_COUNT.incrementAndGet();
            String m = event.getMessage().getFormattedMessage();
            if (m != null && m.startsWith("CHECK_RIPUP ")) {
              emitCheckRipup(m);
            }
          }
        };
    tap.start();
    // The LOGGER's level is not enough: dispatch filters on the
    // LoggerConfig (the jar's config roots at a higher level). Register
    // a dedicated ALL-level LoggerConfig for the freerouting logger and
    // rebind.
    org.apache.logging.log4j.core.config.Configuration config =
        ctx.getConfiguration();
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
    // Control experiment: root-config appender + direct core trace.
    config.getRootLogger().addAppender(tap, Level.ALL, null);
    ctx.updateLoggers();
    org.apache.logging.log4j.core.Logger core =
        (org.apache.logging.log4j.core.Logger)
            LogManager.getLogger(app.freerouting.Freerouting.class);
    core.addAppender(tap);
    core.trace("CHECK_RIPUP direct=1, net=0, result=0");
    // Self-test: the tap must see its own probe rows (the two
    // `direct=1` / `tap=ok` check_ripup rows at the head of the run).
    app.freerouting.logger.FRLogger.trace("CHECK_RIPUP tap=ok, net=0, result=0");
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

  /**
   * initConnection + the incompleteExpansionRooms reset + getInstance
   * (the MazeSpike.openSearch plumbing).
   */
  private static MazeSearchEngine openSearch(
      RoutingBoard board, AutorouteControl ctrl, int net, Pin start, Pin dest) throws Exception {
    AutorouteEngine engine = new AutorouteEngine(board, ctrl.viaClearanceClass, true);
    engine.initConnection(net, null, null);
    java.lang.reflect.Field incompleteField =
        AutorouteEngine.class.getDeclaredField("incompleteExpansionRooms");
    incompleteField.setAccessible(true);
    incompleteField.set(engine, new ArrayList<>());
    return MazeSearchEngine.getInstance(Set.of(start), Set.of(dest), engine, ctrl);
  }

  private static void emitItemDump(RoutingBoard board) {
    for (Item item : board.getItems()) {
      JsonObject it = obj("item");
      it.addProperty("id", item.getId());
      it.addProperty("kind", item.getClass().getSimpleName());
      it.addProperty("netCount", item.netCount());
      JsonArray nets = new JsonArray(item.netCount());
      for (int i = 0; i < item.netCount(); i++) {
        nets.add(item.getNetNumber(i));
      }
      it.add("nets", nets);
      if (item instanceof Pin pin) {
        FloatPoint c = pin.getCenter().toFloat();
        it.addProperty("cx", d(c.x));
        it.addProperty("cy", d(c.y));
      }
      if (item instanceof Trace trace) {
        it.addProperty("halfWidth", trace.getHalfWidth());
        it.addProperty("length", d(trace.getLength()));
      }
      if (item instanceof PolylineTrace pt) {
        it.addProperty("cornerCount", pt.cornerCount());
        it.addProperty("lines", pt.polyline().lines.length);
      }
      row(it);
    }
  }

  private static void emitCtrl(AutorouteControl ctrl) {
    JsonObject o = obj("ctrl");
    o.addProperty("netNumber", ctrl.netNumber);
    o.addProperty("layerCount", ctrl.layerCount);
    o.addProperty("ripupAllowed", ctrl.ripupAllowed);
    o.addProperty("ripupCosts", ctrl.ripupCosts);
    o.addProperty("ripupPassNo", ctrl.ripupPassNo);
    o.addProperty("startRipupCosts", ctrl.settings.getStartRipupCosts());
    o.addProperty("removeUnconnectedVias", ctrl.removeUnconnectedVias);
    o.addProperty("isFanout", ctrl.isFanout);
    o.addProperty("viasAllowed", ctrl.viasAllowed);
    o.addProperty("traceClearanceClassIndex", ctrl.traceClearanceClassIndex);
    o.addProperty("maxShoveTraceRecursionDepth", ctrl.maxShoveTraceRecursionDepth);
    o.addProperty("maxShoveViaRecursionDepth", ctrl.maxShoveViaRecursionDepth);
    JsonArray half = new JsonArray(ctrl.layerCount);
    JsonArray comp = new JsonArray(ctrl.layerCount);
    JsonArray traceHalf = new JsonArray(ctrl.layerCount);
    for (int f = 0; f < ctrl.layerCount; f++) {
      comp.add(ctrl.compensatedTraceHalfWidth[f]);
      traceHalf.add(ctrl.traceHalfWidth[f]);
      half.add(ctrl.layerActive[f]);
    }
    o.add("compensatedTraceHalfWidth", comp);
    o.add("traceHalfWidth", traceHalf);
    o.add("layerActive", half);
    row(o);
  }

  /** Raw nextDouble probes of a FRESH generator (not the engine's). */
  private static void emitRngProbe(int seed) {
    java.util.Random probe = new java.util.Random(seed);
    for (int i = 0; i < 5; i++) {
      JsonObject o = obj("rng");
      o.addProperty("seed", seed);
      o.addProperty("i", i);
      o.addProperty("v", d(probe.nextDouble()));
      row(o);
    }
  }

  private static Pin pinNearest(RoutingBoard board, int targetX) {
    Pin best = null;
    int bestDist = Integer.MAX_VALUE;
    for (Item item : board.getItems()) {
      if (item instanceof Pin pin) {
        int dist = (int) Math.abs(pin.getCenter().toFloat().x - targetX);
        if (dist < bestDist) {
          bestDist = dist;
          best = pin;
        }
      }
    }
    return best;
  }

  private static JsonObject doorSectionRow(String type, String phase, MazeTraceShover.DoorSection ds) {
    JsonObject o = obj(type);
    o.addProperty("phase", phase);
    o.addProperty("doorId", ds.door.getId());
    o.addProperty("sectionIndex", ds.sectionIndex);
    o.addProperty("dimension", ds.door.dimension);
    o.addProperty("ax", d(ds.sectionLine.a.x));
    o.addProperty("ay", d(ds.sectionLine.a.y));
    o.addProperty("bx", d(ds.sectionLine.b.x));
    o.addProperty("by", d(ds.sectionLine.b.y));
    return o;
  }

  /**
   * The per-element direct probes: checkRipup under every PASSES entry
   * (the RNG lifetime witness = the two pass-4 rows of one engine),
   * checkLeavingRippedItem + enterThroughSmallDoor on 1-dim doors, the
   * shove probe (both directions) on trace obstacle rooms.
   */
  private static void probeElement(
      String phase,
      MazeSearchEngine search,
      AutorouteControl ctrl,
      RoutingBoard board,
      MazeRipupResolver resolver,
      MazeListElement e,
      ObstacleExpansionRoom oer,
      Pin destPin,
      java.lang.reflect.Method smallDoor)
      throws Exception {
    Item obstacle = oer.getItem();
    int obstacleId = obstacle.getId();

    for (int pass : PASSES) {
      JsonObject marker = obj("call");
      marker.addProperty("phase", phase);
      marker.addProperty("pass", pass);
      marker.addProperty("seed", ctrl.ripupCosts);
      marker.addProperty("obstacle", obstacleId);
      marker.addProperty("cornerNo", oer.getIndexInItem());
      row(marker);
      ctrl.ripupPassNo = pass;
      int cost = resolver.checkRipup(e, obstacle, false);
      JsonObject res = obj("call_result");
      res.addProperty("phase", phase);
      res.addProperty("pass", pass);
      res.addProperty("obstacle", obstacleId);
      res.addProperty("cost", cost);
      row(res);
    }

    // --- Surgical gate probes (gates the drain never reaches) -------
    // isRoutable -1: the protect wire (17) fails the FIRST gate.
    JsonObject g1 = obj("gate_probe");
    g1.addProperty("phase", phase);
    g1.addProperty("probe", "not_routable");
    g1.addProperty("obstacle", 17);
    g1.addProperty("cost", resolver.checkRipup(e, itemById(board, 17), false));
    row(g1);
    // Via-arm -1: VB2 (19) has a non-Trace contact (pin), VB3 (20) a
    // userFixed trace contact.
    JsonObject g2 = obj("gate_probe");
    g2.addProperty("phase", phase);
    g2.addProperty("probe", "via_nontrace_contact");
    g2.addProperty("obstacle", 19);
    g2.addProperty("cost", resolver.checkRipup(e, itemById(board, 19), false));
    row(g2);
    JsonObject g3 = obj("gate_probe");
    g3.addProperty("phase", phase);
    g3.addProperty("probe", "via_userfixed_contact");
    g3.addProperty("obstacle", 20);
    g3.addProperty("cost", resolver.checkRipup(e, itemById(board, 20), false));
    row(g3);
    // Fanout-protect economics on wB1 (9): protection holds ONLY while
    // ctrl.ripupCosts <= startRipupCosts*2 (=2 here) — raising costs
    // disables it. SEED2 (protection ON, costs=2): the detour block is
    // SKIPPED (fanout factor 3472.2 > 1), detour stays 1, and the
    // result = 2*10000*3472.22 = 69444444. PROBE/SEED10 (protection
    // OFF): the block runs (MAX detour), the factor is absent, result
    // floors to 1.
    if (phase.equals("SEED2") || phase.equals("PROBE")) {
      JsonObject marker9 = obj("call");
      marker9.addProperty("phase", phase);
      marker9.addProperty("pass", ctrl.ripupPassNo);
      marker9.addProperty("seed", ctrl.ripupCosts);
      marker9.addProperty("obstacle", 9);
      marker9.addProperty("cornerNo", -1);
      row(marker9);
      int cost9 = resolver.checkRipup(e, itemById(board, 9), false);
      JsonObject res9 = obj("call_result");
      res9.addProperty("phase", phase);
      res9.addProperty("pass", ctrl.ripupPassNo);
      res9.addProperty("obstacle", 9);
      res9.addProperty("cost", cost9);
      row(res9);
    }
    // ALREADY_RIPPED: a synthetic element whose door's OTHER room is
    // the obstacle room itself — previousItem == obstacleItem → cost 1
    // BEFORE any economics (and no check_ripup log row: the branch
    // returns early). The absence of the log row between the marker
    // and the result is the capture discriminator vs the max-floor.
    if (e.door instanceof ExpansionDoor ed3 && e.nextRoom != null) {
      app.freerouting.autoroute.expansion.CompleteExpansionRoom otherSide =
          ed3.otherRoom(e.nextRoom);
      if (otherSide != null) {
        MazeListElement synth =
            new MazeListElement(
                e.door,
                e.sectionNoOfDoor,
                null,
                0,
                0.0,
                0.0,
                otherSide,
                e.shapeEntry,
                false,
                MazeSearchElement.Adjustment.NONE,
                false);
        JsonObject markerA = obj("call");
        markerA.addProperty("phase", phase);
        markerA.addProperty("pass", ctrl.ripupPassNo);
        markerA.addProperty("seed", ctrl.ripupCosts);
        markerA.addProperty("obstacle", obstacleId);
        markerA.addProperty("cornerNo", -2);
        row(markerA);
        int alreadyCost = resolver.checkRipup(synth, obstacle, false);
        JsonObject resA = obj("already_ripped");
        resA.addProperty("phase", phase);
        resA.addProperty("obstacle", obstacleId);
        resA.addProperty("cost", alreadyCost);
        row(resA);
      }
    }

    if (e.door instanceof ExpansionDoor ed && ed.dimension == 1) {
      JsonObject lv = obj("leaving");
      lv.addProperty("phase", phase);
      lv.addProperty("obstacle", obstacleId);
      lv.addProperty("verdict", resolver.checkLeavingRippedItem(e));
      row(lv);
      for (Item ignore : new Item[] {obstacle, destPin}) {
        boolean v = (Boolean) smallDoor.invoke(resolver, e, ignore);
        JsonObject sd = obj("small_door");
        sd.addProperty("phase", phase);
        sd.addProperty("obstacle", obstacleId);
        sd.addProperty("ignoreId", ignore.getId());
        sd.addProperty("verdict", v);
        sd.addProperty("checkRadius", ctrl.compensatedTraceHalfWidth[oer.getLayer()] + 2);
        FloatPoint[] corners = new FloatPoint[ed.getShape().borderLineCount()];
        JsonArray arr = new JsonArray(corners.length);
        for (int i = 0; i < corners.length; i++) {
          FloatPoint c = ed.getShape().cornerApprox(i);
          JsonObject cj = new JsonObject();
          cj.addProperty("x", d(c.x));
          cj.addProperty("y", d(c.y));
          arr.add(cj);
        }
        sd.add("doorCorners", arr);
        row(sd);
      }
    }

    if (obstacle instanceof PolylineTrace pt) {
      for (boolean left : new boolean[] {false, true}) {
        List<MazeTraceShover.DoorSection> toDoors = new LinkedList<>();
        boolean verdict = MazeTraceShover.checkShoveTraceLine(e, oer, board, ctrl, left, toDoors);
        JsonObject sv = obj("shove");
        sv.addProperty("phase", phase);
        sv.addProperty("obstacle", obstacleId);
        sv.addProperty("left", left);
        sv.addProperty("verdict", verdict);
        sv.addProperty("cornerNo", oer.getIndexInItem());
        sv.addProperty("lines", pt.polyline().lines.length);
        sv.addProperty("halfWidth", pt.getHalfWidth());
        sv.addProperty("ctrlHalfWidth", ctrl.traceHalfWidth[oer.getLayer()]);
        sv.addProperty("doorMaxWidth", d(ed(e).getShape().maxWidth()));
        sv.addProperty("toDoors", toDoors.size());
        row(sv);
        for (MazeTraceShover.DoorSection ds : toDoors) {
          row(doorSectionRow("shove_door", phase, ds));
        }
      }
    } else {
      List<MazeTraceShover.DoorSection> toDoors = new LinkedList<>();
      boolean verdict = MazeTraceShover.checkShoveTraceLine(e, oer, board, ctrl, false, toDoors);
      JsonObject sv = obj("shove_nontrace");
      sv.addProperty("phase", phase);
      sv.addProperty("obstacle", obstacleId);
      sv.addProperty("kind", obstacle.getClass().getSimpleName());
      sv.addProperty("verdict", verdict);
      row(sv);
    }
  }

  /** The element's door as an ExpansionDoor (probe precondition). */
  private static ExpansionDoor ed(MazeListElement e) {
    return (ExpansionDoor) e.door;
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
    attachRipupTap();

    // ---- meta + items over the first parse -----------------------------
    RoutingBoard board0 = parse(bytes, Paths.get(p_args[0]).getFileName().toString());
    JsonObject meta = obj("meta");
    IntBox bb = board0.boundingBox;
    JsonArray bounds = new JsonArray(4);
    bounds.add(bb.ll.x);
    bounds.add(bb.ll.y);
    bounds.add(bb.ur.x);
    bounds.add(bb.ur.y);
    meta.add("bounds", bounds);
    meta.addProperty("layerCount", board0.getLayerCount());
    row(meta);
    emitItemDump(board0);

    // ---- the three phases ----------------------------------------------
    int[] seeds = {2, 10, 1000};
    for (int p = 0; p < SEED_PHASES.length; p++) {
      String phase = SEED_PHASES[p];
      int seed = seeds[p];
      RoutingBoard board = parse(bytes, Paths.get(p_args[0]).getFileName().toString());
      int netA = board.rules.nets.get("NET_A", 1).netNumber;
      Pin start = pinNearest(board, 20000);
      Pin dest = pinNearest(board, 100000);
      AutorouteControl ctrl = new AutorouteControl(board, netA, new RouterSettings(board));
      ctrl.ripupAllowed = true;
      ctrl.ripupCosts = seed;
      ctrl.ripupPassNo = 1;
      ctrl.viasAllowed = false;
      // SEED2 keeps fanout protection ON (removeUnconnectedVias=false);
      // SEED10/PROBE use the Java default (true, protection off) —
      // the protect-arm contrast within one fixture.
      ctrl.removeUnconnectedVias = seed != 2;
      ctrl.ripupPassNo = 1;
      emitCtrl(ctrl);
      emitRngProbe(seed);

      MazeSearchEngine search = openSearch(board, ctrl, netA, start, dest);
      MazeRipupResolver resolver = new MazeRipupResolver(search);
      java.lang.reflect.Method smallDoor =
          MazeRipupResolver.class.getDeclaredMethod(
              "enterThroughSmallDoor", MazeListElement.class, Item.class);
      smallDoor.setAccessible(true);

      java.util.Set<Integer> harvested = new java.util.TreeSet<>();
      java.util.Set<String> rippedHarvested = new java.util.TreeSet<>();
      boolean staleDone = false;
      boolean gateContrastDone = false;
      for (int k = 0; k < CAP; k++) {
        if (search.mazeExpansionList.isEmpty()) {
          break;
        }
        // Pop attribution row: engine-side rows (its own checkRipup
        // calls) land between two pop rows, so the Rust pins can align
        // the whole stream by replaying the same drain.
        MazeListElement head = search.mazeExpansionList.first();
        JsonObject pop = obj("pop");
        pop.addProperty("phase", phase);
        pop.addProperty("k", k);
        pop.addProperty("doorId", head.door.getId());
        pop.addProperty("section", head.sectionNoOfDoor);
        pop.addProperty("roomRipped", head.roomRipped);
        row(pop);
        // Delayed (roomRipped) elements: probe the :506 leaving path.
        MazeListElement rippedElem = null;
        for (MazeListElement e2 : search.mazeExpansionList) {
          if (e2.roomRipped) {
            rippedElem = e2;
            break;
          }
        }
        if (rippedElem != null
            && rippedHarvested.add(rippedElem.door.getId() + ":" + rippedElem.sectionNoOfDoor)) {
          JsonObject lv = obj("leaving_ripped");
          lv.addProperty("phase", phase);
          lv.addProperty("k", k);
          lv.addProperty("doorId", rippedElem.door.getId());
          lv.addProperty("section", rippedElem.sectionNoOfDoor);
          lv.addProperty("verdict", resolver.checkLeavingRippedItem(rippedElem));
          row(lv);
          // ignoreItem variants: the destination pin, plus the obstacle
          // item of the room being left (the natural ripup ignore).
          // null is NOT legal (sharesNet(null) NPEs at :260).
          Item fromObstacle = null;
          if (rippedElem.door instanceof ExpansionDoor rd) {
            if (rd.otherRoom(rippedElem.nextRoom)
                instanceof ObstacleExpansionRoom fromRoom2) {
              fromObstacle = fromRoom2.getItem();
            }
          }
          java.util.List<Item> variants = new ArrayList<>();
          variants.add(dest);
          if (fromObstacle != null) {
            variants.add(fromObstacle);
          }
          for (Item ignore : variants) {
            boolean v = (Boolean) smallDoor.invoke(resolver, rippedElem, ignore);
            JsonObject sd = obj("small_door_ripped");
            sd.addProperty("phase", phase);
            sd.addProperty("doorId", rippedElem.door.getId());
            sd.addProperty("section", rippedElem.sectionNoOfDoor);
            sd.addProperty("ignoreId", ignore.getId());
            sd.addProperty("verdict", v);
            row(sd);
          }
        }
        // Harvest scan over the WHOLE front (a ripup delay may re-add
        // elements; the drain itself mutates the set, so scan per k).
        MazeListElement found = null;
        ObstacleExpansionRoom foundRoom = null;
        for (MazeListElement e : search.mazeExpansionList) {
          if (e.nextRoom instanceof ObstacleExpansionRoom oer) {
            found = e;
            foundRoom = oer;
            break;
          }
        }
        if (found != null && !harvested.contains(foundRoom.getItem().getId())) {
          harvested.add(foundRoom.getItem().getId());
          JsonObject hv = obj("harvest");
          hv.addProperty("phase", phase);
          hv.addProperty("k", k);
          hv.addProperty("obstacle", foundRoom.getItem().getId());
          hv.addProperty("kind", foundRoom.getItem().getClass().getSimpleName());
          hv.addProperty("cornerNo", foundRoom.getIndexInItem());
          hv.addProperty("doorId", found.door.getId());
          hv.addProperty("section", found.sectionNoOfDoor);
          hv.addProperty("dimension", found.door.getDimension());
          hv.addProperty("alreadyChecked", found.alreadyChecked);
          hv.addProperty("roomRipped", found.roomRipped);
          row(hv);
          probeElement(
              phase, search, ctrl, board, resolver, found, foundRoom, dest, smallDoor);

          // The stale-index FALSE asymmetry + the true-gate contrast,
          // once per phase on the first PolylineTrace obstacle room.
          if (!staleDone && foundRoom.getItem() instanceof PolylineTrace pt) {
            staleDone = true;
            for (int staleIdx : new int[] {5, -1}) {
              ObstacleExpansionRoom stale =
                  new ObstacleExpansionRoom(pt, staleIdx, search.autorouteEngine.autorouteSearchTree);
              List<MazeTraceShover.DoorSection> ignored = new LinkedList<>();
              boolean v =
                  MazeTraceShover.checkShoveTraceLine(found, stale, board, ctrl, false, ignored);
              JsonObject sr = obj("shove_stale");
              sr.addProperty("phase", phase);
              sr.addProperty("staleIdx", staleIdx);
              sr.addProperty("verdict", v);
              row(sr);
            }
          }
          if (!gateContrastDone) {
            gateContrastDone = true;
            int layer = foundRoom.getLayer();
            int saved = ctrl.traceHalfWidth[layer];
            ctrl.traceHalfWidth[layer] = saved == 999 ? 998 : 999;
            List<MazeTraceShover.DoorSection> ignored = new LinkedList<>();
            boolean v =
                MazeTraceShover.checkShoveTraceLine(
                    found, foundRoom, board, ctrl, false, ignored);
            ctrl.traceHalfWidth[layer] = saved;
            JsonObject gc = obj("shove_halfwidth_gate");
            gc.addProperty("phase", phase);
            gc.addProperty("forcedHalfWidth", saved == 999 ? 998 : 999);
            gc.addProperty("obstacleHalfWidth",
                foundRoom.getItem() instanceof Trace tr ? tr.getHalfWidth() : -1);
            gc.addProperty("verdict", v);
            row(gc);
          }
        }
        search.occupyNextElement();
      }
      JsonObject fin = obj("phase_done");
      fin.addProperty("phase", phase);
      fin.addProperty("frontSize", search.mazeExpansionList.size());
      fin.addProperty("destination", destinationReached(search));
      row(fin);
    }
  }

  /** Linear item lookup (ids are stable per the item dump rows). */
  private static Item itemById(RoutingBoard board, int id) {
    for (Item item : board.getItems()) {
      if (item.getId() == id) {
        return item;
      }
    }
    return null;
  }

  /** Java `destinationDoor` is private — the "reached" witness via reflection. */
  private static boolean destinationReached(MazeSearchEngine search) throws Exception {
    java.lang.reflect.Field f =
        MazeSearchEngine.class.getDeclaredField("destinationDoor");
    f.setAccessible(true);
    return f.get(search) != null;
  }
}
