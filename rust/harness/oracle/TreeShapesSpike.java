// TreeShapesSpike.java — jar spike for M2 Task 6 (search-tree config,
// clearance compensation, drill-hole inflation) and Task 7 (the
// per-kind construction: trace / obstacle / conduction / component
// outline / board outline, the sectioning threshold, the keepoutOutside
// branches).
//
// Lives OUTSIDE src/ (harness-side; the frozen Java tree is never
// touched). Declares the app.freerouting.board.searchtree package for
// access to the package-private ShapeSearchTree(directions, board, cc)
// constructor and the protected drillHoleObstacle /
// drillHoleClearanceDelta members (the M2 plan sanctions package
// declarations for oracle launchers; the single-file launcher rejects
// package/path mismatch, hence the javac flow).
//
// Run (JDK 25, from the repo root):
//   mkdir -p /tmp/epic-t6-classes && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/javac -cp build/libs/freerouting-current-executable.jar \
//       -d /tmp/epic-t6-classes rust/harness/oracle/TreeShapesSpike.java && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/java -cp build/libs/freerouting-current-executable.jar:/tmp/epic-t6-classes \
//       app.freerouting.board.searchtree.TreeShapesSpike \
//       fixtures/Issue575-drc_dev-board_4_hole_clearance_violations.dsn \
//       | tee /tmp/epic-t6-treeshapes.out
//
// Ports the selection of DrillHoleClearanceShapeTest.java (first via
// with drillRadius>0, first pin with drillRadius>0 && tileShapeCount>0
// && getShape(0)!=null, first via with a copper-less layer) and dumps,
// per holeClearance in {0, 200000, 2500} and per tree variant (base
// cc0, 45-degree cc0, 90-degree cc0, 45-degree cc1, base cc1):
//   - every calculateTreeShapes result shape EXACTLY (box 4 ints /
//     octagon 8 ints / circle center+radius; never a formatting
//     toString),
//   - the per-shape inputs: shapeLayer(i), the CURRENT shape (post
//     null-synthesis, the exact object delta sees),
//     clearanceCompensationValue(itemClass, layer) and
//     drillHoleClearanceDelta(item, currentShape, layer),
//   - board facts: restriction, holeClearance, class names, the matrix
//     cells (with and without the safety margin), and per-item facts:
//     id, clearance class, center, padstack name, drillRadius
//     (Double.toString), holeOnly, spans.
//
// Output discipline (project pin rule 5): exact ints via field access,
// doubles via Double.toString.
package app.freerouting.board.searchtree;

import app.freerouting.board.actions.ItemIdGenerator;
import app.freerouting.board.facade.BasicBoard;
import app.freerouting.board.model.items.ComponentOutline;
import app.freerouting.board.model.items.ConductionArea;
import app.freerouting.board.model.items.DrillItem;
import app.freerouting.board.model.items.Item;
import app.freerouting.board.model.items.ObstacleArea;
import app.freerouting.board.model.items.Pin;
import app.freerouting.board.model.items.Via;
import app.freerouting.board.model.structure.AngleRestriction;
import app.freerouting.board.model.structure.BoardOutline;
import app.freerouting.board.model.structure.Unit;
import app.freerouting.board.trace.PolylineTrace;
import app.freerouting.core.library.Padstack;
import app.freerouting.datastructures.IdGenerator;
import app.freerouting.geometry.planar.Circle;
import app.freerouting.geometry.planar.FloatPoint;
import app.freerouting.geometry.planar.FortyfiveDegreeBoundingDirections;
import app.freerouting.geometry.planar.IntBox;
import app.freerouting.geometry.planar.IntOctagon;
import app.freerouting.geometry.planar.IntPoint;
import app.freerouting.geometry.planar.Point;
import app.freerouting.geometry.planar.PolygonShape;
import app.freerouting.geometry.planar.Shape;
import app.freerouting.geometry.planar.TileShape;
import app.freerouting.io.BoardReadResult;
import app.freerouting.io.specctra.DsnReader;
import app.freerouting.rules.BoardRules;
import java.io.ByteArrayInputStream;
import java.nio.file.Files;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;

public final class TreeShapesSpike {

  // ---- exact renderers -------------------------------------------------------

  private static String fmt(Shape shape) {
    if (shape == null) {
      return "null";
    }
    if (shape instanceof IntBox b) {
      return "box[" + b.ll.x + " " + b.ll.y + " " + b.ur.x + " " + b.ur.y + "]";
    }
    if (shape instanceof IntOctagon o) {
      return "oct["
          + o.leftX
          + " "
          + o.bottomY
          + " "
          + o.rightX
          + " "
          + o.topY
          + " "
          + o.upperLeftDiagonalX
          + " "
          + o.lowerRightDiagonalX
          + " "
          + o.lowerLeftDiagonalX
          + " "
          + o.upperRightDiagonalX
          + "]";
    }
    if (shape instanceof Circle c) {
      String center;
      if (c.center instanceof IntPoint p) {
        center = p.x + " " + p.y;
      } else {
        FloatPoint f = c.center.toFloat();
        center = Double.toString(f.x) + " " + Double.toString(f.y);
      }
      return "circle[" + center + " r=" + Double.toString(c.radius) + "]";
    }
    if (shape instanceof PolygonShape poly) {
      IntBox bb = (IntBox) poly.boundingBox();
      return "poly[box " + bb.ll.x + " " + bb.ll.y + " " + bb.ur.x + " " + bb.ur.y + "]";
    }
    if (shape instanceof TileShape t) {
      // Simplex: print through its bounding octagon (never a formatting
      // toString).
      return "tile[" + fmt(t.boundingOctagon()) + "]";
    }
    return shape.getClass().getSimpleName();
  }

  private static String pt(Point point) {
    if (point instanceof IntPoint p) {
      return p.x + " " + p.y;
    }
    FloatPoint f = point.toFloat();
    return Double.toString(f.x) + " " + Double.toString(f.y);
  }

  // ---- item metadata ---------------------------------------------------------

  private static void dumpItem(String label, DrillItem item) {
    if (item == null) {
      System.out.println("ITEM " + label + " none");
      return;
    }
    Padstack ps = item.getPadstack();
    System.out.println(
        "ITEM "
            + label
            + " kind="
            + item.getClass().getSimpleName()
            + " id="
            + item.getId()
            + " class="
            + item.clearanceClassIndex()
            + " center="
            + pt(item.getCenter())
            + " padstack="
            + (ps == null ? "null" : ps.name)
            + " drillRadius="
            + (ps == null ? "na" : Double.toString(ps.getDrillRadius()))
            + " holeOnly="
            + (ps != null && ps.holeOnly)
            + " psFrom="
            + (ps == null ? "na" : ps.fromLayer())
            + " psTo="
            + (ps == null ? "na" : ps.toLayer())
            + " psLayerCount="
            + (ps == null ? "na" : ps.boardLayerCount())
            + " firstLayer="
            + item.firstLayer()
            + " lastLayer="
            + item.lastLayer()
            + " tileShapeCount="
            + item.tileShapeCount());
    for (int i = 0; i < item.tileShapeCount(); i++) {
      Shape shape = item.getShape(i);
      String borderDistance;
      if (shape == null) {
        borderDistance = "na";
      } else {
        borderDistance = Double.toString(shape.borderDistance(item.getCenter().toFloat()));
      }
      System.out.println(
          "  SHAPE i="
              + i
              + " layer="
              + item.shapeLayer(i)
              + " raw="
              + fmt(shape)
              + " borderDistanceToCenter="
              + borderDistance);
    }
    // The delta fallback input: the RAW padstack shape per layer and its
    // border distance from the origin.
    if (ps != null) {
      for (int layer = 0; layer < ps.boardLayerCount(); layer++) {
        Shape raw = ps.getShape(layer);
        String borderDistance;
        if (raw == null) {
          borderDistance = "na";
        } else {
          borderDistance = Double.toString(raw.borderDistance(FloatPoint.ZERO));
        }
        System.out.println("  PADSTACK l=" + layer + " raw=" + fmt(raw) + " bd0=" + borderDistance);
      }
    }
  }

  // ---- tree dumps ------------------------------------------------------------

  private static void dumpTreeShapes(
      String treeLabel, ShapeSearchTree tree, BasicBoard board, DrillItem item) {
    // The CURRENT shape per index (post null-synthesis) — exactly what
    // calculateTreeShapes sees (ShapeSearchTree.java:877-880).
    Shape[] current = new Shape[item.tileShapeCount()];
    for (int i = 0; i < current.length; i++) {
      current[i] = item.getShape(i);
      if (current[i] == null) {
        current[i] = tree.drillHoleObstacle(item);
      }
    }
    TileShape[] result = tree.calculateTreeShapes(item);
    System.out.println(
        "SHAPES hc="
            + board.rules.getHoleClearance()
            + " tree="
            + tree.key
            + " ("
            + treeLabel
            + ")"
            + " item="
            + item.getClass().getSimpleName()
            + " id="
            + item.getId()
            + " n="
            + result.length);
    for (int i = 0; i < result.length; i++) {
      int layer = item.shapeLayer(i);
      int comp = tree.clearanceCompensationValue(item.clearanceClassIndex(), layer);
      int delta;
      if (current[i] == null) {
        delta = 0;
      } else {
        delta = tree.drillHoleClearanceDelta(item, current[i], layer);
      }
      System.out.println(
          "  i="
              + i
              + " layer="
              + layer
              + " comp="
              + comp
              + " delta="
              + delta
              + " current="
              + fmt(current[i])
              + " treeShape="
              + fmt(result[i]));
    }
  }

  public static void main(String[] args) throws Exception {
    String path =
        args.length >= 1
            ? args[0]
            : "fixtures/Issue575-drc_dev-board_4_hole_clearance_violations.dsn";
    byte[] bytes = Files.readAllBytes(Paths.get(path));
    IdGenerator idGenerator = new ItemIdGenerator();
    BoardReadResult read =
        DsnReader.readBoard(
            new ByteArrayInputStream(bytes), null, idGenerator, Paths.get(path).getFileName().toString());
    if (!(read instanceof BoardReadResult.Success success)) {
      System.out.println("READ_FAILED " + read);
      System.exit(1);
      return;
    }
    BasicBoard board = success.board();

    // ---- board facts ---------------------------------------------------------
    int layerCount = board.layerStructure.layers.length;
    int classCount = board.rules.clearanceMatrix.getClassCount();
    System.out.println(
        "BOARD items="
            + board.getItems().size()
            + " layerCount="
            + layerCount
            + " restriction="
            + board.rules.getTraceAngleRestriction()
            + " holeClearance="
            + board.rules.getHoleClearance()
            + " classCount="
            + classCount
            + " defaultClass="
            + BoardRules.defaultClearanceClass());
    for (int i = 0; i < classCount; i++) {
      System.out.println(
          "  CLASS " + i + " name=" + board.rules.clearanceMatrix.getName(i));
    }
    for (int layer = 0; layer < layerCount; layer++) {
      System.out.println("  LAYER " + layer + " " + board.layerStructure.layers[layer].name);
      for (int j = 0; j < classCount; j++) {
        StringBuilder cells = new StringBuilder();
        for (int i = 0; i < classCount; i++) {
          if (i > 0) {
            cells.append(',');
          }
          cells
              .append('v')
              .append(i)
              .append('.')
              .append(j)
              .append('=')
              .append(board.rules.clearanceMatrix.getValue(i, j, layer, false));
        }
        System.out.println(
            "    MATRIX l="
                + layer
                + " row_j="
                + j
                + " "
                + cells
                + " ccValue(j,l)="
                + board.rules.clearanceMatrix.clearanceCompensationValue(j, layer)
                + " maxValue(j,l)="
                + board.rules.clearanceMatrix.maxValue(j, layer));
      }
    }
    // The safety-margin form (ClearanceMatrix.java:17 margin 16): +16 on
    // every cell through getValue(..., true).
    System.out.println(
        "  MARGIN v1.1.l0 no="
            + board.rules.clearanceMatrix.getValue(1, 1, 0, false)
            + " with="
            + board.rules.clearanceMatrix.getValue(1, 1, 0, true));

    // ---- item selection (mirrors DrillHoleClearanceShapeTest) ---------------
    List<Via> vias = new ArrayList<>(board.getVias());
    List<Pin> pins = new ArrayList<>(board.getPins());
    StringBuilder viaIds = new StringBuilder("VIA_ORDER");
    for (Via via : vias) {
      viaIds.append(' ').append(via.getId());
    }
    System.out.println(viaIds);
    StringBuilder pinIds = new StringBuilder("PIN_ORDER_FIRST_12");
    for (int i = 0; i < Math.min(12, pins.size()); i++) {
      pinIds.append(' ').append(pins.get(i).getId());
    }
    System.out.println(pinIds);

    Via selVia = null;
    for (Via via : vias) {
      if (via.getPadstack() != null && via.getPadstack().getDrillRadius() > 0) {
        selVia = via;
        break;
      }
    }
    Pin selPin = null;
    for (Pin pin : pins) {
      if (pin.getPadstack() != null
          && pin.getPadstack().getDrillRadius() > 0
          && pin.tileShapeCount() > 0
          && pin.getShape(0) != null) {
        selPin = pin;
        break;
      }
    }
    System.out.println("SELECT via=" + (selVia == null ? "none" : selVia.getId()));
    System.out.println("SELECT pin=" + (selPin == null ? "none" : selPin.getId()));

    dumpItem("via", selVia);
    dumpItem("pin", selPin);

    // Branch pins: the first pin whose layer-0 shape is a POLYGON (the
    // RoundRect pads — borderDistance is NOT IMPLEMENTED on PolygonShape
    // and returns 0, PolygonShape.java:184-187) and the first with an
    // IntBox layer-0 shape (the Rect pads — the 45-degree tree's
    // isIntBox swap, ShapeSearchTree45Degree.java:502-508).
    java.util.Map<String, Integer> shapeKindCensus = new java.util.TreeMap<>();
    Pin polyPin = null;
    Pin boxPin = null;
    for (Pin pin : pins) {
      if (pin.tileShapeCount() <= 0 || pin.getShape(0) == null) {
        continue;
      }
      String kind = pin.getShape(0).getClass().getSimpleName();
      shapeKindCensus.merge(kind, 1, Integer::sum);
      if (polyPin == null && pin.getShape(0) instanceof PolygonShape) {
        polyPin = pin;
      }
      if (boxPin == null && pin.getShape(0) instanceof IntBox) {
        boxPin = pin;
      }
    }
    System.out.println("PIN_SHAPE_KINDS " + shapeKindCensus);
    System.out.println("SELECT polyPin=" + (polyPin == null ? "none" : polyPin.getId()));
    System.out.println("SELECT boxPin=" + (boxPin == null ? "none" : boxPin.getId()));
    dumpItem("polyPin", polyPin);
    dumpItem("boxPin", boxPin);

    // ---- tree shape dumps ----------------------------------------------------
    dumpAllTrees(board, selVia, selPin, polyPin, boxPin);

    // ---- SYNTHETIC hole-padstack vias (the null-in-span case) ---------------
    // The DSN-exported padstacks of these fixtures define a shape on every
    // layer, so the copper-less-layer branch (ShapeSearchTree.java:877-882)
    // never arises from the parse. Insert it through the real board API
    // (the ItemGeometrySpike precedent): a padstack with copper ONLY on
    // layers 0 and 3 of the 4-layer board -> span 0..3 with layers 1,2
    // null. The colon name gives a parseable drill radius
    // (smallestRadius 3000 x 300/600 = 1500.0). The second via repeats
    // it with clearance class 0 (the clearanceCompensationValue guard,
    // ShapeSearchTree.java:105-107).
    if (layerCount >= 4) {
      app.freerouting.geometry.planar.ConvexShape[] holeShapes =
          new app.freerouting.geometry.planar.ConvexShape[layerCount];
      holeShapes[0] = new Circle(new IntPoint(0, 0), 3000);
      holeShapes[layerCount - 1] = new Circle(new IntPoint(0, 0), 3000);
      Padstack holePadstack =
          board.library.padstacks.add("spike_hole_600:300", holeShapes, true, false);
      Via holeVia =
          board.insertVia(
              holePadstack,
              new IntPoint(700000, -300000),
              new int[0],
              1,
              app.freerouting.board.model.structure.FixedState.SYSTEM_FIXED,
              false);
      Via holeViaC0 =
          board.insertVia(
              holePadstack,
              new IntPoint(800000, -400000),
              new int[0],
              0,
              app.freerouting.board.model.structure.FixedState.SYSTEM_FIXED,
              false);
      System.out.println(
          "SYNTH padstack=" + holePadstack.name + " id=" + holePadstack.id);
      dumpItem("holeVia", holeVia);
      dumpItem("holeViaC0", holeViaC0);
      dumpAllTrees(board, holeVia, holeViaC0);
    }
    board.rules.setHoleClearance(0);

    // ---- BASE-DISPATCH restriction sweep -----------------------------------
    // The BASE calculateTreeShapes reads the board's angle restriction at
    // CALL time (ShapeSearchTree.java:885-893); every corpus fixture
    // parses as FORTYFIVE_DEGREE, so the boundingBox and boundingTile
    // hull arms have no fixture capture. Re-query the BASE tree (the
    // subclasses ignore the board restriction) under NINETY_DEGREE and
    // NONE, at hc=0 and hc=200000, for the circle via and the box pin
    // (the box pin is where enlarge-octagon vs offset-box diverges).
    for (AngleRestriction restriction :
        new AngleRestriction[] {
          AngleRestriction.NINETY_DEGREE, AngleRestriction.NONE
        }) {
      board.rules.setTraceAngleRestriction(restriction);
      System.out.println("RESTRICTION_SWEEP " + restriction);
      for (int hc : new int[] {0, 200000}) {
        board.rules.setHoleClearance(hc);
        ShapeSearchTree base0 =
            new ShapeSearchTree(FortyfiveDegreeBoundingDirections.INSTANCE, board, 0);
        if (selVia != null) {
          dumpTreeShapes("sweep-base0", base0, board, selVia);
        }
        if (boxPin != null) {
          dumpTreeShapes("sweep-base0", base0, board, boxPin);
        }
      }
    }
    board.rules.setHoleClearance(0);
    board.rules.setTraceAngleRestriction(AngleRestriction.FORTYFIVE_DEGREE);

    // ---- T7: the per-kind construction -------------------------------------
    t7Phase(board);

    // A second fixture (the conduction-area board): T7 phases only.
    if (args.length >= 2) {
      byte[] more = Files.readAllBytes(Paths.get(args[1]));
      IdGenerator idGenerator2 = new ItemIdGenerator();
      BoardReadResult read2 =
          DsnReader.readBoard(
              new ByteArrayInputStream(more),
              null,
              idGenerator2,
              Paths.get(args[1]).getFileName().toString());
      if (read2 instanceof BoardReadResult.Success success2) {
        t7Phase(success2.board());
      } else {
        System.out.println("READ_FAILED2 " + read2);
      }
    }

    // The crafted threshold boards (see the constants below).
    t7Phase(parseDsn(T7_SECTION_DSN, "t7-section.dsn"));
    t7Phase(parseDsn(T7_CLAMP_DSN, "t7-clamp.dsn"));
    t7Phase(parseDsn(T7_NOHOST_DSN, "t7-nohost.dsn"));
  }

  // ---- T7: crafted boards ----------------------------------------------------
  //
  // The obstacle path's sectioning threshold is
  //   50000, but min(500 * resolution-in-mils, 50000) when a host CAD
  //   was recorded (ShapeSearchTree.java:916-920). Three boards hold the
  //   keepout's INTERNAL box identical (120000x60000) and vary only the
  //   clamp inputs:
  //   * SECTION (um 10 + host CAD): resMil 254.0 -> 500x254 = 127000 ->
  //     the CAP wins, threshold 50000 (the corpus um-10 situation),
  //   * CLAMP (um 1 + host CAD): resMil 25.4 -> 500x25.4 = 12700 ->
  //     the CLAMP wins, threshold 12700 — a finer section grid,
  //   * NOHOST (um 1, NO host CAD): the guard skips the min entirely ->
  //     threshold 50000 even at um 1.
  // CLAMP vs NOHOST isolates the hostCadExists GUARD; SECTION vs CLAMP
  // isolates the resolution ARITHMETIC.

  private static final String T7_SECTION_DSN =
      """
      (pcb t7-section.dsn
        (parser
          (host_cad KICAD)
          (string_quote ")
          (space_in_quoted_tokens on)
        )
        (resolution um 10)
        (unit um)
        (structure
          (layer F.Cu (type signal))
          (layer B.Cu (type signal))
          (boundary (rect pcb 0 0 14000 8000))
          (keepout (rect F.Cu 0 0 12000 6000))
          (rule (width 250) (clearance 200))
        )
        (placement)
        (library)
        (network
          (net T7NET)
        )
      )
      """;

  private static final String T7_CLAMP_DSN =
      """
      (pcb t7-clamp.dsn
        (parser
          (host_cad KICAD)
          (string_quote ")
          (space_in_quoted_tokens on)
        )
        (resolution um 1)
        (unit um)
        (structure
          (layer F.Cu (type signal))
          (layer B.Cu (type signal))
          (boundary (rect pcb 0 0 140000 80000))
          (keepout (rect F.Cu 0 0 120000 60000))
          (rule (width 250) (clearance 200))
        )
        (placement)
        (library)
        (network
          (net T7NET)
        )
      )
      """;

  private static final String T7_NOHOST_DSN =
      """
      (pcb t7-nohost.dsn
        (parser
          (string_quote ")
          (space_in_quoted_tokens on)
        )
        (resolution um 1)
        (unit um)
        (structure
          (layer F.Cu (type signal))
          (layer B.Cu (type signal))
          (boundary (rect pcb 0 0 140000 80000))
          (keepout (rect F.Cu 0 0 120000 60000))
          (rule (width 250) (clearance 200))
        )
        (placement)
        (library)
        (network
          (net T7NET)
        )
      )
      """;

  private static BasicBoard parseDsn(String dsn, String name) {
    IdGenerator idGenerator = new ItemIdGenerator();
    BoardReadResult read =
        DsnReader.readBoard(new ByteArrayInputStream(dsn.getBytes()), null, idGenerator, name);
    if (!(read instanceof BoardReadResult.Success success)) {
      System.out.println("READ_FAILED " + name + " " + read);
      return null;
    }
    return success.board();
  }

  /** Renders an item id for a T7_SELECT line (null-safe). */
  private static String id(Item item) {
    return item == null ? "none" : Integer.toString(item.getId());
  }

  private static void t7Phase(BasicBoard board) {
    if (board == null) {
      return;
    }
    // COMM facts — the resolution-clamp inputs of the obstacle threshold.
    double resMil = board.communication.getResolution(Unit.MIL);
    System.out.println(
        "COMM items="
            + board.getItems().size()
            + " hostCadExists="
            + board.communication.hostCadExists()
            + " unit="
            + board.communication.unit
            + " resolution="
            + board.communication.resolution
            + " resMil="
            + Double.toString(resMil)
            + " threshold="
            + Double.toString(Math.min(500 * resMil, 50000)));

    // Per-kind probes (ConductionArea BEFORE ObstacleArea — it is a
    // subclass; a fresh probe tree for the >=2-sections witness).
    ShapeSearchTree probeTree =
        new ShapeSearchTree(FortyfiveDegreeBoundingDirections.INSTANCE, board, 0);
    PolylineTrace firstTrace = null;
    PolylineTrace multiTrace = null;
    ObstacleArea obstacle = null;
    ObstacleArea sectionedObstacle = null;
    ConductionArea conduction = null;
    ComponentOutline componentOutline = null;
    BoardOutline outline = null;
    for (Item item : board.getItems()) {
      if (item instanceof ConductionArea area) {
        if (conduction == null) {
          conduction = area;
        }
      } else if (item instanceof ObstacleArea obs) {
        if (obstacle == null) {
          obstacle = obs;
        }
        if (sectionedObstacle == null && obs.treeShapeCount(probeTree) >= 2) {
          sectionedObstacle = obs;
        }
      } else if (item instanceof PolylineTrace trace) {
        if (firstTrace == null) {
          firstTrace = trace;
        }
        if (multiTrace == null && trace.tileShapeCount() >= 3) {
          multiTrace = trace;
        }
      } else if (item instanceof ComponentOutline comp) {
        if (componentOutline == null) {
          componentOutline = comp;
        }
      } else if (item instanceof BoardOutline bo) {
        if (outline == null) {
          outline = bo;
        }
      }
    }
    System.out.println(
        "T7_SELECT trace="
            + id(firstTrace)
            + " multiTrace="
            + id(multiTrace)
            + " obstacle="
            + id(obstacle)
            + " sectionedObstacle="
            + id(sectionedObstacle)
            + " conduction="
            + id(conduction)
            + " componentOutline="
            + id(componentOutline)
            + " outline="
            + id(outline));
    dumpKindAllTrees(
        board,
        firstTrace,
        multiTrace,
        obstacle,
        sectionedObstacle,
        conduction,
        componentOutline,
        outline);

    // The keepoutOutside branches on a REAL parse: the read inserted the
    // outline into the DEFAULT tree, so its line shapes are already
    // cached; generateKeepoutOutside does remove+insert with NO
    // derived-data clear (BoardOutline.java:234-244) — BUT the manager's
    // remove NULLS the whole searchTreesInfo container
    // (Item.java:1078-1080), so the post-flip default tree RECOMPUTES
    // and serves the fresh AREA shapes, same as a fresh tree identity.
    // Captured: Issue575 flips 8 line octagons -> 8 value-different
    // area octagons; Issue054 flips 132 -> 136 shapes (bug-094: an
    // earlier port reading assumed STALE line shapes here and was
    // disproved by exactly this capture).
    if (outline != null) {
      ShapeSearchTree defaultTree = board.searchTreeManager.getDefaultTree();
      System.out.println(
          "KEEPOUT_OUTLINE id="
              + outline.getId()
              + " flag="
              + outline.keepoutOutsideOutlineGenerated()
              + " lineCount="
              + outline.lineCount()
              + " halfWidth="
              + outline.getHalfWidth());
      dumpKindShapes("default-parse", defaultTree, board, outline);
      outline.generateKeepoutOutside(true);
      dumpKindShapes("default-after-flip-area", defaultTree, board, outline);
      dumpKindShapes("fresh45-after-flip-area", new ShapeSearchTree45Degree(board, 0), board, outline);
    }
  }

  /**
   * Dumps a NON-DRILL item's tree shapes through the REAL cache path
   * ({@code treeShapeCount(tree)} / {@code getTreeShape(tree, i)} — the
   * public Item surface ShapeTree.insert itself calls), one line per
   * shape: the layer and compensation the construction reads, and the
   * exact shape.
   */
  private static void dumpKindShapes(
      String treeLabel, ShapeSearchTree tree, BasicBoard board, Item item) {
    if (item == null) {
      return;
    }
    int n = item.treeShapeCount(tree);
    System.out.println(
        "KIND_SHAPES tree="
            + tree.key
            + " ("
            + treeLabel
            + ") kind="
            + item.getClass().getSimpleName()
            + " id="
            + item.getId()
            + " class="
            + item.clearanceClassIndex()
            + " n="
            + n);
    for (int i = 0; i < n; i++) {
      int layer = item.shapeLayer(i);
      int comp = tree.clearanceCompensationValue(item.clearanceClassIndex(), layer);
      System.out.println(
          "  i="
              + i
              + " layer="
              + layer
              + " comp="
              + comp
              + " treeShape="
              + fmt(item.getTreeShape(tree, i)));
    }
  }

  private static void dumpKindAllTrees(BasicBoard board, Item... targets) {
    // The non-drill constructions read no hole clearance (the delta is
    // drill-only) — a single pass at hc=0.
    board.rules.setHoleClearance(0);
    ShapeSearchTree base0 =
        new ShapeSearchTree(FortyfiveDegreeBoundingDirections.INSTANCE, board, 0);
    ShapeSearchTree45Degree deg450 = new ShapeSearchTree45Degree(board, 0);
    ShapeSearchTree90Degree deg900 = new ShapeSearchTree90Degree(board, 0);
    ShapeSearchTree45Degree deg451 = new ShapeSearchTree45Degree(board, 1);
    ShapeSearchTree base1 =
        new ShapeSearchTree(FortyfiveDegreeBoundingDirections.INSTANCE, board, 1);
    String[] labels = {"base0", "deg450", "deg900", "deg451", "base1"};
    ShapeSearchTree[] trees = {base0, deg450, deg900, deg451, base1};
    for (int t = 0; t < trees.length; t++) {
      for (Item item : targets) {
        dumpKindShapes(labels[t], trees[t], board, item);
      }
    }
  }

  private static void dumpAllTrees(
      BasicBoard board, DrillItem... targets) {
    int[] holeClearances = {0, 200000, 2500};
    for (int hc : holeClearances) {
      board.rules.setHoleClearance(hc);
      ShapeSearchTree base0 =
          new ShapeSearchTree(FortyfiveDegreeBoundingDirections.INSTANCE, board, 0);
      ShapeSearchTree45Degree deg450 = new ShapeSearchTree45Degree(board, 0);
      ShapeSearchTree90Degree deg900 = new ShapeSearchTree90Degree(board, 0);
      ShapeSearchTree45Degree deg451 = new ShapeSearchTree45Degree(board, 1);
      ShapeSearchTree base1 =
          new ShapeSearchTree(FortyfiveDegreeBoundingDirections.INSTANCE, board, 1);
      String[] labels = {"base0", "deg450", "deg900", "deg451", "base1"};
      ShapeSearchTree[] trees = {base0, deg450, deg900, deg451, base1};
      for (int t = 0; t < trees.length; t++) {
        for (DrillItem item : targets) {
          if (item != null) {
            dumpTreeShapes(labels[t], trees[t], board, item);
          }
        }
      }
    }
  }
}
