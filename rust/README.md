# EpicRouter Rust Workspace

Rust rewrite of the Freerouting autorouter (design:
`docs/superpowers/specs/2026-09-11-epicrouter-rust-rewrite-design.md`).
The Java tree under `src/` stays untouched as the parity oracle.

Status: M0–M4 complete (M4's exit verdict pending the milestone
adjudication) — scaffold + golden baselines, geometry kernel,
DSN/SES I/O, arena board model, incremental spatial index, the
detail-routing core (`epic-router`: expansion → maze → ripup/shove →
locator → forced insertion → batch driver), `epic-drc` (incompletes +
clearance counting), the M4 fanout + batch-optimizer stages with the
assembled full pipeline (`pipeline/full.rs`, the `RoutingPipeline.run()`
equivalent), and `epic-cli` (headless route: `epicrouter -de
board.dsn -do out.ses`); 1,422 workspace tests green. The M4 full-face
battery (fanout → router → optimizer vs the M0 full-flow jar records)
reads **8 pass / 3 red**: bm01 completion-by-1 at the 993.16 plateau
(buglog 175/181), bm06 5-vs-2 + score (buglog 189's room-completion
divergence, upstream of the 181 victim-choice family), bm11
wall-integrity at 600.2 s with the engine AT Java's exact terminal face
(buglog 175 — per-pass search cost, M5's lift). All seven standing
compares stay green (corpus 5000/5000; DSN 1,332; SES 20; ses-snap 5;
index 33; undo 33; drc 17) and the events compare is green (3,520
golden rows, exit 0 since M4-T6). The CI router gate stays in
`--report-only`: the M4 close adjudicated the flip HELD on both
preconditions (battery 8/3 ≠ 11/11; suite wall 2061.2 s ≈ 34 min >
the CI compare step's 30-min budget). M5 flip conditions: the full
battery reads 11/11 AND its wall fits the budget (the design doc §6 M4
exit note and the rust-check.yml comment).
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
- `crates/epic-router` — routing core (M3: the detail-routing core —
  expansion → maze → ripup/shove → locator → forced insertion → batch
  driver; M4: the fanout stage, the batch optimizer, and the
  full-pipeline assembly `pipeline/full.rs`; global routing stays M6+;
  `SEAM.md` in the crate is the port ledger)
- `crates/epic-engine` — 13-line scaffold, still unlanded (the layered
  settings face lives in `epic-cli/src/settings.rs`; jobs/seeds have no
  consumer yet)
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
# Java vs the committed router-only records. Currently 11 pass / 0 red
# (green since M4-T6 — the M3-era reds fell to the fanout stage plus
# the tightener/via-optimizer/shover wiring; the M3 face and its triage
# live in the design doc §6 M3 exit note).
cargo run -q -p epic-harness --release -- router compare
# M4 full-pipeline battery face: `--profile full` drops BOTH
# comparability flags (the assembled fanout → router → optimizer
# pipeline runs on the Rust side) and gates against the M0 full-flow
# jar records (`baselines/java/A`, default-settings captures, reused
# verbatim at M4-T11). The default (no flag) stays the router-only
# face; the T11 full battery reads 8 pass / 3 red (bm01 completion
# red by 1 at a 993.16 plateau; bm06 5-vs-2 + score, the bug-181
# victim-choice family; bm11 wall-integrity red with the engine AT
# Java's exact 975.00/3/0 face at the kill) — T12's face.
cargo run -q -p epic-harness --release -- router compare --profile full
# CI posture (M4-T13): --report-only prints every verdict and exits 0.
# The flip to a hard gate stays HELD after M4 — both preconditions
# unmet: the milestone full-face battery reads 8 pass / 3 red (buglog
# 175/181/189; logs/M4-T12/evidence/battery_full_t12.log) and its
# suite wall (2061.2 s release ≈ 34 min) exceeds the compare step's
# 30-min budget. M5 flip conditions: the full battery reads 11/11 AND
# the wall fits the budget — then drop --report-only, update BOTH
# ci_tripwire.rs pins in the SAME commit, add the events-compare step
# (java-free, green since M4-T6) with its own pin, and refresh the
# posture prose (this file's status block and this comment, plus the
# design doc §6 note's CI paragraph); the rust-check.yml comment
# records the condition.
cargo run -q -p epic-harness -- router compare --report-only
# Determinism self-gate: one DSN, two runs, byte-identical SES +
# manifest (canaries at the M4 close: ses 253e7b14… / manifest
# 8c4a9773… — the M3-exit values were 96ba7300… / 16714d5e…; every
# rotation is recorded in SEAM's task dossiers). STALENESS has TWO
# witnessed axes: `-p epic-harness` does not rebuild the spawned
# epic-cli bin (library-only dep), and a debug-profile run once
# adopted its stale target/debug sibling over the fresh release bin.
# The router gates now bail STRUCTURALLY when the resolved bin is
# older than the newest crate source (naming the bin and both rebuild
# commands) and resolution prefers release — keep the rebuild habit:
cargo build -q -p epic-cli --release
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

Two record faces serve the router-compare profiles (T11): the M0
full-flow default-settings jar captures under `baselines/java/` (the
`--profile full` battery reads these) and the M3-T14 router-only
records under `baselines/router-only/` (the default router-only face).
T11 reused the M0 `java/A` set verbatim (all 11 `fixture_sha256`
re-verified) — reuse, not recapture.
