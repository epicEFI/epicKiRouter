// RouteEventProbe.java — the M3-T16 jar-side probe oracle for the
// ROUTE EVENT STREAM (the maze-level differential parity corpus):
// the RAW_SECTION assign/skip rows of MazeSearchEngine plus the
// AutoroutePassRunner's compare_trace_ripped_item /
// compare_trace_route_item rows, captured through the real
// runBatchLoop() entry point. AutorouteEngineProbe/BatchDriverProbe
// house pattern: ONE JVM per run, deterministic JSONL rows on stdout,
// double-run byte-identical (the harness captures twice and diffs).
//
// Build/run (JDK 25, from the repo root):
//   mkdir -p /tmp/epic-t16-classes && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/javac \
//       -cp build/libs/freerouting-current-executable.jar \
//       -d /tmp/epic-t16-classes rust/harness/oracle/RouteEventProbe.java && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/java \
//       -cp build/libs/freerouting-current-executable.jar:/tmp/epic-t16-classes \
//       app.freerouting.autoroute.pipeline.RouteEventProbe \
//       rust/harness/fixtures/locator-spike/t9_locator45.dsn 1
//
// argv: alternating (dsn-path, startRipupCosts) pairs — the ONE tuned
// scalar per fixture world (t9/t7 use the baseSettings default 1; the
// forced-ripup fixture e1_ripup uses 40000 so the pass-2 reroute takes
// the detour instead of ripping back). Everything else is the
// completion world: fanout OFF, all layers active, maxPasses 10.
//
// The probe declares app.freerouting.autoroute.pipeline so the
// package-private driver seams stay reachable (the protected `job`
// field, BatchAutorouter's ctor).
//
// ROW GRAMMAR (what lands in the JSONL):
//   {"fixture":"<name>","type":"settings_witness",...} — the resolved
//     cost table + scalars of the world (the Rust compare builds its
//     BatchSettings FROM this witness: settings parity by construction).
//   {"fixture":"<name>","type":"trace_row","msg":"..."} — the tapped
//     rows, in emission order. RAW_SECTION rows are one-arg
//     FRLogger.trace rows (already bare); ripped_item/route_item rows
//     are FIVE-arg granular rows whose formatted message carries the
//     "[method] [operation] " wrapper and the ": <impacted items>"
//     tail — BOTH STRIPPED here so the stored text is the bare row the
//     Rust mirror emits.
//   {"fixture":"<name>","type":"run","returned":b} + {"...","type":
//     "incompletes",...} — the run-outcome witnesses.
//
// NORMALIZATIONS: identity hashes Class@h -> Class#ordinal (defensive;
// these rows carry ids, not hashes). No wall-clock value appears in
// any tapped row (the info-level duration rows are NOT tapped).
package app.freerouting.autoroute.pipeline;

import app.freerouting.autoroute.maze.AutorouteControl;
import app.freerouting.board.actions.ItemIdGenerator;
import app.freerouting.board.facade.RoutingBoard;
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
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import java.io.ByteArrayInputStream;
import java.nio.file.Files;
import java.nio.file.Paths;
import java.util.Map;
import java.util.TreeMap;
import java.util.regex.Matcher;
import java.util.regex.Pattern;
import org.apache.logging.log4j.Level;
import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.core.LogEvent;
import org.apache.logging.log4j.core.LoggerContext;
import org.apache.logging.log4j.core.appender.AbstractAppender;
import org.apache.logging.log4j.core.config.Property;

public class RouteEventProbe {

  private static final Gson GSON = new Gson();

  /**
   * The T16 event-stream tokens: the maze engine's RAW_SECTION rows
   * (one-arg trace) and the pass runner's two five-arg comparison
   * rows. Everything else the driver logs is filtered out.
   */
  private static final String[] TAP_TOKENS = {
    "RAW_SECTION assign",
    "RAW_SECTION skip",
    "compare_trace_ripped_item",
    "compare_trace_route_item",
  };

  /**
   * The fixture the CURRENT driver world is running — the tap stamps
   * every trace row with it (the router logs through the static
   * FRLogger, which carries no fixture context). Set/cleared around
   * each world; the whole run is synchronous on the main thread.
   */
  private static String currentFixture = "?";

  /**
   * Tap-breadth asymmetry (quality review O-1): the Java tap matches
   * SUBSTRINGS (contains) while the Rust classifier
   * (route_events.rs kind_of) is starts_with. Harmless for the corpus —
   * every tapped row leads with its token — and any over-capture is
   * LOUD at compare time: a leaked extra kind fails the golden's strict
   * unpinned-row face (witnessed: a broadened tap grows e1 340 to 904
   * rows and the compare rejects the grown golden).
   */
  private static boolean isTappedRow(String msg) {
    for (String token : TAP_TOKENS) {
      if (msg.contains(token)) {
        return true;
      }
    }
    return false;
  }

  /**
   * The five-arg wrapper: `"[%s] [%s] %s: %s".formatted(method,
   * operation, message, impactedItems)` (FRLogger.trace) — replaced by
   * the bare `operation message` text the Rust mirror emits (the
   * "[method] " wrapper and the ": <impactedItems>" tail both go; the
   * OPERATION stays, because the Rust row leads with it). One-arg rows
   * (RAW_SECTION) match neither pattern.
   */
  private static final Pattern WRAPPER = Pattern.compile("^\\[[^\\]]*\\] \\[([^\\]]*)\\] ");

  private static final Pattern IMPACTED_TAIL =
      Pattern.compile(": Net #\\d+,Item #\\d+(,Type=[A-Za-z]+)?$");

  /** Identity-hash tokens (Class@hash -> Class#ordinal by first appearance). */
  private static final Map<String, Integer> IDENTITY_ORDINALS = new TreeMap<>();
  private static final Pattern IDENTITY_HASH = Pattern.compile("[A-Za-z0-9.$]+@[0-9a-f]+");

  private static String normalize(String msg) {
    String out = WRAPPER.matcher(msg).replaceFirst("$1 ");
    out = IMPACTED_TAIL.matcher(out).replaceAll("");
    if (out.indexOf('@') >= 0) {
      Matcher m = IDENTITY_HASH.matcher(out);
      StringBuilder sb = new StringBuilder();
      while (m.find()) {
        String token = m.group();
        Integer ordinal =
            IDENTITY_ORDINALS.computeIfAbsent(token, k -> IDENTITY_ORDINALS.size() + 1);
        String classPart = token.substring(0, token.indexOf('@'));
        m.appendReplacement(sb, Matcher.quoteReplacement(classPart + "#" + ordinal));
      }
      m.appendTail(sb);
      out = sb.toString();
    }
    return out;
  }

  private static synchronized void row(JsonObject o) {
    System.out.println(GSON.toJson(o));
  }

  /** The log4j tap (EngineProbe wiring — the named logger is additive=false). */
  private static void attachNativeTap() {
    LoggerContext ctx = (LoggerContext) LogManager.getContext(false);
    AbstractAppender tap =
        new AbstractAppender("route-event-tap", null, null, true, Property.EMPTY_ARRAY) {
          @Override
          public void append(LogEvent event) {
            String m = event.getMessage().getFormattedMessage();
            if (m != null && isTappedRow(m)) {
              JsonObject o = new JsonObject();
              o.addProperty("fixture", currentFixture);
              o.addProperty("type", "trace_row");
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

  /** A fresh parse of the fixture bytes (EngineProbe pattern). */
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

  /**
   * The router assembly (BatchDriverProbe pattern): the public 7-arg
   * ctor over a bare StoppableThread, a RoutingJob injected into the
   * protected `job` field (the batch loop's job.log* calls), fanout
   * OFF (removeUnconnectedVias = !fanout = TRUE, the job-ctor
   * derivation). No task-state / board-updated listeners: the T16
   * stream is the trace rows only.
   */
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

  /**
   * The world's settings (BatchDriverProbe.baseSettings): the
   * geometry-derived cost table of RouterSettings(board), pullTight
   * 500, and the SCORE penalty scalars backfilled from DefaultSettings
   * (the bare-board ctor leaves them null; an EMPTY scoring box NPEs in
   * getMaximumScore). The engine-facing costs keep their board-derived
   * fallbacks (via costs 1, NOT the DefaultSettings 50).
   */
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

  /**
   * The settings witness: the AutorouteControl view of the world's
   * cost table (what the Rust compare builds its BatchSettings from)
   * plus the scalars. Emitted AFTER the tune so the recorded values
   * are the resolved ones.
   */
  private static void runSettingsWitness(RoutingBoard board, String fixture, RouterSettings settings) {
    AutorouteControl ctrl =
        new AutorouteControl(board, firstRoutedNet(board), settings);
    JsonObject out = new JsonObject();
    out.addProperty("fixture", fixture);
    out.addProperty("type", "settings_witness");
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
    out.addProperty("plane_via_costs", ctrl.settings.getPlaneViaCosts());
    out.addProperty("vias_allowed", ctrl.viasAllowed);
    out.addProperty("automatic_neckdown", ctrl.withNeckdown);
    out.addProperty("start_ripup_costs", settings.getStartRipupCosts());
    out.addProperty("fanout_enabled", settings.isFanoutEnabled());
    out.addProperty("run_router", settings.getRunRouter());
    out.addProperty("max_items", settings.autorouter.maxItems);
    out.addProperty("unrouted_net_penalty", Double.toString(settings.scoring.unroutedNetPenalty));
    out.addProperty(
        "clearance_violation_penalty", Double.toString(settings.scoring.clearanceViolationPenalty));
    out.addProperty("bend_penalty", Double.toString(settings.scoring.bendPenalty));
    out.addProperty("scoring_via_costs", settings.scoring.viaCosts);
    out.addProperty("max_passes", settings.autorouter.maxPasses);
    row(out);
  }

  /** The smallest existing net number (the witness's AutorouteControl
   * needs A net; the cost table itself is net-independent on these
   * plane-less fixtures). */
  private static int firstRoutedNet(RoutingBoard board) {
    for (int net = 1; net <= board.rules.nets.maxNetNumber(); net++) {
      if (board.rules.nets.get(net) != null) {
        return net;
      }
    }
    return 1;
  }

  /**
   * The completion world: fanout OFF, all layers active, maxPasses 10,
   * the fixture's startRipupCosts. Emits the run + incompletes witness
   * rows after the tapped stream.
   */
  private static void runDriverWorld(byte[] bytes, String fileName, int startRipupCosts)
      throws Exception {
    String fixture = fileName.replaceFirst("\\.dsn$", "");
    currentFixture = fixture;
    // Quality review MIN-3: the identity map is PER-WORLD state. Without
    // this clear, a hash-carrying row's ordinal would depend on first
    // appearance ACROSS fixtures (deterministic only for a fixed
    // manifest order; silently re-stamped by a reorder). Dead-in-practice
    // today (zero '@' in the committed golden) — the clear changes no
    // captured bytes and is untestable until a hash row exists, so it
    // deliberately carries no pin.
    IDENTITY_ORDINALS.clear();
    RoutingBoard board = parse(bytes, fileName);
    RouterSettings settings = baseSettings(board);
    settings.setFanoutEnabled(false);
    settings.setStartRipupCosts(startRipupCosts);
    settings.autorouter.maxPasses = 10;
    runSettingsWitness(board, fixture, settings);
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
      throw new IllegalArgumentException("usage: RouteEventProbe <dsn> <startRipupCosts> ...");
    }
    // The five-arg granular rows need granularTraceEnabled AND a bare
    // GlobalSettings (DebugControl filters through it; a probe JVM has
    // none) — AutorouteEngineProbe pattern.
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
