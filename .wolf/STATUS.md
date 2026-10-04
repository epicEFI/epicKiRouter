# STATUS — session handoff

Last updated: 2026-10-04 (152-H landed; post-baseline upstream sweep closed)

## ✅ Done

- **M0–M11 ALL COMPLETE; M10-T4 sunset the Java tree** (git history +
  grafted `origin/master` at `e7f9bdf1` keep it reachable). The parity
  record post-sunset = the committed goldens (`rust/harness/baselines/`,
  corpus dirs, `tiers.yaml`) + the digest literals in AMENDMENT 3 of the
  M10 sunset plan.
- **Readiness campaign verdict READY** (2026-10-02,
  `logs/readiness-2026-10-01/verdict.md`): E2E batches closed, live
  SIGINT probe 5/5 graceful, crash family 2-found/2-fixed/2-live-verified.
- **Tier-1 ports landed**: P1 #935a `58ae2f5c0`+`96492284c`, P2 #935b
  `ca0e785cc`, P3 #925a `7453ec611`, P4 #925b `e61998820`, P5
  `f0fdb120d`, P6 #811 `e986721b6`.
- **Tier-2 landed**: #931 (all 9 clusters, M11), #933 s1/s2/s3,
  #152 A `156ecbfa8` / B `1ad3a8f5e` / G `51e6b895d` / H `4de7509da`
  (the zone-island DRC family COMPLETE), #885 `f75222bf0`, #930
  `e1f7d173f`, cgrid `dbe1f1caa`, bug-255 `1a0a6ce92`.
- **Post-baseline upstream sweep CLOSED 2026-10-04**
  (`logs/readiness-2026-10-01/upstream-sweep-2026-10-04.md`): all 117
  commits in `e7f9bdf1..upstream/master` dispositioned — every engine
  commit landed, rest N/A with code-level reasons. Ride-along law: if
  worker-board reuse (#19) ever lands, upstream 8dcd4cd68's maze-state
  clearing MUST ride.
- **Census law: 42/1827/0/19** (152-H battery, per-component
  reconciled). Buglog through id 268. Export chain tip `8e5e81e5a`
  (epicki/main).

## 🚀 Next quest

**Standing /loop verification-and-hardening campaign** (runs until
Tyler says stop): gates/tests (every cargo capped
`systemd-run --user --scope -p MemoryMax=6G`; census arithmetic
declared before runs, reconciled against constituents), hunt
bugs/gaps/holes with tests, land ranked improvements, export-push
every landing.
**Open queue (all user-gated or conditional):**
1. #8 large-files strip (13 × >50MB in epicki) — Tyler's decision.
2. B1 mixed trace widths — blocked on Tyler's .dsn.
3. #19 worker-board reuse — only if memory pressure justifies (then
   the 8dcd4cd68 ride-along applies).
4. global-router-robustness round — optional; T4 stash@{1} C+D WIP
   parked.
5. Any new upstream fetch → extend the sweep table BEFORE touching
   code.

## 📁 Active architecture

- `rust/` workspace (run cargo from `rust/`): `epic-geometry`,
  `epic-index`, `epic-board`, `epic-dsn`, `epic-drc`, `epic-router`,
  `epic-engine`, `epic-cli`, `epic-gui`, `epic-harness` (harness/).
- Runtime: DSN → epic-dsn → epic-board → epic-router (fanout → batch
  passes → optimizer) → epic-drc → SES. GUI egui, CLI headless.
- The engine is SINGLE-THREADED per run (no routing thread spawns);
  epic-index's only atomic is an id counter — upstream #798's
  search-tree race is N/A by architecture.
- Goldens immutable; new goldens only via sanctioned capture doors.
- GitHub CI RETIRED for the standalone era — the authoritative gate
  battery is LOCAL (fmt + clippy `-D warnings` + census + compares +
  tierA + gv faces via `logs/readiness-2026-10-01/run_gates_*.sh`).
- Export law: epic/main product commits → epicki orphan snapshots
  (`git rev-parse <sha>^{tree}` → read-tree/write-tree MATCH assert →
  `commit-tree -p <prev-export>` → bare `git push epicki`); chore(wolf)
  never exported; no version bump.

## ⚠️ External blockers

- None. (Never touch other Claude sessions; defer heavy runs while
  the other instance loads the CPU.)

## 🔧 Useful commands

- Gates: clone the newest `logs/readiness-2026-10-01/run_gates_*.sh`
  (26 gates: both-profile cleans, fmt, clippy ×2, census, det1,
  threads, 8 compares, tierA, 7 gv faces). Audit per-gate `exit=`
  lines — the scripts are NOT fail-fast.
- Census: `cargo test --workspace` → **42/1827/0/19** (declare
  arithmetic before running; reconcile per-crate after).
- Staleness law (buglog 265): after tests-only edits, `cargo clean -p
  epic-cli` BOTH profiles or the census tripwire sees a stale bin.
- fish traps: no `===` separators; heredocs/python → script files.
- Commits end with `Co-Authored-By: Claude Code <noreply@anthropic.com>`;
  the `git push epicki` jail carve-out wants the push as its OWN
  command (compounds blocked).
