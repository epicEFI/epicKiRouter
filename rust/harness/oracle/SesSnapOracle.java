// SesSnapOracle.java — differential SES endpoint-SNAP oracle for the T40
// snappedEndpoint port (M2 Task 15, decisions D23/D24). Run (repo root —
// manifest paths are repo-relative and resolve against the JVM working
// directory):
//   ~/.jdks/jdk-25.0.4.1+1/bin/java -cp build/libs/freerouting-current-executable.jar \
//       rust/harness/oracle/SesSnapOracle.java <mode> snap-manifest.jsonl <out-dir>
//   mode = scan    : parse + per-endpoint snap diagnostics only (no files)
//   mode = capture : same + the session bytes as goldens (sha256 + length)
// Lives OUTSIDE src/ — the frozen tree is never touched; only the built jar
// is consumed.
//
// EXACT CALL SEQUENCE per fixture (the plan's open spike item, resolved):
//   DsnReader.readBoard(bytes, null, new ItemIdGenerator(), fileName)
//     -> BasicBoard        (the reader's own wiring scope ALREADY ran
//                            normalizeAllTraces — Wiring.java:343-353 — so
//                            the board handed to SesWriter is post-normalize;
//                            there is NO extra call between read and write)
//     -> per-trace diagnostics (getStartContacts/getEndContacts + the
//        reflection-called real snappedEndpoint)
//     -> SesWriter.write(board, out, design)
// The Rust replay mirrors it: read_board -> SesBoard ->
// Board::from_ses_board -> insert_items_creation_order ->
// normalize_all_traces -> contacts -> write_session_with_contacts.
//
// THE REACHABILITY FINDING (pinned by this oracle's counters):
//   SesWriter.snappedEndpoint (:426-453) iterates the wire's CONTACT set.
//   Trace.getNormalContacts (Trace.java:173-203) accepts a DrillItem only
//   when `point.equals(drillItem.getCenter())` — and Point.equals is
//   CLASS-STRICT (IntPoint.java:33-45, RationalPoint.java:73-86: same
//   getClass() + same value). So every drill contact sits at distance
//   EXACTLY 0.0 from the corner, the `centerDistance <= 0.5` early-out
//   (:444-447) fires, and snappedEndpoint returns null. The inradius branch
//   (:448-450) is UNREACHABLE through this contacts path on ANY board —
//   parse-time or routed. Empirical witness (jar, this oracle's ancestor
//   spike /tmp/epic-t15-spike1/SnapReachSpike.java): Issue593-BBD_Mars-64
//   DSN + companion SES import — 215 traces, 209 drill-contacted
//   endpoints, snapFired=0, every dist printed 0.000000. The corpus
//   therefore pins the rule's INPUT surface (drill-contacted endpoints,
//   ruleReach > 0 required per fixture) and byte parity of the wired
//   writer; the output-changing branch is pinned by crafted unit vectors
//   in Rust (epic-dsn ses::writer tests), not by real boards.
//
// Per-case result line (flushed per case, D14):
//   {"id":"...","file":"...","result":"Success","golden":"...",
//    "bytes":1234,"sha256":"<hex>","traces":12,"ruleReach":8,
//    "earlyOuts":8,"snapFired":0,"contactMismatches":0,
//    "drillContacts":{"pin":6,"via":2},"multiDrillEndpoints":1}
// Diagnostic rows on stdout between result lines:
//   SNAPLOG <case> trace=<id> side=start|end contacts=[id:Kind,...]
//   SNAPLOG <case> trace=<id> side=.. drill=<id> kind=Pin|Via dist=%.6f
//        inradius=%.6f shape=<SimpleClassName>|span-skip|shape-null
//   SNAPLOG <case> trace=<id> side=.. verdict=early-out|snap|no-qualifier
//        real=<null|x,y>   (real = the reflection-called jar result.
//        contactMismatches is a TRIPWIRE, not an equality proof:
//        FloatPoint has no equals override, so real.equals(reimplemented)
//        compares references — vacuously green while both sides return
//        null (the snapFired=0 corpus-wide reality), loud the moment
//        either the jar or the reimplementation returns non-null)
// Definitions mirror the Rust provider exactly: ruleReach counts endpoints
// whose drill list survives the instanceof + layer-span + null-shape
// filters; earlyOuts counts rule-reach endpoints that returned via the
// <=0.5 arm; snapFired counts non-null results (pinned expectation: 0);
// drillContacts{pin,via} counts EXAMINED drill rows by kind (the <=0.5
// break means a multi-drill endpoint logs only its FIRST row — e.g.
// BatCharge trace=107 contacts=[153:Via 13:Pin] logs just the via row);
// multiDrillEndpoints counts endpoints whose RAW contact set holds >=2
// DrillItems (the coincident via+pin shape; drillContacts rows therefore
// sum to ruleReach, NOT to the pre-quirk drill count).
import app.freerouting.board.actions.ItemIdGenerator;
import app.freerouting.board.facade.BasicBoard;
import app.freerouting.board.model.items.DrillItem;
import app.freerouting.board.model.items.Item;
import app.freerouting.board.model.items.Pin;
import app.freerouting.board.model.items.Trace;
import app.freerouting.board.model.items.Via;
import app.freerouting.board.trace.PolylineTrace;
import app.freerouting.datastructures.IdGenerator;
import app.freerouting.geometry.planar.FloatPoint;
import app.freerouting.geometry.planar.Point;
import app.freerouting.geometry.planar.Shape;
import app.freerouting.io.BoardReadResult;
import app.freerouting.io.specctra.DsnReader;
import app.freerouting.io.specctra.SesWriter;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import java.io.BufferedReader;
import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStreamReader;
import java.io.OutputStreamWriter;
import java.io.BufferedWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.security.MessageDigest;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import java.util.Set;

public class SesSnapOracle {

  public static void main(String[] p_args) throws Exception {
    if (p_args.length < 3) {
      System.err.println(
          "usage: SesSnapOracle scan|capture <manifest.jsonl> <out-dir>");
      System.exit(2);
    }
    boolean capture = p_args[0].equals("capture");
    Path outDir = Paths.get(p_args[2]);
    Files.createDirectories(outDir);
    BufferedReader in =
        new BufferedReader(
            new InputStreamReader(
                Files.newInputStream(Paths.get(p_args[1])), StandardCharsets.UTF_8));
    BufferedWriter out =
        new BufferedWriter(new OutputStreamWriter(System.out, StandardCharsets.UTF_8));
    java.lang.reflect.Method snap = null;
    String line;
    while ((line = in.readLine()) != null) {
      line = line.trim();
      if (line.isEmpty()) {
        continue;
      }
      String id = null;
      String path;
      String design;
      String golden;
      try {
        JsonObject caseObj = JsonParser.parseString(line).getAsJsonObject();
        id = caseObj.get("id").getAsString();
        path = caseObj.get("path").getAsString();
        design = caseObj.get("design").getAsString();
        golden = caseObj.get("golden").getAsString();
      } catch (RuntimeException e) {
        out.flush();
        if (id != null) {
          System.err.println("manifest format error for case id " + id + ": " + e);
        } else {
          String raw = line.length() <= 120 ? line : line.substring(0, 120) + "...";
          System.err.println("manifest format error on line '" + raw + "': " + e);
        }
        System.exit(3);
        return;
      }
      // Resolved once: the package-private static rule, via reflection
      // (the established harness pattern for package-private members).
      if (snap == null) {
        snap =
            Class.forName("app.freerouting.io.specctra.SesWriter")
                .getDeclaredMethod("snappedEndpoint", PolylineTrace.class, boolean.class);
        snap.setAccessible(true);
      }
      JsonObject result = evaluateCase(id, path, design, golden, outDir, capture, snap);
      out.write(result.toString());
      out.write("\n");
      out.flush();
    }
    out.flush();
  }

  /** One manifest case. Never throws. */
  static JsonObject evaluateCase(
      String id,
      String path,
      String design,
      String golden,
      Path outDir,
      boolean capture,
      java.lang.reflect.Method snap) {
    JsonObject result = new JsonObject();
    result.addProperty("id", id);
    result.addProperty("file", path);
    byte[] bytes;
    try {
      bytes = Files.readAllBytes(Paths.get(path));
    } catch (IOException e) {
      result.addProperty("result", "IoError");
      return result;
    }
    BoardReadResult read;
    try {
      IdGenerator idGenerator = new ItemIdGenerator();
      read =
          DsnReader.readBoard(
              new ByteArrayInputStream(bytes), null, idGenerator, Paths.get(path).getFileName()
                  .toString());
    } catch (Throwable t) {
      result.addProperty("result", "ParseError");
      return result;
    }
    if (!(read instanceof BoardReadResult.Success success)) {
      result.addProperty(
          "result",
          read instanceof BoardReadResult.OutlineMissing
              ? "OutlineMissing"
              : read instanceof BoardReadResult.IoError ? "IoError" : "ParseError");
      return result;
    }
    try {
      BasicBoard board = success.board();
      Counters counters = diagnose(id, board, snap);
      if (capture) {
        ByteArrayOutputStream session = new ByteArrayOutputStream();
        SesWriter.write(board, session, design);
        byte[] sessionBytes = session.toByteArray();
        Files.write(outDir.resolve(golden), sessionBytes);
        result.addProperty("result", "Success");
        result.addProperty("golden", golden);
        result.addProperty("bytes", sessionBytes.length);
        StringBuilder hex = new StringBuilder();
        MessageDigest digest;
        try {
          digest = MessageDigest.getInstance("SHA-256");
        } catch (java.security.NoSuchAlgorithmException e) {
          throw new IllegalStateException(e);
        }
        for (byte b : digest.digest(sessionBytes)) {
          hex.append(String.format("%02x", b));
        }
        result.addProperty("sha256", hex.toString());
        result.addProperty("traces", counters.traces);
        result.addProperty("ruleReach", counters.ruleReach);
        result.addProperty("earlyOuts", counters.earlyOuts);
        result.addProperty("snapFired", counters.snapFired);
        result.addProperty("contactMismatches", counters.mismatches);
        addDrillContacts(result, counters);
        result.addProperty("multiDrillEndpoints", counters.multiDrillEndpoints);
        return result;
      }
      result.addProperty("result", "Success");
      result.addProperty("traces", counters.traces);
      result.addProperty("ruleReach", counters.ruleReach);
      result.addProperty("earlyOuts", counters.earlyOuts);
      result.addProperty("snapFired", counters.snapFired);
      result.addProperty("contactMismatches", counters.mismatches);
      addDrillContacts(result, counters);
      result.addProperty("multiDrillEndpoints", counters.multiDrillEndpoints);
      return result;
    } catch (Throwable t) {
      System.err.println("snap-oracle error for case id " + id + ": " + t);
      JsonObject error = new JsonObject();
      error.addProperty("id", id);
      error.addProperty("result", "EmitError");
      return error;
    }
  }

  /** The per-endpoint diagnostics + counters (module docs). */
  static Counters diagnose(String id, BasicBoard board, java.lang.reflect.Method snap)
      throws Exception {
    Counters counters = new Counters();
    List<PolylineTrace> traces = new ArrayList<>();
    for (Item item : board.getItems()) {
      if (item instanceof PolylineTrace trace) {
        traces.add(trace);
      }
    }
    // DESCENDING id — the writer's own walk order (writeNet iterates the
    // item list by Item.compareTo = other.id - id), so the log aligns with
    // the golden's wire order.
    traces.sort(Comparator.comparingInt(Item::getId).reversed());
    counters.traces = traces.size();
    for (PolylineTrace trace : traces) {
      for (boolean startSide : new boolean[] {true, false}) {
        Set<Item> contacts = startSide ? trace.getStartContacts() : trace.getEndContacts();
        StringBuilder contactIds = new StringBuilder();
        for (Item contact : contacts) {
          if (contactIds.length() > 0) {
            contactIds.append(' ');
          }
          contactIds
              .append(contact.getId())
              .append(':')
              .append(contact instanceof Pin ? "Pin"
                  : contact instanceof Via ? "Via"
                  : contact instanceof Trace ? "Trace"
                      : contact.getClass().getSimpleName());
        }
        System.out.printf(
            "SNAPLOG %s trace=%d side=%s contacts=[%s]%n",
            id, trace.getId(), startSide ? "start" : "end", contactIds);
        // The reimplementation, mirroring snappedEndpoint :426-453 exactly
        // (including the FUNCTION-LEVEL return of the <=0.5 arm).
        FloatPoint corner =
            (startSide ? trace.firstCorner() : trace.lastCorner()).toFloat();
        int layer = trace.getLayer();
        boolean reached = false;
        String verdict = "no-qualifier";
        FloatPoint reimplemented = null;
        int examined = 0;
        // multiDrillEndpoints PRE-PASS: counting drills inside the walk
        // loop would undercount — the walk BREAKS at the first <=0.5
        // drill (the function-level quirk), never visiting the second
        // drill of a coincident via+pin endpoint.
        int drillContactsRaw = 0;
        for (Item contact : contacts) {
          if (contact instanceof DrillItem) {
            drillContactsRaw++;
          }
        }
        if (drillContactsRaw >= 2) {
          counters.multiDrillEndpoints++;
        }
        for (Item contact : contacts) {
          if (!(contact instanceof DrillItem drill)) {
            continue;
          }
          if (layer < drill.firstLayer() || layer > drill.lastLayer()) {
            System.out.printf(
                "SNAPLOG %s trace=%d side=%s drill=%d kind=%s span-skip%n",
                id, trace.getId(), startSide ? "start" : "end", contact.getId(),
                drill instanceof Pin ? "Pin" : drill instanceof Via ? "Via" : "Drill");
            continue;
          }
          Shape padShape = drill.getShape(layer - drill.firstLayer());
          if (padShape == null) {
            System.out.printf(
                "SNAPLOG %s trace=%d side=%s drill=%d kind=%s shape-null%n",
                id, trace.getId(), startSide ? "start" : "end", contact.getId(),
                drill instanceof Pin ? "Pin" : drill instanceof Via ? "Via" : "Drill");
            continue;
          }
          reached = true;
          examined++;
          if (drill instanceof Pin) {
            counters.pinRows++;
          } else if (drill instanceof Via) {
            counters.viaRows++;
          }
          FloatPoint center = drill.getCenter().toFloat();
          double centerDistance = corner.distance(center);
          double borderDistance = padShape.borderDistance(center);
          System.out.printf(
              "SNAPLOG %s trace=%d side=%s drill=%d kind=%s dist=%.6f inradius=%.6f shape=%s%n",
              id, trace.getId(), startSide ? "start" : "end", contact.getId(),
              drill instanceof Pin ? "Pin" : drill instanceof Via ? "Via" : "Drill",
              centerDistance, borderDistance, padShape.getClass().getSimpleName());
          if (centerDistance <= 0.5) {
            verdict = "early-out";
            break; // Java :446 returns null for the WHOLE function
          }
          if (centerDistance <= borderDistance) {
            verdict = "snap";
            reimplemented = center;
            break;
          }
        }
        if (reached) {
          counters.ruleReach++;
        }
        if ("early-out".equals(verdict)) {
          counters.earlyOuts++;
        }
        if ("snap".equals(verdict)) {
          counters.snapFired++;
        }
        FloatPoint real = (FloatPoint) snap.invoke(null, trace, startSide);
        boolean agree =
            (real == null && reimplemented == null)
                || (real != null && reimplemented != null && real.equals(reimplemented));
        if (!agree) {
          counters.mismatches++;
        }
        if (examined > 0 || !agree) {
          System.out.printf(
              "SNAPLOG %s trace=%d side=%s verdict=%s real=%s%n",
              id,
              trace.getId(),
              startSide ? "start" : "end",
              verdict,
              real == null ? "null" : real.x + "," + real.y);
        }
      }
    }
    return counters;
  }

  static final class Counters {
    int traces;
    int ruleReach;
    int earlyOuts;
    int snapFired;
    int mismatches;
    int pinRows;
    int viaRows;
    int multiDrillEndpoints;
  }

  /** The drillContacts result object (kind split of EXAMINED rows). */
  static void addDrillContacts(JsonObject result, Counters counters) {
    JsonObject drillContacts = new JsonObject();
    drillContacts.addProperty("pin", counters.pinRows);
    drillContacts.addProperty("via", counters.viaRows);
    result.add("drillContacts", drillContacts);
  }
}
