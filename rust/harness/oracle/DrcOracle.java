// DrcOracle.java — the jar-side oracle for the M3 Task 2 DRC parity
// corpus (rust/harness/src/drc_corpus.rs is the doc-of-record for the
// protocol; THIS file mirrors it exactly).
//
// Lives OUTSIDE src/ (harness-side; the frozen Java tree is never
// touched). Declares the app.freerouting.datastructures package for
// the IndexOracle compile precedent — everything it touches is public
// API (DesignRulesChecker, NetIncompletes' public constructor,
// ClearanceViolation's public fields).
//
// Build/run (JDK 25, from the repo root; the harness's `drc golden`
// does exactly this):
//   mkdir -p /tmp/epic-drc-classes && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/javac \
//       -cp build/libs/freerouting-current-executable.jar \
//       -d /tmp/epic-drc-classes rust/harness/oracle/DrcOracle.java && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/java \
//       -Duser.language=en -Duser.country=US \
//       -cp build/libs/freerouting-current-executable.jar:/tmp/epic-drc-classes \
//       app.freerouting.datastructures.DrcOracle <manifest.jsonl>
//
// Input: one {"id":"drc-NNNN","path":"repo/relative.dsn"} per line.
// Output: ONE JSONL result line per fixture (stdout lines starting
// with {"id" are the results; the jar's FRLogger noise is interleaved
// and dropped by the reader). An evaluate-time throwable emits a
// full-null "EvalError" record and the batch CONTINUES (DsnParseOracle
// discipline — the Rust side never emits that string, so compare
// flags it loudly); a malformed manifest line is a named exit 3.
//
// The protocol per fixture (both sides identical — see the Rust
// module docs):
//   parse → reinsertTreeItems() (the uniform fill normalization, the
//   index-corpus discipline: Java's read fills trees ascending, the
//   port's rebuild path is descending, so BOTH sides populate the
//   default tree through the shared public reinsert) →
//   DesignRulesChecker(board, null).calculateAllIncompletes() →
//   incomplete_count = getIncompleteCount(), max_connections = the
//   public maxConnections field (endpoint CODE formula, NOT the stale
//   "(formula: total_items - netCount)" log string) →
//   getAllClearanceViolations() deduped by Java's
//   sorted-id-"-"-sorted-id-"-"-layer key; emitted as {a,b,layer}
//   with a=min(id1,id2), b=max (the dedup makes direction irrelevant;
//   the sort canonicalizes) →
//   per_net rows (schema v2): the RAW per-net item lists rebuilt with
//   calculateAllIncompletes' own itemList loop (Connectable items
//   appended per net number, multi-net items in EACH list), then a
//   fresh public NetIncompletes(netNo, rawItems, board) per net with
//   items ≥ 1 — items = the RAW list size, groups =
//   getConnectedGroupCount() (post-filter), incomplete_count =
//   count(). Rows sorted by net_no ascending (nets with items only).
//   v2 adds the Delaunay surface the port must reproduce (the count
//   alone is edge-set-dependent under cocircular degeneracy — the
//   M3-T2 falsification of the max(0, groups-1) claim is exactly
//   that, so the goldens pin the edge set, not just its Kruskal
//   yield): ratsnest = {id, n} per GROUPED (filtered) item — the
//   Delaunay input objects with getRatsnestCorners().length corners,
//   sorted by id (n == 0 is the falsification mechanism: a
//   both-ends-contacted trace contributes no corner); edges = the
//   (min-id, max-id) pairs of ALL triangulation ResultEdges —
//   degenerate zero-length coincident-corner edges included, exact
//   duplicates collapsed (mirroring the TreeSet<Edge> the pairs
//   feed), sorted lexicographically. Reconstructed from public API
//   (getConnectedSet grouping + public PlanarDelaunayTriangulation)
//   with the count cross-check below as the drift alarm: within one
//   JVM the reconstruction's HashSet<Item> iterates the same objects
//   in the same order as the checker's, so the reconstructed
//   triangulation IS the checker's.
//
// Equivalence claim under test (M3-T2 brief): incomplete_count ==
// max(0, groups - 1) whenever groups >= 1 — i.e. the Delaunay/Kruskal
// airline count equals the connected-group count minus one. The
// oracle emits BOTH groups and count per net and WARNS on stderr when
// they diverge (a witness must SURVIVE the capture, not kill it);
// the capture-time analysis and the Rust compare enforce it.
//
// Machinery self-checks (named exit 4 — a broken oracle must never
// write a poisoned golden): every fresh NetIncompletes count equals
// checker.getIncompleteCount(netNo), and the per-net sum equals
// getIncompleteCount().
//
// Determinism: the per_net walk is by ascending net number over the
// board's own itemList order (insertion = ascending id); the
// violations list is sorted by (a, b, layer); no HashMap iteration
// anywhere in the output path.
package app.freerouting.datastructures;

import app.freerouting.board.facade.BasicBoard;
import app.freerouting.board.model.items.Connectable;
import app.freerouting.board.model.items.ConductionArea;
import app.freerouting.board.model.items.DrillItem;
import app.freerouting.board.model.items.Item;
import app.freerouting.board.actions.ItemIdGenerator;
import app.freerouting.drc.ClearanceViolation;
import app.freerouting.drc.DesignRulesChecker;
import app.freerouting.drc.NetIncompletes;
import app.freerouting.io.BoardReadResult;
import app.freerouting.io.specctra.DsnReader;
import app.freerouting.geometry.planar.Point;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import java.io.BufferedWriter;
import java.io.ByteArrayInputStream;
import java.io.OutputStreamWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.Collection;
import java.util.Comparator;
import java.util.Formatter;
import java.util.HashSet;
import java.util.Iterator;
import java.util.List;
import java.util.Set;

public final class DrcOracle {

  private DrcOracle() {}

  /**
   * NetIncompletes.NetItem twin for the schema-v2 reconstruction: an Item wrapper carrying its
   * ratsnest corners into the public triangulation. Deliberately holds no equals/hashCode —
   * identity semantics, exactly like the private original.
   */
  private static final class TriStorable implements PlanarDelaunayTriangulation.Storable {

    final Item item;

    TriStorable(Item item) {
      this.item = item;
    }

    @Override
    public Point[] getTriangulationCorners() {
      return this.item.getRatsnestCorners();
    }
  }

  // ---------------------------------------------------------------------
  // tiny JSON writer (field order matches the Rust GoldenRecord exactly)
  // ---------------------------------------------------------------------

  static final class Json {
    final StringBuilder sb = new StringBuilder();

    Json raw(String s) {
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

    Json nul() {
      sb.append("null");
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
          default -> {
            if (c < 0x20) {
              try (Formatter f = new Formatter()) {
                out.append(f.format("\\u%04x", (int) c).toString());
              }
            } else {
              out.append(c);
            }
          }
        }
      }
      return out.toString();
    }
  }

  // ---------------------------------------------------------------------
  // the protocol
  // ---------------------------------------------------------------------

  static String evaluateFixture(String id, String path, byte[] bytes) throws Exception {
    IdGenerator idGenerator = new ItemIdGenerator();
    String name = Paths.get(path).getFileName().toString();
    BoardReadResult read =
        DsnReader.readBoard(new ByteArrayInputStream(bytes), null, idGenerator, name);
    if (!(read instanceof BoardReadResult.Success success)) {
      return new Json().raw("{\"id\":").str(id).raw(",\"file\":").str(path)
          .raw(",\"result\":\"read-failed\",\"incomplete_count\":null,"
              + "\"max_connections\":null,\"clearance_violations_total\":null,"
              + "\"violations\":null,\"per_net\":null}").sb.toString();
    }
    BasicBoard board = success.board();

    // The uniform fill normalization (index-corpus discipline): both
    // sides populate the default tree through the shared public
    // reinsert, so the DRC queries below run over identically-filled
    // trees regardless of each reader's fill order.
    board.searchTreeManager.reinsertTreeItems();

    DesignRulesChecker checker = new DesignRulesChecker(board, null);
    checker.calculateAllIncompletes();
    long incompleteCount = checker.getIncompleteCount();
    long maxConnections = checker.maxConnections;

    // Clearance violations, deduped by Java's sorted-id key
    // (getAllClearanceViolations). The dedup keeps the FIRST
    // occurrence of each (pair, layer); direction is irrelevant
    // (A-B and B-A share a key), so the canonical row is
    // (min, max, layer), sorted by (a, b, layer).
    Collection<ClearanceViolation> violations = checker.getAllClearanceViolations();
    long[][] vrows = new long[violations.size()][3];
    int vi = 0;
    for (ClearanceViolation violation : violations) {
      int id1 = violation.firstItem.getId();
      int id2 = violation.secondItem.getId();
      vrows[vi][0] = Math.min(id1, id2);
      vrows[vi][1] = Math.max(id1, id2);
      vrows[vi][2] = violation.layer;
      vi++;
    }
    java.util.Arrays.sort(
        vrows,
        (x, y) -> {
          for (int k = 0; k < 3; k++) {
            if (x[k] != y[k]) {
              return Long.compare(x[k], y[k]);
            }
          }
          return 0;
        });

    // Per-net rows: rebuild the RAW net item lists with
    // calculateAllIncompletes' own loop (itemList insertion order =
    // ascending id; Connectable items appended per net number,
    // multi-net items in EACH list).
    int maxNetNo = board.rules.nets.maxNetNumber();
    List<List<Item>> netItemLists = new ArrayList<>(maxNetNo);
    for (int i = 0; i < maxNetNo; i++) {
      netItemLists.add(new ArrayList<>());
    }
    Iterator<UndoableObjects.UndoableObjectNode> it = board.itemList.startReadObject();
    for (; ; ) {
      Item currentItem = (Item) board.itemList.readObject(it);
      if (currentItem == null) {
        break;
      }
      if (currentItem instanceof Connectable) {
        for (int i = 0; i < currentItem.netCount(); i++) {
          netItemLists.get(currentItem.getNetNumber(i) - 1).add(currentItem);
        }
      }
    }

    long sumCounts = 0;
    StringBuilder perNet = new StringBuilder("[");
    boolean anyNet = false;
    for (int netNo = 1; netNo <= maxNetNo; netNo++) {
      List<Item> raw = netItemLists.get(netNo - 1);
      if (raw.isEmpty()) {
        continue;
      }
      // The constructor is read-only over the board, so this fresh
      // run alongside checker's own netIncompletes is safe.
      NetIncompletes netIncompletes = new NetIncompletes(netNo, raw, board);
      int count = netIncompletes.count();
      int groups = netIncompletes.getConnectedGroupCount();
      int checkerCount = checker.getIncompleteCount(netNo);
      if (count != checkerCount) {
        System.err.println(
            "machinery error for case id "
                + id
                + ": fresh NetIncompletes count "
                + count
                + " != checker.getIncompleteCount("
                + netNo
                + ") "
                + checkerCount);
        System.exit(4);
        return null;
      }
      sumCounts += count;
      if (count != Math.max(0, groups - 1)) {
        // Equivalence WITNESS — the capture survives so the claim's
        // falsifier is inspectable; the Rust compare will flag the
        // divergent row loudly (the Rust port implements the FULL
        // Delaunay/Kruskal semantics, so both sides carry the
        // witness identically).
        System.err.println(
            "equivalence witness for case id "
                + id
                + ": net "
                + netNo
                + " has count "
                + count
                + " but max(0, groups-1) = "
                + Math.max(0, groups - 1)
                + " (groups="
                + groups
                + ", rawItems="
                + raw.size()
                + ")");
      }

      // ---- schema-v2 reconstruction (public API only) --------------
      // Filter verbatim, group via getConnectedSet (within-group
      // order = the TreeSet's descending id), triangulate, and emit
      // the ratsnest corner counts + canonical edge pairs. The
      // count/groups rows above are the drift alarm: if this
      // reconstruction ever disagreed with the checker's own run,
      // the machinery check below cannot be silenced.
      List<Item> filtered = new ArrayList<>();
      for (Item item : raw) {
        if (item.isTail()) {
          continue;
        }
        if (!(item instanceof ConductionArea)
            && !(item instanceof DrillItem)
            && item.getNormalContacts().isEmpty()) {
          continue;
        }
        filtered.add(item);
      }
      List<TriStorable> storables = new ArrayList<>();
      Set<Item> remaining = new HashSet<>(filtered);
      int distinctSets = 0;
      while (!remaining.isEmpty()) {
        Item startItem = remaining.iterator().next();
        Set<Item> connectedSet = startItem.getConnectedSet(netNo);
        List<Item> members = new ArrayList<>();
        for (Item member : connectedSet) {
          if (remaining.contains(member)) {
            members.add(member);
          }
        }
        if (!members.isEmpty()) {
          distinctSets++;
        }
        for (Item member : members) {
          storables.add(new TriStorable(member));
        }
        remaining.removeAll(members);
      }
      if (distinctSets != groups) {
        System.err.println(
            "machinery error for case id "
                + id
                + ": net "
                + netNo
                + " reconstructed groups "
                + distinctSets
                + " != checker groups "
                + groups);
        System.exit(4);
        return null;
      }
      storables.sort(Comparator.comparingInt(storable -> storable.item.getId()));
      StringBuilder ratsnest = new StringBuilder("[");
      for (int si = 0; si < storables.size(); si++) {
        if (si > 0) {
          ratsnest.append(',');
        }
        ratsnest
            .append("{\"id\":")
            .append(storables.get(si).item.getId())
            .append(",\"n\":")
            .append(storables.get(si).getTriangulationCorners().length)
            .append('}');
      }
      ratsnest.append(']');
      long[][] erows = new long[0][];
      if (storables.size() > 1) {
        // Same construction the NetIncompletes constructor makes
        // (setSeed(99) per instance makes each triangulation
        // independent of JVM history).
        PlanarDelaunayTriangulation triangulation =
            new PlanarDelaunayTriangulation(
                new ArrayList<PlanarDelaunayTriangulation.Storable>(storables));
        List<long[]> edgeList = new ArrayList<>();
        for (PlanarDelaunayTriangulation.ResultEdge resultEdge : triangulation.getEdgeLines()) {
          int ea = ((TriStorable) resultEdge.startObject).item.getId();
          int eb = ((TriStorable) resultEdge.endObject).item.getId();
          edgeList.add(new long[] {Math.min(ea, eb), Math.max(ea, eb)});
        }
        edgeList.sort(
            (x, y) -> {
              for (int k = 0; k < 2; k++) {
                if (x[k] != y[k]) {
                  return Long.compare(x[k], y[k]);
                }
              }
              return 0;
            });
        // Collapse exact duplicates (two ResultEdges over the same
        // item pair): the NetIncompletes TreeSet<Edge> does the same
        // by full geometry, and (pair) is all this schema pins.
        List<long[]> deduped = new ArrayList<>();
        for (long[] edge : edgeList) {
          if (deduped.isEmpty()
              || deduped.get(deduped.size() - 1)[0] != edge[0]
              || deduped.get(deduped.size() - 1)[1] != edge[1]) {
            deduped.add(edge);
          }
        }
        erows = deduped.toArray(new long[0][]);
      }
      StringBuilder edgesJson = new StringBuilder("[");
      for (int ei = 0; ei < erows.length; ei++) {
        if (ei > 0) {
          edgesJson.append(',');
        }
        edgesJson.append('[').append(erows[ei][0]).append(',').append(erows[ei][1]).append(']');
      }
      edgesJson.append(']');
      if (anyNet) {
        perNet.append(',');
      }
      anyNet = true;
      perNet.append("{\"net_no\":")
          .append(netNo)
          .append(",\"items\":")
          .append(raw.size())
          .append(",\"groups\":")
          .append(groups)
          .append(",\"incomplete_count\":")
          .append(count)
          .append(",\"ratsnest\":")
          .append(ratsnest)
          .append(",\"edges\":")
          .append(edgesJson)
          .append('}');
    }
    perNet.append(']');
    if (sumCounts != incompleteCount) {
      System.err.println(
          "machinery error for case id "
              + id
              + ": per-net sum "
              + sumCounts
              + " != getIncompleteCount() "
              + incompleteCount);
      System.exit(4);
      return null;
    }

    StringBuilder violationsJson = new StringBuilder("[");
    for (int i = 0; i < vrows.length; i++) {
      if (i > 0) {
        violationsJson.append(',');
      }
      violationsJson
          .append("{\"a\":")
          .append(vrows[i][0])
          .append(",\"b\":")
          .append(vrows[i][1])
          .append(",\"layer\":")
          .append(vrows[i][2])
          .append('}');
    }
    violationsJson.append(']');

    return new Json()
        .raw("{\"id\":")
        .str(id)
        .raw(",\"file\":")
        .str(path)
        .raw(",\"result\":\"ok\"")
        .raw(",\"incomplete_count\":")
        .num(incompleteCount)
        .raw(",\"max_connections\":")
        .num(maxConnections)
        .raw(",\"clearance_violations_total\":")
        .num(vrows.length)
        .raw(",\"violations\":")
        .raw(violationsJson.toString())
        .raw(",\"per_net\":")
        .raw(perNet.toString())
        .raw("}")
        .sb.toString();
  }

  // ---------------------------------------------------------------------
  // entry
  // ---------------------------------------------------------------------

  public static void main(String[] args) throws Exception {
    if (args.length != 1) {
      System.err.println("usage: DrcOracle <manifest.jsonl>");
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
        JsonObject entry = JsonParser.parseString(trimmed).getAsJsonObject();
        id = entry.get("id").getAsString();
        path = entry.get("path").getAsString();
      } catch (RuntimeException e) {
        out.flush();
        // Named-exit manifest-error path (DsnParseOracle convention):
        // name the id when the line parsed far enough to yield one;
        // otherwise the raw line, truncated.
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
        // Per-fixture capture (DsnParseOracle discipline): one bad
        // fixture must not kill the batch. The record carries the
        // FULL null-field shape so the Rust reader parses it, with a
        // result string the Rust side NEVER emits — the committed
        // line fails compare loudly instead of truncating the capture.
        System.err.println("evaluate error for case id " + id + ": " + t);
        record =
            "{\"id\":\""
                + Json.esc(id)
                + "\",\"file\":\""
                + Json.esc(path)
                + "\",\"result\":\"EvalError\",\"incomplete_count\":null,"
                + "\"max_connections\":null,\"clearance_violations_total\":null,"
                + "\"violations\":null,\"per_net\":null}";
      }
      out.write(record);
      out.write("\n");
      // Flush per case: interleaved logger noise keeps result lines
      // atomic (M1a bug-078 lesson).
      out.flush();
    }
    out.flush();
  }
}
