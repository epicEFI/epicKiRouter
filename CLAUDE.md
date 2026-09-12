# OpenWolf

This project uses OpenWolf for context management. The always-on rules live in `.claude/rules/openwolf.md`; the hooks handle bookkeeping (anatomy index, memory log, read tracking) automatically.

For the full operating protocol (session handoff, memory discipline, bug logging), load the `openwolf` skill, or read `.wolf/OPENWOLF.md`. Regenerate the session handoff with `/handoff`.


# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Context

This is **EpicRouter**, a fork of [Freerouting](https://github.com/freerouting/freerouting) (the post-2026-refactor codebase) intended as a ground-up improvement of the PCB autorouter: speed, routing quality, DRC accuracy, and human-quality route aesthetics — with zero regressions vs the Freerouting baseline.

- Read `AGENTS.md` first for the full project persona, deep architecture notes (DRC internals, copper pours, settings merger invariants, analytics, MCP), and investigation history. It is the canonical long-form reference.
- `docs/architecture.md` is the authoritative architecture map with a package glossary and Mermaid diagram — keep it updated when structure changes.
- `src_v19/` is the frozen v1.9 historical tree, compiled for compatibility only. Never refactor, format, or Checkstyle it.
- The routing/correctness baseline for parity work is stable **Freerouting v2.3.0** (not v1.9).

## Build & Test Commands

Java 25, Gradle 9 (use `./gradlew`):

**Environment setup (this machine):**
- Temurin JDK 25 is installed at `~/.jdks/jdk-25.0.4.1+1` (Gradle auto-detects it). The system default JDK is 21 — insufficient.
- The tree requires a git repo (`build.gradle` embeds `git rev-parse HEAD`); it fails with exit 128 otherwise. Upstream history is grafted as `origin/master` (baseline commit `e7f9bdf1`).
- Do not rely on Gradle toolchain auto-provisioning (foojay-resolver 0.8.0 + Gradle 9.7.1 fails with `JvmVendorSpec.IBM_SEMERU` missing).

- `./gradlew test` — fast unit tests (default; excludes slow/gui)
- `./gradlew testSlow` — slow fixture/benchmark tests only (`@Tag("slow")`, enable with `-PincludeSlowTests=true`)
- `./gradlew testAll` — fast + slow + serial + gui
- `./gradlew check` — full verification (Spotless + Checkstyle + tests incl. serial)
- Single test: `./gradlew test --tests app.freerouting.fixtures.Dac2020Bm01RoutingTest` (quickest routing smoke test)
- `./gradlew executableJar` — build runnable JAR → `build/libs/freerouting-current-executable.jar`
- `./gradlew run` — run current dev build; `./gradlew runV19` — run legacy v1.9 build
- `./gradlew spotlessCheck checkstyleMain checkstyleTest checkstyleRewriteRecipes` — quality gates (run before handoff)
- `pre-commit run --all-files` — repo hygiene hooks
- Headless routing: `java -jar build/libs/freerouting-current-executable.jar -de board.dsn -do out.ses`

Test-task timeout budget is 30 minutes. Bound long routing tests with `TestingSettings.setMaxPasses(n)` / `setMaxItems(m)`; use `setJobTimeoutString("HH:MM:SS")` only as a supplement — the timeout fires after a pass completes.

## Rust rewrite (EpicRouter)

The approved design lives at
`docs/superpowers/specs/2026-09-11-epicrouter-rust-rewrite-design.md`; the M0
plan at `docs/superpowers/plans/2026-09-11-epicrouter-m0-rust-scaffold-and-baselines.md`.
The Rust workspace is `rust/` (run cargo from there). Rules:

- NEVER modify Java sources under `src/` — the Java engine is the parity
  oracle frozen at `e7f9bdf1` until M10.
- Golden baselines in `rust/harness/baselines/` are committed artifacts;
  regenerate only via `epic-harness capture` and explain why in the commit.
- The oracle jar needs JDK 25 (`~/.jdks/jdk-25.0.4.1+1/bin/java`), not the
  system Java 21; `epic-harness` resolves this automatically.
- CI (rust-check.yml) runs fmt + clippy `-D warnings` + unit tests on any
  commit touching `rust/`; keep clippy clean including `clippy::unwrap_used`.

## Quality Gate Rules (do not violate)

- **Never** run `spotlessApply` as automatic cleanup — it rewrites hundreds of files. Use `spotlessCheck`.
- Never stage files automatically; keep formatting-only changes separate from functional ones.
- Don't touch `core.autocrlf` to fix line endings; inspect `.gitattributes` and the diff instead.
- Run `python scripts/i18n/extract-context.py --check` when Java or translation sources change.

## Architecture

Runtime flow: `DSN input → io.specctra parser → board/rules model → autorouter → drc → SES output`. GUI (Swing), REST API (`api.v1`/`api.mcp`), and job scheduler (`management`) all build on this headless core pipeline.

### Routing engine (the algorithmic heart)

- `board` — live board model: `board.model.items` (pins/vias/traces/keepouts), `board.facade` (`BasicBoard`, `RoutingBoard`), `board.searchtree` (spatial indexes for clearance/overlap queries), `board.trace` (polyline geometry + normalization), `board.optimize` (pull-tight/shove/via optimization).
- `autoroute.pipeline` — stage orchestration (fanout → batch autorouter passes → optimizer passes); `autoroute.maze` (maze search engine), `autoroute.expansion` (expansion rooms/doors), `autoroute.drill`, `autoroute.path` (found-path reconstruction/insertion).
- `rules` — nets, clearance classes, via rules; `drc` — violation detection; `geometry.planar` — shape/point math used everywhere.
- Two distinct scores, both 0–1000: **router score** (incomplete connections + DRC) and **optimizer score** (length/via/bend excess vs lower bounds). See `docs/scoring.md`.

### Boundaries (enforced by ArchUnit — keep green)

- Core packages (`autoroute`, `board`, `rules`, `drc`, `geometry`) must not depend on GUI or API packages.
- Headless paths (`api`, `management`, `core`) must not touch `GuiBoardManager`/`InteractiveState`/`WorkspaceSettings`.
- `io.specctra.parser` internals are private to `io.specctra` public entry points.
- Boundary rules live in `src/test/java/app/freerouting/architecture/ModuleBoundariesArchTest.java`; accepted debt is listed in `docs/architecture.md` and `docs/issues/soc-gui-separation-and-accessibility-plan.md` §12.

### Settings system invariant

`SettingsMerger` layers nine `SettingsSource`s by priority (0=defaults → 70=API), copying a field only when non-null and non-default. Therefore **all `RouterSettings` fields must be nullable reference types with no initializers** — an initialized field would silently break source precedence. Defaults belong only in `DefaultSettings.getSettings()`.

### GUI vs headless

`HeadlessBoardManager` is the base; `GuiBoardManager` adds Swing/`WorkspaceSettings`/`.frb` serialization. Shared logic goes in headless; anything needing a display goes in GUI. `WorkspaceSettings.getWorkspaceSettings()` returns `null` in headless mode by design — guard before use.

## Critical Project Knowledge

- **Routing algorithm changes are high-risk.** Minor changes can regress trace optimization, clearance, or completion rates. Always validate against the test suite plus real `.dsn` fixtures (`src/test/java/app/freerouting/fixtures/RoutingFixtureTest.java`, boards in `fixtures/`, 160+ real boards).
- **DRC completeness:** `BoardStatistics.clearanceViolations.totalCount` is incomplete (uses outline-only count). Use `DesignRulesChecker.getAllClearanceViolations()` for real violation counts.
- **KiCad quirk:** DSN export omits copper-to-edge clearance, so Freerouting routes to default class at the board edge and KiCad DRC flags it afterwards (see `docs/issues/Issue558-...md`).
- **Copper pours:** modeled as `ConductionArea`; `rules.Net.contains_plane` switches plane-routing mode (stub+via to plane). Known open bug: plane routing can introduce clearance violations (Issue 093/152).
- **Logging:** use `FRLogger.trace(method, operation, message, impactedItems, impactedPoints)` for algorithmic detail; keep diagnostic payloads synchronized between WIP and baseline builds before diffing logs.
- **Regression workflow:** compare WIP vs v2.3.0 on golden fixtures; classify divergence (numeric drift vs ordering/tie-break) before fixing; exit criteria = no new clearance violations + no completion regression + stable compare metrics.
- Issue specs live in `docs/issues/`; scratch analysis goes in git-ignored `logs/<IssueNNN>/`.
- Test placement: issue-regression/full-pipeline → `fixtures/` package; package-scoped unit tests → matching test package.
- License: GPLv3 — keep dependencies compatible. The product name in prose is always "**Freerouting**".
