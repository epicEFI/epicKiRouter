---
description: session handoff, regenerate with /handoff when a quest finishes
budget_tokens: 1000
---
# STATUS — EpicRouter

> Single source of truth for resuming work. Read this FIRST when starting a session.
> Update this file at the end of every work phase so the next `/clear` resumes in 1 read.
> Last updated: 2026-09-12 (M0 complete)

---

## ✅ Done

- **M0 complete** (2026-09-12, `5508eaaf`…`10648eb9` on `epic/main`): Rust workspace (9 scaffold crates + `epic-harness`), 23 golden baselines (A=11/B=9/C=3, all COMPLETED, verify-proven deterministic), CI `rust-check.yml`, README + CLAUDE.md Rust section. Exit criteria green (cargo 30/0/1-ignored; gradle 558, sole failure = documented load-flake, solo rerun green). Pipeline: subagent-driven, spec+quality review per task.
- Baseline truths: `multichannel_mixer` 160/203 incomplete (score 141.22) = improvement target; DRC violations on 7 boards (worst 285); oracle `git_sha:"unknown"` uniformly (ungated); `fixture_sha256` populated (free for M3 identity gate).

---

## 🚀 Next phase

**Goal:** M1a in flight — port the geometry kernel to `epic-geometry` with bit-parity proven by a differential corpus (plan `26d9941e`, 11 tasks). M1b (epic-dsn) plan follows M1a.

### Acceptance criteria (M1a)
1. Corpus ≥5k seeded cases, 100% exact match vs Java oracle (doubles as raw bits).
2. Algebraic invariant tests green (commutativity, idempotence, sort stability).
3. `cargo test/clippy/fmt` green; `corpus compare` wired into CI (no JDK needed).

### Files to create / edit
| Type | File | Content |
|---|---|---|
| done | `docs/superpowers/plans/2026-09-12-epicrouter-m1a-geometry-kernel.md` | M1a plan (committed `26d9941e`) |
| mod | `rust/crates/epic-geometry/src/**` | i32+Java-wraparound, double predicates, num-bigint fallback (NOT i128 — deferred to M5) |
| new | `rust/harness/oracle/GeometryCorpusOracle.java` | single-file Java evaluator (outside frozen src/) |
| new | `rust/harness/src/corpus.rs` | corpus generate/golden/compare subcommands |

### Closed decisions
- Strategy C (port exact kernel + I/O, reinvent router) — user-approved.
- M1 split: M1a geometry+corpus first, then M1b DSN/SES (independent subsystems).
- Parity-first arithmetic (D1): mirror Java int/long/double/BigInteger bit-for-bit; wraparound is load-bearing; i128 widening = M5.
- num-bigint 0.4 = only new dep; no serde in epic-geometry; java.util.Random LCG ported (PolygonShape reseeds Random(99)).

### Open decisions
- (none blocking)

---

## 📁 Active architecture

- **Stack:** Rust 1.93.0 (edition 2024, pinned) workspace in `rust/`; Java 25 oracle (`~/.jdks/jdk-25.0.4.1+1`, system java is 21); Gradle 9 via `./gradlew`.
- **Key modules:** `rust/crates/epic-{geometry,dsn,board,index,drc,router,engine,cli,gui}` (scaffolds until their milestone) + `rust/harness` (`tiers`/`manifest`/`oracle`/`baseline`; capture/verify/list CLI).
- **Patterns:** parity gates never compare textual SES; wall time is trend-only; broken environments must fail LOUD; baselines are committed artifacts, regenerate only via `epic-harness capture` with justification.

---

## ⚠️ External blockers (don't block coding)

- Oracle runs need the jar (`./gradlew -q executableJar`) + JDK 25 (harness auto-resolves; `EPIC_ORACLE_JAVA` overrides; `EPIC_SKIP_GRADLE=1` skips build).
- Never push to `upstream`; never modify `src/` Java sources (frozen at `e7f9bdf1`); never touch other Claude instances/global `~/.claude`.

---

## 🔧 Useful commands

```bash
cd rust
cargo test --workspace                                   # unit tests, no Java needed
cargo test -p epic-harness -- --ignored                  # e2e (jar + JDK 25)
EPIC_SKIP_GRADLE=1 cargo run -p epic-harness --release -- list
EPIC_SKIP_GRADLE=1 cargo run -p epic-harness --release -- verify --tier A   # ~20 min
./gradlew test                                           # Java suite ~6 min (Display8DigitRoutingTest is load-flaky; rerun solo)
```

---

## 📚 References (read IF needed)

- `.wolf/cerebrum.md` — User Preferences + Do-Not-Repeat + Decision Log
- `.wolf/anatomy.md` — token-efficient file index (query with `openwolf find`)
- `.wolf/buglog.json` — known bugs + fixes (~13 entries incl. GUI-interception bug-007)
- Design: `docs/superpowers/specs/2026-09-11-epicrouter-rust-rewrite-design.md`; M0 plan: `docs/superpowers/plans/2026-09-11-epicrouter-m0-rust-scaffold-and-baselines.md`
