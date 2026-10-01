# Migrating from Freerouting to EpicRouter 2.0

EpicRouter 2.0 is a ground-up Rust rewrite of
[Freerouting](https://github.com/freerouting/freerouting). This guide is for
Freerouting users: what carries over unchanged, what the new CLI and GUI look
like, what differs by design, and — honestly — what the recorded bounds at
2.0 are. Every claim below cites its record; the pointers under
[Where the records live](#where-the-records-live) resolve them.

## 1. The workflow is identical

Nothing changes about how you interchange boards with your EDA tool:

- **DSN in → route → SES out.** You export a Specctra `.dsn` from KiCad (or
  any host that speaks the Specctra/Electra DSN interface), route it, and
  import the resulting `.ses` session file back. This is the same workflow
  Freerouting users know, and it is unchanged.
- **The interchange is pinned at the byte level, not claimed.** The
  committed parity record shows full DSN parse parity on all **1,332**
  fixtures (175 digest + 1,157 soak) and byte-equal SES session emission on
  the 20-fixture tier A+B set, both against the Freerouting Java engine
  captured before the rewrite's engine swap (`rust/README.md`, M1b; the
  `dsn compare` / `dsn ses-compare` harness gates stay green on every
  change). The same `.dsn` you fed Freerouting loads here; the `.ses` it
  writes imports exactly as before.
- **The GUI workflow carries over:** load a `.dsn` → autoroute → export the
  `.ses`.

**KiCad integrations at 2.0 — the decision:** EpicRouter 2.0 ships two
faces: the headless CLI (`epic-cli`) and the desktop GUI (`epic-gui`). There
is **no jar/plugin face**. The KiCad integration *is* the unchanged DSN/SES
interchange of section 1: export the `.dsn` from KiCad's Specctra
interface, route it, import the `.ses`. The Freerouting KiCad plugin — which
drove the Java jar from inside KiCad — belongs to the sunsetted Java tree
(deleted at the M10-T4 sunset); EpicRouter users use the identical file
flow, from the command line or the GUI, with no plugin.

## 2. The CLI face

Freerouting's headless invocation

```bash
java -jar freerouting.jar -de board.dsn -do out.ses
```

becomes

```bash
epic-cli route -de board.dsn -do out.ses
```

The `-de`/`-do` trigger face is the same: passing both starts an immediate
headless route (`CliSettings.java:100-107` — git history, `e7f9bdf1` — its
implicit router-enable, ported
verbatim — `epic-engine/src/settings.rs`). One addition: `epic-cli` takes an
explicit `route` subcommand, and `epic-cli --version` (new at 2.0) prints
the version face and exits 0.

### Flag mapping

| Freerouting | EpicRouter 2.0 | Notes |
|---|---|---|
| `-de <board.dsn>` | `route -de <board.dsn>` | Same meaning. Freerouting accepts multi-file/`+`-concatenated inputs (`.dsn`, optional `.ses`, optional `.rules`); EpicRouter takes **exactly one** DSN — a second `-de` file or a `+` concatenation is a hard error (the narrowing is documented in `epic-engine/src/settings.rs`). |
| `-do <out.ses>` | `route -do <out.ses>` | Same meaning. Freerouting also writes `.dsn`/`.scr` outputs by extension; EpicRouter writes the Specctra **session file** (the `.ses` KiCad imports). |
| `-mp <passes>` | `-mp <passes>` | Max autorouter passes. Same. |
| `-mt <threads>` | `-mt <threads>` | **Different default and different guarantee.** Freerouting defaults to (cores − 1); EpicRouter defaults to 1, and the result does not depend on the value: the output is byte-identical across `-mt 1/3/4` (the in-tree threads-invariance gate, `rust/README.md`). Threads change speed, never the board. |
| `-oit <threshold>` | `-oit <threshold>` | Deprecated exactly as in Freerouting (a warning fires; use `--router.optimizer.improvement_threshold=<value>`). |
| `-scoring-version <v>` | `-scoring-version <v>` | Same dual-application to both score boxes; `-router-scoring-version` / `-optimizer-scoring-version` address one box each. |
| `--router.<setting>=<value>` | same | The same property namespace (`router.*`, `optimizer.*`). |
| — | `--result-json <manifest.json>` | New: writes a machine-readable run manifest (schema version, `app_version`, counts, digests). |
| — | `--deterministic-budgets=on\|off` | New; **on by default** — see [Determinism is default-ON](#determinism-is-default-on). |
| — | `--dump-aesthetics <file>` | New: the four-metric aesthetics sidecar (a measurement face, never mixed into the manifest). |
| — | `--version` | New at 2.0: `epicrouter 2.0.0`, exit 0. |
| `-di` `-dr` `-drc` `-inc` `-im` `-us` `-hr` `-is` `-l` `-host` `-dct` `--debug.*` | **not carried** | ⚠️ An unknown short flag is **silently skipped** (Freerouting's own `CliSettings` behavior, ported verbatim), and an unknown `--property=value` outside `router.*`/`optimizer.*` is ignored. A script passing, say, `-inc GND,VCC` will *not* error — it will be ignored. Audit scripts that use these flags. |

### Settings layering

Settings resolve in layers — **defaults → DSN → CLI** — with the Java
`SettingsMerger` precedence the Rust engine inherits: a value in a higher
layer overrides a lower one only when that value is explicitly present. A
CLI flag overrides the board's declared setting; the DSN's declared settings
override the engine defaults; anything unset stays at the default. (The
GUI's session layer rides on top of the same merger.) This is the same
layering invariant Freerouting documented — nullable fields, defaults in
exactly one place — recorded in the project's architecture notes
(`CLAUDE.md`, "Settings system invariant"; `epic-engine/src/settings.rs`).

### Determinism is default-ON

Two product differences are deliberate:

- **Deterministic reruns.** Identical input routes to identical output
  bytes, every time. The engine's determinism is enforced by a standing
  self-gate: one DSN, two runs, byte-identical SES + manifest, pinned by
  committed digest canaries (`rust/README.md`, the determinism gate).
- **Budgets stop AT the budget.** With `--deterministic-budgets=on` (the
  default), a wall-clock budget ends the pass loop at the budget — the
  router stops on the recorded final board rather than overrunning. At 2.0
  the budget-vs-natural-wall question was answered **in favor of the
  budget** (the M10-T3 record, row 8: bounded, deterministic at the budget
  face; the wall-clock stop is load-positioned, so a board still
  mid-convergence at its budget stop is empirically, not structurally,
  byte-stable). Timeouts and budgets are never silently extended.

## 3. The GUI

EpicRouter ships an egui desktop shell (`epic-gui`):

- **Load** a `.dsn` (file dialog; parse warnings and an outline-missing
  notice surface inline, and an outline-less board routes on the default
  boundary — the same face as the CLI).
- **Route** with live progress: the stage sequence (fanout → routing →
  optimization) and per-pass counters stream into a progress panel, and the
  canvas redraws as engine snapshots land.
- **Cancel** mid-run.
- **Export** the `.ses` (the export is gated: it is refused on a cancelled
  session and enabled once the run finished).
- **Overlay views** on the canvas: ratsnest airlines, DRC markers,
  congestion data, and tuning/pair data — each sourced from the engine's
  board snapshots, each toggleable.

### Cancel semantics at 2.0 (the post-M10-T1 behavior)

The cancel button raises the session's stop flag, and the final state
depends on where the route is:

- **Cancel during the routing stage** → the run ends **CANCELLED**; nothing
  is exported (the export gate refuses a CANCELLED session).
- **Cancel during the optimization stage** → the stop is honored
  immediately, and the session lands **COMPLETED — not CANCELLED**: the
  routing stage had already finished, so the SES export carries the full
  routed board with the remaining optimization passes cut short. This is
  the M10-T1 stop-flag wiring finding: Java's pipeline propagates no stop
  state out of the optimizer (`RoutingPipeline.java:121-134` — git
  history, `e7f9bdf1` — is void), and
  EpicRouter keeps the COMPLETED face for that window.
- **A cancel raised before the route starts** is a documented pass-through:
  the session completes COMPLETED.

The status bar shows the final state either way (`route: <state> incomplete
N violations M`).

### One route per session

A session routes **once**. Re-routing is disabled — to route again (for
example after changing settings), load the DSN fresh. The GUI's Route
button disables after a run; the engine enforces the same rule on the API
path with a clean refusal (`RouteError::AlreadyRouted { final_state }`
carrying the prior run's final state) instead of a silent re-run.

## 4. What differs by design

- **Determinism and threads-invariance are gates, not goals-of-the-week.**
  Every change must leave the determinism canaries untouched: the SES
  digest and the manifest digest of the standing self-gate board are
  committed literals, and the manifest digest is **version-blind** (the
  `app_version`/`git_sha` fields are normalized out of the digest, so a
  release bump can never rotate it). The threads-invariance gate proves the
  board is byte-identical across thread counts (`rust/README.md`, the gate
  section).
- **The engine is a Rust workspace.** Freerouting's Java engine is the
  frozen parity oracle this rewrite was measured against; it was sunset at
  M10 (the tree lives in git history and upstream, grafted as
  `origin/master` at `e7f9bdf1`). The crate map: `epic-geometry` (exact 45°
  integer geometry), `epic-index` (incremental spatial index), `epic-board`
  (arena board model), `epic-dsn` (Specctra I/O), `epic-drc`
  (incompletes + clearance counting), `epic-router` (fanout → maze routing
  → optimizer pipeline, plus the default-OFF global-planning, push-and-shove,
  tuning, and gloss stages), `epic-engine` (the headless application core:
  settings layering, sessions, the event stream), `epic-cli`, `epic-gui`,
  `epic-harness` (the parity/benchmark harness).
- **Speed: only the committed record speaks.** The CI router gate — the
  Tier A router-only battery in gate mode — runs 11 green / 0 red at
  **Σ 286.5 s**, 5.3–6.3× headroom under the compare step's 30-minute
  budget (the committed record: `rust/README.md`; the gate-lap evidence
  itself is internal dev-box scratch, not shipped). The
  Tier A full-comparison record is 11 pass / 0 red against the Freerouting
  Java records (completion ≥, violations ≤, score ≥, row for row). No
  wall-clock claim against a local Java install is made here; run your own
  boards and compare.
- **Extra routing intelligence ships default-OFF.** The global-planning
  (congestion/PathFinder), push-and-shove, length-tuning, and gloss
  (aesthetics) stage families are available behind `router.*` settings and
  are all off at defaults. In particular, the 45° flow pass ships
  **available-but-unengaged** — the default pipeline does not engage the
  flow pass (the M10-T3 disposition, row 2).

## 5. Known bounds at 2.0

The M10 measurement moment (T3) re-faced every carried deviation and
dispositioned each row; the rows below rode **post-2.0** with named owners
and exit conditions (the full table is committed in the M10 plan,
AMENDMENT 3; the fuller report is internal dev-box scratch, not shipped). In
user terms:

| Board / face | What you may see at 2.0 | Record |
|---|---|---|
| bm01 (Tier A) | Completes; on the session path the 1800 s tier budget stops it AT the budget on a final board byte-identical to its natural-wall completion. The router-only CI face is green (11/0). | T3 row 6 (the ot=4 <!-- codespell:ignore (ot=4 is the bm01 lever name, design :800) --> lever stays opt-in, declined) |
| bm05 (Tier B) | A completion red: 29 vs 18 (Rust vs Java record) at the re-faced battery, ~289 s — an engine-capability gap in the congestion family, owned post-2.0. | T3 row 7 |
| bm10 (Tier B) | The 900 s tier budget is the binding product face: the session path stops AT budget (byte-reproducible at the budget face, empirically); the engine completes unbounded at its natural wall (~1446 s recorded). Budgets are never raised. | T3 row 8 |
| interf_u (Issue 093) | One router-introduced clearance violation vs Java's 0 (~72 s), flag-independent — the plane-routing clearance family carried from upstream. | T3 row 9; Issue 093/152 |
| 1Bitsy | Completion red 2 vs 1 incompletes (~271 s). | T3 row 12 |
| StickHub / LimeSDR (Tiers B/C) | Killed at the 900 s wall on the tier walks (both engines' budgets; row 8's budget-face decision is the attribution precedent). | T3 row 11 |
| multichannel_mixer (+ mm-unrouted) | Parity floors **holding exact** (160=160; 128=128, cv 285=285) — re-recorded, no drift. | T3 row 10 |
| Aesthetics family | The teardrop instrument-vs-consumer tension stands documented (the four-metric instrument is a measurement instrument, not a 2.0 product face); the via_density lever stays closed post-2.0; flow ships unengaged (section 4). | T3 rows 1–3 |
| Copper pours | Advisory island detection ships (a manifest face; zero divergences); the ROUTING-side pour face (plane-routing clearance) is first-measured post-2.0 — no argv measures it today. | T3 row 5 |

One internal gate-arming row also rides post-2.0 — the F3 check (an
internal corpus-reopen trigger from the pre-2.0 records; the diagonal-trigger
arming is dev-facing instrumentation, corpora-gated, not a user-facing bound);
see the cited full table (the M10 plan, AMENDMENT 3).

Two honest scoping notes: the Tier B/C tier walks read 4 green / 5 red and
1 green / 2 red — row-for-row identical to the M8/M9 records (no 2.0
regression, recorded carries); and the wall-clock budget stops are
load-positioned (section 2's determinism note).

## 6. License and attribution

EpicRouter is free software under **GPLv3** (the `LICENSE` file). It is a
fork of **Freerouting** by Andras Fuchs — the Java engine, its algorithmic
decisions, and its deep architecture are the foundation of this rewrite,
and the project owes its existence to the upstream project and its
contributors. The upstream history is grafted into this repository as
`origin/master` at baseline `e7f9bdf1`, and the product name in prose
remains "**Freerouting**" wherever the upstream project is meant.

## Where the records live

- **Committed parity goldens** (the post-sunset comparability carriers):
  `rust/harness/baselines/`, the corpus dirs, `rust/harness/config/tiers.yaml`.
- **The standing gate posture:** `rust/README.md` (status block + the
  compare/gate command faces).
- **The M10 record:** `docs/superpowers/plans/2026-09-30-epicrouter-m10-sunset-and-release.md`
  (AMENDMENTS 1–5), the design doc §6 exit notes
  (`docs/superpowers/specs/2026-09-11-epicrouter-rust-rewrite-design.md`).
- **The T3 deviation table** (section 5's source): the M10 plan AMENDMENT 3
  (committed); the full report is task scratch under `logs/M10-T3/`.
- **Port ledger:** `rust/crates/epic-router/SEAM.md`.
