// BatchDriverProbe.java — the M3-T12 jar-side probe oracle for the
// BATCH DRIVER assembly (BatchAutorouter + AutorouteBatchLoop +
// AutoroutePassRunner over the real runBatchLoop() entry point).
// AutorouteEngineProbe house pattern: ONE JVM per run, deterministic
// JSONL rows on stdout, double-run byte-identical.
//
// Build/run (JDK 25, from the repo root):
//   mkdir -p /tmp/epic-t12-classes && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/javac \
//       -cp build/libs/freerouting-current-executable.jar \
//       -d /tmp/epic-t12-classes rust/harness/oracle/BatchDriverProbe.java && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/java \
//       -cp build/libs/freerouting-current-executable.jar:/tmp/epic-t12-classes \
//       app.freerouting.autoroute.pipeline.BatchDriverProbe \
//       rust/harness/fixtures/locator-spike/t9_locator45.dsn
//
// The probe declares app.freerouting.autoroute.pipeline (the driver
// classes' package) so the package-private seams stay reachable:
// getAutorouteItems(RoutingBoard), the protected `job` field, and the
// reusable collections inside it.
//
// WHAT THE ORACLE VALIDATES (row-literal parity for the T12 port):
//   - the queue row  "Queuing item for routing: <Simple> on net '<n>'
//     (connected: X/Y)"  (BatchAutorouter.getAutorouteItems :393) and
//     the queue order/count against the Rust port's exact rows;
//   - the driver info rows through RoutingJob.logInfo/logDebug (the
//     "[PROBE] " prefix is the job.shortName face): the stage-start
//     row "Auto-routing stage started on board '<hash>' with baseline
//     score %.2f for %d unrouted item(s).", the pass-completed row
//     "Auto-routing pass #%d on board '<hash>' was completed in
//     %.2f seconds with score %s" (FRLogger.formatScore suffix), the
//     stagnation pair ("has not improved by more than 0.5 points in
//     the last 10 passes"/"since pass #8 ... after 18 passes"), the
//     fanout-recovery row, the max-items row, the pass header and
//     per-net incomplete rows, and the engine failure detail rows;
//   - the TASK-STATE event stream (NamedAlgorithm listener API):
//     STARTED/RUNNING/FINISHED/CANCELLED with pass numbers — the
//     exact state sequence the Rust driver renders as task_state
//     rows (the no-layers world fires CANCELLED pass=0 BEFORE the
//     IllegalArgumentException, matching the port's order);
//   - the BOARD-UPDATED event stream (RouterCounters public fields),
//     rendered in the Rust row shape — the counters oracle for the
//     board_updated rows ("phase=autoroute pass=1 queued=4 ...").
//
// NORMALIZATIONS: Java MD5 board hashes (32-hex) -> <boardhash>; pass
// durations -> <t> seconds (wall clock); identity hashes Class@h ->
// Class#ordinal. The FRLogger.info second argument (the job UUID) is
// not part of the formatted message. Everything else in a row is
// literal Java output.
//
// Worlds (fresh parse each; the fixture is t9_locator45.dsn, whose
// own nets 33/98 are the two unrouted pin pairs — 2 incompletes, 4
// queue items on the bare board):
//   settings_witness — the resolved cost table of
//                      new RouterSettings(board) through
//                      AutorouteControl (the table the Rust worlds
//                      hardcode as settings_ir), plus the scalar
//                      driver settings (via costs, start ripup,
//                      fanout, run-router, maxPasses/maxItems).
//   queue            — bare board, getAutorouteItems: count + per-item
//                      id/kind/nets; the native queue rows tapped.
//   completion       — layers [T,T], maxPasses 10: the router routes
//                      both pairs in pass 1 (pass 2 finds an empty
//                      queue) -> FINISHED; incompletes witness
//                      (getIncompleteCount=0 vs maxConnections=2 —
//                      the sum-vs-lower-bound semantics on record).
//   starved_local    — layers [F,T], fanout OFF: every attempt fails,
//                      constant 0.0 score, the pass-LOCAL stagnation
//                      tracker fires at pass 18.
//   starved_global   — layers [F,T], fanout ON (default): the
//                      one-time fanout recovery fires (pass 11) and
//                      resets the local counter; the GLOBAL tracker
//                      fires at pass 18.
//   max_passes       — starved, maxPasses 3: the loop re-enters at
//                      pass 4, the gate stops the run, Ok(false).
//   max_items        — starved, maxItems 1: the second attempt sees
//                      the budget spent, the info row fires mid-pass.
//   no_layers        — layers [F,F]: the warn row, the CANCELLED
//                      pass=0 event, the IllegalArgumentException.
package app.freerouting.autoroute.pipeline;

import app.freerouting.autoroute.events.BoardUpdatedEvent;
import app.freerouting.autoroute.events.BoardUpdatedEventListener;
import app.freerouting.autoroute.events.TaskStateChangedEvent;
import app.freerouting.autoroute.events.TaskStateChangedEventListener;
import app.freerouting.autoroute.maze.AutorouteControl;
import app.freerouting.board.actions.ItemIdGenerator;
import app.freerouting.board.facade.RoutingBoard;
import app.freerouting.board.model.items.Item;
import app.freerouting.core.RouterCounters;
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
import java.util.List;
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

public class BatchDriverProbe {

  private static final Gson GSON = new Gson();

  /**
   * The T12 driver-row tokens: every FRLogger emission the batch
   * pipeline makes on these worlds (info/debug/warn levels; the
   * five-arg trace family is off — granularTraceEnabled stays false).
   */
  private static final String[] TAP_TOKENS = {
    "Queuing item for routing",
    "Auto-routing stage started",
    "Auto-routing pass #",
    "was completed in",
    "has not improved by more than",
    "fanout recovery cleanup",
    "Restoring an earlier board",
    "Restoring the best board",
    "Max items limit reached",
    "The router was not able to improve",
    "Failed to route",
    "incompletes across",
    "incomplete(s)",
    "Cannot start autorouter",
    "The following connections could not be routed",
    "Queuing",
  };

  private static boolean isTappedRow(String msg) {
    for (String token : TAP_TOKENS) {
      if (msg.contains(token)) {
        return true;
      }
    }
    return false;
  }

  /** 32-hex board hashes (Java getHash = MD5 of the serialized board). */
  private static final Pattern BOARD_HASH = Pattern.compile("\\b[0-9a-f]{32}\\b");

  /** Pass durations ("0.02 seconds") — wall clock, never row-stable. */
  private static final Pattern DURATION = Pattern.compile("[0-9]+\\.[0-9]{2} seconds");

  /** Identity-hash tokens (Class@hash -> Class#ordinal by first appearance). */
  private static final Map<String, Integer> IDENTITY_ORDINALS = new TreeMap<>();
  private static final Pattern IDENTITY_HASH = Pattern.compile("[A-Za-z0-9.$]+@[0-9a-f]+");

  private static String normalize(String msg) {
    String out = DURATION.matcher(msg).replaceAll("<t> seconds");
    out = BOARD_HASH.matcher(out).replaceAll("<boardhash>");
    Matcher m = IDENTITY_HASH.matcher(out);
    StringBuilder sb = new StringBuilder();
    while (m.find()) {
      String token = m.group();
      Integer ordinal = IDENTITY_ORDINALS.computeIfAbsent(token, k -> IDENTITY_ORDINALS.size() + 1);
      String classPart = token.substring(0, token.indexOf('@'));
      m.appendReplacement(sb, Matcher.quoteReplacement(classPart + "#" + ordinal));
    }
    m.appendTail(sb);
    return sb.toString();
  }

  private static synchronized void row(JsonObject o) {
    System.out.println(GSON.toJson(o));
  }

  private static JsonObject obj(String type) {
    JsonObject o = new JsonObject();
    o.addProperty("type", type);
    return o;
  }

  /** The log4j tap for the native driver rows (EngineProbe wiring). */
  private static void attachNativeTap() {
    LoggerContext ctx = (LoggerContext) LogManager.getContext(false);
    AbstractAppender tap =
        new AbstractAppender("batch-driver-tap", null, null, true, Property.EMPTY_ARRAY) {
          @Override
          public void append(LogEvent event) {
            String m = event.getMessage().getFormattedMessage();
            if (m != null && isTappedRow(m)) {
              JsonObject o = obj("log_row");
              o.addProperty("level", event.getLevel().toString());
              o.addProperty("msg", normalize(m));
              row(o);
            }
          }
        };
    tap.start();
    // FULL wiring (EngineProbe pattern): FRLogger logs to the
    // "app.freerouting.Freerouting" NAMED logger, whose jar config
    // carries additivity=false — a root-only attachment receives
    // NOTHING. The named logger needs its own LoggerConfig + direct
    // appender; root stays attached for the residual loggers.
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
   * The router assembly the job ctor performs, minus the job: the
   * public 7-arg ctor over a bare StoppableThread, then a RoutingJob
   * (no-arg ctor) injected into the protected `job` field so the
   * batch loop's job.logInfo/logDebug rows fire with a fixed
   * "[PROBE] " prefix. The task-state and board-updated listeners
   * ride the NamedAlgorithm event API — the exact streams the Rust
   * driver renders as task_state / board_updated rows.
   */
  private static BatchAutorouter makeRouter(
      RoutingBoard board, RouterSettings settings, boolean fanoutEnabled) {
    StoppableThread thread =
        new StoppableThread() {
          // The router never runs this thread's action loop; the
          // batch loop polls the stop flag synchronously.
          @Override
          protected void threadAction() {}

          @Override
          public String toString() {
            return "probe-thread";
          }
        };
    // The job ctor derives removeUnconnectedVias = !isFanoutEnabled().
    BatchAutorouter router =
        new BatchAutorouter(
            thread,
            board,
            settings,
            !fanoutEnabled,
            true,
            settings.getStartRipupCosts(),
            500);
    router.job = new RoutingJob();
    router.job.shortName = "PROBE";
    router.job.routerSettings = settings;
    router.addTaskStateChangedEventListener(
        (TaskStateChangedEventListener)
            event -> {
              JsonObject o = obj("task_state");
              o.addProperty("state", event.getTaskState().toString());
              o.addProperty("pass", event.getPassNumber());
              o.addProperty("hash", normalize(event.getBoardHash()));
              row(o);
            });
    router.addBoardUpdatedEventListener(
        (BoardUpdatedEventListener)
            (BoardUpdatedEvent event) -> {
              RouterCounters c = event.getRouterCounters();
              JsonObject o = obj("board_updated");
              o.addProperty(
                  "counters",
                  "phase="
                      + c.phase
                      + " pass="
                      + c.passCount
                      + " queued="
                      + c.queuedToBeRoutedCount
                      + " skipped="
                      + c.skippedCount
                      + " ripped="
                      + c.rippedCount
                      + " failed="
                      + c.failedToBeRoutedCount
                      + " routed="
                      + c.routedCount
                      + " incomplete="
                      + c.incompleteCount
                      + " fanout_extra_vias="
                      + c.fanoutExtraViasCount);
              row(o);
            });
    return router;
  }

  /**
   * The world's settings: the geometry-derived cost table of
   * RouterSettings(board) — per-layer trace costs, via costs 1, bend
   * 0, start ripup 1, the table the Rust worlds hardcode — plus the
   * SCORE penalty scalars backfilled from DefaultSettings. The bare
   * board ctor leaves the penalty scalars null (an EMPTY scoring box
   * NPEs in Java's getMaximumScore unboxes — the "Java NPE parity"
   * arms the Rust port documents; a bare probe job reproduces exactly
   * that). Only the three score-read scalars are backfilled: the
   * engine-facing costs keep their board-derived fallbacks (via
   * costs 1, NOT the DefaultSettings 50).
   */
  private static RouterSettings baseSettings(RoutingBoard board) {
    RouterSettings settings = new RouterSettings(board);
    // The 7-arg ctor carries pullTightAccuracy=500, but the fanout
    // stage reads settings.tracePullTightAccuracy directly — mirror
    // the job-ctor fallback (null -> 500) on the settings box too.
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
    // The score's cost term unboxes scoring.viaCosts (no fallback —
    // third NPE face). Backfill the WITNESSED fallback value 1 (what
    // getViaCosts() resolves to when the box field is null), NOT the
    // DefaultSettings 50: the engine ctrl keeps its board-derived via
    // costs and the score's via term stays consistent with it. (The
    // Rust world scores with DefaultSettings via 50 — score VALUES
    // are not pinned, only row texts.)
    if (settings.scoring.viaCosts == null) {
      settings.scoring.viaCosts = 1;
    }
    return settings;
  }

  /**
   * The cost-table witness: the AutorouteControl view of
   * new RouterSettings(board) — the table the Rust worlds hardcode —
   * plus the scalar driver settings the batch loop reads.
   */
  private static void runSettingsWitness(RoutingBoard board) {
    RouterSettings settings = baseSettings(board);
    AutorouteControl ctrl = new AutorouteControl(board, 94, settings);
    JsonObject out = obj("settings_witness");
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
    out.addProperty("max_passes", settings.autorouter.maxPasses);
    out.addProperty("max_items", settings.autorouter.maxItems);
    row(out);
  }

  /**
   * The queue world: the package-private getAutorouteItems over the
   * bare board — count + per-item rows; the "Queuing item for
   * routing" debug rows are tapped natively from FRLogger.
   */
  private static void runQueueWorld(byte[] bytes, String fileName) throws Exception {
    RoutingBoard board = parse(bytes, fileName);
    BatchAutorouter router = makeRouter(board, baseSettings(board), true);
    List<Item> items = router.getAutorouteItems(board);
    JsonObject out = obj("queue");
    out.addProperty("count", items.size());
    JsonArray arr = new JsonArray();
    for (Item item : items) {
      JsonObject it = new JsonObject();
      it.addProperty("id", item.getId());
      it.addProperty("kind", item.getClass().getSimpleName());
      it.addProperty("nets", java.util.Arrays.toString(item.netNumbers));
      arr.add(it);
    }
    out.add("items", arr);
    row(out);
  }

  /**
   * A full runBatchLoop world: the run row (the boolean return), then
   * the incompletes witness — a FRESH DesignRulesChecker's
   * getIncompleteCount (the per-net SUM) against maxConnections (the
   * endpoint lower bound). On the routed completion world the two
   * diverge (0 vs 2) — the semantics discriminator on record.
   */
  private static void runDriverWorld(
      String world, byte[] bytes, String fileName, java.util.function.Consumer<RouterSettings> tune,
      boolean fanoutEnabled)
      throws Exception {
    RoutingBoard board = parse(bytes, fileName);
    RouterSettings settings = baseSettings(board);
    tune.accept(settings);
    // The loop reads settings.isFanoutEnabled() directly (the board
    // ctor leaves the Boolean null → false); make the ctor arg and
    // the setting agree, like a settings-sourced job would.
    settings.setFanoutEnabled(fanoutEnabled);
    BatchAutorouter router = makeRouter(board, settings, fanoutEnabled);
    JsonObject out = obj("run");
    out.addProperty("world", world);
    try {
      boolean returned = router.runBatchLoop();
      out.addProperty("returned", returned);
    } catch (IllegalArgumentException e) {
      out.addProperty("caught", e.getClass().getSimpleName());
      out.addProperty("message", normalize(e.getMessage()));
    }
    row(out);
    DesignRulesChecker drc = new DesignRulesChecker(board, null);
    drc.calculateAllIncompletes();
    JsonObject w = obj("incompletes");
    w.addProperty("world", world);
    w.addProperty("incomplete_count", drc.getIncompleteCount());
    w.addProperty("max_connections", drc.maxConnections);
    row(w);
  }

  public static void main(String[] args) throws Exception {
    byte[] bytes = Files.readAllBytes(Paths.get(args[0]));
    String fileName = args.length > 1 ? args[1] : "t9_locator45.dsn";
    // The driver rows are info/debug level — the granular five-arg
    // trace family stays OFF for this probe.
    attachNativeTap();

    // World: the cost-table + scalar settings witness.
    runSettingsWitness(parse(bytes, fileName));

    // World: the bare-board queue (4 items, the nets-33/98 pairs).
    runQueueWorld(bytes, fileName);

    // World: the completion run, fanout OFF — the M4-seam mirror of
    // the Rust completion world (the port's driver has no fanout
    // stage, so the fanout-disabled world is the row-parity world;
    // note it carries the job-ctor derivation removeUnconnectedVias
    // = !fanout = TRUE, which the Rust world's separate
    // remove_unconnected_vias field keeps false — counters may
    // differ in the tail-removal face, not in the driver rows).
    runDriverWorld(
        "completion_fanout_off",
        bytes,
        fileName,
        settings -> {
          settings.autorouter.maxPasses = 10;
          settings.setFanoutEnabled(false);
        },
        false);

    // World: the completion run, fanout ON (the production default) —
    // the LIVE M4 fanout stage on record (its rows are the stage's
    // own dossier; the autoroute-phase rows ride on top).
    runDriverWorld(
        "completion_fanout_on",
        bytes,
        fileName,
        settings -> settings.autorouter.maxPasses = 10,
        true);

    // World: starved + fanout OFF -> pass-local stagnation at 18.
    runDriverWorld(
        "starved_local",
        bytes,
        fileName,
        settings -> {
          settings.setLayerActive(0, false);
          settings.setLayerActive(1, true);
          settings.setFanoutEnabled(false);
        },
        false);

    // World: starved + fanout ON -> the one-time recovery (pass 11),
    // then the GLOBAL stagnation tracker fires at pass 18.
    runDriverWorld(
        "starved_global",
        bytes,
        fileName,
        settings -> {
          settings.setLayerActive(0, false);
          settings.setLayerActive(1, true);
        },
        true);

    // World: starved, maxPasses 3 — the currentPass gate at pass 4.
    runDriverWorld(
        "max_passes",
        bytes,
        fileName,
        settings -> {
          settings.setLayerActive(0, false);
          settings.setLayerActive(1, true);
          settings.setFanoutEnabled(false);
          settings.autorouter.maxPasses = 3;
        },
        false);

    // World: starved, maxItems 1 — the pass-runner budget stop.
    runDriverWorld(
        "max_items",
        bytes,
        fileName,
        settings -> {
          settings.setLayerActive(0, false);
          settings.setLayerActive(1, true);
          settings.setFanoutEnabled(false);
          settings.autorouter.maxItems = 1;
        },
        false);

    // World: no active signal layers — warn + CANCELLED(0) + the IAE.
    runDriverWorld(
        "no_layers",
        bytes,
        fileName,
        settings -> {
          settings.setLayerActive(0, false);
          settings.setLayerActive(1, false);
        },
        true);
  }
}
