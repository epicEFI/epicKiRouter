# STATUS — session handoff

Last updated: 2026-09-22 (M3 complete)

## ✅ Done

- **M0–M2** (unchanged — see git history and `docs/architecture.md`): 9-crate scaffold, tier matrix, oracle runner; M1a geometry pure-ports 5000/5000; M1b DSN read/SES write parity; M2 searchtree + index/undo corpora.
- **M3 — Detail routing core + epic-cli: COMPLETE (2026-09-22, verdict `M3_EXIT_WITH_DEVIATIONS`).**
  - Plan `77dcaedb`, tasks **#42–#60** (19) all closed under the two-stage review protocol (spec → quality → fix rounds by original implementer → scoped re-review).
  - Final chain through **`c4bcf69ad`** (epic/main, never pushed). Landmarks: T11 autoroute E2E (`d9b216905`), T12 batch pipeline (`c1b66070f`), T13 `epic-cli route` (`f7f4b7db8`), T16 events corpus (`d283f4210`, golden sha `cc50216c…`), T17a panic fixes 169/170 (`91440a937`), T17b bugs 176/177 (`d2f35e228`), T17c battery truth + CI tripwire pins (`cc275b5a5`…`a4c072430`), T17d docs close (`f8f8734e8`, `c4bcf69ad`).
  - **Adjudication** (`logs/M3-T17/milestone-review-m3.md`): criterion 1 MET_WITH_DEVIATIONS (kernel pin suites + E2E captures + events corpus; deviation = events divergence #61), criterion 2 NOT_MET 8/11 (all three reds root-caused non-parity: bm06 TUNING buglog 181, bm01 WALL-LIMITED buglog 175, bm11 red-by-1-at-truth), criterion 3 MET (zero regressions on all six M2 compares). Deviations table red→root cause→M4 lever embedded in design §6 M3 note.
  - **Gates at HEAD**: fmt/clippy clean; bare suite **1305 / 0 / 15** (15 ignored = 9 capture replays + 6 harness slow/e2e); **seven compares** green — corpus 5000/5000, dsn 1332, ses 20, ses-snap 5, index 33, undo 33, drc 17; determinism digests `96ba7300…`/`16714d5e…` byte-identical; battery **8 PASS / 3 RED** (every red root-caused, zero router-introduced violations on all 11); `events compare` exit 1 honest-red (#61).
  - **CI flip = HOLD** (adjudicated): flip is ONE commit when 11/11 battery AND wall-resolved profile (Σ tier ≈5160s vs 30-min CI step) — drop `--report-only`, update both `ci_tripwire.rs` pins same-commit, refresh the three prose posture sites (rust/README.md:19 status block, :159 gates comment, design doc:158 CI paragraph).

## 🚀 Next quest

**Goal:** M4 — Full pipeline: fanout + detail + optimizer (design §6 M4 row). Exit criterion: **zero-regression moment** — all fixtures ≥ Java on completion/violations/score.
First step: write the M4 plan (writing-plans skill, from design §5/§6 M4 + recon of the Java `autoroute.pipeline` fanout/optimizer orchestration and `board.optimize`).
**M4 carry-forwards (from the M3 adjudication, priority order):**
1. **#61 events-divergence close FIRST** (task #61; Classes A/B/C, buglog 172–174; exit = `events compare` exits 0). Class B door-slicing first — hypothesized causal ancestor of the bm06/bm11 victim-choice drift (reasoning in design §6 M3 note bm06 lever cell).
2. The three completion reds are M4's own exit criteria: bm06/bm11 tuning levers (buglog 181), bm01 per-pass search cost (natural end ≈8.5–13.4h must shrink; M5 arena/parallelism as backstop).
3. CI flip at M4's moment, per the one-commit protocol above; stale rust-check.yml compare-step comment (pre-adjudication bm11 framing) refreshed at the FIRST workflow-touching M4 commit.
4. M2 leftovers still open: the four cut pub items return with their first consumer, pinned then.
5. Banked inert mutants (hoist-if-third-copy `item_is_drillable` twins; QR-2/QR-3b/QR-7/Q2/Q3/Q7, MIN-3, buglog 178, ViaOptimizer POINTS sites) — revisit opportunistically; chore(wolf) banked commit (cerebrum consolidation, buglog ids >95 unstaged) when convenient.
### Acceptance criteria (M4, from design §6)
1. Full pipeline (fanout + detail + optimizer) runs on the fixture set.
2. All fixtures ≥ Java on completion / violations / score — the zero-regression moment.
3. Gates green; zero regressions on all M2/M3 compares.
### Open decisions
- None blocking; optimizer decomposition (pull-tight → shove → via opt staging) resolved during planning from the Java pass structure.

## 📁 Active architecture

- `rust/` workspace (run cargo from `rust/`): `epic-geometry`, `epic-dsn`, `epic-board`, `epic-rules`, `epic-drc`, `epic-search`, `epic-router` (maze/expansion/drill/path/ripup + SEAM.md), `epic-engine` (13-line M4 scaffold), `epic-cli`, `harness` (epic-harness: corpus/compare/battery/determinism/events/tripwire).
- Oracle: Java tree frozen at `e7f9bdf1` (`src/` read-only), jar run via JDK 25 `~/.jdks/jdk-25.0.4.1+1/bin/java`; `EPIC_SKIP_GRADLE=1` for compares.
- Committed artifacts never modified: `rust/harness/baselines/**`, corpus dirs, events-golden (`cc50216c…`), `tiers.yaml`.
- CI: `.github/workflows/rust-check.yml` (fmt + clippy `-D warnings` + tests + java-free corpus compare `--report-only`); flip guarded by `rust/harness/src/ci_tripwire.rs` pins.
- Records: `rust/crates/epic-router/SEAM.md` (differential ledger + milestone-criterion map), `.wolf/buglog.json` (181 entries; committed file tops out at id 95 — working tree is authoritative), `logs/M3-T17/` (T17 review chain + milestone adjudication + bm06/bm01/bm11 evidence).

## ⚠️ External blockers

- None. (Watchdog: never kill/modify processes that are not ours — another Claude instance runs on this machine.)

## 🔧 Useful commands

- Gates (from `rust/`): `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace` → **1305/0/15**.
- Compares (repo root, harness bin, `EPIC_SKIP_GRADLE=1`, explicit command lines, real exit codes — never pipe a gate in fish): corpus / dsn / ses / ses-snap / index / undo / drc; `router determinism` digests; `events compare` (currently exit 1 = #61).
- Battery: release profile, CHEAP subset only for probes (bm08, ecc83-pp, ecc83-pp_v2); NEVER bm01/bm11 unbounded, never bm06 in debug; never re-run the one-off captures (evidence on disk in `logs/M3-T17/`).
- JDK 25 for oracles; `cd` every Bash call (fish); commits end with `Co-Authored-By: Claude Code <noreply@anthropic.com>`; never push (no remote on epic/main).
