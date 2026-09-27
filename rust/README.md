# EpicRouter Rust Workspace

Rust rewrite of the Freerouting autorouter (design:
`docs/superpowers/specs/2026-09-11-epicrouter-rust-rewrite-design.md`).
The Java tree under `src/` stays untouched as the parity oracle.

Status: M0–M4 complete (M4 exited M4_EXIT_WITH_DEVIATIONS; design §6
M4 exit note), M5 executed on top (wall slices + deterministic
parallelism) — scaffold + golden baselines, geometry kernel,
DSN/SES I/O, arena board model, incremental spatial index, the
detail-routing core (`epic-router`: expansion → maze → ripup/shove →
locator → forced insertion → batch driver), `epic-drc` (incompletes +
clearance counting), the M4 fanout + batch-optimizer stages with the
assembled full pipeline (`pipeline/full.rs`, the `RoutingPipeline.run()`
equivalent), the deterministic per-net partitioned executor with
`--threads`, and `epic-cli` (headless route: `epicrouter -de
board.dsn -do out.ses`); **1,449 workspace tests green**. The M6-T3 battery re-faces — the
first tier-aware full batteries (full flow, default settings;
`--tier A|B|C`, default A byte-identical) — read: Tier A
**10 green / 1 red** (bm01 killed at the 1800.2 s tier on the
INTEGRITY gate — routing complete 0/0/1000.00, the optimizer stage is
the wall; the completion-only reading is 11/11), Tier B
**4 green / 5 red**, Tier C **1 green / 2 red**; the BEFORE-face
tables live in `logs/M6-T3/report-t3.md`. All seven standing
compares stay green (corpus 5000/5000; DSN 1,332; SES 20; ses-snap 5;
index 33; undo 33; drc 17), the events compare is green (3,520
golden rows, exit 0 since M4-T6), determinism canaries are unrotated
(ses `dc271114…` / manifest `cf607714…`), and the in-tree
threads-invariance gate is green (bm06 SES `77caf2099a2f…` / manifest
`90f08ad2…` byte-identical across `-mt 1/3/4`). The CI router gate
stays in `--report-only`: the M6-T3 close adjudicated the flip
NO-GO (honest hold) on BOTH preconditions — the Tier A battery
verdict face reads 10 green / 1 red ≠ 11 green / 0 red (the
completion-only reading is 11/11; both faces named per the plan's
corrected precondition wording), and Σ Tier A 2089.5 s > the compare
step's 30-min budget (over by 289.5 s = 16.1%). Reopen bound: the
other ten Tier A walls sum 289.3 s — the flip reopens when bm01
completes < 1510.7 s; the named lever is deterministic candidate
parallelism (buglog 197), recorded not attempted. The bound does not
rise — the wall must drop (the rust-check.yml comment records the
adjudication). Threads-gate CI wiring lands WITH the flip
(re-deferred with cause: the preconditions failed at M6-T3).
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
# face; the M4-era face (T11/T12 — the CURRENT face is the status
# block's M5-T8 composition): the T11 full battery reads 8 pass / 3
# red (bm01 completion
# red by 1 at a 993.16 plateau; bm06 5-vs-2 + score, the bug-181
# victim-choice family; bm11 wall-integrity red with the engine AT
# Java's exact 975.00/3/0 face at the kill) — T12's face.
cargo run -q -p epic-harness --release -- router compare --profile full
# CI posture (M4-T13, refreshed at M5-T9, re-refreshed at M6-T4 to the
# M6-T3 hold): --report-only prints every verdict and exits 0. The
# battery is now TIER-AWARE (`--tier A|B|C`, default A byte-identical —
# the T3 chore commit 62c417a38) and the workspace census is 1,449
# passed / 0 failed / 17 ignored. The flip stays HELD after M6-T3 —
# both preconditions fail: the Tier A battery verdict face is 10 green
# / 1 red (bm01's 1800.2 s INTEGRITY-gate tier kill — routing complete
# 0/0/1000.00, optimizer P1 571.93 s REJECTED, incumbent restored; the
# completion-only reading is 11/11, both faces named) and Σ Tier A =
# 2089.5 s > the compare step's 30-min budget (over by 289.5 s =
# 16.1%). The per-tier BEFORE-face tables (A 10/1, B 4/9, C 1/3) live
# in logs/M6-T3/report-t3.md. REOPEN BOUND: the other ten Tier A walls
# sum 289.3 s — the flip reopens exactly when bm01's total completes
# < 1510.7 s. Named lever, recorded NOT attempted: deterministic
# candidate parallelism (buglog 197; Java DefaultSettings.java:182
# optimizer.maxThreads = cores−1 vs Rust's mandated 1-thread face).
# The bound does not rise; the wall must drop (the rust-check.yml
# comment records the bound-rise adjudication: NOT taken — bounds are
# never raised to fit walls, the same discipline as tiers.yaml
# timeouts).
#
# RESIDUALS LEDGER (the standing record): the B/C completion reds are
# the T8/T9 tuning population (bm05 29 vs 18, interf_u 1 vs 0, 1Bitsy
# 2 vs 1, bm04 3 vs 2); kill faces bm01 (Tier A) and bm10 / StickHub /
# LimeSDR (Tiers B/C); the FANOUT-FULL-FLOW GATE GAP — the events
# fixtures are capture replays that never exercise the changed fanout
# arm and the router-only/ses/det gates run fanout-disabled, so the
# buglog-181 fix carries ZERO byte-invariance coverage until a
# fanout-full-flow golden exists (guards meanwhile: the ci_tripwire
# pins + sanctioned faces + the batteries); the Σ-decomposition
# battery print is BANKED as a harness improvement for the first
# harness touch (not implemented in the M6-T4 docs-only task); the
# banked quality-r2 NITs (shared no-extras sweep helper, bail! idiom
# consistency, multi-concern walk pin) share that same first-touch
# owner; T5's pour worklist: 43 parse-time plane-induced rows on
# multichannel_mixer-unrouted / CM5_MINIMA_3 (the Java post-route
# real-walk instrument is T5's first item).
#
# EVIDENCE CONVENTION (the DNR-14 lesson; quality-r1 finding 6, closed
# in practice by the T3 fix round): gate evidence logs open with the
# exact invocation line, so the log alone proves the flags.
#
# B/C SUM COMPOSITION (spec finding 3 / NOTE-8): the battery's printed
# Σ includes the completing reds' detail-pass re-burns, so the
# per-fixture wall columns do NOT re-sum to it; Tier A (the
# adjudicated Σ) sums exactly.
#
# At the flip: drop --report-only, update BOTH ci_tripwire.rs pins in
# the SAME commit, add the events-compare step (java-free, green since
# M4-T6) with its own pin, and refresh the posture prose (this file's
# status block and this comment, plus the design doc §6 note's CI
# paragraph, SEAM posture rows); the rust-check.yml comment records
# the condition. The threads-gate CI wiring lands WITH the flip
# (re-deferred with cause at M6-T4: the flip preconditions failed at
# M6-T3).
cargo run -q -p epic-harness -- router compare --report-only
# Determinism self-gate: one DSN, two runs, byte-identical SES +
# manifest (canaries at the M5-T3 fix: ses dc271114… / manifest
# cf607714… — the M4-close values were 253e7b14… / 8c4a9773…, the
# M3-exit values 96ba7300… / 16714d5e…; the M5-T3 rotation is the
# sanctioned copper-to-edge movement (the canary rides bm08, whose
# outline gets the same fallback-class promotion as bm06), recorded
# in SEAM's T3 dossier). STALENESS has TWO
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
# Event-stream compare vs the committed Java probe golden:
# green since M4-T6 — 3,520 golden trace rows aligned, exit 0 (the #61
# classes closed: buglog 172-174). Not in CI — it rides the future flip
# commit with its own pin (see the CI posture comment above).
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
