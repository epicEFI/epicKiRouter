// DrillSpike.java — the M3-T5 jar-side probe oracle for the drill
// subsystem port (DrillPageArray / DrillPage / ExpansionDrill +
// MazeExpansionEngine.expandToOtherLayers). ExpansionSpike house
// pattern: ONE JVM per run, deterministic JSONL rows on stdout (lines
// starting with "{" are the results; the jar's FRLogger noise is
// interleaved and dropped).
//
// Build/run (JDK 25, from the repo root):
//   mkdir -p /tmp/epic-drill-classes && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/javac \
//       -cp build/libs/freerouting-current-executable.jar \
//       -d /tmp/epic-drill-classes rust/harness/oracle/DrillSpike.java && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/java \
//       -Duser.language=en -Duser.country=US \
//       -cp build/libs/freerouting-current-executable.jar:/tmp/epic-drill-classes \
//       app.freerouting.autoroute.maze.DrillSpike \
//       rust/harness/fixtures/drill-spike/t5_drill_pages.dsn
//
// The oracle declares the app.freerouting.autoroute.maze package so the
// package-private MazeSearchEngine / MazeExpansionEngine constructors,
// the mazeExpansionList / destinationDistance fields, and the
// AutorouteControl.ViaMask nested class are directly reachable. The
// private DrillPageArray grid fields are read through reflection (the
// ExpansionSpike Field precedent); everything else is public API.
//
// Determinism: rows are emitted from ArrayList/LinkedList/TreeSet
// iteration orders only; every double is printed with Double.toString
// (exact round-trip); no HashSet/HashMap iteration reaches a row; the
// board mutation order (via insertions) is fixed.
package app.freerouting.autoroute.maze;

import app.freerouting.autoroute.drill.DrillPage;
import app.freerouting.autoroute.drill.DrillPageArray;
import app.freerouting.autoroute.drill.ExpansionDrill;
import app.freerouting.autoroute.expansion.CompleteFreeSpaceExpansionRoom;
import app.freerouting.autoroute.expansion.IncompleteFreeSpaceExpansionRoom;
import app.freerouting.autoroute.expansion.ObstacleExpansionRoom;
import app.freerouting.board.actions.ItemIdGenerator;
import app.freerouting.board.facade.BasicBoard;
import app.freerouting.board.facade.RoutingBoard;
import app.freerouting.board.model.items.Via;
import app.freerouting.board.model.structure.FixedState;
import app.freerouting.core.library.Padstack;
import app.freerouting.geometry.planar.FloatLine;
import app.freerouting.geometry.planar.FloatPoint;
import app.freerouting.geometry.planar.IntBox;
import app.freerouting.geometry.planar.IntPoint;
import app.freerouting.geometry.planar.TileShape;
import app.freerouting.io.BoardReadResult;
import app.freerouting.io.specctra.DsnReader;
import app.freerouting.rules.ViaInfo;
import app.freerouting.rules.ViaRule;
import app.freerouting.settings.RouterSettings;
import com.google.gson.Gson;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import java.io.ByteArrayInputStream;
import java.lang.reflect.Field;
import java.nio.file.Files;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.Collection;
import java.util.List;
import java.util.Set;

public class DrillSpike {

  private static final Gson GSON = new Gson();

  /** All full-drill dump pages: keepout page + the two pin pages. */
  private static final int[][] FULL_PAGES = {{5, 10}, {8, 5}, {8, 20}};

  private static void row(JsonObject obj) {
    System.out.println(GSON.toJson(obj));
  }

  private static JsonObject obj(String kind) {
    JsonObject o = new JsonObject();
    o.addProperty("type", kind);
    return o;
  }

  private static JsonArray intArr(int[] values) {
    JsonArray arr = new JsonArray(values.length);
    for (int v : values) {
      arr.add(v);
    }
    return arr;
  }

  private static void intArray(JsonObject o, String name, int[] values) {
    o.add(name, intArr(values));
  }

  private static Field field(Class<?> c, String name) throws Exception {
    Field f = c.getDeclaredField(name);
    f.setAccessible(true);
    return f;
  }

  private static int intField(Object owner, String name) throws Exception {
    return (Integer) field(owner.getClass(), name).get(owner);
  }

  /** One expandToOtherLayers run + the emitted list in TreeSet order. */
  private static void runLayerChange(
      String runId,
      MazeSearchEngine search,
      MazeExpansionEngine expansionEngine,
      ExpansionDrill drill,
      int sectionNoOfDoor,
      double expansionValue,
      FloatLine shapeEntry)
      throws Exception {
    search.mazeExpansionList.clear();
    MazeListElement element =
        new MazeListElement(
            drill,
            sectionNoOfDoor,
            null,
            0,
            expansionValue,
            expansionValue,
            null,
            shapeEntry,
            false,
            MazeSearchElement.Adjustment.NONE,
            false);
    expansionEngine.expandToOtherLayers(element);
    int k = 0;
    Set<MazeListElement> list = search.mazeExpansionList;
    for (MazeListElement e : list) {
      JsonObject o = obj("emit");
      o.addProperty("run", runId);
      o.addProperty("k", k);
      o.addProperty("section", e.sectionNoOfDoor);
      o.addProperty("expansionValue", Double.toString(e.expansionValue));
      o.addProperty("sortingValue", Double.toString(e.sortingValue));
      o.addProperty("roomRipped", e.roomRipped);
      o.addProperty("nextRoomId", e.nextRoom != null ? e.nextRoom.getId() : -1);
      o.addProperty("sectionOfBacktrack", e.sectionNoOfBacktrackDoor);
      o.addProperty("alreadyChecked", e.alreadyChecked);
      row(o);
      k++;
    }
    JsonObject summary = obj("runSummary");
    summary.addProperty("run", runId);
    summary.addProperty("emitted", k);
    row(summary);
  }

  /** Drill candidate rows for one page: full dump or count only. */
  private static void dumpPage(
      int j,
      int i,
      DrillPage page,
      boolean attach,
      AutorouteEngine engine,
      boolean full) {
    Collection<ExpansionDrill> drills = page.getDrills(engine, attach);
    JsonObject o = obj("page");
    o.addProperty("j", j);
    o.addProperty("i", i);
    o.addProperty("attach", attach);
    o.addProperty("drillCount", drills.size());
    if (!full) {
      row(o);
      return;
    }
    int d = 0;
    for (ExpansionDrill drill : drills) {
      JsonObject r = obj("drill");
      r.addProperty("j", j);
      r.addProperty("i", i);
      r.addProperty("attach", attach);
      r.addProperty("d", d);
      IntPoint loc = (IntPoint) drill.location;
      r.addProperty("locX", loc.x);
      r.addProperty("locY", loc.y);
      r.addProperty("firstLayer", drill.firstLayer);
      r.addProperty("lastLayer", drill.lastLayer);
      StringBuilder rooms = new StringBuilder("[");
      for (int s = 0; s < drill.roomArr.length; s++) {
        if (s > 0) {
          rooms.append(',');
        }
        rooms.append(drill.roomArr[s] != null ? drill.roomArr[s].getId() : -1);
      }
      rooms.append(']');
      r.addProperty("rooms", rooms.toString());
      row(r);
      d++;
    }
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

    // ---- meta ---------------------------------------------------------
    JsonObject meta = obj("meta");
    IntBox bb = board.boundingBox;
    intArray(meta, "bounds", new int[] {bb.ll.x, bb.ll.y, bb.ur.x, bb.ur.y});
    meta.addProperty("layerCount", board.getLayerCount());
    row(meta);

    // ---- parsed items (id cross-check for the Rust replay) ------------
    for (app.freerouting.board.model.items.Item item :
        board.getItems()) {
      JsonObject it = obj("item");
      it.addProperty("id", item.getId());
      it.addProperty("kind", item.getClass().getSimpleName());
      it.addProperty("netCount", item.netCount());
      row(it);
    }

    // ---- the via rule -------------------------------------------------
    ViaRule viaRule = board.rules.getDefaultViaRule();
    JsonObject ruleRow = obj("viaRule");
    ruleRow.addProperty("name", viaRule.name);
    ruleRow.addProperty("viaCount", viaRule.viaCount());
    for (int i = 0; i < viaRule.viaCount(); i++) {
      ViaInfo via = viaRule.getVia(i);
      Padstack padstack = via.getPadstack();
      JsonObject v = new JsonObject();
      v.addProperty("name", via.getName());
      v.addProperty("padstack", padstack.name);
      intArray(v, "span", new int[] {padstack.fromLayer(), padstack.toLayer()});
      v.addProperty("clearanceClass", via.getClearanceClassIndex());
      v.addProperty("attachSmdAllowed", via.attachSmdAllowed());
      ruleRow.add("via" + i, v);
    }
    row(ruleRow);

    // ---- nets + engines ----------------------------------------------
    int netA = board.rules.nets.get("NET_A", 1).netNumber;
    int netB = board.rules.nets.get("NET_B", 1).netNumber;
    JsonObject netsRow = obj("nets");
    netsRow.addProperty("netA", netA);
    netsRow.addProperty("netB", netB);
    row(netsRow);

    // The engine's autoroute tree is compensated for the net's trace
    // clearance class (production config): class 0 builds an
    // UNCOMPENSATED tree whose raw query branch cannot handle rooms.
    AutorouteControl ctrlA = new AutorouteControl(board, netA, new RouterSettings());
    AutorouteEngine engineA =
        new AutorouteEngine(board, ctrlA.viaClearanceClass, true);
    engineA.initConnection(netA, null, null);
    // Production state mirror: by the time drills enumerate, earlier
    // pipeline phases have populated incompleteExpansionRooms (the
    // completion path removes the passed room from it; with a fresh
    // null list that remove NPEs and the catch rejects every drill).
    field(AutorouteEngine.class, "incompleteExpansionRooms")
        .set(engineA, new ArrayList<>());

    // ---- page grid ----------------------------------------------------
    double defaultViaDiameter = board.rules.getDefaultViaDiameter();
    int maxPageWidth = (int) (5 * defaultViaDiameter);
    maxPageWidth = Math.max(maxPageWidth, 10000);
    DrillPageArray arrayA = engineA.drillPageArray;
    JsonObject pm = obj("pageMeta");
    pm.addProperty("defaultViaDiameter", Double.toString(defaultViaDiameter));
    pm.addProperty("maxPageWidth", maxPageWidth);
    pm.addProperty("columnCount", intField(arrayA, "columnCount"));
    pm.addProperty("rowCount", intField(arrayA, "rowCount"));
    pm.addProperty("pageWidth", intField(arrayA, "pageWidth"));
    pm.addProperty("pageHeight", intField(arrayA, "pageHeight"));
    row(pm);

    // ---- candidate enumeration (attach=false, then attach=true on a
    // FRESH array: the memo key excludes attachSmd) ---------------------
    DrillPage[][] pagesA = (DrillPage[][]) field(DrillPageArray.class, "pages").get(arrayA);
    for (int j = 0; j < intField(arrayA, "rowCount"); j++) {
      for (int i = 0; i < intField(arrayA, "columnCount"); i++) {
        boolean full = false;
        for (int[] fp : FULL_PAGES) {
          if (fp[0] == j && fp[1] == i) {
            full = true;
            break;
          }
        }
        IntBox ps = pagesA[j][i].shape;
        dumpPage(j, i, pagesA[j][i], false, engineA, full);
        if (full) {
          JsonObject pb = obj("pageBounds");
          pb.addProperty("j", j);
          pb.addProperty("i", i);
          intArray(pb, "box", new int[] {ps.ll.x, ps.ll.y, ps.ur.x, ps.ur.y});
          row(pb);
        }
      }
    }
    DrillPageArray arrayAttach = new DrillPageArray(board, maxPageWidth);
    DrillPage[][] pagesAttach = (DrillPage[][]) field(DrillPageArray.class, "pages").get(arrayAttach);
    for (int[] fp : FULL_PAGES) {
      dumpPage(fp[0], fp[1], pagesAttach[fp[0]][fp[1]], true, engineA, true);
    }

    // ---- overlappingPages probes --------------------------------------
    int cols = intField(arrayA, "columnCount");
    int rows = intField(arrayA, "rowCount");
    int pw = intField(arrayA, "pageWidth");
    int ph = intField(arrayA, "pageHeight");
    int[][] probes = {
      {bb.ll.x, bb.ll.y, bb.ur.x, bb.ur.y},
      {pw, ph, 2 * pw, 2 * ph},
      {100000, 100000, 104000, 100000},
    };
    for (int[] probe : probes) {
      TileShape shape =
          new IntBox(new IntPoint(probe[0], probe[1]), new IntPoint(probe[2], probe[3]));
      Collection<DrillPage> overlaps = arrayA.overlappingPages(shape);
      List<int[]> coords = new ArrayList<>();
      for (int j = 0; j < rows; j++) {
        for (int i = 0; i < cols; i++) {
          if (overlaps.contains(pagesA[j][i])) {
            coords.add(new int[] {j, i});
          }
        }
      }
      JsonObject o = obj("overlapPages");
      intArray(o, "probe", probe);
      JsonArray arr = new JsonArray(coords.size());
      for (int[] c : coords) {
        JsonArray pair = new JsonArray(2);
        pair.add(c[0]);
        pair.add(c[1]);
        arr.add(pair);
      }
      o.add("pages", arr);
      row(o);
    }

    // ---- ctrl capture (NET_A) -----------------------------------------
    JsonObject ctrlRow = obj("ctrl");
    ctrlRow.addProperty("net", "NET_A");
    ctrlRow.addProperty("netNumber", ctrlA.netNumber);
    ctrlRow.addProperty("viaRuleName", ctrlA.viaRule.name);
    ctrlRow.addProperty("viaClearanceClass", ctrlA.viaClearanceClass);
    ctrlRow.addProperty("attachSmdAllowed", ctrlA.attachSmdAllowed);
    ctrlRow.addProperty("ripupAllowed", ctrlA.ripupAllowed);
    ctrlRow.addProperty("viasAllowed", ctrlA.viasAllowed);
    ctrlRow.addProperty("viaLowerBound", ctrlA.viaLowerBound);
    ctrlRow.addProperty("viaUpperBound", ctrlA.viaUpperBound);
    JsonArray costs = new JsonArray(ctrlA.layerCount);
    for (int f = 0; f < ctrlA.layerCount; f++) {
      costs.add(intArr(ctrlA.addViaCosts[f].toLayer));
    }
    ctrlRow.add("addViaCosts", costs);
    JsonArray masks = new JsonArray(ctrlA.viaInfos.length);
    for (AutorouteControl.ViaMask m : ctrlA.viaInfos) {
      JsonObject mv = new JsonObject();
      mv.addProperty("fromLayer", m.fromLayer);
      mv.addProperty("toLayer", m.toLayer);
      mv.addProperty("attachSmdAllowed", m.attachSmdAllowed);
      masks.add(mv);
    }
    ctrlRow.add("viaInfos", masks);
    row(ctrlRow);

    // ---- layer-change runs (NET_A) ------------------------------------
    MazeSearchEngine searchA = new MazeSearchEngine(engineA, ctrlA);
    MazeExpansionEngine mzeA = new MazeExpansionEngine(searchA);
    ExpansionDrill pinDrill = null;
    for (ExpansionDrill drill : pagesAttach[8][5].getDrills(engineA, true)) {
      if (pinDrill == null
          || drill.location.equals(new IntPoint(200000, 330000))) {
        pinDrill = drill;
      }
    }
    if (pinDrill == null) {
      JsonObject miss = obj("pin-drill-miss");
      row(miss);
    } else {
      JsonObject idRow = obj("drillId");
      idRow.addProperty("id", pinDrill.getId());
      IntPoint loc = (IntPoint) pinDrill.location;
      idRow.addProperty("locX", loc.x);
      idRow.addProperty("locY", loc.y);
      row(idRow);
      FloatLine entry =
          new FloatLine(new FloatPoint(190000, 295000), new FloatPoint(210000, 305000));
      runLayerChange("A-full", searchA, mzeA, pinDrill, 0, 100.0, entry);
      AutorouteControl.ViaMask[] saved = ctrlA.viaInfos;
      ctrlA.viaInfos = new AutorouteControl.ViaMask[] {new AutorouteControl.ViaMask(0, 3, false)};
      ctrlA.attachSmdAllowed = false;
      runLayerChange("A-mask3-noattach", searchA, mzeA, pinDrill, 0, 100.0, entry);
      ctrlA.viaInfos = new AutorouteControl.ViaMask[] {new AutorouteControl.ViaMask(0, 1, false)};
      runLayerChange("A-mask01-noattach", searchA, mzeA, pinDrill, 0, 100.0, entry);
      ctrlA.viaInfos = new AutorouteControl.ViaMask[] {new AutorouteControl.ViaMask(0, 1, true)};
      runLayerChange("A-mask01-attach", searchA, mzeA, pinDrill, 0, 100.0, entry);
      ctrlA.viaInfos = saved;
      ctrlA.attachSmdAllowed = true;
      // destinationDistance probes (the T8 seam's real heuristic).
      double[][] dprobes = {
        {200000, 300000, 0},
        {200000, 300000, 1},
        {200000, 300000, 3},
        {500000, 300000, 1},
      };
      for (double[] dp : dprobes) {
        JsonObject d = obj("dist");
        d.addProperty("x", dp[0]);
        d.addProperty("y", dp[1]);
        d.addProperty("layer", (int) dp[2]);
        d.addProperty(
            "value",
            Double.toString(
                searchA.destinationDistance.calculate(
                    new FloatPoint(dp[0], dp[1]), (int) dp[2])));
        row(d);
      }
    }

    // Ripped-via positive control on the A side: the same battery as
    // NET_B's, on the net whose free-space runs demonstrably emit.
    Via viaA =
        board.insertVia(
            viaRule.getVia(0).getPadstack(),
            new IntPoint(300000, 150000),
            new int[] {netA},
            ctrlA.viaClearanceClass,
            FixedState.UNFIXED,
            false);
    ExpansionDrill ripDrillA = buildRipDrill(engineA, viaA, 300000, 150000, true);
    JsonObject viaRowA = obj("insertedVia");
    viaRowA.addProperty("which", "A");
    viaRowA.addProperty("id", viaA.getId());
    row(viaRowA);
    FloatLine entryA2 =
        new FloatLine(new FloatPoint(290000, 145000), new FloatPoint(310000, 155000));
    ctrlA.ripupAllowed = true;
    runLayerChange("A-rip-positive", searchA, mzeA, ripDrillA, 0, 100.0, entryA2);
    ctrlA.ripupAllowed = false;

    // ---- ripped-via battery (NET_B) -----------------------------------
    AutorouteControl ctrlB = new AutorouteControl(board, netB, new RouterSettings());
    AutorouteEngine engineB =
        new AutorouteEngine(board, ctrlB.viaClearanceClass, true);
    engineB.initConnection(netB, null, null);
    field(AutorouteEngine.class, "incompleteExpansionRooms")
        .set(engineB, new ArrayList<>());
    MazeSearchEngine searchB = new MazeSearchEngine(engineB, ctrlB);
    MazeExpansionEngine mzeB = new MazeExpansionEngine(searchB);
    Padstack padstackT = viaRule.getVia(0).getPadstack();
    Padstack padstackX = board.library.padstacks.get("VIA_X");
    JsonObject psRow = obj("padstacks");
    psRow.addProperty("viaT", padstackT != null ? padstackT.name : "null");
    psRow.addProperty("viaX", padstackX != null ? padstackX.name : "null");
    row(psRow);
    // Free-space rooms come from the shared autoroute tree (the page
    // enumeration already tiled the board); buildRipDrill discovers
    // them with the production calculateExpansionRooms call.
    int viaClass = ctrlB.viaClearanceClass;
    Via viaB =
        board.insertVia(
            padstackT,
            new IntPoint(500000, 450000),
            new int[] {netB},
            viaClass,
            FixedState.UNFIXED,
            false);
    JsonObject viaRowB = obj("insertedVia");
    viaRowB.addProperty("which", "B");
    viaRowB.addProperty("id", viaB.getId());
    row(viaRowB);
    // Positive control: a drill with NO ripped room over an EMPTY
    // spot (viaB itself sits at (500000,450000) and would block the
    // free-branch checkLayer probes) — the free-space branch must
    // emit {1,2,3}.
    ExpansionDrill freeDrill = buildRipDrill(engineB, viaB, 620000, 450000, false);
    ExpansionDrill ripDrill = buildRipDrill(engineB, viaB, 500000, 450000, true);
    FloatLine entryB =
        new FloatLine(new FloatPoint(490000, 445000), new FloatPoint(510000, 455000));
    ctrlB.ripupAllowed = true;
    JsonObject diag = obj("ripDiag");
    diag.addProperty("ripupAllowed", ctrlB.ripupAllowed);
    diag.addProperty("isFanout", ctrlB.isFanout);
    diag.addProperty("viaRuleName", ctrlB.viaRule.name);
    diag.addProperty("viaNonNull", viaB != null);
    diag.addProperty(
        "padstackIdentical", viaB != null && viaB.getPadstack() == padstackT);
    diag.addProperty(
        "viaClass", viaB != null ? viaB.clearanceClassIndex() : -1);
    diag.addProperty("ctrlClass", ctrlB.viaClearanceClass);
    diag.addProperty("ctrlNet", ctrlB.netNumber);
    diag.addProperty(
        "room0",
        ripDrill.roomArr[0] != null
            ? ripDrill.roomArr[0].getClass().getSimpleName()
            : "null");
    row(diag);
    runLayerChange("B-free-control", searchB, mzeB, freeDrill, 0, 100.0, entryB);
    runLayerChange("B-rip-positive", searchB, mzeB, ripDrill, 0, 100.0, entryB);
    ctrlB.ripupAllowed = false;
    runLayerChange("B-rip-ripup-off", searchB, mzeB, ripDrill, 0, 100.0, entryB);
    ctrlB.ripupAllowed = true;
    // Wrong clearance class: the via class must match ctrl.viaClearanceClass.
    Via viaB2 =
        board.insertVia(
            padstackT,
            new IntPoint(520000, 450000),
            new int[] {netB},
            0,
            FixedState.UNFIXED,
            false);
    ExpansionDrill ripDrill2 = buildRipDrill(engineB, viaB2, 520000, 450000, true);
    JsonObject viaRowB2 = obj("insertedVia");
    viaRowB2.addProperty("which", "B2");
    viaRowB2.addProperty("id", viaB2.getId());
    row(viaRowB2);
    runLayerChange("B-rip-wrong-class", searchB, mzeB, ripDrill2, 0, 100.0, entryB);
    // Foreign padstack: not in the via rule.
    Via viaB3 =
        board.insertVia(
            padstackX,
            new IntPoint(540000, 450000),
            new int[] {netB},
            viaClass,
            FixedState.UNFIXED,
            false);
    ExpansionDrill ripDrill3 = buildRipDrill(engineB, viaB3, 540000, 450000, true);
    JsonObject viaRowB3 = obj("insertedVia");
    viaRowB3.addProperty("which", "B3");
    viaRowB3.addProperty("id", viaB3.getId());
    row(viaRowB3);
    runLayerChange("B-rip-foreign-padstack", searchB, mzeB, ripDrill3, 0, 100.0, entryB);

    row(obj("done"));
  }

  /**
   * A drill over a via location: the room discovery is the production
   * call (calculateExpansionRooms finds the existing free-space tree
   * rooms on every layer); with rip=true slot 0 is then overwritten
   * with the obstacle room over the via — the state a ripped via
   * leaves in the maze; with rip=false the drill stays fully free
   * (the positive control for the free-space branch). A failed
   * discovery (no room on some layer) is documented and the drill
   * keeps its null slot.
   */
  private static ExpansionDrill buildRipDrill(
      AutorouteEngine engine, Via via, int x, int y, boolean rip) {
    IntPoint center = new IntPoint(x, y);
    ExpansionDrill drill =
        new ExpansionDrill(
            new IntBox(new IntPoint(x - 4000, y - 4000), new IntPoint(x + 4000, y + 4000)),
            center,
            0,
            engine.board.getLayerCount() - 1);
    boolean discovered = drill.calculateExpansionRooms(engine);
    JsonObject o = obj("ripDiscovery");
    o.addProperty("x", x);
    o.addProperty("y", y);
    o.addProperty("rip", rip);
    o.addProperty("viaNonNull", via != null);
    o.addProperty("discovered", discovered);
    StringBuilder rooms = new StringBuilder("[");
    for (int s = 0; s < drill.roomArr.length; s++) {
      if (s > 0) {
        rooms.append(',');
      }
      rooms.append(drill.roomArr[s] != null ? drill.roomArr[s].getId() : -1);
    }
    rooms.append(']');
    o.addProperty("rooms", rooms.toString());
    row(o);
    if (rip) {
      drill.roomArr[0] = new ObstacleExpansionRoom(via, 0, engine.autorouteSearchTree);
    }
    return drill;
  }
}
