---
description: session handoff, regenerate with /handoff when a quest finishes
budget_tokens: 1000
---
# STATUS — EpicRouter

> Single source of truth for resuming work. Read this FIRST when starting a session.
> Update this file at the end of every work phase so the next `/clear` resumes in 1 read.
> Last updated: 2026-09-14 (M1b complete)

---

## ✅ Done

- **M0 complete** (2026-09-12): Rust workspace, 23 golden baselines (A=11/B=9/C=3), CI, docs.
- **M1a complete** (2026-09-13): `epic-geometry` bit-parity port, 5000/5000 differential corpus green (java-free CI).
- **M1b complete** (2026-09-14, plan `a3eab9e3`, 14/14 tasks closed): `epic-dsn` full DSN parse parity — **1,332/1,332 fixtures** (175 digest + 1,157 soak) byte-identical digests vs the Java oracle; one documented ledgered divergence (dsn-0151 `normalizeAllTraces`, deferred to M2 per D11). SES writer (`ses/writer.rs` + `write_scope.rs`) emits **byte-equal sessions on all 20 tier A+B fixtures** (`dsn ses-compare`, exceeds the canonical-green criterion; T40 endpoint-snap classifier in place, pinned, expected to fire only post-M2). Both gates CI-wired java-free (`dsn compare` 22.7s, `dsn ses-compare` 0.2s). T13 reviews: spec SPEC_COMPLIANT (apostrophe pin fix `f0ce4218`), quality APPROVED_WITH_MINORS (5/6 minors fixed `867024c6`; minor 2 — write_net `unreachable!` → enum restructure — deferred to M2, pinned-unreachable).
- M1b key commits: `657a070e` (T13-A) `ddd1c7c9` (B1) `c5a7a453` (B2) `8aa2d422` (B3) `11a65b4c` (B4) `b3f80cd2` (T14 docs/CI) `f0ce4218` `867024c6` (review fixes).
- Baseline truths (M0): `multichannel_mixer` 160/203 incomplete (score 141.22) = improvement target; DRC violations on 7 boards (worst 285).

---

## 🚀 Next quest

**Goal:** M2 — `epic-board` (arena board model) + `epic-index` (incremental spatial index). Design §6 M2 row: "Index query-set parity; snapshot/undo correct." Includes the deferred M1b items: dsn-0151 `normalizeAllTraces` (prune the ledger entry once ported), T40 endpoint snapping (contact-aware, unlocks post-routing SES parity), write_net enum restructure.

First step: write the M2 plan (writing-plans skill, from design doc §5/§6 + recon of `board/` Java sources: `board.facade`, `board.model.items`, `board.searchtree`, `board.trace` normalization).

### Acceptance criteria (M2, from design)
1. Index query-set parity vs Java searchtree (differential corpus, java-free compare in CI).
2. Snapshot/undo correctness (Java `UndoableObjects`/`board.undo` semantics).
3. Gates green; zero regressions on the 1,332-fixture parse + 20-fixture SES gates.

### Open decisions
- None blocking; M2 plan decisions (arena vs SoA layout timing, undo log shape) resolved during planning.

---

## 📁 Active architecture

- **Stack:** Rust 1.93.0 workspace in `rust/`; Java 25 oracle (`~/.jdks/jdk-25.0.4.1+1`, system java is 21); Gradle 9 via `./gradlew`.
- **Key crates:** `epic-geometry` (done), `epic-dsn` (done: lexer/keyword/shape/scope readers → `SesBoard` IR + `ses::writer`), `epic-board`/`epic-index` (next), `epic-{drc,router,engine,cli,gui}` (later) + `rust/harness` (corpus/dsn/ses_compare/oracle/tiers/baseline).
- **Patterns:** parity gates compare digests/canonical bytes, never wall time; goldens are committed artifacts, regenerate only via the harness subcommands with justification; broken environments fail LOUD; pins follow the cerebrum rules (jar-captured, non-degenerate, branch-covering, anchor-blind-aware, no formatting-toString capture).

---

## ⚠️ External blockers (don't block coding)

- Oracle runs need the jar (`./gradlew -q executableJar`) + JDK 25 (harness auto-resolves; `EPIC_ORACLE_JAVA` overrides; `EPIC_SKIP_GRADLE=1` skips build).
- Never push to `upstream`; never modify `src/` Java sources (frozen at `e7f9bdf1`; `rust/harness/oracle/*.java` is harness-side, editable); never touch other Claude instances/global `~/.claude`.

---

## 🔧 Useful commands

```bash
cd rust
cargo test --workspace                                   # unit tests, no Java needed
EPIC_SKIP_GRADLE=1 cargo run -q -p epic-harness -- corpus compare    # 5000/5000
EPIC_SKIP_GRADLE=1 cargo run -q -p epic-harness -- dsn compare       # 1332/1332, ~23s
EPIC_SKIP_GRADLE=1 cargo run -q -p epic-harness -- dsn ses-compare   # 20/20 byte-equal
./gradlew test                                           # Java suite ~6 min (Display8DigitRoutingTest load-flaky; rerun solo)
```

---

## 📚 References (read IF needed)

- `.wolf/cerebrum.md` — User Preferences + Do-Not-Repeat + Decision Log
- `.wolf/anatomy.md` — token-efficient file index (query with `openwolf find`)
- `.wolf/buglog.json` — known bugs + fixes
- Design: `docs/superpowers/specs/2026-09-11-epicrouter-rust-rewrite-design.md`; M1b plan: `docs/superpowers/plans/2026-09-13-epicrouter-m1b-dsn-parser.md`
