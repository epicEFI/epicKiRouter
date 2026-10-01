// SesEmitOracle.java — differential SES-emission oracle for the epic-dsn
// SesWriter port (M1b Task 13, Part B3). Run (repo root — manifest paths
// are repo-relative and resolve against the JVM working directory):
//   ~/.jdks/jdk-25.0.4.1+1/bin/java -cp build/libs/freerouting-current-executable.jar \
//       rust/harness/oracle/SesEmitOracle.java ses-manifest.jsonl <golden-out-dir>
// Lives OUTSIDE src/ — the frozen tree is never touched; only the built jar
// is consumed.
//
// Reads one case per line (written by `epic-harness dsn ses-golden`):
//   {"id":"ses-0001","path":"scripts/benchmark/fixtures/.../x.dsn",
//    "design":"x.dsn","golden":"Dir__x.ses.golden"}
// and for every case prints ONE result line (flushed per case, D14 — the
// golden FILE is written before its result line, so a mid-run crash leaves
// every completed artifact on disk):
//   {"id":"...","file":"<path verbatim>","result":"Success",
//    "golden":"Dir__x.ses.golden","bytes":1234,"sha256":"<hex>"}
// Non-Success results (OutlineMissing/ParseError/IoError) carry only
// id/file/result and write NO golden file — the Rust side owns the
// committed corpus directory and bails loudly if a case stops producing
// one.
//
// designName semantics: the real CLI threads the design FILE NAME into
// SesWriter.write (HeadlessBoardManager.saveAsSpecctraSessionSes callers;
// real-world proof fixtures/Issue313-FastTest.ses opens with
// `(session Issue313-FastTest.ses` / `(base_design Issue313-FastTest.dsn`),
// NOT the full -de path and NOT the board's (pcb ...) name. The manifest
// therefore carries the file name explicitly — both sides must derive the
// session header from the same string.
//
// evaluateCase has THREE distinct catch scopes (the DsnParseOracle
// discipline, extended one step): (1) around readBoard only — any
// Throwable maps to "ParseError" (the documented Task-12 equivalence);
// (2) around SesWriter.write + the golden file write — a Throwable emits
// the DISTINCT marker result "EmitError" (id + result only) plus a stderr
// line, never "ParseError", so an emission bug in EITHER engine fails
// loudly at compare time instead of masquerading as a parse failure in
// the committed corpus.
import app.freerouting.board.actions.ItemIdGenerator;
import app.freerouting.board.facade.BasicBoard;
import app.freerouting.datastructures.IdGenerator;
import app.freerouting.io.specctra.DsnReader;
import app.freerouting.io.specctra.SesWriter;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import java.io.BufferedReader;
import java.io.BufferedWriter;
import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStreamReader;
import java.io.OutputStreamWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.security.MessageDigest;

public class SesEmitOracle {

  public static void main(String[] p_args) throws Exception {
    if (p_args.length < 2) {
      System.err.println("usage: SesEmitOracle <manifest.jsonl> <golden-out-dir>");
      System.exit(2);
    }
    Path outDir = Paths.get(p_args[1]);
    Files.createDirectories(outDir);
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
        // M1a convention (GeometryCorpusOracle/DsnParseOracle): name the
        // offending id when the line parsed far enough to yield one.
        if (id != null) {
          System.err.println("manifest format error for case id " + id + ": " + e);
        } else {
          String raw = line.length() <= 120 ? line : line.substring(0, 120) + "...";
          System.err.println("manifest format error on line '" + raw + "': " + e);
        }
        System.exit(3);
        return;
      }
      JsonObject result = evaluateCase(id, path, design, golden, outDir);
      out.write(result.toString());
      out.write("\n");
      // Flush per case: FRLogger writes straight to System.out between
      // result lines; flushed lines stay atomic (M1a bug-078 lesson).
      out.flush();
    }
    out.flush();
  }

  /** One manifest case: parse, emit, write golden. Never throws. */
  static JsonObject evaluateCase(String id, String path, String design, String golden, Path outDir) {
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
    // Catch scope 1: readBoard ONLY (the documented ParseError
    // equivalence, plan :270). One bad file must not kill the batch.
    app.freerouting.io.BoardReadResult read;
    try {
      // Null observers + a real ItemIdGenerator mirror the readBoard
      // smoke path exactly as DsnParseOracle drives it; the design name
      // only feeds log messages.
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
      // Catch scope 2: SesWriter.write + the golden file write. An
      // emission failure must NOT masquerade as "ParseError" (a lie in
      // the committed corpus); the distinct "EmitError" marker (id +
      // result only) fails loudly at compare time — the Rust port never
      // emits that string.
      try {
        return emitRecord(result, success.board(), design, golden, outDir);
      } catch (Throwable t) {
        System.err.println("emit error for case id " + id + ": " + t);
        JsonObject emitError = new JsonObject();
        emitError.addProperty("id", id);
        emitError.addProperty("result", "EmitError");
        return emitError;
      }
    }
    if (read instanceof app.freerouting.io.BoardReadResult.OutlineMissing) {
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

  /** The Success emission record: session bytes → golden file + digest. */
  static JsonObject emitRecord(
      JsonObject result, BasicBoard board, String design, String golden, Path outDir)
      throws IOException {
    ByteArrayOutputStream session = new ByteArrayOutputStream();
    SesWriter.write(board, session, design);
    byte[] sessionBytes = session.toByteArray();
    // Raw bytes, no newline munging — the golden is byte-identical to
    // what `SesWriter.write` flushed (UTF-8 per IndentFileWriter).
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
    return result;
  }
}
