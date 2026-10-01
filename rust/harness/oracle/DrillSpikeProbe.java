// DrillSpikeProbe.java — the M3-T5 completion-probe oracle. Fresh JVM,
// fresh AutorouteEngine over the SAME fixture as DrillSpike (t5_drill_pages.dsn),
// NO page walk: this isolates the exact behavior of ONE drill-seam
// completion (engine.completeExpansionRoom(new IncompleteFreeSpaceExpansionRoom(
// null, layer, pointShape))) against an items-only shared tree — the
// state the DrillSpike capture's FIRST completions per layer see.
// The main spike cannot host these probes: the autoroute search tree is
// SHARED via board.searchTreeManager (keyed by clearance class), so a
// probe completion before the page walk would shift every room id.
//
// Build/run (JDK 25, from the repo root):
//   mkdir -p /tmp/epic-drill-classes && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/javac \
//       -cp build/libs/freerouting-current-executable.jar \
//       -d /tmp/epic-drill-classes rust/harness/oracle/DrillSpikeProbe.java && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/java \
//       -Duser.language=en -Duser.country=US \
//       -cp build/libs/freerouting-current-executable.jar:/tmp/epic-drill-classes \
//       app.freerouting.autoroute.maze.DrillSpikeProbe \
//       rust/harness/fixtures/drill-spike/t5_drill_pages.dsn
package app.freerouting.autoroute.maze;

import app.freerouting.autoroute.expansion.CompleteFreeSpaceExpansionRoom;
import app.freerouting.autoroute.expansion.IncompleteFreeSpaceExpansionRoom;
import app.freerouting.board.actions.ItemIdGenerator;
import app.freerouting.board.facade.BasicBoard;
import app.freerouting.board.facade.RoutingBoard;
import app.freerouting.geometry.planar.IntBox;
import app.freerouting.geometry.planar.IntPoint;
import app.freerouting.geometry.planar.TileShape;
import app.freerouting.io.BoardReadResult;
import app.freerouting.io.specctra.DsnReader;
import com.google.gson.Gson;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import java.io.ByteArrayInputStream;
import java.lang.reflect.Field;
import java.nio.file.Files;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.Collection;

public class DrillSpikeProbe {

  private static final Gson GSON = new Gson();

  private static void row(JsonObject o) {
    System.out.println(GSON.toJson(o));
  }

  private static JsonObject obj(String kind) {
    JsonObject o = new JsonObject();
    o.addProperty("type", kind);
    return o;
  }

  private static void intArray(JsonObject o, String name, int[] values) {
    JsonArray arr = new JsonArray(values.length);
    for (int v : values) {
      arr.add(v);
    }
    o.add(name, arr);
  }

  private static Field field(Class<?> c, String name) throws Exception {
    Field f = c.getDeclaredField(name);
    f.setAccessible(true);
    return f;
  }

  private static int intField(Object owner, String name) throws Exception {
    return (Integer) field(owner.getClass(), name).get(owner);
  }

  public static void main(String[] p_args) throws Exception {
    if (p_args.length < 1) {
      row(obj("usage-error"));
      System.exit(1);
    }
    byte[] bytes = Files.readAllBytes(Paths.get(p_args[0]));
    BoardReadResult read =
        DsnReader.readBoard(
            new ByteArrayInputStream(bytes),
            null,
            new ItemIdGenerator(),
            Paths.get(p_args[0]).getFileName().toString());
    if (!(read instanceof BoardReadResult.Success success)) {
      row(obj("read-not-success"));
      System.exit(3);
      return;
    }
    BasicBoard basicBoard = success.board();
    RoutingBoard board = (RoutingBoard) basicBoard;
    board.searchTreeManager.reinsertTreeItems();

    AutorouteEngine engine = new AutorouteEngine(board, 1, true);
    engine.initConnection(1, null, null);
    field(AutorouteEngine.class, "incompleteExpansionRooms").set(engine, new ArrayList<>());

    // One completion probe per layer at an empty-region point (all in
    // DB units). Each probe is the EXACT drill-seam call:
    // calculateExpansionRooms -> completeExpansionRoom(new
    // IncompleteFreeSpaceExpansionRoom(null, layer, pointShape)).
    int[][] probes = {
      {10000, 10000},
      {500000, 10000},
      {10000, 500000},
    };
    for (int[] probe : probes) {
      for (int layer = 0; layer < 4; layer++) {
        TileShape pointShape = TileShape.getInstance(new IntPoint(probe[0], probe[1]));
        IncompleteFreeSpaceExpansionRoom probeRoom =
            new IncompleteFreeSpaceExpansionRoom(null, layer, pointShape);
        int idBefore = intField(engine, "expansionRoomInstanceCount");
        Collection<CompleteFreeSpaceExpansionRoom> res =
            engine.completeExpansionRoom(probeRoom);
        int idAfter = intField(engine, "expansionRoomInstanceCount");
        @SuppressWarnings("unchecked")
        Collection<IncompleteFreeSpaceExpansionRoom> incompletes =
            (Collection<IncompleteFreeSpaceExpansionRoom>)
                field(AutorouteEngine.class, "incompleteExpansionRooms").get(engine);
        JsonObject o = obj("completionProbe");
        o.addProperty("x", probe[0]);
        o.addProperty("y", probe[1]);
        o.addProperty("layer", layer);
        o.addProperty("count", res.size());
        o.addProperty("burned", idAfter - idBefore);
        o.addProperty("incompleteCount", incompletes.size());
        JsonArray rooms = new JsonArray();
        for (CompleteFreeSpaceExpansionRoom r : res) {
          JsonObject rr = new JsonObject();
          rr.addProperty("id", r.getId());
          rr.addProperty("layer", r.getLayer());
          IntBox b = r.getShape().boundingBox();
          intArray(rr, "box", new int[] {b.ll.x, b.ll.y, b.ur.x, b.ur.y});
          rooms.add(rr);
        }
        o.add("rooms", rooms);
        row(o);
      }
    }
    row(obj("done"));
  }
}
