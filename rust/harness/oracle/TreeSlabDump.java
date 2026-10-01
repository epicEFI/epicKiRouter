// TreeSlabDump.java — M4-T6 DIAGNOSTIC ONLY (not part of any golden).
// Dumps every item tree shape (per compensated autoroute tree) whose
// bounding box intersects a slab, with EXACT octagon/box ints, for the
// wall-divergence triage (Java left cut 671300 vs Rust 671890).
//
// Build/run (JDK 25, from the repo root):
//   mkdir -p /tmp/epic-t6-classes && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/javac -cp build/libs/freerouting-current-executable.jar \
//       -d /tmp/epic-t6-classes rust/harness/oracle/TreeSlabDump.java && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/java -cp build/libs/freerouting-current-executable.jar:/tmp/epic-t6-classes \
//       app.freerouting.board.searchtree.TreeSlabDump \
//       rust/harness/fixtures/maze-spike/t7_ripup.dsn lx rx ly uy
package app.freerouting.board.searchtree;

import app.freerouting.board.actions.ItemIdGenerator;
import app.freerouting.board.facade.RoutingBoard;
import app.freerouting.board.model.items.Item;
import app.freerouting.datastructures.UndoableObjects;
import app.freerouting.geometry.planar.IntBox;
import app.freerouting.geometry.planar.IntOctagon;
import app.freerouting.geometry.planar.TileShape;
import app.freerouting.io.BoardReadResult;
import app.freerouting.io.specctra.DsnReader;
import java.io.ByteArrayInputStream;
import java.nio.file.Files;
import java.nio.file.Paths;

public final class TreeSlabDump {

  private TreeSlabDump() {}

  public static void main(String[] args) throws Exception {
    byte[] bytes = Files.readAllBytes(Paths.get(args[0]));
    String fileName = Paths.get(args[0]).getFileName().toString();
    BoardReadResult read =
        DsnReader.readBoard(
            new ByteArrayInputStream(bytes), null, new ItemIdGenerator(), fileName);
    if (!(read instanceof BoardReadResult.Success success)) {
      throw new IllegalStateException("parse failed");
    }
    RoutingBoard board = (RoutingBoard) success.board();
    board.searchTreeManager.reinsertTreeItems();

    int slabLx = Integer.parseInt(args[1]);
    int slabRx = Integer.parseInt(args[2]);
    int slabLy = Integer.parseInt(args[3]);
    int slabUy = Integer.parseInt(args[4]);

    for (int cc = 0; cc <= 1; cc++) {
      ShapeSearchTree tree = board.searchTreeManager.getAutorouteTree(cc);
      System.out.println(
          "TREE_SLAB tree cc="
              + cc
              + " class="
              + tree.getClass().getSimpleName()
              + " compensated="
              + tree.compensatedClearanceClassNo);
      java.util.Iterator<UndoableObjects.UndoableObjectNode> it =
          board.itemList.startReadObject();
      for (;;) {
        Item item = (Item) board.itemList.readObject(it);
        if (item == null) {
          break;
        }
        int n = item.treeShapeCount(tree);
        for (int i = 0; i < n; i++) {
          TileShape s = item.getTreeShape(tree, i);
          if (s == null) {
            continue;
          }
          IntBox b = s.boundingBox();
          if (b.ur.x < slabLx || b.ll.x > slabRx || b.ur.y < slabLy || b.ll.y > slabUy) {
            continue;
          }
          StringBuilder sb = new StringBuilder();
          sb.append("TREE_SLAB cc=")
              .append(cc)
              .append(" id=")
              .append(item.getId())
              .append(" cls=")
              .append(item.getClass().getSimpleName())
              .append(" nets=")
              .append(java.util.Arrays.toString(item.netNumbers))
              .append(" layer=")
              .append(item.shapeLayer(i))
              .append(" shapeIdx=")
              .append(i)
              .append(" bbox=[(")
              .append(b.ll.x)
              .append(",")
              .append(b.ll.y)
              .append(")..(")
              .append(b.ur.x)
              .append(",")
              .append(b.ur.y)
              .append(")]");
          if (s instanceof IntOctagon oct) {
            sb.append(" oct=[lx=")
                .append(oct.leftX)
                .append(" ly=")
                .append(oct.bottomY)
                .append(" rx=")
                .append(oct.rightX)
                .append(" uy=")
                .append(oct.topY)
                .append(" ulx=")
                .append(oct.upperLeftDiagonalX)
                .append(" lrx=")
                .append(oct.lowerRightDiagonalX)
                .append(" llx=")
                .append(oct.lowerLeftDiagonalX)
                .append(" urx=")
                .append(oct.upperRightDiagonalX)
                .append("]");
          } else {
            sb.append(" other=").append(s);
          }
          System.out.println(sb);
        }
      }
    }
  }
}
