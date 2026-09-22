// IdOrderProbe.java — M4-T1 (buglog 172) parse-time item-id ORDER dump.
// Prints one line per board-item INSERT, in insert order, with the item id,
// simple class name, and bounding box, plus GEN_MAX at the end. The purpose
// is the id-assignment ORDER oracle: Java burns ids at Item construction
// (Item.java:86-89) inside BasicBoard.insert* in DsnReader read order; the
// observer (BoardItemRepository.insertItem -> notifyNew, unguarded by the
// active flag) fires on every real insert — items dropped by the insert
// guards never appear here, matching the Rust insert-site semantics.
//
// Output contract: the FIRST stdout line is the FORMAT header below. The
// Rust pin `parse_world_board_matches_the_jar_post_parse_id_sequence`
// (harness/src/route_events.rs) cross-checks the declared constant against
// the format its row literals were captured under — change the output
// shape ONLY together with a pin re-capture (bump the constant AND the
// literals; the pin dies loudly on any drift, so keep the declaration on
// one line: the pin extracts the single-line `FORMAT = "` declaration).
//
// Run (the house javac + FQCN pattern, JDK 25, from the repo root; do NOT
// change the oracle build wiring in harness/src/oracle.rs):
//   mkdir -p /tmp/epic-idorder-classes && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/javac \
//       -cp build/libs/freerouting-current-executable.jar \
//       -d /tmp/epic-idorder-classes rust/harness/oracle/IdOrderProbe.java && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/java \
//       -cp build/libs/freerouting-current-executable.jar:/tmp/epic-idorder-classes \
//       app.freerouting.board.actions.IdOrderProbe <file.dsn>
package app.freerouting.board.actions;

import app.freerouting.board.model.items.Item;
import app.freerouting.board.model.structure.Component;
import app.freerouting.board.state.BoardObservers;
import app.freerouting.datastructures.IdGenerator;
import app.freerouting.geometry.planar.IntBox;
import app.freerouting.io.BoardReadResult;
import app.freerouting.io.specctra.DsnReader;
import java.io.FileInputStream;
import java.io.InputStream;
import java.util.ArrayList;
import java.util.List;

public class IdOrderProbe {
  static final String FORMAT = "id-order-probe/2 (insert-order rows; DEL rows; descending-id POST walk; GEN_MAX)";

  public static void main(String[] args) throws Exception {
    System.out.println("FORMAT " + FORMAT);
    List<String> rows = new ArrayList<>();
    BoardObservers observers =
        new BoardObservers() {
          @Override
          public void notifyDeleted(Item item) {
            rows.add("DEL id=" + item.getId() + " " + item.getClass().getSimpleName());
          }

          @Override
          public void notifyChanged(Item item) {}

          @Override
          public void notifyNew(Item item) {
            IntBox b = item.boundingBox();
            String corners = "";
            if (item instanceof app.freerouting.board.trace.PolylineTrace trace) {
              StringBuilder sb = new StringBuilder(" corners=");
              for (app.freerouting.geometry.planar.Point p : trace.polyline().corners()) {
                app.freerouting.geometry.planar.IntPoint ip =
                    (app.freerouting.geometry.planar.IntPoint) p;
                sb.append("(").append(ip.x).append(",").append(ip.y).append(")");
              }
              corners = sb.toString();
            }
            rows.add(
                "id="
                    + item.getId()
                    + " "
                    + item.getClass().getSimpleName()
                    + " bounds=["
                    + b.ll.x
                    + ","
                    + b.ll.y
                    + "]..["
                    + b.ur.x
                    + ","
                    + b.ur.y
                    + "]"
                    + corners);
          }

          @Override
          public void notifyMoved(Component component) {}

          @Override
          public void activate() {}

          @Override
          public void deactivate() {}

          @Override
          public boolean isActive() {
            return true;
          }
        };
    IdGenerator idGenerator = new ItemIdGenerator();
    Object boardObj = null;
    try (InputStream in = new FileInputStream(args[0])) {
      BoardReadResult result = DsnReader.readBoard(in, observers, idGenerator);
      System.out.println("result=" + result.getClass().getSimpleName());
      if (result instanceof BoardReadResult.Success success) {
        boardObj = success.board();
      }
    }
    for (String row : rows) {
      System.out.println(row);
    }
    // The POST-parse board walk (after the in-read normalizeAllTraces):
    // the definitive id sequence the live board carries into routing.
    if (boardObj instanceof app.freerouting.board.facade.BasicBoard board) {
      System.out.println("-- post-parse board (descending id):");
      var it = board.itemList.startReadObject();
      for (; ; ) {
        Object obj = board.itemList.readObject(it);
        if (!(obj instanceof Item item)) {
          break;
        }
        IntBox b = item.boundingBox();
        String corners = "";
        if (item instanceof app.freerouting.board.trace.PolylineTrace trace) {
          StringBuilder sb = new StringBuilder(" corners=");
          for (app.freerouting.geometry.planar.Point p : trace.polyline().corners()) {
            app.freerouting.geometry.planar.IntPoint ip =
                (app.freerouting.geometry.planar.IntPoint) p;
            sb.append("(").append(ip.x).append(",").append(ip.y).append(")");
          }
          corners = sb.toString();
        }
        System.out.println(
            "POST id="
                + item.getId()
                + " "
                + item.getClass().getSimpleName()
                + " bounds=["
                + b.ll.x
                + ","
                + b.ll.y
                + "]..["
                + b.ur.x
                + ","
                + b.ur.y
                + "]"
                + corners);
      }
    }
    System.out.println("GEN_MAX " + idGenerator.maxGeneratedId());
    System.out.println("rows=" + rows.size());
  }
}
