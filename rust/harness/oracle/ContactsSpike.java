// ContactsSpike.java — jar spike for M2 Task 10 (the trace CONTACTS
// seam: Trace.getStartContacts / getEndContacts / getNormalContacts,
// Trace.java:104-203).
//
// Lives OUTSIDE src/ (harness-side; the frozen Java tree is never
// touched). Declares the Trace package app.freerouting.board.model.items
// (everything the spike reads is public; the package keeps it beside
// the code it pins, like QuerySpike in board.searchtree).
//
// Run (JDK 25, from the repo root):
//   mkdir -p /tmp/epic-t10-classes && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/javac -cp build/libs/freerouting-current-executable.jar \
//       -d /tmp/epic-t10-classes rust/harness/oracle/ContactsSpike.java && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/java -cp build/libs/freerouting-current-executable.jar:/tmp/epic-t10-classes \
//       app.freerouting.board.model.items.ContactsSpike \
//       fixtures/Issue163-pic_programmer.dsn \
//       | tee /tmp/epic-t10-contacts.out
//
// The spiked fixture is Issue163-pic_programmer.dsn (297 wires, 270
// pins, 7 vias — trace endpoints land on pads and vias).
//
// Sections (output line prefixes):
//   S    — pic_programmer, first 6 traces by DESCENDING id: start/end
//          contact sets as (id:kind) in TreeSet iteration order, plus
//          the exact-int corners. S_ALL is the no-arg union form.
//   SC   — the SAME rows after setClearanceCompensationUsed(true):
//          the compensation-invariance witness (membership unchanged;
//          the per-kind checks are exact corner/center equality).
//   M    — the mid-corner pair scan on pic_programmer: a trace Y whose
//          endpoint sits on a MID corner of a longer trace X. Y's
//          contacts at that endpoint must EXCLUDE X.
//   C    — the crafted trap board: full item dump, then per-trap rows
//          (mid-corner, corner guard, foreign net both flags, order,
//          conduction-area contains incl. the border point, keepout
//          candidate-but-never-contact with the raw candidate set).
//   R    — re-query after a same-corner insertion on the crafted
//          board: the COMPUTE-ON-DEMAND witness (no cache field in
//          Trace.java; a fresh query sees the new item).
//   N    — net-0 traces inserted via insertTraceWithoutCleaning: net 0
//          IS a shareable net for sharesNetNo (plain array
//          intersection) — two net-0 traces contact each other — while
//          isObstacle(0) stays true (the T8 contrast row).
//   C2/CC1 — a FRESH parse of the crafted board re-dumped before and
//          after the compensation flip (R/N never ran on this parse).
//   X    — TileShape.getInstance(point)'s runtime class (closes the
//          javadoc-says-IntOctagon / code-returns-IntBox question).
//
// Output discipline (project pin rules): exact ints via field access,
// never a formatting toString.
package app.freerouting.board.model.items;

import app.freerouting.board.actions.ItemIdGenerator;
import app.freerouting.board.facade.BasicBoard;
import app.freerouting.board.model.structure.BoardOutline;
import app.freerouting.board.model.structure.FixedState;
import app.freerouting.board.trace.PolylineTrace;
import app.freerouting.datastructures.IdGenerator;
import app.freerouting.geometry.planar.IntBox;
import app.freerouting.geometry.planar.IntPoint;
import app.freerouting.geometry.planar.Point;
import app.freerouting.geometry.planar.Polyline;
import app.freerouting.geometry.planar.TileShape;
import app.freerouting.io.BoardReadResult;
import app.freerouting.io.specctra.DsnReader;
import java.io.ByteArrayInputStream;
import java.nio.file.Files;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;

public final class ContactsSpike {

  private ContactsSpike() {}

  // ---------------------------------------------------------------------
  // helpers
  // ---------------------------------------------------------------------

  /** One-letter kind annotation per contact (never the object identity). */
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
    if (item instanceof ComponentOutline) {
      return "CO";
    }
    if (item instanceof ObstacleArea) {
      return "K";
    }
    return "?";
  }

  /** Renders a contact set as [(id:kind),...] in its own iteration order. */
  static String contacts(Set<Item> set) {
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

  /** Renders a candidate object set as [id:kind,...]. */
  static String objects(Set<?> set) {
    StringBuilder sb = new StringBuilder("[");
    boolean first = true;
    for (Object o : set) {
      if (!first) {
        sb.append(",");
      }
      first = false;
      Item item = (Item) o;
      sb.append(item.getId()).append(":").append(kind(item));
    }
    return sb.append("]").toString();
  }

  /** An exact-int "x,y" for a Point (IntPoints render raw). */
  static String xy(Point p) {
    if (p instanceof IntPoint ip) {
      return ip.x + "," + ip.y;
    }
    return p.toString();
  }

  static String nets(Item item) {
    StringBuilder sb = new StringBuilder("[");
    for (int i = 0; i < item.netNumbers.length; i++) {
      if (i > 0) {
        sb.append(",");
      }
      sb.append(item.netNumbers[i]);
    }
    return sb.append("]").toString();
  }

  /** All traces of the board, DESCENDING id (the T60 order). */
  static List<PolylineTrace> tracesDescending(BasicBoard board) {
    List<PolylineTrace> result = new ArrayList<>();
    for (Item item : board.getItems()) {
      if (item instanceof PolylineTrace trace) {
        result.add(trace);
      }
    }
    result.sort((a, b) -> b.getId() - a.getId());
    return result;
  }

  /** The contact rows for one trace (prefix distinguishes re-dumps).
   * Null-safe: a null trace (signature-map miss after a parse or
   * normalize divergence) prints clean rows instead of an NPE that
   * would truncate every later capture section. */
  static void dumpTraceRows(String prefix, PolylineTrace trace) {
    if (trace == null) {
      System.out.println(prefix + " id=null (lookup failed)");
      System.out.println(prefix + "_ALL id=null (lookup failed)");
      return;
    }
    System.out.println(prefix + " " + trace.getId() + " " + trace.getLayer()
        + " first=" + xy(trace.firstCorner()) + " last=" + xy(trace.lastCorner())
        + " start=" + contacts(trace.getStartContacts())
        + " end=" + contacts(trace.getEndContacts()));
    System.out.println(prefix + "_ALL " + trace.getId() + " all="
        + contacts(trace.getNormalContacts()));
  }

  static String idOf(PolylineTrace trace) {
    return trace == null ? "?" : Integer.toString(trace.getId());
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

  // ---------------------------------------------------------------------
  // the crafted trap boards
  // ---------------------------------------------------------------------

  // MAIN board (documents the parse-time normalizeAllTraces mutation —
  // the ledgered dsn-0151 divergence class, classification evidence
  // for this task): plan as below, but note that Java's read path
  // (Wiring.java:345-353 board.normalizeAllTraces()) MUTATES it —
  // the bent trace A is SPLIT at its mid corner into ids 16/17 (where
  // B's endpoint lands), the C+D+via triangle loses C to removeIfCycle
  // and D is re-inserted as id 14. The Rust reader does NOT normalize
  // (M2 Task 13), so the PARSE states differ on exactly these items.
  // The contact pins therefore run on the PURE board below.
  static final String CRAFTED_DSN =
      "(pcb t10-contacts.dsn\n"
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
      + "    (wire (path F.Cu 250  10000 10000  20000 20000  30000 10000) (net MINE))\n"
      + "    (wire (path F.Cu 250  20000 20000  20000 30000) (net MINE))\n"
      + "    (wire (path F.Cu 250  40000 40000  50000 40000) (net MINE))\n"
      + "    (wire (path F.Cu 250  60000 40000  40000 40000) (net MINE))\n"
      + "    (via PAD_C600 40000 40000 (net MINE))\n"
      + "    (wire (path F.Cu 250  20000 40000  10000 40000) (net MINE))\n"
      + "    (wire (rect F.Cu 50000 10000 60000 20000) (net MINE))\n"
      + "    (wire (path F.Cu 250  55000 15000  55000 25000) (net MINE))\n"
      + "    (wire (path F.Cu 250  50000 15000  50000 25000) (net MINE))\n"
      + "    (wire (path F.Cu 250  80000 15000  80000 30000) (net MINE))\n"
      + "  )\n"
      + ")\n";

  // PURE board — the parse state normalizeAllTraces leaves UNTOUCHED
  // (no bent trace, no trace pair sharing a via corner at read time;
  // the E/AREA/F/F2/G items are proven mutation-free by the MAIN
  // board's capture: they kept their ids and corners). The mid-corner,
  // via-corner, re-query and net-0 traps are then built by POST-PARSE
  // insertTraceWithoutCleaning insertions — which never normalize — so
  // the Java capture and the Rust port (same DSN, same generator
  // state, same insertion order) agree on every id.
  //
  // Parse items: 1=outline, 2=keepout, 3=pin P1 (net OTHER), 4=E,
  // 5=via, 6=AREA, 7=F, 8=F2, 9=G; inserted afterwards:
  //   A3 10  (30000,50000)-(35000,55000)-(40000,50000)  bent, mid
  //          corner (35000,55000)
  //   B3 11  (35000,55000)-(35000,60000)                starts on A3's
  //          MID corner
  //   C3 12  (40000,40000)-(50000,40000)
  //   D3 13  (60000,40000)-(40000,40000)                both touch the
  //          parse via (40000,40000) — the ORDER trap
  //   H  14  (50000,40000)-(45000,50000)                re-query trap
  //   ta 15  (65000,40000)-(70000,40000)  net 0
  //   tb 16  (70000,40000)-(75000,40000)  net 0
  static final String PURE_DSN =
      "(pcb t10-pure.dsn\n"
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

  // ---------------------------------------------------------------------
  // sections
  // ---------------------------------------------------------------------

  /** S/SC — the pic_programmer rows, optionally after the comp flip. */
  static void sectionS(BasicBoard board, String prefix, int count) {
    List<PolylineTrace> traces = tracesDescending(board);
    System.out.println("--" + prefix + "-- trace count=" + traces.size()
        + " ccUsed=" + board.searchTreeManager.isClearanceCompensationUsed()
        + " first " + count + " by descending id");
    for (int i = 0; i < Math.min(count, traces.size()); i++) {
      dumpTraceRows(prefix, traces.get(i));
    }
  }

  /** M — the mid-corner pair scan (first pair in descending-Y order). */
  static void sectionM(BasicBoard board) {
    System.out.println("--M-- mid-corner pair scan");
    List<PolylineTrace> traces = tracesDescending(board);
    for (PolylineTrace y : traces) {
      Point[] ys = {y.firstCorner(), y.lastCorner()};
      boolean found = false;
      for (PolylineTrace x : traces) {
        if (x == y) {
          continue;
        }
        Point[] xc = x.polyline().corners();
        if (xc.length < 3) {
          continue;
        }
        for (int i = 1; i < xc.length - 1 && !found; i++) {
          for (Point ye : ys) {
            if (xc[i].equals(ye)) {
              found = true;
              System.out.println("M_PAIR Y=" + y.getId() + " X=" + x.getId()
                  + " corner=" + xy(xc[i]));
              StringBuilder cornerStr = new StringBuilder();
              for (int c = 0; c < xc.length; c++) {
                if (c > 0) {
                  cornerStr.append(" ");
                }
                cornerStr.append(xy(xc[c]));
              }
              System.out.println("M_X_CORNERS " + x.getId() + " [" + cornerStr + "]");
              dumpTraceRows("M_Y", y);
              dumpTraceRows("M_X", x);
              // The point-form AT the mid corner: the corner guard
              // returns empty even though items sit there.
              System.out.println("M_GUARD X=" + x.getId() + " point=" + xy(xc[i])
                  + " point_form=" + contacts(x.getNormalContacts(xc[i], false)));
              break;
            }
          }
        }
        if (found) {
          break;
        }
      }
      if (found) {
        return;
      }
    }
    System.out.println("M_NONE no mid-corner endpoint pair on this fixture");
  }

  /** The shared item-dump loop (id DESCENDING). */
  static void dumpItems(BasicBoard board, String prefix) {
    List<Item> items = new ArrayList<>(board.getItems());
    items.sort((a, b) -> b.getId() - a.getId());
    for (Item item : items) {
      String geometry;
      if (item instanceof PolylineTrace trace) {
        StringBuilder sb = new StringBuilder();
        Point[] corners = trace.polyline().corners();
        for (int i = 0; i < corners.length; i++) {
          if (i > 0) {
            sb.append(" ");
          }
          sb.append(xy(corners[i]));
        }
        geometry = "corners=[" + sb + "]";
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
          + " layer=" + item.firstLayer() + " nets=" + nets(item) + " " + geometry);
    }
  }

  /** C — the MAIN board: the normalizeAllTraces mutation record. */
  static void sectionC() {
    BasicBoard board = parseDsnBytes(CRAFTED_DSN.getBytes(), "t10-contacts.dsn");
    if (board == null) {
      return;
    }
    System.out.println("--C-- MAIN crafted board AFTER parse-time "
        + "normalizeAllTraces (mutation record)");
    dumpItems(board, "C");
    // Locate the post-normalize items by corner signature.
    Map<String, PolylineTrace> bySignature = signatureMap(board);
    PolylineTrace a1 = bySignature.get("10000,10000|20000,20000|2");
    PolylineTrace a2 = bySignature.get("20000,20000|30000,10000|2");
    PolylineTrace b = bySignature.get("20000,20000|20000,30000|2");
    // C's original signature (40000,40000 -> 50000,40000): absent
    // post-normalize when removeIfCycle removed it. D's (60000,40000
    // -> 40000,40000): the re-inserted copy, expected as id 14. Both
    // INTERPOLATED so a divergence prints the real state, not the
    // recorded narrative.
    PolylineTrace cAfter = bySignature.get("40000,40000|50000,40000|2");
    PolylineTrace dAfter = bySignature.get("60000,40000|40000,40000|2");
    System.out.println("C_NORMALIZE_SPLIT A=4 -> [" + idOf(a1) + " " + idOf(a2)
        + "] (the bent trace split where B's endpoint lands); "
        + (cAfter == null
            ? "C=6 removed by removeIfCycle"
            : "C=6 still present as id " + cAfter.getId())
        + ", D=7 re-inserted as id " + idOf(dAfter)
        + " (classification: missing feature, ledgered dsn-0151 class)");
    // B now contacts BOTH halves (each has (20000,20000) as its own
    // endpoint) — descending id, and never itself.
    dumpTraceRows("C_MID_B", b);
    dumpTraceRows("C_MID_A1", a1);
    dumpTraceRows("C_MID_A2", a2);
  }

  static Map<String, PolylineTrace> signatureMap(BasicBoard board) {
    Map<String, PolylineTrace> bySignature = new HashMap<>();
    for (PolylineTrace trace : tracesDescending(board)) {
      Point[] c = trace.polyline().corners();
      bySignature.put(xy(c[0]) + "|" + xy(c[c.length - 1]) + "|" + c.length, trace);
    }
    return bySignature;
  }

  /** P — the PURE board: parse (no mutation), then the insertion traps. */
  static void sectionP() {
    BasicBoard board = parseDsnBytes(PURE_DSN.getBytes(), "t10-pure.dsn");
    if (board == null) {
      return;
    }
    System.out.println("--P-- PURE crafted board (parse state is normalize-clean)");
    dumpItems(board, "P");
    Map<String, PolylineTrace> bySignature = signatureMap(board);
    PolylineTrace traceE = bySignature.get("20000,40000|10000,40000|2");
    PolylineTrace traceF = bySignature.get("55000,15000|55000,25000|2");
    PolylineTrace traceF2 = bySignature.get("50000,15000|50000,25000|2");
    PolylineTrace traceG = bySignature.get("80000,15000|80000,30000|2");
    ConductionArea areaItem = null;
    for (Item item : board.getItems()) {
      if (item instanceof ConductionArea area) {
        areaItem = area;
      }
    }
    System.out.println("P_PARSE_IDS E=" + idOf(traceE) + " F=" + idOf(traceF)
        + " F2=" + idOf(traceF2) + " G=" + idOf(traceG)
        + " AREA=" + (areaItem == null ? -1 : areaItem.getId()));

    // --- Trap: foreign net — E's start on the foreign pin center. ---
    dumpTraceRows("P_E", traceE);
    System.out.println("P_FOREIGN E=" + traceE.getId()
        + " start_false=" + contacts(traceE.getNormalContacts(traceE.firstCorner(), false))
        + " start_true=" + contacts(traceE.getNormalContacts(traceE.firstCorner(), true)));
    System.out.println("P_FOREIGN_CANDIDATES point=" + xy(traceE.firstCorner())
        + " layer=" + traceE.getLayer() + " "
        + objects(board.overlappingObjects(
            TileShape.getInstance(traceE.firstCorner()), traceE.getLayer())));

    // --- Trap: kind filter — conduction area contains (strict + border). ---
    dumpTraceRows("P_F", traceF);
    dumpTraceRows("P_F2", traceF2);
    System.out.println("P_AREA_CONTAINS area=" + areaItem.getId()
        + " F=" + areaItem.getArea().contains(traceF.firstCorner())
        + " F2=" + areaItem.getArea().contains(traceF2.firstCorner()));

    // --- Trap: kind filter — keepout candidate, never a contact. ---
    dumpTraceRows("P_G", traceG);
    System.out.println("P_KEEP_G_TRUE G=" + traceG.getId()
        + " point_form_true=" + contacts(traceG.getNormalContacts(traceG.firstCorner(), true)));
    System.out.println("P_KEEP_CANDIDATES point=" + xy(traceG.firstCorner())
        + " layer=" + traceG.getLayer() + " "
        + objects(board.overlappingObjects(
            TileShape.getInstance(traceG.firstCorner()), traceG.getLayer())));

    // --- The insertion traps (insertTraceWithoutCleaning: no normalize,
    //     exact id parity with the Rust port's same-order inserts). ---
    PolylineTrace any = traceE;
    java.util.function.BiFunction<Point[], int[], PolylineTrace> insert =
        (corners, netsArr) -> board.insertTraceWithoutCleaning(
            new Polyline(corners), any.getLayer(), any.getHalfWidth(),
            netsArr, any.clearanceClassIndex(), FixedState.UNFIXED);

    // A3 bent + B3 at its mid corner: the MID-CORNER and GUARD traps.
    Point[] a3c = {new IntPoint(30000, 50000), new IntPoint(35000, 55000),
      new IntPoint(40000, 50000)};
    PolylineTrace a3 = insert.apply(a3c, any.netNumbers);
    Point[] b3c = {new IntPoint(35000, 55000), new IntPoint(35000, 60000)};
    PolylineTrace b3 = insert.apply(b3c, any.netNumbers);
    System.out.println("P_INSERTED A3=" + a3.getId() + " B3=" + b3.getId());
    dumpTraceRows("P_MID_B3", b3);
    dumpTraceRows("P_MID_A3", a3);
    Point midCorner = a3.polyline().corners()[1];
    System.out.println("P_GUARD A3=" + a3.getId() + " point=" + xy(midCorner)
        + " point_form=" + contacts(a3.getNormalContacts(midCorner, false)));
    System.out.println("P_MID_CANDIDATES point=" + xy(midCorner)
        + " layer=" + b3.getLayer() + " "
        + objects(board.overlappingObjects(TileShape.getInstance(midCorner), b3.getLayer())));

    // C3 + D3 sharing the parse via corner: the ORDER trap.
    Point[] c3c = {new IntPoint(40000, 40000), new IntPoint(50000, 40000)};
    PolylineTrace c3 = insert.apply(c3c, any.netNumbers);
    Point[] d3c = {new IntPoint(60000, 40000), new IntPoint(40000, 40000)};
    PolylineTrace d3 = insert.apply(d3c, any.netNumbers);
    System.out.println("P_INSERTED C3=" + c3.getId() + " D3=" + d3.getId());
    dumpTraceRows("P_ORDER_C3", c3);
    dumpTraceRows("P_ORDER_D3", d3);
    System.out.println("P_ORDER_CANDIDATES point=" + xy(c3.firstCorner())
        + " layer=" + c3.getLayer() + " "
        + objects(board.overlappingObjects(
            TileShape.getInstance(c3.firstCorner()), c3.getLayer())));

    // H at C3's free end: the COMPUTE-ON-DEMAND re-query witness.
    System.out.println("P_REQUERY_BEFORE C3=" + c3.getId()
        + " end=" + contacts(c3.getEndContacts()));
    Point[] hc = {new IntPoint(50000, 40000), new IntPoint(45000, 50000)};
    PolylineTrace h = insert.apply(hc, any.netNumbers);
    System.out.println("P_INSERTED H=" + h.getId());
    System.out.println("P_REQUERY_AFTER C3=" + c3.getId()
        + " end=" + contacts(c3.getEndContacts())
        + " (a cached port would still show [])");
    System.out.println("P_H H=" + h.getId() + " start=" + contacts(h.getStartContacts())
        + " end=" + contacts(h.getEndContacts()));
    // The no-arg union form on a trace whose start and end sets differ.
    System.out.println("P_UNION C3=" + c3.getId() + " all=" + contacts(c3.getNormalContacts()));

    // net-0 pair: sharesNetNo is plain intersection — 0 intersects 0.
    Point[] tac = {new IntPoint(65000, 40000), new IntPoint(70000, 40000)};
    PolylineTrace ta = insert.apply(tac, new int[] {0});
    Point[] tbc = {new IntPoint(70000, 40000), new IntPoint(75000, 40000)};
    PolylineTrace tb = insert.apply(tbc, new int[] {0});
    System.out.println("P_NET0 ta=" + ta.getId() + " nets=" + nets(ta)
        + " start=" + contacts(ta.getStartContacts())
        + " end=" + contacts(ta.getEndContacts()));
    System.out.println("P_NET0 tb=" + tb.getId() + " nets=" + nets(tb)
        + " start=" + contacts(tb.getStartContacts())
        + " end=" + contacts(tb.getEndContacts()));
    System.out.println("P_SHARES ta.sharesNetNo([0])=" + ta.sharesNetNo(new int[] {0})
        + " ta.sharesNetNo([])=" + ta.sharesNetNo(new int[0])
        + " ta.sharesNetNo([1])=" + ta.sharesNetNo(new int[] {1})
        + " ta.isObstacle(0)=" + ta.isObstacle(0)
        + " (net 0 contacts like a real net but never IGNORES, T8 contrast)");

    // Compensation flip: membership invariance on the same rows.
    board.searchTreeManager.setClearanceCompensationUsed(true);
    System.out.println("P_FLIPPED ccUsed="
        + board.searchTreeManager.isClearanceCompensationUsed());
    dumpTraceRows("PC1_MID_B3", b3);
    dumpTraceRows("PC1_ORDER_C3", c3);
    dumpTraceRows("PC1_F", traceF);
    System.out.println("PC1_FOREIGN_TRUE E=" + traceE.getId()
        + " start_true=" + contacts(traceE.getNormalContacts(traceE.firstCorner(), true)));
  }

  /** X — the TileShape.getInstance runtime class. */
  static void sectionX() {
    System.out.println("--X-- TileShape.getInstance(point) runtime class");
    Point p = new IntPoint(40000, 40000);
    TileShape shape = TileShape.getInstance(p);
    System.out.println("X_TILESHAPE class=" + shape.getClass().getSimpleName()
        + " (javadoc claims IntOctagon; the code returns point.surroundingBox())");
    IntBox box = (IntBox) shape;
    System.out.println("X_BOX ll=" + box.ll.x + "," + box.ll.y
        + " ur=" + box.ur.x + "," + box.ur.y);
  }

  public static void main(String[] args) throws Exception {
    String path = args.length >= 1 ? args[0] : "fixtures/Issue163-pic_programmer.dsn";
    BasicBoard board = parseDsnBytes(Files.readAllBytes(Paths.get(path)),
        Paths.get(path).getFileName().toString());
    if (board != null) {
      sectionS(board, "S", 6);
      sectionM(board);
      board.searchTreeManager.setClearanceCompensationUsed(true);
      sectionS(board, "SC", 6);
    }
    sectionC();
    sectionP();
    sectionX();
  }
}
