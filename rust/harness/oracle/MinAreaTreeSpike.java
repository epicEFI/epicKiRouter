package app.freerouting.datastructures;

import app.freerouting.geometry.planar.FortyfiveDegreeBoundingDirections;
import app.freerouting.geometry.planar.IntBox;
import app.freerouting.geometry.planar.IntOctagon;
import app.freerouting.geometry.planar.IntPoint;
import app.freerouting.geometry.planar.OrthogonalBoundingDirections;
import app.freerouting.geometry.planar.RegularTileShape;
import app.freerouting.geometry.planar.ShapeBoundingDirections;
import app.freerouting.geometry.planar.TileShape;
import java.util.Set;

/**
 * M2 Task 5 jar spike: pins the exact MinAreaTree/ShapeTree semantics
 * (T50 eager-union + tie descent, T51 history-dependent structure and the
 * strict-shrink remove loop, T52 overlaps sorted-set behavior) by driving
 * scripted insert/remove sequences and printing the FULL tree structure
 * after every step:
 *
 *   - pre-order dump of every node: `I <bounds>` / `L obj=<id> idx=<i>
 *     <bounds>`, children in first/second order
 *   - per inner node the recomputed TIGHT bounds of its children and a
 *     STALE marker when it differs (the T51 staleness probe: does a pure
 *     insert/remove sequence ever leave non-tight inner bounds?)
 *   - `size()` and the `toArray()` in-order leaf order `(id,idx),...`
 *   - `overlaps(query)` result iteration order (TreeSet by Leaf.compareTo)
 *
 * Declares the `app.freerouting.datastructures` package for access to the
 * protected `root` field and the package-private `TreeNode.parent` (the
 * M2 plan sanctions package declarations for oracle launchers).
 *
 * The Storable impl mirrors Item.compareTo (Item.java:94-103):
 * `item.id - id` — DESCENDING id.
 *
 * Run (JDK 25, from the repo root; the package declaration requires javac,
 * the single-file launcher rejects package/path mismatch):
 *   mkdir -p /tmp/epic-t5-classes && \
 *   ~/.jdks/jdk-25.0.4.1+1/bin/javac -cp build/libs/freerouting-current-executable.jar \
 *       -d /tmp/epic-t5-classes rust/harness/oracle/MinAreaTreeSpike.java && \
 *   ~/.jdks/jdk-25.0.4.1+1/bin/java -cp build/libs/freerouting-current-executable.jar:/tmp/epic-t5-classes \
 *       app.freerouting.datastructures.MinAreaTreeSpike > /tmp/epic-t5-tree.out
 */
public final class MinAreaTreeSpike {

  /** Storable mirror of Item: (id, shapes), compareTo descending id. */
  static final class BoxObj implements ShapeTree.Storable {
    final int id;
    final TileShape[] shapes;
    ShapeTree.Leaf[] entries;

    BoxObj(int id, TileShape... shapes) {
      this.id = id;
      this.shapes = shapes;
    }

    @Override
    public int compareTo(Object other) {
      // Item.java:94-103 verbatim shape: item.id - id (descending).
      if (other instanceof BoxObj obj) {
        return obj.id - id;
      }
      return 1;
    }

    @Override
    public int treeShapeCount(ShapeTree shapeTree) {
      return shapes.length;
    }

    @Override
    public TileShape getTreeShape(ShapeTree tree, int index) {
      return shapes[index];
    }

    @Override
    public void setSearchTreeEntries(ShapeTree.Leaf[] entries, ShapeTree tree) {
      this.entries = entries;
    }
  }

  // ---- construction helpers -------------------------------------------------

  private static IntBox box(int llx, int lly, int urx, int ury) {
    return new IntBox(new IntPoint(llx, lly), new IntPoint(urx, ury));
  }

  /** A true (non-hull) octagon: box [llx,urx]x[lly,ury] with 45-degree corners cut by `cut`. */
  private static IntOctagon cutOctagon(int llx, int lly, int urx, int ury, int cut) {
    // Octagon bounds: verticals at llx/urx, horizontals at lly/ury, diagonals
    // cutting `cut` off each corner in 45 degrees.
    // ul diagonal (y = -x + c, c = ulx): passes (llx, lly+cut) and (llx+cut, lly)
    //   -> c = llx + lly + cut.
    // lr diagonal (y = -x + c): passes (urx, ury-cut) and (urx-cut, ury) -> c = urx + ury - cut.
    // ll diagonal (y = x + c, c = llx intercept): passes (llx, ury-cut) and (llx+cut, ury)
    //   -> c = llx - ury + cut.
    // ur diagonal (y = x + c): passes (urx, lly+cut) and (urx-cut, lly) -> c = urx - lly - cut.
    return new IntOctagon(
        llx,
        lly,
        urx,
        ury,
        llx + lly + cut,
        urx + ury - cut,
        llx - ury + cut,
        urx - lly - cut);
  }

  private static String fmt(RegularTileShape s) {
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

  // ---- dump -----------------------------------------------------------------

  private final MinAreaTree tree;

  private MinAreaTreeSpike(ShapeBoundingDirections dirs) {
    this.tree = new MinAreaTree(dirs);
  }

  /** Full dump: pre-order structure + tightness probe + toArray order. */
  private void dump(String label) {
    System.out.println("  dump " + label + " size=" + tree.size());
    ShapeTree.Leaf[] arr = tree.toArray();
    StringBuilder order = new StringBuilder("    toArray=[");
    for (int i = 0; i < arr.length; i++) {
      if (i > 0) {
        order.append(',');
      }
      order.append('(').append(((BoxObj) arr[i].object).id).append(',').append(
              arr[i].shapeIndexInObject)
          .append(')');
    }
    order.append(']');
    System.out.println(order);
    if (tree.root == null) {
      System.out.println("    <empty>");
      return;
    }
    StringBuilder sb = new StringBuilder();
    dumpNode(sb, tree.root, 1);
    System.out.print(sb);
  }

  private void dumpNode(StringBuilder sb, ShapeTree.TreeNode node, int depth) {
    sb.append("    ".repeat(depth));
    if (node instanceof ShapeTree.Leaf leaf) {
      sb.append("L obj=")
          .append(((BoxObj) leaf.object).id)
          .append(" idx=")
          .append(leaf.shapeIndexInObject)
          .append(' ')
          .append(fmt(leaf.boundingShape))
          .append('\n');
      return;
    }
    ShapeTree.InnerNode inner = (ShapeTree.InnerNode) node;
    // The remove loop's own recomputation shape (MinAreaTree.java:169-171):
    // secondChild.boundingShape.union(firstChild.boundingShape).
    RegularTileShape tight =
        inner.secondChild.boundingShape.union(inner.firstChild.boundingShape);
    sb.append("I ")
        .append(fmt(inner.boundingShape))
        .append(" tight=")
        .append(fmt(tight))
        // IntBox/IntOctagon do not override equals — compare the canonical
        // strings (value equality), never .equals (identity).
        .append(fmt(tight).equals(fmt(inner.boundingShape)) ? "" : " STALE")
        .append('\n');
    dumpNode(sb, inner.firstChild, depth + 1);
    dumpNode(sb, inner.secondChild, depth + 1);
  }

  private void insert(BoxObj obj) {
    tree.insert(obj);
  }

  private void remove(BoxObj obj, int shapeIndex) {
    tree.removeLeaf(obj.entries[shapeIndex]);
  }

  private void overlaps(String label, RegularTileShape query) {
    Set<ShapeTree.Leaf> found = tree.overlaps(query);
    StringBuilder sb = new StringBuilder("  overlaps " + label + " q=" + fmt(query) + " -> [");
    boolean first = true;
    for (ShapeTree.Leaf leaf : found) {
      if (!first) {
        sb.append(',');
      }
      first = false;
      sb.append('(')
          .append(((BoxObj) leaf.object).id)
          .append(',')
          .append(leaf.shapeIndexInObject)
          .append(')');
    }
    sb.append(']');
    System.out.println(sb);
  }

  // ---- scenarios ------------------------------------------------------------

  private void run() {
    // Each scenario gets a FRESH tree (no state leak between scenarios).
    newSpike().scenarioTieOrthogonal();
    newSpike().scenarioEagerUnionRight();
    newSpike().scenarioEagerUnionDeep();
    newSpike().scenarioHistoryVsFresh();
    newSpike().scenarioShrinkLoop();
    newSpike().scenarioRemoveEdges();
    newSpike().scenarioOverlapsDedupOrder();
    newSpike().scenarioFortyfiveMixed();
  }

  private static MinAreaTreeSpike newSpike() {
    return new MinAreaTreeSpike(OrthogonalBoundingDirections.INSTANCE);
  }

  /** S1: exact area-increase tie at the root must descend FIRST (T50). */
  private void scenarioTieOrthogonal() {
    System.out.println("SCEN tie_orth");
    tree.insert(new BoxObj(1, box(0, 0, 10, 10)));
    tree.insert(new BoxObj(2, box(100, 0, 110, 10)));
    dump("after 1,2");
    insert(new BoxObj(3, box(50, 0, 60, 10)));
    dump("after 3 (tie 500/500 -> first)");
    overlaps("tie-center", box(45, 0, 65, 10));
  }

  /** S2: insert descending right at the root; the root bounds must still grow. */
  private void scenarioEagerUnionRight() {
    System.out.println("SCEN eager_right_orth");
    insert(new BoxObj(1, box(0, 0, 10, 10)));
    insert(new BoxObj(2, box(20, 0, 30, 10)));
    dump("after 1,2");
    insert(new BoxObj(3, box(25, -5, 40, 5)));
    dump("after 3 (descends right, root must grow to [0 40]x[-5 10])");
  }

  /** S3: two-level descent; every VISITED inner node grows eagerly. */
  private void scenarioEagerUnionDeep() {
    System.out.println("SCEN eager_deep_orth");
    insert(new BoxObj(1, box(0, 0, 10, 10)));
    insert(new BoxObj(2, box(20, 0, 30, 10)));
    insert(new BoxObj(3, box(0, 100, 10, 110)));
    insert(new BoxObj(4, box(25, 95, 35, 105)));
    dump("after 1..4");
    insert(new BoxObj(5, box(2, 108, 4, 120)));
    dump("after 5 (path visits root then N1; both must grow)");
  }

  /**
   * S4: history dependence (T51): A,B,C,D inserted, A removed (sibling promotion
   * + strict-shrink), E re-inserted; the dump must differ from the fresh
   * B,C,D,E build in the same scenario.
   */
  private void scenarioHistoryVsFresh() {
    System.out.println("SCEN history_orth");
    BoxObj a = new BoxObj(1, box(0, 0, 10, 10));
    BoxObj b = new BoxObj(2, box(100, 0, 110, 10));
    BoxObj c = new BoxObj(3, box(45, 0, 55, 10));
    BoxObj d = new BoxObj(4, box(0, 100, 10, 110));
    insert(a);
    insert(b);
    insert(c);
    insert(d);
    dump("after A B C D");
    remove(a, 0);
    dump("after remove A");
    insert(new BoxObj(5, box(0, 0, 10, 10)));
    dump("after insert E");
    System.out.println("SCEN fresh_orth");
    MinAreaTreeSpike fresh = new MinAreaTreeSpike(OrthogonalBoundingDirections.INSTANCE);
    fresh.insert(new BoxObj(2, box(100, 0, 110, 10)));
    fresh.insert(new BoxObj(3, box(45, 0, 55, 10)));
    fresh.insert(new BoxObj(4, box(0, 100, 10, 110)));
    fresh.insert(new BoxObj(5, box(0, 0, 10, 10)));
    fresh.dump("fresh B C D E");
  }

  /**
   * S4b: the strict-shrink ancestor loop must actually ASSIGN (T51): after
   * the eager_deep layout, removing leaf 5 (whose extent is not covered by
   * its sibling or uncle) shrinks the grandparent AND the root; removing
   * leaf 4 then shrinks the right subtree and the root again.
   */
  private void scenarioShrinkLoop() {
    System.out.println("SCEN shrink_loop_orth");
    BoxObj l1 = new BoxObj(1, box(0, 0, 10, 10));
    BoxObj l2 = new BoxObj(2, box(20, 0, 30, 10));
    BoxObj l3 = new BoxObj(3, box(0, 100, 10, 110));
    BoxObj l4 = new BoxObj(4, box(25, 95, 35, 105));
    BoxObj l5 = new BoxObj(5, box(2, 108, 4, 120));
    insert(l1);
    insert(l2);
    insert(l3);
    insert(l4);
    insert(l5);
    dump("after 1..5");
    remove(l5, 0);
    dump("after remove 5 (two ancestor levels must shrink)");
    remove(l4, 0);
    dump("after remove 4 (right subtree + root shrink again)");
  }

  /** S5: remove-to-empty, remove-root-single-leaf, overlaps on empty. */
  private void scenarioRemoveEdges() {
    System.out.println("SCEN remove_edges_orth");
    BoxObj one = new BoxObj(1, box(0, 0, 10, 10));
    BoxObj two = new BoxObj(2, box(20, 0, 30, 10));
    insert(one);
    insert(two);
    dump("after 1,2");
    overlaps("two-leaf", box(0, 0, 30, 10));
    remove(one, 0);
    dump("after remove 1 (sibling promoted to root)");
    remove(two, 0);
    dump("after remove 2 (empty)");
    overlaps("empty", box(0, 0, 100, 100));
    BoxObj solo = new BoxObj(9, box(50, 50, 60, 60));
    insert(solo);
    dump("after insert solo");
    overlaps("solo", box(55, 55, 65, 65));
    remove(solo, 0);
    dump("after remove solo (root leaf -> empty)");
  }

  /** S6: overlaps dedup (same object inserted twice) + descending-id order. */
  private void scenarioOverlapsDedupOrder() {
    System.out.println("SCEN overlaps_dedup_orth");
    BoxObj seven = new BoxObj(7, box(0, 0, 10, 10), box(20, 0, 30, 10));
    BoxObj three = new BoxObj(3, box(15, 0, 18, 10));
    BoxObj nine = new BoxObj(9, box(500, 0, 510, 10));
    insert(seven);
    insert(three);
    insert(nine);
    dump("after 7,3,9");
    overlaps("wide", box(5, 0, 40, 10));
    overlaps("border-touch", box(10, 0, 20, 10));
    // Insert the SAME object again: 4 leaves, two per (7,i) — the TreeSet
    // must still hold each (7,i) once (dedup on Leaf.compareTo == 0).
    insert(seven);
    dump("after re-insert 7");
    overlaps("wide-again", box(5, 0, 40, 10));
  }

  /**
   * S7: 45-degree tree — box tree-shapes are stored as octagon hulls (the
   * real board configuration), plus one TRUE octagon leaf and mixed
   * octagon/box overlap queries (the (Box,Oct)/(Oct,Box) intersects arms).
   */
  private void scenarioFortyfiveMixed() {
    System.out.println("SCEN fortyfive_mixed");
    MinAreaTreeSpike spike = new MinAreaTreeSpike(FortyfiveDegreeBoundingDirections.INSTANCE);
    spike.insert(new BoxObj(1, box(0, 0, 10, 10)));
    spike.insert(new BoxObj(2, box(100, 0, 110, 10)));
    spike.insert(new BoxObj(3, box(50, 0, 60, 10)));
    spike.dump("after 1,2,3 (octagon-hull tie)");
    IntOctagon oct = cutOctagon(200, 0, 260, 60, 20);
    spike.insert(new BoxObj(4, oct));
    spike.dump("after 4 (true octagon)");
    spike.overlaps("box-vs-oct-leaf", box(230, 30, 240, 40));
    spike.overlaps("oct-query-vs-boxes", cutOctagon(5, -5, 15, 15, 5));
  }

  public static void main(String[] args) {
    MinAreaTreeSpike spike = new MinAreaTreeSpike(OrthogonalBoundingDirections.INSTANCE);
    spike.run();
  }
}
