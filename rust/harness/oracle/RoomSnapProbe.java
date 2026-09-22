// RoomSnapProbe.java — M4-T2 DIAGNOSTIC ONLY (not part of any golden).
// RouteEventProbe clone that SNAPSHOTs the expansion-room + door state
// of the live AutorouteEngine at the first RAW_SECTION assign row of a
// chosen net, by reflection through RoutingBoard.autorouteEngine (the
// retained engine — the probe JVM is single-threaded, so the state at
// the trace call is exactly the state the maze evaluation used).
//
// Build/run (JDK 25, from the repo root):
//   mkdir -p /tmp/epic-t2-classes && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/javac \
//       -cp build/libs/freerouting-current-executable.jar \
//       -d /tmp/epic-t2-classes rust/harness/oracle/RoomSnapProbe.java && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/java \
//       -cp build/libs/freerouting-current-executable.jar:/tmp/epic-t2-classes \
//       app.freerouting.autoroute.pipeline.RoomSnapProbe \
//       rust/harness/fixtures/event-stream/e1_ripup.dsn 40000 2
//
// argv: <dsn> <startRipupCosts> <snapshotNet> — the dump fires on the
// FIRST assign row whose trailing net=<snapshotNet>.
package app.freerouting.autoroute.pipeline;

import app.freerouting.board.facade.RoutingBoard;
import app.freerouting.board.actions.ItemIdGenerator;
import app.freerouting.autoroute.maze.AutorouteEngine;
import app.freerouting.core.RoutingJob;
import app.freerouting.core.StoppableThread;
import app.freerouting.drc.DesignRulesChecker;
import app.freerouting.geometry.planar.IntBox;
import app.freerouting.geometry.planar.IntOctagon;
import app.freerouting.geometry.planar.TileShape;
import app.freerouting.io.BoardReadResult;
import app.freerouting.io.specctra.DsnReader;
import app.freerouting.logger.FRLogger;
import app.freerouting.settings.RouterSettings;
import app.freerouting.settings.RoutingCostSettings;
import app.freerouting.settings.sources.DefaultSettings;
import com.google.gson.Gson;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import java.io.ByteArrayInputStream;
import java.lang.reflect.Field;
import java.nio.file.Files;
import java.nio.file.Paths;
import java.util.Collection;
import java.util.List;
import java.util.regex.Pattern;
import org.apache.logging.log4j.Level;
import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.core.LogEvent;
import org.apache.logging.log4j.core.LoggerContext;
import org.apache.logging.log4j.core.appender.AbstractAppender;
import org.apache.logging.log4j.core.config.Property;

public class RoomSnapProbe {

  private static final Gson GSON = new Gson();

  private static final String[] TAP_TOKENS = {"RAW_SECTION assign"};

  private static String currentFixture = "?";
  private static int snapshotNet = -1;
  private static boolean snapped = false;
  private static RoutingBoard board = null;

  private static boolean isTappedRow(String msg) {
    for (String token : TAP_TOKENS) {
      if (msg.contains(token)) {
        return true;
      }
    }
    return false;
  }

  private static final Pattern WRAPPER =
      java.util.regex.Pattern.compile("^\\[[^\\]]*\\] \\[([^\\]]*)\\] ");

  private static String normalize(String msg) {
    return WRAPPER.matcher(msg).replaceFirst("$1 ");
  }

  private static synchronized void row(JsonObject o) {
    System.out.println(GSON.toJson(o));
  }

  private static String shapeToString(TileShape shape) {
    if (shape == null) {
      return "null";
    }
    if (shape instanceof IntOctagon oct) {
      return "Oct(lx="
          + oct.leftX
          + ",ly="
          + oct.bottomY
          + ",rx="
          + oct.rightX
          + ",uy="
          + oct.topY
          + ",ulx="
          + oct.upperLeftDiagonalX
          + ",lrx="
          + oct.lowerRightDiagonalX
          + ",llx="
          + oct.lowerLeftDiagonalX
          + ",urx="
          + oct.upperRightDiagonalX
          + ")";
    }
    if (shape instanceof IntBox box) {
      return "Box[("
          + box.ll.x
          + ","
          + box.ll.y
          + ")..("
          + box.ur.x
          + ","
          + box.ur.y
          + ")]";
    }
    return shape.getClass().getSimpleName() + " " + shape.boundingBox();
  }

  private static List<?> engineRoomList(AutorouteEngine engine, String fieldName) {
    try {
      Field f = AutorouteEngine.class.getDeclaredField(fieldName);
      f.setAccessible(true);
      Object val = f.get(engine);
      @SuppressWarnings("unchecked")
      List<?> list = (List<?>) val;
      return list;
    } catch (Exception e) {
      System.out.println("{\"type\":\"snap_error\",\"field\":\"" + fieldName
          + "\",\"err\":\"" + e + "\"}");
      return List.of();
    }
  }

  private static AutorouteEngine boardEngine(RoutingBoard b) {
    try {
      Field f = findField(b.getClass(), "autorouteEngine");
      f.setAccessible(true);
      return (AutorouteEngine) f.get(b);
    } catch (Exception e) {
      System.out.println("{\"type\":\"snap_error\",\"field\":\"autorouteEngine\",\"err\":\""
          + e + "\"}");
      return null;
    }
  }

  private static Field findField(Class<?> c, String name) throws NoSuchFieldException {
    for (Class<?> k = c; k != null; k = k.getSuperclass()) {
      try {
        return k.getDeclaredField(name);
      } catch (NoSuchFieldException e) {
        // continue up
      }
    }
    throw new NoSuchFieldException(name);
  }

  /** The room snapshot: every incomplete + complete room, and all of
   * their doors (with current lazy shapes and section counts). */
  private static void dumpSnapshot(String tag) {
    JsonObject out = new JsonObject();
    out.addProperty("type", "room_snapshot");
    out.addProperty("tag", tag);
    out.addProperty("fixture", currentFixture);
    AutorouteEngine engine = boardEngine(board);
    if (engine == null) {
      row(out);
      return;
    }
    JsonArray rooms = new JsonArray();
    Collection<?>[] lists = new Collection<?>[] {
        engineRoomList(engine, "incompleteExpansionRooms"),
        engineRoomList(engine, "completeExpansionRooms"),
    };
    String[] kinds = new String[] {"incomplete", "complete"};
    for (int li = 0; li < lists.length; li++) {
      for (Object roomObj : lists[li]) {
        app.freerouting.autoroute.expansion.ExpansionRoom room =
            (app.freerouting.autoroute.expansion.ExpansionRoom) roomObj;
        JsonObject r = new JsonObject();
        r.addProperty("kind", kinds[li]);
        r.addProperty("class", room.getClass().getSimpleName());
        r.addProperty("id", room.getId());
        r.addProperty("layer", room.getLayer());
        r.addProperty("shape", shapeToString(room.getShape()));
        JsonArray doors = new JsonArray();
        for (app.freerouting.autoroute.expansion.ExpansionDoor door : room.getDoors()) {
          JsonObject d = new JsonObject();
          app.freerouting.autoroute.expansion.ExpansionRoom other =
              door.otherRoom(room);
          d.addProperty("other_id", other == null ? -1 : other.getId());
          d.addProperty("other_class", other == null ? "null" : other.getClass().getSimpleName());
          d.addProperty("dim", door.getDimension());
          d.addProperty("shape", shapeToString(door.getShape()));
          try {
            d.addProperty("sections", door.mazeSearchElementCount());
          } catch (RuntimeException e) {
            d.addProperty("sections", -1);
          }
          doors.add(d);
        }
        r.add("doors", doors);
        rooms.add(r);
      }
    }
    out.add("rooms", rooms);
    row(out);
    dumpTraceTreeShapes();
  }

  /** Every PolylineTrace item: id, net, layer, polyline corners, and
   * its search-tree shapes — the decomposition ground truth. */
  private static void dumpTraceTreeShapes() {
    JsonObject out = new JsonObject();
    out.addProperty("type", "trace_tree_shapes");
    out.addProperty("fixture", currentFixture);
    JsonArray traces = new JsonArray();
    try {
      AutorouteEngine engine = boardEngine(board);
      Object searchTreeObj = engine.getClass().getField("autorouteSearchTree").get(engine);
      app.freerouting.board.searchtree.ShapeSearchTree tree =
          (app.freerouting.board.searchtree.ShapeSearchTree) searchTreeObj;
      for (Object itemObj : board.getItems()) {
        if (!(itemObj instanceof app.freerouting.board.trace.PolylineTrace trace)) {
          continue;
        }
        JsonObject t = new JsonObject();
        t.addProperty("item_id", trace.getId());
        t.addProperty("net_no", trace.netNumbers[0]);
        t.addProperty("layer", trace.getLayer());
        StringBuilder pts = new StringBuilder();
        for (app.freerouting.geometry.planar.Point p : trace.polyline().corners()) {
          if (pts.length() > 0) {
            pts.append(' ');
          }
          pts.append('(').append(((app.freerouting.geometry.planar.IntPoint) p).x)
              .append(',').append(((app.freerouting.geometry.planar.IntPoint) p).y).append(')');
        }
        t.addProperty("polyline", pts.toString());
        StringBuilder shapes = new StringBuilder();
        int shapeCount = trace.treeShapeCount(tree);
        for (int i = 0; i < shapeCount; i++) {
          if (shapes.length() > 0) {
            shapes.append(' ');
          }
          shapes.append('[').append(i).append(']')
              .append(shapeToString(trace.getTreeShape(tree, i)));
        }
        t.addProperty("tree_shapes", shapes.toString());
        traces.add(t);
      }
    } catch (Exception e) {
      out.addProperty("error", String.valueOf(e));
    }
    out.add("traces", traces);
    row(out);
  }

  private static void attachNativeTap() {
    LoggerContext ctx = (LoggerContext) LogManager.getContext(false);
    AbstractAppender tap =
        new AbstractAppender("room-snap-tap", null, null, true, Property.EMPTY_ARRAY) {
          @Override
          public void append(LogEvent event) {
            String m = event.getMessage().getFormattedMessage();
            if (m == null || !isTappedRow(m)) {
              return;
            }
            JsonObject o = new JsonObject();
            o.addProperty("fixture", currentFixture);
            o.addProperty("type", "trace_row");
            o.addProperty("msg", normalize(m));
            row(o);
            if (!snapped && m.endsWith("net=" + snapshotNet)) {
              snapped = true;
              dumpSnapshot("first-assign-net" + snapshotNet);
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

  private static RoutingBoard parse(byte[] bytes, String fileName) throws Exception {
    BoardReadResult read =
        DsnReader.readBoard(
            new ByteArrayInputStream(bytes), null, new ItemIdGenerator(), fileName);
    if (!(read instanceof BoardReadResult.Success success)) {
      throw new IllegalStateException("parse failed");
    }
    RoutingBoard b = (RoutingBoard) success.board();
    b.searchTreeManager.reinsertTreeItems();
    return b;
  }

  private static BatchAutorouter makeRouter(RoutingBoard b, RouterSettings settings) {
    StoppableThread thread =
        new StoppableThread() {
          @Override
          protected void threadAction() {}

          @Override
          public String toString() {
            return "probe-thread";
          }
        };
    BatchAutorouter router =
        new BatchAutorouter(
            thread,
            b,
            settings,
            true,
            true,
            settings.getStartRipupCosts(),
            500);
    router.job = new RoutingJob();
    router.job.shortName = "PROBE";
    router.job.routerSettings = settings;
    return router;
  }

  private static RouterSettings baseSettings(RoutingBoard b) {
    RouterSettings settings = new RouterSettings(b);
    settings.tracePullTightAccuracy = 500;
    RoutingCostSettings fallback = new DefaultSettings().getSettings().scoring;
    if (settings.scoring.unroutedNetPenalty == null) {
      settings.scoring.unroutedNetPenalty = fallback.unroutedNetPenalty;
    }
    if (settings.scoring.clearanceViolationPenalty == null) {
      settings.scoring.clearanceViolationPenalty = fallback.clearanceViolationPenalty;
    }
    if (settings.scoring.bendPenalty == null) {
      settings.scoring.bendPenalty = fallback.bendPenalty;
    }
    if (settings.scoring.viaCosts == null) {
      settings.scoring.viaCosts = 1;
    }
    return settings;
  }

  private static void runDriverWorld(byte[] bytes, String fileName, int startRipupCosts)
      throws Exception {
    String fixture = fileName.replaceFirst("\\.dsn$", "");
    currentFixture = fixture;
    snapped = false;
    RoutingBoard b = parse(bytes, fileName);
    board = b;
    RouterSettings settings = baseSettings(b);
    settings.setFanoutEnabled(false);
    settings.setStartRipupCosts(startRipupCosts);
    settings.autorouter.maxPasses = 10;
    BatchAutorouter router = makeRouter(b, settings);
    boolean returned = router.runBatchLoop();
    JsonObject out = new JsonObject();
    out.addProperty("fixture", fixture);
    out.addProperty("type", "run");
    out.addProperty("returned", returned);
    row(out);
    DesignRulesChecker drc = new DesignRulesChecker(b, null);
    drc.calculateAllIncompletes();
    JsonObject w = new JsonObject();
    w.addProperty("fixture", fixture);
    w.addProperty("type", "incompletes");
    w.addProperty("incomplete_count", drc.getIncompleteCount());
    w.addProperty("max_connections", drc.maxConnections);
    row(w);
  }

  public static void main(String[] args) throws Exception {
    if (args.length != 3) {
      throw new IllegalArgumentException(
          "usage: RoomSnapProbe <dsn> <startRipupCosts> <snapshotNet>");
    }
    FRLogger.granularTraceEnabled = true;
    if (app.freerouting.Freerouting.globalSettings == null) {
      app.freerouting.Freerouting.globalSettings =
          new app.freerouting.settings.GlobalSettings();
    }
    snapshotNet = Integer.parseInt(args[2]);
    attachNativeTap();

    byte[] bytes = Files.readAllBytes(Paths.get(args[0]));
    String fileName = Paths.get(args[0]).getFileName().toString();
    runDriverWorld(bytes, fileName, Integer.parseInt(args[1]));
  }
}
