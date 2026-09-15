// CombineSpike.java — jar spike for M2 Task 11 (the trace COMBINE seam:
// PolylineTrace.combine / combineAtStart / combineAtEnd,
// PolylineTrace.java:175-456, and the cycle-removal seam
// BasicBoard.removeIfCycle / getTraceTail, BasicBoard.java:1307-1366,
// with Trace.isCycle + Item.isCycleRecu / getConnectionItems).
//
// Lives OUTSIDE src/ (harness-side; the frozen Java tree is never
// touched). Declares the app.freerouting.datastructures package —
// unlike ContactsSpike — because the default-tree dump reads
// ShapeTree.TreeNode's protected firstChild/secondChild fields (the
// IndexOracle dumpLines pattern); everything else it touches is
// public.
//
// Run (JDK 25, from the repo root; the sed strips FRLogger's leading
// run timestamp from the degenerate-input WARN lines so two runs
// diff clean — the WARN TEXT itself is deterministic):
//   mkdir -p /tmp/epic-t11-classes && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/javac -cp build/libs/freerouting-current-executable.jar \
//       -d /tmp/epic-t11-classes rust/harness/oracle/CombineSpike.java && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/java -cp build/libs/freerouting-current-executable.jar:/tmp/epic-t11-classes \
//       app.freerouting.datastructures.CombineSpike \
//       | sed -E 's/^[0-9]{4}-[0-9]{2}-[0-9]{2} [0-9]{2}:[0-9]{2}:[0-9]{2}\.[0-9]+ //' \
//       | tee /tmp/epic-t11-combine.out
//   # and the pure-square walk probe (terminates; see the Y1 analysis
//   # at the case):
//   ~/.jdks/jdk-25.0.4.1+1/bin/java -cp build/libs/freerouting-current-executable.jar:/tmp/epic-t11-classes \
//       app.freerouting.datastructures.CombineSpike hangprobe 2>&1 | tee /tmp/epic-t11-combine-hang.out
//
// Every case parses a FRESH copy of the PURE board (the ContactsSpike
// board whose parse state normalizeAllTraces leaves untouched: no
// bent wire, no cycle at read time), then builds the trap geometry
// with insertTraceWithoutCleaning — which never normalizes — so ids
// are deterministic: parse ids 1..9 (1 outline, 2 keepout, 3 pin,
// 4 trace E, 5 via, 6 conduction AREA, 7 F, 8 F2, 9 G), inserts
// continue at 10 in each case's own insertion order.
//
// Sections (output line prefixes):
//   B    — the untouched parse baseline: item dump + default-tree dump
//          (the common BEFORE state of every case).
//   C1..C7 — the combine basics: collinear merge, L merge, reverse
//          merge, chain-of-3 in one combine() loop, both-ends (start
//          tried first), two-contact refusal, zero-contact refusal.
//   R1..R6 — refusals and the equality traps: width mismatch, layer
//          mismatch (manifests as zero contacts), fixed-state
//          mismatch, same-fixed COMBINES (equality semantics),
//          USER_FIXED refusal (deletion-forbidden), and the area
//          strip (raw vs stripped contact lists).
//   D1..D3 — degenerate joins: partial overlap (truncation), U-turn
//          through the join corner (the replaceGeometry witness),
//          closed ring (merge without cap loss).
//   Y2..Y7 — cycles: square + pendant (removeIfCycle removes ONLY the
//          two square traces T1/T2 via the connection-items walk; the
//          pendant pair and T5 survive — the capture's POST dump),
//          via triangle (isCycle FALSE — the seed blocks the DFS),
//          tail positive rows, non-cycle refusal, overlap cycle
//          (connectionItems = self only; the tail walk eats the
//          partner).
//   Y1   — the pure square, in the hangprobe pass ONLY. isCycle is
//          true. getConnectionItems TERMINATES (the fork-detection
//          break, not a visited set, ends the walk) and returns a
//          3-item set; the PROBE rows dump each square trace's stored
//          corners, contact sets, and the pairwise
//          firstCommonLayer/normalContactPoint values, so the port
//          can reproduce the walk's input state exactly.
//   G    — the insertTraceWithoutCleaning guards: 1-corner input (no
//          id burned), doubled-back input (polygon collapse, still no
//          burn), closed ring UNFIXED (refused AFTER the ctor — the id
//          IS burned, shown by the witness gap) vs USER_FIXED
//          (inserts), and the BasicBoard half-width accumulators vs
//          the BoardRules getters.
//   PINWALK — the isRoutable KIND GATE (fix round): a pin is never
//          routable (base-class false; only Trace/Via override), so
//          the connection walk EXCLUDES the pin and STOPS there,
//          while a via IS routable and lands in the result set.
//   DEGEN — the single-segment join probe (fix round): two
//          single-segment traces joined carry 3 stored lines
//          (corners+1) and MERGE — ONE survivor. SPIKE-VS-RECON
//          CORRECTION: the review's both-removed prediction was wrong.
//   DEGEN2 — the TRUE shrink-rule collapse (fix round): B retraces A
//          exactly, the joined ctor cleans to the EMPTY polyline
//          (joinedLen=0 < 3) → the receiver AND the other trace are
//          removed, the tree back to the 31-row baseline, both ids
//          burned.
//   DEGEN3 — the overrun boundary (fix round): the retrace extends
//          past A's first corner — 3 real lines survive the ctor, the
//          merge runs and A lives on shrunk to the overrun.
//   *_TREE — post-combine default-tree dumps for C4/C5/D1/D3 (fix
//          round MINOR 2; setups identical to the original cases).
//
// BRANCH_PRED rows mirror combineAtStart/End's join arithmetic from
// PUBLIC inputs (polyline().lines, getSearchTreeEntries) and call the
// REAL canonicalizing Polyline(Line[]) ctor, so the printed
// merge/replace prediction is the oracle's own arithmetic, never a
// hand simulation.
//
// Output discipline (project pin rules): exact ints via field access,
// literal capture rows only, TreeSet iteration orders preserved.
package app.freerouting.datastructures;

import app.freerouting.board.actions.ItemIdGenerator;
import app.freerouting.board.facade.BasicBoard;
import app.freerouting.board.searchtree.SearchTreeObject;
import app.freerouting.board.searchtree.ShapeSearchTree;
import app.freerouting.board.model.items.ConductionArea;
import app.freerouting.board.model.items.DrillItem;
import app.freerouting.board.model.items.Item;
import app.freerouting.board.model.items.ObstacleArea;
import app.freerouting.board.model.items.Pin;
import app.freerouting.board.model.items.Trace;
import app.freerouting.board.model.items.Via;
import app.freerouting.board.model.structure.BoardOutline;
import app.freerouting.board.model.structure.FixedState;
import app.freerouting.board.trace.PolylineTrace;
import app.freerouting.geometry.planar.IntBox;
import app.freerouting.geometry.planar.IntPoint;
import app.freerouting.geometry.planar.Line;
import app.freerouting.geometry.planar.Point;
import app.freerouting.geometry.planar.Polyline;
import app.freerouting.geometry.planar.RegularTileShape;
import app.freerouting.geometry.planar.TileShape;
import app.freerouting.io.BoardReadResult;
import app.freerouting.io.specctra.DsnReader;
import java.io.ByteArrayInputStream;
import java.util.ArrayList;
import java.util.Collection;
import java.util.List;
import java.util.Set;

public final class CombineSpike {

  private CombineSpike() {}

  // ---------------------------------------------------------------------
  // helpers
  // ---------------------------------------------------------------------

  static String kind(Item item) {
    if (item instanceof PolylineTrace) {
      return "T";
    }
    if (item instanceof app.freerouting.board.model.items.Pin) {
      return "P";
    }
    if (item instanceof app.freerouting.board.model.items.Via) {
      return "V";
    }
    if (item instanceof ConductionArea) {
      return "A";
    }
    if (item instanceof BoardOutline) {
      return "BO";
    }
    if (item instanceof app.freerouting.board.model.items.ComponentOutline) {
      return "CO";
    }
    if (item instanceof ObstacleArea) {
      return "K";
    }
    return "?";
  }

  static String items(Collection<Item> set) {
    StringBuilder sb = new StringBuilder("[");
    boolean first = true;
    for (Item item : set) {
      if (!first) {
        sb.append(",");
      }
      first = false;
      sb.append(item.getId()).append(":").append(kind(item));
    }
    return sb.append("]").toString();
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

  static String cornersOfPoly(Polyline poly) {
    StringBuilder sb = new StringBuilder("[");
    Point[] cs = poly.corners();
    for (int i = 0; i < cs.length; i++) {
      if (i > 0) {
        sb.append(" ");
      }
      sb.append(xy(cs[i]));
    }
    return sb.append("]").toString();
  }

  static String cornersOf(PolylineTrace trace) {
    return cornersOfPoly(trace.polyline());
  }

  static void dumpItems(BasicBoard board, String prefix) {
    List<Item> list = new ArrayList<>(board.getItems());
    list.sort((a, b) -> b.getId() - a.getId());
    for (Item item : list) {
      String geometry;
      if (item instanceof PolylineTrace trace) {
        geometry = "corners=" + cornersOf(trace);
      } else if (item instanceof DrillItem drill) {
        geometry = "center=" + xy(drill.getCenter())
            + " span=" + drill.firstLayer() + ".." + drill.lastLayer();
      } else if (item instanceof ObstacleArea area) {
        IntBox bb = (IntBox) area.getArea().boundingBox();
        geometry = "bbox=[" + bb.ll.x + " " + bb.ll.y + " " + bb.ur.x + " " + bb.ur.y + "]";
      } else {
        geometry = "-";
      }
      System.out.println(prefix + "_ITEM id=" + item.getId()
          + " kind=" + item.getClass().getSimpleName()
          + " layer=" + item.firstLayer() + " fixed=" + item.getFixedState()
          + " " + geometry);
    }
  }

  /** The contacts of one trace endpoint, one-letter annotated. */
  static String endContacts(PolylineTrace trace, boolean atStart) {
    Point p = atStart ? trace.firstCorner() : trace.lastCorner();
    return items(trace.getNormalContacts(p, false));
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

  /** The trace template: parse trace E (id 4, F.Cu, halfWidth 250, net MINE=[1]). */
  static PolylineTrace tmpl(BasicBoard board) {
    for (Item item : board.getItems()) {
      if (item instanceof PolylineTrace trace && trace.getId() == 4) {
        return trace;
      }
    }
    return null;
  }

  static PolylineTrace ins(BasicBoard board, PolylineTrace tmpl, Point[] corners) {
    return board.insertTraceWithoutCleaning(new Polyline(corners), tmpl.getLayer(),
        tmpl.getHalfWidth(), tmpl.netNumbers, tmpl.clearanceClassIndex(), FixedState.UNFIXED);
  }

  static PolylineTrace insFixed(BasicBoard board, PolylineTrace tmpl, Point[] corners,
      FixedState fixed) {
    return board.insertTraceWithoutCleaning(new Polyline(corners), tmpl.getLayer(),
        tmpl.getHalfWidth(), tmpl.netNumbers, tmpl.clearanceClassIndex(), fixed);
  }

  static PolylineTrace insWidth(BasicBoard board, PolylineTrace tmpl, Point[] corners,
      int halfWidth) {
    return board.insertTraceWithoutCleaning(new Polyline(corners), tmpl.getLayer(),
        halfWidth, tmpl.netNumbers, tmpl.clearanceClassIndex(), FixedState.UNFIXED);
  }

  static PolylineTrace insLayer(BasicBoard board, PolylineTrace tmpl, Point[] corners,
      int layer) {
    return board.insertTraceWithoutCleaning(new Polyline(corners), layer,
        tmpl.getHalfWidth(), tmpl.netNumbers, tmpl.clearanceClassIndex(), FixedState.UNFIXED);
  }

  /** insNet: an insert carrying ANOTHER item's net numbers (the
   * pin-routability case needs an OTHER-net trace). */
  static PolylineTrace insNet(BasicBoard board, PolylineTrace tmpl, Point[] corners,
      int[] nets) {
    return board.insertTraceWithoutCleaning(new Polyline(corners), tmpl.getLayer(),
        tmpl.getHalfWidth(), nets, tmpl.clearanceClassIndex(), FixedState.UNFIXED);
  }

  // ---------------------------------------------------------------------
  // the tree dump (IndexOracle's byte-equal MinAreaTree format)
  // ---------------------------------------------------------------------

  static List<String> dumpLines(ShapeSearchTree tree) {
    List<String> out = new ArrayList<>();
    if (tree.root != null) {
      dumpNode(tree.root, 0, out);
    }
    return out;
  }

  static void dumpNode(ShapeTree.TreeNode node, int depth, List<String> out) {
    String indent = "    ".repeat(depth);
    if (node instanceof ShapeTree.Leaf leaf) {
      out.add(indent + "L obj=" + ((SearchTreeObject) leaf.object).getId()
          + " idx=" + leaf.shapeIndexInObject + " " + fmtTile(leaf.boundingShape));
      return;
    }
    ShapeTree.InnerNode inner = (ShapeTree.InnerNode) node;
    out.add(indent + "I " + fmtTile(inner.boundingShape));
    dumpNode(inner.firstChild, depth + 1, out);
    dumpNode(inner.secondChild, depth + 1, out);
  }

  static String fmtTile(RegularTileShape s) {
    if (s instanceof app.freerouting.geometry.planar.IntBox b) {
      return "box[" + b.ll.x + " " + b.ll.y + " " + b.ur.x + " " + b.ur.y + "]";
    }
    app.freerouting.geometry.planar.IntOctagon o = (app.freerouting.geometry.planar.IntOctagon) s;
    return "oct[" + o.leftX + " " + o.bottomY + " " + o.rightX + " " + o.topY
        + " " + o.upperLeftDiagonalX + " " + o.lowerRightDiagonalX
        + " " + o.lowerLeftDiagonalX + " " + o.upperRightDiagonalX + "]";
  }

  /** Leaf count + full row dump under the given prefix. */
  static void dumpTree(BasicBoard board, String prefix) {
    List<String> lines = dumpLines(board.searchTreeManager.getDefaultTree());
    System.out.println(prefix + "_TREE lines=" + lines.size());
    for (String line : lines) {
      System.out.println(prefix + "_TREE_ROW " + line);
    }
  }

  /** Leaf count only (the before-state of refusal cases). */
  static void treeCount(BasicBoard board, String prefix) {
    System.out.println(prefix + "_TREE lines=" + dumpLines(
        board.searchTreeManager.getDefaultTree()).size());
  }

  // ---------------------------------------------------------------------
  // the branch predictor (the oracle's own arithmetic, public inputs)
  // ---------------------------------------------------------------------

  /**
   * Mirrors combineAtStart/combineAtEnd's join: reverseOrder detection,
   * the reversed+opposited otherLines, skipLine (POSITIONAL collinearity,
   * Line.isEqualOrOpposite), the two arraycopies, and the canonicalizing
   * ctor — then classifies MERGE_ENTRIES vs REPLACE_GEOMETRY from
   * (joinedLen != newCount || !hasDefaultEntries). Prints the survivor
   * corner list the ctor produces.
   */
  static String branchPred(BasicBoard board, PolylineTrace a, PolylineTrace b, boolean atStart) {
    Line[] aL = a.polyline().lines;
    Line[] bL = b.polyline().lines;
    boolean reverseOrder;
    if (atStart) {
      reverseOrder = a.firstCorner().equals(b.firstCorner());
    } else {
      reverseOrder = a.lastCorner().equals(b.lastCorner());
    }
    Line[] o;
    if (reverseOrder) {
      o = new Line[bL.length];
      for (int i = 0; i < o.length; i++) {
        o[i] = bL[o.length - 1 - i].opposite();
      }
    } else {
      o = bL;
    }
    boolean skipLine = atStart
        ? o[o.length - 2].isEqualOrOpposite(aL[1])
        : aL[aL.length - 2].isEqualOrOpposite(o[1]);
    int newCount = aL.length + o.length - 2;
    if (skipLine) {
      --newCount;
    }
    Line[] newLines = new Line[newCount];
    int joinPos;
    if (atStart) {
      System.arraycopy(o, 0, newLines, 0, o.length - 1);
      joinPos = o.length - 1;
      if (skipLine) {
        --joinPos;
      }
      System.arraycopy(aL, 1, newLines, joinPos, aL.length - 1);
    } else {
      System.arraycopy(aL, 0, newLines, 0, aL.length - 1);
      joinPos = aL.length - 1;
      if (skipLine) {
        --joinPos;
      }
      System.arraycopy(o, 1, newLines, joinPos, o.length - 1);
    }
    Polyline joined = new Polyline(newLines);
    boolean hasEntries =
        a.getSearchTreeEntries(board.searchTreeManager.getDefaultTree()) != null
            && b.getSearchTreeEntries(board.searchTreeManager.getDefaultTree()) != null;
    String branch =
        (joined.lines.length != newCount || !hasEntries) ? "REPLACE_GEOMETRY" : "MERGE_ENTRIES";
    StringBuilder sb = new StringBuilder("branch=").append(branch)
        .append(" atStart=").append(atStart)
        .append(" reverse=").append(reverseOrder)
        .append(" skipLine=").append(skipLine)
        .append(" newCount=").append(newCount)
        .append(" joinedLen=").append(joined.lines.length)
        .append(" joinedCorners=").append(joined.lines.length < 2
            ? "NONE" : joined.cornerCount())
        .append(" hasEntries=").append(hasEntries);
    if (joined.lines.length >= 2) {
      sb.append(" survivor=[");
      Point[] cs = joined.corners();
      for (int i = 0; i < cs.length; i++) {
        if (i > 0) {
          sb.append(" ");
        }
        sb.append(xy(cs[i]));
      }
      sb.append("]");
    } else {
      sb.append(" survivor=EMPTY");
    }
    return sb.toString();
  }

  // ---------------------------------------------------------------------
  // the board (ContactsSpike's PURE_DSN, unchanged)
  // ---------------------------------------------------------------------

  static final String PURE_DSN =
      "(pcb t11-pure.dsn\n"
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
    return parseDsnBytes(PURE_DSN.getBytes(), "t11-pure.dsn");
  }

  /** The parse-id witness row every case prints first. */
  static void ids(BasicBoard board, String prefix) {
    PolylineTrace t = tmpl(board);
    System.out.println(prefix + "_IDS tmpl=" + (t == null ? "?" : t.getId())
        + " count=" + board.getItems().size());
  }

  // ---------------------------------------------------------------------
  // baseline
  // ---------------------------------------------------------------------

  static void sectionB() {
    BasicBoard board = fresh();
    if (board == null) {
      return;
    }
    System.out.println("--B-- parse baseline (the common BEFORE state)");
    ids(board, "B");
    dumpItems(board, "B");
    dumpTree(board, "B");
  }

  // ---------------------------------------------------------------------
  // C: the combine basics
  // ---------------------------------------------------------------------

  static void caseC1() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = ins(board, t, new Point[] {ip(10000, 20000), ip(30000, 20000)});
    PolylineTrace b = ins(board, t, new Point[] {ip(30000, 20000), ip(50000, 20000)});
    System.out.println("--C1-- collinear end-join A=" + a.getId() + " B=" + b.getId());
    treeCount(board, "C1_PRE");
    System.out.println("C1_PRED " + branchPred(board, a, b, false));
    System.out.println("C1_COMBINE " + a.combine());
    System.out.println("C1_A_ALIVE " + a.isOnTheBoard() + " corners=" + cornersOf(a));
    dumpItems(board, "C1_POST");
    dumpTree(board, "C1_POST");
    PolylineTrace w = ins(board, t, new Point[] {ip(60000, 20000), ip(65000, 20000)});
    System.out.println("C1_WITNESS_ID " + w.getId() + " (no id reuse)");
  }

  static void caseC2() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = ins(board, t, new Point[] {ip(10000, 30000), ip(30000, 30000)});
    PolylineTrace b = ins(board, t, new Point[] {ip(30000, 30000), ip(30000, 50000)});
    System.out.println("--C2-- L end-join A=" + a.getId() + " B=" + b.getId());
    treeCount(board, "C2_PRE");
    System.out.println("C2_PRED " + branchPred(board, a, b, false));
    System.out.println("C2_COMBINE " + a.combine());
    System.out.println("C2_A_ALIVE " + a.isOnTheBoard() + " corners=" + cornersOf(a));
    dumpItems(board, "C2_POST");
    dumpTree(board, "C2_POST");
  }

  static void caseC3() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = ins(board, t, new Point[] {ip(30000, 40000), ip(10000, 40000)});
    PolylineTrace b = ins(board, t, new Point[] {ip(30000, 40000), ip(50000, 40000)});
    System.out.println("--C3-- reverse start-join A=" + a.getId() + " B=" + b.getId());
    System.out.println("C3_E_CORNERS e=" + t.getId() + " " + cornersOf(t)
        + " (the parse trace A's combine() loop meets in iteration 2)");
    System.out.println("C3_A_AFTER_INSERT " + cornersOf(a)
        + " (verbatim-stored: no insert-time clipping)");
    System.out.println("C3_B_AFTER_INSERT " + cornersOf(b));
    treeCount(board, "C3_PRE");
    System.out.println("C3_PRED_START " + branchPred(board, a, b, true));
    System.out.println("C3_COMBINE " + a.combine());
    System.out.println("C3_A_ALIVE " + a.isOnTheBoard() + " corners=" + cornersOf(a)
        + " (the loop made TWO joins: B at the start, then E at the new end)");
    dumpItems(board, "C3_POST");
    dumpTree(board, "C3_POST");
  }

  static void caseC4() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = ins(board, t, new Point[] {ip(10000, 50000), ip(30000, 50000)});
    PolylineTrace b = ins(board, t, new Point[] {ip(30000, 50000), ip(50000, 50000)});
    PolylineTrace c = ins(board, t, new Point[] {ip(50000, 50000), ip(70000, 50000)});
    System.out.println("--C4-- chain of 3 A=" + a.getId() + " B=" + b.getId()
        + " C=" + c.getId() + " (one combine() folds both)");
    System.out.println("C4_PRED_AB " + branchPred(board, a, b, false));
    System.out.println("C4_COMBINE " + a.combine());
    System.out.println("C4_A_ALIVE " + a.isOnTheBoard() + " corners=" + cornersOf(a));
    dumpItems(board, "C4_POST");
  }

  static void caseC5() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = ins(board, t, new Point[] {ip(30000, 20000), ip(50000, 20000)});
    PolylineTrace b1 = ins(board, t, new Point[] {ip(10000, 20000), ip(30000, 20000)});
    PolylineTrace b2 = ins(board, t, new Point[] {ip(50000, 20000), ip(70000, 20000)});
    System.out.println("--C5-- both ends A=" + a.getId() + " B1=" + b1.getId()
        + " B2=" + b2.getId() + " (start tried first, then end)");
    System.out.println("C5_PRED_START " + branchPred(board, a, b1, true));
    System.out.println("C5_PRED_END " + branchPred(board, a, b2, false));
    System.out.println("C5_COMBINE " + a.combine());
    System.out.println("C5_A_ALIVE " + a.isOnTheBoard() + " corners=" + cornersOf(a));
    dumpItems(board, "C5_POST");
  }

  static void caseC6() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = ins(board, t, new Point[] {ip(20000, 20000), ip(30000, 20000)});
    PolylineTrace b1 = ins(board, t, new Point[] {ip(30000, 20000), ip(40000, 20000)});
    PolylineTrace b2 = ins(board, t, new Point[] {ip(30000, 20000), ip(30000, 30000)});
    System.out.println("--C6-- two-contact refusal A=" + a.getId() + " B1=" + b1.getId()
        + " B2=" + b2.getId());
    System.out.println("C6_END_CONTACTS " + endContacts(a, false));
    System.out.println("C6_COMBINE " + a.combine());
    dumpItems(board, "C6_POST");
  }

  static void caseC7() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = ins(board, t, new Point[] {ip(20000, 20000), ip(30000, 20000)});
    System.out.println("--C7-- zero-contact refusal A=" + a.getId());
    System.out.println("C7_START_CONTACTS " + endContacts(a, true)
        + " END_CONTACTS " + endContacts(a, false));
    System.out.println("C7_COMBINE " + a.combine());
    System.out.println("C7_A_ALIVE " + a.isOnTheBoard() + " corners=" + cornersOf(a));
  }

  // ---------------------------------------------------------------------
  // R: refusals and equality traps
  // ---------------------------------------------------------------------

  static void caseR1() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = ins(board, t, new Point[] {ip(10000, 20000), ip(30000, 20000)});
    PolylineTrace b = insWidth(board, t, new Point[] {ip(30000, 20000), ip(50000, 20000)}, 500);
    System.out.println("--R1-- width mismatch A=" + a.getId() + " (hw 250) B=" + b.getId()
        + " (hw 500)");
    System.out.println("R1_CONTACTS " + endContacts(a, false));
    System.out.println("R1_COMBINE " + a.combine());
    System.out.println("R1_A_ALIVE " + a.isOnTheBoard() + " corners=" + cornersOf(a)
        + " B_ALIVE " + b.isOnTheBoard());
  }

  static void caseR2() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = ins(board, t, new Point[] {ip(10000, 20000), ip(30000, 20000)});
    PolylineTrace b = insLayer(board, t, new Point[] {ip(30000, 20000), ip(50000, 20000)}, 1);
    System.out.println("--R2-- layer mismatch A=" + a.getId() + " (F.Cu) B=" + b.getId()
        + " (B.Cu) — manifests as zero contacts");
    System.out.println("R2_A_END_CONTACTS " + endContacts(a, false)
        + " B_START_CONTACTS " + endContacts(b, true));
    System.out.println("R2_COMBINE " + a.combine() + " " + b.combine());
    System.out.println("R2_A_ALIVE " + a.isOnTheBoard() + " B_ALIVE " + b.isOnTheBoard());
    // Quality-round probe (MINOR 3b): the disjoint-interval -1 branch
    // of firstCommonLayer — the ONLY cross-layer pair on this board.
    System.out.println("R2_FCL_AB " + a.firstCommonLayer(b));
  }

  static void caseR3() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = ins(board, t, new Point[] {ip(10000, 20000), ip(30000, 20000)});
    PolylineTrace b = insFixed(board, t, new Point[] {ip(30000, 20000), ip(50000, 20000)},
        FixedState.SHOVE_FIXED);
    System.out.println("--R3-- fixed mismatch A=" + a.getId() + " (UNFIXED) B=" + b.getId()
        + " (SHOVE_FIXED)");
    System.out.println("R3_CONTACTS " + endContacts(a, false));
    System.out.println("R3_COMBINE " + a.combine());
    System.out.println("R3_A_ALIVE " + a.isOnTheBoard() + " B_ALIVE " + b.isOnTheBoard());
    // Quality-round probe (MINOR 3d): SHOVE_FIXED does NOT count as
    // user-fixed and is not deletion-forbidden — a port mutant using
    // >= SHOVE_FIXED would flip the first field.
    System.out.println("R3_B_USER_FIXED " + b.isUserFixed() + " B_FORBIDDEN "
        + b.isDeletionForbidden());
  }

  static void caseR4() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = insFixed(board, t, new Point[] {ip(10000, 20000), ip(30000, 20000)},
        FixedState.SHOVE_FIXED);
    PolylineTrace b = insFixed(board, t, new Point[] {ip(30000, 20000), ip(50000, 20000)},
        FixedState.SHOVE_FIXED);
    System.out.println("--R4-- same-fixed COMBINES A=" + a.getId() + " B=" + b.getId()
        + " (equality, not forbiddenness)");
    System.out.println("R4_PRED " + branchPred(board, a, b, false));
    System.out.println("R4_COMBINE " + a.combine());
    System.out.println("R4_A_ALIVE " + a.isOnTheBoard() + " corners=" + cornersOf(a)
        + " B_ALIVE " + b.isOnTheBoard());
  }

  static void caseR5() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = ins(board, t, new Point[] {ip(10000, 20000), ip(30000, 20000)});
    PolylineTrace b = insFixed(board, t, new Point[] {ip(30000, 20000), ip(50000, 20000)},
        FixedState.USER_FIXED);
    System.out.println("--R5-- USER_FIXED refusal A=" + a.getId() + " B=" + b.getId()
        + " (fixed mismatch AND deletion-forbidden)");
    System.out.println("R5_CONTACTS " + endContacts(a, false)
        + " bForbidden=" + b.isDeletionForbidden());
    System.out.println("R5_COMBINE " + a.combine());
    System.out.println("R5_A_ALIVE " + a.isOnTheBoard() + " B_ALIVE " + b.isOnTheBoard());
  }

  static void caseR6() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace f = null;
    for (Item item : board.getItems()) {
      if (item instanceof PolylineTrace trace && trace.getId() == 7) {
        f = trace;
      }
    }
    ConductionArea area = null;
    for (Item item : board.getItems()) {
      if (item instanceof ConductionArea ca) {
        area = ca;
      }
    }
    PolylineTrace p = ins(board, t, new Point[] {ip(55000, 15000), ip(55000, 5000)});
    System.out.println("--R6-- area strip F=" + f.getId() + " AREA=" + area.getId()
        + " P=" + p.getId());
    System.out.println("R6_F_START_RAW "
        + items(f.getNormalContacts(f.firstCorner(), false))
        + " (raw; the AREA must be stripped by ignoreAreas)");
    System.out.println("R6_F_START contact_size="
        + f.getNormalContacts(f.firstCorner(), false).size());
    System.out.println("R6_PRED_START " + branchPred(board, f, p, true));
    System.out.println("R6_COMBINE " + f.combine());
    System.out.println("R6_F_ALIVE " + f.isOnTheBoard() + " corners=" + cornersOf(f));
    System.out.println("R6_P_ALIVE " + p.isOnTheBoard());
    dumpItems(board, "R6_POST");
    dumpTree(board, "R6_POST");
  }

  // ---------------------------------------------------------------------
  // D: degenerate joins
  // ---------------------------------------------------------------------

  static void caseD1() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = ins(board, t, new Point[] {ip(10000, 10000), ip(30000, 10000)});
    PolylineTrace b = ins(board, t, new Point[] {ip(30000, 10000), ip(20000, 10000)});
    System.out.println("--D1-- partial overlap A=" + a.getId() + " B=" + b.getId()
        + " (B doubles back halfway)");
    System.out.println("D1_PRED " + branchPred(board, a, b, false));
    System.out.println("D1_COMBINE " + a.combine());
    System.out.println("D1_A_ALIVE " + a.isOnTheBoard() + " corners=" + cornersOf(a));
    System.out.println("D1_B_ALIVE " + b.isOnTheBoard());
    dumpItems(board, "D1_POST");
  }

  static void caseD2() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = ins(board, t, new Point[] {ip(10000, 10000), ip(30000, 10000),
      ip(30000, 30000)});
    dumpTree(board, "D2_AFTER_A");
    PolylineTrace b = ins(board, t, new Point[] {ip(30000, 30000), ip(30000, 10000)});
    System.out.println("--D2-- U-turn A=" + a.getId() + " B=" + b.getId()
        + " (B returns along A's first leg — the replaceGeometry witness)");
    dumpTree(board, "D2_AFTER_AB");
    System.out.println("D2_PRED " + branchPred(board, a, b, false));
    System.out.println("D2_COMBINE " + a.combine());
    System.out.println("D2_A_ALIVE " + a.isOnTheBoard()
        + (a.isOnTheBoard() ? " corners=" + cornersOf(a) : " (removed)"));
    System.out.println("D2_B_ALIVE " + b.isOnTheBoard());
    dumpItems(board, "D2_POST");
    dumpTree(board, "D2_POST");
  }

  static void caseD3() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = ins(board, t, new Point[] {ip(10000, 10000), ip(30000, 10000),
      ip(30000, 30000), ip(10000, 30000)});
    PolylineTrace b = ins(board, t, new Point[] {ip(10000, 30000), ip(10000, 10000)});
    System.out.println("--D3-- closed ring A=" + a.getId() + " B=" + b.getId());
    System.out.println("D3_PRED " + branchPred(board, a, b, false));
    System.out.println("D3_COMBINE " + a.combine());
    System.out.println("D3_A_ALIVE " + a.isOnTheBoard() + " corners=" + cornersOf(a));
    System.out.println("D3_B_ALIVE " + b.isOnTheBoard());
    dumpItems(board, "D3_POST");
  }

  // ---------------------------------------------------------------------
  // Y: cycles
  // ---------------------------------------------------------------------

  static void caseY2() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace t1 = ins(board, t, new Point[] {ip(20000, 20000), ip(40000, 20000)});
    PolylineTrace t2 = ins(board, t, new Point[] {ip(40000, 20000), ip(40000, 40000)});
    PolylineTrace t3 = ins(board, t, new Point[] {ip(40000, 40000), ip(20000, 40000)});
    PolylineTrace t4 = ins(board, t, new Point[] {ip(20000, 40000), ip(20000, 20000)});
    PolylineTrace t5 = ins(board, t, new Point[] {ip(20000, 20000), ip(20000, 10000)});
    System.out.println("--Y2-- square + pendant T1=" + t1.getId() + " T2=" + t2.getId()
        + " T3=" + t3.getId() + " T4=" + t4.getId() + " T5=" + t5.getId());
    System.out.println("Y2_ISCYCLE t1=" + t1.isCycle());
    System.out.println("Y2_TAIL_BEFORE e0=" + (board.getTraceTail(ip(20000, 20000),
        t1.getLayer(), t1.netNumbers) == null ? "null" : "hit")
        + " e1=" + (board.getTraceTail(ip(40000, 20000), t1.getLayer(),
            t1.netNumbers) == null ? "null" : "hit"));
    System.out.println("Y2_CONN_ITEMS " + items(t1.getConnectionItems()));
    System.out.println("Y2_REMOVE_IF_CYCLE " + board.removeIfCycle(t1));
    dumpItems(board, "Y2_POST");
    PolylineTrace w = ins(board, t, new Point[] {ip(60000, 20000), ip(65000, 20000)});
    System.out.println("Y2_WITNESS_ID " + w.getId() + " (ids 10..14 burned)");
  }

  static void caseY3() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace t1 = ins(board, t, new Point[] {ip(40000, 40000), ip(50000, 40000)});
    PolylineTrace t2 = ins(board, t, new Point[] {ip(60000, 40000), ip(40000, 40000)});
    System.out.println("--Y3-- via triangle T1=" + t1.getId() + " T2=" + t2.getId()
        + " VIA=5 (both start contacts of T1 are SEEDED — DFS blocked)");
    System.out.println("Y3_T1_START " + endContacts(t1, true)
        + " END " + endContacts(t1, false));
    System.out.println("Y3_ISCYCLE t1=" + t1.isCycle());
    System.out.println("Y3_REMOVE_IF_CYCLE " + board.removeIfCycle(t1));
    dumpItems(board, "Y3_POST");
  }

  static void caseY5() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = ins(board, t, new Point[] {ip(10000, 10000), ip(20000, 10000)});
    System.out.println("--Y5-- getTraceTail positive rows A=" + a.getId());
    Trace tailStart = board.getTraceTail(ip(10000, 10000), a.getLayer(), a.netNumbers);
    Trace tailEnd = board.getTraceTail(ip(20000, 10000), a.getLayer(), a.netNumbers);
    Trace tailMid = board.getTraceTail(ip(15000, 10000), a.getLayer(), a.netNumbers);
    Trace tailForeign = board.getTraceTail(ip(10000, 10000), a.getLayer(), new int[] {2});
    System.out.println("Y5_TAIL start=" + (tailStart == null ? "null" : tailStart.getId())
        + " end=" + (tailEnd == null ? "null" : tailEnd.getId())
        + " mid=" + (tailMid == null ? "null" : tailMid.getId())
        + " foreignNet=" + (tailForeign == null ? "null" : tailForeign.getId()));
  }

  static void caseY6() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = ins(board, t, new Point[] {ip(20000, 20000), ip(30000, 20000)});
    PolylineTrace b = ins(board, t, new Point[] {ip(30000, 20000), ip(30000, 30000)});
    System.out.println("--Y6-- non-cycle refusal A=" + a.getId() + " B=" + b.getId());
    System.out.println("Y6_ISCYCLE a=" + a.isCycle());
    System.out.println("Y6_REMOVE_IF_CYCLE " + board.removeIfCycle(a));
    System.out.println("Y6_A_ALIVE " + a.isOnTheBoard() + " B_ALIVE " + b.isOnTheBoard());
  }

  static void caseY7() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = ins(board, t, new Point[] {ip(10000, 10000), ip(30000, 10000)});
    PolylineTrace b = ins(board, t, new Point[] {ip(30000, 10000), ip(10000, 10000)});
    System.out.println("--Y7-- overlap cycle A=" + a.getId() + " B=" + b.getId()
        + " (isOverlap shortcut; connectionItems = self only)");
    System.out.println("Y7_A_START " + endContacts(a, true)
        + " END " + endContacts(a, false));
    System.out.println("Y7_ISCYCLE a=" + a.isCycle());
    System.out.println("Y7_CONN_ITEMS " + items(a.getConnectionItems()));
    System.out.println("Y7_REMOVE_IF_CYCLE " + board.removeIfCycle(a));
    dumpItems(board, "Y7_POST");
    PolylineTrace w = ins(board, t, new Point[] {ip(60000, 20000), ip(65000, 20000)});
    System.out.println("Y7_WITNESS_ID " + w.getId());
  }

  /** Y1 — the pure square. The walk probe: dump every input the
   * getConnectionItems walk consumes (stored corners, per-endpoint
   * and union contact sets, pairwise firstCommonLayer /
   * normalContactPoint), then the walk's return set from three
   * different start items. The walk terminates through the
   * fork-detection break (no visited set exists); the exact result
   * membership is jar-defined and pinned from this capture. */
  static void caseY1HangProbe() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace t1 = ins(board, t, new Point[] {ip(20000, 20000), ip(40000, 20000)});
    PolylineTrace t2 = ins(board, t, new Point[] {ip(40000, 20000), ip(40000, 40000)});
    PolylineTrace t3 = ins(board, t, new Point[] {ip(40000, 40000), ip(20000, 40000)});
    PolylineTrace t4 = ins(board, t, new Point[] {ip(20000, 40000), ip(20000, 20000)});
    System.out.println("--Y1-- pure square (hangprobe pass) T1=" + t1.getId()
        + " T2=" + t2.getId() + " T3=" + t3.getId() + " T4=" + t4.getId());
    PolylineTrace[] square = {t1, t2, t3, t4};
    for (PolylineTrace tr : square) {
      System.out.println("Y1_PROBE id=" + tr.getId() + " corners=" + cornersOf(tr)
          + " routable=" + tr.isRoutable()
          + " start=" + endContacts(tr, true) + " end=" + endContacts(tr, false)
          + " all=" + items(tr.getNormalContacts()));
    }
    for (Item item : board.getItems()) {
      if (item instanceof app.freerouting.board.model.items.Via via) {
        System.out.println("Y1_VIA id=" + via.getId() + " all=" + items(via.getNormalContacts()));
      }
    }
    for (PolylineTrace x : square) {
      for (PolylineTrace y : square) {
        if (x == y) {
          continue;
        }
        Point ncp = x.normalContactPoint(y);
        System.out.println("Y1_PAIR x=" + x.getId() + " y=" + y.getId()
            + " fcl=" + x.firstCommonLayer(y)
            + " ncp=" + (ncp == null ? "null" : xy(ncp)));
      }
    }
    System.out.println("Y1_ISCYCLE t1=" + t1.isCycle());
    System.out.println("Y1_CONN_ITEMS_RETURNED " + items(t1.getConnectionItems()));
    System.out.println("Y1_CONN_12 " + items(t3.getConnectionItems()));
    System.out.println("Y1_CONN_13 " + items(t4.getConnectionItems()));
  }

  // ---------------------------------------------------------------------
  // G: the insertTraceWithoutCleaning guards (BasicBoard.java:179-207)
  // ---------------------------------------------------------------------

  static void caseG() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    System.out.println("--G-- insert guards tmpl_hw=" + t.getHalfWidth());
    // 1 corner: cornerCount -1 < 2 → null BEFORE the ctor: no id burned.
    PolylineTrace g1 = board.insertTraceWithoutCleaning(
        new Polyline(new Point[] {ip(60000, 20000)}), t.getLayer(),
        t.getHalfWidth(), t.netNumbers, t.clearanceClassIndex(), FixedState.UNFIXED);
    System.out.println("G_ONE_CORNER " + (g1 == null ? "null" : g1.getId()));
    PolylineTrace w1 = ins(board, t, new Point[] {ip(60000, 20000), ip(65000, 20000)});
    System.out.println("G_NEXT_AFTER_ONE_CORNER " + w1.getId());
    // The doubled-back 3-point input COLLAPSES in Polygon construction
    // (B.side_of(A, A) is collinear → [A,A] → [A]) to the empty
    // polyline → ALSO refused by the cornerCount guard, no burn. (The
    // closed-corner guard is NOT reachable from an [A,B,A] input.)
    PolylineTrace g2 = board.insertTraceWithoutCleaning(
        new Polyline(new Point[] {ip(70000, 20000), ip(75000, 20000), ip(70000, 20000)}),
        t.getLayer(), t.getHalfWidth(), t.netNumbers, t.clearanceClassIndex(),
        FixedState.UNFIXED);
    System.out.println("G_DOUBLED_BACK " + (g2 == null ? "null" : g2.getId()));
    PolylineTrace w2 = ins(board, t, new Point[] {ip(60000, 21000), ip(65000, 21000)});
    System.out.println("G_NEXT_AFTER_DOUBLED_BACK " + w2.getId());
    // A REAL closed ring survives the Polygon cleanup (no consecutive
    // dup, no collinear middle): cornerCount 4 >= 2 → the ctor burns
    // an id → first corner == last corner and UNFIXED → refused AFTER
    // the ctor: the id STAYS burned.
    Point[] ring = {ip(60000, 30000), ip(70000, 30000), ip(70000, 40000), ip(60000, 30000)};
    PolylineTrace g3 = board.insertTraceWithoutCleaning(
        new Polyline(ring), t.getLayer(), t.getHalfWidth(), t.netNumbers,
        t.clearanceClassIndex(), FixedState.UNFIXED);
    System.out.println("G_RING_UNFIXED " + (g3 == null ? "null" : g3.getId()));
    PolylineTrace w3 = insWidth(board, t, new Point[] {ip(60000, 22000), ip(65000, 22000)}, 250);
    System.out.println("G_NEXT_AFTER_RING_UNFIXED " + w3.getId()
        + " (the burned id shows as a gap)");
    // The same ring USER_FIXED: inserts.
    PolylineTrace g4 = board.insertTraceWithoutCleaning(
        new Polyline(ring), t.getLayer(), t.getHalfWidth(), t.netNumbers,
        t.clearanceClassIndex(), FixedState.USER_FIXED);
    System.out.println("G_RING_USER_FIXED id="
        + (g4 == null ? "null" : g4.getId())
        + " hw=" + (g4 == null ? "?" : g4.getHalfWidth())
        + " corners=" + (g4 == null ? "?" : cornersOf(g4)));
    // The half-width range tracking (:198-200) used the PARAM (w3 came
    // through insWidth at 250, the template carries 125) — and
    // accumulates into BasicBoard's OWN pair (init 1000 / 10000), not
    // the BoardRules fields the parse fills (which stay at 125).
    System.out.println("G_BB_MAX " + board.getMaxTraceHalfWidth());
    System.out.println("G_BB_MIN " + board.getMinTraceHalfWidth());
    System.out.println("G_MAX_TRACE_HW " + board.rules.getMaxTraceHalfWidth());
  }

  // ---------------------------------------------------------------------
  // Fix-round additions (spec review of 333f5499): the isRoutable kind
  // gate (MAJOR 1), the degenerate collapse (MAJOR 2), and the missing
  // post-combine tree dumps (MINOR 2). All APPENDED — every row above
  // this point is byte-identical to the pre-fix capture.
  // ---------------------------------------------------------------------

  /**
   * PINWALK — the isRoutable kind-gate witness. Item.isRoutable()
   * returns FALSE in the BASE class (Item.java:908-910); only Trace
   * (Trace.java:206-209) and Via (Via.java:147-150) override it — a
   * PIN is never routable no matter its nets. Three probes: (1) the
   * OTHER-net trace A (id 10) touches same-net pin 3 — the walk
   * reaches the pin (it IS a contact) but must EXCLUDE it and STOP;
   * (2) walking FROM the pin — result.add(this) is gated on
   * isRoutable, so the pin's own walk still returns only A; (3)
   * walking FROM the via — a Via IS routable (the override), so the
   * via lands IN the result set: the kind gate is Trace|Via, not
   * trace-only. All walks are read-only; every item survives.
   */
  static void casePinWalk() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    Pin pin = null;
    Via via = null;
    for (Item item : board.getItems()) {
      if (item instanceof Pin p) {
        pin = p;
      }
      if (item instanceof Via v) {
        via = v;
      }
    }
    PolylineTrace a = insNet(board, t, new Point[] {ip(20000, 40000), ip(20000, 30000)},
        pin.netNumbers);
    PolylineTrace b = ins(board, t, new Point[] {ip(40000, 40000), ip(50000, 40000)});
    System.out.println("--PINWALK-- routable kind gate PIN=" + pin.getId()
        + " A=" + a.getId() + " (pin's net) B=" + b.getId() + " VIA=" + via.getId());
    System.out.println("PINWALK_NETS pin=" + pin.netNumbers[0] + " a=" + a.netNumbers[0]
        + " b=" + b.netNumbers[0] + " via=" + via.netNumbers[0]);
    System.out.println("PINWALK_ROUTABLE pin=" + pin.isRoutable() + " via=" + via.isRoutable()
        + " trace=" + a.isRoutable());
    System.out.println("PINWALK_A_CONTACTS " + items(a.getNormalContacts(a.firstCorner(),
        false)));
    System.out.println("PINWALK_FROM_TRACE_A " + items(a.getConnectionItems()));
    System.out.println("PINWALK_FROM_PIN " + items(pin.getConnectionItems()));
    System.out.println("PINWALK_FROM_VIA " + items(via.getConnectionItems()));
    dumpItems(board, "PINWALK_POST");
    // Quality-round probes (MINOR 3a/3c), appended AFTER the pinned
    // rows above: a SECOND via at via's own center (the Drill↔Drill
    // equal-centers contact — a one-via board leaves that dispatch
    // dead), a USER_FIXED same-net trace and a NETLESS trace (the two
    // non-kind clauses of the routable gate). PAD_C600 spans ONE
    // layer, so insertVia's splitTraces tail never runs (the
    // fromLayer..toLayer range is empty) and no trace crosses the
    // touched centers — nothing above moves.
    Via via2 = board.insertVia(via.getPadstack(), ip(40000, 40000), via.netNumbers,
        via.clearanceClassIndex(), FixedState.UNFIXED, false);
    PolylineTrace fixedTrace = insFixed(board, t, new Point[] {ip(75000, 45000),
        ip(85000, 45000)}, FixedState.USER_FIXED);
    PolylineTrace netlessTrace = insNet(board, t, new Point[] {ip(75000, 50000),
        ip(85000, 50000)}, new int[0]);
    System.out.println("PINWALK_VIA2_ID " + via2.getId());
    System.out.println("PINWALK_DRILL_DRILL_NCP " + xy(via.normalContactPoint(via2)));
    System.out.println("PINWALK_VIA2_CONTACTS " + items(via2.getNormalContacts()));
    System.out.println("PINWALK_VIA_CONTACTS_NOW " + items(via.getNormalContacts()));
    System.out.println("PINWALK_FIXED_ID " + fixedTrace.getId() + " ROUTABLE "
        + fixedTrace.isRoutable());
    System.out.println("PINWALK_NETLESS_ID " + netlessTrace.getId() + " ROUTABLE "
        + netlessTrace.isRoutable());
  }

  /**
   * DEGEN — the single-segment join probe. SPIKE-VS-RECON CORRECTION:
   * the review predicted two collinear single-segment (2-line) traces
   * joined end-to-end would drop below 3 lines and lose BOTH traces —
   * the jar disagrees: a constructed Polyline stores corners+1 lines,
   * so the join holds 3 lines (DEGEN_PRED joinedLen=3) and ONE merged
   * survivor (DEGEN_A_ALIVE true, B removed). The shrink rule
   * (PolylineTrace.java:324-327 / :448-451) only fires when the ctor
   * DEGENERATES the join — DEGEN2 constructs that case (joinedLen=0,
   * both removed).
   */
  static void caseDegen() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = ins(board, t, new Point[] {ip(60000, 30000), ip(70000, 30000)});
    PolylineTrace b = ins(board, t, new Point[] {ip(70000, 30000), ip(85000, 30000)});
    System.out.println("--DEGEN-- single-segment join A=" + a.getId() + " B=" + b.getId()
        + " (3 stored lines after the join: MERGES, one survivor)");
    System.out.println("DEGEN_PRED " + branchPred(board, a, b, false));
    System.out.println("DEGEN_COMBINE " + a.combine());
    System.out.println("DEGEN_A_ALIVE " + a.isOnTheBoard() + " B_ALIVE " + b.isOnTheBoard());
    dumpItems(board, "DEGEN_POST");
    dumpTree(board, "DEGEN_POST");
    PolylineTrace w = ins(board, t, new Point[] {ip(60000, 35000), ip(65000, 35000)});
    System.out.println("DEGEN_NEXT_ID " + w.getId() + " (B's 11 burned, A kept 10)");
  }

  /**
   * DEGEN2 — the zero-area double-back probe: B retraces BOTH of A's
   * legs in reverse (the L and its mirror). The exactly-once contract
   * holds at the shared corner; the join produces a spike path (out
   * and back) whose Polyline ctor cleanup EMPTIES it — joinedLen 0
   * < 3 fires the shrink rule and removes BOTH traces (capture:
   * DEGEN2_PRED joinedLen=0 survivor=EMPTY, both ALIVE false, tree
   * back to the 31-row baseline, NEXT_ID 12).
   */
  static void caseDegen2() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = ins(board, t, new Point[] {ip(10000, 10000), ip(30000, 10000),
      ip(30000, 30000)});
    PolylineTrace b = ins(board, t, new Point[] {ip(30000, 30000), ip(30000, 10000),
      ip(10000, 10000)});
    System.out.println("--DEGEN2-- zero-area double-back A=" + a.getId() + " B=" + b.getId()
        + " (B is A reversed: joined ctor may empty → both removed)");
    System.out.println("DEGEN2_PRED_START " + branchPred(board, a, b, true));
    System.out.println("DEGEN2_PRED_END " + branchPred(board, a, b, false));
    System.out.println("DEGEN2_COMBINE " + a.combine());
    System.out.println("DEGEN2_A_ALIVE " + a.isOnTheBoard()
        + (a.isOnTheBoard() ? " corners=" + cornersOf(a) : " (removed)")
        + " B_ALIVE " + b.isOnTheBoard());
    dumpItems(board, "DEGEN2_POST");
    dumpTree(board, "DEGEN2_POST");
    PolylineTrace w = ins(board, t, new Point[] {ip(60000, 35000), ip(65000, 35000)});
    System.out.println("DEGEN2_NEXT_ID " + w.getId());
  }

  /**
   * DEGEN3 — the overrun variant: B doubles back over A's vertical
   * leg and then OVERRUNS past A's first corner (collinear extension,
   * not an exact retrace). Probes whether the ctor cleanup differs
   * from DEGEN2's exact retrace.
   */
  static void caseDegen3() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = ins(board, t, new Point[] {ip(10000, 10000), ip(30000, 10000),
      ip(30000, 30000)});
    PolylineTrace b = ins(board, t, new Point[] {ip(30000, 30000), ip(30000, 10000),
      ip(0, 10000)});
    System.out.println("--DEGEN3-- double-back + overrun A=" + a.getId() + " B=" + b.getId());
    System.out.println("DEGEN3_PRED_START " + branchPred(board, a, b, true));
    System.out.println("DEGEN3_PRED_END " + branchPred(board, a, b, false));
    System.out.println("DEGEN3_COMBINE " + a.combine());
    System.out.println("DEGEN3_A_ALIVE " + a.isOnTheBoard()
        + (a.isOnTheBoard() ? " corners=" + cornersOf(a) : " (removed)")
        + " B_ALIVE " + b.isOnTheBoard());
    dumpItems(board, "DEGEN3_POST");
    dumpTree(board, "DEGEN3_POST");
    PolylineTrace w = ins(board, t, new Point[] {ip(60000, 35000), ip(65000, 35000)});
    System.out.println("DEGEN3_NEXT_ID " + w.getId());
  }

  /**
   * C4/C5/D1/D3 post-combine tree dumps (MINOR 2): the original cases
   * printed only PRED/POST_ITEM rows; these siblings repeat the setup
   * EXACTLY and dump the default tree after combine() — the D18-safe
   * leaf-level pins for the remaining merge cases.
   */
  static void caseC4Tree() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = ins(board, t, new Point[] {ip(10000, 50000), ip(30000, 50000)});
    PolylineTrace b = ins(board, t, new Point[] {ip(30000, 50000), ip(50000, 50000)});
    PolylineTrace c = ins(board, t, new Point[] {ip(50000, 50000), ip(70000, 50000)});
    System.out.println("--C4_TREE-- post-combine tree (setup = caseC4)");
    System.out.println("C4_TREE_COMBINE " + a.combine());
    dumpTree(board, "C4_POST");
  }

  static void caseC5Tree() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = ins(board, t, new Point[] {ip(30000, 20000), ip(50000, 20000)});
    PolylineTrace b1 = ins(board, t, new Point[] {ip(10000, 20000), ip(30000, 20000)});
    PolylineTrace b2 = ins(board, t, new Point[] {ip(50000, 20000), ip(70000, 20000)});
    System.out.println("--C5_TREE-- post-combine tree (setup = caseC5)");
    System.out.println("C5_TREE_COMBINE " + a.combine());
    dumpTree(board, "C5_POST");
  }

  static void caseD1Tree() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = ins(board, t, new Point[] {ip(10000, 10000), ip(30000, 10000)});
    PolylineTrace b = ins(board, t, new Point[] {ip(30000, 10000), ip(20000, 10000)});
    System.out.println("--D1_TREE-- post-combine tree (setup = caseD1)");
    System.out.println("D1_TREE_COMBINE " + a.combine());
    dumpTree(board, "D1_POST");
  }

  static void caseD3Tree() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = ins(board, t, new Point[] {ip(10000, 10000), ip(30000, 10000),
      ip(30000, 30000), ip(10000, 30000)});
    PolylineTrace b = ins(board, t, new Point[] {ip(10000, 30000), ip(10000, 10000)});
    System.out.println("--D3_TREE-- post-combine tree (setup = caseD3)");
    System.out.println("D3_TREE_COMBINE " + a.combine());
    dumpTree(board, "D3_POST");
  }

  public static void main(String[] args) throws Exception {
    if (args.length >= 1 && args[0].equals("hangprobe")) {
      caseY1HangProbe();
      return;
    }
    sectionB();
    caseC1();
    caseC2();
    caseC3();
    caseC4();
    caseC5();
    caseC6();
    caseC7();
    caseR1();
    caseR2();
    caseR3();
    caseR4();
    caseR5();
    caseR6();
    caseD1();
    caseD2();
    caseD3();
    caseY2();
    caseY3();
    caseY5();
    caseY6();
    caseY7();
    caseG();
    casePinWalk();
    caseDegen();
    caseDegen2();
    caseDegen3();
    caseC4Tree();
    caseC5Tree();
    caseD1Tree();
    caseD3Tree();
    System.out.println("--DONE--");
  }
}
