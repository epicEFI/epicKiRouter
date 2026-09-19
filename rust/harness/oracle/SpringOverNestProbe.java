// SpringOverNestProbe.java — the M3-T10b FIX-ROUND capture probe for
// the springOver containment direction (spec review MAJOR-1). ONE JVM
// per run, deterministic JSONL rows on stdout; consumer drops the jar
// noise (lines not starting with "{").
//
// Build/run (JDK 25, from the repo root):
//   mkdir -p /tmp/epic-t10b-fix-classes && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/javac \
//       -cp build/libs/freerouting-current-executable.jar \
//       -d /tmp/epic-t10b-fix-classes rust/harness/oracle/SpringOverNestProbe.java && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/java \
//       -cp build/libs/freerouting-current-executable.jar:/tmp/epic-t10b-fix-classes \
//       app.freerouting.board.optimize.SpringOverNestProbe \
//       rust/harness/fixtures/locator-spike/t9_locator45.dsn
//
// World (fresh parse, items inserted through the real board API):
//   - fixture PIN id 101 (net 33, bbox [180000,280000,220000,320000]):
//     an obstacle through the contactPins branch (the probe passes an
//     EMPTY contactPins set, so every pin not in it counts);
//   - a USER_FIXED through-all via (padstack +/-5000) at the pin's
//     center (200000,300000): an obstacle through
//     `!isRoutable()` (Via.isRoutable = !isUserFixed && netCount > 0,
//     Via.java:147-149 — a SHOVE_FIXED via is still routable and can
//     NEVER be the second obstacle; the pair is Java's motivating
//     "fixed vias inside of pins", TraceShover.java:673-674).
// The via bbox [195000,295000,205000,305000] nests strictly inside the
// pin bbox. The springOver scan must converge on the OUTER box (the
// pin) from EITHER encounter order (Java arm 1: current contains found
// -> replace); the argument-swapped form keeps the INNERMOST box (wrap
// around the via).
//
// The probe then drives the PUBLIC TraceShover.springOverObstacles
// (:827) with a horizontal 2-corner polyline through both obstacles
// (net N001, halfWidth 50, empty contactPins) and emits the returned
// polyline's corners + bounding box (or null), plus the shoving state.
//
// Determinism: ids are emission-ordered by the single-threaded insert
// sequence; corners are Double.toString; no HashSet/HashMap iteration
// reaches a row.
package app.freerouting.board.optimize;

import app.freerouting.board.actions.ItemIdGenerator;
import app.freerouting.board.facade.BasicBoard;
import app.freerouting.board.facade.RoutingBoard;
import app.freerouting.board.model.items.Item;
import app.freerouting.board.model.items.Via;
import app.freerouting.board.model.structure.FixedState;
import app.freerouting.board.trace.PolylineTrace;
import app.freerouting.core.library.Padstack;
import app.freerouting.geometry.planar.FloatPoint;
import app.freerouting.geometry.planar.IntBox;
import app.freerouting.geometry.planar.IntPoint;
import app.freerouting.geometry.planar.Polyline;
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
import java.util.TreeSet;

public class SpringOverNestProbe {

  private static final Gson GSON = new Gson();

  private static synchronized void row(JsonObject o) {
    System.out.println(GSON.toJson(o));
  }

  private static JsonObject obj(String type) {
    JsonObject o = new JsonObject();
    o.addProperty("type", type);
    return o;
  }

  private static JsonArray boxRow(IntBox box) {
    JsonArray a = new JsonArray();
    a.add(box.ll.x);
    a.add(box.ll.y);
    a.add(box.ur.x);
    a.add(box.ur.y);
    return a;
  }

  private static void emitPolyline(String field, Polyline polyline) {
    JsonArray corners = new JsonArray();
    for (int i = 0; i < polyline.cornerCount(); i++) {
      FloatPoint c = polyline.cornerApprox(i);
      JsonArray pt = new JsonArray();
      pt.add(Double.toString(c.x));
      pt.add(Double.toString(c.y));
      corners.add(pt);
    }
    JsonObject o = obj("polyline");
    o.addProperty("field", field);
    o.add("corners", corners);
    o.add("bbox", boxRow(polyline.boundingBox()));
    row(o);
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
    int layerCount = board.layerStructure.layers.length;
    int springNet = board.rules.nets.get("N001", 1).netNumber;

    // the nest pair: fixture pin 101 + a USER_FIXED via at its center
    Padstack viaPadstack =
        board.library.padstacks.add(
            new IntBox(new IntPoint(-5000, -5000), new IntPoint(5000, 5000)),
            0,
            layerCount - 1);
    Via nestVia =
        board.insertVia(
            viaPadstack,
            new IntPoint(200000, 300000),
            new int[] {board.rules.nets.get("N002", 1).netNumber},
            0,
            FixedState.USER_FIXED,
            false);

    JsonObject world = obj("world");
    world.addProperty("nestPinId", 101);
    world.add("nestPinBbox", boxRow(new IntBox(new IntPoint(180000, 280000), new IntPoint(220000, 320000))));
    world.addProperty("nestViaId", nestVia.getId());
    world.addProperty("nestViaX", 200000);
    world.addProperty("nestViaY", 300000);
    world.addProperty("springNet", springNet);
    world.addProperty("layers", layerCount);
    row(world);

    // provenance inventory (id-sorted): the inserted via only
    JsonArray items = new JsonArray();
    JsonObject it = new JsonObject();
    it.addProperty("id", nestVia.getId());
    it.addProperty("kind", nestVia.getClass().getSimpleName());
    it.addProperty("routable", nestVia.isRoutable());
    items.add(it);
    JsonObject inv = obj("inserts");
    inv.add("items", items);
    row(inv);

    // the spring: horizontal through pin and via centers
    Polyline springLine =
        new Polyline(new IntPoint(170000, 300000), new IntPoint(230000, 300000));

    // Part 2 — the depth-1 killer arm (spec-review F1): the RAW
    // counterclockwise springOver attempt at recursionDepth 1, the
    // budget under which the containment direction is observable. At
    // depth 20 the wrap recursion heals a wrong inner-first pick (the
    // final springOverObstacles circuit is identical either way); at
    // depth 1 the correct (outer-box) predicates wrap the pin
    // directly, while the swapped ones burn the budget on the inner
    // via and fail when the pin surfaces in the depth-0 recursion.
    // Reflection: the method is private (:611).
    java.lang.reflect.Method springOverMethod =
        TraceShover.class.getDeclaredMethod(
            "springOver",
            Polyline.class,
            int.class,
            int.class,
            int[].class,
            int.class,
            boolean.class,
            int.class,
            java.util.Set.class);
    springOverMethod.setAccessible(true);
    Polyline depth1 =
        (Polyline)
            springOverMethod.invoke(
                new TraceShover(board),
                springLine,
                50,
                0,
                new int[] {springNet},
                0,
                true,
                1,
                new TreeSet<>());
    JsonObject d1 = obj("springOverDepth1");
    d1.addProperty("resultNull", depth1 == null);
    row(d1);
    if (depth1 != null) {
      emitPolyline("depth1", depth1);
    }

    Polyline result =
        new TraceShover(board)
            .springOverObstacles(
                springLine, 50, 0, new int[] {springNet}, 0, new TreeSet<>());
    JsonObject out = obj("springOverObstacles");
    out.addProperty("resultNull", result == null);
    row(out);
    if (result != null) {
      emitPolyline("result", result);
    }
    Item failing = board.getShoveFailingObstacle();
    JsonObject state = obj("shoving-state");
    state.addProperty("failingObstacleId", failing == null ? -1 : failing.getId());
    row(state);
  }
}
