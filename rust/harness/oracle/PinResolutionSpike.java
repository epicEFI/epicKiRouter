// PinResolutionSpike.java — jar spike for M2 Task 3 (pin placement
// resolution). Lives OUTSIDE src/ (harness-side; the frozen tree is never
// touched). Run from the repo root:
//
//   ~/.jdks/jdk-25.0.4.1+1/bin/java -cp build/libs/freerouting-current-executable.jar \
//       rust/harness/oracle/PinResolutionSpike.java <fixture.dsn>
//       | tee /tmp/epic-t3-pins.out
//
// Fixture choice: NO tier-A fixture has a non-90-degree component rotation
// (scanned 2026-09-14: every `(place ...)` rotation in the 11 tier-A boards
// is a multiple of 90). The smallest tier-B fixture with non-90 rotations is
// KiCad_10_demos/StickHub.dsn (39 non-90 placements, 57 back-side
// placements, no `(flip_style ...)` scope, so the default mirror-FIRST
// style is in effect). The mirror-AFTER (rotate-first) branch and the
// Component.rotate turn-angle trap are covered by crafted Rust-side tests
// (both flip styles are exercised there); this spike pins the fixture-real
// branches: the 90-degree exact branch, the non-90 float branch, the
// back-side mirror-before branch, and (if the fixture has one) the
// Pin.getCenter pad-shape CORRECTION.
//
// Output discipline (project pin rule 5): every coordinate is printed as
// EXACT Java longs/ints via explicit field access — never FloatPoint
// .toString (it rounds to 4 fraction digits). Doubles print via
// Double.toString (raw). One PIN line per sampled pin, plus CORRECTION
// lines for every pin whose raw center falls outside its first pad shape.
//
// The per-pin line mirrors Pin.java:64-119 exactly:
//   - packagePin.relativeLocation (ImagePinIr.rel_location) — raw;
//   - pin.relativeLocation() — the resolved vector (branch applied);
//   - rawCenter = component.getLocation().translateBy(pin.relativeLocation())
//     (Pin.java:98) BEFORE the pad-shape check;
//   - firstShape = first non-null pin.getShape(i) over the padstack layer
//     span (Pin.java:106-111);
//   - corrected = !firstShape.containsInside(rawCenter) (Pin.java:114);
//   - expectedCenter = corrected ? firstShape.centreOfGravity().round()
//     (Pin.java:115) : rawCenter — must equal pin.getCenter().
import app.freerouting.board.actions.ItemIdGenerator;
import app.freerouting.board.facade.BasicBoard;
import app.freerouting.board.model.items.Item;
import app.freerouting.board.model.items.Pin;
import app.freerouting.board.model.structure.Component;
import app.freerouting.core.library.Package;
import app.freerouting.core.library.Padstack;
import app.freerouting.datastructures.IdGenerator;
import app.freerouting.geometry.planar.FloatPoint;
import app.freerouting.geometry.planar.IntPoint;
import app.freerouting.geometry.planar.Point;
import app.freerouting.geometry.planar.Shape;
import app.freerouting.io.BoardReadResult;
import app.freerouting.io.specctra.DsnReader;
import java.io.ByteArrayInputStream;
import java.nio.file.Files;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;

public class PinResolutionSpike {

  public static void main(String[] args) throws Exception {
    String path =
        args.length >= 1
            ? args[0]
            : "scripts/benchmark/fixtures/KiCad_10_demos/StickHub.dsn";
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
        "BOARD items="
            + board.getItems().size()
            + " components="
            + board.components.count()
            + " layers="
            + board.getLayerCount()
            + " flipStyleRotateFirst="
            + board.components.getFlipStyleRotateFirst());

    List<Pin> pins = new ArrayList<>();
    for (Item item : board.getItems()) {
      if (item instanceof Pin pin) {
        pins.add(pin);
      }
    }
    System.out.println("PINS total=" + pins.size());

    // Sample selection: for every DISTINCT (side, rotation) pair, the first
    // component and its first 2 pins (deterministic: item iteration order
    // is the board's, the per-pair first-wins keyed by the printed key).
    java.util.LinkedHashMap<String, Integer> takenPerKey = new java.util.LinkedHashMap<>();
    java.util.LinkedHashSet<String> seenKeys = new java.util.LinkedHashSet<>();
    int corrections = 0;
    for (Pin pin : pins) {
      Component component = board.components.get(pin.getComponentId());
      if (component == null || component.getLocation() == null) {
        continue;
      }
      IntPoint loc = (IntPoint) component.getLocation();
      double rotation = component.getRotationInDegree();
      boolean front = component.placedOnFront();
      String key = (front ? "front" : "back") + " rot=" + Double.toString(rotation);
      Package libPackage = component.getPackage();
      Package.Pin packagePin = libPackage.getPin(pin.getPinIndex());
      if (packagePin == null) {
        continue;
      }
      String branch = (rotation % 90 == 0) ? "90" : "float";
      String mirror =
          front ? "none" : (board.components.getFlipStyleRotateFirst() ? "after" : "before");

      // The raw center BEFORE the pad-shape correction (Pin.java:98).
      app.freerouting.geometry.planar.Vector rel = pin.relativeLocation();
      Point rawCenter = component.getLocation().translateBy(rel);
      Padstack padstack = pin.getPadstack();
      Shape firstShape = null;
      int shapeIndex = -1;
      if (padstack != null) {
        int fromLayer = padstack.fromLayer();
        int toLayer = padstack.toLayer();
        for (int i = 0; i < toLayer - fromLayer + 1; i++) {
          Shape current = pin.getShape(i);
          if (current != null) {
            firstShape = current;
            shapeIndex = i;
            break;
          }
        }
      }
      boolean corrected = firstShape != null && !firstShape.containsInside(rawCenter);
      Point expectedCenter = rawCenter;
      if (corrected) {
        FloatPoint gravity = firstShape.centreOfGravity();
        expectedCenter = gravity.round();
      }
      Point center = pin.getCenter();
      boolean agree = center.equals(expectedCenter);

      int taken = takenPerKey.getOrDefault(key, 0);
      boolean sample = taken < 2;
      if (sample || corrected) {
        takenPerKey.put(key, taken + 1);
        seenKeys.add(key);
        app.freerouting.geometry.planar.IntVector relRawVec =
            (app.freerouting.geometry.planar.IntVector) packagePin.relativeLocation;
        StringBuilder line = new StringBuilder("PIN");
        line.append(" comp=").append(component.id);
        line.append(" compName=").append(component.name);
        line.append(" pinIdx=").append(pin.getPinIndex());
        line.append(" side=").append(front ? "front" : "back");
        line.append(" rot=").append(Double.toString(rotation));
        line.append(" loc=").append(loc.x).append(' ').append(loc.y);
        line.append(" pkgPinRot=").append(Double.toString(packagePin.rotationInDegree));
        line.append(" relRaw=").append(relRawVec.x).append(' ').append(relRawVec.y);
        if (rel instanceof app.freerouting.geometry.planar.IntVector relVec) {
          line.append(" rel=").append(relVec.x).append(' ').append(relVec.y);
        } else {
          line.append(" rel=?").append(rel.getClass().getSimpleName());
        }
        line.append(" branch=").append(branch);
        line.append(" mirror=").append(mirror);
        if (rawCenter instanceof IntPoint rawInt) {
          line.append(" rawCenter=").append(rawInt.x).append(' ').append(rawInt.y);
        } else {
          line.append(" rawCenter=?");
        }
        line.append(" shapeIdx=").append(shapeIndex);
        line.append(" corrected=").append(corrected);
        if (corrected) {
          corrections++;
          FloatPoint gravity = firstShape.centreOfGravity();
          IntPoint correctedInt = (IntPoint) expectedCenter;
          line.append(" gravity=").append(Double.toString(gravity.x))
              .append(' ').append(Double.toString(gravity.y));
          line.append(" correctedCenter=").append(correctedInt.x).append(' ')
              .append(correctedInt.y);
        }
        if (center instanceof IntPoint centerInt) {
          line.append(" center=").append(centerInt.x).append(' ').append(centerInt.y);
        } else {
          line.append(" center=?");
        }
        line.append(" agree=").append(agree);
        System.out.println(line);
      }
    }
    System.out.println("SAMPLED_KEYS " + seenKeys.size() + " CORRECTIONS " + corrections);
    System.out.println("KEYS " + seenKeys);

    // ---------------------------------------------------------------------
    // SYNTHETIC pad-shape CORRECTION. No corpus fixture triggers
    // Pin.java:114 (every fixture pad shape contains its raw center), so the
    // correction branch is constructed here through the REAL board API: an
    // off-center pad box — IntBox (1000,1000)-(9000,9000) on layer 0, whose
    // padstack ORIGIN (0,0) lies OUTSIDE the box. With the package pin at
    // relativeLocation (0,0) the raw center equals the component location,
    // which the translated box does not contain, so getCenter must replace it
    // with centreOfGravity().round() (Pin.java:115).
    // ---------------------------------------------------------------------
    app.freerouting.core.library.Padstacks padstacks = board.library.padstacks;
    app.freerouting.geometry.planar.ConvexShape[] padShapes =
        new app.freerouting.geometry.planar.ConvexShape[board.getLayerCount()];
    padShapes[0] =
        new app.freerouting.geometry.planar.IntBox(
            new IntPoint(1000, 1000), new IntPoint(9000, 9000));
    app.freerouting.core.library.Padstack spikePadstack =
        padstacks.add("spike_offcenter", padShapes, false, false);
    Package.Pin spikePkgPin =
        new Package.Pin(
            "1", spikePadstack.id, new app.freerouting.geometry.planar.IntVector(0, 0), 0.0);
    Package spikePackage =
        board.library.packages.add(
            "spike_pkg",
            new Package.Pin[] {spikePkgPin},
            new Shape[0],
            new double[0],
            new boolean[0],
            new Package.Keepout[0],
            new Package.Keepout[0],
            new Package.Keepout[0],
            true);
    Component spikeComponent =
        board.components.add(
            "SPIKE1", new IntPoint(1_000_000, -1_000_000), 0.0, true, spikePackage,
            spikePackage, false, null);
    Pin spikePin =
        board.insertPin(
            spikeComponent.id, 0, new int[0], 1,
            app.freerouting.board.model.structure.FixedState.SYSTEM_FIXED);

    app.freerouting.geometry.planar.Vector spikeRel = spikePin.relativeLocation();
    Point spikeRaw = spikeComponent.getLocation().translateBy(spikeRel);
    Shape spikeShape = null;
    for (int i = 0; i < spikePadstack.toLayer() - spikePadstack.fromLayer() + 1; i++) {
      Shape current = spikePin.getShape(i);
      if (current != null) {
        spikeShape = current;
        break;
      }
    }
    boolean spikeCorrected = spikeShape != null && !spikeShape.containsInside(spikeRaw);
    FloatPoint spikeGravity = spikeShape.centreOfGravity();
    Point spikeExpected =
        spikeCorrected ? spikeGravity.round() : spikeRaw;
    Point spikeCenter = spikePin.getCenter();
    System.out.println(
        "SYNTHETIC comp=" + spikeComponent.id
            + " loc=1000000 -1000000"
            + " rel=" + ((app.freerouting.geometry.planar.IntVector) spikeRel).x + " "
                + ((app.freerouting.geometry.planar.IntVector) spikeRel).y
            + " rawCenter=" + ((IntPoint) spikeRaw).x + " " + ((IntPoint) spikeRaw).y
            + " corrected=" + spikeCorrected
            + " gravity=" + Double.toString(spikeGravity.x) + " " + Double.toString(spikeGravity.y)
            + " expected=" + ((IntPoint) spikeExpected).x + " " + ((IntPoint) spikeExpected).y
            + " center=" + ((IntPoint) spikeCenter).x + " " + ((IntPoint) spikeCenter).y
            + " agree=" + spikeCenter.equals(spikeExpected));

    // ---------------------------------------------------------------------
    // PHASE 6: rotate-first (mirror-AFTER) coverage. Everything above ran
    // under the fixture's default mirror-FIRST style. Flipping the LIVE
    // table flag re-queries (a) an existing back-side fixture pin — same
    // pin, both styles, the two captures differ — and (b) a synthetic
    // back-side component with PIN ROTATION 90 and a 2-layer padstack
    // whose per-layer shapes are DISTINCT and asymmetric, so the
    // back-side padstack-layer remap (layerCount - index - firstLayer - 1,
    // Pin.java:248-257), the pin-rotation arm of Pin.getShape
    // (Pin.java:200-213), and the getCenter pad-shape CORRECTION through
    // the full transform chain are all observable. (c) flips the style
    // back to mirror-FIRST BEFORE inserting a FRESH pin (SPIKE3, same
    // geometry) — the mirror-BEFORE arm of getShape. (Re-querying the
    // SPIKE2 pin instead would return Java's MEMOIZED shapes — see the
    // note at (c).)
    // ---------------------------------------------------------------------
    board.components.setFlipStyleRotateFirst(true);
    Pin flipPin = null;
    for (Pin pin : pins) {
      Component component = board.components.get(pin.getComponentId());
      if (component != null && component.id == 93 && pin.getPinIndex() == 0) {
        flipPin = pin;
        break;
      }
    }
    if (flipPin != null) {
      app.freerouting.geometry.planar.Vector flipRel = flipPin.relativeLocation();
      System.out.println(
          "FLIPSTYLE comp=93 pinIdx=0 rel="
              + ((app.freerouting.geometry.planar.IntVector) flipRel).x
              + " " + ((app.freerouting.geometry.planar.IntVector) flipRel).y);
    } else {
      System.out.println("FLIPSTYLE_MISSING");
    }

    app.freerouting.geometry.planar.ConvexShape[] mlShapes =
        new app.freerouting.geometry.planar.ConvexShape[board.getLayerCount()];
    mlShapes[0] =
        new app.freerouting.geometry.planar.IntBox(
            new IntPoint(-2000, -1000), new IntPoint(6000, 1000));
    mlShapes[1] =
        new app.freerouting.geometry.planar.IntBox(
            new IntPoint(1000, -5000), new IntPoint(11000, 4000));
    app.freerouting.core.library.Padstack mlPadstack =
        padstacks.add("spike_ml", mlShapes, false, false);
    Package.Pin mlPkgPin =
        new Package.Pin(
            "1", mlPadstack.id, new app.freerouting.geometry.planar.IntVector(20000, 10000), 90.0);
    Package mlPackage =
        board.library.packages.add(
            "spike_ml_pkg",
            new Package.Pin[] {mlPkgPin},
            new Shape[0],
            new double[0],
            new boolean[0],
            new Package.Keepout[0],
            new Package.Keepout[0],
            new Package.Keepout[0],
            true);
    Component mlComponent =
        board.components.add(
            "SPIKE2", new IntPoint(1_500_000, -900_000), 90.0, false, mlPackage,
            mlPackage, false, null);
    Pin mlPin =
        board.insertPin(
            mlComponent.id, 0, new int[0], 1,
            app.freerouting.board.model.structure.FixedState.SYSTEM_FIXED);
    app.freerouting.geometry.planar.Vector mlRel = mlPin.relativeLocation();
    Point mlRaw = mlComponent.getLocation().translateBy(mlRel);
    Shape mlShape0 = mlPin.getShape(0);
    Shape mlShape1 = mlPin.getShape(1);
    FloatPoint mlGravity = mlShape0.centreOfGravity();
    Point mlCenter = mlPin.getCenter();
    System.out.println(
        "MLPIN comp=" + mlComponent.id
            + " rel=" + ((app.freerouting.geometry.planar.IntVector) mlRel).x
                + " " + ((app.freerouting.geometry.planar.IntVector) mlRel).y
            + " rawCenter=" + ((IntPoint) mlRaw).x + " " + ((IntPoint) mlRaw).y
            + " shape0=" + corners(mlShape0)
            + " shape1=" + corners(mlShape1)
            + " gravity=" + Double.toString(mlGravity.x) + " " + Double.toString(mlGravity.y)
            + " center=" + ((IntPoint) mlCenter).x + " " + ((IntPoint) mlCenter).y);

    // (c) Java MEMOIZES the shape array (precalculatedShapes, computed
    // once on the FIRST getShape call — Pin.java:170-236), so
    // re-querying mlPin under a flipped style would return the CACHED
    // rotate-first shapes (tried: byte-identical corners — that pins
    // the cache, not the chain). The mirror-BEFORE arm is therefore
    // pinned with a FRESH pin: flip the style back BEFORE inserting
    // SPIKE3, same padstack/package geometry.
    board.components.setFlipStyleRotateFirst(false);
    Component mirrorFirstComponent =
        board.components.add(
            "SPIKE3", new IntPoint(1_500_000, -900_000), 90.0, false, mlPackage,
            mlPackage, false, null);
    Pin mirrorFirstPin =
        board.insertPin(
            mirrorFirstComponent.id, 0, new int[0], 1,
            app.freerouting.board.model.structure.FixedState.SYSTEM_FIXED);
    app.freerouting.geometry.planar.Vector mfRel = mirrorFirstPin.relativeLocation();
    Point mfCenter = mirrorFirstPin.getCenter();
    System.out.println(
        "SPIKE3_MIRRORFIRST comp=" + mirrorFirstComponent.id
            + " rel=" + ((app.freerouting.geometry.planar.IntVector) mfRel).x
                + " " + ((app.freerouting.geometry.planar.IntVector) mfRel).y
            + " shape0=" + corners(mirrorFirstPin.getShape(0))
            + " shape1=" + corners(mirrorFirstPin.getShape(1))
            + " center=" + ((IntPoint) mfCenter).x + " " + ((IntPoint) mfCenter).y);
  }

  /** Prints a tile shape's four corners as x,y;x,y;x,y;x,y (exact ints). */
  private static String corners(Shape shape) {
    if (!(shape instanceof app.freerouting.geometry.planar.TileShape tile)) {
      return shape == null ? "null" : "?" + shape.getClass().getSimpleName();
    }
    StringBuilder sb = new StringBuilder();
    for (int i = 0; i < 4; i++) {
      IntPoint c = (IntPoint) tile.corner(i);
      if (i > 0) {
        sb.append(';');
      }
      sb.append(c.x).append(',').append(c.y);
    }
    return sb.toString();
  }
}
