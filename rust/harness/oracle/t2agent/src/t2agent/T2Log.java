package t2agent;

import java.io.BufferedWriter;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.nio.file.StandardOpenOption;
import app.freerouting.geometry.planar.TileShape;
import app.freerouting.autoroute.expansion.IncompleteFreeSpaceExpansionRoom;
import app.freerouting.board.facade.BasicBoard;
import app.freerouting.board.model.items.Item;
import app.freerouting.board.searchtree.ShapeSearchTree;
import app.freerouting.datastructures.ShapeTree;
import app.freerouting.geometry.planar.IntBox;
import app.freerouting.geometry.planar.IntOctagon;

/**
 * SCRATCH (M5-T2, logs/-uncommitted): row sink for the live-process instrument. Never throws,
 * never mutates engine state; rows are flushed so a kill keeps the evidence.
 */
public final class T2Log {

  static volatile BufferedWriter out;
  static final Object LOCK = new Object();
  static volatile int rowCounter = 0;
  static volatile int dumpCount = 0;
  static volatile int maxRows = 2000000;
  static volatile int netFilter = 21;
  static volatile boolean installed = false;

  private T2Log() {}

  public static void init(String args) {
    String logPath = "logs/M5-T2/scratch/agent/t2agent.log";
    if (args != null) {
      for (String o : args.split(";")) {
        String t = o.trim();
        if (t.startsWith("log=")) {
          logPath = t.substring(4);
        } else if (t.startsWith("net=")) {
          netFilter = Integer.parseInt(t.substring(4));
        } else if (t.startsWith("maxrows=")) {
          maxRows = Integer.parseInt(t.substring(8));
        }
      }
    }
    try {
      Path p = Paths.get(logPath);
      if (p.getParent() != null) {
        Files.createDirectories(p.getParent());
      }
      out = Files.newBufferedWriter(
          p, StandardOpenOption.CREATE, StandardOpenOption.TRUNCATE_EXISTING,
          StandardOpenOption.WRITE);
      logRaw("# T2Log open " + logPath);
    } catch (Throwable e) {
      System.err.println("T2Log INIT FAILED: " + e);
    }
  }

  /** Raw sink for pre-installed lines. */
  public static void logRaw(String line) {
    synchronized (LOCK) {
      if (out == null) {
        return;
      }
      try {
        out.write(line);
        out.write('\n');
        out.flush();
      } catch (Throwable e) {
        System.err.println("T2Log WRITE FAILED: " + e);
      }
    }
  }

  static int nextRow() {
    synchronized (LOCK) {
      return ++rowCounter;
    }
  }

  /**
   * OnMethodEnter sink for completeShape. Returns the row number, or -1 when filtered out.
   */
  public static int enterRow(
      Object self, Object room, int netNumber, Object ignoreObject, Object ignoreShape) {
    if (netNumber != netFilter || rowCounter > maxRows) {
      return -1;
    }
    try {
      int row = nextRow();
      StringBuilder b = new StringBuilder(256);
      b.append("ENTER row=").append(row)
          .append(" tree=").append(System.identityHashCode(self))
          .append(" net=").append(netNumber);
      if (room instanceof app.freerouting.autoroute.expansion.FreeSpaceExpansionRoom fr) {
        b.append(" layer=").append(fr.getLayer());
        TileShape rs = fr.getShape();
        b.append(" in_shape=");
        appendShape(b, rs);
        if (room instanceof IncompleteFreeSpaceExpansionRoom ir) {
          b.append(" contained=");
          appendShape(b, ir.getContainedShape());
        }
      } else {
        b.append(" room_class=").append(room == null ? "null" : room.getClass().getName());
      }
      b.append(" ignore=").append(ignoreObject == null
          ? "null"
          : Integer.toString(System.identityHashCode(ignoreObject)));
      logRaw(b.toString());
      return row;
    } catch (Throwable t) {
      logRaw("# enterRow FAILED: " + t);
      return -1;
    }
  }

  /** OnMethodExit sink for completeShape. */
  public static void exitRow(
      Object self, Object room, int netNumber, Object result, int rowNo) {
    if (netNumber != netFilter) {
      return;
    }
    try {
      StringBuilder b = new StringBuilder(256);
      b.append("EXIT row=").append(rowNo)
          .append(" tree=").append(System.identityHashCode(self))
          .append(" net=").append(netNumber)
          .append(" size=").append(result == null ? -1 : ((java.util.Collection<?>) result).size());
      java.util.Collection<?> rooms = (java.util.Collection<?>) result;
      int i = 0;
      if (rooms != null) {
        for (Object r : rooms) {
          TileShape s = (r instanceof app.freerouting.autoroute.expansion.FreeSpaceExpansionRoom fr2)
              ? fr2.getShape()
              : null;
          b.append(" r").append(i).append('=');
          appendShape(b, s);
          i++;
        }
      }
      logRaw(b.toString());
      maybeDump(self, rooms);
    } catch (Throwable t) {
      logRaw("# exitRow FAILED: " + t);
    }
  }

  /** Appends an octagon's eight fields, or the bbox of a non-octagon TileShape, or "null". */
  static void appendShape(StringBuilder b, TileShape s) {
    if (s == null) {
      b.append("null");
    } else if (s instanceof IntOctagon o) {
      b.append("Oct(lx=").append(o.leftX)
          .append(",ly=").append(o.bottomY)
          .append(",rx=").append(o.rightX)
          .append(",uy=").append(o.topY)
          .append(",ulx=").append(o.upperLeftDiagonalX)
          .append(",lrx=").append(o.lowerRightDiagonalX)
          .append(",llx=").append(o.lowerLeftDiagonalX)
          .append(",urx=").append(o.upperRightDiagonalX).append(')');
    } else {
      IntBox bb = s.boundingBox();
      b.append("Box[(").append(bb.ll.x).append(',').append(bb.ll.y)
          .append(")..(").append(bb.ur.x).append(',').append(bb.ur.y).append(")]");
    }
  }

  /** Fork-face trigger: net-21 single-room result with the north edge inside the 189 band. */
  static void maybeDump(Object self, java.util.Collection<?> rooms) {
    try {
      if (rooms == null || rooms.size() != 1 || dumpCount >= 32) {
        return;
      }
      Object only = rooms.iterator().next();
      if (!(only instanceof app.freerouting.autoroute.expansion.FreeSpaceExpansionRoom fr)
          || !(fr.getShape() instanceof IntOctagon o)) {
        return;
      }
      if (o.topY > -911000 || o.topY < -911700) {
        return;
      }
      if (fr.getLayer() != 0) {
        return;
      }
      dumpCount++;
      logRaw("TRIGGER row_out_topY=" + o.topY + " dump=" + dumpCount);
      T2Dump.dump(self, o);
    } catch (Throwable t) {
      logRaw("# maybeDump FAILED: " + t);
    }
  }

  static void registerShutdown() {
    Runtime.getRuntime().addShutdownHook(new Thread(T2Log::closeSink));
  }

  static void closeSink() {
    synchronized (LOCK) {
      if (out != null) {
        try {
          out.write("# T2Log close rows=" + rowCounter + " dumps=" + dumpCount + "\n");
          out.flush();
          out.close();
        } catch (Throwable e) {
          System.err.println("T2Log CLOSE FAILED: " + e);
        }
        out = null;
      }
    }
  }
}
