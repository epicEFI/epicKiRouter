// RoomDiagProbe.java — M4-T2 DIAGNOSTIC ONLY (not part of any golden).
// RouteEventProbe clone whose tap captures the expansion-room
// completion traces (COMPLETE_ROOM / ROOM_EDGE_REMOVE, five-arg
// FRLogger.trace rows in AutorouteEngine/Sorted45DegreeRoomNeighbours)
// plus the RAW_SECTION rows, so the room/door state around the events
// divergence can be diffed against the Rust side.
//
// Build/run (JDK 25, from the repo root):
//   mkdir -p /tmp/epic-t2-classes && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/javac \
//       -cp build/libs/freerouting-current-executable.jar \
//       -d /tmp/epic-t2-classes rust/harness/oracle/RoomDiagProbe.java && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/java \
//       -cp build/libs/freerouting-current-executable.jar:/tmp/epic-t2-classes \
//       app.freerouting.autoroute.pipeline.RoomDiagProbe \
//       rust/harness/fixtures/event-stream/e1_ripup.dsn 40000 > /tmp/e1-java-diag.jsonl
package app.freerouting.autoroute.pipeline;

import app.freerouting.board.facade.RoutingBoard;
import app.freerouting.board.actions.ItemIdGenerator;
import app.freerouting.core.RoutingJob;
import app.freerouting.core.StoppableThread;
import app.freerouting.drc.DesignRulesChecker;
import app.freerouting.io.BoardReadResult;
import app.freerouting.io.specctra.DsnReader;
import app.freerouting.logger.FRLogger;
import app.freerouting.settings.RouterSettings;
import app.freerouting.settings.RoutingCostSettings;
import app.freerouting.settings.sources.DefaultSettings;
import com.google.gson.Gson;
import com.google.gson.JsonObject;
import java.io.ByteArrayInputStream;
import java.nio.file.Files;
import java.nio.file.Paths;
import java.util.regex.Pattern;
import org.apache.logging.log4j.Level;
import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.core.LogEvent;
import org.apache.logging.log4j.core.LoggerContext;
import org.apache.logging.log4j.core.appender.AbstractAppender;
import org.apache.logging.log4j.core.config.Property;

public class RoomDiagProbe {

  private static final Gson GSON = new Gson();

  /** The room-completion + maze rows needed for the T2 door diagnosis. */
  private static final String[] TAP_TOKENS = {
    "COMPLETE_ROOM",
    "ROOM_EDGE_REMOVE",
    "RAW_SECTION assign",
  };

  private static String currentFixture = "?";

  private static boolean isTappedRow(String msg) {
    for (String token : TAP_TOKENS) {
      if (msg.contains(token)) {
        return true;
      }
    }
    return false;
  }

  /** Strip the five-arg wrapper, keep the rest verbatim. */
  private static final Pattern WRAPPER =
      java.util.regex.Pattern.compile("^\\[[^\\]]*\\] \\[([^\\]]*)\\] ");

  private static String normalize(String msg) {
    return WRAPPER.matcher(msg).replaceFirst("$1 ");
  }

  private static synchronized void row(JsonObject o) {
    System.out.println(GSON.toJson(o));
  }

  private static void attachNativeTap() {
    LoggerContext ctx = (LoggerContext) LogManager.getContext(false);
    AbstractAppender tap =
        new AbstractAppender("room-diag-tap", null, null, true, Property.EMPTY_ARRAY) {
          @Override
          public void append(LogEvent event) {
            String m = event.getMessage().getFormattedMessage();
            if (m != null && isTappedRow(m)) {
              JsonObject o = new JsonObject();
              o.addProperty("fixture", currentFixture);
              o.addProperty("msg", normalize(m));
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

  private static BatchAutorouter makeRouter(RoutingBoard board, RouterSettings settings) {
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
            board,
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

  private static RouterSettings baseSettings(RoutingBoard board) {
    RouterSettings settings = new RouterSettings(board);
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
    RoutingBoard board = parse(bytes, fileName);
    RouterSettings settings = baseSettings(board);
    settings.setFanoutEnabled(false);
    settings.setStartRipupCosts(startRipupCosts);
    settings.autorouter.maxPasses = 10;
    BatchAutorouter router = makeRouter(board, settings);
    boolean returned = router.runBatchLoop();
    JsonObject out = new JsonObject();
    out.addProperty("fixture", fixture);
    out.addProperty("type", "run");
    out.addProperty("returned", returned);
    row(out);
    DesignRulesChecker drc = new DesignRulesChecker(board, null);
    drc.calculateAllIncompletes();
    JsonObject w = new JsonObject();
    w.addProperty("fixture", fixture);
    w.addProperty("type", "incompletes");
    w.addProperty("incomplete_count", drc.getIncompleteCount());
    w.addProperty("max_connections", drc.maxConnections);
    row(w);
  }

  public static void main(String[] args) throws Exception {
    if (args.length < 2 || args.length % 2 != 0) {
      throw new IllegalArgumentException("usage: RoomDiagProbe <dsn> <startRipupCosts> ...");
    }
    FRLogger.granularTraceEnabled = true;
    if (app.freerouting.Freerouting.globalSettings == null) {
      app.freerouting.Freerouting.globalSettings =
          new app.freerouting.settings.GlobalSettings();
    }
    attachNativeTap();

    for (int i = 0; i + 1 < args.length; i += 2) {
      byte[] bytes = Files.readAllBytes(Paths.get(args[i]));
      String fileName = Paths.get(args[i]).getFileName().toString();
      int startRipupCosts = Integer.parseInt(args[i + 1]);
      runDriverWorld(bytes, fileName, startRipupCosts);
    }
  }
}
