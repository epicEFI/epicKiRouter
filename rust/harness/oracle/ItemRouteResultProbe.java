// ItemRouteResultProbe.java — M4-T9 quality-review MIN-1 probe: what does
// the FROZEN jar actually answer for `improvementPercentage` on the
// quality-review ladder world (via 2→1, length 100→100)? The port's
// `improvement_percentage` computes the via ratio as FLOAT division; the
// quoted Java (`ItemRouteResult.java:58-65`) divides two `int` fields,
// which in Java is INTEGER division (1/2 == 0) promoted only afterwards —
// Java face 0.5 vs port 0.25. Run (repo root):
//   ~/.jdks/jdk-25.0.4.1+1/bin/java -cp build/libs/freerouting-current-executable.jar \
//       rust/harness/oracle/ItemRouteResultProbe.java
// Lives OUTSIDE src/ — the frozen tree is never touched; only the built
// jar is consumed (the BoundsOracle precedent).
//
// Prints, per world: the constructor arguments and the jar's
// `improvementPercentage()` to full float precision (Float.toString —
// exact shortest representation, no capture-tool rounding).
// Worlds: the ladder pin's world, its exact-ratio control, the
// via-equal/length-equal zero face, the via-INCREASING incomplete-rung
// winner (the `route_improved == 0.0` drop-face world from the review),
// and a non-exact length ratio for the record.

import app.freerouting.autoroute.ItemRouteResult;

public class ItemRouteResultProbe {

  private static void row(String name, int viaBefore, int viaAfter, double lenBefore, double lenAfter, int incBefore, int incAfter) {
    ItemRouteResult r =
        new ItemRouteResult(7, viaBefore, viaAfter, lenBefore, lenAfter, incBefore, incAfter);
    System.out.println(
        name
            + " via "
            + viaBefore
            + "->"
            + viaAfter
            + " len "
            + lenBefore
            + "->"
            + lenAfter
            + " inc "
            + incBefore
            + "->"
            + incAfter
            + " : improved="
            + r.improved()
            + " pct="
            + r.improvementPercentage());
  }

  public static void main(String[] args) {
    row("W1_ladder_world ", 2, 1, 100.0, 100.0, 0, 0);
    row("W2_exact_ratio  ", 4, 2, 100.0, 100.0, 0, 0);
    row("W3_equal_equal  ", 2, 2, 100.0, 100.0, 0, 0);
    row("W4_via_increases", 3, 2, 100.0, 100.0, 1, 0);
    row("W5_len_ratio    ", 2, 2, 100.0, 50.0, 0, 0);
    row("W6_zero_baseline", 0, 0, 0.0, 0.0, 0, 0);
  }
}
