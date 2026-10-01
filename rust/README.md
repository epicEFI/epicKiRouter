# EpicRouter Rust Workspace

Rust rewrite of the Freerouting autorouter (design:
`docs/superpowers/specs/2026-09-11-epicrouter-rust-rewrite-design.md`).
The Java tree (`src/`) was the frozen parity oracle through the rewrite —
it was sunsetted at M10-T4 (deleted; history preserved in git history and
the grafted `origin/master` at `e7f9bdf1`).

Status: **M10 CLOSED-ADJUDICATED — `M10_EXIT_WITH_DEVIATIONS`** (the
2026-10-01 terminal adjudication: all six exit criteria MET; the carry rows
ride post-2.0 with named owners) — the release posture: the Java sunset landed (T4: `src/`, `src_v19/`,
the gradle build, the Java CI faces deleted; the committed goldens are the
post-sunset parity record); workspace version **2.0.0** + the `--version`
face + the Linux release artifact
`epicrouter-2.0.0-x86_64-unknown-linux-gnu.tar.gz`
(sha256 `0d055d42fe00ab1f79e93d46beadd7b8b041fd2a4cac48e31983f43217ebe844`,
smoke-tested from the unpacked artifact) (T5); the migration guide landed
(`docs/migration-guide.md`, T6). Census **1690/0/19** at the M10 close,
additions only 1682 → 1690 (the ledger chain in the plan AMENDMENTS 1–7).
Canaries: ses `dc271114…`
IMMOVABLE throughout; manifest `c6644a91…` (the version-blind successor
since T5; the raw `cf607714…` retired there). The M10 VERDICT is
adjudicated: `M10_EXIT_WITH_DEVIATIONS` (design §6, 2026-10-01). Earlier milestone history:

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
board.dsn -do out.ses`); **1,629 workspace tests green**. M6
executed on top (the routing-intelligence family, ALL default-OFF —
at defaults every output stayed byte-identical, canaries unrotated):
floating-island detection (advisory `pour_islands` manifest face +
the region-level clamp behind `router.plane_island_clamp`), the
global-planning stage `epic-router/src/global/` (congestion map,
per-net guides + planned net order, pattern routing, PathFinder
history costs) behind `router.congestion_global` (+ `.pattern` /
`.pathfinder`), and push-and-shove insertion behind
`router.push_shove`. The Tier B/C AFTER-face (T10) measured the
milestone's strictly> criterion at 2/12 with the ALL-ON regime
net-harmful — the measured best-known regime is **default +
push_shove-only** (1-Wire-Wing 0<1, bm04 0<2, StickHub's standing
kill face flipped to completion at exact Java parity); Issue093-class
router-introduced violations = 0 on every completing pour fixture
(real counter, both faces), and island detection re-confirmed with
zero divergence. The M6-T3 battery re-faces — the
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
(ses `dc271114…` / manifest `c6644a91…` — the manifest digest is
version-blind since M10-T5: `app_version`/`git_sha` are normalized out
of the digest face, the raw-bytes `cf607714…` retired there), and the
in-tree
threads-invariance gate is green (bm06 SES `77caf2099a2f…` / manifest
`dbf7a6f9…` byte-identical across `-mt 1/3/4` — the manifest digest is
version-blind since M10-T5; the raw `90f08ad2…` literal is no longer
reproducible at 2.0.0). The CI router gate
FLIPPED at M8-T8 (2026-09-29, tree `9fe3a0751`): the exact post-flip
CI face (router-only profile, mode: gate) reads 11 green / 0 red at
Σ 286.5 s — 5.3–6.3× headroom under the compare step's 30-min budget
(DNR-18-reconciled, `logs/M8-T8/evidence/80-gate-laps.log`); strip
context: the M6-T3 reopen frame's full-flow Σ with the T7 lever
(bm01 ot=4 1340.78 s + the 285.7 s strip = 1626.5 s) also fits — the <!-- codespell:ignore (ot=4 is the bm01 lever name) -->
lever is opt-in and NOT load-bearing for the CI step (the router-only
profile never engages the optimizer). The four prior holds (M4 close,
M5-T8, M6-T3, M7) stand as history. The bound does not
rise — the wall must drop (the rust-check.yml comment records the
adjudication). The threads-gate CI wiring did NOT land with the
flip (M8-T8): it stays in-tree-only (harness `router
threads-invariance`), re-owned to the next workflow-touching act. M7
executed on top (the tuning family, INPUT-DRIVEN — a DSN
`(circuit (length …))` declaration activates; constraint-free input
routes byte-identically at defaults, canaries unrotated): the
length-constraint model + min-length honoring gate (`epic-board`
`net_class_length_bounds` / `min_length_gate_allows`), clearance-aware
meander insertion (`epic-router/src/pipeline/tuning.rs`), class-
constraint length matching (match-to-target with meander budget), and
differential pairs (`pipeline/pairs.rs`, declared via
`router.tuning.pairs` — the KiCad DSN export declares none). The
tuning population is the six committed fixtures under
`harness/fixtures/tuning/` (+ the two pair goldens), measured DRC-clean
at the REAL counter (ri=0 on all six, T7); Tier A held 11/11
row-for-row (10 green / 1 red completion-only); census 1547/0/17
(the M7-exit census; 1629 at the M8 flip), additions only; the flip reopen bound is UNCHANGED (canonical 1510.7 s;
the T7 this-run pair 1800.1 s kill / 333.0 s strip / 1467.0 s bound).
M8 executed on top (the gloss family, ALL default-OFF — at defaults
every output stayed byte-identical, canaries unrotated): the
four-metric aesthetics measurer (`epic-board/src/aesthetics.rs` —
`aesthetics_metrics` :238; the two doors are the `--dump-aesthetics`
sidecar on `epic-cli/src/route.rs` — sidecar ONLY, never into the
manifest, the canary-rotation trap named in the code — and the
java-free harness `aesthetics --dsn` reference door) over the
committed 21-board PCBench sample (`harness/fixtures/aesthetics/`:
`sample.yaml` + 24 reference goldens + `select.py`); and the gloss
stage family (`epic-router/src/pipeline/gloss.rs` — the bus
detector/hug-spread pass, the POST-tightener 45° flow pass, the
return-path-aware via-placement pass, and the graded-wire teardrop
pass; slotted post-tuning in `pipeline/full.rs`) behind
`router.gloss.bus` / `.flow` / `.via_place` / `.teardrops`. The T8
measurement record: the two criterion-1 regression findings, the
via-density monotone violation, and the artifact-dominated bend
improvement ALL own to the teardrop tapers' added routed length
(flow-alone an exact no-op on the sample corpus), while criterion 2
(no completion/DRC regression) held clean (20/20 grid, Tier A 10+1
held, canaries unrotated, census 1629/0/17); the CI flip LANDED at
M8-T8 (recorded above); the B/C re-faces are the deviations refresh in
design §6 M8 exit note and `logs/M8-T8/report-t8.md`.
Status: **M9_EXIT_WITH_DEVIATIONS** — design §6 M9 exit note (adjudicated
2026-09-30 at `66f563cf5`: criteria 2/3/4/5 MET; criterion 1 MET WITH
FINDINGS — the interactive workflow is byte-identical to the CLI on the
17/17 battery, but the demonstrated cancel is routing-stage-only (the
mid-optimization cancel structurally ineffective on the flagless
optimizer-stage `StopFace`, pre-existing, carried M10 as a wiring
change); census 1682/0/19, additions only 1629→1682; the seven
compares, events 3,520, and the determinism canaries all unrotated),
`logs/M9-T7/report-t7.md` (the measurement record),
`logs/M9-T8/report-t8.md` (the close-out), `logs/M9-ADJUDICATION/` (the
adjudication), plan AMENDMENTS 1–8. The M8 row: **M8_EXIT_WITH_DEVIATIONS** — design §6 M8 exit note
(adjudicated 2026-09-29 at `277ee7970`: criterion 1 NOT MET on the
committed four-metric instrument — the teardrop-owned regression
findings; criteria 2/3/5 MET, 4 discharged; the CI flip SUSTAINED at
`ba5e587bb`), `logs/M8-T8/report-t8.md`, `logs/M8-ADJUDICATION/`,
the M8 criterion map in SEAM.md. The M7 row:
**M7_EXIT_WITH_DEVIATIONS** (design §6 M7
exit note, `logs/M7-T7/report-t7.md`, the M7 criterion map in
SEAM.md).
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
  33/33 snapshot/undo replay parity vs the Java oracle; M6: the
  floating-island detector `src/islands.rs` — advisory `pour_islands`
  manifest face, plus the region-seeded clamp helper behind
  `router.plane_island_clamp`)
- `crates/epic-index` — incremental spatial index (M2 done: ShapeTree +
  MinAreaTree with tree-shape precalc and compensation — 33/33
  query-set parity vs the Java oracle)
- `crates/epic-drc` — exact incremental DRC (M3 done: incompletes +
  clearance violations vs the Java oracle, 17-fixture compare; the
  full incremental-DRC engine remains future work — it was not an M6
  deliverable)
- `crates/epic-router` — routing core (M3: the detail-routing core —
  expansion → maze → ripup/shove → locator → forced insertion → batch
  driver; M4: the fanout stage, the batch optimizer, and the
  full-pipeline assembly `pipeline/full.rs`; M6: the global-planning
  stage `src/global/` — congestion map, per-net guides + planned net
  order, pattern routing, PathFinder history costs — behind
  `router.congestion_global` (+ `.pattern`/`.pathfinder`), plus the
  push-and-shove insertion arm behind `router.push_shove`, all
  default-OFF; `SEAM.md` in the crate is the port ledger)
- `crates/epic-engine` — the headless application core (M9 done: the
  layered settings face — defaults→DSN→CLI→session, the Java
  `SettingsMerger` precedence — moved here from epic-cli at M9-T1; the
  `Session` load→settings→route→cancel→export workflow, CLI-path parity
  pinned by SES byte-identity; the versioned `EngineEvent` stream +
  `TeeDriverSink` (the parity trace stream forwarded unchanged) +
  `BoardSnapshot` with the overlay data faces — ratsnest/DRC
  markers/congestion/tuning). The CLI never constructs the session or
  the tee — default-path-inert by construction
- `crates/epic-cli` — `epicrouter` CLI (M3 done: headless `route`,
  manifest + SES + exit-code semantics)
- `crates/epic-gui` — the headless view core (M9-T4/T5: the pure
  `ViewModel -> RenderList` projection + committed goldens, always
  compiled) + the desktop shell (M9-T6: eframe 0.35 + rfd + png
  behind the DEFAULT-OFF `desktop` feature — a thin host: worker
  thread over the engine `Session`, canvas, panels, dialogs; NO
  geometry logic). CI posture: compile-gated ONLY
  (`cargo clippy -p epic-gui --all-targets --features desktop --
  -D warnings`, tripwire-pinned) — the default workspace build stays
  egui/wgpu-free, and the shell smokes are dev-box evidence
  (`--smoke` / `--cancel-smoke` on `DISPLAY=:1`, PNG + logs), never
  gates
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
# CI posture (M4-T13, refreshed M5-T9/M6-T4; FLIPPED at M8-T8 — the
# compare step now runs GATE mode and exit is a hard gate; the four
# prior holds are history): the
# battery is now TIER-AWARE (`--tier A|B|C`, default A byte-identical —
# the T3 chore commit 62c417a38) and the workspace census is 1,629
# passed / 0 failed / 17 ignored. The flip LANDED at M8-T8 —
# the post-flip CI face (router-only, gate) is 11 green / 0 red at
# Σ 286.5 s (arithmetic in the status block and the rust-check.yml
# comment; DNR-18-reconciled). The full-profile held face HELD:
# 10 green / 1 red bm01 (mode: report, exit 0), and in mode: gate the
# same bm01 tier-kill reds it (exit 1) — the deterministic budgets do
# NOT preflight-skip bm01's optimizer; the full profile is NOT the CI
# step's face. The pre-flip M6-T3 hold record (10 green / 1 red,
# Σ 2089.5 s > 1800 s, reopen bound 1510.7 s) lives in the git
# history of this file and logs/M6-T3/. The per-tier BEFORE-face
# tables (A 10 green / 1 red, B 4/5, C 1/2 green/red) live
# in logs/M6-T3/report-t3.md; the M8-T8 re-faces (B 4/5 both faces,
# C 1/2 both faces, gloss-ON row-for-row identical) live in
# logs/M8-T8/.
#
# RESIDUALS LEDGER (the standing record; the B/C faces are the T10
# AFTER-face unless noted): the B/C completion reds after the
# sanctioned tuning are bm05 27 vs 18, bm10 3 vs 1 (+ the one
# router-introduced violation, ri=1, flag-independent, buglog 205,
# owner M7), interf_u 3 vs 0, and the 1Bitsy / CM5 / LimeSDR tier
# kills, with mm / mm-u exact-parity floors (byte-stable across flag
# faces — engine-capability gaps, not settings gaps); kill faces
# bm01 (Tier A) and bm10 / StickHub / LimeSDR (Tiers B/C); the FANOUT-FULL-FLOW GATE GAP — the events
# fixtures are capture replays that never exercise the changed fanout
# arm and the router-only/ses/det gates run fanout-disabled, so the
# buglog-181 fix carries ZERO byte-invariance coverage until a
# fanout-full-flow golden exists (guards meanwhile: the ci_tripwire
# pins + sanctioned faces + the batteries); the Σ-decomposition
# battery print is BANKED as a harness improvement for the first
# harness touch (not implemented in the M6-T4 docs-only task); the
# banked quality-r2 NITs (shared no-extras sweep helper, bail! idiom
# consistency, multi-concern walk pin) share that same first-touch
# owner; T5's pour worklist CLOSED (M6-T5, zero divergences): the
# end-to-end `contains_plane` audit found every Java plane face already
# ported Java-exactly (connection_router.rs:113-158, batch.rs:1105-1116,
# contacts.rs stop_at_plane, drill isDrillable, fanout escape arms) and
# the NEW Java post-route real-walk instrument (git-ignored scratch:
# logs/M6-T5/scratch/PostRoutePlaneOracle.java; production headless
# flow + getAllClearanceViolations) shows Java's own router introduces
# ZERO violations on all 10 pour fixtures (row-level: post == parse on
# all 332 rows) — the Issue093-class router-introduced face is 0 on
# BOTH engines, the plan-level STOP fork never fired; Rust re-runs of
# the pour population are byte-identical (9/9 SES sha equal, StickHub
# killed at the same 900.1 s wall); the 43 parse-time plane-induced
# rows (mixer-unrouted 30, CM5_MINIMA_3 13) are input-board conditions,
# identical in both engines, and stay the recorded contrast face (NOT
# drivable to 0 without editing input boards; Java's post-route
# StickHub walk: 1 violation, 0 introduced — Rust's kill face is the
# wall, not a divergence). Pour-fragmentation AVOIDANCE is beyond-Java
# (upstream has no split/island face at all); the T6 DETECTOR half of
# the design :14/:72 work is DONE (M6-T6): `epic-board/src/islands.rs`
# partitions each filled pour into 4-connected metal regions over a
# 1-unit integer lattice (foreign copper carves; same-net pins/vias/
# traces seed; boundary constant MIN_BRIDGE_WIDTH=1 mutation-verified
# BOTH directions on the `island-spike` crafted worlds) and reports
# region/island counts + a SHA-256 geometry digest as an ADDITIVE
# ADVISORY manifest face (`pour_islands`, emitted only for boards with
# pours — pour-free manifests keep their bytes, so the determinism
# canaries are UNROTATED: ses dc271114… / manifest cf607714… verified
# post-change). Parse-time population face (10 pour fixtures):
# ecc83-pp 3 regions/2 islands, ecc83-pp_v2 5/4, multichannel_mixer
# GND 3/2 (+ a fully-covered vbias pour), CM5_MINIMA_3 18 pours
# (1 island, +5V layer 3), the remaining pours clean — NO Java
# contrast exists (upstream detects nothing; this is the milestone's
# first beyond-Java capability, no oracle, NOT parity-checked by
# construction). The
# connectivity CLAMP half rides `router.plane_island_clamp` (Rust-only,
# default OFF, dead code at defaults; the settings-ON gate seam +
# smoke pin landed, its route effect is T8/T9 work).
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
# EXECUTED AT THE FLIP (M8-T8, one commit): --report-only dropped,
# both ci_tripwire.rs pin faces updated (the compare-step pin now pins
# the exact GATE argv; the events step pinned inside the same test fn
# — census unchanged), the events-compare step added with a 10-minute
# wall, and the posture prose refreshed (this file's status block and
# this comment, the design doc §6 note's CI paragraph, the SEAM
# posture row, docs/architecture.md). The threads-gate CI wiring
# LANDED at M9-T6 (2026-09-30): `cargo run -q -p epic-harness
# --release -- router threads-invariance` joins the router steps with
# its own 30-minute wall, and the default-off `desktop` shell gained
# its compile-gate step (desktop-clippy, 30 min) — both argv
# tripwire-pinned in ci_tripwire.rs.
cargo run -q -p epic-harness -- router compare
# Determinism self-gate: one DSN, two runs, byte-identical SES +
# manifest (canaries at the M5-T3 fix: ses dc271114… / manifest
# cf607714… — the M4-close values were 253e7b14… / 8c4a9773…, the
# M3-exit values 96ba7300… / 16714d5e…; the M5-T3 rotation is the
# sanctioned copper-to-edge movement (the canary rides bm08, whose
# outline gets the same fallback-class promotion as bm06), recorded
# in SEAM's T3 dossier; the M10-T5 handoff made the manifest face
# VERSION-BLIND (app_version/git_sha normalized out of the digest —
# a release bump can never rotate it) and retired the raw
# cf607714… for c6644a91…; ses dc271114… immovable throughout). STALENESS has TWO
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
