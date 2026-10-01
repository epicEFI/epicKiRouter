// QuerySpike.java — jar spike for M2 Task 8 (the overlap query family:
// overlappingTreeEntries / overlappingObjects / overlappingItems, the
// with-clearance core and its T57 compensated dispatch, the T53
// octagon-skip, the T56 (int)(1.2*maxValue) truncation, the
// EntrySortedByClearance tie order, checkShape, validateEntries).
//
// Lives OUTSIDE src/ (harness-side; the frozen Java tree is never
// touched). Declares the app.freerouting.board.searchtree package for
// access to the package-private members the query core relies on, like
// TreeShapesSpike before it.
//
// Run (JDK 25, from the repo root):
//   mkdir -p /tmp/epic-t8-classes && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/javac -cp build/libs/freerouting-current-executable.jar \
//       -d /tmp/epic-t8-classes rust/harness/oracle/QuerySpike.java && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/java -cp build/libs/freerouting-current-executable.jar:/tmp/epic-t8-classes \
//       app.freerouting.board.searchtree.QuerySpike \
//       fixtures/Issue575-drc_dev-board_4_hole_clearance_violations.dsn \
//       | tee /tmp/epic-t8-query.out
//
// Sections:
//   A — Issue575, default tree: the plain entry query (layer filter,
//       ignore-nets incl. the net-0 quirk, entry ORDER = the
//       TreeSet<Leaf> candidate order), the box-vs-octagon exact-test
//       control, overlappingObjects.
//   B — T57: the 4-arg dispatch form vs the 5-arg core (flag off) and
//       vs the plain 3-arg form (flag on, after the rebuild).
//   C — crafted board: T56 truncation sweep along the diagonal face of
//       a 45-degree trace's stored octagon; the tie order; the layer
//       filter; the ignore-net filter on a net-carrying trace.
//   D — T53: a synthetic SearchTreeObject with an OCTAGON shape in a
//       real ShapeSearchTree90Degree — the skip fires for an octagon
//       query and not for the same-region box query.
//   E — validateEntries: true, poisoned (swapped slots) false,
//       restored true; the absent-array case (NPE, caught and dumped).
//   F — checkShape: bbox-containment reject, obstacle reject, the
//       ignore-net accept, free-space accept.
//
// Output discipline (project pin rules): exact ints via field access,
// doubles via Double.toString, never a formatting toString.
package app.freerouting.board.searchtree;

import app.freerouting.board.actions.ItemIdGenerator;
import app.freerouting.board.facade.BasicBoard;
import app.freerouting.board.model.items.Item;
import app.freerouting.board.model.items.Via;
import app.freerouting.datastructures.IdGenerator;
import app.freerouting.datastructures.ShapeTree;
import app.freerouting.geometry.planar.ConvexShape;
import app.freerouting.geometry.planar.FloatPoint;
import app.freerouting.geometry.planar.IntBox;
import app.freerouting.geometry.planar.IntOctagon;
import app.freerouting.geometry.planar.IntPoint;
import app.freerouting.geometry.planar.Point;
import app.freerouting.geometry.planar.Shape;
import app.freerouting.geometry.planar.TileShape;
import app.freerouting.io.BoardReadResult;
import app.freerouting.io.specctra.DsnReader;
import app.freerouting.rules.ClearanceMatrix;
import java.io.ByteArrayInputStream;
import java.nio.file.Files;
import java.nio.file.Paths;
import java.util.Collection;
import java.util.LinkedList;
import java.util.List;
import java.util.Set;

public final class QuerySpike {

  private QuerySpike() {}

  // ---------------------------------------------------------------------
  // helpers
  // ---------------------------------------------------------------------

  /** The exact-int renderer of TreeShapesSpike (box/oct/circle). */
  static String fmt(Shape shape) {
    if (shape instanceof IntBox b) {
      return "box[" + b.ll.x + " " + b.ll.y + " " + b.ur.x + " " + b.ur.y + "]";
    }
    if (shape instanceof IntOctagon o) {
      return "oct[" + o.leftX + " " + o.bottomY + " " + o.rightX + " " + o.topY + " "
          + o.upperLeftDiagonalX + " " + o.lowerRightDiagonalX + " " + o.lowerLeftDiagonalX + " "
          + o.upperRightDiagonalX + "]";
    }
    if (shape instanceof TileShape t) {
      // A Simplex (or other non-regular tile): render through its
      // bounding octagon, exactly like TreeShapesSpike's fmt.
      return "tile[" + fmt(t.boundingOctagon()) + "]";
    }
    return "shape[" + shape.getClass().getSimpleName() + "]";
  }

  /** Renders an entry collection as [(id,i),...] (ids via SearchTreeObject.getId). */
  static String entries(Collection<ShapeTree.TreeEntry> list) {
    StringBuilder sb = new StringBuilder("[");
    boolean first = true;
    for (ShapeTree.TreeEntry e : list) {
      if (!first) {
        sb.append(",");
      }
      first = false;
      sb.append("(").append(((SearchTreeObject) e.object).getId()).append("#")
          .append(e.shapeIndexInObject).append(")");
    }
    return sb.append("]").toString();
  }

  /** Renders a leaf collection (the raw candidate order) as [(id,i),...]. */
  static String leaves(Collection<ShapeTree.Leaf> list) {
    StringBuilder sb = new StringBuilder("[");
    boolean first = true;
    for (ShapeTree.Leaf leaf : list) {
      if (!first) {
        sb.append(",");
      }
      first = false;
      sb.append("(").append(((SearchTreeObject) leaf.object).getId()).append("#")
          .append(leaf.shapeIndexInObject).append(")");
    }
    return sb.append("]").toString();
  }

  /** Renders an object set as [id,...]. */
  static String objects(Set<SearchTreeObject> set) {
    StringBuilder sb = new StringBuilder("[");
    boolean first = true;
    for (SearchTreeObject o : set) {
      if (!first) {
        sb.append(",");
      }
      first = false;
      sb.append(o.getId());
    }
    return sb.append("]").toString();
  }

  /** Runs the 3-arg plain entry query into a fresh list. */
  static List<ShapeTree.TreeEntry> plainEntries(
      ShapeSearchTree tree, ConvexShape shape, int layer, int[] nets) {
    List<ShapeTree.TreeEntry> result = new LinkedList<>();
    tree.overlappingTreeEntries(shape, layer, nets, result);
    return result;
  }

  /** Runs the 5-arg with-clearance core into a fresh list. */
  static List<ShapeTree.TreeEntry> coreEntries(
      ShapeSearchTree tree, ConvexShape shape, int layer, int[] nets, int clClass) {
    List<ShapeTree.TreeEntry> result = new LinkedList<>();
    tree.overlappingTreeEntriesWithClearance(shape, layer, nets, clClass, result);
    return result;
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

  private static BasicBoard parseDsnText(String dsn, String name) {
    return parseDsnBytes(dsn.getBytes(), name);
  }

  /** An IntPoint center of an item (rounding a RationalPoint like Java would). */
  static IntPoint centerOf(Item item) {
    Point p = item instanceof Via via ? via.getCenter() : null;
    if (p == null) {
      return null;
    }
    return p instanceof IntPoint ip ? ip : p.toFloat().round();
  }

  // ---------------------------------------------------------------------
  // the synthetic T53 object: a SearchTreeObject with an OCTAGON shape
  // ---------------------------------------------------------------------

  /** Mirrors Item.compareTo (descending id) so the leaf sort matches items. */
  static final class OctaObstacle implements SearchTreeObject {
    final int id;
    final TileShape shape;
    ShapeTree.Leaf[] stored;

    OctaObstacle(int id, TileShape shape) {
      this.id = id;
      this.shape = shape;
    }

    @Override
    public int compareTo(Object other) {
      if (other instanceof OctaObstacle o) {
        return o.id - this.id;
      }
      return 1;
    }

    @Override
    public int treeShapeCount(ShapeTree tree) {
      return 1;
    }

    @Override
    public TileShape getTreeShape(ShapeTree tree, int index) {
      return shape;
    }

    @Override
    public void setSearchTreeEntries(ShapeTree.Leaf[] entries, ShapeTree tree) {
      this.stored = entries;
    }

    @Override
    public boolean isObstacle(int netNumber) {
      return true;
    }

    @Override
    public boolean isTraceObstacle(int netNumber) {
      return true;
    }

    @Override
    public int shapeLayer(int index) {
      return 0;
    }

    @Override
    public int getId() {
      return id;
    }
  }

  // ---------------------------------------------------------------------
  // sections
  // ---------------------------------------------------------------------

  static void sectionA(BasicBoard board) {
    System.out.println("--A-- plain entry queries, default tree, via 815 region");
    ShapeSearchTree tree = board.searchTreeManager.getDefaultTree();
    Via via = (Via) board.getItem(815);
    IntPoint c = centerOf(via);
    System.out.println("A_VIA id=815 class=" + via.clearanceClassIndex()
        + " nets=" + java.util.Arrays.toString(via.netNumbers)
        + " center=" + c.x + "," + c.y);
    IntBox qBox = new IntBox(new IntPoint(c.x - 150, c.y - 150), new IntPoint(c.x + 150, c.y + 150));
    IntOctagon qOct = qBox.boundingOctagon();
    System.out.println("A_QBOX " + fmt(qBox));
    System.out.println("A_QOCT " + fmt(qOct));

    // The raw candidate order (the TreeSet<Leaf> of MinAreaTree.overlaps).
    System.out.println("A_LEAVES box " + leaves(tree.overlaps(qBox.boundingOctagon())));
    System.out.println("A_LEAVES oct " + leaves(tree.overlaps(qOct)));

    // Layer filter + ignore-nets (incl. the net-0 quirk).
    int viaNet = via.netNumbers != null && via.netNumbers.length > 0 ? via.netNumbers[0] : 1;
    for (int layer : new int[] {-1, 0, 1}) {
      System.out.println("A_ENTRIES box layer=" + layer + " nets=[] "
          + entries(plainEntries(tree, qBox, layer, new int[0])));
      System.out.println("A_ENTRIES oct layer=" + layer + " nets=[] "
          + entries(plainEntries(tree, qOct, layer, new int[0])));
    }
    System.out.println("A_ENTRIES box layer=-1 nets=[" + viaNet + "] "
        + entries(plainEntries(tree, qBox, -1, new int[] {viaNet})));
    System.out.println("A_ENTRIES box layer=-1 nets=[0] "
        + entries(plainEntries(tree, qBox, -1, new int[] {0})));
    System.out.println("A_ENTRIES box layer=-1 nets=[9999] "
        + entries(plainEntries(tree, qBox, -1, new int[] {9999})));

    // overlappingObjects: the 2-arg TreeSet form.
    System.out.println("A_OBJECTS box layer=0 " + objects(tree.overlappingObjects(qBox, 0)));
    System.out.println("A_OBJECTS box layer=-1 " + objects(tree.overlappingObjects(qBox, -1)));
  }

  /** The T57 dispatch trap case: an AUTOROUTE class-1 tree while the
   * MANAGER flag is OFF — the tree's derived flag is what the dispatch
   * must read (treeFlag=true, managerFlag=false). A manager-flag port
   * takes the core branch here and diverges from the plain rows. */
  static void sectionB2(BasicBoard board) {
    System.out.println("--B2-- T57 dispatch on an autoroute class-1 tree, manager flag OFF");
    ShapeSearchTree t1 = board.searchTreeManager.getAutorouteTree(1);
    System.out.println("B2_TREE class=" + t1.getClass().getSimpleName()
        + " cc=" + t1.compensatedClearanceClassNo
        + " treeFlag=" + t1.isClearanceCompensationUsed()
        + " managerFlag=" + board.searchTreeManager.isClearanceCompensationUsed());
    Via via = (Via) board.getItem(815);
    IntPoint c = centerOf(via);
    IntBox qBox = new IntBox(new IntPoint(c.x - 150, c.y - 150), new IntPoint(c.x + 150, c.y + 150));
    System.out.println("B2_QBOX " + fmt(qBox));
    System.out.println("B2_4ARG_DISPATCH " + entries(
        new LinkedList<>(t1.overlappingTreeEntriesWithClearance(qBox, 0, new int[0], 1))));
    System.out.println("B2_PLAIN         " + entries(plainEntries(t1, qBox, 0, new int[0])));
    System.out.println("B2_CORE          " + entries(coreEntries(t1, qBox, 0, new int[0], 1)));
  }

  static void sectionB(BasicBoard board) {
    System.out.println("--B-- T57 dispatch, same query, both flag states");
    ShapeSearchTree tree = board.searchTreeManager.getDefaultTree();
    Via via = (Via) board.getItem(815);
    IntPoint c = centerOf(via);
    IntBox qBox = new IntBox(new IntPoint(c.x - 150, c.y - 150), new IntPoint(c.x + 150, c.y + 150));
    ClearanceMatrix m = board.rules.clearanceMatrix;
    System.out.println("B_FLAG before=" + tree.isClearanceCompensationUsed()
        + " maxValue(1,0)=" + m.maxValue(1, 0)
        + " maxClearance=(int)(1.2*that)= " + (int) (1.2 * m.maxValue(1, 0)));

    List<ShapeTree.TreeEntry> d4 = new LinkedList<>(tree.overlappingTreeEntriesWithClearance(
        qBox, 0, new int[0], 1));
    List<ShapeTree.TreeEntry> d5 = coreEntries(tree, qBox, 0, new int[0], 1);
    System.out.println("B4_CC0_DISPATCH " + entries(d4));
    System.out.println("B5_CC0_CORE     " + entries(d5));
    // The clearance annotation of each core entry (recomputed with the
    // exact formula the core used).
    StringBuilder ann = new StringBuilder("[");
    boolean first = true;
    for (ShapeTree.TreeEntry e : d5) {
      if (!first) {
        ann.append(",");
      }
      first = false;
      Item it = (Item) e.object;
      ann.append("(").append(it.getId()).append("#").append(e.shapeIndexInObject)
          .append(" cls=").append(it.clearanceClassIndex())
          .append(" c=").append(m.getValue(1, it.clearanceClassIndex(), 0, true)).append(")");
    }
    System.out.println("B5_CC0_CLEARANCES " + ann + "]");

    board.searchTreeManager.setClearanceCompensationUsed(true);
    ShapeSearchTree newTree = board.searchTreeManager.getDefaultTree();
    System.out.println("B_FLAG after=" + newTree.isClearanceCompensationUsed()
        + " sameObject=" + (newTree == tree));
    List<ShapeTree.TreeEntry> d4b = new LinkedList<>(newTree.overlappingTreeEntriesWithClearance(
        qBox, 0, new int[0], 1));
    List<ShapeTree.TreeEntry> plain = plainEntries(newTree, qBox, 0, new int[0]);
    System.out.println("B4_CC1_DISPATCH " + entries(d4b));
    System.out.println("B_PLAIN_CC1      " + entries(plain));
    // The DIVERGENCE witness: the 5-arg core on the compensated tree
    // would double-compensate (the offset window AND the stored shapes
    // already carry the compensation), so it must NOT agree with the
    // dispatched/plain rows on this dense fixture. A dispatch pin that
    // only checks dispatch == plain == core is anchor-blind.
    System.out.println("B5_CC1_CORE      " + entries(coreEntries(newTree, qBox, 0, new int[0], 1)));
  }

  static void sectionE(BasicBoard board) {
    System.out.println("--E-- validateEntries");
    ShapeSearchTree tree = board.searchTreeManager.getDefaultTree();
    Via via = (Via) board.getItem(815);
    System.out.println("E_TRUE manager=" + board.searchTreeManager.validateEntries(via)
        + " tree=" + tree.validateEntries(via));
    ShapeTree.Leaf[] arr = via.getSearchTreeEntries(tree);
    System.out.println("E_ARRAY len=" + arr.length);
    if (arr.length >= 2) {
      ShapeTree.Leaf tmp = arr[0];
      arr[0] = arr[1];
      arr[1] = tmp;
      System.out.println("E_POISONED manager=" + board.searchTreeManager.validateEntries(via)
          + " tree=" + tree.validateEntries(via));
      tmp = arr[0];
      arr[0] = arr[1];
      arr[1] = tmp;
      System.out.println("E_RESTORED manager=" + board.searchTreeManager.validateEntries(via)
          + " tree=" + tree.validateEntries(via));
    }
    // The absent case: an item with NO tree entries returns null from
    // getSearchTreeEntries and Java NPEs inside validateEntries.
    Item noEntries = null;
    for (Item item : board.getItems()) {
      if (item.getSearchTreeEntries(tree) == null) {
        noEntries = item;
        break;
      }
    }
    if (noEntries != null) {
      System.out.print("E_ABSENT id=" + noEntries.getId() + " → ");
      try {
        board.searchTreeManager.validateEntries(noEntries);
        System.out.println("returned true");
      } catch (NullPointerException npe) {
        System.out.println("NullPointerException");
      }
    } else {
      System.out.println("E_ABSENT none-found");
    }
  }

  /** The crafted board of sections C/D/F. */
  static final String CRAFTED_DSN =
      "(pcb t8-query.dsn\n"
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
      + "    (keepout (rect F.Cu 10000 10000 30000 20000))\n"
      + "    (keepout (rect F.Cu 70000 10000 90000 20000))\n"
      + "    (rule (width 250) (clearance 14))\n"
      + "  )\n"
      + "  (placement)\n"
      + "  (library)\n"
      + "  (network\n"
      + "    (net T8NET)\n"
      + "  )\n"
      + "  (wiring\n"
      + "    (wire (path F.Cu 250  10000 10000 40000 40000) (net T8NET))\n"
      + "  )\n"
      + ")\n";

  static void sectionC(BasicBoard board) {
    System.out.println("--C-- crafted board: T56 sweep, tie, layer, ignore");
    ClearanceMatrix m = board.rules.clearanceMatrix;
    System.out.println("C_FACTS items=" + board.getItems().size()
        + " classCount=" + m.getClassCount()
        + " defaultClass=" + app.freerouting.rules.BoardRules.defaultClearanceClass()
        + " maxValue(1,0)=" + m.maxValue(1, 0)
        + " maxValue(1,1)=" + m.maxValue(1, 1));
    for (Item item : board.getItems()) {
      System.out.println("C_ITEM id=" + item.getId() + " kind=" + item.getClass().getSimpleName()
          + " class=" + item.clearanceClassIndex()
          + " nets=" + java.util.Arrays.toString(item.netNumbers));
    }
    ShapeSearchTree tree = board.searchTreeManager.getDefaultTree();
    // The stored tree shapes of the trace (id 4) and keepout K1 (id 2).
    Item trace = board.getItem(4);
    Item k1 = board.getItem(2);
    Item k2 = board.getItem(3);
    System.out.println("C_TRACE_SHAPES n=" + trace.treeShapeCount(tree));
    for (int i = 0; i < trace.treeShapeCount(tree); i++) {
      System.out.println("C_TRACE_SHAPE i=" + i + " layer=" + trace.shapeLayer(i)
          + " shape=" + fmt(trace.getTreeShape(tree, i)));
    }
    System.out.println("C_K1_SHAPE " + fmt(k1.getTreeShape(tree, 0)));
    System.out.println("C_K2_SHAPE " + fmt(k2.getTreeShape(tree, 0)));
    System.out.println("C_T56 maxValue(1,0)=" + m.maxValue(1, 0)
        + " product=" + Double.toString(1.2 * m.maxValue(1, 0))
        + " (int)=" + (int) (1.2 * m.maxValue(1, 0)));
    System.out.println("C_T56_EXTRA (int)(1.2*2000)=" + (int) (1.2 * 2000));

    // The sweep: octagon queries marching up the diagonal past the
    // trace's far corner (40000,40000). The y base is shifted by 1 so
    // the trunc-vs-round flip differs: candidate iff (80001+2d) -
    // round(sqrt2*maxClearance) <= 80177; trunc(16.8)=16 -> diag 23 ->
    // d <= 99; round(16.8)=17 -> diag 24 -> d <= 100. d=100 is the
    // discriminating row.
    System.out.println("C_T56_OFFS offset(16): axis=16 diag="
        + 23 + " | a rounded port (17): diag=" + 24);
    for (int d = 60; d <= 130; d++) {
      IntBox b = new IntBox(new IntPoint(40000 + d, 40001 + d),
          new IntPoint(40100 + d, 40101 + d));
      IntOctagon q = b.boundingOctagon();
      System.out.println("C_SWEEP d=" + d + " → "
          + entries(coreEntries(tree, q, 0, new int[0], 1)));
    }

    // The tie: enlarge the matrix so the window spans both keepouts.
    m.setValue(1, 1, 0, 20000);
    System.out.println("C_TIE_SETUP maxValue(1,0)=" + m.maxValue(1, 0)
        + " (int)(1.2*that)=" + (int) (1.2 * m.maxValue(1, 0)));
    IntBox mid = new IntBox(new IntPoint(49500, 14000), new IntPoint(50500, 16000));
    IntOctagon midOct = mid.boundingOctagon();
    System.out.println("C_TIE_QUERY " + fmt(midOct));
    System.out.println("C_TIE layer=0 nets=[] → "
        + entries(coreEntries(tree, midOct, 0, new int[0], 1)));
    System.out.println("C_TIE_AGAIN → "
        + entries(coreEntries(tree, midOct, 0, new int[0], 1)));
    System.out.println("C_TIE layer=1 nets=[] → "
        + entries(coreEntries(tree, midOct, 1, new int[0], 1)));

    // The ignore-net filter on a net-carrying item (the trace, net
    // T8NET). Query right over the trace.
    int traceNet = trace.netNumbers.length > 0 ? trace.netNumbers[0] : 1;
    IntBox onTrace = new IntBox(new IntPoint(29750, 29750), new IntPoint(30250, 30250));
    IntOctagon onTraceOct = onTrace.boundingOctagon();
    System.out.println("C_TRACE_NET n=" + traceNet);
    System.out.println("C_IGNORE nets=[] → " + entries(coreEntries(tree, onTraceOct, 0, new int[0], 1)));
    System.out.println("C_IGNORE nets=[" + traceNet + "] → "
        + entries(coreEntries(tree, onTraceOct, 0, new int[] {traceNet}, 1)));
    System.out.println("C_IGNORE nets=[0] → "
        + entries(coreEntries(tree, onTraceOct, 0, new int[] {0}, 1)));
  }

  static void sectionD(BasicBoard board) {
    System.out.println("--D-- T53 octagon-skip, synthetic object in a 90-degree tree");
    IntOctagon o = new IntOctagon(0, 0, 1000, 1000, -500, 500, 0, 1500);
    ShapeSearchTree90Degree t90 = new ShapeSearchTree90Degree(board, 0);
    OctaObstacle obstacle = new OctaObstacle(7, o);
    t90.insert(obstacle);
    System.out.println("D_OCT " + fmt(o));
    System.out.println("D_OCT_BBOX " + fmt(o.boundingBox()));
    // Q: octagon whose bbox is inside O's leaf bounds but whose region
    // is disjoint from O (the corner cut: x+y > 1500).
    IntBox qb = new IntBox(new IntPoint(800, 800), new IntPoint(1000, 1000));
    IntOctagon q = qb.boundingOctagon();
    System.out.println("D_Q " + fmt(q));
    System.out.println("D_Q_BBOX " + fmt(q.boundingBox()));
    System.out.println("D_Q_INTERSECTS_O " + q.intersects(o));
    System.out.println("D_ENTRIES oct-query → " + entries(plainEntries(t90, q, -1, new int[0])));
    System.out.println("D_ENTRIES box-query → " + entries(plainEntries(t90, qb, -1, new int[0])));
    // Controls: a genuinely overlapping region.
    IntBox cb = new IntBox(new IntPoint(0, 0), new IntPoint(200, 200));
    System.out.println("D_CTRL oct → " + entries(plainEntries(t90, cb.boundingOctagon(), -1, new int[0])));
    System.out.println("D_CTRL box → " + entries(plainEntries(t90, cb, -1, new int[0])));
    // The production-tree control: in a 45-degree-directions tree the
    // leaf bounds ARE the octagon, so the disjoint query is not even a
    // candidate (the skip's effect is unreachable there).
    ShapeSearchTree45Degree t45 = new ShapeSearchTree45Degree(board, 0);
    OctaObstacle obstacle45 = new OctaObstacle(8, o);
    t45.insert(obstacle45);
    System.out.println("D_ENTRIES_45 oct-query → "
        + entries(plainEntries(t45, q, -1, new int[0])));
  }

  /** Section G — the DISCRIMINATING T57 rows. On the via-815 query of
   * sections B/B2 the plain, dispatched, and core forms all coincide
   * (captured there), so those rows cannot fail a wrong dispatch. Here
   * the crafted board's sweep query marches up the trace octagon's
   * diagonal face: plain drops out when the query's x+y min passes the
   * STORED urx, core drops out `clearance' later — and on the
   * compensated trees the stored urx itself is widened, so their
   * flip points sit at larger d. The rows where plain != core on each
   * tree state are the dispatch pin. */
  static void sectionG() {
    BasicBoard board = parseDsnText(CRAFTED_DSN, "t8-query.dsn");
    System.out.println("--G-- T57 discriminating dispatch rows (plain != core)");
    ShapeSearchTree t0 = board.searchTreeManager.getDefaultTree();
    System.out.println("G_T0 cc=" + t0.compensatedClearanceClassNo
        + " treeFlag=" + t0.isClearanceCompensationUsed()
        + " managerFlag=" + board.searchTreeManager.isClearanceCompensationUsed());
    for (int d = 85; d <= 115; d += 5) {
      IntOctagon q = sweepQuery(d);
      System.out.println("G_CC0 d=" + d + " 4ARG=" + entries(new LinkedList<>(
          t0.overlappingTreeEntriesWithClearance(q, 0, new int[0], 1)))
          + " PLAIN=" + entries(plainEntries(t0, q, 0, new int[0]))
          + " CORE=" + entries(coreEntries(t0, q, 0, new int[0], 1)));
    }
    ShapeSearchTree t1 = board.searchTreeManager.getAutorouteTree(1);
    System.out.println("G_T1 cc=" + t1.compensatedClearanceClassNo
        + " treeFlag=" + t1.isClearanceCompensationUsed()
        + " managerFlag=" + board.searchTreeManager.isClearanceCompensationUsed());
    for (int d = 85; d <= 115; d += 5) {
      IntOctagon q = sweepQuery(d);
      System.out.println("G_AR1 d=" + d + " 4ARG=" + entries(new LinkedList<>(
          t1.overlappingTreeEntriesWithClearance(q, 0, new int[0], 1)))
          + " PLAIN=" + entries(plainEntries(t1, q, 0, new int[0]))
          + " CORE=" + entries(coreEntries(t1, q, 0, new int[0], 1)));
    }
    board.searchTreeManager.setClearanceCompensationUsed(true);
    ShapeSearchTree t2 = board.searchTreeManager.getDefaultTree();
    System.out.println("G_T2 cc=" + t2.compensatedClearanceClassNo
        + " treeFlag=" + t2.isClearanceCompensationUsed()
        + " managerFlag=" + board.searchTreeManager.isClearanceCompensationUsed());
    for (int d = 85; d <= 115; d += 5) {
      IntOctagon q = sweepQuery(d);
      System.out.println("G_CC1 d=" + d + " 4ARG=" + entries(new LinkedList<>(
          t2.overlappingTreeEntriesWithClearance(q, 0, new int[0], 1)))
          + " PLAIN=" + entries(plainEntries(t2, q, 0, new int[0]))
          + " CORE=" + entries(coreEntries(t2, q, 0, new int[0], 1)));
    }
  }

  /** The section-C sweep query at offset d: a 100x100 box riding the
   * trace's diagonal end, y-shifted by 1 (the C sweep's parity form). */
  static IntOctagon sweepQuery(int d) {
    return new IntBox(new IntPoint(40000 + d, 40001 + d),
        new IntPoint(40100 + d, 40101 + d)).boundingOctagon();
  }

  static void sectionF() {
    // A FRESH parse: sectionC's matrix edit (v(1,1,0)=20000) would
    // move every checkShape clearance window to 24000 and swallow the
    // discriminating near/far cases below.
    BasicBoard board = parseDsnText(CRAFTED_DSN, "t8-query.dsn");
    System.out.println("--F-- checkShape");
    System.out.println("F_BBOX " + fmt(board.getBoundingBox()));
    IntBox overEdge = new IntBox(new IntPoint(99000, 10000), new IntPoint(102000, 20000));
    IntBox overKeepout = new IntBox(new IntPoint(18000, 14000), new IntPoint(22000, 16000));
    IntBox free = new IntBox(new IntPoint(45000, 30000), new IntPoint(46000, 31000));
    IntBox overTrace = new IntBox(new IntPoint(30000, 30000), new IntPoint(30500, 30500));
    int traceNet = board.getItem(4).netNumbers.length > 0 ? board.getItem(4).netNumbers[0] : 1;
    System.out.println("F_CASE over-edge layer=0 nets=[] → "
        + board.checkShape(overEdge, 0, new int[0], 1));
    System.out.println("F_CASE over-keepout layer=0 nets=[] → "
        + board.checkShape(overKeepout, 0, new int[0], 1));
    System.out.println("F_CASE over-keepout layer=1 nets=[] → "
        + board.checkShape(overKeepout, 1, new int[0], 1));
    System.out.println("F_CASE free layer=0 nets=[] → " + board.checkShape(free, 0, new int[0], 1));
    System.out.println("F_CASE over-trace layer=0 nets=[] → "
        + board.checkShape(overTrace, 0, new int[0], 1));
    System.out.println("F_CASE over-trace layer=0 nets=[" + traceNet + "] → "
        + board.checkShape(overTrace, 0, new int[] {traceNet}, 1));
  }

  public static void main(String[] args) throws Exception {
    String path =
        args.length >= 1
            ? args[0]
            : "fixtures/Issue575-drc_dev-board_4_hole_clearance_violations.dsn";
    BasicBoard board = parseDsnBytes(Files.readAllBytes(Paths.get(path)),
        Paths.get(path).getFileName().toString());
    if (board != null) {
      sectionA(board);
      // B2 first: getAutorouteTree(1) must run while the manager flag is
      // still OFF; sectionB's flip would rebuild the tree list after.
      sectionB2(board);
      sectionB(board);
      sectionE(board);
    }
    BasicBoard crafted = parseDsnText(CRAFTED_DSN, "t8-query.dsn");
    if (crafted != null) {
      sectionC(crafted);
      sectionD(crafted);
      sectionF();
      sectionG();
    }
  }
}
