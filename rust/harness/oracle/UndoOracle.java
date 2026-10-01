// UndoOracle.java — the jar-side oracle for the M2 Task 14 undo parity
// corpus (rust/harness/src/undo_corpus.rs is the doc-of-record for the
// protocol; THIS file mirrors it exactly).
//
// Lives OUTSIDE src/ (harness-side; the frozen Java tree is never
// touched). NO package declaration: it calls
// `DsnParseOracle.canonicalGeometryText` (same default package) and the
// PUBLIC `BasicBoard` fields (`itemList`, `components`,
// `searchTreeManager`, `communication` — BasicBoard.java:70-94). The
// two private `stackLevel` fields are read reflectively (classpath =
// unnamed module, setAccessible is fine).
//
// Build/run (JDK 25, from the repo root; the harness's `undo golden`
// does exactly this):
//   mkdir -p /tmp/epic-undo-classes && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/javac \
//       -cp build/libs/freerouting-current-executable.jar \
//       -d /tmp/epic-undo-classes \
//       rust/harness/oracle/DsnParseOracle.java rust/harness/oracle/UndoOracle.java && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/java \
//       -Duser.language=en -Duser.country=US \
//       -cp build/libs/freerouting-current-executable.jar:/tmp/epic-undo-classes \
//       UndoOracle <manifest.jsonl>
//
// Input: one {"id":"und-NNNN","path":"repo/relative.dsn"} per line.
// Output: ONE JSONL result line per fixture (stdout lines starting
// with {"id" are the results; FRLogger noise is interleaved and
// dropped by the reader). An evaluate-time throwable emits a full-null
// "EvalError" record and the batch CONTINUES (DsnParseOracle
// discipline).
//
// The protocol per fixture (both sides identical — see the Rust module
// docs):
//   parse (the standard read path — its in-scope normalizeAllTraces
//   tail already ran) → derive the script from the board's OWN items
//   (descending walk: t1 = first PolylineTrace, X = the next
//   PolylineTrace that is not deletion-forbidden) → baseline digest →
//   the step script (snap / insert_trace (t1's first segment duplicate)
//   / remove_item(X) / normalize_all / query / undo / redo /
//   pop_snapshot) with a full state digest after EVERY step.
//
// Undo/redo capture — the A/B discipline: EVERY fixture is parsed and
// replayed TWICE on independent boards.
//   Board A drives the REAL facade (`board.undo(changedNets)`) and
//   yields the primary digest (what the port must match).
//   Board B drives the container directly
//   (`components.undo(null)` + `itemList.undo(cancelled, restored)` +
//   a verbatim copy of BasicBoard.applyUndoRedoSideEffects) to EXPOSE
//   the cancelled/restored collections, which the facade does not
//   return. At every undo/redo step the two boards' return values,
//   changed-nets sets, item counts, stack levels, geometry and tree
//   hashes must agree — any disagreement emits result "ABDiverge" (a
//   string the Rust side never emits, so compare fails loudly at
//   capture fidelity instead of pinning a lie).
//
// Determinism: the script is a pure function of the fixture (ids by
// RULE from the descending walk); every collection walk is a
// deterministic structure (the descending skip list, the sorted
// canonical text, the tree's TreeSet order); the changed-nets HashSet
// is emitted SORTED. No HashMap iteration reaches the output.
import app.freerouting.board.actions.ItemIdGenerator;
import app.freerouting.board.facade.BasicBoard;
import app.freerouting.board.model.items.Item;
import app.freerouting.board.model.structure.Components;
import app.freerouting.board.model.structure.FixedState;
import app.freerouting.board.searchtree.SearchTreeObject;
import app.freerouting.board.searchtree.ShapeSearchTree;
import app.freerouting.board.trace.PolylineTrace;
import app.freerouting.datastructures.ShapeTree;
import app.freerouting.datastructures.UndoableObjects;
import app.freerouting.geometry.planar.IntBox;
import app.freerouting.geometry.planar.IntPoint;
import app.freerouting.geometry.planar.Polyline;
import app.freerouting.io.BoardReadResult;
import app.freerouting.io.specctra.DsnReader;
import com.google.gson.JsonParser;
import java.io.BufferedWriter;
import java.io.ByteArrayInputStream;
import java.io.OutputStreamWriter;
import java.lang.reflect.Field;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Paths;
import java.security.MessageDigest;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.Iterator;
import java.util.LinkedList;
import java.util.List;
import java.util.Set;
import java.util.TreeSet;

public final class UndoOracle {

  private UndoOracle() {}

  // ---------------------------------------------------------------------
  // tiny JSON writer
  // ---------------------------------------------------------------------

  static final class Json {
    final StringBuilder sb = new StringBuilder();

    Json raw(CharSequence s) {
      sb.append(s);
      return this;
    }

    Json str(String s) {
      sb.append('"').append(esc(s)).append('"');
      return this;
    }

    Json num(long n) {
      sb.append(n);
      return this;
    }

    Json bool(boolean b) {
      sb.append(b);
      return this;
    }

    Json nul() {
      sb.append("null");
      return this;
    }

    Json numOrNull(Long n) {
      if (n == null) {
        return nul();
      }
      return num(n);
    }

    Json strOrNull(String s) {
      if (s == null) {
        return nul();
      }
      return str(s);
    }

    Json longArray(long[] values) {
      sb.append('[');
      for (int i = 0; i < values.length; i++) {
        if (i > 0) {
          sb.append(',');
        }
        num(values[i]);
      }
      sb.append(']');
      return this;
    }

    Json strArray(List<String> values) {
      sb.append('[');
      for (int i = 0; i < values.size(); i++) {
        if (i > 0) {
          sb.append(',');
        }
        str(values.get(i));
      }
      sb.append(']');
      return this;
    }

    static String esc(String s) {
      StringBuilder out = new StringBuilder();
      for (int i = 0; i < s.length(); i++) {
        char c = s.charAt(i);
        switch (c) {
          case '"' -> out.append("\\\"");
          case '\\' -> out.append("\\\\");
          case '\n' -> out.append("\\n");
          case '\r' -> out.append("\\r");
          case '\t' -> out.append("\\t");
          default -> out.append(c);
        }
      }
      return out.toString();
    }
  }

  // ---------------------------------------------------------------------
  // digests
  // ---------------------------------------------------------------------

  static String sha256Text(String text) throws Exception {
    MessageDigest digest = MessageDigest.getInstance("SHA-256");
    byte[] hashed = digest.digest(text.getBytes(StandardCharsets.UTF_8));
    StringBuilder hex = new StringBuilder();
    for (byte b : hashed) {
      hex.append(String.format("%02x", b));
    }
    return hex.toString();
  }

  /** The reflectively-read private `stackLevel` of an undo list. */
  static long levelOf(UndoableObjects list) throws Exception {
    Field f = UndoableObjects.class.getDeclaredField("stackLevel");
    f.setAccessible(true);
    return f.getInt(list);
  }

  private static Field componentsUndoListField = null;

  /** The components-side undo list (private field) — for its level. */
  static UndoableObjects componentsUndoList(Components components) throws Exception {
    if (componentsUndoListField == null) {
      Field f = Components.class.getDeclaredField("undoList");
      f.setAccessible(true);
      componentsUndoListField = f;
    }
    return (UndoableObjects) componentsUndoListField.get(components);
  }

  /** The sorted `id:idx` lines of the default tree's live leaf set —
   * the D27 content witness (a skipped tree remove/insert leaves the
   * SET wrong even when queries at one point still agree). Uses the
   * PUBLIC `ShapeTree.toArray()` (ShapeTree.java:67) — the exact twin
   * of the Rust `SearchTree::to_array()`; the raw order is the
   * in-order leaf walk, and the TreeSet makes the content a SET. */
  static List<String> treePairLines(BasicBoard board) {
    TreeSet<String> pairs = new TreeSet<>();
    for (ShapeTree.Leaf leaf : ((ShapeSearchTree) board.searchTreeManager.getDefaultTree())
        .toArray()) {
      pairs.add(((SearchTreeObject) leaf.object).getId() + ":" + leaf.shapeIndexInObject);
    }
    return new ArrayList<>(pairs);
  }

  // ---------------------------------------------------------------------
  // the script
  // ---------------------------------------------------------------------

  /** The per-fixture script inputs derived from the descending walk. */
  static final class Script {
    PolylineTrace t1;
    PolylineTrace victim; // X, may be null
  }

  /** Derives the script: t1 = the FIRST PolylineTrace of the
   * DESCENDING-id walk (the Item.compareTo order — the canonical
   * text's order; `getItems()`'s collection order is not trusted, the
   * explicit compareTo sort mirrors DsnParseOracle.canonicalGeometryText);
   * X = the NEXT PolylineTrace in that order that is not
   * deletion-forbidden. Pure function of the parse. */
  static Script deriveScript(BasicBoard board) {
    Script script = new Script();
    List<Item> items = new ArrayList<>(board.getItems());
    items.sort(Item::compareTo);
    for (Item item : items) {
      if (!(item instanceof PolylineTrace trace)) {
        continue;
      }
      if (script.t1 == null) {
        script.t1 = trace;
        continue;
      }
      if (script.victim == null && !trace.isDeletionForbidden()) {
        script.victim = trace;
        break;
      }
    }
    return script;
  }

  static IntBox queryBox(Script script) {
    IntPoint corner = (IntPoint) script.t1.polyline().corner(0);
    int hw = script.t1.getHalfWidth();
    return new IntBox(
        new IntPoint(corner.x - hw, corner.y - hw), new IntPoint(corner.x + hw, corner.y + hw));
  }

  // ---------------------------------------------------------------------
  // the replay
  // ---------------------------------------------------------------------

  /** One board's replay state: the board plus the digest machinery. */
  static final class Replay {
    final BasicBoard board;
    final Script script;
    final List<String> focusedIds = new ArrayList<>();
    final IntBox qbox;
    final int qLayer;

    Replay(BasicBoard board, Script script) {
      this.board = board;
      this.script = script;
      if (script.t1 != null) {
        focusedIds.add(String.valueOf(script.t1.getId()));
        if (script.victim != null) {
          focusedIds.add(String.valueOf(script.victim.getId()));
        }
      }
      this.qbox = script.t1 != null ? queryBox(script) : null;
      this.qLayer = script.t1 != null ? script.t1.getLayer() : 0;
    }

    /** The full per-step state digest (JSON object BODY — no braces;
     * the caller embeds it inside its own object). */
    Json digestBody() throws Exception {
      String geo = DsnParseOracle.canonicalGeometryText(board);
      int geoLines = geo.isEmpty() ? 0 : geo.split("\n", -1).length - 1;
      List<String> focused = new ArrayList<>();
      for (String line : geo.split("\n")) {
        int sp = line.indexOf(' ');
        if (sp > 0 && focusedIds.contains(line.substring(sp + 1).split(" ", 2)[0])) {
          focused.add(line);
        }
      }
      List<String> pairs = treePairLines(board);
      long[] queryIds = null;
      if (qbox != null) {
        Set<SearchTreeObject> objects =
            ((ShapeSearchTree) board.searchTreeManager.getDefaultTree())
                .overlappingObjects(qbox, qLayer);
        queryIds = new long[objects.size()];
        int i = 0;
        for (SearchTreeObject object : objects) {
          queryIds[i++] = object.getId();
        }
      }
      Json step =
          new Json()
              .raw("\"item_level\":")
              .num(levelOf(board.itemList))
              .raw(",\"comp_level\":")
              .num(levelOf(componentsUndoList(board.components)))
              .raw(",\"next_id\":")
              .num(board.communication.idGenerator.maxGeneratedId())
              .raw(",\"item_count\":")
              .num(board.getItems().size())
              .raw(",\"geo_sha\":")
              .str(sha256Text(geo))
              .raw(",\"geo_lines\":")
              .num(geoLines)
              .raw(",\"geo_focused\":")
              .strArray(focused)
              .raw(",\"tree_sha\":")
              .str(sha256Text(String.join("\n", pairs)))
              .raw(",\"tree_n\":")
              .num(pairs.size())
              .raw(",\"tree_pairs\":");
      if (pairs.size() <= 400) {
        step.strArray(pairs);
      } else {
        step.nul();
      }
      step.raw(",\"query\":");
      if (queryIds == null) {
        step.nul();
      } else {
        step.longArray(queryIds);
      }
      return step;
    }

    /** The state fingerprint used for the A/B equality check. */
    String fingerprint() throws Exception {
      String geo = DsnParseOracle.canonicalGeometryText(board);
      List<String> pairs = treePairLines(board);
      return board.getItems().size()
          + "/"
          + levelOf(board.itemList)
          + "/"
          + levelOf(componentsUndoList(board.components))
          + "/"
          + board.communication.idGenerator.maxGeneratedId()
          + "/"
          + sha256Text(geo)
          + "/"
          + sha256Text(String.join("\n", pairs));
    }
  }

  /** Verbatim copy of `BasicBoard.applyUndoRedoSideEffects`
   * (BasicBoard.java:1256-1288) — board B's side-effect phase (the
   * facade does not expose the collections). `currentItem.board = this`
   * and `clearAutorouteInfo` are kept as the no-ops they are here (the
   * arena has no board back-pointer; autoroute info is not landed). */
  static void applyUndoRedoSideEffectsCopy(
      BasicBoard board,
      LinkedList<UndoableObjects.Storable> cancelledObjects,
      LinkedList<UndoableObjects.Storable> restoredObjects,
      Set<Integer> changedNets) {
    Iterator<UndoableObjects.Storable> it = cancelledObjects.iterator();
    while (it.hasNext()) {
      Item currentItem = (Item) it.next();
      board.searchTreeManager.remove(currentItem);
      if ((board.communication != null) && (board.communication.observers != null)) {
        board.communication.observers.notifyDeleted(currentItem);
      }
      if (changedNets != null) {
        for (int i = 0; i < currentItem.netCount(); i++) {
          changedNets.add(currentItem.getNetNumber(i));
        }
      }
    }
    it = restoredObjects.iterator();
    while (it.hasNext()) {
      Item currentItem = (Item) it.next();
      board.searchTreeManager.insert(currentItem);
      currentItem.clearAutorouteInfo();
      if ((board.communication != null) && (board.communication.observers != null)) {
        board.communication.observers.notifyNew(currentItem);
      }
      if (changedNets != null) {
        for (int i = 0; i < currentItem.netCount(); i++) {
          changedNets.add(currentItem.getNetNumber(i));
        }
      }
    }
  }

  /** Runs the undo step on board A (real facade). Returns
   * {ret, sortedNets}. */
  record FacadeUndoResult(boolean ret, long[] sortedNets) {}

  static FacadeUndoResult facadeUndo(BasicBoard board, boolean redo) {
    Set<Integer> changed = new HashSet<>();
    boolean ret = redo ? board.redo(changed) : board.undo(changed);
    long[] nets = changed.stream().mapToLong(Integer::longValue).sorted().toArray();
    return new FacadeUndoResult(ret, nets);
  }

  /** Board B's undo-step capture: the return value + nets, PLUS the
   * ordered cancelled/restored id lists exposed by the container call. */
  record ExposedResult(
      boolean ret,
      long[] sortedNets,
      long[] cancelledIds,
      long[] restoredIds,
      String fingerprint) {}

  static ExposedResult exposedUndo(Replay replay, boolean redo) throws Exception {
    BasicBoard board = replay.board;
    boolean compRet = redo ? board.components.redo(null) : board.components.undo(null);
    LinkedList<UndoableObjects.Storable> cancelled = new LinkedList<>();
    LinkedList<UndoableObjects.Storable> restored = new LinkedList<>();
    boolean ret =
        redo
            ? board.itemList.redo(cancelled, restored)
            : board.itemList.undo(cancelled, restored);
    Set<Integer> changed = new HashSet<>();
    applyUndoRedoSideEffectsCopy(board, cancelled, restored, changed);
    long[] nets = changed.stream().mapToLong(Integer::longValue).sorted().toArray();
    long[] cancelledIds = cancelled.stream().mapToLong(s -> ((Item) s).getId()).toArray();
    long[] restoredIds = restored.stream().mapToLong(s -> ((Item) s).getId()).toArray();
    return new ExposedResult(ret, nets, cancelledIds, restoredIds, replay.fingerprint());
  }

  // ---------------------------------------------------------------------
  // the protocol
  // ---------------------------------------------------------------------

  static String evaluateFixture(String id, String path, byte[] bytes) throws Exception {
    // Board A + board B: two independent parses (the read's in-scope
    // normalizeAllTraces tail already ran on both).
    ItemIdGenerator genA = new ItemIdGenerator();
    String name = Paths.get(path).getFileName().toString();
    BoardReadResult readA =
        DsnReader.readBoard(new ByteArrayInputStream(bytes), null, genA, name);
    if (!(readA instanceof BoardReadResult.Success successA)) {
      return new Json().raw("{\"id\":").str(id).raw(",\"file\":").str(path)
          .raw(",\"result\":\"read-failed\",\"facts\":null,\"steps\":null}").sb.toString();
    }
    ItemIdGenerator genB = new ItemIdGenerator();
    BoardReadResult readB =
        DsnReader.readBoard(new ByteArrayInputStream(bytes), null, genB, name);
    if (!(readB instanceof BoardReadResult.Success successB)) {
      return new Json().raw("{\"id\":").str(id).raw(",\"file\":").str(path)
          .raw(",\"result\":\"read-failed\",\"facts\":null,\"steps\":null}").sb.toString();
    }
    BasicBoard boardA = successA.board();
    BasicBoard boardB = successB.board();

    Script script = deriveScript(boardA);
    Script scriptB = deriveScript(boardB);
    Replay replayA = new Replay(boardA, script);
    Replay replayB = new Replay(boardB, scriptB);

    // ---- facts -------------------------------------------------------
    Long tId = script.t1 != null ? (long) script.t1.getId() : null;
    Long tLayer = script.t1 != null ? (long) script.t1.getLayer() : null;
    Long tHw = script.t1 != null ? (long) script.t1.getHalfWidth() : null;
    Long tCls = script.t1 != null ? (long) script.t1.clearanceClassIndex() : null;
    long[] tNets = {};
    String insC0 = null;
    String insC1 = null;
    if (script.t1 != null) {
      IntPoint c0 = (IntPoint) script.t1.polyline().corner(0);
      IntPoint c1 = (IntPoint) script.t1.polyline().corner(1);
      insC0 = c0.x + " " + c0.y;
      insC1 = c1.x + " " + c1.y;
      tNets = java.util.Arrays.stream(script.t1.netNumbers).asLongStream().toArray();
    }
    Long xId = script.victim != null ? (long) script.victim.getId() : null;

    Json facts =
        new Json()
            .raw("{\"items\":")
            .num(boardA.getItems().size())
            .raw(",\"t_id\":")
            .numOrNull(tId)
            .raw(",\"t_layer\":")
            .numOrNull(tLayer)
            .raw(",\"t_hw\":")
            .numOrNull(tHw)
            .raw(",\"t_cls\":")
            .numOrNull(tCls)
            .raw(",\"t_c0\":")
            .strOrNull(insC0)
            .raw(",\"t_c1\":")
            .strOrNull(insC1)
            .raw(",\"t_nets\":")
            .longArray(tNets)
            .raw(",\"x_id\":")
            .numOrNull(xId)
            .raw("}");

    // ---- the step script ----------------------------------------------
    List<String> steps = new ArrayList<>();
    StringBuilder abNote = new StringBuilder();
    String result = "ok";

    // Step 0: the post-parse baseline (a divergence here is a READ bug,
    // not an undo bug — it self-explains).
    steps.add(
        new Json()
            .raw("{\"n\":0,\"op\":")
            .str("baseline")
            .raw(",\"ret\":null,\"ids_cancelled\":[],\"ids_restored\":[],\"nets\":[],\"digest\":{")
            .raw(replayA.digestBody().sb)
            .raw("}}")
            .sb.toString());

    int n = 0;
    long sId = -1;
    // The op list: snap, insert, snap, remove, normalize, query, undo,
    // query, undo, redo, query, pop, undo, redo, query, undo, query,
    // pop, undo — the insert/remove/normalize/query steps only when the
    // fixture carries a trace (pure function of the parse).
    List<String> ops = new ArrayList<>();
    ops.add("snap");
    if (script.t1 != null) {
      ops.add("insert_trace");
      ops.add("snap");
      if (script.victim != null) {
        ops.add("remove_item");
      }
      ops.add("normalize_all");
      ops.add("query");
    }
    ops.add("undo");
    if (script.t1 != null) {
      ops.add("query");
    }
    ops.add("undo");
    ops.add("redo");
    if (script.t1 != null) {
      ops.add("query");
    }
    ops.add("pop_snapshot");
    ops.add("undo");
    ops.add("redo");
    if (script.t1 != null) {
      ops.add("query");
    }
    ops.add("undo");
    if (script.t1 != null) {
      ops.add("query");
    }
    ops.add("pop_snapshot");
    ops.add("undo");

    for (String op : ops) {
      n++;
      final Json head = new Json().raw("{\"n\":").num(n).raw(",\"op\":").str(op);
      boolean isUndo = op.equals("undo");
      boolean isRedo = op.equals("redo");

      Long ret = null;
      long[] cancelledIds = {};
      long[] restoredIds = {};
      long[] nets = {};

      switch (op) {
        case "snap" -> {
          boardA.generateSnapshot();
          boardB.generateSnapshot();
        }
        case "insert_trace" -> {
          IntPoint c0 = (IntPoint) script.t1.polyline().corner(0);
          IntPoint c1 = (IntPoint) script.t1.polyline().corner(1);
          Polyline polyline = new Polyline(new app.freerouting.geometry.planar.Point[] {c0, c1});
          boardA.insertTrace(
              polyline,
              script.t1.getLayer(),
              script.t1.getHalfWidth(),
              script.t1.netNumbers,
              script.t1.clearanceClassIndex(),
              FixedState.UNFIXED);
          boardB.insertTrace(
              polyline,
              scriptB.t1.getLayer(),
              scriptB.t1.getHalfWidth(),
              scriptB.t1.netNumbers,
              scriptB.t1.clearanceClassIndex(),
              FixedState.UNFIXED);
          sId = boardA.communication.idGenerator.maxGeneratedId();
        }
        case "remove_item" -> {
          boardA.removeItem(script.victim);
          boardB.removeItem(scriptB.victim);
        }
        case "normalize_all" -> {
          ret = boardA.normalizeAllTraces() ? 1L : 0L;
          boardB.normalizeAllTraces();
        }
        case "query" -> {
          // The read-only op: the digest's query row IS the capture.
        }
        case "pop_snapshot" -> {
          ret = boardA.popSnapshot() ? 1L : 0L;
          boardB.popSnapshot();
        }
        case "undo", "redo" -> {
          FacadeUndoResult a = facadeUndo(boardA, isRedo);
          ExposedResult b = exposedUndo(replayB, isRedo);
          ret = a.ret() ? 1L : 0L;
          nets = a.sortedNets();
          // cancelled/restored IN ORDER — board B's exposed lists. The
          // cancelled list may carry an object ALSO in restored (the
          // swap case); both raw orders are the contract.
          cancelledIds = b.cancelledIds();
          restoredIds = b.restoredIds();
          // The A/B fidelity check: the exposed-lists replay must land
          // in the SAME state as the real facade call.
          boolean agree =
              a.ret() == b.ret()
                  && java.util.Arrays.equals(a.sortedNets(), b.sortedNets())
                  && replayA.fingerprint().equals(b.fingerprint());
          if (!agree && result.equals("ok")) {
            result = "ABDiverge";
            abNote
                .append("step ")
                .append(n)
                .append(" ")
                .append(op)
                .append(" A{ret=")
                .append(a.ret())
                .append(" nets=")
                .append(java.util.Arrays.toString(a.sortedNets()))
                .append(" fp=")
                .append(replayA.fingerprint())
                .append("} B{ret=")
                .append(b.ret())
                .append(" nets=")
                .append(java.util.Arrays.toString(b.sortedNets()))
                .append(" fp=")
                .append(b.fingerprint())
                .append("}");
          }
        }
        default -> throw new IllegalStateException("unknown op " + op);
      }

      Json row =
          new Json()
              .raw(head.sb)
              .raw(",\"ret\":")
              .raw(ret == null ? "null" : (ret == 1 ? "true" : "false"))
              .raw(",\"ids_cancelled\":")
              .longArray(cancelledIds)
              .raw(",\"ids_restored\":")
              .longArray(restoredIds)
              .raw(",\"nets\":")
              .longArray(nets)
              .raw(",\"digest\":{")
              .raw(replayA.digestBody().sb)
              .raw("}}");
      steps.add(row.sb.toString());
    }

    // facts.s_id LAST (both sides): the insert's id-gen watermark — the
    // dsn-0151 id-burn drift detector (a future fixture where the
    // sides' generators disagree diverges HERE).
    Json factsFinal =
        new Json()
            .raw(facts.sb.toString().substring(0, facts.sb.length() - 1))
            .raw(",\"s_id\":")
            .numOrNull(sId >= 0 ? sId : null)
            .raw("}");

    if (abNote.length() > 0) {
      System.err.println("ABDiverge for case id " + id + ": " + abNote);
    }

    return new Json()
        .raw("{\"id\":")
        .str(id)
        .raw(",\"file\":")
        .str(path)
        .raw(",\"result\":")
        .str(result)
        .raw(",\"facts\":")
        .raw(factsFinal.sb)
        .raw(",\"steps\":[")
        .raw(String.join(",", steps))
        .raw("]}")
        .sb.toString();
  }

  // ---------------------------------------------------------------------
  // entry
  // ---------------------------------------------------------------------

  public static void main(String[] args) throws Exception {
    if (args.length != 1) {
      System.err.println("usage: UndoOracle <manifest.jsonl>");
      System.exit(2);
    }
    List<String> lines = Files.readAllLines(Paths.get(args[0]), StandardCharsets.UTF_8);
    BufferedWriter out =
        new BufferedWriter(new OutputStreamWriter(System.out, StandardCharsets.UTF_8));
    for (String line : lines) {
      String trimmed = line.trim();
      if (trimmed.isEmpty()) {
        continue;
      }
      String id = null;
      String path = null;
      try {
        com.google.gson.JsonObject entry = JsonParser.parseString(trimmed).getAsJsonObject();
        id = entry.get("id").getAsString();
        path = entry.get("path").getAsString();
      } catch (RuntimeException e) {
        out.flush();
        if (id != null) {
          System.err.println("manifest format error for case id " + id + ": " + e);
        } else {
          String raw = trimmed.length() <= 120 ? trimmed : trimmed.substring(0, 120) + "...";
          System.err.println("manifest format error on line '" + raw + "': " + e);
        }
        System.exit(3);
        return;
      }
      String record;
      try {
        byte[] bytes = Files.readAllBytes(Paths.get(path));
        record = evaluateFixture(id, path, bytes);
      } catch (Throwable t) {
        System.err.println("evaluate error for case id " + id + ": " + t);
        record =
            "{\"id\":\""
                + Json.esc(id)
                + "\",\"file\":\""
                + Json.esc(path)
                + "\",\"result\":\"EvalError\",\"facts\":null,\"steps\":null}";
      }
      out.write(record);
      out.write("\n");
      out.flush();
    }
    out.flush();
  }
}
