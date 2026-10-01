// DsnParseOracle.java — differential parse oracle for the epic-dsn port
// (M1b Task 11). Run (repo root — manifest paths are repo-relative and
// resolve against the JVM working directory):
//   ~/.jdks/jdk-25.0.4.1+1/bin/java -cp build/libs/freerouting-current-executable.jar \
//       rust/harness/oracle/DsnParseOracle.java dsn-manifest.jsonl > dsn-golden.jsonl
// Lives OUTSIDE src/ — the frozen tree is never touched; only the built jar
// is consumed.
//
// Reads one case per line: {"id":"...","path":"..."} and prints ONE result
// line per case (flushed per case, D14 — a mid-run crash preserves every
// completed line, the M1a bug-078 lesson):
//   {"id":"...","file":"<path verbatim>","result":"Success","stats":{...},
//    "geometry_sha256":"...","clearance":{...},"net_table":[...],
//    "layer_table":[...],"warnings_n":[...],"unit":"UM","resolution":10,
//    "snap_angle":"45"}
// Non-Success results (OutlineMissing/ParseError/IoError) carry only
// id/file/result. evaluateCase has TWO DISTINCT catch scopes: (1) around
// readBoard only — any Throwable maps to "ParseError", the documented
// Task-12 equivalence (plan :270; e.g. the binary Cadence fixture
// Issue006); (2) around successRecord — a Throwable in the digest/
// serialization code emits the DISTINCT marker result "DigestError"
// (id + result only) plus a stderr line, never "ParseError", so a digest
// bug fails loudly at compare time instead of lying in the committed
// golden. A malformed manifest line exits 3 with the offending id on
// stderr, or the truncated raw line when no id can be extracted from it.
//
// The digest record contract is `rust/harness/src/dsn_digest.rs` — that
// file is the DOC-OF-RECORD; the geometry-text formats below must
// byte-match `canonical_geometry_text` (T/V/K/A lines in DESCENDING item
// id order, `<fixed>` tokens, `<net-count> <nets...>` on A lines, SES
// identifier quoting on V padstack names) and `normalize_warning_digits`
// (every [0-9]+ run -> "#", D12).
import app.freerouting.board.actions.ItemIdGenerator;
import app.freerouting.board.facade.BasicBoard;
import app.freerouting.board.model.items.ConductionArea;
import app.freerouting.board.model.items.Item;
import app.freerouting.board.model.items.ObstacleArea;
import app.freerouting.board.model.items.Pin;
import app.freerouting.board.model.items.Trace;
import app.freerouting.board.model.items.Via;
import app.freerouting.board.model.structure.AngleRestriction;
import app.freerouting.board.model.structure.FixedState;
import app.freerouting.board.model.structure.Unit;
import app.freerouting.board.trace.PolylineTrace;
import app.freerouting.datastructures.IdGenerator;
import app.freerouting.geometry.planar.Circle;
import app.freerouting.geometry.planar.IntBox;
import app.freerouting.geometry.planar.IntOctagon;
import app.freerouting.geometry.planar.IntPoint;
import app.freerouting.geometry.planar.Point;
import app.freerouting.geometry.planar.PolylineArea;
import app.freerouting.geometry.planar.PolygonShape;
import app.freerouting.geometry.planar.Simplex;
import app.freerouting.io.specctra.DsnReader;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import java.io.BufferedReader;
import java.io.BufferedWriter;
import java.io.ByteArrayInputStream;
import java.io.IOException;
import java.io.InputStreamReader;
import java.io.OutputStreamWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Paths;
import java.security.MessageDigest;
import java.util.ArrayList;
import java.util.List;

public class DsnParseOracle {

  /** The SES reserved-character set (`SesWriter.java:62`). */
  static final char[] SES_RESERVED_CHARS = {
    '(', ')', ' ', ';', '-', '_', '/', '~', '{', '}'
  };

  public static void main(String[] p_args) throws Exception {
    if (p_args.length < 1) {
      System.err.println("usage: DsnParseOracle <manifest.jsonl>");
      System.exit(2);
    }
    BufferedReader in =
        new BufferedReader(
            new InputStreamReader(
                Files.newInputStream(Paths.get(p_args[0])), StandardCharsets.UTF_8));
    BufferedWriter out =
        new BufferedWriter(new OutputStreamWriter(System.out, StandardCharsets.UTF_8));
    String line;
    while ((line = in.readLine()) != null) {
      line = line.trim();
      if (line.isEmpty()) {
        continue;
      }
      String id = null;
      String path;
      try {
        JsonObject caseObj = JsonParser.parseString(line).getAsJsonObject();
        id = caseObj.get("id").getAsString();
        path = caseObj.get("path").getAsString();
      } catch (RuntimeException e) {
        out.flush();
        // M1a convention (GeometryCorpusOracle): name the offending id when
        // the line parsed far enough to yield one; a line with no
        // extractable id (e.g. unparseable JSON) falls back to the raw
        // line, truncated.
        if (id != null) {
          System.err.println("manifest format error for case id " + id + ": " + e);
        } else {
          String raw = line.length() <= 120 ? line : line.substring(0, 120) + "...";
          System.err.println("manifest format error on line '" + raw + "': " + e);
        }
        System.exit(3);
        return;
      }
      JsonObject result = evaluateCase(id, path);
      out.write(result.toString());
      out.write("\n");
      // Flush per case: FRLogger writes straight to System.out between
      // result lines; flushed lines stay atomic (M1a bug-078 lesson).
      out.flush();
    }
    out.flush();
  }

  /** One manifest case: parse, digest, serialize. Never throws. */
  static JsonObject evaluateCase(String id, String path) {
    JsonObject result = new JsonObject();
    result.addProperty("id", id);
    result.addProperty("file", path);
    byte[] bytes;
    try {
      bytes = Files.readAllBytes(Paths.get(path));
    } catch (IOException e) {
      // The Rust port reads in-memory bytes; a manifest file missing on
      // disk is a harness error, not a parse result — fail loudly.
      result.addProperty("result", "IoError");
      return result;
    }
    // Catch scope 1: readBoard ONLY. Any throwable read failure is the
    // documented "ParseError" equivalence (plan :270) — e.g.
    // fixtures/Issue006-*.dsn, the binary Cadence file. One bad file must
    // not kill the batch.
    app.freerouting.io.BoardReadResult read;
    try {
      // Null observers + a real ItemIdGenerator (the same generator
      // DsnReader itself installs for null inputs) mirror the readBoard
      // smoke path; the design name only feeds log messages.
      IdGenerator idGenerator = new ItemIdGenerator();
      read =
          DsnReader.readBoard(
              new ByteArrayInputStream(bytes),
              null,
              idGenerator,
              Paths.get(path).getFileName().toString());
    } catch (Throwable t) {
      result.addProperty("result", "ParseError");
      return result;
    }
    if (read instanceof app.freerouting.io.BoardReadResult.Success success) {
      // Catch scope 2: digest/serialization ONLY. A bug in the digest code
      // must NOT masquerade as "ParseError" (that would be a lie in the
      // committed golden, sending the port work chasing a phantom parser
      // bug); it emits the distinct "DigestError" marker (id + result
      // only) — the Rust side never emits that string, so compare flags
      // the line loudly instead of silently passing.
      try {
        return successRecord(result, success.board(), success.warnings());
      } catch (Throwable t) {
        System.err.println("digest error for case id " + id + ": " + t);
        JsonObject digestError = new JsonObject();
        digestError.addProperty("id", id);
        digestError.addProperty("result", "DigestError");
        return digestError;
      }
    }
    if (read instanceof app.freerouting.io.BoardReadResult.OutlineMissing) {
      // Plan: non-Success results carry only id/file/result.
      result.addProperty("result", "OutlineMissing");
      return result;
    }
    if (read instanceof app.freerouting.io.BoardReadResult.IoError) {
      result.addProperty("result", "IoError");
      return result;
    }
    result.addProperty("result", "ParseError");
    return result;
  }

  // -------------------------------------------------------------------------
  // The Success digest record (schema: plan :105-119, dsn_digest.rs).
  // -------------------------------------------------------------------------

  /** The `stats` object (T13: extracted from successRecord — the SAME
   * builder feeds {@code stats} and the new {@code post_stats}; the two
   * must be value-identical because the second normalizeAllTraces call
   * is a fixpoint by construction). */
  static JsonObject buildStats(BasicBoard board) {
    JsonObject stats = new JsonObject();
    stats.addProperty("layers", board.getLayerCount());
    stats.addProperty("items", board.getItems().size());
    stats.addProperty("components", board.components.count());
    int pads = 0;
    int traces = 0;
    int vias = 0;
    for (Item item : board.getItems()) {
      if (item instanceof Pin) {
        pads++;
      } else if (item instanceof Trace) {
        traces++;
      } else if (item instanceof Via) {
        vias++;
      }
    }
    stats.addProperty("pads", pads);
    stats.addProperty("nets", board.rules.nets.maxNetNumber());
    stats.addProperty("traces", traces);
    stats.addProperty("vias", vias);
    return stats;
  }

  /** Fills {@code result} (id/file already added by {@link #evaluateCase}). */
  static JsonObject successRecord(
      JsonObject result, BasicBoard board, List<String> warnings) {
    result.addProperty("result", "Success");
    result.add("stats", buildStats(board));
    result.addProperty("geometry_sha256", geometrySha256(board));
    result.add("clearance", clearanceOut(board));
    JsonArray netTable = new JsonArray();
    for (int no = 1; no <= board.rules.nets.maxNetNumber(); no++) {
      app.freerouting.rules.Net net = board.rules.nets.get(no);
      netTable.add(net == null ? "" : net.name);
    }
    result.add("net_table", netTable);
    JsonArray layerTable = new JsonArray();
    for (app.freerouting.board.model.structure.Layer layer : board.layerStructure.layers) {
      JsonObject layerOut = new JsonObject();
      layerOut.addProperty("name", layer.name);
      layerOut.addProperty("signal", layer.isSignal);
      layerTable.add(layerOut);
    }
    result.add("layer_table", layerTable);
    JsonArray warningsN = new JsonArray();
    for (String warning : warnings) {
      warningsN.add(normalizeWarningDigits(warning));
    }
    result.add("warnings_n", warningsN);
    result.addProperty("unit", board.communication.unit.name());
    result.addProperty("resolution", board.communication.resolution);
    result.addProperty("snap_angle", snapAngleOut(board.rules.getTraceAngleRestriction()));
    // T13 post-normalize fields: the read path already ended its wiring
    // scope with normalizeAllTraces (Wiring.java:343-353), so
    // stats/geometry above are ALREADY post-normalize; this explicit
    // SECOND call runs from the fixpoint (the CA6 spike pin: result
    // false) and re-digests — post_stats == stats and
    // post_geometry_sha256 == geometry_sha256 for every fixture, the
    // goldens' ENCODED idempotence claim.
    board.normalizeAllTraces();
    result.add("post_stats", buildStats(board));
    result.addProperty("post_geometry_sha256", geometrySha256(board));
    return result;
  }

  static String snapAngleOut(AngleRestriction restriction) {
    switch (restriction) {
      case NONE:
        return "none";
      case NINETY_DEGREE:
        return "90";
      default:
        return "45";
    }
  }

  static JsonObject clearanceOut(BasicBoard board) {
    JsonObject out = new JsonObject();
    int classCount = board.rules.clearanceMatrix.getClassCount();
    JsonArray classes = new JsonArray();
    for (int i = 0; i < classCount; i++) {
      classes.add(board.rules.clearanceMatrix.getName(i));
    }
    // values[i][j] = the layer-0 clearance between classes i and j with no
    // safety margin — the mirror of ClearanceIr.values[0][j][i]
    // (rust/crates/epic-dsn/src/sink.rs, `ClearanceIr`).
    JsonArray values = new JsonArray();
    for (int i = 0; i < classCount; i++) {
      JsonArray row = new JsonArray();
      for (int j = 0; j < classCount; j++) {
        row.add(board.rules.clearanceMatrix.getValue(i, j, 0, false));
      }
      values.add(row);
    }
    out.add("classes", classes);
    out.add("values", values);
    return out;
  }

  // -------------------------------------------------------------------------
  // Canonical geometry text (dsn_digest.rs DOC-OF-RECORD) + SHA-256.
  // -------------------------------------------------------------------------

  static String fixedToken(FixedState fixed) {
    switch (fixed) {
      case SHOVE_FIXED:
        return "shove_fixed";
      case USER_FIXED:
        return "user_fixed";
      case SYSTEM_FIXED:
        return "system_fixed";
      default:
        return "unfixed";
    }
  }

  static String geometrySha256(BasicBoard board) {
    MessageDigest digest;
    try {
      digest = MessageDigest.getInstance("SHA-256");
    } catch (java.security.NoSuchAlgorithmException e) {
      throw new IllegalStateException(e);
    }
    digest.update(canonicalGeometryText(board).getBytes(StandardCharsets.UTF_8));
    StringBuilder hex = new StringBuilder();
    for (byte b : digest.digest()) {
      hex.append(String.format("%02x", b));
    }
    return hex.toString();
  }

  static String coordinateOut(Point corner) {
    // Parse-produced corners are IntPoints; anything else rounds like
    // `java_round` (Math.round) exactly as the Rust encoder does.
    if (corner instanceof IntPoint intPoint) {
      return intPoint.x + " " + intPoint.y;
    }
    return Math.round(corner.toFloat().x) + " " + Math.round(corner.toFloat().y);
  }

  /** The `<shape-encoding>` of one border shape (dsn_digest.rs `encode_shape`). */
  static String encodeShape(Object shape) {
    if (shape instanceof IntBox box) {
      return "box " + box.ll.x + " " + box.ll.y + " " + box.ur.x + " " + box.ur.y;
    }
    if (shape instanceof IntOctagon octagon) {
      return "octagon "
          + octagon.leftX
          + " "
          + octagon.bottomY
          + " "
          + octagon.rightX
          + " "
          + octagon.topY
          + " "
          + octagon.upperLeftDiagonalX
          + " "
          + octagon.lowerRightDiagonalX
          + " "
          + octagon.lowerLeftDiagonalX
          + " "
          + octagon.upperRightDiagonalX;
    }
    if (shape instanceof Simplex simplex) {
      StringBuilder out = new StringBuilder("simplex ").append(simplex.cornerApproxArr().length);
      for (var corner : simplex.cornerApproxArr()) {
        out.append(' ').append(Math.round(corner.x)).append(' ').append(Math.round(corner.y));
      }
      return out.toString();
    }
    if (shape instanceof PolygonShape polygon) {
      StringBuilder out =
          new StringBuilder("polygon ").append(Integer.toString(polygon.corners.length));
      for (Point corner : polygon.corners) {
        out.append(' ').append(coordinateOut(corner));
      }
      return out.toString();
    }
    if (shape instanceof Circle circle) {
      return "circle " + circle.center.x + " " + circle.center.y + " " + circle.radius;
    }
    // Unreachable from a parse; a new Area subclass must be mirrored in
    // dsn_digest.rs `encode_shape` first. Surface it as a marker line
    // instead of crashing the batch.
    return "unencoded-" + shape.getClass().getSimpleName();
  }

  /** The border encoding plus ` win ...` per hole in file order. */
  static String encodeArea(Object area) {
    if (area instanceof PolylineArea polylineArea) {
      StringBuilder out = new StringBuilder(encodeShape(polylineArea.getBorder()));
      for (var hole : polylineArea.getHoles()) {
        out.append(" win ").append(encodeShape(hole));
      }
      return out.toString();
    }
    return encodeShape(area);
  }

  /**
   * One line per trace/via/keepout/conduction area in DESCENDING id order
   * (T39 — `Item.compareTo` is reversed, so sorting by it yields the
   * descending enumeration the board's item store iterates). Pins, the
   * board outline and component outlines consume ids but emit NO line.
   */
  static String canonicalGeometryText(BasicBoard board) {
    List<Item> items = new ArrayList<>(board.getItems());
    items.sort((first, second) -> first.compareTo(second));
    StringBuilder text = new StringBuilder();
    for (Item item : items) {
      if (item instanceof PolylineTrace trace) {
        text.append("T ")
            .append(trace.getId())
            .append(' ')
            .append(trace.getLayer())
            .append(' ')
            .append(trace.getHalfWidth())
            .append(' ')
            .append(fixedToken(trace.getFixedState()));
        var polyline = trace.polyline();
        for (int i = 0; i < polyline.cornerCount(); i++) {
          text.append(' ').append(coordinateOut(polyline.corner(i)));
        }
        text.append('\n');
      } else if (item instanceof Via via) {
        String padstack =
            via.getPadstack() == null
                ? "<unresolved>"
                : quoteJavaIdentifier(
                    via.getPadstack().name,
                    board.communication.specctraParserInfo.stringQuote);
        text.append("V ")
            .append(via.getId())
            .append(' ')
            .append(padstack)
            .append(' ')
            .append(coordinateOut(via.getCenter()))
            .append(' ')
            .append(fixedToken(via.getFixedState()))
            .append('\n');
      } else if (item instanceof ConductionArea conductionArea) {
        text.append("A ")
            .append(conductionArea.getId())
            .append(' ')
            .append(conductionArea.getLayer())
            .append(' ')
            .append(conductionArea.clearanceClassIndex())
            .append(' ')
            .append(fixedToken(conductionArea.getFixedState()))
            .append(' ')
            .append(conductionArea.netCount());
        for (int i = 0; i < conductionArea.netCount(); i++) {
          text.append(' ').append(conductionArea.getNetNumber(i));
        }
        text.append(' ').append(encodeArea(conductionArea.getArea())).append('\n');
      } else if (item instanceof ObstacleArea keepout) {
        // Component outlines extend Item directly (not ObstacleArea), so
        // every remaining ObstacleArea is a keepout kind — the K line.
        text.append("K ")
            .append(keepout.getId())
            .append(' ')
            .append(keepout.getLayer())
            .append(' ')
            .append(keepout.clearanceClassIndex())
            .append(' ')
            .append(fixedToken(keepout.getFixedState()))
            .append(' ')
            .append(encodeArea(keepout.getArea()))
            .append('\n');
      }
      // Pin / ComponentOutline / BoardOutline: line-free id consumers.
    }
    return text.toString();
  }

  // -------------------------------------------------------------------------
  // D12 + T38 helpers (mirrors of dsn_digest.rs / write_scope.rs).
  // -------------------------------------------------------------------------

  /** D12: every `[0-9]+` run becomes `#`. */
  static String normalizeWarningDigits(String warning) {
    return warning.replaceAll("[0-9]+", "#");
  }

  /**
   * The SES identifier rule (write_scope.rs `quote_java_identifier`, T38):
   * the off-by-one de-quote loop, quote-char stripping, then quote when a
   * reserved char, a signed-nonpositive UTF-8 byte, or a leading
   * (optionally `-`-prefixed) digit is present.
   */
  static String quoteJavaIdentifier(String name, String stringQuote) {
    while (name.length() > 2
        && name.charAt(0) == '"'
        && name.charAt(name.length() - 1) == '"') {
      name = name.substring(1, name.length() - 2);
    }
    if (!stringQuote.isEmpty() && name.contains(stringQuote)) {
      name = name.replace(stringQuote, "");
    }
    boolean needQuotes = false;
    for (char reserved : SES_RESERVED_CHARS) {
      if (name.indexOf(reserved) >= 0) {
        needQuotes = true;
        break;
      }
    }
    if (!needQuotes) {
      for (byte ch : name.getBytes(StandardCharsets.UTF_8)) {
        if (ch <= 0) {
          needQuotes = true;
          break;
        }
      }
    }
    if (!needQuotes && name.matches("^-?\\d.*")) {
      needQuotes = true;
    }
    return needQuotes ? stringQuote + name + stringQuote : name;
  }
}
