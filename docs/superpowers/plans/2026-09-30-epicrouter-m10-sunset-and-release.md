# EpicRouter M10 — Sunset the Java Oracle, Release 2.0 — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Take the M9-closed engine (`M9_EXIT_WITH_DEVIATIONS` at `379ee6c47`, census 1682/0/19) to the design §5 M10 exit — **"2.0: sunset Java oracle, release — Docs, Linux packaging first, migration guide"** (design `:116`, `:63`: "The existing Java tree is not modified during the rewrite (it is the oracle); it is deleted at M10") — by (1) landing the ONE chartered engine-code change (the stop-flag WIRING, buglog 224, the only 14-row-carry item that touches code), (2) dispositioning every carried deviation at the M10 measurement moment, (3) deleting the Java tree + retiring its CI, (4) shipping the Linux 2.0 release artifact + migration guide — with the verdict slot filled by the terminal milestone adjudication.

**Architecture:** Four movements. (1) **Engine wiring** (`epic-router`): `run_optimization_stage` currently constructs a fresh flagless `StopFace::default()` (`full.rs:342`, the documented T9 artifact) — the fix threads the PARENT face's shared flag into the stage so external raises become visible to the optimizer loop (Java's `BatchOptimizer.java:384` reads the shared `isStopRequested()` face; the CLI production face is flagless so CLI byte-invariance is STRUCTURAL). The Q7 `final_state_for` duplicate hoists into `epic-router` beside `StopReason` (both hosts consume). (2) **The measurement moment** (M9-T7's pattern): the full battery + the 13 carried deviations rows re-faced and dispositioned. (3) **The sunset**: `src/`, `src_v19/`, the gradle build, and the Java CI workflows deleted/reworked in commits (history NEVER rewritten; the parity record's new home = the committed goldens + git history + the grafted `origin/master`). (4) **The release**: version 2.0.0, a CI-built Linux tarball (epic-cli + the desktop shell) with checksums + artifact smoke, the migration guide, the exit note, the adjudication.

**Tech Stack:** Rust workspace in `rust/` (run cargo from there; toolchain pinned 1.93, `rust-version` `Cargo.toml:20`); branch `epic/main`; NEVER push (no remote exists); until T4 lands, NEVER modify Java under `src/`/`src_v19/` (reading fine; T4 is the sanctioned sunset act — after T4 the law is moot); committed artifacts (`rust/harness/baselines/**`, corpus dirs, events-golden, `config/tiers.yaml`, ALL goldens incl. the T4 trio + T5 six + the T3 event-kind golden) never modified — NEW goldens are NEW files via sanctioned capture; canaries ses `dc271114627632d6a959891559d2ee837c180142c06fb36fde20f441f0f91deb` + manifest `cf6077141d2f2910b709764b8d3a61ace9e8be2defb85b92b4c956b68b438639` IMMOVABLE; timeouts NEVER raised; **every cargo/test/harness command capped `systemd-run --user --scope -p MemoryMax=6G` (buglog 194)**; **desktop-feature cargo commands ALSO `--jobs 4`**; clippy gates ALWAYS `--all-targets` (DNR-14); every evidence log opens with the wrapper-echoed invocation and ends `exit=N` (never an exit through a pipe); census expectations declared as ARITHMETIC in log headers BEFORE the run; implementer reports in ENGLISH; implementer commits end `Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>`.

---

## Opening rules (binding on every task)

1. **Census baseline 1682/0/19** (M9's ledger). Every task declares its arithmetic (`1682 + N new pins = expected`) BEFORE the run and reconciles after. Additions only — a deletion of a pin is a finding, not cleanup.
2. **DNR-15** (cite at measured/COMMITTED tree), **DNR-16** (±1 mutants BOTH directions where pins are chartered), **DNR-17** (spawned-bin freshness — rebuild + own exit line), **DNR-18** (every adopted aggregate reconciled against its constituents), **DNR-19** (every adopted CONSTANT's derivation re-checked in writing), **E-4** (clippy first-round-red claims need in-log red), **E-5** (rebuild evidence = command + own exit), buglog **217** (fmt evidence terminates in the green recheck).
3. **Behavioral expectations cite their derivation or are labeled unprobed** (the M9-T2 charter lesson — three charter bugs came from unverified behavior claims).
4. **Engine-crate census at every close-out face** (the M9-QF1 lesson): a task touching `epic-router`/`epic-board`/`epic-geometry`/`epic-drc` appends its SEAM/ledger row in the SAME cycle (for epic-router: SEAM.md, append-only, inside the task's own commit unless a binding law collides — then the deferral is argued and the coordinator discharges it); the close-out enumerates ALL engine crates, not the named exemplar. **Exclusivity claims enumerate the whole namespace.**
5. **A "legacy-no-field" buglog bucket is not a "closed" bucket** (the M9-QF3 lesson): sweeps read entries, not schema.
6. **The eframe bump procedure** (AM6 lesson): IF any task bumps eframe/egui versions, the `__screenshot` smoke face MUST be re-probed (private API, outside semver) before evidence is claimed. No bump is chartered in M10; the rule arms on any incidental bump.
7. `.wolf/` edits stay in the WORKING TREE (the coordinator banks them); `logs/<task>/` is git-ignored scratch (reports + evidence live there).
8. Dev box: fish shell (quote `===`; absolute paths; python via `<<'EOF'` heredocs; nested heredocs in `bash -c '...'` FAIL); JDK 25 at `~/.jdks/jdk-25.0.4.1+1` (needed only while the Java tree lives — T3 and earlier if any oracle capture is needed; none is chartered); `DISPLAY=:1` for shell smokes.
9. **Never touch other processes** (another Claude instance runs another project on this box).

## Recon basis (2026-09-30, at tree `343fba0a2`; every anchor personally verified)

- **The stop wiring**: `pipeline::full::run(manager, board, settings, optimizer_settings, run_optimizer, stop: StopFace, sink) -> PipelineOutcome` (`full.rs:369-378`) hands `&stop` to `run_routing_stage` (`:399`) but `run_optimization_stage(..., full_stop: bool, ...)` (`full.rs:313-321`) gets only the post-routing snapshot `stage.stop_full` (`:399-408`) and constructs `let mut stage_stop = StopFace::default();` (`:342`) — the flagless face. `StopFace` (`batch.rs:175-186`: `flag: Option<Arc<AtomicBool>>`, `local`, `max_items_faced`, `full_stop`; `from_flag` `:183`, `is_requested` `:190` (reads local OR the shared flag), `is_full_stop_requested` `:205`, `request` `:214`, `request_full` `:228`) has NO public flag accessor — the wiring adds one. The optimizer's dead polls: `optimizer.rs:1238-1242/:1458/:1738/:1802`. The module docs' STOP-GATE SEMANTICS residual 2 names this exact artifact (`full.rs:46-49`, citing `BatchOptimizer.java:384`).
- **The session face**: `session.rs:586-588` — `full::run(..., StopFace::from_flag(Some(Arc::clone(&self.stop_flag))), ...)`; the final-state mapping `:593-596` (`Err`→TERMINATED / `Ok(true)`→COMPLETED / `Ok(false)`→`final_state_for(false, outcome.stop_reason)`); the private duplicate `fn final_state_for` at `session.rs:878` (12 lines, doc `:873-877` explains the epic-engine-cannot-depend-on-epic-cli constraint); the CLI's public copy `route.rs:447` (call site `:1293-1297`). `StopReason` lives at `batch.rs:280` — the hoist target. Both hosts already import from `epic_router::pipeline::batch` (`session.rs:60`, `route.rs:38`).
- **The CLI production stop face is FLAGLESS** (module doc `full.rs:47-48`: "production headless faces carry no flag (`StopFace::default()`)"; verified: `route.rs` constructs the flagless face for `full::run`) — therefore wiring the parent flag into the optimizer stage is a NO-OP on the CLI path BY CONSTRUCTION (a `None` flag can never be raised); byte-invariance is structural, and the battery re-proves it measured.
- **The Java surface**: `src/` (17M), `src_v19/` (6.8M), `build.gradle`, `settings.gradle`, `gradlew` (+ `.bat`), `gradle/` (enumerate at execution: `git ls-files | grep -iE 'gradle|\.java$'`), `fixtures/` at repo root STAYS (the boards are the Rust fixtures' source of truth). Workflows referencing java/gradle: `create-release.yml` (JDK setup + `./gradlew build` + windows `dist` + jar upload), `create-snapshot.yml`, `gradle-build-on-pr.yml`, `gui-a11y.yml`, `docker-nightly.yml`, `docker-release.yml`, `deploy-pages.yml` (inspect at execution; disposition each: retire, or rework to the Rust artifact). `rust-check.yml` is ALREADY java-free (its 7 "java" matches are comments). `scripts/build/`, `scripts/i18n/extract-context.py` (Java-source i18n check), and pre-commit hooks referencing java/spotless retire with the sunset.
- **The harness oracle face**: `rust/harness/src/oracle.rs:212` `jar_path` → `build/libs/freerouting-current-executable.jar`; the CAPTURE modes build the jar via gradle (dev-time only — dies at T4; the code path stays, its module docs gain the sunset note, and the capture subcommands fail with a clear error when gradle/src are absent — verify the failure is clean). The COMPARE faces are golden-based and java-free (rust-check.yml green without JDK).
- **Version/packaging faces**: workspace `version = "0.1.0"` (`rust/Cargo.toml:17`); NO `--version` face in `epic-cli/src/main.rs` (grep empty). The desktop shell builds via `-p epic-gui --features desktop`. rust-check.yml runs on `ubuntu-latest` + dtolnay toolchain 1.93.0 (no system libs — eframe dlopens X11/wayland at runtime).
- **The carried deviations** (the M10 intake = design §6 M9 exit note's 14-row table `:972-996`, adjudicator-confirmed): stop-flag WIRING (the only code row — T1); teardrop instrument-vs-consumer decision; flow engagement; via_density lever; F3 trigger re-check; StickHub/CM5 pour battery; bm01 adoption + P2 skew; bm05 29>18; bm10 budget-policy; interf_u 1>0; multichannel parity floors re-record; StickHub/LimeSDR kills; 1Bitsy 2>1; bm04 3>2 (the threads-gate row CLOSED at M9-T6).
- **Q6 (re-route)**: M9-T6 recorded "re-route stays disabled — the workflow letter is load→route→export once; T8+ product decision". **Q7 (hoist)**: M9-T2's deviations register — "the two `final_state_for` copies hold equal only transitively; M10 hoists the mapping into epic-router next to `StopReason`, both hosts consume".

## Milestone exit criteria (concrete; the adjudication judges THESE)

1. **The stop-flag wiring landed and proven**: the optimizer stage consumes the parent face's shared flag (buglog 224 → fixed); a DETERMINISTIC pin shows a mid-optimization external stop terminating the stage (and the session's final state reading CANCELLED where the mapping applies); the CLI battery is byte-identical (structural flagless proof + measured re-proof); the seven compares + canaries green.
2. **The Q7 hoist landed** (one `final_state_for` beside `StopReason`; both hosts consume; the duplicate pin sets consolidated) **and the Q6 re-route product decision recorded** (the current session face probed; the decision in the exit note + migration guide).
3. **The deviations table dispositioned at the M10 moment**: every one of the 13 carried rows re-faced against fresh battery/re-face runs and each CLOSED or re-owned post-2.0 with a named owner + exit condition; parity floors re-recorded.
4. **The Java oracle sunsetted**: `src/`, `src_v19/`, the gradle build, and the Java CI faces deleted in commits (history preserved, nothing rewritten); `rust-check.yml` green on a tree with no Java; no dangling references (workflows, scripts, docs, hooks); the parity record's preservation stated (committed goldens + git history + `origin/master`).
5. **2.0 released**: workspace version 2.0.0 + `--version` face; a CI-built Linux x86_64 tarball (epic-cli + the desktop shell + README + LICENSE) with sha256 checksums, smoke-tested from the artifact; the migration guide (Freerouting → EpicRouter: identical DSN/SES workflow, CLI flag mapping, GUI tour, differences, known bounds, GPLv3 + attribution); README/architecture faces updated.
6. **Zero regressions to the last**: census additions only (1682 → the M10 ledger); canaries IMMOVABLE; nothing pushed; verdict slot filled by the terminal adjudication.

---

### Task 1: the stop-flag WIRING (the one engine-code change; buglog 224)

**Files:**
- Modify: `rust/crates/epic-router/src/pipeline/batch.rs` (add the flag accessor on `StopFace`, ~`186-190`)
- Modify: `rust/crates/epic-router/src/pipeline/full.rs` (`run_optimization_stage` `:313-321` + `:342`; the call site `:399-408`; the module-doc STOP-GATE SEMANTICS residual 2 `:44-49`)
- Modify: `rust/crates/epic-router/src/pipeline/optimizer.rs` (ONLY if the stop-reason propagation needs the outcome face — probe first)
- Modify: `rust/crates/epic-engine/src/session.rs` (no signature change expected; the deterministic pin lives here or in a test module)
- Test: `rust/crates/epic-router/tests/` (wiring pins) + `rust/crates/epic-engine/tests/` or `session.rs` test module (the end-to-end deterministic cancel pin)
- Modify: `rust/crates/epic-router/SEAM.md` (append the M10-T1 row — INSIDE this task's commit; Opening rule 4)

- [ ] **Step 1: add the accessor** on `StopFace` (batch.rs, after `from_flag`):

```rust
    /// The shared flag, if this face has one (the M10-T1 wiring face:
    /// the optimizer stage constructs its own face OVER the parent's
    /// flag so external raises are visible mid-optimization — Java's
    /// `BatchOptimizer` reads the shared `isStopRequested()` face via
    /// `job.thread`, `BatchOptimizer.java:384`).
    #[must_use]
    pub fn flag(&self) -> Option<&Arc<AtomicBool>> {
        self.flag.as_ref()
    }
```

- [ ] **Step 2: thread the parent face into the stage.** `run_optimization_stage` gains a `parent_stop: &StopFace` parameter (after `full_stop: bool`); the call site in `run` passes `&stop`; `:342` becomes:

```rust
    // M10-T1: the stage face SHARES the parent's flag (buglog 224's
    // wiring fix) — a flagless parent (the CLI production face) yields
    // exactly the old `StopFace::default()`; a flagged parent (the
    // session/GUI face) makes external raises visible to the
    // optimizer's polls (`optimizer.rs:1238-1242/:1458/:1738/:1802`).
    let mut stage_stop = StopFace::from_flag(parent_stop.flag().cloned());
```

  Update the module-doc residual 2 (`full.rs:44-49`) from "PORT RESIDUAL" to "WIRED at M10-T1" with the buglog-224 closure note. Run `cargo test -p epic-router` — every existing pin must pass UNCHANGED (the flagless default face is byte-identical to `from_flag(None)`).

- [ ] **Step 3: the deterministic wiring pin** (epic-router tests): construct a flagged parent (`StopFace::from_flag(Some(Arc::new(AtomicBool::new(false))))`), a world whose optimization stage runs, and raise the flag via the SINK at the Optimization stage transition (a test `DriverSink` whose `task_state`/board_updated hook stores `true` — deterministic, no timing); assert the optimizer loop observed the stop (the outcome face the stage returns — probe which field carries it; if NO face propagates, wire `stop_reason` onto the stage outcome citing Java's `RoutingPipeline.java:122-133` and document the probe).

- [ ] **Step 4: the end-to-end session pin** (epic-engine): `Session::load` a small fixture, call `route` with a sink that raises `session.stop_flag()` (or the public cancel face) at the first Optimization event; assert `final_state` reads CANCELLED (or the engine's honest face if the optimizer's stop maps differently — cite `final_state_for`'s derivation either way). DNR-16: ±1 mutant both directions on the new pins (flip the wiring to `StopFace::default()` → the pin MUST fail; restore → green).

- [ ] **Step 5: the byte-invariance re-proof** (capped): `cargo test --workspace` (census arithmetic declared first), the harness battery on the Tier A + workflow set (the M9-T7 faces), canaries + events compare green. Log INVOCATION-headed, `exit=N`-tailed.

- [ ] **Step 6: bookkeeping**: buglog 224 → `status: fixed` citing the commit + the pin; SEAM.md M10-T1 row appended (append-only); fmt + clippy `--all-targets` both faces (workspace + desktop `--jobs 4`); commit (implementer trailer); report `logs/M10-T1/report-t1.md` (§1 what/§2 pins/§3 gates/§4 census declared-vs-measured/§5 deviations/§6 STATUS).

### Task 2: the Q7 hoist + the Q6 re-route product decision

**Files:**
- Modify: `rust/crates/epic-router/src/pipeline/batch.rs` (add `pub fn final_state_for(run_ok: bool, stop_reason: Option<StopReason>) -> &'static str` beside `StopReason` `:280` — moved VERBATIM from `route.rs:447`, with its doc)
- Modify: `rust/crates/epic-cli/src/route.rs` (delete the local `:447` copy; import from `epic_router::pipeline::batch`)
- Modify: `rust/crates/epic-engine/src/session.rs` (delete the private `:878` copy; import; the `:873-877` constraint doc updates to the import)
- Test: pin the SINGLE source (both hosts' faces already pinned — M9-T2's arms + session tests; add one pin asserting the public path compiles from both crates if coverage is not already exact — probe first, do not duplicate)
- Modify: `rust/crates/epic-router/SEAM.md` (append the M10-T2 row, same commit)

- [ ] **Step 1: probe the Q6 current face first** (Opening rule 3): what does a second `Session::route()` do today (error? re-route? panic?) — cite the code path; record in the report. **The product decision (pre-made, charter-level): re-route stays DISABLED** — one route per Session; a re-route requires a fresh Session; the current face must be a clean documented error (if it is not, make it one — smallest change, pinned).
- [ ] **Step 2: the move** (verbatim body; both hosts consume; `cargo test --workspace` — census arithmetic: net 0 new pins expected unless Step 1's error-face pin adds one; declare).
- [ ] **Step 3: gates + commit + report** (`logs/M10-T2/report-t2.md`): fmt/clippy both faces, SEAM row, the Q6 decision recorded for the exit note + migration guide.

### Task 3: the M10 measurement moment (re-faces + deviations dispositionment; ZERO commits)

**Files:** Create `logs/M10-T3/` (report + evidence only — the M9-T7 no-commit pattern).

- [ ] **Step 1: the battery** (capped, INVOCATION-headed): the full workflow battery (Tier A + Tier B/C + crafted) at the post-T1/T2 tree — session-vs-CLI byte-identity 17/17 expected; the seven compares; events; canaries; Tier A gate face; census re-measured (declared arithmetic).
- [ ] **Step 2: the 13-row re-face table** — each carried row re-faced against fresh runs (bm05/bm10/1Bitsy/bm04/interf_u/StickHub/LimeSDR faces; parity floors re-recorded; the F3 trigger re-checked: did any new geometry-corpus face appear in M10? if no, carried); each row dispositioned: CLOSED (with the closing evidence) or re-owned post-2.0 (named owner + exit condition). The decision rows land their DECISIONS here in argument form for the exit note: teardrop instrument-vs-consumer (the corrected AM2 law governs; M10 has no aesthetics criterion — the decision is the instrument letter kept + the tension documented, or a chartered post-2.0 exclusion), bm01 adoption, bm10 budget policy, flow engagement, via_density lever, pour battery (run it if Tier B/C faces allow within the 30-min budget; else carried with the reason).
- [ ] **Step 3: report** (`logs/M10-T3/report-t3.md`): the tables, the wall faces (stage vs process — QF2's digit-provenance law), the gate lap, the census ledger update (1682 → the T1/T2 sums → measured), the deviations dispositionment table (the exit note's input).

### Task 4: the Java sunset (the deletion)

**Files:**
- Delete: `src/`, `src_v19/`, `build.gradle`, `settings.gradle`, `gradlew`, `gradlew.bat`, `gradle/`, `gradle.properties` (if present — enumerate precisely at execution)
- Delete/retire/rework: `.github/workflows/gradle-build-on-pr.yml`, `gui-a11y.yml`, `docker-nightly.yml`, `docker-release.yml` (retire); `create-release.yml`, `create-snapshot.yml` (rework → T5's Rust faces; may land as the T5 commit instead — coordinate: retire here, rework in T5); `deploy-pages.yml` (inspect: if it deploys Java-javadoc/docs of the Java tree, retire; if it deploys repo docs, keep); `scripts/build/`, `scripts/i18n/` (retire with the Java sources), pre-commit java/spotless/i18n refs
- Modify: `CLAUDE.md` (the Build & Test section rewritten: the Java commands removed, the Rust faces primary, the JDK/gradle notes become historical), `AGENTS.md` (sunset addendum at the top of the Java-notes sections — not a rewrite), `README.md` (root — the Freerouting build face replaced by the EpicRouter 2.0 face, GPLv3 + Freerouting attribution PRESERVED), `docs/architecture.md` (the Java-package Mermaid/glossary marked historical; the Rust tree is the product), `rust/harness/src/oracle.rs` (module docs: the sunset note — capture modes unavailable post-sunset, the jar's preservation story), `.gitignore` (java build entries)
- Verify: `rust-check.yml` green on the Java-free tree; `git ls-files | grep -iE '\.java$|gradle'` returns ONLY intentional keepers (none expected); `grep -rn "src/main/java\|gradlew" .github/ scripts/ docs/*.md CLAUDE.md AGENTS.md README.md` has no live references (historical prose in docs/issues/ and the exit notes is FINE — history is not a dangling reference)

- [ ] **Step 1: the enumeration commit** — `git rm -r` the Java tree + gradle files (one commit, the sunset act; body states the preservation story: git history, committed goldens, `origin/master` graft).
- [ ] **Step 2: the CI/scripts/docs commit** — retire/rework the workflows, the scripts, CLAUDE.md/AGENTS.md/README/architecture faces, oracle.rs docs, .gitignore.
- [ ] **Step 3: the green proof** (capped): `cargo test --workspace` at the post-sunset tree (census unchanged from T3's measurement — arithmetic declared); `pre-commit run --all-files` (post-hook-retirement); the seven compares green. Report `logs/M10-T4/report-t4.md`.

### Task 5: version 2.0.0 + the Linux release artifact

**Files:**
- Modify: `rust/Cargo.toml` (`version = "2.0.0"` at `:17`)
- Modify: `rust/crates/epic-cli/src/main.rs` (a `--version` face printing `epicrouter 2.0.0` — probe the arg-parser idiom used and follow it; pinned)
- Rework: `.github/workflows/create-release.yml` (the Rust release: ubuntu-latest + dtolnay 1.93.0 mirroring rust-check.yml; `cargo build --release -p epic-cli` + `cargo build --release -p epic-gui --features desktop`; tar `epicrouter-2.0.0-x86_64-unknown-linux-gnu.tar.gz` containing the two bins + README + LICENSE + the migration guide pointer; sha256; upload-release-assets; NO windows/java jobs)
- Create: `rust/scripts/package-linux.sh` (the local reproduction of the tarball — same command sequence, `set -euo pipefail`, prints the sha256)

- [ ] **Step 1: version + `--version` pin** (declare census arithmetic).
- [ ] **Step 2: the local artifact + smoke** (capped; desktop build `--jobs 4`): run `package-linux.sh`; smoke FROM THE ARTIFACT — unpack to a temp dir, `./epic-cli --version` = `epicrouter 2.0.0`, `./epic-cli -de <fixture.dsn> -do out.ses` exit 0 + non-empty SES + (if the fixture has a golden) byte-match; the desktop bin launches headless-check via `--smoke` if the face exists (M9-T6's `--smoke <dsn> --frames N --out <png>`), else a `--help` exit-0 face. Evidence logs INVOCATION-headed.
- [ ] **Step 3: the workflow** (exact-argv pins in `ci_tripwire.rs` for any NEW rust-check-adjacent step — the M8-T8 pattern; the release workflow itself is not in the CI gate). Report `logs/M10-T5/report-t5.md`.

### Task 6: the migration guide + release docs

**Files:**
- Create: `docs/migration-guide.md` — Freerouting → EpicRouter 2.0: (1) the workflow is IDENTICAL (DSN in → route → SES out; the KiCad/Specctra interchange unchanged); (2) the CLI face (flag mapping table `epic-cli` vs Freerouting's `-de/-do` + the settings layers defaults→DSN→CLI; the determinism default ON difference); (3) the GUI (load → route with live progress → cancel → export; the overlay views; the post-T1 cancel semantics); (4) what differs by design (determinism/threads-invariance, the engine rewrite, speed); (5) known bounds at 2.0 (the deviations rows that rode post-2.0 — honest, from T3's table); (6) license: GPLv3, a fork of Freerouting, attribution preserved
- Modify: `README.md` (root — the 2.0 product face), `rust/README.md` (the M10 posture row; the milestone history preserved), `docs/architecture.md` (the 2.0 face)

- [ ] **Step 1: write the guide** (every claim cites its record — the T3 tables, the M9/M10 notes; unprobed prose is banned: Opening rule 3 applies to user-facing claims too).
- [ ] **Step 2: the doc faces + commit + report** (`logs/M10-T6/report-t6.md`).

### Task 7: the M10 exit note + close-out (the M9-T8 pattern)

**Files:**
- Modify: `docs/superpowers/specs/2026-09-11-epicrouter-rust-rewrite-design.md` (§6: the M10 exit note in the M8/M9 shape — header (close-out commit named by SHA — the deviation-3 lesson does not recur: the note names the PRIOR commit, the fill names its own parent), verdict slot `**VERDICT: <per milestone adjudication>**` OPEN, criterion verdicts 1–6 with citations, the deviations/carry table (T3's dispositionment, verbatim-cell law), the honest-arc beats, the census ledger chain, the closing slot-stays-OPEN line)
- Modify: `docs/architecture.md`, `rust/README.md` (the M10 CLOSED-PENDING-ADJUDICATION flip), `.wolf/buglog.json` (the sweep — working tree only)
- [ ] The close-out verification faces (engine-crate census over ALL of M10's span — Opening rule 4; the goldens-integrity probe; the census doubly-measured) + the REVIEW-CYCLE RECORD + the charter of the terminal adjudication. Report `logs/M10-T7/report-t7.md`.

### Task 8: the MILESTONE ADJUDICATION (terminal — the plan's final act)

- [ ] Fresh opus, report-only (the M4-T17d→M9 pattern): A1 the six criterion verdicts (each spot-checked against artifacts), A2 the deviations dispositions, A3 the honest-record check, A4 the verdict recommendation (`M10_EXIT_CLEAN` / `M10_EXIT_WITH_DEVIATIONS` / `M10_NOT_MET`), A5 the verdict-slot fill text. The fill lands as the coordinator's follow-up docs commit (slot fill + the adjudicator's own errata + the README flip). **After the fill + the AMENDMENT bank, the loop's mandate ("until m10 and everything is complete") is DISCHARGED.**

---

## Self-review

- **Spec coverage:** design `:116` (docs/packaging/migration) → T5/T6; `:63` (deletion) → T4; the adjudicated carry table → T1 (stop-flag) + T3 (all rows); Q7/Q6 → T2; the exit-note/adjudication protocol → T7/T8. The `:32`/`CLAUDE.md` "oracle until M10" law is honored by task ORDER (T1–T3 precede T4).
- **Placeholder scan:** none — every code step carries its exact code or an exact probe instruction; survey steps (T4's workflow disposition) name their enumeration command and decision rule.
- **Type consistency:** `run_optimization_stage`'s new `parent_stop: &StopFace` param matches the `run` call site; `flag()` accessor matches `from_flag`'s `Option<Arc<AtomicBool>>`; `final_state_for`'s signature identical at all three sites.
