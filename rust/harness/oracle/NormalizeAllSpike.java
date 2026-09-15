// NormalizeAllSpike.java — jar spike for M2 Task 13 (the
// normalizeAllTraces DRIVER: BasicBoard.normalizeAllTraces
// BasicBoard.java:799-885, the read-path call Wiring.java:343-353,
// over the per-trace seams PolylineTrace.normalize :801 and
// BasicBoard.removeIfCycle :1336).
//
// Same harness pattern as SplitSpike (T12): the ContactsSpike/CombineSpike
// PURE board (parse ids 1..9; the in-read normalizeAllTraces call leaves
// it untouched), trap geometry added POST-parse through
// insertTraceWithoutCleaning (never normalizes), ids deterministic from
// 10. Run (JDK 25, repo root; the sed strips FRLogger's leading
// timestamp so two runs diff clean):
//   mkdir -p /tmp/epic-t13-classes && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/javac -cp build/libs/freerouting-current-executable.jar \
//       -d /tmp/epic-t13-classes rust/harness/oracle/NormalizeAllSpike.java && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/java -cp build/libs/freerouting-current-executable.jar:/tmp/epic-t13-classes \
//       app.freerouting.datastructures.NormalizeAllSpike 2>&1 \
//       | sed -E 's/^[0-9]{4}-[0-9]{2}-[0-9]{2} [0-9]{2}:[0-9]{2}:[0-9]{2}\.[0-9]+ //' \
//       | tee /tmp/epic-t13-normalize-all.out
//
// The ITERATION-COUNT instrument: BasicBoard.normalizeAllTraces exposes
// no counters, so the spike runs a line-for-line TWIN of the driver
// loop (same grouping walk, same gates, same re-collect — calling the
// REAL trace.normalize(null) and REAL board.removeIfCycle) with a
// per-net iteration counter, parameterized over the ACROSS-NET group
// order (ascending vs descending keys). The REAL driver runs too, on
// its own fresh board copy; the CA<n>_AGREE rows prove the twin's end
// state equals the driver's end state (both twin orders AND the driver
// agree — the T13 net-group isolation witness), so the twin's
// iteration counts ARE the driver's. The driver's HashMap over small
// Integer keys iterates ascending; twin_asc mirrors it, twin_desc is
// the order-swap witness.
//
// Sections (output line prefixes):
//   CA1  — the 3-segment collinear chain (one net): combine folds the
//          chain inside iteration 1 (combine is itself iterative),
//          iteration 2 confirms no-op -> exit. Survivor id, burned
//          ids, NEXT_ID exact.
//   CA2  — two nets, each a chain, PLUS the multi-net pair X,Y
//          (nets [1,2]): the fold happens in whichever group runs
//          first, the second group's pass over it is a no-op.
//          twin_asc vs twin_desc vs driver: identical end boards.
//   CA3  — the cycle square (4 open traces, same net, UNFIXED):
//          normalize false on every edge -> !isUserFixed() &&
//          removeIfCycle fires on the first edge walked; the whole
//          connected ring goes in one removeItems; iteration 2 walks
//          an empty group -> exit.
//   CA4  — the same square USER_FIXED ((type protect) state):
//          normalize refuses nothing to split/combine, removeIfCycle
//          is SKIPPED by the driver's !isUserFixed() gate; board
//          unchanged, result false, single iteration.
//   CA5  — the crossing pair (same net): normalize(null) splits BOTH
//          traces at the crossing (the T12 surface) and the follow-up
//          iterations combine the collinear pieces; the exact id churn
//          and per-iteration states are the port's pin source.
//   CA6  — dsn-0151 itself (fixtures/Issue723-CombineStackOverflow.dsn,
//          the real fixture): the READ already ran normalizeAllTraces
//          inside the wiring scope (Wiring.java:343-353), so the
//          captured stats (items=2, traces=1) and survivor row ARE the
//          post-driver state; the spike then runs the driver AGAIN
//          (the explicit second call the post_ golden fields make) —
//          result false, twin count 1 per trace-bearing net (fixpoint).
//          The in-read FIRST call is not instrumentable from harness
//          side (no hook point outside the jar); the CA1 pin shows the
//          folding shape (combine folds a whole collinear chain inside
//          one iteration), which is the mechanics dsn-0151's first
//          call used.
//   CA7  — the ID CHANNEL (quality-round characterization): two nets,
//          each a CA5 crossing pair in a disjoint region. Split work
//          ALLOCATES generator ids, so group order changes the
//          id->geometry mapping (same live id set, different boards):
//          CA7_ORDER_DIFFER true, CA7_AGREE asc=true desc=false (the
//          driver, ascending like Java's small-Integer HashMap,
//          matches twin_asc only). Both twins' AFTER dumps are
//          captured; the port pins the ASCENDING rows literally.
//
// Output discipline: exact ints via field access, literal capture rows
// only, descending-id dumps, no HashMap-order leakage into any row the
// port pins (the only order-sensitive rows are the explicitly labeled
// twin asc/desc counts and CA7's id-mapping dumps, whose asc != desc
// IS the documented id-channel claim).
package app.freerouting.datastructures;

import app.freerouting.board.actions.ItemIdGenerator;
import app.freerouting.board.facade.BasicBoard;
import app.freerouting.board.model.items.ConductionArea;
import app.freerouting.board.model.items.Item;
import app.freerouting.board.model.items.ObstacleArea;
import app.freerouting.board.model.items.Pin;
import app.freerouting.board.model.items.Via;
import app.freerouting.board.model.structure.BoardOutline;
import app.freerouting.board.model.structure.FixedState;
import app.freerouting.board.trace.PolylineTrace;
import app.freerouting.geometry.planar.IntPoint;
import app.freerouting.geometry.planar.Point;
import app.freerouting.geometry.planar.Polyline;
import app.freerouting.io.BoardReadResult;
import app.freerouting.io.specctra.DsnReader;
import java.io.ByteArrayInputStream;
import java.nio.file.Files;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;

public final class NormalizeAllSpike {

  private NormalizeAllSpike() {}

  // ---------------------------------------------------------------------
  // helpers (mirrored from SplitSpike)
  // ---------------------------------------------------------------------

  static String kind(Item item) {
    if (item instanceof PolylineTrace) {
      return "T";
    }
    if (item instanceof Pin) {
      return "P";
    }
    if (item instanceof Via) {
      return "V";
    }
    if (item instanceof ConductionArea) {
      return "A";
    }
    if (item instanceof BoardOutline) {
      return "BO";
    }
    if (item instanceof ObstacleArea) {
      return "K";
    }
    return "?";
  }

  static String xy(Point p) {
    if (p instanceof IntPoint ip) {
      return ip.x + "," + ip.y;
    }
    return p.toString();
  }

  static IntPoint ip(int x, int y) {
    return new IntPoint(x, y);
  }

  static String cornersOf(PolylineTrace trace) {
    Point[] cs = trace.polyline().corners();
    StringBuilder sb = new StringBuilder("[");
    for (int i = 0; i < cs.length; i++) {
      if (i > 0) {
        sb.append(" ");
      }
      sb.append(xy(cs[i]));
    }
    return sb.append("]").toString();
  }

  static void dumpItems(BasicBoard board, String prefix) {
    List<Item> list = new ArrayList<>(board.getItems());
    list.sort((a, b) -> b.getId() - a.getId());
    for (Item item : list) {
      String geometry;
      if (item instanceof PolylineTrace trace) {
        geometry = "corners=" + cornersOf(trace) + " nets="
            + java.util.Arrays.toString(trace.netNumbers) + " fixed=" + trace.getFixedState();
      } else {
        geometry = "-";
      }
      System.out.println(prefix + "_ITEM id=" + item.getId()
          + " kind=" + kind(item) + " " + geometry);
    }
  }

  private static BasicBoard parseDsnBytes(byte[] bytes, String name) {
    IdGenerator idGenerator = new ItemIdGenerator();
    BoardReadResult read =
        DsnReader.readBoard(new ByteArrayInputStream(bytes), null, idGenerator, name);
    if (!(read instanceof BoardReadResult.Success success)) {
      System.out.println("READ_FAILED " + name + " " + read);
      return null;
    }
    return success.board();
  }

  /** The id-4 parse trace as the insert template; falls back to the
   * highest-id trace on the board (dsn-0151 has no id 4 — its only
   * trace is the combine survivor 4001). */
  static PolylineTrace tmpl(BasicBoard board) {
    PolylineTrace fallback = null;
    for (Item item : board.getItems()) {
      if (item instanceof PolylineTrace trace) {
        if (trace.getId() == 4) {
          return trace;
        }
        if (fallback == null || trace.getId() > fallback.getId()) {
          fallback = trace;
        }
      }
    }
    return fallback;
  }

  static PolylineTrace ins(BasicBoard board, PolylineTrace tmpl, IntPoint a, IntPoint b) {
    return board.insertTraceWithoutCleaning(new Polyline(new Point[] {a, b}), tmpl.getLayer(),
        tmpl.getHalfWidth(), tmpl.netNumbers, tmpl.clearanceClassIndex(), FixedState.UNFIXED);
  }

  static PolylineTrace insPoly(BasicBoard board, PolylineTrace tmpl, Point[] corners) {
    return board.insertTraceWithoutCleaning(new Polyline(corners), tmpl.getLayer(),
        tmpl.getHalfWidth(), tmpl.netNumbers, tmpl.clearanceClassIndex(), FixedState.UNFIXED);
  }

  static PolylineTrace insFixed(BasicBoard board, PolylineTrace tmpl, IntPoint a, IntPoint b) {
    return board.insertTraceWithoutCleaning(new Polyline(new Point[] {a, b}), tmpl.getLayer(),
        tmpl.getHalfWidth(), tmpl.netNumbers, tmpl.clearanceClassIndex(), FixedState.USER_FIXED);
  }

  static PolylineTrace insNet(BasicBoard board, PolylineTrace tmpl, IntPoint a, IntPoint b,
      int[] nets) {
    return board.insertTraceWithoutCleaning(new Polyline(new Point[] {a, b}), tmpl.getLayer(),
        tmpl.getHalfWidth(), nets, tmpl.clearanceClassIndex(), FixedState.UNFIXED);
  }

  static void witness(BasicBoard board, PolylineTrace t, String prefix) {
    PolylineTrace w = ins(board, t, ip(60000, 35000), ip(65000, 35000));
    System.out.println(prefix + "_NEXT_ID " + w.getId());
  }

  /** id:kind:corners over all items, DESCENDING id — the twin-vs-driver
   * agreement signature (state, not order, is the claim). */
  static String signature(BasicBoard board) {
    List<Item> list = new ArrayList<>(board.getItems());
    list.sort((a, b) -> b.getId() - a.getId());
    StringBuilder sb = new StringBuilder();
    for (Item item : list) {
      sb.append(item.getId()).append(":").append(kind(item));
      if (item instanceof PolylineTrace trace) {
        sb.append(":").append(cornersOf(trace));
      }
      sb.append(";");
    }
    return sb.toString();
  }

  // ---------------------------------------------------------------------
  // the instrumented twin of BasicBoard.normalizeAllTraces
  // (BasicBoard.java:799-885) — identical walks and gates, calling the
  // REAL per-trace seams; only the loop gained counters and the
  // group-order parameter. The CME-retry catch blocks of the original
  // (:808-813, :868-872) are structurally unreachable and omitted.
  // ---------------------------------------------------------------------

  static boolean twin(BasicBoard board, boolean descendingKeys, Map<Integer, Integer> counts) {
    boolean result = false;
    java.util.Map<Integer, List<PolylineTrace>> tracesByNet = new java.util.HashMap<>();
    var it = board.itemList.startReadObject();
    for (; ; ) {
      Item currentItem = (Item) board.itemList.readObject(it);
      if (currentItem == null) {
        break;
      }
      if (currentItem instanceof PolylineTrace currentTrace && currentItem.isOnTheBoard()) {
        for (int netNumber : currentTrace.netNumbers) {
          tracesByNet
              .computeIfAbsent(netNumber, k -> new ArrayList<>())
              .add(currentTrace);
        }
      }
    }
    List<Integer> keys = new ArrayList<>(tracesByNet.keySet());
    keys.sort(Integer::compareTo);
    if (descendingKeys) {
      Collections.reverse(keys);
    }
    for (int netNumber : keys) {
      List<PolylineTrace> netTraces = tracesByNet.get(netNumber);
      boolean somethingChanged = true;
      int iterationCount = 0;
      while (somethingChanged) {
        ++iterationCount;
        if (iterationCount > 2000) {
          // the MAX_NORMALIZE_ITERATIONS cap — unreachable in spikes
          break;
        }
        somethingChanged = false;
        for (PolylineTrace currentTrace : netTraces) {
          if (currentTrace.isOnTheBoard()) {
            if (currentTrace.normalize(null)) {
              somethingChanged = true;
              result = true;
            } else if (!currentTrace.isUserFixed() && board.removeIfCycle(currentTrace)) {
              somethingChanged = true;
              result = true;
            }
          }
        }
        // If something changed, collect the traces for this net again
        // (THIS net only, descending walk — :860-882).
        if (somethingChanged) {
          netTraces.clear();
          var it2 = board.itemList.startReadObject();
          for (; ; ) {
            Item currentItem = (Item) board.itemList.readObject(it2);
            if (currentItem == null) {
              break;
            }
            if (currentItem.containsNet(netNumber)
                && currentItem instanceof PolylineTrace currentTrace
                && currentItem.isOnTheBoard()) {
              netTraces.add(currentTrace);
            }
          }
        }
      }
      counts.put(netNumber, iterationCount);
    }
    return result;
  }

  interface BoardBuilder {
    BasicBoard build();
  }

  /** Builds the case board, dumps BEFORE, runs both twin orders and the
   * REAL driver on three fresh copies, prints counts + results + the
   * agreement rows + AFTER + NEXT_ID. */
  static void runCase(String tag, BoardBuilder build) {
    BasicBoard bAsc = build.build();
    Map<Integer, Integer> ascCounts = new TreeMap<>();
    boolean ascResult = twin(bAsc, false, ascCounts);
    String ascSig = signature(bAsc);

    BasicBoard bDesc = build.build();
    Map<Integer, Integer> descCounts = new TreeMap<>();
    boolean descResult = twin(bDesc, true, descCounts);
    String descSig = signature(bDesc);

    BasicBoard bDrv = build.build();
    dumpItems(bDrv, tag + "_BEFORE");
    boolean drvResult = bDrv.normalizeAllTraces();

    System.out.println(tag + "_TWIN_ASC result=" + ascResult + " counts=" + ascCounts);
    System.out.println(tag + "_TWIN_DESC result=" + descResult + " counts=" + descCounts);
    System.out.println(tag + "_DRIVER result=" + drvResult);
    String drvSig = signature(bDrv);
    System.out.println(tag + "_AGREE asc=" + ascSig.equals(drvSig)
        + " desc=" + descSig.equals(drvSig));
    dumpItems(bDrv, tag + "_AFTER");
    witness(bDrv, tmpl(bDrv), tag);
  }

  // ---------------------------------------------------------------------
  // the board (ContactsSpike/CombineSpike/SplitSpike PURE_DSN, unchanged)
  // ---------------------------------------------------------------------

  static final String PURE_DSN =
      "(pcb t13-pure.dsn\n"
      + "  (parser\n"
      + "    (string_quote \")\n"
      + "    (space_in_quoted_tokens on)\n"
      + "  )\n"
      + "  (resolution um 1)\n"
      + "  (unit um)\n"
      + "  (structure\n"
      + "    (layer F.Cu (type signal))\n"
      + "    (layer B.Cu (type signal))\n"
      + "    (boundary (rect pcb 0 0 100000 60000))\n"
      + "    (keepout (rect F.Cu 70000 10000 90000 20000))\n"
      + "    (rule (width 250) (clearance 14))\n"
      + "  )\n"
      + "  (placement\n"
      + "    (component CMP1\n"
      + "      (place CMP1 20000 40000 front 0)\n"
      + "    )\n"
      + "  )\n"
      + "  (library\n"
      + "    (padstack PAD_C600\n"
      + "      (shape (circle F.Cu 600 0 0))\n"
      + "    )\n"
      + "    (image CMP1\n"
      + "      (pin PAD_C600 P1 0 0)\n"
      + "    )\n"
      + "  )\n"
      + "  (network\n"
      + "    (net MINE)\n"
      + "    (net OTHER (pins CMP1-P1))\n"
      + "  )\n"
      + "  (wiring\n"
      + "    (wire (path F.Cu 250  20000 40000  10000 40000) (net MINE))\n"
      + "    (via PAD_C600 40000 40000 (net MINE))\n"
      + "    (wire (rect F.Cu 50000 10000 60000 20000) (net MINE))\n"
      + "    (wire (path F.Cu 250  55000 15000  55000 25000) (net MINE))\n"
      + "    (wire (path F.Cu 250  50000 15000  50000 25000) (net MINE))\n"
      + "    (wire (path F.Cu 250  80000 15000  80000 30000) (net MINE))\n"
      + "  )\n"
      + ")\n";

  static BasicBoard fresh() {
    return parseDsnBytes(PURE_DSN.getBytes(), "t13-pure.dsn");
  }

  public static void main(String[] p_args) throws Exception {
    // CA1 — the 3-segment collinear chain, one net.
    runCase("CA1",
        () -> {
          BasicBoard board = fresh();
          PolylineTrace t = tmpl(board);
          ins(board, t, ip(10000, 45000), ip(20000, 45000));
          ins(board, t, ip(20000, 45000), ip(30000, 45000));
          ins(board, t, ip(30000, 45000), ip(40000, 45000));
          return board;
        });

    // CA2 — two nets, each a chain, plus the multi-net pair X,Y
    // (nets [1,2]): both groups fold it; whichever runs first does the
    // work, the other is a no-op over it.
    runCase("CA2",
        () -> {
          BasicBoard board = fresh();
          PolylineTrace t = tmpl(board);
          ins(board, t, ip(10000, 45000), ip(20000, 45000));
          ins(board, t, ip(20000, 45000), ip(30000, 45000));
          insNet(board, t, ip(60000, 45000), ip(70000, 45000), new int[] {1, 2});
          insNet(board, t, ip(70000, 45000), ip(80000, 45000), new int[] {1, 2});
          insNet(board, t, ip(60000, 30000), ip(70000, 30000), new int[] {2});
          insNet(board, t, ip(70000, 30000), ip(80000, 30000), new int[] {2});
          return board;
        });

    // CA3 — the cycle square (4 open traces, same net, UNFIXED).
    runCase("CA3",
        () -> {
          BasicBoard board = fresh();
          PolylineTrace t = tmpl(board);
          ins(board, t, ip(10000, 45000), ip(20000, 45000));
          ins(board, t, ip(20000, 45000), ip(20000, 55000));
          ins(board, t, ip(20000, 55000), ip(10000, 55000));
          ins(board, t, ip(10000, 55000), ip(10000, 45000));
          return board;
        });

    // CA3B — the TRUE else-branch case: a 2-trace out-and-back loop
    // (both endpoints shared, non-collinear at the junctions — combine
    // cannot pre-merge it, unlike the CA3 square), placed CLEAR of every
    // parse trace (the first draft sat above parse trace 4; the loop's
    // middle corners touched its interior, the found-first split of 4
    // fired, and the split-internal cycle pass unwound the ring instead
    // — see the debug-run note in the task log). normalize false on the
    // first walked edge -> !isUserFixed() && removeIfCycle fires; BOTH
    // traces of the loop go in one removeItems (Trace.isCycle,
    // Trace.java:272: start-contact expansion reaches this trace again
    // via an end contact).
    runCase("CA3B",
        () -> {
          BasicBoard board = fresh();
          PolylineTrace t = tmpl(board);
          insPoly(board, t, new Point[] {ip(60000, 45000), ip(65000, 50000), ip(70000, 45000)});
          insPoly(board, t, new Point[] {ip(60000, 45000), ip(65000, 40000), ip(70000, 45000)});
          return board;
        });

    // CA3C — the REAL else-branch firing: a trace whose BOTH endpoints
    // contact the same-net conduction area 6 (50000-60000 x
    // 10000-20000). Combine cannot merge through an area, split has no
    // trace line to split by -> normalize false -> removeIfCycle:
    // isCycle (Trace.java:272) expands the start contact (the area) and
    // reaches the trace again via its end contact -> the trace goes in
    // one removeItems; the AREA survives (the T12 AR1 pin).
    runCase("CA3C",
        () -> {
          BasicBoard board = fresh();
          PolylineTrace t = tmpl(board);
          ins(board, t, ip(54000, 15000), ip(58000, 15000));
          return board;
        });

    // CA4 — the same square USER_FIXED ((type protect) state): the
    // driver's !isUserFixed() gate skips removeIfCycle; nothing changes.
    runCase("CA4",
        () -> {
          BasicBoard board = fresh();
          PolylineTrace t = tmpl(board);
          insFixed(board, t, ip(10000, 45000), ip(20000, 45000));
          insFixed(board, t, ip(20000, 45000), ip(20000, 55000));
          insFixed(board, t, ip(20000, 55000), ip(10000, 55000));
          insFixed(board, t, ip(10000, 55000), ip(10000, 45000));
          return board;
        });

    // CA5 — the crossing pair (same net): normalize(null) splits BOTH,
    // then the combines churn ids.
    runCase("CA5",
        () -> {
          BasicBoard board = fresh();
          PolylineTrace t = tmpl(board);
          ins(board, t, ip(25000, 20000), ip(25000, 35000));
          ins(board, t, ip(20000, 30000), ip(30000, 30000));
          return board;
        });

    // CA7 — THE ID CHANNEL (quality-round characterization): two nets,
    // each a CA5-shaped crossing pair in a disjoint region (net 1 at
    // x=25000, net 2 at x=65000 — both clear of the parse items and of
    // each other). The split work ALLOCATES generator ids, so the
    // across-net group order is OBSERVABLE in the id->geometry mapping
    // (not in the live id set): ascending processes net 1 first and its
    // pieces take the low ids; descending swaps them. The end boards
    // therefore DIFFER across orders (CA7_ORDER_DIFFER true) while each
    // order is internally consistent and the driver (ascending, like
    // Java's small-Integer HashMap) agrees with twin_asc only
    // (CA7_AGREE asc=true desc=false). The geometry/contact channel
    // stays order-free — this case is the honest characterization the
    // normalize_all module doc cites, NOT an isolation failure.
    BoardBuilder buildC7 = () -> {
      BasicBoard board = fresh();
      PolylineTrace t = tmpl(board);
      insNet(board, t, ip(25000, 20000), ip(25000, 35000), new int[] {1});
      insNet(board, t, ip(20000, 30000), ip(30000, 30000), new int[] {1});
      insNet(board, t, ip(65000, 20000), ip(65000, 35000), new int[] {2});
      insNet(board, t, ip(60000, 30000), ip(70000, 30000), new int[] {2});
      return board;
    };
    BasicBoard c7Asc = buildC7.build();
    Map<Integer, Integer> c7AscCounts = new TreeMap<>();
    boolean c7AscResult = twin(c7Asc, false, c7AscCounts);
    String c7AscSig = signature(c7Asc);
    BasicBoard c7Desc = buildC7.build();
    Map<Integer, Integer> c7DescCounts = new TreeMap<>();
    boolean c7DescResult = twin(c7Desc, true, c7DescCounts);
    String c7DescSig = signature(c7Desc);
    BasicBoard c7Drv = buildC7.build();
    dumpItems(c7Drv, "CA7_BEFORE");
    boolean c7DrvResult = c7Drv.normalizeAllTraces();
    System.out.println("CA7_TWIN_ASC result=" + c7AscResult + " counts=" + c7AscCounts);
    System.out.println("CA7_TWIN_DESC result=" + c7DescResult + " counts=" + c7DescCounts);
    System.out.println("CA7_DRIVER result=" + c7DrvResult);
    String c7DrvSig = signature(c7Drv);
    System.out.println("CA7_AGREE asc=" + c7AscSig.equals(c7DrvSig)
        + " desc=" + c7DescSig.equals(c7DrvSig));
    System.out.println("CA7_ORDER_DIFFER " + !c7AscSig.equals(c7DescSig));
    dumpItems(c7Asc, "CA7_ASC_AFTER");
    dumpItems(c7Desc, "CA7_DESC_AFTER");
    witness(c7Drv, tmpl(c7Drv), "CA7");

    // CA6 — dsn-0151 itself (the real fixture). The READ already ran
    // normalizeAllTraces inside the wiring scope; the captured rows are
    // the POST state; the explicit call below is the SECOND call (the
    // one the post_ golden fields make) — a fixpoint by construction.
    BasicBoard board =
        parseDsnBytes(
            Files.readAllBytes(Paths.get("fixtures/Issue723-CombineStackOverflow.dsn")),
            "dsn-0151");
    int pads = 0;
    int traces = 0;
    int vias = 0;
    for (Item item : board.getItems()) {
      if (item instanceof Pin) {
        pads++;
      } else if (item instanceof app.freerouting.board.model.items.Trace) {
        traces++;
      } else if (item instanceof Via) {
        vias++;
      }
    }
    System.out.println("CA6_STATS items=" + board.getItems().size()
        + " pads=" + pads + " traces=" + traces + " vias=" + vias
        + " nets=" + board.rules.nets.maxNetNumber());
    dumpItems(board, "CA6_TRACE");
    boolean secondResult = board.normalizeAllTraces();
    System.out.println("CA6_DRIVER result=" + secondResult);
    BasicBoard board2 =
        parseDsnBytes(
            Files.readAllBytes(Paths.get("fixtures/Issue723-CombineStackOverflow.dsn")),
            "dsn-0151-2nd");
    boolean dummySecond = board2.normalizeAllTraces(); // the same second call
    Map<Integer, Integer> twinCounts = new TreeMap<>();
    boolean twinResult = twin(board2, false, twinCounts);
    System.out.println("CA6_TWIN second=" + dummySecond + " twin=" + twinResult
        + " counts=" + twinCounts);
    System.out.println("CA6_AGREE " + signature(board).equals(signature(board2)));
    witness(board2, tmpl(board2), "CA6");
  }
}
