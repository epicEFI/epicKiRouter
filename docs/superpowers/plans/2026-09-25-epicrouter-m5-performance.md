# EpicRouter M5 — Performance: Arena/SoA + Deterministic Parallelism Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Take the M4-completed full pipeline from ~6–30× slower than Java to the design §6 M5 exit — **Tier A Σ wall ≤ 0.5× Java** (`autorouter_seconds + optimizer_seconds` from the committed baseline manifests), a **large-board trend toward 10×**, and **`--threads N` byte-identical to `--threads 1`** — while closing the inherited completion-parity residuals (buglog 189 → 181) and landing the CI compare flip when its two preconditions (full battery 11/11 AND wall under the 30-min budget) are met.

**Architecture:** The engine's observable behavior is FROZEN by tripwires — every perf change lands behind byte-invariance: the seven M2/M3 compares, the events golden (`cc50216c…`), the router-only Tier A faces, and the determinism canaries (ses `253e7b1401775172…` / manifest `8c4a97738375e67d…`) must be UNROTATED by every arena slice. Work proceeds as measurement-ranked vertical slices: the maze/expansion hot `BTreeMap`s and the priority queue (`epic-router/src/engine.rs` — `graveyard`/`obstacle_rooms`/`room_tree_entries` :462-477 and the PQ site T1 ranks #1), the existing `epic-index` slab (`shape_tree.rs` — already a `NodeIdx` slab; optimize, don't convert), and the geometry/rational churn sites. Parallelism is per-net fixed-ID partitioning in the batch pass loop with deterministic reduction — the design :78/:260 contract — landing the `-mt`/`router.max_threads` seam that already exists parsed-but-unused (`epic-cli/src/settings.rs:62-64`).

**Tech Stack:** Rust workspace in `rust/` (run cargo from there); Java oracle via `rust/harness` (JDK 25 at `~/.jdks/jdk-25.0.4.1+1`); branch `epic/main`; NEVER push; NEVER modify Java under `src/`/`src_v19/` (reading fine; `rust/harness/oracle/*.java` is harness-side/editable); committed artifacts (`rust/harness/baselines/**`, corpus dirs, events-golden sha `cc50216c…`, `tiers.yaml`) never modified — **`tiers.yaml` timeouts are NOT raised to make walls fit; the wall must genuinely drop**.

**Recon basis (2026-09-25, at tree `6094e1b53`):** design §2/:22/:26 (allocation-churn evidence: boxed RB-tree PQ with per-insert allocation, pointer-chasing `ShapeTree`, exact-rational geometry, multi-GB cumulative churn; 3–10× layout + 2–6× parallelism + 2–4× SIMD estimates), §5 :98 (no-slow-regression ratchet "From M5: Tier A ≤ 0.5× Java"), §6 :111 (M5 row), :260-261 (determinism contract; slab+epoch index design, "not per-pass arenas"); the M4 exit note's deviations table + carry-forwards; buglog 175/181/189 verbatim; `rust/harness/config/tiers.yaml` (Tier A = the 11 canary fixtures; Σ Rust release battery wall 2061.2 s at M4 exit); Java walls in `rust/harness/baselines/java/{A,B,C}/**/*.baseline.json` (`autorouter_seconds`, `optimizer_seconds`); epic-index already slab-allocated (`shape_tree.rs:1-52`); CLI thread seams parsed-unused (`settings.rs:62-64`, `:140-142`). Implementers RE-VERIFY every anchor — the committed tree wins every conflict, including with this plan.

**Key facts baked in:**
- **Tier A wall truth at M4 exit:** Rust Σ 2061.2 s (release battery, `logs/M4-T12/evidence/battery_full_t12.log`) vs Java Σ ≈ T1 extracts (per-fixture `autorouter_seconds + optimizer_seconds`; spot: bm01 106.37 s, bm11 47.92 s). bm11's Rust face: 62–64 s/pass × ~10 passes, harness-killed at 600.2 s — the per-pass cost is the dominant wall; bm01: 20 passes in 673.6 s (stagnation-stopped, not wall-limited).
- **The index is NOT the Java pointer-chase anymore:** M2 already ported it as a `NodeIdx` slab. Slice B optimizes the existing slab (epoch reclamation per design :261, hot-path query allocation), it does not convert anything.
- **Determinism is the product:** `DOOR_TAG_COUNTER` (`expansion/door.rs:54-61`) is a global counter whose construction order feeds byte-output; parallelism makes its assignment order live — T7 must give it a deterministic per-partition assignment, and that door.rs edit is the TRIGGER for the T2 BACKLOG pair (unpinned `getSectionSegments` EMPTY door-shape and both-complete-NULL arms, SEAM :728) to gain its pins.
- **Java's own threading posture** (the port's contrast): router core effectively single-threaded (upstream #289); `BatchOptimizer`'s thread pool makes Java's optimizer output run-to-run nondeterministic — our `--threads N ≡ --threads 1` contract is strictly stronger and is the gate, not a nicety.
- **Buglog 189's instruments are already built:** lever (a) the attempted recording-subclass form preserved at `logs/M4-T12/evidence/instrument_T12RecordingTree45.java.txt` (a `ShapeSearchTree45Degree` subclass overriding `completeShape` to log inputs/results — land it INSIDE the real CLI process via premain/agent or a harness crate calling `initializeCli` with injection between load and route); lever (b) the `getHash()` jshell reproduction oracle (`BoardSnapshotManager.java:58-72`; the direct-read hash `4207fa7f…` reproduced byte-exactly, `instrument_directread_hash.out`); the three board hashes with roles (CLI `b84a773a…`, cliflow replication `202d1017…`, direct-read `4207fa7f…`). Standing candidates: matrix val 3000-vs-2000 on outline×class-1, or thicker raw outline insertion.

## Milestone exit criteria (design §6 M5 + inherited carry-forwards)

1. **Tier A ≤ 0.5× Java wall** — Σ over the 11 Tier A fixtures, Rust release full-pipeline wall vs the baselines' `autorouter_seconds + optimizer_seconds`, measured by the harness's own wall faces and recorded as the M5 wall table (the §5 :98 ratchet's first notch).
2. **Large-board trend toward 10×** — per-fixture Rust/Java ratio table across Tier A + the Tier B/C sample; the criterion is the TREND (ratios growing with board size), stated with numbers, not a single threshold.
3. **Threads-invariance byte-identical** — `--threads N` output (SES + manifest + score faces) byte-identical to `--threads 1`, pinned in-tree and CI-enforced.
4. **Completion parity (inherited; the flip's precondition):** buglog 189 closed (bm06 green on the full-face battery), 181 dispositioned by lever-(c) re-derivation (bm01 green or honestly re-chartered with fresh evidence), full battery **11/11**.
5. **The CI flip** — when 11/11 AND Σ battery wall < the compare step's 30-min budget both hold: the ONE-commit protocol (drop `--report-only`, both `ci_tripwire` pins updated same-commit, the events compare step added with its own pin, posture prose refreshed at every enumerated surface). If either precondition fails at milestone end: honest hold, recorded — the M4-T13 protocol, not a re-litigation.
6. **Gates green; zero regressions** — all seven compares (corpus 5000/5000, dsn 1332/0, ses 20, ses-snap 5, index 33, undo 33, drc 17), events 3520 exit 0, determinism canaries UNROTATED through every slice, census additions only.

**Carry-forwards baked into tasks:** 189 levers (a)/(b)/(c) → T2/T3; 181 lever (c) → T3; per-pass search cost (bm01/bm11) → T1 baseline + T4–T7 slices + T8 face; door-tag construction order + the M4-T2 BACKLOG pair's pin obligation (the unpinned `getSectionSegments` arms, SEAM :728 — pins land at the door.rs edit) → T7; the flip → T8 readiness + T9 landing; M4-T9's F3 exit condition (the crafted through-hole-pin fixture for that task's optimizer stop-face pin) → opportunistic in T3 or T8 (first consumer wins, per the return-with-first-consumer rule); keep_point sixth-field candidate (`epic-board/src/trace_tightener/mod.rs:416`) → returns only if a slice touches that file.

**Byte-invariance rule (recurring, every slice):** a perf change is correct IFF the outputs are byte-identical to the pre-slice tree on: the seven compares, the events golden, the router-only Tier A compare face, and ×2 determinism runs (canaries unrotated). Any rotation = an ordering change = a bug in the slice, not churn to be re-cited. The ONLY sanctioned digest rotation in this milestone is none.

---

## Task 1: The M5 wall-ratio baseline + allocation-cost profile (measurement only, no engine change)

**Files:** Create `logs/M5-T1/` (report + evidence); harness additions ONLY if a wall/alloc face is missing (a counting-allocator `#[global_allocator]` behind a feature or env flag in a harness bin — std-only, no new deps; locate the harness bin faces via `rust/harness/src/main.rs` subcommands). NO `rust/crates/epic-*` changes.

- [ ] Extract the Java wall truth: for all 11 Tier A fixtures + the Tier B/C sample, read `autorouter_seconds + optimizer_seconds` from `rust/harness/baselines/java/{A,B,C}/**/*.baseline.json` (spot anchors: bm01 106.37, bm11 47.92); produce the Σ-Tier-A Java wall and the per-fixture table. Record schema caveats (any fixture missing the fields → named, not papered over).
- [ ] Rust before-picture: re-run the full Tier A release battery ONCE (the M5 baseline face; ~34 min at M4-exit speed — sanctioned as this milestone's before-measurement; NEVER re-run ad hoc afterwards: T8 re-measures the after). Capture per-fixture walls + Σ; verify the three known reds reproduce their M4 faces (8/3 with the same triage lines — any NEW face is a finding, not noise).
- [ ] Allocation/cost profile: counting-allocator instrument (allocs + bytes, per-phase if cheap) on single-fixture release runs of bm08 (small/green), bm06 (mid/red-189), bm11 (wall-dominated); plus `perf stat`/`time -v` RSS if available. Produce the RANKED table: top allocation sites / structures / phases (the BTreeMap family in `engine.rs` :462-477, the maze PQ site — locate via the profile, `openwolf find MazeSearchEngine` for the ported structure — the `epic-index` query hot path (`complete_shape.rs`, `min_area_tree.rs`), rational/geometry churn in `geometry`/shape faces).
- [ ] The slice order decision: from the ranking, fix T4=slice A, T5=slice B, T6=slice C targets (the plan's defaults are the BTreeMap/PQ family, the index slab hot path, geometry churn — OVERRIDDEN by measurement if the profile says otherwise; record the binding ranking in the report + SEAM).
- [ ] Gates (unchanged tree): fmt, clippy `--workspace --all-targets -- -D warnings`, bare `cargo test --workspace` 1422/0/17. Evidence to `logs/M5-T1/evidence/`. ONE commit if anything landed (harness-only): `chore(m5): T1 wall baseline + alloc profile instrument (harness-side only)` — else no commit, report only.

## Task 2: Buglog 189 root-cause — the live-process instrument (lever (a), fallbacks (b)/(c))

**Files:** Create `logs/M5-T2/` (report + evidence); scratch under `logs/M5-T2/scratch/` (the instrumented Java/.jsh — harness-side, `rust/harness/oracle/` editable if the agent/injection needs a home there); NO engine change expected.

- [ ] Land the recording instrument INSIDE the real CLI process: preferred forms, in order — (1) a `premain` javaagent JAR (built from scratch sources, attached via `-javaagent` to the frozen oracle jar's JVM; the jar itself untouched) or (2) a harness crate/classpath shim that calls `initializeCli` with the recording-tree installation injected between board load and route. The subclass form to port: `logs/M4-T12/evidence/instrument_T12RecordingTree45.java.txt` (`completeShape` override logging inputs/results). If BOTH forms are genuinely blocked (document why, mechanically), fall back to lever (b) intensification: `getHash()`-oracle bisect of the CLI-vs-directread state delta (`BoardSnapshotManager.java:58-72`; hash roles: CLI `b84a773a…` vs cliflow `202d1017…` vs direct-read `4207fa7f…`).
- [ ] Capture the CLI flow's ACTUAL `completeShape` inputs/results at the first fanout search (pin U9-1, net 21, the north-edge −911636 face); compare against the reconstruction face (−911136) and the standing candidates (matrix val 3000-vs-2000 on outline×class-1 at the compensated band; thicker raw outline insertion). Expected deliverable: the exact input difference (a shape, a clearance value, a tree-state delta) named with instrument-logged rows.
- [ ] Reproduce ×2 (determinism of the instrument itself); preserve all instrument outputs under `evidence/` with invocation headers (the T12 evidence discipline: `timeout 300 ~/.jdks/jdk-25.0.4.1+1/bin/jshell --class-path build/libs/freerouting-current-executable.jar …` style).
- [ ] Root-cause verdict: either the named mechanism (→ T3 ports the fix) or the exhausted-ladder record (→ lever (c) re-derivation charters T3 differently — STOP and report if the ladder exhausts; that is a plan-level decision point, do NOT improvise a fix).
- [ ] Gates + ONE commit if anything landed (scratch/evidence are `logs/` = uncommitted by rule; an `oracle/` addition commits as `chore(m5): T2 live-process recording instrument (buglog 189 lever a)`).

## Task 3: The 189 fix + battery re-face + the 181 lever-(c) disposition

**Files:** Modify the Rust site T2 names (expected: the outline-insertion/clearance-matrix face in the board-construction or compensated-class-1 path — `openwolf find` from T2's verdict; likely `epic-board`/`epic-rules`); pins in the matching test tree; buglog 189/181 appends.

- [ ] Port the fix Java-exactly from T2's instrument evidence; pins follow the T3-M5 measured-boundary discipline (cerebrum 16): pin the EXACT divergent boundary ±1 unit both directions (the −911636/−911136 band edges), plus a room-completion witness on the bm06 U9-1 first-fanout search (crafted minimal world preferred over the heavy fixture).
- [ ] Ripple verification (mandatory, report each): the seven compares (byte-invariance expected OUTSIDE bm06-family behavior — but the fix CHANGES bm06's route: the router-only Tier A face for bm06 WILL move — that is the sanctioned movement; determinism canaries may rotate IF they ride bm06-class boards — CHECK which fixture the canaries ride; if they rotate, update SEAM citations at this commit and say so, per the M4-T1 precedent); events golden (expect unchanged — the events corpus never carried bm06); full battery re-face is T8's, not this task's — instead bm06 + bm01 single-fixture release re-runs (permitted) with the completion/score/violation faces recorded.
- [ ] 181 disposition (lever (c), the re-derivation): re-diff the attempt stream on bm06 post-fix (the T12/T17c instrument discipline — production `run_route` + `CaptureDriverSink`, board hashes per pass). If the 189 fix collapses the divergence cascade (bm06 reaches ≥ Java's 2/8/971.63 and bm01's plateau face moves toward 0 incomplete): close 181 with the evidence. If a residual fork remains: re-charter 181 with the NEW first-fork row + owner (M5-continued or M6), honestly — do NOT tune tie-breaks to force a close (the T17c discipline).
- [ ] Buglog: 189 CLOSED (fix commit pinned); 181 dispositioned; 175 appended if the wall faces moved (the completion fix may change pass counts → wall).
- [ ] Gates + ONE commit `fix(m5): room-completion parity at the CLI live-state divergence (buglog 189, levers a/b) + 181 disposition`.

## Task 4: Arena slice A — the maze/expansion hot maps + priority queue

**Files:** Modify `rust/crates/epic-router/src/engine.rs` (:462-477 the `BTreeMap` family: `graveyard`, `obstacle_rooms`, `room_tree_entries`, + `ripped_item_list` :1001) and the PQ site T1 ranked #1 (locate via profile; the ported `MazeSearchEngine` structure); pins in the matching test tree.

- [ ] Replace the ranked structures with arena/slab-backed equivalents whose ITERATION/POP ORDER is provably identical (u64-keyed slab + free-list for the graveyards; an ordered-index or arena-node PQ preserving the Java tie-order semantics — the comparator port is invariant, only the storage changes). NO semantic change: same inserts, same iteration, same removals.
- [ ] Byte-invariance gate (the recurring rule): seven compares + events golden + router-only Tier A face + determinism ×2 canaries UNROTATED + census additions only. Any rotation = the slice changed order = fix before landing.
- [ ] Wall delta: single-fixture release re-runs of the T1 profile set (bm08/bm06/bm11); record per-fixture delta + alloc-count delta (the counting allocator) in SEAM + the report. If the delta is NOT positive on the wall-dominated fixture, record honestly and proceed (slices compose; T8 judges the Σ).
- [ ] Gates + ONE commit `perf(m5): slice A — arena-back the maze/expansion hot maps + PQ (byte-invariant)`.

## Task 5: Arena slice B — the epic-index slab hot path

**Files:** Modify `rust/crates/epic-index/src/` (`shape_tree.rs` the existing `NodeIdx` slab, `complete_shape.rs`, `min_area_tree.rs`, `search_tree.rs`) per T1's ranking of the index's cost share.

- [ ] The design :261 direction: epoch reclamation for search-mutating flows (incremental insert/remove on the rip-up hot path — `shape_tree.rs:16` notes removal semantics; `search_tree.rs:262` the Java-slot-match pin must KEEP matching), query-path allocation elimination (the per-query boxes/vectors the profile ranks), pointer-chase reduction (SoA fields for the hot node members if measured). The slab already exists — this slice makes its hot paths allocation-free.
- [ ] Byte-invariance gate (recurring rule, all of it — the index feeds EVERYTHING: index/undo compares 33/33 are the direct witnesses).
- [ ] Wall + alloc delta on the T1 set; SEAM + report.
- [ ] Gates + ONE commit `perf(m5): slice B — epic-index hot-path allocation elimination (byte-invariant)`.

## Task 6: Arena slice C — geometry/rational churn (T1's residual top sites)

**Files:** Modify the geometry/shape faces T1 ranked (expected: `epic-geometry` rational hot paths, boxed shape returns in `complete_shape`/clearance queries, `IntOctagon`/`TileShape` temporaries); contents RE-POINTED by T1's binding ranking if it differs from the expectation.

- [ ] Interning/caching for repeated exact-rational values ONLY where equality semantics are preserved exactly (no float slack, no rounding — the parity corpus is the judge); eliminate the ranked temporaries/boxes. SIMD kernels are OUT of scope for this slice (2–4× design estimate is real but only if T1's profile names a ≥15% kernel — record the decision either way).
- [ ] Byte-invariance gate (recurring rule) + corpus compare 5000/5000 as the geometry judge.
- [ ] Wall + alloc delta; SEAM + report.
- [ ] Gates + ONE commit `perf(m5): slice C — geometry/rational churn elimination (byte-invariant)`.

## Task 7: Deterministic parallelism — per-net partitioning + `--threads` + the door.rs pins

**Files:** Modify `rust/crates/epic-router/src/pipeline/batch.rs` (the pass loop — net partitioning + deterministic reduction), `rust/crates/epic-router/src/expansion/door.rs` (DOOR_TAG_COUNTER deterministic assignment — THE M5-checklist item), `rust/crates/epic-cli/src/settings.rs:62-64` + `main.rs` (`-mt`/`router.max_threads` wiring — the seam comment documents it), harness compare/determinism faces for `--threads`; the T2 BACKLOG pins land in door.rs's test tree AT THIS EDIT.

- [ ] Partition the batch pass's per-net work by FIXED NET-ID ranges (design :260); every shared-mutation point gets a deterministic reduction order (sorted by net id, then by the existing tie order — reduction results must equal the sequential interleaving the byte-contract demands; where the sequential order is time-interleaved-but-deterministic, the parallel order must reproduce ITS output, which may require serializing exactly the conflicted points and parallelizing only the independent searches).
- [ ] `DOOR_TAG_COUNTER` (`expansion/door.rs:54-61`): construction order must remain output-deterministic under threads — per-partition tag assignment reconciled to the sequential counter order, or tag assignment hoisted to the deterministic reduction. Land the M4-T2 BACKLOG pins at this edit: the unpinned `getSectionSegments` EMPTY door-shape arm (Java `:109-112`) and both-complete-NULL arm (Java `:124-127`), SEAM :728's obligation.
- [ ] `--threads N` CLI face: wire `router.max_threads`/`-mt` (default 1 = today's behavior byte-identical); N>1 engages the partitioned executor. Threads-invariance GATE: `--threads 4` (and one odd N, e.g. 3) output byte-identical to `--threads 1` on SES + manifest + score faces for ≥2 fixtures (bm08 + bm06), pinned in-tree; harness `router determinism` gains a threads face (or a sibling command) and the CI assertion lands per the design :78 "CI-enforced" contract — as its OWN workflow-touching commit per the standing protocol (the yml comment refreshed at that commit too).
- [ ] Deterministic budgets interact: call-tick budgets are per-search and thread-count-invariant — assert a threads face on one budget-exercising world.
- [ ] Byte-invariance gate (recurring rule, at `--threads 1`) + the NEW threads-invariance faces + census additions only.
- [ ] Wall delta on bm11 (the parallelism-sensitive fixture — per-net searches are the wall); SEAM + report.
- [ ] Gates + ONE commit `feat(m5): deterministic per-net parallelism + --threads (threads-invariance byte-identical) + door.rs BACKLOG pins`. (If the CI assertion needs the yml, that is a SECOND, separate commit — ci vs feat never mix.)

## Task 8: The M5 wall face + flip readiness (the milestone's measurement moment)

**Files:** Create `logs/M5-T8/` (report + evidence); no engine change expected.

- [ ] Full Tier A release battery (the AFTER face): expect 11/11 (T3's completion work + no regression through the slices); record Σ wall + per-fixture walls.
- [ ] The exit tables: (a) Σ-Tier-A Rust vs 0.5× Σ-Java (criterion 1, pass/fail with numbers); (b) the per-fixture ratio table incl. Tier B/C sample — the trend-toward-10× statement (criterion 2); (c) threads-invariance faces re-confirmed at final HEAD (criterion 3).
- [ ] Flip readiness adjudication (both preconditions, cited): 11/11 AND Σ wall < 30-min CI budget (the compare step's `timeout-minutes: 30` at the committed line — re-read it, cerebrum 15). GO → T9. NO-GO on either → the honest-hold branch (T9 becomes the record task; any residual red gets the deviations-table treatment).
- [ ] M4-T9's F3 exit condition, opportunistically: if any optimizer stop-face world is touched in this milestone's test additions, land the crafted through-hole-pin fixture pin (return-with-first-consumer rule).
- [ ] Gates (full lap: fmt, clippy `--all-targets`, bare census, seven compares, events, determinism ×2) — the pre-flip lap. Report + evidence only; no commit.

## Task 9: The CI flip (GO branch) — the ONE-commit protocol, or the honest hold

**Files:** Modify `.github/workflows/rust-check.yml` (compare step run line + the events step + comments), `rust/harness/src/ci_tripwire.rs` (both pins + the events pin, same-commit), `rust/README.md` + `docs/superpowers/specs/2026-09-11-epicrouter-rust-rewrite-design.md` + SEAM posture rows (the enumerated refresh list: yml comment block, README gates comment + status block + events comment, design §6 CI paragraph).

- [ ] GO branch (preconditions met, T8-cited): ONE commit — drop `--report-only` from the compare run line; update BOTH `ci_tripwire` pins to the new truth same-commit (the pin MUST fail on the pre-commit tree — verify by the mutant discipline, edit + edit-back); ADD the events compare step with its own pin (java-free, ~0.4 s, exit-0 truth); refresh every posture surface (the M4-T13 enumerated list, incl. the events comment's "rides the future flip commit" wording); record the M5-UPDATE parenthetical in the design §6 M4-note CI paragraph (append-not-rewrite).
- [ ] Wall margin check: the CI budget must absorb the battery with margin (record Σ wall vs 30 min; if < 2× margin, note the risk honestly in the yml comment — the WALL WARNING family survives in reduced form).
- [ ] NO-GO branch (either precondition failed): leave `--report-only` + pins intact; record the hold + residuals at all surfaces (the M4-T13 protocol verbatim); the deviations table gains the named rows.
- [ ] Gates: bare `cargo test --workspace` (both pins ok), events compare exit 0, fmt, clippy. ONE commit total (GO) or one docs-only commit (NO-GO).

## Task 10: M5 close-out + adjudication prep

**Files:** Modify `docs/superpowers/specs/2026-09-11-epicrouter-rust-rewrite-design.md` (the M5 exit note, drafted with the verdict slot OPEN — `**VERDICT: <per milestone adjudication>**`), `docs/architecture.md`, `rust/README.md`, `rust/crates/epic-router/SEAM.md` (the M5 criterion map appended below the untouched M4 map); `.wolf/buglog.json` sweep.

- [ ] Buglog sweep: every entry touched by M5 (175/181/189/186-adjacent none expected) dispositioned with pinned commits or honest open-with-owner; sweep the whole ledger for M5-relevant stragglers.
- [ ] The M5 exit note in the M3/M4 shape: criterion verdicts with cited evidence (the T8 tables), deviations table if any red remains (face/root-cause/owner/buglog per row), carry-forwards list (every line traces to a dossier/buglog), the verdict slot left open for the fresh adjudicator.
- [ ] Architecture map + README at the M5 face (perf architecture landed, threads contract, the flip outcome); SEAM M5 criterion map appended (zero deletions above).
- [ ] Gates (cheap set) + ONE docs commit. Then the coordinator dispatches the MILESTONE ADJUDICATION (fresh opus, T17d pattern, report-only) — this plan's terminal act, per the M3/M4 precedent.
