// LocatorSpike.java — the M3-T9 jar-side probe oracle for the
// FoundConnectionLocator port (backtrack walk + 45-degree / any-angle
// corner synthesis). MazeSpike house pattern: ONE JVM per run,
// deterministic JSONL rows on stdout (lines starting with "{" are the
// spike results; the jar's FRLogger noise is interleaved and dropped,
// EXCEPT the net-gated locator hooks — compare_trace_* / BACKTRACK_* —
// which ARE the literal capture format and are extracted by grep).
//
// Build/run (JDK 25, from the repo root):
//   mkdir -p /tmp/epic-t9-classes && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/javac \
//       -cp build/libs/freerouting-current-executable.jar \
//       -d /tmp/epic-t9-classes rust/harness/oracle/LocatorSpike.java && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/java \
//       -Duser.language=en -Duser.country=US \
//       -Dfreerouting.logging.console.level=TRACE \
//       -cp build/libs/freerouting-current-executable.jar:/tmp/epic-t9-classes \
//       app.freerouting.autoroute.path.LocatorSpike \
//       rust/harness/fixtures/locator-spike/t9_locator45.dsn
//
// The oracle declares the app.freerouting.autoroute.path package so the
// protected locator fields (backtrackArray) and the inner types are
// directly reachable. Reached reflectively: MazeSearchEngine.Result's
// package-private ctor, the engine's private destinationDoor /
// sectionNoOfDestinationDoor fields, and the engine's private
// incompleteExpansionRooms field (the MazeSpike precedents).
//
// Capture protocol: per phase — fresh board parse, AutorouteControl
// with the phase flags, AutorouteEngine + initConnection + the
// incompleteExpansionRooms reset, MazeSearchEngine.getInstance, full
// drain (`while occupyNextElement()`), then
// FoundConnectionLocator.getInstance (which emits the hook rows during
// its ctor AND fills rippedItemList/ripupCosts), then the spike's own
// JSON rows (locator meta, FULL per-trace corner lists, ripped ids,
// ripup costs sorted by item id).
//
// Determinism: rows are emitted from TreeSet iteration or sorted-by-id
// walks only; every double is printed with Double.toString; no
// HashMap iteration reaches a row (ripupCosts is a HashMap in Java —
// the spike copies it into an id-sorted array before printing).
package app.freerouting.autoroute.path;

import app.freerouting.autoroute.expansion.ExpandableObject;
import app.freerouting.autoroute.maze.AutorouteControl;
import app.freerouting.autoroute.maze.AutorouteEngine;
import app.freerouting.autoroute.maze.MazeSearchEngine;
import app.freerouting.board.facade.BasicBoard;
import app.freerouting.board.facade.RoutingBoard;
import app.freerouting.board.model.items.Item;
import app.freerouting.board.actions.ItemIdGenerator;
import app.freerouting.board.model.items.Pin;
import app.freerouting.board.model.structure.AngleRestriction;
import app.freerouting.geometry.planar.IntPoint;
import app.freerouting.io.BoardReadResult;
import app.freerouting.io.specctra.DsnReader;
import app.freerouting.settings.RouterSettings;
import com.google.gson.Gson;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import java.io.ByteArrayInputStream;
import java.lang.reflect.Constructor;
import java.nio.file.Files;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeMap;
import java.util.TreeSet;

public class LocatorSpike {

  private static final Gson GSON = new Gson();

  private static final java.lang.reflect.Field DEST_DOOR_FIELD;
  private static final java.lang.reflect.Field DEST_SECTION_FIELD;
  private static final java.lang.reflect.Field INCOMPLETE_FIELD;
  private static final java.lang.reflect.Field FRONT_FIELD;
  private static final java.lang.reflect.Field RIPPED_FIELD;
  private static final Constructor<MazeSearchEngine.Result> RESULT_CTOR;

  static {
    try {
      DEST_DOOR_FIELD = MazeSearchEngine.class.getDeclaredField("destinationDoor");
      DEST_DOOR_FIELD.setAccessible(true);
      DEST_SECTION_FIELD =
          MazeSearchEngine.class.getDeclaredField("sectionNoOfDestinationDoor");
      DEST_SECTION_FIELD.setAccessible(true);
      INCOMPLETE_FIELD =
          AutorouteEngine.class.getDeclaredField("incompleteExpansionRooms");
      INCOMPLETE_FIELD.setAccessible(true);
      FRONT_FIELD = MazeSearchEngine.class.getDeclaredField("mazeExpansionList");
      FRONT_FIELD.setAccessible(true);
      RIPPED_FIELD =
          app.freerouting.autoroute.maze.MazeListElement.class.getDeclaredField("roomRipped");
      RIPPED_FIELD.setAccessible(true);
      RESULT_CTOR =
          MazeSearchEngine.Result.class.getDeclaredConstructor(ExpandableObject.class, int.class);
      RESULT_CTOR.setAccessible(true);
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

  private static final class Phase {
    final String name;
    final String netName;
    final boolean vias;
    final boolean ripup;
    final boolean fanout;
    final boolean ninety;
    final int ripupCosts;

    Phase(String name, String netName, boolean vias, boolean ripup, boolean fanout,
        boolean ninety) {
      this(name, netName, vias, ripup, fanout, ninety, -1);
    }

    Phase(String name, String netName, boolean vias, boolean ripup, boolean fanout,
        boolean ninety, int ripupCosts) {
      this.name = name;
      this.netName = netName;
      this.vias = vias;
      this.ripup = ripup;
      this.fanout = fanout;
      this.ninety = ninety;
      this.ripupCosts = ripupCosts;
    }
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

  private static Pin pinById(RoutingBoard board, int id) {
    for (Item item : board.getItems()) {
      if (item instanceof Pin pin && pin.getId() == id) {
        return pin;
      }
    }
    return null;
  }

  private static void runPhase(byte[] bytes, String fileName, Phase ph) throws Exception {
    RoutingBoard board = parse(bytes, fileName);
    if (ph.ninety) {
      board.rules.setTraceAngleRestriction(AngleRestriction.NINETY_DEGREE);
    }
    if (board.rules.nets.get(ph.netName, 1) == null) {
      JsonObject skip = obj("phase-skip");
      skip.addProperty("phase", ph.name);
      skip.addProperty("reason", "net " + ph.netName + " not found");
      row(skip);
      return;
    }
    int net = board.rules.nets.get(ph.netName, 1).netNumber;

    // start pin = lower id, destination = higher id (deterministic).
    Pin startPin = null;
    Pin destPin = null;
    for (Item item : board.getItems()) {
      if (!(item instanceof Pin pin) || !item.containsNet(net)) {
        continue;
      }
      if (startPin == null || pin.getId() < startPin.getId()) {
        destPin = startPin;
        startPin = pin;
      } else {
        destPin = pin;
      }
    }
    if (startPin == null || (!ph.fanout && destPin == null)) {
      JsonObject skip = obj("phase-skip");
      skip.addProperty("phase", ph.name);
      skip.addProperty("reason", "pins missing");
      row(skip);
      return;
    }

    AutorouteControl ctrl = new AutorouteControl(board, net, new RouterSettings(board));
    ctrl.viasAllowed = ph.vias;
    ctrl.ripupAllowed = ph.ripup;
    ctrl.isFanout = ph.fanout;
    if (ph.ripupCosts >= 0) {
      ctrl.ripupCosts = ph.ripupCosts;
    }

    AutorouteEngine engine = new AutorouteEngine(board, ctrl.viaClearanceClass, true);
    engine.initConnection(net, null, null);
    INCOMPLETE_FIELD.set(engine, new ArrayList<>());

    Set<Item> destSet = ph.fanout ? Set.of() : Set.of(destPin);
    MazeSearchEngine search =
        MazeSearchEngine.getInstance(Set.of(startPin), destSet, engine, ctrl);
    JsonObject open = obj("searchOpen");
    open.addProperty("phase", ph.name);
    open.addProperty("net", net);
    open.addProperty("startId", startPin.getId());
    open.addProperty("destId", ph.fanout ? -1 : destPin.getId());
    open.addProperty("vias", ph.vias);
    open.addProperty("ripup", ph.ripup);
    open.addProperty("fanout", ph.fanout);
    open.addProperty(
        "angleRestriction", board.rules.getTraceAngleRestriction().toString());
    open.addProperty("ok", search != null);
    row(open);
    if (search == null) {
      JsonObject nl = obj("searchNull");
      nl.addProperty("phase", ph.name);
      row(nl);
      return;
    }
    long popCount = 0;
    while (search.occupyNextElement()) {
      popCount++;
    }
    @SuppressWarnings("unchecked")
    java.util.SortedSet<app.freerouting.autoroute.maze.MazeListElement> front =
        (java.util.SortedSet<app.freerouting.autoroute.maze.MazeListElement>)
            FRONT_FIELD.get(search);
    long rippedQueued = 0;
    for (app.freerouting.autoroute.maze.MazeListElement e : front) {
      if (RIPPED_FIELD.getBoolean(e)) {
        rippedQueued++;
      }
    }

    ExpandableObject destDoor = (ExpandableObject) DEST_DOOR_FIELD.get(search);
    int destSection = DEST_SECTION_FIELD.getInt(search);
    JsonObject mr = obj("mazeResult");
    mr.addProperty("phase", ph.name);
    mr.addProperty("found", destDoor != null);
    mr.addProperty("popCount", popCount);
    mr.addProperty("rippedQueued", rippedQueued);
    mr.addProperty("frontRest", front.size());
    if (destDoor != null) {
      mr.addProperty("destinationType", destDoor.getClass().getSimpleName());
      mr.addProperty("section", destSection);
    }
    row(mr);
    if (destDoor == null) {
      return;
    }

    MazeSearchEngine.Result result =
        RESULT_CTOR.newInstance(destDoor, destSection);
    TreeSet<Item> rippedItemList = new TreeSet<>();
    Map<Item, Integer> ripupCosts = new HashMap<>();

    // The hook rows (compare_trace_* / BACKTRACK_*) print inside this
    // call, interleaved BEFORE the spike rows below.
    FoundConnectionLocator locator =
        FoundConnectionLocator.getInstance(
            result,
            ctrl,
            engine.autorouteSearchTree,
            board.rules.getTraceAngleRestriction(),
            rippedItemList,
            ripupCosts);
    if (locator == null) {
      JsonObject nl = obj("locatorNull");
      nl.addProperty("phase", ph.name);
      row(nl);
      return;
    }

    JsonObject lm = obj("locator");
    lm.addProperty("phase", ph.name);
    lm.addProperty("class", locator.getClass().getSimpleName());
    lm.addProperty(
        "startItem", locator.startItem != null ? locator.startItem.getId() : -1);
    lm.addProperty("startLayer", locator.startLayer);
    lm.addProperty(
        "targetItem", locator.targetItem != null ? locator.targetItem.getId() : -1);
    lm.addProperty("targetLayer", locator.targetLayer);
    lm.addProperty("backtrackSize", locator.backtrackArray.length);
    lm.addProperty("traceCount", locator.connectionItems.size());
    row(lm);

    int ti = 0;
    for (FoundConnectionLocator.ResultItem item : locator.connectionItems) {
      JsonObject t = obj("trace");
      t.addProperty("phase", ph.name);
      t.addProperty("i", ti++);
      t.addProperty("layer", item.layer);
      t.addProperty("n", item.corners.length);
      JsonArray arr = new JsonArray(item.corners.length * 2);
      for (IntPoint c : item.corners) {
        arr.add(c.x);
        arr.add(c.y);
      }
      t.add("corners", arr);
      row(t);
    }

    JsonObject rp = obj("ripped");
    rp.addProperty("phase", ph.name);
    rp.addProperty("count", rippedItemList.size());
    // TreeSet<Item> natural order = Item.compareTo = other.id - id:
    // DESCENDING id iteration (Java truth; the T9 brief's "ascending"
    // was wrong — Java wins).
    JsonArray ids = new JsonArray(rippedItemList.size());
    for (Item item : rippedItemList) {
      ids.add(item.getId());
    }
    rp.add("ids", ids);
    row(rp);

    JsonObject co = obj("costs");
    co.addProperty("phase", ph.name);
    co.addProperty("count", ripupCosts.size());
    // ripupCosts is a HashMap — copy into an id-sorted structure before
    // printing (no HashMap iteration order may reach a row).
    TreeMap<Integer, Integer> byId = new TreeMap<>();
    for (Map.Entry<Item, Integer> entry : ripupCosts.entrySet()) {
      byId.put(entry.getKey().getId(), entry.getValue());
    }
    JsonArray pairs = new JsonArray(byId.size());
    for (Map.Entry<Integer, Integer> entry : byId.entrySet()) {
      pairs.add(entry.getKey() + ":" + entry.getValue());
    }
    co.add("pairs", pairs);
    row(co);
  }

  public static void main(String[] p_args) throws Exception {
    if (p_args.length < 1) {
      row(obj("usage-error"));
      System.exit(1);
    }
    String path = p_args[0];
    byte[] bytes;
    try {
      bytes = Files.readAllBytes(Paths.get(path));
    } catch (Exception e) {
      row(obj("read-error"));
      System.exit(2);
      return;
    }
    String fileName = Paths.get(path).getFileName().toString();
    row(obj("begin"));

    List<Phase> phases = new ArrayList<>();
    if (fileName.contains("45")) {
      phases.add(new Phase("A45", "NET_33", false, false, false, false));
      phases.add(new Phase("A45V", "NET_33", true, false, false, false));
      phases.add(new Phase("A90", "NET_33", false, false, false, true));
      phases.add(new Phase("AFAN", "NET_33", true, false, true, false));
      phases.add(new Phase("ABT", "NET_98", false, false, false, false));
    } else if (fileName.contains("any")) {
      phases.add(new Phase("BANY", "NET_33", false, false, false, false));
      phases.add(new Phase("BANYV", "NET_33", true, false, false, false));
      phases.add(new Phase("BBT", "NET_98", true, false, false, false));
    } else {
      // CBT: vias allowed — the maze detours to B.Cu and rips nothing
      // (pins the empty-ripped-list arm). CBTR: vias forbidden — the
      // F.Cu gap corridor cannot clear the pre-routed NET_B wiring (the
      // free slivers above/below it are under 2·halfwidth + clearance
      // tall), so the found connection MUST cross ripped obstacles:
      // roomRipped=true rows + the obstacle-room cost harvest (:256-265,
      // :315-323) fire.
      phases.add(new Phase("CBT", "NET_A", true, true, false, false));
      phases.add(new Phase("CBTR", "NET_A", false, true, false, false));
      phases.add(new Phase("CBTR2", "NET_A", false, true, false, false, 2));
      phases.add(new Phase("CBTR10", "NET_A", false, true, false, false, 10));
    }
    for (Phase ph : phases) {
      try {
        runPhase(bytes, fileName, ph);
      } catch (Exception e) {
        JsonObject err = obj("phase-error");
        err.addProperty("phase", ph.name);
        err.addProperty("message", String.valueOf(e));
        row(err);
      }
    }
    row(obj("done"));
  }
}
