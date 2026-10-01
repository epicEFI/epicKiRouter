// SplitSpike.java — jar spike for M2 Task 12 (the trace SPLIT +
// NORMALIZATION seam: PolylineTrace.split(IntOctagon)
// PolylineTrace.java:465-691, split(Point) :699-712,
// split(int, Line) :719-760, splitInsideDrillPadProhibited
// :768-792, PolylineTrace.normalize :801 →
// PolylineTraceNormalization.normalize (MAX_NORMALIZATION_DEPTH 16),
// plus the support seams BasicBoard.pickItems (BasicBoard.java:1087)
// and geometry.planar.Polyline.split (Polyline.java:758-835) /
// LineSegment.intersection (LineSegment.java:229-278)).
//
// Same harness pattern as CombineSpike (T11): the ContactsSpike PURE
// board (parse ids 1..9, parse state normalizeAllTraces leaves
// untouched), trap geometry added POST-parse through
// insertTraceWithoutCleaning (never normalizes), ids deterministic
// from 10. Run (JDK 25, repo root; the sed strips FRLogger's leading
// timestamp so two runs diff clean):
//   mkdir -p /tmp/epic-t12-classes && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/javac -cp build/libs/freerouting-current-executable.jar \
//       -d /tmp/epic-t12-classes rust/harness/oracle/SplitSpike.java && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/java -cp build/libs/freerouting-current-executable.jar:/tmp/epic-t12-classes \
//       app.freerouting.datastructures.SplitSpike 2>&1 \
//       | sed -E 's/^[0-9]{4}-[0-9]{2}-[0-9]{2} [0-9]{2}:[0-9]{2}:[0-9]{2}\.[0-9]+ //' \
//       | tee /tmp/epic-t12-split.out
//
// Sections (output line prefixes):
//   B    — parse baseline witness (item ids + tree count).
//   X1   — the X-crossing pair: found-first split (B split by A's
//          line), then the own split (A split by B's line), recursive
//          piece splits, the two-pass cycle removal, the entry-loop
//          break. Full post tree dump.
//   X2   — the RECEIVER-ORDER contrast: LineSegment.intersection
//          called both ways on the X1 pair, a collinear-overlap pair,
//          and a diagonal pair — the returned LINE (its a/b defining
//          points) per call, pinning `other.middle` semantics.
//   X3   — the collinear overlap pair: 2 closing lines, the
//          first-success break, found split at ITS first common
//          point-refusal then the second line, own split, duplicate
//          geometry with distinct ids.
//   X4   — the self-crossing trace: the [i-1,i+1] skip (own entries
//          at i-1..i+1), the corner-equality skip NOT firing (unequal
//          corners), the REAL self-split, the closed-ring piece, and
//          the split-internal removeIfCycle pass removing it.
//   X4L  — the loop trace (visits P twice): the corner-equality
//          ELSE-form skip firing (corner(1) == corner(5)), a real own
//          split at P through another own entry, the closure piece,
//          recursion, and the ring removed by the split's cycle pass.
//   DRL1 — the drill split at the via center as the LAST entry: the
//          split result is DISCARDED, ownTraceSplit stays false, and
//          the deleted `this` is ADDED to the result
//          (`result.add(this)` on a removed trace).
//   DRL2 — the drill split as the FIRST entry: the split happens,
//          the loop-top guard then finds `this` off the board and
//          returns the EMPTY collection (early return path).
//   PAD1 — the FOREIGN-net pin is SKIPPED by the sharesNet gate
//          (:776): the split point is inside the OTHER-net pin's pad,
//          but padFound never fires → the split PROCEEDS (both traces
//          split at the crossing).
//   PAD2 — the split point is the pin's center, but the pin is
//          FOREIGN (net OTHER): sharesNet gates it out before the
//          center-allow can run; the split is allowed by the same-net
//          trace endpoint clause (PAD6 is the true center-allow pin).
//   PAD3 — ALLOWED via a foreign... same-net trace ENDPOINT at the
//          split point (contrast witness for PAD1).
//   PAD4 — THE PRECEDENCE QUIRK (:785-786): `currentTrace != this &&
//          first.equals(isect) || last.equals(isect)` parses as
//          `(!= this && first) || last` — the trace's OWN last corner
//          at the isect ALLOWS a split inside a foreign pad
//          (bug-compat; correct precedence would refuse). The second
//          insert (the closed ring piece) is refused by
//          insertTraceWithoutCleaning's first==last guard below
//          USER_FIXED — split(Point) returns [piece, null].
//   PAD5 — the REAL refusal (PAD_PAD_DSN, the pin is net MINE): a
//          same-net pin whose center != the isect sets padFound →
//          both inner splits null → nothing changes, result=[this].
//   PAD6 — the CENTER allow in the same DSN: the isect equals the
//          same-net pin's center → immediate `return false` (allowed)
//          before padFound matters.
//   AR1  — the area-cycle removal: both endpoints contact the
//          conduction area → removeItem(this) + EMPTY result.
//   AR2  — contrast: netClass.setIgnoreCyclesWithAreas(true) skips
//          the removal, the trace survives.
//   DEL1 — deletion-forbidden refusal: two USER_FIXED traces cross,
//          both inner splits null, nothing changes.
//   DEL2 — mixed: the USER_FIXED found trace refuses, the plain
//          receiver still splits around it.
//   CLIP1 — the clipShape restriction: a crossing OUTSIDE the clip's
//          segment-bbox filter never splits (i-loop continue);
//          CLIP2 — the same geometry WITHOUT the clip splits.
//   NORM1 — normalize on a degenerate (2-corner first==last) piece
//          kept: USER_FIXED → deletion-forbidden → survives.
//   NORM2 — the same degenerate trace flipped UNFIXED → removed by
//          the degenerate check (checked BEFORE the combine
//          else-if, gated on !isDeletionForbidden).
//   NORM3 — the recursion: split → piece combines a neighbor →
//          normalize(piece, depth+1) — the merged survivor.
//   NORM4 — a staircase cascade that keeps normalize recursion well
//          under the cap (depth 16 in PASS 1; each level splits at a
//          via crossing and combines the next collinear extension)
//          plus a stable SECOND PASS over the survivors. The cap
//          itself is NOT reachable here: its only trace is Java's
//          FRLogger.debug, which JUL drops below the console level.
//   NORM5A/B/C — the size!=1 contract: no-op → false; the
//          last-entry drill split returns [deleted this] (size 1)
//          → normalize FALSE while DELETING the trace; the first-
//          entry drill split returns [] (size 0) → TRUE.
//   PICK — BasicBoard.pickItems rows (point, layer, null filter).
//
// Output discipline: exact ints via field access, literal capture
// rows only, LinkedList/TreeSet iteration orders preserved.
package app.freerouting.datastructures;

import app.freerouting.board.actions.ItemIdGenerator;
import app.freerouting.board.facade.BasicBoard;
import app.freerouting.board.model.items.ConductionArea;
import app.freerouting.board.model.items.DrillItem;
import app.freerouting.board.model.items.Item;
import app.freerouting.board.model.items.ObstacleArea;
import app.freerouting.board.model.items.Pin;
import app.freerouting.board.model.items.Trace;
import app.freerouting.board.model.items.Via;
import app.freerouting.board.model.structure.BoardOutline;
import app.freerouting.board.model.structure.FixedState;
import app.freerouting.board.searchtree.SearchTreeObject;
import app.freerouting.board.searchtree.ShapeSearchTree;
import app.freerouting.board.trace.PolylineTrace;
import app.freerouting.geometry.planar.IntBox;
import app.freerouting.geometry.planar.IntOctagon;
import app.freerouting.geometry.planar.IntPoint;
import app.freerouting.geometry.planar.Line;
import app.freerouting.geometry.planar.LineSegment;
import app.freerouting.geometry.planar.Point;
import app.freerouting.geometry.planar.Polyline;
import app.freerouting.geometry.planar.RegularTileShape;
import app.freerouting.io.BoardReadResult;
import app.freerouting.io.specctra.DsnReader;
import app.freerouting.rules.NetClass;
import java.io.ByteArrayInputStream;
import java.util.ArrayList;
import java.util.Collection;
import java.util.List;

public final class SplitSpike {

  private SplitSpike() {}

  // ---------------------------------------------------------------------
  // helpers (mirrored from CombineSpike)
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
    if (item instanceof app.freerouting.board.model.items.ComponentOutline) {
      return "CO";
    }
    if (item instanceof ObstacleArea) {
      return "K";
    }
    return "?";
  }

  static String items(Collection<? extends Item> set) {
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

  static PolylineTrace insNet(BasicBoard board, PolylineTrace tmpl, Point[] corners,
      int[] nets) {
    return board.insertTraceWithoutCleaning(new Polyline(corners), tmpl.getLayer(),
        tmpl.getHalfWidth(), nets, tmpl.clearanceClassIndex(), FixedState.UNFIXED);
  }

  static Via insVia(BasicBoard board, Via template, IntPoint center) {
    return board.insertVia(template.getPadstack(), center, template.netNumbers,
        template.clearanceClassIndex(), FixedState.UNFIXED, false);
  }

  // ---------------------------------------------------------------------
  // the tree dump (CombineSpike's format)
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
    if (s instanceof IntBox b) {
      return "box[" + b.ll.x + " " + b.ll.y + " " + b.ur.x + " " + b.ur.y + "]";
    }
    IntOctagon o = (IntOctagon) s;
    return "oct[" + o.leftX + " " + o.bottomY + " " + o.rightX + " " + o.topY
        + " " + o.upperLeftDiagonalX + " " + o.lowerRightDiagonalX
        + " " + o.lowerLeftDiagonalX + " " + o.upperRightDiagonalX + "]";
  }

  static void dumpTree(BasicBoard board, String prefix) {
    List<String> lines = dumpLines(board.searchTreeManager.getDefaultTree());
    System.out.println(prefix + "_TREE lines=" + lines.size());
    for (String line : lines) {
      System.out.println(prefix + "_TREE_ROW " + line);
    }
  }

  static void treeCount(BasicBoard board, String prefix) {
    System.out.println(prefix + "_TREE lines=" + dumpLines(
        board.searchTreeManager.getDefaultTree()).size());
  }

  // ---------------------------------------------------------------------
  // split-result dumpers
  // ---------------------------------------------------------------------

  /** The split(IntOctagon) result in LIST ORDER (LinkedList parity). */
  static void dumpSplitPieces(Collection<PolylineTrace> pieces, String prefix) {
    StringBuilder sb = new StringBuilder("size=").append(pieces.size()).append(" [");
    boolean first = true;
    for (PolylineTrace piece : pieces) {
      if (!first) {
        sb.append(" ");
      }
      first = false;
      sb.append(piece.getId()).append(":").append(cornersOf(piece));
    }
    sb.append("]");
    System.out.println(prefix + " " + sb);
  }

  /** The split(Point) result: null or the 2 pieces. */
  static void dumpSplitPoint(Trace[] pieces, String prefix) {
    if (pieces == null) {
      System.out.println(prefix + " null");
      return;
    }
    StringBuilder sb = new StringBuilder("[");
    for (int i = 0; i < pieces.length; i++) {
      if (i > 0) {
        sb.append(" ");
      }
      Trace t = pieces[i];
      sb.append(t == null ? "null" : t.getId() + ":" + cornersOf((PolylineTrace) t));
    }
    sb.append("]");
    System.out.println(prefix + " " + sb);
  }

  /** A post-case id witness: the next insert lands here. */
  static void witness(BasicBoard board, PolylineTrace t, String prefix) {
    PolylineTrace w = ins(board, t, new Point[] {ip(60000, 35000), ip(65000, 35000)});
    System.out.println(prefix + "_NEXT_ID " + w.getId());
  }

  /** One intersection call, both receiver orders. */
  static void dumpIsect(LineSegment receiver, LineSegment argument, String prefix) {
    Line[] result = receiver.intersection(argument);
    StringBuilder sb = new StringBuilder("len=").append(result.length).append(" [");
    for (int i = 0; i < result.length; i++) {
      if (i > 0) {
        sb.append(" ");
      }
      sb.append("a=").append(xy(result[i].a)).append(" b=").append(xy(result[i].b));
    }
    sb.append("]");
    System.out.println(prefix + " " + sb);
  }

  // ---------------------------------------------------------------------
  // the board (ContactsSpike/CombineSpike PURE_DSN, unchanged)
  // ---------------------------------------------------------------------

  static final String PURE_DSN =
      "(pcb t12-pure.dsn\n"
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
    return parseDsnBytes(PURE_DSN.getBytes(), "t12-pure.dsn");
  }

  /**
   * PURE_DSN with the PIN on net MINE (net 1) instead of OTHER: the
   * parse item set is identical (ids 1..9), but
   * splitInsideDrillPadProhibited's sharesNet gate now lets the pin
   * set padFound — the REAL refusal/center-allow geometry. Used only
   * by PAD5/PAD6; every other case keeps PURE_DSN so the banked rows
   * stay byte-identical.
   */
  static final String PURE_PAD_DSN =
      "(pcb t12-pad.dsn\n"
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
      + "    (net MINE (pins CMP1-P1))\n"
      + "    (net OTHER)\n"
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

  static BasicBoard freshPad() {
    return parseDsnBytes(PURE_PAD_DSN.getBytes(), "t12-pad.dsn");
  }

  // ---------------------------------------------------------------------
  // cases
  // ---------------------------------------------------------------------

  static void sectionB() {
    BasicBoard board = fresh();
    List<Item> list = new ArrayList<>(board.getItems());
    list.sort((a, b) -> b.getId() - a.getId());
    StringBuilder sb = new StringBuilder();
    for (Item item : list) {
      if (sb.length() > 0) {
        sb.append(",");
      }
      sb.append(item.getId()).append(":").append(kind(item));
    }
    System.out.println("--B-- baseline");
    System.out.println("B_IDS count=" + list.size() + " " + sb);
    treeCount(board, "B");
  }

  /** X1 — the X-crossing pair. */
  static void caseX1() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = ins(board, t, new Point[] {ip(30000, 20000), ip(50000, 20000)});
    PolylineTrace b = ins(board, t, new Point[] {ip(40000, 10000), ip(40000, 30000)});
    System.out.println("--X1-- x-cross A=" + a.getId() + " B=" + b.getId());
    Collection<PolylineTrace> pieces = a.split((IntOctagon) null);
    dumpSplitPieces(pieces, "X1_RESULT");
    dumpItems(board, "X1_POST");
    dumpTree(board, "X1_POST");
    witness(board, t, "X1");
  }

  /** X2 — the receiver-order contrast on LineSegment.intersection. */
  static void caseX2() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = ins(board, t, new Point[] {ip(30000, 20000), ip(50000, 20000)});
    PolylineTrace b = ins(board, t, new Point[] {ip(40000, 10000), ip(40000, 30000)});
    LineSegment segA = new LineSegment(a.polyline(), 1);
    LineSegment segB = new LineSegment(b.polyline(), 1);
    System.out.println("--X2-- receiver order (found-first uses found.intersection"
        + "(current); own split uses current.intersection(found))");
    dumpIsect(segB, segA, "X2_CROSS_FOUND_FIRST");
    dumpIsect(segA, segB, "X2_CROSS_OWN");
    PolylineTrace c = ins(board, t, new Point[] {ip(20000, 10000), ip(50000, 10000)});
    PolylineTrace d = ins(board, t, new Point[] {ip(30000, 10000), ip(65000, 10000)});
    LineSegment segC = new LineSegment(c.polyline(), 1);
    LineSegment segD = new LineSegment(d.polyline(), 1);
    dumpIsect(segD, segC, "X2_OVERLAP_FOUND_FIRST");
    dumpIsect(segC, segD, "X2_OVERLAP_OWN");
    // A perpendicular pair through a DIAGONAL, both orders.
    LineSegment diag = new LineSegment(
        new Polyline(new Point[] {ip(60000, 20000), ip(70000, 30000)}), 1);
    LineSegment vert = new LineSegment(
        new Polyline(new Point[] {ip(65000, 15000), ip(65000, 35000)}), 1);
    dumpIsect(vert, diag, "X2_DIAG_FOUND_FIRST");
    dumpIsect(diag, vert, "X2_DIAG_OWN");
  }

  /** X3 — the collinear overlap pair. */
  static void caseX3() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = ins(board, t, new Point[] {ip(20000, 10000), ip(50000, 10000)});
    PolylineTrace b = ins(board, t, new Point[] {ip(30000, 10000), ip(65000, 10000)});
    System.out.println("--X3-- collinear overlap A=" + a.getId() + " B=" + b.getId());
    Collection<PolylineTrace> pieces = a.split((IntOctagon) null);
    dumpSplitPieces(pieces, "X3_RESULT");
    dumpItems(board, "X3_POST");
    witness(board, t, "X3");
  }

  /** X4 — the self-crossing trace. */
  static void caseX4() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = ins(board, t, new Point[] {ip(10000, 10000), ip(50000, 10000),
      ip(50000, 40000), ip(30000, 40000), ip(30000, 0)});
    System.out.println("--X4-- self-cross A=" + a.getId()
        + " corners=" + cornersOf(a));
    Collection<PolylineTrace> pieces = a.split((IntOctagon) null);
    dumpSplitPieces(pieces, "X4_RESULT");
    dumpItems(board, "X4_POST");
    witness(board, t, "X4");
  }

  /** X4L — the loop trace: visits P=(20000,20000) at corner(1) and
   * corner(5); exercises the corner-equality ELSE-form skip and the
   * split-internal cycle pass on the closure ring. */
  static void caseX4L() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace l = ins(board, t, new Point[] {ip(10000, 10000), ip(20000, 20000),
      ip(30000, 20000), ip(30000, 30000), ip(20000, 30000), ip(20000, 20000),
      ip(10000, 30000)});
    System.out.println("--X4L-- loop trace L=" + l.getId()
        + " corners=" + cornersOf(l));
    Collection<PolylineTrace> pieces = l.split((IntOctagon) null);
    dumpSplitPieces(pieces, "X4L_RESULT");
    dumpItems(board, "X4L_POST");
    witness(board, t, "X4L");
  }

  /** DRL1 — drill split as the LAST entry: the deleted this is
   * returned. */
  static void caseDrl1() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = ins(board, t, new Point[] {ip(30000, 40000), ip(50000, 40000)});
    Via via = (Via) board.getItem(5);
    System.out.println("--DRL1-- drill split last-entry A=" + a.getId());
    Collection<PolylineTrace> pieces = a.split((IntOctagon) null);
    dumpSplitPieces(pieces, "DRL1_RESULT");
    System.out.println("DRL1_A_ON_BOARD " + a.isOnTheBoard());
    dumpItems(board, "DRL1_POST");
    dumpTree(board, "DRL1_POST");
    witness(board, t, "DRL1");
  }

  /** DRL2 — drill split as the FIRST entry: early EMPTY return. */
  static void caseDrl2() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = ins(board, t, new Point[] {ip(30000, 40000), ip(50000, 40000)});
    Via via = (Via) board.getItem(5);
    Via via2 = insVia(board, via, ip(45000, 40000));
    System.out.println("--DRL2-- drill split first-entry A=" + a.getId()
        + " via2=" + via2.getId());
    Collection<PolylineTrace> pieces = a.split((IntOctagon) null);
    dumpSplitPieces(pieces, "DRL2_RESULT");
    System.out.println("DRL2_A_ON_BOARD " + a.isOnTheBoard());
    dumpItems(board, "DRL2_POST");
    witness(board, t, "DRL2");
  }

  /** PAD1 — the pad prohibition REFUSES: split point inside the
   * foreign pin's pad, no same-net trace corner there. */
  static void casePad1() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = ins(board, t, new Point[] {ip(10000, 40250), ip(30000, 40250)});
    PolylineTrace b = ins(board, t, new Point[] {ip(20000, 38000), ip(20000, 42000)});
    System.out.println("--PAD1-- pad prohibition A=" + a.getId() + " B=" + b.getId());
    Collection<PolylineTrace> pieces = a.split((IntOctagon) null);
    dumpSplitPieces(pieces, "PAD1_RESULT");
    dumpItems(board, "PAD1_POST");
    witness(board, t, "PAD1");
  }

  /** PAD2 — ALLOWED at the pin CENTER (same-net trace endpoint at the
   * split point). */
  static void casePad2() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = ins(board, t, new Point[] {ip(20000, 30000), ip(20000, 50000)});
    System.out.println("--PAD2-- split at pin center A=" + a.getId());
    Collection<PolylineTrace> pieces = a.split((IntOctagon) null);
    dumpSplitPieces(pieces, "PAD2_RESULT");
    dumpItems(board, "PAD2_POST");
    witness(board, t, "PAD2");
  }

  /** PAD3 — ALLOWED via a same-net trace ENDPOINT at the split point
   * (the PAD1 contrast witness). */
  static void casePad3() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = ins(board, t, new Point[] {ip(10000, 40250), ip(30000, 40250)});
    PolylineTrace u = ins(board, t, new Point[] {ip(20000, 40250), ip(20000, 45250)});
    System.out.println("--PAD3-- endpoint allows A=" + a.getId() + " U=" + u.getId());
    Collection<PolylineTrace> pieces = a.split((IntOctagon) null);
    dumpSplitPieces(pieces, "PAD3_RESULT");
    dumpItems(board, "PAD3_POST");
    witness(board, t, "PAD3");
  }

  /** PAD4 — the :785-786 PRECEDENCE QUIRK: the trace's OWN last
   * corner at the isect allows the split inside the foreign pad. */
  static void casePad4() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    int[] otherNets = {2};
    PolylineTrace w = insNet(board, t, new Point[] {ip(10000, 40250), ip(30000, 40250),
      ip(30000, 45000), ip(20000, 45000), ip(20000, 40250)}, otherNets);
    System.out.println("--PAD4-- precedence quirk W=" + w.getId()
        + " nets=" + java.util.Arrays.toString(w.netNumbers)
        + " last=" + xy(w.lastCorner()));
    Trace[] pieces = w.split(ip(20000, 40250));
    dumpSplitPoint(pieces, "PAD4_SPLIT_POINT");
    dumpItems(board, "PAD4_POST");
    witness(board, t, "PAD4");
  }

  /** PAD5 — the REAL refusal: in PURE_PAD_DSN the pin is net MINE
   * (same-net with A and B), its center (20000,40000) != the isect
   * (20000,40250) → padFound → both inner splits null → result is
   * [this] unchanged. */
  static void casePad5() {
    BasicBoard board = freshPad();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = ins(board, t, new Point[] {ip(10000, 40250), ip(30000, 40250)});
    PolylineTrace b = ins(board, t, new Point[] {ip(20000, 38000), ip(20000, 42000)});
    System.out.println("--PAD5-- same-net pin refusal A=" + a.getId() + " B=" + b.getId()
        + " pinNets=" + java.util.Arrays.toString(((Pin) board.getItem(3)).netNumbers));
    Collection<PolylineTrace> pieces = a.split((IntOctagon) null);
    dumpSplitPieces(pieces, "PAD5_RESULT");
    System.out.println("PAD5_A_ON_BOARD " + a.isOnTheBoard()
        + " B_ON_BOARD " + b.isOnTheBoard());
    dumpItems(board, "PAD5_POST");
    witness(board, t, "PAD5");
  }

  /** PAD6 — the CENTER allow in PURE_PAD_DSN: the isect equals the
   * same-net pin's center → immediate return false (allowed) before
   * padFound could matter. */
  static void casePad6() {
    BasicBoard board = freshPad();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = ins(board, t, new Point[] {ip(20000, 30000), ip(20000, 50000)});
    System.out.println("--PAD6-- same-net pin center allow A=" + a.getId());
    Collection<PolylineTrace> pieces = a.split((IntOctagon) null);
    dumpSplitPieces(pieces, "PAD6_RESULT");
    dumpItems(board, "PAD6_POST");
    witness(board, t, "PAD6");
  }

  /** AR1 — the area-cycle removal: both endpoints contact area 6. */
  static void caseAr1() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = ins(board, t, new Point[] {ip(50500, 10500), ip(59500, 10500)});
    System.out.println("--AR1-- area cycle A=" + a.getId());
    Collection<PolylineTrace> pieces = a.split((IntOctagon) null);
    dumpSplitPieces(pieces, "AR1_RESULT");
    System.out.println("AR1_A_ON_BOARD " + a.isOnTheBoard());
    dumpItems(board, "AR1_POST");
    witness(board, t, "AR1");
  }

  /** AR2 — contrast: ignoreCyclesWithAreas=true keeps the trace. */
  static void caseAr2() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    NetClass netClass = board.rules.nets.get(1).getNetClass();
    netClass.setIgnoreCyclesWithAreas(true);
    System.out.println("--AR2-- ignoreCyclesWithAreas flipped: " + netClass.getIgnoreCyclesWithAreas());
    PolylineTrace a = ins(board, t, new Point[] {ip(50500, 10500), ip(59500, 10500)});
    Collection<PolylineTrace> pieces = a.split((IntOctagon) null);
    dumpSplitPieces(pieces, "AR2_RESULT");
    System.out.println("AR2_A_ON_BOARD " + a.isOnTheBoard());
    dumpItems(board, "AR2_POST");
    witness(board, t, "AR2");
  }

  /** DEL1 — deletion-forbidden refusal: both traces USER_FIXED. */
  static void caseDel1() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace u1 = insFixed(board, t, new Point[] {ip(10000, 10000),
        ip(30000, 10000)}, FixedState.USER_FIXED);
    PolylineTrace u2 = insFixed(board, t, new Point[] {ip(20000, 8000),
        ip(20000, 12000)}, FixedState.USER_FIXED);
    System.out.println("--DEL1-- both forbidden U1=" + u1.getId() + " U2=" + u2.getId());
    Collection<PolylineTrace> pieces = u1.split((IntOctagon) null);
    dumpSplitPieces(pieces, "DEL1_RESULT");
    System.out.println("DEL1_U1_FIXED " + u1.getFixedState()
        + " U2_FIXED " + u2.getFixedState());
    dumpItems(board, "DEL1_POST");
    witness(board, t, "DEL1");
  }

  /** DEL2 — mixed: the USER_FIXED found trace refuses, the plain
   * receiver still splits around it. */
  static void caseDel2() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace w = ins(board, t, new Point[] {ip(10000, 20000), ip(30000, 20000)});
    PolylineTrace v = insFixed(board, t, new Point[] {ip(20000, 18000),
        ip(20000, 22000)}, FixedState.USER_FIXED);
    System.out.println("--DEL2-- mixed W=" + w.getId() + " V=" + v.getId());
    Collection<PolylineTrace> pieces = w.split((IntOctagon) null);
    dumpSplitPieces(pieces, "DEL2_RESULT");
    System.out.println("DEL2_V_FIXED " + v.getFixedState()
        + " V_ON_BOARD " + v.isOnTheBoard());
    dumpItems(board, "DEL2_POST");
    witness(board, t, "DEL2");
  }

  /** CLIP1 — the clipShape restriction (B's crossing of segment 0 is
   * outside the clip bbox filter). */
  static void caseClip1() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = ins(board, t, new Point[] {ip(20000, 20000), ip(60000, 20000),
      ip(70000, 30000)});
    PolylineTrace b = ins(board, t, new Point[] {ip(40000, 10000), ip(40000, 30000)});
    PolylineTrace c = ins(board, t, new Point[] {ip(65000, 15000), ip(65000, 35000)});
    IntOctagon clip = new IntBox(ip(62000, 24000), ip(68000, 28000)).boundingOctagon();
    System.out.println("--CLIP1-- clip restriction A=" + a.getId() + " B=" + b.getId()
        + " C=" + c.getId());
    Collection<PolylineTrace> pieces = a.split(clip);
    dumpSplitPieces(pieces, "CLIP1_RESULT");
    dumpItems(board, "CLIP1_POST");
    witness(board, t, "CLIP1");
  }

  /** CLIP2 — the same geometry WITHOUT the clip splits at both
   * crossings (the CLIP1 contrast). */
  static void caseClip2() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = ins(board, t, new Point[] {ip(20000, 20000), ip(60000, 20000),
      ip(70000, 30000)});
    PolylineTrace b = ins(board, t, new Point[] {ip(40000, 10000), ip(40000, 30000)});
    PolylineTrace c = ins(board, t, new Point[] {ip(65000, 15000), ip(65000, 35000)});
    System.out.println("--CLIP2-- no clip A=" + a.getId());
    Collection<PolylineTrace> pieces = a.split((IntOctagon) null);
    dumpSplitPieces(pieces, "CLIP2_RESULT");
    dumpItems(board, "CLIP2_POST");
    witness(board, t, "CLIP2");
  }

  /** A 3-line polyline whose corner 0 equals corner 1 (the
   * degenerate normalize input): y=20000, x=30000, y=20000. */
  static Polyline degeneratePoly() {
    Line h1 = new Line(ip(20000, 20000), ip(40000, 20000));
    Line v = new Line(ip(30000, 20000), ip(30000, 30000));
    Line h2 = new Line(ip(20000, 20000), ip(40000, 20000));
    return new Polyline(new Line[] {h1, v, h2});
  }

  /** NORM1 — the degenerate trace USER_FIXED: deletion-forbidden →
   * the degenerate piece is KEPT, normalize returns its split
   * verdict. */
  static void caseNorm1() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    Polyline deg = degeneratePoly();
    System.out.println("--NORM1-- degenerate USER_FIXED ctorCorners="
        + cornersOfPoly(deg));
    PolylineTrace d = board.insertTraceWithoutCleaning(deg, t.getLayer(),
        t.getHalfWidth(), t.netNumbers, t.clearanceClassIndex(), FixedState.USER_FIXED);
    if (d == null) {
      System.out.println("NORM1_INSERT null");
      return;
    }
    System.out.println("NORM1_INSERT id=" + d.getId()
        + " corners=" + cornersOf(d) + " fixed=" + d.getFixedState());
    boolean changed = d.normalize(null);
    System.out.println("NORM1_NORMALIZE " + changed);
    System.out.println("NORM1_ALIVE " + d.isOnTheBoard()
        + (d.isOnTheBoard() ? " corners=" + cornersOf(d) : ""));
    dumpItems(board, "NORM1_POST");
    witness(board, t, "NORM1");
  }

  /** NORM2 — the same degenerate trace flipped UNFIXED: the
   * degenerate check REMOVES it (checked before the combine
   * else-if). */
  static void caseNorm2() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    Polyline deg = degeneratePoly();
    PolylineTrace d = board.insertTraceWithoutCleaning(deg, t.getLayer(),
        t.getHalfWidth(), t.netNumbers, t.clearanceClassIndex(), FixedState.USER_FIXED);
    if (d == null) {
      System.out.println("NORM2_INSERT null");
      return;
    }
    d.setFixedState(FixedState.UNFIXED);
    System.out.println("--NORM2-- degenerate UNFIXED id=" + d.getId()
        + " fixed=" + d.getFixedState());
    boolean changed = d.normalize(null);
    System.out.println("NORM2_NORMALIZE " + changed);
    System.out.println("NORM2_ALIVE " + d.isOnTheBoard());
    dumpItems(board, "NORM2_POST");
    witness(board, t, "NORM2");
  }

  /** NORM3 — the recursion: A's pieces re-combine B, normalize
   * recurses at depth 1. */
  static void caseNorm3() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = ins(board, t, new Point[] {ip(30000, 20000), ip(40000, 20000)});
    PolylineTrace b = ins(board, t, new Point[] {ip(40000, 20000), ip(50000, 20000)});
    PolylineTrace c = ins(board, t, new Point[] {ip(35000, 10000), ip(35000, 30000)});
    System.out.println("--NORM3-- recursion A=" + a.getId() + " B=" + b.getId()
        + " C=" + c.getId());
    boolean changed = a.normalize(null);
    System.out.println("NORM3_NORMALIZE " + changed);
    dumpItems(board, "NORM3_POST");
    witness(board, t, "NORM3");
  }

  /** NORM3P — the NORM3 cascade with the normalize steps replicated
   * MANUALLY (observation probe, not a pinned-semantics row set):
   * split, then per piece combine(), then for the combined piece the
   * recursive split + the per-piece removeIfCycle verdicts, so the
   * removal of the 8-split lower half is attributable to an exact
   * call. */
  static void caseNorm3p() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace a = ins(board, t, new Point[] {ip(30000, 20000), ip(40000, 20000)});
    PolylineTrace b = ins(board, t, new Point[] {ip(40000, 20000), ip(50000, 20000)});
    PolylineTrace c = ins(board, t, new Point[] {ip(35000, 10000), ip(35000, 30000)});
    System.out.println("--NORM3P-- probe A=" + a.getId() + " B=" + b.getId() + " C=" + c.getId());
    Collection<PolylineTrace> pieces = a.split((IntOctagon) null);
    dumpSplitPieces(pieces, "NORM3P_SPLIT");
    for (PolylineTrace piece : pieces) {
      if (!piece.isOnTheBoard()) {
        System.out.println("NORM3P_PIECE id=" + piece.getId() + " DEAD");
        continue;
      }
      boolean combined = piece.combine();
      System.out.println("NORM3P_COMBINE id=" + piece.getId() + " combined=" + combined
          + " alive=" + piece.isOnTheBoard()
          + (piece.isOnTheBoard() ? " corners=" + cornersOf(piece) : ""));
      if (combined && piece.isOnTheBoard()) {
        Collection<PolylineTrace> sp2 = piece.split((IntOctagon) null);
        dumpSplitPieces(sp2, "NORM3P_S" + piece.getId());
        for (PolylineTrace p2 : sp2) {
          if (p2.isOnTheBoard()) {
            System.out.println("NORM3P_RIC id=" + p2.getId()
                + " removed=" + board.removeIfCycle(p2)
                + " alive=" + p2.isOnTheBoard());
          }
        }
        dumpItems(board, "NORM3P_AFTER_RIC");
      }
    }
    dumpItems(board, "NORM3P_FINAL");
  }

  /** NORM4 — the depth cap: a staircase of collinear extensions C_i
   * (each joined at depth i) crossed by verticals V_i (each split at
   * depth i) drives normalizationDepth to 17+; the cap debug line
   * fires and the second pass is stable. */
  static void caseNorm4() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace z = ins(board, t, new Point[] {ip(15000, 20000), ip(17000, 20000)});
    // C_i for i = 1..18: collinear extension (15000+2000i)-(17000+2000i).
    for (int i = 1; i <= 18; i++) {
      ins(board, t, new Point[] {ip(15000 + 2000 * i, 20000),
          ip(17000 + 2000 * i, 20000)});
    }
    // V_i for i = 1..17: vertical at x = 16000+2000i, y 18000..22000
    // — crosses C_i's middle.
    for (int i = 1; i <= 17; i++) {
      ins(board, t, new Point[] {ip(16000 + 2000 * i, 18000),
          ip(16000 + 2000 * i, 22000)});
    }
    System.out.println("--NORM4-- depth cap Z=" + z.getId()
        + " items=" + board.getItems().size());
    boolean changed = z.normalize(null);
    System.out.println("NORM4_NORMALIZE " + changed);
    dumpItems(board, "NORM4_PASS1");
    // The stability double-call: normalize every alive trace again.
    // NOT all-false (an earlier draft comment claimed so): the depth
    // cap left the spiral head UN-merged in pass 1 (traces 47
    // [50000,20000 50000,22000], 53 [48000,20000 50000,20000], 115
    // [50000,20000 50000,25000] — 47/115 collinear-overlapping), and
    // pass 2 restarts at depth 0, so it merges the remainder:
    // 115/53/47 return true and the combined piece 116
    // [48000,20000 50000,20000 50000,25000] appears in PASS2.
    List<PolylineTrace> alive = new ArrayList<>();
    for (Item item : board.getItems()) {
      if (item instanceof PolylineTrace trace) {
        alive.add(trace);
      }
    }
    alive.sort((x, y) -> y.getId() - x.getId());
    StringBuilder sb = new StringBuilder();
    for (PolylineTrace trace : alive) {
      if (sb.length() > 0) {
        sb.append(",");
      }
      sb.append(trace.getId()).append("=").append(trace.normalize(null));
    }
    System.out.println("NORM4_SECOND_PASS " + sb);
    dumpItems(board, "NORM4_PASS2");
    witness(board, t, "NORM4");
  }

  /** NORM5A — a no-op normalize returns false. */
  static void caseNorm5a() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace k = ins(board, t, new Point[] {ip(60000, 35000), ip(65000, 35000)});
    System.out.println("--NORM5A-- no-op K=" + k.getId());
    System.out.println("NORM5A_NORMALIZE " + k.normalize(null));
    System.out.println("NORM5A_ALIVE " + k.isOnTheBoard()
        + " corners=" + cornersOf(k));
  }

  /** NORM5B — the last-entry drill split: split returns
   * [deleted this] (size 1) → normalize returns FALSE while
   * DELETING the trace. */
  static void caseNorm5b() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace d = ins(board, t, new Point[] {ip(30000, 40000), ip(50000, 40000)});
    System.out.println("--NORM5B-- drill split under normalize D=" + d.getId());
    boolean changed = d.normalize(null);
    System.out.println("NORM5B_NORMALIZE " + changed);
    System.out.println("NORM5B_D_ALIVE " + d.isOnTheBoard());
    dumpItems(board, "NORM5B_POST");
    witness(board, t, "NORM5B");
  }

  /** NORM5C — the first-entry drill split: split returns [] (size 0)
   * → normalize returns TRUE, trace deleted. */
  static void caseNorm5c() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace d = ins(board, t, new Point[] {ip(30000, 40000), ip(50000, 40000)});
    Via via = (Via) board.getItem(5);
    Via via2 = insVia(board, via, ip(45000, 40000));
    System.out.println("--NORM5C-- first-entry drill D=" + d.getId()
        + " via2=" + via2.getId());
    boolean changed = d.normalize(null);
    System.out.println("NORM5C_NORMALIZE " + changed);
    System.out.println("NORM5C_D_ALIVE " + d.isOnTheBoard());
    dumpItems(board, "NORM5C_POST");
    witness(board, t, "NORM5C");
  }

  /** PICK — BasicBoard.pickItems rows. */
  static void sectionPick() {
    BasicBoard board = fresh();
    System.out.println("--PICK-- pickItems(point, layer, null)");
    System.out.println("PICK_PIN_CENTER " + items(board.pickItems(ip(20000, 40000), 0, null)));
    System.out.println("PICK_VIA_CENTER " + items(board.pickItems(ip(40000, 40000), 0, null)));
    System.out.println("PICK_AREA " + items(board.pickItems(ip(55000, 15000), 0, null)));
    System.out.println("PICK_ALL_LAYERS "
        + items(board.pickItems(ip(20000, 40000), -1, null)));
    System.out.println("PICK_EMPTY " + items(board.pickItems(ip(0, 0), 0, null)));
  }

  /** REQ1 — the requery APPEND semantics vs no-requery (mutation M5:
   * deleting the requery entirely passed every earlier pin). A U-trace
   * B (legs at x=25000 and x=65000) crosses A twice; the FIRST found
   * split removes B, so only the RE-QUERY's appended piece entries can
   * trigger the split at the SECOND crossing. Java: requery appends
   * into the SAME accumulated list (ShapeSearchTree's walk only adds,
   * :437-438 — no clear) and rewinds to the head (:586-588): stale
   * entries re-walk as refused nulls, then the pieces' entries split
   * the second crossing. No-requery would leave B in 2 pieces; Java's
   * append gives 3. */
  static void caseReq1() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace b = ins(board, t, new Point[] {ip(25000, 20000), ip(25000, 50000),
      ip(65000, 50000), ip(65000, 20000)});
    PolylineTrace a = ins(board, t, new Point[] {ip(20000, 30000), ip(90000, 30000)});
    System.out.println("--REQ1-- requery second-crossing U B=" + b.getId()
        + " A=" + a.getId() + " corners=" + cornersOf(a));
    Collection<PolylineTrace> pieces = a.split((IntOctagon) null);
    dumpSplitPieces(pieces, "REQ1_RESULT");
    dumpItems(board, "REQ1_POST");
    witness(board, t, "REQ1");
  }

  /** REQ2 — append-and-rewind vs REPLACE (the quality-round finding):
   * the same U as REQ1 but A runs through the TEMPLATE via 5 at
   * (40000,40000). Java: the first found split requeries (append) and
   * rewinds; the re-walked OLD prefix reaches the via BEFORE the
   * appended piece entries, the drill split removes the receiver, and
   * the walk dies at the next loop-top — the second crossing is never
   * split (the piece entries sit at the list TAIL, behind the via).
   * A REPLACE requery would inline the pieces in fresh tree order and
   * (piece-before-via) split the second crossing — different ids and
   * post items. */
  static void caseReq2() {
    BasicBoard board = fresh();
    PolylineTrace t = tmpl(board);
    PolylineTrace b = ins(board, t, new Point[] {ip(25000, 20000), ip(25000, 50000),
      ip(65000, 50000), ip(65000, 20000)});
    PolylineTrace a = ins(board, t, new Point[] {ip(20000, 40000), ip(90000, 40000)});
    System.out.println("--REQ2-- append-vs-replace via-later U B=" + b.getId()
        + " A=" + a.getId() + " corners=" + cornersOf(a));
    Collection<PolylineTrace> pieces = a.split((IntOctagon) null);
    dumpSplitPieces(pieces, "REQ2_RESULT");
    dumpItems(board, "REQ2_POST");
    witness(board, t, "REQ2");
  }

  public static void main(String[] args) throws Exception {
    sectionB();
    caseX1();
    caseX2();
    caseX3();
    caseX4();
    caseX4L();
    caseDrl1();
    caseDrl2();
    casePad1();
    casePad2();
    casePad3();
    casePad4();
    casePad5();
    casePad6();
    caseAr1();
    caseAr2();
    caseDel1();
    caseDel2();
    caseClip1();
    caseClip2();
    caseNorm1();
    caseNorm2();
    caseNorm3();
    caseNorm3p();
    caseNorm4();
    caseNorm5a();
    caseNorm5b();
    caseNorm5c();
    caseReq1();
    caseReq2();
    sectionPick();
    System.out.println("--DONE--");
  }
}
