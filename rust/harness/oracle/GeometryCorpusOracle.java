// GeometryCorpusOracle.java — differential oracle for the epic-geometry port.
// Run (repo root):
//   ~/.jdks/jdk-25.0.4.1+1/bin/java -cp build/libs/freerouting-current-executable.jar \
//       rust/harness/oracle/GeometryCorpusOracle.java cases.jsonl > golden.jsonl
// Lives OUTSIDE src/ — the frozen tree is never touched; only the built jar is consumed.
//
// Reads one case per line: {"id":"...","op":"...","args":{...}} and prints one
// result line per case: {"id":"...","out":{...}}.
//
// Encoding conventions (mirrored by rust/harness/src/corpus.rs):
// - int point: [x,y]; float point: [x_bits,y_bits] (raw f64 bit patterns);
// - octagon: [lx,ly,rx,uy,ulx,lrx,llx,urx] (IntOctagon ctor order);
//   box: [llx,lly,urx,ury]; direction/vector: [x,y];
// - line: {"a":[x,y],"b":[x,y]}; float line: {"a":[bx,by],"b":[bx,by]};
// - doubles ALWAYS as {"bits":<i64>} (Double.doubleToRawLongBits), floats as
//   {"fbits":<i32>} (Float.floatToRawIntBits);
// - Line.intersection never returns null: a parallel/collinear pair yields an
//   infinite RationalPoint (z=0) and is serialized as {"kind":"Infinity"};
//   FloatLine.intersection DOES return null for parallel lines ({"kind":"Null"});
// - Side -> "left"/"collinear"/"right", Signum -> "pos"/"zero"/"neg";
//   compareTo-style results as {"cmp":signum}.
import app.freerouting.geometry.planar.Circle;
import app.freerouting.geometry.planar.Direction;
import app.freerouting.geometry.planar.FloatLine;
import app.freerouting.geometry.planar.FloatPoint;
import app.freerouting.geometry.planar.IntBox;
import app.freerouting.geometry.planar.IntOctagon;
import app.freerouting.geometry.planar.IntPoint;
import app.freerouting.geometry.planar.Line;
import app.freerouting.geometry.planar.LineSegment;
import app.freerouting.geometry.planar.Point;
import app.freerouting.geometry.planar.Polyline;
import app.freerouting.geometry.planar.RationalPoint;
import app.freerouting.geometry.planar.RationalVector;
import app.freerouting.geometry.planar.Side;
import app.freerouting.geometry.planar.Simplex;
import app.freerouting.geometry.planar.TileShape;
import app.freerouting.geometry.planar.Vector;
import app.freerouting.datastructures.Signum;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import java.io.BufferedReader;
import java.io.BufferedWriter;
import java.io.IOException;
import java.io.InputStreamReader;
import java.io.OutputStreamWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Paths;

public class GeometryCorpusOracle {

  /** Unknown op id — fatal (nonzero exit), unlike per-case eval failures. */
  static final class UnknownOpException extends RuntimeException {
    UnknownOpException(String op) {
      super(op);
    }
  }

  static String sideOut(Side s) {
    if (s == Side.ON_THE_LEFT) {
      return "left";
    }
    return s == Side.COLLINEAR ? "collinear" : "right";
  }

  static String signumOut(Signum s) {
    if (s == Signum.POSITIVE) {
      return "pos";
    }
    return s == Signum.ZERO ? "zero" : "neg";
  }

  static JsonObject strOut(String v) {
    JsonObject o = new JsonObject();
    o.addProperty("v", v);
    return o;
  }

  static JsonArray int2(int x, int y) {
    JsonArray a = new JsonArray(2);
    a.add(x);
    a.add(y);
    return a;
  }

  static IntPoint pt(JsonObject args, String key) {
    JsonArray a = args.getAsJsonArray(key);
    return new IntPoint(a.get(0).getAsInt(), a.get(1).getAsInt());
  }

  static FloatPoint fpt(JsonObject args, String key) {
    JsonArray a = args.getAsJsonArray(key);
    return new FloatPoint(
        Double.longBitsToDouble(a.get(0).getAsLong()), Double.longBitsToDouble(a.get(1).getAsLong()));
  }

  static Vector vec(JsonObject args, String key) {
    JsonArray a = args.getAsJsonArray(key);
    return Vector.getInstance(a.get(0).getAsInt(), a.get(1).getAsInt());
  }

  static IntBox box(JsonObject args, String key) {
    JsonArray a = args.getAsJsonArray(key);
    return new IntBox(a.get(0).getAsInt(), a.get(1).getAsInt(), a.get(2).getAsInt(), a.get(3).getAsInt());
  }

  static IntOctagon oct(JsonObject args, String key, boolean normalize) {
    JsonArray a = args.getAsJsonArray(key);
    IntOctagon o =
        new IntOctagon(
            a.get(0).getAsInt(),
            a.get(1).getAsInt(),
            a.get(2).getAsInt(),
            a.get(3).getAsInt(),
            a.get(4).getAsInt(),
            a.get(5).getAsInt(),
            a.get(6).getAsInt(),
            a.get(7).getAsInt());
    return normalize ? o.normalize() : o;
  }

  static Line line(JsonObject args, String key) {
    JsonObject l = args.getAsJsonObject(key);
    JsonObject a = l.getAsJsonObject("a");
    JsonObject b = l.getAsJsonObject("b");
    return new Line(
        new IntPoint(a.get("x").getAsInt(), a.get("y").getAsInt()),
        new IntPoint(b.get("x").getAsInt(), b.get("y").getAsInt()));
  }

  static FloatLine fline(JsonObject args, String key) {
    JsonObject l = args.getAsJsonObject(key);
    return new FloatLine(fpt(l, "a"), fpt(l, "b"));
  }

  static JsonObject ptOut(Point p) {
    JsonObject o = new JsonObject();
    if (p instanceof IntPoint ip) {
      o.addProperty("kind", "IntPoint");
      o.add("v", int2(ip.x, ip.y));
      return o;
    }
    RationalPoint rp = (RationalPoint) p;
    if (rp.isInfinite()) {
      o.addProperty("kind", "Infinity");
      return o;
    }
    // toFloat() is exact for this op table only because every emitted
    // RationalPoint arises from 45-degree cross products whose denominators
    // divide 2; arbitrary-slope inputs would need exact-rational
    // serialization (the fields are package-private).
    FloatPoint f = rp.toFloat();
    o.addProperty("kind", "RationalPoint");
    o.addProperty("x_bits", Double.doubleToRawLongBits(f.x));
    o.addProperty("y_bits", Double.doubleToRawLongBits(f.y));
    return o;
  }

  static JsonObject fptOut(FloatPoint p) {
    JsonObject o = new JsonObject();
    o.addProperty("kind", "FloatPoint");
    o.addProperty("x_bits", Double.doubleToRawLongBits(p.x));
    o.addProperty("y_bits", Double.doubleToRawLongBits(p.y));
    return o;
  }

  static JsonArray lineOutArr(Line l) {
    JsonArray a = new JsonArray(2);
    a.add(ptOut(l.a));
    a.add(ptOut(l.b));
    return a;
  }

  static JsonObject lineOut(Line l) {
    JsonObject o = new JsonObject();
    o.addProperty("kind", "Line");
    o.add("a", ptOut(l.a));
    o.add("b", ptOut(l.b));
    return o;
  }

  static JsonObject vecOut(Vector v) {
    JsonObject o = new JsonObject();
    if (v instanceof app.freerouting.geometry.planar.IntVector iv) {
      o.addProperty("kind", "IntVector");
      o.add("v", int2(iv.x, iv.y));
      return o;
    }
    RationalVector rv = (RationalVector) v;
    o.addProperty("kind", "RationalVector");
    o.addProperty("x", rv.x.toString());
    o.addProperty("y", rv.y.toString());
    o.addProperty("z", rv.z.toString());
    return o;
  }

  static JsonObject dirOut(Direction d) {
    Vector v = d.getVector();
    JsonObject o = new JsonObject();
    o.addProperty("kind", "Direction");
    if (v instanceof app.freerouting.geometry.planar.IntVector iv) {
      o.add("v", int2(iv.x, iv.y));
    } else {
      RationalVector rv = (RationalVector) v;
      o.addProperty("v", rv.x.toString() + "/" + rv.y.toString() + "/" + rv.z.toString());
    }
    return o;
  }

  /**
   * Emptiness by FIELD equality with the sentinel, not Java reference `==`:
   * the Rust port replaced reference identity with field equality (its
   * derived PartialEq), so the goldens must use the value-level contract too.
   */
  static boolean boxIsEmpty(IntBox b) {
    return b.ll.x == IntBox.EMPTY.ll.x
        && b.ll.y == IntBox.EMPTY.ll.y
        && b.ur.x == IntBox.EMPTY.ur.x
        && b.ur.y == IntBox.EMPTY.ur.y;
  }

  static boolean octIsEmpty(IntOctagon r) {
    return r.leftX == IntOctagon.EMPTY.leftX
        && r.bottomY == IntOctagon.EMPTY.bottomY
        && r.rightX == IntOctagon.EMPTY.rightX
        && r.topY == IntOctagon.EMPTY.topY
        && r.upperLeftDiagonalX == IntOctagon.EMPTY.upperLeftDiagonalX
        && r.lowerRightDiagonalX == IntOctagon.EMPTY.lowerRightDiagonalX
        && r.lowerLeftDiagonalX == IntOctagon.EMPTY.lowerLeftDiagonalX
        && r.upperRightDiagonalX == IntOctagon.EMPTY.upperRightDiagonalX;
  }

  static JsonObject boxOut(IntBox b) {
    JsonObject o = new JsonObject();
    o.addProperty("kind", "IntBox");
    o.add("ll", int2(b.ll.x, b.ll.y));
    o.add("ur", int2(b.ur.x, b.ur.y));
    o.addProperty("empty", boxIsEmpty(b));
    return o;
  }

  static JsonObject octOut(IntOctagon r) {
    JsonObject o = new JsonObject();
    o.addProperty("kind", "IntOctagon");
    JsonArray v = new JsonArray(8);
    v.add(r.leftX);
    v.add(r.bottomY);
    v.add(r.rightX);
    v.add(r.topY);
    v.add(r.upperLeftDiagonalX);
    v.add(r.lowerRightDiagonalX);
    v.add(r.lowerLeftDiagonalX);
    v.add(r.upperRightDiagonalX);
    o.add("v", v);
    o.addProperty("empty", octIsEmpty(r));
    return o;
  }

  static JsonObject simplexOut(Simplex s) {
    JsonObject o = new JsonObject();
    o.addProperty("kind", "Simplex");
    JsonArray lines = new JsonArray();
    for (int i = 0; i < s.borderLineCount(); i++) {
      lines.add(lineOut(s.borderLine(i)));
    }
    o.add("lines", lines);
    return o;
  }

  static JsonObject tileOut(TileShape t) {
    if (t instanceof IntBox b) {
      return boxOut(b);
    }
    if (t instanceof IntOctagon oct) {
      return octOut(oct);
    }
    return simplexOut((Simplex) t);
  }

  static JsonObject segOut(LineSegment s) {
    JsonObject o = new JsonObject();
    o.addProperty("kind", "LineSegment");
    o.add("start", lineOut(s.getStartClosingLine()));
    o.add("middle", lineOut(s.getLine()));
    o.add("end", lineOut(s.getEndClosingLine()));
    return o;
  }

  static JsonObject polyOut(Polyline p) {
    JsonObject o = new JsonObject();
    o.addProperty("kind", "Polyline");
    JsonArray lines = new JsonArray();
    for (Line l : p.lines) {
      lines.add(lineOut(l));
    }
    o.add("lines", lines);
    return o;
  }

  static JsonObject cmpOut(int c) {
    JsonObject o = new JsonObject();
    o.addProperty("cmp", Integer.signum(c));
    return o;
  }

  static JsonObject dblOut(double d) {
    JsonObject o = new JsonObject();
    o.addProperty("bits", Double.doubleToRawLongBits(d));
    return o;
  }

  static JsonObject boolOut(boolean b) {
    JsonObject o = new JsonObject();
    o.addProperty("v", b);
    return o;
  }

  static JsonObject longOut(long v) {
    JsonObject o = new JsonObject();
    o.addProperty("v", v);
    return o;
  }

  static Point[] points(JsonObject args, String key) {
    JsonArray a = args.getAsJsonArray(key);
    Point[] pts = new Point[a.size()];
    for (int i = 0; i < a.size(); i++) {
      JsonArray p = a.get(i).getAsJsonArray();
      pts[i] = new IntPoint(p.get(0).getAsInt(), p.get(1).getAsInt());
    }
    return pts;
  }

  static Line[] lines(JsonObject args, String key) {
    JsonArray a = args.getAsJsonArray(key);
    Line[] ls = new Line[a.size()];
    for (int i = 0; i < a.size(); i++) {
      JsonObject l = a.get(i).getAsJsonObject();
      JsonObject la = l.getAsJsonObject("a");
      JsonObject lb = l.getAsJsonObject("b");
      ls[i] =
          new Line(
              new IntPoint(la.get("x").getAsInt(), la.get("y").getAsInt()),
              new IntPoint(lb.get("x").getAsInt(), lb.get("y").getAsInt()));
    }
    return ls;
  }

  static JsonObject evaluateOp(String op, JsonObject args) {
    switch (op) {
      case "intpoint.determinant" -> {
        return longOut(pt(args, "a").determinant(pt(args, "b")));
      }
      case "intpoint.difference_by" -> {
        return vecOut(pt(args, "a").differenceBy(pt(args, "b")));
      }
      case "intpoint.fortyfive_degree_projection" -> {
        return ptOut(pt(args, "a").fortyfiveDegreeProjection(pt(args, "b")));
      }
      case "intpoint.perpendicular_projection" -> {
        return ptOut(line(args, "line").perpendicularProjection(pt(args, "p")));
      }
      case "intpoint.surrounding_octagon" -> {
        return octOut(pt(args, "a").surroundingOctagon());
      }
      case "vector.side_of" -> {
        return strOut(sideOut(vec(args, "a").sideOf(vec(args, "b"))));
      }
      case "vector.projection" -> {
        JsonObject o = new JsonObject();
        o.addProperty("v", signumOut(vec(args, "a").projection(vec(args, "b"))));
        return o;
      }
      case "vector.turn_90_degree" -> {
        return vecOut(vec(args, "a").turn90Degree(args.get("factor").getAsInt()));
      }
      case "direction.get_instance" -> {
        return dirOut(Direction.getInstance(vec(args, "v")));
      }
      case "direction.turn_45_degree" -> {
        return dirOut(Direction.getInstance(vec(args, "v")).turn45Degree(args.get("factor").getAsInt()));
      }
      case "direction.compare_to" -> {
        return cmpOut(
            Direction.getInstance(vec(args, "a"))
                .compareTo(Direction.getInstance(vec(args, "b"))));
      }
      case "point.compare_xy" -> {
        return cmpOut(pt(args, "a").compareXY(pt(args, "b")));
      }
      case "point.translate_by" -> {
        return ptOut(pt(args, "a").translateBy(vec(args, "v")));
      }
      case "intbox.intersection" -> {
        return boxOut(box(args, "a").intersection(box(args, "b")));
      }
      case "intbox.union" -> {
        return boxOut(box(args, "a").union(box(args, "b")));
      }
      case "intbox.offset_double" -> {
        return boxOut(
            box(args, "a").offset(Double.longBitsToDouble(args.get("dist_bits").getAsLong())));
      }
      case "intbox.area" -> {
        return dblOut(box(args, "a").area());
      }
      case "intbox.circumference" -> {
        return dblOut(box(args, "a").circumference());
      }
      case "intbox.intersects" -> {
        return boolOut(box(args, "a").intersects(box(args, "b")));
      }
      case "intbox.cutout" -> {
        TileShape[] parts = box(args, "a").cutout((TileShape) box(args, "b"));
        JsonObject o = new JsonObject();
        o.addProperty("kind", "TileShapeList");
        JsonArray arr = new JsonArray();
        for (TileShape t : parts) {
          arr.add(tileOut(t));
        }
        o.add("v", arr);
        return o;
      }
      case "octagon.normalize" -> {
        return octOut(oct(args, "a", false).normalize());
      }
      case "octagon.intersection" -> {
        return octOut(oct(args, "a", true).intersection(oct(args, "b", true)));
      }
      case "octagon.union" -> {
        return octOut(oct(args, "a", true).union(oct(args, "b", true)));
      }
      case "octagon.offset" -> {
        return octOut(oct(args, "a", true).offset(Double.longBitsToDouble(args.get("dist_bits").getAsLong())));
      }
      case "octagon.intersects" -> {
        return boolOut(oct(args, "a", true).intersects(oct(args, "b", true)));
      }
      case "octagon.overlaps" -> {
        return boolOut(oct(args, "a", true).overlaps(oct(args, "b", true)));
      }
      case "octagon.contains_point" -> {
        return boolOut(oct(args, "a", true).contains(pt(args, "p")));
      }
      case "octagon.contains_float_point" -> {
        return boolOut(oct(args, "a", true).contains(fpt(args, "p")));
      }
      case "octagon.area" -> {
        return dblOut(oct(args, "a", true).area());
      }
      case "octagon.bounding_box" -> {
        return boxOut(((TileShape) oct(args, "a", true)).boundingBox());
      }
      case "octagon.corner" -> {
        return ptOut(oct(args, "a", true).corner(args.get("no").getAsInt()));
      }
      case "octagon.enlarge" -> {
        return octOut(oct(args, "a", true).enlarge(Double.longBitsToDouble(args.get("offset_bits").getAsLong())));
      }
      case "octagon.compare_edge" -> {
        return strOut(
            sideOut(oct(args, "a", true).compare(oct(args, "b", true), args.get("edge").getAsInt())));
      }
      case "octagon.side_of_border_line" -> {
        return strOut(
            sideOut(
                oct(args, "a", true)
                    .sideOfBorderLine(
                        args.get("x").getAsInt(), args.get("y").getAsInt(), args.get("no").getAsInt())));
      }
      case "line.get_instance" -> {
        return lineOut(Line.getInstance(pt(args, "a"), Direction.getInstance(vec(args, "dir"))));
      }
      case "line.intersection" -> {
        return ptOut(line(args, "a").intersection(line(args, "b")));
      }
      case "line.intersection_approx" -> {
        return fptOut(line(args, "a").intersectionApprox(line(args, "b")));
      }
      case "line.side_of_point" -> {
        return strOut(sideOut(line(args, "a").sideOf(pt(args, "p"))));
      }
      case "line.side_of_intersection" -> {
        return strOut(sideOut(line(args, "a").sideOfIntersection(line(args, "l1"), line(args, "l2"))));
      }
      case "line.perpendicular_projection" -> {
        return ptOut(line(args, "a").perpendicularProjection(pt(args, "p")));
      }
      case "line.fast_equals" -> {
        return boolOut(line(args, "a").fastEquals(line(args, "b")));
      }
      case "line.compare_to" -> {
        return cmpOut(line(args, "a").compareTo(line(args, "b")));
      }
      case "line.direction" -> {
        return dirOut(line(args, "a").direction());
      }
      case "line.length" -> {
        JsonObject o = new JsonObject();
        o.addProperty("fbits", Float.floatToRawIntBits(line(args, "a").length()));
        return o;
      }
      case "line_segment.intersection" -> {
        Line[] cuts =
            new LineSegment(line(args, "s"), line(args, "m"), line(args, "e"))
                .intersection(new LineSegment(line(args, "s2"), line(args, "m2"), line(args, "e2")));
        JsonObject o = new JsonObject();
        o.addProperty("kind", "LineList");
        JsonArray arr = new JsonArray();
        for (Line l : cuts) {
          arr.add(lineOut(l));
        }
        o.add("v", arr);
        return o;
      }
      case "line_segment.bounding_box" -> {
        return boxOut(new LineSegment(line(args, "s"), line(args, "m"), line(args, "e")).boundingBox());
      }
      case "line_segment.sort_endpoints_in_xy" -> {
        return segOut(
            new LineSegment(line(args, "s"), line(args, "m"), line(args, "e")).sortEndpointsInXY());
      }
      case "tileshape.from_8_ints" -> {
        JsonArray v = args.getAsJsonArray("v");
        return octOut(
            TileShape.getInstance(
                v.get(0).getAsInt(),
                v.get(1).getAsInt(),
                v.get(2).getAsInt(),
                v.get(3).getAsInt(),
                v.get(4).getAsInt(),
                v.get(5).getAsInt(),
                v.get(6).getAsInt(),
                v.get(7).getAsInt()));
      }
      case "tileshape.intersection_box_oct" -> {
        return tileOut(box(args, "box").intersection((TileShape) oct(args, "oct", true)));
      }
      case "tileshape.from_points" -> {
        return tileOut(TileShape.getInstance(points(args, "points")));
      }
      // No `simplex.remove_redundant` op: removeRedundantLines() is
      // package-private (Simplex.java:884); pruning is covered transitively
      // by Simplex.getInstance (:46) and intersection (:629).
      case "simplex.get_instance" -> {
        return simplexOut(Simplex.getInstance(lines(args, "lines")));
      }
      case "simplex.intersection" -> {
        return simplexOut(
            Simplex.getInstance(lines(args, "lines"))
                .intersection(Simplex.getInstance(lines(args, "lines2"))));
      }
      case "polyline.ctor" -> {
        return polyOut(new Polyline(lines(args, "lines")));
      }
      case "polyline.bounding_box" -> {
        return boxOut(new Polyline(lines(args, "lines")).boundingBox());
      }
      case "polyline.length_approx" -> {
        return dblOut(new Polyline(lines(args, "lines")).lengthApprox());
      }
      case "polyline.offset_shape" -> {
        TileShape t =
            new Polyline(lines(args, "lines"))
                .offsetShape(args.get("half_width").getAsInt(), args.get("no").getAsInt());
        if (t == null) {
          JsonObject o = new JsonObject();
          o.addProperty("kind", "Null");
          return o;
        }
        return tileOut(t);
      }
      case "polyline.is_multiple_of_45_degree" -> {
        return boolOut(new Polyline(lines(args, "lines")).isMultipleOf45Degree());
      }
      case "circle.intersects_octagon" -> {
        return boolOut(
            new Circle(pt(args, "center"), args.get("radius").getAsInt())
                .intersects(oct(args, "oct", true)));
      }
      case "circle.bounding_octagon" -> {
        return octOut(new Circle(pt(args, "center"), args.get("radius").getAsInt()).boundingOctagon());
      }
      case "floatpoint.round" -> {
        return ptOut(fpt(args, "p").round());
      }
      case "floatpoint.round_to_grid" -> {
        return ptOut(
            fpt(args, "p")
                .roundToGrid(args.get("h").getAsInt(), args.get("v").getAsInt()));
      }
      case "floatpoint.distance_square" -> {
        return dblOut(fpt(args, "a").distanceSquare(fpt(args, "b")));
      }
      case "floatpoint.inside_circle" -> {
        return boolOut(fpt(args, "p").insideCircle(fpt(args, "p1"), fpt(args, "p2"), fpt(args, "p3")));
      }
      case "floatline.intersection" -> {
        FloatPoint r = fline(args, "a").intersection(fline(args, "b"));
        if (r == null) {
          JsonObject o = new JsonObject();
          o.addProperty("kind", "Null");
          return o;
        }
        return fptOut(r);
      }
      case "floatline.projection" -> {
        return fptOut(fline(args, "a").perpendicularProjection(fpt(args, "p")));
      }
      default -> throw new UnknownOpException(op);
    }
  }

  public static void main(String[] p_args) throws Exception {
    if (p_args.length < 1) {
      System.err.println("usage: GeometryCorpusOracle <cases.jsonl>");
      System.exit(2);
    }
    BufferedReader in =
        new BufferedReader(
            new InputStreamReader(Files.newInputStream(Paths.get(p_args[0])), StandardCharsets.UTF_8));
    BufferedWriter out =
        new BufferedWriter(new OutputStreamWriter(System.out, StandardCharsets.UTF_8));
    String line;
    while ((line = in.readLine()) != null) {
      line = line.trim();
      if (line.isEmpty()) {
        continue;
      }
      JsonObject caseObj = JsonParser.parseString(line).getAsJsonObject();
      String id = caseObj.get("id").getAsString();
      String op = caseObj.get("op").getAsString();
      JsonObject result = new JsonObject();
      result.addProperty("id", id);
      try {
        result.add("out", evaluateOp(op, caseObj.getAsJsonObject("args")));
      } catch (UnknownOpException e) {
        out.flush();
        System.err.println("unknown op for case id " + id + ": " + e.getMessage());
        System.exit(3);
      } catch (RuntimeException e) {
        JsonObject err = new JsonObject();
        err.addProperty("kind", "JavaException");
        err.addProperty("msg", e.toString());
        result.add("out", err);
      }
      out.write(result.toString());
      out.write("\n");
      // Flush per case: FRLogger.warn inside evaluateOp (e.g.
      // Polyline.offsetShape "no out of range") writes straight to
      // System.out; a lazily-buffered result line would interleave with
      // it and corrupt the JSONL stream. Flushed lines are atomic, and
      // the consumer drops non-`{"id"` lines (the warnings).
      out.flush();
    }
    out.flush();
  }
}
