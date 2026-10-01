// BoundsOracle.java — T8 differential oracle for the optimizer lower
// bounds (BoardStatisticsBoundsCalculator + BoardStatistics bounds).
// Run (repo root):
//   ~/.jdks/jdk-25.0.4.1+1/bin/java -cp build/libs/freerouting-current-executable.jar \
//       rust/harness/oracle/BoundsOracle.java <dsn> [netno]
// Lives OUTSIDE src/ — the frozen tree is never touched; only the built jar is
// consumed (the TraceTightenerProbe precedent).
//
// Prints, per net 1..maxNetNumber: the terminal count, each terminal's raw
// f64 x/y (Double.doubleToRawLongBits), the per-net MST manhattan length
// (raw bits) and bend count, and the via lower bound; then the accumulated
// totals (raw bits) and the final f32 casts (Float.floatToRawIntBits).
// Optional second arg: print only that net's line.
//
// WHY TRANSCRIBED: the real calculator is package-private in the jar
// (`final class BoardStatisticsBoundsCalculator`) — direct invocation is
// impossible — so the arithmetic below is transcribed verbatim from the
// frozen tree (reviewer-verified line-identical). The public alternative
// `new BoardStatistics(board).bounds` (BoardStatistics.java:94, wired at
// :228) exposes the TOTALS only, not the per-net walk-order face — which
// is the point of this probe.
//
// Serialization: doubles as {"bits":<i64>}; floats as {"fbits":<i32>} — the
// capture-tool-rounding lesson (cerebrum mode 5) — never through toString.

import app.freerouting.board.facade.BasicBoard;
import app.freerouting.io.specctra.DsnReader;
import app.freerouting.board.model.items.ConductionArea;
import app.freerouting.board.model.items.Item;
import app.freerouting.board.model.items.Pin;
import app.freerouting.board.model.structure.Unit;
import app.freerouting.geometry.planar.FloatPoint;
import app.freerouting.rules.Net;
import app.freerouting.rules.ViaInfo;
import app.freerouting.rules.ViaRule;

import java.io.ByteArrayInputStream;
import java.nio.file.Files;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.HashSet;
import java.util.List;
import java.util.Set;

public class BoundsOracle {

  public static void main(String[] args) throws Exception {
    byte[] bytes = Files.readAllBytes(Paths.get(args[0]));
    int onlyNet = args.length > 1 ? Integer.parseInt(args[1]) : -1;
    app.freerouting.io.BoardReadResult read =
        DsnReader.readBoard(
            new ByteArrayInputStream(bytes),
            null,
            null,
            Paths.get(args[0]).getFileName().toString());
    if (!(read instanceof app.freerouting.io.BoardReadResult.Success success)) {
      System.out.println("result= " + read);
      return;
    }
    BasicBoard board = success.board();

    double boardUnitToMmFactor =
        Unit.scale(1.0, board.communication.unit, Unit.MM)
            / (board.communication.resolution > 0 ? board.communication.resolution : 1);
    System.out.println(
        "factor_bits=" + Double.doubleToRawLongBits(boardUnitToMmFactor)
            + " unit=" + board.communication.unit
            + " resolution=" + board.communication.resolution);

    int maxNetNumber = board.rules.nets.maxNetNumber();
    double minTraceLength = 0.0;
    int minViaCount = 0;
    int minBendCount = 0;
    for (int netNumber = 1; netNumber <= maxNetNumber; netNumber++) {
      Net net = board.rules.nets.get(netNumber);
      if (net == null) {
        continue;
      }
      List<Terminal> terminals = getTerminals(board, net);
      if (terminals.size() < 2) {
        continue;
      }
      MstResult mst = calculateMst(terminals);
      int viaCount = calculateMinimumViaCount(net, terminals);
      minTraceLength += mst.length * boardUnitToMmFactor;
      minBendCount += mst.bendCount;
      minViaCount += viaCount;
      if (onlyNet < 0 || netNumber == onlyNet) {
        StringBuilder sb = new StringBuilder();
        sb.append("net ").append(netNumber)
            .append(" terms=").append(terminals.size())
            .append(" mst=").append(Double.doubleToRawLongBits(mst.length))
            .append(" bends=").append(mst.bendCount)
            .append(" vias=").append(viaCount)
            .append(" contrib=").append(Double.doubleToRawLongBits(mst.length * boardUnitToMmFactor));
        for (Terminal t : terminals) {
          sb.append(" T[")
              .append(Double.doubleToRawLongBits(t.x)).append(",")
              .append(Double.doubleToRawLongBits(t.y)).append(",")
              .append(t.signalLayers).append("]");
        }
        System.out.println(sb);
      }
    }
    System.out.println("sum_bits=" + Double.doubleToRawLongBits(minTraceLength));
    System.out.println("min_trace_length_mm=" + Float.floatToRawIntBits((float) minTraceLength));
    System.out.println("min_via_count=" + minViaCount);
    System.out.println("min_bend_count=" + minBendCount);
  }

  // --- the verbatim calculator walk (BoardStatisticsBoundsCalculator) ------

  record Terminal(double x, double y, Set<Integer> signalLayers) {}

  record MstResult(double length, int bendCount) {}

  record ViaSpan(int firstLayer, int lastLayer) {}

  static List<Terminal> getTerminals(BasicBoard board, Net net) {
    List<Terminal> result = new ArrayList<>();
    for (Item item : net.getTerminalItems()) {
      Terminal terminal = toTerminal(board, item);
      if (terminal != null && !terminal.signalLayers().isEmpty()) {
        result.add(terminal);
      }
    }
    return result;
  }

  static Terminal toTerminal(BasicBoard board, Item item) {
    double x;
    double y;
    Set<Integer> signalLayers = new HashSet<>();
    if (item instanceof Pin pin) {
      FloatPoint center = pin.getCenter().toFloat();
      x = center.x;
      y = center.y;
      for (int layer = pin.firstLayer(); layer <= pin.lastLayer(); layer++) {
        if (pin.getShape(layer - pin.firstLayer()) != null) {
          addSignalLayer(board, layer, signalLayers);
        }
      }
    } else if (item instanceof ConductionArea area) {
      FloatPoint center = area.getArea().getBorder().centreOfGravity();
      x = center.x;
      y = center.y;
      addSignalLayer(board, area.getLayer(), signalLayers);
    } else {
      return null;
    }
    return new Terminal(x, y, signalLayers);
  }

  static void addSignalLayer(BasicBoard board, int layer, Set<Integer> signalLayers) {
    if (layer >= 0
        && layer < board.layerStructure.layers.length
        && board.layerStructure.layers[layer].isSignal) {
      signalLayers.add(layer);
    }
  }

  static MstResult calculateMst(List<Terminal> terminals) {
    boolean[] used = new boolean[terminals.size()];
    double[] distances = new double[terminals.size()];
    int[] parents = new int[terminals.size()];
    Arrays.fill(distances, Double.POSITIVE_INFINITY);
    Arrays.fill(parents, -1);
    distances[0] = 0.0;
    double length = 0.0;
    int bendCount = 0;
    for (int edge = 0; edge < terminals.size(); edge++) {
      int current = -1;
      for (int index = 0; index < terminals.size(); index++) {
        if (!used[index] && (current < 0 || distances[index] < distances[current])) {
          current = index;
        }
      }
      if (current < 0) {
        break;
      }
      used[current] = true;
      if (parents[current] >= 0) {
        Terminal from = terminals.get(parents[current]);
        Terminal to = terminals.get(current);
        length += distances[current];
        if (from.x != to.x && from.y != to.y) {
          bendCount++;
        }
      }
      for (int index = 0; index < terminals.size(); index++) {
        if (!used[index]) {
          double distance = manhattanDistance(terminals.get(current), terminals.get(index));
          if (distance < distances[index]) {
            distances[index] = distance;
            parents[index] = current;
          }
        }
      }
    }
    return new MstResult(length, bendCount);
  }

  static double manhattanDistance(Terminal first, Terminal second) {
    return Math.abs(first.x - second.x) + Math.abs(first.y - second.y);
  }

  static int calculateMinimumViaCount(Net net, List<Terminal> terminals) {
    LayerGroups groups = new LayerGroups();
    for (Terminal terminal : terminals) {
      groups.addLayers(terminal.signalLayers());
    }
    if (groups.count() <= 1) {
      return 0;
    }
    ViaRule viaRule = net.getNetClass() != null ? net.getNetClass().getViaRule() : null;
    if (viaRule == null) {
      return 0;
    }
    Set<Integer> remainingGroups = groups.roots();
    List<ViaSpan> spans = new ArrayList<>();
    for (int index = 0; index < viaRule.viaCount(); index++) {
      ViaInfo viaInfo = viaRule.getVia(index);
      int firstLayer = viaInfo.getPadstack().fromLayer();
      int lastLayer = viaInfo.getPadstack().toLayer();
      spans.add(new ViaSpan(Math.min(firstLayer, lastLayer), Math.max(firstLayer, lastLayer)));
    }
    int viaCount = 0;
    while (!remainingGroups.isEmpty()) {
      ViaSpan bestSpan = null;
      Set<Integer> bestCoveredGroups = Set.of();
      for (ViaSpan span : spans) {
        Set<Integer> coveredGroups = groups.groupsCoveredBy(span);
        coveredGroups.retainAll(remainingGroups);
        if (coveredGroups.size() > bestCoveredGroups.size()) {
          bestSpan = span;
          bestCoveredGroups = coveredGroups;
        }
      }
      if (bestSpan == null || bestCoveredGroups.isEmpty()) {
        break;
      }
      remainingGroups.removeAll(bestCoveredGroups);
      viaCount++;
    }
    return viaCount;
  }

  static final class LayerGroups {
    private final java.util.Map<Integer, Integer> parent = new java.util.HashMap<>();

    void addLayers(java.util.Collection<Integer> layers) {
      Integer firstLayer = null;
      for (int layer : layers) {
        parent.putIfAbsent(layer, layer);
        if (firstLayer == null) {
          firstLayer = layer;
        } else {
          union(firstLayer, layer);
        }
      }
    }

    int count() {
      return roots().size();
    }

    Set<Integer> roots() {
      Set<Integer> roots = new HashSet<>();
      for (int layer : parent.keySet()) {
        roots.add(find(layer));
      }
      return roots;
    }

    Set<Integer> groupsCoveredBy(ViaSpan span) {
      Set<Integer> result = new HashSet<>();
      for (int layer : parent.keySet()) {
        if (layer >= span.firstLayer && layer <= span.lastLayer) {
          result.add(find(layer));
        }
      }
      return result;
    }

    private int find(int layer) {
      int currentParent = parent.get(layer);
      if (currentParent != layer) {
        currentParent = find(currentParent);
        parent.put(layer, currentParent);
      }
      return currentParent;
    }

    private void union(int firstLayer, int secondLayer) {
      int firstRoot = find(firstLayer);
      int secondRoot = find(secondLayer);
      if (firstRoot != secondRoot) {
        parent.put(secondRoot, firstRoot);
      }
    }
  }
}
