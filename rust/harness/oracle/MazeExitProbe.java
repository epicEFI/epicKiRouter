// MazeExitProbe.java — one-shot diagnostic for the M3-T6 drill-phase
// compare-corner investigation. Parses the T6 maze fixture, then dumps
// the exact inputs and outputs of `Pin.nearestTraceExitCorner` for the
// start pin (the `expandToDrill` compare-corner override):
//   - pin center, pad shape on layer 0
//   - board.rules.getPinEdgeToTurnDist()
//   - getTraceExitRestrictions(layer) for every pin layer
//   - nearestTraceExitCorner(drillGravity, compensatedHalfWidth, layer)
//   - ctrl.viaRadii / traceCosts / compensatedTraceHalfWidth
// Build/run exactly like MazeSpike.java (JDK 25, repo root).
package app.freerouting.autoroute.maze;

import app.freerouting.board.facade.BasicBoard;
import app.freerouting.board.facade.RoutingBoard;
import app.freerouting.board.model.items.Item;
import app.freerouting.board.model.items.Pin;
import app.freerouting.geometry.planar.FloatPoint;
import app.freerouting.geometry.planar.Shape;
import app.freerouting.io.BoardReadResult;
import app.freerouting.io.specctra.DsnReader;
import app.freerouting.settings.RouterSettings;
import com.google.gson.Gson;
import com.google.gson.JsonObject;
import java.io.ByteArrayInputStream;
import java.nio.file.Files;
import java.nio.file.Paths;

public class MazeExitProbe {

  private static final Gson GSON = new Gson();

  private static void row(JsonObject obj) {
    System.out.println(GSON.toJson(obj));
  }

  private static JsonObject obj(String kind) {
    JsonObject o = new JsonObject();
    o.addProperty("type", kind);
    return o;
  }

  private static String d(double v) {
    return Double.toString(v);
  }

  public static void main(String[] p_args) throws Exception {
    byte[] bytes = Files.readAllBytes(Paths.get(p_args[0]));
    BoardReadResult read =
        DsnReader.readBoard(
            new ByteArrayInputStream(bytes),
            null,
            new app.freerouting.board.actions.ItemIdGenerator(),
            Paths.get(p_args[0]).getFileName().toString());
    if (!(read instanceof BoardReadResult.Success success)) {
      row(obj("read-not-success"));
      System.exit(3);
      return;
    }
    BasicBoard basicBoard = success.board();
    RoutingBoard board = (RoutingBoard) basicBoard;
    int netA = board.rules.nets.get("NET_A", 1).netNumber;
    AutorouteControl ctrl = new AutorouteControl(board, netA, new RouterSettings(board));

    Pin startPin = null;
    for (Item item : board.getItems()) {
      if (item instanceof Pin pin && pin.containsNet(netA)) {
        if (startPin == null || pin.getId() < startPin.getId()) {
          startPin = pin;
        }
      }
    }
    if (startPin == null) {
      row(obj("no-pin"));
      System.exit(4);
      return;
    }

    JsonObject ctrlRow = obj("ctrl");
    ctrlRow.addProperty("viaRadius0", d(ctrl.viaRadii[0]));
    ctrlRow.addProperty("halfWidth0", ctrl.compensatedTraceHalfWidth[0]);
    ctrlRow.addProperty("cost0h", d(ctrl.traceCosts[0].horizontal()));
    ctrlRow.addProperty("cost0v", d(ctrl.traceCosts[0].vertical()));
    row(ctrlRow);

    JsonObject pinRow = obj("pin");
    pinRow.addProperty("id", startPin.getId());
    app.freerouting.geometry.planar.IntPoint pinCenter =
        (app.freerouting.geometry.planar.IntPoint) startPin.getCenter();
    pinRow.addProperty("centerX", pinCenter.x);
    pinRow.addProperty("centerY", pinCenter.y);
    pinRow.addProperty("firstLayer", startPin.firstLayer());
    pinRow.addProperty("lastLayer", startPin.lastLayer());
    Shape padShape = startPin.getShape(0);
    pinRow.addProperty("shapeClass", padShape.getClass().getSimpleName());
    pinRow.addProperty(
        "shapeBounds",
        padShape.boundingBox().ll.x
            + ","
            + padShape.boundingBox().ll.y
            + ","
            + padShape.boundingBox().ur.x
            + ","
            + padShape.boundingBox().ur.y);
    row(pinRow);

    JsonObject edgeRow = obj("edgeToTurnDist");
    edgeRow.addProperty("value", d(board.rules.getPinEdgeToTurnDist()));
    row(edgeRow);

    // The drill-gravity from-point of the capture's drill -1701202883
    // (full page (8,9) piece: centre of gravity (365120.5, 318812.5)).
    FloatPoint fromPoint = new FloatPoint(365121.0, 318813.0);
    for (int layer = startPin.firstLayer(); layer <= startPin.lastLayer(); layer++) {
      JsonObject rRow = obj("restrictions");
      rRow.addProperty("layer", layer);
      var restrictions = startPin.getTraceExitRestrictions(layer);
      rRow.addProperty("count", restrictions.size());
      for (var restriction : restrictions) {
        JsonObject r = new JsonObject();
        r.addProperty("direction", restriction.direction.toString());
        r.addProperty("minLength", d(restriction.minLength));
        rRow.add("r", r);
      }
      FloatPoint corner =
          startPin.nearestTraceExitCorner(fromPoint, ctrl.compensatedTraceHalfWidth[layer], layer);
      rRow.addProperty("cornerX", corner != null ? d(corner.x) : "null");
      rRow.addProperty("cornerY", corner != null ? d(corner.y) : "null");
      row(rRow);
    }
  }
}
