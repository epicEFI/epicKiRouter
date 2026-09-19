// ShapeTraceDebtProbe.java — the M3-T10b jar-side probe for the T10a
// BANKED CAPTURE DEBTS (SEAM.md T10a rows, "coverage gaps banked for
// T10b") plus the T9 via-worlds debt (real `ForcedViaInserter.checkLayer`
// verdicts for the four de-scoped locator pins). New file (not an edit of
// ShapeTraceEntriesProbe) so T10a's committed captures stay frozen.
//
// Build/run (JDK 25, from the repo root):
//   mkdir -p /tmp/epic-t10b-classes && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/javac \
//       -cp build/libs/freerouting-current-executable.jar \
//       -d /tmp/epic-t10b-classes rust/harness/oracle/ShapeTraceDebtProbe.java && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/java \
//       -cp build/libs/freerouting-current-executable.jar:/tmp/epic-t10b-classes \
//       app.freerouting.board.searchtree.ShapeTraceDebtProbe \
//       rust/harness/fixtures/locator-spike/t9_locator45.dsn
//
// IMPORTANT placement fact (learned from the first capture run): the
// fixture's own ObstacleAreas (items 2-4) cover the LOWERCY half of the
// board (every world with y < cy collected item 4 and died in the store
// ladder before the arm under test). ALL worlds live at y > cy in
// area-free bands, well separated.
//
// Worlds (all read-only: storeItems mutates nothing; every board
// mutation happens before the first run):
//   w1_dedup   — two SAME-net foreign traces fully crossing the shape:
//                four consecutive same-net chain entries -> the resort
//                triple-dedup removes the two middles (2 entries left).
//   w2_stack   — two DIFFERENT-net foreign traces crossing in an X: the
//                edge-sorted chain interleaves the two net sets ->
//                calculateStackLevels violates the close-at-open-level
//                property -> storeItems false.
//   w3a/w3b    — from-side borderIntersection worlds: w3a projects
//                PAST the side end (>= side length -> the reset arm
//                swaps the side to (no, null) and the anchor walk uses
//                the compareCorner2 branch); w3b projects mid-side (no
//                reset, intersection-projection branch). Two vertical
//                stubs put entries on the from side (edge 0) so the
//                anchor walk has candidates; the rotated chains differ.
//   w4_headtrim— two own-net traces crossing BEFORE the foreign trace
//                (lower ids): the head-trim (Item.netsEqual, TWO-STEP
//                remove) drops both own-net head entries while their
//                tail-side entries remain mid-chain.
//   w5_proj    — stub traces of DIFFERENT nets (no dedup interference)
//                whose in-shape end has EXACTLY ONE contact (a same-net
//                via at the end corner): the contactCount == 1
//                projection-entry arm inserts entries with
//                traceLineNo == lines.length - 1 (end stub) and
//                traceLineNo == 0 (start stub). Via radius == trace half
//                width reaches the viaTraceDiff == 0 arm and passes
//                (corner containsInside).
//   w6_diffneg — via radius (100) < trace half width (200) at the end
//                corner: viaTraceDiff < 0 -> storeTrace false with
//                foundObstacle == the via.
//   w7_diffeq  — via radius == trace half width and the end corner
//                EXACTLY on the offset-shape boundary: contains holds,
//                containsInside does not -> storeEndCorner = false -> NO
//                projection entry in the chain (contrast w5_proj).
//   w8_eqop    — the stored stub (class 0) contacts an UNFIXED trace of
//                clearance class 1 with the SAME half width: Java's
//                tautological `contactItem.clearanceClassIndex() !=
//                contactTrace.clearanceClassIndex()` (both names are the
//                SAME object) stays false -> no fail; the fix-mutant
//                (compare against the STORED trace's class) would fail
//                with foundObstacle == the class-1 contact.
//   v_*        — the T9 debt: real ForcedViaInserter.checkLayer verdicts
//                (zero radius; attach-smd pin worlds on/off per pin;
//                start-trace check vs via check separated by a
//                SHOVE_FIXED trace, which the shove cannot move; the
//                room-simplex exclusion; the ninety-degree tile branch).
//
// Determinism: same discipline as ShapeTraceEntriesProbe — one JVM per
// run, TreeSet obstacle lists (item id order), reflection chain walk in
// list order, Double.toString for every float. No identity hashes reach
// a row (no shape toString is emitted).
package app.freerouting.board.searchtree;

import app.freerouting.board.actions.ForcedPadRouter.CheckDrillResult;
import app.freerouting.board.actions.ForcedViaInserter;
import app.freerouting.board.actions.ItemIdGenerator;
import app.freerouting.board.facade.BasicBoard;
import app.freerouting.board.facade.RoutingBoard;
import app.freerouting.board.model.items.DrillItem;
import app.freerouting.board.model.items.Item;
import app.freerouting.board.model.items.Pin;
import app.freerouting.board.model.items.Via;
import app.freerouting.board.model.structure.AngleRestriction;
import app.freerouting.board.model.structure.FixedState;
import app.freerouting.board.model.structure.ShapeEntrySide;
import app.freerouting.board.trace.PolylineTrace;
import app.freerouting.core.library.Padstack;
import app.freerouting.geometry.planar.FloatPoint;
import app.freerouting.geometry.planar.IntBox;
import app.freerouting.geometry.planar.IntPoint;
import app.freerouting.geometry.planar.Polyline;
import app.freerouting.geometry.planar.TileShape;
import app.freerouting.io.BoardReadResult;
import app.freerouting.io.specctra.DsnReader;
import com.google.gson.Gson;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import java.io.ByteArrayInputStream;
import java.lang.reflect.Field;
import java.nio.file.Files;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import java.util.TreeSet;

public class ShapeTraceDebtProbe {

  private static final Gson GSON = new Gson();

  private static final Field LIST_ANCHOR_FIELD;
  private static final Field EP_NEXT_FIELD;
  private static final Field EP_TRACE_FIELD;
  private static final Field EP_TRACE_LINE_NO_FIELD;
  private static final Field EP_ENTRY_APPROX_FIELD;
  private static final Field EP_EDGE_INDEX_FIELD;
  private static final Field EP_STACK_LEVEL_FIELD;

  static {
    try {
      LIST_ANCHOR_FIELD = ShapeTraceEntries.class.getDeclaredField("listAnchor");
      LIST_ANCHOR_FIELD.setAccessible(true);
      Class<?> epClass = Class.forName("app.freerouting.board.searchtree.ShapeTraceEntries$EntryPoint");
      EP_NEXT_FIELD = epClass.getDeclaredField("next");
      EP_NEXT_FIELD.setAccessible(true);
      EP_TRACE_FIELD = epClass.getDeclaredField("trace");
      EP_TRACE_FIELD.setAccessible(true);
      EP_TRACE_LINE_NO_FIELD = epClass.getDeclaredField("traceLineNo");
      EP_TRACE_LINE_NO_FIELD.setAccessible(true);
      EP_ENTRY_APPROX_FIELD = epClass.getDeclaredField("entryApprox");
      EP_ENTRY_APPROX_FIELD.setAccessible(true);
      EP_EDGE_INDEX_FIELD = epClass.getDeclaredField("edgeIndex");
      EP_EDGE_INDEX_FIELD.setAccessible(true);
      EP_STACK_LEVEL_FIELD = epClass.getDeclaredField("stackLevel");
      EP_STACK_LEVEL_FIELD.setAccessible(true);
    } catch (Exception e) {
      throw new ExceptionInInitializerError(e);
    }
  }

  private static void row(JsonObject obj) {
    System.out.println(GSON.toJson(obj));
  }

  private static void addPoint(JsonArray arr, FloatPoint p) {
    JsonArray pt = new JsonArray();
    pt.add(Double.toString(p.x));
    pt.add(Double.toString(p.y));
    arr.add(pt);
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

  private static String shapeNameOf(TileShape shape) {
    IntBox bb = shape.boundingBox();
    return bb.ll.x + ":" + bb.ll.y + ":" + bb.ur.x + ":" + bb.ur.y;
  }

  private static IntBox box(int x1, int y1, int x2, int y2) {
    return new IntBox(new IntPoint(x1, y1), new IntPoint(x2, y2));
  }

  /** Dumps the entry chain (listAnchor .. end) via reflection. */
  private static void dumpChain(String run, ShapeTraceEntries entries) throws Exception {
    Object anchor = LIST_ANCHOR_FIELD.get(entries);
    JsonObject chain = new JsonObject();
    chain.addProperty("type", "chain");
    chain.addProperty("run", run);
    JsonArray list = new JsonArray();
    int order = 0;
    while (anchor != null) {
      JsonObject ep = new JsonObject();
      ep.addProperty("order", order);
      PolylineTrace trace = (PolylineTrace) EP_TRACE_FIELD.get(anchor);
      ep.addProperty("traceId", trace.getId());
      ep.addProperty("lineNo", (Integer) EP_TRACE_LINE_NO_FIELD.get(anchor));
      FloatPoint approx = (FloatPoint) EP_ENTRY_APPROX_FIELD.get(anchor);
      JsonArray approxArr = new JsonArray();
      approxArr.add(Double.toString(approx.x));
      approxArr.add(Double.toString(approx.y));
      ep.add("approx", approxArr);
      ep.addProperty("edge", (Integer) EP_EDGE_INDEX_FIELD.get(anchor));
      ep.addProperty("level", (Integer) EP_STACK_LEVEL_FIELD.get(anchor));
      list.add(ep);
      anchor = EP_NEXT_FIELD.get(anchor);
      order++;
    }
    chain.add("entries", list);
    row(chain);
  }

  /** Runs storeItems and dumps the observable surface. */
  private static void runStore(
      String run,
      RoutingBoard board,
      int ownNet,
      TileShape shape,
      ShapeEntrySide fromSide,
      int layer)
      throws Exception {
    TreeSet<Item> items = new TreeSet<>(Comparator.comparingInt(Item::getId));
    items.addAll(board.overlappingItems(shape.boundingBox(), layer));

    ShapeTraceEntries entries =
        new ShapeTraceEntries(shape, layer, new int[] {ownNet}, 0, fromSide, board);
    boolean result = entries.storeItems(items, false, false);

    JsonObject meta = new JsonObject();
    meta.addProperty("type", "run");
    meta.addProperty("run", run);
    meta.addProperty("shape", shapeNameOf(shape));
    meta.addProperty("layer", layer);
    meta.addProperty("ownNet", ownNet);
    meta.addProperty("result", result);
    JsonArray itemIds = new JsonArray();
    for (Item item : items) {
      itemIds.add(item.getId());
    }
    meta.add("items", itemIds);
    row(meta);

    dumpChain(run, entries);

    JsonObject counts = new JsonObject();
    counts.addProperty("type", "counts");
    counts.addProperty("run", run);
    counts.addProperty("pieceCount", entries.substituteTraceCount());
    counts.addProperty("maxStackLevel", entries.stackDepth());
    counts.addProperty("tailsInShape", entries.traceTailsInShape());
    Item obstacle = entries.getFoundObstacle();
    counts.addProperty("foundObstacle", obstacle == null ? -1 : obstacle.getId());
    row(counts);
  }

  private static void checkLayerRow(String run, CheckDrillResult result) {
    JsonObject o = new JsonObject();
    o.addProperty("type", "checklayer");
    o.addProperty("run", run);
    o.addProperty("result", result.name());
    row(o);
  }

  public static void main(String[] args) throws Exception {
    byte[] bytes = Files.readAllBytes(Paths.get(args[0]));
    String fileName = args.length > 1 ? args[1] : "t9_locator45.dsn";
    RoutingBoard board = parse(bytes, fileName);

    IntBox bbox = board.boundingBox;
    int cx = (bbox.ll.x + bbox.ur.x) / 2;
    int cy = (bbox.ll.y + bbox.ur.y) / 2;
    int ownNet = board.rules.nets.get("N001", 1).netNumber;
    int foreignNet = board.rules.nets.get("N002", 1).netNumber;
    int foreignNet2 = board.rules.nets.get("N003", 1).netNumber;
    int layerCount = board.layerStructure.layers.length;
    int layer = 0;
    int cl00 = board.clearanceValue(0, 0, layer);

    // The via padstack: +-100 (trace half width parity) through all
    // layers.
    Padstack via100 =
        board.library.padstacks.add(
            box(-100, -100, 100, 100), 0, layerCount - 1);

    JsonObject world = new JsonObject();
    world.addProperty("type", "world");
    world.addProperty("cx", cx);
    world.addProperty("cy", cy);
    world.addProperty("ownNet", ownNet);
    world.addProperty("foreignNet", foreignNet);
    world.addProperty("foreignNet2", foreignNet2);
    world.addProperty("layers", layerCount);
    world.addProperty("cl00", cl00);
    // The fixture's keepout geometry — world bands are chosen between
    // these boxes (the first capture run died on them).
    JsonArray areas = new JsonArray();
    List<Item> sortedPre = new ArrayList<>(board.getItems());
    sortedPre.sort(Comparator.comparingInt(Item::getId));
    for (Item item : sortedPre) {
      if (item instanceof app.freerouting.board.model.items.ObstacleArea area) {
        IntBox bb = area.getArea().getBorder().boundingBox();
        areas.add(bb.ll.x + ":" + bb.ll.y + ":" + bb.ur.x + ":" + bb.ur.y);
      }
    }
    world.add("areas", areas);
    row(world);

    // LAYOUT (verified against the areas row): the area-free window is
    // y in (296000, 320000), x in (480000, 520000). The w-worlds stack
    // in Y up to w5, the rest pack side-by-side in X inside the top
    // sliver y in [318400, 319900].

    // ---- w1_dedup: two same-net traces fully crossing the shape ----
    board.insertTraceWithoutCleaning(
        new Polyline(
            new IntPoint(cx - 2000, cy + 2000), new IntPoint(cx + 2000, cy + 2000)),
        0,
        100,
        new int[] {foreignNet},
        0,
        FixedState.UNFIXED);
    board.insertTraceWithoutCleaning(
        new Polyline(
            new IntPoint(cx - 2000, cy + 3000), new IntPoint(cx + 2000, cy + 3000)),
        0,
        100,
        new int[] {foreignNet},
        0,
        FixedState.UNFIXED);
    runStore(
        "w1_dedup",
        board,
        ownNet,
        box(cx - 700, cy + 1300, cx + 700, cy + 3700),
        new ShapeEntrySide(0, null),
        layer);

    // ---- w2_stack: two different-net traces crossing in an X ----
    board.insertTraceWithoutCleaning(
        new Polyline(
            new IntPoint(cx - 2000, cy + 6000), new IntPoint(cx + 2000, cy + 6000)),
        0,
        100,
        new int[] {foreignNet},
        0,
        FixedState.UNFIXED);
    board.insertTraceWithoutCleaning(
        new Polyline(
            new IntPoint(cx, cy + 4000), new IntPoint(cx, cy + 8000)),
        0,
        100,
        new int[] {foreignNet2},
        0,
        FixedState.UNFIXED);
    runStore(
        "w2_stack",
        board,
        ownNet,
        box(cx - 700, cy + 5300, cx + 700, cy + 6700),
        new ShapeEntrySide(0, null),
        layer);

    // ---- w3a/w3b: resort anchor walk, reset arm vs mid-side control --
    // Two vertical N002 stubs CROSS the shape (ends outside the offset
    // shape — a fully-inside polyline has NO entrance points), and the
    // horizontal trace is N003: three distinct chain nets defeat the
    // triple-dedup so the FULL pre-resort chain stays visible.
    board.insertTraceWithoutCleaning(
        new Polyline(
            new IntPoint(cx - 2000, cy + 10000), new IntPoint(cx + 2000, cy + 10000)),
        0,
        100,
        new int[] {foreignNet2},
        0,
        FixedState.UNFIXED);
    board.insertTraceWithoutCleaning(
        new Polyline(
            new IntPoint(cx - 500, cy + 9000), new IntPoint(cx - 500, cy + 11000)),
        0,
        100,
        new int[] {foreignNet},
        0,
        FixedState.UNFIXED);
    board.insertTraceWithoutCleaning(
        new Polyline(
            new IntPoint(cx + 400, cy + 9000), new IntPoint(cx + 400, cy + 11000)),
        0,
        100,
        new int[] {foreignNet},
        0,
        FixedState.UNFIXED);
    IntBox w3Shape = box(cx - 700, cy + 9300, cx + 700, cy + 10700);
    // w3b: borderIntersection ON the side line, projection mid-side
    // (fromPointDist 100 < side length 1400 -> NO reset); the
    // intersection-branch walk breaks at the first bottom-side entry
    // (the listAnchor itself -> NO rotation).
    runStore(
        "w3b_mid",
        board,
        ownNet,
        w3Shape,
        new ShapeEntrySide(0, new FloatPoint(cx - 600.0, cy + 9300.0)),
        layer);
    // w3a: borderIntersection PAST the side end (projection 3200 from
    // corner1 >= side length 1400 -> the RESET arm swaps the side to
    // (0, null)); the compareCorner2 walk breaks at the second
    // bottom-side entry -> the chain rotates there.
    runStore(
        "w3a_reset",
        board,
        ownNet,
        w3Shape,
        new ShapeEntrySide(0, new FloatPoint(cx + 2500.0, cy + 9300.0)),
        layer);

    // ---- w4_headtrim: own, own, foreign — the TWO-STEP trims --------
    // Chain (edge order, left edge top->bottom):
    // [own1_r, own2_r, f_r, f_l, own2_l, own1_l]. The tail trim removes
    // two consecutive own-net tail nodes; the head trim removes two
    // consecutive own-net head nodes (netsEqual on the Item face).
    board.insertTraceWithoutCleaning(
        new Polyline(
            new IntPoint(cx - 2000, cy + 12000), new IntPoint(cx + 2000, cy + 12000)),
        0,
        100,
        new int[] {ownNet},
        0,
        FixedState.UNFIXED);
    board.insertTraceWithoutCleaning(
        new Polyline(
            new IntPoint(cx - 2000, cy + 13000), new IntPoint(cx + 2000, cy + 13000)),
        0,
        100,
        new int[] {ownNet},
        0,
        FixedState.UNFIXED);
    board.insertTraceWithoutCleaning(
        new Polyline(
            new IntPoint(cx - 2000, cy + 14000), new IntPoint(cx + 2000, cy + 14000)),
        0,
        100,
        new int[] {foreignNet},
        0,
        FixedState.UNFIXED);
    runStore(
        "w4_headtrim",
        board,
        ownNet,
        box(cx - 700, cy + 11300, cx + 700, cy + 14700),
        new ShapeEntrySide(0, null),
        layer);

    // ---- w4b_interior: own, foreign, own — interior SURVIVAL --------
    // Chain [own1_r, f_r, own2_r, own2_l, f_l, own1_l]: only the outer
    // own-net nodes are trimmed (head step 1, tail step 1); the own2
    // pair SURVIVES mid-chain — the trim is ends-only.
    board.insertTraceWithoutCleaning(
        new Polyline(
            new IntPoint(503000, cy + 12000), new IntPoint(507000, cy + 12000)),
        0,
        100,
        new int[] {ownNet},
        0,
        FixedState.UNFIXED);
    board.insertTraceWithoutCleaning(
        new Polyline(
            new IntPoint(503000, cy + 13000), new IntPoint(507000, cy + 13000)),
        0,
        100,
        new int[] {foreignNet},
        0,
        FixedState.UNFIXED);
    board.insertTraceWithoutCleaning(
        new Polyline(
            new IntPoint(503000, cy + 14000), new IntPoint(507000, cy + 14000)),
        0,
        100,
        new int[] {ownNet},
        0,
        FixedState.UNFIXED);
    runStore(
        "w4b_interior",
        board,
        ownNet,
        box(504300, cy + 11300, 505700, cy + 14700),
        new ShapeEntrySide(0, null),
        layer);

    // ---- w5_proj: contactCount == 1 projection entries, diff == 0 ----
    board.insertTraceWithoutCleaning(
        new Polyline(
            new IntPoint(cx - 2000, cy + 16000), new IntPoint(cx, cy + 16000)),
        0,
        100,
        new int[] {foreignNet},
        0,
        FixedState.UNFIXED);
    board.insertVia(
        via100,
        new IntPoint(cx, cy + 16000),
        new int[] {foreignNet},
        0,
        FixedState.UNFIXED,
        false);
    board.insertTraceWithoutCleaning(
        new Polyline(
            new IntPoint(cx, cy + 18000), new IntPoint(cx + 2000, cy + 18000)),
        0,
        100,
        new int[] {foreignNet2}, // different net: no dedup interference
        0,
        FixedState.UNFIXED);
    board.insertVia(
        via100,
        new IntPoint(cx, cy + 18000),
        new int[] {foreignNet2},
        0,
        FixedState.UNFIXED,
        false);
    runStore(
        "w5_proj",
        board,
        ownNet,
        box(cx - 700, cy + 15300, cx + 700, cy + 18700),
        new ShapeEntrySide(0, null),
        layer);

    // ---- top sliver worlds: y in [318400, 319900] --------------------
    int w6x = cx - 13000;
    int w7x = cx;
    int w8x = cx + 13000;

    // w6_diffneg: via radius 100 < trace half width 200 at the end
    // corner -> viaTraceDiff < 0 -> storeTrace false, foundObstacle =
    // the via.
    board.insertTraceWithoutCleaning(
        new Polyline(
            new IntPoint(w6x - 2000, 319000), new IntPoint(w6x, 319000)),
        0,
        200,
        new int[] {foreignNet},
        0,
        FixedState.UNFIXED);
    board.insertVia(
        via100,
        new IntPoint(w6x, 319000),
        new int[] {foreignNet},
        0,
        FixedState.UNFIXED,
        false);
    runStore(
        "w6_diffneg",
        board,
        ownNet,
        box(w6x - 700, 318700, w6x + 300, 319300),
        new ShapeEntrySide(0, null),
        layer);

    // w7_diffeq: end corner EXACTLY on the offset-shape boundary
    // (offset = halfWidth + clearance + c_offset_add = 101 + cl00):
    // contains holds, containsInside does not -> storeEndCorner false
    // -> NO projection entry (contrast w5_proj).
    int eOff = 101 + cl00;
    int eX = w7x + 300 + eOff; // trace end exactly on the offset boundary
    board.insertTraceWithoutCleaning(
        new Polyline(
            new IntPoint(eX - 3000, 319200), new IntPoint(eX, 319200)),
        0,
        100,
        new int[] {foreignNet},
        0,
        FixedState.UNFIXED);
    board.insertVia(
        via100,
        new IntPoint(eX, 319200),
        new int[] {foreignNet},
        0,
        FixedState.UNFIXED,
        false);
    runStore(
        "w7_diffeq",
        board,
        ownNet,
        box(eX - 1400 - eOff, 318900, eX - eOff, 319500),
        new ShapeEntrySide(0, null),
        layer);

    // w8_eqop: stored stub (class 0) contacts an UNFIXED class-1 trace
    // of the same half width. Java's tautological
    // `contactItem.clearanceClassIndex() != contactTrace.
    // clearanceClassIndex()` (both names: the SAME contact object)
    // stays false -> no fail. The fix-mutant (compare the CONTACT's
    // class against the STORED trace's class 0) fails with
    // foundObstacle = the class-1 contact.
    board.insertTraceWithoutCleaning(
        new Polyline(
            new IntPoint(w8x - 2000, 319000), new IntPoint(w8x, 319000)),
        0,
        100,
        new int[] {foreignNet},
        0,
        FixedState.UNFIXED);
    board.insertTraceWithoutCleaning(
        new Polyline(
            new IntPoint(w8x, 319000), new IntPoint(w8x + 2000, 319000)),
        0,
        100,
        new int[] {foreignNet},
        1, // clearance class 1 — the fix-mutant discriminator
        FixedState.UNFIXED);
    runStore(
        "w8_eqop",
        board,
        ownNet,
        box(w8x - 700, 318300, w8x + 700, 319700),
        new ShapeEntrySide(0, null),
        layer);

    // ---- v_*: the T9 debt — real checkLayer verdicts -----------------

    // v1: zero radius short-circuits to DRILLABLE.
    checkLayerRow(
        "v1_zero_radius",
        ForcedViaInserter.checkLayer(
            0.0, 0, false, box(cx - 2000, 314800, cx + 2000, 318800),
            new IntPoint(cx, 316000), layer,
            new int[] {foreignNet}, 10, 0, board, 0, 0));

    // v2: EVERY pin under a same-net via — attach allowed vs forbidden.
    // One row per pin (the fixture's pin field decides DRILLABLE /
    // DRILLABLE_WITH_ATTACH_SMD / NOT_DRILLABLE per pin; the Rust pin
    // transcribes the rows verbatim).
    List<Item> pinSorted = new ArrayList<>(board.getItems());
    pinSorted.sort(Comparator.comparingInt(Item::getId));
    for (Item item : pinSorted) {
      if (item instanceof Pin pin && pin.netNumbers.length > 0) {
        FloatPoint pc = ((DrillItem) pin).getCenter().toFloat();
        IntPoint pinLoc = new IntPoint((int) Math.round(pc.x), (int) Math.round(pc.y));
        IntBox pinRoom =
            box(pinLoc.x - 2000, pinLoc.y - 2000, pinLoc.x + 2000, pinLoc.y + 2000);
        CheckDrillResult withAttach =
            ForcedViaInserter.checkLayer(
                100.0, 0, true, pinRoom, pinLoc, layer,
                pin.netNumbers, 10, 0, board, 0, 0);
        CheckDrillResult noAttach =
            ForcedViaInserter.checkLayer(
                100.0, 0, false, pinRoom, pinLoc, layer,
                pin.netNumbers, 10, 0, board, 0, 0);
        JsonObject v2 = new JsonObject();
        v2.addProperty("type", "checklayer_pin");
        v2.addProperty("run", "v2_pin_" + pin.getId());
        v2.addProperty("x", pinLoc.x);
        v2.addProperty("y", pinLoc.y);
        v2.addProperty("withAttach", withAttach.name());
        v2.addProperty("noAttach", noAttach.name());
        row(v2);
      }
    }

    // v3: the start-trace check (traceHalfWidth 400) fails where the
    // via-only check (traceHalfWidth 0) passes: a SHOVE_FIXED trace of
    // a DIFFERENT net sits 450 above the probe point (surface at 350;
    // the start-trace circle reaches 400, the via octagon ~116 incl.
    // clearance; the shove cannot move a SHOVE_FIXED trace). Same-net
    // would be copper-shared away (the run-3 lesson).
    IntPoint p3 = new IntPoint(492000, 316500);
    board.insertTraceWithoutCleaning(
        new Polyline(
            new IntPoint(489000, 316950), new IntPoint(495000, 316950)),
        0,
        100,
        new int[] {foreignNet2},
        0,
        FixedState.SHOVE_FIXED);
    IntBox p3Room = box(490000, 314800, 494000, 318800);
    checkLayerRow(
        "v3_start_trace",
        ForcedViaInserter.checkLayer(
            100.0, 0, true, p3Room, p3, layer,
            new int[] {foreignNet}, 10, 0, board, 400, 0));
    checkLayerRow(
        "v3_via_only",
        ForcedViaInserter.checkLayer(
            100.0, 0, true, p3Room, p3, layer,
            new int[] {foreignNet}, 10, 0, board, 0, 0));

    // v4: the room simplex excludes every from-side probe point ->
    // calculateFromSide answers null -> NOT_DRILLABLE (before any board
    // check; the probe location is far from the room box).
    checkLayerRow(
        "v4_room_excluded",
        ForcedViaInserter.checkLayer(
            100.0, 0, false,
            box(cx - 1000, cy - 1000, cx + 1000, cy + 1000),
            new IntPoint(cx + 6000, cy + 6000), layer,
            new int[] {foreignNet}, 10, 0, board, 0, 0));

    // v5: the ninety-degree branch (IntBox tiles, direct side
    // numbering). The restriction is flipped and RESTORED. A
    // SHOVE_FIXED trace of a DIFFERENT net 450 above the probe
    // separates the clean world (DRILLABLE) from the start-trace world
    // (NOT_DRILLABLE).
    board.insertTraceWithoutCleaning(
        new Polyline(
            new IntPoint(503200, 318350), new IntPoint(509200, 318350)),
        0,
        100,
        new int[] {foreignNet2},
        0,
        FixedState.SHOVE_FIXED);
    board.rules.setTraceAngleRestriction(AngleRestriction.NINETY_DEGREE);
    IntPoint p5 = new IntPoint(506200, 317900);
    IntBox p5Room = box(504200, 315900, 508200, 319900);
    checkLayerRow(
        "v5_ninety_clean",
        ForcedViaInserter.checkLayer(
            150.0, 0, true, p5Room, p5, layer,
            new int[] {foreignNet}, 10, 0, board, 0, 0));
    checkLayerRow(
        "v5_ninety_trace",
        ForcedViaInserter.checkLayer(
            150.0, 0, true, p5Room, p5, layer,
            new int[] {foreignNet}, 10, 0, board, 400, 0));
    board.rules.setTraceAngleRestriction(AngleRestriction.FORTYFIVE_DEGREE);

    // Final inventory row: every inserted id (deterministic order).
    JsonObject inv = new JsonObject();
    inv.addProperty("type", "inserted");
    JsonArray ids = new JsonArray();
    List<Item> sorted = new ArrayList<>(board.getItems());
    sorted.sort(Comparator.comparingInt(Item::getId));
    for (Item item : sorted) {
      if (item.getId() > 104) {
        ids.add(item.getId());
      }
    }
    inv.add("ids", ids);
    row(inv);
  }
}
