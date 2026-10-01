package t2agent;

import app.freerouting.board.facade.BasicBoard;
import app.freerouting.board.model.items.Item;
import app.freerouting.board.searchtree.SearchTreeObject;
import app.freerouting.board.searchtree.ShapeSearchTree;
import app.freerouting.datastructures.ShapeTree;
import app.freerouting.geometry.planar.IntBox;
import app.freerouting.geometry.planar.IntOctagon;
import app.freerouting.geometry.planar.TileShape;
import java.lang.reflect.Field;

/**
 * M5-T2 (buglog 189 lever (a); SOURCES COMMITTED at rust/harness/oracle/t2agent/ - the BUILT
 * FAT JAR is the uncommitted part, built per README.md): the fork-row north-band dump. Read-only: reflectively reads the tree's root,
 * walks the node graph, and prints each band leaf's stored tree shape (the EXACT face
 * completeShape sees) and the clearance-matrix band rows. Tied to board.getHash() so the
 * dump names its board state.
 */
public final class T2Dump {

  static BasicBoard boardOf(ShapeSearchTree tree) {
    try {
      Field f = ShapeSearchTree.class.getDeclaredField("board");
      f.setAccessible(true);
      return (BasicBoard) f.get(tree);
    } catch (Throwable t) {
      T2Log.logRaw("# boardOf FAILED: " + t);
      return null;
    }
  }

  public static void dump(Object self, IntOctagon roomOut) {
    try {
      if (!(self instanceof ShapeSearchTree tree)) {
        T2Log.logRaw("# dump: self not a ShapeSearchTree");
        return;
      }
      BasicBoard board = boardOf(tree);
      String hash = "?";
      try {
        hash = String.valueOf(board.getHash());
      } catch (Throwable t) {
        // keep going; the dump's leaves matter more than the hash
      }
      int m = T2Log.margin;
      T2Log.logRaw("DUMP tree=" + System.identityHashCode(tree)
          + " ccl=" + tree.compensatedClearanceClassNo + " hash=" + hash
          + " bandY=[" + roomOut.topY + ".." + (roomOut.topY + m) + "]"
          + " bandX=[" + (roomOut.leftX - m) + ".." + (roomOut.rightX + m) + "]");
      Object root = readRoot(tree);
      if (root == null) {
        T2Log.logRaw("# dump: tree root is null");
        return;
      }
      long xlo = (long) roomOut.leftX - (long) m;
      long xhi = (long) roomOut.rightX + (long) m;
      long ylo = roomOut.topY;
      long yhi = (long) roomOut.topY + (long) m;
      walk(root, tree, xlo, xhi, ylo, yhi);
      dumpMatrix(board, tree);
      T2Log.logRaw("DUMP-END");
    } catch (Throwable t) {
      T2Log.logRaw("# dump FAILED: " + t);
    }
  }

  static Object readRoot(ShapeTree tree) {
    try {
      Field f = ShapeTree.class.getDeclaredField("root");
      f.setAccessible(true);
      return f.get(tree);
    } catch (Throwable t) {
      T2Log.logRaw("# readRoot FAILED: " + t);
      return null;
    }
  }

  static void walk(
      Object node, ShapeSearchTree tree, long xlo, long xhi, long ylo, long yhi) {
    try {
      if (node == null) {
        return;
      }
      if (node instanceof ShapeTree.InnerNode inner) {
        walk(inner.firstChild, tree, xlo, xhi, ylo, yhi);
        walk(inner.secondChild, tree, xlo, xhi, ylo, yhi);
        return;
      }
      if (node instanceof ShapeTree.Leaf leaf) {
        IntBox bb = leaf.boundingShape.boundingBox();
        boolean hit = bb.ur.x >= xlo && bb.ll.x <= xhi && bb.ur.y >= ylo && bb.ll.y <= yhi;
        if (!hit) {
          return;
        }
        leafRow(leaf, tree);
      }
    } catch (Throwable t) {
      T2Log.logRaw("# walk FAILED: " + t);
    }
  }

  static void leafRow(ShapeTree.Leaf leaf, ShapeSearchTree tree) {
    try {
      ShapeTree.Storable st = leaf.object;
      StringBuilder b = new StringBuilder(200);
      b.append("LFDUMP key=").append(System.identityHashCode(st))
          .append(" idx=").append(leaf.shapeIndexInObject)
          .append(" kind=").append(st.getClass().getSimpleName());
      if (st instanceof SearchTreeObject so) {
        b.append(" id=").append(so.getId()).append(" layer=").append(so.shapeLayer(leaf.shapeIndexInObject));
      }
      if (st instanceof Item it) {
        b.append(" nets=").append(java.util.Arrays.toString(it.netNumbers))
            .append(" cclass=").append(it.clearanceClassIndex());
      }
      b.append(" stored=");
      TileShape storedShape = st.getTreeShape(tree, leaf.shapeIndexInObject);
      T2Log.appendShape(b, storedShape);
      b.append(" leafBBox=");
      IntBox lb = leaf.boundingShape.boundingBox();
      b.append("Box[(").append(lb.ll.x).append(',').append(lb.ll.y)
          .append(")..(").append(lb.ur.x).append(',').append(lb.ur.y).append(")]");
      T2Log.logRaw(b.toString());
    } catch (Throwable t) {
      T2Log.logRaw("# leafRow FAILED: " + t);
    }
  }

  static void dumpMatrix(BasicBoard board, ShapeSearchTree tree) {
    try {
      Object rulesObj = board.getClass().getField("rules").get(board);
      Object cmObj = rulesObj.getClass().getField("clearanceMatrix").get(rulesObj);
      java.lang.reflect.Method getValue = cmObj.getClass().getMethod("getValue",
          int.class, int.class, int.class, boolean.class);
      java.lang.reflect.Method getClassCount = cmObj.getClass().getMethod("getClassCount");
      int classCount = (Integer) getClassCount.invoke(cmObj);
      int ccl = tree.compensatedClearanceClassNo;
      StringBuilder b = new StringBuilder(400);
      b.append("MATRIX ccl=").append(ccl);
      for (int c = 0; c < classCount; c++) {
        for (boolean margin : new boolean[] {false, true}) {
          b.append(" v(").append(c).append(',').append(ccl).append(',').append(margin).append(")=")
              .append(getValue.invoke(cmObj, c, ccl, 0, margin));
        }
      }
      T2Log.logRaw(b.toString());
      StringBuilder b2 = new StringBuilder(400);
      b2.append("MATRIX-REV ccl=").append(ccl);
      for (int c = 0; c < classCount; c++) {
        b2.append(" v(").append(ccl).append(',').append(c).append(")=")
            .append(getValue.invoke(cmObj, ccl, c, 0, false));
      }
      T2Log.logRaw(b2.toString());
    } catch (Throwable t) {
      T2Log.logRaw("# dumpMatrix FAILED: " + t);
    }
  }
}
