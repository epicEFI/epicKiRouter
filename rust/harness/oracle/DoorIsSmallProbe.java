// T9 debug probe (harness-side, NOT part of the capture protocol):
// runs ONE maze-search phase and dumps the per-door doorIsSmall input
// table — for every ExpansionDoor in the search world (transitive BFS
// from the tree rooms through door endpoints): dimension, endpoint room
// ids/kinds, the THREE arm values (box maxWidth, octagon maxWidth,
// diagonal length), the guard verdict, and the door shape corners.
// Usage: DoorIsSmallProbe <fixture.dsn> <NET_X> <NINETY|FORTYFIVE|NONE>
// Diff protocol: match rows against instrumented Rust door_is_small
// calls by (a, b, dim); a per-door arm-value mismatch localizes the
// culprit; equal values with unequal verdicts localize the guard/width.
package app.freerouting.autoroute.maze;

import app.freerouting.autoroute.maze.AutorouteControl;
import app.freerouting.autoroute.maze.AutorouteEngine;
import app.freerouting.autoroute.maze.MazeSearchEngine;
import app.freerouting.board.facade.BasicBoard;
import app.freerouting.board.facade.RoutingBoard;
import app.freerouting.board.model.items.Item;
import app.freerouting.board.model.items.Pin;
import app.freerouting.board.model.structure.AngleRestriction;
import app.freerouting.board.actions.ItemIdGenerator;
import app.freerouting.board.searchtree.SearchTreeObject;
import app.freerouting.autoroute.expansion.CompleteExpansionRoom;
import app.freerouting.autoroute.expansion.CompleteFreeSpaceExpansionRoom;
import app.freerouting.autoroute.expansion.ExpansionDoor;
import app.freerouting.autoroute.expansion.ExpansionRoom;
import app.freerouting.geometry.planar.FloatLine;
import app.freerouting.geometry.planar.IntBox;
import app.freerouting.geometry.planar.IntPoint;
import app.freerouting.geometry.planar.TileShape;
import app.freerouting.io.BoardReadResult;
import app.freerouting.io.specctra.DsnReader;
import app.freerouting.settings.RouterSettings;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import java.io.ByteArrayInputStream;
import java.lang.reflect.Field;
import java.nio.file.Files;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.IdentityHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;

public class DoorIsSmallProbe {

  private static final com.google.gson.Gson GSON = new com.google.gson.Gson();
  private static Field INCOMPLETE_FIELD;

  static {
    try {
      INCOMPLETE_FIELD =
          AutorouteEngine.class.getDeclaredField("incompleteExpansionRooms");
      INCOMPLETE_FIELD.setAccessible(true);
    } catch (Exception e) {
      throw new ExceptionInInitializerError(e);
    }
  }

  private static void row(JsonObject obj) {
    System.out.println(GSON.toJson(obj));
  }

  private static JsonObject obj(String kind) {
    JsonObject o = new JsonObject();
    o.addProperty("type", kind);
    return o;
  }

  public static void main(String[] p_args) throws Exception {
    String path = p_args[0];
    String netName = p_args[1];
    String restriction = p_args[2];
    byte[] bytes = Files.readAllBytes(Paths.get(path));
    String fileName = Paths.get(path).getFileName().toString();

    BoardReadResult read =
        DsnReader.readBoard(
            new ByteArrayInputStream(bytes), null, new ItemIdGenerator(), fileName);
    if (!(read instanceof BoardReadResult.Success success)) {
      row(obj("parse-failed"));
      return;
    }
    BasicBoard basicBoard = success.board();
    RoutingBoard board = (RoutingBoard) basicBoard;
    board.searchTreeManager.reinsertTreeItems();
    switch (restriction) {
      case "NINETY" -> board.rules.setTraceAngleRestriction(AngleRestriction.NINETY_DEGREE);
      case "NONE" -> board.rules.setTraceAngleRestriction(AngleRestriction.NONE);
      default -> board.rules.setTraceAngleRestriction(AngleRestriction.FORTYFIVE_DEGREE);
    }
    int net = board.rules.nets.get(netName, 1).netNumber;

    Pin startPin = null;
    Pin destPin = null;
    for (Item item : board.getItems()) {
      if (!(item instanceof Pin pin) || !item.containsNet(net)) {
        continue;
      }
      if (startPin == null || pin.getId() < startPin.getId()) {
        destPin = startPin;
        startPin = pin;
      } else {
        destPin = pin;
      }
    }

    AutorouteControl ctrl = new AutorouteControl(board, net, new RouterSettings(board));
    ctrl.viasAllowed = false;
    ctrl.ripupAllowed = false;
    ctrl.isFanout = false;

    AutorouteEngine engine = new AutorouteEngine(board, ctrl.viaClearanceClass, true);
    engine.initConnection(net, null, null);
    INCOMPLETE_FIELD.set(engine, new ArrayList<>());

    MazeSearchEngine search =
        MazeSearchEngine.getInstance(Set.of(startPin), Set.of(destPin), engine, ctrl);
    JsonObject open = obj("searchOpen");
    open.addProperty("restriction", restriction);
    open.addProperty("net", net);
    open.addProperty("startId", startPin.getId());
    open.addProperty("destId", destPin.getId());
    row(open);

    long popCount = 0;
    while (search.occupyNextElement()) {
      popCount++;
    }
    JsonObject mr = obj("mazeResult");
    mr.addProperty("popCount", popCount);
    row(mr);

    // ---- the door table: BFS from the tree rooms through door
    // endpoints (obstacle rooms are not in the tree but hang off their
    // neighbours' doors).
    IntBox whole =
        new IntBox(new IntPoint(-100000000, -100000000), new IntPoint(100000000, 100000000));
    Set<SearchTreeObject> treeObjs = engine.autorouteSearchTree.overlappingObjects(whole, -1);
    List<ExpansionRoom> queue = new ArrayList<>();
    Map<ExpansionRoom, Boolean> seen = new IdentityHashMap<>();
    for (SearchTreeObject o : treeObjs) {
      if (o instanceof CompleteExpansionRoom room) {
        if (seen.put(room, Boolean.TRUE) == null) {
          queue.add(room);
        }
      }
    }
    Map<ExpansionDoor, Boolean> doorsSeen = new IdentityHashMap<>();
    for (int qi = 0; qi < queue.size(); qi++) {
      ExpansionRoom room = queue.get(qi);
      for (ExpansionDoor door : room.getDoors()) {
        if (doorsSeen.put(door, Boolean.TRUE) != null) {
          continue;
        }
        for (ExpansionRoom endpoint : new ExpansionRoom[] {door.firstRoom, door.secondRoom}) {
          if (seen.put(endpoint, Boolean.TRUE) == null) {
            queue.add(endpoint);
          }
        }
        ExpansionRoom a = door.firstRoom;
        ExpansionRoom b = door.secondRoom;
        boolean guard =
            door.dimension == 1
                || (a instanceof CompleteFreeSpaceExpansionRoom
                    && b instanceof CompleteFreeSpaceExpansionRoom);
        JsonObject d = obj("door");
        d.addProperty("a", a.getId());
        d.addProperty("b", b.getId());
        d.addProperty("dim", door.dimension);
        d.addProperty("ak", a.getClass().getSimpleName());
        d.addProperty("bk", b.getClass().getSimpleName());
        d.addProperty("guard", guard);
        TileShape shape = door.getShape();
        d.addProperty("empty", shape.isEmpty());
        for (ExpansionRoom endpoint : new ExpansionRoom[] {a, b}) {
          if (endpoint.getId() == 1 || endpoint.getId() == 49) {
            JsonObject r = obj("room");
            r.addProperty("id", endpoint.getId());
            r.addProperty("kind", endpoint.getClass().getSimpleName());
            r.addProperty("layer", endpoint.getLayer());
            JsonArray rc = new JsonArray();
            TileShape rs = endpoint.getShape();
            for (int i = 0; i < rs.borderLineCount(); i++) {
              app.freerouting.geometry.planar.FloatPoint c = rs.cornerApprox(i);
              JsonArray cp = new JsonArray(2);
              cp.add(c.x);
              cp.add(c.y);
              rc.add(cp);
            }
            r.add("corners", rc);
            row(r);
          }
        }
        if (!shape.isEmpty()) {
          IntBox bb = shape.boundingBox();
          d.addProperty("box", bb.maxWidth());
          app.freerouting.geometry.planar.IntOctagon oct = shape.boundingOctagon();
          d.addProperty("oct", oct == null ? -1.0 : oct.maxWidth());
          FloatLine diag = shape.diagonalCornerSegment();
          if (diag != null) {
            d.addProperty("diag", diag.b.distance(diag.a));
          }
          JsonArray corners = new JsonArray();
          for (int i = 0; i < shape.borderLineCount(); i++) {
            app.freerouting.geometry.planar.FloatPoint c = shape.cornerApprox(i);
            JsonArray cp = new JsonArray(2);
            cp.add(c.x);
            cp.add(c.y);
            corners.add(cp);
          }
          d.add("corners", corners);
        }
        row(d);
      }
    }
    row(obj("done"));
  }
}
