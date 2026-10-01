# EpicRouter M4 — Full Pipeline (Fanout + Detail Optimizer + Batch Optimizer) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Port the Java full routing pipeline — fanout stage, changed-area detail optimizer (TraceTightener + ViaOptimizer), and the board-level BatchOptimizer — and close the M3 event-stream divergence (#61), reaching the design §6 M4 exit: **zero-regression moment — all fixtures ≥ Java on completion/violations/score**, then flip the CI compare gate.

**Architecture:** The pipeline stays in `epic-router::pipeline` (where `BatchDriver`, `BoardHistory`, `BoardStatistics`, `DriverSink` already live — `epic-engine` remains the jobs/settings scaffold per its design §4.1 charter; do NOT move the pipeline there). Fanout fills the reserved seam at `pipeline/batch.rs:384-397`; the detail optimizer fills the documented `PullTightSeam` break (`epic-board/src/routing_board_insert.rs:152`, planned widening :160-166) at its two production call sites (`pipeline/connection_router.rs:196,:212`); the BatchOptimizer is a new `pipeline/optimizer.rs` sibling to `batch.rs`, invoked from `epic-cli` step 4→5. Java byte-parity remains impossible by design (Java is wall-clock nondeterministic); parity is proven directionally vs re-captured full-pipeline Java baselines, by the determinism self-gate, and by the events micro-corpus reaching exit 0 (#61).

**Tech Stack:** Rust workspace in `rust/` (run cargo from there); Java oracle via `rust/harness` (JDK 25 at `~/.jdks/jdk-25.0.4.1+1`); branch `epic/main`; NEVER push; NEVER modify Java under `src/`/`src_v19/` (reading fine; `rust/harness/oracle/*.java` is harness-side/editable); committed artifacts (`rust/harness/baselines/**`, corpus dirs, events-golden sha `cc50216c…`, `tiers.yaml`) never modified — baseline ADDITIONS via `epic-harness capture` with commit explanation are the sanctioned regeneration path.

**Recon basis:** controller-commissioned recons of the Java M4 surface (`autoroute/pipeline/`, `board/optimize/`, `settings/`, `core/scoring/`; 2026-09-22) and the Rust current surface (2026-09-22). All anchors are recon-verified but implementers RE-VERIFY against the jar/sources — **the Java wins every conflict, including with this plan**.

**Key recon findings baked in (Java):**
- The optimizer is TWO layers: (a) changed-area cleanup `RoutingBoard.optChangedArea` → `TraceTightener.getInstance(angle).optChangedArea(traceCosts)` (`RoutingBoard.java:151-190`, `RoutingBoardOperations.java:52-79`), called after EVERY connection, after tail removal, after each fanout escape — NOT gated by optimizer settings; (b) the board-level `BatchOptimizer` (1,183 l) rip-and-reroute stage.
- Fanout is split: `BatchFanout` (800 l) + `RoutingBoard.fanout` (`RoutingBoard.java:978-1110`) + maze-side arms (`MazeSearchEngine.java:88-117` TreeSet add-override escape-length rejection, `:360-368` first-drill termination, `:988-995` no destination distance; `MazeRipupResolver.java:59-62` fanout cost factor). Ordering fully deterministic (component TreeSet more-SMD-first/id; pins `outer_first`/`inner_first` by SMD gravity-center distance, tie `pinIndex`).
- Java randomness: only `BatchAutorouter.java:135` `Random(0)` (multi-thread shuffle — dead for us) and `MazeSearchEngine` `Random` reseeded with `ctrl.ripupCosts` (already ported, T7). Geometry seeds 99 already ported (M3-T2).
- BatchOptimizer acceptance gates (`optimizerCandidateRejectionReason` :565-581): connectivity regression → reject; DRC-count regression → reject; score not strictly improved → reject + restore incumbent. ONE winning candidate applied per pass; improvement-threshold stop (default 2.5%); `maxConsecutiveFailures` 12 (pass 1) / 50.
- Optimizer score V2_LOWER_BOUND (`BoardStatistics.java:808-835`): `max(0, 1000 − lengthPenalty − viaPenalty − bendPenalty)` vs lower bounds (`minTraceLengthMm`, `minViaCount`, `minBendCount`) + difficulty (`ensureDifficulty` :743-778).
- Java's BatchOptimizer thread pool makes ITS output run-to-run nondeterministic; our posture (as with wall-nondeterminism in M3) is: Rust sequential-deterministic port, Java baseline = one captured sample, gate directional with the existing relative-ε idiom.

## Milestone exit criteria (design §6 M4)

1. **Full pipeline runs on the fixture set**: fanout → batch passes → optimizer, end to end through `epic-cli route` with per-phase manifest rows.
2. **Zero-regression moment**: every Tier A fixture ≥ Java on completion / violations / score vs FULL-PIPELINE Java baselines (battery 11/11), with the M3 reds closed or root-caused inside this milestone: bm06/bm11 via #61-then-tuning (buglog 181), bm01 via per-pass search cost (buglog 175; M5 as backstop — if bm01 remains wall-limited at milestone end, that is a NOT_MET-clean exit face, not a silent pass).
3. **Gates green; zero regressions**: all seven M3 compares stay green (corpus 5000/5000, dsn 1332, ses 20, ses-snap 5, index 33, undo 33, drc 17); `events compare` reaches **exit 0** (#61 closed); determinism self-gate holds (digest VALUES may rotate when id alignment lands — the byte-identical-across-runs property is the gate; rotate SEAM citations at that commit); census additions only.

**Carry-forwards baked into tasks:** #61 Classes A/B/C (T1/T2/T6 — Class C is expected to be entangled with pull-tight: Java's golden `compare_trace_route_item` netItems faces were produced WITH `optChangedArea` active while Rust ran `NoPullTight`; so #61's exit-0 lands at T6, after the tightener); CI flip one-commit protocol (T13); stale rust-check.yml compare-step comment refreshed at the FIRST workflow-touching commit (none planned before T13); the four cut pub items return with first consumer, pinned then; banked inert mutants hoist-if-third-copy opportunistically.

**Wall→tick rule (recurring):** every Java wall-clock limit entering M4 scope (fanout `maxMillisecondsPerPin·(passNo+1)`, pull-tight 1000–2000 ms limits, optimizer per-candidate deadline) gets a deterministic call-tick analog behind `settings.deterministic_budgets` following the T12 `RouteBudget` pattern (`engine.rs:256`), pinned to the Java multiplication formula; wall faces remain for interactive/unbounded runs only.

---

## Task 1: Class A — parse-time item-id assignment alignment (buglog 172)

**Files:** Modify `rust/crates/epic-board/src/` (the `Board::from_ses_board` id-assignment path — locate via buglog 172 / `openwolf find from_ses_board`); pins in `rust/crates/epic-board/src/` + one events-side pin in `rust/harness/src/route_events.rs` ONLY if a witness row needs harness-side assertion (prefer engine-side pins).

- [ ] Root-cause the order: diff `Board::from_ses_board` id allocation against Java `DsnReader` + `ItemIdGenerator` (read the Java parse path; the buglog witness: t7_ripup assign ordinal 3 seeds door item=10 (Rust) where Java seeds item=13, same geometry `bounds (578750,338750)..(611250,361250)`, Java never references ids 10/25). Port the Java order exactly; RE-VERIFY with a fresh jar id-dump probe on the t7 world if the order is not obvious from source.
- [ ] Pins: an id-sequence pin asserting the Rust parse assigns ids in the Java order on ≥2 corpus fixtures (witness: jar dump or the events golden itself); keep the M2-corpura invariance in mind — index/undo corpora byte-equality MUST stay green (they ride the `read_board` path, not `from_ses_board` — verify that claim before relying on it; if WRONG and the corpora shift, STOP and report — that is a plan-level decision point).
- [ ] Ripple verification (mandatory, report each): `events compare` (t7 face should improve: 13 shared ids already byte-identical; expect the item=10/25 phantom doors gone); all seven compares; determinism digests (rotation EXPECTED — ids feed the descending-id seed walk `batch.rs:814`, so routes may shift; update digest citations at this commit and say so); battery CHEAP subset (shape may move — record, do not chase).
- [ ] Gates + ONE commit `fix(m4): align from_ses_board item-id order with DsnReader/ItemIdGenerator (events Class A, buglog 172)`.

## Task 2: Class B — expansion-room door-slicing alignment + causal-ancestor checkpoint (buglog 173)

**Files:** Modify `rust/crates/epic-router/src/expansion/` (`door.rs` `shape_between` :121 / `get_section_segments` :137; `room.rs`, `neighbours.rs`, `neighbours_forty_five.rs`); tests in the same tree.

- [ ] Diff Java `ExpansionRoom.calculateExpansionRooms` door partitioning against the Rust constructors for the three witnessed faces (buglog 173): e1_ripup ordinal 58 — same wall, Java 7 sections spanning `(216250,238750)..(983750,238750)` vs Rust 4 spanning `(595340,238750)..(983750,238750)`; t9_locator45 ordinal 1350 — same `ExpansionDrill` origin `(365121,318813)`, different wall door (Java `x=475440 y2=428938` vs Rust `x=313495 y2=429531`); seed enumeration order (Java doors 11,12,13,15,18 vs Rust 10,11,12,13). Find the partitioning rule difference (sub-range vs full-wall slicing is the signature of a missing section-splitting step); port it Java-exactly.
- [ ] Pins: door-section count + bounds pins on the two witnessed walls (craft minimal worlds if the fixtures are heavy; the events golden rows are the witness); enumeration-order pin for the seed sequence.
- [ ] `events compare` — expect e1/t9 faces to move; triage what remains (Class C rows are T6 scope; pull-tight-entangled rows also T6).
- [ ] **Causal-ancestor checkpoint (the bm06/bm11 hypothesis test)**: re-run bm06 and bm11 as single-fixture RELEASE runs (permitted: release profile, external `timeout`, ~88 s and ~1,089 s walls) — record whether either completion red flips or moves. Either outcome is evidence: record it in SEAM + buglog 181 (the hypothesis in the design §6 M3 note is now tested, not assumed). Do NOT tune tie-breaks in this task regardless of outcome.
- [ ] Gates + ONE commit `fix(m4): expansion-room door slicing parity (events Class B, buglog 173) + ancestor-checkpoint evidence`.

## Task 3: TraceTightener base + 90° variant (detail optimizer, layer A)

**Files:** Create `rust/crates/epic-board/src/trace_tightener/` (`mod.rs`, `tightener90.rs`); modify `rust/crates/epic-board/src/routing_board_insert.rs` (`PullTightSeam` widening per the documented planned break :160-166 — add `trace_costs`, time-limit/stop face); swap the two `NoPullTight` production call sites `rust/crates/epic-router/src/pipeline/connection_router.rs:196,:212`.

- [ ] Port `TraceTightener.java` (547 l): `getInstance` angle dispatch (`:87-107`), `optChangedArea(traceCosts)` fixpoint (`:119-168` — `while somethingChanged` over layers × regions enlarged by `1.5*(maxClearance + 2*maxTraceHalfWidth)`), `PolylineTrace → pullTight` vs `smoothenEndCornersAtTrace` dispatch, per-`Via` `ViaOptimizer.optViaLocation` hook (no-op until T5 — seam only), acid-trap avoidance flag (`:516`).
- [ ] Port `TraceTightener90.java` (169 l) including `avoidAcidTraps`. The 45° variant is T4 — if `getInstance` would select 45° on a fixture, land the dispatch with a `todo`-free fallback: select 90 only when the board restriction is orthogonal; 45° boards keep `NoPullTight` until T4 (state this in SEAM; the dispatch pin asserts which variant each tier fixture selects).
- [ ] Angle-restriction census (cheap, informs T4 scope): grep the tier fixtures' rule sections for trace angle restrictions; report which of Tier A/B/C use 90°/45°/any-angle. Any-angle boards: scope decision recorded for T4 (port vs defer-with-stub).
- [ ] Widening break: `PullTightAlgo` gains the tightener's real parameters; both call sites pass the real tightener; `NoPullTight` stays for tests/oracle-only paths. Deterministic budget face for the pull-tight time limit (wall→tick rule).
- [ ] Pins: fixpoint-termination pin (a world where one more iteration changes nothing — guard against infinite loops AND against early exit); region-enlargement arithmetic pin; jar-witnessed before/after geometry on ≥2 crafted traces (acid-trap arm included); the `RouterCounters`/event faces must NOT regress (`compare_trace_opt_changed_area_before/after` id-watermark rows at `connection_router.rs:212` — their semantics may now be non-empty; re-triage the events corpus honestly if rows shift; Class C close stays T6).
- [ ] Ripple: all seven compares; determinism digests (rotation EXPECTED — pull-tight changes post-route geometry); battery CHEAP (record movement; the bm01 lever prediction is pass walls SHRINK — measure Σ pass walls on one bm01-style fixture cheaply or note it deferred to T12).
- [ ] Gates + ONE commit `feat(m4): TraceTightener base + 90° variant behind the widened PullTightSeam (detail optimizer layer A)`.

## Task 4: TraceTightener45 (+ any-angle scope decision)

**Files:** Create `rust/crates/epic-board/src/trace_tightener/tightener45.rs` (and `tightener_any_angle.rs` ONLY if the T3 census found any-angle tier fixtures); tests alongside.

- [ ] Port `TraceTightener45.java` (674 l) — diagonal repositioning variant; pin each translation arm against jar-witnessed geometry (this is the highest-parity-risk file of the milestone: diagonal translation on integer coordinates is where tie-boundary bugs live — see cerebrum mode 11; pin AT the tie boundaries specifically).
- [ ] If any-angle deferred: land a pinned stub asserting which fixtures select it (so a future fixture flip fails loudly, not silently), and record the deferral in SEAM with the fixture census as its boundary.
- [ ] Ripple: as T3 (compares, digests — rotation possible, battery CHEAP record).
- [ ] Gates + ONE commit `feat(m4): TraceTightener45 — diagonal pull-tight (detail optimizer layer A)`.

## Task 5: ViaOptimizer (via pull-tight)

**Files:** Create `rust/crates/epic-board/src/trace_tightener/via_optimizer.rs`; consumes the T10b `drill_item_mover.rs` substrate; wire the T3 seam hook.

- [ ] Port `ViaOptimizer.java` (733 l): `optViaLocation` `:34-90` (via connected to ≤2 traces → relocate to reduce layer-trace cost; recursion depth 10; `DrillItemMover` insertion), `optPlaneOrFanoutVia` — scope the PLANE face to what a non-plane-aware engine can honor (plane contacts may not exist yet — M6 scope; land the fanout-via face, stub+pin the plane face with a loud boundary note if unexercisable on tier fixtures).
- [ ] Pins: recursion-depth boundary (cerebrum mode 12 — pin at a budget the production caller actually passes); relocation-rejects arm (cost goes up → stay); jar-witnessed via relocations on ≥2 crafted worlds; the ≤2-traces gate (3-trace vias untouched).
- [ ] Ripple: standard (compares, digests, battery CHEAP record).
- [ ] Gates + ONE commit `feat(m4): ViaOptimizer — via pull-tight behind optChangedArea (detail optimizer layer A)`.

## Task 6: Class C close + #61 exit (events compare → exit 0)

**Files:** Modify the insert-path id-allocation sites surfaced by the re-triage (`rust/crates/epic-router/src/path/inserter.rs` and/or `epic-board` insertion undo — locus decided by the triage, not pre-committed); `rust/harness/src/route_events.rs` only if compare diagnostics need a new face.

- [ ] With pull-tight now live, re-triage ALL remaining events divergences. Buglog 174's faces: `netItems`/`maxItemId` churn (e1 62 vs 390, t7 26 vs 24, t9 130 vs 122) and the t7 route-slot COUNT drift (golden 7 vs Rust 8 — Rust's extra attempt row 6 `result=ROUTED` vs golden `NO_UNCONNECTED_NETS`). Separate id-allocation-order residue from pull-tight-geometry effects (the golden was produced WITH Java pull-tight active — after T3/T4/T5 Rust geometry should CONVERGE toward the golden, not drift; any remaining divergence is allocation order or attempt-order).
- [ ] Port the alignment (Java insert-path allocation order); the attempt-count face (t7 slot 8) if it survives is a search-order face — fix only if it is allocation-induced; otherwise classify honestly (it may be the same family as buglog 181's attempt-42 fork — record, don't force).
- [ ] **Exit gate: `events compare` exits 0 on all three worlds.** If a face cannot close without tuning (victim-choice family), STOP and report — #61's exit criterion then needs a milestone-level adjudication (honest partial close vs deferred face), do not paper over it.
- [ ] Update the events-corpus CI posture note (SEAM T16 row: "deliberately not in CI until they close" — with exit 0, ADDING the events compare to CI is a T13 flip-adjacent decision; recommend in SEAM, flip only at T13).
- [ ] Gates + ONE commit `fix(m4): events Class C close — insert-path id allocation + attempt face (buglog 174); events compare exit 0 (#61 CLOSED)`.

## Task 7: Fanout stage

**Files:** Create `rust/crates/epic-router/src/pipeline/fanout.rs`; modify `pipeline/batch.rs` (the reserved seam :384-397; `RouterCounters::fanout_extra_vias_count`), `maze/search_engine.rs` + `maze/list_element.rs` (fanout arms), `maze/ripup.rs` (fanout cost factor), `epic-board` (`RoutingBoard.fanout` equivalent + `smd_pin_count` consumers), `epic-cli/src/route.rs` (`ManifestPhases.fanout` fill + phase-filtered `passes_completed` backfill — the SEAM T13 bank at SEAM.md:583), settings (`FanoutSettings` group into `RouterSettingsIr` + CLI flags + validate).

- [ ] `BatchFanout` port (800 l): `fanoutBoard` loop (maxPasses default 20, break on `routedCount==0`, oscillation detector `(routedCount<<32)^viaCount` 3-identical-states stop, full-board-hash repeat stop, maxItems cap); `fanoutPass` (ripupCosts `= start·(passNo+1)`, `-1` when `ripupAllowed==false`; per-pin budget `maxMillisecondsPerPin·(passNo+1)` default 10000 — wall→tick rule; no-via-rule pin skip + `fallbackToBoardVias` append; strict-DRC revert on success `BatchFanout.java:287-304`; counters ROUTED/ALREADY_CONNECTED/FAILED/INSERT_ERROR).
- [ ] Ordering EXACTLY (determinism-critical): components in TreeSet order (more SMD pins first, tie component id — `:702-713`); pins by `pinSortingOrder` (`outer_first` default / `inner_first`) on distance to the component's SMD gravity center, then `distanceToClosestOnNet`, then `surroundingsDensity` (pins within 20 mm), tie `pinIndex` (`:761-797`).
- [ ] `RoutingBoard.fanout` port (`RoutingBoard.java:978-1110`): SMD-only single-layer single-net guard; unconnected targets sorted by distance to pin center; `AutorouteControl` `is_fanout=true` + via-rule append; `remove_unconnected_vias=false`; ≤4 targets closest-first else whole set; ROUTED → `opt_changed_area` (real tightener, T3+).
- [ ] Maze arms: `MazeListElement` frontier rejection by `maxEscapeLengthMm` (default 2.5 mm, coord fallback 3000) on the start layer + `ExpansionDrill` rejection by `minEscapeLengthMm` (2.5 mm, fallback 500); first-drill termination `:360-368`; no destination distance in fanout mode `:988-995`; `MazeRipupResolver` fanout cost factor `(halfWidth/length)^2 · FANOUT_COST_CONSTANT` `:59-62`.
- [ ] `remove_unconnected_vias` interaction: pass-end tail removal preserves fanout vias (`FANOUT_VIA` stop option) — already derived `!fanout_enabled` in `build_batch_settings`; the one-time stagnation fanout-recovery (`batch.rs:597-630` exists) now becomes live — pin its firing on a crafted stagnation world.
- [ ] Pins: ordering pins (gravity-center tie, pinIndex tie); oscillation-detector pin (3 identical states → stop, 2 → continue); escape-length boundary pins (at exactly min/max — cerebrum mode 11 tie discipline); strict-DRC revert pin (a fanout success that carries a violation is rolled back); counters pin vs jar on ≥2 tier fixtures.
- [ ] Ripple: events corpus is router-only (fanout off in its `SettingsWitness`) — must stay exit-status-stable post-T6; all compares; digests (rotation EXPECTED — new stage); battery CHEAP + one bm06-class release single-fixture (fanout changes P1 seeds — record, don't chase).
- [ ] Gates + ONE commit `feat(m4): fanout stage — BatchFanout + RoutingBoard.fanout + maze escape arms (deterministic)`.

## Task 8: Optimizer score V2_LOWER_BOUND + lower bounds

**Files:** Modify `rust/crates/epic-router/src/pipeline/board_statistics.rs` (extend the existing `BoardStatistics`); tests alongside.

- [ ] Port `getOptimizerScore` V2 (`BoardStatistics.java:797-835`): the three penalties with weights 1000/2000/500, floors `lengthFloor` 1.0 / `difficultyScaleFloor` 1.0; the lower-bounds computation (`this.bounds` — `minTraceLengthMm`, `minViaCount`, `minBendCount`; recon-inside how Java derives them — MST/airline faces; port exactly) + `ensureDifficulty` `:743-778`.
- [ ] Settings: `OptimizerScoreSettings` group (version, excess weights, floors) into the settings tree with the merger invariant (nullable, no initializers — CLAUDE.md).
- [ ] Pins: penalty arithmetic on crafted boards (each penalty at 0, at the floor, above); bounds pins vs jar on ≥2 tier fixtures (bounds are deterministic parse/ratsnest faces — exact equality pins, not ε); the `max(0, …)` clamp arm.
- [ ] Gates + ONE commit `feat(m4): optimizer score V2_LOWER_BOUND + lower bounds + difficulty (parity-pinned)`.

## Task 9: BatchOptimizer stage

**Files:** Create `rust/crates/epic-router/src/pipeline/optimizer.rs` (+ `optimizer/candidate.rs` if size demands); modify `epic-cli/src/route.rs` (step 4→5 invocation), settings (`OptimizerSettings` group + flags + validate).

- [ ] Preflight guards (`evaluatePreFlightGuards` :155-219): incompletes > 0 → skip; no vias + score ≥ 950 or length within 5% of min → skip; score ≥ 995 → skip; length within 2% + vias ≤ min → skip; all vias mandatory SMD layer transitions (`:225-275`) → skip; `enable_preflight_guards=false` bypass. Pin EACH guard arm + the bypass.
- [ ] Pass loop (`runBatchLoop` :278-547): incumbent = deep-copy snapshot (Rust: the `BoardHistory` snapshot machinery); `withPreferredDirections = (pass % 2 != 0)`; `optRoutePass`; acceptance = the three rejection gates (`:565-581`, connectivity/DRC/score-not-strictly-improved) + restore incumbent on reject; stop when relative improvement < threshold (default 2.5%, fraction < 0.1 auto-×100 sanitation `:350-373`); `useIncreasedRipupCosts` drop after non-improving pass.
- [ ] `optRoutePass` `:626-817` — SEQUENTIAL-deterministic port of the threaded shape: candidates from `ReadSortedRouteItems` `:1086-1182` (ascending x, then y, then layer; vias preferred over traces at the same location; skip user-fixed vias, shove-fixed traces, traces touching an unfixed via); per candidate: snapshot board, rip the containing connection (+ adjacent unfixed trace contacts `:855-877`), reroute via the T11 connection machinery with `autoroutePassesForOptimizingItem` semantics (maxAutoroutePasses 6; ripupCosts `= start·10`, ×0.6 trace factor `:884-891`), score, keep-or-discard; ONE winning candidate applied per pass (`:787-795` — first-improver in candidate order = the deterministic analog of Java's race); `maxConsecutiveFailures` early stop (12 pass 1 / 50 after); per-candidate budget wall→tick.
- [ ] Pins: acceptance-gate pins (each rejection reason fires on a crafted regression); one-winner-per-pass pin (two improving candidates → only the first applies); restore-incumbent pin (board state byte-equal to snapshot after reject); threshold-stop pin; the candidate-order pin (ascending x/y/layer + via preference) — order is determinism-critical.
- [ ] Honest-red posture: if a tier fixture's optimizer outcome diverges from the Java sample beyond ε (Java's own thread nondeterminism bounds this), record as a triage line, do not chase byte-equality.
- [ ] Ripple: all compares; digests; battery CHEAP.
- [ ] Gates + ONE commit `feat(m4): BatchOptimizer — rip-and-reroute optimizer stage (sequential-deterministic)`.

## Task 10: Full-pipeline assembly + CLI wiring

**Files:** Create `rust/crates/epic-router/src/pipeline/full.rs` (the `RoutingPipeline` equivalent); modify `epic-cli/src/route.rs` (`run_route` step 4→5: routing stage → optimization stage; `ManifestPhases.optimizer` fill + `optimizer_score` render at :163/:254; phase-filtered counters backfill), `pipeline/event_sink.rs` (stage task-state events if Java emits per-stage STARTED/FINISHED — mirror).

- [ ] `RoutingPipeline.run()` equivalent (`RoutingPipeline.java:86-134`): `run_routing_stage` (router-enabled gate `maxPasses == null || >= 0`; fanout-only mode = `max_passes=0` temporary override, run, restore — port it, it is cheap and settings-reachable) → `run_optimization_stage` (gated on `run_optimizer` + not stop-requested); stage IDLE transitions; `board.finish_autoroute()` equivalent if the Rust board has one (recon-inside; if it is a Java-side no-op for headless, record and skip).
- [ ] Manifest: per-phase rows (fanout/autorouter/optimizer) with the T7 counters + optimizer score; the phase-blind `passes_completed` backfill becomes phase-filtered (SEAM T13 bank) — pin the attribution.
- [ ] Pins: stage-order pin (optimizer never runs before router completes/stops); fanout-only-mode pin; stop-request short-circuit pin.
- [ ] Ripple: standard + `epic-cli route` smoke on one tier fixture with fanout+optimizer on (release).
- [ ] Gates + ONE commit `feat(m4): full pipeline assembly — routing → optimization stages + per-phase manifest (RoutingPipeline parity)`.

## Task 11: Full-pipeline baselines + battery face flip

**Files:** Modify `rust/harness/src/router_compare.rs` (`route_argv` :100 profile switch router-only|full; `COMPARABILITY_FLAG` handling), `harness/src/main.rs` (capture/verify profile plumbing), `harness/src/tiers.rs` if the profile needs per-fixture settings; NEW baseline dir via `epic-harness capture` (additive only).

- [ ] Verify the M0 baseline profile FIRST (cheap): were the M0 captures default-settings full-pipeline with the same jar/settings face the battery needs? If exactly reusable, reuse and record why; else capture a `full` profile (Java jar, fanout+optimizer enabled, tier settings as tiers.yaml specifies) — additive capture, commit with explanation.
- [ ] `router compare` gains the profile face: router-only mode MUST remain the M2/M3 regression face (corpus/events/digs unchanged); `full` mode drops the `--router.fanout.enabled=false` comparability flag and compares Rust full-pipeline vs full baselines on completion/violations/score with the existing directional gates + ε idiom.
- [ ] Battery: the Tier A face flips to full-pipeline (both sides); the M3 router-only battery results remain recorded in SEAM as the M3 exit snapshot (do not overwrite the historical rows — add the M4 face alongside).
- [ ] Pins: profile-switch pin (wrong profile → loud failure); the flag-drop is gated by profile (router-only keeps the flag — pin both arms).
- [ ] Run: full battery (release, external timeouts, the one-off protocol for wall fixtures); record the honest 11/11-or-not inventory with no "?" rows (T12 works the reds).
- [ ] Gates + ONE commit `feat(m4): full-pipeline baselines + router-compare profile face (battery flips to full)`.

## Task 12: Tier A close-out — bm06/bm11 tuning + bm01 wall (iterate to 11/11)

**Files:** Modify the victim-choice/tie-break sites the T2 checkpoint + T6 triage identified (`maze/ripup.rs` victim selection and/or the rip-free exploration order — buglog 181's levers); bm01: measure, then optimize per-pass cost (pull-tight is now on — re-measure the pass walls; hot-profile ONE pass; the lever is search cost, M5 arena/parallelism is the backstop, NEVER tier edits).

- [ ] Order of operations: (1) re-read the T2 checkpoint + T6 attempt-face evidence; (2) align victim-choice tie-breaks Java-exactly (each alignment pinned at the witnessed fork — buglog 181's attempt-42 / via-841-vs-trace-903 / net-1-335-vs-net-5-1199 forks are the pin worlds); (3) re-run bm06+bm11 single-fixture release after EACH alignment (cheap, ~88 s/~1,089 s); (4) bm01: measure Σ pass walls at full pipeline; if still wall-limited, profile and land the top search-cost fix; the natural-end 8.5–13.4 h must shrink materially — if it cannot within M4, record honestly as the M5-chartered remainder (buglog 175 stays open with the M4 measurement appended).
- [ ] Constraint: tuning changes must not regress the events corpus (exit 0 holds) or the seven compares; every tie-break change is a Java-parity port with a witness, never a free-parameter search (cerebrum: anchors are hypotheses; the jar wins).
- [ ] Full battery at end: the 11/11-or-not truth table, no "?" rows, all triage lines with buglog ids.
- [ ] Gates + ONE commit per logical alignment (2–4 commits expected) `fix(m4): victim-choice tie-break alignment (buglog 181) — <fork>`.

## Task 13: CI flip + M4 docs + milestone adjudication

**Files:** Modify `.github/workflows/rust-check.yml` (drop `--report-only` on the compare step; refresh the stale pre-adjudication compare-step comment), `rust/harness/src/ci_tripwire.rs` (BOTH pins updated same-commit — the pin fires otherwise; this is the designed tripwire), `rust/README.md` (:19 status block, :159 gates comment, the epic-engine "M4 scaffold" crate-list line), `docs/superpowers/specs/2026-09-11-epicrouter-rust-rewrite-design.md` (:158 CI paragraph + the §6 M4 exit note), `docs/architecture.md`, `rust/crates/epic-router/SEAM.md` (M4 criterion map — successor to the T17c map), `.wolf/buglog.json` closures.

- [ ] **The flip is ONE commit** (the adjudicated protocol): only when the battery reads 11/11 AND the profile/wall precondition holds (test-step timeout absorbs the suite — verify Σ walls; if NOT, the flip condition is unmet: record and leave `--report-only` in place with the pins intact, that outcome is honest). Same-commit: yml change + both tripwire pins + all three prose posture sites + the stale yml comment. Run `cargo test --workspace` to watch the pins pass against the NEW yml (and temporarily verify a pin WOULD fire by edit+edit-back on a copy — mutant discipline).
- [ ] Decide + record the events-compare-in-CI question (T6 recommendation): if flipped in, it rides the same commit family with its own pin.
- [ ] Docs: README M4 rows (battery face, baseline profiles, crate relabels), design §6 M4 exit note (shape of the M3 note: verdict + deviations table + carry-forwards), architecture map deltas, SEAM M4 criterion map.
- [ ] **Milestone adjudication** (fresh reviewer, the T17d pattern): adjudicate M4 against design §6 M4's own text; produce the verdict token; then the docs land per its authority.
- [ ] Gates + the flip commit `ci(m4): flip the rust-check compare gate live (11/11 battery; tripwire pins updated same-commit)` — then the docs commit(s) after adjudication.

---

## Standing rules (every task)

- Two-stage review per task (session protocol): fresh spec reviewer → fix rounds by the original implementer → re-review to SPEC_COMPLIANT → fresh quality reviewer → fix rounds → APPROVED. Reviewers re-run gates and mutants themselves.
- Gates per task: `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings` (incl. `clippy::unwrap_used` clean); bare `cargo test --workspace` (census additions only — report the triple); the seven compares with `EPIC_SKIP_GRADLE=1`, explicit command lines, REAL exit codes (never pipe a gate — the fish `$status`-after-pipe trap); determinism digests reported (value rotations declared, byte-stability asserted).
- Battery discipline: CHEAP subset only for probes (bm08, ecc83-pp, ecc83-pp_v2 — release, external `timeout`); bm06/bm11 single-fixture RELEASE runs permitted; NEVER bm01/bm11 unbounded; never bm06 in debug; never re-run the M3 one-off captures.
- Mutants by edit + edit-back (NEVER `git checkout --`); committed artifacts never touched; `.wolf/` + `logs/` never in feat commits; commits end with `Co-Authored-By: Claude Code <noreply@anthropic.com>`; never push; never stage automatically; `cd` every Bash call (fish); NEVER kill or modify any process that is not yours.
- Every task that moves routing: expect digest rotation and battery movement — RECORD them in SEAM; the M3 exit snapshot rows are historical, never overwritten.
- Settings tasks (T7/T9/T10): flag uptake is verified from the rendered MANIFEST, never from the `GlobalSettings` "Unknown settings property" WARN line (buglog 168 — two independent settings layers consume `--x=v` argv; the WARN line diagnoses the wrong one).
