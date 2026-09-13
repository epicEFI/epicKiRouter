# EpicRouter M1a: Geometry Kernel Port + Differential Corpus — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Port the Java `geometry.planar` kernel (34 classes, ~11.8k LOC) to `epic-geometry` with bit-level parity, proven by a differential corpus: Rust generates seeded random shape operations, the frozen Java jar evaluates them as the oracle, and every result must match exactly.

**Architecture:** Transliteration, not reinvention — every module mirrors one Java class with identical representation, identical operation order, and identical arithmetic widths (i32 with Java wraparound semantics, `double` where Java uses double, `num-bigint` where Java uses BigInteger). Parity is enforced by a three-command corpus flow added to `epic-harness` (`corpus generate|golden|compare`) that drives a ~150-line Java evaluator living **outside** the frozen `src/` tree via the fat jar's classpath. Optimization (i128/checked arithmetic) is explicitly deferred to M5 — this plan amends design §4.1's "i128 fast path" wording accordingly (see Locked Decisions).

**Tech Stack:** Rust 1.93 (workspace), `num-bigint = "0.4"` (new dep, BigInteger fallback only), existing `epic-harness` (serde_json already present). Java side: single-file source launcher (`java -cp <jar> File.java`), Gson already inside the jar.

**Design doc:** `docs/superpowers/specs/2026-09-11-epicrouter-rust-rewrite-design.md` (§4.1 layout, §5 parity gates, §6 M1 row, §9 risk register rows 1-3).

**M1 split note:** M1 = M1a (this plan: `epic-geometry` + corpus) followed by M1b (`epic-dsn` + fixture parse parity + SES round-trip, separate plan). Both must be green for the M1 exit criteria.

**Standing constraints (every task):**
- Never modify Java sources under `src/` (frozen oracle at `e7f9bdf1`) or anything outside this repo; never touch other Claude Code instances or global `~/.claude`.
- Never push to `upstream`. Commits land on `epic/main`, trailer `Co-Authored-By: Claude Code <noreply@anthropic.com>`.
- The corpus Java evaluator lives at `rust/harness/oracle/GeometryCorpusOracle.java` — inside `rust/`, never `src/`.
- Keep `cargo clippy --workspace --all-targets -- -D warnings` green (incl. `clippy::unwrap_used`; tests use `.expect()`).

---

## Ground truth from recon (established 2026-09-12; do not re-derive)

### The exact-arithmetic ladder (why parity means i32+double, not i128)

1. **`int` coordinates everywhere.** `IntPoint` ctor checks `|x| > CRIT_INT` only to LOG (IntPoint.java:20-25) — soft guard. Java int ops wrap silently (two's complement); **callers rely on it** (e.g. `Line.length()` at Line.java:561-566 computes `dx*dx + dy*dy` in int, overflowing for |dx| > ~46341 — a Rust `i32` port with `wrapping_*` reproduces this exactly; an i64 port diverges).
2. **`long` only in `IntPoint.determinant` / `IntVector.determinant`** (`(long)x*oy - (long)y*ox` — IntPoint.java:131, IntVector.java:64).
3. **`double` as an EXACT predicate** in `IntVector.sideOf` (IntVector.java:136), `IntVector.projection` (:158), `IntDirection.compareTo/determinant` (IntDirection.java:76,116), `Line.compareTo` (Line.java:469), `Line.fastEquals` (:91). Exact only because the DSN reader clamps coordinates to ≲ CRIT_INT/5 (Structure.java:1189-1201). Port double-for-double.
4. **`BigInteger`** in `Line.intersection` general formula, `IntPoint.perpendicularProjection`, RationalPoint/RationalVector, `BigIntAux` (determinant, `addRationalCoordinates` — adds by product of denominators, NEVER gcd-reduced; binaryGcd is Java-math's private gcd clone).

`Limits`: `CRIT_INT = 2^25 = 33_554_432`, `CRIT_DOUBLE = 2^53`, `CRIT_INT_BIG`, `sqrt2`.

### Parity traps (each has a pinned test or corpus case)

| # | Trap | Java behavior to reproduce |
|---|---|---|
| T1 | `Math.round(double)` | Closest long with ties toward +∞ (-2.5 → -2), **implemented bit-exactly per JDK 7+ (JDK-6430675)** — NOT the naive `floor(x + 0.5)`, which mis-rounds values 1 ulp below a half (`Math.round(0.49999999999999994) == 0`). Rust `f64::round` also differs (rounds halves away from zero). Affects `IntOctagon.offset` (:299-303), `IntBox.offset` (:447), `FloatPoint.round` (:103). `Math.rint` (half-to-even) ONLY in `FloatPoint.roundToGrid` (:141,147) |
| T2 | `IntOctagon.isEmpty()` | **Reference identity** `this == EMPTY` (IntOctagon.java:92-94) — not field comparison. Rust: value-compare against the sentinel field tuple; equivalent on all canonical outputs (EMPTY is only produced by ops that return the constant), and the corpus only tests emptiness through those ops |
| T3 | `IntBox.isEmpty()` | Value-based (`ll.x>ur.x ‖ ll.y>ur.y`); `EMPTY` = inverted sentinel box. Asymmetric with octagon — keep both behaviors |
| T4 | `IntDirection.turn45Degree(negative)` | Java `%` keeps sign → negative `n` hits default case → returns `IntDirection(0,0)` (NULL). `turn90Degree` normalizes negatives by while-loops. Asymmetry is load-bearing |
| T5 | `RationalPoint` non-canonical | Ctor does NOT normalize; equality is cross-multiplication (`determinant`), never structural; `getInstance(x,y,z)` reduces only if `x.mod(z)==0 && y.mod(z)==0` (single-divisor test, not gcd); `hashCode` reduces by gcd but `getId()` does not |
| T6 | `Line.intersection` | 16 special-cased 45°/orthogonal fast paths (Line.java:218-304) BEFORE the general BigInteger formula; fast-path branch order is observable |
| T7 | `IntOctagon.normalize()` | Fixed 12-step tightening sequence (IntOctagon.java:399-541) using `Math.ceil(x/2.0)`/`Math.floor(x/2.0)` — step order and the double div-2 rounding are observable on degenerate slivers |
| T8 | `Line.sideOfIntersection` | tolerance-1.0 float check FIRST, exact re-check only on COLLINEAR (Line.java:152-164) |
| T9 | `Direction.equals` | collinear side + POSITIVE projection (hand-written; opposite directions unequal for `Line.equals` but equal for `isEqualOrOpposite`) |
| T10 | `Polyline` ctor | direction flipping decided by `sideOf` of FLOAT corner approximations; then `removeConsecutiveParallelLines` then `removeOverlaps` — order observable |
| T11 | Empty octagon sentinel | `EMPTY` = all side-dist fields at ±33554432; `area()` on it returns garbage (2.25e15) — reproduce the encoding; corpus records raw fields + `empty` flag, never interprets area when empty |
| T12 | doubles in corpus output | compare as raw IEEE bits (`Double.doubleToRawLongBits` ↔ `f64::to_bits`) — never via string formatting (Java/Rust shortest-roundtrip differ) |
| T13 | `Point.getInstance(BigInt,BigInt,BigInt)` | down-converts to IntPoint only when both divide evenly and fit CRIT_INT_BIG; else RationalPoint — port the same test |
| T14 | Lazy memoization (`Line.dir`, `IntOctagon.precalculatedToSimplex`, `Simplex.intersectionSides`, `Polyline.precalculatedCorners`…) | performance-only in Java; Rust uses plain `OnceLock`/recompute — results identical because sort order (not caches) determines outcomes. Do NOT introduce caching that changes evaluation order |
| T15 | f64→i32 `as` cast semantics (added Task 4 review) | **JLS 5.1.3 double→int narrowing SATURATES** (NaN→0, ±Inf/out-of-range→clamp ±MAX) — Rust `f64 as i32` is bit-identical at every magnitude (empirically verified JDK 25 vs rustc during the Task 4 quality review). Only **long→int** keeps low 32 bits, and Rust `i64 as i32` wraps identically. Do NOT add a `java_int_cast(f64)` helper — it would encode a divergence that does not exist |
| T16 | `RationalPoint.perpendicularProjection` vs `IntPoint.perpendicularProjection` (added Task 6) | Upstream defect, bug-compatible port: for the SAME affine input the RationalPoint variant's projY numerator ADDS `det*vx*z` where the IntPoint variant SUBTRACTS `det*vx` — projections of (3,5) onto y=1 come out Int(3,1) vs Int(3,−1) (pins PP1/PP2r). RationalPoint projX uses `det*vy*z` (the correct term); do NOT "fix" either sign |
| T17 | `LineSegment.stairApproximation45` (added Task 6) | Upstream defect at LineSegment.java:432: the function-of-y branch calls `functionValueApprox(currentY)` (x-function fed a y!) instead of `functionInYValueApprox`. Reproduce verbatim; pin ST2 (45-degree stairs of a slope-½ segment) is sensitive to it |
| T18 | `Simplex.cutoutFrom(Simplex)` division-line bookkeeping (added Task 7) | Two upstream quirks, bug-compatible port: (1) the loop ends with the dead store `nextDivisionLine = prevDivisionLine` (Simplex.java:860) and `prevDivisionLine` is never assigned, so every `prevDivisionLine != null` merge branch is dead — port keeps `prev_division_line` always-`None`; (2) `calcDivisionLines` uses `isExact` with strict `<` on the 1e-4 tolerance while other tolerance sites use `<=` — measure-zero divergence, left as-is. Pin P12: 5 pieces, piece 3 carries 5 border lines via the `merge_first` branch only |
| T19 | Java `Direction` comparisons come in TWO flavors — each SITE must be read, not assumed (corrected in the Task 7 spec review) | `==` is reference identity: it occurs at the `calcDivisionLines` NULL guards (`currentProjectionDir == Direction.NULL`, true only for the shared constant `Point.perpendicularDirection` returns in its collinear branch) where an `IntDirection::is_null()` zero-vector check is a sound port. `.equals` (inherited, final) is VALUE equality — collinear side + POSITIVE projection, under which distinct NULL instances are unequal — and maps to Rust `==`. Simplex.java:1137 `firstProjectionDir.equals(secondProjectionDir)` is the `.equals` flavor; an identity-flag port diverges from it on raw parallel-pair simplices. Pin PB9: raw outer simplex with a duplicated same-direction line pair drives the second-division scan and the 2-division-line branch |
| T20 | `Line` equality is ANCHOR-BLIND in both languages — anchor identity is observable parity only through endpoint pairs (added Task 7 quality review) | Java `Line.equals` compares supporting line + direction, not where the line starts/ends; Rust `Line ==` mirrors it, so `assert_eq!` on `Line` values CANNOT detect a wall-anchor divergence (the P12 pin literals were mis-transcribed with (6,−5) anchors and still passed 263/263). Meanwhile the differential corpus serializes lines as their two anchor points, so anchors ARE observable parity. Java carries ORIGINAL `Line` objects through cutout/intersection unchanged (`IntOctagon.cutoutFrom(Simplex)` = `toSimplex().cutoutFrom(simplex)`; the square's original right wall (6,0)→(6,6) survives into the pieces), and the port does the same — verified stage-by-stage against the jar (P12b). Rule: pins that claim line identity must compare `(&line.a, &line.b)` endpoint pairs, not `Line` equality. Pins P12/PB9 are anchor-strict |
| T21 | `TileShape.indexOfNearestCorner` seeds its minimum with `Double.MIN_VALUE` (added Task 8) | Upstream oracle bug, bug-compatible port: `Double.MIN_VALUE` is the smallest POSITIVE double (4.9e-324), not the most negative — a corner distance can beat it only by being exactly 0, so for any query point that is not itself a corner the result is pinned to index 0 even when a later corner is strictly nearer (pin PINC1: box (0,0)-(10,10), query (3,7) returns 0 although corner (0,10) is nearer; PINC2: query ON a corner returns its index because 0 < 4.9e-324). Port seeds `f64::from_bits(1)`; do NOT "fix" to `-f64::MAX` or `f64::INFINITY`. Related empirical note: `Line.intersectionApprox` is inexact at FLIP_BIG magnitudes (corner for the PFL2 input comes out (2^27, 2^27) instead of the exact (2^27+1, 2^27)), which flips the Polyline ctor direction decision — pin the observed oracle output, never hand-modeled FP algebra |
| T22 | Infinite corner points flow through `LineSegment` (added Task 10) | Java `LineSegment.startPoint()/endPoint()` call `middle.intersection(closingLine)`, which NEVER returns null: for a closing line parallel to the middle line it returns the infinite `RationalPoint(isX, isY, 0)`, and every consumer (`sortEndpointsInXY`, `intersection`, `sideOfIntersection`, `contains`, `toSimplex`, `borderIntersections`) keeps computing through it with exact cross-multiplied `compareXY`/`sideOf` — Java never panics. The port originally panicked at `start_point`/`end_point` and skipped the parallel candidate in `border_intersections`; corpus cases seg-000020/93/125 (`sort_endpoints_in_xy`) and seg-000151 (`line_segment.intersection`) panicked where Java degrades gracefully. Fix: `Line::intersection_point` (Java-verbatim, returns the infinite RationalPoint) + the `Option`-returning `intersection` wrapper on top; `LineSegment` consumes `intersection_point` only. Pins INF1 (Rat(822986573811360, 822986573811360, 0)), SORT-INF1/2/3, CMP-INF1/2, I-INF |
| T23 | Oracle args are RAW constructors, not `getInstance` (added Task 10) | `GeometryCorpusOracle.pt()`/`points()` build `new IntPoint(x, y)` directly, so an input above ±CRIT_INT (2^25) stays an IntPoint; the Rust harness initially called `Point::get_instance(x, y)`, which boxes a RationalPoint above ±CRIT_INT — corpus case ipt-000140 (`difference_by` of [-2583, -185824083] and [721500227, 26196855]) then returned RationalVector{z=1} on the Rust side while Java returned IntVector. NOT a port bug (Java `IntPoint.differenceBy(IntPoint)` returns IntVector directly; there is no promotion site). Rule: harness arg construction must mirror the oracle constructor-for-constructor; `vec()` (which DOES use `Vector.getInstance`) stays `Vector::get_instance` |

### Java classes → Rust modules (the port map)

| Java class (geometry/planar/) | Rust module (epic-geometry/src/) |
|---|---|
| Limits, Side | `limits.rs`, `side.rs` (+ `rounding.rs` for T1 helpers) |
| FloatPoint, FloatLine | `float_point.rs`, `float_line.rs` |
| Point, IntPoint, RationalPoint | `point.rs` (enum + dispatch), `int_point.rs`, `rational_point.rs` |
| Vector, IntVector, RationalVector | `vector.rs`, `int_vector.rs`, `rational_vector.rs` |
| Direction, IntDirection, BigIntDirection, FortyfiveDegreeDirection | `direction.rs`, `int_direction.rs`, `big_int_direction.rs`, `fortyfive_degree_direction.rs` |
| Line, LineSegment | `line.rs`, `line_segment.rs` |
| Area, Shape, ConvexShape (interfaces) | `shape.rs` (Rust traits) |
| IntBox | `int_box.rs` |
| IntOctagon | `int_octagon.rs` |
| TileShape, RegularTileShape, Simplex | `tile_shape.rs`, `regular_tile_shape.rs`, `simplex.rs` |
| Polygon, PolygonShape, PolylineShape | `polygon.rs`, `polygon_shape.rs`, `polyline_shape.rs` |
| Polyline | `polyline.rs` |
| PolylineArea | `polyline_area.rs` (Stoppable → `&AtomicBool` param) |
| Circle, Ellipse | `circle.rs`, `ellipse.rs` (Ellipse display-only) |
| ShapeBoundingDirections, OrthogonalBoundingDirections, FortyfiveDegreeBoundingDirections | `bounding.rs` |
| datastructures: Signum, BigIntAux | fold into `side.rs`, `big_int_aux.rs` |
| FRLogger usage | drop (replace with nothing; geometry math never logs in the corpus path) |

Sealed hierarchies (pinned by `SealedGeometryHierarchyTest`): `Point → {IntPoint, RationalPoint}`; `TileShape → {RegularTileShape, Simplex}`; `RegularTileShape → {IntBox, IntOctagon}`. Rust mirrors as enums/traits with exactly these variants.

### Differential corpus mechanics (verified live with jshell)

- The jar is a fat jar (28,291 classes); geometry API is public with public final int fields; Gson included. `~/.jdks/jdk-25.0.4.1+1/bin/java` runs single-file source programs: `java -cp <jar> GeometryCorpusOracle.java cases.jsonl > golden.jsonl`.
- Rust generates cases (seeded, deterministic), Java evaluates, both sides emit result lines; compare by id + field equality.
- Case/result JSONL (one per line):

```json
{"id":"oct-000042","op":"octagon.intersection","args":{"a":[0,1000,0,700,-500,1500,-500,1500],"b":[500,2000,300,900,-500,2500,-200,2000]}}
{"id":"oct-000042","out":{"kind":"IntOctagon","v":[500,1000,300,700,300,1000,300,1000],"empty":false,"area_bits":-9007199254740992}}
```

Rules: ints decimal; doubles ALWAYS as `bits: i64` (`doubleToRawLongBits` / `f64::to_bits`); shape results carry `kind` + raw fields; `TileShape` results dispatch on the sealed hierarchy (`IntBox | IntOctagon | Simplex`); `Simplex` results serialize as the line array (each line as its two points). **Points at infinity** (RationalPoint with z=0 — what Java `Line.intersection` returns for parallel/collinear pairs, never null; settled empirically in the Task 6 spec review) serialize on BOTH sides as `{"kind":"Infinity"}`: the Rust `None` and the Java z=0 result map to the same token, so the generator still emits parallel/collinear pairs and compare pins the mapping instead of skipping the branch.

---

## Locked decisions (do not relitigate during execution)

- **D1 — Bit-parity arithmetic now, widening later.** Port Java's int/long/double/BigInteger exactly (including wraparound and double predicates). i128/checked arithmetic is an M5 optimization behind this same corpus. This amends the `epic-geometry` lib.rs doc comment (written in M0 as "i128 fast path with checked arithmetic") — update that comment in Task 1 to describe the parity-first policy.
- **D2 — `num-bigint = "0.4"`** is the only new dependency, used for BigInteger-fallback paths. No proptest/fuzzing deps in M1a (the corpus IS the property layer; algebraic invariants are plain `#[test]` loops with a small seeded RNG in-crate).
- **D3 — No serde in epic-geometry.** The crate stays pure math; corpus (de)serialization types live in `epic-harness`. (Exception: none. If a corpus op needs to name a geometry value, the harness pattern-matches on the public API.)
- **D4 — API style:** Rust snake_case of the Java camelCase names (`side_of`, `turn_45_degree`, `intersection`, `bounding_octagon`, `offset_shape`). Double dispatch becomes enum `match` or trait methods — but the OBSERVABLE semantics (which overload runs) must be preserved.
- **D5 — Corpus artifacts are committed:** `rust/harness/corpus/geometry-cases.jsonl` (seeded generator output) and `rust/harness/corpus/geometry-golden.jsonl` (Java's answers). Regenerating goldens requires the jar; `compare` runs in CI-able time (<1 min) with `EPIC_SKIP_GRADLE` semantics n/a (no gradle needed once goldens exist).
- **D6 — Simplex scope:** port fully (sort by `Line.compareTo`, `remove_redundant_lines`, intersection, cutout) — it is reachable from `IntOctagon.to_simplex` and the corpus. `PolygonShape.split_to_convex` ports with the seeded `Random(99)` reproduced exactly (Java `java.util.Random` LCG — implement the 48-bit LCG in Rust; `PolygonShape.java:18-19,534-535`).
- **D7 — `java.util.Random` LCG** is ported into `epic-geometry` (private helper, `java_random.rs`) because `PolygonShape` reseeds `new Random(99)` internally and its decomposition order must match. Seeds: `next(bits)` standard 48-bit LCG.

---

## Task 1: Foundations — limits, rounding, side, BigRational aux

**Files:**
- Create: `rust/crates/epic-geometry/src/limits.rs`, `rounding.rs`, `side.rs`, `big_int_aux.rs`, `java_random.rs`
- Modify: `rust/crates/epic-geometry/src/lib.rs` (module decls + doc-comment amendment per D1)
- Modify: `rust/crates/epic-geometry/Cargo.toml` (num-bigint dep)

- [ ] **Step 1: Write the failing tests** (new `#[cfg(test)]` mod per file; shown for `rounding.rs` — the T1 trap):

```rust
// rounding.rs
/// Java 7+ `Math.round(double)` (JDK-6430675): closest integer with ties
/// toward +infinity, computed without the double-rounding artifact of the
/// naive `floor(d + 0.5)` formula (which mis-rounds values 1 ulp below a
/// half, e.g. 0.49999999999999994). `d - d.floor()` is exact for finite
/// doubles, so `frac >= 0.5` is the exact tie test. NaN -> 0, ±Inf and
/// out-of-range saturate (same as Java's `(long)` cast).
pub fn java_round(d: f64) -> i64 {
    let floor = d.floor();
    let frac = d - floor;
    if frac >= 0.5 { (floor + 1.0) as i64 } else { floor as i64 }
}
/// Java `Math.rint`: half-to-even.
pub fn java_rint(d: f64) -> f64 { /* nearest, ties to even */ }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn java_round_matches_floor_plus_half_semantics() {
        assert_eq!(java_round(2.5), 3);      // toward +inf
        assert_eq!(java_round(-2.5), -2);    // NOT Rust round()'s -3
        assert_eq!(java_round(-2.6), -3);
        assert_eq!(java_round(2.4), 2);
        assert_eq!(java_round(0.49999999999999994), 0); // JDK-6430675
    }
    #[test]
    fn java_rint_rounds_halves_to_even() {
        assert_eq!(java_rint(2.5), 2.0);
        assert_eq!(java_rint(3.5), 4.0);
        assert_eq!(java_rint(-2.5), -2.0);
    }
}
```

Also: `limits.rs` (CRIT_INT = 33_554_432 etc. + test asserting the values); `side.rs` (`Side::{Positive, Collinear, Negative}` = Java `Side`/`Signum` union, with `negate`); `big_int_aux.rs` (`determinant(a,b,c,d) -> BigInt` = `a*d - c*b` in Java's order, `add_rational_coordinates` using **product of denominators, never gcd-reduced** (BigIntAux.java) + test that (1/2)+(1/3) yields (5,6) unreduced, `binary_gcd` unsigned semantics + test vs gcd on positives); `java_random.rs` (48-bit LCG: seed `(seed ^ 0x5DEECE66D) & ((1<<48)-1)`; `next(bits)` = `(seed = seed*0x5DEECE66D + 0xB & mask) >>> (48 - bits)`; `next_int() = next(32)` as i32; `next_double() = ((next(26) as i64) << 27 + next(27)) / 2^53` — pin against Java `Random(99)`: run jshell `new java.util.Random(99)` first and put the ACTUAL first-nextInt/first-nextDouble values in the test — do not trust any pre-listed constant blindly).

- [ ] **Step 2: Run `cd rust && cargo test -p epic-geometry`** — red first (modules missing), then implement until green.

- [ ] **Step 3: Update `lib.rs`** doc comment: replace "i128 fast path with checked arithmetic and BigRational fallback; no mul_add; operation order identical" with the D1 policy: "bit-parity port of Java geometry/planar: i32 with Java wraparound (wrapping_*), double-for-double predicates, num-bigint fallback; i128 widening deferred to M5 behind the differential corpus. No mul_add; operation order identical to the Java original." Declare `pub mod` for the five new modules.

- [ ] **Step 4: Commit**

```bash
git add rust/crates/epic-geometry/
git commit -m "feat(m1a): geometry foundations — limits, java rounding, side, bigint aux, java Random LCG

Co-Authored-By: Claude Code <noreply@anthropic.com>"
```

---

## Task 2: Points and vectors — IntPoint, RationalPoint, FloatPoint, Vector family

**Files:**
- Create: `rust/crates/epic-geometry/src/{point,int_point,rational_point,float_point,vector,int_vector,rational_vector}.rs`
- Modify: `lib.rs`

Port from `geometry/planar/{Point,IntPoint,RationalPoint,FloatPoint,Vector,IntVector,RationalVector}.java`.

- [ ] **Step 1: Define the sealed representation.** `Point` is an enum `{ Int(IntPoint), Rational(Box<RationalPoint>) }` mirroring the sealed hierarchy; `Vector` likewise. Every Java polymorphic op (`difference_by`, `translate_by`, `compare_x/compare_y/compare_xy`, `perpendicular_projection`, `side_of`, `turn_90_degree`, `equals`) becomes a `match` pair dispatch — the dispatch table must cover all 2×2 type pairs exactly as Java's double dispatch does (Point.java delegates; read each branch in IntPoint.java/RationalPoint.java).
- [ ] **Step 2: IntPoint** — `pub struct IntPoint { pub x: i32, pub y: i32 }`. Ctor logs-nothing on `|x| > CRIT_INT` (soft guard — in Rust: debug_assert only, never panic, parity with log-and-continue). Ops: `determinant` in **i64** (T-ladder rule 2), `surrounding_octagon` (int arithmetic, may wrap — use wrapping ops if javac would wrap), `fortyfive_degree_projection` (min over double array, `<` first-index-wins tie-break, diag `((dy as f64) ± (dx as f64))/2.0` then truncate-cast — IntPoint.java:219-252), `distance_square` (f64), `round` (java_round), `compare_*` lexicographic.
- [ ] **Step 3: RationalPoint** — `pub struct RationalPoint { x: BigInt, y: BigInt, z: BigInt }` (z ≥ 0 enforced; z==0 = point at infinity, all such points equal). NOT normalized in ctor (T5). `PartialEq` = cross-multiplication via `big_int_aux::determinant` (x1*z2 == x2*z1 && y1*z2 == y2*z1, treating z==0 specially). `hash` = gcd-reduced (mirror RationalPoint.hashCode:93-109) — hand-derive `Hash` accordingly; `get_id` (raw, unreduced hash) as a separate method. `Point::get_big_int_instance(x,y,z)` implements T13's single-divisor down-conversion.
- [ ] **Step 4: FloatPoint** — plain `f64` pair; `round` → java_round to IntPoint; `round_to_grid` → java_rint (T1); `inside_circle` with the `< radius_square - 1` tolerance (FloatPoint.java:461-466); `distance_square`, `length`, arithmetic.
- [ ] **Step 5: Vectors** — `IntVector` ctor "range check omitted for performance" → no check; `side_of`/`projection` in **double** (T-ladder rule 3, IntVector.java:136,158); `Vector::get_instance` promotes to RationalVector iff `|x| > CRIT_INT || |y| > CRIT_INT`. RationalVector ctor normalizes sign of z only.
- [ ] **Step 6: Tests** pinning Java-pinned semantics (from `PointEqualsHashCodeTest.java`): IntPoint hash `31*x + y`; RationalPoint (100,200,50) == (2,4,1); (−4,6,2) == (−2,3,1); all z=0 points equal with hash 0; plus trap tests: IntPoint.determinant uses i64 at the CRIT_INT boundary (e.g. det of (33_554_432, 0)-(0, 33_554_432) — construct a case where i32 mult would wrap and assert the exact Java value, computed by hand or via jshell); float-point round(-2.5) == (-2).
- [ ] **Step 7: `cargo test -p epic-geometry && cargo clippy --workspace --all-targets -- -D warnings` → green; commit** `feat(m1a): point + vector families (int/rational/float) with Java double dispatch`

---

## Task 3: Directions

**Files:** Create `rust/crates/epic-geometry/src/{direction,int_direction,big_int_direction,fortyfive_degree_direction}.rs`; modify `lib.rs`.

- Port `Direction.java` (9 static IntDirection constants RIGHT..NULL, `get_instance(Vector)` normalization = gcd-reduce + sign rule), `IntDirection.java` (normalized gcd-reduced pair; `turn_45_degree` with the **8-entry table and `n = factor % 8` Java-sign semantics — negative factor → NULL** (T4); `turn_90_degree` normalizing while-loops; `compareTo` = angle order via double determinant (IntDirection.java:76); `equals` = collinear + positive projection (T9)), `BigIntDirection.java` (turn45 unimplemented → return self, matching the warn-and-return-this), `FortyfiveDegreeDirection` (enum of 8 + `get_direction()` mapping).
- Tests: turn45(1) == RIGHT45 rotated as Java; turn45(-1) == NULL; turn90 negative normalization; direction equality for opposite vectors (unequal) vs collinear same-sign (equal); compareTo ordering of the 8 compass directions matches declaration angle order.

Commit: `feat(m1a): direction family — 45° turn table, angle order, promotion`.

---

## Task 4: IntBox

**Files:** Create `rust/crates/epic-geometry/src/int_box.rs`.

- Port `IntBox.java` fully: `EMPTY` sentinel (inverted CRIT_INT box), **value-based `is_empty`** (T3), `intersection` (returns EMPTY sentinel on disjoint), `union`, `area` (f64), `circumference` (int arithmetic — may wrap), `offset(i32,i32)` and `offset(f64)` via java_round, `bounding_octagon`, `contains`, `intersects`, `overlaps`, `compare(other, edge_index)` (RegularTileShape edge order), `cutout`/`cutout_from`, `distance`/`border_distance` (f64), `to_simplex`, `turn_90_degree`, `enlarge`, `center`, `largest_enclosed_box` if public.
- Tests: EMPTY sentinel round-trip; is_empty value semantics (a hand-constructed inverted box IS empty — contrast with octagon task's identity semantics in a doc comment); offset(-2.5) rounding per T1; intersection of disjoint returns the sentinel fields exactly.

Commit: `feat(m1a): IntBox — value-empty semantics, cutout, offset rounding`.

---

## Task 5: IntOctagon (the router's hot shape)

**Files:** Create `rust/crates/epic-geometry/src/int_octagon.rs`.

- Port `IntOctagon.java` fully. Representation: 8 `pub i32` fields in Java declaration order (`left_x, bottom_y, right_x, top_y, upper_left_diagonal_x, lower_right_diagonal_x, lower_left_diagonal_x, upper_right_diagonal_x` — diag values are x-axis intercepts).
- `EMPTY` constant = the ±33554432 sentinel tuple (T11); `is_empty()` = field-equality with the sentinel, with a doc comment explaining the Java reference-identity subtlety (T2) and why value-equality is equivalent on canonical outputs.
- **`normalize()`**: the fixed 12-step sequence from IntOctagon.java:399-541 — port step-for-step in the same order, using java ceil/floor on `x / 2.0` doubles exactly where Java does, ending with the emptiness check returning `EMPTY`. This is the single most order-sensitive function in the kernel (T7).
- `intersection` (8-int branch arithmetic + normalize), `union`, `intersects`/`overlaps` (:639-669), `contains(point/float_point)` (:332), `offset(f64)` (:298, java_round), `enlarge`, `bounding_box`, `corner(index)`, `left/right/lower/upper value` accessors (:716-740), `side_of_border_line` (:581), `compare(other, edge_index)` (:749), `dimension`, `area` (f64 — garbage-on-empty reproduced, T11), `to_simplex` (memoized in Java — compute fresh; D4/T14), `turn_90_degree`, `nearest_border_projections` insertion-sorted top-k with strict `<` and FortyfiveDegreeDirection enum-order iteration (:913-940).
- Tests: normalize idempotence (normalize(normalize(x)) == normalize(x)) on seeded random octagons (use the in-crate seeded RNG, not rand); a degenerate sliver case hand-pinned from jshell (generate during implementation: run `new IntOctagon(...).normalize()` in jshell, paste the 8 ints into the test as the expected value with the jshell transcript in a comment); EMPTY field values exact; area-on-empty returns the exact Java garbage double (bit-compare).

Commit: `feat(m1a): IntOctagon — 12-step normalize, sentinel empty, hot intersection/overlap`.

---

## Task 6: Lines — Line, LineSegment, FloatLine

**Files:** Create `rust/crates/epic-geometry/src/{line,line_segment,float_line}.rs`.

- `line.rs`: `Line { a: Point, b: Point, dir: OnceLock<Direction> }` (cache is fine — it memoizes a pure function; T14). Port: `direction()`, `fast_equals` (double det, :91), `equals` (exact side+projection), `is_equal_or_opposite`, `side_of(point)` and `side_of(float_point, tolerance)`, `side_of_intersection` tolerance-then-exact ladder (T8, :152-164), `perpendicular_projection`, `intersection(other) -> Option<Point>` with **all 16 fast paths in Java's branch order before the BigInteger general formula** (T6, :218-304 — enumerate them in a comment table as you port), `intersection_approx`, `function_value_approx`, `compare_to` (:469 double), `length` (int wraparound preserved — wrapping ops), `middle_approx`, `change_length_approx`, `is_orthogonal/diagonal/multiple_of_45_degree`, `turn_90_degree`, `translate_by`.
- `line_segment.rs`: triple-of-lines representation, memoized corners as plain recomputation; `intersection(LineSegment) -> Option<Line>` (returns LINES not points), `sort_endpoints_in_xy`, `bounding_box/bounding_octagon`, `perpendicular_projection`, stair-width offsets (java_round).
- `float_line.rs`: pure f64 ops ported directly; `is_contained_in_box` with the 0.01 tolerance literals.
- Tests: one fast-path intersection per category (orthogonal×45, 45×45, orthogonal×orthogonal, parallel→None, collinear→None) with values pinned from jshell; the tolerance ladder case where float says near but exact says COLLINEAR (construct from Line.java:152-164 semantics); length wraparound case (dx=dy=46341 → wrapped int value as f64).

Commit: `feat(m1a): line family — 16-path intersection, tolerance ladders, approx layer`.

---

## Task 7: TileShape hierarchy — traits, Simplex

**Files:** Create `rust/crates/epic-geometry/src/{shape,tile_shape,regular_tile_shape,simplex}.rs`.

- `shape.rs`: `Area`, `Shape`, `ConvexShape` as traits mirroring the Java interfaces (is_empty, bounding_box, contains, split_to_convex, offset/shrink/max_width/min_width, intersects double-dispatch). In Rust the 4-overload `intersects` double dispatch becomes `fn intersects(&self, other: &ShapeRef) -> bool` over an enum `ShapeRef<'a> { IntBox(&'a IntBox), IntOctagon(&'a IntOctagon), Simplex(&'a Simplex), Circle(&'a Circle) }` — every implementing pair must run the SAME Java overload logic as the Java dispatch would select.
- `tile_shape.rs`: enum `TileShape { RegularTileShape(RegularTileShape), Simplex(Box<Simplex>) }` mirroring the sealed split; factories `get_instance(lines) -> Option<TileShape>` (sort + simplify), `from_points`, `from_8_ints` (→ normalized octagon), `from_4_ints` (→ IntBox→octagon per Java); `intersection(TileShape)` reverse double dispatch; `cutout_from` triple dispatch; `is_outside`, `contains`, `distance`, `border_distance`, `nearest_border_points_approx` (:378-446 strict-`<` insertion top-k, enum-order iteration).
- `regular_tile_shape.rs`: the shared IntBox/IntOctagon ops (`compare(other, edge_index)`, `union`, `contains`), `RegularTileShape` enum `enum { IntBox(IntBox), IntOctagon(IntOctagon) }`.
- `simplex.rs`: `Simplex { lines: Vec<Line> }` (sorted by `Line::compare_to`, stable sort — Java `Arrays.sort` is stable for objects); `get_instance`, `remove_redundant_lines` (:884+ with the Java `intersection_sides` cache kept verbatim including its invalidation — T14-compliant: caching that preserves evaluation order), `intersection` (:620-630), `cutout` (:706), `is_outside`, `contains`, `bounding_box`, `split_to_convex`.
- Tests: simplex dedup keeps the first of equal-direction lines (stable-sort observable); `from_8_ints` normalization; intersection box∩octagon→octagon dispatch; cutout producing multi-tile results.

Commit: `feat(m1a): TileShape sealed hierarchy — reverse dispatch, Simplex reduce`.

---

## Task 8: Polylines, polygons, areas, circles

**Files:** Create `rust/crates/epic-geometry/src/{polyline_shape,polyline,polygon,polygon_shape,polyline_area,circle,ellipse,bounding}.rs`.

- `polyline.rs`: `Polyline { lines: Vec<Line> }` — ctor: direction flipping via `side_of` on FLOAT corner approximations, then `remove_consecutive_parallel_lines` then `remove_overlaps` (T10 order); `corner_approx`, `bounding_box/bounding_octagon`, `length_approx`, `offset_shape(int,int)`, `contains`, `is_orthogonal`, `is_multiple_of_45_degree`, `fast_equals`, `equals`.
- `polyline_shape.rs` / `polygon.rs` / `polygon_shape.rs`: corners-based shapes; `PolygonShape::split_to_convex` **with `Random(99)` reseeded before use, driving the LCG from Task 1** (D6/D7, PolygonShape.java:18-19,534-535) — the decomposition ORDER is parity-observable.
- `polyline_area.rs`: border + holes; `split_to_convex(&AtomicBool)` for the Stoppable hook.
- `circle.rs`: IntPoint center + i32 radius; intersects via float/int distance mix as Java; `split_to_convex` → bounding tile.
- `ellipse.rs`: display-only struct (center FloatPoint, rotation f64, radii) — no Shape impl.
- `bounding.rs`: `ShapeBoundingDirections` trait + Orthogonal/FortyfiveDegree impls (`bounds(IntBox/IntOctagon) -> TileShape`) — these feed M2's search tree.
- Tests: Polyline ctor corner-flip on a case where float side-of differs from exact (construct one); overlap-removal order; circle-vs-octagon intersects on a touching-boundary case; bounding-direction octagon of a known box.

Commit: `feat(m1a): polyline/polygon/area/circle — ctor normalization, seeded ear-split`.

---

## Task 9: Differential corpus — Java evaluator + harness subcommands

**Files:**
- Create: `rust/harness/oracle/GeometryCorpusOracle.java` (~150 lines, single-file source program)
- Create: `rust/harness/src/corpus.rs`
- Modify: `rust/harness/src/main.rs` (add `Corpus { Generate, Golden, Compare }` subcommands)

- [ ] **Step 1: Write the Java evaluator.** Requirements: read cases.jsonl path from args[0]; one result line per case on stdout; every double emitted as `bits` i64 via `Double.doubleToRawLongBits`; `empty` computed as `r == IntOctagon.EMPTY` where the receiver is a canonical op result; unknown op → nonzero exit with the offending id on stderr; no package declaration (single-file source launcher); uses Gson from the jar classpath. Dispatch helper sketch (the implementer writes the real file — the skeleton has deliberate rough edges):

```java
// GeometryCorpusOracle.java — differential oracle for the epic-geometry port.
// Run (repo root): ~/.jdks/jdk-25.0.4.1+1/bin/java -cp build/libs/freerouting-current-executable.jar \
//     rust/harness/oracle/GeometryCorpusOracle.java cases.jsonl > golden.jsonl
// Lives OUTSIDE src/ — the frozen tree is never touched; only the built jar is consumed.
import app.freerouting.geometry.planar.*;
import com.google.gson.Gson;
import com.google.gson.JsonObject;
import java.io.*;
import java.nio.file.*;
import java.util.*;

public class GeometryCorpusOracle {
    static JsonObject evaluateOp(String op, JsonObject a) {
        switch (op) {
            case "octagon.normalize":   -> use oct(a,"v").normalize()
            case "octagon.intersection" -> use oct(a,"a").intersection(oct(a,"b"))
            // ... one arm per corpus op from the op table; default: throw
        }
        throw new IllegalArgumentException("unknown op " + op);
    }
    // helpers: ip(o,k) -> IntPoint from 2-int array; oct(o,k) -> IntOctagon from 8-int array;
    // octOut(r) -> {kind:"IntOctagon", v:[8 ints], empty:r == IntOctagon.EMPTY};
    // dblOut(d) -> {bits: Double.doubleToRawLongBits(d)}; build JsonArrays explicitly.
    public static void main(String[] args) throws Exception { /* stream lines, Gson parse, println result */ }
}
```

**Op table** (each becomes both a Java case and a Rust dispatch arm): `intpoint.{determinant,difference_by,fortyfive_degree_projection,perpendicular_projection,surrounding_octagon}`, `vector.{side_of,projection,turn_90_degree}`, `direction.{get_instance,turn_45_degree,compare_to}`, `point.{compare_xy,translate_by}`, `intbox.{intersection,union,offset_double,area,circumference,intersects,cutout}`, `octagon.{normalize,intersection,union,offset,intersects,overlaps,contains_point,contains_float_point,area,bounding_box,corner,enlarge,compare_edge,side_of_border_line}`, `line.{get_instance,intersection,intersection_approx,side_of_point,side_of_intersection,perpendicular_projection,fast_equals,compare_to,direction,length}`, `line_segment.{intersection,bounding_box,sort_endpoints_in_xy}`, `tileshape.{from_8_ints,intersection_box_oct,from_points}`, `simplex.{get_instance,remove_redundant,intersection}`, `polyline.{ctor,bounding_box,length_approx,offset_shape,is_multiple_of_45_degree}`, `circle.{intersects_octagon,bounding_octagon}`, `floatpoint.{round,round_to_grid,distance_square,inside_circle}`, `floatline.{intersection,projection}`.

- [ ] **Step 2: `corpus.rs` in the harness.** `epic-harness corpus generate` — seeded case generator (deterministic; the seed is a CLI arg with a fixed default, e.g. 20260912; magnitude distribution centered on realistic board coords (±10^4) PLUS a tail sampling up to ±CRIT_INT and a few beyond, to exercise wraparound paths; parallel/collinear `line.intersection` pairs are emitted deliberately to pin the Infinity↔None mapping — see corpus mechanics rules); writes `rust/harness/corpus/geometry-cases.jsonl`. `corpus golden` — spawns the Java evaluator via the existing `oracle.rs` java/jar resolution (`java -cp <jar> <file.java> cases.jsonl`, argv array, stdout redirected to `geometry-golden.jsonl`); requires the jar. `corpus compare` — runs each case through `epic-geometry`, diffs against golden by id, prints `PASS n/n` or up to 20 first-mismatch lines (op, id, expected vs actual JSON), exit 1 on any mismatch.
- [ ] **Step 3: Rust-side result emission** mirrors the Java helper semantics exactly (T12 bit-doubles; `empty` by sentinel-field equality — agreeing with Java's identity check on canonical outputs).
- [ ] **Step 4: Smoke**: generate a 200-case corpus, run golden (jar exists), compare → expect mismatches are POSSIBLE if port bugs exist — Task 10 is where parity goes green. Verify plumbing only: deterministic generate (two runs byte-identical), golden line count == case count, compare reports (not crashes) on a hand-broken golden.
- [ ] **Step 5: Commit** `feat(m1a): differential corpus — Java evaluator + generate/golden/compare`.

---

## Task 10: Corpus parity green + invariant tests

**Files:** Modify `rust/crates/epic-geometry/**` (fix divergences), `rust/harness/src/corpus.rs` (generator coverage gaps).

- [x] **Step 1: Generate the full corpus**: ≥ 5,000 cases, ops weighted toward the hot path (intersection/normalize/side_of/offset/contains ≈ 60%), include dedicated edge bands: coordinates at ±CRIT_INT, at ±46341 (length wraparound), degenerate slivers (octagons whose normalize tightens differently per step order), diagonal-heavy inputs, empty-producing pairs (disjoint boxes/octagons).
  - **Weighting adjudication (spec review 2026-09-12)**: the ≈60% target is measured at the FAMILY level (octagon+line+intbox+line_segment = 57.3%, +tileshape = 61.1% of the 5000-case corpus), not per op name. Every family and op remains present; hot ops are the most frequent individual ops. Within-family op weighting was considered and rejected — complexity without parity value.
- [ ] **Step 2: `corpus golden` + `corpus compare` → iterate to 100% pass.** Every mismatch: classify per the trap table (T1-T14) before fixing; if a mismatch reveals a plan-trap omission, append the trap to this plan's table (doc-of-record) in the fix commit.
- [ ] **Step 3: Algebraic invariant tests** (plain `#[test]` loops, seeded in-crate RNG, 10k iterations each): intersection commutativity (a∩b == b∩a for octagon/box), normalize idempotence, `side_of` sign antisymmetry under operand swap (where Java agrees — check IntVector.sideOf asymmetry in Java first and port what Java does, not what algebra says), polyline ctor idempotence (Polyline(Polyline(p).lines) == Polyline(p)), simplex sort-order stability.
- [ ] **Step 4: Commit cases + goldens** (D5): `test(m1a): geometry corpus (N cases) green vs Java oracle + algebraic invariants`.

---

## Task 11: M1a exit criteria + docs

- [ ] **Step 1:** `cd rust && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check` — all green.
- [ ] **Step 2:** `EPIC_SKIP_GRADLE=1 cargo run -q -p epic-harness -- corpus compare` — green against committed goldens (no Java needed → CI-able; DO add `corpus compare` as a CI step in `rust-check.yml` — it needs only the committed files, zero JDK).
- [ ] **Step 3:** Update `rust/README.md` (geometry done; corpus commands) and the crate's milestone note; append the M1a line to `.wolf/memory.md`.
- [ ] **Step 4:** Commit `docs(m1a): README + corpus CI step + session log`.
- [ ] **Step 5:** Report: M1a exit = corpus green + invariants green + cargo/CI green. M1b (DSN) plan follows separately.

---

## Self-review notes

1. **Spec coverage vs design §6 M1 row**: "Geometry parity green" = Tasks 9-10 (corpus) + Task 10 Step 3 (properties); "DSN parses all fixtures / SES round-trip" = M1b plan (deferred by the M1 split note). §5 geometry gate = the corpus; §9 risks 1-3 (overflow, double drift, tie-break) are addressed by D1 + the trap table + corpus edge bands.
2. **No placeholders**: foundations/tests/corpus-schema code is complete above; bulk ports reference exact Java files/lines with per-class trap lists — the Java source in-repo is the normative spec (a faithful transliteration cannot be both complete and inlined without duplicating 11.8k LOC).
3. **Type consistency**: `Point`/`Vector` enums + boxed Rational variants; `TileShape`/`RegularTileShape` enums; `java_round` shared by all offset sites; corpus `bits:i64` convention used by both evaluators.
4. **Known honest simplifications**: `is_empty` value-equality (T2 — equivalent on canonical outputs, documented); Simplex `intersection_sides` cache kept verbatim incl. invalidation (T14-compliant); FRLogger dropped (geometry math never logs in corpus paths).
