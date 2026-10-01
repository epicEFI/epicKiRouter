package t2agent;

import java.io.BufferedWriter;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.nio.file.StandardOpenOption;
import app.freerouting.geometry.planar.TileShape;
import app.freerouting.autoroute.expansion.IncompleteFreeSpaceExpansionRoom;
import app.freerouting.geometry.planar.IntBox;
import app.freerouting.geometry.planar.IntOctagon;

/**
 * M5-T2 (buglog 189 lever (a); SOURCES COMMITTED at rust/harness/oracle/t2agent/ - the BUILT
 * FAT JAR is the uncommitted part, built per README.md): row sink for the live-process
 * instrument. Never throws, never mutates engine state; rows are flushed so a kill keeps the
 * evidence. Exceptional-exit face: the exit advice fires via ByteBuddy's default
 * onThrowable=true, so a thrown completeShape logs EXIT with size=-1 and no room columns and
 * the enter/exit row pairing stays balanced. Re-entrancy non-goal: completeShape does not
 * self-recur in the frozen engine (see the quality-review record); hypothetical nesting would
 * still pair by row number.
 */
public final class T2Log {

  static volatile BufferedWriter out;
  static final Object LOCK = new Object();
  static volatile int rowCounter = 0;
  static volatile int dumpCount = 0;

  // ---------------------------------------------------------------------------
  // Instrument options: the ONE home for every fork-tunable (the T3 surface).
  // Defaults reproduce the M5-T2 bm06 capture byte-for-byte; every value is
  // overridable via the -javaagent option string (semicolon-separated, parsed in
  // init(); e.g. -javaagent:t2agent.jar=log=/abs/run.log;net=21). This block and
  // the README's option table are one source of truth - edit both together.
  // ---------------------------------------------------------------------------
  /** Sink path (log=). Default: the M5-T2 evidence home, relative to the repo root. */
  static volatile String logPath = "logs/M5-T2/scratch/agent/t2agent.log";
  /** Net row filter (net=). Default: the bm06 fork net. */
  static volatile int netFilter = 21;
  /** Row budget (maxrows=). */
  static volatile int maxRows = 2000000;
  /** Dump-trigger upper band edge, EXCLUSIVE (bandhi=). */
  static volatile int bandHi = -911000;
  /** Dump-trigger lower band edge, EXCLUSIVE (bandlo=). */
  static volatile int bandLo = -911700;
  /** Dump-trigger layer (bandlayer=). */
  static volatile int bandLayer = 0;
  /** Max band dumps before the trigger stops firing (dumpcap=). */
  static volatile int dumpCap = 32;
  /** Dump band margin in DBU (margin=); feeds the DUMP header and the leaf walk bounds. */
  static volatile int margin = 2000;

  private T2Log() {}

  public static void init(String args) {
    if (args != null) {
      for (String o : args.split(";")) {
        String k = o.trim();
        try {
          if (k.startsWith("log=")) {
            logPath = k.substring(4);
          } else if (k.startsWith("net=")) {
            netFilter = Integer.parseInt(k.substring(4));
          } else if (k.startsWith("maxrows=")) {
            maxRows = Integer.parseInt(k.substring(8));
          } else if (k.startsWith("bandhi=")) {
            bandHi = Integer.parseInt(k.substring(7));
          } else if (k.startsWith("bandlo=")) {
            bandLo = Integer.parseInt(k.substring(7));
          } else if (k.startsWith("bandlayer=")) {
            bandLayer = Integer.parseInt(k.substring(10));
          } else if (k.startsWith("dumpcap=")) {
            dumpCap = Integer.parseInt(k.substring(8));
          } else if (k.startsWith("margin=")) {
            margin = Integer.parseInt(k.substring(7));
          }
        } catch (NumberFormatException e) {
          System.err.println("T2Log option parse warning: " + o);
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
    if (netNumber != netFilter || rowCounter >= maxRows) {
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
      java.util.Collection<?> rooms = null;
      if (result instanceof java.util.Collection<?> c) {
        rooms = c;
      }
      StringBuilder b = new StringBuilder(256);
      b.append("EXIT row=").append(rowNo)
          .append(" tree=").append(System.identityHashCode(self))
          .append(" net=").append(netNumber)
          .append(" size=").append(rooms == null ? -1 : rooms.size());
      int i = 0;
      for (Object r : rooms == null ? java.util.List.<Object>of() : rooms) {
        TileShape s = (r instanceof app.freerouting.autoroute.expansion.FreeSpaceExpansionRoom fr2)
            ? fr2.getShape()
            : null;
        b.append(" r").append(i).append('=');
        appendShape(b, s);
        i++;
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
      if (rooms == null || rooms.size() != 1 || dumpCount >= dumpCap) {
        return;
      }
      Object only = rooms.iterator().next();
      if (!(only instanceof app.freerouting.autoroute.expansion.FreeSpaceExpansionRoom fr)
          || !(fr.getShape() instanceof IntOctagon o)) {
        return;
      }
      if (o.topY >= bandHi || o.topY <= bandLo) {
        return;
      }
      if (fr.getLayer() != bandLayer) {
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
