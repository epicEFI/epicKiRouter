---
description: session handoff, regenerate with /handoff when a quest finishes
budget_tokens: 1000
---
# STATUS — EpicRouter

> Single source of truth for resuming work. Read this FIRST when starting a session.
> Update this file at the end of every work phase so the next `/clear` resumes in 1 read.
> Last updated: 2026-09-15 (M2 complete)

---

## ✅ Done

- **M0 complete** (2026-09-12): Rust workspace, 23 golden baselines (A=11/B=9/C=3), CI, docs.
- **M1a complete** (2026-09-13): `epic-geometry` bit-parity port, 5000/5000 differential corpus green (java-free CI).
- **M1b complete** (2026-09-14): `epic-dsn` full parse parity — 1,332/1,332 fixtures byte-identical digests; SES writer byte-equal on 20/20 (T40 snap deferred to M2 per D11).
- **M2 complete** (2026-09-15, plan `8b6e1cc1`, 16/16 tasks closed, range `8b6e1cc1..f9685cba`): `epic-board` (arena board, undoable containers, rules/placement, item geometry, tree-manager seam, contacts, trace combine/split/normalize, `normalize_all_traces`, undo facade, session-contacts provider) + `epic-index` (ShapeTree/MinAreaTree, ShapeSearchTree config/manager/compensation, tree-shapes, query family). **All six compares green**: corpus 5000/5000, dsn 1,332/1,332 ledger EMPTY (D22 retired at T13), ses-compare 20/20 byte-stable, index 33/33, undo 33/33, ses-snap 5/5 (snapFired=0 canary — snap output-arm provably dead upstream, spec-adjudicated). **927 workspace tests**. Exit docs (`9f6c9a33`) + milestone review round (`f9685cba`). Final milestone review: spec SPEC_COMPLIANT, quality APPROVED_WITH_MINORS→APPROVED.
- M2 deferred items (recorded, NOT debt): dsn-0151 ledger retired; T40 snap landed with the dead-arm finding; write_net `Emittable` enum done (D24).
- Baseline truths (M0): `multichannel_mixer` 160/203 incomplete (score 141.22) = improvement target; DRC violations on 7 boards (worst 285).

---

## 🚀 Next quest

**Goal:** M3 — Detail routing core + `epic-cli` (design §6 M3 row). Exit criteria: single-net quality ≥ Java; Tier A per-fixture completion ≥ Java and violations ≤ Java.

First step: write the M3 plan (writing-plans skill, from design §5/§6 + recon of `autoroute/` Java sources: `autoroute.maze`, `autoroute.expansion`, `autoroute.drill`, `autoroute.path`, pipeline orchestration).

**M3 carry-forwards (from the M2 milestone review, priority order):**
1. Extract `harness/src/corpus_common.rs` (manifest entry, strict JSONL load, alignment, sha-hex) BEFORE the routing corpus is born — the shell is triplicated across dsn/index/undo corpora with visible drift (truncate caps 120 vs 160).
2. The four cut pub items (`Component::is_placed`, `NetClass::is_active_routing_layer`, `BoardRules::default_net_class`, `BoardRules::via_rule_by_id`) return with their first M3 consumer, pinned at that point.
3. If M3 adds a second tree to the undo replay, the undo-digest topology blind spot (documented in `undo_corpus.rs` coverage boundary) must be closed first.

### Acceptance criteria (M3, from design)
1. Single-net routing quality ≥ Java on the differential set.
2. Tier A per-fixture completion ≥ Java and violations ≤ Java.
3. Gates green; zero regressions on all six M2 compares.

### Open decisions
- None blocking; routing-engine decomposition (maze core seam vs expansion strategy) resolved during planning.

---

## 📁 Active architecture

- **Stack:** Rust 1.93.0 workspace in `rust/`; Java 25 oracle (`~/.jdks/jdk-25.0.4.1+1`, system java is 21); Gradle 9 via `./gradlew`.
- **Key crates:** `epic-geometry`/`epic-dsn`/`epic-board`/`epic-index` (done), `epic-{drc,router,engine,cli,gui}` (later) + `rust/harness` (corpus/dsn/ses_compare/index_corpus/undo_corpus/oracle/tiers/baseline + the four spike oracles).
- **Patterns:** parity gates compare digests/canonical bytes, never wall time; goldens are committed artifacts, regenerate only via the harness subcommands with justification; broken environments fail LOUD; pins follow the cerebrum rules (jar-captured, non-degenerate, branch-covering, anchor-blind-aware; modes 6-9: memoized-accessor, equality-contrast, literal-capture-rows, winner-selection).

---

## ⚠️ External blockers (don't block coding)

- Oracle runs need the jar (`./gradlew -q executableJar`) + JDK 25 (harness auto-resolves; `EPIC_ORACLE_JAVA` overrides; `EPIC_SKIP_GRADLE=1` skips build).
- Never push to `upstream`; never modify `src/` Java sources (frozen at `e7f9bdf1`; `rust/harness/oracle/*.java` is harness-side, editable); never touch other Claude instances/global `~/.claude`.

---

## 🔧 Useful commands

```bash
cd rust
cargo test --workspace                                   # unit tests, no Java needed (927)
EPIC_SKIP_GRADLE=1 cargo run -q -p epic-harness -- corpus compare    # 5000/5000
EPIC_SKIP_GRADLE=1 cargo run -q -p epic-harness -- dsn compare       # 1332/1332, ~200s
EPIC_SKIP_GRADLE=1 cargo run -q -p epic-harness -- dsn ses-compare   # 20/20 byte-equal
EPIC_SKIP_GRADLE=1 cargo run -q -p epic-harness -- index compare     # 33/33
EPIC_SKIP_GRADLE=1 cargo run -q -p epic-harness -- undo compare      # 33/33
EPIC_SKIP_GRADLE=1 cargo run -q -p epic-harness -- dsn ses-snap-compare  # 5/5
./gradlew test                                           # Java suite ~6 min (Display8DigitRoutingTest load-flaky; rerun solo)
```

---
