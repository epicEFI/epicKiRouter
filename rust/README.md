# EpicRouter Rust Workspace

Rust rewrite of the Freerouting autorouter (design:
`docs/superpowers/specs/2026-09-11-epicrouter-rust-rewrite-design.md`).
The Java tree under `src/` stays untouched as the parity oracle.

Status: M0–M3 complete — scaffold + golden baselines, geometry kernel,
DSN/SES I/O, arena board model, incremental spatial index, the
detail-routing core (`epic-router`: expansion → maze → ripup/shove →
locator → forced insertion → batch driver), `epic-drc` (incompletes +
clearance counting), and `epic-cli` (headless route: `epicrouter -de
board.dsn -do out.ses`); 1,305 workspace tests green. M3 exited
EXIT_WITH_DEVIATIONS: no instrument in M3 shows a single-net route of
lower quality than the Java oracle's, but the Tier A router battery
stands 8 pass / 3 red (bm06 tuning, bm11 red-by-1-at-truth, bm01
wall-limited — each root-caused as non-parity, with an M4 lever on
record), and the event-stream compare is honest-red pending #61. All
six M2 compares remain green; the CI router gate stays in
`--report-only` until the battery reads 11/11 (flip conditions in the
M3 exit note of the design doc's §6 and the rust-check.yml comment).
Port ledger: `crates/epic-router/SEAM.md`.

## Layout

- `crates/epic-geometry` — exact 45° integer geometry kernel (M1a done:
  bit-parity port + 5k-case differential corpus vs the Java oracle)
- `crates/epic-dsn` — Specctra DSN/SES I/O (M1b done: full DSN parse
  parity on all 1,332 fixtures — 175 digest + 1,157 soak — plus byte-equal
  SES session emission on tier A+B vs the Java oracle)
- `crates/epic-board` — arena board model (M2 done: ItemId arena +
  undoable containers, rules/structure/placement, item geometry, the
  tree-manager seam, trace combine/split/normalize, and the undo facade —
  33/33 snapshot/undo replay parity vs the Java oracle)
- `crates/epic-index` — incremental spatial index (M2 done: ShapeTree +
  MinAreaTree with tree-shape precalc and compensation — 33/33
  query-set parity vs the Java oracle)
- `crates/epic-drc` — exact incremental DRC (M3 done: incompletes +
  clearance violations vs the Java oracle, 17-fixture compare; the
  incremental DRC engine remains M6)
- `crates/epic-router` — detail-routing core (M3 done: expansion →
  maze → ripup/shove → locator → forced insertion → batch driver;
  `SEAM.md` in the crate is the port ledger; fanout/optimizer/global
  stages are M4/M6+)
- `crates/epic-engine` — 13-line scaffold (M4: jobs, layered settings,
  seeds — M3's settings face lives in `epic-cli/src/settings.rs`)
- `crates/epic-cli` — `epicrouter` CLI (M3 done: headless `route`,
  manifest + SES + exit-code semantics)
- `crates/epic-gui` — egui frontend (M9)
- `harness` — parity + benchmark harness against the Java oracle

## Commands

```bash
cd rust
cargo test --workspace              # unit tests (no Java needed)
cargo test -p epic-harness -- --ignored   # harness e2e tests (needs jar + JDK 25)

# Golden baselines (needs the oracle jar; see below)
./gradlew -q executableJar                      # from repo root
EPIC_SKIP_GRADLE=1 cargo run -p epic-harness --release -- list
EPIC_SKIP_GRADLE=1 cargo run -p epic-harness --release -- capture --tier A
EPIC_SKIP_GRADLE=1 cargo run -p epic-harness --release -- verify  --tier A
```

### Corpus

```bash
cd rust
# Differential geometry corpus (committed goldens — no Java needed)
cargo run -q -p epic-harness -- corpus compare   # PASS 5000/5000
# Regenerating
cargo run -q -p epic-harness -- corpus generate              # pure Rust
EPIC_ORACLE_JAVA=~/.jdks/jdk-25.0.4.1+1/bin/java \
  cargo run -q -p epic-harness -- corpus golden              # needs jar + JDK 25; explain WHY in the commit
```

### DSN parse parity (M1b)

```bash
cd rust
# All 1,332 fixtures (175 digest + 1,157 soak) vs the committed Java-oracle
# digest goldens — java-free, all-match (the M1-era dsn-0151 divergence
# ledger was retired once epic-board gained normalizeAllTraces, D22).
cargo run -q -p epic-harness -- dsn compare
# Regenerate manifest / capture goldens (needs jar + JDK 25; explain WHY)
cargo run -q -p epic-harness -- dsn manifest
EPIC_ORACLE_JAVA=~/.jdks/jdk-25.0.4.1+1/bin/java \
  cargo run -q -p epic-harness -- dsn golden --set all
```

### SES emission parity (M1b)

```bash
cd rust
# Tier A+B (20 fixtures): Rust ses::writer output vs the committed Java
# session goldens — java-free, byte-equal expected.
cargo run -q -p epic-harness -- dsn ses-compare
# Recapture the goldens (needs jar + JDK 25; explain WHY in the commit)
EPIC_ORACLE_JAVA=~/.jdks/jdk-25.0.4.1+1/bin/java \
  cargo run -q -p epic-harness -- dsn ses-golden
```

### Index query-set parity (M2)

```bash
cd rust
# Tree dumps + scripted query replay on tier + stressor fixtures vs the
# committed Java-oracle goldens (epic-board's tree manager driving
# epic-index) — java-free.
cargo run -q -p epic-harness -- index compare    # 33/33 identical
# Recapture the goldens (needs jar + JDK 25; explain WHY in the commit)
EPIC_ORACLE_JAVA=~/.jdks/jdk-25.0.4.1+1/bin/java \
  cargo run -q -p epic-harness -- index golden
```

### Snapshot/undo parity (M2)

```bash
cd rust
# Scripted snapshot/insert/remove/undo/redo/pop replay on tier +
# stressor fixtures vs the committed Java-oracle goldens (epic-board's
# undo facade) — java-free.
cargo run -q -p epic-harness -- undo compare     # 33/33 identical
# Recapture the goldens (needs jar + JDK 25; explain WHY in the commit)
EPIC_ORACLE_JAVA=~/.jdks/jdk-25.0.4.1+1/bin/java \
  cargo run -q -p epic-harness -- undo golden
```

### SES endpoint-snap canary (M2)

```bash
cd rust
# The contacts-wired session writer over the 5 routed fixtures:
# byte-equal sessions plus Java counter reproduction. snapFired=0 is
# the committed canary — the snap rule's output-changing arm is
# provably dead upstream (crates/epic-dsn/src/ses/writer.rs module docs).
cargo run -q -p epic-harness -- dsn ses-snap-compare
# Recapture the corpus + stats sidecar (needs jar + JDK 25; explain WHY)
EPIC_ORACLE_JAVA=~/.jdks/jdk-25.0.4.1+1/bin/java \
  cargo run -q -p epic-harness -- dsn ses-snap-golden
```

### Router quality gates (M3)

All java-free — the committed router-only Java records ARE the Java
side; these commands never touch the JVM. They complement the standing
compare gates above (geometry corpus 5000/5000; DSN 1,332 all-match;
SES 20/20; index 33/33; undo 33/33; ses-snap 5/5).

```bash
cd rust
# Tier A directional battery (the release profile is the official
# run): per-fixture completion >= Java, violations <= Java, score >=
# Java vs the committed router-only records. Currently 8 pass / 3 red
# (bm06 tuning buglog 181; bm11 red-by-1-at-truth buglog 175; bm01
# wall-limited buglog 175 — each root-caused non-parity, M4 levers on
# record; see the M3 exit note in the design doc §6).
cargo run -q -p epic-harness --release -- router compare
# CI posture: --report-only prints every verdict and exits 0. The flip
# to a hard gate is ONE same-commit act once the battery reads 11/11
# under a wall-resolved profile: drop --report-only, update both
# ci_tripwire.rs pins, and refresh the posture prose (this file's
# status block and this comment, plus the design doc §6 note's CI
# paragraph); the rust-check.yml comment records the condition.
cargo run -q -p epic-harness -- router compare --report-only
# Determinism self-gate: one DSN, two runs, byte-identical SES +
# manifest (digests 96ba7300… / 16714d5e… at the M3 exit).
cargo run -q -p epic-harness -- router determinism
# DRC compare: parse-time incompletes + clearance violations on 17
# fixtures (11 Tier A + 3 PCBench pre-routed + 3 crafted pin boards).
cargo run -q -p epic-harness -- drc compare
# Event-stream compare vs the committed Java probe golden: HONEST-RED
# by design — exit 1 localizes the first divergence (triaged as #61:
# item-id assignment / door-slicing / insert-path-id classes, buglog
# 172-174). Not in CI until #61 closes.
cargo run -q -p epic-harness -- events compare
```

## Oracle prerequisites

- Jar: `build/libs/freerouting-current-executable.jar` via `./gradlew executableJar`.
- **Java 25 required** (system `java` may be 21). The harness auto-detects
  `~/.jdks/jdk-25*/bin/java`; override with `EPIC_ORACLE_JAVA`.
- `EPIC_SKIP_GRADLE=1` skips the jar build (use when iterating).
- The oracle jar is built from THIS tree at baseline `e7f9bdf1`; never modify
  Java sources without re-baselining and recording why.

## Baselines

`harness/baselines/java/<tier>/<fixture>.baseline.json` are committed
artifacts. `capture` regenerates them; `verify` re-runs the oracle and gates
on final_state, exit_code, incomplete_count, clearance_violations_total, and
normalized_score (relative epsilon 1e-6). Wall time is a trend, never a gate.
