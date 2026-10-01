// IndexOracle.java — the jar-side oracle for the M2 Task 9 index
// parity corpus (rust/harness/src/index_corpus.rs is the doc-of-record
// for the protocol; THIS file mirrors it exactly).
//
// Lives OUTSIDE src/ (harness-side; the frozen Java tree is never
// touched). Declares the app.freerouting.datastructures package for
// access to ShapeTree.root / InnerNode / Leaf (MinAreaTreeSpike
// precedent) — everything else it touches is public API.
//
// Build/run (JDK 25, from the repo root; the harness's `index golden`
// does exactly this):
//   mkdir -p /tmp/epic-index-classes && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/javac \
//       -cp build/libs/freerouting-current-executable.jar \
//       -d /tmp/epic-index-classes rust/harness/oracle/IndexOracle.java && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/java \
//       -Duser.language=en -Duser.country=US \
//       -cp build/libs/freerouting-current-executable.jar:/tmp/epic-index-classes \
//       app.freerouting.datastructures.IndexOracle <manifest.jsonl>
//
// Input: one {"id":"idx-NNNN","path":"repo/relative.dsn"} per line.
// Output: ONE JSONL result line per fixture (stdout lines starting
// with {"id" are the results; the jar's FRLogger noise is interleaved
// and dropped by the reader). An evaluate-time throwable emits a
// full-null "EvalError" record and the batch CONTINUES (DsnParseOracle
// discipline — the Rust side never emits that string, so compare
// flags it loudly); a malformed manifest line is a named exit 3.
//
// The protocol per fixture (both sides identical — see the Rust
// module docs):
//   parse → pre-steps (drill-inflate: setHoleClearance(2500)) →
//   reinsertTreeItems() →
//   cc0 dump → setClearanceCompensationUsed(true) → cc1 dump →
//   getAutorouteTree(2) → cc2 dump →
//   replay rmT/insS/rmS with the per-tree query set (own0/own1/
//   xlate±hw/ctrbox × 6 ignore lists) → with-clearance core at
//   classes {0,1,2} on cc1 → objects/items family (ctrbox
//   overlapping_objects per queried tree + one items_with_clearance
//   row on cc1) → check_shape (ctrbox + outside) →
//   validate_entries on the first 5 items with entries.
//
// Determinism: every collection walk is the board's own ordered
// structure (getItems() is the descending skip-list; the tree walks
// are deterministic pre-order). No HashMap iteration anywhere in the
// output path.
package app.freerouting.datastructures;

import app.freerouting.board.actions.ItemIdGenerator;
import app.freerouting.board.facade.BasicBoard;
import app.freerouting.board.model.items.Item;
import app.freerouting.board.model.items.Trace;
import app.freerouting.board.model.structure.FixedState;
import app.freerouting.board.searchtree.SearchTreeObject;
import app.freerouting.board.searchtree.ShapeSearchTree;
import app.freerouting.board.trace.PolylineTrace;
import app.freerouting.datastructures.ShapeTree.TreeNode;
import app.freerouting.geometry.planar.IntBox;
import app.freerouting.geometry.planar.IntOctagon;
import app.freerouting.geometry.planar.IntPoint;
import app.freerouting.geometry.planar.Point;
import app.freerouting.geometry.planar.Polyline;
import app.freerouting.geometry.planar.RegularTileShape;
import app.freerouting.geometry.planar.Shape;
import app.freerouting.geometry.planar.Simplex;
import app.freerouting.geometry.planar.TileShape;
import app.freerouting.geometry.planar.Vector;
import app.freerouting.io.BoardReadResult;
import app.freerouting.io.specctra.DsnReader;
import app.freerouting.rules.BoardRules;
import app.freerouting.rules.ClearanceMatrix;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import java.io.BufferedWriter;
import java.io.ByteArrayInputStream;
import java.io.OutputStreamWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Paths;
import java.security.MessageDigest;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collection;
import java.util.Formatter;
import java.util.List;
import java.util.Set;

public final class IndexOracle {

  private IndexOracle() {}

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

    Json entryArray(Collection<ShapeTree.TreeEntry> entries) {
      sb.append('[');
      boolean first = true;
      for (ShapeTree.TreeEntry entry : entries) {
        if (!first) {
          sb.append(',');
        }
        first = false;
        sb.append('[')
            .append(((SearchTreeObject) entry.object).getId())
            .append(',')
            .append(entry.shapeIndexInObject)
            .append(']');
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
  // shape formatting (the spikes' exact-int fmt)
  // ---------------------------------------------------------------------

  static String fmtTile(RegularTileShape s) {
    if (s instanceof IntBox b) {
      return "box[" + b.ll.x + " " + b.ll.y + " " + b.ur.x + " " + b.ur.y + "]";
    }
    IntOctagon o = (IntOctagon) s;
    return "oct["
        + o.leftX
        + " "
        + o.bottomY
        + " "
        + o.rightX
        + " "
        + o.topY
        + " "
        + o.upperLeftDiagonalX
        + " "
        + o.lowerRightDiagonalX
        + " "
        + o.lowerLeftDiagonalX
        + " "
        + o.upperRightDiagonalX
        + "]";
  }

  /** `translate_by` of the Rust corpus: the TILE-level translate is
   * concrete-typed (the TileShape interface method returns a
   * PolylineShape); dispatch exactly like the Rust enum match. */
  static TileShape translate(TileShape s, int dx, int dy) {
    Vector v = Vector.getInstance(dx, dy);
    if (s instanceof IntBox b) {
      return b.translateBy(v);
    }
    if (s instanceof IntOctagon o) {
      return o.translateBy(v);
    }
    return ((Simplex) s).translateBy(v);
  }

  /** `fmt_shape` of the Rust corpus: tiles as-is, a simplex through its
   * bounding octagon (`tile[oct[...]]`). */
  static String fmtShape(Shape shape) {
    if (shape instanceof IntBox || shape instanceof IntOctagon) {
      return fmtTile((RegularTileShape) shape);
    }
    if (shape instanceof Simplex simplex) {
      IntOctagon oct = simplex.boundingOctagon();
      if (oct == null) {
        return null;
      }
      return "tile[" + fmtTile(oct) + "]";
    }
    return null;
  }

  // ---------------------------------------------------------------------
  // tree dump (MinAreaTree structure, byte-equal with the Rust port's
  // MinAreaTree::dump_lines)
  // ---------------------------------------------------------------------

  static List<String> dumpLines(ShapeSearchTree tree) {
    List<String> out = new ArrayList<>();
    if (tree.root != null) {
      dumpNode(tree.root, 0, out);
    }
    return out;
  }

  static void dumpNode(TreeNode node, int depth, List<String> out) {
    String indent = "    ".repeat(depth);
    if (node instanceof ShapeTree.Leaf leaf) {
      out.add(
          indent
              + "L obj="
              + ((SearchTreeObject) leaf.object).getId()
              + " idx="
              + leaf.shapeIndexInObject
              + " "
              + fmtTile(leaf.boundingShape));
      return;
    }
    ShapeTree.InnerNode inner = (ShapeTree.InnerNode) node;
    out.add(indent + "I " + fmtTile(inner.boundingShape));
    dumpNode(inner.firstChild, depth + 1, out);
    dumpNode(inner.secondChild, depth + 1, out);
  }

  static String sha256(List<String> lines) throws Exception {
    MessageDigest digest = MessageDigest.getInstance("SHA-256");
    for (String line : lines) {
      digest.update(line.getBytes(java.nio.charset.StandardCharsets.UTF_8));
      digest.update((byte) '\n');
    }
    StringBuilder hex = new StringBuilder();
    for (byte b : digest.digest()) {
      hex.append(String.format("%02x", b));
    }
    return hex.toString();
  }

  // ---------------------------------------------------------------------
  // the protocol
  // ---------------------------------------------------------------------

  /** One (tag, shape) of the per-tree query set. */
  record QShape(String tag, TileShape shape) {}

  record TreeDump(long n, String sha, List<String> head, List<String> lines) {}

  static TreeDump digest(List<String> lines) throws Exception {
    List<String> head = new ArrayList<>();
    for (int i = 0; i < Math.min(40, lines.size()); i++) {
      head.add(lines.get(i));
    }
    return new TreeDump(lines.size(), sha256(lines), head, lines);
  }

  static String evaluateFixture(String id, String path, byte[] bytes) throws Exception {
    IdGenerator idGenerator = new ItemIdGenerator();
    String name = Paths.get(path).getFileName().toString();
    BoardReadResult read =
        DsnReader.readBoard(new ByteArrayInputStream(bytes), null, idGenerator, name);
    if (!(read instanceof BoardReadResult.Success success)) {
      return new Json().raw("{\"id\":").str(id).raw(",\"file\":").str(path)
          .raw(",\"result\":\"read-failed\"}").sb.toString();
    }
    BasicBoard board = success.board();

    // Pre-steps + the uniform fill normalization.
    List<String> pre = new ArrayList<>();
    if (name.equals("drill-inflate.dsn")) {
      board.rules.setHoleClearance(2500);
      pre.add("setHoleClearance=2500");
    }
    board.searchTreeManager.reinsertTreeItems();

    // ---- facts -------------------------------------------------------
    Collection<Item> items = board.getItems();
    long itemCount = items.size();
    long layerCount = board.getLayerCount();
    ClearanceMatrix matrix = board.rules.clearanceMatrix;
    List<String> classes = new ArrayList<>();
    for (int i = 0; i < matrix.getClassCount(); i++) {
      String className = matrix.getName(i);
      classes.add(className == null ? "null" : className);
    }
    IntBox bbox = board.getBoundingBox();
    long[] bboxJson = null;
    if (bbox != null) {
      bboxJson = new long[] {bbox.ll.x, bbox.ll.y, bbox.ur.x, bbox.ur.y};
    }

    // T = first trace descending, else first netted item; N = first 3.
    Trace t = null;
    Item fallback = null;
    List<Integer> widths = new ArrayList<>();
    List<Item> nItems = new ArrayList<>();
    for (Item item : items) {
      if (t == null && item instanceof Trace trace) {
        t = trace;
      }
      if (item instanceof Trace trace) {
        widths.add(trace.getHalfWidth());
      }
      if (fallback == null && item.netNumbers.length > 0) {
        fallback = item;
      }
      if (nItems.size() < 3) {
        nItems.add(item);
      }
    }
    Item tItem;
    Long tLayer = null;
    Long tHw = null;
    Integer tNet = null;
    if (t != null) {
      tItem = t;
      tLayer = (long) t.getLayer();
      tHw = (long) t.getHalfWidth();
      for (int net : t.netNumbers) {
        if (net != 0) {
          tNet = net;
          break;
        }
      }
    } else if (fallback != null) {
      tItem = fallback;
      for (int net : fallback.netNumbers) {
        if (net != 0) {
          tNet = net;
          break;
        }
      }
    } else {
      tItem = null;
    }
    long hw = tHw != null ? tHw : 250;
    int[] sorted = new int[widths.size()];
    for (int i = 0; i < sorted.length; i++) {
      sorted[i] = widths.get(i);
    }
    Arrays.sort(sorted);
    long medianW = sorted.length > 0 ? sorted[sorted.length / 2] : 0;
    int netA = tNet != null ? tNet : 1;
    int netB = netA;
    outer:
    for (Item item : items) {
      for (int net : item.netNumbers) {
        if (net != 0 && net != netA) {
          netB = net;
          break outer;
        }
      }
    }
    String snap = snapOf(board.rules);
    long[] nIds = new long[nItems.size()];
    for (int i = 0; i < nIds.length; i++) {
      nIds[i] = nItems.get(i).getId();
    }

    Json facts =
        new Json()
            .raw("{\"items\":")
            .num(itemCount)
            .raw(",\"layers\":")
            .num(layerCount)
            .raw(",\"classes\":")
            .strArray(classes)
            .raw(",\"bbox\":");
    if (bboxJson == null) {
      facts.nul();
    } else {
      facts.longArray(bboxJson);
    }
    facts.raw(",\"t_id\":").numOrNull(tItem == null ? null : (long) tItem.getId());
    facts.raw(",\"t_kind\":").strOrNull(tItem == null ? null : (t != null ? "trace" : "item"));
    facts.raw(",\"t_layer\":").numOrNull(tLayer);
    facts.raw(",\"t_hw\":").numOrNull(tHw);
    facts.raw(",\"t_net\":").numOrNull(tNet == null ? null : (long) tNet);
    facts.raw(",\"n_ids\":").longArray(nIds);
    facts.raw(",\"hw\":").num(hw);
    facts.raw(",\"median_w\":").num(medianW);
    facts.raw(",\"net_a\":").num(netA);
    facts.raw(",\"net_b\":").num(netB);
    facts.raw(",\"snap\":").str(snap);
    facts.raw(",\"pre\":").strArray(pre);

    // ---- tree build sequence ------------------------------------------
    ShapeSearchTree cc0 = board.searchTreeManager.getDefaultTree();
    TreeDump d0 = digest(dumpLines(cc0));
    board.searchTreeManager.setClearanceCompensationUsed(true);
    ShapeSearchTree cc1 = board.searchTreeManager.getDefaultTree();
    TreeDump d1 = digest(dumpLines(cc1));
    ShapeSearchTree cc2 = board.searchTreeManager.getAutorouteTree(2);
    TreeDump d2 = digest(dumpLines(cc2));

    Json trees = new Json().raw("[");
    appendTree(trees, "cc0", cc0, d0);
    trees.raw(",");
    appendTree(trees, "cc1", cc1, d1);
    trees.raw(",");
    appendTree(trees, "cc2", cc2, d2);
    trees.raw("]");

    // ---- the per-tree query set ---------------------------------------
    ShapeSearchTree[] queryTrees = {cc1, cc2};
    List<List<QShape>> perTree = new ArrayList<>();
    for (ShapeSearchTree tree : queryTrees) {
      List<QShape> qs = new ArrayList<>();
      for (Item anchor : nItems) {
        int count = anchor.treeShapeCount(tree);
        TileShape own0 = null;
        TileShape own1 = null;
        for (int i = 0; i < count; i++) {
          TileShape shape = anchor.getTreeShape(tree, i);
          if (shape != null) {
            if (own0 == null) {
              own0 = shape;
            } else if (own1 == null) {
              own1 = shape;
              break;
            }
          }
        }
        if (own0 == null) {
          continue;
        }
        qs.add(new QShape("own0", own0));
        if (own1 != null) {
          qs.add(new QShape("own1", own1));
        }
        qs.add(new QShape("xlate+hw", translate(own0, (int) hw, 0)));
        qs.add(new QShape("xlate-hw", translate(own0, 0, (int) -hw)));
        break;
      }
      if (bbox != null) {
        int cx = bbox.ll.x + (bbox.ur.x - bbox.ll.x) / 2;
        int cy = bbox.ll.y + (bbox.ur.y - bbox.ll.y) / 2;
        int half = (int) medianW;
        qs.add(
            new QShape(
                "ctrbox",
                new IntBox(
                    new IntPoint(cx - half, cy - half), new IntPoint(cx + half, cy + half))));
      }
      perTree.add(qs);
    }
    int[][] igs = {
      {},
      {netA},
      {netB},
      {netA, netB},
      {0},
      {9999},
    };

    StringBuilder queries = new StringBuilder("[");
    boolean anyQuery = false;

    // Phase rmT.
    if (tItem != null) {
      board.searchTreeManager.remove(tItem);
      for (int ti = 0; ti < queryTrees.length; ti++) {
        anyQuery |=
            appendQueries(
                queries, board, queryTrees[ti], perTree.get(ti), "rmT", igs, anyQuery);
      }
    }

    Json sshapes = null;
    Json wc = null;
    Json ob = null;
    Json ic = null;
    Json cs = null;
    Json val = null;
    String result = "no-bbox";

    if (bbox != null) {
      result = "ok";
      int cx = bbox.ll.x + (bbox.ur.x - bbox.ll.x) / 2;
      int cy = bbox.ll.y + (bbox.ur.y - bbox.ll.y) / 2;
      int sLayer = tLayer != null ? tLayer.intValue() : 0;
      Polyline polyline =
          new Polyline(
              new Point[] {
                new IntPoint(cx - (int) hw, cy), new IntPoint(cx + (int) hw, cy)
              });
      PolylineTrace s =
          board.insertTraceWithoutCleaning(
              polyline, sLayer, (int) hw, new int[0], 1, FixedState.UNFIXED);

      // S's shapes per tree.
      sshapes = new Json().raw("[");
      for (int ti = 0; ti < queryTrees.length; ti++) {
        ShapeSearchTree tree = queryTrees[ti];
        if (ti > 0) {
          sshapes.raw(",");
        }
        sshapes.raw("{\"key\":").str(tree.key).raw(",\"shapes\":[");
        int count = s.treeShapeCount(tree);
        for (int i = 0; i < count; i++) {
          if (i > 0) {
            sshapes.raw(",");
          }
          TileShape shape = s.getTreeShape(tree, i);
          if (shape == null) {
            sshapes.nul();
          } else {
            String fmt = fmtShape(shape);
            if (fmt == null) {
              sshapes.nul();
            } else {
              sshapes.str(fmt);
            }
          }
        }
        sshapes.raw("]}");
      }
      sshapes.raw("]");

      // Phase insS.
      for (int ti = 0; ti < queryTrees.length; ti++) {
        anyQuery |=
            appendQueries(
                queries, board, queryTrees[ti], perTree.get(ti), "insS", igs, anyQuery);
      }

      // Phase rmS.
      board.searchTreeManager.remove(s);
      for (int ti = 0; ti < queryTrees.length; ti++) {
        anyQuery |=
            appendQueries(
                queries, board, queryTrees[ti], perTree.get(ti), "rmS", igs, anyQuery);
      }
      queries.append("]");

      // with-clearance core on cc1 at classes {0,1,2}.
      wc = new Json().raw("[");
      int[][] wcIgs = {{}, {netA}};
      boolean anyWc = false;
      for (QShape q : perTree.get(0)) {
        for (int cls = 0; cls <= 2; cls++) {
          for (int[] ig : wcIgs) {
            List<ShapeTree.TreeEntry> entries = new ArrayList<>();
            // sLayer (NOT -1): the clearance windows read the matrix's
            // per-layer row maxima and a negative layer zeroes them
            // (the T56 trap would degenerate to exact tests) — mirrors
            // plan.t_layer on the Rust side.
            cc1.overlappingTreeEntriesWithClearance(q.shape(), sLayer, ig, cls, entries);
            if (anyWc) {
              wc.raw(",");
            }
            anyWc = true;
            wc.raw("{\"key\":")
                .str(cc1.key)
                .raw(",\"tag\":")
                .str(q.tag())
                .raw(",\"ig\":")
                .longArray(Arrays.stream(ig).mapToLong(x -> x).toArray())
                .raw(",\"cls\":")
                .num(cls)
                .raw(",\"e\":")
                .entryArray(entries)
                .raw("}");
          }
        }
      }
      wc.raw("]");

      // objects/items family (the T8 surface): the ctrbox's OBJECTS on
      // each queried tree (layer -1 like the plain rows — the ids are
      // the entry query's objects DEDUPED and TreeSet-descending, not
      // entry order) and one DISPATCHING with-clearance ITEMS row on
      // cc1 at S's scripted class (cc1 carries compensation, so this
      // takes the plain branch — the wc rows above pin the 5-arg core,
      // this pins the dispatch). Mirrors the ob/ic sections of
      // evaluate_rust.
      int ccy = bbox.ll.y + (bbox.ur.y - bbox.ll.y) / 2;
      int half = (int) medianW;
      IntBox ctrbox =
          new IntBox(
              new IntPoint(cx - half, ccy - half), new IntPoint(cx + half, ccy + half));
      ob = new Json().raw("[");
      for (int ti = 0; ti < queryTrees.length; ti++) {
        if (ti > 0) {
          ob.raw(",");
        }
        Set<SearchTreeObject> objects = queryTrees[ti].overlappingObjects(ctrbox, -1);
        ob.raw("{\"key\":")
            .str(queryTrees[ti].key)
            .raw(",\"e\":")
            .longArray(objects.stream().mapToLong(SearchTreeObject::getId).toArray())
            .raw("}");
      }
      ob.raw("]");
      ic = new Json().raw("[");
      Set<Item> itemsOverlap = cc1.overlappingItemsWithClearance(ctrbox, sLayer, new int[0], 1);
      ic.raw("{\"key\":")
          .str(cc1.key)
          .raw(",\"e\":")
          .longArray(itemsOverlap.stream().mapToLong(Item::getId).toArray())
          .raw("}");
      ic.raw("]");

      // check_shape: the center box + a deliberately-outside box.
      cs = new Json().raw("[");
      IntBox outside =
          new IntBox(
              new IntPoint(bbox.ur.x + 10_000, bbox.ur.y + 10_000),
              new IntPoint(bbox.ur.x + 11_000, bbox.ur.y + 11_000));
      boolean anyCs = false;
      String[] csTags = {"ctrbox", "outside"};
      IntBox[] csShapes = {ctrbox, outside};
      int[][] csNets = {{}, {netA}};
      for (int i = 0; i < csTags.length; i++) {
        for (int[] nets : csNets) {
          // Same sLayer convention as wc.
          boolean ok = board.checkShape(csShapes[i], sLayer, nets, 1);
          if (anyCs) {
            cs.raw(",");
          }
          anyCs = true;
          cs.raw("{\"tag\":")
              .str(csTags[i])
              .raw(",\"nets\":")
              .longArray(Arrays.stream(nets).mapToLong(x -> x).toArray())
              .raw(",\"ok\":")
              .bool(ok)
              .raw("}");
        }
      }
      cs.raw("]");

      // validate_entries on the first 5 items with entries in cc1.
      val = new Json().raw("[");
      boolean anyVal = false;
      int validated = 0;
      for (Item item : items) {
        if (validated >= 5) {
          break;
        }
        if (item.getSearchTreeEntries(cc1) == null) {
          continue;
        }
        boolean ok = board.searchTreeManager.validateEntries(item);
        if (anyVal) {
          val.raw(",");
        }
        anyVal = true;
        val.raw("{\"id\":").num(item.getId()).raw(",\"ok\":").bool(ok).raw("}");
        validated++;
      }
      val.raw("]");

      // s_id LAST in facts (both sides): the scripted trace's
      // allocated id — the dsn-0151 id-burn drift detector (a future
      // fixture where the sides' generators disagree diverges HERE
      // instead of as a cryptic queries[i].e mismatch).
      facts.raw(",\"s_id\":").num(s.getId());
    } else {
      queries.append("]");
      facts.raw(",\"s_id\":").nul();
    }
    facts.raw("}");

    return new Json()
        .raw("{\"id\":")
        .str(id)
        .raw(",\"file\":")
        .str(path)
        .raw(",\"result\":")
        .str(result)
        .raw(",\"facts\":")
        .raw(facts.sb.toString())
        .raw(",\"trees\":")
        .raw(trees.sb.toString())
        .raw(",\"sshapes\":")
        .raw(sshapes == null ? "null" : sshapes.sb.toString())
        .raw(",\"queries\":")
        .raw(queries.toString())
        .raw(",\"wc\":")
        .raw(wc == null ? "null" : wc.sb.toString())
        .raw(",\"ob\":")
        .raw(ob == null ? "null" : ob.sb.toString())
        .raw(",\"ic\":")
        .raw(ic == null ? "null" : ic.sb.toString())
        .raw(",\"cs\":")
        .raw(cs == null ? "null" : cs.sb.toString())
        .raw(",\"val\":")
        .raw(val == null ? "null" : val.sb.toString())
        .raw("}")
        .sb.toString();
  }

  static String snapOf(BoardRules rules) {
    String name = rules.getTraceAngleRestriction().name();
    return switch (name) {
      case "FORTYFIVE_DEGREE" -> "45";
      case "NINETY_DEGREE" -> "90";
      default -> "none";
    };
  }

  static void appendTree(Json out, String phase, ShapeSearchTree tree, TreeDump dump) {
    out.raw("{\"phase\":")
        .str(phase)
        .raw(",\"key\":")
        .str(tree.key)
        .raw(",\"flag\":")
        .bool(tree.isClearanceCompensationUsed())
        .raw(",\"n\":")
        .num(dump.n())
        .raw(",\"sha256\":")
        .str(dump.sha())
        .raw(",\"head\":")
        .strArray(dump.head())
        .raw(",\"lines\":");
    if (dump.n() <= 200) {
      out.strArray(dump.lines());
    } else {
      out.nul();
    }
    out.raw("}");
  }

  /** Appends the tag × ignore-list rows of one (tree, phase) block. */
  static boolean appendQueries(
      StringBuilder queries,
      BasicBoard board,
      ShapeSearchTree tree,
      List<QShape> qshapes,
      String phase,
      int[][] igs,
      boolean any) {
    for (QShape q : qshapes) {
      for (int[] ig : igs) {
        List<ShapeTree.TreeEntry> entries = new ArrayList<>();
        tree.overlappingTreeEntries(q.shape(), -1, ig, entries);
        if (any) {
          queries.append(',');
        }
        any = true;
        queries.append("{\"phase\":\"").append(phase).append("\",\"key\":\"");
        queries.append(Json.esc(tree.key));
        queries.append("\",\"tag\":\"").append(q.tag()).append("\",\"ig\":");
        long[] igJson = new long[ig.length];
        for (int i = 0; i < ig.length; i++) {
          igJson[i] = ig[i];
        }
        queries.append(new Json().longArray(igJson).sb);
        queries.append(",\"e\":");
        queries.append(new Json().entryArray(entries).sb);
        queries.append('}');
      }
    }
    return any;
  }

  // ---------------------------------------------------------------------
  // entry
  // ---------------------------------------------------------------------

  public static void main(String[] args) throws Exception {
    if (args.length != 1) {
      System.err.println("usage: IndexOracle <manifest.jsonl>");
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
                + "\",\"result\":\"EvalError\",\"facts\":null,\"trees\":null,"
                + "\"sshapes\":null,\"queries\":null,\"wc\":null,\"ob\":null,\"ic\":null,"
                + "\"cs\":null,\"val\":null}";
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
