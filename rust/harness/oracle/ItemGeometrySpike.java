// ItemGeometrySpike.java — jar spike for M2 Task 4 (item geometry).
// Lives OUTSIDE src/ (harness-side; the frozen Java tree is never
// touched). Run from the repo root (two runs, two fixtures):
//
//   ~/.jdks/jdk-25.0.4.1+1/bin/java -cp build/libs/freerouting-current-executable.jar \
//       rust/harness/oracle/ItemGeometrySpike.java \
//       scripts/benchmark/fixtures/DAC2020_boards/DAC2020_bm08.dsn \
//       | tee /tmp/epic-t4-items-bm08.out
//
//   ~/.jdks/jdk-25.0.4.1+1/bin/java -cp build/libs/freerouting-current-executable.jar \
//       rust/harness/oracle/ItemGeometrySpike.java \
//       scripts/benchmark/fixtures/KiCad_10_demos/complex_hierarchy.dsn \
//       | tee /tmp/epic-t4-items-ch.out
//
// Fixture choice (2026-09-14):
//   - DAC2020_bm08.dsn is the SMALLEST tier-A fixture with a boundary
//     (5501 bytes; the outline is a closed 4-corner pcb path, 2 signal
//     layers). It carries the OUTLINE observables: the board bounding
//     box (outline bbox + offset(1000), Structure.java:1207-1208), the
//     keepout area derivation (BoardOutline.java:184-190 —
//     PolylineArea(board.boundingBox, shapes)), the line keepout
//     (HALF_WIDTH=100 inflation, BoardOutline.java:28 + the line branch
//     of ShapeSearchTree.calculateTreeShapes(BoardOutline):940-990), and
//     fixture-real drills.
//   - complex_hierarchy.dsn is the tier fixture whose layer 0
//     (top_copper) is (type power) — NOT signal — so DrillItem.minWidth
//     (:369-388) exercises its non-signal SKIP branch there. Its own
//     padstacks have equal shapes on both layers (skip anchor-blind),
//     so the skip is made non-degenerate SYNTHETICALLY below.
//
// Output discipline (project pin rule 5): every coordinate is an exact
// Java long/int via explicit field access — never a formatting
// toString. Doubles print via Double.toString (raw). Shape corners
// print as x,y;x,y;... for TileShapes.
import app.freerouting.board.actions.ItemIdGenerator;
import app.freerouting.board.facade.BasicBoard;
import app.freerouting.board.model.items.ComponentOutline;
import app.freerouting.board.model.items.DrillItem;
import app.freerouting.board.model.items.Item;
import app.freerouting.board.model.items.Pin;
import app.freerouting.board.model.items.Via;
import app.freerouting.board.model.structure.BoardOutline;
import app.freerouting.board.model.structure.Component;
import app.freerouting.core.library.Package;
import app.freerouting.core.library.Padstack;
import app.freerouting.datastructures.IdGenerator;
import app.freerouting.geometry.planar.Area;
import app.freerouting.geometry.planar.FloatPoint;
import app.freerouting.geometry.planar.IntBox;
import app.freerouting.geometry.planar.IntPoint;
import app.freerouting.geometry.planar.Point;
import app.freerouting.geometry.planar.PolylineArea;
import app.freerouting.geometry.planar.PolylineShape;
import app.freerouting.geometry.planar.Shape;
import app.freerouting.geometry.planar.TileShape;
import app.freerouting.geometry.planar.Vector;
import app.freerouting.io.BoardReadResult;
import app.freerouting.io.specctra.DsnReader;
import java.io.ByteArrayInputStream;
import java.nio.file.Files;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;

public class ItemGeometrySpike {

  public static void main(String[] args) throws Exception {
    String path =
        args.length >= 1
            ? args[0]
            : "scripts/benchmark/fixtures/DAC2020_boards/DAC2020_bm08.dsn";
    byte[] bytes = Files.readAllBytes(Paths.get(path));
    IdGenerator idGenerator = new ItemIdGenerator();
    BoardReadResult read =
        DsnReader.readBoard(
            new ByteArrayInputStream(bytes),
            null,
            idGenerator,
            Paths.get(path).getFileName().toString());
    if (!(read instanceof BoardReadResult.Success success)) {
      System.out.println("READ_FAILED " + read);
      System.exit(1);
      return;
    }
    BasicBoard board = success.board();
    System.out.println(
        "BOARD items=" + board.getItems().size()
            + " layers=" + board.getLayerCount()
            + " flipFirst=" + board.components.getFlipStyleRotateFirst());
    for (int i = 0; i < board.getLayerCount(); i++) {
      System.out.println(
          "LAYER " + i
              + " name=" + board.layerStructure.layers[i].name
              + " signal=" + board.layerStructure.layers[i].isSignal);
    }
    IntBox bbox = board.boundingBox;
    System.out.println("BOARD_BBOX " + box(bbox));

    // -------------------------------------------------------------------
    // OUTLINE: the keepout AREA (BoardOutline.java:184-190) and the LINE
    // keepout (the else branch of ShapeSearchTree.java:940-990, the only
    // consumer of HALF_WIDTH=100 — BoardOutline.java:28, :256-258).
    // getTileShape(i) (Item.java:195-200) routes through the DEFAULT
    // tree, whose per-layer compensation value is printed alongside so
    // the Rust pin can reproduce the exact offset width.
    // -------------------------------------------------------------------
    BoardOutline outline = null;
    for (Item item : board.getItems()) {
      if (item instanceof BoardOutline candidate) {
        outline = candidate;
        break;
      }
    }
    if (outline == null) {
      System.out.println("OUTLINE_MISSING");
    } else {
      int outlineClass = outline.clearanceClassIndex();
      System.out.println(
          "OUTLINE id=" + outline.getId()
              + " shapes=" + outline.shapeCount()
              + " lineCount=" + outline.lineCount()
              + " halfWidth=" + outline.getHalfWidth()
              + " keepoutOutside=" + outline.keepoutOutsideOutlineGenerated()
              + " clearanceClass=" + outlineClass);
      for (int layer = 0; layer < board.getLayerCount(); layer++) {
        System.out.println(
            "OUTLINE_CMP layer=" + layer
                + " class=" + outlineClass
                + " cmp=" + board.searchTreeManager.getDefaultTree()
                    .clearanceCompensationValue(outlineClass, layer));
      }
      for (int s = 0; s < outline.shapeCount(); s++) {
        PolylineShape shape = outline.getShape(s);
        System.out.println(
            "OUTLINE_SHAPE s=" + s + " lines=" + shape.borderLineCount()
                + " bbox=" + box(shape.boundingBox()));
      }
      Area keepout = outline.getKeepoutArea();
      System.out.println(
          "OUTLINE_KEEPOUT border=" + keepout.getBorder().getClass().getSimpleName()
              + " borderBBox=" + box(keepout.boundingBox())
              + " holes=" + keepout.getHoles().length);
      Shape[] holes = keepout.getHoles();
      for (int h = 0; h < holes.length; h++) {
        PolylineShape hole = (PolylineShape) holes[h];
        System.out.println(
            "OUTLINE_HOLE h=" + h
                + " lines=" + hole.borderLineCount()
                + " bbox=" + box(hole.boundingBox()));
      }
      // The LINE keepout: lineCount * layerCount tiles, each
      // offsetShape(HALF_WIDTH + cmp, 0) of the 3-line polyline through
      // the border corner (ShapeSearchTree.java:975-988).
      int tileCount = outline.tileShapeCount();
      System.out.println("OUTLINE_TILES n=" + tileCount);
      for (int i = 0; i < tileCount; i++) {
        TileShape tile = outline.getTileShape(i);
        System.out.println("OUTLINE_TILE i=" + i + " corners=" + corners(tile));
      }
    }

    // -------------------------------------------------------------------
    // DRILLS: fixture-real first/last layer + minWidth samples
    // (DrillItem.java:162-185, :369-388).
    // -------------------------------------------------------------------
    List<Via> vias = new ArrayList<>();
    List<Pin> pins = new ArrayList<>();
    for (Item item : board.getItems()) {
      if (item instanceof Via via) {
        vias.add(via);
      } else if (item instanceof Pin pin) {
        pins.add(pin);
      }
    }
    System.out.println("DRILLS vias=" + vias.size() + " pins=" + pins.size());
    int viaSamples = 0;
    for (Via via : vias) {
      if (viaSamples++ >= 2) {
        break;
      }
      printDrill("via", via, via.getPadstack(), via.attachAllowed);
    }
    int frontSamples = 0;
    int backSamples = 0;
    for (Pin pin : pins) {
      Component component = board.components.get(pin.getComponentId());
      if (component == null) {
        continue;
      }
      boolean front = component.placedOnFront();
      if (front && frontSamples < 2) {
        frontSamples++;
        printDrill("pin", pin, pin.getPadstack(), false);
      } else if (!front && backSamples < 1) {
        backSamples++;
        printDrill("pin", pin, pin.getPadstack(), false);
      }
    }

    // -------------------------------------------------------------------
    // SYNTHETIC drills (bm08): a partial-span asymmetric padstack whose
    // only shape sits on layer 1 — via first=last=1; a BACK-side pin on
    // the same padstack — first=last=0 (the mirrored span,
    // DrillItem.java:168/:181 with boardLayerCount - to/from - 1), its
    // shape mirrored through the mirror-BEFORE arm of Pin.getShape
    // (:219-221); and a full-span via with different per-layer shapes.
    // -------------------------------------------------------------------
    if (vias.isEmpty() && path.contains("bm08")) {
      syntheticDrills(board);
    }
    if (path.contains("complex_hierarchy")) {
      syntheticNonSignalSkip(board);
    }

    // -------------------------------------------------------------------
    // SYNTHETIC ComponentOutline.getArea (ComponentOutline.java:192-216)
    // — the SAME placement chain as ObstacleArea.getArea with isFront in
    // the mirror role. Inserted through the real board API so ids and
    // the components' flip style are live.
    // -------------------------------------------------------------------
    syntheticComponentOutlines(board);
  }

  private static void printDrill(String kind, DrillItem drill, Padstack padstack, boolean attach) {
    Point center = drill.getCenter();
    IntPoint c = (IntPoint) center;
    System.out.println(
        "DRILL kind=" + kind
            + " id=" + drill.getId()
            + " center=" + c.x + " " + c.y
            + " padstack=" + (padstack == null ? "null" : padstack.name)
            + " psFrom=" + (padstack == null ? "?" : padstack.fromLayer())
            + " psTo=" + (padstack == null ? "?" : padstack.toLayer())
            + " psLayerCount=" + (padstack == null ? "?" : padstack.boardLayerCount())
            + " placedFront=" + drill.isPlacedOnFront()
            + " firstLayer=" + drill.firstLayer()
            + " lastLayer=" + drill.lastLayer()
            + " attach=" + attach
            + " minWidth=" + Double.toString(drill.minWidth()));
  }

  /** Synthetic partial-span/back-side/full-span drills (2-layer board). */
  private static void syntheticDrills(BasicBoard board) {
    app.freerouting.core.library.Padstacks padstacks = board.library.padstacks;
    int layerCount = board.getLayerCount();

    // Partial-span asymmetric padstack: shape ONLY on layer 1.
    app.freerouting.geometry.planar.ConvexShape[] partial =
        new app.freerouting.geometry.planar.ConvexShape[layerCount];
    partial[1] = new IntBox(new IntPoint(-1000, -1000), new IntPoint(5000, 1000));
    Padstack partialPadstack = padstacks.add("spike_partial", partial, true, false);
    Via partialVia =
        board.insertVia(
            partialPadstack,
            new IntPoint(700000, -300000),
            new int[0],
            1,
            app.freerouting.board.model.structure.FixedState.SYSTEM_FIXED,
            false);
    printDrill("synthVia", partialVia, partialPadstack, false);
    Shape viaShape = partialVia.getShape(0);
    System.out.println(
        "SYNTH_VIA_SHAPE i=0 " + corners((TileShape) viaShape));

    // Back-side pin on the same padstack: mirrored span 2-1-1 = 0.
    Package.Pin partialPin =
        new Package.Pin(
            "1", partialPadstack.id, new app.freerouting.geometry.planar.IntVector(20000, 10000),
            0.0);
    Package partialPackage =
        board.library.packages.add(
            "spike_partial_pkg",
            new Package.Pin[] {partialPin},
            new Shape[0],
            new double[0],
            new boolean[0],
            new Package.Keepout[0],
            new Package.Keepout[0],
            new Package.Keepout[0],
            true);
    Component backComponent =
        board.components.add(
            "SPIKE_BACK", new IntPoint(1_200_000, -800_000), 0.0, false, partialPackage,
            partialPackage, false, null);
    Pin backPin =
        board.insertPin(
            backComponent.id, 0, new int[0], 1,
            app.freerouting.board.model.structure.FixedState.SYSTEM_FIXED);
    printDrill("synthBackPin", backPin, backPin.getPadstack(), false);
    Shape backShape = backPin.getShape(0);
    System.out.println(
        "SYNTH_BACKPIN_SHAPE i=0 " + corners((TileShape) backShape));

    // Full-span via with DIFFERENT per-layer shapes (both signal here).
    app.freerouting.geometry.planar.ConvexShape[] full =
        new app.freerouting.geometry.planar.ConvexShape[layerCount];
    full[0] = new IntBox(new IntPoint(-5000, -5000), new IntPoint(5000, 5000));
    full[1] = new IntBox(new IntPoint(-2000, -1000), new IntPoint(2000, 1000));
    Padstack fullPadstack = padstacks.add("spike_full", full, true, false);
    Via fullVia =
        board.insertVia(
            fullPadstack,
            new IntPoint(650000, -250000),
            new int[0],
            1,
            app.freerouting.board.model.structure.FixedState.SYSTEM_FIXED,
            false);
    printDrill("synthFullVia", fullVia, fullPadstack, false);
    System.out.println(
        "SYNTH_FULL_VIA_SHAPE i=0 " + corners((TileShape) fullVia.getShape(0)));
    System.out.println(
        "SYNTH_FULL_VIA_SHAPE i=1 " + corners((TileShape) fullVia.getShape(1)));
  }

  /**
   * Synthetic NON-SIGNAL skip for minWidth (complex_hierarchy: layer 0 is power, layer 1 signal).
   * The power-layer shape is the SMALLER one, so minWidth == 4000 proves the skip; a port that
   * forgets `!isSignal -> continue` returns 1000.
   */
  private static void syntheticNonSignalSkip(BasicBoard board) {
    app.freerouting.core.library.Padstacks padstacks = board.library.padstacks;
    int layerCount = board.getLayerCount();
    app.freerouting.geometry.planar.ConvexShape[] shapes =
        new app.freerouting.geometry.planar.ConvexShape[layerCount];
    shapes[0] = new IntBox(new IntPoint(-500, -500), new IntPoint(500, 500)); // power: 1000
    shapes[1] =
        new IntBox(new IntPoint(-2000, -2000), new IntPoint(2000, 2000)); // signal: 4000
    Padstack padstack = padstacks.add("spike_power", shapes, true, false);
    Via via =
        board.insertVia(
            padstack,
            new IntPoint(150000, -60000),
            new int[0],
            1,
            app.freerouting.board.model.structure.FixedState.SYSTEM_FIXED,
            false);
    printDrill("synthPowerVia", via, padstack, false);
  }

  /**
   * Synthetic ComponentOutline areas: a back-side (mirror branch) exact-90 turn and a front-side
   * rotateApprox, both through insertComponentOutline so the item is board-live.
   */
  private static void syntheticComponentOutlines(BasicBoard board) {
    PolylineShape border =
        new IntBox(new IntPoint(-5000, -2500), new IntPoint(5000, 2500));
    Area area = new PolylineArea(border, new PolylineShape[0]);
    ComponentOutline back =
        board.insertComponentOutline(
            area,
            false,
            new app.freerouting.geometry.planar.IntVector(1301800, -728800),
            90.0,
            0,
            true,
            false,
            true,
            app.freerouting.board.model.structure.FixedState.SYSTEM_FIXED);
    Area backArea = back.getArea();
    System.out.println(
        "CO_BACK id=" + back.getId()
            + " isFront=" + back.isFront()
            + " courtyard=" + back.isCourtyard()
            + " fabrication=" + back.isFabrication()
            + " closed=" + back.isClosed()
            + " border=" + backArea.getBorder().getClass().getSimpleName()
            + " bbox=" + box(backArea.boundingBox()));
    ComponentOutline approx =
        board.insertComponentOutline(
            area,
            true,
            new app.freerouting.geometry.planar.IntVector(1742920, -1341750),
            330.0,
            0,
            false,
            true,
            false,
            app.freerouting.board.model.structure.FixedState.SYSTEM_FIXED);
    Area approxArea = approx.getArea();
    System.out.println(
        "CO_APPROX id=" + approx.getId()
            + " isFront=" + approx.isFront()
            + " bbox=" + box(approxArea.boundingBox())
            + " border=" + corners((TileShape) approxArea.getBorder()));
  }

  /** "lx ly ux uy" via explicit int fields. */
  private static String box(IntBox box) {
    return box.ll.x + " " + box.ll.y + " " + box.ur.x + " " + box.ur.y;
  }

  /**
   * Prints a tile shape's corners as x,y;x,y;... — exact ints for IntPoint corners; non-integral
   * (rational) corners fall back to the raw double of toFloat() marked with a leading `r` (only
   * the all-integer fixtures' tiles are pinned; rational corners are diagnostic-only).
   */
  private static String corners(TileShape tile) {
    if (tile == null) {
      return "null";
    }
    StringBuilder sb = new StringBuilder();
    int n = tile.borderLineCount();
    for (int i = 0; i < n; i++) {
      Point corner = tile.corner(i);
      if (i > 0) {
        sb.append(';');
      }
      if (corner instanceof IntPoint c) {
        sb.append(c.x).append(',').append(c.y);
      } else {
        FloatPoint f = corner.toFloat();
        sb.append('r').append(Double.toString(f.x)).append(',').append(Double.toString(f.y));
      }
    }
    return sb.toString();
  }
}
