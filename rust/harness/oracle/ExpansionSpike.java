// ExpansionSpike.java — the M3-T4 jar-side probe oracle for the
// expansion-graph port (epic-index completeShape + epic-router
// rooms/doors/neighbours). DrcOracle house pattern: ONE JVM per run,
// deterministic JSONL rows on stdout (lines starting with "{" are the
// results; the jar's FRLogger noise is interleaved and dropped).
//
// Build/run (JDK 25, from the repo root):
//   mkdir -p /tmp/epic-expansion-classes && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/javac \
//       -cp build/libs/freerouting-current-executable.jar \
//       -d /tmp/epic-expansion-classes rust/harness/oracle/ExpansionSpike.java && \
//   ~/.jdks/jdk-25.0.4.1+1/bin/java \
//       -Duser.language=en -Duser.country=US \
//       -cp build/libs/freerouting-current-executable.jar:/tmp/epic-expansion-classes \
//       app.freerouting.autoroute.expansion.ExpansionSpike <board.dsn>
//
// The oracle declares the app.freerouting.autoroute.expansion package so
// the package-private SortedRoomNeighbours.selectCalculationMode is
// directly callable (the IndexOracle package-declaration precedent).
// Everything else it touches is public API.
//
// Determinism: rows are emitted from ArrayList/LinkedList/TreeSet orders
// only; every double is printed with Double.toString (exact round-trip);
// no HashSet/HashMap iteration reaches a row.
package app.freerouting.autoroute.expansion;

import app.freerouting.autoroute.maze.AutorouteEngine;
import app.freerouting.board.actions.ItemIdGenerator;
import app.freerouting.board.facade.BasicBoard;
import app.freerouting.board.facade.RoutingBoard;
import app.freerouting.board.model.items.Item;
import app.freerouting.board.searchtree.ShapeSearchTree;
import app.freerouting.board.searchtree.ShapeSearchTree45Degree;
import app.freerouting.board.searchtree.ShapeSearchTree90Degree;
import app.freerouting.geometry.planar.FloatLine;
import app.freerouting.geometry.planar.IntBox;
import app.freerouting.geometry.planar.IntOctagon;
import app.freerouting.geometry.planar.IntPoint;
import app.freerouting.geometry.planar.Line;
import app.freerouting.geometry.planar.Shape;
import app.freerouting.geometry.planar.Simplex;
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
import java.util.Comparator;
import java.util.List;

public class ExpansionSpike {

  private static final Gson GSON = new Gson();

  /** Octagon row: the 8 coordinates in Java field order. */
  private static JsonArray oct(IntOctagon o) {
    JsonArray arr = new JsonArray(8);
    arr.add(o.leftX);
    arr.add(o.bottomY);
    arr.add(o.rightX);
    arr.add(o.topY);
    arr.add(o.upperLeftDiagonalX);
    arr.add(o.lowerRightDiagonalX);
    arr.add(o.lowerLeftDiagonalX);
    arr.add(o.upperRightDiagonalX);
    return arr;
  }

  /** Bounding-box row of a tile shape (null shape → JsonNull). */
  private static JsonArray bounds(TileShape shape) {
    JsonArray arr = new JsonArray(4);
    if (shape == null) {
      arr.add((String) "null");
      arr.add((String) "null");
      arr.add((String) "null");
      arr.add((String) "null");
      return arr;
    }
    IntBox bb = shape.boundingBox();
    arr.add(bb.ll.x);
    arr.add(bb.ll.y);
    arr.add(bb.ur.x);
    arr.add(bb.ur.y);
    return arr;
  }

  /**
   * Class-aware shape emission: every shape row carries shape_class plus
   * the coordinates in the class's native form (octagon: 8 coords in Java
   * field order; box: ll/ur; segment: a/b; point: x/y; anything else:
   * bounding box). No casts — the base completeShape arm returns
   * Simplex-shaped rooms and door shapes can be LineSegment/Point.
   */
  private static void addShape(JsonObject row, String key, Shape shape) {
    if (shape == null) {
      row.addProperty(key + "_class", "null");
      return;
    }
    row.addProperty(key + "_class", shape.getClass().getSimpleName());
    if (shape instanceof IntOctagon o) {
      row.add(key, oct(o));
    } else if (shape instanceof IntBox b) {
      row.add(key, bounds(b));
    } else {
      row.add(key, bounds(shape.boundingBox()));
    }
  }

  private static void emit(JsonObject row) {
    System.out.println(GSON.toJson(row));
  }

  public static void main(String[] p_args) throws Exception {
    if (p_args.length < 1) {
      System.out.println("{\"row\":\"usage-error\"}");
      System.exit(1);
    }
    byte[] bytes;
    try {
      bytes = Files.readAllBytes(Paths.get(p_args[0]));
    } catch (Exception e) {
      System.out.println("{\"row\":\"read-error\"}");
      System.exit(2);
      return;
    }

    BoardReadResult read;
    try {
      read =
          DsnReader.readBoard(
              new ByteArrayInputStream(bytes),
              null,
              new ItemIdGenerator(),
              Paths.get(p_args[0]).getFileName().toString());
    } catch (Throwable t) {
      System.out.println("{\"row\":\"parse-error\"}");
      System.exit(3);
      return;
    }
    if (!(read instanceof BoardReadResult.Success success)) {
      System.out.println("{\"row\":\"read-not-success\"}");
      System.exit(3);
      return;
    }
    BasicBoard basicBoard = success.board();
    RoutingBoard board = (RoutingBoard) basicBoard;

    // Uniform fill normalization (the index-corpus discipline): Java's
    // read leaves trees partially filled; reinsert makes the tree state
    // deterministic and matches the Rust rebuild path.
    board.searchTreeManager.reinsertTreeItems();

    JsonObject meta = new JsonObject();
    meta.addProperty("row", "meta");
    ShapeSearchTree tree = board.searchTreeManager.getAutorouteTree(0);
    meta.addProperty("tree_class", tree.getClass().getSimpleName());
    IntBox boardBox = board.getBoundingBox();
    JsonArray bb = new JsonArray(4);
    bb.add(boardBox.ll.x);
    bb.add(boardBox.ll.y);
    bb.add(boardBox.ur.x);
    bb.add(boardBox.ur.y);
    meta.add("board_bounds", bb);
    java.util.Collection<app.freerouting.rules.Net> netAList = board.rules.nets.get("NET_A");
    if (netAList.isEmpty()) {
      System.out.println("{\"row\":\"net-not-found\"}");
      System.exit(4);
      return;
    }
    int netA = netAList.iterator().next().netNumber;
    meta.addProperty("net_a", netA);
    meta.addProperty("max_net_no", board.rules.nets.maxNetNumber());
    emit(meta);

    // Per-net class assignment: the dispatch reality check — default-class
    // nets resolve to the plain tree, non-default-class nets create a tree
    // by angle restriction (45° boards: ShapeSearchTree45Degree).
    List<app.freerouting.rules.Net> nets = new ArrayList<>();
    for (int i = 1; i <= board.rules.nets.maxNetNumber(); i++) {
      app.freerouting.rules.Net net = board.rules.nets.get(i);
      if (net != null) {
        nets.add(net);
      }
    }
    nets.sort(Comparator.comparingInt(n -> n.netNumber));
    for (app.freerouting.rules.Net net : nets) {
      JsonObject row = new JsonObject();
      row.addProperty("row", "net");
      row.addProperty("net_number", net.netNumber);
      row.addProperty("name", net.name);
      row.addProperty("class_name", net.getNetClass().getName());
      row.addProperty("trace_clearance_class", net.getNetClass().getTraceClearanceClass());
      emit(row);
    }

    // Items sorted by id (deterministic): the sort-pin's id universe.
    List<Item> items = new ArrayList<>(board.getItems());
    items.sort(Comparator.comparingInt(Item::getId));
    for (Item item : items) {
      JsonObject row = new JsonObject();
      row.addProperty("row", "item");
      row.addProperty("id", item.getId());
      row.addProperty("class", item.getClass().getSimpleName());
      row.addProperty("shape_count", item.tileShapeCount());
      row.addProperty("routable", item.isRoutable());
      emit(row);
    }

    // The class-1 tree: getAutorouteTree(1) on a 45° board creates a
    // ShapeSearchTree45Degree (SearchTreeManager.java:140-175). This is
    // the production arm for non-default-clearance-class nets.
    ShapeSearchTree tree45 = board.searchTreeManager.getAutorouteTree(1);

    // Per-object tree shapes for BOTH trees: exactly the shapes
    // completeShape's obstacle walk consumes (getTreeShape includes the
    // compensation for the 45° tree's class-1 index). The Rust port's
    // pins re-insert these shapes verbatim, so the restrain sequence
    // sees an identical obstacle set.
    for (String tag : new String[] {"plain", "fortyfive"}) {
      ShapeSearchTree dumpTree = tag.equals("plain") ? tree : tree45;
      for (Item item : items) {
        int count = item.tileShapeCount();
        for (int i = 0; i < count; i++) {
          TileShape shape = item.getTreeShape(dumpTree, i);
          JsonObject row = new JsonObject();
          row.addProperty("row", "tree_shape");
          row.addProperty("tree", tag);
          row.addProperty("item_id", item.getId());
          row.addProperty("shape_index", i);
          addShape(row, "shape", shape);
          emit(row);
        }
      }
    }

    // Dispatch rows: the mode selected for each tree class. tree is the
    // plain default tree (class 0), tree45 the 45° tree (class 1); the
    // 90-deg tree is constructed here only as a dispatch witness.
    JsonObject disp = new JsonObject();
    disp.addProperty("row", "dispatch");
    disp.addProperty("tree0_class", tree.getClass().getSimpleName());
    disp.addProperty("tree0_mode", SortedRoomNeighbours.selectCalculationMode(tree).name());
    disp.addProperty("tree45_class", tree45.getClass().getSimpleName());
    disp.addProperty("tree45_mode", SortedRoomNeighbours.selectCalculationMode(tree45).name());
    disp.addProperty(
        "orthogonal",
        SortedRoomNeighbours
            .selectCalculationMode(new ShapeSearchTree90Degree(board, 0))
            .name());
    emit(disp);

    // ---- Section S: completeShape probes on the raw tree ----
    TileShape boardShape = new IntBox(boardBox.ll, boardBox.ur);
    // Octagon room shape for the 45-degree arm: the override requires the
    // ROOM shape (not the contained shape) to be an IntOctagon
    // (ShapeSearchTree45Degree.completeShape startShape guard); the S5
    // probe below pins that guard's empty result.
    IntOctagon boardOct = boardShape.boundingOctagon();

    // S1 corridor seed: octagon contained shape between the keepouts
    // (board database units = DSN units x 10 at resolution um 10).
    runCompleteShape(
        "S1_corridor",
        tree,
        board,
        boardShape,
        new IntBox(new IntPoint(450000, 250000), new IntPoint(550000, 350000)),
        netA);

    // S2 null contained shape → the empty guard arm.
    JsonObject s2 = new JsonObject();
    s2.addProperty("row", "complete_shape");
    s2.addProperty("probe", "S2_null_contained");
    s2.addProperty("result_count", tree.completeShape(
        new IncompleteFreeSpaceExpansionRoom(boardShape, 0, null), netA, null, null).size());
    emit(s2);

    // S3 simplex contained shape → the non-IntOctagon arm (bounding
    // octagon approximation): a triangle above the corridor seed.
    IntPoint t0 = new IntPoint(490000, 360000);
    IntPoint t1 = new IntPoint(510000, 360000);
    IntPoint t2 = new IntPoint(500000, 380000);
    Simplex tri = Simplex.getInstance(new Line[] {new Line(t0, t1), new Line(t1, t2), new Line(t2, t0)});
    runCompleteShape("S3_simplex", tree, board, boardShape, tri, netA);

    // S4 empty 45° tree → the root==null guard arm.
    ShapeSearchTree emptyTree = new ShapeSearchTree45Degree(board, 0);
    JsonObject s4 = new JsonObject();
    s4.addProperty("row", "complete_shape");
    s4.addProperty("probe", "S4_empty_tree");
    int s4count =
        emptyTree
            .completeShape(
                new IncompleteFreeSpaceExpansionRoom(
                    boardShape,
                    0,
                    new IntBox(new IntPoint(450000, 250000), new IntPoint(550000, 350000))),
                netA,
                null,
                null)
            .size();
    s4.addProperty("result_count", s4count);
    emit(s4);

    // (No empty-plain-tree probe: the plain ShapeSearchTree ctor taking
    // bounding directions is package-private; the base arm's root==null
    // guard is identical in structure to the 45° one and trivial.)

    // ---- Section E: the engine flow (room completion + doors) ----
    AutorouteEngine engine = new AutorouteEngine(board, 0, true);
    engine.initConnection(netA, null, null);

    IncompleteFreeSpaceExpansionRoom seed =
        engine.addIncompleteExpansionRoom(
            boardShape,
            0,
            new IntBox(new IntPoint(450000, 250000), new IntPoint(550000, 350000)));

    // Completion round 1.
    java.util.Collection<CompleteFreeSpaceExpansionRoom> completed1 =
        engine.completeExpansionRoom(seed);
    dumpEngineState("E1_after_first", engine, completed1);

    // Completion round 2: complete the first remaining incomplete room
    // (exercises the fromDoorShape/ignoreObject arm inside
    // completeExpansionRoom, because after round 1 the seed room has
    // doors to completed free-space neighbours).
    IncompleteFreeSpaceExpansionRoom second = engine.getFirstIncompleteExpansionRoom();
    if (second != null) {
      java.util.Collection<CompleteFreeSpaceExpansionRoom> completed2 =
          engine.completeExpansionRoom(second);
      dumpEngineState("E2_after_second", engine, completed2);
    }

    // ---- Section T: the 45°-tree battery (NET_B, clearance class 1) ----
    java.util.Collection<app.freerouting.rules.Net> netBList = board.rules.nets.get("NET_B");
    if (netBList.isEmpty()) {
      System.out.println("{\"row\":\"netb-not-found\"}");
      System.exit(4);
      return;
    }
    app.freerouting.rules.Net netB = netBList.iterator().next();
    int netBNo = netB.netNumber;
    int classB = netB.getNetClass().getTraceClearanceClass();

    // T-probes on the 45° tree (same seed shapes as the S battery — the
    // arm contrast is the witness: Simplex rooms here would falsify the
    // dispatch model). The room shape is the octagon board shape — the
    // 45° override requires it (S5 pins the guard).
    JsonObject s5 = new JsonObject();
    s5.addProperty("row", "complete_shape");
    s5.addProperty("probe", "S5_room_shape_not_octagon");
    s5.addProperty("result_count", tree45.completeShape(
        new IncompleteFreeSpaceExpansionRoom(boardShape, 0,
            new IntBox(new IntPoint(450000, 250000), new IntPoint(550000, 350000))),
        netBNo, null, null).size());
    emit(s5);
    runCompleteShape(
        "T1_corridor", tree45, board, boardOct,
        new IntBox(new IntPoint(450000, 250000), new IntPoint(550000, 350000)), netBNo);
    runCompleteShape(
        "T3_simplex", tree45, board, boardOct, tri, netBNo);

    // The engine flow on the 45° tree: seed in the free region above the
    // left keepout (region-separated from the E battery to rule out
    // cross-engine interference).
    AutorouteEngine engine45 = new AutorouteEngine(board, classB, true);
    engine45.initConnection(netBNo, null, null);
    IncompleteFreeSpaceExpansionRoom seed45 =
        engine45.addIncompleteExpansionRoom(
            boardOct,
            0,
            new IntBox(new IntPoint(200000, 450000), new IntPoint(300000, 550000)));
    java.util.Collection<CompleteFreeSpaceExpansionRoom> completedF1 =
        engine45.completeExpansionRoom(seed45);
    dumpEngineState("F1_after_first", engine45, completedF1);
    IncompleteFreeSpaceExpansionRoom second45 = engine45.getFirstIncompleteExpansionRoom();
    if (second45 != null) {
      java.util.Collection<CompleteFreeSpaceExpansionRoom> completedF2 =
          engine45.completeExpansionRoom(second45);
      dumpEngineState("F2_after_second", engine45, completedF2);
    }
    System.exit(0);
  }

  /** Runs one completeShape probe and emits the result-room rows. */
  private static void runCompleteShape(
      String probe,
      ShapeSearchTree tree,
      RoutingBoard board,
      TileShape roomShape,
      TileShape contained,
      int netA) {
    JsonObject head = new JsonObject();
    head.addProperty("row", "complete_shape");
    head.addProperty("probe", probe);
    head.add("contained_bounds", bounds(contained));
    emit(head);
    java.util.Collection<IncompleteFreeSpaceExpansionRoom> result =
        tree.completeShape(
            new IncompleteFreeSpaceExpansionRoom(roomShape, 0, contained), netA, null, null);
    int idx = 0;
    for (IncompleteFreeSpaceExpansionRoom room : result) {
      JsonObject row = new JsonObject();
      row.addProperty("row", "complete_shape_room");
      row.addProperty("probe", probe);
      row.addProperty("index", idx++);
      row.addProperty("shape_class", room.getShape().getClass().getSimpleName());
      row.addProperty(
          "contained_class", room.getContainedShape().getClass().getSimpleName());
      if (room.getShape() instanceof IntOctagon so) {
        row.add("shape", oct(so));
      } else {
        row.add("shape_bounds", bounds(room.getShape()));
      }
      if (room.getContainedShape() instanceof IntOctagon co) {
        row.add("contained", oct(co));
      } else {
        row.add("contained_bounds", bounds(room.getContainedShape()));
      }
      row.addProperty("layer", room.getLayer());
      emit(row);
    }
  }

  /** Emits the whole engine state: rooms, doors, door sections, target doors. */
  private static void dumpEngineState(
      String tag,
      AutorouteEngine engine,
      java.util.Collection<CompleteFreeSpaceExpansionRoom> justCompleted) {
    JsonObject head = new JsonObject();
    head.addProperty("row", "engine_state");
    head.addProperty("tag", tag);
    head.addProperty("net", engine.getNetNumber());
    emit(head);

    for (CompleteFreeSpaceExpansionRoom room : justCompleted) {
      JsonObject row = new JsonObject();
      row.addProperty("row", "completed_room");
      row.addProperty("tag", tag);
      row.addProperty("room_id", room.getId());
      addShape(row, "shape", room.getShape());
      row.addProperty("layer", room.getLayer());
      emit(row);
    }

    // The engine's incomplete rooms in list order (append order = the
    // neighbour-walk order of the sorter). The list field is private
    // with no full-list getter — reflection it is (ArrayList order is
    // deterministic).
    List<IncompleteFreeSpaceExpansionRoom> incomplete;
    try {
      Field field = AutorouteEngine.class.getDeclaredField("incompleteExpansionRooms");
      field.setAccessible(true);
      @SuppressWarnings("unchecked")
      List<IncompleteFreeSpaceExpansionRoom> list =
          (List<IncompleteFreeSpaceExpansionRoom>) field.get(engine);
      incomplete = list == null ? new ArrayList<>() : list;
    } catch (ReflectiveOperationException e) {
      JsonObject row = new JsonObject();
      row.addProperty("row", "reflection-error");
      row.addProperty("tag", tag);
      emit(row);
      incomplete = new ArrayList<>();
    }
    int incIdx = 0;
    for (IncompleteFreeSpaceExpansionRoom room : incomplete) {
      JsonObject row = new JsonObject();
      row.addProperty("row", "incomplete_room");
      row.addProperty("tag", tag);
      row.addProperty("index", incIdx++);
      row.addProperty("room_id", room.getId());
      addShape(row, "shape", room.getShape());
      row.addProperty("layer", room.getLayer());
      emit(row);
    }

    // Completed rooms with doors. The door list order is the sorter's
    // neighbour-processing order (doors are appended per entry).
    for (CompleteFreeSpaceExpansionRoom room : justCompleted) {
      int doorIdx = 0;
      for (ExpansionDoor door : room.getDoors()) {
        JsonObject row = new JsonObject();
        row.addProperty("row", "door");
        row.addProperty("tag", tag);
        row.addProperty("room_id", room.getId());
        row.addProperty("door_index", doorIdx++);
        ExpansionRoom other = door.otherRoom(room);
        row.addProperty("other_id", other == null ? -1 : other.getId());
        row.addProperty(
            "other_class", other == null ? "null" : other.getClass().getSimpleName());
        row.addProperty("dimension", door.dimension);
        addShape(row, "shape", door.getShape());
        emit(row);

        // Door sections at two half widths (offsets in units; the
        // engine's TRACE_WIDTH_TOLERANCE = 2 is added inside).
        for (double halfWidth : new double[] {100.0, 2000.0}) {
          JsonObject srow = new JsonObject();
          srow.addProperty("row", "door_sections");
          srow.addProperty("tag", tag);
          srow.addProperty("room_id", room.getId());
          srow.addProperty("door_index", doorIdx - 1);
          srow.addProperty("half_width", halfWidth);
          JsonArray segs = new JsonArray();
          for (FloatLine seg : door.getSectionSegments(halfWidth)) {
            JsonArray segArr = new JsonArray(4);
            segArr.add(seg.a.x);
            segArr.add(seg.a.y);
            segArr.add(seg.b.x);
            segArr.add(seg.b.y);
            segs.add(segArr);
          }
          srow.add("sections", segs);
          emit(srow);
        }
      }

      // Target doors (own-net connectable items overlapping the room).
      int targetIdx = 0;
      for (TargetItemExpansionDoor tdoor : room.getTargetDoors()) {
        JsonObject row = new JsonObject();
        row.addProperty("row", "target_door");
        row.addProperty("tag", tag);
        row.addProperty("room_id", room.getId());
        row.addProperty("index", targetIdx++);
        row.addProperty("item_id", tdoor.item.getId());
        addShape(row, "shape", tdoor.getShape());
        emit(row);
      }
    }
  }
}
