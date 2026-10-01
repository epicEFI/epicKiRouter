# OpenWolf

This project uses OpenWolf for context management. The always-on rules live in `.claude/rules/openwolf.md`; the hooks handle bookkeeping (anatomy index, memory log, read tracking) automatically.

For the full operating protocol (session handoff, memory discipline, bug logging), load the `openwolf` skill, or read `.wolf/OPENWOLF.md`. Regenerate the session handoff with `/handoff`.


# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Context

This is **EpicRouter**, the Rust rewrite of [Freerouting](https://github.com/freerouting/freerouting) (GPLv3, a fork of the post-2026-refactor Java codebase) intended as a ground-up improvement of the PCB autorouter: speed, routing quality, DRC accuracy, and human-quality route aesthetics — with zero regressions vs the Freerouting baseline. The product name in prose is always "**Freerouting**" where it refers to the upstream project.

- Read `AGENTS.md` for the deep Java architecture notes (DRC internals, copper pours, settings merger invariants, analytics, MCP) and investigation history — **historical record of the sunsetted Java tree** (deleted at M10-T4, reachable in git history and via the grafted `origin/master` at `e7f9bdf1`).
- `docs/architecture.md`: the Java-package map is marked HISTORICAL; the Rust workspace (`rust/`) is the product.
- `src_v19/` and the Java tree lived under `src/` until the M10-T4 sunset; both are git history now.
- The parity record post-sunset: the committed goldens (`rust/harness/baselines/`, corpus dirs, `rust/harness/config/tiers.yaml`) + the committed digest literals in AMENDMENT 3 of `docs/superpowers/plans/2026-09-30-epicrouter-m10-sunset-and-release.md`.

## Build & Test Commands

Rust workspace at `rust/` (run cargo from there), toolchain pinned 1.93:

**Environment setup (this machine):**
- Rust toolchain via rustup, pinned 1.93 (`rust/` rust-toolchain).
- Upstream history is grafted as `origin/master` (baseline commit `e7f9bdf1`).
- Historical (Java era): the gradle/JDK notes lived here; the Java tree was deleted at M10-T4 (commit history has the full build story, the parity oracle jar was built with JDK 25 at `~/.jdks/jdk-25.0.4.1+1`).

- `cargo build` — build the workspace
- `cargo test --workspace` — the test census (fast pins + harness pins; the engine census ledger lives in the milestone plans)
- `cargo test -p <crate>` — single crate
- `cargo run -p epic-harness --release -- <subcommand>` — the parity/comparison harness (compare faces are java-free; the capture family is retired with the sunset — see `rust/harness/src/oracle.rs`)
- `cargo run -p epic-cli -- -de board.dsn -do out.ses` — headless routing
- `cargo clippy --workspace --all-targets -- -D warnings` — lint gate (incl. `clippy::unwrap_used`); desktop-feature cargo additionally capped `--jobs 4`
- `pre-commit run --all-files` — repo hygiene hooks

**Every cargo/test/harness/pre-commit command on this box is capped `systemd-run --user --scope -p MemoryMax=6G`** (buglog 194; desktop-feature cargo also `--jobs 4`).

Test-task timeout budget is 30 minutes. Bound long routing tests with tier timeouts; timeouts are never raised.

## Quality Gate Rules (do not violate)

- Never stage files automatically (no `git add -A`); keep formatting-only changes separate from functional ones.
- Don't touch `core.autocrlf` to fix line endings; inspect `.gitattributes` and the diff instead.
- Golden baselines in `rust/harness/baselines/`, the corpus dirs, `rust/harness/config/tiers.yaml`, and ALL goldens are committed artifacts — never modified; NEW goldens are NEW files via sanctioned capture only.
- CI (rust-check.yml) runs fmt + clippy `-D warnings` + unit tests + the geometry corpus compare (java-free, committed goldens) on any commit touching `rust/`; keep clippy clean including `clippy::unwrap_used`.

## Architecture

Runtime flow: `DSN input → epic-dsn parser → epic-board model → epic-router (fanout → batch autorouter passes → optimizer) → epic-drc → SES output`. GUI (epic-gui, egui), CLI (epic-cli), and the harness all build on this core pipeline. The workspace crates: `epic-geometry` (shape/point math), `epic-index` (spatial index), `epic-board`, `epic-dsn`, `epic-drc`, `epic-router`, `epic-engine` (session), `epic-cli`, `epic-gui`, `epic-harness`.

### Historical (Java era) architecture knowledge

The sections below describe the sunsetted Java engine — preserved as history and because the Rust engine ports its semantics; the authoritative map is `docs/architecture.md` (Java faces marked HISTORICAL, Rust section current).

### Routing engine (the algorithmic heart)

- `board` — live board model: `board.model.items` (pins/vias/traces/keepouts), `board.facade` (`BasicBoard`, `RoutingBoard`), `board.searchtree` (spatial indexes for clearance/overlap queries), `board.trace` (polyline geometry + normalization), `board.optimize` (pull-tight/shove/via optimization).
- `autoroute.pipeline` — stage orchestration (fanout → batch autorouter passes → optimizer passes); `autoroute.maze` (maze search engine), `autoroute.expansion` (expansion rooms/doors), `autoroute.drill`, `autoroute.path` (found-path reconstruction/insertion).
- `rules` — nets, clearance classes, via rules; `drc` — violation detection; `geometry.planar` — shape/point math used everywhere.
- Two distinct scores, both 0–1000: **router score** (incomplete connections + DRC) and **optimizer score** (length/via/bend excess vs lower bounds). See `docs/scoring.md`.

### Boundaries (was enforced by ArchUnit — historical)

- Core packages (`autoroute`, `board`, "rules", `drc`, `geometry`) must not depend on GUI or API packages.
- Headless paths (`api`, `management`, `core`) must not touch `GuiBoardManager`/`InteractiveState`/`WorkspaceSettings`.
- `io.specctra.parser` internals are private to `io.specctra` public entry points.

### Settings system invariant (historical — the Rust engine inherits the layering)

`SettingsMerger` layers nine `SettingsSource`s by priority (0=defaults → 70=CLI/API), copying a field only when non-null and non-default. Therefore **all `RouterSettings` fields must be nullable reference types with no initializers** — an initialized field would silently break source precedence. Defaults belong only in `DefaultSettings.getSettings()`.

### GUI vs headless (historical)

`HeadlessBoardManager` is the base; `GuiBoardManager` adds Swing/`WorkspaceSettings`/`.frb` serialization. Shared logic goes in headless; anything needing a display goes in GUI.

## Critical Project Knowledge

- **Routing algorithm changes are high-risk.** Minor changes can regress trace optimization, clearance, or completion rates. Always validate against the Rust census + the harness compares (committed goldens) + real `.dsn` fixtures (root `fixtures/`, 160+ real boards; harness fixtures under `rust/harness/fixtures/` and `scripts/benchmark/fixtures/`).
- **DRC completeness:** the Java `BoardStatistics.clearanceViolations.totalCount` face was incomplete (outline-only count); the Rust `epic-drc` full-violation face is the truthful count — cite the harness drc-corpus record, not an outline-only count.
- **KiCad quirk:** DSN export omits copper-to-edge clearance, so Freerouting routes to default class at the board edge and KiCad DRC flags it afterwards (see `docs/issues/Issue558-...md`).
- **Copper pours:** modeled as `ConductionArea` (Java) / the Rust pour model; `rules.Net.contains_plane` switched plane-routing mode. Known open bug carried from Java: plane routing can introduce clearance violations (Issue 093/152).
- **Logging:** the Java `FRLogger.trace(...)` face is retired; the Rust engine's event/trace faces are pinned by the harness events-golden compare.
- **Regression workflow (post-sunset):** compare against the committed goldens (the parity record); classify divergence (numeric drift vs ordering/tie-break) before fixing; exit criteria = no new clearance violations + no completion regression + stable compare metrics.
- Issue specs live in `docs/issues/` (historical Java-era investigations); scratch analysis goes in git-ignored `logs/<task>/`.
- License: GPLv3 — keep dependencies compatible.
